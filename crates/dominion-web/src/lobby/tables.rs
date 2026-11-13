//! Tables: the lobby's tables as dominion.games lists them (host, seats, status, and
//! Join where a seat is open), then the games played without one, with Play for a seat
//! held here and Watch where watching is allowed.

use super::home::TableStarter;
use super::rules::TableStatus;
use super::{LobbyFrame, Tab, join_table, use_lobby, use_toast};
use crate::account::use_account;
use crate::api;
use crate::app::Route;
use crate::platform;
use crate::session;
use dioxus::prelude::*;
use dominion_protocol::api::GameSummary;
use dominion_protocol::lobby::{Lobby, SeatKind, Source, TableSpec};

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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Line {
    /// A table's host; a game's players.
    pub title: String,
    pub table: Option<String>,
    pub game: Option<String>,
    pub seats: Vec<SeatMark>,
    pub status: TableStatus,
    pub note: String,
    pub action: Action,
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
    if !spec.timed {
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
/// playing and the finished ones this browser holds a seat in (`held` says which).
pub fn listing(lobby: &Lobby, games: &[GameSummary], held: &dyn Fn(&str) -> bool) -> Vec<Line> {
    let me = lobby.you.as_ref().map(|y| y.user.name.as_str());
    let mine = lobby.table.as_ref().map(|t| t.id.as_str());
    let watchable = |game: &str| games.iter().any(|g| g.game_id == game && g.watchable);
    let mut lines: Vec<Line> = vec![];
    for t in &lobby.tables {
        let seats = (t.seats.iter())
            .map(|s| match (s.kind, &s.user) {
                (SeatKind::Player, Some(u)) if Some(u.name.as_str()) == me => SeatMark::You,
                (SeatKind::Player, Some(u)) => SeatMark::Player(u.name.clone()),
                (SeatKind::Token, _) => SeatMark::Token(s.name.clone().unwrap_or_default()),
                (SeatKind::Agent, Some(u)) => SeatMark::Agent(u.name.clone()),
                _ => SeatMark::Open,
            })
            .collect();
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
            table: Some(t.id.clone()),
            game: t.game.clone(),
            seats,
            status: TableStatus::of(t),
            note: spec_note(&t.spec),
            action,
        });
    }
    let at_table = |game: &str| {
        let of = |g: &Option<String>| g.as_deref() == Some(game);
        lobby.tables.iter().any(|t| of(&t.game) || of(&t.last_game))
    };
    for g in games {
        let holds = held(&g.game_id);
        if at_table(&g.game_id) || !(g.running || holds) {
            continue;
        }
        let seats = (g.names.iter().enumerate())
            .map(|(i, name)| match g.users.get(i).cloned().flatten() {
                Some(u) if Some(u.name.as_str()) == me => SeatMark::You,
                Some(u) => SeatMark::Player(u.name),
                None => SeatMark::Token(name.clone()),
            })
            .collect();
        let source = match g.source {
            Source::Automatch => "Automatch",
            Source::Table => "Table",
            Source::Direct => "By token",
        };
        let rated = if g.rated { "rated" } else { "unrated" };
        lines.push(Line {
            title: g.names.join(", "),
            table: None,
            game: Some(g.game_id.clone()),
            seats,
            status: if g.running {
                TableStatus::Running
            } else {
                TableStatus::PostGame
            },
            note: format!("{source} · {rated}"),
            action: if holds {
                Action::Play
            } else if g.watchable {
                Action::Watch
            } else {
                Action::None
            },
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
                            let key = l.table.clone().or(l.game.clone()).unwrap_or_default();
                            let (table, game, action) = (l.table.clone(), l.game.clone(), l.action);
                            rsx! {
                                div {
                                    key: "{key}",
                                    class: "lb-table-line",
                                    "data-table": l.table.clone().unwrap_or_default(),
                                    "data-game": l.game.clone().unwrap_or_default(),
                                    title: l.game.clone().map(|i| format!("Game {i}")).unwrap_or_default(),
                                    div { class: "lb-table-host", "{l.title}" }
                                    div { class: "lb-table-players",
                                        for (s, mark) in l.seats.iter().enumerate() {
                                            {
                                                let (class, title) = match mark {
                                                    SeatMark::You => ("lb-seat-icon", "You".to_string()),
                                                    SeatMark::Player(name) => ("lb-seat-icon other", name.clone()),
                                                    SeatMark::Token(name) => ("lb-seat-icon token", format!("{name}: played with a seat token")),
                                                    SeatMark::Agent(name) => ("lb-seat-icon token agent", format!("{name}: one of the server's agents")),
                                                    SeatMark::Open => ("lb-seat-icon open", "Empty Slot".to_string()),
                                                };
                                                rsx! { div { key: "{s}", class, title } }
                                            }
                                        }
                                    }
                                    div { class: "lb-table-status",
                                        "{l.status.label()}"
                                        span { class: "lb-table-rules", "{l.note}" }
                                    }
                                    if action != Action::None {
                                        div {
                                            class: "lb-oval lb-table-play",
                                            "data-action": "{action.label().to_lowercase()}",
                                            onclick: move |_| {
                                                let (table, game) = (table.clone(), game.clone());
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
            }
        }
    }
}
