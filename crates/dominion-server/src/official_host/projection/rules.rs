//! Deterministic effect witnesses, separate from the seeded whole-game tests.
use super::tests::play;
use super::*;
fn catalog() -> &'static WireCatalog {
    static C: std::sync::OnceLock<WireCatalog> = std::sync::OnceLock::new();
    C.get_or_init(super::tests::catalog)
}
use dominion_engine::{
    Decision, DecisionKind, Game, GameConfig, KnownCards, Move, Opt, Phase, Purpose, Response, Set,
    Zone,
};
use dominion_online::{
    questions::{Question, QuestionKind},
    server::ServerEncoder,
    transport::{LoginSuccess, Message},
};
use serde_json::{Value, json};

fn fixture(card: Card, players: u8) -> Game {
    fixture_with_colonies(card, players, false)
}
fn fixture_with_colonies(card: Card, players: u8, colonies: bool) -> Game {
    let mut kingdom = vec![
        Card::Village,
        Card::Smithy,
        Card::Moat,
        Card::Bazaar,
        Card::Haven,
        Card::Caravan,
        Card::Lighthouse,
        Card::NativeVillage,
        Card::Island,
        Card::ThroneRoom,
        Card::Bridge,
    ];
    if card.def().kingdom {
        kingdom.push(card);
    }
    kingdom.sort();
    kingdom.dedup();
    let mut cfg = GameConfig::new(players, kingdom, 73);
    cfg.colonies = Some(colonies);
    cfg.shelters = Some(false);
    if card.kind().is_landscape() {
        cfg.landscapes.push(card);
    }
    let game = Game::new(cfg).unwrap();
    let mut state = serde_json::to_value(&game).unwrap();
    for seat in 0..players {
        let old = std::mem::take(
            state["players"][usize::from(seat)]["hand"]
                .as_array_mut()
                .unwrap(),
        );
        for id in old {
            state["zones"][id.as_u64().unwrap() as usize] = json!({"Deck":seat});
            state["players"][usize::from(seat)]["deck"]
                .as_array_mut()
                .unwrap()
                .push(id);
        }
        let hand = if seat == 0 {
            vec![
                if card.def().kingdom {
                    card
                } else {
                    Card::Village
                },
                Card::Copper,
                Card::Silver,
                Card::Estate,
                Card::Village,
                Card::Smithy,
            ]
        } else {
            vec![
                Card::Moat,
                Card::Copper,
                Card::Copper,
                Card::Estate,
                Card::Estate,
                Card::Silver,
            ]
        };
        for c in hand {
            take(&mut state, &game, c, Zone::Hand(PlayerId(seat)));
        }
        // Distinct, known deck-top effects after one/two vanilla draws.
        for c in [
            Card::Village,
            Card::Silver,
            Card::Estate,
            Card::Smithy,
            Card::Copper,
            Card::Gold,
        ] {
            take(&mut state, &game, c, Zone::Deck(PlayerId(seat)));
        }
        take(
            &mut state,
            &game,
            Card::Copper,
            Zone::Discard(PlayerId(seat)),
        );
        take(
            &mut state,
            &game,
            Card::Village,
            Zone::Discard(PlayerId(seat)),
        );
    }
    state["turn"]["phase"] = json!("Action");
    state["turn"]["actions"] = json!(8);
    state["pending"] = serde_json::to_value(Decision {
        player: PlayerId(0),
        kind: DecisionKind::Turn {
            phase: Phase::Action,
            moves: vec![
                Move::Play(serde_json::from_value(state["players"][0]["hand"][0].clone()).unwrap()),
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
            turn["pc"] = json!(2);
        }
    }
    serde_json::from_value(state).unwrap()
}
fn take(state: &mut Value, game: &Game, card: Card, zone: Zone) {
    let pile = game
        .supply_pile(card)
        .or_else(|| game.origin_pile(card))
        .unwrap()
        .index();
    let cards = state["piles"][pile]["cards"].as_array_mut().unwrap();
    let pos = cards
        .iter()
        .rposition(|id| game.card(serde_json::from_value(id.clone()).unwrap()) == card)
        .unwrap();
    let id = cards.remove(pos);
    state["zones"][id.as_u64().unwrap() as usize] = serde_json::to_value(zone).unwrap();
    let (p, key) = match zone {
        Zone::Hand(p) => (p, "hand"),
        Zone::Deck(p) => (p, "deck"),
        Zone::Discard(p) => (p, "discard"),
        _ => panic!(),
    };
    state["players"][p.index()][key]
        .as_array_mut()
        .unwrap()
        .push(id);
}
fn update(game: &Game, seat: u8, index: i32) -> ProjectedUpdate {
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
    super::project(
        hub::SeatObservation {
            view: game.view(PlayerId(seat)),
            known: &KnownCards::default(),
            names: &names,
            decision_id: game.pending().map(|_| "rules-decision"),
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
    .unwrap_or_else(|e| panic!("projection at {:?}: {e}", game.pending()))
}
fn messages(u: &ProjectedUpdate, seat: u8) -> Vec<Message> {
    let p = &u.table.players[usize::from(seat)];
    let mut messages = vec![
        Message::Login(LoginSuccess::new(
            p.id,
            p.name.clone(),
            "synthetic-only".into(),
        )),
        Message::TableJoined {
            table: u.table.clone(),
            show: true,
        },
        Message::Snapshot(u.snapshot.clone()),
    ];
    if let Some(q) = &u.question {
        messages.push(Message::Question(q.wire.clone()));
    }
    messages
}
fn bytes(u: &ProjectedUpdate, seat: u8) -> Vec<u8> {
    ServerEncoder::new()
        .encode_batch(&messages(u, seat))
        .unwrap()
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Enumerate individual options, legal boundary selections and non-palindromic order.
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
            let mut a = vec![];
            let len = elements.len();
            if *min == 0 && *decline >= 0 {
                a.push(vec![]);
            }
            if *max >= 1 && *min <= 1 {
                a.extend((0..len).map(|i| vec![i as i32]));
            }
            if *min > 1 {
                for offset in 0..len {
                    a.push(
                        (0..*min as usize)
                            .map(|i| ((offset + i) % len) as i32)
                            .collect(),
                    );
                }
            }
            if *max > 1 {
                a.push((0..*max).rev().collect());
            }
            if a.is_empty() && *min == 0 {
                a.push(vec![]);
            }
            a
        }
        QuestionKind::Name => vec![vec![2], vec![3], vec![31]],
        QuestionKind::Number { min, max, .. } => {
            vec![vec![*min], vec![*max], vec![min + (max - min) / 2]]
        }
        _ => panic!("unexpected fixture shape"),
    }
}
fn assert_options(game: &Game, u: &ProjectedUpdate) -> Vec<(Vec<i32>, Response)> {
    let Some(q) = &u.question else {
        return vec![];
    };
    assert!(q.resolve_at(20, q.wire.index, &[]).is_err());
    assert!(q.resolve(q.wire.index + 1, &[]).is_err());
    assert!(q.resolve(q.wire.index, &[i32::MAX]).is_err());
    answers(&q.wire)
        .into_iter()
        .map(|a| {
            let response = q.resolve_at(19, q.wire.index, &a).unwrap();
            game.pending()
                .unwrap()
                .validate(&response)
                .unwrap_or_else(|e| panic!("{:?} {a:?}: {response:?} {e}", game.pending()));
            (a, response)
        })
        .collect()
}
fn default_response(game: &Game, variant: usize) -> Response {
    let d = game.pending().unwrap();
    match &d.kind {
        DecisionKind::Turn { .. } => Response::Move(Move::EndPhase),
        DecisionKind::Cards {
            candidates,
            min,
            max,
            ordered,
        } => {
            let n = if variant.is_multiple_of(3) {
                *min
            } else {
                *max
            };
            let mut selected = candidates.iter().copied().take(n).collect::<Vec<_>>();
            if *ordered {
                selected.reverse();
            }
            Response::Cards(selected)
        }
        DecisionKind::Pile { piles, .. } => Response::Pile(Some(piles[variant % piles.len()])),
        DecisionKind::Choose { options, min, .. } => {
            if d.purpose == Purpose::Trigger
                && variant.is_multiple_of(2)
                && options.contains(&Opt::No)
            {
                return Response::option(Opt::No);
            }
            let without_no: Vec<_> = options.iter().copied().filter(|o| *o != Opt::No).collect();
            let n = *min;
            Response::Options(
                (0..n)
                    .map(|i| without_no[(i + variant) % without_no.len()])
                    .collect(),
            )
        }
        DecisionKind::Number { min, max } => Response::Number(if variant.is_multiple_of(2) {
            *min
        } else {
            *max
        }),
        DecisionKind::NameCard => Response::Name(Card::Copper),
    }
}

#[test]
fn every_base2_intrigue2_effect_projects_and_every_offered_option_has_a_target() {
    let mut evidence = vec![];
    for &card in Card::ALL.iter().filter(|c| {
        matches!(c.set(), Set::Base | Set::Intrigue) && c.def().kingdom && !c.def().removed
    }) {
        for variant in 0..6 {
            let mut game = fixture(card, 2);
            for seat in 0..2 {
                let u = update(&game, seat, 1);
                assert!(!bytes(&u, seat).is_empty());
            }
            if !card.is(Types::ACTION) {
                evidence.push(json!({"card":card.name(),"branch":"passive setup/projection"}));
                continue;
            }
            play(&mut game, card);
            evidence.push(json!({"card":card.name(),"variant":variant,"branch":"after explicit play","hand_count":game.hand(PlayerId(0)).len(),"turn_actions":game.turn().actions,"turn_coins":game.turn().coins,"turn_buys":game.turn().buys}));
            for step in 0..24 {
                let Some(d) = game.pending() else {
                    break;
                };
                let seat = d.player.0;
                let u = update(&game, seat, step + 2);
                let _all = assert_options(&game, &u);
                if matches!(d.kind, DecisionKind::Turn { .. }) {
                    break;
                }
                evidence.push(json!({"card":card.name(),"variant":variant,"purpose":format!("{:?}",d.purpose),"story":u.question.as_ref().unwrap().wire.description.story.question_id}));
                let response = default_response(&game, variant);
                game.respond(response).unwrap();
            }
        }
    }
    if let Ok(path) = std::env::var("DOMINION_RULES_COVERAGE") {
        std::fs::write(path, serde_json::to_vec_pretty(&evidence).unwrap()).unwrap();
    }
}

#[test]
fn three_player_attack_reaction_and_private_questions_keep_seat_account_ids_distinct() {
    let mut game = fixture(Card::Militia, 3);
    play(&mut game, Card::Militia);
    assert_eq!(game.pending().unwrap().player, PlayerId(1));
    let first = update(&game, 1, 1);
    assert_eq!(
        first
            .snapshot
            .state
            .players
            .iter()
            .map(|p| p.player_id)
            .collect::<Vec<_>>(),
        vec![1000, 1013, 1026]
    );
    assert_eq!(first.snapshot.state.turn.owner_id, 0);
    for seat in [0, 2] {
        assert!(update(&game, seat, 1).question.is_none());
    }
    let u = update(&game, 1, 1);
    let options = assert_options(&game, &u);
    assert!(options.iter().any(|(_, r)| *r == Response::option(Opt::No)));
    game.respond(Response::option(Opt::No)).unwrap();
    assert_eq!(game.pending().unwrap().player, PlayerId(2));
    let u = update(&game, 2, 2);
    assert_options(&game, &u);
    game.respond(Response::option(Opt::No)).unwrap();
    assert_eq!(game.pending().unwrap().player, PlayerId(1));
    assert_eq!(game.pending().unwrap().purpose, Purpose::Discard);
    while !matches!(game.pending().unwrap().kind, DecisionKind::Turn { .. }) {
        let u = update(&game, game.pending().unwrap().player.0, 3);
        assert_options(&game, &u);
        let r = default_response(&game, 0);
        game.respond(r).unwrap();
    }
    assert_eq!(game.hand(PlayerId(1)).len(), 3);
    assert_eq!(game.hand(PlayerId(2)).len(), 3);
}

fn action(q: &Question, values: &[i32]) -> Value {
    fn leaf<'a>(q: &'a Question, v: &'a [i32], path: &mut Vec<i32>) -> (&'a Question, &'a [i32]) {
        if let QuestionKind::Complex(children) = &q.kind {
            let (&i, rest) = v.split_first().unwrap();
            path.push(i);
            leaf(&children[i as usize], rest, path)
        } else {
            (q, v)
        }
    }
    let mut path = vec![];
    let (q, v) = leaf(q, values, &mut path);
    let mut a = match &q.kind {
        QuestionKind::Name => json!({"name":catalog().catalog.cards[v[0] as usize].key}),
        QuestionKind::Number { .. } => json!({"number":v[0]}),
        _ if q.description.question_type
            == catalog().symbol("QuestionTypes", "ORDER_CARDS").unwrap() =>
        {
            json!({"order":v})
        }
        _ if v.is_empty() => json!({"decline":true}),
        _ => json!({"indices":v}),
    };
    a["path"] = json!(path);
    a
}
fn record(name: &str, u: &ProjectedUpdate, seat: u8, values: &[i32]) -> Value {
    let q = &u.question.as_ref().unwrap().wire;
    json!({"name":name,"messages":[{"hex":hex(&bytes(u,seat))}],"answer":action(q,values),"expected":{"values":values}})
}

#[test]
fn original_consumer_semantic_scenarios() {
    let mut scenarios = vec![];
    for card in [
        Card::Cellar,
        Card::Chapel,
        Card::Harbinger,
        Card::Vassal,
        Card::Mine,
        Card::Remodel,
        Card::Sentry,
        Card::Library,
        Card::Pawn,
        Card::Steward,
        Card::SecretPassage,
        Card::WishingWell,
        Card::Lurker,
        Card::Mill,
        Card::Torturer,
        Card::Swindler,
        Card::Haven,
        Card::NativeVillage,
        Card::Island,
        Card::Recruiter,
        Card::Engineer,
    ] {
        for variant in 0..3 {
            let mut game = fixture(card, 2);
            play(&mut game, card);
            for step in 0..12 {
                let d = game.pending().unwrap();
                if matches!(d.kind, DecisionKind::Turn { .. }) {
                    break;
                }
                let seat = d.player.0;
                let u = update(&game, seat, step + 1);
                let options = assert_options(&game, &u);
                if let Some((v, _)) = options.get(variant % options.len()) {
                    scenarios.push(record(
                        &format!("{}-{variant}-{step}", card.name()),
                        &u,
                        seat,
                        v,
                    ));
                }
                let r = default_response(&game, variant);
                game.respond(r).unwrap();
            }
        }
    }
    if let Ok(path) = std::env::var("DOMINION_RULES_FIXTURES") {
        std::fs::write(path, serde_json::to_vec_pretty(&scenarios).unwrap()).unwrap();
    }
    assert!(scenarios.len() > 50);
}

#[test]
fn cross_set_persistent_resources_landscapes_and_split_pile_witnesses() {
    for card in [
        Card::Bazaar,
        Card::Haven,
        Card::Caravan,
        Card::Lighthouse,
        Card::NativeVillage,
        Card::Outpost,
        Card::Island,
        Card::Baker,
        Card::Recruiter,
        Card::Engineer,
        Card::Alchemist,
        Card::Settlers,
    ] {
        let mut game = fixture(card, 2);
        play(&mut game, card);
        for i in 0..36 {
            let d = game.pending().unwrap();
            let u = update(&game, d.player.0, i + 1);
            assert_options(&game, &u);
            for seat in 0..2 {
                let u = update(&game, seat, i + 1);
                assert!(!bytes(&u, seat).is_empty());
            }
            let r = default_response(&game, 1);
            game.respond(r).unwrap();
            if game.turn().player == PlayerId(0) && game.player(PlayerId(0)).turns_taken > 1 {
                break;
            }
        }
    }
    let mut landscape_cases = vec![];
    for landscape in [Card::Expedition, Card::Cathedral] {
        let mut game = fixture(landscape, 2);
        game.respond(Response::Move(Move::EndPhase)).unwrap();
        play(&mut game, Card::Silver);
        play(&mut game, Card::Copper);
        assert!(
            matches!(&game.pending().unwrap().kind,DecisionKind::Turn{moves,..} if moves.contains(&Move::BuyLandscape(landscape)))
        );
        let u = update(&game, 0, 1);
        let choices = assert_options(&game, &u);
        assert!(
            choices
                .iter()
                .any(|(_, r)| *r == Response::Move(Move::BuyLandscape(landscape)))
        );
        let (values, _) = choices
            .iter()
            .find(|(_, r)| *r == Response::Move(Move::BuyLandscape(landscape)))
            .unwrap();
        landscape_cases.push(record(&format!("buy-{}", landscape.name()), &u, 0, values));
        game.respond(Response::Move(Move::BuyLandscape(landscape)))
            .unwrap();
        for i in 0..20 {
            let d = game.pending().unwrap();
            let u = update(&game, d.player.0, i + 2);
            assert_options(&game, &u);
            let r = default_response(&game, 0);
            game.respond(r).unwrap();
            if game.turn().player == PlayerId(0) && game.player(PlayerId(0)).turns_taken > 1 {
                break;
            }
        }
        let u = update(&game, 0, 30);
        assert!(
            u.snapshot
                .state
                .zones
                .iter()
                .any(|z| z.kind == 7 && z.pile_name == catalog().card(landscape).unwrap())
        );
        let mut case = json!({"name":format!("state-{}",landscape.name()),"messages":[{"hex":hex(&bytes(&u,0))}],"expected":{"gameId":100}});
        if landscape == Card::Cathedral {
            case["expectedSemantic"] = json!({"projects":{"0":["Cathedral"]}});
        }
        landscape_cases.push(case);
        if landscape == Card::Cathedral {
            assert!(u.snapshot.state.players[0].bought_projects.is_empty());
            assert!(u.snapshot.events.iter().any(|e|matches!(e,model::Event::ProjectBought(p) if p.player==0&&p.project==catalog().card(landscape).unwrap())));
        }
    }
    if let Ok(path) = std::env::var("DOMINION_LANDSCAPE_FIXTURES") {
        std::fs::write(path, serde_json::to_vec_pretty(&landscape_cases).unwrap()).unwrap();
    }
}

#[test]
fn ordered_answers_locations_and_exact_dynamic_cost_arguments() {
    let mut game = fixture(Card::Sentry, 2);
    play(&mut game, Card::Sentry);
    game.respond(Response::Cards(vec![])).unwrap();
    game.respond(Response::Cards(vec![])).unwrap();
    // Sentry has only two cards; use Patrol's three-card remaining permutation too.
    let u = update(&game, 0, 1);
    let q = u.question.as_ref().unwrap();
    let DecisionKind::Cards {
        candidates,
        ordered: true,
        ..
    } = &game.pending().unwrap().kind
    else {
        panic!()
    };
    assert_eq!(
        q.resolve(1, &[0, 1]).unwrap(),
        Response::Cards(vec![candidates[0], candidates[1]])
    );
    let mut game = fixture(Card::SecretPassage, 2);
    play(&mut game, Card::SecretPassage);
    let d = game.pending().unwrap();
    let DecisionKind::Cards { candidates, .. } = &d.kind else {
        panic!()
    };
    let chosen = candidates[0];
    game.respond(Response::Cards(vec![chosen])).unwrap();
    let u = update(&game, 0, 2);
    let q = u.question.as_ref().unwrap();
    let DecisionKind::Number { max, .. } = game.pending().unwrap().kind else {
        panic!()
    };
    assert_eq!(q.resolve(2, &[max]).unwrap(), Response::Number(0));
    assert_eq!(q.resolve(2, &[0]).unwrap(), Response::Number(max));
    for (card, plus) in [(Card::Mine, 3), (Card::Remodel, 2), (Card::Upgrade, 1)] {
        let mut game = fixture(card, 2);
        play(&mut game, card);
        let DecisionKind::Cards { candidates, .. } = &game.pending().unwrap().kind else {
            panic!()
        };
        let selected = *candidates
            .iter()
            .find(|&&id| game.card(id) == Card::Silver)
            .unwrap();
        game.respond(Response::Cards(vec![selected])).unwrap();
        let u = update(&game, 0, 3);
        let q = u.question.as_ref().unwrap();
        assert_eq!(
            q.wire.description.story.arguments,
            vec![dominion_online::questions::LogArgument::Amount {
                coins: 3 + plus,
                potions: 0,
                debt: 0
            }]
        );
    }
}

#[test]
fn looking_and_hidden_mat_twin_worlds_do_not_leak_allocations_or_about_cards() {
    for card in [Card::Sentry, Card::NativeVillage, Card::Haven] {
        let mut game = fixture(card, 2);
        play(&mut game, card);
        if card == Card::NativeVillage {
            game.respond(Response::option(Opt::Mode(0))).unwrap();
        }
        if card == Card::Haven {
            let DecisionKind::Cards { candidates, .. } = &game.pending().unwrap().kind else {
                panic!()
            };
            game.respond(Response::Cards(vec![candidates[0]])).unwrap();
        }
        let before = update(&game, 1, 1);
        assert!(before.question.is_none());
        let mut value = serde_json::to_value(&game).unwrap();
        let ids = if card == Card::Sentry {
            value["players"][0]["looking"].as_array().unwrap().clone()
        } else if card == Card::Haven {
            value["players"][0]["set_aside"].as_array().unwrap().clone()
        } else {
            value["players"][0]["mats"]["NativeVillage"]
                .as_array()
                .unwrap()
                .clone()
        };
        assert!(!ids.is_empty());
        for id in ids {
            value["cards"][id.as_u64().unwrap() as usize] = json!("Gold");
        }
        value["players"][0]["deck"]
            .as_array_mut()
            .unwrap()
            .reverse();
        let twin: Game = serde_json::from_value(value).unwrap();
        assert_eq!(
            bytes(&before, 1),
            bytes(&update(&twin, 1, 1), 1),
            "{}",
            card.name()
        );
    }
}

fn add_hand(game: &mut Game, card: Card) {
    let mut state = serde_json::to_value(&*game).unwrap();
    take(&mut state, game, card, Zone::Hand(PlayerId(0)));
    let id: CardId = serde_json::from_value(
        state["players"][0]["hand"]
            .as_array()
            .unwrap()
            .last()
            .unwrap()
            .clone(),
    )
    .unwrap();
    if let Some(moves) = state["pending"]["kind"]["Turn"]["moves"].as_array_mut() {
        moves.insert(0, serde_json::to_value(Move::Play(id)).unwrap());
    }
    *game = serde_json::from_value(state).unwrap();
}
#[test]
fn repeated_duration_reminders_start_ordering_and_real_ability_associations() {
    let mut game = fixture(Card::ThroneRoom, 2);
    add_hand(&mut game, Card::Haven);
    add_hand(&mut game, Card::Caravan);
    add_hand(&mut game, Card::Lighthouse);
    play(&mut game, Card::ThroneRoom);
    let DecisionKind::Cards { candidates, .. } = &game.pending().unwrap().kind else {
        panic!()
    };
    let haven = *candidates
        .iter()
        .find(|&&c| game.card(c) == Card::Haven)
        .unwrap();
    game.respond(Response::Cards(vec![haven])).unwrap();
    for wanted in [Card::Copper, Card::Estate] {
        let DecisionKind::Cards { candidates, .. } = &game.pending().unwrap().kind else {
            panic!()
        };
        let picked = *candidates
            .iter()
            .find(|&&c| game.card(c) == wanted)
            .unwrap();
        game.respond(Response::Cards(vec![picked])).unwrap();
    }
    play(&mut game, Card::Caravan);
    play(&mut game, Card::Lighthouse);
    let u = update(&game, 0, 1);
    assert_eq!(
        u.snapshot.state.players[0]
            .reminders
            .iter()
            .find(|r| r.card_name == catalog().card(Card::Haven).unwrap())
            .unwrap()
            .frequency,
        2
    );
    assert_eq!(
        u.snapshot
            .state
            .associations
            .iter()
            .find(|a| u.snapshot.state.cards[a.card as usize].card_name
                == catalog().card(Card::Haven).unwrap())
            .unwrap()
            .linked_cards
            .len(),
        2
    );
    let opponent = update(&game, 1, 1);
    assert!(
        opponent
            .snapshot
            .state
            .associations
            .iter()
            .all(|a| a.linked_cards.is_empty())
    );
    let mut reached = false;
    for i in 0..40 {
        let d = game.pending().unwrap();
        let u = update(&game, d.player.0, i + 2);
        assert_options(&game, &u);
        if d.purpose == Purpose::Trigger && d.player == PlayerId(0) {
            let QuestionKind::Choice {
                elements: dominion_online::questions::QuestionElements::Abilities(abilities),
                ..
            } = &u.question.as_ref().unwrap().wire.kind
            else {
                panic!()
            };
            assert!(abilities.iter().all(|a| a.association >= 0));
            let havens: Vec<_> = abilities
                .iter()
                .filter(|a| a.name == catalog().symbol("AbilityNames", "HAVEN_ABILITY").unwrap())
                .collect();
            assert_eq!(havens.len(), 2);
            assert_ne!(havens[0].arguments, havens[1].arguments);
            if let Ok(path) = std::env::var("DOMINION_DURATION_FIXTURES") {
                let cases: Vec<_> = assert_options(&game, &u)
                    .iter()
                    .enumerate()
                    .map(|(i, (v, _))| {
                        let mut case=record(&format!("repeated-haven-start-{i}"), &u, 0, v);
                        case["expectedSemantic"]=json!({"reminders":{"0":[{"name":"Haven","count":2,"blinking":false},{"name":"Caravan","count":1,"blinking":false},{"name":"Lighthouse","count":1,"blinking":false}]}});case
                    })
                    .collect();
                std::fs::write(path, serde_json::to_vec_pretty(&cases).unwrap()).unwrap();
            }
            reached = true;
            break;
        }
        let r = default_response(&game, 0);
        game.respond(r).unwrap();
    }
    assert!(reached);
}

#[test]
fn resource_moves_potion_reaction_extra_turn_and_split_top_are_witnessed() {
    let mut scenarios = vec![];
    let mut game = fixture(Card::Recruiter, 2);
    play(&mut game, Card::Recruiter);
    let DecisionKind::Cards { candidates, .. } = &game.pending().unwrap().kind else {
        panic!()
    };
    let silver = *candidates
        .iter()
        .find(|&&c| game.card(c) == Card::Silver)
        .unwrap();
    game.respond(Response::Cards(vec![silver])).unwrap();
    assert_eq!(game.player(PlayerId(0)).villagers, 3);
    let u = update(&game, 0, 1);
    let options = assert_options(&game, &u);
    let (v, _) = options
        .iter()
        .find(|(_, r)| *r == Response::Move(Move::Villager))
        .unwrap();
    scenarios.push(record("villager-move", &u, 0, v));
    let mut game = fixture(Card::Baker, 2);
    play(&mut game, Card::Baker);
    let u = update(&game, 0, 2);
    let options = assert_options(&game, &u);
    let (v, _) = options
        .iter()
        .find(|(_, r)| *r == Response::Move(Move::Coffers))
        .unwrap();
    scenarios.push(record("coffers-move", &u, 0, v));
    let mut game = fixture(Card::Engineer, 2);
    game.respond(Response::Move(Move::EndPhase)).unwrap();
    play(&mut game, Card::Silver);
    game.respond(Response::Move(Move::Buy(
        game.supply_pile(Card::Engineer).unwrap(),
    )))
    .unwrap();
    assert_eq!(game.player(PlayerId(0)).debt, 4);
    let u = update(&game, 0, 3);
    let options = assert_options(&game, &u);
    let (v, _) = options
        .iter()
        .find(|(_, r)| *r == Response::Move(Move::PayDebt))
        .unwrap();
    scenarios.push(record("debt-move", &u, 0, v));
    let mut game = fixture(Card::Alchemist, 2);
    add_hand(&mut game, Card::Potion);
    play(&mut game, Card::Alchemist);
    game.respond(Response::Move(Move::EndPhase)).unwrap();
    play(&mut game, Card::Potion);
    let u = update(&game, 0, 4);
    assert!(
        u.snapshot.state.counters.iter().any(|c| c.kind
            == catalog().symbol("CounterNames", "POTIONS").unwrap()
            && c.value == 1)
    );
    game.respond(Response::Move(Move::EndPhase)).unwrap();
    assert_eq!(game.pending().unwrap().purpose, Purpose::Trigger);
    let u = update(&game, 0, 5);
    let options = assert_options(&game, &u);
    scenarios.push(record("alchemist-cleanup", &u, 0, &options[0].0));
    let mut game = fixture(Card::Outpost, 2);
    play(&mut game, Card::Outpost);
    let mut extra = false;
    for i in 0..10 {
        let u = update(&game, 0, i + 6);
        assert_options(&game, &u);
        if game.turn().extra == Some(Card::Outpost) {
            assert_eq!(
                u.snapshot.state.turn.turn_type,
                catalog().symbol("ExtraTurnTypes", "OUTPOST").unwrap()
            );
            extra = true;
            break;
        }
        game.respond(Response::Move(Move::EndPhase)).unwrap();
    }
    assert!(extra);
    let game = fixture(Card::Settlers, 2);
    let mut state = serde_json::to_value(&game).unwrap();
    let pile = game.supply_pile(Card::Settlers).unwrap().index();
    let mut moved = vec![];
    state["piles"][pile]["cards"]
        .as_array_mut()
        .unwrap()
        .retain(|id| {
            let card = game.card(serde_json::from_value(id.clone()).unwrap());
            if card == Card::Settlers {
                moved.push(id.clone());
                false
            } else {
                true
            }
        });
    for id in moved {
        state["zones"][id.as_u64().unwrap() as usize] = json!("Trash");
        state["trash"].as_array_mut().unwrap().push(id);
    }
    let mut game: Game = serde_json::from_value(state).unwrap();
    add_hand(&mut game, Card::Bridge);
    play(&mut game, Card::Bridge);
    let u = update(&game, 0, 30);
    let top = game
        .pile_top(game.supply_pile(Card::Settlers).unwrap())
        .unwrap();
    assert_eq!(game.card(top), Card::BustlingVillage);
    assert!(u.snapshot.state.cards.iter().any(|c| c.card_name
        == catalog().card(Card::BustlingVillage).unwrap()
        && c.pile_name == catalog().card(Card::Settlers).unwrap()));
    assert!(
        u.snapshot
            .state
            .cost_reduction
            .cost_override
            .iter()
            .any(
                |(name, cost)| *name == catalog().card(Card::BustlingVillage).unwrap()
                    && cost.coins == 4
            )
    );
    for case in &mut scenarios {
        case["expectedSemantic"] = match case["name"].as_str().unwrap() {
            "villager-move" => json!({"playerCounters":{"0":{"VILLAGERS":3}}}),
            "coffers-move" => json!({"playerCounters":{"0":{"COFFERS":2},"1":{"COFFERS":1}}}),
            "debt-move" => json!({"playerCounters":{"0":{"DEBT":4,"COINS":2}}}),
            "alchemist-cleanup" => json!({"playerCounters":{"0":{"POTIONS":1}}}),
            _ => unreachable!(),
        };
    }
    if let Ok(path) = std::env::var("DOMINION_RESOURCE_FIXTURES") {
        std::fs::write(path, serde_json::to_vec_pretty(&scenarios).unwrap()).unwrap();
    }
}

#[test]
fn nonpalindromic_four_card_order_and_original_pawn_shortcut() {
    let mut scenarios = vec![];
    let mut game = fixture(Card::Patrol, 2);
    let mut state = serde_json::to_value(&game).unwrap();
    let old = std::mem::take(state["players"][0]["deck"].as_array_mut().unwrap());
    for id in old {
        state["zones"][id.as_u64().unwrap() as usize] = json!({"Discard":0});
        state["players"][0]["discard"]
            .as_array_mut()
            .unwrap()
            .push(id);
    }
    for card in [
        Card::Village,
        Card::Smithy,
        Card::Silver,
        Card::Copper,
        Card::Gold,
        Card::Silver,
        Card::Copper,
    ] {
        take(&mut state, &game, card, Zone::Deck(PlayerId(0)));
    }
    game = serde_json::from_value(state).unwrap();
    play(&mut game, Card::Patrol);
    let DecisionKind::Cards {
        candidates,
        ordered: true,
        ..
    } = &game.pending().unwrap().kind
    else {
        panic!()
    };
    assert_eq!(candidates.len(), 4);
    let candidates = candidates.clone();
    let u = update(&game, 0, 1);
    let q = u.question.as_ref().unwrap();
    assert_eq!(
        q.resolve(1, &[0, 1, 2, 3]).unwrap(),
        Response::Cards(candidates.clone())
    );
    for a in 0..4 {
        for b in 0..4 {
            for c in 0..4 {
                for d in 0..4 {
                    let order = [a, b, c, d];
                    if order.iter().collect::<BTreeSet<_>>().len() != 4 {
                        continue;
                    }
                    assert_eq!(
                        q.resolve(1, &order).unwrap(),
                        Response::Cards(
                            order
                                .iter()
                                .rev()
                                .map(|&i| candidates[3 - i as usize])
                                .collect()
                        )
                    );
                }
            }
        }
    }
    let mut done_case = record("patrol-original-done-keeps-order", &u, 0, &[0, 1, 2, 3]);
    done_case["answer"] = json!({"send":true});
    done_case["expectedSemantic"] = json!({"targetNames":[["Village","Smithy","Silver","Copper"]]});
    scenarios.push(done_case);
    let values = vec![2, 0, 3, 1];
    let response = q.resolve(1, &values).unwrap();
    scenarios.push(record("patrol-four-card-order", &u, 0, &values));
    game.respond(response.clone()).unwrap();
    let Response::Cards(chosen) = response else {
        panic!()
    };
    assert_eq!(
        &game
            .player(PlayerId(0))
            .deck
            .iter()
            .rev()
            .take(4)
            .copied()
            .collect::<Vec<_>>(),
        &chosen
    );
    let mut game = fixture(Card::Pawn, 2);
    play(&mut game, Card::Pawn);
    let u = update(&game, 0, 2);
    let q = u.question.as_ref().unwrap();
    assert_eq!(
        q.resolve(2, &[0, 1]).unwrap(),
        Response::Options(vec![Opt::Mode(0), Opt::Mode(1)])
    );
    let mut case = record("pawn-original-card-action-shortcut", &u, 0, &[0, 1]);
    case["answer"] = json!({"button":0});
    scenarios.push(case);
    if let Ok(path) = std::env::var("DOMINION_ORDER_FIXTURES") {
        std::fs::write(path, serde_json::to_vec_pretty(&scenarios).unwrap()).unwrap();
    }
}

#[test]
fn three_player_result_order_and_off_turn_consumer_fixture() {
    let mut game = fixture(Card::Militia, 3);
    play(&mut game, Card::Militia);
    let u = update(&game, 1, 1);
    let options = assert_options(&game, &u);
    let (v, _) = options
        .iter()
        .find(|(_, r)| *r == Response::option(Opt::No))
        .unwrap();
    if let Ok(path) = std::env::var("DOMINION_THREE_PLAYER_FIXTURES") {
        std::fs::write(
            path,
            serde_json::to_vec_pretty(&vec![record("three-player-off-turn-moat", &u, 1, v)])
                .unwrap(),
        )
        .unwrap();
    }
    let outcome = Outcome {
        reason: EndReason::PilesEmpty,
        scores: vec![12, 12, 1],
        turns: vec![9, 8, 9],
        winners: vec![PlayerId(1)],
        resigned: vec![],
    };
    let mut state = serde_json::to_value(&game).unwrap();
    state["outcome"] = serde_json::to_value(outcome).unwrap();
    state["pending"] = Value::Null;
    let game: Game = serde_json::from_value(state).unwrap();
    let u = update(&game, 2, 2);
    let result = u.finished.unwrap();
    assert_eq!(
        result
            .players
            .iter()
            .map(|p| p.player_id)
            .collect::<Vec<_>>(),
        vec![1000, 1013, 1026]
    );
    assert_eq!(
        result.players.iter().map(|p| p.rank).collect::<Vec<_>>(),
        vec![2, 1, 3]
    );
}

#[test]
fn about_only_cards_are_disclosed_to_decider_without_allocating_hidden_neighbors() {
    let game = fixture(Card::Sentry, 2);
    let mut state = serde_json::to_value(&game).unwrap();
    let about = state["players"][0]["deck"]
        .as_array()
        .unwrap()
        .last()
        .unwrap()
        .clone();
    state["pending"]["about"] = json!([about]);
    let game: Game = serde_json::from_value(state.clone()).unwrap();
    let own = update(&game, 0, 1);
    let other = update(&game, 1, 1);
    assert!(other.question.is_none());
    assert!(
        own.snapshot
            .state
            .zones
            .iter()
            .any(|z| z.kind == 23 && !z.cards.is_empty())
    );
    let id = about.as_u64().unwrap() as usize;
    state["cards"][id] = json!("Curse");
    let twin: Game = serde_json::from_value(state).unwrap();
    assert_eq!(bytes(&other, 1), bytes(&update(&twin, 1, 1), 1));
    assert_ne!(bytes(&own, 0), bytes(&update(&twin, 0, 1), 0));
}

fn set_deck(game: &mut Game, player: PlayerId, top_first: &[Card]) {
    let mut state = serde_json::to_value(&*game).unwrap();
    let old = std::mem::take(
        state["players"][player.index()]["deck"]
            .as_array_mut()
            .unwrap(),
    );
    for id in old {
        state["zones"][id.as_u64().unwrap() as usize] = json!({"Discard":player.0});
        state["players"][player.index()]["discard"]
            .as_array_mut()
            .unwrap()
            .push(id);
    }
    for &card in top_first.iter().rev() {
        take(&mut state, game, card, Zone::Deck(player));
    }
    *game = serde_json::from_value(state).unwrap();
}
fn semantic_record(
    name: &str,
    u: &ProjectedUpdate,
    seat: u8,
    values: &[i32],
    text: &str,
    target_names: Vec<String>,
) -> Value {
    let mut case = record(name, u, seat, values);
    case["expectedSemantic"] = json!({"questionTexts":[text],"targetNames":[target_names]});
    case
}
#[test]
fn reviewed_optional_play_library_lurker_gain_and_swindler_branches() {
    let mut cases = vec![];
    for yes in [false, true] {
        let mut game = fixture(Card::Vassal, 2);
        set_deck(
            &mut game,
            PlayerId(0),
            &[Card::Village, Card::Copper, Card::Silver],
        );
        play(&mut game, Card::Vassal);
        assert_eq!(game.pending().unwrap().source, Some(Card::Vassal));
        assert_eq!(game.pending().unwrap().purpose, Purpose::Play);
        let referenced = game.view(PlayerId(0)).pending_reference().unwrap();
        assert_eq!(game.card(referenced), Card::Village);
        let u = update(&game, 0, 1);
        let options = assert_options(&game, &u);
        let (v, r) = options
            .iter()
            .find(|(_, r)| *r == Response::option(if yes { Opt::Yes } else { Opt::No }))
            .unwrap();
        let mut case = semantic_record(
            if yes {
                "vassal-play-action"
            } else {
                "vassal-decline-action"
            },
            &u,
            0,
            v,
            "You may play a Village",
            vec!["Village".into()],
        );
        game.respond(r.clone()).unwrap();
        assert_eq!(
            game.zone(referenced),
            if yes {
                Zone::InPlay(PlayerId(0))
            } else {
                Zone::Discard(PlayerId(0))
            }
        );
        case["engineEffect"] = json!({"card":"Village","zone":if yes{"in_play"}else{"discard"}});
        cases.push(case);
        let mut game = fixture(Card::Library, 2);
        set_deck(
            &mut game,
            PlayerId(0),
            &[Card::Village, Card::Copper, Card::Silver, Card::Gold],
        );
        play(&mut game, Card::Library);
        assert_eq!(game.pending().unwrap().source, Some(Card::Library));
        let referenced = game.view(PlayerId(0)).pending_reference().unwrap();
        assert_eq!(game.card(referenced), Card::Village);
        let u = update(&game, 0, 2);
        let options = assert_options(&game, &u);
        let (v, r) = options
            .iter()
            .find(|(_, r)| *r == Response::option(if yes { Opt::Yes } else { Opt::No }))
            .unwrap();
        let mut case = semantic_record(
            if yes {
                "library-skip-action"
            } else {
                "library-keep-action"
            },
            &u,
            0,
            v,
            "You may skip a Village",
            vec![],
        );
        // Library uses modes; their own original button text carries Skip/Keep semantics.
        case["expectedSemantic"]
            .as_object_mut()
            .unwrap()
            .remove("targetNames");
        game.respond(r.clone()).unwrap();
        assert_eq!(game.hand(PlayerId(0)).len(), 7);
        assert_eq!(
            game.zone(referenced),
            if yes {
                Zone::Discard(PlayerId(0))
            } else {
                Zone::Hand(PlayerId(0))
            }
        );
        case["engineEffect"] = json!({"hand_count":7,"Village":if yes{"discard"}else{"hand"}});
        cases.push(case);
    }
    let mut game = fixture(Card::Lurker, 2);
    let mut state = serde_json::to_value(&game).unwrap();
    for card in [Card::Village, Card::Smithy] {
        let pile = game
            .supply_pile(card)
            .or_else(|| game.origin_pile(card))
            .unwrap()
            .index();
        let id = state["piles"][pile]["cards"]
            .as_array_mut()
            .unwrap()
            .pop()
            .unwrap();
        state["zones"][id.as_u64().unwrap() as usize] = json!("Trash");
        state["trash"].as_array_mut().unwrap().push(id);
    }
    game = serde_json::from_value(state).unwrap();
    play(&mut game, Card::Lurker);
    game.respond(Response::option(Opt::Mode(1))).unwrap();
    let u = update(&game, 0, 3);
    let options = assert_options(&game, &u);
    let(v,r)=options.iter().find(|(_,r)|matches!(r,Response::Cards(ids) if ids.len()==1&&game.card(ids[0])==Card::Village)).unwrap();
    let mut case = semantic_record(
        "lurker-gain-from-two-actions",
        &u,
        0,
        v,
        "Gain from the trash",
        vec!["Smithy".into(), "Village".into()],
    );
    let Response::Cards(chosen) = r else { panic!() };
    let picked = chosen[0];
    game.respond(r.clone()).unwrap();
    assert_eq!(game.zone(picked), Zone::Discard(PlayerId(0)));
    case["engineEffect"] = json!({"gained":"Village","from":"trash","to":"discard","player":0});
    cases.push(case);
    let mut game = fixture(Card::Swindler, 2);
    set_deck(&mut game, PlayerId(1), &[Card::Silver]);
    play(&mut game, Card::Swindler);
    game.respond(Response::option(Opt::No)).unwrap();
    assert_eq!(game.pending().unwrap().source, Some(Card::Swindler));
    assert_eq!(game.view(PlayerId(0)).pending_subject(), Some(PlayerId(1)));
    let u = update(&game, 0, 4);
    let options = assert_options(&game, &u);
    let (v, r) = options
        .iter()
        .find(|(_, r)| *r == Response::Pile(game.supply_pile(Card::Village)))
        .unwrap();
    let target_names = if let QuestionKind::Choice {
        elements: dominion_online::questions::QuestionElements::Cards(ids),
        ..
    } = &u.question.as_ref().unwrap().wire.kind
    {
        ids.iter()
            .map(|&id| {
                catalog().catalog.cards[u.snapshot.state.cards[id as usize].card_name as usize]
                    .name
                    .clone()
            })
            .collect()
    } else {
        panic!()
    };
    let mut case = semantic_record(
        "swindler-replacement-for-other-seat",
        &u,
        0,
        v,
        "Choose a card for Seat2",
        target_names,
    );
    let before = game.player(PlayerId(1)).discard.len();
    game.respond(r.clone()).unwrap();
    assert_eq!(game.player(PlayerId(1)).discard.len(), before + 1);
    assert_eq!(
        game.card(*game.player(PlayerId(1)).discard.last().unwrap()),
        Card::Village
    );
    case["engineEffect"] =
        json!({"gained":"Village","player":1,"cost":{"coins":3,"potions":0,"debt":0}});
    cases.push(case);
    if let Ok(path) = std::env::var("DOMINION_REVIEW_FIXTURES") {
        std::fs::write(path, serde_json::to_vec_pretty(&cases).unwrap()).unwrap();
    }
}

#[test]
fn colony_pile_empty_engine_completion_has_original_result() {
    let mut game = fixture_with_colonies(Card::Village, 2, true);
    let pile = game.supply_pile(Card::Colony).unwrap();
    let mut state = serde_json::to_value(&game).unwrap();
    let cards = std::mem::take(
        state["piles"][pile.index()]["cards"]
            .as_array_mut()
            .unwrap(),
    );
    for id in cards {
        state["zones"][id.as_u64().unwrap() as usize] = json!("Trash");
        state["trash"].as_array_mut().unwrap().push(id);
    }
    game = serde_json::from_value(state).unwrap();
    for _ in 0..6 {
        if game.outcome().is_some() {
            break;
        }
        game.respond(Response::Move(Move::EndPhase)).unwrap();
    }
    assert_eq!(game.outcome().unwrap().reason, EndReason::ColoniesGone);
    let u = update(&game, 0, 1);
    let result = u.finished.as_ref().unwrap();
    assert_eq!(result.players.len(), 2);
    for (p, score) in result.players.iter().zip(&game.outcome().unwrap().scores) {
        assert_eq!(p.score.total_points, *score);
    }
    if let Ok(path) = std::env::var("DOMINION_COLONY_FIXTURES") {
        let mut m = messages(&u, 0);
        m.push(Message::GameFinished {
            table: u.table.clone(),
            result: result.clone(),
            continue_allowed: false,
            match_completed: true,
        });
        let score:Vec<_>=result.players.iter().map(|p|json!({"playerId":p.player_id,"rank":p.rank,"totalPoints":p.score.total_points,"usedTurns":p.score.used_turns,"resigned":false,"parts":[]})).collect();
        let case = json!({"name":"colonies-gone-ordinary-engine-completion","messages":[{"hex":hex(&ServerEncoder::new().encode_batch(&m).unwrap())}],"expected":{"score":score},"engineEffect":{"reason":"ColoniesGone"}});
        std::fs::write(path, serde_json::to_vec_pretty(&vec![case]).unwrap()).unwrap();
    }
}

#[test]
fn bustling_village_takes_actual_settlers_from_discard() {
    let mut game = fixture(Card::Settlers, 2);
    add_hand(&mut game, Card::BustlingVillage);
    let mut state = serde_json::to_value(&game).unwrap();
    let ids = state["players"][0]["hand"].as_array_mut().unwrap();
    let i = ids
        .iter()
        .position(|id| game.card(serde_json::from_value(id.clone()).unwrap()) == Card::Settlers)
        .unwrap();
    let id = ids.remove(i);
    state["zones"][id.as_u64().unwrap() as usize] = json!({"Discard":0});
    state["players"][0]["discard"]
        .as_array_mut()
        .unwrap()
        .push(id);
    game = serde_json::from_value(state).unwrap();
    play(&mut game, Card::BustlingVillage);
    assert_eq!(game.pending().unwrap().purpose, Purpose::Move);
    let u = update(&game, 0, 1);
    let options = assert_options(&game, &u);
    let(v,r)=options.iter().find(|(_,r)|matches!(r,Response::Cards(ids) if ids.len()==1&&game.card(ids[0])==Card::Settlers)).unwrap();
    let mut case = semantic_record(
        "bustling-village-take-settlers",
        &u,
        0,
        v,
        "You may take a Settlers",
        vec!["Settlers".into()],
    );
    let before = game.hand(PlayerId(0)).len();
    let Response::Cards(chosen) = r else { panic!() };
    let picked = chosen[0];
    game.respond(r.clone()).unwrap();
    assert_eq!(game.hand(PlayerId(0)).len(), before + 1);
    assert_eq!(game.zone(picked), Zone::Hand(PlayerId(0)));
    case["engineEffect"] = json!({"card":"Settlers","from":"discard","to":"hand","player":0});
    if let Ok(path) = std::env::var("DOMINION_BUSTLING_FIXTURES") {
        std::fs::write(path, serde_json::to_vec_pretty(&vec![case]).unwrap()).unwrap();
    }
}
