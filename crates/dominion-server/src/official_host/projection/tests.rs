use super::super::{
    catalog::tests::fixture,
    profile::{self, KINGDOM},
    questions::AnswerRejection,
};
use super::*;
use dominion_engine::{
    Decision, DecisionKind, Game, GameConfig, KnownCards, Move, Phase, Purpose, Response,
};
use dominion_online::{
    reducer::Reducer,
    server::{AttachedReceiver, ServerEncoder},
    transport::{Decoder, Message},
};
use dominion_protocol::Answer;
use serde_json::{Value, json};

fn project(
    observation: hub::SeatObservation<'_>,
    ids: &WireIdentity,
    catalog: &WireCatalog,
    index: i32,
    reconnect: bool,
) -> Result<ProjectedUpdate, ProjectionError> {
    let table = transport::TableDetails {
        table_id: ids.table_id,
        host_id: ids.player_ids[0],
        players: ids
            .player_ids
            .iter()
            .zip(observation.names)
            .map(|(&id, name)| transport::NamedId {
                id,
                name: name.clone(),
            })
            .collect(),
        ready: vec![true; ids.player_ids.len()],
        bots: vec![],
        spectators: vec![],
        reconnecting: vec![],
        rules: vec![],
        division: String::new(),
        match_games: vec![],
        match_score: None,
        tournament: None,
    };
    let context = HostGameContext {
        wire_ids: ids.clone(),
        table_details: table,
        viewer_role: ViewerRole::Player(observation.view.me().map_or(usize::MAX, PlayerId::index)),
        table_options: dominion_protocol::lobby::TableSpec {
            timed: false,
            rated: false,
            ..Default::default()
        },
    };
    super::project(
        observation,
        &context,
        catalog,
        ProjectionRequest {
            question_index: index,
            reconnect,
            epoch: 0,
        },
    )
}
pub(super) fn catalog() -> WireCatalog {
    WireCatalog::new(fixture()).unwrap()
}
fn ids() -> WireIdentity {
    WireIdentity {
        game_id: 1,
        table_id: 1,
        player_ids: vec![10, 20],
    }
}
fn config() -> GameConfig {
    GameConfig::new(2, KINGDOM.to_vec(), 42)
}
fn update(game: &Game, seat: u8, catalog: &WireCatalog, index: i32) -> ProjectedUpdate {
    project(
        hub::SeatObservation {
            clock: None,
            view: game.view(PlayerId(seat)),
            known: &KnownCards::default(),
            names: &["Ada".into(), "Bob".into()],
            decision_id: game.pending().map(|_| "engine-d1"),
            deciding: game.pending().map(|d| d.player),
            version: 7,
            undos: 2,
            outcome: game.outcome(),
            ended: None,
        },
        &ids(),
        catalog,
        index,
        false,
    )
    .unwrap()
}
fn messages(update: &ProjectedUpdate) -> Vec<Message> {
    let mut out = vec![
        Message::TableDetails(update.table.clone()),
        Message::Snapshot(update.snapshot.clone()),
    ];
    if let Some(q) = &update.question {
        out.push(Message::Question(q.wire.clone()));
    }
    if let Some(result) = &update.finished {
        out.push(Message::GameFinished {
            table: update.table.clone(),
            result: result.clone(),
            continue_allowed: false,
            match_completed: true,
        });
    }
    out
}
fn bytes(update: &ProjectedUpdate) -> Vec<u8> {
    ServerEncoder::new()
        .encode_batch(&messages(update))
        .unwrap()
}
fn receive(update: &ProjectedUpdate, catalog: &WireCatalog, seat: u8) -> AttachedReceiver {
    let mut receiver = AttachedReceiver::new(Reducer::new(
        catalog.catalog.clone(),
        ids().player_ids[seat as usize],
    ));
    receiver
        .apply_batch(Decoder::new().decode(&bytes(update)).unwrap())
        .unwrap();
    receiver
}
fn answer(update: &ProjectedUpdate, catalog: &WireCatalog, indices: Vec<u32>) -> Response {
    let mut receiver = receive(update, catalog, 0);
    let decision = receiver.reducer.view().unwrap().decision.unwrap();
    let (index, values) = receiver
        .reducer
        .answer(&Answer::indices(&decision.id, indices), None)
        .unwrap();
    update
        .question
        .as_ref()
        .unwrap()
        .resolve(index, &values)
        .unwrap()
}

/// Build a stable Action boundary with a chosen hand, keeping every card in one zone.
fn action_game(hand: &[Card]) -> Game {
    let game = Game::new(config()).unwrap();
    let mut state = serde_json::to_value(&game).unwrap();
    let old = std::mem::take(state["players"][0]["hand"].as_array_mut().unwrap());
    for id in old {
        state["zones"][id.as_u64().unwrap() as usize] = json!({"Deck":0});
        state["players"][0]["deck"].as_array_mut().unwrap().push(id);
    }
    let mut selected = Vec::new();
    for &card in hand {
        let pile = game.supply_pile(card).unwrap().index();
        let id = state["piles"][pile]["cards"]
            .as_array_mut()
            .unwrap()
            .pop()
            .unwrap();
        state["zones"][id.as_u64().unwrap() as usize] = json!({"Hand":0});
        state["players"][0]["hand"]
            .as_array_mut()
            .unwrap()
            .push(id.clone());
        if card.is(Types::ACTION) {
            selected.push(Move::Play(serde_json::from_value(id).unwrap()));
        }
    }
    selected.push(Move::EndPhase);
    state["turn"]["phase"] = json!("Action");
    state["turn"]["actions"] = json!(1);
    let decision = Decision {
        player: PlayerId(0),
        kind: DecisionKind::Turn {
            phase: Phase::Action,
            moves: selected,
        },
        purpose: Purpose::Turn,
        public_choice: false,
        source: None,
        about: vec![],
        question: None,
        plays: vec![],
    };
    state["pending"] = serde_json::to_value(decision).unwrap();
    for frame in state["stack"].as_array_mut().unwrap() {
        if let Some(turn) = frame.get_mut("Turn") {
            turn["pc"] = json!(2);
        }
    }
    serde_json::from_value(state).unwrap()
}
pub(super) fn play(game: &mut Game, card: Card) {
    let id = game
        .hand(PlayerId(0))
        .iter()
        .find(|&&id| game.card(id) == card)
        .copied()
        .unwrap();
    game.respond(Response::Move(Move::Play(id))).unwrap();
}

#[test]
fn admission_checks_configuration_and_metadata_not_only_pending_question() {
    let catalog = catalog();
    let cfg = config();
    let meta = hub::Meta::default();
    profile::check_supported(&cfg, &meta, &catalog, false).unwrap();
    let mut cases = vec![];
    let mut c = cfg.clone();
    c.players = 1;
    cases.push(c);
    let mut c = cfg.clone();
    c.kingdom[0] = Card::Possession;
    cases.push(c);
    let mut c = cfg.clone();
    c.landscapes.push(Card::Fountain);
    cases.push(c);
    let mut c = cfg.clone();
    c.shelters = Some(true);
    cases.push(c);
    let mut c = cfg.clone();
    c.setup_choices.insert(Card::Workshop, vec![Card::Village]);
    cases.push(c);
    let mut c = cfg.clone();
    c.turn_limit = Some(10);
    cases.push(c);
    let mut c = cfg.clone();
    c.kingdom[0] = c.kingdom[1];
    cases.push(c);
    for c in &cases {
        assert!(profile::check_supported(c, &meta, &catalog, false).is_err());
    }
    // Admitting unreviewed cards admits the Fountain, never anything else refused.
    for (i, c) in cases.iter().enumerate() {
        assert_eq!(
            profile::check_supported(c, &meta, &catalog, true).is_ok(),
            i == 2,
            "{i}"
        );
    }
    assert!(
        profile::check_supported(
            &cfg,
            &hub::Meta {
                rated: true,
                ..Default::default()
            },
            &catalog,
            false
        )
        .is_err()
    );
    assert!(
        profile::check_supported(
            &cfg,
            &hub::Meta {
                timed: true,
                ..Default::default()
            },
            &catalog,
            false
        )
        .is_err()
    );
}

#[test]
fn snapshot_raw_allocations_are_disclosed_only_and_receiver_facts_agree() {
    let game = Game::new(config()).unwrap();
    let catalog = catalog();
    for seat in 0..2 {
        let update = update(&game, seat, &catalog, 12);
        let state = &update.snapshot.state;
        assert_eq!((update.version, update.undos), (7, 2));
        let visible_slots = state
            .zones
            .iter()
            .flat_map(|z| &z.cards)
            .filter(|&&id| id >= 0)
            .count();
        assert_eq!(state.cards.len(), visible_slots);
        assert_eq!(
            state.card_counts.iter().sum::<i32>() as usize,
            visible_slots
        );
        for z in &state.zones {
            assert_eq!(z.created_by, -1);
            if z.kind == zone::DRAW || (z.kind == zone::HAND && z.owner != i32::from(seat)) {
                assert!(z.cards.iter().all(|&id| id == -1));
            }
            if z.kind == zone::DISCARD {
                assert!(z.cards.iter().skip(1).all(|&id| id == -1));
            }
            // A pile is bottom first: only its top, the last, is disclosed.
            if matches!(z.kind, zone::SUPPLY | zone::TREASURE | zone::VICTORY) {
                assert!(z.cards.iter().rev().skip(1).all(|&id| id == -1));
            }
        }
        assert!(
            state
                .players
                .iter()
                .all(|p| p.card_frequencies.is_empty() && p.reminders.is_empty())
        );
        assert!(state.associations.is_empty() && state.card_statuses.is_empty());
        let receiver = receive(&update, &catalog, seat);
        let view = receiver.reducer.view().unwrap();
        assert_eq!(
            view.you.as_ref().unwrap().hand.count,
            game.hand(PlayerId(seat)).len() as u32
        );
        assert_eq!(view.opponents[0].hand.unknown, 5);
        assert_eq!(view.supply.len(), 17);
        assert_eq!(
            view.decision.is_some(),
            game.pending().unwrap().player == PlayerId(seat)
        );
    }
}

#[test]
fn paired_hidden_worlds_have_identical_wire_bytes_including_question_metadata() {
    let game = Game::new(config()).unwrap();
    let catalog = catalog();
    let base = update(&game, 0, &catalog, 12);
    let mut state = serde_json::to_value(&game).unwrap();
    // Permute every draw pile and exchange distinct unseen cards between opponent
    // hand and deck. Neither identity nor original allocation number may reach wire.
    state["players"][0]["deck"]
        .as_array_mut()
        .unwrap()
        .reverse();
    state["players"][1]["deck"]
        .as_array_mut()
        .unwrap()
        .reverse();
    let hand = game.hand(PlayerId(1));
    let deck = &game.player(PlayerId(1)).deck;
    let pair = hand
        .iter()
        .flat_map(|a| deck.iter().map(move |b| (*a, *b)))
        .find(|(a, b)| game.card(*a) != game.card(*b))
        .unwrap();
    let hi = hand.iter().position(|id| *id == pair.0).unwrap();
    let di = deck.iter().rev().position(|id| *id == pair.1).unwrap();
    state["players"][1]["hand"][hi] = json!(pair.1);
    state["players"][1]["deck"][di] = json!(pair.0);
    state["zones"][pair.0.index()] = json!({"Deck":1});
    state["zones"][pair.1.index()] = json!({"Hand":1});
    let other: Game = serde_json::from_value(state).unwrap();
    assert_eq!(bytes(&base), bytes(&update(&other, 0, &catalog, 12)));
    // Even wholesale hidden allocation swaps do not affect the public namespace.
    let mut state = serde_json::to_value(&other).unwrap();
    let a = pair.0.index();
    let b = pair.1.index();
    let ca = state["cards"][a].clone();
    state["cards"][a] = state["cards"][b].clone();
    state["cards"][b] = ca;
    let another: Game = serde_json::from_value(state).unwrap();
    assert_eq!(bytes(&base), bytes(&update(&another, 0, &catalog, 12)));
}

#[test]
fn exact_action_and_buy_questions_round_trip_every_legal_move() {
    let catalog = catalog();
    for game in [
        action_game(&[Card::Village, Card::Smithy, Card::Copper]),
        Game::new(config()).unwrap(),
    ] {
        let update = update(&game, 0, &catalog, 47);
        let receiver = receive(&update, &catalog, 0);
        let decision = receiver.reducer.view().unwrap().decision.unwrap();
        let mut actual = Vec::new();
        for option in &decision.options {
            let response = answer(&update, &catalog, vec![option.index]);
            game.pending().unwrap().validate(&response).unwrap();
            if !actual.contains(&response) {
                actual.push(response);
            }
        }
        let DecisionKind::Turn { moves, .. } = &game.pending().unwrap().kind else {
            panic!()
        };
        assert_eq!(actual.len(), moves.len());
        for &m in moves {
            assert!(actual.contains(&Response::Move(m)));
        }
        let q = update.question.as_ref().unwrap();
        assert_eq!(q.resolve(46, &[]), Err(AnswerRejection::StaleQuestion));
        for values in [
            vec![-1],
            vec![9999],
            vec![0, 0, 0],
            vec![1, 1, 0],
            vec![0, -1],
        ] {
            assert!(q.resolve(47, &values).is_err(), "{values:?}");
        }
        if game.turn().phase == Phase::Action {
            assert_eq!(decision.prompt, "You may play Actions");
        } else {
            assert!(
                decision.prompt.starts_with(
                    "You may buy cards Answer from one alternative only: (a) options 0-"
                ) && decision.prompt.contains("; (b) options "),
                "{}",
                decision.prompt
            );
        }
    }
}

#[test]
fn cellar_and_chapel_questions_preserve_copies_bounds_and_empty_answers() {
    let catalog = catalog();
    for (card, prompt) in [
        (Card::Cellar, "Discard, then draw"),
        (Card::Chapel, "Trash up to 4 cards"),
    ] {
        let mut game = action_game(&[card, Card::Copper, Card::Copper, Card::Estate]);
        play(&mut game, card);
        let update = update(&game, 0, &catalog, 51);
        let receiver = receive(&update, &catalog, 0);
        let decision = receiver.reducer.view().unwrap().decision.unwrap();
        assert_eq!(decision.prompt, prompt);
        let selected: Vec<_> = decision
            .options
            .iter()
            .filter(|o| o.card.as_deref() == Some("Copper"))
            .map(|o| o.index)
            .collect();
        assert_eq!(selected.len(), 2);
        let response = answer(&update, &catalog, selected);
        let Response::Cards(cards) = &response else {
            panic!()
        };
        assert_eq!(cards.len(), 2);
        assert_ne!(cards[0], cards[1]);
        game.pending().unwrap().validate(&response).unwrap();
        assert_eq!(answer(&update, &catalog, vec![]), Response::Cards(vec![]));
        let q = update.question.as_ref().unwrap();
        assert!(q.resolve(51, &[0, 0]).is_err());
        assert!(q.resolve(51, &[999]).is_err());
        assert!(q.resolve(51, &[0, 1, 2, 3, 4]).is_err());
        let before = game.hand(PlayerId(0)).len();
        game.respond(response).unwrap();
        if card == Card::Cellar {
            assert_eq!(game.hand(PlayerId(0)).len(), before);
        } else {
            assert_eq!(game.trash().len(), 2);
        }
        // The non-deciding seat never receives another player's private question.
        assert!(update_for_other(&game, &catalog).question.is_none());
    }
}
fn update_for_other(game: &Game, catalog: &WireCatalog) -> ProjectedUpdate {
    update(game, 1, catalog, 99)
}

#[test]
fn workshop_gain_uses_pile_targets_not_card_ordinals() {
    let catalog = catalog();
    let mut game = action_game(&[Card::Workshop, Card::Copper]);
    play(&mut game, Card::Workshop);
    let update = update(&game, 0, &catalog, 53);
    let receiver = receive(&update, &catalog, 0);
    let d = receiver.reducer.view().unwrap().decision.unwrap();
    assert_eq!(d.prompt, "Gain a card");
    for option in &d.options {
        let response = answer(&update, &catalog, vec![option.index]);
        game.pending().unwrap().validate(&response).unwrap();
    }
    let village = d
        .options
        .iter()
        .find(|o| o.card.as_deref() == Some("Village"))
        .unwrap();
    let response = answer(&update, &catalog, vec![village.index]);
    assert_eq!(response, Response::Pile(game.supply_pile(Card::Village)));
    assert!(update.question.as_ref().unwrap().resolve(53, &[]).is_err());
    game.respond(response).unwrap();
    assert_eq!(
        game.card(*game.player(PlayerId(0)).discard.last().unwrap()),
        Card::Village
    );
}

#[test]
fn unsupported_decisions_are_refused_instead_of_scraping_labels_or_modes() {
    let catalog = catalog();
    let game = action_game(&[Card::Village, Card::Copper]);
    let update = update(&game, 0, &catalog, 1);
    let mut identities = BTreeMap::new();
    for id in game.hand(PlayerId(0)) {
        if let Some(card) = game.view(PlayerId(0)).card(*id)
            && let Some(wire) = update
                .snapshot
                .state
                .cards
                .iter()
                .find(|c| c.card_name == catalog.card(card).unwrap())
        {
            identities.insert(*id, wire.id);
        }
    }
    let mut d = game.pending().unwrap().clone();
    for kind in [
        DecisionKind::NameCard,
        DecisionKind::Number { min: 0, max: 5 },
        DecisionKind::Choose {
            options: vec![dominion_engine::Opt::Mode(0)],
            labels: vec!["Play Village".into()],
            min: 1,
            max: 1,
        },
        DecisionKind::Cards {
            candidates: game.hand(PlayerId(0)).to_vec(),
            min: 1,
            max: 2,
            ordered: true,
        },
    ] {
        d.kind = kind;
        assert!(matches!(
            questions::prepare(
                &game.view(PlayerId(0)),
                &d,
                "d",
                1,
                &identities,
                &BTreeMap::new(),
                &catalog
            ),
            Err(ProjectionError::Compatibility(_))
        ));
    }
}

#[test]
fn terminal_results_keep_authoritative_ties_turn_tiebreaks_and_resignation() {
    let catalog = catalog();
    let game = Game::new(config()).unwrap();
    let piles = vec![];
    for outcome in [
        Outcome {
            reason: EndReason::PilesEmpty,
            scores: vec![12, 12],
            turns: vec![9, 9],
            winners: vec![PlayerId(0), PlayerId(1)],
            resigned: vec![],
        },
        Outcome {
            reason: EndReason::ProvincesGone,
            scores: vec![12, 12],
            turns: vec![10, 9],
            winners: vec![PlayerId(1)],
            resigned: vec![],
        },
        Outcome {
            reason: EndReason::Resigned,
            scores: vec![100, 0],
            turns: vec![2, 1],
            winners: vec![PlayerId(1)],
            resigned: vec![PlayerId(0)],
        },
    ] {
        let expected = result(&outcome, None, &ids(), &piles).unwrap();
        let mut state = serde_json::to_value(&game).unwrap();
        state["outcome"] = serde_json::to_value(&outcome).unwrap();
        state["pending"] = Value::Null;
        let finished: Game = serde_json::from_value(state).unwrap();
        let update = update(&finished, 0, &catalog, 1);
        assert!(update.question.is_none());
        assert_eq!(update.finished.as_ref().unwrap().players, expected.players);
        let receiver = receive(&update, &catalog, 0);
        let view = receiver.reducer.view().unwrap();
        assert!(!view.game.running);
        assert!(view.decision.is_none());
        let result = view.game.result.unwrap();
        for (i, p) in result.players.iter().enumerate() {
            assert_eq!(p.vp, outcome.scores[i]);
            assert_eq!(p.turns, outcome.turns[i]);
            assert_eq!(p.rank, expected.players[i].rank as u32);
            assert_eq!(p.resigned, outcome.resigned.contains(&PlayerId(i as u8)));
        }
    }
}

fn assert_receiver_counters(game: &Game, catalog: &WireCatalog) {
    for seat in 0..2 {
        let projected = update(game, seat, catalog, 63);
        let receiver = receive(&projected, catalog, seat);
        let view = receiver.reducer.view().unwrap();
        assert_eq!(view.turn.actions, Some(game.turn().actions));
        assert_eq!(view.turn.buys, Some(game.turn().buys));
        assert_eq!(view.turn.coins, Some(game.turn().coins));
        assert_eq!(view.turn.potions, Some(game.turn().potions));
        assert_eq!(
            view.turn.number,
            Some(game.player(game.turn().player).turns_taken)
        );
        assert_eq!(
            view.you.unwrap().hand.count,
            game.hand(PlayerId(seat)).len() as u32
        );
    }
}

#[test]
fn all_ten_cards_project_after_real_engine_effects_including_merchant_and_council_room() {
    let catalog = catalog();
    for (card, draw, actions, buys, coins) in [
        (Card::Village, 1, 2, 1, 0),
        (Card::Smithy, 3, 0, 1, 0),
        (Card::Market, 1, 1, 2, 1),
        (Card::Festival, 0, 2, 2, 2),
        (Card::Laboratory, 2, 1, 1, 0),
        (Card::Merchant, 1, 1, 1, 0),
        (Card::CouncilRoom, 4, 0, 2, 0),
    ] {
        let mut game = action_game(&[card, Card::Copper, Card::Silver]);
        let other = game.hand(PlayerId(1)).len();
        play(&mut game, card);
        assert_eq!(game.hand(PlayerId(0)).len(), 2 + draw, "{}", card.name());
        assert_eq!(
            (game.turn().actions, game.turn().buys, game.turn().coins),
            (actions, buys, coins),
            "{}",
            card.name()
        );
        assert_eq!(
            game.hand(PlayerId(1)).len(),
            other + usize::from(card == Card::CouncilRoom)
        );
        assert_receiver_counters(&game, &catalog);
    }
    let mut merchant = action_game(&[Card::Merchant, Card::Silver, Card::Silver]);
    play(&mut merchant, Card::Merchant);
    if merchant.turn().phase == Phase::Action {
        merchant.respond(Response::Move(Move::EndPhase)).unwrap();
    }
    play(&mut merchant, Card::Silver);
    assert_eq!(merchant.turn().coins, 3);
    assert_receiver_counters(&merchant, &catalog);
    play(&mut merchant, Card::Silver);
    assert_eq!(merchant.turn().coins, 5);
    assert_receiver_counters(&merchant, &catalog);
    // Cellar, Chapel and Workshop each hold an actual interruptible engine frame.
    for card in [Card::Cellar, Card::Chapel, Card::Workshop] {
        let mut game = action_game(&[card, Card::Copper, Card::Estate]);
        play(&mut game, card);
        assert_receiver_counters(&game, &catalog);
    }
}

#[test]
fn disclosed_cards_become_anonymous_again_after_cleanup_and_shuffle() {
    let catalog = catalog();
    let mut game = action_game(&[Card::Village, Card::Copper, Card::Estate]);
    let village = game.hand(PlayerId(0))[0];
    play(&mut game, Card::Village);
    let visible = update(&game, 1, &catalog, 1);
    let village_name = catalog.card(Card::Village).unwrap();
    assert_eq!(
        visible
            .snapshot
            .state
            .cards
            .iter()
            .filter(|c| c.card_name == village_name)
            .count(),
        2
    );
    let mut shuffled = false;
    let mut hidden = false;
    for _ in 0..40 {
        let response = match &game.pending().unwrap().kind {
            DecisionKind::Turn { .. } => Response::Move(Move::EndPhase),
            _ => panic!("unexpected decision"),
        };
        game.respond(response).unwrap();
        shuffled = shuffled
            || game.log().iter().any(|e| {
                matches!(
                    e,
                    dominion_engine::Event::Shuffled {
                        player: PlayerId(0)
                    }
                )
            });
        if shuffled
            && matches!(
                game.zone(village),
                dominion_engine::Zone::Deck(PlayerId(0)) | dominion_engine::Zone::Hand(PlayerId(0))
            )
        {
            hidden = true;
            break;
        }
    }
    assert!(shuffled && hidden);
    let renewed = update(&game, 1, &catalog, 2);
    assert_eq!(
        renewed
            .snapshot
            .state
            .cards
            .iter()
            .filter(|c| c.card_name == village_name)
            .count(),
        1
    );
    let mut receiver = receive(&visible, &catalog, 1);
    receiver.apply_batch(messages(&renewed)).unwrap();
    let view = receiver.reducer.view().unwrap();
    let opponent = &view.opponents[0];
    assert_eq!(opponent.hand.unknown, opponent.hand.count);
    assert_eq!(opponent.deck.unknown, opponent.deck.count);
    assert!(!opponent.owned.known.contains_key("Village"));
    // A different hidden draw order produces the same replacement snapshot bytes.
    let mut state = serde_json::to_value(&game).unwrap();
    state["players"][0]["deck"]
        .as_array_mut()
        .unwrap()
        .reverse();
    let twin: Game = serde_json::from_value(state).unwrap();
    assert_eq!(bytes(&renewed), bytes(&update(&twin, 1, &catalog, 2)));
}

#[test]
fn finished_attach_and_reconnect_preserve_final_facts_without_deck_breakdowns() {
    let catalog = catalog();
    let mut game = Game::new(config()).unwrap();
    game.resign(PlayerId(1));
    let initial = update(&game, 0, &catalog, 1);
    let receiver = receive(&initial, &catalog, 0);
    let before = receiver.reducer.view().unwrap();
    let mut replacement = update(&game, 0, &catalog, 2);
    replacement.snapshot.reconnect = true;
    let mut receiver = receiver;
    receiver.apply_batch(messages(&replacement)).unwrap();
    let after = receiver.reducer.view().unwrap();
    assert!(!after.game.running);
    assert_eq!(before.game.result, after.game.result);
    assert!(after.decision.is_none());
    assert!(
        replacement
            .finished
            .unwrap()
            .players
            .iter()
            .all(|p| p.card_names.is_empty() && p.score.parts.is_empty())
    );
    assert_eq!(initial.table.players.len(), initial.table.ready.len());
    assert!(
        initial.table.spectators.is_empty()
            && initial.table.bots.is_empty()
            && initial.table.reconnecting.is_empty()
    );
}

#[test]
fn supported_games_complete_through_encoded_snapshots_and_semantic_answers() {
    let catalog = catalog();
    for seed in [3, 19] {
        let mut cfg = config();
        cfg.seed = seed;
        let mut game = Game::new(cfg).unwrap();
        let mut receivers = [
            receive(&update(&game, 0, &catalog, 1), &catalog, 0),
            receive(&update(&game, 1, &catalog, 1), &catalog, 1),
        ];
        let mut bought = [BTreeSet::<String>::new(), BTreeSet::<String>::new()];
        let mut ended = false;
        for index in 2..8000 {
            let Some(pending) = game.pending() else {
                ended = true;
                break;
            };
            let seat = pending.player.0;
            let projected = update(&game, seat, &catalog, index);
            let receiver = &mut receivers[seat as usize];
            receiver
                .apply_batch(Decoder::new().decode(&bytes(&projected)).unwrap())
                .unwrap();
            let decision = receiver.reducer.view().unwrap().decision.unwrap();
            let options = &decision.options;
            let selected: Vec<u32> = if matches!(pending.purpose, Purpose::Trash | Purpose::Discard)
            {
                options
                    .iter()
                    .filter(|o| matches!(o.card.as_deref(), Some("Estate" | "Curse")))
                    .map(|o| o.index)
                    .collect()
            } else if pending.purpose == Purpose::Gain {
                vec![
                    options
                        .iter()
                        .find(|o| o.card.as_deref() == Some("Silver"))
                        .unwrap_or(&options[0])
                        .index,
                ]
            } else if let Some(play) = options.iter().find(|o| o.action == "play") {
                vec![play.index]
            } else {
                let wanted = options
                    .iter()
                    .find(|o| o.action == "buy" && o.card.as_deref() == Some("Province"))
                    .or_else(|| {
                        options.iter().find(|o| {
                            o.action == "buy"
                                && o.card.as_ref().is_some_and(|name| {
                                    KINGDOM.iter().any(|c| c.name() == name)
                                        && !bought[seat as usize].contains(name)
                                })
                        })
                    })
                    .or_else(|| {
                        options
                            .iter()
                            .find(|o| o.action == "buy" && o.card.as_deref() == Some("Gold"))
                    })
                    .or_else(|| {
                        options
                            .iter()
                            .find(|o| o.action == "buy" && o.card.as_deref() == Some("Silver"))
                    })
                    .or_else(|| options.iter().find(|o| o.action == "end-phase"))
                    .unwrap();
                if wanted.action == "buy" {
                    bought[seat as usize].insert(wanted.card.clone().unwrap());
                }
                vec![wanted.index]
            };
            let (question, values) = receiver
                .reducer
                .answer(&Answer::indices(&decision.id, selected), None)
                .unwrap();
            let response = projected
                .question
                .as_ref()
                .unwrap()
                .resolve(question, &values)
                .unwrap();
            pending.validate(&response).unwrap();
            game.respond(response).unwrap();
        }
        assert!(ended, "seed {seed} failed to finish");
        assert!(matches!(
            game.outcome().unwrap().reason,
            EndReason::ProvincesGone | EndReason::PilesEmpty
        ));
        for (seat, receiver) in receivers.iter_mut().enumerate() {
            let projected = update(&game, seat as u8, &catalog, 9000);
            receiver
                .apply_batch(Decoder::new().decode(&bytes(&projected)).unwrap())
                .unwrap();
            let result = receiver.reducer.view().unwrap();
            assert!(!result.game.running);
            assert!(result.decision.is_none());
            let result = result.game.result.unwrap();
            for (p, expected) in result.players.iter().zip(&game.outcome().unwrap().scores) {
                assert_eq!(p.vp, *expected);
            }
        }
    }
}

#[test]
fn discard_bottom_identities_and_order_never_enter_raw_snapshot_allocations() {
    let game = Game::new(config()).unwrap();
    let catalog = catalog();
    let mut state = serde_json::to_value(&game).unwrap();
    let mut discard = Vec::new();
    for _ in 0..3 {
        let id = state["players"][1]["deck"]
            .as_array_mut()
            .unwrap()
            .pop()
            .unwrap();
        state["zones"][id.as_u64().unwrap() as usize] = json!({"Discard":1});
        discard.push(id);
    }
    state["players"][1]["discard"] = json!(discard);
    let before: Game = serde_json::from_value(state.clone()).unwrap();
    state["players"][1]["discard"]
        .as_array_mut()
        .unwrap()
        .swap(0, 1);
    let a = discard[0].as_u64().unwrap() as usize;
    let b = discard[1].as_u64().unwrap() as usize;
    let name = state["cards"][a].clone();
    state["cards"][a] = state["cards"][b].clone();
    state["cards"][b] = name;
    let after: Game = serde_json::from_value(state).unwrap();
    let projected = update(&before, 0, &catalog, 1);
    assert_eq!(bytes(&projected), bytes(&update(&after, 0, &catalog, 1)));
    let discard = projected
        .snapshot
        .state
        .zones
        .iter()
        .find(|z| z.kind == zone::DISCARD && z.owner == 1)
        .unwrap();
    assert_eq!(discard.cards.len(), 3);
    assert!(discard.cards[0] >= 0);
    assert_eq!(&discard.cards[1..], &[-1, -1]);
}

#[test]
fn spectators_invalid_fixed_identities_and_oversized_public_values_are_refused() {
    let game = Game::new(config()).unwrap();
    let catalog = catalog();
    let known = KnownCards::default();
    let names = ["Ada".into(), "Bob".into()];
    let observed = |view| hub::SeatObservation {
        clock: None,
        view,
        known: &known,
        names: &names,
        decision_id: Some("d"),
        deciding: game.pending().map(|d| d.player),
        version: 0,
        undos: 0,
        outcome: None,
        ended: None,
    };
    assert!(project(observed(game.watch()), &ids(), &catalog, 1, false).is_err());
    for identity in [
        WireIdentity {
            player_ids: vec![1, 1],
            ..ids()
        },
        WireIdentity {
            game_id: 9_007_199_254_740_992,
            ..ids()
        },
    ] {
        assert!(
            project(
                observed(game.view(PlayerId(0))),
                &identity,
                &catalog,
                1,
                false
            )
            .is_err()
        );
    }
    assert!(project(observed(game.view(PlayerId(0))), &ids(), &catalog, 0, false).is_err());
    assert!(integer(65_537, "zone").is_err());
}

#[test]
fn a_clock_reaches_the_client_as_its_timer_preset_and_each_players_time() {
    let catalog = catalog();
    let game = Game::new(config()).unwrap();
    let (_, friendly) = dominion_protocol::lobby::Clock::PRESETS[1];
    let update = project(
        hub::SeatObservation {
            clock: Some((
                friendly,
                ClockView {
                    left: vec![60_000, 30_000],
                    free: vec![],
                    running: Some(1),
                    increment: None,
                },
            )),
            view: game.view(PlayerId(0)),
            known: &KnownCards::default(),
            names: &["Ada".into(), "Bob".into()],
            decision_id: game.pending().map(|_| "engine-d1"),
            deciding: game.pending().map(|d| d.player),
            version: 7,
            undos: 2,
            outcome: game.outcome(),
            ended: None,
        },
        &ids(),
        &catalog,
        1,
        false,
    )
    .unwrap();
    let timer = &update.snapshot.table_options.timer_settings;
    assert_eq!(
        timer.preset,
        catalog.symbol("TimerPresetOptions", "FRIENDLY").unwrap()
    );
    assert!(timer.force_resign);
    let times: Vec<_> = (update.timers.iter())
        .map(|t| (t.player, t.time, t.running))
        .collect();
    assert_eq!(times, [(10, 60_000, false), (20, 30_000, true)]);
    let bytes = ServerEncoder::new()
        .encode_batch(&[Message::Timers(update.timers.clone())])
        .unwrap();
    let decoded = Decoder::new().decode(&bytes).unwrap();
    assert!(
        matches!(&decoded[..], [Message::Timers(t)] if t.len() == 2 && t[1].running),
        "{decoded:?}"
    );
    // Without a clock the timer is off and no times are sent.
    let plain = self::update(&game, 0, &catalog, 1);
    assert_eq!(
        plain.snapshot.table_options.timer_settings.preset,
        catalog.symbol("TimerPresetOptions", "OFF").unwrap()
    );
    assert!(plain.timers.is_empty());
}

#[test]
fn split_piles_carry_their_marker_to_their_supply_zone() {
    let catalog = catalog();
    let mut kingdom = KINGDOM.to_vec();
    kingdom[0] = Card::Settlers;
    let game = Game::new(GameConfig::new(2, kingdom, 42)).unwrap();
    let state = update(&game, 0, &catalog, 1).snapshot.state;
    let marked: Vec<(i32, i32)> = state
        .pile_markers
        .iter()
        .map(|m| (m.kind, state.zones[m.zone_index as usize].pile_name))
        .collect();
    let split = super::super::catalog::pile_marker(dominion_protocol::PileMarker::Split);
    let settlers = catalog.card(Card::Settlers).unwrap();
    assert_eq!(marked, vec![(split, settlers)]);
    // Anyone may look through it, so its cards go by name: their client shadows it.
    let zone = &state.zones[state.pile_markers[0].zone_index as usize];
    let mut names: Vec<i32> = zone
        .cards
        .iter()
        .map(|&id| state.cards[id as usize].card_name)
        .collect();
    names.dedup();
    let bustling = catalog.card(Card::BustlingVillage).unwrap();
    assert_eq!(names, vec![bustling, settlers]);
}

#[test]
fn the_bane_is_marked_and_traits_attach_to_their_pile() {
    let catalog = catalog();
    let mut kingdom = KINGDOM.to_vec();
    kingdom[0] = Card::YoungWitch;
    let mut config = GameConfig::new(2, kingdom, 42);
    config.landscapes.push(Card::Cheap);
    let game = Game::new(config).unwrap();
    let view = game.view(PlayerId(0));
    let bane = view
        .notes()
        .iter()
        .find(|n| n.setup == Some(dominion_engine::SetupRole::Bane))
        .and_then(|n| n.pile)
        .unwrap();
    let cheap = view.piles().find(|p| p.traits() == [Card::Cheap]).unwrap();
    let (bane, cheap) = (
        catalog.card(view.pile(bane).card()).unwrap(),
        catalog.card(cheap.card()).unwrap(),
    );
    let state = update(&game, 0, &catalog, 1).snapshot.state;
    let marked: Vec<(i32, i32)> = state
        .pile_markers
        .iter()
        .map(|m| (m.kind, state.zones[m.zone_index as usize].pile_name))
        .collect();
    let mark = super::super::catalog::pile_marker(dominion_protocol::PileMarker::Bane);
    assert_eq!(marked, vec![(mark, bane)]);
    let traits: Vec<(i32, i32)> = state
        .zones
        .iter()
        .filter(|z| z.attached_trait != 0)
        .map(|z| (z.pile_name, z.attached_trait))
        .collect();
    assert_eq!(traits, vec![(cheap, catalog.card(Card::Cheap).unwrap())]);
    // Their client draws the Trait's landscape with a card of its pile linked to it.
    let named = |id: i32| state.cards[id as usize].card_name;
    let links: Vec<(i32, Vec<i32>)> = state
        .associations
        .iter()
        .map(|a| {
            (
                named(a.card),
                a.linked_cards.iter().map(|&l| named(l)).collect(),
            )
        })
        .collect();
    assert_eq!(
        links,
        vec![(cheap, vec![catalog.card(Card::Cheap).unwrap()])]
    );
}

fn kingdom_with(cards: &[Card]) -> Vec<Card> {
    let mut kingdom = KINGDOM.to_vec();
    kingdom[..cards.len()].copy_from_slice(cards);
    kingdom
}

fn zone_of(state: &model::GameState, kind: i32) -> Option<&model::Zone> {
    state.zones.iter().find(|z| z.kind == kind)
}

fn names_in(state: &model::GameState, zone: &model::Zone) -> Vec<i32> {
    zone.cards
        .iter()
        .map(|&id| state.cards[id as usize].card_name)
        .collect()
}

#[test]
fn boons_and_hexes_lie_in_their_decks_whose_order_stays_hidden() {
    let catalog = catalog();
    let game = Game::new(GameConfig::new(
        2,
        kingdom_with(&[Card::Bard, Card::Leprechaun]),
        42,
    ))
    .unwrap();
    let mut state = serde_json::to_value(&game).unwrap();
    for deck in ["boons", "hexes"] {
        let draw = state[deck]["draw"].as_array_mut().unwrap();
        let discarded: Vec<Value> = draw.drain(..2).collect();
        state[deck]["discard"] = Value::Array(discarded);
    }
    let game: Game = serde_json::from_value(state).unwrap();
    let mut twin = serde_json::to_value(&game).unwrap();
    for deck in ["boons", "hexes"] {
        twin[deck]["draw"].as_array_mut().unwrap().reverse();
    }
    let twin: Game = serde_json::from_value(twin).unwrap();
    let view = game.view(PlayerId(0));
    for seat in 0..2 {
        let update = update(&game, seat, &catalog, 3);
        assert_eq!(
            bytes(&update),
            bytes(&super::tests::update(&twin, seat, &catalog, 3))
        );
        let state = &update.snapshot.state;
        for (deck, draw, discard, kind) in [
            (
                view.boons(),
                "Boon Drawpile",
                "Boon Discardpile",
                zone::BOON_DISCARD,
            ),
            (
                view.hexes(),
                "Hex Drawpile",
                "Hex Discardpile",
                zone::HEX_DISCARD,
            ),
        ] {
            let pile = state
                .zones
                .iter()
                .find(|z| z.kind == zone::NON_SUPPLY && z.pile_name == catalog.named(draw).unwrap())
                .unwrap();
            assert_eq!(pile.cards, vec![-1; deck.draw_size]);
            let discards = zone_of(state, kind).unwrap();
            assert_eq!(discards.pile_name, catalog.named(discard).unwrap());
            let expected: Vec<i32> = deck
                .discard
                .iter()
                .map(|&c| catalog.card(c).unwrap())
                .collect();
            assert_eq!(names_in(state, discards), expected);
        }
        // Every Boon and Hex is among the game's cards, which their client lists.
        for card in [
            Card::TheSunsGift,
            Card::Famine,
            Card::Deluded,
            Card::Miserable,
        ] {
            assert!(state.card_names.contains(&catalog.card(card).unwrap()));
        }
        receive(&update, &catalog, seat).reducer.view().unwrap();
    }
}

#[test]
fn states_kept_boons_artifacts_and_tokens_reach_the_receiver() {
    let catalog = catalog();
    let game = Game::new(GameConfig::new(
        2,
        kingdom_with(&[Card::Bard, Card::FlagBearer, Card::Leprechaun]),
        42,
    ))
    .unwrap();
    let view = game.view(PlayerId(0));
    let pile = |card: Card| view.supply_pile(card).unwrap();
    let (village, smithy) = (pile(KINGDOM[5]), pile(KINGDOM[6]));
    let mut state = serde_json::to_value(&game).unwrap();
    let card = |c: Card| serde_json::to_value(c).unwrap();
    state["players"][0]["states"] = json!([card(Card::Deluded)]);
    state["players"][0]["boons_kept"] = json!([card(Card::TheFieldsGift)]);
    state["players"][1]["minus_card"] = json!(true);
    state["players"][1]["minus_coin"] = json!(true);
    state["players"][1]["tokens"] = json!({ "PlusAction": village.0 });
    state["piles"][village.index()]["tokens"] = json!([[1, "PlusAction"]]);
    state["piles"][smithy.index()]["embargo"] = json!(2);
    state["piles"][smithy.index()]["trade_route"] = json!(true);
    for artifact in state["artifacts"].as_array_mut().unwrap() {
        artifact[1] = json!(0);
    }
    let game: Game = serde_json::from_value(state).unwrap();
    for seat in 0..2 {
        let update = update(&game, seat, &catalog, 3);
        let receiver = receive(&update, &catalog, seat);
        let view = receiver.reducer.view().unwrap();
        let players: Vec<_> = view.you.iter().chain(&view.opponents).collect();
        let first = players.iter().find(|p| p.index == 0).unwrap();
        let second = players.iter().find(|p| p.index == 1).unwrap();
        assert_eq!(first.states, vec!["Deluded".to_string()]);
        assert_eq!(first.boons, vec!["The Field's Gift".to_string()]);
        assert_eq!(first.artifacts, vec!["Flag".to_string()]);
        assert!(second.tokens.minus_card && second.tokens.minus_coin);
        assert!(!first.tokens.minus_card);
        assert_eq!(second.adventure_tokens.len(), 1);
        let on = |card: Card| {
            view.supply
                .iter()
                .find(|p| p.card == card.name())
                .unwrap()
                .tokens
                .clone()
        };
        assert_eq!(on(KINGDOM[5]).players.len(), 1);
        assert_eq!(on(KINGDOM[6]).embargo, 2);
        assert!(on(KINGDOM[6]).trade_route);
    }
}

#[test]
fn setup_set_asides_lie_in_their_zones_and_riverboats_card_is_linked() {
    let catalog = catalog();
    let mut config = GameConfig::new(2, kingdom_with(&[Card::Druid, Card::Riverboat]), 42);
    config.landscapes.push(Card::WayOfTheMouse);
    let game = Game::new(config).unwrap();
    let notes = game.view(PlayerId(0)).notes();
    let set_aside = |source: Card| -> Vec<i32> {
        let mut cards: Vec<i32> = notes
            .iter()
            .filter(|n| n.source == source)
            .filter_map(|n| n.card)
            .map(|c| catalog.card(c).unwrap())
            .collect();
        cards.sort();
        cards
    };
    for seat in 0..2 {
        let update = update(&game, seat, &catalog, 3);
        let state = &update.snapshot.state;
        for (kind, source) in [
            (zone::DRUID_BOONS, Card::Druid),
            (zone::MOUSE, Card::WayOfTheMouse),
            (zone::THE_BOX, Card::Riverboat),
        ] {
            let mut names = names_in(state, zone_of(state, kind).unwrap());
            names.sort();
            assert_eq!(names, set_aside(source), "{}", source.name());
        }
        let boxed = zone_of(state, zone::THE_BOX).unwrap().cards[0];
        let riverboat = catalog.card(Card::Riverboat).unwrap();
        assert!(state.associations.iter().any(|a| {
            a.card == boxed
                && a.linked_cards
                    .iter()
                    .all(|&l| state.cards[l as usize].card_name == riverboat)
        }));
        let view = receive(&update, &catalog, seat).reducer.view().unwrap();
        let mut sources: Vec<&str> = view
            .notes
            .iter()
            .filter(|n| n.setup.is_some())
            .map(|n| n.source.as_str())
            .collect();
        sources.sort();
        assert_eq!(
            sources,
            ["Druid", "Druid", "Druid", "Riverboat", "Way of the Mouse"]
        );
    }
}

#[test]
fn the_prophecys_suns_and_landmark_points_are_counters_on_their_landscapes() {
    let catalog = catalog();
    let mut config = GameConfig::new(2, kingdom_with(&[Card::RusticVillage]), 42);
    config.landscapes.extend([Card::GreatLeader, Card::Arena]);
    let game = Game::new(config).unwrap();
    let view = game.view(PlayerId(0));
    let (_, suns) = view.prophecy().unwrap();
    let update = update(&game, 0, &catalog, 3);
    let view = receive(&update, &catalog, 0).reducer.view().unwrap();
    let landscape = |card: Card| {
        view.landscapes
            .iter()
            .find(|l| l.card == card.name())
            .unwrap()
    };
    assert_eq!(landscape(Card::GreatLeader).suns, Some(suns));
    assert_eq!(landscape(Card::Arena).vp, Some(12));
}

/// Random games with every kind of table state: each seat not deciding is projected at
/// every decision (no question, so only the state), and their client's model reads it.
#[test]
fn random_games_project_the_table_state_for_every_waiting_seat() {
    use super::survey::{Rng, deal, engine_answer};
    let catalog = catalog();
    let games: &[(&[Card], &[Card])] = &[
        (
            &[
                Card::Bard,
                Card::Druid,
                Card::Fool,
                Card::Leprechaun,
                Card::Skulk,
            ],
            &[],
        ),
        (
            &[
                Card::FlagBearer,
                Card::Swashbuckler,
                Card::Treasurer,
                Card::BorderGuard,
            ],
            &[Card::SinisterPlot, Card::Fleet],
        ),
        (
            &[Card::BridgeTroll, Card::Relic, Card::Ranger, Card::Embargo],
            &[Card::LostArts, Card::Seaway, Card::Mission],
        ),
        (
            &[
                Card::TradeRoute,
                Card::Riverboat,
                Card::RusticVillage,
                Card::Outpost,
            ],
            &[Card::WayOfTheMouse, Card::Arena, Card::SeizeTheDay],
        ),
        (
            &[Card::Wizards, Card::Odysseys, Card::Giant],
            &[Card::FamilyOfInventors, Card::Plan, Card::Ferry],
        ),
    ];
    let mut projected = 0;
    let mut seen = BTreeSet::new();
    for (n, (cards, landscapes)) in games.iter().enumerate() {
        for seed in 0..4 {
            let mut config = GameConfig::new(2, kingdom_with(cards), 500 + seed);
            config.landscapes.extend_from_slice(landscapes);
            let favoured = cards[seed as usize % cards.len()];
            let mut game = deal(Game::new(config).unwrap(), favoured);
            let mut rng = Rng(seed * 31 + n as u64);
            for _ in 0..600 {
                let Some(decider) = game.pending().map(|d| d.player) else {
                    break;
                };
                let view = game.view(decider);
                if view.players().any(|p| {
                    p.hand_size() + p.deck_size() + p.discard_size() + p.in_play().len() < 5
                }) {
                    break;
                }
                let waiting = PlayerId(1 - decider.0);
                let observation = hub::SeatObservation {
                    clock: None,
                    view: game.view(waiting),
                    known: &KnownCards::default(),
                    names: &["Ada".into(), "Bob".into()],
                    decision_id: Some("engine-d1"),
                    deciding: Some(decider),
                    version: 7,
                    undos: 0,
                    outcome: None,
                    ended: None,
                };
                match project(observation, &ids(), &catalog, 1, false) {
                    Ok(update) => {
                        receive(&update, &catalog, waiting.0)
                            .reducer
                            .view()
                            .unwrap();
                        projected += 1;
                        let state = &update.snapshot.state;
                        seen.extend(
                            state
                                .zones
                                .iter()
                                .filter(|z| !z.cards.is_empty())
                                .map(|z| z.kind),
                        );
                        seen.extend(state.counters.iter().map(|c| 100 + c.kind));
                        seen.extend(state.tokens.iter().map(|t| 200 + t.kind));
                        if state.players.iter().any(|p| !p.reminders.is_empty()) {
                            seen.insert(300);
                        }
                    }
                    Err(ProjectionError::UnsupportedState(gap)) => {
                        panic!("{gap} in game {n}, seed {seed}")
                    }
                    Err(e) => panic!("{e} in game {n}, seed {seed}"),
                }
                let answer = engine_answer(&game, favoured, &mut rng);
                game.respond(answer).unwrap();
            }
        }
    }
    assert!(projected > 1000, "{projected}");
    // Boon and Hex discards, Druid's Boons, Way of the Mouse's card, the Box; Embargo,
    // Trade Route, Sinister Plot and Sun counters; -1 Card and -$1 tokens; reminders.
    for kind in [15, 16, 20, 22, 30, 109, 110, 113, 118, 207, 208, 300] {
        assert!(seen.contains(&kind), "never saw {kind}: {seen:?}");
    }
}

#[test]
fn the_face_down_loot_pile_shows_its_count_and_nothing_of_its_order() {
    let catalog = catalog();
    let game = Game::new(GameConfig::new(2, kingdom_with(&[Card::Cutthroat]), 42)).unwrap();
    let view = game.view(PlayerId(0));
    let loot = view.piles().find(|p| p.card() == Card::Loot).unwrap();
    let (index, size) = (loot.id().index(), loot.size());
    let mut twin = serde_json::to_value(&game).unwrap();
    twin["piles"][index]["cards"]
        .as_array_mut()
        .unwrap()
        .reverse();
    let twin: Game = serde_json::from_value(twin).unwrap();
    for seat in 0..2 {
        let update = update(&game, seat, &catalog, 3);
        assert_eq!(
            bytes(&update),
            bytes(&super::tests::update(&twin, seat, &catalog, 3))
        );
        let state = &update.snapshot.state;
        let zone = state
            .zones
            .iter()
            .find(|z| z.pile_name == catalog.card(Card::Loot).unwrap())
            .unwrap();
        assert_eq!(
            (zone.kind, zone.cards.clone()),
            (zone::NON_SUPPLY, vec![-1; size])
        );
    }
}
