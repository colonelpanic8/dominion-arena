//! Turn order across sets: extra turns, skipped turns, the round after the game's end and
//! effects that last until a turn starts.

use super::*;

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

/// What the next turn is: its player, who controls it and the extra turn's source.
fn turn(g: &Game) -> (PlayerId, PlayerId, Option<Card>) {
    let t = g.turn();
    (t.player, t.controller, t.extra)
}

fn in_play_kinds(g: &Game, p: PlayerId) -> Vec<Card> {
    kinds(g, g.in_play(p))
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
