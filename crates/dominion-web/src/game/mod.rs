//! The game page: which seat this browser shows, keeping its [`GameState`] current over
//! the change feed, and answering for it.

pub mod board;
pub mod entry;
pub mod interaction;
pub mod logfmt;
pub mod moves;
pub mod plain;
pub mod state;
pub mod table;
pub mod wording;

use crate::api;
use crate::app::Route;
use crate::lobby::seat_link::SeatLink;
use crate::platform;
use crate::session::{self, HeldSeat};
use dioxus::prelude::*;
use dominion_protocol::Answer;
use dominion_protocol::api::{Change, GameSummary};
use futures_util::StreamExt;
pub use state::{Applied, GameState, Selection};

/// What the game page provides to everything rendered inside it.
#[derive(Clone, Copy)]
pub struct GameCtx {
    pub game: Signal<String>,
    /// The seat shown, if the browser holds one.
    pub seat: Signal<Option<HeldSeat>>,
    pub state: Signal<GameState>,
    /// The seat deciding now, when the game is running and someone is.
    pub deciding: Memo<Option<u8>>,
    /// Sends an answer for the shown seat.
    pub submit: Callback<Answer>,
    /// Takes back the shown seat's last answer.
    pub undo: Callback<()>,
}

pub fn use_game() -> GameCtx {
    use_context::<GameCtx>()
}

/// The page's signals, for the tasks that keep them current.
#[derive(Clone, Copy)]
struct Live {
    game: Signal<String>,
    seat: Signal<Option<HeldSeat>>,
    state: Signal<GameState>,
}

fn still_shown(seat: Signal<Option<HeldSeat>>, token: &str) -> bool {
    seat.peek().as_ref().is_some_and(|s| s.token == token)
}

/// Fetches the view from the end of the log held.
async fn refresh(ctx: Live, token: &str) {
    let game = ctx.game.peek().clone();
    let mut state = ctx.state;
    let mut from = state.peek().log_from();
    for _ in 0..3 {
        let result = api::view(&game, token, Some(from)).await;
        if !still_shown(ctx.seat, token) {
            return;
        }
        match result {
            Ok(sv) => {
                table::before_change();
                let applied = state.write().apply(sv);
                match applied {
                    Applied::Done => break,
                    Applied::Refetch => from = state.peek().log_from(),
                    Applied::Restart => from = 0,
                }
            }
            Err(e) => {
                let mut st = state.write();
                st.gone = matches!(e.status, 401 | 404);
                st.error = Some(e.message);
                return;
            }
        }
    }
}

/// Keeps the shown seat's state current until cancelled.
async fn drive(ctx: Live, token: String) {
    refresh(ctx, &token).await;
    if ctx.state.peek().gone {
        return;
    }
    let (tx, mut rx) = futures_channel::mpsc::unbounded::<Change>();
    let game = ctx.game.peek().clone();
    let watcher = api::watch(&game, &token, move |c| {
        let _ = tx.unbounded_send(c);
    });
    let worker = async {
        while let Some(c) = rx.next().await {
            if ctx.state.peek().is_news(c) {
                refresh(ctx, &token).await;
            }
        }
    };
    futures_util::future::join(watcher, worker).await;
}

async fn send_answer(ctx: Live, answer: Answer) {
    let Some(seat) = ctx.seat.peek().clone() else {
        return;
    };
    let mut state = ctx.state;
    let game = ctx.game.peek().clone();
    state.write().answering = true;
    let result = api::answer(&game, &seat.token, &answer).await;
    if !still_shown(ctx.seat, &seat.token) {
        return;
    }
    let stale = match result {
        Ok(sv) => {
            table::before_change();
            let mut st = state.write();
            st.answering = false;
            st.apply(sv) != Applied::Done
        }
        Err(e) => {
            let stale = state.write().answer_failed(&e);
            if stale {
                let msg = e.message;
                refresh(ctx, &seat.token).await;
                state.write().error = Some(msg);
                return;
            }
            false
        }
    };
    if stale {
        refresh(ctx, &seat.token).await;
    }
}

async fn send_undo(ctx: Live) {
    let Some(seat) = ctx.seat.peek().clone() else {
        return;
    };
    let mut state = ctx.state;
    if state.peek().answering {
        return;
    }
    let game = ctx.game.peek().clone();
    state.write().answering = true;
    let result = api::undo(&game, &seat.token).await;
    if !still_shown(ctx.seat, &seat.token) {
        return;
    }
    let refetch = match result {
        Ok(sv) => {
            table::before_change();
            let mut st = state.write();
            st.answering = false;
            st.apply(sv) != Applied::Done
        }
        Err(e) => state.write().answer_failed(&e),
    };
    if refetch {
        refresh(ctx, &seat.token).await;
    }
}

/// The game as `/api/games` lists it, kept current while some seat hasn't joined.
fn use_summary(game: Signal<String>) -> Signal<Option<GameSummary>> {
    let mut summary = use_signal(|| None::<GameSummary>);
    use_future(move || async move {
        loop {
            let id = game.peek().clone();
            if let Ok(list) = api::games().await {
                let g = list.into_iter().find(|g| g.game_id == id);
                let done = g
                    .as_ref()
                    .is_none_or(|g| !g.running || g.joined.iter().all(|&j| j));
                if *summary.peek() != g {
                    summary.set(g);
                }
                if done {
                    break;
                }
            }
            platform::sleep(2000).await;
        }
    });
    summary
}

/// The seat the game waits on when nobody has joined it: its number and name.
pub fn waiting_for(g: &GameSummary, held: &[HeldSeat]) -> Option<(u8, String)> {
    let d = g.deciding.filter(|_| g.running)?;
    let joined = g.joined.get(usize::from(d)).copied().unwrap_or(true);
    if joined || held.iter().any(|h| h.seat == d) {
        return None;
    }
    Some((d, g.names.get(usize::from(d)).cloned().unwrap_or_default()))
}

/// Says the game is waiting for a seat nobody has joined, with what to give its agent
/// when this browser handed the seat out.
#[component]
pub fn WaitingNotice(#[props(default)] style: String) -> Element {
    let ctx = use_game();
    let summary = use_context::<Signal<Option<GameSummary>>>();
    let mut hidden = use_signal(|| false);
    let game = ctx.game.read().clone();
    let held = session::seats(&game);
    let Some(g) = summary() else {
        return rsx! {};
    };
    let Some((seat, name)) = waiting_for(&g, &held) else {
        return rsx! {};
    };
    if hidden() {
        return rsx! {};
    }
    let link = session::links(&game).into_iter().find(|l| l.seat == seat);
    rsx! {
        div { class: "waiting-notice", role: "status", style, "data-waiting-seat": "{seat}",
            button { class: "waiting-hide", title: "Hide", onclick: move |_| hidden.set(true), "✕" }
            if let Some(l) = link {
                p { class: "waiting-title", "Waiting for {name} to join. Give its agent this line:" }
                SeatLink { game, seat: l, joined: Some(false) }
            } else {
                p { class: "waiting-title", "Waiting for {name}, who hasn't joined this game yet." }
            }
        }
    }
}

#[component]
pub fn GamePage(id: String) -> Element {
    let game = use_signal(|| id.clone());
    let mut seats = use_signal(|| session::seats(&id));
    let mut seat = use_signal(|| seats.peek().first().cloned());
    let mut state = use_signal(GameState::default);
    let deciding = use_memo(move || state.read().deciding);
    let mut follow = use_signal(|| true);
    let mut join_error = use_signal(|| None::<String>);
    let mut driver = use_signal(|| None::<dioxus::core::Task>);

    let live = Live { game, seat, state };
    let summary = use_summary(game);
    use_context_provider(|| summary);
    use_context_provider(|| GameCtx {
        game,
        seat,
        state,
        deciding,
        submit: Callback::new(move |answer: Answer| {
            spawn(send_answer(live, answer));
        }),
        undo: Callback::new(move |()| {
            spawn(send_undo(live));
        }),
    });

    let take_token = move |token: String| {
        let game_id = game.peek().clone();
        spawn(async move {
            match api::seat(&token).await {
                Ok(info) if info.game_id == game_id => {
                    let held = HeldSeat {
                        seat: info.seat,
                        name: info.name,
                        token,
                    };
                    session::hold(&game_id, held.clone());
                    seats.set(session::seats(&game_id));
                    seat.set(Some(held));
                    join_error.set(None);
                }
                Ok(info) => join_error.set(Some(format!(
                    "that token is for game {}, not this one",
                    info.game_id
                ))),
                Err(e) => join_error.set(Some(e.message)),
            }
        });
    };

    use_hook(move || {
        if let Some(token) = platform::take_fragment_token() {
            take_token(token);
        }
    });

    let gone = use_memo(move || {
        let st = state.read();
        st.gone.then(|| st.error.clone().unwrap_or_default())
    });
    use_effect(move || {
        if gone().is_none() {
            return;
        }
        if let Some(s) = seat.peek().as_ref() {
            session::forget(&game.peek(), &s.token);
        }
    });

    use_effect(move || {
        let shown = seat();
        if let Some(t) = driver.take() {
            t.cancel();
        }
        state.set(GameState::default());
        if let Some(s) = shown {
            driver.set(Some(spawn(drive(live, s.token))));
        }
    });

    use_effect(move || {
        let Some(d) = deciding() else { return };
        if !follow() || seat.peek().as_ref().is_some_and(|s| s.seat == d) {
            return;
        }
        if let Some(s) = seats.peek().iter().find(|s| s.seat == d) {
            seat.set(Some(s.clone()));
        }
    });

    let held = seats();
    let mut paste = use_signal(String::new);
    let mut table_on = use_signal(table::table_mode);
    let seat_bar = rsx! {
        div { class: "seat-bar",
            span { class: "mono", "Game {game}" }
            if held.len() > 1 {
                span { class: "muted", "Seat:" }
                for s in held.iter().cloned() {
                    button {
                        key: "{s.seat}",
                        class: if seat().as_ref().is_some_and(|c| c.seat == s.seat) { "toggle on" } else { "toggle" },
                        "data-seat-button": "{s.seat}",
                        onclick: move |_| seat.set(Some(s.clone())),
                        "{s.name}"
                    }
                }
                label { class: "check",
                    input {
                        r#type: "checkbox",
                        checked: follow(),
                        onchange: move |e| follow.set(e.checked()),
                    }
                    "follow the deciding seat"
                }
            } else if let Some(s) = held.first() {
                span { "playing as {s.name}" }
            }
            span { class: "view-toggle",
                button {
                    class: if table_on() { "toggle on" } else { "toggle" },
                    "data-view": "table",
                    onclick: move |_| { table::set_table_mode(true); table_on.set(true); },
                    "Table"
                }
                button {
                    class: if table_on() { "toggle" } else { "toggle on" },
                    "data-view": "plain",
                    onclick: move |_| { table::set_table_mode(false); table_on.set(false); },
                    "Plain"
                }
            }
            Link { class: "right", to: Route::Lobby {}, "Lobby" }
        }
    };
    if let Some(e) = gone() {
        let id = game.read().clone();
        return rsx! {
            div { class: "game-page",
                {seat_bar}
                section { class: "panel", "data-gone": "1",
                    h2 { "This game can't be opened" }
                    p { "The server said: {e}." }
                    p { "It may have been started on a server that didn't keep its games, or this browser's seat token for game {id} is no longer valid. This browser has forgotten that seat." }
                    Link { class: "button primary", to: Route::Lobby {}, "Back to the lobby" }
                }
            }
        };
    }
    if !held.is_empty() && table_on() && join_error().is_none() {
        return rsx! {
            table::TableGame { controls: seat_bar }
        };
    }
    rsx! {
        div { class: "game-page",
            {seat_bar}
            if held.is_empty() {
                section { class: "panel",
                    h2 { "Take a seat" }
                    p { "This browser holds no seat in this game. Paste a seat token:" }
                    div { class: "row",
                        input {
                            class: "grow",
                            placeholder: "seat token",
                            value: "{paste}",
                            oninput: move |e| paste.set(e.value()),
                        }
                        button { class: "primary", onclick: move |_| take_token(paste().trim().to_string()), "Join" }
                    }
                    if let Some(e) = join_error() {
                        p { class: "error", "{e}" }
                    }
                }
            } else {
                if let Some(e) = join_error() {
                    p { class: "error", "{e}" }
                }
                WaitingNotice {}
                plain::PlainGame {}
            }
        }
    }
}
