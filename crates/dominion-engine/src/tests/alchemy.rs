use super::prosperity::{
    can_buy, discard_kinds, mode, next_turn, pile_choices, play_from, trash_kinds, trigger,
    vp_tokens,
};
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

// ---- Possession ----

/// P0 plays Possession and ends their turn; P1's hand is `p1` for the Possession turn.
pub(super) fn possess(kingdom: &[Card], p1: &[Card]) -> Game {
    let mut k = vec![Card::Possession];
    k.extend_from_slice(kingdom);
    let mut g = setup(&k);
    hand(&mut g, P0, &[Card::Possession]);
    deck(&mut g, P0, &[Card::Copper; 10]);
    hand(&mut g, P1, p1);
    deck(&mut g, P1, &[Card::Estate; 10]);
    discard(&mut g, P1, &[]);
    g.start();
    play(&mut g, Card::Possession);
    next_turn(&mut g);
    g
}

#[test]
fn possession_gives_the_left_player_an_extra_turn_the_possessor_controls() {
    let mut g = possess(&[Card::Village], &[Card::Village, Card::Copper]);
    let t = g.turn();
    assert_eq!(
        (t.player, t.controller, t.extra),
        (P1, P0, Some(Card::Possession))
    );
    assert_eq!(g.pending().unwrap().player, P0);
    // The possessor sees the possessed player's hand; the possessed player still does.
    let village = find(&g, P1, Card::Village);
    assert_eq!(g.view(P0).card(village), Some(Card::Village));
    assert_eq!(g.view(P1).card(village), Some(Card::Village));
    play_from(&mut g, P1, Card::Village);
    next_turn(&mut g);
    // Then P1 takes their own turn.
    let t = g.turn();
    assert_eq!((t.player, t.controller, t.extra), (P1, P1, None));
    assert_eq!(g.pending().unwrap().player, P1);
    assert_eq!(g.view(P0).card(g.hand(P1)[0]), None);
}

#[test]
fn the_possessor_gains_what_the_possessed_player_would_gain() {
    let mut g = possess(
        &[Card::Artisan],
        &[Card::Artisan, Card::Copper, Card::Copper, Card::Copper],
    );
    let p0_discard = g.player(P0).discard.len();
    play_from(&mut g, P1, Card::Artisan);
    // Artisan gains to hand; the possessor gains it to their discard pile instead.
    choose_pile(&mut g, Card::Silver);
    assert_eq!(discard_kinds(&g, P0).last(), Some(&Card::Silver));
    // Artisan's topdeck (forced: all Coppers) is the possessed player's.
    assert_eq!(hand_kinds(&g, P1), [Card::Copper, Card::Copper]);
    // Buying: P1 buys, P0 gains.
    play_from(&mut g, P1, Card::Copper);
    play_from(&mut g, P1, Card::Copper);
    buy(&mut g, Card::Estate);
    assert!(g.log().contains(&Event::Bought {
        player: P1,
        card: Card::Estate
    }));
    assert_eq!(g.player(P0).discard.len(), p0_discard + 2);
    assert_eq!(discard_kinds(&g, P0).last(), Some(&Card::Estate));
    assert_eq!(g.count_owned(P1, Card::Silver), 0);
}

/// A card the possessor gains instead is not "the gained card" for what gained it.
#[test]
fn gainers_get_nothing_for_a_card_the_possessor_gains_instead() {
    let mut g = possess(&[Card::Ironworks], &[Card::Ironworks, Card::Copper]);
    play_from(&mut g, P1, Card::Ironworks);
    choose_pile(&mut g, Card::Silver);
    assert_eq!(discard_kinds(&g, P0).last(), Some(&Card::Silver));
    assert_eq!(g.turn().coins, 0);

    let mut g = possess(&[Card::Replace], &[Card::Replace, Card::Gold]);
    play_from(&mut g, P1, Card::Replace);
    choose_pile(&mut g, Card::Province);
    assert_eq!(discard_kinds(&g, P0).last(), Some(&Card::Province));
    assert_eq!(g.count_owned(P0, Card::Curse), 0);
}

#[test]
fn the_possessed_players_trashed_cards_return_to_their_discard_pile_after_the_turn() {
    let mut g = possess(
        &[Card::Chapel],
        &[Card::Chapel, Card::Gold, Card::Estate, Card::Copper],
    );
    let trash = g.trash().len();
    play_from(&mut g, P1, Card::Chapel);
    choose_kinds(&mut g, &[Card::Gold, Card::Estate, Card::Copper]);
    assert_eq!(g.trash().len(), trash);
    assert!(g.log().contains(&Event::Trashed {
        player: P1,
        card: Card::Gold
    }));
    assert_eq!(g.player(P1).set_aside.len(), 3);
    next_turn(&mut g);
    assert!(g.player(P1).set_aside.is_empty());
    assert_eq!(g.count_owned(P1, Card::Gold), 1);
    let discard = discard_kinds(&g, P1);
    assert!(
        [Card::Gold, Card::Estate, Card::Copper]
            .iter()
            .all(|c| discard.contains(c))
    );
}

#[test]
fn possession_does_not_return_other_players_trashed_cards() {
    // A Watchtower the possessor reveals trashes the possessor's own gain for good.
    let mut g = possess(&[Card::Watchtower], &[Card::Gold]);
    hand(&mut g, P0, &[Card::Watchtower]);
    let trash = g.trash().len();
    play_from(&mut g, P1, Card::Gold);
    buy(&mut g, Card::Silver);
    trigger(&mut g, 0);
    mode(&mut g, 0);
    assert_eq!(g.trash().len(), trash + 1);
    next_turn(&mut g);
    assert_eq!(trash_kinds(&g).last(), Some(&Card::Silver));
}

#[test]
fn vp_tokens_on_a_possession_turn_go_to_the_possessed_player() {
    let mut g = possess(&[Card::Bishop], &[Card::Bishop, Card::Gold]);
    play_from(&mut g, P1, Card::Bishop);
    assert_eq!((vp_tokens(&g, P0), vp_tokens(&g, P1)), (0, 1 + 3));
}

#[test]
fn debt_on_a_possession_turn_goes_to_the_possessor() {
    let mut g = possess(&[], &[Card::Copper]);
    g.add_debt(P1, 3);
    assert_eq!((g.player(P0).debt, g.player(P1).debt), (3, 0));
    next_turn(&mut g);
    g.add_debt(P1, 2);
    assert_eq!((g.player(P0).debt, g.player(P1).debt), (3, 2));
}

#[test]
fn the_possessor_reacts_to_gains_and_the_possessed_players_royal_seal_does_not() {
    let mut g = possess(
        &[Card::Watchtower, Card::RoyalSeal],
        &[Card::RoyalSeal, Card::Gold],
    );
    hand(&mut g, P0, &[Card::Watchtower]);
    play_from(&mut g, P1, Card::RoyalSeal);
    play_from(&mut g, P1, Card::Gold);
    buy(&mut g, Card::Silver);
    let d = g.pending().unwrap();
    assert_eq!((d.player, d.source), (P0, Some(Card::Watchtower)));
    trigger(&mut g, 0);
    mode(&mut g, 1);
    assert_eq!(deck_kinds(&g, P0)[0], Card::Silver);
}

#[test]
fn hoard_and_talisman_on_a_possession_turn() {
    let mut g = possess(
        &[Card::Hoard, Card::Talisman],
        &[Card::Hoard, Card::Talisman, Card::Gold, Card::Gold],
    );
    for c in [Card::Hoard, Card::Talisman, Card::Gold, Card::Gold] {
        play_from(&mut g, P1, c);
    }
    g.turn.buys = 2;
    let golds = g.count_owned(P0, Card::Gold) + g.count_owned(P1, Card::Gold);
    // The possessed player bought the Estate but did not gain it: no Gold from Hoard.
    buy(&mut g, Card::Estate);
    assert_eq!(
        g.count_owned(P0, Card::Gold) + g.count_owned(P1, Card::Gold),
        golds
    );
    // Talisman's copy is gained by the possessor too.
    buy(&mut g, Card::Silver);
    assert_eq!(
        &discard_kinds(&g, P0)[g.player(P0).discard.len() - 3..],
        [Card::Estate, Card::Silver, Card::Silver]
    );
}

#[test]
fn a_second_possession_in_a_turn_gives_no_second_extra_turn_in_a_row() {
    let mut g = setup(&[Card::Possession, Card::ThroneRoom]);
    hand(&mut g, P0, &[Card::ThroneRoom, Card::Possession]);
    deck(&mut g, P0, &[Card::Copper; 10]);
    hand(&mut g, P1, &[Card::Copper]);
    g.start();
    play(&mut g, Card::ThroneRoom);
    choose_kinds(&mut g, &[Card::Possession]);
    assert_eq!(g.extra_turns().len(), 0);
    next_turn(&mut g);
    assert_eq!((g.turn().player, g.turn().controller), (P1, P0));
    next_turn(&mut g);
    assert_eq!(
        (g.turn().player, g.turn().controller, g.turn().extra),
        (P1, P1, None)
    );
    assert!(g.extra_turns().is_empty());
}

#[test]
fn a_possessed_player_made_to_play_possession_possesses_the_next_player() {
    let mut g = possess(&[], &[Card::Possession]);
    play_from(&mut g, P1, Card::Possession);
    next_turn(&mut g);
    // P1 now possesses P0, then P1 takes their normal turn.
    assert_eq!((g.turn().player, g.turn().controller), (P0, P1));
    next_turn(&mut g);
    assert_eq!(
        (g.turn().player, g.turn().controller, g.turn().extra),
        (P1, P1, None)
    );
}

#[test]
fn a_possessed_player_made_to_play_voyage_takes_the_voyage_turn_then_their_own() {
    let mut g = possess(&[Card::Odysseys], &[Card::Voyage]);
    play_from(&mut g, P1, Card::Voyage);
    next_turn(&mut g);
    assert_eq!(
        (g.turn().player, g.turn().controller, g.turn().extra),
        (P1, P1, Some(Card::Voyage))
    );
    next_turn(&mut g);
    assert_eq!((g.turn().player, g.turn().extra), (P1, None));
}

#[test]
fn an_outpost_turn_comes_before_a_possession_turn_from_the_same_turn() {
    let mut g = setup(&[Card::Possession, Card::Outpost, Card::Village]);
    hand(
        &mut g,
        P0,
        &[Card::Village, Card::Possession, Card::Outpost],
    );
    deck(&mut g, P0, &[Card::Copper; 10]);
    hand(&mut g, P1, &[Card::Copper]);
    g.start();
    play(&mut g, Card::Village);
    play(&mut g, Card::Possession);
    play(&mut g, Card::Outpost);
    next_turn(&mut g);
    assert_eq!((g.turn().player, g.turn().extra), (P0, Some(Card::Outpost)));
    next_turn(&mut g);
    assert_eq!(
        (g.turn().player, g.turn().controller, g.turn().extra),
        (P1, P0, Some(Card::Possession))
    );
}

#[test]
fn blockade_curses_nobody_on_a_possession_turn() {
    let mut g = setup_with(GameConfig {
        colonies: Some(false),
        shelters: Some(false),
        ..GameConfig::new(3, vec![Card::Possession, Card::Blockade], 7)
    });
    hand(&mut g, P0, &[Card::Blockade]);
    hand(&mut g, P1, &[Card::Possession]);
    hand(&mut g, P2, &[Card::Silver, Card::Silver]);
    g.start();
    play(&mut g, Card::Blockade);
    choose_pile(&mut g, Card::Silver);
    next_turn(&mut g);
    play(&mut g, Card::Possession);
    next_turn(&mut g);
    assert_eq!((g.turn().player, g.turn().controller), (P2, P1));
    play_from(&mut g, P2, Card::Silver);
    play_from(&mut g, P2, Card::Silver);
    buy(&mut g, Card::Silver);
    assert_eq!(discard_kinds(&g, P1).last(), Some(&Card::Silver));
    for p in [P0, P1, P2] {
        assert_eq!(g.count_owned(p, Card::Curse), 0);
    }
}

#[test]
fn possession_turns_do_not_count_for_the_tiebreaker() {
    let mut g = setup_with(GameConfig {
        colonies: Some(false),
        shelters: Some(false),
        turn_limit: Some(3),
        ..GameConfig::new(2, vec![Card::Possession], 7)
    });
    hand(&mut g, P0, &[Card::Possession]);
    g.start();
    play(&mut g, Card::Possession);
    next_turn(&mut g);
    next_turn(&mut g);
    while g.pending().is_some() {
        end_phase(&mut g);
    }
    let outcome = g.outcome().unwrap();
    assert_eq!(outcome.turns, [1, 1]);
    assert_eq!(g.player(P1).turns_taken, 2);
}

/// Random games of Possession with other Alchemy and Prosperity cards (removed ones too)
/// and Outpost, every deck starting with a copy of each Kingdom card.
#[test]
fn random_possession_games_keep_invariants() {
    let pool: Vec<Card> = Card::kingdom_cards()
        .filter(|c| matches!(c.set(), Set::Alchemy | Set::Prosperity))
        .filter(|&c| c != Card::Possession)
        .collect();
    for seed in 0..fuzz_seeds(100) {
        let mut rng = ChaCha8Rng::seed_from_u64(seed);
        let mut kingdom: Vec<Card> = pool.choose_multiple(&mut rng, 7).copied().collect();
        kingdom.extend([Card::Possession, Card::Outpost, Card::ThroneRoom]);
        let mut config = GameConfig::new(rng.random_range(2..=4), kingdom, seed);
        config.turn_limit = Some(60);
        random_game_with_kingdom_decks(config, seed);
    }
}

#[test]
fn a_possessed_players_overpay_does_nothing_since_the_possessor_gains_the_card() {
    let mut g = possess(&[Card::Masterpiece], &[Card::Gold, Card::Gold]);
    play_from(&mut g, P1, Card::Gold);
    play_from(&mut g, P1, Card::Gold);
    let silvers = g.count_owned(P0, Card::Silver) + g.count_owned(P1, Card::Silver);
    buy(&mut g, Card::Masterpiece);
    respond(&mut g, Response::Number(3));
    assert_eq!(discard_kinds(&g, P0).last(), Some(&Card::Masterpiece));
    assert_eq!(
        g.count_owned(P0, Card::Silver) + g.count_owned(P1, Card::Silver),
        silvers
    );
}
