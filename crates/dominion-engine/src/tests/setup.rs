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
fn any_legal_card_can_be_configured() {
    // Removed cards, and a Bane and a Ferryman pile from sets not in the Kingdom.
    let kingdom = [
        Card::YoungWitch,
        Card::Ferryman,
        Card::Riverboat,
        Card::BlackMarket,
    ];
    let landscapes = [Card::WayOfTheMouse, Card::ApproachingArmy];
    let g = game(
        &kingdom,
        &landscapes,
        &[
            (Card::YoungWitch, &[Card::Moat]),
            (Card::Ferryman, &[Card::Thief]),
            (Card::WayOfTheMouse, &[Card::Woodcutter]),
            (Card::Riverboat, &[Card::Mandarin]),
            (Card::BlackMarket, &[Card::Adventurer, Card::FortuneTeller]),
            (Card::ApproachingArmy, &[Card::NobleBrigand]),
        ],
    );
    let st = &g.ext.cornucopia_guilds;
    assert_eq!(g.pile(st.bane.unwrap()).card, Card::Moat);
    assert_eq!(g.pile(st.ferryman.unwrap()).card, Card::Thief);
    assert_eq!(
        g.ext.menagerie.mouse.map(|c| g.printed(c)),
        Some(Card::Woodcutter)
    );
    assert_eq!(
        g.ext.rising_sun.riverboat.map(|c| g.printed(c)),
        Some(Card::Mandarin)
    );
    assert_eq!(black_market(&g), [Card::Adventurer, Card::FortuneTeller]);
    assert!(g.supply_pile(Card::NobleBrigand).is_some());

    // Left to chance, none of them is a removed card, and the Bane and Ferryman's pile
    // come from the sets in the Kingdom.
    for seed in 0..20 {
        let mut config = config(&kingdom, &landscapes, &[]);
        config.seed = seed;
        let g = setup_with(config);
        let sets = kingdom.map(|c| c.set());
        let st = &g.ext.cornucopia_guilds;
        for pile in [st.bane.unwrap(), st.ferryman.unwrap()] {
            let card = g.pile(pile).card;
            assert!(sets.contains(&card.set()), "seed {seed}: {card}");
        }
        let added = g.piles().iter().map(|p| p.card).chain(black_market(&g));
        for card in added.chain(g.ext.menagerie.mouse.map(|c| g.printed(c))) {
            assert!(!card.def().removed, "seed {seed}: {card}");
        }
    }
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
fn possession_is_rejected_in_a_kingdom_and_in_a_setup_choice() {
    let unimplemented = Some(SetupError::Unimplemented(Card::Possession));
    assert_eq!(
        build(config(&[Card::Possession], &[], &[])).err(),
        unimplemented
    );
    assert_eq!(
        build(config(
            &[Card::BlackMarket],
            &[],
            &[(Card::BlackMarket, &[Card::Possession])]
        ))
        .err(),
        unimplemented
    );
}

#[test]
fn possession_is_never_dealt_or_chosen_at_random() {
    for seed in 0..300 {
        let mut config = GameConfig::random(2, &[Set::Alchemy], seed);
        assert!(!config.kingdom.contains(&Card::Possession), "seed {seed}");
        config.kingdom[..2].copy_from_slice(&[Card::BlackMarket, Card::YoungWitch]);
        config.landscapes = vec![Card::WayOfTheMouse];
        let g = build(config).expect("setup");
        assert!(!g.cards.contains(&Card::Possession), "seed {seed}");
    }
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

#[test]
fn five_and_six_players_use_two_sets_of_base_treasures() {
    for (players, coppers, provinces, curses) in [(4, 32, 12, 30), (5, 85, 15, 40), (6, 78, 18, 50)]
    {
        let mut c = config(&[Card::Gardens], &[], &[]);
        c.players = players;
        let g = setup_with(c);
        let size = |card| g.pile(g.supply_pile(card).unwrap()).cards.len();
        let doubled = if players > 4 { 2 } else { 1 };
        assert_eq!(size(Card::Copper), coppers);
        assert_eq!(size(Card::Silver), 40 * doubled);
        assert_eq!(size(Card::Gold), 30 * doubled);
        assert_eq!(size(Card::Province), provinces);
        assert_eq!(size(Card::Curse), curses);
        for card in [Card::Estate, Card::Duchy, Card::Gardens] {
            assert_eq!(size(card), 12);
        }
    }
}

#[test]
fn a_game_has_one_ally_and_one_prophecy_at_most() {
    let kingdom = [Card::Underling, Card::TeaHouse];
    let err = |landscapes: &[Card]| build(config(&kingdom, landscapes, &[])).err();
    assert_eq!(err(&[Card::IslandFolk, Card::DivineWind]), None);
    assert_eq!(
        err(&[Card::IslandFolk, Card::CaveDwellers]),
        Some(SetupError::OnlyOne(Card::IslandFolk, Card::CaveDwellers))
    );
    assert_eq!(
        err(&[Card::DivineWind, Card::ApproachingArmy]),
        Some(SetupError::OnlyOne(Card::DivineWind, Card::ApproachingArmy))
    );
}

#[test]
fn setup_rejects_more_piles_than_a_game_can_hold() {
    let kingdom: Vec<Card> = Card::kingdom_cards()
        .filter(|c| IMPLEMENTED.contains(&c.set()) && *c != Card::Possession)
        .take(260)
        .collect();
    assert_eq!(
        build(GameConfig::new(2, kingdom, 3)).err(),
        Some(SetupError::TooManyPiles)
    );
}

/// `GameConfig::random` as it dealt before slots: kept so that seeds keep their kingdoms.
fn rulebook_deal(players: u8, sets: &[Set], seed: u64) -> GameConfig {
    let mut rng = ChaCha8Rng::seed_from_u64(seed ^ 0x9e37_79b9_7f4a_7c15);
    let mut stack: Vec<Card> = Card::ALL
        .iter()
        .copied()
        .filter(|c| sets.contains(&c.set()) && !c.def().removed)
        .filter(|c| c.def().kingdom || crate::SLOT_KINDS.contains(&c.kind()))
        .collect();
    stack.shuffle(&mut rng);
    let mut config = GameConfig::new(players, vec![], seed);
    for card in stack {
        if config.kingdom.len() == 10 {
            break;
        }
        if card.def().kingdom {
            config.kingdom.push(card);
        } else if config.landscapes.len() < 2
            && !(card.kind() == Kind::Way
                && config.landscapes.iter().any(|l| l.kind() == Kind::Way))
        {
            config.landscapes.push(card);
        }
    }
    config
}

#[test]
fn a_deal_without_rules_is_the_rulebooks() {
    let sets = [Set::Base, Set::Adventures, Set::Menagerie, Set::Renaissance];
    for seed in 0..300 {
        assert_eq!(
            GameConfig::random(2, &sets, seed),
            rulebook_deal(2, &sets, seed)
        );
    }
}

#[test]
fn a_deal_keeps_required_cards_takes_one_from_each_named_set_and_skips_banned_ones() {
    let banned = [
        Card::Village,
        Card::Militia,
        Card::Chapel,
        Card::Cellar,
        Card::Moat,
    ];
    let rules = DealRules {
        sets: vec![Set::Base],
        kingdom: vec![Card::Smithy, Card::Witch],
        from_sets: vec![Set::Seaside, Set::Seaside],
        banned: banned.to_vec(),
        ..DealRules::default()
    };
    for seed in 0..200 {
        let c = GameConfig::deal(2, &rules, seed);
        assert_eq!(c.kingdom.len(), 10);
        assert_eq!(&c.kingdom[..2], &[Card::Smithy, Card::Witch]);
        assert_eq!(
            c.kingdom.iter().filter(|k| k.set() == Set::Seaside).count(),
            2
        );
        assert!(
            c.kingdom
                .iter()
                .all(|k| !banned.contains(k) && k.def().kingdom)
        );
        assert!(
            c.kingdom
                .iter()
                .all(|k| matches!(k.set(), Set::Base | Set::Seaside))
        );
        let distinct: std::collections::BTreeSet<_> = c.kingdom.iter().collect();
        assert_eq!(distinct.len(), 10);
        setup_with(GameConfig {
            colonies: Some(false),
            shelters: Some(false),
            ..c
        });
    }
}

#[test]
fn landscape_slots_take_only_their_kinds_and_ones_that_cannot_stay_empty_are_filled() {
    let event = 1 << 0;
    let project_or_way = (1 << 2) | (1 << 3);
    let rules = DealRules {
        sets: vec![Set::Base, Set::Adventures, Set::Renaissance, Set::Menagerie],
        landscapes: vec![Card::Delve],
        slots: Some(vec![
            LandscapeSlot {
                kinds: event,
                optional: false,
            },
            LandscapeSlot {
                kinds: project_or_way,
                optional: false,
            },
        ]),
        ..DealRules::default()
    };
    for seed in 0..200 {
        let c = GameConfig::deal(2, &rules, seed);
        assert_eq!(c.kingdom.len(), 10);
        assert_eq!(c.landscapes[0], Card::Delve);
        let dealt = &c.landscapes[1..];
        assert_eq!(dealt.len(), 2, "{dealt:?}");
        assert_eq!(dealt.iter().filter(|l| l.kind() == Kind::Event).count(), 1);
        assert_eq!(
            dealt
                .iter()
                .filter(|l| matches!(l.kind(), Kind::Project | Kind::Way))
                .count(),
            1
        );
    }
    let none = DealRules {
        sets: vec![Set::Adventures],
        slots: Some(vec![]),
        ..DealRules::default()
    };
    assert!((0..50).all(|s| GameConfig::deal(2, &none, s).landscapes.is_empty()));
}

/// The log is complete by itself: piles joining during play are logged, with what their
/// setup makes in the open (a Bane, Necromancer's Zombies in the trash), before what that
/// setup gives (Baker's Coffers).
#[test]
fn piles_joining_during_play_are_logged_before_what_their_setup_gives() {
    let mut g = game(&[Card::Smithy], &[], &[]);
    g.start();
    let n = g.log().len();
    add_kingdom_piles(&mut g, &[Card::YoungWitch, Card::Baker, Card::Necromancer]);
    let log = &g.log()[n..];
    let at = |e: &Event| log.iter().position(|l| l == e);
    let added = |card: Card| {
        let pile = g.supply_pile(card).unwrap();
        at(&Event::PileAdded {
            pile,
            count: g.pile(pile).cards.len(),
            supply: true,
        })
        .unwrap_or_else(|| panic!("{card}: {log:#?}"))
    };
    let bane = g.ext.cornucopia_guilds.bane.unwrap();
    let noted = at(&Event::Noted {
        note: g
            .view(P0)
            .notes()
            .into_iter()
            .find(|n| n.pile == Some(bane))
            .unwrap(),
    });
    assert!(noted > Some(added(g.pile(bane).card)));
    let coffers = log.iter().position(|e| {
        matches!(
            e,
            Event::Counter {
                counter: Counter::Coffers,
                ..
            }
        )
    });
    assert!(coffers > Some(added(Card::Baker)));
    let zombie = Event::Added {
        card: Card::ZombieSpy,
        to: Some(Zone::Trash),
    };
    assert!(at(&zombie).is_some(), "{log:#?}");
}
