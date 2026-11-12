use super::intrigue::{modes, random_games_with_copies, trigger};
use super::*;

fn discard_kinds(g: &Game, p: PlayerId) -> Vec<Card> {
    kinds(g, &g.player(p).discard)
}

/// Ends phases until the current player's turn is over.
fn finish_turn(g: &mut Game) {
    let p = g.turn().player;
    while g.turn().player == p && g.outcome().is_none() {
        match &g.pending().expect("a decision is pending").kind {
            DecisionKind::Turn { .. } => end_phase(g),
            k => panic!("unexpected decision {k:?}"),
        }
    }
}

/// Plays out P0's turn and P1's (doing nothing), so P0's next turn begins.
fn next_turn(g: &mut Game) {
    finish_turn(g);
    finish_turn(g);
}

/// A game with Black Market, whose deck is a lone Village until a test replaces it.
pub(super) fn setup_black_market() -> Game {
    setup_with(GameConfig {
        colonies: Some(false),
        shelters: Some(false),
        setup_choices: [(Card::BlackMarket, vec![Card::Village])].into(),
        ..GameConfig::new(2, vec![Card::BlackMarket], 7)
    })
}

fn black_market_deck(g: &Game) -> PileId {
    g.ext.promos.black_market.expect("Black Market deck")
}

#[test]
fn black_market_deck_is_unused_kingdom_cards_face_down() {
    let g = setup(&[Card::BlackMarket, Card::Village, Card::Sauna]);
    let bm = black_market_deck(&g);
    let pile = g.pile(bm);
    assert!(!pile.in_supply && pile.hidden);
    assert_eq!(pile.cards.len(), 60);
    let cards: Vec<Card> = pile.cards.iter().map(|&c| g.card(c)).collect();
    for (i, c) in cards.iter().enumerate() {
        assert!(c.def().kingdom && !c.def().removed, "{c}");
        assert!(!cards[..i].contains(c), "{c} twice");
        assert!(
            ![Card::BlackMarket, Card::Village, Card::Sauna, Card::Avanto].contains(c),
            "{c}"
        );
        assert!(!c.is(Types::TRAVELLER | Types::GATHERING), "{c}");
    }
    let potions = cards.iter().any(|c| c.cost().unwrap().potions > 0);
    assert_eq!(g.supply_pile(Card::Potion).is_some(), potions);
    let v = g.view(P0);
    assert_eq!((v.pile(bm).top(), v.pile(bm).size()), (None, 60));
    assert!(pile.cards.iter().all(|&c| v.card(c).is_none()));
    let mut listed: Vec<Card> = v
        .notes()
        .iter()
        .filter(|n| n.source == Card::BlackMarket && n.pile == Some(bm) && n.count == 1)
        .filter_map(|n| n.card)
        .collect();
    let mut sorted = cards.clone();
    sorted.sort_by_key(|c| c.name());
    listed.sort_by_key(|c| c.name());
    assert_eq!(listed, sorted);
    assert_eq!(
        g.supply_pile(Card::BlackMarket),
        g.pile_of(Card::BlackMarket)
    );
    let no_bm = setup(&[Card::Village]);
    assert_eq!(no_bm.ext.promos.black_market, None);
}

/// A card in the Black Market deck is in the game.
#[test]
fn a_card_in_the_black_market_deck_brings_its_heirloom() {
    let g = setup_with(GameConfig {
        colonies: Some(false),
        shelters: Some(false),
        setup_choices: [(Card::BlackMarket, vec![Card::Shepherd, Card::Village])]
            .into_iter()
            .collect(),
        ..GameConfig::new(2, vec![Card::BlackMarket], 3)
    });
    for p in [P0, P1] {
        assert_eq!(g.count_owned(p, Card::Pasture), 1);
        assert_eq!(g.count_owned(p, Card::Copper), 6);
    }
}

fn black_market_config(deck: Vec<Card>) -> GameConfig {
    GameConfig {
        colonies: Some(false),
        shelters: Some(false),
        setup_choices: [(Card::BlackMarket, deck)].into(),
        ..GameConfig::new(2, vec![Card::BlackMarket, Card::Catapult], 7)
    }
}

/// The deck may hold any card of an unused Kingdom pile, one per pile, and setup adds what
/// they use (Alchemy rulebook: the Potion pile; Knights: "put a Knight directly into the
/// Black Market deck"; Traveller, Gathering: they may be bought from it).
#[test]
fn a_configured_black_market_deck_takes_one_card_of_each_unused_kingdom_pile() {
    let deck = vec![
        Card::Familiar,
        Card::SirMartin,
        Card::SmallCastle,
        Card::Emporium,
        Card::Page,
        Card::Temple,
        Card::Vampire,
    ];
    let g = crate::setup::build(black_market_config(deck)).expect("a legal deck");
    for card in [Card::Potion, Card::TreasureHunter, Card::Bat] {
        assert!(g.pile_of(card).is_some(), "{card}");
    }
    let rejected = |deck: Vec<Card>, choice| {
        let err = crate::setup::build(black_market_config(deck)).unwrap_err();
        assert_eq!(
            err,
            SetupError::InvalidChoice {
                by: Card::BlackMarket,
                choice
            }
        );
    };
    rejected(vec![Card::SirMartin, Card::DameAnna], Card::DameAnna);
    rejected(vec![Card::Rocks], Card::Rocks);
    rejected(vec![Card::Knights], Card::Knights);
    rejected(vec![Card::RuinedVillage], Card::RuinedVillage);
}

/// A card bought from Black Market in the Buy phase (a Treasure under Capitalism) is bought
/// in that phase: no Treasures can be played after it.
#[test]
fn black_market_bought_in_the_buy_phase_ends_playing_treasures() {
    let mut g = setup_black_market();
    let bm = black_market_deck(&g);
    replace(
        &mut g,
        Zone::Pile(bm),
        &[Card::Moat, Card::Smithy, Card::Laboratory],
    );
    g.players[0].projects.push(Card::Capitalism);
    hand(&mut g, P0, &[Card::BlackMarket, Card::Gold, Card::Copper]);
    g.start();
    end_phase(&mut g);
    play(&mut g, Card::BlackMarket);
    choose_kinds(&mut g, &[Card::Gold]);
    choose(&mut g, &[]);
    choose_kinds(&mut g, &[Card::Laboratory]);
    choose_kinds(&mut g, &[Card::Smithy, Card::Moat]);
    let copper = find(&g, P0, Card::Copper);
    let DecisionKind::Turn { moves, .. } = &g.pending().unwrap().kind else {
        panic!("{:?}", g.pending());
    };
    assert!(!moves.contains(&Move::Play(copper)), "{moves:?}");
    assert!(moves.contains(&Move::EndPhase));
}

#[test]
fn debt_can_be_paid_off_before_playing_black_market() {
    let mut g = setup_black_market();
    let bm = black_market_deck(&g);
    replace(
        &mut g,
        Zone::Pile(bm),
        &[Card::Moat, Card::Smithy, Card::Village],
    );
    hand(&mut g, P0, &[Card::BlackMarket]);
    g.players[0].coffers = 1;
    g.players[0].debt = 1;
    g.start();
    respond(&mut g, Response::Move(Move::Coffers));
    respond(&mut g, Response::Move(Move::PayDebt));
    play(&mut g, Card::BlackMarket);
    choose_kinds(&mut g, &[Card::Moat]);
    assert_eq!(g.count_owned(P0, Card::Moat), 1);
}

#[test]
fn black_market_buys_in_the_action_phase_without_using_a_buy() {
    let mut g = setup_black_market();
    let bm = black_market_deck(&g);
    // Listed bottom first: Laboratory is on top.
    replace(
        &mut g,
        Zone::Pile(bm),
        &[
            Card::Moat,
            Card::Smithy,
            Card::Militia,
            Card::Gardens,
            Card::Laboratory,
        ],
    );
    hand(
        &mut g,
        P0,
        &[Card::BlackMarket, Card::Silver, Card::Copper, Card::Gold],
    );
    g.start();
    play(&mut g, Card::BlackMarket);
    assert_eq!(g.turn().coins, 2);
    assert_eq!(purpose(&g), Purpose::Play);
    choose_kinds(&mut g, &[Card::Silver]);
    choose_kinds(&mut g, &[Card::Copper]);
    choose(&mut g, &[]);
    assert_eq!(g.turn().coins, 5);
    assert_eq!(
        kinds(&g, &candidates(&g)),
        [Card::Laboratory, Card::Gardens, Card::Militia]
    );
    choose_kinds(&mut g, &[Card::Laboratory]);
    assert_eq!(discard_kinds(&g, P0), [Card::Laboratory]);
    assert!(g.log().contains(&Event::Bought {
        player: P0,
        card: Card::Laboratory
    }));
    // The other two go under the deck, in the chosen order.
    choose_kinds(&mut g, &[Card::Militia, Card::Gardens]);
    assert_eq!(
        kinds(&g, &g.pile(bm).cards),
        [Card::Gardens, Card::Militia, Card::Moat, Card::Smithy]
    );
    assert_eq!((g.turn().coins, g.turn().buys), (0, 1));
    // Treasures can still be played and a card bought in the Buy phase.
    play(&mut g, Card::Gold);
    buy(&mut g, Card::Silver);
    assert_eq!(g.count_owned(P0, Card::Silver), 2);
}

/// A player has no $ on another player's turn, and can't spend that player's.
#[test]
fn black_market_played_on_another_players_turn_buys_nothing() {
    let kingdom = vec![
        Card::BlackMarket,
        Card::Ambassador,
        Card::Gondola,
        Card::Festival,
    ];
    let deck = vec![Card::Cellar, Card::Laboratory, Card::Moat];
    let mut g = setup_with(GameConfig {
        colonies: Some(false),
        shelters: Some(false),
        setup_choices: [(Card::BlackMarket, deck)].into(),
        ..GameConfig::new(2, kingdom, 7)
    });
    hand(
        &mut g,
        P0,
        &[Card::Festival, Card::Ambassador, Card::Gondola],
    );
    hand(&mut g, P1, &[Card::BlackMarket, Card::Silver]);
    g.start();
    play(&mut g, Card::Festival);
    play_on_another_players_turn(&mut g);
    // P1 plays the Silver, then is only asked for the order of the revealed cards.
    choose_kinds(&mut g, &[Card::Silver]);
    let d = g.pending().unwrap();
    assert_eq!((d.player, d.purpose), (P1, Purpose::Move));
    assert_eq!((g.turn().player, g.turn().coins), (P0, 2));
}

#[test]
fn black_market_offers_only_affordable_cards() {
    let mut g = setup_black_market();
    let bm = black_market_deck(&g);
    replace(
        &mut g,
        Zone::Pile(bm),
        &[Card::Moat, Card::Laboratory, Card::Cellar],
    );
    hand(&mut g, P0, &[Card::BlackMarket]);
    g.start();
    play(&mut g, Card::BlackMarket);
    assert_eq!(kinds(&g, &candidates(&g)), [Card::Cellar, Card::Moat]);
    choose(&mut g, &[]);
    choose_kinds(&mut g, &[Card::Moat, Card::Laboratory, Card::Cellar]);
    assert_eq!(
        kinds(&g, &g.pile(bm).cards),
        [Card::Cellar, Card::Laboratory, Card::Moat]
    );
    assert_eq!(g.turn().coins, 2);
}

#[test]
fn black_market_lets_debt_be_paid_off_first() {
    let mut g = setup_black_market();
    let bm = black_market_deck(&g);
    replace(&mut g, Zone::Pile(bm), &[Card::Cellar]);
    hand(&mut g, P0, &[Card::BlackMarket, Card::Silver]);
    g.players[0].debt = 2;
    g.start();
    play(&mut g, Card::BlackMarket);
    choose_kinds(&mut g, &[Card::Silver]);
    assert_eq!(
        g.pending().unwrap().kind,
        DecisionKind::Number { min: 0, max: 2 }
    );
    respond(&mut g, Response::Number(2));
    choose_kinds(&mut g, &[Card::Cellar]);
    assert_eq!((g.player(P0).debt, g.turn().coins), (0, 0));
    assert_eq!(discard_kinds(&g, P0), [Card::Cellar]);
}

#[test]
fn black_market_sells_animal_fair_for_an_action_trashed_from_the_hand() {
    let mut g = setup_black_market();
    let bm = black_market_deck(&g);
    replace(&mut g, Zone::Pile(bm), &[Card::Smithy, Card::AnimalFair]);
    hand(&mut g, P0, &[Card::BlackMarket, Card::Village]);
    g.start();
    play(&mut g, Card::BlackMarket);
    // $2 pays for neither; Animal Fair can be had for the Village.
    assert_eq!(kinds(&g, &candidates(&g)), [Card::AnimalFair]);
    choose_kinds(&mut g, &[Card::AnimalFair]);
    assert_eq!(kinds(&g, g.trash()).last(), Some(&Card::Village));
    assert_eq!(discard_kinds(&g, P0), [Card::AnimalFair]);
    assert_eq!((g.turn().coins, g.turn().buys), (2, 1));
}

#[test]
fn a_prince_set_up_at_the_start_of_a_turn_plays_its_card_that_turn() {
    let mut config = GameConfig::new(2, vec![Card::Prince, Card::Village], 7);
    config.colonies = Some(false);
    config.shelters = Some(false);
    let mut g = setup_with(config);
    g.start();
    // A Prince holding a Village, set up during the start of P1's next turn (as from a
    // start-of-turn play).
    let prince = g.create(Card::Prince, Zone::InPlay(P1));
    let village = g.create(Card::Village, Zone::SetAside(P1));
    let keeps = vec![prince];
    let data = crate::game::Scratch {
        pick: Some(village),
        ..Default::default()
    };
    g.players[1].turns_taken = 5;
    g.add_lasting(Card::Prince, P1, Some(prince), keeps, Until::Removed, data);
    // Stamped with the turn P1 is about to start.
    g.players[1].turns_taken = 4;
    finish_turn(&mut g);
    assert_eq!(g.turn().player, P1);
    assert_eq!(g.turn().played, [Card::Village]);
}

#[test]
fn stash_goes_where_its_owner_chooses_when_shuffled() {
    let mut g = setup(&[Card::Stash, Card::Smithy]);
    hand(&mut g, P0, &[Card::Smithy]);
    deck(&mut g, P0, &[Card::Gold]);
    discard(
        &mut g,
        P0,
        &[Card::Copper, Card::Stash, Card::Copper, Card::Copper],
    );
    g.start();
    play(&mut g, Card::Smithy);
    assert_eq!(trigger_cards(&g), [Card::Stash]);
    trigger(&mut g, Card::Stash);
    assert_eq!(
        g.pending().unwrap().kind,
        DecisionKind::Number { min: 0, max: 3 }
    );
    // Second from the top of the shuffled cards: drawn with the Gold and a Copper.
    respond(&mut g, Response::Number(1));
    let mut h = hand_kinds(&g, P0);
    h.sort();
    let mut want = vec![Card::Gold, Card::Copper, Card::Stash];
    want.sort();
    assert_eq!(h, want);
    assert_eq!(deck_kinds(&g, P0), [Card::Copper, Card::Copper]);
}

#[test]
fn stash_can_be_left_to_the_shuffle() {
    let mut g = setup(&[Card::Stash, Card::Smithy]);
    hand(&mut g, P0, &[Card::Smithy]);
    deck(&mut g, P0, &[]);
    discard(&mut g, P0, &[Card::Stash, Card::Copper]);
    g.start();
    play(&mut g, Card::Smithy);
    no(&mut g);
    assert_eq!(g.hand(P0).len(), 2);
}

#[test]
fn prince_plays_its_card_at_the_start_of_every_turn() {
    let mut g = setup(&[Card::Prince, Card::Village, Card::MiningVillage]);
    hand(&mut g, P0, &[Card::Prince, Card::Village, Card::Gold]);
    deck(&mut g, P0, &[Card::Copper; 20]);
    g.start();
    play(&mut g, Card::Prince);
    choose_kinds(&mut g, &[Card::Village]);
    let village = g.player(P0).set_aside[0];
    assert_eq!(g.card(village), Card::Village);
    next_turn(&mut g);
    assert_eq!(g.turn().player, P0);
    assert_eq!(g.turn().played, [Card::Village]);
    assert_eq!((g.turn().actions, g.hand(P0).len()), (3, 6));
    assert_eq!(g.zone(village), Zone::SetAside(P0));
    assert_eq!(kinds(&g, g.in_play(P0)), [Card::Prince]);
    next_turn(&mut g);
    assert_eq!(g.turn().played, [Card::Village]);
}

#[test]
fn prince_only_takes_cheap_non_duration_actions() {
    let mut g = setup(&[Card::Prince, Card::Laboratory, Card::Village, Card::Bridge]);
    hand(
        &mut g,
        P0,
        &[Card::Prince, Card::Laboratory, Card::Village, Card::Prince],
    );
    g.start();
    play(&mut g, Card::Prince);
    assert_eq!(kinds(&g, &candidates(&g)), [Card::Village]);
    choose(&mut g, &[]);
    // Nothing set aside: Prince is discarded in Clean-up.
    finish_turn(&mut g);
    assert!(g.in_play(P0).is_empty());
}

#[test]
fn a_prince_played_mining_village_cannot_trash_itself() {
    let mut g = setup(&[Card::Prince, Card::MiningVillage]);
    hand(&mut g, P0, &[Card::Prince, Card::MiningVillage]);
    deck(&mut g, P0, &[Card::Copper; 20]);
    g.start();
    play(&mut g, Card::Prince);
    choose_kinds(&mut g, &[Card::MiningVillage]);
    next_turn(&mut g);
    assert_eq!(g.turn().played, [Card::MiningVillage]);
    assert_eq!(g.turn().actions, 3);
    assert_eq!(g.pending().unwrap().purpose, Purpose::Turn);
    assert_eq!(g.player(P0).set_aside.len(), 1);
}

#[test]
fn throne_room_on_prince_sets_aside_two_cards() {
    let mut g = setup(&[Card::Prince, Card::ThroneRoom, Card::Village]);
    hand(
        &mut g,
        P0,
        &[Card::ThroneRoom, Card::Prince, Card::Village, Card::Village],
    );
    deck(&mut g, P0, &[Card::Copper; 20]);
    g.start();
    play(&mut g, Card::ThroneRoom);
    choose_kinds(&mut g, &[Card::Prince]);
    choose_kinds(&mut g, &[Card::Village]);
    choose_kinds(&mut g, &[Card::Village]);
    next_turn(&mut g);
    trigger(&mut g, Card::Prince);
    assert_eq!(g.turn().played, [Card::Village, Card::Village]);
    assert_eq!(kinds(&g, g.in_play(P0)), [Card::ThroneRoom, Card::Prince]);
}

#[test]
fn captain_plays_supply_cards_now_and_next_turn() {
    let mut g = setup(&[
        Card::Captain,
        Card::Smithy,
        Card::MiningVillage,
        Card::Laboratory,
    ]);
    hand(&mut g, P0, &[Card::Captain]);
    deck(&mut g, P0, &[Card::Copper; 20]);
    g.start();
    let smithies = g.pile(g.supply_pile(Card::Smithy).unwrap()).cards.len();
    play(&mut g, Card::Captain);
    let d = g.pending().unwrap().clone();
    let DecisionKind::Pile { piles, .. } = d.kind else {
        panic!("{d:?}")
    };
    let names: Vec<Card> = piles.iter().map(|&p| g.pile(p).card).collect();
    assert_eq!(names, [Card::Smithy, Card::MiningVillage]);
    choose_pile(&mut g, Card::Smithy);
    assert_eq!(g.hand(P0).len(), 3);
    assert_eq!(
        g.pile(g.supply_pile(Card::Smithy).unwrap()).cards.len(),
        smithies
    );
    next_turn(&mut g);
    // The start-of-turn play: a Mining Village that is not in play can't trash itself.
    choose_pile(&mut g, Card::MiningVillage);
    assert_eq!(g.turn().actions, 3);
    assert_eq!(g.pending().unwrap().purpose, Purpose::Turn);
    assert_eq!(kinds(&g, g.in_play(P0)), [Card::Captain]);
    finish_turn(&mut g);
    assert!(g.in_play(P0).is_empty());
}

/// The Throne Room stays in the Supply, so nothing keeps track of the second play.
#[test]
fn a_duration_played_by_a_captains_throne_room_is_doubled_this_turn_only() {
    let mut g = setup(&[Card::Captain, Card::ThroneRoom, Card::Caravan]);
    hand(&mut g, P0, &[Card::Captain]);
    let mut cards = vec![Card::Caravan];
    cards.extend([Card::Copper; 20]);
    deck(&mut g, P0, &cards);
    g.start();
    // Throne Room is the only card Captain can play; this turn it has nothing to play.
    play(&mut g, Card::Captain);
    next_turn(&mut g);
    choose_kinds(&mut g, &[Card::Caravan]);
    finish_turn(&mut g);
    assert_eq!(kinds(&g, g.in_play(P0)), [Card::Caravan]);
    finish_turn(&mut g);
    assert_eq!((g.turn().player, g.hand(P0).len()), (P0, 6));
}

#[test]
fn church_returns_its_cards_and_offers_a_trash() {
    let mut g = setup(&[Card::Church]);
    hand(
        &mut g,
        P0,
        &[Card::Church, Card::Gold, Card::Silver, Card::Estate],
    );
    deck(&mut g, P0, &[Card::Copper; 20]);
    g.start();
    play(&mut g, Card::Church);
    choose_kinds(&mut g, &[Card::Gold, Card::Estate]);
    assert_eq!(g.view(P1).card(g.player(P0).set_aside[0]), None);
    next_turn(&mut g);
    assert_eq!(purpose(&g), Purpose::Trash);
    assert_eq!(g.hand(P0).len(), 7);
    choose_kinds(&mut g, &[Card::Estate]);
    assert_eq!(kinds(&g, g.trash()).last(), Some(&Card::Estate));
    assert!(hand_kinds(&g, P0).contains(&Card::Gold));
    finish_turn(&mut g);
    assert!(g.in_play(P0).is_empty());
}

#[test]
fn dismantle_gains_a_cheaper_card_and_a_gold() {
    let mut g = setup(&[Card::Dismantle, Card::Village]);
    hand(&mut g, P0, &[Card::Dismantle, Card::Silver]);
    g.start();
    play(&mut g, Card::Dismantle);
    choose_pile(&mut g, Card::Estate);
    assert_eq!(discard_kinds(&g, P0), [Card::Estate, Card::Gold]);

    let mut g = setup(&[Card::Dismantle]);
    hand(&mut g, P0, &[Card::Dismantle, Card::Copper]);
    g.start();
    play(&mut g, Card::Dismantle);
    assert_eq!(kinds(&g, g.trash()).last(), Some(&Card::Copper));
    assert!(g.player(P0).discard.is_empty());
}

#[test]
fn envoy_lets_the_left_player_pick_the_discard() {
    let mut g = setup(&[Card::Envoy]);
    hand(&mut g, P0, &[Card::Envoy]);
    deck(
        &mut g,
        P0,
        &[
            Card::Gold,
            Card::Silver,
            Card::Copper,
            Card::Estate,
            Card::Gold,
            Card::Duchy,
        ],
    );
    g.start();
    play(&mut g, Card::Envoy);
    let d = g.pending().unwrap();
    assert_eq!((d.player, d.purpose), (P1, Purpose::Discard));
    choose_kinds(&mut g, &[Card::Gold]);
    assert_eq!(discard_kinds(&g, P0), [Card::Gold]);
    assert_eq!(g.hand(P0).len(), 4);
    assert_eq!(deck_kinds(&g, P0), [Card::Duchy]);
}

#[test]
fn governor_modes_go_around_the_table() {
    let mut g = setup(&[Card::Governor]);
    hand(&mut g, P0, &[Card::Governor]);
    deck(&mut g, P0, &[Card::Copper; 5]);
    let p1_hand = g.hand(P1).len();
    g.start();
    play(&mut g, Card::Governor);
    modes(&mut g, &[0]);
    assert_eq!((g.hand(P0).len(), g.hand(P1).len()), (3, p1_hand + 1));
    assert_eq!(g.turn().actions, 1);

    let mut g = setup(&[Card::Governor]);
    hand(&mut g, P0, &[Card::Governor]);
    g.start();
    play(&mut g, Card::Governor);
    modes(&mut g, &[1]);
    assert_eq!(discard_kinds(&g, P0), [Card::Gold]);
    assert_eq!(discard_kinds(&g, P1), [Card::Silver]);

    let mut g = setup(&[Card::Governor, Card::Village]);
    hand(&mut g, P0, &[Card::Governor, Card::Silver]);
    hand(&mut g, P1, &[Card::Estate, Card::Copper]);
    g.start();
    play(&mut g, Card::Governor);
    modes(&mut g, &[2]);
    choose_kinds(&mut g, &[Card::Silver]);
    choose_pile(&mut g, Card::Duchy);
    assert_eq!(discard_kinds(&g, P0), [Card::Duchy]);
    assert_eq!(g.pending().unwrap().player, P1);
    choose_kinds(&mut g, &[Card::Estate]);
    choose_pile(&mut g, Card::Village);
    assert_eq!(discard_kinds(&g, P1), [Card::Village]);
}

#[test]
fn governor_trash_is_optional() {
    let mut g = setup(&[Card::Governor]);
    hand(&mut g, P0, &[Card::Governor, Card::Copper]);
    hand(&mut g, P1, &[Card::Copper]);
    g.start();
    play(&mut g, Card::Governor);
    modes(&mut g, &[2]);
    choose(&mut g, &[]);
    choose(&mut g, &[]);
    assert_eq!(g.hand(P0).len(), 1);
    assert_eq!(g.hand(P1).len(), 1);
}

#[test]
fn marchland_scores_per_three_victory_cards() {
    let mut g = setup(&[Card::Marchland, Card::Nobles]);
    g.start();
    for card in [Card::Marchland, Card::Marchland, Card::Nobles, Card::Duchy] {
        g.create(card, Zone::Discard(P0));
    }
    // 7 Victory cards: 2 VP per Marchland.
    assert_eq!(g.score(P0), 3 + 2 * 2 + 2 + 3);
    assert_eq!(
        g.pile(g.supply_pile(Card::Marchland).unwrap()).cards.len(),
        8
    );
}

#[test]
fn gaining_marchland_gives_a_buy_and_coins_for_discards() {
    let mut g = setup(&[Card::Marchland]);
    hand(
        &mut g,
        P0,
        &[Card::Gold, Card::Silver, Card::Estate, Card::Estate],
    );
    g.start();
    play(&mut g, Card::Gold);
    play(&mut g, Card::Silver);
    buy(&mut g, Card::Marchland);
    assert_eq!(purpose(&g), Purpose::Discard);
    choose_kinds(&mut g, &[Card::Estate, Card::Estate]);
    assert_eq!((g.turn().coins, g.turn().buys), (2, 1));
    buy(&mut g, Card::Estate);
    assert_eq!(g.count_owned(P0, Card::Marchland), 1);
}

#[test]
fn sauna_avanto_is_a_split_pile() {
    let g = setup(&[Card::Sauna]);
    let pile = g.pile(g.supply_pile(Card::Sauna).unwrap());
    let cards: Vec<Card> = pile.cards.iter().rev().map(|&c| g.card(c)).collect();
    let mut want = vec![Card::Sauna; 5];
    want.extend([Card::Avanto; 5]);
    assert_eq!(cards, want);
    assert_eq!(g.home_pile(Card::Avanto), g.supply_pile(Card::Sauna));
}

#[test]
fn sauna_plays_avanto_and_trashes_on_silver() {
    let mut g = setup(&[Card::Sauna]);
    hand(
        &mut g,
        P0,
        &[
            Card::Sauna,
            Card::Avanto,
            Card::Silver,
            Card::Silver,
            Card::Estate,
        ],
    );
    deck(&mut g, P0, &[Card::Copper; 6]);
    g.start();
    play(&mut g, Card::Sauna);
    choose_kinds(&mut g, &[Card::Avanto]);
    assert_eq!(g.hand(P0).len(), 3 + 1 + 3);
    assert_eq!(g.turn().actions, 1);
    play(&mut g, Card::Silver);
    trigger(&mut g, Card::Sauna);
    choose_kinds(&mut g, &[Card::Estate]);
    assert_eq!(kinds(&g, g.trash()).last(), Some(&Card::Estate));
    play(&mut g, Card::Silver);
    no(&mut g);
    assert_eq!(g.turn().coins, 4);
}

#[test]
fn avanto_may_play_a_sauna() {
    let mut g = setup(&[Card::Sauna]);
    hand(&mut g, P0, &[Card::Avanto, Card::Sauna]);
    deck(&mut g, P0, &[Card::Copper; 6]);
    g.start();
    play(&mut g, Card::Avanto);
    choose_kinds(&mut g, &[Card::Sauna]);
    assert_eq!(g.hand(P0).len(), 4);
    assert_eq!(g.turn().actions, 1);
}

#[test]
fn walled_village_goes_on_the_deck_with_at_most_one_other_action() {
    let mut g = setup(&[Card::WalledVillage, Card::Smithy]);
    hand(&mut g, P0, &[Card::WalledVillage, Card::Smithy]);
    deck(&mut g, P0, &[Card::Copper; 20]);
    g.start();
    play(&mut g, Card::WalledVillage);
    play(&mut g, Card::Smithy);
    end_phase(&mut g);
    trigger(&mut g, Card::WalledVillage);
    assert_eq!(deck_kinds(&g, P0)[..1], [Card::Copper]);
    assert!(hand_kinds(&g, P0).contains(&Card::WalledVillage));

    let mut g = setup(&[Card::WalledVillage, Card::Village]);
    hand(
        &mut g,
        P0,
        &[Card::WalledVillage, Card::Village, Card::Village],
    );
    deck(&mut g, P0, &[Card::Copper; 20]);
    g.start();
    play(&mut g, Card::WalledVillage);
    play(&mut g, Card::Village);
    play(&mut g, Card::Village);
    finish_turn(&mut g);
    assert_eq!(g.count_owned(P0, Card::WalledVillage), 1);
    assert!(!hand_kinds(&g, P0).contains(&Card::WalledVillage));
}

#[test]
fn two_walled_villages_alone_can_both_go_on_the_deck() {
    let mut g = setup(&[Card::WalledVillage]);
    hand(&mut g, P0, &[Card::WalledVillage, Card::WalledVillage]);
    deck(&mut g, P0, &[Card::Copper; 20]);
    g.start();
    play(&mut g, Card::WalledVillage);
    play(&mut g, Card::WalledVillage);
    end_phase(&mut g);
    trigger(&mut g, Card::WalledVillage);
    trigger(&mut g, Card::WalledVillage);
    assert_eq!(
        hand_kinds(&g, P0)
            .iter()
            .filter(|&&c| c == Card::WalledVillage)
            .count(),
        2
    );
}

/// A game with Summon in which P0 has played `treasures`.
fn summon_game(kingdom: &[Card], treasures: &[Card]) -> Game {
    let mut g = setup_with(GameConfig {
        colonies: Some(false),
        shelters: Some(false),
        landscapes: vec![Card::Summon],
        ..GameConfig::new(2, kingdom.to_vec(), 7)
    });
    hand(&mut g, P0, treasures);
    deck(&mut g, P0, &[Card::Copper; 20]);
    g.start();
    for &card in treasures {
        play(&mut g, card);
    }
    g
}

#[test]
fn summon_sets_the_gained_card_aside_and_plays_it_next_turn() {
    let mut g = summon_game(
        &[Card::Smithy, Card::Laboratory],
        &[Card::Gold, Card::Silver],
    );
    // Smithy is the only Action costing up to $4, so it is gained without asking.
    respond(&mut g, Response::Move(Move::BuyLandscape(Card::Summon)));
    assert_eq!(kinds(&g, &g.player(P0).set_aside), [Card::Smithy]);
    assert_eq!(g.turn().player, P1);
    finish_turn(&mut g);
    assert_eq!(g.turn().played, [Card::Smithy]);
    assert_eq!(g.hand(P0).len(), 8);
    assert_eq!(g.turn().actions, 1);
}

/// Summon finds its card wherever it was gained to.
#[test]
fn summon_sets_aside_a_nomad_camp_gained_onto_the_deck() {
    let mut g = summon_game(&[Card::NomadCamp], &[Card::Gold, Card::Silver]);
    respond(&mut g, Response::Move(Move::BuyLandscape(Card::Summon)));
    assert_eq!(kinds(&g, &g.player(P0).set_aside), [Card::NomadCamp]);
    finish_turn(&mut g);
    assert_eq!(g.turn().played, [Card::NomadCamp]);
}

/// A card moved from where it was gained to is lost track of, even onto the deck.
#[test]
fn summon_loses_track_of_a_card_royal_seal_put_onto_the_deck() {
    let treasures = [Card::Gold, Card::Silver, Card::RoyalSeal];
    let mut g = summon_game(&[Card::Smithy, Card::RoyalSeal], &treasures);
    respond(&mut g, Response::Move(Move::BuyLandscape(Card::Summon)));
    trigger(&mut g, Card::RoyalSeal);
    assert_eq!(g.player(P0).set_aside, []);
    // P0's turn is over: the Smithy was drawn from the deck.
    assert!(hand_kinds(&g, P0).contains(&Card::Smithy));
    finish_turn(&mut g);
    assert_eq!(g.turn().played, []);
}

/// Random games over the Promos, Summon in every other one.
#[test]
fn random_promo_games_keep_invariants() {
    let pool: Vec<Card> = Card::in_set(Set::Promos)
        .filter(|c| c.def().kingdom)
        .collect();
    random_games_with_copies(&pool, &[Card::Avanto], 0..fuzz_seeds(80), |seed| {
        if seed % 2 == 0 {
            vec![Card::Summon]
        } else {
            vec![]
        }
    });
}

#[test]
fn throne_room_on_captain_plays_twice_now_and_twice_next_turn() {
    let mut g = setup(&[Card::Captain, Card::ThroneRoom, Card::Village, Card::Smithy]);
    hand(&mut g, P0, &[Card::ThroneRoom, Card::Captain]);
    deck(&mut g, P0, &[Card::Copper; 30]);
    g.start();
    play(&mut g, Card::ThroneRoom);
    choose_kinds(&mut g, &[Card::Captain]);
    choose_pile(&mut g, Card::Village);
    choose_pile(&mut g, Card::Village);
    assert_eq!(g.turn().actions, 4);
    next_turn(&mut g);
    assert_eq!(kinds(&g, g.in_play(P0)), [Card::ThroneRoom, Card::Captain]);
    choose_pile(&mut g, Card::Smithy);
    choose_pile(&mut g, Card::Smithy);
    assert_eq!(g.hand(P0).len(), 11);
    finish_turn(&mut g);
    assert!(g.in_play(P0).is_empty());
}

#[test]
fn throne_roomed_church_returns_two_batches_each_with_a_trash() {
    let mut g = setup(&[Card::Church, Card::ThroneRoom]);
    hand(
        &mut g,
        P0,
        &[Card::ThroneRoom, Card::Church, Card::Gold, Card::Curse],
    );
    deck(&mut g, P0, &[Card::Copper; 30]);
    g.start();
    play(&mut g, Card::ThroneRoom);
    choose_kinds(&mut g, &[Card::Church]);
    choose_kinds(&mut g, &[Card::Gold]);
    choose_kinds(&mut g, &[Card::Curse]);
    next_turn(&mut g);
    trigger(&mut g, Card::Church);
    assert_eq!(g.hand(P0).len(), 6);
    choose(&mut g, &[]);
    assert_eq!(g.hand(P0).len(), 7);
    choose_kinds(&mut g, &[Card::Curse]);
    assert_eq!(kinds(&g, g.trash()).last(), Some(&Card::Curse));
}
