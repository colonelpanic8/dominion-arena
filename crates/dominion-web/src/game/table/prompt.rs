//! The status and prompt bar, and the choice tray for options the table shows nowhere
//! else: cards in decks, discard piles, set aside, named cards.

use super::pieces::{HoverName, TableCard, seat_class};
use super::{Spot, Ui, use_table};
use crate::app::use_cards;
use crate::card::CardSize;
use crate::game::board::Status;
use crate::game::state::Selection;
use dioxus::prelude::*;
use std::rc::Rc;

/// Number decisions with more values than this get a picker instead of a button each.
const MAX_NUMBER_BUTTONS: usize = 8;

fn picks_text(ui: &Ui, sel: &Selection) -> String {
    let (min, max) = (ui.inter.min, ui.inter.max);
    let n = sel.picks().len();
    if min == max {
        format!("{n} of {min} chosen")
    } else {
        format!("{n} of {min}–{max} chosen")
    }
}

/// The prompt with its source card's name hoverable: leading the prompt where it
/// already names it, else in front of it.
fn prompt_line(d: &dominion_protocol::AgentDecision) -> Element {
    let Some(src) = d.source.clone() else {
        return rsx! { "{d.prompt}" };
    };
    let rest = d
        .prompt
        .strip_prefix(src.as_str())
        .map(str::to_string)
        .unwrap_or_else(|| format!(": {}", d.prompt));
    rsx! {
        HoverName { name: src }
        "{rest}"
    }
}

fn phase_name(p: &str) -> String {
    let mut c = p.chars();
    match c.next() {
        Some(f) => f.to_uppercase().chain(c).collect(),
        None => String::new(),
    }
}

#[component]
fn StatusLine(status: Status) -> Element {
    let s = &status;
    rsx! {
        div { class: "dt-status",
            if let Some(n) = s.turn {
                span { class: "dt-turnno", "Turn {n}" }
            }
            if let Some(w) = &s.player {
                span { class: "dt-who {seat_class(w.seat)}",
                    if s.your_turn { "Your turn" } else { "{w.name}'s turn" }
                }
            }
            if let Some(x) = &s.extra {
                span { class: "dt-extra", "extra turn: " HoverName { name: x.clone() } }
            }
            if let Some(c) = &s.controller {
                if s.player.as_ref().map(|p| p.seat) != Some(c.seat) {
                    span { class: "dt-extra", "played by " span { class: seat_class(c.seat), "{c.name}" } }
                }
            }
            if let Some(p) = &s.phase {
                span { class: "dt-phase", "data-phase": "{p}", "{phase_name(p)}" }
            }
            if let Some(n) = s.actions {
                span { class: "dt-stat", title: "Actions", span { class: "dt-stat-k", "Actions" } "{n}" }
            }
            if let Some(n) = s.buys {
                span { class: "dt-stat", title: "Buys", span { class: "dt-stat-k", "Buys" } "{n}" }
            }
            if let Some(n) = s.coins {
                span { class: "dt-stat dt-coins", title: "Coins", span { class: "dt-coin", "{n}" } }
            }
            if let Some(n) = s.potions.filter(|&n| n > 0) {
                span { class: "dt-stat", title: "Potions", span { class: "dt-potion" } "{n}" }
            }
            if let Some(n) = s.debt.filter(|&n| n > 0) {
                span { class: "dt-stat", title: "Debt", span { class: "dt-debt", "{n}" } }
            }
        }
    }
}

#[component]
pub fn Bar(
    status: Status,
    ui: Option<Rc<Ui>>,
    sel: Selection,
    error: Option<String>,
    busy: bool,
    can_treasures: bool,
) -> Element {
    let ctx = use_table();
    let waiting = if !status.running {
        Some("The game is over.".to_string())
    } else if ui.is_none() {
        Some(match &status.deciding {
            Some(w) => format!("Waiting for {}…", w.name),
            None => "Waiting…".into(),
        })
    } else {
        None
    };
    let buttons: Vec<usize> = ui
        .as_ref()
        .map(|u| {
            (0..u.spots.len())
                .filter(|&i| u.spots[i] == Spot::Button)
                .collect()
        })
        .unwrap_or_default();
    let numbers = ui.as_ref().is_some_and(|u| {
        buttons.len() > MAX_NUMBER_BUTTONS
            && buttons
                .iter()
                .all(|&i| u.inter.targets[i].action == "number")
    });
    let mut number = use_signal(|| None::<usize>);
    rsx! {
        div { class: "dt-bar",
            class: if ui.is_some() { "dt-bar-active" },
            StatusLine { status: status.clone() }
            div { class: "dt-prompt",
                if let Some(w) = waiting {
                    span { class: "dt-waiting", "{w}" }
                }
                if let Some(u) = ui.clone() {
                    span { class: "dt-prompt-text", "data-prompt": "{u.decision.purpose}",
                        {prompt_line(&u.decision)}
                    }
                    if !u.click_answers {
                        span { class: "dt-picks", "{picks_text(&u, &sel)}" }
                    }
                    div { class: "dt-buttons",
                        if numbers {
                            select {
                                class: "dt-number",
                                "data-target-group": "number",
                                "data-options": "{buttons.len()}",
                                onchange: move |e| number.set(e.value().parse().ok()),
                                option { value: "", "Choose…" }
                                for &i in buttons.iter() {
                                    option { key: "{i}", value: "{i}", "{u.inter.targets[i].label}" }
                                }
                            }
                            button {
                                class: "dt-btn dt-btn-gold",
                                disabled: busy || number().is_none(),
                                onclick: move |e| {
                                    e.stop_propagation();
                                    if let Some(i) = number() {
                                        ctx.act(i);
                                    }
                                },
                                "OK"
                            }
                        } else {
                            for &i in buttons.iter() {
                                {
                                    let t = &u.inter.targets[i];
                                    let picked = sel.positions(&t.options);
                                    let key = if t.action == "end-phase" { Some("E") } else { None };
                                    rsx! {
                                        button {
                                            key: "{i}",
                                            class: "dt-btn dt-act-{t.action}",
                                            class: if !picked.is_empty() { "dt-on" },
                                            disabled: busy,
                                            "data-target": "{i}",
                                            "data-options": "1",
                                            onclick: move |e| { e.stop_propagation(); ctx.act(i); },
                                            if u.inter.ordered && !picked.is_empty() {
                                                span { class: "dt-order-n", "{picked[0] + 1}" }
                                            }
                                            "{t.label}"
                                            if let Some(k) = key {
                                                kbd { "{k}" }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        if can_treasures {
                            button {
                                class: "dt-btn dt-btn-gold dt-treasures",
                                disabled: busy,
                                "data-action": "play-treasures",
                                onclick: move |e| { e.stop_propagation(); ctx.play_treasures(); },
                                "Play Treasures"
                                kbd { "T" }
                            }
                        }
                        if !u.click_answers {
                            button {
                                class: "dt-btn dt-btn-gold dt-confirm",
                                disabled: busy || !sel.is_legal(&u.decision),
                                "data-action": "confirm",
                                onclick: move |e| { e.stop_propagation(); ctx.confirm(); },
                                "Confirm"
                                kbd { "Enter" }
                            }
                            if !sel.picks().is_empty() {
                                button {
                                    class: "dt-btn dt-btn-stone",
                                    "data-action": "clear",
                                    onclick: move |e| { e.stop_propagation(); ctx.clear(); },
                                    "Clear"
                                    kbd { "Esc" }
                                }
                            }
                        }
                    }
                }
                if let Some(e) = error {
                    span { class: "dt-error", "data-error": "1", "{e}" }
                }
            }
        }
    }
}

fn option_text(ui: &Ui, index: u32) -> String {
    ui.decision
        .options
        .iter()
        .find(|o| o.index == index)
        .map(|o| o.card.clone().unwrap_or_else(|| o.label.clone()))
        .unwrap_or_default()
}

fn zone_text(zone: &str) -> &str {
    match zone {
        "deck" => "from deck",
        "discard" => "from discard",
        "hand" => "from hand",
        "inPlay" => "in play",
        "setAside" => "set aside",
        "trash" => "from trash",
        "pile" => "from a pile",
        other => other,
    }
}

#[component]
pub fn Tray(ui: Option<Rc<Ui>>, sel: Selection) -> Element {
    let ctx = use_table();
    let cards = use_cards();
    let mut collapsed = use_signal(|| false);
    let mut query = use_signal(String::new);
    let mut dragging = use_signal(|| None::<usize>);
    let Some(u) = ui else {
        return rsx! {};
    };
    let tray: Vec<usize> = (0..u.spots.len())
        .filter(|&i| u.spots[i] == Spot::Tray)
        .collect();
    let show_order = u.inter.ordered && sel.picks().len() > 1;
    if tray.is_empty() && !u.inter.name_any && !show_order {
        return rsx! {};
    }
    if collapsed() {
        return rsx! {
            button {
                class: "dt-btn dt-tray-show",
                "data-action": "show-tray",
                onclick: move |e| { e.stop_propagation(); collapsed.set(false); },
                "Show choices ({tray.len()})"
            }
        };
    }
    let q = query().trim().to_lowercase();
    let results: Vec<String> = if u.inter.name_any && q.len() >= 2 {
        cards
            .nameable()
            .into_iter()
            .filter(|n| n.to_lowercase().contains(&q))
            .take(12)
            .collect()
    } else {
        vec![]
    };
    let picks = sel.picks().to_vec();
    rsx! {
        div { class: "dt-tray", "data-tray": "1", onclick: move |e| e.stop_propagation(),
            div { class: "dt-tray-head",
                span { "{u.decision.prompt}" }
                button {
                    class: "dt-x",
                    title: "Hide (to see the table under it)",
                    "data-action": "hide-tray",
                    onclick: move |_| collapsed.set(true),
                    "–"
                }
            }
            if !tray.is_empty() {
                div { class: "dt-tray-cards",
                    for &i in tray.iter() {
                        {
                            let t = &u.inter.targets[i];
                            let mark = u.mark_target(&sel, i);
                            let label = match (&t.card, &t.zone) {
                                (Some(c), Some(z)) if &t.label == c => Some(zone_text(z).to_string()),
                                (Some(c), _) if &t.label == c => None,
                                _ => Some(t.label.clone()),
                            };
                            let count = (t.options.len() > 1).then_some(t.options.len() as u32);
                            let landscape = t.card.as_deref().and_then(|c| cards.get(c)).is_some_and(crate::card::is_landscape);
                            rsx! {
                                div { key: "{i}", class: "dt-tslot", class: if landscape { "dt-tslot-wide" },
                                    TableCard {
                                        name: t.card.clone().unwrap_or_default(),
                                        size: CardSize::Full,
                                        back: t.card.is_none(),
                                        count,
                                        mark,
                                        label,
                                    }
                                }
                            }
                        }
                    }
                }
            }
            if show_order {
                div { class: "dt-order", "data-order": "1",
                    span { class: "dt-order-label", "Order (first is on top):" }
                    for (pos, &p) in picks.iter().enumerate() {
                        div {
                            key: "{p}",
                            class: "dt-order-chip",
                            draggable: "true",
                            "data-pos": "{pos}",
                            ondragstart: move |_| dragging.set(Some(pos)),
                            ondragover: move |e| e.prevent_default(),
                            ondrop: move |e| {
                                e.prevent_default();
                                if let Some(from) = dragging() {
                                    ctx.reorder(from, pos);
                                }
                                dragging.set(None);
                            },
                            button {
                                class: "dt-arrow",
                                disabled: pos == 0,
                                "data-move": "left",
                                onclick: move |_| ctx.reorder(pos, pos - 1),
                                "◀"
                            }
                            span { "{pos + 1}. " HoverName { name: option_text(&u, p) } }
                            button {
                                class: "dt-arrow",
                                disabled: pos + 1 == picks.len(),
                                "data-move": "right",
                                onclick: move |_| ctx.reorder(pos, pos + 1),
                                "▶"
                            }
                        }
                    }
                }
            }
            if u.inter.name_any {
                div { class: "dt-name-any", "data-target-group": "name",
                    input {
                        class: "dt-search",
                        placeholder: "Name any card…",
                        value: "{query}",
                        oninput: move |e| query.set(e.value()),
                        onkeydown: move |e| {
                            if e.key() == Key::Enter {
                                let q = query();
                                ctx.name(q.trim());
                            }
                        },
                    }
                    div { class: "dt-results",
                        for n in results.iter() {
                            {
                                let name = n.clone();
                                rsx! {
                                    div {
                                        key: "{n}",
                                        class: "dt-result",
                                        "data-name-result": "{n}",
                                        onclick: move |_| ctx.name(&name),
                                        TableCard { name: n.clone(), size: CardSize::Mini, class: "dt-target" }
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
