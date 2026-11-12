//! The players: opponents along the top, the play area of whoever's turn it is, and
//! your deck, hand, discard pile and mats at the bottom.

use super::model::split_play;
use super::pieces::{HoverName, TableCard, seat_class};
use super::supply::Badges;
use super::{Popup, Spot, Ui, mark_at, use_table};
use crate::card::CardSize;
use crate::game::board::{Board, PlayerEntry, Stack, Zone};
use crate::game::state::Selection;
use dioxus::prelude::*;
use dominion_protocol::LogEntry;
use std::rc::Rc;

const UNKNOWN: &str = "unknown";

fn zone_popup(title: String, z: &Zone, note: Option<String>) -> Popup {
    Popup {
        title: format!("{title} ({})", z.count),
        stacks: z.known.clone(),
        unknown: z.unknown,
        note,
    }
}

/// Small labelled stacks: a player's mats, set-aside cards, kept-in-play cards and the
/// landscapes they hold.
#[component]
fn ZoneStrips(
    player: PlayerEntry,
    /// Their cards in play, when the play area doesn't show them.
    show_in_play: bool,
    ui: Option<Rc<Ui>>,
    sel: Selection,
    you: bool,
) -> Element {
    let p = &player;
    let mut groups: Vec<(String, String, Zone)> = vec![];
    if show_in_play && p.in_play.count > 0 {
        groups.push(("inPlay".into(), "In play".into(), p.in_play.clone()));
    }
    for z in &p.zones {
        if z.zone.count > 0 {
            groups.push((z.key.clone(), z.label.clone(), z.zone.clone()));
        }
    }
    let held: Vec<(&str, &Vec<String>)> = [
        ("Projects", &p.projects),
        ("States", &p.states),
        ("Boons", &p.boons),
        ("Artifacts", &p.artifacts),
    ]
    .into_iter()
    .filter(|(_, l)| !l.is_empty())
    .collect();
    rsx! {
        for (key, label, z) in groups.iter() {
            div { key: "{key}", class: "dt-zone", "data-zone": "{key}",
                div { class: "dt-zone-label", "{label} " span { class: "dt-zone-n", "{z.count}" } }
                div { class: "dt-zone-cards",
                    for s in z.known.iter() {
                        TableCard {
                            key: "{s.card}",
                            name: s.card.clone(),
                            size: CardSize::Micro,
                            count: (s.count > 1).then_some(s.count),
                            mark: if you && key == "inPlay" { mark_at(&ui, &sel, Spot::InPlay(s.card.clone())) } else { Default::default() },
                        }
                    }
                    if z.unknown > 0 {
                        span { class: "dt-zone-unknown", "{z.unknown} face down" }
                    }
                }
            }
        }
        for (label, list) in held.iter() {
            div { key: "{label}", class: "dt-zone",
                div { class: "dt-zone-label", "{label}" }
                div { class: "dt-zone-cards",
                    for c in list.iter() {
                        TableCard { key: "{c}", name: c.clone(), size: CardSize::Micro }
                    }
                }
            }
        }
        if !p.counters.is_empty() {
            div { class: "dt-zone dt-counters",
                for (k, v) in p.counters.iter() {
                    span { key: "{k}", "{k} {v}" }
                }
            }
        }
    }
}

#[component]
pub fn Opponents(opponents: Vec<PlayerEntry>, ui: Option<Rc<Ui>>, sel: Selection) -> Element {
    rsx! {
        div { class: "dt-opps",
            for o in opponents.iter() {
                Opponent { key: "{o.seat}", player: o.clone(), ui: ui.clone(), sel: sel.clone() }
            }
        }
    }
}

#[component]
fn Opponent(player: PlayerEntry, ui: Option<Rc<Ui>>, sel: Selection) -> Element {
    let ctx = use_table();
    let p = &player;
    let discard = zone_popup(format!("{}'s discard pile", p.name), &p.discard, None);
    let deck = zone_popup(
        format!("{}'s deck", p.name),
        &p.deck,
        Some("What you know is in it, in no order.".into()),
    );
    let hand = zone_popup(format!("{}'s hand", p.name), &p.hand, None);
    rsx! {
        div {
            class: "dt-opp {seat_class(p.seat)}",
            class: if p.is_turn { "dt-turn" },
            class: if p.deciding { "dt-decides" },
            "data-seat": "{p.seat}",
            div { class: "dt-opp-head",
                span { class: "dt-pname", "{p.name}" }
                if let Some(vp) = p.vp {
                    span { class: "dt-vp", title: "Victory points", "{vp}" }
                }
                if p.is_turn {
                    span { class: "dt-flag", "their turn" }
                }
                if p.deciding {
                    span { class: "dt-flag dt-flag-decides", "deciding" }
                }
                Badges { badges: p.badges.clone() }
            }
            div { class: "dt-opp-row",
                button {
                    class: "dt-count-chip",
                    title: "Hand",
                    onclick: move |e| { e.stop_propagation(); ctx.open(hand.clone()); },
                    span { class: "dt-chip-back" }
                    "Hand {p.hand.count}"
                }
                button {
                    class: "dt-count-chip",
                    title: "Deck",
                    onclick: move |e| { e.stop_propagation(); ctx.open(deck.clone()); },
                    "Deck {p.deck.count}"
                }
                button {
                    class: "dt-count-chip",
                    title: "Discard pile",
                    onclick: move |e| { e.stop_propagation(); ctx.open(discard.clone()); },
                    "Discard {p.discard.count}"
                }
                for u in p.uncertain.iter() {
                    span { class: "dt-uncertain", title: "Cards known only to be among these zones", "{u.text}" }
                }
            }
            div { class: "dt-opp-zones",
                ZoneStrips { player: p.clone(), show_in_play: !p.is_turn, ui: ui.clone(), sel: sel.clone(), you: false }
            }
        }
    }
}

#[component]
pub fn PlayArea(
    board: Rc<Board>,
    log: Vec<LogEntry>,
    ui: Option<Rc<Ui>>,
    sel: Selection,
) -> Element {
    let pa = &board.play_area;
    let Some(who) = pa.player.clone() else {
        return rsx! { div { class: "dt-play" } };
    };
    let (now, earlier) = split_play(&pa.cards, &log, who.seat);
    let yours = board.you.as_ref().is_some_and(|y| y.seat == who.seat);
    let mut marked: Vec<String> = vec![];
    let mut mark_for = |card: &str| {
        if !yours || marked.iter().any(|c| c == card) {
            return Default::default();
        }
        marked.push(card.to_string());
        mark_at(&ui, &sel, Spot::InPlay(card.to_string()))
    };
    let now_marks: Vec<_> = now.iter().map(|s| mark_for(&s.card)).collect();
    let earlier_marks: Vec<_> = earlier.iter().map(|s| mark_for(&s.card)).collect();
    let empty = now.is_empty() && earlier.is_empty() && pa.cards.unknown == 0;
    rsx! {
        div {
            class: "dt-play {seat_class(who.seat)}",
            class: if !earlier.is_empty() { "dt-has-earlier" },
            "data-player": "{who.seat}",
            div { class: "dt-play-label", "{who.name}'s play area" }
            if !earlier.is_empty() {
                div { class: "dt-earlier",
                    div { class: "dt-earlier-label", "From earlier turns" }
                    div { class: "dt-row",
                        for (s, m) in earlier.iter().zip(earlier_marks) {
                            div { key: "e{s.card}", class: "dt-slot",
                                TableCard {
                                    name: s.card.clone(),
                                    size: CardSize::Full,
                                    count: (s.count > 1).then_some(s.count),
                                    mark: m,
                                }
                            }
                        }
                    }
                }
            }
            div { class: "dt-row dt-play-row",
                for (s, m) in now.iter().zip(now_marks) {
                    div { key: "n{s.card}", class: "dt-slot",
                        TableCard {
                            name: s.card.clone(),
                            size: CardSize::Full,
                            count: (s.count > 1).then_some(s.count),
                            mark: m,
                        }
                    }
                }
                if pa.cards.unknown > 0 {
                    div { key: "{UNKNOWN}", class: "dt-slot",
                        TableCard { name: "", size: CardSize::Full, back: true, count: Some(pa.cards.unknown) }
                    }
                }
                if empty {
                    div { class: "dt-play-empty", "No cards in play" }
                }
            }
        }
    }
}

#[component]
pub fn YourArea(board: Rc<Board>, ui: Option<Rc<Ui>>, sel: Selection) -> Element {
    let Some(you) = board.you.clone() else {
        return rsx! { div { class: "dt-you dt-spectating", "Spectating" } };
    };
    let deck = zone_popup(
        "Your deck".into(),
        &you.deck,
        Some(if you.deck_order_known {
            "You know its order.".into()
        } else {
            "What you know is in it, in no order.".into()
        }),
    );
    let discard = zone_popup("Your discard pile".into(), &you.discard, None);
    let discard_top = match (you.discard.known.as_slice(), you.discard.unknown) {
        ([only], 0) => Some(only.card.clone()),
        _ => None,
    };
    let your_turn = board.play_area.player.as_ref().map(|w| w.seat) == Some(you.seat);
    let hand_marks: Vec<_> = you
        .hand
        .known
        .iter()
        .map(|s| mark_at(&ui, &sel, Spot::Hand(s.card.clone())))
        .collect();
    let owned = zone_popup(
        "Everything you own".into(),
        &you.owned,
        (!you.unlocated.is_empty()).then(|| {
            let list: Vec<String> = you
                .unlocated
                .iter()
                .map(|s: &Stack| format!("{} {}", s.count, s.card))
                .collect();
            format!("Not located: {}", list.join(", "))
        }),
    );
    let ctx = use_table();
    rsx! {
        div { class: "dt-you {seat_class(you.seat)}", "data-seat": "{you.seat}",
            div { class: "dt-pilebox", "data-pile": "deck",
                if you.deck.count > 0 {
                    TableCard { name: "", size: CardSize::Full, back: true, count: Some(you.deck.count), opens: deck }
                } else {
                    div { class: "dt-empty-slot", onclick: move |e| { e.stop_propagation(); ctx.open(deck.clone()); }, "0" }
                }
                div { class: "dt-pile-label", "Deck" }
            }
            div { class: "dt-hand",
                if you.hand.count == 0 {
                    div { class: "dt-hand-empty", "Your hand is empty" }
                }
                for (s, m) in you.hand.known.iter().zip(hand_marks) {
                    div { key: "{s.card}", class: "dt-slot",
                        TableCard {
                            name: s.card.clone(),
                            size: CardSize::Full,
                            count: (s.count > 1).then_some(s.count),
                            mark: m,
                        }
                    }
                }
                if you.hand.unknown > 0 {
                    div { key: "{UNKNOWN}", class: "dt-slot",
                        TableCard { name: "", size: CardSize::Full, back: true, count: Some(you.hand.unknown) }
                    }
                }
            }
            div { class: "dt-pilebox", "data-pile": "discard",
                if you.discard.count == 0 {
                    div { class: "dt-empty-slot", "0" }
                } else if let Some(top) = discard_top {
                    TableCard { name: top, size: CardSize::Full, count: Some(you.discard.count), opens: discard }
                } else {
                    TableCard { name: "", size: CardSize::Full, back: true, count: Some(you.discard.count), opens: discard }
                }
                div { class: "dt-pile-label", "Discard" }
            }
            div { class: "dt-yzones",
                div { class: "dt-you-head",
                    span { class: "dt-pname", "{you.name}" }
                    if let Some(vp) = you.vp {
                        span { class: "dt-vp", title: "Victory points", "{vp}" }
                    }
                    button {
                        class: "dt-count-chip",
                        title: "Every card you own",
                        onclick: move |e| { e.stop_propagation(); ctx.open(owned.clone()); },
                        "Owned {you.owned.count}"
                    }
                }
                Badges { badges: you.badges.clone() }
                ZoneStrips { player: you.clone(), show_in_play: !your_turn, ui: ui.clone(), sel: sel.clone(), you: true }
                for u in you.uncertain.iter() {
                    div { class: "dt-uncertain", "{u.text}: "
                        for s in u.candidates.iter() {
                            HoverName { key: "{s.card}", name: s.card.clone(), label: format!("{} {}", s.count, s.card) }
                            " "
                        }
                    }
                }
            }
        }
    }
}
