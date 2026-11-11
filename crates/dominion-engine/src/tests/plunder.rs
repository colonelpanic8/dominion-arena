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

/// Ends the Action phase if it is still on.
fn to_buy(g: &mut Game) {
    if phase(g) == Some(Phase::Action) {
        end_phase(g);
    }
}

fn phase(g: &Game) -> Option<Phase> {
    match g.pending().map(|d| &d.kind) {
        Some(DecisionKind::Turn { phase, .. }) => Some(*phase),
        _ => None,
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

fn aside_kinds(g: &Game, p: PlayerId) -> Vec<Card> {
    kinds(g, &g.player(p).set_aside)
}

fn trash_kinds(g: &Game) -> Vec<Card> {
    kinds(g, g.trash())
}

fn mode(g: &mut Game, i: u8) {
    respond(g, Response::option(Opt::Mode(i)));
}

fn pick(g: &mut Game, card: Card) {
    respond(g, Response::option(Opt::Card(card)));
}

fn buy_event(g: &mut Game, card: Card) {
    respond(g, Response::Move(Move::BuyLandscape(card)));
}

fn gain_count(g: &Game, p: PlayerId, card: Card) -> usize {
    g.log()
        .iter()
        .filter(
            |e| matches!(e, Event::Gained { player, card: c, .. } if *player == p && *c == card),
        )
        .count()
}

fn loot_gained(g: &Game, p: PlayerId) -> usize {
    g.log()
        .iter()
        .filter(|e| matches!(e, Event::Gained { player, card, .. } if *player == p && card.is(Types::LOOT)))
        .count()
}

/// Gives the turn's player `coins` more $ and `buys` Buys in all, then asks again.
fn money(g: &mut Game, coins: u32, buys: u32) {
    g.turn.coins += coins;
    g.turn.buys = buys;
    if matches!(
        g.pending().map(|d| &d.kind),
        Some(DecisionKind::Turn { .. })
    ) {
        g.pending = None;
        g.run(crate::frame::Input::Start);
    }
}

/// Puts Trait `t` on `card`'s pile (and no other).
fn set_trait(g: &mut Game, t: Card, card: Card) {
    for pile in g.piles.iter_mut() {
        pile.traits.retain(|&x| x != t);
    }
    if !g.landscapes.contains(&t) {
        g.landscapes.push(t);
    }
    let pile = g.pile_of(card).expect("pile exists");
    g.piles[pile.index()].traits.push(t);
}

/// Replaces the top of the Loot pile with a new `card`.
fn loot_top(g: &mut Game, card: Card) -> CardId {
    let pile = g.pile_of(Card::Loot).expect("a Loot pile");
    g.create(card, Zone::Pile(pile))
}

/// An unstarted game with `landscapes` too.
fn setup_l(kingdom: &[Card], landscapes: &[Card]) -> Game {
    setup_with(GameConfig {
        colonies: Some(false),
        shelters: Some(false),
        landscapes: landscapes.to_vec(),
        ..GameConfig::new(2, kingdom.to_vec(), 7)
    })
}

fn play_games(cards: &[Card], landscapes: Vec<Card>, players: u8, seed: u64, extra: &[Card]) {
    let mut config = GameConfig::new(players, cards.to_vec(), seed);
    config.landscapes = landscapes;
    config.turn_limit = Some(60);
    let mut g = setup_with(config);
    for p in 0..players {
        let p = PlayerId(p);
        for &c in cards.iter().chain(extra) {
            if c.kind() == Kind::Card {
                g.create(c, Zone::Deck(p));
            }
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
}

/// Random games over Plunder with Throne Room and Moat, each player starting with a copy
/// of every Kingdom card and some Loot, with Events and Traits in play.
#[test]
fn random_plunder_games_keep_invariants() {
    let pool: Vec<Card> = Card::in_set(Set::Plunder)
        .filter(|c| c.def().kingdom)
        .collect();
    let loot = Card::Loot.pile_members();
    let landscapes: Vec<Card> = Card::in_set(Set::Plunder)
        .filter(|c| matches!(c.kind(), Kind::Event | Kind::Trait))
        .collect();
    for seed in 0..150 {
        let mut rng = ChaCha8Rng::seed_from_u64(seed);
        let mut cards = pool.clone();
        cards.shuffle(&mut rng);
        cards.truncate(8);
        cards.extend([Card::ThroneRoom, Card::Moat]);
        let mut ls = landscapes.clone();
        ls.shuffle(&mut rng);
        ls.truncate(4);
        let mut extra = loot.clone();
        extra.shuffle(&mut rng);
        extra.truncate(3);
        extra.extend([Card::Gold, Card::Gold, Card::Gold]);
        let players = rng.random_range(2..=4);
        play_games(&cards, ls, players, seed, &extra);
    }
}

#[test]
fn a_game_with_a_loot_gainer_has_a_face_down_loot_pile_and_others_do_not() {
    let g = setup(&[Card::SackOfLoot]);
    let pile = g.pile_of(Card::Loot).expect("a Loot pile");
    assert!(!g.pile(pile).in_supply);
    let cards = kinds(&g, &g.pile(pile).cards);
    assert_eq!(cards.len(), 30);
    for loot in Card::Loot.pile_members() {
        assert_eq!(cards.iter().filter(|&&c| c == loot).count(), 2, "{loot}");
    }
    let top = g.pile_top(pile).unwrap();
    assert_eq!(g.view(P0).card(top), None);
    assert!(setup(&[Card::Cage]).pile_of(Card::Loot).is_none());
    assert!(
        setup_l(&[Card::Village], &[Card::Looting])
            .pile_of(Card::Loot)
            .is_some()
    );
}

#[test]
fn loot_cannot_be_bought() {
    let mut g = setup(&[Card::SackOfLoot]);
    hand(&mut g, P0, &[]);
    g.start();
    money(&mut g, 10, 1);
    to_buy(&mut g);
    let loot = g.pile_of(Card::Loot).unwrap();
    let DecisionKind::Turn { moves, .. } = &g.pending().unwrap().kind else {
        panic!()
    };
    assert!(!moves.contains(&Move::Buy(loot)));
}

#[test]
fn cage_holds_cards_until_a_victory_card_is_gained_then_returns_them_after_drawing() {
    let mut g = setup(&[Card::Cage]);
    hand(
        &mut g,
        P0,
        &[
            Card::Cage,
            Card::Estate,
            Card::Estate,
            Card::Copper,
            Card::Copper,
        ],
    );
    deck(&mut g, P0, &[Card::Silver; 30]);
    g.start();
    to_buy(&mut g);
    play(&mut g, Card::Cage);
    assert_eq!(purpose(&g), Purpose::SetAside);
    choose_kinds(
        &mut g,
        &[Card::Estate, Card::Estate, Card::Copper, Card::Copper],
    );
    let held = g.player(P0).set_aside[0];
    assert_eq!(g.view(P1).card(held), None);
    next_turn(&mut g);
    next_turn(&mut g);
    assert_eq!(in_play_kinds(&g, P0), [Card::Cage]);
    assert_eq!(aside_kinds(&g, P0).len(), 4);
    money(&mut g, 2, 1);
    to_buy(&mut g);
    buy(&mut g, Card::Estate);
    // The Cage is trashed, and the cards come to the new hand after drawing it.
    assert!(trash_kinds(&g).contains(&Card::Cage));
    assert_eq!(g.hand(P0).len(), 9);
    assert!(g.player(P0).set_aside.is_empty());
    assert!(g.lasting().is_empty());
}

#[test]
fn a_cage_with_nothing_set_aside_still_waits_in_play() {
    let mut g = setup(&[Card::Cage]);
    hand(&mut g, P0, &[Card::Cage, Card::Copper]);
    deck(&mut g, P0, &[Card::Copper; 30]);
    g.start();
    to_buy(&mut g);
    play(&mut g, Card::Cage);
    choose(&mut g, &[]);
    next_turn(&mut g);
    next_turn(&mut g);
    next_turn(&mut g);
    assert_eq!(in_play_kinds(&g, P0), [Card::Cage]);
}

#[test]
fn a_cage_played_because_of_a_victory_gain_waits_for_the_next_one() {
    // Gaining a Province gains a Fawning Cage, which Mining Road plays.
    let mut g = setup(&[Card::Cage, Card::MiningRoad]);
    set_trait(&mut g, Card::Fawning, Card::Cage);
    hand(&mut g, P0, &[Card::MiningRoad, Card::Estate]);
    deck(&mut g, P0, &[Card::Copper; 10]);
    g.start();
    play(&mut g, Card::MiningRoad);
    money(&mut g, 8, 2);
    to_buy(&mut g);
    buy(&mut g, Card::Province);
    resolve_triggers(&mut g);
    assert_eq!(purpose(&g), Purpose::SetAside);
    choose(&mut g, &[]);
    assert_eq!(in_play_kinds(&g, P0), [Card::MiningRoad, Card::Cage]);
    assert!(!trash_kinds(&g).contains(&Card::Cage));
    buy(&mut g, Card::Estate);
    assert!(trash_kinds(&g).contains(&Card::Cage));
}

#[test]
fn grotto_sets_cards_aside_then_discards_and_draws_that_many() {
    let mut g = setup(&[Card::Grotto]);
    hand(
        &mut g,
        P0,
        &[
            Card::Grotto,
            Card::Estate,
            Card::Estate,
            Card::Curse,
            Card::Silver,
        ],
    );
    deck(
        &mut g,
        P0,
        &[Card::Copper; 5]
            .iter()
            .chain(&[Card::Gold; 10])
            .copied()
            .collect::<Vec<_>>(),
    );
    g.start();
    play(&mut g, Card::Grotto);
    assert_eq!(g.turn().actions, 1);
    choose_kinds(&mut g, &[Card::Estate, Card::Estate, Card::Curse]);
    next_turn(&mut g);
    assert_eq!(in_play_kinds(&g, P0), [Card::Grotto]);
    next_turn(&mut g);
    assert_eq!(g.hand(P0).len(), 8);
    assert_eq!(
        hand_kinds(&g, P0)
            .iter()
            .filter(|&&c| c == Card::Gold)
            .count(),
        3
    );
    assert!(discard_kinds(&g, P0).contains(&Card::Curse));
    next_turn(&mut g);
    next_turn(&mut g);
    assert!(!in_play_kinds(&g, P1).contains(&Card::Grotto));
}

#[test]
fn grotto_with_nothing_set_aside_is_discarded_that_turn() {
    let mut g = setup(&[Card::Grotto]);
    hand(&mut g, P0, &[Card::Grotto, Card::Copper]);
    g.start();
    play(&mut g, Card::Grotto);
    choose(&mut g, &[]);
    next_turn(&mut g);
    assert!(g.in_play(P0).is_empty());
}

#[test]
fn jewelled_egg_gives_a_loot_to_the_player_who_trashes_it() {
    let mut g = setup(&[Card::JewelledEgg, Card::Chapel, Card::Bandit]);
    hand(&mut g, P0, &[Card::Bandit]);
    deck(&mut g, P1, &[Card::JewelledEgg, Card::Estate, Card::Copper]);
    g.start();
    play(&mut g, Card::Bandit);
    assert_eq!(trash_kinds(&g).last(), Some(&Card::JewelledEgg));
    assert_eq!(loot_gained(&g, P1), 1);
    assert_eq!(loot_gained(&g, P0), 0);

    let mut g = setup(&[Card::JewelledEgg, Card::Chapel]);
    hand(&mut g, P0, &[Card::Chapel, Card::JewelledEgg, Card::Copper]);
    g.start();
    play(&mut g, Card::Chapel);
    choose_kinds(&mut g, &[Card::JewelledEgg, Card::Copper]);
    assert_eq!(loot_gained(&g, P0), 1);
}

#[test]
fn jewelled_egg_is_a_copper_with_a_buy() {
    let mut g = setup(&[Card::JewelledEgg]);
    hand(&mut g, P0, &[Card::JewelledEgg]);
    g.start();
    to_buy(&mut g);
    play(&mut g, Card::JewelledEgg);
    assert_eq!((g.turn().coins, g.turn().buys), (1, 2));
}

/// Leaves one card in `card`'s pile.
fn last_one(g: &mut Game, card: Card) {
    let pile = g.pile_of(card).unwrap();
    let all = g.pile(pile).cards.clone();
    for &c in &all[1..] {
        g.move_card(c, Zone::Trash, crate::game::Pos::Top);
    }
    g.trash.clear();
    for &c in &all[1..] {
        g.zones[c.index()] = Zone::Trash;
        g.trash.push(c);
    }
}

#[test]
fn search_is_trashed_for_a_loot_when_a_supply_pile_empties() {
    let mut g = setup(&[Card::Search, Card::Village]);
    hand(&mut g, P0, &[Card::Search]);
    deck(&mut g, P0, &[Card::Copper; 20]);
    last_one(&mut g, Card::Village);
    g.start();
    play(&mut g, Card::Search);
    assert_eq!(g.turn().coins, 2);
    next_turn(&mut g);
    assert_eq!(in_play_kinds(&g, P0), [Card::Search]);
    money(&mut g, 3, 1);
    to_buy(&mut g);
    buy(&mut g, Card::Village);
    // P1 emptied the Village pile: P0's Search is trashed and P0 gains a Loot.
    assert!(trash_kinds(&g).contains(&Card::Search));
    assert_eq!(loot_gained(&g, P0), 1);
    assert!(g.lasting().is_empty());
}

#[test]
fn search_ignores_a_pile_already_empty_when_it_was_played() {
    let mut g = setup(&[Card::Search, Card::Village, Card::Smithy]);
    hand(&mut g, P0, &[Card::Search]);
    deck(&mut g, P0, &[Card::Copper; 20]);
    let village = g.pile_of(Card::Village).unwrap();
    for c in g.pile(village).cards.clone() {
        g.move_card(c, Zone::Trash, crate::game::Pos::Top);
    }
    last_one(&mut g, Card::Smithy);
    g.start();
    play(&mut g, Card::Search);
    next_turn(&mut g);
    assert_eq!(in_play_kinds(&g, P0), [Card::Search]);
    money(&mut g, 4, 1);
    to_buy(&mut g);
    buy(&mut g, Card::Smithy);
    assert_eq!(loot_gained(&g, P0), 1);
}

#[test]
fn a_throned_search_is_trashed_once_gains_two_loot_and_throne_room_goes_that_turn() {
    let mut g = setup(&[Card::Search, Card::ThroneRoom, Card::Village]);
    hand(&mut g, P0, &[Card::ThroneRoom, Card::Search]);
    deck(&mut g, P0, &[Card::Copper; 20]);
    last_one(&mut g, Card::Village);
    g.start();
    play(&mut g, Card::ThroneRoom);
    choose_kinds(&mut g, &[Card::Search]);
    assert_eq!(g.turn().coins, 4);
    to_buy(&mut g);
    buy(&mut g, Card::Village);
    resolve_triggers(&mut g);
    assert_eq!(loot_gained(&g, P0), 2);
    assert_eq!(
        trash_kinds(&g)
            .iter()
            .filter(|&&c| c == Card::Search)
            .count(),
        1
    );
    assert!(discard_kinds(&g, P0).contains(&Card::ThroneRoom));
    assert!(g.in_play(P0).is_empty());
}

#[test]
fn search_counts_the_last_province_and_not_the_loot_pile() {
    let mut g = setup(&[Card::Search, Card::SackOfLoot]);
    hand(&mut g, P0, &[Card::Search, Card::SackOfLoot]);
    deck(&mut g, P0, &[Card::Copper; 20]);
    let loot = g.pile_of(Card::Loot).unwrap();
    for c in g.pile(loot).cards.clone()[1..].to_vec() {
        g.move_card(c, Zone::Trash, crate::game::Pos::Top);
    }
    last_one(&mut g, Card::Province);
    g.start();
    play(&mut g, Card::Search);
    to_buy(&mut g);
    play(&mut g, Card::SackOfLoot);
    // The last Loot: a non-Supply pile empties, which Search ignores.
    assert_eq!(loot_gained(&g, P0), 1);
    assert_eq!(in_play_kinds(&g, P0), [Card::Search, Card::SackOfLoot]);
    let loot = g.pile_of(Card::Loot).unwrap();
    g.create(Card::Hammer, Zone::Pile(loot));
    money(&mut g, 8, 1);
    buy(&mut g, Card::Province);
    // The game ends, but the Search gains its Loot first.
    assert!(g.outcome().is_some());
    assert_eq!(loot_gained(&g, P0), 2);
    assert!(trash_kinds(&g).contains(&Card::Search));
}

#[test]
fn shaman_may_trash_and_everyone_gains_from_the_trash_at_the_start_of_each_turn() {
    let mut g = setup(&[Card::Shaman]);
    hand(&mut g, P0, &[Card::Shaman, Card::Estate, Card::Copper]);
    hand(&mut g, P1, &[Card::Copper; 5]);
    g.trash.clear();
    g.start();
    play(&mut g, Card::Shaman);
    assert_eq!((g.turn().actions, g.turn().coins), (1, 1));
    assert_eq!(purpose(&g), Purpose::Trash);
    choose_kinds(&mut g, &[Card::Estate]);
    next_turn(&mut g);
    // P1's turn starts: they must gain the Estate from the trash.
    assert_eq!(gain_count(&g, P1, Card::Estate), 1);
    assert!(g.trash().iter().all(|&c| g.card(c) != Card::Estate));
}

#[test]
fn shaman_gains_nothing_with_nothing_cheap_enough_in_the_trash() {
    let mut g = setup(&[Card::Shaman]);
    let province = g.create(Card::Province, Zone::Trash);
    g.trash.retain(|&c| c == province);
    hand(&mut g, P0, &[Card::Copper]);
    g.trash.retain(|&c| c == province);
    g.start();
    assert_eq!(g.trash(), [province]);
    assert_eq!(g.pending().unwrap().player, P0);
}

#[test]
fn secluded_shrine_trashes_up_to_two_when_a_treasure_is_gained_on_any_turn() {
    let mut g = setup(&[Card::SecludedShrine]);
    hand(
        &mut g,
        P0,
        &[
            Card::SecludedShrine,
            Card::Estate,
            Card::Estate,
            Card::Copper,
        ],
    );
    deck(&mut g, P0, &[Card::Copper; 20]);
    g.start();
    play(&mut g, Card::SecludedShrine);
    assert_eq!(g.turn().coins, 1);
    next_turn(&mut g);
    next_turn(&mut g);
    assert_eq!(in_play_kinds(&g, P0), [Card::SecludedShrine]);
    money(&mut g, 3, 1);
    to_buy(&mut g);
    buy(&mut g, Card::Silver);
    assert_eq!(purpose(&g), Purpose::Trash);
    let before = g.trash().len();
    let two: Vec<CardId> = g.hand(P0)[..2].to_vec();
    choose(&mut g, &two);
    assert_eq!(g.trash().len(), before + 2);
    assert!(g.lasting().is_empty());
    next_turn(&mut g);
    assert!(g.in_play(P0).is_empty());
}

#[test]
fn a_secluded_shrine_played_by_a_gained_gondola_waits_for_the_next_treasure() {
    let mut g = setup(&[Card::SecludedShrine, Card::Gondola]);
    hand(
        &mut g,
        P0,
        &[Card::SecludedShrine, Card::SecludedShrine, Card::Estate],
    );
    deck(&mut g, P0, &[Card::Copper; 20]);
    g.start();
    play(&mut g, Card::SecludedShrine);
    money(&mut g, 4, 1);
    to_buy(&mut g);
    buy(&mut g, Card::Gondola);
    // Gondola's when-gain and the first Shrine both trigger; play the second Shrine first.
    let gondola_first = match &g.pending().unwrap().kind {
        DecisionKind::Choose { labels, .. } => labels
            .iter()
            .position(|l| l.starts_with("Gondola"))
            .unwrap(),
        k => panic!("{k:?}"),
    };
    // The second Shrine is played (the only Action in hand); only the first resolves.
    respond(&mut g, Response::option(Opt::Trigger(gondola_first as u16)));
    assert_eq!(purpose(&g), Purpose::Trash);
    choose(&mut g, &[]);
    let shrines: Vec<_> = g
        .lasting()
        .iter()
        .filter(|l| l.card == Card::SecludedShrine)
        .collect();
    assert_eq!(shrines.len(), 1);
}

#[test]
fn siren_curses_and_draws_up_to_eight_next_turn() {
    let mut g = setup(&[Card::Siren]);
    hand(&mut g, P0, &[Card::Siren]);
    deck(&mut g, P0, &[Card::Copper; 20]);
    g.start();
    play(&mut g, Card::Siren);
    assert_eq!(gain_count(&g, P1, Card::Curse), 1);
    next_turn(&mut g);
    next_turn(&mut g);
    assert_eq!(g.hand(P0).len(), 8);
}

#[test]
fn a_gained_siren_is_trashed_unless_an_action_is_trashed() {
    let mut g = setup(&[Card::Siren, Card::Village]);
    hand(&mut g, P0, &[Card::Village, Card::Copper]);
    g.start();
    money(&mut g, 6, 2);
    to_buy(&mut g);
    buy(&mut g, Card::Siren);
    assert_eq!(purpose(&g), Purpose::Trash);
    choose_kinds(&mut g, &[Card::Village]);
    assert!(discard_kinds(&g, P0).contains(&Card::Siren));
    buy(&mut g, Card::Siren);
    // No Action left in hand: the Siren trashes itself.
    assert!(trash_kinds(&g).contains(&Card::Siren));
    assert_eq!(g.count_owned(P0, Card::Siren), 1);
}

#[test]
fn a_siren_moved_by_insignia_before_its_ability_survives() {
    let mut g = setup(&[Card::Siren, Card::SackOfLoot]);
    hand(&mut g, P0, &[Card::Insignia]);
    g.start();
    to_buy(&mut g);
    play(&mut g, Card::Insignia);
    buy(&mut g, Card::Siren);
    // Order Insignia first.
    let insignia = match &g.pending().unwrap().kind {
        DecisionKind::Choose { labels, .. } => labels
            .iter()
            .position(|l| l.starts_with("Insignia"))
            .unwrap(),
        k => panic!("{k:?}"),
    };
    respond(&mut g, Response::option(Opt::Trigger(insignia as u16)));
    assert_eq!(g.count_owned(P0, Card::Siren), 1);
    assert!(!trash_kinds(&g).contains(&Card::Siren));
}

#[test]
fn a_siren_gained_onto_the_deck_is_still_trashed() {
    let mut g = setup_l(&[Card::Siren], &[Card::Invasion]);
    hand(&mut g, P0, &[]);
    g.start();
    money(&mut g, 10, 1);
    to_buy(&mut g);
    buy_event(&mut g, Card::Invasion);
    assert_eq!(gain_count(&g, P0, Card::Siren), 1);
    assert!(trash_kinds(&g).contains(&Card::Siren));
}

#[test]
fn stowaway_is_played_from_hand_when_anyone_gains_a_duration() {
    let mut g = setup(&[Card::Stowaway, Card::Smithy]);
    hand(&mut g, P0, &[Card::Copper]);
    hand(&mut g, P1, &[Card::Stowaway, Card::Copper]);
    deck(&mut g, P1, &[Card::Silver; 10]);
    g.start();
    money(&mut g, 7, 2);
    to_buy(&mut g);
    buy(&mut g, Card::Smithy);
    assert_eq!(g.pending().unwrap().purpose, Purpose::Turn);
    buy(&mut g, Card::Stowaway);
    assert_eq!(g.pending().unwrap().player, P1);
    assert_eq!(purpose(&g), Purpose::Trigger);
    resolve_triggers(&mut g);
    assert_eq!(in_play_kinds(&g, P1), [Card::Stowaway]);
    // P0 had no Buys left, so P1's turn has started: +2 Cards from the Stowaway.
    assert_eq!(g.turn().player, P1);
    assert_eq!(g.hand(P1).len(), 1 + 2);
}

#[test]
fn taskmaster_repeats_next_turn_after_a_five_is_gained_and_not_otherwise() {
    let mut g = setup(&[Card::Taskmaster, Card::Market]);
    hand(&mut g, P0, &[Card::Taskmaster]);
    deck(&mut g, P0, &[Card::Copper; 30]);
    g.start();
    play(&mut g, Card::Taskmaster);
    assert_eq!((g.turn().actions, g.turn().coins), (1, 1));
    money(&mut g, 4, 1);
    to_buy(&mut g);
    buy(&mut g, Card::Market);
    assert_eq!(in_play_kinds(&g, P0), [Card::Taskmaster]);
    next_turn(&mut g);
    assert_eq!((g.turn().actions, g.turn().coins), (2, 1));
    assert_eq!(in_play_kinds(&g, P0), [Card::Taskmaster]);
    next_turn(&mut g);
    // No $5 gained: discarded in that turn's Clean-up.
    assert!(g.in_play(P0).is_empty());
    next_turn(&mut g);
    assert_eq!((g.turn().actions, g.turn().coins), (1, 0));
}

#[test]
fn taskmaster_ignores_a_five_gained_before_it_was_played() {
    let mut g = setup(&[Card::Taskmaster, Card::Market, Card::Artisan]);
    hand(&mut g, P0, &[Card::Artisan, Card::Taskmaster, Card::Copper]);
    deck(&mut g, P0, &[Card::Copper; 30]);
    g.start();
    g.turn.actions = 2;
    play(&mut g, Card::Artisan);
    choose_pile(&mut g, Card::Market);
    choose_kinds(&mut g, &[Card::Copper]);
    play(&mut g, Card::Taskmaster);
    next_turn(&mut g);
    assert!(g.in_play(P0).is_empty());
}

#[test]
fn taskmaster_uses_the_cost_when_gained() {
    let mut g = setup(&[Card::Taskmaster, Card::Village]);
    set_trait(&mut g, Card::Cheap, Card::Taskmaster);
    hand(&mut g, P0, &[Card::Taskmaster]);
    deck(&mut g, P0, &[Card::Copper; 30]);
    g.start();
    play(&mut g, Card::Taskmaster);
    assert_eq!(g.cost(Card::Taskmaster), Cost::coins(2));
    money(&mut g, 5, 1);
    to_buy(&mut g);
    // A Province costs $8; a Duchy $5.
    buy(&mut g, Card::Duchy);
    assert_eq!(in_play_kinds(&g, P0), [Card::Taskmaster]);
}

#[test]
fn abundance_gives_a_buy_and_three_coins_when_an_action_is_gained() {
    let mut g = setup(&[Card::Abundance, Card::Village]);
    hand(&mut g, P0, &[Card::Abundance]);
    deck(&mut g, P0, &[Card::Copper; 30]);
    g.start();
    to_buy(&mut g);
    play(&mut g, Card::Abundance);
    assert_eq!(g.turn().coins, 0);
    next_turn(&mut g);
    next_turn(&mut g);
    assert_eq!(in_play_kinds(&g, P0), [Card::Abundance]);
    money(&mut g, 3, 1);
    to_buy(&mut g);
    buy(&mut g, Card::Village);
    assert_eq!((g.turn().buys, g.turn().coins), (1, 3));
    assert!(g.lasting().is_empty());
    buy(&mut g, Card::Village);
    // The turn is over: Abundance was discarded in its Clean-up.
    assert_eq!(g.turn().player, P1);
    assert!(g.in_play(P0).is_empty());
}

#[test]
fn cabin_boy_gives_two_coins_or_is_trashed_for_a_duration() {
    let mut g = setup(&[Card::CabinBoy, Card::Longship]);
    hand(&mut g, P0, &[Card::CabinBoy, Card::CabinBoy]);
    deck(&mut g, P0, &[Card::Copper; 30]);
    g.start();
    play(&mut g, Card::CabinBoy);
    assert_eq!((g.turn().actions, g.hand(P0).len()), (1, 2));
    play(&mut g, Card::CabinBoy);
    next_turn(&mut g);
    next_turn(&mut g);
    resolve_triggers(&mut g);
    mode(&mut g, 0);
    assert_eq!(g.turn().coins, 2);
    resolve_triggers(&mut g);
    mode(&mut g, 1);
    choose_pile(&mut g, Card::Longship);
    assert_eq!(
        trash_kinds(&g)
            .iter()
            .filter(|&&c| c == Card::CabinBoy)
            .count(),
        1
    );
    assert_eq!(gain_count(&g, P0, Card::Longship), 1);
}

#[test]
fn crucible_gives_a_coin_per_coin_of_cost_of_the_trashed_card() {
    let mut g = setup(&[Card::Crucible]);
    hand(&mut g, P0, &[Card::Crucible, Card::Gold, Card::Estate]);
    g.start();
    to_buy(&mut g);
    play(&mut g, Card::Crucible);
    choose_kinds(&mut g, &[Card::Gold]);
    assert_eq!(g.turn().coins, 6);
}

#[test]
fn flagship_replays_the_next_non_command_action_even_one_that_trashed_itself() {
    let mut g = setup(&[Card::Flagship, Card::Feast, Card::Village]);
    hand(
        &mut g,
        P0,
        &[
            Card::Village,
            Card::Village,
            Card::Flagship,
            Card::Flagship,
            Card::Feast,
        ],
    );
    deck(&mut g, P0, &[Card::Copper; 30]);
    g.start();
    play(&mut g, Card::Village);
    play(&mut g, Card::Village);
    play(&mut g, Card::Flagship);
    assert_eq!(g.turn().coins, 2);
    // The second Flagship is a Command card: not replayed.
    play(&mut g, Card::Flagship);
    assert_eq!(g.turn().coins, 4);
    play(&mut g, Card::Feast);
    // Feast trashed itself and gains; both Flagships replay it.
    choose_pile(&mut g, Card::Village);
    resolve_triggers(&mut g);
    choose_pile(&mut g, Card::Village);
    resolve_triggers(&mut g);
    choose_pile(&mut g, Card::Village);
    assert_eq!(gain_count(&g, P0, Card::Village), 3);
    assert!(g.lasting().iter().all(|l| l.card != Card::Flagship));
    next_turn(&mut g);
    assert!(g.in_play(P0).is_empty());
}

#[test]
fn fortune_hunter_plays_a_treasure_from_the_top_three_before_putting_the_rest_back() {
    let mut g = setup(&[Card::FortuneHunter, Card::Figurine]);
    hand(&mut g, P0, &[Card::FortuneHunter]);
    deck(
        &mut g,
        P0,
        &[
            Card::Figurine,
            Card::Estate,
            Card::Duchy,
            Card::Gold,
            Card::Silver,
            Card::Copper,
        ],
    );
    g.start();
    play(&mut g, Card::FortuneHunter);
    assert_eq!(g.turn().coins, 2);
    assert_eq!(purpose(&g), Purpose::Play);
    choose_kinds(&mut g, &[Card::Figurine]);
    // Figurine draws the cards under the three looked at.
    assert_eq!(hand_kinds(&g, P0), [Card::Gold, Card::Silver]);
    assert_eq!(purpose(&g), Purpose::TopDeck);
    choose_kinds(&mut g, &[Card::Duchy, Card::Estate]);
    assert_eq!(
        deck_kinds(&g, P0),
        [Card::Duchy, Card::Estate, Card::Copper]
    );
}

#[test]
fn gondola_gives_two_coins_now_or_next_turn_and_plays_an_action_when_gained() {
    let mut g = setup(&[Card::Gondola, Card::Smithy]);
    hand(&mut g, P0, &[Card::Gondola, Card::Gondola, Card::Smithy]);
    deck(&mut g, P0, &[Card::Copper; 30]);
    g.start();
    to_buy(&mut g);
    play(&mut g, Card::Gondola);
    mode(&mut g, 0);
    assert_eq!(g.turn().coins, 2);
    play(&mut g, Card::Gondola);
    mode(&mut g, 1);
    assert_eq!(g.turn().coins, 2);
    money(&mut g, 2, 2);
    buy(&mut g, Card::Gondola);
    // Its when-gain: play the Smithy from hand, using no Action.
    resolve_triggers(&mut g);
    assert!(in_play_kinds(&g, P0).contains(&Card::Smithy));
    next_turn(&mut g);
    // Only the Gondola played for next turn stays.
    assert_eq!(in_play_kinds(&g, P0), [Card::Gondola]);
    next_turn(&mut g);
    assert_eq!(g.turn().coins, 2);
}

#[test]
fn kings_cache_on_a_gondola_chooses_each_time_and_stays_with_it() {
    let mut g = setup(&[Card::Gondola, Card::KingsCache]);
    hand(&mut g, P0, &[Card::KingsCache, Card::Gondola]);
    deck(&mut g, P0, &[Card::Copper; 30]);
    g.start();
    to_buy(&mut g);
    play(&mut g, Card::KingsCache);
    choose_kinds(&mut g, &[Card::Gondola]);
    mode(&mut g, 0);
    mode(&mut g, 1);
    mode(&mut g, 0);
    assert_eq!(g.turn().coins, 4);
    next_turn(&mut g);
    assert_eq!(in_play_kinds(&g, P0), [Card::KingsCache, Card::Gondola]);
    next_turn(&mut g);
    assert_eq!(g.turn().coins, 2);
    next_turn(&mut g);
    assert!(g.in_play(P0).is_empty());
}

#[test]
fn kings_cache_on_a_kings_cache_plays_three_more_treasures_three_times_each() {
    let mut g = setup(&[Card::KingsCache]);
    hand(
        &mut g,
        P0,
        &[
            Card::KingsCache,
            Card::KingsCache,
            Card::Copper,
            Card::Silver,
            Card::Gold,
        ],
    );
    g.start();
    to_buy(&mut g);
    play(&mut g, Card::KingsCache);
    choose_kinds(&mut g, &[Card::KingsCache]);
    choose_kinds(&mut g, &[Card::Copper]);
    choose_kinds(&mut g, &[Card::Silver]);
    choose_kinds(&mut g, &[Card::Gold]);
    assert_eq!(g.turn().coins, 3 * (1 + 2 + 3));
}

#[test]
fn harbor_village_adds_a_coin_after_the_next_action_if_it_gave_coins() {
    let mut g = setup(&[Card::HarborVillage, Card::Militia, Card::Village]);
    hand(
        &mut g,
        P0,
        &[
            Card::HarborVillage,
            Card::Village,
            Card::HarborVillage,
            Card::Militia,
        ],
    );
    deck(&mut g, P0, &[Card::Copper; 30]);
    hand(&mut g, P1, &[Card::Copper; 3]);
    g.start();
    play(&mut g, Card::HarborVillage);
    assert_eq!(g.turn().actions, 2);
    play(&mut g, Card::Village);
    assert_eq!(g.turn().coins, 0);
    assert!(g.lasting().is_empty());
    play(&mut g, Card::HarborVillage);
    play(&mut g, Card::Militia);
    assert_eq!(g.turn().coins, 3);
}

#[test]
fn harbor_village_played_twice_by_throne_room_pays_only_for_the_second_play() {
    let mut g = setup(&[Card::HarborVillage, Card::Militia, Card::ThroneRoom]);
    hand(
        &mut g,
        P0,
        &[Card::ThroneRoom, Card::HarborVillage, Card::Militia],
    );
    deck(&mut g, P0, &[Card::Copper; 30]);
    hand(&mut g, P1, &[Card::Copper; 3]);
    g.start();
    play(&mut g, Card::ThroneRoom);
    choose_kinds(&mut g, &[Card::HarborVillage]);
    play(&mut g, Card::Militia);
    assert_eq!(g.turn().coins, 3);
}

#[test]
fn landing_party_goes_onto_the_deck_after_a_turn_whose_first_card_is_a_treasure() {
    let mut g = setup(&[Card::LandingParty, Card::Figurine]);
    hand(&mut g, P0, &[Card::LandingParty]);
    let mut cards = vec![Card::Copper, Card::Copper, Card::Figurine];
    cards.extend([Card::Copper; 4]);
    cards.extend([Card::Silver; 20]);
    deck(&mut g, P0, &cards);
    g.start();
    play(&mut g, Card::LandingParty);
    assert_eq!((g.hand(P0).len(), g.turn().actions), (2, 2));
    // Not the first card of this turn.
    to_buy(&mut g);
    play(&mut g, Card::Copper);
    next_turn(&mut g);
    next_turn(&mut g);
    assert_eq!(in_play_kinds(&g, P0), [Card::LandingParty]);
    to_buy(&mut g);
    play(&mut g, Card::Figurine);
    // Figurine drew first; then the Landing Party went onto the deck.
    assert_eq!(g.hand(P0).len(), 6);
    assert_eq!(deck_kinds(&g, P0)[0], Card::LandingParty);
    assert!(g.lasting().is_empty());
}

#[test]
fn landing_party_stays_when_the_first_card_played_is_an_action() {
    let mut g = setup(&[Card::LandingParty, Card::Village]);
    hand(&mut g, P0, &[Card::LandingParty]);
    deck(
        &mut g,
        P0,
        &[
            Card::Copper,
            Card::Copper,
            Card::Village,
            Card::Copper,
            Card::Copper,
            Card::Copper,
            Card::Copper,
        ]
        .iter()
        .chain(&[Card::Silver; 20])
        .copied()
        .collect::<Vec<_>>(),
    );
    g.start();
    play(&mut g, Card::LandingParty);
    next_turn(&mut g);
    next_turn(&mut g);
    play(&mut g, Card::Village);
    to_buy(&mut g);
    play(&mut g, Card::Copper);
    assert_eq!(
        in_play_kinds(&g, P0),
        [Card::LandingParty, Card::Village, Card::Copper]
    );
    next_turn(&mut g);
    assert_eq!(in_play_kinds(&g, P0), [Card::LandingParty]);
}

#[test]
fn mapmaker_takes_two_of_the_top_four_and_is_played_when_anyone_gains_a_victory_card() {
    let mut g = setup(&[Card::Mapmaker]);
    hand(&mut g, P0, &[Card::Mapmaker]);
    deck(
        &mut g,
        P0,
        &[
            Card::Gold,
            Card::Estate,
            Card::Silver,
            Card::Curse,
            Card::Copper,
        ],
    );
    hand(&mut g, P1, &[Card::Mapmaker, Card::Copper]);
    deck(
        &mut g,
        P1,
        &[
            Card::Mapmaker,
            Card::Gold,
            Card::Silver,
            Card::Estate,
            Card::Copper,
        ],
    );
    g.start();
    play(&mut g, Card::Mapmaker);
    choose_kinds(&mut g, &[Card::Gold, Card::Silver]);
    assert_eq!(hand_kinds(&g, P0), [Card::Gold, Card::Silver]);
    assert_eq!(discard_kinds(&g, P0), [Card::Estate, Card::Curse]);
    money(&mut g, 2, 1);
    to_buy(&mut g);
    buy(&mut g, Card::Estate);
    // P1 may play Mapmaker in response, and then the Mapmaker it drew.
    assert_eq!(g.pending().unwrap().player, P1);
    resolve_triggers(&mut g);
    choose_kinds(&mut g, &[Card::Mapmaker, Card::Gold]);
    assert_eq!(g.pending().unwrap().player, P1);
    resolve_triggers(&mut g);
    assert_eq!(in_play_kinds(&g, P1), [Card::Mapmaker, Card::Mapmaker]);
}

#[test]
fn mapmaker_with_fewer_than_four_cards_looks_at_what_is_left() {
    let mut g = setup(&[Card::Mapmaker]);
    hand(&mut g, P0, &[Card::Mapmaker]);
    deck(&mut g, P0, &[Card::Gold]);
    discard(&mut g, P0, &[]);
    g.start();
    play(&mut g, Card::Mapmaker);
    assert_eq!(hand_kinds(&g, P0), [Card::Gold]);
}

#[test]
fn maroon_draws_two_cards_per_type_of_the_trashed_card() {
    let mut g = setup(&[Card::Maroon, Card::Cage]);
    set_trait(&mut g, Card::Rich, Card::Cage);
    hand(&mut g, P0, &[Card::Maroon, Card::Cage, Card::Estate]);
    deck(&mut g, P0, &[Card::Copper; 10]);
    g.start();
    play(&mut g, Card::Maroon);
    // Treasure and Duration; the Trait isn't a type.
    choose_kinds(&mut g, &[Card::Cage]);
    assert_eq!(g.hand(P0).len(), 1 + 4);
}

#[test]
fn rope_gives_a_coin_and_a_buy_then_a_card_and_a_trash_next_turn() {
    let mut g = setup(&[Card::Rope]);
    hand(&mut g, P0, &[Card::Rope]);
    deck(
        &mut g,
        P0,
        &[Card::Copper; 5]
            .iter()
            .chain(&[Card::Estate])
            .chain(&[Card::Copper; 10])
            .copied()
            .collect::<Vec<_>>(),
    );
    g.start();
    to_buy(&mut g);
    play(&mut g, Card::Rope);
    assert_eq!((g.turn().coins, g.turn().buys), (1, 2));
    next_turn(&mut g);
    next_turn(&mut g);
    assert_eq!(g.hand(P0).len(), 6);
    assert_eq!(purpose(&g), Purpose::Trash);
    choose_kinds(&mut g, &[Card::Estate]);
    assert!(trash_kinds(&g).contains(&Card::Estate));
}

#[test]
fn swamp_shacks_draws_a_card_per_three_cards_in_play() {
    let mut g = setup(&[Card::SwampShacks, Card::Longship]);
    hand(
        &mut g,
        P0,
        &[Card::Longship, Card::SwampShacks, Card::SwampShacks],
    );
    deck(&mut g, P0, &[Card::Copper; 30]);
    g.start();
    play(&mut g, Card::Longship);
    play(&mut g, Card::SwampShacks);
    assert_eq!(g.hand(P0).len(), 1);
    play(&mut g, Card::SwampShacks);
    // Three cards in play, counting itself.
    assert_eq!(g.hand(P0).len(), 1);
    assert_eq!(g.turn().actions, 4);
}

#[test]
fn tools_gains_a_copy_of_a_card_anyone_has_in_play_but_not_a_loot() {
    let mut g = setup(&[Card::Tools, Card::Longship, Card::SackOfLoot]);
    hand(&mut g, P0, &[Card::Tools, Card::Hammer]);
    g.start();
    let longship = g.supply_pile(Card::Longship).unwrap();
    let top = g.pile_top(longship).unwrap();
    g.move_card(top, Zone::InPlay(P1), crate::game::Pos::Top);
    to_buy(&mut g);
    play(&mut g, Card::Hammer);
    choose_pile(&mut g, Card::Copper);
    play(&mut g, Card::Tools);
    let DecisionKind::Pile { piles, .. } = &g.pending().unwrap().kind else {
        panic!()
    };
    let names: Vec<Card> = piles.iter().map(|&p| g.pile(p).card).collect();
    assert_eq!(names, [Card::Tools, Card::Longship]);
    choose_pile(&mut g, Card::Longship);
    assert_eq!(gain_count(&g, P0, Card::Longship), 1);
}

#[test]
fn buried_treasure_is_played_when_gained_and_pays_next_turn() {
    let mut g = setup(&[Card::BuriedTreasure]);
    hand(&mut g, P0, &[]);
    deck(&mut g, P0, &[Card::Copper; 30]);
    g.start();
    money(&mut g, 5, 1);
    to_buy(&mut g);
    buy(&mut g, Card::BuriedTreasure);
    assert_eq!(in_play_kinds(&g, P1), []);
    assert_eq!(in_play_kinds(&g, P0), [Card::BuriedTreasure]);
    next_turn(&mut g);
    assert_eq!((g.turn().coins, g.turn().buys), (3, 2));
}

#[test]
fn crew_draws_three_and_goes_onto_the_deck_next_turn() {
    let mut g = setup(&[Card::Crew]);
    hand(&mut g, P0, &[Card::Crew]);
    deck(&mut g, P0, &[Card::Copper; 30]);
    g.start();
    play(&mut g, Card::Crew);
    assert_eq!(g.hand(P0).len(), 3);
    next_turn(&mut g);
    next_turn(&mut g);
    assert_eq!(deck_kinds(&g, P0)[0], Card::Crew);
    assert!(g.in_play(P0).is_empty());
}

#[test]
fn cutthroat_attacks_and_gains_a_loot_when_anyone_gains_a_treasure_costing_five_or_more() {
    let mut g = setup(&[Card::Cutthroat, Card::Moat]);
    hand(&mut g, P0, &[Card::Cutthroat]);
    deck(&mut g, P0, &[Card::Copper; 30]);
    deck(&mut g, P1, &[Card::Copper; 30]);
    g.start();
    play(&mut g, Card::Cutthroat);
    let two = g.hand(P1)[..2].to_vec();
    choose(&mut g, &two);
    assert_eq!(g.hand(P1).len(), 3);
    next_turn(&mut g);
    money(&mut g, 9, 2);
    to_buy(&mut g);
    buy(&mut g, Card::Silver);
    assert_eq!(loot_gained(&g, P0), 0);
    buy(&mut g, Card::Gold);
    // P1 gained a Gold: P0 gains a Loot.
    assert_eq!(loot_gained(&g, P0), 1);
    assert!(g.lasting().is_empty());
}

#[test]
fn a_loot_gained_triggers_cutthroats() {
    let mut g = setup(&[Card::Cutthroat]);
    hand(
        &mut g,
        P0,
        &[Card::Cutthroat, Card::Cutthroat, Card::Village],
    );
    deck(&mut g, P0, &[Card::Copper; 30]);
    hand(&mut g, P1, &[Card::Copper; 3]);
    g.start();
    g.turn.actions = 1;
    play(&mut g, Card::Village);
    play(&mut g, Card::Cutthroat);
    play(&mut g, Card::Cutthroat);
    money(&mut g, 6, 1);
    to_buy(&mut g);
    buy(&mut g, Card::Gold);
    resolve_triggers(&mut g);
    // The first Cutthroat's Loot wakes the second.
    assert_eq!(loot_gained(&g, P0), 2);
}

#[test]
fn enlarge_remodels_now_and_at_the_start_of_the_next_turn() {
    let mut g = setup(&[Card::Enlarge]);
    hand(&mut g, P0, &[Card::Enlarge, Card::Estate]);
    deck(&mut g, P0, &[Card::Copper; 30]);
    g.start();
    play(&mut g, Card::Enlarge);
    choose_pile(&mut g, Card::Silver);
    next_turn(&mut g);
    next_turn(&mut g);
    // Trashing a Copper from a hand of Coppers needs no choice.
    choose_pile(&mut g, Card::Estate);
    assert_eq!(gain_count(&g, P0, Card::Silver), 1);
    assert_eq!(gain_count(&g, P0, Card::Estate), 1);
}

#[test]
fn figurine_draws_two_and_may_discard_an_action_for_a_buy_and_a_coin() {
    let mut g = setup(&[Card::Figurine, Card::Village]);
    hand(&mut g, P0, &[Card::Figurine]);
    deck(&mut g, P0, &[Card::Village, Card::Copper]);
    g.start();
    to_buy(&mut g);
    play(&mut g, Card::Figurine);
    choose_kinds(&mut g, &[Card::Village]);
    assert_eq!((g.turn().buys, g.turn().coins), (2, 1));
    assert_eq!(discard_kinds(&g, P0), [Card::Village]);
}
