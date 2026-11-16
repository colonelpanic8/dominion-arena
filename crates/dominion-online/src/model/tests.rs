use super::*;
use crate::wire::Writer;

fn ints(w: &mut Writer, values: &[i32]) {
    w.ints(values);
}
fn bools(w: &mut Writer, values: &[bool]) {
    w.int(values.len() as i32);
    for &value in values {
        w.boolean(value);
    }
}
fn reduction(w: &mut Writer) {
    w.int(2);
    w.int(1);
    w.int(0);
    w.int(1);
    w.int(1);
    w.int(2);
    w.int(3);
    w.int(1);
    w.int(31);
    w.int(4);
    w.int(1);
    w.int(2);
}
fn status(w: &mut Writer) {
    w.int(0);
    w.boolean(true);
    w.int(1);
    ints(w, &[1]);
    w.int(2);
    ints(w, &[-1, 3]);
    ints(w, &[1]);
    ints(w, &[1, 2]);
}
fn reminder(w: &mut Writer) {
    w.int(31);
    w.boolean(true);
    w.int(2);
}
fn event(w: &mut Writer, ordinal: i32) {
    w.int(ordinal);
    match ordinal {
        0 => {
            w.int(2);
            w.int(3);
            ints(w, &[7, -1]);
            ints(w, &[-1, 9]);
            w.int(1);
            w.int(5);
        }
        1 | 2 => {
            w.int(3);
            w.int(9);
        }
        3 => {
            w.int(0);
            w.int(5);
            w.int(7);
            w.int(1);
        }
        4 => {
            w.int(1);
            w.boolean(true);
        }
        5 => {
            w.int(0);
            ints(w, &[1, -1]);
            w.int(2);
            w.int(3);
            w.boolean(false);
            w.boolean(true);
        }
        6 => reduction(w),
        7 => {
            w.int(0);
            w.int(3);
            w.boolean(true);
        }
        8 => {
            w.int(3);
            w.int(1);
            w.int(24);
            w.int(7);
        }
        9 => w.int(3),
        10 | 11 | 14 => {
            w.int(0);
            w.int(31);
        }
        12 => {
            for value in [2, 12, 3, 5, -1] {
                w.int(value);
            }
        }
        13 => {
            w.int(1);
            w.int(1);
            reminder(w);
        }
        15 => {
            w.int(0);
            ints(w, &[-1, 1]);
        }
        16 => status(w),
        17 | 20 => {
            w.int(0);
            w.int(31);
        }
        18 => {
            ints(w, &[0, 1]);
            ints(w, &[0, 3]);
            ints(w, &[2]);
        }
        19 => {
            w.int(3);
            ints(w, &[-1, 1]);
        }
        _ => unreachable!(),
    }
}
fn snapshot() -> Vec<u8> {
    let mut w = Writer::new();
    w.boolean(true);
    w.long(123_456_789_012);
    w.int(1);
    w.int(42); // player
    w.int(1);
    w.int(2);
    w.int(7); // player frequency
    w.int(1);
    reminder(&mut w);
    ints(&mut w, &[31]);
    ints(&mut w, &[2, 31]);
    ints(&mut w, &[2, 31]);
    ints(&mut w, &[1, 1]);
    ints(&mut w, &[3]);
    ints(&mut w, &[0]);
    ints(&mut w, &[0]);
    ints(&mut w, &[0]);
    w.int(1);
    ints(&mut w, &[0, -1]);
    ints(&mut w, &[-1]);
    ints(&mut w, &[0]);
    bools(&mut w, &[true]);
    w.int(1);
    w.int(0);
    w.int(3); // marker
    ints(&mut w, &[6]);
    ints(&mut w, &[0]);
    ints(&mut w, &[3]);
    bools(&mut w, &[true]);
    ints(&mut w, &[4]);
    ints(&mut w, &[0]);
    ints(&mut w, &[0]);
    ints(&mut w, &[2]);
    ints(&mut w, &[-1]);
    ints(&mut w, &[-1]);
    bools(&mut w, &[true]);
    for value in [0, 5, 0, 0] {
        w.int(value);
    }
    reduction(&mut w);
    w.int(1);
    w.int(0);
    w.int(31); // rename
    w.int(1);
    w.int(0);
    w.int(2); // behaviour
    w.int(1);
    w.int(0);
    w.int(31); // image
    w.int(1);
    w.int(0);
    ints(&mut w, &[0, 3]); // type change
    w.int(1);
    w.int(0);
    ints(&mut w, &[1, -1]); // associations
    w.int(1);
    status(&mut w);
    w.int(1);
    w.int(2);
    w.int(1); // autoplay
    w.boolean(true);
    w.int(1);
    w.int(0);
    w.long(30_000);
    w.boolean(false);
    w.int(2);
    w.int(2);
    event(&mut w, 1);
    event(&mut w, 20);
    w.finish()
}
#[test]
fn complete_snapshot_consumes_outer_options_and_event_suffix() {
    let bytes = snapshot();
    let mut r = Reader::new(&bytes);
    let snapshot = Snapshot::parse(&mut r).unwrap();
    r.finish().unwrap();
    assert!(snapshot.reconnect);
    assert_eq!(snapshot.game_id, 123_456_789_012);
    assert_eq!(snapshot.state.players[0].player_id, 42);
    assert_eq!(snapshot.state.players[0].card_frequencies[0].frequency, 7);
    assert_eq!(
        snapshot.state.cards[1],
        Card {
            id: 1,
            card_name: 31,
            pile_name: 31
        }
    );
    assert_eq!(snapshot.state.zones[0].cards, [0, -1]);
    assert!(snapshot.state.tokens[0].is_flipped);
    assert_eq!(snapshot.state.counters[0].value, 2);
    assert_eq!(snapshot.state.turn.turn_number, 5);
    assert_eq!(
        snapshot.state.cost_reduction.cost_override[0].1,
        Cost {
            coins: 4,
            potions: 1,
            debt: 2
        }
    );
    assert_eq!(
        snapshot.state.temporary_effects.card_type_changes[0].added,
        [0, 3]
    );
    assert_eq!(snapshot.state.associations[0].linked_cards, [1, -1]);
    assert_eq!(
        snapshot.state.card_statuses[0].set_aside_card_lists,
        [vec![-1, 3], vec![1]]
    );
    assert_eq!(snapshot.autoplay_settings[0].value, 1);
    assert_eq!(
        snapshot.table_options.timer_settings.settings,
        [(0, 30_000)]
    );
    assert_eq!(snapshot.events.len(), 2);
    let json = serde_json::to_string(&snapshot).unwrap();
    assert_eq!(serde_json::from_str::<Snapshot>(&json).unwrap(), snapshot);
}
#[test]
fn all_twenty_one_event_payloads_consume_exactly() {
    for ordinal in 0..21 {
        let mut fixture = Writer::new();
        event(&mut fixture, ordinal);
        let bytes = fixture.finish();
        let mut reader = Reader::new(&bytes);
        Event::parse(&mut reader).unwrap();
        reader.finish().unwrap();
        for length in 0..bytes.len() {
            assert!(
                Event::parse(&mut Reader::new(&bytes[..length])).is_err(),
                "event {ordinal} accepted {length} byte prefix"
            );
        }
    }
    let mut w = Writer::new();
    for i in 0..21 {
        event(&mut w, i);
    }
    w.int(12345);
    let bytes = w.finish();
    let mut r = Reader::new(&bytes);
    for i in 0..21 {
        let parsed = Event::parse(&mut r).unwrap();
        if i == 0 {
            assert!(
                matches!(&parsed,Event::CardMove(CardMove{before,after,..}) if before==&[7,-1]&&after==&[-1,9])
            );
        }
        if i == 16 {
            assert!(
                matches!(&parsed,Event::CardStatus(CardStatus{properties,..}) if properties==&[1,2])
            );
        }
    }
    assert_eq!(r.int().unwrap(), 12345);
    r.finish().unwrap();
}
#[test]
fn rejects_truncation_and_unknown_discriminants() {
    let bytes = snapshot();
    for len in 0..bytes.len() {
        assert!(
            Snapshot::parse(&mut Reader::new(&bytes[..len])).is_err(),
            "accepted {len} byte prefix"
        );
    }
    for ordinal in [-1, 21, i32::MAX] {
        assert!(Event::parse(&mut Reader::new(&ordinal.to_be_bytes())).is_err());
    }
    let mut w = Writer::new();
    w.int(0);
    w.int(1);
    w.int(2);
    ints(&mut w, &[0]);
    ints(&mut w, &[0, 1]);
    w.int(0);
    w.int(0);
    assert!(Event::parse(&mut Reader::new(&w.finish())).is_err());
    let mut w = Writer::new();
    w.int(3);
    for value in [0, 1, 10, 0] {
        w.int(value);
    }
    assert!(Event::parse(&mut Reader::new(&w.finish())).is_err());
    let mut w = Writer::new();
    w.int(17);
    w.int(0);
    w.int(881);
    assert!(Event::parse(&mut Reader::new(&w.finish())).is_err());
    let mut w = Writer::new();
    w.int(8);
    w.int(0);
    w.int(0);
    w.int(35);
    w.int(0);
    assert!(Event::parse(&mut Reader::new(&w.finish())).is_err());
}
#[test]
fn log_ticker_and_result_preserve_typed_payloads() {
    let mut w = Writer::new();
    w.int(5);
    w.int(2); // log start and count
    w.int(0);
    w.int(0);
    w.int(2);
    w.int(0); // named, no arguments
    w.int(1);
    w.int(7);
    w.int(0);
    ints(&mut w, &[2, 1]);
    w.boolean(true);
    w.int(1);
    w.int(8);
    w.int(1);
    ints(&mut w, &[0]);
    w.boolean(false);
    w.boolean(true); // ticker decision + start
    w.long(9);
    w.long(10);
    w.int(-1);
    ints(&mut w, &[2]);
    w.int(1); // result
    w.int(42);
    w.int(1);
    w.int(25);
    w.int(13);
    w.int(1); // player+score
    w.int(2);
    w.int(7);
    w.int(7);
    w.int(0);
    w.int(0);
    w.int(0);
    w.int(0); // score part+named explanation
    w.int(1);
    w.int(2);
    w.int(7);
    w.int(-1);
    w.int(-1);
    w.boolean(false);
    let bytes = w.finish();
    let mut r = Reader::new(&bytes);
    let log = LogChange::parse(&mut r).unwrap();
    assert_eq!(log.start_index, 5);
    assert_eq!(log.entries.len(), 2);
    assert!(Ticker::parse(&mut r).unwrap().is_start_game);
    let result = GameResult::parse(&mut r).unwrap();
    r.finish().unwrap();
    assert_eq!(result.players[0].score.parts[0].points, 7);
}

#[test]
fn rejects_mismatched_snapshot_parallel_arrays() {
    let mut fixture = Writer::new();
    fixture.boolean(false);
    fixture.long(1);
    fixture.int(0);
    fixture.ints(&[2]);
    fixture.ints(&[]);
    fixture.ints(&[1]);
    let error = Snapshot::parse(&mut Reader::new(&fixture.finish())).unwrap_err();
    assert!(error.to_string().contains("parallel arrays"));
}
