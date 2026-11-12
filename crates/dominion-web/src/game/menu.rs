//! The game menu (Resign, back to the lobby) and the question asked before resigning.

use super::use_game;
use crate::app::Route;
use dioxus::prelude::*;

/// The menu button and, once pressed, its entries beside it.
#[component]
pub fn GameMenu() -> Element {
    let ctx = use_game();
    let mut open = use_signal(|| false);
    let mut resigning = ctx.resigning;
    let running = (ctx.state.read().view.as_ref()).is_some_and(|v| v.game.running);
    let can_resign = running && ctx.seat.read().is_some();
    rsx! {
        span { class: "game-menu right",
            if open() {
                if can_resign {
                    button {
                        "data-action": "resign",
                        onclick: move |e| {
                            e.stop_propagation();
                            open.set(false);
                            resigning.set(true);
                        },
                        "Resign"
                    }
                }
                Link { class: "button", to: Route::Lobby {}, "data-action": "lobby", "Lobby" }
            }
            button {
                class: if open() { "toggle on" } else { "toggle" },
                title: "Game menu",
                "data-action": "menu",
                "aria-expanded": "{open()}",
                onclick: move |e| {
                    e.stop_propagation();
                    open.toggle();
                },
                "☰"
            }
        }
    }
}

/// "Are you sure you want to resign?", over the game.
#[component]
pub fn ResignDialog() -> Element {
    let ctx = use_game();
    let mut resigning = ctx.resigning;
    let running = (ctx.state.read().view.as_ref()).is_some_and(|v| v.game.running);
    if !resigning() || !running {
        return rsx! {};
    }
    rsx! {
        div {
            class: "resign-ask",
            role: "dialog",
            "data-resign": "1",
            onclick: move |e| e.stop_propagation(),
            p { "Are you sure you want to resign?" }
            div { class: "resign-buttons",
                button {
                    "data-action": "resign-yes",
                    onclick: move |_| {
                        resigning.set(false);
                        ctx.resign.call(());
                    },
                    "Resign"
                }
                button {
                    "data-action": "resign-no",
                    onclick: move |_| resigning.set(false),
                    "Cancel"
                }
            }
        }
    }
}
