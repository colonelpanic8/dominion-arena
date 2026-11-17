//! The events@1 stream over random games: folding it gives the engine's table.

use dominion_agent::events::Recorder;
use dominion_engine::{
    Card, CardId, DecisionKind, EndReason, ExtraLimit, Game, GameConfig, Mat, Move,
    Phase as EnginePhase, PlayerId, Response, Set, Until, Zone as EngineZone,
};
use dominion_protocol::events::{
    self, CardRef, Effect, Ending, Event, ExtraTurn, Phase, SeatZone, Table, TurnKind, TurnLimits,
    TurnState, Zone,
};
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

/// A response that moves a game on: Actions and Treasures played, the dearest thing
/// bought, so that games end on piles and Fleet's round is played.
fn eager_response(game: &Game, rng: &mut ChaCha8Rng) -> Response {
    let d = game.pending().expect("a decision");
    if let DecisionKind::Turn { moves, .. } = &d.kind
        && rng.random_bool(0.9)
    {
        let plays: Vec<&Move> = (moves.iter())
            .filter(|m| matches!(m, Move::Play(_)))
            .collect();
        if let Some(m) = plays.choose(rng) {
            return Response::Move(**m);
        }
        let price = |m: &Move| {
            let cost = match *m {
                Move::Buy(pile) => game.cost_of(*game.pile(pile).cards.last().expect("a card")),
                Move::BuyLandscape(card) => game.cost(card),
                _ => return None,
            };
            Some(cost.coins as u32 + cost.debt as u32 + 3 * cost.potions as u32)
        };
        if let Some(m) = (moves.iter())
            .filter(|m| price(m).is_some_and(|p| p > 1))
            .max_by_key(|m| (price(m), rng.random::<u8>()))
        {
            return Response::Move(*m);
        }
    }
    random_response(game, rng)
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
    let limits = game.turn_limits();
    for (name, value) in [
        ("actions", turn.actions),
        ("buys", turn.buys),
        ("coins", turn.coins),
        ("potions", turn.potions),
        ("trade-route-mat", game.trade_route_mat()),
        ("sun", game.prophecy().map_or(0, |(_, suns)| suns)),
        ("next-hand", turn.cleanup_draw as u32),
        ("bought", u32::from(turn.bought)),
        (
            "hand-plays",
            limits.hand_plays.map_or(0, |_| turn.hand_plays),
        ),
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
            ("skip-turns", i64::from(p.skip_turns)),
            ("turns-skipped", i64::from(p.turns_skipped)),
        ] {
            counters.insert(format!("{i}/{name}"), value);
        }
    }
    let started = (0..game.num_players()).any(|i| game.player(PlayerId(i as u8)).turns_taken > 0);
    let turn = (started && turn.phase != EnginePhase::End).then(|| TurnState {
        player: turn.player.0,
        number: game.player(turn.player).turns_taken,
        extra: turn.extra.map(|c| c.name().to_string()),
        phase: phase_of(turn.phase),
        kind: match (turn.extra, game.ending()) {
            (Some(_), _) => TurnKind::Extra,
            (None, Some(_)) => TurnKind::Final,
            (None, None) => TurnKind::Regular,
        },
        limits: TurnLimits {
            no_card_buys: limits.no_card_buys,
            hand_plays: limits.hand_plays,
        },
    });
    if turn.is_none() {
        for name in [
            "actions",
            "buys",
            "coins",
            "potions",
            "next-hand",
            "bought",
            "hand-plays",
        ] {
            counters.remove(name);
        }
    }
    counters.retain(|_, value| *value != 0);
    let aside = |c: &CardId| matches!(game.zone(*c), EngineZone::SetAside(_));
    let effects = (game.lasting().iter())
        .map(|l| {
            let view = game.view(l.owner);
            let mut holds: Vec<CardId> = vec![];
            for &c in l.data.pick.iter().chain(&l.data.cards) {
                if aside(&c) && !holds.contains(&c) {
                    holds.push(c);
                }
            }
            Effect {
                id: l.id,
                card: l.card.name().to_string(),
                source: l.source.as_ref().map(card),
                owner: l.owner.0,
                until: match view.lasting_ends(l) {
                    Until::EndOfTurn => events::Until::EndOfTurn,
                    Until::StartOfTurn(p) => events::Until::StartOfTurn(p.0),
                    Until::ExtraTurn => events::Until::ExtraTurn,
                    Until::Removed => events::Until::Removed,
                },
                ending: l.ending,
                keeps: as_arrived(&l.keeps),
                holds: as_arrived(&holds),
                tokens: view.lasting_tokens(l),
            }
        })
        .collect();
    let face_up = (0..game.num_players())
        .flat_map(|i| game.player(PlayerId(i as u8)).face_up.iter().map(card))
        .collect();
    let extra_turns = (game.extra_turns().iter())
        .map(|e| ExtraTurn {
            id: e.id,
            player: e.player.0,
            card: e.source.name().to_string(),
            limit: match e.limit {
                ExtraLimit::None => events::ExtraLimit::None,
                ExtraLimit::ThirdInARow => events::ExtraLimit::ThirdInARow,
            },
            effect: e.lasting,
        })
        .collect();
    let ending = game.ending().map(|reason| Ending {
        reason: match reason {
            EndReason::ProvincesGone => "provinces",
            EndReason::ColoniesGone => "colonies",
            EndReason::PilesEmpty => "piles",
            EndReason::TurnLimit => "turn-limit",
            EndReason::Resigned => "resigned",
        }
        .into(),
        final_round: game.final_round().iter().map(|p| p.0).collect(),
    });
    Table {
        zones,
        counters,
        turn,
        ended: None,
        effects,
        face_up,
        extra_turns,
        ending,
    }
}

fn phase_of(phase: EnginePhase) -> Phase {
    match phase {
        EnginePhase::Start => Phase::Start,
        EnginePhase::Action => Phase::Action,
        EnginePhase::Buy => Phase::Buy,
        EnginePhase::Night => Phase::Night,
        EnginePhase::Cleanup => Phase::Cleanup,
        EnginePhase::End => Phase::End,
    }
}

fn check(game: &Game, recorder: &Recorder, events: Option<&[Event]>, what: &str) {
    let mut folded = recorder.table().clone();
    folded.ended = None;
    let expected = engine_table(game);
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
        assert_eq!(folded.turn, expected.turn, "{what}");
        assert_eq!(folded.effects, expected.effects, "{what}");
        assert_eq!(folded.face_up, expected.face_up, "{what}");
        assert_eq!(folded.extra_turns, expected.extra_turns, "{what}");
        assert_eq!(folded.ending, expected.ending, "{what}");
    }
    if let Some(turn) = &folded.turn {
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

/// Plays one game to its end, holding the folded stream to the engine's table at every
/// decision.
fn follow(config: GameConfig, seed: u64, respond: fn(&Game, &mut ChaCha8Rng) -> Response) {
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
        let response = respond(&game, &mut rng);
        game.respond(response).unwrap();
        answers += 1;
    }
    assert!(recorder.table().ended.is_some(), "seed {seed}");
}

#[test]
fn the_fold_of_the_full_stream_is_the_engines_table() {
    let sets = released();
    for seed in 0..seeds(40) {
        let players = 2 + (seed % 3) as u8;
        let mut config = GameConfig::random(players, &sets, seed);
        config.turn_limit = Some(40 + 20 * (seed % 5) as u32);
        follow(config, seed, random_response);
    }
}

/// Kingdoms of cards about time: Durations holding and keeping cards, effects that change,
/// extra turns, skipped turns, returning to the Action phase, Fleet's round.
#[test]
fn the_fold_follows_effects_and_turns() {
    use Card::*;
    const CARDS: &[Card] = &[
        Outpost,
        Haven,
        Odysseys,
        Wizards,
        Archive,
        BridgeTroll,
        Taskmaster,
        Quartermaster,
        CargoShip,
        Crypt,
        Forts,
        ThroneRoom,
        KingsCourt,
        Villa,
        Cavalry,
        Necromancer,
        Lighthouse,
        Corsair,
        Prince,
        Church,
        Research,
        Sailor,
        Tactician,
        Cage,
        Grotto,
        Underling,
        Wharf,
        FaithfulHound,
        BlessedVillage,
        Gear,
        SwampHag,
        HauntedWoods,
        Monkey,
        FlagBearer,
        Mandarin,
        Daimyo,
        Gatekeeper,
    ];
    const LANDSCAPES: &[Card] = &[
        Mission,
        SeizeTheDay,
        Expedition,
        Journey,
        Fleet,
        Donate,
        MountainPass,
        IslandFolk,
    ];
    for seed in 0..seeds(40) {
        let mut rng = ChaCha8Rng::seed_from_u64(seed);
        let kingdom = CARDS.choose_multiple(&mut rng, 10).copied().collect();
        let mut config = GameConfig::new(2 + (seed % 2) as u8, kingdom, seed);
        config.landscapes = LANDSCAPES.choose_multiple(&mut rng, 3).copied().collect();
        config.turn_limit = Some(150);
        let respond = if seed % 4 == 0 {
            random_response
        } else {
            eager_response
        };
        follow(config, seed, respond);
    }
}
