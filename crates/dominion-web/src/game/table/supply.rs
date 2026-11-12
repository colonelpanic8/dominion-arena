//! The Supply as dominion.games lays it out: the basic piles in two columns at the left
//! edge (Victory cards and Curses, Treasures), the kingdom in a grid filled from the
//! bottom row up by cost, landscapes in a column beside it, non-Supply piles as strips,
//! and the trash as a tab on the top edge.

use super::layout::{Geometry, Rect};
use super::pieces::{BadgeView, TableCard};
use super::{Popup, Spot, Ui, mark_at, use_table};
use crate::app::use_cards;
use crate::card::CardSize;
use crate::game::board::{Board, CardDeck, Holding, LandscapeEntry, PileEntry};
use crate::game::state::Selection;
use dioxus::prelude::*;
use std::rc::Rc;

fn is_treasure(p: &PileEntry) -> bool {
    p.types.iter().any(|t| t == "Treasure")
}

/// The basic piles: Victory cards, Curses and the rest top to bottom, most expensive
/// first; and the Treasures likewise.
pub fn basic_columns(b: &Board) -> (Vec<PileEntry>, Vec<PileEntry>) {
    let (mut treasures, mut rest): (Vec<_>, Vec<_>) =
        b.supply.basic.iter().cloned().partition(is_treasure);
    let key = |p: &PileEntry| {
        let c = crate::game::board::cost_key(p.cost.as_ref());
        (std::cmp::Reverse(c), p.name.clone())
    };
    let curse_last = |p: &PileEntry| p.types.iter().any(|t| t == "Curse");
    treasures.sort_by_key(key);
    rest.sort_by_key(|p| (curse_last(p), key(p)));
    (rest, treasures)
}

/// What sits beside the kingdom, in order: landscapes by cost and name, then Artifacts,
/// Boons and Hexes.
#[derive(Clone, Debug, PartialEq)]
pub enum Wide {
    Landscape(LandscapeEntry),
    Artifact(Holding),
    Deck(&'static str, CardDeck),
}

pub fn wides(b: &Board) -> Vec<Wide> {
    let mut ls = b.landscapes.clone();
    ls.sort_by(|a, b| {
        crate::game::board::cost_key(a.cost.as_ref())
            .cmp(&crate::game::board::cost_key(b.cost.as_ref()))
            .then_with(|| a.card.cmp(&b.card))
    });
    let mut out: Vec<Wide> = ls.into_iter().map(Wide::Landscape).collect();
    out.extend(b.artifacts.iter().cloned().map(Wide::Artifact));
    if let Some(d) = b.boons.clone() {
        out.push(Wide::Deck("Boons", d));
    }
    if let Some(d) = b.hexes.clone() {
        out.push(Wide::Deck("Hexes", d));
    }
    out
}

/// The kingdom piles, cheapest first.
fn kingdom(b: &Board) -> Vec<PileEntry> {
    let mut piles: Vec<PileEntry> = b.supply.kingdom.iter().flatten().cloned().collect();
    piles.sort_by(|a, b| {
        crate::game::board::cost_key(a.cost.as_ref())
            .cmp(&crate::game::board::cost_key(b.cost.as_ref()))
            .then_with(|| a.name.cmp(&b.name))
    });
    piles
}

#[component]
pub fn SupplyArea(
    board: Rc<Board>,
    g: Rc<Geometry>,
    ui: Option<Rc<Ui>>,
    sel: Selection,
) -> Element {
    let (victory, treasures) = basic_columns(&board);
    let kingdom = kingdom(&board);
    let wide = wides(&board);
    let mut non_supply = board.non_supply.clone();
    non_supply.sort_by(|a, b| {
        crate::game::board::cost_key(b.cost.as_ref())
            .cmp(&crate::game::board::cost_key(a.cost.as_ref()))
            .then_with(|| a.name.cmp(&b.name))
    });
    let basic: Vec<(PileEntry, Rect)> = victory
        .into_iter()
        .zip(g.victory.iter().copied())
        .chain(treasures.into_iter().zip(g.treasures.iter().copied()))
        .collect();
    rsx! {
        div { class: "dt-supply",
            for (p, r) in basic.iter() {
                Pile { key: "b{p.name}", pile: p.clone(), at: *r, ui: ui.clone(), sel: sel.clone() }
            }
            for (p, r) in kingdom.iter().zip(g.kingdom.iter()) {
                Pile { key: "k{p.name}", pile: p.clone(), at: *r, ui: ui.clone(), sel: sel.clone() }
            }
            for (w, r) in wide.iter().zip(g.landscapes.iter()) {
                match w.clone() {
                    Wide::Landscape(l) => rsx! { Landscape { key: "l{l.card}", entry: l, at: *r, ui: ui.clone(), sel: sel.clone() } },
                    Wide::Artifact(a) => rsx! { Artifact { key: "a{a.card}", holding: a, at: *r } },
                    Wide::Deck(label, d) => rsx! { Deck { key: "{label}", label, deck: d, at: *r } },
                }
            }
            for (p, r) in non_supply.iter().zip(g.non_supply.iter()) {
                Pile { key: "n{p.name}", pile: p.clone(), at: *r, ui: ui.clone(), sel: sel.clone(), size: CardSize::Micro }
            }
            TrashTab { board: board.clone(), g: g.clone() }
        }
    }
}

/// A tab on the table's edge naming a mat or the trash, with its count: hovering opens
/// its window, clicking keeps it open.
#[component]
pub fn Tab(
    label: String,
    count: u32,
    at: Rect,
    popup: Popup,
    #[props(default)] art: Option<String>,
    #[props(default)] anchor: Option<String>,
    #[props(default)] z: i32,
) -> Element {
    let ctx = use_table();
    let p_hover = popup.clone();
    let p_click = popup.clone();
    let open = ctx
        .popup
        .read()
        .as_ref()
        .is_some_and(|(q, _)| q.title == popup.title);
    rsx! {
        div {
            class: "dt-tab",
            class: if open { "dt-tab-open" },
            style: "{at.style()}z-index:{10000 - z};",
            "data-anchor": anchor,
            "data-tab": "{label}",
            onmouseenter: move |_| ctx.peek_open(p_hover.clone()),
            onmouseleave: move |_| ctx.peek_close(),
            onclick: move |e| {
                e.stop_propagation();
                ctx.open(p_click.clone());
            },
            if let Some(a) = art {
                div { class: "dt-tab-art", style: "background-image:var(--dt-{a});" }
            }
            span { class: "dt-tab-label", "{label}" }
            span { class: "dt-tab-n", "{count}" }
        }
    }
}

#[component]
fn TrashTab(board: Rc<Board>, g: Rc<Geometry>) -> Element {
    let b = &board;
    let popup = Popup {
        title: "Trash".into(),
        stacks: b.trash.known.clone(),
        unknown: b.trash.unknown,
        note: None,
    };
    let w = 11.0 * g.vw;
    let at = Rect::new(
        (100.0 - super::layout::RIGHT_COLUMN) * g.vw - w,
        0.0,
        w,
        4.0 * g.vh,
    );
    rsx! {
        Tab {
            label: "Trash",
            count: b.trash.count,
            at,
            popup,
            art: "trash-mat".to_string(),
            anchor: "trash".to_string(),
        }
    }
}

/// The anchor of a pile, for cards seen moving to and from it.
pub fn pile_anchor(at: crate::game::board::PileAt) -> String {
    match at {
        crate::game::board::PileAt::Supply(i) => format!("pile:s{i}"),
        crate::game::board::PileAt::NonSupply(i) => format!("pile:n{i}"),
    }
}

#[component]
fn Pile(
    pile: PileEntry,
    at: Rect,
    ui: Option<Rc<Ui>>,
    sel: Selection,
    #[props(default = CardSize::Mini)] size: CardSize,
) -> Element {
    let cards = use_cards();
    let printed = cards.get(&pile.card).and_then(|c| c.cost);
    let cost = pile.cost.clone().filter(|c| {
        printed.is_none_or(|p| (p.coins, p.potions, p.debt) != (c.coins, c.potions, c.debt))
    });
    let mark = mark_at(&ui, &sel, Spot::Pile(pile.at));
    let changed = cost.is_some();
    // A pile of different cards (split or mixed) shows a back peeking out under it.
    let split = !pile.hidden
        && size == CardSize::Mini
        && cards
            .get(&pile.name)
            .is_some_and(|c| c.kind == "randomizer" || c.pile.is_some())
        && pile.count > 1;
    let plus = mark.target.is_some() && size != CardSize::Full;
    rsx! {
        TableCard {
            name: pile.card.clone(),
            size,
            cost,
            count: Some(pile.count),
            anchor: pile_anchor(pile.at),
            mark,
            empty: pile.empty,
            back: pile.hidden,
            badges: pile.badges.clone(),
            at,
            plus,
            split,
            class: format!(
                "dt-pile{}{}{}",
                if pile.kingdom { " dt-kpile" } else { "" },
                if size == CardSize::Micro { " dt-nspile" } else { "" },
                if changed { " dt-cost-changed" } else { "" },
            ),
        }
    }
}

#[component]
fn Landscape(entry: LandscapeEntry, at: Rect, ui: Option<Rc<Ui>>, sel: Selection) -> Element {
    let cards = use_cards();
    let printed = cards.get(&entry.card).and_then(|c| c.cost);
    let cost = entry.cost.clone().filter(|c| {
        printed.is_none_or(|p| (p.coins, p.potions, p.debt) != (c.coins, c.potions, c.debt))
    });
    let mark = mark_at(&ui, &sel, Spot::Landscape(entry.card.clone()));
    let plus = mark.target.is_some();
    rsx! {
        TableCard {
            name: entry.card.clone(),
            size: CardSize::Mini,
            cost,
            mark,
            plus,
            badges: entry.badges.clone(),
            cubes: entry.owners.iter().map(|w| (w.seat, w.name.clone())).collect::<Vec<_>>(),
            class: "dt-landscape dt-kind-{entry.kind}",
            at,
        }
    }
}

/// The Boons or Hexes deck, face down with how many are left, beside its discard pile.
#[component]
fn Deck(label: String, deck: CardDeck, at: Rect) -> Element {
    let ctx = use_table();
    let top = deck.discard.last().cloned();
    let mut stacks: Vec<crate::game::board::Stack> = vec![];
    for c in &deck.discard {
        match stacks.iter_mut().find(|s| &s.card == c) {
            Some(s) => s.count += 1,
            None => stacks.push(crate::game::board::Stack {
                card: c.clone(),
                count: 1,
            }),
        }
    }
    let popup = Popup {
        title: format!("{label}: discarded"),
        stacks,
        unknown: 0,
        note: Some(format!("{} left to draw", deck.count)),
    };
    let open = popup.clone();
    let lower = label.to_lowercase();
    let half = Rect::new(at.x, at.y, at.w / 2.0 - 2.0, at.h);
    let right = Rect::new(at.x + at.w / 2.0 + 2.0, at.y, at.w / 2.0 - 2.0, at.h);
    rsx! {
        div {
            class: "dt-deckback dt-deck-{lower} dt-click",
            style: "{half.style()}",
            "data-deck": "{label}",
            title: "{label}: {deck.count} left to draw",
            onclick: move |e| { e.stop_propagation(); ctx.open(open.clone()); },
            span { class: "dt-deckback-label", "{label}" }
            span { class: "dt-deckback-n", "{deck.count}" }
        }
        match top {
            Some(card) => rsx! {
                TableCard {
                    name: card,
                    size: CardSize::Mini,
                    count: Some(deck.discard.len() as u32),
                    opens: popup,
                    class: "dt-landscape dt-deck-discard",
                    at: right,
                }
            },
            None => rsx! {},
        }
    }
}

#[component]
fn Artifact(holding: Holding, at: Rect) -> Element {
    let cubes: Vec<(u8, String)> = holding
        .holder
        .iter()
        .map(|w| (w.seat, w.name.clone()))
        .collect();
    rsx! {
        TableCard { name: holding.card.clone(), size: CardSize::Mini, class: "dt-landscape", cubes, at }
    }
}

/// Badges in a row, for players.
#[component]
pub fn Badges(badges: Vec<crate::game::board::Badge>) -> Element {
    rsx! {
        span { class: "dt-badges",
            for (i, b) in badges.iter().enumerate() {
                BadgeView { key: "{i}", badge: b.clone() }
            }
        }
    }
}
