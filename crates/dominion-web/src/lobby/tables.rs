//! Tables: this server's games as dominion.games lists tables (host, seats, status,
//! Play), with its filter line; this browser's own table while no game has started from
//! it is the one "New" line.

use super::home::TableStarter;
use super::{LobbyFrame, Tab, use_table};
use crate::api;
use crate::app::Route;
use crate::platform;
use crate::session;
use dioxus::prelude::*;
use dominion_protocol::api::GameSummary;

const REFRESH_MS: u32 = 3000;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Status {
    New,
    Running,
    PostGame,
}

impl Status {
    fn label(self) -> &'static str {
        match self {
            Status::New => "New",
            Status::Running => "Running",
            Status::PostGame => "Post-game",
        }
    }
}

const STATUSES: [Status; 3] = [Status::New, Status::Running, Status::PostGame];

struct Line {
    host: String,
    id: Option<String>,
    seats: usize,
    held: usize,
    status: Status,
    playable: bool,
}

#[component]
pub fn TablesTab() -> Element {
    let table = use_table();
    let nav = navigator();
    let mut games = use_signal(|| None::<Result<Vec<GameSummary>, String>>);
    let mut refresh = use_signal(|| 0u32);
    use_future(move || async move {
        loop {
            games.set(Some(api::games().await.map_err(|e| e.to_string())));
            platform::sleep(REFRESH_MS).await;
        }
    });
    use_effect(move || {
        if refresh() > 0 {
            spawn(async move {
                games.set(Some(api::games().await.map_err(|e| e.to_string())));
            });
        }
    });
    let mut host = use_signal(String::new);
    let mut players = use_signal(|| [true; 3]);
    let mut statuses = use_signal(|| [true; 3]);
    let mut sort = use_signal(|| 0u8);

    let held = session::all();
    let mut lines: Vec<Line> = vec![];
    if let Some(t) = table.read().as_ref()
        && t.game.is_none()
    {
        lines.push(Line {
            host: t
                .rules
                .names()
                .ok()
                .and_then(|n| n.into_iter().next())
                .unwrap_or_default(),
            id: None,
            seats: t.rules.seats.len(),
            held: t.rules.seats.iter().filter(|s| s.plays_here()).count(),
            status: Status::New,
            playable: true,
        });
    }
    let error = match games() {
        Some(Err(e)) => Some(e),
        _ => None,
    };
    for g in games().and_then(|g| g.ok()).unwrap_or_default() {
        let mine = held.get(&g.game_id).map(|s| s.len()).unwrap_or(0);
        lines.push(Line {
            host: g.names.first().cloned().unwrap_or_default(),
            id: Some(g.game_id.clone()),
            seats: g.names.len(),
            held: mine,
            status: if g.running {
                Status::Running
            } else {
                Status::PostGame
            },
            playable: mine > 0,
        });
    }
    let prefix = host().to_lowercase();
    let p = players();
    let st = statuses();
    lines.retain(|l| {
        let size = match l.seats {
            2 => p[0],
            3 => p[1],
            _ => p[2],
        };
        let status = st[STATUSES.iter().position(|s| *s == l.status).unwrap_or(0)];
        l.host.to_lowercase().starts_with(&prefix) && size && status
    });
    match sort() {
        1 => lines.sort_by(|a, b| a.seats.cmp(&b.seats).then(a.host.cmp(&b.host))),
        2 => lines.sort_by(|a, b| b.seats.cmp(&a.seats).then(a.host.cmp(&b.host))),
        _ => lines.sort_by_key(|l| l.host.to_lowercase()),
    }
    let sort_class = ["lb-sorter", "lb-sorter sort-up", "lb-sorter sort-down"][sort() as usize];
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
                    div { class: "lb-refresh", title: "Refresh", onclick: move |_| refresh += 1 }
                    div { class: sort_class, title: "Sort", onclick: move |_| sort.set((sort() + 1) % 3) }
                    for (i, label) in ["2P", "3P", "4P+"].into_iter().enumerate() {
                        button {
                            key: "{label}",
                            class: if p[i] { "lb-oval active" } else { "lb-oval" },
                            onclick: move |_| players.write()[i] ^= true,
                            "{label}"
                        }
                    }
                    for (i, s) in STATUSES.into_iter().enumerate() {
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
                    for (i, l) in lines.into_iter().enumerate() {
                        {
                            let id = l.id.clone();
                            rsx! {
                                div {
                                    key: "{i}{l.host}",
                                    class: "lb-table-line",
                                    "data-game": l.id.clone().unwrap_or_default(),
                                    title: l.id.clone().map(|i| format!("Game {i}")).unwrap_or_default(),
                                    div { class: "lb-table-host", "{l.host}" }
                                    div { class: "lb-table-players",
                                        for s in 0..l.seats {
                                            div {
                                                key: "{s}",
                                                class: if s < l.held { "lb-seat-icon" } else { "lb-seat-icon other" },
                                            }
                                        }
                                    }
                                    div { class: "lb-table-status", "{l.status.label()}" }
                                    div {
                                        class: if l.playable { "lb-oval lb-table-play" } else { "lb-oval lb-table-play disabled" },
                                        "data-action": "play",
                                        onclick: move |_| {
                                            if !l.playable {
                                                return;
                                            }
                                            match &id {
                                                Some(id) => { nav.push(Route::Game { id: id.clone() }); }
                                                None => { nav.push(Route::MyTable {}); }
                                            }
                                        },
                                        "Play"
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
