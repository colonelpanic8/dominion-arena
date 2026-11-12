//! Fuzz harness: biased agents, rich starting states and targeted kingdoms, with oracles
//! checked at every decision. Each `fuzz_*` test runs one kingdom family.
//!
//! Environment: `FUZZ_SEEDS` (games per family), `FUZZ_START` (first seed), `FUZZ_ONLY`
//! (one seed, with details), `FUZZ_THREADS`, `FUZZ_STATS` (coverage), `FUZZ_COVER` (uses
//! per card), `FUZZ_AUDIT` (print the review-by-hand lists), `FUZZ_KEYS` (every game an
//! oracle fired in), `FUZZ_VIEWS` (percentage of decisions at which a view is checked
//! against hidden cards being swapped), `FUZZ_LOG` and `FUZZ_TRACE` (with `FUZZ_ONLY`: the
//! log, and card moves, decisions and answers).

use super::*;
use crate::frame::Frame;
use crate::game::{Mat, Phase, Player, Pos, Token, Until};
use serde_json::Value as Json;
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Once;

mod unused;

// ---- Findings ----

/// What one game's oracles reported: each key once, with the first detail.
#[derive(Default)]
struct Flags {
    bugs: Vec<(String, String)>,
    /// Not failures: lists to review by hand.
    audit: Vec<(String, String)>,
}

impl Flags {
    fn bug(&mut self, key: impl Into<String>, detail: impl FnOnce() -> String) {
        let key = key.into();
        if !self.bugs.iter().any(|(k, _)| *k == key) {
            self.bugs.push((key, detail()));
        }
    }

    fn audit(&mut self, key: impl Into<String>, detail: impl FnOnce() -> String) {
        let key = key.into();
        if !self.audit.iter().any(|(k, _)| *k == key) {
            self.audit.push((key, detail()));
        }
    }
}

/// Oracle keys (prefixes) of open design questions, with what each is about: reported,
/// not failures.
const OPEN: &[(&str, &str)] = &[(
    "view-depends-on-hidden-cards:score",
    "every view has exact scores, so a face-down pass shows",
)];

thread_local! {
    /// `Some` while a fuzz game runs on this thread: panics are captured (with their
    /// location) instead of printed.
    static CAPTURE: RefCell<Option<String>> = const { RefCell::new(None) };
}

fn install_hook() {
    static HOOK: Once = Once::new();
    HOOK.call_once(|| {
        let prev = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let captured = CAPTURE.with(|c| {
                let mut c = c.borrow_mut();
                if c.is_some() {
                    *c = Some(
                        info.location()
                            .map(|l| format!("{}:{}", l.file(), l.line()))
                            .unwrap_or_default(),
                    );
                    true
                } else {
                    false
                }
            });
            if !captured {
                prev(info);
            }
        }));
    });
}

/// When each thread's current fuzz game last made progress, and which game it is.
static HEARTBEATS: std::sync::Mutex<Vec<(std::thread::ThreadId, std::time::Instant, String)>> =
    std::sync::Mutex::new(Vec::new());

/// Records that this thread's game is alive (`None`: it is over). The first call starts a
/// watchdog that aborts the test run if a game makes no progress for a minute: an effect
/// that loops without asking anything would otherwise run until memory is gone.
fn heartbeat(game: Option<&str>) {
    static WATCHDOG: Once = Once::new();
    WATCHDOG.call_once(|| {
        std::thread::spawn(|| {
            loop {
                std::thread::sleep(std::time::Duration::from_secs(2));
                let beats = HEARTBEATS.lock().unwrap_or_else(|e| e.into_inner());
                if let Some((_, _, game)) = beats.iter().find(|b| b.1.elapsed().as_secs() >= 60) {
                    eprintln!("fuzz watchdog: {game} is stuck inside one response; aborting");
                    std::process::abort();
                }
            }
        });
    });
    let me = std::thread::current().id();
    let mut beats = HEARTBEATS.lock().unwrap_or_else(|e| e.into_inner());
    beats.retain(|b| b.0 != me);
    if let Some(game) = game {
        beats.push((me, std::time::Instant::now(), game.to_string()));
    }
}

fn env_u64(name: &str) -> Option<u64> {
    std::env::var(name).ok().and_then(|s| s.parse().ok())
}

fn json(g: &Game) -> String {
    serde_json::to_string(g).expect("a game serializes")
}

fn ctx(g: &Game) -> String {
    let tail: Vec<String> = g
        .log()
        .iter()
        .rev()
        .take(14)
        .rev()
        .map(|e| format!("{e:?}"))
        .collect();
    let t = g.turn();
    format!(
        "{}'s turn {} ({:?}, extra {:?}); pending {:?}; log tail: {}",
        t.player,
        g.player(t.player).turns_taken,
        t.phase,
        t.extra,
        g.pending(),
        tail.join(", ")
    )
}

// ---- The biased agent ----

/// Plays whatever it can, accepts most optional effects, and buys Kingdom cards and
/// landscapes in preference to ending the phase. `greed` 0 is uniformly random play.
struct Biased {
    rng: ChaCha8Rng,
    greed: f64,
}

impl Biased {
    fn count(&mut self, min: usize, max: usize) -> usize {
        if min >= max {
            return min;
        }
        let x: f64 = self.rng.random();
        if x < 0.5 * self.greed {
            max
        } else if x > 0.93 {
            min
        } else {
            self.rng.random_range(min..=max)
        }
    }

    fn turn_move(&mut self, g: &Game, phase: Phase, moves: &[Move]) -> Move {
        let rng = &mut self.rng;
        if !rng.random_bool(self.greed) {
            return *moves.choose(rng).unwrap();
        }
        let plays: Vec<Move> = moves
            .iter()
            .copied()
            .filter(|m| matches!(m, Move::Play(_)))
            .collect();
        let has = |m: Move| moves.contains(&m);
        if !plays.is_empty() && rng.random_bool(0.96) {
            return *plays.choose(rng).unwrap();
        }
        let p = g.turn().player;
        match phase {
            Phase::Action => {
                let stuck =
                    g.turn().actions == 0 && g.hand(p).iter().any(|&c| g.is(c, Types::ACTION));
                if has(Move::Villager) && rng.random_bool(if stuck { 0.9 } else { 0.1 }) {
                    return Move::Villager;
                }
                if has(Move::PayDebt) && rng.random_bool(0.4) {
                    return Move::PayDebt;
                }
            }
            Phase::Buy => {
                if has(Move::Coffers) && rng.random_bool(0.55) {
                    return Move::Coffers;
                }
                if has(Move::PayDebt) && rng.random_bool(0.9) {
                    return Move::PayDebt;
                }
                let buys: Vec<(Move, f64)> = moves
                    .iter()
                    .filter_map(|&m| {
                        let w = match m {
                            Move::Buy(pile) => {
                                let top = g.pile_top(pile)?;
                                let card = g.card(top);
                                let coins = g.cost_of(top).coins as f64;
                                match card {
                                    Card::Curse => 0.03,
                                    Card::Copper => 0.1,
                                    Card::Estate => 0.2,
                                    c if c.def().basic => 1.0 + coins / 3.0,
                                    _ => 3.0 + coins,
                                }
                            }
                            Move::BuyLandscape(_) => 5.0,
                            _ => return None,
                        };
                        Some((m, w))
                    })
                    .collect();
                if !buys.is_empty() && rng.random_bool(0.94) {
                    return buys.choose_weighted(rng, |b| b.1).unwrap().0;
                }
            }
            _ => {}
        }
        Move::EndPhase
    }

    fn decide(&mut self, g: &Game, d: &Decision) -> Response {
        match &d.kind {
            DecisionKind::Turn { phase, moves } => Response::Move(self.turn_move(g, *phase, moves)),
            DecisionKind::Cards {
                candidates,
                min,
                max,
                ..
            } => {
                let n = self.count(*min, *max);
                let mut chosen = candidates.clone();
                chosen.shuffle(&mut self.rng);
                chosen.truncate(n);
                Response::Cards(chosen)
            }
            DecisionKind::Pile { piles, optional } => {
                let skip = *optional && self.rng.random_bool(0.3 - 0.25 * self.greed);
                Response::Pile(piles.choose(&mut self.rng).copied().filter(|_| !skip))
            }
            DecisionKind::Choose {
                options, min, max, ..
            } => {
                let greedy = self.rng.random_bool(self.greed);
                let rng = &mut self.rng;
                let pick = |o: Opt| Response::Options(vec![o]);
                let triggers: Vec<Opt> = options
                    .iter()
                    .copied()
                    .filter(|o| matches!(o, Opt::Trigger(_)))
                    .collect();
                let ways: Vec<Opt> = options
                    .iter()
                    .copied()
                    .filter(|o| matches!(o, Opt::Card(c) if c.kind() == Kind::Way))
                    .collect();
                if greedy && *max == 1 && !triggers.is_empty() && rng.random_bool(0.92) {
                    return pick(*triggers.choose(rng).unwrap());
                }
                if greedy && *max == 1 && options.contains(&Opt::Yes) && rng.random_bool(0.85) {
                    return pick(Opt::Yes);
                }
                if greedy && *max == 1 && !ways.is_empty() && rng.random_bool(0.5) {
                    return pick(*ways.choose(rng).unwrap());
                }
                let n = self.count(*min, *max);
                let mut chosen = options.clone();
                chosen.shuffle(&mut self.rng);
                chosen.truncate(n);
                Response::Options(chosen)
            }
            DecisionKind::NameCard => {
                if self.rng.random_bool(0.8) {
                    let id = CardId(self.rng.random_range(0..g.card_count() as u32));
                    Response::Name(g.printed(id))
                } else {
                    let cards: Vec<Card> = Card::ALL
                        .iter()
                        .copied()
                        .filter(|c| c.kind() == Kind::Card)
                        .collect();
                    Response::Name(*cards.choose(&mut self.rng).unwrap())
                }
            }
            DecisionKind::Number { min, max } => {
                let n = self.count(0, (*max - *min) as usize);
                Response::Number(*min + n as i32)
            }
        }
    }
}

// ---- Kingdoms ----

fn pool() -> Vec<Card> {
    // Possession is unimplemented: setup rejects it.
    Card::kingdom_cards()
        .filter(|c| IMPLEMENTED.contains(&c.set()) && *c != Card::Possession)
        .collect()
}

/// The cards of the Kingdom pile named `k`, and `k` itself.
fn pile_cards(k: Card) -> Vec<Card> {
    let mut v = k.pile_members();
    if !v.contains(&k) {
        v.push(k);
    }
    v
}

fn pile_text(k: Card) -> String {
    pile_cards(k)
        .iter()
        .map(|c| c.text().to_lowercase())
        .collect::<Vec<_>>()
        .join("\n")
}

fn ty(name: &str) -> Types {
    Types::parse(name).unwrap_or_else(|| panic!("no type {name}"))
}

/// Kingdom piles with a card whose text contains one of `pats` (lower case).
fn with_text(pats: &[&str]) -> Vec<Card> {
    pool()
        .into_iter()
        .filter(|&k| {
            let t = pile_text(k);
            pats.iter().any(|p| t.contains(p))
        })
        .collect()
}

fn with_type(names: &[&str]) -> Vec<Card> {
    let types = names.iter().fold(Types::empty(), |t, n| t | ty(n));
    pool()
        .into_iter()
        .filter(|&k| pile_cards(k).iter().any(|c| c.is(types)))
        .collect()
}

fn landscapes(kinds: &[Kind]) -> Vec<Card> {
    Card::ALL
        .iter()
        .copied()
        .filter(|c| kinds.contains(&c.kind()) && IMPLEMENTED.contains(&c.set()))
        .collect()
}

const DEALT: &[Kind] = &[
    Kind::Event,
    Kind::Landmark,
    Kind::Project,
    Kind::Way,
    Kind::Trait,
];

fn landscapes_with_text(pats: &[&str]) -> Vec<Card> {
    landscapes(DEALT)
        .into_iter()
        .filter(|l| {
            let t = l.text().to_lowercase();
            pats.iter().any(|p| t.contains(p))
        })
        .collect()
}

/// A kingdom of `forced`, up to `n_core` of `core`, and random piles to make 10.
fn kingdom(rng: &mut ChaCha8Rng, forced: &[Card], core: &[Card], n_core: usize) -> Vec<Card> {
    let mut k = forced.to_vec();
    let mut core: Vec<Card> = core.iter().copied().filter(|c| !k.contains(c)).collect();
    core.sort();
    core.dedup();
    core.shuffle(rng);
    core.truncate(n_core.min(10usize.saturating_sub(k.len())));
    k.extend(core);
    let mut rest = pool();
    rest.shuffle(rng);
    for c in rest {
        if k.len() >= 10 {
            break;
        }
        if !k.contains(&c) {
            k.push(c);
        }
    }
    k
}

/// `forced` plus up to `n` of `from` (at most one Way in all).
fn deal_landscapes(rng: &mut ChaCha8Rng, forced: &[Card], from: &[Card], n: usize) -> Vec<Card> {
    let mut out = forced.to_vec();
    let mut from = from.to_vec();
    from.shuffle(rng);
    for l in from {
        if out.len() >= forced.len() + n {
            break;
        }
        let second_way = l.kind() == Kind::Way && out.iter().any(|o| o.kind() == Kind::Way);
        if !out.contains(&l) && !second_way {
            out.push(l);
        }
    }
    out
}

#[derive(Default)]
struct Kingdom {
    cards: Vec<Card>,
    landscapes: Vec<Card>,
    choices: BTreeMap<Card, Vec<Card>>,
    /// Only decks of Kingdom cards and money: no extra tokens, mats or small piles.
    plain: bool,
}

type Family = fn(&mut ChaCha8Rng, u64) -> Kingdom;

fn some_landscapes(rng: &mut ChaCha8Rng, max: usize) -> Vec<Card> {
    let n = rng.random_range(0..=max);
    deal_landscapes(rng, &[], &landscapes(DEALT), n)
}

fn stack(rng: &mut ChaCha8Rng, forced: &[Card], core: &[Card], n_core: usize) -> Kingdom {
    Kingdom {
        cards: kingdom(rng, forced, core, n_core),
        landscapes: some_landscapes(rng, 3),
        ..Kingdom::default()
    }
}

fn fam_mixed(rng: &mut ChaCha8Rng, seed: u64) -> Kingdom {
    let config = GameConfig::random(2, IMPLEMENTED, seed);
    let mut cards = config.kingdom;
    if let Some(i) = cards.iter().position(|&c| c == Card::Possession) {
        cards[i] = *pool().iter().find(|c| !cards.contains(c)).unwrap();
    }
    let removed: Vec<Card> = pool().into_iter().filter(|c| c.def().removed).collect();
    for i in 0..rng.random_range(0..=3) {
        let card = *removed.choose(rng).unwrap();
        if !cards.contains(&card) {
            cards[i] = card;
        }
    }
    Kingdom {
        cards,
        landscapes: config.landscapes,
        ..Kingdom::default()
    }
}

fn fam_landscapes(rng: &mut ChaCha8Rng, _: u64) -> Kingdom {
    let n = rng.random_range(4..=9);
    Kingdom {
        cards: kingdom(rng, &[], &[], 0),
        landscapes: deal_landscapes(rng, &[], &landscapes(DEALT), n),
        ..Kingdom::default()
    }
}

fn fam_thrones_durations(rng: &mut ChaCha8Rng, _: u64) -> Kingdom {
    let mut core = with_text(&["twice", "three times", "play it again", "replay"]);
    core.truncate(usize::MAX);
    let durations = with_type(&["Duration"]);
    let mut cards = kingdom(rng, &[], &core, 4);
    let more = kingdom(rng, &cards.clone()[..cards.len().min(4)], &durations, 6);
    cards = more;
    Kingdom {
        cards,
        landscapes: some_landscapes(rng, 3),
        ..Kingdom::default()
    }
}

fn fam_commands_oneshots(rng: &mut ChaCha8Rng, _: u64) -> Kingdom {
    let commands = with_type(&["Command"]);
    let oneshots = with_text(&[
        "trash this",
        "return this to",
        "exile this",
        "set this aside",
        "put this on your tavern mat",
    ]);
    let first = kingdom(rng, &[], &commands, 4);
    Kingdom {
        cards: kingdom(rng, &first[..4.min(first.len())], &oneshots, 6),
        landscapes: some_landscapes(rng, 3),
        ..Kingdom::default()
    }
}

fn fam_gains(rng: &mut ChaCha8Rng, _: u64) -> Kingdom {
    let core = with_text(&[
        "would gain",
        "when you gain",
        "when any player gains",
        "when another player gains",
        "whenever you gain",
        "next time you gain",
        "gains a",
        "when you buy",
    ]);
    let ls = landscapes_with_text(&["gain"]);
    let n = rng.random_range(1..=5);
    Kingdom {
        cards: kingdom(rng, &[], &core, 10),
        landscapes: deal_landscapes(rng, &[], &ls, n),
        ..Kingdom::default()
    }
}

fn fam_costs(rng: &mut ChaCha8Rng, _: u64) -> Kingdom {
    let reducers = with_text(&["less", "costs $"]);
    let remodels = with_text(&[
        "costing exactly",
        "costing up to",
        "more than it",
        "more than the trashed",
        "costing less than",
        "per $1 it costs",
        "its cost",
    ]);
    let first = kingdom(rng, &[], &reducers, 4);
    let ls = landscapes_with_text(&["cost", "less"]);
    let n = rng.random_range(0..=4);
    Kingdom {
        cards: kingdom(rng, &first[..4.min(first.len())], &remodels, 6),
        landscapes: deal_landscapes(rng, &[], &ls, n),
        ..Kingdom::default()
    }
}

fn fam_attacks(rng: &mut ChaCha8Rng, _: u64) -> Kingdom {
    let reactions = with_type(&["Reaction"]);
    let mut blockers = reactions.clone();
    blockers.extend([Card::Lighthouse, Card::Guardian, Card::Peasant]);
    let first = kingdom(rng, &[], &blockers, 4);
    Kingdom {
        cards: kingdom(
            rng,
            &first[..4.min(first.len())],
            &with_type(&["Attack"]),
            6,
        ),
        landscapes: some_landscapes(rng, 3),
        ..Kingdom::default()
    }
}

fn fam_extra_turns(rng: &mut ChaCha8Rng, _: u64) -> Kingdom {
    let core = with_text(&["extra turn", "another turn", "skip", "next turn"]);
    let ls = landscapes_with_text(&["extra turn", "another turn", "next turn"]);
    let n = rng.random_range(1..=4);
    let mut k = Kingdom {
        cards: kingdom(rng, &[], &core, 8),
        landscapes: deal_landscapes(rng, &[], &ls, n),
        ..Kingdom::default()
    };
    if rng.random_bool(0.3) {
        k.landscapes.push(Card::IslandFolk);
        if !with_type(&["Liaison"]).iter().any(|c| k.cards.contains(c)) {
            k.cards[9] = Card::Underling;
        }
    }
    k
}

fn fam_ways(rng: &mut ChaCha8Rng, seed: u64) -> Kingdom {
    let ways = landscapes(&[Kind::Way]);
    let way = ways[seed as usize % ways.len()];
    let actions: Vec<Card> = with_type(&["Action"]);
    let others = landscapes(&[Kind::Event, Kind::Landmark, Kind::Project, Kind::Trait]);
    let n = rng.random_range(0..=3);
    Kingdom {
        cards: kingdom(rng, &[], &actions, 10),
        landscapes: deal_landscapes(rng, &[way], &others, n),
        ..Kingdom::default()
    }
}

/// Way of the Chameleon over cards that give +$ or +Cards in the middle of their
/// instructions or later in the turn, with what replays them, lets them choose more
/// (Elder), plays them on other players' turns or makes Treasures Actions.
fn fam_chameleon(rng: &mut ChaCha8Rng, seed: u64) -> Kingdom {
    let givers = with_text(&[
        "for +$",
        "+$1 per",
        "+$2 per",
        ", +$",
        ": +$",
        "+1 card per",
        "this turn",
        "the next time",
        "until your next turn",
        "each of your turns",
        "choose one",
        "choose two",
    ]);
    let mut forced = vec![];
    let mut ls = vec![Card::WayOfTheChameleon];
    if seed.is_multiple_of(2) {
        forced.push(Card::Townsfolk);
    }
    let mut also = with_text(&["twice", "three times", "replay"]);
    if seed.is_multiple_of(3) {
        ls.push(Card::Enlightenment);
        also = with_type(&["Omen"]);
    } else if seed % 3 == 1 {
        also.extend(with_type(&["Reaction"]));
    }
    let n = forced.len() + 3;
    let first = kingdom(rng, &forced, &also, 3);
    ls.extend(
        some_landscapes(rng, 2)
            .into_iter()
            .filter(|l| l.kind() != Kind::Way),
    );
    Kingdom {
        cards: kingdom(rng, &first[..n.min(first.len())], &givers, 10),
        landscapes: ls,
        ..Kingdom::default()
    }
}

fn fam_piles(rng: &mut ChaCha8Rng, _: u64) -> Kingdom {
    let split: Vec<Card> = pool()
        .into_iter()
        .filter(|k| k.pile_members().len() > 1)
        .collect();
    let touching = with_text(&[
        "pile",
        "supply",
        "rotate",
        "from the trash",
        "non-supply",
        "not in the supply",
    ]);
    let first = kingdom(rng, &[], &split, 5);
    let ls = landscapes_with_text(&["pile", "supply"]);
    let n = rng.random_range(0..=4);
    Kingdom {
        cards: kingdom(rng, &first[..5.min(first.len())], &touching, 5),
        landscapes: deal_landscapes(rng, &[], &ls, n),
        ..Kingdom::default()
    }
}

fn fam_prophecies(rng: &mut ChaCha8Rng, seed: u64) -> Kingdom {
    let prophecies = landscapes(&[Kind::Prophecy]);
    // Divine Wind every other game.
    let prophecy = if seed.is_multiple_of(2) {
        Card::DivineWind
    } else {
        prophecies[(seed / 2) as usize % prophecies.len()]
    };
    let omens = with_type(&["Omen"]);
    let n = rng.random_range(3..=6);
    let first = kingdom(rng, &[], &omens, n);
    let shadows = with_type(&["Shadow"]);
    let rising: Vec<Card> = pool()
        .into_iter()
        .filter(|c| c.set() == Set::RisingSun || shadows.contains(c))
        .collect();
    let m = rng.random_range(0..=4);
    let mut ls = vec![prophecy];
    ls.extend(some_landscapes(rng, 3));
    Kingdom {
        cards: kingdom(rng, &first[..n.min(first.len())], &rising, m),
        landscapes: ls,
        ..Kingdom::default()
    }
}

fn fam_markets(rng: &mut ChaCha8Rng, seed: u64) -> Kingdom {
    let mut forced = vec![Card::BlackMarket, Card::YoungWitch];
    if seed.is_multiple_of(3) {
        forced.push(Card::Ferryman);
    }
    let cards = kingdom(rng, &forced, &[], 0);
    let mut choices = BTreeMap::new();
    let unused: Vec<Card> = pool()
        .into_iter()
        .filter(|c| !cards.contains(c) && !c.def().removed && c.kind() == Kind::Card)
        .collect();
    let costing = |lo: u8, hi: u8| -> Vec<Card> {
        unused
            .iter()
            .copied()
            .filter(|c| {
                c.cost()
                    .is_some_and(|x| x.potions == 0 && x.debt == 0 && (lo..=hi).contains(&x.coins))
            })
            .collect()
    };
    let mut ls = some_landscapes(rng, 3);
    if rng.random_bool(0.6) {
        let bane = *costing(2, 3).choose(rng).unwrap();
        choices.insert(Card::YoungWitch, vec![bane]);
        if rng.random_bool(0.5) {
            let mut deck: Vec<Card> = unused.iter().copied().filter(|&c| c != bane).collect();
            deck.shuffle(rng);
            deck.truncate(rng.random_range(1..=60));
            choices.insert(Card::BlackMarket, deck);
        }
    }
    if rng.random_bool(0.3) && !ls.iter().any(|l| l.kind() == Kind::Way) {
        ls.push(Card::WayOfTheMouse);
    }
    Kingdom {
        cards,
        landscapes: ls,
        choices,
        ..Kingdom::default()
    }
}

/// Cards held outside the Supply all game (Ferryman's pile, the cards Way of the Mouse
/// and Riverboat set aside, the Bane, the Black Market deck), each an explicit choice
/// among every card the rules allow, removed first-edition cards included.
fn fam_holders(rng: &mut ChaCha8Rng, seed: u64) -> Kingdom {
    let mut forced = vec![];
    for c in [
        Card::Ferryman,
        Card::Riverboat,
        Card::BlackMarket,
        Card::YoungWitch,
    ] {
        if rng.random_bool(0.6) {
            forced.push(c);
        }
    }
    let mouse = !seed.is_multiple_of(3);
    if !mouse && !forced.contains(&Card::Ferryman) {
        forced.push(Card::Ferryman);
    }
    let cards = kingdom(rng, &forced, &[], 0);
    let mut unused: Vec<Card> = pool()
        .into_iter()
        .filter(|c| !cards.contains(c) && c.kind() == Kind::Card)
        .collect();
    unused.sort();
    let duration = ty("Duration");
    // Every candidate gets its turn as the seeds go by.
    let mut turn = seed as usize / 3;
    let mut take = |ok: &dyn Fn(Card, Cost) -> bool| -> Option<Vec<Card>> {
        let fits: Vec<usize> = (0..unused.len())
            .filter(|&i| {
                let c = unused[i];
                c.cost()
                    .is_some_and(|x| x.potions == 0 && x.debt == 0 && ok(c, x))
            })
            .collect();
        let i = *fits.get(turn % fits.len().max(1))?;
        turn /= fits.len().max(1);
        turn += seed as usize % 7;
        Some(vec![unused.remove(i)])
    };
    let mut choices = BTreeMap::new();
    let mut ls = vec![];
    if mouse {
        ls.push(Card::WayOfTheMouse);
        choices.extend(
            take(&|c, x| c.is(Types::ACTION) && (2..=3).contains(&x.coins))
                .map(|c| (Card::WayOfTheMouse, c)),
        );
    }
    if cards.contains(&Card::Ferryman) {
        choices.extend(take(&|_, x| (3..=4).contains(&x.coins)).map(|c| (Card::Ferryman, c)));
    }
    if cards.contains(&Card::Riverboat) {
        choices.extend(
            take(&|c, x| c.is(Types::ACTION) && !c.is(duration) && x.coins == 5)
                .map(|c| (Card::Riverboat, c)),
        );
    }
    if cards.contains(&Card::YoungWitch) {
        choices.extend(take(&|_, x| (2..=3).contains(&x.coins)).map(|c| (Card::YoungWitch, c)));
    }
    if cards.contains(&Card::BlackMarket) && rng.random_bool(0.7) {
        unused.shuffle(rng);
        let n = rng.random_range(1..=40).min(unused.len());
        choices.insert(Card::BlackMarket, unused[..n].to_vec());
    }
    ls.extend(
        some_landscapes(rng, 3)
            .into_iter()
            .filter(|l| l.kind() != Kind::Way || !mouse),
    );
    Kingdom {
        cards,
        landscapes: ls,
        choices,
        ..Kingdom::default()
    }
}

/// Configurations no one would deal: kingdoms of 0 to 25 piles, a dozen landscapes with
/// several Ways among them.
fn fam_odd(rng: &mut ChaCha8Rng, _: u64) -> Kingdom {
    let mut cards = pool();
    cards.shuffle(rng);
    cards.truncate(*[0usize, 1, 2, 3, 5, 8, 12, 16, 25].choose(rng).unwrap());
    let mut ls = landscapes(DEALT);
    ls.shuffle(rng);
    ls.truncate(rng.random_range(0..=14));
    Kingdom {
        cards,
        landscapes: ls,
        ..Kingdom::default()
    }
}

/// Cards that leave play in their own way, and cards played on other players' turns.
fn fam_leaving_play(rng: &mut ChaCha8Rng, _: u64) -> Kingdom {
    let leavers = with_text(&[
        "discard this from play",
        "discard this",
        "from play",
        "put this onto your deck",
        "put this into your hand",
        "return this",
        "in play",
    ]);
    let mut off_turn = with_type(&["Reaction"]);
    off_turn.extend(with_text(&["play this", "play it", "you may play"]));
    let first = kingdom(rng, &[], &leavers, 5);
    let pick = [
        Card::WayOfTheFrog,
        Card::Reckless,
        Card::Tireless,
        Card::Journey,
        Card::Panic,
        Card::Hasty,
        Card::Patient,
    ];
    let mut ls: Vec<Card> = pick.into_iter().filter(|_| rng.random_bool(0.45)).collect();
    ls.extend(
        some_landscapes(rng, 2)
            .into_iter()
            .filter(|l| l.kind() != Kind::Way && !pick.contains(l)),
    );
    Kingdom {
        cards: kingdom(rng, &first[..5.min(first.len())], &off_turn, 5),
        landscapes: ls,
        ..Kingdom::default()
    }
}

/// Everything that waits for the start of a turn: set-aside cards to play or take, cards
/// to play then, and the Durations.
fn fam_start_of_turn(rng: &mut ChaCha8Rng, _: u64) -> Kingdom {
    let core = with_text(&[
        "at the start of your next turn",
        "start of each of your turns",
        "at the start of your turn",
        "set it aside",
        "set aside",
        "setting it aside",
    ]);
    let ls = landscapes_with_text(&[
        "start of your next turn",
        "start of each of your turns",
        "start of your turn",
        "set aside",
        "set it aside",
    ]);
    let n = rng.random_range(2..=6);
    Kingdom {
        cards: kingdom(rng, &[], &core, 10),
        landscapes: deal_landscapes(rng, &[], &ls, n),
        ..Kingdom::default()
    }
}

fn fam_night(rng: &mut ChaCha8Rng, _: u64) -> Kingdom {
    let core = with_type(&["Night", "Fate", "Doom"]);
    Kingdom {
        cards: kingdom(rng, &[], &core, 8),
        landscapes: some_landscapes(rng, 3),
        ..Kingdom::default()
    }
}

fn fam_mats_tokens(rng: &mut ChaCha8Rng, _: u64) -> Kingdom {
    let mut core = with_type(&["Reserve", "Traveller", "Gathering"]);
    core.extend(with_text(&[
        "exile", "token", " mat", "journey", "coffers", "villager",
    ]));
    let ls = landscapes_with_text(&["token", "exile", "coffers", "villager", " mat"]);
    let n = rng.random_range(1..=5);
    Kingdom {
        cards: kingdom(rng, &[], &core, 9),
        landscapes: deal_landscapes(rng, &[], &ls, n),
        ..Kingdom::default()
    }
}

fn fam_debt_vp(rng: &mut ChaCha8Rng, _: u64) -> Kingdom {
    let mut core = with_text(&["debt", " vp", "overpay"]);
    core.extend(pool().into_iter().filter(|k| {
        pile_cards(*k)
            .iter()
            .any(|c| c.cost().is_some_and(|x| x.debt > 0))
    }));
    let ls = landscapes_with_text(&["debt", "vp"]);
    let n = rng.random_range(1..=6);
    Kingdom {
        cards: kingdom(rng, &[], &core, 9),
        landscapes: deal_landscapes(rng, &[], &ls, n),
        ..Kingdom::default()
    }
}

fn fam_allies(rng: &mut ChaCha8Rng, seed: u64) -> Kingdom {
    let allies = landscapes(&[Kind::Ally]);
    let ally = allies[seed as usize % allies.len()];
    let liaisons = with_type(&["Liaison"]);
    let n = rng.random_range(2..=5);
    let first = kingdom(rng, &[], &liaisons, n);
    let allies_set: Vec<Card> = pool()
        .into_iter()
        .filter(|c| c.set() == Set::Allies)
        .collect();
    let mut ls = vec![ally];
    ls.extend(some_landscapes(rng, 2));
    Kingdom {
        cards: kingdom(rng, &first[..n.min(first.len())], &allies_set, 4),
        landscapes: ls,
        ..Kingdom::default()
    }
}

fn fam_traits(rng: &mut ChaCha8Rng, _: u64) -> Kingdom {
    let traits = landscapes(&[Kind::Trait]);
    let n = rng.random_range(2..=6);
    let cards = kingdom(rng, &[], &with_type(&["Action", "Treasure"]), 10);
    let mut ls = deal_landscapes(rng, &[], &traits, n);
    let mut choices = BTreeMap::new();
    if rng.random_bool(0.5) {
        let mut piles: Vec<Card> = cards
            .iter()
            .copied()
            .filter(|c| c.is(Types::ACTION | Types::TREASURE))
            .collect();
        piles.shuffle(rng);
        for (&t, &pile) in ls.iter().zip(&piles) {
            choices.insert(t, vec![pile]);
        }
    }
    ls.extend(some_landscapes(rng, 2));
    ls.dedup();
    let mut seen = vec![];
    ls.retain(|l| {
        let new = !seen.contains(l);
        seen.push(*l);
        new
    });
    Kingdom {
        cards,
        landscapes: ls,
        choices,
        ..Kingdom::default()
    }
}

fn fam_plunder(rng: &mut ChaCha8Rng, _: u64) -> Kingdom {
    let mut core = with_text(&["loot", "the next time", "next time"]);
    core.extend(with_type(&["Shadow"]));
    core.extend(pool().into_iter().filter(|c| c.set() == Set::Plunder));
    let ls: Vec<Card> = landscapes(DEALT)
        .into_iter()
        .filter(|l| l.set() == Set::Plunder || l.set() == Set::RisingSun)
        .collect();
    let n = rng.random_range(1..=5);
    Kingdom {
        cards: kingdom(rng, &[], &core, 9),
        landscapes: deal_landscapes(rng, &[], &ls, n),
        ..Kingdom::default()
    }
}

/// Simple kingdoms whose scores an independent recount can check.
fn fam_scoring(rng: &mut ChaCha8Rng, _: u64) -> Kingdom {
    let alt_vp = [
        Card::Gardens,
        Card::Duke,
        Card::Vineyard,
        Card::Fairgrounds,
        Card::SilkRoad,
        Card::Feodum,
        Card::DistantLands,
        Card::Castles,
        Card::Clashes,
        Card::Marchland,
        Card::Island,
        Card::NativeVillage,
        Card::Haven,
        Card::Monument,
        Card::Bishop,
        Card::Goons,
        Card::Shepherd,
        Card::Cemetery,
        Card::Tournament,
        Card::Mill,
        Card::Farm,
        Card::Nobles,
        Card::Tunnel,
        Card::Farmland,
    ];
    Kingdom {
        cards: kingdom(rng, &[], &alt_vp, 8),
        landscapes: vec![],
        plain: true,
        ..Kingdom::default()
    }
}

// ---- Rich starting states ----

/// Lower-case text of every card and landscape in the game.
fn game_text(g: &Game) -> String {
    let mut cards: BTreeSet<Card> = g.cards.iter().copied().collect();
    cards.extend(g.landscapes.iter().copied());
    cards.extend(g.prophecy.map(|(p, _)| p));
    cards
        .iter()
        .map(|c| format!("{}\n{}", c.name(), c.text()).to_lowercase())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Seeds an unstarted game: decks with the Kingdom cards and money, and (`level` 2) the
/// tokens, mats, Projects and Artifacts the cards in the game use, and small piles.
fn enrich(g: &mut Game, rng: &mut ChaCha8Rng, level: u8) {
    if level == 0 {
        return;
    }
    let n = g.num_players();
    let text = game_text(g);
    let has = |s: &str| text.contains(s);
    let in_game = |g: &Game, c: Card| g.cards.contains(&c) || g.landscapes.contains(&c);
    let kingdom_piles: Vec<Card> = g
        .piles
        .iter()
        .filter(|p| p.in_supply && p.card.def().kingdom)
        .map(|p| p.card)
        .collect();
    let skip_piles = [g.ext.cornucopia_guilds.ferryman, g.ext.promos.black_market];
    for p in 0..n {
        let pid = PlayerId(p as u8);
        let mut extra: Vec<Card> = vec![];
        for &k in &kingdom_piles {
            let members = k.pile_members();
            if members.is_empty() {
                continue;
            }
            for _ in 0..rng.random_range(1..=2) {
                extra.push(*members.choose(rng).unwrap());
            }
        }
        extra.extend(std::iter::repeat_n(Card::Gold, rng.random_range(0..=3)));
        if g.supply_pile(Card::Platinum).is_some() {
            extra.extend(std::iter::repeat_n(Card::Platinum, rng.random_range(0..=2)));
        }
        if g.supply_pile(Card::Potion).is_some() {
            extra.extend(std::iter::repeat_n(Card::Potion, rng.random_range(1..=3)));
        }
        for c in extra {
            g.create(c, Zone::Deck(pid));
        }
        // Real cards from the non-Supply piles (Horses, Spoils, Loot, Prizes, ...).
        for i in 0..g.piles.len() {
            let pile = &g.piles[i];
            let loot = pile.card == Card::Loot;
            if pile.in_supply
                || pile.removed
                || (pile.hidden && !loot)
                || pile.cards.len() < 2
                || skip_piles.contains(&Some(PileId(i as u8)))
            {
                continue;
            }
            if rng.random_bool(0.35) {
                let any = *pile.cards.choose(rng).unwrap();
                g.move_card(any, Zone::Deck(pid), Pos::Top);
            }
        }
        let pl = &mut g.players[p];
        let mut all = std::mem::take(&mut pl.deck);
        all.append(&mut pl.hand);
        all.shuffle(rng);
        let hand = all.split_off(all.len() - 5.min(all.len()));
        for &c in &all {
            g.zones[c.index()] = Zone::Deck(pid);
        }
        for &c in &hand {
            g.zones[c.index()] = Zone::Hand(pid);
        }
        g.players[p].deck = all;
        g.players[p].hand = hand;
    }
    if level < 2 {
        return;
    }
    let token_sources: [(Card, &[Token]); 7] = [
        (
            Card::Peasant,
            &[
                Token::PlusCard,
                Token::PlusAction,
                Token::PlusBuy,
                Token::PlusCoin,
            ],
        ),
        (Card::Pathfinding, &[Token::PlusCard]),
        (Card::LostArts, &[Token::PlusAction]),
        (Card::Seaway, &[Token::PlusBuy]),
        (Card::Training, &[Token::PlusCoin]),
        (Card::Ferry, &[Token::MinusCost]),
        (Card::Plan, &[Token::Trashing]),
    ];
    let action_piles: Vec<PileId> = g.supply_piles_where(|t, _| t.intersects(Types::ACTION));
    let reserve = ty("Reserve");
    for p in 0..n {
        let pid = PlayerId(p as u8);
        if has("coffers") && rng.random_bool(0.6) {
            g.players[p].coffers = rng.random_range(1..=5);
        }
        if has("villager") && rng.random_bool(0.6) {
            g.players[p].villagers = rng.random_range(1..=4);
        }
        if has("favor") && rng.random_bool(0.6) {
            g.players[p].favors = rng.random_range(1..=6);
        }
        if has("debt") && rng.random_bool(0.25) {
            g.players[p].debt = rng.random_range(1..=4);
        }
        if rng.random_bool(0.2) {
            g.players[p].vp_tokens = rng.random_range(1..=5);
        }
        let to_mat = |g: &mut Game, rng: &mut ChaCha8Rng, mat: Mat, max: usize| {
            for _ in 0..rng.random_range(1..=max) {
                if let Some(&c) = g.players[p].deck.choose(rng) {
                    g.move_card(c, Zone::Mat(pid, mat), Pos::Top);
                }
            }
        };
        if has("exile") && rng.random_bool(0.5) {
            to_mat(g, rng, Mat::Exile, 3);
        }
        if in_game(g, Card::Island) && rng.random_bool(0.5) {
            to_mat(g, rng, Mat::Island, 2);
        }
        if in_game(g, Card::NativeVillage) && rng.random_bool(0.5) {
            to_mat(g, rng, Mat::NativeVillage, 3);
        }
        for c in g.players[p].deck.clone() {
            if g.printed(c).is(reserve) && rng.random_bool(0.5) {
                g.move_card(c, Zone::Mat(pid, Mat::Tavern), Pos::Top);
            }
        }
        for l in g.landscapes.clone() {
            if l.kind() == Kind::Project && rng.random_bool(0.35) {
                g.players[p].projects.push(l);
            }
        }
        for (source, tokens) in token_sources {
            if !in_game(g, source) || action_piles.is_empty() {
                continue;
            }
            for &token in tokens {
                if rng.random_bool(0.4) && !g.players[p].tokens.contains_key(&token) {
                    let pile = *action_piles.choose(rng).unwrap();
                    g.players[p].tokens.insert(token, Some(pile));
                    g.piles[pile.index()].tokens.push((pid, token));
                }
            }
        }
        if [Card::Relic, Card::Borrow, Card::Raid]
            .iter()
            .any(|&c| in_game(g, c))
        {
            g.players[p].minus_card = rng.random_bool(0.25);
        }
        if [Card::BridgeTroll, Card::Ball]
            .iter()
            .any(|&c| in_game(g, c))
        {
            g.players[p].minus_coin = rng.random_bool(0.25);
        }
    }
    for i in 0..g.artifacts.len() {
        if rng.random_bool(0.4) {
            g.artifacts[i].1 = Some(PlayerId(rng.random_range(0..n) as u8));
        }
    }
    if rng.random_bool(0.4) {
        let supply: Vec<usize> = (0..g.piles.len())
            .filter(|&i| g.piles[i].in_supply)
            .collect();
        for _ in 0..rng.random_range(2..=5) {
            let i = *supply.choose(rng).unwrap();
            let keep = rng.random_range(1..=3);
            while g.piles[i].cards.len() > keep {
                let bottom = g.piles[i].cards[0];
                g.move_card(bottom, Zone::Trash, Pos::Top);
            }
        }
    }
}

// ---- Trace tracking, independent of `Knowledge` ----

fn sees(turn: (PlayerId, PlayerId), v: PlayerId, p: PlayerId) -> bool {
    v == p || turn == (p, v)
}

fn public_zone(g: &Game, zone: Zone) -> bool {
    match zone {
        Zone::InPlay(_) | Zone::Trash => true,
        Zone::Mat(_, mat) => mat != Mat::NativeVillage,
        Zone::Pile(pile) => !g.pile(pile).hidden,
        Zone::Deck(_) | Zone::Hand(_) | Zone::Discard(_) | Zone::SetAside(_) | Zone::Looking(_) => {
            false
        }
    }
}

fn private_zone(turn: (PlayerId, PlayerId), zone: Zone, v: PlayerId) -> bool {
    match zone {
        Zone::Hand(p) | Zone::SetAside(p) | Zone::Looking(p) | Zone::Mat(p, Mat::NativeVillage) => {
            sees(turn, v, p)
        }
        _ => false,
    }
}

/// What `v` sees of `id` from where it is, not counting what a decision offers them.
fn base_visible(g: &Game, v: PlayerId, id: CardId) -> bool {
    let mine = |p: PlayerId| v == p;
    match g.zone(id) {
        Zone::Hand(p) => mine(p),
        Zone::SetAside(p) | Zone::Looking(p) => mine(p) || g.player(p).face_up.contains(&id),
        Zone::InPlay(_) | Zone::Trash => true,
        Zone::Discard(p) => g.player(p).discard.last() == Some(&id),
        Zone::Pile(pile) => !g.pile(pile).hidden && g.pile_top(pile) == Some(id),
        Zone::Mat(p, mat) => mat != Mat::NativeVillage || mine(p),
        Zone::Deck(_) => false,
    }
}

fn offered_cards(d: &Decision) -> Vec<CardId> {
    match &d.kind {
        DecisionKind::Turn { moves, .. } => moves
            .iter()
            .filter_map(|m| match m {
                Move::Play(c) => Some(*c),
                _ => None,
            })
            .collect(),
        DecisionKind::Cards { candidates, .. } => candidates.clone(),
        _ => vec![],
    }
    .into_iter()
    .chain(d.about.iter().copied())
    .collect()
}

fn zone_kind(zone: Zone) -> &'static str {
    match zone {
        Zone::Deck(_) => "deck",
        Zone::Hand(_) => "hand",
        Zone::Discard(_) => "discard",
        Zone::InPlay(_) => "play",
        Zone::SetAside(_) => "set-aside",
        Zone::Looking(_) => "looking",
        Zone::Mat(_, Mat::NativeVillage) => "native-village",
        Zone::Mat(..) => "mat",
        Zone::Pile(_) => "pile",
        Zone::Trash => "trash",
    }
}

/// Follows the trace: where it says each card is, and which cards each player has been
/// shown and could follow since.
struct Tracker {
    zone: Vec<Option<Zone>>,
    seen: Vec<Vec<bool>>,
    recent: Vec<HashSet<CardId>>,
    turn: (PlayerId, PlayerId),
}

impl Tracker {
    fn new(players: usize) -> Tracker {
        Tracker {
            zone: vec![],
            seen: vec![vec![]; players],
            recent: vec![HashSet::new(); players],
            turn: (PlayerId(0), PlayerId(0)),
        }
    }

    fn apply(&mut self, g: &Game, entry: &Trace, flags: &mut Flags) {
        let n = self.seen.len();
        match entry {
            Trace::Created { card, zone, .. } => {
                let i = card.index();
                if self.zone.len() <= i {
                    self.zone.resize(i + 1, None);
                    for s in &mut self.seen {
                        s.resize(i + 1, false);
                    }
                }
                if self.zone[i].is_some() {
                    flags.bug("trace-created-twice", || format!("{card:?}"));
                }
                self.zone[i] = Some(*zone);
            }
            Trace::Moved { card, from, to, .. } => {
                let i = card.index();
                if self.zone.get(i).copied().flatten() != Some(*from) {
                    flags.bug("trace-move-from-wrong-zone", || {
                        format!(
                            "{card:?} ({}) traced as moving from {from:?}, was in {:?}; {}",
                            g.printed(*card),
                            self.zone.get(i),
                            ctx(g)
                        )
                    });
                }
                if i < self.zone.len() {
                    self.zone[i] = Some(*to);
                }
                let shown = public_zone(g, *from) || public_zone(g, *to);
                let lost = match from {
                    Zone::Hand(_) | Zone::Looking(_) | Zone::Mat(_, Mat::NativeVillage) => true,
                    Zone::Deck(_) | Zone::Discard(_) => {
                        !matches!(entry, Trace::Moved { top: true, .. })
                    }
                    _ => false,
                };
                for v in 0..n {
                    let pv = PlayerId(v as u8);
                    if shown
                        || private_zone(self.turn, *from, pv)
                        || private_zone(self.turn, *to, pv)
                    {
                        self.seen[v][i] = true;
                        self.recent[v].insert(*card);
                    } else if lost && !self.recent[v].contains(card) {
                        self.seen[v][i] = false;
                    }
                }
            }
            Trace::Mixed { cards } => {
                for v in 0..n {
                    for c in cards {
                        self.seen[v][c.index()] = false;
                        self.recent[v].remove(c);
                    }
                }
            }
            Trace::Shown { card, to } => {
                for v in 0..n {
                    if to.is_none_or(|p| sees(self.turn, PlayerId(v as u8), p)) {
                        self.seen[v][card.index()] = true;
                        self.recent[v].insert(*card);
                    }
                }
            }
            Trace::LookedOver { player, cards } => {
                for v in 0..n {
                    if sees(self.turn, PlayerId(v as u8), *player) {
                        for card in cards {
                            self.seen[v][card.index()] = true;
                            self.recent[v].insert(*card);
                        }
                    }
                }
            }
            Trace::Turn { player } => self.turn = (*player, *player),
            Trace::Changed { .. } => {}
        }
    }

    /// At a decision: everyone sees what is visible; audits cards a decision offers that
    /// its player was never shown.
    fn observe(&mut self, g: &Game, flags: &mut Flags) {
        for (i, zone) in self.zone.iter().enumerate() {
            let id = CardId(i as u32);
            if *zone != Some(g.zone(id)) {
                flags.bug("card-moved-without-trace", || {
                    format!(
                        "{id:?} ({}) is in {:?} but the trace says {zone:?}; {}",
                        g.printed(id),
                        g.zone(id),
                        ctx(g)
                    )
                });
            }
        }
        if self.zone.len() != g.card_count() {
            flags.bug("card-created-without-trace", || ctx(g));
            return;
        }
        for v in 0..self.seen.len() {
            for i in 0..self.zone.len() {
                if base_visible(g, PlayerId(v as u8), CardId(i as u32)) {
                    self.seen[v][i] = true;
                }
            }
            self.recent[v].clear();
        }
        let Some(d) = g.pending() else { return };
        let v = d.player;
        for c in offered_cards(d) {
            if !self.seen[v.index()][c.index()] {
                let zone = g.zone(c);
                let whose = match zone.owner() {
                    Some(p) if sees(self.turn, v, p) => "own",
                    Some(_) => "other's",
                    None => "shared",
                };
                let source = d.source.map_or("turn".to_string(), |s| s.to_string());
                flags.audit(
                    format!("offered-unseen:{source}:{whose} {}", zone_kind(zone)),
                    || format!("{c:?} ({}) offered to {v}; {}", g.printed(c), ctx(g)),
                );
                self.seen[v.index()][c.index()] = true;
            }
        }
    }
}

// ---- Oracles ----

fn unique<T: PartialEq>(items: &[T]) -> bool {
    items
        .iter()
        .enumerate()
        .all(|(i, x)| !items[..i].contains(x))
}

fn check_decision(g: &Game, d: &Decision, flags: &mut Flags) {
    let source = d.source.map_or("-".to_string(), |c| c.to_string());
    let t = g.turn();
    if d.player.index() >= g.num_players() {
        flags.bug("decision-for-nobody", || ctx(g));
        return;
    }
    if d.forced(g).is_some() {
        flags.bug(format!("forced-decision-surfaced:{source}"), || ctx(g));
    }
    let bad = |flags: &mut Flags, what: &str| {
        flags.bug(format!("malformed-decision:{what}:{source}"), || ctx(g));
    };
    match &d.kind {
        DecisionKind::Turn { phase, moves } => {
            if !unique(moves) || !moves.contains(&Move::EndPhase) || moves.len() < 2 {
                bad(flags, "turn-moves");
            }
            if *phase != t.phase || d.player != t.player {
                bad(flags, "turn-phase-or-player");
            }
            let top_level = g.stack.len() == 2
                && matches!(g.stack[0], Frame::Game { .. })
                && matches!(g.stack[1], Frame::Turn { .. });
            let looking = g.players.iter().any(|pl| !pl.looking.is_empty());
            if !top_level
                || !g.gains.is_empty()
                || !g.plays.is_empty()
                || !g.trashing.is_empty()
                || !g.holds.is_empty()
                || looking
            {
                flags.bug("leftover-effect-state-at-turn-decision", || {
                    format!(
                        "stack {}, gains {:?}, plays {:?}, trashing {:?}, holds {:?}; {}",
                        g.stack.len(),
                        g.gains,
                        g.plays,
                        g.trashing,
                        g.holds,
                        ctx(g)
                    )
                });
            }
            let pl = g.player(t.player);
            for &m in moves {
                let ok = match m {
                    Move::Play(c) => {
                        let types = match phase {
                            Phase::Action => Types::ACTION,
                            Phase::Buy => Types::TREASURE,
                            _ => Types::NIGHT,
                        };
                        c.index() < g.card_count()
                            && g.is(c, types)
                            && matches!(g.zone(c), Zone::Hand(p) | Zone::Deck(p) if p == t.player)
                            && (*phase != Phase::Action || t.actions > 0)
                    }
                    Move::Buy(pile) => {
                        pile.index() < g.piles().len()
                            && g.pile(pile).in_supply
                            && !g.pile(pile).cards.is_empty()
                            && *phase == Phase::Buy
                            && pl.debt == 0
                    }
                    Move::BuyLandscape(l) => {
                        g.has_landscape(l) && *phase == Phase::Buy && pl.debt == 0
                    }
                    Move::Coffers => pl.coffers > 0,
                    Move::Villager => pl.villagers > 0 && *phase == Phase::Action,
                    Move::PayDebt => pl.debt > 0 && t.coins > 0,
                    Move::EndPhase => true,
                };
                if !ok {
                    flags.bug(
                        format!("illegal-move-offered:{:?}", std::mem::discriminant(&m)),
                        || format!("{m:?}; {}", ctx(g)),
                    );
                }
            }
        }
        DecisionKind::Cards {
            candidates,
            min,
            max,
            ..
        } => {
            if !unique(candidates)
                || min > max
                || *max > candidates.len()
                || *max == 0
                || candidates.iter().any(|c| c.index() >= g.card_count())
            {
                bad(flags, "cards");
            }
        }
        DecisionKind::Pile { piles, optional } => {
            if piles.is_empty() {
                let what = if *optional {
                    "optional-pile-choice-without-piles"
                } else {
                    "unanswerable-pile-choice"
                };
                flags.bug(format!("{what}:{source}"), || ctx(g));
            }
            if !unique(piles) || piles.iter().any(|p| p.index() >= g.piles().len()) {
                bad(flags, "piles");
            }
        }
        DecisionKind::Choose {
            options,
            labels,
            min,
            max,
        } => {
            if !unique(options)
                || labels.len() != options.len()
                || min > max
                || *max > options.len()
                || *max == 0
            {
                bad(flags, "choose");
            }
        }
        DecisionKind::NameCard => {}
        DecisionKind::Number { min, max } => {
            if min >= max {
                bad(flags, "number");
            }
        }
    }
}

/// The numbers after each `phrase` in `text` ("costing up to $" -> 4): digits or a
/// number word.
/// An option said to play an Action (`Decision::plays`) starts its player playing one, or
/// asks them which card to play first.
fn check_plays(g: &Game, d: &Decision, flags: &mut Flags) {
    let DecisionKind::Choose { options, .. } = &d.kind else {
        if !d.plays.is_empty() {
            flags.bug("plays-on-no-choice", || ctx(g));
        }
        return;
    };
    for &o in &d.plays {
        let key = || format!("plays-but-does-not:{:?}:{o:?}", d.source);
        if !options.contains(&o) {
            flags.bug(key(), || ctx(g));
            continue;
        }
        let mut after = g.clone();
        let since = after.log().len();
        if after.respond(Response::Options(vec![o])).is_err() {
            continue;
        }
        let played = after.log()[since..]
            .iter()
            .any(|e| matches!(e, Event::Played { player, .. } if *player == d.player));
        let asked = after
            .pending()
            .is_some_and(|n| n.player == d.player && n.purpose == Purpose::Play);
        if !played && !asked {
            flags.bug(key(), || ctx(g));
        }
    }
}

fn numbers_after(text: &str, phrase: &str) -> Vec<usize> {
    let words = ["zero", "a", "two", "three", "four", "five", "six", "seven"];
    text.match_indices(phrase)
        .filter_map(|(i, _)| {
            let rest = &text[i + phrase.len()..];
            let word: String = rest.chars().take_while(|c| c.is_alphanumeric()).collect();
            word.parse().ok().or_else(|| {
                words
                    .iter()
                    .position(|w| *w == word)
                    .or((word == "an" || word == "one").then_some(1))
            })
        })
        .collect()
}

/// What a decision asks, against the numbers in its card's text. Only lists to review:
/// texts with several clauses are beyond this.
fn check_against_text(g: &Game, d: &Decision, flags: &mut Flags) {
    let Some(source) = d.source else { return };
    let text = source.text().to_lowercase();
    match &d.kind {
        DecisionKind::Pile { piles, .. } if d.purpose == Purpose::Gain => {
            let up_to = numbers_after(&text, "costing up to $");
            let exactly = numbers_after(&text, "costing exactly $");
            let relative = ["more than", "less than", " more", " less", " per "];
            if text.matches("costing").count() != 1
                || up_to.len() + exactly.len() != 1
                || relative.iter().any(|r| text.contains(r))
            {
                return;
            }
            let plain = |c: Cost| c.potions == 0 && c.debt == 0;
            for &pile in piles {
                let Some(top) = g.pile_top(pile) else {
                    continue;
                };
                let cost = g.cost_of(top);
                let ok = match (up_to.first(), exactly.first()) {
                    (Some(&n), _) => plain(cost) && cost.coins as usize <= n,
                    (_, Some(&n)) => plain(cost) && cost.coins as usize == n,
                    _ => true,
                };
                if !ok {
                    flags.audit(
                        format!("text-cost:{source} offers {} at {cost}", g.card(top)),
                        || ctx(g),
                    );
                }
            }
            if let Some(&n) = up_to.first()
                && text.contains(&format!("gain a card costing up to ${n}."))
            {
                for pile in g.supply_piles_where(|_, c| plain(c) && c.coins as usize <= n) {
                    if !piles.contains(&pile) {
                        flags.audit(
                            format!("text-cost:{source} does not offer {}", g.pile(pile).card),
                            || ctx(g),
                        );
                    }
                }
            }
        }
        DecisionKind::Cards {
            candidates,
            min,
            max,
            ..
        } if matches!(d.purpose, Purpose::Trash | Purpose::Discard) => {
            let verb = if d.purpose == Purpose::Trash {
                "trash"
            } else {
                "discard"
            };
            if text.contains("any number") || text.matches(verb).count() != 1 {
                return;
            }
            let hand = candidates
                .iter()
                .all(|&c| matches!(g.zone(c), Zone::Hand(_)));
            if !hand {
                return;
            }
            let n = candidates.len();
            let mut allowed: Vec<(usize, usize)> = vec![];
            for k in numbers_after(&text, &format!("{verb} up to ")) {
                allowed.push((0, k.min(n)));
            }
            for form in [format!("{verb} "), format!("{verb}s ")] {
                for k in numbers_after(&text, &form) {
                    allowed.push((k.min(n), k.min(n)));
                }
            }
            for k in numbers_after(&text, "down to ") {
                allowed.push((n.saturating_sub(k), n.saturating_sub(k)));
            }
            if text.contains(" may ") {
                allowed.extend(allowed.clone().iter().map(|&(_, hi)| (0, hi)));
            }
            if !allowed.is_empty() && !allowed.contains(&(*min, *max)) {
                flags.audit(
                    format!("text-count:{source} asks to {verb} {min}..={max} of {n}"),
                    || format!("text allows {allowed:?}; {}", ctx(g)),
                );
            }
        }
        _ => {}
    }
}

/// Responses of every kind that `d` must reject.
fn invalid_responses(g: &Game, d: &Decision) -> Vec<Response> {
    let mut out = vec![];
    let no_card = CardId(g.card_count() as u32 + 7);
    let no_pile = PileId(250);
    match &d.kind {
        DecisionKind::Turn { moves, .. } => {
            out.extend([Response::Cards(vec![]), Response::Options(vec![])]);
            for m in [
                Move::Play(no_card),
                Move::Buy(no_pile),
                Move::BuyLandscape(Card::Copper),
                Move::Coffers,
                Move::Villager,
                Move::PayDebt,
            ] {
                if !moves.contains(&m) {
                    out.push(Response::Move(m));
                }
            }
            // A card that exists but isn't offered.
            if let Some(c) = (0..g.card_count() as u32)
                .map(CardId)
                .find(|c| !moves.contains(&Move::Play(*c)))
            {
                out.push(Response::Move(Move::Play(c)));
            }
        }
        DecisionKind::Cards {
            candidates,
            min,
            max,
            ..
        } => {
            out.extend([Response::Move(Move::EndPhase), Response::Pile(None)]);
            out.push(Response::Cards(vec![no_card]));
            if let Some(&c) = candidates.first() {
                out.push(Response::Cards(vec![c, c]));
            }
            if *max < candidates.len() {
                out.push(Response::Cards(candidates.clone()));
            }
            if *min > 0 {
                out.push(Response::Cards(candidates[..*min - 1].to_vec()));
            }
            if let Some(c) = (0..g.card_count() as u32)
                .map(CardId)
                .find(|c| !candidates.contains(c))
            {
                out.push(Response::Cards(vec![c]));
            }
        }
        DecisionKind::Pile { piles, optional } => {
            out.extend([Response::Number(0), Response::Options(vec![Opt::Yes])]);
            out.push(Response::Pile(Some(no_pile)));
            if let Some(p) = (0..g.piles().len() as u8)
                .map(PileId)
                .find(|p| !piles.contains(p))
            {
                out.push(Response::Pile(Some(p)));
            }
            if !optional {
                out.push(Response::Pile(None));
            }
        }
        DecisionKind::Choose {
            options, min, max, ..
        } => {
            out.extend([Response::Move(Move::EndPhase), Response::Name(Card::Copper)]);
            out.push(Response::Options(vec![Opt::Player(PlayerId(77))]));
            out.push(Response::Options(vec![options[0], options[0]]));
            if *max < options.len() {
                out.push(Response::Options(options.clone()));
            }
            if *min > 0 {
                out.push(Response::Options(options[..*min - 1].to_vec()));
            }
        }
        DecisionKind::NameCard => {
            out.extend([Response::Number(1), Response::Cards(vec![])]);
            out.extend([Card::Knights, Card::WayOfTheMouse, Card::TheSeasGift].map(Response::Name));
        }
        DecisionKind::Number { min, max } => {
            out.extend([Response::Pile(None), Response::Options(vec![])]);
            out.extend([Response::Number(min - 1), Response::Number(max + 1)]);
        }
    }
    out
}

/// Arrays in the sets' state that only hold something while an effect resolves.
const TRANSIENT_EXT: &[(&str, &str)] = &[
    ("adventures", "extra_ports"),
    ("renaissance", "sewers"),
    ("renaissance", "experiments"),
];

fn numbers(v: &Json, out: &mut HashSet<u64>) {
    match v {
        Json::Number(n) => {
            out.extend(n.as_u64());
        }
        Json::Array(a) => a.iter().for_each(|x| numbers(x, out)),
        Json::Object(o) => o.values().for_each(|x| numbers(x, out)),
        _ => {}
    }
}

const BIG: u32 = 1_000_000;

/// Structural invariants that hold at every decision.
fn check_invariants(g: &Game, flags: &mut Flags) {
    let t = g.turn();
    for l in g.lasting() {
        if l.owner.index() >= g.num_players() {
            flags.bug("lasting-owner", || ctx(g));
        }
    }
    for &c in g.masks.keys() {
        if !matches!(g.zone(c), Zone::InPlay(_)) {
            flags.bug("mask-on-a-card-not-in-play", || ctx(g));
        }
    }
    if g.coins_as_cards.is_some() {
        flags.bug("coins-as-cards-outside-a-step", || ctx(g));
    }
    let counters = [t.actions, t.buys, t.coins, t.potions, g.trade_route_mat];
    if counters.iter().any(|&x| x > BIG) {
        flags.bug("turn-counter-wrapped", || ctx(g));
    }
    if t.played.len() != t.played_ids.len() || t.played.len() != t.played_at.len() {
        flags.bug("turn-played-vectors-disagree", || ctx(g));
    }
    for (p, pl) in g.players.iter().enumerate() {
        let pid = PlayerId(p as u8);
        let counters = [pl.coffers, pl.villagers, pl.favors, pl.debt, pl.pirate_ship];
        if counters.iter().any(|&x| x > BIG) || pl.skip_turns > BIG {
            flags.bug("player-counter-wrapped", || ctx(g));
        }
        if pl.vp_tokens < 0 {
            flags.bug("negative-vp-tokens", || {
                format!("{pid}: {}; {}", pl.vp_tokens, ctx(g))
            });
        }
        if !unique(&pl.projects) {
            flags.bug("project-bought-twice", || {
                format!("{pid}: {:?}; {}", pl.projects, ctx(g))
            });
        }
        for &c in pl.mat(Mat::Tavern) {
            let card = g.printed(c);
            if !card.is(ty("Reserve")) && card != Card::Copper {
                flags.audit(format!("on-the-tavern-mat:{card}"), || ctx(g));
            }
        }
        for &c in &pl.in_play {
            if !g.is(c, Types::ACTION | Types::TREASURE | Types::NIGHT) {
                flags.audit(
                    format!("in-play-without-a-playable-type:{}", g.printed(c)),
                    || ctx(g),
                );
            }
        }
        if !unique(&pl.states) || !unique(&pl.boons_kept) {
            flags.bug("duplicate-state-or-kept-boon", || {
                format!("{pid}: {:?} {:?}; {}", pl.states, pl.boons_kept, ctx(g))
            });
        }
        let both = |a: Card, b: Card| pl.states.contains(&a) && pl.states.contains(&b);
        if both(Card::Deluded, Card::Envious) || both(Card::Miserable, Card::TwiceMiserable) {
            flags.bug("both-sides-of-a-state", || {
                format!("{pid}: {:?}; {}", pl.states, ctx(g))
            });
        }
        for (&token, &pile) in &pl.tokens {
            if let Some(pile) = pile
                && !g.pile(pile).tokens.contains(&(pid, token))
            {
                flags.bug("player-token-not-on-its-pile", || {
                    format!("{pid} {token:?} {pile:?}; {}", ctx(g))
                });
            }
        }
        if pl.extra_turns_taken > pl.turns_taken {
            flags.bug("more-extra-turns-than-turns", || ctx(g));
        }
    }
    let lost = |pl: &&Player| pl.states.contains(&Card::LostInTheWoods);
    if g.players.iter().filter(lost).count() > 1 {
        flags.bug("unique-state-held-twice", || ctx(g));
    }
    for (i, pile) in g.piles().iter().enumerate() {
        if pile.vp_tokens < 0 || pile.debt_tokens < 0 || pile.embargo > BIG {
            flags.bug(format!("negative-pile-tokens:{}", pile.card), || ctx(g));
        }
        for &(p, token) in &pile.tokens {
            if g.player(p).tokens.get(&token) != Some(&Some(PileId(i as u8))) {
                flags.bug("pile-token-not-its-player's", || {
                    format!("{p} {token:?} on {}; {}", pile.card, ctx(g))
                });
            }
        }
        // Wiki, Ferryman: Temple and Farmers' Market add and take VP on their own pile
        // even when it is not in the Supply.
        let own_vp = matches!(pile.card, Card::Temple | Card::FarmersMarket)
            && pile.embargo == 0
            && !pile.trade_route
            && pile.tokens.is_empty()
            && pile.traits.is_empty();
        // Harsh Winter puts Debt on the pile of any card gained.
        let harsh_winter = g.prophecy().is_some_and(|(p, _)| p == Card::HarshWinter);
        if !pile.in_supply
            && !pile.removed
            && !own_vp
            && (pile.vp_tokens != 0
                || (pile.debt_tokens != 0 && !harsh_winter)
                || pile.embargo != 0
                || pile.trade_route
                || !pile.tokens.is_empty()
                || !pile.traits.is_empty())
        {
            flags.bug(
                format!("tokens-on-a-pile-outside-the-supply:{}", pile.card),
                || format!("{pile:?}; {}", ctx(g)),
            );
        }
        let holds_any = pile.removed || pile.holder;
        if let Some(&c) = pile
            .cards
            .iter()
            .find(|&&c| g.home_pile(g.printed(c)) != Some(PileId(i as u8)))
            && !holds_any
        {
            flags.bug(format!("card-in-the-wrong-pile:{}", pile.card), || {
                format!("{} in the {} pile; {}", g.printed(c), pile.card, ctx(g))
            });
        }
        if !unique(&pile.tokens) || !unique(&pile.traits) {
            flags.bug("duplicate-pile-tokens-or-traits", || ctx(g));
        }
    }
    let artifacts: Vec<Card> = g.artifacts().iter().map(|a| a.0).collect();
    if !unique(&artifacts) {
        flags.bug("duplicate-artifact", || ctx(g));
    }
    // Boons and Hexes: never two of one.
    let mut boons: Vec<Card> = g.boons().draw.clone();
    boons.extend(&g.boons().discard);
    boons.extend(&g.ext.nocturne.druid);
    for pl in &g.players {
        boons.extend(&pl.boons_kept);
    }
    let mut hexes = g.hexes().draw.clone();
    hexes.extend(&g.hexes().discard);
    if !unique(&boons) || !unique(&hexes) {
        flags.bug("duplicate-boon-or-hex", || {
            format!("boons {boons:?}, hexes {hexes:?}; {}", ctx(g))
        });
    }
    if g.lasting().len() > 400 {
        flags.bug("lasting-effects-accumulate", || {
            let mut by_card: BTreeMap<Card, usize> = BTreeMap::new();
            for l in g.lasting() {
                *by_card.entry(l.card).or_default() += 1;
            }
            format!("{by_card:?}; {}", ctx(g))
        });
    }
    if g.extra_turns().len() > 40 {
        flags.bug("extra-turn-queue-grows", || ctx(g));
    }
    let top_level = matches!(
        g.pending().map(|d| &d.kind),
        Some(DecisionKind::Turn { .. })
    );
    if !top_level {
        return;
    }
    // Only at a turn's own decisions: nothing is mid-flight.
    let ext = serde_json::to_value(&g.ext).expect("set state serializes");
    for (set, field) in TRANSIENT_EXT {
        match &ext[set][field] {
            Json::Array(a) if a.is_empty() => {}
            other => flags.bug(format!("transient-state-left-over:{set}.{field}"), || {
                format!("{other}; {}", ctx(g))
            }),
        }
    }
    let delayed_boons = g
        .lasting()
        .iter()
        .filter(|l| l.data.card.is_some_and(|c| c.kind() == Kind::Boon))
        .count();
    if !boons.is_empty() && boons.len() + delayed_boons != 12 {
        flags.bug("boons-not-conserved", || {
            format!(
                "{} boons + {delayed_boons} delayed: {boons:?}; {}",
                boons.len(),
                ctx(g)
            )
        });
    }
    if !hexes.is_empty() && hexes.len() != 12 {
        flags.bug("hexes-not-conserved", || format!("{hexes:?}; {}", ctx(g)));
    }
}

/// A card that has stayed where an effect put it is there: nothing moves a card without
/// the game noticing (the stop-moving rule's stamps), and on a deck nothing covers it.
fn check_stayed(g: &Game, flags: &mut Flags) {
    let on_top = |c: CardId| match g.zone(c) {
        Zone::Deck(p) => g.player(p).deck.last() == Some(&c),
        _ => true,
    };
    let mut wrong: Vec<CardId> = vec![];
    for st in g.gains.iter().chain(&g.last_gain) {
        if g.stayed(st.card, st.at) && !(g.zone(st.card) == st.zone() && on_top(st.card)) {
            wrong.push(st.card);
        }
    }
    for st in &g.trashing {
        if g.stayed(st.card, st.at) && g.zone(st.card) != Zone::Trash {
            wrong.push(st.card);
        }
    }
    let t = &g.turn;
    for (&c, &at) in t.played_ids.iter().zip(&t.played_at) {
        if g.stayed(c, at) && !matches!(g.zone(c), Zone::InPlay(_)) {
            wrong.push(c);
        }
    }
    for c in wrong {
        flags.bug(format!("stayed-card-elsewhere:{}", g.printed(c)), || {
            format!("{c:?} is in {:?}; {}", g.zone(c), ctx(g))
        });
    }
}

/// Cards taken off a deck to be looked at are held by a frame still on the stack, which
/// puts them back when it finishes.
fn check_looking(g: &Game, flags: &mut Flags) {
    for (p, pl) in g.players.iter().enumerate() {
        for &c in &pl.looking {
            let held = g.held(PlayerId(p as u8), c);
            if !held || g.holds.iter().any(|h| h.depth > g.stack.len()) {
                flags.bug(format!("looking-card-not-held:{}", g.printed(c)), || {
                    format!("P{}'s {c:?}; holds {:?}; {}", p + 1, g.holds, ctx(g))
                });
            }
        }
    }
}

/// Set-aside cards are held by something that will release them: a lasting effect or a
/// set's state. One whose effect has ended (a Duration that left play stops, and what it
/// set aside stays there) is only listed.
fn check_set_aside(g: &Game, m: &Memory, flags: &mut Flags) {
    let ext = serde_json::to_value(&g.ext).expect("set state serializes");
    let mut tracked = HashSet::new();
    numbers(&ext, &mut tracked);
    for l in g.lasting() {
        tracked.extend(l.source.map(|c| c.0 as u64));
        tracked.extend(l.data.pick.map(|c| c.0 as u64));
        tracked.extend(l.keeps.iter().chain(&l.data.cards).map(|c| c.0 as u64));
    }
    for (p, pl) in g.players.iter().enumerate() {
        for &c in &pl.set_aside {
            // A card from the Black Market deck that can't return to its pile (Encampment)
            // stays set aside.
            let homeless = g.home_pile(g.printed(c)).is_none();
            if tracked.contains(&(c.0 as u64)) || homeless {
                continue;
            }
            let detail = || format!("P{}'s {c:?}; lasting {:?}; {}", p + 1, g.lasting(), ctx(g));
            if m.held.contains(&c) {
                flags.audit(
                    format!("set-aside-card-whose-effect-ended:{}", g.printed(c)),
                    detail,
                );
            } else {
                flags.bug(
                    format!("set-aside-card-nothing-tracks:{}", g.printed(c)),
                    detail,
                );
            }
        }
    }
}

/// The `View` shows a card exactly when its zone or a pending decision lets it.
fn check_views(g: &Game, flags: &mut Flags) {
    for v in 0..g.num_players() {
        let v = PlayerId(v as u8);
        let view = g.view(v);
        let offered: Vec<CardId> = g
            .pending()
            .filter(|d| d.player == v)
            .map(offered_cards)
            .unwrap_or_default();
        if view.pending().is_some() != g.pending().is_some_and(|d| d.player == v) {
            flags.bug("view-shows-another-player's-decision", || ctx(g));
        }
        for i in 0..g.card_count() as u32 {
            let id = CardId(i);
            let expected = base_visible(g, v, id) || offered.contains(&id);
            let shown = view.card(id).is_some();
            let same = view.zone(id).is_some() == shown
                && view.cost_of(id).is_some() == shown
                && view.types(id).is_some() == shown;
            // What the view says of a card it shows is what the game says.
            if shown
                && (view.card(id) != Some(g.card(id))
                    || view.zone(id) != Some(g.zone(id))
                    || view.cost_of(id) != Some(g.cost_of(id))
                    || view.types(id) != Some(g.types(id)))
            {
                flags.bug("view-misstates-a-card", || {
                    format!("{v} {id:?} ({}); {}", g.printed(id), ctx(g))
                });
            }
            if shown != expected || !same {
                flags.bug(format!("view-visibility:{}", zone_kind(g.zone(id))), || {
                    format!(
                        "{v} sees={shown} expected={expected} {id:?} ({}) in {:?}; {}",
                        g.printed(id),
                        g.zone(id),
                        ctx(g)
                    )
                });
            }
        }
        let visible = |c: &CardId| view.card(*c).is_some();
        for pv in view.players() {
            let p = pv.id();
            let mine = v == p;
            if pv.hand().is_some() != mine || pv.mat(Mat::NativeVillage).is_some() != mine {
                flags.bug("view-hand-or-native-village", || ctx(g));
            }
            if !pv.set_aside().iter().all(visible) {
                flags.bug("view-set-aside", || ctx(g));
            }
            if !pv.looking().iter().all(visible) {
                flags.bug("view-looking", || ctx(g));
            }
        }
        for (shown, real) in view.lasting().iter().zip(g.lasting()) {
            let ids = shown
                .source
                .iter()
                .chain(&shown.keeps)
                .chain(&shown.data.cards)
                .chain(&shown.data.pick);
            if !ids.clone().all(visible) {
                flags.bug(
                    format!("view-lasting-shows-hidden-card:{}", real.card),
                    || ctx(g),
                );
            }
            let hidden = real
                .source
                .iter()
                .chain(&real.keeps)
                .chain(&real.data.cards)
                .chain(&real.data.pick)
                .any(|c| !visible(c));
            if hidden && real.data.card.is_some() {
                flags.audit(
                    format!("lasting-names-a-card-with-hidden-cards:{}", real.card),
                    || format!("{real:?} for {v}; {}", ctx(g)),
                );
            }
        }
        for pile in view.piles() {
            if pile.top().is_some_and(|c| !visible(&c)) {
                flags.bug("view-pile-top", || ctx(g));
            }
        }
    }
}

/// Everything `v`'s `View` shows, in named parts.
fn view_parts(g: &Game, v: PlayerId) -> Vec<(&'static str, String)> {
    let view = g.view(v);
    let cards: Vec<_> = (0..g.card_count() as u32)
        .map(CardId)
        .map(|id| {
            (
                view.card(id),
                view.zone(id),
                view.cost_of(id),
                view.types(id),
            )
        })
        .collect();
    let scores: Vec<i32> = view.players().map(|p| p.score()).collect();
    let players: Vec<String> = view
        .players()
        .map(|p| {
            let mats = [Mat::Tavern, Mat::Island, Mat::NativeVillage, Mat::Exile]
                .map(|m| (p.mat(m).map(<[CardId]>::to_vec), p.mat_size(m)));
            format!(
                "{:?} {} {} {} {:?} {:?} {:?} {} {:?} {} {mats:?} {} {} {} {} {} {:?} {:?} {:?}",
                p.hand(),
                p.hand_size(),
                p.deck_size(),
                p.discard_size(),
                p.discard_top(),
                p.in_play(),
                p.set_aside(),
                p.set_aside_size(),
                p.looking(),
                p.looking_size(),
                p.vp_tokens(),
                p.coffers(),
                p.villagers(),
                p.favors(),
                p.debt(),
                p.projects(),
                p.states(),
                p.tokens(),
            )
        })
        .collect();
    let piles: Vec<_> = view
        .piles()
        .map(|p| (p.card(), p.size(), p.top(), p.vp_tokens(), p.debt_tokens()))
        .collect();
    let mut names: Vec<Card> = g.cards.clone();
    names.extend(g.landscapes());
    names.sort();
    names.dedup();
    let costs: Vec<Cost> = names.iter().map(|&c| view.cost(c)).collect();
    vec![
        ("cards", format!("{cards:?}")),
        ("score", format!("{scores:?}")),
        ("players", format!("{players:?}")),
        ("piles", format!("{piles:?}")),
        ("costs", format!("{costs:?}")),
        ("lasting", format!("{:?}", view.lasting())),
        ("notes", format!("{:?}", view.notes())),
        ("trash", format!("{:?}", view.trash())),
    ]
}

/// Nothing a `View` shows depends on which is which of two cards its player can't tell
/// apart: swaps two such cards in a copy of the game and compares.
fn check_view_noninterference(g: &Game, k: &Knowledge, rng: &mut ChaCha8Rng, flags: &mut Flags) {
    let v = PlayerId(rng.random_range(0..g.num_players()) as u8);
    let view = g.view(v);
    let mut pairs = vec![];
    for group in k.groups(v) {
        let hidden: Vec<CardId> = group
            .cards
            .iter()
            .copied()
            .filter(|&c| view.card(c).is_none())
            .collect();
        if hidden.len() != group.cards.len() {
            continue;
        }
        // Prefer two cards of different players, where a leak of ownership would show.
        let other = |a: CardId| {
            hidden
                .iter()
                .copied()
                .filter(|&b| g.printed(b) != g.printed(a))
                .max_by_key(|&b| g.zone(b).owner() != g.zone(a).owner())
        };
        if let Some((a, b)) = hidden.iter().find_map(|&a| other(a).map(|b| (a, b))) {
            pairs.push((a, b));
        }
    }
    let before = view_parts(g, v);
    for (a, b) in pairs {
        let mut altered = g.clone();
        altered.cards.swap(a.index(), b.index());
        for ((part, x), (_, y)) in before.iter().zip(view_parts(&altered, v)) {
            if *x != y {
                flags.bug(format!("view-depends-on-hidden-cards:{part}"), || {
                    format!(
                        "{v} can't tell {a:?} ({} in {:?}) from {b:?} ({} in {:?}); {}",
                        g.printed(a),
                        g.zone(a),
                        g.printed(b),
                        g.zone(b),
                        ctx(g)
                    )
                });
            }
        }
    }
}

/// `Knowledge` agrees with the true game state.
fn check_knowledge(g: &Game, k: &Knowledge, flags: &mut Flags) {
    for v in 0..g.num_players() {
        let v = PlayerId(v as u8);
        let view = g.view(v);
        let mut grouped = 0;
        for group in k.groups(v) {
            grouped += group.cards.len();
            if let Some(ids) = group.ids {
                let mut actual = CardCount::new();
                for &c in group.cards {
                    *actual.entry(g.printed(c)).or_default() += 1;
                }
                if !super::knowledge::some_of(ids, group.extra, &actual) {
                    flags.bug("knowledge-group-identities-wrong", || {
                        format!(
                            "{v}'s group {:?} is {actual:?}, known as {ids:?} less {}; {}",
                            group.cards,
                            group.extra,
                            ctx(g)
                        )
                    });
                }
            }
            if group.cards.len() > 1 && group.cards.iter().any(|&c| view.card(c).is_some()) {
                flags.bug("knowledge-visible-card-in-a-group", || ctx(g));
            }
        }
        if grouped != g.card_count() {
            flags.bug("knowledge-does-not-cover-every-card", || ctx(g));
        }
        let known = k.known(g, v);
        let hand = known.zone(Zone::Hand(v));
        if hand.unknown != 0 || hand.count != g.hand(v).len() {
            flags.bug("knowledge-own-hand-unknown", || {
                format!("{v}: {hand:?}; {}", ctx(g))
            });
        }
        let mut total = 0;
        for (&zone, z) in &known.zones {
            total += z.count;
            let mut actual = CardCount::new();
            let mut count = 0;
            for i in 0..g.card_count() as u32 {
                let c = CardId(i);
                if g.zone(c) == zone {
                    count += 1;
                    *actual
                        .entry(view.card(c).unwrap_or(g.printed(c)))
                        .or_default() += 1;
                }
            }
            let known_n: u32 = z.known.values().sum();
            if z.count != count || known_n as usize + z.unknown != count {
                flags.bug("knowledge-zone-count-wrong", || {
                    format!("{v} {zone:?}: {z:?}, really {count}; {}", ctx(g))
                });
            }
            for (card, n) in &z.known {
                if actual.get(card).is_none_or(|m| m < n) {
                    flags.bug("knowledge-zone-identities-wrong", || {
                        format!(
                            "{v} knows {n} {card} in {zone:?}, which has {actual:?}; {}",
                            ctx(g)
                        )
                    });
                }
            }
        }
        if total != g.card_count() {
            flags.bug("knowledge-zones-do-not-cover-every-card", || ctx(g));
        }
    }
}

/// Cards a card's printed name stands for in this step: printed, and what it is now.
fn names(g: &Game, id: CardId) -> [Card; 2] {
    [g.printed(id), g.card(id)]
}

/// The log and the trace of one step agree about what was shown to everyone.
fn check_step(g: &Game, tracker: &Tracker, events: &[Event], trace: &[Trace], flags: &mut Flags) {
    let last_played = g
        .log()
        .iter()
        .rev()
        .find_map(|e| match e {
            Event::Played { card, .. } => Some(card.to_string()),
            _ => None,
        })
        .unwrap_or_default();
    let mut shown: Vec<CardId> = vec![];
    let mut moved_public: Vec<CardId> = vec![];
    let mut discarded: Vec<CardId> = vec![];
    for entry in trace {
        match entry {
            Trace::Shown { card, to: None } => shown.push(*card),
            Trace::Moved { card, from, to, .. } => {
                let followed = tracker.seen.iter().all(|s| s[card.index()]);
                if public_zone(g, *from) || public_zone(g, *to) || followed {
                    moved_public.push(*card);
                }
                if matches!(to, Zone::Discard(_)) {
                    discarded.push(*card);
                }
            }
            _ => {}
        }
    }
    let mut named: Vec<Card> = vec![];
    for e in events {
        let (card, kind) = match e {
            Event::Revealed { card, .. } => (*card, "Revealed"),
            Event::SetAside { card, .. } => (*card, "SetAside"),
            Event::Discarded { card, .. } => (*card, "Discarded"),
            Event::Moved { cards, .. } => {
                for &card in cards {
                    named.push(card);
                    let public = shown
                        .iter()
                        .chain(&moved_public)
                        .any(|&c| names(g, c).contains(&card));
                    if card.kind() == Kind::Card && !public {
                        flags.audit(
                            format!("event-names-a-card-the-trace-never-shows:Moved:{last_played}"),
                            || format!("{e:?}; {}", ctx(g)),
                        );
                    }
                }
                continue;
            }
            Event::Played { card, .. }
            | Event::Gained { card, .. }
            | Event::Trashed { card, .. }
            | Event::Bought { card, .. } => {
                named.push(*card);
                continue;
            }
            Event::RevealedHand { cards, .. } => {
                named.extend(cards);
                continue;
            }
            _ => continue,
        };
        named.push(card);
        if card.kind() != Kind::Card {
            continue;
        }
        let public = shown
            .iter()
            .chain(&moved_public)
            .any(|&c| names(g, c).contains(&card));
        if !public {
            flags.audit(
                format!("event-names-a-card-the-trace-never-shows:{kind}:{last_played}"),
                || format!("{e:?}; {}", ctx(g)),
            );
        }
    }
    for &c in &shown {
        let everyone = (0..g.num_players()).all(|v| base_visible(g, PlayerId(v as u8), c));
        let logged = names(g, c).iter().any(|n| named.contains(n));
        if !everyone && !logged && !moved_public.contains(&c) && !discarded.contains(&c) {
            flags.audit(
                format!("trace-shows-a-card-no-event-names:{last_played}"),
                || format!("{c:?} ({}) in {:?}; {}", g.printed(c), g.zone(c), ctx(g)),
            );
        }
    }
}

/// The log since the previous turn began: a step can run from one turn into the next.
fn this_turn(g: &Game) -> &[Event] {
    let log = g.log();
    let mut starts = log
        .iter()
        .enumerate()
        .rev()
        .filter(|(_, e)| matches!(e, Event::TurnStart { .. }))
        .map(|(i, _)| i);
    starts.next();
    &log[starts.next().unwrap_or(0)..]
}

/// Moves that should come with an event (a gain, a trashing, a play), since effects react
/// to those: lists the ones that came without, to review.
fn check_move_discipline(g: &Game, events: &[Event], trace: &[Trace], flags: &mut Flags) {
    let named = |pred: &dyn Fn(&Event) -> Option<Card>, id: CardId| {
        events
            .iter()
            .filter_map(pred)
            .any(|c| names(g, id).contains(&c))
    };
    let gained = |e: &Event| match e {
        Event::Gained { card, .. } => Some(*card),
        _ => None,
    };
    let trashed = |e: &Event| match e {
        Event::Trashed { card, .. } => Some(*card),
        _ => None,
    };
    let played = |e: &Event| match e {
        Event::Played { card, .. } => Some(*card),
        _ => None,
    };
    // Every gain brings its player a card from somewhere else.
    for p in 0..g.num_players() {
        let p = PlayerId(p as u8);
        let gains = events
            .iter()
            .filter(|e| matches!(e, Event::Gained { player, .. } if *player == p))
            .count();
        let arrivals = trace
            .iter()
            .filter(|t| {
                matches!(t, Trace::Moved { from, to, .. }
                    if to.owner() == Some(p) && from.owner() != Some(p))
            })
            .count();
        if gains > arrivals {
            flags.bug("gain-without-a-card-arriving", || {
                format!("{p}: {gains} gains, {arrivals} cards; {}", ctx(g))
            });
        }
    }
    for entry in trace {
        let Trace::Moved { card, from, to, .. } = *entry else {
            continue;
        };
        let name = g.printed(card);
        // Wiki, Ferryman: besides gaining a Ferryman, Rats gains from its own pile there
        // and Changeling can be exchanged for; Gladiator trashes "from its pile".
        if let Zone::Pile(pile) = from
            && Some(pile) == g.ext.cornucopia_guilds.ferryman
            && !(name == Card::Rats && named(&gained, card))
            && name != Card::Changeling
            && !(name == Card::Gladiator && to == Zone::Trash)
            && !g.log().iter().rev().take(80).any(|e| {
                matches!(
                    e,
                    Event::Gained {
                        card: Card::Ferryman,
                        ..
                    }
                )
            })
        {
            flags.bug(
                format!("card-left-the-ferryman-pile:{name} to {}", zone_kind(to)),
                || ctx(g),
            );
        }
        if let Zone::Pile(pile) = from
            && to.owner().is_some()
            && !named(&gained, card)
        {
            flags.audit(
                format!("left-a-pile-without-a-gain:{name} to {}", zone_kind(to)),
                || format!("from the {} pile; {}", g.pile(pile).card, ctx(g)),
            );
        }
        // A card changes hands only by being passed (Masquerade); anything else goes
        // through the trash or a pile.
        if let (Some(a), Some(b)) = (from.owner(), to.owner())
            && a != b
            && !this_turn(g).iter().any(|e| {
                matches!(
                    e,
                    Event::Played {
                        card: Card::Masquerade,
                        ..
                    }
                )
            })
        {
            flags.bug(
                format!(
                    "card-changed-hands:{name} from {} to {}",
                    zone_kind(from),
                    zone_kind(to)
                ),
                || format!("{a} to {b}; {}", ctx(g)),
            );
        }
        // A card put where it already is: harmless when an effect puts a gained card
        // into the hand it was gained to (Sleigh, Villa) or sets it aside again
        // (Blockade), but in a discard pile it is a card gained twice.
        if from == to && matches!(from, Zone::Discard(_)) {
            flags.bug(
                format!("card-moved-to-where-it-was:{name} in {}", zone_kind(from)),
                || ctx(g),
            );
        }
        if from == Zone::Trash && to != Zone::Trash && !named(&gained, card) {
            flags.audit(
                format!("left-the-trash-without-a-gain:{name} to {}", zone_kind(to)),
                || ctx(g),
            );
        }
        if to == Zone::Trash && from != Zone::Trash && !named(&trashed, card) {
            flags.audit(
                format!(
                    "entered-the-trash-without-a-trashing:{name} from {}",
                    zone_kind(from)
                ),
                || ctx(g),
            );
        }
        if matches!(to, Zone::InPlay(_))
            && !matches!(from, Zone::InPlay(_))
            && !named(&played, card)
        {
            flags.audit(
                format!(
                    "entered-play-without-a-play:{name} from {}",
                    zone_kind(from)
                ),
                || ctx(g),
            );
        }
    }
}

/// Cards the sets' state and lasting effects refer to are where those expect them.
fn check_references(g: &Game, flags: &mut Flags) {
    let st = &g.ext;
    for &c in &st.menagerie.invested {
        if !matches!(g.zone(c), Zone::Mat(_, Mat::Exile)) {
            flags.bug("invested-card-not-in-exile", || {
                format!("{c:?} ({}) in {:?}; {}", g.printed(c), g.zone(c), ctx(g))
            });
        }
    }
    for &(p, c) in &st.adventures.inherited {
        if let Some(c) = c
            && g.zone(c) != Zone::SetAside(p)
        {
            flags.bug("inherited-card-not-set-aside", || ctx(g));
        }
    }
    for c in [st.menagerie.mouse, st.rising_sun.riverboat]
        .into_iter()
        .flatten()
    {
        if !matches!(g.zone(c), Zone::Pile(_)) {
            flags.bug(
                format!("setup-card-left-its-holder:{}", g.printed(c)),
                || format!("{c:?} ({}) in {:?}; {}", g.printed(c), g.zone(c), ctx(g)),
            );
        }
    }
    // Cards a lasting effect keeps in play that are somewhere else: a list (a Duration
    // that left play is being dealt with).
    for l in g.lasting() {
        for &c in &l.keeps {
            if !matches!(g.zone(c), Zone::InPlay(_)) {
                flags.audit(
                    format!(
                        "lasting-keeps-a-card-in:{}:{} in {}",
                        l.card,
                        g.printed(c),
                        zone_kind(g.zone(c))
                    ),
                    || format!("{l:?}; {}", ctx(g)),
                );
            }
        }
    }
    for l in g.lasting() {
        for &c in l.data.cards.iter().chain(&l.data.pick) {
            if !matches!(g.zone(c), Zone::SetAside(_)) {
                flags.audit(
                    format!(
                        "lasting-refers-to-a-card-in:{}:{}",
                        l.card,
                        zone_kind(g.zone(c))
                    ),
                    || format!("{l:?}; {}", ctx(g)),
                );
            }
        }
    }
}

/// An independent recount of `p`'s score, for games with nothing it doesn't model.
fn recount(g: &Game, p: PlayerId) -> Option<i32> {
    if g.landscapes()
        .iter()
        .any(|l| matches!(l.kind(), Kind::Landmark | Kind::Ally) || *l == Card::Inheritance)
        || g.prophecy().is_some()
    {
        return None;
    }
    let pl = g.player(p);
    let owned: Vec<Card> = pl.owned().map(|c| g.printed(c)).collect();
    let count = |f: &dyn Fn(Card) -> bool| owned.iter().filter(|&&c| f(c)).count() as i32;
    let distinct = |f: &dyn Fn(Card) -> bool| {
        owned
            .iter()
            .filter(|&&c| f(c))
            .collect::<BTreeSet<_>>()
            .len() as i32
    };
    let castle = ty("Castle");
    let mut total = pl.vp_tokens;
    for id in pl.owned() {
        total += match g.printed(id) {
            Card::Gardens => owned.len() as i32 / 10,
            Card::Duke => count(&|c| c == Card::Duchy),
            Card::Vineyard => count(&|c| c.is(Types::ACTION)) / 3,
            Card::Fairgrounds => 2 * (distinct(&|_| true) / 5),
            Card::SilkRoad => count(&|c| c.is(Types::VICTORY)) / 4,
            Card::Feodum => count(&|c| c == Card::Silver) / 3,
            Card::DistantLands => {
                if g.zone(id) == Zone::Mat(p, Mat::Tavern) {
                    4
                } else {
                    0
                }
            }
            Card::HumbleCastle => count(&|c| c.is(castle)),
            Card::KingsCastle => 2 * count(&|c| c.is(castle)),
            Card::Pasture => count(&|c| c == Card::Estate),
            Card::Territory => distinct(&|c| c.is(Types::VICTORY)),
            Card::Marchland => count(&|c| c.is(Types::VICTORY)) / 3,
            Card::Demesne => count(&|c| c == Card::Gold),
            c if c.text().contains("Worth") => return None,
            c => c.def().vp as i32,
        };
    }
    for s in &pl.states {
        total += match s {
            Card::Miserable => -2,
            Card::TwiceMiserable => -4,
            _ => 0,
        };
    }
    Some(total)
}

fn check_outcome(g: &Game, flags: &mut Flags) {
    let Some(o) = g.outcome() else {
        flags.bug("no-outcome-and-no-decision", || ctx(g));
        return;
    };
    let n = g.num_players();
    for p in 0..n {
        let pid = PlayerId(p as u8);
        if o.scores[p] != g.score(pid) {
            flags.bug("outcome-score-differs-from-score", || ctx(g));
        }
        if let Some(mine) = recount(g, pid)
            && mine != o.scores[p]
        {
            flags.bug("score-differs-from-recount", || {
                let owned: Vec<Card> = g.player(pid).owned().map(|c| g.printed(c)).collect();
                format!(
                    "{pid}: engine {}, recount {mine}; owns {owned:?}",
                    o.scores[p]
                )
            });
        }
        let starts = g
            .log()
            .iter()
            .filter(|e| matches!(e, Event::TurnStart { player, .. } if *player == pid))
            .count() as u32;
        let pl = g.player(pid);
        if starts != pl.turns_taken
            || o.turns[p] != pl.turns_taken - pl.extra_turns_taken + pl.turns_skipped
        {
            flags.bug("outcome-turn-counts", || {
                format!("{pid}: {starts} turns started, {:?}", o.turns)
            });
        }
    }
    let limit_hit = g
        .config()
        .turn_limit
        .is_some_and(|limit| g.players.iter().map(|p| p.turns_taken).sum::<u32>() >= limit);
    let justified = match o.reason {
        EndReason::TurnLimit => limit_hit,
        _ => game_should_be_over(g),
    };
    // Fleet's extra round comes after the end, and can refill a pile.
    if !justified && !g.has_landscape(Card::Fleet) {
        flags.bug(
            format!("game-ended-without-its-end-condition:{:?}", o.reason),
            || ctx(g),
        );
    }
    let rank = |p: usize| (o.scores[p], std::cmp::Reverse(o.turns[p]));
    let best = (0..n).map(rank).max().unwrap();
    let winners: Vec<PlayerId> = (0..n)
        .filter(|&p| rank(p) == best)
        .map(|p| PlayerId(p as u8))
        .collect();
    if winners != o.winners || winners.is_empty() {
        flags.bug("winners-inconsistent-with-scores", || format!("{o:?}"));
    }
    if let Some(limit) = g.config().turn_limit {
        let turns: u32 = g.players.iter().map(|p| p.turns_taken).sum();
        if turns > limit + n as u32 {
            flags.bug("turn-limit-overrun", || {
                format!("{turns} turns, limit {limit}")
            });
        }
    }
}

/// The legal answers to `d` worth trying one by one: every move, pile and single option
/// or card, and the extremes.
fn alternatives(g: &Game, d: &Decision) -> Vec<Response> {
    let mut out = vec![];
    match &d.kind {
        DecisionKind::Turn { moves, .. } => out.extend(moves.iter().map(|&m| Response::Move(m))),
        DecisionKind::Cards {
            candidates,
            min,
            max,
            ..
        } => {
            if (*min..=*max).contains(&1) {
                out.extend(candidates.iter().map(|&c| Response::Cards(vec![c])));
            }
            out.push(Response::Cards(candidates[..*min].to_vec()));
            out.push(Response::Cards(
                candidates[candidates.len() - *max..].to_vec(),
            ));
            let mut reversed = candidates.clone();
            reversed.reverse();
            reversed.truncate(*max);
            out.push(Response::Cards(reversed));
        }
        DecisionKind::Pile { piles, optional } => {
            out.extend(piles.iter().map(|&p| Response::Pile(Some(p))));
            if *optional {
                out.push(Response::Pile(None));
            }
        }
        DecisionKind::Choose {
            options, min, max, ..
        } => {
            if (*min..=*max).contains(&1) {
                out.extend(options.iter().map(|&o| Response::option(o)));
            }
            out.push(Response::Options(options[..*min].to_vec()));
            out.push(Response::Options(options[options.len() - *max..].to_vec()));
        }
        DecisionKind::NameCard => {
            let mut names: Vec<Card> = g.cards.clone();
            names.sort();
            names.dedup();
            out.extend(names.into_iter().map(Response::Name));
        }
        DecisionKind::Number { min, max } => {
            out.extend([*min, *max, (*min + *max) / 2].map(Response::Number));
        }
    }
    out
}

fn panic_key(payload: Box<dyn std::any::Any + Send>) -> (String, String) {
    let message = payload
        .downcast_ref::<String>()
        .cloned()
        .or_else(|| payload.downcast_ref::<&str>().map(|s| s.to_string()))
        .unwrap_or_else(|| "panic".to_string());
    // Keep capturing: this may be a probe inside a game.
    let location = CAPTURE
        .with(|c| c.borrow_mut().replace(String::new()))
        .unwrap_or_default();
    let short: String = message
        .chars()
        .filter(|c| !c.is_ascii_digit())
        .take(90)
        .collect();
    (format!("panic at {location}: {short}"), message)
}

/// Tries other answers to `d` on copies of the game, each followed for a few decisions.
fn probe(g: &Game, d: &Decision, rng: &mut ChaCha8Rng, flags: &mut Flags) {
    let mut alts = alternatives(g, d);
    alts.shuffle(rng);
    alts.truncate(10);
    for r in alts {
        let mut h = g.clone();
        h.trace = None;
        let mut agent = Biased {
            rng: ChaCha8Rng::seed_from_u64(rng.random()),
            greed: 0.9,
        };
        let mut sub = Flags::default();
        let result = catch_unwind(AssertUnwindSafe(|| {
            if let Err(e) = h.respond(r.clone()) {
                sub.bug("legal-answer-rejected", || {
                    format!("{r:?}: {e}; {}", ctx(g))
                });
                return;
            }
            for _ in 0..12 {
                let Some(next) = h.pending().cloned() else {
                    break;
                };
                check_zones(&h);
                check_decision(&h, &next, &mut sub);
                check_invariants(&h, &mut sub);
                let answer = agent.decide(&h, &next);
                if let Err(e) = h.respond(answer.clone()) {
                    sub.bug("no-valid-response-found", || {
                        format!("{answer:?}: {e}; {}", ctx(&h))
                    });
                    return;
                }
            }
        }));
        for (key, detail) in sub.bugs {
            flags.bug(key, || format!("after trying {r:?}: {detail}"));
        }
        if let Err(payload) = result {
            let (key, message) = panic_key(payload);
            flags.bug(key, || {
                format!("{message}; after trying {r:?} at {}", ctx(g))
            });
        }
    }
}

/// Pile sizes and starting decks right after setup. A list to review, since the
/// expectations here are the common rules only.
fn check_setup(g: &Game, flags: &mut Flags) {
    let n = g.num_players();
    let victory = if n == 2 { 8 } else { 12 };
    for pile in g.piles().iter().filter(|p| p.in_supply) {
        let expected = match pile.card {
            Card::Copper => continue,
            Card::Province => [8, 12, 12, 15, 18][n - 2],
            Card::Estate | Card::Duchy | Card::Colony | Card::Castles => victory,
            Card::Curse | Card::Ruins => 10 * (n - 1),
            Card::Silver => 40,
            Card::Gold => 30,
            Card::Platinum => 12,
            Card::Potion => 16,
            Card::Rats => 20,
            Card::Port => 12,
            Card::Knights => 10,
            c if c.kind() == Kind::Randomizer => 16,
            c if c.is(Types::VICTORY) && c.pile_members().len() == 1 => victory,
            _ => 10,
        };
        if pile.cards.len() != expected {
            flags.audit(
                format!(
                    "setup-pile-size:{}:{} for {n} players",
                    pile.card,
                    pile.cards.len()
                ),
                || format!("expected {expected}"),
            );
        }
    }
    for (p, pl) in g.players.iter().enumerate() {
        if pl.hand.len() != 5 || pl.deck.len() != 5 || pl.owned().count() != 10 {
            flags.bug("starting-cards", || format!("P{}: {pl:?}", p + 1));
        }
    }
}

/// The end-of-game condition, recomputed.
fn game_should_be_over(g: &Game) -> bool {
    let empty = |card| {
        g.supply_pile(card)
            .is_some_and(|p| g.pile(p).cards.is_empty())
    };
    let piles = g
        .piles()
        .iter()
        .filter(|p| p.in_supply && p.cards.is_empty())
        .count();
    empty(Card::Province)
        || empty(Card::Colony)
        || piles >= if g.num_players() >= 5 { 4 } else { 3 }
}

// ---- Driving one game ----

/// What the oracles remember between decisions.
/// An "until your next turn" effect: whose turn, the turns they had taken when it was
/// first seen, its card, and the cards that keep it going (its source and its replayer).
type UntilStart = (PlayerId, u32, Card, [Option<CardId>; 2]);

#[derive(Default)]
struct Memory {
    cards: usize,
    piles: usize,
    turn_no: u32,
    in_play: HashSet<CardId>,
    /// Cards nothing kept in play that were still there after the last Clean-up.
    survivors: BTreeMap<CardId, u32>,
    end_of_turn: Vec<(u32, Card)>,
    /// "Until your next turn" effects: id, its player, and their turn count when first seen.
    until_start: BTreeMap<u32, UntilStart>,
    decisions_this_turn: usize,
    /// Cards in play that nothing kept, at the last decision.
    unkept: HashSet<CardId>,
    /// The last two turns seen: player and whether it was an extra turn.
    last_turns: Vec<(PlayerId, bool)>,
    /// The last regular turn's player and the turns skipped in all up to then.
    last_regular: Option<(PlayerId, u32)>,
    /// Cards a lasting effect has held set aside at some decision.
    held: HashSet<CardId>,
    /// Some player has had a turn to skip (Lich).
    skips: bool,
    /// The furthest phase this turn reached, and the turn.
    phase: (u32, u8),
    /// The turn's Actions at the last decision, once Snowy Village stopped +Actions.
    frozen_actions: Option<(u32, u32)>,
}

/// Checks that compare a decision with the one before it.
fn check_progress(g: &Game, trace: &[Trace], m: &mut Memory, flags: &mut Flags) {
    if g.card_count() != m.cards {
        if g.piles().len() == m.piles && m.cards != 0 && !g.has_landscape(Card::Inherited) {
            flags.bug("cards-created-mid-game", || {
                let new: Vec<Card> = (m.cards..g.card_count())
                    .map(|i| g.printed(CardId(i as u32)))
                    .collect();
                format!("{new:?}; {}", ctx(g))
            });
        }
        m.cards = g.card_count();
        m.piles = g.piles().len();
    }
    let t = g.turn();
    let turn_no = g
        .log()
        .iter()
        .filter(|e| matches!(e, Event::TurnStart { .. }))
        .count() as u32;
    let kept: HashSet<CardId> = g.lasting().iter().flat_map(|l| l.keeps.clone()).collect();
    if turn_no != m.turn_no {
        m.decisions_this_turn = 0;
        // Cards left in play through three Clean-ups with nothing keeping them.
        let survivors: BTreeMap<CardId, u32> = g
            .players
            .iter()
            .flat_map(|p| p.in_play.clone())
            .filter(|c| !kept.contains(c) && m.unkept.contains(c))
            .map(|c| (c, m.survivors.get(&c).copied().unwrap_or(0) + 1))
            .collect();
        // Cards their owner's own Clean-up left in play with nothing keeping them: a list,
        // since Journey does that.
        for &c in survivors.keys() {
            if Some(g.zone(c)) == t.previous.map(Zone::InPlay) && turn_no == m.turn_no + 1 {
                let journey = if g.has_landscape(Card::Journey) {
                    " (with Journey)"
                } else {
                    ""
                };
                flags.audit(
                    format!("left-in-play-by-its-own-clean-up{journey}:{}", g.printed(c)),
                    || format!("{c:?}; lasting {:?}; {}", g.lasting(), ctx(g)),
                );
            }
        }
        // Journey bought on both of two turns in a row leaves cards out through two.
        for (&c, &n) in &survivors {
            if n >= 3 {
                flags.bug(format!("card-stuck-in-play:{}", g.printed(c)), || {
                    format!(
                        "{c:?} in {:?}; lasting {:?}; {}",
                        g.zone(c),
                        g.lasting(),
                        ctx(g)
                    )
                });
            }
        }
        m.survivors = survivors;
        for &(id, card) in &m.end_of_turn {
            // Taskmaster repeating at the start of a turn reuses its lasting effect.
            if card != Card::Taskmaster
                && g.lasting()
                    .iter()
                    .any(|l| l.id == id && cards::lasting_ends(g, l) == Until::EndOfTurn)
            {
                flags.bug(format!("this-turn-effect-outlives-its-turn:{card}"), || {
                    ctx(g)
                });
            }
        }
        m.end_of_turn.clear();
        // Unless this turn's start-of-turn effects have emptied a pile since.
        let quiet = matches!(g.log().last(), Some(Event::TurnStart { .. }));
        if g.ending.is_none() && m.turn_no > 0 && quiet && game_should_be_over(g) {
            flags.bug("game-continues-after-its-end-condition", || ctx(g));
        }
        check_turn_order(g, m, turn_no, flags);
        m.turn_no = turn_no;
    }
    // A turn's phases come in order; only a few effects return to the Action phase.
    let rank = match t.phase {
        Phase::Start => 0,
        Phase::Action => 1,
        Phase::Buy => 2,
        Phase::Night => 3,
        Phase::Cleanup => 4,
        Phase::End => 5,
    };
    let returns = [Card::Villa, Card::Cavalry, Card::Launch, Card::Continue];
    if m.phase.0 == turn_no
        && rank < m.phase.1
        && !returns
            .iter()
            .any(|&c| g.cards.contains(&c) || g.has_landscape(c))
    {
        flags.bug(format!("phase-went-back-to:{:?}", t.phase), || ctx(g));
    }
    m.phase = (
        turn_no,
        if m.phase.0 == turn_no {
            rank.max(m.phase.1)
        } else {
            rank
        },
    );
    m.decisions_this_turn += 1;
    if m.decisions_this_turn == 4000 {
        flags.bug("turn-with-4000-decisions", || ctx(g));
    }
    // "Until your next turn" effects that ended before that turn started. (One whose
    // extra turn was skipped ends early by design: Voyage and Outpost with Lich.)
    m.skips |= g
        .players
        .iter()
        .any(|p| p.turns_skipped > 0 || p.skip_turns > 0);
    let skipped = m.skips;
    m.until_start
        .retain(|&id, &mut (p, first, card, [source, tracker])| {
            let alive = g.lasting().iter().any(|l| l.id == id);
            // A Duration that left play, or whose replayer did, ended with the turn, perhaps
            // with no decision since.
            let left_play = [source, tracker]
                .into_iter()
                .flatten()
                .any(|c| !matches!(g.zone(c), Zone::InPlay(_)));
            if !alive && g.player(p).turns_taken == first && !skipped && !left_play {
                flags.bug(format!("until-next-turn-effect-ended-early:{card}"), || {
                    ctx(g)
                });
            }
            alive
        });
    m.in_play = g.players.iter().flat_map(|p| p.in_play.clone()).collect();
    m.unkept = m
        .in_play
        .iter()
        .copied()
        .filter(|c| !kept.contains(c))
        .collect();
    for l in g.lasting() {
        m.held
            .extend(l.data.cards.iter().chain(&l.data.pick).copied());
    }
    // An effect that ended with a turn since the last decision may have held what it set
    // aside then: a replay whose Throne Room never entered play (Captain's, replayed by
    // Daimyo), a Cargo Ship trashed in Clean-up.
    let mut aside = vec![];
    for entry in trace {
        match entry {
            Trace::Moved {
                card,
                to: Zone::SetAside(_),
                ..
            } => aside.push(*card),
            Trace::Turn { .. } => m.held.extend(aside.drain(..)),
            _ => {}
        }
    }
    if matches!(
        g.pending().map(|d| &d.kind),
        Some(DecisionKind::Turn { .. })
    ) {
        check_set_aside(g, m, flags);
    }
    check_looking(g, flags);
    check_stayed(g, flags);
    check_turn_records(g, flags);
    if t.ignore_plus_actions {
        if let Some((turn, actions)) = m.frozen_actions
            && turn == turn_no
            && t.actions > actions
        {
            flags.bug("actions-rose-after-snowy-village", || ctx(g));
        }
        m.frozen_actions = Some((turn_no, t.actions));
    } else {
        m.frozen_actions = None;
    }
    for l in g.lasting() {
        // When the players are told it ends.
        match cards::lasting_ends(g, l) {
            Until::EndOfTurn => {
                // A Duration that left play ends with the turn.
                m.until_start.remove(&l.id);
                if !m.end_of_turn.iter().any(|&(id, _)| id == l.id) {
                    m.end_of_turn.push((l.id, l.card));
                }
            }
            Until::StartOfTurn(p) => {
                let turns = g.player(p).turns_taken;
                // Made on the owner's turn `l.turn`; seen first at the start of the turn
                // that ends it, it was made before that turn.
                let made = if l.owner == p {
                    l.turn.min(turns)
                } else {
                    turns
                };
                let &mut (_, first, _, _) =
                    m.until_start
                        .entry(l.id)
                        .or_insert((p, made, l.card, [l.source, l.tracker]));
                let starting = t.player == p && t.phase == Phase::Start;
                // Taskmaster repeating at the start of a turn reuses its lasting effect.
                if turns > first && !starting && l.card != Card::Taskmaster {
                    flags.bug(
                        format!("until-next-turn-effect-outlives-it:{}", l.card),
                        || format!("{l:?}; {}", ctx(g)),
                    );
                }
            }
            Until::ExtraTurn => {
                let pending = g.extra_turns().iter().any(|e| e.lasting == Some(l.id));
                if !pending && !l.ending {
                    flags.bug(
                        format!("extra-turn-effect-outlives-its-turn:{}", l.card),
                        || format!("{l:?}; {}", ctx(g)),
                    );
                }
            }
            Until::Removed => {}
        }
        if l.until == Until::Removed {
            if let Some(c) = l.source
                && g.printed(c) == l.card
                && l.card.is(Types::DURATION)
                && matches!(g.zone(c), Zone::Discard(_) | Zone::Deck(_) | Zone::Hand(_))
            {
                flags.audit(
                    format!("duration-cleaned-up-with-its-effect-pending:{}", l.card),
                    || format!("{l:?}, card in {:?}; {}", g.zone(c), ctx(g)),
                );
            }
            for &c in &l.keeps {
                let age = g.player(l.owner).turns_taken - l.turn;
                if age >= 1
                    && matches!(g.zone(c), Zone::InPlay(_))
                    && !g.printed(c).is(Types::DURATION)
                {
                    flags.audit(
                        format!("non-duration-kept-in-play:{} by {}", g.printed(c), l.card),
                        || format!("{l:?}; {}", ctx(g)),
                    );
                }
            }
            // A Duration's effect that has not resolved two of its owner's turns
            // later: fine for the open-ended ones, so only a list to review.
            let age = g.player(l.owner).turns_taken - l.turn;
            let past_start = t.player == l.owner && t.phase != Phase::Start;
            if age >= 3 || (age == 2 && past_start) {
                flags.audit(format!("long-lived-effect:{}", l.card), || {
                    format!("{l:?}; {}", ctx(g))
                });
            }
        }
    }
}

/// Extra turns that never come as a third turn in a row.
const NOT_THIRD: &[Card] = &[
    Card::Outpost,
    Card::Voyage,
    Card::Mission,
    Card::Journey,
    Card::IslandFolk,
];

/// At a new turn's first decision: turn order and the limits on extra turns.
fn check_turn_order(g: &Game, m: &mut Memory, turn_no: u32, flags: &mut Flags) {
    let t = g.turn();
    let n = g.num_players() as u32;
    let skipped: u32 = g.players.iter().map(|p| p.turns_skipped).sum();
    if turn_no != m.turn_no + 1 {
        // A turn passed without a decision: start over.
        m.last_turns.clear();
        m.last_regular = None;
    }
    let extra = t.extra.is_some();
    let streak = m
        .last_turns
        .iter()
        .rev()
        .take_while(|(p, _)| *p == t.player)
        .count();
    if let Some(source) = t.extra {
        if NOT_THIRD.contains(&source) && streak >= 2 {
            flags.bug(format!("third-turn-in-a-row:{source}"), || ctx(g));
        }
    } else if g.ending.is_none() {
        if let Some((last, skipped_then)) = m.last_regular {
            let expected = (last.0 as u32 + 1 + (skipped - skipped_then)) % n;
            if t.player.0 as u32 != expected {
                flags.bug("turn-order", || {
                    format!(
                        "after {last}'s turn (skips {skipped_then} -> {skipped}); {}",
                        ctx(g)
                    )
                });
            }
        }
        m.last_regular = Some((t.player, skipped));
    }
    m.last_turns.push((t.player, extra));
    if t.previous != m.last_turns.iter().rev().nth(1).map(|(p, _)| *p) && m.last_turns.len() > 1 {
        flags.bug("turn-previous-wrong", || ctx(g));
    }
}

/// `Turn`'s records of this turn agree with the log.
fn check_turn_records(g: &Game, flags: &mut Flags) {
    let t = g.turn();
    let log = g.log();
    let Some(start) = log
        .iter()
        .rposition(|e| matches!(e, Event::TurnStart { .. }))
    else {
        return;
    };
    let mut gained = vec![];
    let mut trashed = vec![];
    let mut bought = vec![];
    let mut played = vec![];
    let mut played_by = vec![];
    for e in &log[start..] {
        match e {
            Event::Gained { player, card, .. } if *player == t.player => gained.push(*card),
            Event::Trashed { player, card } if *player == t.player => trashed.push(*card),
            Event::Bought { player, card } if *player == t.player => bought.push(*card),
            Event::Played { player, card } => {
                played.push(*card);
                played_by.push(*player);
            }
            _ => {}
        }
    }
    if t.extra == Some(Card::Mission) && !bought.is_empty() {
        flags.bug("card-bought-on-a-mission-turn", || ctx(g));
    }
    if t.extra == Some(Card::Voyage) && t.hand_plays > 3 {
        flags.bug("more-than-three-plays-on-a-voyage-turn", || ctx(g));
    }
    // "Once per turn" and "Once per game" Events.
    for l in g.landscapes() {
        let text = l.text().to_lowercase();
        let bought = |events: &[Event], p: PlayerId| {
            events
                .iter()
                .filter(|e| matches!(e, Event::BoughtLandscape { player, card } if *player == p && card == l))
                .count()
        };
        if text.starts_with("once per turn") && bought(&log[start..], t.player) > 1 {
            flags.bug(format!("once-per-turn-event-bought-twice:{l}"), || ctx(g));
        }
        if text.starts_with("once per game") && bought(log, t.player) > 1 {
            flags.bug(format!("once-per-game-event-bought-twice:{l}"), || ctx(g));
        }
    }
    if t.played_by != played_by {
        flags.bug("turn-record-differs-from-log:played_by", || {
            format!(
                "turn.played_by = {:?}, log says {played_by:?}; {}",
                t.played_by,
                ctx(g)
            )
        });
    }
    for (what, turn, logged) in [
        ("gained", &t.gained, &gained),
        ("trashed", &t.trashed, &trashed),
        ("bought_cards", &t.bought_cards, &bought),
        ("played", &t.played, &played),
    ] {
        if turn != logged {
            flags.bug(format!("turn-record-differs-from-log:{what}"), || {
                format!("turn.{what} = {turn:?}, log says {logged:?}; {}", ctx(g))
            });
        }
    }
}

#[derive(Default)]
struct Report {
    flags: Flags,
    plays: BTreeMap<Card, u32>,
    /// Kingdom cards and landscapes in the game.
    present: BTreeSet<Card>,
    /// The Prophecy took effect.
    prophesied: bool,
    landscapes_bought: u32,
    steps: usize,
    turns: u32,
    reason: Option<EndReason>,
    setup: String,
    /// The whole log, kept only for `FUZZ_LOG`: events, and with `FUZZ_TRACE` also card
    /// moves, decisions and answers.
    log: Vec<String>,
}

struct Plan {
    config: GameConfig,
    rich: u8,
    greed: Vec<f64>,
    /// How often a turn gets extra Actions, Buys, $ or cards (see [`Boost`]).
    boost: f64,
    /// Start with decks of only a few cards.
    thin: bool,
}

/// A mid-turn windfall, as if earlier plays had given it: it takes games to big turns,
/// emptied decks and expensive buys that random play rarely reaches.
#[derive(Clone, Debug)]
enum Boost {
    Turn { actions: u32, buys: u32, coins: u32 },
    Draw(usize),
}

fn apply_boost(g: &mut Game, boost: &Boost) {
    match *boost {
        Boost::Turn {
            actions,
            buys,
            coins,
        } => {
            if !g.turn.ignore_plus_actions {
                g.turn.actions += actions;
            }
            g.turn.buys += buys;
            g.turn.coins += coins;
        }
        Boost::Draw(n) => {
            let p = g.turn.player;
            for _ in 0..n {
                if let Some(&top) = g.players[p.index()].deck.last() {
                    g.move_card(top, Zone::Hand(p), Pos::Top);
                }
            }
        }
    }
}

/// Leaves each player only a few cards.
fn thin_decks(g: &mut Game, rng: &mut ChaCha8Rng) {
    for p in 0..g.num_players() {
        let keep = rng.random_range(1..=8);
        loop {
            let pl = &g.players[p];
            let cards: Vec<CardId> = pl.deck.iter().chain(&pl.hand).copied().collect();
            if cards.len() <= keep {
                break;
            }
            g.move_card(*cards.choose(rng).unwrap(), Zone::Trash, Pos::Top);
        }
    }
}

fn plan(family: Family, seed: u64) -> Plan {
    let mut rng = ChaCha8Rng::seed_from_u64(seed ^ 0x0b5e_55ed);
    let k = family(&mut rng, seed);
    let players = *[2u8, 2, 2, 3, 3, 4, 4, 5, 6].choose(&mut rng).unwrap();
    let mut config = GameConfig::new(players, k.cards, seed);
    config.landscapes = k.landscapes;
    config.setup_choices = k.choices;
    config.colonies = *[None, Some(true), Some(false)].choose(&mut rng).unwrap();
    config.shelters = *[None, Some(true), Some(false)].choose(&mut rng).unwrap();
    config.turn_limit = Some(players as u32 * rng.random_range(8..=16));
    let rich = if k.plain {
        1
    } else {
        *[0u8, 1, 1, 2, 2, 2].choose(&mut rng).unwrap()
    };
    let greed = (0..players)
        .map(|_| *[0.0, 0.6, 0.9, 0.9, 0.97].choose(&mut rng).unwrap())
        .collect();
    let boost = if k.plain {
        0.0
    } else {
        *[0.0, 0.0, 0.1, 0.3].choose(&mut rng).unwrap()
    };
    let thin = !k.plain && rng.random_bool(0.15);
    Plan {
        config,
        rich,
        greed,
        boost,
        thin,
    }
}

const MAX_STEPS: usize = 150_000;

/// Sets the game up. A choice the rules don't allow is dropped (and listed); the rest are
/// kept.
fn build(config: &mut GameConfig, flags: &mut Flags) -> Option<Game> {
    loop {
        match crate::setup::build(config.clone()) {
            Ok(g) => return Some(g),
            Err(crate::SetupError::InvalidChoice { by, choice })
                if config.setup_choices.contains_key(&by) =>
            {
                flags.audit(format!("setup-choice-rejected:{by}:{choice}"), String::new);
                let list = config.setup_choices.get_mut(&by).unwrap();
                list.retain(|c| *c != choice);
                if list.is_empty() {
                    config.setup_choices.remove(&by);
                }
            }
            Err(crate::SetupError::ChoiceCount(by) | crate::SetupError::UnusedChoice(by))
                if config.setup_choices.contains_key(&by) =>
            {
                flags.audit(format!("setup-choice-unused:{by}"), String::new);
                config.setup_choices.remove(&by);
            }
            Err(e) => {
                flags.bug(format!("setup-fails:{e}"), || format!("{config:?}"));
                return None;
            }
        }
    }
}

fn setup_text(seed: u64, plan: &Plan, g: &Game) -> String {
    let config = &plan.config;
    format!(
        "seed {seed}: {} players, rich {}, boost {}, thin {}, greed {:?}, kingdom {:?}, landscapes {:?}, prophecy {:?}, \
         colonies {:?}, shelters {:?}, choices {:?}",
        config.players,
        plan.rich,
        plan.boost,
        plan.thin,
        plan.greed,
        config.kingdom,
        g.landscapes(),
        g.prophecy(),
        config.colonies,
        config.shelters,
        config.setup_choices
    )
}

fn drive(seed: u64, mut plan: Plan, report: &mut Report) {
    let flags = &mut report.flags;
    let mut rng = ChaCha8Rng::seed_from_u64(seed ^ 0xd1ce);
    let Some(mut g) = build(&mut plan.config, flags) else {
        return;
    };
    check_setup(&g, flags);
    enrich(&mut g, &mut rng, plan.rich);
    if plan.thin {
        thin_decks(&mut g, &mut rng);
    }
    report.setup = setup_text(seed, &plan, &g);
    check_zones(&g);
    let base = g.clone();
    g.enable_trace();
    g.start();

    let n = g.num_players();
    let mut agents: Vec<Biased> = (0..n)
        .map(|p| Biased {
            rng: ChaCha8Rng::seed_from_u64(seed * 16 + p as u64),
            greed: plan.greed[p],
        })
        .collect();
    let mut k = Knowledge::new(n);
    let mut tracker = Tracker::new(n);
    let mut memory = Memory::default();
    let mut responses: Vec<Response> = vec![];
    let mut boosts: Vec<(usize, Boost)> = vec![];
    let mut log_seen = 0;
    let thorough = seed.is_multiple_of(4);
    // How often a view is compared with the view of a game with two hidden cards
    // swapped (`FUZZ_VIEWS`, a percentage).
    let view_rate = env_u64("FUZZ_VIEWS").map_or(0.02, |pct| pct.min(100) as f64 / 100.0);
    let keep_log = std::env::var("FUZZ_LOG").is_ok();
    let keep_trace = std::env::var("FUZZ_TRACE").is_ok();
    loop {
        let trace: Vec<Trace> = g.trace().unwrap_or_default().to_vec();
        for entry in &trace {
            tracker.apply(&g, entry, flags);
        }
        check_step(&g, &tracker, &g.log()[log_seen..], &trace, flags);
        check_move_discipline(&g, &g.log()[log_seen..], &trace, flags);
        check_references(&g, flags);
        if keep_log {
            if keep_trace {
                report.log.extend(trace.iter().filter_map(|t| match t {
                    Trace::Moved { card, from, to, .. } => Some(format!(
                        "    move {card:?} ({}) {from:?} -> {to:?}",
                        g.printed(*card)
                    )),
                    _ => None,
                }));
            }
            report
                .log
                .extend(g.log()[log_seen..].iter().map(|e| format!("{e:?}")));
        }
        log_seen = g.log().len();
        k.update(&mut g);
        tracker.observe(&g, flags);
        check_zones(&g);
        // The costly checks: at every decision of some games, one in four in the rest.
        if thorough || g.pending().is_none() || rng.random_bool(0.25) {
            check_knowledge(&g, &k, flags);
            check_views(&g, flags);
        }
        let Some(d) = g.pending().cloned() else { break };
        check_decision(&g, &d, flags);
        check_plays(&g, &d, flags);
        check_against_text(&g, &d, flags);
        check_invariants(&g, flags);
        check_progress(&g, &trace, &mut memory, flags);

        if rng.random_bool(0.02) {
            let before = json(&g);
            for r in invalid_responses(&g, &d) {
                if g.respond(r.clone()).is_ok() {
                    flags.bug("invalid-response-accepted", || format!("{r:?} to {d:?}"));
                    return;
                }
            }
            if json(&g) != before {
                flags.bug("rejected-response-changed-the-game", || ctx(&g));
            }
        }
        if rng.random_bool(0.01) {
            let before = json(&g);
            match serde_json::from_str::<Game>(&before) {
                Ok(restored) => {
                    if json(&restored) != before {
                        flags.bug("serialization-round-trip-differs", || ctx(&g));
                    }
                    g = restored;
                }
                Err(e) => flags.bug("game-does-not-deserialize", || format!("{e}; {}", ctx(&g))),
            }
        }

        if rng.random_bool(0.03) {
            probe(&g, &d, &mut rng, flags);
        }
        if rng.random_bool(view_rate) {
            check_view_noninterference(&g, &k, &mut rng, flags);
            if let Err(error) = super::knowledge::check_revealed_order_twins(&g, &k, &mut rng) {
                flags.bug("knowledge-depends-on-private-revealed-order", || {
                    format!("{error}; {}", ctx(&g))
                });
            }
        }

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
            apply_boost(&mut g, &boost);
            boosts.push((responses.len(), boost));
        }

        let r = agents[d.player.index()].decide(&g, &d);
        if keep_log && keep_trace {
            report.log.push(format!("  ? {d:?}"));
            report.log.push(format!("  > {r:?}"));
        }
        heartbeat(Some(&report.setup));
        if let Err(e) = g.respond(r.clone()) {
            flags.bug("no-valid-response-found", || {
                format!("{r:?} rejected: {e}; {}", ctx(&g))
            });
            return;
        }
        responses.push(r);
        report.steps += 1;
        if report.steps >= MAX_STEPS {
            flags.bug("game-does-not-end", || ctx(&g));
            return;
        }
    }
    check_outcome(&g, flags);
    report.present = g.cards.iter().chain(g.landscapes()).copied().collect();
    report.prophesied = g.prophecy().is_some_and(|(_, sun)| sun == 0);
    report.reason = g.outcome().map(|o| o.reason);
    report.turns = g.players.iter().map(|p| p.turns_taken).sum();
    for e in g.log() {
        match e {
            Event::Played { card, .. } => *report.plays.entry(*card).or_default() += 1,
            Event::UsedWay { way, .. } => *report.plays.entry(*way).or_default() += 1,
            Event::BoughtLandscape { card, .. } => {
                report.landscapes_bought += 1;
                *report.plays.entry(*card).or_default() += 1;
            }
            _ => {}
        }
    }

    // The same responses, untraced and never serialized, give the same game.
    let mut replay = base;
    replay.start();
    let mut boosts = boosts.iter().peekable();
    for (i, r) in responses.iter().enumerate() {
        while let Some((_, boost)) = boosts.next_if(|(at, _)| *at == i) {
            apply_boost(&mut replay, boost);
        }
        if let Err(e) = replay.respond(r.clone()) {
            flags.bug("replay-rejects-a-response", || format!("{r:?}: {e}"));
            return;
        }
    }
    g.trace = None;
    if json(&replay) != json(&g) {
        flags.bug("replay-differs", || {
            format!(
                "logs equal: {}, outcomes {:?} / {:?}",
                replay.log() == g.log(),
                replay.outcome(),
                g.outcome()
            )
        });
    }
}

type Driver = fn(u64, Plan, &mut Report);

fn run_game(family: Family, driver: Driver, seed: u64) -> Report {
    install_hook();
    let mut report = Report::default();
    let plan = plan(family, seed);
    CAPTURE.with(|c| *c.borrow_mut() = Some(String::new()));
    let result = catch_unwind(AssertUnwindSafe(|| driver(seed, plan, &mut report)));
    heartbeat(None);
    if let Err(payload) = result {
        let (key, message) = panic_key(payload);
        report.flags.bug(key, || message);
    }
    CAPTURE.with(|c| *c.borrow_mut() = None);
    report
}

/// Runs a family's seeds and fails with every oracle that fired.
fn run_family(name: &str, family: Family, default_seeds: u64) {
    run_family_with(name, family, drive, default_seeds);
}

fn run_family_with(name: &str, family: Family, driver: Driver, default_seeds: u64) {
    let start = env_u64("FUZZ_START").unwrap_or(0);
    let seeds: Vec<u64> = match env_u64("FUZZ_ONLY") {
        Some(seed) => vec![seed],
        None => (start..start + fuzz_seeds(default_seeds)).collect(),
    };
    let threads = env_u64("FUZZ_THREADS").unwrap_or(1).max(1) as usize;
    let next = std::sync::atomic::AtomicUsize::new(0);
    let reports: Vec<(u64, Report)> = std::thread::scope(|s| {
        let handles: Vec<_> = (0..threads)
            .map(|_| {
                s.spawn(|| {
                    let mut out = vec![];
                    loop {
                        let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        let Some(&seed) = seeds.get(i) else { break };
                        out.push((seed, run_game(family, driver, seed)));
                    }
                    out
                })
            })
            .collect();
        let mut all: Vec<(u64, Report)> = handles
            .into_iter()
            .flat_map(|h| h.join().expect("fuzz worker"))
            .collect();
        all.sort_by_key(|(seed, _)| *seed);
        all
    });

    let verbose = std::env::var("FUZZ_ONLY").is_ok();
    let mut bugs: BTreeMap<String, (usize, u64, String)> = BTreeMap::new();
    let mut audit: BTreeMap<String, (usize, u64, String)> = BTreeMap::new();
    for (seed, report) in &reports {
        if verbose {
            eprintln!("{name} {}", report.setup);
            for (i, e) in report.log.iter().enumerate() {
                eprintln!("LOG {i} {e}");
            }
        }
        if std::env::var("FUZZ_KEYS").is_ok() {
            for (key, _) in &report.flags.bugs {
                eprintln!("KEY {name} {seed} {key} | {}", report.setup);
            }
        }
        for (into, from) in [
            (&mut bugs, &report.flags.bugs),
            (&mut audit, &report.flags.audit),
        ] {
            for (key, detail) in from {
                into.entry(key.clone())
                    .or_insert_with(|| (0, *seed, detail.clone()))
                    .0 += 1;
            }
        }
    }
    if std::env::var("FUZZ_STATS").is_ok() {
        let games = reports.len().max(1);
        let steps: usize = reports.iter().map(|(_, r)| r.steps).sum();
        let turns: u32 = reports.iter().map(|(_, r)| r.turns).sum();
        let mut plays: BTreeMap<Card, u32> = BTreeMap::new();
        let mut reasons: BTreeMap<String, u32> = BTreeMap::new();
        for (_, r) in &reports {
            for (c, n) in &r.plays {
                *plays.entry(*c).or_default() += n;
            }
            *reasons.entry(format!("{:?}", r.reason)).or_default() += 1;
        }
        let bought: u32 = reports.iter().map(|(_, r)| r.landscapes_bought).sum();
        let prophesied = reports.iter().filter(|(_, r)| r.prophesied).count();
        eprintln!("{name}: {prophesied} games with their Prophecy in effect");
        eprintln!(
            "{name}: {games} games, {} decisions and {} turns per game, {} cards played \
             ({} plays), {bought} landscapes bought, ends {reasons:?}",
            steps / games,
            turns as usize / games,
            plays.len(),
            plays.values().sum::<u32>(),
        );
    }
    // Cards and landscapes in games of this run, with how often they were used (played,
    // bought, or used as a Way), for finding what the families never reach.
    if std::env::var("FUZZ_COVER").is_ok() {
        let mut present: BTreeMap<Card, (u32, u32)> = BTreeMap::new();
        for (_, r) in &reports {
            for c in &r.present {
                let e = present.entry(*c).or_default();
                e.0 += 1;
                e.1 += r.plays.get(c).copied().unwrap_or(0);
            }
        }
        for (c, (games, uses)) in present {
            eprintln!("COVER {name}\t{c}\t{games}\t{uses}");
        }
    }
    if std::env::var("FUZZ_AUDIT").is_ok() || verbose {
        for (key, (count, seed, detail)) in &audit {
            let detail: String = detail.chars().take(1500).collect();
            eprintln!("AUDIT {name} [{count} games, first seed {seed}] {key}\n    {detail}");
        }
    }
    let open = |key: &str| OPEN.iter().find(|(prefix, _)| key.starts_with(prefix));
    for (key, (count, seed, _)) in &bugs {
        if let Some((_, about)) = open(key) {
            eprintln!("OPEN {name} [{count} games, first seed {seed}] {key}: {about}");
        }
    }
    bugs.retain(|key, _| open(key).is_none());
    if bugs.is_empty() {
        return;
    }
    let mut out = format!("{name}: {} oracle(s) fired\n", bugs.len());
    for (key, (count, seed, detail)) in &bugs {
        let detail: String = detail
            .chars()
            .take(if verbose { 6000 } else { 1200 })
            .collect();
        out += &format!("- {key} ({count} games, first seed {seed})\n    {detail}\n");
    }
    panic!("{out}");
}

macro_rules! families {
    ($($test:ident => $family:ident, $seeds:expr;)*) => {
        $(
            #[test]
            fn $test() {
                run_family(stringify!($family), $family, $seeds);
            }
        )*

        const FAMILIES: &[Family] = &[$($family),*];
    };
}

families! {
    fuzz_mixed_kingdoms => fam_mixed, 6;
    fuzz_many_landscapes => fam_landscapes, 6;
    fuzz_thrones_and_durations => fam_thrones_durations, 6;
    fuzz_commands_and_one_shots => fam_commands_oneshots, 6;
    fuzz_gain_replacements => fam_gains, 6;
    fuzz_cost_changes_and_remodelers => fam_costs, 6;
    fuzz_attacks_and_reactions => fam_attacks, 6;
    fuzz_extra_turns => fam_extra_turns, 6;
    fuzz_ways => fam_ways, 6;
    fuzz_way_of_the_chameleon => fam_chameleon, 6;
    fuzz_split_piles => fam_piles, 6;
    fuzz_prophecies => fam_prophecies, 6;
    fuzz_black_market_and_banes => fam_markets, 6;
    fuzz_night_fate_doom => fam_night, 6;
    fuzz_mats_and_tokens => fam_mats_tokens, 6;
    fuzz_debt_and_vp => fam_debt_vp, 6;
    fuzz_allies => fam_allies, 6;
    fuzz_traits => fam_traits, 6;
    fuzz_plunder_and_shadows => fam_plunder, 6;
    fuzz_scoring => fam_scoring, 6;
    fuzz_cards_held_outside_the_supply => fam_holders, 6;
    fuzz_odd_configurations => fam_odd, 6;
    fuzz_cards_leaving_play => fam_leaving_play, 6;
    fuzz_start_of_turn_effects => fam_start_of_turn, 6;
}
