use super::*;
use rand::seq::SliceRandom;

pub(super) fn discard_kinds(g: &Game, p: PlayerId) -> Vec<Card> {
    kinds(g, &g.player(p).discard)
}

pub(super) fn trash_kinds(g: &Game) -> Vec<Card> {
    kinds(g, g.trash())
}

pub(super) fn mode(g: &mut Game, i: u8) {
    respond(g, Response::option(Opt::Mode(i)));
}

pub(super) fn trigger(g: &mut Game, i: u16) {
    assert_eq!(purpose(g), Purpose::Trigger);
    respond(g, Response::option(Opt::Trigger(i)));
}

pub(super) fn name(g: &mut Game, card: Card) {
    respond(g, Response::Name(card));
}

pub(super) fn vp_tokens(g: &Game, p: PlayerId) -> i32 {
    g.player(p).vp_tokens
}

/// The cards named by the piles of the pending `Pile` decision.
pub(super) fn pile_choices(g: &Game) -> Vec<Card> {
    match &g.pending().expect("a decision is pending").kind {
        DecisionKind::Pile { piles, .. } => piles.iter().map(|&p| g.pile(p).card).collect(),
        k => panic!("expected a Pile decision, got {k:?}"),
    }
}

/// Whether the pending turn decision offers buying `card`.
pub(super) fn can_buy(g: &Game, card: Card) -> bool {
    let pile = g.supply_pile(card).expect("pile in the Supply");
    match &g.pending().expect("a decision is pending").kind {
        DecisionKind::Turn { moves, .. } => moves.contains(&Move::Buy(pile)),
        k => panic!("expected a Turn decision, got {k:?}"),
    }
}

pub(super) fn empty_pile(g: &mut Game, card: Card) {
    let pile = g.supply_pile(card).expect("pile in the Supply");
    for c in g.pile(pile).cards.clone() {
        g.move_card(c, Zone::Trash, crate::game::Pos::Top);
    }
}

/// Plays `card` from `p`'s hand (whoever decides).
pub(super) fn play_from(g: &mut Game, p: PlayerId, card: Card) {
    let id = find(g, p, card);
    respond(g, Response::Move(Move::Play(id)));
}

fn turn_starts(g: &Game) -> usize {
    g.log()
        .iter()
        .filter(|e| matches!(e, Event::TurnStart { .. }))
        .count()
}

/// Ends phases until the next turn starts.
pub(super) fn next_turn(g: &mut Game) {
    let n = turn_starts(g);
    while turn_starts(g) == n {
        match &g.pending().expect("a decision is pending").kind {
            DecisionKind::Turn { .. } => end_phase(g),
            k => panic!("unexpected decision {k:?}"),
        }
    }
}

#[test]
fn anvil_discards_a_treasure_to_gain_a_card_up_to_four() {
    let mut g = setup(&[Card::Anvil, Card::Smithy]);
    hand(&mut g, P0, &[Card::Anvil, Card::Silver]);
    g.start();
    play(&mut g, Card::Anvil);
    assert_eq!(g.turn().coins, 1);
    choose_kinds(&mut g, &[Card::Silver]);
    let piles = pile_choices(&g);
    assert!(piles.contains(&Card::Smithy) && piles.contains(&Card::Silver));
    assert!(!piles.contains(&Card::Gold) && !piles.contains(&Card::Duchy));
    choose_pile(&mut g, Card::Smithy);
    assert_eq!(discard_kinds(&g, P0), [Card::Silver, Card::Smithy]);
}

#[test]
fn anvil_may_decline_to_discard() {
    let mut g = setup(&[Card::Anvil]);
    hand(&mut g, P0, &[Card::Anvil, Card::Silver]);
    g.start();
    play(&mut g, Card::Anvil);
    choose(&mut g, &[]);
    assert_eq!(discard_kinds(&g, P0), []);
    assert_eq!(hand_kinds(&g, P0), [Card::Silver]);
}

#[test]
fn bank_counts_the_treasures_in_play_including_itself() {
    let mut g = setup(&[Card::Bank]);
    hand(
        &mut g,
        P0,
        &[Card::Copper, Card::Silver, Card::Bank, Card::Bank],
    );
    g.start();
    play(&mut g, Card::Copper);
    play(&mut g, Card::Silver);
    play(&mut g, Card::Bank);
    assert_eq!(g.turn().coins, 1 + 2 + 3);
    play(&mut g, Card::Bank);
    assert_eq!(g.turn().coins, 1 + 2 + 3 + 4);
}

#[test]
fn bishop_scores_half_the_trashed_cost_and_others_may_trash() {
    let mut g = setup(&[Card::Bishop]);
    hand(&mut g, P0, &[Card::Bishop, Card::Gold, Card::Estate]);
    hand(&mut g, P1, &[Card::Estate, Card::Copper, Card::Copper]);
    g.start();
    play(&mut g, Card::Bishop);
    assert_eq!((g.turn().coins, vp_tokens(&g, P0)), (1, 1));
    choose_kinds(&mut g, &[Card::Gold]);
    assert_eq!(vp_tokens(&g, P0), 1 + 3);
    assert_eq!(
        (g.pending().unwrap().player, purpose(&g)),
        (P1, Purpose::Trash)
    );
    choose_kinds(&mut g, &[Card::Estate]);
    assert_eq!(vp_tokens(&g, P1), 0);
    assert_eq!(
        &trash_kinds(&g)[trash_kinds(&g).len() - 2..],
        [Card::Gold, Card::Estate]
    );
}

#[test]
fn bishop_with_an_empty_hand_still_gives_a_coin_and_a_vp() {
    let mut g = setup(&[Card::Bishop]);
    hand(&mut g, P0, &[Card::Bishop]);
    hand(&mut g, P1, &[]);
    g.start();
    play(&mut g, Card::Bishop);
    assert_eq!((g.turn().coins, vp_tokens(&g, P0)), (1, 1));
}

#[test]
fn charlatan_curses_and_makes_curse_a_one_coin_treasure() {
    let mut g = setup(&[Card::Charlatan]);
    let ids = hand(&mut g, P0, &[Card::Charlatan, Card::Curse]);
    g.start();
    assert!(g.is(ids[1], Types::TREASURE | Types::CURSE));
    play(&mut g, Card::Charlatan);
    assert_eq!(g.turn().coins, 3);
    assert_eq!(discard_kinds(&g, P1), [Card::Curse]);
    play(&mut g, Card::Curse);
    assert_eq!(g.turn().coins, 4);
    // Still a Curse when scoring.
    let estates = g.count_owned(P0, Card::Estate) as i32;
    assert_eq!(g.score(P0), estates - 1);
}

#[test]
fn without_charlatan_a_curse_is_no_treasure() {
    let mut g = setup(&[Card::Smithy]);
    let ids = hand(&mut g, P0, &[Card::Curse]);
    g.start();
    assert!(!g.is(ids[0], Types::TREASURE));
}

#[test]
fn moat_blocks_charlatan() {
    let mut g = setup(&[Card::Charlatan, Card::Moat]);
    hand(&mut g, P0, &[Card::Charlatan]);
    hand(&mut g, P1, &[Card::Moat]);
    g.start();
    play(&mut g, Card::Charlatan);
    trigger(&mut g, 0);
    assert_eq!(discard_kinds(&g, P1), []);
    assert_eq!(g.turn().coins, 3);
}

#[test]
fn city_scales_with_empty_supply_piles() {
    for (empty, cards, buys, coins) in [(0, 1, 1, 0), (1, 2, 1, 0), (2, 2, 2, 1), (3, 2, 2, 1)] {
        let mut g = setup(&[Card::City, Card::Smithy, Card::Village]);
        hand(&mut g, P0, &[Card::City]);
        deck(&mut g, P0, &[Card::Estate; 5]);
        for &card in &[Card::Curse, Card::Smithy, Card::Village][..empty] {
            empty_pile(&mut g, card);
        }
        g.start();
        play(&mut g, Card::City);
        let t = g.turn();
        assert_eq!(
            (g.hand(P0).len(), t.actions, t.buys, t.coins),
            (cards, 2, buys, coins),
            "{empty} empty piles"
        );
    }
}

#[test]
fn city_played_on_another_players_turn_gives_no_buy() {
    let mut g = setup(&[Card::City, Card::Ambassador, Card::Gondola]);
    empty_pile(&mut g, Card::Curse);
    empty_pile(&mut g, Card::Estate);
    hand(&mut g, P0, &[Card::Ambassador, Card::Gondola]);
    hand(&mut g, P1, &[Card::City]);
    deck(&mut g, P1, &[Card::Copper; 5]);
    g.start();
    play_on_another_players_turn(&mut g);
    assert_eq!(g.hand(P1).len(), 2);
    let t = g.turn();
    assert_eq!((t.player, t.actions, t.buys, t.coins), (P0, 0, 1, 0));
}

#[test]
fn clerk_makes_players_with_five_cards_topdeck_one() {
    let mut g = setup(&[Card::Clerk]);
    hand(&mut g, P0, &[Card::Clerk]);
    hand(
        &mut g,
        P1,
        &[
            Card::Copper,
            Card::Copper,
            Card::Copper,
            Card::Estate,
            Card::Gold,
        ],
    );
    g.start();
    // Declines playing it at the start of the turn.
    no(&mut g);
    play(&mut g, Card::Clerk);
    assert_eq!(g.turn().coins, 2);
    assert_eq!(
        (g.pending().unwrap().player, purpose(&g)),
        (P1, Purpose::TopDeck)
    );
    choose_kinds(&mut g, &[Card::Estate]);
    assert_eq!(deck_kinds(&g, P1)[0], Card::Estate);
    assert_eq!(g.hand(P1).len(), 4);
}

#[test]
fn clerk_ignores_players_with_fewer_than_five_cards() {
    let mut g = setup(&[Card::Clerk]);
    hand(&mut g, P0, &[Card::Clerk]);
    hand(&mut g, P1, &[Card::Copper; 4]);
    g.start();
    no(&mut g);
    play(&mut g, Card::Clerk);
    assert_eq!(g.hand(P1).len(), 4);
    assert_eq!(g.pending().unwrap().player, P0);
}

#[test]
fn clerks_may_be_played_at_the_start_of_the_turn_without_an_action() {
    let mut g = setup(&[Card::Clerk, Card::Village]);
    hand(&mut g, P0, &[Card::Clerk, Card::Clerk, Card::Village]);
    hand(
        &mut g,
        P1,
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
    trigger(&mut g, 0);
    assert_eq!(g.pending().unwrap().player, P1);
    let c = g.hand(P1)[0];
    choose(&mut g, &[c]);
    trigger(&mut g, 0);
    // P1 is down to 5 cards, so the second Clerk hits them too.
    let c = g.hand(P1)[0];
    choose(&mut g, &[c]);
    assert_eq!(g.hand(P1).len(), 4);
    assert_eq!(kinds(&g, g.in_play(P0)), [Card::Clerk, Card::Clerk]);
    assert_eq!((g.turn().actions, g.turn().coins), (1, 4));
    assert_eq!(g.turn().phase, Phase::Action);
}

#[test]
fn collection_gives_a_vp_per_action_gained_after_it_is_played() {
    let mut g = setup(&[Card::Collection, Card::Smithy]);
    hand(
        &mut g,
        P0,
        &[Card::Collection, Card::Collection, Card::Gold],
    );
    g.start();
    play(&mut g, Card::Collection);
    play(&mut g, Card::Collection);
    play(&mut g, Card::Gold);
    assert_eq!((g.turn().coins, g.turn().buys), (7, 3));
    buy(&mut g, Card::Smithy);
    assert_eq!(vp_tokens(&g, P0), 2);
    buy(&mut g, Card::Silver);
    assert_eq!(vp_tokens(&g, P0), 2);
}

#[test]
fn crystal_ball_may_play_an_action_from_the_deck_in_the_buy_phase() {
    let mut g = setup(&[Card::CrystalBall, Card::Smithy]);
    hand(&mut g, P0, &[Card::CrystalBall]);
    deck(
        &mut g,
        P0,
        &[Card::Smithy, Card::Copper, Card::Copper, Card::Copper],
    );
    g.start();
    play(&mut g, Card::CrystalBall);
    assert_eq!(g.turn().coins, 1);
    mode(&mut g, 2);
    assert_eq!(hand_kinds(&g, P0), [Card::Copper; 3]);
    assert_eq!(g.turn().actions, 1);
    assert_eq!(g.turn().phase, Phase::Buy);
}

#[test]
fn crystal_ball_may_trash_discard_or_leave_the_top_card() {
    for (i, trashed, discarded, on_deck) in [
        (0, true, false, false),
        (1, false, true, false),
        (2, false, false, true),
    ] {
        let mut g = setup(&[Card::CrystalBall]);
        hand(&mut g, P0, &[Card::CrystalBall]);
        deck(&mut g, P0, &[Card::Estate]);
        g.start();
        play(&mut g, Card::CrystalBall);
        // An Estate can't be played: the options are trash, discard, leave.
        match &g.pending().unwrap().kind {
            DecisionKind::Choose { options, .. } => assert_eq!(options.len(), 3),
            k => panic!("{k:?}"),
        }
        let pick = [0, 1, 3][i];
        let trash = g.trash().len();
        mode(&mut g, pick);
        assert_eq!(g.trash().len() == trash + 1, trashed);
        assert_eq!(discard_kinds(&g, P0) == [Card::Estate], discarded);
        assert_eq!(deck_kinds(&g, P0) == [Card::Estate], on_deck);
    }
}

#[test]
fn expand_gains_a_card_costing_up_to_three_more() {
    let mut g = setup(&[Card::Expand]);
    hand(&mut g, P0, &[Card::Expand, Card::Silver]);
    g.start();
    play(&mut g, Card::Expand);
    let piles = pile_choices(&g);
    assert!(piles.contains(&Card::Gold) && piles.contains(&Card::Silver));
    assert!(!piles.contains(&Card::Province));
    choose_pile(&mut g, Card::Gold);
    assert_eq!(discard_kinds(&g, P0), [Card::Gold]);
    assert_eq!(trash_kinds(&g).last(), Some(&Card::Silver));
}

#[test]
fn forge_gains_a_card_costing_exactly_the_total() {
    let mut g = setup(&[Card::Forge, Card::Market]);
    hand(
        &mut g,
        P0,
        &[Card::Forge, Card::Estate, Card::Silver, Card::Copper],
    );
    g.start();
    play(&mut g, Card::Forge);
    choose_kinds(&mut g, &[Card::Estate, Card::Silver]);
    let mut piles = pile_choices(&g);
    piles.sort();
    assert_eq!(piles, [Card::Duchy, Card::Market]);
    choose_pile(&mut g, Card::Market);
    assert_eq!(discard_kinds(&g, P0), [Card::Market]);
}

#[test]
fn forge_trashing_nothing_gains_a_zero_cost_card() {
    let mut g = setup(&[Card::Forge]);
    hand(&mut g, P0, &[Card::Forge, Card::Estate]);
    g.start();
    play(&mut g, Card::Forge);
    choose(&mut g, &[]);
    let mut piles = pile_choices(&g);
    piles.sort();
    assert_eq!(piles, [Card::Copper, Card::Curse]);
}

#[test]
fn grand_market_cannot_be_bought_with_a_copper_in_play() {
    let mut g = setup(&[Card::GrandMarket]);
    hand(
        &mut g,
        P0,
        &[Card::Gold, Card::Silver, Card::Silver, Card::Copper],
    );
    g.start();
    play(&mut g, Card::Gold);
    play(&mut g, Card::Silver);
    play(&mut g, Card::Silver);
    // A Copper in hand does not stop it.
    assert!(can_buy(&g, Card::GrandMarket));
    play(&mut g, Card::Copper);
    assert!(!can_buy(&g, Card::GrandMarket));
}

#[test]
fn hoard_gains_a_gold_per_hoard_when_buying_a_victory_card() {
    let mut g = setup(&[Card::Hoard]);
    hand(&mut g, P0, &[Card::Hoard, Card::Hoard, Card::Gold]);
    g.start();
    play(&mut g, Card::Hoard);
    play(&mut g, Card::Hoard);
    play(&mut g, Card::Gold);
    g.turn.buys = 2;
    buy(&mut g, Card::Silver);
    assert_eq!(discard_kinds(&g, P0), [Card::Silver]);
    buy(&mut g, Card::Estate);
    assert_eq!(
        &discard_kinds(&g, P0)[..4],
        [Card::Silver, Card::Estate, Card::Gold, Card::Gold]
    );
}

#[test]
fn hoard_ignores_victory_cards_gained_without_buying() {
    let mut g = setup(&[Card::Hoard, Card::Anvil]);
    hand(&mut g, P0, &[Card::Hoard, Card::Anvil, Card::Copper]);
    g.start();
    play(&mut g, Card::Hoard);
    play(&mut g, Card::Anvil);
    choose_kinds(&mut g, &[Card::Copper]);
    choose_pile(&mut g, Card::Estate);
    assert_eq!(discard_kinds(&g, P0), [Card::Copper, Card::Estate]);
}

#[test]
fn hoard_still_gains_gold_when_watchtower_trashes_the_victory_card() {
    let mut g = setup(&[Card::Hoard, Card::Watchtower]);
    hand(&mut g, P0, &[Card::Hoard, Card::Watchtower]);
    g.start();
    end_phase(&mut g);
    play(&mut g, Card::Hoard);
    buy(&mut g, Card::Estate);
    // Watchtower and Hoard both trigger; resolve Watchtower first.
    let labels = match &g.pending().unwrap().kind {
        DecisionKind::Choose { labels, .. } => labels.clone(),
        k => panic!("{k:?}"),
    };
    let wt = labels
        .iter()
        .position(|l| l.starts_with("Watchtower"))
        .unwrap();
    trigger(&mut g, wt as u16);
    mode(&mut g, 0);
    assert_eq!(trash_kinds(&g).last(), Some(&Card::Estate));
    assert!(discard_kinds(&g, P0).contains(&Card::Gold));
}

#[test]
fn investment_trashes_a_card_then_gives_a_coin() {
    let mut g = setup(&[Card::Investment]);
    hand(&mut g, P0, &[Card::Investment, Card::Estate]);
    g.start();
    play(&mut g, Card::Investment);
    mode(&mut g, 0);
    assert_eq!(g.turn().coins, 1);
    assert_eq!(trash_kinds(&g).last(), Some(&Card::Estate));
    assert_eq!(kinds(&g, g.in_play(P0)), [Card::Investment]);
}

#[test]
fn investment_trashes_itself_for_a_vp_per_differently_named_treasure() {
    let mut g = setup(&[Card::Investment]);
    hand(
        &mut g,
        P0,
        &[
            Card::Investment,
            Card::Estate,
            Card::Copper,
            Card::Copper,
            Card::Silver,
        ],
    );
    g.start();
    play(&mut g, Card::Investment);
    choose_kinds(&mut g, &[Card::Estate]);
    mode(&mut g, 1);
    assert_eq!(vp_tokens(&g, P0), 2);
    assert_eq!(g.turn().coins, 0);
    assert_eq!(trash_kinds(&g).last(), Some(&Card::Investment));
    // The revealed Treasures can still be played.
    play(&mut g, Card::Silver);
    assert_eq!(g.turn().coins, 2);
}

#[test]
fn kings_court_plays_an_action_three_times() {
    let mut g = setup(&[Card::KingsCourt, Card::Village]);
    hand(&mut g, P0, &[Card::KingsCourt, Card::Village]);
    deck(&mut g, P0, &[Card::Copper; 5]);
    g.start();
    play(&mut g, Card::KingsCourt);
    choose_kinds(&mut g, &[Card::Village]);
    assert_eq!(g.turn().actions, 6);
    assert_eq!(g.hand(P0).len(), 3);
    assert_eq!(
        g.turn().played,
        [
            Card::KingsCourt,
            Card::Village,
            Card::Village,
            Card::Village
        ]
    );
}

#[test]
fn kings_court_may_play_nothing() {
    let mut g = setup(&[Card::KingsCourt, Card::Village]);
    hand(&mut g, P0, &[Card::KingsCourt, Card::Village]);
    g.start();
    play(&mut g, Card::KingsCourt);
    choose(&mut g, &[]);
    assert_eq!(hand_kinds(&g, P0), [Card::Village]);
}

#[test]
fn magnate_draws_a_card_per_treasure_in_hand() {
    let mut g = setup(&[Card::Magnate]);
    hand(
        &mut g,
        P0,
        &[Card::Magnate, Card::Copper, Card::Silver, Card::Estate],
    );
    deck(&mut g, P0, &[Card::Estate; 5]);
    g.start();
    play(&mut g, Card::Magnate);
    assert_eq!(g.hand(P0).len(), 3 + 2);
}

#[test]
fn mint_gains_a_copy_of_a_revealed_treasure() {
    let mut g = setup(&[Card::Mint]);
    hand(&mut g, P0, &[Card::Mint, Card::Silver]);
    g.start();
    play(&mut g, Card::Mint);
    choose_kinds(&mut g, &[Card::Silver]);
    assert_eq!(discard_kinds(&g, P0), [Card::Silver]);
    assert_eq!(hand_kinds(&g, P0), [Card::Silver]);
}

#[test]
fn gaining_mint_trashes_non_duration_treasures_in_play() {
    let mut g = setup(&[Card::Mint, Card::Astrolabe]);
    hand(
        &mut g,
        P0,
        &[
            Card::Astrolabe,
            Card::Copper,
            Card::Copper,
            Card::Copper,
            Card::Copper,
            Card::Silver,
        ],
    );
    g.start();
    for _ in 0..4 {
        play(&mut g, Card::Copper);
    }
    play(&mut g, Card::Astrolabe);
    assert_eq!(g.turn().coins, 5);
    buy(&mut g, Card::Mint);
    assert_eq!(kinds(&g, g.in_play(P0)), [Card::Astrolabe]);
    assert_eq!(
        &trash_kinds(&g)[trash_kinds(&g).len() - 4..],
        [Card::Copper; 4]
    );
    // The Silver in hand stays.
    assert_eq!(hand_kinds(&g, P0), [Card::Silver]);
}

#[test]
fn monument_gives_two_coins_and_a_vp() {
    let mut g = setup(&[Card::Monument]);
    hand(&mut g, P0, &[Card::Monument]);
    g.start();
    play(&mut g, Card::Monument);
    assert_eq!((g.turn().coins, vp_tokens(&g, P0)), (2, 1));
}

#[test]
fn peddler_costs_two_less_per_action_in_play_during_the_buy_phase() {
    let mut g = setup(&[Card::Peddler, Card::KingsCourt, Card::WorkersVillage]);
    hand(
        &mut g,
        P0,
        &[Card::KingsCourt, Card::WorkersVillage, Card::Peddler],
    );
    deck(&mut g, P0, &[Card::Copper; 5]);
    g.start();
    play(&mut g, Card::KingsCourt);
    choose_kinds(&mut g, &[Card::WorkersVillage]);
    assert_eq!(g.turn().phase, Phase::Action);
    assert_eq!(g.cost(Card::Peddler), Cost::coins(8));
    end_phase(&mut g);
    // King's Court and one Worker's Village: two cards in play, however often played.
    assert_eq!(g.cost(Card::Peddler), Cost::coins(4));
    // Every Peddler, including the one in hand.
    assert_eq!(g.cost_of(find(&g, P0, Card::Peddler)), Cost::coins(4));
    assert_eq!(g.cost(Card::KingsCourt), Cost::coins(7));
}

#[test]
fn quarry_makes_actions_cost_two_less_cumulatively() {
    let mut g = setup(&[Card::Quarry, Card::KingsCourt, Card::Peddler]);
    hand(&mut g, P0, &[Card::Quarry, Card::Quarry]);
    g.start();
    play(&mut g, Card::Quarry);
    assert_eq!(g.cost(Card::KingsCourt), Cost::coins(5));
    play(&mut g, Card::Quarry);
    assert_eq!(g.cost(Card::KingsCourt), Cost::coins(3));
    assert_eq!(g.cost(Card::Peddler), Cost::coins(4));
    assert_eq!(g.cost(Card::Silver), Cost::coins(3));
    assert_eq!(g.turn().coins, 2);
    next_turn(&mut g);
    assert_eq!(g.cost(Card::KingsCourt), Cost::coins(7));
}

#[test]
fn rabble_discards_actions_and_treasures_and_the_victim_orders_the_rest() {
    let mut g = setup(&[Card::Rabble, Card::Smithy]);
    hand(&mut g, P0, &[Card::Rabble]);
    deck(&mut g, P0, &[Card::Copper; 3]);
    deck(
        &mut g,
        P1,
        &[Card::Estate, Card::Smithy, Card::Duchy, Card::Gold],
    );
    g.start();
    play(&mut g, Card::Rabble);
    assert_eq!(g.hand(P0).len(), 3);
    let d = g.pending().unwrap();
    assert_eq!((d.player, d.purpose), (P1, Purpose::TopDeck));
    let mut order = candidates(&g);
    order.reverse();
    assert_eq!(kinds(&g, &order), [Card::Duchy, Card::Estate]);
    choose(&mut g, &order);
    assert_eq!(discard_kinds(&g, P1), [Card::Smithy]);
    assert_eq!(deck_kinds(&g, P1), [Card::Duchy, Card::Estate, Card::Gold]);
}

#[test]
fn tiara_plays_a_treasure_twice_and_may_topdeck_gained_cards() {
    let mut g = setup(&[Card::Tiara, Card::Smithy]);
    hand(&mut g, P0, &[Card::Tiara, Card::Silver, Card::Gold]);
    deck(&mut g, P0, &[Card::Copper; 5]);
    g.start();
    play(&mut g, Card::Tiara);
    choose_kinds(&mut g, &[Card::Silver]);
    assert_eq!((g.turn().coins, g.turn().buys), (4, 2));
    play(&mut g, Card::Gold);
    buy(&mut g, Card::Smithy);
    trigger(&mut g, 0);
    assert_eq!(deck_kinds(&g, P0)[0], Card::Smithy);
}

#[test]
fn tiara_still_topdecks_a_mint_whose_gain_trashed_it() {
    let mut g = setup(&[Card::Tiara, Card::Mint]);
    hand(&mut g, P0, &[Card::Tiara, Card::Gold]);
    deck(&mut g, P0, &[Card::Copper; 5]);
    g.start();
    play(&mut g, Card::Tiara);
    choose_kinds(&mut g, &[Card::Gold]);
    buy(&mut g, Card::Mint);
    // Mint's trash first, then Tiara's topdeck.
    let labels = match &g.pending().unwrap().kind {
        DecisionKind::Choose { labels, .. } => labels.clone(),
        k => panic!("{k:?}"),
    };
    let mint = labels.iter().position(|l| l == "Mint").unwrap();
    trigger(&mut g, mint as u16);
    assert!(g.in_play(P0).is_empty());
    trigger(&mut g, 0);
    assert_eq!(deck_kinds(&g, P0)[0], Card::Mint);
}

#[test]
fn vault_discards_for_coins_and_others_may_discard_two_to_draw() {
    let mut g = setup(&[Card::Vault]);
    hand(&mut g, P0, &[Card::Vault, Card::Estate]);
    deck(&mut g, P0, &[Card::Estate, Card::Copper, Card::Copper]);
    hand(&mut g, P1, &[Card::Estate, Card::Estate, Card::Copper]);
    deck(&mut g, P1, &[Card::Gold]);
    g.start();
    play(&mut g, Card::Vault);
    choose_kinds(&mut g, &[Card::Estate, Card::Estate]);
    assert_eq!(g.turn().coins, 2);
    assert_eq!(g.pending().unwrap().player, P1);
    yes(&mut g);
    choose_kinds(&mut g, &[Card::Estate, Card::Estate]);
    assert_eq!(hand_kinds(&g, P1), [Card::Copper, Card::Gold]);
}

#[test]
fn vault_victim_with_one_card_may_discard_it_but_draws_nothing() {
    let mut g = setup(&[Card::Vault]);
    hand(&mut g, P0, &[Card::Vault]);
    hand(&mut g, P1, &[Card::Estate]);
    deck(&mut g, P1, &[Card::Gold]);
    g.start();
    play(&mut g, Card::Vault);
    choose(&mut g, &[]);
    yes(&mut g);
    assert!(g.hand(P1).is_empty());
    assert_eq!(discard_kinds(&g, P1), [Card::Estate]);
}

#[test]
fn war_chest_cannot_gain_cards_named_for_war_chests_this_turn() {
    let mut g = setup(&[Card::WarChest, Card::Smithy, Card::Village]);
    hand(&mut g, P0, &[Card::WarChest, Card::WarChest]);
    g.start();
    play(&mut g, Card::WarChest);
    let d = g.pending().unwrap();
    assert_eq!((d.player, d.kind.clone()), (P1, DecisionKind::NameCard));
    name(&mut g, Card::Smithy);
    let piles = pile_choices(&g);
    assert!(!piles.contains(&Card::Smithy) && !piles.contains(&Card::Gold));
    assert!(piles.contains(&Card::Village) && piles.contains(&Card::Duchy));
    choose_pile(&mut g, Card::Village);
    assert_eq!(discard_kinds(&g, P0), [Card::Village]);
    play(&mut g, Card::WarChest);
    name(&mut g, Card::Village);
    let piles = pile_choices(&g);
    assert!(!piles.contains(&Card::Smithy) && !piles.contains(&Card::Village));
}

#[test]
fn watchtower_draws_to_six() {
    let mut g = setup(&[Card::Watchtower]);
    hand(&mut g, P0, &[Card::Watchtower, Card::Copper, Card::Copper]);
    deck(&mut g, P0, &[Card::Estate; 6]);
    g.start();
    play(&mut g, Card::Watchtower);
    assert_eq!(g.hand(P0).len(), 6);
}

#[test]
fn watchtower_shuffles_once_and_stops_when_the_cards_run_out() {
    let mut g = setup(&[Card::Watchtower]);
    hand(&mut g, P0, &[Card::Watchtower]);
    deck(&mut g, P0, &[Card::Estate]);
    discard(&mut g, P0, &[Card::Copper, Card::Copper]);
    g.start();
    play(&mut g, Card::Watchtower);
    assert_eq!(g.hand(P0).len(), 3);
    assert!(g.player(P0).deck.is_empty() && g.player(P0).discard.is_empty());
    let shuffles = g
        .log()
        .iter()
        .filter(|e| matches!(e, Event::Shuffled { .. }));
    assert_eq!(shuffles.count(), 1);
}

#[test]
fn watchtower_trashes_a_curse_gained_on_another_players_turn() {
    let mut g = setup(&[Card::Watchtower, Card::Charlatan]);
    hand(&mut g, P0, &[Card::Charlatan]);
    hand(&mut g, P1, &[Card::Watchtower]);
    g.start();
    play(&mut g, Card::Charlatan);
    assert_eq!(g.pending().unwrap().player, P1);
    trigger(&mut g, 0);
    mode(&mut g, 0);
    assert_eq!(trash_kinds(&g).last(), Some(&Card::Curse));
    assert_eq!(hand_kinds(&g, P1), [Card::Watchtower]);
}

#[test]
fn watchtower_puts_a_bought_card_onto_the_deck() {
    let mut g = setup(&[Card::Watchtower]);
    hand(&mut g, P0, &[Card::Watchtower, Card::Gold]);
    deck(&mut g, P0, &[]);
    g.start();
    end_phase(&mut g);
    play(&mut g, Card::Gold);
    g.turn.buys = 2;
    buy(&mut g, Card::Silver);
    trigger(&mut g, 0);
    mode(&mut g, 1);
    assert_eq!(deck_kinds(&g, P0), [Card::Silver]);
    assert!(g.player(P0).discard.is_empty());
}

#[test]
fn colonies_add_platinum_and_colony_piles() {
    let g = setup_with(GameConfig {
        colonies: Some(true),
        ..GameConfig::new(2, vec![Card::Bank], 1)
    });
    let size = |c| g.pile(g.supply_pile(c).unwrap()).cards.len();
    assert_eq!((size(Card::Platinum), size(Card::Colony)), (12, 8));
}

#[test]
fn contraband_stops_the_player_buying_the_named_card() {
    let mut g = setup(&[Card::Contraband]);
    hand(&mut g, P0, &[Card::Contraband, Card::Gold]);
    g.start();
    play(&mut g, Card::Contraband);
    assert_eq!(g.pending().unwrap().player, P1);
    name(&mut g, Card::Gold);
    assert_eq!((g.turn().coins, g.turn().buys), (3, 2));
    play(&mut g, Card::Gold);
    assert!(!can_buy(&g, Card::Gold));
    assert!(can_buy(&g, Card::Duchy));
}

#[test]
fn counting_house_takes_coppers_from_the_discard_pile() {
    let mut g = setup(&[Card::CountingHouse]);
    hand(&mut g, P0, &[Card::CountingHouse]);
    discard(&mut g, P0, &[Card::Copper, Card::Estate, Card::Copper]);
    g.start();
    play(&mut g, Card::CountingHouse);
    let all = candidates(&g);
    choose(&mut g, &all);
    assert_eq!(hand_kinds(&g, P0), [Card::Copper, Card::Copper]);
    assert_eq!(discard_kinds(&g, P0), [Card::Estate]);
}

#[test]
fn goons_attacks_and_gives_a_vp_per_card_bought_per_copy_in_play() {
    let mut g = setup(&[Card::Goons, Card::KingsCourt]);
    hand(&mut g, P0, &[Card::KingsCourt, Card::Goons, Card::Gold]);
    hand(
        &mut g,
        P1,
        &[
            Card::Copper,
            Card::Silver,
            Card::Gold,
            Card::Estate,
            Card::Duchy,
        ],
    );
    g.start();
    play(&mut g, Card::KingsCourt);
    choose_kinds(&mut g, &[Card::Goons]);
    // The first play leaves P1 three cards; the others do nothing more.
    choose_kinds(&mut g, &[Card::Estate, Card::Duchy]);
    assert_eq!(hand_kinds(&g, P1), [Card::Copper, Card::Silver, Card::Gold]);
    assert_eq!((g.turn().coins, g.turn().buys), (6, 4));
    play(&mut g, Card::Gold);
    buy(&mut g, Card::Silver);
    buy(&mut g, Card::Copper);
    // One Goons in play, however often it was played.
    assert_eq!(vp_tokens(&g, P0), 2);
}

#[test]
fn loan_trashes_or_discards_the_first_treasure_and_discards_the_rest() {
    let mut g = setup(&[Card::Loan, Card::Smithy]);
    hand(&mut g, P0, &[Card::Loan]);
    deck(
        &mut g,
        P0,
        &[Card::Estate, Card::Smithy, Card::Silver, Card::Gold],
    );
    g.start();
    play(&mut g, Card::Loan);
    assert_eq!(g.turn().coins, 1);
    mode(&mut g, 1);
    assert_eq!(trash_kinds(&g).last(), Some(&Card::Silver));
    assert_eq!(discard_kinds(&g, P0), [Card::Estate, Card::Smithy]);
    assert_eq!(deck_kinds(&g, P0), [Card::Gold]);
}

#[test]
fn mountebank_victims_discard_a_curse_or_gain_a_curse_and_a_copper() {
    let mut g = setup_with(GameConfig {
        colonies: Some(false),
        shelters: Some(false),
        ..GameConfig::new(3, vec![Card::Mountebank], 7)
    });
    hand(&mut g, P0, &[Card::Mountebank]);
    hand(&mut g, P1, &[Card::Curse]);
    hand(&mut g, P2, &[Card::Curse]);
    g.start();
    play(&mut g, Card::Mountebank);
    assert_eq!(g.turn().coins, 2);
    assert_eq!(g.pending().unwrap().player, P1);
    yes(&mut g);
    assert_eq!(g.pending().unwrap().player, P2);
    no(&mut g);
    assert_eq!(discard_kinds(&g, P1), [Card::Curse]);
    assert!(g.hand(P1).is_empty());
    assert_eq!(discard_kinds(&g, P2), [Card::Curse, Card::Copper]);
}

#[test]
fn royal_seal_may_put_gained_cards_onto_the_deck() {
    let mut g = setup(&[Card::RoyalSeal, Card::Smithy]);
    hand(&mut g, P0, &[Card::RoyalSeal, Card::Silver]);
    g.start();
    play(&mut g, Card::RoyalSeal);
    play(&mut g, Card::Silver);
    g.turn.buys = 2;
    buy(&mut g, Card::Smithy);
    trigger(&mut g, 0);
    assert_eq!(deck_kinds(&g, P0)[0], Card::Smithy);
}

#[test]
fn talisman_gains_a_copy_of_each_cheap_non_victory_card_bought() {
    let mut g = setup(&[Card::Talisman, Card::Smithy, Card::Market]);
    hand(
        &mut g,
        P0,
        &[Card::Talisman, Card::Talisman, Card::Gold, Card::Gold],
    );
    g.start();
    for _ in 0..2 {
        play(&mut g, Card::Talisman);
        play(&mut g, Card::Gold);
    }
    g.turn.coins = 20;
    g.turn.buys = 4;
    buy(&mut g, Card::Smithy);
    assert_eq!(discard_kinds(&g, P0), [Card::Smithy; 3]);
    buy(&mut g, Card::Estate);
    buy(&mut g, Card::Market);
    assert_eq!(
        discard_kinds(&g, P0),
        [
            Card::Smithy,
            Card::Smithy,
            Card::Smithy,
            Card::Estate,
            Card::Market
        ]
    );
}

#[test]
fn trade_route_tokens_move_to_the_mat_when_victory_piles_are_gained_from() {
    let mut g = setup(&[Card::TradeRoute, Card::Gardens]);
    let tokens: Vec<Card> = g
        .piles()
        .iter()
        .filter(|p| p.trade_route)
        .map(|p| p.card)
        .collect();
    assert_eq!(
        tokens,
        [Card::Estate, Card::Duchy, Card::Province, Card::Gardens]
    );
    hand(
        &mut g,
        P0,
        &[Card::TradeRoute, Card::Copper, Card::Silver, Card::Gold],
    );
    g.start();
    play(&mut g, Card::TradeRoute);
    choose_kinds(&mut g, &[Card::Copper]);
    assert_eq!((g.turn().coins, g.turn().buys), (0, 2));
    play(&mut g, Card::Silver);
    play(&mut g, Card::Gold);
    buy(&mut g, Card::Estate);
    buy(&mut g, Card::Estate);
    // One token per pile, however many cards were gained from it.
    assert_eq!(g.trade_route_mat(), 1);
    let estates = g.supply_pile(Card::Estate).unwrap();
    assert!(!g.pile(estates).trade_route);
}

#[test]
fn gaining_a_victory_card_from_the_trash_moves_its_piles_trade_route_token() {
    let mut g = setup(&[Card::TradeRoute]);
    let duchy = g.create(Card::Duchy, Zone::Trash);
    g.start();
    g.stack
        .push(crate::frame::Frame::gain_card(P0, duchy, Dest::Discard));
    g.run(crate::frame::Input::Start);
    assert_eq!(g.trade_route_mat(), 1);
    assert!(!g.pile(g.supply_pile(Card::Duchy).unwrap()).trade_route);
}

#[test]
fn trade_route_gives_a_coin_per_token_on_the_mat() {
    let mut g = setup(&[Card::TradeRoute]);
    g.trade_route_mat = 2;
    hand(&mut g, P0, &[Card::TradeRoute, Card::Estate]);
    g.start();
    play(&mut g, Card::TradeRoute);
    assert_eq!(g.turn().coins, 2);
    assert_eq!(trash_kinds(&g).last(), Some(&Card::Estate));
}

#[test]
fn venture_plays_the_first_treasure_revealed() {
    let mut g = setup(&[Card::Venture]);
    hand(&mut g, P0, &[Card::Venture]);
    deck(
        &mut g,
        P0,
        &[
            Card::Estate,
            Card::Venture,
            Card::Duchy,
            Card::Silver,
            Card::Gold,
        ],
    );
    g.start();
    play(&mut g, Card::Venture);
    assert_eq!(g.turn().coins, 1 + 1 + 2);
    assert_eq!(discard_kinds(&g, P0), [Card::Estate, Card::Duchy]);
    assert_eq!(
        kinds(&g, g.in_play(P0)),
        [Card::Venture, Card::Venture, Card::Silver]
    );
    assert_eq!(deck_kinds(&g, P0), [Card::Gold]);
}

/// Random games over Prosperity (removed cards included) with Throne Room, Moat and
/// Watchtower, each player starting with a copy of every Kingdom card.
#[test]
fn random_prosperity_games_keep_invariants() {
    let pool: Vec<Card> = Card::in_set(Set::Prosperity)
        .filter(|c| c.def().kingdom)
        .collect();
    for seed in 0..fuzz_seeds(150) {
        let mut rng = ChaCha8Rng::seed_from_u64(seed);
        let mut cards = pool.clone();
        cards.shuffle(&mut rng);
        cards.truncate(8);
        cards.extend([Card::ThroneRoom, Card::Moat]);
        let mut config = GameConfig::new(rng.random_range(2..=4), cards, seed);
        config.turn_limit = Some(60);
        random_game_with_kingdom_decks(config, seed);
    }
}
