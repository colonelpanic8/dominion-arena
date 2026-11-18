//! events@1 is its types, not JSON: a stream written in another serde encoding reads
//! back the same and folds to the same table.

use dominion_agent::events::Recorder;
use dominion_engine::{Card, DecisionKind, Game, GameConfig, Response, Set};
use dominion_protocol::events::{Event, Table};

/// The first legal answer.
fn first(game: &Game) -> Response {
    match &game.pending().expect("a decision").kind {
        DecisionKind::Turn { moves, .. } => Response::Move(moves[0]),
        DecisionKind::Cards {
            candidates, min, ..
        } => Response::Cards(candidates[..*min].to_vec()),
        DecisionKind::Pile { piles, .. } => Response::Pile(piles.first().copied()),
        DecisionKind::Choose { options, min, .. } => Response::Options(options[..*min].to_vec()),
        DecisionKind::NameCard => Response::Name(Card::Copper),
        DecisionKind::Number { min, .. } => Response::Number(*min),
    }
}

#[test]
fn a_stream_survives_an_encoding_that_is_not_json() {
    let sets: Vec<Set> = (Set::ALL.iter().copied())
        .filter(|&s| s != Set::Arcana)
        .collect();
    for seed in 0..12 {
        let mut config = GameConfig::random(2 + (seed % 3) as u8, &sets, seed);
        config.turn_limit = Some(30);
        let mut game = Game::new_journaled(config).unwrap();
        let mut recorder = Recorder::new();
        let mut events = vec![];
        loop {
            events.extend(recorder.record(&mut game, &[]).unwrap());
            if game.outcome().is_some() {
                break;
            }
            let response = first(&game);
            game.respond(response).unwrap();
        }
        let mut table = Table::default();
        let mut read = vec![];
        for event in &events {
            let mut bytes = vec![];
            ciborium::into_writer(event, &mut bytes).unwrap();
            let back: Event = ciborium::from_reader(&bytes[..])
                .unwrap_or_else(|e| panic!("seed {seed}, event {}: {e}", event.seq));
            assert_eq!(&back, event, "seed {seed}");
            table.apply(&back).unwrap();
            read.push(back);
        }
        assert!(&table == recorder.table(), "seed {seed}");
        // What was read back is enough to play the game again.
        dominion_agent::replay::replay(&read).unwrap_or_else(|d| panic!("seed {seed}: {d}"));
    }
}
