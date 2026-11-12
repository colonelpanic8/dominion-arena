use super::prosperity::{can_buy, discard_kinds, pile_choices, trash_kinds, trigger};
use super::*;

#[test]
fn potion_pays_for_potion_costs() {
    let mut g = setup(&[Card::Familiar]);
    hand(&mut g, P0, &[Card::Potion, Card::Silver, Card::Copper]);
    g.start();
    play(&mut g, Card::Silver);
    play(&mut g, Card::Copper);
    assert!(!can_buy(&g, Card::Familiar));
    play(&mut g, Card::Potion);
    assert_eq!((g.turn().coins, g.turn().potions), (3, 1));
    assert!(can_buy(&g, Card::Familiar));
    buy(&mut g, Card::Familiar);
    assert_eq!(g.turn().potions, 0);
}

#[test]
fn a_potion_pile_of_sixteen_is_added_only_for_potion_cost_kingdoms() {
    let g = setup(&[Card::Golem]);
    let potions = g.supply_pile(Card::Potion).unwrap();
    assert_eq!(g.pile(potions).cards.len(), 16);
    assert!(
        setup(&[Card::Herbalist])
            .supply_pile(Card::Potion)
            .is_none()
    );
}

#[test]
fn alchemist_goes_onto_the_deck_with_a_potion_in_play() {
    let mut g = setup(&[Card::Alchemist]);
    hand(&mut g, P0, &[Card::Alchemist, Card::Potion]);
    deck(&mut g, P0, &[Card::Copper; 8]);
    g.start();
    play(&mut g, Card::Alchemist);
    play(&mut g, Card::Potion);
    end_phase(&mut g);
    trigger(&mut g, 0);
    assert_eq!(g.turn().player, P1);
    assert!(hand_kinds(&g, P0).contains(&Card::Alchemist));
}

#[test]
fn alchemist_played_on_another_players_turn_may_go_onto_its_players_deck() {
    let mut g = setup(&[Card::Alchemist]);
    hand(&mut g, P1, &[Card::Alchemist, Card::Potion]);
    deck(&mut g, P1, &[Card::Copper; 8]);
    g.start();
    play_out_of_turn(&mut g, P1, Card::Alchemist);
    play_out_of_turn(&mut g, P1, Card::Potion);
    end_phases(&mut g);
    let d = g.pending().unwrap();
    assert_eq!((d.player, d.purpose), (P1, Purpose::Trigger));
    trigger(&mut g, 0);
    assert_eq!(deck_kinds(&g, P1)[0], Card::Alchemist);
}

#[test]
fn alchemist_stays_without_a_potion_in_play() {
    let mut g = setup(&[Card::Alchemist]);
    hand(&mut g, P0, &[Card::Alchemist, Card::Potion]);
    deck(&mut g, P0, &[Card::Copper; 8]);
    g.start();
    play(&mut g, Card::Alchemist);
    end_phase(&mut g);
    assert_eq!(g.turn().player, P1);
    assert!(discard_kinds(&g, P0).contains(&Card::Alchemist));
}

#[test]
fn alchemist_resolves_before_herbalist_so_both_it_and_the_potion_go_back() {
    let mut g = setup(&[Card::Alchemist, Card::Herbalist]);
    hand(
        &mut g,
        P0,
        &[Card::Alchemist, Card::Herbalist, Card::Potion],
    );
    deck(&mut g, P0, &[Card::Copper; 8]);
    g.start();
    play(&mut g, Card::Alchemist);
    play(&mut g, Card::Herbalist);
    assert_eq!((g.turn().buys, g.turn().coins), (2, 1));
    play(&mut g, Card::Potion);
    end_phase(&mut g);
    trigger(&mut g, 0);
    assert_eq!(deck_kinds(&g, P0)[0], Card::Alchemist);
    trigger(&mut g, 0);
    choose_kinds(&mut g, &[Card::Potion]);
    assert_eq!(g.turn().player, P1);
    let hand = hand_kinds(&g, P0);
    assert!(hand.contains(&Card::Alchemist) && hand.contains(&Card::Potion));
}

#[test]
fn a_throne_roomed_herbalist_puts_back_two_treasures() {
    let mut g = setup(&[Card::Herbalist, Card::ThroneRoom]);
    hand(
        &mut g,
        P0,
        &[Card::ThroneRoom, Card::Herbalist, Card::Gold, Card::Silver],
    );
    deck(&mut g, P0, &[Card::Copper; 8]);
    g.start();
    play(&mut g, Card::ThroneRoom);
    choose_kinds(&mut g, &[Card::Herbalist]);
    play(&mut g, Card::Gold);
    play(&mut g, Card::Silver);
    end_phase(&mut g);
    trigger(&mut g, 0);
    choose_kinds(&mut g, &[Card::Gold]);
    trigger(&mut g, 0);
    choose_kinds(&mut g, &[Card::Silver]);
    let hand = hand_kinds(&g, P0);
    assert!(hand.contains(&Card::Gold) && hand.contains(&Card::Silver));
}

#[test]
fn herbalist_cannot_put_back_a_treasure_staying_in_play() {
    let mut g = setup(&[Card::Herbalist, Card::Astrolabe]);
    hand(&mut g, P0, &[Card::Herbalist, Card::Astrolabe]);
    deck(&mut g, P0, &[Card::Copper; 8]);
    g.start();
    play(&mut g, Card::Herbalist);
    play(&mut g, Card::Astrolabe);
    end_phase(&mut g);
    // Nothing to offer: the turn simply ends.
    assert_eq!(g.turn().player, P1);
    assert_eq!(kinds(&g, g.in_play(P0)), [Card::Astrolabe]);
}

#[test]
fn apothecary_takes_coppers_and_potions_and_orders_the_rest() {
    let mut g = setup(&[Card::Apothecary]);
    hand(&mut g, P0, &[Card::Apothecary]);
    deck(
        &mut g,
        P0,
        &[
            Card::Gold,
            Card::Estate,
            Card::Potion,
            Card::Silver,
            Card::Copper,
            Card::Duchy,
        ],
    );
    g.start();
    play(&mut g, Card::Apothecary);
    assert_eq!(g.turn().actions, 1);
    let mut order = candidates(&g);
    order.reverse();
    assert_eq!(kinds(&g, &order), [Card::Silver, Card::Estate]);
    choose(&mut g, &order);
    assert_eq!(hand_kinds(&g, P0), [Card::Gold, Card::Potion, Card::Copper]);
    assert_eq!(
        deck_kinds(&g, P0),
        [Card::Silver, Card::Estate, Card::Duchy]
    );
}

#[test]
fn apprentice_draws_per_coin_and_two_more_for_a_potion() {
    for (trashed, drawn) in [(Card::Golem, 6), (Card::Gold, 6), (Card::Copper, 0)] {
        let mut g = setup(&[Card::Apprentice, Card::Golem]);
        hand(&mut g, P0, &[Card::Apprentice, trashed]);
        deck(&mut g, P0, &[Card::Estate; 8]);
        g.start();
        play(&mut g, Card::Apprentice);
        assert_eq!(g.hand(P0).len(), drawn, "{trashed}");
        assert_eq!(trash_kinds(&g).last(), Some(&trashed));
    }
}

#[test]
fn familiar_curses_the_other_players() {
    let mut g = setup(&[Card::Familiar]);
    hand(&mut g, P0, &[Card::Familiar]);
    deck(&mut g, P0, &[Card::Copper; 3]);
    g.start();
    play(&mut g, Card::Familiar);
    assert_eq!((g.hand(P0).len(), g.turn().actions), (1, 1));
    assert_eq!(discard_kinds(&g, P1), [Card::Curse]);
}

#[test]
fn golem_plays_the_first_two_non_golem_actions_in_the_chosen_order() {
    let mut g = setup(&[Card::Golem, Card::Village, Card::Smithy]);
    hand(&mut g, P0, &[Card::Golem]);
    deck(
        &mut g,
        P0,
        &[
            Card::Estate,
            Card::Golem,
            Card::Smithy,
            Card::Copper,
            Card::Village,
            Card::Gold,
            Card::Silver,
            Card::Silver,
            Card::Silver,
        ],
    );
    g.start();
    play(&mut g, Card::Golem);
    assert_eq!(purpose(&g), Purpose::Play);
    choose_kinds(&mut g, &[Card::Village]);
    assert_eq!(
        kinds(&g, g.in_play(P0)),
        [Card::Golem, Card::Village, Card::Smithy]
    );
    // Village drew the Gold, then Smithy three Silvers.
    assert_eq!(
        hand_kinds(&g, P0),
        [Card::Gold, Card::Silver, Card::Silver, Card::Silver]
    );
    assert_eq!(
        discard_kinds(&g, P0),
        [Card::Estate, Card::Golem, Card::Copper]
    );
    assert_eq!(g.turn().actions, 2);
}

#[test]
fn golem_finding_one_action_plays_it_and_reshuffles_to_look() {
    let mut g = setup(&[Card::Golem, Card::Smithy]);
    hand(&mut g, P0, &[Card::Golem]);
    deck(&mut g, P0, &[Card::Estate]);
    discard(&mut g, P0, &[Card::Smithy]);
    g.start();
    play(&mut g, Card::Golem);
    assert_eq!(kinds(&g, g.in_play(P0)), [Card::Golem, Card::Smithy]);
    // Smithy drew the Estate back after it was discarded and reshuffled.
    assert_eq!(hand_kinds(&g, P0), [Card::Estate]);
}

#[test]
fn philosophers_stone_counts_deck_and_discard_in_fives() {
    let mut g = setup(&[Card::PhilosophersStone]);
    hand(&mut g, P0, &[Card::PhilosophersStone, Card::Copper]);
    deck(&mut g, P0, &[Card::Copper; 7]);
    discard(&mut g, P0, &[Card::Estate; 7]);
    g.start();
    play(&mut g, Card::PhilosophersStone);
    assert_eq!(g.turn().coins, 2);
}

#[test]
fn scrying_pool_lets_its_player_choose_for_everyone_then_digs_for_a_non_action() {
    let mut g = setup(&[Card::ScryingPool, Card::Village, Card::Smithy]);
    hand(&mut g, P0, &[Card::ScryingPool]);
    deck(
        &mut g,
        P0,
        &[
            Card::Estate,
            Card::Village,
            Card::Smithy,
            Card::Copper,
            Card::Gold,
        ],
    );
    deck(&mut g, P1, &[Card::Gold, Card::Estate]);
    g.start();
    play(&mut g, Card::ScryingPool);
    assert_eq!(g.pending().unwrap().player, P0);
    yes(&mut g);
    assert_eq!(g.pending().unwrap().player, P0);
    yes(&mut g);
    assert_eq!(discard_kinds(&g, P1), [Card::Gold]);
    assert_eq!(
        hand_kinds(&g, P0),
        [Card::Village, Card::Smithy, Card::Copper]
    );
    assert_eq!(deck_kinds(&g, P0), [Card::Gold]);
    assert_eq!(g.turn().actions, 1);
}

#[test]
fn moat_protects_from_scrying_pool_but_its_player_still_reveals() {
    let mut g = setup(&[Card::ScryingPool, Card::Moat]);
    hand(&mut g, P0, &[Card::ScryingPool]);
    deck(&mut g, P0, &[Card::Estate, Card::Copper]);
    hand(&mut g, P1, &[Card::Moat]);
    deck(&mut g, P1, &[Card::Gold]);
    g.start();
    play(&mut g, Card::ScryingPool);
    trigger(&mut g, 0);
    no(&mut g);
    assert_eq!(deck_kinds(&g, P1), [Card::Gold]);
    assert_eq!(hand_kinds(&g, P0), [Card::Estate]);
}

#[test]
fn transmute_gains_by_the_trashed_cards_types() {
    for (trashed, gained) in [
        (Card::Village, Some(Card::Duchy)),
        (Card::Copper, Some(Card::Transmute)),
        (Card::Estate, Some(Card::Gold)),
        (Card::Curse, None),
    ] {
        let mut g = setup(&[Card::Transmute, Card::Village]);
        hand(&mut g, P0, &[Card::Transmute, trashed]);
        g.start();
        play(&mut g, Card::Transmute);
        assert_eq!(trash_kinds(&g).last(), Some(&trashed));
        assert_eq!(discard_kinds(&g, P0), Vec::from_iter(gained), "{trashed}");
    }
}

#[test]
fn transmute_treats_a_charlatan_curse_as_a_treasure() {
    let mut g = setup(&[Card::Transmute, Card::Charlatan]);
    hand(&mut g, P0, &[Card::Transmute, Card::Curse]);
    g.start();
    play(&mut g, Card::Transmute);
    assert_eq!(discard_kinds(&g, P0), [Card::Transmute]);
}

#[test]
fn university_may_gain_an_action_costing_up_to_five_without_a_potion() {
    let mut g = setup(&[
        Card::University,
        Card::Market,
        Card::Familiar,
        Card::Artisan,
    ]);
    hand(&mut g, P0, &[Card::University]);
    g.start();
    play(&mut g, Card::University);
    assert_eq!(g.turn().actions, 2);
    let mut piles = pile_choices(&g);
    piles.sort();
    // No Potion costs (Familiar, University) and nothing over $5 (Artisan).
    assert_eq!(piles, [Card::Market]);
    respond(&mut g, Response::Pile(None));
    assert!(discard_kinds(&g, P0).is_empty());
}

#[test]
fn vineyard_is_worth_a_vp_per_three_actions() {
    let mut g = setup(&[Card::Vineyard, Card::Village]);
    hand(&mut g, P0, &[Card::Vineyard, Card::Vineyard]);
    deck(&mut g, P0, &[Card::Village; 7]);
    discard(&mut g, P0, &[]);
    g.start();
    let estates = g.count_owned(P0, Card::Estate) as i32;
    assert_eq!(g.score(P0), estates + 2 * 2);
}
