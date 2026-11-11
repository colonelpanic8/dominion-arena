use super::*;
use rand::seq::SliceRandom;

fn turn_starts(g: &Game) -> usize {
    g.log()
        .iter()
        .filter(|e| matches!(e, Event::TurnStart { .. }))
        .count()
}

/// Ends phases until the next turn starts.
fn next_turn(g: &mut Game) {
    let n = turn_starts(g);
    while turn_starts(g) == n {
        match &g.pending().expect("a decision is pending").kind {
            DecisionKind::Turn { .. } => end_phase(g),
            k => panic!("unexpected decision {k:?}"),
        }
    }
}

/// Ends phases until the Night phase (the hand must hold a Night card).
fn to_night(g: &mut Game) {
    while g.turn().phase != Phase::Night {
        match &g.pending().expect("a decision is pending").kind {
            DecisionKind::Turn { .. } => end_phase(g),
            k => panic!("unexpected decision {k:?}"),
        }
    }
}

fn to_buy(g: &mut Game) {
    if g.turn().phase == Phase::Action {
        end_phase(g);
    }
}

fn trigger(g: &mut Game, i: u16) {
    respond(g, Response::option(Opt::Trigger(i)));
}

fn mode(g: &mut Game, i: u8) {
    respond(g, Response::option(Opt::Mode(i)));
}

fn pick(g: &mut Game, card: Card) {
    respond(g, Response::option(Opt::Card(card)));
}

fn in_play_kinds(g: &Game, p: PlayerId) -> Vec<Card> {
    kinds(g, g.in_play(p))
}

fn discard_kinds(g: &Game, p: PlayerId) -> Vec<Card> {
    kinds(g, &g.player(p).discard)
}

fn trash_kinds(g: &Game) -> Vec<Card> {
    kinds(g, g.trash())
}

fn pile_size(g: &Game, card: Card) -> usize {
    g.pile(g.pile_of(card).unwrap()).cards.len()
}

/// Makes `top` (top first) the next Boons; the other Boons go to the discard pile.
fn stack_boons(g: &mut Game, top: &[Card]) {
    g.boons.draw = top.iter().rev().copied().collect();
    g.boons.discard = crate::cards::nocturne::BOONS
        .iter()
        .copied()
        .filter(|b| !top.contains(b) && !g.ext.nocturne.druid.contains(b))
        .collect();
}

/// Makes `top` (top first) the next Hexes; the other Hexes go to the discard pile.
fn stack_hexes(g: &mut Game, top: &[Card]) {
    g.hexes.draw = top.iter().rev().copied().collect();
    g.hexes.discard = crate::cards::nocturne::HEXES
        .iter()
        .copied()
        .filter(|h| !top.contains(h))
        .collect();
}

fn pile_options(g: &Game) -> Vec<Card> {
    match &g.pending().expect("a decision is pending").kind {
        DecisionKind::Pile { piles, .. } => piles.iter().map(|&p| g.pile(p).card).collect(),
        k => panic!("expected a Pile decision, got {k:?}"),
    }
}

fn buy_moves(g: &Game) -> Vec<Card> {
    match &g.pending().expect("a decision is pending").kind {
        DecisionKind::Turn { moves, .. } => moves
            .iter()
            .filter_map(|m| match m {
                Move::Buy(p) => Some(g.pile(*p).card),
                _ => None,
            })
            .collect(),
        k => panic!("expected a Turn decision, got {k:?}"),
    }
}

/// Every Boon and Hex is somewhere it can come back from.
fn check_boons_and_hexes(g: &Game) {
    let boons = g.boons();
    let held: usize = g.players.iter().map(|p| p.boons_kept.len()).sum();
    let saved = g
        .lasting()
        .iter()
        .filter(|l| l.card == Card::BlessedVillage)
        .count();
    let total = boons.draw.len() + boons.discard.len() + held + saved + g.ext.nocturne.druid.len();
    assert!(total == 0 || total == 12, "{total} Boons");
    let hexes = g.hexes().draw.len() + g.hexes().discard.len();
    assert!(hexes == 0 || hexes == 12, "{hexes} Hexes");
}

/// Random games over Nocturne with Throne Room and Moat, each player starting with a copy
/// of every Kingdom card, Heirloom and non-Supply card so that all of them get played.
#[test]
fn random_nocturne_games_keep_invariants() {
    let pool: Vec<Card> = Card::in_set(Set::Nocturne)
        .filter(|c| c.def().kingdom)
        .collect();
    let extras = [
        Card::Bat,
        Card::Ghost,
        Card::Imp,
        Card::WillOWisp,
        Card::Wish,
        Card::CursedGold,
        Card::Goat,
        Card::HauntedMirror,
        Card::LuckyCoin,
        Card::MagicLamp,
        Card::Pasture,
        Card::Pouch,
    ];
    for seed in 0..150 {
        let mut rng = ChaCha8Rng::seed_from_u64(seed);
        let mut cards = pool.clone();
        cards.shuffle(&mut rng);
        cards.truncate(8);
        cards.extend([Card::ThroneRoom, Card::Moat]);
        let players = rng.random_range(2..=4);
        let mut config = GameConfig::new(players, cards.clone(), seed);
        config.turn_limit = Some(60);
        let mut g = setup_with(config);
        for p in 0..players {
            let p = PlayerId(p);
            for &c in cards.iter().chain(&extras) {
                g.create(c, Zone::Deck(p));
            }
            let mut deck = std::mem::take(&mut g.players[p.index()].deck);
            g.shuffle(&mut deck);
            g.players[p.index()].deck = deck;
        }
        g.start();
        let mut agents: Vec<Chaos> = (0..players)
            .map(|p| Chaos(ChaCha8Rng::seed_from_u64(seed * 10 + p as u64)))
            .collect();
        let mut steps = 0;
        while let Some(d) = g.pending().cloned() {
            check_zones(&g);
            let r = agents[d.player.index()].decide(&g.view(d.player), &d);
            if let Err(e) = g.respond(r.clone()) {
                panic!("seed {seed}: random response {r:?} to {d:?} rejected: {e}");
            }
            steps += 1;
            assert!(steps < 100_000, "seed {seed}: game does not end");
        }
        check_zones(&g);
        check_boons_and_hexes(&g);
    }
}

// ---- Setup ----

#[test]
fn heirlooms_replace_starting_coppers_and_the_coppers_go_to_the_pile() {
    let g = setup(&[Card::Pixie, Card::Tracker]);
    for p in [P0, P1] {
        assert_eq!(g.count_owned(p, Card::Copper), 5);
        assert_eq!(g.count_owned(p, Card::Goat), 1);
        assert_eq!(g.count_owned(p, Card::Pouch), 1);
        assert_eq!(g.count_owned(p, Card::Estate), 3);
    }
    let copper = g.supply_pile(Card::Copper).unwrap();
    assert_eq!(g.pile(copper).cards.len(), 60 - 14 + 4);
}

#[test]
fn setup_adds_boons_hexes_and_the_piles_the_cards_use() {
    let g = setup(&[
        Card::Druid,
        Card::Vampire,
        Card::Necromancer,
        Card::Exorcist,
        Card::Leprechaun,
    ]);
    assert_eq!(g.boons().draw.len(), 9);
    assert_eq!(g.ext.nocturne.druid.len(), 3);
    assert!(
        g.ext
            .nocturne
            .druid
            .iter()
            .all(|b| !g.boons().draw.contains(b))
    );
    assert_eq!(g.hexes().draw.len(), 12);
    for (card, n) in [
        (Card::WillOWisp, 12),
        (Card::Imp, 13),
        (Card::Ghost, 6),
        (Card::Wish, 12),
        (Card::Bat, 10),
    ] {
        let pile = g.pile_of(card).unwrap();
        assert!(!g.pile(pile).in_supply, "{card}");
        assert_eq!(g.pile(pile).cards.len(), n, "{card}");
    }
    assert_eq!(
        trash_kinds(&g),
        [Card::ZombieApprentice, Card::ZombieMason, Card::ZombieSpy]
    );

    let plain = setup(&[Card::Smithy, Card::Monastery]);
    assert!(plain.boons().draw.is_empty() && plain.hexes().draw.is_empty());
    assert!(plain.pile_of(Card::Imp).is_none() && plain.pile_of(Card::WillOWisp).is_none());
    assert!(plain.trash().is_empty());
}

// ---- Night and gaining to hand ----

#[test]
fn night_cards_gained_to_hand_are_played_the_same_night_without_an_action() {
    let mut g = setup(&[Card::Guardian]);
    hand(&mut g, P0, &[Card::Copper, Card::Copper]);
    deck(&mut g, P0, &[Card::Copper; 10]);
    g.start();
    to_buy(&mut g);
    play(&mut g, Card::Copper);
    play(&mut g, Card::Copper);
    buy(&mut g, Card::Guardian);
    assert_eq!(hand_kinds(&g, P0), [Card::Guardian]);
    assert_eq!(g.turn().phase, Phase::Night);
    assert_eq!(g.turn().actions, 1);
    play(&mut g, Card::Guardian);
    // The Night phase ended with no Night card left, and Guardian stays out.
    assert_eq!(g.turn().player, P1);
    assert_eq!(in_play_kinds(&g, P0), [Card::Guardian]);
}

#[test]
fn a_night_card_gained_onto_the_deck_stays_there() {
    let mut g = setup(&[Card::GhostTown, Card::Armory]);
    hand(&mut g, P0, &[Card::Armory]);
    deck(&mut g, P0, &[Card::Copper; 10]);
    g.start();
    play(&mut g, Card::Armory);
    choose_pile(&mut g, Card::GhostTown);
    assert_eq!(deck_kinds(&g, P0)[0], Card::GhostTown);
}

#[test]
fn night_watchman_den_of_sin_and_ghost_town_are_gained_to_hand() {
    let mut g = setup(&[
        Card::NightWatchman,
        Card::DenOfSin,
        Card::GhostTown,
        Card::Workshop,
    ]);
    hand(&mut g, P0, &[Card::Workshop]);
    deck(&mut g, P0, &[Card::Copper; 10]);
    g.start();
    play(&mut g, Card::Workshop);
    choose_pile(&mut g, Card::NightWatchman);
    assert_eq!(hand_kinds(&g, P0), [Card::NightWatchman]);
}

// ---- Durations ----

#[test]
fn guardian_blocks_attacks_until_its_owners_next_turn_and_then_gives_a_coin() {
    let mut g = setup(&[Card::Guardian, Card::Militia]);
    hand(&mut g, P0, &[Card::Guardian]);
    deck(&mut g, P0, &[Card::Copper; 15]);
    hand(
        &mut g,
        P1,
        &[
            Card::Militia,
            Card::Copper,
            Card::Copper,
            Card::Copper,
            Card::Copper,
        ],
    );
    deck(&mut g, P1, &[Card::Militia; 10]);
    g.start();
    to_night(&mut g);
    play(&mut g, Card::Guardian);
    play(&mut g, Card::Militia);
    assert_eq!(g.hand(P0).len(), 5);
    assert_eq!(g.turn().coins, 2);
    next_turn(&mut g);
    assert_eq!(g.turn().coins, 1);
    assert!(g.lasting().is_empty());
    assert_eq!(in_play_kinds(&g, P0), [Card::Guardian]);
    next_turn(&mut g);
    play(&mut g, Card::Militia);
    assert_eq!(g.hand(P0).len(), 3);
}

#[test]
fn ghost_town_and_den_of_sin_draw_next_turn() {
    let mut g = setup(&[Card::GhostTown, Card::DenOfSin]);
    hand(&mut g, P0, &[Card::GhostTown, Card::DenOfSin]);
    deck(&mut g, P0, &[Card::Copper; 15]);
    g.start();
    to_night(&mut g);
    play(&mut g, Card::GhostTown);
    play(&mut g, Card::DenOfSin);
    assert_eq!(in_play_kinds(&g, P0), [Card::GhostTown, Card::DenOfSin]);
    next_turn(&mut g);
    trigger(&mut g, 0);
    assert_eq!(g.hand(P0).len(), 8);
    assert_eq!(g.turn().actions, 2);
    next_turn(&mut g);
    assert!(g.in_play(P0).is_empty());
}

#[test]
fn cobbler_gains_a_card_costing_up_to_four_to_hand_next_turn() {
    let mut g = setup(&[Card::Cobbler, Card::Smithy]);
    hand(&mut g, P0, &[Card::Cobbler]);
    deck(&mut g, P0, &[Card::Copper; 15]);
    g.start();
    to_night(&mut g);
    play(&mut g, Card::Cobbler);
    next_turn(&mut g);
    assert!(pile_options(&g).contains(&Card::Smithy));
    assert!(!pile_options(&g).contains(&Card::Gold));
    choose_pile(&mut g, Card::Smithy);
    assert!(hand_kinds(&g, P0).contains(&Card::Smithy));
    assert_eq!(g.hand(P0).len(), 6);
}

#[test]
fn crypt_sets_treasures_aside_and_returns_one_each_turn() {
    let mut g = setup(&[Card::Crypt]);
    hand(
        &mut g,
        P0,
        &[Card::Silver, Card::Gold, Card::Copper, Card::Crypt],
    );
    deck(&mut g, P0, &[Card::Estate; 20]);
    g.start();
    to_buy(&mut g);
    play(&mut g, Card::Silver);
    play(&mut g, Card::Gold);
    play(&mut g, Card::Copper);
    to_night(&mut g);
    play(&mut g, Card::Crypt);
    choose_kinds(&mut g, &[Card::Silver, Card::Gold]);
    assert_eq!(in_play_kinds(&g, P0), [Card::Crypt]);
    assert_eq!(g.view(P1).card(g.player(P0).set_aside[0]), None);
    next_turn(&mut g);
    choose_kinds(&mut g, &[Card::Gold]);
    assert!(hand_kinds(&g, P0).contains(&Card::Gold));
    next_turn(&mut g);
    assert_eq!(in_play_kinds(&g, P0), [Card::Crypt]);
    next_turn(&mut g);
    assert!(hand_kinds(&g, P0).contains(&Card::Silver));
    assert!(g.lasting().is_empty());
    next_turn(&mut g);
    assert!(g.in_play(P0).is_empty());
}

#[test]
fn raider_makes_five_card_hands_discard_a_copy_and_pays_three_next_turn() {
    let mut g = setup_with(GameConfig {
        colonies: Some(false),
        shelters: Some(false),
        ..GameConfig::new(3, vec![Card::Raider], 7)
    });
    hand(&mut g, P0, &[Card::Copper, Card::Raider]);
    deck(&mut g, P0, &[Card::Estate; 15]);
    hand(
        &mut g,
        P1,
        &[
            Card::Copper,
            Card::Silver,
            Card::Estate,
            Card::Estate,
            Card::Estate,
        ],
    );
    hand(
        &mut g,
        P2,
        &[Card::Copper, Card::Copper, Card::Copper, Card::Copper],
    );
    g.start();
    to_buy(&mut g);
    play(&mut g, Card::Copper);
    to_night(&mut g);
    play(&mut g, Card::Raider);
    assert_eq!(
        hand_kinds(&g, P1),
        [Card::Silver, Card::Estate, Card::Estate, Card::Estate]
    );
    assert_eq!(g.hand(P2).len(), 4);
    next_turn(&mut g);
    next_turn(&mut g);
    assert_eq!(g.turn().coins, 3);
}

#[test]
fn raider_against_a_hand_without_a_copy_reveals_it() {
    let mut g = setup(&[Card::Raider]);
    hand(&mut g, P0, &[Card::Raider]);
    hand(&mut g, P1, &[Card::Estate; 5]);
    g.start();
    to_night(&mut g);
    play(&mut g, Card::Raider);
    assert_eq!(g.hand(P1).len(), 5);
    assert!(
        g.log()
            .iter()
            .any(|e| matches!(e, Event::RevealedHand { player, .. } if *player == P1))
    );
}

#[test]
fn secret_cave_discards_three_for_three_coins_next_turn() {
    let mut g = setup(&[Card::SecretCave]);
    hand(
        &mut g,
        P0,
        &[Card::SecretCave, Card::Estate, Card::Estate, Card::Estate],
    );
    deck(&mut g, P0, &[Card::Copper; 15]);
    g.start();
    play(&mut g, Card::SecretCave);
    assert_eq!(g.turn().actions, 1);
    yes(&mut g);
    choose_kinds(&mut g, &[Card::Estate, Card::Estate, Card::Estate]);
    assert_eq!(hand_kinds(&g, P0), [Card::Copper]);
    next_turn(&mut g);
    next_turn(&mut g);
    assert_eq!(g.turn().coins, 3);
    assert_eq!(in_play_kinds(&g, P0), [Card::SecretCave]);
}

#[test]
fn secret_cave_discarding_fewer_than_three_gives_nothing() {
    let mut g = setup(&[Card::SecretCave]);
    hand(&mut g, P0, &[Card::SecretCave, Card::Estate]);
    deck(&mut g, P0, &[Card::Copper; 15]);
    g.start();
    play(&mut g, Card::SecretCave);
    yes(&mut g);
    assert!(g.hand(P0).is_empty());
    next_turn(&mut g);
    assert!(g.in_play(P0).is_empty());
    assert!(g.lasting().is_empty());
}

#[test]
fn ghost_sets_an_action_aside_and_plays_it_twice_next_turn() {
    let mut g = setup(&[Card::Exorcist, Card::Village]);
    hand(&mut g, P0, &[Card::Ghost, Card::Monastery]);
    deck(
        &mut g,
        P0,
        &[Card::Copper, Card::Estate, Card::Village, Card::Silver],
    );
    discard(&mut g, P0, &[Card::Copper; 10]);
    g.start();
    to_night(&mut g);
    play(&mut g, Card::Ghost);
    assert_eq!(kinds(&g, &g.player(P0).set_aside), [Card::Village]);
    assert_eq!(discard_kinds(&g, P0)[10..], [Card::Copper, Card::Estate]);
    next_turn(&mut g);
    assert_eq!(in_play_kinds(&g, P0), [Card::Ghost]);
    next_turn(&mut g);
    assert_eq!(in_play_kinds(&g, P0), [Card::Ghost, Card::Village]);
    assert_eq!(g.turn().actions, 5);
    assert_eq!(g.hand(P0).len(), 7);
    next_turn(&mut g);
    assert!(g.in_play(P0).is_empty());
}

#[test]
fn ghost_stays_out_with_a_duration_it_plays() {
    let mut g = setup(&[Card::Exorcist, Card::Caravan]);
    hand(&mut g, P0, &[Card::Ghost]);
    deck(&mut g, P0, &[Card::Caravan]);
    discard(&mut g, P0, &[Card::Copper; 15]);
    g.start();
    to_night(&mut g);
    play(&mut g, Card::Ghost);
    next_turn(&mut g);
    assert_eq!(g.hand(P0).len(), 7);
    next_turn(&mut g);
    next_turn(&mut g);
    assert_eq!(in_play_kinds(&g, P0), [Card::Ghost, Card::Caravan]);
    trigger(&mut g, 0);
    assert_eq!(g.hand(P0).len(), 7);
    next_turn(&mut g);
    assert!(g.in_play(P0).is_empty());
}

#[test]
fn ghost_without_an_action_discards_everything_and_leaves_play() {
    let mut g = setup(&[Card::Exorcist]);
    hand(&mut g, P0, &[Card::Ghost]);
    deck(&mut g, P0, &[Card::Copper, Card::Estate]);
    discard(&mut g, P0, &[Card::Silver]);
    g.start();
    to_night(&mut g);
    play(&mut g, Card::Ghost);
    assert!(g.lasting().is_empty());
    next_turn(&mut g);
    assert!(g.in_play(P0).is_empty());
}

#[test]
fn night_watchman_discards_and_reorders_the_top_five() {
    let mut g = setup(&[Card::NightWatchman]);
    hand(&mut g, P0, &[Card::NightWatchman, Card::Monastery]);
    deck(
        &mut g,
        P0,
        &[
            Card::Estate,
            Card::Copper,
            Card::Silver,
            Card::Gold,
            Card::Province,
            Card::Duchy,
        ],
    );
    g.start();
    to_night(&mut g);
    play(&mut g, Card::NightWatchman);
    choose_kinds(&mut g, &[Card::Estate, Card::Province]);
    choose_kinds(&mut g, &[Card::Gold, Card::Silver, Card::Copper]);
    assert_eq!(
        deck_kinds(&g, P0),
        [Card::Gold, Card::Silver, Card::Copper, Card::Duchy]
    );
    assert_eq!(discard_kinds(&g, P0), [Card::Estate, Card::Province]);
}

// ---- Night cards ----

fn decline(g: &mut Game) {
    respond(g, Response::option(Opt::No));
}

#[test]
fn changeling_trashes_itself_and_gains_a_copy_of_a_card_in_play() {
    let mut g = setup(&[Card::Changeling]);
    hand(
        &mut g,
        P0,
        &[Card::Silver, Card::Changeling, Card::Guardian],
    );
    g.start();
    to_buy(&mut g);
    play(&mut g, Card::Silver);
    to_night(&mut g);
    play(&mut g, Card::Changeling);
    // Gaining the Silver may exchange it for a Changeling.
    assert_eq!(purpose(&g), Purpose::Trigger);
    decline(&mut g);
    assert_eq!(trash_kinds(&g).last(), Some(&Card::Changeling));
    assert_eq!(discard_kinds(&g, P0), [Card::Silver]);
}

#[test]
fn a_gained_card_costing_three_or_more_may_be_exchanged_for_a_changeling() {
    let mut g = setup(&[Card::Changeling, Card::Skulk]);
    hand(&mut g, P0, &[Card::Silver, Card::Silver]);
    g.start();
    to_buy(&mut g);
    play(&mut g, Card::Silver);
    play(&mut g, Card::Silver);
    buy(&mut g, Card::Skulk);
    // Skulk's Gold and the exchange: exchange first; the Gold still comes.
    trigger(&mut g, 1);
    // The Gold may be exchanged too.
    decline(&mut g);
    assert_eq!(discard_kinds(&g, P0)[..2], [Card::Changeling, Card::Gold]);
    assert_eq!(pile_size(&g, Card::Skulk), 10);
    assert_eq!(pile_size(&g, Card::Changeling), 9);
}

#[test]
fn cheap_cards_cannot_be_exchanged_for_a_changeling() {
    let mut g = setup(&[Card::Changeling]);
    hand(&mut g, P0, &[Card::Copper, Card::Copper]);
    g.start();
    to_buy(&mut g);
    play(&mut g, Card::Copper);
    play(&mut g, Card::Copper);
    buy(&mut g, Card::Estate);
    assert_eq!(g.turn().player, P1);
    assert_eq!(g.count_owned(P0, Card::Changeling), 0);
    assert_eq!(pile_size(&g, Card::Estate), 7);
}

#[test]
fn devils_workshop_depends_on_the_cards_gained_this_turn() {
    let mut g = setup(&[Card::DevilsWorkshop]);
    hand(&mut g, P0, &[Card::DevilsWorkshop]);
    g.start();
    to_night(&mut g);
    play(&mut g, Card::DevilsWorkshop);
    assert_eq!(discard_kinds(&g, P0)[..1], [Card::Gold]);

    let mut g = setup(&[Card::DevilsWorkshop, Card::Smithy]);
    hand(&mut g, P0, &[Card::DevilsWorkshop]);
    g.start();
    to_buy(&mut g);
    buy(&mut g, Card::Copper);
    play(&mut g, Card::DevilsWorkshop);
    assert!(!pile_options(&g).contains(&Card::Gold));
    choose_pile(&mut g, Card::Smithy);
    assert_eq!(discard_kinds(&g, P0)[..2], [Card::Copper, Card::Smithy]);

    let mut g = setup(&[Card::DevilsWorkshop]);
    hand(&mut g, P0, &[Card::Pouch, Card::DevilsWorkshop]);
    g.start();
    to_buy(&mut g);
    play(&mut g, Card::Pouch);
    buy(&mut g, Card::Copper);
    buy(&mut g, Card::Copper);
    play(&mut g, Card::DevilsWorkshop);
    assert_eq!(
        discard_kinds(&g, P0)[..3],
        [Card::Copper, Card::Copper, Card::Imp]
    );
}

#[test]
fn exorcist_trashes_a_card_to_gain_a_cheaper_spirit() {
    let mut g = setup(&[Card::Exorcist]);
    hand(&mut g, P0, &[Card::Exorcist, Card::Silver, Card::Guardian]);
    g.start();
    to_night(&mut g);
    play(&mut g, Card::Exorcist);
    choose_kinds(&mut g, &[Card::Silver]);
    assert_eq!(pile_options(&g), [Card::WillOWisp, Card::Imp]);
    choose_pile(&mut g, Card::Imp);
    assert_eq!(discard_kinds(&g, P0), [Card::Imp]);
}

#[test]
fn monastery_trashes_one_card_per_card_gained_from_hand_or_coppers_in_play() {
    let mut g = setup(&[Card::Monastery]);
    hand(
        &mut g,
        P0,
        &[Card::Pouch, Card::Copper, Card::Monastery, Card::Estate],
    );
    g.start();
    to_buy(&mut g);
    play(&mut g, Card::Pouch);
    play(&mut g, Card::Copper);
    buy(&mut g, Card::Copper);
    buy(&mut g, Card::Copper);
    play(&mut g, Card::Monastery);
    assert_eq!(kinds(&g, &candidates(&g)), [Card::Estate, Card::Copper]);
    choose_kinds(&mut g, &[Card::Estate]);
    choose_kinds(&mut g, &[Card::Copper]);
    assert_eq!(
        trash_kinds(&g)[trash_kinds(&g).len() - 2..],
        [Card::Estate, Card::Copper]
    );
    assert_eq!(g.turn().player, P1);
}

#[test]
fn vampire_hexes_gains_a_card_and_turns_into_a_bat() {
    let mut g = setup(&[Card::Vampire]);
    hand(&mut g, P0, &[Card::Vampire]);
    stack_hexes(&mut g, &[Card::Greed]);
    g.start();
    to_night(&mut g);
    play(&mut g, Card::Vampire);
    assert_eq!(deck_kinds(&g, P1)[0], Card::Copper);
    assert!(!pile_options(&g).contains(&Card::Vampire));
    assert!(!pile_options(&g).contains(&Card::Gold));
    choose_pile(&mut g, Card::Duchy);
    assert_eq!(discard_kinds(&g, P0)[..2], [Card::Duchy, Card::Bat]);
    assert_eq!(pile_size(&g, Card::Vampire), 11);
    assert_eq!(pile_size(&g, Card::Bat), 9);
    assert_eq!(g.hexes().discard.len(), 12);
}

#[test]
fn bat_trashes_up_to_two_cards_and_if_it_did_turns_into_a_vampire() {
    let mut g = setup(&[Card::Vampire]);
    hand(
        &mut g,
        P0,
        &[Card::Bat, Card::Estate, Card::Copper, Card::Guardian],
    );
    g.start();
    to_night(&mut g);
    play(&mut g, Card::Bat);
    choose_kinds(&mut g, &[Card::Estate, Card::Copper]);
    assert_eq!(discard_kinds(&g, P0), [Card::Vampire]);
    assert_eq!(pile_size(&g, Card::Vampire), 9);
    assert!(g.in_play(P0).is_empty());

    let mut g = setup(&[Card::Vampire]);
    hand(&mut g, P0, &[Card::Bat, Card::Estate, Card::Guardian]);
    g.start();
    to_night(&mut g);
    play(&mut g, Card::Bat);
    choose(&mut g, &[]);
    assert_eq!(in_play_kinds(&g, P0), [Card::Bat]);
}

#[test]
fn werewolf_draws_by_day_and_hexes_at_night() {
    let mut g = setup(&[Card::Werewolf]);
    hand(&mut g, P0, &[Card::Werewolf]);
    deck(&mut g, P0, &[Card::Copper; 10]);
    g.start();
    play(&mut g, Card::Werewolf);
    assert_eq!(g.hand(P0).len(), 3);

    let mut g = setup(&[Card::Werewolf]);
    hand(&mut g, P0, &[Card::Werewolf, Card::Guardian]);
    deck(&mut g, P0, &[Card::Copper; 10]);
    stack_hexes(&mut g, &[Card::Greed]);
    g.start();
    to_night(&mut g);
    play(&mut g, Card::Werewolf);
    assert_eq!(g.hand(P0).len(), 1);
    assert_eq!(deck_kinds(&g, P1)[0], Card::Copper);
}

#[test]
fn moat_stops_a_hex_and_no_hex_is_turned_over_for_nobody() {
    let mut g = setup(&[Card::Skulk, Card::Moat]);
    hand(&mut g, P0, &[Card::Skulk]);
    hand(&mut g, P1, &[Card::Moat]);
    stack_hexes(&mut g, &[Card::Greed]);
    g.start();
    play(&mut g, Card::Skulk);
    trigger(&mut g, 0);
    assert_eq!(g.hexes().draw, [Card::Greed]);
    assert_eq!(g.turn().buys, 2);
}

#[test]
fn one_hex_is_received_by_each_other_player() {
    let mut g = setup_with(GameConfig {
        colonies: Some(false),
        shelters: Some(false),
        ..GameConfig::new(3, vec![Card::Skulk], 7)
    });
    hand(&mut g, P0, &[Card::Skulk]);
    stack_hexes(&mut g, &[Card::Greed, Card::Plague]);
    g.start();
    play(&mut g, Card::Skulk);
    assert_eq!(deck_kinds(&g, P1)[0], Card::Copper);
    assert_eq!(deck_kinds(&g, P2)[0], Card::Copper);
    assert_eq!(g.hexes().draw, [Card::Plague]);
}

#[test]
fn skulk_gains_a_gold_when_gained() {
    let mut g = setup(&[Card::Skulk]);
    hand(&mut g, P0, &[Card::Silver, Card::Silver]);
    g.start();
    to_buy(&mut g);
    play(&mut g, Card::Silver);
    play(&mut g, Card::Silver);
    buy(&mut g, Card::Skulk);
    assert_eq!(discard_kinds(&g, P0)[..2], [Card::Skulk, Card::Gold]);
}

#[test]
fn tormentor_gains_an_imp_alone_and_hexes_otherwise() {
    let mut g = setup(&[Card::Tormentor]);
    hand(&mut g, P0, &[Card::Tormentor]);
    stack_hexes(&mut g, &[Card::Greed]);
    g.start();
    play(&mut g, Card::Tormentor);
    assert_eq!(discard_kinds(&g, P0), [Card::Imp]);
    assert_eq!(g.turn().coins, 2);
    assert_eq!(g.hexes().draw, [Card::Greed]);

    let mut g = setup(&[Card::Tormentor, Card::Village]);
    hand(&mut g, P0, &[Card::Village, Card::Tormentor]);
    deck(&mut g, P0, &[Card::Estate; 5]);
    stack_hexes(&mut g, &[Card::Greed]);
    g.start();
    play(&mut g, Card::Village);
    play(&mut g, Card::Tormentor);
    assert!(discard_kinds(&g, P0).is_empty());
    assert_eq!(deck_kinds(&g, P1)[0], Card::Copper);
}

#[test]
fn cursed_village_draws_to_six_and_hexes_when_gained() {
    let mut g = setup(&[Card::CursedVillage]);
    hand(&mut g, P0, &[Card::CursedVillage, Card::Copper]);
    deck(&mut g, P0, &[Card::Copper; 10]);
    g.start();
    play(&mut g, Card::CursedVillage);
    assert_eq!(g.hand(P0).len(), 6);
    assert_eq!(g.turn().actions, 2);

    let mut g = setup(&[Card::CursedVillage]);
    hand(&mut g, P0, &[Card::Gold, Card::Silver]);
    deck(&mut g, P0, &[Card::Estate; 10]);
    stack_hexes(&mut g, &[Card::Greed]);
    g.start();
    to_buy(&mut g);
    play(&mut g, Card::Gold);
    play(&mut g, Card::Silver);
    buy(&mut g, Card::CursedVillage);
    assert_eq!(g.turn().player, P1);
    assert_eq!(g.hand(P0).len(), 5);
    assert!(hand_kinds(&g, P0).contains(&Card::Copper));
}

#[test]
fn leprechaun_gains_a_gold_and_a_hex_or_a_wish_with_exactly_seven_in_play() {
    let mut g = setup(&[Card::Leprechaun]);
    hand(&mut g, P0, &[Card::Leprechaun]);
    deck(&mut g, P0, &[Card::Estate; 10]);
    stack_hexes(&mut g, &[Card::Greed]);
    g.start();
    play(&mut g, Card::Leprechaun);
    assert_eq!(discard_kinds(&g, P0), [Card::Gold]);
    assert_eq!(deck_kinds(&g, P0)[0], Card::Copper);

    let mut g = setup(&[Card::Leprechaun]);
    hand(&mut g, P0, &[Card::Leprechaun]);
    for _ in 0..6 {
        g.create(Card::Copper, Zone::InPlay(P0));
    }
    stack_hexes(&mut g, &[Card::Greed]);
    g.start();
    play(&mut g, Card::Leprechaun);
    assert_eq!(discard_kinds(&g, P0), [Card::Gold, Card::Wish]);
    assert_eq!(g.hexes().draw, [Card::Greed]);
}

#[test]
fn necromancer_plays_an_action_from_the_trash_once_per_turn() {
    let mut g = setup(&[Card::Necromancer, Card::Village]);
    hand(
        &mut g,
        P0,
        &[Card::Village, Card::Necromancer, Card::Necromancer],
    );
    deck(&mut g, P0, &[Card::Copper; 15]);
    g.start();
    play(&mut g, Card::Village);
    play(&mut g, Card::Necromancer);
    assert_eq!(
        kinds(&g, &candidates(&g)),
        [Card::ZombieApprentice, Card::ZombieMason, Card::ZombieSpy]
    );
    choose_kinds(&mut g, &[Card::ZombieSpy]);
    no(&mut g);
    assert!(trash_kinds(&g).contains(&Card::ZombieSpy));
    assert_eq!(g.turn().actions, 2);
    play(&mut g, Card::Necromancer);
    assert_eq!(
        kinds(&g, &candidates(&g)),
        [Card::ZombieApprentice, Card::ZombieMason]
    );
    choose_kinds(&mut g, &[Card::ZombieMason]);
    choose_pile(&mut g, Card::Copper);
    next_turn(&mut g);
    next_turn(&mut g);
    assert!(g.lasting().is_empty());
}

#[test]
fn necromancer_cannot_trash_a_pixie_it_plays_but_tragic_hero_still_gains() {
    let mut g = setup(&[Card::Necromancer, Card::Pixie]);
    hand(&mut g, P0, &[Card::Necromancer]);
    deck(&mut g, P0, &[Card::Copper; 10]);
    let pixie = g.create(Card::Pixie, Zone::Trash);
    stack_boons(&mut g, &[Card::TheSeasGift]);
    g.start();
    play(&mut g, Card::Necromancer);
    choose(&mut g, &[pixie]);
    assert_eq!(g.zone(pixie), Zone::Trash);
    assert_eq!(g.boons().discard.last(), Some(&Card::TheSeasGift));
    assert_eq!(g.turn().actions, 1);

    let mut g = setup(&[Card::Necromancer, Card::TragicHero]);
    hand(
        &mut g,
        P0,
        &[
            Card::Necromancer,
            Card::Copper,
            Card::Copper,
            Card::Copper,
            Card::Copper,
            Card::Copper,
        ],
    );
    deck(&mut g, P0, &[Card::Copper; 10]);
    let hero = g.create(Card::TragicHero, Zone::Trash);
    g.start();
    play(&mut g, Card::Necromancer);
    choose(&mut g, &[hero]);
    assert_eq!(g.hand(P0).len(), 8);
    choose_pile(&mut g, Card::Gold);
    assert_eq!(g.zone(hero), Zone::Trash);
    assert_eq!(discard_kinds(&g, P0), [Card::Gold]);
}

#[test]
fn conclave_and_imp_play_an_action_not_in_play() {
    let mut g = setup(&[Card::Conclave, Card::Village]);
    hand(&mut g, P0, &[Card::Conclave, Card::Conclave, Card::Village]);
    deck(&mut g, P0, &[Card::Copper; 10]);
    g.start();
    play(&mut g, Card::Conclave);
    assert_eq!(kinds(&g, &candidates(&g)), [Card::Village]);
    choose_kinds(&mut g, &[Card::Village]);
    assert_eq!((g.turn().actions, g.turn().coins), (3, 2));

    let mut g = setup(&[Card::Tormentor, Card::Village]);
    hand(&mut g, P0, &[Card::Imp, Card::Imp, Card::Village]);
    deck(&mut g, P0, &[Card::Copper; 10]);
    g.start();
    play(&mut g, Card::Imp);
    assert_eq!(kinds(&g, &candidates(&g)), [Card::Village]);
    choose_kinds(&mut g, &[Card::Village]);
    assert_eq!(g.turn().actions, 2);
    assert_eq!(g.hand(P0).len(), 4);
}

#[test]
fn tragic_hero_with_eight_cards_trashes_itself_for_a_treasure() {
    let mut g = setup(&[Card::TragicHero]);
    let mut cards = vec![Card::TragicHero];
    cards.extend([Card::Copper; 5]);
    hand(&mut g, P0, &cards);
    deck(&mut g, P0, &[Card::Estate; 10]);
    g.start();
    play(&mut g, Card::TragicHero);
    assert!(pile_options(&g).iter().all(|c| c.is(Types::TREASURE)));
    choose_pile(&mut g, Card::Gold);
    assert_eq!(trash_kinds(&g).last(), Some(&Card::TragicHero));
    assert_eq!(discard_kinds(&g, P0), [Card::Gold]);
    assert_eq!(g.turn().buys, 2);

    let mut g = setup(&[Card::TragicHero]);
    hand(&mut g, P0, &[Card::TragicHero, Card::Copper]);
    deck(&mut g, P0, &[Card::Estate; 10]);
    g.start();
    play(&mut g, Card::TragicHero);
    assert_eq!(in_play_kinds(&g, P0), [Card::TragicHero]);
}

#[test]
fn pooka_trashes_a_treasure_other_than_cursed_gold_for_four_cards() {
    let mut g = setup(&[Card::Pooka]);
    hand(&mut g, P0, &[Card::Pooka, Card::CursedGold, Card::Silver]);
    deck(&mut g, P0, &[Card::Copper; 10]);
    g.start();
    play(&mut g, Card::Pooka);
    assert_eq!(kinds(&g, &candidates(&g)), [Card::Silver]);
    choose_kinds(&mut g, &[Card::Silver]);
    assert_eq!(g.hand(P0).len(), 5);
}

#[test]
fn shepherd_discards_victory_cards_for_two_cards_each() {
    let mut g = setup(&[Card::Shepherd]);
    hand(
        &mut g,
        P0,
        &[Card::Shepherd, Card::Estate, Card::Pasture, Card::Copper],
    );
    deck(&mut g, P0, &[Card::Copper; 10]);
    g.start();
    play(&mut g, Card::Shepherd);
    choose_kinds(&mut g, &[Card::Estate, Card::Pasture]);
    assert_eq!(g.hand(P0).len(), 5);
    assert_eq!(g.turn().actions, 1);
}

#[test]
fn cemetery_trashes_up_to_four_cards_when_gained() {
    let mut g = setup(&[Card::Cemetery]);
    hand(
        &mut g,
        P0,
        &[
            Card::Silver,
            Card::Silver,
            Card::Estate,
            Card::Estate,
            Card::Curse,
        ],
    );
    g.start();
    to_buy(&mut g);
    play(&mut g, Card::Silver);
    play(&mut g, Card::Silver);
    buy(&mut g, Card::Cemetery);
    choose_kinds(&mut g, &[Card::Estate, Card::Curse]);
    assert_eq!(g.count_owned(P0, Card::Cemetery), 1);
    assert_eq!(g.count_owned(P0, Card::Curse), 0);
    assert!(trash_kinds(&g).ends_with(&[Card::Estate, Card::Curse]));
}

#[test]
fn faithful_hound_discarded_to_an_attack_returns_to_hand_at_end_of_turn() {
    let mut g = setup(&[Card::FaithfulHound, Card::Militia]);
    hand(&mut g, P0, &[Card::Militia]);
    hand(
        &mut g,
        P1,
        &[
            Card::FaithfulHound,
            Card::Copper,
            Card::Copper,
            Card::Copper,
            Card::Estate,
        ],
    );
    g.start();
    play(&mut g, Card::Militia);
    choose_kinds(&mut g, &[Card::FaithfulHound, Card::Estate]);
    trigger(&mut g, 0);
    assert_eq!(kinds(&g, &g.player(P1).set_aside), [Card::FaithfulHound]);
    next_turn(&mut g);
    assert_eq!(
        hand_kinds(&g, P1),
        [
            Card::Copper,
            Card::Copper,
            Card::Copper,
            Card::FaithfulHound
        ]
    );
    assert!(g.lasting().is_empty());
}

#[test]
fn faithful_hound_draws_two_and_is_not_offered_in_cleanup() {
    let mut g = setup(&[Card::FaithfulHound]);
    hand(&mut g, P0, &[Card::FaithfulHound]);
    deck(&mut g, P0, &[Card::Copper; 10]);
    g.start();
    play(&mut g, Card::FaithfulHound);
    assert_eq!(g.hand(P0).len(), 2);
    next_turn(&mut g);
    assert_eq!(discard_kinds(&g, P0).len(), 3);
}

// ---- Heirlooms, Spirits, Wish and Zombies ----

#[test]
fn heirloom_treasures() {
    let mut g = setup(&[Card::Pooka, Card::Fool, Card::Pixie, Card::Tracker]);
    hand(
        &mut g,
        P0,
        &[
            Card::CursedGold,
            Card::LuckyCoin,
            Card::Goat,
            Card::Pouch,
            Card::Estate,
        ],
    );
    g.start();
    to_buy(&mut g);
    play(&mut g, Card::CursedGold);
    play(&mut g, Card::LuckyCoin);
    play(&mut g, Card::Goat);
    choose_kinds(&mut g, &[Card::Estate]);
    play(&mut g, Card::Pouch);
    assert_eq!((g.turn().coins, g.turn().buys), (6, 2));
    assert_eq!(discard_kinds(&g, P0), [Card::Curse, Card::Silver]);
    assert_eq!(trash_kinds(&g).last(), Some(&Card::Estate));
}

#[test]
fn pasture_is_worth_a_vp_per_estate() {
    let mut g = setup(&[Card::Shepherd]);
    hand(&mut g, P0, &[Card::Pasture, Card::Estate]);
    deck(&mut g, P0, &[Card::Estate, Card::Estate]);
    discard(&mut g, P0, &[]);
    assert_eq!(g.score(P0), 3 + 3);
}

#[test]
fn magic_lamp_with_six_single_cards_in_play_becomes_three_wishes() {
    let mut g = setup(&[Card::SecretCave]);
    hand(
        &mut g,
        P0,
        &[
            Card::Copper,
            Card::Silver,
            Card::Gold,
            Card::Pouch,
            Card::Goat,
            Card::MagicLamp,
        ],
    );
    g.start();
    to_buy(&mut g);
    for c in [Card::Copper, Card::Silver, Card::Gold, Card::Pouch] {
        play(&mut g, c);
    }
    play(&mut g, Card::Goat);
    choose(&mut g, &[]);
    play(&mut g, Card::MagicLamp);
    assert_eq!(trash_kinds(&g).last(), Some(&Card::MagicLamp));
    assert_eq!(discard_kinds(&g, P0), [Card::Wish; 3]);
    assert_eq!(g.turn().coins, 1 + 2 + 3 + 1 + 1 + 1);

    let mut g = setup(&[Card::SecretCave]);
    hand(
        &mut g,
        P0,
        &[
            Card::Copper,
            Card::Copper,
            Card::Silver,
            Card::Gold,
            Card::Pouch,
            Card::MagicLamp,
        ],
    );
    g.start();
    to_buy(&mut g);
    for c in [
        Card::Copper,
        Card::Copper,
        Card::Silver,
        Card::Gold,
        Card::Pouch,
    ] {
        play(&mut g, c);
    }
    play(&mut g, Card::MagicLamp);
    assert!(in_play_kinds(&g, P0).contains(&Card::MagicLamp));
}

#[test]
fn haunted_mirror_trashed_may_discard_an_action_for_a_ghost() {
    let mut g = setup(&[Card::Cemetery, Card::Chapel]);
    hand(
        &mut g,
        P0,
        &[Card::Chapel, Card::HauntedMirror, Card::Village],
    );
    g.start();
    play(&mut g, Card::Chapel);
    choose_kinds(&mut g, &[Card::HauntedMirror]);
    trigger(&mut g, 0);
    assert_eq!(discard_kinds(&g, P0), [Card::Village, Card::Ghost]);
}

#[test]
fn will_o_wisp_takes_the_revealed_card_if_it_costs_two_or_less() {
    let mut g = setup(&[Card::Bard]);
    hand(&mut g, P0, &[Card::WillOWisp]);
    deck(&mut g, P0, &[Card::Copper, Card::Estate, Card::Silver]);
    g.start();
    play(&mut g, Card::WillOWisp);
    assert_eq!(hand_kinds(&g, P0), [Card::Copper, Card::Estate]);
    assert_eq!(g.turn().actions, 1);

    let mut g = setup(&[Card::Bard]);
    hand(&mut g, P0, &[Card::WillOWisp]);
    deck(&mut g, P0, &[Card::Copper, Card::Silver]);
    g.start();
    play(&mut g, Card::WillOWisp);
    assert_eq!(hand_kinds(&g, P0), [Card::Copper]);
    assert_eq!(deck_kinds(&g, P0), [Card::Silver]);
}

#[test]
fn wish_returns_to_its_pile_to_gain_a_card_to_hand() {
    let mut g = setup(&[Card::Leprechaun]);
    hand(&mut g, P0, &[Card::Wish]);
    g.start();
    play(&mut g, Card::Wish);
    assert!(pile_options(&g).contains(&Card::Gold));
    assert!(!pile_options(&g).contains(&Card::Province));
    choose_pile(&mut g, Card::Gold);
    assert_eq!(hand_kinds(&g, P0), [Card::Gold]);
    assert_eq!(pile_size(&g, Card::Wish), 13);
}

#[test]
fn the_zombies() {
    let mut g = setup(&[Card::Necromancer, Card::Village]);
    hand(&mut g, P0, &[Card::ZombieApprentice, Card::Village]);
    deck(&mut g, P0, &[Card::Copper; 10]);
    g.start();
    play(&mut g, Card::ZombieApprentice);
    choose_kinds(&mut g, &[Card::Village]);
    assert_eq!((g.hand(P0).len(), g.turn().actions), (3, 1));

    let mut g = setup(&[Card::Necromancer, Card::Smithy]);
    hand(&mut g, P0, &[Card::ZombieMason]);
    deck(&mut g, P0, &[Card::Silver]);
    g.start();
    play(&mut g, Card::ZombieMason);
    assert_eq!(trash_kinds(&g).last(), Some(&Card::Silver));
    assert!(pile_options(&g).contains(&Card::Smithy));
    assert!(!pile_options(&g).contains(&Card::Gold));
    respond(&mut g, Response::Pile(None));
    assert!(g.player(P0).discard.is_empty());

    let mut g = setup(&[Card::Necromancer]);
    hand(&mut g, P0, &[Card::ZombieSpy]);
    deck(&mut g, P0, &[Card::Copper, Card::Estate]);
    g.start();
    play(&mut g, Card::ZombieSpy);
    assert_eq!(hand_kinds(&g, P0), [Card::Copper]);
    yes(&mut g);
    assert_eq!(discard_kinds(&g, P0), [Card::Estate]);
    assert_eq!(g.turn().actions, 1);
}
