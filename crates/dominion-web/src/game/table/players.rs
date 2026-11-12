//! The players: opponents along the top, the play area of whoever's turn it is, and
//! your deck, hand, discard pile and mats at the bottom.

use super::layout::{self, Row};
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
            div {
                key: "{key}",
                class: "dt-zone",
                "data-zone": "{key}",
                "data-anchor": if key == "inPlay" { format!("inplay:{}", p.seat) } else { format!("zone:{}:{key}", p.seat) },
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
pub fn Opponents(opponents: Vec<PlayerEntry>) -> Element {
    rsx! {
        div { class: "dt-opps", style: "--n:{opponents.len().max(1)};",
            for o in opponents.iter() {
                Opponent { key: "{o.seat}", player: o.clone() }
            }
        }
    }
}

/// A player's piles as small cards: the hand as a fan of backs, the deck, the discard.
#[component]
fn SmallPile(
    anchor: String,
    label: String,
    count: u32,
    #[props(default)] top: Option<String>,
    #[props(default)] fan: bool,
    popup: Popup,
) -> Element {
    let ctx = use_table();
    let backs = if fan { count.clamp(1, 5) } else { 1 };
    rsx! {
        div {
            class: "dt-spile dt-click",
            class: if fan { "dt-fan" },
            class: if count == 0 { "dt-spile-empty" },
            "data-anchor": "{anchor}",
            title: "{label}: {count}",
            onclick: move |e| { e.stop_propagation(); ctx.open(popup.clone()); },
            div { class: "dt-spile-cards", style: "--fan:{backs};",
                if count > 0 {
                    match top.clone() {
                        Some(card) => rsx! {
                            div { class: "dt-spile-card", "data-name": "{card}",
                                crate::card::CardView { name: card.clone(), size: CardSize::Full }
                            }
                        },
                        None => rsx! {
                            for i in 0..backs {
                                div { key: "{i}", class: "dt-spile-card", style: "--i:{i};",
                                    crate::card::CardBack { size: CardSize::Full }
                                }
                            }
                        },
                    }
                }
                span { class: "dt-spile-n", "{count}" }
            }
            div { class: "dt-spile-label", "{label}" }
        }
    }
}

/// A discard pile's top card when the seat can tell it: one kind of card, all known.
fn discard_top(z: &Zone) -> Option<String> {
    match (z.known.as_slice(), z.unknown) {
        ([only], 0) => Some(only.card.clone()),
        _ => None,
    }
}

/// Mats, set-aside cards, cards kept in play and the landscapes a player holds, as
/// chips that open them.
#[component]
fn ZoneChips(player: PlayerEntry, show_in_play: bool) -> Element {
    let ctx = use_table();
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
    let held: Vec<String> = p
        .projects
        .iter()
        .chain(&p.states)
        .chain(&p.boons)
        .chain(&p.artifacts)
        .cloned()
        .collect();
    rsx! {
        div { class: "dt-ochips",
            for (key, label, z) in groups.into_iter() {
                {
                    let anchor = if key == "inPlay" { format!("inplay:{}", p.seat) } else { format!("zone:{}:{key}", p.seat) };
                    let popup = zone_popup(format!("{}: {label}", p.name), &z, None);
                    let names: Vec<String> = z.known.iter().map(|s| s.card.clone()).collect();
                    rsx! {
                        button {
                            key: "{key}",
                            class: "dt-ochip",
                            "data-anchor": "{anchor}",
                            "data-zone": "{key}",
                            title: "{label}",
                            onclick: move |e| { e.stop_propagation(); ctx.open(popup.clone()); },
                            span { class: "dt-ochip-k", "{label}" }
                            span { class: "dt-ochip-n", "{z.count}" }
                            for n in names.iter() {
                                span { key: "{n}", class: "dt-ochip-card", "data-name": "{n}",
                                    HoverName { name: n.clone() }
                                }
                            }
                        }
                    }
                }
            }
            for c in held.iter() {
                span { key: "h{c}", class: "dt-ochip dt-ochip-held", HoverName { name: c.clone() } }
            }
            for u in p.uncertain.iter() {
                span { class: "dt-uncertain", title: "Cards known only to be among these zones", "{u.text}" }
            }
            if !p.counters.is_empty() {
                for (k, v) in p.counters.iter() {
                    span { key: "c{k}", class: "dt-ochip dt-ochip-held", "{k} {v}" }
                }
            }
        }
    }
}

#[component]
fn Opponent(player: PlayerEntry) -> Element {
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
            "data-anchor": "player:{p.seat}",
            div { class: "dt-opp-head",
                span { class: "dt-pname", "{p.name}" }
                if let Some(vp) = p.vp {
                    span { class: "dt-vp", title: "Victory points", "{vp}" }
                }
                if p.deciding {
                    span { class: "dt-flag dt-flag-decides", "deciding" }
                } else if p.is_turn {
                    span { class: "dt-flag", "their turn" }
                }
                Badges { badges: p.badges.clone() }
            }
            div { class: "dt-opp-body",
                SmallPile { anchor: format!("hand:{}", p.seat), label: "Hand", count: p.hand.count, fan: true, popup: hand }
                SmallPile { anchor: format!("deck:{}", p.seat), label: "Deck", count: p.deck.count, popup: deck }
                SmallPile { anchor: format!("discard:{}", p.seat), label: "Discard", count: p.discard.count, top: discard_top(&p.discard), popup: discard }
                ZoneChips { player: p.clone(), show_in_play: !p.is_turn }
            }
        }
    }
}

/// One card of a row: a stack, or the seat's unknown cards as a back.
#[derive(Clone, Debug, PartialEq)]
struct Slot {
    card: String,
    count: u32,
    back: bool,
    earlier: bool,
    mark: crate::game::table::Mark,
}

/// The style placing one slot of a row laid out by [`Row`].
fn slot_style(row: &Row, last: bool) -> String {
    match row.step {
        Some(step) if !last => format!("width:{:.1}px;margin-right:{:.1}px;", row.w, step - row.w),
        _ => format!("width:{:.1}px;", row.w),
    }
}

#[component]
pub fn PlayArea(
    board: Rc<Board>,
    log: Vec<LogEntry>,
    ui: Option<Rc<Ui>>,
    sel: Selection,
    /// The play area's box, and the widest its cards may be.
    w: f64,
    h: f64,
    gap: f64,
    max_w: f64,
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
    let mut slots: Vec<Slot> = vec![];
    for (stacks, early) in [(&earlier, true), (&now, false)] {
        for s in stacks {
            slots.push(Slot {
                card: s.card.clone(),
                count: s.count,
                back: false,
                earlier: early,
                mark: mark_for(&s.card),
            });
        }
    }
    if pa.cards.unknown > 0 {
        slots.push(Slot {
            card: String::new(),
            count: pa.cards.unknown,
            back: true,
            earlier: false,
            mark: Default::default(),
        });
    }
    let split = !earlier.is_empty() && slots.iter().any(|s| !s.earlier);
    let pad = 2.0 * gap;
    let avail_w = (w - 2.0 * pad - if split { 4.0 * gap } else { 0.0 }).max(10.0);
    let avail_h = (h - 2.0 * pad - 1.2 * gap).max(10.0);
    let row = layout::play(slots.len(), avail_w, avail_h, gap, max_w);
    let per = slots.len().div_ceil(row.rows.max(1)).max(1);
    let size = if row.mini {
        CardSize::Mini
    } else {
        CardSize::Full
    };
    let rows: Vec<Vec<Slot>> = slots.chunks(per).map(<[Slot]>::to_vec).collect();
    let empty = slots.is_empty();
    rsx! {
        div {
            class: "dt-play {seat_class(who.seat)}",
            class: if empty { "dt-play-is-empty" },
            "data-player": "{who.seat}",
            "data-anchor": "inplay:{who.seat}",
            div { class: "dt-play-label",
                if yours { "Your cards in play" } else { "{who.name}'s cards in play" }
            }
            div { class: "dt-play-rows",
                for (ri, r) in rows.iter().enumerate() {
                    div { key: "{ri}", class: "dt-row dt-play-row",
                        for (i, s) in r.iter().enumerate() {
                            div {
                                key: "{s.earlier}{s.card}",
                                class: "dt-slot",
                                class: if s.earlier { "dt-earlier" },
                                class: if split && s.earlier && r.get(i + 1).is_some_and(|n| !n.earlier) { "dt-split" },
                                style: slot_style(&row, i + 1 == r.len()),
                                TableCard {
                                    name: s.card.clone(),
                                    size,
                                    back: s.back,
                                    count: (s.count > 1 || s.back).then_some(s.count),
                                    mark: s.mark.clone(),
                                    label: (s.earlier && !row.mini).then(|| "earlier turn".to_string()),
                                }
                            }
                        }
                    }
                }
            }
            if empty {
                div { class: "dt-play-empty", "No cards in play" }
            }
        }
    }
}

#[component]
pub fn YourArea(
    board: Rc<Board>,
    ui: Option<Rc<Ui>>,
    sel: Selection,
    hand_w: f64,
    hand_h: f64,
    gap: f64,
) -> Element {
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
    let top = discard_top(&you.discard);
    let mut slots: Vec<Slot> = you
        .hand
        .known
        .iter()
        .map(|s| Slot {
            card: s.card.clone(),
            count: s.count,
            back: false,
            earlier: false,
            mark: mark_at(&ui, &sel, Spot::Hand(s.card.clone())),
        })
        .collect();
    if you.hand.unknown > 0 {
        slots.push(Slot {
            card: String::new(),
            count: you.hand.unknown,
            back: true,
            earlier: false,
            mark: Default::default(),
        });
    }
    let row = layout::hand(slots.len(), hand_w.max(10.0), hand_h.max(10.0), gap);
    let ctx = use_table();
    rsx! {
        div { class: "dt-you {seat_class(you.seat)}", "data-seat": "{you.seat}",
            div { class: "dt-pilebox", "data-pile": "deck", "data-anchor": "deck:{you.seat}",
                if you.deck.count > 0 {
                    TableCard { name: "", size: CardSize::Full, back: true, count: Some(you.deck.count), opens: deck }
                } else {
                    div { class: "dt-empty-slot", onclick: move |e| { e.stop_propagation(); ctx.open(deck.clone()); }, "0" }
                }
                div { class: "dt-pile-label", "Deck" }
            }
            div { class: "dt-hand", "data-anchor": "hand:{you.seat}",
                if you.hand.count == 0 {
                    div { class: "dt-hand-empty", "Your hand is empty" }
                }
                for (i, s) in slots.iter().enumerate() {
                    div { key: "{s.back}{s.card}", class: "dt-slot", style: slot_style(&row, i + 1 == slots.len()),
                        TableCard {
                            name: s.card.clone(),
                            size: CardSize::Full,
                            back: s.back,
                            count: (s.count > 1 || s.back).then_some(s.count),
                            mark: s.mark.clone(),
                        }
                    }
                }
            }
            div { class: "dt-pilebox", "data-pile": "discard", "data-anchor": "discard:{you.seat}",
                if you.discard.count == 0 {
                    div { class: "dt-empty-slot", "0" }
                } else if let Some(top) = top {
                    TableCard { name: top, size: CardSize::Full, count: Some(you.discard.count), opens: discard }
                } else {
                    TableCard { name: "", size: CardSize::Full, back: true, count: Some(you.discard.count), opens: discard }
                }
                div { class: "dt-pile-label", "Discard" }
            }
        }
    }
}

/// Your name, VP, tokens, mats and the like, in the side column.
#[component]
pub fn YouPanel(board: Rc<Board>, ui: Option<Rc<Ui>>, sel: Selection) -> Element {
    let Some(you) = board.you.clone() else {
        return rsx! {};
    };
    let your_turn = board.play_area.player.as_ref().map(|w| w.seat) == Some(you.seat);
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
        div { class: "dt-yzones {seat_class(you.seat)}", "data-anchor": "player:{you.seat}",
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
                Badges { badges: you.badges.clone() }
            }
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
