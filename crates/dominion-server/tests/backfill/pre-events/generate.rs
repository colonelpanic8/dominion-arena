// This program made the snapshots beside it, run as an example of dominion-server at
// cfefa17, the commit before games were recorded (`cargo run -p dominion-server --example
// old_snapshots -- <dir>`, then the `.lock` removed). It is not compiled here.

//! Plays games through this build's hub and leaves their snapshots in a state directory.

use dominion_agent::{Answer, WayChoice};
use dominion_engine::Card;
use dominion_server::hub::{Hub, NewGame, Seat};
use rand::seq::SliceRandom;
use rand::{Rng, SeedableRng};
use std::time::Duration;

fn kingdom(names: &[&str]) -> Vec<String> {
    names.iter().map(|s| s.to_string()).collect()
}

/// Plays `n` answers (or until the game ends) with random choices.
fn play(hub: &Hub, seats: &[Seat], n: usize, rng: &mut rand::rngs::StdRng, way: Option<Card>) {
    for _ in 0..n {
        let Some((seat, d)) = seats
            .iter()
            .find_map(|s| Some((s, hub.view(s).unwrap().0.decision?)))
        else {
            return;
        };
        if d.kind == "turn"
            && d.phase.as_deref() == Some("buy")
            && rng.random_bool(0.5)
            && hub.play_treasures(seat, &d.id).is_ok()
        {
            continue;
        }
        let answer = if d.name_any {
            Answer::name(&d.id, "Copper")
        } else {
            let (min, max) = (d.min.unwrap_or(1) as usize, d.max.unwrap_or(1) as usize);
            let mut picks: Vec<u32> = (0..d.options.len() as u32).collect();
            picks.shuffle(rng);
            picks.truncate(rng.random_range(min..=max.min(picks.len())));
            Answer::indices(&d.id, picks)
        };
        let w = match (way, rng.random_range(0..3)) {
            (Some(w), 0) => Some(WayChoice::Way(w)),
            (Some(_), 1) => Some(WayChoice::None),
            _ => None,
        };
        hub.answer_with(seat, &answer, w).unwrap();
    }
}

fn main() {
    let dir = std::path::PathBuf::from(std::env::args().nth(1).expect("a directory"));
    let (hub, _) = Hub::open(&dir, Duration::from_secs(1 << 30)).unwrap();
    let mut rng = rand::rngs::StdRng::seed_from_u64(7);
    let base = kingdom(&[
        "Village", "Smithy", "Militia", "Moat", "Market", "Witch", "Cellar", "Chapel",
        "Throne Room", "Bureaucrat",
    ]);
    let games: Vec<(Vec<String>, Vec<String>, Option<Card>, u64)> = vec![
        // g1: finished, retired
        (base.clone(), vec![], None, 1),
        // g2: resigned (abandoned)
        (base.clone(), vec![], None, 2),
        // g3: unfinished, retired
        (base.clone(), vec![], None, 3),
        // g4: Ways, finished
        (
            kingdom(&[
                "Village", "Smithy", "Militia", "Moat", "Throne Room", "Market", "Laboratory",
                "Festival", "Mine", "Bureaucrat",
            ]),
            kingdom(&["Way of the Ox", "Way of the Sheep"]),
            Some(Card::WayOfTheOx),
            4,
        ),
        // g5: Star Chart and Avoid, finished
        (
            kingdom(&[
                "Village", "Smithy", "Cellar", "Harbinger", "Laboratory", "Market", "Sentry",
                "Vassal", "Moneylender", "Library",
            ]),
            kingdom(&["Star Chart", "Avoid"]),
            None,
            5,
        ),
        // g6: in progress, hosted
        (base.clone(), vec![], None, 6),
    ];
    let mut all = vec![];
    for (k, l, way, seed) in games {
        let created = hub
            .create(&NewGame {
                players: 2,
                kingdom: k,
                landscapes: l,
                seed: Some(seed),
                turn_limit: Some(24),
                ..NewGame::default()
            })
            .unwrap();
        let seats: Vec<Seat> = created
            .seats
            .iter()
            .map(|s| hub.claim(&created.game_id, &s.token).unwrap())
            .collect();
        all.push((created.game_id, seats, way));
    }
    for (id, seats, way) in &all {
        let n = match id.as_str() {
            "g2" => 40,
            "g3" | "g6" => 30,
            _ => 100_000,
        };
        play(&hub, seats, n, &mut rng, *way);
        if id == "g2" {
            hub.resign(&seats[1], "abandoned").unwrap();
        }
    }
    hub.retire("g1").unwrap();
    hub.retire("g3").unwrap();
    for g in hub.list() {
        println!("{} running={}", g.game_id, g.running);
    }
}
