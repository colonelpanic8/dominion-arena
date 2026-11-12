use super::cornucopia_guilds::fuzz_sets;
use super::*;

#[test]
fn random_games_with_every_card() {
    let plays = fuzz_sets(&[Set::Hinterlands], &[Card::Province, Card::Gold], 300);
    for card in Card::in_set(Set::Hinterlands) {
        if !card.is(Types::VICTORY) {
            assert!(plays.iter().any(|&(c, _)| c == card), "{card} never played");
        }
    }
}

#[test]
fn random_games_mixing_sets() {
    fuzz_sets(
        &[Set::Base, Set::CornucopiaGuilds, Set::Hinterlands],
        &[Card::Province],
        300,
    );
}

use crate::game::Zone;

fn discard_kinds(g: &Game, p: PlayerId) -> Vec<Card> {
    kinds(g, &g.player(p).discard)
}

fn mode(g: &mut Game, m: u8) {
    respond(g, Response::option(Opt::Mode(m)));
}

fn trigger(g: &mut Game, i: u16) {
    respond(g, Response::option(Opt::Trigger(i)));
}

/// Gains the top card of `card`'s pile for `p` as if by an effect.
fn gain(g: &mut Game, p: PlayerId, card: Card) {
    let pile = g.pile_of(card).unwrap();
    g.stack
        .push(crate::frame::Frame::gain(p, pile, Dest::Discard));
    g.pending = None;
    g.run(crate::frame::Input::Start);
}

fn offered_piles(g: &Game) -> Vec<Card> {
    match &g.pending().unwrap().kind {
        DecisionKind::Pile { piles, .. } => piles.iter().map(|&p| g.pile(p).card).collect(),
        k => panic!("expected a Pile decision, got {k:?}"),
    }
}

#[test]
fn berserker_gains_cheaper_and_attacks() {
    let mut g = setup(&[Card::Berserker]);
    hand(&mut g, P0, &[Card::Berserker]);
    g.start();
    play(&mut g, Card::Berserker);
    assert!(!offered_piles(&g).contains(&Card::Berserker));
    choose_pile(&mut g, Card::Silver);
    let d = g.pending().unwrap();
    assert_eq!((d.player, d.purpose), (P1, Purpose::Discard));
}

#[test]
fn berserker_plays_itself_when_gained_with_an_action_in_play() {
    let mut g = setup(&[Card::Berserker, Card::Village]);
    hand(&mut g, P0, &[Card::Village]);
    hand(&mut g, P1, &[Card::Copper, Card::Copper, Card::Copper]);
    deck(&mut g, P0, &[Card::Copper]);
    g.start();
    play(&mut g, Card::Village);
    gain(&mut g, P0, Card::Berserker);
    assert!(kinds(&g, g.in_play(P0)).contains(&Card::Berserker));
    assert_eq!(purpose(&g), Purpose::Gain);
}

#[test]
fn berserker_gained_with_no_action_in_play_stays_put() {
    let mut g = setup(&[Card::Berserker]);
    hand(&mut g, P0, &[Card::Gold, Card::Gold]);
    g.start();
    play(&mut g, Card::Gold);
    play(&mut g, Card::Gold);
    buy(&mut g, Card::Berserker);
    assert_eq!(g.count_owned(P0, Card::Berserker), 1);
    assert!(!g.log().iter().any(|e| matches!(
        e,
        Event::Played {
            card: Card::Berserker,
            ..
        }
    )));
}

#[test]
fn border_village_gains_a_cheaper_card() {
    let mut g = setup(&[Card::BorderVillage, Card::Laboratory]);
    hand(&mut g, P0, &[Card::Gold, Card::Gold]);
    g.start();
    play(&mut g, Card::Gold);
    play(&mut g, Card::Gold);
    buy(&mut g, Card::BorderVillage);
    let offered = offered_piles(&g);
    assert!(offered.contains(&Card::Laboratory) && !offered.contains(&Card::Gold));
    choose_pile(&mut g, Card::Laboratory);
    assert_eq!(g.count_owned(P0, Card::Laboratory), 1);
}

#[test]
fn cartographer_discards_and_reorders() {
    let mut g = setup(&[Card::Cartographer]);
    hand(&mut g, P0, &[Card::Cartographer]);
    deck(
        &mut g,
        P0,
        &[
            Card::Copper,
            Card::Estate,
            Card::Gold,
            Card::Curse,
            Card::Silver,
            Card::Duchy,
        ],
    );
    g.start();
    play(&mut g, Card::Cartographer);
    choose_kinds(&mut g, &[Card::Estate, Card::Curse]);
    choose_kinds(&mut g, &[Card::Silver, Card::Gold]);
    assert_eq!(deck_kinds(&g, P0), [Card::Silver, Card::Gold, Card::Duchy]);
    assert_eq!(discard_kinds(&g, P0), [Card::Estate, Card::Curse]);
}

#[test]
fn cauldron_curses_on_the_third_action_gained() {
    let mut g = setup(&[Card::Cauldron, Card::Village]);
    hand(
        &mut g,
        P0,
        &[Card::Cauldron, Card::Gold, Card::Gold, Card::Gold],
    );
    g.start();
    gain(&mut g, P0, Card::Village);
    play(&mut g, Card::Cauldron);
    play(&mut g, Card::Gold);
    play(&mut g, Card::Gold);
    play(&mut g, Card::Gold);
    buy(&mut g, Card::Village);
    assert_eq!(g.count_owned(P1, Card::Curse), 0);
    buy(&mut g, Card::Village);
    assert_eq!(g.count_owned(P1, Card::Curse), 1);
}

/// Under Enlightenment the Silvers gained are Actions.
#[test]
fn cauldron_counts_cards_that_are_actions_by_an_effect() {
    let mut g = setup(&[Card::Cauldron]);
    g.prophecy = Some((Card::Enlightenment, 0));
    hand(&mut g, P0, &[Card::Cauldron]);
    g.start();
    end_phase(&mut g);
    play(&mut g, Card::Cauldron);
    gain(&mut g, P0, Card::Silver);
    gain(&mut g, P0, Card::Silver);
    assert_eq!(g.count_owned(P1, Card::Curse), 0);
    gain(&mut g, P0, Card::Silver);
    assert_eq!(g.count_owned(P1, Card::Curse), 1);
}

#[test]
fn moat_protects_from_cauldron() {
    let mut g = setup(&[Card::Cauldron, Card::Village, Card::Moat]);
    hand(
        &mut g,
        P0,
        &[Card::Cauldron, Card::Gold, Card::Gold, Card::Gold],
    );
    hand(&mut g, P1, &[Card::Moat]);
    g.start();
    gain(&mut g, P0, Card::Village);
    gain(&mut g, P0, Card::Village);
    play(&mut g, Card::Cauldron);
    trigger(&mut g, 0);
    play(&mut g, Card::Gold);
    buy(&mut g, Card::Village);
    assert_eq!(g.count_owned(P1, Card::Curse), 0);
}

#[test]
fn crossroads_gives_actions_only_the_first_time() {
    let mut g = setup(&[Card::Crossroads, Card::ThroneRoom]);
    hand(
        &mut g,
        P0,
        &[
            Card::ThroneRoom,
            Card::Crossroads,
            Card::Estate,
            Card::Duchy,
        ],
    );
    deck(&mut g, P0, &[Card::Copper; 6]);
    g.start();
    play(&mut g, Card::ThroneRoom);
    choose_kinds(&mut g, &[Card::Crossroads]);
    assert_eq!(g.turn().actions, 3);
    assert_eq!(g.hand(P0).len(), 2 + 2 + 2);
}

#[test]
fn develop_gains_one_more_and_one_less_onto_the_deck() {
    let mut g = setup(&[Card::Develop, Card::Smithy]);
    hand(&mut g, P0, &[Card::Develop, Card::Smithy]);
    g.start();
    play(&mut g, Card::Develop);
    // Smithy ($4): a $5 card and a $3 card.
    let offered = offered_piles(&g);
    assert!(offered.contains(&Card::Duchy) && offered.contains(&Card::Silver));
    choose_pile(&mut g, Card::Silver);
    // Only a $5 card is left to gain, and Duchy is the only one.
    assert_eq!(deck_kinds(&g, P0)[..2], [Card::Duchy, Card::Silver]);
}

#[test]
fn farmland_upgrades_by_exactly_two_when_gained() {
    let mut g = setup(&[Card::Farmland, Card::Smithy]);
    hand(&mut g, P0, &[Card::Gold, Card::Gold, Card::Estate]);
    g.start();
    play(&mut g, Card::Gold);
    play(&mut g, Card::Gold);
    buy(&mut g, Card::Farmland);
    // Estate is the only card to trash. Estate ($2): Smithy is the only $4 card, gained at once.
    assert_eq!(g.count_owned(P0, Card::Smithy), 1);
}

#[test]
fn farmland_cannot_gain_a_farmland() {
    let mut g = setup(&[Card::Farmland]);
    hand(&mut g, P0, &[Card::Gold, Card::Gold, Card::Gold]);
    g.start();
    play(&mut g, Card::Gold);
    play(&mut g, Card::Gold);
    // The Gold left in hand ($6) would need an $8 card: only Province.
    buy(&mut g, Card::Farmland);
    assert_eq!(g.count_owned(P0, Card::Province), 1);
    assert_eq!(g.count_owned(P0, Card::Farmland), 1);
}

#[test]
fn fools_gold_is_worth_more_after_the_first() {
    let mut g = setup(&[Card::FoolsGold]);
    hand(
        &mut g,
        P0,
        &[Card::FoolsGold, Card::FoolsGold, Card::FoolsGold],
    );
    g.start();
    play(&mut g, Card::FoolsGold);
    assert_eq!(g.turn().coins, 1);
    play(&mut g, Card::FoolsGold);
    play(&mut g, Card::FoolsGold);
    assert_eq!(g.turn().coins, 9);
}

#[test]
fn fools_gold_reacts_to_another_player_gaining_a_province() {
    let mut g = setup(&[Card::FoolsGold]);
    hand(&mut g, P0, &[Card::Gold, Card::Gold, Card::Silver]);
    hand(&mut g, P1, &[Card::FoolsGold]);
    g.start();
    play(&mut g, Card::Gold);
    play(&mut g, Card::Gold);
    play(&mut g, Card::Silver);
    buy(&mut g, Card::Province);
    let d = g.pending().unwrap();
    assert_eq!((d.player, d.purpose), (P1, Purpose::Trigger));
    trigger(&mut g, 0);
    assert_eq!(deck_kinds(&g, P1)[0], Card::Gold);
    assert_eq!(kinds(&g, g.trash()).last(), Some(&Card::FoolsGold));
}

#[test]
fn guard_dog_reacts_to_an_attack_and_is_discarded_that_turn() {
    let mut g = setup(&[Card::GuardDog, Card::Militia]);
    hand(&mut g, P0, &[Card::Militia]);
    hand(
        &mut g,
        P1,
        &[Card::GuardDog, Card::Copper, Card::Copper, Card::Estate],
    );
    deck(&mut g, P1, &[Card::Silver; 8]);
    g.start();
    play(&mut g, Card::Militia);
    trigger(&mut g, 0);
    // 3 + 2 cards: 5, so +2 more = 7; then discard down to 3.
    let d = g.pending().unwrap();
    assert_eq!((d.player, d.purpose), (P1, Purpose::Discard));
    let hand_now = g.hand(P1).to_vec();
    assert_eq!(hand_now.len(), 7);
    choose(&mut g, &hand_now[..4]);
    let dog = g.in_play(P1)[0];
    end_phase(&mut g);
    assert_eq!(g.zone(dog), Zone::Discard(P1));
}

#[test]
fn haggler_gains_per_play_on_bought_cards_only() {
    let mut g = setup(&[Card::Haggler, Card::ThroneRoom]);
    hand(
        &mut g,
        P0,
        &[Card::ThroneRoom, Card::Haggler, Card::Gold, Card::Gold],
    );
    g.start();
    play(&mut g, Card::ThroneRoom);
    choose_kinds(&mut g, &[Card::Haggler]);
    play(&mut g, Card::Gold);
    play(&mut g, Card::Gold);
    buy(&mut g, Card::Province);
    // Two Haggler effects: each gains a cheaper non-Victory card.
    let offered = offered_piles(&g);
    assert!(offered.contains(&Card::Gold) && !offered.contains(&Card::Duchy));
    choose_pile(&mut g, Card::Gold);
    choose_pile(&mut g, Card::Silver);
    assert_eq!(g.count_owned(P0, Card::Gold), 3);
}

#[test]
fn highway_lasts_the_turn_and_throne_room_doubles_it() {
    let mut g = setup(&[Card::Highway, Card::ThroneRoom]);
    hand(&mut g, P0, &[Card::ThroneRoom, Card::Highway]);
    deck(&mut g, P0, &[Card::Copper; 5]);
    g.start();
    play(&mut g, Card::ThroneRoom);
    choose_kinds(&mut g, &[Card::Highway]);
    assert_eq!(g.cost(Card::Province).coins, 6);
    assert_eq!(g.cost(Card::Copper).coins, 0);
}

#[test]
fn inn_shuffles_actions_from_the_discard_pile_when_gained() {
    let mut g = setup(&[Card::Inn, Card::Village]);
    hand(&mut g, P0, &[Card::Gold, Card::Gold]);
    deck(&mut g, P0, &[Card::Copper]);
    discard(&mut g, P0, &[Card::Village, Card::Estate]);
    g.start();
    play(&mut g, Card::Gold);
    play(&mut g, Card::Gold);
    buy(&mut g, Card::Inn);
    choose_kinds(&mut g, &[Card::Village, Card::Inn]);
    // Clean-up drew the shuffled deck: Copper, Village, Inn, and two from the discard.
    let mut drawn = hand_kinds(&g, P0);
    drawn.sort();
    assert!(drawn.contains(&Card::Inn) && drawn.contains(&Card::Village));
}

#[test]
fn jack_of_all_trades_does_its_four_things() {
    let mut g = setup(&[Card::JackOfAllTrades]);
    hand(&mut g, P0, &[Card::JackOfAllTrades, Card::Estate]);
    deck(
        &mut g,
        P0,
        &[
            Card::Curse,
            Card::Copper,
            Card::Copper,
            Card::Copper,
            Card::Copper,
        ],
    );
    g.start();
    play(&mut g, Card::JackOfAllTrades);
    yes(&mut g);
    assert_eq!(g.hand(P0).len(), 5);
    choose_kinds(&mut g, &[Card::Estate]);
    assert_eq!(discard_kinds(&g, P0), [Card::Silver, Card::Curse]);
    assert_eq!(kinds(&g, g.trash()).last(), Some(&Card::Estate));
}

#[test]
fn margrave_makes_others_draw_then_discard() {
    let mut g = setup(&[Card::Margrave]);
    hand(&mut g, P0, &[Card::Margrave]);
    hand(&mut g, P1, &[Card::Copper; 4]);
    deck(&mut g, P1, &[Card::Estate]);
    g.start();
    play(&mut g, Card::Margrave);
    assert_eq!(g.turn().buys, 2);
    assert_eq!(g.hand(P1).len(), 5);
    choose_kinds(&mut g, &[Card::Estate, Card::Copper]);
    assert_eq!(g.hand(P1).len(), 3);
}

#[test]
fn nomads_gives_coins_when_gained_or_trashed() {
    let mut g = setup(&[Card::Nomads, Card::Chapel]);
    hand(&mut g, P0, &[Card::Chapel, Card::Nomads]);
    g.start();
    play(&mut g, Card::Chapel);
    choose_kinds(&mut g, &[Card::Nomads]);
    assert_eq!(g.turn().coins, 2);
    gain(&mut g, P0, Card::Nomads);
    assert_eq!(g.turn().coins, 4);
    // Gaining on another player's turn gives that turn nothing.
    gain(&mut g, P1, Card::Nomads);
    assert_eq!(g.turn().coins, 4);
}

#[test]
fn oasis_discards_a_card() {
    let mut g = setup(&[Card::Oasis]);
    hand(&mut g, P0, &[Card::Oasis, Card::Estate]);
    deck(&mut g, P0, &[Card::Copper]);
    g.start();
    play(&mut g, Card::Oasis);
    choose_kinds(&mut g, &[Card::Estate]);
    assert_eq!(discard_kinds(&g, P0), [Card::Estate]);
    assert_eq!(g.turn().coins, 1);
}

#[test]
fn scheme_topdecks_an_action_in_clean_up() {
    let mut g = setup(&[Card::Scheme, Card::Smithy]);
    hand(&mut g, P0, &[Card::Scheme, Card::Smithy]);
    deck(&mut g, P0, &[Card::Copper; 10]);
    g.start();
    play(&mut g, Card::Scheme);
    play(&mut g, Card::Smithy);
    end_phase(&mut g);
    // Clean-up: the optional Scheme trigger, then the card.
    trigger(&mut g, 0);
    choose_kinds(&mut g, &[Card::Smithy]);
    assert!(hand_kinds(&g, P0).contains(&Card::Smithy));
    assert!(discard_kinds(&g, P0).contains(&Card::Scheme));
}

#[test]
fn souk_loses_a_coin_per_card_in_hand_but_not_below_zero() {
    let mut g = setup(&[Card::Souk]);
    hand(
        &mut g,
        P0,
        &[Card::Souk, Card::Estate, Card::Estate, Card::Estate],
    );
    g.start();
    play(&mut g, Card::Souk);
    assert_eq!(g.turn().coins, 4);
    assert_eq!(g.turn().buys, 2);

    let mut g = setup(&[Card::Souk, Card::Village]);
    let mut cards = vec![Card::Village, Card::Souk];
    cards.extend([Card::Estate; 9]);
    hand(&mut g, P0, &cards);
    deck(&mut g, P0, &[Card::Estate]);
    g.start();
    play(&mut g, Card::Village);
    play(&mut g, Card::Souk);
    assert_eq!(g.turn().coins, 0);
}

#[test]
fn souk_trashes_up_to_two_when_gained() {
    let mut g = setup(&[Card::Souk]);
    hand(
        &mut g,
        P0,
        &[
            Card::Gold,
            Card::Gold,
            Card::Estate,
            Card::Curse,
            Card::Copper,
        ],
    );
    g.start();
    play(&mut g, Card::Gold);
    play(&mut g, Card::Gold);
    buy(&mut g, Card::Souk);
    choose_kinds(&mut g, &[Card::Estate, Card::Curse]);
    let t = g.trash().len();
    assert_eq!(kinds(&g, &g.trash()[t - 2..]), [Card::Estate, Card::Curse]);
}

#[test]
fn spice_merchant_trashes_a_treasure_for_a_choice() {
    let mut g = setup(&[Card::SpiceMerchant]);
    hand(&mut g, P0, &[Card::SpiceMerchant, Card::Copper]);
    g.start();
    play(&mut g, Card::SpiceMerchant);
    choose_kinds(&mut g, &[Card::Copper]);
    mode(&mut g, 1);
    assert_eq!((g.turn().coins, g.turn().buys), (2, 2));
}

#[test]
fn stables_discards_a_treasure_to_draw_three() {
    let mut g = setup(&[Card::Stables]);
    hand(&mut g, P0, &[Card::Stables, Card::Copper]);
    deck(&mut g, P0, &[Card::Estate; 3]);
    g.start();
    play(&mut g, Card::Stables);
    choose_kinds(&mut g, &[Card::Copper]);
    assert_eq!(hand_kinds(&g, P0), [Card::Estate; 3]);
    assert_eq!(g.turn().actions, 1);
}

#[test]
fn trader_trashes_for_silvers() {
    let mut g = setup(&[Card::Trader]);
    hand(&mut g, P0, &[Card::Trader, Card::Estate]);
    g.start();
    play(&mut g, Card::Trader);
    assert_eq!(discard_kinds(&g, P0), [Card::Silver, Card::Silver]);
}

/// Trader counts the trashed card's cost once, after the trash and what it set off: a
/// Market Square's Gold makes the Destrier cost $5, and the Silvers gained meanwhile don't
/// lower the count.
#[test]
fn trader_counts_the_trashed_cards_cost_after_trashing_it() {
    let mut g = setup(&[Card::Trader, Card::Destrier, Card::MarketSquare]);
    hand(
        &mut g,
        P0,
        &[Card::Trader, Card::Destrier, Card::MarketSquare],
    );
    g.start();
    play(&mut g, Card::Trader);
    choose_kinds(&mut g, &[Card::Destrier]);
    trigger(&mut g, 0);
    assert_eq!(g.count_owned(P0, Card::Silver), 5);
}

#[test]
fn trader_exchanges_a_gained_card_after_its_when_gain() {
    let mut g = setup(&[Card::Trader, Card::BorderVillage]);
    hand(&mut g, P0, &[Card::Trader, Card::Gold, Card::Gold]);
    g.start();
    end_phase(&mut g);
    play(&mut g, Card::Gold);
    play(&mut g, Card::Gold);
    buy(&mut g, Card::BorderVillage);
    // Border Village's own ability and Trader's exchange: exchange first.
    let d = g.pending().unwrap();
    assert_eq!(d.purpose, Purpose::Trigger);
    let options = match &d.kind {
        DecisionKind::Choose { labels, .. } => labels.clone(),
        k => panic!("{k:?}"),
    };
    let trader = options
        .iter()
        .position(|l| l.starts_with("Trader"))
        .unwrap();
    trigger(&mut g, trader as u16);
    // Border Village is back in its pile; its ability still gains a cheaper card.
    assert_eq!(g.count_owned(P0, Card::BorderVillage), 0);
    assert_eq!(
        g.pile(g.supply_pile(Card::BorderVillage).unwrap())
            .cards
            .len(),
        10
    );
    choose_pile(&mut g, Card::Duchy);
    // And that Duchy can be exchanged too.
    trigger(&mut g, 0);
    assert_eq!(g.count_owned(P0, Card::Duchy), 0);
    assert_eq!(g.count_owned(P0, Card::Silver), 2);
}

#[test]
fn trail_plays_when_discarded_trashed_or_gained() {
    let mut g = setup(&[Card::Trail, Card::Militia]);
    hand(&mut g, P0, &[Card::Militia]);
    hand(
        &mut g,
        P1,
        &[
            Card::Trail,
            Card::Copper,
            Card::Copper,
            Card::Copper,
            Card::Copper,
        ],
    );
    deck(&mut g, P1, &[Card::Silver; 3]);
    g.start();
    play(&mut g, Card::Militia);
    choose_kinds(&mut g, &[Card::Trail, Card::Copper]);
    trigger(&mut g, 0);
    let trail = g.in_play(P1)[0];
    assert_eq!(g.card(trail), Card::Trail);
    assert_eq!(g.hand(P1).len(), 4);
    end_phase(&mut g);
    assert_eq!(g.zone(trail), Zone::Discard(P1));

    let mut g = setup(&[Card::Trail, Card::Chapel]);
    hand(&mut g, P0, &[Card::Chapel, Card::Trail]);
    deck(&mut g, P0, &[Card::Copper; 3]);
    g.start();
    play(&mut g, Card::Chapel);
    choose_kinds(&mut g, &[Card::Trail]);
    trigger(&mut g, 0);
    assert_eq!(kinds(&g, g.in_play(P0)), [Card::Chapel, Card::Trail]);

    let mut g = setup(&[Card::Trail]);
    deck(&mut g, P0, &[Card::Copper; 3]);
    g.start();
    gain(&mut g, P0, Card::Trail);
    trigger(&mut g, 0);
    assert!(kinds(&g, g.in_play(P0)).contains(&Card::Trail));
}

#[test]
fn tunnel_gains_a_gold_when_discarded() {
    let mut g = setup(&[Card::Tunnel, Card::Militia]);
    hand(&mut g, P0, &[Card::Militia]);
    hand(
        &mut g,
        P1,
        &[
            Card::Tunnel,
            Card::Tunnel,
            Card::Copper,
            Card::Copper,
            Card::Copper,
        ],
    );
    g.start();
    play(&mut g, Card::Militia);
    choose_kinds(&mut g, &[Card::Tunnel, Card::Tunnel]);
    trigger(&mut g, 0);
    trigger(&mut g, 0);
    assert_eq!(g.count_owned(P1, Card::Gold), 2);
}

#[test]
fn tunnel_does_nothing_in_clean_up() {
    let mut g = setup(&[Card::Tunnel]);
    hand(&mut g, P0, &[Card::Tunnel]);
    g.start();
    end_phase(&mut g);
    assert_eq!(g.pending().unwrap().player, P1);
    assert_eq!(g.count_owned(P0, Card::Gold), 0);
}

#[test]
fn weavers_discarded_together_each_react() {
    let mut g = setup(&[Card::Weaver, Card::Militia]);
    hand(&mut g, P0, &[Card::Militia]);
    hand(
        &mut g,
        P1,
        &[
            Card::Weaver,
            Card::Weaver,
            Card::Copper,
            Card::Copper,
            Card::Copper,
        ],
    );
    g.start();
    play(&mut g, Card::Militia);
    choose_kinds(&mut g, &[Card::Weaver, Card::Weaver]);
    trigger(&mut g, 0);
    mode(&mut g, 0);
    let d = g.pending().unwrap();
    assert_eq!((d.player, d.purpose), (P1, Purpose::Trigger));
    trigger(&mut g, 0);
    mode(&mut g, 0);
    assert_eq!(g.count_owned(P1, Card::Silver), 4);
    assert_eq!(kinds(&g, g.in_play(P1)), [Card::Weaver, Card::Weaver]);
}

#[test]
fn trails_discarded_together_lose_track_after_a_reshuffle() {
    let mut g = setup(&[Card::Trail, Card::Militia]);
    hand(&mut g, P0, &[Card::Militia]);
    hand(
        &mut g,
        P1,
        &[
            Card::Trail,
            Card::Trail,
            Card::Copper,
            Card::Copper,
            Card::Copper,
        ],
    );
    deck(&mut g, P1, &[]);
    discard(&mut g, P1, &[]);
    g.start();
    play(&mut g, Card::Militia);
    choose_kinds(&mut g, &[Card::Trail, Card::Trail]);
    // Both Trails were discarded before either reacts.
    assert_eq!(discard_kinds(&g, P1), [Card::Trail, Card::Trail]);
    trigger(&mut g, 0);
    // Playing it drew from a reshuffled discard pile: the other Trail went with it.
    assert_eq!(g.in_play(P1).len(), 1);
    assert_eq!(g.pending().unwrap().player, P0);
}

#[test]
fn cauldron_curses_even_if_the_third_gain_gains_more_actions_first() {
    let mut g = setup(&[Card::Cauldron, Card::Village, Card::BorderVillage]);
    hand(&mut g, P0, &[Card::Cauldron, Card::Gold, Card::Gold]);
    g.start();
    gain(&mut g, P0, Card::Village);
    gain(&mut g, P0, Card::Village);
    play(&mut g, Card::Cauldron);
    play(&mut g, Card::Gold);
    play(&mut g, Card::Gold);
    buy(&mut g, Card::BorderVillage);
    // Resolve Border Village's gain first, gaining a fourth Action.
    let labels = match &g.pending().unwrap().kind {
        DecisionKind::Choose { labels, .. } => labels.clone(),
        k => panic!("{k:?}"),
    };
    let bv = labels.iter().position(|l| l == "Border Village").unwrap();
    trigger(&mut g, bv as u16);
    choose_pile(&mut g, Card::Village);
    assert_eq!(g.count_owned(P1, Card::Curse), 1);
}

#[test]
fn cauldron_played_after_the_third_action_gain_does_nothing() {
    let mut g = setup(&[Card::Cauldron, Card::Village]);
    hand(&mut g, P0, &[Card::Cauldron, Card::Gold]);
    g.start();
    for _ in 0..3 {
        gain(&mut g, P0, Card::Village);
    }
    play(&mut g, Card::Cauldron);
    play(&mut g, Card::Gold);
    buy(&mut g, Card::Village);
    assert_eq!(g.count_owned(P1, Card::Curse), 0);
}

#[test]
fn wheelwright_discards_to_gain_an_action_costing_as_much() {
    let mut g = setup(&[Card::Wheelwright, Card::Smithy, Card::Village]);
    hand(&mut g, P0, &[Card::Wheelwright, Card::Smithy]);
    deck(&mut g, P0, &[Card::Copper]);
    g.start();
    play(&mut g, Card::Wheelwright);
    choose_kinds(&mut g, &[Card::Smithy]);
    let offered = offered_piles(&g);
    assert!(offered.contains(&Card::Village) && !offered.contains(&Card::Silver));
    choose_pile(&mut g, Card::Smithy);
    assert_eq!(discard_kinds(&g, P0), [Card::Smithy, Card::Smithy]);
}

#[test]
fn witchs_hut_curses_when_both_discards_are_actions() {
    let mut g = setup(&[Card::WitchsHut, Card::Village]);
    hand(&mut g, P0, &[Card::WitchsHut, Card::Village, Card::Village]);
    deck(&mut g, P0, &[Card::Copper; 4]);
    g.start();
    play(&mut g, Card::WitchsHut);
    choose_kinds(&mut g, &[Card::Village, Card::Village]);
    assert_eq!(g.count_owned(P1, Card::Curse), 1);

    let mut g = setup(&[Card::WitchsHut, Card::Village]);
    hand(&mut g, P0, &[Card::WitchsHut, Card::Village, Card::Village]);
    deck(&mut g, P0, &[Card::Copper; 4]);
    g.start();
    play(&mut g, Card::WitchsHut);
    choose_kinds(&mut g, &[Card::Village, Card::Copper]);
    assert_eq!(g.count_owned(P1, Card::Curse), 0);
}

#[test]
fn cache_gains_two_coppers() {
    let mut g = setup(&[Card::Cache]);
    g.start();
    gain(&mut g, P0, Card::Cache);
    assert_eq!(
        discard_kinds(&g, P0),
        [Card::Cache, Card::Copper, Card::Copper]
    );
}

#[test]
fn duchess_lets_each_player_discard_their_top_card() {
    let mut g = setup(&[Card::Duchess]);
    hand(&mut g, P0, &[Card::Duchess]);
    deck(&mut g, P0, &[Card::Estate]);
    deck(&mut g, P1, &[Card::Gold]);
    g.start();
    play(&mut g, Card::Duchess);
    yes(&mut g);
    assert_eq!(g.pending().unwrap().player, P1);
    no(&mut g);
    assert_eq!(discard_kinds(&g, P0), [Card::Estate]);
    assert_eq!(deck_kinds(&g, P1), [Card::Gold]);
    assert_eq!(g.turn().coins, 2);
}

#[test]
fn duchess_may_be_gained_with_a_duchy() {
    let mut g = setup(&[Card::Duchess]);
    g.start();
    gain(&mut g, P0, Card::Duchy);
    trigger(&mut g, 0);
    assert_eq!(g.count_owned(P0, Card::Duchess), 1);
}

#[test]
fn embassy_and_ill_gotten_gains_give_others_cards_when_gained() {
    let mut g = setup(&[Card::Embassy, Card::IllGottenGains]);
    g.start();
    gain(&mut g, P0, Card::Embassy);
    assert_eq!(discard_kinds(&g, P1), [Card::Silver]);
    gain(&mut g, P0, Card::IllGottenGains);
    assert_eq!(discard_kinds(&g, P1), [Card::Silver, Card::Curse]);
}

#[test]
fn ill_gotten_gains_gains_a_copper_to_hand() {
    let mut g = setup(&[Card::IllGottenGains]);
    hand(&mut g, P0, &[Card::IllGottenGains]);
    g.start();
    play(&mut g, Card::IllGottenGains);
    yes(&mut g);
    assert_eq!(hand_kinds(&g, P0), [Card::Copper]);
    assert_eq!(g.turn().coins, 1);
}

#[test]
fn mandarin_topdecks_treasures_in_play_when_gained() {
    let mut g = setup(&[Card::Mandarin]);
    hand(&mut g, P0, &[Card::Gold, Card::Silver]);
    deck(&mut g, P0, &[]);
    g.start();
    play(&mut g, Card::Gold);
    play(&mut g, Card::Silver);
    buy(&mut g, Card::Mandarin);
    choose_kinds(&mut g, &[Card::Silver, Card::Gold]);
    // Clean-up drew them back.
    assert_eq!(hand_kinds(&g, P0)[..2], [Card::Silver, Card::Gold]);
}

#[test]
fn noble_brigand_steals_on_play_and_on_buy() {
    let mut g = setup(&[Card::NobleBrigand]);
    hand(&mut g, P0, &[Card::NobleBrigand]);
    deck(&mut g, P1, &[Card::Gold, Card::Silver]);
    g.start();
    play(&mut g, Card::NobleBrigand);
    choose_kinds(&mut g, &[Card::Gold]);
    assert_eq!(discard_kinds(&g, P0), [Card::Gold]);
    assert_eq!(discard_kinds(&g, P1), [Card::Silver]);

    let mut g = setup(&[Card::NobleBrigand]);
    hand(&mut g, P0, &[Card::Gold, Card::Gold]);
    deck(&mut g, P1, &[Card::Estate, Card::Estate]);
    g.start();
    play(&mut g, Card::Gold);
    play(&mut g, Card::Gold);
    buy(&mut g, Card::NobleBrigand);
    // No Treasure revealed: P1 gains a Copper.
    assert_eq!(
        discard_kinds(&g, P1),
        [Card::Estate, Card::Estate, Card::Copper]
    );
}

#[test]
fn nomad_camp_is_gained_onto_the_deck() {
    let mut g = setup(&[Card::NomadCamp]);
    g.start();
    gain(&mut g, P0, Card::NomadCamp);
    assert_eq!(deck_kinds(&g, P0)[0], Card::NomadCamp);
}

#[test]
fn nomad_camp_gained_to_hand_stays_in_hand() {
    let mut g = setup(&[Card::NomadCamp, Card::Artisan]);
    hand(&mut g, P0, &[Card::Artisan, Card::Estate]);
    g.start();
    play(&mut g, Card::Artisan);
    choose_pile(&mut g, Card::NomadCamp);
    // Artisan goes on to ask for a card from the hand to put onto the deck.
    assert_eq!(purpose(&g), Purpose::TopDeck);
    assert_eq!(hand_kinds(&g, P0), [Card::Estate, Card::NomadCamp]);
}

#[test]
fn oracle_reveals_for_everyone_then_draws() {
    let mut g = setup(&[Card::Oracle]);
    hand(&mut g, P0, &[Card::Oracle]);
    deck(
        &mut g,
        P0,
        &[Card::Gold, Card::Gold, Card::Copper, Card::Copper],
    );
    deck(&mut g, P1, &[Card::Gold, Card::Estate]);
    g.start();
    play(&mut g, Card::Oracle);
    // Keep my Golds (in the order chosen), discard theirs.
    no(&mut g);
    yes(&mut g);
    assert_eq!(hand_kinds(&g, P0), [Card::Gold, Card::Gold]);
    assert_eq!(discard_kinds(&g, P1), [Card::Gold, Card::Estate]);
}

#[test]
fn silk_road_counts_victory_cards() {
    let mut g = setup(&[Card::SilkRoad]);
    g.start();
    let base = g
        .player(P0)
        .owned()
        .filter(|&c| g.is(c, Types::VICTORY))
        .count();
    for _ in 0..(8 - base) {
        g.create(Card::SilkRoad, Zone::Discard(P0));
    }
    assert_eq!(crate::cards::vp(&g, P0, Card::SilkRoad), Some(2));
}

#[test]
fn scheme_with_throne_room_topdecks_two_actions() {
    let mut g = setup(&[Card::Scheme, Card::ThroneRoom, Card::Smithy]);
    hand(&mut g, P0, &[Card::ThroneRoom, Card::Scheme]);
    deck(
        &mut g,
        P0,
        &[
            Card::Smithy,
            Card::Copper,
            Card::Copper,
            Card::Copper,
            Card::Copper,
        ],
    );
    g.start();
    play(&mut g, Card::ThroneRoom);
    choose_kinds(&mut g, &[Card::Scheme]);
    play(&mut g, Card::Smithy);
    end_phase(&mut g);
    trigger(&mut g, 0);
    choose_kinds(&mut g, &[Card::Smithy]);
    trigger(&mut g, 0);
    choose_kinds(&mut g, &[Card::ThroneRoom]);
    let drawn = hand_kinds(&g, P0);
    assert!(drawn.contains(&Card::Smithy) && drawn.contains(&Card::ThroneRoom));
}

#[test]
fn guard_dog_can_draw_a_moat_in_time() {
    let mut g = setup(&[Card::GuardDog, Card::Militia, Card::Moat]);
    hand(&mut g, P0, &[Card::Militia]);
    hand(&mut g, P1, &[Card::GuardDog, Card::Copper]);
    deck(
        &mut g,
        P1,
        &[Card::Moat, Card::Copper, Card::Copper, Card::Copper],
    );
    g.start();
    play(&mut g, Card::Militia);
    trigger(&mut g, 0);
    // The Moat just drawn reacts to the same Attack.
    let d = g.pending().unwrap();
    assert_eq!((d.player, d.purpose), (P1, Purpose::Trigger));
    trigger(&mut g, 0);
    assert_eq!(g.hand(P1).len(), 5);
    assert_eq!(g.pending().unwrap().player, P0);
}

#[test]
fn mandarin_puts_a_card_from_hand_onto_the_deck() {
    let mut g = setup(&[Card::Mandarin]);
    hand(&mut g, P0, &[Card::Mandarin, Card::Estate, Card::Gold]);
    g.start();
    play(&mut g, Card::Mandarin);
    choose_kinds(&mut g, &[Card::Gold]);
    assert_eq!(deck_kinds(&g, P0)[0], Card::Gold);
    assert_eq!(g.turn().coins, 3);
}
