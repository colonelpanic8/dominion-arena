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
    for seed in 0..fuzz_seeds(150) {
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
fn a_secluded_shrine_that_left_play_still_waits_that_turn_only() {
    let shrine = |gain_now: bool| {
        let mut g = setup(&[Card::SecludedShrine]);
        hand(&mut g, P0, &[Card::SecludedShrine, Card::Estate]);
        deck(&mut g, P0, &[Card::Copper, Card::Estate].repeat(10));
        g.start();
        play(&mut g, Card::SecludedShrine);
        leave_play(&mut g, P0, Card::SecludedShrine);
        if !gain_now {
            next_turn(&mut g);
            next_turn(&mut g);
        }
        money(&mut g, 3, 2);
        to_buy(&mut g);
        buy(&mut g, Card::Silver);
        g
    };
    assert_eq!(purpose(&shrine(true)), Purpose::Trash);
    let later = shrine(false);
    assert_eq!((later.turn().player, purpose(&later)), (P0, Purpose::Turn));
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
fn a_taskmaster_that_left_play_does_not_repeat() {
    let mut g = setup(&[Card::Taskmaster, Card::Market]);
    hand(&mut g, P0, &[Card::Taskmaster]);
    deck(&mut g, P0, &[Card::Copper; 30]);
    g.start();
    play(&mut g, Card::Taskmaster);
    leave_play(&mut g, P0, Card::Taskmaster);
    money(&mut g, 4, 1);
    to_buy(&mut g);
    buy(&mut g, Card::Market);
    next_turn(&mut g);
    assert_eq!(g.turn().player, P0);
    assert_eq!((g.turn().actions, g.turn().coins), (1, 0));
    assert!(g.lasting().is_empty());
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

#[test]
fn first_mate_plays_copies_of_an_action_including_drawn_ones_then_draws_to_six() {
    let mut g = setup(&[Card::FirstMate, Card::Smithy]);
    hand(&mut g, P0, &[Card::FirstMate, Card::Smithy, Card::Village]);
    deck(
        &mut g,
        P0,
        &[
            Card::Smithy,
            Card::Copper,
            Card::Copper,
            Card::Silver,
            Card::Silver,
            Card::Silver,
            Card::Gold,
            Card::Gold,
            Card::Gold,
            Card::Gold,
        ],
    );
    g.start();
    play(&mut g, Card::FirstMate);
    choose_kinds(&mut g, &[Card::Smithy]);
    // The Smithy drew another Smithy; only Smithies are offered now.
    assert_eq!(kinds(&g, &candidates(&g)), [Card::Smithy]);
    choose_kinds(&mut g, &[Card::Smithy]);
    assert_eq!(
        in_play_kinds(&g, P0),
        [Card::FirstMate, Card::Smithy, Card::Smithy]
    );
    assert_eq!(g.hand(P0).len(), 6);
}

#[test]
fn first_mate_with_no_action_just_draws_to_six() {
    let mut g = setup(&[Card::FirstMate]);
    hand(&mut g, P0, &[Card::FirstMate, Card::Copper]);
    deck(&mut g, P0, &[Card::Copper; 10]);
    g.start();
    play(&mut g, Card::FirstMate);
    assert_eq!(g.hand(P0).len(), 6);
}

#[test]
fn frigate_makes_others_discard_down_to_four_after_each_action_until_its_next_turn() {
    let mut g = setup(&[Card::Frigate, Card::Village]);
    hand(&mut g, P0, &[Card::Frigate]);
    deck(&mut g, P0, &[Card::Copper; 30]);
    hand(
        &mut g,
        P1,
        &[
            Card::Village,
            Card::Village,
            Card::Copper,
            Card::Copper,
            Card::Copper,
        ],
    );
    deck(&mut g, P1, &[Card::Estate; 10]);
    g.start();
    play(&mut g, Card::Frigate);
    assert_eq!(g.turn().coins, 3);
    next_turn(&mut g);
    play(&mut g, Card::Village);
    // Village drew to 5: discard down to 4 after it resolved.
    assert_eq!(g.pending().unwrap().player, P1);
    assert_eq!(purpose(&g), Purpose::Discard);
    choose_kinds(&mut g, &[Card::Estate]);
    assert_eq!(g.hand(P1).len(), 4);
    // Back at 4 after the second Village: nothing to discard.
    play(&mut g, Card::Village);
    assert_eq!(g.hand(P1).len(), 4);
    next_turn(&mut g);
    assert_eq!(in_play_kinds(&g, P0), [Card::Frigate]);
    assert!(g.lasting().is_empty());
    next_turn(&mut g);
    assert!(g.in_play(P0).is_empty());
}

#[test]
fn a_frigate_nobody_is_hit_by_is_discarded_that_turn() {
    let mut g = setup(&[Card::Frigate, Card::Moat]);
    hand(&mut g, P0, &[Card::Frigate]);
    hand(&mut g, P1, &[Card::Moat]);
    g.start();
    play(&mut g, Card::Frigate);
    yes_trigger(&mut g);
    next_turn(&mut g);
    assert!(g.in_play(P0).is_empty());
}

/// Accepts the one optional trigger offered.
fn yes_trigger(g: &mut Game) {
    respond(g, Response::option(Opt::Trigger(0)));
}

#[test]
fn longship_gives_two_actions_now_and_two_cards_next_turn() {
    let mut g = setup(&[Card::Longship]);
    hand(&mut g, P0, &[Card::Longship]);
    deck(&mut g, P0, &[Card::Copper; 30]);
    g.start();
    play(&mut g, Card::Longship);
    assert_eq!(g.turn().actions, 2);
    next_turn(&mut g);
    next_turn(&mut g);
    assert_eq!(g.hand(P0).len(), 7);
}

#[test]
fn mining_road_may_play_a_gained_treasure_once_and_two_roads_cannot_play_it_twice() {
    let mut g = setup(&[Card::MiningRoad]);
    hand(&mut g, P0, &[Card::MiningRoad, Card::MiningRoad]);
    deck(&mut g, P0, &[Card::Copper; 30]);
    g.start();
    play(&mut g, Card::MiningRoad);
    play(&mut g, Card::MiningRoad);
    assert_eq!((g.turn().coins, g.turn().buys), (4, 3));
    to_buy(&mut g);
    buy(&mut g, Card::Silver);
    // Both Roads offer; once one plays it, the other has nothing to play.
    assert_eq!(purpose(&g), Purpose::Trigger);
    yes_trigger(&mut g);
    assert_eq!(g.turn().coins, 1 + 2);
    assert_eq!(
        g.lasting()
            .iter()
            .filter(|l| l.card == Card::MiningRoad)
            .count(),
        1
    );
    buy(&mut g, Card::Copper);
    yes_trigger(&mut g);
    assert_eq!(g.turn().coins, 4);
    assert!(g.lasting().is_empty());
}

#[test]
fn pendant_counts_differently_named_treasures_in_play_including_itself() {
    let mut g = setup(&[Card::Pendant, Card::Gondola]);
    hand(
        &mut g,
        P0,
        &[Card::Pendant, Card::Copper, Card::Copper, Card::Silver],
    );
    g.start();
    to_buy(&mut g);
    play(&mut g, Card::Copper);
    play(&mut g, Card::Copper);
    play(&mut g, Card::Silver);
    play(&mut g, Card::Pendant);
    assert_eq!(g.turn().coins, 4 + 3);
}

#[test]
fn pickaxe_trashes_a_card_and_gains_a_loot_to_hand_for_three_or_more() {
    let mut g = setup(&[Card::Pickaxe]);
    hand(
        &mut g,
        P0,
        &[Card::Pickaxe, Card::Pickaxe, Card::Silver, Card::Estate],
    );
    g.start();
    to_buy(&mut g);
    play(&mut g, Card::Pickaxe);
    choose_kinds(&mut g, &[Card::Estate]);
    assert_eq!(loot_gained(&g, P0), 0);
    // The Silver is the only card left to trash.
    play(&mut g, Card::Pickaxe);
    assert_eq!(loot_gained(&g, P0), 1);
    assert!(g.hand(P0).iter().any(|&c| g.card(c).is(Types::LOOT)));
}

#[test]
fn pilgrim_draws_four_and_puts_any_card_from_hand_onto_the_deck() {
    let mut g = setup(&[Card::Pilgrim]);
    hand(&mut g, P0, &[Card::Pilgrim, Card::Estate]);
    deck(&mut g, P0, &[Card::Copper; 10]);
    g.start();
    play(&mut g, Card::Pilgrim);
    choose_kinds(&mut g, &[Card::Estate]);
    assert_eq!(g.hand(P0).len(), 4);
    assert_eq!(deck_kinds(&g, P0)[0], Card::Estate);
}

#[test]
fn quartermaster_gains_cards_aside_and_hands_them_out_on_later_turns() {
    let mut g = setup(&[Card::Quartermaster, Card::ThroneRoom, Card::Smithy]);
    hand(&mut g, P0, &[Card::ThroneRoom, Card::Quartermaster]);
    deck(&mut g, P0, &[Card::Copper; 40]);
    g.start();
    play(&mut g, Card::ThroneRoom);
    choose_kinds(&mut g, &[Card::Quartermaster]);
    next_turn(&mut g);
    next_turn(&mut g);
    // Throne Room: one set of cards, two choices each turn.
    resolve_triggers(&mut g);
    mode(&mut g, 0);
    choose_pile(&mut g, Card::Silver);
    resolve_triggers(&mut g);
    mode(&mut g, 0);
    choose_pile(&mut g, Card::Smithy);
    assert_eq!(aside_kinds(&g, P0), [Card::Silver, Card::Smithy]);
    next_turn(&mut g);
    next_turn(&mut g);
    resolve_triggers(&mut g);
    mode(&mut g, 1);
    choose_kinds(&mut g, &[Card::Smithy]);
    resolve_triggers(&mut g);
    mode(&mut g, 1);
    assert!(hand_kinds(&g, P0).contains(&Card::Silver));
    assert!(hand_kinds(&g, P0).contains(&Card::Smithy));
    assert_eq!(
        in_play_kinds(&g, P0),
        [Card::ThroneRoom, Card::Quartermaster]
    );
}

#[test]
fn quartermaster_played_at_the_start_of_a_turn_acts_that_turn() {
    let mut g = setup_with(GameConfig {
        landscapes: vec![Card::Piazza],
        ..GameConfig::new(2, vec![Card::Quartermaster], 7)
    });
    g.players[0].projects.push(Card::Piazza);
    hand(&mut g, P0, &[]);
    deck(&mut g, P0, &[Card::Quartermaster, Card::Copper]);
    g.start();
    // Piazza played it; it is still the start of the turn.
    mode(&mut g, 0);
    choose_pile(&mut g, Card::Silver);
    assert_eq!(aside_kinds(&g, P0), [Card::Silver]);
}

#[test]
fn silver_mine_gains_a_cheaper_treasure_to_hand() {
    let mut g = setup(&[Card::SilverMine, Card::Gondola]);
    hand(&mut g, P0, &[Card::SilverMine]);
    g.start();
    to_buy(&mut g);
    play(&mut g, Card::SilverMine);
    let DecisionKind::Pile { piles, .. } = &g.pending().unwrap().kind else {
        panic!()
    };
    let names: Vec<Card> = piles.iter().map(|&p| g.pile(p).card).collect();
    assert_eq!(names, [Card::Copper, Card::Silver, Card::Gondola]);
    choose_pile(&mut g, Card::Gondola);
    assert_eq!(hand_kinds(&g, P0), [Card::Gondola]);
}

#[test]
fn tricksters_curse_and_each_keep_a_treasure_for_the_next_hand() {
    let mut g = setup(&[Card::Trickster, Card::Village]);
    hand(
        &mut g,
        P0,
        &[
            Card::Village,
            Card::Trickster,
            Card::Trickster,
            Card::Gold,
            Card::Silver,
        ],
    );
    deck(&mut g, P0, &[Card::Copper; 30]);
    hand(&mut g, P1, &[Card::Copper; 3]);
    g.start();
    play(&mut g, Card::Village);
    play(&mut g, Card::Trickster);
    play(&mut g, Card::Trickster);
    assert_eq!(gain_count(&g, P1, Card::Curse), 2);
    to_buy(&mut g);
    play(&mut g, Card::Gold);
    play(&mut g, Card::Silver);
    end_phase(&mut g);
    resolve_triggers(&mut g);
    choose_kinds(&mut g, &[Card::Gold]);
    resolve_triggers(&mut g);
    // The Silver is the only Treasure left: no choice.
    assert_eq!(g.turn().player, P1);
    let mut hand = hand_kinds(&g, P0);
    hand.sort();
    assert_eq!(
        hand,
        [
            Card::Copper,
            Card::Copper,
            Card::Copper,
            Card::Copper,
            Card::Copper,
            Card::Gold,
            Card::Silver
        ]
    );
}

#[test]
fn wealthy_village_gains_a_loot_with_three_differently_named_treasures_in_play() {
    for (treasures, loot) in [
        ([Card::Copper, Card::Silver, Card::Silver], 0),
        ([Card::Copper, Card::Silver, Card::Gold], 1),
    ] {
        let mut g = setup(&[Card::WealthyVillage]);
        hand(&mut g, P0, &treasures);
        g.start();
        to_buy(&mut g);
        for t in treasures {
            play(&mut g, t);
        }
        money(&mut g, 0, 2);
        buy(&mut g, Card::WealthyVillage);
        assert_eq!(loot_gained(&g, P0), loot);
    }
}

#[test]
fn sack_of_loot_gives_a_coin_a_buy_and_a_loot() {
    let mut g = setup(&[Card::SackOfLoot]);
    hand(&mut g, P0, &[Card::SackOfLoot]);
    g.start();
    to_buy(&mut g);
    play(&mut g, Card::SackOfLoot);
    assert_eq!((g.turn().coins, g.turn().buys), (1, 2));
    assert_eq!(loot_gained(&g, P0), 1);
}

#[test]
fn amphora_gives_a_buy_and_three_coins_now_or_next_turn() {
    let mut g = setup(&[Card::SackOfLoot]);
    hand(&mut g, P0, &[Card::Amphora, Card::Amphora]);
    deck(&mut g, P0, &[Card::Copper; 30]);
    g.start();
    to_buy(&mut g);
    play(&mut g, Card::Amphora);
    mode(&mut g, 0);
    assert_eq!((g.turn().coins, g.turn().buys), (3, 2));
    play(&mut g, Card::Amphora);
    mode(&mut g, 1);
    assert_eq!((g.turn().coins, g.turn().buys), (3, 2));
    next_turn(&mut g);
    assert_eq!(in_play_kinds(&g, P0), [Card::Amphora]);
    next_turn(&mut g);
    assert_eq!((g.turn().coins, g.turn().buys), (3, 2));
}

#[test]
fn doubloons_comes_with_a_gold() {
    let mut g = setup(&[Card::SackOfLoot]);
    hand(&mut g, P0, &[Card::SackOfLoot]);
    g.start();
    loot_top(&mut g, Card::Doubloons);
    to_buy(&mut g);
    play(&mut g, Card::SackOfLoot);
    assert_eq!(gain_count(&g, P0, Card::Doubloons), 1);
    assert_eq!(gain_count(&g, P0, Card::Gold), 1);
}

#[test]
fn endless_chalice_pays_a_coin_and_a_buy_every_turn_for_the_rest_of_the_game() {
    let mut g = setup(&[Card::SackOfLoot]);
    hand(&mut g, P0, &[Card::EndlessChalice]);
    deck(&mut g, P0, &[Card::Copper; 40]);
    g.start();
    to_buy(&mut g);
    play(&mut g, Card::EndlessChalice);
    assert_eq!((g.turn().coins, g.turn().buys), (1, 2));
    for _ in 0..3 {
        next_turn(&mut g);
        next_turn(&mut g);
        assert_eq!((g.turn().coins, g.turn().buys), (1, 2));
        assert_eq!(in_play_kinds(&g, P0), [Card::EndlessChalice]);
    }
}

#[test]
fn figurehead_pays_three_and_draws_two_next_turn() {
    let mut g = setup(&[Card::SackOfLoot]);
    hand(&mut g, P0, &[Card::Figurehead]);
    deck(&mut g, P0, &[Card::Copper; 30]);
    g.start();
    to_buy(&mut g);
    play(&mut g, Card::Figurehead);
    assert_eq!(g.turn().coins, 3);
    next_turn(&mut g);
    next_turn(&mut g);
    assert_eq!(g.hand(P0).len(), 7);
}

#[test]
fn hammer_pays_three_and_gains_a_card_costing_up_to_four() {
    let mut g = setup(&[Card::SackOfLoot, Card::Smithy]);
    hand(&mut g, P0, &[Card::Hammer]);
    g.start();
    to_buy(&mut g);
    play(&mut g, Card::Hammer);
    choose_pile(&mut g, Card::Smithy);
    assert_eq!(g.turn().coins, 3);
    assert_eq!(gain_count(&g, P0, Card::Smithy), 1);
}

#[test]
fn insignia_lets_each_card_gained_this_turn_go_onto_the_deck() {
    let mut g = setup(&[Card::SackOfLoot]);
    hand(&mut g, P0, &[Card::Insignia]);
    deck(&mut g, P0, &[Card::Estate; 10]);
    g.start();
    to_buy(&mut g);
    play(&mut g, Card::Insignia);
    money(&mut g, 3, 3);
    buy(&mut g, Card::Silver);
    yes_trigger(&mut g);
    assert_eq!(deck_kinds(&g, P0)[0], Card::Silver);
    buy(&mut g, Card::Copper);
    respond(&mut g, Response::option(Opt::No));
    assert_eq!(deck_kinds(&g, P0)[0], Card::Silver);
    assert_eq!(discard_kinds(&g, P0), [Card::Copper]);
}

#[test]
fn jewels_goes_to_the_bottom_of_the_deck_next_turn_leaving_kings_cache_until_clean_up() {
    let mut g = setup(&[Card::KingsCache, Card::SackOfLoot]);
    hand(&mut g, P0, &[Card::KingsCache, Card::Jewels]);
    deck(&mut g, P0, &[Card::Copper; 30]);
    g.start();
    to_buy(&mut g);
    play(&mut g, Card::KingsCache);
    choose_kinds(&mut g, &[Card::Jewels]);
    assert_eq!((g.turn().coins, g.turn().buys), (9, 4));
    next_turn(&mut g);
    assert_eq!(in_play_kinds(&g, P0), [Card::KingsCache, Card::Jewels]);
    next_turn(&mut g);
    resolve_triggers(&mut g);
    assert_eq!(
        g.player(P0).deck.first().map(|&c| g.card(c)),
        Some(Card::Jewels)
    );
    assert_eq!(in_play_kinds(&g, P0), [Card::KingsCache]);
    next_turn(&mut g);
    assert!(g.in_play(P0).is_empty());
}

#[test]
fn orb_plays_a_card_from_the_discard_pile_or_gives_a_buy_and_three_coins() {
    let mut g = setup(&[Card::SackOfLoot]);
    hand(&mut g, P0, &[Card::Orb, Card::Orb]);
    discard(&mut g, P0, &[Card::Estate, Card::Gold]);
    g.start();
    to_buy(&mut g);
    play(&mut g, Card::Orb);
    mode(&mut g, 0);
    assert_eq!(g.turn().coins, 3);
    assert!(in_play_kinds(&g, P0).contains(&Card::Gold));
    play(&mut g, Card::Orb);
    mode(&mut g, 1);
    assert_eq!((g.turn().coins, g.turn().buys), (6, 2));
}

#[test]
fn prize_goat_pays_three_and_a_buy_and_may_trash() {
    let mut g = setup(&[Card::SackOfLoot]);
    hand(&mut g, P0, &[Card::PrizeGoat, Card::Curse, Card::Copper]);
    g.start();
    to_buy(&mut g);
    play(&mut g, Card::PrizeGoat);
    choose_kinds(&mut g, &[Card::Curse]);
    assert_eq!((g.turn().coins, g.turn().buys), (3, 2));
    assert_eq!(trash_kinds(&g).last(), Some(&Card::Curse));
}

#[test]
fn puzzle_box_keeps_a_card_for_the_next_hand() {
    let mut g = setup(&[Card::SackOfLoot]);
    hand(&mut g, P0, &[Card::PuzzleBox, Card::Gold]);
    deck(&mut g, P0, &[Card::Copper; 30]);
    g.start();
    to_buy(&mut g);
    play(&mut g, Card::PuzzleBox);
    choose_kinds(&mut g, &[Card::Gold]);
    next_turn(&mut g);
    assert_eq!(g.hand(P0).len(), 6);
    assert!(hand_kinds(&g, P0).contains(&Card::Gold));
    assert!(discard_kinds(&g, P0).contains(&Card::PuzzleBox));
}

#[test]
fn sextant_discards_any_of_the_top_five_and_puts_the_rest_back_in_any_order() {
    let mut g = setup(&[Card::SackOfLoot]);
    hand(&mut g, P0, &[Card::Sextant]);
    deck(
        &mut g,
        P0,
        &[
            Card::Estate,
            Card::Gold,
            Card::Curse,
            Card::Silver,
            Card::Copper,
            Card::Duchy,
        ],
    );
    g.start();
    to_buy(&mut g);
    play(&mut g, Card::Sextant);
    choose_kinds(&mut g, &[Card::Estate, Card::Curse]);
    choose_kinds(&mut g, &[Card::Silver, Card::Gold, Card::Copper]);
    assert_eq!(
        deck_kinds(&g, P0),
        [Card::Silver, Card::Gold, Card::Copper, Card::Duchy]
    );
    assert_eq!(discard_kinds(&g, P0), [Card::Estate, Card::Curse]);
}

#[test]
fn shield_blocks_every_attack_of_a_turn_and_stays_in_hand() {
    let mut g = setup(&[Card::Militia, Card::Siren, Card::SackOfLoot]);
    hand(&mut g, P0, &[Card::Militia, Card::Siren, Card::Village]);
    hand(
        &mut g,
        P1,
        &[
            Card::Shield,
            Card::Copper,
            Card::Copper,
            Card::Copper,
            Card::Copper,
        ],
    );
    g.start();
    play(&mut g, Card::Village);
    play(&mut g, Card::Militia);
    assert_eq!(g.pending().unwrap().player, P1);
    yes_trigger(&mut g);
    assert_eq!(g.hand(P1).len(), 5);
    play(&mut g, Card::Siren);
    yes_trigger(&mut g);
    assert_eq!(gain_count(&g, P1, Card::Curse), 0);
    assert!(hand_kinds(&g, P1).contains(&Card::Shield));
}

#[test]
fn moat_blocks_a_sword_which_otherwise_makes_others_discard_down_to_four() {
    let mut g = setup(&[Card::Moat, Card::SackOfLoot]);
    hand(&mut g, P0, &[Card::Sword, Card::Sword]);
    hand(
        &mut g,
        P1,
        &[
            Card::Moat,
            Card::Copper,
            Card::Copper,
            Card::Copper,
            Card::Copper,
            Card::Copper,
        ],
    );
    g.start();
    to_buy(&mut g);
    play(&mut g, Card::Sword);
    yes_trigger(&mut g);
    assert_eq!((g.turn().coins, g.turn().buys), (3, 2));
    assert_eq!(g.hand(P1).len(), 6);
    play(&mut g, Card::Sword);
    respond(&mut g, Response::option(Opt::No));
    choose_kinds(&mut g, &[Card::Copper, Card::Copper]);
    assert_eq!(g.hand(P1).len(), 4);
}

#[test]
fn spell_scroll_gains_a_card_cheaper_than_its_cost_now_and_may_play_it() {
    let mut g = setup(&[
        Card::SackOfLoot,
        Card::Bridge,
        Card::Smithy,
        Card::KingsCache,
    ]);
    hand(&mut g, P0, &[Card::Bridge, Card::SpellScroll]);
    deck(&mut g, P0, &[Card::Copper; 10]);
    g.start();
    play(&mut g, Card::Bridge);
    to_buy(&mut g);
    play(&mut g, Card::SpellScroll);
    // It costs $6 now, as does King's Cache: that isn't cheaper.
    let DecisionKind::Pile { piles, .. } = &g.pending().unwrap().kind else {
        panic!()
    };
    assert!(piles.iter().any(|&p| g.pile(p).card == Card::Gold));
    assert!(!piles.iter().any(|&p| g.pile(p).card == Card::KingsCache));
    choose_pile(&mut g, Card::Smithy);
    yes(&mut g);
    assert!(in_play_kinds(&g, P0).contains(&Card::Smithy));
    assert_eq!(g.hand(P0).len(), 3);
    assert!(trash_kinds(&g).contains(&Card::SpellScroll));
}

#[test]
fn a_siren_gained_with_spell_scroll_trashes_itself_and_is_not_played() {
    let mut g = setup(&[Card::SackOfLoot, Card::Siren]);
    hand(&mut g, P0, &[Card::SpellScroll]);
    g.start();
    play(&mut g, Card::SpellScroll);
    choose_pile(&mut g, Card::Siren);
    assert!(trash_kinds(&g).contains(&Card::Siren));
    assert_eq!(phase(&g), Some(Phase::Buy));
}

#[test]
fn staff_pays_three_and_a_buy_and_may_play_an_action() {
    let mut g = setup(&[Card::SackOfLoot, Card::Smithy]);
    hand(&mut g, P0, &[Card::Staff, Card::Smithy]);
    deck(&mut g, P0, &[Card::Copper; 10]);
    g.start();
    to_buy(&mut g);
    play(&mut g, Card::Staff);
    choose_kinds(&mut g, &[Card::Smithy]);
    assert_eq!((g.turn().coins, g.turn().buys), (3, 2));
    assert_eq!(g.hand(P0).len(), 3);
}

/// A game with Events `events`, P0 in the Buy phase with `coins` and `buys`.
fn event_game(
    kingdom: &[Card],
    events: &[Card],
    hand_cards: &[Card],
    coins: u32,
    buys: u32,
) -> Game {
    let mut g = setup_l(kingdom, events);
    hand(&mut g, P0, hand_cards);
    deck(&mut g, P0, &[Card::Copper; 30]);
    g.start();
    to_buy(&mut g);
    money(&mut g, coins, buys);
    g
}

#[test]
fn bury_puts_a_card_from_the_discard_pile_on_the_bottom_of_the_deck() {
    let mut g = event_game(&[Card::Village], &[Card::Bury], &[], 1, 1);
    discard(&mut g, P0, &[Card::Gold, Card::Estate]);
    money(&mut g, 0, 1);
    buy_event(&mut g, Card::Bury);
    assert_eq!(g.turn().buys, 1);
    choose_kinds(&mut g, &[Card::Gold]);
    assert_eq!(
        g.player(P0).deck.first().map(|&c| g.card(c)),
        Some(Card::Gold)
    );
}

#[test]
fn avoid_keeps_up_to_three_cards_per_avoid_out_of_the_next_shuffle() {
    let mut g = setup_l(&[Card::Village], &[Card::Avoid]);
    hand(&mut g, P0, &[]);
    deck(&mut g, P0, &[]);
    discard(
        &mut g,
        P0,
        &[
            Card::Estate,
            Card::Estate,
            Card::Curse,
            Card::Curse,
            Card::Copper,
            Card::Gold,
            Card::Gold,
        ],
    );
    g.start();
    money(&mut g, 4, 3);
    to_buy(&mut g);
    buy_event(&mut g, Card::Avoid);
    buy_event(&mut g, Card::Avoid);
    assert_eq!(g.turn().buys, 3);
    g.stack.push(crate::frame::Frame::draw(P0, 3));
    g.pending = None;
    g.run(crate::frame::Input::Start);
    resolve_triggers(&mut g);
    choose_kinds(&mut g, &[Card::Estate, Card::Estate, Card::Curse]);
    resolve_triggers(&mut g);
    choose_kinds(&mut g, &[Card::Curse]);
    resolve_triggers(&mut g);
    // Four cards stayed in the discard pile; the other three were shuffled and drawn.
    let mut drawn = hand_kinds(&g, P0);
    drawn.sort();
    assert_eq!(drawn, [Card::Copper, Card::Gold, Card::Gold]);
    assert_eq!(discard_kinds(&g, P0).len(), 4);
}

#[test]
fn avoid_leaving_too_few_cards_draws_what_it_can_without_shuffling_again() {
    let mut g = setup_l(&[Card::Village], &[Card::Avoid]);
    hand(&mut g, P0, &[]);
    deck(&mut g, P0, &[]);
    discard(&mut g, P0, &[Card::Estate, Card::Copper, Card::Gold]);
    g.start();
    money(&mut g, 2, 1);
    to_buy(&mut g);
    buy_event(&mut g, Card::Avoid);
    g.stack.push(crate::frame::Frame::draw(P0, 3));
    g.pending = None;
    g.run(crate::frame::Input::Start);
    resolve_triggers(&mut g);
    choose_kinds(&mut g, &[Card::Estate, Card::Copper]);
    assert_eq!(hand_kinds(&g, P0), [Card::Gold]);
    assert_eq!(discard_kinds(&g, P0), [Card::Estate, Card::Copper]);
}

#[test]
fn deliver_sets_gained_cards_aside_until_after_drawing_and_twice_does_nothing_more() {
    let mut g = event_game(&[Card::Village], &[Card::Deliver], &[], 7, 3);
    buy_event(&mut g, Card::Deliver);
    buy_event(&mut g, Card::Deliver);
    assert_eq!(g.turn().buys, 3);
    buy(&mut g, Card::Silver);
    assert_eq!(aside_kinds(&g, P0), [Card::Silver]);
    buy(&mut g, Card::Copper);
    buy(&mut g, Card::Copper);
    assert_eq!(g.turn().player, P1);
    let mut h = hand_kinds(&g, P0);
    h.sort();
    let mut expected = vec![Card::Copper; 7];
    expected.push(Card::Silver);
    assert_eq!(h, expected);
}

#[test]
fn peril_trashes_an_action_from_hand_for_a_loot() {
    let mut g = event_game(&[Card::Village], &[Card::Peril], &[Card::Village], 2, 2);
    buy_event(&mut g, Card::Peril);
    choose_kinds(&mut g, &[Card::Village]);
    assert_eq!(loot_gained(&g, P0), 1);
}

/// A Fortress is trashed, then returns to the hand.
#[test]
fn peril_trashing_a_fortress_gains_a_loot() {
    let mut g = event_game(&[Card::Fortress], &[Card::Peril], &[Card::Fortress], 2, 2);
    buy_event(&mut g, Card::Peril);
    choose_kinds(&mut g, &[Card::Fortress]);
    assert_eq!(loot_gained(&g, P0), 1);
    assert_eq!(hand_kinds(&g, P0), [Card::Fortress]);
}

#[test]
fn rush_plays_the_next_action_gained_once_even_if_bought_twice() {
    let mut g = event_game(&[Card::Smithy], &[Card::Rush], &[], 12, 4);
    buy_event(&mut g, Card::Rush);
    buy_event(&mut g, Card::Rush);
    buy(&mut g, Card::Smithy);
    assert_eq!(in_play_kinds(&g, P0), [Card::Smithy]);
    assert_eq!(g.hand(P0).len(), 3);
    buy(&mut g, Card::Smithy);
    assert_eq!(in_play_kinds(&g, P1), []);
    assert_eq!(gain_count(&g, P0, Card::Smithy), 2);
    assert_eq!(g.count_owned(P0, Card::Smithy), 2);
}

#[test]
fn foray_gains_a_loot_for_three_different_names_only() {
    let mut g = event_game(
        &[Card::Village],
        &[Card::Foray],
        &[Card::Estate, Card::Copper, Card::Curse, Card::Copper],
        6,
        2,
    );
    buy_event(&mut g, Card::Foray);
    choose_kinds(&mut g, &[Card::Estate, Card::Copper, Card::Curse]);
    assert_eq!(loot_gained(&g, P0), 1);

    let mut g = event_game(
        &[Card::Village],
        &[Card::Foray],
        &[Card::Estate, Card::Copper],
        3,
        1,
    );
    buy_event(&mut g, Card::Foray);
    assert_eq!(loot_gained(&g, P0), 0);
    assert_eq!(discard_kinds(&g, P0).len(), 2);
}

#[test]
fn launch_returns_to_the_action_phase_once_per_turn() {
    let mut g = setup_l(&[Card::Village], &[Card::Launch]);
    hand(&mut g, P0, &[Card::Copper, Card::Copper, Card::Copper]);
    deck(&mut g, P0, &[Card::Village, Card::Copper, Card::Copper]);
    g.start();
    to_buy(&mut g);
    play(&mut g, Card::Copper);
    play(&mut g, Card::Copper);
    play(&mut g, Card::Copper);
    buy_event(&mut g, Card::Launch);
    assert_eq!(phase(&g), Some(Phase::Action));
    assert_eq!(hand_kinds(&g, P0), [Card::Village]);
    assert_eq!((g.turn().actions, g.turn().buys), (2, 1));
    play(&mut g, Card::Village);
    to_buy(&mut g);
    // Treasures can be played again, but Launch can't be bought again.
    let DecisionKind::Turn { moves, .. } = &g.pending().unwrap().kind else {
        panic!()
    };
    assert!(!moves.contains(&Move::BuyLandscape(Card::Launch)));
    assert!(moves.iter().any(|m| matches!(m, Move::Play(_))));
}

#[test]
fn mirror_gains_a_copy_of_the_next_action_per_mirror() {
    let mut g = event_game(&[Card::Village], &[Card::Mirror], &[], 9, 3);
    buy_event(&mut g, Card::Mirror);
    buy_event(&mut g, Card::Mirror);
    buy(&mut g, Card::Village);
    assert_eq!(gain_count(&g, P0, Card::Village), 3);
}

#[test]
fn prepare_sets_the_hand_aside_and_plays_its_actions_and_treasures_next_turn() {
    let mut g = setup_l(&[Card::Village, Card::Smithy], &[Card::Prepare]);
    hand(&mut g, P0, &[Card::Smithy, Card::Gold, Card::Estate]);
    deck(&mut g, P0, &[Card::Copper; 30]);
    g.start();
    money(&mut g, 3, 1);
    to_buy(&mut g);
    buy_event(&mut g, Card::Prepare);
    assert_eq!(
        aside_kinds(&g, P0),
        [Card::Smithy, Card::Gold, Card::Estate]
    );
    next_turn(&mut g);
    choose_kinds(&mut g, &[Card::Smithy]);
    assert_eq!(in_play_kinds(&g, P0), [Card::Smithy, Card::Gold]);
    assert_eq!(g.hand(P0).len(), 8);
    assert_eq!(g.turn().coins, 3);
    assert!(discard_kinds(&g, P0).contains(&Card::Estate));
    // Playing the Smithy used no Action.
    assert_eq!(g.turn().actions, 1);
}

#[test]
fn scrounge_trashes_a_card_or_gains_an_estate_from_the_trash_and_a_card_up_to_five() {
    let mut g = event_game(
        &[Card::Village],
        &[Card::Scrounge],
        &[Card::Curse, Card::Copper],
        6,
        2,
    );
    g.trash.clear();
    buy_event(&mut g, Card::Scrounge);
    mode(&mut g, 0);
    choose_kinds(&mut g, &[Card::Curse]);
    assert_eq!(trash_kinds(&g), [Card::Curse]);
    // No Estate in the trash: nothing.
    buy_event(&mut g, Card::Scrounge);
    mode(&mut g, 1);
    assert_eq!(g.turn().player, P1);

    let mut g = event_game(&[Card::Village], &[Card::Scrounge], &[], 3, 1);
    g.create(Card::Estate, Zone::Trash);
    buy_event(&mut g, Card::Scrounge);
    mode(&mut g, 1);
    choose_pile(&mut g, Card::Duchy);
    assert_eq!(gain_count(&g, P0, Card::Estate), 1);
    assert_eq!(gain_count(&g, P0, Card::Duchy), 1);
}

#[test]
fn journey_keeps_cards_in_play_and_takes_an_extra_turn_but_not_a_third() {
    let mut g = setup_l(&[Card::Village], &[Card::Journey]);
    hand(&mut g, P0, &[Card::Village, Card::Copper]);
    deck(&mut g, P0, &[Card::Copper; 30]);
    g.start();
    play(&mut g, Card::Village);
    to_buy(&mut g);
    play(&mut g, Card::Copper);
    money(&mut g, 4, 1);
    buy_event(&mut g, Card::Journey);
    // The extra turn starts with the Village and Copper still in play.
    assert_eq!(g.turn().player, P0);
    assert_eq!(g.turn().extra, Some(Card::Journey));
    assert_eq!(in_play_kinds(&g, P0), [Card::Village, Card::Copper]);
    assert_eq!(g.hand(P0).len(), 5);
    to_buy(&mut g);
    money(&mut g, 4, 1);
    buy_event(&mut g, Card::Journey);
    // No third turn in a row; the cards still stay in play, until the next Clean-up.
    assert_eq!(g.turn().player, P1);
    assert_eq!(in_play_kinds(&g, P0), [Card::Village, Card::Copper]);
    next_turn(&mut g);
    assert_eq!(g.turn().player, P0);
    assert!(g.in_play(P0).is_empty());
}

#[test]
fn maelstrom_trashes_three_and_others_with_five_or_more_trash_one() {
    let mut g = setup_l(&[Card::Village], &[Card::Maelstrom]);
    hand(
        &mut g,
        P0,
        &[Card::Estate, Card::Estate, Card::Curse, Card::Copper],
    );
    hand(
        &mut g,
        P1,
        &[
            Card::Copper,
            Card::Copper,
            Card::Copper,
            Card::Copper,
            Card::Estate,
        ],
    );
    g.start();
    g.trash.clear();
    money(&mut g, 4, 1);
    to_buy(&mut g);
    buy_event(&mut g, Card::Maelstrom);
    choose_kinds(&mut g, &[Card::Estate, Card::Estate, Card::Curse]);
    assert_eq!(g.pending().unwrap().player, P1);
    choose_kinds(&mut g, &[Card::Estate]);
    assert_eq!(trash_kinds(&g).len(), 4);
}

#[test]
fn looting_gains_a_loot() {
    let mut g = event_game(&[Card::Village], &[Card::Looting], &[], 6, 1);
    buy_event(&mut g, Card::Looting);
    assert_eq!(loot_gained(&g, P0), 1);
}

#[test]
fn invasion_plays_an_attack_gains_a_duchy_an_action_onto_the_deck_and_a_loot_to_play() {
    let mut g = event_game(
        &[Card::Militia, Card::Smithy],
        &[Card::Invasion],
        &[Card::Militia],
        10,
        2,
    );
    hand(&mut g, P1, &[Card::Copper; 3]);
    money(&mut g, 0, 2);
    loot_top(&mut g, Card::Hammer);
    buy_event(&mut g, Card::Invasion);
    choose_kinds(&mut g, &[Card::Militia]);
    assert_eq!(g.turn().coins, 2);
    choose_pile(&mut g, Card::Smithy);
    assert_eq!(deck_kinds(&g, P0)[0], Card::Smithy);
    assert_eq!(gain_count(&g, P0, Card::Duchy), 1);
    // The Hammer was played: +$3 and a gain.
    choose_pile(&mut g, Card::Silver);
    assert_eq!(g.turn().coins, 5);
    assert!(in_play_kinds(&g, P0).contains(&Card::Hammer));
}

#[test]
fn prosper_gains_a_loot_and_differently_named_treasures() {
    let mut g = event_game(&[Card::Village], &[Card::Prosper], &[], 10, 1);
    buy_event(&mut g, Card::Prosper);
    assert_eq!(loot_gained(&g, P0), 1);
    choose_pile(&mut g, Card::Gold);
    choose_pile(&mut g, Card::Silver);
    let DecisionKind::Pile { piles, optional } = &g.pending().unwrap().kind else {
        panic!()
    };
    assert!(*optional);
    let names: Vec<Card> = piles.iter().map(|&p| g.pile(p).card).collect();
    assert_eq!(names, [Card::Copper]);
    respond(&mut g, Response::Pile(None));
    assert_eq!(gain_count(&g, P0, Card::Gold), 1);
    assert_eq!(gain_count(&g, P0, Card::Silver), 1);
}

#[test]
fn a_trait_goes_on_a_random_action_or_treasure_kingdom_pile() {
    for seed in 0..20 {
        let g = setup_with(GameConfig {
            landscapes: vec![Card::Rich, Card::Cheap],
            ..GameConfig::new(2, vec![Card::Gardens, Card::Village, Card::Cage], seed)
        });
        let traited: Vec<(Card, Vec<Card>)> = g
            .piles()
            .iter()
            .filter(|p| !p.traits.is_empty())
            .map(|p| (p.card, p.traits.clone()))
            .collect();
        assert_eq!(traited.len(), 2, "{traited:?}");
        assert!(
            traited
                .iter()
                .all(|(c, t)| matches!(c, Card::Village | Card::Cage) && t.len() == 1)
        );
    }
}

#[test]
fn cheap_cards_cost_one_less_but_not_below_zero() {
    let mut g = setup(&[Card::Village, Card::Cage]);
    set_trait(&mut g, Card::Cheap, Card::Village);
    assert_eq!(g.cost(Card::Village), Cost::coins(2));
    set_trait(&mut g, Card::Cheap, Card::Cage);
    assert_eq!(g.cost(Card::Cage), Cost::coins(1));
    assert_eq!(g.cost(Card::Village), Cost::coins(3));
}

#[test]
fn cursed_gives_the_card_then_a_loot_then_a_curse() {
    let mut g = setup(&[Card::Village]);
    set_trait(&mut g, Card::Cursed, Card::Village);
    add_kingdom_piles(&mut g, &[]);
    hand(&mut g, P0, &[]);
    g.start();
    money(&mut g, 3, 1);
    to_buy(&mut g);
    buy(&mut g, Card::Village);
    let gains: Vec<Card> = g
        .log()
        .iter()
        .filter_map(|e| match e {
            Event::Gained {
                player: P0, card, ..
            } => Some(*card),
            _ => None,
        })
        .collect();
    assert_eq!(gains.len(), 3);
    assert_eq!(gains[0], Card::Village);
    assert!(gains[1].is(Types::LOOT));
    assert_eq!(gains[2], Card::Curse);
}

#[test]
fn fated_cards_may_go_on_top_or_bottom_when_shuffling() {
    let mut g = setup(&[Card::Village, Card::Smithy]);
    set_trait(&mut g, Card::Fated, Card::Village);
    hand(&mut g, P0, &[Card::Smithy]);
    deck(&mut g, P0, &[]);
    discard(
        &mut g,
        P0,
        &[
            Card::Village,
            Card::Village,
            Card::Copper,
            Card::Copper,
            Card::Estate,
            Card::Estate,
        ],
    );
    g.start();
    play(&mut g, Card::Smithy);
    resolve_triggers(&mut g);
    let villages: Vec<CardId> = candidates(&g);
    choose(&mut g, &villages[..1]);
    choose(&mut g, &villages[1..]);
    // One Village on top (drawn first), one at the bottom.
    assert_eq!(hand_kinds(&g, P0)[0], Card::Village);
    assert_eq!(
        g.player(P0).deck.first().map(|&c| g.card(c)),
        Some(Card::Village)
    );
}

#[test]
fn fated_cards_put_under_can_go_among_shadow_cards() {
    for (order, bottom) in [
        ([Card::Ninja, Card::Village], Card::Village),
        ([Card::Village, Card::Ninja], Card::Ninja),
    ] {
        let mut g = setup(&[Card::Village, Card::Smithy, Card::Ninja]);
        set_trait(&mut g, Card::Fated, Card::Village);
        hand(&mut g, P0, &[Card::Smithy]);
        deck(&mut g, P0, &[]);
        discard(
            &mut g,
            P0,
            &[
                Card::Village,
                Card::Ninja,
                Card::Copper,
                Card::Copper,
                Card::Copper,
                Card::Copper,
            ],
        );
        g.start();
        play(&mut g, Card::Smithy);
        resolve_triggers(&mut g);
        choose(&mut g, &[]);
        choose_kinds(&mut g, &[Card::Village]);
        choose_kinds(&mut g, &order);
        let deck = deck_kinds(&g, P0);
        assert_eq!(deck.len(), 3);
        assert_eq!(deck[2], bottom);
        assert_eq!(deck[1], order[0]);
    }
}

#[test]
fn fawning_gains_a_fawning_card_with_each_province() {
    let mut g = setup(&[Card::Village]);
    set_trait(&mut g, Card::Fawning, Card::Village);
    hand(&mut g, P0, &[]);
    g.start();
    money(&mut g, 8, 1);
    to_buy(&mut g);
    buy(&mut g, Card::Province);
    assert_eq!(gain_count(&g, P0, Card::Village), 1);
}

#[test]
fn friendly_discards_one_friendly_card_at_the_start_of_clean_up_to_gain_another() {
    let mut g = setup(&[Card::Village]);
    set_trait(&mut g, Card::Friendly, Card::Village);
    hand(&mut g, P0, &[Card::Village, Card::Village]);
    deck(&mut g, P0, &[Card::Copper; 10]);
    g.start();
    to_buy(&mut g);
    end_phase(&mut g);
    // Night is skipped; at the start of Clean-up:
    yes_trigger(&mut g);
    assert_eq!(gain_count(&g, P0, Card::Village), 1);
    assert_eq!(g.turn().player, P1);
}

#[test]
fn hasty_cards_are_set_aside_when_gained_and_played_next_turn() {
    let mut g = setup(&[Card::Smithy]);
    set_trait(&mut g, Card::Hasty, Card::Smithy);
    hand(&mut g, P0, &[]);
    deck(&mut g, P0, &[Card::Copper; 20]);
    g.start();
    money(&mut g, 4, 1);
    to_buy(&mut g);
    buy(&mut g, Card::Smithy);
    assert_eq!(aside_kinds(&g, P0), [Card::Smithy]);
    next_turn(&mut g);
    assert_eq!(in_play_kinds(&g, P0), [Card::Smithy]);
    assert_eq!(g.hand(P0).len(), 8);
    assert_eq!(g.turn().actions, 1);
}

#[test]
fn inherited_replaces_an_estate_or_the_starting_card_each_player_chooses() {
    let mut g = Game::new(GameConfig {
        landscapes: vec![Card::Inherited],
        colonies: Some(false),
        shelters: Some(false),
        ..GameConfig::new(2, vec![Card::Village], 1)
    })
    .unwrap();
    // P0 keeps the default (an Estate); P1 replaces a Copper, which goes back to its pile.
    assert_eq!(g.pending().unwrap().player, P0);
    assert!(g.hand(P0).is_empty());
    pick(&mut g, Card::Estate);
    assert_eq!(g.pending().unwrap().player, P1);
    pick(&mut g, Card::Copper);
    for (p, coppers, estates) in [(P0, 7, 2), (P1, 6, 3)] {
        assert_eq!(g.count_owned(p, Card::Village), 1);
        assert_eq!(g.count_owned(p, Card::Copper), coppers);
        assert_eq!(g.count_owned(p, Card::Estate), estates);
        assert_eq!(g.hand(p).len(), 5);
    }
    let village = g.pile_of(Card::Village).unwrap();
    assert_eq!(g.pile(village).cards.len(), 8);
    let copper = g.pile_of(Card::Copper).unwrap();
    assert_eq!(g.pile(copper).cards.len(), 60 - 14 + 1);
    assert_eq!(gain_count(&g, P0, Card::Village), 0);
    check_zones(&g);
}

#[test]
fn inspiring_plays_an_action_without_a_copy_in_play_after_it() {
    let mut g = setup(&[Card::Village, Card::Smithy]);
    set_trait(&mut g, Card::Inspiring, Card::Village);
    hand(&mut g, P0, &[Card::Village, Card::Village, Card::Smithy]);
    deck(&mut g, P0, &[Card::Copper; 10]);
    g.start();
    play(&mut g, Card::Village);
    yes_trigger(&mut g);
    // Only the Smithy: a Village is already in play.
    assert_eq!(in_play_kinds(&g, P0), [Card::Village, Card::Smithy]);
    assert_eq!(g.turn().actions, 2);
}

#[test]
fn nearby_and_rich_give_a_buy_and_a_silver_on_gaining() {
    let mut g = setup(&[Card::Village, Card::Smithy]);
    set_trait(&mut g, Card::Nearby, Card::Village);
    set_trait(&mut g, Card::Rich, Card::Smithy);
    hand(&mut g, P0, &[]);
    g.start();
    money(&mut g, 7, 1);
    to_buy(&mut g);
    buy(&mut g, Card::Village);
    assert_eq!(g.turn().buys, 1);
    buy(&mut g, Card::Smithy);
    assert_eq!(gain_count(&g, P0, Card::Silver), 1);
}

#[test]
fn patient_cards_are_set_aside_at_clean_up_and_played_next_turn() {
    let mut g = setup(&[Card::Smithy]);
    set_trait(&mut g, Card::Patient, Card::Smithy);
    hand(&mut g, P0, &[Card::Smithy, Card::Smithy, Card::Copper]);
    deck(&mut g, P0, &[Card::Copper; 30]);
    g.start();
    g.turn.actions = 0;
    end_phase(&mut g);
    to_buy(&mut g);
    end_phase(&mut g);
    yes_trigger(&mut g);
    choose_kinds(&mut g, &[Card::Smithy, Card::Smithy]);
    assert_eq!(aside_kinds(&g, P0), [Card::Smithy, Card::Smithy]);
    next_turn(&mut g);
    assert_eq!(in_play_kinds(&g, P0), [Card::Smithy, Card::Smithy]);
    assert_eq!(g.hand(P0).len(), 11);
}

#[test]
fn pious_may_trash_a_card_when_one_is_gained() {
    let mut g = setup(&[Card::Village]);
    set_trait(&mut g, Card::Pious, Card::Village);
    hand(&mut g, P0, &[Card::Estate, Card::Curse]);
    g.start();
    money(&mut g, 3, 1);
    to_buy(&mut g);
    buy(&mut g, Card::Village);
    yes_trigger(&mut g);
    choose_kinds(&mut g, &[Card::Curse]);
    assert_eq!(trash_kinds(&g).last(), Some(&Card::Curse));
}

#[test]
fn reckless_cards_follow_their_instructions_twice_and_return_to_their_pile() {
    let mut g = setup(&[Card::Smithy, Card::Militia, Card::Moat]);
    set_trait(&mut g, Card::Reckless, Card::Smithy);
    hand(&mut g, P0, &[Card::Smithy]);
    deck(&mut g, P0, &[Card::Copper; 20]);
    g.start();
    play(&mut g, Card::Smithy);
    assert_eq!(g.hand(P0).len(), 6);
    assert_eq!(
        g.log()
            .iter()
            .filter(|e| matches!(e, Event::Played { .. }))
            .count(),
        1
    );
    let pile = g.pile_of(Card::Smithy).unwrap();
    next_turn(&mut g);
    assert_eq!(g.pile(pile).cards.len(), 11);
    assert_eq!(g.count_owned(P0, Card::Smithy), 0);
}

#[test]
fn one_moat_reveal_covers_both_runs_of_a_reckless_attack() {
    let mut g = setup(&[Card::Militia, Card::Moat]);
    set_trait(&mut g, Card::Reckless, Card::Militia);
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
    yes_trigger(&mut g);
    assert_eq!(g.turn().coins, 4);
    assert_eq!(g.hand(P1).len(), 5);
}

#[test]
fn shy_cards_may_be_discarded_at_the_start_of_the_turn_for_two_cards() {
    let mut g = setup(&[Card::Village]);
    set_trait(&mut g, Card::Shy, Card::Village);
    hand(&mut g, P0, &[]);
    deck(
        &mut g,
        P0,
        &[
            Card::Village,
            Card::Village,
            Card::Copper,
            Card::Copper,
            Card::Copper,
        ]
        .iter()
        .chain(&[Card::Silver; 10])
        .copied()
        .collect::<Vec<_>>(),
    );
    g.start();
    next_turn(&mut g);
    next_turn(&mut g);
    yes_trigger(&mut g);
    // One Shy card only.
    assert_eq!(g.hand(P0).len(), 6);
    assert_eq!(discard_kinds(&g, P0), [Card::Village]);
}

#[test]
fn tireless_cards_go_onto_the_deck_after_the_new_hand_is_drawn() {
    let mut g = setup(&[Card::Village]);
    set_trait(&mut g, Card::Tireless, Card::Village);
    hand(&mut g, P0, &[Card::Village]);
    deck(&mut g, P0, &[Card::Copper; 20]);
    g.start();
    play(&mut g, Card::Village);
    next_turn(&mut g);
    assert_eq!(deck_kinds(&g, P0)[0], Card::Village);
    assert_eq!(g.hand(P0).len(), 5);
    assert!(!hand_kinds(&g, P0).contains(&Card::Village));
}

#[test]
fn a_trait_still_applies_after_its_pile_is_empty() {
    let mut g = setup(&[Card::Smithy]);
    set_trait(&mut g, Card::Reckless, Card::Smithy);
    let pile = g.pile_of(Card::Smithy).unwrap();
    for c in g.pile(pile).cards.clone() {
        g.move_card(c, Zone::Trash, crate::game::Pos::Top);
    }
    hand(&mut g, P0, &[Card::Smithy]);
    deck(&mut g, P0, &[Card::Copper; 20]);
    g.start();
    play(&mut g, Card::Smithy);
    assert_eq!(g.hand(P0).len(), 6);
}

#[test]
fn a_gold_gained_from_a_tunnel_discarded_to_a_cutthroat_does_not_wake_it() {
    let mut g = setup(&[Card::Cutthroat, Card::Tunnel]);
    hand(&mut g, P0, &[Card::Cutthroat]);
    hand(
        &mut g,
        P1,
        &[
            Card::Tunnel,
            Card::Copper,
            Card::Copper,
            Card::Copper,
            Card::Copper,
        ],
    );
    g.start();
    play(&mut g, Card::Cutthroat);
    choose_kinds(&mut g, &[Card::Tunnel, Card::Copper]);
    resolve_triggers(&mut g);
    assert_eq!(gain_count(&g, P1, Card::Gold), 1);
    assert_eq!(loot_gained(&g, P0), 0);
    assert!(g.lasting().iter().any(|l| l.card == Card::Cutthroat));
}

#[test]
fn a_search_played_by_the_gain_that_empties_a_pile_waits_for_the_next() {
    let mut g = setup(&[Card::Search, Card::Gondola]);
    hand(&mut g, P0, &[Card::Search]);
    deck(&mut g, P0, &[Card::Copper; 10]);
    last_one(&mut g, Card::Gondola);
    g.start();
    money(&mut g, 4, 2);
    to_buy(&mut g);
    buy(&mut g, Card::Gondola);
    yes_trigger(&mut g);
    assert_eq!(in_play_kinds(&g, P0), [Card::Search]);
    assert_eq!(loot_gained(&g, P0), 0);
    assert!(g.lasting().iter().any(|l| l.card == Card::Search));
}

#[test]
fn a_reckless_duration_does_its_next_turn_part_twice() {
    let mut g = setup(&[Card::Longship]);
    set_trait(&mut g, Card::Reckless, Card::Longship);
    hand(&mut g, P0, &[Card::Longship]);
    deck(&mut g, P0, &[Card::Copper; 30]);
    g.start();
    play(&mut g, Card::Longship);
    assert_eq!(g.turn().actions, 4);
    next_turn(&mut g);
    assert_eq!(in_play_kinds(&g, P0), [Card::Longship]);
    next_turn(&mut g);
    resolve_triggers(&mut g);
    assert_eq!(g.hand(P0).len(), 9);
    next_turn(&mut g);
    assert_eq!(g.count_owned(P0, Card::Longship), 0);
}

#[test]
fn kings_cache_on_a_cage_holds_three_sets_and_returns_them_all() {
    let mut g = setup(&[Card::Cage, Card::KingsCache]);
    hand(
        &mut g,
        P0,
        &[
            Card::KingsCache,
            Card::Cage,
            Card::Estate,
            Card::Curse,
            Card::Duchy,
        ],
    );
    deck(&mut g, P0, &[Card::Copper; 30]);
    g.start();
    to_buy(&mut g);
    play(&mut g, Card::KingsCache);
    choose_kinds(&mut g, &[Card::Cage]);
    choose_kinds(&mut g, &[Card::Estate]);
    choose_kinds(&mut g, &[Card::Curse]);
    choose_kinds(&mut g, &[Card::Duchy]);
    next_turn(&mut g);
    next_turn(&mut g);
    assert_eq!(in_play_kinds(&g, P0), [Card::KingsCache, Card::Cage]);
    money(&mut g, 2, 1);
    to_buy(&mut g);
    buy(&mut g, Card::Estate);
    resolve_triggers(&mut g);
    resolve_triggers(&mut g);
    // Trashed once; all three cards come back; King's Cache is discarded that turn.
    assert_eq!(
        trash_kinds(&g).iter().filter(|&&c| c == Card::Cage).count(),
        1
    );
    assert_eq!(g.hand(P0).len(), 5 + 3);
    assert!(g.in_play(P0).is_empty());
}

#[test]
fn a_siren_gained_with_a_quartermaster_still_trashes_itself() {
    let mut g = setup(&[Card::Quartermaster, Card::Siren]);
    hand(&mut g, P0, &[Card::Quartermaster]);
    deck(&mut g, P0, &[Card::Copper; 30]);
    g.start();
    play(&mut g, Card::Quartermaster);
    next_turn(&mut g);
    next_turn(&mut g);
    mode(&mut g, 0);
    choose_pile(&mut g, Card::Siren);
    assert!(trash_kinds(&g).contains(&Card::Siren));
    assert!(g.player(P0).set_aside.is_empty());
    assert!(g.lasting().iter().all(|l| l.data.cards.is_empty()));
}

#[test]
fn inherited_with_shelters_replaces_a_hovel_unless_another_card_is_chosen() {
    let mut g = Game::new(GameConfig {
        landscapes: vec![Card::Inherited],
        colonies: Some(false),
        shelters: Some(true),
        ..GameConfig::new(2, vec![Card::Village], 1)
    })
    .unwrap();
    pick(&mut g, Card::Hovel);
    pick(&mut g, Card::Necropolis);
    assert_eq!(g.count_owned(P0, Card::Hovel), 0);
    assert_eq!(g.count_owned(P0, Card::Necropolis), 1);
    assert_eq!(g.count_owned(P1, Card::Hovel), 1);
    assert_eq!(g.count_owned(P1, Card::Necropolis), 0);
    for p in [P0, P1] {
        assert_eq!(g.player(p).owned().count(), 10);
    }
    check_zones(&g);
}

#[test]
fn harbor_village_ignores_the_coin_of_a_plus_coin_token() {
    let mut g = setup(&[Card::HarborVillage, Card::Village]);
    hand(&mut g, P0, &[Card::HarborVillage, Card::Village]);
    deck(&mut g, P0, &[Card::Copper; 10]);
    let village = g.pile_of(Card::Village).unwrap();
    g.players[0].tokens.insert(Token::PlusCoin, Some(village));
    g.piles[village.index()].tokens.push((P0, Token::PlusCoin));
    g.start();
    play(&mut g, Card::HarborVillage);
    play(&mut g, Card::Village);
    assert_eq!(g.turn().coins, 1);
}

#[test]
fn taskmaster_counts_the_cost_a_card_had_as_it_was_gained() {
    // Destrier costs $6, then $5 once it is gained: that doesn't count.
    let mut g = setup(&[Card::Taskmaster, Card::Destrier]);
    hand(&mut g, P0, &[Card::Taskmaster]);
    deck(&mut g, P0, &[Card::Copper; 30]);
    g.start();
    play(&mut g, Card::Taskmaster);
    money(&mut g, 5, 1);
    to_buy(&mut g);
    buy(&mut g, Card::Destrier);
    assert_eq!(gain_count(&g, P0, Card::Destrier), 1);
    assert!(g.in_play(P0).is_empty());
}

#[test]
fn avoid_can_leave_every_shuffled_card_in_the_discard_pile() {
    let mut g = setup_l(&[Card::Village], &[Card::Avoid]);
    hand(&mut g, P0, &[]);
    deck(&mut g, P0, &[]);
    discard(&mut g, P0, &[Card::Estate, Card::Curse]);
    g.start();
    money(&mut g, 2, 1);
    to_buy(&mut g);
    buy_event(&mut g, Card::Avoid);
    g.stack.push(crate::frame::Frame::draw(P0, 2));
    g.pending = None;
    g.run(crate::frame::Input::Start);
    choose_kinds(&mut g, &[Card::Estate, Card::Curse]);
    assert!(g.hand(P0).is_empty());
    assert_eq!(discard_kinds(&g, P0).len(), 2);
}

#[test]
fn search_counts_a_pile_refilled_and_emptied_again() {
    let mut g = setup(&[Card::Search, Card::Village]);
    hand(&mut g, P0, &[Card::Search]);
    deck(&mut g, P0, &[Card::Copper; 20]);
    let village = g.pile_of(Card::Village).unwrap();
    for c in g.pile(village).cards.clone() {
        g.move_card(c, Zone::Trash, crate::game::Pos::Top);
    }
    g.start();
    play(&mut g, Card::Search);
    let back = g
        .trash()
        .iter()
        .copied()
        .find(|&c| g.card(c) == Card::Village)
        .unwrap();
    g.return_to_pile(back);
    money(&mut g, 3, 1);
    to_buy(&mut g);
    buy(&mut g, Card::Village);
    assert_eq!(loot_gained(&g, P0), 1);
}

#[test]
fn flagship_after_band_of_misfits_replays_the_card_it_played() {
    let mut g = setup(&[
        Card::Flagship,
        Card::BandOfMisfits,
        Card::Smithy,
        Card::Village,
    ]);
    hand(
        &mut g,
        P0,
        &[Card::Village, Card::Flagship, Card::BandOfMisfits],
    );
    deck(&mut g, P0, &[Card::Copper; 20]);
    g.start();
    play(&mut g, Card::Village);
    play(&mut g, Card::Flagship);
    play(&mut g, Card::BandOfMisfits);
    choose_pile(&mut g, Card::Smithy);
    resolve_triggers(&mut g);
    // Smithy twice, with no second choice.
    assert_eq!(g.hand(P0).len(), 1 + 6);
}

/// Moves every card of `card`'s pile to the trash.
fn empty_pile(g: &mut Game, card: Card) {
    let pile = g.pile_of(card).expect("pile exists");
    for c in g.pile(pile).cards.clone() {
        g.move_card(c, Zone::Trash, crate::game::Pos::Top);
    }
}

/// Which of the triggers being asked about is `card`'s.
fn trigger_index(g: &Game, card: Card) -> u16 {
    let p = g.pending().expect("a decision is pending").player;
    g.view(p)
        .pending_triggers()
        .iter()
        .position(|t| t.card == card)
        .unwrap_or_else(|| panic!("no {card} trigger pending")) as u16
}

fn play_count(g: &Game, card: Card) -> usize {
    g.log()
        .iter()
        .filter(|e| matches!(e, Event::Played { card: c, .. } if *c == card))
        .count()
}

#[test]
fn prepare_plays_an_encampment_that_sets_itself_aside_only_once() {
    let mut g = setup_l(&[Card::Encampment], &[Card::Prepare]);
    hand(&mut g, P0, &[Card::Encampment]);
    let mut cards = vec![Card::Gold];
    cards.extend([Card::Copper; 30]);
    deck(&mut g, P0, &cards);
    g.start();
    to_buy(&mut g);
    money(&mut g, 3, 1);
    buy_event(&mut g, Card::Prepare);
    next_turn(&mut g);
    assert_eq!(g.turn().player, P0);
    // Encampment, played by Prepare: don't reveal the Gold, so it is set aside again.
    assert_eq!(
        (purpose(&g), g.pending().unwrap().source),
        (Purpose::Reveal, Some(Card::Encampment))
    );
    choose(&mut g, &[]);
    assert_eq!(purpose(&g), Purpose::Turn);
    assert_eq!(play_count(&g, Card::Encampment), 1);
    // It is Encampment's now, not one of the rest Prepare discards.
    assert_eq!(aside_kinds(&g, P0), [Card::Encampment]);
}

#[test]
fn invasion_with_no_duchies_left_still_gains_an_action_and_a_loot() {
    let mut g = setup_l(&[Card::Smithy], &[Card::Invasion]);
    empty_pile(&mut g, Card::Duchy);
    hand(&mut g, P0, &[]);
    deck(&mut g, P0, &[Card::Copper; 30]);
    g.start();
    to_buy(&mut g);
    money(&mut g, 10, 2);
    loot_top(&mut g, Card::Doubloons);
    buy_event(&mut g, Card::Invasion);
    assert_eq!(deck_kinds(&g, P0)[0], Card::Smithy);
    assert_eq!(loot_gained(&g, P0), 1);
    assert!(in_play_kinds(&g, P0).contains(&Card::Doubloons));
}

#[test]
fn cursed_with_an_empty_loot_pile_still_gains_a_curse() {
    let mut g = setup(&[Card::Village]);
    set_trait(&mut g, Card::Cursed, Card::Village);
    add_kingdom_piles(&mut g, &[]);
    empty_pile(&mut g, Card::Loot);
    hand(&mut g, P0, &[]);
    g.start();
    money(&mut g, 3, 1);
    to_buy(&mut g);
    buy(&mut g, Card::Village);
    assert_eq!(gain_count(&g, P0, Card::Village), 1);
    assert_eq!(gain_count(&g, P0, Card::Curse), 1);
}

#[test]
fn prosper_with_an_empty_loot_pile_still_gains_treasures() {
    let mut g = setup_l(&[Card::Village], &[Card::Prosper]);
    empty_pile(&mut g, Card::Loot);
    hand(&mut g, P0, &[]);
    deck(&mut g, P0, &[Card::Copper; 30]);
    g.start();
    to_buy(&mut g);
    money(&mut g, 10, 1);
    buy_event(&mut g, Card::Prosper);
    assert_eq!(purpose(&g), Purpose::Gain);
    choose_pile(&mut g, Card::Gold);
    assert_eq!(gain_count(&g, P0, Card::Gold), 1);
}

#[test]
fn flagship_replays_the_throne_room_played_next_not_the_card_it_plays() {
    let mut g = setup(&[
        Card::Flagship,
        Card::ThroneRoom,
        Card::Smithy,
        Card::Village,
    ]);
    hand(
        &mut g,
        P0,
        &[
            Card::Flagship,
            Card::ThroneRoom,
            Card::Smithy,
            Card::Village,
        ],
    );
    deck(&mut g, P0, &[Card::Copper; 30]);
    g.start();
    g.turn.actions = 2;
    play(&mut g, Card::Flagship);
    play(&mut g, Card::ThroneRoom);
    choose_kinds(&mut g, &[Card::Smithy]);
    // The Smithy finishes first, but the next Action played was the Throne Room.
    assert_eq!(g.pending().unwrap().source, Some(Card::ThroneRoom));
    choose_kinds(&mut g, &[Card::Village]);
    assert_eq!(
        g.turn().played,
        [
            Card::Flagship,
            Card::ThroneRoom,
            Card::Smithy,
            Card::Smithy,
            Card::ThroneRoom,
            Card::Village,
            Card::Village,
        ]
    );
}

#[test]
fn harbor_village_pays_for_coins_the_next_action_gave_and_then_lost() {
    let mut g = setup(&[Card::HarborVillage, Card::PoorHouse]);
    let mut cards = vec![Card::HarborVillage, Card::PoorHouse];
    cards.extend([Card::Copper; 4]);
    hand(&mut g, P0, &cards);
    deck(&mut g, P0, &[Card::Estate; 10]);
    g.start();
    play(&mut g, Card::HarborVillage);
    play(&mut g, Card::PoorHouse);
    // +$4, then -$1 per Treasure in hand.
    assert_eq!(g.turn().coins, 1);
}

#[test]
fn harbor_village_ignores_the_coin_of_a_boon_the_next_action_received() {
    let mut g = setup(&[Card::HarborVillage, Card::Druid]);
    g.ext.nocturne.druid = vec![
        Card::TheForestsGift,
        Card::TheSeasGift,
        Card::TheMountainsGift,
    ];
    hand(&mut g, P0, &[Card::HarborVillage, Card::Druid]);
    deck(&mut g, P0, &[Card::Estate; 10]);
    g.start();
    play(&mut g, Card::HarborVillage);
    play(&mut g, Card::Druid);
    pick(&mut g, Card::TheForestsGift);
    assert_eq!(g.turn().coins, 1);
}

#[test]
fn harbor_village_looks_at_the_next_action_played_not_one_that_action_plays() {
    // Vassal gives +$2 and plays a Village, which finishes first.
    let mut g = setup(&[Card::HarborVillage, Card::Vassal, Card::Village]);
    hand(&mut g, P0, &[Card::HarborVillage, Card::Vassal]);
    let mut cards = vec![Card::Estate, Card::Village];
    cards.extend([Card::Estate; 3]);
    deck(&mut g, P0, &cards);
    g.start();
    play(&mut g, Card::HarborVillage);
    play(&mut g, Card::Vassal);
    yes(&mut g);
    assert_eq!(
        g.turn().played,
        [Card::HarborVillage, Card::Vassal, Card::Village]
    );
    assert_eq!(g.turn().coins, 3);
}

#[test]
fn harbor_village_does_not_pay_for_the_coins_of_a_card_the_next_action_played() {
    // Throne Room gives no +$ itself; the Militia it plays is a later Action.
    let mut g = setup(&[Card::HarborVillage, Card::ThroneRoom, Card::Militia]);
    hand(
        &mut g,
        P0,
        &[Card::HarborVillage, Card::ThroneRoom, Card::Militia],
    );
    deck(&mut g, P0, &[Card::Estate; 10]);
    hand(&mut g, P1, &[Card::Copper; 3]);
    g.start();
    play(&mut g, Card::HarborVillage);
    play(&mut g, Card::ThroneRoom);
    choose_kinds(&mut g, &[Card::Militia]);
    assert_eq!(g.turn().coins, 4);
}

/// P0 buys an Estate; P1 plays a Mapmaker with Trait `t` from their hand in response,
/// and P0's turn ends.
fn off_turn_mapmaker(t: Card) -> (Game, CardId) {
    let mut g = setup(&[Card::Mapmaker]);
    set_trait(&mut g, t, Card::Mapmaker);
    hand(&mut g, P0, &[]);
    deck(&mut g, P0, &[Card::Copper; 20]);
    let mapmaker = hand(&mut g, P1, &[Card::Mapmaker])[0];
    deck(&mut g, P1, &[Card::Copper; 20]);
    g.start();
    money(&mut g, 2, 1);
    to_buy(&mut g);
    buy(&mut g, Card::Estate);
    assert_eq!(g.pending().unwrap().player, P1);
    resolve_triggers(&mut g);
    assert_eq!(g.turn().player, P1);
    (g, mapmaker)
}

#[test]
fn a_reckless_card_played_on_another_players_turn_returns_to_its_pile() {
    let (g, mapmaker) = off_turn_mapmaker(Card::Reckless);
    let pile = g.pile_of(Card::Mapmaker).unwrap();
    assert_eq!(g.zone(mapmaker), Zone::Pile(pile));
}

#[test]
fn a_tireless_card_played_on_another_players_turn_goes_onto_the_deck() {
    let (g, mapmaker) = off_turn_mapmaker(Card::Tireless);
    assert_eq!(g.player(P1).deck.last(), Some(&mapmaker));
}

/// P0 plays a Highwayman with Trait `t`, which discards itself from play at the start
/// of their next turn.
fn highwayman_leaving_play(t: Card) -> (Game, CardId) {
    let mut g = setup(&[Card::Highwayman]);
    set_trait(&mut g, t, Card::Highwayman);
    let highwayman = hand(&mut g, P0, &[Card::Highwayman])[0];
    deck(&mut g, P0, &[Card::Copper; 30]);
    g.start();
    play(&mut g, Card::Highwayman);
    next_turn(&mut g);
    next_turn(&mut g);
    assert_eq!(g.turn().player, P0);
    (g, highwayman)
}

#[test]
fn a_tireless_highwayman_discarded_at_the_start_of_the_turn_goes_onto_the_deck() {
    let (mut g, highwayman) = highwayman_leaving_play(Card::Tireless);
    assert_eq!(g.zone(highwayman), Zone::SetAside(P0));
    assert_eq!(g.hand(P0).len(), 8);
    next_turn(&mut g);
    assert_eq!(g.player(P0).deck.last(), Some(&highwayman));
}

#[test]
fn a_reckless_highwayman_discarded_at_the_start_of_the_turn_returns_to_its_pile() {
    let (g, highwayman) = highwayman_leaving_play(Card::Reckless);
    let pile = g.pile_of(Card::Highwayman).unwrap();
    assert_eq!(g.zone(highwayman), Zone::Pile(pile));
    // Its instructions were followed twice: +3 Cards twice.
    assert_eq!(g.hand(P0).len(), 11);
}

#[test]
fn a_reckless_card_played_as_way_of_the_chameleon_follows_its_instructions_twice() {
    let mut g = setup_l(&[Card::Smithy], &[Card::WayOfTheChameleon]);
    set_trait(&mut g, Card::Reckless, Card::Smithy);
    hand(&mut g, P0, &[Card::Smithy]);
    deck(&mut g, P0, &[Card::Copper; 20]);
    g.start();
    play(&mut g, Card::Smithy);
    pick(&mut g, Card::WayOfTheChameleon);
    assert_eq!((g.turn().coins, g.hand(P0).len()), (6, 0));
}

/// An Omen's +1 Sun is part of its instructions: a Reckless one removes two Sun tokens,
/// also as Way of the Chameleon.
#[test]
fn a_reckless_omen_removes_two_sun_tokens() {
    for chameleon in [false, true] {
        let mut g = setup_l(&[Card::Poet], &[Card::WayOfTheChameleon, Card::GreatLeader]);
        g.prophecy = Some((Card::GreatLeader, 5));
        set_trait(&mut g, Card::Reckless, Card::Poet);
        hand(&mut g, P0, &[Card::Poet]);
        deck(&mut g, P0, &[Card::Estate; 10]);
        g.start();
        play(&mut g, Card::Poet);
        if chameleon {
            pick(&mut g, Card::WayOfTheChameleon);
        } else {
            no(&mut g);
        }
        assert_eq!(g.prophecy, Some((Card::GreatLeader, 3)), "{chameleon}");
    }
}

#[test]
fn rush_and_mirror_apply_to_the_next_action_gained_not_one_gained_because_of_it() {
    for event in [Card::Rush, Card::Mirror] {
        let mut g = setup_l(&[Card::BorderVillage, Card::Village], &[event]);
        hand(&mut g, P0, &[]);
        deck(&mut g, P0, &[Card::Copper; 30]);
        g.start();
        to_buy(&mut g);
        money(&mut g, 9, 2);
        buy_event(&mut g, event);
        buy(&mut g, Card::BorderVillage);
        // Border Village's own ability first: the Village it gains is not the next Action.
        let i = trigger_index(&g, Card::BorderVillage);
        respond(&mut g, Response::option(Opt::Trigger(i)));
        choose_pile(&mut g, Card::Village);
        if event == Card::Rush {
            assert_eq!(g.turn().played, [Card::BorderVillage]);
        } else {
            choose_pile(&mut g, Card::Village);
            assert_eq!(gain_count(&g, P0, Card::BorderVillage), 2);
            assert_eq!(gain_count(&g, P0, Card::Village), 2);
        }
    }
}

#[test]
fn taskmaster_played_while_a_five_is_being_gained_counts_it() {
    // Rush plays the gained Specialist, which plays the Taskmaster twice, all while the
    // Specialist's gain is still being resolved.
    let mut g = setup_l(&[Card::Taskmaster, Card::Specialist], &[Card::Rush]);
    hand(&mut g, P0, &[Card::Taskmaster]);
    deck(&mut g, P0, &[Card::Copper; 30]);
    g.start();
    to_buy(&mut g);
    money(&mut g, 7, 2);
    buy_event(&mut g, Card::Rush);
    buy(&mut g, Card::Specialist);
    choose_kinds(&mut g, &[Card::Taskmaster]);
    mode(&mut g, 0);
    assert_eq!(
        g.turn().played,
        [Card::Specialist, Card::Taskmaster, Card::Taskmaster]
    );
    next_turn(&mut g);
    next_turn(&mut g);
    resolve_triggers(&mut g);
    assert_eq!((g.turn().actions, g.turn().coins), (3, 2));
}

#[test]
fn journey_keeps_a_card_played_at_the_start_of_clean_up() {
    let mut g = setup_l(&[Card::Village, Card::Sheepdog], &[Card::Journey]);
    set_trait(&mut g, Card::Friendly, Card::Village);
    hand(&mut g, P0, &[Card::Village, Card::Sheepdog, Card::Copper]);
    deck(&mut g, P0, &[Card::Copper; 30]);
    g.start();
    to_buy(&mut g);
    play(&mut g, Card::Copper);
    money(&mut g, 3, 1);
    buy_event(&mut g, Card::Journey);
    // Start of Clean-up: Friendly discards the Village for another; Sheepdog is played.
    assert_eq!(g.pending().unwrap().source, Some(Card::Friendly));
    yes_trigger(&mut g);
    assert_eq!(g.pending().unwrap().source, Some(Card::Sheepdog));
    yes_trigger(&mut g);
    assert_eq!((g.turn().player, g.turn().extra), (P0, Some(Card::Journey)));
    assert_eq!(in_play_kinds(&g, P0), [Card::Copper, Card::Sheepdog]);
}

#[test]
fn bury_does_not_reveal_the_buried_card() {
    let mut g = setup_l(&[Card::Village], &[Card::Bury]);
    hand(&mut g, P0, &[]);
    deck(&mut g, P0, &[Card::Copper; 30]);
    let buried = discard(&mut g, P0, &[Card::Gold, Card::Estate])[0];
    g.enable_trace();
    g.start();
    to_buy(&mut g);
    money(&mut g, 1, 2);
    buy_event(&mut g, Card::Bury);
    choose(&mut g, &[buried]);
    assert_eq!(g.player(P0).deck.first(), Some(&buried));
    let named = |e: &Event| {
        matches!(
            e,
            Event::Moved {
                card: Some(Card::Gold),
                ..
            }
        )
    };
    assert!(!g.log().iter().any(named));
    let shown = Trace::Shown {
        card: buried,
        to: None,
    };
    assert!(!g.trace().unwrap().contains(&shown));
}

#[test]
fn orb_looks_through_the_discard_pile_before_asking_which_option() {
    let mut g = setup(&[Card::SackOfLoot]);
    hand(&mut g, P0, &[Card::Orb]);
    let under = discard(&mut g, P0, &[Card::Village, Card::Estate])[0];
    g.enable_trace();
    g.start();
    to_buy(&mut g);
    play(&mut g, Card::Orb);
    assert_eq!(purpose(&g), Purpose::Mode);
    let looked = Trace::Shown {
        card: under,
        to: Some(P0),
    };
    assert!(g.trace().unwrap().contains(&looked));
}

#[test]
fn a_hasty_card_gained_with_quartermaster_is_not_on_the_quartermaster() {
    let mut g = setup(&[Card::Quartermaster, Card::Village]);
    set_trait(&mut g, Card::Hasty, Card::Village);
    hand(&mut g, P0, &[Card::Quartermaster]);
    deck(&mut g, P0, &[Card::Copper; 40]);
    g.start();
    play(&mut g, Card::Quartermaster);
    next_turn(&mut g);
    next_turn(&mut g);
    mode(&mut g, 0);
    choose_pile(&mut g, Card::Village);
    assert_eq!(aside_kinds(&g, P0), [Card::Village]);
    next_turn(&mut g);
    next_turn(&mut g);
    // Quartermaster first: it has no card to put into the hand.
    let i = trigger_index(&g, Card::Quartermaster);
    respond(&mut g, Response::option(Opt::Trigger(i)));
    mode(&mut g, 1);
    assert!(!hand_kinds(&g, P0).contains(&Card::Village));
    assert!(in_play_kinds(&g, P0).contains(&Card::Village));
}

#[test]
fn a_second_avoid_picks_from_the_shuffled_cards_only() {
    let mut g = setup_l(&[Card::Village], &[Card::Avoid]);
    hand(&mut g, P0, &[]);
    deck(&mut g, P0, &[Card::Province; 2]);
    discard(&mut g, P0, &[Card::Estate; 6]);
    g.start();
    money(&mut g, 4, 3);
    to_buy(&mut g);
    buy_event(&mut g, Card::Avoid);
    buy_event(&mut g, Card::Avoid);
    // Looking at 4 cards shuffles the discard pile under the two Provinces.
    g.stack.push(crate::frame::Frame::look(P0, 4));
    g.pending = None;
    g.run(crate::frame::Input::Start);
    resolve_triggers(&mut g);
    choose_kinds(&mut g, &[Card::Estate; 3]);
    resolve_triggers(&mut g);
    assert_eq!(kinds(&g, &candidates(&g)), [Card::Estate; 3]);
}

#[test]
fn star_chart_after_avoid_picks_from_the_shuffled_cards_only() {
    let mut g = setup_l(&[Card::Village], &[Card::Avoid, Card::StarChart]);
    g.players[0].projects.push(Card::StarChart);
    hand(&mut g, P0, &[]);
    let gold = deck(&mut g, P0, &[Card::Gold])[0];
    let mut cards = vec![Card::Estate; 3];
    cards.extend([Card::Silver, Card::Duchy]);
    discard(&mut g, P0, &cards);
    g.start();
    money(&mut g, 2, 2);
    to_buy(&mut g);
    buy_event(&mut g, Card::Avoid);
    // Looking at 5 cards shuffles the discard pile under the Gold.
    g.stack.push(crate::frame::Frame::look(P0, 5));
    g.pending = None;
    g.run(crate::frame::Input::Start);
    let i = trigger_index(&g, Card::Avoid);
    respond(&mut g, Response::option(Opt::Trigger(i)));
    choose_kinds(&mut g, &[Card::Estate; 3]);
    let i = trigger_index(&g, Card::StarChart);
    respond(&mut g, Response::option(Opt::Trigger(i)));
    assert!(!candidates(&g).contains(&gold));
    choose_kinds(&mut g, &[Card::Silver]);
    assert_eq!(deck_kinds(&g, P0), [Card::Gold, Card::Silver, Card::Duchy]);
}

#[test]
fn differently_named_fated_cards_put_under_are_ordered() {
    let mut g = setup(&[Card::Encampment, Card::Smithy]);
    set_trait(&mut g, Card::Fated, Card::Encampment);
    hand(&mut g, P0, &[Card::Smithy]);
    deck(&mut g, P0, &[]);
    let mut cards = vec![Card::Encampment, Card::Plunder];
    cards.extend([Card::Copper; 4]);
    discard(&mut g, P0, &cards);
    g.start();
    play(&mut g, Card::Smithy);
    resolve_triggers(&mut g);
    choose(&mut g, &[]);
    choose_kinds(&mut g, &[Card::Encampment, Card::Plunder]);
    choose_kinds(&mut g, &[Card::Plunder, Card::Encampment]);
    assert_eq!(
        deck_kinds(&g, P0),
        [Card::Copper, Card::Plunder, Card::Encampment]
    );
}

/// Capital's "when you discard this from play" and Tireless both happen when the Capital
/// is discarded from play.
#[test]
fn a_tireless_capital_still_takes_its_debt() {
    let mut g = setup(&[Card::Capital]);
    set_trait(&mut g, Card::Tireless, Card::Capital);
    let capital = hand(&mut g, P0, &[Card::Capital])[0];
    deck(&mut g, P0, &[Card::Copper; 20]);
    g.start();
    to_buy(&mut g);
    play(&mut g, Card::Capital);
    g.turn.coins = 0;
    end_phase(&mut g);
    assert_eq!(g.turn().player, P1);
    assert_eq!(g.player(P0).deck.last(), Some(&capital));
    assert_eq!(g.player(P0).debt, 6);
}

#[test]
fn a_capital_set_aside_by_trickster_still_takes_its_debt() {
    let mut g = setup(&[Card::Capital, Card::Trickster]);
    hand(&mut g, P0, &[Card::Trickster, Card::Capital]);
    deck(&mut g, P0, &[Card::Copper; 20]);
    g.start();
    play(&mut g, Card::Trickster);
    play(&mut g, Card::Capital);
    g.turn.coins = 0;
    end_phase(&mut g);
    assert_eq!(purpose(&g), Purpose::Trigger);
    let i = trigger_index(&g, Card::Trickster);
    respond(&mut g, Response::option(Opt::Trigger(i)));
    assert_eq!(g.turn().player, P1);
    assert!(hand_kinds(&g, P0).contains(&Card::Capital));
    assert_eq!(g.player(P0).debt, 6);
}

/// Divine Wind's FAQ: "In games using this" abilities keep working for the removed piles.
#[test]
fn shaman_still_gains_from_the_trash_after_divine_wind_removed_its_pile() {
    let mut g = setup_l(&[Card::TeaHouse, Card::Shaman], &[Card::DivineWind]);
    g.prophecy = Some((Card::DivineWind, 1));
    hand(&mut g, P0, &[Card::TeaHouse]);
    deck(&mut g, P0, &[Card::Copper; 10]);
    // The cards `hand` and `deck` replaced are in the trash.
    g.start();
    assert_eq!(
        (purpose(&g), g.pending().unwrap().source),
        (Purpose::Gain, Some(Card::Shaman))
    );
    choose_kinds(&mut g, &[Card::Copper]);
    play(&mut g, Card::TeaHouse);
    assert_eq!(g.supply_pile(Card::Shaman), None);
    next_turn(&mut g);
    assert_eq!(g.turn().player, P1);
    assert_eq!(
        (purpose(&g), g.pending().unwrap().source),
        (Purpose::Gain, Some(Card::Shaman))
    );
}
