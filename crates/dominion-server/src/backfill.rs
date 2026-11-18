//! `dominion games backfill`: `dominion/events@1` streams for the games saved before the
//! server recorded them. A saved table keeps its game's config and every answer it took,
//! and a game is determined by them, so playing it again recording gives its stream. The
//! engine may have changed since, so a stream is written only when the game played again
//! is the saved one: every log entry, the final state and the outcome, and the stream
//! itself replays to it. Its header says it was reconstructed, and when.
//!
//! It works offline, on the state directory and `expired/`: a run that writes takes the
//! directory's lock, so the server must be stopped; a dry run takes nothing and writes
//! nothing (snapshots are replaced atomically, so it can read beside a running server).
//! A stream is written beside its snapshot, atomically and durably, never over another,
//! and a snapshot is never touched. A game "has a stream" only if it is a whole one: the
//! one its table records, in step with it as the server checks at startup, or one an
//! earlier run rebuilt. A game whose recording stopped, or whose stream isn't in step, gets
//! a rebuilt one beside its incomplete stream, which is kept. A hosted game that can still
//! change (in progress, or able to be undone) is left alone: a stream written now could
//! stop being the game's. The server's `record_events` setting doesn't apply: a backfill
//! is run on purpose.

use crate::hub::{self, Meta, SavedMeta, Stream, StreamError};
use crate::store::{self, EXPIRED, Store};
use dominion_agent::recording::{self, Encoding};
use dominion_agent::{Table, Unreplayed, replay};
use dominion_engine::{EndReason, Event as LogEntry, PlayerId};
use dominion_protocol::events::{Event, Header};
use serde::Deserialize;
use serde_json::Value;
use std::fmt;
use std::fs;
use std::io::{self, BufWriter, Write};
use std::path::Path;

/// Why a saved game can't be converted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// This build can't read the snapshot.
    Unreadable(String),
    /// The table didn't keep every answer (saved before it did).
    Incomplete,
    /// A reconstructed stream is beside it that isn't its own; it is left as it is.
    Foreign(String),
    /// This build can't set the game up from its config.
    Setup(String),
    /// The game played again went differently: at log entry `entry` (if the logs differ),
    /// from answer `answer` on (of `answers`).
    Diverged {
        answer: Option<usize>,
        answers: usize,
        entry: Option<usize>,
        detail: String,
    },
    /// Every log entry is the same, but the game came out differently.
    Differs(String),
    /// What the game did can't be written as events.
    Unrecordable(String),
    /// The stream it records doesn't replay to the game.
    Unreplayable(String),
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Refusal::Unreadable(why) => write!(f, "the snapshot can't be read: {why}"),
            Refusal::Incomplete => f.write_str(
                "it didn't keep every answer (saved before tables kept them all), so it can't \
                 be played again",
            ),
            Refusal::Foreign(why) => write!(
                f,
                "{why}; it is never served, and a run rebuilds it once that file is removed"
            ),
            Refusal::Setup(why) => write!(f, "this build can't set it up: {why}"),
            Refusal::Diverged {
                answer,
                answers,
                detail,
                ..
            } => match answer {
                Some(a) => write!(
                    f,
                    "it plays differently from answer {a} of {answers}: {detail}"
                ),
                None => write!(f, "it plays differently: {detail}"),
            },
            Refusal::Differs(why) => write!(f, "it comes out differently: {why}"),
            Refusal::Unrecordable(why) => write!(f, "its events can't be recorded: {why}"),
            Refusal::Unreplayable(why) => write!(f, "the stream it records doesn't replay: {why}"),
        }
    }
}

/// How a converted game stands at the end of its stream.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Ending {
    Finished,
    /// `seat` resigned; `cause` is the host's (`resigned`, `abandoned` or `timeout`). The
    /// stream says `resigned`, as the engine does.
    Resigned {
        seat: PlayerId,
        cause: String,
    },
    /// Waiting on a decision.
    InProgress,
}

/// A saved game converted.
#[derive(Clone, Debug)]
pub struct Converted {
    /// The stream, from `setup`: exactly what the table would have recorded.
    pub events: Vec<Event>,
    pub ending: Ending,
    pub names: Vec<String>,
}

impl Converted {
    pub fn ending_text(&self) -> String {
        match &self.ending {
            Ending::Finished => "finished".into(),
            Ending::Resigned { seat, cause } => {
                let who = match self.names.get(seat.index()) {
                    Some(name) if *name != seat.to_string() => format!("{name} ({seat})"),
                    _ => seat.to_string(),
                };
                format!("{who} out ({cause})")
            }
            Ending::InProgress => "in progress".into(),
        }
    }
}

/// The hub's saved wrapper, as far as the backfill needs it.
#[derive(Deserialize)]
struct Saved {
    table: Table,
    #[serde(default)]
    meta: SavedMeta,
}

/// A saved game as read: the snapshot as written, its table and its meta.
struct Read {
    raw: Value,
    table: Table,
    meta: Meta,
}

fn read(bytes: &[u8], id: &str) -> Result<Read, Refusal> {
    let raw: Value =
        serde_json::from_slice(bytes).map_err(|e| Refusal::Unreadable(e.to_string()))?;
    let saved = Saved::deserialize(&raw).map_err(|e| Refusal::Unreadable(e.to_string()))?;
    if saved.table.id() != id {
        return Err(Refusal::Unreadable(format!(
            "it holds game {}",
            saved.table.id()
        )));
    }
    Ok(Read {
        raw,
        table: saved.table,
        meta: saved.meta.into_meta(),
    })
}

/// Reads a saved game (`<id>.json`) and plays it again recording, checking that it is the
/// same game: the stream, or why there can't be one. Whatever the table's own recording.
pub fn convert(bytes: &[u8], id: &str) -> Result<Converted, Refusal> {
    convert_read(&read(bytes, id)?)
}

fn convert_read(saved: &Read) -> Result<Converted, Refusal> {
    let raw = &saved.raw;
    let table = &saved.table;
    if !table.complete() {
        return Err(Refusal::Incomplete);
    }
    let again = match table.rerecorded() {
        Ok(again) => again,
        Err(Unreplayed {
            table: None, why, ..
        }) => return Err(Refusal::Setup(why)),
        Err(Unreplayed {
            table: Some(partial),
            answer,
            why,
        }) => return Err(diverged(table, &partial, answer, Some(why))),
    };
    let (old, new) = (table.game(), again.game());
    if old.log() != new.log() {
        return Err(diverged(table, &again, None, None));
    }
    if old.outcome() != new.outcome() {
        return Err(Refusal::Differs(format!(
            "outcome: saved {}, replayed {}",
            json(&old.outcome()),
            json(&new.outcome())
        )));
    }
    let as_now = |g| serde_json::to_value(g).map_err(|e| Refusal::Differs(e.to_string()));
    let mut path = String::new();
    if let Some(at) = first_difference(
        &raw["table"]["game"],
        &as_now(old)?,
        &as_now(new)?,
        &mut path,
    ) {
        return Err(Refusal::Differs(format!("the game's state, at {at}")));
    }
    if let Some(why) = again.recording_stopped() {
        return Err(Refusal::Unrecordable(why.into()));
    }
    let events = again.unwritten().to_vec();
    if again.recorded() != Some(events.len() as u64) {
        return Err(Refusal::Unrecordable("events were lost".into()));
    }
    let replayed = replay::replay(&events)
        .map_err(|d| Refusal::Unreplayable(d.to_string().replace('\n', "; ")))?;
    if replayed.game.log() != old.log() || replayed.game.outcome() != old.outcome() {
        return Err(Refusal::Unreplayable(
            "it replays to another game than the saved one".into(),
        ));
    }
    let ending = match old.outcome() {
        None => Ending::InProgress,
        Some(o) if o.reason == EndReason::Resigned => Ending::Resigned {
            seat: o.resigned.first().copied().unwrap_or(PlayerId(0)),
            cause: saved
                .meta
                .ended
                .clone()
                .unwrap_or_else(|| "resigned".into()),
        },
        Some(_) => Ending::Finished,
    };
    Ok(Converted {
        events,
        ending,
        names: table.names().to_vec(),
    })
}

/// Where the logs of the saved game and the one played again first differ.
/// `why` the replay stopped, if it did.
fn diverged(saved: &Table, again: &Table, answer: Option<usize>, why: Option<String>) -> Refusal {
    let (old, new) = (saved.game().log(), again.game().log());
    let entry = (0..old.len().max(new.len())).find(|&i| old.get(i) != new.get(i));
    let detail = match (entry, why) {
        (Some(i), why) if i < new.len() || why.is_none() => {
            let line = |e: Option<&LogEntry>| e.map_or("nothing".into(), json);
            let then = why.map(|w| format!("; then {w}")).unwrap_or_default();
            format!(
                "log entry {i}{}: saved {}, replayed {}{then}",
                context(saved, i),
                line(old.get(i)),
                line(new.get(i))
            )
        }
        (_, why) => why.unwrap_or_default(),
    };
    let answer = answer.or_else(|| entry.and_then(|i| saved.answer_for_log(i)));
    Refusal::Diverged {
        answer,
        answers: saved.answers(),
        entry,
        detail,
    }
}

/// The turn log entry `i` of `table`'s game is in, and the card played last before it.
fn context(table: &Table, i: usize) -> String {
    let log = &table.game().log()[..i.min(table.game().log().len())];
    let Some(start) = log
        .iter()
        .rposition(|e| matches!(e, LogEntry::TurnStart { .. }))
    else {
        return String::new();
    };
    let LogEntry::TurnStart { player, turn } = log[start] else {
        unreachable!()
    };
    let name = table
        .names()
        .get(player.index())
        .map_or("?", |n| n.as_str());
    let card = log[start..].iter().rev().find_map(|e| match e {
        LogEntry::Played { card, .. } => Some(card.name()),
        _ => None,
    });
    match card {
        Some(card) => format!(" ({name}'s turn {turn}, after {card} was played)"),
        None => format!(" ({name}'s turn {turn})"),
    }
}

fn json(value: &impl serde::Serialize) -> String {
    let mut s = serde_json::to_string(value).unwrap_or_else(|e| e.to_string());
    if s.len() > 300 {
        let mut cut = 300;
        while !s.is_char_boundary(cut) {
            cut -= 1;
        }
        s.truncate(cut);
        s.push('…');
    }
    s
}

/// What a game played again keeps that the saved one has no part in.
const RECORDING_ONLY: [&str; 3] = ["journal", "chance", "trace"];

/// The first place where `old` and `new` (the saved game and the one played again, as this
/// build writes them) differ, looking only at what `raw` (the saved game as written when it
/// was saved) holds: what a build since has added is no part of the saved game.
fn first_difference(raw: &Value, old: &Value, new: &Value, path: &mut String) -> Option<String> {
    match (raw, old, new) {
        (Value::Object(r), Value::Object(a), Value::Object(b)) => {
            for key in r.keys() {
                if path.is_empty() && RECORDING_ONLY.contains(&key.as_str()) {
                    continue;
                }
                let len = path.len();
                path.push('.');
                path.push_str(key);
                let found = match (a.get(key), b.get(key)) {
                    (None, None) => None,
                    (Some(x), Some(y)) => first_difference(&r[key], x, y, path),
                    (x, y) => Some(format!("{path}: saved {}, replayed {}", json(&x), json(&y))),
                };
                if found.is_some() {
                    return found;
                }
                path.truncate(len);
            }
            None
        }
        (Value::Array(r), Value::Array(a), Value::Array(b))
            if r.len() == a.len() && a.len() == b.len() =>
        {
            for (i, ((r, x), y)) in r.iter().zip(a).zip(b).enumerate() {
                let len = path.len();
                path.push_str(&format!("[{i}]"));
                if let Some(found) = first_difference(r, x, y, path) {
                    return Some(found);
                }
                path.truncate(len);
            }
            None
        }
        _ => (old != new).then(|| format!("{path}: saved {}, replayed {}", json(old), json(new))),
    }
}

/// Where a game's files are.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Place {
    /// The state directory: the server hosts it.
    Hosted,
    /// `expired/`: kept to be looked at, never hosted again.
    Expired,
}

/// What the backfill did with one game.
#[derive(Clone, Debug)]
pub enum Status {
    /// Converted; its stream is written, or would be in a dry run.
    Converted {
        events: usize,
        ending: String,
        /// Why the stream its table recorded is incomplete, if it had one: that recording
        /// stopped, or isn't in step with the game. Its file, if any, is kept, marked
        /// incomplete.
        replaces: Option<String>,
    },
    /// It has a whole stream already, which is left as it is.
    HasStream(Encoding),
    /// Hosted and in progress: nothing written. Its answers so far convert, to this many
    /// events.
    InProgress(usize),
    /// Hosted, over, and its rules still allow an undo: nothing written until it is
    /// retired. It converts, to this many events.
    Undoable(usize),
    NotConvertible(Refusal),
    /// It converted, but writing its stream failed.
    Failed(String),
}

#[derive(Clone, Debug)]
pub struct Entry {
    pub id: String,
    pub place: Place,
    pub status: Status,
}

impl fmt::Display for Entry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let place = match self.place {
            Place::Hosted => "hosted",
            Place::Expired => "expired",
        };
        let (status, detail) = match &self.status {
            Status::Converted {
                events,
                ending,
                replaces,
            } => (
                "converted",
                format!(
                    "{}; {ending}{}",
                    dominion_engine::amount(*events as i64, "event", "events"),
                    match replaces {
                        Some(why) => format!(
                            "; stands in for its own recording, which is incomplete ({why}): \
                             any incomplete stream is kept"
                        ),
                        None => String::new(),
                    }
                ),
            ),
            Status::HasStream(e) => ("has a stream", format!("{e}, left as it is")),
            Status::InProgress(_) => (
                "in progress",
                "not written while the server hosts it: run again once it has ended (its \
                 answers so far convert)"
                    .into(),
            ),
            Status::Undoable(_) => (
                "can be undone",
                "not written while its rules let it be undone: run again once it is retired \
                 (it converts)"
                    .into(),
            ),
            Status::NotConvertible(why) => ("not convertible", why.to_string()),
            Status::Failed(why) => ("failed", why.clone()),
        };
        write!(f, "{:<7} {place:<8} {status:<16} {detail}", self.id)
    }
}

/// What a backfill found.
#[derive(Clone, Debug, Default)]
pub struct Report {
    pub dry_run: bool,
    pub entries: Vec<Entry>,
}

impl Report {
    fn count(&self, f: impl Fn(&Status) -> bool) -> usize {
        self.entries.iter().filter(|e| f(&e.status)).count()
    }

    /// How many games failed to be written.
    pub fn failed(&self) -> usize {
        self.count(|s| matches!(s, Status::Failed(_)))
    }
}

impl fmt::Display for Report {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let converted = self.count(|s| matches!(s, Status::Converted { .. }));
        write!(
            f,
            "{}: {} {}, {} already had a stream, {} in progress (left for later), {} able to \
             be undone (left until retired), {} not convertible",
            dominion_engine::amount(self.entries.len() as i64, "game", "games"),
            converted,
            if self.dry_run {
                "would be converted"
            } else {
                "converted"
            },
            self.count(|s| matches!(s, Status::HasStream(_))),
            self.count(|s| matches!(s, Status::InProgress(_))),
            self.count(|s| matches!(s, Status::Undoable(_))),
            self.count(|s| matches!(s, Status::NotConvertible(_))),
        )?;
        match self.failed() {
            0 => Ok(()),
            n => write!(f, ", {n} failed to be written"),
        }
    }
}

pub struct Options {
    /// Convert and check everything, write nothing.
    pub dry_run: bool,
    /// What streams are written in: the server's `events_encoding`.
    pub encoding: Encoding,
    /// When, for the header (RFC 3339).
    pub at: String,
}

impl Options {
    pub fn now(dry_run: bool, encoding: Encoding) -> Options {
        Options {
            dry_run,
            encoding,
            at: crate::openai::rfc3339(std::time::SystemTime::now()),
        }
    }
}

/// Backfills every game in state directory `dir` and its `expired/`, telling `each` about
/// each game as it is done. A run that writes holds the directory's lock throughout.
pub fn run(dir: &Path, options: &Options, mut each: impl FnMut(&Entry)) -> io::Result<Report> {
    let store = match options.dry_run {
        true => {
            fs::metadata(dir)?;
            None
        }
        false => Some(Store::open(dir)?),
    };
    let expired = dir.join(EXPIRED);
    let mut report = Report {
        dry_run: options.dry_run,
        entries: vec![],
    };
    for (place, here) in [(Place::Hosted, dir), (Place::Expired, &expired)] {
        for id in games(here)? {
            let status = backfill(dir, store.as_ref(), here, &id, place, options);
            let entry = Entry { id, place, status };
            each(&entry);
            report.entries.push(entry);
        }
    }
    Ok(report)
}

/// The ids of the games saved in `dir`, in order.
fn games(dir: &Path) -> io::Result<Vec<String>> {
    let entries = match fs::read_dir(dir) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(vec![]),
        entries => entries?,
    };
    let mut ids: Vec<(u64, String)> = vec![];
    for entry in entries {
        let name = entry?.file_name();
        let Some(stem) = name.to_str().and_then(|n| n.strip_suffix(".json")) else {
            continue;
        };
        if let Some(n) = stem.strip_prefix('g').and_then(|n| n.parse::<u64>().ok())
            && format!("g{n}") == stem
        {
            ids.push((n, stem.into()));
        }
    }
    ids.sort();
    Ok(ids.into_iter().map(|(_, id)| id).collect())
}

/// Backfills game `id`, saved in `dir` (the state directory `state` or its `expired/`).
/// `store` is held for a run that writes.
fn backfill(
    state: &Path,
    store: Option<&Store>,
    dir: &Path,
    id: &str,
    place: Place,
    options: &Options,
) -> Status {
    let bytes = match fs::read(dir.join(format!("{id}.json"))) {
        Ok(bytes) => bytes,
        Err(e) => return Status::NotConvertible(Refusal::Unreadable(e.to_string())),
    };
    let saved = match read(&bytes, id) {
        Ok(saved) => saved,
        Err(why) => return Status::NotConvertible(why),
    };
    // What the server would find at startup.
    let replaces = match hub::stream_state(state, &saved.table) {
        Ok(Stream::File(encoding, _) | Stream::Reconstructed(encoding, _)) => {
            return Status::HasStream(encoding);
        }
        Ok(Stream::New) => Some("it records, but none of its stream is written".into()),
        Ok(Stream::Off) => None,
        Err(StreamError::Recorded(why)) => Some(why),
        Err(e @ StreamError::Reconstructed(_)) => {
            return Status::NotConvertible(Refusal::Foreign(e.to_string()));
        }
    };
    let converted = match convert_read(&saved) {
        Ok(c) => c,
        Err(why) => return Status::NotConvertible(why),
    };
    if place == Place::Hosted && hub::can_change(&saved.meta, &saved.table) {
        let events = converted.events.len();
        return match saved.table.is_over() {
            false => Status::InProgress(events),
            true => Status::Undoable(events),
        };
    }
    let done = Status::Converted {
        events: converted.events.len(),
        ending: converted.ending_text(),
        replaces: replaces.clone(),
    };
    let Some(store) = store else {
        return done;
    };
    // An incomplete stream the server hasn't yet renamed (it would at its next start)
    // makes way, kept, for the whole one.
    if replaces.is_some()
        && store::stream_file(state, id).is_some()
        && let Err(e) = store.mark_incomplete(id)
    {
        return Status::Failed(format!("marking its stream incomplete: {e}"));
    }
    let header = recording::reconstructed_header(id, &options.at);
    match write_new(dir, id, options.encoding, &header, &converted.events) {
        Ok(true) => done,
        Ok(false) => Status::HasStream(options.encoding),
        Err(e) => Status::Failed(format!("writing its stream: {e}")),
    }
}

/// Writes game `id`'s stream into `dir`, atomically and only if it has none there: false
/// if it has.
fn write_new(
    dir: &Path,
    id: &str,
    encoding: Encoding,
    header: &Header,
    events: &[Event],
) -> io::Result<bool> {
    let path = dir.join(format!("{id}{}", encoding.suffix()));
    let tmp = dir.join(format!(".{id}{}.backfill.tmp", encoding.suffix()));
    let written = write_tmp(&tmp, encoding, header, events).and_then(|()| {
        // A link, unlike a rename, never replaces what is there.
        match fs::hard_link(&tmp, &path) {
            Ok(()) => Ok(true),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => Ok(false),
            Err(e) => Err(e),
        }
    });
    let _ = fs::remove_file(&tmp);
    let written = written?;
    store::sync_dir(dir)?;
    Ok(written)
}

fn write_tmp(path: &Path, encoding: Encoding, header: &Header, events: &[Event]) -> io::Result<()> {
    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    let file = options.open(path)?;
    let mut out = BufWriter::new(&file);
    encoding.write(&mut out, header)?;
    for event in events {
        encoding.write(&mut out, event)?;
    }
    out.flush()?;
    drop(out);
    file.sync_all()
}

/// `dominion games backfill`: prints a line per game as it goes, then the totals.
pub fn cli(dir: &Path, options: &Options) -> Result<(), String> {
    println!(
        "{} {}",
        match options.dry_run {
            true => "Dry run (nothing is written) of the backfill of",
            false => "Backfilling",
        },
        dir.display()
    );
    let report = run(dir, options, |entry| println!("{entry}"))
        .map_err(|e| format!("{}: {e}", dir.display()))?;
    println!("{report}");
    match report.failed() {
        0 => Ok(()),
        n => Err(format!("{n} streams could not be written")),
    }
}
