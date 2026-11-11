use super::*;
use rand::seq::SliceRandom;
use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;

/// An unstarted 2-player game with `ally` as the game's Ally.
fn game_with(kingdom: &[Card], ally: Card) -> Game {
    setup_with(GameConfig {
        colonies: Some(false),
        shelters: Some(false),
        landscapes: vec![ally],
        ..GameConfig::new(2, kingdom.to_vec(), 7)
    })
}

/// An unstarted game whose Ally (Plateau Shepherds) only matters when scoring.
fn game(kingdom: &[Card]) -> Game {
    game_with(kingdom, Card::PlateauShepherds)
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

fn moves(g: &Game) -> Vec<Move> {
    match &g.pending().expect("a decision is pending").kind {
        DecisionKind::Turn { moves, .. } => moves.clone(),
        k => panic!("expected a Turn decision, got {k:?}"),
    }
}

fn can_play(g: &Game, card: Card) -> bool {
    moves(g)
        .iter()
        .any(|m| matches!(m, Move::Play(c) if g.card(*c) == card))
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

/// Whether a trigger named after `card` is offered now.
fn offered(g: &Game, card: Card) -> bool {
    match g.pending().map(|d| (&d.kind, d.purpose)) {
        Some((DecisionKind::Choose { labels, .. }, Purpose::Trigger)) => {
            labels.iter().any(|l| l.starts_with(card.name()))
        }
        _ => false,
    }
}

/// Declines every pending optional trigger.
fn decline(g: &mut Game) {
    assert_eq!(purpose(g), Purpose::Trigger);
    no(g);
}

fn modes(g: &mut Game, chosen: &[u8]) {
    let opts = chosen.iter().map(|&i| Opt::Mode(i)).collect();
    respond(g, Response::Options(opts));
}

fn name(g: &mut Game, card: Card) {
    respond(g, Response::Name(card));
}

fn in_play_kinds(g: &Game, p: PlayerId) -> Vec<Card> {
    kinds(g, g.in_play(p))
}

fn discard_kinds(g: &Game, p: PlayerId) -> Vec<Card> {
    kinds(g, &g.player(p).discard)
}

fn favors(g: &Game, p: PlayerId) -> u32 {
    g.player(p).favors
}

fn set_favors(g: &mut Game, p: PlayerId, n: u32) {
    g.players[p.index()].favors = n;
}

fn pile_kinds(g: &Game, name: Card) -> Vec<Card> {
    let pile = g.pile_of(name).unwrap();
    g.pile(pile)
        .cards
        .iter()
        .rev()
        .map(|&c| g.card(c))
        .collect()
}

fn top(g: &Game, name: Card) -> Option<Card> {
    g.pile_top(g.pile_of(name).unwrap()).map(|c| g.card(c))
}

fn choose_one_pile(g: &mut Game, top_card: Card) {
    let DecisionKind::Pile { piles, .. } = &g.pending().unwrap().kind else {
        panic!("expected a pile decision, got {:?}", g.pending());
    };
    let pile = *piles
        .iter()
        .find(|&&p| g.pile_top(p).is_some_and(|t| g.card(t) == top_card))
        .unwrap_or_else(|| panic!("no pile topped by {top_card} offered"));
    respond(g, Response::Pile(Some(pile)));
}

// ---- Setup, Favors, split piles ----

#[test]
fn a_kingdom_with_a_liaison_gets_one_random_ally_and_a_favor_each() {
    for seed in 0..10 {
        let g = setup_with(GameConfig::new(
            3,
            vec![Card::Underling, Card::Smithy],
            seed,
        ));
        let allies: Vec<Card> = g
            .landscapes()
            .iter()
            .copied()
            .filter(|c| c.kind() == Kind::Ally)
            .collect();
        assert_eq!(allies.len(), 1);
        for p in 0..3 {
            assert_eq!(favors(&g, PlayerId(p)), 1);
        }
    }
    let g = setup_with(GameConfig::new(2, vec![Card::Smithy], 1));
    assert!(g.landscapes().is_empty());
    assert_eq!(favors(&g, P0), 0);
    let g = game_with(&[Card::Wizards], Card::MountainFolk);
    assert_eq!(g.landscapes(), [Card::MountainFolk]);
    assert_eq!(favors(&g, P1), 1);
}

#[test]
fn importer_gives_each_player_five_favors_at_setup() {
    let g = game(&[Card::Importer]);
    assert_eq!((favors(&g, P0), favors(&g, P1)), (5, 5));
}

#[test]
fn a_random_kingdom_with_a_split_pile_liaison_gets_an_ally() {
    let g = setup_with(GameConfig::new(2, vec![Card::Wizards], 3));
    assert_eq!(g.landscapes().len(), 1);
    assert_eq!(g.landscapes()[0].kind(), Kind::Ally);
}

#[test]
fn split_piles_hold_four_of_each_card_in_order() {
    let g = game(&[
        Card::Augurs,
        Card::Clashes,
        Card::Forts,
        Card::Odysseys,
        Card::Townsfolk,
        Card::Wizards,
    ]);
    let augurs = pile_kinds(&g, Card::Augurs);
    assert_eq!(augurs.len(), 16);
    assert_eq!(
        &augurs[..5],
        &[Card::HerbGatherer; 4]
            .iter()
            .copied()
            .chain([Card::Acolyte])
            .collect::<Vec<_>>()[..]
    );
    assert_eq!(augurs[15], Card::Sibyl);
    assert_eq!(top(&g, Card::Clashes), Some(Card::BattlePlan));
    assert_eq!(top(&g, Card::Forts), Some(Card::Tent));
    assert_eq!(top(&g, Card::Odysseys), Some(Card::OldMap));
    assert_eq!(top(&g, Card::Townsfolk), Some(Card::TownCrier));
    assert_eq!(top(&g, Card::Wizards), Some(Card::Student));
    assert!(g.supply_pile(Card::Augurs).is_some());
}

#[test]
fn only_the_top_card_of_a_split_pile_can_be_gained() {
    let mut g = game(&[Card::Townsfolk, Card::Workshop]);
    hand(&mut g, P0, &[Card::Workshop]);
    g.start();
    play(&mut g, Card::Workshop);
    let DecisionKind::Pile { piles, .. } = &g.pending().unwrap().kind else {
        panic!()
    };
    let townsfolk = g.pile_of(Card::Townsfolk).unwrap();
    assert!(piles.contains(&townsfolk));
    choose_pile(&mut g, Card::Townsfolk);
    assert_eq!(discard_kinds(&g, P0), [Card::TownCrier]);
    assert_eq!(g.pile(townsfolk).cards.len(), 15);
}

#[test]
fn rotating_moves_the_top_card_and_its_copies_to_the_bottom() {
    let mut g = game(&[Card::Townsfolk]);
    let pile = g.pile_of(Card::Townsfolk).unwrap();
    crate::cards::allies::rotate(&mut g, P0, pile);
    let cards = pile_kinds(&g, Card::Townsfolk);
    assert_eq!(cards[0], Card::Blacksmith);
    assert_eq!(&cards[12..], &[Card::TownCrier; 4]);
    assert!(g.log().contains(&Event::Rotated { player: P0, pile }));
}

#[test]
fn a_returned_card_on_top_is_rotated_alone() {
    let mut g = game(&[Card::Wizards, Card::Swap]);
    let pile = g.pile_of(Card::Wizards).unwrap();
    // Student on top of the Conjurers once the Students are gone.
    let students: Vec<CardId> = g.pile(pile).cards[12..].to_vec();
    for c in &students {
        g.move_card(*c, Zone::Trash, crate::game::Pos::Top);
    }
    let student = students[0];
    g.move_card(student, Zone::Pile(pile), crate::game::Pos::Top);
    crate::cards::allies::rotate(&mut g, P0, pile);
    let cards = pile_kinds(&g, Card::Wizards);
    assert_eq!(cards[0], Card::Conjurer);
    assert_eq!(*cards.last().unwrap(), Card::Student);
}

#[test]
fn a_split_pile_is_empty_only_when_all_its_cards_are_gone() {
    let mut g = game(&[Card::Townsfolk]);
    let pile = g.pile_of(Card::Townsfolk).unwrap();
    let cards: Vec<CardId> = g.pile(pile).cards.clone();
    for &c in &cards[1..] {
        g.move_card(c, Zone::Trash, crate::game::Pos::Top);
    }
    assert_eq!(g.empty_supply_piles(), 0);
    g.move_card(cards[0], Zone::Trash, crate::game::Pos::Top);
    assert_eq!(g.empty_supply_piles(), 1);
}

// ---- Kingdom cards ----

#[test]
fn underling_draws_and_gives_a_favor() {
    let mut g = game(&[Card::Underling]);
    hand(&mut g, P0, &[Card::Underling]);
    deck(&mut g, P0, &[Card::Copper; 6]);
    g.start();
    play(&mut g, Card::Underling);
    assert_eq!(g.hand(P0).len(), 1);
    assert_eq!(g.turn().actions, 1);
    assert_eq!(favors(&g, P0), 2);
}

#[test]
fn bauble_chooses_two_different_options_and_can_topdeck_gains() {
    let mut g = game(&[Card::Bauble]);
    hand(&mut g, P0, &[Card::Bauble, Card::Gold]);
    deck(&mut g, P0, &[Card::Copper; 6]);
    g.start();
    play(&mut g, Card::Bauble);
    assert_eq!(
        g.respond(Response::Options(vec![Opt::Mode(0), Opt::Mode(0)])),
        Err(ResponseError::DuplicateOption(Opt::Mode(0)))
    );
    modes(&mut g, &[0, 3]);
    assert_eq!(g.turn().buys, 2);
    play(&mut g, Card::Gold);
    buy(&mut g, Card::Silver);
    assert!(offered(&g, Card::Bauble));
    accept(&mut g, Card::Bauble);
    assert_eq!(deck_kinds(&g, P0)[0], Card::Silver);
    buy(&mut g, Card::Copper);
    decline(&mut g);
    // Clean-up drew the Silver from the deck; the Copper was discarded.
    assert!(hand_kinds(&g, P0).contains(&Card::Silver));
    assert!(discard_kinds(&g, P0).contains(&Card::Copper));
}

#[test]
fn bauble_gives_a_coin_and_a_favor() {
    let mut g = game(&[Card::Bauble]);
    hand(&mut g, P0, &[Card::Bauble]);
    g.start();
    play(&mut g, Card::Bauble);
    modes(&mut g, &[1, 2]);
    assert_eq!(g.turn().coins, 1);
    assert_eq!(favors(&g, P0), 2);
}

#[test]
fn sycophant_discards_three_for_three_coins() {
    let mut g = game(&[Card::Sycophant]);
    hand(
        &mut g,
        P0,
        &[
            Card::Sycophant,
            Card::Estate,
            Card::Estate,
            Card::Copper,
            Card::Copper,
        ],
    );
    g.start();
    play(&mut g, Card::Sycophant);
    choose_kinds(&mut g, &[Card::Estate, Card::Estate, Card::Copper]);
    assert_eq!(g.turn().coins, 3);
    assert_eq!(g.turn().actions, 1);
    assert_eq!(g.hand(P0).len(), 1);
}

#[test]
fn sycophant_with_one_card_still_gives_three_and_with_none_nothing() {
    let mut g = game(&[Card::Sycophant]);
    hand(&mut g, P0, &[Card::Sycophant, Card::Estate]);
    g.start();
    play(&mut g, Card::Sycophant);
    assert_eq!(g.turn().coins, 3);
    let mut g = game(&[Card::Sycophant]);
    hand(&mut g, P0, &[Card::Sycophant]);
    g.start();
    play(&mut g, Card::Sycophant);
    assert_eq!(g.turn().coins, 0);
}

#[test]
fn sycophant_gives_two_favors_when_gained_or_trashed() {
    let mut g = game(&[Card::Sycophant, Card::Chapel]);
    hand(&mut g, P0, &[Card::Chapel, Card::Sycophant, Card::Silver]);
    g.start();
    play(&mut g, Card::Chapel);
    choose_kinds(&mut g, &[Card::Sycophant]);
    assert_eq!(favors(&g, P0), 3);
    play(&mut g, Card::Silver);
    buy(&mut g, Card::Sycophant);
    assert_eq!(favors(&g, P0), 5);
}

#[test]
fn importer_gains_a_card_costing_up_to_five_next_turn() {
    let mut g = game(&[Card::Importer, Card::Laboratory]);
    hand(&mut g, P0, &[Card::Importer]);
    deck(&mut g, P0, &[Card::Copper; 10]);
    g.start();
    play(&mut g, Card::Importer);
    next_turn(&mut g);
    assert_eq!(in_play_kinds(&g, P0), [Card::Importer]);
    next_turn(&mut g);
    choose_pile(&mut g, Card::Laboratory);
    assert_eq!(discard_kinds(&g, P0), [Card::Laboratory]);
    next_turn(&mut g);
    assert!(g.in_play(P0).is_empty());
}

#[test]
fn merchant_camp_may_go_onto_the_deck_when_discarded_from_play() {
    let mut g = game(&[Card::MerchantCamp]);
    hand(&mut g, P0, &[Card::MerchantCamp, Card::MerchantCamp]);
    deck(&mut g, P0, &[Card::Copper; 10]);
    g.start();
    play(&mut g, Card::MerchantCamp);
    play(&mut g, Card::MerchantCamp);
    assert_eq!((g.turn().actions, g.turn().coins), (3, 2));
    to_buy(&mut g);
    end_phase(&mut g);
    // One of the two goes onto the deck, the other is discarded.
    accept(&mut g, Card::MerchantCamp);
    decline(&mut g);
    assert_eq!(g.count_owned(P0, Card::MerchantCamp), 2);
    assert!(hand_kinds(&g, P0).contains(&Card::MerchantCamp));
    assert_eq!(
        discard_kinds(&g, P0)
            .iter()
            .filter(|&&c| c == Card::MerchantCamp)
            .count(),
        1
    );
}

#[test]
fn sentinel_trashes_up_to_two_of_five_and_puts_the_rest_back() {
    let mut g = game(&[Card::Sentinel]);
    hand(&mut g, P0, &[Card::Sentinel]);
    deck(
        &mut g,
        P0,
        &[
            Card::Curse,
            Card::Estate,
            Card::Gold,
            Card::Silver,
            Card::Copper,
            Card::Duchy,
        ],
    );
    g.start();
    play(&mut g, Card::Sentinel);
    assert_eq!(candidates(&g).len(), 5);
    choose_kinds(&mut g, &[Card::Curse, Card::Estate]);
    choose_kinds(&mut g, &[Card::Copper, Card::Gold, Card::Silver]);
    assert_eq!(
        deck_kinds(&g, P0),
        [Card::Copper, Card::Gold, Card::Silver, Card::Duchy]
    );
    assert_eq!(
        kinds(&g, &g.trash()[g.trash().len() - 2..]),
        [Card::Curse, Card::Estate]
    );
}

#[test]
fn sentinel_shuffles_and_looks_at_what_there_is() {
    let mut g = game(&[Card::Sentinel]);
    hand(&mut g, P0, &[Card::Sentinel]);
    deck(&mut g, P0, &[Card::Copper]);
    discard(&mut g, P0, &[Card::Estate, Card::Estate]);
    g.start();
    play(&mut g, Card::Sentinel);
    assert_eq!(candidates(&g).len(), 3);
}

#[test]
fn broker_trashes_a_card_for_one_thing_per_coin_it_costs() {
    let mut g = game(&[Card::Broker]);
    hand(
        &mut g,
        P0,
        &[Card::Broker, Card::Broker, Card::Estate, Card::Silver],
    );
    deck(&mut g, P0, &[Card::Copper; 6]);
    g.start();
    play(&mut g, Card::Broker);
    choose_kinds(&mut g, &[Card::Estate]);
    modes(&mut g, &[1]);
    assert_eq!(g.turn().actions, 2);
    play(&mut g, Card::Broker);
    modes(&mut g, &[3]);
    assert_eq!(favors(&g, P0), 4);
}

#[test]
fn broker_gets_nothing_for_a_potion_in_the_cost() {
    let mut g = game(&[Card::Broker]);
    hand(&mut g, P0, &[Card::Broker, Card::Vineyard]);
    g.start();
    play(&mut g, Card::Broker);
    modes(&mut g, &[2]);
    assert_eq!(g.turn().coins, 0);
}

#[test]
fn carpenter_gives_an_action_and_gains_while_no_pile_is_empty() {
    let mut g = game(&[Card::Carpenter, Card::Smithy]);
    hand(&mut g, P0, &[Card::Carpenter, Card::Estate]);
    g.start();
    play(&mut g, Card::Carpenter);
    assert_eq!(g.turn().actions, 1);
    choose_pile(&mut g, Card::Smithy);
    assert_eq!(discard_kinds(&g, P0), [Card::Smithy]);
}

#[test]
fn carpenter_with_an_empty_pile_trashes_and_upgrades() {
    let mut g = game(&[Card::Carpenter, Card::Smithy, Card::Laboratory]);
    let smithy = g.supply_pile(Card::Smithy).unwrap();
    for c in g.pile(smithy).cards.clone() {
        g.move_card(c, Zone::Trash, crate::game::Pos::Top);
    }
    hand(&mut g, P0, &[Card::Carpenter, Card::Silver]);
    g.start();
    play(&mut g, Card::Carpenter);
    assert_eq!(g.turn().actions, 0);
    choose_pile(&mut g, Card::Laboratory);
    assert_eq!(discard_kinds(&g, P0), [Card::Laboratory]);
    assert!(kinds(&g, g.trash()).contains(&Card::Silver));
}

#[test]
fn courier_discards_the_top_card_and_may_play_a_card_from_the_discard_pile() {
    let mut g = game(&[Card::Courier]);
    hand(&mut g, P0, &[Card::Courier]);
    deck(&mut g, P0, &[Card::Gold, Card::Copper]);
    discard(&mut g, P0, &[Card::Estate, Card::Village]);
    g.start();
    play(&mut g, Card::Courier);
    assert_eq!(g.turn().coins, 1);
    assert_eq!(kinds(&g, &candidates(&g)), [Card::Village, Card::Gold]);
    choose_kinds(&mut g, &[Card::Gold]);
    assert_eq!(g.turn().coins, 4);
    assert_eq!(in_play_kinds(&g, P0), [Card::Courier, Card::Gold]);
}

#[test]
fn innkeeper_draws_and_discards_by_its_mode() {
    let mut g = game(&[Card::Innkeeper]);
    hand(
        &mut g,
        P0,
        &[Card::Innkeeper, Card::Innkeeper, Card::Estate],
    );
    deck(&mut g, P0, &[Card::Copper; 12]);
    g.start();
    play(&mut g, Card::Innkeeper);
    modes(&mut g, &[2]);
    assert_eq!(g.hand(P0).len(), 7);
    choose_kinds(
        &mut g,
        &[
            Card::Estate,
            Card::Copper,
            Card::Copper,
            Card::Copper,
            Card::Copper,
            Card::Copper,
        ],
    );
    assert_eq!(hand_kinds(&g, P0), [Card::Innkeeper]);
    play(&mut g, Card::Innkeeper);
    modes(&mut g, &[0]);
    assert_eq!(g.hand(P0).len(), 1);
    assert_eq!(g.turn().actions, 1);
}

#[test]
fn royal_galley_plays_an_action_now_and_again_next_turn() {
    let mut g = game(&[Card::RoyalGalley, Card::Village]);
    hand(&mut g, P0, &[Card::RoyalGalley, Card::Village]);
    deck(&mut g, P0, &[Card::Copper; 20]);
    g.start();
    play(&mut g, Card::RoyalGalley);
    choose_kinds(&mut g, &[Card::Village]);
    assert_eq!(g.turn().actions, 2);
    next_turn(&mut g);
    assert_eq!(in_play_kinds(&g, P0), [Card::RoyalGalley, Card::Village]);
    next_turn(&mut g);
    assert_eq!(g.turn().actions, 3);
    assert_eq!(g.hand(P0).len(), 6);
    next_turn(&mut g);
    assert!(g.in_play(P0).is_empty());
}

#[test]
fn royal_galley_offers_only_non_durations_and_playing_nothing_is_discarded() {
    let mut g = game(&[Card::RoyalGalley, Card::Feast]);
    hand(&mut g, P0, &[Card::RoyalGalley, Card::Feast, Card::Caravan]);
    deck(&mut g, P0, &[Card::Copper; 20]);
    g.start();
    play(&mut g, Card::RoyalGalley);
    assert_eq!(kinds(&g, &candidates(&g)), [Card::Feast]);
    choose(&mut g, &[]);
    next_turn(&mut g);
    assert!(g.in_play(P0).is_empty());
}

#[test]
fn royal_galley_is_discarded_when_the_card_it_played_left_play() {
    let mut g = game(&[Card::RoyalGalley, Card::Feast]);
    hand(&mut g, P0, &[Card::RoyalGalley, Card::Feast]);
    deck(&mut g, P0, &[Card::Copper; 20]);
    g.start();
    play(&mut g, Card::RoyalGalley);
    choose_kinds(&mut g, &[Card::Feast]);
    choose_pile(&mut g, Card::Silver);
    next_turn(&mut g);
    assert!(g.in_play(P0).is_empty());
    assert!(g.lasting().is_empty());
}

#[test]
fn town_chooses_cards_and_actions_or_a_buy_and_coins() {
    let mut g = game(&[Card::Town]);
    hand(&mut g, P0, &[Card::Town, Card::Town]);
    deck(&mut g, P0, &[Card::Copper; 5]);
    g.start();
    play(&mut g, Card::Town);
    modes(&mut g, &[0]);
    assert_eq!((g.turn().actions, g.hand(P0).len()), (2, 2));
    play(&mut g, Card::Town);
    modes(&mut g, &[1]);
    assert_eq!((g.turn().buys, g.turn().coins), (2, 2));
}

#[test]
fn barbarian_trashes_top_cards_for_cheaper_cards_sharing_a_type_or_curses() {
    let mut g = setup_with(GameConfig {
        colonies: Some(false),
        shelters: Some(false),
        landscapes: vec![Card::PlateauShepherds],
        ..GameConfig::new(3, vec![Card::Barbarian, Card::Village], 7)
    });
    hand(&mut g, P0, &[Card::Barbarian]);
    deck(&mut g, P1, &[Card::Gold]);
    deck(&mut g, P2, &[Card::Estate]);
    g.start();
    play(&mut g, Card::Barbarian);
    assert_eq!(g.turn().coins, 2);
    assert_eq!(g.pending().unwrap().player, P1);
    let DecisionKind::Pile { piles, .. } = &g.pending().unwrap().kind else {
        panic!()
    };
    let offered: Vec<Card> = piles.iter().map(|&p| g.pile(p).card).collect();
    assert!(offered.contains(&Card::Silver) && offered.contains(&Card::Copper));
    assert!(!offered.contains(&Card::Village));
    choose_pile(&mut g, Card::Silver);
    assert_eq!(discard_kinds(&g, P1), [Card::Silver]);
    assert_eq!(discard_kinds(&g, P2), [Card::Curse]);
    assert_eq!(
        kinds(&g, &g.trash()[g.trash().len() - 2..]),
        [Card::Gold, Card::Estate]
    );
}

#[test]
fn barbarian_gives_a_curse_when_there_is_nothing_to_trash() {
    let mut g = game(&[Card::Barbarian]);
    hand(&mut g, P0, &[Card::Barbarian]);
    deck(&mut g, P1, &[]);
    discard(&mut g, P1, &[]);
    g.start();
    play(&mut g, Card::Barbarian);
    assert_eq!(discard_kinds(&g, P1), [Card::Curse]);
}

#[test]
fn capital_city_discards_two_for_coins_and_pays_two_for_cards() {
    let mut g = game(&[Card::CapitalCity]);
    hand(
        &mut g,
        P0,
        &[Card::CapitalCity, Card::Estate, Card::Estate, Card::Copper],
    );
    deck(&mut g, P0, &[Card::Copper; 6]);
    g.start();
    play(&mut g, Card::CapitalCity);
    assert_eq!(g.turn().actions, 2);
    yes(&mut g);
    choose_kinds(&mut g, &[Card::Estate, Card::Estate]);
    assert_eq!(g.turn().coins, 2);
    yes(&mut g);
    assert_eq!(g.turn().coins, 0);
    assert_eq!(g.hand(P0).len(), 4);
}

#[test]
fn capital_city_discarding_fewer_than_two_gives_no_coins() {
    let mut g = game(&[Card::CapitalCity]);
    hand(&mut g, P0, &[Card::CapitalCity]);
    deck(&mut g, P0, &[Card::Estate]);
    g.start();
    play(&mut g, Card::CapitalCity);
    yes(&mut g);
    assert_eq!(g.turn().coins, 0);
    assert!(g.hand(P0).is_empty());
    // No $2 to pay: the turn moves on.
    assert!(matches!(
        g.pending().unwrap().kind,
        DecisionKind::Turn { .. }
    ));
}

#[test]
fn contract_sets_an_action_aside_to_play_next_turn_and_is_discarded_then() {
    let mut g = game(&[Card::Contract, Card::Caravan]);
    hand(&mut g, P0, &[Card::Contract, Card::Caravan]);
    deck(&mut g, P0, &[Card::Copper; 20]);
    g.start();
    to_buy(&mut g);
    play(&mut g, Card::Contract);
    assert_eq!(g.turn().coins, 2);
    assert_eq!(favors(&g, P0), 2);
    choose_kinds(&mut g, &[Card::Caravan]);
    assert_eq!(kinds(&g, &g.player(P0).set_aside), [Card::Caravan]);
    next_turn(&mut g);
    assert_eq!(in_play_kinds(&g, P0), [Card::Contract]);
    next_turn(&mut g);
    assert_eq!(in_play_kinds(&g, P0), [Card::Contract, Card::Caravan]);
    assert_eq!(g.hand(P0).len(), 6);
    assert_eq!(g.turn().actions, 2);
    next_turn(&mut g);
    // Contract is discarded; the Caravan it played stays for its own Duration.
    assert_eq!(in_play_kinds(&g, P0), [Card::Caravan]);
}

#[test]
fn contract_without_a_set_aside_card_is_discarded_that_turn() {
    let mut g = game(&[Card::Contract]);
    hand(&mut g, P0, &[Card::Contract, Card::Village]);
    g.start();
    to_buy(&mut g);
    play(&mut g, Card::Contract);
    choose(&mut g, &[]);
    next_turn(&mut g);
    assert!(g.in_play(P0).is_empty());
}

#[test]
fn emissary_gives_an_action_and_two_favors_only_when_it_shuffled() {
    let mut g = game(&[Card::Emissary]);
    hand(&mut g, P0, &[Card::Emissary, Card::Emissary]);
    deck(&mut g, P0, &[Card::Copper; 3]);
    discard(&mut g, P0, &[Card::Silver; 4]);
    g.start();
    play(&mut g, Card::Emissary);
    assert_eq!(g.turn().actions, 0);
    assert_eq!(favors(&g, P0), 1);
    let mut g = game(&[Card::Emissary]);
    hand(&mut g, P0, &[Card::Emissary]);
    deck(&mut g, P0, &[Card::Copper; 2]);
    discard(&mut g, P0, &[Card::Silver; 4]);
    g.start();
    play(&mut g, Card::Emissary);
    assert_eq!(g.turn().actions, 1);
    assert_eq!(favors(&g, P0), 3);
    assert_eq!(g.hand(P0).len(), 3);
}

#[test]
fn emissary_with_an_empty_discard_pile_does_not_shuffle() {
    let mut g = game(&[Card::Emissary]);
    hand(&mut g, P0, &[Card::Emissary]);
    deck(&mut g, P0, &[Card::Copper; 2]);
    discard(&mut g, P0, &[]);
    g.start();
    play(&mut g, Card::Emissary);
    assert_eq!(g.turn().actions, 0);
    assert_eq!(favors(&g, P0), 1);
}

#[test]
fn galleria_gives_a_buy_for_each_gain_costing_three_or_four() {
    let mut g = game(&[Card::Galleria, Card::Smithy]);
    hand(&mut g, P0, &[Card::Galleria, Card::Gold, Card::Gold]);
    g.start();
    play(&mut g, Card::Galleria);
    play(&mut g, Card::Gold);
    play(&mut g, Card::Gold);
    assert_eq!(g.turn().coins, 9);
    buy(&mut g, Card::Silver);
    assert_eq!(g.turn().buys, 1);
    buy(&mut g, Card::Smithy);
    assert_eq!(g.turn().buys, 1);
    assert_eq!(g.turn().coins, 2);
}

#[test]
fn galleria_gives_no_buy_for_a_card_costing_a_potion() {
    let mut g = game(&[Card::Galleria, Card::Golem, Card::Market]);
    hand(
        &mut g,
        P0,
        &[Card::Market, Card::Galleria, Card::Potion, Card::Gold],
    );
    deck(&mut g, P0, &[Card::Copper; 5]);
    g.start();
    play(&mut g, Card::Market);
    play(&mut g, Card::Galleria);
    play(&mut g, Card::Potion);
    play(&mut g, Card::Gold);
    buy(&mut g, Card::Golem);
    assert_eq!(g.turn().buys, 1);
}

#[test]
fn guildmaster_gives_a_favor_per_gain_usable_on_the_same_gain() {
    let mut g = game_with(&[Card::Guildmaster], Card::BandOfNomads);
    set_favors(&mut g, P0, 0);
    hand(&mut g, P0, &[Card::Guildmaster, Card::Copper]);
    deck(&mut g, P0, &[Card::Copper; 6]);
    g.start();
    play(&mut g, Card::Guildmaster);
    assert_eq!(g.turn().coins, 3);
    play(&mut g, Card::Copper);
    buy(&mut g, Card::Silver);
    // The Favor from Guildmaster is there before Band of Nomads is offered.
    assert_eq!(favors(&g, P0), 1);
    accept(&mut g, Card::BandOfNomads);
    modes(&mut g, &[2]);
    assert_eq!(favors(&g, P0), 0);
    assert_eq!(g.turn().buys, 1);
}

#[test]
fn highwayman_stops_the_first_treasure_each_other_player_plays_each_turn() {
    let mut g = game(&[Card::Highwayman]);
    hand(&mut g, P0, &[Card::Highwayman]);
    deck(&mut g, P0, &[Card::Copper; 20]);
    hand(&mut g, P1, &[Card::Silver, Card::Copper]);
    deck(&mut g, P1, &[Card::Copper; 20]);
    g.start();
    play(&mut g, Card::Highwayman);
    next_turn(&mut g);
    assert_eq!(g.turn().player, P1);
    play(&mut g, Card::Silver);
    assert_eq!(g.turn().coins, 0);
    play(&mut g, Card::Copper);
    assert_eq!(g.turn().coins, 1);
    next_turn(&mut g);
    // Discarded first, then +3 Cards.
    assert!(g.in_play(P0).is_empty());
    assert_eq!(g.hand(P0).len(), 8);
    assert!(g.lasting().is_empty());
    next_turn(&mut g);
    play(&mut g, Card::Copper);
    assert_eq!(g.turn().coins, 1);
}

/// With Charlatan in the game a Curse is a Treasure, so one played first is the first
/// Treasure.
#[test]
fn highwayman_counts_a_curse_that_charlatan_makes_a_treasure() {
    let mut g = game(&[Card::Highwayman, Card::Charlatan]);
    hand(&mut g, P0, &[Card::Highwayman]);
    deck(&mut g, P0, &[Card::Copper; 10]);
    hand(&mut g, P1, &[Card::Curse, Card::Silver]);
    deck(&mut g, P1, &[Card::Copper; 10]);
    g.start();
    play(&mut g, Card::Highwayman);
    next_turn(&mut g);
    to_buy(&mut g);
    play(&mut g, Card::Curse);
    assert_eq!(g.turn().coins, 0);
    play(&mut g, Card::Silver);
    assert_eq!(g.turn().coins, 2);
}

/// Under Capitalism, Militia is a Treasure during its owner's turns.
#[test]
fn highwayman_counts_an_action_that_capitalism_makes_a_treasure() {
    let mut g = game(&[Card::Highwayman, Card::Militia]);
    g.players[1].projects.push(Card::Capitalism);
    hand(&mut g, P0, &[Card::Highwayman]);
    deck(&mut g, P0, &[Card::Copper; 10]);
    hand(&mut g, P1, &[Card::Militia, Card::Silver]);
    deck(&mut g, P1, &[Card::Copper; 10]);
    g.start();
    play(&mut g, Card::Highwayman);
    next_turn(&mut g);
    play(&mut g, Card::Militia);
    assert_eq!(g.turn().coins, 0);
    to_buy(&mut g);
    play(&mut g, Card::Silver);
    assert_eq!(g.turn().coins, 2);
}

#[test]
fn highwayman_is_not_cumulative_and_moat_blocks_it() {
    let mut g = game(&[Card::Highwayman, Card::Moat]);
    hand(
        &mut g,
        P0,
        &[Card::Highwayman, Card::Highwayman, Card::Village],
    );
    deck(&mut g, P0, &[Card::Copper; 20]);
    hand(&mut g, P1, &[Card::Copper, Card::Copper]);
    deck(&mut g, P1, &[Card::Copper; 20]);
    g.start();
    play(&mut g, Card::Village);
    play(&mut g, Card::Highwayman);
    play(&mut g, Card::Highwayman);
    next_turn(&mut g);
    play(&mut g, Card::Copper);
    play(&mut g, Card::Copper);
    assert_eq!(g.turn().coins, 1);

    let mut g = game(&[Card::Highwayman, Card::Moat]);
    hand(&mut g, P0, &[Card::Highwayman]);
    deck(&mut g, P0, &[Card::Copper; 20]);
    hand(&mut g, P1, &[Card::Moat, Card::Copper]);
    deck(&mut g, P1, &[Card::Copper; 20]);
    g.start();
    play(&mut g, Card::Highwayman);
    accept(&mut g, Card::Moat);
    next_turn(&mut g);
    to_buy(&mut g);
    play(&mut g, Card::Copper);
    assert_eq!(g.turn().coins, 1);
}

/// Highwayman stops what an Attack does, not the Reactions to it being played.
#[test]
fn reactions_answer_an_attack_highwayman_stops() {
    let mut g = game(&[Card::Highwayman, Card::Relic, Card::Moat]);
    hand(&mut g, P0, &[Card::Highwayman]);
    deck(&mut g, P0, &[Card::Copper; 20]);
    hand(&mut g, P1, &[Card::Relic]);
    deck(&mut g, P1, &[Card::Copper; 20]);
    g.start();
    play(&mut g, Card::Highwayman);
    next_turn(&mut g);
    let mut cards = vec![Card::Moat];
    cards.extend([Card::Copper; 4]);
    hand(&mut g, P0, &cards);
    play(&mut g, Card::Relic);
    accept(&mut g, Card::Moat);
    assert_eq!(g.pending().unwrap().player, P1);
    assert_eq!(g.turn().coins, 0);
}

#[test]
fn throne_room_on_highwayman_draws_six_and_discards_it_once() {
    let mut g = game(&[Card::Highwayman, Card::ThroneRoom]);
    hand(&mut g, P0, &[Card::ThroneRoom, Card::Highwayman]);
    deck(&mut g, P0, &[Card::Copper; 20]);
    g.start();
    play(&mut g, Card::ThroneRoom);
    choose_kinds(&mut g, &[Card::Highwayman]);
    next_turn(&mut g);
    next_turn(&mut g);
    while purpose(&g) == Purpose::Trigger {
        accept(&mut g, Card::Highwayman);
    }
    assert_eq!(g.hand(P0).len(), 11);
    assert_eq!(in_play_kinds(&g, P0), [Card::ThroneRoom]);
    next_turn(&mut g);
    assert!(g.in_play(P0).is_empty());
}

#[test]
fn hunter_takes_an_action_a_treasure_and_a_victory_card_and_discards_the_rest() {
    let mut g = game(&[Card::Hunter]);
    hand(&mut g, P0, &[Card::Hunter]);
    deck(
        &mut g,
        P0,
        &[Card::Stronghold, Card::Copper, Card::Silver, Card::Estate],
    );
    g.start();
    play(&mut g, Card::Hunter);
    assert_eq!(g.turn().actions, 1);
    // Stronghold is the only Action; then a Treasure is picked; no Victory card is left.
    choose_kinds(&mut g, &[Card::Silver]);
    let mut h = hand_kinds(&g, P0);
    h.sort();
    let mut want = vec![Card::Stronghold, Card::Silver];
    want.sort();
    assert_eq!(h, want);
    assert_eq!(discard_kinds(&g, P0), [Card::Copper]);
    assert_eq!(deck_kinds(&g, P0), [Card::Estate]);
}

#[test]
fn marquis_draws_a_card_per_card_in_hand_and_discards_down_to_ten() {
    let mut g = game(&[Card::Marquis]);
    hand(
        &mut g,
        P0,
        &[
            Card::Marquis,
            Card::Copper,
            Card::Copper,
            Card::Copper,
            Card::Copper,
            Card::Copper,
            Card::Copper,
        ],
    );
    deck(&mut g, P0, &[Card::Estate; 10]);
    g.start();
    play(&mut g, Card::Marquis);
    assert_eq!(g.turn().buys, 2);
    assert_eq!(g.hand(P0).len(), 12);
    choose_kinds(&mut g, &[Card::Estate, Card::Estate]);
    assert_eq!(g.hand(P0).len(), 10);
}

#[test]
fn marquis_discards_down_to_ten_even_if_the_deck_runs_out() {
    let mut g = game(&[Card::Marquis]);
    let mut h = vec![Card::Marquis];
    h.extend([Card::Copper; 11]);
    hand(&mut g, P0, &h);
    deck(&mut g, P0, &[]);
    g.start();
    play(&mut g, Card::Marquis);
    assert_eq!(g.hand(P0).len(), 10);
}

#[test]
fn modify_trashes_then_gives_a_card_and_action_or_a_better_card() {
    let mut g = game(&[Card::Modify, Card::Laboratory]);
    hand(
        &mut g,
        P0,
        &[Card::Modify, Card::Modify, Card::Silver, Card::Estate],
    );
    deck(&mut g, P0, &[Card::Copper; 5]);
    g.start();
    play(&mut g, Card::Modify);
    choose_kinds(&mut g, &[Card::Silver]);
    modes(&mut g, &[1]);
    choose_pile(&mut g, Card::Laboratory);
    assert_eq!(discard_kinds(&g, P0), [Card::Laboratory]);
    // No Action left for the second Modify.
    assert_eq!(g.turn().actions, 0);
    let mut g = game(&[Card::Modify]);
    hand(&mut g, P0, &[Card::Modify]);
    deck(&mut g, P0, &[Card::Copper; 5]);
    g.start();
    play(&mut g, Card::Modify);
    modes(&mut g, &[0]);
    assert_eq!((g.turn().actions, g.hand(P0).len()), (1, 1));
}

#[test]
fn skirmisher_makes_the_others_discard_to_three_when_an_attack_is_gained() {
    let mut g = game(&[Card::Skirmisher, Card::Militia]);
    hand(&mut g, P0, &[Card::Skirmisher, Card::Gold]);
    deck(&mut g, P0, &[Card::Copper; 5]);
    g.start();
    play(&mut g, Card::Skirmisher);
    assert_eq!((g.turn().actions, g.turn().coins), (1, 1));
    play(&mut g, Card::Gold);
    play(&mut g, Card::Copper);
    buy(&mut g, Card::Militia);
    assert_eq!(g.pending().unwrap().player, P1);
    choose_kinds(&mut g, &[Card::Copper, Card::Copper]);
    assert_eq!(g.hand(P1).len(), 3);
}

#[test]
fn skirmisher_spares_a_player_who_revealed_moat_when_it_was_played() {
    let mut g = game(&[Card::Skirmisher, Card::Moat]);
    hand(&mut g, P0, &[Card::Skirmisher, Card::Gold, Card::Silver]);
    deck(&mut g, P0, &[Card::Copper; 5]);
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
    play(&mut g, Card::Skirmisher);
    accept(&mut g, Card::Moat);
    play(&mut g, Card::Gold);
    play(&mut g, Card::Silver);
    buy(&mut g, Card::Skirmisher);
    assert_eq!(g.hand(P1).len(), 5);
}

#[test]
fn specialist_plays_a_card_and_plays_it_again_or_gains_a_copy() {
    let mut g = game(&[Card::Specialist, Card::Village]);
    hand(&mut g, P0, &[Card::Specialist, Card::Village]);
    deck(&mut g, P0, &[Card::Copper; 5]);
    g.start();
    play(&mut g, Card::Specialist);
    choose_kinds(&mut g, &[Card::Village]);
    modes(&mut g, &[0]);
    assert_eq!(g.turn().actions, 4);
    assert_eq!(g.hand(P0).len(), 2);
    let mut g = game(&[Card::Specialist, Card::Village]);
    hand(&mut g, P0, &[Card::Specialist, Card::Silver]);
    g.start();
    play(&mut g, Card::Specialist);
    choose_kinds(&mut g, &[Card::Silver]);
    modes(&mut g, &[1]);
    assert_eq!(g.turn().coins, 2);
    assert_eq!(discard_kinds(&g, P0), [Card::Silver]);
}

#[test]
fn swap_returns_an_action_and_gains_a_different_one_to_the_hand() {
    let mut g = game(&[Card::Swap, Card::Village, Card::Smithy]);
    hand(&mut g, P0, &[Card::Swap, Card::Village]);
    deck(&mut g, P0, &[Card::Copper; 5]);
    g.start();
    play(&mut g, Card::Swap);
    choose_kinds(&mut g, &[Card::Village]);
    let DecisionKind::Pile { piles, .. } = &g.pending().unwrap().kind else {
        panic!()
    };
    let village = g.supply_pile(Card::Village).unwrap();
    assert!(!piles.contains(&village));
    choose_pile(&mut g, Card::Smithy);
    assert!(hand_kinds(&g, P0).contains(&Card::Smithy));
    assert_eq!(g.pile(village).cards.len(), 11);
}

#[test]
fn town_crier_gains_a_silver_then_may_rotate_the_townsfolk() {
    let mut g = game(&[Card::Townsfolk]);
    hand(&mut g, P0, &[Card::TownCrier]);
    g.start();
    play(&mut g, Card::TownCrier);
    modes(&mut g, &[1]);
    assert_eq!(discard_kinds(&g, P0), [Card::Silver]);
    yes(&mut g);
    assert_eq!(top(&g, Card::Townsfolk), Some(Card::Blacksmith));
}

#[test]
fn blacksmith_draws_up_to_six_or_two_or_one_and_an_action() {
    let mut g = game(&[Card::Townsfolk]);
    hand(&mut g, P0, &[Card::Blacksmith, Card::Copper]);
    deck(&mut g, P0, &[Card::Copper; 10]);
    g.start();
    play(&mut g, Card::Blacksmith);
    modes(&mut g, &[0]);
    assert_eq!(g.hand(P0).len(), 6);
}

#[test]
fn miller_looks_at_four_and_takes_one() {
    let mut g = game(&[Card::Townsfolk]);
    hand(&mut g, P0, &[Card::Miller]);
    deck(
        &mut g,
        P0,
        &[
            Card::Copper,
            Card::Gold,
            Card::Estate,
            Card::Silver,
            Card::Duchy,
        ],
    );
    g.start();
    play(&mut g, Card::Miller);
    assert_eq!(g.turn().actions, 1);
    choose_kinds(&mut g, &[Card::Gold]);
    assert_eq!(hand_kinds(&g, P0), [Card::Gold]);
    assert_eq!(discard_kinds(&g, P0).len(), 3);
    assert_eq!(deck_kinds(&g, P0), [Card::Duchy]);
}

#[test]
fn elder_lets_the_card_it_plays_choose_an_extra_option() {
    let mut g = game(&[Card::Townsfolk, Card::Town]);
    hand(&mut g, P0, &[Card::Elder, Card::Town, Card::Town]);
    deck(&mut g, P0, &[Card::Copper; 5]);
    g.start();
    play(&mut g, Card::Elder);
    assert_eq!(g.turn().coins, 2);
    let town = find(&g, P0, Card::Town);
    choose(&mut g, &[town]);
    modes(&mut g, &[0, 1]);
    assert_eq!(g.turn().actions, 2);
    assert_eq!((g.turn().buys, g.turn().coins), (2, 4));
    assert!(g.lasting().is_empty());
    // A Town played normally later gets one choice only.
    play(&mut g, Card::Town);
    assert_eq!(
        g.respond(Response::Options(vec![Opt::Mode(0), Opt::Mode(1)])),
        Err(ResponseError::WrongCount {
            got: 2,
            min: 1,
            max: 1
        })
    );
}

#[test]
fn elder_on_blacksmith_does_the_choices_in_printed_order() {
    let mut g = game(&[Card::Townsfolk]);
    hand(&mut g, P0, &[Card::Elder, Card::Blacksmith]);
    deck(&mut g, P0, &[Card::Copper; 10]);
    g.start();
    play(&mut g, Card::Elder);
    choose_kinds(&mut g, &[Card::Blacksmith]);
    modes(&mut g, &[2, 0]);
    // Draw to 6 first, then +1 Card.
    assert_eq!(g.hand(P0).len(), 7);
    assert_eq!(g.turn().actions, 1);
}

#[test]
fn elder_on_stronghold_gives_coins_now_and_cards_next_turn() {
    let mut g = game(&[Card::Townsfolk, Card::Forts]);
    hand(&mut g, P0, &[Card::Elder, Card::Stronghold]);
    deck(&mut g, P0, &[Card::Copper; 20]);
    g.start();
    play(&mut g, Card::Elder);
    choose_kinds(&mut g, &[Card::Stronghold]);
    modes(&mut g, &[0, 1]);
    assert_eq!(g.turn().coins, 5);
    next_turn(&mut g);
    assert_eq!(in_play_kinds(&g, P0), [Card::Stronghold]);
    next_turn(&mut g);
    assert_eq!(g.hand(P0).len(), 8);
}

#[test]
fn acolyte_trashes_for_a_gold_and_itself_for_the_top_augur() {
    let mut g = game(&[Card::Augurs]);
    hand(&mut g, P0, &[Card::Acolyte, Card::Estate]);
    g.start();
    play(&mut g, Card::Acolyte);
    choose_kinds(&mut g, &[Card::Estate]);
    assert_eq!(discard_kinds(&g, P0), [Card::Gold]);
    yes(&mut g);
    assert_eq!(discard_kinds(&g, P0), [Card::Gold, Card::HerbGatherer]);
    assert!(kinds(&g, g.trash()).contains(&Card::Acolyte));
}

#[test]
fn acolyte_abilities_are_optional() {
    let mut g = game(&[Card::Augurs]);
    hand(&mut g, P0, &[Card::Acolyte, Card::Estate]);
    g.start();
    play(&mut g, Card::Acolyte);
    choose(&mut g, &[]);
    no(&mut g);
    assert!(discard_kinds(&g, P0).is_empty());
    assert_eq!(in_play_kinds(&g, P0), [Card::Acolyte]);
}

#[test]
fn sorceress_puts_the_top_card_into_hand_and_curses_on_a_right_guess() {
    let mut g = game(&[Card::Augurs]);
    hand(&mut g, P0, &[Card::Sorceress, Card::Sorceress]);
    deck(&mut g, P0, &[Card::Gold, Card::Silver]);
    g.start();
    play(&mut g, Card::Sorceress);
    name(&mut g, Card::Gold);
    assert_eq!(discard_kinds(&g, P1), [Card::Curse]);
    assert!(hand_kinds(&g, P0).contains(&Card::Gold));
    play(&mut g, Card::Sorceress);
    name(&mut g, Card::Gold);
    assert!(hand_kinds(&g, P0).contains(&Card::Silver));
    assert_eq!(discard_kinds(&g, P1), [Card::Curse]);
}

#[test]
fn sibyl_puts_one_card_on_top_and_another_on_the_bottom() {
    let mut g = game(&[Card::Augurs]);
    hand(&mut g, P0, &[Card::Sibyl]);
    deck(
        &mut g,
        P0,
        &[
            Card::Copper,
            Card::Silver,
            Card::Gold,
            Card::Estate,
            Card::Duchy,
        ],
    );
    g.start();
    play(&mut g, Card::Sibyl);
    assert_eq!(g.turn().actions, 1);
    choose_kinds(&mut g, &[Card::Gold]);
    choose_kinds(&mut g, &[Card::Estate]);
    assert_eq!(deck_kinds(&g, P0), [Card::Gold, Card::Duchy, Card::Estate]);
}

#[test]
fn herb_gatherer_puts_the_deck_in_the_discard_pile_and_may_play_a_treasure_from_it() {
    let mut g = game(&[Card::Augurs]);
    hand(&mut g, P0, &[Card::HerbGatherer]);
    deck(&mut g, P0, &[Card::Gold, Card::Estate]);
    discard(&mut g, P0, &[Card::Silver]);
    g.start();
    play(&mut g, Card::HerbGatherer);
    assert!(g.player(P0).deck.is_empty());
    assert_eq!(g.turn().buys, 2);
    choose_kinds(&mut g, &[Card::Gold]);
    assert_eq!(g.turn().coins, 3);
    no(&mut g);
    assert_eq!(top(&g, Card::Augurs), Some(Card::HerbGatherer));
}

#[test]
fn battle_plan_reveals_an_attack_for_a_card_and_rotates_any_pile() {
    let mut g = game(&[Card::Clashes, Card::Townsfolk, Card::Militia]);
    hand(&mut g, P0, &[Card::BattlePlan, Card::Militia]);
    deck(&mut g, P0, &[Card::Copper; 5]);
    g.start();
    play(&mut g, Card::BattlePlan);
    choose_kinds(&mut g, &[Card::Militia]);
    assert_eq!(g.hand(P0).len(), 3);
    choose_pile(&mut g, Card::Townsfolk);
    assert_eq!(top(&g, Card::Townsfolk), Some(Card::Blacksmith));
}

#[test]
fn archer_makes_players_with_five_cards_reveal_all_but_one_and_discard_one() {
    let mut g = game(&[Card::Clashes]);
    hand(&mut g, P0, &[Card::Archer]);
    hand(
        &mut g,
        P1,
        &[
            Card::Gold,
            Card::Copper,
            Card::Copper,
            Card::Silver,
            Card::Estate,
        ],
    );
    g.start();
    play(&mut g, Card::Archer);
    assert_eq!(g.turn().coins, 2);
    assert_eq!(g.pending().unwrap().player, P1);
    choose_kinds(&mut g, &[Card::Gold]);
    assert_eq!(g.pending().unwrap().player, P0);
    assert_eq!(candidates(&g).len(), 4);
    choose_kinds(&mut g, &[Card::Silver]);
    assert_eq!(discard_kinds(&g, P1), [Card::Silver]);
}

#[test]
fn archer_leaves_a_player_with_four_cards_alone() {
    let mut g = game(&[Card::Clashes]);
    hand(&mut g, P0, &[Card::Archer]);
    hand(
        &mut g,
        P1,
        &[Card::Gold, Card::Copper, Card::Copper, Card::Silver],
    );
    g.start();
    play(&mut g, Card::Archer);
    assert_eq!(g.hand(P1).len(), 4);
}

#[test]
fn warlord_stops_others_playing_an_action_they_have_two_copies_of_in_play() {
    let mut g = game(&[Card::Clashes, Card::Village, Card::ThroneRoom]);
    hand(&mut g, P0, &[Card::Warlord]);
    deck(&mut g, P0, &[Card::Copper; 20]);
    hand(
        &mut g,
        P1,
        &[Card::Village, Card::Village, Card::Village, Card::Copper],
    );
    deck(&mut g, P1, &[Card::Copper; 20]);
    g.start();
    play(&mut g, Card::Warlord);
    assert_eq!(g.turn().actions, 1);
    next_turn(&mut g);
    play(&mut g, Card::Village);
    play(&mut g, Card::Village);
    assert!(!can_play(&g, Card::Village));
    to_buy(&mut g);
    assert!(can_play(&g, Card::Copper));
    next_turn(&mut g);
    // The Attack ended; Warlord draws 2.
    assert_eq!(g.hand(P0).len(), 7);
}

#[test]
fn warlord_lets_throne_room_play_a_card_with_one_copy_in_play() {
    let mut g = game(&[Card::Clashes, Card::Village, Card::ThroneRoom]);
    hand(&mut g, P0, &[Card::Warlord]);
    deck(&mut g, P0, &[Card::Copper; 20]);
    hand(
        &mut g,
        P1,
        &[
            Card::Village,
            Card::ThroneRoom,
            Card::Village,
            Card::Village,
        ],
    );
    deck(&mut g, P1, &[Card::Copper; 20]);
    g.start();
    play(&mut g, Card::Warlord);
    next_turn(&mut g);
    play(&mut g, Card::Village);
    play(&mut g, Card::ThroneRoom);
    choose_kinds(&mut g, &[Card::Village]);
    assert_eq!(g.turn().actions, 5);
    assert!(!can_play(&g, Card::Village));
}

#[test]
fn territory_counts_differently_named_victory_cards_and_gains_golds_for_empty_piles() {
    let mut g = game(&[Card::Clashes, Card::Smithy, Card::Village]);
    for card in [Card::Smithy, Card::Village] {
        let pile = g.supply_pile(card).unwrap();
        for c in g.pile(pile).cards.clone() {
            g.move_card(c, Zone::Trash, crate::game::Pos::Top);
        }
    }
    let clashes = g.pile_of(Card::Clashes).unwrap();
    let territory = g.pile(clashes).cards[0];
    g.move_card(territory, Zone::Pile(clashes), crate::game::Pos::Top);
    hand(&mut g, P0, &[Card::Gold, Card::Gold]);
    g.start();
    play(&mut g, Card::Gold);
    play(&mut g, Card::Gold);
    respond(&mut g, Response::Move(Move::Buy(clashes)));
    assert_eq!(g.count_owned(P0, Card::Gold), 4);
    // Estates (3) and Territory itself.
    assert_eq!(crate::cards::vp(&g, P0, Card::Territory), Some(2));
}

#[test]
fn tent_may_rotate_the_forts_and_go_onto_the_deck() {
    let mut g = game(&[Card::Forts]);
    hand(&mut g, P0, &[Card::Tent]);
    deck(&mut g, P0, &[Card::Copper; 10]);
    g.start();
    play(&mut g, Card::Tent);
    yes(&mut g);
    assert_eq!(top(&g, Card::Forts), Some(Card::Garrison));
    assert_eq!(g.turn().coins, 2);
    to_buy(&mut g);
    end_phase(&mut g);
    accept(&mut g, Card::Tent);
    assert!(hand_kinds(&g, P0).contains(&Card::Tent));
}

#[test]
fn garrison_draws_a_card_per_gain_next_turn() {
    let mut g = game(&[Card::Forts]);
    hand(&mut g, P0, &[Card::Garrison, Card::Silver]);
    deck(&mut g, P0, &[Card::Copper; 20]);
    g.start();
    play(&mut g, Card::Garrison);
    play(&mut g, Card::Silver);
    buy(&mut g, Card::Copper);
    assert_eq!(g.turn().player, P1);
    assert_eq!(in_play_kinds(&g, P0), [Card::Garrison]);
    next_turn(&mut g);
    assert_eq!(g.hand(P0).len(), 6);
    next_turn(&mut g);
    assert!(g.in_play(P0).is_empty());
}

#[test]
fn garrison_with_no_gains_is_discarded_that_turn() {
    let mut g = game(&[Card::Forts]);
    hand(&mut g, P0, &[Card::Garrison]);
    deck(&mut g, P0, &[Card::Copper; 20]);
    g.start();
    play(&mut g, Card::Garrison);
    next_turn(&mut g);
    assert!(g.in_play(P0).is_empty());
    assert!(g.lasting().is_empty());
}

#[test]
fn throne_roomed_garrison_gets_two_tokens_per_gain() {
    let mut g = game(&[Card::Forts, Card::ThroneRoom]);
    hand(&mut g, P0, &[Card::ThroneRoom, Card::Garrison]);
    deck(&mut g, P0, &[Card::Copper; 20]);
    g.start();
    play(&mut g, Card::ThroneRoom);
    choose_kinds(&mut g, &[Card::Garrison]);
    buy(&mut g, Card::Silver);
    assert_eq!(in_play_kinds(&g, P0), [Card::ThroneRoom, Card::Garrison]);
    next_turn(&mut g);
    while purpose(&g) == Purpose::Trigger {
        accept(&mut g, Card::Garrison);
    }
    assert_eq!(g.hand(P0).len(), 7);
}

#[test]
fn hill_fort_gains_a_card_into_hand_or_gives_a_card_and_an_action() {
    let mut g = game(&[Card::Forts, Card::Smithy]);
    hand(&mut g, P0, &[Card::HillFort, Card::HillFort]);
    deck(&mut g, P0, &[Card::Copper; 5]);
    g.start();
    play(&mut g, Card::HillFort);
    choose_pile(&mut g, Card::Smithy);
    modes(&mut g, &[0]);
    assert!(hand_kinds(&g, P0).contains(&Card::Smithy));
    assert_eq!(g.turn().actions, 0);
    let mut g = game(&[Card::Forts, Card::Smithy]);
    hand(&mut g, P0, &[Card::HillFort]);
    deck(&mut g, P0, &[Card::Copper; 5]);
    g.start();
    play(&mut g, Card::HillFort);
    choose_pile(&mut g, Card::Silver);
    modes(&mut g, &[1]);
    assert_eq!((g.turn().actions, g.hand(P0).len()), (1, 1));
    assert_eq!(discard_kinds(&g, P0), [Card::Silver]);
}

#[test]
fn stronghold_gives_three_coins_or_three_cards_next_turn_and_two_vp() {
    let mut g = game(&[Card::Forts]);
    hand(&mut g, P0, &[Card::Stronghold]);
    deck(&mut g, P0, &[Card::Copper; 20]);
    g.start();
    play(&mut g, Card::Stronghold);
    modes(&mut g, &[0]);
    assert_eq!(g.turn().coins, 3);
    next_turn(&mut g);
    assert!(g.in_play(P0).is_empty());
    assert_eq!(Card::Stronghold.def().vp, 2);
    assert_eq!(Card::DistantShore.def().vp, 2);
}

#[test]
fn old_map_discards_draws_and_may_rotate() {
    let mut g = game(&[Card::Odysseys]);
    hand(&mut g, P0, &[Card::OldMap, Card::Estate]);
    deck(&mut g, P0, &[Card::Copper, Card::Silver]);
    g.start();
    play(&mut g, Card::OldMap);
    choose_kinds(&mut g, &[Card::Estate]);
    assert_eq!(hand_kinds(&g, P0), [Card::Copper, Card::Silver]);
    yes(&mut g);
    assert_eq!(top(&g, Card::Odysseys), Some(Card::Voyage));
}

#[test]
fn voyage_takes_an_extra_turn_with_three_plays_from_hand() {
    let mut g = game(&[Card::Odysseys, Card::Village]);
    hand(&mut g, P0, &[Card::Voyage]);
    deck(
        &mut g,
        P0,
        &[
            Card::Village,
            Card::Village,
            Card::Village,
            Card::Village,
            Card::Copper,
        ],
    );
    g.start();
    play(&mut g, Card::Voyage);
    assert_eq!(g.turn().actions, 1);
    next_turn(&mut g);
    assert_eq!(g.turn().player, P0);
    assert_eq!(g.turn().extra, Some(Card::Voyage));
    play(&mut g, Card::Village);
    play(&mut g, Card::Village);
    play(&mut g, Card::Village);
    assert!(!can_play(&g, Card::Village));
    to_buy(&mut g);
    assert!(!can_play(&g, Card::Copper));
    next_turn(&mut g);
    assert_eq!(g.turn().player, P1);
    // Voyage was discarded in the Voyage turn's Clean-up.
    assert!(g.in_play(P0).is_empty());
}

#[test]
fn voyage_on_an_extra_turn_gives_no_third_turn() {
    let mut g = game(&[Card::Odysseys]);
    hand(&mut g, P0, &[Card::Voyage]);
    deck(
        &mut g,
        P0,
        &[
            Card::Voyage,
            Card::Copper,
            Card::Copper,
            Card::Copper,
            Card::Copper,
            Card::Copper,
        ],
    );
    g.start();
    play(&mut g, Card::Voyage);
    next_turn(&mut g);
    play(&mut g, Card::Voyage);
    next_turn(&mut g);
    assert_eq!(g.turn().player, P1);
}

#[test]
fn sunken_treasure_gains_an_action_not_in_play() {
    let mut g = game(&[Card::Odysseys, Card::Village, Card::Smithy]);
    hand(&mut g, P0, &[Card::Village, Card::SunkenTreasure]);
    g.start();
    play(&mut g, Card::Village);
    to_buy(&mut g);
    play(&mut g, Card::SunkenTreasure);
    let DecisionKind::Pile { piles, .. } = &g.pending().unwrap().kind else {
        panic!()
    };
    assert!(!piles.contains(&g.supply_pile(Card::Village).unwrap()));
    choose_pile(&mut g, Card::Smithy);
    assert_eq!(discard_kinds(&g, P0), [Card::Smithy]);
}

#[test]
fn distant_shore_draws_and_gains_an_estate() {
    let mut g = game(&[Card::Odysseys]);
    hand(&mut g, P0, &[Card::DistantShore]);
    deck(&mut g, P0, &[Card::Copper; 3]);
    g.start();
    play(&mut g, Card::DistantShore);
    assert_eq!((g.turn().actions, g.hand(P0).len()), (1, 2));
    assert_eq!(discard_kinds(&g, P0), [Card::Estate]);
}

#[test]
fn student_trashes_a_treasure_for_a_favor_and_goes_onto_the_deck() {
    let mut g = game(&[Card::Wizards]);
    hand(&mut g, P0, &[Card::Student, Card::Copper]);
    deck(&mut g, P0, &[Card::Estate]);
    g.start();
    play(&mut g, Card::Student);
    no(&mut g);
    assert_eq!(kinds(&g, g.trash()).last(), Some(&Card::Copper));
    assert_eq!(favors(&g, P0), 2);
    assert_eq!(deck_kinds(&g, P0), [Card::Student, Card::Estate]);
}

#[test]
fn student_trashing_a_non_treasure_stays_in_play() {
    let mut g = game(&[Card::Wizards]);
    hand(&mut g, P0, &[Card::Student, Card::Estate]);
    g.start();
    play(&mut g, Card::Student);
    yes(&mut g);
    assert_eq!(top(&g, Card::Wizards), Some(Card::Conjurer));
    assert_eq!(favors(&g, P0), 1);
    assert_eq!(in_play_kinds(&g, P0), [Card::Student]);
}

#[test]
fn conjurer_gains_a_card_and_returns_to_hand_next_turn() {
    let mut g = game(&[Card::Wizards]);
    hand(&mut g, P0, &[Card::Conjurer]);
    deck(&mut g, P0, &[Card::Copper; 20]);
    g.start();
    play(&mut g, Card::Conjurer);
    choose_pile(&mut g, Card::Silver);
    next_turn(&mut g);
    assert_eq!(in_play_kinds(&g, P0), [Card::Conjurer]);
    next_turn(&mut g);
    assert!(hand_kinds(&g, P0).contains(&Card::Conjurer));
    assert!(g.in_play(P0).is_empty());
}

#[test]
fn sorcerer_curses_each_player_who_names_wrong_and_leaves_the_card() {
    let mut g = game(&[Card::Wizards]);
    hand(&mut g, P0, &[Card::Sorcerer]);
    deck(&mut g, P0, &[Card::Copper; 5]);
    deck(&mut g, P1, &[Card::Gold]);
    g.start();
    play(&mut g, Card::Sorcerer);
    assert_eq!(g.pending().unwrap().player, P1);
    name(&mut g, Card::Silver);
    assert_eq!(discard_kinds(&g, P1), [Card::Curse]);
    assert_eq!(deck_kinds(&g, P1), [Card::Gold]);
}

#[test]
fn lich_draws_six_and_skips_the_next_turn() {
    let mut g = game(&[Card::Wizards]);
    hand(&mut g, P0, &[Card::Lich]);
    deck(&mut g, P0, &[Card::Copper; 20]);
    g.start();
    play(&mut g, Card::Lich);
    assert_eq!((g.turn().actions, g.hand(P0).len()), (2, 6));
    next_turn(&mut g);
    assert_eq!(g.turn().player, P1);
    next_turn(&mut g);
    assert_eq!(g.turn().player, P1);
    assert_eq!(g.player(P0).turns_taken, 1);
    assert_eq!(g.player(P0).turns_skipped, 1);
    next_turn(&mut g);
    assert_eq!(g.turn().player, P0);
}

#[test]
fn a_trashed_lich_is_discarded_and_gains_a_cheaper_card_from_the_trash() {
    let mut g = game(&[Card::Wizards, Card::Chapel]);
    hand(&mut g, P0, &[Card::Chapel, Card::Lich, Card::Sycophant]);
    g.start();
    g.create(Card::Silver, Zone::Trash);
    g.create(Card::Gold, Zone::Trash);
    play(&mut g, Card::Chapel);
    choose_kinds(&mut g, &[Card::Lich, Card::Sycophant]);
    // Sycophant's Favors and Lich resolve in either order.
    while purpose(&g) == Purpose::Trigger {
        accept(&mut g, Card::Lich);
    }
    let offered = kinds(&g, &candidates(&g));
    assert!(offered.contains(&Card::Sycophant) && offered.contains(&Card::Silver));
    assert!(!offered.contains(&Card::Gold));
    choose_kinds(&mut g, &[Card::Sycophant]);
    resolve_rest(&mut g);
    let d = discard_kinds(&g, P0);
    assert!(d.len() == 2 && d.contains(&Card::Lich) && d.contains(&Card::Sycophant));
}

fn resolve_rest(g: &mut Game) {
    while g.pending().is_some_and(|d| d.purpose == Purpose::Trigger) {
        respond(g, Response::option(Opt::Trigger(0)));
    }
}

#[test]
fn lich_skips_a_voyage_turn() {
    let mut g = game(&[Card::Wizards, Card::Odysseys]);
    hand(&mut g, P0, &[Card::Lich, Card::Voyage]);
    deck(&mut g, P0, &[Card::Copper; 20]);
    g.start();
    play(&mut g, Card::Voyage);
    play(&mut g, Card::Lich);
    next_turn(&mut g);
    assert_eq!(g.turn().player, P1);
    next_turn(&mut g);
    assert_eq!(g.turn().player, P0);
    assert_eq!(g.turn().extra, None);
}

/// Random games with Allies kingdoms, each of the 23 Allies in turn, Throne Room and Moat,
/// and every deck seeded with one of each Allies card, so that everything gets played.
#[test]
fn random_games_with_every_allies_card_keep_invariants() {
    let allies: Vec<Card> = Card::in_set(Set::Allies)
        .filter(|c| c.kind() == Kind::Ally)
        .collect();
    let piles: Vec<Card> = Card::in_set(Set::Allies)
        .filter(|c| c.def().kingdom)
        .collect();
    let cards: Vec<Card> = Card::in_set(Set::Allies)
        .filter(|c| c.kind() == Kind::Card)
        .collect();
    assert_eq!((allies.len(), piles.len(), cards.len()), (23, 31, 49));
    let mut played = std::collections::BTreeSet::new();
    for seed in 0..fuzz_seeds(46) {
        let mut rng = ChaCha8Rng::seed_from_u64(seed);
        let mut kingdom = piles.clone();
        kingdom.shuffle(&mut rng);
        kingdom.truncate(8);
        kingdom.extend([Card::ThroneRoom, Card::Moat]);
        let config = GameConfig {
            landscapes: vec![allies[seed as usize % allies.len()]],
            turn_limit: Some(60),
            ..GameConfig::new(rng.random_range(2..=4), kingdom, seed)
        };
        let mut g = setup_with(config);
        for p in 0..g.num_players() {
            for &card in cards.iter().chain(&[Card::Gold, Card::Gold]) {
                g.create(card, Zone::Deck(PlayerId(p as u8)));
            }
            let mut deck = std::mem::take(&mut g.players[p].deck);
            g.shuffle(&mut deck);
            g.players[p].deck = deck;
            g.players[p].favors += 6;
        }
        g.start();
        let g = drive_randomly(g, seed);
        assert!(g.outcome().is_some());
        for e in g.log() {
            if let Event::Played { card, .. } = e {
                played.insert(*card);
            }
        }
    }
    let unplayed: Vec<Card> = cards
        .iter()
        .copied()
        .filter(|c| c.is(Types::ACTION | Types::TREASURE) && !played.contains(c))
        .collect();
    if fuzz_seeds(46) >= 46 {
        assert!(unplayed.is_empty(), "never played: {unplayed:?}");
    }
}

// ---- Allies ----

#[test]
fn architects_guild_gains_cheaper_non_victory_cards_and_chains_off_itself() {
    let mut g = game_with(&[Card::Underling], Card::ArchitectsGuild);
    set_favors(&mut g, P0, 4);
    hand(&mut g, P0, &[Card::Gold, Card::Gold]);
    g.start();
    play(&mut g, Card::Gold);
    play(&mut g, Card::Gold);
    buy(&mut g, Card::Gold);
    accept(&mut g, Card::ArchitectsGuild);
    assert_eq!(favors(&g, P0), 2);
    let DecisionKind::Pile { piles, .. } = &g.pending().unwrap().kind else {
        panic!()
    };
    let names: Vec<Card> = piles.iter().map(|&p| g.pile(p).card).collect();
    assert!(names.contains(&Card::Silver) && !names.contains(&Card::Duchy));
    assert!(!names.contains(&Card::Gold));
    choose_pile(&mut g, Card::Silver);
    accept(&mut g, Card::ArchitectsGuild);
    choose_pile(&mut g, Card::Copper);
    assert_eq!(favors(&g, P0), 0);
    let mut d = discard_kinds(&g, P0);
    d.retain(|c| matches!(c, Card::Gold | Card::Silver | Card::Copper));
    assert!(g.log().contains(&Event::Gained {
        player: P0,
        card: Card::Copper,
        to: Dest::Discard
    }));
}

#[test]
fn band_of_nomads_spends_a_favor_on_a_gain_costing_three_or_more() {
    let mut g = game_with(&[Card::Underling], Card::BandOfNomads);
    hand(&mut g, P0, &[Card::Gold, Card::Market]);
    deck(&mut g, P0, &[Card::Copper; 10]);
    g.start();
    play(&mut g, Card::Market);
    play(&mut g, Card::Gold);
    buy(&mut g, Card::Copper);
    // Copper costs $0: nothing offered.
    assert_eq!(purpose(&g), Purpose::Turn);
    buy(&mut g, Card::Silver);
    accept(&mut g, Card::BandOfNomads);
    modes(&mut g, &[2]);
    assert_eq!(favors(&g, P0), 0);
    assert_eq!(g.turn().buys, 1);
}

#[test]
fn cave_dwellers_discards_and_draws_as_often_as_desired() {
    let mut g = game_with(&[Card::Underling], Card::CaveDwellers);
    set_favors(&mut g, P0, 2);
    hand(&mut g, P0, &[Card::Estate, Card::Curse, Card::Copper]);
    deck(&mut g, P0, &[Card::Gold, Card::Silver, Card::Copper]);
    g.start();
    accept(&mut g, Card::CaveDwellers);
    choose_kinds(&mut g, &[Card::Estate]);
    assert!(hand_kinds(&g, P0).contains(&Card::Gold));
    yes(&mut g);
    choose_kinds(&mut g, &[Card::Curse]);
    assert!(hand_kinds(&g, P0).contains(&Card::Silver));
    assert_eq!(favors(&g, P0), 0);
    assert_eq!(purpose(&g), Purpose::Turn);
}

#[test]
fn circle_of_witches_curses_the_others_after_a_liaison_for_three_favors() {
    let mut g = game_with(&[Card::Underling, Card::Moat], Card::CircleOfWitches);
    set_favors(&mut g, P0, 2);
    hand(&mut g, P0, &[Card::Underling]);
    deck(&mut g, P0, &[Card::Copper; 5]);
    hand(&mut g, P1, &[Card::Moat]);
    g.start();
    play(&mut g, Card::Underling);
    // The Favor Underling just gave counts.
    accept(&mut g, Card::CircleOfWitches);
    assert_eq!(favors(&g, P0), 0);
    assert_eq!(discard_kinds(&g, P1), [Card::Curse]);
}

#[test]
fn city_state_plays_an_action_gained_on_your_turn_for_two_favors() {
    let mut g = game_with(&[Card::Underling, Card::Village], Card::CityState);
    set_favors(&mut g, P0, 2);
    hand(&mut g, P0, &[Card::Gold]);
    deck(&mut g, P0, &[Card::Copper; 5]);
    g.start();
    play(&mut g, Card::Gold);
    buy(&mut g, Card::Village);
    accept(&mut g, Card::CityState);
    assert!(g.log().contains(&Event::Played {
        player: P0,
        card: Card::Village
    }));
    assert_eq!(favors(&g, P0), 0);
}

#[test]
fn city_state_does_not_work_on_another_players_turn() {
    let mut g = game_with(
        &[Card::Underling, Card::Village, Card::Swindler],
        Card::CityState,
    );
    set_favors(&mut g, P1, 2);
    hand(&mut g, P0, &[Card::Swindler]);
    deck(&mut g, P1, &[Card::Silver]);
    g.start();
    play(&mut g, Card::Swindler);
    choose_pile(&mut g, Card::Village);
    assert_eq!(discard_kinds(&g, P1), [Card::Village]);
    assert!(!offered(&g, Card::CityState));
}

#[test]
fn coastal_haven_keeps_cards_in_hand_for_favors() {
    let mut g = game_with(&[Card::Underling], Card::CoastalHaven);
    set_favors(&mut g, P0, 2);
    hand(&mut g, P0, &[Card::Gold, Card::Silver, Card::Estate]);
    deck(&mut g, P0, &[Card::Copper; 10]);
    g.start();
    to_buy(&mut g);
    end_phase(&mut g);
    accept(&mut g, Card::CoastalHaven);
    choose_kinds(&mut g, &[Card::Gold, Card::Silver]);
    assert_eq!(favors(&g, P0), 0);
    assert_eq!(g.hand(P0).len(), 7);
    assert!(hand_kinds(&g, P0).contains(&Card::Gold));
    assert_eq!(discard_kinds(&g, P0), [Card::Estate]);
    assert!(g.lasting().is_empty());
}

#[test]
fn crafters_guild_gains_a_card_onto_the_deck_for_two_favors() {
    let mut g = game_with(&[Card::Underling, Card::Smithy], Card::CraftersGuild);
    set_favors(&mut g, P0, 2);
    hand(&mut g, P0, &[Card::Copper]);
    deck(&mut g, P0, &[Card::Estate]);
    g.start();
    accept(&mut g, Card::CraftersGuild);
    choose_pile(&mut g, Card::Smithy);
    assert_eq!(deck_kinds(&g, P0), [Card::Smithy, Card::Estate]);
    assert_eq!(favors(&g, P0), 0);
}

#[test]
fn desert_guides_discards_the_hand_and_draws_five() {
    let mut g = game_with(&[Card::Underling], Card::DesertGuides);
    hand(&mut g, P0, &[Card::Estate, Card::Estate]);
    deck(&mut g, P0, &[Card::Gold; 5]);
    g.start();
    accept(&mut g, Card::DesertGuides);
    assert_eq!(hand_kinds(&g, P0), [Card::Gold; 5]);
    assert_eq!(discard_kinds(&g, P0), [Card::Estate, Card::Estate]);
    assert_eq!(purpose(&g), Purpose::Turn);
}

#[test]
fn family_of_inventors_puts_favors_on_non_victory_piles_to_lower_costs() {
    let mut g = game_with(
        &[Card::Underling, Card::Smithy, Card::Gardens],
        Card::FamilyOfInventors,
    );
    set_favors(&mut g, P0, 2);
    hand(&mut g, P0, &[Card::Copper]);
    g.start();
    accept(&mut g, Card::FamilyOfInventors);
    let DecisionKind::Pile { piles, .. } = &g.pending().unwrap().kind else {
        panic!()
    };
    let names: Vec<Card> = piles.iter().map(|&p| g.pile(p).card).collect();
    assert!(names.contains(&Card::Smithy) && names.contains(&Card::Curse));
    assert!(!names.contains(&Card::Province) && !names.contains(&Card::Gardens));
    choose_pile(&mut g, Card::Smithy);
    assert_eq!(g.cost(Card::Smithy), Cost::coins(3));
    assert_eq!(g.view(P1).cost(Card::Smithy), Cost::coins(3));
    assert_eq!(favors(&g, P0), 1);
}

#[test]
fn family_of_inventors_tokens_add_up_and_stop_at_zero() {
    let mut g = game_with(&[Card::Underling], Card::FamilyOfInventors);
    let silver = g.supply_pile(Card::Silver).unwrap();
    g.ext.allies.inventors.push((silver, 4));
    assert_eq!(g.cost(Card::Silver), Cost::ZERO);
    let id = g.pile_top(silver).unwrap();
    assert_eq!(g.cost_of(id), Cost::ZERO);
}

#[test]
fn fellowship_of_scribes_draws_after_an_action_with_four_or_fewer_cards_in_hand() {
    let mut g = game_with(&[Card::Underling, Card::Village], Card::FellowshipOfScribes);
    hand(&mut g, P0, &[Card::Village, Card::Copper]);
    deck(&mut g, P0, &[Card::Copper; 5]);
    g.start();
    play(&mut g, Card::Village);
    accept(&mut g, Card::FellowshipOfScribes);
    assert_eq!(g.hand(P0).len(), 3);
    assert_eq!(favors(&g, P0), 0);
}

#[test]
fn forest_dwellers_looks_at_three_discards_any_and_reorders_the_rest() {
    let mut g = game_with(&[Card::Underling], Card::ForestDwellers);
    hand(&mut g, P0, &[Card::Copper]);
    deck(
        &mut g,
        P0,
        &[Card::Estate, Card::Silver, Card::Gold, Card::Curse],
    );
    g.start();
    accept(&mut g, Card::ForestDwellers);
    choose_kinds(&mut g, &[Card::Estate]);
    choose_kinds(&mut g, &[Card::Gold, Card::Silver]);
    assert_eq!(deck_kinds(&g, P0), [Card::Gold, Card::Silver, Card::Curse]);
    assert_eq!(discard_kinds(&g, P0), [Card::Estate]);
}

#[test]
fn gang_of_pickpockets_discards_down_to_four_unless_a_favor_is_spent() {
    let mut g = game_with(&[Card::Underling], Card::GangOfPickpockets);
    hand(&mut g, P0, &[Card::Copper; 5]);
    deck(&mut g, P0, &[Card::Copper; 10]);
    hand(
        &mut g,
        P1,
        &[
            Card::Estate,
            Card::Copper,
            Card::Copper,
            Card::Copper,
            Card::Copper,
        ],
    );
    deck(&mut g, P1, &[Card::Copper; 10]);
    g.start();
    yes(&mut g);
    assert_eq!((g.hand(P0).len(), favors(&g, P0)), (5, 0));
    next_turn(&mut g);
    no(&mut g);
    choose_kinds(&mut g, &[Card::Estate]);
    assert_eq!(g.hand(P1).len(), 4);
    next_turn(&mut g);
    // No Favors left: P0 just discards (all Coppers, so no choice).
    assert_eq!(g.hand(P0).len(), 4);
}

#[test]
fn island_folk_takes_an_extra_turn_for_five_favors_but_not_a_third_in_a_row() {
    let mut g = game_with(&[Card::Underling], Card::IslandFolk);
    set_favors(&mut g, P0, 10);
    hand(&mut g, P0, &[Card::Copper]);
    deck(&mut g, P0, &[Card::Copper; 20]);
    g.start();
    end_phase(&mut g);
    accept(&mut g, Card::IslandFolk);
    assert_eq!(g.turn().player, P0);
    assert_eq!(g.turn().extra, Some(Card::IslandFolk));
    end_phase(&mut g);
    assert_eq!(g.turn().player, P1);
    assert_eq!(favors(&g, P0), 5);
}

#[test]
fn island_folk_and_voyage_in_one_turn_take_only_the_island_folk_turn() {
    let mut g = game_with(&[Card::Underling, Card::Odysseys], Card::IslandFolk);
    set_favors(&mut g, P0, 5);
    hand(&mut g, P0, &[Card::Voyage]);
    deck(&mut g, P0, &[Card::Village; 20]);
    g.start();
    play(&mut g, Card::Voyage);
    to_buy(&mut g);
    end_phase(&mut g);
    accept(&mut g, Card::IslandFolk);
    assert_eq!(g.turn().extra, Some(Card::IslandFolk));
    // Not limited to 3 plays from the hand.
    for _ in 0..4 {
        play(&mut g, Card::Village);
    }
    next_turn(&mut g);
    assert_eq!(g.turn().player, P1);
    assert!(g.in_play(P0).is_empty());
}

#[test]
fn league_of_bankers_gives_a_coin_per_four_favors_at_the_start_of_the_buy_phase() {
    let mut g = game_with(&[Card::Underling], Card::LeagueOfBankers);
    set_favors(&mut g, P0, 9);
    hand(&mut g, P0, &[Card::Copper]);
    g.start();
    assert_eq!(g.turn().coins, 2);
    assert_eq!(favors(&g, P0), 9);
}

#[test]
fn league_of_shopkeepers_rewards_liaisons_by_favors_held() {
    let mut g = game_with(&[Card::Underling], Card::LeagueOfShopkeepers);
    set_favors(&mut g, P0, 9);
    hand(&mut g, P0, &[Card::Underling]);
    deck(&mut g, P0, &[Card::Copper; 5]);
    g.start();
    play(&mut g, Card::Underling);
    assert_eq!(g.turn().coins, 1);
    assert_eq!((g.turn().actions, g.turn().buys), (2, 2));
    assert_eq!(favors(&g, P0), 10);
}

#[test]
fn league_of_shopkeepers_with_throne_room_checks_after_each_play() {
    let mut g = game_with(
        &[Card::Underling, Card::ThroneRoom],
        Card::LeagueOfShopkeepers,
    );
    set_favors(&mut g, P0, 3);
    hand(&mut g, P0, &[Card::ThroneRoom, Card::Underling]);
    deck(&mut g, P0, &[Card::Copper; 5]);
    g.start();
    play(&mut g, Card::ThroneRoom);
    choose_kinds(&mut g, &[Card::Underling]);
    assert_eq!(g.turn().coins, 1);
    assert_eq!(favors(&g, P0), 5);
}

#[test]
fn market_towns_plays_actions_from_hand_at_the_start_of_the_buy_phase() {
    let mut g = game_with(&[Card::Underling, Card::Village], Card::MarketTowns);
    set_favors(&mut g, P0, 2);
    hand(&mut g, P0, &[Card::Village, Card::Village, Card::Copper]);
    deck(&mut g, P0, &[Card::Copper; 5]);
    g.start();
    end_phase(&mut g);
    accept(&mut g, Card::MarketTowns);
    let village = find(&g, P0, Card::Village);
    choose(&mut g, &[village]);
    let village = find(&g, P0, Card::Village);
    choose(&mut g, &[village]);
    assert_eq!(favors(&g, P0), 0);
    assert_eq!(in_play_kinds(&g, P0), [Card::Village, Card::Village]);
    assert_eq!(g.turn().phase, Phase::Buy);
}

#[test]
fn mountain_folk_draws_three_for_five_favors_only() {
    let mut g = game_with(&[Card::Underling], Card::MountainFolk);
    set_favors(&mut g, P0, 4);
    hand(&mut g, P0, &[Card::Village]);
    deck(&mut g, P0, &[Card::Copper; 5]);
    g.start();
    assert_eq!(purpose(&g), Purpose::Turn);
    let mut g = game_with(&[Card::Underling], Card::MountainFolk);
    set_favors(&mut g, P0, 5);
    hand(&mut g, P0, &[Card::Village]);
    deck(&mut g, P0, &[Card::Copper; 5]);
    g.start();
    accept(&mut g, Card::MountainFolk);
    assert_eq!(g.hand(P0).len(), 4);
}

#[test]
fn order_of_astrologers_puts_cards_on_top_of_the_shuffled_cards() {
    let mut g = game_with(&[Card::Underling, Card::Smithy], Card::OrderOfAstrologers);
    set_favors(&mut g, P0, 2);
    hand(&mut g, P0, &[Card::Smithy]);
    deck(&mut g, P0, &[Card::Copper]);
    discard(
        &mut g,
        P0,
        &[
            Card::Estate,
            Card::Curse,
            Card::Gold,
            Card::Silver,
            Card::Duchy,
        ],
    );
    g.start();
    play(&mut g, Card::Smithy);
    accept(&mut g, Card::OrderOfAstrologers);
    assert_eq!(candidates(&g).len(), 5);
    choose_kinds(&mut g, &[Card::Gold, Card::Silver]);
    choose_kinds(&mut g, &[Card::Gold, Card::Silver]);
    let mut h = hand_kinds(&g, P0);
    h.sort();
    let mut want = vec![Card::Copper, Card::Gold, Card::Silver];
    want.sort();
    assert_eq!(h, want);
    assert_eq!(favors(&g, P0), 0);
}

#[test]
fn order_of_masons_leaves_cards_out_of_the_shuffle_and_shuffles_only_once() {
    let mut g = game_with(&[Card::Underling, Card::Smithy], Card::OrderOfMasons);
    hand(&mut g, P0, &[Card::Smithy]);
    deck(&mut g, P0, &[]);
    discard(
        &mut g,
        P0,
        &[Card::Province, Card::Province, Card::Curse, Card::Curse],
    );
    g.start();
    play(&mut g, Card::Smithy);
    accept(&mut g, Card::OrderOfMasons);
    choose_kinds(&mut g, &[Card::Province, Card::Province]);
    assert_eq!(hand_kinds(&g, P0), [Card::Curse, Card::Curse]);
    assert_eq!(discard_kinds(&g, P0), [Card::Province, Card::Province]);
    assert_eq!(favors(&g, P0), 0);
}

#[test]
fn peaceful_cult_trashes_a_card_per_favor_spent() {
    let mut g = game_with(&[Card::Underling], Card::PeacefulCult);
    set_favors(&mut g, P0, 2);
    hand(
        &mut g,
        P0,
        &[Card::Estate, Card::Estate, Card::Estate, Card::Copper],
    );
    g.start();
    accept(&mut g, Card::PeacefulCult);
    assert_eq!(
        g.respond(Response::Cards(g.hand(P0)[..3].to_vec())),
        Err(ResponseError::WrongCount {
            got: 3,
            min: 0,
            max: 2
        })
    );
    choose_kinds(&mut g, &[Card::Estate, Card::Estate]);
    assert_eq!(favors(&g, P0), 0);
    assert_eq!(g.hand(P0).len(), 2);
}

#[test]
fn plateau_shepherds_pairs_favors_with_cards_costing_two() {
    let mut g = game_with(&[Card::Underling], Card::PlateauShepherds);
    // Three Estates and seven Coppers.
    assert_eq!(g.score(P0), 3 + 2);
    set_favors(&mut g, P0, 5);
    assert_eq!(g.score(P0), 3 + 6);
}

/// A Plateau Shepherds game that ends after P0's first turn.
fn one_turn_game(kingdom: &[Card]) -> Game {
    setup_with(GameConfig {
        colonies: Some(false),
        shelters: Some(false),
        landscapes: vec![Card::PlateauShepherds],
        turn_limit: Some(1),
        ..GameConfig::new(2, kingdom.to_vec(), 7)
    })
}

fn score_after_the_turn(g: &mut Game, p: PlayerId) -> i32 {
    while g.outcome().is_none() {
        end_phase(g);
    }
    g.outcome().unwrap().scores[p.index()]
}

#[test]
fn plateau_shepherds_scores_without_canal() {
    let mut g = one_turn_game(&[Card::Underling]);
    g.players[0].projects.push(Card::Canal);
    set_favors(&mut g, P0, 3);
    g.start();
    let estate = find(&g, P0, Card::Estate);
    assert_eq!(g.cost_of(estate), Cost::coins(1));
    // "During your turns" is over: three Estates and three pairs with them.
    assert_eq!(score_after_the_turn(&mut g, P0), 3 + 6);
}

#[test]
fn plateau_shepherds_scores_fisherman_at_its_printed_cost() {
    let mut g = one_turn_game(&[Card::Underling, Card::Fisherman]);
    set_favors(&mut g, P0, 2);
    hand(&mut g, P0, &[]);
    deck(&mut g, P0, &[Card::Fisherman; 5]);
    discard(&mut g, P0, &[]);
    g.start();
    assert_eq!(g.cost(Card::Fisherman), Cost::coins(2));
    assert_eq!(score_after_the_turn(&mut g, P0), 0);
    assert!(g.player(P0).discard.is_empty());
}

#[test]
fn trappers_lodge_puts_a_gained_card_onto_the_deck_for_a_favor() {
    let mut g = game_with(&[Card::Underling], Card::TrappersLodge);
    hand(&mut g, P0, &[Card::Gold]);
    deck(&mut g, P0, &[]);
    discard(&mut g, P0, &[]);
    g.start();
    play(&mut g, Card::Gold);
    buy(&mut g, Card::Silver);
    accept(&mut g, Card::TrappersLodge);
    assert!(
        hand_kinds(&g, P0).contains(&Card::Silver) || deck_kinds(&g, P0).contains(&Card::Silver)
    );
    assert!(!g.log().iter().any(|e| matches!(
        e,
        Event::Discarded {
            card: Card::Silver,
            ..
        }
    )));
    assert_eq!(favors(&g, P0), 0);
}

#[test]
fn woodworkers_guild_trashes_an_action_to_gain_an_action() {
    let mut g = game_with(&[Card::Underling, Card::Laboratory], Card::WoodworkersGuild);
    hand(&mut g, P0, &[Card::Village, Card::Copper]);
    g.start();
    end_phase(&mut g);
    accept(&mut g, Card::WoodworkersGuild);
    choose_pile(&mut g, Card::Laboratory);
    assert_eq!(discard_kinds(&g, P0), [Card::Laboratory]);
    assert!(kinds(&g, g.trash()).contains(&Card::Village));
}

// ---- Interactions ----

#[test]
fn guildmaster_favor_from_a_city_state_gain_can_pay_for_the_play() {
    let mut g = game_with(&[Card::Guildmaster, Card::Village], Card::CityState);
    set_favors(&mut g, P0, 1);
    hand(&mut g, P0, &[Card::Guildmaster]);
    deck(&mut g, P0, &[Card::Copper; 5]);
    g.start();
    play(&mut g, Card::Guildmaster);
    buy(&mut g, Card::Village);
    accept(&mut g, Card::CityState);
    assert!(g.log().contains(&Event::Played {
        player: P0,
        card: Card::Village
    }));
}

#[test]
fn highwayman_attack_lasts_past_a_turn_lich_skips() {
    let mut g = game(&[Card::Highwayman, Card::Wizards]);
    hand(&mut g, P0, &[Card::Highwayman, Card::Lich]);
    deck(&mut g, P0, &[Card::Copper; 20]);
    hand(&mut g, P1, &[Card::Copper; 5]);
    deck(&mut g, P1, &[Card::Copper; 20]);
    g.start();
    play(&mut g, Card::Lich);
    play(&mut g, Card::Highwayman);
    next_turn(&mut g);
    play(&mut g, Card::Copper);
    assert_eq!(g.turn().coins, 0);
    next_turn(&mut g);
    assert_eq!(g.turn().player, P1);
    play(&mut g, Card::Copper);
    assert_eq!(g.turn().coins, 0);
}

#[test]
fn games_with_allies_resume_identically_after_serialization() {
    let mut g = game_with(&[Card::Townsfolk, Card::Underling], Card::FamilyOfInventors);
    hand(&mut g, P0, &[Card::Elder, Card::Town]);
    deck(&mut g, P0, &[Card::Copper; 5]);
    g.start();
    play(&mut g, Card::Elder);
    let town = find(&g, P0, Card::Town);
    choose(&mut g, &[town]);
    let mut restored: Game = serde_json::from_str(&serde_json::to_string(&g).unwrap()).unwrap();
    modes(&mut g, &[0, 1]);
    modes(&mut restored, &[0, 1]);
    assert_eq!(
        serde_json::to_string(&g).unwrap(),
        serde_json::to_string(&restored).unwrap()
    );
}

#[test]
fn royal_galley_replaying_a_throne_room_on_a_duration_leaves_the_throne_room_in_play() {
    let mut g = game(&[Card::RoyalGalley, Card::ThroneRoom, Card::Caravan]);
    hand(
        &mut g,
        P0,
        &[Card::RoyalGalley, Card::ThroneRoom, Card::Village],
    );
    deck(&mut g, P0, &[Card::Copper; 30]);
    g.start();
    play(&mut g, Card::RoyalGalley);
    choose_kinds(&mut g, &[Card::ThroneRoom]);
    choose_kinds(&mut g, &[Card::Village]);
    next_turn(&mut g);
    assert_eq!(in_play_kinds(&g, P0), [Card::RoyalGalley, Card::ThroneRoom]);
    // P0's next hand gets a Caravan for the Throne Room to play.
    let caravan = g.create(Card::Caravan, Zone::Hand(P0));
    next_turn(&mut g);
    choose(&mut g, &[caravan]);
    next_turn(&mut g);
    assert_eq!(in_play_kinds(&g, P0), [Card::ThroneRoom, Card::Caravan]);
}

#[test]
fn a_skirmisher_gained_and_played_with_city_state_attacks_for_its_own_gain() {
    let mut g = game_with(&[Card::Skirmisher], Card::CityState);
    set_favors(&mut g, P0, 2);
    hand(&mut g, P0, &[Card::Gold, Card::Silver]);
    deck(&mut g, P0, &[Card::Copper; 5]);
    hand(&mut g, P1, &[Card::Copper; 5]);
    g.start();
    play(&mut g, Card::Gold);
    play(&mut g, Card::Silver);
    buy(&mut g, Card::Skirmisher);
    accept(&mut g, Card::CityState);
    assert_eq!(g.hand(P1).len(), 3);
}

#[test]
fn a_voyage_whose_turn_lich_skips_is_discarded_in_the_next_clean_up() {
    let mut g = game(&[Card::Wizards, Card::Odysseys]);
    hand(&mut g, P0, &[Card::Lich, Card::Voyage]);
    deck(&mut g, P0, &[Card::Copper; 20]);
    g.start();
    play(&mut g, Card::Voyage);
    play(&mut g, Card::Lich);
    next_turn(&mut g);
    assert_eq!(g.turn().player, P1);
    assert!(in_play_kinds(&g, P0).contains(&Card::Voyage));
    next_turn(&mut g);
    assert!(!in_play_kinds(&g, P0).contains(&Card::Voyage));
}

/// A game in which P0 has played Highwayman and Enlightenment is in effect; P1 is in
/// their Action phase with a Silver (an Action too, under Enlightenment).
fn highwayman_under_enlightenment() -> Game {
    let mut g = setup_with(GameConfig {
        colonies: Some(false),
        shelters: Some(false),
        landscapes: vec![Card::PlateauShepherds, Card::Enlightenment],
        ..GameConfig::new(2, vec![Card::Highwayman], 7)
    });
    g.prophecy = Some((Card::Enlightenment, 0));
    hand(&mut g, P0, &[Card::Highwayman]);
    deck(&mut g, P0, &[Card::Copper; 20]);
    hand(&mut g, P1, &[Card::Silver, Card::Copper]);
    deck(&mut g, P1, &[Card::Estate; 20]);
    g.start();
    play(&mut g, Card::Highwayman);
    next_turn(&mut g);
    assert_eq!(g.turn().phase, Phase::Action);
    play(&mut g, Card::Silver);
    g
}

#[test]
fn highwayman_or_enlightenment_applies_as_the_player_orders_them() {
    let mut g = highwayman_under_enlightenment();
    accept(&mut g, Card::Highwayman);
    // Enlightenment resolved last: +1 Card +1 Action instead of $2.
    assert_eq!((g.turn().actions, g.turn().coins), (1, 0));
    assert_eq!(g.hand(P1).len(), 2);

    let mut g = highwayman_under_enlightenment();
    accept(&mut g, Card::Enlightenment);
    // Highwayman resolved last: the Silver does nothing.
    assert_eq!((g.turn().actions, g.turn().coins), (0, 0));
    assert_eq!(g.hand(P1).len(), 1);
}

/// A game in which P0 has played Highwayman and Enchantress and P1 plays a Crown, their
/// first Treasure and first Action.
fn crown_under_highwayman_and_enchantress() -> Game {
    let mut g = setup(&[
        Card::Highwayman,
        Card::Enchantress,
        Card::Crown,
        Card::Village,
    ]);
    hand(
        &mut g,
        P0,
        &[Card::Village, Card::Highwayman, Card::Enchantress],
    );
    deck(&mut g, P0, &[Card::Copper; 10]);
    hand(&mut g, P1, &[Card::Crown, Card::Copper, Card::Copper]);
    deck(&mut g, P1, &[Card::Estate; 10]);
    g.start();
    play(&mut g, Card::Village);
    play(&mut g, Card::Highwayman);
    play(&mut g, Card::Enchantress);
    next_turn(&mut g);
    play(&mut g, Card::Crown);
    g
}

#[test]
fn highwayman_or_enchantress_applies_as_the_player_orders_them() {
    let mut g = crown_under_highwayman_and_enchantress();
    let d = g.pending().unwrap();
    assert_eq!((d.player, d.purpose), (P1, Purpose::Trigger));
    accept(&mut g, Card::Highwayman);
    // Enchantress resolved last: +1 Card +1 Action.
    assert_eq!(g.turn().actions, 1);
    assert_eq!(g.hand(P1).len(), 3);

    let mut g = crown_under_highwayman_and_enchantress();
    accept(&mut g, Card::Enchantress);
    // Highwayman resolved last: the Crown does nothing.
    assert_eq!(g.turn().actions, 0);
    assert_eq!(hand_kinds(&g, P1), [Card::Copper, Card::Copper]);
    assert!(matches!(
        g.pending().unwrap().kind,
        DecisionKind::Turn { .. }
    ));
}

/// A game with Way of the Sheep in which P0 has played Highwayman; P1 is in their Action
/// phase with a Crown and a Village.
fn crown_under_highwayman_with_a_way() -> Game {
    let mut g = setup_with(GameConfig {
        colonies: Some(false),
        shelters: Some(false),
        landscapes: vec![Card::WayOfTheSheep],
        ..GameConfig::new(2, vec![Card::Highwayman, Card::Crown], 7)
    });
    hand(&mut g, P0, &[Card::Highwayman]);
    deck(&mut g, P0, &[Card::Copper; 10]);
    hand(&mut g, P1, &[Card::Crown, Card::Village, Card::Copper]);
    deck(&mut g, P1, &[Card::Copper; 10]);
    g.start();
    play(&mut g, Card::Highwayman);
    respond(&mut g, Response::option(Opt::No));
    next_turn(&mut g);
    g
}

#[test]
fn an_action_treasure_highwayman_stops_can_still_be_played_as_a_way() {
    let mut g = crown_under_highwayman_with_a_way();
    play(&mut g, Card::Crown);
    respond(&mut g, Response::option(Opt::Card(Card::WayOfTheSheep)));
    assert_eq!(g.turn().coins, 2);

    let mut g = crown_under_highwayman_with_a_way();
    play(&mut g, Card::Crown);
    respond(&mut g, Response::option(Opt::No));
    // Played as itself it does nothing: no Action to play twice.
    assert!(matches!(
        g.pending().unwrap().kind,
        DecisionKind::Turn { .. }
    ));
    assert_eq!(in_play_kinds(&g, P1), [Card::Crown]);
}

#[test]
fn a_liaison_joining_mid_game_brings_an_ally_and_its_favors() {
    let mut g = setup_with(GameConfig::new(2, vec![Card::Smithy], 1));
    assert!(g.landscapes().is_empty());
    add_kingdom_piles(&mut g, &[Card::Wizards]);
    assert_eq!(g.landscapes().len(), 1);
    assert_eq!(g.landscapes()[0].kind(), Kind::Ally);
    assert_eq!((favors(&g, P0), favors(&g, P1)), (1, 1));
    add_kingdom_piles(&mut g, &[Card::Importer, Card::Emissary]);
    assert_eq!(g.landscapes().len(), 1);
    assert_eq!(favors(&g, P0), 5);
}
