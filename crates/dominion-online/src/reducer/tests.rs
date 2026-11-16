use super::*;
use crate::catalog::CardInfo;
use crate::questions::{QuestionDescription, QuestionKind};

fn catalog() -> Arc<Catalog> {
    let cards = [
        ("BACK", "Back", vec![]),
        ("COPPER", "Copper", vec!["Treasure"]),
        ("VILLAGE", "Village", vec!["Action"]),
        ("GOLD", "Gold", vec!["Treasure"]),
    ]
    .into_iter()
    .map(|(key, name, types)| CardInfo {
        key: key.into(),
        name: name.into(),
        cost: view::Cost {
            coins: 0,
            potions: 0,
            debt: 0,
        },
        types: types.into_iter().map(str::to_string).collect(),
        kingdom: name == "Village",
    })
    .collect();
    Arc::new(Catalog {
        cards,
        enums: BTreeMap::from([("Types".into(), vec!["ACTION".into(), "TREASURE".into()])]),
        labels: BTreeMap::new(),
    })
}
fn snapshot() -> model::Snapshot {
    let player = |id| model::Player {
        player_id: id,
        card_frequencies: vec![],
        reminders: vec![],
        bought_projects: vec![],
    };
    let zone = |index, kind, owner, cards| model::Zone {
        index,
        kind,
        pile_name: if kind == 4 { 2 } else { 0 },
        owner,
        cards,
        created_by: -1,
        attached_trait: 0,
        is_visible: true,
    };
    let cards = [1, 2, 3, 1, 3, 2, 1, 2]
        .into_iter()
        .enumerate()
        .map(|(id, name)| model::Card {
            id: id as i32,
            card_name: name,
            pile_name: name,
        })
        .collect();
    model::Snapshot {
        reconnect: false,
        game_id: 123,
        state: model::GameState {
            players: vec![player(42), player(99)],
            card_names: vec![1, 2, 3],
            pile_names: vec![1, 2, 3],
            card_counts: vec![3, 3, 2],
            cards,
            zones: vec![
                zone(0, 0, 0, vec![0]),
                zone(1, 1, 0, vec![1, 2]),
                zone(2, 2, 0, vec![3]),
                zone(3, 8, 0, vec![]),
                zone(10, 0, 1, vec![4]),
                zone(11, 1, 1, vec![5]),
                zone(12, 2, 1, vec![6]),
                zone(13, 8, 1, vec![]),
                zone(20, 4, -1, vec![7]),
                zone(30, 23, 0, vec![]),
            ],
            pile_markers: vec![],
            tokens: vec![],
            counters: vec![],
            turn: model::TurnDescription {
                owner_id: 0,
                turn_number: 1,
                turn_type: 0,
                controller_id: 0,
            },
            cost_reduction: model::CostReduction {
                generic: 0,
                by_type: vec![],
                by_card: vec![],
                cost_override: vec![],
            },
            temporary_effects: model::TemporaryEffects {
                renames: vec![],
                behaviour_changes: vec![],
                image_changes: vec![],
                card_type_changes: vec![],
            },
            associations: vec![],
            card_statuses: vec![],
        },
        autoplay_settings: vec![],
        table_options: model::TableOptions {
            players_see_spectator_chat: false,
            timer_settings: model::TimerSettings {
                settings: vec![],
                force_resign: false,
                preset: 4,
            },
        },
        events: vec![],
    }
}
fn reducer() -> Reducer {
    let mut r = Reducer::new(catalog(), 42);
    r.replace(snapshot()).unwrap();
    r
}
fn movement(from: i32, to: i32, before: Vec<i32>, after: Vec<i32>) -> model::Event {
    model::Event::CardMove(model::CardMove {
        from_zone: from,
        to_zone: to,
        before,
        after,
        movement_type: 1,
        animation_class: 5,
    })
}
fn question() -> Question {
    let story = Story {
        question_id: 0,
        arguments: vec![],
        context: None,
    };
    Question {
        index: 7,
        description: QuestionDescription {
            question_type: 11,
            association: -1,
            story: story.clone(),
            short_story: story,
        },
        kind: QuestionKind::Number {
            min: 0,
            max: 2,
            default: 0,
            decline: -1,
        },
    }
}
fn decision_id(r: &Reducer) -> String {
    r.view().unwrap().decision.unwrap().id
}

#[test]
fn initial_snapshot_hides_allocated_hand_and_deck_names() {
    let r = reducer();
    let v = r.view().unwrap();
    let you = v.you.as_ref().unwrap();
    assert_eq!(you.hand.known, BTreeMap::from([("Copper".into(), 1)]));
    assert_eq!((you.deck.count, you.deck.unknown), (2, 2));
    assert!(you.deck.known.is_empty());
    assert_eq!(
        (v.opponents[0].hand.count, v.opponents[0].hand.unknown),
        (1, 1)
    );
    assert_eq!(
        (v.opponents[0].deck.count, v.opponents[0].deck.unknown),
        (1, 1)
    );
    assert!(!serde_json::to_string(&v).unwrap().contains("Gold"));
}
#[test]
fn hidden_catalog_identity_is_unavailable_to_question_projection() {
    let r = reducer();
    assert_eq!(QuestionContext::card_name(&r, 0), Some("Copper".into()));
    assert_eq!(QuestionContext::card_name(&r, 2), None);
    assert_eq!(QuestionContext::card_name(&r, 4), None);
}
#[test]
fn reveal_anonymous_movement_and_shuffle_preserve_only_earned_knowledge() {
    let mut r = reducer();
    r.apply(model::Event::Inspection(model::Inspection {
        player: 0,
        cards: vec![2],
        from_zone: 1,
        to_zone: -1,
        is_public: false,
        linked_to_question: false,
    }))
    .unwrap();
    assert_eq!(
        r.view().unwrap().you.unwrap().deck.known,
        BTreeMap::from([("Gold".into(), 1)])
    );
    r.apply(movement(0, 1, vec![0], vec![-1])).unwrap();
    let you = r.view().unwrap().you.unwrap();
    assert_eq!(you.hand.count, 0);
    assert_eq!(you.deck.count, 3);
    assert_eq!(
        you.deck.known,
        BTreeMap::from([("Copper".into(), 1), ("Gold".into(), 1)])
    );
    r.apply(model::Event::Shuffle(model::Shuffle {
        owner: 0,
        include_discard: true,
    }))
    .unwrap();
    let you = r.view().unwrap().you.unwrap();
    assert_eq!(you.discard.count, 0);
    assert_eq!(you.deck.count, 4);
    assert!(!you.deck.order_known);
    assert_eq!(
        you.deck.known,
        BTreeMap::from([("Copper".into(), 2), ("Gold".into(), 1)])
    );
    r.apply(movement(1, 0, vec![-1], vec![-1])).unwrap();
    let you = r.view().unwrap().you.unwrap();
    assert_eq!(you.hand.count, 1);
    assert_eq!(you.hand.unknown, 1);
    assert_eq!(you.deck.count, 3);
    assert_eq!(you.deck.known, BTreeMap::from([("Copper".into(), 1)]));
    assert_eq!(you.owned.known.get("Gold"), Some(&1));
}
#[test]
fn snapshot_replacement_invalidates_answers_and_replay_is_deterministic() {
    let mut r = reducer();
    r.ask(question()).unwrap();
    let id = decision_id(&r);
    let old_epoch = r.epoch;
    r.apply(movement(0, 3, vec![0], vec![0])).unwrap();
    r.replace(snapshot()).unwrap();
    assert_eq!(r.epoch, old_epoch + 1);
    assert!(r.view().unwrap().decision.is_none());
    assert!(r.answer(&view::Answer::indices(id, vec![0]), None).is_err());
    let events = [
        movement(0, 3, vec![0], vec![0]),
        movement(1, 0, vec![-1], vec![2]),
        model::Event::Shuffle(model::Shuffle {
            owner: 0,
            include_discard: true,
        }),
    ];
    for event in &events {
        r.apply(event.clone()).unwrap();
    }
    let expected = serde_json::to_value(r.view().unwrap()).unwrap();
    r.replace(snapshot()).unwrap();
    for event in events {
        r.apply(event).unwrap();
    }
    assert_eq!(serde_json::to_value(r.view().unwrap()).unwrap(), expected);
}
#[test]
fn answers_reject_stale_ids_and_are_consumed_once() {
    let mut r = reducer();
    r.ask(question()).unwrap();
    let old = decision_id(&r);
    r.ask(question()).unwrap();
    let current = decision_id(&r);
    assert_ne!(old, current);
    assert!(
        r.answer(&view::Answer::indices(old, vec![0]), None)
            .is_err()
    );
    assert_eq!(
        r.answer(&view::Answer::indices(current.clone(), vec![1]), None)
            .unwrap(),
        (7, vec![1])
    );
    assert!(
        r.answer(&view::Answer::indices(current, vec![1]), None)
            .is_err()
    );
}
#[test]
fn malformed_batch_does_not_commit_earlier_moves_and_invalidates_question() {
    let mut r = reducer();
    r.ask(question()).unwrap();
    let before = r.view().unwrap();
    let state = serde_json::to_value(r.state().unwrap()).unwrap();
    let epoch = r.epoch;
    assert!(r.apply(movement(0, 3, vec![0, 999], vec![0, 999])).is_err());
    assert_eq!(serde_json::to_value(r.state().unwrap()).unwrap(), state);
    let after = r.view().unwrap();
    assert_eq!(after.you, before.you);
    assert_eq!(after.opponents, before.opponents);
    assert_eq!(after.supply, before.supply);
    assert_eq!(r.epoch, epoch + 1);
    assert!(after.decision.is_none());
}

#[test]
fn shuffle_forgets_observed_copy_identity_but_preserves_composition() {
    let mut r = reducer();
    r.apply(movement(0, 1, vec![0], vec![0])).unwrap();
    assert_eq!(QuestionContext::card_name(&r, 0), Some("Copper".into()));
    r.apply(model::Event::Shuffle(model::Shuffle {
        owner: 0,
        include_discard: false,
    }))
    .unwrap();
    let you = r.view().unwrap().you.unwrap();
    assert_eq!(you.deck.known.get("Copper"), Some(&1));
    assert_eq!(QuestionContext::card_name(&r, 0), None);
    assert_eq!(QuestionContext::card_zone(&r, 0), None);
}

#[test]
fn projected_snapshot_and_question_follow_shared_schema() {
    let schema: serde_json::Value = serde_json::from_str(include_str!(
        "../../../dominion-agent/protocol/agent-view.schema.json"
    ))
    .unwrap();
    let validator = jsonschema::validator_for(&schema).unwrap();
    let mut reducer = Reducer::new(catalog(), 42);
    reducer.replace(snapshot()).unwrap();
    let value = serde_json::to_value(reducer.view().unwrap()).unwrap();
    let errors: Vec<_> = validator
        .iter_errors(&value)
        .map(|e| e.to_string())
        .collect();
    assert!(errors.is_empty(), "{errors:?}");
}

#[test]
fn deck_reorder_accepts_anonymous_members_without_revealing_allocations() {
    let mut reducer = Reducer::new(catalog(), 42);
    reducer.replace(snapshot()).unwrap();
    reducer
        .apply(model::Event::PileReorder(model::PileReorder {
            pile: 1,
            cards: vec![-1, -1],
        }))
        .unwrap();
    let deck = reducer.view().unwrap().you.unwrap().deck;
    assert_eq!(deck.count, 2);
    assert_eq!(deck.unknown, 2);
    assert!(deck.known.is_empty());
}

#[test]
fn absent_pile_top_sentinels_are_not_card_identities() {
    let mut reducer = Reducer::new(catalog(), 42);
    reducer.replace(snapshot()).unwrap();
    reducer
        .apply(model::Event::PilesStatus(model::PilesStatus {
            draw_index: 1,
            draw_size: 2,
            discard_index: 2,
            discard_size: 0,
            top_card_id: -2,
        }))
        .unwrap();
    assert_eq!(reducer.view().unwrap().you.unwrap().discard.count, 0);
}

#[test]
fn controlled_turn_reveals_only_the_controlled_hand_and_not_its_deck() {
    let mut s = snapshot();
    s.state.turn.owner_id = 1;
    s.state.turn.controller_id = 0;
    let mut r = Reducer::new(catalog(), 42);
    r.replace(s).unwrap();
    let v = r.view().unwrap();
    assert!(v.turn.your_turn);
    assert_eq!(v.opponents[0].hand.known.get("Gold"), Some(&1));
    assert_eq!(v.opponents[0].deck.unknown, 1);
    assert_eq!(r.card_name(5), None);
}
#[test]
fn visible_auxiliary_state_is_projected_and_hidden_associations_stay_hidden() {
    let mut r = reducer();
    r.apply(model::Event::BehaviourChange(model::BehaviourChange {
        id: 0,
        card_name: 2,
    }))
    .unwrap();
    r.apply(model::Event::ImageChange(model::ImageChange {
        id: 0,
        card_name: 3,
    }))
    .unwrap();
    r.apply(model::Event::Association(model::Association {
        card: 0,
        linked_cards: vec![4, 3],
    }))
    .unwrap();
    r.state.as_mut().unwrap().tokens.push(model::Token {
        kind: 0,
        owner: 0,
        zone: 20,
        is_flipped: false,
    });
    r.state.as_mut().unwrap().counters.push(model::Counter {
        index: 0,
        kind: 9,
        owner: -1,
        value: 2,
        zone: 20,
        card: -1,
        is_visible: true,
    });
    r.state
        .as_mut()
        .unwrap()
        .card_statuses
        .push(model::CardStatus {
            card: 0,
            stays_out: true,
            played_card_lists: vec![],
            set_aside_card_lists: vec![vec![3, 4]],
            properties: vec![],
        });
    let v = r.view().unwrap();
    assert_eq!(v.supply[0].tokens.embargo, 2);
    assert_eq!(v.supply[0].tokens.players.len(), 1);
    assert_eq!(v.lasting[0].cards, vec!["Copper"]);
    assert!(v.notes.iter().any(|n| n.source == "Uses Village abilities"));
    assert!(v.notes.iter().any(|n| n.source == "Associated with Copper"));
    assert!(!v.notes.iter().any(|n| n.source.contains("Gold")));
    assert_eq!(r.state().unwrap().temporary_effects.image_changes.len(), 1);
}

#[test]
fn another_players_private_inspection_moves_anonymously() {
    let mut r = reducer();
    r.state.as_mut().unwrap().zones.push(model::Zone {
        index: 40,
        kind: 24,
        pile_name: 0,
        owner: 1,
        cards: vec![],
        created_by: -1,
        attached_trait: 0,
        is_visible: true,
    });
    r.zones.insert(40, KnowledgeZone::default());
    r.apply(model::Event::Inspection(model::Inspection {
        player: 1,
        cards: vec![5],
        from_zone: 11,
        to_zone: 40,
        is_public: false,
        linked_to_question: false,
    }))
    .unwrap();
    assert_eq!(r.zone_view(40).unwrap().unknown, 1);
    assert_eq!(r.card_name(5), None);
}

#[test]
fn upstream_placeholder_dialects_render_player_names_and_amounts_without_reparsing_names() {
    let mut r = reducer();
    r.names.insert(42, "PLAYER1".into());
    let args = vec![
        LogArgument::Player(0),
        LogArgument::Number(3),
        LogArgument::Amount {
            coins: 4,
            potions: 0,
            debt: 0,
        },
    ];
    assert_eq!(
        r.render_template(
            "Pass to PLAYER0; NUMBER1 CARDS1; AMOUNT2; Argument1.".into(),
            &args
        )
        .unwrap(),
        "Pass to PLAYER1; 3 3; $4, 0 Potions, 0 Debt; 3."
    );
}
