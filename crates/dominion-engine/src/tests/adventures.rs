use super::*;
use crate::game::{Mat, Token};

/// An unstarted 2-player game with `events` beside the Supply.
fn game(kingdom: &[Card], events: &[Card]) -> Game {
    setup_with(GameConfig {
        colonies: Some(false),
        shelters: Some(false),
        landscapes: events.to_vec(),
        ..GameConfig::new(2, kingdom.to_vec(), 7)
    })
}

fn on_tavern(g: &mut Game, p: PlayerId, card: Card) -> CardId {
    g.create(card, Zone::Mat(p, Mat::Tavern))
}

fn tavern_kinds(g: &Game, p: PlayerId) -> Vec<Card> {
    kinds(g, g.player(p).mat(Mat::Tavern))
}

fn discard_kinds(g: &Game, p: PlayerId) -> Vec<Card> {
    kinds(g, &g.player(p).discard)
}

/// Resolves the pending trigger named after `card`.
fn accept(g: &mut Game, card: Card) {
    let d = g.pending().expect("a decision is pending");
    let DecisionKind::Choose {
        options, labels, ..
    } = &d.kind
    else {
        panic!("expected a trigger choice, got {d:?}");
    };
    let i = labels
        .iter()
        .position(|l| l.starts_with(card.name()))
        .unwrap_or_else(|| panic!("no {card} trigger in {labels:?}"));
    let opt = options[i];
    respond(g, Response::option(opt));
}

fn mode(g: &mut Game, i: u8) {
    respond(g, Response::option(Opt::Mode(i)));
}

fn buy_event(g: &mut Game, event: Card) {
    respond(g, Response::Move(Move::BuyLandscape(event)));
}

/// Ends phases until the turn passes to someone else.
fn finish_turn(g: &mut Game) {
    let p = g.turn().player;
    while g.turn().player == p {
        match &g.pending().expect("a decision is pending").kind {
            DecisionKind::Turn { .. } => end_phase(g),
            k => panic!("unexpected decision {k:?}"),
        }
    }
}

fn place(g: &mut Game, p: PlayerId, token: Token, pile: PileId) {
    g.players[p.index()].tokens.insert(token, Some(pile));
    g.piles[pile.index()].tokens.push((p, token));
}

fn moves(g: &Game) -> Vec<Move> {
    match &g.pending().expect("a decision is pending").kind {
        DecisionKind::Turn { moves, .. } => moves.clone(),
        k => panic!("expected a Turn decision, got {k:?}"),
    }
}

/// Random games with Adventures kingdoms, Events, Throne Room and Moat, every deck seeded
/// with each Kingdom card, some Travellers and money so that everything gets played and
/// bought.
fn fuzz(seeds: std::ops::Range<u64>) {
    let events: Vec<Card> = Card::in_set(Set::Adventures)
        .filter(|c| c.kind() == Kind::Event)
        .collect();
    for seed in seeds {
        let mut rng = ChaCha8Rng::seed_from_u64(seed);
        let players = rng.random_range(2..=4);
        let mut config = GameConfig::random(players, &[Set::Adventures], seed);
        config.landscapes = events.choose_multiple(&mut rng, 4).copied().collect();
        for (i, traveller) in [Card::Page, Card::Peasant].into_iter().enumerate() {
            if !config.kingdom.contains(&traveller) && rng.random_bool(0.5) {
                config.kingdom[i] = traveller;
            }
        }
        config.kingdom.truncate(8);
        config.kingdom.extend([Card::ThroneRoom, Card::Moat]);
        config.turn_limit = Some(60);
        let mut g = setup_with(config.clone());
        for p in 0..players {
            let p = PlayerId(p);
            let mut extra = config.kingdom.clone();
            extra.extend([Card::Silver, Card::Gold, Card::Gold, Card::Gold, Card::Gold]);
            if rng.random_bool(0.3) {
                extra.extend([Card::Champion, Card::Teacher, Card::Hireling]);
            }
            for card in extra {
                g.create(card, Zone::Discard(p));
            }
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
    }
}

#[test]
fn random_adventures_games_keep_invariants() {
    fuzz(0..fuzz_seeds(60));
}

// ---- Reserve cards ----

#[test]
fn coin_of_the_realm_goes_to_the_tavern_mat_and_is_called_for_two_actions() {
    let mut g = game(&[Card::CoinOfTheRealm, Card::Smithy], &[]);
    hand(&mut g, P0, &[Card::Smithy, Card::Smithy]);
    deck(
        &mut g,
        P0,
        &[Card::CoinOfTheRealm, Card::Copper, Card::Copper],
    );
    g.start();
    play(&mut g, Card::Smithy);
    // Drew the Coin of the Realm; it isn't on the mat yet, so nothing to call.
    assert_eq!(g.turn().actions, 0);
    play(&mut g, Card::CoinOfTheRealm);
    assert_eq!(g.turn().coins, 1);
    assert_eq!(tavern_kinds(&g, P0), [Card::CoinOfTheRealm]);
    finish_turn(&mut g);
    // It stays on the mat through Clean-up.
    assert_eq!(tavern_kinds(&g, P0), [Card::CoinOfTheRealm]);
}

#[test]
fn coin_of_the_realm_is_called_after_an_action() {
    let mut g = game(&[Card::CoinOfTheRealm, Card::Smithy], &[]);
    on_tavern(&mut g, P0, Card::CoinOfTheRealm);
    hand(&mut g, P0, &[Card::Smithy, Card::Smithy]);
    deck(&mut g, P0, &[Card::Copper; 8]);
    g.start();
    play(&mut g, Card::Smithy);
    assert_eq!(purpose(&g), Purpose::Trigger);
    accept(&mut g, Card::CoinOfTheRealm);
    assert_eq!(g.turn().actions, 2);
    assert!(tavern_kinds(&g, P0).is_empty());
    assert_eq!(
        kinds(&g, g.in_play(P0)),
        [Card::Smithy, Card::CoinOfTheRealm]
    );
    play(&mut g, Card::Smithy);
    assert_eq!(g.hand(P0).len(), 6);
    finish_turn(&mut g);
    // Discarded with the other cards in play.
    assert!(tavern_kinds(&g, P0).is_empty());
    assert!(g.in_play(P0).is_empty());
}

#[test]
fn royal_carriage_replays_the_action_it_follows() {
    let mut g = game(&[Card::RoyalCarriage, Card::Smithy], &[]);
    on_tavern(&mut g, P0, Card::RoyalCarriage);
    hand(&mut g, P0, &[Card::Smithy]);
    deck(&mut g, P0, &[Card::Copper; 8]);
    g.start();
    play(&mut g, Card::Smithy);
    accept(&mut g, Card::RoyalCarriage);
    assert_eq!(g.hand(P0).len(), 6);
    assert_eq!(g.turn().played, [Card::Smithy, Card::Smithy]);
    assert_eq!(
        kinds(&g, g.in_play(P0)),
        [Card::Smithy, Card::RoyalCarriage]
    );
}

#[test]
fn royal_carriage_is_not_offered_for_a_raze_that_trashed_itself() {
    let mut g = game(&[Card::RoyalCarriage, Card::Raze], &[]);
    on_tavern(&mut g, P0, Card::RoyalCarriage);
    let raze = hand(&mut g, P0, &[Card::Raze])[0];
    deck(&mut g, P0, &[Card::Copper, Card::Silver, Card::Gold]);
    g.start();
    play(&mut g, Card::Raze);
    // The hand is empty, so Raze trashes itself; Raze costs $2: look at two.
    assert_eq!(kinds(&g, &candidates(&g)), [Card::Copper, Card::Silver]);
    choose_kinds(&mut g, &[Card::Silver]);
    assert_eq!(g.zone(raze), Zone::Trash);
    assert_eq!(hand_kinds(&g, P0), [Card::Silver]);
    assert_eq!(discard_kinds(&g, P0), [Card::Copper]);
    assert_eq!(tavern_kinds(&g, P0), [Card::RoyalCarriage]);
    assert_eq!(g.pending().unwrap().purpose, Purpose::Turn);
}

#[test]
fn a_royal_carriage_replaying_a_duration_stays_in_play_with_it() {
    let mut g = game(&[Card::RoyalCarriage, Card::Dungeon], &[]);
    on_tavern(&mut g, P0, Card::RoyalCarriage);
    hand(&mut g, P0, &[Card::Dungeon]);
    deck(&mut g, P0, &[Card::Copper; 12]);
    g.start();
    play(&mut g, Card::Dungeon);
    accept(&mut g, Card::RoyalCarriage);
    finish_turn(&mut g);
    assert_eq!(
        kinds(&g, g.in_play(P0)),
        [Card::Dungeon, Card::RoyalCarriage]
    );
}

#[test]
fn ratcatcher_guide_and_transmogrify_are_called_at_the_start_of_the_turn() {
    let mut g = game(&[Card::Ratcatcher, Card::Guide, Card::Transmogrify], &[]);
    on_tavern(&mut g, P0, Card::Ratcatcher);
    on_tavern(&mut g, P0, Card::Guide);
    on_tavern(&mut g, P0, Card::Transmogrify);
    hand(
        &mut g,
        P0,
        &[
            Card::Estate,
            Card::Estate,
            Card::Curse,
            Card::Copper,
            Card::Copper,
        ],
    );
    deck(&mut g, P0, &[Card::Silver; 5]);
    g.start();
    assert_eq!(purpose(&g), Purpose::Trigger);
    accept(&mut g, Card::Ratcatcher);
    choose_kinds(&mut g, &[Card::Curse]);
    accept(&mut g, Card::Transmogrify);
    choose_kinds(&mut g, &[Card::Estate]);
    // Up to $3: Silver, into the hand.
    choose_pile(&mut g, Card::Silver);
    assert_eq!(g.hand(P0).len(), 4);
    assert!(hand_kinds(&g, P0).contains(&Card::Silver));
    accept(&mut g, Card::Guide);
    assert_eq!(hand_kinds(&g, P0), [Card::Silver; 5]);
    assert_eq!(discard_kinds(&g, P0).len(), 4);
    assert_eq!(
        kinds(&g, g.in_play(P0)),
        [Card::Ratcatcher, Card::Transmogrify, Card::Guide]
    );
    assert!(tavern_kinds(&g, P0).is_empty());
    assert_eq!(g.turn().actions, 1);
}

#[test]
fn start_of_turn_calls_may_be_declined() {
    let mut g = game(&[Card::Ratcatcher], &[]);
    on_tavern(&mut g, P0, Card::Ratcatcher);
    g.start();
    no(&mut g);
    assert_eq!(tavern_kinds(&g, P0), [Card::Ratcatcher]);
}

#[test]
fn reserve_cards_go_to_the_mat_when_played() {
    let mut g = game(&[Card::Ratcatcher, Card::ThroneRoom], &[]);
    hand(&mut g, P0, &[Card::ThroneRoom, Card::Ratcatcher]);
    deck(&mut g, P0, &[Card::Copper; 5]);
    g.start();
    play(&mut g, Card::ThroneRoom);
    choose_kinds(&mut g, &[Card::Ratcatcher]);
    assert_eq!(tavern_kinds(&g, P0), [Card::Ratcatcher]);
    assert_eq!(g.hand(P0).len(), 2);
    assert_eq!(g.turn().actions, 2);
}

#[test]
fn transmogrify_with_an_empty_hand_gains_nothing() {
    let mut g = game(&[Card::Transmogrify], &[]);
    on_tavern(&mut g, P0, Card::Transmogrify);
    hand(&mut g, P0, &[]);
    g.start();
    accept(&mut g, Card::Transmogrify);
    assert_eq!(g.turn().gained, []);
    assert_eq!(kinds(&g, g.in_play(P0)), [Card::Transmogrify]);
}

#[test]
fn teacher_moves_a_bonus_token_to_a_pile_without_the_players_tokens() {
    let mut g = game(&[Card::Peasant, Card::Village, Card::Smithy], &[]);
    on_tavern(&mut g, P0, Card::Teacher);
    hand(&mut g, P0, &[Card::Village, Card::Village]);
    deck(&mut g, P0, &[Card::Copper; 8]);
    let smithy = g.supply_pile(Card::Smithy).unwrap();
    place(&mut g, P0, Token::MinusCost, smithy);
    g.start();
    accept(&mut g, Card::Teacher);
    mode(&mut g, 1);
    let DecisionKind::Pile { piles, .. } = &g.pending().unwrap().kind else {
        panic!("expected a pile choice");
    };
    assert!(!piles.contains(&smithy));
    assert!(piles.contains(&g.supply_pile(Card::Peasant).unwrap()));
    choose_pile(&mut g, Card::Village);
    play(&mut g, Card::Village);
    // +1 Action token, then +2 Actions.
    assert_eq!(g.turn().actions, 3);
    assert_eq!(kinds(&g, g.in_play(P0)), [Card::Teacher, Card::Village]);
}

#[test]
fn duplicate_gains_a_copy_of_a_card_costing_up_to_six() {
    let mut g = game(&[Card::Duplicate], &[]);
    on_tavern(&mut g, P0, Card::Duplicate);
    hand(&mut g, P0, &[Card::Gold, Card::Gold, Card::Gold]);
    g.start();
    play(&mut g, Card::Gold);
    play(&mut g, Card::Gold);
    buy(&mut g, Card::Gold);
    accept(&mut g, Card::Duplicate);
    assert_eq!(g.count_owned(P0, Card::Gold), 5);
    assert!(discard_kinds(&g, P0).contains(&Card::Duplicate));
}

#[test]
fn duplicate_is_not_offered_for_a_card_costing_more_than_six() {
    let mut g = game(&[Card::Duplicate], &[]);
    on_tavern(&mut g, P0, Card::Duplicate);
    hand(&mut g, P0, &[Card::Gold, Card::Gold, Card::Gold]);
    g.start();
    for _ in 0..3 {
        play(&mut g, Card::Gold);
    }
    buy(&mut g, Card::Province);
    assert_eq!(g.turn().player, P1);
    assert_eq!(tavern_kinds(&g, P0), [Card::Duplicate]);
}

#[test]
fn duplicate_works_on_another_players_turn() {
    let mut g = game(&[Card::Duplicate, Card::Witch], &[]);
    on_tavern(&mut g, P1, Card::Duplicate);
    hand(&mut g, P0, &[Card::Witch]);
    deck(&mut g, P0, &[Card::Copper; 5]);
    g.start();
    play(&mut g, Card::Witch);
    accept(&mut g, Card::Duplicate);
    assert_eq!(discard_kinds(&g, P1), [Card::Curse, Card::Curse]);
    assert_eq!(kinds(&g, g.in_play(P1)), [Card::Duplicate]);
    finish_turn(&mut g);
    // Discarded in the Clean-up of the turn it was called.
    finish_turn(&mut g);
    assert!(g.in_play(P1).is_empty());
}

#[test]
fn wine_merchant_is_discarded_from_the_mat_with_two_unspent() {
    let mut g = game(&[Card::WineMerchant], &[]);
    hand(&mut g, P0, &[Card::WineMerchant]);
    g.start();
    play(&mut g, Card::WineMerchant);
    assert_eq!((g.turn().coins, g.turn().buys), (4, 2));
    assert_eq!(tavern_kinds(&g, P0), [Card::WineMerchant]);
    end_phase(&mut g);
    accept(&mut g, Card::WineMerchant);
    assert!(tavern_kinds(&g, P0).is_empty());
    assert_eq!(discard_kinds(&g, P0), [Card::WineMerchant]);
}

#[test]
fn wine_merchant_stays_with_less_than_two_unspent() {
    let mut g = game(&[Card::WineMerchant], &[]);
    on_tavern(&mut g, P0, Card::WineMerchant);
    hand(&mut g, P0, &[Card::Silver, Card::Copper, Card::Copper]);
    g.start();
    play(&mut g, Card::Silver);
    play(&mut g, Card::Copper);
    play(&mut g, Card::Copper);
    buy(&mut g, Card::Silver);
    assert_eq!(g.turn().player, P1);
    assert_eq!(tavern_kinds(&g, P0), [Card::WineMerchant]);
}

#[test]
fn distant_lands_is_worth_four_only_on_the_tavern_mat() {
    let mut g = game(&[Card::DistantLands], &[]);
    hand(&mut g, P0, &[Card::DistantLands]);
    g.create(Card::DistantLands, Zone::Discard(P0));
    g.start();
    let before = g.score(P0);
    play(&mut g, Card::DistantLands);
    assert_eq!(tavern_kinds(&g, P0), [Card::DistantLands]);
    assert_eq!(g.score(P0), before + 4);
    assert!(Card::DistantLands.is(Types::VICTORY));
}

#[test]
fn guide_draws_a_fresh_hand() {
    let mut g = game(&[Card::Guide], &[]);
    hand(&mut g, P0, &[Card::Guide, Card::Estate]);
    deck(&mut g, P0, &[Card::Copper; 3]);
    g.start();
    play(&mut g, Card::Guide);
    assert_eq!(g.hand(P0).len(), 2);
    assert_eq!(tavern_kinds(&g, P0), [Card::Guide]);
}

// ---- Travellers ----

#[test]
fn page_is_exchanged_for_a_treasure_hunter_when_discarded_from_play() {
    let mut g = game(&[Card::Page], &[]);
    let page = hand(&mut g, P0, &[Card::Page])[0];
    deck(&mut g, P0, &[Card::Copper; 6]);
    g.start();
    play(&mut g, Card::Page);
    assert_eq!((g.hand(P0).len(), g.turn().actions), (1, 1));
    end_phase(&mut g);
    accept(&mut g, Card::Page);
    assert_eq!(g.zone(page), Zone::Pile(g.supply_pile(Card::Page).unwrap()));
    assert!(discard_kinds(&g, P0).contains(&Card::TreasureHunter));
    assert_eq!(
        g.pile(g.pile_of(Card::TreasureHunter).unwrap()).cards.len(),
        4
    );
    // Exchanging is not gaining.
    assert!(!g.log().iter().any(|e| matches!(e, Event::Gained { .. })));
}

#[test]
fn travellers_are_not_exchanged_from_the_hand_or_with_an_empty_pile() {
    let mut g = game(&[Card::Page, Card::Peasant], &[]);
    let soldiers = g.pile_of(Card::Soldier).unwrap();
    for c in g.pile(soldiers).cards.clone() {
        g.move_card(c, Zone::Trash, crate::game::Pos::Top);
    }
    hand(&mut g, P0, &[Card::Peasant, Card::Page]);
    g.start();
    play(&mut g, Card::Peasant);
    finish_turn(&mut g);
    assert_eq!(discard_kinds(&g, P0), [Card::Peasant, Card::Page]);
}

#[test]
fn the_upgrade_piles_are_set_up_out_of_the_supply() {
    let g = game(&[Card::Page, Card::Peasant, Card::Port], &[]);
    for card in [
        Card::TreasureHunter,
        Card::Warrior,
        Card::Hero,
        Card::Champion,
        Card::Soldier,
        Card::Fugitive,
        Card::Disciple,
        Card::Teacher,
    ] {
        let pile = g.pile(g.pile_of(card).unwrap());
        assert_eq!((pile.cards.len(), pile.in_supply), (5, false), "{card}");
    }
    assert_eq!(g.pile(g.supply_pile(Card::Port).unwrap()).cards.len(), 12);
    assert!(g.player(P0).journey_face_up);
    let g = game(&[Card::Smithy], &[]);
    assert!(g.pile_of(Card::Champion).is_none());
}

#[test]
fn treasure_hunter_gains_a_silver_per_card_the_right_player_gained_last_turn() {
    let mut g = game(&[Card::Page, Card::Peasant], &[]);
    hand(&mut g, P0, &[Card::Copper; 5]);
    deck(
        &mut g,
        P0,
        &[
            Card::TreasureHunter,
            Card::Copper,
            Card::Copper,
            Card::Copper,
            Card::Copper,
        ],
    );
    hand(
        &mut g,
        P1,
        &[
            Card::Peasant,
            Card::Copper,
            Card::Copper,
            Card::Copper,
            Card::Copper,
        ],
    );
    g.start();
    finish_turn(&mut g);
    play(&mut g, Card::Peasant);
    buy(&mut g, Card::Copper);
    buy(&mut g, Card::Copper);
    // Peasant's exchange.
    no(&mut g);
    assert_eq!(g.turn().player, P0);
    play(&mut g, Card::TreasureHunter);
    assert_eq!(g.turn().gained, [Card::Silver, Card::Silver]);
    assert_eq!((g.turn().actions, g.turn().coins), (1, 1));
}

#[test]
fn warrior_hits_once_per_traveller_in_play() {
    let mut g = game(&[Card::Page], &[]);
    hand(&mut g, P0, &[Card::Page, Card::Warrior]);
    deck(&mut g, P0, &[Card::Copper; 5]);
    deck(&mut g, P1, &[Card::Silver, Card::Estate, Card::Gold]);
    g.start();
    play(&mut g, Card::Page);
    play(&mut g, Card::Warrior);
    assert_eq!(g.hand(P0).len(), 3);
    assert_eq!(kinds(&g, g.trash()).last(), Some(&Card::Silver));
    assert_eq!(discard_kinds(&g, P1), [Card::Estate]);
    assert_eq!(deck_kinds(&g, P1), [Card::Gold]);
}

#[test]
fn warrior_trashes_only_cards_costing_three_or_four() {
    let mut g = game(&[Card::Page, Card::Smithy, Card::Village], &[]);
    hand(&mut g, P0, &[Card::Village, Card::Warrior, Card::Warrior]);
    deck(&mut g, P0, &[Card::Copper; 5]);
    deck(&mut g, P1, &[Card::Smithy, Card::Village, Card::Gold]);
    g.start();
    play(&mut g, Card::Village);
    play(&mut g, Card::Warrior);
    // Smithy costs $4: trashed.
    assert_eq!(kinds(&g, g.trash()).last(), Some(&Card::Smithy));
    play(&mut g, Card::Warrior);
    // Two Warriors in play: Village ($3) trashed, Gold discarded.
    assert_eq!(kinds(&g, g.trash()).last(), Some(&Card::Village));
    assert_eq!(discard_kinds(&g, P1), [Card::Gold]);
}

#[test]
fn hero_gains_a_treasure() {
    let mut g = game(&[Card::Page], &[]);
    hand(&mut g, P0, &[Card::Hero]);
    g.start();
    play(&mut g, Card::Hero);
    choose_pile(&mut g, Card::Gold);
    assert_eq!(discard_kinds(&g, P0), [Card::Gold]);
    assert_eq!(g.turn().coins, 2);
}

#[test]
fn champion_protects_from_later_attacks_and_gives_an_action_per_action() {
    let mut g = game(&[Card::Page, Card::Smithy, Card::Militia], &[]);
    hand(&mut g, P0, &[Card::Champion, Card::Smithy]);
    deck(&mut g, P0, &[Card::Copper; 10]);
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
    g.start();
    play(&mut g, Card::Champion);
    assert_eq!(g.turn().actions, 1);
    play(&mut g, Card::Smithy);
    assert_eq!(g.turn().actions, 1);
    finish_turn(&mut g);
    assert_eq!(kinds(&g, g.in_play(P0)), [Card::Champion]);
    play(&mut g, Card::Militia);
    assert_eq!(g.hand(P0).len(), 5);
    finish_turn(&mut g);
    finish_turn(&mut g);
    assert_eq!(kinds(&g, g.in_play(P0)), [Card::Champion]);
}

#[test]
fn soldier_gives_a_coin_per_other_attack_and_makes_big_hands_discard() {
    let mut g = game(&[Card::Peasant, Card::Village], &[]);
    hand(&mut g, P0, &[Card::Village, Card::Soldier, Card::Soldier]);
    deck(&mut g, P0, &[Card::Copper; 5]);
    g.start();
    play(&mut g, Card::Village);
    play(&mut g, Card::Soldier);
    assert_eq!(g.pending().unwrap().player, P1);
    let c = g.hand(P1)[0];
    choose(&mut g, &[c]);
    assert_eq!(g.turn().coins, 2);
    play(&mut g, Card::Soldier);
    let c = g.hand(P1)[0];
    choose(&mut g, &[c]);
    assert_eq!(g.turn().coins, 2 + 3);
    assert_eq!(g.hand(P1).len(), 3);
}

#[test]
fn fugitive_draws_two_and_discards_one() {
    let mut g = game(&[Card::Peasant], &[]);
    hand(&mut g, P0, &[Card::Fugitive, Card::Estate]);
    deck(&mut g, P0, &[Card::Copper, Card::Silver]);
    g.start();
    play(&mut g, Card::Fugitive);
    choose_kinds(&mut g, &[Card::Estate]);
    assert_eq!(hand_kinds(&g, P0), [Card::Copper, Card::Silver]);
    assert_eq!(discard_kinds(&g, P0), [Card::Estate]);
    assert_eq!(g.turn().actions, 1);
}

#[test]
fn disciple_plays_an_action_twice_and_gains_a_copy() {
    let mut g = game(&[Card::Peasant, Card::Village], &[]);
    hand(&mut g, P0, &[Card::Disciple, Card::Village]);
    deck(&mut g, P0, &[Card::Copper; 5]);
    g.start();
    play(&mut g, Card::Disciple);
    choose_kinds(&mut g, &[Card::Village]);
    assert_eq!(g.turn().actions, 4);
    assert_eq!(g.hand(P0).len(), 2);
    assert_eq!(discard_kinds(&g, P0), [Card::Village]);
}

#[test]
fn disciple_on_a_non_supply_card_gains_nothing() {
    let mut g = game(&[Card::Peasant], &[]);
    hand(&mut g, P0, &[Card::Disciple, Card::Fugitive, Card::Estate]);
    deck(&mut g, P0, &[Card::Copper; 5]);
    g.start();
    play(&mut g, Card::Disciple);
    choose_kinds(&mut g, &[Card::Fugitive]);
    choose_kinds(&mut g, &[Card::Estate]);
    assert_eq!(g.hand(P0).len(), 3);
    assert_eq!(g.turn().gained, []);
}

// ---- Durations ----

#[test]
fn amulet_chooses_now_and_at_the_start_of_the_next_turn() {
    let mut g = game(&[Card::Amulet], &[]);
    hand(&mut g, P0, &[Card::Amulet, Card::Estate]);
    g.start();
    play(&mut g, Card::Amulet);
    assert_eq!(purpose(&g), Purpose::Mode);
    mode(&mut g, 1);
    assert_eq!(kinds(&g, g.trash()).last(), Some(&Card::Estate));
    finish_turn(&mut g);
    assert_eq!(kinds(&g, g.in_play(P0)), [Card::Amulet]);
    finish_turn(&mut g);
    mode(&mut g, 2);
    assert_eq!(g.turn().gained, [Card::Silver]);
    finish_turn(&mut g);
    assert!(g.in_play(P0).is_empty());
}

#[test]
fn bridge_troll_cheapens_cards_this_turn_and_next_and_gives_a_buy_each_time() {
    let mut g = game(&[Card::BridgeTroll], &[]);
    hand(&mut g, P0, &[Card::BridgeTroll]);
    deck(&mut g, P0, &[Card::Copper; 10]);
    hand(&mut g, P1, &[Card::Copper; 5]);
    g.start();
    play(&mut g, Card::BridgeTroll);
    assert_eq!(g.turn().buys, 2);
    assert_eq!(g.cost(Card::Silver), Cost::coins(2));
    assert!(g.player(P1).minus_coin);
    finish_turn(&mut g);
    assert_eq!(g.cost(Card::Silver), Cost::coins(3));
    play(&mut g, Card::Copper);
    assert_eq!(g.turn().coins, 0);
    assert!(!g.player(P1).minus_coin);
    finish_turn(&mut g);
    assert_eq!(g.turn().buys, 2);
    assert_eq!(g.cost(Card::Silver), Cost::coins(2));
    // Events aren't cards.
    finish_turn(&mut g);
    assert!(g.in_play(P0).is_empty());
    assert_eq!(g.cost(Card::Silver), Cost::coins(3));
}

#[test]
fn a_throne_roomed_bridge_troll_cheapens_two_on_both_turns() {
    let mut g = game(&[Card::BridgeTroll, Card::ThroneRoom], &[]);
    hand(&mut g, P0, &[Card::ThroneRoom, Card::BridgeTroll]);
    deck(&mut g, P0, &[Card::Copper; 10]);
    g.start();
    play(&mut g, Card::ThroneRoom);
    choose_kinds(&mut g, &[Card::BridgeTroll]);
    assert_eq!((g.turn().buys, g.cost(Card::Silver)), (3, Cost::coins(1)));
    finish_turn(&mut g);
    finish_turn(&mut g);
    // Its two identical Duration effects resolve without asking for an order.
    assert_eq!((g.turn().buys, g.cost(Card::Silver)), (3, Cost::coins(1)));
    assert_eq!(
        kinds(&g, g.in_play(P0)),
        [Card::ThroneRoom, Card::BridgeTroll]
    );
}

#[test]
fn caravan_guard_draws_now_and_gives_a_coin_next_turn() {
    let mut g = game(&[Card::CaravanGuard], &[]);
    hand(&mut g, P0, &[Card::CaravanGuard]);
    deck(&mut g, P0, &[Card::Copper; 10]);
    g.start();
    play(&mut g, Card::CaravanGuard);
    assert_eq!((g.hand(P0).len(), g.turn().actions), (1, 1));
    finish_turn(&mut g);
    finish_turn(&mut g);
    assert_eq!(g.turn().coins, 1);
}

#[test]
fn caravan_guard_is_played_from_the_hand_against_an_attack() {
    let mut g = game(&[Card::CaravanGuard, Card::Militia], &[]);
    hand(&mut g, P0, &[Card::Militia]);
    deck(&mut g, P0, &[Card::Copper; 10]);
    hand(
        &mut g,
        P1,
        &[
            Card::CaravanGuard,
            Card::Copper,
            Card::Copper,
            Card::Estate,
            Card::Estate,
        ],
    );
    deck(&mut g, P1, &[Card::CaravanGuard, Card::Silver, Card::Gold]);
    g.start();
    play(&mut g, Card::Militia);
    accept(&mut g, Card::CaravanGuard);
    // It drew another Caravan Guard, which may be played too.
    accept(&mut g, Card::CaravanGuard);
    assert_eq!(
        kinds(&g, g.in_play(P1)),
        [Card::CaravanGuard, Card::CaravanGuard]
    );
    // The attacker gets none of the +Actions.
    assert_eq!(g.turn().actions, 0);
    choose_kinds(&mut g, &[Card::Estate, Card::Estate]);
    assert_eq!(g.hand(P1).len(), 3);
    finish_turn(&mut g);
    assert_eq!(g.turn().player, P1);
    assert_eq!(g.turn().coins, 2);
    finish_turn(&mut g);
    assert!(g.in_play(P1).is_empty());
}

#[test]
fn dungeon_draws_and_discards_now_and_next_turn() {
    let mut g = game(&[Card::Dungeon], &[]);
    hand(&mut g, P0, &[Card::Dungeon, Card::Estate]);
    deck(
        &mut g,
        P0,
        &[
            Card::Copper,
            Card::Silver,
            Card::Gold,
            Card::Copper,
            Card::Copper,
            Card::Copper,
            Card::Copper,
            Card::Copper,
            Card::Copper,
            Card::Copper,
        ],
    );
    g.start();
    play(&mut g, Card::Dungeon);
    assert_eq!(g.turn().actions, 1);
    choose_kinds(&mut g, &[Card::Estate, Card::Copper]);
    assert_eq!(hand_kinds(&g, P0), [Card::Silver]);
    finish_turn(&mut g);
    assert_eq!(kinds(&g, g.in_play(P0)), [Card::Dungeon]);
    finish_turn(&mut g);
    assert_eq!(purpose(&g), Purpose::Discard);
    assert_eq!(candidates(&g).len(), 7);
    let two = candidates(&g)[..2].to_vec();
    choose(&mut g, &two);
    assert_eq!(g.hand(P0).len(), 5);
}

#[test]
fn gear_sets_cards_aside_until_the_next_turn() {
    let mut g = game(&[Card::Gear], &[]);
    let kept = hand(&mut g, P0, &[Card::Gear, Card::Gold, Card::Estate]);
    deck(&mut g, P0, &[Card::Copper; 10]);
    g.start();
    play(&mut g, Card::Gear);
    choose(&mut g, &kept[1..]);
    assert_eq!(g.zone(kept[1]), Zone::SetAside(P0));
    finish_turn(&mut g);
    assert_eq!(kinds(&g, g.in_play(P0)), [Card::Gear]);
    finish_turn(&mut g);
    assert_eq!(g.hand(P0).len(), 7);
    assert_eq!(g.zone(kept[1]), Zone::Hand(P0));
}

#[test]
fn gear_setting_nothing_aside_is_discarded_the_same_turn() {
    let mut g = game(&[Card::Gear], &[]);
    hand(&mut g, P0, &[Card::Gear, Card::Gold]);
    deck(&mut g, P0, &[Card::Copper; 10]);
    g.start();
    play(&mut g, Card::Gear);
    choose(&mut g, &[]);
    finish_turn(&mut g);
    assert!(g.in_play(P0).is_empty());
}

#[test]
fn haunted_woods_topdecks_the_hand_of_a_buyer_and_draws_three_next_turn() {
    let mut g = game(&[Card::HauntedWoods], &[]);
    hand(&mut g, P0, &[Card::HauntedWoods]);
    deck(&mut g, P0, &[Card::Copper; 10]);
    hand(&mut g, P1, &[Card::Silver, Card::Estate, Card::Copper]);
    deck(&mut g, P1, &[Card::Gold; 4]);
    g.start();
    play(&mut g, Card::HauntedWoods);
    finish_turn(&mut g);
    play(&mut g, Card::Silver);
    buy(&mut g, Card::Copper);
    assert_eq!(
        (g.pending().unwrap().player, purpose(&g)),
        (P1, Purpose::TopDeck)
    );
    choose_kinds(&mut g, &[Card::Copper, Card::Estate]);
    assert_eq!(
        hand_kinds(&g, P1),
        [
            Card::Copper,
            Card::Estate,
            Card::Gold,
            Card::Gold,
            Card::Gold
        ]
    );
    assert_eq!(g.turn().player, P0);
    assert_eq!(g.hand(P0).len(), 8);
}

#[test]
fn haunted_woods_ignores_gains_that_were_not_bought() {
    let mut g = game(&[Card::HauntedWoods, Card::Workshop], &[]);
    hand(&mut g, P0, &[Card::HauntedWoods]);
    deck(&mut g, P0, &[Card::Copper; 10]);
    hand(&mut g, P1, &[Card::Workshop, Card::Estate, Card::Copper]);
    g.start();
    play(&mut g, Card::HauntedWoods);
    finish_turn(&mut g);
    play(&mut g, Card::Workshop);
    choose_pile(&mut g, Card::Silver);
    assert_eq!(hand_kinds(&g, P1), [Card::Estate, Card::Copper]);
}

#[test]
fn swamp_hag_curses_buyers_and_gives_three_coins_next_turn() {
    let mut g = game(&[Card::SwampHag, Card::ThroneRoom], &[]);
    hand(&mut g, P0, &[Card::ThroneRoom, Card::SwampHag]);
    deck(&mut g, P0, &[Card::Copper; 10]);
    hand(&mut g, P1, &[Card::Copper; 5]);
    g.start();
    play(&mut g, Card::ThroneRoom);
    choose_kinds(&mut g, &[Card::SwampHag]);
    finish_turn(&mut g);
    buy(&mut g, Card::Copper);
    // One Curse per Swamp Hag effect.
    assert_eq!(g.count_owned(P1, Card::Curse), 2);
    assert_eq!(g.turn().player, P0);
    assert_eq!(g.turn().coins, 6);
}

#[test]
fn moat_blocks_swamp_hag_only_when_it_is_played() {
    let mut g = game(&[Card::SwampHag, Card::Moat], &[]);
    hand(&mut g, P0, &[Card::SwampHag]);
    deck(&mut g, P0, &[Card::Copper; 10]);
    hand(&mut g, P1, &[Card::Moat, Card::Copper]);
    g.start();
    play(&mut g, Card::SwampHag);
    accept(&mut g, Card::Moat);
    finish_turn(&mut g);
    end_phase(&mut g);
    buy(&mut g, Card::Copper);
    assert_eq!(g.count_owned(P1, Card::Curse), 0);
}

#[test]
fn hireling_draws_a_card_every_turn_for_the_rest_of_the_game() {
    let mut g = game(&[Card::Hireling, Card::ThroneRoom], &[]);
    hand(&mut g, P0, &[Card::ThroneRoom, Card::Hireling]);
    deck(&mut g, P0, &[Card::Copper; 20]);
    g.start();
    play(&mut g, Card::ThroneRoom);
    choose_kinds(&mut g, &[Card::Hireling]);
    for _ in 0..3 {
        finish_turn(&mut g);
        finish_turn(&mut g);
        assert_eq!(g.hand(P0).len(), 7);
        assert_eq!(kinds(&g, g.in_play(P0)), [Card::ThroneRoom, Card::Hireling]);
    }
}

// ---- Other Kingdom cards ----

#[test]
fn artificer_gains_a_card_costing_exactly_one_per_card_discarded_onto_the_deck() {
    let mut g = game(&[Card::Artificer], &[]);
    hand(&mut g, P0, &[Card::Artificer, Card::Estate, Card::Curse]);
    deck(&mut g, P0, &[Card::Copper, Card::Gold]);
    g.start();
    play(&mut g, Card::Artificer);
    assert_eq!((g.turn().actions, g.turn().coins), (1, 1));
    choose_kinds(&mut g, &[Card::Estate, Card::Curse]);
    let DecisionKind::Pile { piles, optional } = &g.pending().unwrap().kind else {
        panic!("expected a pile choice");
    };
    assert!(*optional);
    assert_eq!(
        piles.iter().map(|&p| g.pile(p).card).collect::<Vec<_>>(),
        [Card::Estate]
    );
    choose_pile(&mut g, Card::Estate);
    assert_eq!(deck_kinds(&g, P0), [Card::Estate, Card::Gold]);
}

#[test]
fn giant_alternates_between_a_coin_and_a_big_attack() {
    let mut g = game(&[Card::Giant, Card::Village], &[]);
    hand(&mut g, P0, &[Card::Village, Card::Giant, Card::Giant]);
    deck(&mut g, P0, &[Card::Copper; 5]);
    deck(&mut g, P1, &[Card::Silver, Card::Copper]);
    g.start();
    play(&mut g, Card::Village);
    play(&mut g, Card::Giant);
    assert_eq!(g.turn().coins, 1);
    assert!(!g.player(P0).journey_face_up);
    play(&mut g, Card::Giant);
    assert_eq!(g.turn().coins, 6);
    assert_eq!(kinds(&g, g.trash()).last(), Some(&Card::Silver));
    assert_eq!(g.count_owned(P1, Card::Curse), 0);
}

#[test]
fn giant_discards_and_curses_outside_three_to_six() {
    let mut g = game(&[Card::Giant], &[]);
    hand(&mut g, P0, &[Card::Giant]);
    deck(&mut g, P0, &[Card::Copper; 5]);
    g.players[0].journey_face_up = false;
    deck(&mut g, P1, &[Card::Province]);
    g.start();
    play(&mut g, Card::Giant);
    assert_eq!(discard_kinds(&g, P1), [Card::Province, Card::Curse]);
}

#[test]
fn lost_city_makes_the_other_players_draw_when_gained() {
    let mut g = game(&[Card::LostCity], &[]);
    hand(&mut g, P0, &[Card::Gold, Card::Gold]);
    g.start();
    play(&mut g, Card::Gold);
    play(&mut g, Card::Gold);
    buy(&mut g, Card::LostCity);
    assert_eq!(g.hand(P1).len(), 6);
}

#[test]
fn magpie_takes_a_treasure_or_gains_a_magpie() {
    let mut g = game(&[Card::Magpie, Card::Village], &[]);
    hand(&mut g, P0, &[Card::Village, Card::Magpie, Card::Magpie]);
    deck(
        &mut g,
        P0,
        &[
            Card::Copper,
            Card::Copper,
            Card::Estate,
            Card::Copper,
            Card::Silver,
        ],
    );
    g.start();
    play(&mut g, Card::Village);
    play(&mut g, Card::Magpie);
    // Revealed the Estate: a Magpie, and the Estate stays on top.
    assert_eq!(g.turn().gained, [Card::Magpie]);
    assert_eq!(
        deck_kinds(&g, P0),
        [Card::Estate, Card::Copper, Card::Silver]
    );
    play(&mut g, Card::Magpie);
    // Drew the Estate and revealed a Copper, which goes into the hand.
    assert_eq!(g.turn().gained, [Card::Magpie]);
    assert_eq!(
        hand_kinds(&g, P0),
        [Card::Copper, Card::Copper, Card::Estate, Card::Copper]
    );
}

#[test]
fn messenger_may_put_the_deck_into_the_discard_pile() {
    let mut g = game(&[Card::Messenger], &[]);
    hand(&mut g, P0, &[Card::Messenger]);
    deck(&mut g, P0, &[Card::Copper, Card::Estate]);
    g.start();
    play(&mut g, Card::Messenger);
    yes(&mut g);
    assert!(g.player(P0).deck.is_empty());
    assert_eq!(discard_kinds(&g, P0).len(), 2);
    assert_eq!((g.turn().buys, g.turn().coins), (2, 2));
    // Not discarding: no Discard events.
    assert!(!g.log().iter().any(|e| matches!(e, Event::Discarded { .. })));
}

#[test]
fn messenger_bought_first_gains_a_card_for_everyone() {
    let mut g = game(&[Card::Messenger], &[]);
    hand(&mut g, P0, &[Card::Gold, Card::Gold, Card::Gold]);
    g.start();
    for _ in 0..3 {
        play(&mut g, Card::Gold);
    }
    g.turn.buys = 2;
    buy(&mut g, Card::Messenger);
    choose_pile(&mut g, Card::Silver);
    assert_eq!(g.turn().gained, [Card::Messenger, Card::Silver]);
    assert_eq!(discard_kinds(&g, P1), [Card::Silver]);
    buy(&mut g, Card::Messenger);
    assert_eq!(g.turn().player, P1);
    assert_eq!(g.count_owned(P0, Card::Messenger), 2);
    assert_eq!(g.count_owned(P0, Card::Silver), 1);
}

#[test]
fn miser_puts_coppers_on_the_mat_or_pays_for_them() {
    let mut g = game(&[Card::Miser, Card::Village], &[]);
    on_tavern(&mut g, P0, Card::Copper);
    hand(
        &mut g,
        P0,
        &[Card::Village, Card::Miser, Card::Miser, Card::Copper],
    );
    deck(&mut g, P0, &[Card::Estate; 5]);
    g.start();
    play(&mut g, Card::Village);
    play(&mut g, Card::Miser);
    mode(&mut g, 0);
    assert_eq!(tavern_kinds(&g, P0), [Card::Copper, Card::Copper]);
    play(&mut g, Card::Miser);
    mode(&mut g, 1);
    assert_eq!(g.turn().coins, 2);
    // Coppers on the mat are still owned.
    assert_eq!(g.count_owned(P0, Card::Copper), 2);
}

#[test]
fn port_comes_with_another_port() {
    let mut g = game(&[Card::Port, Card::Workshop], &[]);
    hand(&mut g, P0, &[Card::Workshop]);
    g.start();
    play(&mut g, Card::Workshop);
    choose_pile(&mut g, Card::Port);
    assert_eq!(g.turn().gained, [Card::Port, Card::Port]);
    assert_eq!(g.pile(g.supply_pile(Card::Port).unwrap()).cards.len(), 10);
}

#[test]
fn ranger_draws_five_every_other_play() {
    let mut g = game(&[Card::Ranger, Card::Village], &[]);
    hand(&mut g, P0, &[Card::Village, Card::Ranger, Card::Ranger]);
    deck(&mut g, P0, &[Card::Copper; 12]);
    g.start();
    play(&mut g, Card::Village);
    play(&mut g, Card::Ranger);
    assert_eq!(g.hand(P0).len(), 2);
    play(&mut g, Card::Ranger);
    assert_eq!(g.hand(P0).len(), 6);
    assert_eq!(g.turn().buys, 3);
}

#[test]
fn raze_trashes_a_card_and_looks_at_one_card_per_coin_it_cost() {
    let mut g = game(&[Card::Raze], &[]);
    hand(&mut g, P0, &[Card::Raze, Card::Estate]);
    deck(&mut g, P0, &[Card::Copper, Card::Gold, Card::Silver]);
    g.start();
    play(&mut g, Card::Raze);
    choose_kinds(&mut g, &[Card::Estate]);
    assert_eq!(kinds(&g, &candidates(&g)), [Card::Copper, Card::Gold]);
    choose_kinds(&mut g, &[Card::Gold]);
    assert_eq!(hand_kinds(&g, P0), [Card::Gold]);
    assert_eq!(discard_kinds(&g, P0), [Card::Copper]);
    assert_eq!(deck_kinds(&g, P0), [Card::Silver]);
    assert_eq!(kinds(&g, g.in_play(P0)), [Card::Raze]);
}

#[test]
fn raze_on_a_card_costing_zero_looks_at_nothing() {
    let mut g = game(&[Card::Raze], &[]);
    hand(&mut g, P0, &[Card::Raze, Card::Copper]);
    deck(&mut g, P0, &[Card::Gold]);
    g.start();
    play(&mut g, Card::Raze);
    choose_kinds(&mut g, &[Card::Copper]);
    assert_eq!(deck_kinds(&g, P0), [Card::Gold]);
    assert!(g.hand(P0).is_empty());
}

#[test]
fn relic_gives_the_other_players_their_minus_card_token() {
    let mut g = game(&[Card::Relic], &[]);
    hand(&mut g, P0, &[Card::Relic]);
    deck(&mut g, P0, &[Card::Copper; 5]);
    deck(&mut g, P1, &[Card::Copper; 10]);
    g.start();
    play(&mut g, Card::Relic);
    assert_eq!(g.turn().coins, 2);
    assert!(g.player(P1).minus_card);
    finish_turn(&mut g);
    finish_turn(&mut g);
    // P1's Clean-up drew one card fewer.
    assert_eq!(g.hand(P1).len(), 4);
    assert!(!g.player(P1).minus_card);
}

#[test]
fn moat_blocks_relic() {
    let mut g = game(&[Card::Relic, Card::Moat], &[]);
    hand(&mut g, P0, &[Card::Relic]);
    hand(&mut g, P1, &[Card::Moat]);
    g.start();
    play(&mut g, Card::Relic);
    accept(&mut g, Card::Moat);
    assert!(!g.player(P1).minus_card);
}

#[test]
fn storyteller_plays_treasures_and_turns_all_coins_into_cards() {
    let mut g = game(&[Card::Storyteller], &[]);
    hand(
        &mut g,
        P0,
        &[Card::Storyteller, Card::Silver, Card::Copper, Card::Estate],
    );
    deck(&mut g, P0, &[Card::Copper; 10]);
    g.start();
    play(&mut g, Card::Storyteller);
    choose_kinds(&mut g, &[Card::Silver]);
    choose_kinds(&mut g, &[Card::Copper]);
    assert_eq!(g.turn().coins, 0);
    assert_eq!(g.hand(P0).len(), 1 + 1 + 3);
    assert_eq!(
        kinds(&g, g.in_play(P0)),
        [Card::Storyteller, Card::Silver, Card::Copper]
    );
}

#[test]
fn treasure_trove_gains_a_gold_and_a_copper() {
    let mut g = game(&[Card::TreasureTrove], &[]);
    hand(&mut g, P0, &[Card::TreasureTrove]);
    g.start();
    play(&mut g, Card::TreasureTrove);
    assert_eq!(g.turn().coins, 2);
    assert_eq!(g.turn().gained, [Card::Gold, Card::Copper]);
}

// ---- Events ----

/// Sets the turn's $ and Buys and re-asks the pending Buy-phase decision.
fn budget(g: &mut Game, coins: u32, buys: u32) {
    g.turn.coins = coins;
    g.turn.buys = buys;
    g.pending = None;
    g.run(crate::frame::Input::Start);
}

/// Starts a game in P0's Buy phase with `coins` and an empty hand.
fn buy_phase(kingdom: &[Card], events: &[Card], coins: u32) -> Game {
    let mut g = game(kingdom, events);
    hand(&mut g, P0, &[]);
    deck(&mut g, P0, &[Card::Copper; 10]);
    g.start();
    budget(&mut g, coins, 1);
    g
}

#[test]
fn events_cost_a_buy_and_are_never_gained() {
    let mut g = buy_phase(&[Card::Smithy], &[Card::Expedition], 3);
    assert!(moves(&g).contains(&Move::BuyLandscape(Card::Expedition)));
    buy_event(&mut g, Card::Expedition);
    assert_eq!(g.turn().player, P1);
    assert!(g.turn().gained.is_empty());
}

#[test]
fn alms_gains_a_card_up_to_four_with_no_treasures_in_play_once_a_turn() {
    let mut g = buy_phase(&[Card::Smithy], &[Card::Alms], 0);
    let coins = g.turn().coins;
    budget(&mut g, coins, 3);
    buy_event(&mut g, Card::Alms);
    choose_pile(&mut g, Card::Smithy);
    assert_eq!(g.turn().gained, [Card::Smithy]);
    assert!(!moves(&g).contains(&Move::BuyLandscape(Card::Alms)));
}

#[test]
fn alms_does_nothing_with_a_treasure_in_play() {
    let mut g = game(&[Card::Smithy], &[Card::Alms]);
    hand(&mut g, P0, &[Card::Copper]);
    g.start();
    play(&mut g, Card::Copper);
    buy_event(&mut g, Card::Alms);
    assert!(g.turn().gained.is_empty());
}

#[test]
fn borrow_gives_a_buy_and_a_coin_for_the_minus_card_token() {
    let mut g = buy_phase(&[Card::Smithy], &[Card::Borrow], 0);
    buy_event(&mut g, Card::Borrow);
    assert_eq!((g.turn().buys, g.turn().coins), (1, 1));
    assert!(g.player(P0).minus_card);
    assert!(!moves(&g).contains(&Move::BuyLandscape(Card::Borrow)));
    end_phase(&mut g);
    // Clean-up draws one card fewer.
    assert_eq!(g.hand(P0).len(), 4);
}

#[test]
fn borrow_gives_no_coin_when_the_token_is_already_on_the_deck() {
    let mut g = buy_phase(&[Card::Smithy], &[Card::Borrow], 0);
    g.players[0].minus_card = true;
    buy_event(&mut g, Card::Borrow);
    assert_eq!((g.turn().buys, g.turn().coins), (1, 0));
}

#[test]
fn quest_discards_an_attack_for_a_gold() {
    let mut g = game(&[Card::Militia], &[Card::Quest]);
    hand(&mut g, P0, &[Card::Militia, Card::Copper]);
    g.start();
    end_phase(&mut g);
    buy_event(&mut g, Card::Quest);
    mode(&mut g, 0);
    assert_eq!(g.count_owned(P0, Card::Gold), 1);
    assert!(discard_kinds(&g, P0).contains(&Card::Militia));
}

#[test]
fn quest_with_too_few_cards_discards_them_and_gains_nothing() {
    let mut g = game(&[Card::Militia], &[Card::Quest]);
    hand(&mut g, P0, &[Card::Curse, Card::Estate, Card::Estate]);
    g.start();
    let coins = g.turn().coins;
    budget(&mut g, coins, 2);
    buy_event(&mut g, Card::Quest);
    mode(&mut g, 1);
    assert_eq!(discard_kinds(&g, P0), [Card::Curse]);
    buy_event(&mut g, Card::Quest);
    mode(&mut g, 2);
    assert_eq!(
        discard_kinds(&g, P0),
        [Card::Curse, Card::Estate, Card::Estate]
    );
    assert!(g.turn().gained.is_empty());
}

#[test]
fn save_sets_a_card_aside_and_returns_it_after_drawing() {
    let mut g = game(&[Card::Smithy], &[Card::Save]);
    let gold = hand(&mut g, P0, &[Card::Gold, Card::Copper])[0];
    deck(&mut g, P0, &[Card::Copper; 10]);
    g.start();
    let buys = g.turn().buys;
    budget(&mut g, 1, buys);
    buy_event(&mut g, Card::Save);
    choose(&mut g, &[gold]);
    assert_eq!(g.zone(gold), Zone::SetAside(P0));
    assert_eq!(g.turn().buys, 1);
    assert!(!moves(&g).contains(&Move::BuyLandscape(Card::Save)));
    end_phase(&mut g);
    assert_eq!(g.hand(P0).len(), 6);
    assert_eq!(g.zone(gold), Zone::Hand(P0));
}

#[test]
fn scouting_party_discards_three_of_the_top_five_and_puts_the_rest_back() {
    let mut g = buy_phase(&[Card::Smithy], &[Card::ScoutingParty], 2);
    deck(
        &mut g,
        P0,
        &[
            Card::Estate,
            Card::Gold,
            Card::Curse,
            Card::Silver,
            Card::Estate,
            Card::Copper,
        ],
    );
    buy_event(&mut g, Card::ScoutingParty);
    choose_kinds(&mut g, &[Card::Estate, Card::Curse, Card::Estate]);
    choose_kinds(&mut g, &[Card::Silver, Card::Gold]);
    assert_eq!(deck_kinds(&g, P0), [Card::Silver, Card::Gold, Card::Copper]);
    assert_eq!(g.turn().buys, 1);
}

#[test]
fn travelling_fair_lets_gained_cards_go_onto_the_deck_this_turn() {
    let mut g = buy_phase(&[Card::Smithy], &[Card::TravellingFair], 5);
    buy_event(&mut g, Card::TravellingFair);
    assert_eq!(g.turn().buys, 2);
    buy(&mut g, Card::Silver);
    accept(&mut g, Card::TravellingFair);
    assert_eq!(deck_kinds(&g, P0)[0], Card::Silver);
    buy(&mut g, Card::Copper);
    no(&mut g);
    assert_eq!(g.turn().player, P1);
    assert!(discard_kinds(&g, P0).contains(&Card::Copper));
}

#[test]
fn bonfire_trashes_coppers_in_play_and_keeps_their_coins() {
    let mut g = game(&[Card::Smithy], &[Card::Bonfire]);
    hand(&mut g, P0, &[Card::Copper; 5]);
    g.start();
    for _ in 0..5 {
        play(&mut g, Card::Copper);
    }
    let coins = g.turn().coins;
    budget(&mut g, coins, 2);
    buy_event(&mut g, Card::Bonfire);
    let coppers = candidates(&g)[..2].to_vec();
    choose(&mut g, &coppers);
    assert_eq!(g.in_play(P0).len(), 3);
    assert_eq!(g.turn().coins, 2);
}

#[test]
fn expedition_draws_two_extra_cards_for_the_next_hand_and_stacks() {
    let mut g = buy_phase(&[Card::Smithy], &[Card::Expedition], 6);
    let coins = g.turn().coins;
    budget(&mut g, coins, 2);
    buy_event(&mut g, Card::Expedition);
    buy_event(&mut g, Card::Expedition);
    assert_eq!(g.hand(P0).len(), 9);
}

#[test]
fn ferry_makes_a_pile_cost_two_less_on_its_owners_turns() {
    let mut g = buy_phase(&[Card::Smithy, Card::Village], &[Card::Ferry], 3);
    budget(&mut g, 3, 2);
    buy_event(&mut g, Card::Ferry);
    choose_pile(&mut g, Card::Smithy);
    assert_eq!(g.cost(Card::Smithy), Cost::coins(2));
    assert_eq!(g.cost(Card::Village), Cost::coins(3));
    end_phase(&mut g);
    assert_eq!(g.turn().player, P1);
    assert_eq!(g.cost(Card::Smithy), Cost::coins(4));
}

#[test]
fn plan_lets_a_gain_from_the_pile_trash_a_card_from_the_hand() {
    let mut g = game(&[Card::Smithy, Card::Workshop], &[Card::Plan]);
    on_tavern(&mut g, P0, Card::Copper);
    hand(&mut g, P0, &[Card::Workshop, Card::Estate, Card::Copper]);
    let smithy = g.supply_pile(Card::Smithy).unwrap();
    place(&mut g, P0, Token::Trashing, smithy);
    g.start();
    play(&mut g, Card::Workshop);
    choose_pile(&mut g, Card::Silver);
    // Not from that pile: nothing.
    assert_eq!(g.hand(P0).len(), 2);
    let buys = g.turn().buys;
    budget(&mut g, 4, buys);
    buy(&mut g, Card::Smithy);
    choose_kinds(&mut g, &[Card::Estate]);
    assert_eq!(kinds(&g, g.trash()).last(), Some(&Card::Estate));
}

#[test]
fn mission_gives_an_extra_turn_without_buying_cards() {
    let mut g = buy_phase(&[Card::Smithy], &[Card::Mission, Card::Expedition], 4);
    buy_event(&mut g, Card::Mission);
    assert_eq!(g.turn().player, P0);
    assert_eq!(g.turn().extra, Some(Card::Mission));
    budget(&mut g, 8, 3);
    let m = moves(&g);
    assert!(!m.iter().any(|m| matches!(m, Move::Buy(_))));
    assert!(m.contains(&Move::BuyLandscape(Card::Expedition)));
    // Not a third turn in a row.
    buy_event(&mut g, Card::Mission);
    end_phase(&mut g);
    assert_eq!(g.turn().player, P1);
}

#[test]
fn pilgrimage_flips_the_journey_token_and_copies_differently_named_cards() {
    let mut g = game(&[Card::Smithy], &[Card::Pilgrimage]);
    hand(
        &mut g,
        P0,
        &[Card::Silver, Card::Silver, Card::Copper, Card::Gold],
    );
    g.start();
    for _ in 0..4 {
        let c = candidates_of_play(&g);
        respond(&mut g, Response::Move(Move::Play(c)));
    }
    let coins = g.turn().coins;
    budget(&mut g, coins, 2);
    buy_event(&mut g, Card::Pilgrimage);
    // Face down now: nothing.
    assert!(!g.player(P0).journey_face_up);
    assert!(!moves(&g).contains(&Move::BuyLandscape(Card::Pilgrimage)));
    end_phase(&mut g);
    finish_turn(&mut g);
    let mut g2 = game(&[Card::Smithy], &[Card::Pilgrimage]);
    hand(&mut g2, P0, &[Card::Silver, Card::Silver, Card::Gold]);
    g2.players[0].journey_face_up = false;
    g2.start();
    for _ in 0..3 {
        let c = candidates_of_play(&g2);
        respond(&mut g2, Response::Move(Move::Play(c)));
    }
    buy_event(&mut g2, Card::Pilgrimage);
    assert_eq!(candidates(&g2).len(), 2);
    let all = candidates(&g2);
    choose(&mut g2, &all);
    assert_eq!(g2.count_owned(P0, Card::Silver), 3);
    assert_eq!(g2.count_owned(P0, Card::Gold), 2);
}

/// The first card the pending Turn decision lets the player play.
fn candidates_of_play(g: &Game) -> CardId {
    moves(g)
        .into_iter()
        .find_map(|m| match m {
            Move::Play(c) => Some(c),
            _ => None,
        })
        .expect("a card to play")
}

#[test]
fn ball_takes_the_minus_coin_token_and_gains_two_cards_up_to_four() {
    let mut g = buy_phase(&[Card::Smithy], &[Card::Ball], 5);
    buy_event(&mut g, Card::Ball);
    choose_pile(&mut g, Card::Smithy);
    choose_pile(&mut g, Card::Silver);
    assert!(g.player(P0).minus_coin);
    assert_eq!(g.count_owned(P0, Card::Smithy), 1);
    assert_eq!(g.count_owned(P0, Card::Silver), 1);
}

#[test]
fn raid_gains_a_silver_per_silver_in_play_and_isnt_an_attack() {
    let mut g = game(&[Card::Moat], &[Card::Raid]);
    hand(&mut g, P0, &[Card::Silver, Card::Silver, Card::Copper]);
    hand(&mut g, P1, &[Card::Moat]);
    g.start();
    play(&mut g, Card::Silver);
    play(&mut g, Card::Silver);
    play(&mut g, Card::Copper);
    buy_event(&mut g, Card::Raid);
    assert_eq!(g.count_owned(P0, Card::Silver), 4);
    assert!(g.player(P1).minus_card);
}

#[test]
fn seaway_gains_an_action_and_puts_the_buy_token_on_its_pile() {
    let mut g = buy_phase(
        &[Card::Smithy, Card::Village, Card::Festival],
        &[Card::Seaway],
        5,
    );
    buy_event(&mut g, Card::Seaway);
    let DecisionKind::Pile { piles, .. } = &g.pending().unwrap().kind else {
        panic!("expected a pile choice");
    };
    assert_eq!(piles.len(), 2);
    choose_pile(&mut g, Card::Village);
    let village = g.supply_pile(Card::Village).unwrap();
    assert_eq!(g.player(P0).tokens[&Token::PlusBuy], Some(village));
    assert_eq!(g.pile(village).tokens, [(P0, Token::PlusBuy)]);
}

#[test]
fn trade_gains_a_silver_per_card_trashed() {
    let mut g = game(&[Card::Smithy], &[Card::Trade]);
    hand(&mut g, P0, &[Card::Estate, Card::Curse, Card::Copper]);
    g.start();
    let buys = g.turn().buys;
    budget(&mut g, 5, buys);
    buy_event(&mut g, Card::Trade);
    choose_kinds(&mut g, &[Card::Estate, Card::Curse]);
    assert_eq!(g.count_owned(P0, Card::Silver), 2);
}

#[test]
fn bonus_tokens_apply_to_every_play_of_a_card_from_their_pile() {
    let mut g = game(
        &[Card::Smithy, Card::ThroneRoom],
        &[Card::LostArts, Card::Training, Card::Pathfinding],
    );
    hand(&mut g, P0, &[Card::ThroneRoom, Card::Smithy]);
    deck(&mut g, P0, &[Card::Copper; 20]);
    g.start();
    let smithy = g.supply_pile(Card::Smithy).unwrap();
    for t in [Token::PlusAction, Token::PlusCoin, Token::PlusCard] {
        place(&mut g, P0, t, smithy);
    }
    play(&mut g, Card::ThroneRoom);
    choose_kinds(&mut g, &[Card::Smithy]);
    assert_eq!(g.turn().actions, 2);
    assert_eq!(g.turn().coins, 2);
    assert_eq!(g.hand(P0).len(), 2 * 4);
}

#[test]
fn lost_arts_training_and_pathfinding_move_their_tokens() {
    let mut g = buy_phase(
        &[Card::Smithy, Card::Village],
        &[Card::LostArts, Card::Training, Card::Pathfinding],
        20,
    );
    let coins = g.turn().coins;
    budget(&mut g, coins, 4);
    buy_event(&mut g, Card::LostArts);
    choose_pile(&mut g, Card::Smithy);
    buy_event(&mut g, Card::Training);
    choose_pile(&mut g, Card::Smithy);
    buy_event(&mut g, Card::LostArts);
    let DecisionKind::Pile { piles, .. } = &g.pending().unwrap().kind else {
        panic!("expected a pile choice");
    };
    // Only Action Supply piles.
    assert_eq!(piles.len(), 2);
    choose_pile(&mut g, Card::Village);
    let smithy = g.supply_pile(Card::Smithy).unwrap();
    assert_eq!(g.pile(smithy).tokens, [(P0, Token::PlusCoin)]);
    assert_eq!(
        g.player(P0).tokens[&Token::PlusAction],
        g.supply_pile(Card::Village)
    );
}

#[test]
fn inheritance_sets_an_action_aside_and_makes_estates_play_it() {
    let mut g = game(
        &[Card::Village, Card::Smithy, Card::Festival],
        &[Card::Inheritance],
    );
    hand(&mut g, P0, &[]);
    deck(
        &mut g,
        P0,
        &[
            Card::Estate,
            Card::Estate,
            Card::Copper,
            Card::Copper,
            Card::Copper,
        ],
    );
    g.start();
    let buys = g.turn().buys;
    budget(&mut g, 7, buys);
    buy_event(&mut g, Card::Inheritance);
    let DecisionKind::Pile { piles, .. } = &g.pending().unwrap().kind else {
        panic!("expected a pile choice");
    };
    // Up to $4, not Festival.
    assert_eq!(piles.len(), 2);
    choose_pile(&mut g, Card::Village);
    let village = g.player(P0).set_aside[0];
    assert_eq!(g.card(village), Card::Village);
    assert_eq!(g.view(P1).card(village), Some(Card::Village));
    assert!(g.view(P1).notes().contains(&Note {
        source: Card::Inheritance,
        player: Some(P0),
        pile: None,
        card: Some(Card::Village),
        count: 0,
    }));
    assert_eq!(g.turn().player, P1);
    let estate = find(&g, P1, Card::Estate);
    // Not Actions on another player's turn.
    assert!(!g.is(estate, Types::ACTION));
    finish_turn(&mut g);
    let estate = find(&g, P0, Card::Estate);
    assert!(g.is(estate, Types::ACTION | Types::COMMAND));
    assert!(g.is(estate, Types::VICTORY));
    play(&mut g, Card::Estate);
    assert_eq!(g.turn().actions, 2);
    assert_eq!(g.zone(village), Zone::SetAside(P0));
    play(&mut g, Card::Estate);
    assert_eq!(g.turn().actions, 3);
    assert_eq!(
        g.turn().played,
        [Card::Estate, Card::Village, Card::Estate, Card::Village]
    );
    // Once per game.
    let buys = g.turn().buys;
    budget(&mut g, 7, buys);
    end_phase(&mut g);
    assert!(!moves(&g).contains(&Move::BuyLandscape(Card::Inheritance)));
    // The set-aside card is part of the deck for scoring.
    assert_eq!(g.count_owned(P0, Card::Village), 1);
}

#[test]
fn inheritance_does_not_offer_duration_or_command_cards() {
    let mut g = buy_phase(&[Card::Amulet, Card::Smithy], &[Card::Inheritance], 7);
    buy_event(&mut g, Card::Inheritance);
    assert!(
        g.player(P0)
            .set_aside
            .iter()
            .all(|&c| g.card(c) == Card::Smithy)
    );
    assert_eq!(g.player(P0).set_aside.len(), 1);
}

#[test]
fn an_inherited_card_gets_the_tokens_on_its_pile_and_champion() {
    let mut g = game(&[Card::Village, Card::Page], &[Card::Inheritance]);
    let village = g.supply_pile(Card::Village).unwrap();
    let top = g.pile_top(village).unwrap();
    g.move_card(top, Zone::SetAside(P0), crate::game::Pos::Top);
    g.ext.adventures.inherited.push((P0, Some(top)));
    place(&mut g, P0, Token::PlusCoin, village);
    hand(&mut g, P0, &[Card::Champion, Card::Estate]);
    deck(&mut g, P0, &[Card::Copper; 5]);
    g.start();
    play(&mut g, Card::Champion);
    play(&mut g, Card::Estate);
    // Champion's +1 Action for the Estate and again for the Village, then +2 Actions.
    assert_eq!(g.turn().actions, 4);
    assert_eq!(g.turn().coins, 1);
}

// ---- Rulings ----

#[test]
fn champion_does_not_stop_an_attack_played_before_it() {
    let mut g = game(&[Card::SwampHag, Card::Page], &[]);
    hand(&mut g, P0, &[Card::SwampHag]);
    deck(&mut g, P0, &[Card::Copper; 10]);
    hand(&mut g, P1, &[Card::Champion, Card::Copper]);
    g.start();
    play(&mut g, Card::SwampHag);
    finish_turn(&mut g);
    play(&mut g, Card::Champion);
    buy(&mut g, Card::Copper);
    assert_eq!(g.count_owned(P1, Card::Curse), 1);
}

#[test]
fn seaway_can_gain_an_estate_for_a_player_with_inheritance() {
    let mut g = game(&[Card::Smithy], &[Card::Seaway]);
    let village = g.create(Card::Village, Zone::SetAside(P0));
    g.ext.adventures.inherited.push((P0, Some(village)));
    hand(&mut g, P0, &[]);
    g.start();
    budget(&mut g, 5, 1);
    buy_event(&mut g, Card::Seaway);
    choose_pile(&mut g, Card::Estate);
    assert_eq!(
        g.player(P0).tokens[&Token::PlusBuy],
        g.supply_pile(Card::Estate)
    );
}

#[test]
fn messenger_does_nothing_extra_when_it_is_not_the_first_gain_in_the_buy_phase() {
    let mut g = game(&[Card::Messenger, Card::Workshop], &[]);
    hand(&mut g, P0, &[Card::Gold, Card::Gold]);
    g.start();
    play(&mut g, Card::Gold);
    play(&mut g, Card::Gold);
    budget(&mut g, 6, 2);
    buy(&mut g, Card::Copper);
    buy(&mut g, Card::Messenger);
    assert_eq!(g.turn().player, P1);
    assert_eq!(g.count_owned(P1, Card::Silver), 0);
    assert_eq!(g.player(P1).discard.len(), 0);
}

#[test]
fn messenger_gained_in_the_action_phase_does_nothing_extra() {
    let mut g = game(&[Card::Messenger, Card::Workshop], &[]);
    hand(&mut g, P0, &[Card::Workshop]);
    g.start();
    play(&mut g, Card::Workshop);
    choose_pile(&mut g, Card::Messenger);
    assert_eq!(g.turn().gained, [Card::Messenger]);
    assert_eq!(purpose(&g), Purpose::Turn);
}

#[test]
fn raze_uses_the_cost_lowered_by_bridge_troll() {
    let mut g = game(&[Card::Raze, Card::BridgeTroll, Card::Village], &[]);
    hand(
        &mut g,
        P0,
        &[Card::Village, Card::BridgeTroll, Card::Raze, Card::Estate],
    );
    deck(&mut g, P0, &[Card::Copper, Card::Silver, Card::Gold]);
    g.start();
    play(&mut g, Card::Village);
    play(&mut g, Card::BridgeTroll);
    play(&mut g, Card::Raze);
    choose_kinds(&mut g, &[Card::Estate]);
    // The Estate costs $1 this turn: one card, taken into the hand.
    // (Village drew the Copper.)
    assert_eq!(deck_kinds(&g, P0), [Card::Gold]);
    assert_eq!(hand_kinds(&g, P0), [Card::Copper, Card::Silver]);
}

#[test]
fn coin_of_the_realm_called_on_another_players_turn_gives_nothing() {
    let mut g = game(
        &[Card::CoinOfTheRealm, Card::CaravanGuard, Card::Militia],
        &[],
    );
    on_tavern(&mut g, P1, Card::CoinOfTheRealm);
    hand(&mut g, P0, &[Card::Militia]);
    deck(&mut g, P0, &[Card::Copper; 10]);
    hand(
        &mut g,
        P1,
        &[Card::CaravanGuard, Card::Copper, Card::Copper],
    );
    g.start();
    play(&mut g, Card::Militia);
    accept(&mut g, Card::CaravanGuard);
    accept(&mut g, Card::CoinOfTheRealm);
    assert_eq!(g.turn().actions, 0);
    assert_eq!(
        kinds(&g, g.in_play(P1)),
        [Card::CaravanGuard, Card::CoinOfTheRealm]
    );
}

#[test]
fn saves_card_is_set_aside_face_down() {
    let mut g = game(&[Card::Smithy], &[Card::Save]);
    let gold = hand(&mut g, P0, &[Card::Gold, Card::Copper])[0];
    g.start();
    let buys = g.turn().buys;
    budget(&mut g, 1, buys);
    buy_event(&mut g, Card::Save);
    choose(&mut g, &[gold]);
    assert_eq!(g.view(P1).card(gold), None);
    assert_eq!(g.view(P0).card(gold), Some(Card::Gold));
}
