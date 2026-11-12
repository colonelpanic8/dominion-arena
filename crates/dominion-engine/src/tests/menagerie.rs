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
    assert_eq!(purpose(g), Purpose::Way);
    respond(g, Response::option(Opt::Card(w)));
}

/// Plays the Action being played for its own effect rather than a Way.
fn own(g: &mut Game) {
    assert_eq!(purpose(g), Purpose::Way);
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
        .position(|l| l.starts_with("Exile: "))
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
    assert_eq!(trigger_cards(&g)[0], Card::Silver);
    assert_eq!(
        trigger_labels(&g),
        [
            "Exile: discard every Silver from your Exile mat (optional)",
            "Decline"
        ]
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

/// On another player's turn, Way of the Chameleon turns Black Cat's +2 Cards into +$2,
/// which does nothing; the others still gain Curses.
#[test]
fn black_cat_with_way_of_the_chameleon_off_turn_gives_nothing_but_still_curses() {
    let mut g = way_game(Card::WayOfTheChameleon, &[Card::BlackCat]);
    hand(&mut g, P0, &[Card::Copper, Card::Copper]);
    hand(&mut g, P1, &[Card::BlackCat]);
    g.start();
    play_treasures(&mut g);
    buy(&mut g, Card::Estate);
    trigger(&mut g, Card::BlackCat);
    way(&mut g, Card::WayOfTheChameleon);
    assert!(g.hand(P1).is_empty());
    assert_eq!(g.turn().coins, 0);
    assert_eq!(gained(&g, P0), [Card::Estate, Card::Curse]);
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

/// Gatekeeper's page: "This does not lose track of a card if it never moves. So if you
/// gain a Nomad Camp onto your deck, then revealing Watchtower to topdeck the Nomad Camp
/// will not block Gatekeeper."
#[test]
fn gatekeeper_exiles_a_nomad_camp_that_watchtower_left_on_the_deck() {
    let mut g = setup(&[Card::Gatekeeper, Card::NomadCamp, Card::Watchtower]);
    hand(&mut g, P0, &[Card::Watchtower, Card::Silver, Card::Silver]);
    hand(&mut g, P1, &[Card::Gatekeeper]);
    g.start();
    play_out_of_turn(&mut g, P1, Card::Gatekeeper);
    to_buy(&mut g);
    play(&mut g, Card::Silver);
    play(&mut g, Card::Silver);
    buy(&mut g, Card::NomadCamp);
    trigger(&mut g, Card::Watchtower);
    respond(&mut g, Response::option(Opt::Mode(1)));
    assert_eq!(exile_kinds(&g, P0), [Card::NomadCamp]);
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
fn kiln_played_during_another_play_waits_for_the_next_one() {
    let mut g = setup(&[Card::Kiln]);
    g.players[0].projects.push(Card::Innovation);
    hand(&mut g, P0, &[Card::Kiln, Card::Kiln]);
    g.start();
    g.turn.actions = 2;
    play(&mut g, Card::Kiln);
    play(&mut g, Card::Kiln);
    // The first Kiln gains a copy of the second, which Innovation plays at once.
    yes(&mut g);
    trigger(&mut g, Card::Innovation);
    // The second Kiln's play had already begun: nothing more to copy for it.
    assert_eq!(purpose(&g), Purpose::Turn);
    assert_eq!(gained(&g, P0), [Card::Kiln]);
    assert_eq!(g.turn().coins, 6);
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

/// Two Sheepdogs are one option, and the second is offered again once the first is played.
#[test]
fn two_sheepdogs_are_one_option_each_playable() {
    let mut g = setup(&[Card::Sheepdog]);
    hand(&mut g, P0, &[Card::Sheepdog, Card::Sheepdog, Card::Copper]);
    deck(&mut g, P0, &[Card::Silver; 4]);
    g.start();
    play_treasures(&mut g);
    buy(&mut g, Card::Copper);
    let labels = trigger_labels(&g);
    assert_eq!(labels.len(), 2, "{labels:?}");
    trigger(&mut g, Card::Sheepdog);
    assert_eq!(in_play_kinds(&g, P0), [Card::Copper, Card::Sheepdog]);
    assert_eq!(trigger_labels(&g), labels);
    trigger(&mut g, Card::Sheepdog);
    let sheepdogs = g
        .log()
        .iter()
        .filter(|e| {
            matches!(
                e,
                Event::Played {
                    card: Card::Sheepdog,
                    ..
                }
            )
        })
        .count();
    assert_eq!(sheepdogs, 2);
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
    assert!(g.log().contains(&Event::TopDecked {
        player: P0,
        card: Some(Card::Horse)
    }));
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
    assert_eq!(trigger_cards(&g), [Card::Sleigh]);
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

/// Quarry makes Actions cheaper, and the game decides what is an Action: under
/// Enlightenment a Silver, under Inheritance an Estate. Wayfarer follows their costs.
#[test]
fn wayfarer_follows_a_cost_that_depends_on_the_types_a_card_has_now() {
    let mut g = setup_with_landscapes(
        &[Card::Wayfarer, Card::Quarry, Card::Village],
        &[Card::Enlightenment, Card::Inheritance],
    );
    g.prophecy = Some((Card::Enlightenment, 0));
    let village = g.pile_top(g.supply_pile(Card::Village).unwrap()).unwrap();
    g.move_card(village, Zone::SetAside(P0), crate::game::Pos::Top);
    g.ext.adventures.inherited.push((P0, Some(village)));
    hand(&mut g, P0, &[Card::Quarry]);
    g.start();
    end_phase(&mut g);
    play(&mut g, Card::Quarry);
    g.turn.buys = 3;
    assert_eq!(g.cost(Card::Silver), Cost::coins(1));
    buy(&mut g, Card::Silver);
    assert_eq!(g.cost(Card::Wayfarer), Cost::coins(1));
    buy(&mut g, Card::Estate);
    assert_eq!(g.turn().player, P0);
    assert_eq!(g.cost(Card::Wayfarer), Cost::ZERO);
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
    assert_eq!(
        trigger_labels(&g),
        [
            "Sleigh: discard it to put Gold into your hand or onto your deck (optional)",
            "Decline"
        ]
    );
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
fn march_looks_through_the_discard_pile_and_plays_an_action_from_it() {
    let mut g = setup_with_landscapes(&[Card::Village], &[Card::March]);
    hand(&mut g, P0, &[Card::Gold]);
    let pile = discard(&mut g, P0, &[Card::Village, Card::Estate]);
    g.enable_trace();
    g.start();
    g.turn.buys = 2;
    play_treasures(&mut g);
    g.take_trace();
    buy_event(&mut g, Card::March);
    // P0 looked at the whole pile, not only at the Action on offer.
    let looked: Vec<CardId> = g
        .take_trace()
        .into_iter()
        .filter_map(|t| match t {
            Trace::Shown { card, to } if to == Some(P0) => Some(card),
            _ => None,
        })
        .collect();
    assert_eq!(looked, pile);
    choose_kinds(&mut g, &[Card::Village]);
    assert_eq!(in_play_kinds(&g, P0), [Card::Gold, Card::Village]);
}

/// The look through the discard pile is over once March plays a card from it: a Silver
/// the Workshop gains there is named when Royal Seal puts it onto the deck, as a bought
/// one is.
#[test]
fn march_names_a_card_moved_from_the_discard_pile_while_the_card_it_played_resolves() {
    let put_on_the_deck = |march: bool| {
        let mut g = setup_with_landscapes(&[Card::Workshop, Card::RoyalSeal], &[Card::March]);
        hand(&mut g, P0, &[Card::RoyalSeal, Card::Copper, Card::Copper]);
        deck(&mut g, P0, &[Card::Estate; 5]);
        discard(&mut g, P0, &[Card::Workshop]);
        g.start();
        play(&mut g, Card::RoyalSeal);
        play(&mut g, Card::Copper);
        play(&mut g, Card::Copper);
        if march {
            buy_event(&mut g, Card::March);
            choose_kinds(&mut g, &[Card::Workshop]);
            choose_pile(&mut g, Card::Silver);
        } else {
            buy(&mut g, Card::Silver);
        }
        assert_eq!(trigger_cards(&g), [Card::RoyalSeal]);
        respond(&mut g, Response::option(Opt::Trigger(0)));
        (g.log().iter())
            .filter(|e| matches!(e, Event::TopDecked { .. }))
            .cloned()
            .collect::<Vec<_>>()
    };
    let named = [Event::TopDecked {
        player: P0,
        card: Some(Card::Silver),
    }];
    assert_eq!(put_on_the_deck(false), named);
    assert_eq!(put_on_the_deck(true), named);
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

/// A game may have up to 128 piles (`setup::MAX_PILES`): Populate takes one card from a
/// pile past the 64th too.
#[test]
fn populate_gains_one_card_from_a_pile_past_the_64th() {
    let mut g = setup_with_landscapes(&[Card::Smithy], &[Card::Populate]);
    while g.piles().len() < 70 {
        g.add_pile(crate::game::Pile::new(Card::Curse, false));
    }
    add_kingdom_piles(&mut g, &[Card::Village]);
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
fn populate_skips_treasure_piles_under_enlightenment() {
    let mut g = setup_with_landscapes(&[Card::Village], &[Card::Populate]);
    g.prophecy = Some((Card::Enlightenment, 0));
    hand(
        &mut g,
        P0,
        &[Card::Gold, Card::Gold, Card::Gold, Card::Copper],
    );
    g.start();
    play_treasures(&mut g);
    buy_event(&mut g, Card::Populate);
    // Enlightenment changes cards, not piles: the Treasure piles are no Action piles.
    assert_eq!(gained(&g, P0), [Card::Village]);
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

/// "Put the matches back and discard the rest": the matches are back on the deck before the
/// Tunnel is discarded, so the Gold Watchtower puts onto the deck lands on top of them.
#[test]
fn pursue_puts_the_matches_back_before_discarding_the_rest() {
    let mut g = setup_with_landscapes(&[Card::Tunnel, Card::Watchtower], &[Card::Pursue]);
    hand(&mut g, P0, &[Card::Silver, Card::Watchtower]);
    deck(
        &mut g,
        P0,
        &[
            Card::Copper,
            Card::Tunnel,
            Card::Copper,
            Card::Estate,
            Card::Gold,
        ],
    );
    g.start();
    play_treasures(&mut g);
    buy_event(&mut g, Card::Pursue);
    respond(&mut g, Response::Name(Card::Copper));
    // Reveal Tunnel for a Gold, and Watchtower to put the Gold onto the deck.
    super::intrigue::trigger(&mut g, Card::Tunnel);
    super::intrigue::trigger(&mut g, Card::Watchtower);
    respond(&mut g, Response::option(Opt::Mode(1)));
    assert_eq!(
        deck_kinds(&g, P0),
        [Card::Gold, Card::Copper, Card::Copper, Card::Gold]
    );
    assert_eq!(discard_kinds(&g, P0), [Card::Tunnel, Card::Estate]);
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
        assert!(g.log().contains(&Event::UsedWay {
            player: P0,
            card: Card::Smithy,
            way: w
        }));
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
fn way_of_the_chameleon_changes_only_its_own_priests_later_coins() {
    let mut g = way_game(Card::WayOfTheChameleon, &[Card::Priest]);
    hand(
        &mut g,
        P0,
        &[
            Card::Priest,
            Card::Priest,
            Card::Estate,
            Card::Estate,
            Card::Estate,
        ],
    );
    deck(&mut g, P0, &[Card::Gold; 5]);
    g.start();
    g.turn.actions = 2;
    play(&mut g, Card::Priest);
    way(&mut g, Card::WayOfTheChameleon);
    choose_kinds(&mut g, &[Card::Estate]);
    assert_eq!(hand_kinds(&g, P0)[3..], [Card::Gold; 2]);
    play(&mut g, Card::Priest);
    own(&mut g);
    choose_kinds(&mut g, &[Card::Estate]);
    // The first Priest's +$2 became +2 Cards; the second gave +$2 itself.
    assert_eq!(g.turn().coins, 2);
    assert_eq!(g.hand(P0).len(), 5);
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

/// The -1 Card token stays when +Cards become +$, the -$1 token applies to those $, and
/// the -1 Card token applies to the cards that +$ become.
#[test]
fn way_of_the_chameleon_exchanges_before_the_minus_tokens_apply() {
    let mut g = way_game(Card::WayOfTheChameleon, &[Card::Smithy, Card::Moneylender]);
    hand(&mut g, P0, &[Card::Smithy, Card::Moneylender, Card::Copper]);
    deck(&mut g, P0, &[Card::Silver; 5]);
    g.players[0].minus_card = true;
    g.players[0].minus_coin = true;
    g.start();
    g.turn.actions = 2;
    play(&mut g, Card::Smithy);
    way(&mut g, Card::WayOfTheChameleon);
    assert_eq!((g.turn().coins, g.hand(P0).len()), (2, 2));
    assert!(g.player(P0).minus_card && !g.player(P0).minus_coin);
    play(&mut g, Card::Moneylender);
    way(&mut g, Card::WayOfTheChameleon);
    choose_kinds(&mut g, &[Card::Copper]);
    assert_eq!((g.turn().coins, g.hand(P0).len()), (2, 2));
    assert!(!g.player(P0).minus_card);
}

/// Enchantress and Highwayman change what the card does, whatever Way is used, and the
/// Way doesn't apply to what Enchantress gives instead (wiki, Way of the Chameleon). The
/// +1 Card and +$1 tokens on a card's pile are not its instructions.
#[test]
fn way_of_the_chameleon_exchanges_only_the_cards_instructions_as_followed() {
    let kingdom = [Card::Enchantress, Card::Highwayman, Card::Smithy];
    for (attack, got) in [(Card::Enchantress, (0, 1 + 1)), (Card::Highwayman, (0, 1))] {
        let mut g = way_game(Card::WayOfTheChameleon, &kingdom);
        g.players[1].projects.push(Card::Capitalism);
        hand(&mut g, P0, &[attack]);
        deck(&mut g, P0, &[Card::Copper; 10]);
        hand(&mut g, P1, &[Card::Militia, Card::Estate]);
        deck(&mut g, P1, &[Card::Copper; 10]);
        g.start();
        play(&mut g, attack);
        own(&mut g);
        next_turn(&mut g);
        // An Action, and under Capitalism a Treasure: the first of each this turn.
        play(&mut g, Card::Militia);
        way(&mut g, Card::WayOfTheChameleon);
        assert_eq!((g.turn().coins, g.hand(P1).len()), got, "{attack}");
        assert_eq!(g.hand(P0).len(), 5, "{attack}");
    }

    let mut g = way_game(Card::WayOfTheChameleon, &kingdom);
    hand(&mut g, P0, &[Card::Smithy]);
    deck(&mut g, P0, &[Card::Copper; 10]);
    let smithy = g.pile_of(Card::Smithy).unwrap();
    for token in [Token::PlusCard, Token::PlusCoin] {
        g.players[0].tokens.insert(token, Some(smithy));
        g.piles[smithy.index()].tokens.push((P0, token));
    }
    g.start();
    play(&mut g, Card::Smithy);
    way(&mut g, Card::WayOfTheChameleon);
    assert_eq!((g.turn().coins, g.hand(P0).len()), (1 + 3, 1));
}

/// Monkey: "Until your next turn, when the player to your right gains a card, +1 Card."
/// The Way changes what the card's instructions give later the same turn too, whatever
/// kind of effect gives it (wiki, Monkey).
#[test]
fn way_of_the_chameleon_changes_what_a_duration_gives_later_the_same_turn() {
    let mut g = way_game(Card::WayOfTheChameleon, &[Card::Monkey, Card::Witch]);
    hand(&mut g, P0, &[Card::Monkey, Card::Witch]);
    deck(&mut g, P0, &[Card::Gold; 12]);
    g.start();
    g.turn.actions = 2;
    play(&mut g, Card::Monkey);
    way(&mut g, Card::WayOfTheChameleon);
    play(&mut g, Card::Witch);
    own(&mut g);
    // Witch's +2 Cards; the Curse P1 gains gives Monkey's +1 Card, which is +$1.
    assert_eq!((g.turn().coins, g.hand(P0).len()), (1, 2));
    let coin = Event::Bonus {
        player: P0,
        actions: 0,
        buys: 0,
        coins: 1,
    };
    let monkey = Event::Resolved {
        player: P0,
        card: Card::Monkey,
    };
    assert!(logged_in_order(&g, &[monkey, coin]));
    // After that turn it gives cards: for the Copper P1 buys, and at the start of P0's
    // next turn, which follows.
    next_turn(&mut g);
    to_buy(&mut g);
    buy(&mut g, Card::Copper);
    assert_eq!(g.turn().player, P0);
    assert_eq!((g.turn().coins, g.hand(P0).len()), (0, 5 + 2));
}

/// Hireling and Samurai give their bonus at the start of each turn, the one they are
/// played at the start of included (wiki, Hireling): that one is exchanged, and only for
/// the play that used the Way.
#[test]
fn way_of_the_chameleon_changes_a_start_of_turn_bonus_given_the_turn_it_is_played() {
    for (card, now, later) in [
        (Card::Hireling, (1, 6), (0, 8)),
        (Card::Samurai, (2, 5), (3, 5)),
    ] {
        let mut g = way_game(Card::WayOfTheChameleon, &[card, Card::Mastermind]);
        hand(&mut g, P0, &[Card::Mastermind]);
        let mut cards = vec![card];
        cards.extend([Card::Copper; 4]);
        cards.extend([Card::Estate; 12]);
        deck(&mut g, P0, &cards);
        // Samurai's Attack finds no more than 3 cards in P1's hand.
        hand(&mut g, P1, &[Card::Copper; 3]);
        deck(&mut g, P1, &[]);
        discard(&mut g, P1, &[]);
        g.start();
        play(&mut g, Card::Mastermind);
        own(&mut g);
        next_turn(&mut g);
        next_turn(&mut g);
        // Mastermind plays it three times: with the Way, then twice without.
        choose_kinds(&mut g, &[card]);
        way(&mut g, Card::WayOfTheChameleon);
        own(&mut g);
        own(&mut g);
        assert_eq!((g.turn().coins, g.hand(P0).len()), now, "{card}");
        next_turn(&mut g);
        next_turn(&mut g);
        assert_eq!((g.turn().coins, g.hand(P0).len()), later, "{card}");
    }
}

/// Abundance: "The next time you gain an Action card, +1 Buy and +$3." Played with the
/// Way in a Buy phase after Enlightenment, when Treasures are Actions and nothing replaces
/// what they do.
#[test]
fn way_of_the_chameleon_changes_a_next_time_effect_that_comes_the_same_turn() {
    let mut g = enlightened(Card::WayOfTheChameleon);
    hand(&mut g, P0, &[Card::Abundance, Card::Gold, Card::Gold]);
    deck(&mut g, P0, &[Card::Copper; 10]);
    g.start();
    to_buy(&mut g);
    play(&mut g, Card::Abundance);
    way(&mut g, Card::WayOfTheChameleon);
    play(&mut g, Card::Gold);
    own(&mut g);
    play(&mut g, Card::Gold);
    own(&mut g);
    buy(&mut g, Card::Smithy);
    assert_eq!((g.turn().coins, g.turn().buys), (2, 1));
    assert_eq!(hand_kinds(&g, P0), [Card::Copper; 3]);
}

/// Endless Chalice: "Now and at the start of each of your turns for the rest of the game:
/// $1, +1 Buy." A Treasure's $ are +$ to the Way (wiki, Way of the Chameleon).
#[test]
fn way_of_the_chameleon_changes_only_what_endless_chalice_gives_now() {
    let mut g = enlightened(Card::WayOfTheChameleon);
    hand(&mut g, P0, &[Card::EndlessChalice]);
    deck(&mut g, P0, &[Card::Estate; 20]);
    g.start();
    to_buy(&mut g);
    play(&mut g, Card::EndlessChalice);
    way(&mut g, Card::WayOfTheChameleon);
    assert_eq!((g.turn().coins, g.turn().buys), (0, 2));
    assert_eq!(hand_kinds(&g, P0), [Card::Estate]);
    next_turn(&mut g);
    next_turn(&mut g);
    assert_eq!((g.turn().coins, g.turn().buys), (1, 2));
    assert_eq!(g.hand(P0).len(), 5);
}

/// Harbor Village: "After the next Action you play this turn, if it gave you +$, +$1."
/// The Way affects that +$1 too, though the effect is over by the time it is given
/// (wiki, Harbor Village).
#[test]
fn way_of_the_chameleon_changes_harbor_villages_later_coin() {
    let mut g = way_game(
        Card::WayOfTheChameleon,
        &[Card::HarborVillage, Card::Festival],
    );
    hand(&mut g, P0, &[Card::HarborVillage, Card::Festival]);
    deck(&mut g, P0, &[Card::Gold; 5]);
    g.start();
    play(&mut g, Card::HarborVillage);
    way(&mut g, Card::WayOfTheChameleon);
    assert_eq!((g.turn().coins, g.hand(P0).len()), (1, 1));
    play(&mut g, Card::Festival);
    own(&mut g);
    // $1 for Harbor Village's +1 Card and Festival's $2; Harbor Village's +$1 is a card.
    assert_eq!(g.turn().coins, 3);
    assert_eq!(hand_kinds(&g, P0), [Card::Gold]);
    let harbor_village = Event::Resolved {
        player: P0,
        card: Card::HarborVillage,
    };
    let drew = Event::Drew {
        player: P0,
        count: 1,
    };
    assert!(logged_in_order(&g, &[harbor_village, drew]));
    assert!(!g.log().iter().any(|e| matches!(e, Event::Bonus { .. })));
}

/// Harbor Village asks what the next Action gave after the Way changed it (wiki, Way of
/// the Chameleon).
#[test]
fn harbor_village_counts_what_a_card_gave_after_way_of_the_chameleon() {
    for (next, coins) in [
        (Card::Smithy, 3 + 1),
        (Card::Festival, 0),
        (Card::Moneylender, 0),
    ] {
        let mut g = way_game(Card::WayOfTheChameleon, &[Card::HarborVillage, next]);
        hand(&mut g, P0, &[Card::HarborVillage, next, Card::Copper]);
        deck(&mut g, P0, &[Card::Estate; 6]);
        g.start();
        play(&mut g, Card::HarborVillage);
        own(&mut g);
        play(&mut g, next);
        way(&mut g, Card::WayOfTheChameleon);
        if next == Card::Moneylender {
            choose_kinds(&mut g, &[Card::Copper]);
        }
        assert_eq!(g.turn().coins, coins, "{next}");
    }
}

/// Taskmaster: "+1 Action, +$1, and if you gain a card costing exactly $5 this turn, then
/// at the start of your next turn, repeat this ability." Only what the Way's turn gives is
/// exchanged, and the repeat is no play a Way could be used on (wiki, Taskmaster).
#[test]
fn way_of_the_chameleon_does_not_change_taskmasters_repeat_on_a_later_turn() {
    let mut g = way_game(Card::WayOfTheChameleon, &[Card::Taskmaster, Card::Market]);
    hand(&mut g, P0, &[Card::Taskmaster, Card::Gold, Card::Silver]);
    deck(&mut g, P0, &[Card::Copper; 20]);
    g.start();
    play(&mut g, Card::Taskmaster);
    way(&mut g, Card::WayOfTheChameleon);
    assert_eq!((g.turn().coins, g.hand(P0).len()), (0, 3));
    to_buy(&mut g);
    play(&mut g, Card::Gold);
    play(&mut g, Card::Silver);
    // The turn ends with its only Buy.
    buy(&mut g, Card::Market);
    next_turn(&mut g);
    assert_eq!(g.turn().player, P0);
    assert_eq!((g.turn().coins, g.hand(P0).len()), (1, 5));
}

/// +$ on another player's turn is still +$, so the Way makes it +Cards there, as much
/// for $ given in the middle of a card's instructions as for its "+$2"; and +Cards are
/// +$, which another player's turn has no use for.
#[test]
fn way_of_the_chameleon_exchanges_on_another_players_turn() {
    let kingdom = [Card::Festival, Card::Moneylender, Card::Smithy];
    let mut g = way_game(Card::WayOfTheChameleon, &kingdom);
    hand(
        &mut g,
        P1,
        &[
            Card::Festival,
            Card::Moneylender,
            Card::Smithy,
            Card::Copper,
        ],
    );
    deck(&mut g, P1, &[Card::Silver; 10]);
    g.start();
    play_out_of_turn(&mut g, P1, Card::Festival);
    way(&mut g, Card::WayOfTheChameleon);
    assert_eq!(g.hand(P1).len(), 3 + 2);
    play_out_of_turn(&mut g, P1, Card::Moneylender);
    way(&mut g, Card::WayOfTheChameleon);
    choose_kinds(&mut g, &[Card::Copper]);
    assert_eq!(g.hand(P1).len(), 3 + 3);
    play_out_of_turn(&mut g, P1, Card::Smithy);
    way(&mut g, Card::WayOfTheChameleon);
    assert_eq!((g.hand(P1).len(), g.turn().coins), (5, 0));
}

/// Capital City: "You may discard 2 cards for +$2", so with the Way for +2 Cards (wiki,
/// Capital City): the cards are discarded first, and can be the ones drawn.
#[test]
fn way_of_the_chameleon_draws_for_capital_city_after_its_discard() {
    let mut g = way_game(Card::WayOfTheChameleon, &[Card::CapitalCity]);
    hand(&mut g, P0, &[Card::CapitalCity, Card::Estate, Card::Estate]);
    deck(&mut g, P0, &[]);
    discard(&mut g, P0, &[]);
    g.start();
    play(&mut g, Card::CapitalCity);
    way(&mut g, Card::WayOfTheChameleon);
    yes(&mut g);
    assert_eq!(hand_kinds(&g, P0), [Card::Estate, Card::Estate]);
    assert_eq!(g.turn().coins, 1);
}

/// Elder lets a card played with the Way do one more of its choices (wiki, Elder). The
/// cards that +$ became are in the hand for what the card does next.
#[test]
fn way_of_the_chameleon_draws_before_the_rest_of_what_an_elders_card_does() {
    let elder = |card: Card| {
        let mut g = way_game(Card::WayOfTheChameleon, &[Card::Townsfolk, card]);
        hand(&mut g, P0, &[Card::Elder, card, Card::Estate, Card::Estate]);
        deck(&mut g, P0, &[Card::Gold; 10]);
        hand(&mut g, P1, &[Card::Copper; 4]);
        g.start();
        play(&mut g, Card::Elder);
        own(&mut g);
        choose_kinds(&mut g, &[card]);
        way(&mut g, Card::WayOfTheChameleon);
        g
    };
    // Minion: +$2 is +2 Cards, which "discard your hand" discards; +4 Cards is +$4.
    let mut g = elder(Card::Minion);
    modes(&mut g, &[0, 1]);
    assert_eq!(g.turn().coins, 2 + 4);
    assert_eq!(hand_kinds(&g, P0), []);
    // Steward: +$2 is +2 Cards, which can be the 2 cards trashed.
    let mut g = elder(Card::Steward);
    modes(&mut g, &[1, 2]);
    assert_eq!(kinds(&g, &candidates(&g))[2..], [Card::Gold; 2]);
    choose_kinds(&mut g, &[Card::Gold, Card::Gold]);
    assert_eq!(hand_kinds(&g, P0), [Card::Estate; 2]);
    // Amulet: +$1 is +1 Card, which can be the card trashed.
    let mut g = elder(Card::Amulet);
    modes(&mut g, &[0, 1]);
    choose_kinds(&mut g, &[Card::Gold]);
    assert_eq!(hand_kinds(&g, P0), [Card::Estate; 2]);
    assert_eq!(g.turn().coins, 2);
}

/// The Way is used on a play, not on a card: Scepter replays a Storyteller that isn't
/// finished (wiki, Scepter), and that play, made without the Way, is as printed.
#[test]
fn way_of_the_chameleon_does_not_change_another_play_of_the_same_card() {
    let mut g = way_game(Card::WayOfTheChameleon, &[Card::Storyteller, Card::Scepter]);
    hand(&mut g, P0, &[Card::Storyteller, Card::Scepter]);
    deck(&mut g, P0, &[Card::Estate; 10]);
    g.start();
    play(&mut g, Card::Storyteller);
    way(&mut g, Card::WayOfTheChameleon);
    choose_kinds(&mut g, &[Card::Scepter]);
    modes(&mut g, &[1]);
    own(&mut g);
    // The replay: +1 Card, pays $0. Then the first play: +$1 (for +1 Card), pays it, +$1.
    assert_eq!(hand_kinds(&g, P0), [Card::Estate]);
    assert_eq!(g.turn().coins, 1);

    // Throne Room's two plays of a Merchant: only the first one's later +$1 is a card.
    let mut g = way_game(Card::WayOfTheChameleon, &[Card::ThroneRoom, Card::Merchant]);
    hand(
        &mut g,
        P0,
        &[Card::ThroneRoom, Card::Merchant, Card::Silver],
    );
    deck(&mut g, P0, &[Card::Estate; 10]);
    g.start();
    play(&mut g, Card::ThroneRoom);
    own(&mut g);
    choose_kinds(&mut g, &[Card::Merchant]);
    way(&mut g, Card::WayOfTheChameleon);
    own(&mut g);
    assert_eq!((g.turn().coins, g.hand(P0).len()), (1, 2));
    play_treasure(&mut g, Card::Silver);
    assert_eq!((g.turn().coins, g.hand(P0).len()), (1 + 2 + 1, 2));
}

/// A game saved in the middle of a play using the Way goes on exchanging, and a save
/// without the fields the exchange keeps still loads.
#[test]
fn way_of_the_chameleon_goes_on_in_a_saved_game() {
    let kingdom = [Card::Merchant, Card::Moneylender];
    let mut g = way_game(Card::WayOfTheChameleon, &kingdom);
    hand(
        &mut g,
        P0,
        &[
            Card::Merchant,
            Card::Moneylender,
            Card::Copper,
            Card::Silver,
        ],
    );
    deck(&mut g, P0, &[Card::Gold; 5]);
    g.start();
    play(&mut g, Card::Merchant);
    way(&mut g, Card::WayOfTheChameleon);
    play(&mut g, Card::Moneylender);
    way(&mut g, Card::WayOfTheChameleon);
    let json = serde_json::to_string(&g).unwrap();
    let mut g: Game = serde_json::from_str(&json).unwrap();
    choose_kinds(&mut g, &[Card::Copper]);
    play_treasure(&mut g, Card::Silver);
    assert_eq!(g.turn().coins, 1 + 2);
    assert_eq!(hand_kinds(&g, P0), [Card::Gold; 4]);

    fn strip(v: &mut serde_json::Value) {
        match v {
            serde_json::Value::Object(o) => {
                o.remove("exchange");
                o.remove("chameleon_effects");
                o.remove("chameleon_turn");
                o.values_mut().for_each(strip);
            }
            serde_json::Value::Array(a) => a.iter_mut().for_each(strip),
            _ => {}
        }
    }
    let mut old: serde_json::Value = serde_json::from_str(&json).unwrap();
    strip(&mut old);
    let mut g: Game = serde_json::from_value(old).unwrap();
    choose_kinds(&mut g, &[Card::Copper]);
    assert_eq!(g.turn().coins, 1 + 3);
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

/// The Way follows the play it was used for: a card that left play and came back has
/// moved, so the effect can't put it onto the deck (the stop-moving rule).
#[test]
fn way_of_the_frog_cannot_move_a_card_that_left_play_and_came_back() {
    let kingdom = [Card::Village, Card::Smithy, Card::Festival];
    let mut g = way_game(Card::WayOfTheFrog, &kingdom);
    hand(&mut g, P0, &kingdom);
    deck(&mut g, P0, &[Card::Copper; 10]);
    g.start();
    play(&mut g, Card::Festival);
    own(&mut g);
    play(&mut g, Card::Smithy);
    way(&mut g, Card::WayOfTheFrog);
    leave_play(&mut g, P0, Card::Smithy);
    let smithy = *g.trash().last().unwrap();
    g.move_card(smithy, Zone::Hand(P0), crate::game::Pos::Top);
    play(&mut g, Card::Village);
    own(&mut g);
    play(&mut g, Card::Smithy);
    own(&mut g);
    next_turn(&mut g);
    assert_eq!(g.zone(smithy), Zone::Discard(P0));
}

#[test]
fn cards_played_with_way_of_the_frog_go_onto_the_deck_in_a_chosen_order() {
    let mut g = way_game(Card::WayOfTheFrog, &[Card::Village, Card::Smithy]);
    hand(&mut g, P0, &[Card::Village, Card::Smithy]);
    deck(&mut g, P0, &[Card::Copper; 10]);
    g.start();
    g.turn.cleanup_draw = 1;
    play(&mut g, Card::Village);
    way(&mut g, Card::WayOfTheFrog);
    play(&mut g, Card::Smithy);
    way(&mut g, Card::WayOfTheFrog);
    to_buy(&mut g);
    end_phase(&mut g);
    assert_eq!(
        trigger_labels(&g),
        ["Way of the Frog (Village)", "Way of the Frog (Smithy)"]
    );
    respond(&mut g, Response::option(Opt::Trigger(1)));
    // The Village went on last, so it is the card drawn.
    assert_eq!(hand_kinds(&g, P0), [Card::Village]);
    assert_eq!(deck_kinds(&g, P0)[0], Card::Smithy);
}

// A card played on another player's turn is discarded from play in that turn's Clean-up.
#[test]
fn way_of_the_frog_used_out_of_turn_puts_the_card_onto_its_owners_deck() {
    let mut g = way_game(Card::WayOfTheFrog, &[Card::BlackCat]);
    hand(&mut g, P0, &[Card::Copper, Card::Copper]);
    hand(&mut g, P1, &[Card::BlackCat]);
    g.start();
    play_treasures(&mut g);
    buy(&mut g, Card::Estate);
    trigger(&mut g, Card::BlackCat);
    way(&mut g, Card::WayOfTheFrog);
    assert_eq!(g.turn().player, P1);
    assert_eq!(deck_kinds(&g, P1).first(), Some(&Card::BlackCat));
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
fn way_of_the_horse_after_a_haven_played_normally_strands_its_card() {
    let mut g = way_game(Card::WayOfTheHorse, &[Card::ThroneRoom, Card::Haven]);
    hand(&mut g, P0, &[Card::ThroneRoom, Card::Haven, Card::Gold]);
    deck(&mut g, P0, &[Card::Copper; 20]);
    g.start();
    play(&mut g, Card::ThroneRoom);
    own(&mut g);
    choose_kinds(&mut g, &[Card::Haven]);
    own(&mut g);
    choose_kinds(&mut g, &[Card::Gold]);
    way(&mut g, Card::WayOfTheHorse);
    next_turn(&mut g);
    // Nothing is left for the Throne Room to stay in play for.
    assert!(g.in_play(P0).is_empty());
    next_turn(&mut g);
    assert_eq!(kinds(&g, &g.player(P0).set_aside), [Card::Gold]);
    assert_eq!(g.count_owned(P0, Card::Gold), 1);
    assert!(g.lasting().is_empty());
}

#[test]
fn a_duration_played_after_way_of_the_horse_returned_it_does_nothing_later() {
    let mut g = way_game(Card::WayOfTheHorse, &[Card::ThroneRoom, Card::Wharf]);
    hand(&mut g, P0, &[Card::ThroneRoom, Card::Wharf]);
    deck(&mut g, P0, &[Card::Copper; 20]);
    g.start();
    play(&mut g, Card::ThroneRoom);
    own(&mut g);
    choose_kinds(&mut g, &[Card::Wharf]);
    way(&mut g, Card::WayOfTheHorse);
    own(&mut g);
    assert_eq!((g.hand(P0).len(), g.turn().buys), (4, 2));
    next_turn(&mut g);
    assert!(g.in_play(P0).is_empty());
    next_turn(&mut g);
    assert_eq!(g.turn().player, P0);
    assert_eq!((g.hand(P0).len(), g.turn().buys), (5, 1));
}

#[test]
fn way_of_the_horse_returns_a_card_to_its_empty_pile() {
    let mut g = way_game(Card::WayOfTheHorse, &[Card::Smithy]);
    let pile = g.pile_of(Card::Smithy).unwrap();
    for c in g.pile(pile).cards.clone() {
        g.move_card(c, Zone::Trash, crate::game::Pos::Top);
    }
    hand(&mut g, P0, &[Card::Smithy]);
    deck(&mut g, P0, &[Card::Copper; 5]);
    g.start();
    play(&mut g, Card::Smithy);
    way(&mut g, Card::WayOfTheHorse);
    assert_eq!(pile_len(&g, Card::Smithy), 1);
    assert_eq!((g.turn().actions, g.hand(P0).len()), (1, 2));
}

#[test]
fn way_of_the_horse_draws_what_there_is() {
    let mut g = way_game(Card::WayOfTheHorse, &[Card::Smithy]);
    hand(&mut g, P0, &[Card::Smithy]);
    deck(&mut g, P0, &[]);
    discard(&mut g, P0, &[Card::Gold]);
    g.start();
    play(&mut g, Card::Smithy);
    way(&mut g, Card::WayOfTheHorse);
    assert_eq!(hand_kinds(&g, P0), [Card::Gold]);
    assert_eq!(g.turn().actions, 1);
    assert_eq!(pile_len(&g, Card::Smithy), 11);
}

/// King's Court plays a Caravan as itself, then with Way of the Horse, which returns it to
/// its pile, then with Way of the Chameleon: the last play still happens, and nothing is
/// left for the next turn.
#[test]
fn kings_court_uses_a_way_on_each_play_separately() {
    let mut g = setup_with_landscapes(
        &[Card::KingsCourt, Card::Caravan],
        &[Card::WayOfTheHorse, Card::WayOfTheChameleon],
    );
    hand(&mut g, P0, &[Card::KingsCourt, Card::Caravan]);
    deck(&mut g, P0, &[Card::Copper; 20]);
    g.start();
    play(&mut g, Card::KingsCourt);
    own(&mut g);
    choose_kinds(&mut g, &[Card::Caravan]);
    own(&mut g);
    assert_eq!(g.hand(P0).len(), 1);
    way(&mut g, Card::WayOfTheHorse);
    assert_eq!(g.hand(P0).len(), 3);
    assert_eq!(pile_len(&g, Card::Caravan), 11);
    way(&mut g, Card::WayOfTheChameleon);
    assert_eq!(g.hand(P0).len(), 3);
    assert_eq!((g.turn().actions, g.turn().coins), (3, 1));
    assert_eq!(in_play_kinds(&g, P0), [Card::KingsCourt]);
    next_turn(&mut g);
    assert!(g.in_play(P0).is_empty());
    next_turn(&mut g);
    assert_eq!(g.turn().player, P0);
    assert_eq!(g.hand(P0).len(), 5);
    assert!(g.lasting().is_empty());
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
    let mouse = g.ext.menagerie.mouse.unwrap();
    let holder = g.zone(mouse);
    g.start();
    play(&mut g, Card::Smithy);
    way(&mut g, Card::WayOfTheMouse);
    assert_eq!(g.hand(P0).len(), 2);
    assert_eq!(g.zone(mouse), holder);
}

/// "In games using this" holds while Shaman is only the card Way of the Mouse set aside
/// (wiki, In games using this): the Way silences the rest of what is below its line.
#[test]
fn a_shaman_set_aside_by_way_of_the_mouse_is_used_by_the_game() {
    let mut g = way_game(Card::WayOfTheMouse, &[Card::Smithy]);
    set_mouse(&mut g, Card::Shaman);
    replace(&mut g, Zone::Trash, &[Card::Silver]);
    g.start();
    assert_eq!(gained(&g, P0), [Card::Silver]);
}

/// A game whose Way of the Mouse card is `mouse`, with P0 about to play a Village.
fn mouse_game(mouse: Card) -> Game {
    let mut g = setup_with(GameConfig {
        colonies: Some(false),
        shelters: Some(false),
        landscapes: vec![Card::WayOfTheMouse],
        setup_choices: [(Card::WayOfTheMouse, vec![mouse])].into(),
        ..GameConfig::new(2, vec![Card::Village], 7)
    });
    assert_eq!(g.card(g.ext.menagerie.mouse.unwrap()), mouse);
    hand(&mut g, P0, &[Card::Village, Card::Copper]);
    hand(&mut g, P1, &[Card::Estate; 5]);
    g.start();
    g
}

#[test]
fn way_of_the_mouses_gladiator_has_no_pile_to_trash_from() {
    let mut g = mouse_game(Card::Gladiator);
    let mouse = g.ext.menagerie.mouse.unwrap();
    let holder = g.zone(mouse);
    play(&mut g, Card::Village);
    way(&mut g, Card::WayOfTheMouse);
    // P0 reveals the Copper and P1 has none: +$1, but no Gladiator pile.
    assert_eq!(g.turn().coins, 3);
    assert_eq!(g.zone(mouse), holder);
}

#[test]
fn way_of_the_mouses_farmers_market_has_no_pile_to_put_vp_on() {
    let mut g = mouse_game(Card::FarmersMarket);
    play(&mut g, Card::Village);
    way(&mut g, Card::WayOfTheMouse);
    assert_eq!((g.turn().coins, g.turn().buys), (0, 2));
    assert!(g.piles().iter().all(|p| p.vp_tokens == 0));
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
fn cards_set_aside_by_way_of_the_turtle_are_told_apart_when_ordering_their_plays() {
    let mut g = way_game(Card::WayOfTheTurtle, &[Card::Village, Card::Smithy]);
    hand(&mut g, P0, &[Card::Village, Card::Village, Card::Smithy]);
    deck(&mut g, P0, &[Card::Copper; 20]);
    g.start();
    play(&mut g, Card::Village);
    own(&mut g);
    play(&mut g, Card::Village);
    way(&mut g, Card::WayOfTheTurtle);
    play(&mut g, Card::Smithy);
    way(&mut g, Card::WayOfTheTurtle);
    next_turn(&mut g);
    next_turn(&mut g);
    assert_eq!(
        trigger_labels(&g),
        ["Way of the Turtle (Village)", "Way of the Turtle (Smithy)"]
    );
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

/// Fortune: "Double your $": not "+$", which is all the Way exchanges (as it leaves
/// "draw until" alone). The -$1 token does apply to it (wiki, Fortune).
#[test]
fn way_of_the_chameleon_does_not_change_fortunes_doubling() {
    let mut g = enlightened(Card::WayOfTheChameleon);
    hand(&mut g, P0, &[Card::Gold, Card::Fortune]);
    deck(&mut g, P0, &[Card::Copper; 5]);
    g.start();
    to_buy(&mut g);
    play(&mut g, Card::Gold);
    own(&mut g);
    g.players[0].minus_coin = true;
    play(&mut g, Card::Fortune);
    way(&mut g, Card::WayOfTheChameleon);
    assert_eq!((g.turn().coins, g.turn().buys), (3 + 3 - 1, 2));
    assert!(g.hand(P0).is_empty());
}

#[test]
fn a_way_replaces_the_coin_a_curse_makes_with_charlatan() {
    for (way_used, coins, cards) in [(Card::WayOfTheOx, 0, 0), (Card::WayOfTheChameleon, 0, 1)] {
        let mut g = setup_with_landscapes(&[Card::Charlatan], &[way_used]);
        g.prophecy = Some((Card::Enlightenment, 0));
        hand(&mut g, P0, &[Card::Curse]);
        deck(&mut g, P0, &[Card::Copper; 5]);
        g.start();
        to_buy(&mut g);
        // A Treasure (Charlatan), so an Action (Enlightenment).
        play(&mut g, Card::Curse);
        way(&mut g, way_used);
        assert_eq!(
            (g.turn().coins, g.hand(P0).len()),
            (coins, cards),
            "{way_used}"
        );
    }
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

/// Returning a Peddler takes an Action out of play, so it costs $2 more once returned.
#[test]
fn butterfly_gains_for_what_the_returned_card_costs_once_returned() {
    let mut g = setup_with(GameConfig {
        colonies: Some(false),
        shelters: Some(false),
        landscapes: vec![Card::Capitalism, Card::WayOfTheButterfly],
        ..GameConfig::new(
            2,
            vec![
                Card::Peddler,
                Card::Village,
                Card::KingsCourt,
                Card::Laboratory,
            ],
            7,
        )
    });
    g.players[0].projects.push(Card::Capitalism);
    hand(&mut g, P0, &[Card::Village, Card::Peddler]);
    g.start();
    play(&mut g, Card::Village);
    no(&mut g);
    if g.turn().phase == Phase::Action {
        end_phase(&mut g);
    }
    play(&mut g, Card::Peddler);
    respond(&mut g, Response::option(Opt::Card(Card::WayOfTheButterfly)));
    assert_eq!(g.cost(Card::Peddler), Cost::coins(4));
    yes(&mut g);
    assert_eq!(g.cost(Card::Peddler), Cost::coins(6));
    // King's Court is the only card costing exactly $7.
    assert_eq!(kinds(&g, &g.player(P0).discard), [Card::KingsCourt]);
}
