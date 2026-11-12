//! The players as dominion.games shows them: a bar per player at the left (opponents at
//! the top, you above your deck), the active opponent's hand, deck and discard pile
//! peeking upside down from the top edge, the turn player's play row, your hand, and
//! tabs for mats and set-aside cards.

use super::layout::{Geometry, Rect};
use super::pieces::{CardName, TableCard, cube_icon, seat_class};
use super::supply::{Badges, Tab};
use super::{Popup, Spot, Ui, mark_at, use_table};
use crate::assets::use_assets;
use crate::card::CardSize;
use crate::game::board::{Board, PlayerEntry, Stack, Zone};
use crate::game::state::Selection;
use dioxus::prelude::*;
use std::rc::Rc;

fn zone_popup(title: String, z: &Zone, note: Option<String>) -> Popup {
    Popup {
        title,
        stacks: z.known.clone(),
        unknown: z.unknown,
        note,
    }
}

/// A player's name bar: name, VP, and under them their tokens and held landscapes.
#[component]
fn PlayerBar(player: PlayerEntry, at: Rect, you: bool, active: bool) -> Element {
    let ctx = use_table();
    let assets = use_assets();
    let p = &player;
    let seat = p.seat;
    let held: Vec<String> = p
        .states
        .iter()
        .chain(&p.boons)
        .chain(&p.artifacts)
        .cloned()
        .collect();
    let cubes = p.projects.clone();
    let cube = assets.file_url(&cube_icon(seat));
    let account = p.account.clone();
    let away = account.as_ref().is_some_and(|a| !a.connected);
    // The name shrinks to fit beside the VP shield (and the connection mark).
    let vh = at.h / if you { 6.0 } else { 5.5 };
    let chars = p.name.chars().count().max(1) as f64;
    let beside = if p.account.is_some() { 7.5 } else { 5.5 };
    let name_px = (2.5 * vh).min((at.w - beside * vh) / (0.55 * chars));
    rsx! {
        div {
            class: "dt-bar {seat_class(seat)}",
            class: if you { "dt-bar-you" },
            class: if active { "dt-bar-active" },
            class: if p.is_turn { "dt-turn" },
            class: if away { "dt-bar-away" },
            style: "{at.style()}",
            "data-seat": "{seat}",
            "data-connected": account.as_ref().map(|a| a.connected.to_string()),
            "data-anchor": "player:{seat}",
            onclick: move |e| {
                e.stop_propagation();
                if !you {
                    ctx.choose_opponent(seat);
                }
            },
            div { class: "dt-bar-top",
                if let Some(a) = &account {
                    span {
                        class: if a.connected { "dt-conn dt-conn-on" } else { "dt-conn" },
                        title: "{a.title()}",
                    }
                }
                span { class: "dt-pname", style: "font-size:{name_px:.1}px;", "{p.name}" }
                if let Some(vp) = p.vp {
                    span { class: "dt-vp", title: "Victory points", "{vp}" }
                }
            }
            div { class: "dt-bar-bottom",
                Badges { badges: p.badges.clone() }
                for c in cubes.iter() {
                    span { key: "{c}", class: "dt-bar-cube", title: "{c}",
                        if let Some(src) = cube.clone() { img { src, alt: "", draggable: "false" } }
                    }
                }
                for c in held.iter() {
                    span { key: "h{c}", class: "dt-bar-held", CardName { name: c.clone() } }
                }
                for (k, v) in p.counters.iter() {
                    span { key: "c{k}", class: "dt-bar-held", "{k} {v}" }
                }
                if let Some(a) = &account {
                    span { class: "dt-bar-who", title: "{a.title()}",
                        if away { "away " }
                        "{a.tag()}"
                    }
                }
            }
        }
    }
}

/// A player's mats, set-aside cards and (off their turn) cards in play, as tabs.
fn tabs(p: &PlayerEntry, show_in_play: bool) -> Vec<(String, String, Zone, Option<String>)> {
    let mut out = vec![];
    if show_in_play && p.in_play.count > 0 {
        out.push(("inPlay".into(), "In play".into(), p.in_play.clone(), None));
    }
    for z in &p.zones {
        if z.zone.count > 0 {
            let art = match z.key.as_str() {
                "tavern" => Some("tavern-mat".to_string()),
                "exile" => Some("exile-mat".to_string()),
                _ => None,
            };
            out.push((z.key.clone(), z.label.clone(), z.zone.clone(), art));
        }
    }
    out
}

#[component]
fn PlayerTabs(player: PlayerEntry, g: Rc<Geometry>, you: bool, show_in_play: bool) -> Element {
    let p = &player;
    let list = tabs(p, show_in_play);
    let budget = if you { 75.0 } else { 50.0 } * g.vw;
    let n = list.len().max(1) as f64;
    let w = (11.0 * g.vw).min(budget / n);
    let y = if you { 96.0 * g.vh + 1.0 } else { 0.0 };
    let x0 = if you { 13.0 * g.vw } else { 0.0 };
    rsx! {
        for (i, (key, label, z, art)) in list.into_iter().enumerate() {
            Tab {
                key: "{key}",
                label: label.clone(),
                count: z.count,
                at: Rect::new(x0 + i as f64 * w * 0.95, y, w, 4.0 * g.vh),
                popup: zone_popup(format!("{}: {label}", p.name), &z, None),
                art,
                anchor: if key == "inPlay" { format!("inplay:{}", p.seat) } else { format!("zone:{}:{key}", p.seat) },
                z: i as i32,
            }
        }
    }
}

#[component]
pub fn Opponents(board: Rc<Board>, g: Rc<Geometry>, active: Option<u8>) -> Element {
    let ctx = use_table();
    let b = &board;
    let them = active.and_then(|s| b.others().iter().find(|o| o.seat == s).cloned());
    rsx! {
        for (o, r) in b.others().iter().zip(g.opponent_bars.iter()) {
            PlayerBar { key: "{o.seat}", player: o.clone(), at: *r, you: false, active: Some(o.seat) == active }
        }
        if let Some(o) = them {
            {
                let hand_n = o.hand.count as usize;
                let row = g.opp_hand_row(hand_n.min(12));
                let hand = zone_popup(format!("{}'s hand", o.name), &o.hand, None);
                let deck = zone_popup(format!("{}'s deck", o.name), &o.deck, None);
                let discard = zone_popup(format!("{}'s discard pile", o.name), &o.discard, None);
                let hand_box = if hand_n > 0 {
                    Rect::new(row.xs[0], row.y, row.xs[row.xs.len() - 1] + row.w - row.xs[0], row.h)
                } else {
                    Rect::new(g.opp_hand_x, g.opp_hand_y, row.w, row.h)
                };
                rsx! {
                    div {
                        class: "dt-opp-hand",
                        style: "{hand_box.style()}",
                        "data-anchor": "hand:{o.seat}",
                        onclick: move |e| { e.stop_propagation(); ctx.open(hand.clone()); },
                        for (i, x) in row.xs.iter().enumerate() {
                            TableCard {
                                key: "{i}",
                                name: "",
                                size: CardSize::Full,
                                back: true,
                                inverted: true,
                                count: (i + 1 == row.xs.len()).then_some(o.hand.count),
                                at: Rect::new(x - hand_box.x, 0.0, row.w, row.h),
                            }
                        }
                    }
                    if o.discard.count > 0 {
                        TableCard {
                            name: o.discard_top.clone().unwrap_or_default(),
                            size: CardSize::Full,
                            back: o.discard_top.is_none(),
                            inverted: true,
                            at: g.opp_discard,
                            anchor: format!("discard:{}", o.seat),
                            opens: discard,
                            class: "dt-peek",
                        }
                    } else {
                        div { class: "dt-anchor-box", style: "{g.opp_discard.style()}", "data-anchor": "discard:{o.seat}" }
                    }
                    if o.deck.count > 0 {
                        TableCard {
                            name: "",
                            size: CardSize::Full,
                            back: true,
                            inverted: true,
                            count: Some(o.deck.count),
                            at: g.opp_deck,
                            anchor: format!("deck:{}", o.seat),
                            opens: deck,
                            class: "dt-peek",
                        }
                    } else {
                        div { class: "dt-anchor-box", style: "{g.opp_deck.style()}", "data-anchor": "deck:{o.seat}" }
                    }
                    PlayerTabs { player: o.clone(), g: g.clone(), you: false, show_in_play: !o.is_turn }
                }
            }
        }
    }
}

/// The turn player's cards in play, in the order played, from the left, each over the
/// last and stepping down 0, 4 and 8vh in turn.
#[component]
pub fn PlayArea(board: Rc<Board>, g: Rc<Geometry>, ui: Option<Rc<Ui>>, sel: Selection) -> Element {
    let pa = &board.play_area;
    let Some(who) = pa.player.clone() else {
        return rsx! {};
    };
    let yours = board.you.as_ref().is_some_and(|y| y.seat == who.seat);
    let theirs = board.front().is_none_or(|f| f.seat != who.seat);
    let mut slots: Vec<Stack> = pa.order.clone();
    let shown: u32 = slots.iter().map(|s| s.count).sum();
    if pa.cards.count > shown {
        slots.push(Stack {
            card: String::new(),
            count: pa.cards.count - shown,
        });
    }
    let mut marked: Vec<String> = vec![];
    let row = g.play_row(slots.len());
    let area = Rect::new(
        g.play_x,
        g.play_y,
        g.play_right - g.play_x,
        g.play_h + 8.0 * g.vh,
    );
    rsx! {
        div {
            class: "dt-play {seat_class(who.seat)}",
            class: if theirs { "dt-play-theirs" },
            style: "{area.style()}",
            "data-player": "{who.seat}",
            "data-anchor": "inplay:{who.seat}",
            for (i, s) in slots.iter().enumerate() {
                {
                    let mark = if yours && !s.card.is_empty() && !marked.contains(&s.card) {
                        marked.push(s.card.clone());
                        mark_at(&ui, &sel, Spot::InPlay(s.card.clone()))
                    } else {
                        Default::default()
                    };
                    let at = Rect::new(row.xs[i] - area.x, g.play_top(i) - area.y, row.w, row.h);
                    rsx! {
                        TableCard {
                            key: "{i}{s.card}",
                            name: s.card.clone(),
                            size: CardSize::Full,
                            back: s.card.is_empty(),
                            count: (s.count > 1).then_some(s.count),
                            mark,
                            at,
                            z: 100 + 6 * i as i32,
                            class: "dt-slot",
                        }
                    }
                }
            }
        }
    }
}

/// The most backs drawn for a watched hand.
const WATCHED_HAND_BACKS: u32 = 12;

/// The area of the seat's own player: their bar, deck and discard pile, hand and tabs.
/// Someone watching sees the first player there.
#[component]
pub fn YourArea(board: Rc<Board>, g: Rc<Geometry>, ui: Option<Rc<Ui>>, sel: Selection) -> Element {
    let Some(you) = board.front().cloned() else {
        return rsx! {};
    };
    let watching = board.watching;
    let (deck, discard) = if watching {
        (
            zone_popup(format!("{}'s deck", you.name), &you.deck, None),
            zone_popup(format!("{}'s discard pile", you.name), &you.discard, None),
        )
    } else {
        (
            zone_popup(
                "Your deck".into(),
                &you.deck,
                Some(if you.deck_order_known {
                    "You know its order.".into()
                } else {
                    "What you know is in it, in no order.".into()
                }),
            ),
            zone_popup("Your discard pile".into(), &you.discard, None),
        )
    };
    // Someone watching sees a hand as its player's opponents do: backs, one a card, the
    // last with their number; what is known of it opens from them.
    let hand = watching.then(|| zone_popup(format!("{}'s hand", you.name), &you.hand, None));
    let mut slots: Vec<(Stack, bool)> = if watching {
        vec![]
    } else {
        you.hand.known.iter().map(|s| (s.clone(), false)).collect()
    };
    let faces = slots.len();
    let backs = if watching {
        you.hand.count.min(WATCHED_HAND_BACKS)
    } else {
        you.hand.unknown.min(1)
    };
    for i in 0..backs {
        let last = i + 1 == backs;
        let count = match (watching, last) {
            (true, true) => you.hand.count,
            (false, _) => you.hand.unknown,
            _ => 1,
        };
        slots.push((
            Stack {
                card: String::new(),
                count,
            },
            true,
        ));
    }
    let row = g.hand_row(slots.len());
    let hand_box = Rect::new(g.hand_x, g.hand_y, g.hand_right - g.hand_x, g.hand_h);
    let your_turn = board.play_area.player.as_ref().map(|w| w.seat) == Some(you.seat);
    let click_answers = ui.as_ref().is_some_and(|u| u.click_answers);
    let ctx = use_table();
    rsx! {
        PlayerBar { player: you.clone(), at: g.your_bar, you: true, active: false }
        if you.discard.count > 0 {
            TableCard {
                name: you.discard_top.clone().unwrap_or_default(),
                size: CardSize::Full,
                back: you.discard_top.is_none(),
                at: g.your_discard,
                anchor: format!("discard:{}", you.seat),
                opens: discard,
                class: "dt-own-pile",
            }
        } else {
            div { class: "dt-anchor-box", style: "{g.your_discard.style()}", "data-anchor": "discard:{you.seat}" }
        }
        if you.deck.count > 0 {
            TableCard {
                name: "",
                size: CardSize::Full,
                back: true,
                count: Some(you.deck.count),
                at: g.your_deck,
                anchor: format!("deck:{}", you.seat),
                opens: deck,
                class: "dt-own-pile",
            }
        } else {
            div { class: "dt-anchor-box", style: "{g.your_deck.style()}", "data-anchor": "deck:{you.seat}" }
        }
        div {
            class: "dt-hand {seat_class(you.seat)}",
            class: if watching { "dt-click" },
            style: "{hand_box.style()}",
            "data-anchor": "hand:{you.seat}",
            onclick: move |e| {
                if let Some(p) = hand.clone() {
                    e.stop_propagation();
                    ctx.open(p);
                }
            },
            for (i, (s, back)) in slots.iter().enumerate() {
                {
                    let mark = if *back { Default::default() } else { mark_at(&ui, &sel, Spot::Hand(s.card.clone())) };
                    let all = mark.target.is_some()
                        && !mark.tone.is_empty()
                        && (click_answers && mark.tone == "play" || !click_answers && s.count > 1 && mark.options > 1);
                    let key = if *back { format!("back{}", i - faces) } else { format!("false{}", s.card) };
                    rsx! {
                        TableCard {
                            key: "{key}",
                            name: s.card.clone(),
                            size: CardSize::Full,
                            back: *back,
                            count: (s.count > 1).then_some(s.count),
                            mark,
                            all,
                            at: Rect::new(row.xs[i] - hand_box.x, 0.0, row.w, row.h),
                            z: 2000 + 2 * i as i32,
                            class: "dt-slot",
                        }
                    }
                }
            }
        }
        PlayerTabs { player: you.clone(), g: g.clone(), you: true, show_in_play: !your_turn }
    }
}
