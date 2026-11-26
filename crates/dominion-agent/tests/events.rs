//! The events@1 stream over random games: folding it gives the engine's table.

use dominion_agent::events::Recorder;
use dominion_engine::{
    Card, CardId, DecisionKind, EndReason, ExtraLimit, Game, GameConfig, Kind, Mat, Move,
    Phase as EnginePhase, PileId, PlayerId, Response, Set, SetupRole, Step, Token, Trace, Until,
    Verb, Zone as EngineZone,
};
use dominion_protocol::events::{
    self, CardRef, CounterKey, CounterName, CounterOf, Effect, Ending, Event, ExtraTurn, Moment,
    Pack, Phase, PileRole, PileState, Recipe, RecipeGroup, RecipeKind, Role, SeatZone, Table,
    TokenAt, TokenKind, TokenState, TurnKind, TurnLimits, TurnState, What, Zone,
};
use rand::seq::{IndexedRandom, SliceRandom};
use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;
use std::collections::BTreeMap;

mod support;
use support::responders::{eager_response, random_response};

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

/// The table as the engine has it.
fn engine_table(game: &Game) -> Table {
    let card = |id: &CardId| CardRef::named(game.printed(*id).name());
    let top_first = |cards: &[CardId]| cards.iter().rev().map(card).collect::<Vec<_>>();
    let bag = |cards: &[CardId]| {
        let mut cards: Vec<CardRef> = cards.iter().map(card).collect();
        cards.sort();
        cards
    };
    let mut zones = BTreeMap::new();
    for i in 0..game.num_players() {
        let p = game.player(PlayerId(i as u8));
        let seat = |z| Zone::Seat(i as u8, z);
        zones.insert(seat(SeatZone::Deck), top_first(&p.deck));
        zones.insert(seat(SeatZone::Discard), top_first(&p.discard));
        zones.insert(seat(SeatZone::Hand), bag(&p.hand));
        zones.insert(seat(SeatZone::Play), bag(&p.in_play));
        zones.insert(seat(SeatZone::Aside), bag(&p.set_aside));
        zones.insert(seat(SeatZone::Looking), bag(&p.looking));
        for (mat, zone) in [
            (Mat::Tavern, SeatZone::Tavern),
            (Mat::Island, SeatZone::Island),
            (Mat::NativeVillage, SeatZone::NativeVillage),
            (Mat::Exile, SeatZone::Exile),
        ] {
            zones.insert(seat(zone), bag(p.mat(mat)));
        }
    }
    zones.insert(Zone::Trash, top_first(game.trash()));
    zones.retain(|_, cards| !cards.is_empty());
    let notes = game.watch().notes();
    let named = |cards: &mut dyn Iterator<Item = &Card>| {
        cards.map(|c| CardRef::named(c.name())).collect::<Vec<_>>()
    };
    let druid: Vec<Card> = (notes.iter())
        .filter(|n| n.source == Card::Druid)
        .filter_map(|n| n.card)
        .collect();
    for (zone, cards) in [
        (Zone::Boons(Pack::Deck), &game.boons().draw),
        (Zone::Boons(Pack::Discard), &game.boons().discard),
        (Zone::Boons(Pack::Druid), &druid),
        (Zone::Hexes(Pack::Deck), &game.hexes().draw),
        (Zone::Hexes(Pack::Discard), &game.hexes().discard),
    ] {
        if !cards.is_empty() {
            zones.insert(zone, named(&mut cards.iter().rev()));
        }
    }
    for i in 0..game.num_players() {
        let kept = &game.player(PlayerId(i as u8)).boons_kept;
        if !kept.is_empty() {
            let mut kept = named(&mut kept.iter());
            kept.sort();
            zones.insert(Zone::Seat(i as u8, SeatZone::Boons), kept);
        }
    }
    let mut counters = BTreeMap::new();
    let mut piles = BTreeMap::new();
    let mut names: Vec<Zone> = vec![];
    for (i, pile) in game.piles().iter().enumerate() {
        let id = PileId(i as u8);
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
        let favors = (notes.iter())
            .filter(|n| n.source == Card::FamilyOfInventors && n.pile == Some(id))
            .map(|n| i64::from(n.count))
            .sum();
        for (name, value) in [
            (CounterName::Vp, i64::from(pile.vp_tokens)),
            (CounterName::Debt, i64::from(pile.debt_tokens)),
            (CounterName::Embargo, i64::from(pile.embargo)),
            (CounterName::TradeRoute, i64::from(pile.trade_route)),
            (CounterName::Favors, favors),
        ] {
            counters.insert(CounterKey::new(CounterOf::Pile(zone.clone()), name), value);
        }
        let mut roles: Vec<PileRole> = vec![];
        for n in &notes {
            let Some(role) = n.setup else { continue };
            let here = n.pile == Some(id)
                || (role == SetupRole::SetAside && pile.holder && n.card == Some(pile.card));
            let role = PileRole {
                card: n.source.name().to_string(),
                role: match role {
                    SetupRole::SetAside => Role::SetAside,
                    SetupRole::Bane => Role::Bane,
                    SetupRole::ExtraPile => Role::ExtraPile,
                    SetupRole::ChosenPile => Role::ChosenPile,
                    SetupRole::Deck => Role::Deck,
                },
            };
            if here && !roles.contains(&role) {
                roles.push(role);
            }
        }
        let name = |c: Card| c.name().to_string();
        piles.insert(
            zone.clone(),
            PileState {
                card: name(pile.card),
                supply: pile.in_supply,
                hidden: pile.hidden,
                removed: pile.removed && !pile.holder,
                roles,
                traits: pile.traits.iter().map(|&t| name(t)).collect(),
                recipe: pile.structure.as_ref().map(|s| Recipe {
                    kind: match s.kind {
                        dominion_engine::PileKind::Stacked => RecipeKind::Stacked,
                        dominion_engine::PileKind::Rotating => RecipeKind::Rotating,
                        dominion_engine::PileKind::Shuffled => RecipeKind::Shuffled,
                    },
                    cards: (s.cards.iter())
                        .map(|g| RecipeGroup {
                            card: name(g.card),
                            count: g.count,
                        })
                        .collect(),
                    sample: s.sample_size,
                }),
            },
        );
        zones.insert(zone, top_first(&pile.cards));
    }
    let turn = game.turn();
    let limits = game.turn_limits();
    for (name, value) in [
        (CounterName::Actions, turn.actions),
        (CounterName::Buys, turn.buys),
        (CounterName::Coins, turn.coins),
        (CounterName::Potions, turn.potions),
        (CounterName::TradeRouteMat, game.trade_route_mat()),
        (
            CounterName::Sun,
            game.prophecy().map_or(0, |(_, suns)| suns),
        ),
        (CounterName::NextHand, turn.cleanup_draw as u32),
        (CounterName::Bought, u32::from(turn.bought)),
        (
            CounterName::HandPlays,
            limits.hand_plays.map_or(0, |_| turn.hand_plays),
        ),
    ] {
        counters.insert(CounterKey::global(name), i64::from(value));
    }
    for n in &notes {
        if n.source.kind() == Kind::Landmark && n.pile.is_none() && n.player.is_none() {
            let key = CounterOf::Landscape(n.source.name().to_string());
            counters.insert(CounterKey::new(key, CounterName::Vp), i64::from(n.count));
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
            CounterName::Actions,
            CounterName::Buys,
            CounterName::Coins,
            CounterName::Potions,
            CounterName::NextHand,
            CounterName::Bought,
            CounterName::HandPlays,
        ] {
            counters.remove(&CounterKey::global(name));
        }
    }
    let mut tokens = vec![];
    let mut holdings = BTreeMap::<String, Vec<u8>>::new();
    for i in 0..game.num_players() {
        let seat = i as u8;
        let p = game.player(PlayerId(seat));
        let plot = (notes.iter())
            .filter(|n| n.source == Card::SinisterPlot && n.player == Some(PlayerId(seat)))
            .map(|n| i64::from(n.count))
            .sum();
        for (name, value) in [
            (CounterName::Coffers, i64::from(p.coffers)),
            (CounterName::Villagers, i64::from(p.villagers)),
            (CounterName::Favors, i64::from(p.favors)),
            (CounterName::Debt, i64::from(p.debt)),
            (CounterName::VpTokens, i64::from(p.vp_tokens)),
            (CounterName::PirateShip, i64::from(p.pirate_ship)),
            (CounterName::SinisterPlot, plot),
            (CounterName::SkipTurns, i64::from(p.skip_turns)),
            (CounterName::TurnsSkipped, i64::from(p.turns_skipped)),
        ] {
            counters.insert(CounterKey::seat(seat, name), value);
        }
        for (&token, &pile) in &p.tokens {
            let token = match token {
                Token::PlusCard => TokenKind::PlusCard,
                Token::PlusAction => TokenKind::PlusAction,
                Token::PlusBuy => TokenKind::PlusBuy,
                Token::PlusCoin => TokenKind::PlusCoin,
                Token::MinusCost => TokenKind::MinusCost,
                Token::Trashing => TokenKind::Trashing,
                Token::Estate => TokenKind::Estate,
            };
            let at = pile.map_or(TokenAt::Player, |id| {
                TokenAt::Pile(names[id.index()].clone())
            });
            tokens.push(TokenState {
                player: seat,
                token,
                at,
            });
        }
        for (out, token, at) in [
            (p.minus_card, TokenKind::MinusCard, TokenAt::Player),
            (p.minus_coin, TokenKind::MinusCoin, TokenAt::Player),
            (!p.journey_face_up, TokenKind::Journey, TokenAt::FaceDown),
        ] {
            if out {
                tokens.push(TokenState {
                    player: seat,
                    token,
                    at,
                });
            }
        }
        for card in p.projects.iter().chain(&p.states) {
            holdings
                .entry(card.name().to_string())
                .or_default()
                .push(seat);
        }
    }
    tokens.sort_by_key(|t| (t.player, t.token));
    for &(card, holder) in game.artifacts() {
        if let Some(p) = holder {
            holdings
                .entry(card.name().to_string())
                .or_default()
                .push(p.0);
        }
    }
    for holders in holdings.values_mut() {
        holders.sort();
    }
    counters.retain(|_, value| *value != 0);
    let up: Vec<CardId> = (0..game.num_players())
        .flat_map(|i| game.player(PlayerId(i as u8)).face_up.iter().copied())
        .collect();
    let effects = (game.lasting().iter())
        .map(|l| {
            let view = game.view(l.owner);
            let holds = &l.holds;
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
                keeps: bag(&l.keeps),
                face_up: bag(&(holds.iter().copied())
                    .filter(|c| up.contains(c))
                    .collect::<Vec<_>>()),
                holds: bag(holds),
                tokens: view.lasting_tokens(l),
            }
        })
        .collect();
    let mut face_up = BTreeMap::new();
    for i in 0..game.num_players() {
        let p = game.player(PlayerId(i as u8));
        for (zone, cards) in [
            (SeatZone::Aside, &p.set_aside),
            (SeatZone::Looking, &p.looking),
        ] {
            let up: Vec<CardId> = (cards.iter().copied())
                .filter(|c| p.face_up.contains(c))
                .collect();
            if !up.is_empty() {
                face_up.insert(Zone::Seat(i as u8, zone), bag(&up));
            }
        }
    }
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
            EndReason::Stalled => "stalled",
        }
        .into(),
        final_round: game.final_round().iter().map(|p| p.0).collect(),
    });
    let mut cards: Vec<Card> = (0..game.card_count())
        .map(|i| game.printed(CardId(i as u32)))
        .collect();
    cards.sort();
    cards.dedup();
    let names_of = |types: dominion_engine::Types| types.names().map(str::to_string).collect();
    Table {
        zones,
        counters,
        turn,
        ended: None,
        piles,
        landscapes: {
            let mut names: Vec<String> = (game.landscapes().iter())
                .chain(game.artifacts().iter().map(|(a, _)| a))
                .map(|c| c.name().to_string())
                .collect();
            names.sort();
            names
        },
        prophecy: game.prophecy().map(|(c, _)| c.name().to_string()),
        holdings,
        tokens,
        notes: (notes.iter())
            .filter(|n| dominion_engine::table_note(n))
            .map(|n| dominion_protocol::events::Note {
                source: n.source.name().to_string(),
                player: n.player.map(|p| p.0),
                pile: n.pile.map(|id| names[id.index()].clone()),
                card: n.card.map(|c| c.name().to_string()),
                count: n.count,
            })
            .collect(),
        costs: (cards.iter())
            .map(|&c| {
                let cost = game.cost(c);
                let cost = dominion_protocol::events::Cost {
                    coins: u32::from(cost.coins),
                    potions: u32::from(cost.potions),
                    debt: u32::from(cost.debt),
                };
                (c.name().to_string(), cost)
            })
            .collect(),
        types: (cards.iter())
            .filter(|&&c| game.card_types(c) != c.types())
            .map(|&c| (c.name().to_string(), names_of(game.card_types(c))))
            .collect(),
        effects,
        face_up,
        extra_turns,
        ending,
        progress: Default::default(),
        face_down: {
            let down = bag(&game.face_down_in_trash());
            (!down.is_empty())
                .then_some((Zone::Trash, down))
                .into_iter()
                .collect()
        },
        kept: (0..game.num_players())
            .filter_map(|i| {
                let kept: Vec<CardId> = (game.player(PlayerId(i as u8)).in_play.iter().copied())
                    .filter(|c| game.lasting().iter().any(|l| l.keeps.contains(c)))
                    .collect();
                (!kept.is_empty()).then(|| (Zone::Seat(i as u8, SeatZone::Play), bag(&kept)))
            })
            .collect(),
    }
}

/// An effect holds only cards set aside in its owner's zone, a card set aside is held by at
/// most one effect, and one held by none is no effect's (a card gained to be set aside,
/// before its effect is made): what the table says of holdings is true of the cards,
/// however the engine came to it.
fn check_holdings(game: &Game, what: &str) {
    for l in game.lasting() {
        for &c in &l.holds {
            assert_eq!(
                game.zone(c),
                EngineZone::SetAside(l.owner),
                "{what}: {} holds {} not set aside",
                l.card,
                game.card(c)
            );
        }
    }
    for i in 0..game.num_players() {
        for &c in &game.player(PlayerId(i as u8)).set_aside {
            let by: Vec<Card> = (game.lasting().iter())
                .filter(|l| l.holds.contains(&c))
                .map(|l| l.card)
                .collect();
            assert!(by.len() <= 1, "{what}: {} held by {by:?}", game.card(c));
            let names: Vec<Card> = (game.lasting().iter())
                .filter(|l| l.data.pick == Some(c) || l.data.cards.contains(&c))
                .map(|l| l.card)
                .collect();
            assert!(
                !by.is_empty() || names.is_empty(),
                "{what}: {} set aside, held by none, named by {names:?}",
                game.card(c)
            );
        }
    }
}

fn phase_of(phase: EnginePhase) -> Phase {
    match phase {
        EnginePhase::Start => Phase::Start,
        EnginePhase::Action => Phase::Action,
        EnginePhase::Buy => Phase::Buy,
        EnginePhase::Night => Phase::Night,
        EnginePhase::Cleanup => Phase::Cleanup,
        EnginePhase::End => unreachable!("no turn is in progress"),
    }
}

/// Boons and Hexes out of their decks: taken, being received, or kept (which the
/// engine's table says whose).
fn out_of_decks(table: &mut Table) -> Vec<CardRef> {
    let mut out = vec![];
    for (zone, cards) in &mut table.zones {
        if let Zone::Seat(_, SeatZone::Boons | SeatZone::Hexes) = zone {
            out.append(cards);
        }
    }
    table.zones.retain(|_, cards| !cards.is_empty());
    out.sort();
    out
}

fn check(game: &Game, recorder: &Recorder, events: Option<&[Event]>, what: &str) {
    check_holdings(game, what);
    let mut folded = recorder.table().clone();
    folded.ended = None;
    let mut expected = engine_table(game);
    // The engine says who keeps a Boon, not who took the Boons out of their deck: those it
    // doesn't have anywhere else are the ones the stream has with a player.
    for (zone, kept) in &expected.zones {
        if let Zone::Seat(_, SeatZone::Boons) = zone {
            let held = folded.zone(zone);
            assert!(kept.iter().all(|c| held.contains(c)), "{what}: {zone}");
        }
    }
    let mut taken = out_of_decks(&mut folded);
    let kept = out_of_decks(&mut expected);
    for card in kept {
        let i = taken.iter().position(|c| *c == card);
        taken.remove(i.unwrap_or_else(|| panic!("{what}: {card} is kept")));
    }
    let mut all: Vec<&CardRef> = (expected.zones.iter())
        .filter(|(z, _)| matches!(z, Zone::Boons(_) | Zone::Hexes(_)))
        .flat_map(|(_, cards)| cards)
        .collect();
    for card in &taken {
        assert!(!all.contains(&card), "{what}: {card} is in two places");
        all.push(card);
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
        assert_eq!(folded.turn, expected.turn, "{what}");
        assert_eq!(folded.effects, expected.effects, "{what}");
        assert_eq!(folded.face_up, expected.face_up, "{what}");
        assert_eq!(folded.extra_turns, expected.extra_turns, "{what}");
        assert_eq!(folded.ending, expected.ending, "{what}");
        assert_eq!(folded.piles, expected.piles, "{what}");
        assert_eq!(folded.landscapes, expected.landscapes, "{what}");
        assert_eq!(folded.prophecy, expected.prophecy, "{what}");
        assert_eq!(folded.holdings, expected.holdings, "{what}");
        assert_eq!(folded.tokens, expected.tokens, "{what}");
        assert_eq!(folded.notes, expected.notes, "{what}");
        assert_eq!(folded.costs, expected.costs, "{what}");
        assert_eq!(folded.types, expected.types, "{what}");
        assert_eq!(folded.face_down, expected.face_down, "{what}");
        assert_eq!(folded.kept, expected.kept, "{what}");
        assert!(folded == expected, "{what}");
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

/// Plays one game to its end through a recorded [`dominion_agent::Table`], holding the
/// folded stream to the engine's table at every decision. Now and then the table is saved
/// as JSON and play goes on with the copy read back, as a server restarted would: its
/// game must go on as a twin never saved and never journaled does, decision for decision,
/// and the stream it records must be the one a replay records afresh.
fn follow(
    config: GameConfig,
    seed: u64,
    respond: fn(&Game, &mut ChaCha8Rng) -> Response,
) -> Vec<Event> {
    let mut twin = Game::new_traced(config.clone()).unwrap();
    // The same game journaled: what the engine did, to hold the stream's sightings to.
    let mut journaled = Game::new_journaled(config.clone()).unwrap();
    let mut steps = journaled.take_journal();
    let mut table = dominion_agent::Table::new_recorded("g", config, vec![]).unwrap();
    let mut rng = ChaCha8Rng::seed_from_u64(seed);
    let mut events = vec![];
    let mut answers = 0;
    loop {
        let what = format!("seed {seed}, after {answers} answers");
        if let Some(why) = table.recording_stopped() {
            panic!("{what}: recording stopped: {why}");
        }
        let new = table.unwritten().to_vec();
        table.mark_written();
        let recent: Vec<String> = (new.iter())
            .map(|e| serde_json::to_string(e).unwrap())
            .collect();
        let what = format!("{what}, after\n{}\n", recent.join("\n"));
        events.extend(new);
        let (game, recorder) = (table.game(), table.recorder().unwrap());
        assert_eq!(game.log().len(), twin.log().len(), "{what}");
        assert_eq!(game.pending(), twin.pending(), "{what}");
        // Reading every event back at every decision is quadratic: now and then.
        let all = answers % 97 == 0 || game.outcome().is_some();
        check(game, recorder, all.then_some(&events[..]), &what);
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
        if answers % 23 == seed % 23 {
            let json = serde_json::to_string(&table).unwrap_or_else(|e| panic!("{what}: {e}"));
            table = serde_json::from_str(&json).unwrap_or_else(|e| panic!("{what}: {e}"));
        }
        // Now and then a seat resigns, out of turn as often as not.
        if seed % 13 == 7 && answers == 30 + seed % 50 {
            let seat = PlayerId((seed % twin.num_players() as u64) as u8);
            twin.resign(seat);
            journaled.resign(seat);
            steps.extend(journaled.take_journal());
            table.resign(seat).unwrap();
            continue;
        }
        let response = respond(&twin, &mut rng);
        twin.respond(response.clone()).unwrap();
        journaled.respond(response.clone()).unwrap();
        steps.extend(journaled.take_journal());
        let seat = table.deciding().unwrap();
        let id = table.decision_id().unwrap();
        table.respond(seat, &id, response).unwrap();
        answers += 1;
    }
    let game = table.game();
    assert!(
        table.recorded_table().unwrap().ended.is_some(),
        "seed {seed}"
    );
    // A hand revealed where it lies is in the stream, though nothing moves.
    let revealed = (game.log().iter())
        .filter(|e| matches!(e, dominion_engine::Event::RevealedHand { cards, .. } if !cards.is_empty()))
        .count();
    let shown = (events.iter())
        .filter(|e| {
            matches!(&e.what, What::Show(s) if s.to.is_none() && matches!(s.zone, Zone::Seat(_, SeatZone::Hand)))
        })
        .count();
    assert!(
        shown >= revealed,
        "seed {seed}: {revealed} hands revealed, {shown} shown"
    );
    check_causes(&events, &format!("seed {seed}"));
    check_questions(&events, &format!("seed {seed}"));
    check_sightings(&steps, journaled.log(), &events, &format!("seed {seed}"));
    // The stream is the record: played again from it alone, the game writes it again.
    let replayed = dominion_agent::replay::replay(&events)
        .unwrap_or_else(|d| panic!("seed {seed}: the replay differs at {d}"));
    assert_eq!(replayed.game.outcome(), game.outcome(), "seed {seed}");
    assert_eq!(
        replayed.recorder.written(),
        events.len() as u64,
        "seed {seed}"
    );
    events
}

/// Whether the journal's `step` writes no event of its own: a reading of the table (which
/// writes one only when something changed), a marker, a log line no event comes from.
fn quiet(step: &Step, log: &[dominion_engine::Event]) -> bool {
    use dominion_engine::Event as E;
    match step {
        Step::Logged { index } => !matches!(
            log[*index],
            E::Received { .. }
                | E::Bought { .. }
                | E::BoughtLandscape { .. }
                | E::Named { .. }
                | E::UsedWay { .. }
                | E::Unaffected { .. }
                | E::Called { .. }
                | E::Chosen { .. }
        ),
        Step::Gauge { .. }
        | Step::EffectBegan(_)
        | Step::EffectChanged(_)
        | Step::EffectEnded { .. }
        | Step::Token { .. }
        | Step::Holding { .. }
        | Step::Pile { .. }
        | Step::Landscape { .. }
        | Step::Notes(_)
        | Step::Derived { .. }
        | Step::Face { .. }
        | Step::Kept { .. }
        | Step::Occurred(_)
        | Step::FaceUp(_) => true,
        _ => false,
    }
}

/// The events around the `i`th, as JSON Lines.
fn around(events: &[Event], i: usize) -> String {
    (events[i.saturating_sub(6)..(i + 6).min(events.len())].iter())
        .map(|e| serde_json::to_string(e).unwrap())
        .collect::<Vec<_>>()
        .join("\n")
}

fn fate(zone: &Zone) -> bool {
    matches!(
        zone,
        Zone::Boons(_) | Zone::Hexes(_) | Zone::Seat(_, SeatZone::Boons | SeatZone::Hexes)
    )
}

/// How a card moves: from, to, the verb.
type How = (EngineZone, EngineZone, Option<Verb>);

fn moved(step: &Step) -> Option<(CardId, How)> {
    match *step {
        Step::Trace {
            entry: Trace::Moved { card, from, to, .. },
            verb,
        } => Some((card, (from, to, verb))),
        _ => None,
    }
}

fn shown(step: &Step) -> Option<(CardId, Option<u8>)> {
    match *step {
        Step::Trace {
            entry: Trace::Shown { card, to },
            ..
        } => Some((card, to.map(|p| p.0))),
        _ => None,
    }
}

/// Holds the stream's sightings and moves to the engine's journal of the same game: cards
/// the engine moves together are one event; a `show` is written exactly when no move next
/// to it shows its cards to the same eyes (`What::seen`), and once.
fn check_sightings(steps: &[Step], log: &[dominion_engine::Event], events: &[Event], what: &str) {
    // Each card a step moves is the next card of the stream's moves.
    let mut seen = vec![];
    let mut cards = vec![];
    let mut table = Table::default();
    // The opening shuffles and draws are written from where setup dealt the cards, not
    // from moves.
    let dealt = 1
        + (events.iter().skip(1))
            .take_while(|e| e.by.is_none() && matches!(e.what, What::Shuffle(_) | What::Draw(_)))
            .count();
    for (i, e) in events.iter().enumerate() {
        seen.push(e.what.seen(&table));
        if i < dealt {
            table.apply(e).unwrap();
            continue;
        }
        match &e.what {
            What::Play(p) if p.from.is_some() => cards.push(i),
            // Boons and Hexes move by name, not as cards the trace follows.
            w => match w.as_move() {
                Some((_, m)) if !fate(&m.from) && !fate(&m.to) => {
                    cards.extend(vec![i; m.cards.len()])
                }
                _ => {}
            },
        }
        table.apply(e).unwrap();
    }
    let at: Vec<usize> = (0..steps.len())
        .filter(|&j| moved(&steps[j]).is_some())
        .collect();
    assert_eq!(at.len(), cards.len(), "{what}: moves");
    let event_of: BTreeMap<usize, usize> = at.iter().copied().zip(cards.iter().copied()).collect();
    let quiet = |s: &Step| quiet(s, log);

    // Cards trashed or discarded one after the other with nothing between that writes an
    // event (in one frame: Frame::trash and Frame::discard move them all before any of
    // their abilities) are one event.
    for &j in &at {
        let (_, how) = moved(&steps[j]).unwrap();
        if !matches!(how.2, Some(Verb::Trash | Verb::Discard)) {
            continue;
        }
        let k = (j + 1..steps.len()).find(|&k| !quiet(&steps[k]) && shown(&steps[k]).is_none());
        if let Some(k) = k
            && moved(&steps[k]).is_some_and(|(_, h)| h == how)
        {
            assert_eq!(
                event_of[&j],
                event_of[&k],
                "{what}: cards moved together are two events:\n{}",
                around(events, event_of[&j])
            );
        }
    }

    // No sighting says what a move next to it says, nor what the sighting before it said.
    let covered = |s: &events::Show, i: usize, zone: &Zone| {
        let (moved, from_to): (Vec<CardRef>, bool) = match &events[i].what {
            What::Play(p) => (vec![p.card.clone()], p.from.as_ref() == Some(zone)),
            w => match w.as_move() {
                Some((_, m)) => (m.cards.clone(), &m.from == zone || &m.to == zone),
                None => return false,
            },
        };
        let mut left = moved;
        from_to
            && seen[i].is_some_and(|v| v.covers(s.to))
            && s.cards.iter().all(|c| {
                let k = left.iter().position(|m| m == c);
                k.map(|k| left.remove(k)).is_some()
            })
    };
    for i in 0..events.len().saturating_sub(1) {
        let (a, b) = (&events[i].what, &events[i + 1].what);
        if let (What::Show(_) | What::Mix { .. }, What::Show(_) | What::Mix { .. }) = (a, b) {
            assert!(
                a != b,
                "{what}: events {i} and {} are the same sighting:\n{}",
                i + 1,
                around(events, i)
            );
        }
        if let What::Show(s) = a {
            let from = b.as_move().map(|(_, m)| &m.from);
            let play = matches!(b, What::Play(p) if p.from.as_ref() == Some(&s.zone));
            assert!(
                !((from == Some(&s.zone) || play) && covered(s, i + 1, &s.zone)),
                "{what}: the show at {i} says what the move after it says"
            );
        }
        if let What::Show(s) = b
            && let Some((_, m)) = a.as_move()
        {
            assert!(
                !(m.to == s.zone && covered(s, i, &s.zone)),
                "{what}: the show at {} says what the move before it says",
                i + 1
            );
        }
    }

    // And every showing no move beside it discloses has its `show`; every gain and reveal,
    // which the rule says everyone sees, was shown.
    let mut names: BTreeMap<CardId, String> = BTreeMap::new();
    // Where each card lies, and which lie face up, or face down in the trash.
    let mut zones: BTreeMap<CardId, EngineZone> = BTreeMap::new();
    let mut up = std::collections::BTreeSet::new();
    let mut down = std::collections::BTreeSet::new();
    for (j, step) in steps.iter().enumerate() {
        match step {
            Step::Trace {
                entry: Trace::Created { card, is, zone },
                ..
            } => {
                names.insert(*card, is.name().to_string());
                zones.insert(*card, *zone);
            }
            Step::Trace {
                entry: Trace::Moved { card, to, .. },
                ..
            } => {
                zones.insert(*card, *to);
                up.remove(card);
                down.remove(card);
            }
            Step::FaceUp(card) => {
                up.insert(*card);
            }
            Step::Face { card, down: d } => {
                if *d {
                    down.insert(*card);
                } else {
                    down.remove(card);
                }
            }
            Step::Trace {
                entry: Trace::Changed { card, to, .. },
                ..
            } => {
                names.insert(*card, to.name().to_string());
            }
            _ => {}
        }
        if let Some((card, _)) = moved(step)
            && let i = event_of[&j]
            && matches!(events[i].what, What::Gain(_) | What::Reveal(_))
        {
            let before = (0..j).rev().find(|&k| !quiet(&steps[k]));
            assert!(
                before.and_then(|k| shown(&steps[k])) == Some((card, None)),
                "{what}: event {i} shows {} unseen",
                names[&card]
            );
        }
        let Some((card, to)) = shown(step) else {
            continue;
        };
        // Seen where it lies already, by those eyes.
        let in_sight = up.contains(&card)
            || match zones[&card] {
                EngineZone::Trash => !down.contains(&card),
                EngineZone::InPlay(_) => true,
                EngineZone::Mat(p, mat) => mat != Mat::NativeVillage || to == Some(p.0),
                EngineZone::Hand(p) | EngineZone::SetAside(p) | EngineZone::Looking(p) => {
                    to == Some(p.0)
                }
                _ => false,
            };
        if in_sight {
            continue;
        }
        // Its move: the next move of the cards moved with it, or the move that brought it.
        let mut next = None;
        let mut how = None;
        for (k, step) in steps.iter().enumerate().skip(j + 1) {
            if quiet(step) || shown(step).is_some() {
                continue;
            }
            match moved(step) {
                Some((c, h)) if how.is_none_or(|w| w == h) => {
                    how = Some(h);
                    if c == card {
                        next = Some(k);
                        break;
                    }
                }
                _ => break,
            }
        }
        let last = (0..j)
            .rev()
            .find(|&k| !quiet(&steps[k]) && shown(&steps[k]).is_none())
            .filter(|&k| moved(&steps[k]).is_some_and(|(c, _)| c == card));
        let disclosed = [next, last]
            .into_iter()
            .flatten()
            .any(|k| seen[event_of[&k]].is_some_and(|v| v.covers(to)));
        if disclosed {
            continue;
        }
        let after = at
            .iter()
            .find(|&&k| k > j)
            .map_or(events.len(), |k| event_of[k]);
        let before = (at.iter().rev())
            .filter(|&&k| k < j)
            .map(|k| event_of[k])
            .find(|&i| i < after);
        let name = CardRef::named(names[&card].clone());
        let written = (before.map_or(0, |i| i + 1)..after).any(
            |i| matches!(&events[i].what, What::Show(s) if s.to == to && s.cards.contains(&name)),
        );
        assert!(
            written,
            "{what}: {name} shown before event {after} has no show:\n{}\n{:#?}",
            around(events, after),
            &steps[j.saturating_sub(4)..(j + 6).min(steps.len())]
        );
    }
}

/// Every question is answered by the event after it, by its seat, unless the game ends
/// there; nothing else is a question's, and every answer answers one.
fn check_questions(events: &[Event], what: &str) {
    for (i, e) in events.iter().enumerate() {
        let next = events.get(i + 1).map(|e| &e.what);
        match &e.what {
            What::Ask(ask) => {
                let options = ask.options.len() as u32;
                assert!(
                    options > 0 || ask.name_any,
                    "{what}: event {i} offers nothing"
                );
                match next {
                    Some(What::Answer(a)) => {
                        assert_eq!(a.seat, ask.seat, "{what}: event {i}");
                        let picked = a.indices.as_deref().unwrap_or_default();
                        assert!(
                            picked.iter().all(|&o| o < options),
                            "{what}: event {}",
                            i + 1
                        );
                    }
                    Some(What::GameEnd(end)) => assert_eq!(end.reason, "resigned", "{what}"),
                    None => {}
                    Some(_) => panic!("{what}: event {i} is asked and not answered"),
                }
            }
            What::Answer(_) => assert!(
                matches!(
                    i.checked_sub(1).map(|j| &events[j].what),
                    Some(What::Ask(_))
                ),
                "{what}: event {i} answers no question"
            ),
            _ => {}
        }
    }
}

/// Kingdoms around what changes the table outside its zones: Boons and Hexes, tokens,
/// holdings, pile roles, landscapes joining, costs and types.
const FOCUS: &[(&[&str], &[&str])] = &[
    (
        &[
            "Druid",
            "Pixie",
            "Fool",
            "Sacred Grove",
            "Blessed Village",
            "Tracker",
            "Idol",
        ],
        &[],
    ),
    (
        &[
            "Bard",
            "Cursed Village",
            "Werewolf",
            "Skulk",
            "Leprechaun",
            "Druid",
        ],
        &[],
    ),
    (
        &[
            "Ranger",
            "Giant",
            "Bridge Troll",
            "Relic",
            "Raze",
            "Haunted Woods",
        ],
        &["Inheritance", "Pilgrimage", "Lost Arts", "Plan"],
    ),
    (
        &[
            "Flag Bearer",
            "Swashbuckler",
            "Treasurer",
            "Border Guard",
            "Villain",
            "Bridge",
        ],
        &["Sinister Plot", "Capitalism", "Canal", "Citadel"],
    ),
    (
        &[
            "Young Witch",
            "Ferryman",
            "Black Market",
            "Riverboat",
            "Highway",
            "Peddler",
        ],
        &["Obelisk", "Way of the Mouse", "Ferry"],
    ),
    (
        &[
            "Kitsune",
            "Rustic Village",
            "Mountain Shrine",
            "Poet",
            "Tea House",
            "Charlatan",
        ],
        &["Divine Wind"],
    ),
    (
        &[
            "Underling",
            "Sycophant",
            "Importer",
            "Quarry",
            "Destrier",
            "Fisherman",
        ],
        &["Family of Inventors", "Enlightenment"],
    ),
    (
        &[
            "Kitsune",
            "Rustic Village",
            "River Shrine",
            "Wayfarer",
            "Exorcist",
            "Recruiter",
        ],
        &["Enlightenment", "Cheap", "Arena", "Basilica"],
    ),
    (
        &[
            "Hermit",
            "Rats",
            "Marauder",
            "Fortress",
            "Procession",
            "Quartermaster",
            "Grotto",
        ],
        &["Inherited"],
    ),
    (
        &[
            "Smithy",
            "Stash",
            "Cellar",
            "Laboratory",
            "Sentry",
            "Animal Fair",
        ],
        &["Star Chart", "Avoid", "Fated", "Order of Astrologers"],
    ),
    (
        &[
            "Necromancer",
            "Rogue",
            "Cargo Ship",
            "Wharf",
            "Graverobber",
            "Lurker",
        ],
        &["Citadel"],
    ),
];

fn phase_in(e: &What) -> Option<Phase> {
    match e {
        What::Phase { phase } => Some(*phase),
        _ => None,
    }
}

fn by_of<'a>(events: &'a [Event], e: &Event) -> Option<&'a What> {
    e.by.map(|by| &events[by as usize].what)
}

/// Holds what each event says caused it to what the stream shows happened.
fn check_causes(events: &[Event], what: &str) {
    let at = |seq: u64| &events[seq as usize].what;
    let mut turn = None;
    // The `turn-end` of the last turn, once it has ended: between turns.
    let mut ended = None;
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
            && !at(by).is_occurrence()
        {
            fail("caused by something that is no occurrence");
        }
        // A card goes from one player to another only by being passed.
        if let Some((verb, m)) = e.what.as_move() {
            let seat = |z: &Zone| match z {
                Zone::Seat(p, _) => Some(*p),
                _ => None,
            };
            let between = matches!((seat(&m.from), seat(&m.to)), (Some(a), Some(b)) if a != b);
            if between != (verb == events::Verb::Pass) {
                fail("a card passed, or given to another player other than by passing");
            }
        }
        // Questions stand apart: no other event happens inside one. Inside cards trashed
        // or discarded together the only question is which of their abilities is next.
        if matches!(e.what, What::Ask(_) | What::Answer(_)) {
            if let (What::Ask(ask), Some(What::Trash(m) | What::Discard(m))) =
                (&e.what, by_of(events, e))
                && m.cards.len() > 1
                && ask.purpose != "trigger"
            {
                fail("inside several cards' occurrences, a question other than their order");
            }
            table.apply(e).unwrap();
            continue;
        }
        if let What::Turn(_) = e.what {
            turn = Some(seq);
            ended = None;
        }
        // Everything in a turn happens inside it; between turns, only the game's own and
        // what ends with the turn (inside its `turn-end`).
        let mut root = seq;
        let mut through_end = false;
        while let Some(by) = events[root as usize].by {
            root = by;
            through_end |= Some(by) == ended;
        }
        let between = turn.is_none() || ended.is_some();
        match (&e.what, e.by) {
            (
                What::Turn(_)
                | What::TurnSkipped { .. }
                | What::GameEnd(_)
                | What::GameEnding(_)
                | What::Choose(_),
                None,
            ) => {}
            (What::Resolve { when, .. }, None) if turn.is_none() && *when == Moment::GameStart => {}
            (_, None) if between => {}
            (_, None) => fail("nothing caused it"),
            _ if !between && Some(root) != turn => fail("not inside the turn in progress"),
            _ if between && turn.is_some() && !through_end && Some(root) <= ended => {
                fail("between turns, inside a turn that has ended")
            }
            _ => {}
        }
        if let What::TurnEnd { .. } = e.what {
            if !matches!(by_of(events, e), Some(What::Turn(_))) || e.by != turn {
                fail("a turn-end that isn't its turn's");
            }
            ended = Some(seq);
        }
        let by = e.by.map(at);
        let by_phase = by.and_then(phase_in);
        // Each gain is its own occurrence.
        if let What::Gain(m) = &e.what
            && m.cards.len() != 1
        {
            fail("a gain of several cards");
        }
        // Inside cards trashed or discarded together, each its own occurrence, only an
        // ability can say which card it is about.
        if let Some(What::Trash(m) | What::Discard(m)) = by
            && m.cards.len() > 1
            && !matches!(e.what, What::Resolve { .. })
        {
            fail("inside several cards' occurrences, not saying which");
        }
        match &e.what {
            What::Resolve {
                when,
                about,
                target,
                origin,
                index,
                ..
            } => {
                // The card it answers, said by its index when the move has several.
                let holds = |event: Option<&What>, card: &CardRef| {
                    event.and_then(What::as_move).is_some_and(|(_, m)| {
                        match (m.cards.len(), index) {
                            (1, None) => m.cards[0] == *card,
                            (1, Some(_)) | (_, None) => false,
                            (_, Some(i)) => m.cards.get(*i as usize) == Some(card),
                        }
                    })
                };
                // Only cards trashed or discarded together share an event.
                if index.is_some() && !matches!(when, Moment::Trash | Moment::Discard) {
                    fail("an index for a moment that is no move of several cards");
                }
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
                        What::Play(_)
                            | What::Resolve { .. }
                            | What::Buy { .. }
                            | What::Receive { .. }
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
                        if !matches!(next, Some((What::Counter(c), Some(b))) if b == seq && c.counter.name == CounterName::Actions && c.delta == -1)
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
                    let piles = [SeatZone::Deck, SeatZone::Discard].map(|z| Zone::Seat(seat, z));
                    let cards = piles.iter().map(|z| table.zone(z).len()).sum::<usize>();
                    // Not the card itself, played from one of them (City-state's gain).
                    let own = play.from.as_ref().is_some_and(|z| piles.contains(z));
                    if c.def().plus.cards >= 2 && cards > usize::from(own) {
                        drawable.push(seq);
                    }
                }
            }
            What::Buy { .. } if by_phase.is_some() => {
                let next = events.get(seq as usize + 1).map(|e| (&e.what, e.by));
                if !matches!(next, Some((What::Counter(c), Some(b))) if b == seq && matches!(c.counter.name, CounterName::Buys | CounterName::Actions) && c.delta == -1)
                {
                    fail("a buy doesn't spend a Buy");
                }
            }
            What::Draw(_)
            | What::Discard(_)
            | What::Shuffle(_)
            | What::Trash(_)
            | What::Gain(_)
                if by_phase.is_some_and(|p| p != Phase::Cleanup) =>
            {
                fail("moved by a phase other than Clean-up");
            }
            _ if matches!(by, Some(What::Turn(_)))
                && !matches!(
                    e.what,
                    What::Phase { .. }
                        | What::Resolve { .. }
                        | What::TurnEnd { .. }
                        | What::EffectEnd {
                            why: events::EffectEnd::Expired,
                            ..
                        }
                        | What::Derived { .. }
                        | What::Kept { .. }
                ) =>
            {
                fail(
                    "only phases, start-of-turn abilities, effects expiring and the turn's end \
                     happen directly in a turn",
                );
            }
            _ if matches!(by, Some(What::TurnEnd { .. }))
                && !matches!(
                    e.what,
                    What::EffectEnd { .. }
                        | What::ExtraTurnRemoved { .. }
                        | What::Discard(_)
                        | What::Derived { .. }
                        | What::Face { .. }
                        | What::Kept { .. }
                ) =>
            {
                fail("more than effects, extra turns and kept Boons ending with the turn");
            }
            What::Effect(_) | What::ExtraTurn(_)
                if !matches!(
                    by,
                    Some(
                        What::Play(_)
                            | What::Resolve { .. }
                            | What::Buy { .. }
                            | What::Receive { .. }
                    )
                ) =>
            {
                fail("an effect or extra turn that no play, ability, buy or Boon made");
            }
            // Clean-up's draw uses up the -1 Card token.
            What::Token {
                token: events::TokenKind::MinusCard,
                at: None,
                ..
            } if by_phase == Some(Phase::Cleanup) => {}
            // Spending Coffers, a move with no occurrence of its own, uses up the -$1 token.
            What::Token {
                token: events::TokenKind::MinusCoin,
                at: None,
                ..
            } if by_phase.is_some()
                && matches!(
                    events.get(seq as usize - 1).map(|e| (&e.what, e.by)),
                    Some((What::Counter(c), b)) if b == e.by && c.counter.name == CounterName::Coffers
                ) => {}
            What::Take(_) | What::Token { .. } | What::Holding { .. }
                if by_phase.is_some() || matches!(by, Some(What::Turn(_))) =>
            {
                fail("a Boon, Hex, token or holding moved by a phase or turn itself");
            }
            _ => {}
        }
        // A name is priced once a card of it is in the game, and unpriced once none is.
        let priced: Vec<(CardRef, bool)> = match &e.what {
            What::Derived { costs, .. } => (costs.iter())
                .filter(|(card, cost)| table.costs.contains_key(*card) != cost.is_some())
                .map(|(card, cost)| (CardRef::named(card.clone()), cost.is_some()))
                .collect(),
            _ => vec![],
        };
        table.apply(e).unwrap();
        for (card, now) in priced {
            if table.zones.values().any(|cards| cards.contains(&card)) != now {
                fail(&format!(
                    "{card} priced or not before it is in the game or gone"
                ));
            }
        }
    }
    // What changed a play's instructions (a Way, Enchantress) is written before what the
    // play then does: its bonuses, replaced ones too, are the play's.
    for (seq, e) in events.iter().enumerate() {
        let Some(play) = e.by.filter(|&b| matches!(at(b), What::Play(_))) else {
            continue;
        };
        let bonus = match &e.what {
            What::Draw(_) => true,
            What::Counter(c) => {
                matches!(
                    c.counter.name,
                    CounterName::Actions | CounterName::Buys | CounterName::Coins
                ) && c.delta > 0
            }
            _ => false,
        };
        let changed_after = events[seq..].iter().any(|later| {
            later.by == Some(play)
                && matches!(
                    later.what,
                    What::Way { .. }
                        | What::Resolve {
                            when: Moment::Play,
                            ..
                        }
                )
        });
        if bonus && changed_after {
            let json = serde_json::to_string(e).unwrap();
            panic!("{what}: event {seq}, a bonus of play {play} before what changed it\n{json}");
        }
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
        // A seat resigned before the play could draw.
        let cut = matches!(events.last().map(|e| &e.what), Some(What::GameEnd(end)) if end.reason == "resigned")
            && !(events[play as usize..].iter()).any(|e| matches!(e.what, What::Turn(_)));
        if !changed && !drew && !cut {
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

#[test]
fn the_fold_keeps_up_with_boons_tokens_holdings_and_derived_values() {
    let sets = released();
    let card = |name: &&str| Card::from_name(name).unwrap_or_else(|| panic!("{name}"));
    for seed in 0..seeds(40) {
        let (kingdom, landscapes) = FOCUS[seed as usize % FOCUS.len()];
        let mut config = GameConfig::random(2 + (seed % 3) as u8, &sets, seed);
        let focus: Vec<Card> = kingdom.iter().map(card).collect();
        config.kingdom.retain(|c| !focus.contains(c));
        config.kingdom.truncate(10 - focus.len());
        config.kingdom.extend(focus);
        config.landscapes = landscapes.iter().map(card).collect();
        config.turn_limit = Some(60 + 20 * (seed % 4) as u32);
        follow(config, seed, random_response);
    }
}

/// Where a replay of `events` first goes differently, if it does.
fn differs_at(events: &[Event]) -> Option<u64> {
    dominion_agent::replay::replay(events).err().map(|d| d.seq)
}

/// The first event of `events` that doesn't fold.
fn unfolds_at(events: &[Event]) -> Option<u64> {
    let mut table = Table::default();
    events
        .iter()
        .find(|e| table.apply(e).is_err())
        .map(|e| e.seq)
}

/// The table after `events`.
fn folded(events: &[Event]) -> Table {
    let mut table = Table::default();
    for e in events {
        table.apply(e).unwrap();
    }
    table
}

/// A stream changed where it records a cause (an answer, a shuffle's cards) or cut short
/// replays the same up to there and no further.
#[test]
fn a_replay_finds_where_a_stream_was_changed() {
    let sets = released();
    for seed in 0..seeds(20) {
        let mut config = GameConfig::random(2 + (seed % 3) as u8, &sets, seed);
        config.turn_limit = Some(40);
        let events = follow(config, seed, eager_response);
        let what = format!("seed {seed}");
        let pick = |n: usize| (seed as usize * 7919) % n;

        // An answer to a move changed to a different kind of move: the game goes
        // differently right after it.
        let answers: Vec<(usize, u32)> = (0..events.len() - 1)
            .filter_map(|i| match (&events[i].what, &events[i + 1].what) {
                (What::Ask(ask), What::Answer(a)) if ask.kind == "turn" => {
                    let k = a.indices.as_ref()?[0] as usize;
                    let other =
                        (ask.options.iter()).position(|o| o.action != ask.options[k].action)?;
                    Some((i + 1, other as u32))
                }
                _ => None,
            })
            .collect();
        let (i, other) = answers[pick(answers.len())];
        let mut changed = events.clone();
        if let What::Answer(a) = &mut changed[i].what {
            a.indices = Some(vec![other]);
        }
        assert_eq!(
            differs_at(&changed),
            Some(i as u64 + 1),
            "{what}: answer {i}"
        );

        // A shuffle missing a card is refused where it is recorded.
        let shuffles: Vec<usize> = (0..events.len())
            .filter(|&i| matches!(&events[i].what, What::Shuffle(m) if m.cards.len() > 1))
            .collect();
        let s = shuffles[pick(shuffles.len())];
        let mut changed = events.clone();
        if let What::Shuffle(m) = &mut changed[s].what {
            m.cards.pop();
            m.pos.truncate(m.cards.len());
        }
        assert_eq!(differs_at(&changed), Some(s as u64), "{what}: shuffle {s}");

        // Two cards of a shuffle swapped are another outcome the game takes: it goes
        // differently after the shuffle, by the first event that no longer folds (sooner
        // when an answer picks one of those cards by its place: Order of Astrologers).
        let into_deck: Vec<usize> = (shuffles.iter().copied())
            .filter(|&i| match &events[i].what {
                // Not some of a deck's cards shuffled again where they lay.
                What::Shuffle(m) => {
                    matches!(m.to, Zone::Seat(_, SeatZone::Deck))
                        && !matches!(m.at, events::At::Under(_))
                        && m.cards.iter().any(|c| *c != m.cards[0])
                }
                _ => false,
            })
            .collect();
        let s = into_deck[pick(into_deck.len())];
        let mut changed = events.clone();
        let before = folded(&events[..s]);
        if let What::Shuffle(m) = &mut changed[s].what {
            let j = m.cards.iter().position(|c| *c != m.cards[0]).unwrap();
            m.cards.swap(0, j);
            m.pos = taken_from(before.zone(&m.from), &m.cards);
        }
        let unfolds = unfolds_at(&changed);
        assert!(unfolds.is_none_or(|e| e > s as u64), "{what}: shuffle {s}");
        let differs = differs_at(&changed);
        assert!(
            differs.is_none_or(|d| d > s as u64),
            "{what}: shuffle {s} swapped: {differs:?}"
        );
        assert!(
            unfolds.is_none_or(|u| differs.is_some_and(|d| d <= u)),
            "{what}: shuffle {s} swapped: {differs:?}, unfolds at {unfolds:?}"
        );

        // Cut after a question, the stream is a game waiting on that question's seat.
        let asks: Vec<usize> = (0..events.len())
            .filter(|&i| matches!(events[i].what, What::Ask(_)))
            .collect();
        let a = asks[pick(asks.len())];
        let replay = dominion_agent::replay::replay(&events[..=a])
            .unwrap_or_else(|d| panic!("{what}: cut after {a}: {d}"));
        let What::Ask(ask) = &events[a].what else {
            unreachable!()
        };
        assert_eq!(
            replay.game.pending().map(|d| d.player.0),
            Some(ask.seat),
            "{what}"
        );
        // Cut anywhere else, it ends where the game goes on.
        let cuts: Vec<usize> = (1..events.len())
            .filter(|&c| !matches!(events[c - 1].what, What::Ask(_)))
            .collect();
        let cut = cuts[pick(cuts.len())];
        assert_eq!(
            differs_at(&events[..cut]),
            Some(cut as u64),
            "{what}: cut at {cut}"
        );
    }
}

/// How many cards lie over each of `cards` in `zone` as they leave it one after the
/// other, each the topmost of its name; nothing when each is the top card.
fn taken_from(zone: &[CardRef], cards: &[CardRef]) -> Vec<u32> {
    let mut left = zone.to_vec();
    let depths: Vec<u32> = (cards.iter())
        .map(|card| {
            let i = left.iter().position(|c| c == card).unwrap();
            left.remove(i);
            i as u32
        })
        .collect();
    match depths.iter().all(|&d| d == 0) {
        true => vec![],
        false => depths,
    }
}

/// A game played through a `Table`, with its conveniences (Autoplay Treasures, Way
/// questions answered in advance), is recorded as the answers they stand for.
#[test]
fn a_tables_stream_replays() {
    use Card::*;
    use dominion_agent::{Answer, Table, WayChoice};
    for seed in 0..seeds(10) {
        let mut rng = ChaCha8Rng::seed_from_u64(seed);
        let kingdom = vec![
            Village, Smithy, Militia, Moat, ThroneRoom, Market, Laboratory, Festival, Mine,
            Bureaucrat,
        ];
        let mut config = GameConfig::new(2, kingdom, seed);
        config.landscapes = vec![WayOfTheOx, WayOfTheSheep];
        config.turn_limit = Some(30);
        let names = vec!["Ann".to_string(), "Bo".to_string()];
        let mut table = Table::new_recorded("g", config, names).unwrap();
        while !table.is_over() {
            let seat = table.deciding().unwrap();
            let view = table.view(seat);
            let d = view.decision.unwrap();
            if d.kind == "turn"
                && d.phase.as_deref() == Some("buy")
                && rng.random_bool(0.5)
                && table.play_treasures(seat, &d.id).is_ok()
            {
                continue;
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
            let way = match rng.random_range(0..3) {
                0 => Some(WayChoice::Way(WayOfTheOx)),
                1 => Some(WayChoice::None),
                _ => None,
            };
            table.answer_with(seat, &answer, way).unwrap();
        }
        let events = table.unwritten();
        let replay =
            dominion_agent::replay::replay(events).unwrap_or_else(|d| panic!("seed {seed}: {d}"));
        assert_eq!(replay.game.outcome(), table.game().outcome(), "seed {seed}");
    }
}

/// Buys Star Chart and Stashes, and resolves Stash's ability before Star Chart's: a Stash
/// placed among the shuffled cards keeps its place when Star Chart's pick has the others
/// shuffled again.
fn stash_before_star_chart(game: &Game, rng: &mut ChaCha8Rng) -> Response {
    let d = game.pending().expect("a decision");
    match &d.kind {
        DecisionKind::Choose {
            options, labels, ..
        } if d.purpose == dominion_engine::Purpose::Trigger => {
            if let Some(i) = labels.iter().position(|l| l.contains("Stash")) {
                return Response::Options(vec![options[i]]);
            }
        }
        DecisionKind::Turn { moves, .. } => {
            let owns = game.player(d.player).projects.contains(&Card::StarChart);
            let wanted = [
                Move::BuyLandscape(Card::StarChart),
                Move::Buy(game.pile_of(Card::Stash).expect("Stash's pile")),
            ];
            for m in wanted.iter().skip(usize::from(owns)) {
                if moves.contains(m) {
                    return Response::Move(*m);
                }
            }
        }
        _ => {}
    }
    eager_response(game, rng)
}

#[test]
fn a_stash_placed_before_a_reshuffle_keeps_its_place() {
    use Card::*;
    let mut among = 0;
    for seed in 0..seeds(40) {
        let kingdom = vec![
            Stash, Smithy, Moat, Cellar, Village, Market, Laboratory, Festival, Mine, Militia,
        ];
        let mut config = GameConfig::new(2, kingdom, seed);
        config.landscapes = vec![StarChart];
        config.turn_limit = Some(40);
        let events = follow(config, seed, stash_before_star_chart);
        among += (1..events.len())
            .filter(|&i| {
                matches!(&events[i].what, What::Insert(m) if m.cards == [CardRef::named("Stash")])
                    && matches!(&events[i - 1].what, What::Shuffle(m) if m.from == m.to && m.at != events::At::Top)
            })
            .count();
    }
    assert!(among > 0, "no Stash lay among cards shuffled again");
}

/// Buys Donate and trashes every card with it: in Debt with no cards, the player is never
/// asked anything again.
fn donate_everything(game: &Game, rng: &mut ChaCha8Rng) -> Response {
    let d = game.pending().expect("a decision");
    match &d.kind {
        DecisionKind::Turn { moves, .. } if moves.contains(&Move::BuyLandscape(Card::Donate)) => {
            Response::Move(Move::BuyLandscape(Card::Donate))
        }
        DecisionKind::Cards { candidates, .. } if d.source == Some(Card::Donate) => {
            Response::Cards(candidates.clone())
        }
        _ => eager_response(game, rng),
    }
}

/// A game nobody can be asked anything in any more ends `stalled`, an ending like any
/// other: in the stream, the fold and the replay.
#[test]
fn a_stalled_game_ends_in_the_stream() {
    use Card::*;
    let kingdom = vec![
        Village, Smithy, Moat, Cellar, Market, Laboratory, Festival, Mine, Militia, Chapel,
    ];
    let mut config = GameConfig::new(2, kingdom, 1);
    config.landscapes = vec![Donate];
    let events = follow(config, 1, donate_everything);
    let reasons: Vec<&str> = (events.iter())
        .filter_map(|e| match &e.what {
            What::GameEnding(e) => Some(e.reason.as_str()),
            What::GameEnd(e) => Some(e.reason.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(reasons, ["stalled", "stalled"]);
}

/// Replay fails cleanly on a stream no game wrote, reports differences in stream order, and
/// neither needs nor checks the seed.
#[test]
fn a_replay_fails_cleanly_and_in_order() {
    let config = GameConfig::random(2, &released(), 5);
    let events = follow(config, 5, eager_response);
    let ask = (events.iter())
        .position(|e| matches!(e.what, What::Ask(_)))
        .unwrap();

    // A resignation of a seat that doesn't exist.
    let mut bad = events[..=ask].to_vec();
    bad.push(Event {
        seq: ask as u64 + 1,
        by: None,
        what: What::GameEnd(events::GameEnd {
            reason: "resigned".into(),
            result: vec![events::SeatResult {
                seat: 9,
                vp: 0,
                turns: 0,
                rank: 2,
                resigned: true,
            }],
        }),
    });
    assert_eq!(differs_at(&bad), Some(ask as u64 + 1));

    // The seed is provenance: absent or changed, the stream replays.
    for seed in [None, Some(11)] {
        let mut changed = events.clone();
        if let What::Setup(setup) = &mut changed[0].what {
            setup.config.as_mut().unwrap().seed = seed;
        }
        assert_eq!(differs_at(&changed), None, "seed {seed:?}");
    }

    // A malformed outcome late in the stream is no reason to fail before an earlier
    // difference.
    let shuffle = (events.iter())
        .rposition(|e| matches!(&e.what, What::Shuffle(m) if m.cards.len() > 1))
        .unwrap();
    let answer = (events[..shuffle].iter())
        .rposition(|e| matches!(&e.what, What::Answer(_)))
        .unwrap();
    let mut changed = events.clone();
    if let What::Shuffle(m) = &mut changed[shuffle].what {
        m.cards[0] = CardRef::named("No Such Card");
    }
    assert_eq!(differs_at(&changed), Some(shuffle as u64));
    if let What::Answer(a) = &mut changed[answer].what {
        a.indices = Some(vec![u32::MAX]);
    }
    assert_eq!(differs_at(&changed), Some(answer as u64));
}
