mod common;
use common::*;
use dominion_online::{
    Error,
    model::*,
    questions::*,
    server::*,
    transport::{self, Decoder, Message},
    wire::{MAX_COLLECTION, MAX_SAFE_LONG, Writer},
};

fn bytes(hex: &str) -> Vec<u8> {
    hex.as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|s| u8::from_str_radix(std::str::from_utf8(s).unwrap(), 16).unwrap())
        .collect()
}
fn roundtrip(messages: &[Message]) -> Vec<Message> {
    Decoder::new()
        .decode(&ServerEncoder::new().encode_batch(messages).unwrap())
        .unwrap()
}
#[test]
fn five_commands_match_upstream_68_command_fixtures() {
    let fixtures: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/outgoing.json")).unwrap();
    let mut count = 0;
    for fixture in fixtures.as_array().unwrap() {
        let opcode = fixture["opcode"].as_i64().unwrap();
        let expected = match opcode {
            35 => ClientCommand::ReconnectGame,
            37 => ClientCommand::AnswerQuestion {
                question: 0,
                values: vec![0],
                autoplay: false,
            },
            38 => ClientCommand::Resign,
            44 => ClientCommand::Ping,
            51 => ClientCommand::AnimationCompensationAck,
            _ => continue,
        };
        let frame = bytes(fixture["hex"].as_str().unwrap());
        assert_eq!(decode_command(&frame).unwrap(), expected);
        for n in 0..frame.len() {
            assert!(
                decode_command(&frame[..n]).is_err(),
                "opcode {opcode}, prefix {n}"
            );
        }
        let mut trailing = frame.clone();
        trailing.push(0);
        assert!(decode_command(&trailing).is_err());
        count += 1;
    }
    assert_eq!(count, 5);
}
#[test]
fn malformed_commands_are_rejected_exactly() {
    for opcode in [0_i32, 1, 36, 39, 49, 68, -1] {
        assert!(matches!(
            decode_command(&opcode.to_be_bytes()),
            Err(Error::Unsupported(_))
        ));
    }
    for count in [-1, MAX_COLLECTION as i32 + 1, i32::MAX] {
        let mut w = Writer::new();
        w.int(37);
        w.int(1);
        w.int(count);
        w.boolean(false);
        assert!(decode_command(&w.finish()).is_err());
    }
    let mut negative = transport::command::answer(0, &[]);
    negative[4..8].copy_from_slice(&(-1_i32).to_be_bytes());
    assert!(decode_command(&negative).is_err());
    let mut bool_bad = transport::command::answer(1, &[]);
    *bool_bad.last_mut().unwrap() = 2;
    assert!(decode_command(&bool_bad).is_err());
    assert!(decode_command(&vec![0; MAX_COMMAND_BYTES + 1]).is_err());
    let mut autoplay = transport::command::answer(1, &[2]);
    *autoplay.last_mut().unwrap() = 1;
    assert_eq!(
        decode_command(&autoplay).unwrap(),
        ClientCommand::AnswerQuestion {
            question: 1,
            values: vec![2],
            autoplay: true
        }
    );
}
#[test]
fn snapshot_question_result_and_all_events_roundtrip() {
    let mut snapshot = snapshot();
    snapshot.events = events();
    let messages = vec![
        Message::TableDetails(table()),
        Message::Snapshot(snapshot.clone()),
        Message::Question(choice(42)),
        Message::GameFinished {
            table: table(),
            result: result(),
            continue_allowed: false,
            match_completed: true,
        },
        Message::Pong,
    ];
    let decoded = roundtrip(&messages);
    assert!(matches!(&decoded[0],Message::TableDetails(t) if *t==table()));
    assert!(matches!(&decoded[1],Message::Snapshot(s) if *s==snapshot));
    assert!(matches!(&decoded[2],Message::Question(q) if *q==choice(42)));
    assert!(
        matches!(&decoded[3],Message::GameFinished{table:t,result:r,continue_allowed:false,match_completed:true} if *t==table() && *r==result())
    );
    for (event, decoded) in events().into_iter().zip(roundtrip(
        &events().into_iter().map(Message::Event).collect::<Vec<_>>(),
    )) {
        assert!(matches!(decoded,Message::Event(e) if e==event));
    }
}
#[test]
fn counters_continue_across_batches_and_reset_on_new_socket() {
    let mut enc = ServerEncoder::new();
    let mut dec = Decoder::new();
    for n in [2, 1, 3] {
        assert_eq!(
            dec.decode(&enc.encode_batch(&vec![Message::Pong; n]).unwrap())
                .unwrap()
                .len(),
            n
        );
    }
    assert_eq!(
        ServerEncoder::new().encode_batch(&[Message::Pong]).unwrap()[..4],
        [0, 0, 0, 0]
    );
    assert!(enc.encode_batch(&[Message::LobbyNotice]).is_err());
    assert!(enc.encode_batch(&[Message::Pong]).is_err());
}
#[test]
fn allocation_and_parallel_array_disagreement_never_produces_bytes() {
    let mut bad = snapshot();
    bad.state.cards[0].card_name = 3;
    assert!(
        ServerEncoder::new()
            .encode_batch(&[Message::Snapshot(bad)])
            .is_err()
    );
    let mut bad = snapshot();
    bad.state.pile_names.pop();
    assert!(
        ServerEncoder::new()
            .encode_batch(&[Message::Snapshot(bad)])
            .is_err()
    );
    let mut bad = snapshot();
    bad.state.card_counts[0] = -1;
    assert!(
        ServerEncoder::new()
            .encode_batch(&[Message::Snapshot(bad)])
            .is_err()
    );
    let mut bad = table();
    bad.ready.pop();
    assert!(
        ServerEncoder::new()
            .encode_batch(&[Message::TableDetails(bad)])
            .is_err()
    );
}
#[test]
fn invalid_numbers_sizes_and_enums_fail_without_panicking() {
    let mut bad = snapshot();
    bad.game_id = MAX_SAFE_LONG + 1;
    assert!(
        ServerEncoder::new()
            .encode_batch(&[Message::Snapshot(bad)])
            .is_err()
    );
    let mut bad = snapshot();
    bad.state.zones[0].kind = 35;
    assert!(
        ServerEncoder::new()
            .encode_batch(&[Message::Snapshot(bad)])
            .is_err()
    );
    let mut bad = snapshot();
    bad.state.players[0].bought_projects = vec![0; MAX_COLLECTION + 1];
    assert!(
        ServerEncoder::new()
            .encode_batch(&[Message::Snapshot(bad)])
            .is_err()
    );
    let mut bad = table();
    bad.rules.push((
        transport::TableRule {
            id: 6,
            value: transport::RuleValue::Double(f64::NAN),
        },
        false,
    ));
    assert!(
        ServerEncoder::new()
            .encode_batch(&[Message::TableDetails(bad)])
            .is_err()
    );
    let mut bad = snapshot();
    bad.events = vec![Event::CardMove(CardMove {
        from_zone: 0,
        to_zone: 1,
        before: vec![0],
        after: vec![],
        movement_type: 0,
        animation_class: 0,
    })];
    assert!(
        ServerEncoder::new()
            .encode_batch(&[Message::Snapshot(bad)])
            .is_err()
    );
    let mut bad = choice(1);
    if let QuestionKind::Choice { elements, .. } = &mut bad.kind {
        *elements = QuestionElements::Modes(vec![221]);
    }
    assert!(
        ServerEncoder::new()
            .encode_batch(&[Message::Question(bad)])
            .is_err()
    );
}
#[test]
fn question_trees_have_only_one_root_index_and_respect_depth() {
    let mut root = choice(42);
    root.description.question_type = 19;
    let mut child = choice(42);
    child.description.question_type = 18;
    let mut number = choice(42);
    number.description.question_type = 11;
    number.kind = QuestionKind::Number {
        min: 2,
        max: 4,
        default: 3,
        decline: -1,
    };
    child.kind = QuestionKind::Complex(vec![number, choice(42)]);
    root.kind = QuestionKind::Complex(vec![choice(42), child]);
    assert!(
        matches!(&roundtrip(&[Message::Question(root.clone())])[0],Message::Question(q) if *q==root)
    );
    for _ in 0..50 {
        let mut parent = choice(42);
        parent.description.question_type = 19;
        parent.kind = QuestionKind::Complex(vec![root]);
        root = parent;
    }
    assert!(
        ServerEncoder::new()
            .encode_batch(&[Message::Question(root)])
            .is_err()
    );
    let mut arg = LogArgument::Number(1);
    for _ in 0..50 {
        arg = LogArgument::Event {
            kind: 0,
            ability: Ability {
                description_type: 0,
                name: 0,
                association: -1,
                arguments: vec![],
                optional: false,
                copy_index: 0,
                copy_total: 1,
            },
            arguments: vec![arg],
        };
    }
    let mut q = choice(42);
    q.description.story.arguments = vec![arg];
    assert!(
        ServerEncoder::new()
            .encode_batch(&[Message::Question(q)])
            .is_err()
    );
}

#[test]
#[ignore = "exports synthetic Rust encoder fixtures for offline pinned upstream proof"]
fn export_reverse_proof() {
    let mut enc = ServerEncoder::new();
    let mut fixtures = vec![];
    for (name, message) in proof_messages() {
        let bytes = enc.encode_batch(&[message]).unwrap();
        fixtures.push(serde_json::json!({"name":name,"hex":bytes.iter().map(|b|format!("{b:02x}")).collect::<String>()}));
    }
    let path = std::env::var("DOMINION_REVERSE_PROOF_OUT").expect("output path");
    std::fs::write(
        path,
        serde_json::to_string_pretty(&fixtures).unwrap() + "\n",
    )
    .unwrap();
}

#[test]
fn encoded_server_fixtures_match_offline_upstream_proof_inputs() {
    let fixtures: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/server.json")).unwrap();
    let mut encoder = ServerEncoder::new();
    let expected = proof_messages();
    assert_eq!(fixtures.as_array().unwrap().len(), expected.len());
    for (fixture, (name, message)) in fixtures.as_array().unwrap().iter().zip(expected) {
        assert_eq!(fixture["name"], name);
        assert_eq!(
            encoder.encode_batch(&[message]).unwrap(),
            bytes(fixture["hex"].as_str().unwrap())
        );
    }
}

#[test]
fn story_argument_shapes_and_nonempty_result_parts_roundtrip() {
    let ability = Ability {
        description_type: 2,
        name: 0,
        association: -1,
        arguments: vec![LogArgument::Number(2)],
        optional: false,
        copy_index: 0,
        copy_total: 1,
    };
    let mut q = choice(42);
    q.description.story.arguments = vec![
        LogArgument::CardNames(vec![dominion_online::questions::CardFrequency {
            name: 2,
            frequency: 3,
        }]),
        LogArgument::Player(0),
        LogArgument::Zone(0),
        LogArgument::OwnZone { player: 0, zone: 0 },
        LogArgument::Number(2),
        LogArgument::Counter(0),
        LogArgument::Turn {
            owner: 0,
            number: 12,
            kind: 0,
            controller: 0,
        },
        LogArgument::Amount {
            coins: 2,
            potions: 0,
            debt: 0,
        },
        LogArgument::Ability(ability.clone()),
        LogArgument::Event {
            kind: 0,
            ability,
            arguments: vec![LogArgument::Number(1)],
        },
        LogArgument::Card(0),
        LogArgument::SingularCard(0),
        LogArgument::Token(0),
        LogArgument::DirectionalZone {
            direction: 1,
            zone: 0,
        },
        LogArgument::Players(vec![0, 1]),
        LogArgument::MetaGame(MetaGameInfo {
            game_id: 7,
            rated: false,
            card_pool_level: 0,
            levels: vec![(0, 0.25)],
            likelihoods: vec![(0, vec![2])],
            timer_preset: 4,
        }),
    ];
    q.description.story.context = Some(LogArgument::Card(0));
    assert!(
        matches!(&roundtrip(&[Message::Question(q.clone())])[0],Message::Question(decoded) if *decoded==q)
    );
    let mut result = result();
    result.players[0].score.parts = vec![ScorePart {
        card_name: 2,
        points: 3,
        frequency: 1,
        reserved: 0,
        explanation: LogEntry::Named {
            name: 0,
            depth: 0,
            arguments: vec![LogArgument::Number(3)],
        },
    }];
    let decoded = roundtrip(&[Message::GameFinished {
        table: table(),
        result: result.clone(),
        continue_allowed: false,
        match_completed: true,
    }]);
    assert!(matches!(&decoded[0],Message::GameFinished{result:r,..} if *r==result));
}
#[test]
fn string_and_batch_limits_and_optional_enum_sentinels_are_enforced() {
    let mut bad = table();
    bad.division = "x".repeat(MAX_BATCH_BYTES);
    assert!(
        ServerEncoder::new()
            .encode_batch(&[Message::TableDetails(bad)])
            .is_err()
    );
    let mut bad = table();
    bad.tournament = Some(-1);
    assert!(
        ServerEncoder::new()
            .encode_batch(&[Message::TableDetails(bad)])
            .is_err()
    );
    let mut bad = choice(42);
    bad.description.story.question_id = 736;
    assert!(
        ServerEncoder::new()
            .encode_batch(&[Message::Question(bad)])
            .is_err()
    );
    let mut bad = choice(42);
    if let QuestionKind::Choice { decline, .. } = &mut bad.kind {
        *decline = 71;
    }
    assert!(
        ServerEncoder::new()
            .encode_batch(&[Message::Question(bad)])
            .is_err()
    );
}
