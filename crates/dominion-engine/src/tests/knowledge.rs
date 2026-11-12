use super::prosperity::{next_turn, play_from};
use super::*;
use crate::game::{Mat, Zone};
use std::collections::BTreeMap;
use std::fmt::Write;

fn start(mut g: Game) -> (Game, Knowledge) {
    g.enable_trace();
    g.start();
    let mut k = Knowledge::new(g.num_players());
    k.update(&mut g);
    (g, k)
}

fn count(cards: &[(Card, u32)]) -> CardCount {
    cards.iter().copied().collect()
}

#[test]
fn discarding_identical_revealed_cards_with_envoy_gives_the_same_log() {
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
        ],
    );
    let (mut g, mut k) = start(g);
    play(&mut g, Card::Envoy);
    k.update(&mut g);
    let golds: Vec<_> = candidates(&g)
        .into_iter()
        .filter(|&c| g.card(c) == Card::Gold)
        .collect();
    let [(a, ka), (b, kb)] = worlds(&g, &k, golds[0], golds[1]);
    assert_eq!(a.log(), b.log());
    assert_eq!(ka.known(&a, P0), kb.known(&b, P0));
}

#[test]
fn revealing_identical_hand_cards_preserves_the_same_knowledge_after_cleanup() {
    let mut g = setup(&[Card::Encampment]);
    hand(&mut g, P0, &[Card::Encampment, Card::Plunder, Card::Copper]);
    deck(
        &mut g,
        P0,
        &[
            Card::Silver,
            Card::Plunder,
            Card::Gold,
            Card::Estate,
            Card::Copper,
            Card::Duchy,
            Card::Silver,
            Card::Copper,
        ],
    );
    let (mut g, mut k) = start(g);
    let gold = g.create(Card::Gold, Zone::Hand(P0));
    g.show(gold);
    k.update(&mut g);
    play(&mut g, Card::Encampment);
    k.update(&mut g);
    let copies: Vec<_> = candidates(&g)
        .into_iter()
        .filter(|&c| g.card(c) == Card::Plunder)
        .collect();

    let world = |copy| {
        let (mut g, mut k) = (g.clone(), k.clone());
        choose(&mut g, &[copy]);
        k.update(&mut g);
        next_turn(&mut g);
        k.update(&mut g);
        (g, k)
    };
    let ((a, ka), (b, kb)) = (world(copies[0]), world(copies[1]));
    assert_eq!(observation(&a, P1, 0), observation(&b, P1, 0));
    assert_eq!(ka.known(&a, P1), kb.known(&b, P1));
    assert_eq!(
        ka.known(&a, P1)
            .zone(Zone::Discard(P0))
            .known
            .get(&Card::Plunder),
        Some(&1)
    );
    assert_eq!(
        ka.known(&a, P1)
            .zone(Zone::Discard(P0))
            .known
            .get(&Card::Gold),
        Some(&1)
    );
}

#[test]
fn privately_ordering_discards_does_not_reveal_which_cards_were_selected() {
    let mut g = setup(&[Card::Herald]);
    hand(&mut g, P0, &[Card::Gold; 2]);
    deck(&mut g, P0, &[Card::Silver; 8]);
    discard(
        &mut g,
        P0,
        &[Card::Gold, Card::Copper, Card::Estate, Card::Duchy],
    );
    let (mut g, mut k) = start(g);
    play(&mut g, Card::Gold);
    play(&mut g, Card::Gold);
    buy(&mut g, Card::Herald);
    respond(&mut g, Response::Number(2));
    k.update(&mut g);
    let choices = candidates(&g);
    let world = |chosen: &[CardId]| {
        let (mut g, mut k) = (g.clone(), k.clone());
        choose(&mut g, chosen);
        k.update(&mut g);
        choose(&mut g, chosen);
        k.update(&mut g);
        (g, k)
    };
    let ((a, ka), (b, kb)) = (world(&choices[..2]), world(&choices[2..4]));
    assert_eq!(observation(&a, P1, 0), observation(&b, P1, 0));
    assert_eq!(ka.known(&a, P1), kb.known(&b, P1));
}

#[test]
fn discarding_a_revealed_hand_keeps_every_known_copy() {
    let mut g = setup(&[Card::Village]);
    hand(&mut g, P0, &[Card::Gold, Card::Copper, Card::Gold]);
    deck(
        &mut g,
        P0,
        &[
            Card::Gold,
            Card::Estate,
            Card::Copper,
            Card::Silver,
            Card::Duchy,
            Card::Silver,
        ],
    );
    let (mut g, mut k) = start(g);
    g.emit(Event::RevealedHand {
        player: P0,
        cards: vec![Card::Gold, Card::Copper, Card::Gold],
    });
    k.update(&mut g);
    assert_eq!(
        k.known(&g, P1).zone(Zone::Hand(P0)).known,
        count(&[(Card::Gold, 2), (Card::Copper, 1)])
    );
    next_turn(&mut g);
    k.update(&mut g);
    assert_eq!(
        k.known(&g, P1).zone(Zone::Discard(P0)).known,
        count(&[(Card::Gold, 2), (Card::Copper, 1)])
    );
}

#[test]
fn shuffling_searched_discards_with_a_discarded_hand_forgets_their_positions() {
    let mut g = setup(&[Card::Village]);
    let cards = hand(&mut g, P0, &[Card::Estate, Card::Copper, Card::Copper]);
    deck(&mut g, P0, &[Card::Silver]);
    discard(&mut g, P0, &[Card::Gold, Card::Duchy, Card::Copper]);
    let (mut g, mut k) = start(g);
    g.reveal(P0, cards[0]);
    g.look_through_discard(P0);
    k.update(&mut g);
    let mut expected = None;
    for seed in 0..10 {
        let (mut g, mut k) = (g.clone(), k.clone());
        g.rng = ChaCha8Rng::seed_from_u64(seed);
        next_turn(&mut g);
        k.update(&mut g);
        let seen = (observation(&g, P1, 0), k.known(&g, P1));
        match &expected {
            None => expected = Some(seen),
            Some(expected) => assert_eq!(&seen, expected, "shuffle {seed}"),
        }
    }
}

#[test]
fn mixing_some_known_copies_keeps_the_untouched_copies_known() {
    let mut g = setup(&[Card::Chapel, Card::Artist]);
    let hand = hand(&mut g, P0, &[Card::Chapel, Card::Copper, Card::Artist]);
    let deck = deck(&mut g, P0, &[Card::Copper; 5]);
    let (mut g, mut k) = start(g);
    for &c in hand.iter().chain(&deck) {
        g.show(c);
    }
    g.mixed(deck.clone());
    g.put_in_hand(deck[0], P0);
    k.update(&mut g);
    play(&mut g, Card::Chapel);
    k.update(&mut g);
    let world = |copy| {
        let (mut g, mut k) = (g.clone(), k.clone());
        choose(&mut g, &[copy]);
        k.update(&mut g);
        next_turn(&mut g);
        k.update(&mut g);
        (g, k)
    };
    let ((a, ka), (b, kb)) = (world(hand[1]), world(deck[0]));
    assert_eq!(observation(&a, P1, 0), observation(&b, P1, 0));
    assert_eq!(ka.known(&a, P1), kb.known(&b, P1));
    assert_eq!(
        ka.known(&a, P1)
            .zone(Zone::Hand(P0))
            .known
            .get(&Card::Copper),
        Some(&4)
    );
}

#[test]
fn a_family_in_one_zone_keeps_its_whole_known_multiset() {
    let mut g = setup(&[Card::Sauna]);
    hand(&mut g, P0, &[]);
    let cards = deck(&mut g, P0, &[Card::Copper, Card::Sauna]);
    let (g, k) = start(g);
    let world = |mixed: Vec<CardId>| {
        let (mut g, mut k) = (g.clone(), k.clone());
        g.mixed(mixed);
        k.update(&mut g);
        (g, k)
    };
    let ((a, ka), (b, kb)) = (world(vec![cards[0]]), world(cards));
    assert_eq!(observation(&a, P1, 0), observation(&b, P1, 0));
    assert_eq!(ka.known(&a, P1), kb.known(&b, P1));
    assert_eq!(
        ka.known(&a, P1).zone(Zone::Deck(P0)).known,
        count(&[(Card::Copper, 1), (Card::Sauna, 1)])
    );
}

#[test]
fn fated_placements_are_public_but_identical_copies_are_indistinguishable() {
    for above in [0, 4] {
        for seed in 0..8 {
            let mut g = setup(&[Card::Village, Card::NightWatchman]);
            g.landscapes.push(Card::Fated);
            let pile = g.pile_of(Card::Village).unwrap();
            g.piles[pile.index()].traits.push(Card::Fated);
            hand(&mut g, P0, &[Card::NightWatchman]);
            deck(&mut g, P0, &vec![Card::Estate; above]);
            discard(
                &mut g,
                P0,
                &[
                    Card::Village,
                    Card::Village,
                    Card::Copper,
                    Card::Estate,
                    Card::Duchy,
                    Card::Silver,
                    Card::Gold,
                ],
            );
            let (mut g, mut k) = start(g);
            g.rng = ChaCha8Rng::seed_from_u64(seed);
            end_phase(&mut g);
            play(&mut g, Card::NightWatchman);
            if purpose(&g) == Purpose::Trigger {
                super::intrigue::trigger(&mut g, Card::Fated);
            }
            k.update(&mut g);
            let cards = candidates(&g);
            let world = |top: &[CardId], bottom: &[CardId]| {
                let (mut g, mut k) = (g.clone(), k.clone());
                let mut seen =
                    respond_traced(&mut g, &mut k, Response::Cards(top.to_vec()))[1].clone();
                if g.pending().is_some_and(|d| d.source == Some(Card::Fated)) {
                    seen.push_str(
                        &respond_traced(&mut g, &mut k, Response::Cards(bottom.to_vec()))[1],
                    );
                }
                (g, k, seen)
            };
            let (a, ka, sa) = world(&cards[..1], &cards[1..]);
            let (b, kb, sb) = world(&cards[1..], &cards[..1]);
            assert_eq!(sa, sb);
            assert_eq!(observation(&a, P1, 0), observation(&b, P1, 0));
            assert_eq!(ka.known(&a, P1), kb.known(&b, P1));
            let (c, kc, _) = world(&cards, &[]);
            assert_ne!(observation(&a, P1, 0), observation(&c, P1, 0));
            assert_eq!(
                kc.known(&c, P1)
                    .zone(Zone::Looking(P0))
                    .known
                    .get(&Card::Village),
                Some(&(2.min(5 - above) as u32))
            );
            assert_eq!(
                ka.known(&a, P1)
                    .zone(Zone::Looking(P0))
                    .known
                    .get(&Card::Village),
                Some(&1)
            );
        }
    }
}

fn uncertain_over(known: &KnownCards, zones: &[Zone]) -> Option<CardCount> {
    known
        .uncertain
        .iter()
        .find(|u| {
            u.zones.len() == zones.len()
                && zones.iter().all(|z| u.zones.iter().any(|(x, _)| x == z))
        })
        .map(|u| u.candidates.clone())
}

#[test]
fn opponents_know_a_hand_only_as_part_of_the_starting_cards() {
    let (g, k) = start(setup(&[Card::Village]));
    let mine = k.known(&g, P0);
    assert_eq!(mine.zone(Zone::Hand(P0)).unknown, 0);
    assert_eq!(mine.zone(Zone::Hand(P0)).count, 5);
    let theirs = k.known(&g, P1);
    assert_eq!(
        theirs.zone(Zone::Hand(P0)),
        ZoneKnowledge {
            count: 5,
            known: BTreeMap::new(),
            unknown: 5
        }
    );
    assert_eq!(
        uncertain_over(&theirs, &[Zone::Hand(P0), Zone::Deck(P0)]),
        Some(count(&[(Card::Copper, 7), (Card::Estate, 3)]))
    );
    assert_eq!(
        theirs.owned[0],
        (count(&[(Card::Copper, 7), (Card::Estate, 3)]), 0)
    );
}

/// Cards that could only be one card are known wherever they are.
#[test]
fn cards_whose_candidates_are_all_one_card_are_known() {
    let mut g = setup(&[Card::Village]);
    hand(&mut g, P0, &[Card::Copper; 5]);
    deck(&mut g, P0, &[Card::Copper; 5]);
    let (g, k) = start(g);
    let theirs = k.known(&g, P1);
    for zone in [Zone::Hand(P0), Zone::Deck(P0)] {
        let known = theirs.zone(zone);
        assert_eq!(
            (known.known, known.unknown),
            (count(&[(Card::Copper, 5)]), 0)
        );
    }
    assert!(theirs.uncertain.is_empty());
}

#[test]
fn a_shuffle_mixes_known_cards_into_the_deck() {
    let mut g = setup(&[Card::Moat]);
    hand(&mut g, P0, &[Card::Moat, Card::Copper, Card::Copper]);
    deck(&mut g, P0, &[]);
    discard(&mut g, P0, &[Card::Silver, Card::Gold, Card::Estate]);
    let (mut g, mut k) = start(g);
    play(&mut g, Card::Moat);
    k.update(&mut g);
    assert_eq!(g.player(P0).deck.len(), 1);
    let theirs = k.known(&g, P1);
    assert_eq!(theirs.zone(Zone::Hand(P0)).unknown, 4);
    assert_eq!(
        uncertain_over(&theirs, &[Zone::Hand(P0), Zone::Deck(P0)]),
        Some(count(&[
            (Card::Copper, 2),
            (Card::Silver, 1),
            (Card::Gold, 1),
            (Card::Estate, 1)
        ]))
    );
    // The owner saw what they drew, so they know the card left in their deck.
    let left = g.printed(g.player(P0).deck[0]);
    assert_eq!(
        k.known(&g, P0).zone(Zone::Deck(P0)).known,
        count(&[(left, 1)])
    );
}

#[test]
fn a_revealed_card_put_on_a_deck_is_known_when_drawn() {
    let mut g = setup(&[Card::Bureaucrat]);
    hand(&mut g, P0, &[Card::Bureaucrat]);
    hand(
        &mut g,
        P1,
        &[
            Card::Duchy,
            Card::Copper,
            Card::Copper,
            Card::Copper,
            Card::Copper,
        ],
    );
    deck(&mut g, P1, &[Card::Silver; 5]);
    let (mut g, mut k) = start(g);
    play(&mut g, Card::Bureaucrat);
    k.update(&mut g);
    let deck = k.known(&g, P0).zone(Zone::Deck(P1));
    assert_eq!(deck.known.get(&Card::Duchy), Some(&1));
    next_turn(&mut g);
    k.update(&mut g);
    next_turn(&mut g);
    k.update(&mut g);
    // P1 drew the Duchy from the top of their deck in Clean-up.
    let hand = k.known(&g, P0).zone(Zone::Hand(P1));
    assert_eq!(hand.known.get(&Card::Duchy), Some(&1));
}

#[test]
fn cards_being_looked_at_are_off_the_deck_and_known_only_to_the_looker() {
    let mut g = setup(&[Card::SackOfLoot]);
    hand(&mut g, P0, &[Card::Sextant]);
    let mut cards = vec![Card::Gold; 2];
    cards.extend([Card::Silver; 4]);
    deck(&mut g, P0, &cards);
    let (mut g, mut k) = start(g);
    play(&mut g, Card::Sextant);
    k.update(&mut g);
    assert_eq!(g.player(P0).deck.len(), 1);
    let mine = k.known(&g, P0).zone(Zone::Looking(P0));
    assert_eq!(mine.known, count(&[(Card::Gold, 2), (Card::Silver, 3)]));
    let theirs = k.known(&g, P1).zone(Zone::Looking(P0));
    assert_eq!((theirs.count, theirs.unknown), (5, 5));
    let view = g.view(P1);
    assert_eq!(view.player(P0).looking_size(), 5);
    assert!(view.player(P0).looking().is_empty());
}

#[test]
fn a_card_put_on_a_deck_from_hand_stays_hidden() {
    let mut g = setup(&[Card::Courtyard]);
    hand(&mut g, P0, &[Card::Courtyard, Card::Gold]);
    deck(&mut g, P0, &[Card::Copper; 5]);
    let (mut g, mut k) = start(g);
    play(&mut g, Card::Courtyard);
    choose_kinds(&mut g, &[Card::Gold]);
    k.update(&mut g);
    assert_eq!(
        k.known(&g, P1).zone(Zone::Deck(P0)).known.get(&Card::Gold),
        None
    );
    assert_eq!(
        k.known(&g, P0).zone(Zone::Deck(P0)).known.get(&Card::Gold),
        Some(&1)
    );
}

#[test]
fn a_card_picked_from_the_discard_pile_is_known_on_the_deck() {
    let mut g = setup(&[Card::Harbinger]);
    hand(&mut g, P0, &[Card::Harbinger]);
    deck(&mut g, P0, &[Card::Copper; 5]);
    discard(&mut g, P0, &[Card::Estate, Card::Gold, Card::Silver]);
    let (mut g, mut k) = start(g);
    play(&mut g, Card::Harbinger);
    k.update(&mut g);
    choose_kinds(&mut g, &[Card::Gold]);
    k.update(&mut g);
    let deck = k.known(&g, P0).zone(Zone::Deck(P0));
    assert_eq!((deck.known.get(&Card::Gold), deck.unknown), (Some(&1), 0));
    assert_eq!(
        k.known(&g, P1).zone(Zone::Deck(P0)).known.get(&Card::Gold),
        None
    );
}

#[test]
fn a_card_leaving_a_hidden_hand_could_be_any_of_its_cards() {
    let mut g = setup(&[Card::Bureaucrat, Card::Haven]);
    hand(&mut g, P0, &[Card::Bureaucrat]);
    hand(&mut g, P1, &[Card::Duchy, Card::Haven, Card::Copper]);
    deck(&mut g, P1, &[Card::Silver; 5]);
    let (mut g, mut k) = start(g);
    play(&mut g, Card::Bureaucrat);
    k.update(&mut g);
    next_turn(&mut g);
    k.update(&mut g);
    // Haven draws the Duchy back, then sets a card from hand aside face down.
    play_from(&mut g, P1, Card::Haven);
    k.update(&mut g);
    assert_eq!(
        k.known(&g, P0).zone(Zone::Hand(P1)).known,
        count(&[(Card::Duchy, 1)])
    );
    choose_kinds(&mut g, &[Card::Copper]);
    k.update(&mut g);
    let theirs = k.known(&g, P0);
    for zone in [Zone::Hand(P1), Zone::SetAside(P1)] {
        assert_eq!(theirs.zone(zone).known.get(&Card::Duchy), None);
    }
    assert!(theirs.uncertain.iter().any(|u| {
        u.candidates.contains_key(&Card::Duchy)
            && u.zones.iter().any(|(z, _)| *z == Zone::Hand(P1))
            && u.zones.iter().any(|(z, _)| *z == Zone::SetAside(P1))
    }));
}

#[test]
fn a_player_knows_the_cards_they_picked_out_of_a_shuffle() {
    let mut g = setup_with(GameConfig {
        colonies: Some(false),
        shelters: Some(false),
        landscapes: vec![Card::OrderOfMasons],
        ..GameConfig::new(2, vec![Card::Underling, Card::Smithy], 7)
    });
    hand(&mut g, P0, &[Card::Smithy]);
    deck(&mut g, P0, &[]);
    discard(
        &mut g,
        P0,
        &[
            Card::Province,
            Card::Province,
            Card::Curse,
            Card::Gold,
            Card::Gold,
            Card::Gold,
        ],
    );
    let (mut g, mut k) = start(g);
    play(&mut g, Card::Smithy);
    k.update(&mut g);
    // Order of Masons: the shuffled cards P0 chooses go into the discard pile.
    respond(&mut g, Response::option(Opt::Trigger(0)));
    k.update(&mut g);
    choose_kinds(&mut g, &[Card::Province, Card::Curse]);
    k.update(&mut g);
    assert_eq!(
        k.known(&g, P0).zone(Zone::Discard(P0)),
        ZoneKnowledge {
            count: 2,
            known: count(&[(Card::Province, 1), (Card::Curse, 1)]),
            unknown: 0
        }
    );
    // The other player sees only the top one.
    assert_eq!(k.known(&g, P1).zone(Zone::Discard(P0)).unknown, 1);
}

#[test]
fn everyone_follows_a_revealed_card_picked_from_under_the_top_of_a_deck() {
    let mut g = setup(&[Card::Hunter]);
    hand(&mut g, P0, &[Card::Hunter]);
    deck(
        &mut g,
        P0,
        &[Card::Copper, Card::Silver, Card::Estate, Card::Gold],
    );
    let (mut g, mut k) = start(g);
    play(&mut g, Card::Hunter);
    k.update(&mut g);
    // Of the revealed Treasures, the Silver; the Estate is the only Victory card.
    choose_kinds(&mut g, &[Card::Silver]);
    k.update(&mut g);
    assert_eq!(
        k.known(&g, P1).zone(Zone::Hand(P0)).known,
        count(&[(Card::Silver, 1), (Card::Estate, 1)])
    );
}

#[test]
fn a_passed_card_is_known_to_both_players() {
    let mut g = setup(&[Card::Masquerade]);
    hand(&mut g, P0, &[Card::Masquerade, Card::Gold]);
    hand(&mut g, P1, &[Card::Curse, Card::Copper]);
    let (mut g, mut k) = start(g);
    play(&mut g, Card::Masquerade);
    k.update(&mut g);
    choose_kinds(&mut g, &[Card::Gold]);
    k.update(&mut g);
    choose_kinds(&mut g, &[Card::Curse]);
    k.update(&mut g);
    let p0 = k.known(&g, P0);
    assert_eq!(p0.zone(Zone::Hand(P1)).known.get(&Card::Gold), Some(&1));
    assert_eq!(p0.zone(Zone::Hand(P0)).known.get(&Card::Curse), Some(&1));
    let p1 = k.known(&g, P1);
    assert_eq!(p1.zone(Zone::Hand(P1)).known.get(&Card::Gold), Some(&1));
}

/// Two worlds that differ only in which of two cards the deciding player privately picks.
fn worlds(g: &Game, k: &Knowledge, a: CardId, b: CardId) -> [(Game, Knowledge); 2] {
    [a, b].map(|c| {
        let (mut g, mut k) = (g.clone(), k.clone());
        choose(&mut g, &[c]);
        k.update(&mut g);
        (g, k)
    })
}

/// P0 knows one Estate in P1's hand. P1 discards an Estate from a hand holding two: P0
/// can't tell which one, so can no longer place an Estate in that hand.
#[test]
fn a_card_seen_leaving_a_hidden_hand_could_be_any_copy_of_it() {
    let mut g = setup(&[Card::Bureaucrat, Card::Village, Card::Cellar]);
    hand(&mut g, P0, &[Card::Bureaucrat]);
    let p1 = hand(
        &mut g,
        P1,
        &[
            Card::Estate,
            Card::Estate,
            Card::Village,
            Card::Cellar,
            Card::Copper,
        ],
    );
    deck(&mut g, P1, &[Card::Copper; 5]);
    let (mut g, mut k) = start(g);
    play(&mut g, Card::Bureaucrat);
    k.update(&mut g);
    let seen = *g.player(P1).deck.last().unwrap();
    let unseen = if seen == p1[0] { p1[1] } else { p1[0] };
    next_turn(&mut g);
    k.update(&mut g);
    play_from(&mut g, P1, Card::Village);
    k.update(&mut g);
    assert_eq!(
        k.known(&g, P0).zone(Zone::Hand(P1)).known,
        count(&[(Card::Estate, 1)])
    );
    play_from(&mut g, P1, Card::Cellar);
    k.update(&mut g);
    let [(ga, ka), (gb, kb)] = worlds(&g, &k, unseen, seen);
    let a = ka.known(&ga, P0).zone(Zone::Hand(P1));
    assert_eq!(a, kb.known(&gb, P0).zone(Zone::Hand(P1)));
    assert_eq!(a.known.get(&Card::Estate), None);
}

/// A hand discarded whole in Clean-up takes its known cards to the discard pile.
#[test]
fn a_known_card_is_followed_when_its_whole_hand_is_discarded() {
    let mut g = setup(&[Card::Bureaucrat]);
    hand(&mut g, P0, &[Card::Bureaucrat]);
    hand(&mut g, P1, &[Card::Duchy, Card::Copper]);
    deck(&mut g, P1, &[Card::Silver; 12]);
    let (mut g, mut k) = start(g);
    play(&mut g, Card::Bureaucrat);
    k.update(&mut g);
    // P1 draws the Duchy, then discards it with the rest of that hand a turn later.
    for _ in 0..4 {
        next_turn(&mut g);
        k.update(&mut g);
    }
    assert_eq!(
        k.known(&g, P0)
            .zone(Zone::Discard(P1))
            .known
            .get(&Card::Duchy),
        Some(&1)
    );
}

/// Rabble: P1 puts the three revealed Victory cards back in an order of their own. P0
/// doesn't know the order, but knows they are on that deck, and in the hand once drawn.
#[test]
fn revealed_cards_put_back_on_a_deck_stay_known() {
    let mut g = setup(&[Card::Rabble]);
    hand(&mut g, P0, &[Card::Rabble]);
    deck(&mut g, P0, &[Card::Copper; 10]);
    hand(&mut g, P1, &[Card::Copper; 5]);
    deck(
        &mut g,
        P1,
        &[
            Card::Estate,
            Card::Duchy,
            Card::Province,
            Card::Silver,
            Card::Silver,
            Card::Gold,
            Card::Gold,
        ],
    );
    let (mut g, mut k) = start(g);
    play(&mut g, Card::Rabble);
    k.update(&mut g);
    let revealed = candidates(&g);
    choose(&mut g, &[revealed[1], revealed[2], revealed[0]]);
    k.update(&mut g);
    let top3 = count(&[(Card::Estate, 1), (Card::Duchy, 1), (Card::Province, 1)]);
    assert_eq!(k.known(&g, P0).zone(Zone::Deck(P1)).known, top3);
    next_turn(&mut g);
    k.update(&mut g);
    next_turn(&mut g);
    k.update(&mut g);
    assert_eq!(k.known(&g, P0).zone(Zone::Hand(P1)).known, top3);
}

/// Navigator: P1 puts five cards back in an order P0 can't see, even when they are five
/// Estates and there is no order to ask for.
#[test]
fn cards_put_back_in_a_forced_order_are_mixed_too() {
    let mut g = setup(&[Card::Bureaucrat, Card::Navigator]);
    hand(&mut g, P0, &[Card::Bureaucrat]);
    hand(
        &mut g,
        P1,
        &[Card::Navigator, Card::Estate, Card::Copper, Card::Copper],
    );
    deck(
        &mut g,
        P1,
        &[
            Card::Estate,
            Card::Estate,
            Card::Estate,
            Card::Estate,
            Card::Copper,
            Card::Copper,
        ],
    );
    let (mut g, mut k) = start(g);
    play(&mut g, Card::Bureaucrat);
    k.update(&mut g);
    next_turn(&mut g);
    k.update(&mut g);
    assert_eq!(
        k.known(&g, P0).zone(Zone::Deck(P1)).known,
        count(&[(Card::Estate, 1)])
    );
    play_from(&mut g, P1, Card::Navigator);
    k.update(&mut g);
    respond(&mut g, Response::option(Opt::Mode(1)));
    k.update(&mut g);
    assert_eq!(
        k.known(&g, P0).zone(Zone::Deck(P1)).known,
        count(&[(Card::Estate, 1)])
    );
}

/// Sleigh is discarded onto the card just gained, which then moves in everyone's view.
#[test]
fn everyone_follows_a_gained_card_moved_from_under_the_top_of_a_discard_pile() {
    for (mode, zone) in [(0, Zone::Hand(P0)), (1, Zone::Deck(P0))] {
        let mut g = setup(&[Card::Sleigh]);
        hand(&mut g, P0, &[Card::Sleigh, Card::Gold]);
        deck(&mut g, P0, &[Card::Copper; 5]);
        discard(&mut g, P0, &[]);
        let (mut g, mut k) = start(g);
        g.turn.buys = 3;
        end_phase(&mut g);
        play(&mut g, Card::Gold);
        buy(&mut g, Card::Copper);
        k.update(&mut g);
        no(&mut g);
        k.update(&mut g);
        buy(&mut g, Card::Silver);
        k.update(&mut g);
        respond(&mut g, Response::option(Opt::Trigger(0)));
        k.update(&mut g);
        respond(&mut g, Response::option(Opt::Mode(mode)));
        k.update(&mut g);
        for viewer in [P0, P1] {
            let known = k.known(&g, viewer);
            assert_eq!(known.zone(zone).known.get(&Card::Silver), Some(&1));
            assert_eq!(
                known.zone(Zone::Discard(P0)).known,
                count(&[(Card::Copper, 1), (Card::Sleigh, 1)])
            );
        }
    }
}

/// Hermit offers only the non-Treasures of the discard pile, but its player looks through
/// all of it.
#[test]
fn looking_through_a_discard_pile_shows_all_of_it() {
    let mut g = setup(&[Card::Hermit]);
    hand(&mut g, P0, &[Card::Hermit]);
    let pile = discard(&mut g, P0, &[Card::Gold, Card::Estate]);
    g.enable_trace();
    g.start();
    g.take_trace();
    play(&mut g, Card::Hermit);
    let looked: Vec<CardId> = g
        .take_trace()
        .into_iter()
        .filter_map(|t| match t {
            Trace::Shown { card, to } if to == Some(P0) => Some(card),
            _ => None,
        })
        .collect();
    assert_eq!(looked, pile);
}

/// Harbinger: its player knows the card they took from their discard pile. The others
/// saw a card leave a pile its owner was looking through, not which.
#[test]
fn only_its_owner_follows_a_card_picked_out_of_a_discard_pile() {
    let mut g = setup(&[Card::Harbinger]);
    hand(&mut g, P0, &[Card::Harbinger]);
    deck(&mut g, P0, &[Card::Copper; 5]);
    discard(&mut g, P0, &[]);
    let (mut g, mut k) = start(g);
    // Everyone saw these gained.
    let pile: Vec<CardId> = [Card::Gold, Card::Estate, Card::Curse]
        .into_iter()
        .map(|card| g.create(card, Zone::Discard(P0)))
        .collect();
    k.update(&mut g);
    play(&mut g, Card::Harbinger);
    k.update(&mut g);
    let [(ga, ka), (gb, kb)] = worlds(&g, &k, pile[0], pile[1]);
    let mine = ka.known(&ga, P0);
    assert_eq!(
        mine.zone(Zone::Deck(P0)).known,
        count(&[(Card::Gold, 1), (Card::Copper, 4)])
    );
    assert_eq!(
        mine.zone(Zone::Discard(P0)).known,
        count(&[(Card::Estate, 1), (Card::Curse, 1)])
    );
    let theirs = ka.known(&ga, P1);
    assert_eq!(theirs, kb.known(&gb, P1));
    assert_eq!(theirs.zone(Zone::Deck(P0)).known.get(&Card::Gold), None);
    // The Curse is still on top of the pile.
    assert_eq!(
        theirs.zone(Zone::Discard(P0)).known,
        count(&[(Card::Curse, 1)])
    );
}

/// Harbinger taking the top card of the discard pile: its player needn't keep the pile in
/// order (wiki, Discard: "it is not necessary to keep it in the same order"), so the card
/// now on top doesn't tell the others which card was taken.
#[test]
fn the_new_top_of_a_discard_pile_looked_through_says_nothing_of_the_card_taken() {
    let mut g = setup(&[Card::Harbinger]);
    hand(&mut g, P0, &[Card::Harbinger]);
    deck(&mut g, P0, &[Card::Copper; 5]);
    discard(&mut g, P0, &[]);
    let (mut g, mut k) = start(g);
    let pile: Vec<CardId> = [Card::Gold, Card::Estate, Card::Curse]
        .into_iter()
        .map(|card| g.create(card, Zone::Discard(P0)))
        .collect();
    k.update(&mut g);
    play(&mut g, Card::Harbinger);
    choose(&mut g, &[pile[2]]);
    k.update(&mut g);
    let theirs = k.known(&g, P1);
    assert_eq!(
        theirs.zone(Zone::Discard(P0)).known,
        count(&[(Card::Estate, 1)])
    );
    assert_eq!(
        uncertain_over(&theirs, &[Zone::Deck(P0), Zone::Discard(P0)]),
        Some(count(&[(Card::Gold, 1), (Card::Curse, 1)]))
    );
}

/// Mountain Village takes one card from a discard pile whose cards the others knew, some
/// only as cards of the deck's and hand's group: they know the pile holds all of those
/// cards but the one that went into the hand, and what they knew of the deck and hand
/// doesn't take in the pile's cards.
#[test]
fn a_card_taken_from_a_discard_pile_leaves_what_is_known_of_the_rest() {
    let mut g = setup(&[Card::MountainVillage]);
    hand(
        &mut g,
        P0,
        &[Card::MountainVillage, Card::Copper, Card::Copper],
    );
    deck(&mut g, P0, &[Card::Copper, Card::Estate, Card::Estate]);
    // The Estate on top is seen; the Copper under it is one of the starting cards.
    let estate = discard(&mut g, P0, &[Card::Copper, Card::Estate])[1];
    let (mut g, mut k) = start(g);
    let gold = g.create(Card::Gold, Zone::Discard(P0));
    g.create(Card::Silver, Zone::Discard(P0));
    k.update(&mut g);
    let starting = count(&[
        (Card::Copper, 4),
        (Card::Estate, 2),
        (Card::MountainVillage, 1),
    ]);
    let all = [Zone::Hand(P0), Zone::Deck(P0), Zone::Discard(P0)];
    assert_eq!(
        uncertain_over(&k.known(&g, P1), &all),
        Some(starting.clone())
    );
    play(&mut g, Card::MountainVillage);
    k.update(&mut g);
    let [(ga, ka), (gb, kb)] = worlds(&g, &k, gold, estate);
    let theirs = ka.known(&ga, P1);
    assert_eq!(theirs, kb.known(&gb, P1));
    // The Silver now on top is seen. The pile's other cards, one of them now in the hand,
    // are the Gold, the Estate and one of the starting cards.
    assert_eq!(
        theirs.zone(Zone::Discard(P0)).known,
        count(&[(Card::Silver, 1)])
    );
    let taken = theirs
        .uncertain
        .iter()
        .find(|u| u.zones == [(Zone::Hand(P0), 1), (Zone::Discard(P0), 2)])
        .expect("the pile's cards");
    assert_eq!(
        (&taken.candidates, taken.outside),
        (
            &count(&[(Card::Copper, 4), (Card::Estate, 3), (Card::Gold, 1)]),
            5
        )
    );
    // The deck and the rest of the hand hold the starting cards but one, as before.
    let rest = theirs
        .uncertain
        .iter()
        .find(|u| u.zones == [(Zone::Deck(P0), 3), (Zone::Hand(P0), 2)])
        .expect("the starting cards");
    assert_eq!(
        (&rest.candidates, rest.outside),
        (&count(&[(Card::Copper, 4), (Card::Estate, 2)]), 1)
    );
    assert_eq!(
        theirs.owned[0],
        (
            count(&[
                (Card::Copper, 4),
                (Card::Estate, 3),
                (Card::Gold, 1),
                (Card::Silver, 1),
                (Card::MountainVillage, 1)
            ]),
            0
        )
    );
}

/// Avoid: its player knows which of the cards being shuffled they kept out.
#[test]
fn a_player_knows_the_cards_avoid_kept_out_of_the_shuffle() {
    let mut g = setup_with(GameConfig {
        colonies: Some(false),
        shelters: Some(false),
        landscapes: vec![Card::Avoid],
        ..GameConfig::new(2, vec![Card::Village, Card::Smithy], 7)
    });
    hand(&mut g, P0, &[Card::Copper; 3]);
    deck(&mut g, P0, &[]);
    let pile = discard(
        &mut g,
        P0,
        &[
            Card::Gold,
            Card::Silver,
            Card::Estate,
            Card::Duchy,
            Card::Province,
            Card::Curse,
        ],
    );
    let (mut g, mut k) = start(g);
    for _ in 0..3 {
        play(&mut g, Card::Copper);
    }
    respond(&mut g, Response::Move(Move::BuyLandscape(Card::Avoid)));
    end_phase(&mut g);
    k.update(&mut g);
    assert_eq!(g.pending().unwrap().source, Some(Card::Avoid));
    choose(&mut g, &[pile[5], pile[2]]);
    k.update(&mut g);
    assert_eq!(
        k.known(&g, P0).zone(Zone::Discard(P0)).known,
        count(&[(Card::Curse, 1), (Card::Estate, 1)])
    );
}

/// The groups of `viewer`'s knowledge with cards in `zone`.
fn groups_in(g: &Game, k: &Knowledge, viewer: PlayerId, zone: Zone) -> Vec<Vec<CardId>> {
    k.groups(viewer)
        .filter(|group| group.cards.iter().any(|&c| g.zone(c) == zone))
        .map(|group| group.cards.to_vec())
        .collect()
}

/// Star Chart's player looks through the cards they shuffle and the ones left on the
/// deck: they know the one they put on top, and the others are shuffled again.
#[test]
fn star_chart_shows_the_shuffled_cards_and_shuffles_the_others_again() {
    let mut g = setup(&[Card::Smithy]);
    g.players[0].projects.push(Card::StarChart);
    hand(&mut g, P0, &[Card::Smithy]);
    let above = deck(&mut g, P0, &[Card::Silver]);
    let shuffled = discard(
        &mut g,
        P0,
        &[
            Card::Gold,
            Card::Estate,
            Card::Duchy,
            Card::Province,
            Card::Curse,
            Card::Copper,
        ],
    );
    let (mut g, mut k) = start(g);
    play(&mut g, Card::Smithy);
    respond(&mut g, Response::option(Opt::Trigger(0)));
    assert_eq!(g.pending().unwrap().about, above);
    choose(&mut g, &[shuffled[0]]);
    k.update(&mut g);
    assert_eq!(hand_kinds(&g, P0)[..2], [Card::Silver, Card::Gold]);
    let rest = g.player(P0).deck.clone();
    assert_eq!(rest.len(), 4);
    let mut groups = groups_in(&g, &k, P0, Zone::Deck(P0));
    groups.iter_mut().for_each(|group| group.sort());
    let mut rest_sorted = rest.clone();
    rest_sorted.sort();
    assert_eq!(groups, [rest_sorted]);
}

/// What `viewer` knows of `p`'s hand, deck and discard pile, the groups spread over them
/// in a fixed order.
fn known_of(g: &Game, k: &Knowledge, viewer: PlayerId, p: PlayerId) -> Vec<String> {
    let known = k.known(g, viewer);
    let zones = [Zone::Hand(p), Zone::Deck(p), Zone::Discard(p)];
    let mut uncertain: Vec<String> = known.uncertain.iter().map(|u| format!("{u:?}")).collect();
    uncertain.sort();
    zones
        .map(|z| format!("{:?}", known.zone(z)))
        .into_iter()
        .chain(uncertain)
        .collect()
}

/// A shuffle of the whole deck (Annex; Inn and Famine shuffle the same way) mixes the
/// cards Star Chart looked through on an earlier shuffle with the others: drawing some of
/// them tells the others nothing.
#[test]
fn a_shuffle_of_the_whole_deck_mixes_cards_looked_through_before() {
    let mut g = setup_with(GameConfig {
        colonies: Some(false),
        shelters: Some(false),
        landscapes: vec![Card::Annex, Card::StarChart],
        ..GameConfig::new(2, vec![Card::Smithy], 7)
    });
    g.players[0].projects.push(Card::StarChart);
    hand(&mut g, P0, &[Card::Smithy]);
    deck(&mut g, P0, &[]);
    let mut pile = vec![Card::Gold; 4];
    pile.extend([Card::Silver; 3]);
    pile.extend([Card::Estate; 3]);
    let pile = discard(&mut g, P0, &pile);
    let (mut g, mut k) = start(g);
    play(&mut g, Card::Smithy);
    respond(&mut g, Response::option(Opt::Trigger(0)));
    choose(&mut g, &[pile[0]]);
    k.update(&mut g);
    next_turn(&mut g);
    k.update(&mut g);
    next_turn(&mut g);
    k.update(&mut g);
    let pl = g.player(P0);
    assert_eq!((pl.hand.len(), pl.deck.len(), pl.discard.len()), (5, 2, 4));
    // Annex shuffles the 4 cards of the discard pile into the 2 of the deck, Star Chart
    // declined; Clean-up draws 5 of the 6.
    let world = |seed: u64| {
        let (mut g, mut k) = (g.clone(), k.clone());
        g.rng = ChaCha8Rng::seed_from_u64(seed);
        respond(&mut g, Response::Move(Move::BuyLandscape(Card::Annex)));
        choose(&mut g, &[]);
        respond(&mut g, Response::option(Opt::No));
        next_turn(&mut g);
        k.update(&mut g);
        (g, k)
    };
    let (first, first_k) = world(0);
    let mut hands = std::collections::BTreeSet::new();
    for seed in 0..12 {
        let (g, k) = world(seed);
        hands.insert(hand_kinds(&g, P0));
        assert_eq!(g.log(), first.log());
        assert_eq!(
            known_of(&g, &k, P1, P0),
            known_of(&first, &first_k, P1, P0),
            "seed {seed}"
        );
    }
    assert!(hands.len() > 1, "the shuffles all came out the same");
}

/// Bad Omens leaves two Coppers as the deck, as everyone knows.
#[test]
fn everyone_knows_the_coppers_bad_omens_leaves_on_the_deck() {
    let mut g = setup(&[Card::Skulk]);
    hand(&mut g, P0, &[Card::Skulk]);
    hand(&mut g, P1, &[]);
    deck(
        &mut g,
        P1,
        &[
            Card::Copper,
            Card::Estate,
            Card::Copper,
            Card::Silver,
            Card::Copper,
        ],
    );
    discard(&mut g, P1, &[]);
    g.hexes.draw = vec![Card::BadOmens];
    let (mut g, mut k) = start(g);
    play(&mut g, Card::Skulk);
    k.update(&mut g);
    for viewer in [P0, P1] {
        assert_eq!(
            k.known(&g, viewer).zone(Zone::Deck(P1)).known,
            count(&[(Card::Copper, 2)])
        );
    }
}

/// Crypt's Treasures are face down: the others saw them go under it, not which one comes
/// back.
#[test]
fn only_its_owner_follows_a_card_taken_from_under_a_crypt() {
    let mut g = setup(&[Card::Crypt]);
    hand(&mut g, P0, &[Card::Gold, Card::Copper, Card::Crypt]);
    deck(&mut g, P0, &[Card::Estate; 10]);
    let (mut g, mut k) = start(g);
    play(&mut g, Card::Gold);
    play(&mut g, Card::Copper);
    end_phase(&mut g);
    play(&mut g, Card::Crypt);
    let under = candidates(&g);
    choose(&mut g, &under);
    k.update(&mut g);
    assert_eq!(
        k.known(&g, P1).zone(Zone::SetAside(P0)).known,
        count(&[(Card::Gold, 1), (Card::Copper, 1)])
    );
    while g.pending().unwrap().source != Some(Card::Crypt) {
        end_phase(&mut g);
        k.update(&mut g);
    }
    let [(ga, ka), (gb, kb)] = worlds(&g, &k, under[0], under[1]);
    let theirs = ka.known(&ga, P1);
    assert_eq!(theirs, kb.known(&gb, P1));
    // The Estates it drew could only be Estates.
    assert_eq!(
        theirs.zone(Zone::Hand(P0)).known,
        count(&[(Card::Estate, 5)])
    );
    assert_eq!(
        uncertain_over(&theirs, &[Zone::Hand(P0), Zone::SetAside(P0)]),
        Some(count(&[(Card::Gold, 1), (Card::Copper, 1)]))
    );
    assert_eq!(
        ka.known(&ga, P0)
            .zone(Zone::Hand(P0))
            .known
            .get(&Card::Gold),
        Some(&1)
    );
}

/// A Stash set aside face down between two Treasures has its own back, and does not tell
/// which of the others comes back first.
#[test]
fn a_stash_under_a_crypt_does_not_tell_which_treasure_is_taken() {
    let mut g = setup(&[Card::Crypt, Card::Stash]);
    hand(
        &mut g,
        P0,
        &[Card::Gold, Card::Stash, Card::Copper, Card::Crypt],
    );
    deck(&mut g, P0, &[Card::Estate; 10]);
    let (mut g, mut k) = start(g);
    for card in [Card::Gold, Card::Stash, Card::Copper] {
        play(&mut g, card);
    }
    end_phase(&mut g);
    play(&mut g, Card::Crypt);
    let under = candidates(&g);
    assert_eq!(kinds(&g, &under), [Card::Gold, Card::Stash, Card::Copper]);
    choose(&mut g, &under);
    k.update(&mut g);
    while g.pending().unwrap().source != Some(Card::Crypt) {
        end_phase(&mut g);
        k.update(&mut g);
    }
    let [(ga, ka), (gb, kb)] = worlds(&g, &k, under[0], under[2]);
    assert_eq!(ka.known(&ga, P1), kb.known(&gb, P1));
}

/// The same with Archive, for cards everyone saw go onto the deck around a Stash, or over
/// it.
#[test]
fn a_stash_set_aside_with_cards_from_the_deck_does_not_tell_which_one_is_taken() {
    for top in [
        [Card::Gold, Card::Stash, Card::Copper],
        [Card::Gold, Card::Copper, Card::Stash],
    ] {
        let mut g = setup(&[Card::Archive, Card::Stash]);
        hand(&mut g, P0, &[Card::Archive]);
        deck(&mut g, P0, &[]);
        let (mut g, mut k) = start(g);
        for &card in top.iter().rev() {
            let id = g.create(card, Zone::Deck(P0));
            g.show(id);
        }
        k.update(&mut g);
        play(&mut g, Card::Archive);
        k.update(&mut g);
        let aside = candidates(&g);
        let of = |card: Card| *aside.iter().find(|&&c| g.card(c) == card).unwrap();
        let [(ga, ka), (gb, kb)] = worlds(&g, &k, of(Card::Gold), of(Card::Copper));
        assert_eq!(ka.known(&ga, P1), kb.known(&gb, P1), "{top:?}");
    }
}

/// Archive: P0 knows the Duchy on top of P1's deck goes under the Archive, but not which
/// of the three cards P1 takes.
#[test]
fn a_known_card_set_aside_face_down_with_others_is_lost_when_one_is_taken() {
    let mut g = setup(&[Card::Archive, Card::Bureaucrat]);
    hand(&mut g, P0, &[Card::Bureaucrat]);
    hand(&mut g, P1, &[Card::Archive, Card::Duchy]);
    deck(&mut g, P1, &[Card::Copper; 6]);
    let (mut g, mut k) = start(g);
    play(&mut g, Card::Bureaucrat);
    k.update(&mut g);
    next_turn(&mut g);
    k.update(&mut g);
    play_from(&mut g, P1, Card::Archive);
    k.update(&mut g);
    // The rest of P1's deck could only be Coppers.
    assert_eq!(
        k.known(&g, P0).zone(Zone::SetAside(P1)).known,
        count(&[(Card::Duchy, 1), (Card::Copper, 2)])
    );
    let aside = candidates(&g);
    let duchy = *aside.iter().find(|&&c| g.card(c) == Card::Duchy).unwrap();
    let copper = *aside.iter().find(|&&c| g.card(c) == Card::Copper).unwrap();
    let [(ga, ka), (gb, kb)] = worlds(&g, &k, duchy, copper);
    let theirs = ka.known(&ga, P0);
    assert_eq!(theirs, kb.known(&gb, P0));
    for zone in [Zone::Hand(P1), Zone::SetAside(P1)] {
        assert_eq!(theirs.zone(zone).known.get(&Card::Duchy), None);
    }
}

/// Annex: nobody else sees which cards stay in the discard pile, even if one of them was
/// its top card.
#[test]
fn only_its_owner_knows_which_discards_annex_kept() {
    let mut g = setup_with(GameConfig {
        colonies: Some(false),
        shelters: Some(false),
        landscapes: vec![Card::Annex],
        ..GameConfig::new(2, vec![Card::Village], 7)
    });
    hand(&mut g, P0, &[Card::Estate]);
    deck(&mut g, P0, &[Card::Copper; 6]);
    let pile = discard(&mut g, P0, &[Card::Gold, Card::Silver, Card::Curse]);
    let (mut g, mut k) = start(g);
    respond(&mut g, Response::Move(Move::BuyLandscape(Card::Annex)));
    k.update(&mut g);
    let [(ga, ka), (gb, kb)] = worlds(&g, &k, pile[2], pile[0]);
    // The kept card, the Duchy, and the Estate discarded in Clean-up.
    let theirs = ka.known(&ga, P1);
    assert_eq!(theirs, kb.known(&gb, P1));
    assert_eq!(
        theirs.zone(Zone::Discard(P0)),
        ZoneKnowledge {
            count: 3,
            known: count(&[(Card::Duchy, 1), (Card::Estate, 1)]),
            unknown: 1
        }
    );
    assert_eq!(
        ka.known(&ga, P0)
            .zone(Zone::Discard(P0))
            .known
            .get(&Card::Curse),
        Some(&1)
    );
}

/// Mapmaker: P0 knows the Duchy on top of P1's deck and sees which two of the four cards
/// P1 looked at are discarded, so the Duchy went to the hand.
#[test]
fn a_known_card_not_among_the_discards_went_to_the_hand() {
    let mut g = setup(&[Card::Mapmaker, Card::Bureaucrat]);
    hand(&mut g, P0, &[Card::Bureaucrat]);
    hand(&mut g, P1, &[Card::Mapmaker, Card::Duchy]);
    deck(
        &mut g,
        P1,
        &[
            Card::Silver,
            Card::Gold,
            Card::Estate,
            Card::Copper,
            Card::Copper,
        ],
    );
    let (mut g, mut k) = start(g);
    play(&mut g, Card::Bureaucrat);
    k.update(&mut g);
    next_turn(&mut g);
    k.update(&mut g);
    play_from(&mut g, P1, Card::Mapmaker);
    k.update(&mut g);
    choose_kinds(&mut g, &[Card::Duchy, Card::Silver]);
    k.update(&mut g);
    let theirs = k.known(&g, P0);
    assert_eq!(
        theirs.zone(Zone::Hand(P1)).known,
        count(&[(Card::Duchy, 1)])
    );
    assert_eq!(
        theirs.zone(Zone::Discard(P1)).known,
        count(&[(Card::Gold, 1), (Card::Estate, 1)])
    );
}

/// Shadow cards go to the bottom when shuffled, and each shows what it is from its back.
#[test]
fn everyone_knows_where_a_shadow_card_is() {
    let mut g = setup(&[Card::Alley, Card::Smithy]);
    hand(&mut g, P0, &[Card::Smithy]);
    deck(&mut g, P0, &[]);
    discard(
        &mut g,
        P0,
        &[
            Card::Alley,
            Card::Copper,
            Card::Estate,
            Card::Gold,
            Card::Silver,
        ],
    );
    let (mut g, mut k) = start(g);
    play(&mut g, Card::Smithy);
    k.update(&mut g);
    let known = k.known(&g, P1);
    assert_eq!(
        known.zone(Zone::Deck(P0)),
        ZoneKnowledge {
            count: 2,
            known: count(&[(Card::Alley, 1)]),
            unknown: 1
        }
    );
    assert_eq!(known.zone(Zone::Hand(P0)).unknown, 3);
}

/// Which cards a pile holds is public even when their order isn't: the face-down Black
/// Market deck, the shuffled Knights.
#[test]
fn everyone_knows_what_a_shuffled_pile_is_made_of() {
    let (g, k) = start(setup(&[Card::BlackMarket, Card::Knights]));
    let market = (0..g.piles().len())
        .map(|i| PileId(i as u8))
        .find(|&p| g.pile(p).hidden)
        .unwrap();
    for pile in [market, g.supply_pile(Card::Knights).unwrap()] {
        let known = k.known(&g, P0).zone(Zone::Pile(pile));
        let held = printed(&g, &g.pile(pile).cards);
        assert_eq!((known.known, known.unknown), (held, 0));
    }
}

/// The Ruins are dealt face down from all fifty: only six players, who use them all, know
/// what the pile holds; below six only its face-up top card is known.
#[test]
fn the_ruins_pile_is_known_only_when_six_players_use_all_fifty() {
    for (players, size) in [(5, 40), (6, 50)] {
        let (g, k) = start(setup_with(GameConfig {
            colonies: Some(false),
            shelters: Some(false),
            ..GameConfig::new(players, vec![Card::Cultist], 7)
        }));
        let pile = g.pile_of(Card::Ruins).unwrap();
        let known = k.known(&g, P0).zone(Zone::Pile(pile));
        let held = printed(&g, &g.pile(pile).cards);
        assert_eq!(known.count, size);
        if players == 6 {
            assert_eq!((known.known, known.unknown), (held, 0));
        } else {
            let top = g.printed(g.pile_top(pile).unwrap());
            assert_eq!((known.known, known.unknown), (count(&[(top, 1)]), size - 1));
        }
    }
}

/// After a card passes between two other players, a viewer can't say whose each of their
/// cards is, but still knows how many each of them owns.
#[test]
fn players_who_passed_a_card_still_own_their_number_of_cards() {
    let mut g = setup_with(GameConfig {
        colonies: Some(false),
        shelters: Some(false),
        ..GameConfig::new(3, vec![Card::Masquerade], 7)
    });
    hand(&mut g, P0, &[Card::Masquerade, Card::Copper]);
    hand(&mut g, P1, &[Card::Curse, Card::Copper]);
    hand(&mut g, P2, &[Card::Copper, Card::Copper]);
    let (mut g, mut k) = start(g);
    play(&mut g, Card::Masquerade);
    for pass in [Card::Copper, Card::Curse, Card::Copper] {
        choose_kinds(&mut g, &[pass]);
        k.update(&mut g);
    }
    let known = k.known(&g, P0);
    for p in [P1, P2] {
        let (ids, unknown) = &known.owned[p.index()];
        assert_eq!(
            ids.values().sum::<u32>() as usize + unknown,
            g.player(p).owned().count()
        );
    }
}

/// The true identities of `cards`, as `viewer` would name the ones they see.
fn truth(g: &Game, viewer: PlayerId, cards: &[CardId]) -> CardCount {
    let view = g.view(viewer);
    let mut out = CardCount::new();
    for &c in cards {
        *out.entry(view.card(c).unwrap_or(g.printed(c))).or_default() += 1;
    }
    out
}

/// Whether `actual` is all of `ids` but `extra` of them.
pub(super) fn some_of(ids: &CardCount, extra: u32, actual: &CardCount) -> bool {
    let within = actual
        .iter()
        .all(|(c, &n)| ids.get(c).is_some_and(|&m| m >= n));
    within && ids.values().sum::<u32>() == actual.values().sum::<u32>() + extra
}

fn check_knowledge(g: &Game, k: &Knowledge, step: usize) {
    for v in 0..g.num_players() {
        let v = PlayerId(v as u8);
        for group in k.groups(v) {
            if let Some(ids) = group.ids {
                let actual: CardCount = group.cards.iter().fold(CardCount::new(), |mut m, &c| {
                    *m.entry(g.printed(c)).or_default() += 1;
                    m
                });
                assert!(
                    some_of(ids, group.extra, &actual),
                    "{v}'s group {:?} is {actual:?}, known as {ids:?} less {}",
                    group.cards,
                    group.extra
                );
            }
            if group.cards.len() > 1 {
                for &c in group.cards {
                    assert!(g.view(v).card(c).is_none(), "{v} sees {c:?} in a group");
                }
            }
        }
        let known = k.known(g, v);
        for (&zone, z) in &known.zones {
            let cards: Vec<CardId> = (0..g.card_count() as u32)
                .map(CardId)
                .filter(|&c| g.zone(c) == zone)
                .collect();
            assert_eq!(z.count, cards.len());
            let actual = truth(g, v, &cards);
            for (card, n) in &z.known {
                assert!(
                    actual.get(card).is_some_and(|m| m >= n),
                    "{v} knows {n} {card} in {zone:?}, which has {actual:?}"
                );
            }
        }
        if step.is_multiple_of(5) {
            check_no_leak(g, k, v, &known);
        }
    }
}

/// Changing what `v` can't observe doesn't change what they know.
fn check_no_leak(g: &Game, k: &Knowledge, v: PlayerId, known: &KnownCards) {
    let mut altered = g.clone();
    for o in 0..g.num_players() {
        if o != v.index() {
            altered.players[o].deck.reverse();
        }
    }
    let view = g.view(v);
    for group in k.groups(v) {
        if group.cards.len() > 1 && group.cards.iter().all(|&c| view.card(c).is_none()) {
            let ids: Vec<Card> = group.cards.iter().map(|&c| g.printed(c)).collect();
            for (i, &c) in group.cards.iter().enumerate() {
                altered.cards[c.index()] = ids[(i + 1) % ids.len()];
            }
        }
    }
    assert_eq!(&k.known(&altered, v), known, "{v}'s knowledge leaks");
}

fn drive_traced(mut g: Game, seed: u64) {
    let mut k = Knowledge::new(g.num_players());
    let mut agents: Vec<Chaos> = (0..g.num_players())
        .map(|p| Chaos(ChaCha8Rng::seed_from_u64(seed * 10 + p as u64)))
        .collect();
    let mut step = 0;
    k.update(&mut g);
    while let Some(d) = g.pending().cloned() {
        check_knowledge(&g, &k, step);
        let r = agents[d.player.index()].decide(&g.view(d.player), &d);
        g.respond(r).unwrap();
        k.update(&mut g);
        step += 1;
        assert!(step < 100_000, "seed {seed}: game does not end");
    }
    check_knowledge(&g, &k, 0);
}

#[test]
fn knowledge_matches_the_truth_and_does_not_leak_in_random_games() {
    for seed in 0..fuzz_seeds(40) {
        let mut rng = ChaCha8Rng::seed_from_u64(seed ^ 0x6b6e6f77);
        let players = rng.random_range(2..=4);
        let mut config = GameConfig::random(players, IMPLEMENTED, seed);
        config.turn_limit = Some(40);
        let mut g = setup_with(config);
        let extra: Vec<Card> = g
            .config()
            .kingdom
            .iter()
            .map(|&k| k.pile_members()[0])
            .collect();
        for p in 0..g.num_players() {
            for &card in &extra {
                g.create(card, Zone::Deck(PlayerId(p as u8)));
            }
        }
        g.enable_trace();
        g.start();
        drive_traced(g, seed);
    }
}

/// Stash has its own back, so everyone follows it through a shuffle and into a hand.
#[test]
fn everyone_knows_where_a_stash_is() {
    let mut g = setup(&[Card::Stash, Card::Smithy]);
    hand(&mut g, P0, &[Card::Smithy]);
    deck(&mut g, P0, &[]);
    discard(
        &mut g,
        P0,
        &[
            Card::Stash,
            Card::Copper,
            Card::Estate,
            Card::Gold,
            Card::Silver,
        ],
    );
    let (mut g, mut k) = start(g);
    play(&mut g, Card::Smithy);
    k.update(&mut g);
    super::intrigue::trigger(&mut g, Card::Stash);
    k.update(&mut g);
    respond(&mut g, Response::Number(0));
    k.update(&mut g);
    assert!(hand_kinds(&g, P0).contains(&Card::Stash));
    let hand = k.known(&g, P1).zone(Zone::Hand(P0));
    assert_eq!((hand.known, hand.unknown), (count(&[(Card::Stash, 1)]), 2));
    assert_eq!(k.known(&g, P1).zone(Zone::Deck(P0)).unknown, 2);
}

// ---- Twin worlds: what a player knows can't depend on what they couldn't observe ----

fn names(g: &Game, cards: &[CardId]) -> Vec<&'static str> {
    let mut v: Vec<&'static str> = cards.iter().map(|&c| g.card(c).name()).collect();
    v.sort();
    v
}

/// What `v` can observe at a decision by the rules alone: the public log and state, the
/// cards they may look at, and their own decision. Card ids are left out.
fn observation(g: &Game, v: PlayerId, log_from: usize) -> String {
    let t = g.turn();
    let mut out = String::new();
    let _ = writeln!(out, "log {:?}", &g.log()[log_from.min(g.log().len())..]);
    let _ = writeln!(
        out,
        "turn {:?} {:?} a{} b{} c{} p{}",
        t.player, t.phase, t.actions, t.buys, t.coins, t.potions
    );
    for (i, pl) in g.players.iter().enumerate() {
        let p = PlayerId(i as u8);
        let _ = writeln!(
            out,
            "{p}: deck {} hand {} discard {} top {:?} play {:?} aside {}",
            pl.deck.len(),
            pl.hand.len(),
            pl.discard.len(),
            pl.discard.last().map(|&c| g.card(c)),
            names(g, &pl.in_play),
            pl.set_aside.len(),
        );
        if p == v {
            let _ = writeln!(out, "  hand {:?}", names(g, &pl.hand));
            let _ = writeln!(out, "  aside {:?}", names(g, &pl.set_aside));
        } else {
            let _ = writeln!(out, "  face up {:?}", names(g, &pl.face_up));
        }
        // Stash and Shadow cards show through their backs wherever they are.
        let backs = |cards: &[CardId]| {
            let marked = |c: &&CardId| {
                let card = g.printed(**c);
                card == Card::Stash || card.is(Types::SHADOW)
            };
            names(g, &cards.iter().filter(marked).copied().collect::<Vec<_>>())
        };
        let _ = writeln!(
            out,
            "  backs: deck {:?} hand {:?} discard {:?} aside {:?}",
            backs(&pl.deck),
            backs(&pl.hand),
            backs(&pl.discard),
            backs(&pl.set_aside)
        );
        for (&mat, cards) in &pl.mats {
            if mat != Mat::NativeVillage || p == v {
                let _ = writeln!(out, "  {mat:?} {:?}", names(g, cards));
            } else {
                let _ = writeln!(out, "  {mat:?} {}", cards.len());
            }
        }
        let _ = writeln!(
            out,
            "  vp{} co{} vi{} fa{} de{} ps{} {:?} {:?} {:?} {:?}",
            pl.vp_tokens,
            pl.coffers,
            pl.villagers,
            pl.favors,
            pl.debt,
            pl.pirate_ship,
            pl.projects,
            pl.states,
            pl.boons_kept,
            pl.tokens
        );
    }
    for pile in g.piles() {
        let top = pile
            .cards
            .last()
            .filter(|_| !pile.hidden)
            .map(|&c| g.card(c));
        let _ = writeln!(
            out,
            "pile {} x{} top {top:?} {} {} {}",
            pile.card,
            pile.cards.len(),
            pile.vp_tokens,
            pile.debt_tokens,
            pile.embargo
        );
    }
    let _ = writeln!(out, "trash {:?}", names(g, g.trash()));
    if let Some(d) = g.pending() {
        let _ = writeln!(
            out,
            "deciding {:?} {:?} {:?}",
            d.player, d.purpose, d.source
        );
        if d.player == v {
            let sig = |c: &CardId| format!("{} in {:?}", g.card(*c), g.zone(*c));
            let kind = match &d.kind {
                DecisionKind::Turn { phase, moves } => {
                    let mut m: Vec<String> = moves
                        .iter()
                        .map(|m| match m {
                            Move::Play(c) => format!("play {}", sig(c)),
                            m => format!("{m:?}"),
                        })
                        .collect();
                    m.sort();
                    format!("{phase:?} {m:?}")
                }
                DecisionKind::Cards {
                    candidates,
                    min,
                    max,
                    ordered,
                } => {
                    let mut c: Vec<String> = candidates.iter().map(sig).collect();
                    c.sort();
                    format!("{c:?} {min} {max} {ordered}")
                }
                k => format!("{k:?}"),
            };
            let _ = writeln!(out, "{kind}");
        }
    }
    out
}

/// What `v` knows, as text.
fn digest(g: &Game, k: &Knowledge, v: PlayerId) -> String {
    let known = k.known(g, v);
    let mut zones: Vec<String> = known
        .zones
        .iter()
        .map(|(z, zk)| format!("{z:?} {zk:?}"))
        .collect();
    zones.sort();
    format!(
        "zones {zones:#?}\nuncertain {:#?}\nowned {:?}",
        known.uncertain, known.owned
    )
}

/// `r`, an answer to `a`'s decision, as an answer to `b`'s: the same choice of cards by
/// name and zone.
fn translate(a: &Game, b: &Game, r: &Response) -> Option<Response> {
    let (da, db) = (a.pending()?, b.pending()?);
    if da.player != db.player {
        return None;
    }
    let sig = |g: &Game, c: CardId| (g.card(c), g.zone(c));
    let translated = match (r, &db.kind) {
        (Response::Move(Move::Play(c)), DecisionKind::Turn { moves, .. }) => {
            let want = sig(a, *c);
            if moves.contains(&Move::Play(*c)) && sig(b, *c) == want {
                r.clone()
            } else {
                moves.iter().find_map(|m| match m {
                    Move::Play(x) if sig(b, *x) == want => Some(Response::Move(*m)),
                    _ => None,
                })?
            }
        }
        (Response::Cards(chosen), DecisionKind::Cards { candidates, .. }) => {
            let mut left = candidates.clone();
            let mut out = vec![];
            for &c in chosen {
                let want = sig(a, c);
                let i = left
                    .iter()
                    .position(|&x| x == c && sig(b, x) == want)
                    .or_else(|| left.iter().position(|&x| sig(b, x) == want))?;
                out.push(left.remove(i));
            }
            Response::Cards(out)
        }
        _ => r.clone(),
    };
    db.validate(&translated).ok()?;
    Some(translated)
}

/// Answers and updates `k`, returning what each player watched happen meanwhile that the
/// log and the next decision don't show: every card movement (without the card, unless
/// they see it) and the cards they looked at.
fn respond_traced(g: &mut Game, k: &mut Knowledge, r: Response) -> Vec<String> {
    // Names, choices and moves are announced.
    let said = match &r {
        Response::Name(_) | Response::Options(_) | Response::Number(_) | Response::Pile(_) => {
            format!("{r:?}\n")
        }
        Response::Move(Move::Play(c)) => format!("play {}\n", g.card(*c)),
        Response::Move(m) => format!("{m:?}\n"),
        _ => String::new(),
    };
    let since = g.log().len();
    g.respond(r).unwrap();
    let mut seen = vec![said; g.num_players()];
    // Boons and Hexes are revealed and received with no card id.
    for e in &g.log()[since..] {
        if let Event::Revealed { card, .. } | Event::Received { card, .. } = e
            && matches!(card.kind(), Kind::Boon | Kind::Hex)
        {
            seen.iter_mut()
                .for_each(|out| out.push_str(&format!("{card}\n")));
        }
    }
    for entry in g.trace().unwrap_or_default() {
        match entry {
            Trace::Moved {
                card,
                from,
                to,
                top,
                bottom,
                under,
            } => {
                if from == to {
                    continue;
                }
                for (v, out) in seen.iter_mut().enumerate() {
                    let v = PlayerId(v as u8);
                    let mine = |z: &Zone| match *z {
                        Zone::Hand(p) | Zone::SetAside(p) | Zone::Mat(p, Mat::NativeVillage) => {
                            p == v
                        }
                        _ => false,
                    };
                    let public = |z: &Zone| match *z {
                        Zone::InPlay(_) | Zone::Trash => true,
                        Zone::Mat(_, mat) => mat != Mat::NativeVillage,
                        Zone::Pile(pile) => !g.pile(pile).hidden,
                        _ => false,
                    };
                    let what = if mine(from) || mine(to) || public(from) || public(to) {
                        g.printed(*card).name()
                    } else {
                        "a card"
                    };
                    // Whether it was the top card shows only for decks and piles.
                    let top = matches!(from, Zone::Deck(_) | Zone::Pile(_)) && *top;
                    let placement = if matches!(to, Zone::Deck(_)) {
                        format!(" bottom {bottom} under {under}")
                    } else {
                        String::new()
                    };
                    let _ = writeln!(out, "{what} {from:?} -> {to:?} top {top}{placement}");
                }
            }
            Trace::Positioned {
                player,
                bottom,
                under,
                ..
            } => {
                for out in &mut seen {
                    let zone = Zone::Deck(*player);
                    let _ = writeln!(
                        out,
                        "a card {zone:?} -> {zone:?} top false bottom {bottom} under {under}"
                    );
                }
            }
            Trace::LookedOver { player, cards } => {
                for card in cards {
                    let _ = writeln!(seen[player.index()], "looked at {}", g.printed(*card));
                }
            }
            Trace::Shown { card, to: None } => {
                for out in seen.iter_mut() {
                    let _ = writeln!(out, "shown {}", g.printed(*card));
                }
            }
            Trace::Shown { card, to: Some(p) } => {
                for (v, out) in seen.iter_mut().enumerate() {
                    let v = PlayerId(v as u8);
                    if *p == v {
                        let _ = writeln!(out, "looked at {}", g.printed(*card));
                    }
                }
            }
            _ => {}
        }
    }
    k.update(g);
    seen
}

/// Plays two worlds on, giving the same answers where possible. While a player has
/// observed the same things in both, they must know the same things in both.
fn twins_agree(
    mut a: (Game, Knowledge),
    mut b: (Game, Knowledge),
    mut same: Vec<bool>,
    log_from: usize,
    rng: &mut ChaCha8Rng,
) -> Result<(), String> {
    let mut history = String::new();
    for _ in 0..25 {
        for (v, same) in same.iter_mut().enumerate() {
            let p = PlayerId(v as u8);
            if !*same {
                continue;
            }
            if observation(&a.0, p, log_from) != observation(&b.0, p, log_from) {
                // The log says no more than the table and the moves they watched (which
                // of two public cards of one name moved is no news).
                let log =
                    |g: &Game| -> Vec<Event> { g.log()[log_from.min(g.log().len())..].to_vec() };
                if observation(&a.0, p, usize::MAX) == observation(&b.0, p, usize::MAX)
                    && log(&a.0) != log(&b.0)
                {
                    return Err(format!(
                        "{p} observed the same in both worlds but the log differs:\n{:?}\n{:?}\nAnswers since the split:\n{history}",
                        log(&a.0),
                        log(&b.0)
                    ));
                }
                *same = false;
                continue;
            }
            let (ka, kb) = (digest(&a.0, &a.1, p), digest(&b.0, &b.1, p));
            if ka != kb {
                let only = |x: &str, y: &str| -> String {
                    x.lines()
                        .filter(|l| !y.lines().any(|m| m == *l))
                        .collect::<Vec<_>>()
                        .join("\n")
                };
                let log = &a.0.log()[log_from.min(a.0.log().len())..];
                return Err(format!(
                    "{p} observed the same in both worlds (log since the split: {log:?})\nbut knows\n{}\nin one and\n{}\nin the other.\nAnswers since the split:\n{history}",
                    only(&ka, &kb),
                    only(&kb, &ka)
                ));
            }
        }
        if !same.iter().any(|&s| s) {
            break;
        }
        let (Some(da), Some(db)) = (a.0.pending().cloned(), b.0.pending().cloned()) else {
            break;
        };
        let mut chaos = Chaos(ChaCha8Rng::seed_from_u64(rng.random()));
        let ra = chaos.decide(&a.0.view(da.player), &da);
        let rb =
            translate(&a.0, &b.0, &ra).unwrap_or_else(|| chaos.decide(&b.0.view(db.player), &db));
        let _ = writeln!(history, "  {:?} {ra:?} / {:?} {rb:?}", da.player, db.player);
        let sa = respond_traced(&mut a.0, &mut a.1, ra);
        let sb = respond_traced(&mut b.0, &mut b.1, rb);
        for (same, (x, y)) in same.iter_mut().zip(sa.iter().zip(&sb)) {
            *same &= x == y;
        }
    }
    Ok(())
}

/// A random game in which every Kingdom card and landscape gets its turn (the removed
/// ones too), each player starting with one of every Kingdom card.
fn traced_random_game(seed: u64, players: u8) -> (Game, Knowledge) {
    let mut config = GameConfig::random(players, IMPLEMENTED, seed);
    config.turn_limit = Some(40);
    let implemented = |c: &&Card| IMPLEMENTED.contains(&c.set());
    let kingdom: Vec<Card> = Card::ALL
        .iter()
        .filter(implemented)
        .copied()
        .filter(|c| c.def().kingdom && !c.def().unimplemented)
        .collect();
    let landscape = |c: &Card| {
        matches!(
            c.kind(),
            Kind::Event | Kind::Landmark | Kind::Project | Kind::Way | Kind::Trait
        )
    };
    let landscapes: Vec<Card> = Card::ALL
        .iter()
        .filter(implemented)
        .copied()
        .filter(landscape)
        .collect();
    let forced = kingdom[seed as usize % kingdom.len()];
    if !config.kingdom.contains(&forced) {
        config.kingdom[0] = forced;
    }
    let forced = landscapes[seed as usize % landscapes.len()];
    if !config.landscapes.contains(&forced) {
        config
            .landscapes
            .retain(|l| l.kind() != Kind::Way || forced.kind() != Kind::Way);
        config.landscapes.truncate(1);
        config.landscapes.push(forced);
    }
    let mut g = setup_with(config);
    let mut extra: Vec<Card> = g
        .config()
        .kingdom
        .iter()
        .map(|&k| k.pile_members()[0])
        .collect();
    extra.push(Card::Gold);
    if g.supply_pile(Card::Potion).is_some() {
        extra.push(Card::Potion);
    }
    for p in 0..g.num_players() {
        for &card in &extra {
            g.create(card, Zone::Deck(PlayerId(p as u8)));
        }
    }
    start(g)
}

fn printed(g: &Game, cards: &[CardId]) -> CardCount {
    let mut count = CardCount::new();
    for &c in cards {
        *count.entry(g.printed(c)).or_default() += 1;
    }
    count
}

/// Hidden things that differ between two worlds (how the next shuffles come out, which
/// card a player privately picks) don't change what the others know. Nor is what everyone
/// watched forgotten: players know their own deck and discard pile and, every gain, trash
/// and return being public, which cards each player owns (until a Masquerade passes one).
#[test]
fn knowledge_depends_only_on_what_a_player_can_observe() {
    let mut failures = vec![];
    for seed in 0..fuzz_seeds(30) {
        let mut rng = ChaCha8Rng::seed_from_u64(seed ^ 0x7717);
        let players = rng.random_range(2..=4);
        let (mut g, mut k) = traced_random_game(seed, players);
        let passes = (0..g.card_count() as u32).any(|c| g.printed(CardId(c)) == Card::Masquerade);
        let mut agents: Vec<Chaos> = (0..g.num_players())
            .map(|p| Chaos(ChaCha8Rng::seed_from_u64(seed * 10 + p as u64)))
            .collect();
        let mut step = 0;
        'game: while let Some(d) = g.pending().cloned() {
            for v in 0..g.num_players() {
                let viewer = PlayerId(v as u8);
                let known = k.known(&g, viewer);
                for (p, owned) in known.owned.iter().enumerate() {
                    let cards: Vec<CardId> = g.players[p].owned().collect();
                    let truth = (printed(&g, &cards), 0);
                    if !passes && *owned != truth {
                        failures.push(format!(
                            "seed {seed} step {step}: {viewer} thinks P{} owns {owned:?}, truly {truth:?}",
                            p + 1
                        ));
                        break 'game;
                    }
                }
                // Players know their own deck and discard pile.
                for zone in [Zone::Deck(viewer), Zone::Discard(viewer)] {
                    let pl = g.player(viewer);
                    let cards = if zone == Zone::Deck(viewer) {
                        &pl.deck
                    } else {
                        &pl.discard
                    };
                    let (mine, truth) = (known.zone(zone), printed(&g, cards));
                    if (&mine.known, mine.unknown) != (&truth, 0) {
                        failures.push(format!(
                            "seed {seed} step {step}: {viewer} knows {mine:?} of their {zone:?}, truly {truth:?}"
                        ));
                        break 'game;
                    }
                }
            }
            let r = agents[d.player.index()].decide(&g.view(d.player), &d);
            if rng.random_bool(0.2) {
                let log_from = g.log().len();
                // A world where the hidden future differs.
                let mut a = (g.clone(), k.clone());
                let mut b = (g.clone(), k.clone());
                b.0.rng = ChaCha8Rng::seed_from_u64(rng.random());
                let mut same = vec![true; g.num_players()];
                // Or one where the deciding player picks something else.
                if rng.random_bool(0.5) {
                    let other = agents[d.player.index()].decide(&g.view(d.player), &d);
                    if other != r {
                        b = (g.clone(), k.clone());
                        let sa = respond_traced(&mut a.0, &mut a.1, r.clone());
                        let sb = respond_traced(&mut b.0, &mut b.1, other);
                        for (same, (x, y)) in same.iter_mut().zip(sa.iter().zip(&sb)) {
                            *same &= x == y;
                        }
                        same[d.player.index()] = false;
                    }
                }
                if let Err(e) = twins_agree(a, b, same, log_from, &mut rng) {
                    failures.push(format!(
                        "seed {seed} step {step} ({:?} {:?} {:?}): {e}",
                        d.player, d.purpose, d.source
                    ));
                    break 'game;
                }
            }
            respond_traced(&mut g, &mut k, r);
            step += 1;
            assert!(step < 100_000, "seed {seed}: game does not end");
        }
    }
    assert!(
        failures.is_empty(),
        "{} games:\n{}",
        failures.len(),
        failures.join("\n=====\n")
    );
}

/// Tracing changes nothing, and a traced game and its knowledge survive serialization.
#[test]
fn traced_games_play_like_untraced_ones_and_survive_serialization() {
    for seed in 0..fuzz_seeds(20) {
        let mut rng = ChaCha8Rng::seed_from_u64(seed ^ 0x5eed);
        let players = rng.random_range(2..=6);
        let mut config = GameConfig::random(players, IMPLEMENTED, seed);
        config.turn_limit = Some(30);
        let mut plain = Game::new(config.clone()).unwrap();
        let mut traced = Game::new_traced(config.clone()).unwrap();
        let mut resumed = Game::new_traced(config).unwrap();
        let mut k = Knowledge::new(players as usize);
        let mut k_resumed = k.clone();
        k.update(&mut traced);
        k_resumed.update(&mut resumed);
        let mut chaos = Chaos(ChaCha8Rng::seed_from_u64(seed));
        let mut step = 0;
        while let Some(d) = plain.pending().cloned() {
            assert_eq!(traced.pending(), Some(&d), "seed {seed} step {step}");
            assert_eq!(traced.log(), plain.log(), "seed {seed} step {step}");
            let r = chaos.decide(&plain.view(d.player), &d);
            plain.respond(r.clone()).unwrap();
            traced.respond(r.clone()).unwrap();
            resumed.respond(r).unwrap();
            assert_eq!(traced.trace(), resumed.trace(), "seed {seed} step {step}");
            if step % 5 == 0 {
                let json = serde_json::to_string(&(&resumed, &k_resumed)).unwrap();
                (resumed, k_resumed) = serde_json::from_str(&json).unwrap();
            }
            k.update(&mut traced);
            k_resumed.update(&mut resumed);
            step += 1;
        }
        assert_eq!(traced.log(), plain.log(), "seed {seed}");
        for v in 0..players {
            let v = PlayerId(v);
            assert_eq!(
                k.known(&traced, v),
                k_resumed.known(&resumed, v),
                "seed {seed}"
            );
        }
    }
}

#[test]
fn private_revealed_put_back_order_does_not_reach_other_seats() {
    let mut g = setup(&[Card::Rabble, Card::Village]);
    hand(&mut g, P0, &[Card::Rabble]);
    deck(&mut g, P0, &[Card::Copper; 10]);
    hand(&mut g, P1, &[Card::Village]);
    deck(
        &mut g,
        P1,
        &[
            Card::Estate,
            Card::Duchy,
            Card::Province,
            Card::Silver,
            Card::Silver,
        ],
    );
    let (mut g, mut k) = start(g);
    play(&mut g, Card::Rabble);
    k.update(&mut g);
    let revealed = candidates(&g);
    assert_eq!(g.pending().unwrap().player, P1);
    let world = |order: [usize; 3]| {
        let (mut g, mut k) = (g.clone(), k.clone());
        choose(&mut g, &order.map(|i| revealed[i]));
        k.update(&mut g);
        while g.pending().unwrap().player == P0 {
            end_phase(&mut g);
            k.update(&mut g);
        }
        play(&mut g, Card::Village);
        k.update(&mut g);
        (g, k)
    };
    let ((ga, ka), (gb, kb)) = (world([0, 1, 2]), world([1, 2, 0]));
    // The same public log, and P1 drew a different card in each world.
    assert_eq!(ga.log(), gb.log());
    assert_ne!(hand_kinds(&ga, P1), hand_kinds(&gb, P1));
    let (a, b) = (ka.known(&ga, P0), kb.known(&gb, P0));
    assert_eq!(a.zone(Zone::Hand(P1)), b.zone(Zone::Hand(P1)));
    assert_eq!(a.zone(Zone::Deck(P1)), b.zone(Zone::Deck(P1)));
}

#[test]
fn revealed_cards_split_between_hand_and_deck_stay_known() {
    let mut g = setup(&[Card::Seer]);
    hand(&mut g, P0, &[Card::Seer]);
    deck(
        &mut g,
        P0,
        &[
            Card::Gold,
            Card::Estate,
            Card::Silver,
            Card::Copper,
            Card::Duchy,
        ],
    );
    let (mut g, mut k) = start(g);
    play(&mut g, Card::Seer);
    k.update(&mut g);
    // +1 Card drew the Gold; Estate and Silver went to the hand, the Copper back.
    assert_eq!(deck_kinds(&g, P0), [Card::Copper, Card::Duchy]);
    assert!(logged_in_order(
        &g,
        &[moved(
            P0,
            Zone::Looking(P0),
            Zone::Deck(P0),
            &[Card::Copper],
            1
        )]
    ));
    let theirs = k.known(&g, P1);
    assert_eq!(
        theirs.zone(Zone::Deck(P0)).known,
        count(&[(Card::Copper, 1)]),
        "{theirs:#?}"
    );
    assert_eq!(
        theirs.zone(Zone::Hand(P0)).known,
        count(&[(Card::Estate, 1), (Card::Silver, 1)])
    );
}

#[test]
fn looking_over_a_shuffle_preserves_known_cards_left_above_it() {
    for choice in [None, Some(Card::StarChart), Some(Card::OrderOfAstrologers)] {
        let mut g = if choice == Some(Card::OrderOfAstrologers) {
            setup_with(GameConfig {
                landscapes: vec![Card::OrderOfAstrologers],
                colonies: Some(false),
                shelters: Some(false),
                ..GameConfig::new(2, vec![Card::Underling, Card::Smithy], 7)
            })
        } else {
            setup(&[Card::Smithy])
        };
        if choice == Some(Card::StarChart) {
            g.players[0].projects.push(Card::StarChart);
        }
        g.players[0].favors = 2;
        hand(&mut g, P0, &[Card::Smithy]);
        deck(&mut g, P0, &[]);
        let shuffled = discard(
            &mut g,
            P0,
            &[
                Card::Gold,
                Card::Estate,
                Card::Duchy,
                Card::Province,
                Card::Curse,
                Card::Copper,
            ],
        );
        let (mut g, mut k) = start(g);
        g.create(Card::Silver, Zone::Deck(P0));
        k.update(&mut g);
        assert_eq!(
            k.known(&g, P1).zone(Zone::Deck(P0)).known,
            count(&[(Card::Silver, 1)])
        );
        play(&mut g, Card::Smithy);
        k.update(&mut g);
        if choice.is_some() {
            respond(&mut g, Response::option(Opt::Trigger(0)));
            k.update(&mut g);
            choose(&mut g, &[shuffled[0]]);
            k.update(&mut g);
        }
        assert_eq!(hand_kinds(&g, P0)[0], Card::Silver);
        let theirs = k.known(&g, P1);
        assert_eq!(
            theirs.zone(Zone::Hand(P0)).known.get(&Card::Silver),
            Some(&1),
            "shuffle choice {choice:?}: {:?} {:?}",
            theirs.zone(Zone::Hand(P0)),
            theirs.uncertain
        );
    }
}

/// Twin worlds from the pending decision: one where the hidden future differs (how the
/// next shuffles come out), or one where the deciding player answers otherwise. A player
/// who observed the same in both must know the same, and read the same log.
pub(super) fn check_hidden_twins(
    g: &Game,
    k: &Knowledge,
    rng: &mut ChaCha8Rng,
) -> Result<(), String> {
    let Some(d) = g.pending() else { return Ok(()) };
    let mut chaos = Chaos(ChaCha8Rng::seed_from_u64(rng.random()));
    let mut a = (g.clone(), k.clone());
    let mut b = a.clone();
    b.0.rng = ChaCha8Rng::seed_from_u64(rng.random());
    let mut same = vec![true; g.num_players()];
    let (r, other) = (
        chaos.decide(&g.view(d.player), d),
        chaos.decide(&g.view(d.player), d),
    );
    if rng.random_bool(0.5) && other != r {
        b = a.clone();
        let sa = respond_traced(&mut a.0, &mut a.1, r);
        let sb = respond_traced(&mut b.0, &mut b.1, other);
        for (same, (x, y)) in same.iter_mut().zip(sa.iter().zip(&sb)) {
            *same &= x == y;
        }
        same[d.player.index()] = false;
    }
    twins_agree(a, b, same, g.log().len(), rng)
}

pub(super) fn check_revealed_order_twins(
    g: &Game,
    k: &Knowledge,
    rng: &mut ChaCha8Rng,
) -> Result<(), String> {
    let Some(d) = g.pending() else { return Ok(()) };
    let DecisionKind::Cards {
        candidates,
        max,
        ordered: true,
        ..
    } = &d.kind
    else {
        return Ok(());
    };
    if *max < 2
        || !candidates.iter().any(|&c| {
            matches!(g.zone(c), Zone::Looking(p)
        if g.player(p).face_up.contains(&c))
        })
    {
        return Ok(());
    }
    let cards: Vec<_> = candidates.iter().copied().take(*max).collect();
    let mut reversed = cards.clone();
    reversed.reverse();
    let mut a = (g.clone(), k.clone());
    let mut b = a.clone();
    let sa = respond_traced(&mut a.0, &mut a.1, Response::Cards(cards));
    let sb = respond_traced(&mut b.0, &mut b.1, Response::Cards(reversed));
    let same = sa
        .iter()
        .zip(&sb)
        .enumerate()
        .map(|(v, (a, b))| PlayerId(v as u8) != d.player && a == b)
        .collect();
    twins_agree(a, b, same, g.log().len(), rng)
}
