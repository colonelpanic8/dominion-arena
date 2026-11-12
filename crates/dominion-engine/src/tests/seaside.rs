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
    if let Some(DecisionKind::Turn {
        phase: Phase::Action,
        ..
    }) = g.pending().map(|d| &d.kind)
    {
        end_phase(g);
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

fn mat_kinds(g: &Game, p: PlayerId, mat: Mat) -> Vec<Card> {
    kinds(g, g.player(p).mat(mat))
}

fn mode(g: &mut Game, i: u8) {
    respond(g, Response::option(Opt::Mode(i)));
}

#[test]
fn haven_sets_a_card_aside_until_next_turn_and_stays_in_play() {
    let mut g = setup(&[Card::Haven]);
    hand(&mut g, P0, &[Card::Haven, Card::Gold, Card::Estate]);
    deck(&mut g, P0, &[Card::Copper; 20]);
    g.start();
    play(&mut g, Card::Haven);
    assert_eq!(purpose(&g), Purpose::SetAside);
    choose_kinds(&mut g, &[Card::Gold]);
    assert_eq!(kinds(&g, &g.player(P0).set_aside), [Card::Gold]);
    let gold = g.player(P0).set_aside[0];
    assert_eq!(g.view(P1).card(gold), None);
    let theirs = g.view(P1);
    assert!(theirs.player(P0).set_aside().is_empty());
    assert_eq!(theirs.player(P0).set_aside_size(), 1);
    assert!(theirs.lasting().iter().all(|l| l.data.pick.is_none()));
    assert_eq!(g.view(P0).player(P0).set_aside(), [gold]);
    assert_eq!(g.view(P0).lasting()[0].data.pick, Some(gold));
    next_turn(&mut g);
    assert_eq!(in_play_kinds(&g, P0), [Card::Haven]);
    next_turn(&mut g);
    assert!(hand_kinds(&g, P0).contains(&Card::Gold));
    assert_eq!(g.hand(P0).len(), 6);
    assert_eq!(in_play_kinds(&g, P0), [Card::Haven]);
    next_turn(&mut g);
    assert!(g.in_play(P0).is_empty());
    assert!(discard_kinds(&g, P0).contains(&Card::Haven));
}

#[test]
fn throne_roomed_havens_holding_different_cards_are_ordered() {
    let mut g = setup(&[Card::Haven, Card::ThroneRoom]);
    hand(
        &mut g,
        P0,
        &[Card::ThroneRoom, Card::Haven, Card::Gold, Card::Estate],
    );
    deck(&mut g, P0, &[Card::Copper; 20]);
    g.start();
    play(&mut g, Card::ThroneRoom);
    choose_kinds(&mut g, &[Card::Haven]);
    choose_kinds(&mut g, &[Card::Gold]);
    choose_kinds(&mut g, &[Card::Estate]);
    next_turn(&mut g);
    next_turn(&mut g);
    assert_eq!(purpose(&g), Purpose::Trigger);
    let triggers = g.view(P0).pending_triggers();
    assert_eq!(triggers.len(), 2);
    assert!(triggers.iter().all(|t| t.card == Card::Haven));
    assert!(g.view(P1).pending_triggers().is_empty());
    respond(&mut g, Response::option(Opt::Trigger(0)));
    let hand = hand_kinds(&g, P0);
    assert!(hand.contains(&Card::Gold) && hand.contains(&Card::Estate));
}

#[test]
fn haven_with_nothing_to_set_aside_is_discarded_this_turn() {
    let mut g = setup(&[Card::Haven]);
    hand(&mut g, P0, &[Card::Haven]);
    deck(&mut g, P0, &[]);
    g.start();
    play(&mut g, Card::Haven);
    next_turn(&mut g);
    assert!(g.in_play(P0).is_empty());
    assert!(g.lasting().is_empty());
}

#[test]
fn throne_room_on_a_duration_doubles_both_parts_and_stays_with_it() {
    let mut g = setup(&[Card::ThroneRoom, Card::Caravan]);
    hand(&mut g, P0, &[Card::ThroneRoom, Card::Caravan]);
    deck(&mut g, P0, &[Card::Copper; 12]);
    g.start();
    play(&mut g, Card::ThroneRoom);
    choose_kinds(&mut g, &[Card::Caravan]);
    assert_eq!(g.hand(P0).len(), 2);
    assert_eq!(g.turn().actions, 2);
    next_turn(&mut g);
    assert_eq!(in_play_kinds(&g, P0), [Card::ThroneRoom, Card::Caravan]);
    next_turn(&mut g);
    resolve_triggers(&mut g);
    assert_eq!(g.hand(P0).len(), 7);
    assert_eq!(in_play_kinds(&g, P0), [Card::ThroneRoom, Card::Caravan]);
    next_turn(&mut g);
    assert!(g.in_play(P0).is_empty());
}

#[test]
fn throne_room_on_a_duration_that_does_nothing_is_discarded() {
    let mut g = setup(&[Card::ThroneRoom, Card::Tactician]);
    hand(&mut g, P0, &[Card::ThroneRoom, Card::Tactician]);
    deck(&mut g, P0, &[Card::Copper; 12]);
    g.start();
    play(&mut g, Card::ThroneRoom);
    choose_kinds(&mut g, &[Card::Tactician]);
    next_turn(&mut g);
    assert!(g.in_play(P0).is_empty());
}

#[test]
fn throne_room_on_tactician_gives_the_bonus_once() {
    let mut g = setup(&[Card::ThroneRoom, Card::Tactician]);
    hand(
        &mut g,
        P0,
        &[
            Card::ThroneRoom,
            Card::Tactician,
            Card::Copper,
            Card::Estate,
        ],
    );
    deck(&mut g, P0, &[Card::Copper; 15]);
    g.start();
    play(&mut g, Card::ThroneRoom);
    choose_kinds(&mut g, &[Card::Tactician]);
    assert!(g.hand(P0).is_empty());
    assert_eq!(discard_kinds(&g, P0).len(), 2);
    next_turn(&mut g);
    assert_eq!(in_play_kinds(&g, P0), [Card::ThroneRoom, Card::Tactician]);
    next_turn(&mut g);
    assert_eq!(g.hand(P0).len(), 10);
    assert_eq!((g.turn().actions, g.turn().buys), (2, 2));
}

#[test]
fn a_duration_that_left_play_does_nothing_on_later_turns() {
    let mut g = setup(&[Card::Astrolabe, Card::Mandarin]);
    hand(&mut g, P0, &[Card::Astrolabe, Card::Gold, Card::Silver]);
    deck(&mut g, P0, &[Card::Estate; 10]);
    g.start();
    play(&mut g, Card::Astrolabe);
    play(&mut g, Card::Gold);
    play(&mut g, Card::Silver);
    // Gaining Mandarin puts the Treasures in play, Astrolabe included, onto the deck.
    buy(&mut g, Card::Mandarin);
    let order = candidates(&g);
    choose(&mut g, &order);
    assert!(g.in_play(P0).is_empty());
    next_turn(&mut g);
    next_turn(&mut g);
    assert_eq!(g.turn().player, P0);
    assert_eq!((g.turn().coins, g.turn().buys), (0, 1));
    // Played again, it works again, once.
    play(&mut g, Card::Astrolabe);
    next_turn(&mut g);
    next_turn(&mut g);
    assert_eq!(g.turn().player, P0);
    assert_eq!((g.turn().coins, g.turn().buys), (1, 2));
}

#[test]
fn a_duration_whose_tiara_left_play_is_not_doubled_on_later_turns() {
    let mut g = setup(&[Card::Astrolabe, Card::Tiara, Card::Mint]);
    hand(
        &mut g,
        P0,
        &[Card::Tiara, Card::Astrolabe, Card::Gold, Card::Estate],
    );
    deck(&mut g, P0, &[Card::Estate; 10]);
    g.start();
    play(&mut g, Card::Tiara);
    choose_kinds(&mut g, &[Card::Astrolabe]);
    play(&mut g, Card::Gold);
    assert_eq!((g.turn().coins, g.turn().buys), (5, 4));
    // Gaining Mint trashes the Tiara but not the Astrolabe; Tiara's own effect is declined.
    buy(&mut g, Card::Mint);
    respond(&mut g, Response::option(Opt::Trigger(0)));
    no(&mut g);
    assert_eq!(in_play_kinds(&g, P0), [Card::Astrolabe]);
    next_turn(&mut g);
    next_turn(&mut g);
    assert_eq!(g.turn().player, P0);
    assert_eq!((g.turn().coins, g.turn().buys), (1, 2));
}

#[test]
fn a_duration_whose_kings_court_left_play_is_played_once_on_later_turns() {
    let mut g = setup(&[Card::Procession, Card::KingsCourt, Card::FishingVillage]);
    hand(
        &mut g,
        P0,
        &[Card::Procession, Card::KingsCourt, Card::FishingVillage],
    );
    deck(&mut g, P0, &[Card::Estate; 10]);
    g.start();
    // Procession trashes the King's Court after it played Fishing Village three times.
    play(&mut g, Card::Procession);
    choose_kinds(&mut g, &[Card::KingsCourt]);
    choose_kinds(&mut g, &[Card::FishingVillage]);
    assert_eq!(g.turn().coins, 3);
    assert_eq!(
        in_play_kinds(&g, P0),
        [Card::Procession, Card::FishingVillage]
    );
    next_turn(&mut g);
    assert_eq!(in_play_kinds(&g, P0), [Card::FishingVillage]);
    next_turn(&mut g);
    assert_eq!(g.turn().player, P0);
    assert_eq!((g.turn().actions, g.turn().coins), (2, 1));
}

#[test]
fn a_haven_whose_throne_room_left_play_returns_only_its_first_card() {
    let mut g = setup(&[Card::Procession, Card::ThroneRoom, Card::Haven]);
    hand(
        &mut g,
        P0,
        &[
            Card::Procession,
            Card::ThroneRoom,
            Card::Haven,
            Card::Gold,
            Card::Silver,
        ],
    );
    deck(&mut g, P0, &[Card::Estate; 12]);
    g.start();
    play(&mut g, Card::Procession);
    choose_kinds(&mut g, &[Card::ThroneRoom]);
    choose_kinds(&mut g, &[Card::Haven]);
    choose_kinds(&mut g, &[Card::Gold]);
    choose_kinds(&mut g, &[Card::Silver]);
    assert_eq!(in_play_kinds(&g, P0), [Card::Procession, Card::Haven]);
    next_turn(&mut g);
    next_turn(&mut g);
    assert!(hand_kinds(&g, P0).contains(&Card::Gold));
    // The second play's card stays set aside for the rest of the game, still the player's.
    assert_eq!(kinds(&g, &g.player(P0).set_aside), [Card::Silver]);
    next_turn(&mut g);
    next_turn(&mut g);
    assert_eq!(kinds(&g, &g.player(P0).set_aside), [Card::Silver]);
    assert_eq!(g.count_owned(P0, Card::Silver), 1);
    assert!(g.lasting().is_empty());
}

#[test]
fn a_lighthouse_that_left_play_protects_and_pays_no_longer() {
    let mut g = setup(&[Card::Lighthouse, Card::Militia]);
    hand(&mut g, P0, &[Card::Lighthouse]);
    deck(&mut g, P0, &[Card::Copper; 10]);
    hand(&mut g, P1, &[Card::Militia]);
    g.start();
    play(&mut g, Card::Lighthouse);
    leave_play(&mut g, P0, Card::Lighthouse);
    next_turn(&mut g);
    play(&mut g, Card::Militia);
    assert_eq!(g.hand(P0).len(), 3);
    next_turn(&mut g);
    assert_eq!((g.turn().player, g.turn().coins), (P0, 0));
}

#[test]
fn a_corsair_that_left_play_attacks_and_draws_no_longer() {
    let mut g = setup(&[Card::Corsair]);
    hand(&mut g, P0, &[Card::Corsair]);
    deck(&mut g, P0, &[Card::Copper; 15]);
    hand(&mut g, P1, &[Card::Silver]);
    g.start();
    play(&mut g, Card::Corsair);
    leave_play(&mut g, P0, Card::Corsair);
    next_turn(&mut g);
    to_buy(&mut g);
    play(&mut g, Card::Silver);
    assert_eq!(in_play_kinds(&g, P1), [Card::Silver]);
    next_turn(&mut g);
    assert_eq!((g.turn().player, g.hand(P0).len()), (P0, 5));
}

#[test]
fn an_outpost_that_left_play_gives_no_extra_turn_but_still_a_hand_of_three() {
    let mut g = setup(&[Card::Outpost]);
    hand(&mut g, P0, &[Card::Outpost]);
    deck(&mut g, P0, &[Card::Copper; 10]);
    g.start();
    play(&mut g, Card::Outpost);
    leave_play(&mut g, P0, Card::Outpost);
    next_turn(&mut g);
    assert_eq!((g.turn().player, g.turn().extra), (P1, None));
    assert_eq!(g.hand(P0).len(), 3);
    assert!(g.lasting().is_empty());
}

#[test]
fn lighthouse_gives_a_coin_now_and_next_turn_and_blocks_attacks_until_then() {
    let mut g = setup(&[Card::Lighthouse, Card::Militia]);
    hand(&mut g, P0, &[Card::Lighthouse, Card::Copper]);
    deck(&mut g, P0, &[Card::Copper; 10]);
    hand(
        &mut g,
        P1,
        &[
            Card::Militia,
            Card::Copper,
            Card::Copper,
            Card::Copper,
            Card::Copper,
        ],
    );
    g.start();
    play(&mut g, Card::Lighthouse);
    assert_eq!((g.turn().actions, g.turn().coins), (1, 1));
    next_turn(&mut g);
    play(&mut g, Card::Militia);
    assert_eq!(g.hand(P0).len(), 5);
    assert_eq!(g.turn().coins, 2);
    assert!(g.log().contains(&Event::Unaffected {
        player: P0,
        attack: Card::Militia,
        by: Card::Lighthouse
    }));
    next_turn(&mut g);
    assert_eq!(g.turn().coins, 1);
    assert!(g.lasting().is_empty());
    assert_eq!(in_play_kinds(&g, P0), [Card::Lighthouse]);
    let start = Event::TurnStart {
        player: P0,
        turn: 2,
    };
    let resolved = Event::Resolved {
        player: P0,
        card: Card::Lighthouse,
    };
    let bonus = Event::Bonus {
        player: P0,
        actions: 0,
        buys: 0,
        coins: 1,
    };
    assert!(logged_in_order(&g, &[start, resolved, bonus]));
}

/// Lighthouse only makes its player unaffected, which leaves nothing for a Moat to do:
/// no order to ask.
#[test]
fn lighthouse_resolves_before_a_moat_without_asking() {
    let mut g = setup(&[Card::Lighthouse, Card::Militia, Card::Moat]);
    hand(&mut g, P0, &[Card::Lighthouse]);
    deck(&mut g, P0, &[Card::Copper; 10]);
    hand(&mut g, P1, &[Card::Militia]);
    g.start();
    play(&mut g, Card::Lighthouse);
    next_turn(&mut g);
    hand(&mut g, P0, &[Card::Moat]);
    play(&mut g, Card::Militia);
    assert_ne!(g.pending().unwrap().player, P0);
    assert!(g.log().iter().any(|e| matches!(
        e,
        Event::Unaffected {
            player: P0,
            by: Card::Lighthouse,
            ..
        }
    )));
    assert_eq!(hand_kinds(&g, P0), [Card::Moat]);
}

#[test]
fn lighthouse_does_not_protect_after_it_resolves() {
    let mut g = setup(&[Card::Lighthouse, Card::Militia]);
    hand(&mut g, P0, &[Card::Lighthouse]);
    deck(&mut g, P0, &[Card::Copper, Card::Estate].repeat(8));
    deck(
        &mut g,
        P1,
        &[
            Card::Militia,
            Card::Copper,
            Card::Copper,
            Card::Copper,
            Card::Copper,
        ],
    );
    g.start();
    play(&mut g, Card::Lighthouse);
    next_turn(&mut g);
    next_turn(&mut g);
    next_turn(&mut g);
    play(&mut g, Card::Militia);
    assert_eq!(
        (g.pending().unwrap().player, purpose(&g)),
        (P0, Purpose::Discard)
    );
}

#[test]
fn native_village_stores_cards_privately_and_takes_them_all_back() {
    let mut g = setup(&[Card::NativeVillage]);
    hand(&mut g, P0, &[Card::NativeVillage, Card::NativeVillage]);
    deck(&mut g, P0, &[Card::Gold, Card::Silver]);
    g.start();
    play(&mut g, Card::NativeVillage);
    mode(&mut g, 0);
    assert_eq!(mat_kinds(&g, P0, Mat::NativeVillage), [Card::Gold]);
    let gold = g.player(P0).mat(Mat::NativeVillage)[0];
    assert_eq!(g.view(P1).card(gold), None);
    assert_eq!(g.view(P0).card(gold), Some(Card::Gold));
    let theirs = g.view(P1).player(P0);
    assert_eq!(theirs.mat(Mat::NativeVillage), None);
    assert_eq!(theirs.mat_size(Mat::NativeVillage), 1);
    assert_eq!(
        g.view(P0).player(P0).mat(Mat::NativeVillage),
        Some(&[gold][..])
    );
    play(&mut g, Card::NativeVillage);
    mode(&mut g, 1);
    assert_eq!(hand_kinds(&g, P0), [Card::Gold]);
    assert_eq!(g.turn().actions, 3);
}

#[test]
fn astrolabe_gives_a_coin_and_a_buy_now_and_next_turn() {
    let mut g = setup(&[Card::Astrolabe]);
    hand(&mut g, P0, &[Card::Astrolabe]);
    deck(&mut g, P0, &[Card::Estate; 10]);
    g.start();
    to_buy(&mut g);
    play(&mut g, Card::Astrolabe);
    assert_eq!((g.turn().coins, g.turn().buys), (1, 2));
    next_turn(&mut g);
    next_turn(&mut g);
    assert_eq!((g.turn().coins, g.turn().buys), (1, 2));
    assert_eq!(in_play_kinds(&g, P0), [Card::Astrolabe]);
}

#[test]
fn fishing_village_and_merchant_ship_pay_out_next_turn() {
    let mut g = setup(&[Card::FishingVillage, Card::MerchantShip]);
    hand(&mut g, P0, &[Card::FishingVillage, Card::MerchantShip]);
    deck(&mut g, P0, &[Card::Estate; 10]);
    g.start();
    play(&mut g, Card::FishingVillage);
    play(&mut g, Card::MerchantShip);
    assert_eq!((g.turn().actions, g.turn().coins), (1, 3));
    next_turn(&mut g);
    next_turn(&mut g);
    resolve_triggers(&mut g);
    assert_eq!((g.turn().actions, g.turn().coins), (2, 3));
}

#[test]
fn lookout_trashes_one_discards_one_and_puts_one_back() {
    let mut g = setup(&[Card::Lookout]);
    hand(&mut g, P0, &[Card::Lookout]);
    deck(
        &mut g,
        P0,
        &[Card::Curse, Card::Estate, Card::Gold, Card::Silver],
    );
    g.start();
    play(&mut g, Card::Lookout);
    assert_eq!(purpose(&g), Purpose::Trash);
    choose_kinds(&mut g, &[Card::Curse]);
    assert_eq!(purpose(&g), Purpose::Discard);
    choose_kinds(&mut g, &[Card::Estate]);
    assert_eq!(kinds(&g, g.trash()).last(), Some(&Card::Curse));
    assert_eq!(discard_kinds(&g, P0), [Card::Estate]);
    assert_eq!(deck_kinds(&g, P0), [Card::Gold, Card::Silver]);
}

/// Wiki, Lookout: the three cards are not the top cards of the deck while Lookout
/// resolves, so trashing an Overgrown Estate draws the next one down.
#[test]
fn a_card_lookout_trashes_draws_the_card_under_the_three() {
    let mut g = setup(&[Card::Lookout]);
    hand(&mut g, P0, &[Card::Lookout]);
    deck(
        &mut g,
        P0,
        &[
            Card::OvergrownEstate,
            Card::Copper,
            Card::Estate,
            Card::Silver,
        ],
    );
    g.start();
    play(&mut g, Card::Lookout);
    choose_kinds(&mut g, &[Card::OvergrownEstate]);
    assert_eq!(hand_kinds(&g, P0), [Card::Silver]);
    choose_kinds(&mut g, &[Card::Copper]);
    assert_eq!(deck_kinds(&g, P0), [Card::Estate]);
}

/// Wiki, Lookout: a card put on the deck while Lookout resolves (a Tunnel discarded gains
/// a Gold, which Progress puts there) ends up under the card put back.
#[test]
fn the_card_lookout_puts_back_goes_over_one_put_on_the_deck_meanwhile() {
    let mut config = GameConfig {
        colonies: Some(false),
        shelters: Some(false),
        ..GameConfig::new(2, vec![Card::Lookout, Card::Tunnel], 0)
    };
    config.landscapes = vec![Card::Progress];
    let mut g = setup_with(config);
    g.prophecy = Some((Card::Progress, 0));
    hand(&mut g, P0, &[Card::Lookout]);
    deck(
        &mut g,
        P0,
        &[Card::Copper, Card::Tunnel, Card::Estate, Card::Silver],
    );
    g.start();
    play(&mut g, Card::Lookout);
    choose_kinds(&mut g, &[Card::Copper]);
    choose_kinds(&mut g, &[Card::Tunnel]);
    super::intrigue::trigger(&mut g, Card::Tunnel);
    assert_eq!(deck_kinds(&g, P0), [Card::Estate, Card::Gold, Card::Silver]);
}

#[test]
fn lookout_with_one_card_must_trash_it() {
    let mut g = setup(&[Card::Lookout]);
    hand(&mut g, P0, &[Card::Lookout]);
    deck(&mut g, P0, &[Card::Gold]);
    g.start();
    play(&mut g, Card::Lookout);
    assert_eq!(kinds(&g, g.trash()).last(), Some(&Card::Gold));
}

#[test]
fn monkey_draws_when_the_player_to_the_right_gains_and_next_turn() {
    let mut g = setup(&[Card::Monkey, Card::Witch]);
    hand(&mut g, P0, &[Card::Monkey]);
    deck(&mut g, P0, &[Card::Copper; 15]);
    hand(&mut g, P1, &[Card::Witch, Card::Silver, Card::Copper]);
    deck(&mut g, P1, &[Card::Copper; 15]);
    g.start();
    play(&mut g, Card::Monkey);
    // Only the player to the right's gains count.
    to_buy(&mut g);
    buy(&mut g, Card::Copper);
    assert_eq!(g.hand(P0).len(), 5);
    assert_eq!(g.turn().player, P1);
    play(&mut g, Card::Witch);
    assert_eq!(g.hand(P0).len(), 5);
    to_buy(&mut g);
    play(&mut g, Card::Silver);
    play(&mut g, Card::Copper);
    buy(&mut g, Card::Silver);
    // +1 Card for the Silver, then +1 Card as P0's turn starts.
    assert_eq!(g.turn().player, P0);
    assert_eq!(g.hand(P0).len(), 7);
    next_turn(&mut g);
    to_buy(&mut g);
    buy(&mut g, Card::Copper);
    assert_eq!(g.hand(P0).len(), 5);
}

#[test]
fn sea_chart_takes_the_top_card_if_a_copy_is_in_play() {
    let mut g = setup(&[Card::SeaChart, Card::Village]);
    hand(&mut g, P0, &[Card::SeaChart, Card::Village]);
    deck(&mut g, P0, &[Card::Estate, Card::Copper, Card::Estate]);
    g.start();
    play(&mut g, Card::Village);
    play(&mut g, Card::SeaChart);
    // No Estate in play: the revealed Estate stays.
    assert_eq!(deck_kinds(&g, P0), [Card::Estate]);
    assert_eq!(g.hand(P0).len(), 2);
    let mut g2 = setup(&[Card::SeaChart]);
    hand(&mut g2, P0, &[Card::SeaChart]);
    deck(&mut g2, P0, &[Card::Estate, Card::SeaChart, Card::Gold]);
    g2.start();
    play(&mut g2, Card::SeaChart);
    assert_eq!(hand_kinds(&g2, P0), [Card::Estate, Card::SeaChart]);
    assert_eq!(deck_kinds(&g2, P0), [Card::Gold]);
}

#[test]
fn smugglers_gains_a_copy_of_a_cheap_card_the_right_player_gained() {
    let mut g = setup(&[Card::Smugglers, Card::Festival]);
    hand(&mut g, P0, &[Card::Estate]);
    hand(
        &mut g,
        P1,
        &[Card::Festival, Card::Gold, Card::Gold, Card::Gold],
    );
    deck(
        &mut g,
        P0,
        &[
            Card::Smugglers,
            Card::Copper,
            Card::Copper,
            Card::Copper,
            Card::Copper,
        ],
    );
    g.start();
    next_turn(&mut g);
    play(&mut g, Card::Festival);
    to_buy(&mut g);
    for _ in 0..3 {
        play(&mut g, Card::Gold);
    }
    buy(&mut g, Card::Province);
    buy(&mut g, Card::Silver);
    assert_eq!(g.turn().player, P0);
    play(&mut g, Card::Smugglers);
    assert_eq!(g.count_owned(P0, Card::Silver), 1);
    assert_eq!(g.count_owned(P0, Card::Province), 0);
}

#[test]
fn smugglers_does_nothing_if_the_right_player_gained_nothing_cheap() {
    let mut g = setup(&[Card::Smugglers]);
    hand(&mut g, P0, &[Card::Smugglers]);
    g.start();
    play(&mut g, Card::Smugglers);
    assert_eq!(g.pending().unwrap().purpose, Purpose::Turn);
    assert_eq!(g.player(P0).discard.len(), 0);
}

#[test]
fn warehouse_draws_three_and_discards_three() {
    let mut g = setup(&[Card::Warehouse]);
    hand(&mut g, P0, &[Card::Warehouse, Card::Estate]);
    deck(&mut g, P0, &[Card::Gold, Card::Estate, Card::Curse]);
    g.start();
    play(&mut g, Card::Warehouse);
    choose_kinds(&mut g, &[Card::Estate, Card::Estate, Card::Curse]);
    assert_eq!(hand_kinds(&g, P0), [Card::Gold]);
    assert_eq!(g.turn().actions, 1);
}

#[test]
fn blockade_sets_the_gain_aside_and_curses_copies_gained_on_their_turn() {
    let mut g = setup(&[Card::Blockade]);
    hand(&mut g, P0, &[Card::Blockade]);
    deck(&mut g, P0, &[Card::Copper; 15]);
    hand(&mut g, P1, &[Card::Silver, Card::Copper]);
    g.start();
    play(&mut g, Card::Blockade);
    choose_pile(&mut g, Card::Silver);
    assert_eq!(kinds(&g, &g.player(P0).set_aside), [Card::Silver]);
    assert!(discard_kinds(&g, P0).is_empty());
    next_turn(&mut g);
    to_buy(&mut g);
    play(&mut g, Card::Silver);
    play(&mut g, Card::Copper);
    buy(&mut g, Card::Silver);
    assert_eq!(g.count_owned(P1, Card::Curse), 1);
    assert_eq!(g.turn().player, P0);
    assert!(hand_kinds(&g, P0).contains(&Card::Silver));
    assert_eq!(g.hand(P0).len(), 6);
    assert_eq!(in_play_kinds(&g, P0), [Card::Blockade]);
}

#[test]
fn blockade_does_not_curse_a_player_who_revealed_moat() {
    let mut g = setup(&[Card::Blockade, Card::Moat]);
    hand(&mut g, P0, &[Card::Blockade]);
    hand(
        &mut g,
        P1,
        &[Card::Moat, Card::Silver, Card::Copper, Card::Copper],
    );
    g.start();
    play(&mut g, Card::Blockade);
    respond(&mut g, Response::option(Opt::Trigger(0)));
    choose_pile(&mut g, Card::Silver);
    next_turn(&mut g);
    to_buy(&mut g);
    play(&mut g, Card::Silver);
    play(&mut g, Card::Copper);
    buy(&mut g, Card::Silver);
    assert_eq!(g.count_owned(P1, Card::Curse), 0);
}

#[test]
fn caravan_draws_at_the_start_of_next_turn() {
    let mut g = setup(&[Card::Caravan]);
    hand(&mut g, P0, &[Card::Caravan]);
    deck(&mut g, P0, &[Card::Copper; 12]);
    g.start();
    play(&mut g, Card::Caravan);
    assert_eq!(g.hand(P0).len(), 1);
    next_turn(&mut g);
    assert_eq!(g.hand(P0).len(), 5);
    next_turn(&mut g);
    assert_eq!(g.hand(P0).len(), 6);
}

#[test]
fn cutpurse_makes_others_discard_a_copper_or_reveal() {
    let mut g = setup_with(GameConfig {
        colonies: Some(false),
        shelters: Some(false),
        ..GameConfig::new(3, vec![Card::Cutpurse], 7)
    });
    hand(&mut g, P0, &[Card::Cutpurse]);
    hand(&mut g, P1, &[Card::Copper, Card::Estate]);
    hand(&mut g, P2, &[Card::Estate, Card::Silver]);
    g.start();
    play(&mut g, Card::Cutpurse);
    assert_eq!(g.turn().coins, 2);
    assert_eq!(hand_kinds(&g, P1), [Card::Estate]);
    assert_eq!(discard_kinds(&g, P1), [Card::Copper]);
    assert_eq!(g.hand(P2).len(), 2);
    assert!(g.log().contains(&Event::RevealedHand {
        player: P2,
        cards: vec![Card::Estate, Card::Silver]
    }));
}

#[test]
fn island_moves_itself_and_a_card_to_the_mat_and_scores() {
    let mut g = setup(&[Card::Island, Card::ThroneRoom]);
    hand(
        &mut g,
        P0,
        &[Card::ThroneRoom, Card::Island, Card::Estate, Card::Copper],
    );
    g.start();
    let before = g.score(P0);
    play(&mut g, Card::ThroneRoom);
    choose_kinds(&mut g, &[Card::Island]);
    choose_kinds(&mut g, &[Card::Estate]);
    assert_eq!(
        mat_kinds(&g, P0, Mat::Island),
        [Card::Island, Card::Estate, Card::Copper]
    );
    assert_eq!(g.score(P0), before);
    assert_eq!(Card::Island.def().vp, 2);
    let v = g.view(P1);
    let island = v.player(P0).mat(Mat::Island).unwrap();
    let seen: Vec<Option<Card>> = island.iter().map(|&c| v.card(c)).collect();
    assert_eq!(
        seen,
        [Some(Card::Island), Some(Card::Estate), Some(Card::Copper)]
    );
}

#[test]
fn salvager_gives_coins_equal_to_the_trashed_cost() {
    let mut g = setup(&[Card::Salvager]);
    hand(&mut g, P0, &[Card::Salvager, Card::Gold, Card::Estate]);
    g.start();
    play(&mut g, Card::Salvager);
    choose_kinds(&mut g, &[Card::Gold]);
    assert_eq!((g.turn().coins, g.turn().buys), (6, 2));
}

/// "+$1 per $1 it costs" is counted after the trash and what it set off.
#[test]
fn salvager_counts_the_trashed_cards_cost_after_trashing_it() {
    let mut g = setup(&[Card::Salvager, Card::Destrier, Card::MarketSquare]);
    hand(
        &mut g,
        P0,
        &[Card::Salvager, Card::Destrier, Card::MarketSquare],
    );
    g.start();
    play(&mut g, Card::Salvager);
    choose_kinds(&mut g, &[Card::Destrier]);
    super::intrigue::trigger(&mut g, Card::MarketSquare);
    assert_eq!(g.cost(Card::Destrier), Cost::coins(5));
    assert_eq!(g.turn().coins, 5);
}

/// A "when you gain" effect set up while a card is being gained applies to that gain.
#[test]
fn a_sailor_played_by_a_gained_gondola_can_play_that_gondola() {
    let mut g = setup(&[Card::Sailor, Card::Gondola]);
    hand(&mut g, P0, &[Card::Sailor, Card::Gold, Card::Silver]);
    deck(&mut g, P0, &[Card::Copper; 20]);
    g.start();
    to_buy(&mut g);
    play(&mut g, Card::Gold);
    play(&mut g, Card::Silver);
    buy(&mut g, Card::Gondola);
    // Gondola's when-gain ability plays the Sailor.
    respond(&mut g, Response::option(Opt::Trigger(0)));
    assert_eq!(trigger_cards(&g), [Card::Sailor]);
    respond(&mut g, Response::option(Opt::Trigger(0)));
    // The Gondola, for its +$2 next turn.
    mode(&mut g, 1);
    assert_eq!(g.turn().player, P1);
    assert_eq!(in_play_kinds(&g, P0), [Card::Sailor, Card::Gondola]);
}

#[test]
fn sailor_plays_a_gained_duration_once_and_pays_next_turn() {
    let mut g = setup(&[Card::Sailor, Card::Caravan, Card::Festival]);
    hand(
        &mut g,
        P0,
        &[
            Card::Sailor,
            Card::Festival,
            Card::Gold,
            Card::Gold,
            Card::Silver,
        ],
    );
    deck(&mut g, P0, &[Card::Copper; 15]);
    g.start();
    play(&mut g, Card::Sailor);
    play(&mut g, Card::Festival);
    to_buy(&mut g);
    play(&mut g, Card::Gold);
    play(&mut g, Card::Gold);
    play(&mut g, Card::Silver);
    buy(&mut g, Card::Caravan);
    assert_eq!(purpose(&g), Purpose::Trigger);
    respond(&mut g, Response::option(Opt::Trigger(0)));
    assert_eq!(in_play_kinds(&g, P0).last(), Some(&Card::Caravan));
    // Once per turn: the second Caravan isn't offered.
    buy(&mut g, Card::Caravan);
    assert_eq!(g.turn().player, P1);
    next_turn(&mut g);
    resolve_triggers(&mut g);
    // Sailor: +$2 and may trash a card.
    assert_eq!(purpose(&g), Purpose::Trash);
    choose_kinds(&mut g, &[Card::Copper]);
    resolve_triggers(&mut g);
    assert_eq!(g.turn().coins, 2);
    assert_eq!(g.hand(P0).len(), 5);
}

#[test]
fn tide_pools_discards_two_at_the_start_of_next_turn() {
    let mut g = setup(&[Card::TidePools]);
    hand(&mut g, P0, &[Card::TidePools]);
    deck(&mut g, P0, &[Card::Copper, Card::Estate].repeat(8));
    g.start();
    play(&mut g, Card::TidePools);
    assert_eq!((g.hand(P0).len(), g.turn().actions), (3, 1));
    next_turn(&mut g);
    next_turn(&mut g);
    assert_eq!(purpose(&g), Purpose::Discard);
    choose_kinds(&mut g, &[Card::Estate, Card::Estate]);
    assert_eq!(g.hand(P0).len(), 3);
}

#[test]
fn treasure_map_needs_two_maps_trashed_for_four_golds() {
    let mut g = setup(&[Card::TreasureMap, Card::ThroneRoom]);
    hand(
        &mut g,
        P0,
        &[
            Card::ThroneRoom,
            Card::TreasureMap,
            Card::TreasureMap,
            Card::TreasureMap,
        ],
    );
    deck(&mut g, P0, &[Card::Copper]);
    g.start();
    play(&mut g, Card::ThroneRoom);
    choose_kinds(&mut g, &[Card::TreasureMap]);
    assert!(g.hand(P0).is_empty());
    assert_eq!(
        deck_kinds(&g, P0),
        [Card::Gold, Card::Gold, Card::Gold, Card::Gold, Card::Copper]
    );
    assert_eq!(
        kinds(&g, g.trash())
            .iter()
            .filter(|&&c| c == Card::TreasureMap)
            .count(),
        3
    );
}

#[test]
fn a_lone_treasure_map_trashes_itself_for_nothing() {
    let mut g = setup(&[Card::TreasureMap]);
    hand(&mut g, P0, &[Card::TreasureMap]);
    deck(&mut g, P0, &[]);
    g.start();
    play(&mut g, Card::TreasureMap);
    assert_eq!(kinds(&g, g.trash()).last(), Some(&Card::TreasureMap));
    assert!(g.player(P0).deck.is_empty());
}

#[test]
fn corsair_trashes_the_first_silver_or_gold_each_other_player_plays() {
    let mut g = setup(&[Card::Corsair]);
    hand(&mut g, P0, &[Card::Corsair]);
    deck(&mut g, P0, &[Card::Copper; 15]);
    hand(&mut g, P1, &[Card::Silver, Card::Gold, Card::Silver]);
    g.start();
    play(&mut g, Card::Corsair);
    assert_eq!(g.turn().coins, 2);
    next_turn(&mut g);
    to_buy(&mut g);
    play(&mut g, Card::Silver);
    assert_eq!(kinds(&g, g.trash()).last(), Some(&Card::Silver));
    assert_eq!(g.turn().coins, 2);
    play(&mut g, Card::Gold);
    play(&mut g, Card::Silver);
    assert_eq!(g.turn().coins, 7);
    assert_eq!(in_play_kinds(&g, P1), [Card::Gold, Card::Silver]);
    next_turn(&mut g);
    assert_eq!(g.hand(P0).len(), 6);
    assert_eq!(in_play_kinds(&g, P0), [Card::Corsair]);
}

/// Corsair says "each turn", as Highwayman does: "this can affect a Treasure that another
/// player plays during your turn" (wiki, Highwayman).
#[test]
fn corsair_trashes_a_silver_another_player_plays_on_its_players_turn() {
    let mut g = setup(&[
        Card::Corsair,
        Card::Ambassador,
        Card::Gondola,
        Card::Storyteller,
        Card::Festival,
    ]);
    hand(
        &mut g,
        P0,
        &[
            Card::Festival,
            Card::Corsair,
            Card::Ambassador,
            Card::Gondola,
        ],
    );
    hand(&mut g, P1, &[Card::Storyteller, Card::Silver]);
    g.start();
    play(&mut g, Card::Festival);
    play(&mut g, Card::Corsair);
    play_on_another_players_turn(&mut g);
    // P1's Storyteller plays the Silver.
    choose_kinds(&mut g, &[Card::Silver]);
    assert_eq!(g.turn().player, P0);
    assert_eq!(kinds(&g, g.trash()).last(), Some(&Card::Silver));
}

#[test]
fn two_corsairs_still_trash_only_one_treasure() {
    let mut g = setup(&[Card::Corsair, Card::Village]);
    hand(&mut g, P0, &[Card::Village, Card::Corsair, Card::Corsair]);
    deck(&mut g, P0, &[Card::Copper; 15]);
    hand(&mut g, P1, &[Card::Silver, Card::Gold]);
    g.start();
    play(&mut g, Card::Village);
    play(&mut g, Card::Corsair);
    play(&mut g, Card::Corsair);
    next_turn(&mut g);
    to_buy(&mut g);
    play(&mut g, Card::Gold);
    play(&mut g, Card::Silver);
    assert_eq!(in_play_kinds(&g, P1), [Card::Silver]);
}

#[test]
fn a_player_who_revealed_moat_to_corsair_keeps_their_silver() {
    let mut g = setup(&[Card::Corsair, Card::Moat]);
    hand(&mut g, P0, &[Card::Corsair]);
    hand(&mut g, P1, &[Card::Moat, Card::Silver]);
    g.start();
    play(&mut g, Card::Corsair);
    respond(&mut g, Response::option(Opt::Trigger(0)));
    next_turn(&mut g);
    to_buy(&mut g);
    play(&mut g, Card::Silver);
    assert_eq!(in_play_kinds(&g, P1), [Card::Silver]);
}

#[test]
fn outpost_gives_an_extra_turn_with_a_three_card_hand_but_not_a_third() {
    let mut g = setup(&[Card::Outpost, Card::MerchantShip]);
    hand(&mut g, P0, &[Card::Outpost, Card::MerchantShip]);
    deck(
        &mut g,
        P0,
        &[
            Card::Outpost,
            Card::Copper,
            Card::Copper,
            Card::Estate,
            Card::Estate,
            Card::Estate,
            Card::Estate,
            Card::Estate,
            Card::Estate,
        ],
    );
    g.start();
    play(&mut g, Card::Outpost);
    next_turn(&mut g);
    assert_eq!((g.turn().player, g.turn().extra), (P0, Some(Card::Outpost)));
    assert_eq!(g.hand(P0).len(), 3);
    assert_eq!(in_play_kinds(&g, P0), [Card::Outpost]);
    play(&mut g, Card::Outpost);
    next_turn(&mut g);
    assert_eq!(g.turn().player, P1);
    // The Outpost played on the extra turn still shrinks the next hand. Its turn fails
    // only between turns ("You can't be sure Outpost will fail until you are in between
    // turns", wiki, Outpost), so it is discarded in the next Clean-up, P1's.
    assert_eq!(g.hand(P0).len(), 3);
    assert_eq!(in_play_kinds(&g, P0), [Card::Outpost]);
    next_turn(&mut g);
    assert!(g.in_play(P0).is_empty());
}

/// Outpost played on another player's turn: "you'll draw 3 cards whenever your next
/// Clean-up phase is" (wiki, Outpost), the one of its extra turn.
#[test]
fn outpost_played_on_another_players_turn_shrinks_its_players_next_hand() {
    let mut g = setup(&[Card::Outpost, Card::Ambassador, Card::Gondola]);
    hand(&mut g, P0, &[Card::Ambassador, Card::Gondola]);
    hand(&mut g, P1, &[Card::Outpost]);
    deck(&mut g, P1, &[Card::Copper; 10]);
    g.start();
    play_on_another_players_turn(&mut g);
    next_turn(&mut g);
    assert_eq!((g.turn().player, g.turn().extra), (P1, Some(Card::Outpost)));
    assert_eq!(in_play_kinds(&g, P1), [Card::Outpost]);
    next_turn(&mut g);
    assert_eq!((g.turn().player, g.turn().extra), (P1, None));
    assert_eq!(g.hand(P1).len(), 3);
    assert!(g.in_play(P1).is_empty());
}

#[test]
fn outpost_turn_is_the_next_turn_for_durations() {
    let mut g = setup(&[Card::Outpost, Card::MerchantShip, Card::Village]);
    hand(
        &mut g,
        P0,
        &[Card::Village, Card::Outpost, Card::MerchantShip],
    );
    deck(&mut g, P0, &[Card::Copper; 10]);
    g.start();
    play(&mut g, Card::Village);
    play(&mut g, Card::Outpost);
    play(&mut g, Card::MerchantShip);
    next_turn(&mut g);
    assert_eq!(g.turn().extra, Some(Card::Outpost));
    assert_eq!(g.turn().coins, 2);
    next_turn(&mut g);
    assert_eq!(g.turn().player, P1);
    assert!(g.in_play(P0).is_empty());
}

/// "Throne Room stays in play with Outpost even though you won't get a second extra turn.
/// ... If Outpost fails ... you discard it during Clean-up of the next turn, whoever's
/// turn it is." (wiki, Outpost)
#[test]
fn throne_room_outpost_gives_one_extra_turn() {
    let mut g = setup(&[Card::Outpost, Card::ThroneRoom]);
    hand(&mut g, P0, &[Card::ThroneRoom, Card::Outpost]);
    deck(&mut g, P0, &[Card::Copper; 10]);
    g.start();
    play(&mut g, Card::ThroneRoom);
    choose_kinds(&mut g, &[Card::Outpost]);
    next_turn(&mut g);
    assert_eq!(g.hand(P0).len(), 3);
    assert_eq!(in_play_kinds(&g, P0), [Card::ThroneRoom, Card::Outpost]);
    next_turn(&mut g);
    assert_eq!((g.turn().player, g.turn().extra), (P1, None));
    assert_eq!(in_play_kinds(&g, P0), [Card::ThroneRoom, Card::Outpost]);
    next_turn(&mut g);
    assert!(g.in_play(P0).is_empty());
}

/// "If you play Outpost and Voyage ... you choose to take the Voyage turn next. ... In
/// Clean-up of the Voyage turn, you discard Voyage but not Outpost and draw 5 cards. In
/// between turns again, Outpost now fails ... The next player takes their turn and during
/// their Clean-up, you discard Outpost." (wiki, Outpost)
#[test]
fn outpost_stays_in_play_until_its_own_turn_fails() {
    let mut g = setup(&[Card::Outpost, Card::Odysseys, Card::Village]);
    hand(&mut g, P0, &[Card::Village, Card::Outpost, Card::Voyage]);
    deck(&mut g, P0, &[Card::Copper; 30]);
    g.start();
    play(&mut g, Card::Village);
    play(&mut g, Card::Outpost);
    play(&mut g, Card::Voyage);
    to_buy(&mut g);
    end_phase(&mut g);
    respond(&mut g, Response::option(Opt::Card(Card::Voyage)));
    assert_eq!((g.turn().player, g.turn().extra), (P0, Some(Card::Voyage)));
    assert_eq!(in_play_kinds(&g, P0), [Card::Outpost, Card::Voyage]);
    next_turn(&mut g);
    assert_eq!((g.turn().player, g.turn().extra), (P1, None));
    assert_eq!(g.hand(P0).len(), 5);
    assert_eq!(in_play_kinds(&g, P0), [Card::Outpost]);
    next_turn(&mut g);
    assert!(g.in_play(P0).is_empty());
}

/// "You can take 2 Outpost turns in a row if the preceding turn wasn't yours" (wiki,
/// Outpost): a Throne Room played on another player's turn keeps itself and the Outpost
/// in play until the second.
#[test]
fn throne_room_outpost_played_on_another_players_turn_stays_for_both_outpost_turns() {
    let mut g = setup(&[
        Card::Outpost,
        Card::ThroneRoom,
        Card::Ambassador,
        Card::Gondola,
    ]);
    hand(&mut g, P0, &[Card::Ambassador, Card::Gondola]);
    hand(&mut g, P1, &[Card::ThroneRoom, Card::Outpost]);
    deck(&mut g, P1, &[Card::Copper; 20]);
    g.start();
    play_on_another_players_turn(&mut g);
    choose_kinds(&mut g, &[Card::ThroneRoom]);
    choose_kinds(&mut g, &[Card::Outpost]);
    next_turn(&mut g);
    assert_eq!((g.turn().player, g.turn().extra), (P1, Some(Card::Outpost)));
    next_turn(&mut g);
    assert_eq!((g.turn().player, g.turn().extra), (P1, Some(Card::Outpost)));
    assert_eq!(in_play_kinds(&g, P1), [Card::ThroneRoom, Card::Outpost]);
    next_turn(&mut g);
    assert_eq!((g.turn().player, g.turn().extra), (P1, None));
    assert!(g.in_play(P1).is_empty());
}

#[test]
fn lich_skips_the_first_of_two_outpost_turns_and_the_second_happens() {
    let mut g = setup(&[Card::Outpost, Card::Wizards]);
    hand(&mut g, P0, &[Card::Lich, Card::Outpost, Card::Outpost]);
    deck(&mut g, P0, &[Card::Copper; 20]);
    g.start();
    play(&mut g, Card::Lich);
    play(&mut g, Card::Outpost);
    play(&mut g, Card::Outpost);
    next_turn(&mut g);
    assert_eq!((g.turn().player, g.turn().extra), (P0, Some(Card::Outpost)));
    next_turn(&mut g);
    assert_eq!(g.turn().player, P1);
}

#[test]
fn outpost_on_a_turn_right_after_an_extra_turn_of_its_player_is_a_third_turn_in_a_row() {
    let mut g = setup(&[Card::Outpost]);
    deck(&mut g, P1, &[Card::Outpost; 10]);
    g.start();
    // As if P1 had set up an extra turn of their own during P0's turn.
    g.extra_turns.push_back(ExtraTurn {
        player: P1,
        source: Card::Mission,
        limit: ExtraLimit::ThirdInARow,
        lasting: None,
    });
    next_turn(&mut g);
    assert_eq!((g.turn().player, g.turn().extra), (P1, Some(Card::Mission)));
    next_turn(&mut g);
    assert_eq!((g.turn().player, g.turn().extra), (P1, None));
    play(&mut g, Card::Outpost);
    next_turn(&mut g);
    assert_eq!(g.turn().player, P0);
}

#[test]
fn pirate_is_played_when_anyone_gains_a_treasure_and_gains_one_next_turn() {
    let mut g = setup(&[Card::Pirate]);
    hand(&mut g, P0, &[Card::Copper, Card::Copper, Card::Copper]);
    hand(&mut g, P1, &[Card::Pirate, Card::Estate]);
    deck(&mut g, P1, &[Card::Copper; 10]);
    g.start();
    to_buy(&mut g);
    for _ in 0..3 {
        play(&mut g, Card::Copper);
    }
    buy(&mut g, Card::Silver);
    let d = g.pending().unwrap();
    assert_eq!((d.player, d.purpose), (P1, Purpose::Trigger));
    respond(&mut g, Response::option(Opt::Trigger(0)));
    assert_eq!(in_play_kinds(&g, P1), [Card::Pirate]);
    // P0's turn ended; Pirate resolves at the start of P1's.
    assert_eq!(g.turn().player, P1);
    assert_eq!(purpose(&g), Purpose::Gain);
    choose_pile(&mut g, Card::Gold);
    assert_eq!(hand_kinds(&g, P1), [Card::Estate, Card::Gold]);
    next_turn(&mut g);
    assert!(g.in_play(P1).is_empty());
}

#[test]
fn a_pirate_played_at_the_start_of_a_turn_waits_for_the_next_one() {
    let mut g = setup(&[Card::Pirate]);
    hand(&mut g, P0, &[Card::Copper, Card::Copper, Card::Copper]);
    hand(&mut g, P1, &[Card::Pirate, Card::Pirate, Card::Estate]);
    deck(&mut g, P1, &[Card::Copper; 10]);
    g.start();
    to_buy(&mut g);
    for _ in 0..3 {
        play(&mut g, Card::Copper);
    }
    buy(&mut g, Card::Silver);
    respond(&mut g, Response::option(Opt::Trigger(0)));
    no(&mut g);
    assert_eq!(g.turn().player, P1);
    choose_pile(&mut g, Card::Gold);
    // Gaining the Gold lets P1 play the other Pirate, at the start of their turn.
    assert_eq!(purpose(&g), Purpose::Trigger);
    respond(&mut g, Response::option(Opt::Trigger(0)));
    assert_eq!(in_play_kinds(&g, P1), [Card::Pirate, Card::Pirate]);
    assert_eq!(g.pending().unwrap().purpose, Purpose::Turn);
    assert_eq!(hand_kinds(&g, P1), [Card::Estate, Card::Gold]);
    next_turn(&mut g);
    // Only the first Pirate is discarded.
    assert_eq!(in_play_kinds(&g, P1), [Card::Pirate]);
    next_turn(&mut g);
    assert_eq!(g.turn().player, P1);
    assert_eq!(purpose(&g), Purpose::Gain);
}

#[test]
fn pirate_can_be_declined() {
    let mut g = setup(&[Card::Pirate]);
    hand(&mut g, P0, &[Card::Copper, Card::Copper, Card::Copper]);
    hand(&mut g, P1, &[Card::Pirate]);
    g.start();
    to_buy(&mut g);
    for _ in 0..3 {
        play(&mut g, Card::Copper);
    }
    buy(&mut g, Card::Silver);
    no(&mut g);
    assert_eq!(hand_kinds(&g, P1)[0], Card::Pirate);
}

#[test]
fn sea_witch_curses_and_sifts_next_turn() {
    let mut g = setup(&[Card::SeaWitch]);
    hand(&mut g, P0, &[Card::SeaWitch]);
    deck(&mut g, P0, &[Card::Copper, Card::Estate].repeat(8));
    g.start();
    play(&mut g, Card::SeaWitch);
    assert_eq!(g.hand(P0).len(), 2);
    assert_eq!(g.count_owned(P1, Card::Curse), 1);
    next_turn(&mut g);
    next_turn(&mut g);
    assert_eq!(purpose(&g), Purpose::Discard);
    assert_eq!(g.hand(P0).len(), 7);
    choose_kinds(&mut g, &[Card::Estate, Card::Estate]);
    assert_eq!(g.hand(P0).len(), 5);
}

#[test]
fn treasury_may_go_on_the_deck_without_a_victory_gain_in_the_buy_phase() {
    let mut g = setup(&[Card::Treasury]);
    hand(&mut g, P0, &[Card::Treasury, Card::Silver]);
    deck(&mut g, P0, &[Card::Copper; 10]);
    g.start();
    play(&mut g, Card::Treasury);
    to_buy(&mut g);
    play(&mut g, Card::Silver);
    buy(&mut g, Card::Silver);
    assert_eq!(purpose(&g), Purpose::Trigger);
    respond(&mut g, Response::option(Opt::Trigger(0)));
    assert!(hand_kinds(&g, P0).contains(&Card::Treasury));
}

#[test]
fn treasury_cannot_go_on_the_deck_after_gaining_a_victory_card() {
    let mut g = setup(&[Card::Treasury]);
    hand(&mut g, P0, &[Card::Treasury, Card::Copper]);
    deck(&mut g, P0, &[Card::Copper; 10]);
    g.start();
    play(&mut g, Card::Treasury);
    to_buy(&mut g);
    play(&mut g, Card::Copper);
    play(&mut g, Card::Copper);
    buy(&mut g, Card::Estate);
    assert_eq!(g.turn().player, P1);
    assert!(discard_kinds(&g, P0).contains(&Card::Treasury));
}

/// A Treasury played on another player's turn is discarded in that turn's Clean-up, and
/// its player had no Buy phase to gain a Victory card in.
#[test]
fn treasury_played_on_another_players_turn_may_go_on_the_deck() {
    let mut g = setup(&[Card::Treasury, Card::Ambassador, Card::Gondola]);
    hand(&mut g, P0, &[Card::Ambassador, Card::Gondola]);
    hand(&mut g, P1, &[Card::Treasury]);
    deck(&mut g, P1, &[Card::Copper; 5]);
    g.start();
    play_on_another_players_turn(&mut g);
    to_buy(&mut g);
    play(&mut g, Card::Gondola);
    mode(&mut g, 0);
    buy(&mut g, Card::Estate);
    assert_eq!(g.pending().unwrap().player, P1);
    assert_eq!(trigger_cards(&g), [Card::Treasury]);
    respond(&mut g, Response::option(Opt::Trigger(0)));
    assert_eq!(g.turn().player, P1);
    assert_eq!(deck_kinds(&g, P1)[0], Card::Treasury);
}

#[test]
fn treasury_ignores_victory_cards_gained_outside_the_buy_phase() {
    let mut g = setup(&[Card::Treasury, Card::Workshop]);
    hand(&mut g, P0, &[Card::Treasury, Card::Workshop]);
    deck(&mut g, P0, &[Card::Copper; 10]);
    g.start();
    play(&mut g, Card::Treasury);
    play(&mut g, Card::Workshop);
    choose_pile(&mut g, Card::Estate);
    to_buy(&mut g);
    end_phase(&mut g);
    assert_eq!(purpose(&g), Purpose::Trigger);
    respond(&mut g, Response::option(Opt::Trigger(0)));
    assert!(hand_kinds(&g, P0).contains(&Card::Treasury));
}

#[test]
fn wharf_twice_with_throne_room() {
    let mut g = setup(&[Card::Wharf, Card::ThroneRoom]);
    hand(&mut g, P0, &[Card::ThroneRoom, Card::Wharf]);
    deck(&mut g, P0, &[Card::Copper; 20]);
    g.start();
    play(&mut g, Card::ThroneRoom);
    choose_kinds(&mut g, &[Card::Wharf]);
    assert_eq!((g.hand(P0).len(), g.turn().buys), (4, 3));
    next_turn(&mut g);
    next_turn(&mut g);
    resolve_triggers(&mut g);
    assert_eq!((g.hand(P0).len(), g.turn().buys), (9, 3));
    // Each play's next-turn part says whose it is.
    let part = [
        Event::Resolved {
            player: P0,
            card: Card::Wharf,
        },
        Event::Bonus {
            player: P0,
            actions: 0,
            buys: 1,
            coins: 0,
        },
        Event::Drew {
            player: P0,
            count: 2,
        },
    ];
    assert!(logged_in_order(&g, &[&part[..], &part[..]].concat()));
}

#[test]
fn ambassador_returns_copies_and_each_other_player_gains_one() {
    let mut g = setup(&[Card::Ambassador]);
    hand(
        &mut g,
        P0,
        &[Card::Ambassador, Card::Estate, Card::Estate, Card::Copper],
    );
    g.start();
    let estates = g.supply_pile(Card::Estate).unwrap();
    let before = g.pile(estates).cards.len();
    play(&mut g, Card::Ambassador);
    choose_kinds(&mut g, &[Card::Estate]);
    assert_eq!(purpose(&g), Purpose::Return);
    choose_kinds(&mut g, &[Card::Estate, Card::Estate]);
    assert_eq!(hand_kinds(&g, P0), [Card::Copper]);
    assert_eq!(g.pile(estates).cards.len(), before + 1);
    assert_eq!(discard_kinds(&g, P1), [Card::Estate]);
}

#[test]
fn embargo_trashes_itself_to_add_a_token_that_curses_buyers() {
    let mut g = setup(&[Card::Embargo, Card::ThroneRoom]);
    hand(&mut g, P0, &[Card::ThroneRoom, Card::Embargo]);
    hand(&mut g, P1, &[Card::Copper, Card::Copper, Card::Copper]);
    g.start();
    play(&mut g, Card::ThroneRoom);
    choose_kinds(&mut g, &[Card::Embargo]);
    choose_pile(&mut g, Card::Silver);
    assert_eq!(g.turn().coins, 4);
    assert_eq!(kinds(&g, g.trash()).last(), Some(&Card::Embargo));
    let silver = g.supply_pile(Card::Silver).unwrap();
    assert_eq!(g.pile(silver).embargo, 1);
    next_turn(&mut g);
    to_buy(&mut g);
    for _ in 0..3 {
        play(&mut g, Card::Copper);
    }
    buy(&mut g, Card::Silver);
    assert_eq!(g.count_owned(P1, Card::Curse), 1);
}

#[test]
fn buying_an_embargoed_curse_gains_it_and_a_curse_per_token() {
    let mut g = setup(&[Card::Embargo]);
    let curses = g.supply_pile(Card::Curse).unwrap();
    g.piles[curses.index()].embargo = 2;
    hand(&mut g, P0, &[]);
    g.start();
    buy(&mut g, Card::Curse);
    assert_eq!(g.count_owned(P0, Card::Curse), 3);
}

#[test]
fn an_embargo_token_added_while_a_buy_resolves_adds_one_curse() {
    let mut g = setup(&[Card::Embargo, Card::Talisman]);
    g.players[0].projects.push(Card::Innovation);
    let embargoes = g.supply_pile(Card::Embargo).unwrap();
    g.piles[embargoes.index()].embargo = 1;
    hand(&mut g, P0, &[Card::Talisman, Card::Copper]);
    g.start();
    play(&mut g, Card::Talisman);
    play(&mut g, Card::Copper);
    buy(&mut g, Card::Embargo);
    super::intrigue::trigger(&mut g, Card::Embargo);
    // Talisman's copy, played with Innovation, puts a second token on the pile.
    super::intrigue::trigger(&mut g, Card::Innovation);
    choose_pile(&mut g, Card::Embargo);
    assert_eq!(g.pile(embargoes).embargo, 2);
    assert_eq!(g.count_owned(P0, Card::Curse), 2);
}

#[test]
fn explorer_gains_gold_for_a_province_else_silver_to_hand() {
    let mut g = setup(&[Card::Explorer]);
    hand(
        &mut g,
        P0,
        &[Card::Explorer, Card::Explorer, Card::Province],
    );
    g.start();
    play(&mut g, Card::Explorer);
    assert_eq!(purpose(&g), Purpose::Reveal);
    choose_kinds(&mut g, &[Card::Province]);
    assert!(hand_kinds(&g, P0).contains(&Card::Gold));
    let mut g = setup(&[Card::Explorer]);
    hand(&mut g, P0, &[Card::Explorer]);
    g.start();
    play(&mut g, Card::Explorer);
    assert_eq!(hand_kinds(&g, P0), [Card::Silver]);
}

#[test]
fn ghost_ship_makes_others_put_cards_back_down_to_three() {
    let mut g = setup(&[Card::GhostShip]);
    hand(&mut g, P0, &[Card::GhostShip]);
    deck(&mut g, P0, &[Card::Copper; 5]);
    hand(
        &mut g,
        P1,
        &[
            Card::Gold,
            Card::Estate,
            Card::Copper,
            Card::Silver,
            Card::Curse,
        ],
    );
    deck(&mut g, P1, &[Card::Province]);
    g.start();
    play(&mut g, Card::GhostShip);
    assert_eq!(g.pending().unwrap().player, P1);
    let curse = find(&g, P1, Card::Curse);
    let estate = find(&g, P1, Card::Estate);
    choose(&mut g, &[curse, estate]);
    assert_eq!(
        deck_kinds(&g, P1),
        [Card::Curse, Card::Estate, Card::Province]
    );
    assert_eq!(g.hand(P1).len(), 3);
}

#[test]
fn navigator_discards_all_or_reorders() {
    let mut g = setup(&[Card::Navigator]);
    hand(
        &mut g,
        P0,
        &[Card::Navigator, Card::Navigator, Card::Village],
    );
    deck(
        &mut g,
        P0,
        &[
            Card::Estate,
            Card::Gold,
            Card::Copper,
            Card::Curse,
            Card::Silver,
            Card::Province,
        ],
    );
    g.start();
    play(&mut g, Card::Navigator);
    mode(&mut g, 1);
    let order: Vec<Card> = vec![
        Card::Gold,
        Card::Silver,
        Card::Copper,
        Card::Estate,
        Card::Curse,
    ];
    let mut left = candidates(&g);
    let chosen: Vec<CardId> = order
        .iter()
        .map(|&k| {
            let i = left.iter().position(|&c| g.card(c) == k).unwrap();
            left.remove(i)
        })
        .collect();
    choose(&mut g, &chosen);
    assert_eq!(
        deck_kinds(&g, P0),
        [
            Card::Gold,
            Card::Silver,
            Card::Copper,
            Card::Estate,
            Card::Curse,
            Card::Province
        ]
    );
    assert_eq!(g.turn().coins, 2);
}

#[test]
fn navigator_can_discard_all_five() {
    let mut g = setup(&[Card::Navigator]);
    hand(&mut g, P0, &[Card::Navigator]);
    deck(&mut g, P0, &[Card::Estate; 6]);
    g.start();
    play(&mut g, Card::Navigator);
    mode(&mut g, 0);
    assert_eq!(g.player(P0).discard.len(), 5);
    assert_eq!(g.player(P0).deck.len(), 1);
}

#[test]
fn pearl_diver_may_move_the_bottom_card_to_the_top() {
    let mut g = setup(&[Card::PearlDiver]);
    hand(&mut g, P0, &[Card::PearlDiver]);
    deck(
        &mut g,
        P0,
        &[Card::Copper, Card::Estate, Card::Estate, Card::Gold],
    );
    g.start();
    play(&mut g, Card::PearlDiver);
    assert_eq!(kinds(&g, &candidates(&g)), [Card::Gold]);
    let bottom = candidates(&g);
    choose(&mut g, &bottom);
    assert_eq!(deck_kinds(&g, P0), [Card::Gold, Card::Estate, Card::Estate]);
    // Everyone sees that a card was moved, not which one.
    assert_eq!(
        g.log().last(),
        Some(&Event::TopDecked {
            player: P0,
            card: None
        })
    );
}

#[test]
fn pirate_ship_trashes_treasures_for_tokens_then_pays_per_token() {
    let mut g = setup(&[Card::PirateShip]);
    hand(&mut g, P0, &[Card::PirateShip]);
    deck(
        &mut g,
        P0,
        &[
            Card::PirateShip,
            Card::Copper,
            Card::Copper,
            Card::Copper,
            Card::Copper,
        ],
    );
    deck(
        &mut g,
        P1,
        &[Card::Silver, Card::Gold, Card::Copper, Card::Copper],
    );
    g.start();
    play(&mut g, Card::PirateShip);
    mode(&mut g, 1);
    assert_eq!(g.pending().unwrap().player, P0);
    choose_kinds(&mut g, &[Card::Gold]);
    assert_eq!(kinds(&g, g.trash()).last(), Some(&Card::Gold));
    assert_eq!(discard_kinds(&g, P1), [Card::Silver]);
    assert_eq!(g.player(P0).pirate_ship, 1);
    next_turn(&mut g);
    next_turn(&mut g);
    play(&mut g, Card::PirateShip);
    mode(&mut g, 0);
    assert_eq!(g.turn().coins, 1);
}

#[test]
fn pirate_ship_gets_no_token_if_nothing_was_trashed() {
    let mut g = setup(&[Card::PirateShip]);
    hand(&mut g, P0, &[Card::PirateShip]);
    deck(&mut g, P1, &[Card::Estate, Card::Estate]);
    g.start();
    play(&mut g, Card::PirateShip);
    mode(&mut g, 1);
    assert_eq!(g.player(P0).pirate_ship, 0);
    assert_eq!(discard_kinds(&g, P1), [Card::Estate, Card::Estate]);
}

#[test]
fn sea_hag_discards_the_top_card_and_puts_a_curse_on_the_deck() {
    let mut g = setup(&[Card::SeaHag]);
    hand(&mut g, P0, &[Card::SeaHag]);
    deck(&mut g, P1, &[Card::Gold, Card::Copper]);
    g.start();
    play(&mut g, Card::SeaHag);
    assert_eq!(discard_kinds(&g, P1), [Card::Gold]);
    assert_eq!(deck_kinds(&g, P1), [Card::Curse, Card::Copper]);
}

/// Random games over Seaside (removed cards included) with Throne Room and Moat, each
/// player starting with a copy of every Kingdom card so that all of them get played.
#[test]
fn random_seaside_games_keep_invariants() {
    let pool: Vec<Card> = Card::in_set(Set::Seaside)
        .filter(|c| c.def().kingdom)
        .collect();
    for seed in 0..fuzz_seeds(120) {
        let mut rng = ChaCha8Rng::seed_from_u64(seed);
        let mut cards = pool.clone();
        cards.shuffle(&mut rng);
        cards.truncate(8);
        cards.extend([Card::ThroneRoom, Card::Moat]);
        let players = rng.random_range(2..=4);
        let mut config = GameConfig::new(players, cards.clone(), seed);
        config.turn_limit = Some(60);
        let mut g = setup_with(config);
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

/// Monkey's page: "The Duration effect ends instantaneously when your next turn starts.
/// So if another player gains a card at the start of your turn (e.g. you play a Witch
/// with Prepare), Monkey won't draw you a card from that."
#[test]
fn monkey_stops_drawing_when_its_owners_next_turn_starts() {
    let mut g = setup(&[Card::Monkey, Card::Witch, Card::Farmhands]);
    hand(
        &mut g,
        P0,
        &[Card::Monkey, Card::Witch, Card::Gold, Card::Copper],
    );
    deck(&mut g, P0, &[Card::Copper; 12]);
    hand(&mut g, P1, &[]);
    deck(&mut g, P1, &[Card::Copper; 12]);
    g.start();
    play(&mut g, Card::Monkey);
    // No Action play is left for the Witch: the Buy phase has begun.
    assert_eq!(g.turn().phase, Phase::Buy);
    play(&mut g, Card::Gold);
    play(&mut g, Card::Copper);
    buy(&mut g, Card::Farmhands);
    // Farmhands sets the Witch aside to play at the start of the next turn.
    super::intrigue::trigger(&mut g, Card::Farmhands);
    // With no Buy left the turn ends.
    assert_eq!(g.turn().player, P1);
    next_turn(&mut g);
    assert_eq!(g.turn().player, P0);
    assert_eq!(g.hand(P0).len(), 5);
    // The Witch first: P1's Curse comes after Monkey's "until your next turn" is over.
    super::intrigue::trigger(&mut g, Card::Farmhands);
    assert_eq!(g.count_owned(P1, Card::Curse), 1);
    assert_eq!(
        g.hand(P0).len(),
        8,
        "5, Witch's +2 Cards and Monkey's +1 Card at the start of the turn"
    );
}

/// Smugglers' page: "First you choose a card that the player to your right gained
/// costing up to $6, and then you (try to) gain a copy of it from the Supply. This means
/// you may be able to choose a card that is unavailable to be gained (e.g., a card whose
/// Supply pile is empty, or a card from outside the Supply); and if you do, you gain
/// nothing."
#[test]
fn smugglers_may_choose_a_gained_card_it_cannot_gain() {
    let mut g = setup(&[Card::Smugglers, Card::BanditCamp]);
    hand(&mut g, P0, &[]);
    let mut cards = vec![Card::Smugglers];
    cards.extend([Card::Copper; 6]);
    deck(&mut g, P0, &cards);
    hand(&mut g, P1, &[Card::BanditCamp]);
    deck(&mut g, P1, &[Card::Copper; 12]);
    g.start();
    next_turn(&mut g);
    play(&mut g, Card::BanditCamp);
    buy(&mut g, Card::Curse);
    // With no Buy left the turn ends.
    assert_eq!(g.turn().player, P0);
    play(&mut g, Card::Smugglers);
    let DecisionKind::Choose { options, .. } = &g.pending().unwrap().kind else {
        panic!("{:?}", g.pending())
    };
    assert_eq!(options[..], [Card::Spoils, Card::Curse].map(Opt::Card));
    // The Spoils isn't in the Supply: nothing is gained.
    respond(&mut g, Response::option(Opt::Card(Card::Spoils)));
    assert_eq!(g.count_owned(P0, Card::Spoils), 0);
    assert_eq!(g.count_owned(P0, Card::Curse), 0);
    assert_eq!(purpose(&g), Purpose::Turn);
}
