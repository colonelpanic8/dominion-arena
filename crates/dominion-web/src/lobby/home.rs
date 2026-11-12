//! The Automatch tab: dominion.games' landing tab. Searching for a game against whoever
//! else is searching, Bot Game (a search for agents), the table starter, joining a seat
//! by its token, the lobby's chat, and the sign-in panel while nobody is signed in.

use super::account::SignIn;
use super::chat::Chat;
use super::{
    Fieldset, LobbyFrame, OptionRow, Switch, Tab, create_table, flash, now_ms, signed_in, store,
    use_lobby, use_toast,
};
use crate::account::use_account;
use crate::api;
use crate::app::Route;
use crate::platform;
use crate::session::{self, HeldSeat};
use dioxus::prelude::*;
use dominion_protocol::lobby::{MatchRequest, Opponents, QueueState};

#[component]
pub fn AutomatchTab() -> Element {
    let account = use_account();
    let signed_out = *account.loaded.read() && account.me.read().is_none();
    rsx! {
        LobbyFrame { tab: Tab::Automatch,
            div { class: "lb-columns",
                div { class: "lb-column",
                    Automatch {}
                }
                div { class: "lb-column",
                    if signed_out {
                        SignIn {}
                    }
                    TableStarter {}
                    JoinSeat {}
                    Chat { key: "lobby", channel: "lobby", title: "Lobby Chat", class: "lb-lobby-chat" }
                }
            }
        }
    }
}

/// How long a search has run, as m:ss.
pub fn elapsed(since: u64, now: u64) -> String {
    let s = now.saturating_sub(since) / 1000;
    format!("{}:{:02}", s / 60, s % 60)
}

/// What to search for (player counts, opponents, rated), the search while it runs, and
/// Bot Game: the same search, for agents only.
#[component]
fn Automatch() -> Element {
    let ctx = use_lobby();
    let account = use_account();
    let toast = use_toast();
    let nav = navigator();
    let mut request = use_signal(store::search);
    let mut now = use_signal(now_ms);
    let lobby = ctx.lobby.read();
    let queue = lobby.as_ref().and_then(|l| l.queue.clone());
    let searching = queue
        .as_ref()
        .is_some_and(|q| q.state == QueueState::Searching);
    let counts = lobby.as_ref().map(|l| (l.online, l.searching));
    drop(lobby);
    use_future(move || async move {
        loop {
            platform::sleep(1000).await;
            now.set(now_ms());
        }
    });
    let search = move |request: MatchRequest| {
        spawn(async move {
            if !signed_in(account, toast, nav).await {
                return;
            }
            if let Err(e) = api::search(&request).await {
                flash(toast, e.message);
            }
            ctx.refresh().await;
        });
    };
    let cancel = move |_| {
        spawn(async move {
            let _ = api::cancel_search().await;
            ctx.refresh().await;
        });
    };
    let r = request();
    let mut change = move |f: &dyn Fn(&mut MatchRequest)| {
        f(&mut request.write());
        store::set_search(&request.peek());
    };
    rsx! {
        Fieldset { title: "Automatch", class: "lb-automatch-box",
            OptionRow { label: "Players",
                div { class: "lb-multibutton",
                    for n in 2..=6u8 {
                        div {
                            key: "{n}",
                            class: if r.players.contains(&n) { "lb-multi active" } else { "lb-multi" },
                            "data-players": "{n}",
                            onclick: move |_| change(&|r| {
                                if !r.players.contains(&n) {
                                    r.players.push(n);
                                    r.players.sort_unstable();
                                } else if r.players.len() > 1 {
                                    r.players.retain(|p| *p != n);
                                }
                            }),
                            "{n}"
                        }
                    }
                }
            }
            OptionRow { label: "Opponents",
                div { class: "lb-multibutton",
                    for (o, label, title) in [
                        (Opponents::Any, "Any", "Whoever is searching"),
                        (Opponents::Humans, "Humans", "Only people"),
                        (Opponents::Agents, "Agents", "Only agents: bots, and anyone searching over MCP"),
                    ] {
                        div {
                            key: "{label}",
                            class: if r.opponents == o { "lb-multi active" } else { "lb-multi" },
                            "data-opponents": "{label}",
                            title,
                            onclick: move |_| change(&|r| r.opponents = o),
                            "{label}"
                        }
                    }
                }
            }
            div { class: "lb-row", "data-option": "rated", title: "A game is rated when everyone in it accepts rated games and is a registered player or a bot",
                div { class: "lb-row-label", "Rated games" }
                div { class: "lb-row-rule" }
                Switch { on: r.rated, onclick: move |_| change(&|r| r.rated = !r.rated) }
            }
            div { class: "lb-search",
                match queue {
                    Some(q) if q.state == QueueState::Searching => rsx! {
                        p { class: "lb-searching", role: "status", "data-searching": "1",
                            "Searching for a game... {elapsed(q.since, now())}"
                        }
                        button { class: "lb-button lb-copper lb-search-button", "data-action": "cancel-search", onclick: cancel, "Cancel" }
                    },
                    Some(q) => rsx! {
                        p { class: "lb-searching", role: "status", "data-matched": q.game_id.clone().unwrap_or_default(), "A game was found." }
                        if let Some(id) = q.game_id.clone() {
                            button {
                                class: "lb-button lb-gold lb-search-button",
                                "data-action": "play",
                                onclick: move |_| { nav.push(Route::Game { id: id.clone() }); },
                                "Play"
                            }
                        }
                        button { class: "lb-button lb-copper lb-search-button", "data-action": "search", onclick: move |_| search(request()), "Search Again" }
                    },
                    None => rsx! {
                        button { class: "lb-button lb-gold lb-search-button", "data-action": "search", onclick: move |_| search(request()), "Find Game" }
                    },
                }
            }
            if let Some((online, s)) = counts {
                p { class: "lb-note lb-online", "data-online": "{online}",
                    "{online} online. Searching: {s.humans} "
                    if s.humans == 1 { "person" } else { "people" }
                    ", {s.agents} "
                    if s.agents == 1 { "agent" } else { "agents" }
                    "."
                }
            }
        }
        Fieldset { title: "Bot Game", class: "lb-botmatch",
            p { class: "lb-note", "A game against agents searching for one: bots, and agents signed in over MCP." }
            div { class: "lb-botmatch-buttons",
                for n in 1..=3u8 {
                    button {
                        key: "{n}",
                        class: "lb-button lb-stone lb-botmatch-button",
                        "data-bots": "{n}",
                        disabled: searching,
                        onclick: move |_| search(MatchRequest {
                            players: vec![n + 1],
                            opponents: Opponents::Agents,
                            rated: request.peek().rated,
                        }),
                        if n == 1 { "1 Bot" } else { "{n} Bots" }
                    }
                }
            }
        }
    }
}

/// The row of gold buttons over a rule: Create Table.
#[component]
pub fn TableStarter() -> Element {
    let ctx = use_lobby();
    let account = use_account();
    let toast = use_toast();
    let nav = navigator();
    let has_table = ctx.lobby.read().as_ref().is_some_and(|l| l.table.is_some());
    rsx! {
        div { class: "lb-table-start",
            button {
                class: "lb-button lb-gold lb-starter-button",
                "data-action": "create-table",
                onclick: move |_| {
                    spawn(async move {
                        if create_table(ctx, account, toast, nav).await {
                            nav.push(Route::MyTable {});
                        }
                    });
                },
                if has_table { "My Table" } else { "Create Table" }
            }
        }
    }
}

/// Takes a seat by its token.
#[component]
fn JoinSeat() -> Element {
    let mut token = use_signal(String::new);
    let mut error = use_signal(|| None::<String>);
    let nav = navigator();
    let mut join = move || {
        let t = token().trim().to_string();
        if t.is_empty() {
            return;
        }
        error.set(None);
        spawn(async move {
            match api::seat(&t).await {
                Ok(info) => {
                    session::hold(
                        &info.game_id,
                        HeldSeat {
                            seat: info.seat,
                            name: info.name,
                            token: t,
                        },
                    );
                    nav.push(Route::Game { id: info.game_id });
                }
                Err(e) => error.set(Some(e.message)),
            }
        });
    };
    rsx! {
        Fieldset { title: "Join a Seat", class: "lb-join",
            div { class: "lb-join-row",
                input {
                    class: "lb-input",
                    placeholder: "seat token",
                    value: "{token}",
                    oninput: move |e| token.set(e.value()),
                    onkeydown: move |e| {
                        if e.key() == Key::Enter {
                            join();
                        }
                    },
                }
                button { class: "lb-button lb-copper lb-join-button", onclick: move |_| join(), "Join" }
            }
            if let Some(e) = error() {
                p { class: "lb-error", "{e}" }
            }
        }
    }
}
