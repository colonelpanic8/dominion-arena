//! `dominion games backfill`: streams for games saved before the server recorded them.

use dominion_agent::recording::{self, Encoding};
use dominion_agent::{Answer, Table, WayChoice};
use dominion_engine::{Card, GameConfig, PlayerId};
use dominion_protocol::events::{self, Event, What};
use dominion_protocol::lobby::UndoRule;
use dominion_server::backfill::{self, Ending, Entry, Options, Place, Refusal, Status};
use dominion_server::hub::HubError;
use dominion_server::hub::{Hub, Meta};
use dominion_server::store::Store;
use rand::seq::SliceRandom;
use rand::{Rng, SeedableRng};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

const KEEP: Duration = Duration::from_secs(1 << 30);
const AT: &str = "2026-11-17T12:00:00Z";

fn config(seed: u64) -> GameConfig {
    use Card::*;
    let kingdom = vec![
        Village, Smithy, Militia, Moat, ThroneRoom, Market, Laboratory, Festival, Mine, Bureaucrat,
    ];
    let mut config = GameConfig::new(2, kingdom, seed);
    config.landscapes = vec![WayOfTheOx, WayOfTheSheep];
    config.turn_limit = Some(16);
    config
}

/// A game played the old way (a table that doesn't record), and the same answers given to
/// a table that does: what the backfill must reproduce.
struct Twins {
    old: Table,
    live: Table,
    /// Every answer given, by seat.
    given: Vec<(PlayerId, Given)>,
}

/// An answer, to give again to another decision.
#[derive(Clone, Debug)]
enum Given {
    Treasures,
    Copper(Option<WayChoice>),
    Picks(Vec<u32>, Option<WayChoice>),
}

impl Given {
    fn answer(&self, decision: &str) -> (Answer, Option<WayChoice>) {
        match self {
            Given::Treasures => unreachable!(),
            Given::Copper(way) => (Answer::name(decision, "Copper"), *way),
            Given::Picks(picks, way) => (Answer::indices(decision, picks.clone()), *way),
        }
    }

    /// Gives it to `table` for `seat`.
    fn to_table(&self, table: &mut Table, seat: PlayerId) {
        let d = table.view(seat).decision.unwrap();
        match self {
            Given::Treasures => {
                table.play_treasures(seat, &d.id).unwrap();
            }
            given => {
                let (answer, way) = given.answer(&d.id);
                table.answer_with(seat, &answer, way).unwrap();
            }
        }
    }

    /// Gives it through `hub` for `seat`.
    fn to_hub(&self, hub: &Hub, seat: &dominion_server::hub::Seat) {
        let d = hub.view(seat).unwrap().0.decision.unwrap();
        match self {
            Given::Treasures => {
                hub.play_treasures(seat, &d.id).unwrap();
            }
            given => {
                let (answer, way) = given.answer(&d.id);
                hub.answer_with(seat, &answer, way).unwrap();
            }
        }
    }
}

impl Twins {
    fn new(id: &str, seed: u64) -> Twins {
        let names = vec!["Ann".to_string(), "Bo".to_string()];
        Twins {
            old: Table::new(id, config(seed), names.clone()).unwrap(),
            live: Table::new_recorded(id, config(seed), names).unwrap(),
            given: vec![],
        }
    }

    /// Gives both `n` random answers, or until the game ends, with Autoplay Treasures and
    /// Way questions answered in advance among them.
    fn play(&mut self, seed: u64, n: usize) -> &mut Twins {
        let mut rng = rand::rngs::StdRng::seed_from_u64(seed);
        for _ in 0..n {
            let Some(seat) = self.old.deciding() else {
                break;
            };
            let d = self.old.view(seat).decision.unwrap();
            if d.kind == "turn"
                && d.phase.as_deref() == Some("buy")
                && rng.random_bool(0.4)
                && self.old.play_treasures(seat, &d.id).is_ok()
            {
                self.live.play_treasures(seat, &d.id).unwrap();
                self.given.push((seat, Given::Treasures));
                continue;
            }
            let picks = (!d.name_any).then(|| {
                let (min, max) = (d.min.unwrap_or(1) as usize, d.max.unwrap_or(1) as usize);
                let mut picks: Vec<u32> = (0..d.options.len() as u32).collect();
                picks.shuffle(&mut rng);
                picks.truncate(rng.random_range(min..=max.min(picks.len())));
                picks
            });
            let way = match rng.random_range(0..4) {
                0 => Some(WayChoice::Way(Card::WayOfTheOx)),
                1 => Some(WayChoice::None),
                _ => None,
            };
            let given = match picks {
                Some(picks) => Given::Picks(picks, way),
                None => Given::Copper(way),
            };
            given.to_table(&mut self.old, seat);
            given.to_table(&mut self.live, seat);
            self.given.push((seat, given));
        }
        self
    }

    fn finish(&mut self, seed: u64) -> &mut Twins {
        self.play(seed, usize::MAX)
    }

    fn resign(&mut self, seat: u8) -> &mut Twins {
        self.old.resign(PlayerId(seat)).unwrap();
        self.live.resign(PlayerId(seat)).unwrap();
        self
    }

    fn expected(&self) -> &[Event] {
        self.live.unwritten()
    }

    /// The hub's saved wrapper of the old table.
    fn saved(&self, meta: &Meta) -> Value {
        let tokens: Vec<String> = (0..self.old.names().len())
            .map(|i| format!("{}-{i}", self.old.id()))
            .collect();
        json!({"table": self.old, "tokens": tokens, "meta": meta})
    }
}

fn ended(cause: &str) -> Meta {
    Meta {
        ended: Some(cause.into()),
        ..Meta::default()
    }
}

/// A game whose answers can't be taken back: once over, it can't change.
fn no_undo() -> Meta {
    Meta {
        undo: UndoRule::Off,
        ..Meta::default()
    }
}

/// Checks a converted stream: the live twin's, folding to its table, replaying.
fn check_stream(events: &[Event], twins: &Twins) {
    assert!(events == twins.expected(), "not the stream recorded live");
    let mut folded = events::Table::default();
    for event in events {
        folded.apply(event).unwrap();
    }
    assert!(Some(&folded) == twins.live.recorded_table());
    let replay = dominion_agent::replay::replay(events).unwrap_or_else(|d| panic!("{d}"));
    assert_eq!(replay.game.outcome(), twins.old.game().outcome());
    assert_eq!(replay.game.log(), twins.old.game().log());
}

fn convert(saved: &Value, id: &str) -> Result<backfill::Converted, Refusal> {
    backfill::convert(&serde_json::to_vec(saved).unwrap(), id)
}

#[test]
fn a_finished_game_converts_to_the_stream_it_would_have_recorded() {
    let mut twins = Twins::new("g1", 1);
    twins.finish(1);
    assert!(twins.old.is_over() && twins.old.recorded().is_none());
    let converted = convert(&twins.saved(&Meta::default()), "g1").unwrap();
    assert_eq!(converted.ending, Ending::Finished);
    check_stream(&converted.events, &twins);
}

#[test]
fn a_resigned_game_ends_as_it_did() {
    let mut twins = Twins::new("g1", 2);
    twins.play(2, 40).resign(1);
    let converted = convert(&twins.saved(&ended("timeout")), "g1").unwrap();
    assert_eq!(
        converted.ending,
        Ending::Resigned {
            seat: PlayerId(1),
            cause: "timeout".into()
        }
    );
    check_stream(&converted.events, &twins);
    match &converted.events.last().unwrap().what {
        What::GameEnd(end) => {
            assert_eq!(end.reason, "resigned");
            assert!(end.result[1].resigned);
        }
        other => panic!("ends with {other:?}"),
    }
}

#[test]
fn a_game_in_progress_converts_up_to_its_open_decision() {
    let mut twins = Twins::new("g1", 3);
    twins.play(3, 30);
    assert!(!twins.old.is_over());
    let converted = convert(&twins.saved(&Meta::default()), "g1").unwrap();
    assert_eq!(converted.ending, Ending::InProgress);
    assert!(matches!(
        converted.events.last().unwrap().what,
        What::Ask(_)
    ));
    assert!(converted.events == twins.expected());
    let replay = dominion_agent::replay::replay(&converted.events).unwrap();
    assert_eq!(
        replay.game.pending().map(|d| d.player),
        twins.old.deciding()
    );
}

#[test]
fn a_game_that_did_not_keep_every_answer_is_refused() {
    let mut twins = Twins::new("g1", 4);
    twins.play(4, 20);
    let mut saved = twins.saved(&Meta::default());
    saved["table"]["complete"] = json!(false);
    assert_eq!(convert(&saved, "g1").unwrap_err(), Refusal::Incomplete);
}

#[test]
fn a_game_that_plays_differently_now_is_refused_where_it_differs() {
    let mut twins = Twins::new("g1", 5);
    twins.finish(5);
    let saved = twins.saved(&Meta::default());
    let log = saved["table"]["game"]["log"].as_array().unwrap();
    let k = log.iter().rposition(|e| e.get("Played").is_some()).unwrap();

    // The saved log says another card was played there than the engine plays now.
    let mut changed = saved.clone();
    let card = &mut changed["table"]["game"]["log"][k]["Played"]["card"];
    let other = if *card == json!("Moat") {
        "Mine"
    } else {
        "Moat"
    };
    *card = json!(other);
    match convert(&changed, "g1").unwrap_err() {
        Refusal::Diverged {
            entry: Some(e),
            answer: Some(a),
            detail,
            ..
        } => {
            assert_eq!(e, k);
            assert_eq!(Some(a), twins.old.answer_for_log(k));
            assert!(detail.contains(other), "{detail}");
        }
        other => panic!("{other:?}"),
    }

    // An answer that comes elsewhere in the log than it was given.
    let mut moved = saved.clone();
    let step = &mut moved["table"]["history"][10]["log"];
    *step = json!(step.as_u64().unwrap() + 1);
    assert!(matches!(
        convert(&moved, "g1").unwrap_err(),
        Refusal::Diverged {
            answer: Some(10),
            ..
        }
    ));

    // The same log, a different final state.
    let mut state = saved;
    let coins = &mut state["table"]["game"]["players"][0]["coffers"];
    *coins = json!(coins.as_i64().unwrap_or(0) + 3);
    match convert(&state, "g1").unwrap_err() {
        Refusal::Differs(why) => assert!(why.contains(".players[0].coffers"), "{why}"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn unreadable_and_misnamed_snapshots_are_refused() {
    assert!(matches!(
        backfill::convert(b"{\"table\":", "g1"),
        Err(Refusal::Unreadable(_))
    ));
    let mut twins = Twins::new("g2", 6);
    twins.play(6, 5);
    assert!(matches!(
        convert(&twins.saved(&Meta::default()), "g1"),
        Err(Refusal::Unreadable(_))
    ));
}

/// Every file under `dir`, with its bytes, but the state directory's lock.
fn files(dir: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut out = BTreeMap::new();
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.file_name().is_some_and(|n| n == ".lock") {
            continue;
        }
        if path.is_dir() {
            out.extend(files(&path));
        } else {
            out.insert(path.clone(), fs::read(&path).unwrap());
        }
    }
    out
}

fn run(dir: &Path, dry_run: bool) -> BTreeMap<String, Entry> {
    let options = Options {
        dry_run,
        encoding: Encoding::Cbor,
        at: AT.into(),
    };
    let report = backfill::run(dir, &options, |_| {}).unwrap();
    assert_eq!(report.failed(), 0);
    report
        .entries
        .into_iter()
        .map(|e| (e.id.clone(), e))
        .collect()
}

fn stream(path: &Path) -> recording::Recording {
    let encoding = Encoding::of_file(path.to_str().unwrap()).unwrap();
    recording::read(encoding, &fs::read(path).unwrap(), None).unwrap()
}

#[test]
fn a_state_directory_is_backfilled_once_with_a_dry_run_first() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    // g1 is recorded live, by the hub of this build.
    {
        let (hub, _) = Hub::open(dir, KEEP).unwrap();
        let created = hub
            .create(&dominion_server::hub::NewGame {
                players: 2,
                kingdom: config(0)
                    .kingdom
                    .iter()
                    .map(|c| c.name().to_string())
                    .collect(),
                seed: Some(9),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(created.game_id, "g1");
    }
    let mut finished = Twins::new("g2", 12);
    finished.finish(12);
    let mut resigned = Twins::new("g3", 13);
    resigned.play(13, 20).resign(0);
    let mut playing = Twins::new("g4", 14);
    playing.play(14, 30);
    let mut stopped = Twins::new("g5", 15);
    stopped.play(15, 25);
    assert!(!playing.old.is_over() && !stopped.old.is_over());
    let mut retired = Twins::new("g6", 16);
    retired.finish(16);
    let mut incomplete = Twins::new("g7", 17);
    incomplete.finish(17);
    let mut differs = Twins::new("g8", 18);
    differs.finish(18);
    // Over, and a game made directly whose rules allow undo: written all the same, and
    // served once it can't change.
    let mut undoable = Twins::new("g9", 19);
    undoable.finish(19);
    {
        let store = Store::open(dir).unwrap();
        store.save("g2", &finished.saved(&no_undo())).unwrap();
        store
            .save("g3", &resigned.saved(&ended("abandoned")))
            .unwrap();
        store.save("g4", &playing.saved(&no_undo())).unwrap();
        store.save("g9", &undoable.saved(&Meta::default())).unwrap();
        store.save("g5", &stopped.saved(&Meta::default())).unwrap();
        store.save("g6", &retired.saved(&Meta::default())).unwrap();
        let mut saved = incomplete.saved(&Meta::default());
        saved["table"]["complete"] = json!(false);
        store.save("g7", &saved).unwrap();
        let mut saved = differs.saved(&Meta::default());
        saved["table"]["game"]["log"][3] = json!({"Named": {"player": 0, "card": "Moat"}});
        store.save("g8", &saved).unwrap();
        store.retire("g5").unwrap();
        store.retire("g6").unwrap();
    }
    // What a run cut short leaves behind.
    fs::write(dir.join(".g2.events.cbor.backfill.tmp"), b"junk").unwrap();
    let before = files(dir);

    let dry = run(dir, true);
    assert_eq!(files(dir), before, "a dry run writes nothing");
    let statuses = |entries: &BTreeMap<String, Entry>| -> Vec<(String, Place, String)> {
        entries
            .values()
            .map(|e| {
                let s = match &e.status {
                    Status::Converted { undoable: true, .. } => "converted, undoable".into(),
                    Status::Converted { .. } => "converted".into(),
                    Status::HasStream { encoding, .. } => format!("stream {encoding}"),
                    Status::InProgress(_) => "in progress".into(),
                    Status::NotConvertible(Refusal::Incomplete) => "incomplete".into(),
                    Status::NotConvertible(Refusal::Diverged { entry: Some(3), .. }) => {
                        "diverged at 3".into()
                    }
                    other => format!("{other:?}"),
                };
                (e.id.clone(), e.place, s)
            })
            .collect()
    };
    let expected = |g2: &str, g3: &str, g5: &str, g6: &str, g9: &str| {
        [
            ("g1", Place::Hosted, "stream jsonl"),
            ("g2", Place::Hosted, g2),
            ("g3", Place::Hosted, g3),
            ("g4", Place::Hosted, "in progress"),
            ("g5", Place::Expired, g5),
            ("g6", Place::Expired, g6),
            ("g7", Place::Hosted, "incomplete"),
            ("g8", Place::Hosted, "diverged at 3"),
            ("g9", Place::Hosted, g9),
        ]
        .map(|(id, p, s)| (id.to_string(), p, s.to_string()))
        .to_vec()
    };
    assert_eq!(
        statuses(&dry),
        expected(
            "converted",
            "converted",
            "converted",
            "converted",
            "converted, undoable"
        )
    );

    let wrote = run(dir, false);
    assert_eq!(statuses(&wrote), statuses(&dry));
    let after = files(dir);
    for (path, bytes) in &before {
        if !path.to_str().unwrap().ends_with(".tmp") {
            assert_eq!(after.get(path), Some(bytes), "{} changed", path.display());
        }
    }
    assert!(!dir.join(".g2.events.cbor.backfill.tmp").exists());
    let written: BTreeSet<&PathBuf> = after.keys().filter(|p| !before.contains_key(*p)).collect();
    let streams = [
        (dir.join("g2.events.cbor"), &finished),
        (dir.join("g3.events.cbor"), &resigned),
        (dir.join("expired/g5.events.cbor"), &stopped),
        (dir.join("expired/g6.events.cbor"), &retired),
        (dir.join("g9.events.cbor"), &undoable),
    ];
    assert_eq!(written, streams.iter().map(|(p, _)| p).collect());
    for (path, twins) in &streams {
        let read = stream(path);
        let id = twins.old.id();
        assert_eq!(
            read.header,
            recording::reconstructed_header(id, AT, twins.old.history())
        );
        assert_eq!(
            events::Header {
                reconstructed: None,
                ..read.header.clone()
            },
            recording::header(id)
        );
        assert!(read.events == twins.expected(), "{id}");
    }

    let again = run(dir, false);
    assert_eq!(files(dir), after, "a second run changes nothing");
    assert_eq!(
        statuses(&again),
        expected(
            "stream cbor",
            "stream cbor",
            "stream cbor",
            "stream cbor",
            "stream cbor"
        )
    );
    assert!(matches!(
        again["g9"].status,
        Status::HasStream { unserved: true, .. }
    ));

    // The server serves a backfilled stream once the game can't change, and goes on with
    // a game in progress unrecorded; once that has ended, a run converts it too.
    let (hub, loaded) = Hub::open(dir, KEEP).unwrap();
    assert_eq!((loaded.unrecorded.len(), loaded.stale.len()), (0, 0));
    assert_eq!(
        (loaded.reconstructed, loaded.reconstructed_unserved),
        (2, 1)
    );
    assert_eq!(files(dir), after, "the server starts without touching them");
    let g2 = fs::read(dir.join("g2.events.cbor")).unwrap();
    assert_eq!(hub.recording("g2").unwrap(), (Encoding::Cbor, g2));
    let g6 = fs::read(dir.join("expired/g6.events.cbor")).unwrap();
    assert_eq!(hub.recording("g6").unwrap(), (Encoding::Cbor, g6));
    assert!(hub.recording("g7").is_err());
    assert!(matches!(hub.recording("g9"), Err(HubError::Conflict(_))));
    hub.retire("g9").unwrap();
    let real = backfill::run(
        dir,
        &Options {
            dry_run: false,
            encoding: Encoding::Cbor,
            at: AT.into(),
        },
        |_| {},
    );
    assert_eq!(
        real.unwrap_err().kind(),
        std::io::ErrorKind::ResourceBusy,
        "a run that writes needs the server stopped"
    );
    let seats: Vec<_> = (0..2)
        .map(|i| hub.find(&format!("g4-{i}")).unwrap())
        .collect();
    while let Some((seat, d)) = seats
        .iter()
        .find_map(|s| Some((s, hub.view(s).unwrap().0.decision?)))
    {
        let answer = Answer::indices(&d.id, vec![d.options.last().unwrap().index]);
        hub.answer(seat, &answer).unwrap();
    }
    assert!(matches!(
        run(dir, true)["g4"].status,
        Status::Converted { .. }
    ));
    drop(hub);
    let last = run(dir, false);
    assert!(matches!(last["g4"].status, Status::Converted { .. }));
    assert!(matches!(
        last["g9"].status,
        Status::HasStream {
            unserved: false,
            ..
        }
    ));
    let read = stream(&dir.join("g4.events.cbor"));
    assert!(matches!(read.events.last().unwrap().what, What::GameEnd(_)));
    dominion_agent::replay::replay(&read.events).unwrap();
    let read = stream(&dir.join("expired/g9.events.cbor"));
    assert!(read.events == undoable.expected());
    let (hub, _) = Hub::open(dir, KEEP).unwrap();
    let g9 = fs::read(dir.join("expired/g9.events.cbor")).unwrap();
    assert_eq!(hub.recording("g9").unwrap(), (Encoding::Cbor, g9));
}

/// Writes `events` as the reconstructed stream of `table`'s game as it stands at `path`.
fn write_reconstructed(path: &Path, table: &Table, events: &[Event]) {
    let encoding = Encoding::of_file(path.to_str().unwrap()).unwrap();
    let mut bytes = vec![];
    let header = recording::reconstructed_header(table.id(), AT, table.history());
    encoding.write(&mut bytes, &header).unwrap();
    for event in events {
        encoding.write(&mut bytes, event).unwrap();
    }
    fs::write(path, bytes).unwrap();
}

#[test]
fn a_stopped_or_lost_recording_gets_a_whole_stream_beside_what_is_left_of_it() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    // g1's recording stopped, leaving an incomplete stream.
    let mut stopped = Twins::new("g1", 21);
    stopped.finish(21);
    let mut table = stopped.live.clone();
    table.stop_recording("a test stopped it");
    // g2 records, but its stream is gone.
    let mut lost = Twins::new("g2", 22);
    lost.finish(22);
    // g3 records, but its stream isn't its own, and the server hasn't renamed it yet.
    let mut wrong = Twins::new("g3", 23);
    wrong.finish(23);
    {
        let store = Store::open(dir).unwrap();
        let mut saved = stopped.saved(&no_undo());
        saved["table"] = serde_json::to_value(&table).unwrap();
        store.save("g1", &saved).unwrap();
        for twins in [&lost, &wrong] {
            let mut table = twins.live.clone();
            table.mark_written();
            let mut saved = twins.saved(&no_undo());
            saved["table"] = serde_json::to_value(&table).unwrap();
            store.save(twins.old.id(), &saved).unwrap();
        }
    }
    fs::write(dir.join("g1.incomplete.events.jsonl"), b"what was written").unwrap();
    fs::write(
        dir.join("g3.events.jsonl"),
        b"{\"schema\":\"dominion/events@1\",\"viewer\":\"full\",\"game\":\"g9\"}\n",
    )
    .unwrap();
    let before = files(dir);

    let replaced = |entries: &BTreeMap<String, Entry>| -> Vec<String> {
        entries
            .values()
            .map(|e| match &e.status {
                Status::Converted {
                    replaces: Some(why),
                    ..
                } => why.clone(),
                other => panic!("{}: {other:?}", e.id),
            })
            .collect()
    };
    let dry = replaced(&run(dir, true));
    assert_eq!(files(dir), before, "a dry run writes nothing");
    assert!(dry[0].contains("a test stopped it"), "{dry:?}");
    assert!(dry[1].contains("missing"), "{dry:?}");
    assert!(dry[2].contains("not its stream"), "{dry:?}");
    assert_eq!(replaced(&run(dir, false)), dry);

    let after = files(dir);
    assert_eq!(
        after.get(&dir.join("g1.incomplete.events.jsonl")),
        before.get(&dir.join("g1.incomplete.events.jsonl"))
    );
    assert!(!dir.join("g3.events.jsonl").exists());
    assert_eq!(
        after.get(&dir.join("g3.incomplete.events.jsonl")),
        before.get(&dir.join("g3.events.jsonl")),
        "a stream not in step is kept, marked incomplete, as the server would"
    );
    for twins in [&stopped, &lost, &wrong] {
        let id = twins.old.id();
        let read = stream(&dir.join(format!("{id}.events.cbor")));
        assert_eq!(
            read.header,
            recording::reconstructed_header(id, AT, twins.old.history())
        );
        check_stream(&read.events, twins);
    }

    // The server takes them as they are: neither unrecorded nor repaired, and served.
    let (hub, loaded) = Hub::open(dir, KEEP).unwrap();
    assert_eq!(loaded.unrecorded, vec![]);
    assert_eq!((loaded.reconstructed, loaded.never_recorded), (3, 0));
    for id in ["g1", "g2", "g3"] {
        let bytes = fs::read(dir.join(format!("{id}.events.cbor"))).unwrap();
        assert_eq!(hub.recording(id).unwrap(), (Encoding::Cbor, bytes));
    }
    drop(hub);
    assert_eq!(files(dir), after);
    let again = run(dir, false);
    assert!(again.values().all(|e| matches!(
        e.status,
        Status::HasStream {
            encoding: Encoding::Cbor,
            unserved: false
        }
    )));
    assert_eq!(files(dir), after);

    // A damaged one is reported and left as it is, and never served.
    let path = dir.join("g2.events.cbor");
    let mut bytes = fs::read(&path).unwrap();
    bytes.pop();
    fs::write(&path, &bytes).unwrap();
    let damaged = files(dir);
    let (hub, loaded) = Hub::open(dir, KEEP).unwrap();
    assert_eq!(loaded.unrecorded.len(), 1);
    assert!(loaded.unrecorded[0].1.contains("reconstructed"));
    assert!(hub.recording("g2").is_err());
    drop(hub);
    assert_eq!(files(dir), damaged);
    assert!(matches!(
        run(dir, false)["g2"].status,
        Status::NotConvertible(Refusal::Foreign(_))
    ));
    assert_eq!(files(dir), damaged);
}

/// Was: such a stream (which no backfill wrote then) was refused at startup. Now one of the
/// game as it stands is kept and served once the game can't change, and set aside as soon
/// as the game changes.
#[test]
fn a_reconstructed_stream_is_never_served_while_the_game_can_change() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    let mut over = Twins::new("g1", 31);
    over.finish(31);
    let mut playing = Twins::new("g2", 32);
    playing.play(32, 30);
    {
        let store = Store::open(dir).unwrap();
        store.save("g1", &over.saved(&Meta::default())).unwrap();
        store.save("g2", &playing.saved(&no_undo())).unwrap();
    }
    write_reconstructed(&dir.join("g1.events.jsonl"), &over.old, over.expected());
    write_reconstructed(
        &dir.join("g2.events.jsonl"),
        &playing.old,
        playing.expected(),
    );
    let before = files(dir);
    let (hub, loaded) = Hub::open(dir, KEEP).unwrap();
    assert_eq!((loaded.unrecorded.len(), loaded.stale.len()), (0, 0));
    assert_eq!(
        (loaded.reconstructed, loaded.reconstructed_unserved),
        (0, 2)
    );
    assert!(matches!(hub.recording("g1"), Err(HubError::Conflict(_))));
    assert!(matches!(hub.recording("g2"), Err(HubError::Conflict(_))));
    assert_eq!(files(dir), before, "neither rejected nor rewritten");
    drop(hub);
    let entries = run(dir, false);
    for id in ["g1", "g2"] {
        assert!(
            matches!(entries[id].status, Status::HasStream { unserved: true, .. }),
            "{id}: {:?}",
            entries[id].status
        );
    }
    assert_eq!(files(dir), before);

    // An answer makes g2's stream another history's: set aside, never served.
    let (hub, _) = Hub::open(dir, KEEP).unwrap();
    let seats: Vec<_> = (0..2)
        .map(|i| hub.find(&format!("g2-{i}")).unwrap())
        .collect();
    let (seat, d) = seats
        .iter()
        .find_map(|s| Some((s, hub.view(s).unwrap().0.decision?)))
        .unwrap();
    hub.answer(seat, &Answer::indices(&d.id, vec![d.options[0].index]))
        .unwrap();
    assert!(!dir.join("g2.events.jsonl").exists());
    assert_eq!(
        fs::read(dir.join("g2.stale.events.jsonl")).unwrap(),
        before[&dir.join("g2.events.jsonl")]
    );
    assert!(dir.join("g1.events.jsonl").exists());
}

/// Copies saved games `tests/saved/<name>` into `to`.
fn copy_saved(name: &str, to: &Path) {
    fs::create_dir_all(to).unwrap();
    let from = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/saved")
        .join(name);
    for entry in fs::read_dir(from).unwrap() {
        let path = entry.unwrap().path();
        fs::copy(&path, to.join(path.file_name().unwrap())).unwrap();
    }
}

/// The real old snapshots the recording tests keep (`tests/saved`): one saved before
/// recording, one recorded by a build whose recording state had no version.
#[test]
fn the_saved_snapshots_of_older_builds_backfill() {
    for name in ["before-events", "events-unversioned"] {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        copy_saved(name, dir);
        let old_stream = fs::read(dir.join("g1.events.jsonl")).ok();
        let before = files(dir);
        // Hosted and in progress: left for later, its stream as it was.
        let first = run(dir, false);
        assert!(
            matches!(first["g1"].status, Status::InProgress(_)),
            "{name}: {:?}",
            first["g1"].status
        );
        assert_eq!(files(dir), before);

        // Played to its end by this build; it can still be undone, and is written all
        // the same (it used to wait until it was retired).
        let (hub, loaded) = Hub::open(dir, KEEP).unwrap();
        assert_eq!(loaded.unrecorded.len(), usize::from(old_stream.is_some()));
        let seats: Vec<_> = (0..2)
            .map(|i| {
                let saved: Value =
                    serde_json::from_slice(&fs::read(dir.join("g1.json")).unwrap()).unwrap();
                hub.find(saved["tokens"][i].as_str().unwrap()).unwrap()
            })
            .collect();
        while let Some((seat, d)) = seats
            .iter()
            .find_map(|s| Some((s, hub.view(s).unwrap().0.decision?)))
        {
            let answer = Answer::indices(&d.id, vec![d.options.last().unwrap().index]);
            hub.answer(seat, &answer).unwrap();
        }
        drop(hub);
        assert!(matches!(
            run(dir, true)["g1"].status,
            Status::Converted { undoable: true, .. }
        ));

        // Converted, beside the incomplete stream an older build left; served once retired.
        let entry = &run(dir, false)["g1"];
        match &entry.status {
            Status::Converted {
                ending,
                replaces,
                undoable: true,
                ..
            } => {
                assert_eq!(ending, "finished", "{name}");
                assert_eq!(
                    replaces
                        .as_ref()
                        .is_some_and(|r| r.contains("earlier build")),
                    old_stream.is_some(),
                    "{name}: {replaces:?}"
                );
            }
            other => panic!("{name}: {other:?}"),
        }
        let (hub, _) = Hub::open(dir, KEEP).unwrap();
        assert!(matches!(hub.recording("g1"), Err(HubError::Conflict(_))));
        drop(hub);
        Store::open(dir).unwrap().retire("g1").unwrap();
        assert_eq!(
            fs::read(dir.join("expired/g1.incomplete.events.jsonl")).ok(),
            old_stream
        );
        let path = dir.join("expired/g1.events.cbor");
        let read = stream(&path);
        assert!(read.header.reconstructed.is_some());
        let replay = dominion_agent::replay::replay(&read.events).unwrap();
        assert!(replay.game.outcome().is_some());
        let (hub, _) = Hub::open(dir, KEEP).unwrap();
        assert_eq!(
            hub.recording("g1").unwrap(),
            (Encoding::Cbor, fs::read(&path).unwrap())
        );
    }
}

fn copy(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap();
        if path.is_dir() {
            copy(&path, &to.join(name));
        } else if path.extension().is_some_and(|e| e == "json") {
            fs::copy(&path, to.join(name)).unwrap();
        }
    }
}

/// Snapshots saved by the server before games were recorded (`backfill/pre-events`).
#[test]
fn snapshots_saved_before_recording_convert() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    copy(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/backfill/pre-events"),
        dir,
    );
    let first = run(dir, false);
    let ending = |id: &str| match &first[id].status {
        Status::Converted { ending, .. } => ending.clone(),
        other => panic!("{id}: {other:?}"),
    };
    assert_eq!(ending("g1"), "finished");
    assert_eq!(ending("g2"), "P2 out (abandoned)");
    assert_eq!(ending("g3"), "in progress");
    assert!(matches!(first["g6"].status, Status::InProgress(_)));
    // A finished game made directly, whose rules allow undo: written all the same (it
    // used to wait until it was retired).
    assert!(matches!(
        first["g4"].status,
        Status::Converted { undoable: true, .. }
    ));
    // Star Chart and Avoid now shuffle again the cards they leave, so the engine plays
    // this game differently from its first such choice on: it gets no stream.
    assert!(matches!(
        first["g5"].status,
        Status::NotConvertible(Refusal::Diverged { .. })
    ));
    for id in ["g4", "g5"] {
        Store::open(dir).unwrap().retire(id).unwrap();
    }
    let second = run(dir, false);
    assert!(matches!(
        second["g4"].status,
        Status::HasStream {
            unserved: false,
            ..
        }
    ));
    assert!(matches!(
        second["g5"].status,
        Status::NotConvertible(Refusal::Diverged { .. })
    ));
    assert!(!dir.join("expired/g5.events.cbor").exists());
    for path in [
        "g2.events.cbor",
        "expired/g4.events.cbor",
        "expired/g1.events.cbor",
        "expired/g3.events.cbor",
    ] {
        let read = stream(&dir.join(path));
        assert!(read.header.reconstructed.is_some());
        dominion_agent::replay::replay(&read.events).unwrap_or_else(|d| panic!("{path}: {d}"));
    }
}

fn saved_table(dir: &Path, id: &str) -> Table {
    let saved: Value =
        serde_json::from_slice(&fs::read(dir.join(format!("{id}.json"))).unwrap()).unwrap();
    serde_json::from_value(saved["table"].clone()).unwrap()
}

fn seats(hub: &Hub, dir: &Path, id: &str) -> Vec<dominion_server::hub::Seat> {
    let saved: Value =
        serde_json::from_slice(&fs::read(dir.join(format!("{id}.json"))).unwrap()).unwrap();
    (0..2)
        .map(|i| hub.find(saved["tokens"][i].as_str().unwrap()).unwrap())
        .collect()
}

/// The finished game of the pre-recording snapshots that can still be undone (g4): written
/// on the first run, kept but not served by the server, set aside once it is undone, and
/// written again, for the game as it then stands, once it is over again.
#[test]
fn a_finished_game_that_can_be_undone_converts_and_is_served_once_it_cannot_change() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    copy(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/backfill/pre-events"),
        dir,
    );
    let first = run(dir, false);
    assert!(matches!(
        first["g4"].status,
        Status::Converted { undoable: true, .. }
    ));
    let path = dir.join("g4.events.cbor");
    let original = fs::read(&path).unwrap();
    let before_undo = saved_table(dir, "g4");
    assert_eq!(
        stream(&path).header.reconstructed.unwrap().history,
        Some(before_undo.history())
    );

    // The server hosts it as before, the stream kept as it is and not served.
    let before = files(dir);
    let (hub, loaded) = Hub::open(dir, KEEP).unwrap();
    assert_eq!((loaded.unrecorded.len(), loaded.stale.len()), (0, 0));
    assert_eq!(
        (loaded.reconstructed, loaded.reconstructed_unserved),
        (1, 1)
    );
    assert_eq!(files(dir), before, "neither rejected nor rewritten");
    assert!(matches!(hub.recording("g4"), Err(HubError::Conflict(_))));

    // Undone, and played on differently: the old stream is set aside, never served.
    let seats = seats(&hub, dir, "g4");
    let middle = before_undo.game().log().len() / 2;
    assert!(hub.undo(&seats[0], Some(middle)).unwrap());
    assert!(!path.exists());
    assert_eq!(
        fs::read(dir.join("g4.stale.events.cbor")).unwrap(),
        original
    );
    assert!(matches!(hub.recording("g4"), Err(HubError::Conflict(_))));
    while let Some((seat, d)) = seats
        .iter()
        .find_map(|s| Some((s, hub.view(s).unwrap().0.decision?)))
    {
        let answer = Answer::indices(&d.id, vec![d.options.last().unwrap().index]);
        hub.answer(seat, &answer).unwrap();
    }
    assert!(matches!(hub.recording("g4"), Err(HubError::Conflict(_))));
    drop(hub);
    let after_undo = saved_table(dir, "g4");
    assert!(after_undo.is_over());
    assert_ne!(after_undo.game().log(), before_undo.game().log());
    assert!(!path.exists());

    // Over again: the next run writes the stream of the game as it now stands.
    let second = run(dir, false);
    assert!(
        matches!(
            second["g4"].status,
            Status::Converted { undoable: true, .. }
        ),
        "{:?}",
        second["g4"].status
    );
    let read = stream(&path);
    assert_eq!(
        read.header.reconstructed.as_ref().unwrap().history,
        Some(after_undo.history())
    );
    let recorded = after_undo.rerecorded().unwrap();
    assert!(
        read.events == recorded.unwritten(),
        "not the stream recorded"
    );
    let replay = dominion_agent::replay::replay(&read.events).unwrap();
    assert_eq!(replay.game.log(), after_undo.game().log());
    assert_eq!(replay.game.outcome(), after_undo.game().outcome());

    // Served once retired, whole, beside the stale one.
    let (hub, loaded) = Hub::open(dir, KEEP).unwrap();
    assert_eq!((loaded.stale.len(), loaded.reconstructed_unserved), (0, 1));
    assert!(matches!(hub.recording("g4"), Err(HubError::Conflict(_))));
    let bytes = fs::read(&path).unwrap();
    hub.retire("g4").unwrap();
    assert_eq!(hub.recording("g4").unwrap(), (Encoding::Cbor, bytes));
    assert_eq!(
        fs::read(dir.join("expired/g4.stale.events.cbor")).unwrap(),
        original
    );
}

/// A game undone and finished again the same way takes the same answers, so the stream
/// it gives is event for event the old one, but its history (an undo since) is another:
/// the old stream is stale, to the server and the backfill alike, whoever finds it.
#[test]
fn a_game_undone_and_finished_the_same_way_has_another_history() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    let mut twins = Twins::new("g1", 41);
    twins.finish(41);
    Store::open(dir)
        .unwrap()
        .save("g1", &twins.saved(&Meta::default()))
        .unwrap();
    assert!(matches!(
        run(dir, false)["g1"].status,
        Status::Converted { undoable: true, .. }
    ));
    let path = dir.join("g1.events.cbor");
    let stale = dir.join("g1.stale.events.cbor");
    let original = fs::read(&path).unwrap();

    // The last answer taken back, and given again.
    let (hub, _) = Hub::open(dir, KEEP).unwrap();
    let (seat, given) = twins.given.last().cloned().unwrap();
    let hub_seat = hub.find(&format!("g1-{}", seat.index())).unwrap();
    assert!(hub.undo(&hub_seat, None).unwrap());
    twins.live.undo(seat).unwrap();
    assert_eq!(fs::read(&stale).unwrap(), original);
    given.to_hub(&hub, &hub_seat);
    given.to_table(&mut twins.live, seat);
    assert!(matches!(hub.recording("g1"), Err(HubError::Conflict(_))));
    drop(hub);
    let table = saved_table(dir, "g1");
    assert!(table.is_over());
    assert_eq!(table.game().log(), twins.old.game().log());
    let old_header = stream(&stale).header.reconstructed.unwrap();
    let was = old_header.history.unwrap();
    let now = table.history();
    assert_eq!((was.answers, was.undos), (now.answers, 0));
    assert_eq!(now.undos, 1);
    assert_ne!(was.digest, now.digest);

    // Put back (a server stopped between the undo's snapshot and setting the stream
    // aside), it is found stale at startup and set aside.
    fs::remove_file(&stale).unwrap();
    fs::write(&path, &original).unwrap();
    let (hub, loaded) = Hub::open(dir, KEEP).unwrap();
    assert_eq!(loaded.stale.len(), 1, "{:?}", loaded.stale);
    assert_eq!(
        (loaded.reconstructed, loaded.reconstructed_unserved),
        (0, 0)
    );
    assert!(!path.exists());
    assert_eq!(fs::read(&stale).unwrap(), original);
    assert!(hub.recording("g1").is_err());
    drop(hub);

    // And by the backfill, which sets it aside and writes the game's.
    fs::remove_file(&stale).unwrap();
    fs::write(&path, &original).unwrap();
    let before = files(dir);
    let is_stale = |status: &Status, set_aside: bool| match status {
        Status::Stale {
            why,
            aside,
            moved,
            then,
        } => {
            assert!(why.contains("1 undo"), "{why}");
            assert_eq!(aside, Path::new("g1.stale.events.cbor"));
            assert_eq!(*moved, set_aside);
            assert!(matches!(**then, Status::Converted { undoable: true, .. }));
        }
        other => panic!("{other:?}"),
    };
    is_stale(&run(dir, true)["g1"].status, false);
    assert_eq!(files(dir), before, "a dry run writes nothing");
    is_stale(&run(dir, false)["g1"].status, true);
    assert_eq!(fs::read(&stale).unwrap(), original);
    let read = stream(&path);
    assert_eq!(read.header.reconstructed.unwrap().history, Some(now));
    assert!(read.events == twins.live.unwritten());
    assert!(
        read.events == stream(&stale).events,
        "the same game, event for event"
    );

    // Once no undo is allowed it can't change, and is served.
    let mut saved: Value = serde_json::from_slice(&fs::read(dir.join("g1.json")).unwrap()).unwrap();
    saved["meta"]["undo"] = json!("off");
    fs::write(dir.join("g1.json"), serde_json::to_vec(&saved).unwrap()).unwrap();
    let (hub, loaded) = Hub::open(dir, KEEP).unwrap();
    assert_eq!(
        (loaded.reconstructed, loaded.reconstructed_unserved),
        (1, 0)
    );
    assert_eq!(
        hub.recording("g1").unwrap(),
        (Encoding::Cbor, fs::read(&path).unwrap())
    );
    dominion_agent::replay::replay(&read.events).unwrap();
}
