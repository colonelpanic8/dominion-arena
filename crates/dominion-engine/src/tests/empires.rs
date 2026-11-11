use super::intrigue::{modes, random_games_with_copies, trigger};
use super::*;

/// Random games over Empires with copies of its cards in every deck, and two random
/// Events or Landmarks.
#[test]
fn random_empires_games_keep_invariants() {
    let pool: Vec<Card> = Card::in_set(Set::Empires)
        .filter(|c| c.def().kingdom)
        .collect();
    let landscapes: Vec<Card> = Card::in_set(Set::Empires)
        .filter(|c| c.kind().is_landscape())
        .collect();
    let extra: Vec<Card> = Card::in_set(Set::Empires)
        .filter(|c| c.kind() == Kind::Card && !c.def().kingdom)
        .collect();
    random_games_with_copies(&pool, &extra, 0..fuzz_seeds(150), |seed| {
        let mut rng = ChaCha8Rng::seed_from_u64(seed ^ 0xe);
        let mut l = landscapes.clone();
        l.shuffle(&mut rng);
        l.truncate(3);
        l
    });
}

fn setup_landscapes(kingdom: &[Card], landscapes: &[Card]) -> Game {
    setup_with(GameConfig {
        colonies: Some(false),
        shelters: Some(false),
        landscapes: landscapes.to_vec(),
        ..GameConfig::new(2, kingdom.to_vec(), 7)
    })
}

fn turn_starts(g: &Game) -> usize {
    g.log()
        .iter()
        .filter(|e| matches!(e, Event::TurnStart { .. }))
        .count()
}

/// Ends phases (resolving triggers in the order offered) until the next turn starts.
fn next_turn(g: &mut Game) {
    let n = turn_starts(g);
    while turn_starts(g) == n {
        let d = g.pending().expect("a decision is pending");
        match &d.kind {
            DecisionKind::Turn { .. } => end_phase(g),
            _ if d.purpose == Purpose::Trigger => resolve_triggers(g),
            k => panic!("unexpected decision {k:?}"),
        }
    }
}

/// Ends the Action phase if it is still on.
fn to_buy(g: &mut Game) {
    if let Some(DecisionKind::Turn {
        phase: Phase::Action,
        ..
    }) = g.pending().map(|d| &d.kind)
    {
        end_phase(g);
    }
}

fn resolve_triggers(g: &mut Game) {
    while g.pending().is_some_and(|d| d.purpose == Purpose::Trigger) {
        respond(g, Response::option(Opt::Trigger(0)));
    }
}

fn moves(g: &Game) -> Vec<Move> {
    match &g.pending().expect("a decision is pending").kind {
        DecisionKind::Turn { moves, .. } => moves.clone(),
        k => panic!("expected a Turn decision, got {k:?}"),
    }
}

fn phase(g: &Game) -> Phase {
    match &g.pending().expect("a decision is pending").kind {
        DecisionKind::Turn { phase, .. } => *phase,
        k => panic!("expected a Turn decision, got {k:?}"),
    }
}

fn can_buy(g: &Game, card: Card) -> bool {
    moves(g).contains(&Move::Buy(top_pile(g, card)))
}

/// Buys the top card of whichever Supply pile has `card` on top.
fn buy(g: &mut Game, card: Card) {
    let pile = top_pile(g, card);
    respond(g, Response::Move(Move::Buy(pile)));
}

fn top_pile(g: &Game, card: Card) -> PileId {
    (0..g.piles().len())
        .map(|i| PileId(i as u8))
        .find(|&id| g.pile(id).in_supply && g.pile_top(id).is_some_and(|t| g.card(t) == card))
        .unwrap_or_else(|| panic!("no Supply pile with {card} on top"))
}

fn buy_event(g: &mut Game, card: Card) {
    respond(g, Response::Move(Move::BuyLandscape(card)));
}

fn number(g: &mut Game, n: i32) {
    respond(g, Response::Number(n));
}

fn play_all(g: &mut Game, card: Card) {
    let p = g.pending().unwrap().player;
    while g.hand(p).iter().any(|&c| g.card(c) == card) {
        play(g, card);
    }
}

/// A pile's cards, top first.
fn pile_kinds(g: &Game, card: Card) -> Vec<Card> {
    let pile = g.pile_of(card).expect("pile exists");
    g.pile(pile)
        .cards
        .iter()
        .rev()
        .map(|&c| g.card(c))
        .collect()
}

/// Takes the top `n` cards off a pile (into the trash).
fn strip(g: &mut Game, card: Card, n: usize) {
    let pile = g.pile_of(card).expect("pile exists");
    for _ in 0..n {
        let top = g.pile_top(pile).expect("pile has cards");
        g.move_card(top, Zone::Trash, crate::game::Pos::Top);
    }
}

fn in_play_kinds(g: &Game, p: PlayerId) -> Vec<Card> {
    kinds(g, g.in_play(p))
}

fn discard_kinds(g: &Game, p: PlayerId) -> Vec<Card> {
    kinds(g, &g.player(p).discard)
}

fn vp(g: &Game, p: PlayerId) -> i32 {
    g.player(p).vp_tokens
}

fn pile_vp(g: &Game, card: Card) -> i32 {
    g.pile(g.pile_of(card).unwrap()).vp_tokens
}

// ---- Piles and Debt ----

#[test]
fn split_piles_have_five_of_each_half_with_the_top_half_on_top() {
    let mut g = setup(&[
        Card::Catapult,
        Card::Encampment,
        Card::Gladiator,
        Card::Patrician,
        Card::Settlers,
    ]);
    for (top, bottom) in [
        (Card::Catapult, Card::Rocks),
        (Card::Encampment, Card::Plunder),
        (Card::Gladiator, Card::Fortune),
        (Card::Patrician, Card::Emporium),
        (Card::Settlers, Card::BustlingVillage),
    ] {
        let cards = pile_kinds(&g, top);
        assert_eq!(cards[..5], [top; 5]);
        assert_eq!(cards[5..], [bottom; 5]);
    }
    strip(&mut g, Card::Catapult, 5);
    assert_eq!(g.empty_supply_piles(), 0);
    hand(&mut g, P0, &[Card::Gold, Card::Gold]);
    g.start();
    to_buy(&mut g);
    play_all(&mut g, Card::Gold);
    buy(&mut g, Card::Rocks);
    assert_eq!(g.count_owned(P0, Card::Rocks), 1);
}

#[test]
fn the_castles_pile_is_sorted_by_cost_with_eight_or_twelve_cards() {
    let g = setup(&[Card::Castles]);
    assert_eq!(
        pile_kinds(&g, Card::Castles),
        [
            Card::HumbleCastle,
            Card::CrumblingCastle,
            Card::SmallCastle,
            Card::HauntedCastle,
            Card::OpulentCastle,
            Card::SprawlingCastle,
            Card::GrandCastle,
            Card::KingsCastle,
        ]
    );
    let g = super::intrigue::setup3(&[Card::Castles]);
    assert_eq!(
        pile_kinds(&g, Card::Castles),
        [
            Card::HumbleCastle,
            Card::HumbleCastle,
            Card::CrumblingCastle,
            Card::SmallCastle,
            Card::SmallCastle,
            Card::HauntedCastle,
            Card::OpulentCastle,
            Card::OpulentCastle,
            Card::SprawlingCastle,
            Card::GrandCastle,
            Card::KingsCastle,
            Card::KingsCastle,
        ]
    );
}

#[test]
fn buying_a_debt_card_takes_debt_that_blocks_buying_until_paid() {
    let mut g = setup(&[Card::Overlord, Card::Village]);
    hand(&mut g, P0, &[Card::Gold, Card::Gold]);
    g.start();
    to_buy(&mut g);
    play_all(&mut g, Card::Gold);
    g.turn.buys = 2;
    buy(&mut g, Card::Overlord);
    assert_eq!(g.player(P0).debt, 8);
    assert!(!moves(&g).iter().any(|m| matches!(m, Move::Buy(_))));
    for _ in 0..5 {
        respond(&mut g, Response::Move(Move::PayDebt));
    }
    assert_eq!(g.player(P0).debt, 3);
    assert_eq!(moves(&g), [Move::PayDebt, Move::EndPhase]);
}

// ---- Kingdom cards ----

#[test]
fn archive_gives_one_set_aside_card_now_and_one_at_each_of_the_next_two_turns() {
    let mut g = setup(&[Card::Archive]);
    hand(&mut g, P0, &[Card::Archive]);
    deck(
        &mut g,
        P0,
        &[
            Card::Gold,
            Card::Silver,
            Card::Duchy,
            Card::Copper,
            Card::Copper,
        ],
    );
    g.start();
    play(&mut g, Card::Archive);
    assert_eq!(g.player(P0).set_aside.len(), 3);
    assert_eq!(g.view(P1).card(g.player(P0).set_aside[0]), None);
    choose_kinds(&mut g, &[Card::Gold]);
    assert_eq!(hand_kinds(&g, P0), [Card::Gold]);
    next_turn(&mut g);
    assert_eq!(in_play_kinds(&g, P0), [Card::Archive]);
    next_turn(&mut g);
    choose_kinds(&mut g, &[Card::Duchy]);
    assert!(hand_kinds(&g, P0).contains(&Card::Duchy));
    next_turn(&mut g);
    assert_eq!(in_play_kinds(&g, P0), [Card::Archive]);
    next_turn(&mut g);
    // The last card comes by itself.
    assert!(hand_kinds(&g, P0).contains(&Card::Silver));
    assert!(g.player(P0).set_aside.is_empty());
    next_turn(&mut g);
    assert!(g.in_play(P0).is_empty());
    assert!(g.lasting().is_empty());
}

#[test]
fn two_archives_keep_separate_sets() {
    let mut g = setup(&[Card::Archive, Card::Village]);
    hand(&mut g, P0, &[Card::Village, Card::Archive, Card::Archive]);
    deck(
        &mut g,
        P0,
        &[
            &[
                Card::Village,
                Card::Gold,
                Card::Silver,
                Card::Duchy,
                Card::Estate,
                Card::Copper,
                Card::Province,
            ][..],
            &[Card::Curse; 12],
        ]
        .concat(),
    );
    g.start();
    play(&mut g, Card::Village);
    play(&mut g, Card::Archive);
    choose_kinds(&mut g, &[Card::Gold]);
    play(&mut g, Card::Archive);
    choose_kinds(&mut g, &[Card::Estate]);
    next_turn(&mut g);
    next_turn(&mut g);
    // One card from each Archive, from its own set.
    let offered: Vec<Vec<Card>> = (0..2)
        .map(|_| {
            resolve_triggers(&mut g);
            let offered = candidates(&g);
            let c = kinds(&g, &offered);
            choose(&mut g, &offered[..1]);
            c
        })
        .collect();
    let mut sets: Vec<Vec<Card>> = offered
        .into_iter()
        .map(|mut v| {
            v.sort();
            v
        })
        .collect();
    sets.sort();
    let mut a = vec![Card::Silver, Card::Duchy];
    a.sort();
    let mut b = vec![Card::Copper, Card::Province];
    b.sort();
    let mut want = vec![a, b];
    want.sort();
    assert_eq!(sets, want);
}

#[test]
fn capital_takes_six_debt_when_discarded_from_play_and_lets_the_coins_left_pay_it() {
    let mut g = setup(&[Card::Capital, Card::Crown]);
    hand(&mut g, P0, &[Card::Capital, Card::Copper]);
    g.start();
    to_buy(&mut g);
    play(&mut g, Card::Capital);
    play(&mut g, Card::Copper);
    assert_eq!(g.turn().buys, 2);
    buy(&mut g, Card::Silver);
    end_phase(&mut g);
    // $4 left: pay up to 4 of the 6 Debt.
    assert_eq!(
        g.pending().unwrap().kind,
        DecisionKind::Number { min: 0, max: 4 }
    );
    number(&mut g, 3);
    assert_eq!(g.player(P0).debt, 3);
}

#[test]
fn capital_played_twice_by_crown_takes_debt_once() {
    let mut g = setup(&[Card::Capital, Card::Crown]);
    hand(&mut g, P0, &[Card::Crown, Card::Capital]);
    g.start();
    to_buy(&mut g);
    play(&mut g, Card::Crown);
    choose_kinds(&mut g, &[Card::Capital]);
    assert_eq!(g.turn().coins, 12);
    assert_eq!(g.turn().buys, 3);
    end_phase(&mut g);
    number(&mut g, 0);
    assert_eq!(g.player(P0).debt, 6);
}

#[test]
fn catapult_curses_for_three_or_more_and_cuts_hands_for_a_treasure() {
    let mut g = setup(&[Card::Catapult]);
    hand(
        &mut g,
        P0,
        &[
            Card::Catapult,
            Card::Catapult,
            Card::Catapult,
            Card::Silver,
            Card::Estate,
            Card::Copper,
        ],
    );
    g.turn.actions = 3;
    g.start();
    g.turn.actions = 3;
    play(&mut g, Card::Catapult);
    choose_kinds(&mut g, &[Card::Silver]);
    assert_eq!(discard_kinds(&g, P1), [Card::Curse]);
    assert_eq!(g.pending().unwrap().player, P1);
    let two = g.hand(P1)[..2].to_vec();
    choose(&mut g, &two);
    assert_eq!(g.hand(P1).len(), 3);
    assert_eq!(g.turn().coins, 1);
    play(&mut g, Card::Catapult);
    choose_kinds(&mut g, &[Card::Estate]);
    assert_eq!(g.count_owned(P1, Card::Curse), 1);
    play(&mut g, Card::Catapult);
    // Copper: a Treasure costing less than $3, but P1 already has 3 cards.
    assert_eq!(g.count_owned(P1, Card::Curse), 1);
    assert_eq!(g.turn().coins, 3);
}

#[test]
fn rocks_gives_a_silver_onto_the_deck_in_the_buy_phase_and_into_the_hand_otherwise() {
    let mut g = setup(&[Card::Catapult]);
    strip(&mut g, Card::Catapult, 5);
    hand(&mut g, P0, &[Card::Gold, Card::Catapult, Card::Rocks]);
    deck(&mut g, P0, &[Card::Copper; 3]);
    hand(&mut g, P1, &[Card::Copper; 3]);
    g.start();
    play(&mut g, Card::Catapult);
    choose_kinds(&mut g, &[Card::Rocks]);
    // Trashed in the Action phase: the Silver goes to the hand.
    assert_eq!(hand_kinds(&g, P0), [Card::Gold, Card::Silver]);
    assert_eq!(g.count_owned(P1, Card::Curse), 1);
    to_buy(&mut g);
    g.turn.buys = 2;
    play(&mut g, Card::Gold);
    play(&mut g, Card::Silver);
    buy(&mut g, Card::Rocks);
    assert_eq!(deck_kinds(&g, P0)[0], Card::Silver);
}

#[test]
fn chariot_race_draws_and_compares_costs_with_the_left_player() {
    let mut g = setup(&[Card::ChariotRace]);
    hand(&mut g, P0, &[Card::ChariotRace, Card::ChariotRace]);
    deck(&mut g, P0, &[Card::Gold, Card::Copper]);
    deck(&mut g, P1, &[Card::Silver]);
    g.start();
    play(&mut g, Card::ChariotRace);
    assert_eq!(g.turn().coins, 1);
    assert_eq!(vp(&g, P0), 1);
    assert_eq!(deck_kinds(&g, P1), [Card::Silver]);
    play(&mut g, Card::ChariotRace);
    assert_eq!(g.turn().coins, 1);
    assert_eq!(vp(&g, P0), 1);
    assert_eq!(hand_kinds(&g, P0), [Card::Gold, Card::Copper]);
}

#[test]
fn chariot_race_needs_a_card_on_both_sides() {
    let mut g = setup(&[Card::ChariotRace]);
    hand(&mut g, P0, &[Card::ChariotRace]);
    deck(&mut g, P0, &[Card::Gold]);
    deck(&mut g, P1, &[]);
    discard(&mut g, P1, &[]);
    g.start();
    play(&mut g, Card::ChariotRace);
    assert_eq!(g.turn().coins, 0);
    assert_eq!(vp(&g, P0), 0);
}

#[test]
fn charm_gives_a_buy_and_two_coins() {
    let mut g = setup(&[Card::Charm]);
    hand(&mut g, P0, &[Card::Charm]);
    g.start();
    to_buy(&mut g);
    play(&mut g, Card::Charm);
    modes(&mut g, &[0]);
    assert_eq!((g.turn().coins, g.turn().buys), (2, 2));
}

#[test]
fn charm_gains_a_differently_named_card_with_the_same_cost_on_the_next_gain() {
    let mut g = setup(&[Card::Charm, Card::Village, Card::Catapult]);
    hand(&mut g, P0, &[Card::Charm, Card::Silver, Card::Copper]);
    g.start();
    to_buy(&mut g);
    play(&mut g, Card::Charm);
    modes(&mut g, &[1]);
    play(&mut g, Card::Silver);
    play(&mut g, Card::Copper);
    buy(&mut g, Card::Silver);
    let DecisionKind::Pile { piles, optional } = &g.pending().unwrap().kind else {
        panic!("expected a pile choice");
    };
    assert!(optional);
    let names: Vec<Card> = piles.iter().map(|&p| g.pile(p).card).collect();
    assert_eq!(names.len(), 2);
    assert!(names.contains(&Card::Village) && names.contains(&Card::Catapult));
    choose_pile(&mut g, Card::Village);
    assert_eq!(g.count_owned(P0, Card::Village), 1);
}

#[test]
fn charm_is_spent_by_the_card_gained_not_by_what_its_gain_ability_gains() {
    let mut g = setup(&[Card::Charm, Card::Catapult, Card::Smithy]);
    strip(&mut g, Card::Catapult, 5);
    hand(&mut g, P0, &[Card::Charm, Card::Gold, Card::Copper]);
    g.start();
    to_buy(&mut g);
    play(&mut g, Card::Charm);
    modes(&mut g, &[1]);
    play(&mut g, Card::Gold);
    play(&mut g, Card::Copper);
    buy(&mut g, Card::Rocks);
    // Rocks's own Silver first: the Charm must still offer a $4 card for the Rocks.
    trigger(&mut g, Card::Rocks);
    let DecisionKind::Pile { piles, .. } = &g.pending().unwrap().kind else {
        panic!("expected Charm's pile choice, got {:?}", g.pending());
    };
    let names: Vec<Card> = piles.iter().map(|&p| g.pile(p).card).collect();
    assert_eq!(names, [Card::Smithy]);
    choose_pile(&mut g, Card::Smithy);
    assert_eq!(g.count_owned(P0, Card::Smithy), 1);
    assert_eq!(g.count_owned(P0, Card::Silver), 1);
}

#[test]
fn city_quarter_draws_a_card_per_action_in_hand() {
    let mut g = setup(&[Card::CityQuarter]);
    hand(
        &mut g,
        P0,
        &[Card::CityQuarter, Card::Village, Card::Crown, Card::Copper],
    );
    deck(&mut g, P0, &[Card::Copper; 5]);
    g.start();
    play(&mut g, Card::CityQuarter);
    assert_eq!(g.hand(P0).len(), 5);
    assert_eq!(g.turn().actions, 2);
}

#[test]
fn crown_plays_an_action_twice_in_the_action_phase_and_a_treasure_twice_in_the_buy_phase() {
    let mut g = setup(&[Card::Crown, Card::Village]);
    hand(
        &mut g,
        P0,
        &[Card::Crown, Card::Village, Card::Crown, Card::Gold],
    );
    deck(&mut g, P0, &[Card::Copper; 5]);
    g.start();
    play(&mut g, Card::Crown);
    choose_kinds(&mut g, &[Card::Village]);
    assert_eq!(g.turn().actions, 4);
    to_buy(&mut g);
    play(&mut g, Card::Crown);
    choose_kinds(&mut g, &[Card::Gold]);
    assert_eq!(g.turn().coins, 6);
}

#[test]
fn encampment_stays_with_a_gold_and_otherwise_returns_to_its_pile_at_cleanup() {
    let mut g = setup(&[Card::Encampment]);
    hand(
        &mut g,
        P0,
        &[Card::Encampment, Card::Encampment, Card::Gold],
    );
    deck(&mut g, P0, &[Card::Copper; 10]);
    g.start();
    play(&mut g, Card::Encampment);
    choose_kinds(&mut g, &[Card::Gold]);
    assert_eq!(in_play_kinds(&g, P0), [Card::Encampment]);
    play(&mut g, Card::Encampment);
    choose(&mut g, &[]);
    assert_eq!(kinds(&g, &g.player(P0).set_aside), [Card::Encampment]);
    assert_eq!(pile_kinds(&g, Card::Encampment).len(), 10);
    to_buy(&mut g);
    end_phase(&mut g);
    assert_eq!(pile_kinds(&g, Card::Encampment).len(), 11);
    assert_eq!(g.count_owned(P0, Card::Encampment), 1);
    assert!(g.player(P0).set_aside.is_empty());
}

#[test]
fn encampment_covers_plunder_when_it_returns_and_throne_room_returns_it_once() {
    let mut g = setup(&[Card::Encampment, Card::ThroneRoom]);
    strip(&mut g, Card::Encampment, 5);
    hand(&mut g, P0, &[Card::ThroneRoom, Card::Encampment]);
    deck(&mut g, P0, &[Card::Copper; 10]);
    g.start();
    play(&mut g, Card::ThroneRoom);
    choose_kinds(&mut g, &[Card::Encampment]);
    assert_eq!(g.turn().actions, 4);
    next_turn(&mut g);
    assert_eq!(pile_kinds(&g, Card::Encampment)[0], Card::Encampment);
    assert_eq!(pile_kinds(&g, Card::Encampment).len(), 6);
    assert_eq!(in_play_kinds(&g, P0), Vec::<Card>::new());
}

#[test]
fn enchantress_turns_the_first_action_of_each_other_players_turn_into_a_cantrip() {
    let mut g = setup(&[Card::Enchantress, Card::Smithy]);
    hand(&mut g, P0, &[Card::Enchantress]);
    deck(&mut g, P0, &[Card::Copper; 12]);
    hand(&mut g, P1, &[Card::Smithy, Card::Smithy]);
    deck(&mut g, P1, &[Card::Copper; 12]);
    g.start();
    play(&mut g, Card::Enchantress);
    next_turn(&mut g);
    play(&mut g, Card::Smithy);
    assert_eq!(g.hand(P1).len(), 2);
    assert_eq!(g.turn().actions, 1);
    play(&mut g, Card::Smithy);
    assert_eq!(g.hand(P1).len(), 4);
    next_turn(&mut g);
    assert_eq!(g.hand(P0).len(), 7);
    assert_eq!(in_play_kinds(&g, P0), [Card::Enchantress]);
    next_turn(&mut g);
    assert!(g.in_play(P0).is_empty());
}

#[test]
fn enchantress_affects_only_the_first_action_even_twice_and_moat_blocks_it() {
    let mut g = setup(&[Card::Enchantress, Card::Smithy, Card::Moat]);
    hand(&mut g, P0, &[Card::ThroneRoom, Card::Enchantress]);
    deck(&mut g, P0, &[Card::Copper; 12]);
    hand(&mut g, P1, &[Card::Moat, Card::Smithy]);
    deck(&mut g, P1, &[Card::Copper; 12]);
    g.start();
    play(&mut g, Card::ThroneRoom);
    choose_kinds(&mut g, &[Card::Enchantress]);
    trigger(&mut g, Card::Moat);
    trigger(&mut g, Card::Moat);
    next_turn(&mut g);
    play(&mut g, Card::Smithy);
    assert_eq!(g.hand(P1).len(), 4);
}

#[test]
fn an_enchanted_sacrifice_trashes_nothing_and_an_enchanted_duration_is_discarded() {
    let mut g = setup(&[Card::Enchantress, Card::Sacrifice, Card::Archive]);
    hand(&mut g, P0, &[Card::Enchantress]);
    deck(&mut g, P0, &[Card::Copper; 12]);
    hand(&mut g, P1, &[Card::Archive, Card::Gold]);
    deck(&mut g, P1, &[Card::Copper; 12]);
    g.start();
    play(&mut g, Card::Enchantress);
    next_turn(&mut g);
    play(&mut g, Card::Archive);
    assert!(g.player(P1).set_aside.is_empty());
    assert_eq!(g.hand(P1).len(), 2);
    assert_eq!(g.turn().actions, 1);
    next_turn(&mut g);
    assert!(g.in_play(P1).is_empty());
}

#[test]
fn engineer_gains_up_to_four_never_debt_and_may_trash_itself_for_another() {
    let mut g = setup(&[Card::Engineer, Card::Village, Card::Overlord]);
    hand(&mut g, P0, &[Card::Engineer]);
    g.start();
    play(&mut g, Card::Engineer);
    let DecisionKind::Pile { piles, .. } = &g.pending().unwrap().kind else {
        panic!("expected a pile choice");
    };
    let names: Vec<Card> = piles.iter().map(|&p| g.pile(p).card).collect();
    assert!(names.contains(&Card::Village) && names.contains(&Card::Silver));
    assert!(!names.contains(&Card::Engineer) && !names.contains(&Card::Overlord));
    choose_pile(&mut g, Card::Village);
    yes(&mut g);
    choose_pile(&mut g, Card::Village);
    assert_eq!(g.count_owned(P0, Card::Village), 2);
    assert_eq!(g.count_owned(P0, Card::Engineer), 0);
}

#[test]
fn a_throne_roomed_engineer_trashes_itself_once() {
    let mut g = setup(&[Card::Engineer, Card::ThroneRoom]);
    hand(&mut g, P0, &[Card::ThroneRoom, Card::Engineer]);
    g.start();
    play(&mut g, Card::ThroneRoom);
    choose_kinds(&mut g, &[Card::Engineer]);
    choose_pile(&mut g, Card::Silver);
    yes(&mut g);
    choose_pile(&mut g, Card::Silver);
    choose_pile(&mut g, Card::Silver);
    // No trash offer the second time: the Engineer is gone.
    assert_eq!(g.count_owned(P0, Card::Silver), 3);
    assert_eq!(purpose(&g), Purpose::Turn);
}

#[test]
fn farmers_market_gathers_four_times_then_pays_out_and_trashes_itself() {
    let mut g = setup(&[Card::FarmersMarket]);
    hand(&mut g, P0, &[Card::FarmersMarket; 5]);
    g.start();
    g.turn.actions = 5;
    let mut coins = vec![];
    for _ in 0..4 {
        let before = g.turn().coins;
        play(&mut g, Card::FarmersMarket);
        coins.push(g.turn().coins - before);
    }
    assert_eq!(coins, [1, 2, 3, 4]);
    play(&mut g, Card::FarmersMarket);
    assert_eq!(vp(&g, P0), 4);
    assert_eq!(pile_vp(&g, Card::FarmersMarket), 0);
    assert_eq!(g.count_owned(P0, Card::FarmersMarket), 4);
    assert_eq!(g.turn().buys, 6);
}

#[test]
fn forum_draws_three_discards_two_and_gives_a_buy_when_gained() {
    let mut g = setup(&[Card::Forum]);
    hand(&mut g, P0, &[Card::Forum, Card::Gold, Card::Gold]);
    deck(&mut g, P0, &[Card::Estate, Card::Estate, Card::Copper]);
    g.start();
    play(&mut g, Card::Forum);
    choose_kinds(&mut g, &[Card::Estate, Card::Estate]);
    assert_eq!(hand_kinds(&g, P0), [Card::Gold, Card::Gold, Card::Copper]);
    to_buy(&mut g);
    play_all(&mut g, Card::Gold);
    buy(&mut g, Card::Forum);
    assert_eq!(g.turn().buys, 1);
    assert!(can_buy(&g, Card::Copper));
}

#[test]
fn gladiator_gives_a_coin_and_trashes_a_gladiator_unless_the_left_player_has_a_copy() {
    let mut g = setup(&[Card::Gladiator, Card::Village]);
    hand(
        &mut g,
        P0,
        &[
            Card::Village,
            Card::Gladiator,
            Card::Gladiator,
            Card::Copper,
            Card::Estate,
        ],
    );
    hand(&mut g, P1, &[Card::Copper]);
    g.start();
    play(&mut g, Card::Village);
    play(&mut g, Card::Gladiator);
    choose_kinds(&mut g, &[Card::Estate]);
    assert_eq!(g.turn().coins, 3);
    assert_eq!(pile_kinds(&g, Card::Gladiator).len(), 9);
    assert!(kinds(&g, g.trash()).contains(&Card::Gladiator));
    play(&mut g, Card::Gladiator);
    choose_kinds(&mut g, &[Card::Copper]);
    assert_eq!(g.pending().unwrap().player, P1);
    choose_kinds(&mut g, &[Card::Copper]);
    assert_eq!(g.turn().coins, 5);
    assert_eq!(pile_kinds(&g, Card::Gladiator).len(), 9);
}

#[test]
fn fortune_doubles_once_a_turn_and_gains_a_gold_per_gladiator_in_play() {
    let mut g = setup(&[Card::Gladiator, Card::Village]);
    hand(
        &mut g,
        P0,
        &[
            Card::Village,
            Card::Gladiator,
            Card::Gladiator,
            Card::Fortune,
            Card::Fortune,
            Card::Gold,
        ],
    );
    hand(&mut g, P1, &[Card::Gladiator]);
    g.start();
    play(&mut g, Card::Village);
    play(&mut g, Card::Gladiator);
    choose_kinds(&mut g, &[Card::Gladiator]);
    choose_kinds(&mut g, &[Card::Gladiator]);
    play(&mut g, Card::Gladiator);
    choose_kinds(&mut g, &[Card::Gold]);
    assert_eq!(g.turn().coins, 5);
    to_buy(&mut g);
    play(&mut g, Card::Gold);
    play(&mut g, Card::Fortune);
    assert_eq!(g.turn().coins, 16);
    play(&mut g, Card::Fortune);
    assert_eq!(g.turn().coins, 16);
    assert_eq!(g.turn().buys, 3);
    strip(&mut g, Card::Gladiator, 4);
    buy(&mut g, Card::Fortune);
    assert_eq!(g.count_owned(P0, Card::Gold), 3);
    assert_eq!(g.player(P0).debt, 8);
}

#[test]
fn groundskeeper_gives_a_vp_per_victory_card_gained_this_turn_per_play() {
    let mut g = setup(&[Card::Groundskeeper, Card::ThroneRoom]);
    hand(
        &mut g,
        P0,
        &[
            Card::ThroneRoom,
            Card::Groundskeeper,
            Card::Gold,
            Card::Gold,
        ],
    );
    deck(&mut g, P0, &[Card::Copper; 5]);
    g.start();
    play(&mut g, Card::ThroneRoom);
    choose_kinds(&mut g, &[Card::Groundskeeper]);
    to_buy(&mut g);
    g.turn.buys = 2;
    play_all(&mut g, Card::Gold);
    buy(&mut g, Card::Estate);
    assert_eq!(vp(&g, P0), 2);
    buy(&mut g, Card::Silver);
    assert_eq!(vp(&g, P0), 2);
    next_turn(&mut g);
    next_turn(&mut g);
    assert!(g.lasting().is_empty());
}

#[test]
fn legionary_reveals_a_gold_to_make_the_others_discard_to_two_then_draw() {
    let mut g = setup(&[Card::Legionary]);
    hand(&mut g, P0, &[Card::Legionary, Card::Gold]);
    hand(
        &mut g,
        P1,
        &[
            Card::Copper,
            Card::Estate,
            Card::Silver,
            Card::Copper,
            Card::Copper,
        ],
    );
    deck(&mut g, P1, &[Card::Gold]);
    g.start();
    play(&mut g, Card::Legionary);
    assert_eq!(g.turn().coins, 3);
    choose_kinds(&mut g, &[Card::Gold]);
    choose_kinds(&mut g, &[Card::Copper, Card::Copper, Card::Estate]);
    assert_eq!(hand_kinds(&g, P1), [Card::Silver, Card::Copper, Card::Gold]);
}

#[test]
fn overlord_plays_a_supply_card_up_to_five_leaving_it_there() {
    let mut g = setup(&[
        Card::Overlord,
        Card::Village,
        Card::Archive,
        Card::CityQuarter,
        Card::Laboratory,
    ]);
    hand(&mut g, P0, &[Card::Overlord]);
    deck(&mut g, P0, &[Card::Copper; 5]);
    g.start();
    play(&mut g, Card::Overlord);
    let DecisionKind::Pile { piles, .. } = &g.pending().unwrap().kind else {
        panic!("expected a pile choice");
    };
    let names: Vec<Card> = piles.iter().map(|&p| g.pile(p).card).collect();
    assert_eq!(names.len(), 2);
    assert!(names.contains(&Card::Village) && names.contains(&Card::Laboratory));
    choose_pile(&mut g, Card::Laboratory);
    assert_eq!(g.hand(P0).len(), 2);
    assert_eq!(g.turn().actions, 1);
    assert_eq!(pile_kinds(&g, Card::Laboratory).len(), 10);
    assert_eq!(in_play_kinds(&g, P0), [Card::Overlord]);
}

#[test]
fn overlord_playing_encampment_or_farmers_market_cannot_move_them() {
    let mut g = setup(&[
        Card::Overlord,
        Card::Encampment,
        Card::FarmersMarket,
        Card::Village,
    ]);
    let fm = g.pile_of(Card::FarmersMarket).unwrap();
    g.piles[fm.index()].vp_tokens = 4;
    hand(&mut g, P0, &[Card::Village, Card::Overlord, Card::Overlord]);
    deck(&mut g, P0, &[Card::Copper; 5]);
    g.start();
    play(&mut g, Card::Village);
    play(&mut g, Card::Overlord);
    choose_pile(&mut g, Card::Encampment);
    assert_eq!(pile_kinds(&g, Card::Encampment).len(), 10);
    assert!(g.player(P0).set_aside.is_empty());
    play(&mut g, Card::Overlord);
    choose_pile(&mut g, Card::FarmersMarket);
    assert_eq!(vp(&g, P0), 4);
    assert_eq!(pile_kinds(&g, Card::FarmersMarket).len(), 10);
}

#[test]
fn patrician_takes_a_card_costing_five_or_more_but_not_a_debt_only_card() {
    let mut g = setup(&[Card::Patrician]);
    hand(&mut g, P0, &[Card::Patrician, Card::Patrician]);
    deck(
        &mut g,
        P0,
        &[Card::Copper, Card::Gold, Card::Overlord, Card::Copper],
    );
    g.start();
    play(&mut g, Card::Patrician);
    assert_eq!(
        hand_kinds(&g, P0),
        [Card::Patrician, Card::Copper, Card::Gold]
    );
    play(&mut g, Card::Patrician);
    assert_eq!(deck_kinds(&g, P0), [Card::Copper]);
    assert!(hand_kinds(&g, P0).contains(&Card::Overlord));
    assert_eq!(g.hand(P0).len(), 3);
}

#[test]
fn emporium_gives_two_vp_when_gained_with_five_actions_in_play() {
    for (villages, want) in [(4, 0), (5, 2)] {
        let mut g = setup(&[Card::Patrician, Card::Village]);
        strip(&mut g, Card::Patrician, 5);
        let mut cards = vec![Card::Village; villages];
        cards.extend([Card::Gold, Card::Gold]);
        hand(&mut g, P0, &cards);
        g.start();
        for _ in 0..villages {
            play(&mut g, Card::Village);
        }
        to_buy(&mut g);
        play_all(&mut g, Card::Gold);
        buy(&mut g, Card::Emporium);
        assert_eq!(vp(&g, P0), want, "{villages} Villages");
    }
}

#[test]
fn plunder_gives_a_vp_each_play() {
    let mut g = setup(&[Card::Encampment, Card::Crown]);
    hand(&mut g, P0, &[Card::Crown, Card::Plunder]);
    g.start();
    to_buy(&mut g);
    play(&mut g, Card::Crown);
    choose_kinds(&mut g, &[Card::Plunder]);
    assert_eq!(g.turn().coins, 4);
    assert_eq!(vp(&g, P0), 2);
}

#[test]
fn royal_blacksmith_draws_five_and_discards_every_copper() {
    let mut g = setup(&[Card::RoyalBlacksmith]);
    hand(
        &mut g,
        P0,
        &[Card::RoyalBlacksmith, Card::Copper, Card::Estate],
    );
    deck(
        &mut g,
        P0,
        &[
            Card::Copper,
            Card::Silver,
            Card::Copper,
            Card::Gold,
            Card::Duchy,
        ],
    );
    g.start();
    play(&mut g, Card::RoyalBlacksmith);
    assert_eq!(
        hand_kinds(&g, P0),
        [Card::Estate, Card::Silver, Card::Gold, Card::Duchy]
    );
    assert_eq!(g.player(P0).discard.len(), 3);
}

#[test]
fn sacrifice_gives_a_bonus_per_type_of_the_trashed_card() {
    let mut g = setup(&[Card::Sacrifice, Card::Crown]);
    hand(
        &mut g,
        P0,
        &[
            Card::Sacrifice,
            Card::Sacrifice,
            Card::Sacrifice,
            Card::Crown,
            Card::Estate,
            Card::Curse,
        ],
    );
    deck(&mut g, P0, &[Card::Copper; 5]);
    g.start();
    play(&mut g, Card::Sacrifice);
    choose_kinds(&mut g, &[Card::Crown]);
    assert_eq!((g.turn().actions, g.turn().coins), (2, 2));
    assert_eq!(g.hand(P0).len(), 6);
    play(&mut g, Card::Sacrifice);
    choose_kinds(&mut g, &[Card::Estate]);
    assert_eq!(vp(&g, P0), 2);
    play(&mut g, Card::Sacrifice);
    choose_kinds(&mut g, &[Card::Curse]);
    assert_eq!((g.turn().actions, g.turn().coins, vp(&g, P0)), (0, 2, 2));
}

#[test]
fn settlers_and_bustling_village_take_a_copper_or_a_settlers_from_the_discard_pile() {
    let mut g = setup(&[Card::Settlers]);
    hand(&mut g, P0, &[Card::Settlers, Card::BustlingVillage]);
    deck(&mut g, P0, &[Card::Estate, Card::Estate]);
    discard(&mut g, P0, &[Card::Copper, Card::Settlers, Card::Silver]);
    g.start();
    play(&mut g, Card::Settlers);
    choose_kinds(&mut g, &[Card::Copper]);
    play(&mut g, Card::BustlingVillage);
    choose_kinds(&mut g, &[Card::Settlers]);
    assert_eq!(
        hand_kinds(&g, P0),
        [Card::Estate, Card::Copper, Card::Estate, Card::Settlers]
    );
    assert_eq!(g.turn().actions, 3);
}

#[test]
fn temple_trashes_differently_named_cards_and_gaining_it_takes_the_pile() {
    let mut g = setup(&[Card::Temple]);
    hand(
        &mut g,
        P0,
        &[
            Card::Temple,
            Card::Copper,
            Card::Copper,
            Card::Estate,
            Card::Curse,
            Card::Gold,
            Card::Silver,
        ],
    );
    g.start();
    play(&mut g, Card::Temple);
    choose_kinds(&mut g, &[Card::Copper]);
    assert!(!kinds(&g, &candidates(&g)).contains(&Card::Copper));
    choose_kinds(&mut g, &[Card::Estate]);
    choose_kinds(&mut g, &[Card::Curse]);
    assert_eq!(hand_kinds(&g, P0), [Card::Copper, Card::Gold, Card::Silver]);
    assert_eq!(vp(&g, P0), 1);
    assert_eq!(pile_vp(&g, Card::Temple), 1);
    to_buy(&mut g);
    play_all(&mut g, Card::Gold);
    play_all(&mut g, Card::Silver);
    buy(&mut g, Card::Temple);
    assert_eq!(vp(&g, P0), 2);
    assert_eq!(pile_vp(&g, Card::Temple), 0);
}

#[test]
fn temple_adds_its_token_even_with_an_empty_hand() {
    let mut g = setup(&[Card::Temple]);
    hand(&mut g, P0, &[Card::Temple]);
    g.start();
    play(&mut g, Card::Temple);
    assert_eq!(pile_vp(&g, Card::Temple), 1);
}

#[test]
fn villa_bought_goes_to_hand_and_returns_to_the_action_phase() {
    let mut g = setup_landscapes(&[Card::Villa], &[Card::Arena]);
    hand(
        &mut g,
        P0,
        &[Card::Gold, Card::Copper, Card::Silver, Card::Estate],
    );
    deck(&mut g, P0, &[Card::Copper; 5]);
    g.start();
    to_buy(&mut g);
    play(&mut g, Card::Gold);
    play(&mut g, Card::Copper);
    buy(&mut g, Card::Villa);
    assert_eq!(phase(&g), Phase::Action);
    assert_eq!(g.turn().actions, 2);
    // Keep an Action card in hand for Arena.
    let village = g.create(Card::Village, Zone::Hand(P0));
    play(&mut g, Card::Villa);
    assert_eq!(g.turn().buys, 1);
    end_phase(&mut g);
    // A new Buy phase: Arena is offered again, and Treasures can be played again.
    assert_eq!(purpose(&g), Purpose::Trigger);
    respond(&mut g, Response::option(Opt::No));
    assert_eq!(phase(&g), Phase::Buy);
    assert!(g.hand(P0).contains(&village));
    play(&mut g, Card::Silver);
    assert_eq!(g.turn().coins, 3);
}

#[test]
fn villa_gained_in_the_action_phase_stays_in_it() {
    let mut g = setup(&[Card::Villa, Card::Engineer]);
    hand(&mut g, P0, &[Card::Engineer]);
    g.start();
    play(&mut g, Card::Engineer);
    choose_pile(&mut g, Card::Villa);
    no(&mut g);
    assert_eq!(hand_kinds(&g, P0), [Card::Villa]);
    assert_eq!(g.turn().actions, 1);
    play(&mut g, Card::Villa);
}

#[test]
fn wild_hunt_draws_and_adds_a_token_or_gains_an_estate_and_takes_them() {
    let mut g = setup(&[Card::WildHunt, Card::Village]);
    hand(&mut g, P0, &[Card::Village, Card::WildHunt, Card::WildHunt]);
    deck(&mut g, P0, &[Card::Copper; 5]);
    g.start();
    play(&mut g, Card::Village);
    play(&mut g, Card::WildHunt);
    modes(&mut g, &[0]);
    assert_eq!(g.hand(P0).len(), 5);
    assert_eq!(pile_vp(&g, Card::WildHunt), 1);
    play(&mut g, Card::WildHunt);
    modes(&mut g, &[1]);
    assert_eq!(vp(&g, P0), 1);
    assert_eq!(pile_vp(&g, Card::WildHunt), 0);
    assert_eq!(g.count_owned(P0, Card::Estate), 1);
}
