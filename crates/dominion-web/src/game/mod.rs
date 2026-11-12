//! The game page: which seat this browser shows, keeping its [`GameState`] current over
//! the change feed, and answering for it.

pub mod board;
pub mod interaction;
pub mod logfmt;
pub mod plain;
pub mod state;

use crate::api;
use crate::app::Route;
use crate::platform;
use crate::session::{self, HeldSeat};
use dioxus::prelude::*;
use dominion_protocol::Answer;
use dominion_protocol::api::Change;
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
    for _ in 0..3 {
        let from = state.peek().log_from();
        let result = api::view(&game, token, Some(from)).await;
        if !still_shown(ctx.seat, token) {
            return;
        }
        match result {
            Ok(sv) => {
                if state.write().apply(sv) == Applied::Done {
                    break;
                }
            }
            Err(e) => {
                state.write().error = Some(e.message);
                return;
            }
        }
    }
}

/// Keeps the shown seat's state current until cancelled.
async fn drive(ctx: Live, token: String) {
    let (tx, mut rx) = futures_channel::mpsc::unbounded::<Change>();
    let game = ctx.game.peek().clone();
    let watcher = api::watch(&game, &token, move |c| {
        let _ = tx.unbounded_send(c);
    });
    let worker = async {
        refresh(ctx, &token).await;
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
            let mut st = state.write();
            st.answering = false;
            st.apply(sv) == Applied::Refetch
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
    use_context_provider(|| GameCtx {
        game,
        seat,
        state,
        deciding,
        submit: Callback::new(move |answer: Answer| {
            spawn(send_answer(live, answer));
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
    rsx! {
        div { class: "game-page",
            div { class: "seat-bar",
                span { class: "mono", "Game {game}" }
                if held.len() > 1 {
                    span { class: "muted", "Seat:" }
                    for s in held.iter().cloned() {
                        button {
                            key: "{s.seat}",
                            class: if seat().as_ref().is_some_and(|c| c.seat == s.seat) { "toggle on" } else { "toggle" },
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
                Link { class: "right", to: Route::Lobby {}, "Lobby" }
            }
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
                plain::PlainGame {}
            }
        }
    }
}
