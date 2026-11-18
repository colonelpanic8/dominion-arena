//! The events@1 stream over random games: folding it gives the engine's table.

use dominion_agent::events::Recorder;
use dominion_engine::{
    Card, CardId, DecisionKind, EndReason, ExtraLimit, Game, GameConfig, Kind, Mat, Move,
    Phase as EnginePhase, PileId, PlayerId, Response, Set, SetupRole, Token, Until,
    Zone as EngineZone,
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
    let aside = |c: &CardId| matches!(game.zone(*c), EngineZone::SetAside(_));
    let up: Vec<CardId> = (0..game.num_players())
        .flat_map(|i| game.player(PlayerId(i as u8)).face_up.iter().copied())
        .collect();
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
                keeps: bag(&l.keeps),
                face_up: bag(&(holds.iter().copied())
                    .filter(|c| up.contains(c))
                    .collect::<Vec<_>>()),
                holds: bag(&holds),
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
        assert!(folded == expected, "{what}");
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
    check_causes(&events, &format!("seed {seed}"));
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
];

/// Whether `e` is an occurrence: something other events can happen inside.
fn occurrence(e: &What) -> bool {
    matches!(
        e,
        What::Turn(_)
            | What::Phase { .. }
            | What::TurnEnd { .. }
            | What::Receive { .. }
            | What::Play(_)
            | What::Buy { .. }
            | What::Resolve { .. }
            | What::Gain(_)
            | What::Trash(_)
            | What::Discard(_)
            | What::Shuffle(_)
    )
}

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
            && !occurrence(at(by))
        {
            fail("caused by something that is no occurrence");
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
            What::Draw(_) | What::Discard(_) | What::Shuffle(_)
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
            What::Take(_) | What::Token { .. } | What::Holding { .. }
                if by_phase.is_some() || matches!(by, Some(What::Turn(_))) =>
            {
                fail("a Boon, Hex, token or holding moved by a phase or turn itself");
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
