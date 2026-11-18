//! A table's recorded stream: saved with the table, and written in either encoding.

use dominion_agent::recording::{self, Encoding};
use dominion_agent::{Answer, Table};
use dominion_engine::{GameConfig, Set};
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
