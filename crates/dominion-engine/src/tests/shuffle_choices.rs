//! "When you shuffle" choices (Star Chart, Avoid, Order of Astrologers, Order of Masons,
//! Fated): the player looks through the shuffled cards and picks among them by name. Which
//! copy of a name the engine takes must not change what comes next: the cards left are
//! shuffled again.

use super::*;
use crate::chance::{Chance, Outcomes, Site};
use crate::frame::{Frame, Input};
use std::collections::BTreeMap;

/// Every order of `cards`, one per permutation of the copies: each equally likely.
fn orders(cards: &[Card]) -> Vec<Vec<Card>> {
    if cards.is_empty() {
        return vec![vec![]];
    }
    let mut out = vec![];
    for i in 0..cards.len() {
        let mut rest = cards.to_vec();
        let first = rest.remove(i);
        for mut order in orders(&rest) {
            order.insert(0, first);
            out.push(order);
        }
    }
    out
}

/// A started game where P0's discard pile is about to be shuffled with the card under test
/// waiting for it.
fn game(card: Card) -> Game {
    let landscapes = match card {
        Card::StarChart => vec![],
        Card::Fated => vec![Card::Fated],
        other => vec![other],
    };
    let mut g = setup_with(GameConfig {
        colonies: Some(false),
        shelters: Some(false),
        landscapes,
        ..GameConfig::new(2, vec![Card::Village, Card::Smithy], 7)
    });
    match card {
        Card::StarChart => g.players[0].projects.push(Card::StarChart),
        Card::Fated => {
            let pile = g.pile_of(Card::Village).unwrap();
            g.piles[pile.index()].traits.push(Card::Fated);
        }
        Card::OrderOfAstrologers | Card::OrderOfMasons => g.players[0].favors = 1,
        _ => {}
    }
    hand(&mut g, P0, &[]);
    deck(&mut g, P0, &[]);
    g.start();
    if card == Card::Avoid {
        g.turn.coins = 2;
        g.pending = None;
        g.run(Input::Start);
        if matches!(
            g.pending().map(|d| &d.kind),
            Some(DecisionKind::Turn {
                phase: crate::game::Phase::Action,
                ..
            })
        ) {
            end_phase(&mut g);
        }
        respond(&mut g, Response::Move(Move::BuyLandscape(Card::Avoid)));
    }
    g
}

/// How often P0's deck ends as each order (top first), over every order the shuffle can
/// give `cards` and every order a shuffle of `rest` can give, when `by`'s question is
/// answered with the first (or the last) copy of `pick` it offers.
fn decks(
    by: Card,
    cards: &[Card],
    pick: Card,
    rest: &[Card],
    last: bool,
) -> BTreeMap<Vec<Card>, usize> {
    let mut out = BTreeMap::new();
    for order in orders(cards) {
        for again in orders(rest) {
            let mut g = game(by);
            discard(&mut g, P0, cards);
            let mut outcomes = Outcomes::default();
            outcomes.push(Site::Deck(P0), order.clone(), None);
            outcomes.push(Site::Deck(P0), again.clone(), None);
            g.chance = Some(Box::new(Chance {
                given: Some(outcomes),
                ..Chance::default()
            }));
            g.stack.push(Frame::shuffle(P0));
            g.pending = None;
            g.run(Input::Start);
            while g.pending().is_some_and(|d| d.purpose == Purpose::Trigger) {
                respond(&mut g, Response::option(Opt::Trigger(0)));
            }
            let offered = candidates(&g);
            let copies: Vec<CardId> = (offered.iter().copied())
                .filter(|&c| g.card(c) == pick)
                .collect();
            let c = if last {
                copies[copies.len() - 1]
            } else {
                copies[0]
            };
            choose(&mut g, &[c]);
            // Fated asks which go to the bottom too.
            while g.pending().is_some_and(|d| d.source == Some(by)) {
                choose(&mut g, &[]);
            }
            assert!(g.chance_refused().is_none(), "{by}: {order:?} {again:?}");
            *out.entry(deck_kinds(&g, P0)).or_default() += 1;
        }
    }
    out
}

#[test]
fn which_copy_a_shuffle_choice_takes_changes_nothing() {
    use Card::{Copper, Gold, Village};
    // (the card, the cards shuffled, the name picked, where it goes)
    let cases = [
        (Card::StarChart, Copper, true),
        (Card::OrderOfAstrologers, Copper, true),
        (Card::Fated, Village, true),
        (Card::Avoid, Copper, false),
        (Card::OrderOfMasons, Copper, false),
    ];
    for (by, pick, on_top) in cases {
        let cards = [pick, pick, Gold];
        let rest = [pick, Gold];
        let first = decks(by, &cards, pick, &rest, false);
        let last = decks(by, &cards, pick, &rest, true);
        // The rules: the card picked goes where it goes, the others are in random order.
        let games = orders(&cards).len() * orders(&rest).len();
        let mut want = BTreeMap::new();
        for order in [[pick, Gold], [Gold, pick]] {
            let mut deck = order.to_vec();
            if on_top {
                deck.insert(0, pick);
            }
            want.insert(deck, games / 2);
        }
        assert_eq!(first, want, "{by}, picking the first {pick}");
        assert_eq!(last, want, "{by}, picking the last {pick}");
    }
}
