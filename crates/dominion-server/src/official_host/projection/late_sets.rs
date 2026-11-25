//! Allies, Plunder and Rising Sun: random games with copies of each admitted card dealt
//! into every starting deck. Every decision projects for every seat, and every wire answer
//! offered to the decider resolves to a response the engine accepts.
//! `LATE_SETS_CARDS=<names>` checks other cards; `LATE_SETS_REPORT=1` prints instead of failing.
use super::*;
use dominion_engine::{DecisionKind, Game, GameConfig, KnownCards, Move, Response, Set};
use dominion_online::questions::{Question, QuestionKind};
use std::panic::{AssertUnwindSafe, catch_unwind};

const FILLER: &[Card] = &[
    Card::Village,
    Card::Smithy,
    Card::Market,
    Card::Cellar,
    Card::Militia,
    Card::Moat,
    Card::Laboratory,
    Card::Festival,
    Card::Workshop,
    Card::Remodel,
];

fn catalog() -> &'static WireCatalog {
    static C: std::sync::OnceLock<WireCatalog> = std::sync::OnceLock::new();
    C.get_or_init(super::tests::catalog)
}

struct Rng(u64);
impl Rng {
    fn below(&mut self, n: usize) -> usize {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        ((self.0 >> 33) as usize) % n.max(1)
    }
}

fn update(game: &Game, seat: u8, index: i32) -> Result<ProjectedUpdate, ProjectionError> {
    let n = game.num_players();
    let names: Vec<_> = (0..n).map(|i| format!("Seat{}", i + 1)).collect();
    let context = HostGameContext {
        wire_ids: WireIdentity {
            game_id: 100,
            table_id: 31,
            player_ids: (0..n).map(|i| 1000 + (i as i32) * 13).collect(),
        },
        table_details: transport::TableDetails {
            table_id: 31,
            host_id: 1000,
            players: (0..n)
                .map(|i| transport::NamedId {
                    id: 1000 + (i as i32) * 13,
                    name: names[i].clone(),
                })
                .collect(),
            ready: vec![true; n],
            bots: vec![],
            spectators: vec![],
            reconnecting: vec![],
            rules: vec![],
            division: String::new(),
            match_games: vec![],
            match_score: None,
            tournament: None,
        },
        viewer_role: ViewerRole::Player(usize::from(seat)),
        table_options: dominion_protocol::lobby::TableSpec {
            timed: false,
            rated: false,
            ..Default::default()
        },
    };
    project(
        hub::SeatObservation {
            clock: None,
            view: game.view(PlayerId(seat)),
            known: &KnownCards::default(),
            names: &names,
            decision_id: game.pending().map(|_| "late-sets"),
            deciding: game.pending().map(|d| d.player),
            version: index as u64,
            undos: 0,
            outcome: game.outcome(),
            ended: None,
        },
        &context,
        catalog(),
        ProjectionRequest {
            question_index: index,
            reconnect: false,
            epoch: 19,
        },
    )
}

/// Every leaf choice of a question: each element alone, the empty answer where it is
/// allowed, and the largest selection.
fn answers(q: &Question) -> Vec<Vec<i32>> {
    match &q.kind {
        QuestionKind::Complex(children) => children
            .iter()
            .enumerate()
            .flat_map(|(i, q)| {
                answers(q)
                    .into_iter()
                    .map(move |a| std::iter::once(i as i32).chain(a).collect())
            })
            .collect(),
        QuestionKind::Choice {
            min, max, elements, ..
        } => {
            let len = elements.len() as i32;
            let mut a = vec![];
            if *min == 0 {
                a.push(vec![]);
            }
            if *min <= 1 && *max >= 1 {
                a.extend((0..len).map(|i| vec![i]));
            }
            if *max > 1 {
                a.push((0..(*max).min(len)).rev().collect());
            }
            if *min > 1 {
                a.push((0..*min).collect());
            }
            a
        }
        QuestionKind::Number { min, max, .. } => vec![vec![*min], vec![*max]],
        QuestionKind::Name => vec![vec![2]],
        _ => vec![],
    }
}

/// Deals copies of `card` in place of some of each player's starting Coppers.
fn deal(game: Game, card: Card) -> Game {
    let mut state = serde_json::to_value(&game).unwrap();
    let copper = serde_json::to_value(Card::Copper).unwrap();
    let with = serde_json::to_value(card).unwrap();
    for p in 0..game.num_players() {
        let mut changed = 0;
        for zone in ["hand", "deck"] {
            let ids: Vec<usize> = state["players"][p][zone]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_u64().unwrap() as usize)
                .collect();
            for id in ids {
                if changed < 3 && state["cards"][id] == copper {
                    state["cards"][id] = with.clone();
                    changed += 1;
                }
            }
        }
    }
    serde_json::from_value(state).unwrap()
}

fn game_for(card: Card, seed: u64) -> Option<Game> {
    let mut kingdom: Vec<Card> = FILLER.to_vec();
    let pile = card.def().pile.filter(|p| p.def().kingdom);
    if let Some(pile) = pile.or(card.def().kingdom.then_some(card)) {
        kingdom.truncate(9);
        kingdom.push(pile);
    }
    let mut cfg = GameConfig::new(2, kingdom, 5000 + seed);
    cfg.colonies = Some(false);
    cfg.shelters = Some(false);
    if card.kind().is_landscape() {
        cfg.landscapes.push(card);
    }
    let game = Game::new(cfg).ok()?;
    Some(if card.kind() == dominion_engine::Kind::Card {
        deal(game, card)
    } else {
        game
    })
}

fn respond_for(game: &Game, card: Card, rng: &mut Rng) -> Option<Response> {
    let d = game.pending()?;
    let DecisionKind::Turn { moves, .. } = &d.kind else {
        return None;
    };
    let preferred: Vec<_> = moves
        .iter()
        .filter(|m| match m {
            Move::Play(id) => game.card(*id) == card,
            Move::BuyLandscape(l) => *l == card,
            _ => false,
        })
        .collect();
    if !preferred.is_empty() && rng.below(4) != 0 {
        return Some(Response::Move(*preferred[rng.below(preferred.len())]));
    }
    let others: Vec<_> = moves.iter().filter(|m| **m != Move::EndPhase).collect();
    Some(if others.is_empty() || rng.below(4) == 0 {
        Response::Move(Move::EndPhase)
    } else {
        Response::Move(*others[rng.below(others.len())])
    })
}

/// Plays `seeds` games and returns every refusal.
fn check(card: Card, seeds: u64) -> Vec<String> {
    let mut errors = std::collections::BTreeSet::new();
    for seed in 0..seeds {
        let Some(mut game) = game_for(card, seed) else {
            errors.insert("setup".into());
            continue;
        };
        let mut rng = Rng(seed * 7919 + 101);
        for step in 0..200 {
            let Some(d) = game.pending() else { break };
            let view = game.view(d.player);
            // A table trashed down to nothing can play turns forever without a question.
            if view
                .players()
                .any(|p| p.hand_size() + p.deck_size() + p.discard_size() + p.in_play().len() < 5)
            {
                break;
            }
            let decider = d.player.0;
            let mut question = None;
            for seat in 0..game.num_players() as u8 {
                match catch_unwind(AssertUnwindSafe(|| update(&game, seat, step + 1))) {
                    Ok(Ok(u)) if seat == decider => question = u.question,
                    Ok(Ok(_)) => {}
                    Ok(Err(e)) => {
                        errors.insert(e.to_string());
                    }
                    Err(_) => {
                        errors.insert("projection panicked".into());
                    }
                }
            }
            let mut responses = vec![];
            if let Some(q) = &question {
                for a in answers(&q.wire) {
                    match q.resolve_at(19, q.wire.index, &a) {
                        Ok(r) => match game.pending().unwrap().validate(&r) {
                            Ok(()) => responses.push(r),
                            Err(e) => {
                                errors.insert(format!("{:?} {:?} {a:?}: {e}", d.source, d.purpose));
                            }
                        },
                        Err(e) => {
                            errors.insert(format!("{:?} {:?} {a:?}: {e:?}", d.source, d.purpose));
                        }
                    }
                }
            }
            let response = respond_for(&game, card, &mut rng).or_else(|| {
                (!responses.is_empty()).then(|| responses.swap_remove(rng.below(responses.len())))
            });
            let Some(response) = response else { break };
            if game.respond(response).is_err() {
                break;
            }
        }
    }
    errors.into_iter().collect()
}

fn late_set(card: Card) -> bool {
    matches!(card.set(), Set::Allies | Set::Plunder | Set::RisingSun)
}

#[test]
fn allies_plunder_and_rising_sun_admitted_cards_project_and_resolve() {
    let only: Option<Vec<String>> = std::env::var("LATE_SETS_CARDS")
        .ok()
        .map(|s| s.split(',').map(|x| x.trim().to_lowercase()).collect());
    let seeds = std::env::var("LATE_SETS_SEEDS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(3);
    let report = std::env::var("LATE_SETS_REPORT").is_ok();
    let cards: Vec<Card> = Card::ALL
        .iter()
        .copied()
        .filter(|&c| late_set(c) && c.kind() != dominion_engine::Kind::Randomizer)
        .filter(|&c| match &only {
            Some(only) => only.contains(&c.name().to_lowercase()),
            None => super::super::questions::semantics::set_requirements(c).is_some(),
        })
        .collect();
    let mut failures = vec![];
    for card in cards {
        let errors = check(card, seeds);
        if report {
            eprintln!("{}: {errors:?}", card.name());
        } else if !errors.is_empty() {
            failures.push(format!("{}: {errors:?}", card.name()));
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}
