//! Prints the events@1 stream of a game whose seats take the first option of every
//! decision: `cargo run -p dominion-agent --example events [seed]`.

use dominion_agent::events::Recorder;
use dominion_engine::{DecisionKind, Game, GameConfig, Response, Set};
use dominion_protocol::events::{Header, SCHEMA, Viewer};

fn main() {
    let seed = (std::env::args().nth(1)).map_or(1, |s| s.parse().expect("a seed"));
    let mut config = GameConfig::random(2, &[Set::Base], seed);
    config.turn_limit = Some(30);
    let mut game = Game::new_journaled(config).expect("a kingdom");
    let mut recorder = Recorder::new();
    let header = Header {
        schema: SCHEMA.into(),
        viewer: Viewer::Full,
        game: None,
    };
    println!("{}", serde_json::to_string(&header).expect("json"));
    loop {
        for event in recorder.record(&mut game, &[]).expect("one game") {
            println!("{}", serde_json::to_string(&event).expect("json"));
        }
        let Some(d) = game.pending() else {
            break;
        };
        let response = match &d.kind {
            DecisionKind::Turn { moves, .. } => Response::Move(moves[0]),
            DecisionKind::Cards {
                candidates, min, ..
            } => Response::Cards(candidates[..*min].to_vec()),
            DecisionKind::Pile { piles, .. } => Response::Pile(piles.first().copied()),
            DecisionKind::Choose { options, min, .. } => {
                Response::Options(options[..(*min).max(1)].to_vec())
            }
            DecisionKind::NameCard => Response::Name(dominion_engine::Card::Copper),
            DecisionKind::Number { min, .. } => Response::Number(*min),
        };
        game.respond(response).expect("a legal answer");
    }
}
