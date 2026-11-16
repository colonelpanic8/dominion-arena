mod common;

use common::card_db;
use dominion_engine::{Card, Kind};
use std::collections::BTreeSet;

#[test]
fn split_piles_can_be_inspected_from_either_half() {
    let cards = card_db();
    for name in ["Encampment", "Plunder", "encampment"] {
        let members: Vec<&str> = cards
            .pile_members(name)
            .iter()
            .map(|c| c.name.as_str())
            .collect();
        assert_eq!(members, ["Encampment", "Plunder"]);
    }
    assert!(cards.pile_members("not a card").is_empty());
}

#[test]
fn every_pile_member_finds_the_whole_pile_without_its_randomizer() {
    let cards = card_db();
    for &card in Card::ALL {
        let pile = card.def().pile.unwrap_or(card);
        let expected: BTreeSet<&str> = pile
            .pile_members()
            .into_iter()
            .filter(|c| c.kind() == Kind::Card)
            .map(Card::name)
            .collect();
        let actual: BTreeSet<&str> = cards
            .pile_members(card.name())
            .iter()
            .map(|c| c.name.as_str())
            .collect();
        assert_eq!(actual, expected, "{}", card.name());
    }
}
