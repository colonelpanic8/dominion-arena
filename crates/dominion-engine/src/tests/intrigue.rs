use super::*;

pub(crate) fn modes(g: &mut Game, picks: &[u8]) {
    respond(
        g,
        Response::Options(picks.iter().map(|&i| Opt::Mode(i)).collect()),
    );
}

/// Resolves the pending trigger whose label names `card`.
pub(crate) fn trigger(g: &mut Game, card: Card) {
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
    respond(g, Response::option(options[i]));
}

pub(crate) fn trigger_labels(g: &Game) -> Vec<String> {
    match &g.pending().expect("a decision is pending").kind {
        DecisionKind::Choose { labels, .. } => labels.clone(),
        k => panic!("expected a Choose decision, got {k:?}"),
    }
}

pub(crate) fn setup3(kingdom: &[Card]) -> Game {
    setup_with(GameConfig {
        colonies: Some(false),
        shelters: Some(false),
        ..GameConfig::new(3, kingdom.to_vec(), 7)
    })
}

fn discard_kinds(g: &Game, p: PlayerId) -> Vec<Card> {
    kinds(g, &g.player(p).discard)
}

#[test]
fn baron_discards_an_estate_for_four() {
    let mut g = setup(&[Card::Baron]);
    hand(&mut g, P0, &[Card::Baron, Card::Estate, Card::Copper]);
    g.start();
    play(&mut g, Card::Baron);
    choose_kinds(&mut g, &[Card::Estate]);
    assert_eq!((g.turn().coins, g.turn().buys), (4, 2));
    assert_eq!(discard_kinds(&g, P0), [Card::Estate]);
}

#[test]
fn baron_gains_an_estate_when_not_discarding() {
    let mut g = setup(&[Card::Baron]);
    hand(&mut g, P0, &[Card::Baron, Card::Estate]);
    g.start();
    play(&mut g, Card::Baron);
    choose(&mut g, &[]);
    assert_eq!(g.turn().coins, 0);
    assert_eq!(discard_kinds(&g, P0), [Card::Estate]);
    assert_eq!(hand_kinds(&g, P0), [Card::Estate]);

    let mut g = setup(&[Card::Baron]);
    hand(&mut g, P0, &[Card::Baron, Card::Copper]);
    g.start();
    play(&mut g, Card::Baron);
    assert_eq!(discard_kinds(&g, P0), [Card::Estate]);
}

#[test]
fn bridge_lowers_costs_this_turn_and_stacks() {
    let mut g = setup(&[Card::Bridge, Card::ThroneRoom, Card::Workshop]);
    hand(
        &mut g,
        P0,
        &[Card::ThroneRoom, Card::Bridge, Card::Copper, Card::Copper],
    );
    g.start();
    play(&mut g, Card::ThroneRoom);
    choose_kinds(&mut g, &[Card::Bridge]);
    assert_eq!(g.cost(Card::Gold), Cost::coins(4));
    assert_eq!(g.cost(Card::Estate), Cost::coins(0));
    assert_eq!(g.cost(Card::Copper), Cost::coins(0));
    assert_eq!((g.turn().coins, g.turn().buys), (2, 3));
    play(&mut g, Card::Copper);
    play(&mut g, Card::Copper);
    buy(&mut g, Card::Gold);
    assert_eq!(g.count_owned(P0, Card::Gold), 1);
    end_phase(&mut g);
    assert_eq!(g.cost(Card::Gold), Cost::coins(6));
}

#[test]
fn conspirator_counts_actions_played_including_throne_room_replays() {
    let mut g = setup(&[Card::Conspirator, Card::Village]);
    hand(
        &mut g,
        P0,
        &[Card::Village, Card::Conspirator, Card::Conspirator],
    );
    deck(&mut g, P0, &[Card::Copper; 5]);
    g.start();
    play(&mut g, Card::Village);
    play(&mut g, Card::Conspirator);
    assert_eq!((g.turn().actions, g.turn().coins), (1, 2));
    let hand_before = g.hand(P0).len();
    play(&mut g, Card::Conspirator);
    assert_eq!((g.turn().actions, g.turn().coins), (1, 4));
    assert_eq!(g.hand(P0).len(), hand_before);

    let mut g = setup(&[Card::Conspirator, Card::ThroneRoom]);
    hand(&mut g, P0, &[Card::ThroneRoom, Card::Conspirator]);
    deck(&mut g, P0, &[Card::Copper; 5]);
    g.start();
    play(&mut g, Card::ThroneRoom);
    choose_kinds(&mut g, &[Card::Conspirator]);
    assert_eq!((g.turn().actions, g.turn().coins), (1, 4));
    assert_eq!(g.hand(P0).len(), 1);
}

/// Under Enlightenment the Coppers played were Actions.
#[test]
fn conspirator_counts_cards_that_were_actions_by_an_effect() {
    let mut g = setup(&[Card::Conspirator]);
    g.prophecy = Some((Card::Enlightenment, 0));
    hand(&mut g, P0, &[Card::Copper, Card::Copper, Card::Conspirator]);
    deck(&mut g, P0, &[Card::Estate; 5]);
    g.start();
    play(&mut g, Card::Copper);
    play(&mut g, Card::Copper);
    play(&mut g, Card::Conspirator);
    assert_eq!((g.turn().actions, g.turn().coins), (1, 2));
    assert_eq!(g.hand(P0).len(), 3);
}

#[test]
fn courtier_chooses_one_option_per_type() {
    let mut g = setup(&[Card::Courtier, Card::Mill]);
    hand(&mut g, P0, &[Card::Courtier, Card::Mill]);
    g.start();
    play(&mut g, Card::Courtier);
    match &g.pending().unwrap().kind {
        DecisionKind::Choose { min, max, .. } => assert_eq!((*min, *max), (2, 2)),
        k => panic!("{k:?}"),
    }
    modes(&mut g, &[0, 2]);
    assert_eq!((g.turn().actions, g.turn().coins), (1, 3));
    assert_eq!(hand_kinds(&g, P0), [Card::Mill]);

    let mut g = setup(&[Card::Courtier]);
    hand(&mut g, P0, &[Card::Courtier, Card::Copper]);
    g.start();
    play(&mut g, Card::Courtier);
    modes(&mut g, &[3]);
    assert_eq!(discard_kinds(&g, P0), [Card::Gold]);
}

#[test]
fn courtyard_puts_a_card_back() {
    let mut g = setup(&[Card::Courtyard]);
    hand(&mut g, P0, &[Card::Courtyard, Card::Estate]);
    deck(
        &mut g,
        P0,
        &[Card::Gold, Card::Silver, Card::Copper, Card::Duchy],
    );
    g.start();
    play(&mut g, Card::Courtyard);
    assert_eq!(purpose(&g), Purpose::TopDeck);
    choose_kinds(&mut g, &[Card::Estate]);
    assert_eq!(deck_kinds(&g, P0), [Card::Estate, Card::Duchy]);
    assert_eq!(g.hand(P0).len(), 3);
}

#[test]
fn diplomat_gives_actions_only_with_a_small_hand() {
    let mut g = setup(&[Card::Diplomat]);
    hand(&mut g, P0, &[Card::Diplomat, Card::Copper, Card::Copper]);
    deck(&mut g, P0, &[Card::Copper; 5]);
    g.start();
    play(&mut g, Card::Diplomat);
    assert_eq!(g.turn().actions, 2);

    let mut g = setup(&[Card::Diplomat]);
    hand(
        &mut g,
        P0,
        &[
            Card::Diplomat,
            Card::Copper,
            Card::Copper,
            Card::Copper,
            Card::Copper,
        ],
    );
    deck(&mut g, P0, &[Card::Copper; 5]);
    g.start();
    play(&mut g, Card::Diplomat);
    assert_eq!(g.turn().actions, 0);
}

#[test]
fn diplomat_reacts_to_attacks_and_can_be_revealed_again() {
    let mut g = setup(&[Card::Militia, Card::Diplomat, Card::Moat]);
    hand(&mut g, P0, &[Card::Militia]);
    hand(
        &mut g,
        P1,
        &[
            Card::Diplomat,
            Card::Estate,
            Card::Estate,
            Card::Copper,
            Card::Copper,
            Card::Copper,
            Card::Copper,
        ],
    );
    deck(
        &mut g,
        P1,
        &[Card::Moat, Card::Silver, Card::Gold, Card::Gold],
    );
    g.start();
    play(&mut g, Card::Militia);
    trigger(&mut g, Card::Diplomat);
    assert_eq!(g.hand(P1).len(), 9);
    choose_kinds(&mut g, &[Card::Estate, Card::Estate, Card::Copper]);
    // Six cards in hand: Diplomat may be revealed again; the drawn Moat is offered too.
    let labels = trigger_labels(&g);
    assert!(
        labels.iter().any(|l| l.starts_with("Diplomat")),
        "{labels:?}"
    );
    assert!(labels.iter().any(|l| l.starts_with("Moat")), "{labels:?}");
    trigger(&mut g, Card::Diplomat);
    choose_kinds(&mut g, &[Card::Copper, Card::Copper, Card::Copper]);
    // Five cards: Diplomat, Moat, Silver, Gold, Gold.
    trigger(&mut g, Card::Moat);
    no(&mut g);
    assert_eq!(g.hand(P1).len(), 5);
    assert_eq!(g.pending().unwrap().player, P0);
}

#[test]
fn diplomat_needs_five_cards_to_react() {
    let mut g = setup(&[Card::Militia, Card::Diplomat]);
    hand(&mut g, P0, &[Card::Militia]);
    hand(
        &mut g,
        P1,
        &[Card::Diplomat, Card::Estate, Card::Copper, Card::Copper],
    );
    g.start();
    play(&mut g, Card::Militia);
    assert_eq!(
        (g.pending().unwrap().player, purpose(&g)),
        (P1, Purpose::Discard)
    );
}

#[test]
fn duke_is_worth_a_point_per_duchy() {
    let mut g = setup(&[Card::Duke]);
    g.start();
    for card in [
        Card::Duke,
        Card::Duke,
        Card::Duchy,
        Card::Duchy,
        Card::Duchy,
    ] {
        g.create(card, Zone::Discard(P0));
    }
    assert_eq!(g.score(P0), 3 + 3 * 3 + 2 * 3);
    assert_eq!(g.pile(g.supply_pile(Card::Duke).unwrap()).cards.len(), 8);
}

#[test]
fn farm_mill_nobles_and_great_hall_score() {
    let mut g = setup(&[Card::Farm]);
    g.start();
    for card in [Card::Farm, Card::Mill, Card::Nobles, Card::GreatHall] {
        g.create(card, Zone::Discard(P0));
    }
    assert_eq!(g.score(P0), 3 + 2 + 1 + 2 + 1);
}

#[test]
fn farm_is_worth_two() {
    let mut g = setup(&[Card::Farm]);
    hand(&mut g, P0, &[Card::Farm]);
    g.start();
    play(&mut g, Card::Farm);
    assert_eq!(g.turn().coins, 2);
}

#[test]
fn ironworks_gives_a_bonus_per_type_gained() {
    let mut g = setup(&[Card::Ironworks, Card::Mill]);
    hand(&mut g, P0, &[Card::Ironworks]);
    deck(&mut g, P0, &[Card::Gold]);
    g.start();
    play(&mut g, Card::Ironworks);
    choose_pile(&mut g, Card::Mill);
    assert_eq!(g.turn().actions, 1);
    assert_eq!(hand_kinds(&g, P0), [Card::Gold]);

    let mut g = setup(&[Card::Ironworks]);
    hand(&mut g, P0, &[Card::Ironworks]);
    g.start();
    play(&mut g, Card::Ironworks);
    choose_pile(&mut g, Card::Silver);
    assert_eq!((g.turn().actions, g.turn().coins), (0, 1));
}

#[test]
fn ironworks_gains_a_duchy_under_bridge() {
    let mut g = setup(&[Card::Ironworks, Card::Bridge, Card::Village]);
    hand(&mut g, P0, &[Card::Village, Card::Bridge, Card::Ironworks]);
    deck(&mut g, P0, &[Card::Copper, Card::Copper]);
    g.start();
    play(&mut g, Card::Village);
    play(&mut g, Card::Bridge);
    play(&mut g, Card::Ironworks);
    choose_pile(&mut g, Card::Duchy);
    assert_eq!(g.count_owned(P0, Card::Duchy), 1);
    assert_eq!(g.hand(P0).len(), 2);
}

#[test]
fn lurker_trashes_from_the_supply_or_gains_from_the_trash() {
    let mut g = setup(&[Card::Lurker, Card::Nobles]);
    hand(&mut g, P0, &[Card::Lurker, Card::Lurker]);
    g.start();
    play(&mut g, Card::Lurker);
    modes(&mut g, &[0]);
    choose_pile(&mut g, Card::Nobles);
    assert_eq!(kinds(&g, g.trash()).last(), Some(&Card::Nobles));
    play(&mut g, Card::Lurker);
    modes(&mut g, &[1]);
    assert_eq!(discard_kinds(&g, P0), [Card::Nobles]);
}

#[test]
fn masquerade_passes_between_players_with_cards() {
    let mut g = setup3(&[Card::Masquerade]);
    hand(&mut g, P0, &[Card::Masquerade, Card::Estate]);
    deck(&mut g, P0, &[Card::Copper, Card::Copper]);
    hand(&mut g, P1, &[Card::Gold, Card::Curse]);
    hand(&mut g, P2, &[]);
    g.start();
    play(&mut g, Card::Masquerade);
    assert_eq!(
        (g.pending().unwrap().player, purpose(&g)),
        (P0, Purpose::Pass)
    );
    choose_kinds(&mut g, &[Card::Estate]);
    assert_eq!(g.pending().unwrap().player, P1);
    choose_kinds(&mut g, &[Card::Curse]);
    // P2 has no cards, so P1 passes to P0.
    assert_eq!(hand_kinds(&g, P1), [Card::Gold, Card::Estate]);
    assert!(g.hand(P2).is_empty());
    assert_eq!(purpose(&g), Purpose::Trash);
    choose_kinds(&mut g, &[Card::Curse]);
    assert_eq!(kinds(&g, g.trash()).last(), Some(&Card::Curse));
    assert_eq!(hand_kinds(&g, P0), [Card::Copper, Card::Copper]);
}

#[test]
fn mill_needs_two_discards_for_coins() {
    let mut g = setup(&[Card::Mill]);
    hand(&mut g, P0, &[Card::Mill, Card::Estate, Card::Curse]);
    deck(&mut g, P0, &[Card::Copper]);
    g.start();
    play(&mut g, Card::Mill);
    yes(&mut g);
    choose_kinds(&mut g, &[Card::Estate, Card::Curse]);
    assert_eq!(g.turn().coins, 2);

    let mut g = setup(&[Card::Mill]);
    hand(&mut g, P0, &[Card::Mill]);
    deck(&mut g, P0, &[Card::Copper]);
    g.start();
    play(&mut g, Card::Mill);
    yes(&mut g);
    assert_eq!(discard_kinds(&g, P0), [Card::Copper]);
    assert_eq!(g.turn().coins, 0);
}

#[test]
fn mining_village_trashes_itself_once_under_throne_room() {
    let mut g = setup(&[Card::MiningVillage, Card::ThroneRoom]);
    hand(&mut g, P0, &[Card::ThroneRoom, Card::MiningVillage]);
    deck(&mut g, P0, &[Card::Copper; 4]);
    g.start();
    play(&mut g, Card::ThroneRoom);
    choose_kinds(&mut g, &[Card::MiningVillage]);
    assert_eq!(purpose(&g), Purpose::Trash);
    yes(&mut g);
    // The second play gets +1 Card +2 Actions but can't trash it again.
    assert_eq!(g.turn().coins, 2);
    assert_eq!(g.turn().actions, 4);
    assert_eq!(g.hand(P0).len(), 2);
    assert_eq!(kinds(&g, g.trash()).last(), Some(&Card::MiningVillage));
}

#[test]
fn minion_attacks_only_hands_of_five_or_more() {
    let mut g = setup3(&[Card::Minion]);
    hand(&mut g, P0, &[Card::Minion, Card::Estate]);
    deck(&mut g, P0, &[Card::Copper; 4]);
    hand(&mut g, P1, &[Card::Estate; 5]);
    deck(&mut g, P1, &[Card::Silver; 4]);
    hand(&mut g, P2, &[Card::Estate; 4]);
    g.start();
    play(&mut g, Card::Minion);
    modes(&mut g, &[1]);
    assert_eq!(hand_kinds(&g, P0), [Card::Copper; 4]);
    assert_eq!(hand_kinds(&g, P1), [Card::Silver; 4]);
    assert_eq!(g.hand(P2).len(), 4);
    assert_eq!(g.turn().actions, 1);

    let mut g = setup(&[Card::Minion]);
    hand(&mut g, P0, &[Card::Minion]);
    g.start();
    play(&mut g, Card::Minion);
    modes(&mut g, &[0]);
    assert_eq!(g.turn().coins, 2);
}

#[test]
fn nobles_chooses_cards_or_actions() {
    let mut g = setup(&[Card::Nobles]);
    hand(&mut g, P0, &[Card::Nobles, Card::Nobles]);
    deck(&mut g, P0, &[Card::Copper; 5]);
    g.start();
    play(&mut g, Card::Nobles);
    modes(&mut g, &[1]);
    assert_eq!(g.turn().actions, 2);
    play(&mut g, Card::Nobles);
    modes(&mut g, &[0]);
    assert_eq!(g.hand(P0).len(), 3);
}

#[test]
fn patrol_takes_victory_cards_and_curses() {
    let mut g = setup(&[Card::Patrol]);
    hand(&mut g, P0, &[Card::Patrol]);
    deck(
        &mut g,
        P0,
        &[
            Card::Copper,
            Card::Copper,
            Card::Copper,
            Card::Estate,
            Card::Gold,
            Card::Curse,
            Card::Silver,
            Card::Duchy,
        ],
    );
    g.start();
    play(&mut g, Card::Patrol);
    choose_kinds(&mut g, &[Card::Silver, Card::Gold]);
    let mut h = hand_kinds(&g, P0);
    h.sort();
    let mut want = vec![
        Card::Copper,
        Card::Copper,
        Card::Copper,
        Card::Estate,
        Card::Curse,
    ];
    want.sort();
    assert_eq!(h, want);
    assert_eq!(deck_kinds(&g, P0), [Card::Silver, Card::Gold, Card::Duchy]);
}

#[test]
fn pawn_picks_two_different_options() {
    let mut g = setup(&[Card::Pawn]);
    hand(&mut g, P0, &[Card::Pawn]);
    deck(&mut g, P0, &[Card::Copper]);
    g.start();
    play(&mut g, Card::Pawn);
    assert_eq!(
        g.respond(Response::Options(vec![Opt::Mode(0), Opt::Mode(0)])),
        Err(ResponseError::DuplicateOption(Opt::Mode(0)))
    );
    modes(&mut g, &[0, 3]);
    assert_eq!(g.turn().coins, 1);
    assert_eq!(hand_kinds(&g, P0), [Card::Copper]);
}

#[test]
fn replace_topdecks_actions_and_curses_for_victory_cards() {
    let mut g = setup(&[Card::Replace, Card::Moat]);
    hand(
        &mut g,
        P0,
        &[Card::Replace, Card::Replace, Card::Estate, Card::Silver],
    );
    hand(&mut g, P1, &[Card::Copper]);
    g.start();
    play(&mut g, Card::Replace);
    choose_kinds(&mut g, &[Card::Estate]);
    choose_pile(&mut g, Card::Silver);
    assert_eq!(deck_kinds(&g, P0)[0], Card::Silver);
    assert_eq!(g.count_owned(P1, Card::Curse), 0);

    // A Victory card makes the others gain a Curse (Moat would block it).
    let mut g = setup(&[Card::Replace, Card::Moat]);
    hand(&mut g, P0, &[Card::Replace, Card::Silver]);
    hand(&mut g, P1, &[Card::Copper]);
    g.start();
    play(&mut g, Card::Replace);
    choose_pile(&mut g, Card::Duchy);
    assert_eq!(discard_kinds(&g, P0), [Card::Duchy]);
    assert_eq!(g.count_owned(P1, Card::Curse), 1);

    let mut g = setup(&[Card::Replace, Card::Moat]);
    hand(&mut g, P0, &[Card::Replace, Card::Silver]);
    hand(&mut g, P1, &[Card::Moat]);
    g.start();
    play(&mut g, Card::Replace);
    trigger(&mut g, Card::Moat);
    choose_pile(&mut g, Card::Duchy);
    assert_eq!(g.count_owned(P1, Card::Curse), 0);
}

#[test]
fn secret_passage_puts_a_card_anywhere_in_the_deck() {
    let mut g = setup(&[Card::SecretPassage]);
    hand(&mut g, P0, &[Card::SecretPassage, Card::Gold]);
    deck(
        &mut g,
        P0,
        &[
            Card::Copper,
            Card::Copper,
            Card::Estate,
            Card::Silver,
            Card::Duchy,
        ],
    );
    g.start();
    play(&mut g, Card::SecretPassage);
    choose_kinds(&mut g, &[Card::Gold]);
    assert_eq!(
        g.pending().unwrap().kind,
        DecisionKind::Number { min: 0, max: 3 }
    );
    respond(&mut g, Response::Number(2));
    assert_eq!(
        deck_kinds(&g, P0),
        [Card::Estate, Card::Silver, Card::Gold, Card::Duchy]
    );
}

#[test]
fn shanty_town_draws_without_actions_in_hand() {
    let mut g = setup(&[Card::ShantyTown]);
    hand(
        &mut g,
        P0,
        &[Card::ShantyTown, Card::ShantyTown, Card::Copper],
    );
    deck(&mut g, P0, &[Card::Copper, Card::Estate, Card::Silver]);
    g.start();
    play(&mut g, Card::ShantyTown);
    assert_eq!(g.hand(P0).len(), 2);
    play(&mut g, Card::ShantyTown);
    assert_eq!(g.hand(P0).len(), 3);
    assert_eq!(g.turn().actions, 3);
}

#[test]
fn steward_modes() {
    let mut g = setup(&[Card::Steward]);
    hand(
        &mut g,
        P0,
        &[
            Card::Steward,
            Card::Steward,
            Card::Village,
            Card::Estate,
            Card::Curse,
        ],
    );
    deck(&mut g, P0, &[Card::Copper; 2]);
    g.start();
    play(&mut g, Card::Steward);
    modes(&mut g, &[2]);
    choose_kinds(&mut g, &[Card::Estate, Card::Curse]);
    assert!(kinds(&g, g.trash()).ends_with(&[Card::Estate, Card::Curse]));
    assert_eq!(hand_kinds(&g, P0), [Card::Steward, Card::Village]);
}

#[test]
fn swindler_swaps_the_top_card_for_one_of_the_same_cost() {
    let mut g = setup(&[Card::Swindler, Card::Village]);
    hand(&mut g, P0, &[Card::Swindler]);
    deck(&mut g, P1, &[Card::Silver, Card::Gold]);
    g.start();
    play(&mut g, Card::Swindler);
    assert_eq!(g.turn().coins, 2);
    let d = g.pending().unwrap();
    assert_eq!((d.player, d.purpose), (P0, Purpose::Gain));
    choose_pile(&mut g, Card::Village);
    assert_eq!(kinds(&g, g.trash()).last(), Some(&Card::Silver));
    assert_eq!(discard_kinds(&g, P1), [Card::Village]);
    assert_eq!(deck_kinds(&g, P1), [Card::Gold]);
}

#[test]
fn swindler_gives_a_curse_for_a_copper() {
    let mut g = setup(&[Card::Swindler]);
    hand(&mut g, P0, &[Card::Swindler]);
    deck(&mut g, P1, &[Card::Copper]);
    g.start();
    play(&mut g, Card::Swindler);
    choose_pile(&mut g, Card::Curse);
    assert_eq!(discard_kinds(&g, P1), [Card::Curse]);
}

/// Swindler trashes the top card without revealing it, so a Patron gives no Coffers.
#[test]
fn swindler_does_not_reveal_the_card_it_trashes() {
    let mut g = setup(&[Card::Swindler, Card::Patron]);
    hand(&mut g, P0, &[Card::Swindler]);
    deck(&mut g, P1, &[Card::Patron, Card::Copper]);
    g.start();
    play(&mut g, Card::Swindler);
    assert_eq!(kinds(&g, g.trash()).last(), Some(&Card::Patron));
    assert_eq!(g.player(P1).coffers, 0);
}

#[test]
fn torturer_victims_choose() {
    let mut g = setup3(&[Card::Torturer]);
    hand(&mut g, P0, &[Card::Torturer]);
    hand(&mut g, P1, &[Card::Copper, Card::Estate, Card::Silver]);
    hand(&mut g, P2, &[Card::Copper]);
    deck(&mut g, P0, &[Card::Copper; 3]);
    g.start();
    play(&mut g, Card::Torturer);
    assert_eq!(g.pending().unwrap().player, P1);
    modes(&mut g, &[0]);
    choose_kinds(&mut g, &[Card::Copper, Card::Estate]);
    assert_eq!(g.pending().unwrap().player, P2);
    modes(&mut g, &[1]);
    assert_eq!(hand_kinds(&g, P1), [Card::Silver]);
    assert_eq!(hand_kinds(&g, P2), [Card::Copper, Card::Curse]);
}

#[test]
fn trading_post_needs_two_cards_for_a_silver() {
    let mut g = setup(&[Card::TradingPost]);
    hand(
        &mut g,
        P0,
        &[Card::TradingPost, Card::Estate, Card::Curse, Card::Copper],
    );
    g.start();
    play(&mut g, Card::TradingPost);
    choose_kinds(&mut g, &[Card::Estate, Card::Curse]);
    assert_eq!(hand_kinds(&g, P0), [Card::Copper, Card::Silver]);

    let mut g = setup(&[Card::TradingPost]);
    hand(&mut g, P0, &[Card::TradingPost, Card::Estate]);
    g.start();
    play(&mut g, Card::TradingPost);
    assert!(g.hand(P0).is_empty());
    assert_eq!(g.count_owned(P0, Card::Silver), 0);
}

#[test]
fn upgrade_gains_a_card_costing_exactly_one_more() {
    let mut g = setup(&[Card::Upgrade, Card::Bridge, Card::Village]);
    hand(&mut g, P0, &[Card::Upgrade, Card::Estate]);
    deck(&mut g, P0, &[Card::Gold]);
    g.start();
    play(&mut g, Card::Upgrade);
    choose_kinds(&mut g, &[Card::Estate]);
    let d = g.pending().unwrap().clone();
    let DecisionKind::Pile { piles, .. } = d.kind else {
        panic!()
    };
    let names: Vec<Card> = piles.iter().map(|&p| g.pile(p).card).collect();
    assert_eq!(names, [Card::Silver, Card::Village]);
    choose_pile(&mut g, Card::Silver);
    assert_eq!(discard_kinds(&g, P0), [Card::Silver]);
}

#[test]
fn upgrade_after_bridge_turns_a_copper_into_an_estate() {
    let mut g = setup(&[Card::Upgrade, Card::Bridge, Card::Village]);
    hand(
        &mut g,
        P0,
        &[Card::Village, Card::Bridge, Card::Upgrade, Card::Copper],
    );
    deck(&mut g, P0, &[Card::Gold, Card::Gold]);
    g.start();
    play(&mut g, Card::Village);
    play(&mut g, Card::Bridge);
    play(&mut g, Card::Upgrade);
    choose_kinds(&mut g, &[Card::Copper]);
    // Estate costs $1 now; nothing else in this kingdom does.
    assert_eq!(discard_kinds(&g, P0), [Card::Estate]);
}

#[test]
fn wishing_well_takes_the_named_card() {
    let mut g = setup(&[Card::WishingWell]);
    hand(&mut g, P0, &[Card::WishingWell, Card::WishingWell]);
    deck(
        &mut g,
        P0,
        &[Card::Copper, Card::Gold, Card::Copper, Card::Silver],
    );
    g.start();
    play(&mut g, Card::WishingWell);
    respond(&mut g, Response::Name(Card::Gold));
    assert!(g.log().contains(&Event::Named {
        player: P0,
        card: Card::Gold
    }));
    assert_eq!(
        hand_kinds(&g, P0),
        [Card::WishingWell, Card::Copper, Card::Gold]
    );
    play(&mut g, Card::WishingWell);
    respond(&mut g, Response::Name(Card::Gold));
    assert_eq!(deck_kinds(&g, P0), [Card::Silver]);
}

#[test]
fn coppersmith_boosts_coppers_this_turn() {
    let mut g = setup(&[Card::Coppersmith, Card::ThroneRoom]);
    hand(
        &mut g,
        P0,
        &[
            Card::ThroneRoom,
            Card::Coppersmith,
            Card::Copper,
            Card::Silver,
        ],
    );
    g.start();
    play(&mut g, Card::ThroneRoom);
    choose_kinds(&mut g, &[Card::Coppersmith]);
    play(&mut g, Card::Copper);
    play(&mut g, Card::Silver);
    assert_eq!(g.turn().coins, 3 + 2);
}

#[test]
fn great_hall_is_a_cantrip() {
    let mut g = setup(&[Card::GreatHall]);
    hand(&mut g, P0, &[Card::GreatHall]);
    deck(&mut g, P0, &[Card::Copper]);
    g.start();
    play(&mut g, Card::GreatHall);
    assert_eq!((g.turn().actions, g.hand(P0).len()), (1, 1));
    assert_eq!(
        g.pile(g.supply_pile(Card::GreatHall).unwrap()).cards.len(),
        8
    );
}

#[test]
fn saboteur_trashes_the_first_card_costing_three_or_more() {
    let mut g = setup(&[Card::Saboteur, Card::Village]);
    hand(&mut g, P0, &[Card::Saboteur]);
    deck(
        &mut g,
        P1,
        &[Card::Copper, Card::Estate, Card::Gold, Card::Silver],
    );
    g.start();
    play(&mut g, Card::Saboteur);
    let d = g.pending().unwrap().clone();
    assert_eq!((d.player, d.purpose), (P1, Purpose::Gain));
    let DecisionKind::Pile { piles, optional } = d.kind else {
        panic!()
    };
    assert!(optional);
    assert!(piles.contains(&g.supply_pile(Card::Silver).unwrap()));
    assert!(!piles.contains(&g.supply_pile(Card::Duchy).unwrap()));
    choose_pile(&mut g, Card::Village);
    assert_eq!(kinds(&g, g.trash()).last(), Some(&Card::Gold));
    let mut d = discard_kinds(&g, P1);
    d.sort();
    let mut want = vec![Card::Copper, Card::Estate, Card::Village];
    want.sort();
    assert_eq!(d, want);
    assert_eq!(deck_kinds(&g, P1), [Card::Silver]);
}

#[test]
fn saboteur_misses_without_expensive_cards() {
    let mut g = setup(&[Card::Saboteur]);
    hand(&mut g, P0, &[Card::Saboteur]);
    deck(&mut g, P1, &[Card::Copper, Card::Estate]);
    discard(&mut g, P1, &[Card::Curse]);
    g.start();
    play(&mut g, Card::Saboteur);
    assert_eq!(g.player(P1).discard.len(), 3);
    assert!(g.player(P1).deck.is_empty());
    assert_eq!(g.pending().unwrap().player, P0);
}

#[test]
fn scout_takes_victory_cards() {
    let mut g = setup(&[Card::Scout]);
    hand(&mut g, P0, &[Card::Scout]);
    deck(
        &mut g,
        P0,
        &[
            Card::Estate,
            Card::Curse,
            Card::Nobles,
            Card::Gold,
            Card::Copper,
        ],
    );
    g.start();
    play(&mut g, Card::Scout);
    choose_kinds(&mut g, &[Card::Gold, Card::Curse]);
    assert_eq!(hand_kinds(&g, P0), [Card::Estate, Card::Nobles]);
    assert_eq!(deck_kinds(&g, P0), [Card::Gold, Card::Curse, Card::Copper]);
    assert_eq!(g.turn().actions, 1);
}

#[test]
fn secret_chamber_discards_for_coins() {
    let mut g = setup(&[Card::SecretChamber]);
    hand(
        &mut g,
        P0,
        &[
            Card::SecretChamber,
            Card::Estate,
            Card::Estate,
            Card::Copper,
        ],
    );
    g.start();
    play(&mut g, Card::SecretChamber);
    choose_kinds(&mut g, &[Card::Estate, Card::Estate]);
    assert_eq!(g.turn().coins, 2);
}

#[test]
fn secret_chamber_reacts_by_drawing_and_putting_back() {
    let mut g = setup(&[Card::SecretChamber, Card::Militia]);
    hand(&mut g, P0, &[Card::Militia]);
    hand(
        &mut g,
        P1,
        &[
            Card::SecretChamber,
            Card::Estate,
            Card::Estate,
            Card::Copper,
        ],
    );
    deck(&mut g, P1, &[Card::Gold, Card::Silver]);
    g.start();
    play(&mut g, Card::Militia);
    trigger(&mut g, Card::SecretChamber);
    assert_eq!(g.hand(P1).len(), 6);
    choose_kinds(&mut g, &[Card::Estate]);
    choose_kinds(&mut g, &[Card::Estate]);
    assert_eq!(deck_kinds(&g, P1), [Card::Estate, Card::Estate]);
    // It may be revealed again; decline, then discard down to 3.
    let labels = trigger_labels(&g);
    assert!(labels.iter().any(|l| l.starts_with("Secret Chamber")));
    no(&mut g);
    choose_kinds(&mut g, &[Card::SecretChamber]);
    assert_eq!(hand_kinds(&g, P1), [Card::Copper, Card::Gold, Card::Silver]);
}

#[test]
fn tribute_rewards_each_differently_named_card() {
    let mut g = setup(&[Card::Tribute, Card::Mill]);
    hand(&mut g, P0, &[Card::Tribute]);
    deck(&mut g, P0, &[Card::Copper, Card::Copper]);
    deck(&mut g, P1, &[Card::Mill, Card::Silver, Card::Gold]);
    g.start();
    play(&mut g, Card::Tribute);
    assert_eq!((g.turn().actions, g.turn().coins), (2, 2));
    assert_eq!(g.hand(P0).len(), 2);
    assert_eq!(discard_kinds(&g, P1), [Card::Mill, Card::Silver]);

    let mut g = setup(&[Card::Tribute]);
    hand(&mut g, P0, &[Card::Tribute]);
    deck(&mut g, P1, &[Card::Silver, Card::Silver]);
    g.start();
    play(&mut g, Card::Tribute);
    assert_eq!(g.turn().coins, 2);
}

/// Random games with 8 cards from `pool` plus Throne Room and Moat as the kingdom, each
/// player starting with a copy of every Kingdom card (and of `extra`) so that all of them
/// get played. `landscapes` gives a seed's landscapes.
pub(crate) fn random_games_with_copies(
    pool: &[Card],
    extra: &[Card],
    seeds: std::ops::Range<u64>,
    landscapes: impl Fn(u64) -> Vec<Card>,
) {
    for seed in seeds {
        let mut rng = ChaCha8Rng::seed_from_u64(seed);
        let mut cards = pool.to_vec();
        cards.shuffle(&mut rng);
        cards.truncate(8);
        cards.extend([Card::ThroneRoom, Card::Moat]);
        let players = rng.random_range(2..=4);
        let mut config = GameConfig::new(players, cards.clone(), seed);
        config.turn_limit = Some(60);
        config.landscapes = landscapes(seed);
        let mut g = setup_with(config);
        for p in 0..players {
            let p = PlayerId(p);
            for &c in cards.iter().chain(extra) {
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
    }
}

/// Random games over Intrigue, removed cards included.
#[test]
fn random_intrigue_games_keep_invariants() {
    let pool: Vec<Card> = Card::in_set(Set::Intrigue)
        .filter(|c| c.def().kingdom)
        .collect();
    random_games_with_copies(&pool, &[], 0..fuzz_seeds(120), |_| vec![]);
}

#[test]
fn moat_blocks_torturer_and_swindler() {
    let mut g = setup(&[Card::Torturer, Card::Swindler, Card::Moat, Card::Village]);
    hand(&mut g, P0, &[Card::Village, Card::Torturer, Card::Swindler]);
    deck(&mut g, P0, &[Card::Copper; 5]);
    hand(&mut g, P1, &[Card::Moat, Card::Copper]);
    deck(&mut g, P1, &[Card::Gold]);
    g.start();
    play(&mut g, Card::Village);
    play(&mut g, Card::Torturer);
    trigger(&mut g, Card::Moat);
    play(&mut g, Card::Swindler);
    trigger(&mut g, Card::Moat);
    assert_eq!(hand_kinds(&g, P1), [Card::Moat, Card::Copper]);
    assert_eq!(deck_kinds(&g, P1), [Card::Gold]);
    assert_eq!(g.turn().coins, 2);
}
