//! The `dominion/agent-view@2` protocol for games hosted by `dominion-engine`.
//!
//! A [`Table`] hosts one game and tracks what each seat knows. [`Table::view`] gives a
//! seat's [`AgentView`]: everything that seat may know (with perfect recall), plus its
//! open decision as a flat list of indexed options. [`Table::answer`] takes option
//! indices (or a card name) and plays them; [`Table::answer_with`] can also answer the
//! Way questions that follow in advance. [`text::render`] renders a view compactly for
//! language models. The JSON Schema is vendored at `protocol/agent-view.schema.json`.

pub mod events;
mod project;
pub mod recording;
pub mod replay;
mod table;
pub mod text;
pub mod turns;

pub use dominion_protocol::view::*;
pub use project::WAY_HINT;
pub use table::{AnswerError, Answered, Table, WayChoice};

use dominion_engine::{Card, Kind};

/// The vendored JSON Schema of agent-view@2.
pub const JSON_SCHEMA: &str = include_str!("../protocol/agent-view.schema.json");

/// The glossary entry for a card, landscape or other card-shaped object.
pub fn glossary_entry(card: Card) -> GlossaryEntry {
    let def = card.def();
    let mut text = card.text().to_string();
    if text.is_empty() {
        let mut values = vec![];
        if def.coins > 0 {
            values.push(format!("${}", def.coins));
        }
        if def.potions > 0 {
            values.push(dominion_engine::amount(def.potions, "Potion", "Potions"));
        }
        if def.vp != 0 {
            values.push(format!("{} VP", def.vp));
        }
        text = values.join("\n");
    }
    GlossaryEntry {
        name: card.name().to_string(),
        types: card.type_names().into_iter().map(String::from).collect(),
        set: Some(card.set().to_string()),
        cost: card.cost().map(project::cost),
        text,
    }
}

/// Looks a card up by name (ignoring case, spaces and punctuation).
pub fn glossary(name: &str) -> Option<GlossaryEntry> {
    Card::from_name(name).map(glossary_entry)
}

/// Publicly present components, excluding cards included only as possible references.
pub(crate) fn component_sources(view: &AgentView) -> std::collections::BTreeSet<Card> {
    let mut names = Vec::new();
    for pile in view.supply.iter().chain(&view.non_supply) {
        names.push(pile.card.as_str());
        names.extend(pile.top.as_deref());
        names.extend(pile.traits.iter().map(String::as_str));
    }
    names.extend(view.landscapes.iter().map(|l| l.card.as_str()));
    names.extend(view.artifacts.iter().map(|a| a.card.as_str()));
    for note in &view.notes {
        names.push(note.source.as_str());
        names.extend(note.card.as_deref());
    }
    for player in view.you.iter().chain(&view.opponents) {
        for zone in [&player.hand, &player.in_play, &player.discard]
            .into_iter()
            .chain(player.other_zones.values())
        {
            names.extend(zone.known.keys().map(String::as_str));
        }
        names.extend(player.deck.known.keys().map(String::as_str));
        names.extend(player.owned.known.keys().map(String::as_str));
        names.extend(
            player
                .projects
                .iter()
                .chain(&player.states)
                .chain(&player.boons)
                .chain(&player.artifacts)
                .map(String::as_str),
        );
    }
    names.extend(view.trash.known.keys().map(String::as_str));
    let mut sources: std::collections::BTreeSet<Card> =
        names.into_iter().filter_map(Card::from_name).collect();
    for pile in view.supply.iter().chain(&view.non_supply) {
        if let Some(structure) = &pile.structure {
            if structure.sample_size.is_none() {
                sources.extend(
                    structure
                        .cards
                        .iter()
                        .filter_map(|group| Card::from_name(&group.card)),
                );
            }
        } else if let Some(card) = Card::from_name(&pile.card) {
            sources.extend(card.pile_members());
        }
    }
    let heirlooms: Vec<_> = sources
        .iter()
        .filter_map(|card| card.related().heirloom)
        .collect();
    sources.extend(heirlooms);
    sources
}

/// Official entries for visible cards, including every possible member of their piles.
pub fn glossary_for(view: &AgentView) -> Vec<GlossaryEntry> {
    let mut names = std::collections::BTreeSet::new();
    let mut zone = |z: &CardCount| names.extend(z.keys().cloned());
    for p in view.you.iter().chain(&view.opponents) {
        for z in [&p.hand, &p.in_play, &p.discard]
            .into_iter()
            .chain(p.other_zones.values())
        {
            zone(&z.known);
        }
        zone(&p.deck.known);
        zone(&p.owned.known);
    }
    zone(&view.trash.known);
    for p in view.you.iter().chain(&view.opponents) {
        names.extend(
            p.projects
                .iter()
                .chain(&p.states)
                .chain(&p.boons)
                .chain(&p.artifacts)
                .cloned(),
        );
    }
    for pile in view.supply.iter().chain(&view.non_supply) {
        names.insert(pile.card.clone());
        names.extend(pile.top.clone());
        names.extend(pile.traits.iter().cloned());
        if let Some(structure) = &pile.structure {
            names.extend(structure.cards.iter().map(|group| group.card.clone()));
        } else if let Some(card) = Card::from_name(&pile.card) {
            names.extend(card.pile_members().iter().map(|c| c.name().to_string()));
        }
    }
    names.extend(
        view.game
            .kingdom
            .iter()
            .chain(&view.game.landscapes)
            .cloned(),
    );
    names.extend(view.landscapes.iter().map(|l| l.card.clone()));
    names.extend(view.artifacts.iter().map(|a| a.card.clone()));
    for lasting in &view.lasting {
        names.insert(lasting.card.clone());
        names.extend(lasting.cards.iter().cloned());
    }
    for note in &view.notes {
        names.insert(note.source.clone());
        names.extend(note.card.clone());
    }
    let receives_boons = component_sources(view)
        .iter()
        .any(|card| card.receives_boons());
    for &card in Card::ALL {
        if (view.boons.is_some() && receives_boons && card.kind() == Kind::Boon)
            || (view.hexes.is_some() && (card.kind() == Kind::Hex || card.is_hex_state()))
        {
            names.insert(card.name().to_string());
        }
    }
    if let Some(d) = &view.decision {
        names.extend(d.source.clone());
        names.extend(d.options.iter().filter_map(|o| o.card.clone()));
    }
    names.iter().filter_map(|n| glossary(n)).collect()
}
