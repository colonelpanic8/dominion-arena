use super::prosperity::{next_turn, play_from};
use super::*;
use crate::game::Zone;
use std::collections::BTreeMap;

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

#[test]
fn the_possessor_sees_and_remembers_the_possessed_hand() {
    let mut g = setup(&[Card::Possession, Card::Village]);
    hand(&mut g, P0, &[Card::Possession]);
    deck(&mut g, P0, &[Card::Copper; 10]);
    hand(&mut g, P1, &[Card::Village, Card::Gold, Card::Duchy]);
    deck(&mut g, P1, &[Card::Estate; 10]);
    let (mut g, mut k) = start(g);
    play(&mut g, Card::Possession);
    k.update(&mut g);
    next_turn(&mut g);
    k.update(&mut g);
    assert_eq!(g.turn().controller, P0);
    let seen = k.known(&g, P0).zone(Zone::Hand(P1));
    assert_eq!(
        seen.known,
        count(&[(Card::Village, 1), (Card::Gold, 1), (Card::Duchy, 1)])
    );
    play_from(&mut g, P1, Card::Village);
    k.update(&mut g);
    next_turn(&mut g);
    k.update(&mut g);
    // P1's own turn: P0 no longer sees the hand, but remembers the cards discarded in
    // the possessed turn's Clean-up and the hand they drew then.
    assert_eq!(g.turn().controller, P1);
    let hand = k.known(&g, P0).zone(Zone::Hand(P1));
    assert_eq!(hand.known, count(&[(Card::Estate, 5)]));
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

fn check_knowledge(g: &Game, k: &Knowledge, step: usize) {
    for v in 0..g.num_players() {
        let v = PlayerId(v as u8);
        for group in k.groups(v) {
            if let Some(ids) = group.ids {
                let actual: CardCount = group.cards.iter().fold(CardCount::new(), |mut m, &c| {
                    *m.entry(g.printed(c)).or_default() += 1;
                    m
                });
                assert_eq!(ids, &actual, "{v}'s group {:?}", group.cards);
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
