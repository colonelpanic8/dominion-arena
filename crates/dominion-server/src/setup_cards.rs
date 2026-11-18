//! What a finished game was set up with besides its Kingdom piles and landscapes, read
//! from a view of it: Supply piles beyond the base cards (Platinum, Colony, Potion, Ruins)
//! and the starting cards that stood in for Estates or Coppers (Shelters, Heirlooms).

use dominion_engine::{Card, Types};
use dominion_protocol::view::AgentView;
use std::collections::BTreeSet;

const BASE: [&str; 7] = [
    "Copper", "Silver", "Gold", "Estate", "Duchy", "Province", "Curse",
];

/// Those cards' names: the piles in Supply order, then the starting cards by name.
pub fn setup_cards(view: &AgentView) -> Vec<String> {
    let piles = view
        .supply
        .iter()
        .filter(|p| !p.kingdom && !BASE.contains(&p.card.as_str()));
    let mut out: Vec<String> = piles.map(|p| p.card.clone()).collect();
    let players = view.you.iter().chain(&view.opponents);
    let known = (players.flat_map(|p| p.owned.known.keys())).chain(view.trash.known.keys());
    let starting = Types::SHELTER | Types::HEIRLOOM;
    let started: BTreeSet<&String> = known
        .filter(|name| Card::from_name(name).is_some_and(|c| c.def().types.intersects(starting)))
        .collect();
    out.extend(started.into_iter().cloned());
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use dominion_agent::Table;
    use dominion_engine::{GameConfig, PlayerId};

    fn cards(kingdom: &[Card], colonies: bool, shelters: bool) -> Vec<String> {
        let mut config = GameConfig::new(2, kingdom.to_vec(), 7);
        config.colonies = Some(colonies);
        config.shelters = Some(shelters);
        let table = Table::new("g1", config, vec!["A".into(), "B".into()]).unwrap();
        let mut cards = setup_cards(&table.view(PlayerId(0)));
        cards.sort();
        cards
    }

    #[test]
    fn colonies_shelters_and_heirlooms_are_named() {
        let kingdom = [
            Card::Village,
            Card::Smithy,
            Card::Market,
            Card::Moat,
            Card::Cellar,
            Card::Militia,
            Card::Mine,
            Card::Witch,
            Card::Library,
            Card::Pixie,
        ];
        assert_eq!(
            cards(&kingdom, true, true),
            [
                "Colony",
                "Goat",
                "Hovel",
                "Necropolis",
                "Overgrown Estate",
                "Platinum"
            ]
        );
        assert_eq!(cards(&kingdom, false, false), ["Goat"]);
    }
}
