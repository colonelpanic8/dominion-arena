//! The events@1 stream over random games: folding it gives the engine's table.

use dominion_agent::events::Recorder;
use dominion_engine::{
    Card, CardId, DecisionKind, Game, GameConfig, Kind, Mat, PileId, PlayerId, Response, Set,
    SetupRole, Token,
};
use dominion_protocol::events::{
    CardRef, CounterKey, CounterName, CounterOf, Event, Pack, Phase, PileRole, PileState, Recipe,
    RecipeGroup, RecipeKind, Role, SeatZone, Table, TokenAt, TokenKind, TokenState, Zone,
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
    let notes = game.watch().notes();
    let named = |cards: &mut dyn Iterator<Item = &Card>| {
        cards
            .map(|c| CardRef::Named(c.name().to_string()))
            .collect::<Vec<_>>()
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
            zones.insert(
                Zone::Seat(i as u8, SeatZone::Boons),
                named(&mut kept.iter()),
            );
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
    ] {
        counters.insert(CounterKey::global(name), i64::from(value));
    }
    for n in &notes {
        if n.source.kind() == Kind::Landmark && n.pile.is_none() && n.player.is_none() {
            let key = CounterOf::Landscape(n.source.name().to_string());
            counters.insert(CounterKey::new(key, CounterName::Vp), i64::from(n.count));
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
    let mut cards: Vec<Card> = (0..game.card_count())
        .map(|i| game.printed(CardId(i as u32)))
        .collect();
    cards.sort();
    cards.dedup();
    let names_of = |types: dominion_engine::Types| types.names().map(str::to_string).collect();
    Table {
        zones,
        counters,
        turn: None,
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
    let turn = folded.turn.take();
    folded.ended = None;
    let mut expected = engine_table(game);
    if turn.is_none() {
        // Before the first turn the engine's turn is a blank one.
        for name in [
            CounterName::Actions,
            CounterName::Buys,
            CounterName::Coins,
            CounterName::Potions,
        ] {
            expected.counters.remove(&CounterKey::global(name));
        }
    }
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
];

#[test]
fn the_fold_of_the_full_stream_is_the_engines_table() {
    let sets = released();
    for seed in 0..seeds(40) {
        let players = 2 + (seed % 3) as u8;
        let mut config = GameConfig::random(players, &sets, seed);
        config.turn_limit = Some(40 + 20 * (seed % 5) as u32);
        play_folding(config, seed);
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
        play_folding(config, seed);
    }
}

/// Plays a game with random answers, holding the folded stream to the engine's table at
/// every decision.
fn play_folding(config: GameConfig, seed: u64) {
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
}
