//! dominion.games games as an account's client was told them, kept to be replayed: each
//! one a JSON Lines file of [`Input`]s in `<account state>/games/`, `<game>.partial.jsonl`
//! while it is played and `<game>.jsonl` once it has finished.
//!
//! Recording is a side record: a file that can't be written stops that game's recording,
//! never the client.

use dominion_online::{
    catalog::{Catalog, VERSION},
    model,
    record::Input,
    reducer::Reducer,
};
use dominion_protocol::api::{Replay, ReplayTurn, SeatView};
use dominion_protocol::lobby::{GameMeta, OFFICIAL_GAME_PREFIX};
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// Where an account whose state is kept in `state` keeps its recordings.
pub fn dir(state: &Path) -> PathBuf {
    state.join("games")
}

fn finished(dir: &Path, game: i64) -> PathBuf {
    dir.join(format!("{game}.jsonl"))
}

fn partial(dir: &Path, game: i64) -> PathBuf {
    dir.join(format!("{game}.partial.jsonl"))
}

/// Records the games an account's reducer is fed.
pub(super) struct Recorder {
    dir: PathBuf,
    game: Option<Writing>,
}

struct Writing {
    id: i64,
    file: BufWriter<File>,
    names: BTreeMap<i32, String>,
    asking: bool,
    /// Something was written since the last frame.
    unframed: bool,
}

impl Recorder {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir, game: None }
    }

    /// `reducer` has just taken `snapshot`: its game is recorded from here on.
    pub fn snapshot(&mut self, reducer: &Reducer, snapshot: model::Snapshot) {
        if self.game.as_ref().is_none_or(|g| g.id != snapshot.game_id) {
            self.game = None;
            if let Err(e) = self.open(reducer, snapshot.game_id) {
                eprintln!(
                    "dominion: recording dominion.games game {}: {e}",
                    snapshot.game_id
                );
            }
        }
        self.write(Input::Snapshot {
            snapshot: Box::new(snapshot),
        });
        if let Some(g) = &mut self.game {
            g.asking = false;
        }
    }

    /// `reducer` has just taken `input` (an event, question or log change).
    pub fn input(&mut self, input: Input) {
        if let (Some(g), Input::Question { .. }) = (&mut self.game, &input) {
            g.asking = true;
        }
        self.write(input);
    }

    /// The game ended with `result`: its recording is complete.
    pub fn finish(&mut self, result: model::GameResult) {
        if self.game.as_ref().is_none_or(|g| g.id != result.game_id) {
            return;
        }
        self.write(Input::Finish { result });
        self.write(Input::Frame);
        let Some(mut g) = self.game.take() else {
            return;
        };
        let done = g
            .file
            .flush()
            .and_then(|()| g.file.get_ref().sync_all())
            .and_then(|()| fs::rename(partial(&self.dir, g.id), finished(&self.dir, g.id)));
        if let Err(e) = done {
            eprintln!("dominion: recording dominion.games game {}: {e}", g.id);
        }
    }

    /// Everything `reducer` took together has been taken: one frame from the official
    /// server, or an answer.
    pub fn frame(&mut self, reducer: &Reducer) {
        let Some(g) = &mut self.game else {
            return;
        };
        let cleared = g.asking && !reducer.asking();
        let names = (g.names != reducer.names).then(|| reducer.names.clone());
        let framed = g.unframed;
        if cleared {
            self.write(Input::Cleared);
        }
        if let Some(names) = names {
            self.write(Input::Names {
                names: names.clone().into_iter().collect(),
            });
            if let Some(g) = &mut self.game {
                g.names = names;
            }
        }
        if framed || cleared {
            self.write(Input::Frame);
        }
        if let Some(g) = &mut self.game {
            g.asking = reducer.asking();
            g.unframed = false;
            if let Err(e) = g.file.flush() {
                eprintln!("dominion: recording dominion.games game {}: {e}", g.id);
                self.game = None;
            }
        }
    }

    fn open(&mut self, reducer: &Reducer, id: i64) -> io::Result<()> {
        if finished(&self.dir, id).exists() {
            return Ok(());
        }
        fs::create_dir_all(&self.dir)?;
        let mut file = OpenOptions::new()
            .create(true)
            .read(true)
            .append(true)
            .open(partial(&self.dir, id))?;
        // A line a crash cut short ends here, so that the next is whole.
        let len = file.metadata()?.len();
        if len > 0 {
            let mut last = [0];
            file.seek(SeekFrom::Start(len - 1))?;
            file.read_exact(&mut last)?;
            if last[0] != b'\n' {
                file.write_all(b"\n")?;
            }
        }
        self.game = Some(Writing {
            id,
            file: BufWriter::new(file),
            names: reducer.names.clone(),
            asking: false,
            unframed: false,
        });
        self.write(Input::Begin {
            game: id,
            client: VERSION.into(),
            player: reducer.player_id,
            names: reducer.names.clone().into_iter().collect(),
        });
        Ok(())
    }

    fn write(&mut self, input: Input) {
        let Some(g) = &mut self.game else {
            return;
        };
        let written = serde_json::to_vec(&input)
            .map_err(io::Error::other)
            .and_then(|mut line| {
                line.push(b'\n');
                g.file.write_all(&line)
            });
        match written {
            Ok(()) => g.unframed |= !matches!(input, Input::Frame),
            Err(e) => {
                eprintln!("dominion: recording dominion.games game {}: {e}", g.id);
                self.game = None;
            }
        }
    }
}

/// The finished games the accounts recorded, replayed.
pub struct Replays {
    dirs: Vec<PathBuf>,
    /// Where `dominion online sync` keeps official clients' code, which the card lists of
    /// games recorded under an older client are read from.
    cache: Option<PathBuf>,
    catalogs: Mutex<HashMap<String, Arc<Catalog>>>,
    games: Mutex<VecDeque<(i64, Arc<Recording>)>>,
    /// Points of games replayed lately: the game, how many inputs were taken, the reducer.
    points: Mutex<VecDeque<(i64, usize, Reducer)>>,
}

/// How many recordings are kept read.
const GAMES: usize = 16;
/// How many replayed points are kept to go on from.
const POINTS: usize = 8;

struct Recording {
    client: String,
    inputs: Vec<Input>,
    /// For each step, how many inputs are taken to show it.
    ends: Vec<usize>,
    turns: Vec<ReplayTurn>,
}

impl Recording {
    fn read(path: &Path) -> io::Result<Recording> {
        let text = fs::read_to_string(path)?;
        // A line a crash cut short is left out.
        let inputs: Vec<Input> = (text.lines())
            .filter_map(|line| serde_json::from_str(line).ok())
            .collect();
        let client = (inputs.iter())
            .find_map(|i| match i {
                Input::Begin { client, .. } => Some(client.clone()),
                _ => None,
            })
            .ok_or_else(|| io::Error::other("it has no beginning"))?;
        let (mut ends, mut turns) = (vec![], vec![]);
        let mut shown = false;
        let mut turn = None;
        for (i, input) in inputs.iter().enumerate() {
            let now = match input {
                Input::Snapshot { snapshot } => Some(&snapshot.state.turn),
                Input::Event {
                    event: model::Event::TurnDescription(t),
                } => Some(t),
                _ => None,
            };
            if let Some(t) = now.filter(|t| t.owner_id >= 0 && t.turn_number > 0) {
                let this = (t.owner_id, t.turn_number);
                if turn != Some(this) {
                    turn = Some(this);
                    turns.push(ReplayTurn {
                        step: ends.len(),
                        player: t.owner_id as u8,
                        turn: t.turn_number as u32,
                    });
                }
            }
            match input {
                Input::Snapshot { .. }
                | Input::Log { .. }
                | Input::Question { .. }
                | Input::Finish { .. } => shown = true,
                Input::Frame if shown => {
                    ends.push(i + 1);
                    shown = false;
                }
                _ => {}
            }
        }
        if ends.is_empty() {
            return Err(io::Error::other("it has nothing to show"));
        }
        if let Some(last) = ends.last_mut() {
            *last = inputs.len();
        }
        let steps = ends.len();
        turns.retain(|t| t.step < steps);
        Ok(Recording {
            client,
            inputs,
            ends,
            turns,
        })
    }
}

/// Why a replay can't be shown.
#[derive(Debug)]
pub enum Refused {
    /// No such game was recorded, or it hasn't finished.
    Missing(String),
    BadRequest(String),
    Failed(String),
}

impl Replays {
    /// The recordings in `dirs`, `current` being this client's card list.
    pub fn new(dirs: Vec<PathBuf>, cache: Option<PathBuf>, current: Arc<Catalog>) -> Self {
        Self {
            dirs,
            cache,
            catalogs: Mutex::new(HashMap::from([(VERSION.to_string(), current)])),
            games: Mutex::default(),
            points: Mutex::default(),
        }
    }

    /// The game a history id (`dominion.games-dominion.games:<n>`), an official game id
    /// (`dominion.games:<n>`) or a bare number names.
    fn number(id: &str) -> Option<i64> {
        let id = id.strip_prefix(OFFICIAL_GAME_PREFIX).unwrap_or(id);
        id.strip_prefix("dominion.games:")
            .unwrap_or(id)
            .parse()
            .ok()
    }

    fn path(&self, game: i64) -> Option<PathBuf> {
        (self.dirs.iter())
            .map(|d| finished(d, game))
            .find(|p| p.exists())
    }

    /// Whether official game `id` was recorded whole.
    pub fn has(&self, id: &str) -> bool {
        Self::number(id).is_some_and(|n| self.path(n).is_some())
    }

    fn recording(&self, game: i64) -> Result<Arc<Recording>, Refused> {
        let mut games = crate::hub::lock(&self.games);
        if let Some(i) = games.iter().position(|(g, _)| *g == game) {
            let entry = games.remove(i).expect("found");
            let recording = entry.1.clone();
            games.push_front(entry);
            return Ok(recording);
        }
        let path = self.path(game).ok_or_else(|| {
            Refused::Missing(format!(
                "dominion.games game {game} has no finished recording here"
            ))
        })?;
        let recording = Arc::new(
            Recording::read(&path)
                .map_err(|e| Refused::Failed(format!("reading {}: {e}", path.display())))?,
        );
        games.push_front((game, recording.clone()));
        games.truncate(GAMES);
        Ok(recording)
    }

    fn catalog(&self, client: &str) -> Result<Arc<Catalog>, Refused> {
        let mut catalogs = crate::hub::lock(&self.catalogs);
        if let Some(c) = catalogs.get(client) {
            return Ok(c.clone());
        }
        let missing = || {
            Refused::Failed(format!(
                "the game was recorded under dominion.games client {client}, whose card \
                 list this server doesn't have"
            ))
        };
        let path = (self.cache.as_ref())
            .map(|d| d.join(format!("body-{client}.js")))
            .ok_or_else(missing)?;
        let bytes = fs::read(&path).map_err(|_| missing())?;
        let catalog = Arc::new(
            Catalog::from_bundle(&bytes)
                .map_err(|e| Refused::Failed(format!("reading {}: {e}", path.display())))?,
        );
        catalogs.insert(client.into(), catalog.clone());
        Ok(catalog)
    }

    /// Game `id` as it stood at `step`, as its recording account saw it, with the log
    /// from entry `log_from` on.
    pub fn view(&self, id: &str, step: usize, log_from: usize) -> Result<SeatView, Refused> {
        let game = Self::number(id)
            .ok_or_else(|| Refused::BadRequest(format!("{id} is no dominion.games game")))?;
        let recording = self.recording(game)?;
        let steps = recording.ends.len() - 1;
        if step > steps {
            return Err(Refused::BadRequest(format!(
                "dominion.games game {game} has {steps} steps"
            )));
        }
        let end = recording.ends[step];
        let start = {
            let points = crate::hub::lock(&self.points);
            (points.iter())
                .filter(|(g, taken, _)| *g == game && *taken <= end)
                .max_by_key(|(_, taken, _)| *taken)
                .map(|(_, taken, r)| (*taken, r.clone()))
        };
        let (taken, mut reducer) = match start {
            Some(start) => start,
            None => (0, Reducer::new(self.catalog(&recording.client)?, -1)),
        };
        for input in &recording.inputs[taken..end] {
            reducer.feed(input).map_err(|e| {
                Refused::Failed(format!("replaying dominion.games game {game}: {e}"))
            })?;
        }
        let mut view = reducer
            .view()
            .map_err(|e| Refused::Failed(format!("dominion.games game {game}: {e}")))?;
        let deciding = view.decision.take().and(view.seat);
        view.spectating = true;
        view.since_last_decision.retain(|e| e.seq >= log_from);
        let seen = SeatView {
            view,
            version: step as u64,
            deciding,
            table: reducer.table().unwrap_or_default(),
            undos: 0,
            can_undo: false,
            undo: Default::default(),
            meta: GameMeta {
                watchable: true,
                now: crate::hub::now_ms(),
                ..Default::default()
            },
            replay: Some(Replay {
                step,
                steps,
                turns: recording.turns.clone(),
                seat: reducer.seat(),
            }),
        };
        let mut points = crate::hub::lock(&self.points);
        points.push_front((game, end, reducer));
        points.truncate(POINTS);
        Ok(seen)
    }
}
