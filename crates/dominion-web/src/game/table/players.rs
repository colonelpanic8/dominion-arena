//! The players as dominion.games shows them: a bar per player at the left (opponents at
//! the top, you above your deck), the active opponent's hand, deck and discard pile
//! peeking upside down from the top edge, the turn player's play row, your hand, and
//! tabs for mats and set-aside cards.

use super::layout::{Geometry, Rect};
use super::pieces::{CardName, Reminder, TableCard, cube_icon, seat_class};
use super::supply::{Badges, Tab};
use super::{Composition, DecksView, Popup, Spot, Ui, mark_at, use_table};
use crate::app::use_cards;
use crate::assets::use_assets;
use crate::card::CardSize;
use crate::game::board::{
    Badge, Board, CountsKey, PlayerEntry, Stack, Uncertain, Zone, owned_side_by_side, sort_counts,
};
use crate::game::state::Selection;
use dioxus::prelude::*;
use std::rc::Rc;

fn zone_popup(title: String, z: &Zone, note: Option<String>) -> Popup {
    Popup {
        title,
        stacks: z.known.clone(),
        unknown: z.unknown,
        note,
        composition: None,
        played: false,
    }
}

/// A player's name bar: name, VP, and under them their tokens and held landscapes.
#[component]
fn PlayerBar(player: PlayerEntry, at: Rect, you: bool, active: bool) -> Element {
    let ctx = use_table();
    let assets = use_assets();
    let p = &player;
    let seat = p.seat;
    let reminders: Vec<String> = p.states.iter().chain(&p.artifacts).cloned().collect();
    let cubes = p.projects.clone();
    let cube = assets.file_url(&cube_icon(seat));
    let account = p.account.clone();
    let away = account.as_ref().is_some_and(|a| !a.connected);
    let clock = crate::game::use_clock().and_then(|c| crate::game::meta::seat_clock(&c, seat));
    // The name shrinks to fit beside the VP shield (and the connection mark).
    let vh = at.h / if you { 6.0 } else { 5.5 };
    let chars = p.name.chars().count().max(1) as f64;
    let beside =
        if p.account.is_some() { 7.5 } else { 5.5 } + if clock.is_some() { 5.0 } else { 0.0 };
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
                if let Some(c) = &clock {
                    span {
                        class: "dt-clock",
                        class: if c.running { "dt-clock-running" },
                        class: if let Some(level) = c.level { "dt-clock-{level}" },
                        title: if c.running { "Time left: running" } else { "Time left" },
                        "data-clock": "{seat}",
                        if let Some(free) = &c.free {
                            span { class: "dt-clock-free", title: "Free time for this decision", "{free}" }
                        }
                        "{c.text}"
                    }
                }
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
                for c in reminders.iter() {
                    Reminder { key: "r{c}", name: c.clone() }
                }
                for c in p.boons.iter() {
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
                if let Some((text, title)) = p.level.map(crate::game::meta::level_text) {
                    span { class: "dt-bar-who", "data-level": "{seat}", title: "{title}", "{text}" }
                }
            }
        }
    }
}

/// A player's mats, set-aside cards and (off their turn) cards in play, as tabs.
fn tabs(p: &PlayerEntry, show_in_play: bool) -> Vec<(String, String, Zone, Option<String>)> {
    let mut out = vec![];
    if show_in_play && p.in_play.count > 0 {
        let shown: u32 = p.play_order.iter().map(|s| s.count).sum();
        let played = Zone {
            count: p.in_play.count,
            known: p.play_order.clone(),
            unknown: p.in_play.count.saturating_sub(shown),
        };
        out.push(("inPlay".into(), "In play".into(), played, None));
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
    rsx! {
        for (i, (key, label, z, art)) in list.into_iter().enumerate() {
            Tab {
                key: "{key}",
                label: label.clone(),
                count: z.count,
                at: Rect::new(i as f64 * w * 0.95, y, w, 4.0 * g.vh),
                popup: Popup {
                    played: key == "inPlay",
                    ..zone_popup(format!("{}: {label}", p.name), &z, None)
                },
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
                let slots = hand_slots(&o.hand, b.watching);
                let row = g.opp_hand_row(slots.len());
                let hand = zone_popup(format!("{}'s hand", o.name), &o.hand, None);
                let deck = deck_popup(&o);
                let hand_box = if !slots.is_empty() {
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
                        for (i, (s, back)) in slots.iter().enumerate() {
                            TableCard {
                                key: "{i}",
                                name: s.card.clone(),
                                size: CardSize::Full,
                                back: *back,
                                inverted: true,
                                count: (s.count > 1).then_some(s.count),
                                at: Rect::new(row.xs[i] - hand_box.x, 0.0, row.w, row.h),
                                z: if *back { (2 * slots.len() - 2 * i) as i32 } else { 0 },
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
                            opens: deck.clone(),
                            class: "dt-peek",
                        }
                    } else {
                        EmptyPile { class: "dt-peek", at: g.opp_discard, anchor: format!("discard:{}", o.seat), opens: deck.clone() }
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
                            opens: deck.clone(),
                            class: "dt-peek",
                        }
                    } else {
                        EmptyPile { class: "dt-peek", at: g.opp_deck, anchor: format!("deck:{}", o.seat), opens: deck.clone() }
                    }
                    PlayerTabs { player: o.clone(), g: g.clone(), you: false, show_in_play: !o.is_turn }
                }
            }
        }
    }
}

/// The turn player's cards in play, in the order played, from the left, each over the
/// last and stepping down 0, 4 and 8vh in turn: dominion.games' row, in which the
/// overlap says nothing about which card played which.
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
    // Tokens on a lasting effect's card (Garrison's), one effect to each copy in play.
    let mut tokens: Vec<_> = board
        .lasting
        .iter()
        .filter(|l| l.owner.seat == who.seat)
        .filter_map(|l| Some((l.card.as_str(), l.count.filter(|&n| n > 0)?)))
        .collect();
    let slot_badges: Vec<Vec<Badge>> = slots
        .iter()
        .map(|s| {
            let Some(i) = tokens.iter().position(|(card, _)| *card == s.card) else {
                return vec![];
            };
            let (_, n) = tokens.remove(i);
            vec![Badge {
                kind: "tokens".into(),
                text: format!("{n} token(s) on {}", s.card),
                count: Some(n),
                player: None,
                card: None,
            }]
        })
        .collect();
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
                            badges: slot_badges[i].clone(),
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

fn hand_slots(hand: &Zone, faces: bool) -> Vec<(Stack, bool)> {
    let mut slots: Vec<_> = if faces {
        hand.known.iter().map(|s| (s.clone(), false)).collect()
    } else {
        vec![]
    };
    let unknown = if faces { hand.unknown } else { hand.count };
    let backs = if faces { unknown.min(1) } else { unknown };
    for i in 0..backs {
        slots.push((
            Stack {
                card: String::new(),
                count: if i == 0 { unknown } else { 1 },
            },
            true,
        ));
    }
    slots
}

fn deck_popup(p: &PlayerEntry) -> Popup {
    Popup {
        title: format!("{}'s deck composition", p.name),
        stacks: vec![],
        unknown: 0,
        note: None,
        composition: Some(Composition::Seat(p.seat)),
        played: false,
    }
}

/// Where an empty deck or discard pile would be: still opens the deck composition.
#[component]
fn EmptyPile(at: Rect, anchor: String, opens: Popup, class: &'static str) -> Element {
    let ctx = use_table();
    let key = opens.clone();
    rsx! {
        div {
            class: "dt-anchor-box dt-click {class}", style: "{at.style()}",
            "data-anchor": "{anchor}", role: "button", tabindex: 0,
            title: "Deck composition",
            onclick: move |e| { e.stop_propagation(); ctx.open(opens.clone()); },
            onkeydown: move |e| {
                if e.key() == Key::Enter || e.key() == Key::Character(" ".into()) {
                    e.stop_propagation(); e.prevent_default(); ctx.open(key.clone());
                }
            },
        }
    }
}

/// A zone's cards as a grid, or that it is empty.
#[component]
fn ZoneCards(zone: Zone) -> Element {
    rsx! {
        div { class: "dt-composition-grid",
            if zone.count == 0 {
                p { class: "dt-composition-empty", "Empty." }
            } else {
                CompositionCards { zone }
            }
        }
    }
}

/// The cards of a zone as tiles, most copies first, and its unknown cards as one back.
#[component]
fn CompositionCards(zone: Zone) -> Element {
    let mut known = zone.known.clone();
    known.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.card.cmp(&b.card)));
    rsx! {
        for s in known {
            div { key: "{s.card}", class: "dt-composition-tile",
                TableCard {
                    name: s.card.clone(),
                    size: CardSize::Full,
                    count: (s.count > 1).then_some(s.count),
                }
            }
        }
        if zone.unknown > 0 {
            div { class: "dt-composition-tile",
                TableCard { name: "", size: CardSize::Full, back: true, count: Some(zone.unknown) }
                div { class: "dt-composition-name", "Unknown" }
            }
        }
    }
}

/// One player's deck composition in its own window.
#[component]
pub fn DeckComposition(board: Rc<Board>, seat: u8) -> Element {
    let ctx = use_table();
    let Some(p) = board.players().find(|p| p.seat == seat) else {
        return rsx! {};
    };
    rsx! {
        div {
            class: "dt-composition",
            role: "dialog",
            "aria-label": "{p.name}'s deck composition",
            onclick: move |e| e.stop_propagation(),
            div { class: "dt-composition-head",
                span { "{p.name}'s deck composition" }
                div { class: "dt-composition-actions",
                    button {
                        class: "dt-composition-tab", "data-action": "all-decks",
                        onclick: move |_| ctx.open(all_decks_popup()),
                        "All decks"
                    }
                    button { class: "dt-composition-x", "data-close": "popup", onclick: move |_| ctx.close(), "✕" }
                }
            }
            div { class: "dt-composition-body",
                CompositionBody { board: board.clone(), seat, shared: true }
            }
        }
    }
}

/// Everything the player owns, then each part of it one at a time: hovering, focusing
/// or clicking a tab shows that part. `shared` adds what is uncertain about every
/// player's cards.
#[component]
fn CompositionBody(board: Rc<Board>, seat: u8, shared: bool) -> Element {
    let mut open = use_signal(|| None::<(u8, String)>);
    let Some(p) = board.players().find(|p| p.seat == seat) else {
        return rsx! {};
    };
    let mut sections = p.composition().into_iter();
    let whole = sections.next();
    let rest = p.rest();
    let combined = rest.is_some();
    let parts: Vec<_> = rest.into_iter().chain(sections).collect();
    // A tab chosen for another player's deck says nothing about this one's.
    let shown = open
        .read()
        .as_ref()
        .filter(|(of, _)| *of == seat)
        .and_then(|(_, key)| parts.iter().find(|s| s.key == *key))
        .or(parts.first())
        .cloned();
    // The combined tab holds the cards known to be owned and seen nowhere else.
    let unlocated: &[Stack] = if combined { &[] } else { &p.unlocated };
    let shared_notes: &[Uncertain] = if shared { &board.uncertain } else { &[] };
    let uncertain: Vec<_> = p.uncertain.iter().chain(shared_notes).cloned().collect();
    rsx! {
        if let Some(whole) = whole {
            section { class: "dt-composition-section", "data-zone": "{whole.key}",
                h3 { "{whole.label} ({whole.zone.count})" }
                ZoneCards { zone: whole.zone.clone() }
            }
        }
        if let Some(shown) = shown {
            section { class: "dt-composition-section",
                div { class: "dt-composition-tabs", role: "tablist",
                    for s in parts.iter() {
                        {
                            let key = s.key.clone();
                            let (k1, k2) = (key.clone(), key.clone());
                            rsx! {
                                button {
                                    key: "{key}",
                                    class: "dt-composition-tab",
                                    class: if key == shown.key { "on" },
                                    role: "tab",
                                    "aria-selected": key == shown.key,
                                    onmouseenter: move |_| open.set(Some((seat, k1.clone()))),
                                    onfocus: move |_| open.set(Some((seat, k2.clone()))),
                                    onclick: move |_| open.set(Some((seat, key.clone()))),
                                    "{s.label} ({s.zone.count})"
                                }
                            }
                        }
                    }
                }
                div { class: "dt-composition-panel", role: "tabpanel", "data-zone": "{shown.key}",
                    ZoneCards { zone: shown.zone.clone() }
                }
            }
        }
        if !unlocated.is_empty() || !uncertain.is_empty() {
            section { class: "dt-composition-section",
                h3 { "Uncertain locations" }
                if !unlocated.is_empty() {
                    p { class: "dt-composition-note",
                        "Known to be owned, location unknown: "
                        for (i, s) in unlocated.iter().enumerate() {
                            if i > 0 { "; " }
                            CardName { name: s.card.clone() }
                            " × {s.count}"
                        }
                    }
                }
                for (i, u) in uncertain.into_iter().enumerate() {
                    UncertainNote { key: "uncertain{i}", u }
                }
            }
        }
    }
}

#[component]
fn UncertainNote(u: Uncertain) -> Element {
    rsx! {
        p { class: "dt-composition-note",
            "{u.count} of "
            for (j, s) in u.candidates.iter().enumerate() {
                if j > 0 { "; " }
                CardName { name: s.card.clone() }
                " × {s.count}"
            }
            if u.unknown > 0 { "; Unknown × {u.unknown}" }
            " are among {u.text}."
        }
    }
}

pub fn all_decks_popup() -> Popup {
    Popup {
        title: "Deck compositions".into(),
        stacks: vec![],
        unknown: 0,
        note: None,
        composition: Some(Composition::All),
        played: false,
    }
}

/// Every player's deck composition at once: each one's whole window side by side, or a
/// row per card with how many each player owns.
#[component]
pub fn AllDecks(board: Rc<Board>) -> Element {
    let ctx = use_table();
    let mut view = ctx.decks_view;
    let mode = view();
    let style = match mode {
        DecksView::Columns => format!(
            "width: min(calc({} * 60 * var(--vh)), 97%);",
            board.players().count()
        ),
        DecksView::Counts => "width: min(calc(120 * var(--vh)), 94%);".into(),
    };
    rsx! {
        div {
            class: "dt-composition dt-decks",
            style: "{style}",
            role: "dialog",
            "aria-label": "Deck compositions",
            onclick: move |e| e.stop_propagation(),
            div { class: "dt-composition-head",
                span { "Deck compositions" }
                div { class: "dt-composition-actions",
                    for (m, label) in [(DecksView::Columns, "Side by side"), (DecksView::Counts, "Card counts")] {
                        button {
                            key: "{label}",
                            class: "dt-composition-tab",
                            class: if mode == m { "on" },
                            "aria-pressed": mode == m,
                            onclick: move |_| view.set(m),
                            "{label}"
                        }
                    }
                    button { class: "dt-composition-x", "data-close": "popup", onclick: move |_| ctx.close(), "✕" }
                }
            }
            div { class: "dt-composition-body",
                match mode {
                    DecksView::Columns => rsx! { DecksColumns { board } },
                    DecksView::Counts => rsx! { DecksCounts { board } },
                }
            }
        }
    }
}

/// A player's name over their column: opens their own deck composition.
#[component]
fn DecksPlayer(player: PlayerEntry) -> Element {
    let ctx = use_table();
    let opens = deck_popup(&player);
    rsx! {
        button {
            class: "dt-decks-player {seat_class(player.seat)}",
            title: "{player.name}'s deck composition",
            onclick: move |_| ctx.open(opens.clone()),
            span { class: "dt-decks-name", "{player.name}" }
            span { class: "dt-decks-total", "{player.owned.count} cards" }
        }
    }
}

/// Each player's deck composition in a column of its own.
#[component]
fn DecksColumns(board: Rc<Board>) -> Element {
    let players: Vec<PlayerEntry> = board.players().cloned().collect();
    let columns = format!(
        "grid-template-columns: repeat({}, minmax(calc(34 * var(--vh)), 1fr));",
        players.len()
    );
    rsx! {
        div { class: "dt-decks-columns", style: "{columns}",
            for p in players {
                div { key: "{p.seat}", class: "dt-decks-column",
                    DecksPlayer { player: p.clone() }
                    CompositionBody { board: board.clone(), seat: p.seat, shared: false }
                }
            }
        }
        if !board.uncertain.is_empty() {
            section { class: "dt-composition-section",
                h3 { "Uncertain locations" }
                for (i, u) in board.uncertain.iter().enumerate() {
                    UncertainNote { key: "{i}", u: u.clone() }
                }
            }
        }
    }
}

/// A column's header in the card counts: sorts the rows by it.
#[component]
fn CountsHeader(
    class: String,
    name: String,
    sub: String,
    title: String,
    /// Whether the rows are sorted by this column, and if so whether smallest first.
    sorted: Option<bool>,
    onclick: EventHandler<()>,
) -> Element {
    let arrow = match sorted {
        Some(true) => " ▲",
        Some(false) => " ▼",
        None => "",
    };
    rsx! {
        button {
            class: "dt-decks-player {class}",
            class: if sorted.is_some() { "sorted" },
            title: "{title}",
            "aria-sort": match sorted {
                Some(true) => "ascending",
                Some(false) => "descending",
                None => "none",
            },
            onclick: move |_| onclick.call(()),
            span { class: "dt-decks-name", "{name}{arrow}" }
            span { class: "dt-decks-total", "{sub}" }
        }
    }
}

/// A row per card any player owns, with how many each owns, sortable by any player's
/// counts; two players also get the difference between them.
#[component]
fn DecksCounts(board: Rc<Board>) -> Element {
    let cards = use_cards();
    let mut sort = use_table().decks_sort;
    let players: Vec<&PlayerEntry> = board.players().collect();
    let pair = players.len() == 2;
    let mut rows = owned_side_by_side(&players, &cards);
    let order = sort();
    let sorted_by = order.by.filter(|k| match k {
        CountsKey::Player(i) => *i < players.len(),
        CountsKey::Delta => pair,
    });
    if let Some(key) = sorted_by {
        sort_counts(&mut rows, key, order.ascending);
    }
    let sorted = move |key| (sorted_by == Some(key)).then_some(order.ascending);
    let unknown: Vec<u32> = players.iter().map(|p| p.owned.unknown).collect();
    let columns = format!(
        "grid-template-columns: calc(16 * var(--vh)) repeat({}, minmax(0, 1fr));",
        players.len() + usize::from(pair)
    );
    let cell = |n: u32, most: bool| {
        rsx! {
            div {
                class: "dt-decks-n",
                class: if n == 0 { "none" },
                class: if most { "most" },
                if n == 0 { "–" } else { "{n}" }
            }
        }
    };
    let most = |counts: &[u32]| {
        let max = counts.iter().copied().max().unwrap_or(0);
        let differ = counts.iter().any(|&n| n != max);
        move |n: u32| differ && n == max
    };
    let seats: Vec<u8> = players.iter().map(|p| p.seat).collect();
    let delta = |a: u32, b: u32| {
        let d = i64::from(a) - i64::from(b);
        let leader = match d.signum() {
            1 => seat_class(seats[0]),
            -1 => seat_class(seats[1]),
            _ => String::new(),
        };
        rsx! {
            div {
                class: "dt-decks-n dt-decks-delta {leader}",
                class: if d == 0 { "none" },
                if d == 0 { "–" } else { "{d:+}" }
            }
        }
    };
    let total_delta = pair.then(|| {
        let d = i64::from(players[0].owned.count) - i64::from(players[1].owned.count);
        if d == 0 {
            "even".to_string()
        } else {
            format!("{d:+} cards")
        }
    });
    rsx! {
        div { class: "dt-decks-grid", style: "{columns}",
            div {}
            for (i, p) in players.iter().enumerate() {
                CountsHeader {
                    key: "h{p.seat}",
                    class: seat_class(p.seat),
                    name: p.name.clone(),
                    sub: format!("{} cards", p.owned.count),
                    title: format!("Sort by {}'s counts", p.name),
                    sorted: sorted(CountsKey::Player(i)),
                    onclick: move |_| sort.set(sort().toggle(CountsKey::Player(i))),
                }
            }
            if let Some(sub) = total_delta {
                CountsHeader {
                    class: "dt-decks-diff",
                    name: "Difference",
                    sub,
                    title: format!("Sort by {} − {}", players[0].name, players[1].name),
                    sorted: sorted(CountsKey::Delta),
                    onclick: move |_| sort.set(sort().toggle(CountsKey::Delta)),
                }
            }
            for (card, counts) in rows.iter() {
                div { key: "c{card}", class: "dt-decks-card",
                    TableCard { name: card.clone(), size: CardSize::Micro }
                }
                for (i, &n) in counts.iter().enumerate() {
                    Fragment { key: "{card}{i}", {cell(n, most(counts)(n))} }
                }
                if pair {
                    Fragment { key: "{card}d", {delta(counts[0], counts[1])} }
                }
            }
            if unknown.iter().any(|&n| n > 0) {
                div { class: "dt-decks-card dt-decks-label", "Unknown" }
                for (i, &n) in unknown.iter().enumerate() {
                    Fragment { key: "u{i}", {cell(n, false)} }
                }
                if pair {
                    div {}
                }
            }
        }
    }
}

/// The area of the seat's own player: their bar, deck and discard pile, hand and tabs.
/// Someone watching sees the first player there.
#[component]
pub fn YourArea(board: Rc<Board>, g: Rc<Geometry>, ui: Option<Rc<Ui>>, sel: Selection) -> Element {
    let Some(you) = board.front().cloned() else {
        return rsx! {};
    };
    let watching = board.watching;
    let deck = deck_popup(&you);
    let hand = watching.then(|| zone_popup(format!("{}'s hand", you.name), &you.hand, None));
    let slots = hand_slots(&you.hand, true);
    let faces = slots.iter().filter(|(_, back)| !back).count();
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
                opens: deck.clone(),
                class: "dt-own-pile",
            }
        } else {
            EmptyPile { class: "dt-own-pile", at: g.your_discard, anchor: format!("discard:{}", you.seat), opens: deck.clone() }
        }
        if you.deck.count > 0 {
            TableCard {
                name: "",
                size: CardSize::Full,
                back: true,
                count: Some(you.deck.count),
                at: g.your_deck,
                anchor: format!("deck:{}", you.seat),
                opens: deck.clone(),
                class: "dt-own-pile",
            }
        } else {
            EmptyPile { class: "dt-own-pile", at: g.your_deck, anchor: format!("deck:{}", you.seat), opens: deck.clone() }
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
                    let ways = ui.as_ref().map(|u| u.ways_of(mark.target)).unwrap_or_default();
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
                            ways,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn visible_hands_show_known_cards_and_keep_unknown_cards_face_down() {
        let hand = Zone {
            count: 5,
            known: vec![Stack {
                card: "Copper".into(),
                count: 3,
            }],
            unknown: 2,
        };
        let visible = hand_slots(&hand, true);
        assert_eq!(
            visible,
            vec![
                (
                    Stack {
                        card: "Copper".into(),
                        count: 3
                    },
                    false
                ),
                (
                    Stack {
                        card: String::new(),
                        count: 2
                    },
                    true
                ),
            ]
        );
        let hidden = hand_slots(&hand, false);
        assert_eq!(hidden.len(), 5);
        assert!(hidden.iter().all(|(s, back)| s.card.is_empty() && *back));
        assert_eq!(hidden[0].0.count, 5);
    }
}
