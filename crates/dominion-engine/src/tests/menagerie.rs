use super::intrigue::{modes, random_games_with_copies, setup3, trigger, trigger_labels};
use super::*;

fn landscapes_of(kind: Kind) -> Vec<Card> {
    Card::in_set(Set::Menagerie)
        .filter(|c| c.kind() == kind)
        .collect()
}

fn setup_with_landscapes(kingdom: &[Card], landscapes: &[Card]) -> Game {
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

fn phase(g: &Game) -> Option<Phase> {
    match g.pending().map(|d| &d.kind) {
        Some(DecisionKind::Turn { phase, .. }) => Some(*phase),
        _ => None,
    }
}

/// Uses `w` for the Action being played.
fn way(g: &mut Game, w: Card) {
    assert_eq!(purpose(g), Purpose::Mode);
    respond(g, Response::option(Opt::Card(w)));
}

/// Plays the Action being played for its own effect rather than a Way.
fn own(g: &mut Game) {
    assert_eq!(purpose(g), Purpose::Mode);
    no(g);
}

fn buy_event(g: &mut Game, e: Card) {
    to_buy(g);
    respond(g, Response::Move(Move::BuyLandscape(e)));
}

fn can_buy_event(g: &Game, e: Card) -> bool {
    match &g.pending().unwrap().kind {
        DecisionKind::Turn { moves, .. } => moves.contains(&Move::BuyLandscape(e)),
        _ => false,
    }
}

fn can_buy(g: &Game, card: Card) -> bool {
    let Some(pile) = g.supply_pile(card) else {
        return false;
    };
    match &g.pending().unwrap().kind {
        DecisionKind::Turn { moves, .. } => moves.contains(&Move::Buy(pile)),
        _ => false,
    }
}

/// Resolves pending triggers in the order offered.
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

fn exile_kinds(g: &Game, p: PlayerId) -> Vec<Card> {
    kinds(g, g.player(p).mat(Mat::Exile))
}

fn pile_len(g: &Game, card: Card) -> usize {
    g.pile(g.pile_of(card).unwrap()).cards.len()
}

fn gained(g: &Game, p: PlayerId) -> Vec<Card> {
    g.log()
        .iter()
        .filter_map(|e| match e {
            Event::Gained { player, card, .. } if *player == p => Some(*card),
            _ => None,
        })
        .collect()
}

/// Puts `cards` on `p`'s Exile mat.
fn exiled(g: &mut Game, p: PlayerId, cards: &[Card]) -> Vec<CardId> {
    replace(g, Zone::Mat(p, Mat::Exile), cards)
}

fn empty_pile(g: &mut Game, card: Card) {
    let pile = g.supply_pile(card).unwrap();
    for c in g.pile(pile).cards.clone() {
        g.move_card(c, Zone::Trash, crate::game::Pos::Top);
    }
}

fn play_treasures(g: &mut Game) {
    to_buy(g);
    let p = g.turn().player;
    for c in g.cards_in_hand_where(p, Types::TREASURE) {
        respond(g, Response::Move(Move::Play(c)));
    }
}

// ---- Random games ----

/// Random games over Menagerie, each with one of its Ways and one of its Events.
#[test]
fn random_menagerie_games_keep_invariants() {
    let pool: Vec<Card> = Card::in_set(Set::Menagerie)
        .filter(|c| c.def().kingdom)
        .collect();
    let ways = landscapes_of(Kind::Way);
    let events = landscapes_of(Kind::Event);
    random_games_with_copies(&pool, &[Card::Horse], 0..160, |seed| {
        let i = seed as usize;
        vec![ways[i % ways.len()], events[(i / 2) % events.len()]]
    });
}

// ---- Horse, Exile ----

#[test]
fn the_horse_pile_is_out_of_the_supply_and_only_in_games_that_use_it() {
    let g = setup(&[Card::Sleigh]);
    let horses = g.pile_of(Card::Horse).expect("a Horse pile");
    assert!(!g.pile(horses).in_supply);
    assert_eq!(g.pile(horses).cards.len(), 30);
    assert!(setup(&[Card::Smithy]).pile_of(Card::Horse).is_none());
    let g = setup_with_landscapes(&[Card::Smithy], &[Card::Ride]);
    assert!(g.pile_of(Card::Horse).is_some());
}

#[test]
fn horse_draws_two_gives_an_action_and_returns_to_its_pile() {
    let mut g = setup(&[Card::Sleigh]);
    hand(&mut g, P0, &[Card::Horse]);
    deck(&mut g, P0, &[Card::Copper; 5]);
    g.start();
    play(&mut g, Card::Horse);
    assert_eq!(g.hand(P0).len(), 2);
    assert_eq!(g.turn().actions, 1);
    assert!(g.in_play(P0).is_empty());
    assert_eq!(pile_len(&g, Card::Horse), 31);
}

#[test]
fn a_falconer_cannot_gain_a_horse_from_its_non_supply_pile() {
    let mut g = setup(&[Card::Falconer, Card::Sleigh]);
    hand(&mut g, P0, &[Card::Falconer]);
    g.start();
    play(&mut g, Card::Falconer);
    let DecisionKind::Pile { piles, .. } = &g.pending().unwrap().kind else {
        panic!("expected a pile choice");
    };
    assert!(!piles.contains(&g.pile_of(Card::Horse).unwrap()));
}

#[test]
fn gaining_a_card_lets_you_discard_its_copies_from_exile() {
    let mut g = setup(&[Card::Sanctuary]);
    exiled(&mut g, P0, &[Card::Silver, Card::Silver, Card::Estate]);
    hand(&mut g, P0, &[Card::Gold]);
    g.start();
    g.turn.buys = 2;
    play_treasures(&mut g);
    buy(&mut g, Card::Silver);
    assert_eq!(purpose(&g), Purpose::Trigger);
    trigger(&mut g, Card::Sanctuary);
    assert_eq!(exile_kinds(&g, P0), [Card::Estate]);
    assert_eq!(discard_kinds(&g, P0), [Card::Silver; 3]);
}

#[test]
fn copies_may_stay_in_exile_and_still_score() {
    let mut g = setup(&[Card::Sanctuary]);
    exiled(&mut g, P0, &[Card::Estate, Card::Duchy]);
    hand(&mut g, P0, &[Card::Silver, Card::Copper]);
    g.start();
    let before = g.score(P0);
    g.turn.buys = 2;
    play_treasures(&mut g);
    buy(&mut g, Card::Estate);
    no(&mut g);
    assert_eq!(exile_kinds(&g, P0), [Card::Estate, Card::Duchy]);
    assert_eq!(g.score(P0), before + 1);
    assert_eq!(
        g.view(P1).card(g.player(P0).mat(Mat::Exile)[0]),
        Some(Card::Estate)
    );
}

// ---- Kingdom cards ----

#[test]
fn animal_fair_gives_a_buy_per_empty_supply_pile() {
    let mut g = setup(&[Card::AnimalFair, Card::Smithy, Card::Village]);
    empty_pile(&mut g, Card::Smithy);
    empty_pile(&mut g, Card::Village);
    hand(&mut g, P0, &[Card::AnimalFair]);
    g.start();
    play(&mut g, Card::AnimalFair);
    assert_eq!((g.turn().coins, g.turn().buys), (4, 3));
}

#[test]
fn animal_fair_can_be_bought_by_trashing_an_action_instead_of_paying() {
    let mut g = setup(&[Card::AnimalFair, Card::Village]);
    hand(&mut g, P0, &[Card::Village, Card::Copper]);
    g.start();
    g.turn.buys = 2;
    play_treasures(&mut g);
    assert!(can_buy(&g, Card::AnimalFair));
    buy(&mut g, Card::AnimalFair);
    assert_eq!(g.turn().coins, 1);
    assert_eq!(kinds(&g, g.trash()).last(), Some(&Card::Village));
    assert_eq!(gained(&g, P0), [Card::AnimalFair]);
}

#[test]
fn animal_fair_needs_its_cost_or_an_action_in_hand() {
    let mut g = setup(&[Card::AnimalFair, Card::Village]);
    hand(&mut g, P0, &[Card::Gold, Card::Gold]);
    g.start();
    play_treasures(&mut g);
    assert!(!can_buy(&g, Card::AnimalFair));
}

#[test]
fn animal_fair_with_its_cost_in_hand_may_still_be_paid_by_trashing() {
    let mut g = setup(&[Card::AnimalFair, Card::Village]);
    hand(
        &mut g,
        P0,
        &[Card::Village, Card::Gold, Card::Gold, Card::Copper],
    );
    g.start();
    g.turn.buys = 2;
    play_treasures(&mut g);
    buy(&mut g, Card::AnimalFair);
    assert_eq!(purpose(&g), Purpose::Trash);
    no(&mut g);
    assert_eq!(g.turn().coins, 0);
    assert_eq!(hand_kinds(&g, P0), [Card::Village]);
}

#[test]
fn barge_now_draws_three_and_gives_a_buy() {
    let mut g = setup(&[Card::Barge]);
    hand(&mut g, P0, &[Card::Barge]);
    deck(&mut g, P0, &[Card::Copper; 10]);
    g.start();
    play(&mut g, Card::Barge);
    modes(&mut g, &[0]);
    assert_eq!((g.hand(P0).len(), g.turn().buys), (3, 2));
    next_turn(&mut g);
    assert!(g.in_play(P0).is_empty());
}

#[test]
fn barge_next_turn_stays_in_play_and_draws_then() {
    let mut g = setup(&[Card::Barge]);
    hand(&mut g, P0, &[Card::Barge]);
    deck(&mut g, P0, &[Card::Copper; 20]);
    g.start();
    play(&mut g, Card::Barge);
    modes(&mut g, &[1]);
    assert_eq!(g.hand(P0).len(), 0);
    next_turn(&mut g);
    assert_eq!(in_play_kinds(&g, P0), [Card::Barge]);
    next_turn(&mut g);
    assert_eq!((g.hand(P0).len(), g.turn().buys), (8, 2));
    next_turn(&mut g);
    assert!(g.in_play(P0).is_empty());
}

#[test]
fn black_cat_on_its_owners_turn_draws_two_and_curses_nobody() {
    let mut g = setup(&[Card::BlackCat]);
    hand(&mut g, P0, &[Card::BlackCat]);
    g.start();
    play(&mut g, Card::BlackCat);
    assert_eq!(g.hand(P0).len(), 2);
    assert!(gained(&g, P1).is_empty());
}

#[test]
fn black_cat_reacts_to_another_players_victory_gain_and_curses_from_the_turns_player() {
    let mut g = setup3(&[Card::BlackCat]);
    let curses = g.supply_pile(Card::Curse).unwrap();
    for c in g.pile(curses).cards.clone().into_iter().skip(1) {
        g.move_card(c, Zone::Trash, crate::game::Pos::Top);
    }
    hand(&mut g, P0, &[Card::Copper, Card::Copper]);
    hand(&mut g, P1, &[Card::BlackCat]);
    g.start();
    play_treasures(&mut g);
    buy(&mut g, Card::Estate);
    assert_eq!(g.pending().unwrap().player, P1);
    trigger(&mut g, Card::BlackCat);
    assert_eq!(g.hand(P1).len(), 2);
    // Curses go in turn order from the player whose turn it is: P0 gets the last one.
    assert_eq!(gained(&g, P0), [Card::Estate, Card::Curse]);
    assert!(gained(&g, P2).is_empty());
}

#[test]
fn black_cat_ignores_cards_that_are_not_victory_cards() {
    let mut g = setup(&[Card::BlackCat]);
    hand(&mut g, P0, &[Card::Copper, Card::Copper, Card::Copper]);
    hand(&mut g, P1, &[Card::BlackCat]);
    g.start();
    play_treasures(&mut g);
    buy(&mut g, Card::Silver);
    assert_eq!(g.pending().unwrap().player, P1);
}

#[test]
fn black_cat_played_out_of_turn_is_answered_by_moat() {
    let mut g = setup(&[Card::BlackCat, Card::Moat]);
    hand(&mut g, P0, &[Card::Copper, Card::Copper, Card::Moat]);
    hand(&mut g, P1, &[Card::BlackCat]);
    g.start();
    play_treasures(&mut g);
    buy(&mut g, Card::Estate);
    trigger(&mut g, Card::BlackCat);
    assert_eq!(g.pending().unwrap().player, P0);
    trigger(&mut g, Card::Moat);
    assert_eq!(gained(&g, P0), [Card::Estate]);
}

#[test]
fn bounty_hunter_gives_three_only_for_a_card_with_no_copy_in_exile() {
    let mut g = setup(&[Card::BountyHunter]);
    exiled(&mut g, P0, &[Card::Estate]);
    hand(
        &mut g,
        P0,
        &[
            Card::BountyHunter,
            Card::BountyHunter,
            Card::Estate,
            Card::Copper,
        ],
    );
    g.start();
    play(&mut g, Card::BountyHunter);
    choose_kinds(&mut g, &[Card::Estate]);
    assert_eq!((g.turn().coins, g.turn().actions), (0, 1));
    play(&mut g, Card::BountyHunter);
    assert_eq!(g.turn().coins, 3);
    assert_eq!(
        exile_kinds(&g, P0),
        [Card::Estate, Card::Estate, Card::Copper]
    );
}

#[test]
fn camel_train_exiles_a_non_victory_card_and_a_gold_when_gained() {
    let mut g = setup(&[Card::CamelTrain, Card::Smithy]);
    hand(
        &mut g,
        P0,
        &[Card::CamelTrain, Card::Copper, Card::Copper, Card::Copper],
    );
    g.start();
    play(&mut g, Card::CamelTrain);
    let DecisionKind::Pile { piles, .. } = &g.pending().unwrap().kind else {
        panic!("expected a pile choice");
    };
    assert!(!piles.contains(&g.supply_pile(Card::Estate).unwrap()));
    choose_pile(&mut g, Card::Smithy);
    assert_eq!(exile_kinds(&g, P0), [Card::Smithy]);
    play_treasures(&mut g);
    buy(&mut g, Card::CamelTrain);
    assert_eq!(exile_kinds(&g, P0), [Card::Smithy, Card::Gold]);
    assert_eq!(gained(&g, P0), [Card::CamelTrain]);
}

#[test]
fn cardinal_exiles_a_card_costing_three_to_six_and_discards_the_rest() {
    let mut g = setup(&[Card::Cardinal]);
    hand(&mut g, P0, &[Card::Cardinal]);
    deck(&mut g, P1, &[Card::Silver, Card::Gold, Card::Copper]);
    g.start();
    play(&mut g, Card::Cardinal);
    assert_eq!(g.pending().unwrap().player, P1);
    choose_kinds(&mut g, &[Card::Gold]);
    assert_eq!(g.turn().coins, 2);
    assert_eq!(exile_kinds(&g, P1), [Card::Gold]);
    assert_eq!(discard_kinds(&g, P1), [Card::Silver]);
    assert_eq!(deck_kinds(&g, P1), [Card::Copper]);
}

#[test]
fn cardinal_discards_both_cards_when_neither_costs_three_to_six() {
    let mut g = setup(&[Card::Cardinal]);
    hand(&mut g, P0, &[Card::Cardinal]);
    deck(&mut g, P1, &[Card::Copper, Card::Province]);
    g.start();
    play(&mut g, Card::Cardinal);
    assert!(exile_kinds(&g, P1).is_empty());
    assert_eq!(discard_kinds(&g, P1), [Card::Copper, Card::Province]);
}

#[test]
fn cavalry_gains_two_horses() {
    let mut g = setup(&[Card::Cavalry]);
    hand(&mut g, P0, &[Card::Cavalry]);
    g.start();
    play(&mut g, Card::Cavalry);
    assert_eq!(gained(&g, P0), [Card::Horse, Card::Horse]);
}

#[test]
fn cavalry_bought_draws_two_gives_a_buy_and_returns_to_the_action_phase() {
    let mut g = setup(&[Card::Cavalry, Card::Village]);
    hand(&mut g, P0, &[Card::Silver, Card::Silver]);
    deck(&mut g, P0, &[Card::Village, Card::Copper, Card::Copper]);
    g.start();
    play_treasures(&mut g);
    buy(&mut g, Card::Cavalry);
    assert_eq!(phase(&g), Some(Phase::Action));
    assert_eq!(hand_kinds(&g, P0), [Card::Village, Card::Copper]);
    assert_eq!((g.turn().actions, g.turn().buys), (1, 1));
    play(&mut g, Card::Village);
    // A new Buy phase: Treasures can be played again.
    assert_eq!(phase(&g), Some(Phase::Buy));
    let copper = find(&g, P0, Card::Copper);
    respond(&mut g, Response::Move(Move::Play(copper)));
    assert_eq!(g.turn().coins, 1);
}

#[test]
fn cavalry_gained_in_the_action_phase_draws_and_gives_a_buy_but_stays_there() {
    let mut g = setup(&[Card::Cavalry, Card::Workshop]);
    hand(&mut g, P0, &[Card::Workshop]);
    deck(&mut g, P0, &[Card::Copper; 5]);
    g.start();
    play(&mut g, Card::Workshop);
    choose_pile(&mut g, Card::Cavalry);
    assert_eq!(g.hand(P0).len(), 2);
    assert_eq!(g.turn().buys, 2);
    assert_eq!(phase(&g), Some(Phase::Buy));
}

#[test]
fn coven_exiles_curses_and_when_none_are_left_its_victims_discard_theirs() {
    let mut g = setup(&[Card::Coven]);
    hand(&mut g, P0, &[Card::Coven, Card::Coven]);
    exiled(&mut g, P1, &[Card::Curse, Card::Curse]);
    g.start();
    play(&mut g, Card::Coven);
    assert_eq!((g.turn().actions, g.turn().coins), (1, 2));
    assert_eq!(exile_kinds(&g, P1), [Card::Curse; 3]);
    empty_pile(&mut g, Card::Curse);
    play(&mut g, Card::Coven);
    assert!(exile_kinds(&g, P1).is_empty());
    assert_eq!(discard_kinds(&g, P1), [Card::Curse; 3]);
    assert!(gained(&g, P1).is_empty());
}

#[test]
fn displace_exiles_a_card_and_gains_a_differently_named_one_up_to_two_more() {
    let mut g = setup(&[Card::Displace, Card::Smithy, Card::Village]);
    hand(&mut g, P0, &[Card::Displace, Card::Silver]);
    g.start();
    play(&mut g, Card::Displace);
    let DecisionKind::Pile { piles, .. } = &g.pending().unwrap().kind else {
        panic!("expected a pile choice");
    };
    assert!(!piles.contains(&g.supply_pile(Card::Silver).unwrap()));
    assert!(piles.contains(&g.supply_pile(Card::Displace).unwrap()));
    assert!(!piles.contains(&g.supply_pile(Card::Gold).unwrap()));
    choose_pile(&mut g, Card::Village);
    assert_eq!(exile_kinds(&g, P0), [Card::Silver]);
    assert_eq!(gained(&g, P0), [Card::Village]);
}

#[test]
fn falconer_gains_a_cheaper_card_to_hand() {
    let mut g = setup(&[Card::Falconer, Card::Smithy]);
    hand(&mut g, P0, &[Card::Falconer]);
    g.start();
    play(&mut g, Card::Falconer);
    let DecisionKind::Pile { piles, .. } = &g.pending().unwrap().kind else {
        panic!("expected a pile choice");
    };
    assert!(!piles.contains(&g.supply_pile(Card::Falconer).unwrap()));
    choose_pile(&mut g, Card::Smithy);
    assert_eq!(hand_kinds(&g, P0), [Card::Smithy]);
}

#[test]
fn falconer_reacts_when_anyone_gains_a_card_with_two_types() {
    let mut g = setup(&[Card::Falconer, Card::Mill, Card::Village]);
    hand(&mut g, P0, &[Card::Silver, Card::Silver, Card::Falconer]);
    hand(&mut g, P1, &[Card::Falconer]);
    g.start();
    play_treasures(&mut g);
    buy(&mut g, Card::Mill);
    // The turn's player reacts first.
    assert_eq!(g.pending().unwrap().player, P0);
    trigger(&mut g, Card::Falconer);
    choose_pile(&mut g, Card::Village);
    assert_eq!(g.pending().unwrap().player, P1);
    trigger(&mut g, Card::Falconer);
    choose_pile(&mut g, Card::Village);
    assert_eq!(hand_kinds(&g, P1), [Card::Village]);
}

#[test]
fn falconer_does_not_react_to_a_card_with_one_type() {
    let mut g = setup(&[Card::Falconer]);
    hand(&mut g, P0, &[Card::Silver, Card::Copper, Card::Falconer]);
    g.start();
    play_treasures(&mut g);
    buy(&mut g, Card::Silver);
    assert_eq!(g.pending().unwrap().player, P1);
}

#[test]
fn fisherman_costs_three_less_while_the_turn_players_discard_pile_is_empty() {
    let mut g = setup(&[Card::Fisherman]);
    hand(&mut g, P0, &[Card::Copper, Card::Copper]);
    discard(&mut g, P1, &[Card::Copper]);
    g.start();
    assert_eq!(g.cost(Card::Fisherman), Cost::coins(2));
    play_treasures(&mut g);
    buy(&mut g, Card::Fisherman);
    assert_eq!(g.cost(Card::Fisherman), Cost::coins(5));
}

#[test]
fn two_fishermen_cannot_both_be_bought_for_two() {
    let mut g = setup(&[Card::Fisherman]);
    hand(&mut g, P0, &[Card::Silver, Card::Copper, Card::Copper]);
    g.start();
    g.turn.buys = 2;
    play_treasures(&mut g);
    buy(&mut g, Card::Fisherman);
    assert!(!can_buy(&g, Card::Fisherman));
}

#[test]
fn destrier_costs_one_less_per_card_the_turns_player_gained() {
    let mut g = setup(&[Card::Destrier, Card::Sleigh, Card::Witch]);
    hand(&mut g, P0, &[Card::Sleigh, Card::Witch]);
    g.start();
    g.turn.actions = 2;
    assert_eq!(g.cost(Card::Destrier), Cost::coins(6));
    play(&mut g, Card::Sleigh);
    assert_eq!(g.cost(Card::Destrier), Cost::coins(4));
    play(&mut g, Card::Witch);
    assert_eq!(g.cost(Card::Destrier), Cost::coins(4));
}

#[test]
fn gatekeeper_exiles_actions_and_treasures_others_gain_until_its_owners_next_turn() {
    let mut g = setup(&[Card::Gatekeeper, Card::Village]);
    hand(&mut g, P0, &[Card::Gatekeeper]);
    hand(&mut g, P1, &[Card::Gold, Card::Gold, Card::Gold]);
    g.start();
    play(&mut g, Card::Gatekeeper);
    next_turn(&mut g);
    g.turn.buys = 4;
    play_treasures(&mut g);
    buy(&mut g, Card::Silver);
    assert_eq!(exile_kinds(&g, P1), [Card::Silver]);
    buy(&mut g, Card::Estate);
    assert_eq!(exile_kinds(&g, P1), [Card::Silver]);
    // Having an exiled Silver, the next one is not Exiled and may bring it back.
    buy(&mut g, Card::Silver);
    trigger(&mut g, Card::Sanctuary);
    assert!(exile_kinds(&g, P1).is_empty());
    next_turn(&mut g);
    assert_eq!(g.turn().coins, 3);
    assert!(g.lasting().is_empty());
}

#[test]
fn gatekeeper_exiles_the_first_of_two_horses_but_not_the_second() {
    let mut g = setup(&[Card::Gatekeeper, Card::Sleigh]);
    hand(&mut g, P0, &[Card::Gatekeeper]);
    hand(&mut g, P1, &[Card::Sleigh]);
    g.start();
    play(&mut g, Card::Gatekeeper);
    next_turn(&mut g);
    play(&mut g, Card::Sleigh);
    assert_eq!(exile_kinds(&g, P1), [Card::Horse]);
    // The second Horse finds a copy in Exile: P1 may discard it.
    trigger(&mut g, Card::Sanctuary);
    assert!(exile_kinds(&g, P1).is_empty());
    assert_eq!(discard_kinds(&g, P1), [Card::Horse, Card::Horse]);
}

#[test]
fn a_sleigh_beats_gatekeeper_by_moving_the_gained_card_first() {
    let mut g = setup(&[Card::Gatekeeper, Card::Sleigh]);
    hand(&mut g, P0, &[Card::Gatekeeper]);
    hand(&mut g, P1, &[Card::Sleigh, Card::Gold]);
    g.start();
    play(&mut g, Card::Gatekeeper);
    next_turn(&mut g);
    g.turn.buys = 2;
    play_treasures(&mut g);
    buy(&mut g, Card::Silver);
    trigger(&mut g, Card::Sleigh);
    modes(&mut g, &[0]);
    assert!(exile_kinds(&g, P1).is_empty());
    assert!(hand_kinds(&g, P1).contains(&Card::Silver));
}

#[test]
fn moat_protects_from_gatekeeper() {
    let mut g = setup(&[Card::Gatekeeper, Card::Moat]);
    hand(&mut g, P0, &[Card::Gatekeeper]);
    hand(&mut g, P1, &[Card::Moat, Card::Gold]);
    g.start();
    play(&mut g, Card::Gatekeeper);
    trigger(&mut g, Card::Moat);
    next_turn(&mut g);
    to_buy(&mut g);
    play_treasures(&mut g);
    buy(&mut g, Card::Silver);
    assert!(exile_kinds(&g, P1).is_empty());
}

#[test]
fn gatekeeper_works_on_the_turn_it_is_played() {
    let mut g = setup_with_landscapes(&[Card::Gatekeeper], &[Card::Bargain]);
    hand(&mut g, P0, &[Card::Gatekeeper, Card::Gold, Card::Copper]);
    g.start();
    play(&mut g, Card::Gatekeeper);
    play_treasures(&mut g);
    buy_event(&mut g, Card::Bargain);
    choose_pile(&mut g, Card::Silver);
    assert_eq!(exile_kinds(&g, P1), [Card::Horse]);
}

#[test]
fn goatherd_draws_a_card_per_card_the_player_to_the_right_trashed_last_turn() {
    let mut g = setup(&[Card::Goatherd, Card::Chapel]);
    hand(&mut g, P0, &[Card::Copper; 5]);
    deck(
        &mut g,
        P0,
        &[
            Card::Goatherd,
            Card::Estate,
            Card::Estate,
            Card::Estate,
            Card::Estate,
        ]
        .into_iter()
        .chain([Card::Copper; 10])
        .collect::<Vec<_>>(),
    );
    hand(
        &mut g,
        P1,
        &[Card::Chapel, Card::Estate, Card::Estate, Card::Copper],
    );
    g.start();
    next_turn(&mut g);
    play(&mut g, Card::Chapel);
    choose_kinds(&mut g, &[Card::Estate, Card::Estate, Card::Copper]);
    next_turn(&mut g);
    play(&mut g, Card::Goatherd);
    choose_kinds(&mut g, &[Card::Estate]);
    // +1 Action, then 3 cards for P1's 3 trashes.
    assert_eq!(g.turn().actions, 1);
    assert_eq!(g.hand(P0).len(), 3 + 3);
}

#[test]
fn goatherd_draws_nothing_if_the_player_to_the_right_trashed_nothing() {
    let mut g = setup(&[Card::Goatherd]);
    hand(&mut g, P0, &[Card::Goatherd, Card::Estate]);
    g.start();
    play(&mut g, Card::Goatherd);
    choose(&mut g, &[]);
    assert_eq!(hand_kinds(&g, P0), [Card::Estate]);
}

#[test]
fn groom_gives_a_bonus_per_type_of_the_gained_card() {
    let mut g = setup(&[Card::Groom, Card::Mill, Card::Village]);
    hand(&mut g, P0, &[Card::Groom, Card::Groom, Card::Groom]);
    deck(&mut g, P0, &[Card::Copper; 5]);
    g.start();
    g.turn.actions = 3;
    play(&mut g, Card::Groom);
    choose_pile(&mut g, Card::Mill);
    // Mill is an Action and a Victory card: a Horse, then +1 Card +1 Action.
    assert_eq!(gained(&g, P0), [Card::Mill, Card::Horse]);
    assert_eq!(g.turn().actions, 3);
    assert_eq!(g.hand(P0).len(), 3);
    play(&mut g, Card::Groom);
    choose_pile(&mut g, Card::Silver);
    assert_eq!(gained(&g, P0)[2..], [Card::Silver, Card::Silver]);
    play(&mut g, Card::Groom);
    choose_pile(&mut g, Card::Estate);
    assert_eq!(gained(&g, P0)[4..], [Card::Estate]);
    assert_eq!((g.turn().actions, g.hand(P0).len()), (2, 2));
}

#[test]
fn hostelry_when_gained_discards_treasures_for_horses() {
    let mut g = setup(&[Card::Hostelry, Card::Workshop]);
    hand(
        &mut g,
        P0,
        &[Card::Workshop, Card::Copper, Card::Silver, Card::Estate],
    );
    g.start();
    play(&mut g, Card::Workshop);
    choose_pile(&mut g, Card::Hostelry);
    choose_kinds(&mut g, &[Card::Copper, Card::Silver]);
    assert_eq!(gained(&g, P0), [Card::Hostelry, Card::Horse, Card::Horse]);
    assert_eq!(hand_kinds(&g, P0), [Card::Estate]);
    assert!(g.log().contains(&Event::Revealed {
        player: P0,
        card: Card::Silver
    }));
}

#[test]
fn hostelry_needs_no_treasures_discarded() {
    let mut g = setup(&[Card::Hostelry, Card::Workshop]);
    hand(&mut g, P0, &[Card::Workshop, Card::Copper]);
    g.start();
    play(&mut g, Card::Workshop);
    choose_pile(&mut g, Card::Hostelry);
    choose(&mut g, &[]);
    assert_eq!(gained(&g, P0), [Card::Hostelry]);
}

#[test]
fn hunting_lodge_may_discard_the_hand_for_five_cards() {
    let mut g = setup(&[Card::HuntingLodge]);
    hand(
        &mut g,
        P0,
        &[Card::HuntingLodge, Card::Estate, Card::Estate],
    );
    deck(
        &mut g,
        P0,
        &[Card::Curse]
            .into_iter()
            .chain([Card::Silver; 5])
            .collect::<Vec<_>>(),
    );
    g.start();
    play(&mut g, Card::HuntingLodge);
    assert_eq!(g.turn().actions, 2);
    yes(&mut g);
    assert_eq!(hand_kinds(&g, P0), [Card::Silver; 5]);
    assert_eq!(discard_kinds(&g, P0).len(), 3);
}

#[test]
fn kiln_gains_a_copy_of_the_next_card_played() {
    let mut g = setup(&[Card::Kiln, Card::Village]);
    hand(&mut g, P0, &[Card::Kiln, Card::Village, Card::Village]);
    g.start();
    play(&mut g, Card::Kiln);
    assert_eq!(g.turn().coins, 2);
    to_buy(&mut g);
    g.turn.phase = Phase::Action;
    let mut g = setup(&[Card::Kiln, Card::Village]);
    hand(&mut g, P0, &[Card::Village, Card::Kiln, Card::Village]);
    deck(&mut g, P0, &[Card::Copper; 5]);
    g.start();
    play(&mut g, Card::Village);
    play(&mut g, Card::Kiln);
    play(&mut g, Card::Village);
    assert_eq!(purpose(&g), Purpose::Gain);
    yes(&mut g);
    assert_eq!(gained(&g, P0), [Card::Village]);
    play_treasures(&mut g);
    assert_eq!(gained(&g, P0), [Card::Village]);
}

#[test]
fn kiln_gains_its_copy_before_victims_decide_on_moat() {
    let mut g = setup(&[Card::Kiln, Card::Militia, Card::Moat, Card::Village]);
    hand(&mut g, P0, &[Card::Village, Card::Kiln, Card::Militia]);
    hand(
        &mut g,
        P1,
        &[
            Card::Moat,
            Card::Copper,
            Card::Copper,
            Card::Copper,
            Card::Copper,
        ],
    );
    deck(&mut g, P0, &[Card::Copper; 5]);
    g.start();
    play(&mut g, Card::Village);
    play(&mut g, Card::Kiln);
    play(&mut g, Card::Militia);
    assert_eq!(g.pending().unwrap().player, P0);
    yes(&mut g);
    assert_eq!(gained(&g, P0), [Card::Militia]);
    assert_eq!(g.pending().unwrap().player, P1);
    trigger(&mut g, Card::Moat);
    assert_eq!(g.hand(P1).len(), 5);
}

#[test]
fn a_throne_roomed_kiln_gains_a_kiln_and_still_copies_the_next_card() {
    let mut g = setup(&[Card::Kiln, Card::ThroneRoom, Card::Village]);
    hand(&mut g, P0, &[Card::ThroneRoom, Card::Kiln, Card::Village]);
    g.start();
    g.turn.actions = 2;
    play(&mut g, Card::ThroneRoom);
    choose_kinds(&mut g, &[Card::Kiln]);
    yes(&mut g);
    assert_eq!(gained(&g, P0), [Card::Kiln]);
    play(&mut g, Card::Village);
    yes(&mut g);
    assert_eq!(gained(&g, P0), [Card::Kiln, Card::Village]);
    assert_eq!(g.turn().coins, 4);
}

#[test]
fn livery_gains_a_horse_per_card_costing_four_or_more_and_is_cumulative() {
    let mut g = setup(&[Card::Livery, Card::Smithy, Card::ThroneRoom]);
    hand(&mut g, P0, &[Card::ThroneRoom, Card::Livery]);
    g.start();
    play(&mut g, Card::ThroneRoom);
    choose_kinds(&mut g, &[Card::Livery]);
    to_buy(&mut g);
    g.turn.buys = 3;
    g.turn.coins = 10;
    buy(&mut g, Card::Silver);
    assert_eq!(gained(&g, P0), [Card::Silver]);
    buy(&mut g, Card::Smithy);
    resolve_triggers(&mut g);
    assert_eq!(
        gained(&g, P0),
        [Card::Silver, Card::Smithy, Card::Horse, Card::Horse]
    );
}

#[test]
fn livery_checks_the_cost_a_fisherman_had_as_it_was_gained() {
    let mut g = setup(&[Card::Livery, Card::Fisherman]);
    hand(&mut g, P0, &[Card::Livery]);
    g.start();
    play(&mut g, Card::Livery);
    to_buy(&mut g);
    buy(&mut g, Card::Fisherman);
    assert_eq!(gained(&g, P0), [Card::Fisherman]);
}

#[test]
fn mastermind_plays_an_action_three_times_next_turn() {
    let mut g = setup(&[Card::Mastermind, Card::Village]);
    hand(&mut g, P0, &[Card::Mastermind]);
    deck(
        &mut g,
        P0,
        &[Card::Village]
            .into_iter()
            .chain([Card::Copper; 20])
            .collect::<Vec<_>>(),
    );
    g.start();
    play(&mut g, Card::Mastermind);
    next_turn(&mut g);
    assert_eq!(in_play_kinds(&g, P0), [Card::Mastermind]);
    next_turn(&mut g);
    choose_kinds(&mut g, &[Card::Village]);
    assert_eq!(g.turn().actions, 7);
    assert_eq!(g.hand(P0).len(), 4 + 3);
    next_turn(&mut g);
    assert!(g.in_play(P0).is_empty());
}

#[test]
fn a_mastermind_playing_a_horse_three_times_returns_it_once() {
    let mut g = setup(&[Card::Mastermind, Card::Sleigh]);
    hand(&mut g, P0, &[Card::Mastermind]);
    let mut cards = vec![Card::Horse];
    cards.extend([Card::Copper; 30]);
    deck(&mut g, P0, &cards);
    g.start();
    play(&mut g, Card::Mastermind);
    next_turn(&mut g);
    next_turn(&mut g);
    choose_kinds(&mut g, &[Card::Horse]);
    assert_eq!(g.hand(P0).len(), 4 + 6);
    assert_eq!(g.turn().actions, 4);
    assert_eq!(pile_len(&g, Card::Horse), 31);
}

#[test]
fn mastermind_stays_with_a_barge_one_of_whose_plays_was_for_next_turn() {
    let mut g = setup(&[Card::Mastermind, Card::Barge]);
    hand(&mut g, P0, &[Card::Mastermind]);
    deck(
        &mut g,
        P0,
        &[Card::Barge]
            .into_iter()
            .chain([Card::Copper; 40])
            .collect::<Vec<_>>(),
    );
    g.start();
    play(&mut g, Card::Mastermind);
    next_turn(&mut g);
    next_turn(&mut g);
    choose_kinds(&mut g, &[Card::Barge]);
    modes(&mut g, &[0]);
    modes(&mut g, &[1]);
    modes(&mut g, &[0]);
    assert_eq!(g.turn().buys, 3);
    next_turn(&mut g);
    assert_eq!(in_play_kinds(&g, P0), [Card::Mastermind, Card::Barge]);
    next_turn(&mut g);
    assert_eq!(g.turn().buys, 2);
    next_turn(&mut g);
    assert!(g.in_play(P0).is_empty());
}

#[test]
fn paddock_gains_two_horses_and_an_action_per_empty_supply_pile() {
    let mut g = setup(&[Card::Paddock, Card::Village]);
    empty_pile(&mut g, Card::Village);
    hand(&mut g, P0, &[Card::Paddock]);
    g.start();
    play(&mut g, Card::Paddock);
    assert_eq!(gained(&g, P0), [Card::Horse, Card::Horse]);
    assert_eq!((g.turn().coins, g.turn().actions), (2, 1));
}

#[test]
fn sanctuary_may_exile_a_card_from_hand() {
    let mut g = setup(&[Card::Sanctuary]);
    hand(&mut g, P0, &[Card::Sanctuary, Card::Estate]);
    deck(&mut g, P0, &[Card::Copper; 5]);
    g.start();
    play(&mut g, Card::Sanctuary);
    assert_eq!((g.turn().actions, g.turn().buys), (1, 2));
    choose_kinds(&mut g, &[Card::Estate]);
    assert_eq!(exile_kinds(&g, P0), [Card::Estate]);
    assert_eq!(hand_kinds(&g, P0), [Card::Copper]);
}

#[test]
fn scrap_gives_a_different_thing_per_coin_of_the_trashed_cards_cost() {
    let mut g = setup(&[Card::Scrap]);
    hand(&mut g, P0, &[Card::Scrap, Card::Estate]);
    deck(&mut g, P0, &[Card::Copper; 5]);
    g.start();
    play(&mut g, Card::Scrap);
    modes(&mut g, &[0, 5]);
    assert_eq!(hand_kinds(&g, P0), [Card::Copper]);
    assert_eq!(gained(&g, P0), [Card::Horse]);
    assert_eq!((g.turn().actions, g.turn().buys, g.turn().coins), (0, 1, 0));
}

#[test]
fn scrap_on_a_six_cost_card_gives_all_six() {
    let mut g = setup(&[Card::Scrap]);
    hand(&mut g, P0, &[Card::Scrap, Card::Gold]);
    deck(&mut g, P0, &[Card::Copper; 5]);
    g.start();
    play(&mut g, Card::Scrap);
    assert_eq!(hand_kinds(&g, P0), [Card::Copper]);
    assert_eq!(gained(&g, P0), [Card::Silver, Card::Horse]);
    assert_eq!((g.turn().actions, g.turn().buys, g.turn().coins), (1, 2, 1));
}

#[test]
fn scrap_draws_before_gaining_so_a_drawn_sheepdog_reacts() {
    let mut g = setup(&[Card::Scrap, Card::Sheepdog]);
    hand(&mut g, P0, &[Card::Scrap, Card::Gold]);
    deck(&mut g, P0, &[Card::Sheepdog, Card::Copper, Card::Copper]);
    g.start();
    play(&mut g, Card::Scrap);
    trigger(&mut g, Card::Sheepdog);
    assert_eq!(hand_kinds(&g, P0), [Card::Copper, Card::Copper]);
}

#[test]
fn sheepdog_is_played_when_its_owner_gains_a_card() {
    let mut g = setup(&[Card::Sheepdog]);
    hand(&mut g, P0, &[Card::Sheepdog, Card::Copper, Card::Copper]);
    deck(&mut g, P0, &[Card::Silver, Card::Silver]);
    g.start();
    g.turn.buys = 2;
    play_treasures(&mut g);
    buy(&mut g, Card::Estate);
    trigger(&mut g, Card::Sheepdog);
    assert_eq!(hand_kinds(&g, P0), [Card::Silver, Card::Silver]);
    assert_eq!(
        in_play_kinds(&g, P0),
        [Card::Copper, Card::Copper, Card::Sheepdog]
    );
}

#[test]
fn sleigh_gains_two_horses_and_reacts_to_put_a_gained_card_on_the_deck() {
    let mut g = setup(&[Card::Sleigh]);
    hand(&mut g, P0, &[Card::Sleigh, Card::Sleigh]);
    deck(&mut g, P0, &[Card::Copper]);
    g.start();
    play(&mut g, Card::Sleigh);
    // The first Horse: discard the other Sleigh to put it onto the deck.
    trigger(&mut g, Card::Sleigh);
    modes(&mut g, &[1]);
    assert_eq!(deck_kinds(&g, P0), [Card::Horse, Card::Copper]);
    assert_eq!(discard_kinds(&g, P0), [Card::Sleigh, Card::Horse]);
}

#[test]
fn a_sleigh_gained_to_the_hand_discards_itself_but_cannot_move_itself_back() {
    let mut g = setup(&[Card::Sleigh, Card::Falconer]);
    hand(&mut g, P0, &[Card::Falconer]);
    g.start();
    play(&mut g, Card::Falconer);
    choose_pile(&mut g, Card::Sleigh);
    trigger(&mut g, Card::Sleigh);
    assert_eq!(discard_kinds(&g, P0), [Card::Sleigh]);
    assert!(g.hand(P0).is_empty());
}

#[test]
fn snowy_village_ignores_further_actions_this_turn() {
    let mut g = setup(&[Card::SnowyVillage, Card::Village, Card::ThroneRoom]);
    hand(
        &mut g,
        P0,
        &[
            Card::Village,
            Card::SnowyVillage,
            Card::Village,
            Card::ThroneRoom,
            Card::Smithy,
        ],
    );
    deck(&mut g, P0, &[Card::Copper; 20]);
    g.players[0].villagers = 1;
    g.start();
    play(&mut g, Card::Village);
    play(&mut g, Card::SnowyVillage);
    assert_eq!((g.turn().actions, g.turn().buys), (5, 2));
    play(&mut g, Card::Village);
    assert_eq!(g.turn().actions, 4);
    play(&mut g, Card::ThroneRoom);
    choose_kinds(&mut g, &[Card::Smithy]);
    assert_eq!(g.turn().actions, 3);
    respond(&mut g, Response::Move(Move::Villager));
    assert_eq!(g.turn().actions, 3);
    next_turn(&mut g);
    assert!(!g.turn().ignore_plus_actions);
}

#[test]
fn stockpile_gives_three_and_a_buy_and_exiles_itself() {
    let mut g = setup(&[Card::Stockpile]);
    hand(&mut g, P0, &[Card::Stockpile]);
    exiled(&mut g, P0, &[Card::Stockpile]);
    g.start();
    play_treasures(&mut g);
    assert_eq!((g.turn().coins, g.turn().buys), (3, 2));
    assert_eq!(exile_kinds(&g, P0), [Card::Stockpile; 2]);
    buy(&mut g, Card::Stockpile);
    trigger(&mut g, Card::Sanctuary);
    assert!(exile_kinds(&g, P0).is_empty());
    assert_eq!(discard_kinds(&g, P0), [Card::Stockpile; 3]);
}

#[test]
fn supplies_is_one_coin_and_a_horse_onto_the_deck() {
    let mut g = setup(&[Card::Supplies]);
    hand(&mut g, P0, &[Card::Supplies]);
    deck(&mut g, P0, &[Card::Copper]);
    g.start();
    play_treasures(&mut g);
    assert_eq!(g.turn().coins, 1);
    assert_eq!(deck_kinds(&g, P0), [Card::Horse, Card::Copper]);
}

#[test]
fn village_green_now_or_next_turn() {
    let mut g = setup(&[Card::VillageGreen]);
    hand(&mut g, P0, &[Card::VillageGreen, Card::VillageGreen]);
    deck(&mut g, P0, &[Card::Copper; 20]);
    g.start();
    play(&mut g, Card::VillageGreen);
    modes(&mut g, &[0]);
    assert_eq!((g.turn().actions, g.hand(P0).len()), (2, 2));
    play(&mut g, Card::VillageGreen);
    modes(&mut g, &[1]);
    next_turn(&mut g);
    assert_eq!(in_play_kinds(&g, P0), [Card::VillageGreen]);
    next_turn(&mut g);
    assert_eq!((g.turn().actions, g.hand(P0).len()), (3, 6));
}

#[test]
fn village_green_discarded_other_than_in_clean_up_may_be_played() {
    let mut g = setup(&[Card::VillageGreen, Card::Cellar]);
    hand(
        &mut g,
        P0,
        &[Card::Cellar, Card::VillageGreen, Card::Estate],
    );
    deck(&mut g, P0, &[Card::Copper; 10]);
    g.start();
    play(&mut g, Card::Cellar);
    choose_kinds(&mut g, &[Card::VillageGreen, Card::Estate]);
    trigger(&mut g, Card::VillageGreen);
    modes(&mut g, &[0]);
    assert_eq!(in_play_kinds(&g, P0), [Card::Cellar, Card::VillageGreen]);
    assert_eq!(g.turn().actions, 3);
    assert_eq!(g.hand(P0).len(), 1 + 2);
}

#[test]
fn village_green_discarded_by_cardinal_is_played_out_of_turn() {
    let mut g = setup(&[Card::VillageGreen, Card::Cardinal]);
    hand(&mut g, P0, &[Card::Cardinal]);
    deck(
        &mut g,
        P1,
        &[Card::VillageGreen, Card::Copper, Card::Copper, Card::Copper],
    );
    g.start();
    play(&mut g, Card::Cardinal);
    // Village Green ($4) is exiled or discarded: P1 must Exile it as the only one in range.
    assert_eq!(exile_kinds(&g, P1), [Card::VillageGreen]);
    let mut g = setup(&[Card::VillageGreen, Card::Cardinal]);
    hand(&mut g, P0, &[Card::Cardinal]);
    deck(
        &mut g,
        P1,
        &[Card::VillageGreen, Card::Silver, Card::Copper, Card::Copper],
    );
    g.start();
    play(&mut g, Card::Cardinal);
    choose_kinds(&mut g, &[Card::Silver]);
    trigger(&mut g, Card::VillageGreen);
    modes(&mut g, &[1]);
    assert_eq!(in_play_kinds(&g, P1), [Card::VillageGreen]);
    // The +2 Actions of a play now would not help P0.
    assert_eq!(g.turn().actions, 0);
}

#[test]
fn wayfarer_draws_three_and_may_gain_a_silver() {
    let mut g = setup(&[Card::Wayfarer]);
    hand(&mut g, P0, &[Card::Wayfarer]);
    deck(&mut g, P0, &[Card::Copper; 5]);
    g.start();
    play(&mut g, Card::Wayfarer);
    yes(&mut g);
    assert_eq!(g.hand(P0).len(), 3);
    assert_eq!(gained(&g, P0), [Card::Silver]);
}

#[test]
fn wayfarer_costs_what_the_last_other_card_gained_this_turn_costs() {
    let mut g = setup(&[Card::Wayfarer, Card::Bridge]);
    hand(&mut g, P0, &[Card::Bridge, Card::Gold, Card::Gold]);
    g.start();
    assert_eq!(g.cost(Card::Wayfarer), Cost::coins(6));
    g.turn.buys = 3;
    play(&mut g, Card::Bridge);
    assert_eq!(g.cost(Card::Wayfarer), Cost::coins(5));
    play_treasures(&mut g);
    buy(&mut g, Card::Silver);
    // Bridge applies to the Silver only.
    assert_eq!(g.cost(Card::Wayfarer), Cost::coins(2));
    buy(&mut g, Card::Wayfarer);
    assert_eq!(g.cost(Card::Wayfarer), Cost::coins(2));
    buy(&mut g, Card::Curse);
    assert_eq!(g.cost(Card::Wayfarer), Cost::ZERO);
    next_turn(&mut g);
    assert_eq!(g.cost(Card::Wayfarer), Cost::coins(6));
}
