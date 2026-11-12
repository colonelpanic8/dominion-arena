//! An Event or Project no one buys and a Way no one uses change nothing: each game is
//! played twice in step, once with such landscapes added, and the two are compared at
//! every decision. `FUZZ_EXTRA` names the one landscape to add.

use super::*;

const KINDS: &[Kind] = &[Kind::Event, Kind::Project, Kind::Way];

/// Landscapes whose setup changes the game before anyone uses them: the Horse and Loot
/// piles, Tax's Debt tokens, the card Way of the Mouse sets aside.
const SETUP: &[Card] = &[
    Card::Bargain,
    Card::Demand,
    Card::Ride,
    Card::Stampede,
    Card::Foray,
    Card::Invasion,
    Card::Looting,
    Card::Peril,
    Card::Prosper,
    Card::Tax,
    Card::WayOfTheMouse,
];

/// The game without what the added landscapes are allowed to change: the list of
/// landscapes, the question being asked, and the log (compared apart).
fn core(g: &mut Game) -> Json {
    let log = std::mem::take(&mut g.log);
    let mut v = serde_json::to_value(&*g).unwrap();
    g.log = log;
    let o = v.as_object_mut().unwrap();
    for key in ["config", "landscapes", "pending", "log"] {
        o.remove(key);
    }
    v
}

/// Where two values first differ, and how.
fn diff(a: &Json, b: &Json, path: &str) -> Option<(String, String)> {
    let short = |v: &Json| v.to_string().chars().take(400).collect::<String>();
    match (a, b) {
        _ if a == b => None,
        (Json::Object(x), Json::Object(y)) => x.keys().chain(y.keys()).find_map(|k| {
            diff(
                x.get(k).unwrap_or(&Json::Null),
                y.get(k).unwrap_or(&Json::Null),
                &format!("{path}.{k}"),
            )
        }),
        (Json::Array(x), Json::Array(y)) if x.len() == y.len() => x
            .iter()
            .zip(y)
            .enumerate()
            .find_map(|(i, (x, y))| diff(x, y, &format!("{path}[{i}]"))),
        _ => Some((
            path.to_string(),
            format!("{} / with them {}", short(a), short(b)),
        )),
    }
}

/// `d` without the options the added landscapes give.
fn without(d: &Decision, extra: &[Card]) -> Decision {
    let mut d = d.clone();
    match &mut d.kind {
        DecisionKind::Turn { moves, .. } => {
            moves.retain(|m| !matches!(m, Move::BuyLandscape(l) if extra.contains(l)));
        }
        DecisionKind::Choose {
            options,
            labels,
            max,
            ..
        } => {
            let keep: Vec<bool> = options
                .iter()
                .map(|o| !matches!(o, Opt::Card(l) if extra.contains(l)))
                .collect();
            let mut at = keep.iter();
            options.retain(|_| *at.next().unwrap());
            let mut at = keep.iter();
            labels.retain(|_| *at.next().unwrap());
            *max = (*max).min(options.len());
        }
        _ => {}
    }
    d
}

/// The answer to a question with one.
fn only(d: &Decision) -> Option<Response> {
    match &d.kind {
        DecisionKind::Turn { moves, .. } if moves.len() == 1 => Some(Response::Move(moves[0])),
        DecisionKind::Choose { options, .. } if options.len() == 1 => {
            Some(Response::option(options[0]))
        }
        _ => None,
    }
}

/// Up to one unused landscape of each kind whose setup changes nothing.
fn extras(config: &GameConfig, g: &mut Game, rng: &mut ChaCha8Rng, flags: &mut Flags) -> Vec<Card> {
    let mut pool = landscapes(KINDS);
    pool.retain(|l| !g.landscapes.contains(l));
    pool.shuffle(rng);
    if let Ok(name) = std::env::var("FUZZ_EXTRA") {
        pool.retain(|l| l.name() == name);
    }
    let plain = core(g);
    let mut extra: Vec<Card> = vec![];
    let mut tried = 0;
    for l in pool {
        if extra.iter().any(|e| e.kind() == l.kind()) {
            continue;
        }
        tried += 1;
        if tried > 6 {
            break;
        }
        let mut with = config.clone();
        with.landscapes.extend(&extra);
        with.landscapes.push(l);
        match crate::setup::build(with) {
            Ok(mut b) => match diff(&plain, &core(&mut b), "") {
                None if b.log == g.log => extra.push(l),
                changed if SETUP.contains(&l) => drop(changed),
                Some((path, detail)) => {
                    flags.bug(format!("unused-landscape-changes-setup:{l}"), || {
                        format!("{path}: {detail}")
                    });
                }
                None => flags.bug(format!("unused-landscape-changes-setup:{l}"), || {
                    "the log".into()
                }),
            },
            Err(e) => flags.bug(format!("unused-landscape-rejected:{l}"), || e.to_string()),
        }
    }
    extra
}

pub(super) fn drive_pair(seed: u64, mut plan: Plan, report: &mut Report) {
    let flags = &mut report.flags;
    let mut rng = ChaCha8Rng::seed_from_u64(seed ^ 0x0add);
    let Some(mut a) = build(&mut plan.config, flags) else {
        return;
    };
    let extra = extras(&plan.config, &mut a, &mut rng, flags);
    enrich(&mut a, &mut rng, plan.rich);
    if plan.thin {
        thin_decks(&mut a, &mut rng);
    }
    report.setup = format!("{}, added {extra:?}", setup_text(seed, &plan, &a));
    // Their setup changed nothing, so the game with them is this one with them listed.
    let mut b = a.clone();
    b.landscapes.extend(&extra);
    b.config.landscapes.extend(&extra);
    a.start();
    b.start();

    let mut agents: Vec<Biased> = (0..a.num_players())
        .map(|p| Biased {
            rng: ChaCha8Rng::seed_from_u64(seed * 16 + p as u64),
            greed: plan.greed[p],
        })
        .collect();
    let mut log_seen = 0;
    loop {
        heartbeat(Some(&report.setup));
        let (da, db) = (a.pending().cloned(), b.pending().cloned());
        let about = |a: &Game, b: &Game| {
            format!(
                "added {extra:?}\nwithout: {:?}\n{}\nwith: {:?}\n{}",
                a.pending(),
                ctx(a),
                b.pending(),
                ctx(b)
            )
        };
        if let Some(db) = &db {
            let plain = without(db, &extra);
            if da.as_ref() != Some(&plain) {
                // A question only the added landscapes raise: answer it without them.
                let Some(r) = only(&plain) else {
                    flags.bug(
                        format!("unused-landscape-changes-a-decision:{:?}", db.purpose),
                        || about(&a, &b),
                    );
                    return;
                };
                b.respond(r).expect("the only answer");
                report.steps += 1;
                if report.steps >= MAX_STEPS {
                    flags.bug("game-does-not-end", || ctx(&b));
                    return;
                }
                continue;
            }
        } else if da.is_some() {
            flags.bug("unused-landscape-ends-the-game", || about(&a, &b));
            return;
        }
        if let Some((path, detail)) = diff(&core(&mut a), &core(&mut b), "") {
            let key: String = path
                .chars()
                .filter(|c| !c.is_ascii_digit() && !"[]".contains(*c))
                .collect();
            flags.bug(format!("unused-landscape-changes-the-game:{key}"), || {
                format!("{path}: {detail}\n{}", about(&a, &b))
            });
            return;
        }
        if a.log.len() != b.log.len() || a.log[log_seen..] != b.log[log_seen..] {
            let at = (log_seen..a.log.len().min(b.log.len()))
                .find(|&i| a.log[i] != b.log[i])
                .unwrap_or(a.log.len().min(b.log.len()));
            flags.bug("unused-landscape-changes-the-log", || {
                format!(
                    "{:?} / with them {:?}\n{}",
                    a.log.get(at),
                    b.log.get(at),
                    about(&a, &b)
                )
            });
            return;
        }
        log_seen = a.log.len();
        let Some(d) = da else { break };

        if matches!(
            d.kind,
            DecisionKind::Turn {
                phase: Phase::Action | Phase::Buy,
                ..
            }
        ) && rng.random_bool(plan.boost)
        {
            let boost = if rng.random_bool(0.4) {
                Boost::Draw(rng.random_range(1..=6))
            } else {
                Boost::Turn {
                    actions: rng.random_range(0..=3),
                    buys: rng.random_range(0..=2),
                    coins: rng.random_range(0..=8),
                }
            };
            apply_boost(&mut a, &boost);
            apply_boost(&mut b, &boost);
        }

        let r = agents[d.player.index()].decide(&a, &d);
        if let Err(e) = a.respond(r.clone()) {
            flags.bug("no-valid-response-found", || {
                format!("{r:?} rejected: {e}; {}", ctx(&a))
            });
            return;
        }
        if let Err(e) = b.respond(r.clone()) {
            flags.bug("unused-landscape-rejects-an-answer", || {
                format!("{r:?} rejected: {e}\n{}", about(&a, &b))
            });
            return;
        }
        report.steps += 1;
        if report.steps >= MAX_STEPS {
            flags.bug("game-does-not-end", || ctx(&a));
            return;
        }
    }
    report.reason = a.outcome().map(|o| o.reason);
    report.turns = a.players.iter().map(|p| p.turns_taken).sum();
}

fn fam_any(rng: &mut ChaCha8Rng, seed: u64) -> Kingdom {
    FAMILIES[seed as usize % FAMILIES.len()](rng, seed)
}

#[test]
fn fuzz_unused_landscapes_change_nothing() {
    run_family_with("fam_any", fam_any, drive_pair, 24);
}
