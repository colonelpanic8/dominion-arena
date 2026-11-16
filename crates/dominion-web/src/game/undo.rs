//! The undo waiting to be allowed: the shown seat's own (withdraw it), one waiting for
//! the shown seat (allow or refuse it), or someone else's, and a refusal of the seat's.

use super::plain::player_name;
use super::use_game;
use dioxus::prelude::*;
use dominion_protocol::lobby::UndoRule;

#[component]
pub fn UndoNotice(#[props(default)] style: String) -> Element {
    let ctx = use_game();
    let st = ctx.state.read();
    let Some(view) = st.view.as_ref() else {
        return rsx! {};
    };
    let undo = st.undo.clone();
    let busy = st.answering;
    let seat = view.seat;
    let name = |s: u8| player_name(view, s);
    let answers = |n: u32| {
        if n == 1 {
            "1 answer".to_string()
        } else {
            format!("{n} answers")
        }
    };
    let names = |seats: &[u8]| {
        seats
            .iter()
            .map(|&s| name(s))
            .collect::<Vec<_>>()
            .join(" and ")
    };
    if let Some(r) = undo.request {
        let what = answers(r.answers);
        let waiting = names(&r.waiting);
        let asker = name(r.by);
        drop(st);
        return if Some(r.by) == seat {
            rsx! {
                div { class: "waiting-notice undo-notice", role: "status", style, "data-undo": "asked",
                    p { class: "waiting-title", "You asked to take back {what}." }
                    p { "Waiting for {waiting} to allow it." }
                    button {
                        "data-action": "cancel-undo",
                        disabled: busy,
                        onclick: move |_| ctx.cancel_undo.call(()),
                        "Withdraw"
                    }
                }
            }
        } else if seat.is_some_and(|s| r.waiting.contains(&s)) {
            rsx! {
                div { class: "waiting-notice undo-notice", role: "alert", style, "data-undo": "asking",
                    p { class: "waiting-title", "{asker} asks to take back {what}." }
                    button {
                        "data-action": "allow-undo",
                        disabled: busy,
                        onclick: move |_| ctx.answer_undo.call(true),
                        "Allow"
                    }
                    " "
                    button {
                        "data-action": "refuse-undo",
                        disabled: busy,
                        onclick: move |_| ctx.answer_undo.call(false),
                        "Refuse"
                    }
                }
            }
        } else {
            rsx! {
                div { class: "waiting-notice undo-notice", role: "status", style, "data-undo": "other",
                    p { class: "waiting-title", "{asker} asked to take back {what}; waiting for {waiting}." }
                }
            }
        };
    }
    if let Some(by) = undo.refused_by {
        let refuser = name(by);
        drop(st);
        return rsx! {
            div { class: "waiting-notice undo-notice", role: "status", style, "data-undo": "refused",
                p { class: "waiting-title", "{refuser} refused your undo." }
            }
        };
    }
    rsx! {}
}

/// The Undo button's tooltip under `rule`.
pub fn undo_title(rule: UndoRule) -> &'static str {
    match rule {
        UndoRule::Ask => "Ask the other players to let you take back your last answer (Ctrl+Z)",
        _ => "Take back your last answer (Ctrl+Z)",
    }
}
