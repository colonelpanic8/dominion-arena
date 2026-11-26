//! The Supply as dominion.games lays it out: the basic piles in two columns at the left
//! edge (Victory cards and Curses, Treasures), the kingdom in a grid filled from the
//! bottom row up by cost, landscapes in a column beside it, non-Supply piles as strips,
//! and the trash as a tab on the top edge.

use super::layout::{Geometry, Rect};
use super::pieces::{BadgeView, TableCard};
use super::{Popup, Spot, Ui, mark_at, use_table};
use crate::app::use_cards;
use crate::card::{CardSize, aspect_ratio};
use crate::game::board::{Board, CardDeck, LandscapeEntry, PileEntry, SetupSetAside};
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
    let key = |p: &PileEntry| (std::cmp::Reverse(p.printed), p.name.clone());
    let curse_last = |p: &PileEntry| p.types.iter().any(|t| t == "Curse");
    treasures.sort_by_key(key);
    rest.sort_by_key(|p| (curse_last(p), key(p)));
    (rest, treasures)
}

/// What sits beside the kingdom, in order: landscapes by cost and name, then cards set
/// aside during setup that have no pile or landscape of their source's to sit on.
#[derive(Clone, Debug, PartialEq)]
pub enum Wide {
    Landscape(LandscapeEntry),
    SetAside(SetupSetAside),
}

pub fn wides(b: &Board) -> Vec<Wide> {
    let mut ls = b.landscapes.clone();
    ls.sort_by(|a, b| {
        crate::game::board::cost_key(a.cost.as_ref())
            .cmp(&crate::game::board::cost_key(b.cost.as_ref()))
            .then_with(|| a.card.cmp(&b.card))
    });
    let mut out: Vec<Wide> = ls.into_iter().map(Wide::Landscape).collect();
    out.extend(
        b.set_asides
            .iter()
            .filter(|s| host(b, &s.source).is_none())
            .cloned()
            .map(Wide::SetAside),
    );
    out
}

/// A non-Supply strip: the Hexes' and Boons' decks first, as dominion.games sorts its
/// "Hex Drawpile" and "Boon Drawpile" (landscapes, after the cards, then reversed), then
/// the piles, most expensive first.
#[derive(Clone, Debug, PartialEq)]
pub enum Strip {
    Deck(&'static str, CardDeck),
    Pile(PileEntry),
}

pub fn strips(b: &Board) -> Vec<Strip> {
    let mut piles = b.non_supply.clone();
    piles.sort_by(|a, b| b.printed.cmp(&a.printed).then_with(|| b.name.cmp(&a.name)));
    let decks = [("Hexes", &b.hexes), ("Boons", &b.boons)];
    decks
        .into_iter()
        .filter_map(|(label, d)| Some(Strip::Deck(label, d.clone()?)))
        .chain(piles.into_iter().map(Strip::Pile))
        .collect()
}

/// What a card set aside for `source` is drawn on: its pile or its landscape.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Host {
    Pile,
    Landscape,
}

fn host(b: &Board, source: &str) -> Option<Host> {
    if b.supply.kingdom.iter().flatten().any(|p| p.name == source) {
        Some(Host::Pile)
    } else if b.landscapes.iter().any(|l| l.card == source) {
        Some(Host::Landscape)
    } else {
        None
    }
}

/// The kingdom piles, cheapest first by the cost printed on each pile's own card.
fn kingdom(b: &Board) -> Vec<PileEntry> {
    let mut piles: Vec<PileEntry> = b.supply.kingdom.iter().flatten().cloned().collect();
    piles.sort_by(|a, b| a.printed.cmp(&b.printed).then_with(|| a.name.cmp(&b.name)));
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
    let strips = strips(&board);
    let basic: Vec<(PileEntry, Rect)> = victory
        .into_iter()
        .zip(g.victory.iter().copied())
        .chain(treasures.into_iter().zip(g.treasures.iter().copied()))
        .collect();
    let cards = use_cards();
    let insets: Vec<(SetupSetAside, Rect)> = board
        .set_asides
        .iter()
        .filter_map(|s| {
            let on = match host(&board, &s.source)? {
                Host::Pile => kingdom
                    .iter()
                    .zip(g.kingdom.iter())
                    .find(|(p, _)| p.name == s.source)
                    .map(|(_, r)| (*r, false))?,
                Host::Landscape => wide
                    .iter()
                    .zip(g.landscapes.iter())
                    .find(|(w, _)| matches!(w, Wide::Landscape(l) if l.card == s.source))
                    .map(|(_, r)| (*r, true))?,
            };
            let mut same = board.set_asides.iter().filter(|o| o.source == s.source);
            let n = same.clone().count();
            let i = same.position(|o| o == s).unwrap_or(0);
            let wide_card = cards.get(&s.card).is_some_and(crate::card::is_landscape);
            Some((s.clone(), super::layout::inset(on.0, on.1, wide_card, i, n)))
        })
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
                    Wide::SetAside(entry) => rsx! { SetAsideCard { key: "s{entry.source}:{entry.card}", entry, at: *r, ui: ui.clone(), sel: sel.clone() } },
                }
            }
            for (s, r) in strips.iter().zip(g.non_supply.iter()) {
                match s.clone() {
                    Strip::Deck(label, d) => rsx! { Deck { key: "{label}", label, deck: d, at: *r } },
                    Strip::Pile(p) => rsx! { Pile { key: "n{p.name}", pile: p, at: *r, ui: ui.clone(), sel: sel.clone(), size: CardSize::Micro } },
                }
            }
            for (entry, r) in insets.iter() {
                SetAsideCard { key: "i{entry.source}:{entry.card}", entry: entry.clone(), at: *r, ui: ui.clone(), sel: sel.clone(), inset: true }
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
    /// The count before the title, as on dominion.games' trash; a mat's follows it.
    #[props(default)]
    count_first: bool,
) -> Element {
    // One line, shrunk to fit beside the count as dominion.games fits it.
    let vh = at.h / 4.0;
    let room = (at.w - 4.4 * vh).max(vh);
    let chars = label.chars().count().max(1) as f64;
    let label_px = (2.4 * vh).min(room / (0.62 * chars));
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
            span { class: "dt-tab-label", style: "font-size:{label_px:.1}px;", "{label}" }
            span { class: "dt-tab-n", class: if count_first { "dt-tab-n-first" }, "{count}" }
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
        composition: None,
        played: false,
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
            count_first: true,
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

/// What a pile's mini card prints under its art: its markers, then the types
/// dominion.games shows on piles of the card the pile is named after.
fn pile_line(pile: &PileEntry, cards: &crate::cards::CardDb) -> Vec<String> {
    let shown = cards.get(&pile.name).into_iter().flat_map(|c| &c.types);
    pile.markers
        .iter()
        .map(|m| m.name().to_string())
        .chain(
            shown
                .filter(|t| dominion_protocol::TYPES_SHOWN_ON_PILE.contains(&t.as_str()))
                .cloned(),
        )
        .collect()
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
        printed.is_some_and(|p| (p.coins, p.potions, p.debt) != (c.coins, c.potions, c.debt))
    });
    let mark = mark_at(&ui, &sel, Spot::Pile(pile.at));
    let changed = cost.is_some();
    let inspect_pile = !pile.hidden && cards.pile_members(&pile.name).len() > 1;
    let split = pile.shadowed && size == CardSize::Mini;
    // Shrunk to leave the back room, as dominion.games does.
    let at = if split {
        Rect::new(at.x, at.y, at.w * 0.95, at.h * 0.95)
    } else {
        at
    };
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
            // A shuffled pile (Loot) on its strip shows its name, as dominion.games does.
            back: pile.hidden && size != CardSize::Micro,
            badges: pile.badges.clone(),
            at,
            plus,
            split,
            pile: pile_line(&pile, &cards),
            attached: pile.attached.clone(),
            inspect_pile,
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
        printed.is_some_and(|p| (p.coins, p.potions, p.debt) != (c.coins, c.potions, c.debt))
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
            holds: entry.pile.as_ref().map(|p| (p.name.clone(), p.cost.clone())),
            class: "dt-landscape dt-kind-{entry.kind}",
            at,
        }
    }
}

/// A public card set aside during setup, marked with the card that set it aside.
#[component]
fn SetAsideCard(
    entry: SetupSetAside,
    at: Rect,
    ui: Option<Rc<Ui>>,
    sel: Selection,
    /// Drawn on its source's pile or landscape, which names it already.
    #[props(default)]
    inset: bool,
) -> Element {
    let SetupSetAside { source, card } = entry;
    let cards = use_cards();
    let aspect = f64::from(aspect_ratio(cards.get(&card), CardSize::Mini));
    let w = at.w.min(at.h / aspect);
    let h = w * aspect;
    let at = Rect::new(at.x + (at.w - w) / 2.0, at.y + (at.h - h) / 2.0, w, h);
    let mark = mark_at(&ui, &sel, Spot::Landscape(card.clone()));
    let plus = mark.target.is_some();
    let badge = crate::game::board::Badge {
        kind: "note".into(),
        text: format!("{card} set aside for {source}"),
        count: None,
        player: None,
        card: Some(source),
    };
    rsx! {
        TableCard {
            name: card,
            size: CardSize::Mini,
            mark,
            plus,
            badges: if inset { vec![] } else { vec![badge] },
            class: if inset { "dt-setup-set-aside dt-inset" } else { "dt-setup-set-aside" },
            at,
            z: i32::from(inset),
        }
    }
}

/// The Boons or Hexes deck, as dominion.games' "Boon Drawpile" or "Hex Drawpile": a
/// non-Supply strip counting the cards left to draw, opening its discard pile.
#[component]
fn Deck(label: &'static str, deck: CardDeck, at: Rect) -> Element {
    let cards = use_cards();
    let (kind, title) = match label {
        "Hexes" => ("Hex", "Hex Drawpile"),
        _ => ("Boon", "Boon Drawpile"),
    };
    // Any card of the deck's type draws its strip's frame.
    let face = cards
        .all()
        .iter()
        .find(|c| c.types.len() == 1 && c.types[0] == kind)
        .map_or_else(|| title.to_string(), |c| c.name.clone());
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
        composition: None,
        played: false,
    };
    rsx! {
        TableCard {
            name: face,
            title: title.to_string(),
            size: CardSize::Micro,
            count: Some(deck.count),
            opens: popup,
            class: "dt-pile dt-nspile dt-deck-{kind.to_lowercase()}",
            at,
        }
    }
}

/// Badges in a row, for players.
#[component]
pub fn Badges(badges: Vec<crate::game::board::Badge>) -> Element {
    if badges.is_empty() {
        return rsx! {};
    }
    rsx! {
        span { class: "dt-badges",
            for (i, b) in badges.iter().enumerate() {
                BadgeView { key: "{i}", badge: b.clone() }
            }
        }
    }
}
