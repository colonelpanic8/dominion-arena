//! The games a server hosts, and the seat tokens that let a client act for a seat.

use crate::store::Store;
use dominion_agent::{AgentView, Answer, AnswerError, Answered, Table, WayChoice};
use dominion_engine::{
    Card, DealRules, Game, GameConfig, LandscapeSlot, PlayerId, SLOT_KINDS, Set,
};
pub use dominion_protocol::api::{
    Created, DealRequest, Dealt, GameSummary, NewGame, SeatToken, SeatView,
};
use dominion_protocol::lobby::{GameMeta, MyGame, Source, UserRef};
use dominion_protocol::view::GameInfo;
use rand::Rng;
use serde::{Deserialize, Serialize};
use std::any::Any;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs;
use std::io;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::watch;

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
    /// Answers can't be taken back: an undo shows a player what would have happened.
    pub no_undo: bool,
    /// The lobby table that started it.
    pub table: Option<String>,
    /// Milliseconds since the Unix epoch.
    pub started: u64,
    /// Why it ended before its end: `resigned` or `abandoned`.
    pub ended: Option<String>,
    pub ended_at: Option<u64>,
    /// Its record is in the history.
    pub recorded: bool,
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

/// A seat that only just became the deciding one is given this long to come back.
const AWAY_GRACE_MS: u64 = 30_000;
/// A seat counts as connected this long after its last call.
const CONNECTED_MS: u64 = 20_000;

struct Hosted {
    table: Mutex<Table>,
    tokens: Vec<String>,
    /// Locked after `table`, never before.
    meta: Mutex<Meta>,
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
}

impl Hosted {
    fn new(table: Table, tokens: Vec<String>, meta: Meta, changed: u64) -> Hosted {
        let now = now_ms();
        let seats = tokens.len();
        Hosted {
            table: Mutex::new(table),
            meta: Mutex::new(meta),
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
    meta: Meta,
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

/// The most games in progress unless [`Hub::set_max_games`] says otherwise.
pub const DEFAULT_MAX_GAMES: usize = 1000;

pub struct Hub {
    index: Mutex<Index>,
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
fn names(req: &NewGame) -> Result<Vec<String>, HubError> {
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

fn load(path: &Path, stem: &str) -> Result<Saved, String> {
    let bytes = fs::read(path).map_err(|e| e.to_string())?;
    let mut saved: Saved = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    if saved.table.id() != stem {
        return Err(format!("holds game {}, not {stem}", saved.table.id()));
    }
    let seats = saved.table.names().len();
    if saved.tokens.len() != seats {
        return Err(format!(
            "has {} seat tokens for {seats} seats",
            saved.tokens.len(),
        ));
    }
    saved.meta.owners.resize(seats, None);
    Ok(saved)
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
    /// games whose last change is older than `keep_finished` are deleted. Fails if `dir`
    /// can't be written to or another server has it open.
    pub fn open(dir: &Path, keep_finished: Duration) -> io::Result<(Hub, Loaded)> {
        let store = Store::open(dir)?;
        let mut loaded = Loaded::default();
        let mut index = Index::default();
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
            let saved = match load(&path, stem) {
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
            if saved.table.is_over() && age >= keep_finished {
                fs::remove_file(&path)?;
                loaded.expired += 1;
                continue;
            }
            let changed = now_ms().saturating_sub(age.as_millis() as u64);
            index.insert(
                stem,
                Hosted::new(saved.table, saved.tokens, saved.meta, changed),
            );
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
        let config = config(req)?;
        let names = names(req)?;
        let meta = Meta {
            watchable: req.watchable.unwrap_or(false),
            ..Meta::default()
        };
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
        meta.started = now_ms();
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

    /// The seat `token` claims in game `id`.
    pub fn claim(&self, id: &str, token: &str) -> Result<Seat, HubError> {
        let index = lock(&self.index);
        let Some(game) = index.games.get(id) else {
            return Err(HubError::UnknownGame(id.into()));
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
        let seats = index.owned.get(user).into_iter().flatten();
        seats.filter(|s| s.game == id).cloned().collect()
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

    /// Whether account `user` plays in a timed game that is still running.
    pub fn in_timed_game(&self, user: &str) -> bool {
        let games: Vec<Arc<Hosted>> = {
            let index = lock(&self.index);
            let seats = index.owned.get(user).into_iter().flatten();
            seats
                .filter_map(|s| index.games.get(&s.game).cloned())
                .collect()
        };
        games
            .iter()
            .any(|g| !lock(&g.table).is_over() && lock(&g.meta).timed)
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

    pub fn meta(&self, id: &str) -> Result<Meta, HubError> {
        Ok(lock(&self.game(id)?.meta).clone())
    }

    /// `seat`'s view, its text rendering, and `Table::undo_note`, read together.
    pub fn view(&self, seat: &Seat) -> Result<(AgentView, String, Option<String>), HubError> {
        let game = self.game(&seat.game)?;
        let table = lock(&game.table);
        engine(|| {
            let view = table.view(seat.seat);
            let text = dominion_agent::text::render(&view);
            (view, text, table.undo_note(seat.seat))
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
            watchers: game.watchers.load(Ordering::Relaxed),
            ended: meta.ended.clone(),
            now,
        };
        (shown, meta.owners.clone())
    }

    /// `seat`'s view, with the log from `log_from` on if given, the game's version and
    /// the seat deciding, all read together.
    pub fn seat_view(&self, seat: &Seat, log_from: Option<usize>) -> Result<SeatView, HubError> {
        let game = self.game(&seat.game)?;
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
                can_undo: table.can_undo(seat.seat) && self.may_undo(&game, &table),
                meta,
            };
            (seat_view, owners)
        };
        seat_view.meta.users = self.users(&owners);
        Ok(seat_view)
    }

    /// Game `id` as someone watching sees it, with the log from `log_from` on.
    pub fn watch_view(&self, id: &str, log_from: usize) -> Result<SeatView, HubError> {
        let game = self.game(id)?;
        let refused = || HubError::Forbidden(format!("game {id} can't be watched"));
        let (mut seat_view, owners) = {
            let table = lock(&game.table);
            if !lock(&game.meta).watchable {
                return Err(refused());
            }
            let view = engine(|| table.watch_view(log_from))?.ok_or_else(refused)?;
            let extras = engine(|| table.watch_extras())?.unwrap_or_default();
            let (meta, owners) = self.game_meta(&game, &table);
            let seat_view = SeatView {
                view,
                version: *game.version.borrow(),
                deciding: table.deciding().map(|p| p.0),
                table: extras,
                undos: table.undos(),
                can_undo: false,
                meta,
            };
            (seat_view, owners)
        };
        seat_view.meta.users = self.users(&owners);
        Ok(seat_view)
    }

    /// Game `id`'s version, which every change bumps, and its chat's counter.
    pub fn subscribe(&self, id: &str) -> Result<Feed, HubError> {
        let game = self.game(id)?;
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
        let game = self.game(&seat.game)?;
        game.streams[seat.seat.index()].fetch_add(1, Ordering::Relaxed);
        Ok(Connection {
            game,
            seat: seat.seat.index(),
        })
    }

    /// Counts someone watching game `id` until the result is dropped.
    pub fn watching(&self, id: &str) -> Result<Watching, HubError> {
        let game = self.game(id)?;
        game.watchers.fetch_add(1, Ordering::Relaxed);
        Ok(Watching(game))
    }

    /// The name of `seat`.
    pub fn seat_name(&self, seat: &Seat) -> Result<String, HubError> {
        let game = self.game(&seat.game)?;
        let table = lock(&game.table);
        Ok(table.names()[seat.seat.index()].clone())
    }

    /// After a change to `game`, made with `table` locked: saves it and wakes its
    /// followers. What to tell [`Hub::ended`] once the lock is released.
    fn changed(&self, game: &Hosted, table: &Table) -> Option<Ended> {
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
        game.since.store(now_ms(), Ordering::Relaxed);
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
        let game = self.game(&seat.game)?;
        let (answered, ended) = {
            let mut table = lock(&game.table);
            // Played on a copy, so an answer the engine fails on leaves the game as it was.
            let mut next = table.clone();
            let answered = engine(|| next.answer_with(seat.seat, answer, way))??;
            *table = next;
            (answered, self.changed(&game, &table))
        };
        self.ended(ended);
        Ok(answered)
    }

    /// Whether answers can be taken back in `game`: where its rules allow it, and never
    /// once a game between accounts is over, as it has been recorded.
    fn may_undo(&self, game: &Hosted, table: &Table) -> bool {
        let meta = lock(&game.meta);
        !meta.no_undo && !(table.is_over() && meta.source != Source::Direct)
    }

    /// Takes back `seat`'s last answer (`Table::undo`).
    pub fn undo(&self, seat: &Seat) -> Result<(), HubError> {
        let game = self.game(&seat.game)?;
        let ended = {
            let mut table = lock(&game.table);
            if !self.may_undo(&game, &table) {
                return Err(HubError::Forbidden(
                    "answers can't be taken back in this game".into(),
                ));
            }
            if !table.undo(seat.seat) {
                return Err(HubError::Answer(AnswerError::CannotUndo));
            }
            self.changed(&game, &table)
        };
        self.ended(ended);
        Ok(())
    }

    /// Plays the Treasures `seat` could play in its open Buy phase decision `decision`
    /// ([`Table::play_treasures`]). How many it played.
    pub fn play_treasures(&self, seat: &Seat, decision: &str) -> Result<u32, HubError> {
        let game = self.game(&seat.game)?;
        let (played, ended) = {
            let mut table = lock(&game.table);
            let mut next = table.clone();
            let played = engine(|| next.play_treasures(seat.seat, decision))??;
            *table = next;
            (played, self.changed(&game, &table))
        };
        self.ended(ended);
        Ok(played)
    }

    /// Ends the game with `seat` out of it: `cause` is `resigned` or `abandoned`.
    pub fn resign(&self, seat: &Seat, cause: &str) -> Result<(), HubError> {
        let game = self.game(&seat.game)?;
        let ended = {
            let mut table = lock(&game.table);
            let mut next = table.clone();
            engine(|| next.resign(seat.seat))??;
            *table = next;
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

    /// The deciding seats of timed games whose time is up.
    pub fn overdue(&self) -> Vec<Seat> {
        let timing = *lock(&self.timing);
        let games: Vec<Arc<Hosted>> = lock(&self.index).games.values().cloned().collect();
        let now = now_ms();
        games
            .iter()
            .filter(|g| lock(&g.meta).timed)
            .filter_map(|g| {
                let table = lock(&g.table);
                let deciding = table.deciding()?;
                (g.forfeit_at(deciding, timing)? <= now).then(|| Seat {
                    game: table.id().into(),
                    seat: deciding,
                })
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

    /// Deletes the finished games that last changed more than `keep_finished` ago, and
    /// the unfinished ones nobody has touched for `keep_idle`. How many.
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
            let last = (game.seen.iter())
                .map(|s| s.load(Ordering::Relaxed))
                .chain([game.changed.load(Ordering::Relaxed)])
                .max()
                .unwrap_or(0);
            let keep = if over { keep_finished } else { keep_idle };
            let connected = game.streams.iter().any(|s| s.load(Ordering::Relaxed) > 0);
            if (over || !connected) && u128::from(now.saturating_sub(last)) >= keep.as_millis() {
                lock(&self.index).remove(&id);
                if let Some(store) = &self.store
                    && let Err(e) = store.remove(&id)
                {
                    eprintln!("dominion: deleting game {id}: {e}");
                }
                expired += 1;
            }
        }
        expired
    }

    /// Waits until `seat` must decide or the game is over, for at most `timeout`. Returns
    /// whether it is so.
    pub async fn wait(&self, seat: &Seat, timeout: Duration) -> Result<bool, HubError> {
        let game = self.game(&seat.game)?;
        let _connected = self.connect(seat)?;
        let mut rx = game.version.subscribe();
        let ready = || {
            let table = lock(&game.table);
            table.is_over() || table.deciding() == Some(seat.seat)
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
