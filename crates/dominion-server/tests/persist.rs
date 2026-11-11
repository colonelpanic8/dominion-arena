use dominion_agent::Answer;
use dominion_server::hub::{Hub, NewGame, Seat};
use serde_json::Value;
use std::path::Path;
use std::time::Duration;

const KEEP: Duration = Duration::from_secs(3600);

fn new_game(turn_limit: Option<u32>) -> NewGame {
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
        seed: Some(3),
        turn_limit,
        ..NewGame::default()
    }
}

fn create(hub: &Hub, turn_limit: Option<u32>) -> (Vec<String>, Vec<Seat>) {
    let created = hub.create(&new_game(turn_limit)).unwrap();
    let tokens: Vec<String> = created.seats.into_iter().map(|s| s.token).collect();
    let seats = tokens
        .iter()
        .map(|t| hub.claim(&created.game_id, t).unwrap())
        .collect();
    (tokens, seats)
}

/// Answers whichever seat is deciding with its first option, `n` times or until the
/// game ends.
fn advance(hub: &Hub, seats: &[Seat], n: usize) {
    for _ in 0..n {
        let Some((seat, d)) = seats
            .iter()
            .find_map(|s| Some((s, hub.view(s).unwrap().0.decision?)))
        else {
            return;
        };
        let answer = Answer::indices(&d.id, vec![d.options[0].index]);
        hub.answer(seat, &answer).unwrap();
    }
}

fn views(hub: &Hub, seats: &[Seat]) -> Vec<Value> {
    seats
        .iter()
        .map(|s| serde_json::to_value(hub.view(s).unwrap().0).unwrap())
        .collect()
}

#[cfg(unix)]
fn mode(path: &Path) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path).unwrap().permissions().mode() & 0o777
}

#[test]
fn games_survive_a_restart() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("games");
    let (tokens, before) = {
        let (hub, loaded) = Hub::open(&dir, KEEP).unwrap();
        assert_eq!(loaded.games, 0);
        let (tokens, seats) = create(&hub, None);
        advance(&hub, &seats, 25);
        (tokens, views(&hub, &seats))
    };
    #[cfg(unix)]
    {
        assert_eq!(mode(&dir), 0o700);
        assert_eq!(mode(&dir.join("g1.json")), 0o600);
    }

    let (hub, loaded) = Hub::open(&dir, KEEP).unwrap();
    assert_eq!(loaded.games, 1);
    let seats: Vec<Seat> = tokens.iter().map(|t| hub.find(t).unwrap()).collect();
    assert_eq!(seats[1].seat.0, 1);
    assert_eq!(views(&hub, &seats), before);
    advance(&hub, &seats, 10);
    assert_ne!(views(&hub, &seats), before);
    assert_eq!(hub.create(&new_game(None)).unwrap().game_id, "g2");
}

#[test]
fn finished_games_expire_and_unreadable_files_are_left_alone() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    {
        let (hub, _) = Hub::open(dir, KEEP).unwrap();
        let (_, finished) = create(&hub, Some(2));
        advance(&hub, &finished, 10_000);
        create(&hub, None);
        let running: Vec<bool> = hub.list().iter().map(|g| g.running).collect();
        assert_eq!(running, [false, true]);
    }
    std::fs::write(dir.join("g9.json"), "not a game").unwrap();

    let (_, loaded) = Hub::open(dir, KEEP).unwrap();
    assert_eq!((loaded.games, loaded.expired), (2, 0));
    assert_eq!(loaded.unreadable.len(), 1);

    let (hub, loaded) = Hub::open(dir, Duration::ZERO).unwrap();
    assert_eq!((loaded.games, loaded.expired), (1, 1));
    assert!(!dir.join("g1.json").exists());
    assert!(dir.join("g9.json").exists());
    assert_eq!(hub.list()[0].game_id, "g2");
    // Not g3: ids run on from the highest file, so g9.json is never written over.
    assert_eq!(hub.create(&new_game(None)).unwrap().game_id, "g10");
    assert_eq!(
        std::fs::read_to_string(dir.join("g9.json")).unwrap(),
        "not a game"
    );
}

#[test]
fn files_that_do_not_hold_their_game_are_skipped() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    {
        let (hub, _) = Hub::open(dir, KEEP).unwrap();
        create(&hub, None);
    }
    let saved = std::fs::read_to_string(dir.join("g1.json")).unwrap();
    // A copy would be a second g1.
    std::fs::write(dir.join("g5.json"), &saved).unwrap();
    let mut seatless: Value = serde_json::from_str(&saved).unwrap();
    seatless["table"]["id"] = "g3".into();
    seatless["tokens"].as_array_mut().unwrap().pop();
    std::fs::write(dir.join("g3.json"), seatless.to_string()).unwrap();
    // What a server that died while saving leaves behind.
    std::fs::write(dir.join(".g1.json.tmp"), &saved[..saved.len() / 2]).unwrap();

    let (hub, loaded) = Hub::open(dir, KEEP).unwrap();
    assert_eq!(loaded.games, 1);
    let mut skipped: Vec<String> = loaded
        .unreadable
        .iter()
        .map(|(path, why)| format!("{}: {why}", path.file_name().unwrap().display()))
        .collect();
    skipped.sort();
    assert_eq!(
        skipped,
        [
            "g3.json: has 1 seat tokens for 2 seats",
            "g5.json: holds game g1, not g5"
        ]
    );
    assert_eq!(hub.list().len(), 1);
    assert!(!dir.join(".g1.json.tmp").exists());
    assert_eq!(std::fs::read_to_string(dir.join("g1.json")).unwrap(), saved);
}

#[test]
fn a_directory_serves_one_hub_at_a_time() {
    let tmp = tempfile::tempdir().unwrap();
    let (first, _) = Hub::open(tmp.path(), KEEP).unwrap();
    let second = Hub::open(tmp.path(), KEEP).err().map(|e| e.to_string());
    assert_eq!(
        second.as_deref(),
        Some("another dominion server is using this directory")
    );
    drop(first);
    assert!(Hub::open(tmp.path(), KEEP).is_ok());
}

#[cfg(unix)]
#[test]
fn a_directory_that_cannot_be_written_to_is_refused() {
    use std::os::unix::fs::PermissionsExt;
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    {
        let (hub, _) = Hub::open(dir, KEEP).unwrap();
        create(&hub, None);
    }
    let set_mode = |mode| std::fs::set_permissions(dir, PermissionsExt::from_mode(mode)).unwrap();
    set_mode(0o500);
    // Root writes anywhere, so there is nothing to refuse.
    let writable = std::fs::write(dir.join("x"), "").is_ok();
    let opened = Hub::open(dir, KEEP).map(|_| ());
    set_mode(0o700);
    assert!(writable || opened.is_err());
}
