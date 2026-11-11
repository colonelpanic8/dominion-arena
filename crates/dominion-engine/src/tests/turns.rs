//! Turn order across sets: extra turns, skipped turns, the round after the game's end and
//! effects that last until a turn starts.

use super::*;
use crate::game::{ExtraLimit, ExtraTurn, Pos};

fn game(players: u8, kingdom: &[Card], landscapes: &[Card]) -> Game {
    setup_with(GameConfig {
        colonies: Some(false),
        shelters: Some(false),
        landscapes: landscapes.to_vec(),
        ..GameConfig::new(players, kingdom.to_vec(), 7)
    })
}

fn turn_starts(g: &Game) -> usize {
    g.log()
        .iter()
        .filter(|e| matches!(e, Event::TurnStart { .. }))
        .count()
}

/// Ends phases until the turn is over: the next one has started, the game has ended, or a
/// player is asked which extra turn to take.
fn end_turn(g: &mut Game) {
    let n = turn_starts(g);
    while turn_starts(g) == n
        && let Some(DecisionKind::Turn { .. }) = g.pending().map(|d| &d.kind)
    {
        end_phase(g);
    }
}

/// Answers `p`'s choice among their extra turns with the one from `source`.
fn take_turn(g: &mut Game, p: PlayerId, source: Card) {
    let d = g.pending().expect("a decision is pending");
    assert_eq!((d.player, d.purpose), (p, Purpose::Either), "{d:?}");
    respond(g, Response::option(Opt::Card(source)));
}

/// What the next turn is: its player, who controls it and the extra turn's source.
fn turn(g: &Game) -> (PlayerId, PlayerId, Option<Card>) {
    let t = g.turn();
    (t.player, t.controller, t.extra)
}

fn in_play_kinds(g: &Game, p: PlayerId) -> Vec<Card> {
    kinds(g, g.in_play(p))
}

/// Enters the Buy phase, if the Action phase is still on, with `coins` and `buys`.
fn buy_phase(g: &mut Game, coins: u32, buys: u32) {
    if let Some(DecisionKind::Turn {
        phase: Phase::Action,
        ..
    }) = g.pending().map(|d| &d.kind)
    {
        end_phase(g);
    }
    g.turn.coins = coins;
    g.turn.buys = buys;
    g.pending = None;
    g.run(crate::frame::Input::Start);
}

/// P0 plays `actions`, then buys `events` with their Buys, which ends their first turn.
fn first_turn(kingdom: &[Card], actions: &[Card], events: &[Card]) -> Game {
    let mut g = game(2, kingdom, events);
    hand(&mut g, P0, actions);
    deck(&mut g, P0, &[Card::Copper; 20]);
    g.start();
    for &card in actions {
        play(&mut g, card);
    }
    buy_phase(&mut g, 4 * events.len() as u32, events.len() as u32);
    for &event in events {
        respond(&mut g, Response::Move(Move::BuyLandscape(event)));
    }
    g
}

fn leave_one(g: &mut Game, card: Card) {
    let pile = g.supply_pile(card).unwrap();
    let all = g.pile(pile).cards.clone();
    for &c in &all[1..] {
        g.move_card(c, Zone::Trash, Pos::Top);
    }
}

fn buy_the_last_province(g: &mut Game) {
    buy_phase(g, 8, 1);
    buy(g, Card::Province);
}

#[test]
fn seize_the_days_turn_can_follow_missions_but_not_the_other_way_round() {
    for events in [
        [Card::SeizeTheDay, Card::Mission],
        [Card::Mission, Card::SeizeTheDay],
    ] {
        let mut g = first_turn(&[], &[], &events);
        take_turn(&mut g, P0, Card::Mission);
        assert_eq!(turn(&g), (P0, P0, Some(Card::Mission)));
        end_turn(&mut g);
        assert_eq!(turn(&g), (P0, P0, Some(Card::SeizeTheDay)));
        end_turn(&mut g);
        assert_eq!(turn(&g), (P1, P1, None));

        let mut g = first_turn(&[], &[], &events);
        take_turn(&mut g, P0, Card::SeizeTheDay);
        assert_eq!(turn(&g), (P0, P0, Some(Card::SeizeTheDay)));
        end_turn(&mut g);
        // Mission's would be a third turn in a row.
        assert_eq!(turn(&g), (P1, P1, None));
        assert!(g.extra_turns().is_empty());
    }
}

#[test]
fn of_missions_and_journeys_turns_the_player_takes_one_and_the_other_fails() {
    for taken in [Card::Mission, Card::Journey] {
        let mut g = first_turn(&[], &[], &[Card::Mission, Card::Journey]);
        take_turn(&mut g, P0, taken);
        assert_eq!(turn(&g), (P0, P0, Some(taken)));
        end_turn(&mut g);
        assert_eq!(turn(&g), (P1, P1, None));
    }
}

#[test]
fn an_outpost_whose_turn_fails_is_discarded_in_the_next_clean_up() {
    let mut g = first_turn(&[Card::Outpost], &[Card::Outpost], &[Card::Mission]);
    take_turn(&mut g, P0, Card::Mission);
    assert_eq!(turn(&g), (P0, P0, Some(Card::Mission)));
    // Outpost still left only 3 cards for this hand.
    assert_eq!(g.hand(P0).len(), 3);
    assert_eq!(in_play_kinds(&g, P0), [Card::Outpost]);
    end_turn(&mut g);
    assert_eq!(turn(&g), (P1, P1, None));
    assert!(g.in_play(P0).is_empty());
}

/// Possession doesn't pass control between turns: the possessed player chooses among their
/// own extra turns, and the Possession turn can't be their second extra turn in a row.
#[test]
fn a_player_about_to_be_possessed_chooses_among_their_own_extra_turns() {
    for taken in [Card::Possession, Card::Mission] {
        let mut g = game(2, &[Card::Possession], &[Card::Mission]);
        hand(&mut g, P0, &[Card::Possession]);
        g.start();
        play(&mut g, Card::Possession);
        // As if P1 had set up an extra turn of their own during P0's turn.
        g.extra_turns.push_back(ExtraTurn {
            player: P1,
            source: Card::Mission,
            controller: P1,
            limit: ExtraLimit::ThirdInARow,
        });
        end_turn(&mut g);
        take_turn(&mut g, P1, taken);
        if taken == Card::Possession {
            assert_eq!(turn(&g), (P1, P0, Some(Card::Possession)));
            end_turn(&mut g);
        }
        assert_eq!(turn(&g), (P1, P1, Some(Card::Mission)));
        end_turn(&mut g);
        assert_eq!(turn(&g), (P1, P1, None));
    }
}

#[test]
fn lich_skips_the_extra_turn_its_player_takes_first() {
    let mut g = first_turn(
        &[Card::Wizards, Card::Odysseys],
        &[Card::Voyage, Card::Lich],
        &[Card::Mission],
    );
    take_turn(&mut g, P0, Card::Voyage);
    assert_eq!(turn(&g), (P0, P0, Some(Card::Mission)));
    assert_eq!(g.player(P0).skip_turns, 0);
    end_turn(&mut g);
    assert_eq!(turn(&g), (P1, P1, None));
    assert!(g.in_play(P0).is_empty());
}

/// A skipped turn isn't taken, so the turns around it are in a row.
#[test]
fn turns_around_another_players_skipped_turn_are_in_a_row() {
    let mut g = game(2, &[Card::Wizards], &[Card::Mission]);
    hand(&mut g, P1, &[Card::Lich]);
    deck(&mut g, P1, &[Card::Copper; 20]);
    g.start();
    end_turn(&mut g);
    play(&mut g, Card::Lich);
    end_turn(&mut g);
    end_turn(&mut g);
    assert_eq!(turn(&g), (P0, P0, None));
    assert_eq!(g.player(P0).turns_taken, 3);
    buy_phase(&mut g, 4, 1);
    respond(&mut g, Response::Move(Move::BuyLandscape(Card::Mission)));
    // Mission's would be P0's third turn in a row.
    assert_eq!(turn(&g), (P1, P1, None));
}

#[test]
fn lich_skips_a_fleet_turn() {
    let mut g = game(3, &[Card::Wizards], &[Card::Fleet]);
    g.players[0].projects.push(Card::Fleet);
    g.players[1].projects.push(Card::Fleet);
    leave_one(&mut g, Card::Province);
    hand(&mut g, P0, &[Card::Lich]);
    deck(&mut g, P0, &[Card::Copper; 20]);
    g.start();
    play(&mut g, Card::Lich);
    buy_the_last_province(&mut g);
    assert_eq!(turn(&g), (P1, P1, None));
    end_turn(&mut g);
    let outcome = g.outcome().expect("P0's Fleet turn is skipped");
    assert_eq!(outcome.turns, [1, 0, 0]);
}

#[test]
fn extra_turns_are_taken_between_fleet_turns_but_not_after_the_last() {
    let mut g = game(2, &[Card::Outpost], &[Card::Fleet]);
    g.players[1].projects.push(Card::Fleet);
    leave_one(&mut g, Card::Province);
    hand(&mut g, P0, &[Card::Outpost]);
    hand(&mut g, P1, &[Card::Outpost]);
    g.start();
    play(&mut g, Card::Outpost);
    buy_the_last_province(&mut g);
    assert_eq!(turn(&g), (P0, P0, Some(Card::Outpost)));
    end_turn(&mut g);
    assert_eq!(turn(&g), (P1, P1, None));
    play(&mut g, Card::Outpost);
    end_turn(&mut g);
    assert!(g.outcome().is_some());
}

#[test]
fn no_extra_turn_is_taken_once_the_game_has_ended() {
    let mut g = game(2, &[Card::Outpost], &[]);
    leave_one(&mut g, Card::Province);
    hand(&mut g, P0, &[Card::Outpost]);
    g.start();
    play(&mut g, Card::Outpost);
    buy_the_last_province(&mut g);
    assert!(g.outcome().is_some());
}

#[test]
fn frigate_played_at_the_start_of_a_turn_lasts_until_the_next_turn() {
    let mut g = game(2, &[Card::Frigate, Card::Village], &[Card::Piazza]);
    g.players[0].projects.push(Card::Piazza);
    hand(&mut g, P0, &[Card::Copper]);
    deck(&mut g, P0, &[Card::Frigate]);
    let mut cards = vec![Card::Village];
    cards.extend([Card::Copper; 4]);
    hand(&mut g, P1, &cards);
    deck(&mut g, P1, &[Card::Copper; 5]);
    g.start();
    // Piazza plays the Frigate at the start of P0's turn.
    assert_eq!(in_play_kinds(&g, P0), [Card::Frigate]);
    end_turn(&mut g);
    assert_eq!(in_play_kinds(&g, P0), [Card::Frigate]);
    play(&mut g, Card::Village);
    // P1 has 5 cards in hand after the Village and discards down to 4.
    assert_eq!(g.hand(P1).len(), 4);
    end_turn(&mut g);
    end_turn(&mut g);
    assert_eq!(turn(&g), (P1, P1, None));
    assert!(g.in_play(P0).is_empty());
}

#[test]
fn outpost_played_at_the_start_of_a_turn_stays_in_play_through_its_extra_turn() {
    let mut g = game(2, &[Card::Outpost], &[Card::Piazza]);
    g.players[0].projects.push(Card::Piazza);
    deck(
        &mut g,
        P0,
        &[Card::Outpost, Card::Copper, Card::Copper, Card::Copper],
    );
    g.start();
    assert_eq!(in_play_kinds(&g, P0), [Card::Outpost]);
    end_turn(&mut g);
    assert_eq!(turn(&g), (P0, P0, Some(Card::Outpost)));
    assert_eq!(in_play_kinds(&g, P0), [Card::Outpost]);
    end_turn(&mut g);
    assert!(g.in_play(P0).is_empty());
}
