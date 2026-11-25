//! Dark Ages, Cornucopia & Guilds, Adventures and Nocturne: every card admitted from these
//! sets is dealt into every starting deck and played through seeded games. Each decision
//! is projected for each seat; every wire answer offered must resolve to a response the
//! engine accepts, and every story, mode, ability and set-specific button an admitted
//! card's question shows must be one its `symbols` declares.
use super::*;
use crate::official_host::questions::{PreparedQuestion, semantics};
use dominion_engine::{
    Decision, DecisionKind, Game, GameConfig, KnownCards, Move, Opt, Purpose, Response, Set,
};
use dominion_online::questions::{LogArgument, Question, QuestionElements, QuestionKind};

fn catalog() -> &'static WireCatalog {
    static C: std::sync::OnceLock<WireCatalog> = std::sync::OnceLock::new();
    C.get_or_init(super::tests::catalog)
}

pub(super) fn update(game: &Game, seat: u8, index: i32) -> Result<ProjectedUpdate, String> {
    let n = game.num_players();
    let names: Vec<_> = (0..n).map(|i| format!("Seat{}", i + 1)).collect();
    let context = HostGameContext {
        wire_ids: WireIdentity {
            game_id: 100,
            table_id: 31,
            player_ids: (0..n).map(|i| 1000 + (i as i32) * 13).collect(),
        },
        table_details: transport::TableDetails {
            table_id: 31,
            host_id: 1000,
            players: (0..n)
                .map(|i| transport::NamedId {
                    id: 1000 + (i as i32) * 13,
                    name: names[i].clone(),
                })
                .collect(),
            ready: vec![true; n],
            bots: vec![],
            spectators: vec![],
            reconnecting: vec![],
            rules: vec![],
            division: String::new(),
            match_games: vec![],
            match_score: None,
            tournament: None,
        },
        viewer_role: ViewerRole::Player(usize::from(seat)),
        table_options: dominion_protocol::lobby::TableSpec {
            timed: false,
            rated: false,
            ..Default::default()
        },
    };
    project(
        hub::SeatObservation {
            clock: None,
            view: game.view(PlayerId(seat)),
            known: &KnownCards::default(),
            names: &names,
            decision_id: game.pending().map(|_| "q-darkages"),
            deciding: game.pending().map(|d| d.player),
            version: index as u64,
            undos: 0,
            outcome: game.outcome(),
            ended: None,
        },
        &context,
        catalog(),
        ProjectionRequest {
            question_index: index,
            reconnect: false,
            epoch: 19,
        },
    )
    .map_err(|e| e.to_string())
}

/// Every individual option, the boundary selections and one reversed order.
fn answers(q: &Question) -> Vec<Vec<i32>> {
    match &q.kind {
        QuestionKind::Complex(children) => children
            .iter()
            .enumerate()
            .flat_map(|(i, q)| {
                answers(q)
                    .into_iter()
                    .map(move |a| std::iter::once(i as i32).chain(a).collect())
            })
            .collect(),
        QuestionKind::Choice {
            min,
            max,
            elements,
            decline,
            ..
        } => {
            let len = elements.len() as i32;
            let mut a = vec![];
            if *min == 0 {
                a.push(vec![]);
            }
            if *min <= 1 && *max >= 1 {
                a.extend((0..len).map(|i| vec![i]));
            }
            if *min > 1 {
                a.push((0..*min).collect());
            }
            if *max > 1 {
                a.push((0..*max.min(&len)).rev().collect());
            }
            let _ = decline;
            a
        }
        QuestionKind::Name => vec![vec![2], vec![31]],
        QuestionKind::Number { min, max, .. } => vec![vec![*min], vec![*max]],
        _ => panic!("unexpected question shape"),
    }
}

struct Rng(u64);
impl Rng {
    fn below(&mut self, n: usize) -> usize {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        ((self.0 >> 33) as usize) % n.max(1)
    }
}

/// The symbols of `q` that only a card's own binding chooses.
fn shown(q: &Question, out: &mut Vec<(&'static str, i32)>) {
    out.push(("QuestionIds", q.description.story.question_id));
    match &q.kind {
        QuestionKind::Complex(children) => children.iter().for_each(|q| shown(q, out)),
        QuestionKind::Choice {
            elements, decline, ..
        } => {
            if *decline >= 0 {
                out.push(("DeclineButtonIds", *decline));
            }
            match elements {
                QuestionElements::Modes(modes) => {
                    out.extend(modes.iter().map(|&m| ("CardModes", m)))
                }
                QuestionElements::Abilities(abilities) => {
                    out.extend(abilities.iter().map(|a| ("AbilityNames", a.name)))
                }
                _ => {}
            }
        }
        _ => {}
    }
}

const GENERIC: &[(&str, &str)] = &[
    ("QuestionIds", "GAME_RESOLVE_ABILITY"),
    ("QuestionIds", "GAME_MAY_REACT_WITH"),
    ("DeclineButtonIds", "DONE_REACTING"),
    ("DeclineButtonIds", "DONT_TRASH"),
    ("DeclineButtonIds", "DONT_DISCARD"),
    ("DeclineButtonIds", "DONT_GAIN"),
    ("DeclineButtonIds", "DONT_PLAY"),
    ("DeclineButtonIds", "DONT_TOPDECK"),
    ("DeclineButtonIds", "DONT_REVEAL"),
    ("DeclineButtonIds", "DONE_SETTING_ASIDE"),
    ("DeclineButtonIds", "DONT"),
];

/// Checks a projected question against the symbols its card declares.
fn declared(game: &Game, d: &Decision, q: &Question, admitted: &[Card]) {
    let c = catalog();
    let mut owners = vec![];
    if d.purpose == Purpose::Trigger {
        owners.extend(
            game.view(d.player)
                .pending_triggers()
                .iter()
                .map(|t| t.card),
        );
    } else if let Some(source) = d.source {
        owners.push(source);
    }
    if !owners.iter().any(|o| admitted.contains(o)) {
        return;
    }
    let allowed: Vec<(&str, i32)> = owners
        .iter()
        .flat_map(|&o| semantics::symbols(o).iter())
        .chain(GENERIC)
        .filter_map(|&(t, s)| c.symbol(t, s).ok().map(|i| (t, i)))
        .collect();
    let mut seen = vec![];
    shown(q, &mut seen);
    for (table, symbol) in seen {
        if d.purpose == Purpose::Trigger && table != "AbilityNames" {
            continue;
        }
        if d.purpose == Purpose::Trigger
            && !owners.iter().any(|&o| {
                semantics::symbols(o)
                    .iter()
                    .any(|(t, _)| *t == "AbilityNames")
            })
        {
            continue;
        }
        assert!(
            allowed.contains(&(table, symbol)),
            "{owners:?} {:?}: {table} {symbol} is not declared",
            d.purpose
        );
    }
}

/// Swaps some of each player's starting Coppers for `cards`.
fn deal(game: Game, cards: &[Card]) -> Game {
    let mut state = serde_json::to_value(&game).unwrap();
    let copper = serde_json::to_value(Card::Copper).unwrap();
    for p in 0..game.num_players() {
        let mut changed = 0;
        for zone in ["hand", "deck"] {
            let ids: Vec<usize> = state["players"][p][zone]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_u64().unwrap() as usize)
                .collect();
            for id in ids {
                if changed < 4 && state["cards"][id] == copper {
                    let card = cards[changed % cards.len()];
                    state["cards"][id] = serde_json::to_value(card).unwrap();
                    changed += 1;
                }
            }
        }
    }
    serde_json::from_value(state).unwrap()
}

const FILLER: &[Card] = &[
    Card::Village,
    Card::Smithy,
    Card::Market,
    Card::Cellar,
    Card::Militia,
    Card::Moat,
    Card::Laboratory,
    Card::Festival,
    Card::Workshop,
    Card::Remodel,
];

/// The Kingdom pile a non-Kingdom card comes from in these sets' games.
fn kingdom_of(card: Card) -> Option<Card> {
    use Card::*;
    if card.def().kingdom {
        return Some(card);
    }
    Some(match card.def().pile {
        Some(Knights) => Knights,
        Some(Ruins) => Marauder,
        _ => match card {
            Ruins => Marauder,
            Spoils => BanditCamp,
            Madman => Hermit,
            Mercenary => Urchin,
            BagOfGold | Diadem | Followers | Princess | TrustySteed => Tournament,
            TreasureHunter | Warrior | Hero | Champion => Page,
            Soldier | Fugitive | Disciple | Teacher => Peasant,
            WillOWisp => Bard,
            Imp => DevilsWorkshop,
            Ghost => Exorcist,
            Wish => Leprechaun,
            Bat => Vampire,
            ZombieApprentice | ZombieMason | ZombieSpy => Necromancer,
            TheSwampsGift | TheFieldsGift | TheForestsGift | TheMountainsGift | TheRiversGift
            | TheSeasGift | TheEarthsGift | TheFlamesGift | TheMoonsGift | TheSkysGift
            | TheSunsGift | TheWindsGift => Bard,
            BadOmens | Delusion | Envy | Famine | Fear | Greed | Haunting | Locusts | Misery
            | Plague | Poverty | War | Deluded | Envious | Miserable | TwiceMiserable => Werewolf,
            LostInTheWoods => Fool,
            _ if Card::ALL.iter().any(|k| k.related().heirloom == Some(card)) => *Card::ALL
                .iter()
                .find(|k| k.related().heirloom == Some(card))
                .unwrap(),
            Coronet | Courser | Demesne | Housecarl | HugeTurnip | Renown => Joust,
            _ => return None,
        },
    })
}

/// Plays seeded games with `card` in every starting deck.
fn exercise(card: Card, admitted: &[Card], seeds: u64, steps: i32) {
    let landscape = card.kind().is_landscape();
    let home = match kingdom_of(card) {
        Some(home) => home,
        None if landscape => Card::Village,
        None => panic!("{card} has no game to be played in"),
    };
    for seed in 0..seeds {
        let mut kingdom: Vec<Card> = FILLER.iter().copied().filter(|&c| c != home).collect();
        kingdom.truncate(9);
        kingdom.push(home);
        let mut cfg = GameConfig::new(2, kingdom, 500 + seed);
        cfg.colonies = Some(false);
        cfg.shelters = Some(false);
        if landscape {
            cfg.landscapes.push(card);
        }
        let game = Game::new(cfg).unwrap();
        // A landscape is bought: Golds pay for it.
        let mut game = if card.kind() == dominion_engine::Kind::Card {
            deal(game, &[card])
        } else if home.kind() == dominion_engine::Kind::Card && !landscape {
            deal(game, &[home])
        } else if landscape {
            deal(game, &[Card::Gold])
        } else {
            game
        };
        let mut rng = Rng(seed * 7919 + 17);
        for step in 0..steps {
            let Some(d) = game.pending().cloned() else {
                break;
            };
            let view = game.view(d.player);
            if view
                .players()
                .any(|p| p.hand_size() + p.deck_size() + p.discard_size() + p.in_play().len() < 5)
            {
                break;
            }
            for seat in 0..game.num_players() as u8 {
                if seat != d.player.0 {
                    update(&game, seat, step + 1)
                        .unwrap_or_else(|e| panic!("{card}: seat {seat} at {d:?}: {e}"));
                }
            }
            let u = update(&game, d.player.0, step + 1)
                .unwrap_or_else(|e| panic!("{card}: {d:?}: {e}"));
            let q = u.question.expect("decider has a question");
            if !matches!(d.kind, DecisionKind::Turn { .. }) {
                declared(&game, &d, &q.wire, admitted);
            }
            let mut offered = vec![];
            for a in answers(&q.wire) {
                let response = q
                    .resolve_at(19, q.wire.index, &a)
                    .unwrap_or_else(|e| panic!("{card}: {d:?} {a:?}: {e:?}"));
                d.validate(&response)
                    .unwrap_or_else(|e| panic!("{card}: {d:?} {a:?} {response:?}: {e}"));
                offered.push(response);
            }
            let response = match &d.kind {
                DecisionKind::Turn { moves, .. } => {
                    let mine: Vec<_> = moves
                        .iter()
                        .filter(|m| match m {
                            Move::Play(id) => game.card(*id) == card,
                            Move::BuyLandscape(l) => *l == card,
                            _ => false,
                        })
                        .collect();
                    if !mine.is_empty() && rng.below(3) != 0 {
                        Response::Move(*mine[rng.below(mine.len())])
                    } else {
                        offered.swap_remove(rng.below(offered.len()))
                    }
                }
                DecisionKind::Choose { options, .. }
                    if d.purpose == Purpose::Trigger && options.contains(&Opt::No) =>
                {
                    let picked = offered.swap_remove(rng.below(offered.len()));
                    if rng.below(3) == 0 {
                        Response::option(Opt::No)
                    } else {
                        picked
                    }
                }
                _ => offered.swap_remove(rng.below(offered.len())),
            };
            game.respond(response).unwrap();
        }
    }
}

/// The cards of `sets` the transport admits.
fn admitted(sets: &[Set]) -> Vec<Card> {
    let report = crate::official_host::profile::coverage_report(catalog());
    Card::ALL
        .iter()
        .copied()
        .filter(|c| sets.contains(&c.set()) && !c.def().unimplemented)
        .filter(|c| {
            report
                .cards
                .iter()
                .any(|r| r.card == c.name() && r.semantic_mapped)
        })
        .collect()
}

fn exercise_sets(sets: &[Set]) {
    let admitted = admitted(sets);
    assert!(!admitted.is_empty());
    let report = crate::official_host::profile::coverage_report(catalog());
    for &card in &admitted {
        let r = report.cards.iter().find(|r| r.card == card.name()).unwrap();
        assert!(
            r.gaps.is_empty() || !r.catalog_present,
            "{card}: {:?}",
            r.gaps
        );
        if kingdom_of(card).is_some() || card.kind().is_landscape() {
            exercise(card, &admitted, 3, 90);
        }
    }
}

#[test]
fn dark_ages_admitted_cards_project_and_answer_through_seeded_games() {
    exercise_sets(&[Set::DarkAges]);
}

#[test]
fn cornucopia_guilds_admitted_cards_project_and_answer_through_seeded_games() {
    exercise_sets(&[Set::CornucopiaGuilds]);
}

#[test]
fn adventures_admitted_cards_project_and_answer_through_seeded_games() {
    exercise_sets(&[Set::Adventures]);
}

#[test]
fn nocturne_admitted_cards_project_and_answer_through_seeded_games() {
    exercise_sets(&[Set::Nocturne]);
}

/// A two-player game at P0's Action phase: `hand` (its first card to play) and the top of
/// each deck, top first, are the given cards.
fn table(kingdom: &[Card], hand: &[Card], tops: [&[Card]; 2]) -> Game {
    let mut cards = kingdom.to_vec();
    cards.extend(FILLER.iter().filter(|c| !kingdom.contains(c)));
    cards.truncate(10);
    let mut cfg = GameConfig::new(2, cards, 91);
    cfg.colonies = Some(false);
    cfg.shelters = Some(false);
    let game = Game::new(cfg).unwrap();
    let mut state = serde_json::to_value(&game).unwrap();
    let ids = |state: &serde_json::Value, p: usize, zone: &str| -> Vec<usize> {
        state["players"][p][zone]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_u64().unwrap() as usize)
            .collect()
    };
    let in_hand = ids(&state, 0, "hand");
    for (&id, &card) in in_hand.iter().zip(hand) {
        state["cards"][id] = serde_json::to_value(card).unwrap();
    }
    for (p, top) in tops.iter().enumerate() {
        let deck = ids(&state, p, "deck");
        for (&id, &card) in deck.iter().rev().zip(top.iter()) {
            state["cards"][id] = serde_json::to_value(card).unwrap();
        }
    }
    state["turn"]["phase"] = serde_json::json!("Action");
    state["turn"]["actions"] = serde_json::json!(1);
    state["pending"] = serde_json::to_value(Decision {
        player: PlayerId(0),
        kind: DecisionKind::Turn {
            phase: dominion_engine::Phase::Action,
            moves: vec![
                Move::Play(serde_json::from_value(in_hand[0].into()).unwrap()),
                Move::EndPhase,
            ],
        },
        purpose: Purpose::Turn,
        public_choice: false,
        source: None,
        about: vec![],
        question: None,
        plays: vec![],
    })
    .unwrap();
    for frame in state["stack"].as_array_mut().unwrap() {
        if let Some(turn) = frame.get_mut("Turn") {
            turn["pc"] = serde_json::json!(2);
        }
    }
    serde_json::from_value(state).unwrap()
}

fn sym(table: &str, name: &str) -> i32 {
    catalog().symbol(table, name).unwrap()
}

/// The deciding seat's question.
fn asked(game: &Game) -> PreparedQuestion {
    let d = game.pending().unwrap();
    update(game, d.player.0, 7).unwrap().question.unwrap()
}

/// Answers the pending question through the wire.
fn answer(game: &mut Game, values: &[i32]) {
    let q = asked(game);
    let response = q.resolve(q.wire.index, values).unwrap();
    game.respond(response).unwrap();
}

fn story(q: &PreparedQuestion) -> i32 {
    q.wire.description.story.question_id
}

fn modes(q: &PreparedQuestion) -> Vec<i32> {
    match &q.wire.kind {
        QuestionKind::Choice {
            elements: QuestionElements::Modes(m),
            ..
        } => m.clone(),
        other => panic!("not modes: {other:?}"),
    }
}

/// Answers with the element whose response picks a card of `card`.
fn pick(game: &mut Game, card: Card) {
    let q = asked(game);
    let QuestionKind::Choice { elements, .. } = &q.wire.kind else {
        panic!("not a choice");
    };
    let i = (0..elements.len() as i32)
        .find(|&i| match q.resolve(q.wire.index, &[i]) {
            Ok(Response::Cards(c)) => game.card(c[0]) == card,
            Ok(Response::Pile(Some(p))) => game.view(PlayerId(0)).pile(p).card() == card,
            _ => false,
        })
        .unwrap_or_else(|| panic!("no {card} offered"));
    answer(game, &[i]);
}

#[test]
fn count_storeroom_and_remake_name_each_of_their_questions() {
    use Card::*;
    let mut g = table(
        &[Count],
        &[Count, Copper, Estate, Silver, Estate],
        [&[], &[]],
    );
    super::tests::play(&mut g, Count);
    let q = asked(&g);
    assert_eq!(story(&q), sym("QuestionIds", "COUNT_PAY"));
    assert_eq!(
        modes(&q),
        ["COUNT_DISCARD", "COUNT_TOPDECK", "COUNT_GAIN_COPPER"].map(|m| sym("CardModes", m))
    );
    answer(&mut g, &[2]);
    let q = asked(&g);
    assert_eq!(story(&q), sym("QuestionIds", "COUNT_PROFIT"));
    assert_eq!(
        modes(&q),
        ["COUNT_COINS", "COUNT_TRASH_HAND", "COUNT_GAIN_DUCHY"].map(|m| sym("CardModes", m))
    );

    let mut g = table(&[Storeroom], &[Storeroom, Copper, Estate], [&[], &[]]);
    super::tests::play(&mut g, Storeroom);
    assert_eq!(story(&asked(&g)), sym("QuestionIds", "STOREROOM_CARDS"));
    answer(&mut g, &[]);
    assert_eq!(story(&asked(&g)), sym("QuestionIds", "STOREROOM_COINS"));

    let mut g = table(
        &[Remake],
        &[Remake, Estate, Estate, Copper, Copper],
        [&[], &[]],
    );
    super::tests::play(&mut g, Remake);
    assert_eq!(story(&asked(&g)), sym("QuestionIds", "REMAKE_TRASH_FIRST"));
    pick(&mut g, Estate);
    let q = asked(&g);
    assert_eq!(story(&q), sym("QuestionIds", "REMAKE_GAIN_FIRST"));
    assert_eq!(
        q.wire.description.story.arguments,
        [LogArgument::Amount {
            coins: 3,
            potions: 0,
            debt: 0
        }]
    );
    pick(&mut g, Silver);
    assert_eq!(story(&asked(&g)), sym("QuestionIds", "REMAKE_TRASH_SECOND"));
    pick(&mut g, Estate);
    assert_eq!(story(&asked(&g)), sym("QuestionIds", "REMAKE_GAIN_SECOND"));
}

#[test]
fn mercenary_buttons_and_jester_players_answer_the_engine_options() {
    use Card::*;
    let mut g = table(
        &[Urchin],
        &[Mercenary, Copper, Copper, Estate, Estate],
        [&[], &[]],
    );
    super::tests::play(&mut g, Mercenary);
    let q = asked(&g);
    assert_eq!(story(&q), sym("QuestionIds", "MERCENARY_MULTIPLE"));
    let QuestionKind::Complex(children) = &q.wire.kind else {
        panic!("Mercenary asks with buttons");
    };
    let buttons: Vec<i32> = children
        .iter()
        .map(|c| match c.kind {
            QuestionKind::Choice { decline, .. } => decline,
            _ => panic!("button"),
        })
        .collect();
    assert_eq!(
        buttons,
        ["CONTINUE", "DONT_TRASH"].map(|b| sym("DeclineButtonIds", b))
    );
    assert_eq!(
        q.resolve(q.wire.index, &[0]),
        Ok(Response::option(Opt::Yes))
    );
    assert_eq!(q.resolve(q.wire.index, &[1]), Ok(Response::option(Opt::No)));

    let mut g = table(&[Jester], &[Jester, Copper], [&[], &[Silver]]);
    super::tests::play(&mut g, Jester);
    let q = asked(&g);
    assert_eq!(story(&q), sym("QuestionIds", "JESTER"));
    assert_eq!(
        modes(&q),
        ["JESTER_THEY_GAIN", "JESTER_YOU_GAIN"].map(|m| sym("CardModes", m))
    );
    assert_eq!(
        q.wire.description.story.arguments,
        [
            LogArgument::CardNames(vec![dominion_online::questions::CardFrequency {
                name: catalog().card(Silver).unwrap(),
                frequency: 1,
            }]),
            LogArgument::Player(1),
        ]
    );
    assert_eq!(
        q.resolve(q.wire.index, &[1]),
        Ok(Response::option(Opt::Player(PlayerId(0))))
    );
}

#[test]
fn overpay_reserve_calls_and_boons_by_name() {
    use Card::*;
    // Overpaying is an amount, counted up from none.
    let mut g = table(
        &[Masterpiece],
        &[Gold, Gold, Gold, Copper, Copper],
        [&[], &[]],
    );
    g.respond(Response::Move(Move::EndPhase)).unwrap();
    while let Some(Decision {
        kind: DecisionKind::Turn { moves, .. },
        ..
    }) = g.pending().cloned()
    {
        let next = moves
            .iter()
            .find(|m| matches!(m, Move::Play(_)))
            .or_else(|| {
                moves.iter().find(|m| {
                    matches!(m, Move::Buy(p) if g.view(PlayerId(0)).pile(*p).card() == Masterpiece)
                })
            })
            .copied()
            .unwrap();
        g.respond(Response::Move(next)).unwrap();
    }
    let q = asked(&g);
    assert_eq!(story(&q), sym("QuestionIds", "MASTERPIECE"));
    assert_eq!(
        q.wire.description.question_type,
        sym("QuestionTypes", "OVERPAY")
    );
    let QuestionKind::Number {
        min, max, default, ..
    } = q.wire.kind
    else {
        panic!("overpay is a number");
    };
    assert_eq!((min, max, default), (0, 8, 0));
    assert_eq!(q.resolve(q.wire.index, &[2]), Ok(Response::Number(2)));

    // A Reserve's call names the card it is about.
    let mut g = table(
        &[Duplicate],
        &[Duplicate, Silver, Silver, Copper],
        [&[], &[]],
    );
    super::tests::play(&mut g, Duplicate);
    g.respond(Response::Move(Move::EndPhase)).unwrap();
    while let Some(Decision {
        kind: DecisionKind::Turn { moves, .. },
        ..
    }) = g.pending().cloned()
    {
        let next = moves
            .iter()
            .find(|m| matches!(m, Move::Play(_)))
            .or_else(|| {
                moves.iter().find(
                    |m| matches!(m, Move::Buy(p) if g.view(PlayerId(0)).pile(*p).card() == Village),
                )
            })
            .copied()
            .unwrap();
        g.respond(Response::Move(next)).unwrap();
    }
    let q = asked(&g);
    let QuestionKind::Choice {
        elements: QuestionElements::Abilities(abilities),
        ..
    } = &q.wire.kind
    else {
        panic!("a trigger window");
    };
    assert_eq!(abilities[0].name, sym("AbilityNames", "CALL_DUPLICATE"));
    assert_eq!(abilities[0].description_type, 2);
    assert_eq!(
        abilities[0].arguments,
        [LogArgument::CardNames(vec![
            dominion_online::questions::CardFrequency {
                name: catalog().card(Village).unwrap(),
                frequency: 1,
            }
        ])]
    );

    // Druid's Boons have no card on the table: they are offered by name.
    let mut g = table(&[Druid], &[Druid, Copper], [&[], &[]]);
    super::tests::play(&mut g, Druid);
    let d = g.pending().unwrap().clone();
    let DecisionKind::Choose { options, .. } = &d.kind else {
        panic!("Druid chooses a Boon");
    };
    let q = asked(&g);
    assert_eq!(story(&q), sym("QuestionIds", "DRUID"));
    let QuestionKind::Choice {
        elements: QuestionElements::Names(names),
        ..
    } = &q.wire.kind
    else {
        panic!("Boons by name");
    };
    let boons: Vec<i32> = options
        .iter()
        .map(|o| match o {
            Opt::Card(b) => catalog().card(*b).unwrap(),
            _ => panic!("a Boon"),
        })
        .collect();
    assert_eq!(names, &boons);
    assert_eq!(
        q.resolve(q.wire.index, &[1]),
        Ok(Response::option(options[1]))
    );
}
