use super::*;
use crate::game::{Phase, Zone};
use rand::seq::SliceRandom;
use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;

/// Random games with kingdoms from `sets` (removed cards included), each player starting
/// with a copy of every Kingdom card and of `extra` so that expensive cards get played.
/// Returns how often each card was played.
pub(crate) fn fuzz_sets(sets: &[Set], extra: &[Card], games: u64) -> Vec<(Card, usize)> {
    let pool: Vec<Card> = Card::kingdom_cards()
        .filter(|c| sets.contains(&c.set()))
        .collect();
    let mut plays = std::collections::BTreeMap::new();
    for seed in 0..games {
        let mut rng = ChaCha8Rng::seed_from_u64(seed);
        let mut kingdom = pool.clone();
        kingdom.shuffle(&mut rng);
        kingdom.truncate(10);
        let players = rng.random_range(2..=4);
        let mut config = GameConfig::new(players, kingdom.clone(), seed);
        config.turn_limit = Some(40);
        let mut g = setup_with(config);
        for p in 0..players {
            for &c in kingdom.iter().chain(extra) {
                g.create(c, Zone::Discard(PlayerId(p)));
            }
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
        for e in g.log() {
            if let Event::Played { card, .. } = e {
                *plays.entry(*card).or_insert(0) += 1;
            }
        }
    }
    plays.into_iter().collect()
}

#[test]
fn random_games_with_every_card() {
    let mut extra = vec![Card::Province, Card::Gold];
    extra.extend(Card::in_set(Set::CornucopiaGuilds).filter(|c| !c.def().kingdom));
    let plays = fuzz_sets(&[Set::CornucopiaGuilds], &extra, 300);
    for card in Card::in_set(Set::CornucopiaGuilds) {
        if !card.is(Types::VICTORY) || card.is(Types::ACTION) {
            assert!(plays.iter().any(|&(c, _)| c == card), "{card} never played");
        }
    }
}

fn discard_kinds(g: &Game, p: PlayerId) -> Vec<Card> {
    kinds(g, &g.player(p).discard)
}

fn pick(g: &mut Game, opt: Opt) {
    respond(g, Response::option(opt));
}

fn mode(g: &mut Game, modes: &[u8]) {
    respond(
        g,
        Response::Options(modes.iter().map(|&m| Opt::Mode(m)).collect()),
    );
}

/// Plays every Treasure in hand, then ends the Action phase if needed.
fn play_treasures(g: &mut Game) {
    let p = g.turn().player;
    while let Some(&c) = g.hand(p).iter().find(|&&c| g.is(c, Types::TREASURE)) {
        respond(g, Response::Move(Move::Play(c)));
    }
}

#[test]
fn advisor_lets_the_left_player_pick_the_discard() {
    let mut g = setup(&[Card::Advisor]);
    hand(&mut g, P0, &[Card::Advisor]);
    deck(
        &mut g,
        P0,
        &[Card::Gold, Card::Silver, Card::Estate, Card::Copper],
    );
    g.start();
    play(&mut g, Card::Advisor);
    let d = g.pending().unwrap();
    assert_eq!((d.player, d.purpose), (P1, Purpose::Discard));
    choose_kinds(&mut g, &[Card::Gold]);
    assert_eq!(hand_kinds(&g, P0), [Card::Silver, Card::Estate]);
    assert_eq!(discard_kinds(&g, P0), [Card::Gold]);
    assert_eq!(g.turn().actions, 1);
}

#[test]
fn baker_gives_everyone_a_coffers_at_setup_and_one_when_played() {
    let mut g = setup(&[Card::Baker]);
    assert_eq!(g.player(P0).coffers, 1);
    assert_eq!(g.player(P1).coffers, 1);
    hand(&mut g, P0, &[Card::Baker]);
    g.start();
    play(&mut g, Card::Baker);
    assert_eq!(g.player(P0).coffers, 2);
}

#[test]
fn butcher_spends_coffers_to_raise_the_gain() {
    let mut g = setup(&[Card::Butcher, Card::Laboratory]);
    hand(&mut g, P0, &[Card::Butcher, Card::Estate]);
    g.players[0].coffers = 1;
    g.start();
    play(&mut g, Card::Butcher);
    assert_eq!(g.player(P0).coffers, 3);
    choose_kinds(&mut g, &[Card::Estate]);
    // Estate ($2) + 3 Coffers: up to $5.
    choose_pile(&mut g, Card::Laboratory);
    assert_eq!(g.player(P0).coffers, 0);
    assert_eq!(discard_kinds(&g, P0), [Card::Laboratory]);
    assert_eq!(g.turn().coins, 0);
}

#[test]
fn butcher_may_trash_nothing() {
    let mut g = setup(&[Card::Butcher]);
    hand(&mut g, P0, &[Card::Butcher, Card::Estate]);
    g.start();
    play(&mut g, Card::Butcher);
    choose(&mut g, &[]);
    assert_eq!(g.player(P0).coffers, 2);
    assert_eq!(hand_kinds(&g, P0), [Card::Estate]);
}

#[test]
fn carnival_keeps_one_of_each_name() {
    let mut g = setup(&[Card::Carnival]);
    hand(&mut g, P0, &[Card::Carnival]);
    deck(
        &mut g,
        P0,
        &[Card::Copper, Card::Copper, Card::Farmhands, Card::Copper],
    );
    g.start();
    play(&mut g, Card::Carnival);
    assert_eq!(hand_kinds(&g, P0), [Card::Copper, Card::Farmhands]);
    assert_eq!(discard_kinds(&g, P0), [Card::Copper, Card::Copper]);
}

#[test]
fn farmhands_sets_aside_a_card_to_play_next_turn() {
    let mut g = setup(&[Card::Farmhands, Card::Smithy]);
    hand(&mut g, P0, &[Card::Smithy, Card::Gold, Card::Copper]);
    g.start();
    end_phase(&mut g);
    play(&mut g, Card::Gold);
    play(&mut g, Card::Copper);
    buy(&mut g, Card::Farmhands);
    // Smithy is the only card to set aside once the trigger is accepted.
    respond(&mut g, Response::option(Opt::Trigger(0)));
    let smithy = g.player(P0).set_aside[0];
    assert_eq!(g.card(smithy), Card::Smithy);
    // P1's turn goes straight to its Buy phase: end it.
    end_phase(&mut g);
    // P0's next turn starts by playing the Smithy, using no Action.
    assert_eq!(g.zone(smithy), Zone::InPlay(P0));
    assert_eq!(g.turn().actions, 1);
    assert_eq!(g.hand(P0).len(), 5 + 3);
}

#[test]
fn farmhands_gained_at_the_start_of_a_turn_plays_the_card_next_turn() {
    let mut g = setup(&[Card::Farmhands, Card::Cobbler]);
    hand(&mut g, P0, &[Card::Cobbler]);
    deck(&mut g, P0, &[Card::Estate; 20]);
    g.start();
    while g.turn().phase != Phase::Night {
        end_phase(&mut g);
    }
    play(&mut g, Card::Cobbler);
    end_phase(&mut g);
    // At the start of P0's turn Cobbler gains Farmhands to hand, which sets itself aside.
    assert_eq!((g.turn().player, g.turn().phase), (P0, Phase::Start));
    choose_pile(&mut g, Card::Farmhands);
    let farmhands = find(&g, P0, Card::Farmhands);
    respond(&mut g, Response::option(Opt::Trigger(0)));
    assert_eq!(g.zone(farmhands), Zone::SetAside(P0));
    assert_eq!((g.turn().player, g.turn().phase), (P0, Phase::Buy));
    end_phase(&mut g);
    end_phase(&mut g);
    assert_eq!(g.turn().player, P0);
    assert_eq!(g.zone(farmhands), Zone::InPlay(P0));
}

#[test]
fn farrier_overpay_draws_extra_cards_at_end_of_turn() {
    let mut g = setup(&[Card::Farrier]);
    hand(&mut g, P0, &[Card::Gold, Card::Gold]);
    deck(&mut g, P0, &[Card::Copper; 10]);
    g.start();
    play_treasures(&mut g);
    buy(&mut g, Card::Farrier);
    assert_eq!(purpose(&g), Purpose::Overpay);
    respond(&mut g, Response::Number(2));
    assert_eq!(g.hand(P0).len(), 7);
}

/// Talisman gains a copy of the overpaid Herald, which Rush plays; it plays a Black Market,
/// which buys a card before the bought Herald is gained. The overpay is the bought one's.
#[test]
fn an_overpay_survives_a_buy_made_before_its_card_is_gained() {
    let mut g = setup_with(crate::GameConfig {
        colonies: Some(false),
        shelters: Some(false),
        landscapes: vec![Card::Rush],
        setup_choices: [(
            Card::BlackMarket,
            vec![Card::Moat, Card::Smithy, Card::Village],
        )]
        .into(),
        ..crate::GameConfig::new(2, vec![Card::Herald, Card::Talisman, Card::BlackMarket], 7)
    });
    hand(
        &mut g,
        P0,
        &[Card::Talisman, Card::Gold, Card::Gold, Card::Gold],
    );
    deck(
        &mut g,
        P0,
        &[Card::Estate, Card::BlackMarket, Card::Copper, Card::Copper],
    );
    g.start();
    play_treasures(&mut g);
    respond(&mut g, Response::Move(Move::BuyLandscape(Card::Rush)));
    buy(&mut g, Card::Herald);
    respond(&mut g, Response::Number(1));
    choose_kinds(&mut g, &[Card::Village]);
    choose_kinds(&mut g, &[Card::Moat, Card::Smithy]);
    assert_eq!(purpose(&g), Purpose::TopDeck);
    choose_kinds(&mut g, &[Card::Village]);
    // On top of the deck, so drawn in Clean-up.
    assert!(hand_kinds(&g, P0).contains(&Card::Village));
}

#[test]
fn farrier_gained_without_buying_draws_nothing_extra() {
    let mut g = setup(&[Card::Farrier, Card::Workshop]);
    hand(&mut g, P0, &[Card::Workshop]);
    deck(&mut g, P0, &[Card::Copper; 10]);
    g.start();
    play(&mut g, Card::Workshop);
    choose_pile(&mut g, Card::Farrier);
    end_phase(&mut g);
    assert_eq!(g.pending().unwrap().player, P1);
    assert_eq!(g.hand(P0).len(), 5);
}

#[test]
fn ferryman_discards_a_card() {
    let mut g = setup(&[Card::Ferryman]);
    hand(&mut g, P0, &[Card::Ferryman, Card::Estate, Card::Estate]);
    deck(&mut g, P0, &[Card::Copper; 5]);
    g.start();
    play(&mut g, Card::Ferryman);
    let d = g.pending().unwrap();
    assert_eq!((d.player, d.purpose), (P0, Purpose::Discard));
    choose_kinds(&mut g, &[Card::Estate]);
    assert_eq!(discard_kinds(&g, P0), [Card::Estate]);
    assert_eq!(g.hand(P0).len(), 3);
    assert_eq!(g.turn().actions, 1);
}

#[test]
fn ferryman_gains_a_card_from_its_pile() {
    let mut g = setup(&[Card::Ferryman, Card::Workshop]);
    let pile = g
        .ext
        .cornucopia_guilds
        .ferryman
        .expect("a pile was set aside");
    let card = g.pile(pile).card;
    assert!(!g.pile(pile).in_supply);
    assert!(matches!(g.cost(card).coins, 3 | 4));
    assert!(g.supply_pile(card).is_none());
    hand(&mut g, P0, &[Card::Workshop]);
    g.start();
    play(&mut g, Card::Workshop);
    let piles = match &g.pending().unwrap().kind {
        DecisionKind::Pile { piles, .. } => piles.clone(),
        k => panic!("{k:?}"),
    };
    assert!(
        !piles.contains(&pile),
        "Ferryman's pile is not in the Supply"
    );
    // Ferryman costs $5, so Workshop can't gain it: gain it directly instead.
    choose_pile(&mut g, Card::Silver);
    let ferryman = g.pile_top(g.supply_pile(Card::Ferryman).unwrap()).unwrap();
    g.stack
        .push(crate::frame::Frame::gain_card(P0, ferryman, Dest::Discard));
    g.pending = None;
    g.run(crate::frame::Input::Start);
    assert_eq!(g.count_owned(P0, Card::Ferryman), 1);
    assert_eq!(g.count_owned(P0, card), 1);
    assert_eq!(g.pile(pile).cards.len(), 9);
}

#[test]
fn footpad_draws_on_gains_in_action_phases_only() {
    let mut g = setup(&[Card::Footpad, Card::Workshop]);
    hand(&mut g, P0, &[Card::Workshop, Card::Silver]);
    deck(&mut g, P0, &[Card::Copper; 5]);
    g.start();
    play(&mut g, Card::Workshop);
    choose_pile(&mut g, Card::Silver);
    assert_eq!(g.hand(P0).len(), 2);
    play_treasures(&mut g);
    buy(&mut g, Card::Silver);
    let draws: Vec<usize> = g
        .log()
        .iter()
        .filter_map(|e| match e {
            Event::Drew { player: P0, count } => Some(*count),
            _ => None,
        })
        .collect();
    // Workshop's gain drew one; buying the Silver drew none; then Clean-up, short of
    // cards, shuffled and drew five.
    assert_eq!(draws, [1, 5]);
}

#[test]
fn footpad_does_not_draw_for_a_gain_that_returns_to_the_action_phase() {
    let mut g = setup(&[Card::Footpad, Card::Cavalry]);
    hand(&mut g, P0, &[Card::Silver, Card::Silver]);
    deck(&mut g, P0, &[Card::Copper; 5]);
    g.start();
    play_treasures(&mut g);
    buy(&mut g, Card::Cavalry);
    // Cavalry's +2 Cards only: it was gained in the Buy phase.
    assert_eq!(g.hand(P0).len(), 2);
}

#[test]
fn footpad_in_the_black_market_deck_draws_on_gains() {
    let mut g = setup_with(GameConfig {
        setup_choices: [(Card::BlackMarket, vec![Card::Footpad])].into(),
        ..GameConfig::new(2, vec![Card::BlackMarket, Card::Workshop], 7)
    });
    hand(&mut g, P0, &[Card::Workshop]);
    deck(&mut g, P0, &[Card::Copper; 5]);
    g.start();
    play(&mut g, Card::Workshop);
    choose_pile(&mut g, Card::Silver);
    assert_eq!(hand_kinds(&g, P0), [Card::Copper]);
}

#[test]
fn footpad_attacks_and_gives_coffers() {
    let mut g = setup(&[Card::Footpad]);
    hand(&mut g, P0, &[Card::Footpad]);
    g.start();
    play(&mut g, Card::Footpad);
    assert_eq!(g.player(P0).coffers, 2);
    let d = g.pending().unwrap();
    assert_eq!((d.player, d.purpose), (P1, Purpose::Discard));
}

#[test]
fn hamlet_discards_for_an_action_and_a_buy() {
    let mut g = setup(&[Card::Hamlet]);
    hand(&mut g, P0, &[Card::Hamlet, Card::Estate, Card::Curse]);
    deck(&mut g, P0, &[Card::Copper]);
    g.start();
    play(&mut g, Card::Hamlet);
    choose_kinds(&mut g, &[Card::Estate]);
    choose_kinds(&mut g, &[Card::Curse]);
    assert_eq!(g.turn().actions, 2);
    assert_eq!(g.turn().buys, 2);
}

#[test]
fn hamlet_bonuses_need_a_discard() {
    let mut g = setup(&[Card::Hamlet]);
    hand(&mut g, P0, &[Card::Hamlet, Card::Estate, Card::Curse]);
    deck(&mut g, P0, &[Card::Copper]);
    g.start();
    play(&mut g, Card::Hamlet);
    choose(&mut g, &[]);
    choose_kinds(&mut g, &[Card::Curse]);
    assert_eq!(g.turn().actions, 1);
    assert_eq!(g.turn().buys, 2);
}

#[test]
fn herald_plays_a_revealed_action() {
    let mut g = setup(&[Card::Herald, Card::Smithy]);
    hand(&mut g, P0, &[Card::Herald]);
    deck(
        &mut g,
        P0,
        &[
            Card::Copper,
            Card::Smithy,
            Card::Copper,
            Card::Copper,
            Card::Copper,
        ],
    );
    g.start();
    play(&mut g, Card::Herald);
    assert_eq!(kinds(&g, g.in_play(P0)), [Card::Herald, Card::Smithy]);
    assert_eq!(g.hand(P0).len(), 4);
    assert_eq!(g.turn().actions, 1);
}

#[test]
fn herald_leaves_a_non_action_on_top() {
    let mut g = setup(&[Card::Herald]);
    hand(&mut g, P0, &[Card::Herald]);
    deck(&mut g, P0, &[Card::Copper, Card::Gold]);
    g.start();
    play(&mut g, Card::Herald);
    assert_eq!(deck_kinds(&g, P0), [Card::Gold]);
}

#[test]
fn herald_overpay_topdecks_from_the_discard_pile_including_itself() {
    let mut g = setup(&[Card::Herald]);
    hand(&mut g, P0, &[Card::Gold, Card::Gold]);
    deck(&mut g, P0, &[]);
    discard(&mut g, P0, &[Card::Province, Card::Gold, Card::Copper]);
    g.start();
    play_treasures(&mut g);
    buy(&mut g, Card::Herald);
    respond(&mut g, Response::Number(2));
    assert_eq!(purpose(&g), Purpose::TopDeck);
    choose_kinds(&mut g, &[Card::Herald, Card::Gold]);
    let order: Vec<CardId> = candidates(&g);
    let herald = *order.iter().find(|&&c| g.card(c) == Card::Herald).unwrap();
    let gold = *order.iter().find(|&&c| g.card(c) == Card::Gold).unwrap();
    choose(&mut g, &[herald, gold]);
    // Clean-up drew them (and the two Golds played and discarded after).
    let drawn = hand_kinds(&g, P0);
    assert!(drawn.contains(&Card::Herald) && drawn.contains(&Card::Gold));
}

#[test]
fn horn_of_plenty_counts_names_in_play_and_trashes_itself_for_victory() {
    let mut g = setup(&[Card::HornOfPlenty, Card::Village]);
    hand(
        &mut g,
        P0,
        &[
            Card::Village,
            Card::HornOfPlenty,
            Card::Copper,
            Card::Silver,
        ],
    );
    deck(&mut g, P0, &[Card::Copper, Card::Copper]);
    g.start();
    play(&mut g, Card::Village);
    play(&mut g, Card::Copper);
    play(&mut g, Card::Silver);
    play(&mut g, Card::HornOfPlenty);
    // Village, Copper, Silver, Horn: up to $4. Nothing costing $5 is offered.
    let piles = match &g.pending().unwrap().kind {
        DecisionKind::Pile { piles, .. } => piles.clone(),
        k => panic!("{k:?}"),
    };
    assert!(!piles.contains(&g.supply_pile(Card::Duchy).unwrap()));
    choose_pile(&mut g, Card::Estate);
    assert_eq!(kinds(&g, g.trash()).last(), Some(&Card::HornOfPlenty));
    assert_eq!(g.turn().coins, 3);
}

#[test]
fn hunting_party_finds_a_card_not_in_hand() {
    let mut g = setup(&[Card::HuntingParty]);
    hand(
        &mut g,
        P0,
        &[Card::HuntingParty, Card::Copper, Card::Estate],
    );
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
    play(&mut g, Card::HuntingParty);
    // Drew the Copper; then Estate and Copper are copies, Silver is not.
    assert_eq!(
        hand_kinds(&g, P0),
        [Card::Copper, Card::Estate, Card::Copper, Card::Silver]
    );
    assert_eq!(discard_kinds(&g, P0), [Card::Estate, Card::Copper]);
    assert_eq!(deck_kinds(&g, P0), [Card::Gold]);
}

#[test]
fn infirmary_overpay_plays_it_once_per_coin() {
    let mut g = setup(&[Card::Infirmary]);
    hand(
        &mut g,
        P0,
        &[Card::Gold, Card::Gold, Card::Curse, Card::Estate],
    );
    deck(&mut g, P0, &[Card::Copper; 10]);
    g.start();
    play_treasures(&mut g);
    buy(&mut g, Card::Infirmary);
    respond(&mut g, Response::Number(2));
    // Playing it puts it into play.
    assert_eq!(
        kinds(&g, g.in_play(P0)),
        [Card::Gold, Card::Gold, Card::Infirmary]
    );
    choose_kinds(&mut g, &[Card::Curse]);
    choose_kinds(&mut g, &[Card::Estate]);
    assert_eq!(
        kinds(&g, &g.trash()[g.trash().len() - 2..]),
        [Card::Curse, Card::Estate]
    );
    assert_eq!(infirmary_plays(&g), 2);
}

fn infirmary_plays(g: &Game) -> usize {
    let played = |e: &&Event| {
        matches!(
            e,
            Event::Played {
                card: Card::Infirmary,
                ..
            }
        )
    };
    g.log().iter().filter(played).count()
}

#[test]
fn infirmary_exchanged_before_its_overpay_is_not_played() {
    let mut g = setup(&[Card::Infirmary, Card::Trader]);
    hand(&mut g, P0, &[Card::Gold, Card::Copper, Card::Trader]);
    deck(&mut g, P0, &[Card::Estate; 10]);
    g.start();
    end_phase(&mut g);
    play_treasures(&mut g);
    buy(&mut g, Card::Infirmary);
    respond(&mut g, Response::Number(1));
    let labels = match &g.pending().unwrap().kind {
        DecisionKind::Choose { labels, .. } => labels.clone(),
        k => panic!("{k:?}"),
    };
    let trader = labels.iter().position(|l| l.starts_with("Trader")).unwrap();
    respond(&mut g, Response::option(Opt::Trigger(trader as u16)));
    // Back on its pile, it can't be put into play.
    let pile = g.supply_pile(Card::Infirmary).unwrap();
    assert_eq!(g.pile(pile).cards.len(), 10);
    assert_eq!(infirmary_plays(&g), 0);
    assert_eq!(g.hand(P0).len(), 5);
}

#[test]
fn jester_curses_on_victory_and_lets_the_attacker_choose_otherwise() {
    let mut g = setup(&[Card::Jester]);
    hand(&mut g, P0, &[Card::Jester]);
    deck(&mut g, P1, &[Card::Estate]);
    g.start();
    play(&mut g, Card::Jester);
    assert_eq!(g.count_owned(P1, Card::Curse), 1);
    assert_eq!(g.turn().coins, 2);

    let mut g = setup(&[Card::Jester]);
    hand(&mut g, P0, &[Card::Jester]);
    deck(&mut g, P1, &[Card::Silver]);
    g.start();
    play(&mut g, Card::Jester);
    assert_eq!(purpose(&g), Purpose::Gain);
    pick(&mut g, Opt::Player(P0));
    assert_eq!(discard_kinds(&g, P0), [Card::Silver]);
    assert_eq!(discard_kinds(&g, P1), [Card::Silver]);
}

/// Jester has its victims discard, not reveal: a Patron discarded that way gives nothing.
#[test]
fn jester_does_not_reveal_the_discarded_card() {
    let mut g = setup(&[Card::Jester, Card::Patron]);
    hand(&mut g, P0, &[Card::Jester]);
    deck(&mut g, P1, &[Card::Patron]);
    g.start();
    play(&mut g, Card::Jester);
    pick(&mut g, Opt::Player(P0));
    assert_eq!(discard_kinds(&g, P1), [Card::Patron]);
    assert_eq!(g.player(P1).coffers, 0);
}

#[test]
fn moat_blocks_jester() {
    let mut g = setup(&[Card::Jester, Card::Moat]);
    hand(&mut g, P0, &[Card::Jester]);
    hand(&mut g, P1, &[Card::Moat]);
    deck(&mut g, P1, &[Card::Estate]);
    g.start();
    play(&mut g, Card::Jester);
    respond(&mut g, Response::option(Opt::Trigger(0)));
    assert_eq!(g.count_owned(P1, Card::Curse), 0);
    assert_eq!(deck_kinds(&g, P1), [Card::Estate]);
}

#[test]
fn journeyman_skips_the_named_card() {
    let mut g = setup(&[Card::Journeyman]);
    hand(&mut g, P0, &[Card::Journeyman]);
    deck(
        &mut g,
        P0,
        &[
            Card::Copper,
            Card::Silver,
            Card::Copper,
            Card::Estate,
            Card::Gold,
            Card::Duchy,
        ],
    );
    g.start();
    play(&mut g, Card::Journeyman);
    respond(&mut g, Response::Name(Card::Copper));
    assert_eq!(hand_kinds(&g, P0), [Card::Silver, Card::Estate, Card::Gold]);
    assert_eq!(discard_kinds(&g, P0), [Card::Copper, Card::Copper]);
    assert_eq!(deck_kinds(&g, P0), [Card::Duchy]);
}

#[test]
fn joust_trades_a_province_for_a_reward_until_clean_up() {
    let mut g = setup(&[Card::Joust]);
    assert_eq!(g.pile(g.pile_of(Card::Courser).unwrap()).cards.len(), 1);
    assert!(!g.pile(g.pile_of(Card::Courser).unwrap()).in_supply);
    hand(&mut g, P0, &[Card::Joust, Card::Province]);
    deck(&mut g, P0, &[Card::Copper; 8]);
    g.start();
    play(&mut g, Card::Joust);
    choose_kinds(&mut g, &[Card::Province]);
    choose_pile(&mut g, Card::Courser);
    assert!(hand_kinds(&g, P0).contains(&Card::Courser));
    let province = g.player(P0).set_aside[0];
    assert_eq!(g.turn().coins, 1);
    end_phase(&mut g);
    end_phase(&mut g);
    assert_eq!(g.zone(province), Zone::Discard(P0));
}

#[test]
fn rewards_come_two_of_each_with_three_players() {
    let g = setup_with(GameConfig::new(3, vec![Card::Joust], 1));
    for r in [Card::Coronet, Card::Renown, Card::HugeTurnip] {
        assert_eq!(g.pile(g.pile_of(r).unwrap()).cards.len(), 2);
    }
}

#[test]
fn menagerie_draws_three_with_no_duplicates() {
    let mut g = setup(&[Card::Menagerie]);
    hand(&mut g, P0, &[Card::Menagerie, Card::Copper, Card::Silver]);
    deck(&mut g, P0, &[Card::Estate; 5]);
    g.start();
    play(&mut g, Card::Menagerie);
    assert_eq!(g.hand(P0).len(), 5);

    let mut g = setup(&[Card::Menagerie]);
    hand(&mut g, P0, &[Card::Menagerie, Card::Copper, Card::Copper]);
    deck(&mut g, P0, &[Card::Estate; 5]);
    g.start();
    play(&mut g, Card::Menagerie);
    assert_eq!(g.hand(P0).len(), 3);
}

#[test]
fn merchant_guild_gives_coffers_per_card_gained_in_the_buy_phase() {
    let mut g = setup(&[Card::MerchantGuild, Card::ThroneRoom]);
    hand(
        &mut g,
        P0,
        &[Card::ThroneRoom, Card::MerchantGuild, Card::Silver],
    );
    g.start();
    play(&mut g, Card::ThroneRoom);
    choose_kinds(&mut g, &[Card::MerchantGuild]);
    play_treasures(&mut g);
    // +3 Buys, $4.
    buy(&mut g, Card::Silver);
    buy(&mut g, Card::Copper);
    end_phase(&mut g);
    // Two plays, two cards gained: 2 x 2 Coffers.
    assert_eq!(g.player(P0).coffers, 4);
}

#[test]
fn plaza_discards_a_treasure_for_coffers() {
    let mut g = setup(&[Card::Plaza]);
    hand(&mut g, P0, &[Card::Plaza]);
    deck(&mut g, P0, &[Card::Copper]);
    g.start();
    play(&mut g, Card::Plaza);
    choose_kinds(&mut g, &[Card::Copper]);
    assert_eq!(g.player(P0).coffers, 1);
    assert_eq!(g.turn().actions, 2);
}

#[test]
fn remake_twice_gains_exactly_one_more() {
    let mut g = setup(&[Card::Remake]);
    hand(&mut g, P0, &[Card::Remake, Card::Estate, Card::Copper]);
    g.start();
    play(&mut g, Card::Remake);
    choose_kinds(&mut g, &[Card::Estate]);
    // Silver is the only card costing $3, so it is gained; nothing costs exactly $1.
    assert_eq!(hand_kinds(&g, P0), []);
    assert_eq!(discard_kinds(&g, P0), [Card::Silver]);
    assert_eq!(
        kinds(&g, &g.trash()[g.trash().len() - 2..]),
        [Card::Estate, Card::Copper]
    );
}

#[test]
fn shop_plays_an_action_without_a_copy_in_play() {
    let mut g = setup(&[Card::Shop, Card::Smithy]);
    hand(&mut g, P0, &[Card::Shop, Card::Shop, Card::Smithy]);
    deck(&mut g, P0, &[Card::Copper; 5]);
    g.start();
    play(&mut g, Card::Shop);
    assert_eq!(kinds(&g, &candidates(&g)), [Card::Smithy]);
    choose_kinds(&mut g, &[Card::Smithy]);
    assert_eq!(kinds(&g, g.in_play(P0)), [Card::Shop, Card::Smithy]);
    assert_eq!(g.turn().actions, 0);
    assert_eq!(g.turn().coins, 1);
}

#[test]
fn soothsayer_curses_and_victims_draw() {
    let mut g = setup(&[Card::Soothsayer]);
    hand(&mut g, P0, &[Card::Soothsayer]);
    let before = g.hand(P1).len();
    g.start();
    play(&mut g, Card::Soothsayer);
    assert_eq!(discard_kinds(&g, P0), [Card::Gold]);
    assert_eq!(g.count_owned(P1, Card::Curse), 1);
    assert_eq!(g.hand(P1).len(), before + 1);
}

#[test]
fn soothsayer_victims_without_a_curse_do_not_draw() {
    let mut g = setup(&[Card::Soothsayer]);
    let curses = g.supply_pile(Card::Curse).unwrap();
    for c in g.pile(curses).cards.clone() {
        g.move_card(c, Zone::Trash, crate::game::Pos::Top);
    }
    hand(&mut g, P0, &[Card::Soothsayer]);
    let before = g.hand(P1).len();
    g.start();
    play(&mut g, Card::Soothsayer);
    assert_eq!(g.hand(P1).len(), before);
}

#[test]
fn stonemason_gains_two_cheaper_cards() {
    let mut g = setup(&[Card::Stonemason]);
    hand(&mut g, P0, &[Card::Stonemason, Card::Gold]);
    g.start();
    play(&mut g, Card::Stonemason);
    choose_pile(&mut g, Card::Duchy);
    choose_pile(&mut g, Card::Silver);
    assert_eq!(discard_kinds(&g, P0), [Card::Duchy, Card::Silver]);
}

#[test]
fn stonemason_overpay_gains_two_actions_costing_the_overpay() {
    let mut g = setup(&[Card::Stonemason, Card::Village, Card::Smithy]);
    hand(&mut g, P0, &[Card::Gold, Card::Gold]);
    g.start();
    play_treasures(&mut g);
    buy(&mut g, Card::Stonemason);
    respond(&mut g, Response::Number(3));
    // Village is the only Action costing exactly $3: two of them, no choice needed.
    assert_eq!(g.count_owned(P0, Card::Village), 2);
    assert_eq!(g.count_owned(P0, Card::Stonemason), 1);
}

#[test]
fn stonemason_overpay_with_a_potion_gains_potion_cost_actions() {
    let mut g = setup(&[Card::Stonemason, Card::Golem]);
    hand(&mut g, P0, &[Card::Gold, Card::Gold, Card::Potion]);
    g.start();
    play_treasures(&mut g);
    buy(&mut g, Card::Stonemason);
    respond(&mut g, Response::Number(4));
    assert_eq!(purpose(&g), Purpose::Overpay);
    respond(&mut g, Response::Number(1));
    // Golem is the only Action costing exactly $4 and a Potion.
    assert_eq!(g.count_owned(P0, Card::Golem), 2);
    assert!(g.log().contains(&Event::Overpaid {
        player: P0,
        amount: 4,
        potions: 1,
    }));
}

#[test]
fn overpay_abilities_happen_on_gain_even_if_exchanged() {
    let mut g = setup(&[Card::Masterpiece, Card::Trader]);
    hand(&mut g, P0, &[Card::Gold, Card::Gold, Card::Trader]);
    g.start();
    end_phase(&mut g);
    play_treasures(&mut g);
    buy(&mut g, Card::Masterpiece);
    respond(&mut g, Response::Number(2));
    // The overpay and Trader's exchange both trigger on the gain: exchange first.
    let labels = match &g.pending().unwrap().kind {
        DecisionKind::Choose { labels, .. } => labels.clone(),
        k => panic!("{k:?}"),
    };
    let trader = labels.iter().position(|l| l.starts_with("Trader")).unwrap();
    respond(&mut g, Response::option(Opt::Trigger(trader as u16)));
    // Each Silver the overpay gains could be exchanged too: decline.
    while g
        .pending()
        .is_some_and(|d| d.purpose == Purpose::Trigger && d.player == P0)
    {
        no(&mut g);
    }
    assert_eq!(g.count_owned(P0, Card::Masterpiece), 0);
    assert_eq!(g.count_owned(P0, Card::Silver), 1 + 2);
}

#[test]
fn footpad_draws_for_any_player_gaining_in_an_action_phase() {
    let mut g = setup(&[Card::Footpad, Card::Witch]);
    hand(&mut g, P0, &[Card::Witch]);
    deck(&mut g, P0, &[Card::Copper; 5]);
    let before = g.hand(P1).len();
    g.start();
    play(&mut g, Card::Witch);
    assert_eq!(g.count_owned(P1, Card::Curse), 1);
    assert_eq!(g.hand(P1).len(), before + 1);
}

#[test]
fn footpad_draws_for_a_gain_at_the_start_of_a_turn() {
    let mut g = setup(&[Card::Footpad, Card::Shaman]);
    g.create(Card::Gold, Zone::Trash);
    g.start();
    // Shaman's gain: the start of a turn is part of its Action phase.
    assert_eq!(g.count_owned(P0, Card::Gold), 1);
    assert_eq!(g.hand(P0).len(), 6);
}

#[test]
fn moat_comes_before_revealing_a_bane() {
    let mut g = setup(&[Card::YoungWitch, Card::Moat]);
    hand(&mut g, P0, &[Card::YoungWitch, Card::Estate, Card::Estate]);
    hand(&mut g, P1, &[Card::Moat]);
    deck(&mut g, P0, &[Card::Copper, Card::Copper]);
    g.start();
    play(&mut g, Card::YoungWitch);
    let d = g.pending().unwrap();
    assert_eq!((d.player, d.purpose), (P1, Purpose::Trigger));
    respond(&mut g, Response::option(Opt::Trigger(0)));
    choose_kinds(&mut g, &[Card::Estate, Card::Estate]);
    assert_eq!(g.count_owned(P1, Card::Curse), 0);
}

#[test]
fn merchant_guild_ignores_action_phase_gains() {
    let mut g = setup(&[Card::MerchantGuild, Card::Workshop, Card::Village]);
    hand(
        &mut g,
        P0,
        &[Card::Village, Card::MerchantGuild, Card::Workshop],
    );
    deck(&mut g, P0, &[Card::Estate]);
    g.start();
    play(&mut g, Card::Village);
    play(&mut g, Card::MerchantGuild);
    play(&mut g, Card::Workshop);
    choose_pile(&mut g, Card::Silver);
    buy(&mut g, Card::Copper);
    end_phase(&mut g);
    assert_eq!(g.player(P0).coffers, 1);
}

/// The Buy phase ends each time the turn leaves it (Villa), and Merchant Guild counts the
/// cards gained in that Buy phase.
#[test]
fn merchant_guild_counts_each_buy_phase_separately() {
    let mut g = setup(&[Card::MerchantGuild, Card::Villa]);
    hand(&mut g, P0, &[Card::MerchantGuild, Card::Gold]);
    deck(&mut g, P0, &[Card::Estate; 10]);
    g.start();
    play(&mut g, Card::MerchantGuild);
    play(&mut g, Card::Gold);
    buy(&mut g, Card::Villa);
    assert_eq!(g.turn().phase, Phase::Action);
    assert_eq!(g.player(P0).coffers, 1);
    play(&mut g, Card::Villa);
    buy(&mut g, Card::Copper);
    while g.turn().player == P0 {
        end_phase(&mut g);
    }
    assert_eq!(g.player(P0).coffers, 2);
}

#[test]
fn young_witch_bane_in_hand_blocks_the_curse() {
    let mut g = setup(&[Card::YoungWitch]);
    let bane_pile = g.ext.cornucopia_guilds.bane.expect("a Bane pile");
    let bane = g.pile(bane_pile).card;
    assert!(g.pile(bane_pile).in_supply);
    assert!(matches!(g.cost(bane).coins, 2 | 3));
    assert!(
        g.view(P1)
            .notes()
            .iter()
            .any(|n| n.source == Card::YoungWitch && n.pile == Some(bane_pile))
    );
    hand(&mut g, P0, &[Card::YoungWitch, Card::Estate, Card::Estate]);
    hand(&mut g, P1, &[bane, Card::Copper]);
    deck(&mut g, P0, &[Card::Copper, Card::Copper]);
    g.start();
    play(&mut g, Card::YoungWitch);
    choose_kinds(&mut g, &[Card::Estate, Card::Estate]);
    let d = g.pending().unwrap();
    assert_eq!((d.player, d.purpose), (P1, Purpose::Reveal));
    yes(&mut g);
    assert_eq!(g.count_owned(P1, Card::Curse), 0);
}

#[test]
fn young_witch_curses_without_a_bane() {
    let mut g = setup(&[Card::YoungWitch]);
    hand(&mut g, P0, &[Card::YoungWitch, Card::Estate, Card::Estate]);
    hand(&mut g, P1, &[Card::Copper]);
    deck(&mut g, P0, &[Card::Copper, Card::Copper]);
    g.start();
    play(&mut g, Card::YoungWitch);
    choose_kinds(&mut g, &[Card::Estate, Card::Estate]);
    assert_eq!(g.count_owned(P1, Card::Curse), 1);
}

#[test]
fn fairgrounds_counts_differently_named_cards() {
    let mut g = setup(&[Card::Fairgrounds]);
    g.start();
    let vp = |g: &Game| crate::cards::vp(g, P0, Card::Fairgrounds);
    // Copper, Estate, Silver, Gold: 4 names.
    for c in [Card::Silver, Card::Gold] {
        g.create(c, Zone::Discard(P0));
    }
    assert_eq!(vp(&g), Some(0));
    g.create(Card::Fairgrounds, Zone::Discard(P0));
    assert_eq!(vp(&g), Some(2));
}

#[test]
fn coronet_plays_an_action_and_a_treasure_twice() {
    let mut g = setup(&[Card::Joust, Card::Village]);
    hand(
        &mut g,
        P0,
        &[Card::Coronet, Card::Village, Card::Silver, Card::Renown],
    );
    deck(&mut g, P0, &[Card::Copper; 5]);
    g.start();
    play(&mut g, Card::Coronet);
    assert_eq!(kinds(&g, &candidates(&g)), [Card::Village]);
    choose_kinds(&mut g, &[Card::Village]);
    choose_kinds(&mut g, &[Card::Silver]);
    assert_eq!(g.turn().actions, 4);
    assert_eq!(g.turn().coins, 4);
}

#[test]
fn courser_does_two_different_options_in_order() {
    let mut g = setup(&[Card::Joust]);
    hand(&mut g, P0, &[Card::Courser]);
    deck(&mut g, P0, &[Card::Copper; 5]);
    g.start();
    play(&mut g, Card::Courser);
    mode(&mut g, &[3, 0]);
    assert_eq!(g.hand(P0).len(), 2);
    assert_eq!(discard_kinds(&g, P0), [Card::Silver; 4]);
}

#[test]
fn demesne_gains_a_gold_and_scores_per_gold() {
    let mut g = setup(&[Card::Joust]);
    hand(&mut g, P0, &[Card::Demesne]);
    g.start();
    play(&mut g, Card::Demesne);
    assert_eq!((g.turn().actions, g.turn().buys), (2, 3));
    assert_eq!(discard_kinds(&g, P0), [Card::Gold]);
    assert_eq!(crate::cards::vp(&g, P0, Card::Demesne), Some(1));
}

#[test]
fn housecarl_draws_per_differently_named_action_in_play() {
    let mut g = setup(&[Card::Joust, Card::Village]);
    hand(&mut g, P0, &[Card::Village, Card::Village, Card::Housecarl]);
    deck(&mut g, P0, &[Card::Copper; 10]);
    g.start();
    play(&mut g, Card::Village);
    play(&mut g, Card::Village);
    play(&mut g, Card::Housecarl);
    assert_eq!(g.hand(P0).len(), 2 + 2);
}

#[test]
fn huge_turnip_counts_the_new_coffers() {
    let mut g = setup(&[Card::Joust]);
    hand(&mut g, P0, &[Card::HugeTurnip]);
    g.players[0].coffers = 1;
    g.start();
    play(&mut g, Card::HugeTurnip);
    assert_eq!(g.player(P0).coffers, 3);
    assert_eq!(g.turn().coins, 3);
}

#[test]
fn renown_twice_with_throne_room_makes_cards_four_cheaper() {
    let mut g = setup(&[Card::Joust, Card::ThroneRoom]);
    hand(&mut g, P0, &[Card::ThroneRoom, Card::Renown]);
    g.start();
    play(&mut g, Card::ThroneRoom);
    choose_kinds(&mut g, &[Card::Renown]);
    assert_eq!(g.cost(Card::Gold).coins, 2);
    assert_eq!(g.cost(Card::Copper).coins, 0);
    assert_eq!(g.turn().buys, 3);
}

#[test]
fn tournament_gives_a_prize_or_a_bonus() {
    let mut g = setup(&[Card::Tournament]);
    assert!(g.pile_of(Card::Princess).is_some());
    hand(&mut g, P0, &[Card::Tournament, Card::Province]);
    hand(&mut g, P1, &[Card::Copper]);
    deck(&mut g, P0, &[Card::Copper, Card::Copper]);
    g.start();
    play(&mut g, Card::Tournament);
    yes(&mut g);
    choose_pile(&mut g, Card::BagOfGold);
    // Bag of Gold went on the deck, then +1 Card drew it.
    assert!(hand_kinds(&g, P0).contains(&Card::BagOfGold));
    assert_eq!(discard_kinds(&g, P0), [Card::Province]);
    assert_eq!(g.turn().coins, 1);
}

#[test]
fn tournament_without_bonus_when_another_player_reveals() {
    let mut g = setup(&[Card::Tournament]);
    hand(&mut g, P0, &[Card::Tournament]);
    hand(&mut g, P1, &[Card::Province]);
    g.start();
    play(&mut g, Card::Tournament);
    assert_eq!(g.pending().unwrap().player, P1);
    yes(&mut g);
    assert_eq!(g.turn().coins, 0);
    assert_eq!(g.hand(P0).len(), 0);
    assert_eq!(hand_kinds(&g, P1), [Card::Province]);
}

#[test]
fn prizes() {
    let mut g = setup(&[Card::Tournament]);
    hand(
        &mut g,
        P0,
        &[
            Card::BagOfGold,
            Card::Princess,
            Card::Diadem,
            Card::TrustySteed,
        ],
    );
    deck(&mut g, P0, &[Card::Copper; 3]);
    g.start();
    play(&mut g, Card::BagOfGold);
    assert_eq!(deck_kinds(&g, P0)[0], Card::Gold);
    play(&mut g, Card::TrustySteed);
    mode(&mut g, &[1, 3]);
    assert_eq!(g.player(P0).deck.len(), 0);
    assert_eq!(g.count_owned(P0, Card::Silver), 4);
    assert_eq!(g.turn().actions, 2);
    play(&mut g, Card::Princess);
    assert_eq!(g.cost(Card::Gold).coins, 4);
    assert_eq!(g.turn().buys, 2);
    // No Actions left in hand: on to the Buy phase.
    play(&mut g, Card::Diadem);
    assert_eq!(g.turn().coins, 2 + 1);
}

#[test]
fn followers_gains_an_estate_and_attacks_each_player_in_turn() {
    let mut g = setup_with(GameConfig::new(3, vec![Card::Tournament], 1));
    hand(&mut g, P0, &[Card::Followers]);
    deck(&mut g, P0, &[Card::Copper; 2]);
    g.start();
    play(&mut g, Card::Followers);
    assert_eq!(discard_kinds(&g, P0), [Card::Estate]);
    // P1 gains a Curse and discards before P2 gains one.
    let curses = |g: &Game| [P1, P2].map(|p| g.count_owned(p, Card::Curse));
    let d = g.pending().unwrap();
    assert_eq!((d.player, d.purpose), (P1, Purpose::Discard));
    assert_eq!(curses(&g), [1, 0]);
    let hand = candidates(&g);
    choose(&mut g, &hand[..2]);
    let d = g.pending().unwrap();
    assert_eq!((d.player, d.purpose), (P2, Purpose::Discard));
    assert_eq!(curses(&g), [1, 1]);
}

#[test]
fn doctor_trashes_named_cards_and_overpay_looks_at_the_top() {
    let mut g = setup(&[Card::Doctor]);
    hand(&mut g, P0, &[Card::Doctor]);
    deck(
        &mut g,
        P0,
        &[Card::Copper, Card::Silver, Card::Copper, Card::Gold],
    );
    g.start();
    play(&mut g, Card::Doctor);
    let trashed = g.trash().len();
    respond(&mut g, Response::Name(Card::Copper));
    assert_eq!(
        kinds(&g, &g.trash()[trashed..]),
        [Card::Copper, Card::Copper]
    );
    assert_eq!(deck_kinds(&g, P0), [Card::Silver, Card::Gold]);

    let mut g = setup(&[Card::Doctor]);
    hand(&mut g, P0, &[Card::Gold, Card::Gold]);
    deck(&mut g, P0, &[Card::Estate, Card::Silver]);
    g.start();
    play_treasures(&mut g);
    buy(&mut g, Card::Doctor);
    respond(&mut g, Response::Number(2));
    mode(&mut g, &[0]);
    mode(&mut g, &[2]);
    assert_eq!(kinds(&g, g.trash()).last(), Some(&Card::Estate));
}

/// The trashed Trail is played and draws a card: the cards Doctor revealed are off the
/// deck, so it draws the one under them, and Doctor puts back the other two.
#[test]
fn a_draw_in_the_middle_of_doctor_takes_the_card_under_the_revealed_ones() {
    let mut g = setup(&[Card::Doctor, Card::Trail]);
    hand(&mut g, P0, &[Card::Doctor]);
    deck(
        &mut g,
        P0,
        &[Card::Trail, Card::Gold, Card::Silver, Card::Estate],
    );
    g.start();
    play(&mut g, Card::Doctor);
    respond(&mut g, Response::Name(Card::Trail));
    respond(&mut g, Response::option(Opt::Trigger(0)));
    assert_eq!(hand_kinds(&g, P0), [Card::Estate]);
    choose_kinds(&mut g, &[Card::Silver, Card::Gold]);
    assert_eq!(deck_kinds(&g, P0), [Card::Silver, Card::Gold]);
}

#[test]
fn farming_village_finds_a_treasure_or_action() {
    let mut g = setup(&[Card::FarmingVillage]);
    hand(&mut g, P0, &[Card::FarmingVillage]);
    deck(&mut g, P0, &[Card::Estate, Card::Duchy, Card::Silver]);
    g.start();
    play(&mut g, Card::FarmingVillage);
    assert_eq!(hand_kinds(&g, P0), [Card::Silver]);
    assert_eq!(discard_kinds(&g, P0), [Card::Estate, Card::Duchy]);
}

#[test]
fn fortune_teller_digs_for_victory_or_curse() {
    let mut g = setup(&[Card::FortuneTeller]);
    hand(&mut g, P0, &[Card::FortuneTeller]);
    deck(
        &mut g,
        P1,
        &[Card::Copper, Card::Silver, Card::Curse, Card::Gold],
    );
    discard(&mut g, P1, &[]);
    g.start();
    play(&mut g, Card::FortuneTeller);
    assert_eq!(deck_kinds(&g, P1), [Card::Curse, Card::Gold]);
    assert_eq!(discard_kinds(&g, P1), [Card::Copper, Card::Silver]);
}

#[test]
fn harvest_pays_per_differently_named_card() {
    let mut g = setup(&[Card::Harvest]);
    hand(&mut g, P0, &[Card::Harvest]);
    deck(
        &mut g,
        P0,
        &[Card::Copper, Card::Silver, Card::Copper, Card::Estate],
    );
    g.start();
    play(&mut g, Card::Harvest);
    assert_eq!(g.turn().coins, 3);
    assert_eq!(g.player(P0).discard.len(), 4);
}

#[test]
fn horse_traders_sets_aside_against_an_attack_and_returns() {
    let mut g = setup(&[Card::HorseTraders, Card::Militia]);
    hand(&mut g, P0, &[Card::Militia]);
    hand(
        &mut g,
        P1,
        &[
            Card::HorseTraders,
            Card::Copper,
            Card::Copper,
            Card::Copper,
            Card::Estate,
        ],
    );
    deck(&mut g, P1, &[Card::Gold; 6]);
    g.start();
    play(&mut g, Card::Militia);
    respond(&mut g, Response::option(Opt::Trigger(0)));
    // Down to 4 cards: discards one.
    choose_kinds(&mut g, &[Card::Estate]);
    let ht = g.player(P1).set_aside[0];
    end_phase(&mut g);
    // P1's turn: +1 Card and Horse Traders back in hand.
    assert_eq!(g.zone(ht), Zone::Hand(P1));
    assert_eq!(g.hand(P1).len(), 3 + 1 + 1);
}

#[test]
fn masterpiece_overpay_gains_silvers() {
    let mut g = setup(&[Card::Masterpiece]);
    hand(&mut g, P0, &[Card::Gold, Card::Gold]);
    g.start();
    play_treasures(&mut g);
    buy(&mut g, Card::Masterpiece);
    respond(&mut g, Response::Number(3));
    assert_eq!(g.count_owned(P0, Card::Silver), 3);
}

#[test]
fn overpay_applies_to_the_bought_card_not_to_a_copy_gained_first() {
    let mut g = setup(&[Card::Masterpiece, Card::Talisman]);
    hand(&mut g, P0, &[Card::Talisman, Card::Gold, Card::Gold]);
    g.start();
    play_treasures(&mut g);
    buy(&mut g, Card::Masterpiece);
    respond(&mut g, Response::Number(2));
    assert_eq!(
        kinds(&g, &g.player(P0).discard[..4]),
        [
            Card::Masterpiece,
            Card::Masterpiece,
            Card::Silver,
            Card::Silver
        ]
    );
}

#[test]
fn taxman_makes_others_discard_a_copy_and_gains_onto_the_deck() {
    let mut g = setup(&[Card::Taxman]);
    hand(&mut g, P0, &[Card::Taxman, Card::Silver]);
    hand(
        &mut g,
        P1,
        &[
            Card::Silver,
            Card::Copper,
            Card::Copper,
            Card::Copper,
            Card::Estate,
        ],
    );
    g.start();
    play(&mut g, Card::Taxman);
    choose_kinds(&mut g, &[Card::Silver]);
    assert_eq!(discard_kinds(&g, P1), [Card::Silver]);
    choose_pile(&mut g, Card::Gold);
    assert_eq!(deck_kinds(&g, P0)[0], Card::Gold);
}

#[test]
fn coronet_stays_in_play_with_a_duration_it_played() {
    let mut g = setup(&[Card::Joust, Card::Caravan]);
    hand(&mut g, P0, &[Card::Coronet, Card::Caravan]);
    deck(&mut g, P0, &[Card::Copper; 12]);
    g.start();
    play(&mut g, Card::Coronet);
    choose_kinds(&mut g, &[Card::Caravan]);
    // No Treasure.
    choose(&mut g, &[]);
    end_phase(&mut g);
    end_phase(&mut g);
    assert_eq!(kinds(&g, g.in_play(P0)), [Card::Coronet, Card::Caravan]);
}
