//! Setup across sets: what one set's cards need, whichever set put them in the game, and
//! configured setup choices.

use super::*;
use crate::setup::{SetupError, build};

fn config(kingdom: &[Card], landscapes: &[Card], choices: &[(Card, &[Card])]) -> GameConfig {
    GameConfig {
        colonies: Some(false),
        shelters: Some(false),
        landscapes: landscapes.to_vec(),
        setup_choices: choices.iter().map(|&(by, c)| (by, c.to_vec())).collect(),
        ..GameConfig::new(2, kingdom.to_vec(), 3)
    }
}

fn game(kingdom: &[Card], landscapes: &[Card], choices: &[(Card, &[Card])]) -> Game {
    setup_with(config(kingdom, landscapes, choices))
}

fn black_market(g: &Game) -> Vec<Card> {
    let deck = g.ext.promos.black_market.expect("a Black Market deck");
    let mut cards: Vec<Card> = g.pile(deck).cards.iter().map(|&c| g.printed(c)).collect();
    cards.reverse();
    cards
}

#[test]
fn a_looter_in_the_black_market_deck_brings_ruins() {
    let g = game(
        &[Card::BlackMarket],
        &[],
        &[(Card::BlackMarket, &[Card::DeathCart])],
    );
    assert_eq!(black_market(&g), [Card::DeathCart]);
    let ruins = g.supply_pile(Card::Ruins).expect("a Ruins pile");
    assert_eq!(g.pile(ruins).cards.len(), 10);
}

#[test]
fn a_looter_set_aside_by_riverboat_or_ferryman_brings_ruins() {
    let g = game(
        &[Card::Riverboat],
        &[],
        &[(Card::Riverboat, &[Card::Cultist])],
    );
    assert_eq!(
        g.ext.rising_sun.riverboat.map(|c| g.printed(c)),
        Some(Card::Cultist)
    );
    assert!(g.supply_pile(Card::Ruins).is_some());

    let g = game(
        &[Card::Ferryman, Card::Hermit],
        &[],
        &[(Card::Ferryman, &[Card::DeathCart])],
    );
    let ferryman = g.ext.cornucopia_guilds.ferryman.expect("Ferryman's pile");
    assert_eq!(g.pile(ferryman).card, Card::DeathCart);
    assert!(!g.pile(ferryman).in_supply);
    assert!(g.supply_pile(Card::Ruins).is_some());
}

#[test]
fn horse_gainers_in_the_black_market_deck_bring_horses() {
    let g = game(
        &[Card::BlackMarket],
        &[],
        &[(Card::BlackMarket, &[Card::Groom, Card::Village])],
    );
    let horses = g.pile_of(Card::Horse).expect("a Horse pile");
    assert!(!g.pile(horses).in_supply);
    assert_eq!(g.pile(horses).cards.len(), 30);

    let dealt = (0..100).any(|seed| {
        let g = setup_with(GameConfig::new(2, vec![Card::BlackMarket], seed));
        let horse_user = [Card::Groom, Card::Livery, Card::Scrap, Card::Supplies]
            .iter()
            .any(|c| black_market(&g).contains(c));
        if horse_user {
            assert!(g.pile_of(Card::Horse).is_some(), "seed {seed}");
        }
        horse_user
    });
    assert!(dealt);
}

#[test]
fn tax_and_defiled_shrine_cover_piles_added_late_in_setup() {
    let g = game(
        &[Card::BlackMarket, Card::YoungWitch],
        &[Card::Tax, Card::DefiledShrine],
        &[
            (Card::BlackMarket, &[Card::DeathCart]),
            (Card::YoungWitch, &[Card::Hamlet]),
        ],
    );
    for card in [Card::Ruins, Card::Hamlet, Card::Copper, Card::YoungWitch] {
        let pile = g.supply_pile(card).unwrap();
        assert_eq!(g.pile(pile).debt_tokens, 1, "{card}");
    }
    for card in [Card::Ruins, Card::Hamlet] {
        assert_eq!(g.pile(g.supply_pile(card).unwrap()).vp_tokens, 2, "{card}");
    }
}

#[test]
fn tax_covers_approaching_armys_pile() {
    let g = game(
        &[Card::Village],
        &[Card::ApproachingArmy, Card::Tax],
        &[(Card::ApproachingArmy, &[Card::Militia])],
    );
    let militia = g
        .supply_pile(Card::Militia)
        .expect("Approaching Army's pile");
    assert_eq!(g.pile(militia).debt_tokens, 1);
}

#[test]
fn configured_choices_are_used() {
    let g = game(
        &[
            Card::YoungWitch,
            Card::Ferryman,
            Card::Riverboat,
            Card::Druid,
            Card::BlackMarket,
            Card::Smithy,
        ],
        &[
            Card::WayOfTheMouse,
            Card::Obelisk,
            Card::Cheap,
            Card::ApproachingArmy,
        ],
        &[
            (Card::YoungWitch, &[Card::Moat]),
            (Card::Ferryman, &[Card::Militia]),
            (Card::WayOfTheMouse, &[Card::Village]),
            (Card::Riverboat, &[Card::Laboratory]),
            (
                Card::Druid,
                &[Card::TheSeasGift, Card::TheSunsGift, Card::TheSkysGift],
            ),
            (Card::BlackMarket, &[Card::Witch, Card::Market]),
            (Card::ApproachingArmy, &[Card::Bandit]),
            (Card::Obelisk, &[Card::Moat]),
            (Card::Cheap, &[Card::Bandit]),
        ],
    );
    let st = &g.ext.cornucopia_guilds;
    assert_eq!(g.pile(st.bane.unwrap()).card, Card::Moat);
    assert_eq!(g.pile(st.ferryman.unwrap()).card, Card::Militia);
    assert_eq!(
        g.ext.menagerie.mouse.map(|c| g.printed(c)),
        Some(Card::Village)
    );
    assert_eq!(
        g.ext.rising_sun.riverboat.map(|c| g.printed(c)),
        Some(Card::Laboratory)
    );
    assert_eq!(
        g.ext.nocturne.druid,
        [Card::TheSeasGift, Card::TheSunsGift, Card::TheSkysGift]
    );
    assert!(!g.boons().draw.contains(&Card::TheSunsGift));
    assert_eq!(black_market(&g), [Card::Witch, Card::Market]);
    assert!(g.supply_pile(Card::Bandit).is_some());
    assert_eq!(g.obelisk_pile(), g.supply_pile(Card::Moat));
    let bandit = g.supply_pile(Card::Bandit).unwrap();
    assert_eq!(g.pile(bandit).traits, [Card::Cheap]);
}

#[test]
fn invalid_choices_are_rejected() {
    let err = |kingdom: &[Card], landscapes: &[Card], choices: &[(Card, &[Card])]| {
        build(config(kingdom, landscapes, choices)).err()
    };
    // Bane must cost $2 or $3.
    assert_eq!(
        err(
            &[Card::YoungWitch],
            &[],
            &[(Card::YoungWitch, &[Card::Smithy])]
        ),
        Some(SetupError::InvalidChoice {
            by: Card::YoungWitch,
            choice: Card::Smithy
        })
    );
    // Already in the Kingdom.
    assert_eq!(
        err(
            &[Card::YoungWitch, Card::Moat],
            &[],
            &[(Card::YoungWitch, &[Card::Moat])]
        ),
        Some(SetupError::InvalidChoice {
            by: Card::YoungWitch,
            choice: Card::Moat
        })
    );
    assert_eq!(
        err(&[Card::Druid], &[], &[(Card::Druid, &[Card::TheSeasGift])]),
        Some(SetupError::ChoiceCount(Card::Druid))
    );
    assert_eq!(
        err(
            &[Card::BlackMarket],
            &[],
            &[(Card::BlackMarket, &[Card::Witch, Card::Witch])]
        ),
        Some(SetupError::InvalidChoice {
            by: Card::BlackMarket,
            choice: Card::Witch
        })
    );
    assert_eq!(
        err(
            &[Card::Smithy],
            &[],
            &[(Card::Riverboat, &[Card::Laboratory])]
        ),
        Some(SetupError::UnusedChoice(Card::Riverboat))
    );
    assert_eq!(
        err(
            &[Card::Smithy],
            &[Card::Obelisk],
            &[(Card::Obelisk, &[Card::Moat])]
        ),
        Some(SetupError::InvalidChoice {
            by: Card::Obelisk,
            choice: Card::Moat
        })
    );
}

#[test]
fn a_card_configured_for_one_choice_is_not_dealt_to_another() {
    // Young Witch's random Bane is chosen before Way of the Mouse's configured card.
    for seed in 0..40 {
        let mut config = config(
            &[Card::YoungWitch],
            &[Card::WayOfTheMouse],
            &[(Card::WayOfTheMouse, &[Card::Hamlet])],
        );
        config.seed = seed;
        let g = setup_with(config);
        let bane = g.ext.cornucopia_guilds.bane.unwrap();
        assert_ne!(g.pile(bane).card, Card::Hamlet, "seed {seed}");
        assert_eq!(
            g.ext.menagerie.mouse.map(|c| g.printed(c)),
            Some(Card::Hamlet)
        );
    }
}

#[test]
fn setup_choices_round_trip_through_json() {
    let config = config(
        &[Card::YoungWitch],
        &[],
        &[(Card::YoungWitch, &[Card::Moat])],
    );
    let json = serde_json::to_string(&config).unwrap();
    assert_eq!(serde_json::from_str::<GameConfig>(&json).unwrap(), config);
    let plain = GameConfig::new(2, vec![Card::Smithy], 1);
    let json = serde_json::to_string(&plain).unwrap();
    assert!(!json.contains("setup_choices"));
    assert_eq!(serde_json::from_str::<GameConfig>(&json).unwrap(), plain);
}
