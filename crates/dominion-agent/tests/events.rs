//! The events@1 stream over random games: folding it gives the engine's table.

use dominion_agent::events::Recorder;
use dominion_engine::{Card, CardId, DecisionKind, Game, GameConfig, Mat, PlayerId, Response, Set};
use dominion_protocol::events::{CardRef, Event, Moment, Phase, SeatZone, Table, What, Zone};
use rand::seq::{IndexedRandom, SliceRandom};
use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;
use std::collections::BTreeMap;

fn seeds(default: u64) -> u64 {
    std::env::var("FUZZ_SEEDS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(default)
}

fn released() -> Vec<Set> {
    (Set::ALL.iter().copied())
        .filter(|&s| s != Set::Arcana)
        .collect()
}

/// A random legal response.
fn random_response(game: &Game, rng: &mut ChaCha8Rng) -> Response {
    let d = game.pending().expect("a decision");
    let some = |n: usize, min: usize, max: usize, rng: &mut ChaCha8Rng| {
        let mut picks: Vec<usize> = (0..n).collect();
        picks.shuffle(rng);
        picks.truncate(rng.random_range(min..=max.min(n)));
        picks
    };
    match &d.kind {
        DecisionKind::Turn { moves, .. } => Response::Move(*moves.choose(rng).expect("a move")),
        DecisionKind::Cards {
            candidates,
            min,
            max,
            ..
        } => Response::Cards(
            (some(candidates.len(), *min, *max, rng).into_iter())
                .map(|i| candidates[i])
                .collect(),
        ),
        DecisionKind::Pile { piles, optional } => {
            Response::Pile(match *optional && rng.random_bool(0.2) {
                true => None,
                false => piles.choose(rng).copied(),
            })
        }
        DecisionKind::Choose {
            options, min, max, ..
        } => Response::Options(
            (some(options.len(), *min, *max, rng).into_iter())
                .map(|i| options[i])
                .collect(),
        ),
        DecisionKind::NameCard => Response::Name(
            *[
                Card::Copper,
                Card::Silver,
                Card::Gold,
                Card::Estate,
                Card::Curse,
            ]
            .choose(rng)
            .expect("a name"),
        ),
        DecisionKind::Number { min, max } => Response::Number(rng.random_range(*min..=*max)),
    }
}

/// The table as the engine has it.
fn engine_table(game: &Game) -> Table {
    let card = |id: &CardId| CardRef::seen(game.printed(*id).name(), id.0);
    let top_first = |cards: &[CardId]| cards.iter().rev().map(card).collect::<Vec<_>>();
    let as_arrived = |cards: &[CardId]| cards.iter().map(card).collect::<Vec<_>>();
    let mut zones = BTreeMap::new();
    for i in 0..game.num_players() {
        let p = game.player(PlayerId(i as u8));
        let seat = |z| Zone::Seat(i as u8, z);
        zones.insert(seat(SeatZone::Deck), top_first(&p.deck));
        zones.insert(seat(SeatZone::Discard), top_first(&p.discard));
        zones.insert(seat(SeatZone::Hand), as_arrived(&p.hand));
        zones.insert(seat(SeatZone::Play), as_arrived(&p.in_play));
        zones.insert(seat(SeatZone::Aside), as_arrived(&p.set_aside));
        zones.insert(seat(SeatZone::Looking), as_arrived(&p.looking));
        for (mat, zone) in [
            (Mat::Tavern, SeatZone::Tavern),
            (Mat::Island, SeatZone::Island),
            (Mat::NativeVillage, SeatZone::NativeVillage),
            (Mat::Exile, SeatZone::Exile),
        ] {
            zones.insert(seat(zone), as_arrived(p.mat(mat)));
        }
    }
    zones.insert(Zone::Trash, top_first(game.trash()));
    zones.retain(|_, cards| !cards.is_empty());
    let mut counters = BTreeMap::new();
    let mut names: Vec<Zone> = vec![];
    for pile in game.piles() {
        let named = |name: String| match pile.holder {
            true => Zone::Aside(name),
            false => Zone::Pile(name),
        };
        let base = pile.card.name().to_string();
        let zone = (1..)
            .map(|n| match n {
                1 => named(base.clone()),
                n => named(format!("{base} ({n})")),
            })
            .find(|z| !names.contains(z))
            .unwrap();
        names.push(zone.clone());
        for (name, value) in [
            ("vp", i64::from(pile.vp_tokens)),
            ("debt", i64::from(pile.debt_tokens)),
            ("embargo", i64::from(pile.embargo)),
            ("trade-route", i64::from(pile.trade_route)),
        ] {
            counters.insert(format!("{zone}/{name}"), value);
        }
        zones.insert(zone, top_first(&pile.cards));
    }
    let turn = game.turn();
    for (name, value) in [
        ("actions", turn.actions),
        ("buys", turn.buys),
        ("coins", turn.coins),
        ("potions", turn.potions),
        ("trade-route-mat", game.trade_route_mat()),
        ("sun", game.prophecy().map_or(0, |(_, suns)| suns)),
    ] {
        counters.insert(name.to_string(), i64::from(value));
    }
    for i in 0..game.num_players() {
        let p = game.player(PlayerId(i as u8));
        for (name, value) in [
            ("coffers", i64::from(p.coffers)),
            ("villagers", i64::from(p.villagers)),
            ("favors", i64::from(p.favors)),
            ("debt", i64::from(p.debt)),
            ("vp-tokens", i64::from(p.vp_tokens)),
            ("pirate-ship", i64::from(p.pirate_ship)),
        ] {
            counters.insert(format!("{i}/{name}"), value);
        }
    }
    counters.retain(|_, value| *value != 0);
    Table {
        zones,
        counters,
        turn: None,
        ended: None,
    }
}

fn check(game: &Game, recorder: &Recorder, events: Option<&[Event]>, what: &str) {
    let mut folded = recorder.table().clone();
    let turn = folded.turn.take();
    folded.ended = None;
    let mut expected = engine_table(game);
    if turn.is_none() {
        // Before the first turn the engine's turn is a blank one.
        for name in ["actions", "buys", "coins", "potions"] {
            expected.counters.remove(name);
        }
    }
    if folded != expected {
        for (zone, cards) in &expected.zones {
            assert_eq!(folded.zone(zone), cards, "{what}: {zone}");
        }
        assert_eq!(
            folded.zones.keys().collect::<Vec<_>>(),
            expected.zones.keys().collect::<Vec<_>>(),
            "{what}"
        );
        assert_eq!(folded.counters, expected.counters, "{what}");
    }
    if let (None, Some(turn)) = (game.outcome(), turn) {
        assert_eq!(turn.player, game.turn().player.0, "{what}");
        let phase = serde_json::to_value(turn.phase).unwrap();
        let engine = format!("{:?}", game.turn().phase).to_lowercase();
        assert_eq!(phase, engine, "{what}");
        assert_ne!(turn.phase, Phase::End, "{what}");
    }
    // The stream folds to the same table for anyone who reads it as JSON.
    let Some(events) = events else {
        return;
    };
    let mut read = Table::default();
    for (i, event) in events.iter().enumerate() {
        assert_eq!(event.seq, i as u64, "{what}");
        let json = serde_json::to_string(event).unwrap();
        let back: Event =
            serde_json::from_str(&json).unwrap_or_else(|e| panic!("{what}: {json}: {e}"));
        assert_eq!(&back, event, "{what}: {json}");
        read.apply(&back).unwrap_or_else(|e| panic!("{what}: {e}"));
    }
    assert!(
        &read == recorder.table(),
        "{what}: read back, the stream folds differently"
    );
}

/// Whether `e` is an occurrence: something other events can happen inside.
fn occurrence(e: &What) -> bool {
    matches!(
        e,
        What::Turn(_)
            | What::Phase { .. }
            | What::Play(_)
            | What::Buy { .. }
            | What::Resolve { .. }
            | What::Gain(_)
            | What::Trash(_)
            | What::Discard(_)
            | What::Shuffle(_)
    )
}

fn phase_of(e: &What) -> Option<Phase> {
    match e {
        What::Phase { phase } => Some(*phase),
        _ => None,
    }
}

/// Holds what each event says caused it to what the stream shows happened.
fn check_causes(events: &[Event], what: &str) {
    let at = |seq: u64| &events[seq as usize].what;
    let mut turn = None;
    // How many cards each seat could draw from, as each play began.
    let mut table = Table::default();
    let mut drawable = vec![];
    for e in events {
        let seq = e.seq;
        let fail = |why: &str| {
            let json = serde_json::to_string(e).unwrap();
            let cause =
                e.by.map(|by| serde_json::to_string(&events[by as usize]).unwrap());
            panic!("{what}: event {seq}: {why}\n{json}\nby {cause:?}")
        };
        if let Some(by) = e.by
            && !occurrence(at(by))
        {
            fail("caused by something that is no occurrence");
        }
        if let What::Turn(_) = e.what {
            turn = Some(seq);
        }
        // Everything in a turn happens inside it; between turns, only the game's own.
        let mut root = seq;
        while let Some(by) = events[root as usize].by {
            root = by;
        }
        match (&e.what, e.by) {
            (
                What::Turn(_) | What::TurnSkipped { .. } | What::GameEnd(_) | What::Choose(_),
                None,
            ) => {}
            (What::Resolve { when, .. }, None) if turn.is_none() && *when == Moment::GameStart => {}
            (_, None) if turn.is_none() => {}
            (_, None) => fail("nothing caused it"),
            _ if Some(root) != turn && turn.is_some() => fail("not inside the turn in progress"),
            _ => {}
        }
        let by = e.by.map(at);
        let by_phase = by.and_then(phase_of);
        match &e.what {
            What::Resolve {
                when,
                about,
                target,
                origin,
                ..
            } => {
                let holds = |event: Option<&What>, card: &CardRef| {
                    event
                        .and_then(What::as_move)
                        .is_some_and(|(_, m)| m.cards.contains(card))
                };
                let right = match when {
                    Moment::GameStart => by.is_none(),
                    Moment::StartTurn => matches!(by, Some(What::Turn(_))),
                    Moment::StartBuyPhase => by_phase == Some(Phase::Buy),
                    Moment::StartNight => by_phase == Some(Phase::Night),
                    Moment::StartCleanup => by_phase == Some(Phase::Cleanup),
                    Moment::EndTurn => by_phase == Some(Phase::Cleanup),
                    Moment::DiscardFromPlay | Moment::EndBuyPhase => {
                        by_phase.is_none_or(|p| p == Phase::Cleanup || p == Phase::Buy)
                    }
                    Moment::Play | Moment::AfterPlay => matches!(by, Some(What::Play(_))),
                    Moment::Attack => match by {
                        Some(What::Play(play)) => target.is_some_and(|t| t != play.player),
                        _ => false,
                    },
                    Moment::Buy => matches!(by, Some(What::Buy { .. })),
                    Moment::Gain => {
                        matches!(by, Some(What::Gain(_))) && holds(by, about.as_ref().unwrap())
                    }
                    Moment::Trash => {
                        matches!(by, Some(What::Trash(_))) && holds(by, about.as_ref().unwrap())
                    }
                    Moment::Discard => {
                        matches!(by, Some(What::Discard(_))) && holds(by, about.as_ref().unwrap())
                    }
                    Moment::Shuffle => matches!(by, Some(What::Shuffle(_))),
                    Moment::WouldGain | Moment::LeaveSupply => about.is_some(),
                    Moment::WouldShuffle => true,
                };
                if !right {
                    fail(&format!("a resolve {when:?} with the wrong cause"));
                }
                if let Some(origin) = origin
                    && !matches!(
                        at(*origin),
                        What::Play(_) | What::Resolve { .. } | What::Buy { .. }
                    )
                {
                    fail("an origin that makes no lasting effect");
                }
            }
            What::Play(play) => {
                match (by_phase, &play.from) {
                    (Some(Phase::Action), Some(_)) => {
                        // The Action it costs.
                        let next = events.get(seq as usize + 1).map(|e| (&e.what, e.by));
                        if !matches!(next, Some((What::Counter(c), Some(b))) if b == seq && c.counter == "actions" && c.delta == -1)
                        {
                            fail("a play from the Action phase doesn't spend an Action");
                        }
                    }
                    (Some(Phase::Buy | Phase::Night), Some(_)) => {}
                    (None, _) if !matches!(by, Some(What::Turn(_))) => {}
                    _ => fail("a play the phase can't make"),
                }
                if let Some(c) = play.card.name().and_then(dominion_engine::Card::from_name) {
                    let seat = play.player;
                    let cards = table.zone(&Zone::Seat(seat, SeatZone::Deck)).len()
                        + table.zone(&Zone::Seat(seat, SeatZone::Discard)).len();
                    if c.def().plus.cards >= 2 && cards > 0 {
                        drawable.push(seq);
                    }
                }
            }
            What::Buy { .. } if by_phase.is_some() => {
                let next = events.get(seq as usize + 1).map(|e| (&e.what, e.by));
                if !matches!(next, Some((What::Counter(c), Some(b))) if b == seq && (c.counter == "buys" || c.counter == "actions") && c.delta == -1)
                {
                    fail("a buy doesn't spend a Buy");
                }
            }
            What::Draw(_) | What::Discard(_) | What::Shuffle(_)
                if by_phase.is_some_and(|p| p != Phase::Cleanup) =>
            {
                fail("moved by a phase other than Clean-up");
            }
            _ if matches!(by, Some(What::Turn(_)))
                && !matches!(e.what, What::Phase { .. } | What::Resolve { .. }) =>
            {
                fail("only phases and start-of-turn abilities happen at the start of a turn");
            }
            _ => {}
        }
        table.apply(e).unwrap();
    }
    // A card's +Cards are drawn by its play, unless something changed what the play does.
    for play in drawable {
        let changed = events[play as usize..].iter().any(|e| {
            e.by == Some(play)
                && matches!(
                    e.what,
                    What::Way { .. }
                        | What::Resolve {
                            when: Moment::Play,
                            ..
                        }
                )
        });
        let drew = (events[play as usize..].iter())
            .any(|e| e.by == Some(play) && matches!(e.what, What::Draw(_)));
        if !changed && !drew {
            let json = serde_json::to_string(&events[play as usize]).unwrap();
            panic!("{what}: event {play} draws nothing of its own\n{json}");
        }
    }
}

#[test]
fn the_fold_of_the_full_stream_is_the_engines_table() {
    let sets = released();
    for seed in 0..seeds(40) {
        let players = 2 + (seed % 3) as u8;
        let mut config = GameConfig::random(players, &sets, seed);
        config.turn_limit = Some(40 + 20 * (seed % 5) as u32);
        let mut game = Game::new_journaled(config).unwrap();
        let mut rng = ChaCha8Rng::seed_from_u64(seed);
        let mut recorder = Recorder::new();
        let mut events = vec![];
        let mut answers = 0;
        loop {
            let what = format!("seed {seed}, after {answers} answers");
            let new = recorder
                .record(&mut game, &[])
                .unwrap_or_else(|e| panic!("{what}: {e}"));
            let recent: Vec<String> = (new.iter())
                .map(|e| serde_json::to_string(e).unwrap())
                .collect();
            let what = format!("{what}, after\n{}\n", recent.join("\n"));
            events.extend(new);
            // Reading every event back at every decision is quadratic: now and then.
            let all = answers % 97 == 0 || game.outcome().is_some();
            check(&game, &recorder, all.then_some(&events[..]), &what);
            if !all {
                assert_eq!(
                    events.len() as u64,
                    events.last().map_or(0, |e| e.seq + 1),
                    "{what}"
                );
            }
            if game.outcome().is_some() {
                break;
            }
            let response = random_response(&game, &mut rng);
            game.respond(response).unwrap();
            answers += 1;
        }
        assert!(recorder.table().ended.is_some(), "seed {seed}");
        check_causes(&events, &format!("seed {seed}"));
    }
}
