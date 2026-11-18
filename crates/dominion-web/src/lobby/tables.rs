//! Tables: the lobby's tables as dominion.games lists them (host, seats, status, and
//! Join where a seat is open), then the running games played without one, with Play for a
//! seat held here and Watch where watching is allowed. Finished games are History's.
//! A running game's seats show whose turn it is and, where anyone may watch it, each
//! seat's VP, with the cards it owns while hovered.

use super::avatar::Avatar;
use super::history::{SeatIcon, agent_line};
use super::home::TableStarter;
use super::rules::TableStatus;
use super::{LobbyFrame, Tab, join_table, use_lobby, use_toast};
use crate::account::use_account;
use crate::api;
use crate::app::{Route, use_cards};
use crate::card::{CardSize, CardView};
use crate::game::board::card_order;
use crate::platform;
use crate::session;
use dioxus::prelude::*;
use dominion_protocol::api::{GameSummary, Progress};
use dominion_protocol::lobby::{
    Lobby, PlayedBy, SeatAgent, SeatKind, Source, TableSpec, UserKind, UserRef,
};

const REFRESH_MS: u32 = 3000;

/// Who sits in a seat, as its icon shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SeatMark {
    You,
    Player(String),
    /// A seat played by token.
    Token(String),
    /// One of the server's agents.
    Agent(String),
    Open,
}

/// What a line's button does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// Sit down at the table.
    Join,
    /// Go to My Table: it is yours.
    Open,
    /// Go to the game: this browser holds a seat in it.
    Play,
    Watch,
    None,
}

impl Action {
    fn label(self) -> &'static str {
        match self {
            Action::Join => "Join",
            Action::Open => "Open",
            Action::Play => "Play",
            Action::Watch => "Watch",
            Action::None => "",
        }
    }
}

/// A seat on a line: who sits in it, and what draws their face.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Seat {
    pub mark: SeatMark,
    pub user: Option<UserRef>,
    pub agent: Option<SeatAgent>,
}

impl Seat {
    fn of(mark: SeatMark) -> Seat {
        Seat {
            mark,
            user: None,
            agent: None,
        }
    }

    pub fn name(&self) -> Option<&str> {
        match &self.mark {
            SeatMark::You => self.user.as_ref().map(|u| u.name.as_str()),
            SeatMark::Player(n) | SeatMark::Token(n) | SeatMark::Agent(n) => Some(n),
            SeatMark::Open => None,
        }
    }

    /// What its face says while hovered.
    fn title(&self) -> String {
        let name = self.name().unwrap_or_default();
        match (&self.mark, &self.agent, &self.user) {
            (SeatMark::Open, _, _) => "Empty Slot".into(),
            (_, Some(a), _) => format!("{name}: {}", agent_line(a)),
            (SeatMark::You, _, _) => "You".into(),
            (_, _, Some(u)) if u.kind == UserKind::Bot => match &u.owner {
                Some(owner) => format!("{name}: {owner}'s bot"),
                None => format!("{name}: a bot"),
            },
            (SeatMark::Token(_), _, _) => format!("{name}: played with a seat token"),
            _ => name.to_string(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Line {
    /// A table's host; a game's players.
    pub title: String,
    /// What its first column shows: a table's host, or for an official account's game,
    /// dominion.games (a game's seats name its players).
    pub label: String,
    pub table: Option<String>,
    pub game: Option<String>,
    pub seats: Vec<Seat>,
    pub status: TableStatus,
    pub note: String,
    pub action: Action,
    /// The official account whose dominion.games game this is.
    pub official: Option<String>,
    /// Where its running game stands.
    pub progress: Option<Progress>,
    /// The seat its running game waits for.
    pub deciding: Option<u8>,
}

impl Line {
    pub fn marks(&self) -> Vec<SeatMark> {
        self.seats.iter().map(|s| s.mark.clone()).collect()
    }

    fn key(&self) -> String {
        match &self.official {
            Some(account) => format!("official-{account}"),
            None => (self.table.clone().or(self.game.clone())).unwrap_or_default(),
        }
    }
}

/// A running game's seats: each one's account, agent or token.
fn game_seats(g: &GameSummary, me: Option<&str>) -> Vec<Seat> {
    (g.names.iter().enumerate())
        .map(|(i, name)| {
            let user = g.users.get(i).cloned().flatten();
            let agent = g.agents.get(i).cloned().flatten();
            let mark = match (&user, &agent) {
                (Some(u), _) if Some(u.name.as_str()) == me => SeatMark::You,
                (_, Some(_)) => SeatMark::Agent(name.clone()),
                (Some(u), None) => SeatMark::Player(u.name.clone()),
                (None, None) => SeatMark::Token(name.clone()),
            };
            Seat { mark, user, agent }
        })
        .collect()
}

/// A game's kingdom as kingdom text.
pub fn kingdom_code(game: &GameSummary) -> String {
    let names = game.kingdom.iter().chain(&game.landscapes);
    let names: Vec<String> = names.map(|n| n.to_lowercase()).collect();
    names.join(", ")
}

/// What everyone sees of a table's rules, in a few words.
pub fn spec_note(spec: &TableSpec) -> String {
    let mut parts = vec![if spec.rated { "Rated" } else { "Unrated" }.to_string()];
    if let Some(clock) = spec.clock {
        parts.push(format!("clock {}", clock.label()));
    } else if !spec.timed {
        parts.push("untimed".into());
    }
    let chosen = spec.kingdom.len() + spec.landscapes.len();
    if chosen > 0 {
        parts.push(format!("{chosen} chosen"));
    }
    if !spec.sets.is_empty() {
        parts.push(spec.sets.join(", "));
    }
    parts.join(" · ")
}

/// The Tables tab's lines: every table, then the running games no listed table is
/// playing (`held` says which this browser holds a seat in).
pub fn listing(lobby: &Lobby, games: &[GameSummary], held: &dyn Fn(&str) -> bool) -> Vec<Line> {
    let me = lobby.you.as_ref().map(|y| y.user.name.as_str());
    let mine = lobby.table.as_ref().map(|t| t.id.as_str());
    let watchable = |game: &str| games.iter().any(|g| g.game_id == game && g.watchable);
    let running = |game: &Option<String>| {
        let game = game.as_deref()?;
        games.iter().find(|g| g.game_id == game && g.running)
    };
    let mut lines: Vec<Line> = vec![];
    for t in &lobby.tables {
        let summary = running(&t.game);
        let seats = match summary {
            Some(g) => game_seats(g, me),
            None => (t.seats.iter())
                .map(|s| {
                    let mark = match (s.kind, &s.user) {
                        (SeatKind::Player, Some(u)) if Some(u.name.as_str()) == me => SeatMark::You,
                        (SeatKind::Player, Some(u)) => SeatMark::Player(u.name.clone()),
                        (SeatKind::Token, _) => SeatMark::Token(s.name.clone().unwrap_or_default()),
                        (SeatKind::Agent, Some(u)) => SeatMark::Agent(u.name.clone()),
                        _ => SeatMark::Open,
                    };
                    let agent = (s.kind == SeatKind::Agent).then(|| SeatAgent {
                        agent: s.user.as_ref().map(|u| u.name.clone()).unwrap_or_default(),
                        ..SeatAgent::default()
                    });
                    Seat {
                        mark,
                        user: s.user.clone(),
                        agent,
                    }
                })
                .collect(),
        };
        let open = t.seats.iter().any(|s| s.kind == SeatKind::Open);
        let action = match &t.game {
            _ if Some(t.id.as_str()) == mine => Action::Open,
            Some(game) if held(game) => Action::Play,
            Some(game) if watchable(game) => Action::Watch,
            None if open => Action::Join,
            _ => Action::None,
        };
        lines.push(Line {
            title: t.host.clone(),
            label: t.host.clone(),
            table: Some(t.id.clone()),
            game: t.game.clone(),
            seats,
            status: TableStatus::of(t),
            note: spec_note(&t.spec),
            action,
            official: None,
            progress: summary.and_then(|g| g.progress.clone()),
            deciding: summary.and_then(|g| g.deciding),
        });
    }
    let at_table = |game: &str| {
        let of = |g: &Option<String>| g.as_deref() == Some(game);
        lobby.tables.iter().any(|t| of(&t.game) || of(&t.last_game))
    };
    for g in games {
        if !g.running || at_table(&g.game_id) {
            continue;
        }
        let seats = game_seats(g, me);
        // Seats played by token are the usual case: only another origin is worth saying.
        let source = match g.source {
            Source::Automatch => Some("Automatch"),
            Source::Table => Some("Table"),
            Source::Direct => None,
            Source::Official => Some("dominion.games"),
        };
        let rated = if g.rated { "Rated" } else { "Unrated" };
        lines.push(Line {
            title: g.names.join(", "),
            label: String::new(),
            table: None,
            game: Some(g.game_id.clone()),
            seats,
            status: TableStatus::Running,
            note: match source {
                Some(source) => format!("{source} · {rated}"),
                None => rated.into(),
            },
            action: if held(&g.game_id) {
                Action::Play
            } else if g.watchable {
                Action::Watch
            } else {
                Action::None
            },
            official: None,
            progress: g.progress.clone(),
            deciding: g.deciding,
        });
    }
    for a in &lobby.official {
        let Some(g) = a.game.as_ref().filter(|g| g.running) else {
            continue;
        };
        let seats = (g.players.iter().enumerate())
            .map(|(i, name)| match &a.agent {
                Some(agent) if g.seat == Some(i as u8) => Seat {
                    agent: Some(SeatAgent {
                        agent: agent.clone(),
                        ..SeatAgent::default()
                    }),
                    ..Seat::of(SeatMark::Agent(agent.clone()))
                },
                _ => Seat::of(SeatMark::Player(name.clone())),
            })
            .collect();
        let rated = if g.rated { "rated" } else { "unrated" };
        lines.push(Line {
            title: g.players.join(", "),
            label: "dominion.games".into(),
            table: None,
            game: Some(g.id.clone()),
            seats,
            status: TableStatus::Running,
            note: format!("As {} · {rated}", a.account),
            action: Action::Watch,
            official: Some(a.id.clone()),
            progress: None,
            deciding: None,
        });
    }
    lines
}

#[component]
pub fn TablesTab() -> Element {
    let ctx = use_lobby();
    let account = use_account();
    let toast = use_toast();
    let nav = navigator();
    let mut games = use_signal(|| None::<Result<Vec<GameSummary>, String>>);
    let load = move || async move {
        let listed = api::games().await.map_err(|e| e.to_string());
        if *games.peek() != Some(listed.clone()) {
            games.set(Some(listed));
        }
    };
    use_future(move || async move {
        loop {
            load().await;
            platform::sleep(REFRESH_MS).await;
        }
    });
    // A table's game starting or ending changes the lobby.
    use_effect(move || {
        let _ = ctx.lobby.read();
        spawn(load());
    });
    let mut host = use_signal(String::new);
    let mut players = use_signal(|| [true; 3]);
    let mut statuses = use_signal(|| [true; 3]);
    let mut sort = use_signal(|| 0u8);
    let peek = use_signal(|| None::<Peek>);

    let held = session::all();
    let listed = games().and_then(|g| g.ok()).unwrap_or_default();
    let mut lines = match ctx.lobby.read().as_ref() {
        Some(lobby) => listing(lobby, &listed, &|game| held.contains_key(game)),
        None => vec![],
    };
    let error = match games() {
        Some(Err(e)) => Some(e),
        _ => None,
    };
    let prefix = host().to_lowercase();
    let p = players();
    let st = statuses();
    lines.retain(|l| {
        let size = match l.seats.len() {
            2 => p[0],
            3 => p[1],
            _ => p[2],
        };
        let status = st[TableStatus::ALL
            .iter()
            .position(|s| *s == l.status)
            .unwrap_or(0)];
        l.title.to_lowercase().starts_with(&prefix) && size && status
    });
    match sort() {
        1 => lines.sort_by(|a, b| {
            a.seats
                .len()
                .cmp(&b.seats.len())
                .then(a.title.cmp(&b.title))
        }),
        2 => lines.sort_by(|a, b| {
            b.seats
                .len()
                .cmp(&a.seats.len())
                .then(a.title.cmp(&b.title))
        }),
        _ => lines.sort_by_key(|l| (l.table.is_none(), l.title.to_lowercase())),
    }
    let sort_class = ["lb-sorter", "lb-sorter sort-up", "lb-sorter sort-down"][sort() as usize];
    let empty = lines.is_empty();
    let peeked = lines.clone();
    rsx! {
        LobbyFrame { tab: Tab::Tables,
            div { class: "lb-tables-page",
                TableStarter {}
                div { class: "lb-table-line lb-sort-line",
                    input {
                        class: "lb-host-input",
                        placeholder: "Host",
                        value: "{host}",
                        oninput: move |e| host.set(e.value()),
                    }
                    div {
                        class: "lb-refresh",
                        title: "Refresh",
                        onclick: move |_| {
                            spawn(async move {
                                ctx.refresh().await;
                                load().await;
                            });
                        },
                    }
                    div { class: sort_class, title: "Sort", onclick: move |_| sort.set((sort() + 1) % 3) }
                    for (i, label) in ["2P", "3P", "4P+"].into_iter().enumerate() {
                        button {
                            key: "{label}",
                            class: if p[i] { "lb-oval active" } else { "lb-oval" },
                            onclick: move |_| players.write()[i] ^= true,
                            "{label}"
                        }
                    }
                    for (i, s) in TableStatus::ALL.into_iter().enumerate() {
                        button {
                            key: "{s.label()}",
                            class: if st[i] { "lb-oval active" } else { "lb-oval" },
                            onclick: move |_| statuses.write()[i] ^= true,
                            "{s.label()}"
                        }
                    }
                }
                if let Some(e) = error {
                    p { class: "lb-error", "{e}" }
                }
                div { class: "lb-tables-list",
                    if empty {
                        p { class: "lb-note lb-tables-empty", "No tables. Create one, or search for a game on the Automatch tab." }
                    }
                    for l in lines {
                        {
                            let key = l.key();
                            let (table, game, action) = (l.table.clone(), l.game.clone(), l.action);
                            let official = l.official.clone();
                            let turn = l.progress.as_ref().map(turn_text);
                            rsx! {
                                div {
                                    key: "{key}",
                                    class: if l.progress.is_some() { "lb-table-line running" } else { "lb-table-line" },
                                    "data-table": l.table.clone().unwrap_or_default(),
                                    "data-game": l.game.clone().unwrap_or_default(),
                                    title: l.game.clone().map(|i| format!("Game {i}")).unwrap_or_default(),
                                    div { class: "lb-table-host", title: "{l.title}", "{l.label}" }
                                    div { class: "lb-table-players",
                                        for (s, seat) in l.seats.iter().enumerate() {
                                            SeatChip {
                                                key: "{s}",
                                                seat: seat.clone(),
                                                index: s as u8,
                                                line: key.clone(),
                                                progress: l.progress.clone(),
                                                deciding: l.deciding,
                                                peek,
                                            }
                                        }
                                    }
                                    div { class: "lb-table-status",
                                        match turn {
                                            Some(turn) => rsx! { span { class: "lb-table-state lb-table-turn", "{turn}" } },
                                            None => rsx! { span { class: "lb-table-state", "{l.status.label()}" } },
                                        }
                                        span { class: "lb-table-rules", "{l.note}" }
                                    }
                                    if action != Action::None {
                                        div {
                                            class: "lb-oval lb-table-play",
                                            "data-action": "{action.label().to_lowercase()}",
                                            onclick: move |_| {
                                                let (table, game) = (table.clone(), game.clone());
                                                if let Some(account) = official.clone() {
                                                    nav.push(Route::OnlineAccount { account });
                                                    return;
                                                }
                                                match (action, table, game) {
                                                    (Action::Join, Some(id), _) => {
                                                        spawn(async move {
                                                            if join_table(ctx, account, toast, nav, id).await {
                                                                nav.push(Route::MyTable {});
                                                            }
                                                        });
                                                    }
                                                    (Action::Open, _, _) => { nav.push(Route::MyTable {}); }
                                                    (Action::Play | Action::Watch, _, Some(id)) => { nav.push(Route::Game { id }); }
                                                    _ => {}
                                                }
                                            },
                                            "{action.label()}"
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                if let Some((line, seat, at)) = peek() {
                    if let Some(l) = peeked.iter().find(|l| l.key() == line) {
                        DeckPeek { line: l.clone(), seat, at }
                    }
                }
            }
        }
    }
}

/// The seat whose cards are shown: its line's key, its index, and where the pointer
/// entered it.
type Peek = (String, u8, (f64, f64));

/// "Turn 7 · Buy".
fn turn_text(p: &Progress) -> String {
    let mut phase = p.phase.clone();
    if let Some(first) = phase.get_mut(..1) {
        first.make_ascii_uppercase();
    }
    match p.phase.as_str() {
        "" | "start" | "end" => format!("Turn {}", p.turn),
        "cleanup" => format!("Turn {} · Clean-up", p.turn),
        _ => format!("Turn {} · {phase}", p.turn),
    }
}

/// A seat's face: the account's picture, an agent's maker's logo, a bot, a token or an
/// open seat.
#[component]
fn SeatFace(seat: Seat) -> Element {
    let icon = match (&seat.mark, &seat.agent, &seat.user) {
        (SeatMark::Open, _, _) => None,
        (_, Some(a), _) => Some(PlayedBy::Agent(a.clone())),
        (_, _, Some(u)) if u.kind == UserKind::Bot => Some(PlayedBy::Bot {
            owner: u.owner.clone(),
        }),
        (_, _, Some(u)) => {
            return rsx! { Avatar { user: u.clone(), size: "var(--seat-face)" } };
        }
        (SeatMark::Token(_), _, _) => Some(PlayedBy::Token),
        _ => Some(PlayedBy::Person),
    };
    match icon {
        Some(by) => rsx! {
            span { class: "lb-tseat-icon", SeatIcon { played_by: by } }
        },
        None => rsx! { span { class: "lb-tseat-icon open" } },
    }
}

/// A seat on a line: its face, and in a running game its name, VP and whether it is that
/// seat's turn. Hovering one whose cards are known shows them.
#[component]
fn SeatChip(
    seat: Seat,
    index: u8,
    line: String,
    progress: Option<Progress>,
    deciding: Option<u8>,
    peek: Signal<Option<Peek>>,
) -> Element {
    let Some(p) = progress else {
        let title = seat.title();
        return rsx! {
            div { class: "lb-tseat", title, SeatFace { seat } }
        };
    };
    let turn = p.player == index;
    let waits = deciding == Some(index) && !turn;
    let known = p.seats.get(index as usize).cloned();
    let mut class = String::from("lb-tseat named");
    if turn {
        class.push_str(" turn");
    }
    if waits {
        class.push_str(" deciding");
    }
    let mut title = seat.title();
    if turn {
        title.push_str(" — their turn");
    } else if waits {
        title.push_str(" — deciding");
    }
    let name = seat.name().unwrap_or_default().to_string();
    let has_cards = known.is_some();
    rsx! {
        div {
            class,
            title: if has_cards { String::new() } else { title.clone() },
            "data-seat": "{index}",
            onmouseenter: move |e: MouseEvent| {
                if has_cards {
                    let c = e.client_coordinates();
                    peek.set(Some((line.clone(), index, (c.x, c.y))));
                }
            },
            onmouseleave: move |_| {
                let mut peek = peek;
                peek.set(None);
            },
            SeatFace { seat: seat.clone() }
            span { class: "lb-tseat-name", "{name}" }
            if let Some(k) = known {
                span { class: "lb-tseat-vp", title: "Victory points if the game ended now", "{k.vp}" }
            }
        }
    }
}

/// The cards a seat owns, beside the pointer, in dominion.games' order, as of the latest
/// listing.
#[component]
fn DeckPeek(line: Line, seat: u8, at: (f64, f64)) -> Element {
    let cards = use_cards();
    let Some(p) = line.progress.as_ref() else {
        return rsx! {};
    };
    let Some(known) = p.seats.get(seat as usize) else {
        return rsx! {};
    };
    let who = line.seats.get(seat as usize);
    let name = who.and_then(Seat::name).unwrap_or_default().to_string();
    let what = who.map(Seat::title).unwrap_or_default();
    let mut what = (what.strip_prefix(&format!("{name}: ")))
        .unwrap_or(&what)
        .to_string();
    if let Some(first) = what.get_mut(..1) {
        first.make_ascii_uppercase();
    }
    let mut owned: Vec<(&String, &u32)> = known.owned.iter().collect();
    owned.sort_by(|a, b| card_order(&cards, a.0, b.0));
    let total: u32 = known.owned.values().sum();
    let (vw, vh) = platform::viewport();
    let rows = owned.len().div_ceil(2) as f64;
    let (width, height) = (vw * 0.26, vh * (0.09 + rows * 0.034));
    let (x, y) = at;
    let left = if x + 16.0 + width > vw {
        (x - 16.0 - width).max(4.0)
    } else {
        x + 16.0
    };
    let top = (y - height / 2.0).clamp(4.0, (vh - height - 4.0).max(4.0));
    let turn = (p.player == seat).then(|| turn_text(p));
    rsx! {
        div { class: "lb-deck-peek", style: "left: {left}px; top: {top}px; width: {width}px;",
            div { class: "lb-deck-peek-head",
                span { class: "lb-deck-peek-name", "{name}" }
                span { class: "lb-deck-peek-sum", "{known.vp} VP · {total} cards" }
            }
            if what != name {
                div { class: "lb-deck-peek-what", "{what}" }
            }
            if let Some(turn) = turn {
                div { class: "lb-deck-peek-what", "Their turn: {turn}" }
            }
            div { class: "lb-deck-peek-cards",
                for (card, n) in owned {
                    div { key: "{card}", class: "lb-deck-peek-row",
                        span { class: "lb-deck-peek-count", "{n}" }
                        div { class: "lb-deck-peek-strip",
                            CardView { name: card.clone(), size: CardSize::Micro }
                        }
                    }
                }
            }
        }
    }
}
