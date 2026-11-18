//! `dominion games backfill`: streams for games saved before the server recorded them.

use dominion_agent::recording::{self, Encoding};
use dominion_agent::{Answer, Table, WayChoice};
use dominion_engine::{Card, GameConfig, PlayerId};
use dominion_protocol::events::{self, Event, What};
use dominion_server::backfill::{self, Ending, Entry, Options, Place, Refusal, Status};
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
}

impl Twins {
    fn new(id: &str, seed: u64) -> Twins {
        let names = vec!["Ann".to_string(), "Bo".to_string()];
        Twins {
            old: Table::new(id, config(seed), names.clone()).unwrap(),
            live: Table::new_recorded(id, config(seed), names).unwrap(),
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
                continue;
            }
            let answer = if d.name_any {
                Answer::name(&d.id, "Copper")
            } else {
                let (min, max) = (d.min.unwrap_or(1) as usize, d.max.unwrap_or(1) as usize);
                let mut picks: Vec<u32> = (0..d.options.len() as u32).collect();
                picks.shuffle(&mut rng);
                picks.truncate(rng.random_range(min..=max.min(picks.len())));
                Answer::indices(&d.id, picks)
            };
            let way = match rng.random_range(0..4) {
                0 => Some(WayChoice::Way(Card::WayOfTheOx)),
                1 => Some(WayChoice::None),
                _ => None,
            };
            self.old.answer_with(seat, &answer, way).unwrap();
            self.live.answer_with(seat, &answer, way).unwrap();
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

/// Every file under `dir`, with its bytes.
fn files(dir: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut out = BTreeMap::new();
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
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
    {
        let store = Store::open(dir).unwrap();
        store.save("g2", &finished.saved(&Meta::default())).unwrap();
        store
            .save("g3", &resigned.saved(&ended("abandoned")))
            .unwrap();
        store.save("g4", &playing.saved(&Meta::default())).unwrap();
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
                    Status::Converted { .. } => "converted".into(),
                    Status::HasStream(e) => format!("stream {e}"),
                    Status::InProgress(Ok(_)) => "in progress".into(),
                    Status::InProgress(Err(why)) => format!("in progress: {why}"),
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
    let expected = |g2: &str, g3: &str, g5: &str, g6: &str| {
        [
            ("g1", Place::Hosted, "stream jsonl"),
            ("g2", Place::Hosted, g2),
            ("g3", Place::Hosted, g3),
            ("g4", Place::Hosted, "in progress"),
            ("g5", Place::Expired, g5),
            ("g6", Place::Expired, g6),
            ("g7", Place::Hosted, "incomplete"),
            ("g8", Place::Hosted, "diverged at 3"),
        ]
        .map(|(id, p, s)| (id.to_string(), p, s.to_string()))
        .to_vec()
    };
    assert_eq!(
        statuses(&dry),
        expected("converted", "converted", "converted", "converted")
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
    ];
    assert_eq!(written, streams.iter().map(|(p, _)| p).collect());
    for (path, twins) in &streams {
        let read = stream(path);
        let id = twins.old.id();
        assert_eq!(read.header, recording::reconstructed_header(id, AT));
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
        expected("stream cbor", "stream cbor", "stream cbor", "stream cbor")
    );

    // The server serves a backfilled stream once the game is over, and goes on with a
    // game in progress unrecorded; once that has ended, a run converts it too.
    let (hub, loaded) = Hub::open(dir, KEEP).unwrap();
    assert_eq!(loaded.unrecorded.len(), 0);
    let g2 = fs::read(dir.join("g2.events.cbor")).unwrap();
    assert_eq!(hub.recording("g2").unwrap(), (Encoding::Cbor, g2));
    let g6 = fs::read(dir.join("expired/g6.events.cbor")).unwrap();
    assert_eq!(hub.recording("g6").unwrap(), (Encoding::Cbor, g6));
    assert!(hub.recording("g7").is_err());
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
    let read = stream(&dir.join("g4.events.cbor"));
    assert!(matches!(read.events.last().unwrap().what, What::GameEnd(_)));
    dominion_agent::replay::replay(&read.events).unwrap();
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
    assert_eq!(ending("g4"), "finished");
    assert_eq!(ending("g5"), "finished");
    assert!(matches!(first["g6"].status, Status::InProgress(Ok(_))));
    for path in [
        "g2.events.cbor",
        "g4.events.cbor",
        "g5.events.cbor",
        "expired/g1.events.cbor",
        "expired/g3.events.cbor",
    ] {
        let read = stream(&dir.join(path));
        assert!(read.header.reconstructed.is_some());
        dominion_agent::replay::replay(&read.events).unwrap_or_else(|d| panic!("{path}: {d}"));
    }
}
