#![allow(dead_code)]
use dominion_online::{
    catalog::{CardInfo, Catalog},
    model::*,
    questions::*,
    transport::{NamedId, TableDetails},
};
use std::{collections::BTreeMap, sync::Arc};

pub fn catalog() -> Arc<Catalog> {
    Arc::new(Catalog {
        cards: ["Back", "Estate", "Copper", "Silver"]
            .iter()
            .map(|name| CardInfo {
                key: name.to_uppercase(),
                name: (*name).into(),
                cost: dominion_protocol::Cost::default(),
                types: if *name == "Village" {
                    vec!["Action".into()]
                } else {
                    vec![]
                },
                kingdom: *name == "Village",
            })
            .collect(),
        enums: BTreeMap::new(),
        labels: BTreeMap::from([("DeclineButtonIds".into(), vec![Some("End Actions".into())])]),
        texts: BTreeMap::new(),
    })
}
pub fn table() -> TableDetails {
    TableDetails {
        table_id: 1,
        host_id: 10,
        players: vec![
            NamedId {
                id: 10,
                name: "Alice".into(),
            },
            NamedId {
                id: 20,
                name: "Bob".into(),
            },
        ],
        ready: vec![true, false],
        bots: vec![],
        spectators: vec![],
        reconnecting: vec![],
        rules: vec![],
        division: String::new(),
        match_games: vec![],
        match_score: None,
        tournament: None,
    }
}
pub fn snapshot() -> Snapshot {
    let zone = |index, kind, owner, cards| Zone {
        index,
        kind,
        owner,
        cards,
        pile_name: 0,
        created_by: -1,
        attached_trait: 0,
        is_visible: true,
    };
    Snapshot {
        reconnect: false,
        game_id: 7,
        state: GameState {
            players: vec![10, 20]
                .into_iter()
                .map(|player_id| Player {
                    player_id,
                    card_frequencies: vec![],
                    reminders: vec![],
                    bought_projects: vec![],
                })
                .collect(),
            card_names: vec![2, 3],
            pile_names: vec![2, 3],
            card_counts: vec![2, 1],
            cards: vec![
                Card {
                    id: 0,
                    card_name: 2,
                    pile_name: 2,
                },
                Card {
                    id: 1,
                    card_name: 2,
                    pile_name: 2,
                },
                Card {
                    id: 2,
                    card_name: 3,
                    pile_name: 3,
                },
            ],
            zones: vec![
                zone(0, 0, 0, vec![0]),
                zone(1, 1, 0, vec![-1, -1]),
                zone(2, 2, 0, vec![1]),
                zone(3, 0, 1, vec![-1]),
                zone(4, 1, 1, vec![-1, -1]),
                zone(5, 2, 1, vec![]),
                zone(6, 8, 0, vec![2]),
                zone(7, 8, 1, vec![]),
                Zone {
                    pile_name: 3,
                    ..zone(8, 4, -1, vec![-1, -1])
                },
                zone(9, 3, -1, vec![]),
            ],
            pile_markers: vec![],
            tokens: vec![],
            counters: vec![Counter {
                index: 0,
                kind: 0,
                owner: 0,
                value: 1,
                zone: -1,
                card: -1,
                is_visible: true,
            }],
            turn: TurnDescription {
                owner_id: 0,
                turn_number: 12,
                turn_type: 0,
                controller_id: 0,
            },
            cost_reduction: CostReduction {
                generic: 0,
                by_type: vec![],
                by_card: vec![],
                cost_override: vec![],
            },
            temporary_effects: TemporaryEffects {
                renames: vec![],
                behaviour_changes: vec![],
                image_changes: vec![],
                card_type_changes: vec![],
            },
            associations: vec![],
            card_statuses: vec![],
        },
        autoplay_settings: vec![AutoplaySetting {
            card_name: 2,
            value: 0,
        }],
        table_options: TableOptions {
            players_see_spectator_chat: false,
            timer_settings: TimerSettings {
                settings: vec![(0, 1000)],
                force_resign: false,
                preset: 4,
            },
        },
        events: vec![],
    }
}
pub fn result() -> GameResult {
    GameResult {
        table_id: 1,
        game_id: 7,
        rating_type: None,
        empty_piles: vec![3],
        auto_continue: false,
        players: vec![(10, 1, 20, 12, None), (20, 2, 25, 11, Some(0))]
            .into_iter()
            .map(
                |(player_id, rank, total_points, used_turns, resignation_type)| PlayerResult {
                    player_id,
                    rank,
                    score: Score {
                        total_points,
                        used_turns,
                        parts: vec![],
                    },
                    card_names: vec![],
                    resign_index: if resignation_type.is_some() { 0 } else { -1 },
                    resignation_type,
                },
            )
            .collect(),
    }
}
pub fn story() -> Story {
    Story {
        question_id: 0,
        arguments: vec![],
        context: None,
    }
}
pub fn choice(index: i32) -> Question {
    Question {
        index,
        description: QuestionDescription {
            question_type: 0,
            association: -1,
            story: story(),
            short_story: story(),
        },
        kind: QuestionKind::Choice {
            min: 0,
            max: 1,
            elements: QuestionElements::Cards(vec![0]),
            decline: 0,
            defaults: vec![],
            affected: vec![],
        },
    }
}
pub fn events() -> Vec<Event> {
    vec![
        Event::CardMove(CardMove {
            from_zone: 0,
            to_zone: 6,
            before: vec![0],
            after: vec![0],
            movement_type: 0,
            animation_class: 0,
        }),
        Event::CounterChange(CounterChange {
            index: 0,
            new_value: 2,
        }),
        Event::PileUpdate(PileUpdate {
            index: 8,
            top_card_id: 2,
        }),
        Event::TurnDescription(snapshot().state.turn),
        Event::Shuffle(Shuffle {
            owner: 0,
            include_discard: true,
        }),
        Event::Inspection(Inspection {
            player: 0,
            cards: vec![0],
            from_zone: 0,
            to_zone: 6,
            is_public: true,
            linked_to_question: false,
        }),
        Event::CostReduction(CostReduction {
            generic: 1,
            by_type: vec![(0, 1)],
            by_card: vec![(2, 1)],
            cost_override: vec![(
                3,
                Cost {
                    coins: 2,
                    potions: 0,
                    debt: 0,
                },
            )],
        }),
        Event::TokenChange(TokenChange {
            token: 0,
            zone: 8,
            is_flipped: true,
        }),
        Event::ZoneCreated(ZoneCreated {
            index: 10,
            owner: 0,
            kind: 8,
            created_by: -1,
        }),
        Event::ZoneDestroyed(ZoneDestroyed { index: 10 }),
        Event::Rename(Rename {
            id: 0,
            card_name: 3,
        }),
        Event::BehaviourChange(BehaviourChange {
            id: 0,
            card_name: 3,
        }),
        Event::PilesStatus(PilesStatus {
            draw_index: 1,
            draw_size: 2,
            discard_index: 2,
            discard_size: 1,
            top_card_id: 1,
        }),
        Event::RemindersChange(RemindersChange {
            player: 0,
            reminders: vec![Reminder {
                card_name: 2,
                blinking: true,
                frequency: 1,
            }],
        }),
        Event::ImageChange(ImageChange {
            id: 0,
            card_name: 3,
        }),
        Event::Association(Association {
            card: 0,
            linked_cards: vec![2],
        }),
        Event::CardStatus(CardStatus {
            card: 0,
            stays_out: true,
            played_card_lists: vec![vec![2]],
            set_aside_card_lists: vec![vec![-1]],
            properties: vec![0],
        }),
        Event::ProjectBought(ProjectBought {
            player: 0,
            project: 3,
        }),
        Event::ChangeCardTypes(ChangeCardTypes {
            cards: vec![0],
            added: vec![0],
            removed: vec![1],
        }),
        Event::PileReorder(PileReorder {
            pile: 8,
            cards: vec![2, -1],
        }),
        Event::ProjectRemoved(ProjectRemoved {
            player: 0,
            project: 3,
        }),
    ]
}

pub fn proof_messages() -> Vec<(String, dominion_online::transport::Message)> {
    use dominion_online::transport::Message;
    let mut messages = vec![
        ("table".into(), Message::TableDetails(table())),
        ("snapshot".into(), Message::Snapshot(snapshot())),
        ("question".into(), Message::Question(choice(42))),
        (
            "finished".into(),
            Message::GameFinished {
                table: table(),
                result: result(),
                continue_allowed: false,
                match_completed: true,
            },
        ),
        ("pong".into(), Message::Pong),
    ];
    for (index, event) in events().into_iter().enumerate() {
        messages.push((format!("event-{index}"), Message::Event(event)));
    }
    let mut s = snapshot();
    s.events = events();
    messages.push(("snapshot-events".into(), Message::Snapshot(s)));
    let a = Ability {
        description_type: 2,
        name: 0,
        association: -1,
        arguments: vec![LogArgument::Number(3)],
        optional: true,
        copy_index: 0,
        copy_total: 1,
    };
    for (index, elements) in [
        QuestionElements::Abilities(vec![a]),
        QuestionElements::ExtraTurns(vec![ExtraTurn {
            owner: 0,
            number: 2,
            kind: 0,
            controller: 0,
        }]),
        QuestionElements::Zones(vec![0]),
        QuestionElements::Modes(vec![0]),
        QuestionElements::Names(vec![2]),
        QuestionElements::Buttons(vec![0]),
        QuestionElements::Cleanup(vec![CleanupAction {
            names: vec![2],
            is_optional: true,
            allows_reordering: false,
            is_enabled: true,
        }]),
        QuestionElements::BuyableCards(vec![(0, true)]),
        QuestionElements::ExplainedCards(vec![(0, 1, -1)]),
    ]
    .into_iter()
    .enumerate()
    {
        let mut q = choice(42);
        if let QuestionKind::Choice { elements: e, .. } = &mut q.kind {
            *e = elements;
        }
        messages.push((format!("elements-{}", index + 1), Message::Question(q)));
    }
    let mut number = choice(42);
    number.description.question_type = 11;
    number.kind = QuestionKind::Number {
        min: 1,
        max: 3,
        default: 2,
        decline: -1,
    };
    let mut delayed = choice(42);
    delayed.description.question_type = 21;
    delayed.kind = QuestionKind::Delayed { decline: 0 };
    let mut name = choice(42);
    name.description.question_type = 20;
    name.kind = QuestionKind::Name;
    let mut complex = choice(42);
    complex.description.question_type = 18;
    complex.kind = QuestionKind::Complex(vec![choice(42), number.clone()]);
    for (key, q) in [
        ("number", number),
        ("delayed", delayed),
        ("name", name),
        ("complex", complex),
    ] {
        messages.push((key.into(), Message::Question(q)));
    }
    messages
}
