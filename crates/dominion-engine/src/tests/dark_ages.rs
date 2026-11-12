use super::*;
use rand::SeedableRng;

fn mode(g: &mut Game, i: u8) {
    respond(g, Response::option(Opt::Mode(i)));
}

fn trigger(g: &mut Game, i: u16) {
    respond(g, Response::option(Opt::Trigger(i)));
}

fn name(g: &mut Game, card: Card) {
    respond(g, Response::Name(card));
}

/// Puts the cards `hand`/`deck` replaced back in their piles, leaving the trash empty.
fn empty_trash(g: &mut Game) {
    for c in g.trash().to_vec() {
        assert!(g.return_to_pile(c));
    }
}

fn pile_len(g: &Game, card: Card) -> usize {
    g.pile(g.pile_of(card).expect("pile exists")).cards.len()
}

fn discard_kinds(g: &Game, p: PlayerId) -> Vec<Card> {
    kinds(g, &g.player(p).discard)
}

fn trash_kinds(g: &Game) -> Vec<Card> {
    kinds(g, g.trash())
}

fn owned_where(g: &Game, p: PlayerId, types: Types) -> usize {
    g.player(p).owned().filter(|&c| g.is(c, types)).count()
}

// ---- Setup ----

#[test]
fn a_looter_brings_a_ruins_pile_of_ten_per_player_after_the_first() {
    for (players, size) in [(2, 10), (3, 20), (4, 30), (6, 50)] {
        let g = setup_with(GameConfig {
            shelters: Some(false),
            ..GameConfig::new(players, vec![Card::Marauder], 1)
        });
        let pile = g.supply_pile(Card::Ruins).expect("Ruins in the Supply");
        let cards = &g.pile(pile).cards;
        assert_eq!(cards.len(), size);
        for kind in Card::Ruins.pile_members() {
            let n = cards.iter().filter(|&&c| g.card(c) == kind).count();
            assert!(n <= 10, "{kind}: {n}");
        }
    }
    let g = setup(&[Card::Sage]);
    assert_eq!(g.pile_of(Card::Ruins), None);
    assert_eq!(g.pile_of(Card::Spoils), None);
}

#[test]
fn non_supply_piles_come_with_the_cards_that_use_them() {
    let g = setup(&[Card::BanditCamp, Card::Hermit, Card::Urchin]);
    for (card, size) in [
        (Card::Spoils, 15),
        (Card::Madman, 10),
        (Card::Mercenary, 10),
    ] {
        let pile = g.pile_of(card).expect("pile exists");
        assert!(!g.pile(pile).in_supply, "{card}");
        assert_eq!(pile_len(&g, card), size);
    }
}

#[test]
fn the_knights_pile_holds_the_ten_knights_shuffled() {
    let tops: std::collections::BTreeSet<Card> = (0..20)
        .map(|seed| {
            let g = setup_with(GameConfig::new(2, vec![Card::Knights], seed));
            let pile = g.supply_pile(Card::Knights).expect("Knights pile");
            let mut names: Vec<Card> = kinds(&g, &g.pile(pile).cards);
            let top = *names.last().unwrap();
            names.sort();
            let mut all = Card::Knights.pile_members();
            all.sort();
            assert_eq!(names, all);
            top
        })
        .collect();
    assert!(tops.len() > 3, "the top Knight varies by seed");
}

#[test]
fn rats_pile_has_twenty_cards() {
    let g = setup_with(GameConfig::new(4, vec![Card::Rats], 1));
    assert_eq!(pile_len(&g, Card::Rats), 20);
}

#[test]
fn shelters_replace_the_starting_estates() {
    let g = setup_with(GameConfig {
        shelters: Some(true),
        ..GameConfig::new(2, vec![Card::Sage], 1)
    });
    for p in [P0, P1] {
        for shelter in [Card::Hovel, Card::Necropolis, Card::OvergrownEstate] {
            assert_eq!(g.count_owned(p, shelter), 1);
        }
        assert_eq!(g.count_owned(p, Card::Estate), 0);
    }
    assert_eq!(pile_len(&g, Card::Estate), 8);
}

// ---- $1 - $3 ----

#[test]
fn poor_house_loses_a_coin_per_treasure_but_not_below_zero() {
    let mut g = setup(&[Card::PoorHouse]);
    hand(
        &mut g,
        P0,
        &[Card::PoorHouse, Card::Copper, Card::Silver, Card::Estate],
    );
    g.start();
    play(&mut g, Card::PoorHouse);
    assert_eq!(g.turn().coins, 2);

    let mut g = setup(&[Card::PoorHouse]);
    hand(
        &mut g,
        P0,
        &[
            Card::PoorHouse,
            Card::Copper,
            Card::Copper,
            Card::Copper,
            Card::Copper,
            Card::Copper,
        ],
    );
    g.start();
    play(&mut g, Card::PoorHouse);
    assert_eq!(g.turn().coins, 0);
}

#[test]
fn beggar_gains_three_coppers_to_hand() {
    let mut g = setup(&[Card::Beggar]);
    hand(&mut g, P0, &[Card::Beggar]);
    g.start();
    play(&mut g, Card::Beggar);
    assert_eq!(hand_kinds(&g, P0), [Card::Copper; 3]);
}

#[test]
fn beggar_is_discarded_to_an_attack_for_two_silvers_one_on_the_deck() {
    let mut g = setup(&[Card::Beggar, Card::Marauder]);
    hand(&mut g, P0, &[Card::Marauder]);
    hand(&mut g, P1, &[Card::Beggar, Card::Copper]);
    deck(&mut g, P1, &[Card::Estate]);
    g.start();
    play(&mut g, Card::Marauder);
    assert_eq!(g.pending().unwrap().player, P1);
    trigger(&mut g, 0);
    assert_eq!(deck_kinds(&g, P1), [Card::Silver, Card::Estate]);
    let discard = discard_kinds(&g, P1);
    assert!(discard.contains(&Card::Beggar) && discard.contains(&Card::Silver));
    // The attack still happens.
    assert_eq!(owned_where(&g, P1, Types::RUINS), 1);
}

#[test]
fn squire_gives_a_coin_and_one_of_three_bonuses() {
    for (m, actions, buys, silvers) in [(0, 2, 1, 0), (1, 0, 3, 0), (2, 0, 1, 1)] {
        let mut g = setup(&[Card::Squire]);
        hand(&mut g, P0, &[Card::Squire]);
        g.start();
        play(&mut g, Card::Squire);
        mode(&mut g, m);
        assert_eq!(g.turn().coins, 1);
        assert_eq!(g.turn().actions, actions);
        assert_eq!(g.turn().buys, buys);
        assert_eq!(g.count_owned(P0, Card::Silver), silvers);
    }
}

#[test]
fn squire_trashed_gains_an_attack_from_the_supply() {
    let mut g = setup(&[Card::Squire, Card::JunkDealer, Card::Marauder, Card::Urchin]);
    hand(&mut g, P0, &[Card::JunkDealer, Card::Squire]);
    deck(&mut g, P0, &[Card::Copper]);
    g.start();
    play(&mut g, Card::JunkDealer);
    choose_kinds(&mut g, &[Card::Squire]);
    // Marauder and Urchin; not the Mercenary, which is not in the Supply.
    let d = g.pending().unwrap();
    let DecisionKind::Pile { piles, .. } = &d.kind else {
        panic!("expected a pile choice, got {d:?}")
    };
    assert_eq!(piles.len(), 2);
    choose_pile(&mut g, Card::Marauder);
    assert_eq!(g.count_owned(P0, Card::Marauder), 1);
}

#[test]
fn vagrant_takes_a_curse_ruins_shelter_or_victory_card() {
    let mut g = setup(&[Card::Vagrant]);
    hand(&mut g, P0, &[Card::Vagrant]);
    deck(&mut g, P0, &[Card::Copper, Card::Estate, Card::Silver]);
    g.start();
    play(&mut g, Card::Vagrant);
    assert_eq!(hand_kinds(&g, P0), [Card::Copper, Card::Estate]);

    let mut g = setup(&[Card::Vagrant]);
    hand(&mut g, P0, &[Card::Vagrant]);
    deck(&mut g, P0, &[Card::Copper, Card::Silver]);
    g.start();
    play(&mut g, Card::Vagrant);
    assert_eq!(hand_kinds(&g, P0), [Card::Copper]);
    assert_eq!(deck_kinds(&g, P0), [Card::Silver]);

    let mut g = setup(&[Card::Vagrant]);
    hand(&mut g, P0, &[Card::Vagrant]);
    deck(&mut g, P0, &[Card::Copper, Card::Necropolis]);
    g.start();
    play(&mut g, Card::Vagrant);
    assert_eq!(hand_kinds(&g, P0), [Card::Copper, Card::Necropolis]);
}

#[test]
fn forager_pays_per_differently_named_treasure_in_the_trash() {
    let mut g = setup(&[Card::Forager]);
    hand(&mut g, P0, &[Card::Forager, Card::Copper, Card::Estate]);
    empty_trash(&mut g);
    g.create(Card::Copper, Zone::Trash);
    g.create(Card::Silver, Zone::Trash);
    g.start();
    play(&mut g, Card::Forager);
    choose_kinds(&mut g, &[Card::Copper]);
    assert_eq!(g.turn().coins, 2);
    assert_eq!((g.turn().actions, g.turn().buys), (1, 2));

    let mut g = setup(&[Card::Forager]);
    hand(&mut g, P0, &[Card::Forager, Card::Estate, Card::Gold]);
    empty_trash(&mut g);
    g.start();
    play(&mut g, Card::Forager);
    choose_kinds(&mut g, &[Card::Gold]);
    assert_eq!(g.turn().coins, 1);
}

#[test]
fn hermit_trashes_from_the_discard_pile_and_gains_up_to_three() {
    let mut g = setup(&[Card::Hermit, Card::Sage]);
    hand(&mut g, P0, &[Card::Hermit, Card::Copper]);
    discard(&mut g, P0, &[Card::Estate, Card::Silver]);
    g.start();
    play(&mut g, Card::Hermit);
    // Non-Treasures from the discard pile or the hand.
    assert_eq!(kinds(&g, &candidates(&g)), [Card::Estate]);
    choose_kinds(&mut g, &[Card::Estate]);
    choose_pile(&mut g, Card::Sage);
    assert_eq!(trash_kinds(&g).last(), Some(&Card::Estate));
    assert_eq!(g.count_owned(P0, Card::Sage), 1);
}

#[test]
fn hermit_is_exchanged_for_a_madman_if_nothing_was_gained_in_the_buy_phase() {
    let mut g = setup(&[Card::Hermit, Card::Sage]);
    hand(&mut g, P0, &[Card::Hermit]);
    discard(&mut g, P0, &[]);
    g.start();
    play(&mut g, Card::Hermit);
    choose_pile(&mut g, Card::Sage);
    // Gaining in the Action phase doesn't count.
    assert_eq!(g.turn().phase, Phase::Buy);
    end_phase(&mut g);
    assert_eq!(g.count_owned(P0, Card::Hermit), 0);
    assert_eq!(g.count_owned(P0, Card::Madman), 1);
    assert_eq!(pile_len(&g, Card::Hermit), 11);
    assert_eq!(pile_len(&g, Card::Madman), 9);

    let mut g = setup(&[Card::Hermit, Card::Sage]);
    hand(&mut g, P0, &[Card::Hermit]);
    g.start();
    play(&mut g, Card::Hermit);
    choose_pile(&mut g, Card::Sage);
    buy(&mut g, Card::Copper);
    assert_eq!(g.count_owned(P0, Card::Hermit), 1);
    assert_eq!(g.count_owned(P0, Card::Madman), 0);
}

/// Played in the Buy phase (Market Towns), Hermit's own gain is a gain in it.
#[test]
fn hermit_played_in_the_buy_phase_counts_the_card_it_gains() {
    let mut g = setup_with(GameConfig {
        landscapes: vec![Card::MarketTowns],
        ..GameConfig::new(2, vec![Card::Hermit, Card::Underling], 7)
    });
    g.players[0].favors = 1;
    hand(&mut g, P0, &[Card::Hermit]);
    discard(&mut g, P0, &[]);
    g.start();
    end_phase(&mut g);
    trigger(&mut g, 0);
    choose_kinds(&mut g, &[Card::Hermit]);
    choose_pile(&mut g, Card::Silver);
    assert_eq!(g.turn().phase, Phase::Buy);
    end_phase(&mut g);
    assert_eq!(g.count_owned(P0, Card::Hermit), 1);
    assert_eq!(g.count_owned(P0, Card::Madman), 0);
}

#[test]
fn hermit_is_exchanged_after_a_second_buy_phase_without_gains() {
    let mut g = setup(&[Card::Hermit, Card::Villa]);
    hand(&mut g, P0, &[Card::Hermit, Card::Gold, Card::Copper]);
    discard(&mut g, P0, &[]);
    g.start();
    end_phase(&mut g);
    play(&mut g, Card::Gold);
    play(&mut g, Card::Copper);
    // The first Buy phase gains a Villa, which returns the turn to its Action phase.
    buy(&mut g, Card::Villa);
    play(&mut g, Card::Hermit);
    choose(&mut g, &[]);
    choose_pile(&mut g, Card::Silver);
    end_phase(&mut g);
    assert_eq!(g.turn().player, P1);
    assert_eq!(g.count_owned(P0, Card::Madman), 1);
}

#[test]
fn a_throne_roomed_hermit_exchanges_once() {
    let mut g = setup(&[Card::Hermit, Card::ThroneRoom, Card::Sage]);
    hand(&mut g, P0, &[Card::ThroneRoom, Card::Hermit]);
    discard(&mut g, P0, &[]);
    g.start();
    play(&mut g, Card::ThroneRoom);
    choose_kinds(&mut g, &[Card::Hermit]);
    choose_pile(&mut g, Card::Sage);
    choose(&mut g, &[]);
    choose_pile(&mut g, Card::Sage);
    end_phase(&mut g);
    assert_eq!(g.count_owned(P0, Card::Madman), 1);
    assert_eq!(pile_len(&g, Card::Madman), 9);
}

#[test]
fn madman_returns_itself_and_draws_a_card_per_card_in_hand() {
    let mut g = setup(&[Card::Hermit]);
    hand(
        &mut g,
        P0,
        &[Card::Madman, Card::Copper, Card::Copper, Card::Copper],
    );
    deck(&mut g, P0, &[Card::Estate; 5]);
    g.start();
    play(&mut g, Card::Madman);
    assert_eq!(g.hand(P0).len(), 6);
    assert_eq!(g.turn().actions, 2);
    assert_eq!(pile_len(&g, Card::Madman), 11);
}

#[test]
fn a_throne_roomed_madman_returns_and_draws_only_once() {
    let mut g = setup(&[Card::Hermit, Card::ThroneRoom]);
    hand(&mut g, P0, &[Card::ThroneRoom, Card::Madman, Card::Copper]);
    deck(&mut g, P0, &[Card::Estate; 5]);
    g.start();
    play(&mut g, Card::ThroneRoom);
    choose_kinds(&mut g, &[Card::Madman]);
    assert_eq!(g.hand(P0).len(), 2);
    assert_eq!(g.turn().actions, 4);
}

#[test]
fn market_square_is_discarded_for_a_gold_when_your_card_is_trashed() {
    let mut g = setup(&[Card::MarketSquare, Card::JunkDealer]);
    hand(
        &mut g,
        P0,
        &[
            Card::JunkDealer,
            Card::MarketSquare,
            Card::MarketSquare,
            Card::Estate,
        ],
    );
    deck(&mut g, P0, &[Card::Copper]);
    g.start();
    play(&mut g, Card::JunkDealer);
    choose_kinds(&mut g, &[Card::Estate]);
    assert_eq!(purpose(&g), Purpose::Trigger);
    trigger(&mut g, 0);
    // The other Market Square may react to the same trash too; decline it.
    assert_eq!(purpose(&g), Purpose::Trigger);
    no(&mut g);
    assert_eq!(discard_kinds(&g, P0), [Card::MarketSquare, Card::Gold]);
    assert_eq!(g.count_owned(P0, Card::MarketSquare), 2);
}

#[test]
fn market_square_reacts_to_an_attack_trashing_your_card() {
    let mut g = setup(&[Card::MarketSquare, Card::Knights]);
    hand(&mut g, P0, &[Card::SirBailey]);
    deck(&mut g, P0, &[Card::Copper]);
    hand(&mut g, P1, &[Card::MarketSquare]);
    deck(&mut g, P1, &[Card::Silver, Card::Copper]);
    g.start();
    play(&mut g, Card::SirBailey);
    assert_eq!(g.pending().unwrap().player, P1);
    trigger(&mut g, 0);
    assert_eq!(g.count_owned(P1, Card::Gold), 1);
}

#[test]
fn sage_takes_the_first_card_costing_three_or_more() {
    let mut g = setup(&[Card::Sage]);
    hand(&mut g, P0, &[Card::Sage]);
    deck(
        &mut g,
        P0,
        &[Card::Copper, Card::Estate, Card::Silver, Card::Gold],
    );
    g.start();
    play(&mut g, Card::Sage);
    assert_eq!(hand_kinds(&g, P0), [Card::Silver]);
    assert_eq!(discard_kinds(&g, P0), [Card::Copper, Card::Estate]);
    assert_eq!(deck_kinds(&g, P0), [Card::Gold]);
}

#[test]
fn sage_finding_nothing_discards_everything() {
    let mut g = setup(&[Card::Sage]);
    hand(&mut g, P0, &[Card::Sage]);
    deck(&mut g, P0, &[Card::Copper, Card::Estate]);
    discard(&mut g, P0, &[]);
    g.start();
    play(&mut g, Card::Sage);
    assert!(g.hand(P0).is_empty());
    assert_eq!(g.player(P0).discard.len(), 2);
}

#[test]
fn storeroom_discards_to_draw_then_discards_for_coins() {
    let mut g = setup(&[Card::Storeroom]);
    hand(
        &mut g,
        P0,
        &[Card::Storeroom, Card::Estate, Card::Estate, Card::Copper],
    );
    deck(&mut g, P0, &[Card::Silver, Card::Silver]);
    g.start();
    play(&mut g, Card::Storeroom);
    choose_kinds(&mut g, &[Card::Estate, Card::Estate]);
    assert_eq!(
        hand_kinds(&g, P0),
        [Card::Copper, Card::Silver, Card::Silver]
    );
    choose_kinds(&mut g, &[Card::Copper]);
    assert_eq!(g.turn().coins, 1);
    assert_eq!(g.turn().buys, 2);
}

#[test]
fn urchin_makes_others_discard_to_four() {
    let mut g = setup(&[Card::Urchin]);
    hand(&mut g, P0, &[Card::Urchin]);
    deck(&mut g, P0, &[Card::Copper]);
    hand(
        &mut g,
        P1,
        &[
            Card::Copper,
            Card::Copper,
            Card::Estate,
            Card::Estate,
            Card::Silver,
        ],
    );
    g.start();
    play(&mut g, Card::Urchin);
    assert_eq!(g.pending().unwrap().player, P1);
    choose_kinds(&mut g, &[Card::Estate]);
    assert_eq!(g.hand(P1).len(), 4);
    assert_eq!(g.turn().actions, 1);
}

#[test]
fn urchin_in_play_is_trashed_for_a_mercenary_when_another_attack_is_played() {
    let mut g = setup(&[Card::Urchin, Card::Marauder]);
    hand(&mut g, P0, &[Card::Urchin, Card::Marauder]);
    deck(&mut g, P0, &[Card::Copper]);
    hand(&mut g, P1, &[Card::Copper; 3]);
    g.start();
    play(&mut g, Card::Urchin);
    play(&mut g, Card::Marauder);
    assert_eq!(purpose(&g), Purpose::Trigger);
    trigger(&mut g, 0);
    assert_eq!(g.count_owned(P0, Card::Urchin), 0);
    assert_eq!(g.count_owned(P0, Card::Mercenary), 1);
    assert_eq!(g.count_owned(P0, Card::Spoils), 1);
}

#[test]
fn an_urchin_played_twice_cannot_trash_itself() {
    let mut g = setup(&[Card::Urchin, Card::ThroneRoom]);
    hand(&mut g, P0, &[Card::ThroneRoom, Card::Urchin]);
    deck(&mut g, P0, &[Card::Copper; 3]);
    hand(&mut g, P1, &[Card::Copper; 3]);
    g.start();
    play(&mut g, Card::ThroneRoom);
    choose_kinds(&mut g, &[Card::Urchin]);
    assert_eq!(g.count_owned(P0, Card::Urchin), 1);
    assert_eq!(g.count_owned(P0, Card::Mercenary), 0);
}

#[test]
fn mercenary_trashes_two_for_cards_coins_and_a_discard_attack() {
    let mut g = setup(&[Card::Urchin]);
    hand(
        &mut g,
        P0,
        &[Card::Mercenary, Card::Estate, Card::Estate, Card::Copper],
    );
    deck(&mut g, P0, &[Card::Silver, Card::Silver]);
    hand(
        &mut g,
        P1,
        &[
            Card::Copper,
            Card::Copper,
            Card::Copper,
            Card::Estate,
            Card::Estate,
        ],
    );
    g.start();
    play(&mut g, Card::Mercenary);
    yes(&mut g);
    choose_kinds(&mut g, &[Card::Estate, Card::Estate]);
    assert_eq!(
        hand_kinds(&g, P0),
        [Card::Copper, Card::Silver, Card::Silver]
    );
    assert_eq!(g.turn().coins, 2);
    assert_eq!(g.pending().unwrap().player, P1);
    choose_kinds(&mut g, &[Card::Estate, Card::Estate]);
    assert_eq!(g.hand(P1).len(), 3);
}

#[test]
fn mercenary_trashing_a_single_card_gets_nothing() {
    let mut g = setup(&[Card::Urchin]);
    hand(&mut g, P0, &[Card::Mercenary, Card::Estate]);
    deck(&mut g, P0, &[Card::Silver, Card::Silver]);
    g.start();
    play(&mut g, Card::Mercenary);
    yes(&mut g);
    assert!(g.hand(P0).is_empty());
    assert_eq!(g.turn().coins, 0);
}

// ---- $4 ----

#[test]
fn armory_gains_a_card_up_to_four_onto_the_deck() {
    let mut g = setup(&[Card::Armory]);
    hand(&mut g, P0, &[Card::Armory]);
    deck(&mut g, P0, &[Card::Copper]);
    g.start();
    play(&mut g, Card::Armory);
    choose_pile(&mut g, Card::Silver);
    assert_eq!(deck_kinds(&g, P0), [Card::Silver, Card::Copper]);
}

#[test]
fn death_cart_trashes_an_action_or_itself_for_five() {
    let mut g = setup(&[Card::DeathCart, Card::Sage]);
    hand(&mut g, P0, &[Card::DeathCart, Card::Sage]);
    g.start();
    play(&mut g, Card::DeathCart);
    choose_kinds(&mut g, &[Card::Sage]);
    assert_eq!(g.turn().coins, 5);
    assert_eq!(kinds(&g, g.in_play(P0)), [Card::DeathCart]);

    let mut g = setup(&[Card::DeathCart]);
    hand(&mut g, P0, &[Card::DeathCart]);
    g.start();
    play(&mut g, Card::DeathCart);
    choose_kinds(&mut g, &[Card::DeathCart]);
    assert_eq!(g.turn().coins, 5);
    assert!(g.in_play(P0).is_empty());

    let mut g = setup(&[Card::DeathCart]);
    hand(&mut g, P0, &[Card::DeathCart]);
    g.start();
    play(&mut g, Card::DeathCart);
    choose(&mut g, &[]);
    assert_eq!(g.turn().coins, 0);
}

#[test]
fn gaining_a_death_cart_gains_two_ruins() {
    let mut g = setup(&[Card::DeathCart]);
    hand(&mut g, P0, &[Card::Gold, Card::Copper]);
    g.start();
    play(&mut g, Card::Gold);
    play(&mut g, Card::Copper);
    buy(&mut g, Card::DeathCart);
    assert_eq!(g.count_owned(P0, Card::DeathCart), 1);
    assert_eq!(owned_where(&g, P0, Types::RUINS), 2);
    assert_eq!(pile_len(&g, Card::Ruins), 8);
}

#[test]
fn feodum_is_worth_a_point_per_three_silvers_and_gains_three_when_trashed() {
    let mut g = setup(&[Card::Feodum, Card::JunkDealer]);
    hand(&mut g, P0, &[Card::JunkDealer, Card::Feodum, Card::Feodum]);
    deck(&mut g, P0, &[Card::Copper]);
    discard(&mut g, P0, &[Card::Silver; 7]);
    let base = g.score(P0);
    assert_eq!(base, 4);
    g.start();
    play(&mut g, Card::JunkDealer);
    choose_kinds(&mut g, &[Card::Feodum]);
    assert_eq!(g.count_owned(P0, Card::Silver), 10);
    assert_eq!(g.score(P0), 3);
}

#[test]
fn fortress_returns_to_hand_when_trashed() {
    let mut g = setup(&[Card::Fortress, Card::JunkDealer]);
    hand(&mut g, P0, &[Card::JunkDealer, Card::Fortress]);
    deck(&mut g, P0, &[Card::Copper]);
    g.start();
    play(&mut g, Card::JunkDealer);
    choose_kinds(&mut g, &[Card::Fortress]);
    assert!(hand_kinds(&g, P0).contains(&Card::Fortress));
    assert!(!trash_kinds(&g).contains(&Card::Fortress));
}

#[test]
fn ironmonger_gives_a_bonus_per_type_of_the_revealed_card() {
    let mut g = setup(&[Card::Ironmonger]);
    hand(&mut g, P0, &[Card::Ironmonger]);
    deck(&mut g, P0, &[Card::Copper, Card::Silver]);
    g.start();
    play(&mut g, Card::Ironmonger);
    no(&mut g);
    assert_eq!(g.turn().coins, 1);
    assert_eq!(deck_kinds(&g, P0), [Card::Silver]);

    let mut g = setup(&[Card::Ironmonger]);
    hand(&mut g, P0, &[Card::Ironmonger]);
    deck(&mut g, P0, &[Card::Copper, Card::Estate, Card::Gold]);
    g.start();
    play(&mut g, Card::Ironmonger);
    yes(&mut g);
    assert_eq!(hand_kinds(&g, P0), [Card::Copper, Card::Gold]);
    assert_eq!(discard_kinds(&g, P0), [Card::Estate]);

    let mut g = setup(&[Card::Ironmonger]);
    hand(&mut g, P0, &[Card::Ironmonger]);
    deck(&mut g, P0, &[Card::Copper, Card::Ironmonger]);
    g.start();
    play(&mut g, Card::Ironmonger);
    no(&mut g);
    assert_eq!(g.turn().actions, 2);
}

#[test]
fn marauder_gains_a_spoils_and_gives_ruins_and_moat_blocks_it() {
    let mut g = setup_with(GameConfig {
        shelters: Some(false),
        ..GameConfig::new(3, vec![Card::Marauder, Card::Moat], 7)
    });
    hand(&mut g, P0, &[Card::Marauder]);
    hand(&mut g, P1, &[Card::Moat]);
    hand(&mut g, P2, &[Card::Copper]);
    g.start();
    play(&mut g, Card::Marauder);
    trigger(&mut g, 0);
    assert_eq!(g.count_owned(P0, Card::Spoils), 1);
    assert_eq!(owned_where(&g, P1, Types::RUINS), 0);
    assert_eq!(owned_where(&g, P2, Types::RUINS), 1);
}

#[test]
fn procession_plays_twice_trashes_and_gains_an_action_costing_one_more() {
    let mut g = setup(&[Card::Procession, Card::Vagrant, Card::Sage]);
    hand(&mut g, P0, &[Card::Procession, Card::Vagrant]);
    deck(&mut g, P0, &[Card::Copper; 4]);
    g.start();
    play(&mut g, Card::Procession);
    choose_kinds(&mut g, &[Card::Vagrant]);
    assert_eq!(g.hand(P0).len(), 2);
    assert_eq!(g.turn().actions, 2);
    assert_eq!(trash_kinds(&g).last(), Some(&Card::Vagrant));
    assert_eq!(g.count_owned(P0, Card::Sage), 1);
}

#[test]
fn procession_resolves_the_trashed_cards_ability_before_the_gain() {
    let mut g = setup(&[
        Card::Procession,
        Card::Cultist,
        Card::Altar,
        Card::HuntingGrounds,
    ]);
    hand(&mut g, P0, &[Card::Procession, Card::Cultist]);
    deck(&mut g, P0, &[Card::Copper; 10]);
    g.start();
    play(&mut g, Card::Procession);
    choose_kinds(&mut g, &[Card::Cultist]);
    // +2 Cards twice, then +3 Cards for the trash, before choosing the gain.
    assert_eq!(g.hand(P0).len(), 7);
    assert_eq!(purpose(&g), Purpose::Gain);
    choose_pile(&mut g, Card::Altar);
    assert_eq!(g.count_owned(P0, Card::Altar), 1);
    assert_eq!(owned_where(&g, P1, Types::RUINS), 2);
}

#[test]
fn procession_on_a_madman_cannot_trash_it_but_still_gains() {
    let mut g = setup(&[Card::Procession, Card::Hermit, Card::Vagrant]);
    hand(&mut g, P0, &[Card::Procession, Card::Madman]);
    deck(&mut g, P0, &[Card::Copper; 4]);
    g.start();
    play(&mut g, Card::Procession);
    choose_kinds(&mut g, &[Card::Madman]);
    // Madman costs $0, so an Action costing exactly $1: none in this kingdom.
    assert_eq!(pile_len(&g, Card::Madman), 11);
    assert!(!trash_kinds(&g).contains(&Card::Madman));
    assert_eq!(g.turn().actions, 4);
}

#[test]
fn rats_gains_a_rats_and_trashes_a_non_rats_card() {
    let mut g = setup(&[Card::Rats]);
    hand(&mut g, P0, &[Card::Rats, Card::Rats, Card::Estate]);
    deck(&mut g, P0, &[Card::Copper]);
    g.start();
    play(&mut g, Card::Rats);
    assert_eq!(kinds(&g, &candidates(&g)), [Card::Estate, Card::Copper]);
    choose_kinds(&mut g, &[Card::Estate]);
    assert_eq!(g.count_owned(P0, Card::Rats), 3);

    let mut g = setup(&[Card::Rats]);
    hand(&mut g, P0, &[Card::Rats, Card::Rats]);
    deck(&mut g, P0, &[Card::Rats]);
    g.start();
    play(&mut g, Card::Rats);
    assert_eq!(hand_kinds(&g, P0), [Card::Rats, Card::Rats]);
}

#[test]
fn trashed_rats_and_overgrown_estate_draw_a_card() {
    for card in [Card::Rats, Card::OvergrownEstate] {
        let mut g = setup(&[Card::Rats, Card::JunkDealer]);
        hand(&mut g, P0, &[Card::JunkDealer, card]);
        deck(&mut g, P0, &[Card::Copper, Card::Gold]);
        g.start();
        play(&mut g, Card::JunkDealer);
        choose_kinds(&mut g, &[card]);
        assert_eq!(hand_kinds(&g, P0), [Card::Copper, Card::Gold], "{card}");
    }
}

#[test]
fn scavenger_may_dump_the_deck_and_puts_a_discarded_card_on_top() {
    let mut g = setup(&[Card::Scavenger]);
    hand(&mut g, P0, &[Card::Scavenger]);
    deck(&mut g, P0, &[Card::Copper, Card::Estate]);
    discard(&mut g, P0, &[Card::Gold]);
    g.start();
    play(&mut g, Card::Scavenger);
    assert_eq!(g.turn().coins, 2);
    yes(&mut g);
    choose_kinds(&mut g, &[Card::Gold]);
    assert_eq!(deck_kinds(&g, P0), [Card::Gold]);
    assert_eq!(g.player(P0).discard.len(), 2);
}

#[test]
fn wandering_minstrel_puts_actions_back_in_order_and_discards_the_rest() {
    let mut g = setup(&[Card::WanderingMinstrel, Card::Sage, Card::Vagrant]);
    hand(&mut g, P0, &[Card::WanderingMinstrel]);
    deck(
        &mut g,
        P0,
        &[
            Card::Copper,
            Card::Sage,
            Card::Estate,
            Card::Vagrant,
            Card::Gold,
        ],
    );
    g.start();
    play(&mut g, Card::WanderingMinstrel);
    assert_eq!(g.turn().actions, 2);
    choose_kinds(&mut g, &[Card::Vagrant, Card::Sage]);
    assert_eq!(deck_kinds(&g, P0), [Card::Vagrant, Card::Sage, Card::Gold]);
    assert_eq!(discard_kinds(&g, P0), [Card::Estate]);
}

// ---- $5 ----

#[test]
fn band_of_misfits_plays_a_cheaper_action_from_the_supply_leaving_it_there() {
    let mut g = setup(&[Card::BandOfMisfits, Card::Fortress, Card::Altar, Card::Sage]);
    hand(&mut g, P0, &[Card::BandOfMisfits]);
    deck(&mut g, P0, &[Card::Copper; 3]);
    g.start();
    play(&mut g, Card::BandOfMisfits);
    let d = g.pending().unwrap();
    let DecisionKind::Pile { piles, .. } = &d.kind else {
        panic!("expected a pile choice, got {d:?}")
    };
    // Fortress and Sage; not Altar ($6) or Band of Misfits itself.
    assert_eq!(piles.len(), 2);
    choose_pile(&mut g, Card::Fortress);
    assert_eq!(g.hand(P0).len(), 1);
    assert_eq!(g.turn().actions, 2);
    assert_eq!(pile_len(&g, Card::Fortress), 10);
    assert_eq!(kinds(&g, g.in_play(P0)), [Card::BandOfMisfits]);
    assert_eq!(g.turn().played, [Card::BandOfMisfits, Card::Fortress]);
}

#[test]
fn band_of_misfits_playing_death_cart_cannot_trash_the_supply_card() {
    let mut g = setup(&[Card::BandOfMisfits, Card::DeathCart, Card::Fortress]);
    hand(&mut g, P0, &[Card::BandOfMisfits, Card::Fortress]);
    g.start();
    play(&mut g, Card::BandOfMisfits);
    choose_pile(&mut g, Card::DeathCart);
    assert_eq!(kinds(&g, &candidates(&g)), [Card::Fortress]);
    choose_kinds(&mut g, &[Card::Fortress]);
    assert_eq!(g.turn().coins, 5);
    assert_eq!(pile_len(&g, Card::DeathCart), 10);
    // Fortress came back from the trash.
    assert_eq!(hand_kinds(&g, P0), [Card::Fortress]);
}

#[test]
fn bandit_camp_gains_a_spoils() {
    let mut g = setup(&[Card::BanditCamp]);
    hand(&mut g, P0, &[Card::BanditCamp]);
    deck(&mut g, P0, &[Card::Copper]);
    g.start();
    play(&mut g, Card::BanditCamp);
    assert_eq!(g.turn().actions, 2);
    assert_eq!(hand_kinds(&g, P0), [Card::Copper]);
    assert_eq!(discard_kinds(&g, P0), [Card::Spoils]);
    assert_eq!(pile_len(&g, Card::Spoils), 14);
}

#[test]
fn spoils_gives_three_and_returns_to_its_pile() {
    let mut g = setup(&[Card::BanditCamp]);
    hand(&mut g, P0, &[Card::Spoils]);
    g.start();
    play(&mut g, Card::Spoils);
    assert_eq!(g.turn().coins, 3);
    assert_eq!(pile_len(&g, Card::Spoils), 16);
    assert!(g.in_play(P0).is_empty());
}

#[test]
fn catacombs_takes_the_three_or_discards_them_for_three_more() {
    let mut g = setup(&[Card::Catacombs]);
    hand(&mut g, P0, &[Card::Catacombs]);
    deck(
        &mut g,
        P0,
        &[
            Card::Copper,
            Card::Silver,
            Card::Gold,
            Card::Estate,
            Card::Duchy,
            Card::Province,
        ],
    );
    g.start();
    play(&mut g, Card::Catacombs);
    mode(&mut g, 0);
    assert_eq!(hand_kinds(&g, P0), [Card::Copper, Card::Silver, Card::Gold]);

    let mut g = setup(&[Card::Catacombs]);
    hand(&mut g, P0, &[Card::Catacombs]);
    deck(
        &mut g,
        P0,
        &[
            Card::Copper,
            Card::Silver,
            Card::Gold,
            Card::Estate,
            Card::Duchy,
            Card::Province,
        ],
    );
    g.start();
    play(&mut g, Card::Catacombs);
    mode(&mut g, 1);
    assert_eq!(
        hand_kinds(&g, P0),
        [Card::Estate, Card::Duchy, Card::Province]
    );
    assert_eq!(g.player(P0).discard.len(), 3);
}

#[test]
fn trashed_catacombs_gains_a_cheaper_card() {
    let mut g = setup(&[Card::Catacombs, Card::JunkDealer]);
    hand(&mut g, P0, &[Card::JunkDealer, Card::Catacombs]);
    deck(&mut g, P0, &[Card::Copper]);
    g.start();
    play(&mut g, Card::JunkDealer);
    choose_kinds(&mut g, &[Card::Catacombs]);
    let d = g.pending().unwrap();
    let DecisionKind::Pile { piles, .. } = &d.kind else {
        panic!("expected a pile choice, got {d:?}")
    };
    assert!(!piles.contains(&g.supply_pile(Card::JunkDealer).unwrap()));
    assert!(!piles.contains(&g.supply_pile(Card::Catacombs).unwrap()));
    choose_pile(&mut g, Card::Silver);
    assert_eq!(g.count_owned(P0, Card::Silver), 1);
}

#[test]
fn count_makes_two_choices() {
    let mut g = setup(&[Card::Count]);
    hand(
        &mut g,
        P0,
        &[Card::Count, Card::Estate, Card::Estate, Card::Copper],
    );
    g.start();
    play(&mut g, Card::Count);
    mode(&mut g, 0);
    choose_kinds(&mut g, &[Card::Estate, Card::Estate]);
    mode(&mut g, 2);
    assert_eq!(
        discard_kinds(&g, P0),
        [Card::Estate, Card::Estate, Card::Duchy]
    );

    let mut g = setup(&[Card::Count]);
    hand(&mut g, P0, &[Card::Count, Card::Gold, Card::Estate]);
    g.start();
    play(&mut g, Card::Count);
    mode(&mut g, 1);
    choose_kinds(&mut g, &[Card::Gold]);
    mode(&mut g, 1);
    assert_eq!(deck_kinds(&g, P0)[0], Card::Gold);
    assert!(g.hand(P0).is_empty());
    assert_eq!(trash_kinds(&g).last(), Some(&Card::Estate));

    let mut g = setup(&[Card::Count]);
    hand(&mut g, P0, &[Card::Count]);
    g.start();
    play(&mut g, Card::Count);
    mode(&mut g, 2);
    mode(&mut g, 0);
    assert_eq!(g.turn().coins, 3);
    assert_eq!(discard_kinds(&g, P0), [Card::Copper]);
}

#[test]
fn cards_trashed_together_are_all_trashed_before_market_square_could_react() {
    let mut g = setup(&[Card::Count, Card::MarketSquare, Card::Fortress]);
    hand(
        &mut g,
        P0,
        &[
            Card::Count,
            Card::Estate,
            Card::MarketSquare,
            Card::Fortress,
        ],
    );
    g.start();
    play(&mut g, Card::Count);
    mode(&mut g, 2);
    mode(&mut g, 1);
    // No Market Square offer: it was trashed along with the Estate. Fortress comes back.
    assert_eq!(hand_kinds(&g, P0), [Card::Fortress]);
    assert!(trash_kinds(&g).ends_with(&[Card::Estate, Card::MarketSquare]));
}

#[test]
fn counterfeit_plays_a_treasure_twice_and_trashes_it() {
    let mut g = setup(&[Card::Counterfeit]);
    hand(&mut g, P0, &[Card::Counterfeit, Card::Silver]);
    g.start();
    play(&mut g, Card::Counterfeit);
    choose_kinds(&mut g, &[Card::Silver]);
    assert_eq!(g.turn().coins, 5);
    assert_eq!(g.turn().buys, 2);
    assert_eq!(trash_kinds(&g).last(), Some(&Card::Silver));
}

#[test]
fn counterfeit_on_spoils_pays_twice_but_cannot_trash_it() {
    let mut g = setup(&[Card::Counterfeit, Card::BanditCamp]);
    hand(&mut g, P0, &[Card::Counterfeit, Card::Spoils]);
    g.start();
    play(&mut g, Card::Counterfeit);
    choose_kinds(&mut g, &[Card::Spoils]);
    assert_eq!(g.turn().coins, 7);
    assert_eq!(pile_len(&g, Card::Spoils), 16);
    assert!(!trash_kinds(&g).contains(&Card::Spoils));
}

#[test]
fn cultist_gives_ruins_and_may_chain_another_cultist_for_free() {
    let mut g = setup(&[Card::Cultist]);
    hand(&mut g, P0, &[Card::Cultist, Card::Cultist]);
    deck(&mut g, P0, &[Card::Copper; 6]);
    g.start();
    play(&mut g, Card::Cultist);
    assert_eq!(owned_where(&g, P1, Types::RUINS), 1);
    choose_kinds(&mut g, &[Card::Cultist]);
    assert_eq!(owned_where(&g, P1, Types::RUINS), 2);
    assert_eq!(hand_kinds(&g, P0), [Card::Copper; 4]);
    assert_eq!(g.turn().actions, 0);
}

#[test]
fn a_cultist_trashed_by_a_knight_draws_but_not_the_revealed_card() {
    let mut g = setup(&[Card::Cultist, Card::Knights]);
    hand(&mut g, P0, &[Card::SirBailey]);
    deck(&mut g, P0, &[Card::Copper]);
    hand(&mut g, P1, &[]);
    deck(
        &mut g,
        P1,
        &[
            Card::Cultist,
            Card::Silver,
            Card::Copper,
            Card::Copper,
            Card::Copper,
        ],
    );
    g.start();
    play(&mut g, Card::SirBailey);
    assert_eq!(g.pending().unwrap().player, P1);
    choose_kinds(&mut g, &[Card::Cultist]);
    assert_eq!(hand_kinds(&g, P1), [Card::Copper; 3]);
    assert_eq!(discard_kinds(&g, P1), [Card::Silver]);
}

#[test]
fn graverobber_gains_from_the_trash_onto_the_deck() {
    let mut g = setup(&[Card::Graverobber]);
    hand(&mut g, P0, &[Card::Graverobber]);
    deck(&mut g, P0, &[Card::Copper]);
    empty_trash(&mut g);
    g.create(Card::Gold, Zone::Trash);
    g.create(Card::Province, Zone::Trash);
    g.create(Card::Estate, Zone::Trash);
    g.start();
    play(&mut g, Card::Graverobber);
    mode(&mut g, 0);
    assert_eq!(deck_kinds(&g, P0), [Card::Gold, Card::Copper]);
}

#[test]
fn graverobber_upgrades_an_action_by_up_to_three() {
    let mut g = setup(&[Card::Graverobber, Card::Vagrant, Card::Altar]);
    hand(
        &mut g,
        P0,
        &[Card::Graverobber, Card::Vagrant, Card::Copper],
    );
    g.start();
    play(&mut g, Card::Graverobber);
    mode(&mut g, 1);
    let d = g.pending().unwrap();
    let DecisionKind::Pile { piles, .. } = &d.kind else {
        panic!("expected a pile choice, got {d:?}")
    };
    assert!(piles.contains(&g.supply_pile(Card::Graverobber).unwrap()));
    assert!(!piles.contains(&g.supply_pile(Card::Altar).unwrap()));
    choose_pile(&mut g, Card::Graverobber);
    assert_eq!(g.count_owned(P0, Card::Graverobber), 2);
    assert_eq!(trash_kinds(&g).last(), Some(&Card::Vagrant));
}

#[test]
fn junk_dealer_draws_and_trashes() {
    let mut g = setup(&[Card::JunkDealer]);
    hand(&mut g, P0, &[Card::JunkDealer, Card::Estate]);
    deck(&mut g, P0, &[Card::Copper]);
    g.start();
    play(&mut g, Card::JunkDealer);
    choose_kinds(&mut g, &[Card::Estate]);
    assert_eq!(hand_kinds(&g, P0), [Card::Copper]);
    assert_eq!((g.turn().actions, g.turn().coins), (1, 1));
}

#[test]
fn knights_trash_a_card_costing_three_to_six_and_discard_the_rest() {
    let mut g = setup(&[Card::Knights]);
    hand(&mut g, P0, &[Card::SirBailey]);
    deck(&mut g, P0, &[Card::Copper]);
    deck(&mut g, P1, &[Card::Silver, Card::Estate, Card::Copper]);
    g.start();
    play(&mut g, Card::SirBailey);
    assert_eq!(trash_kinds(&g).last(), Some(&Card::Silver));
    assert_eq!(discard_kinds(&g, P1), [Card::Estate]);
    assert_eq!(kinds(&g, g.in_play(P0)), [Card::SirBailey]);
    assert_eq!(g.turn().actions, 1);
}

#[test]
fn the_victim_chooses_which_card_the_knight_trashes() {
    let mut g = setup(&[Card::Knights]);
    hand(&mut g, P0, &[Card::DameSylvia]);
    deck(&mut g, P1, &[Card::Silver, Card::Duchy]);
    g.start();
    play(&mut g, Card::DameSylvia);
    assert_eq!(g.pending().unwrap().player, P1);
    choose_kinds(&mut g, &[Card::Duchy]);
    assert_eq!(discard_kinds(&g, P1), [Card::Silver]);
    assert_eq!(g.turn().coins, 2);
}

#[test]
fn a_knight_trashing_a_knight_is_trashed_too() {
    let mut g = setup(&[Card::Knights]);
    hand(&mut g, P0, &[Card::DameMolly]);
    deck(&mut g, P1, &[Card::DameJosephine, Card::Copper]);
    g.start();
    play(&mut g, Card::DameMolly);
    assert_eq!(
        trash_kinds(&g)[trash_kinds(&g).len() - 2..],
        [Card::DameJosephine, Card::DameMolly]
    );
    assert_eq!(g.turn().actions, 2);
}

#[test]
fn a_potion_or_debt_cost_is_not_from_three_to_six() {
    let mut g = setup(&[Card::Knights]);
    hand(&mut g, P0, &[Card::SirDestry]);
    deck(&mut g, P0, &[Card::Copper, Card::Copper]);
    deck(&mut g, P1, &[Card::Golem, Card::Engineer]);
    g.start();
    play(&mut g, Card::SirDestry);
    assert_eq!(discard_kinds(&g, P1), [Card::Golem, Card::Engineer]);
}

#[test]
fn dame_anna_trashes_up_to_two_cards_first() {
    let mut g = setup(&[Card::Knights]);
    hand(
        &mut g,
        P0,
        &[Card::DameAnna, Card::Estate, Card::Estate, Card::Copper],
    );
    deck(&mut g, P1, &[Card::Copper, Card::Copper]);
    g.start();
    play(&mut g, Card::DameAnna);
    choose_kinds(&mut g, &[Card::Estate, Card::Estate]);
    assert_eq!(hand_kinds(&g, P0), [Card::Copper]);
    assert_eq!(discard_kinds(&g, P1), [Card::Copper, Card::Copper]);
}

#[test]
fn dame_natalie_may_gain_a_card_up_to_three() {
    let mut g = setup(&[Card::Knights]);
    hand(&mut g, P0, &[Card::DameNatalie]);
    g.start();
    play(&mut g, Card::DameNatalie);
    choose_pile(&mut g, Card::Silver);
    assert_eq!(g.count_owned(P0, Card::Silver), 1);
}

#[test]
fn sir_michael_makes_others_discard_to_three_before_the_trashing() {
    let mut g = setup(&[Card::Knights]);
    hand(&mut g, P0, &[Card::SirMichael]);
    hand(
        &mut g,
        P1,
        &[
            Card::Copper,
            Card::Copper,
            Card::Copper,
            Card::Estate,
            Card::Estate,
        ],
    );
    deck(&mut g, P1, &[Card::Silver, Card::Copper]);
    g.start();
    play(&mut g, Card::SirMichael);
    choose_kinds(&mut g, &[Card::Estate, Card::Estate]);
    assert_eq!(g.hand(P1).len(), 3);
    assert_eq!(trash_kinds(&g).last(), Some(&Card::Silver));
    assert_eq!(
        discard_kinds(&g, P1),
        [Card::Estate, Card::Estate, Card::Copper]
    );
}

#[test]
fn sir_vander_trashed_gains_its_owner_a_gold() {
    let mut g = setup(&[Card::Knights]);
    hand(&mut g, P0, &[Card::SirMartin]);
    deck(&mut g, P1, &[Card::SirVander, Card::Copper]);
    g.start();
    play(&mut g, Card::SirMartin);
    assert_eq!(g.count_owned(P1, Card::Gold), 1);
    assert_eq!(g.count_owned(P0, Card::SirMartin), 0);
    assert_eq!(g.turn().buys, 3);
}

#[test]
fn dame_josephine_is_worth_two_victory_points() {
    let mut g = setup(&[Card::Knights]);
    hand(&mut g, P0, &[Card::DameJosephine]);
    deck(&mut g, P0, &[]);
    discard(&mut g, P0, &[]);
    assert_eq!(g.score(P0), 2);
}

#[test]
fn mystic_takes_the_top_card_if_named() {
    let mut g = setup(&[Card::Mystic]);
    hand(&mut g, P0, &[Card::Mystic]);
    deck(&mut g, P0, &[Card::Silver, Card::Gold]);
    g.start();
    play(&mut g, Card::Mystic);
    name(&mut g, Card::Silver);
    assert_eq!(hand_kinds(&g, P0), [Card::Silver]);
    assert_eq!((g.turn().actions, g.turn().coins), (1, 2));

    let mut g = setup(&[Card::Mystic]);
    hand(&mut g, P0, &[Card::Mystic]);
    deck(&mut g, P0, &[Card::Silver]);
    g.start();
    play(&mut g, Card::Mystic);
    name(&mut g, Card::Gold);
    assert!(g.hand(P0).is_empty());
    assert_eq!(deck_kinds(&g, P0), [Card::Silver]);
}

#[test]
fn pillage_trashes_itself_gains_spoils_and_takes_a_card_from_a_full_hand() {
    let mut g = setup(&[Card::Pillage]);
    hand(&mut g, P0, &[Card::Pillage]);
    hand(
        &mut g,
        P1,
        &[
            Card::Gold,
            Card::Copper,
            Card::Copper,
            Card::Copper,
            Card::Copper,
        ],
    );
    g.start();
    play(&mut g, Card::Pillage);
    assert_eq!(g.pending().unwrap().player, P0);
    choose_kinds(&mut g, &[Card::Gold]);
    assert_eq!(discard_kinds(&g, P1), [Card::Gold]);
    assert_eq!(g.count_owned(P0, Card::Spoils), 2);
    assert_eq!(trash_kinds(&g).last(), Some(&Card::Pillage));
}

#[test]
fn a_throne_roomed_pillage_only_works_once() {
    let mut g = setup(&[Card::Pillage, Card::ThroneRoom]);
    hand(&mut g, P0, &[Card::ThroneRoom, Card::Pillage]);
    hand(&mut g, P1, &[Card::Copper; 4]);
    g.start();
    play(&mut g, Card::ThroneRoom);
    choose_kinds(&mut g, &[Card::Pillage]);
    assert_eq!(g.count_owned(P0, Card::Spoils), 2);
}

#[test]
fn rebuild_upgrades_the_first_victory_card_not_named() {
    let mut g = setup(&[Card::Rebuild]);
    hand(&mut g, P0, &[Card::Rebuild]);
    deck(
        &mut g,
        P0,
        &[Card::Copper, Card::Province, Card::Estate, Card::Gold],
    );
    g.start();
    play(&mut g, Card::Rebuild);
    name(&mut g, Card::Province);
    choose_pile(&mut g, Card::Duchy);
    assert_eq!(trash_kinds(&g).last(), Some(&Card::Estate));
    assert_eq!(
        discard_kinds(&g, P0),
        [Card::Copper, Card::Province, Card::Duchy]
    );
    assert_eq!(deck_kinds(&g, P0), [Card::Gold]);
    assert_eq!(g.turn().actions, 1);
}

#[test]
fn rogue_gains_from_the_trash_or_else_attacks() {
    let mut g = setup(&[Card::Rogue]);
    hand(&mut g, P0, &[Card::Rogue]);
    empty_trash(&mut g);
    g.create(Card::Silver, Zone::Trash);
    g.create(Card::Copper, Zone::Trash);
    deck(&mut g, P1, &[Card::Gold, Card::Copper]);
    g.start();
    play(&mut g, Card::Rogue);
    assert_eq!(g.count_owned(P0, Card::Silver), 1);
    assert_eq!(g.count_owned(P1, Card::Gold), 1);
    assert_eq!(g.turn().coins, 2);

    let mut g = setup(&[Card::Rogue]);
    hand(&mut g, P0, &[Card::Rogue]);
    empty_trash(&mut g);
    deck(&mut g, P1, &[Card::Gold, Card::Copper]);
    g.start();
    play(&mut g, Card::Rogue);
    assert_eq!(trash_kinds(&g).last(), Some(&Card::Gold));
    assert_eq!(discard_kinds(&g, P1), [Card::Copper]);
}

// ---- $6 ----

#[test]
fn altar_trashes_and_gains_up_to_five() {
    let mut g = setup(&[Card::Altar]);
    hand(&mut g, P0, &[Card::Altar, Card::Estate]);
    g.start();
    play(&mut g, Card::Altar);
    choose_pile(&mut g, Card::Duchy);
    assert_eq!(trash_kinds(&g).last(), Some(&Card::Estate));
    assert_eq!(discard_kinds(&g, P0), [Card::Duchy]);
}

#[test]
fn an_overgrown_estate_trashed_by_altar_draws_before_the_gain() {
    let mut g = setup(&[Card::Altar]);
    hand(&mut g, P0, &[Card::Altar, Card::OvergrownEstate]);
    deck(&mut g, P0, &[Card::Gold]);
    g.start();
    play(&mut g, Card::Altar);
    assert_eq!(hand_kinds(&g, P0), [Card::Gold]);
    assert_eq!(purpose(&g), Purpose::Gain);
}

#[test]
fn trashed_hunting_grounds_gains_a_duchy_or_three_estates() {
    let mut g = setup(&[Card::HuntingGrounds, Card::JunkDealer]);
    hand(&mut g, P0, &[Card::JunkDealer, Card::HuntingGrounds]);
    deck(&mut g, P0, &[Card::Copper]);
    g.start();
    play(&mut g, Card::JunkDealer);
    choose_kinds(&mut g, &[Card::HuntingGrounds]);
    mode(&mut g, 1);
    assert_eq!(discard_kinds(&g, P0), [Card::Estate; 3]);
}

// ---- Shelters and Ruins ----

#[test]
fn hovel_may_be_trashed_when_a_victory_card_is_gained() {
    let mut g = setup_with(GameConfig {
        shelters: Some(true),
        colonies: Some(false),
        ..GameConfig::new(2, vec![Card::Sage], 7)
    });
    let hovel = hand(&mut g, P0, &[Card::Hovel, Card::Silver])[0];
    g.start();
    play(&mut g, Card::Silver);
    buy(&mut g, Card::Estate);
    assert_eq!(purpose(&g), Purpose::Trigger);
    trigger(&mut g, 0);
    assert_eq!(g.zone(hovel), Zone::Trash);
    assert_eq!(trash_kinds(&g).last(), Some(&Card::Hovel));
}

#[test]
fn survivors_discards_or_reorders_the_top_two() {
    let mut g = setup(&[Card::Marauder]);
    hand(&mut g, P0, &[Card::Survivors]);
    deck(&mut g, P0, &[Card::Estate, Card::Gold, Card::Copper]);
    g.start();
    play(&mut g, Card::Survivors);
    mode(&mut g, 1);
    choose_kinds(&mut g, &[Card::Gold, Card::Estate]);
    assert_eq!(deck_kinds(&g, P0), [Card::Gold, Card::Estate, Card::Copper]);

    let mut g = setup(&[Card::Marauder]);
    hand(&mut g, P0, &[Card::Survivors]);
    deck(&mut g, P0, &[Card::Estate, Card::Gold, Card::Copper]);
    g.start();
    play(&mut g, Card::Survivors);
    mode(&mut g, 0);
    assert_eq!(deck_kinds(&g, P0), [Card::Copper]);
    assert_eq!(discard_kinds(&g, P0), [Card::Estate, Card::Gold]);
}

#[test]
fn ruins_and_necropolis_give_their_bonuses() {
    let mut g = setup(&[Card::Marauder]);
    hand(
        &mut g,
        P0,
        &[
            Card::Necropolis,
            Card::Necropolis,
            Card::RuinedVillage,
            Card::AbandonedMine,
            Card::RuinedMarket,
            Card::RuinedLibrary,
        ],
    );
    deck(&mut g, P0, &[Card::Copper]);
    g.start();
    for card in [
        Card::Necropolis,
        Card::Necropolis,
        Card::RuinedVillage,
        Card::AbandonedMine,
        Card::RuinedLibrary,
        Card::RuinedMarket,
    ] {
        play(&mut g, card);
    }
    assert_eq!(g.turn().coins, 1);
    assert_eq!(g.turn().buys, 2);
    assert_eq!(hand_kinds(&g, P0), [Card::Copper]);
}

// ---- Rulings and interactions ----

#[test]
fn a_market_square_drawn_by_a_trash_ability_can_react_to_that_trash() {
    let mut g = setup(&[Card::MarketSquare, Card::JunkDealer]);
    hand(&mut g, P0, &[Card::JunkDealer, Card::OvergrownEstate]);
    deck(&mut g, P0, &[Card::Copper, Card::MarketSquare]);
    g.start();
    play(&mut g, Card::JunkDealer);
    choose_kinds(&mut g, &[Card::OvergrownEstate]);
    assert_eq!(purpose(&g), Purpose::Trigger);
    trigger(&mut g, 0);
    assert_eq!(g.count_owned(P0, Card::Gold), 1);
}

#[test]
fn catacombs_trashed_by_remodel_gains_before_remodel_does() {
    let mut g = setup(&[Card::Remodel, Card::Catacombs]);
    hand(&mut g, P0, &[Card::Remodel, Card::Catacombs]);
    g.start();
    play(&mut g, Card::Remodel);
    assert_eq!(g.pending().unwrap().source, Some(Card::Catacombs));
    choose_pile(&mut g, Card::Silver);
    assert_eq!(g.pending().unwrap().source, Some(Card::Remodel));
    choose_pile(&mut g, Card::Gold);
    assert_eq!(discard_kinds(&g, P0), [Card::Silver, Card::Gold]);
}

#[test]
fn hovel_reacts_to_a_victory_card_gained_from_the_trash() {
    let mut g = setup_with(GameConfig {
        shelters: Some(true),
        colonies: Some(false),
        ..GameConfig::new(2, vec![Card::Rogue], 7)
    });
    // The replaced starting hand in the trash costs at most $1.
    let hovel = hand(&mut g, P0, &[Card::Rogue, Card::Hovel])[1];
    g.create(Card::Duchy, Zone::Trash);
    g.start();
    play(&mut g, Card::Rogue);
    trigger(&mut g, 0);
    assert_eq!(g.count_owned(P0, Card::Duchy), 1);
    assert_eq!(g.zone(hovel), Zone::Trash);
}

#[test]
fn a_throne_roomed_cultist_gives_two_ruins_and_draws_four() {
    let mut g = setup(&[Card::Cultist, Card::ThroneRoom]);
    hand(&mut g, P0, &[Card::ThroneRoom, Card::Cultist]);
    deck(&mut g, P0, &[Card::Copper; 6]);
    g.start();
    play(&mut g, Card::ThroneRoom);
    choose_kinds(&mut g, &[Card::Cultist]);
    assert_eq!(g.hand(P0).len(), 4);
    assert_eq!(owned_where(&g, P1, Types::RUINS), 2);
}

#[test]
fn beggar_reaction_is_optional() {
    let mut g = setup(&[Card::Beggar, Card::Marauder]);
    hand(&mut g, P0, &[Card::Marauder]);
    hand(&mut g, P1, &[Card::Beggar]);
    g.start();
    play(&mut g, Card::Marauder);
    no(&mut g);
    assert_eq!(hand_kinds(&g, P1), [Card::Beggar]);
    assert_eq!(g.count_owned(P1, Card::Silver), 0);
}

#[test]
fn moat_protects_from_a_knight() {
    let mut g = setup(&[Card::Knights, Card::Moat]);
    hand(&mut g, P0, &[Card::SirBailey]);
    hand(&mut g, P1, &[Card::Moat]);
    deck(&mut g, P1, &[Card::Silver, Card::Gold]);
    g.start();
    play(&mut g, Card::SirBailey);
    trigger(&mut g, 0);
    assert_eq!(deck_kinds(&g, P1), [Card::Silver, Card::Gold]);
}

#[test]
fn graverobber_options_can_be_chosen_when_they_cannot_be_done() {
    let mut g = setup(&[Card::Graverobber]);
    hand(&mut g, P0, &[Card::Graverobber, Card::Copper]);
    empty_trash(&mut g);
    g.start();
    play(&mut g, Card::Graverobber);
    mode(&mut g, 0);
    assert_eq!(g.turn().phase, Phase::Buy);

    let mut g = setup(&[Card::Graverobber]);
    hand(&mut g, P0, &[Card::Graverobber, Card::Copper]);
    g.start();
    play(&mut g, Card::Graverobber);
    mode(&mut g, 1);
    assert_eq!(hand_kinds(&g, P0), [Card::Copper]);
}

#[test]
fn a_hermit_that_left_play_is_not_exchanged() {
    let mut g = setup(&[Card::Hermit, Card::Procession, Card::Sage]);
    hand(&mut g, P0, &[Card::Procession, Card::Hermit]);
    discard(&mut g, P0, &[]);
    g.start();
    play(&mut g, Card::Procession);
    choose_kinds(&mut g, &[Card::Hermit]);
    choose_pile(&mut g, Card::Sage);
    choose(&mut g, &[]);
    choose_pile(&mut g, Card::Sage);
    // Procession trashed it; no Action costs exactly $4, so nothing is gained.
    end_phase(&mut g);
    assert_eq!(g.count_owned(P0, Card::Madman), 0);
    assert_eq!(trash_kinds(&g).last(), Some(&Card::Hermit));
}

#[test]
fn the_abilities_of_cards_trashed_together_resolve_in_the_players_order() {
    let mut g = setup(&[Card::Urchin, Card::Rats, Card::Cultist]);
    hand(&mut g, P0, &[Card::Mercenary, Card::Rats, Card::Cultist]);
    deck(&mut g, P0, &[Card::Copper; 6]);
    g.start();
    play(&mut g, Card::Mercenary);
    // Both cards in hand are trashed.
    yes(&mut g);
    // Rats' +1 Card and Cultist's +3 Cards are both pending: the player picks the order.
    let d = g.pending().unwrap();
    let DecisionKind::Choose { labels, .. } = &d.kind else {
        panic!("expected the trigger order, got {d:?}");
    };
    assert_eq!(d.purpose, Purpose::Trigger);
    let cultist = labels
        .iter()
        .position(|l| l.starts_with("Cultist"))
        .unwrap();
    assert!(labels.iter().any(|l| l.starts_with("Rats")));
    respond(&mut g, Response::option(Opt::Trigger(cultist as u16)));
    // +3 Cards, +1 Card, then Mercenary's +2 Cards.
    assert_eq!(g.hand(P0).len(), 6);
}

// ---- Random games ----

/// Random games of Dark Ages kingdoms where every deck starts with a copy of each
/// Kingdom and non-Supply card, so the expensive cards get played.
#[test]
fn random_dark_ages_games_keep_invariants() {
    let extras = [
        Card::Spoils,
        Card::Madman,
        Card::Mercenary,
        Card::Gold,
        Card::Survivors,
    ];
    for seed in 0..fuzz_seeds(200) {
        let mut rng = rand_chacha::ChaCha8Rng::seed_from_u64(seed);
        let players = rng.random_range(2..=4);
        let mut config = GameConfig::random(players, &[Set::DarkAges], seed);
        if seed % 3 == 0 && !config.kingdom.contains(&Card::Knights) {
            config.kingdom[0] = Card::Knights;
        }
        config.turn_limit = Some(60);
        let mut g = setup_with(config.clone());
        let mut cards: Vec<Card> = config
            .kingdom
            .iter()
            .flat_map(|k| k.pile_members())
            .chain(extras)
            .collect();
        cards.retain(|c| *c != Card::Knights);
        for p in 0..players {
            let p = PlayerId(p);
            for &c in &cards {
                g.create(c, Zone::Deck(p));
            }
            let mut deck = std::mem::take(&mut g.players[p.index()].deck);
            g.shuffle(&mut deck);
            g.players[p.index()].deck = deck;
        }
        g.start();
        let mut agents: Vec<Chaos> = (0..players)
            .map(|p| Chaos(rand_chacha::ChaCha8Rng::seed_from_u64(seed * 10 + p as u64)))
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
        assert!(
            g.trashing.is_empty(),
            "seed {seed}: trash bookkeeping leaked"
        );
        assert!(!g.outcome().unwrap().winners.is_empty());
    }
}
