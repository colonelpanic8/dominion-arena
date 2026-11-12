//! Elder: the card it plays may choose an extra option of each "choose", resolved in
//! printed order; other choices are unaffected.

use super::*;

fn game(kingdom: &[Card]) -> Game {
    let mut k = vec![Card::Townsfolk];
    k.extend_from_slice(kingdom);
    setup(&k)
}

fn add_hand(g: &mut Game, cards: &[Card]) {
    for &c in cards {
        g.create(c, Zone::Hand(P0));
    }
}

/// The pending `Choose` decision's purpose and maximum picks.
fn choice(g: &Game) -> (Purpose, usize) {
    let d = g.pending().expect("a decision is pending");
    match &d.kind {
        DecisionKind::Choose { max, .. } => (d.purpose, *max),
        k => panic!("expected a choice, got {k:?}"),
    }
}

fn modes(g: &mut Game, chosen: &[u8]) {
    respond(
        g,
        Response::Options(chosen.iter().map(|&i| Opt::Mode(i)).collect()),
    );
}

/// Answers the pending "choose" with `chosen`, checking that Elder allows one more pick
/// than the card's own `max`.
fn extra(g: &mut Game, max: usize, chosen: &[u8]) {
    assert_eq!(choice(g), (Purpose::Mode, max + 1));
    modes(g, chosen);
}

/// Starts the game, plays Elder and has it play `card`.
fn elder_plays(g: &mut Game, card: Card) {
    g.start();
    play(g, Card::Elder);
    choose_kinds(g, &[card]);
}

fn to_buy(g: &mut Game) {
    if let Some(DecisionKind::Turn {
        phase: Phase::Action,
        ..
    }) = g.pending().map(|d| &d.kind)
    {
        end_phase(g);
    }
}

/// Plays Treasure `card` as Elder would (an Action under Enlightenment, in the Buy phase).
fn elder_plays_treasure(g: &mut Game, card: Card) {
    let data = Scratch {
        pick: Some(find(g, P0, card)),
        i: g.turn().played_ids.len(),
        ..Scratch::default()
    };
    g.add_lasting(Card::Elder, P0, None, vec![], Until::Removed, data);
    play(g, card);
}

fn discard_kinds(g: &Game, p: PlayerId) -> Vec<Card> {
    kinds(g, &g.player(p).discard)
}

fn has(cards: &[Card], card: Card) -> bool {
    cards.contains(&card)
}

type Case = (Card, &'static [Card], fn(&mut Game));

/// Each card Elder affects, the rest of its kingdom, and a script run on an unstarted game
/// where P0 holds Elder and the card over a deck of 10 Coppers.
const CASES: &[Case] = &[
    (Card::Courtier, &[], |g| {
        add_hand(g, &[Card::Copper]);
        elder_plays(g, Card::Courtier);
        extra(g, 1, &[1, 2]);
        assert_eq!((g.turn().buys, g.turn().coins), (2, 5));
    }),
    (Card::Lurker, &[], |g| {
        elder_plays(g, Card::Lurker);
        extra(g, 1, &[0, 1]);
        choose_pile(g, Card::Lurker);
        assert!(has(&discard_kinds(g, P0), Card::Lurker));
    }),
    (Card::Minion, &[], |g| {
        add_hand(g, &[Card::Estate, Card::Estate]);
        elder_plays(g, Card::Minion);
        extra(g, 1, &[0, 1]);
        assert_eq!(g.turn().coins, 4);
        assert_eq!(hand_kinds(g, P0), [Card::Copper; 4]);
    }),
    (Card::Nobles, &[], |g| {
        elder_plays(g, Card::Nobles);
        extra(g, 1, &[0, 1]);
        assert_eq!((g.hand(P0).len(), g.turn().actions), (3, 2));
    }),
    (Card::Pawn, &[], |g| {
        elder_plays(g, Card::Pawn);
        extra(g, 2, &[0, 1, 2]);
        assert_eq!(g.hand(P0).len(), 1);
        assert_eq!((g.turn().actions, g.turn().buys, g.turn().coins), (1, 2, 2));
    }),
    (Card::Steward, &[], |g| {
        elder_plays(g, Card::Steward);
        // +2 Cards first, then trash the two drawn.
        extra(g, 1, &[0, 2]);
        assert!(g.hand(P0).is_empty());
        assert_eq!(g.turn().coins, 2);
    }),
    (Card::NativeVillage, &[], |g| {
        deck(g, P0, &[Card::Gold, Card::Copper]);
        elder_plays(g, Card::NativeVillage);
        extra(g, 1, &[0, 1]);
        assert_eq!(hand_kinds(g, P0), [Card::Gold]);
    }),
    (Card::PirateShip, &[], |g| {
        deck(g, P1, &[Card::Silver, Card::Copper]);
        elder_plays(g, Card::PirateShip);
        extra(g, 1, &[0, 1]);
        choose_kinds(g, &[Card::Silver]);
        assert_eq!(g.player(P0).pirate_ship, 1);
        assert_eq!(g.turn().coins, 2);
    }),
    (Card::Courser, &[Card::Joust], |g| {
        elder_plays(g, Card::Courser);
        extra(g, 2, &[0, 1, 2]);
        assert_eq!(g.hand(P0).len(), 2);
        assert_eq!((g.turn().actions, g.turn().coins), (2, 4));
    }),
    (Card::TrustySteed, &[Card::Tournament], |g| {
        elder_plays(g, Card::TrustySteed);
        extra(g, 2, &[1, 2, 3]);
        assert_eq!((g.turn().actions, g.turn().coins), (2, 4));
        assert!(g.player(P0).deck.is_empty());
        let silvers = discard_kinds(g, P0)
            .iter()
            .filter(|&&c| c == Card::Silver)
            .count();
        assert_eq!(silvers, 4);
    }),
    (Card::SpiceMerchant, &[], |g| {
        add_hand(g, &[Card::Copper]);
        elder_plays(g, Card::SpiceMerchant);
        choose_kinds(g, &[Card::Copper]);
        extra(g, 1, &[0, 1]);
        assert_eq!(g.hand(P0).len(), 2);
        assert_eq!((g.turn().actions, g.turn().buys, g.turn().coins), (1, 2, 4));
    }),
    (Card::Catacombs, &[], |g| {
        deck(
            g,
            P0,
            &[
                Card::Gold,
                Card::Silver,
                Card::Copper,
                Card::Estate,
                Card::Estate,
                Card::Estate,
            ],
        );
        elder_plays(g, Card::Catacombs);
        // Into the hand, then discarded from it, then +3 Cards.
        extra(g, 1, &[0, 1]);
        assert_eq!(hand_kinds(g, P0), [Card::Estate; 3]);
        assert_eq!(
            discard_kinds(g, P0),
            [Card::Gold, Card::Silver, Card::Copper]
        );
    }),
    (Card::Count, &[], |g| {
        add_hand(g, &[Card::Estate, Card::Estate, Card::Province]);
        elder_plays(g, Card::Count);
        extra(g, 1, &[0, 2]);
        choose_kinds(g, &[Card::Estate, Card::Estate]);
        extra(g, 1, &[0, 2]);
        assert_eq!(g.turn().coins, 5);
        assert_eq!(hand_kinds(g, P0), [Card::Province]);
        assert_eq!(
            discard_kinds(g, P0),
            [Card::Estate, Card::Estate, Card::Copper, Card::Duchy]
        );
    }),
    (Card::Graverobber, &[], |g| {
        add_hand(g, &[Card::Village]);
        g.create(Card::Silver, Zone::Trash);
        elder_plays(g, Card::Graverobber);
        extra(g, 1, &[0, 1]);
        choose_pile(g, Card::Gold);
        assert_eq!(deck_kinds(g, P0)[0], Card::Silver);
        assert_eq!(discard_kinds(g, P0), [Card::Gold]);
    }),
    (Card::Squire, &[], |g| {
        elder_plays(g, Card::Squire);
        extra(g, 1, &[1, 2]);
        assert_eq!(g.turn().buys, 3);
        assert_eq!(discard_kinds(g, P0), [Card::Silver]);
    }),
    (Card::Amulet, &[], |g| {
        elder_plays(g, Card::Amulet);
        extra(g, 1, &[0, 2]);
        assert_eq!(g.turn().coins, 3);
        assert_eq!(discard_kinds(g, P0), [Card::Silver]);
    }),
    (Card::Miser, &[], |g| {
        add_hand(g, &[Card::Copper]);
        elder_plays(g, Card::Miser);
        extra(g, 1, &[0, 1]);
        assert_eq!(g.turn().coins, 3);
    }),
    (Card::WildHunt, &[], |g| {
        elder_plays(g, Card::WildHunt);
        extra(g, 1, &[0, 1]);
        assert_eq!(g.hand(P0).len(), 3);
        assert_eq!(g.player(P0).vp_tokens, 1);
    }),
    (Card::Treasurer, &[], |g| {
        add_hand(g, &[Card::Gold]);
        elder_plays(g, Card::Treasurer);
        // Trash the Gold, then gain it back.
        extra(g, 1, &[0, 1]);
        choose_kinds(g, &[Card::Gold]);
        assert_eq!(hand_kinds(g, P0), [Card::Gold]);
    }),
    (Card::Scrap, &[], |g| {
        add_hand(g, &[Card::Silver]);
        elder_plays(g, Card::Scrap);
        extra(g, 3, &[0, 1, 2, 3]);
        assert_eq!(g.hand(P0).len(), 1);
        assert_eq!((g.turn().actions, g.turn().buys, g.turn().coins), (1, 2, 3));
    }),
    (Card::Blacksmith, &[], |g| {
        elder_plays(g, Card::Blacksmith);
        extra(g, 1, &[1, 2]);
        assert_eq!((g.hand(P0).len(), g.turn().actions), (3, 1));
    }),
    (Card::Broker, &[], |g| {
        add_hand(g, &[Card::Silver]);
        elder_plays(g, Card::Broker);
        extra(g, 1, &[1, 2]);
        assert_eq!((g.turn().actions, g.turn().coins), (3, 5));
    }),
    (Card::HillFort, &[Card::Forts], |g| {
        elder_plays(g, Card::HillFort);
        choose_pile(g, Card::Silver);
        extra(g, 1, &[0, 1]);
        assert_eq!(hand_kinds(g, P0), [Card::Silver, Card::Copper]);
        assert_eq!(g.turn().actions, 1);
    }),
    (Card::Innkeeper, &[], |g| {
        elder_plays(g, Card::Innkeeper);
        // All Coppers: the discard is forced.
        extra(g, 1, &[0, 1]);
        assert_eq!(g.hand(P0).len(), 1);
    }),
    (Card::Modify, &[], |g| {
        add_hand(g, &[Card::Silver]);
        elder_plays(g, Card::Modify);
        extra(g, 1, &[0, 1]);
        choose_pile(g, Card::Duchy);
        assert_eq!((g.hand(P0).len(), g.turn().actions), (1, 1));
        assert_eq!(discard_kinds(g, P0), [Card::Duchy]);
    }),
    (Card::Specialist, &[Card::Village], |g| {
        add_hand(g, &[Card::Village]);
        elder_plays(g, Card::Specialist);
        choose_kinds(g, &[Card::Village]);
        extra(g, 1, &[0, 1]);
        assert_eq!(
            g.turn().played,
            [Card::Elder, Card::Specialist, Card::Village, Card::Village]
        );
        assert_eq!(discard_kinds(g, P0), [Card::Village]);
    }),
    (Card::Stronghold, &[Card::Forts], |g| {
        elder_plays(g, Card::Stronghold);
        extra(g, 1, &[0, 1]);
        assert_eq!(g.turn().coins, 5);
    }),
    (Card::Town, &[], |g| {
        elder_plays(g, Card::Town);
        extra(g, 1, &[0, 1]);
        assert_eq!((g.turn().actions, g.turn().buys, g.turn().coins), (2, 2, 4));
    }),
    (Card::TownCrier, &[], |g| {
        elder_plays(g, Card::TownCrier);
        extra(g, 1, &[0, 1]);
        no(g);
        assert_eq!(g.turn().coins, 4);
        assert_eq!(discard_kinds(g, P0), [Card::Silver]);
    }),
    (Card::Kitsune, &[], |g| {
        elder_plays(g, Card::Kitsune);
        extra(g, 2, &[0, 1, 3]);
        assert_eq!((g.turn().actions, g.turn().coins), (2, 4));
        assert_eq!(discard_kinds(g, P0), [Card::Silver]);
    }),
    (Card::Governor, &[], |g| {
        elder_plays(g, Card::Governor);
        extra(g, 1, &[0, 1]);
        assert_eq!((g.hand(P0).len(), g.hand(P1).len()), (3, 6));
        assert_eq!(discard_kinds(g, P0), [Card::Gold]);
        assert_eq!(discard_kinds(g, P1), [Card::Silver]);
    }),
    (Card::Bauble, &[], |g| {
        hand(g, P0, &[Card::Bauble]);
        g.start();
        to_buy(g);
        let favors = g.player(P0).favors;
        elder_plays_treasure(g, Card::Bauble);
        extra(g, 2, &[0, 1, 2]);
        assert_eq!((g.turn().buys, g.turn().coins), (2, 1));
        assert_eq!(g.player(P0).favors, favors + 1);
    }),
    (Card::Charm, &[], |g| {
        hand(g, P0, &[Card::Charm]);
        g.start();
        to_buy(g);
        elder_plays_treasure(g, Card::Charm);
        extra(g, 1, &[0, 1]);
        assert_eq!((g.turn().buys, g.turn().coins), (2, 2));
        assert!(g.lasting().iter().any(|l| l.card == Card::Charm));
    }),
    (Card::Investment, &[], |g| {
        hand(g, P0, &[Card::Investment, Card::Copper, Card::Silver]);
        g.start();
        to_buy(g);
        elder_plays_treasure(g, Card::Investment);
        choose_kinds(g, &[Card::Copper]);
        extra(g, 1, &[0, 1]);
        assert_eq!(g.turn().coins, 1);
        assert_eq!(g.player(P0).vp_tokens, 1);
    }),
    (Card::Scepter, &[], |g| {
        hand(g, P0, &[Card::Village, Card::Scepter]);
        g.start();
        play(g, Card::Village);
        to_buy(g);
        elder_plays_treasure(g, Card::Scepter);
        extra(g, 1, &[0, 1]);
        assert_eq!(g.turn().coins, 2);
        assert_eq!(g.turn().actions, 4);
    }),
    (Card::Orb, &[], |g| {
        hand(g, P0, &[Card::Orb]);
        discard(g, P0, &[Card::Silver]);
        g.start();
        to_buy(g);
        elder_plays_treasure(g, Card::Orb);
        extra(g, 1, &[0, 1]);
        assert_eq!((g.turn().buys, g.turn().coins), (2, 5));
    }),
];

#[test]
fn elder_adds_an_option_to_every_choose_card() {
    for &(card, others, script) in CASES {
        println!("{card}");
        let mut kingdom = others.to_vec();
        if card.def().kingdom {
            kingdom.push(card);
        }
        let mut g = game(&kingdom);
        hand(&mut g, P0, &[Card::Elder, card]);
        deck(&mut g, P0, &[Card::Copper; 10]);
        script(&mut g);
        check_zones(&g);
    }
}

#[test]
fn elder_on_scrap_trashing_a_zero_cost_card_gives_one_choice() {
    let mut g = game(&[Card::Scrap]);
    hand(&mut g, P0, &[Card::Elder, Card::Scrap, Card::Copper]);
    elder_plays(&mut g, Card::Scrap);
    extra(&mut g, 0, &[3]);
    assert_eq!(g.turn().coins, 3);
}

#[test]
fn elder_does_not_widen_choices_that_are_not_a_choose() {
    let cases: &[(Card, &[Card])] = &[
        (Card::Navigator, &[]),
        (Card::Weaver, &[]),
        (Card::Barge, &[]),
        (Card::VillageGreen, &[]),
    ];
    for &(card, extra_hand) in cases {
        let mut g = game(&[card]);
        hand(&mut g, P0, &[Card::Elder, card]);
        add_hand(&mut g, extra_hand);
        deck(&mut g, P0, &[Card::Copper; 10]);
        elder_plays(&mut g, card);
        assert_eq!(choice(&g), (Purpose::Either, 1), "{card}");
    }
    let mut g = game(&[Card::Gondola]);
    hand(&mut g, P0, &[Card::Gondola]);
    g.start();
    to_buy(&mut g);
    elder_plays_treasure(&mut g, Card::Gondola);
    assert_eq!(choice(&g), (Purpose::Either, 1));
}

#[test]
fn elder_does_not_widen_another_players_choice() {
    let mut g = game(&[Card::Torturer]);
    hand(&mut g, P0, &[Card::Elder, Card::Torturer]);
    deck(&mut g, P0, &[Card::Copper; 10]);
    elder_plays(&mut g, Card::Torturer);
    assert_eq!(g.pending().unwrap().player, P1);
    assert_eq!(choice(&g), (Purpose::Either, 1));
}

#[test]
fn elder_gives_no_extra_option_to_a_royal_carriage_replay() {
    let mut g = game(&[Card::Pawn, Card::RoyalCarriage]);
    g.create(Card::RoyalCarriage, Zone::Mat(P0, Mat::Tavern));
    hand(&mut g, P0, &[Card::Elder, Card::Pawn]);
    deck(&mut g, P0, &[Card::Copper; 10]);
    elder_plays(&mut g, Card::Pawn);
    extra(&mut g, 2, &[1, 2, 3]);
    let d = g.pending().expect("Royal Carriage is offered");
    let DecisionKind::Choose {
        options, labels, ..
    } = &d.kind
    else {
        panic!("expected the trigger choice, got {d:?}");
    };
    let i = labels
        .iter()
        .position(|l| l.starts_with("Royal Carriage"))
        .expect("Royal Carriage is offered");
    let opt = options[i];
    respond(&mut g, Response::option(opt));
    assert_eq!(choice(&g), (Purpose::Mode, 2));
}

/// Elder's own play keeps its extra option once the card was replayed in the middle of
/// it, and the replay gets none: "you won't get an additional choice on the second play"
/// (wiki, Elder); "Scepter can replay an Action that isn't finished resolving" (wiki,
/// Scepter). Specialist plays Scepter, which replays the Specialist.
#[test]
fn elder_keeps_its_extra_option_after_a_replay_inside_its_play() {
    let mut g = game(&[Card::Specialist, Card::Scepter, Card::Village]);
    hand(
        &mut g,
        P0,
        &[Card::Elder, Card::Specialist, Card::Scepter, Card::Village],
    );
    deck(&mut g, P0, &[Card::Estate; 10]);
    elder_plays(&mut g, Card::Specialist);
    choose_kinds(&mut g, &[Card::Scepter]);
    // Scepter replays the Specialist, which plays the Village and gains a copy of it.
    modes(&mut g, &[1]);
    choose_kinds(&mut g, &[Card::Specialist]);
    choose_kinds(&mut g, &[Card::Village]);
    assert_eq!(choice(&g), (Purpose::Mode, 1));
    modes(&mut g, &[1]);
    // Back in Elder's play: play the Scepter again, gain a copy of it, or both.
    assert_eq!(g.pending().unwrap().source, Some(Card::Specialist));
    extra(&mut g, 1, &[0, 1]);
}
