//! The table's models against real games: where each option of every decision is
//! answered, what the board shows of each view, and the log's lines.

mod common;

use common::card_db;
use dominion_agent::Table;
use dominion_engine::{Card, GameConfig, PlayerId, Set};
use dominion_protocol::{AgentDecision, AgentView, PlayerView, Tokens};
use dominion_web::cards::CardDb;
use dominion_web::game::board::{Board, PileAt, PileEntry, PlayerEntry, Zone, board};
use dominion_web::game::interaction::{Interaction, Place, Target, TreasureRun, interaction};
use dominion_web::game::logfmt::{Span, log_lines};
use dominion_web::game::state::Selection;
use dominion_web::game::table::model::{Spot, spots};
use dominion_web::game::{entry, wording};
use rand::rngs::StdRng;
use rand::seq::IndexedRandom;
use rand::{Rng, SeedableRng};
use std::collections::{BTreeMap, BTreeSet};

fn option_card(d: &AgentDecision, i: u32) -> &dominion_protocol::AgentOption {
    d.options.iter().find(|o| o.index == i).unwrap()
}

fn check_placement(view: &AgentView, d: &AgentDecision, inter: &Interaction) {
    let mut seen = BTreeSet::new();
    for t in &inter.targets {
        assert!(!t.options.is_empty());
        for &i in &t.options {
            assert!(seen.insert(i), "option {i} in two targets: {inter:?}");
            let o = option_card(d, i);
            assert_eq!(
                (&o.card, &o.zone, &o.action),
                (&t.card, &t.zone, &t.action),
                "{t:?}"
            );
        }
        let card = t.card.as_deref();
        let n = t.options.len() as u32;
        let you = view.you.as_ref();
        match t.place {
            Place::Hand | Place::InPlay => {
                let z = you.map(|y| {
                    if t.place == Place::Hand {
                        &y.hand
                    } else {
                        &y.in_play
                    }
                });
                let held = z
                    .and_then(|z| z.known.get(card.unwrap()))
                    .copied()
                    .unwrap_or(0);
                assert!(held >= n, "{t:?} but {held} held");
            }
            Place::Supply => {
                let p = t.pile.unwrap().pile(view).unwrap();
                assert_eq!(p.top.as_deref().unwrap_or(&p.card), card.unwrap(), "{t:?}");
            }
            Place::Landscape => {
                assert!(
                    view.landscapes
                        .iter()
                        .any(|l| Some(l.card.as_str()) == card)
                );
            }
            Place::Tray => assert!(t.card.is_some() || t.zone.is_some(), "{t:?}"),
            Place::Button => assert!(t.card.is_none() && t.options.len() == 1, "{t:?}"),
            Place::Ability => assert!(t.card.is_some() && t.action == "trigger", "{t:?}"),
        }
    }
    let all: BTreeSet<u32> = d.options.iter().map(|o| o.index).collect();
    assert_eq!(seen, all);
}

/// A selection built the way a player builds one: clicking targets (and now and then
/// unclicking one) until it has a legal number of picks.
fn click_until_legal(d: &AgentDecision, inter: &Interaction, rng: &mut StdRng) -> Selection {
    let mut sel = Selection::default();
    if inter.min == 1 && inter.max == 1 {
        let going_on: Vec<&Target> = inter
            .targets
            .iter()
            .filter(|t| t.action != "end-phase")
            .collect();
        let t = if d.kind == "turn" && !going_on.is_empty() && rng.random_bool(0.9) {
            going_on.choose(rng).unwrap()
        } else {
            inter.targets.choose(rng).unwrap()
        };
        sel.click(d, &t.options);
        return sel;
    }
    let total = d.options.len() as u32;
    let k = rng.random_range(inter.min..=inter.max.min(total)) as usize;
    let mut steps = 0;
    while sel.picks().len() < k {
        steps += 1;
        assert!(steps < 10_000, "clicking never reached {k}: {inter:?}");
        let t = inter.targets.choose(rng).unwrap();
        let before = sel.count(&t.options);
        let clicked = sel.click(d, &t.options);
        assert_eq!(sel.count(&t.options), before + usize::from(clicked));
        if clicked && rng.random_bool(0.2) {
            assert!(sel.unclick(&t.options));
            assert_eq!(sel.count(&t.options), before);
        }
    }
    let counted: usize = inter.targets.iter().map(|t| sel.count(&t.options)).sum();
    assert_eq!(counted, sel.picks().len());
    let mut positions: Vec<usize> = inter
        .targets
        .iter()
        .flat_map(|t| sel.positions(&t.options))
        .collect();
    positions.sort();
    assert_eq!(positions, (0..sel.picks().len()).collect::<Vec<_>>());
    sel
}

/// Every target has one spot on the table, no two share one, and each is drawn where
/// its place says.
fn check_spots(b: &Board, inter: &Interaction) {
    let spots = spots(b, inter);
    assert_eq!(spots.len(), inter.targets.len());
    let mut seen = BTreeSet::new();
    for (t, s) in inter.targets.iter().zip(&spots) {
        if !matches!(s, Spot::Tray | Spot::Button | Spot::Ability) {
            assert!(seen.insert(format!("{s:?}")), "{s:?} twice: {inter:?}");
        }
        let card = t.card.clone().unwrap_or_default();
        match s {
            Spot::Hand(c) => assert!(t.place == Place::Hand && *c == card),
            Spot::InPlay(c) => assert!(t.place == Place::InPlay && *c == card),
            Spot::Pile(at) => assert_eq!(Some(*at), t.pile),
            Spot::Landscape(c) => assert!(t.place == Place::Landscape && *c == card),
            Spot::Button => assert_eq!(t.place, Place::Button),
            Spot::Ability => assert_eq!(t.place, Place::Ability),
            Spot::Tray => assert!(!matches!(t.place, Place::Button | Place::Ability)),
        }
    }
}

fn zone_sum(z: &Zone) -> u32 {
    z.known.iter().map(|s| s.count).sum()
}

fn same_zone(z: &Zone, count: u32, known: &BTreeMap<String, u32>, unknown: u32) {
    assert_eq!(z.count, count);
    assert_eq!(z.unknown, unknown);
    let mine: BTreeMap<String, u32> = z.known.iter().map(|s| (s.card.clone(), s.count)).collect();
    let theirs: BTreeMap<String, u32> = known
        .iter()
        .filter(|(_, n)| **n > 0)
        .map(|(c, n)| (c.clone(), *n))
        .collect();
    assert_eq!(mine, theirs);
}

fn token_count(t: &Tokens) -> usize {
    [
        t.coffers != 0,
        t.villagers != 0,
        t.favors != 0,
        t.debt != 0,
        t.vp_tokens != 0,
        t.pirate_ship != 0,
        t.minus_card,
        t.minus_coin,
        t.journey_face_up.is_some(),
    ]
    .iter()
    .filter(|&&b| b)
    .count()
}

fn check_player(e: &PlayerEntry, p: &PlayerView) {
    assert_eq!(
        (e.seat, &e.name, e.vp, e.turns),
        (p.index, &p.name, p.vp, p.turns_taken)
    );
    same_zone(&e.hand, p.hand.count, &p.hand.known, p.hand.unknown);
    assert_eq!(zone_sum(&e.hand) + e.hand.unknown, p.hand.count);
    same_zone(
        &e.in_play,
        p.in_play.count,
        &p.in_play.known,
        p.in_play.unknown,
    );
    same_zone(&e.deck, p.deck.count, &p.deck.known, p.deck.unknown);
    assert_eq!(e.deck_order_known, p.deck.order_known);
    same_zone(
        &e.discard,
        p.discard.count,
        &p.discard.known,
        p.discard.unknown,
    );
    assert_eq!(e.zones.len(), p.other_zones.len());
    for z in &e.zones {
        let v = &p.other_zones[&z.key];
        same_zone(&z.zone, v.count, &v.known, v.unknown);
    }
    assert_eq!(
        e.badges.len(),
        token_count(&p.tokens) + p.adventure_tokens.len()
    );
    assert_eq!(
        (&e.projects, &e.states, &e.boons, &e.artifacts),
        (&p.projects, &p.states, &p.boons, &p.artifacts)
    );
    assert_eq!(zone_sum(&e.owned), p.owned.known.values().sum::<u32>());
    assert_eq!(e.owned.unknown, p.owned.unknown);
    assert_eq!(
        e.unlocated.iter().map(|s| s.count).sum::<u32>(),
        p.unlocated.values().sum::<u32>()
    );
    assert_eq!(e.uncertain.len(), p.uncertain.len());
    assert_eq!(e.counters.len(), p.counters.len());
}

fn check_board(b: &Board, view: &AgentView, deciding: Option<u8>) {
    let shown: Vec<&PileEntry> = b
        .supply
        .basic
        .iter()
        .chain(b.supply.kingdom.iter().flatten())
        .chain(&b.non_supply)
        .collect();
    let mut at: Vec<PileAt> = shown.iter().map(|e| e.at).collect();
    at.sort();
    assert_eq!(at, PileAt::all(view).map(|(a, _)| a).collect::<Vec<_>>());
    assert!(
        b.supply
            .kingdom
            .iter()
            .all(|r| !r.is_empty() && r.len() <= 5)
    );
    assert!(b.supply.kingdom.iter().flatten().all(|p| p.kingdom));
    assert!(b.supply.basic.iter().all(|p| !p.kingdom));
    assert!(
        b.non_supply
            .iter()
            .all(|e| matches!(e.at, PileAt::NonSupply(_)))
    );
    for e in shown {
        let p = e.at.pile(view).unwrap();
        assert_eq!((&e.name, e.count), (&p.card, p.count));
        assert_eq!(e.empty, p.count == 0);
        let t = &p.tokens;
        let tokens = usize::from(t.embargo > 0)
            + usize::from(t.trade_route)
            + usize::from(t.vp != 0)
            + usize::from(t.debt != 0)
            + t.players.len()
            + p.traits.len();
        let notes = view
            .notes
            .iter()
            .filter(|n| n.player.is_none() && n.pile.as_ref() != Some(&n.source))
            .filter(|n| match (&n.pile, &n.card) {
                (Some(pile), None) => *pile == e.name,
                (None, Some(_)) => n.source == e.name,
                _ => false,
            })
            .count();
        assert_eq!(e.badges.len(), tokens + notes);
    }
    let lands: Vec<&str> = b.landscapes.iter().map(|l| l.card.as_str()).collect();
    let want: Vec<&str> = view.landscapes.iter().map(|l| l.card.as_str()).collect();
    assert_eq!(lands, want);

    assert_eq!(b.you.is_some(), view.you.is_some());
    if let (Some(e), Some(p)) = (&b.you, &view.you) {
        check_player(e, p);
    }
    assert_eq!(b.opponents.len(), view.opponents.len());
    for (e, p) in b.opponents.iter().zip(&view.opponents) {
        check_player(e, p);
    }
    assert_eq!(
        b.play_area.player.as_ref().map(|w| w.seat),
        view.turn.player
    );
    if let Some(p) = view
        .you
        .iter()
        .chain(&view.opponents)
        .find(|p| Some(p.index) == view.turn.player)
    {
        same_zone(
            &b.play_area.cards,
            p.in_play.count,
            &p.in_play.known,
            p.in_play.unknown,
        );
    }
    same_zone(
        &b.trash,
        view.trash.count,
        &view.trash.known,
        view.trash.unknown,
    );
    assert_eq!(b.boons.is_some(), view.boons.is_some());
    assert_eq!(b.hexes.is_some(), view.hexes.is_some());
    assert_eq!(b.artifacts.len(), view.artifacts.len());
    assert_eq!(b.lasting.len(), view.lasting.len());
    assert_eq!(b.extra_turns.len(), view.extra_turns.len());
    assert_eq!(b.notes.len(), view.notes.len());
    assert_eq!(b.counters.len(), view.counters.len());
    assert_eq!(b.uncertain.len(), view.uncertain.len());
    let s = &b.status;
    assert_eq!(s.deciding.as_ref().map(|w| w.seat), deciding);
    assert_eq!(s.player.as_ref().map(|w| w.seat), view.turn.player);
    assert_eq!(
        (
            s.phase.as_ref(),
            s.actions,
            s.buys,
            s.coins,
            s.potions,
            s.debt,
            s.turn
        ),
        (
            view.turn.phase.as_ref(),
            view.turn.actions,
            view.turn.buys,
            view.turn.coins,
            view.turn.potions,
            view.turn.debt,
            view.turn.number
        )
    );
}

#[derive(Default)]
struct Stats {
    decisions: BTreeMap<String, usize>,
    /// Decisions of a kind with at least one option in a place.
    placed: BTreeMap<(String, Place), usize>,
    /// Decisions of a kind with nothing on the board: only the tray and buttons.
    off_board: BTreeMap<String, usize>,
    treasure_runs: usize,
    treasure_plays: usize,
}

impl Stats {
    fn record(&mut self, d: &AgentDecision, inter: &Interaction) {
        *self.decisions.entry(d.kind.clone()).or_default() += 1;
        let places: BTreeSet<Place> = inter.targets.iter().map(|t| t.place).collect();
        if places
            .iter()
            .all(|p| matches!(p, Place::Tray | Place::Button | Place::Ability))
        {
            *self.off_board.entry(d.kind.clone()).or_default() += 1;
        }
        for p in places {
            *self.placed.entry((d.kind.clone(), p)).or_default() += 1;
        }
    }

    fn table(&self) -> String {
        let places = [
            Place::Hand,
            Place::InPlay,
            Place::Supply,
            Place::Landscape,
            Place::Tray,
            Place::Button,
            Place::Ability,
        ];
        let mut s = String::from("| kind | decisions |");
        for p in places {
            s.push_str(&format!(" {p:?} |"));
        }
        s.push_str(" tray/buttons only |\n|---|---:|");
        s.push_str(&"---:|".repeat(places.len() + 1));
        for (kind, n) in &self.decisions {
            s.push_str(&format!("\n| {kind} | {n} |"));
            for p in places {
                let k = self.placed.get(&(kind.clone(), p)).copied().unwrap_or(0);
                s.push_str(&format!(" {k} |"));
            }
            let off = self.off_board.get(kind).copied().unwrap_or(0);
            s.push_str(&format!(" {off} |"));
        }
        s
    }
}

/// Plays a game with answers made by clicking targets, checking every decision's
/// placement and every view's board.
fn play_checked(table: &mut Table, rng: &mut StdRng, cards: &CardDb, stats: &mut Stats) {
    for _ in 0..400 {
        let Some(seat) = table.deciding() else { return };
        let view = table.view(seat);
        let d = view.decision.clone().expect("the deciding seat decides");
        let inter = interaction(&view, &d, cards);
        check_placement(&view, &d, &inter);
        stats.record(&d, &inter);
        let b = board(&view, Some(seat.0), &table.extras(seat), cards);
        check_board(&b, &view, Some(seat.0));
        check_spots(&b, &inter);
        if rng.random_bool(0.25) {
            let other = PlayerId(((seat.index() + 1) % table.names().len()) as u8);
            let theirs = table.view(other);
            check_board(
                &board(&theirs, Some(seat.0), &table.extras(other), cards),
                &theirs,
                Some(seat.0),
            );
        }

        if let Some(run) = TreasureRun::start(&view, cards)
            && rng.random_bool(0.5)
        {
            stats.treasure_runs += 1;
            let mut v = view;
            while let Some(a) = run.next(&v, cards) {
                table.resolve(seat, &a).unwrap();
                table.answer(seat, &a).unwrap();
                stats.treasure_plays += 1;
                v = table.view(seat);
            }
            continue;
        }

        let mut sel = click_until_legal(&d, &inter, rng);
        let answer = sel
            .answer(&d)
            .unwrap_or_else(|| panic!("clicked an illegal selection {sel:?} for {d:?}"));
        table
            .resolve(seat, &answer)
            .unwrap_or_else(|e| panic!("{e}: {answer:?} for {d:?}"));
        if d.ordered && sel.picks().len() > 1 {
            let n = sel.picks().len();
            sel.reorder(rng.random_range(0..n), rng.random_range(0..n));
        }
        table.answer(seat, &sel.answer(&d).unwrap()).unwrap();
    }
}

#[test]
fn every_option_has_one_place_and_every_view_a_whole_board() {
    let cards = card_db();
    let mut stats = Stats::default();
    let games = 300;
    for seed in 0..games {
        let mut c = GameConfig::random(2 + (seed % 3) as u8, Set::ALL, seed);
        c.turn_limit = Some(14);
        let mut rng = StdRng::seed_from_u64(seed);
        let mut table = Table::new(format!("g{seed}"), c, vec![]).unwrap();
        play_checked(&mut table, &mut rng, &cards, &mut stats);
        let names = table.names().to_vec();
        let log = table.view_from(PlayerId(0), 0).since_last_decision;
        let lines = log_lines(&log, &names, &cards);
        assert_eq!(
            lines.iter().map(|l| l.count as usize).sum::<usize>(),
            log.len()
        );
    }
    println!(
        "{games} games; options per place (decisions with an option there):\n{}\n\
         treasure runs: {}, Treasures played by them: {}",
        stats.table(),
        stats.treasure_runs,
        stats.treasure_plays
    );
    assert!(stats.treasure_plays > stats.treasure_runs);
}

fn config(kingdom: &[&str], players: u8, seed: u64) -> GameConfig {
    let kingdom = kingdom
        .iter()
        .map(|n| Card::from_name(n).unwrap_or_else(|| panic!("{n}")))
        .collect();
    let mut c = GameConfig::new(players, kingdom, seed);
    c.turn_limit = Some(60);
    c
}

/// A throwaway responder that steers a game somewhere: in a turn it plays or buys a
/// `wanted` card when it can, else plays a Treasure, else buys the dearest card offered,
/// else clicks at random.
fn steer(view: &AgentView, cards: &CardDb, wanted: &[&str], rng: &mut StdRng) -> Selection {
    let d = view.decision.as_ref().unwrap();
    let inter = interaction(view, d, cards);
    if d.kind == "turn" {
        let dearest = d
            .options
            .iter()
            .filter(|o| o.action == "buy")
            .max_by_key(|o| o.cost.as_ref().map(|c| (c.potions, c.coins)));
        let wanted_first = wanted.iter().find_map(|w| {
            d.options
                .iter()
                .find(|o| o.card.as_deref() == Some(*w) && o.action != "select")
        });
        let treasure = inter.treasures.first().map(|&i| option_card(d, i));
        if let Some(o) = wanted_first.or(treasure).or(dearest) {
            return Selection::new(vec![o.index]);
        }
    }
    click_until_legal(d, &inter, rng)
}

#[test]
fn play_treasures_stops_at_a_question_and_when_none_are_left() {
    let cards = card_db();
    let (mut asked, mut none_left) = (0, 0);
    let mut start_view = None;
    for seed in 0..20 {
        let c = config(&["Crown", "Village", "Market", "Moat"], 2, seed);
        let mut table = Table::new("g", c, vec![]).unwrap();
        let mut rng = StdRng::seed_from_u64(seed);
        while let Some(seat) = table.deciding() {
            let view = table.view(seat);
            if let Some(run) = TreasureRun::start(&view, &cards) {
                start_view.get_or_insert_with(|| view.clone());
                let mut v = view;
                while let Some(a) = run.next(&v, &cards) {
                    table.answer(seat, &a).unwrap();
                    v = table.view(seat);
                }
                let d = v.decision.as_ref().unwrap();
                if d.kind == "turn" {
                    assert_eq!(d.phase.as_deref(), Some("buy"));
                    assert!(interaction(&v, d, &cards).treasures.is_empty());
                    none_left += 1;
                } else {
                    assert_eq!(d.source.as_deref(), Some("Crown"), "{d:?}");
                    asked += 1;
                }
                continue;
            }
            let sel = steer(&view, &cards, &["Crown"], &mut rng);
            let d = view.decision.as_ref().unwrap();
            table.answer(seat, &sel.answer(d).unwrap()).unwrap();
        }
    }
    assert!(
        asked > 0 && none_left > 0,
        "asked {asked}, none left {none_left}"
    );

    let view = start_view.unwrap();
    let run = TreasureRun::start(&view, &cards).unwrap();
    let mut night = view.clone();
    night.decision.as_mut().unwrap().phase = Some("night".into());
    assert_eq!(run.next(&night, &cards), None, "the phase changed");
    let mut other = view.clone();
    other.seat = Some(1 - view.seat.unwrap());
    assert_eq!(run.next(&other, &cards), None, "another seat's view");
    let mut waiting = view.clone();
    waiting.decision = None;
    assert_eq!(run.next(&waiting, &cards), None, "no decision");
    let d = view.decision.as_ref().unwrap();
    let first = run.next(&view, &cards).unwrap();
    let o = option_card(d, first.indices.unwrap()[0]);
    assert_eq!(o.action, "play");
    assert!(
        cards
            .get(o.card.as_deref().unwrap())
            .unwrap()
            .types
            .contains(&"Treasure".into())
    );
}

#[test]
fn an_ordered_selection_keeps_the_order_clicked() {
    let cards = card_db();
    for seed in 0..20 {
        let c = config(&["Sentry", "Village"], 2, seed);
        let mut table = Table::new("g", c, vec![]).unwrap();
        let mut rng = StdRng::seed_from_u64(seed);
        while let Some(seat) = table.deciding() {
            let view = table.view(seat);
            let d = view.decision.clone().unwrap();
            if d.ordered && d.options.len() >= 2 && d.min == d.max {
                let inter = interaction(&view, &d, &cards);
                let mut sel = Selection::default();
                let order: Vec<&Target> = inter.targets.iter().rev().collect();
                for t in &order {
                    while sel.click(&d, &t.options) {}
                }
                assert_eq!(sel.picks().len(), d.options.len());
                let first = order[0];
                assert_eq!(sel.positions(&first.options)[0], 0);
                assert!(sel.unclick(&first.options));
                assert_eq!(sel.count(&first.options), first.options.len() - 1);
                assert!(sel.answer(&d).is_none());
                assert!(sel.click(&d, &first.options));
                assert_eq!(
                    sel.positions(&first.options).last(),
                    Some(&(d.options.len() - 1))
                );
                assert!(first.options.contains(sel.picks().last().unwrap()));
                let forward = table.resolve(seat, &sel.answer(&d).unwrap()).unwrap();
                let n = sel.picks().len();
                for i in 0..n {
                    sel.reorder(n - 1, i);
                }
                let back = table.resolve(seat, &sel.answer(&d).unwrap()).unwrap();
                match (forward, back) {
                    (dominion_engine::Response::Cards(a), dominion_engine::Response::Cards(b)) => {
                        assert_eq!(a.into_iter().rev().collect::<Vec<_>>(), b);
                    }
                    other => panic!("{other:?}"),
                }
                return;
            }
            let sel = steer(&view, &cards, &["Sentry"], &mut rng);
            table.answer(seat, &sel.answer(&d).unwrap()).unwrap();
        }
    }
    panic!("no ordered decision met");
}

#[test]
fn log_lines_follow_a_real_game() {
    let cards = card_db();
    let c = config(&["Smithy", "Militia", "Moat", "Village"], 2, 4);
    let mut table = Table::new("g", c, vec!["Ann".into(), "Bo".into()]).unwrap();
    let mut rng = StdRng::seed_from_u64(4);
    while let Some(seat) = table.deciding() {
        let view = table.view(seat);
        let d = view.decision.clone().unwrap();
        let sel = steer(&view, &cards, &["Smithy", "Militia"], &mut rng);
        table.answer(seat, &sel.answer(&d).unwrap()).unwrap();
    }
    let view = table.view_from(PlayerId(0), 0);
    let names = dominion_web::game::logfmt::names(&view);
    assert_eq!(names, ["Ann", "Bo"]);
    let short = dominion_web::game::logfmt::initials(&names);
    assert_eq!(short, ["A", "B"]);
    let log = &view.since_last_decision;
    let lines = log_lines(log, &names, &cards);
    assert_eq!(
        lines.iter().map(|l| l.count as usize).sum::<usize>(),
        log.len()
    );

    let mut turns: BTreeMap<u8, u32> = BTreeMap::new();
    let (mut treasures, mut buys, mut under_smithy, mut cleanup) = (0, 0, 0, 0);
    for (k, l) in lines.iter().enumerate() {
        let e = log.iter().find(|e| e.seq == l.seq).unwrap();
        let text = l.text(&short);
        assert_eq!(l.is_turn(), e.kind == "turn-start");
        if l.is_turn() {
            let p = l.player.unwrap();
            let t = l.turn.unwrap();
            assert_eq!(text, format!("Turn {t} - {}", names[usize::from(p)]));
            assert!(turns.get(&p).is_none_or(|&last| t == last + 1));
            turns.insert(p, t);
            assert_eq!(l.depth, 0);
            continue;
        }
        if let Some(p) = e.player {
            assert_eq!(l.spans.first(), Some(&Span::Player(p)));
        }
        assert!(text.ends_with('.') || text.ends_with(')'), "{text}");
        if l.kind == "played" && text.contains("Copper") {
            assert!(text.ends_with(')'), "{text}");
            treasures += 1;
        }
        if l.kind == "bought" {
            assert!(text.contains(" buys and gains "), "{text}");
            buys += 1;
        }
        let prev = k.checked_sub(1).map(|i| &lines[i]);
        if l.kind == "drew"
            && prev.is_some_and(|p| p.kind == "played" && p.text(&short).contains("Smithy"))
        {
            assert_eq!(l.depth, 1);
            under_smithy += 1;
        }
        if l.kind == "drew" && lines.get(k + 1).is_some_and(|n| n.is_turn()) {
            assert_eq!(l.depth, 0);
            cleanup += 1;
        }
    }
    assert!(turns.values().all(|&t| t > 5));
    assert!(
        treasures > 0 && buys > 0 && under_smithy > 0 && cleanup > 0,
        "{treasures} {buys} {under_smithy} {cleanup}"
    );
}

#[test]
fn every_log_entry_is_worded_by_a_rule() {
    let cards = card_db();
    let names: Vec<String> = ["Ann", "Bo Peep", "Cy", "Di"].map(String::from).to_vec();
    let mut kinds: BTreeMap<String, usize> = BTreeMap::new();
    for seed in 0..200 {
        let players = 2 + (seed % 3) as usize;
        let mut c = GameConfig::random(players as u8, Set::ALL, seed);
        c.turn_limit = Some(16);
        let seats = names[..players].to_vec();
        let mut table = Table::new(format!("g{seed}"), c, seats.clone()).unwrap();
        let mut rng = StdRng::seed_from_u64(seed);
        for _ in 0..600 {
            let Some(seat) = table.deciding() else { break };
            let view = table.view(seat);
            let d = view.decision.clone().unwrap();
            let sel = click_until_legal(&d, &interaction(&view, &d, &cards), &mut rng);
            table.answer(seat, &sel.answer(&d).unwrap()).unwrap();
        }
        let log = table.view_from(PlayerId(0), 0).since_last_decision;
        for e in &log {
            assert!(entry::parse(e, &seats).is_some(), "no rule reads {e:?}");
            *kinds.entry(e.kind.clone()).or_default() += 1;
        }
        let short = dominion_web::game::logfmt::initials(&seats);
        for l in log_lines(&log, &seats, &cards) {
            let text = l.text(&short);
            assert!(
                l.is_turn() || text.ends_with('.') || text.ends_with(')'),
                "{text}"
            );
        }
    }
    println!("{kinds:?}");
}

#[test]
fn way_questions_read_like_dominion_games() {
    let cards = card_db();
    let mut c = config(&["Village", "Smithy"], 2, 3);
    c.landscapes = vec![Card::WayOfTheOx];
    let mut table = Table::new("g", c, vec![]).unwrap();
    let mut rng = StdRng::seed_from_u64(3);
    let (mut hinted, mut asked) = (0, 0);
    while let Some(seat) = table.deciding() {
        let view = table.view(seat);
        let d = view.decision.clone().unwrap();
        if d.prompt.ends_with(dominion_agent::WAY_HINT) {
            hinted += 1;
            assert!(!wording::own_prompt(&d).contains("Way"), "{}", d.prompt);
        }
        if d.options
            .iter()
            .any(|o| o.card.as_deref() == Some("Way of the Ox"))
        {
            asked += 1;
            let source = d.source.clone().unwrap();
            assert_eq!(
                wording::prompt(&d, &cards),
                format!("Consider the Way you play {source}")
            );
        }
        let sel = steer(&view, &cards, &["Village", "Smithy"], &mut rng);
        table.answer(seat, &sel.answer(&d).unwrap()).unwrap();
        if hinted > 0 && asked > 2 {
            return;
        }
    }
    panic!("hinted {hinted}, asked {asked}");
}
