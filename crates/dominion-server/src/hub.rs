//! The games a server hosts, and the seat tokens that let a client act for a seat.

mod model;

use crate::steps::Runner;
use crate::store::Store;
use dominion_agent::recording::{self, Encoding};
use dominion_agent::{AgentView, Answer, AnswerError, Answered, Table, WayChoice};
use dominion_engine::{
    Card, DealRules, Game, GameConfig, LandscapeSlot, PlayerId, SLOT_KINDS, Set,
};
pub use dominion_protocol::api::{
    Created, DealRequest, Dealt, GameSummary, NewGame, SeatToken, SeatView,
};
use dominion_protocol::api::{Progress, Replay, ReplayTurn, SeatProgress, UndoRequest, UndoState};
use dominion_protocol::events::History;
use dominion_protocol::lobby::{
    AgentAttempt, AgentRun, AgentSetup, Clock, ClockInfo, ClockStatus, ClockView, GameMeta, MyGame,
    RatingInfo, SeatAgent, Source, UndoRule, UserRef,
};
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
    #[error("{0}")]
    NotFound(String),
    /// The change couldn't be saved, so it wasn't made: the game is as it was.
    #[error("the game couldn't be saved ({0}); nothing was changed. Try again")]
    Unsaved(String),
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
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub model_controls: Vec<crate::model_control::ModelControl>,
    pub source: Source,
    /// Per seat: the id of the account playing it.
    pub owners: Vec<Option<String>>,
    pub rated: bool,
    /// When it was taken out of the rated record (milliseconds since the Unix epoch): it
    /// was rated, and no longer is.
    pub voided: Option<u64>,
    pub watchable: bool,
    /// The deciding seat forfeits when it is away or takes too long ([`Timing`]).
    pub timed: bool,
    /// A chess clock: a seat whose time runs out forfeits.
    pub clock: Option<ClockState>,
    /// Whether answers can be taken back: an undo shows a player what would have happened.
    pub undo: UndoRule,
    /// Per seat: played by one of the server's agents, which allow every undo.
    pub agents: Vec<bool>,
    /// Per seat: played by an agent the server doesn't run: over MCP, or with a bot's key.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub remote_agents: Vec<bool>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub launch_options: Vec<dominion_protocol::lobby::LaunchOptions>,
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
    /// Per seat: what played it for one of the server's agents, as its runner reported.
    pub runs: Vec<Option<AgentRun>>,
}

/// The most of an agent's error kept with its game.
const MAX_AGENT_ERROR: usize = 1000;

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
    pub seed: u64,
    /// With its result.
    pub info: GameInfo,
    /// What it was set up with besides its Kingdom piles and landscapes.
    pub setup: Vec<String>,
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
    pub prompts: Vec<String>,
    pub view: AgentView,
    pub log: Vec<dominion_agent::LogEntry>,
    pub glossary: Vec<dominion_agent::GlossaryEntry>,
    pub undos: u64,
    /// What the seat is told about undos.
    pub note: Option<String>,
    /// The game's chess clock as it stands.
    pub clock: Option<ClockView>,
    /// Freshness and availability metadata for `clock`.
    pub clock_info: Option<ClockInfo>,
    /// What people at the table have said, its last lines in order.
    pub chat: Vec<dominion_protocol::lobby::ChatMessage>,
    /// The caller is the seat's model and a person holds the seat.
    pub advising: Option<Advising>,
}

/// What a seat's model is told while a person holds the seat.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Advising {
    /// It owes the person a note now.
    pub wanted: bool,
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
    /// Locked after `table`.
    stream: Mutex<StreamFile>,
}

/// A hosted game's stream file, and the writes to it that failed running.
#[derive(Clone, Debug)]
struct StreamFile {
    at: Stream,
    failures: u32,
    /// For a reconstructed stream, the history it was built from (the game's when it was
    /// loaded): once the game's is another, the file is set aside as stale.
    built: Option<History>,
}

/// After this many failed writes running, a game's stream stops.
const STREAM_FAILURES: u32 = 3;

/// Where a hosted game's event stream stands on disk.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Stream {
    /// None is kept: the game doesn't record one, or its file doesn't hold the events its
    /// table says were written ([`Loaded::unrecorded`]).
    Off,
    /// No file yet: the first write makes one, in the hub's encoding.
    New,
    /// The file's encoding, and its length up to the end of the events written.
    File(Encoding, u64),
    /// A whole stream `dominion games backfill` rebuilt ([`check_reconstructed`]), its
    /// encoding and length: never written to, served only once the game can't change,
    /// and set aside as stale once the game's history is no longer the one it was built
    /// from.
    Reconstructed(Encoding, u64),
}

/// Why a game's stream on disk can't be kept.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum StreamError {
    /// The stream it records isn't in step with it, or its recording stopped: its
    /// recording stops for good.
    Recorded(String),
    /// The reconstructed stream beside it was built from another history of the game (it
    /// was undone since) or doesn't say which: never served, and set aside as stale.
    Stale(String),
    /// The reconstructed stream beside it isn't its own: left as it is, never served.
    Reconstructed(String),
}

impl std::fmt::Display for StreamError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StreamError::Recorded(why) => f.write_str(why),
            StreamError::Stale(why) => write!(f, "its reconstructed stream is stale: {why}"),
            StreamError::Reconstructed(why) => write!(f, "its reconstructed stream: {why}"),
        }
    }
}

/// Why a reconstructed stream isn't the game's.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Unfit {
    /// It was built from another history of the game, or doesn't say which.
    Stale(String),
    /// It is no whole stream of this game.
    Broken(String),
}

impl std::fmt::Display for Unfit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Unfit::Stale(why) | Unfit::Broken(why) => f.write_str(why),
        }
    }
}

impl Hosted {
    /// Whether `seat`'s caller may answer for it now: whoever holds the seat, of its
    /// player and its model.
    fn turn_of(&self, seat: &Seat) -> Result<(), HubError> {
        let meta = lock(&self.meta);
        let Some(control) = meta.model_controls.get(seat.seat.index()) else {
            return Ok(());
        };
        if seat.model || control.attached.is_none() {
            control.model_turn()
        } else {
            control.player_turn()
        }
    }

    /// Whether `seat`'s caller is its model, with a note owed to the person holding it.
    fn advice_wanted(&self, table: &Table, seat: &Seat) -> bool {
        let wanted = {
            let meta = lock(&self.meta);
            (meta.model_controls.get(seat.seat.index()))
                .filter(|c| seat.model || c.attached.is_none())
                .filter(|c| c.may_want_advice())
                .cloned()
        };
        wanted.is_some_and(|control| {
            control.wants_advice(&crate::model_control::Moment::of(&table.view(seat.seat)))
        })
    }

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

    /// `stream` as [`stream_state`] found it for `table`.
    fn new(table: Table, tokens: Vec<String>, meta: Meta, changed: u64, stream: Stream) -> Hosted {
        let now = now_ms();
        let seats = tokens.len();
        let built = matches!(stream, Stream::Reconstructed(..)).then(|| table.history());
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
            stream: Mutex::new(StreamFile {
                at: stream,
                failures: 0,
                built,
            }),
        }
    }

    /// Notes that `seat`'s player is here. A model attached to a person's seat isn't them.
    fn touch(&self, seat: &Seat) {
        if seat.model {
            return;
        }
        if let Some(j) = self.joined.get(seat.seat.index()) {
            j.store(true, Ordering::Relaxed);
            self.seen[seat.seat.index()].store(now_ms(), Ordering::Relaxed);
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
    /// Claimed with the token of the model attached to the seat, not the seat's own.
    pub model: bool,
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
pub(crate) struct SavedMeta {
    #[serde(flatten)]
    meta: Meta,
    /// What a game saved before [`Meta::undo`] said instead.
    #[serde(default)]
    no_undo: bool,
}

impl SavedMeta {
    pub(crate) fn into_meta(self) -> Meta {
        let mut meta = self.meta;
        if self.no_undo {
            meta.undo = UndoRule::Off;
        }
        meta
    }
}

/// What [`Hub::open`] found in its directory.
#[derive(Debug, Default)]
pub struct Loaded {
    pub games: usize,
    /// Finished games older than the retention period, deleted.
    pub expired: usize,
    /// Files that couldn't be read, left in place.
    pub unreadable: Vec<(PathBuf, String)>,
    /// Games whose recording stopped, or whose saved recording can't go on (saved by
    /// another build, or its stream doesn't hold what their table says was written to it):
    /// hosted, recording nothing more, their stream file, if any, marked incomplete.
    pub unrecorded: Vec<(PathBuf, String)>,
    /// Games saved without a recording: before games were recorded, or with recording off.
    pub never_recorded: usize,
    /// Games whose stream `dominion games backfill` rebuilt, served.
    pub reconstructed: usize,
    /// Games whose stream `dominion games backfill` rebuilt, which can still change (a
    /// finished game that can be undone): hosted as ever, the stream kept and served once
    /// the game can't change, or set aside as stale if it changes first.
    pub reconstructed_unserved: usize,
    /// Reconstructed streams built from a history their game no longer has (it was
    /// undone since): set aside as `<id>.stale.events.<encoding>`, never served.
    pub stale: Vec<(PathBuf, String)>,
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
                model: false,
            };
            if let Some(Some(owner)) = lock(&hosted.meta).owners.get(i) {
                self.owned
                    .entry(owner.clone())
                    .or_default()
                    .push(seat.clone());
            }
            self.seats.insert(token.clone(), seat);
        }
        for (i, control) in lock(&hosted.meta).model_controls.iter().enumerate() {
            if let Some(model) = &control.attached {
                let seat = Seat {
                    game: id.into(),
                    seat: PlayerId(i as u8),
                    model: true,
                };
                self.seats.insert(model.token.clone(), seat);
            }
        }
        self.games.insert(id.into(), Arc::new(hosted));
    }

    fn remove(&mut self, id: &str) {
        if let Some(game) = self.games.remove(id) {
            for token in &game.tokens {
                self.seats.remove(token);
            }
            self.seats.retain(|_, seat| seat.game != id);
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
type Ratings = Box<dyn Fn(&[Option<String>]) -> Vec<Option<RatingInfo>> + Send + Sync>;

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
    /// Account ids to their ratings, called with no lock held.
    ratings: OnceLock<Ratings>,
    /// What new games' event streams are written in.
    encoding: Mutex<Encoding>,
    /// Whether new games are recorded (where games are saved).
    record: AtomicBool,
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
            ratings: OnceLock::new(),
            encoding: Mutex::default(),
            record: AtomicBool::new(true),
        }
    }
}

/// Counts a call waiting on a game for a seat, until dropped.
pub struct Connection {
    game: Arc<Hosted>,
    /// None: the model attached to a person's seat, which isn't them being there.
    seat: Option<usize>,
}

impl Drop for Connection {
    fn drop(&mut self) {
        if let Some(seat) = self.seat {
            self.game.seen[seat].store(now_ms(), Ordering::Relaxed);
            self.game.streams[seat].fetch_sub(1, Ordering::Relaxed);
        }
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
    let saved = (saved.table, saved.tokens, saved.meta.into_meta());
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

/// Where game `table`'s stream stands on disk in state directory `dir` (or its
/// `expired/`), checked against the table.
///
/// The stream is kept in step with the table's snapshot by [`Hub::commit`]: whenever the
/// server stopped, the stream's first [`Table::written`] events are the table's, and
/// anything after them is cut off by the next write. A stream whose header says it was
/// `reconstructed` is no recording's: it is checked as a whole ([`check_reconstructed`])
/// whatever the table's recording says, and never written to.
pub(crate) fn stream_state(dir: &Path, table: &Table) -> Result<Stream, StreamError> {
    let file = crate::store::stream_file(dir, table.id());
    if let Some((path, encoding)) = &file {
        let at = |e: String| format!("{}: {e}", path.display());
        let bytes = fs::read(path).map_err(|e| StreamError::Reconstructed(at(e.to_string())))?;
        if recording::read(*encoding, &bytes, Some(0))
            .is_ok_and(|r| r.header.reconstructed.is_some())
        {
            return check_reconstructed(*encoding, &bytes, table)
                .map(|()| Stream::Reconstructed(*encoding, bytes.len() as u64))
                .map_err(|e| match e {
                    Unfit::Stale(e) => StreamError::Stale(at(e)),
                    Unfit::Broken(e) => StreamError::Reconstructed(at(e)),
                });
        }
    }
    recorded_state(file, table).map_err(StreamError::Recorded)
}

/// [`stream_state`] of a stream the table recorded, in `file` if it has one.
pub(crate) fn recorded_state(
    file: Option<(PathBuf, Encoding)>,
    table: &Table,
) -> Result<Stream, String> {
    if let Some(why) = table.recording_stopped() {
        return Err(format!("its recording stopped: {why}"));
    }
    let Some(written) = table.written() else {
        return Ok(Stream::Off);
    };
    let Some((path, encoding)) = file else {
        return match written {
            0 => Ok(Stream::New),
            _ => Err(format!("its event stream, of {written} events, is missing")),
        };
    };
    let bytes = fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let read = recording::read(encoding, &bytes, Some(written as usize))
        .map_err(|e| format!("{}: {e}", path.display()))?;
    if read.header.game.as_deref() != Some(table.id()) {
        return Err(format!("{} is not its stream", path.display()));
    }
    let len = read.len_of(written as usize).ok_or_else(|| {
        format!(
            "{} holds {} of the {written} events written",
            path.display(),
            read.events.len()
        )
    })?;
    // Its events and those still to write must be the stream the table recorded.
    if !folds_to(read.events.iter().chain(table.unwritten()), table) {
        return Err(format!(
            "{}'s first {written} events are not the game's",
            path.display()
        ));
    }
    Ok(Stream::File(encoding, len as u64))
}

/// Whether `events` are a whole stream that folds to the table `table` recorded.
fn folds_to<'a>(
    events: impl Iterator<Item = &'a dominion_protocol::events::Event>,
    table: &Table,
) -> bool {
    let mut folded = dominion_protocol::events::Table::default();
    for event in events {
        if folded.apply(event).is_err() {
            return false;
        }
    }
    table.recorded_table() == Some(&folded)
}

/// Stops recording `table`'s stream at load, for `why`, and marks its file, if any,
/// incomplete. Its snapshot says so from its next change; until then loading it again
/// finds the same.
fn stop_stream(store: &Store, table: &mut Table, why: &str) {
    table.stop_recording(why);
    if let Err(e) = store.mark_incomplete(table.id()) {
        eprintln!(
            "dominion: marking game {}'s stream incomplete: {e}",
            table.id()
        );
    }
}

/// The snapshot of `table`: when its recording state can't be written, without it (its
/// recording stops), so that recording never keeps a game from being saved.
fn snapshot(table: &mut Table, tokens: &[String], meta: &Meta) -> Result<Vec<u8>, String> {
    let write = |table: &Table| {
        serde_json::to_vec(&SavedRef {
            table,
            tokens,
            meta,
        })
    };
    match write(table) {
        Ok(bytes) => Ok(bytes),
        Err(e) if table.records() => {
            eprintln!(
                "dominion: game {}'s recording can't be saved, so it stops: {e}",
                table.id()
            );
            table.stop_recording(format!("its recording can't be saved: {e}"));
            write(table).map_err(|e| e.to_string())
        }
        Err(e) => Err(e.to_string()),
    }
}

fn result_summary(table: &Table, meta: &Meta, joined: Vec<bool>) -> GameSummary {
    let watchable = meta.watchable && table.watchable();
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
        watchable,
        users: vec![],
        agents: (0..table.names().len())
            .map(|i| match meta.runs.get(i) {
                Some(Some(run)) => Some(SeatAgent::from(&run.setup)),
                _ if meta.agents.get(i).copied().unwrap_or(false) => Some(SeatAgent {
                    runner_strategy: meta
                        .launch_options
                        .get(i)
                        .copied()
                        .unwrap_or_default()
                        .strategy,
                    speed: meta.launch_options.get(i).and_then(|o| o.speed),
                    agent: table.names()[i].clone(),
                    ..SeatAgent::default()
                }),
                _ => None,
            })
            .collect(),
        progress: progress(table, watchable),
    }
}

/// Where `table`'s game stands while it runs; what each seat owns and would score only
/// when anyone may watch it.
fn progress(table: &Table, watchable: bool) -> Option<Progress> {
    let g = table.game();
    if table.is_over() {
        return None;
    }
    let t = g.turn();
    let seats = if watchable {
        (0..g.num_players() as u8)
            .map(|p| {
                let p = PlayerId(p);
                let mut owned = BTreeMap::new();
                for id in g.player(p).owned() {
                    *owned.entry(g.card(id).name().to_string()).or_insert(0) += 1;
                }
                SeatProgress {
                    vp: g.score(p),
                    owned,
                }
            })
            .collect()
    } else {
        vec![]
    };
    Some(Progress {
        player: t.player.0,
        turn: g.player(t.player).turns_taken,
        phase: format!("{:?}", t.phase).to_lowercase(),
        seats,
    })
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
                // A write the last server didn't finish; the game's files are intact.
                if name.ends_with(".tmp") {
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
            let mut table = table;
            let mut state = stream_state(store.dir(), &table);
            if let Err(StreamError::Stale(e)) = &state {
                // Of a history the game no longer has: set aside, and the game has what is
                // left (its own recording, or none) until a backfill writes it afresh.
                let why = match store.mark_stale(stem) {
                    Ok(Some(to)) => format!("{e}; set aside as {}", to.display()),
                    Ok(None) => e.clone(),
                    Err(err) => format!("{e}; setting it aside: {err}"),
                };
                loaded.stale.push((path.clone(), why));
                state = stream_state(store.dir(), &table);
            }
            let stream = match state {
                Ok(Stream::Off) => {
                    loaded.never_recorded += 1;
                    Stream::Off
                }
                Ok(stream @ Stream::Reconstructed(..)) => {
                    // A recording whose stream was lost and rebuilt: the rebuilt one stands.
                    table.stop_recording("its stream was rebuilt by `dominion games backfill`");
                    // One of a game that can still change is kept as it is, and served
                    // once the game can't change; if it changes first, it is set aside.
                    match can_change(&meta, &table) {
                        true => loaded.reconstructed_unserved += 1,
                        false => loaded.reconstructed += 1,
                    }
                    stream
                }
                Ok(stream) => stream,
                Err(StreamError::Recorded(e)) => {
                    stop_stream(&store, &mut table, &e);
                    loaded.unrecorded.push((path.clone(), e));
                    Stream::Off
                }
                Err(e @ (StreamError::Reconstructed(_) | StreamError::Stale(_))) => {
                    let e = e.to_string();
                    table.stop_recording(&e);
                    loaded.unrecorded.push((path.clone(), e));
                    Stream::Off
                }
            };
            index.insert(stem, Hosted::new(table, tokens, meta, changed, stream));
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
    /// game's seats and `ratings` for their ratings. None may be set twice.
    pub fn set_hooks(
        &self,
        on_end: impl Fn(Ended) + Send + Sync + 'static,
        users: impl Fn(&[Option<String>]) -> Vec<Option<UserRef>> + Send + Sync + 'static,
        ratings: impl Fn(&[Option<String>]) -> Vec<Option<RatingInfo>> + Send + Sync + 'static,
    ) {
        let _ = self.on_end.set(Box::new(on_end));
        let _ = self.users.set(Box::new(users));
        let _ = self.ratings.set(Box::new(ratings));
    }

    fn ratings(&self, owners: &[Option<String>]) -> Vec<Option<RatingInfo>> {
        match self.ratings.get() {
            Some(ratings) if owners.iter().any(Option::is_some) => ratings(owners),
            _ => vec![],
        }
    }

    fn users(&self, owners: &[Option<String>]) -> Vec<Option<UserRef>> {
        match self.users.get() {
            Some(users) if owners.iter().any(Option::is_some) => users(owners),
            _ => vec![None; owners.len()],
        }
    }

    /// What new games' event streams are written in (JSON Lines unless set).
    pub fn set_events_encoding(&self, encoding: Encoding) {
        *lock(&self.encoding) = encoding;
    }

    /// Whether games that start from now on are recorded as event streams (they are
    /// unless set). Games already recording go on.
    pub fn set_record_events(&self, record: bool) {
        self.record.store(record, Ordering::Relaxed);
    }

    /// Commits a change to `game`, made with `table` locked: `next`, the table changed
    /// (none: the same table), with `edit` made to its meta given the table it goes with.
    ///
    /// The snapshot is the commit: the change takes effect only once it is saved, and when
    /// it can't be, nothing changes and the change is refused ([`HubError::Unsaved`]), so
    /// a change is never acknowledged that a restart would lose. Then the events the table
    /// recorded go to its stream ([`Hub::write_events`]).
    fn commit(
        &self,
        game: &Hosted,
        table: &mut Table,
        next: Option<Table>,
        edit: impl FnOnce(&mut Meta, &Table),
    ) -> Result<(), HubError> {
        self.commit_meta(game, table, next, edit, false)
    }

    /// [`Hub::commit`]; with `noted`, an edit to the meta that can't be saved is still made
    /// in memory.
    fn commit_meta(
        &self,
        game: &Hosted,
        table: &mut Table,
        mut next: Option<Table>,
        edit: impl FnOnce(&mut Meta, &Table),
        noted: bool,
    ) -> Result<(), HubError> {
        let mut meta = lock(&game.meta);
        let mut edited = meta.clone();
        let target = next.as_mut().unwrap_or(&mut *table);
        edit(&mut edited, target);
        if let Some(store) = &self.store {
            let id = target.id().to_string();
            let saved = snapshot(target, &game.tokens, &edited)
                .and_then(|bytes| store.save_bytes(&id, &bytes).map_err(|e| e.to_string()));
            if let Err(e) = saved {
                eprintln!("dominion: saving game {id}: {e}");
                if noted {
                    *meta = edited;
                }
                return Err(HubError::Unsaved(e));
            }
        }
        *meta = edited;
        drop(meta);
        if let Some(next) = next {
            *table = next;
        }
        game.changed.store(now_ms(), Ordering::Relaxed);
        self.write_events(game, table);
        Ok(())
    }

    /// Writes the events `table` recorded since its last snapshot to its stream.
    ///
    /// The snapshot has gone first and carries those events ([`Table::unwritten`]); they
    /// are marked written only once the stream holding them is synced. So however the
    /// server stops, the stream on disk holds at least the events the last snapshot says
    /// were written, and they are the snapshot's: [`stream_state`] cuts it back to them,
    /// and the snapshot's unwritten events follow at the next write. After an undo the
    /// table's unwritten events are its whole stream again, which replaces the file. A
    /// write that fails is tried again with the next; after [`STREAM_FAILURES`] running,
    /// or once the table's recording has stopped, the stream stops for good: the file is
    /// marked incomplete and nothing more is written.
    fn write_events(&self, game: &Hosted, table: &mut Table) {
        let Some(store) = &self.store else {
            table.mark_written();
            return;
        };
        let mut stream = lock(&game.stream);
        if let Stream::Reconstructed(..) = stream.at {
            if stream.built.as_ref() == Some(&table.history()) {
                return;
            }
            // Undone: the file is no longer the game's history. It is never written to;
            // set aside, it leaves the game unrecorded, for a backfill to write afresh once
            // the game is over again. Until it is, a restart finds it stale.
            stream.at = Stream::Off;
            stream.built = None;
            if let Err(e) = store.mark_stale(table.id()) {
                eprintln!(
                    "dominion: setting game {}'s stale reconstructed stream aside: {e}",
                    table.id()
                );
            }
        }
        if let Some(why) = table.recording_stopped() {
            if stream.at != Stream::Off {
                eprintln!("dominion: game {}'s recording stopped: {why}", table.id());
                stream.at = Stream::Off;
                if let Err(e) = store.mark_incomplete(table.id()) {
                    eprintln!(
                        "dominion: marking game {}'s stream incomplete: {e}",
                        table.id()
                    );
                }
            }
            return;
        }
        if table.unwritten().is_empty() {
            return;
        }
        let (encoding, len) = match stream.at {
            Stream::Off | Stream::Reconstructed(..) => {
                table.mark_written();
                return;
            }
            Stream::New => (*lock(&self.encoding), 0),
            Stream::File(encoding, len) => (encoding, len),
        };
        match store.write_stream(table.id(), encoding, len, table.unwritten()) {
            Ok(len) => {
                stream.at = Stream::File(encoding, len);
                stream.failures = 0;
                table.mark_written();
            }
            Err(e) => {
                stream.failures += 1;
                eprintln!("dominion: writing game {}'s events: {e}", table.id());
                if stream.failures < STREAM_FAILURES {
                    return;
                }
                drop(stream);
                table.stop_recording(format!(
                    "its stream couldn't be written {STREAM_FAILURES} times running: {e}"
                ));
                let meta = lock(&game.meta);
                let saved = snapshot(table, &game.tokens, &meta)
                    .and_then(|b| store.save_bytes(table.id(), &b).map_err(|e| e.to_string()));
                if let Err(e) = saved {
                    eprintln!("dominion: saving game {}: {e}", table.id());
                }
                drop(meta);
                self.write_events(game, table);
            }
        }
    }

    pub fn create(&self, req: &NewGame) -> Result<Created, HubError> {
        if req.agents.iter().any(Option::is_some) {
            return Err(bad("agent seats must be assigned through the arena".into()));
        }
        let config = config(req)?;
        let names = names(req)?;
        let mut meta = Meta {
            watchable: req.watchable.unwrap_or(true),
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
        let recorded = self.store.is_some() && self.record.load(Ordering::Relaxed);
        let table = engine(|| match recorded {
            true => Table::new_recorded(&id, config, names),
            false => Table::new(&id, config, names),
        })?
        .map_err(|e| bad(e.to_string()))?;
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
        let stream = if recorded { Stream::New } else { Stream::Off };
        let hosted = Hosted::new(table, tokens, meta, now_ms(), stream);
        self.commit(&hosted, &mut lock(&hosted.table), None, |_, _| {})?;
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
        let stream = stream_state(store.dir(), &table)
            .inspect_err(|e| eprintln!("dominion: game {id}: {e}"))
            .unwrap_or(Stream::Off);
        let game = Arc::new(Hosted::new(table, tokens, meta, changed, stream));
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
                model: false,
            });
        };
        let seat = index
            .seats
            .get(token)
            .filter(|seat| seat.game == id)
            .cloned()
            .ok_or_else(|| HubError::BadToken(id.into()))?;
        game.touch(&seat);
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
            game.touch(&seat);
        }
        Ok(seat)
    }

    /// Notes that `seat` is being played now.
    pub fn touch(&self, seat: &Seat) {
        if let Ok(game) = self.game(&seat.game) {
            game.touch(seat);
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
                model: false,
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
        let mut table = lock(&game.table);
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
        let debrief = text[..end].to_string();
        self.commit(&game, &mut table, None, |meta, table| {
            meta.debriefs.resize(table.names().len(), None);
            meta.debriefs[seat.seat.index()] = Some(debrief);
        })?;
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

    /// [`Hub::project_seat`] for a spectator of game `id`: every player's held cards face
    /// up ([`dominion_engine::Game::spectate`]) and what someone watching knows.
    /// Refused unless the game can be watched.
    pub(crate) fn project_watch<T>(
        &self,
        id: &str,
        project: impl for<'a> FnOnce(SeatObservation<'a>) -> T,
    ) -> Result<T, HubError> {
        let game = self.read(id)?;
        let table = lock(&game.table);
        let meta = lock(&game.meta);
        let refused = || HubError::Forbidden(format!("game {id} can't be watched"));
        if !meta.watchable {
            return Err(refused());
        }
        engine(|| {
            let known = table
                .knowledge()
                .watched(table.game())
                .ok_or_else(refused)?;
            let decision = table.decision_id();
            Ok(project(SeatObservation {
                view: table.game().spectate(),
                clock: game.clock(&meta, now_ms()),
                known: &known,
                names: table.names(),
                decision_id: decision.as_deref(),
                deciding: table.deciding(),
                version: *game.version.borrow(),
                undos: table.undos(),
                outcome: table.game().outcome(),
                ended: meta.ended.as_deref(),
            }))
        })?
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
            let advising = self.advising(&game, &table, seat);
            PlayObservation {
                prompts: lock(&game.meta)
                    .model_controls
                    .get(seat.seat.index())
                    .map(|c| c.prompts.clone())
                    .unwrap_or_default(),
                view,
                log: table.view_from(seat.seat, 0).since_last_decision,
                glossary,
                undos: table.undos(),
                note: None,
                clock: None,
                clock_info: None,
                chat: Vec::new(),
                advising,
            }
        })
        .map(|mut observation| {
            let now = now_ms();
            observation.note = self.undo_note(&game, &table, seat.seat);
            observation.clock = game.clock(&lock(&game.meta), now).map(|(_, c)| c);
            observation.clock_info = Some(ClockInfo {
                status: if observation.clock.is_some() {
                    ClockStatus::Available
                } else {
                    ClockStatus::Off
                },
                as_of_ms: now,
                sampled_at_ms: Vec::new(),
            });
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
            voided: meta.voided.is_some(),
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
            levels: vec![],
            ratings: vec![],
            online_ratings: vec![],
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
        seat_view.meta.ratings = self.ratings(&owners);
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
                    seat: None,
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
        seat_view.meta.ratings = self.ratings(&owners);
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
        let seat = (!seat.model).then(|| seat.seat.index());
        if let Some(seat) = seat {
            game.streams[seat].fetch_add(1, Ordering::Relaxed);
        }
        Ok(Connection { game, seat })
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

    /// Commits `next`, a change to `game` made with `table` locked, with `edit` made to its
    /// meta, and wakes its followers. What to tell [`Hub::ended`] once the lock is released.
    fn changed(
        &self,
        game: &Hosted,
        table: &mut Table,
        next: Table,
        edit: impl FnOnce(&mut Meta),
    ) -> Result<Option<Ended>, HubError> {
        let now = now_ms();
        let elapsed = now.saturating_sub(game.since.load(Ordering::Relaxed));
        self.commit(game, table, Some(next), |meta, next| {
            edit(meta);
            if let Some(clock) = &mut meta.clock {
                clock.advance(next, elapsed);
            }
            if next.is_over() {
                meta.ended_at = Some(now);
            }
        })?;
        let ended = table.is_over().then(|| {
            let view = table.view(PlayerId(0));
            Ended {
                id: table.id().into(),
                meta: lock(&game.meta).clone(),
                seed: table.game().config().seed,
                setup: crate::setup_cards::setup_cards(&view),
                info: view.game,
            }
        });
        game.since.store(now, Ordering::Relaxed);
        game.version.send_modify(|v| *v += 1);
        Ok(ended)
    }

    /// Runs `apply` on a copy of `table`. Should the engine fail while journaling, it
    /// is run again on a copy that doesn't record: recording never keeps an answer from
    /// being made.
    fn attempt<T>(
        &self,
        table: &Table,
        apply: impl Fn(&mut Table) -> Result<T, AnswerError>,
    ) -> Result<(Table, T), HubError> {
        let mut next = table.clone();
        match engine(|| apply(&mut next)) {
            Err(HubError::Engine(e)) if table.records() => {
                let mut next = table.clone();
                next.stop_recording(format!("the engine failed while journaling: {e}"));
                let done = engine(|| apply(&mut next))??;
                eprintln!(
                    "dominion: game {}'s recording stopped: the engine failed while \
                     journaling ({e}), and not without",
                    table.id()
                );
                Ok((next, done))
            }
            done => Ok((next, done??)),
        }
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
        self.mutate(seat, |next| {
            next.respond(seat.seat, decision, response.clone())
        })
    }

    /// Ends game `id` if a seat's clock has run out, which nothing may then change.
    fn in_time(&self, game: &Hosted, id: &str) -> Result<(), HubError> {
        let Some(flagged) = game.flagged(now_ms()) else {
            return Ok(());
        };
        let seat = Seat {
            game: id.into(),
            seat: flagged,
            model: false,
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
        apply: impl Fn(&mut Table) -> Result<T, AnswerError>,
    ) -> Result<T, HubError> {
        let game = self.game(&seat.game)?;
        self.in_time(&game, &seat.game)?;
        let (answered, ended) = {
            let mut table = lock(&game.table);
            game.turn_of(seat)?;
            let (next, answered) = self.attempt(&table, &apply)?;
            let ended = self.changed(&game, &mut table, next, |_| {})?;
            self.answered(&game, &table, seat.seat);
            (answered, ended)
        };
        self.ended(ended);
        Ok(answered)
    }

    /// Whether answers can be taken back in `game` now: where its rules allow it, never
    /// once someone resigned, and never once a game between accounts is over, as it has
    /// been recorded.
    fn may_undo(&self, game: &Hosted, table: &Table) -> bool {
        may_undo(&lock(&game.meta), table)
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
            game.turn_of(seat)?;
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
        let (next, ()) = self.attempt(table, |next| next.rewind(by, answer))?;
        self.changed(game, table, next, |_| {})
    }

    /// `seat` allows or refuses the undo waiting for it. Once every seat asked has
    /// allowed it, it happens. Whether it happened.
    pub fn answer_undo(&self, seat: &Seat, allow: bool) -> Result<bool, HubError> {
        let game = self.game(&seat.game)?;
        self.in_time(&game, &seat.game)?;
        let ended = {
            let mut table = lock(&game.table);
            game.turn_of(seat)?;
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
        game.turn_of(seat)?;
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
            game.turn_of(seat)?;
            let (next, played) =
                self.attempt(&table, |next| next.play_treasures(seat.seat, decision))?;
            let ended = self.changed(&game, &mut table, next, |_| {})?;
            self.answered(&game, &table, seat.seat);
            (played, ended)
        };
        self.ended(ended);
        Ok(played)
    }

    /// Ends the game with `seat` out of it: `cause` is `resigned`, `abandoned` or
    /// `timeout`. A seat of the server's agents that is abandoned failed to play, which
    /// says nothing of anyone's skill: a rated game is voided.
    pub fn resign(&self, seat: &Seat, cause: &str) -> Result<(), HubError> {
        let game = self.game(&seat.game)?;
        let ended = {
            let mut table = lock(&game.table);
            if cause == "resigned" {
                game.turn_of(seat)?;
            }
            let (next, ()) = self.attempt(&table, |next| next.resign(seat.seat))?;
            let ended = self.changed(&game, &mut table, next, |meta| {
                meta.ended = Some(cause.into());
                let agent = meta.agents.get(seat.seat.index()) == Some(&true);
                if cause == "abandoned" && agent && meta.rated {
                    meta.rated = false;
                    meta.voided = Some(now_ms());
                }
            })?;
            lock(&game.undo).request = None;
            ended
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
                    model: false,
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
                (table.is_over() && !meta.recorded).then(|| {
                    let view = table.view(PlayerId(0));
                    Ended {
                        id: table.id().into(),
                        meta: meta.clone(),
                        seed: table.game().config().seed,
                        setup: crate::setup_cards::setup_cards(&view),
                        info: view.game,
                    }
                })
            })
            .collect()
    }

    /// Notes that game `id` was voided at `at`, if it is still hosted.
    pub fn void(&self, id: &str, at: u64) {
        if let Ok(game) = self.game(id) {
            self.note(&game, |meta, _| {
                meta.rated = false;
                meta.voided = Some(at);
            });
        }
    }

    /// Notes that game `id`'s record has been written.
    pub fn set_recorded(&self, id: &str) {
        if let Ok(game) = self.game(id) {
            self.note(&game, |meta, _| meta.recorded = true);
        }
    }

    /// Changes game `id`'s meta without counting as a change to the game. Not saved, it
    /// is still noted, so that a record isn't written twice.
    fn note(&self, game: &Hosted, f: impl FnOnce(&mut Meta, usize)) {
        let mut table = lock(&game.table);
        let seats = table.names().len();
        let changed = game.changed.load(Ordering::Relaxed);
        let _ = self.commit_meta(game, &mut table, None, |meta, _| f(meta, seats), true);
        game.changed.store(changed, Ordering::Relaxed);
    }

    /// Notes that one of the server's agents started playing `seat` of game `id` afresh,
    /// keeping its archive in `archive`.
    pub fn agent_attempt(
        &self,
        id: &str,
        seat: u8,
        archive: String,
        setup: AgentSetup,
    ) -> Result<(), HubError> {
        let game = self.game(id)?;
        let seat = usize::from(seat);
        if !lock(&game.meta).agents.get(seat).copied().unwrap_or(false) {
            return Err(HubError::BadRequest("no agent plays that seat".into()));
        }
        self.note(&game, |meta, seats| {
            meta.runs.resize(seats.max(seat + 1), None);
            let mut attempt = AgentAttempt {
                started: now_ms(),
                archive,
                error: None,
                setup: None,
            };
            match &mut meta.runs[seat] {
                Some(run) => {
                    attempt.setup = (setup != run.setup).then_some(setup);
                    run.attempts.push(attempt);
                }
                slot => {
                    *slot = Some(AgentRun {
                        setup,
                        attempts: vec![attempt],
                    })
                }
            }
        });
        Ok(())
    }

    /// Notes why the attempt keeping `archive` at `seat` of game `id` stopped.
    pub fn agent_failed(
        &self,
        id: &str,
        seat: u8,
        archive: &str,
        error: &str,
    ) -> Result<(), HubError> {
        let game = self.game(id)?;
        let mut end = error.len().min(MAX_AGENT_ERROR);
        while !error.is_char_boundary(end) {
            end -= 1;
        }
        self.note(&game, |meta, _| {
            let run = meta
                .runs
                .get_mut(usize::from(seat))
                .and_then(Option::as_mut);
            let attempts = run.map_or(&mut [][..], |r| &mut r.attempts[..]);
            if let Some(attempt) = attempts.iter_mut().rev().find(|a| a.archive == archive) {
                attempt.error = Some(error[..end].to_string());
            }
        });
        Ok(())
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
        game.turn_of(seat)?;
        let seen = table
            .view(seat.seat)
            .since_last_decision
            .iter()
            .map(|e| e.seq)
            .max();
        let run = |table: Table| {
            let mut runner = Runner::new(table, seat.seat, false, seen);
            engine(|| runner.run(steps)).map(|stopped| (runner, stopped))
        };
        let (runner, stopped) = match run(table.clone()) {
            // As `attempt`: the engine failing while journaling stops the recording.
            Err(HubError::Engine(e)) if table.records() => {
                let mut unrecorded = table.clone();
                unrecorded.stop_recording(format!("the engine failed while journaling: {e}"));
                run(unrecorded)?
            }
            run => run?,
        };
        let stopped = stopped.map_err(bad)?;
        let ended = if table.decision_id() != runner.table.decision_id()
            || table.is_over() != runner.table.is_over()
        {
            let ended = self.changed(&game, &mut table, runner.table, |_| {})?;
            self.answered(&game, &table, seat.seat);
            ended
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
    /// whether it is so. A model whose seat a person holds waits until it owes them a note.
    pub async fn wait(&self, seat: &Seat, timeout: Duration) -> Result<bool, HubError> {
        let game = self.game(&seat.game)?;
        let _connected = self.connect(seat)?;
        let mut rx = game.version.subscribe();
        let ready = || {
            let table = lock(&game.table);
            table.is_over()
                || match game.turn_of(seat) {
                    Ok(()) => {
                        table.deciding() == Some(seat.seat) || lock(&game.undo).waits_for(seat.seat)
                    }
                    Err(_) => game.advice_wanted(&table, seat),
                }
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

    /// Game `id`'s recorded event stream, the full one (every hidden card, every
    /// shuffle's order), in the encoding it was written in. Only once the game can no
    /// longer change: it is over, and retired or no undo is allowed ([`Hub::may_undo`]).
    /// Before that it would show what is yet to come, or what an undo would let a seat
    /// play again knowing.
    pub fn recording(&self, id: &str) -> Result<(Encoding, Vec<u8>), HubError> {
        let hosted = self.game(id).ok();
        let game = match &hosted {
            Some(game) => game.clone(),
            None => self.read(id)?,
        };
        let table = lock(&game.table);
        if !table.is_over() {
            return Err(HubError::Conflict(format!(
                "game {id} is in progress: its recorded stream shows every hidden card and \
                 every shuffle's order, so it is served once the game is over"
            )));
        }
        if hosted.is_some() && self.may_undo(&game, &table) {
            return Err(HubError::Conflict(format!(
                "game {id} can still be undone: its recorded stream shows every hidden card \
                 and every shuffle's order, so it is served once the game can't go back \
                 (when it is retired)"
            )));
        }
        self.stream_bytes(&game, &table)
    }

    /// The bytes of `table`'s stream as it stands: its file up to the events written, then
    /// those still to write. Checked to be exactly its stream.
    fn stream_bytes(&self, game: &Hosted, table: &Table) -> Result<(Encoding, Vec<u8>), HubError> {
        let id = table.id();
        let unrecorded = || HubError::NotFound(format!("game {id} has no recorded stream"));
        let write = |e: io::Error| HubError::Engine(format!("game {id}'s recorded stream: {e}"));
        let at = lock(&game.stream).at;
        if let (Stream::Reconstructed(encoding, _), Some(store)) = (at, &self.store) {
            let (path, _) = store.stream_file(id).ok_or_else(unrecorded)?;
            let bytes = fs::read(path).map_err(write)?;
            check_reconstructed(encoding, &bytes, table).map_err(|e| match e {
                Unfit::Stale(e) => {
                    HubError::NotFound(format!("game {id}'s reconstructed stream is stale: {e}"))
                }
                Unfit::Broken(e) => {
                    HubError::Engine(format!("game {id}'s reconstructed stream: {e}"))
                }
            })?;
            return Ok((encoding, bytes));
        }
        if let Some(why) = table.recording_stopped() {
            return Err(HubError::NotFound(format!(
                "game {id}'s recording stopped ({why}): its stream is incomplete"
            )));
        }
        let encoding = match at {
            Stream::File(encoding, len) => Some((encoding, len)),
            Stream::New => None,
            Stream::Off | Stream::Reconstructed(..) => return Err(unrecorded()),
        };
        // After an undo whose stream hasn't replaced the file yet, the events still to
        // write are the whole stream.
        let replacing = table.unwritten().first().is_some_and(|e| e.seq == 0);
        let (encoding, mut bytes) = match (encoding, &self.store) {
            (Some((encoding, len)), Some(store)) if !replacing => {
                let (path, _) = store.stream_file(id).ok_or_else(unrecorded)?;
                let mut bytes = fs::read(path).map_err(write)?;
                bytes.truncate(len as usize);
                (encoding, bytes)
            }
            (encoding, _) => {
                let encoding = encoding.map_or_else(|| *lock(&self.encoding), |(e, _)| e);
                let mut bytes = vec![];
                encoding
                    .write(&mut bytes, &recording::header(id))
                    .map_err(write)?;
                (encoding, bytes)
            }
        };
        for event in table.unwritten() {
            encoding.write(&mut bytes, event).map_err(write)?;
        }
        check_stream(encoding, &bytes, table)
            .map_err(|e| HubError::Engine(format!("game {id}'s recorded stream: {e}")))?;
        Ok((encoding, bytes))
    }
}

/// Checks that `bytes` are exactly the stream `table` recorded: its header, every event in
/// order and nothing after them, folding to the table its recorder keeps.
fn check_stream(encoding: Encoding, bytes: &[u8], table: &Table) -> Result<(), String> {
    let read = recording::read(encoding, bytes, None)?;
    if read.header.game.as_deref() != Some(table.id()) {
        return Err("its header names another game".into());
    }
    let n = read.events.len();
    if Some(n as u64) != table.recorded() || read.len_of(n) != Some(bytes.len()) {
        return Err(format!(
            "it holds {n} events in {} bytes, not the {:?} recorded",
            bytes.len(),
            table.recorded()
        ));
    }
    if !folds_to(read.events.iter(), table) {
        return Err("its events are not the game's".into());
    }
    Ok(())
}

/// Checks that `bytes` are a whole stream `dominion games backfill` rebuilt for `table` as
/// it stands: its header says so, names the game and the history it was built from, which
/// is exactly the table's ([`Table::history`]: its answers and undos), every event folds in
/// order with nothing after them, its setup has the game's seats and seed, and it ends as
/// the game stands (its `game-end` the game's outcome, or none while the game is in
/// progress). The backfill checked it against the game event for event, and a game is
/// determined by its config and answers, so this needs no engine: it holds the file to
/// being that stream still. A stream of another history is [`Unfit::Stale`].
fn check_reconstructed(encoding: Encoding, bytes: &[u8], table: &Table) -> Result<(), Unfit> {
    use dominion_protocol::events::What;
    let read = recording::read(encoding, bytes, None).map_err(Unfit::Broken)?;
    if read.header.game.as_deref() != Some(table.id()) {
        return Err(Unfit::Broken("its header names another game".into()));
    }
    let Some(reconstructed) = &read.header.reconstructed else {
        return Err(Unfit::Broken(
            "its header doesn't say it was reconstructed".into(),
        ));
    };
    let history = table.history();
    match &reconstructed.history {
        None => {
            return Err(Unfit::Stale(
                "its header doesn't say which history of the game it was built from".into(),
            ));
        }
        Some(built) if *built != history => {
            let counts = |h: &History| {
                format!(
                    "{} after {}",
                    dominion_engine::amount(h.answers as i64, "answer", "answers"),
                    dominion_engine::amount(h.undos as i64, "undo", "undos"),
                )
            };
            return Err(Unfit::Stale(match counts(built) == counts(&history) {
                true => format!(
                    "it was built from {}, as many as the game has, but other ones",
                    counts(built)
                ),
                false => format!(
                    "it was built from {}, and the game now has {}",
                    counts(built),
                    counts(&history)
                ),
            }));
        }
        Some(_) => {}
    }
    let broken = |why: &str| Err(Unfit::Broken(why.into()));
    if read.len_of(read.events.len()) != Some(bytes.len()) {
        return broken("its last record is cut short");
    }
    let mut folded = dominion_protocol::events::Table::default();
    for event in &read.events {
        folded
            .apply(event)
            .map_err(|e| Unfit::Broken(format!("event {}: {e}", event.seq)))?;
    }
    let config = table.game().config();
    match read.events.first().map(|e| &e.what) {
        Some(What::Setup(setup))
            if setup.seats == table.names()
                && setup.config.as_ref().and_then(|c| c.seed) == Some(config.seed) => {}
        _ => return broken("its setup is not the game's"),
    }
    let end = match read.events.last().map(|e| &e.what) {
        Some(What::GameEnd(end)) => Some(end),
        _ => None,
    };
    let same = match (table.game().outcome(), end) {
        (None, None) => true,
        (Some(o), Some(end)) => {
            (end.reason == "resigned") == (o.reason == dominion_engine::EndReason::Resigned)
                && end.result.len() == o.scores.len()
                && end.result.iter().all(|r| {
                    let i = r.seat as usize;
                    o.scores.get(i) == Some(&r.vp)
                        && o.turns.get(i) == Some(&r.turns)
                        && o.resigned.contains(&PlayerId(r.seat)) == r.resigned
                })
        }
        _ => false,
    };
    if !same {
        return broken("it doesn't end as the game stands");
    }
    Ok(())
}

/// Whether game `table` may still be taken back: its rules allow undo, it kept every
/// answer, nobody resigned, and it is in progress or a game made directly.
fn may_undo(meta: &Meta, table: &Table) -> bool {
    meta.undo != UndoRule::Off
        && table.can_rewind()
        && !(table.is_over() && meta.source != Source::Direct)
}

/// Whether hosted game `table` can still change: in progress, or able to be undone.
pub(crate) fn can_change(meta: &Meta, table: &Table) -> bool {
    !table.is_over() || may_undo(meta, table)
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
            model: false,
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
    fn a_rated_game_an_agent_abandons_is_voided() {
        let hub = Hub::default();
        let req = NewGame {
            players: 2,
            kingdom: vec!["Village".into()],
            seed: Some(1),
            ..NewGame::default()
        };
        let after = |rated: bool, seat: u8, cause: &str| {
            let meta = Meta {
                rated,
                agents: vec![false, true],
                ..Meta::default()
            };
            let game = (hub.host(config(&req).unwrap(), names(&req).unwrap(), meta)).unwrap();
            let seat = hub.claim(&game.game_id, &game.seats[usize::from(seat)].token);
            hub.resign(&seat.unwrap(), cause).unwrap();
            let meta = hub.meta(&game.game_id).unwrap();
            (meta.rated, meta.voided.is_some())
        };
        assert_eq!(after(true, 1, "abandoned"), (false, true));
        assert_eq!(after(true, 0, "abandoned"), (true, false));
        assert_eq!(after(true, 1, "resigned"), (true, false));
        assert_eq!(after(true, 1, "timeout"), (true, false));
        assert_eq!(after(false, 1, "abandoned"), (false, false));
    }

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

#[cfg(test)]
mod recording_safety_tests {
    use super::*;
    use std::sync::atomic::Ordering::SeqCst;

    const KEEP: Duration = Duration::from_secs(3600);

    fn host(hub: &Hub, meta: Meta) -> Vec<Seat> {
        let req = NewGame {
            players: 2,
            kingdom: ["Village", "Smithy", "Militia", "Moat", "Market"]
                .map(String::from)
                .to_vec(),
            seed: Some(5),
            turn_limit: Some(6),
            ..NewGame::default()
        };
        let created = hub.host(config(&req).unwrap(), vec![], meta).unwrap();
        (created.seats.iter())
            .map(|s| hub.claim(&created.game_id, &s.token).unwrap())
            .collect()
    }

    fn table(hub: &Hub) -> Table {
        lock(&hub.game("g1").unwrap().table).clone()
    }

    fn store(hub: &Hub) -> Arc<Store> {
        hub.store().unwrap()
    }

    /// The deciding seat's answer: its first option, or its last.
    fn next_answer(hub: &Hub, seats: &[Seat], last: bool) -> Option<(Seat, Answer)> {
        let (seat, d) = (seats.iter()).find_map(|s| Some((s, hub.view(s).ok()?.0.decision?)))?;
        let o = if last {
            d.options.last()
        } else {
            d.options.first()
        };
        Some((seat.clone(), Answer::indices(&d.id, vec![o?.index])))
    }

    fn advance(hub: &Hub, seats: &[Seat], n: usize) {
        for _ in 0..n {
            let Some((seat, answer)) = next_answer(hub, seats, false) else {
                return;
            };
            hub.answer(&seat, &answer).unwrap();
        }
    }

    /// The table saved in `dir`.
    fn saved(dir: &Path) -> Table {
        load(&dir.join("g1.json"), "g1").unwrap().0
    }

    /// Game g1 in memory, on disk and in its stream agree.
    fn agree(hub: &Hub, dir: &Path) {
        let live = table(hub);
        let disk = saved(dir);
        assert_eq!(live.decision_id(), disk.decision_id());
        assert_eq!(live.answers(), disk.answers());
        assert_eq!(live.game().log(), disk.game().log());
        assert_eq!(live.recording_stopped(), disk.recording_stopped());
        assert_eq!(live.records(), live.game().journaled());
        if live.records() {
            assert!(live.unwritten().is_empty());
            let stream = stream_state(store(hub).dir(), &disk).unwrap();
            assert!(matches!(stream, Stream::File(..)));
        } else {
            assert!(!dir.join("g1.events.jsonl").exists());
        }
    }

    fn stopped(hub: &Hub, dir: &Path) {
        let live = table(hub);
        assert!(live.recording_stopped().is_some());
        assert!(!live.game().journaled());
        assert!(dir.join("g1.incomplete.events.jsonl").exists());
        assert!(lock(&hub.game("g1").unwrap().stream).at == Stream::Off);
        agree(hub, dir);
    }

    #[test]
    fn a_recorder_fault_stops_recording_and_never_refuses_the_answer() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        let (hub, _) = Hub::open(dir, KEEP).unwrap();
        let seats = host(&hub, Meta::default());
        advance(&hub, &seats, 3);
        let before = table(&hub);
        lock(&hub.game("g1").unwrap().table).inject_recording_fault(0);
        let (seat, answer) = next_answer(&hub, &seats, false).unwrap();
        hub.answer(&seat, &answer).unwrap();
        assert_eq!(table(&hub).answers(), before.answers() + 1);
        assert_ne!(table(&hub).decision_id(), before.decision_id());
        stopped(&hub, dir);
        // From then on it plays, undoes and restarts without recording.
        advance(&hub, &seats, 4);
        assert!(hub.undo(&seats[0], None).unwrap());
        stopped(&hub, dir);
        drop(hub);
        let (hub, loaded) = Hub::open(dir, KEEP).unwrap();
        assert_eq!(loaded.unrecorded.len(), 1);
        advance(&hub, &seats_of(&hub, &before), 10_000);
        stopped(&hub, dir);
    }

    fn seats_of(hub: &Hub, table: &Table) -> Vec<Seat> {
        (0..table.names().len())
            .map(|i| Seat {
                game: "g1".into(),
                seat: PlayerId(i as u8),
                model: false,
            })
            .inspect(|s| assert!(hub.game(&s.game).is_ok()))
            .collect()
    }

    #[test]
    fn a_recorder_fault_inside_steps_commits_every_step_whole() {
        for fault in [0, 1] {
            let tmp = tempfile::tempdir().unwrap();
            let (hub, _) = Hub::open(tmp.path(), KEEP).unwrap();
            let twin_tmp = tempfile::tempdir().unwrap();
            let (twin, _) = Hub::open(twin_tmp.path(), KEEP).unwrap();
            let seats = host(&hub, Meta::default());
            let twin_seats = host(&twin, Meta::default());
            let seat = table(&hub).deciding().unwrap();
            lock(&hub.game("g1").unwrap().table).inject_recording_fault(fault);
            let steps = "treasures; next";
            let done = hub.steps(&seats[seat.index()], steps).unwrap();
            let twin_done = twin.steps(&twin_seats[seat.index()], steps).unwrap();
            assert_eq!(done.stopped, twin_done.stopped, "fault {fault}");
            let (live, kept) = (table(&hub), table(&twin));
            assert!(kept.answers() >= 2 && kept.records());
            assert_eq!(live.answers(), kept.answers(), "fault {fault}");
            assert_eq!(live.decision_id(), kept.decision_id(), "fault {fault}");
            assert_eq!(live.game().log(), kept.game().log(), "fault {fault}");
            assert_eq!(live.game().pending(), kept.game().pending());
            assert_eq!(
                live.view(PlayerId(0)).game,
                kept.view(PlayerId(0)).game,
                "fault {fault}"
            );
            stopped(&hub, tmp.path());
            advance(&hub, &seats, 10_000);
            advance(&twin, &twin_seats, 10_000);
            assert_eq!(table(&hub).game().log(), table(&twin).game().log());
        }
    }

    #[test]
    fn a_change_that_cant_be_saved_is_not_made() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        let (hub, _) = Hub::open(dir, KEEP).unwrap();
        let seats = host(&hub, Meta::default());
        advance(&hub, &seats, 3);
        let files = || {
            (
                fs::read(dir.join("g1.json")).unwrap(),
                fs::read(dir.join("g1.events.jsonl")).unwrap(),
            )
        };
        let (before, on_disk) = (table(&hub), files());
        let version = hub.version("g1").unwrap();
        let (seat, answer) = next_answer(&hub, &seats, false).unwrap();
        store(&hub).fail_saves.store(1, SeqCst);
        assert!(matches!(
            hub.answer(&seat, &answer),
            Err(HubError::Unsaved(_))
        ));
        assert_eq!(hub.version("g1").unwrap(), version);
        assert_eq!(table(&hub).decision_id(), before.decision_id());
        assert_eq!(table(&hub).game().log(), before.game().log());
        assert_eq!(files(), on_disk);
        hub.answer(&seat, &answer).unwrap();
        agree(&hub, dir);
        assert!(table(&hub).records());
    }

    #[test]
    fn an_undo_whose_snapshot_fails_leaves_the_stream_alone() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        let (hub, _) = Hub::open(dir, KEEP).unwrap();
        let seats = host(&hub, Meta::default());
        advance(&hub, &seats, 12);
        let stream = fs::read(dir.join("g1.events.jsonl")).unwrap();
        store(&hub).fail_saves.store(1, SeqCst);
        assert!(matches!(
            hub.undo(&seats[0], None),
            Err(HubError::Unsaved(_))
        ));
        assert_eq!(fs::read(dir.join("g1.events.jsonl")).unwrap(), stream);
        agree(&hub, dir);
        assert!(hub.undo(&seats[0], None).unwrap());
        agree(&hub, dir);
        advance(&hub, &seats, 10_000);
        agree(&hub, dir);
    }

    #[test]
    fn an_undo_whose_stream_fails_to_replace_the_file_is_never_served_mixed() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        let (hub, _) = Hub::open(dir, KEEP).unwrap();
        let seats = host(&hub, Meta::default());
        advance(&hub, &seats, 12);
        let bytes = || {
            let game = hub.game("g1").unwrap();
            let table = lock(&game.table);
            hub.stream_bytes(&game, &table).unwrap().1
        };
        let taken_back = fs::read(dir.join("g1.events.jsonl")).unwrap();
        // The replacement and the next append fail: the file still holds the timeline
        // taken back, the table the stream from event 0.
        store(&hub).fail_streams.store(2, SeqCst);
        assert!(hub.undo(&seats[0], None).unwrap());
        advance(&hub, &seats, 1);
        let live = table(&hub);
        assert!(live.records() && live.unwritten()[0].seq == 0);
        assert_eq!(fs::read(dir.join("g1.events.jsonl")).unwrap(), taken_back);
        let stream = bytes();
        let read = recording::read(Encoding::JsonLines, &stream, None).unwrap();
        assert!(read.events == live.unwritten());
        // A restart before the file is replaced finds the snapshot's stream still to write.
        drop(hub);
        let (hub, loaded) = Hub::open(dir, KEEP).unwrap();
        assert_eq!(loaded.unrecorded.len(), 0);
        let game = hub.game("g1").unwrap();
        assert_eq!(
            hub.stream_bytes(&game, &lock(&game.table)).unwrap().1,
            stream
        );
        let seats = seats_of(&hub, &live);
        advance(&hub, &seats, 1);
        agree(&hub, dir);
        let read = recording::read(
            Encoding::JsonLines,
            &fs::read(dir.join("g1.events.jsonl")).unwrap(),
            None,
        )
        .unwrap();
        assert!(read.events.len() > live.unwritten().len());
        assert!(read.events.starts_with(live.unwritten()));
    }

    #[test]
    fn appends_that_keep_failing_stop_the_recording() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        let (hub, _) = Hub::open(dir, KEEP).unwrap();
        let seats = host(&hub, Meta::default());
        advance(&hub, &seats, 3);
        store(&hub).fail_streams.store(STREAM_FAILURES - 1, SeqCst);
        advance(&hub, &seats, (STREAM_FAILURES - 1) as usize);
        // Failures that pass leave the backlog to the next write.
        assert!(!table(&hub).unwritten().is_empty());
        advance(&hub, &seats, 1);
        agree(&hub, dir);
        store(&hub).fail_streams.store(STREAM_FAILURES, SeqCst);
        advance(&hub, &seats, STREAM_FAILURES as usize);
        stopped(&hub, dir);
        advance(&hub, &seats, 10_000);
        stopped(&hub, dir);
    }
}
