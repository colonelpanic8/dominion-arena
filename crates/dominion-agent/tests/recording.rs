//! A table's recorded stream: saved with the table, and written in either encoding.

use dominion_agent::recording::{self, Encoding};
use dominion_agent::{Answer, Table};
use dominion_engine::{GameConfig, PlayerId, Set};
use dominion_protocol::events::Event;

/// Answers the deciding seat's first option, `n` times or until the game ends.
fn advance(table: &mut Table, n: usize) {
    for _ in 0..n {
        let Some(seat) = table.deciding() else {
            return;
        };
        let d = table.view(seat).decision.expect("a decision");
        let answer = Answer::indices(&d.id, vec![d.options[0].index]);
        table.answer(seat, &answer).unwrap();
    }
}

fn table(seed: u64) -> Table {
    let sets: Vec<Set> = (Set::ALL.iter().copied())
        .filter(|&s| s != Set::Arcana)
        .collect();
    let mut config = GameConfig::random(2 + (seed % 3) as u8, &sets, seed);
    config.turn_limit = Some(12);
    Table::new_recorded(format!("g{seed}"), config, vec![]).unwrap()
}

#[test]
fn a_saved_table_records_on_where_it_left_off() {
    for seed in 0..8 {
        let mut kept = table(seed);
        advance(&mut kept, 40);
        let mut saved: Table =
            serde_json::from_str(&serde_json::to_string(&kept).unwrap()).unwrap();
        assert_eq!(saved.unwritten(), kept.unwritten(), "seed {seed}");
        kept.mark_written();
        saved.mark_written();
        advance(&mut kept, 10_000);
        advance(&mut saved, 10_000);
        assert!(kept.is_over());
        assert!(saved.unwritten() == kept.unwritten(), "seed {seed}");
        assert_eq!(saved.recorded(), kept.recorded());
    }
}

#[test]
fn both_encodings_read_back_the_same_events_and_ignore_a_record_cut_short() {
    for seed in 0..6 {
        let mut table = table(seed);
        advance(&mut table, 10_000);
        let events: Vec<Event> = table.unwritten().to_vec();
        for encoding in Encoding::ALL {
            let mut bytes = vec![];
            encoding
                .write(&mut bytes, &recording::header(table.id()))
                .unwrap();
            for event in &events {
                encoding.write(&mut bytes, event).unwrap();
            }
            let read = recording::read(encoding, &bytes, None).unwrap();
            assert_eq!(read.header, recording::header(table.id()));
            assert!(read.events == events, "seed {seed}, {encoding}");
            assert_eq!(read.len_of(events.len()), Some(bytes.len()));

            let half = read.len_of(5).unwrap();
            let torn = &bytes[..half + (read.len_of(6).unwrap() - half) / 2];
            let read = recording::read(encoding, torn, None).unwrap();
            assert_eq!(read.events.len(), 5, "seed {seed}, {encoding}");
            assert_eq!(read.len_of(5), Some(half));
            assert_eq!(read.len_of(6), None);
        }
    }
}

/// A recorder that fails stops recording for good, and the answer it was recording
/// stands whole: the game goes on exactly as one that never failed.
#[test]
fn a_recorder_fault_never_refuses_or_splits_an_answer() {
    use dominion_agent::WayChoice;
    use dominion_engine::Card::*;
    use rand::seq::SliceRandom;
    use rand::{Rng, SeedableRng};
    let mut stopped = 0;
    for seed in 0..12 {
        let kingdom = vec![
            Village, Smithy, Militia, Moat, ThroneRoom, Market, Laboratory, Festival, Mine,
            Bureaucrat,
        ];
        let mut config = GameConfig::new(2, kingdom, seed);
        config.landscapes = vec![WayOfTheOx, WayOfTheSheep];
        config.turn_limit = Some(10);
        let mut kept = Table::new_recorded("g", config.clone(), vec![]).unwrap();
        let mut faulty = Table::new_recorded("g", config, vec![]).unwrap();
        let mut rng = rand_chacha::ChaCha8Rng::seed_from_u64(seed);
        // Whichever answer comes up after this many, of whatever kind, records wrong.
        let at = rng.random_range(0..40);
        let mut answers = 0;
        while !kept.is_over() {
            if answers == at {
                faulty.inject_recording_fault(rng.random_range(0..3));
            }
            answers += 1;
            let seat = kept.deciding().unwrap();
            let d = kept.view(seat).decision.unwrap();
            if seed % 4 == 3 && answers == 40 {
                kept.resign(seat).unwrap();
                faulty.resign(seat).unwrap();
                continue;
            }
            if d.kind == "turn" && d.phase.as_deref() == Some("buy") && rng.random_bool(0.5) {
                let played = kept.play_treasures(seat, &d.id);
                assert_eq!(faulty.play_treasures(seat, &d.id), played);
                if played.is_ok() {
                    continue;
                }
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
            let way = [
                Some(WayChoice::Way(WayOfTheOx)),
                Some(WayChoice::None),
                None,
            ][rng.random_range(0..3)];
            let answered = kept.answer_with(seat, &answer, way);
            assert_eq!(faulty.answer_with(seat, &answer, way), answered);
            answered.unwrap();
        }
        assert!(kept.records());
        assert!(
            faulty.recording_stopped().is_some() || at >= answers,
            "seed {seed}"
        );
        assert!(faulty.records() == faulty.game().journaled());
        stopped += usize::from(!faulty.records());
        assert_eq!(faulty.game().log(), kept.game().log(), "seed {seed}");
        assert_eq!(faulty.answers(), kept.answers(), "seed {seed}");
        assert_eq!(faulty.view(PlayerId(0)), kept.view(PlayerId(0)));
        // Saved and loaded, it stays stopped.
        let saved: Table = serde_json::from_str(&serde_json::to_string(&faulty).unwrap()).unwrap();
        assert_eq!(saved.recording_stopped(), faulty.recording_stopped());
        // And it can still go back, replaying without a journal.
        if faulty.can_undo(PlayerId(0)) {
            let mut back = faulty.clone();
            let mut kept_back = kept.clone();
            back.undo(PlayerId(0)).unwrap();
            kept_back.undo(PlayerId(0)).unwrap();
            assert_eq!(back.game().log(), kept_back.game().log(), "seed {seed}");
            assert_eq!(back.recording_stopped(), faulty.recording_stopped());
        }
    }
    assert!(stopped >= 8, "{stopped}");
}

/// A table whose saved recording isn't this build's version, or can't be read, loads
/// whole and plays on as it was, with its recording stopped.
#[test]
fn a_recording_from_another_version_stops_and_the_game_plays_on() {
    let mut kept = table(3);
    advance(&mut kept, 30);
    let json: serde_json::Value = serde_json::to_value(&kept).unwrap();
    assert_eq!(
        json["recorder"]["version"],
        dominion_agent::RECORDING_VERSION
    );
    let older = |f: &dyn Fn(&mut serde_json::Value)| {
        let mut json = json.clone();
        f(&mut json);
        serde_json::from_value::<Table>(json).unwrap()
    };
    let cases = [
        older(&|j| j["recorder"]["version"] = 0.into()),
        older(&|j| j["recorder"] = j["recorder"]["state"].clone()),
        older(&|j| j["recorder"]["state"]["ids"] = "?".into()),
        older(&|j| j["game"]["journal"] = "?".into()),
        older(&|j| j["unwritten"] = "?".into()),
    ];
    for mut table in cases {
        assert!(table.recording_stopped().is_some());
        assert!(!table.game().journaled() && table.unwritten().is_empty());
        assert_eq!(table.game().log(), kept.game().log());
        assert_eq!(table.view(PlayerId(1)), kept.view(PlayerId(1)));
        let mut plain = kept.clone();
        advance(&mut table, 10_000);
        advance(&mut plain, 10_000);
        assert_eq!(table.game().log(), plain.game().log());
        assert!(table.recording_stopped().is_some());
    }
}
