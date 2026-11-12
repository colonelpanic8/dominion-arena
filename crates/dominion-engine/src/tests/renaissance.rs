use super::*;
use rand::seq::SliceRandom;

fn turn_starts(g: &Game) -> usize {
    g.log()
        .iter()
        .filter(|e| matches!(e, Event::TurnStart { .. }))
        .count()
}

/// Ends phases until the next turn starts.
fn next_turn(g: &mut Game) {
    let n = turn_starts(g);
    while turn_starts(g) == n {
        match &g.pending().expect("a decision is pending").kind {
            DecisionKind::Turn { .. } => end_phase(g),
            k => panic!("unexpected decision {k:?}"),
        }
    }
}

/// Resolves pending triggers in the order offered.
fn resolve_triggers(g: &mut Game) {
    while g.pending().is_some_and(|d| d.purpose == Purpose::Trigger) {
        respond(g, Response::option(Opt::Trigger(0)));
    }
}

/// Accepts the pending trigger of `card`.
fn trigger(g: &mut Game, card: Card) {
    let d = g.pending().expect("a decision is pending");
    assert_eq!(d.purpose, Purpose::Trigger, "{d:?}");
    let DecisionKind::Choose {
        options, labels, ..
    } = &d.kind
    else {
        panic!("expected a Choose decision")
    };
    let i = labels
        .iter()
        .position(|l| l.starts_with(card.name()))
        .unwrap_or_else(|| panic!("no {card} trigger among {labels:?}"));
    respond(g, Response::option(options[i]));
}

/// Declines the pending optional triggers.
fn decline(g: &mut Game) {
    assert_eq!(purpose(g), Purpose::Trigger);
    no(g);
}

fn mode(g: &mut Game, i: u8) {
    respond(g, Response::option(Opt::Mode(i)));
}

fn phase(g: &Game) -> Option<Phase> {
    match g.pending().map(|d| &d.kind) {
        Some(DecisionKind::Turn { phase, .. }) => Some(*phase),
        _ => None,
    }
}

fn to_buy(g: &mut Game) {
    if phase(g) == Some(Phase::Action) {
        end_phase(g);
    }
}

fn in_play_kinds(g: &Game, p: PlayerId) -> Vec<Card> {
    kinds(g, g.in_play(p))
}

fn discard_kinds(g: &Game, p: PlayerId) -> Vec<Card> {
    kinds(g, &g.player(p).discard)
}

fn trash_kinds(g: &Game) -> Vec<Card> {
    kinds(g, g.trash())
}

fn give_project(g: &mut Game, p: PlayerId, project: Card) {
    g.players[p.index()].projects.push(project);
}

fn moves(g: &Game) -> Vec<Move> {
    match &g.pending().expect("a decision is pending").kind {
        DecisionKind::Turn { moves, .. } => moves.clone(),
        k => panic!("expected a Turn decision, got {k:?}"),
    }
}

/// Empties the trash so tests can look at what gets trashed.
fn clear_trash(g: &mut Game) {
    for c in std::mem::take(&mut g.trash) {
        g.zones[c.index()] = Zone::SetAside(P1);
        g.players[1].set_aside.push(c);
    }
}

// ---- Kingdom cards ----

#[test]
fn acting_troupe_gives_four_villagers_and_trashes_itself() {
    let mut g = setup(&[Card::ActingTroupe]);
    hand(&mut g, P0, &[Card::ActingTroupe]);
    g.start();
    clear_trash(&mut g);
    play(&mut g, Card::ActingTroupe);
    assert_eq!(g.player(P0).villagers, 4);
    assert_eq!(trash_kinds(&g), [Card::ActingTroupe]);
    assert!(moves(&g).contains(&Move::Villager));
}

#[test]
fn throne_room_acting_troupe_gives_eight_villagers() {
    let mut g = setup(&[Card::ActingTroupe, Card::ThroneRoom]);
    hand(&mut g, P0, &[Card::ThroneRoom, Card::ActingTroupe]);
    g.start();
    play(&mut g, Card::ThroneRoom);
    choose_kinds(&mut g, &[Card::ActingTroupe]);
    assert_eq!(g.player(P0).villagers, 8);
}

#[test]
fn border_guard_keeps_one_and_discards_the_other() {
    let mut g = setup(&[Card::BorderGuard]);
    hand(&mut g, P0, &[Card::BorderGuard]);
    deck(&mut g, P0, &[Card::Gold, Card::Estate, Card::Copper]);
    g.start();
    play(&mut g, Card::BorderGuard);
    assert_eq!(g.turn().actions, 1);
    choose_kinds(&mut g, &[Card::Gold]);
    assert_eq!(hand_kinds(&g, P0), [Card::Gold]);
    assert_eq!(discard_kinds(&g, P0), [Card::Estate]);
    assert_eq!(g.artifact_holder(Card::Lantern), None);
    assert_eq!(g.artifact_holder(Card::Horn), None);
}

#[test]
fn border_guard_revealing_two_actions_takes_the_lantern_then_reveals_three() {
    let mut g = setup(&[Card::BorderGuard, Card::Smithy]);
    hand(&mut g, P0, &[Card::BorderGuard, Card::BorderGuard]);
    deck(
        &mut g,
        P0,
        &[
            Card::Smithy,
            Card::Moat,
            Card::Smithy,
            Card::Moat,
            Card::Moat,
        ],
    );
    g.start();
    play(&mut g, Card::BorderGuard);
    choose_kinds(&mut g, &[Card::Smithy]);
    mode(&mut g, 0);
    assert_eq!(g.artifact_holder(Card::Lantern), Some(P0));
    play(&mut g, Card::BorderGuard);
    assert_eq!(candidates(&g).len(), 3);
    choose_kinds(&mut g, &[Card::Smithy]);
    assert_eq!(discard_kinds(&g, P0), [Card::Moat, Card::Moat, Card::Moat]);
    mode(&mut g, 1);
    assert_eq!(g.artifact_holder(Card::Horn), Some(P0));
}

#[test]
fn border_guard_with_the_lantern_needs_three_actions() {
    let mut g = setup(&[Card::BorderGuard, Card::Smithy]);
    hand(&mut g, P0, &[Card::BorderGuard]);
    deck(&mut g, P0, &[Card::Smithy, Card::Smithy, Card::Copper]);
    g.artifacts.iter_mut().for_each(|a| {
        if a.0 == Card::Lantern {
            a.1 = Some(P0);
        }
    });
    g.start();
    play(&mut g, Card::BorderGuard);
    choose_kinds(&mut g, &[Card::Smithy]);
    assert_eq!(g.artifact_holder(Card::Horn), None);
    assert_eq!(phase(&g), Some(Phase::Action));
}

#[test]
fn border_guard_revealing_one_card_takes_nothing() {
    let mut g = setup(&[Card::BorderGuard, Card::Smithy]);
    hand(&mut g, P0, &[Card::BorderGuard]);
    deck(&mut g, P0, &[Card::Smithy]);
    g.start();
    play(&mut g, Card::BorderGuard);
    assert_eq!(hand_kinds(&g, P0), [Card::Smithy]);
    assert_eq!(phase(&g), Some(Phase::Action));
    assert_eq!(g.artifact_holder(Card::Lantern), None);
}

#[test]
fn horn_puts_a_border_guard_on_the_deck_once_per_turn() {
    let mut g = setup(&[Card::BorderGuard, Card::Village]);
    hand(
        &mut g,
        P0,
        &[Card::Village, Card::BorderGuard, Card::BorderGuard],
    );
    deck(&mut g, P0, &[Card::Copper; 15]);
    g.artifacts.iter_mut().for_each(|a| {
        if a.0 == Card::Horn {
            a.1 = Some(P0);
        }
    });
    g.start();
    play(&mut g, Card::Village);
    play(&mut g, Card::BorderGuard);
    play(&mut g, Card::BorderGuard);
    to_buy(&mut g);
    end_phase(&mut g);
    trigger(&mut g, Card::Horn);
    // Only once per turn: the other Border Guard is discarded.
    assert_eq!(
        hand_kinds(&g, P0)
            .iter()
            .filter(|&&c| c == Card::BorderGuard)
            .count(),
        1
    );
    assert!(discard_kinds(&g, P0).contains(&Card::BorderGuard));
}

/// Horn has no "on your turn": a Border Guard played on another player's turn is put back
/// when it is discarded from play in that turn's Clean-up.
#[test]
fn horn_works_on_another_players_turn() {
    let mut g = setup(&[Card::BorderGuard]);
    hand(&mut g, P0, &[Card::Copper]);
    hand(&mut g, P1, &[Card::BorderGuard]);
    deck(&mut g, P1, &[Card::Copper; 7]);
    g.artifacts.iter_mut().for_each(|a| {
        if a.0 == Card::Horn {
            a.1 = Some(P1);
        }
    });
    g.start();
    play_out_of_turn(&mut g, P1, Card::BorderGuard);
    if g.pending().is_some_and(|d| d.player == P1) {
        choose_kinds(&mut g, &[Card::Copper]);
    }
    end_phase(&mut g);
    trigger(&mut g, Card::Horn);
    let top = *g.player(P1).deck.last().unwrap();
    assert_eq!(g.card(top), Card::BorderGuard);
}

#[test]
fn patron_revealed_by_border_guard_in_the_action_phase_gives_coffers() {
    let mut g = setup(&[Card::BorderGuard, Card::Patron]);
    hand(&mut g, P0, &[Card::BorderGuard]);
    deck(&mut g, P0, &[Card::Patron, Card::Patron]);
    g.start();
    play(&mut g, Card::BorderGuard);
    assert_eq!(g.player(P0).coffers, 2);
}

#[test]
fn patron_revealed_at_the_start_of_a_turn_gives_coffers() {
    let mut g = setup(&[Card::Patron, Card::Smithy]);
    give_project(&mut g, P0, Card::Piazza);
    hand(&mut g, P0, &[]);
    deck(&mut g, P0, &[Card::Patron, Card::Copper]);
    g.start();
    // The start of a turn is part of its Action phase. Piazza then played it.
    assert_eq!(g.player(P0).coffers, 1);
    assert_eq!(in_play_kinds(&g, P0), [Card::Patron]);
    assert_eq!(g.player(P0).villagers, 1);
}

#[test]
fn patron_revealed_outside_an_action_phase_gives_nothing() {
    let mut g = setup(&[Card::Patron, Card::Venture]);
    hand(&mut g, P0, &[Card::Venture]);
    deck(&mut g, P0, &[Card::Patron, Card::Copper]);
    g.start();
    play(&mut g, Card::Venture);
    assert_eq!(g.turn().coins, 2);
    assert_eq!(g.player(P0).coffers, 0);
}

#[test]
fn patron_in_a_hand_revealed_to_bureaucrat_gives_coffers() {
    let mut g = setup(&[Card::Patron, Card::Bureaucrat]);
    hand(&mut g, P0, &[Card::Bureaucrat]);
    hand(&mut g, P1, &[Card::Patron, Card::Copper]);
    g.start();
    play(&mut g, Card::Bureaucrat);
    assert_eq!(g.player(P1).coffers, 1);
}

#[test]
fn patron_revealed_from_the_black_market_deck_gives_coffers() {
    let mut g = super::promos::setup_black_market();
    let bm = g.ext.promos.black_market.expect("a Black Market deck");
    g.create(Card::Patron, Zone::Pile(bm));
    hand(&mut g, P0, &[Card::BlackMarket]);
    g.start();
    play(&mut g, Card::BlackMarket);
    assert_eq!(g.player(P0).coffers, 1);
}

#[test]
fn patron_gives_a_villager_and_two_coins() {
    let mut g = setup(&[Card::Patron]);
    hand(&mut g, P0, &[Card::Patron]);
    g.start();
    play(&mut g, Card::Patron);
    assert_eq!(g.player(P0).villagers, 1);
    assert_eq!(g.turn().coins, 2);
}

#[test]
fn cargo_ship_sets_aside_a_gained_card_for_next_turn() {
    let mut g = setup(&[Card::CargoShip, Card::Smithy]);
    hand(&mut g, P0, &[Card::CargoShip, Card::Silver]);
    deck(&mut g, P0, &[Card::Copper; 15]);
    g.start();
    play(&mut g, Card::CargoShip);
    to_buy(&mut g);
    play(&mut g, Card::Silver);
    assert_eq!(g.turn().coins, 4);
    buy(&mut g, Card::Smithy);
    trigger(&mut g, Card::CargoShip);
    assert_eq!(kinds(&g, &g.player(P0).set_aside), [Card::Smithy]);
    // Set aside face up.
    let v = g.view(P1);
    let smithy = v.player(P0).set_aside();
    assert_eq!(smithy.len(), 1);
    assert_eq!(v.card(smithy[0]), Some(Card::Smithy));
    // P0's turn ended with no Buys left; P1's turn now.
    assert_eq!(in_play_kinds(&g, P0), [Card::CargoShip]);
    next_turn(&mut g);
    assert!(hand_kinds(&g, P0).contains(&Card::Smithy));
    assert_eq!(g.hand(P0).len(), 6);
    next_turn(&mut g);
    assert!(g.in_play(P0).is_empty());
}

/// Each Cargo Ship says what keeps it in play (the two effects of one played by a Throne
/// Room are one option), and once one has set the card aside the others aren't offered.
#[test]
fn cargo_ships_say_what_keeps_them_in_play() {
    let mut g = setup(&[Card::CargoShip, Card::ThroneRoom, Card::Village]);
    hand(
        &mut g,
        P0,
        &[
            Card::Village,
            Card::ThroneRoom,
            Card::CargoShip,
            Card::CargoShip,
        ],
    );
    deck(&mut g, P0, &[Card::Copper; 10]);
    g.start();
    play(&mut g, Card::Village);
    play(&mut g, Card::ThroneRoom);
    choose_kinds(&mut g, &[Card::CargoShip]);
    play(&mut g, Card::CargoShip);
    to_buy(&mut g);
    buy(&mut g, Card::Silver);
    let DecisionKind::Choose { labels, .. } = &g.pending().unwrap().kind else {
        panic!("{:?}", g.pending());
    };
    assert_eq!(
        labels,
        &[
            "Cargo Ship played by Throne Room: set Silver aside on Cargo Ship (optional)",
            "Cargo Ship: set Silver aside on Cargo Ship (optional)",
            "Decline all"
        ]
    );
    respond(&mut g, Response::option(Opt::Trigger(1)));
    assert_eq!(g.turn().player, P1);
    assert_eq!(in_play_kinds(&g, P0), [Card::CargoShip]);
}

/// Two Cargo Ships played one after the other are one option: no one can tell which of
/// them stays in play.
#[test]
fn separately_played_cargo_ships_are_one_option() {
    let mut g = setup(&[Card::CargoShip, Card::Village]);
    let cards = [Card::Village, Card::CargoShip, Card::CargoShip];
    hand(&mut g, P0, &cards);
    deck(&mut g, P0, &[Card::Copper; 10]);
    g.start();
    play(&mut g, Card::Village);
    play(&mut g, Card::CargoShip);
    play(&mut g, Card::CargoShip);
    to_buy(&mut g);
    buy(&mut g, Card::Silver);
    assert_eq!(
        super::intrigue::trigger_labels(&g),
        [
            "Cargo Ship: set Silver aside on Cargo Ship (optional)",
            "Decline"
        ]
    );
    trigger(&mut g, Card::CargoShip);
    assert_eq!(kinds(&g, &g.player(P0).set_aside), [Card::Silver]);
    assert_eq!(g.turn().player, P1);
    assert_eq!(in_play_kinds(&g, P0), [Card::CargoShip]);
}

/// "You may use Improve to trash it during Clean-up and gain a different card. If you do,
/// you may then still use Cargo Ship's ability ... the card stays set-aside for the rest
/// of the game" (wiki, Cargo Ship). Beside a Cargo Ship still in play, the options say
/// which is which.
#[test]
fn a_trashed_cargo_ship_is_told_apart_from_one_in_play() {
    let mut g = setup(&[Card::CargoShip, Card::Improve, Card::Village, Card::Smithy]);
    let cards = [
        Card::Village,
        Card::Village,
        Card::CargoShip,
        Card::CargoShip,
        Card::Improve,
    ];
    hand(&mut g, P0, &cards);
    deck(&mut g, P0, &[Card::Copper; 12]);
    g.start();
    for card in cards {
        play(&mut g, card);
    }
    end_phases(&mut g);
    trigger(&mut g, Card::Improve);
    choose_kinds(&mut g, &[Card::CargoShip]);
    // Smithy is the only card costing exactly $4.
    assert_eq!(
        super::intrigue::trigger_labels(&g),
        [
            "Cargo Ship (in the trash): set Smithy aside on Cargo Ship (optional)",
            "Cargo Ship: set Smithy aside on Cargo Ship (optional)",
            "Decline all"
        ]
    );
}

#[test]
fn cargo_ship_that_sets_nothing_aside_is_discarded() {
    let mut g = setup(&[Card::CargoShip, Card::Smithy]);
    hand(&mut g, P0, &[Card::CargoShip, Card::Silver]);
    deck(&mut g, P0, &[Card::Copper; 15]);
    g.start();
    play(&mut g, Card::CargoShip);
    to_buy(&mut g);
    play(&mut g, Card::Silver);
    buy(&mut g, Card::Smithy);
    decline(&mut g);
    next_turn(&mut g);
    assert!(g.in_play(P0).is_empty());
    assert!(g.lasting().is_empty());
}

#[test]
fn throne_room_cargo_ship_sets_aside_two_cards_and_both_stay() {
    let mut g = setup(&[Card::CargoShip, Card::ThroneRoom, Card::Village]);
    hand(&mut g, P0, &[Card::ThroneRoom, Card::CargoShip]);
    deck(&mut g, P0, &[Card::Copper; 15]);
    g.start();
    play(&mut g, Card::ThroneRoom);
    choose_kinds(&mut g, &[Card::CargoShip]);
    assert_eq!(g.turn().coins, 4);
    to_buy(&mut g);
    buy(&mut g, Card::Village);
    assert_eq!(
        super::intrigue::trigger_labels(&g),
        [
            "Cargo Ship played by Throne Room: set Village aside on Cargo Ship (optional)",
            "Decline"
        ]
    );
    trigger(&mut g, Card::CargoShip);
    assert_eq!(kinds(&g, &g.player(P0).set_aside), [Card::Village]);
    assert_eq!(in_play_kinds(&g, P0), [Card::ThroneRoom, Card::CargoShip]);
    next_turn(&mut g);
    assert!(hand_kinds(&g, P0).contains(&Card::Village));
}

#[test]
fn experiment_returns_to_its_pile_and_gains_come_in_pairs() {
    let mut g = setup(&[Card::Experiment]);
    hand(&mut g, P0, &[Card::Experiment, Card::Silver]);
    deck(&mut g, P0, &[Card::Copper; 10]);
    g.start();
    let pile = g.supply_pile(Card::Experiment).unwrap();
    play(&mut g, Card::Experiment);
    assert_eq!(g.hand(P0).len(), 3);
    assert_eq!(g.turn().actions, 1);
    assert_eq!(g.pile(pile).cards.len(), 11);
    assert!(g.in_play(P0).is_empty());
    to_buy(&mut g);
    play(&mut g, Card::Silver);
    play(&mut g, Card::Copper);
    buy(&mut g, Card::Experiment);
    assert_eq!(g.count_owned(P0, Card::Experiment), 2);
    assert_eq!(g.pile(pile).cards.len(), 9);
}

#[test]
fn experiment_gained_to_hand_brings_one_to_the_discard_pile() {
    let mut g = setup(&[Card::Experiment, Card::Sculptor]);
    hand(&mut g, P0, &[Card::Sculptor]);
    g.start();
    play(&mut g, Card::Sculptor);
    choose_pile(&mut g, Card::Experiment);
    assert_eq!(hand_kinds(&g, P0), [Card::Experiment]);
    assert_eq!(discard_kinds(&g, P0), [Card::Experiment]);
    assert_eq!(g.player(P0).villagers, 0);
}

#[test]
fn improve_trashes_an_action_leaving_play_for_one_costing_exactly_one_more() {
    let mut g = setup(&[Card::Improve, Card::Village, Card::Smithy, Card::Hideout]);
    hand(&mut g, P0, &[Card::Village, Card::Improve]);
    deck(&mut g, P0, &[Card::Copper; 15]);
    g.start();
    play(&mut g, Card::Village);
    play(&mut g, Card::Improve);
    assert_eq!(g.turn().coins, 2);
    to_buy(&mut g);
    end_phase(&mut g);
    trigger(&mut g, Card::Improve);
    choose_kinds(&mut g, &[Card::Village]);
    // Exactly $4: Smithy, Silver or Improve... not Village.
    let DecisionKind::Pile { piles, .. } = &g.pending().unwrap().kind else {
        panic!()
    };
    let names: Vec<Card> = piles.iter().map(|&p| g.pile(p).card).collect();
    assert!(names.contains(&Card::Smithy) && !names.contains(&Card::Village));
    assert!(names.iter().all(|&c| g.cost(c) == Cost::coins(4)));
    choose_pile(&mut g, Card::Smithy);
    assert!(discard_kinds(&g, P0).contains(&Card::Smithy));
    assert!(g.trash().iter().any(|&c| g.card(c) == Card::Village));
}

#[test]
fn improve_can_trash_itself() {
    let mut g = setup(&[Card::Improve, Card::Smithy]);
    hand(&mut g, P0, &[Card::Improve]);
    deck(&mut g, P0, &[Card::Copper; 15]);
    g.start();
    play(&mut g, Card::Improve);
    to_buy(&mut g);
    end_phase(&mut g);
    // Improve is the only candidate and Smithy the only $4 card: both forced.
    trigger(&mut g, Card::Improve);
    assert_eq!(g.count_owned(P0, Card::Improve), 0);
    assert_eq!(g.count_owned(P0, Card::Smithy), 1);
}

#[test]
fn improve_played_on_another_players_turn_trashes_in_that_turns_clean_up() {
    let mut g = setup(&[Card::Improve, Card::Smithy]);
    hand(&mut g, P1, &[Card::Improve]);
    g.start();
    play_out_of_turn(&mut g, P1, Card::Improve);
    to_buy(&mut g);
    end_phase(&mut g);
    assert_eq!(g.pending().unwrap().player, P1);
    trigger(&mut g, Card::Improve);
    assert_eq!(g.count_owned(P1, Card::Improve), 0);
    assert_eq!(g.count_owned(P1, Card::Smithy), 1);
}

#[test]
fn improve_cannot_trash_a_duration_staying_in_play() {
    let mut g = setup(&[Card::Improve, Card::Research]);
    hand(&mut g, P0, &[Card::Research, Card::Improve, Card::Silver]);
    deck(&mut g, P0, &[Card::Copper; 15]);
    g.start();
    play(&mut g, Card::Research);
    choose_kinds(&mut g, &[Card::Silver]);
    play(&mut g, Card::Improve);
    to_buy(&mut g);
    end_phase(&mut g);
    trigger(&mut g, Card::Improve);
    // Only Improve: Research stays in play for next turn.
    assert_eq!(g.count_owned(P0, Card::Improve), 0);
    assert_eq!(in_play_kinds(&g, P0), [Card::Research]);
}

#[test]
fn flag_bearer_takes_the_flag_when_gained_or_trashed() {
    let mut g = setup(&[Card::FlagBearer, Card::Chapel]);
    hand(&mut g, P0, &[Card::Gold, Card::Copper, Card::Copper]);
    deck(&mut g, P0, &[Card::Copper; 15]);
    hand(&mut g, P1, &[Card::Chapel, Card::FlagBearer]);
    g.start();
    to_buy(&mut g);
    play(&mut g, Card::Gold);
    play(&mut g, Card::Copper);
    buy(&mut g, Card::FlagBearer);
    assert_eq!(g.artifact_holder(Card::Flag), Some(P0));
    // The Flag: 6 cards.
    assert_eq!(g.hand(P0).len(), 6);
    play(&mut g, Card::Chapel);
    choose_kinds(&mut g, &[Card::FlagBearer]);
    assert_eq!(g.artifact_holder(Card::Flag), Some(P1));
}

#[test]
fn flag_with_outpost_draws_four() {
    let mut g = setup(&[Card::FlagBearer, Card::Outpost]);
    hand(&mut g, P0, &[Card::Outpost]);
    deck(&mut g, P0, &[Card::Copper; 15]);
    g.artifacts.iter_mut().for_each(|a| {
        if a.0 == Card::Flag {
            a.1 = Some(P0);
        }
    });
    g.start();
    play(&mut g, Card::Outpost);
    next_turn(&mut g);
    assert_eq!(g.turn().player, P0);
    assert_eq!(g.hand(P0).len(), 4);
}

#[test]
fn hideout_trashing_a_victory_card_gains_a_curse() {
    let mut g = setup(&[Card::Hideout]);
    hand(&mut g, P0, &[Card::Hideout, Card::Estate]);
    deck(&mut g, P0, &[Card::Copper; 5]);
    g.start();
    play(&mut g, Card::Hideout);
    assert_eq!(g.turn().actions, 2);
    choose_kinds(&mut g, &[Card::Estate]);
    assert_eq!(discard_kinds(&g, P0), [Card::Curse]);
}

#[test]
fn hideout_trashing_a_curse_gains_nothing() {
    let mut g = setup(&[Card::Hideout]);
    hand(&mut g, P0, &[Card::Hideout, Card::Curse]);
    deck(&mut g, P0, &[Card::Copper; 5]);
    g.start();
    play(&mut g, Card::Hideout);
    choose_kinds(&mut g, &[Card::Curse]);
    assert!(g.player(P0).discard.is_empty());
}

#[test]
fn inventor_gains_then_lowers_costs_cumulatively() {
    let mut g = setup(&[Card::Inventor, Card::Village, Card::Smithy]);
    hand(&mut g, P0, &[Card::Village, Card::Inventor, Card::Inventor]);
    deck(&mut g, P0, &[Card::Copper; 10]);
    g.start();
    play(&mut g, Card::Village);
    play(&mut g, Card::Inventor);
    choose_pile(&mut g, Card::Smithy);
    assert_eq!(g.cost(Card::Province), Cost::coins(7));
    play(&mut g, Card::Inventor);
    // The first Inventor's reduction applies: a Duchy costs $4.
    choose_pile(&mut g, Card::Duchy);
    assert_eq!(g.cost(Card::Province), Cost::coins(6));
    assert_eq!(g.cost(Card::Copper), Cost::coins(0));
    next_turn(&mut g);
    assert_eq!(g.cost(Card::Province), Cost::coins(8));
}

#[test]
fn lackeys_gives_two_villagers_when_gained() {
    let mut g = setup(&[Card::Lackeys]);
    hand(&mut g, P0, &[Card::Copper, Card::Copper]);
    g.start();
    to_buy(&mut g);
    play(&mut g, Card::Copper);
    play(&mut g, Card::Copper);
    buy(&mut g, Card::Lackeys);
    assert_eq!(g.player(P0).villagers, 2);
}

#[test]
fn mountain_village_takes_a_card_from_the_discard_pile_or_draws() {
    let mut g = setup(&[Card::MountainVillage]);
    hand(&mut g, P0, &[Card::MountainVillage, Card::MountainVillage]);
    deck(&mut g, P0, &[Card::Silver, Card::Copper]);
    discard(&mut g, P0, &[]);
    g.start();
    play(&mut g, Card::MountainVillage);
    assert_eq!(hand_kinds(&g, P0), [Card::MountainVillage, Card::Silver]);
    discard(&mut g, P0, &[Card::Gold, Card::Estate]);
    play(&mut g, Card::MountainVillage);
    choose_kinds(&mut g, &[Card::Gold]);
    assert_eq!(hand_kinds(&g, P0), [Card::Silver, Card::Gold]);
    assert_eq!(g.turn().actions, 3);
}

#[test]
fn old_witch_curses_and_lets_victims_trash_a_curse() {
    let mut g = setup(&[Card::OldWitch]);
    hand(&mut g, P0, &[Card::OldWitch]);
    deck(&mut g, P0, &[Card::Copper; 5]);
    hand(&mut g, P1, &[Card::Curse, Card::Copper]);
    g.start();
    play(&mut g, Card::OldWitch);
    assert_eq!(g.hand(P0).len(), 3);
    assert_eq!(g.pending().unwrap().player, P1);
    choose_kinds(&mut g, &[Card::Curse]);
    assert_eq!(discard_kinds(&g, P1), [Card::Curse]);
    assert_eq!(hand_kinds(&g, P1), [Card::Copper]);
}

#[test]
fn moat_blocks_old_witch_entirely() {
    let mut g = setup(&[Card::OldWitch, Card::Moat]);
    hand(&mut g, P0, &[Card::OldWitch]);
    deck(&mut g, P0, &[Card::Copper; 5]);
    hand(&mut g, P1, &[Card::Moat, Card::Curse]);
    g.start();
    play(&mut g, Card::OldWitch);
    trigger(&mut g, Card::Moat);
    assert!(g.player(P1).discard.is_empty());
    assert_eq!(hand_kinds(&g, P1), [Card::Moat, Card::Curse]);
}

#[test]
fn priest_gives_two_per_later_trash_this_turn() {
    let mut g = setup(&[Card::Priest, Card::Village, Card::Chapel]);
    hand(
        &mut g,
        P0,
        &[
            Card::Village,
            Card::Village,
            Card::Priest,
            Card::Priest,
            Card::Copper,
            Card::Copper,
            Card::Chapel,
            Card::Estate,
            Card::Estate,
        ],
    );
    deck(&mut g, P0, &[Card::Gold; 10]);
    g.start();
    play(&mut g, Card::Village);
    play(&mut g, Card::Village);
    play(&mut g, Card::Priest);
    choose_kinds(&mut g, &[Card::Copper]);
    assert_eq!(g.turn().coins, 2);
    play(&mut g, Card::Priest);
    choose_kinds(&mut g, &[Card::Copper]);
    assert_eq!(g.turn().coins, 6);
    play(&mut g, Card::Chapel);
    choose_kinds(&mut g, &[Card::Estate, Card::Estate]);
    assert_eq!(g.turn().coins, 14);
    next_turn(&mut g);
    assert!(g.lasting().is_empty());
}

#[test]
fn recruiter_gives_a_villager_per_coin_of_the_trashed_card() {
    let mut g = setup(&[Card::Recruiter]);
    hand(&mut g, P0, &[Card::Recruiter, Card::Silver]);
    deck(&mut g, P0, &[Card::Copper; 5]);
    g.start();
    play(&mut g, Card::Recruiter);
    choose_kinds(&mut g, &[Card::Silver]);
    assert_eq!(g.player(P0).villagers, 3);
}

#[test]
fn research_sets_aside_a_card_per_coin_for_next_turn() {
    let mut g = setup(&[Card::Research]);
    hand(&mut g, P0, &[Card::Research, Card::Silver]);
    deck(
        &mut g,
        P0,
        &[
            Card::Gold,
            Card::Gold,
            Card::Gold,
            Card::Copper,
            Card::Copper,
            Card::Copper,
            Card::Copper,
            Card::Copper,
            Card::Copper,
            Card::Copper,
            Card::Copper,
            Card::Copper,
            Card::Copper,
        ],
    );
    g.start();
    play(&mut g, Card::Research);
    assert_eq!(
        kinds(&g, &g.player(P0).set_aside),
        [Card::Gold, Card::Gold, Card::Gold]
    );
    let aside = g.player(P0).set_aside[0];
    assert_eq!(g.view(P1).card(aside), None);
    next_turn(&mut g);
    assert_eq!(in_play_kinds(&g, P0), [Card::Research]);
    next_turn(&mut g);
    assert_eq!(g.hand(P0).len(), 8);
    assert_eq!(
        hand_kinds(&g, P0)
            .iter()
            .filter(|&&c| c == Card::Gold)
            .count(),
        3
    );
}

#[test]
fn research_trashing_a_copper_is_not_a_duration() {
    let mut g = setup(&[Card::Research]);
    hand(&mut g, P0, &[Card::Research, Card::Copper]);
    deck(&mut g, P0, &[Card::Copper; 12]);
    g.start();
    play(&mut g, Card::Research);
    next_turn(&mut g);
    assert!(g.in_play(P0).is_empty());
    assert!(g.lasting().is_empty());
}

#[test]
fn scepter_gives_two_or_replays_an_action_played_this_turn() {
    let mut g = setup(&[Card::Scepter, Card::Smithy]);
    hand(&mut g, P0, &[Card::Smithy, Card::Scepter, Card::Scepter]);
    deck(&mut g, P0, &[Card::Copper; 10]);
    g.start();
    play(&mut g, Card::Smithy);
    to_buy(&mut g);
    play(&mut g, Card::Scepter);
    mode(&mut g, 1);
    assert_eq!(g.hand(P0).len(), 7);
    play(&mut g, Card::Scepter);
    mode(&mut g, 0);
    assert_eq!(g.turn().coins, 2);
}

#[test]
fn scepter_cannot_replay_a_duration_from_an_earlier_turn() {
    let mut g = setup(&[Card::Scepter, Card::Research]);
    hand(&mut g, P0, &[Card::Research, Card::Gold]);
    deck(&mut g, P0, &[Card::Copper; 4]);
    g.start();
    play(&mut g, Card::Research);
    next_turn(&mut g);
    hand(&mut g, P0, &[Card::Scepter]);
    next_turn(&mut g);
    assert_eq!(in_play_kinds(&g, P0), [Card::Research]);
    to_buy(&mut g);
    play(&mut g, Card::Scepter);
    mode(&mut g, 1);
    // Nothing to replay: the Research was played last turn.
    assert_eq!(phase(&g), Some(Phase::Buy));
}

#[test]
fn scepter_replaying_a_duration_stays_in_play_with_it() {
    let mut g = setup(&[Card::Scepter, Card::CargoShip, Card::Village]);
    hand(&mut g, P0, &[Card::CargoShip, Card::Scepter]);
    deck(&mut g, P0, &[Card::Copper; 15]);
    g.start();
    play(&mut g, Card::CargoShip);
    to_buy(&mut g);
    play(&mut g, Card::Scepter);
    mode(&mut g, 1);
    assert_eq!(g.turn().coins, 4);
    buy(&mut g, Card::Village);
    // Two Cargo Ship abilities: set aside with the replay's.
    respond(&mut g, Response::option(Opt::Trigger(1)));
    next_turn(&mut g);
    assert_eq!(in_play_kinds(&g, P0), [Card::CargoShip, Card::Scepter]);
}

#[test]
fn scepter_cannot_replay_a_card_that_left_play_and_came_back() {
    let mut g = setup(&[Card::Scepter, Card::Duplicate, Card::Swashbuckler]);
    g.artifacts.iter_mut().for_each(|a| {
        if a.0 == Card::TreasureChest {
            a.1 = Some(P0);
        }
    });
    hand(&mut g, P0, &[Card::Duplicate, Card::Scepter]);
    g.start();
    play(&mut g, Card::Duplicate);
    // The Treasure Chest's Gold; call the Duplicate into play for another.
    trigger(&mut g, Card::Duplicate);
    assert_eq!(in_play_kinds(&g, P0), [Card::Duplicate]);
    play(&mut g, Card::Scepter);
    mode(&mut g, 1);
    assert_eq!(in_play_kinds(&g, P0), [Card::Duplicate, Card::Scepter]);
}

/// "Scepter can replay an Action that isn't finished resolving, such as a Storyteller
/// that played Scepter in the first place" (wiki, Scepter): on another player's turn too,
/// "this turn" being that turn.
#[test]
fn scepter_played_on_another_players_turn_replays_an_action_played_then() {
    let mut g = setup(&[
        Card::Scepter,
        Card::Storyteller,
        Card::Ambassador,
        Card::Gondola,
    ]);
    hand(&mut g, P0, &[Card::Ambassador, Card::Gondola]);
    hand(
        &mut g,
        P1,
        &[Card::Storyteller, Card::Scepter, Card::Copper],
    );
    g.start();
    play_on_another_players_turn(&mut g);
    choose_kinds(&mut g, &[Card::Scepter]);
    mode(&mut g, 1);
    assert_eq!(g.turn().played_count(P1, Card::Storyteller), 2);
}

#[test]
fn scholar_discards_the_hand_and_draws_seven() {
    let mut g = setup(&[Card::Scholar]);
    hand(&mut g, P0, &[Card::Scholar, Card::Estate, Card::Estate]);
    deck(&mut g, P0, &[Card::Copper; 10]);
    g.start();
    play(&mut g, Card::Scholar);
    assert_eq!(hand_kinds(&g, P0), [Card::Copper; 7]);
    assert_eq!(discard_kinds(&g, P0), [Card::Estate, Card::Estate]);
}

#[test]
fn sculptor_gains_to_hand_and_a_treasure_gives_a_villager() {
    let mut g = setup(&[Card::Sculptor]);
    hand(&mut g, P0, &[Card::Sculptor]);
    g.start();
    play(&mut g, Card::Sculptor);
    choose_pile(&mut g, Card::Silver);
    assert_eq!(hand_kinds(&g, P0), [Card::Silver]);
    assert_eq!(g.player(P0).villagers, 1);
}

#[test]
fn seer_takes_cards_costing_two_to_four_and_puts_back_the_rest() {
    let mut g = setup(&[Card::Seer]);
    hand(&mut g, P0, &[Card::Seer]);
    deck(
        &mut g,
        P0,
        &[
            Card::Copper,
            Card::Silver,
            Card::Gold,
            Card::Province,
            Card::Estate,
        ],
    );
    g.start();
    play(&mut g, Card::Seer);
    assert_eq!(hand_kinds(&g, P0), [Card::Copper, Card::Silver]);
    choose_kinds(&mut g, &[Card::Province, Card::Gold]);
    assert_eq!(
        deck_kinds(&g, P0),
        [Card::Province, Card::Gold, Card::Estate]
    );
}

#[test]
fn seer_ignores_potion_costs() {
    let mut g = setup(&[Card::Seer]);
    hand(&mut g, P0, &[Card::Seer]);
    deck(
        &mut g,
        P0,
        &[Card::Copper, Card::Apprentice, Card::Familiar, Card::Estate],
    );
    g.start();
    play(&mut g, Card::Seer);
    // Apprentice ($5) and Familiar ($3P) stay; Estate costs $2.
    assert_eq!(hand_kinds(&g, P0), [Card::Copper, Card::Estate]);
    let rest = candidates(&g);
    choose(&mut g, &rest);
    assert_eq!(g.player(P0).deck.len(), 2);
}

#[test]
fn silk_merchant_gives_coffers_and_a_villager_when_gained_or_trashed() {
    let mut g = setup(&[Card::SilkMerchant, Card::Chapel]);
    hand(&mut g, P0, &[Card::Silver, Card::Silver]);
    hand(&mut g, P1, &[Card::Chapel, Card::SilkMerchant]);
    g.start();
    to_buy(&mut g);
    play(&mut g, Card::Silver);
    play(&mut g, Card::Silver);
    buy(&mut g, Card::SilkMerchant);
    assert_eq!((g.player(P0).coffers, g.player(P0).villagers), (1, 1));
    let took = |counter| Event::Counter {
        player: Some(P0),
        pile: None,
        counter,
        delta: 1,
    };
    let order = [
        Event::Resolved {
            player: P0,
            card: Card::SilkMerchant,
        },
        took(Counter::Coffers),
        took(Counter::Villagers),
    ];
    assert!(logged_in_order(&g, &order));
    next_turn(&mut g);
    play(&mut g, Card::Chapel);
    choose_kinds(&mut g, &[Card::SilkMerchant]);
    assert_eq!((g.player(P1).coffers, g.player(P1).villagers), (1, 1));
}

#[test]
fn spices_gives_two_coffers_when_gained() {
    let mut g = setup(&[Card::Spices]);
    hand(&mut g, P0, &[Card::Gold, Card::Silver]);
    g.start();
    to_buy(&mut g);
    play(&mut g, Card::Gold);
    play(&mut g, Card::Silver);
    buy(&mut g, Card::Spices);
    assert_eq!(g.player(P0).coffers, 2);
}

#[test]
fn spices_and_ducat_give_buys_and_ducat_may_trash_a_copper_when_gained() {
    let mut g = setup(&[Card::Spices, Card::Ducat]);
    let copper = hand(&mut g, P0, &[Card::Spices, Card::Ducat, Card::Copper])[2];
    g.start();
    to_buy(&mut g);
    play(&mut g, Card::Spices);
    play(&mut g, Card::Ducat);
    assert_eq!((g.turn().coins, g.turn().buys), (2, 3));
    assert_eq!(g.player(P0).coffers, 1);
    buy(&mut g, Card::Ducat);
    trigger(&mut g, Card::Ducat);
    assert!(g.hand(P0).is_empty());
    assert_eq!(g.zone(copper), Zone::Trash);
}

#[test]
fn swashbuckler_gives_coffers_and_the_treasure_chest_with_a_discard_pile() {
    let mut g = setup(&[Card::Swashbuckler]);
    hand(&mut g, P0, &[Card::Swashbuckler]);
    deck(&mut g, P0, &[Card::Copper; 5]);
    discard(&mut g, P0, &[Card::Estate]);
    g.players[0].coffers = 3;
    g.start();
    play(&mut g, Card::Swashbuckler);
    assert_eq!(g.player(P0).coffers, 4);
    assert_eq!(g.artifact_holder(Card::TreasureChest), Some(P0));
    to_buy(&mut g);
    // The Treasure Chest gains a Gold at the start of the Buy phase.
    assert_eq!(discard_kinds(&g, P0), [Card::Estate, Card::Gold]);
}

#[test]
fn swashbuckler_does_nothing_more_after_a_shuffle_empties_the_discard_pile() {
    let mut g = setup(&[Card::Swashbuckler]);
    hand(&mut g, P0, &[Card::Swashbuckler]);
    deck(&mut g, P0, &[Card::Copper; 2]);
    discard(&mut g, P0, &[Card::Estate; 3]);
    g.players[0].coffers = 5;
    g.start();
    play(&mut g, Card::Swashbuckler);
    assert_eq!(g.player(P0).coffers, 5);
    assert_eq!(g.artifact_holder(Card::TreasureChest), None);
}

#[test]
fn treasurer_trashes_regains_or_takes_the_key() {
    let mut g = setup(&[Card::Treasurer, Card::Village]);
    hand(
        &mut g,
        P0,
        &[
            Card::Village,
            Card::Village,
            Card::Treasurer,
            Card::Treasurer,
            Card::Treasurer,
            Card::Gold,
        ],
    );
    deck(&mut g, P0, &[Card::Copper; 10]);
    g.start();
    play(&mut g, Card::Village);
    play(&mut g, Card::Village);
    play(&mut g, Card::Treasurer);
    assert_eq!(g.turn().coins, 3);
    mode(&mut g, 0);
    choose_kinds(&mut g, &[Card::Gold]);
    assert!(g.hand(P0).iter().all(|&c| g.card(c) != Card::Gold));
    play(&mut g, Card::Treasurer);
    mode(&mut g, 1);
    choose_kinds(&mut g, &[Card::Gold]);
    assert!(hand_kinds(&g, P0).contains(&Card::Gold));
    play(&mut g, Card::Treasurer);
    mode(&mut g, 2);
    assert_eq!(g.artifact_holder(Card::Key), Some(P0));
    next_turn(&mut g);
    next_turn(&mut g);
    assert_eq!(g.turn().coins, 1);
}

#[test]
fn villain_makes_players_with_five_cards_discard_one_costing_two_or_more() {
    let mut g = GameConfig::new(3, vec![Card::Villain], 7);
    g.colonies = Some(false);
    g.shelters = Some(false);
    let mut g = setup_with(g);
    hand(&mut g, P0, &[Card::Villain]);
    hand(
        &mut g,
        P1,
        &[
            Card::Copper,
            Card::Copper,
            Card::Estate,
            Card::Silver,
            Card::Copper,
        ],
    );
    hand(&mut g, P2, &[Card::Copper; 5]);
    g.start();
    play(&mut g, Card::Villain);
    assert_eq!(g.player(P0).coffers, 2);
    assert_eq!(g.pending().unwrap().player, P1);
    assert_eq!(kinds(&g, &candidates(&g)), [Card::Estate, Card::Silver]);
    choose_kinds(&mut g, &[Card::Estate]);
    assert_eq!(discard_kinds(&g, P1), [Card::Estate]);
    assert!(g.log().contains(&Event::RevealedHand {
        player: P2,
        cards: vec![Card::Copper; 5]
    }));
}

// ---- Projects ----

#[test]
fn projects_cost_a_buy_and_each_player_gets_two_cubes() {
    let mut config = GameConfig::new(2, vec![Card::Smithy], 7);
    config.colonies = Some(false);
    config.shelters = Some(false);
    config.landscapes = vec![Card::Fair, Card::Barracks, Card::Cathedral];
    let mut g = setup_with(config);
    hand(&mut g, P0, &[Card::Gold; 5]);
    g.players[0].coffers = 5;
    g.start();
    to_buy(&mut g);
    for _ in 0..5 {
        play(&mut g, Card::Gold);
    }
    g.turn.buys = 4;
    respond(&mut g, Response::Move(Move::BuyLandscape(Card::Fair)));
    assert_eq!(g.player(P0).projects, [Card::Fair]);
    assert!(!moves(&g).contains(&Move::BuyLandscape(Card::Fair)));
    respond(&mut g, Response::Move(Move::BuyLandscape(Card::Barracks)));
    assert!(!moves(&g).contains(&Move::BuyLandscape(Card::Cathedral)));
    assert_eq!(g.turn().coins, 15 - 10);
    next_turn(&mut g);
    next_turn(&mut g);
    resolve_triggers(&mut g);
    assert_eq!((g.turn().actions, g.turn().buys), (2, 2));
}

#[test]
fn projects_are_not_cheaper_with_canal_or_inventor() {
    let mut g = setup(&[Card::Inventor]);
    give_project(&mut g, P0, Card::Canal);
    g.start();
    assert_eq!(g.cost(Card::Fair), Cost::coins(4));
    assert_eq!(g.cost(Card::Gold), Cost::coins(5));
    assert_eq!(g.cost(Card::Copper), Cost::ZERO);
    next_turn(&mut g);
    assert_eq!(g.cost(Card::Gold), Cost::coins(6));
}

#[test]
fn canal_makes_estates_cost_one_for_villain() {
    let mut g = setup(&[Card::Villain]);
    give_project(&mut g, P0, Card::Canal);
    hand(&mut g, P0, &[Card::Villain]);
    hand(
        &mut g,
        P1,
        &[
            Card::Estate,
            Card::Estate,
            Card::Copper,
            Card::Copper,
            Card::Copper,
        ],
    );
    g.start();
    play(&mut g, Card::Villain);
    assert!(g.log().iter().any(|e| matches!(
        e,
        Event::RevealedHand { player, .. } if *player == P1
    )));
}

#[test]
fn cathedral_trashes_a_card_at_the_start_of_turn() {
    let mut g = setup(&[Card::Smithy]);
    give_project(&mut g, P0, Card::Cathedral);
    hand(&mut g, P0, &[Card::Estate, Card::Copper]);
    g.start();
    choose_kinds(&mut g, &[Card::Estate]);
    assert_eq!(hand_kinds(&g, P0), [Card::Copper]);
}

#[test]
fn cathedral_with_an_empty_hand_can_resolve_before_cards_are_drawn() {
    let mut g = setup(&[Card::Smithy]);
    give_project(&mut g, P0, Card::Cathedral);
    give_project(&mut g, P0, Card::SinisterPlot);
    g.ext.renaissance.plot[0] = 2;
    hand(&mut g, P0, &[]);
    deck(&mut g, P0, &[Card::Gold, Card::Gold, Card::Copper]);
    g.start();
    // Cathedral first, with nothing to trash; then Sinister Plot's two cards.
    trigger(&mut g, Card::Cathedral);
    mode(&mut g, 1);
    assert!(phase(&g).is_some(), "{:?}", g.pending());
    assert_eq!(hand_kinds(&g, P0), [Card::Gold, Card::Gold]);
}

#[test]
fn city_gate_draws_then_topdecks() {
    let mut g = setup(&[Card::Smithy]);
    give_project(&mut g, P0, Card::CityGate);
    hand(&mut g, P0, &[Card::Estate, Card::Copper]);
    deck(&mut g, P0, &[Card::Gold, Card::Silver]);
    g.start();
    assert_eq!(g.hand(P0).len(), 3);
    choose_kinds(&mut g, &[Card::Estate]);
    assert_eq!(deck_kinds(&g, P0), [Card::Estate, Card::Silver]);
}

#[test]
fn fair_resolves_without_asking_for_an_order() {
    let mut g = setup(&[Card::Smithy]);
    for project in [Card::Fair, Card::Barracks, Card::CityGate, Card::Piazza] {
        give_project(&mut g, P0, project);
    }
    hand(&mut g, P0, &[Card::Estate, Card::Copper]);
    deck(&mut g, P0, &[Card::Copper, Card::Silver, Card::Gold]);
    g.start();
    assert_eq!((g.turn().buys, g.turn().actions), (2, 1));
    // What the others do can depend on their order (Piazza can play any Action).
    let DecisionKind::Choose { labels, .. } = &g.pending().unwrap().kind else {
        panic!("expected the trigger order");
    };
    assert_eq!(labels, &["Barracks", "City Gate", "Piazza"]);
}

#[test]
fn barracks_after_a_snowy_village_gives_no_action() {
    let mut g = setup(&[Card::SnowyVillage]);
    give_project(&mut g, P0, Card::Barracks);
    give_project(&mut g, P0, Card::Piazza);
    hand(&mut g, P0, &[]);
    deck(&mut g, P0, &[Card::SnowyVillage, Card::Copper]);
    g.start();
    trigger(&mut g, Card::Piazza);
    assert_eq!(g.turn().actions, 5);
}

#[test]
fn the_key_can_be_taken_after_other_start_of_turn_effects() {
    let mut g = setup(&[Card::Treasurer, Card::PoorHouse]);
    give_project(&mut g, P0, Card::Piazza);
    let key = g.artifacts.iter_mut().find(|a| a.0 == Card::Key);
    key.expect("the Key is in the game").1 = Some(P0);
    hand(&mut g, P0, &[Card::Copper; 5]);
    deck(&mut g, P0, &[Card::PoorHouse, Card::Copper, Card::Copper]);
    g.start();
    // Piazza first: Poor House makes $4 - $5 = $0; then the Key's $1 is kept.
    trigger(&mut g, Card::Piazza);
    assert_eq!(g.turn().coins, 1);
}

#[test]
fn pageant_pays_a_coin_for_coffers() {
    let mut g = setup(&[Card::Smithy]);
    give_project(&mut g, P0, Card::Pageant);
    hand(&mut g, P0, &[Card::Copper]);
    g.start();
    to_buy(&mut g);
    play(&mut g, Card::Copper);
    end_phase(&mut g);
    trigger(&mut g, Card::Pageant);
    assert_eq!(g.player(P0).coffers, 1);
}

#[test]
fn exploration_rewards_a_buy_phase_without_gains() {
    let mut config = GameConfig::new(2, vec![Card::Smithy], 7);
    config.colonies = Some(false);
    config.shelters = Some(false);
    config.landscapes = vec![Card::Exploration];
    let mut g = setup_with(config);
    give_project(&mut g, P1, Card::Exploration);
    hand(&mut g, P0, &[Card::Gold, Card::Copper]);
    hand(&mut g, P1, &[Card::Copper]);
    g.start();
    to_buy(&mut g);
    play(&mut g, Card::Gold);
    play(&mut g, Card::Copper);
    // Buying Exploration itself gains nothing.
    respond(
        &mut g,
        Response::Move(Move::BuyLandscape(Card::Exploration)),
    );
    assert_eq!((g.player(P0).coffers, g.player(P0).villagers), (1, 1));
    assert_eq!(g.turn().player, P1);
    play(&mut g, Card::Copper);
    buy(&mut g, Card::Copper);
    assert_eq!((g.player(P1).coffers, g.player(P1).villagers), (0, 0));
}

#[test]
fn exploration_looks_at_each_buy_phase_on_its_own() {
    let mut g = setup(&[Card::Villa]);
    give_project(&mut g, P0, Card::Exploration);
    hand(&mut g, P0, &[Card::Gold, Card::Copper]);
    g.start();
    to_buy(&mut g);
    play(&mut g, Card::Gold);
    play(&mut g, Card::Copper);
    buy(&mut g, Card::Villa);
    assert_eq!(phase(&g), Some(Phase::Action));
    assert_eq!((g.player(P0).coffers, g.player(P0).villagers), (0, 0));
    // The second Buy phase, with nothing to do in it, passes by itself.
    end_phase(&mut g);
    assert_eq!((g.player(P0).coffers, g.player(P0).villagers), (1, 1));
}

#[test]
fn sewers_trashes_one_more_per_trashed_card_but_not_its_own() {
    let mut g = setup(&[Card::Chapel]);
    give_project(&mut g, P0, Card::Sewers);
    hand(
        &mut g,
        P0,
        &[
            Card::Chapel,
            Card::Estate,
            Card::Estate,
            Card::Copper,
            Card::Copper,
        ],
    );
    g.start();
    clear_trash(&mut g);
    play(&mut g, Card::Chapel);
    choose_kinds(&mut g, &[Card::Estate, Card::Estate]);
    trigger(&mut g, Card::Sewers);
    // The second Estate's Sewers; the Copper trashed by Sewers doesn't trigger it.
    trigger(&mut g, Card::Sewers);
    assert_eq!(trash_kinds(&g).len(), 4);
    assert!(g.hand(P0).is_empty());
}

/// Priest played on another player's turn still gives its player +$2 per trash that turn
/// (nothing to spend there, but the -$1 token takes it).
#[test]
fn priest_played_on_another_players_turn_still_pays_for_trashing() {
    let mut g = setup(&[
        Card::Priest,
        Card::ThroneRoom,
        Card::Ambassador,
        Card::Gondola,
    ]);
    hand(&mut g, P0, &[Card::Ambassador, Card::Gondola]);
    hand(
        &mut g,
        P1,
        &[
            Card::ThroneRoom,
            Card::Priest,
            Card::Copper,
            Card::Copper,
            Card::Estate,
        ],
    );
    g.start();
    play_on_another_players_turn(&mut g);
    choose_kinds(&mut g, &[Card::ThroneRoom]);
    choose_kinds(&mut g, &[Card::Priest]);
    choose_kinds(&mut g, &[Card::Copper]);
    g.players[1].minus_coin = true;
    choose_kinds(&mut g, &[Card::Estate]);
    assert!(!g.player(P1).minus_coin);
    assert_eq!(g.turn().player, P0);
}

#[test]
fn priest_does_not_pay_for_its_own_sewers_trash() {
    let mut g = setup(&[Card::Priest]);
    give_project(&mut g, P0, Card::Sewers);
    hand(&mut g, P0, &[Card::Priest, Card::Copper, Card::Estate]);
    g.start();
    play(&mut g, Card::Priest);
    choose_kinds(&mut g, &[Card::Copper]);
    trigger(&mut g, Card::Sewers);
    assert!(g.hand(P0).is_empty());
    assert_eq!(g.turn().coins, 2);
}

#[test]
fn star_chart_puts_the_chosen_card_on_top_of_the_shuffle() {
    let mut g = setup(&[Card::Smithy]);
    give_project(&mut g, P0, Card::StarChart);
    hand(&mut g, P0, &[Card::Smithy]);
    deck(&mut g, P0, &[Card::Copper]);
    discard(
        &mut g,
        P0,
        &[Card::Estate, Card::Estate, Card::Gold, Card::Estate],
    );
    g.start();
    play(&mut g, Card::Smithy);
    trigger(&mut g, Card::StarChart);
    choose_kinds(&mut g, &[Card::Gold]);
    let hand = hand_kinds(&g, P0);
    assert_eq!(hand[0], Card::Copper);
    assert_eq!(hand[1], Card::Gold);
}

#[test]
fn fair_and_barracks_and_key_at_the_start_of_turn() {
    let mut g = setup(&[Card::Treasurer]);
    give_project(&mut g, P0, Card::Fair);
    give_project(&mut g, P0, Card::Barracks);
    g.artifacts.iter_mut().for_each(|a| {
        if a.0 == Card::Key {
            a.1 = Some(P0);
        }
    });
    g.start();
    resolve_triggers(&mut g);
    assert_eq!((g.turn().actions, g.turn().buys, g.turn().coins), (2, 2, 1));
}

#[test]
fn silos_discards_coppers_to_draw() {
    let mut g = setup(&[Card::Smithy]);
    give_project(&mut g, P0, Card::Silos);
    hand(
        &mut g,
        P0,
        &[Card::Copper, Card::Copper, Card::Estate, Card::Copper],
    );
    deck(&mut g, P0, &[Card::Silver, Card::Gold, Card::Silver]);
    g.start();
    choose_kinds(&mut g, &[Card::Copper, Card::Copper]);
    assert_eq!(
        hand_kinds(&g, P0),
        [Card::Estate, Card::Copper, Card::Silver, Card::Gold]
    );
}

#[test]
fn sinister_plot_adds_tokens_then_cashes_them_in() {
    let mut g = setup(&[Card::Smithy]);
    give_project(&mut g, P0, Card::SinisterPlot);
    deck(&mut g, P0, &[Card::Copper; 20]);
    g.start();
    assert_eq!(trigger_labels(&g)[1], "Remove your tokens (you have none)");
    mode(&mut g, 0);
    next_turn(&mut g);
    next_turn(&mut g);
    assert_eq!(trigger_labels(&g)[1], "Remove your 1 token for +1 Card");
    mode(&mut g, 0);
    assert_eq!(g.ext.renaissance.plot[0], 2);
    next_turn(&mut g);
    next_turn(&mut g);
    mode(&mut g, 1);
    assert_eq!(g.hand(P0).len(), 7);
    assert_eq!(g.ext.renaissance.plot[0], 0);
}

#[test]
fn piazza_plays_a_revealed_action_and_leaves_anything_else() {
    let mut g = setup(&[Card::Smithy]);
    give_project(&mut g, P0, Card::Piazza);
    hand(&mut g, P0, &[]);
    deck(
        &mut g,
        P0,
        &[Card::Smithy, Card::Copper, Card::Copper, Card::Copper],
    );
    g.start();
    assert_eq!(in_play_kinds(&g, P0), [Card::Smithy]);
    assert_eq!(g.hand(P0).len(), 3);
    assert_eq!(g.turn().actions, 1);
    let mut g = setup(&[Card::Smithy]);
    give_project(&mut g, P0, Card::Piazza);
    deck(&mut g, P0, &[Card::Gold]);
    g.start();
    assert_eq!(deck_kinds(&g, P0), [Card::Gold]);
}

#[test]
fn crop_rotation_may_discard_a_victory_card_for_two_cards() {
    let mut g = setup(&[Card::Smithy]);
    give_project(&mut g, P0, Card::CropRotation);
    hand(&mut g, P0, &[Card::Estate, Card::Copper]);
    deck(&mut g, P0, &[Card::Gold, Card::Gold]);
    g.start();
    trigger(&mut g, Card::CropRotation);
    assert_eq!(hand_kinds(&g, P0), [Card::Copper, Card::Gold, Card::Gold]);
}

#[test]
fn academy_and_guildhall_reward_gains() {
    let mut g = setup(&[Card::Workshop, Card::Village]);
    give_project(&mut g, P0, Card::Academy);
    give_project(&mut g, P0, Card::Guildhall);
    hand(&mut g, P0, &[Card::Workshop, Card::Copper, Card::Copper]);
    g.start();
    play(&mut g, Card::Workshop);
    choose_pile(&mut g, Card::Village);
    assert_eq!(g.player(P0).villagers, 1);
    to_buy(&mut g);
    play(&mut g, Card::Copper);
    play(&mut g, Card::Copper);
    buy(&mut g, Card::Copper);
    assert_eq!(g.player(P0).coffers, 1);
}

#[test]
fn road_network_draws_when_another_player_gains_a_victory_card() {
    let mut g = setup(&[Card::Smithy]);
    give_project(&mut g, P1, Card::RoadNetwork);
    give_project(&mut g, P0, Card::RoadNetwork);
    hand(&mut g, P0, &[Card::Copper, Card::Copper]);
    g.start();
    to_buy(&mut g);
    play(&mut g, Card::Copper);
    play(&mut g, Card::Copper);
    buy(&mut g, Card::Estate);
    assert_eq!(g.hand(P1).len(), 6);
    assert_eq!(g.hand(P0).len(), 5);
}

#[test]
fn innovation_may_play_one_gained_action_per_turn() {
    let mut g = setup(&[Card::Workshop, Card::Village, Card::Smithy]);
    give_project(&mut g, P0, Card::Innovation);
    hand(
        &mut g,
        P0,
        &[Card::Workshop, Card::Copper, Card::Copper, Card::Copper],
    );
    deck(&mut g, P0, &[Card::Copper; 10]);
    g.start();
    play(&mut g, Card::Workshop);
    choose_pile(&mut g, Card::Smithy);
    // Declined once, it can still be used later this turn.
    decline(&mut g);
    assert_eq!(discard_kinds(&g, P0), [Card::Smithy]);
    to_buy(&mut g);
    play(&mut g, Card::Copper);
    play(&mut g, Card::Copper);
    play(&mut g, Card::Copper);
    buy(&mut g, Card::Village);
    trigger(&mut g, Card::Innovation);
    let played = |c| Event::Played {
        player: P0,
        card: c,
    };
    assert!(g.log().contains(&played(Card::Village)));
    assert!(!g.log().contains(&played(Card::Smithy)));
}

#[test]
fn citadel_replays_the_first_action_each_turn() {
    let mut g = setup(&[Card::Smithy, Card::Village]);
    give_project(&mut g, P0, Card::Citadel);
    hand(&mut g, P0, &[Card::Village, Card::Smithy]);
    deck(&mut g, P0, &[Card::Copper; 20]);
    g.start();
    play(&mut g, Card::Village);
    assert_eq!(g.turn().actions, 4);
    play(&mut g, Card::Smithy);
    // Smithy is not the first Action: drawn 1 + 1 + 3 cards only.
    assert_eq!(g.hand(P0).len(), 5);
}

#[test]
fn citadel_still_replays_an_action_a_royal_carriage_replayed_first() {
    let mut g = setup(&[Card::RoyalCarriage, Card::Village]);
    give_project(&mut g, P0, Card::Citadel);
    g.create(Card::RoyalCarriage, Zone::Mat(P0, crate::game::Mat::Tavern));
    hand(&mut g, P0, &[Card::Village]);
    deck(&mut g, P0, &[Card::Copper; 10]);
    g.start();
    play(&mut g, Card::Village);
    trigger(&mut g, Card::RoyalCarriage);
    // The Village, Royal Carriage's replay, then Citadel's.
    assert_eq!(g.turn().actions, 6);
}

#[test]
fn citadel_replays_a_throne_room() {
    let mut g = setup(&[Card::ThroneRoom, Card::Smithy]);
    give_project(&mut g, P0, Card::Citadel);
    hand(&mut g, P0, &[Card::ThroneRoom, Card::Smithy, Card::Smithy]);
    deck(&mut g, P0, &[Card::Copper; 20]);
    g.start();
    play(&mut g, Card::ThroneRoom);
    choose_kinds(&mut g, &[Card::Smithy]);
    // The replayed Throne Room plays the other Smithy twice.
    choose_kinds(&mut g, &[Card::Smithy]);
    assert_eq!(g.hand(P0).len(), 12);
}

#[test]
fn capitalism_makes_plus_coin_actions_treasures_on_your_turns() {
    let mut g = setup(&[Card::Patron, Card::Smithy]);
    give_project(&mut g, P0, Card::Capitalism);
    hand(&mut g, P0, &[Card::Patron, Card::Smithy]);
    hand(&mut g, P1, &[Card::Patron]);
    g.start();
    let patron = find(&g, P0, Card::Patron);
    let smithy = find(&g, P0, Card::Smithy);
    assert!(g.is(patron, Types::TREASURE));
    assert!(!g.is(smithy, Types::TREASURE));
    end_phase(&mut g);
    assert!(moves(&g).contains(&Move::Play(patron)));
    play(&mut g, Card::Patron);
    assert_eq!(g.turn().coins, 2);
    assert_eq!(g.player(P0).villagers, 1);
    next_turn(&mut g);
    let theirs = find(&g, P1, Card::Patron);
    assert!(!g.is(theirs, Types::TREASURE));
}

#[test]
fn fleet_gives_its_owners_one_more_turn_after_the_game_ends() {
    let mut g = GameConfig::new(3, vec![Card::Smithy], 7);
    g.colonies = Some(false);
    g.shelters = Some(false);
    let mut g = setup_with(g);
    give_project(&mut g, P0, Card::Fleet);
    give_project(&mut g, P2, Card::Fleet);
    let provinces = g.supply_pile(Card::Province).unwrap();
    let all = g.pile(provinces).cards.clone();
    for &c in &all[1..] {
        g.move_card(c, Zone::Trash, crate::game::Pos::Top);
    }
    hand(&mut g, P1, &[Card::Gold; 3]);
    g.start();
    next_turn(&mut g);
    to_buy(&mut g);
    for _ in 0..3 {
        play(&mut g, Card::Gold);
    }
    buy(&mut g, Card::Province);
    assert!(g.outcome().is_none());
    // P2 then P0 take their Fleet turns; P1 doesn't.
    assert_eq!(g.turn().player, P2);
    let fleet: Vec<Option<PlayerId>> = g
        .view(P1)
        .notes()
        .iter()
        .filter(|n| n.source == Card::Fleet)
        .map(|n| n.player)
        .collect();
    assert_eq!(fleet, [None, Some(P0)]);
    next_turn(&mut g);
    assert_eq!(g.turn().player, P0);
    to_buy(&mut g);
    end_phase(&mut g);
    let outcome = g.outcome().expect("the game is over");
    assert_eq!(outcome.reason, EndReason::ProvincesGone);
    // Fleet turns, like extra turns, do not count for the tiebreaker.
    assert_eq!(outcome.turns, [1, 1, 0]);
    assert!(g.view(P1).notes().iter().all(|n| n.source != Card::Fleet));
}

#[test]
fn without_fleet_the_game_ends_at_once() {
    let mut g = setup(&[Card::Smithy]);
    give_project(&mut g, P1, Card::Pageant);
    let provinces = g.supply_pile(Card::Province).unwrap();
    let all = g.pile(provinces).cards.clone();
    for &c in &all[1..] {
        g.move_card(c, Zone::Trash, crate::game::Pos::Top);
    }
    hand(&mut g, P0, &[Card::Gold; 3]);
    g.start();
    to_buy(&mut g);
    for _ in 0..3 {
        play(&mut g, Card::Gold);
    }
    buy(&mut g, Card::Province);
    assert!(g.outcome().is_some());
}

#[test]
fn artifacts_are_set_up_with_their_cards() {
    let g = setup(&[Card::BorderGuard, Card::Treasurer]);
    let arts: Vec<Card> = g.artifacts().iter().map(|a| a.0).collect();
    assert_eq!(arts, [Card::Lantern, Card::Horn, Card::Key]);
    assert!(setup(&[Card::Smithy]).artifacts().is_empty());
}

#[test]
fn improve_can_trash_a_cargo_ship_whose_next_set_aside_card_is_stranded() {
    let mut g = setup(&[
        Card::Improve,
        Card::CargoShip,
        Card::Village,
        Card::Smithy,
        Card::Hideout,
    ]);
    hand(&mut g, P0, &[Card::Village, Card::CargoShip, Card::Improve]);
    deck(&mut g, P0, &[Card::Copper; 15]);
    g.start();
    play(&mut g, Card::Village);
    play(&mut g, Card::CargoShip);
    play(&mut g, Card::Improve);
    to_buy(&mut g);
    end_phase(&mut g);
    trigger(&mut g, Card::Improve);
    choose_kinds(&mut g, &[Card::CargoShip]);
    choose_pile(&mut g, Card::Smithy);
    // Cargo Ship's ability still sets the Smithy aside, but nothing brings it back.
    trigger(&mut g, Card::CargoShip);
    assert_eq!(kinds(&g, &g.player(P0).set_aside), [Card::Smithy]);
    next_turn(&mut g);
    next_turn(&mut g);
    assert_eq!(kinds(&g, &g.player(P0).set_aside), [Card::Smithy]);
    assert_eq!(g.count_owned(P0, Card::Smithy), 1);
}

#[test]
fn the_key_taken_at_the_start_of_turn_gives_its_coin_at_once() {
    let mut g = setup(&[Card::Treasurer]);
    give_project(&mut g, P0, Card::Piazza);
    hand(&mut g, P0, &[]);
    deck(&mut g, P0, &[Card::Treasurer, Card::Copper]);
    g.start();
    // Piazza plays the Treasurer.
    mode(&mut g, 2);
    assert_eq!(g.artifact_holder(Card::Key), Some(P0));
    assert_eq!(g.turn().coins, 4);
}

#[test]
fn with_capitalism_treasurer_can_gain_an_improve_from_the_trash() {
    let mut g = setup(&[Card::Treasurer, Card::Improve]);
    give_project(&mut g, P0, Card::Capitalism);
    hand(&mut g, P0, &[Card::Treasurer]);
    clear_trash(&mut g);
    let improve = g.create(Card::Improve, Zone::Trash);
    g.start();
    play(&mut g, Card::Treasurer);
    mode(&mut g, 1);
    assert_eq!(g.zone(improve), Zone::Hand(P0));
}

#[test]
fn old_witch_lets_victims_trash_a_curse_when_the_curses_are_gone() {
    let mut g = setup(&[Card::OldWitch]);
    let curses = g.supply_pile(Card::Curse).unwrap();
    for c in g.pile(curses).cards.clone() {
        g.move_card(c, Zone::Trash, crate::game::Pos::Top);
    }
    hand(&mut g, P0, &[Card::OldWitch]);
    deck(&mut g, P0, &[Card::Copper; 5]);
    hand(&mut g, P1, &[Card::Curse, Card::Copper]);
    g.start();
    play(&mut g, Card::OldWitch);
    choose_kinds(&mut g, &[Card::Curse]);
    assert_eq!(hand_kinds(&g, P1), [Card::Copper]);
    assert_eq!(g.count_owned(P1, Card::Curse), 0);
}

#[test]
fn an_outpost_turn_comes_before_the_fleet_round() {
    let mut g = setup(&[Card::Outpost]);
    give_project(&mut g, P0, Card::Fleet);
    let provinces = g.supply_pile(Card::Province).unwrap();
    let all = g.pile(provinces).cards.clone();
    for &c in &all[1..] {
        g.move_card(c, Zone::Trash, crate::game::Pos::Top);
    }
    hand(
        &mut g,
        P1,
        &[Card::Outpost, Card::Gold, Card::Gold, Card::Gold],
    );
    g.start();
    next_turn(&mut g);
    play(&mut g, Card::Outpost);
    for _ in 0..3 {
        play(&mut g, Card::Gold);
    }
    buy(&mut g, Card::Province);
    assert_eq!((g.turn().player, g.turn().extra), (P1, Some(Card::Outpost)));
    next_turn(&mut g);
    assert_eq!((g.turn().player, g.turn().extra), (P0, None));
    to_buy(&mut g);
    end_phase(&mut g);
    assert_eq!(g.outcome().expect("game over").turns, [1, 1]);
}

#[test]
fn star_chart_puts_its_card_under_the_cards_left_in_the_deck() {
    let mut g = setup(&[Card::BorderGuard]);
    give_project(&mut g, P0, Card::StarChart);
    hand(&mut g, P0, &[Card::BorderGuard]);
    deck(&mut g, P0, &[Card::Copper]);
    discard(&mut g, P0, &[Card::Estate, Card::Gold, Card::Estate]);
    g.start();
    play(&mut g, Card::BorderGuard);
    trigger(&mut g, Card::StarChart);
    choose_kinds(&mut g, &[Card::Gold]);
    // Border Guard revealed the Copper, then the chosen Gold.
    assert_eq!(kinds(&g, &candidates(&g)), [Card::Copper, Card::Gold]);
}

#[test]
fn citadel_replays_an_action_treasure_played_first_in_the_buy_phase() {
    let mut g = setup(&[Card::FlagBearer]);
    give_project(&mut g, P0, Card::Citadel);
    give_project(&mut g, P0, Card::Capitalism);
    hand(&mut g, P0, &[Card::FlagBearer]);
    g.start();
    to_buy(&mut g);
    play(&mut g, Card::FlagBearer);
    assert_eq!(g.turn().coins, 4);
}

/// Which card was the first Action is settled as the cards are played: a Copper played
/// before Enlightenment took effect doesn't become it afterwards.
#[test]
fn citadel_replays_the_first_card_that_was_an_action_when_played() {
    let mut g = setup(&[Card::Kitsune]);
    g.prophecy = Some((Card::Enlightenment, 1));
    give_project(&mut g, P0, Card::Citadel);
    give_project(&mut g, P0, Card::Capitalism);
    hand(&mut g, P0, &[Card::Copper, Card::Kitsune]);
    g.start();
    to_buy(&mut g);
    play(&mut g, Card::Copper);
    play(&mut g, Card::Kitsune);
    let coins_and_actions = Response::Options(vec![Opt::Mode(0), Opt::Mode(1)]);
    respond(&mut g, coins_and_actions.clone());
    assert!(g.prophecy_active(Card::Enlightenment));
    respond(&mut g, coins_and_actions);
    assert_eq!(g.turn().coins, 1 + 2 + 2);
}

/// Random games over Renaissance with Throne Room and Moat, each player starting with a
/// copy of the Kingdom cards and a random Project, with two more to buy. Half the games
/// start with nearly empty Estate, Curse and Copper piles, so that they end (and Fleet
/// matters).
#[test]
fn random_renaissance_games_keep_invariants() {
    let pool: Vec<Card> = Card::in_set(Set::Renaissance)
        .filter(|c| c.def().kingdom)
        .collect();
    let projects: Vec<Card> = Card::in_set(Set::Renaissance)
        .filter(|c| c.kind() == Kind::Project)
        .collect();
    for seed in 0..fuzz_seeds(150) {
        let mut rng = ChaCha8Rng::seed_from_u64(seed);
        let mut cards = pool.clone();
        cards.shuffle(&mut rng);
        cards.truncate(8);
        cards.extend([Card::ThroneRoom, Card::Moat]);
        let mut landscapes = projects.clone();
        landscapes.shuffle(&mut rng);
        let players = rng.random_range(2..=4);
        let mut config = GameConfig::new(players, cards.clone(), seed);
        config.turn_limit = Some(60);
        config.landscapes = landscapes[..2].to_vec();
        let mut g = setup_with(config);
        for p in 0..players {
            let p = PlayerId(p);
            for &c in &cards {
                g.create(c, Zone::Deck(p));
            }
            let mut deck = std::mem::take(&mut g.players[p.index()].deck);
            g.shuffle(&mut deck);
            g.players[p.index()].deck = deck;
            let mut mine = projects.clone();
            mine.shuffle(&mut rng);
            g.players[p.index()].projects = vec![mine[0]];
        }
        if seed % 2 == 0 {
            for (card, left) in [(Card::Estate, 1), (Card::Curse, 2), (Card::Copper, 4)] {
                let pile = g.supply_pile(card).unwrap();
                let all = g.pile(pile).cards.clone();
                for &c in &all[left..] {
                    g.move_card(c, Zone::Trash, crate::game::Pos::Top);
                }
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
    }
}

#[test]
fn mountain_village_search_does_not_reveal_the_discard_taken() {
    let world = |pick: usize| {
        let mut g = setup(&[Card::MountainVillage]);
        hand(&mut g, P0, &[Card::MountainVillage]);
        let pile = discard(&mut g, P0, &[Card::Gold, Card::Curse, Card::Curse]);
        g.start();
        play(&mut g, Card::MountainVillage);
        choose(&mut g, &[pile[pick]]);
        g
    };
    let (curse, gold) = (world(2), world(0));
    assert_eq!(public_table(&curse, P1), public_table(&gold, P1));
    assert_eq!(curse.log(), gold.log());
}

#[test]
fn sinister_plot_token_is_logged() {
    let mut g = setup(&[Card::Village]);
    g.players[0].projects.push(Card::SinisterPlot);
    g.start();
    assert_eq!(g.pending().unwrap().source, Some(Card::SinisterPlot));
    let before = g.log().len();
    respond(&mut g, Response::option(Opt::Mode(0)));
    assert!(
        g.log()[before..].contains(&Event::CardCounter {
            player: Some(P0),
            card: Card::SinisterPlot,
            pile: None,
            delta: 1,
        }),
        "{:?}",
        g.log()
    );
}
