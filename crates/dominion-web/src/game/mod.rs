//! The game page: which seat this browser shows (one it holds, one its account plays, or
//! none: watching), keeping its [`GameState`] current over the change feed, and answering,
//! resigning and speaking for it.

mod agent_log;
pub mod board;
mod chat;
pub mod counts;
pub mod entry;
pub mod interaction;
pub mod logfmt;
pub mod menu;
pub mod meta;
pub mod moves;
pub mod plain;
mod replay;
pub mod state;
pub mod table;
mod undo;
pub mod wording;

use crate::account::use_account;
use crate::api::{self, ApiError};
use crate::lobby::seat_link::SeatLink;
use crate::platform;
use crate::session::{self, HeldSeat};
use dioxus::prelude::*;
use dominion_protocol::Answer;
use dominion_protocol::api::{Change, GameSummary, SeatView};
use futures_util::StreamExt;
pub use state::{Applied, GameState, Selection};

/// A game as its page is given it: what every source of games provides (this server's
/// games in `GamePage`, a finished one's replay, an official account's game in
/// `online.rs`) and all that [`GameShell`], the table and the plain page read.
#[derive(Clone, Copy)]
pub struct GameCtx {
    pub game: Signal<String>,
    /// The seat shown, if the browser holds one.
    pub seat: Signal<Option<HeldSeat>>,
    /// The game is shown as to someone watching: no seat here.
    pub watching: Memo<bool>,
    pub state: Signal<GameState>,
    /// The seat deciding now, when the game is running and someone is.
    pub deciding: Memo<Option<u8>>,
    /// Sends an answer for the shown seat.
    pub submit: Callback<Answer>,
    /// Sends an answer with the Way question of the card it plays answered: `none` or a
    /// Way's name.
    pub submit_way: Callback<(Answer, String)>,
    /// Asks to take back the shown seat's last answer.
    pub undo: Callback<()>,
    /// Allows (true) or refuses the undo waiting for the shown seat.
    pub answer_undo: Callback<bool>,
    /// Withdraws the shown seat's undo request.
    pub cancel_undo: Callback<()>,
    /// Resigns the shown seat.
    pub resign: Callback<()>,
    /// The question before resigning is open.
    pub resigning: Signal<bool>,
    pub offers: Memo<Offers>,
}

/// What a game's source has besides its views. The page draws only what is offered.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Offers {
    /// The game has a chat (`game-<id>`).
    pub chat: bool,
    /// The id its agents' transcript is kept under.
    pub transcript: Option<String>,
    /// In a replay short of its end: how many log entries are shown, the transcript
    /// shown only as far as them.
    pub transcript_at: Option<usize>,
}

/// Whether one of the game's seats is played by an agent.
fn has_agents(meta: &dominion_protocol::lobby::GameMeta) -> bool {
    (meta.users.iter().flatten()).any(|u| u.kind == dominion_protocol::lobby::UserKind::Bot)
}

pub fn use_game() -> GameCtx {
    use_context::<GameCtx>()
}

/// Provides a game that is only looked at: nothing is answered, undone or resigned.
pub fn use_watched_game(
    game: Signal<String>,
    state: Signal<GameState>,
    offers: Memo<Offers>,
) -> GameCtx {
    let seat = use_signal(|| None);
    let watching = use_memo(|| true);
    let deciding = use_memo(move || state.read().deciding);
    let resigning = use_signal(|| false);
    use_context_provider(|| GameCtx {
        game,
        seat,
        watching,
        state,
        deciding,
        submit: Callback::new(|_| {}),
        submit_way: Callback::new(|_| {}),
        undo: Callback::new(|_| {}),
        answer_undo: Callback::new(|_| {}),
        cancel_undo: Callback::new(|_| {}),
        resign: Callback::new(|_| {}),
        resigning,
        offers,
    })
}

impl GameCtx {
    fn via(&self) -> Option<Via> {
        match self.seat.peek().as_ref() {
            Some(s) => Some(Via::Seat(s.token.clone())),
            None => self.watching.peek().then_some(Via::Watch(None)),
        }
    }

    /// Says `text` in the game's chat: as the seat shown, else as the account watching.
    pub async fn say(self, text: &str) -> Result<(), ApiError> {
        let Some(via) = self.via() else {
            return Ok(());
        };
        let game = self.game.peek().clone();
        api::say(&chat_channel(&game), text, via.token()).await?;
        let counter = self.state.peek().chat_seen.unwrap_or(0);
        if let Ok(log) = fetch_chat(self.state, &game, &via).await
            && self.via() == Some(via)
        {
            let mut state = self.state;
            state.write().merge_chat(counter, log);
        }
        Ok(())
    }
}

/// Milliseconds since the epoch, or 0 where there is no clock (natively).
#[cfg(all(feature = "web", target_arch = "wasm32"))]
fn now_ms() -> u64 {
    js_sys::Date::now() as u64
}

#[cfg(not(all(feature = "web", target_arch = "wasm32")))]
fn now_ms() -> u64 {
    0
}

/// How far the server's clock is ahead of this one, from the server's time in a response
/// just received; `was` when either clock can't be read.
pub(crate) fn server_ahead(server_now: u64, was: i64) -> i64 {
    match (server_now, now_ms()) {
        (0, _) | (_, 0) => was,
        (server, here) => server as i64 - here as i64,
    }
}

/// The deciding seat's time running out, when there is something to say about it; the
/// calling component is drawn again every second while it counts down.
pub fn use_forfeit() -> Option<(meta::Forfeit, String)> {
    let game = use_game();
    let mut now = use_signal(now_ms);
    let counting = use_memo(move || {
        let st = game.state.read();
        st.deciding.is_some() && st.meta.forfeit_at.is_some()
    });
    use_future(move || async move {
        while now_ms() != 0 {
            platform::sleep(1000).await;
            if *counting.peek() {
                now.set(now_ms());
            }
        }
    });
    let st = game.state.read();
    let view = st.view.as_ref()?;
    let server_now = match now() {
        0 => 0,
        here => (here as i64 + st.server_ahead).max(1) as u64,
    };
    let f = meta::forfeit(&st.meta, st.deciding, view.seat, server_now)?;
    let names = logfmt::names(view);
    let name = names.get(usize::from(f.seat)).cloned().unwrap_or_default();
    let text = f.text(&name);
    Some((f, text))
}

/// The game's chess clock as it stands now, if it has one; the calling component is
/// drawn again as the running seat's time counts down.
pub fn use_clock() -> Option<dominion_protocol::lobby::ClockView> {
    let game = use_game();
    let mut now = use_signal(now_ms);
    use_future(move || async move {
        while now_ms() != 0 {
            platform::sleep(250).await;
            let st = game.state.peek();
            if st.meta.clock.as_ref().is_some_and(|c| c.running.is_some()) {
                now.set(now_ms());
            }
        }
    });
    let st = game.state.read();
    let clock = st.meta.clock.as_ref()?;
    let server_now = match now() {
        0 => 0,
        here => (here as i64 + st.server_ahead).max(1) as u64,
    };
    Some(meta::clock_at(clock, st.meta.now, server_now))
}

/// How a page reaches its game: as a seat, by its token, or as someone watching, by the
/// session's cookie (seeing everything, or what one seat knows).
#[derive(Clone, Debug, PartialEq, Eq)]
enum Via {
    Seat(String),
    Watch(Option<u8>),
}

impl Via {
    fn token(&self) -> Option<&str> {
        match self {
            Via::Seat(t) => Some(t),
            Via::Watch(_) => None,
        }
    }

    async fn view(&self, game: &str, log_from: usize) -> api::Result<SeatView> {
        match self {
            Via::Seat(t) => api::view(game, t, Some(log_from)).await,
            Via::Watch(seat) => api::watch_view(game, Some(log_from), *seat).await,
        }
    }
}

fn chat_channel(game: &str) -> String {
    format!("game-{game}")
}

/// The chat's messages after the last one held.
async fn fetch_chat(
    state: Signal<GameState>,
    game: &str,
    via: &Via,
) -> api::Result<Vec<dominion_protocol::lobby::ChatMessage>> {
    let after = state.peek().chat_from();
    let log = api::chat(&chat_channel(game), after, via.token()).await?;
    Ok(log.messages)
}

/// The page's signals, for the tasks that keep them current.
#[derive(Clone, Copy)]
struct Live {
    game: Signal<String>,
    seats: Signal<Vec<HeldSeat>>,
    seat: Signal<Option<HeldSeat>>,
    /// With no seat, the game is watched.
    watch: Signal<bool>,
    watch_as: Signal<Option<u8>>,
    /// The account is being asked for its seats.
    finding: Signal<bool>,
    /// How often that was done: a seat the server then refuses isn't looked for again.
    finds: Signal<u8>,
    /// Why the game is neither played nor watched here.
    refused: Signal<Option<String>>,
    state: Signal<GameState>,
}

impl Live {
    fn still_shown(&self, via: &Via) -> bool {
        match via {
            Via::Seat(t) => self.seat.peek().as_ref().is_some_and(|s| &s.token == t),
            Via::Watch(seat) => {
                self.seat.peek().is_none() && *self.watch.peek() && *self.watch_as.peek() == *seat
            }
        }
    }
}

/// With no seat held here: holds the seats this game has for the account signed in, and
/// failing that watches the game without requiring an account.
async fn find_role(mut ctx: Live) {
    if *ctx.finds.peek() >= 2 {
        return;
    }
    *ctx.finds.write() += 1;
    ctx.finding.set(true);
    let game = ctx.game.peek().clone();
    if let Ok(mine) = api::my_games().await {
        for g in mine.into_iter().filter(|g| g.game_id == game) {
            let held = HeldSeat {
                seat: g.seat,
                name: g.name,
                token: g.token,
            };
            session::hold(&game, held);
        }
    }
    let held = session::seats(&game);
    if ctx.seat.peek().is_none() {
        if let Some(first) = held.first().cloned() {
            ctx.seats.set(held);
            ctx.seat.set(Some(first));
        } else {
            ctx.watch.set(true);
        }
    }
    ctx.finding.set(false);
}

/// The server refused the shown seat's token: forgets it and shows another seat held, or
/// looks for the account's.
fn lose_seat(mut ctx: Live, token: &str) {
    let game = ctx.game.peek().clone();
    session::forget(&game, token);
    let held = session::seats(&game);
    let next = held.first().cloned();
    ctx.seats.set(held);
    let found = next.is_some();
    ctx.seat.set(next);
    if !found {
        spawn(find_role(ctx));
    }
}

/// What a failed fetch means for the page.
fn fetch_failed(mut ctx: Live, via: &Via, e: ApiError) {
    let first = ctx.state.peek().view.is_none();
    match via {
        Via::Seat(token) if e.status == 401 && *ctx.finds.peek() < 2 => lose_seat(ctx, token),
        Via::Watch(_) if first && matches!(e.status, 401 | 403) => {
            ctx.watch.set(false);
            ctx.refused.set(Some(e.message));
        }
        _ => {
            let mut st = ctx.state.write();
            st.gone = matches!(e.status, 401 | 404);
            st.error = Some(e.message);
        }
    }
}

/// Fetches the view from the end of the log held. False when the fetch failed.
async fn refresh(ctx: Live, via: &Via) -> bool {
    let game = ctx.game.peek().clone();
    let mut state = ctx.state;
    let mut from = state.peek().log_from();
    for _ in 0..3 {
        let result = via.view(&game, from).await;
        if !ctx.still_shown(via) {
            return false;
        }
        match result {
            Ok(sv) => {
                if state.peek().is_current(&sv) {
                    break;
                }
                table::before_change();
                let applied = {
                    let mut st = state.write();
                    st.server_ahead = server_ahead(sv.meta.now, st.server_ahead);
                    st.apply(sv)
                };
                match applied {
                    Applied::Done => break,
                    Applied::Refetch => from = state.peek().log_from(),
                    Applied::Restart => from = 0,
                }
            }
            Err(e) => {
                fetch_failed(ctx, via, e);
                return false;
            }
        }
    }
    true
}

/// Keeps the shown game's state current until cancelled.
async fn drive(ctx: Live, via: Via) {
    if !refresh(ctx, &via).await {
        return;
    }
    let (tx, mut rx) = futures_channel::mpsc::unbounded::<Change>();
    let game = ctx.game.peek().clone();
    let on_change = move |c| {
        let _ = tx.unbounded_send(c);
    };
    let feed = async {
        match &via {
            Via::Seat(token) => api::watch(&game, token, on_change).await,
            Via::Watch(_) => api::watch_unseated(&game, on_change).await,
        }
    };
    let worker = async {
        let mut state = ctx.state;
        while let Some(c) = rx.next().await {
            if state.peek().is_news(c) {
                refresh(ctx, &via).await;
            }
            if state.peek().chat_is_news(c)
                && let Ok(log) = fetch_chat(state, &game, &via).await
                && ctx.still_shown(&via)
            {
                state.write().merge_chat(c.chat, log);
            }
        }
    };
    futures_util::future::join(feed, worker).await;
}

/// Takes in the answer to an answer or a resignation.
async fn sent(ctx: Live, via: Via, result: api::Result<SeatView>) {
    let mut state = ctx.state;
    if !ctx.still_shown(&via) {
        return;
    }
    let stale = match result {
        Ok(sv) => {
            table::before_change();
            let mut st = state.write();
            st.answering = false;
            st.server_ahead = server_ahead(sv.meta.now, st.server_ahead);
            st.apply(sv) != Applied::Done
        }
        Err(e) => {
            let stale = state.write().answer_failed(&e);
            if stale {
                let msg = e.message;
                refresh(ctx, &via).await;
                state.write().error = Some(msg);
                return;
            }
            false
        }
    };
    if stale {
        refresh(ctx, &via).await;
    }
}

async fn send_answer(ctx: Live, answer: Answer, way: Option<String>) {
    let Some(seat) = ctx.seat.peek().clone() else {
        return;
    };
    let mut state = ctx.state;
    let game = ctx.game.peek().clone();
    state.write().answering = true;
    let result = api::answer(&game, &seat.token, &answer, way).await;
    sent(ctx, Via::Seat(seat.token), result).await;
}

async fn send_resign(ctx: Live) {
    let Some(seat) = ctx.seat.peek().clone() else {
        return;
    };
    let mut state = ctx.state;
    let game = ctx.game.peek().clone();
    state.write().answering = true;
    let result = api::resign(&game, &seat.token).await;
    sent(ctx, Via::Seat(seat.token), result).await;
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
    sent(ctx, Via::Seat(seat.token), result).await;
}

async fn send_answer_undo(ctx: Live, allow: Option<bool>) {
    let Some(seat) = ctx.seat.peek().clone() else {
        return;
    };
    let mut state = ctx.state;
    if state.peek().answering {
        return;
    }
    let game = ctx.game.peek().clone();
    state.write().answering = true;
    let result = match allow {
        Some(allow) => api::answer_undo(&game, &seat.token, allow).await,
        None => api::cancel_undo(&game, &seat.token).await,
    };
    sent(ctx, Via::Seat(seat.token), result).await;
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

pub use replay::ReplayPage;
pub use undo::{UndoNotice, undo_title};

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

/// Table or Plain, remembered for the session.
#[component]
pub fn ViewToggle(table_on: Signal<bool>) -> Element {
    rsx! {
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
    }
}

/// The bar over every game: its title, whose view is shown (the children), how many
/// watch, the view toggle and the menu.
#[component]
pub fn GameBar(title: String, table_on: Signal<bool>, children: Element) -> Element {
    let watchers = use_game().state.read().meta.watchers;
    rsx! {
        div { class: "seat-bar",
            span { class: "game-title", "{title}" }
            {children}
            if watchers > 0 {
                span { class: "muted", "data-watchers": "{watchers}", title: "People watching this game", "{watchers} watching" }
            }
            ViewToggle { table_on }
            menu::GameMenu {}
        }
    }
}

/// The game of the [`GameCtx`] provided, on the table or as plain text under its bar:
/// the one page every source of games is shown through. `who` says whose view it is and
/// switches it; `extra` is what only this source has (an official account's panel).
#[component]
pub fn GameShell(title: String, who: Element, extra: Element) -> Element {
    let table_on = use_signal(table::table_mode);
    let bar = rsx! {
        GameBar { title, table_on, {who} }
    };
    if table_on() {
        return rsx! {
            table::TableGame { controls: rsx! { {bar} {extra} } }
        };
    }
    rsx! {
        div { class: "game-page",
            {bar}
            {extra}
            WaitingNotice {}
            plain::PlainGame {}
            menu::ResignDialog {}
        }
    }
}

#[component]
pub fn GamePage(id: String) -> Element {
    let game = use_signal(|| id.clone());
    let mut seats = use_signal(|| session::seats(&id));
    let mut seat = use_signal(|| seats.peek().first().cloned());
    let watch = use_signal(|| false);
    let mut watch_as = use_signal(|| None::<u8>);
    let finding = use_signal(|| false);
    let refused = use_signal(|| None::<String>);
    let mut state = use_signal(GameState::default);
    let deciding = use_memo(move || state.read().deciding);
    let watching = use_memo(move || seat.read().is_none() && watch());
    let mut follow = use_signal(|| true);
    let mut join_error = use_signal(|| None::<String>);
    let mut driver = use_signal(|| None::<dioxus::core::Task>);

    let live = Live {
        game,
        seats,
        seat,
        watch,
        watch_as,
        finding,
        finds: use_signal(|| 0),
        refused,
        state,
    };
    let account = use_account();
    let summary = use_summary(game);
    use_context_provider(|| summary);
    let offers = use_memo(move || Offers {
        chat: true,
        transcript: has_agents(&state.read().meta).then(|| game.read().clone()),
        transcript_at: None,
    });
    use_context_provider(|| GameCtx {
        game,
        seat,
        watching,
        state,
        deciding,
        submit: Callback::new(move |answer: Answer| {
            spawn(send_answer(live, answer, None));
        }),
        submit_way: Callback::new(move |(answer, way): (Answer, String)| {
            spawn(send_answer(live, answer, Some(way)));
        }),
        undo: Callback::new(move |()| {
            spawn(send_undo(live));
        }),
        answer_undo: Callback::new(move |allow: bool| {
            spawn(send_answer_undo(live, Some(allow)));
        }),
        cancel_undo: Callback::new(move |()| {
            spawn(send_answer_undo(live, None));
        }),
        resign: Callback::new(move |()| {
            spawn(send_resign(live));
        }),
        resigning: Signal::new(false),
        offers,
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
        } else if seat.peek().is_none() {
            spawn(find_role(live));
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
        let via = match seat() {
            Some(s) => Some(Via::Seat(s.token)),
            None => watch().then(|| Via::Watch(watch_as())),
        };
        if let Some(t) = driver.take() {
            t.cancel();
        }
        state.set(GameState::default());
        if let Some(via) = via {
            driver.set(Some(spawn(drive(live, via))));
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
    let shown = !held.is_empty() || watching();
    let player_names = match summary.read().as_ref() {
        Some(g) => g.names.clone(),
        None => state
            .read()
            .view
            .as_ref()
            .map(logfmt::names)
            .unwrap_or_default(),
    };
    let mut paste = use_signal(String::new);
    let table_on = use_signal(table::table_mode);
    let title = format!("Game {game}");
    let who = rsx! {
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
            } else if watching() {
                span { class: "watching-mark", "data-watching": "1", "watching" }
                span { class: "watch-as", "data-watch-as": "{watch_as().map(i32::from).unwrap_or(-1)}",
                    span { class: "muted", "as:" }
                    button {
                        class: if watch_as().is_none() { "toggle on" } else { "toggle" },
                        title: "Every player's cards",
                        onclick: move |_| watch_as.set(None),
                        "Everyone"
                    }
                    for (i, name) in player_names.iter().cloned().enumerate() {
                        {
                            let i = i as u8;
                            rsx! {
                                button {
                                    key: "{i}",
                                    class: if watch_as() == Some(i) { "toggle on" } else { "toggle" },
                                    title: "What {name} knows",
                                    onclick: move |_| watch_as.set(Some(i)),
                                    "{name}"
                                }
                            }
                        }
                    }
                }
            }
    };
    let seat_bar = rsx! {
        GameBar { title: title.clone(), table_on, {who.clone()} }
    };
    if let Some(e) = gone() {
        let id = game.read().clone();
        return rsx! {
            div { class: "game-page",
                {seat_bar}
                section { class: "panel", "data-gone": "1",
                    h2 { "This game can't be opened" }
                    p { "The server said: {e}." }
                    if watching() {
                        p { "It may have been started on a server that didn't keep its games." }
                    } else {
                        p { "It may have been started on a server that didn't keep its games, or this browser's seat token for game {id} is no longer valid. This browser has forgotten that seat." }
                    }
                    Link { class: "button primary", to: crate::app::Route::Lobby {}, "Back to the lobby" }
                }
            }
        };
    }
    if shown {
        return rsx! {
            GameShell {
                title,
                who,
                extra: rsx! {
                    if let Some(e) = join_error() {
                        p { class: "error", "{e}" }
                    }
                },
            }
        };
    }
    rsx! {
        div { class: "game-page",
            {seat_bar}
            if finding() {
                p { class: "muted", "data-finding": "1", "Looking for your seat…" }
            } else {
                section { class: "panel", "data-no-seat": "1",
                    h2 { "Take a seat" }
                    if let Some(why) = refused() {
                        p { "You have no seat in this game, and the server said: {why}." }
                    } else {
                        p { "This browser holds no seat in this game." }
                    }
                    if account.name().is_none() {
                        div { class: "row",
                            Link { class: "button primary", to: crate::app::Route::Lobby {}, "Log in" }
                        }
                    }
                    p { "Or paste a seat token:" }
                    div { class: "row",
                        input {
                            class: "grow",
                            placeholder: "seat token",
                            value: "{paste}",
                            oninput: move |e| paste.set(e.value()),
                        }
                        button { class: "copper", onclick: move |_| take_token(paste().trim().to_string()), "Join" }
                    }
                    if let Some(e) = join_error() {
                        p { class: "error", "{e}" }
                    }
                }
            }
        }
    }
}
