//! The games a server hosts, and the seat tokens that let a client act for a seat.

use dominion_agent::{AgentView, Answer, AnswerError, Table};
use dominion_engine::{Card, GameConfig, PlayerId, Set};
use rand::Rng;
use serde::{Deserialize, Serialize};
use std::any::Any;
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::io::{self, BufWriter, Write};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, SystemTime};
use tokio::sync::watch;

/// What a client asks for when creating a game.
#[derive(Clone, Debug, Default, Deserialize, Serialize, schemars::JsonSchema)]
pub struct NewGame {
    /// Number of players, 2 to 6.
    pub players: u8,
    /// Seat names, in turn order (default P1, P2, ...).
    #[serde(default)]
    pub names: Vec<String>,
    /// Kingdom piles by name. Without it, a random kingdom is dealt from `sets`.
    #[serde(default)]
    pub kingdom: Vec<String>,
    /// Events, Landmarks, Projects, Ways, ... by name (with `kingdom`).
    #[serde(default)]
    pub landscapes: Vec<String>,
    /// Sets to deal a random kingdom from (default: every released set).
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
}

impl Store {
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

fn card(name: &str, want: impl Fn(Card) -> bool) -> Result<Card, HubError> {
    Card::from_name(name)
        .filter(|&c| want(c))
        .ok_or_else(|| HubError::BadRequest(format!("{name:?} is not a known card here")))
}

fn released_sets() -> Vec<Set> {
    Set::ALL
        .iter()
        .copied()
        .filter(|&s| s != Set::Arcana)
        .collect()
}

pub fn config(req: &NewGame) -> Result<GameConfig, HubError> {
    let seed = req.seed.unwrap_or_else(|| rand::rng().random());
    let mut config = if req.kingdom.is_empty() {
        let sets = if req.sets.is_empty() {
            released_sets()
        } else {
            req.sets
                .iter()
                .map(|name| {
                    let key = |s: &str| s.to_lowercase().replace(['&', ' ', '-', '_'], "");
                    Set::ALL
                        .iter()
                        .copied()
                        .find(|s| key(s.name()) == key(name))
                        .ok_or_else(|| HubError::BadRequest(format!("unknown set {name:?}")))
                })
                .collect::<Result<_, _>>()?
        };
        GameConfig::random(req.players, &sets, seed)
    } else {
        let kingdom = req
            .kingdom
            .iter()
            .map(|n| card(n, |c| c.def().kingdom))
            .collect::<Result<_, _>>()?;
        let mut config = GameConfig::new(req.players, kingdom, seed);
        config.landscapes = req
            .landscapes
            .iter()
            .map(|n| card(n, |c| c.kind().is_landscape()))
            .collect::<Result<_, _>>()?;
        config
    };
    config.turn_limit = req.turn_limit;
    config.colonies = req.colonies;
    config.shelters = req.shelters;
    Ok(config)
}

fn token() -> String {
    format!("{:032x}", rand::rng().random::<u128>())
}

impl Hub {
    /// A hub that keeps its games in `dir` and resumes the ones saved there. Finished
    /// games whose last change is older than `keep_finished` are deleted.
    pub fn open(dir: &Path, keep_finished: Duration) -> io::Result<(Hub, Loaded)> {
        let mut builder = fs::DirBuilder::new();
        builder.recursive(true);
        #[cfg(unix)]
        std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
        builder.create(dir)?;
        let mut loaded = Loaded::default();
        let mut index = Index::default();
        for entry in fs::read_dir(dir)? {
            let path = entry?.path();
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if name.starts_with('.') || !name.ends_with(".json") {
                continue;
            }
            let saved = fs::read(&path)
                .map_err(|e| e.to_string())
                .and_then(|bytes| {
                    serde_json::from_slice::<Saved>(&bytes).map_err(|e| e.to_string())
                });
            let saved = match saved {
                Ok(saved) => saved,
                Err(e) => {
                    loaded.unreadable.push((path, e));
                    continue;
                }
            };
            let id = saved.table.id().to_string();
            if let Some(n) = id.strip_prefix('g').and_then(|n| n.parse().ok()) {
                index.next = index.next.max(n);
            }
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
            index.insert(&id, Hosted::new(saved.table, saved.tokens));
        }
        loaded.games = index.games.len();
        let hub = Hub {
            index: Mutex::new(index),
            store: Some(Store {
                dir: dir.to_path_buf(),
            }),
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
        let id = {
            let mut index = lock(&self.index);
            index.next += 1;
            format!("g{}", index.next)
        };
        let table = engine(|| Table::new(&id, config, req.names.clone()))?
            .map_err(|e| HubError::BadRequest(e.to_string()))?;
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
            .ok_or_else(|| HubError::BadRequest("unknown seat token".into()))
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
        let game = self.game(&seat.game)?;
        let mut table = lock(&game.table);
        // Played on a copy, so an answer the engine fails on leaves the game as it was.
        let mut next = table.clone();
        engine(|| next.answer(seat.seat, answer))??;
        *table = next;
        self.save(&table, &game.tokens);
        game.version.send_modify(|v| *v += 1);
        Ok(())
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
