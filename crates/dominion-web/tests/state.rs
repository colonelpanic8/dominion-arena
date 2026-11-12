//! The game page's model against real games: the log it accumulates and the answers it
//! builds from selections, checked by the server's own `Table`.

mod common;

use common::card_db;
use dominion_agent::Table;
use dominion_engine::{Card, GameConfig, PlayerId, Response, Set};
use dominion_protocol::api::{Change, SeatView};
use dominion_protocol::lobby::{ChatKind, ChatMessage};
use dominion_protocol::{AgentDecision, AgentOption, Answer, DecisionGroup, LogEntry};
use dominion_web::cards::CardDb;
use dominion_web::game::state::{Applied, GameState, Selection, answers_on_click, name_answer};
use rand::rngs::StdRng;
use rand::seq::{IndexedRandom, SliceRandom};
use rand::{Rng, SeedableRng};
use std::collections::BTreeSet;

fn entries(seqs: std::ops::Range<usize>) -> Vec<LogEntry> {
    seqs.map(|seq| LogEntry {
        seq,
        kind: "drew".into(),
        player: Some(0),
        card: None,
        text: format!("entry {seq}"),
    })
    .collect()
}

fn seqs(log: &[LogEntry]) -> Vec<usize> {
    log.iter().map(|e| e.seq).collect()
}

#[test]
fn merging_skips_what_is_held_and_never_leaves_a_gap() {
    let mut st = GameState::default();
    assert_eq!(st.log_from(), 0);
    assert!(st.merge_log(&entries(0..5)));
    assert!(st.merge_log(&entries(3..8)));
    assert!(st.merge_log(&entries(0..2)));
    assert_eq!(seqs(&st.log), (0..8).collect::<Vec<_>>());
    assert!(!st.merge_log(&entries(10..12)));
    assert_eq!(st.log_from(), 8);
    assert!(!st.merge_log(&[entries(8..9), entries(10..11)].concat()));
    assert_eq!(st.log_from(), 9);
    assert!(st.merge_log(&entries(9..12)));
    assert_eq!(seqs(&st.log), (0..12).collect::<Vec<_>>());
}

fn config(kingdom: &[&str], seed: u64) -> GameConfig {
    let kingdom = kingdom
        .iter()
        .map(|n| Card::from_name(n).unwrap_or_else(|| panic!("{n}")))
        .collect();
    let mut c = GameConfig::new(2, kingdom, seed);
    c.turn_limit = Some(40);
    c
}

/// A random selection with a legal number of picks in every group, made by clicking.
/// Turns mostly go on rather than end, so that games reach more kinds of decisions.
fn random_selection(d: &AgentDecision, rng: &mut StdRng) -> Selection {
    let mut sel = Selection::default();
    let going_on: Vec<u32> = d
        .options
        .iter()
        .filter(|o| o.action != "end-phase")
        .map(|o| o.index)
        .collect();
    if d.kind == "turn" && !going_on.is_empty() && rng.random_bool(0.9) {
        sel.toggle(d, *going_on.choose(rng).unwrap());
        return sel;
    }
    let (min, max) = (d.min.unwrap_or(1) as usize, d.max.unwrap_or(1) as usize);
    let mut room = max;
    let mut wanted = vec![];
    for g in &d.groups {
        let mut opts: Vec<u32> = d
            .options
            .iter()
            .filter(|o| o.group == g.group)
            .map(|o| o.index)
            .collect();
        let hi = (g.max as usize).min(opts.len()).min(room);
        let lo = (g.min as usize).min(hi);
        let n = rng.random_range(lo..=hi);
        opts.shuffle(rng);
        wanted.extend(opts.into_iter().take(n));
        room -= n;
    }
    if wanted.len() < min {
        let mut rest: Vec<u32> = d
            .options
            .iter()
            .map(|o| o.index)
            .filter(|i| !wanted.contains(i))
            .collect();
        rest.shuffle(rng);
        wanted.extend(rest.into_iter().take(min - wanted.len()));
    }
    wanted.shuffle(rng);
    for i in wanted {
        sel.toggle(d, i);
    }
    sel
}

#[derive(Default)]
struct Seen {
    multi: usize,
    ordered: usize,
    number: usize,
    name: usize,
    choose_many: usize,
    turn: usize,
}

/// Plays a game with random legal answers built through `Selection`, checking each
/// against the table, and checking the selection's notion of legality both ways.
fn play_checked(table: &mut Table, rng: &mut StdRng, cards: &CardDb, seen: &mut Seen) {
    for _ in 0..800 {
        let Some(seat) = table.deciding() else { return };
        let view = table.view(seat);
        let d = view
            .decision
            .clone()
            .expect("the deciding seat has a decision");
        match d.kind.as_str() {
            "turn" => seen.turn += 1,
            "number" => seen.number += 1,
            "name" => seen.name += 1,
            "choose" if d.max > Some(1) => seen.choose_many += 1,
            "cards" if d.max > Some(1) => seen.multi += 1,
            _ => {}
        }
        if d.ordered {
            seen.ordered += 1;
        }

        let sel = random_selection(&d, rng);
        let answer = sel
            .answer(&d)
            .unwrap_or_else(|| panic!("illegal random selection {sel:?} for {d:?}"));
        let response = table
            .resolve(seat, &answer)
            .unwrap_or_else(|e| panic!("{e}: {answer:?} for {d:?}"));

        if answers_on_click(&d) {
            let o = d.options.choose(rng).unwrap();
            let one = Selection::new(vec![o.index]).answer(&d).expect("one click");
            assert!(table.resolve(seat, &one).is_ok(), "{one:?} for {d:?}");
        }

        let max = d.max.unwrap_or(1) as usize;
        if d.options.len() > max {
            let mut too_many: Vec<u32> = d.options.iter().map(|o| o.index).collect();
            too_many.truncate(max + 1);
            let sel = Selection::new(too_many.clone());
            assert_eq!(sel.answer(&d), None);
            assert!(
                table
                    .resolve(seat, &Answer::indices(d.id.clone(), too_many))
                    .is_err()
            );
        }
        let min = d.min.unwrap_or(1) as usize;
        if min > 0 {
            let short: Vec<u32> = sel.picks().iter().copied().take(min - 1).collect();
            assert_eq!(Selection::new(short.clone()).answer(&d), None);
            assert!(
                table
                    .resolve(seat, &Answer::indices(d.id.clone(), short))
                    .is_err()
            );
        }

        if d.ordered && sel.picks().len() > 1 {
            let mut reversed = sel.clone();
            let n = reversed.picks().len();
            for i in 0..n {
                reversed.reorder(n - 1, i);
            }
            let rev = table.resolve(seat, &reversed.answer(&d).unwrap()).unwrap();
            match (&response, rev) {
                (Response::Cards(a), Response::Cards(b)) => {
                    assert_eq!(a.iter().rev().copied().collect::<Vec<_>>(), b);
                }
                other => panic!("ordered decisions answer with cards: {other:?}"),
            }
        }

        let answer = if d.name_any && rng.random_bool(0.5) {
            let name = cards.nameable().choose(rng).unwrap().clone();
            let named = name_answer(&d, cards, &name.to_lowercase()).expect("a card's name");
            assert!(table.resolve(seat, &named).is_ok(), "{named:?}");
            assert_eq!(name_answer(&d, cards, "Aqueduct"), None);
            named
        } else {
            answer
        };
        table.answer(seat, &answer).unwrap();
    }
}

#[test]
fn random_legal_selections_are_answers_the_table_accepts() {
    let cards = card_db();
    let mut seen = Seen::default();
    let kingdoms: [&[&str]; 3] = [
        &[
            "Cellar",
            "Sentry",
            "Wishing Well",
            "Chapel",
            "Militia",
            "Moat",
            "Throne Room",
            "Library",
            "Harbinger",
            "Vassal",
        ],
        &[
            "Doctor",
            "Stonemason",
            "Secret Passage",
            "Courtier",
            "Masquerade",
            "Pawn",
            "Steward",
            "Nobles",
            "Mill",
            "Lurker",
        ],
        &[
            "Ratcatcher",
            "Coin of the Realm",
            "Haven",
            "Wharf",
            "Royal Carriage",
            "Amulet",
            "Caravan Guard",
            "Native Village",
            "Lookout",
            "Smugglers",
        ],
    ];
    let mut configs: Vec<GameConfig> = vec![];
    for (i, k) in kingdoms.iter().enumerate() {
        for seed in 0..3 {
            configs.push(config(k, 100 * i as u64 + seed));
        }
    }
    for seed in [7, 21, 1234, 99_999] {
        let mut c = GameConfig::random(2 + (seed % 3) as u8, Set::ALL, seed);
        c.turn_limit = Some(24);
        configs.push(c);
    }
    for (i, c) in configs.into_iter().enumerate() {
        let mut rng = StdRng::seed_from_u64(i as u64);
        let mut table = Table::new(format!("g{i}"), c, vec![]).unwrap();
        play_checked(&mut table, &mut rng, &cards, &mut seen);
    }
    assert!(seen.turn > 0);
    assert!(seen.multi > 0, "no multi-card decision met");
    assert!(seen.ordered > 0, "no ordered decision met");
    assert!(seen.number > 0, "no number decision met");
    assert!(seen.name > 0, "no name decision met");
    assert!(seen.choose_many > 0, "no multi-choice decision met");
}

/// What the server sends `seat`: its view with the log from `log_from` (else from its
/// last answer).
fn seat_view(table: &Table, seat: PlayerId, log_from: Option<usize>, version: u64) -> SeatView {
    SeatView {
        view: match log_from {
            Some(from) => table.view_from(seat, from),
            None => table.view(seat),
        },
        version,
        deciding: table.deciding().map(|p| p.0),
        table: table.extras(seat),
        undos: table.undos(),
        can_undo: table.can_undo(seat),
        meta: Default::default(),
    }
}

/// One seat's client: loads with `logFrom=0`, refetches from its cursor (or from 0 after
/// an undo), and takes the response to each of its answers.
struct Client {
    seat: PlayerId,
    state: GameState,
}

impl Client {
    fn fetch(&mut self, table: &Table, version: u64) -> Applied {
        let mut from = self.state.log_from();
        loop {
            match self
                .state
                .apply(seat_view(table, self.seat, Some(from), version))
            {
                Applied::Restart if from != 0 => from = 0,
                applied => return applied,
            }
        }
    }
}

#[test]
fn the_log_is_the_whole_game_log_across_loads_refetches_and_answers() {
    let mut rng = StdRng::seed_from_u64(5);
    let c = config(
        &[
            "Cellar",
            "Sentry",
            "Militia",
            "Moat",
            "Library",
            "Bureaucrat",
            "Witch",
            "Council Room",
            "Spy",
            "Vassal",
        ],
        3,
    );
    let mut table = Table::new("g", c, vec![]).unwrap();
    let mut version = 1;
    let mut clients: Vec<Client> = (0..2)
        .map(|i| Client {
            seat: PlayerId(i),
            state: GameState::default(),
        })
        .collect();
    for cl in &mut clients {
        assert_eq!(cl.fetch(&table, version), Applied::Done);
    }
    // Someone watching: the public log from its own cursor (or from 0 after an undo),
    // fetched now and then.
    let mut watcher = GameState::default();
    let mut watch = |table: &Table, version: u64| {
        let mut from = watcher.log_from();
        let applied = loop {
            match watcher.apply(SeatView {
                view: table.watch_view(from).unwrap(),
                version,
                deciding: table.deciding().map(|p| p.0),
                table: table.watch_extras().unwrap(),
                undos: table.undos(),
                can_undo: false,
                meta: Default::default(),
            }) {
                Applied::Restart if from != 0 => from = 0,
                applied => break applied,
            }
        };
        assert_eq!(applied, Applied::Done);
        assert!(watcher.watching() && watcher.decision().is_none());
        watcher.log.clone()
    };
    let mut stale: Option<SeatView> = None;
    for step in 0..600 {
        let Some(seat) = table.deciding() else { break };
        let cl = &mut clients[seat.index()];
        let d = cl.state.decision().cloned().unwrap_or_else(|| {
            assert_eq!(cl.fetch(&table, version), Applied::Done);
            cl.state
                .decision()
                .cloned()
                .expect("decision after refetch")
        });
        let answer = random_selection(&d, &mut rng).answer(&d).unwrap();
        if step % 7 == 0 {
            stale = Some(seat_view(&table, seat, Some(0), version));
        }
        table.answer(seat, &answer).unwrap();
        version += 1;
        // The server's response to an answer carries its own cursor.
        let applied = cl.state.apply(seat_view(&table, seat, None, version));
        if applied == Applied::Refetch {
            assert_eq!(cl.fetch(&table, version), Applied::Done);
        }
        assert!(cl.state.can_undo);
        if rng.random_bool(0.2) {
            assert!(table.undo(seat));
            version += 1;
            // The response to an undo has the whole log.
            let applied = cl.state.apply(seat_view(&table, seat, Some(0), version));
            assert_eq!(applied, Applied::Done);
            assert_eq!(cl.state.log, table.view_from(seat, 0).since_last_decision);
            continue;
        }
        if let Some(old) = stale.take().filter(|_| step % 7 == 3) {
            let before = cl.state.clone();
            cl.state.apply(old);
            assert_eq!(cl.state.version, before.version);
            assert_eq!(cl.state.view, before.view);
            assert_eq!(cl.state.log, before.log);
        }
        let other = &mut clients[1 - seat.index()];
        if rng.random_bool(0.4) {
            assert_eq!(other.fetch(&table, version), Applied::Done);
        }
        if rng.random_bool(0.3) {
            watch(&table, version);
        }
    }
    assert_eq!(
        watch(&table, version),
        table.watch_view(0).unwrap().since_last_decision
    );
    for cl in &mut clients {
        cl.fetch(&table, version);
        let whole = table.view_from(cl.seat, 0).since_last_decision;
        assert_eq!(cl.state.log, whole, "seat {}", cl.seat);
        let held: BTreeSet<usize> = cl.state.log.iter().map(|e| e.seq).collect();
        assert_eq!(held.len(), cl.state.log.len());
    }
}

fn said(seq: u64, text: &str) -> ChatMessage {
    ChatMessage {
        seq,
        at: 0,
        from: "Ada".into(),
        kind: ChatKind::Player,
        text: text.into(),
    }
}

#[test]
fn the_chat_is_fetched_when_its_counter_moves_and_holds_each_message_once() {
    let mut st = GameState::default();
    let change = |chat| Change { version: 0, chat };
    // Never fetched: even a counter of zero is news (the server may have restarted).
    assert!(st.chat_is_news(change(0)));
    st.merge_chat(0, vec![]);
    assert!(!st.chat_is_news(change(0)));
    assert!(st.chat_is_news(change(2)));
    assert_eq!(st.chat_from(), 0);
    st.merge_chat(2, vec![said(1, "hello"), said(2, "glhf")]);
    assert_eq!(st.chat_from(), 2);
    assert!(!st.chat_is_news(change(2)));
    // A fetch that overlaps what is held adds only what is new.
    st.merge_chat(3, vec![said(2, "glhf"), said(3, "gg")]);
    let seqs: Vec<u64> = st.chat.iter().map(|m| m.seq).collect();
    assert_eq!(seqs, [1, 2, 3]);
}

#[test]
fn a_response_is_current_only_while_nothing_it_says_is_new() {
    let c = config(&["Village", "Smithy"], 1);
    let table = Table::new("g", c, vec![]).unwrap();
    let seat = PlayerId(0);
    let response = |version, from| SeatView {
        view: table.view_from(seat, from),
        version,
        deciding: table.deciding().map(|p| p.0),
        table: table.extras(seat),
        undos: table.undos(),
        can_undo: table.can_undo(seat),
        meta: Default::default(),
    };
    let mut st = GameState::default();
    assert!(!st.is_current(&response(1, 0)));
    st.apply(response(1, 0));
    assert!(st.is_current(&response(1, st.log_from())));
    assert!(st.is_current(&response(1, 0)));
    assert!(!st.is_current(&response(2, st.log_from())));
    let mut left = response(1, st.log_from());
    left.meta.connected = vec![true, false];
    assert!(!st.is_current(&left));
    st.apply(left.clone());
    assert_eq!(st.meta, left.meta);
    assert!(st.is_current(&left));
    // An older response's meta is as stale as its view.
    let mut newer = response(2, st.log_from());
    newer.meta.watchers = 3;
    st.apply(newer);
    st.apply(left);
    assert_eq!(st.meta.watchers, 3);
    st.error = Some("could not reach the server".into());
    assert!(!st.is_current(&response(2, st.log_from())));
}

fn option(index: u32, group: u32) -> AgentOption {
    AgentOption {
        index,
        group,
        action: "select".into(),
        label: format!("o{index}"),
        card: None,
        zone: None,
        cost: None,
        value: None,
        player: None,
    }
}

#[test]
fn groups_bound_their_own_picks() {
    let group = |group, min, max| DecisionGroup {
        group,
        kind: "cards".into(),
        prompt: String::new(),
        min,
        max,
    };
    let d = AgentDecision {
        id: "d1".into(),
        player: Some(0),
        prompt: String::new(),
        kind: "cards".into(),
        purpose: "other".into(),
        source: None,
        phase: None,
        ordered: false,
        name_any: false,
        min: Some(1),
        max: Some(3),
        groups: vec![group(0, 1, 1), group(1, 0, 2)],
        options: vec![
            option(0, 0),
            option(1, 0),
            option(2, 1),
            option(3, 1),
            option(4, 1),
        ],
    };
    assert!(!answers_on_click(&d));
    let mut sel = Selection::default();
    sel.toggle(&d, 2);
    assert_eq!(sel.answer(&d), None, "group 0 needs its pick");
    sel.toggle(&d, 0);
    sel.toggle(&d, 1);
    assert_eq!(sel.picks(), &[2, 1], "a group of one swaps its pick");
    sel.toggle(&d, 3);
    sel.toggle(&d, 4);
    assert_eq!(sel.picks(), &[2, 1, 3], "a full group of two ignores more");
    assert_eq!(sel.answer(&d), Some(Answer::indices("d1", vec![2, 1, 3])));
    sel.toggle(&d, 1);
    assert_eq!(sel.answer(&d), None);
}
