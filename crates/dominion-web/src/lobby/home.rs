//! The Automatch tab: dominion.games' landing tab, with what of it applies here: Bot
//! Game (a game at once against seats for agents), the table starter, and joining a
//! seat by its token where the original lists friends.

use super::my_table::start;
use super::rules::{SeatKind, SeatSpec, TableRules, expansions};
use super::store::MyTable;
use super::{Fieldset, LobbyFrame, Tab, new_table, use_banned, use_table, use_toast};
use crate::api;
use crate::app::{Route, use_cards};
use crate::session::{self, HeldSeat};
use dioxus::prelude::*;

#[component]
pub fn AutomatchTab() -> Element {
    rsx! {
        LobbyFrame { tab: Tab::Automatch,
            div { class: "lb-columns",
                div { class: "lb-column",
                    BotGame {}
                }
                div { class: "lb-column",
                    TableStarter {}
                    JoinSeat {}
                }
            }
        }
    }
}

/// "1 Bot", "2 Bots", "3 Bots": a table with that many seats for agents, started at
/// once with a random kingdom.
#[component]
fn BotGame() -> Element {
    let cards = use_cards();
    let table = use_table();
    let toast = use_toast();
    let banned = use_banned();
    let nav = navigator();
    let sets = expansions(&cards);
    rsx! {
        Fieldset { title: "Bot Game", class: "lb-botmatch",
            div { class: "lb-botmatch-buttons",
                for n in 1..=3usize {
                    {
                        let sets = sets.clone();
                        let banned = banned.clone();
                        rsx! {
                            button {
                                key: "{n}",
                                class: "lb-button lb-stone lb-botmatch-button",
                                "data-bots": "{n}",
                                disabled: cards.is_empty(),
                                onclick: move |_| {
                                    let mut rules = TableRules::new(&super::default_name(), &sets);
                                    rules.seats.truncate(1);
                                    for i in 0..n {
                                        rules.seats.push(SeatSpec {
                                            name: format!("Agent {}", i + 1),
                                            kind: SeatKind::Agent,
                                        });
                                    }
                                    let mut table = table;
                                    table.set(Some(MyTable { rules, game: None, links: vec![] }));
                                    spawn(start(table, banned.clone(), toast, nav, true));
                                },
                                if n == 1 { "1 Bot" } else { "{n} Bots" }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// The row of gold buttons over a rule: Create Table.
#[component]
pub fn TableStarter() -> Element {
    let cards = use_cards();
    let table = use_table();
    let nav = navigator();
    let sets = expansions(&cards);
    rsx! {
        div { class: "lb-table-start",
            button {
                class: "lb-button lb-gold lb-starter-button",
                disabled: cards.is_empty(),
                onclick: move |_| {
                    new_table(table, &sets);
                    nav.push(Route::MyTable {});
                },
                "Create Table"
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
