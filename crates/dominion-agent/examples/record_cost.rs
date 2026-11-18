//! What recording a game costs: one long 4-seat game played the same way with and without
//! recording, through a `Table` as Arena hosts it (snapshot written as JSON after every
//! answer), and through the engine alone to tell the journal's time from the recorder's.
//! `cargo run --release -p dominion-agent --example record_cost [black-market|divine-wind] [seed]`

use dominion_agent::Table;
use dominion_agent::events::Recorder;
use dominion_engine::{Card, DecisionKind, Game, GameConfig, Move, Response};
use rand::seq::{IndexedRandom, SliceRandom};
use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;
use std::time::{Duration, Instant};

fn config(which: &str, seed: u64) -> GameConfig {
    use Card::*;
    let mut config = match which {
        "divine-wind" => {
            let mut c = GameConfig::new(
                4,
                vec![
                    Village, Smithy, Market, Festival, Laboratory, Militia, Moat, Remodel, Mine,
                    Witch,
                ],
                seed,
            );
            c.landscapes = vec![DivineWind, Training, Borrow];
            c
        }
        _ => GameConfig::new(
            4,
            vec![
                BlackMarket,
                Village,
                Smithy,
                Market,
                Festival,
                Laboratory,
                Militia,
                Moat,
                Remodel,
                Witch,
            ],
            seed,
        ),
    };
    config.turn_limit = Some(200);
    config
}

/// Plays Actions and Treasures, buys the dearest thing; otherwise anything legal.
fn respond(game: &Game, rng: &mut ChaCha8Rng) -> Response {
    let d = game.pending().expect("a decision");
    match &d.kind {
        DecisionKind::Turn { moves, .. } => {
            let plays: Vec<&Move> = (moves.iter())
                .filter(|m| matches!(m, Move::Play(_)))
                .collect();
            if let Some(m) = plays.choose(rng) {
                return Response::Move(**m);
            }
            let price = |m: &Move| {
                let cost = match *m {
                    Move::Buy(pile) => game.cost_of(*game.pile(pile).cards.last()?),
                    Move::BuyLandscape(card) => game.cost(card),
                    _ => return None,
                };
                Some(cost.coins as u32 + cost.debt as u32 + 3 * cost.potions as u32)
            };
            let best = (moves.iter())
                .filter(|m| price(m).is_some_and(|p| p > 1))
                .max_by_key(|m| (price(m), rng.random::<u8>()));
            Response::Move(*best.unwrap_or_else(|| moves.choose(rng).expect("a move")))
        }
        DecisionKind::Cards {
            candidates,
            min,
            max,
            ..
        } => {
            let mut picks = candidates.clone();
            picks.shuffle(rng);
            picks.truncate(rng.random_range(*min..=(*max).min(candidates.len())));
            Response::Cards(picks)
        }
        DecisionKind::Pile { piles, .. } => Response::Pile(piles.choose(rng).copied()),
        DecisionKind::Choose {
            options, min, max, ..
        } => {
            let mut picks = options.clone();
            picks.shuffle(rng);
            picks.truncate(rng.random_range(*min..=(*max).min(options.len())));
            Response::Options(picks)
        }
        DecisionKind::NameCard => Response::Name(Card::Copper),
        DecisionKind::Number { min, max } => Response::Number(rng.random_range(*min..=*max)),
    }
}

struct Run {
    answers: u32,
    respond: Duration,
    snapshot: Duration,
    snapshot_bytes: usize,
    max_snapshot_bytes: usize,
    stream_bytes: usize,
    events: usize,
}

fn table_run(config: &GameConfig, seed: u64, recorded: bool) -> Run {
    let mut table = match recorded {
        true => Table::new_recorded("g1", config.clone(), vec![]),
        false => Table::new("g1", config.clone(), vec![]),
    }
    .expect("a kingdom");
    let mut rng = ChaCha8Rng::seed_from_u64(seed);
    let mut run = Run {
        answers: 0,
        respond: Duration::ZERO,
        snapshot: Duration::ZERO,
        snapshot_bytes: 0,
        max_snapshot_bytes: 0,
        stream_bytes: 0,
        events: 0,
    };
    while let Some(seat) = table.deciding() {
        let response = respond(table.game(), &mut rng);
        let id = table.decision_id().expect("open");
        let t = Instant::now();
        table.respond(seat, &id, response).expect("legal");
        run.respond += t.elapsed();
        let t = Instant::now();
        let bytes = serde_json::to_vec(&table).expect("json");
        run.snapshot += t.elapsed();
        run.snapshot_bytes = bytes.len();
        run.max_snapshot_bytes = run.max_snapshot_bytes.max(bytes.len());
        for event in table.unwritten() {
            run.stream_bytes += serde_json::to_vec(event).expect("json").len() + 1;
            run.events += 1;
        }
        table.mark_written();
        run.answers += 1;
    }
    run
}

/// The engine alone: unjournaled, journaled, and the recorder over the journal.
fn engine_run(config: &GameConfig, seed: u64) -> (Duration, Duration, Duration) {
    let mut plain = Game::new_traced(config.clone()).expect("a kingdom");
    let mut journaled = Game::new_journaled(config.clone()).expect("a kingdom");
    let mut recorder = Recorder::new();
    recorder.record(&mut journaled, &[]).expect("one game");
    let mut rng = ChaCha8Rng::seed_from_u64(seed);
    let (mut p, mut j, mut r) = (Duration::ZERO, Duration::ZERO, Duration::ZERO);
    while plain.pending().is_some() {
        let response = respond(&plain, &mut rng);
        let t = Instant::now();
        plain.respond(response.clone()).expect("legal");
        p += t.elapsed();
        let t = Instant::now();
        journaled.respond(response).expect("legal");
        j += t.elapsed();
        let t = Instant::now();
        recorder.record(&mut journaled, &[]).expect("one game");
        r += t.elapsed();
    }
    (p, j, r)
}

fn main() {
    let which = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "black-market".into());
    let seed = (std::env::args().nth(2)).map_or(1, |s| s.parse().expect("a seed"));
    let config = config(&which, seed);
    let us = |d: Duration, n: u32| d.as_secs_f64() * 1e6 / f64::from(n.max(1));
    for recorded in [false, true] {
        let run = table_run(&config, seed, recorded);
        println!(
            "table {}: {} answers, respond {:.0} us/answer, snapshot {:.0} us/answer, \
             final snapshot {} bytes (largest {}), stream {} events in {} bytes",
            if recorded { "recorded" } else { "plain" },
            run.answers,
            us(run.respond, run.answers),
            us(run.snapshot, run.answers),
            run.snapshot_bytes,
            run.max_snapshot_bytes,
            run.events,
            run.stream_bytes,
        );
    }
    let (p, j, r) = engine_run(&config, seed);
    println!(
        "engine: unjournaled {:.1} ms, journaled {:.1} ms (journal {:.1} ms), recorder {:.1} ms",
        p.as_secs_f64() * 1e3,
        j.as_secs_f64() * 1e3,
        (j.saturating_sub(p)).as_secs_f64() * 1e3,
        r.as_secs_f64() * 1e3,
    );
}
