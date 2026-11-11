use super::*;
use rand::seq::{IteratorRandom, SliceRandom};

fn turn_starts(g: &Game) -> usize {
    g.log()
        .iter()
        .filter(|e| matches!(e, Event::TurnStart { .. }))
        .count()
}

/// Ends phases (and resolves triggers in the order offered) until the next turn starts.
fn next_turn(g: &mut Game) {
    let n = turn_starts(g);
    while turn_starts(g) == n {
        match &g.pending().expect("a decision is pending").kind {
            DecisionKind::Turn { .. } => end_phase(g),
            _ if purpose(g) == Purpose::Trigger => respond(g, Response::option(Opt::Trigger(0))),
            k => panic!("unexpected decision {k:?}"),
        }
    }
}

/// Ends the Action phase if it is still on (it is skipped when nothing can be played).
fn to_buy(g: &mut Game) {
    if let Some(DecisionKind::Turn {
        phase: Phase::Action,
        ..
    }) = g.pending().map(|d| &d.kind)
    {
        end_phase(g);
    }
}

/// Asks the pending turn decision again, after a test changed the turn's state.
fn reask(g: &mut Game) {
    g.pending = None;
    g.run(crate::frame::Input::Start);
}

fn resolve_triggers(g: &mut Game) {
    while g.pending().is_some_and(|d| d.purpose == Purpose::Trigger) {
        respond(g, Response::option(Opt::Trigger(0)));
    }
}

fn in_play_kinds(g: &Game, p: PlayerId) -> Vec<Card> {
    kinds(g, g.in_play(p))
}

fn discard_kinds(g: &Game, p: PlayerId) -> Vec<Card> {
    kinds(g, &g.player(p).discard)
}

fn modes(g: &mut Game, ms: &[u8]) {
    respond(
        g,
        Response::Options(ms.iter().map(|&m| Opt::Mode(m)).collect()),
    );
}

fn moves(g: &Game) -> Vec<Move> {
    match &g.pending().expect("a decision is pending").kind {
        DecisionKind::Turn { moves, .. } => moves.clone(),
        k => panic!("expected a Turn decision, got {k:?}"),
    }
}

fn buy_landscape(g: &mut Game, card: Card) {
    respond(g, Response::Move(Move::BuyLandscape(card)));
}

/// A 2-player game with `landscapes`; a Prophecy among them gets `suns` Sun tokens.
fn game(kingdom: &[Card], landscapes: &[Card], suns: u32) -> Game {
    let mut config = GameConfig {
        colonies: Some(false),
        shelters: Some(false),
        ..GameConfig::new(2, kingdom.to_vec(), 7)
    };
    config.landscapes = landscapes.to_vec();
    let mut g = setup_with(config);
    if let Some((p, _)) = g.prophecy {
        g.prophecy = Some((p, suns));
    }
    g
}

/// A game with `prophecy` already in effect.
fn with_prophecy(kingdom: &[Card], prophecy: Card) -> Game {
    game(kingdom, &[prophecy], 0)
}

fn coppers(n: usize) -> Vec<Card> {
    vec![Card::Copper; n]
}

// ---- Omens, Sun tokens, Prophecy setup ----

#[test]
fn sun_tokens_depend_on_the_number_of_players() {
    for (players, suns) in [(2, 5), (3, 8), (4, 10), (5, 12), (6, 13)] {
        let config = GameConfig::new(players, vec![Card::TeaHouse], 1);
        let g = setup_with(config);
        let (prophecy, n) = g.prophecy().expect("an Omen deals a Prophecy");
        assert_eq!(prophecy.kind(), Kind::Prophecy);
        assert_eq!(n, suns);
        assert!(g.has_landscape(prophecy));
    }
    assert_eq!(setup(&[Card::Alley]).prophecy(), None);
}

#[test]
fn an_omen_removes_a_sun_and_the_last_one_makes_the_prophecy_happen() {
    let mut g = game(&[Card::TeaHouse], &[Card::GreatLeader], 2);
    hand(
        &mut g,
        P0,
        &[Card::TeaHouse, Card::TeaHouse, Card::TeaHouse],
    );
    deck(&mut g, P0, &coppers(10));
    g.start();
    play(&mut g, Card::TeaHouse);
    assert_eq!(g.prophecy(), Some((Card::GreatLeader, 1)));
    assert_eq!((g.turn().actions, g.turn().coins), (1, 2));
    assert_eq!(g.hand(P0).len(), 3);
    play(&mut g, Card::TeaHouse);
    // Great Leader happened while Tea House resolved, so it gives Tea House its Action.
    assert_eq!(g.prophecy(), Some((Card::GreatLeader, 0)));
    assert_eq!(g.turn().actions, 2);
    play(&mut g, Card::TeaHouse);
    assert_eq!(g.prophecy(), Some((Card::GreatLeader, 0)));
    assert_eq!((g.turn().actions, g.turn().coins), (3, 6));
}

#[test]
fn throne_room_on_an_omen_removes_two_suns() {
    let mut g = game(&[Card::Poet, Card::ThroneRoom], &[Card::Panic], 5);
    hand(&mut g, P0, &[Card::ThroneRoom, Card::Poet]);
    deck(&mut g, P0, &coppers(10));
    g.start();
    play(&mut g, Card::ThroneRoom);
    choose_kinds(&mut g, &[Card::Poet]);
    assert_eq!(g.prophecy(), Some((Card::Panic, 3)));
}

#[test]
fn a_random_kingdom_with_an_omen_gets_one_prophecy() {
    for seed in 0..40 {
        let config = GameConfig::random(2, &[Set::RisingSun], seed);
        let omen = config.kingdom.iter().any(|c| c.is(Types::OMEN));
        let g = setup_with(config);
        assert_eq!(g.prophecy().is_some(), omen, "seed {seed}");
        let landscapes = g.landscapes().iter().filter(|l| l.kind() == Kind::Prophecy);
        assert_eq!(landscapes.count(), usize::from(omen));
    }
}

// ---- Omens ----

#[test]
fn kitsune_does_two_different_options_in_order_and_moat_stops_the_curse() {
    let mut g = setup(&[Card::Kitsune, Card::Moat]);
    hand(&mut g, P0, &[Card::Kitsune]);
    hand(&mut g, P1, &[Card::Moat]);
    g.start();
    play(&mut g, Card::Kitsune);
    assert_eq!(purpose(&g), Purpose::Trigger);
    respond(&mut g, Response::option(Opt::Trigger(0)));
    modes(&mut g, &[2, 3]);
    assert_eq!(g.count_owned(P1, Card::Curse), 0);
    assert_eq!(discard_kinds(&g, P0), [Card::Silver]);

    let mut g = setup(&[Card::Kitsune]);
    hand(&mut g, P0, &[Card::Kitsune]);
    g.start();
    play(&mut g, Card::Kitsune);
    modes(&mut g, &[0, 1]);
    assert_eq!((g.turn().coins, g.turn().actions), (2, 2));

    let mut g = setup(&[Card::Kitsune]);
    hand(&mut g, P0, &[Card::Kitsune]);
    g.start();
    play(&mut g, Card::Kitsune);
    modes(&mut g, &[1, 2]);
    assert_eq!(g.count_owned(P1, Card::Curse), 1);
    assert_eq!(g.turn().actions, 2);
}

#[test]
fn mountain_shrine_draws_two_when_an_action_is_in_the_trash() {
    let mut g = setup(&[Card::MountainShrine]);
    hand(
        &mut g,
        P0,
        &[Card::MountainShrine, Card::Smithy, Card::Estate],
    );
    deck(&mut g, P0, &coppers(5));
    g.start();
    play(&mut g, Card::MountainShrine);
    assert_eq!(g.turn().coins, 2);
    choose_kinds(&mut g, &[Card::Smithy]);
    assert_eq!(
        hand_kinds(&g, P0),
        [Card::Estate, Card::Copper, Card::Copper]
    );

    let mut g = setup(&[Card::MountainShrine]);
    hand(&mut g, P0, &[Card::MountainShrine, Card::Estate]);
    deck(&mut g, P0, &coppers(5));
    g.start();
    play(&mut g, Card::MountainShrine);
    choose(&mut g, &[]);
    assert_eq!(hand_kinds(&g, P0), [Card::Estate]);
}

#[test]
fn poet_takes_the_top_card_if_it_costs_three_or_less() {
    let mut g = setup(&[Card::Poet, Card::MountainShrine]);
    hand(&mut g, P0, &[Card::Poet]);
    deck(
        &mut g,
        P0,
        &[Card::Copper, Card::Silver, Card::MountainShrine],
    );
    g.start();
    play(&mut g, Card::Poet);
    assert_eq!(hand_kinds(&g, P0), [Card::Copper, Card::Silver]);

    let mut g = setup(&[Card::Poet, Card::MountainShrine]);
    hand(&mut g, P0, &[Card::Poet]);
    deck(&mut g, P0, &[Card::Copper, Card::MountainShrine]);
    g.start();
    play(&mut g, Card::Poet);
    assert_eq!(hand_kinds(&g, P0), [Card::Copper]);
    assert_eq!(deck_kinds(&g, P0), [Card::MountainShrine]);
}

#[test]
fn river_shrine_gains_in_cleanup_unless_a_card_was_gained_in_the_buy_phase() {
    let mut g = setup(&[Card::RiverShrine]);
    hand(
        &mut g,
        P0,
        &[Card::RiverShrine, Card::Estate, Card::Estate, Card::Copper],
    );
    deck(&mut g, P0, &coppers(10));
    g.start();
    play(&mut g, Card::RiverShrine);
    choose_kinds(&mut g, &[Card::Estate, Card::Estate]);
    assert_eq!(g.count_owned(P0, Card::Estate), 0);
    to_buy(&mut g);
    end_phase(&mut g);
    assert_eq!(purpose(&g), Purpose::Gain);
    choose_pile(&mut g, Card::Silver);
    assert!(discard_kinds(&g, P0).contains(&Card::Silver));

    let mut g = setup(&[Card::RiverShrine]);
    hand(&mut g, P0, &[Card::RiverShrine, Card::Copper]);
    g.start();
    play(&mut g, Card::RiverShrine);
    choose(&mut g, &[]);
    to_buy(&mut g);
    play(&mut g, Card::Copper);
    buy(&mut g, Card::Copper);
    assert_eq!(g.pending().unwrap().player, P1);
    assert_eq!(g.count_owned(P0, Card::Silver), 0);
}

#[test]
fn rustic_village_may_discard_two_cards_for_a_card() {
    let mut g = setup(&[Card::RusticVillage]);
    hand(&mut g, P0, &[Card::RusticVillage, Card::Estate]);
    deck(&mut g, P0, &[Card::Curse, Card::Silver]);
    g.start();
    play(&mut g, Card::RusticVillage);
    assert_eq!(g.turn().actions, 2);
    yes(&mut g);
    assert_eq!(hand_kinds(&g, P0), [Card::Silver]);
    assert_eq!(discard_kinds(&g, P0), [Card::Estate, Card::Curse]);
}

// ---- Shadow ----

#[test]
fn shadow_cards_go_to_the_bottom_when_shuffled_and_play_from_the_deck() {
    let mut g = setup(&[Card::Alley]);
    hand(&mut g, P0, &[Card::Copper; 5]);
    deck(&mut g, P0, &[]);
    let mut cards = vec![Card::Estate; 6];
    cards.extend([Card::Alley, Card::Ninja]);
    discard(&mut g, P0, &cards);
    shuffle(&mut g, P0);
    choose_kinds(&mut g, &[Card::Alley, Card::Ninja]);
    let bottom: Vec<Card> = deck_kinds(&g, P0).into_iter().rev().take(2).collect();
    assert!(bottom.iter().all(|c| c.is(Types::SHADOW)));
    g.start();
    let alley = g
        .player(P0)
        .deck
        .iter()
        .copied()
        .find(|&c| g.card(c) == Card::Alley)
        .unwrap();
    assert!(moves(&g).contains(&Move::Play(alley)));
    assert_eq!(g.view(P0).card(alley), Some(Card::Alley));
    respond(&mut g, Response::Move(Move::Play(alley)));
    assert_eq!(g.zone(alley), Zone::InPlay(P0));
    assert_eq!(g.turn().actions, 1);
    // Alley discards from the hand, not a Shadow in the deck.
    assert!(candidates(&g).iter().all(|&c| g.zone(c) == Zone::Hand(P0)));
}

#[test]
fn a_gained_shadow_is_not_put_on_the_bottom() {
    let mut g = setup(&[Card::Ronin]);
    hand(&mut g, P0, &[Card::Gold, Card::Gold]);
    g.start();
    to_buy(&mut g);
    play(&mut g, Card::Gold);
    play(&mut g, Card::Gold);
    buy(&mut g, Card::Ronin);
    assert!(discard_kinds(&g, P0).contains(&Card::Ronin));
}

#[test]
fn throne_room_can_play_a_shadow_card_from_the_deck_twice() {
    let mut g = setup(&[Card::ThroneRoom, Card::Fishmonger]);
    hand(&mut g, P0, &[Card::ThroneRoom]);
    deck(&mut g, P0, &[Card::Copper, Card::Fishmonger]);
    g.start();
    play(&mut g, Card::ThroneRoom);
    choose_kinds(&mut g, &[Card::Fishmonger]);
    assert_eq!(in_play_kinds(&g, P0), [Card::ThroneRoom, Card::Fishmonger]);
    assert_eq!((g.turn().buys, g.turn().coins), (3, 2));
}

#[test]
fn ninja_draws_and_makes_the_others_discard_down_to_three() {
    let mut g = setup(&[Card::Ninja]);
    hand(&mut g, P0, &[Card::Ninja]);
    deck(&mut g, P0, &[Card::Silver]);
    hand(&mut g, P1, &coppers(5));
    g.start();
    play(&mut g, Card::Ninja);
    assert_eq!(hand_kinds(&g, P0), [Card::Silver]);
    assert_eq!(g.hand(P1).len(), 3);
}

#[test]
fn ronin_draws_until_seven_cards_in_hand() {
    let mut g = setup(&[Card::Ronin]);
    hand(&mut g, P0, &[Card::Ronin, Card::Estate]);
    deck(&mut g, P0, &coppers(10));
    g.start();
    play(&mut g, Card::Ronin);
    assert_eq!(g.hand(P0).len(), 7);

    let mut g = setup(&[Card::Ronin]);
    let mut cards = vec![Card::Ronin];
    cards.extend(coppers(8));
    hand(&mut g, P0, &cards);
    deck(&mut g, P0, &coppers(3));
    g.start();
    play(&mut g, Card::Ronin);
    assert_eq!(g.hand(P0).len(), 8);
}

#[test]
fn tanuki_trashes_and_gains_a_card_costing_up_to_two_more() {
    let mut g = setup(&[Card::Tanuki]);
    hand(&mut g, P0, &[Card::Tanuki, Card::Silver]);
    g.start();
    play(&mut g, Card::Tanuki);
    let piles = match &g.pending().unwrap().kind {
        DecisionKind::Pile { piles, .. } => piles.clone(),
        k => panic!("{k:?}"),
    };
    assert!(piles.contains(&g.supply_pile(Card::Tanuki).unwrap()));
    assert!(!piles.contains(&g.supply_pile(Card::Gold).unwrap()));
    choose_pile(&mut g, Card::Tanuki);
    assert_eq!(discard_kinds(&g, P0), [Card::Tanuki]);
}

#[test]
fn fishmonger_from_the_deck_gives_a_buy_and_a_coin() {
    let mut g = setup(&[Card::Fishmonger]);
    hand(&mut g, P0, &[]);
    let ids = deck(&mut g, P0, &[Card::Copper, Card::Fishmonger]);
    g.start();
    respond(&mut g, Response::Move(Move::Play(ids[1])));
    assert_eq!((g.turn().buys, g.turn().coins), (2, 1));
}

// ---- Other Kingdom cards ----

#[test]
fn aristocrat_depends_on_the_aristocrats_in_play() {
    let mut g = setup(&[Card::Aristocrat]);
    hand(&mut g, P0, &[Card::Aristocrat; 4]);
    deck(&mut g, P0, &coppers(10));
    g.start();
    play(&mut g, Card::Aristocrat);
    assert_eq!(g.turn().actions, 3);
    play(&mut g, Card::Aristocrat);
    assert_eq!(g.hand(P0).len(), 5);
    play(&mut g, Card::Aristocrat);
    assert_eq!(g.turn().coins, 3);
    play(&mut g, Card::Aristocrat);
    assert_eq!(g.turn().buys, 4);
}

#[test]
fn artist_draws_a_card_per_card_with_exactly_one_copy_in_play() {
    let mut g = setup(&[Card::Artist, Card::Village]);
    hand(
        &mut g,
        P0,
        &[Card::Village, Card::Village, Card::RootCellar, Card::Artist],
    );
    deck(&mut g, P0, &coppers(10));
    g.start();
    play(&mut g, Card::Village);
    play(&mut g, Card::Village);
    play(&mut g, Card::RootCellar);
    let before = g.hand(P0).len();
    play(&mut g, Card::Artist);
    // Root Cellar and Artist.
    assert_eq!(g.hand(P0).len(), before - 1 + 2);
}

#[test]
fn change_gives_three_with_debt_and_otherwise_remodels_for_debt() {
    let mut g = setup(&[Card::Change]);
    hand(&mut g, P0, &[Card::Change, Card::Copper]);
    g.start();
    play(&mut g, Card::Change);
    choose_pile(&mut g, Card::Gold);
    assert_eq!(g.player(P0).debt, 6);
    assert!(discard_kinds(&g, P0).contains(&Card::Gold));

    let mut g = setup(&[Card::Change]);
    hand(&mut g, P0, &[Card::Change, Card::Copper]);
    g.players[0].debt = 1;
    g.start();
    play(&mut g, Card::Change);
    assert_eq!(g.turn().coins, 3);
    assert_eq!(g.hand(P0).len(), 1);
}

#[test]
fn debt_can_be_paid_in_the_action_phase() {
    let mut g = setup(&[Card::Change, Card::TeaHouse]);
    hand(&mut g, P0, &[Card::TeaHouse, Card::Change, Card::Copper]);
    deck(&mut g, P0, &[Card::Estate]);
    g.players[0].debt = 2;
    g.prophecy = None;
    g.start();
    play(&mut g, Card::TeaHouse);
    respond(&mut g, Response::Move(Move::PayDebt));
    respond(&mut g, Response::Move(Move::PayDebt));
    assert_eq!((g.player(P0).debt, g.turn().coins), (0, 0));
    play(&mut g, Card::Change);
    assert_eq!(purpose(&g), Purpose::Trash);
}

#[test]
fn craftsman_takes_debt_and_gains_a_card_even_with_debt() {
    let mut g = setup(&[Card::Craftsman]);
    hand(&mut g, P0, &[Card::Craftsman]);
    g.players[0].debt = 3;
    g.start();
    play(&mut g, Card::Craftsman);
    choose_pile(&mut g, Card::Duchy);
    assert_eq!(g.player(P0).debt, 5);
    assert_eq!(g.count_owned(P0, Card::Duchy), 1);
}

#[test]
fn root_cellar_imperial_envoy_and_litter_take_debt() {
    let mut g = setup(&[Card::RootCellar, Card::ImperialEnvoy, Card::Litter]);
    hand(
        &mut g,
        P0,
        &[Card::Litter, Card::RootCellar, Card::ImperialEnvoy],
    );
    deck(&mut g, P0, &coppers(15));
    g.start();
    play(&mut g, Card::Litter);
    play(&mut g, Card::RootCellar);
    play(&mut g, Card::ImperialEnvoy);
    assert_eq!(g.player(P0).debt, 6);
    assert_eq!(g.hand(P0).len(), 2 + 3 + 5);
    assert_eq!((g.turn().actions, g.turn().buys), (1, 2));
}

#[test]
fn gold_mine_may_gain_a_gold_for_four_debt() {
    let mut g = setup(&[Card::GoldMine]);
    hand(&mut g, P0, &[Card::GoldMine, Card::GoldMine]);
    deck(&mut g, P0, &coppers(5));
    g.start();
    play(&mut g, Card::GoldMine);
    yes(&mut g);
    assert_eq!(g.player(P0).debt, 4);
    assert_eq!(discard_kinds(&g, P0), [Card::Gold]);
    play(&mut g, Card::GoldMine);
    no(&mut g);
    assert_eq!(g.player(P0).debt, 4);
    assert_eq!((g.turn().actions, g.turn().buys), (1, 3));
}

#[test]
fn daimyo_replays_the_next_non_command_action() {
    let mut g = setup(&[Card::Daimyo, Card::Craftsman]);
    hand(&mut g, P0, &[Card::Daimyo, Card::Daimyo, Card::Craftsman]);
    deck(&mut g, P0, &coppers(10));
    g.start();
    play(&mut g, Card::Daimyo);
    play(&mut g, Card::Daimyo);
    play(&mut g, Card::Craftsman);
    for _ in 0..3 {
        resolve_triggers(&mut g);
        choose_pile(&mut g, Card::Silver);
    }
    assert_eq!(g.count_owned(P0, Card::Silver), 3);
    assert_eq!(g.player(P0).debt, 6);
    assert!(g.lasting().iter().all(|l| l.card != Card::Daimyo));
}

#[test]
fn daimyo_replays_a_throne_room_after_it_finishes() {
    let mut g = setup(&[Card::Daimyo, Card::ThroneRoom, Card::Smithy]);
    hand(
        &mut g,
        P0,
        &[Card::Daimyo, Card::ThroneRoom, Card::Smithy, Card::Smithy],
    );
    deck(&mut g, P0, &coppers(20));
    g.start();
    play(&mut g, Card::Daimyo);
    play(&mut g, Card::ThroneRoom);
    choose_kinds(&mut g, &[Card::Smithy]);
    // The replayed Throne Room plays the other Smithy twice.
    choose_kinds(&mut g, &[Card::Smithy]);
    assert_eq!(g.hand(P0).len(), 1 + 12);
    assert_eq!(g.turn().played.len(), 7);
}

#[test]
fn rice_counts_the_different_types_among_cards_in_play() {
    let mut g = setup(&[Card::Rice, Card::Daimyo, Card::Fishmonger]);
    hand(
        &mut g,
        P0,
        &[Card::Daimyo, Card::Fishmonger, Card::Copper, Card::Rice],
    );
    deck(&mut g, P0, &[Card::Estate]);
    g.start();
    play(&mut g, Card::Daimyo);
    play(&mut g, Card::Fishmonger);
    resolve_triggers(&mut g);
    to_buy(&mut g);
    play(&mut g, Card::Copper);
    play(&mut g, Card::Rice);
    // Action, Command, Shadow, Treasure.
    assert_eq!(g.turn().coins, 2 + 1 + 4);
    assert_eq!(g.turn().buys, 4);
}

#[test]
fn rice_broker_draws_two_for_a_treasure_and_five_for_an_action() {
    let mut g = setup(&[Card::RiceBroker]);
    hand(&mut g, P0, &[Card::RiceBroker, Card::Copper, Card::Smithy]);
    deck(&mut g, P0, &coppers(10));
    g.start();
    play(&mut g, Card::RiceBroker);
    choose_kinds(&mut g, &[Card::Copper]);
    assert_eq!(g.hand(P0).len(), 3);
    let mut g2 = setup(&[Card::RiceBroker]);
    hand(&mut g2, P0, &[Card::RiceBroker, Card::Copper, Card::Smithy]);
    deck(&mut g2, P0, &coppers(10));
    g2.start();
    play(&mut g2, Card::RiceBroker);
    choose_kinds(&mut g2, &[Card::Smithy]);
    assert_eq!(g2.hand(P0).len(), 6);
}

#[test]
fn riverboat_sets_aside_a_five_cost_action_and_plays_it_next_turn_leaving_it_there() {
    let mut g = setup(&[Card::Riverboat]);
    let set_aside = g.ext.rising_sun.riverboat.expect("a card was set aside");
    let card = g.card(set_aside);
    assert!(card.is(Types::ACTION) && !card.is(Types::DURATION));
    assert_eq!(card.cost(), Some(Cost::coins(5)));
    assert_eq!(g.pile_of(card), None);
    // Make it a Laboratory for the test.
    g.cards[set_aside.index()] = Card::Laboratory;
    hand(&mut g, P0, &[Card::Riverboat]);
    deck(&mut g, P0, &coppers(20));
    g.start();
    play(&mut g, Card::Riverboat);
    next_turn(&mut g);
    next_turn(&mut g);
    assert_eq!(g.hand(P0).len(), 7);
    assert_eq!(g.turn().actions, 2);
    assert!(matches!(g.zone(set_aside), Zone::Pile(_)));
    assert_eq!(in_play_kinds(&g, P0), [Card::Riverboat]);
    next_turn(&mut g);
    assert!(g.in_play(P0).is_empty());
}

#[test]
fn riverboats_card_gets_the_setup_it_needs() {
    for seed in 0..200 {
        let config = GameConfig {
            colonies: Some(false),
            shelters: Some(false),
            ..GameConfig::new(2, vec![Card::Riverboat], seed)
        };
        let g = setup_with(config);
        let id = g.ext.rising_sun.riverboat.unwrap();
        if matches!(g.card(id), Card::BanditCamp | Card::Pillage) {
            assert!(g.pile_of(Card::Spoils).is_some());
            return;
        }
    }
    panic!("no seed set aside Bandit Camp or Pillage");
}

#[test]
fn samurai_attacks_once_and_gives_a_coin_at_the_start_of_every_turn() {
    let mut g = setup(&[Card::Samurai]);
    hand(&mut g, P0, &[Card::Samurai]);
    deck(&mut g, P0, &coppers(30));
    hand(&mut g, P1, &coppers(5));
    deck(&mut g, P1, &coppers(30));
    g.start();
    play(&mut g, Card::Samurai);
    assert_eq!(g.hand(P1).len(), 3);
    for _ in 0..3 {
        next_turn(&mut g);
        assert_eq!(g.pending().unwrap().player, P1);
        next_turn(&mut g);
        assert_eq!(g.turn().coins, 1);
        assert_eq!(in_play_kinds(&g, P0), [Card::Samurai]);
    }
}

#[test]
fn snake_witch_returns_to_its_pile_to_curse_when_the_hand_has_no_duplicates() {
    let mut g = setup(&[Card::SnakeWitch]);
    hand(&mut g, P0, &[Card::SnakeWitch, Card::Estate]);
    deck(&mut g, P0, &[Card::Copper]);
    g.start();
    play(&mut g, Card::SnakeWitch);
    yes(&mut g);
    assert_eq!(g.count_owned(P1, Card::Curse), 1);
    assert_eq!(g.count_owned(P0, Card::SnakeWitch), 0);

    let mut g = setup(&[Card::SnakeWitch]);
    hand(&mut g, P0, &[Card::SnakeWitch, Card::Copper]);
    deck(&mut g, P0, &[Card::Copper]);
    g.start();
    play(&mut g, Card::SnakeWitch);
    assert_eq!(g.count_owned(P1, Card::Curse), 0);
    assert_eq!(in_play_kinds(&g, P0), [Card::SnakeWitch]);
}

#[test]
fn a_throne_roomed_snake_witch_curses_only_once() {
    let mut g = setup(&[Card::SnakeWitch, Card::ThroneRoom]);
    hand(&mut g, P0, &[Card::ThroneRoom, Card::SnakeWitch]);
    deck(&mut g, P0, &[Card::Copper, Card::Estate]);
    g.start();
    play(&mut g, Card::ThroneRoom);
    choose_kinds(&mut g, &[Card::SnakeWitch]);
    yes(&mut g);
    yes(&mut g);
    assert_eq!(g.count_owned(P1, Card::Curse), 1);
}

// ---- Events ----

fn buy_phase_with(g: &mut Game, coins: u32) {
    to_buy(g);
    g.turn.coins = coins;
    g.turn.buys = 5;
    reask(g);
}

#[test]
fn amass_gains_an_action_only_with_no_action_in_play() {
    let mut g = game(&[Card::Smithy, Card::Village], &[Card::Amass], 0);
    g.start();
    buy_phase_with(&mut g, 4);
    g.turn.buys = 2;
    buy_landscape(&mut g, Card::Amass);
    choose_pile(&mut g, Card::Smithy);
    assert!(discard_kinds(&g, P0).contains(&Card::Smithy));

    let mut g = game(&[Card::Smithy, Card::Village], &[Card::Amass], 0);
    hand(&mut g, P0, &[Card::Village]);
    g.start();
    play(&mut g, Card::Village);
    buy_phase_with(&mut g, 2);
    buy_landscape(&mut g, Card::Amass);
    assert_eq!(g.count_owned(P0, Card::Smithy), 0);
}

#[test]
fn asceticism_pays_any_amount_to_trash_that_many_cards() {
    let mut g = game(&[Card::Smithy], &[Card::Asceticism], 0);
    hand(&mut g, P0, &[Card::Estate, Card::Estate, Card::Curse]);
    g.start();
    buy_phase_with(&mut g, 6);
    buy_landscape(&mut g, Card::Asceticism);
    respond(&mut g, Response::Number(2));
    choose_kinds(&mut g, &[Card::Estate, Card::Curse]);
    assert_eq!(g.turn().coins, 2);
    assert_eq!(hand_kinds(&g, P0), [Card::Estate]);
}

#[test]
fn continue_returns_to_the_action_phase_and_plays_the_card_gained() {
    let mut g = game(&[Card::Village, Card::Militia], &[Card::Continue], 0);
    hand(&mut g, P0, &[Card::Copper, Card::Smithy]);
    deck(&mut g, P0, &coppers(10));
    g.start();
    end_phase(&mut g);
    play(&mut g, Card::Copper);
    // Village is the only non-Attack Action up to $4, so it is gained without asking.
    buy_landscape(&mut g, Card::Continue);
    assert_eq!(g.count_owned(P0, Card::Militia), 0);
    assert_eq!(g.turn().phase, Phase::Action);
    assert_eq!(in_play_kinds(&g, P0), [Card::Copper, Card::Village]);
    assert_eq!((g.turn().actions, g.turn().buys), (4, 1));
    // Debt can be paid now; Treasures can be played again in the next Buy phase.
    respond(&mut g, Response::Move(Move::PayDebt));
    assert_eq!(g.player(P0).debt, 7);
    play(&mut g, Card::Smithy);
    to_buy(&mut g);
    assert_eq!(g.turn().phase, Phase::Buy);
    assert!(moves(&g).iter().any(|m| matches!(m, Move::Play(_))));
    g.players[0].debt = 0;
    reask(&mut g);
    assert!(!moves(&g).contains(&Move::BuyLandscape(Card::Continue)));
}

#[test]
fn credit_gains_an_action_or_treasure_and_takes_debt_equal_to_its_cost() {
    let mut g = game(&[Card::Smithy], &[Card::Credit], 0);
    g.start();
    buy_phase_with(&mut g, 2);
    buy_landscape(&mut g, Card::Credit);
    let piles = match &g.pending().unwrap().kind {
        DecisionKind::Pile { piles, .. } => piles.clone(),
        k => panic!("{k:?}"),
    };
    assert!(!piles.contains(&g.supply_pile(Card::Province).unwrap()));
    choose_pile(&mut g, Card::Gold);
    assert_eq!(g.player(P0).debt, 6);
}

#[test]
fn foresight_sets_aside_the_first_action_and_puts_it_into_the_next_hand() {
    let mut g = game(&[Card::Smithy], &[Card::Foresight], 0);
    hand(&mut g, P0, &[]);
    deck(
        &mut g,
        P0,
        &[
            Card::Copper,
            Card::Estate,
            Card::Smithy,
            Card::Silver,
            Card::Silver,
            Card::Silver,
            Card::Silver,
            Card::Silver,
        ],
    );
    g.start();
    buy_phase_with(&mut g, 2);
    buy_landscape(&mut g, Card::Foresight);
    assert_eq!(discard_kinds(&g, P0), [Card::Copper, Card::Estate]);
    next_turn(&mut g);
    assert_eq!(g.player(P0).hand.len(), 6);
    assert!(hand_kinds(&g, P0).contains(&Card::Smithy));
}

#[test]
fn gather_gains_cards_costing_exactly_three_four_and_five() {
    let mut g = game(&[Card::Smithy, Card::Laboratory], &[Card::Gather], 0);
    g.start();
    buy_phase_with(&mut g, 7);
    buy_landscape(&mut g, Card::Gather);
    // Silver and Smithy are the only cards costing exactly $3 and $4.
    choose_pile(&mut g, Card::Laboratory);
    assert_eq!(
        discard_kinds(&g, P0),
        [Card::Silver, Card::Smithy, Card::Laboratory]
    );
}

#[test]
fn kintsugi_gains_only_once_a_gold_has_been_gained() {
    let mut g = game(&[Card::Smithy], &[Card::Kintsugi], 0);
    hand(&mut g, P0, &[Card::Estate, Card::Estate]);
    g.start();
    buy_phase_with(&mut g, 6);
    g.turn.buys = 3;
    buy_landscape(&mut g, Card::Kintsugi);
    assert_eq!(g.hand(P0).len(), 1);
    assert_eq!(g.player(P0).discard.len(), 0);
    buy(&mut g, Card::Copper);
    let gold = g.supply_pile(Card::Gold).unwrap();
    g.stack
        .push(crate::frame::Frame::gain(P0, gold, Dest::Discard));
    g.run(crate::frame::Input::Start);
    buy_landscape(&mut g, Card::Kintsugi);
    choose_pile(&mut g, Card::Smithy);
    assert!(discard_kinds(&g, P0).contains(&Card::Smithy));
}

#[test]
fn practice_plays_an_action_twice() {
    let mut g = game(&[Card::Smithy], &[Card::Practice], 0);
    hand(&mut g, P0, &[Card::Smithy]);
    deck(&mut g, P0, &coppers(10));
    g.start();
    buy_phase_with(&mut g, 3);
    buy_landscape(&mut g, Card::Practice);
    choose_kinds(&mut g, &[Card::Smithy]);
    assert_eq!(g.hand(P0).len(), 6);
}

#[test]
fn receive_tribute_needs_three_gains_and_gains_different_actions_not_in_play() {
    let mut g = game(
        &[Card::Smithy, Card::Village, Card::Market],
        &[Card::ReceiveTribute],
        0,
    );
    hand(&mut g, P0, &[Card::Village]);
    deck(&mut g, P0, &coppers(10));
    g.start();
    play(&mut g, Card::Village);
    buy_phase_with(&mut g, 5);
    g.turn.buys = 5;
    buy_landscape(&mut g, Card::ReceiveTribute);
    assert_eq!(g.player(P0).discard.len(), 0);
    buy(&mut g, Card::Copper);
    buy(&mut g, Card::Copper);
    buy(&mut g, Card::Copper);
    g.turn.coins = 5;
    reask(&mut g);
    buy_landscape(&mut g, Card::ReceiveTribute);
    let piles = match &g.pending().unwrap().kind {
        DecisionKind::Pile { piles, .. } => piles.clone(),
        k => panic!("{k:?}"),
    };
    assert!(!piles.contains(&g.supply_pile(Card::Village).unwrap()));
    choose_pile(&mut g, Card::Smithy);
    choose_pile(&mut g, Card::Market);
    assert_eq!(g.count_owned(P0, Card::Smithy), 1);
    assert_eq!(g.count_owned(P0, Card::Market), 1);
}

#[test]
fn sea_trade_draws_a_card_per_action_in_play_and_trashes_up_to_that_many() {
    let mut g = game(&[Card::Village], &[Card::SeaTrade], 0);
    hand(&mut g, P0, &[Card::Village, Card::Village, Card::Curse]);
    deck(&mut g, P0, &coppers(10));
    g.start();
    play(&mut g, Card::Village);
    play(&mut g, Card::Village);
    buy_phase_with(&mut g, 4);
    buy_landscape(&mut g, Card::SeaTrade);
    assert_eq!(g.hand(P0).len(), 5);
    choose_kinds(&mut g, &[Card::Curse, Card::Copper]);
    assert_eq!(g.hand(P0).len(), 3);
}

// ---- Prophecies ----

#[test]
fn approaching_army_adds_an_attack_pile_and_gives_a_coin_after_each_attack() {
    let g = game(&[Card::TeaHouse], &[Card::ApproachingArmy], 5);
    let attacks = g
        .piles()
        .iter()
        .filter(|p| p.in_supply && p.card.is(Types::ATTACK))
        .count();
    assert_eq!(attacks, 1);

    let mut g = with_prophecy(&[Card::Militia], Card::ApproachingArmy);
    hand(&mut g, P0, &[Card::Militia]);
    hand(&mut g, P1, &coppers(3));
    g.start();
    play(&mut g, Card::Militia);
    assert_eq!(g.turn().coins, 3);
}

#[test]
fn biding_time_sets_the_hand_aside_until_the_next_turn() {
    let mut g = with_prophecy(&[Card::Smithy], Card::BidingTime);
    hand(&mut g, P0, &[Card::Gold, Card::Estate]);
    deck(&mut g, P0, &coppers(20));
    g.start();
    next_turn(&mut g);
    assert_eq!(
        kinds(&g, &g.player(P0).set_aside),
        [Card::Gold, Card::Estate]
    );
    assert_eq!(g.view(P1).card(g.player(P0).set_aside[0]), None);
    next_turn(&mut g);
    assert_eq!(g.hand(P0).len(), 7);
    assert!(g.player(P0).set_aside.is_empty());
}

#[test]
fn bureaucracy_gains_a_copper_with_every_card_that_does_not_cost_zero() {
    let mut g = with_prophecy(&[Card::Smithy, Card::Daimyo], Card::Bureaucracy);
    hand(&mut g, P0, &[Card::Gold, Card::Gold]);
    g.start();
    to_buy(&mut g);
    g.turn.buys = 3;
    play(&mut g, Card::Gold);
    play(&mut g, Card::Gold);
    buy(&mut g, Card::Smithy);
    buy(&mut g, Card::Copper);
    buy(&mut g, Card::Daimyo);
    let coppers = g
        .log()
        .iter()
        .filter(|e| {
            matches!(
                e,
                Event::Gained {
                    player: P0,
                    card: Card::Copper,
                    ..
                }
            )
        })
        .count();
    assert_eq!(coppers, 3);
}

#[test]
fn divine_wind_replaces_the_kingdom_piles_with_ten_new_ones() {
    let kingdom = [Card::TeaHouse, Card::Smithy, Card::Village];
    let mut g = game(&kingdom, &[Card::DivineWind], 1);
    hand(&mut g, P0, &[Card::TeaHouse]);
    deck(&mut g, P0, &coppers(5));
    g.start();
    play(&mut g, Card::TeaHouse);
    for k in kingdom {
        assert_eq!(g.supply_pile(k), None);
        assert_eq!(g.pile_of(k), None);
    }
    let kingdom_piles = g
        .piles()
        .iter()
        .filter(|p| p.in_supply && p.card.def().kingdom)
        .count();
    assert_eq!(kingdom_piles, 10);
    assert!(g.supply_pile(Card::Province).is_some());
    check_zones(&g);
}

#[test]
fn enlightenment_makes_treasures_actions_that_draw_in_the_action_phase() {
    let mut g = with_prophecy(&[Card::RiceBroker], Card::Enlightenment);
    hand(&mut g, P0, &[Card::Gold, Card::RiceBroker, Card::Copper]);
    deck(&mut g, P0, &coppers(20));
    g.start();
    let gold = find(&g, P0, Card::Gold);
    assert!(g.is(gold, Types::ACTION));
    play(&mut g, Card::Gold);
    assert_eq!((g.turn().coins, g.turn().actions), (0, 1));
    assert_eq!(g.hand(P0).len(), 3);
    play(&mut g, Card::RiceBroker);
    assert_eq!(g.hand(P0).len(), 1 + 7);
    to_buy(&mut g);
    play(&mut g, Card::Copper);
    assert_eq!(g.turn().coins, 1);
}

#[test]
fn flourishing_trade_lowers_costs_and_lets_actions_be_used_as_buys() {
    let mut g = with_prophecy(&[Card::Smithy, Card::Daimyo], Card::FlourishingTrade);
    hand(&mut g, P0, &[Card::Silver, Card::Silver, Card::Silver]);
    g.start();
    assert_eq!(g.cost(Card::Smithy), Cost::coins(3));
    assert_eq!(
        g.cost(Card::Daimyo),
        Cost {
            debt: 6,
            ..Cost::ZERO
        }
    );
    to_buy(&mut g);
    for _ in 0..3 {
        play(&mut g, Card::Silver);
    }
    buy(&mut g, Card::Smithy);
    buy(&mut g, Card::Smithy);
    assert_eq!(g.count_owned(P0, Card::Smithy), 2);
}

#[test]
fn good_harvest_gives_a_buy_and_a_coin_for_the_first_play_of_each_treasure() {
    let mut g = with_prophecy(&[Card::Smithy], Card::GoodHarvest);
    hand(
        &mut g,
        P0,
        &[Card::Copper, Card::Copper, Card::Copper, Card::Silver],
    );
    g.start();
    to_buy(&mut g);
    for c in [Card::Copper, Card::Copper, Card::Copper, Card::Silver] {
        play(&mut g, c);
    }
    assert_eq!((g.turn().coins, g.turn().buys), (5 + 2, 3));
}

#[test]
fn growth_gains_a_cheaper_card_when_a_treasure_is_gained_and_chains() {
    let mut g = with_prophecy(&[Card::Smithy], Card::Growth);
    hand(&mut g, P0, &[Card::Gold, Card::Gold]);
    g.start();
    to_buy(&mut g);
    play(&mut g, Card::Gold);
    play(&mut g, Card::Gold);
    buy(&mut g, Card::Gold);
    choose_pile(&mut g, Card::Silver);
    choose_pile(&mut g, Card::Estate);
    assert_eq!(
        discard_kinds(&g, P0)[..3],
        [Card::Gold, Card::Silver, Card::Estate]
    );
}

#[test]
fn harsh_winter_puts_debt_on_a_pile_and_the_next_gain_from_it_takes_it() {
    let mut g = with_prophecy(&[Card::Smithy], Card::HarshWinter);
    hand(&mut g, P0, &[Card::Gold, Card::Gold]);
    g.start();
    to_buy(&mut g);
    g.turn.buys = 2;
    play(&mut g, Card::Gold);
    play(&mut g, Card::Gold);
    buy(&mut g, Card::Copper);
    let copper = g.supply_pile(Card::Copper).unwrap();
    assert_eq!(g.pile(copper).debt_tokens, 2);
    buy(&mut g, Card::Copper);
    assert_eq!(g.pile(copper).debt_tokens, 0);
    assert_eq!(g.player(P0).debt, 2);
}

#[test]
fn kind_emperor_gains_an_action_to_hand_when_it_happens_and_each_turn() {
    let mut g = game(&[Card::TeaHouse, Card::Smithy], &[Card::KindEmperor], 1);
    hand(&mut g, P0, &[Card::TeaHouse]);
    deck(&mut g, P0, &coppers(20));
    deck(&mut g, P1, &coppers(20));
    g.start();
    play(&mut g, Card::TeaHouse);
    assert_eq!(purpose(&g), Purpose::Gain);
    choose_pile(&mut g, Card::Smithy);
    assert!(hand_kinds(&g, P0).contains(&Card::Smithy));
    to_buy(&mut g);
    end_phase(&mut g);
    assert_eq!(g.pending().unwrap().player, P1);
    assert_eq!(purpose(&g), Purpose::Gain);
    choose_pile(&mut g, Card::Smithy);
    assert!(hand_kinds(&g, P1).contains(&Card::Smithy));
}

#[test]
fn panic_gives_buys_for_treasures_and_returns_them_to_their_piles() {
    let mut g = with_prophecy(&[Card::Smithy], Card::Panic);
    hand(&mut g, P0, &[Card::Copper, Card::Silver]);
    deck(&mut g, P0, &[Card::Estate; 5]);
    g.start();
    to_buy(&mut g);
    play(&mut g, Card::Copper);
    play(&mut g, Card::Silver);
    assert_eq!(g.turn().buys, 5);
    let silvers = g.pile(g.supply_pile(Card::Silver).unwrap()).cards.len();
    next_turn(&mut g);
    assert_eq!(
        g.pile(g.supply_pile(Card::Silver).unwrap()).cards.len(),
        silvers + 1
    );
    assert_eq!(g.count_owned(P0, Card::Silver), 0);
    assert_eq!(g.count_owned(P0, Card::Copper), 0);
}

#[test]
fn progress_puts_every_gained_card_onto_the_deck() {
    let mut g = with_prophecy(&[Card::Smithy, Card::Witch], Card::Progress);
    hand(&mut g, P0, &[Card::Witch]);
    deck(&mut g, P0, &coppers(5));
    g.start();
    play(&mut g, Card::Witch);
    assert_eq!(deck_kinds(&g, P1)[0], Card::Curse);
    to_buy(&mut g);
    g.turn.coins = 4;
    g.turn.buys = 2;
    reask(&mut g);
    buy(&mut g, Card::Smithy);
    assert_eq!(deck_kinds(&g, P0)[0], Card::Smithy);
}

#[test]
fn rapid_expansion_sets_aside_gained_actions_and_treasures_to_play_next_turn() {
    let mut g = with_prophecy(&[Card::Smithy], Card::RapidExpansion);
    hand(&mut g, P0, &[Card::Gold, Card::Gold]);
    deck(&mut g, P0, &coppers(20));
    g.start();
    to_buy(&mut g);
    g.turn.buys = 2;
    play(&mut g, Card::Gold);
    play(&mut g, Card::Gold);
    buy(&mut g, Card::Smithy);
    buy(&mut g, Card::Estate);
    assert_eq!(kinds(&g, &g.player(P0).set_aside), [Card::Smithy]);
    assert_eq!(discard_kinds(&g, P0)[0], Card::Estate);
    next_turn(&mut g);
    assert_eq!(in_play_kinds(&g, P0), [Card::Smithy]);
    assert_eq!(g.hand(P0).len(), 8);
}

#[test]
fn sickness_asks_for_a_curse_onto_the_deck_or_three_discards() {
    let mut g = with_prophecy(&[Card::Smithy], Card::Sickness);
    hand(&mut g, P0, &coppers(5));
    g.start();
    modes(&mut g, &[0]);
    assert_eq!(deck_kinds(&g, P0)[0], Card::Curse);
    next_turn(&mut g);
    modes(&mut g, &[1]);
    let three = g.hand(P1)[..3].to_vec();
    choose(&mut g, &three);
    assert_eq!(g.hand(P1).len(), 2);
}

// ---- Fuzz ----

#[test]
fn random_rising_sun_games_keep_invariants() {
    let pool: Vec<Card> = Card::in_set(Set::RisingSun)
        .filter(|c| c.def().kingdom)
        .collect();
    let events: Vec<Card> = Card::in_set(Set::RisingSun)
        .filter(|c| c.kind() == Kind::Event)
        .collect();
    let prophecies: Vec<Card> = Card::in_set(Set::RisingSun)
        .filter(|c| c.kind() == Kind::Prophecy)
        .collect();
    let mut seen = std::collections::HashSet::new();
    for seed in 0..fuzz_seeds(600) {
        let mut rng = ChaCha8Rng::seed_from_u64(seed);
        let mut cards = pool.clone();
        cards.shuffle(&mut rng);
        cards.truncate(8);
        cards.extend([Card::ThroneRoom, Card::Moat]);
        // Every Event comes round in turn, plus one at random.
        let rotated = events[seed as usize % events.len()];
        let mut landscapes = vec![rotated];
        landscapes.extend(
            events
                .iter()
                .copied()
                .filter(|&e| e != rotated)
                .choose(&mut rng),
        );
        landscapes.push(prophecies[seed as usize % prophecies.len()]);
        let players = rng.random_range(2..=4);
        let mut config = GameConfig::new(players, cards.clone(), seed);
        config.landscapes = landscapes;
        config.turn_limit = Some(60);
        let mut g = setup_with(config);
        if let Some((p, _)) = g.prophecy {
            g.prophecy = Some((p, rng.random_range(0..3)));
        }
        for p in 0..players {
            let p = PlayerId(p);
            for &c in cards.iter().chain(&[Card::Gold; 3]) {
                g.create(c, Zone::Deck(p));
            }
            let mut deck = std::mem::take(&mut g.players[p.index()].deck);
            g.shuffle(&mut deck);
            g.players[p.index()].deck = deck;
        }
        g.start();
        let g = drive_randomly(g, seed);
        for e in g.log() {
            match e {
                Event::Played { card, .. } | Event::BoughtLandscape { card, .. } => {
                    seen.insert(*card);
                }
                _ => {}
            }
        }
        if let Some((p, 0)) = g.prophecy() {
            seen.insert(p);
        }
    }
    let missing: Vec<Card> = Card::in_set(Set::RisingSun)
        .filter(|c| !seen.contains(c))
        .collect();
    assert!(missing.is_empty(), "never played: {missing:?}");
}

#[test]
fn mountain_shrine_checks_the_trash_after_a_fortress_has_left_it() {
    let mut g = setup(&[Card::MountainShrine, Card::Fortress]);
    hand(
        &mut g,
        P0,
        &[Card::MountainShrine, Card::Fortress, Card::Estate],
    );
    deck(&mut g, P0, &coppers(5));
    g.start();
    play(&mut g, Card::MountainShrine);
    choose_kinds(&mut g, &[Card::Fortress]);
    resolve_triggers(&mut g);
    assert_eq!(hand_kinds(&g, P0), [Card::Estate, Card::Fortress]);
}

#[test]
fn divine_wind_sets_up_the_piles_it_deals() {
    let mut dealt_looter = false;
    for seed in 0..60 {
        let config = GameConfig {
            colonies: Some(false),
            shelters: Some(false),
            landscapes: vec![Card::DivineWind],
            ..GameConfig::new(2, vec![Card::TeaHouse], seed)
        };
        let mut g = setup_with(config);
        g.prophecy = Some((Card::DivineWind, 1));
        hand(&mut g, P0, &[Card::TeaHouse]);
        deck(&mut g, P0, &coppers(5));
        g.start();
        play(&mut g, Card::TeaHouse);
        let looter = g
            .piles()
            .iter()
            .any(|p| p.in_supply && p.card.is(Types::LOOTER));
        if looter {
            assert!(g.supply_pile(Card::Ruins).is_some(), "seed {seed}");
            dealt_looter = true;
        }
        if g.supply_pile(Card::Riverboat).is_some() {
            assert!(g.ext.rising_sun.riverboat.is_some(), "seed {seed}");
        }
        check_zones(&g);
    }
    assert!(dealt_looter);
}

#[test]
fn riverboats_baker_gives_everyone_a_coffers() {
    for seed in 0..400 {
        let config = GameConfig {
            colonies: Some(false),
            shelters: Some(false),
            ..GameConfig::new(2, vec![Card::Riverboat], seed)
        };
        let g = setup_with(config);
        if g.card(g.ext.rising_sun.riverboat.unwrap()) == Card::Baker {
            assert_eq!(g.player(P0).coffers, 1);
            assert_eq!(g.player(P1).coffers, 1);
            return;
        }
    }
    panic!("no seed set aside Baker");
}

/// Shuffles `p`'s discard pile into their deck, outside any turn.
fn shuffle(g: &mut Game, p: PlayerId) {
    g.stack.push(crate::frame::Frame::shuffle(p));
    g.run(crate::frame::Input::Start);
}

#[test]
fn the_player_orders_shadow_cards_put_on_the_bottom_by_a_shuffle() {
    let mut g = setup(&[Card::Alley, Card::Ninja]);
    deck(&mut g, P0, &[]);
    discard(&mut g, P0, &[Card::Alley, Card::Ninja, Card::Copper]);
    shuffle(&mut g, P0);
    match &g.pending().unwrap().kind {
        DecisionKind::Cards { ordered, .. } => assert!(ordered),
        k => panic!("expected an order, got {k:?}"),
    }
    choose_kinds(&mut g, &[Card::Ninja, Card::Alley]);
    assert_eq!(deck_kinds(&g, P0), [Card::Copper, Card::Ninja, Card::Alley]);

    discard(&mut g, P0, &[Card::Alley, Card::Ninja, Card::Copper]);
    deck(&mut g, P0, &[]);
    shuffle(&mut g, P0);
    choose_kinds(&mut g, &[Card::Alley, Card::Ninja]);
    assert_eq!(deck_kinds(&g, P0), [Card::Copper, Card::Alley, Card::Ninja]);
}

#[test]
fn identical_shadow_cards_need_no_order() {
    let mut g = setup(&[Card::Ninja]);
    deck(&mut g, P0, &[]);
    discard(&mut g, P0, &[Card::Ninja, Card::Copper, Card::Ninja]);
    shuffle(&mut g, P0);
    assert!(g.pending().is_none());
    assert_eq!(deck_kinds(&g, P0), [Card::Copper, Card::Ninja, Card::Ninja]);
}

#[test]
fn fishmonger_can_be_played_from_the_deck_in_the_buy_phase_with_capitalism() {
    let mut g = game(&[Card::Fishmonger], &[Card::Capitalism], 0);
    g.players[0].projects.push(Card::Capitalism);
    hand(&mut g, P0, &[Card::Copper]);
    let ids = deck(&mut g, P0, &[Card::Estate, Card::Fishmonger]);
    g.start();
    to_buy(&mut g);
    assert!(moves(&g).contains(&Move::Play(ids[1])));
    respond(&mut g, Response::Move(Move::Play(ids[1])));
    assert_eq!((g.turn().coins, g.turn().buys), (1, 2));
}
