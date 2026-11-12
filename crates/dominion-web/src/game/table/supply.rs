//! The Supply: basic piles in columns, the Kingdom in rows, and a strip of non-Supply
//! piles, landscapes, Boons and Hexes, Artifacts and the trash.

use super::layout::{Middle, SupplyShape, WideItem};
use super::pieces::{BadgeView, TableCard};
use super::{Popup, Spot, Ui, mark_at};
use crate::app::use_cards;
use crate::card::CardSize;
use crate::game::board::{Board, CardDeck, Holding, LandscapeEntry, PileEntry};
use crate::game::state::Selection;
use dioxus::prelude::*;
use std::rc::Rc;

fn is_treasure(p: &PileEntry) -> bool {
    p.types.iter().any(|t| t == "Treasure")
}

/// The basic piles in columns: Treasures, then the rest, at most `rows` to a column.
fn basic_columns(basic: &[PileEntry], rows: usize) -> Vec<Vec<PileEntry>> {
    let (treasures, rest): (Vec<_>, Vec<_>) = basic.iter().cloned().partition(is_treasure);
    let mut cols = vec![];
    for group in [treasures, rest] {
        for chunk in group.chunks(rows.max(1)) {
            cols.push(chunk.to_vec());
        }
    }
    cols
}

/// What sits in the columns beside the Kingdom, in order.
#[derive(Clone, Debug, PartialEq)]
enum Wide {
    Landscape(LandscapeEntry),
    Artifact(Holding),
    Deck(&'static str, CardDeck),
    Trash,
}

fn wides(b: &Board) -> Vec<Wide> {
    let mut out: Vec<Wide> = b.landscapes.iter().cloned().map(Wide::Landscape).collect();
    out.extend(b.artifacts.iter().cloned().map(Wide::Artifact));
    if let Some(d) = b.boons.clone() {
        out.push(Wide::Deck("Boons", d));
    }
    if let Some(d) = b.hexes.clone() {
        out.push(Wide::Deck("Hexes", d));
    }
    out.push(Wide::Trash);
    out
}

/// The Supply band's contents, as far as its layout goes.
pub fn shape(b: &Board) -> SupplyShape {
    let treasures = b.supply.basic.iter().filter(|p| is_treasure(p)).count();
    SupplyShape {
        treasures,
        others: b.supply.basic.len() - treasures,
        kingdom_rows: b.supply.kingdom.len().max(1),
        kingdom_cols: b
            .supply
            .kingdom
            .iter()
            .map(Vec::len)
            .max()
            .unwrap_or(1)
            .max(5),
        wide: wides(b)
            .iter()
            .map(|w| match w {
                Wide::Deck(..) => WideItem::Deck,
                Wide::Trash => WideItem::Trash,
                _ => WideItem::Landscape,
            })
            .collect(),
        non_supply: b.non_supply.len(),
    }
}

#[component]
pub fn SupplyArea(
    board: Rc<Board>,
    lay: Rc<Middle>,
    ui: Option<Rc<Ui>>,
    sel: Selection,
) -> Element {
    let cols = basic_columns(&board.supply.basic, lay.basic_rows);
    let wide = wides(&board);
    let ns_cols: Vec<Vec<PileEntry>> = board
        .non_supply
        .chunks(lay.ns_rows.max(1))
        .map(<[PileEntry]>::to_vec)
        .collect();
    let ns_size = if lay.ns_micro {
        CardSize::Micro
    } else {
        CardSize::Mini
    };
    rsx! {
        div { class: "dt-supply",
            div { class: "dt-basic",
                for (i, col) in cols.iter().enumerate() {
                    div { key: "{i}", class: "dt-bcol",
                        for p in col.iter() {
                            Pile { key: "{p.name}", pile: p.clone(), ui: ui.clone(), sel: sel.clone() }
                        }
                    }
                }
            }
            div { class: "dt-kingdom",
                for (i, row) in board.supply.kingdom.iter().enumerate() {
                    div { key: "{i}", class: "dt-krow",
                        for p in row.iter() {
                            Pile { key: "{p.name}", pile: p.clone(), ui: ui.clone(), sel: sel.clone() }
                        }
                    }
                }
            }
            for (i, col) in lay.wide_cols.iter().enumerate() {
                div { key: "w{i}", class: "dt-wcol",
                    for &j in col.iter() {
                        match wide[j].clone() {
                            Wide::Landscape(l) => rsx! { Landscape { key: "l{l.card}", entry: l, ui: ui.clone(), sel: sel.clone() } },
                            Wide::Artifact(a) => rsx! { Artifact { key: "a{a.card}", holding: a } },
                            Wide::Deck(label, d) => rsx! { Deck { key: "{label}", label, deck: d } },
                            Wide::Trash => rsx! { Trash { key: "{j}", board: board.clone() } },
                        }
                    }
                }
            }
            for (i, col) in ns_cols.iter().enumerate() {
                div { key: "n{i}", class: "dt-nscol", class: if lay.ns_micro { "dt-nscol-micro" },
                    for p in col.iter() {
                        Pile { key: "ns{p.name}", pile: p.clone(), ui: ui.clone(), sel: sel.clone(), size: ns_size }
                    }
                }
            }
        }
    }
}

#[component]
fn Trash(board: Rc<Board>) -> Element {
    let ctx = super::use_table();
    let b = &board;
    let trash = Popup {
        title: format!("Trash ({})", b.trash.count),
        stacks: b.trash.known.clone(),
        unknown: b.trash.unknown,
        note: None,
    };
    let top = b.trash.known.last().map(|s| s.card.clone());
    rsx! {
        div {
            class: "dt-trash dt-click",
            class: if b.trash.count == 0 { "dt-trash-empty" },
            "data-pile": "trash",
            "data-anchor": "trash",
            title: "Open the trash",
            onclick: move |e| {
                e.stop_propagation();
                ctx.open(trash.clone());
            },
            span { class: "dt-trash-label", "Trash" }
            span { class: "dt-trash-n", "{b.trash.count}" }
            if let Some(t) = top {
                span { class: "dt-trash-top", "data-name": "{t}" }
            }
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
    rsx! {
        TableCard {
            name: pile.card.clone(),
            size,
            cost,
            count: Some(pile.count),
            anchor: pile_anchor(pile.at),
            mark,
            dim: pile.empty,
            back: pile.hidden,
            badges: pile.badges.clone(),
            class: format!(
                "dt-pile{}{}{}",
                if pile.kingdom { " dt-kpile" } else { "" },
                if !pile.kingdom && matches!(pile.at, crate::game::board::PileAt::NonSupply(_)) { " dt-nspile" } else { "" },
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

/// The Boons or Hexes deck, face down with how many are left, beside its discard pile.
#[component]
fn Deck(label: String, deck: CardDeck) -> Element {
    let ctx = super::use_table();
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
    rsx! {
        div { class: "dt-deckbox", "data-deck": "{label}",
            div {
                class: "dt-deckback dt-deck-{lower} dt-click",
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
                    }
                },
                None => rsx! {
                    div { class: "dt-deck-discard dt-deck-none", title: "No {label} discarded" }
                },
            }
        }
    }
}

#[component]
fn Artifact(holding: Holding) -> Element {
    let cubes: Vec<(u8, String)> = holding
        .holder
        .iter()
        .map(|w| (w.seat, w.name.clone()))
        .collect();
    let title = match &holding.holder {
        Some(w) => format!("{} holds it", w.name),
        None => "Unclaimed".into(),
    };
    rsx! {
        div { class: "dt-artifact", title: "{title}",
            TableCard { name: holding.card.clone(), size: CardSize::Mini, class: "dt-landscape", cubes }
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
