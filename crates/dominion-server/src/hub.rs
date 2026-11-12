//! The games a server hosts, and the seat tokens that let a client act for a seat.

use dominion_agent::{AgentView, Answer, AnswerError, Answered, Table, WayChoice};
use dominion_engine::{Card, GameConfig, PlayerId, Set};
use rand::Rng;
use serde::{Deserialize, Serialize};
use std::any::Any;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs;
use std::io::{self, BufWriter, Write};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, SystemTime};
use tokio::sync::watch;

/// What a client asks for when creating a game.
#[derive(Clone, Debug, Default, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NewGame {
    /// Number of players, 2 to 6.
    pub players: u8,
    /// Seat names, in turn order (default P1, P2, ...). Each is at most 32 characters on
    /// one line, and no two seats share a name.
    #[serde(default)]
    pub names: Vec<String>,
    /// Kingdom piles by name. Without it, a random kingdom is dealt from `sets`.
    #[serde(default)]
    pub kingdom: Vec<String>,
    /// Events, Landmarks, Projects, Ways, ... by name. With a random kingdom they replace
    /// the landscapes it would deal.
    #[serde(default)]
    pub landscapes: Vec<String>,
    /// Sets to deal a random kingdom from (default: every released set). Not with
    /// `kingdom`.
    #[serde(default)]
    pub sets: Vec<String>,
    pub seed: Option<u64>,
    /// End the game after this many turns in total.
    pub turn_limit: Option<u32>,
    pub colonies: Option<bool>,
    pub shelters: Option<bool>,
}

#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct SeatToken {
    pub seat: u8,
    pub name: String,
    pub token: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Created {
    pub game_id: String,
    pub seats: Vec<SeatToken>,
}

#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct GameSummary {
    pub game_id: String,
    pub names: Vec<String>,
    pub running: bool,
    /// The seat that must decide now.
    pub deciding: Option<u8>,
    pub kingdom: Vec<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum HubError {
    #[error("{0}")]
    BadRequest(String),
    #[error("unknown game {0}")]
    UnknownGame(String),
    #[error("that token is not a seat token of game {0}")]
    BadToken(String),
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
fn engine<T>(f: impl FnOnce() -> T) -> Result<T, HubError> {
    catch_unwind(AssertUnwindSafe(f)).map_err(|p| HubError::Engine(panic_message(&*p)))
}

/// Tables are only ever replaced whole, so one is consistent even behind a poisoned lock.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

struct Hosted {
    table: Mutex<Table>,
    tokens: Vec<String>,
    /// Bumped after every answer, for waiters.
    version: watch::Sender<u64>,
}

impl Hosted {
    fn new(table: Table, tokens: Vec<String>) -> Hosted {
        Hosted {
            table: Mutex::new(table),
            tokens,
            version: watch::channel(0).0,
        }
    }
}

/// A seat a token claims.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Seat {
    pub game: String,
    pub seat: PlayerId,
}

#[derive(Serialize)]
struct SavedRef<'a> {
    table: &'a Table,
    tokens: &'a [String],
}

#[derive(Deserialize)]
struct Saved {
    table: Table,
    tokens: Vec<String>,
}

/// A directory holding one `<game id>.json` per game, rewritten after every change.
struct Store {
    dir: PathBuf,
    /// Held locked while the directory is in use, so two servers never share it.
    _lock: fs::File,
}

impl Store {
    fn open(dir: &Path) -> io::Result<Store> {
        let mut builder = fs::DirBuilder::new();
        builder.recursive(true);
        #[cfg(unix)]
        std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
        builder.create(dir)?;
        let lock = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .open(dir.join(".lock"))?;
        lock.try_lock().map_err(|e| match e {
            fs::TryLockError::WouldBlock => io::Error::new(
                io::ErrorKind::ResourceBusy,
                "another dominion server is using this directory",
            ),
            fs::TryLockError::Error(e) => e,
        })?;
        // Fail now rather than at the first save.
        let probe = dir.join(".probe.tmp");
        fs::write(&probe, b"")?;
        fs::remove_file(&probe)?;
        Ok(Store {
            dir: dir.to_path_buf(),
            _lock: lock,
        })
    }

    fn save(&self, table: &Table, tokens: &[String]) -> io::Result<()> {
        let id = table.id();
        let tmp = self.dir.join(format!(".{id}.json.tmp"));
        let mut options = fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
        let file = options.open(&tmp)?;
        let mut out = BufWriter::new(&file);
        serde_json::to_writer(&mut out, &SavedRef { table, tokens })?;
        out.flush()?;
        drop(out);
        file.sync_all()?;
        fs::rename(&tmp, self.dir.join(format!("{id}.json")))
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
}

#[derive(Default)]
struct Index {
    games: BTreeMap<String, Arc<Hosted>>,
    seats: HashMap<String, Seat>,
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
            self.seats.insert(token.clone(), seat);
        }
        self.games.insert(id.into(), Arc::new(hosted));
    }
}

#[derive(Default)]
pub struct Hub {
    index: Mutex<Index>,
    store: Option<Store>,
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

const MAX_NAME_CHARS: usize = 32;

/// Every seat's name: the ones given, then the defaults. Names go into every client's
/// text, so each must be one printable line, and seats must be told apart by them.
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
            Some(given) => {
                let name = given.trim();
                if name.is_empty() {
                    return Err(bad(format!("seat {i}'s name is empty")));
                }
                if name.chars().count() > MAX_NAME_CHARS {
                    return Err(bad(format!(
                        "seat {i}'s name is longer than {MAX_NAME_CHARS} characters"
                    )));
                }
                if name
                    .chars()
                    .any(|c| c.is_control() || matches!(c, '\u{2028}' | '\u{2029}'))
                {
                    return Err(bad(format!(
                        "seat {i}'s name has a line break or control character"
                    )));
                }
                name.to_string()
            }
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
    let saved: Saved = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    if saved.table.id() != stem {
        return Err(format!("holds game {}, not {stem}", saved.table.id()));
    }
    if saved.tokens.len() != saved.table.names().len() {
        return Err(format!(
            "has {} seat tokens for {} seats",
            saved.tokens.len(),
            saved.table.names().len()
        ));
    }
    Ok(saved)
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
            let age = fs::metadata(&path)
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| SystemTime::now().duration_since(t).ok())
                .unwrap_or_default();
            if saved.table.is_over() && age >= keep_finished {
                fs::remove_file(&path)?;
                loaded.expired += 1;
                continue;
            }
            index.insert(stem, Hosted::new(saved.table, saved.tokens));
        }
        loaded.games = index.games.len();
        let hub = Hub {
            index: Mutex::new(index),
            store: Some(store),
        };
        Ok((hub, loaded))
    }

    fn save(&self, table: &Table, tokens: &[String]) {
        if let Some(store) = &self.store
            && let Err(e) = store.save(table, tokens)
        {
            eprintln!("dominion: saving game {}: {e}", table.id());
        }
    }

    pub fn create(&self, req: &NewGame) -> Result<Created, HubError> {
        let config = config(req)?;
        let names = names(req)?;
        let id = {
            let mut index = lock(&self.index);
            index.next += 1;
            format!("g{}", index.next)
        };
        let table = engine(|| Table::new(&id, config, names))?.map_err(|e| bad(e.to_string()))?;
        let tokens: Vec<String> = (0..table.names().len()).map(|_| token()).collect();
        let seats = tokens
            .iter()
            .enumerate()
            .map(|(i, t)| SeatToken {
                seat: i as u8,
                name: table.names()[i].clone(),
                token: t.clone(),
            })
            .collect();
        self.save(&table, &tokens);
        lock(&self.index).insert(&id, Hosted::new(table, tokens));
        Ok(Created { game_id: id, seats })
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
        if !index.games.contains_key(id) {
            return Err(HubError::UnknownGame(id.into()));
        }
        index
            .seats
            .get(token)
            .filter(|seat| seat.game == id)
            .cloned()
            .ok_or_else(|| HubError::BadToken(id.into()))
    }

    /// The seat `token` claims in whichever game it belongs to.
    pub fn find(&self, token: &str) -> Result<Seat, HubError> {
        lock(&self.index)
            .seats
            .get(token)
            .cloned()
            .ok_or_else(|| bad("unknown seat token".into()))
    }

    pub fn list(&self) -> Vec<GameSummary> {
        let games: Vec<Arc<Hosted>> = lock(&self.index).games.values().cloned().collect();
        games
            .iter()
            .map(|g| {
                let t = lock(&g.table);
                GameSummary {
                    game_id: t.id().into(),
                    names: t.names().to_vec(),
                    running: !t.is_over(),
                    deciding: t.deciding().map(|p| p.0),
                    kingdom: t
                        .game()
                        .config()
                        .kingdom
                        .iter()
                        .map(|c| c.name().to_string())
                        .collect(),
                }
            })
            .collect()
    }

    pub fn view(&self, seat: &Seat) -> Result<(AgentView, String), HubError> {
        let game = self.game(&seat.game)?;
        let table = lock(&game.table);
        engine(|| {
            let view = table.view(seat.seat);
            let text = dominion_agent::text::render(&view);
            (view, text)
        })
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
        let mut table = lock(&game.table);
        // Played on a copy, so an answer the engine fails on leaves the game as it was.
        let mut next = table.clone();
        let answered = engine(|| next.answer_with(seat.seat, answer, way))??;
        *table = next;
        self.save(&table, &game.tokens);
        game.version.send_modify(|v| *v += 1);
        Ok(answered)
    }

    /// Waits until `seat` must decide or the game is over, for at most `timeout`. Returns
    /// whether it is so.
    pub async fn wait(&self, seat: &Seat, timeout: Duration) -> Result<bool, HubError> {
        let game = self.game(&seat.game)?;
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
