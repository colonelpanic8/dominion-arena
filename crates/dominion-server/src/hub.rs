//! The games a server hosts, and the seat tokens that let a client act for a seat.

use crate::steps::Runner;
use crate::store::Store;
use dominion_agent::{AgentView, Answer, AnswerError, Answered, Table, WayChoice};
use dominion_engine::{
    Card, DealRules, Game, GameConfig, LandscapeSlot, PlayerId, SLOT_KINDS, Set,
};
pub use dominion_protocol::api::{
    Created, DealRequest, Dealt, GameSummary, NewGame, SeatToken, SeatView,
};
use dominion_protocol::api::{Replay, ReplayTurn, UndoRequest, UndoState};
use dominion_protocol::lobby::{Clock, ClockView, GameMeta, MyGame, Source, UndoRule, UserRef};
use dominion_protocol::view::GameInfo;
use rand::Rng;
use serde::{Deserialize, Serialize};
use std::any::Any;
use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::fs;
use std::io;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::watch;

/// What [`Hub::steps`] did.
#[derive(Clone, Debug)]
pub struct Stepped {
    pub log: Vec<String>,
    pub stopped: Option<(String, String)>,
}

#[derive(Debug, thiserror::Error)]
pub enum HubError {
    #[error("{0}")]
    BadRequest(String),
    #[error("unknown game {0}")]
    UnknownGame(String),
    #[error("that token is not a seat token of game {0}")]
    BadToken(String),
    #[error("{0}")]
    Forbidden(String),
    #[error("{0}")]
    Conflict(String),
    /// The server is hosting as many games as it will.
    #[error("{0}")]
    Busy(String),
    #[error(transparent)]
    Answer(#[from] AnswerError),
    /// The engine panicked. Whatever it was doing was dropped, so the game is as it was.
    #[error("the game engine failed ({0}); nothing was changed. This is a bug in the engine")]
    Engine(String),
}

/// What a caught panic said.
pub(crate) fn panic_message(panic: &(dyn Any + Send)) -> String {
    panic
        .downcast_ref::<&str>()
        .map(|s| s.to_string())
        .or_else(|| panic.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "panic".into())
}

/// Runs engine code, turning a panic into an error so it stays in the game it happened in.
pub(crate) fn engine<T>(f: impl FnOnce() -> T) -> Result<T, HubError> {
    catch_unwind(AssertUnwindSafe(f)).map_err(|p| HubError::Engine(panic_message(&*p)))
}

/// Tables are only ever replaced whole, so one is consistent even behind a poisoned lock.
pub(crate) fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Milliseconds since the Unix epoch.
pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

/// What the server knows about a game besides its table, saved with it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Meta {
    pub source: Source,
    /// Per seat: the id of the account playing it.
    pub owners: Vec<Option<String>>,
    pub rated: bool,
    pub watchable: bool,
    /// The deciding seat forfeits when it is away or takes too long ([`Timing`]).
    pub timed: bool,
    /// A chess clock: a seat whose time runs out forfeits.
    pub clock: Option<ClockState>,
    /// Whether answers can be taken back: an undo shows a player what would have happened.
    pub undo: UndoRule,
    /// Per seat: played by one of the server's agents, which allow every undo.
    pub agents: Vec<bool>,
    /// The lobby table that started it.
    pub table: Option<String>,
    /// Milliseconds since the Unix epoch.
    pub started: u64,
    /// Why it ended before its end: `resigned`, `abandoned` or `timeout`.
    pub ended: Option<String>,
    pub ended_at: Option<u64>,
    /// Its record is in the history.
    pub recorded: bool,
    /// The account that created it with `create_game`, if one was signed in.
    pub host: Option<String>,
    /// Per seat: what its player is told before the game (`NewGame::briefs`).
    pub briefs: Vec<Option<String>>,
    /// The question each server agent answers once the game is over.
    pub debrief: Option<String>,
    /// Per seat: its answer to `debrief`.
    pub debriefs: Vec<Option<String>>,
}

/// A game's chess clock, as of its last change.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClockState {
    pub rule: Clock,
    /// Per seat: milliseconds left.
    #[serde(default)]
    pub left: Vec<u64>,
    /// The seat whose time has run since the last change.
    #[serde(default)]
    pub running: Option<PlayerId>,
    /// Per seat: the turns it has had the increment for.
    #[serde(default)]
    pub credited: Vec<u32>,
}

impl ClockState {
    pub fn new(rule: Clock) -> ClockState {
        ClockState {
            rule,
            left: vec![],
            running: None,
            credited: vec![],
        }
    }

    /// After a change `elapsed` milliseconds after the last one: charges the seat whose
    /// time ran, adds the increment for each turn begun since, and starts `table`'s
    /// deciding seat's time.
    fn advance(&mut self, table: &Table, elapsed: u64) {
        let seats = table.names().len();
        if self.left.len() != seats {
            self.left = vec![u64::from(self.rule.initial) * 1000; seats];
        }
        self.credited.resize(seats, 0);
        if let Some(running) = self.running
            && let Some(left) = self.left.get_mut(running.index())
        {
            *left = left.saturating_sub(elapsed);
        }
        for seat in 0..seats {
            let taken = table.game().player(PlayerId(seat as u8)).turns_taken;
            let begun = taken.saturating_sub(self.credited[seat]);
            if begun > 0 {
                self.left[seat] += u64::from(begun) * u64::from(self.rule.increment) * 1000;
                self.credited[seat] = taken;
            }
        }
        self.running = if table.is_over() {
            None
        } else {
            table.deciding()
        };
    }

    /// `seat`'s time `elapsed` milliseconds after the last change.
    fn left_at(&self, seat: usize, elapsed: u64) -> u64 {
        let left = self.left.get(seat).copied().unwrap_or(0);
        if self.running.is_some_and(|r| r.index() == seat) {
            left.saturating_sub(elapsed)
        } else {
            left
        }
    }

    /// The running seat and when its time runs out, given when the last change was.
    fn flag_at(&self, since: u64) -> Option<(PlayerId, u64)> {
        let running = self.running?;
        Some((running, since + self.left_at(running.index(), 0)))
    }

    fn view(&self, elapsed: u64) -> ClockView {
        ClockView {
            left: (0..self.left.len())
                .map(|s| self.left_at(s, elapsed))
                .collect(),
            free: vec![],
            running: self.running.map(|r| r.0),
            increment: Some(u64::from(self.rule.increment) * 1000),
        }
    }
}

/// A hosted game, for the operator ([`Hub::hosting`]).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Hosting {
    pub id: String,
    pub names: Vec<String>,
    pub running: bool,
    pub source: Source,
    /// The lobby table that started it.
    pub table: Option<String>,
    /// The server's agents play a seat.
    pub agents: bool,
    /// Since it last changed or a seat last used its token.
    pub idle_seconds: u64,
    /// A call waits on it for a seat.
    pub connected: bool,
}

/// Whether the server agent at `seat` still owes its debrief at `now`, once the game is
/// over.
fn debrief_owed(meta: &Meta, seat: usize, now: u64) -> bool {
    meta.debrief.is_some()
        && meta.agents.get(seat).copied().unwrap_or(false)
        && meta.debriefs.get(seat).is_none_or(Option::is_none)
        && meta
            .ended_at
            .is_none_or(|at| now < at + DEBRIEF_WAIT.as_millis() as u64)
}

/// A game as a record ([`Hub::report`]).
#[derive(Clone, Debug)]
pub struct Report {
    pub game_id: String,
    pub running: bool,
    /// Over, with no server agent's debrief still to come.
    pub settled: bool,
    pub names: Vec<String>,
    /// Per seat: the account playing it.
    pub users: Vec<Option<UserRef>>,
    pub config: GameConfig,
    pub result: Option<dominion_protocol::view::GameResult>,
    /// Once it is over, per seat: every card the player owns, by name.
    pub decks: Vec<BTreeMap<String, u32>>,
    /// The log as someone watching saw it.
    pub log: Vec<dominion_agent::LogEntry>,
    pub meta: Meta,
}

/// A game that has just ended.
#[derive(Clone, Debug)]
pub struct Ended {
    pub id: String,
    pub meta: Meta,
    /// With its result.
    pub info: GameInfo,
}

/// When the deciding seat of a timed game forfeits. Zero is never.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Timing {
    /// With nobody connected for the seat for this long.
    pub away: Duration,
    /// Without an answer for this long, connected or not.
    pub decision: Duration,
}

impl Default for Timing {
    fn default() -> Timing {
        Timing {
            away: Duration::from_secs(5 * 60),
            decision: Duration::from_secs(30 * 60),
        }
    }
}

pub(crate) struct SeatObservation<'a> {
    pub view: dominion_engine::View<'a>,
    /// The game's chess clock: its rule and how it stands now.
    pub clock: Option<(Clock, ClockView)>,
    pub known: &'a dominion_engine::KnownCards,
    pub names: &'a [String],
    pub decision_id: Option<&'a str>,
    pub deciding: Option<PlayerId>,
    pub version: u64,
    pub undos: u64,
    pub outcome: Option<&'a dominion_engine::Outcome>,
    pub ended: Option<&'a str>,
}

pub(crate) struct PlayObservation {
    pub view: AgentView,
    pub log: Vec<dominion_agent::LogEntry>,
    pub glossary: Vec<dominion_agent::GlossaryEntry>,
    pub undos: u64,
    /// What the seat is told about undos.
    pub note: Option<String>,
    /// The game's chess clock as it stands.
    pub clock: Option<ClockView>,
}

fn current_entry(
    game: &Game,
    mut entry: dominion_agent::GlossaryEntry,
) -> dominion_agent::GlossaryEntry {
    let card = Card::from_name(&entry.name).expect("glossary cards exist");
    if entry.cost.is_some() {
        let cost = game.cost(card);
        entry.cost = Some(dominion_agent::Cost {
            coins: cost.coins.into(),
            potions: cost.potions.into(),
            debt: cost.debt.into(),
        });
    }
    entry.types = game.card_types(card).names().map(String::from).collect();
    entry
}

/// An undo a seat asked for, waiting for other seats to allow it.
#[derive(Clone, Debug)]
struct Request {
    by: PlayerId,
    /// It goes back to just before this answer.
    answer: usize,
    waiting: Vec<PlayerId>,
}

/// A game's undo requests. Not saved: a restart drops the one waiting.
#[derive(Default)]
struct Undoing {
    request: Option<Request>,
    /// Per seat: who refused its last request, until it acts again.
    refused: Vec<Option<PlayerId>>,
}

impl Undoing {
    /// `seat` acted (answered, asked again): its request and refusal are behind it.
    fn acted(&mut self, seat: PlayerId) {
        if self.request.as_ref().is_some_and(|r| r.by == seat) {
            self.request = None;
        }
        if let Some(r) = self.refused.get_mut(seat.index()) {
            *r = None;
        }
    }

    fn waits_for(&self, seat: PlayerId) -> bool {
        self.request
            .as_ref()
            .is_some_and(|r| r.waiting.contains(&seat))
    }
}

/// A seat that only just became the deciding one is given this long to come back.
const AWAY_GRACE_MS: u64 = 30_000;
/// A seat counts as connected this long after its last call.
const CONNECTED_MS: u64 = 20_000;

struct Hosted {
    table: Mutex<Table>,
    tokens: Vec<String>,
    /// Locked after `table`, never before.
    meta: Mutex<Meta>,
    /// Locked after `table` and with `meta` released.
    undo: Mutex<Undoing>,
    /// Per seat: its token has been used since the server started.
    joined: Vec<AtomicBool>,
    /// Per seat: when its token was last used.
    seen: Vec<AtomicU64>,
    /// Per seat: calls waiting on the game for it (event streams, `wait_for_decision`).
    streams: Vec<AtomicU32>,
    watchers: AtomicU32,
    /// Who was connected and how many watched when its followers were last told.
    told: Mutex<(Vec<bool>, u32)>,
    /// When the open decision opened.
    since: AtomicU64,
    /// When the game last changed.
    changed: AtomicU64,
    /// Bumped after every answer, for waiters.
    version: watch::Sender<u64>,
    /// Bumped after every chat message.
    chat: watch::Sender<u64>,
    transcript: watch::Sender<u64>,
}

impl Hosted {
    /// When the game last changed or a seat last used its token. A seat's `seen` counts
    /// once it has joined: it starts at the server's start, for the away forfeit.
    fn last_active(&self) -> u64 {
        (self.seen.iter().zip(&self.joined))
            .filter(|(_, joined)| joined.load(Ordering::Relaxed))
            .map(|(seen, _)| seen.load(Ordering::Relaxed))
            .chain([self.changed.load(Ordering::Relaxed)])
            .max()
            .unwrap_or(0)
    }

    /// Whether a call waits on the game for a seat.
    fn waited_on(&self) -> bool {
        self.streams.iter().any(|s| s.load(Ordering::Relaxed) > 0)
    }

    fn new(table: Table, tokens: Vec<String>, meta: Meta, changed: u64) -> Hosted {
        let now = now_ms();
        let seats = tokens.len();
        Hosted {
            table: Mutex::new(table),
            meta: Mutex::new(meta),
            undo: Mutex::new(Undoing {
                request: None,
                refused: vec![None; seats],
            }),
            joined: (0..seats).map(|_| AtomicBool::new(false)).collect(),
            seen: (0..seats).map(|_| AtomicU64::new(now)).collect(),
            streams: (0..seats).map(|_| AtomicU32::new(0)).collect(),
            watchers: AtomicU32::new(0),
            told: Mutex::new((vec![true; seats], 0)),
            since: AtomicU64::new(now),
            changed: AtomicU64::new(changed),
            tokens,
            version: watch::channel(0).0,
            chat: watch::channel(0).0,
            transcript: watch::channel(0).0,
        }
    }

    fn touch(&self, seat: PlayerId) {
        if let Some(j) = self.joined.get(seat.index()) {
            j.store(true, Ordering::Relaxed);
            self.seen[seat.index()].store(now_ms(), Ordering::Relaxed);
        }
    }

    fn connected(&self, seat: usize, now: u64) -> bool {
        self.streams[seat].load(Ordering::Relaxed) > 0
            || now.saturating_sub(self.seen[seat].load(Ordering::Relaxed)) < CONNECTED_MS
    }

    /// The game's clock as it stands at `now`.
    fn clock(&self, meta: &Meta, now: u64) -> Option<(Clock, ClockView)> {
        let clock = meta.clock.as_ref()?;
        let elapsed = now.saturating_sub(self.since.load(Ordering::Relaxed));
        Some((clock.rule, clock.view(elapsed)))
    }

    /// The seat whose clock has run out at `now`, if one has.
    fn flagged(&self, now: u64) -> Option<PlayerId> {
        let meta = lock(&self.meta);
        let (seat, at) = meta
            .clock
            .as_ref()?
            .flag_at(self.since.load(Ordering::Relaxed))?;
        (at <= now).then_some(seat)
    }

    /// When `deciding` forfeits unless it answers.
    fn forfeit_at(&self, deciding: PlayerId, timing: Timing) -> Option<u64> {
        let since = self.since.load(Ordering::Relaxed);
        let slow = (!timing.decision.is_zero()).then(|| since + timing.decision.as_millis() as u64);
        let seat = deciding.index();
        let away = (!timing.away.is_zero() && self.streams[seat].load(Ordering::Relaxed) == 0)
            .then(|| {
                let left = self.seen[seat].load(Ordering::Relaxed) + timing.away.as_millis() as u64;
                left.max(since + AWAY_GRACE_MS)
            });
        match (slow, away) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    }
}

/// A seat a token claims.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Seat {
    pub game: String,
    pub seat: PlayerId,
}

#[derive(Serialize)]
struct SavedRef<'a> {
    table: &'a Table,
    tokens: &'a [String],
    meta: &'a Meta,
}

#[derive(Deserialize)]
struct Saved {
    table: Table,
    tokens: Vec<String>,
    #[serde(default)]
    meta: SavedMeta,
}

#[derive(Default, Deserialize)]
struct SavedMeta {
    #[serde(flatten)]
    meta: Meta,
    /// What a game saved before [`Meta::undo`] said instead.
    #[serde(default)]
    no_undo: bool,
}

/// What [`Hub::open`] found in its directory.
#[derive(Debug, Default)]
pub struct Loaded {
    pub games: usize,
    /// Finished games older than the retention period, deleted.
    pub expired: usize,
    /// Files that couldn't be read, left in place.
    pub unreadable: Vec<(PathBuf, String)>,
}

#[derive(Default)]
struct Index {
    games: BTreeMap<String, Arc<Hosted>>,
    seats: HashMap<String, Seat>,
    /// The seats each account plays.
    owned: HashMap<String, Vec<Seat>>,
    /// The number of the last game id handed out.
    next: u64,
}

impl Index {
    fn insert(&mut self, id: &str, hosted: Hosted) {
        for (i, token) in hosted.tokens.iter().enumerate() {
            let seat = Seat {
                game: id.into(),
                seat: PlayerId(i as u8),
            };
            if let Some(Some(owner)) = lock(&hosted.meta).owners.get(i) {
                self.owned
                    .entry(owner.clone())
                    .or_default()
                    .push(seat.clone());
            }
            self.seats.insert(token.clone(), seat);
        }
        self.games.insert(id.into(), Arc::new(hosted));
    }

    fn remove(&mut self, id: &str) {
        if let Some(game) = self.games.remove(id) {
            for token in &game.tokens {
                self.seats.remove(token);
            }
            for owner in lock(&game.meta).owners.iter().flatten() {
                if let Some(seats) = self.owned.get_mut(owner) {
                    seats.retain(|s| s.game != id);
                    if seats.is_empty() {
                        self.owned.remove(owner);
                    }
                }
            }
        }
    }
}

type OnEnd = Box<dyn Fn(Ended) + Send + Sync>;
type Users = Box<dyn Fn(&[Option<String>]) -> Vec<Option<UserRef>> + Send + Sync>;

/// How many games read from `expired/` are kept loaded.
const ARCHIVE_CACHE: usize = 16;

/// How many replayed points of finished games are kept to go on from.
const REPLAY_CACHE: usize = 8;

/// The most games in progress unless [`Hub::set_max_games`] says otherwise.
pub const DEFAULT_MAX_GAMES: usize = 1000;

pub struct Hub {
    index: Mutex<Index>,
    /// Games read from `expired/` lately, most recent first: never hosted again.
    archive: Mutex<VecDeque<(String, Arc<Hosted>)>>,
    /// Points of finished games replayed lately, most recent first.
    replays: Mutex<VecDeque<(String, Table)>>,
    store: Option<Arc<Store>>,
    timing: Mutex<Timing>,
    max_games: AtomicU64,
    /// Called, with no lock held, when a game ends.
    on_end: OnceLock<OnEnd>,
    /// Account ids to players, called with no lock held.
    users: OnceLock<Users>,
}

impl Default for Hub {
    fn default() -> Hub {
        Hub {
            index: Mutex::default(),
            archive: Mutex::default(),
            replays: Mutex::default(),
            store: None,
            timing: Mutex::default(),
            max_games: AtomicU64::new(DEFAULT_MAX_GAMES as u64),
            on_end: OnceLock::new(),
            users: OnceLock::new(),
        }
    }
}

/// Counts a call waiting on a game for a seat, until dropped.
pub struct Connection {
    game: Arc<Hosted>,
    seat: usize,
}

impl Drop for Connection {
    fn drop(&mut self) {
        self.game.seen[self.seat].store(now_ms(), Ordering::Relaxed);
        self.game.streams[self.seat].fetch_sub(1, Ordering::Relaxed);
    }
}

/// Counts someone watching a game, until dropped.
pub struct Watching(Arc<Hosted>);

impl Drop for Watching {
    fn drop(&mut self) {
        self.0.watchers.fetch_sub(1, Ordering::Relaxed);
    }
}

/// A game's change counters: its version, and its chat's.
pub struct Feed {
    pub version: watch::Receiver<u64>,
    pub chat: watch::Receiver<u64>,
}

fn bad(message: String) -> HubError {
    HubError::BadRequest(message)
}

fn card(name: &str, what: &str, want: impl Fn(Card) -> bool) -> Result<Card, HubError> {
    Card::from_name(name)
        .filter(|&c| want(c))
        .ok_or_else(|| bad(format!("{name:?} is not a {what}")))
}

/// The sets a random kingdom can be dealt from: the ones with Kingdom cards.
fn dealable_sets() -> Vec<Set> {
    let with_kingdom: BTreeSet<Set> = Card::ALL
        .iter()
        .filter(|c| c.def().kingdom && !c.def().removed)
        .map(|c| c.set())
        .collect();
    Set::ALL
        .iter()
        .copied()
        .filter(|s| with_kingdom.contains(s))
        .collect()
}

fn set(name: &str) -> Result<Set, HubError> {
    let key = |s: &str| s.to_lowercase().replace(['&', ' ', '-', '_'], "");
    let sets = dealable_sets();
    sets.iter()
        .copied()
        .find(|s| key(s.name()) == key(name))
        .ok_or_else(|| {
            let names: Vec<&str> = sets.iter().map(|s| s.name()).collect();
            bad(format!(
                "{name:?} is not a set a kingdom can be dealt from; those are {}",
                names.join(", ")
            ))
        })
}

/// The longest brief, in bytes.
pub const MAX_BRIEF: usize = 64 * 1024;
/// The longest debrief question, in bytes.
pub const MAX_DEBRIEF_QUESTION: usize = 4 * 1024;
/// The longest debrief, in bytes.
pub const MAX_DEBRIEF: usize = 64 * 1024;
/// How long after the game a server agent has for its debrief before it leaves anyway.
pub const DEBRIEF_WAIT: Duration = Duration::from_secs(15 * 60);

/// `req`'s briefs and debrief question, checked, into `meta`.
pub fn brief(req: &NewGame, meta: &mut Meta) -> Result<(), HubError> {
    if !req.briefs.is_empty() && req.briefs.len() != usize::from(req.players) {
        return Err(bad(
            "briefs must have one entry per player (null for none)".into()
        ));
    }
    if req.briefs.iter().flatten().any(|b| b.len() > MAX_BRIEF) {
        return Err(bad(format!("a brief is at most {MAX_BRIEF} bytes")));
    }
    let question = req.debrief.as_deref().map(str::trim);
    if question.is_some_and(str::is_empty) {
        return Err(bad("the debrief question is empty".into()));
    }
    if question.is_some_and(|q| q.len() > MAX_DEBRIEF_QUESTION) {
        return Err(bad(format!(
            "the debrief question is at most {MAX_DEBRIEF_QUESTION} bytes"
        )));
    }
    meta.briefs = (req.briefs.iter())
        .map(|b| {
            b.as_deref()
                .map(str::trim)
                .filter(|b| !b.is_empty())
                .map(String::from)
        })
        .collect();
    meta.debrief = question.map(String::from);
    Ok(())
}

pub fn config(req: &NewGame) -> Result<GameConfig, HubError> {
    let seed = req.seed.unwrap_or_else(|| rand::rng().random());
    let sets: Vec<Set> = req.sets.iter().map(|n| set(n)).collect::<Result<_, _>>()?;
    let kingdom: Vec<Card> = req
        .kingdom
        .iter()
        .map(|n| {
            card(n, "Kingdom pile", |c| {
                c.def().kingdom || c.def().unimplemented
            })
        })
        .collect::<Result<_, _>>()?;
    let landscapes: Vec<Card> = req
        .landscapes
        .iter()
        .map(|n| card(n, "landscape", |c| c.kind().is_landscape()))
        .collect::<Result<_, _>>()?;
    let mut config = if kingdom.is_empty() {
        let sets = if sets.is_empty() {
            dealable_sets()
        } else {
            sets
        };
        let mut config = GameConfig::random(req.players, &sets, seed);
        if !landscapes.is_empty() {
            config.landscapes = landscapes;
        }
        config
    } else {
        if !sets.is_empty() {
            return Err(bad(
                "`sets` deals a random kingdom: give it or `kingdom`, not both".into(),
            ));
        }
        let mut config = GameConfig::new(req.players, kingdom, seed);
        config.landscapes = landscapes;
        config
    };
    config.turn_limit = req.turn_limit;
    config.colonies = req.colonies;
    config.shelters = req.shelters;
    for (by, chosen) in &req.setup {
        let by = card(by, "card", |_| true)?;
        let chosen = chosen
            .iter()
            .map(|n| card(n, "card", |_| true))
            .collect::<Result<_, _>>()?;
        config.setup_choices.insert(by, chosen);
    }
    Ok(config)
}

/// The game `req`'s rules deal, and the seed that dealt it.
pub fn deal_config(req: &DealRequest) -> Result<(GameConfig, u64), HubError> {
    let seed = req.seed.unwrap_or_else(|| rand::rng().random());
    let cards = |names: &[String], what: &str, want: fn(Card) -> bool| {
        names
            .iter()
            .map(|n| card(n, what, want))
            .collect::<Result<Vec<_>, _>>()
    };
    let sets = |names: &[String]| names.iter().map(|n| set(n)).collect::<Result<Vec<_>, _>>();
    let slots = match &req.slots {
        None => None,
        Some(slots) => Some(
            slots
                .iter()
                .map(|s| {
                    let mut kinds = 0u8;
                    for k in &s.kinds {
                        let i = SLOT_KINDS
                            .iter()
                            .position(|kind| format!("{kind:?}").eq_ignore_ascii_case(k))
                            .ok_or_else(|| {
                                bad(format!(
                                    "{k:?} is not a landscape slot kind; those are event, \
                                     landmark, project, way and trait"
                                ))
                            })?;
                        kinds |= 1 << i;
                    }
                    Ok(LandscapeSlot {
                        kinds,
                        optional: s.optional,
                    })
                })
                .collect::<Result<Vec<_>, HubError>>()?,
        ),
    };
    let mut rules = DealRules {
        sets: sets(&req.sets)?,
        kingdom: cards(&req.kingdom, "Kingdom pile", |c| c.def().kingdom)?,
        from_sets: sets(&req.from_sets)?,
        landscapes: cards(&req.landscapes, "landscape", |c| c.kind().is_landscape())?,
        slots,
        banned: cards(&req.banned, "card", |_| true)?,
    };
    if rules.sets.is_empty() {
        rules.sets = dealable_sets();
    }
    let config = GameConfig::deal(req.players, &rules, seed);
    Ok((config, seed))
}

/// The kingdom `req`'s rules deal, checked by setting a game up with it.
pub fn deal(req: &DealRequest) -> Result<Dealt, HubError> {
    let (config, seed) = deal_config(req)?;
    let dealt = Dealt {
        kingdom: config
            .kingdom
            .iter()
            .map(|c| c.name().to_string())
            .collect(),
        landscapes: config
            .landscapes
            .iter()
            .map(|c| c.name().to_string())
            .collect(),
        seed,
    };
    engine(|| Game::new(config))?.map_err(|e| bad(e.to_string()))?;
    Ok(dealt)
}

pub const MAX_NAME_CHARS: usize = 32;

/// A seat's name as given, trimmed, or what is wrong with it ("is empty"). Names go into
/// every client's text, so each must be one printable line.
pub fn seat_name(given: &str) -> Result<String, String> {
    let name = given.trim();
    if name.is_empty() {
        return Err("is empty".into());
    }
    if name.chars().count() > MAX_NAME_CHARS {
        return Err(format!("is longer than {MAX_NAME_CHARS} characters"));
    }
    if name
        .chars()
        .any(|c| c.is_control() || matches!(c, '\u{2028}' | '\u{2029}'))
    {
        return Err("has a line break or control character".into());
    }
    Ok(name.to_string())
}

/// Every seat's name: the ones given, then the defaults. Seats must be told apart by
/// them.
pub(crate) fn names(req: &NewGame) -> Result<Vec<String>, HubError> {
    let players = usize::from(req.players);
    if req.names.len() > players {
        return Err(bad(format!(
            "{} names for {players} players",
            req.names.len()
        )));
    }
    let mut names: Vec<String> = Vec::with_capacity(players);
    for i in 0..players {
        let name = match req.names.get(i) {
            None => PlayerId(i as u8).to_string(),
            Some(given) => seat_name(given).map_err(|e| bad(format!("seat {i}'s name {e}")))?,
        };
        if names
            .iter()
            .any(|n| n.to_lowercase() == name.to_lowercase())
        {
            return Err(bad(format!(
                "two seats are named {name:?}; seat names must differ"
            )));
        }
        names.push(name);
    }
    Ok(names)
}

fn token() -> String {
    format!("{:032x}", rand::rng().random::<u128>())
}

/// The number in a game id (`g<number>`).
fn id_number(id: &str) -> Option<u64> {
    id.strip_prefix('g')?.parse().ok()
}

fn load(path: &Path, stem: &str) -> Result<(Table, Vec<String>, Meta), String> {
    let bytes = fs::read(path).map_err(|e| e.to_string())?;
    let saved: Saved = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    let mut meta = saved.meta.meta;
    if saved.meta.no_undo {
        meta.undo = UndoRule::Off;
    }
    let saved = (saved.table, saved.tokens, meta);
    let (table, tokens, mut meta) = saved;
    if table.id() != stem {
        return Err(format!("holds game {}, not {stem}", table.id()));
    }
    let seats = table.names().len();
    if tokens.len() != seats {
        return Err(format!(
            "has {} seat tokens for {seats} seats",
            tokens.len()
        ));
    }
    meta.owners.resize(seats, None);
    meta.agents.resize(seats, false);
    Ok((table, tokens, meta))
}

fn result_summary(table: &Table, meta: &Meta, joined: Vec<bool>) -> GameSummary {
    GameSummary {
        game_id: table.id().into(),
        names: table.names().to_vec(),
        running: !table.is_over(),
        deciding: table.deciding().map(|p| p.0),
        joined,
        kingdom: table
            .game()
            .config()
            .kingdom
            .iter()
            .map(|c| c.name().to_string())
            .collect(),
        landscapes: table
            .game()
            .landscapes()
            .iter()
            .map(|c| c.name().to_string())
            .collect(),
        source: meta.source,
        rated: meta.rated,
        watchable: meta.watchable && table.watchable(),
        users: vec![],
    }
}

impl Hub {
    /// A hub that keeps its games in `dir` and resumes the ones saved there. Finished
    /// games whose last change is older than `keep_finished` are moved to `expired/`.
    /// Fails if `dir` can't be written to or another server has it open.
    pub fn open(dir: &Path, keep_finished: Duration) -> io::Result<(Hub, Loaded)> {
        let store = Store::open(dir)?;
        let mut loaded = Loaded::default();
        let mut index = Index::default();
        // An expired game keeps its id, so no new game is saved over its file.
        match fs::read_dir(store.expired_dir()) {
            Ok(entries) => {
                for entry in entries {
                    let path = entry?.path();
                    let stem = path.file_stem().and_then(|n| n.to_str()).unwrap_or("");
                    index.next = index.next.max(id_number(stem).unwrap_or(0));
                }
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
        for entry in fs::read_dir(dir)? {
            let path = entry?.path();
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if name.starts_with('.') {
                // A write the last server didn't finish; the game's file is intact.
                if name.ends_with(".json.tmp") {
                    fs::remove_file(&path)?;
                }
                continue;
            }
            let Some(stem) = name.strip_suffix(".json") else {
                continue;
            };
            // An unreadable file keeps its id too, so no new game overwrites it.
            index.next = index.next.max(id_number(stem).unwrap_or(0));
            let (table, tokens, meta) = match load(&path, stem) {
                Ok(saved) => saved,
                Err(e) => {
                    loaded.unreadable.push((path, e));
                    continue;
                }
            };
            let modified = fs::metadata(&path).and_then(|m| m.modified()).ok();
            let age = modified
                .and_then(|t| SystemTime::now().duration_since(t).ok())
                .unwrap_or_default();
            if table.is_over() && age >= keep_finished {
                store.retire(stem)?;
                loaded.expired += 1;
                continue;
            }
            let changed = now_ms().saturating_sub(age.as_millis() as u64);
            index.insert(stem, Hosted::new(table, tokens, meta, changed));
        }
        loaded.games = index.games.len();
        let hub = Hub {
            index: Mutex::new(index),
            store: Some(Arc::new(store)),
            ..Hub::default()
        };
        Ok((hub, loaded))
    }

    /// The directory the games are kept in, for what else the server keeps.
    pub fn store(&self) -> Option<Arc<Store>> {
        self.store.clone()
    }

    /// No game id at or under `number` is handed out from now on.
    pub fn reserve_ids(&self, number: u64) {
        let mut index = lock(&self.index);
        index.next = index.next.max(number);
    }

    pub fn set_timing(&self, timing: Timing) {
        *lock(&self.timing) = timing;
    }

    pub fn set_max_games(&self, max: usize) {
        self.max_games.store(max as u64, Ordering::Relaxed);
    }

    /// `on_end` is called whenever a game ends, `users` to name the accounts playing a
    /// game's seats. Neither may be set twice.
    pub fn set_hooks(
        &self,
        on_end: impl Fn(Ended) + Send + Sync + 'static,
        users: impl Fn(&[Option<String>]) -> Vec<Option<UserRef>> + Send + Sync + 'static,
    ) {
        let _ = self.on_end.set(Box::new(on_end));
        let _ = self.users.set(Box::new(users));
    }

    fn users(&self, owners: &[Option<String>]) -> Vec<Option<UserRef>> {
        match self.users.get() {
            Some(users) if owners.iter().any(Option::is_some) => users(owners),
            _ => vec![None; owners.len()],
        }
    }

    fn save(&self, game: &Hosted, table: &Table) {
        game.changed.store(now_ms(), Ordering::Relaxed);
        let meta = lock(&game.meta);
        let saved = SavedRef {
            table,
            tokens: &game.tokens,
            meta: &meta,
        };
        if let Some(store) = &self.store
            && let Err(e) = store.save(table.id(), &saved)
        {
            eprintln!("dominion: saving game {}: {e}", table.id());
        }
    }

    pub fn create(&self, req: &NewGame) -> Result<Created, HubError> {
        if req.agents.iter().any(Option::is_some) {
            return Err(bad("agent seats must be assigned through the arena".into()));
        }
        let config = config(req)?;
        let names = names(req)?;
        let mut meta = Meta {
            watchable: req.watchable.unwrap_or(false),
            clock: req.clock.map(ClockState::new),
            ..Meta::default()
        };
        brief(req, &mut meta)?;
        self.host(config, names, meta)
    }

    /// Starts a game of `config` with a seat per name.
    pub fn host(
        &self,
        config: GameConfig,
        names: Vec<String>,
        mut meta: Meta,
    ) -> Result<Created, HubError> {
        let id = {
            let mut index = lock(&self.index);
            let max = self.max_games.load(Ordering::Relaxed) as usize;
            if index.games.len() >= max
                && index
                    .games
                    .values()
                    .filter(|g| !lock(&g.table).is_over())
                    .count()
                    >= max
            {
                return Err(HubError::Busy(format!(
                    "this server is hosting {max} games, as many as it will; try again later"
                )));
            }
            index.next += 1;
            format!("g{}", index.next)
        };
        let table = engine(|| Table::new(&id, config, names))?.map_err(|e| bad(e.to_string()))?;
        let seats = table.names().len();
        meta.owners.resize(seats, None);
        meta.agents.resize(seats, false);
        meta.briefs.resize(seats, None);
        meta.debriefs = vec![None; seats];
        meta.started = now_ms();
        if let Some(clock) = &mut meta.clock {
            clock.rule.check().map_err(bad)?;
            clock.advance(&table, 0);
        }
        let tokens: Vec<String> = (0..seats).map(|_| token()).collect();
        let created = Created {
            game_id: id.clone(),
            seats: tokens
                .iter()
                .enumerate()
                .map(|(i, t)| SeatToken {
                    seat: i as u8,
                    name: table.names()[i].clone(),
                    token: t.clone(),
                })
                .collect(),
        };
        let hosted = Hosted::new(table, tokens, meta, now_ms());
        self.save(&hosted, &lock(&hosted.table));
        lock(&self.index).insert(&id, hosted);
        Ok(created)
    }

    fn game(&self, id: &str) -> Result<Arc<Hosted>, HubError> {
        lock(&self.index)
            .games
            .get(id)
            .cloned()
            .ok_or_else(|| HubError::UnknownGame(id.into()))
    }

    /// Game `id` to look at: the hosted one, else the one kept in `expired/`, which
    /// nothing changes.
    fn read(&self, id: &str) -> Result<Arc<Hosted>, HubError> {
        self.game(id).or_else(|e| self.archived(id).ok_or(e))
    }

    fn archived(&self, id: &str) -> Option<Arc<Hosted>> {
        let store = self.store.as_ref()?;
        // The id names a file: nothing but `g<number>`.
        if id_number(id).is_none_or(|n| format!("g{n}") != id) {
            return None;
        }
        let mut archive = lock(&self.archive);
        if let Some(i) = archive.iter().position(|(k, _)| k == id) {
            let entry = archive.remove(i)?;
            let game = entry.1.clone();
            archive.push_front(entry);
            return Some(game);
        }
        let path = store.expired_dir().join(format!("{id}.json"));
        let modified = fs::metadata(&path).and_then(|m| m.modified()).ok()?;
        let (table, tokens, meta) = load(&path, id)
            .inspect_err(|e| eprintln!("dominion: reading {}: {e}", path.display()))
            .ok()?;
        let changed = modified
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_millis() as u64);
        let game = Arc::new(Hosted::new(table, tokens, meta, changed));
        archive.push_front((id.into(), game.clone()));
        archive.truncate(ARCHIVE_CACHE);
        Some(game)
    }

    pub fn configuration(&self, id: &str) -> Option<GameConfig> {
        self.game(id)
            .ok()
            .map(|game| lock(&game.table).game().config().clone())
    }

    /// The seat `token` claims in game `id`.
    pub fn claim(&self, id: &str, token: &str) -> Result<Seat, HubError> {
        let index = lock(&self.index);
        let Some(game) = index.games.get(id) else {
            drop(index);
            let game = self.archived(id).ok_or(HubError::UnknownGame(id.into()))?;
            let seat = (game.tokens.iter().position(|t| t == token))
                .ok_or_else(|| HubError::BadToken(id.into()))?;
            return Ok(Seat {
                game: id.into(),
                seat: PlayerId(seat as u8),
            });
        };
        let seat = index
            .seats
            .get(token)
            .filter(|seat| seat.game == id)
            .cloned()
            .ok_or_else(|| HubError::BadToken(id.into()))?;
        game.touch(seat.seat);
        Ok(seat)
    }

    /// The seat `token` claims in whichever game it belongs to.
    pub fn find(&self, token: &str) -> Result<Seat, HubError> {
        let index = lock(&self.index);
        let seat = index
            .seats
            .get(token)
            .cloned()
            .ok_or_else(|| bad("unknown seat token".into()))?;
        if let Some(game) = index.games.get(&seat.game) {
            game.touch(seat.seat);
        }
        Ok(seat)
    }

    /// Notes that `seat` is being played now.
    pub fn touch(&self, seat: &Seat) {
        if let Ok(game) = self.game(&seat.game) {
            game.touch(seat.seat);
        }
    }

    /// The seats account `user` plays in game `id`.
    pub fn owned_in(&self, id: &str, user: &str) -> Vec<Seat> {
        let index = lock(&self.index);
        if index.games.contains_key(id) {
            let seats = index.owned.get(user).into_iter().flatten();
            return seats.filter(|s| s.game == id).cloned().collect();
        }
        drop(index);
        let Some(game) = self.archived(id) else {
            return vec![];
        };
        let meta = lock(&game.meta);
        (meta.owners.iter().enumerate())
            .filter(|(_, owner)| owner.as_deref() == Some(user))
            .map(|(i, _)| Seat {
                game: id.into(),
                seat: PlayerId(i as u8),
            })
            .collect()
    }

    /// The seats account `user` plays: in games still running, and in games that ended
    /// less than `recent` ago.
    pub fn seats_of(&self, user: &str, recent: Duration) -> Vec<MyGame> {
        let seats: Vec<(Seat, Arc<Hosted>)> = {
            let index = lock(&self.index);
            let seats = index.owned.get(user).into_iter().flatten();
            seats
                .filter_map(|s| Some((s.clone(), index.games.get(&s.game)?.clone())))
                .collect()
        };
        let now = now_ms();
        seats
            .into_iter()
            .filter_map(|(seat, game)| {
                let table = lock(&game.table);
                let running = !table.is_over();
                let age = now.saturating_sub(game.changed.load(Ordering::Relaxed));
                (running || u128::from(age) < recent.as_millis()).then(|| MyGame {
                    game_id: seat.game.clone(),
                    seat: seat.seat.0,
                    name: table.names()[seat.seat.index()].clone(),
                    token: game.tokens[seat.seat.index()].clone(),
                    running,
                    opponents: (table.names().iter().enumerate())
                        .filter(|&(i, _)| i != seat.seat.index())
                        .map(|(_, n)| n.clone())
                        .collect(),
                })
            })
            .collect()
    }

    /// Whether account `user` plays in a timed or clocked game that is still running.
    pub fn in_timed_game(&self, user: &str) -> bool {
        let games: Vec<Arc<Hosted>> = {
            let index = lock(&self.index);
            let seats = index.owned.get(user).into_iter().flatten();
            seats
                .filter_map(|s| index.games.get(&s.game).cloned())
                .collect()
        };
        games.iter().any(|g| {
            !lock(&g.table).is_over() && {
                let meta = lock(&g.meta);
                meta.timed || meta.clock.is_some()
            }
        })
    }

    pub fn list(&self) -> Vec<GameSummary> {
        let games: Vec<Arc<Hosted>> = lock(&self.index).games.values().cloned().collect();
        let listed: Vec<(GameSummary, Vec<Option<String>>)> = games
            .iter()
            .map(|g| {
                let t = lock(&g.table);
                let meta = lock(&g.meta);
                let joined = g.joined.iter().map(|j| j.load(Ordering::Relaxed)).collect();
                (result_summary(&t, &meta, joined), meta.owners.clone())
            })
            .collect();
        listed
            .into_iter()
            .map(|(mut summary, owners)| {
                summary.users = self.users(&owners);
                summary
            })
            .collect()
    }

    /// The brief `seat`'s player is shown on joining: none for a server agent's seat,
    /// which gets its brief from the runner.
    pub fn brief(&self, seat: &Seat) -> Option<String> {
        let game = self.read(&seat.game).ok()?;
        let meta = lock(&game.meta);
        if meta.agents.get(seat.seat.index()).copied().unwrap_or(false) {
            return None;
        }
        meta.briefs.get(seat.seat.index()).cloned().flatten()
    }

    /// Whether the server agent at `seat` still owes the debrief of its finished game.
    pub fn debrief_pending(&self, id: &str, seat: u8) -> bool {
        let Ok(game) = self.game(id) else {
            return false;
        };
        let over = lock(&game.table).is_over();
        let meta = lock(&game.meta);
        over && debrief_owed(&meta, usize::from(seat), now_ms())
    }

    /// Records `seat`'s debrief of its finished game, in place of any earlier one.
    pub fn submit_debrief(&self, seat: &Seat, text: &str) -> Result<(), HubError> {
        let game = self.game(&seat.game)?;
        let table = lock(&game.table);
        if !table.is_over() {
            return Err(HubError::BadRequest(
                "a debrief comes once the game is over".into(),
            ));
        }
        let text = text.trim();
        if text.is_empty() {
            return Err(HubError::BadRequest("the debrief is empty".into()));
        }
        let mut end = text.len().min(MAX_DEBRIEF);
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        {
            let mut meta = lock(&game.meta);
            let seats = table.names().len();
            meta.debriefs.resize(seats, None);
            meta.debriefs[seat.seat.index()] = Some(text[..end].to_string());
        }
        self.save(&game, &table);
        game.version.send_modify(|v| *v += 1);
        Ok(())
    }

    /// Game `id` as a record: its setup, its players and how it went, the public log, and
    /// once it is over the decks, briefs and debriefs. `settled` is whether it is over
    /// with no server agent's debrief still to come.
    pub fn report(&self, id: &str) -> Result<Report, HubError> {
        let game = self.read(id)?;
        let (mut report, owners) = {
            let table = lock(&game.table);
            let meta = lock(&game.meta).clone();
            let over = table.is_over();
            let g = table.game();
            let decks = if over {
                (0..g.num_players())
                    .map(|p| {
                        let mut deck: BTreeMap<String, u32> = BTreeMap::new();
                        for id in g.player(PlayerId(p as u8)).owned() {
                            *deck.entry(g.card(id).name().to_string()).or_default() += 1;
                        }
                        deck
                    })
                    .collect()
            } else {
                vec![]
            };
            let view = engine(|| table.watch_view(0))?;
            let now = now_ms();
            let settled =
                over && (0..table.names().len()).all(|seat| !debrief_owed(&meta, seat, now));
            let report = Report {
                game_id: id.into(),
                running: !over,
                settled,
                names: table.names().to_vec(),
                users: vec![],
                config: g.config().clone(),
                result: view.as_ref().and_then(|v| v.game.result.clone()),
                decks,
                log: view.map(|v| v.since_last_decision).unwrap_or_default(),
                meta,
            };
            let owners = report.meta.owners.clone();
            (report, owners)
        };
        report.users = self.users(&owners);
        Ok(report)
    }

    /// Waits until game `id` changes or `timeout` passes.
    pub async fn changed_or(&self, id: &str, version: u64, timeout: Duration) {
        let Ok(game) = self.read(id) else { return };
        let mut changes = game.version.subscribe();
        if *changes.borrow_and_update() != version {
            return;
        }
        let _ = tokio::time::timeout(timeout, changes.changed()).await;
    }

    /// Game `id`'s version, which every change bumps.
    pub fn version(&self, id: &str) -> Result<u64, HubError> {
        Ok(*self.read(id)?.version.borrow())
    }

    pub fn meta(&self, id: &str) -> Result<Meta, HubError> {
        Ok(lock(&self.read(id)?.meta).clone())
    }

    /// `seat`'s view, its text rendering, and what it is told about undos
    /// ([`Hub::undo_note`]), read together.
    pub fn view(&self, seat: &Seat) -> Result<(AgentView, String, Option<String>), HubError> {
        let game = self.read(&seat.game)?;
        let table = lock(&game.table);
        let note = self.undo_note(&game, &table, seat.seat);
        engine(|| {
            let view = table.view(seat.seat);
            let text = dominion_agent::text::render(&view);
            (view, text, note)
        })
    }

    /// What `seat` is told about undos: what others took back since it last answered
    /// (`Table::undo_note`), and the request waiting, its own or one it must answer.
    fn undo_note(&self, game: &Hosted, table: &Table, seat: PlayerId) -> Option<String> {
        let undo = lock(&game.undo);
        let name = |p: PlayerId| table.names()[p.index()].clone();
        let mut lines: Vec<String> = table.undo_note(seat).into_iter().collect();
        if let Some(r) = &undo.request {
            let answers =
                dominion_engine::amount((table.answers() - r.answer) as i64, "answer", "answers");
            if r.by == seat {
                let waiting: Vec<String> = r.waiting.iter().map(|&p| name(p)).collect();
                lines.push(format!(
                    "UNDO you asked to take back {answers}; waiting for {} to allow it \
                     (cancel_undo withdraws it).",
                    waiting.join(" and ")
                ));
            } else if r.waiting.contains(&seat) {
                lines.push(format!(
                    "UNDO {} asks to take back {answers}: call answer_undo with allow true \
                     to let them, or false to refuse.",
                    name(r.by)
                ));
            }
        }
        if let Some(Some(by)) = undo.refused.get(seat.index()) {
            lines.push(format!("UNDO {} refused your undo.", name(*by)));
        }
        (!lines.is_empty()).then(|| lines.join("\n"))
    }

    /// The undo rule and the request waiting, as `seat` (or someone watching) sees it.
    fn undo_state(&self, game: &Hosted, table: &Table, seat: Option<PlayerId>) -> UndoState {
        let rule = lock(&game.meta).undo;
        let undo = lock(&game.undo);
        UndoState {
            rule,
            request: undo.request.as_ref().map(|r| UndoRequest {
                by: r.by.0,
                answers: (table.answers() - r.answer) as u32,
                waiting: r.waiting.iter().map(|p| p.0).collect(),
            }),
            refused_by: seat
                .and_then(|s| undo.refused.get(s.index()).copied().flatten())
                .map(|p| p.0),
        }
    }

    pub(crate) fn project_seat<T>(
        &self,
        seat: &Seat,
        project: impl for<'a> FnOnce(SeatObservation<'a>) -> T,
    ) -> Result<T, HubError> {
        let game = self.read(&seat.game)?;
        let table = lock(&game.table);
        if seat.seat.index() >= table.names().len() {
            return Err(HubError::Forbidden("invalid seat".into()));
        }
        let meta = lock(&game.meta);
        engine(|| {
            let known = table.knowledge().known(table.game(), seat.seat);
            let decision = table.decision_id();
            project(SeatObservation {
                view: table.game().view(seat.seat),
                clock: game.clock(&meta, now_ms()),
                known: &known,
                names: table.names(),
                decision_id: decision.as_deref(),
                deciding: table.deciding(),
                version: *game.version.borrow(),
                undos: table.undos(),
                outcome: table.game().outcome(),
                ended: meta.ended.as_deref(),
            })
        })
    }

    pub(crate) fn admission<T>(
        &self,
        id: &str,
        check: impl FnOnce(&GameConfig, &Meta) -> T,
    ) -> Result<T, HubError> {
        let game = self.game(id)?;
        let table = lock(&game.table);
        let meta = lock(&game.meta);
        engine(|| check(table.game().config(), &meta))
    }

    /// The common MCP presentation reads the seat's actual view and complete visible
    /// log together; its log cursor must not change `sinceLastDecision` in the view.
    pub(crate) fn play_observation(&self, seat: &Seat) -> Result<PlayObservation, HubError> {
        let game = self.read(&seat.game)?;
        let table = lock(&game.table);
        engine(|| {
            let view = table.view(seat.seat);
            let glossary = dominion_agent::glossary_for(&view)
                .into_iter()
                .map(|entry| current_entry(table.game(), entry))
                .collect();
            PlayObservation {
                view,
                log: table.view_from(seat.seat, 0).since_last_decision,
                glossary,
                undos: table.undos(),
                note: None,
                clock: None,
            }
        })
        .map(|mut observation| {
            observation.note = self.undo_note(&game, &table, seat.seat);
            observation.clock = game.clock(&lock(&game.meta), now_ms()).map(|(_, c)| c);
            observation
        })
    }

    pub(crate) fn card_entries(
        &self,
        seat: Option<&Seat>,
        names: &[String],
    ) -> Result<Vec<dominion_agent::GlossaryEntry>, HubError> {
        let entries: Vec<_> = names
            .iter()
            .map(|name| {
                dominion_agent::glossary(name)
                    .ok_or_else(|| bad(format!("no card is named {name:?}")))
            })
            .collect::<Result<_, _>>()?;
        let Some(seat) = seat else { return Ok(entries) };
        let game = self.read(&seat.game)?;
        let table = lock(&game.table);
        engine(|| {
            entries
                .into_iter()
                .map(|entry| current_entry(table.game(), entry))
                .collect()
        })
    }

    /// Who plays `game`, who is connected and what kind of game it is; the accounts are
    /// left for [`Hub::users`], which no lock may be held for.
    fn game_meta(&self, game: &Hosted, table: &Table) -> (GameMeta, Vec<Option<String>>) {
        let meta = lock(&game.meta);
        let now = now_ms();
        let forfeit_at = match table.deciding() {
            Some(deciding) if meta.timed => game.forfeit_at(deciding, *lock(&self.timing)),
            _ => None,
        };
        let shown = GameMeta {
            source: meta.source,
            rated: meta.rated,
            watchable: meta.watchable && table.watchable(),
            timed: meta.timed,
            users: vec![],
            connected: (0..game.tokens.len())
                .map(|s| game.connected(s, now))
                .collect(),
            forfeit_at,
            clock: game.clock(&meta, now).map(|(_, view)| view),
            watchers: game.watchers.load(Ordering::Relaxed),
            ended: meta.ended.clone(),
            now,
        };
        (shown, meta.owners.clone())
    }

    /// `seat`'s view, with the log from `log_from` on if given, the game's version and
    /// the seat deciding, all read together.
    pub fn seat_view(&self, seat: &Seat, log_from: Option<usize>) -> Result<SeatView, HubError> {
        let game = self.read(&seat.game)?;
        let (mut seat_view, owners) = {
            let table = lock(&game.table);
            let view = engine(|| match log_from {
                Some(from) => table.view_from(seat.seat, from),
                None => table.view(seat.seat),
            })?;
            let extras = engine(|| table.extras(seat.seat))?;
            let (meta, owners) = self.game_meta(&game, &table);
            let seat_view = SeatView {
                view,
                version: *game.version.borrow(),
                deciding: table.deciding().map(|p| p.0),
                table: extras,
                undos: table.undos(),
                can_undo: self.can_undo(&game, &table, seat.seat),
                undo: self.undo_state(&game, &table, Some(seat.seat)),
                meta,
                replay: None,
            };
            (seat_view, owners)
        };
        seat_view.meta.users = self.users(&owners);
        Ok(seat_view)
    }

    /// Game `id` as someone watching sees it, with the log from `log_from` on: every
    /// player's cards, or with `as_seat` only what that seat knows, and no decision.
    pub fn watch_view(
        &self,
        id: &str,
        log_from: usize,
        as_seat: Option<u8>,
    ) -> Result<SeatView, HubError> {
        self.watched(id, None, log_from, as_seat)
    }

    /// [`Hub::watch_view`] of finished game `id` as it was after `step` answers.
    pub fn replay_view(
        &self,
        id: &str,
        step: usize,
        log_from: usize,
        as_seat: Option<u8>,
    ) -> Result<SeatView, HubError> {
        self.watched(id, Some(step), log_from, as_seat)
    }

    fn watched(
        &self,
        id: &str,
        step: Option<usize>,
        log_from: usize,
        as_seat: Option<u8>,
    ) -> Result<SeatView, HubError> {
        let game = self.read(id)?;
        let refused = || HubError::Forbidden(format!("game {id} can't be watched"));
        let (mut seat_view, owners) = {
            let real = lock(&game.table);
            if !lock(&game.meta).watchable {
                return Err(refused());
            }
            if let Some(seat) = as_seat
                && usize::from(seat) >= real.names().len()
            {
                return Err(HubError::BadRequest(format!("there is no seat {seat}")));
            }
            let replay = match step {
                None => None,
                Some(_) if !real.is_over() => {
                    return Err(HubError::Conflict(format!(
                        "game {id} is still being played: only a finished game can be replayed"
                    )));
                }
                Some(_) if !real.replayable() => {
                    return Err(HubError::Conflict(format!(
                        "game {id} was saved before games kept their answers, so it can't be \
                         replayed"
                    )));
                }
                Some(step) if step > real.answers() => {
                    return Err(HubError::BadRequest(format!(
                        "game {id} had {} answers",
                        real.answers()
                    )));
                }
                Some(step) => Some(Replay {
                    step,
                    steps: real.answers(),
                    turns: (real.turn_starts().into_iter())
                        .map(|(step, player, turn)| ReplayTurn {
                            step,
                            player: player.0,
                            turn,
                        })
                        .collect(),
                }),
            };
            let replayed = match &replay {
                Some(r) if r.step < r.steps => Some(self.replayed(id, &real, r.step)?),
                _ => None,
            };
            let table = replayed.as_ref().unwrap_or(&real);
            let (view, extras) = match as_seat {
                None => (
                    engine(|| table.watch_view(log_from))?.ok_or_else(refused)?,
                    engine(|| table.watch_extras())?.unwrap_or_default(),
                ),
                Some(seat) => {
                    let seat = PlayerId(seat);
                    let mut view = engine(|| table.view_from(seat, log_from))?;
                    view.decision = None;
                    view.spectating = true;
                    (view, engine(|| table.extras(seat))?)
                }
            };
            let (meta, owners) = self.game_meta(&game, &real);
            let seat_view = SeatView {
                view,
                version: *game.version.borrow(),
                deciding: table.deciding().map(|p| p.0),
                table: extras,
                undos: real.undos(),
                can_undo: false,
                undo: self.undo_state(&game, &real, None),
                meta,
                replay,
            };
            if let Some(t) = replayed {
                self.keep_replay(id, t);
            }
            (seat_view, owners)
        };
        seat_view.meta.users = self.users(&owners);
        Ok(seat_view)
    }

    /// Finished game `real` replayed to just before answer `step`, going on from the
    /// latest earlier point of it replayed lately.
    fn replayed(&self, id: &str, real: &Table, step: usize) -> Result<Table, HubError> {
        let from = {
            let mut replays = lock(&self.replays);
            let best = (replays.iter().enumerate())
                .filter(|(_, (k, t))| k == id && t.answers() <= step)
                .max_by_key(|(_, (_, t))| t.answers())
                .map(|(i, _)| i);
            best.and_then(|i| replays.remove(i)).map(|(_, t)| t)
        };
        engine(|| real.replay(step, from))?.map_err(|e| match e {
            AnswerError::Unreplayable(why) => {
                HubError::Conflict(format!("game {id} doesn't replay as it was played: {why}"))
            }
            e => e.into(),
        })
    }

    fn keep_replay(&self, id: &str, table: Table) {
        let mut replays = lock(&self.replays);
        replays.push_front((id.into(), table));
        replays.truncate(REPLAY_CACHE);
    }

    /// Game `id`'s version, which every change bumps, and its chat's counter.
    pub fn transcript_changes(&self, id: &str) -> Result<watch::Receiver<u64>, HubError> {
        Ok(self.read(id)?.transcript.subscribe())
    }

    pub(crate) fn transcript_changed(&self, id: &str) -> Result<(), HubError> {
        self.game(id)?.transcript.send_modify(|n| *n += 1);
        Ok(())
    }

    pub fn subscribe(&self, id: &str) -> Result<Feed, HubError> {
        let game = self.read(id)?;
        Ok(Feed {
            version: game.version.subscribe(),
            chat: game.chat.subscribe(),
        })
    }

    /// Tells game `id`'s followers that something was said in its chat.
    pub fn chat_said(&self, id: &str) {
        if let Ok(game) = self.game(id) {
            game.chat.send_modify(|v| *v += 1);
        }
    }

    /// Counts a call waiting on the game for `seat` until the result is dropped: while
    /// there is one, the seat is connected.
    pub fn connect(&self, seat: &Seat) -> Result<Connection, HubError> {
        let game = self.read(&seat.game)?;
        game.streams[seat.seat.index()].fetch_add(1, Ordering::Relaxed);
        Ok(Connection {
            game,
            seat: seat.seat.index(),
        })
    }

    /// Ages `seat`'s last call past [`CONNECTED_MS`], so only a live connection counts.
    #[cfg(test)]
    pub(crate) fn forget_last_call(&self, seat: &Seat) {
        let game = self.game(&seat.game).unwrap();
        game.seen[seat.seat.index()].store(0, Ordering::Relaxed);
    }

    /// Counts someone watching game `id` until the result is dropped.
    pub fn watching(&self, id: &str) -> Result<Watching, HubError> {
        let game = self.read(id)?;
        game.watchers.fetch_add(1, Ordering::Relaxed);
        Ok(Watching(game))
    }

    /// The name of `seat`.
    pub fn seat_name(&self, seat: &Seat) -> Result<String, HubError> {
        let game = self.read(&seat.game)?;
        let table = lock(&game.table);
        Ok(table.names()[seat.seat.index()].clone())
    }

    /// After a change to `game`, made with `table` locked: saves it and wakes its
    /// followers. What to tell [`Hub::ended`] once the lock is released.
    fn changed(&self, game: &Hosted, table: &Table) -> Option<Ended> {
        let now = now_ms();
        if let Some(clock) = &mut lock(&game.meta).clock {
            clock.advance(
                table,
                now.saturating_sub(game.since.load(Ordering::Relaxed)),
            );
        }
        let ended = table.is_over().then(|| {
            let mut meta = lock(&game.meta);
            meta.ended_at = Some(now_ms());
            Ended {
                id: table.id().into(),
                meta: meta.clone(),
                info: table.view(PlayerId(0)).game,
            }
        });
        self.save(game, table);
        game.since.store(now, Ordering::Relaxed);
        game.version.send_modify(|v| *v += 1);
        ended
    }

    fn ended(&self, ended: Option<Ended>) {
        if let (Some(ended), Some(on_end)) = (ended, self.on_end.get()) {
            on_end(ended);
        }
    }

    pub fn answer(&self, seat: &Seat, answer: &Answer) -> Result<(), HubError> {
        self.answer_with(seat, answer, None).map(|_| ())
    }

    /// Answers, then answers the Way questions that follow with `way` (`Table::answer_with`).
    pub fn answer_with(
        &self,
        seat: &Seat,
        answer: &Answer,
        way: Option<WayChoice>,
    ) -> Result<Answered, HubError> {
        self.mutate(seat, |next| next.answer_with(seat.seat, answer, way))
    }

    pub(crate) fn respond(
        &self,
        seat: &Seat,
        decision: &str,
        response: dominion_engine::Response,
    ) -> Result<(), HubError> {
        self.mutate(seat, |next| next.respond(seat.seat, decision, response))
    }

    /// Ends game `id` if a seat's clock has run out, which nothing may then change.
    fn in_time(&self, game: &Hosted, id: &str) -> Result<(), HubError> {
        let Some(flagged) = game.flagged(now_ms()) else {
            return Ok(());
        };
        let seat = Seat {
            game: id.into(),
            seat: flagged,
        };
        self.resign(&seat, "timeout")?;
        Err(HubError::Conflict(format!(
            "{}'s time ran out: the game is over",
            self.seat_name(&seat)?
        )))
    }

    fn mutate<T>(
        &self,
        seat: &Seat,
        apply: impl FnOnce(&mut Table) -> Result<T, AnswerError>,
    ) -> Result<T, HubError> {
        let game = self.game(&seat.game)?;
        self.in_time(&game, &seat.game)?;
        let (answered, ended) = {
            let mut table = lock(&game.table);
            let mut next = table.clone();
            let answered = engine(|| apply(&mut next))??;
            *table = next;
            self.answered(&game, &table, seat.seat);
            (answered, self.changed(&game, &table))
        };
        self.ended(ended);
        Ok(answered)
    }

    /// Whether answers can be taken back in `game` now: where its rules allow it, never
    /// once someone resigned, and never once a game between accounts is over, as it has
    /// been recorded.
    fn may_undo(&self, game: &Hosted, table: &Table) -> bool {
        let meta = lock(&game.meta);
        meta.undo != UndoRule::Off
            && table.can_rewind()
            && !(table.is_over() && meta.source != Source::Direct)
    }

    /// Whether `seat` may ask to take back its last answer now.
    fn can_undo(&self, game: &Hosted, table: &Table, seat: PlayerId) -> bool {
        self.may_undo(game, table)
            && table.can_undo(seat)
            && lock(&game.undo)
                .request
                .as_ref()
                .is_none_or(|r| r.by == seat)
    }

    /// After `seat` answered: its own request is withdrawn, and one the game no longer
    /// allows is dropped.
    fn answered(&self, game: &Hosted, table: &Table, seat: PlayerId) {
        let allowed = self.may_undo(game, table);
        let mut undo = lock(&game.undo);
        undo.acted(seat);
        if !allowed {
            undo.request = None;
        }
    }

    /// Asks to take back `seat`'s last answer, or with `log_seq` the answer that led to
    /// that log entry, with every answer after it. Where the game's rule is to ask, the
    /// other seats (but the server's agents) must allow it first; otherwise it happens
    /// now. Whether it happened.
    pub fn undo(&self, seat: &Seat, log_seq: Option<usize>) -> Result<bool, HubError> {
        let game = self.game(&seat.game)?;
        self.in_time(&game, &seat.game)?;
        let ended = {
            let mut table = lock(&game.table);
            if !self.may_undo(&game, &table) {
                return Err(HubError::Forbidden(
                    "answers can't be taken back in this game".into(),
                ));
            }
            let answer = match log_seq {
                None => table
                    .last_answer(seat.seat)
                    .ok_or(HubError::Answer(AnswerError::CannotUndo))?,
                Some(seq) if seq >= table.game().log().len() => {
                    return Err(bad(format!("there is no log entry {seq} yet")));
                }
                Some(seq) => table.answer_for_log(seq).ok_or_else(|| {
                    bad(format!(
                        "log entry {seq} came before the first answer: nothing before it \
                         can be undone"
                    ))
                })?,
            };
            let (rule, agents) = {
                let meta = lock(&game.meta);
                (meta.undo, meta.agents.clone())
            };
            let waiting: Vec<PlayerId> = match rule {
                UndoRule::Ask => (0..game.tokens.len())
                    .map(|i| PlayerId(i as u8))
                    .filter(|&p| p != seat.seat && !agents.get(p.index()).copied().unwrap_or(false))
                    .collect(),
                _ => vec![],
            };
            let mut undo = lock(&game.undo);
            if let Some(r) = undo.request.as_ref().filter(|r| r.by != seat.seat) {
                return Err(HubError::Conflict(format!(
                    "{} has asked for an undo; it must be allowed, refused or withdrawn \
                     before another",
                    table.names()[r.by.index()]
                )));
            }
            undo.acted(seat.seat);
            if !waiting.is_empty() {
                undo.request = Some(Request {
                    by: seat.seat,
                    answer,
                    waiting,
                });
                drop(undo);
                game.version.send_modify(|v| *v += 1);
                return Ok(false);
            }
            drop(undo);
            self.rewind(&game, &mut table, seat.seat, answer)?
        };
        self.ended(ended);
        Ok(true)
    }

    /// Takes the game in `table` back to just before `answer`, for `by`.
    fn rewind(
        &self,
        game: &Hosted,
        table: &mut Table,
        by: PlayerId,
        answer: usize,
    ) -> Result<Option<Ended>, HubError> {
        let mut next = table.clone();
        engine(|| next.rewind(by, answer))??;
        *table = next;
        Ok(self.changed(game, table))
    }

    /// `seat` allows or refuses the undo waiting for it. Once every seat asked has
    /// allowed it, it happens. Whether it happened.
    pub fn answer_undo(&self, seat: &Seat, allow: bool) -> Result<bool, HubError> {
        let game = self.game(&seat.game)?;
        self.in_time(&game, &seat.game)?;
        let ended = {
            let mut table = lock(&game.table);
            let allowed = self.may_undo(&game, &table);
            let mut undo = lock(&game.undo);
            if !undo.waits_for(seat.seat) {
                return Err(HubError::Conflict(
                    "no undo is waiting for you to allow it".into(),
                ));
            }
            let mut request = undo.request.take().expect("a request waits");
            if !allow {
                if let Some(r) = undo.refused.get_mut(request.by.index()) {
                    *r = Some(seat.seat);
                }
                drop(undo);
                game.version.send_modify(|v| *v += 1);
                return Ok(false);
            }
            if !allowed {
                drop(undo);
                game.version.send_modify(|v| *v += 1);
                return Err(HubError::Forbidden(
                    "answers can't be taken back in this game any more".into(),
                ));
            }
            request.waiting.retain(|&p| p != seat.seat);
            if !request.waiting.is_empty() {
                undo.request = Some(request);
                drop(undo);
                game.version.send_modify(|v| *v += 1);
                return Ok(false);
            }
            drop(undo);
            self.rewind(&game, &mut table, request.by, request.answer)?
        };
        self.ended(ended);
        Ok(true)
    }

    /// Withdraws `seat`'s undo request.
    pub fn cancel_undo(&self, seat: &Seat) -> Result<(), HubError> {
        let game = self.game(&seat.game)?;
        let _table = lock(&game.table);
        let mut undo = lock(&game.undo);
        if !undo.request.as_ref().is_some_and(|r| r.by == seat.seat) {
            return Err(HubError::Conflict(
                "you have no undo request to withdraw".into(),
            ));
        }
        undo.request = None;
        drop(undo);
        game.version.send_modify(|v| *v += 1);
        Ok(())
    }

    /// Plays the Treasures `seat` could play in its open Buy phase decision `decision`
    /// ([`Table::play_treasures`]). How many it played.
    pub fn play_treasures(&self, seat: &Seat, decision: &str) -> Result<u32, HubError> {
        let game = self.game(&seat.game)?;
        self.in_time(&game, &seat.game)?;
        let (played, ended) = {
            let mut table = lock(&game.table);
            let mut next = table.clone();
            let played = engine(|| next.play_treasures(seat.seat, decision))??;
            *table = next;
            self.answered(&game, &table, seat.seat);
            (played, self.changed(&game, &table))
        };
        self.ended(ended);
        Ok(played)
    }

    /// Ends the game with `seat` out of it: `cause` is `resigned`, `abandoned` or
    /// `timeout`.
    pub fn resign(&self, seat: &Seat, cause: &str) -> Result<(), HubError> {
        let game = self.game(&seat.game)?;
        let ended = {
            let mut table = lock(&game.table);
            let mut next = table.clone();
            engine(|| next.resign(seat.seat))??;
            *table = next;
            lock(&game.undo).request = None;
            lock(&game.meta).ended = Some(cause.into());
            self.changed(&game, &table)
        };
        self.ended(ended);
        Ok(())
    }

    /// Wakes the followers of every running game in which a seat has come or gone, or
    /// the number watching has changed: neither changes the game, so nothing else says.
    pub fn notice_presence(&self) {
        let games: Vec<Arc<Hosted>> = lock(&self.index).games.values().cloned().collect();
        let now = now_ms();
        for game in games {
            if lock(&game.table).is_over() {
                continue;
            }
            let connected: Vec<bool> = (0..game.tokens.len())
                .map(|s| game.connected(s, now))
                .collect();
            let seen = (connected, game.watchers.load(Ordering::Relaxed));
            let mut told = lock(&game.told);
            if *told != seen {
                *told = seen;
                game.version.send_modify(|v| *v += 1);
            }
        }
    }

    /// The seats whose time is up, with why: `timeout` (their clock ran out) or
    /// `abandoned` (the deciding seat of a timed game was away or too slow).
    pub fn overdue(&self) -> Vec<(Seat, &'static str)> {
        let timing = *lock(&self.timing);
        let games: Vec<Arc<Hosted>> = lock(&self.index).games.values().cloned().collect();
        let now = now_ms();
        games
            .iter()
            .filter_map(|g| {
                let table = lock(&g.table);
                if table.is_over() {
                    return None;
                }
                let seat = |seat| Seat {
                    game: table.id().into(),
                    seat,
                };
                if let Some(flagged) = g.flagged(now) {
                    return Some((seat(flagged), "timeout"));
                }
                let deciding = table.deciding()?;
                let timed = lock(&g.meta).timed;
                (timed && g.forfeit_at(deciding, timing)? <= now)
                    .then(|| (seat(deciding), "abandoned"))
            })
            .collect()
    }

    /// The games that ended and have no record yet.
    pub fn unrecorded(&self) -> Vec<Ended> {
        let games: Vec<Arc<Hosted>> = lock(&self.index).games.values().cloned().collect();
        games
            .iter()
            .filter_map(|g| {
                let table = lock(&g.table);
                let meta = lock(&g.meta);
                (table.is_over() && !meta.recorded).then(|| Ended {
                    id: table.id().into(),
                    meta: meta.clone(),
                    info: table.view(PlayerId(0)).game,
                })
            })
            .collect()
    }

    /// Notes that game `id`'s record has been written.
    pub fn set_recorded(&self, id: &str) {
        if let Ok(game) = self.game(id) {
            let table = lock(&game.table);
            lock(&game.meta).recorded = true;
            let changed = game.changed.load(Ordering::Relaxed);
            self.save(&game, &table);
            game.changed.store(changed, Ordering::Relaxed);
        }
    }

    /// Stops hosting the finished games that last changed more than `keep_finished` ago,
    /// and the unfinished ones nobody has touched for `keep_idle`, moving their files to
    /// `expired/`. How many.
    pub fn expire(&self, keep_finished: Duration, keep_idle: Duration) -> usize {
        let games: Vec<(String, Arc<Hosted>)> = {
            let index = lock(&self.index);
            let games = index.games.iter();
            games.map(|(id, g)| (id.clone(), g.clone())).collect()
        };
        let now = now_ms();
        let mut expired = 0;
        for (id, game) in games {
            let over = lock(&game.table).is_over();
            let keep = if over { keep_finished } else { keep_idle };
            if (over || !game.waited_on())
                && u128::from(now.saturating_sub(game.last_active())) >= keep.as_millis()
            {
                if let Err(e) = self.unhost(&id) {
                    eprintln!("dominion: {e}");
                }
                expired += 1;
            }
        }
        expired
    }

    /// Every hosted game as the operator sees it ([`Hub::retire`]).
    pub fn hosting(&self) -> Vec<Hosting> {
        let games: Vec<Arc<Hosted>> = lock(&self.index).games.values().cloned().collect();
        let now = now_ms();
        let mut hosting: Vec<Hosting> = games
            .iter()
            .map(|g| {
                let t = lock(&g.table);
                let meta = lock(&g.meta);
                Hosting {
                    id: t.id().into(),
                    names: t.names().to_vec(),
                    running: !t.is_over(),
                    source: meta.source,
                    table: meta.table.clone(),
                    agents: meta.agents.iter().any(|a| *a),
                    idle_seconds: now.saturating_sub(g.last_active()) / 1000,
                    connected: g.waited_on(),
                }
            })
            .collect();
        hosting.sort_by_key(|h| (h.id.len(), h.id.clone()));
        hosting
    }

    /// Stops hosting game `id` now, moving its file to `expired/` as expiry does. An
    /// unfinished game a lobby table started or the server's agents play is refused: it
    /// ends by resigning, which tells the table, the agents and the records.
    pub fn retire(&self, id: &str) -> Result<(), HubError> {
        let game = self.game(id)?;
        {
            let table = lock(&game.table);
            let meta = lock(&game.meta);
            if !table.is_over() && (meta.table.is_some() || meta.agents.iter().any(|a| *a)) {
                return Err(HubError::Conflict(format!(
                    "{id} is in progress at a lobby table or with the server's agents: resign it"
                )));
            }
        }
        self.unhost(id).map_err(HubError::Conflict)
    }

    fn unhost(&self, id: &str) -> Result<(), String> {
        lock(&self.index).remove(id);
        lock(&self.archive).retain(|(k, _)| k != id);
        match &self.store {
            Some(store) => {
                (store.retire(id)).map_err(|e| format!("moving game {id} to expired: {e}"))
            }
            None => Ok(()),
        }
    }

    /// Plays `steps` for `seat` (see [`crate::steps`]) until one can't be done or the seat
    /// has nothing left to decide. Returns the log lines the steps produced and the step that
    /// stopped them, with why.
    pub fn steps(&self, seat: &Seat, steps: &str) -> Result<Stepped, HubError> {
        let game = self.game(&seat.game)?;
        self.in_time(&game, &seat.game)?;
        let mut table = lock(&game.table);
        let seen = table
            .view(seat.seat)
            .since_last_decision
            .iter()
            .map(|e| e.seq)
            .max();
        let mut runner = Runner::new(table.clone(), seat.seat, false, seen);
        let stopped = engine(|| runner.run(steps))?.map_err(bad)?;
        let ended = if table.decision_id() != runner.table.decision_id()
            || table.is_over() != runner.table.is_over()
        {
            *table = runner.table;
            self.answered(&game, &table, seat.seat);
            self.changed(&game, &table)
        } else {
            None
        };
        drop(table);
        self.ended(ended);
        Ok(Stepped {
            log: runner.log,
            stopped,
        })
    }

    /// Waits until `seat` must decide or the game is over, for at most `timeout`. Returns
    /// whether it is so.
    pub async fn wait(&self, seat: &Seat, timeout: Duration) -> Result<bool, HubError> {
        let game = self.game(&seat.game)?;
        let _connected = self.connect(seat)?;
        let mut rx = game.version.subscribe();
        let ready = || {
            let table = lock(&game.table);
            table.is_over()
                || table.deciding() == Some(seat.seat)
                || lock(&game.undo).waits_for(seat.seat)
        };
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            if ready() {
                return Ok(true);
            }
            match tokio::time::timeout_at(deadline, rx.changed()).await {
                Ok(Ok(())) => continue,
                _ => return Ok(ready()),
            }
        }
    }
}

#[cfg(test)]
mod shared_response_tests {
    use super::*;
    use dominion_engine::{Move, Response};
    use std::sync::Barrier;

    #[test]
    fn typed_commit_is_atomic_against_http_style_answers_and_persistence() {
        let dir = tempfile::tempdir().unwrap();
        let (hub, _) = Hub::open(dir.path(), Duration::from_secs(3600)).unwrap();
        let hub = Arc::new(hub);
        let game = hub
            .create(&NewGame {
                players: 2,
                kingdom: vec!["Village".into()],
                seed: Some(1),
                ..NewGame::default()
            })
            .unwrap();
        let seat = hub.find(&game.seats[0].token).unwrap();
        let before = hub.seat_view(&seat, None).unwrap();
        let decision = before.view.decision.as_ref().unwrap();
        let answer = Answer::indices(&decision.id, vec![decision.options[0].index]);
        let saved_path = dir.path().join(format!("{}.json", game.game_id));
        let saved_before = fs::read(&saved_path).unwrap();
        let failed: Result<(), HubError> = hub.mutate(&seat, |next| {
            next.respond(seat.seat, &decision.id, Response::Move(Move::EndPhase))
                .unwrap();
            panic!("engine failure after a partial application");
        });
        assert!(matches!(failed, Err(HubError::Engine(_))));
        assert_eq!(fs::read(&saved_path).unwrap(), saved_before);
        assert_eq!(hub.seat_view(&seat, None).unwrap().view, before.view);
        assert!(
            hub.respond(&seat, &decision.id, Response::Cards(vec![]))
                .is_err()
        );
        assert_eq!(fs::read(&saved_path).unwrap(), saved_before);
        assert_eq!(hub.seat_view(&seat, None).unwrap().version, before.version);
        let barrier = Arc::new(Barrier::new(2));
        let (other_hub, other_seat, other_barrier) = (hub.clone(), seat.clone(), barrier.clone());
        let other = std::thread::spawn(move || {
            other_barrier.wait();
            other_hub.answer(&other_seat, &answer)
        });
        barrier.wait();
        let typed = hub.respond(&seat, &decision.id, Response::Move(Move::EndPhase));
        let generic = other.join().unwrap();
        assert_ne!(typed.is_ok(), generic.is_ok());
        assert_eq!(
            hub.seat_view(&seat, None).unwrap().version,
            before.version + 1
        );
        let committed = fs::read(&saved_path).unwrap();
        assert_ne!(committed, saved_before);
        assert!(
            hub.respond(&seat, &decision.id, Response::Move(Move::EndPhase))
                .is_err()
        );
        assert_eq!(fs::read(&saved_path).unwrap(), committed);
    }

    #[test]
    fn observation_is_coherent_and_invalid_seats_are_rejected() {
        let hub = Hub::default();
        let game = hub
            .create(&NewGame {
                players: 2,
                kingdom: vec!["Village".into()],
                seed: Some(1),
                ..NewGame::default()
            })
            .unwrap();
        let seat = hub.find(&game.seats[0].token).unwrap();
        let before = hub.seat_view(&seat, None).unwrap();
        hub.project_seat(&seat, |observation| {
            assert_eq!(observation.version, before.version);
            assert_eq!(
                observation.decision_id,
                before.view.decision.as_ref().map(|d| d.id.as_str())
            );
            assert_eq!(observation.deciding, Some(seat.seat));
            assert!(observation.outcome.is_none());
            assert_eq!(observation.names.len(), 2);
            assert_eq!(observation.view.me(), Some(seat.seat));
        })
        .unwrap();
        let invalid = Seat {
            game: game.game_id,
            seat: PlayerId(9),
        };
        assert!(matches!(
            hub.project_seat(&invalid, |_| ()),
            Err(HubError::Forbidden(_))
        ));
    }
}

#[cfg(test)]
mod archive_tests {
    use super::*;

    #[test]
    fn expired_games_can_be_viewed_but_not_played() {
        let dir = tempfile::tempdir().unwrap();
        let (hub, _) = Hub::open(dir.path(), Duration::from_secs(3600)).unwrap();
        let game = hub
            .create(&NewGame {
                players: 2,
                kingdom: vec!["Village".into()],
                seed: Some(1),
                watchable: Some(true),
                ..NewGame::default()
            })
            .unwrap();
        let id = game.game_id.clone();
        let token = game.seats[0].token.clone();
        let seat = hub.claim(&id, &token).unwrap();
        hub.resign(&seat, "resigned").unwrap();
        let ended = hub.seat_view(&seat, Some(0)).unwrap();
        hub.retire(&id).unwrap();
        assert!(hub.list().is_empty());

        let check = |hub: &Hub| {
            let seat = hub.claim(&id, &token).unwrap();
            assert_eq!(hub.seat_view(&seat, Some(0)).unwrap().view, ended.view);
            assert!(!hub.watch_view(&id, 0, None).unwrap().view.game.running);
            assert!(hub.subscribe(&id).is_ok());
            assert!(matches!(
                hub.resign(&seat, "resigned"),
                Err(HubError::UnknownGame(_))
            ));
            assert!(matches!(hub.claim(&id, "nope"), Err(HubError::BadToken(_))));
            assert!(hub.list().is_empty());
        };
        check(&hub);
        drop(hub);
        let (hub, loaded) = Hub::open(dir.path(), Duration::from_secs(3600)).unwrap();
        assert_eq!(loaded.games, 0);
        check(&hub);
        for id in ["../g1", "g01", "g1/../g1", "expired/g1"] {
            assert!(matches!(
                hub.watch_view(id, 0, None),
                Err(HubError::UnknownGame(_))
            ));
        }
    }
}
