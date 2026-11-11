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

/// Takes the offer to discard the gained card's copies from Exile.
fn discard_from_exile(g: &mut Game) {
    let i = trigger_labels(g)
        .iter()
        .position(|l| l.ends_with("from Exile (optional)"))
        .expect("an offer to discard from Exile");
    respond(g, Response::option(Opt::Trigger(i as u16)));
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
    random_games_with_copies(&pool, &[Card::Horse], 0..fuzz_seeds(160), |seed| {
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
    let mut g = setup(&[Card::BountyHunter]);
    let exile = exiled(&mut g, P0, &[Card::Silver, Card::Silver, Card::Estate]);
    hand(&mut g, P0, &[Card::Gold]);
    g.start();
    // Exile is public.
    let v = g.view(P1);
    assert_eq!(v.player(P0).mat(Mat::Exile), Some(&exile[..]));
    assert_eq!(v.card(exile[2]), Some(Card::Estate));
    g.turn.buys = 2;
    play_treasures(&mut g);
    buy(&mut g, Card::Silver);
    // The offer is about the gained card, whatever put its copies into Exile.
    let d = g.pending().unwrap();
    assert_eq!(
        (d.purpose, d.source),
        (Purpose::Trigger, Some(Card::Silver))
    );
    assert_eq!(
        trigger_labels(&g),
        ["Discard Silver from Exile (optional)", "Done"]
    );
    discard_from_exile(&mut g);
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
    discard_from_exile(&mut g);
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
    discard_from_exile(&mut g);
    assert!(exile_kinds(&g, P1).is_empty());
    assert_eq!(discard_kinds(&g, P1), [Card::Horse, Card::Horse]);
}

#[test]
fn gatekeeper_exiles_a_horse_gained_again_after_its_first_gain_discarded_exiled_horses() {
    let mut g = setup(&[Card::Gatekeeper, Card::Supplies]);
    hand(&mut g, P0, &[Card::Copper]);
    deck(&mut g, P0, &[Card::Gatekeeper]);
    hand(&mut g, P1, &[Card::Supplies]);
    let mut cards = vec![Card::Supplies];
    cards.extend([Card::Copper; 5]);
    deck(&mut g, P1, &cards);
    exiled(&mut g, P1, &[Card::Horse]);
    g.start();
    next_turn(&mut g);
    // P1 gains a Horse onto the deck and discards the Exiled one.
    play_treasures(&mut g);
    discard_from_exile(&mut g);
    assert!(exile_kinds(&g, P1).is_empty());
    next_turn(&mut g);
    play(&mut g, Card::Gatekeeper);
    next_turn(&mut g);
    // That Horse returns to its pile and is the one gained again.
    play(&mut g, Card::Horse);
    play_treasures(&mut g);
    assert_eq!(exile_kinds(&g, P1), [Card::Horse]);
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
fn sleigh_is_discarded_not_played_so_voyages_limit_does_not_stop_it() {
    let mut g = setup_with_landscapes(
        &[Card::Odysseys, Card::Sleigh, Card::Sheepdog],
        &[Card::PlateauShepherds],
    );
    hand(&mut g, P0, &[Card::Voyage]);
    deck(
        &mut g,
        P0,
        &[
            Card::Copper,
            Card::Copper,
            Card::Copper,
            Card::Sleigh,
            Card::Sheepdog,
        ],
    );
    g.start();
    play(&mut g, Card::Voyage);
    next_turn(&mut g);
    assert_eq!(g.turn().extra, Some(Card::Voyage));
    // Three plays from the hand: Sheepdog can no longer be played, Sleigh still reacts.
    play_treasures(&mut g);
    buy(&mut g, Card::Silver);
    assert_eq!(trigger_labels(&g), ["Sleigh (optional)", "Done"]);
    trigger(&mut g, Card::Sleigh);
    modes(&mut g, &[0]);
    assert!(hand_kinds(&g, P0).contains(&Card::Silver));
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
    discard_from_exile(&mut g);
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
fn village_green_discarded_during_clean_up_cannot_be_played() {
    let mut g = setup(&[Card::VillageGreen]);
    g.landscapes.push(Card::Friendly);
    let pile = g.pile_of(Card::VillageGreen).unwrap();
    g.piles[pile.index()].traits.push(Card::Friendly);
    hand(&mut g, P0, &[Card::VillageGreen, Card::Copper]);
    deck(&mut g, P0, &[Card::Copper; 10]);
    exiled(&mut g, P0, &[Card::VillageGreen]);
    g.start();
    to_buy(&mut g);
    end_phase(&mut g);
    // Friendly, at the start of Clean-up, discards the one in hand to gain another,
    // which lets the Exiled one be discarded.
    trigger(&mut g, Card::Friendly);
    discard_from_exile(&mut g);
    assert!(exile_kinds(&g, P0).is_empty());
    assert_eq!(g.count_owned(P0, Card::VillageGreen), 3);
    assert!(in_play_kinds(&g, P0).is_empty());
    assert_eq!(g.turn().player, P1);
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

#[test]
fn kiln_gains_a_stockpile_before_the_played_one_exiles_itself() {
    let mut g = setup(&[Card::Kiln, Card::Stockpile]);
    hand(&mut g, P0, &[Card::Kiln, Card::Stockpile]);
    g.start();
    play(&mut g, Card::Kiln);
    play_treasures(&mut g);
    yes(&mut g);
    assert_eq!(exile_kinds(&g, P0), [Card::Stockpile]);
    assert_eq!(discard_kinds(&g, P0), [Card::Stockpile]);
}

// ---- Events ----

fn play_treasure(g: &mut Game, card: Card) {
    to_buy(g);
    let p = g.turn().player;
    let c = find(g, p, card);
    respond(g, Response::Move(Move::Play(c)));
}

#[test]
fn alliance_gains_six_cards_in_order() {
    let mut g = setup_with_landscapes(&[Card::Smithy], &[Card::Alliance]);
    hand(
        &mut g,
        P0,
        &[Card::Gold, Card::Gold, Card::Gold, Card::Copper],
    );
    g.start();
    play_treasures(&mut g);
    buy_event(&mut g, Card::Alliance);
    assert_eq!(
        gained(&g, P0),
        [
            Card::Province,
            Card::Duchy,
            Card::Estate,
            Card::Gold,
            Card::Silver,
            Card::Copper
        ]
    );
}

#[test]
fn banish_exiles_any_number_of_cards_with_the_same_name() {
    let mut g = setup_with_landscapes(&[Card::Smithy], &[Card::Banish]);
    hand(
        &mut g,
        P0,
        &[
            Card::Gold,
            Card::Copper,
            Card::Estate,
            Card::Estate,
            Card::Estate,
        ],
    );
    g.start();
    g.turn.buys = 2;
    play_treasures(&mut g);
    buy_event(&mut g, Card::Banish);
    choose_kinds(&mut g, &[Card::Estate]);
    let DecisionKind::Cards { candidates, .. } = &g.pending().unwrap().kind else {
        panic!("expected more copies to choose");
    };
    assert_eq!(kinds(&g, candidates), [Card::Estate, Card::Estate]);
    choose_kinds(&mut g, &[Card::Estate]);
    assert_eq!(exile_kinds(&g, P0), [Card::Estate, Card::Estate]);
    assert_eq!(hand_kinds(&g, P0), [Card::Estate]);
}

#[test]
fn bargain_gains_a_non_victory_card_and_gives_each_other_player_a_horse() {
    let mut g = setup_with_landscapes(&[Card::Smithy], &[Card::Bargain]);
    hand(&mut g, P0, &[Card::Gold, Card::Copper]);
    g.start();
    play_treasures(&mut g);
    buy_event(&mut g, Card::Bargain);
    let DecisionKind::Pile { piles, .. } = &g.pending().unwrap().kind else {
        panic!("expected a pile choice");
    };
    assert!(!piles.contains(&g.supply_pile(Card::Duchy).unwrap()));
    assert!(!piles.contains(&g.supply_pile(Card::Gold).unwrap()));
    choose_pile(&mut g, Card::Smithy);
    assert_eq!(gained(&g, P0), [Card::Smithy]);
    assert_eq!(gained(&g, P1), [Card::Horse]);
}

#[test]
fn commerce_gains_a_gold_per_differently_named_card_gained_this_turn() {
    let mut g = setup_with_landscapes(&[Card::Smithy], &[Card::Commerce]);
    hand(&mut g, P0, &[Card::Gold, Card::Gold, Card::Gold]);
    g.start();
    g.turn.buys = 4;
    play_treasures(&mut g);
    buy(&mut g, Card::Copper);
    buy(&mut g, Card::Copper);
    buy(&mut g, Card::Silver);
    buy_event(&mut g, Card::Commerce);
    assert_eq!(
        gained(&g, P0),
        [
            Card::Copper,
            Card::Copper,
            Card::Silver,
            Card::Gold,
            Card::Gold
        ]
    );
}

#[test]
fn delay_sets_an_action_aside_and_plays_it_at_the_start_of_next_turn() {
    let mut g = setup_with_landscapes(&[Card::Village], &[Card::Delay]);
    hand(&mut g, P0, &[Card::Village]);
    deck(&mut g, P0, &[Card::Copper; 20]);
    g.start();
    buy_event(&mut g, Card::Delay);
    choose_kinds(&mut g, &[Card::Village]);
    assert_eq!(kinds(&g, &g.player(P0).set_aside), [Card::Village]);
    next_turn(&mut g);
    assert_eq!(in_play_kinds(&g, P0), [Card::Village]);
    assert_eq!((g.turn().actions, g.hand(P0).len()), (3, 6));
}

#[test]
fn delay_may_set_nothing_aside() {
    let mut g = setup_with_landscapes(&[Card::Village], &[Card::Delay]);
    hand(&mut g, P0, &[Card::Village]);
    g.start();
    buy_event(&mut g, Card::Delay);
    choose(&mut g, &[]);
    assert!(g.lasting().is_empty());
}

#[test]
fn demand_gains_a_horse_then_a_card_both_onto_the_deck() {
    let mut g = setup_with_landscapes(&[Card::Smithy, Card::Destrier], &[Card::Demand]);
    hand(&mut g, P0, &[Card::Gold, Card::Silver]);
    deck(&mut g, P0, &[Card::Copper]);
    g.start();
    g.turn.buys = 2;
    play_treasures(&mut g);
    buy_event(&mut g, Card::Demand);
    // The Horse is gained before the card is chosen, which makes Destrier cheaper.
    assert_eq!(g.cost(Card::Destrier), Cost::coins(5));
    choose_pile(&mut g, Card::Smithy);
    assert_eq!(
        deck_kinds(&g, P0),
        [Card::Smithy, Card::Horse, Card::Copper]
    );
}

#[test]
fn desperation_once_per_turn_gains_a_curse_for_a_buy_and_two() {
    let mut g = setup_with_landscapes(&[Card::Smithy], &[Card::Desperation]);
    hand(&mut g, P0, &[Card::Copper]);
    g.start();
    buy_event(&mut g, Card::Desperation);
    yes(&mut g);
    assert_eq!(gained(&g, P0), [Card::Curse]);
    assert_eq!((g.turn().buys, g.turn().coins), (1, 2));
    assert!(!can_buy_event(&g, Card::Desperation));
    next_turn(&mut g);
    next_turn(&mut g);
    to_buy(&mut g);
    assert!(can_buy_event(&g, Card::Desperation));
}

#[test]
fn desperation_with_no_curse_left_gives_nothing() {
    let mut g = setup_with_landscapes(&[Card::Smithy], &[Card::Desperation]);
    empty_pile(&mut g, Card::Curse);
    g.start();
    buy_event(&mut g, Card::Desperation);
    yes(&mut g);
    assert!(gained(&g, P0).is_empty());
    assert!(g.turn().player == P1 || g.turn().coins == 0);
}

#[test]
fn enclave_gains_a_gold_then_exiles_a_duchy() {
    let mut g = setup_with_landscapes(&[Card::Sleigh], &[Card::Enclave]);
    hand(
        &mut g,
        P0,
        &[Card::Gold, Card::Gold, Card::Silver, Card::Sleigh],
    );
    g.start();
    play_treasures(&mut g);
    buy_event(&mut g, Card::Enclave);
    // Sleigh may react to the Gold, before the Duchy is Exiled.
    assert_eq!(trigger_labels(&g), ["Sleigh (optional)", "Done"]);
    assert!(exile_kinds(&g, P0).is_empty());
    no(&mut g);
    assert_eq!(gained(&g, P0), [Card::Gold]);
    assert_eq!(exile_kinds(&g, P0), [Card::Duchy]);
}

#[test]
fn enhance_trashes_a_non_victory_card_to_gain_one_costing_up_to_two_more() {
    let mut g = setup_with_landscapes(&[Card::Smithy], &[Card::Enhance]);
    hand(
        &mut g,
        P0,
        &[Card::Silver, Card::Copper, Card::Smithy, Card::Estate],
    );
    g.start();
    play_treasure(&mut g, Card::Silver);
    play_treasure(&mut g, Card::Copper);
    buy_event(&mut g, Card::Enhance);
    assert_eq!(candidates(&g).len(), 1);
    choose_kinds(&mut g, &[Card::Smithy]);
    choose_pile(&mut g, Card::Gold);
    assert_eq!(gained(&g, P0), [Card::Gold]);
}

#[test]
fn gamble_discards_the_top_card_and_may_play_an_action_or_treasure() {
    let mut g = setup_with_landscapes(&[Card::Village], &[Card::Gamble]);
    hand(&mut g, P0, &[Card::Silver]);
    deck(&mut g, P0, &[Card::Gold, Card::Copper]);
    g.start();
    play_treasures(&mut g);
    buy_event(&mut g, Card::Gamble);
    assert_eq!(g.turn().buys, 1);
    yes(&mut g);
    assert_eq!(in_play_kinds(&g, P0), [Card::Silver, Card::Gold]);
    assert_eq!(g.turn().coins, 3);
}

#[test]
fn gamble_discards_a_card_that_cannot_be_played() {
    let mut g = setup_with_landscapes(&[Card::Village], &[Card::Gamble]);
    hand(&mut g, P0, &[Card::Silver]);
    deck(&mut g, P0, &[Card::Estate, Card::Copper]);
    g.start();
    play_treasures(&mut g);
    buy_event(&mut g, Card::Gamble);
    assert_eq!(discard_kinds(&g, P0), [Card::Estate]);
    assert_eq!(g.turn().buys, 1);
}

#[test]
fn invest_draws_when_another_player_gains_or_invests_in_a_copy() {
    let mut g = setup_with_landscapes(&[Card::Smithy], &[Card::Invest]);
    hand(&mut g, P0, &[Card::Gold, Card::Copper]);
    deck(&mut g, P0, &[Card::Copper; 20]);
    hand(&mut g, P1, &[Card::Gold, Card::Gold, Card::Gold]);
    g.start();
    play_treasures(&mut g);
    buy_event(&mut g, Card::Invest);
    assert_eq!(exile_kinds(&g, P0), [Card::Smithy]);
    assert_eq!(g.turn().player, P1);
    assert_eq!(g.hand(P0).len(), 5);
    g.turn.buys = 2;
    play_treasures(&mut g);
    buy(&mut g, Card::Smithy);
    assert_eq!(g.hand(P0).len(), 7);
    buy_event(&mut g, Card::Invest);
    assert_eq!(g.hand(P0).len(), 9);
}

#[test]
fn invest_stops_drawing_once_the_card_leaves_exile() {
    let mut g = setup_with_landscapes(&[Card::Smithy], &[Card::Invest]);
    hand(&mut g, P0, &[Card::Gold, Card::Gold, Card::Gold]);
    g.start();
    g.turn.buys = 2;
    play_treasures(&mut g);
    buy_event(&mut g, Card::Invest);
    buy(&mut g, Card::Smithy);
    discard_from_exile(&mut g);
    assert!(exile_kinds(&g, P0).is_empty());
    assert!(g.ext.menagerie.invested.is_empty());
}

#[test]
fn march_plays_an_action_from_the_discard_pile() {
    let mut g = setup_with_landscapes(&[Card::Village], &[Card::March]);
    hand(&mut g, P0, &[Card::Gold]);
    discard(&mut g, P0, &[Card::Village, Card::Estate]);
    g.start();
    g.turn.buys = 2;
    play_treasures(&mut g);
    buy_event(&mut g, Card::March);
    choose_kinds(&mut g, &[Card::Village]);
    assert_eq!(in_play_kinds(&g, P0), [Card::Gold, Card::Village]);
}

#[test]
fn populate_gains_one_card_from_each_action_supply_pile_in_a_chosen_order() {
    let mut g = setup_with_landscapes(
        &[Card::Village, Card::Smithy, Card::Gardens, Card::Sleigh],
        &[Card::Populate],
    );
    empty_pile(&mut g, Card::Sleigh);
    hand(
        &mut g,
        P0,
        &[Card::Gold, Card::Gold, Card::Gold, Card::Copper],
    );
    g.start();
    play_treasures(&mut g);
    buy_event(&mut g, Card::Populate);
    choose_pile(&mut g, Card::Smithy);
    assert_eq!(gained(&g, P0), [Card::Smithy, Card::Village]);
}

#[test]
fn pursue_keeps_the_named_cards_on_the_deck_and_discards_the_rest() {
    let mut g = setup_with_landscapes(&[Card::Village], &[Card::Pursue]);
    hand(&mut g, P0, &[Card::Silver]);
    deck(
        &mut g,
        P0,
        &[
            Card::Copper,
            Card::Estate,
            Card::Copper,
            Card::Silver,
            Card::Gold,
        ],
    );
    g.start();
    play_treasures(&mut g);
    buy_event(&mut g, Card::Pursue);
    respond(&mut g, Response::Name(Card::Copper));
    assert_eq!(deck_kinds(&g, P0), [Card::Copper, Card::Copper, Card::Gold]);
    assert_eq!(discard_kinds(&g, P0), [Card::Estate, Card::Silver]);
    assert_eq!(g.turn().buys, 1);
}

#[test]
fn reap_sets_a_gold_aside_and_plays_it_next_turn() {
    let mut g = setup_with_landscapes(&[Card::Village], &[Card::Reap]);
    hand(&mut g, P0, &[Card::Gold, Card::Gold, Card::Copper]);
    deck(&mut g, P0, &[Card::Estate; 10]);
    g.start();
    play_treasures(&mut g);
    buy_event(&mut g, Card::Reap);
    assert_eq!(kinds(&g, &g.player(P0).set_aside), [Card::Gold]);
    next_turn(&mut g);
    assert_eq!(in_play_kinds(&g, P0), [Card::Gold]);
    assert_eq!(g.turn().coins, 3);
}

#[test]
fn ride_gains_a_horse() {
    let mut g = setup_with_landscapes(&[Card::Village], &[Card::Ride]);
    hand(&mut g, P0, &[Card::Silver]);
    g.start();
    play_treasures(&mut g);
    buy_event(&mut g, Card::Ride);
    assert_eq!(gained(&g, P0), [Card::Horse]);
}

#[test]
fn seize_the_day_gives_an_extra_turn_once_per_game() {
    let mut g = setup_with_landscapes(&[Card::Village], &[Card::SeizeTheDay]);
    hand(&mut g, P0, &[Card::Gold, Card::Copper]);
    deck(
        &mut g,
        P0,
        &[
            Card::Gold,
            Card::Gold,
            Card::Copper,
            Card::Copper,
            Card::Copper,
        ],
    );
    g.start();
    play_treasures(&mut g);
    buy_event(&mut g, Card::SeizeTheDay);
    assert_eq!(g.turn().player, P0);
    assert_eq!(g.turn().extra, Some(Card::SeizeTheDay));
    play_treasures(&mut g);
    assert!(!can_buy_event(&g, Card::SeizeTheDay));
    next_turn(&mut g);
    assert_eq!(g.turn().player, P1);
}

#[test]
fn seize_the_day_bought_on_an_outpost_turn_gives_a_third_turn() {
    let mut g = setup_with_landscapes(&[Card::Outpost], &[Card::SeizeTheDay]);
    hand(&mut g, P0, &[Card::Outpost]);
    deck(
        &mut g,
        P0,
        &[Card::Gold, Card::Gold, Card::Copper, Card::Copper],
    );
    g.start();
    play(&mut g, Card::Outpost);
    next_turn(&mut g);
    assert_eq!(g.turn().extra, Some(Card::Outpost));
    play_treasures(&mut g);
    buy_event(&mut g, Card::SeizeTheDay);
    assert_eq!(g.turn().player, P0);
    assert_eq!(g.turn().extra, Some(Card::SeizeTheDay));
}

#[test]
fn stampede_gains_five_horses_onto_the_deck_with_five_or_fewer_cards_in_play() {
    let mut g = setup_with_landscapes(&[Card::Village], &[Card::Stampede]);
    hand(&mut g, P0, &[Card::Gold, Card::Silver]);
    deck(&mut g, P0, &[Card::Copper]);
    g.start();
    g.turn.buys = 2;
    play_treasures(&mut g);
    buy_event(&mut g, Card::Stampede);
    assert_eq!(gained(&g, P0), [Card::Horse; 5]);
    assert_eq!(deck_kinds(&g, P0)[..5], [Card::Horse; 5]);

    let mut g = setup_with_landscapes(&[Card::Village], &[Card::Stampede]);
    hand(&mut g, P0, &[Card::Copper; 6]);
    g.start();
    play_treasures(&mut g);
    buy_event(&mut g, Card::Stampede);
    assert!(gained(&g, P0).is_empty());
}

#[test]
fn toil_gives_a_buy_and_may_play_an_action_from_the_hand() {
    let mut g = setup_with_landscapes(&[Card::Village], &[Card::Toil]);
    hand(&mut g, P0, &[Card::Silver, Card::Village]);
    deck(&mut g, P0, &[Card::Copper; 5]);
    g.start();
    play_treasures(&mut g);
    buy_event(&mut g, Card::Toil);
    choose_kinds(&mut g, &[Card::Village]);
    assert_eq!(in_play_kinds(&g, P0), [Card::Silver, Card::Village]);
    assert_eq!(g.turn().buys, 1);
}

#[test]
fn transport_exiles_an_action_from_the_supply_or_puts_an_exiled_one_onto_the_deck() {
    let mut g = setup_with_landscapes(&[Card::Village], &[Card::Transport]);
    hand(&mut g, P0, &[Card::Gold, Card::Gold]);
    exiled(&mut g, P0, &[Card::Smithy, Card::Estate]);
    deck(&mut g, P0, &[Card::Copper]);
    g.start();
    g.turn.buys = 3;
    play_treasures(&mut g);
    buy_event(&mut g, Card::Transport);
    modes(&mut g, &[0]);
    assert_eq!(
        exile_kinds(&g, P0),
        [Card::Smithy, Card::Estate, Card::Village]
    );
    buy_event(&mut g, Card::Transport);
    modes(&mut g, &[1]);
    choose_kinds(&mut g, &[Card::Smithy]);
    assert_eq!(exile_kinds(&g, P0), [Card::Estate, Card::Village]);
    assert_eq!(deck_kinds(&g, P0), [Card::Smithy, Card::Copper]);
}

// ---- Ways ----

fn way_game(way: Card, kingdom: &[Card]) -> Game {
    setup_with_landscapes(kingdom, &[way])
}

#[test]
fn a_way_is_offered_for_every_action_and_none_plays_the_card_normally() {
    let mut g = way_game(Card::WayOfTheOx, &[Card::Smithy]);
    hand(&mut g, P0, &[Card::Smithy]);
    deck(&mut g, P0, &[Card::Copper; 5]);
    g.start();
    play(&mut g, Card::Smithy);
    let DecisionKind::Choose { options, .. } = &g.pending().unwrap().kind else {
        panic!("expected the Way choice");
    };
    assert_eq!(options, &[Opt::No, Opt::Card(Card::WayOfTheOx)]);
    own(&mut g);
    assert_eq!(g.hand(P0).len(), 3);
}

#[test]
fn simple_ways_give_their_bonuses_instead_of_the_card() {
    let cases = [
        (Card::WayOfTheMonkey, (0, 2, 1, 0)),
        (Card::WayOfTheMule, (1, 1, 1, 0)),
        (Card::WayOfTheOtter, (0, 1, 0, 2)),
        (Card::WayOfTheOx, (2, 1, 0, 0)),
        (Card::WayOfThePig, (1, 1, 0, 1)),
        (Card::WayOfTheSheep, (0, 1, 2, 0)),
    ];
    for (w, (actions, buys, coins, cards)) in cases {
        let mut g = way_game(w, &[Card::Smithy]);
        hand(&mut g, P0, &[Card::Smithy, Card::Village]);
        deck(&mut g, P0, &[Card::Copper; 5]);
        g.start();
        play(&mut g, Card::Smithy);
        way(&mut g, w);
        let t = g.turn();
        let got = (t.actions, t.buys, t.coins, g.hand(P0).len() - 1);
        assert_eq!(got, (actions, buys, coins, cards), "{w}");
    }
}

#[test]
fn an_attack_played_as_a_way_attacks_nobody_but_moat_is_revealed_first() {
    let mut g = way_game(Card::WayOfTheOx, &[Card::Militia, Card::Moat]);
    hand(&mut g, P0, &[Card::Militia]);
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
    g.start();
    play(&mut g, Card::Militia);
    // P1 decides on Moat before P0 decides on the Way.
    assert_eq!(g.pending().unwrap().player, P1);
    no(&mut g);
    way(&mut g, Card::WayOfTheOx);
    assert_eq!(g.hand(P1).len(), 5);
    assert_eq!((g.turn().actions, g.turn().coins), (2, 0));
}

#[test]
fn a_duration_played_as_a_way_does_not_stay_in_play() {
    let mut g = way_game(Card::WayOfTheOx, &[Card::Barge]);
    hand(&mut g, P0, &[Card::Barge]);
    g.start();
    play(&mut g, Card::Barge);
    way(&mut g, Card::WayOfTheOx);
    next_turn(&mut g);
    assert!(g.in_play(P0).is_empty());
    assert!(g.lasting().is_empty());
}

#[test]
fn throne_room_chooses_a_way_for_each_play() {
    let mut g = way_game(Card::WayOfTheOx, &[Card::ThroneRoom, Card::Smithy]);
    hand(&mut g, P0, &[Card::ThroneRoom, Card::Smithy]);
    deck(&mut g, P0, &[Card::Copper; 5]);
    g.start();
    play(&mut g, Card::ThroneRoom);
    own(&mut g);
    choose_kinds(&mut g, &[Card::Smithy]);
    way(&mut g, Card::WayOfTheOx);
    own(&mut g);
    assert_eq!((g.turn().actions, g.hand(P0).len()), (2, 3));
}

#[test]
fn way_of_the_butterfly_returns_the_card_for_one_costing_exactly_one_more() {
    let mut g = way_game(Card::WayOfTheButterfly, &[Card::Village, Card::Smithy]);
    hand(&mut g, P0, &[Card::Village]);
    g.start();
    play(&mut g, Card::Village);
    way(&mut g, Card::WayOfTheButterfly);
    yes(&mut g);
    assert!(g.in_play(P0).is_empty());
    assert_eq!(pile_len(&g, Card::Village), 11);
    assert_eq!(gained(&g, P0), [Card::Smithy]);
}

#[test]
fn way_of_the_butterfly_returns_a_horse_to_its_non_supply_pile() {
    let mut g = way_game(Card::WayOfTheButterfly, &[Card::Sleigh, Card::Smithy]);
    hand(&mut g, P0, &[Card::Horse]);
    g.start();
    play(&mut g, Card::Horse);
    way(&mut g, Card::WayOfTheButterfly);
    yes(&mut g);
    assert_eq!(pile_len(&g, Card::Horse), 31);
    assert_eq!(gained(&g, P0), [Card::Smithy]);
}

#[test]
fn way_of_the_butterfly_cannot_return_a_card_with_no_pile() {
    let mut g = way_game(Card::WayOfTheButterfly, &[Card::Smithy]);
    hand(&mut g, P0, &[Card::Necropolis]);
    g.start();
    play(&mut g, Card::Necropolis);
    way(&mut g, Card::WayOfTheButterfly);
    assert_eq!(in_play_kinds(&g, P0), [Card::Necropolis]);
    assert!(gained(&g, P0).is_empty());
}

#[test]
fn way_of_the_camel_and_the_worm_exile_a_gold_and_an_estate() {
    for (w, card) in [
        (Card::WayOfTheCamel, Card::Gold),
        (Card::WayOfTheWorm, Card::Estate),
    ] {
        let mut g = way_game(w, &[Card::Village]);
        hand(&mut g, P0, &[Card::Village]);
        g.start();
        play(&mut g, Card::Village);
        way(&mut g, w);
        assert_eq!(exile_kinds(&g, P0), [card]);
    }
}

#[test]
fn way_of_the_chameleon_turns_cards_into_coins_and_coins_into_cards() {
    let mut g = way_game(Card::WayOfTheChameleon, &[Card::Smithy, Card::Moneylender]);
    hand(&mut g, P0, &[Card::Smithy, Card::Moneylender, Card::Copper]);
    deck(&mut g, P0, &[Card::Silver; 10]);
    g.start();
    g.turn.actions = 2;
    play(&mut g, Card::Smithy);
    way(&mut g, Card::WayOfTheChameleon);
    assert_eq!((g.turn().coins, g.hand(P0).len()), (3, 2));
    play(&mut g, Card::Moneylender);
    way(&mut g, Card::WayOfTheChameleon);
    choose_kinds(&mut g, &[Card::Copper]);
    assert_eq!(g.turn().coins, 3);
    assert_eq!(hand_kinds(&g, P0), [Card::Silver; 3]);
}

/// Governor's "+1 (+3) Cards" is +Cards; only its player's are exchanged.
#[test]
fn way_of_the_chameleon_exchanges_governors_plus_cards_for_its_player_only() {
    let mut g = way_game(Card::WayOfTheChameleon, &[Card::Governor]);
    hand(&mut g, P0, &[Card::Governor]);
    deck(&mut g, P0, &[Card::Copper; 5]);
    deck(&mut g, P1, &[Card::Copper; 5]);
    let p1 = g.hand(P1).len();
    g.start();
    play(&mut g, Card::Governor);
    way(&mut g, Card::WayOfTheChameleon);
    modes(&mut g, &[0]);
    assert_eq!((g.hand(P0).len(), g.turn().coins), (0, 3));
    assert_eq!(g.hand(P1).len(), p1 + 1);
}

#[test]
fn way_of_the_chameleon_also_changes_this_turns_later_effects() {
    let mut g = way_game(Card::WayOfTheChameleon, &[Card::Merchant]);
    hand(&mut g, P0, &[Card::Merchant, Card::Silver]);
    deck(&mut g, P0, &[Card::Gold; 5]);
    g.start();
    play(&mut g, Card::Merchant);
    way(&mut g, Card::WayOfTheChameleon);
    // +1 Card +1 Action became +$1 +1 Action.
    assert_eq!(
        (g.turn().coins, g.turn().actions, g.hand(P0).len()),
        (1, 1, 1)
    );
    play_treasure(&mut g, Card::Silver);
    // Merchant's +$1 for the first Silver became +1 Card.
    assert_eq!(g.turn().coins, 3);
    assert_eq!(hand_kinds(&g, P0), [Card::Gold]);
}

#[test]
fn way_of_the_chameleon_changes_plus_cards_after_a_decision_but_not_draw_until() {
    let mut g = way_game(
        Card::WayOfTheChameleon,
        &[Card::HuntingLodge, Card::Library],
    );
    hand(
        &mut g,
        P0,
        &[Card::HuntingLodge, Card::Library, Card::Estate],
    );
    deck(&mut g, P0, &[Card::Copper; 20]);
    g.start();
    play(&mut g, Card::HuntingLodge);
    way(&mut g, Card::WayOfTheChameleon);
    assert_eq!((g.turn().coins, g.turn().actions), (1, 2));
    yes(&mut g);
    assert_eq!(g.turn().coins, 6);
    assert!(g.hand(P0).is_empty());

    let mut g = way_game(Card::WayOfTheChameleon, &[Card::Library]);
    hand(&mut g, P0, &[Card::Library]);
    deck(&mut g, P0, &[Card::Copper; 20]);
    g.start();
    play(&mut g, Card::Library);
    way(&mut g, Card::WayOfTheChameleon);
    assert_eq!((g.turn().coins, g.hand(P0).len()), (0, 7));

    // Not even on a card whose other options say +Cards.
    let mut g = way_game(Card::WayOfTheChameleon, &[Card::Townsfolk]);
    hand(&mut g, P0, &[Card::Blacksmith]);
    deck(&mut g, P0, &[Card::Copper; 20]);
    g.start();
    play(&mut g, Card::Blacksmith);
    way(&mut g, Card::WayOfTheChameleon);
    respond(&mut g, Response::option(Opt::Mode(0)));
    assert_eq!((g.turn().coins, g.hand(P0).len()), (0, 6));
}

#[test]
fn way_of_the_chameleon_on_a_duration_changes_only_this_turn() {
    let mut g = way_game(Card::WayOfTheChameleon, &[Card::MerchantShip]);
    hand(&mut g, P0, &[Card::MerchantShip]);
    deck(&mut g, P0, &[Card::Copper; 20]);
    g.start();
    play(&mut g, Card::MerchantShip);
    way(&mut g, Card::WayOfTheChameleon);
    assert_eq!((g.turn().coins, g.hand(P0).len()), (0, 2));
    next_turn(&mut g);
    next_turn(&mut g);
    assert_eq!(g.turn().coins, 2);
}

#[test]
fn way_of_the_chameleon_does_not_change_a_card_its_card_plays() {
    let mut g = way_game(Card::WayOfTheChameleon, &[Card::ThroneRoom, Card::Smithy]);
    hand(&mut g, P0, &[Card::ThroneRoom, Card::Smithy]);
    deck(&mut g, P0, &[Card::Copper; 10]);
    g.start();
    play(&mut g, Card::ThroneRoom);
    way(&mut g, Card::WayOfTheChameleon);
    choose_kinds(&mut g, &[Card::Smithy]);
    own(&mut g);
    own(&mut g);
    assert_eq!((g.turn().coins, g.hand(P0).len()), (0, 6));
}

#[test]
fn way_of_the_chameleon_leaves_the_minus_coin_token_for_cards() {
    let mut g = way_game(Card::WayOfTheChameleon, &[Card::Moneylender]);
    hand(&mut g, P0, &[Card::Moneylender, Card::Copper]);
    deck(&mut g, P0, &[Card::Silver; 5]);
    g.players[0].minus_coin = true;
    g.start();
    play(&mut g, Card::Moneylender);
    way(&mut g, Card::WayOfTheChameleon);
    choose_kinds(&mut g, &[Card::Copper]);
    assert_eq!(g.hand(P0).len(), 3);
    assert!(g.player(P0).minus_coin);
}

#[test]
fn way_of_the_frog_puts_the_card_onto_the_deck_when_discarded_from_play() {
    let mut g = way_game(Card::WayOfTheFrog, &[Card::Smithy]);
    hand(&mut g, P0, &[Card::Smithy]);
    deck(&mut g, P0, &[Card::Copper; 10]);
    g.start();
    play(&mut g, Card::Smithy);
    way(&mut g, Card::WayOfTheFrog);
    assert_eq!(g.turn().actions, 1);
    next_turn(&mut g);
    assert!(hand_kinds(&g, P0).contains(&Card::Smithy));
}

#[test]
fn way_of_the_frog_on_a_throned_duration_does_not_topdeck_it() {
    let mut g = way_game(Card::WayOfTheFrog, &[Card::ThroneRoom, Card::Barge]);
    hand(&mut g, P0, &[Card::ThroneRoom, Card::Barge]);
    deck(&mut g, P0, &[Card::Copper; 20]);
    g.start();
    play(&mut g, Card::ThroneRoom);
    own(&mut g);
    choose_kinds(&mut g, &[Card::Barge]);
    own(&mut g);
    modes(&mut g, &[1]);
    way(&mut g, Card::WayOfTheFrog);
    next_turn(&mut g);
    assert_eq!(in_play_kinds(&g, P0), [Card::ThroneRoom, Card::Barge]);
}

#[test]
fn way_of_the_goat_trashes_a_card_from_the_hand() {
    let mut g = way_game(Card::WayOfTheGoat, &[Card::Smithy]);
    hand(&mut g, P0, &[Card::Smithy, Card::Estate, Card::Copper]);
    g.start();
    play(&mut g, Card::Smithy);
    way(&mut g, Card::WayOfTheGoat);
    choose_kinds(&mut g, &[Card::Estate]);
    assert_eq!(hand_kinds(&g, P0), [Card::Copper]);
    assert_eq!(kinds(&g, g.trash()).last(), Some(&Card::Estate));
}

#[test]
fn way_of_the_horse_returns_the_card_to_its_pile() {
    let mut g = way_game(Card::WayOfTheHorse, &[Card::Smithy]);
    hand(&mut g, P0, &[Card::Smithy]);
    deck(&mut g, P0, &[Card::Copper; 5]);
    g.start();
    play(&mut g, Card::Smithy);
    way(&mut g, Card::WayOfTheHorse);
    assert_eq!((g.turn().actions, g.hand(P0).len()), (1, 2));
    assert_eq!(pile_len(&g, Card::Smithy), 11);
}

#[test]
fn way_of_the_horse_still_draws_for_a_card_with_no_pile() {
    let mut g = way_game(Card::WayOfTheHorse, &[Card::Smithy]);
    hand(&mut g, P0, &[Card::Necropolis]);
    deck(&mut g, P0, &[Card::Copper; 5]);
    g.start();
    play(&mut g, Card::Necropolis);
    way(&mut g, Card::WayOfTheHorse);
    assert_eq!(g.hand(P0).len(), 2);
    assert_eq!(in_play_kinds(&g, P0), [Card::Necropolis]);
}

#[test]
fn way_of_the_mole_discards_the_hand_and_draws_three() {
    let mut g = way_game(Card::WayOfTheMole, &[Card::Smithy]);
    hand(&mut g, P0, &[Card::Smithy, Card::Estate]);
    deck(&mut g, P0, &[Card::Silver; 5]);
    g.start();
    play(&mut g, Card::Smithy);
    way(&mut g, Card::WayOfTheMole);
    assert_eq!(hand_kinds(&g, P0), [Card::Silver; 3]);
    assert_eq!(discard_kinds(&g, P0), [Card::Estate]);
    assert_eq!(g.turn().actions, 1);
}

fn set_mouse(g: &mut Game, card: Card) {
    let id = g
        .ext
        .menagerie
        .mouse
        .expect("Way of the Mouse set a card aside");
    let Zone::Pile(pile) = g.zone(id) else {
        panic!("the Mouse card is in its pile");
    };
    g.cards[id.index()] = card;
    g.piles[pile.index()].card = card;
}

#[test]
fn way_of_the_mouse_sets_aside_an_unused_cheap_non_duration_action() {
    for seed in 0..20 {
        let g = setup_with(GameConfig {
            landscapes: vec![Card::WayOfTheMouse],
            ..GameConfig::new(2, vec![Card::Village, Card::Smithy], seed)
        });
        let id = g.ext.menagerie.mouse.expect("a card set aside");
        let card = g.card(id);
        assert!(card.def().kingdom && card.is(Types::ACTION) && !card.is(Types::DURATION));
        assert!(matches!(card.cost().unwrap().coins, 2 | 3), "{card}");
        assert!(!g.config().kingdom.contains(&card));
        let Zone::Pile(pile) = g.zone(id) else {
            panic!("set aside in its own pile");
        };
        assert!(!g.pile(pile).in_supply);
    }
}

#[test]
fn way_of_the_mouse_plays_the_set_aside_card_leaving_it_there() {
    let mut g = way_game(Card::WayOfTheMouse, &[Card::Smithy]);
    set_mouse(&mut g, Card::Village);
    hand(&mut g, P0, &[Card::Smithy]);
    deck(&mut g, P0, &[Card::Copper; 5]);
    g.start();
    play(&mut g, Card::Smithy);
    way(&mut g, Card::WayOfTheMouse);
    // Village is played without a Way choice: Way of the Mouse can't be used on it.
    assert_eq!((g.turn().actions, g.hand(P0).len()), (2, 1));
    assert_eq!(in_play_kinds(&g, P0), [Card::Smithy]);
    let mouse = g.ext.menagerie.mouse.unwrap();
    assert!(matches!(g.zone(mouse), Zone::Pile(_)));
}

#[test]
fn way_of_the_mouses_card_cannot_move_itself() {
    let mut g = way_game(Card::WayOfTheMouse, &[Card::Smithy]);
    set_mouse(&mut g, Card::Horse);
    hand(&mut g, P0, &[Card::Smithy]);
    deck(&mut g, P0, &[Card::Copper; 5]);
    g.start();
    play(&mut g, Card::Smithy);
    way(&mut g, Card::WayOfTheMouse);
    assert_eq!(g.hand(P0).len(), 2);
    let mouse = g.ext.menagerie.mouse.unwrap();
    assert_eq!(g.zone(mouse), Zone::Pile(g.pile_of(Card::Horse).unwrap()));
}

#[test]
fn way_of_the_owl_draws_until_six_cards_in_hand() {
    let mut g = way_game(Card::WayOfTheOwl, &[Card::Smithy]);
    hand(&mut g, P0, &[Card::Smithy, Card::Estate, Card::Estate]);
    deck(&mut g, P0, &[Card::Copper; 10]);
    g.start();
    play(&mut g, Card::Smithy);
    way(&mut g, Card::WayOfTheOwl);
    assert_eq!(g.hand(P0).len(), 6);
}

#[test]
fn way_of_the_rat_discards_a_treasure_to_gain_a_copy() {
    let mut g = way_game(Card::WayOfTheRat, &[Card::Smithy]);
    hand(&mut g, P0, &[Card::Smithy, Card::Copper]);
    g.start();
    play(&mut g, Card::Smithy);
    way(&mut g, Card::WayOfTheRat);
    choose_kinds(&mut g, &[Card::Copper]);
    assert_eq!(gained(&g, P0), [Card::Smithy]);
    assert_eq!(discard_kinds(&g, P0), [Card::Copper, Card::Smithy]);
}

#[test]
fn way_of_the_rat_cannot_copy_a_card_that_is_not_in_the_supply() {
    let mut g = way_game(Card::WayOfTheRat, &[Card::Sleigh]);
    hand(&mut g, P0, &[Card::Horse, Card::Copper]);
    g.start();
    play(&mut g, Card::Horse);
    way(&mut g, Card::WayOfTheRat);
    choose_kinds(&mut g, &[Card::Copper]);
    assert!(gained(&g, P0).is_empty());
}

#[test]
fn way_of_the_seal_lets_gained_cards_go_onto_the_deck_this_turn() {
    let mut g = way_game(Card::WayOfTheSeal, &[Card::Smithy]);
    hand(&mut g, P0, &[Card::Smithy, Card::Silver]);
    deck(&mut g, P0, &[Card::Copper]);
    g.start();
    play(&mut g, Card::Smithy);
    way(&mut g, Card::WayOfTheSeal);
    g.turn.buys = 2;
    play_treasures(&mut g);
    assert_eq!(g.turn().coins, 3);
    buy(&mut g, Card::Silver);
    trigger(&mut g, Card::WayOfTheSeal);
    assert_eq!(deck_kinds(&g, P0), [Card::Silver, Card::Copper]);
}

#[test]
fn way_of_the_squirrel_draws_two_at_the_end_of_the_turn() {
    let mut g = way_game(Card::WayOfTheSquirrel, &[Card::Smithy]);
    hand(&mut g, P0, &[Card::Smithy]);
    deck(&mut g, P0, &[Card::Copper; 10]);
    g.start();
    play(&mut g, Card::Smithy);
    way(&mut g, Card::WayOfTheSquirrel);
    assert!(g.hand(P0).is_empty());
    next_turn(&mut g);
    assert_eq!(g.hand(P0).len(), 7);
    assert!(g.lasting().is_empty());
}

#[test]
fn way_of_the_turtle_sets_the_card_aside_and_plays_it_next_turn() {
    let mut g = way_game(Card::WayOfTheTurtle, &[Card::Village]);
    hand(&mut g, P0, &[Card::Village]);
    deck(&mut g, P0, &[Card::Copper; 20]);
    g.start();
    play(&mut g, Card::Village);
    way(&mut g, Card::WayOfTheTurtle);
    assert_eq!(kinds(&g, &g.player(P0).set_aside), [Card::Village]);
    next_turn(&mut g);
    next_turn(&mut g);
    // Played at the start of the turn: it may be set aside again.
    way(&mut g, Card::WayOfTheTurtle);
    assert_eq!(kinds(&g, &g.player(P0).set_aside), [Card::Village]);
    next_turn(&mut g);
    next_turn(&mut g);
    own(&mut g);
    assert_eq!((g.turn().actions, g.hand(P0).len()), (3, 6));
}

#[test]
fn throne_room_and_way_of_the_turtle_twice_sets_the_card_aside_once() {
    let mut g = way_game(Card::WayOfTheTurtle, &[Card::ThroneRoom, Card::Village]);
    hand(&mut g, P0, &[Card::ThroneRoom, Card::Village]);
    deck(&mut g, P0, &[Card::Copper; 20]);
    g.start();
    play(&mut g, Card::ThroneRoom);
    own(&mut g);
    choose_kinds(&mut g, &[Card::Village]);
    way(&mut g, Card::WayOfTheTurtle);
    way(&mut g, Card::WayOfTheTurtle);
    assert_eq!(g.lasting().len(), 1);
    next_turn(&mut g);
    assert!(g.in_play(P0).is_empty());
    next_turn(&mut g);
    own(&mut g);
    assert_eq!(in_play_kinds(&g, P0), [Card::Village]);
}

#[test]
fn way_of_the_turtle_after_a_duration_played_normally_cancels_its_next_turn() {
    let mut g = way_game(Card::WayOfTheTurtle, &[Card::ThroneRoom, Card::Barge]);
    hand(&mut g, P0, &[Card::ThroneRoom, Card::Barge]);
    deck(&mut g, P0, &[Card::Copper; 20]);
    g.start();
    play(&mut g, Card::ThroneRoom);
    own(&mut g);
    choose_kinds(&mut g, &[Card::Barge]);
    own(&mut g);
    modes(&mut g, &[1]);
    way(&mut g, Card::WayOfTheTurtle);
    next_turn(&mut g);
    assert!(g.in_play(P0).is_empty());
    next_turn(&mut g);
    // Only the Turtle's play happens: Barge's own next-turn effect is gone.
    own(&mut g);
    modes(&mut g, &[0]);
    assert_eq!((g.hand(P0).len(), g.turn().buys), (8, 2));
}

// ---- More rulings ----

#[test]
fn black_cat_reacting_on_its_owners_turn_curses_nobody() {
    let mut g = setup(&[Card::BlackCat, Card::Ambassador]);
    hand(
        &mut g,
        P0,
        &[Card::Ambassador, Card::Estate, Card::BlackCat],
    );
    deck(&mut g, P0, &[Card::Copper; 5]);
    g.start();
    play(&mut g, Card::Ambassador);
    choose_kinds(&mut g, &[Card::Estate]);
    choose(&mut g, &[]);
    assert_eq!(gained(&g, P1), [Card::Estate]);
    trigger(&mut g, Card::BlackCat);
    assert_eq!(
        hand_kinds(&g, P0),
        [Card::Estate, Card::Copper, Card::Copper]
    );
    assert_eq!(gained(&g, P1), [Card::Estate]);
}

#[test]
fn a_sheepdog_gained_to_the_hand_may_react_to_its_own_gain() {
    let mut g = setup(&[Card::Sheepdog, Card::Falconer]);
    hand(&mut g, P0, &[Card::Falconer]);
    deck(&mut g, P0, &[Card::Silver; 5]);
    g.start();
    play(&mut g, Card::Falconer);
    choose_pile(&mut g, Card::Sheepdog);
    trigger(&mut g, Card::Sheepdog);
    assert_eq!(hand_kinds(&g, P0), [Card::Silver, Card::Silver]);
}

#[test]
fn sleigh_cannot_move_a_card_way_of_the_seal_already_moved() {
    let mut g = way_game(Card::WayOfTheSeal, &[Card::Sleigh, Card::Smithy]);
    hand(&mut g, P0, &[Card::Smithy, Card::Sleigh, Card::Gold]);
    deck(&mut g, P0, &[Card::Copper]);
    g.start();
    play(&mut g, Card::Smithy);
    way(&mut g, Card::WayOfTheSeal);
    g.turn.buys = 2;
    play_treasures(&mut g);
    buy(&mut g, Card::Silver);
    trigger(&mut g, Card::WayOfTheSeal);
    // The Sleigh may still be discarded, but the Silver stays on the deck.
    trigger(&mut g, Card::Sleigh);
    assert_eq!(deck_kinds(&g, P0), [Card::Silver, Card::Copper]);
    assert_eq!(discard_kinds(&g, P0), [Card::Sleigh]);
}

#[test]
fn vassal_loses_track_of_a_village_green_played_by_its_reaction() {
    let mut g = setup(&[Card::VillageGreen, Card::Vassal]);
    hand(&mut g, P0, &[Card::Vassal]);
    deck(
        &mut g,
        P0,
        &[Card::VillageGreen, Card::Copper, Card::Copper],
    );
    g.start();
    play(&mut g, Card::Vassal);
    trigger(&mut g, Card::VillageGreen);
    modes(&mut g, &[0]);
    assert_eq!(in_play_kinds(&g, P0), [Card::Vassal, Card::VillageGreen]);
    assert_eq!(g.turn().actions, 2);
}

#[test]
fn way_of_the_squirrel_used_out_of_turn_draws_at_the_end_of_that_turn() {
    let mut g = way_game(Card::WayOfTheSquirrel, &[Card::BlackCat]);
    hand(&mut g, P0, &[Card::Copper, Card::Copper]);
    hand(&mut g, P1, &[Card::BlackCat]);
    deck(&mut g, P1, &[Card::Silver; 5]);
    g.start();
    play_treasures(&mut g);
    buy(&mut g, Card::Estate);
    trigger(&mut g, Card::BlackCat);
    way(&mut g, Card::WayOfTheSquirrel);
    assert!(gained(&g, P0) == [Card::Estate]);
    assert_eq!(g.turn().player, P1);
    assert_eq!(hand_kinds(&g, P1), [Card::Silver; 2]);
}

#[test]
fn a_mastermind_whose_throne_room_plays_a_duration_does_not_stay() {
    let mut g = setup(&[Card::Mastermind, Card::ThroneRoom, Card::Barge]);
    hand(&mut g, P0, &[Card::Mastermind]);
    let mut cards = vec![Card::ThroneRoom, Card::Barge];
    cards.extend([Card::Copper; 40]);
    deck(&mut g, P0, &cards);
    g.start();
    play(&mut g, Card::Mastermind);
    next_turn(&mut g);
    next_turn(&mut g);
    choose_kinds(&mut g, &[Card::ThroneRoom]);
    choose_kinds(&mut g, &[Card::Barge]);
    modes(&mut g, &[1]);
    modes(&mut g, &[1]);
    next_turn(&mut g);
    assert_eq!(in_play_kinds(&g, P0), [Card::ThroneRoom, Card::Barge]);
}

#[test]
fn a_throne_room_whose_mastermind_plays_a_duration_stays_with_the_mastermind() {
    let mut g = setup(&[Card::Mastermind, Card::ThroneRoom, Card::Barge]);
    hand(&mut g, P0, &[Card::ThroneRoom, Card::Mastermind]);
    let mut cards = vec![Card::Barge];
    cards.extend([Card::Copper; 40]);
    deck(&mut g, P0, &cards);
    g.start();
    play(&mut g, Card::ThroneRoom);
    choose_kinds(&mut g, &[Card::Mastermind]);
    next_turn(&mut g);
    next_turn(&mut g);
    // The two identical Mastermind effects resolve without an order prompt: the first
    // plays Barge three times; the second has nothing to play.
    choose_kinds(&mut g, &[Card::Barge]);
    for _ in 0..3 {
        modes(&mut g, &[1]);
    }
    next_turn(&mut g);
    assert_eq!(
        in_play_kinds(&g, P0),
        [Card::ThroneRoom, Card::Mastermind, Card::Barge]
    );
}

#[test]
fn a_throne_room_whose_throne_room_plays_a_duration_does_not_stay() {
    let mut g = setup(&[Card::ThroneRoom, Card::Barge]);
    hand(
        &mut g,
        P0,
        &[Card::ThroneRoom, Card::ThroneRoom, Card::Barge],
    );
    deck(&mut g, P0, &[Card::Copper; 40]);
    g.start();
    play(&mut g, Card::ThroneRoom);
    choose_kinds(&mut g, &[Card::ThroneRoom]);
    choose_kinds(&mut g, &[Card::Barge]);
    modes(&mut g, &[1]);
    modes(&mut g, &[1]);
    next_turn(&mut g);
    assert_eq!(in_play_kinds(&g, P0), [Card::ThroneRoom, Card::Barge]);
}

#[test]
fn village_greens_discarded_to_militia_together_can_each_be_played() {
    let mut g = setup(&[Card::VillageGreen, Card::Militia]);
    hand(&mut g, P0, &[Card::Militia]);
    hand(
        &mut g,
        P1,
        &[
            Card::VillageGreen,
            Card::VillageGreen,
            Card::Copper,
            Card::Copper,
            Card::Copper,
        ],
    );
    g.start();
    play(&mut g, Card::Militia);
    choose_kinds(&mut g, &[Card::VillageGreen, Card::VillageGreen]);
    // Both are in the discard pile before either reacts.
    assert_eq!(discard_kinds(&g, P1), [Card::VillageGreen; 2]);
    trigger(&mut g, Card::VillageGreen);
    modes(&mut g, &[1]);
    trigger(&mut g, Card::VillageGreen);
    modes(&mut g, &[1]);
    assert_eq!(in_play_kinds(&g, P1), [Card::VillageGreen; 2]);
}

// ---- Other sets ----

fn enlightened(way: Card) -> Game {
    let mut g = setup_with_landscapes(&[Card::Smithy], &[way, Card::Enlightenment]);
    g.prophecy = Some((Card::Enlightenment, 0));
    g
}

#[test]
fn a_way_can_be_used_on_a_treasure_enlightenment_lets_you_play_as_an_action() {
    let mut g = enlightened(Card::WayOfTheOx);
    hand(&mut g, P0, &[Card::Gold]);
    deck(&mut g, P0, &[Card::Copper; 5]);
    g.start();
    play(&mut g, Card::Gold);
    way(&mut g, Card::WayOfTheOx);
    assert_eq!(
        (g.turn().actions, g.turn().coins, g.hand(P0).len()),
        (2, 0, 0)
    );
}

#[test]
fn enlightenment_still_replaces_a_treasure_played_with_way_of_the_chameleon() {
    let mut g = enlightened(Card::WayOfTheChameleon);
    hand(&mut g, P0, &[Card::Gold]);
    deck(&mut g, P0, &[Card::Copper; 5]);
    g.start();
    play(&mut g, Card::Gold);
    way(&mut g, Card::WayOfTheChameleon);
    assert_eq!(
        (g.turn().actions, g.turn().coins, g.hand(P0).len()),
        (1, 0, 1)
    );
}

#[test]
fn changeling_cannot_exchange_a_fisherman_that_cost_two_when_gained() {
    let mut g = setup(&[Card::Changeling, Card::Fisherman]);
    hand(&mut g, P0, &[Card::Silver]);
    g.start();
    g.turn.buys = 2;
    play_treasures(&mut g);
    buy(&mut g, Card::Fisherman);
    assert_eq!(purpose(&g), Purpose::Turn);
    assert_eq!(discard_kinds(&g, P0), [Card::Fisherman]);
}

#[test]
fn changeling_can_exchange_a_destrier_that_cost_three_when_gained() {
    let mut g = setup(&[Card::Changeling, Card::Destrier]);
    hand(&mut g, P0, &[Card::Gold]);
    g.start();
    g.turn.buys = 5;
    play_treasures(&mut g);
    for _ in 0..3 {
        buy(&mut g, Card::Copper);
    }
    assert_eq!(g.cost(Card::Destrier), Cost::coins(3));
    buy(&mut g, Card::Destrier);
    trigger(&mut g, Card::Changeling);
    assert_eq!(discard_kinds(&g, P0).last(), Some(&Card::Changeling));
}

#[test]
fn horse_users_joining_the_game_late_bring_the_horse_pile() {
    let mut g = setup(&[Card::Smithy]);
    assert!(g.pile_of(Card::Horse).is_none());
    add_kingdom_piles(&mut g, &[Card::Smithy]);
    assert!(g.pile_of(Card::Horse).is_none());
    add_kingdom_piles(&mut g, &[Card::Supplies]);
    let horses = g.pile_of(Card::Horse).expect("a Horse pile");
    assert!(!g.pile(horses).in_supply);
    add_kingdom_piles(&mut g, &[Card::Sleigh]);
    assert_eq!(
        g.piles().iter().filter(|p| p.card == Card::Horse).count(),
        1
    );
}

#[test]
fn toil_may_play_a_shadow_card_from_the_deck() {
    let mut g = setup_with_landscapes(&[Card::Fishmonger], &[Card::Toil]);
    hand(&mut g, P0, &[Card::Silver]);
    deck(&mut g, P0, &[Card::Fishmonger, Card::Copper]);
    g.start();
    play_treasures(&mut g);
    buy_event(&mut g, Card::Toil);
    choose_kinds(&mut g, &[Card::Fishmonger]);
    assert_eq!(in_play_kinds(&g, P0), [Card::Silver, Card::Fishmonger]);
    assert_eq!((g.turn().buys, g.turn().coins), (2, 1));
}
