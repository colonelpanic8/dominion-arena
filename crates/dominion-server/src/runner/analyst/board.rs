use dominion_engine::{Card, Types};
use dominion_protocol::view::{AgentView, CardCount};
use serde_json::{Value, json};
use std::collections::BTreeSet;

pub(super) fn setup(view: &AgentView) -> Value {
    let mut names: BTreeSet<&str> = view
        .game
        .kingdom
        .iter()
        .chain(&view.game.landscapes)
        .map(String::as_str)
        .collect();
    let mut supply = view.supply.clone();
    for pile in supply.iter_mut().filter(|p| p.hidden) {
        pile.top = None;
    }
    let mut extra = view.non_supply.clone();
    // Face-down decks can have a seat-visible top. Their public recipe suffices.
    for pile in &mut extra {
        pile.top = None;
    }
    for pile in supply.iter().chain(&extra) {
        names.insert(&pile.card);
        names.extend(pile.top.as_deref());
        names.extend(pile.members.iter().chain(&pile.traits).map(String::as_str));
        if let Some(structure) = &pile.structure {
            names.extend(structure.cards.iter().map(|group| group.card.as_str()));
        }
    }
    let notes: Vec<_> = view.notes.iter().filter(|n| n.setup.is_some()).collect();
    for n in &notes {
        names.insert(&n.source);
        names.extend(n.card.as_deref());
    }
    let starting = crate::setup_cards::setup_cards(view);
    let starting: BTreeSet<_> = starting
        .iter()
        .filter_map(|s| Card::from_name(s))
        .filter(|c| c.def().types.intersects(Types::HEIRLOOM | Types::SHELTER))
        .chain(
            view.game
                .kingdom
                .iter()
                .filter_map(|s| Card::from_name(s)?.related().heirloom),
        )
        .map(|c| c.name())
        .collect();
    names.extend(starting.iter().copied());
    let definitions: Vec<_> = names
        .into_iter()
        .filter_map(dominion_agent::glossary)
        .collect();
    json!({"kingdom":view.game.kingdom, "supply":supply,"nonSupply":extra,
        "landscapes":view.landscapes,"landscapeNames":view.game.landscapes,"setupNotes":notes,
        "specialStartingCards":starting,"definitions":definitions})
}

// Exact for the usual untouched ten-card opening. Other setups keep the normal seat view.
pub(crate) fn opening_facts(view: &AgentView) -> Option<String> {
    let p = view.you.as_ref()?;
    if view.turn.number != Some(1)
        || !view.turn.your_turn
        || p.hand.count != 5
        || p.deck.count != 5
        || p.owned.unknown != 0
        || p.owned.known.values().sum::<u32>() != 10
        || p.hand.unknown != 0
        || p.deck.unknown != 0
        || p.hand.known.values().sum::<u32>() != 5
        || p.deck.known.values().sum::<u32>() != 5
        || view.turn.coins.unwrap_or_default() != 0
    {
        return None;
    }
    let current = hand_facts(&p.hand.known)?;
    let next = hand_facts(&p.deck.known)?;
    let mut facts = format!(
        "Opening arithmetic assuming two undisturbed five-card draws, printed Treasure money only, one gain per turn, and no other effects. After two gains the reshuffle contains 12 cards minus cards actually trashed. For each coin price the table gives the maximum number of cards remaining in hand after paying. Current hand: {current}. Next five cards: {next}."
    );
    if let Some(cost) = view
        .supply
        .iter()
        .find(|p| p.card == "Cemetery")
        .and_then(|p| p.cost.as_ref())
        && cost.debt == 0
        && cost.potions == 0
        && let Some(left) = next["maxCardsRemainingByPrice"][cost.coins.to_string()].as_u64()
    {
        let trash = left.min(4);
        facts.push_str(&format!(" Buying Cemetery on turn two at this price leaves at most {trash} to trash; the resulting reshuffle contains at least {} cards under those assumptions.", 12-trash));
    }
    Some(facts)
}

fn hand_facts(cards: &CardCount) -> Option<Value> {
    let mut coins = Vec::new();
    for (name, &count) in cards {
        let c = Card::from_name(name)?;
        let money = match c {
            Card::Copper | Card::HauntedMirror | Card::JewelledEgg => 1,
            Card::Silver => 2,
            Card::Gold => 3,
            Card::Platinum => 5,
            _ if c.def().types.contains(Types::TREASURE) => return None,
            _ => 0,
        };
        coins.extend(std::iter::repeat_n(money, count as usize));
    }
    coins.sort_unstable_by(|a, b| b.cmp(a));
    let total: u32 = coins.iter().sum();
    let mut left = std::collections::BTreeMap::from([(0, coins.len())]);
    let mut paid = 0;
    for (i, value) in coins.iter().enumerate() {
        for price in paid + 1..=paid + value {
            left.insert(price, coins.len() - i - 1);
        }
        paid += value;
    }
    Some(json!({"cards":cards,"treasureCoins":total,"maxCardsRemainingByPrice":left}))
}

#[cfg(test)]
mod tests {
    use super::*;
    use dominion_agent::Table;
    use dominion_engine::{GameConfig, PlayerId};

    #[test]
    fn colony_setup_and_cemetery_payment_use_their_respective_views() {
        let mut config = GameConfig::new(
            2,
            vec![
                Card::Cemetery,
                Card::City,
                Card::GrandMarket,
                Card::Artisan,
                Card::Werewolf,
                Card::AnimalFair,
                Card::Abundance,
                Card::JewelledEgg,
                Card::Stables,
                Card::Baron,
            ],
            7,
        );
        config.colonies = Some(true);
        let mut view = Table::new("g", config, vec!["A".into(), "B".into()])
            .unwrap()
            .view(PlayerId(0));
        let public = setup(&view);
        for name in ["Colony", "Platinum", "Haunted Mirror", "Ghost"] {
            assert!(
                public["definitions"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|g| g["name"] == name)
            );
        }
        let cards =
            |items: &[(&str, u32)]| items.iter().map(|(c, n)| (c.to_string(), *n)).collect();
        view.turn.number = Some(1);
        view.turn.your_turn = true;
        let p = view.you.as_mut().unwrap();
        p.hand.known = cards(&[("Copper", 3), ("Estate", 2)]);
        p.hand.count = 5;
        p.hand.unknown = 0;
        p.deck.known = cards(&[("Copper", 3), ("Haunted Mirror", 1), ("Estate", 1)]);
        p.deck.count = 5;
        p.deck.unknown = 0;
        p.owned.known = cards(&[("Copper", 6), ("Haunted Mirror", 1), ("Estate", 3)]);
        p.owned.unknown = 0;
        assert_eq!(setup(&view), public);
        let facts = opening_facts(&view).unwrap();
        assert!(facts.contains("at most 1 to trash") && facts.contains("at least 11 cards"));
        view.you.as_mut().unwrap().deck.unknown = 1;
        assert!(opening_facts(&view).is_none());
    }
}
