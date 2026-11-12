//! The moves inferred between successive views of real games: they take each board to
//! the next exactly, never name a card neither board holds, and give exactly the moves
//! a person would see in a dozen hand-checked steps.

mod common;

use common::card_db;
use dominion_agent::Table;
use dominion_engine::{Card, GameConfig, PlayerId, Set};
use dominion_protocol::{AgentView, LogEntry};
use dominion_web::cards::CardDb;
use dominion_web::game::board::{Board, PileAt, board};
use dominion_web::game::interaction::interaction;
use dominion_web::game::moves::{Inference, Loc, Move, infer};
use dominion_web::game::state::Selection;
use rand::rngs::StdRng;
use rand::seq::IndexedRandom;
use rand::{Rng, SeedableRng};
use std::collections::{BTreeMap, BTreeSet};

/// How many cards each place holds, read off the board independently of the inference.
fn counts(b: &Board) -> BTreeMap<Loc, i64> {
    let mut out = BTreeMap::new();
    for p in b.you.iter().chain(&b.opponents) {
        let s = p.seat;
        out.insert(Loc::Hand(s), i64::from(p.hand.count));
        out.insert(Loc::Deck(s), i64::from(p.deck.count));
        out.insert(Loc::Discard(s), i64::from(p.discard.count));
        out.insert(Loc::InPlay(s), i64::from(p.in_play.count));
        for z in &p.zones {
            out.insert(Loc::Mat(s, z.key.clone()), i64::from(z.zone.count));
        }
    }
    out.insert(Loc::Trash, i64::from(b.trash.count));
    for p in b
        .supply
        .basic
        .iter()
        .chain(b.supply.kingdom.iter().flatten())
        .chain(&b.non_supply)
    {
        out.insert(Loc::Pile(p.at), i64::from(p.count));
    }
    out
}

/// Every card name a board holds anywhere: in its zones or on top of (or naming) a pile.
fn names(b: &Board) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for p in b.you.iter().chain(&b.opponents) {
        for z in [&p.hand, &p.deck, &p.discard, &p.in_play]
            .into_iter()
            .chain(p.zones.iter().map(|z| &z.zone))
        {
            out.extend(z.known.iter().map(|s| s.card.clone()));
        }
    }
    out.extend(b.trash.known.iter().map(|s| s.card.clone()));
    for p in b
        .supply
        .basic
        .iter()
        .chain(b.supply.kingdom.iter().flatten())
        .chain(&b.non_supply)
    {
        out.insert(p.name.clone());
        out.insert(p.card.clone());
    }
    out
}

/// Checks an inference between two boards; true if it was complete.
fn check(old: &Board, new: &Board, log: &[LogEntry], inf: &Inference) -> bool {
    let mut c = counts(old);
    let held: BTreeSet<String> = names(old).union(&names(new)).cloned().collect();
    for m in &inf.moves {
        let from = c.entry(m.from.clone()).or_default();
        *from -= 1;
        assert!(*from >= 0, "{m:?} takes from an empty place: {log:#?}");
        *c.entry(m.to.clone()).or_default() += 1;
        if let Some(card) = &m.card {
            assert!(held.contains(card), "{m:?} invents {card}");
        }
    }
    if inf.complete {
        let want = counts(new);
        let keys: BTreeSet<&Loc> = c.keys().chain(want.keys()).collect();
        for k in keys {
            assert_eq!(
                c.get(k).copied().unwrap_or(0),
                want.get(k).copied().unwrap_or(0),
                "{k:?} after {:#?}\nlog {log:#?}",
                inf.moves
            );
        }
    } else {
        assert!(inf.moves.is_empty());
    }
    inf.complete
}

/// A throwaway responder: a random legal selection, made by clicking targets.
fn random_answer(view: &AgentView, cards: &CardDb, rng: &mut StdRng) -> dominion_protocol::Answer {
    let d = view.decision.as_ref().unwrap();
    let inter = interaction(view, d, cards);
    let mut sel = Selection::default();
    if inter.min == 1 && inter.max == 1 {
        let going: Vec<_> = inter
            .targets
            .iter()
            .filter(|t| t.action != "end-phase")
            .collect();
        let t = if d.kind == "turn" && !going.is_empty() && rng.random_bool(0.85) {
            going.choose(rng).unwrap()
        } else {
            inter.targets.choose(rng).unwrap()
        };
        sel.click(d, &t.options);
    } else {
        let total = d.options.len() as u32;
        let k = rng.random_range(inter.min..=inter.max.min(total)) as usize;
        let mut tries = 0;
        while sel.picks().len() < k && tries < 10_000 {
            tries += 1;
            let t = inter.targets.choose(rng).unwrap();
            sel.click(d, &t.options);
        }
    }
    sel.answer(d).expect("a legal selection")
}

fn seat_board(table: &Table, seat: u8, cards: &CardDb) -> (Board, usize) {
    let v = table.view_from(PlayerId(seat), 0);
    let len = v.since_last_decision.len();
    let deciding = table.deciding().map(|p| p.0);
    (
        board(&v, deciding, &table.extras(PlayerId(seat)), cards),
        len,
    )
}

fn log_from(table: &Table, seat: u8, from: usize) -> Vec<LogEntry> {
    table.view_from(PlayerId(seat), from).since_last_decision
}

#[derive(Default)]
struct Stats {
    steps: usize,
    complete: usize,
    batches: usize,
    batches_complete: usize,
    moves: usize,
}

/// Plays a game at random, checking every seat's inference at every step, and for seat
/// 0 also over runs of steps (what a slow browser sees arrive at once).
fn play(table: &mut Table, players: u8, rng: &mut StdRng, cards: &CardDb, stats: &mut Stats) {
    let mut batch_start = seat_board(table, 0, cards);
    let mut batch_len = 0;
    for _ in 0..700 {
        let Some(seat) = table.deciding() else { break };
        let before: Vec<(Board, usize)> =
            (0..players).map(|s| seat_board(table, s, cards)).collect();
        let view = table.view(seat);
        let a = random_answer(&view, cards, rng);
        table.answer(seat, &a).unwrap();
        for s in 0..players {
            let (old, len) = &before[usize::from(s)];
            let (new, _) = seat_board(table, s, cards);
            let log = log_from(table, s, *len);
            let inf = infer(old, &new, &log);
            stats.steps += 1;
            stats.moves += inf.moves.len();
            if check(old, &new, &log, &inf) {
                stats.complete += 1;
            } else if std::env::var("MOVES_DEBUG").is_ok() {
                println!("incomplete for seat {s}: {log:?}");
            }
        }
        batch_len += 1;
        if batch_len >= rng.random_range(2..12) {
            let (new, len) = seat_board(table, 0, cards);
            let log = log_from(table, 0, batch_start.1);
            let inf = infer(&batch_start.0, &new, &log);
            stats.batches += 1;
            if check(&batch_start.0, &new, &log, &inf) {
                stats.batches_complete += 1;
            } else if std::env::var("MOVES_DEBUG").is_ok() {
                println!("incomplete run: {log:?}");
            }
            batch_start = (new, len);
            batch_len = 0;
        }
    }
}

#[test]
fn moves_take_each_board_to_the_next() {
    let cards = card_db();
    let mut stats = Stats::default();
    let games = 120;
    for seed in 0..games {
        let players = 2 + (seed % 3) as u8;
        let mut c = GameConfig::random(players, Set::ALL, seed);
        c.turn_limit = Some(16);
        let mut rng = StdRng::seed_from_u64(seed);
        let mut table = Table::new(format!("g{seed}"), c, vec![]).unwrap();
        play(&mut table, players, &mut rng, &cards, &mut stats);
    }
    println!(
        "{games} games: {} of {} single steps and {} of {} runs of steps inferred \
         completely, {} moves",
        stats.complete, stats.steps, stats.batches_complete, stats.batches, stats.moves
    );
    // What remains is mostly setup (Heirlooms replacing Coppers, opening hands), which
    // the log doesn't tell and the table never animates.
    assert!(stats.complete * 1000 >= stats.steps * 995);
    assert!(stats.batches_complete * 100 >= stats.batches * 98);
}

// Hand-checked steps.

fn config(kingdom: &[&str], players: u8, seed: u64) -> GameConfig {
    let kingdom = kingdom
        .iter()
        .map(|n| Card::from_name(n).unwrap_or_else(|| panic!("{n}")))
        .collect();
    let mut c = GameConfig::new(players, kingdom, seed);
    c.turn_limit = Some(40);
    c
}

fn mv(card: Option<&str>, from: Loc, to: Loc) -> Move {
    Move {
        card: card.map(String::from),
        from,
        to,
    }
}

/// A step seen by `seat`: the boards before and after and the log between them.
struct Seen {
    old: Board,
    new: Board,
    log: Vec<LogEntry>,
}

impl Seen {
    fn kinds(&self) -> Vec<(&str, Option<u8>, Option<&str>)> {
        self.log
            .iter()
            .map(|e| (e.kind.as_str(), e.player, e.card.as_deref()))
            .collect()
    }

    fn moves(&self) -> Vec<Move> {
        let inf = infer(&self.old, &self.new, &self.log);
        assert!(inf.complete, "incomplete: {:#?}", self.log);
        inf.moves
    }

    fn pile(&self, card: &str) -> Loc {
        let p = self
            .old
            .supply
            .basic
            .iter()
            .chain(self.old.supply.kingdom.iter().flatten())
            .chain(&self.old.non_supply)
            .find(|p| p.name == card)
            .unwrap();
        Loc::Pile(p.at)
    }

    fn hand(&self, b: &Board) -> Vec<(String, u32)> {
        b.you
            .as_ref()
            .unwrap()
            .hand
            .known
            .iter()
            .map(|s| (s.card.clone(), s.count))
            .collect()
    }
}

/// Plays seeded games of `kingdom` with a random responder (steered by `steer` where
/// it says so) until `want` accepts what `seat` saw over the last one to four steps.
fn find(
    kingdom: &[&str],
    seat: u8,
    steer: impl Fn(&AgentView) -> Option<dominion_protocol::Answer>,
    want: impl Fn(&Seen) -> bool,
) -> Seen {
    let cards = card_db();
    for seed in 0..60 {
        let mut table = Table::new("g", config(kingdom, 2, seed), vec![]).unwrap();
        let mut rng = StdRng::seed_from_u64(seed);
        let mut recent: Vec<(Board, usize)> = vec![];
        for _ in 0..600 {
            let Some(who) = table.deciding() else { break };
            recent.push(seat_board(&table, seat, &cards));
            if recent.len() > 4 {
                recent.remove(0);
            }
            let view = table.view(who);
            let a = steer(&view).unwrap_or_else(|| random_answer(&view, &cards, &mut rng));
            table.answer(who, &a).unwrap();
            let (new, _) = seat_board(&table, seat, &cards);
            for (old, len) in recent.iter().rev() {
                let seen = Seen {
                    old: old.clone(),
                    new: new.clone(),
                    log: log_from(&table, seat, *len),
                };
                if want(&seen) {
                    return seen;
                }
            }
        }
    }
    panic!("no such step");
}

fn no_steer(_: &AgentView) -> Option<dominion_protocol::Answer> {
    None
}

/// Answers with the option doing `action` with `card`, when there is one.
fn choose(view: &AgentView, action: &str, card: Option<&str>) -> Option<dominion_protocol::Answer> {
    let d = view.decision.as_ref()?;
    let o = d
        .options
        .iter()
        .find(|o| o.action == action && (card.is_none() || o.card.as_deref() == card))?;
    Some(dominion_protocol::Answer::indices(
        d.id.clone(),
        vec![o.index],
    ))
}

const BASE: [&str; 10] = [
    "Cellar",
    "Moat",
    "Village",
    "Workshop",
    "Militia",
    "Smithy",
    "Festival",
    "Laboratory",
    "Market",
    "Chapel",
];

#[test]
fn playing_a_treasure_moves_it_from_the_hand_to_play() {
    let s = find(&BASE, 0, no_steer, |s| {
        s.kinds() == [("played", Some(0), Some("Copper"))]
    });
    assert_eq!(
        s.moves(),
        [mv(Some("Copper"), Loc::Hand(0), Loc::InPlay(0))]
    );
}

#[test]
fn a_buy_moves_the_card_from_its_pile_to_the_discard_pile() {
    let s = find(&BASE, 0, no_steer, |s| {
        s.kinds().starts_with(&[
            ("bought", Some(0), Some("Silver")),
            ("gained", Some(0), Some("Silver")),
        ])
    });
    let silver = s.pile("Silver");
    assert_eq!(s.moves()[0], mv(Some("Silver"), silver, Loc::Discard(0)));
}

#[test]
fn an_opponents_buy_is_seen_from_their_pile_to_their_discard_pile() {
    let s = find(&BASE, 0, no_steer, |s| {
        s.kinds().starts_with(&[
            ("bought", Some(1), Some("Silver")),
            ("gained", Some(1), Some("Silver")),
        ])
    });
    let silver = s.pile("Silver");
    let got = s.moves();
    assert_eq!(got[0], mv(Some("Silver"), silver, Loc::Discard(1)));
    // Then their Clean-up: their hand (unseen) and what they played to their discard pile.
    let old = s.old.opponents.iter().find(|o| o.seat == 1).unwrap();
    let to_discard = got[1..].iter().filter(|m| m.to == Loc::Discard(1)).count() as u32;
    assert_eq!(to_discard, old.hand.count + old.in_play.count);
    assert!(
        got[1..]
            .iter()
            .filter(|m| m.from == Loc::Hand(1))
            .all(|m| m.card.is_none())
    );
}

#[test]
fn clean_up_discards_play_and_hand_then_draws_five() {
    let s = find(&BASE, 0, no_steer, |s| {
        let k = s.kinds();
        k.first().is_some_and(|e| e.0 == "drew" && e.1 == Some(0))
            && k.iter().any(|e| e.0 == "turn-start" && e.1 == Some(1))
            && k.len() == 2
            && s.old.you.as_ref().unwrap().in_play.count >= 2
            && s.old.you.as_ref().unwrap().hand.count >= 1
    });
    let old = s.old.you.as_ref().unwrap();
    let mut want: Vec<Move> = vec![];
    for st in &old.in_play.known {
        for _ in 0..st.count {
            want.push(mv(Some(&st.card), Loc::InPlay(0), Loc::Discard(0)));
        }
    }
    for st in &old.hand.known {
        for _ in 0..st.count {
            want.push(mv(Some(&st.card), Loc::Hand(0), Loc::Discard(0)));
        }
    }
    let got = s.moves();
    let n = want.len();
    assert_eq!(got[..n], want[..], "{got:#?}");
    let mut faces: BTreeMap<String, u32> = BTreeMap::new();
    for m in &got[n..] {
        assert_eq!((&m.from, &m.to), (&Loc::Deck(0), &Loc::Hand(0)));
        *faces.entry(m.card.clone().unwrap()).or_default() += 1;
    }
    assert_eq!(got.len() - n, 5);
    let mut hand = s.hand(&s.new);
    hand.sort();
    assert_eq!(faces.into_iter().collect::<Vec<_>>(), hand);
}

#[test]
fn cellar_discards_then_draws_as_many() {
    let s = find(
        &["Cellar"],
        0,
        |v| choose(v, "play", Some("Cellar")),
        |s| {
            let k = s.kinds();
            k.iter().filter(|e| e.0 == "discarded").count() == 2
                && k.contains(&("drew", Some(0), None))
                && !k.iter().any(|e| e.0 == "shuffled")
        },
    );
    let got = s.moves();
    let discarded: Vec<&str> = s
        .log
        .iter()
        .filter(|e| e.kind == "discarded")
        .map(|e| e.card.as_deref().unwrap())
        .collect();
    assert_eq!(
        got[..2],
        [
            mv(Some(discarded[0]), Loc::Hand(0), Loc::Discard(0)),
            mv(Some(discarded[1]), Loc::Hand(0), Loc::Discard(0)),
        ]
    );
    assert_eq!(got.len(), 4);
    assert!(
        got[2..]
            .iter()
            .all(|m| m.from == Loc::Deck(0) && m.to == Loc::Hand(0) && m.card.is_some())
    );
}

#[test]
fn chapel_trashes_from_the_hand() {
    let s = find(
        &["Chapel"],
        0,
        |v| choose(v, "play", Some("Chapel")),
        |s| s.kinds().iter().filter(|e| e.0 == "trashed").count() >= 2,
    );
    let trashed: Vec<Move> = s
        .log
        .iter()
        .filter(|e| e.kind == "trashed")
        .map(|e| mv(e.card.as_deref(), Loc::Hand(0), Loc::Trash))
        .collect();
    assert_eq!(s.moves(), trashed);
}

#[test]
fn a_shuffle_moves_the_discard_pile_into_the_deck_before_the_draw() {
    let s = find(&BASE, 0, no_steer, |s| {
        let k = s.kinds();
        k.contains(&("shuffled", Some(0), None)) && k.iter().any(|e| e.0 == "turn-start")
    });
    let got = s.moves();
    let old = s.old.you.as_ref().unwrap();
    let new = s.new.you.as_ref().unwrap();
    eprintln!(
        "SHUF {:#?}\n{:#?}\nold {:?}\nnew {:?}",
        s.log, got, old, new
    );
    let shuffled = got
        .iter()
        .filter(|m| m.from == Loc::Discard(0) && m.to == Loc::Deck(0))
        .count() as u32;
    // Everything discarded before the shuffle went in: the discard pile, what was gained
    // to it and Clean-up's discards.
    let gained = s
        .log
        .iter()
        .filter(|e| e.kind == "gained" && e.player == Some(0))
        .count() as u32;
    let discarded = old.discard.count + gained + old.hand.count + old.in_play.count;
    assert_eq!(shuffled, discarded - new.discard.count);
    let drawn = got
        .iter()
        .filter(|m| m.from == Loc::Deck(0) && m.to == Loc::Hand(0));
    assert_eq!(drawn.count() as u32, new.hand.count);
    let first_shuffle = got.iter().position(|m| m.to == Loc::Deck(0)).unwrap();
    let last_cleanup = got.iter().rposition(|m| m.to == Loc::Discard(0)).unwrap();
    assert!(last_cleanup < first_shuffle);
}

#[test]
fn harbinger_puts_a_card_from_the_discard_pile_onto_the_deck() {
    let s = find(
        &["Harbinger"],
        0,
        |v| choose(v, "play", Some("Harbinger")),
        |s| {
            let k = s.kinds();
            k.first() == Some(&("played", Some(0), Some("Harbinger")))
                && k.iter().any(|e| e.0 == "top-decked")
                && !k.iter().any(|e| e.0 == "shuffled")
        },
    );
    let got = s.moves();
    eprintln!(
        "HARB {:#?}\n{:#?}\nold deck {:?}\nnew deck {:?}\nold discard {:?}",
        s.log,
        got,
        s.old.you.as_ref().unwrap().deck,
        s.new.you.as_ref().unwrap().deck,
        s.old.you.as_ref().unwrap().discard
    );
    // Harbinger to play, its card drawn, then a card from the discard pile to the deck.
    assert_eq!(got[0], mv(Some("Harbinger"), Loc::Hand(0), Loc::InPlay(0)));
    assert_eq!(
        (got[1].from.clone(), got[1].to.clone()),
        (Loc::Deck(0), Loc::Hand(0))
    );
    let last = got.last().unwrap();
    assert_eq!(
        (last.from.clone(), last.to.clone()),
        (Loc::Discard(0), Loc::Deck(0))
    );
    assert!(last.card.is_some());
}

#[test]
fn mine_trashes_a_treasure_and_gains_one_to_the_hand() {
    let steer = |v: &AgentView| {
        choose(v, "play", Some("Mine"))
            .or_else(|| choose(v, "play", Some("Silver")))
            .or_else(|| choose(v, "play", Some("Copper")))
            .or_else(|| choose(v, "buy", Some("Mine")))
            .or_else(|| choose(v, "buy", Some("Silver")))
    };
    let s = find(&["Mine"], 0, steer, |s| {
        s.kinds()
            == [
                ("played", Some(0), Some("Mine")),
                ("trashed", Some(0), Some("Copper")),
                ("gained", Some(0), Some("Silver")),
            ]
    });
    assert_eq!(
        s.moves(),
        [
            mv(Some("Mine"), Loc::Hand(0), Loc::InPlay(0)),
            mv(Some("Copper"), Loc::Hand(0), Loc::Trash),
            mv(Some("Silver"), s.pile("Silver"), Loc::Hand(0)),
        ]
    );
}

#[test]
fn a_duration_stays_in_play_through_clean_up() {
    let s = find(
        &["Caravan"],
        0,
        |v| choose(v, "play", Some("Caravan")),
        |s| {
            let k = s.kinds();
            k.iter().any(|e| e.0 == "turn-start" && e.1 == Some(1))
                && s.old
                    .you
                    .as_ref()
                    .unwrap()
                    .in_play
                    .known
                    .iter()
                    .any(|st| st.card == "Caravan")
                && !k.iter().any(|e| e.0 == "shuffled")
        },
    );
    let got = s.moves();
    assert!(
        !got.iter().any(|m| m.card.as_deref() == Some("Caravan")),
        "{got:#?}"
    );
    assert!(
        s.new
            .you
            .as_ref()
            .unwrap()
            .in_play
            .known
            .iter()
            .any(|st| st.card == "Caravan")
    );
}

#[test]
fn island_goes_to_its_mat_with_the_card_it_sets_aside() {
    let s = find(
        &["Island"],
        0,
        |v| choose(v, "play", Some("Island")),
        |s| {
            let k = s.kinds();
            k.first() == Some(&("played", Some(0), Some("Island")))
                && k.iter()
                    .filter(|e| e.0 == "moved" && e.1 == Some(0))
                    .count()
                    == 2
        },
    );
    let got = s.moves();
    let mat = Loc::Mat(0, "island".into());
    assert!(
        got.iter().all(|m| m.to == mat || m.to == Loc::InPlay(0)),
        "{got:#?}"
    );
    let island = got
        .iter()
        .find(|m| m.card.as_deref() == Some("Island"))
        .unwrap();
    assert_eq!(
        (island.from.clone(), island.to.clone()),
        (Loc::Hand(0), mat.clone())
    );
    assert_eq!(got.iter().filter(|m| m.to == mat).count(), 2);
}

#[test]
fn a_returned_card_goes_back_to_its_pile() {
    let s = find(
        &["Ambassador"],
        0,
        |v| choose(v, "play", Some("Ambassador")),
        |s| s.kinds().iter().any(|e| e.0 == "moved" && e.1 == Some(0)),
    );
    let back = s
        .log
        .iter()
        .find(|e| e.kind == "moved")
        .unwrap()
        .card
        .clone()
        .unwrap();
    let pile = s.pile(&back);
    let got = s.moves();
    assert!(
        got.contains(&mv(Some(&back), Loc::Hand(0), pile)),
        "{got:#?}"
    );
}

#[test]
fn piles_are_where_the_view_says() {
    // A sanity check of the helpers above.
    let s = find(&BASE, 0, no_steer, |s| !s.log.is_empty());
    assert!(matches!(s.pile("Copper"), Loc::Pile(PileAt::Supply(_))));
}
