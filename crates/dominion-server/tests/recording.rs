//! Every hosted game's event stream on disk, beside its snapshot.

use dominion_agent::recording::{self, Encoding};
use dominion_agent::{Answer, Table};
use dominion_engine::{PlayerId, Response};
use dominion_protocol::events::{self, Event};
use dominion_server::hub::{Hub, HubError, NewGame, Seat};
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
    let created = hub.create(&new_game()).unwrap();
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

/// The stream of the game the snapshot holds, recorded afresh by replaying its answers.
fn replayed(saved: &Value) -> Vec<Event> {
    let table: Table = serde_json::from_value(saved["table"].clone()).unwrap();
    let config = table.game().config().clone();
    let mut fresh = Table::new_recorded(table.id(), config, table.names().to_vec()).unwrap();
    for step in saved["table"]["history"].as_array().unwrap() {
        let seat = PlayerId(step["seat"].as_u64().unwrap() as u8);
        let id = fresh.decision_id().unwrap();
        if step["act"] == "Treasures" {
            fresh.play_treasures(seat, &id).unwrap();
        } else {
            let response: Response =
                serde_json::from_value(step["act"]["Answer"]["response"].clone()).unwrap();
            fresh.respond(seat, &id, response).unwrap();
        }
    }
    fresh.unwritten().to_vec()
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
    assert_eq!(
        hub.recording("g1").unwrap(),
        (Encoding::JsonLines, file.clone())
    );
    hub.retire("g1").unwrap();
    assert!(!dir.join("g1.events.jsonl").exists());
    assert_eq!(fs::read(dir.join("expired/g1.events.jsonl")).unwrap(), file);
    assert_eq!(hub.recording("g1").unwrap().1, file);
}

#[test]
fn the_encoding_is_a_setting_and_both_read_back_the_same() {
    let mut streams = vec![];
    for encoding in Encoding::ALL {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        let (hub, _) = Hub::open(dir, KEEP).unwrap();
        hub.set_events_encoding(encoding);
        let seats = create(&hub);
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
    let seats = seats(&hub, &dir);
    advance(&hub, &seats, 10_000, false);
    assert!(!hub.list()[0].running);
    assert_eq!(fs::read(dir.join("g1.events.jsonl")).unwrap(), short);
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
    assert!(matches!(hub.recording("g1"), Err(HubError::NotFound(_))));
}
