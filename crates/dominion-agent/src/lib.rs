//! The `dominion/agent-view@2` protocol for games hosted by `dominion-engine`.
//!
//! A [`Table`] hosts one game and tracks what each seat knows. [`Table::view`] gives a
//! seat's [`AgentView`]: everything that seat may know (with perfect recall), plus its
//! open decision as a flat list of indexed options. [`Table::answer`] takes option
//! indices (or a card name) and plays them; [`Table::answer_with`] can also answer the
//! Way questions that follow in advance. [`text::render`] renders a view compactly for
//! language models. The JSON Schema is vendored at `protocol/agent-view.schema.json`.

mod project;
mod table;
pub mod text;

pub use dominion_protocol::view::*;
pub use project::WAY_HINT;
pub use table::{AnswerError, Answered, Table, WayChoice};

use dominion_engine::Card;

/// The vendored JSON Schema of agent-view@2.
pub const JSON_SCHEMA: &str = include_str!("../protocol/agent-view.schema.json");

/// The glossary entry for a card, landscape or other card-shaped object.
pub fn glossary_entry(card: Card) -> GlossaryEntry {
    GlossaryEntry {
        name: card.name().to_string(),
        types: card.type_names().into_iter().map(String::from).collect(),
        set: Some(card.set().to_string()),
        cost: card.cost().map(project::cost),
        text: card.text().to_string(),
    }
}

/// Looks a card up by name (ignoring case, spaces and punctuation).
pub fn glossary(name: &str) -> Option<GlossaryEntry> {
    Card::from_name(name).map(glossary_entry)
}

/// Glossary entries for every card named in a view: piles, landscapes, known cards,
/// options.
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
    for pile in view.supply.iter().chain(&view.non_supply) {
        names.insert(pile.card.clone());
        names.extend(pile.top.clone());
    }
    names.extend(view.landscapes.iter().map(|l| l.card.clone()));
    if let Some(d) = &view.decision {
        names.extend(d.options.iter().filter_map(|o| o.card.clone()));
    }
    names.iter().filter_map(|n| glossary(n)).collect()
}
