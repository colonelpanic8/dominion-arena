//! Which cards the transport carries through random games (`DOMINION_TRANSPORT_SURVEY=<out.json>`).
//! Each card is played in games whose other piles the transport already carries, with
//! copies of it dealt into every starting deck; every decision is projected for each
//! seat and answered through a wire answer, and the first refusal per card is kept.
use super::*;
use dominion_engine::{DecisionKind, Game, GameConfig, KnownCards, Move, Response};
use dominion_online::questions::{Question, QuestionKind};
use serde_json::{Value, json};
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
    Card::Chapel,
];

pub(super) struct Rng(pub(super) u64);
impl Rng {
    pub(super) fn below(&mut self, n: usize) -> usize {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        ((self.0 >> 33) as usize) % n.max(1)
    }
}

fn project_seat(game: &Game, seat: u8, index: i32) -> Result<ProjectedUpdate, String> {
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
    let result = catch_unwind(AssertUnwindSafe(|| {
        project(
            hub::SeatObservation {
                clock: None,
                view: game.view(PlayerId(seat)),
                known: &KnownCards::default(),
                names: &names,
                decision_id: game.pending().map(|_| "survey"),
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
    }));
    match result {
        Ok(Ok(u)) => Ok(u),
        Ok(Err(e)) => Err(e.to_string()),
        Err(_) => Err("panic: projection".into()),
    }
}

fn catalog() -> &'static WireCatalog {
    static C: std::sync::OnceLock<WireCatalog> = std::sync::OnceLock::new();
    C.get_or_init(super::tests::catalog)
}

fn and_type() -> i32 {
    catalog().symbol("QuestionTypes", "COMPLEX_AND").unwrap()
}

/// A random legal wire answer: indices for each leaf down a random path.
fn wire_answer(q: &Question, rng: &mut Rng) -> Option<Vec<i32>> {
    match &q.kind {
        QuestionKind::Complex(children) if q.description.question_type == and_type() => {
            let mut a = vec![];
            for child in children {
                let values = wire_answer(child, rng)?;
                a.push(values.len() as i32);
                a.extend(values);
            }
            Some(a)
        }
        QuestionKind::Complex(children) => {
            let i = rng.below(children.len());
            let mut a = vec![i as i32];
            a.extend(wire_answer(&children[i], rng)?);
            Some(a)
        }
        QuestionKind::Choice {
            min, max, elements, ..
        } => {
            let len = elements.len() as i32;
            let hi = (*max).min(len);
            let lo = (*min).min(hi);
            let n = lo + rng.below((hi - lo + 1) as usize) as i32;
            let mut pool: Vec<i32> = (0..len).collect();
            let mut out = vec![];
            for _ in 0..n {
                out.push(pool.remove(rng.below(pool.len())));
            }
            Some(out)
        }
        QuestionKind::Number { min, max, .. } => {
            Some(vec![min + rng.below((max - min + 1) as usize) as i32])
        }
        QuestionKind::Name => Some(vec![2]),
        _ => None,
    }
}

/// A random legal engine answer, leaning towards playing `card` and buying.
pub(super) fn engine_answer(game: &Game, card: Card, rng: &mut Rng) -> Response {
    let d = game.pending().unwrap();
    match &d.kind {
        DecisionKind::Turn { moves, .. } => {
            let preferred: Vec<_> = moves
                .iter()
                .filter(|m| match m {
                    Move::Play(id) => game.card(*id) == card,
                    Move::BuyLandscape(l) => *l == card,
                    _ => false,
                })
                .collect();
            if !preferred.is_empty() && rng.below(4) != 0 {
                return Response::Move(*preferred[rng.below(preferred.len())]);
            }
            let others: Vec<_> = moves.iter().filter(|m| **m != Move::EndPhase).collect();
            if others.is_empty() || rng.below(4) == 0 {
                Response::Move(Move::EndPhase)
            } else {
                Response::Move(*others[rng.below(others.len())])
            }
        }
        DecisionKind::Cards {
            candidates,
            min,
            max,
            ordered,
        } => {
            let hi = (*max).min(candidates.len());
            let n = (*min).min(hi) + rng.below(hi - (*min).min(hi) + 1);
            let mut pool = candidates.clone();
            let mut out = vec![];
            for _ in 0..n {
                out.push(pool.remove(rng.below(pool.len())));
            }
            let _ = ordered;
            Response::Cards(out)
        }
        DecisionKind::Pile { piles, optional } => {
            if *optional && rng.below(5) == 0 {
                Response::Pile(None)
            } else {
                Response::Pile(Some(piles[rng.below(piles.len())]))
            }
        }
        DecisionKind::Choose {
            options, min, max, ..
        } => {
            let hi = (*max).min(options.len());
            let n = (*min).min(hi) + rng.below(hi - (*min).min(hi) + 1);
            let mut pool = options.clone();
            let mut out = vec![];
            for _ in 0..n {
                out.push(pool.remove(rng.below(pool.len())));
            }
            Response::Options(out)
        }
        DecisionKind::NameCard => Response::Name(Card::Copper),
        DecisionKind::Number { min, max } => {
            Response::Number(min + rng.below((max - min + 1) as usize) as i32)
        }
    }
}

/// Deals copies of `card` in place of some of each player's starting Coppers.
pub(super) fn deal(game: Game, card: Card) -> Game {
    let mut state = serde_json::to_value(&game).unwrap();
    let copper = serde_json::to_value(Card::Copper).unwrap();
    let with = serde_json::to_value(card).unwrap();
    for p in 0..game.num_players() {
        let mut changed = 0;
        for zone in ["deck", "hand"] {
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
    serde_json::from_value(state).unwrap_or(game)
}

pub(super) fn survey_card(card: Card, seeds: u64) -> (u32, Vec<String>) {
    let mut decisions = 0;
    let mut errors = vec![];
    for seed in 0..seeds {
        let mut kingdom: Vec<Card> = FILLER.iter().copied().filter(|&c| c != card).collect();
        kingdom.truncate(if card.def().kingdom { 9 } else { 10 });
        if card.def().kingdom {
            kingdom.push(card);
        }
        let mut cfg = GameConfig::new(2, kingdom, 1000 + seed);
        cfg.colonies = Some(false);
        cfg.shelters = Some(false);
        if card.kind().is_landscape() {
            cfg.landscapes.push(card);
        }
        let game = match catch_unwind(|| Game::new(cfg)) {
            Ok(Ok(g)) => g,
            Ok(Err(e)) => {
                errors.push(format!("setup: {e}"));
                continue;
            }
            Err(_) => {
                errors.push("setup: panic".into());
                continue;
            }
        };
        let mut game = if card.def().kingdom && card.kind() == dominion_engine::Kind::Card {
            deal(game, card)
        } else {
            game
        };
        let mut rng = Rng(seed * 7919 + 17);
        for step in 0..160 {
            let Some(d) = game.pending() else { break };
            // A table trashed down to nothing can play turns forever without a question.
            let view = game.view(d.player);
            if view
                .players()
                .any(|p| p.hand_size() + p.deck_size() + p.discard_size() + p.in_play().len() < 5)
            {
                break;
            }
            let decider = d.player.0;
            if std::env::var("SURVEY_TRACE").is_ok() {
                eprintln!("step {step} {:?} {:?} {:?}", d.player, d.purpose, d.source);
            }
            let mut answer = None;
            for seat in 0..game.num_players() as u8 {
                if std::env::var("SURVEY_TRACE").is_ok() {
                    eprintln!("  project seat {seat}");
                }
                match project_seat(&game, seat, step + 1) {
                    Ok(u) if seat == decider => answer = u.question,
                    Ok(_) => {}
                    Err(e) => {
                        if errors.len() < 64 {
                            errors.push(e)
                        }
                    }
                }
            }
            decisions += 1;
            let mut response = None;
            if let Some(q) = &answer
                && let Some(values) = wire_answer(&q.wire, &mut rng)
            {
                match catch_unwind(AssertUnwindSafe(|| q.resolve_at(19, q.wire.index, &values))) {
                    Ok(Ok(r)) => match game.pending().unwrap().validate(&r) {
                        Ok(()) => response = Some(r),
                        Err(e) => errors.push(format!(
                            "answer: {:?} {:?} invalid: {e}",
                            d.source, d.purpose
                        )),
                    },
                    Ok(Err(e)) => errors.push(format!(
                        "answer: {:?} {:?} rejected: {e:?}",
                        d.source, d.purpose
                    )),
                    Err(_) => errors.push("answer: panic".into()),
                }
            }
            let response = response.unwrap_or_else(|| engine_answer(&game, card, &mut rng));
            if let Ok(dir) = std::env::var("SURVEY_TRACE") {
                eprintln!("  respond {response:?}");
                if !dir.is_empty() {
                    let state = serde_json::to_vec(&game).unwrap();
                    std::fs::write(format!("{dir}/state.json"), state).unwrap();
                    std::fs::write(
                        format!("{dir}/response.json"),
                        serde_json::to_vec(&response).unwrap(),
                    )
                    .unwrap();
                }
            }
            if game.respond(response).is_err() {
                let fallback = engine_answer(&game, card, &mut rng);
                if game.respond(fallback).is_err() {
                    break;
                }
            }
        }
    }
    (decisions, errors)
}

fn category(error: &str) -> String {
    error
        .split(" (")
        .next()
        .unwrap_or(error)
        .split(": Some(")
        .next()
        .unwrap()
        .chars()
        .take(90)
        .collect()
}

#[test]
#[ignore = "survey: writes DOMINION_TRANSPORT_SURVEY"]
fn transport_survey() {
    let Ok(out) = std::env::var("DOMINION_TRANSPORT_SURVEY") else {
        return;
    };
    let seeds = std::env::var("SURVEY_SEEDS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(3);
    let only: Option<Vec<String>> = std::env::var("SURVEY_ONLY")
        .ok()
        .filter(|s| !s.is_empty())
        .map(|s| s.split(',').map(|x| x.trim().to_lowercase()).collect());
    let mut cards = serde_json::Map::new();
    let mut by_category: std::collections::BTreeMap<String, Vec<String>> = Default::default();
    let mut clean = 0;
    let mut total = 0;
    for &card in Card::ALL.iter().filter(|c| {
        (c.def().kingdom || c.kind().is_landscape())
            && !c.def().unimplemented
            && catalog().card(**c).is_ok()
    }) {
        if let Some(only) = &only
            && !only.iter().any(|o| *o == card.name().to_lowercase())
            && !only
                .iter()
                .any(|o| *o == format!("{:?}", card.set()).to_lowercase())
        {
            continue;
        }
        total += 1;
        let started = std::time::Instant::now();
        let (decisions, errors) = survey_card(card, seeds);
        if std::env::var("SURVEY_VERBOSE").is_ok() {
            eprintln!(
                "{} {:?} {} errors {:?}",
                card.name(),
                started.elapsed(),
                errors.len(),
                errors.first()
            );
        }
        let mut seen: std::collections::BTreeMap<String, usize> = Default::default();
        for e in &errors {
            *seen.entry(category(e)).or_default() += 1;
        }
        if errors.is_empty() {
            clean += 1;
        }
        for c in seen.keys() {
            by_category
                .entry(c.clone())
                .or_default()
                .push(card.name().into());
        }
        cards.insert(
            card.name().into(),
            json!({
                "set": format!("{:?}", card.set()),
                "decisions": decisions,
                "clean": errors.is_empty(),
                "first": errors.first(),
                "categories": seen,
            }),
        );
    }
    let report = json!({
        "clean": clean,
        "total": total,
        "categories": by_category
            .iter()
            .map(|(k, v)| (k.clone(), json!({"count": v.len(), "cards": v})))
            .collect::<serde_json::Map<String, Value>>(),
        "cards": cards,
    });
    std::fs::write(&out, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    eprintln!("transport survey: {clean}/{total} cards clean; report at {out}");
}
