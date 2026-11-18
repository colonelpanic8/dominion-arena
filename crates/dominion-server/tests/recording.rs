//! Every hosted game's event stream on disk, beside its snapshot.

use dominion_agent::recording::{self, Encoding};
use dominion_agent::{Answer, Table};
use dominion_protocol::events::{self, Event};
use dominion_protocol::lobby::UndoRule;
use dominion_server::hub::{self, Hub, HubError, Meta, NewGame, Seat};
use serde_json::Value;
use std::fs;
use std::path::Path;
use std::time::Duration;

const KEEP: Duration = Duration::from_secs(3600);

fn new_game() -> NewGame {
    NewGame {
        players: 2,
        kingdom: [
            "Village",
            "Smithy",
            "Militia",
            "Moat",
            "Market",
            "Witch",
            "Cellar",
            "Chapel",
            "Throne Room",
            "Courtyard",
        ]
        .map(String::from)
        .to_vec(),
        seed: Some(5),
        turn_limit: Some(8),
        ..NewGame::default()
    }
}

fn create(hub: &Hub) -> Vec<Seat> {
    create_with(hub, UndoRule::Free)
}

fn create_with(hub: &Hub, undo: UndoRule) -> Vec<Seat> {
    let req = new_game();
    let meta = Meta {
        undo,
        ..Meta::default()
    };
    let config = hub::config(&req).unwrap();
    let created = hub.host(config, vec![], meta).unwrap();
    (created.seats.iter())
        .map(|s| hub.claim(&created.game_id, &s.token).unwrap())
        .collect()
}

/// Game g1's seats, by the tokens it was saved with.
fn seats(hub: &Hub, dir: &Path) -> Vec<Seat> {
    (saved(dir)["tokens"].as_array().unwrap().iter())
        .map(|t| hub.find(t.as_str().unwrap()).unwrap())
        .collect()
}

/// Answers whichever seat is deciding, `n` times or until the game ends: with its first
/// option, or its last with `last`.
fn advance(hub: &Hub, seats: &[Seat], n: usize, last: bool) {
    for _ in 0..n {
        let Some((seat, d)) = seats
            .iter()
            .find_map(|s| Some((s, hub.view(s).unwrap().0.decision?)))
        else {
            return;
        };
        let option = if last {
            d.options.last()
        } else {
            d.options.first()
        };
        let answer = Answer::indices(&d.id, vec![option.unwrap().index]);
        hub.answer(seat, &answer).unwrap();
    }
}

fn saved(dir: &Path) -> Value {
    serde_json::from_slice(&fs::read(dir.join("g1.json")).unwrap()).unwrap()
}

/// The stream of the game the snapshot holds, recorded afresh by replaying its answers
/// (with their Way pre-answers and Autoplay Treasures runs).
fn replayed(saved: &Value) -> Vec<Event> {
    let table: Table = serde_json::from_value(saved["table"].clone()).unwrap();
    table.rerecorded().unwrap().unwritten().to_vec()
}

/// Checks game g1's stream in `dir` against its snapshot: exactly the stream its answers
/// record, folding to the table its recorder keeps.
fn check(dir: &Path, encoding: Encoding) -> Vec<Event> {
    let saved = saved(dir);
    let bytes = fs::read(dir.join(format!("g1{}", encoding.suffix()))).unwrap();
    let read = recording::read(encoding, &bytes, None).unwrap();
    assert_eq!(read.header, recording::header("g1"));
    assert!(
        read.events == replayed(&saved),
        "the stream is not the game's"
    );
    let table: Table = serde_json::from_value(saved["table"].clone()).unwrap();
    assert_eq!(table.recorded(), Some(read.events.len() as u64));
    let mut folded = events::Table::default();
    for event in &read.events {
        folded.apply(event).unwrap();
    }
    assert!(Some(&folded) == table.recorded_table());
    read.events
}

#[test]
fn a_finished_game_leaves_its_stream_beside_its_snapshot() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    let (hub, _) = Hub::open(dir, KEEP).unwrap();
    let seats = create(&hub);
    advance(&hub, &seats, 5, false);
    assert!(hub.list()[0].running);
    check(dir, Encoding::JsonLines);
    assert!(matches!(hub.recording("g1"), Err(HubError::Conflict(_))));
    advance(&hub, &seats, 10_000, false);
    assert!(!hub.list()[0].running);
    let events = check(dir, Encoding::JsonLines);
    assert!(matches!(
        events.last().unwrap().what,
        events::What::GameEnd(_)
    ));

    let file = fs::read(dir.join("g1.events.jsonl")).unwrap();
    // Over, but it can still be undone.
    assert!(matches!(hub.recording("g1"), Err(HubError::Conflict(_))));
    hub.retire("g1").unwrap();
    assert!(!dir.join("g1.events.jsonl").exists());
    assert_eq!(fs::read(dir.join("expired/g1.events.jsonl")).unwrap(), file);
    assert_eq!(
        hub.recording("g1").unwrap(),
        (Encoding::JsonLines, file.clone())
    );
}

#[test]
fn the_stream_is_served_only_once_the_game_cant_go_back() {
    let tmp = tempfile::tempdir().unwrap();
    let (hub, _) = Hub::open(tmp.path(), KEEP).unwrap();
    let free = create_with(&hub, UndoRule::Free);
    advance(&hub, &free, 10_000, false);
    assert!(matches!(hub.recording("g1"), Err(HubError::Conflict(_))));
    // What the stream would have shown can be played again.
    assert!(hub.undo(&free[0], None).unwrap());

    let off = create_with(&hub, UndoRule::Off);
    advance(&hub, &off, 10_000, false);
    assert!(hub.recording("g2").is_ok());
    assert!(matches!(
        hub.undo(&off[0], None),
        Err(HubError::Forbidden(_))
    ));

    // A resignation ends it for good.
    let resigned = create_with(&hub, UndoRule::Free);
    advance(&hub, &resigned, 6, false);
    assert!(matches!(hub.recording("g3"), Err(HubError::Conflict(_))));
    hub.resign(&resigned[1], "resigned").unwrap();
    assert!(hub.recording("g3").is_ok());
    assert!(hub.undo(&resigned[0], None).is_err());
}

#[test]
fn the_encoding_is_a_setting_and_both_read_back_the_same() {
    let mut streams = vec![];
    for encoding in Encoding::ALL {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        let (hub, _) = Hub::open(dir, KEEP).unwrap();
        hub.set_events_encoding(encoding);
        let seats = create_with(&hub, UndoRule::Off);
        advance(&hub, &seats, 10_000, false);
        let other = Encoding::ALL.into_iter().find(|&e| e != encoding).unwrap();
        assert!(!dir.join(format!("g1{}", other.suffix())).exists());
        streams.push(check(dir, encoding));
        // A game keeps the encoding it began in.
        drop(hub);
        let (hub, _) = Hub::open(dir, KEEP).unwrap();
        hub.set_events_encoding(other);
        assert_eq!(hub.recording("g1").unwrap().0, encoding);
    }
    assert!(streams[0] == streams[1]);
}

/// The files of game g1 as they stand.
fn files(dir: &Path) -> (Vec<u8>, Vec<u8>) {
    (
        fs::read(dir.join("g1.json")).unwrap(),
        fs::read(dir.join("g1.events.jsonl")).unwrap(),
    )
}

#[test]
fn after_a_crash_the_stream_is_cut_back_to_the_snapshot_and_goes_on() {
    let tmp = tempfile::tempdir().unwrap();
    let played = tmp.path().join("played");
    let ((snap_a, stream_a), (snap_b, stream_b)) = {
        let (hub, _) = Hub::open(&played, KEEP).unwrap();
        let seats = create(&hub);
        advance(&hub, &seats, 15, false);
        let a = files(&played);
        advance(&hub, &seats, 1, false);
        assert!(hub.list()[0].running);
        (a, files(&played))
    };
    assert!(stream_b.len() > stream_a.len() && stream_b.starts_with(&stream_a));
    let torn = [&stream_a[..], &stream_b[stream_a.len()..stream_a.len() + 9]].concat();
    let junk = [&stream_b[..], b"{\"seq\":"].concat();
    let crashes = [
        // Stopped after saving the snapshot, before writing its events.
        (&snap_b, &stream_a),
        // ... while writing them.
        (&snap_b, &torn),
        // Didn't save the snapshot (the stream holds more than it says was written).
        (&snap_a, &junk),
    ];
    for (i, (snap, stream)) in crashes.into_iter().enumerate() {
        let dir = tmp.path().join(format!("crash{i}"));
        fs::create_dir(&dir).unwrap();
        fs::write(dir.join("g1.json"), snap).unwrap();
        fs::write(dir.join("g1.events.jsonl"), stream).unwrap();
        let (hub, loaded) = Hub::open(&dir, KEEP).unwrap();
        assert_eq!((loaded.games, loaded.unrecorded.len()), (1, 0), "crash {i}");
        let seats = seats(&hub, &dir);
        advance(&hub, &seats, 1, false);
        check(&dir, Encoding::JsonLines);
        advance(&hub, &seats, 10_000, false);
        check(&dir, Encoding::JsonLines);
    }

    // A stream missing events its snapshot says were written can't be mended: the game
    // goes on without one.
    let dir = tmp.path().join("damaged");
    fs::create_dir(&dir).unwrap();
    fs::write(dir.join("g1.json"), &snap_b).unwrap();
    let short = &stream_a[..stream_a[..stream_a.len() - 1]
        .iter()
        .rposition(|&b| b == b'\n')
        .unwrap()
        + 1];
    fs::write(dir.join("g1.events.jsonl"), short).unwrap();
    let (hub, loaded) = Hub::open(&dir, KEEP).unwrap();
    assert_eq!((loaded.games, loaded.unrecorded.len()), (1, 1));
    // Marked incomplete as it is, and the game no longer records.
    assert!(!dir.join("g1.events.jsonl").exists());
    assert_eq!(
        fs::read(dir.join("g1.incomplete.events.jsonl")).unwrap(),
        short
    );
    let seats = seats(&hub, &dir);
    advance(&hub, &seats, 10_000, false);
    assert!(saved(&dir)["table"]["recorder"]["stopped"].is_string());
    assert!(!hub.list()[0].running);
    assert!(!dir.join("g1.events.jsonl").exists());
    drop(hub);
    // Reported at every start.
    let (hub, loaded) = Hub::open(&dir, KEEP).unwrap();
    assert_eq!((loaded.games, loaded.unrecorded.len()), (1, 1));
    hub.retire("g1").unwrap();
    assert!(dir.join("expired/g1.incomplete.events.jsonl").exists());
    assert!(matches!(hub.recording("g1"), Err(HubError::NotFound(_))));
}

#[test]
fn after_an_undo_the_file_is_the_stream_of_the_game_as_it_now_stands() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    let (hub, _) = Hub::open(dir, KEEP).unwrap();
    let seats = create(&hub);
    advance(&hub, &seats, 30, false);
    assert!(hub.list()[0].running);
    let before = check(dir, Encoding::JsonLines);
    assert!(hub.undo(&seats[0], None).unwrap());
    let undone = check(dir, Encoding::JsonLines);
    assert!(undone.len() < before.len() && before.starts_with(&undone));
    // Something else this time.
    advance(&hub, &seats, 3, true);
    advance(&hub, &seats, 10_000, false);
    let after = check(dir, Encoding::JsonLines);
    assert!(after.starts_with(&undone) && !after.starts_with(&before));
}

/// Game g1's snapshot and stream as they are on disk.
fn on_disk(dir: &Path) -> (Vec<u8>, Vec<u8>) {
    (
        fs::read(dir.join("g1.json")).unwrap(),
        fs::read(dir.join("g1.events.jsonl")).unwrap(),
    )
}

/// Steps through finished game g1 forwards, then back and on again, as one seat too.
fn step_through(hub: &Hub) {
    let steps = hub
        .replay_view("g1", 0, 0, None)
        .unwrap()
        .replay
        .unwrap()
        .steps;
    for step in (0..=steps).chain([steps / 2, 1, steps - 1, steps / 3]) {
        let view = hub.replay_view("g1", step, 0, None).unwrap();
        assert_eq!(view.replay.unwrap().step, step);
        hub.replay_view("g1", step, 0, Some(1)).unwrap();
    }
}

#[test]
fn stepping_through_a_finished_game_leaves_its_stream_as_it_was() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    let (hub, _) = Hub::open(dir, KEEP).unwrap();
    let config = hub::config(&new_game()).unwrap();
    let meta = Meta {
        watchable: true,
        undo: UndoRule::Free,
        ..Meta::default()
    };
    let created = hub.host(config, vec![], meta).unwrap();
    let seats: Vec<Seat> = (created.seats.iter())
        .map(|s| hub.claim(&created.game_id, &s.token).unwrap())
        .collect();
    advance(&hub, &seats, 10_000, false);
    assert!(!hub.list()[0].running);
    let finished = on_disk(dir);
    let events = check(dir, Encoding::JsonLines);
    step_through(&hub);
    assert!(
        on_disk(dir) == finished,
        "stepping changed the game on disk"
    );

    // An undo still makes the file the stream of the game as it now stands, and the
    // points replayed before it are no help to the game that follows.
    assert!(hub.undo(&seats[0], None).unwrap());
    let undone = check(dir, Encoding::JsonLines);
    assert!(undone.len() < events.len() && events.starts_with(&undone));
    advance(&hub, &seats, 3, true);
    advance(&hub, &seats, 10_000, false);
    let after = check(dir, Encoding::JsonLines);
    assert!(after.starts_with(&undone) && !after.starts_with(&events));
    let finished = on_disk(dir);
    step_through(&hub);
    assert!(
        on_disk(dir) == finished,
        "stepping changed the game on disk"
    );
    check(dir, Encoding::JsonLines);

    // Once retired, its stream is served as written.
    hub.retire("g1").unwrap();
    step_through(&hub);
    assert_eq!(
        hub.recording("g1").unwrap(),
        (Encoding::JsonLines, finished.1)
    );
}

/// An answer that plays a card with a Way answered in advance (the table's Way buttons)
/// is recorded as the play's question and the Way question, each asked and answered.
#[test]
fn an_answer_played_with_a_way_records_the_way_question_and_its_answer() {
    use dominion_agent::WayChoice;
    for seed in 0..50 {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        let (hub, _) = Hub::open(dir, KEEP).unwrap();
        let req = NewGame {
            landscapes: vec!["Way of the Sheep".into()],
            shelters: Some(true),
            seed: Some(seed),
            turn_limit: Some(4),
            ..new_game()
        };
        let created = (hub.host(hub::config(&req).unwrap(), vec![], Meta::default())).unwrap();
        let seats: Vec<Seat> = (created.seats.iter())
            .map(|s| hub.claim(&created.game_id, &s.token).unwrap())
            .collect();
        let (seat, d) = (seats.iter())
            .find_map(|s| Some((s, hub.view(s).unwrap().0.decision?)))
            .unwrap();
        let Some(necropolis) = (d.options.iter())
            .find(|o| o.action == "play" && o.card.as_deref() == Some("Necropolis"))
        else {
            continue;
        };
        let answer = Answer::indices(&d.id, vec![necropolis.index]);
        let way = Some(WayChoice::Way(dominion_engine::Card::WayOfTheSheep));
        hub.answer_with(seat, &answer, way).unwrap();
        let next = hub.view(seat).unwrap().0.decision.unwrap();
        assert_eq!(next.kind, "turn", "the Way question was left open");
        advance(&hub, &seats, 10_000, false);

        let events = check(dir, Encoding::JsonLines);
        let asked = (events.windows(2)).position(|w| match (&w[0].what, &w[1].what) {
            (events::What::Ask(a), events::What::Answer(b)) => {
                a.seat == b.seat
                    && a.source.as_deref() == Some("Necropolis")
                    && a.options
                        .iter()
                        .any(|o| o.label.contains("Way of the Sheep"))
            }
            _ => false,
        });
        assert!(asked.is_some(), "no Way question asked and answered");
        let replayed = dominion_agent::replay::replay(&events).unwrap();
        let saved: Table = serde_json::from_value(saved(dir)["table"].clone()).unwrap();
        assert_eq!(replayed.game.outcome(), saved.game().outcome());
        return;
    }
    panic!("no seed starts with Necropolis in hand");
}

#[test]
fn a_game_saved_before_streams_still_loads_and_plays() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    {
        let (hub, _) = Hub::open(dir, KEEP).unwrap();
        let seats = create(&hub);
        advance(&hub, &seats, 20, false);
    }
    let mut old = saved(dir);
    let table = old["table"].as_object_mut().unwrap();
    assert!(table.remove("recorder").is_some());
    table.remove("unwritten");
    assert!(
        table["game"]
            .as_object_mut()
            .unwrap()
            .remove("journal")
            .is_some()
    );
    fs::write(dir.join("g1.json"), serde_json::to_vec(&old).unwrap()).unwrap();
    fs::remove_file(dir.join("g1.events.jsonl")).unwrap();

    let (hub, loaded) = Hub::open(dir, KEEP).unwrap();
    assert_eq!((loaded.games, loaded.unrecorded.len()), (1, 0));
    let seats = seats(&hub, dir);
    advance(&hub, &seats, 10, false);
    assert!(hub.undo(&seats[0], None).unwrap());
    advance(&hub, &seats, 10_000, false);
    assert!(!hub.list()[0].running);
    assert!(!dir.join("g1.events.jsonl").exists());
    assert!(saved(dir)["table"].get("recorder").is_none());
    hub.retire("g1").unwrap();
    assert!(matches!(hub.recording("g1"), Err(HubError::NotFound(_))));
}

/// Buying Borrow puts out the -1 Card token, which the journal once kept where JSON
/// couldn't write it: every snapshot after it failed while the game went on.
#[test]
fn a_game_with_tokens_out_saves_restarts_and_undoes() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    let (hub, _) = Hub::open(dir, KEEP).unwrap();
    let mut req = new_game();
    req.landscapes = vec!["Borrow".into(), "Training".into()];
    let created = hub.create(&req).unwrap();
    let seats: Vec<Seat> = (created.seats.iter())
        .map(|s| hub.claim(&created.game_id, &s.token).unwrap())
        .collect();
    let mut borrowed = false;
    while !borrowed {
        let (seat, d) = seats
            .iter()
            .find_map(|s| Some((s, hub.view(s).unwrap().0.decision?)))
            .unwrap();
        let borrow =
            (d.options.iter()).find(|o| o.action == "buy-landscape" && o.label.contains("Borrow"));
        borrowed = borrow.is_some();
        let option = borrow.unwrap_or(&d.options[0]);
        hub.answer(seat, &Answer::indices(&d.id, vec![option.index]))
            .unwrap();
    }
    let decision = |hub: &Hub| {
        (seats.iter())
            .find_map(|s| hub.view(s).unwrap().0.decision)
            .unwrap()
            .id
    };
    let before = decision(&hub);
    assert_eq!(saved(dir)["table"]["recorder"]["version"], 1);
    drop(hub);
    let (hub, loaded) = Hub::open(dir, KEEP).unwrap();
    assert_eq!((loaded.games, loaded.unrecorded.len()), (1, 0));
    assert_eq!(decision(&hub), before);
    let seats = self::seats(&hub, dir);
    advance(&hub, &seats, 1, false);
    assert!(hub.undo(&seats[0], None).unwrap());
    advance(&hub, &seats, 10_000, false);
    assert!(!hub.list()[0].running);
    check(dir, Encoding::JsonLines);
}

/// A copy of saved game `name` (`tests/saved/<name>`) in a new state directory.
fn saved_copy(name: &str) -> tempfile::TempDir {
    let tmp = tempfile::tempdir().unwrap();
    let from = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/saved")
        .join(name);
    for entry in fs::read_dir(from).unwrap() {
        let path = entry.unwrap().path();
        fs::copy(&path, tmp.path().join(path.file_name().unwrap())).unwrap();
    }
    tmp
}

/// A game saved by the server before games were recorded (commit `cfefa17`) plays on as
/// it was, unrecorded.
#[test]
fn a_game_saved_before_recording_plays_on_unrecorded() {
    let tmp = saved_copy("before-events");
    let dir = tmp.path();
    let before = saved(dir);
    let (hub, loaded) = Hub::open(dir, KEEP).unwrap();
    assert_eq!(
        (loaded.games, loaded.unrecorded.len(), loaded.never_recorded),
        (1, 0, 1)
    );
    let seats = seats(&hub, dir);
    let d = (seats.iter())
        .find_map(|s| hub.view(s).unwrap().0.decision)
        .unwrap();
    assert_eq!(d.id, format!("d{}", before["table"]["seq"]));
    advance(&hub, &seats, 3, false);
    assert!(hub.undo(&seats[0], None).unwrap());
    advance(&hub, &seats, 10_000, false);
    assert!(!hub.list()[0].running);
    assert!(saved(dir)["table"].get("recorder").is_none());
    assert!(!dir.join("g1.events.jsonl").exists());
}

/// A game recorded by a build whose recording state had no version (this branch before
/// it was versioned, `1be2eff`) plays on as it was; its stream stops where it was and is
/// marked incomplete, and it is reported.
#[test]
fn a_game_recorded_by_an_earlier_build_plays_on_with_its_stream_stopped() {
    let tmp = saved_copy("events-unversioned");
    let dir = tmp.path();
    let before = saved(dir);
    let stream = fs::read(dir.join("g1.events.jsonl")).unwrap();
    let (hub, loaded) = Hub::open(dir, KEEP).unwrap();
    assert_eq!((loaded.games, loaded.unrecorded.len()), (1, 1));
    assert!(
        loaded.unrecorded[0].1.contains("earlier build"),
        "{:?}",
        loaded.unrecorded
    );
    assert!(!dir.join("g1.events.jsonl").exists());
    assert_eq!(
        fs::read(dir.join("g1.incomplete.events.jsonl")).unwrap(),
        stream
    );
    let seats = seats(&hub, dir);
    let d = (seats.iter())
        .find_map(|s| hub.view(s).unwrap().0.decision)
        .unwrap();
    assert_eq!(d.id, format!("d{}", before["table"]["seq"]));
    advance(&hub, &seats, 1, false);
    let after = saved(dir);
    assert!(after["table"]["recorder"]["stopped"].is_string());
    assert!(after["table"]["game"].get("journal").is_none());
    let history = after["table"]["history"].as_array().unwrap();
    assert_eq!(
        history[..history.len() - 1],
        before["table"]["history"].as_array().unwrap()[..]
    );
    advance(&hub, &seats, 2, false);
    assert!(hub.undo(&seats[0], None).unwrap());
    advance(&hub, &seats, 10_000, false);
    assert!(!hub.list()[0].running);
    assert!(!dir.join("g1.events.jsonl").exists());
    assert_eq!(
        fs::read(dir.join("g1.incomplete.events.jsonl")).unwrap(),
        stream
    );
}
