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
    let revealed = Event::Revealed {
        player: P1,
        card: Card::Moat,
    };
    let unaffected = Event::Unaffected {
        player: P1,
        attack: Card::Militia,
        by: Card::Moat,
    };
    assert!(logged_in_order(&g, &[revealed, unaffected]));
}

/// Two Moats are one option; once one is revealed the other can't matter and isn't offered.
#[test]
fn two_moats_are_one_option() {
    let mut g = setup(&[Card::Militia, Card::Moat]);
    hand(&mut g, P0, &[Card::Militia]);
    hand(
        &mut g,
        P1,
        &[
            Card::Moat,
            Card::Moat,
            Card::Copper,
            Card::Estate,
            Card::Estate,
        ],
    );
    g.start();
    play(&mut g, Card::Militia);
    assert_eq!(
        super::intrigue::trigger_labels(&g),
        ["Moat: reveal it to be unaffected (optional)", "Decline"]
    );
    respond(&mut g, Response::option(Opt::Trigger(0)));
    assert_eq!(g.hand(P1).len(), 5);
    assert_eq!(g.pending().unwrap().player, P0);
}

#[test]
fn declining_two_moats_declines_both() {
    let mut g = setup(&[Card::Militia, Card::Moat]);
    hand(&mut g, P0, &[Card::Militia]);
    hand(
        &mut g,
        P1,
        &[
            Card::Moat,
            Card::Moat,
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

/// Trashing an Overgrown Estate draws a card in the middle of Sentry: the cards Sentry
/// looks at are off the deck, so it draws the next one down.
#[test]
fn a_draw_in_the_middle_of_sentry_takes_the_card_under_the_ones_it_looks_at() {
    let mut g = setup(&[Card::Sentry]);
    hand(&mut g, P0, &[Card::Sentry]);
    deck(
        &mut g,
        P0,
        &[
            Card::Copper,
            Card::OvergrownEstate,
            Card::Gold,
            Card::Silver,
        ],
    );
    g.start();
    play(&mut g, Card::Sentry);
    choose_kinds(&mut g, &[Card::OvergrownEstate]);
    // Discard nothing and put back whatever Sentry still has.
    while g.pending().is_some_and(|d| d.source == Some(Card::Sentry)) {
        let keep = match purpose(&g) {
            Purpose::Discard => vec![],
            _ => candidates(&g),
        };
        choose(&mut g, &keep);
    }
    assert_eq!(hand_kinds(&g, P0), [Card::Copper, Card::Silver]);
    assert_eq!(deck_kinds(&g, P0), [Card::Gold]);
    assert!(g.player(P0).discard.is_empty());
}

/// P1's Silver is trashed: Market Square gains a Gold, which lets Sheepdog draw 2. The
/// cards Bandit revealed are off the deck, so those are the two under them; Bandit then
/// discards the Estate.
#[test]
fn a_draw_in_the_middle_of_bandit_takes_the_cards_under_the_revealed_ones() {
    let mut g = setup(&[Card::Bandit, Card::MarketSquare, Card::Sheepdog]);
    hand(&mut g, P0, &[Card::Bandit]);
    hand(&mut g, P1, &[Card::MarketSquare, Card::Sheepdog]);
    deck(
        &mut g,
        P1,
        &[Card::Silver, Card::Estate, Card::Copper, Card::Copper],
    );
    discard(&mut g, P1, &[]);
    g.start();
    play(&mut g, Card::Bandit);
    super::intrigue::trigger(&mut g, Card::MarketSquare);
    super::intrigue::trigger(&mut g, Card::Sheepdog);
    assert_eq!(hand_kinds(&g, P1), [Card::Copper, Card::Copper]);
    assert_eq!(
        kinds(&g, &g.player(P1).discard),
        [Card::MarketSquare, Card::Gold, Card::Estate]
    );
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

/// Vassal discards the top card without revealing it, so a Patron gives no Coffers.
#[test]
fn vassal_does_not_reveal_the_card_it_discards() {
    let mut g = setup(&[Card::Vassal, Card::Patron]);
    hand(&mut g, P0, &[Card::Vassal]);
    deck(&mut g, P0, &[Card::Patron, Card::Copper]);
    g.start();
    play(&mut g, Card::Vassal);
    assert_eq!(purpose(&g), Purpose::Play);
    assert!(g.log().contains(&Event::Discarded {
        player: P0,
        card: Card::Patron
    }));
    assert_eq!(g.player(P0).coffers, 0);
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

/// Remodel reads the trashed card's cost when it gains, after what the trash set off: a
/// Market Square discarded for a Gold makes the trashed Destrier cost $5, so Remodel gains
/// up to $7, not a Province.
#[test]
fn remodel_reads_the_trashed_cards_cost_when_it_gains() {
    let mut g = setup(&[Card::Remodel, Card::Destrier, Card::MarketSquare]);
    hand(
        &mut g,
        P0,
        &[Card::Remodel, Card::Destrier, Card::MarketSquare],
    );
    g.start();
    play(&mut g, Card::Remodel);
    choose_kinds(&mut g, &[Card::Destrier]);
    super::intrigue::trigger(&mut g, Card::MarketSquare);
    assert_eq!(g.cost(Card::Destrier), Cost::coins(5));
    let piles = super::prosperity::pile_choices(&g);
    assert!(piles.contains(&Card::Gold), "{piles:?}");
    assert!(!piles.contains(&Card::Province), "{piles:?}");
}
