use super::*;

#[test]
fn throne_room_plays_twice() {
    let mut g = setup(&[Card::ThroneRoom, Card::Smithy]);
    hand(&mut g, P0, &[Card::ThroneRoom, Card::Smithy, Card::Estate]);
    deck(&mut g, P0, &[Card::Copper; 10]);
    g.start();
    play(&mut g, Card::ThroneRoom);
    choose_kinds(&mut g, &[Card::Smithy]);
    assert_eq!(g.hand(P0).len(), 1 + 6);
    assert_eq!(kinds(&g, g.in_play(P0)), [Card::ThroneRoom, Card::Smithy]);
    assert_eq!(
        g.turn().played,
        [Card::ThroneRoom, Card::Smithy, Card::Smithy]
    );
}

#[test]
fn moat_blocks_militia() {
    let mut g = setup(&[Card::Militia, Card::Moat]);
    hand(&mut g, P0, &[Card::Militia]);
    hand(
        &mut g,
        P1,
        &[
            Card::Moat,
            Card::Copper,
            Card::Copper,
            Card::Estate,
            Card::Estate,
        ],
    );
    g.start();
    play(&mut g, Card::Militia);
    let d = g.pending().unwrap();
    assert_eq!((d.player, d.purpose), (P1, Purpose::Trigger));
    respond(&mut g, Response::option(Opt::Trigger(0)));
    assert_eq!(g.hand(P1).len(), 5);
    assert_eq!(g.turn().coins, 2);
    assert_eq!(g.pending().unwrap().player, P0);
}

#[test]
fn militia_without_moat_discards_to_three() {
    let mut g = setup(&[Card::Militia, Card::Moat]);
    hand(&mut g, P0, &[Card::Militia]);
    hand(
        &mut g,
        P1,
        &[
            Card::Moat,
            Card::Copper,
            Card::Copper,
            Card::Estate,
            Card::Estate,
        ],
    );
    g.start();
    play(&mut g, Card::Militia);
    no(&mut g);
    assert_eq!(
        (g.pending().unwrap().player, purpose(&g)),
        (P1, Purpose::Discard)
    );
    choose_kinds(&mut g, &[Card::Estate, Card::Estate]);
    assert_eq!(hand_kinds(&g, P1), [Card::Moat, Card::Copper, Card::Copper]);
}

#[test]
fn merchant_boosts_first_silver_only() {
    let mut g = setup(&[Card::Merchant]);
    hand(
        &mut g,
        P0,
        &[Card::Merchant, Card::Silver, Card::Silver, Card::Copper],
    );
    deck(&mut g, P0, &[Card::Estate; 5]);
    g.start();
    play(&mut g, Card::Merchant);
    play(&mut g, Card::Silver);
    play(&mut g, Card::Silver);
    play(&mut g, Card::Copper);
    assert_eq!(g.turn().coins, 2 + 1 + 2 + 1);
}

#[test]
fn merchant_after_a_silver_does_nothing() {
    let mut g = setup(&[Card::Merchant, Card::Village]);
    hand(&mut g, P0, &[Card::Village, Card::Merchant, Card::Silver]);
    deck(&mut g, P0, &[Card::Estate, Card::Silver, Card::Estate]);
    g.start();
    play(&mut g, Card::Village);
    play(&mut g, Card::Merchant);
    play(&mut g, Card::Silver);
    play(&mut g, Card::Silver);
    assert_eq!(g.turn().coins, 2 + 1 + 2);
}

#[test]
fn library_sets_aside_actions_and_discards_them() {
    let mut g = setup(&[Card::Library, Card::Village]);
    hand(&mut g, P0, &[Card::Library, Card::Copper, Card::Copper]);
    deck(
        &mut g,
        P0,
        &[
            Card::Village,
            Card::Copper,
            Card::Copper,
            Card::Copper,
            Card::Copper,
            Card::Gold,
            Card::Silver,
        ],
    );
    g.start();
    play(&mut g, Card::Library);
    assert_eq!(purpose(&g), Purpose::SetAside);
    yes(&mut g);
    assert_eq!(g.hand(P0).len(), 7);
    assert_eq!(kinds(&g, &g.player(P0).discard), [Card::Village]);
    assert_eq!(deck_kinds(&g, P0), [Card::Silver]);
}

#[test]
fn sentry_trashes_and_reorders() {
    let mut g = setup(&[Card::Sentry]);
    hand(&mut g, P0, &[Card::Sentry]);
    deck(
        &mut g,
        P0,
        &[
            Card::Copper,
            Card::Curse,
            Card::Gold,
            Card::Estate,
            Card::Silver,
        ],
    );
    g.start();
    play(&mut g, Card::Sentry);
    assert_eq!(kinds(&g, &candidates(&g)), [Card::Curse, Card::Gold]);
    choose_kinds(&mut g, &[Card::Curse]);
    choose(&mut g, &[]);
    assert_eq!(kinds(&g, g.trash()).last(), Some(&Card::Curse));
    assert_eq!(deck_kinds(&g, P0), [Card::Gold, Card::Estate, Card::Silver]);
}

#[test]
fn vassal_shuffles_and_plays_the_discarded_action() {
    let mut g = setup(&[Card::Vassal, Card::Smithy]);
    hand(&mut g, P0, &[Card::Vassal]);
    deck(&mut g, P0, &[]);
    discard(&mut g, P0, &[Card::Smithy]);
    g.start();
    play(&mut g, Card::Vassal);
    let d = g.pending().unwrap();
    assert_eq!((d.purpose, d.source), (Purpose::Play, Some(Card::Vassal)));
    yes(&mut g);
    assert!(g.log().contains(&Event::Shuffled { player: P0 }));
    assert_eq!(kinds(&g, g.in_play(P0)), [Card::Vassal, Card::Smithy]);
}

#[test]
fn gardens_count_owned_cards() {
    let mut g = setup(&[Card::Gardens]);
    g.start();
    for _ in 0..19 {
        g.create(Card::Gardens, Zone::Discard(P0));
    }
    // 10 starting cards + 19 Gardens = 29 cards: 3 Estates + 19 * 2.
    assert_eq!(g.score(P0), 3 + 19 * 2);
}

#[test]
fn feast_trashes_itself_and_gains() {
    let mut g = setup(&[Card::Feast, Card::Laboratory]);
    hand(&mut g, P0, &[Card::Feast]);
    g.start();
    play(&mut g, Card::Feast);
    choose_pile(&mut g, Card::Laboratory);
    assert_eq!(kinds(&g, g.trash()).last(), Some(&Card::Feast));
    assert_eq!(kinds(&g, &g.player(P0).discard), [Card::Laboratory]);
}

#[test]
fn adventurer_digs_for_two_treasures() {
    let mut g = setup(&[Card::Adventurer]);
    hand(&mut g, P0, &[Card::Adventurer]);
    deck(
        &mut g,
        P0,
        &[
            Card::Estate,
            Card::Silver,
            Card::Estate,
            Card::Gold,
            Card::Copper,
        ],
    );
    g.start();
    play(&mut g, Card::Adventurer);
    assert_eq!(hand_kinds(&g, P0), [Card::Silver, Card::Gold]);
    assert_eq!(
        kinds(&g, &g.player(P0).discard),
        [Card::Estate, Card::Estate]
    );
    assert_eq!(deck_kinds(&g, P0), [Card::Copper]);
}
