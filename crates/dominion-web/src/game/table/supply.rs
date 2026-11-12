//! The Supply: basic piles in columns, the Kingdom in rows, and a strip of non-Supply
//! piles, landscapes, Boons and Hexes, Artifacts and the trash.

use super::pieces::{BadgeView, Cube, TableCard};
use super::{Popup, Spot, Ui, mark_at};
use crate::app::use_cards;
use crate::card::CardSize;
use crate::game::board::{Board, CardDeck, Holding, LandscapeEntry, PileEntry};
use crate::game::state::Selection;
use dioxus::prelude::*;
use std::rc::Rc;

const MAX_BASIC_ROWS: usize = 5;

fn is_treasure(p: &PileEntry) -> bool {
    p.types.iter().any(|t| t == "Treasure")
}

/// The basic piles in columns: Treasures, then the rest, at most five to a column.
fn basic_columns(basic: &[PileEntry]) -> Vec<Vec<PileEntry>> {
    let (treasures, rest): (Vec<_>, Vec<_>) = basic.iter().cloned().partition(is_treasure);
    let mut cols = vec![];
    for group in [treasures, rest] {
        for chunk in group.chunks(MAX_BASIC_ROWS) {
            cols.push(chunk.to_vec());
        }
    }
    cols
}

/// Widths of the strip's items in Kingdom pile widths.
const STRIP_PILE: f32 = 0.72;
const STRIP_WIDE: f32 = 1.08;
const STRIP_TRASH: f32 = 0.95;
/// The width of a Kingdom row, in Kingdom pile widths, less room for gaps.
const ROW: f32 = 4.3;

/// How much the strip's items shrink and in how many rows they wrap: as many as fit in
/// a Kingdom row's width at full size, then at most two rows, shrinking to fit.
fn strip_fit(b: &Board) -> (f32, u32) {
    let wide = b.landscapes.len()
        + b.artifacts.len()
        + usize::from(b.boons.is_some())
        + usize::from(b.hexes.is_some());
    let total = STRIP_PILE * b.non_supply.len() as f32 + STRIP_WIDE * wide as f32 + STRIP_TRASH;
    let scale = (2.0 * ROW / total).min(1.0);
    let rows = (total * scale / ROW).ceil().max(1.0) as u32;
    (scale, rows)
}

#[component]
pub fn SupplyArea(board: Rc<Board>, ui: Option<Rc<Ui>>, sel: Selection) -> Element {
    let cols = basic_columns(&board.supply.basic);
    let brows = cols.iter().map(Vec::len).max().unwrap_or(1).max(4);
    let rows = board.supply.kingdom.len().max(2);
    let (scale, strip_rows) = strip_fit(&board);
    rsx! {
        div {
            class: "dt-supply",
            style: "--rows:{rows};--brows:{brows};--ss:{scale:.3};--srows:{strip_rows};",
            div { class: "dt-basic",
                for (i, col) in cols.iter().enumerate() {
                    div { key: "{i}", class: "dt-bcol",
                        for p in col.iter() {
                            Pile { key: "{p.name}", pile: p.clone(), ui: ui.clone(), sel: sel.clone() }
                        }
                    }
                }
            }
            div { class: "dt-kingdom-col",
                div { class: "dt-kingdom",
                    for (i, row) in board.supply.kingdom.iter().enumerate() {
                        div { key: "{i}", class: "dt-krow",
                            for p in row.iter() {
                                Pile { key: "{p.name}", pile: p.clone(), ui: ui.clone(), sel: sel.clone() }
                            }
                        }
                    }
                }
                Strip { board: board.clone(), ui: ui.clone(), sel: sel.clone() }
            }
        }
    }
}

#[component]
fn Strip(board: Rc<Board>, ui: Option<Rc<Ui>>, sel: Selection) -> Element {
    let ctx = super::use_table();
    let b = &board;
    let trash = Popup {
        title: format!("Trash ({})", b.trash.count),
        stacks: b.trash.known.clone(),
        unknown: b.trash.unknown,
        note: None,
    };
    rsx! {
        div { class: "dt-strip",
            for l in b.landscapes.iter() {
                Landscape { key: "l{l.card}", entry: l.clone(), ui: ui.clone(), sel: sel.clone() }
            }
            if let Some(d) = b.boons.clone() {
                Deck { label: "Boons", deck: d }
            }
            if let Some(d) = b.hexes.clone() {
                Deck { label: "Hexes", deck: d }
            }
            for a in b.artifacts.iter() {
                Artifact { key: "a{a.card}", holding: a.clone() }
            }
            for p in b.non_supply.iter() {
                Pile { key: "ns{p.name}", pile: p.clone(), ui: ui.clone(), sel: sel.clone(), small: true }
            }
            div {
                class: "dt-trash dt-click",
                class: if b.trash.count == 0 { "dt-trash-empty" },
                "data-pile": "trash",
                title: "Open the trash",
                onclick: move |e| {
                    e.stop_propagation();
                    ctx.open(trash.clone());
                },
                span { class: "dt-trash-label", "Trash" }
                span { class: "dt-trash-n", "{b.trash.count}" }
            }
        }
    }
}

#[component]
fn Pile(
    pile: PileEntry,
    ui: Option<Rc<Ui>>,
    sel: Selection,
    #[props(default)] small: bool,
) -> Element {
    let cards = use_cards();
    let printed = cards.get(&pile.card).and_then(|c| c.cost);
    let cost = pile.cost.clone().filter(|c| {
        printed.is_none_or(|p| (p.coins, p.potions, p.debt) != (c.coins, c.potions, c.debt))
    });
    let mark = mark_at(&ui, &sel, Spot::Pile(pile.at));
    let changed = cost.is_some();
    rsx! {
        TableCard {
            name: pile.card.clone(),
            size: CardSize::Mini,
            cost,
            count: Some(pile.count),
            mark,
            dim: pile.empty,
            back: pile.hidden,
            badges: pile.badges.clone(),
            class: format!(
                "dt-pile{}{}{}",
                if pile.kingdom { " dt-kpile" } else { "" },
                if small { " dt-nspile" } else { "" },
                if changed { " dt-cost-changed" } else { "" },
            ),
        }
    }
}

#[component]
fn Landscape(entry: LandscapeEntry, ui: Option<Rc<Ui>>, sel: Selection) -> Element {
    let cards = use_cards();
    let printed = cards.get(&entry.card).and_then(|c| c.cost);
    let cost = entry.cost.clone().filter(|c| {
        printed.is_none_or(|p| (p.coins, p.potions, p.debt) != (c.coins, c.potions, c.debt))
    });
    let mark = mark_at(&ui, &sel, Spot::Landscape(entry.card.clone()));
    rsx! {
        TableCard {
            name: entry.card.clone(),
            size: CardSize::Mini,
            cost,
            mark,
            badges: entry.badges.clone(),
            cubes: entry.owners.iter().map(|w| (w.seat, w.name.clone())).collect::<Vec<_>>(),
            class: "dt-landscape dt-kind-{entry.kind}",
        }
    }
}

/// The Boons or Hexes deck: its size and the last one discarded.
#[component]
fn Deck(label: String, deck: CardDeck) -> Element {
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
    rsx! {
        div { class: "dt-deckbox", "data-deck": "{label}",
            match top {
                Some(card) => rsx! {
                    TableCard {
                        name: card,
                        size: CardSize::Mini,
                        opens: popup,
                        class: "dt-landscape",
                    }
                },
                None => rsx! {
                    div { class: "dt-deckbox-empty dt-deck-{label.to_lowercase()}", "{label}" }
                },
            }
            div { class: "dt-deckbox-label", "{label} · {deck.count} left" }
        }
    }
}

#[component]
fn Artifact(holding: Holding) -> Element {
    rsx! {
        div { class: "dt-artifact",
            TableCard { name: holding.card.clone(), size: CardSize::Mini, class: "dt-landscape" }
            div { class: "dt-deckbox-label",
                match holding.holder {
                    Some(w) => rsx! {
                        Cube { seat: w.seat, name: w.name.clone() }
                        span { class: super::pieces::seat_class(w.seat), "{w.name}" }
                    },
                    None => rsx! { "unclaimed" },
                }
            }
        }
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
