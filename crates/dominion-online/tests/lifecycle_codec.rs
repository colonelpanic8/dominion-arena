mod common;
use dominion_online::{
    server::*,
    transport::{self as t, Decoder, Message, outgoing::Command},
    wire::Writer,
};

fn bytes(hex: &str) -> Vec<u8> {
    hex.as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|x| u8::from_str_radix(std::str::from_utf8(x).unwrap(), 16).unwrap())
        .collect()
}
#[test]
fn selected_commands_from_original_producers_are_exact_and_credentials_redacted() {
    let fixtures: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("fixtures/outgoing.json")).unwrap();
    for f in fixtures {
        let data = bytes(f["hex"].as_str().unwrap());
        let opcode = f["opcode"].as_i64().unwrap();
        let parsed = decode_lifecycle_command(&data).unwrap();
        for n in 0..data.len() {
            assert!(
                decode_lifecycle_command(&data[..n]).is_err(),
                "opcode {opcode} prefix {n}"
            );
        }
        let mut extra = data.clone();
        extra.push(0);
        assert!(decode_lifecycle_command(&extra).is_err());
        if let LifecycleCommand::Login { password, .. } = parsed {
            assert_eq!(password.expose(), "fixture");
            assert!(!format!("{password:?}").contains("fixture"));
        }
    }
    let cmd = Command::JoinTable {
        table: 100,
        as_player: true,
    }
    .encode()
    .unwrap();
    assert_eq!(
        decode_lifecycle_command(&cmd).unwrap(),
        LifecycleCommand::JoinTable {
            table: 100,
            as_player: true
        }
    );
}
#[test]
fn unsupported_mutations_do_not_gain_a_typed_host_operation() {
    let fixtures: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("fixtures/outgoing.json")).unwrap();
    for opcode in [14_i32, 16, 18, 19, 20, 22, 23, 29, 36, 39, 49, 52, 58, 64] {
        let fixture = &fixtures[opcode as usize];
        assert_eq!(
            decode_lifecycle_command(&bytes(fixture["hex"].as_str().unwrap())).unwrap(),
            LifecycleCommand::Unsupported { opcode }
        );
    }
    assert!(decode_lifecycle_command(&vec![0; MAX_COMMAND_BYTES + 1]).is_err());
    let mut w = Writer::new();
    w.int(4);
    w.long(1);
    w.int(6);
    w.double(f64::NAN);
    assert!(decode_lifecycle_command(&w.finish()).is_err());
}
#[test]
fn probe_has_counter_and_state_without_normal_opcode() {
    for (state, id) in [
        (t::ServerState::Normal, 1_i32),
        (t::ServerState::BadClientVersion, 3),
    ] {
        let bytes = encode_probe(0, state).unwrap();
        assert_eq!(bytes.len(), 8);
        assert_eq!(bytes[4..], id.to_be_bytes());
        assert!(
            matches!(Decoder::probe().decode(&bytes).unwrap().as_slice(),[Message::ServerState(s)] if *s==state)
        );
    }
}
fn lifecycle_messages() -> Vec<Message> {
    vec![
        Message::Login(t::LoginSuccess::new(
            10,
            "Alice".into(),
            "synthetic-only".into(),
        )),
        Message::TableJoined {
            table: common::table(),
            show: true,
        },
        Message::Tables(vec![t::TableSummary {
            table_id: 1,
            host: common::table().players[0].clone(),
            players: 2,
            bots: 0,
            spectators: 0,
            min_players: 2,
            max_players: 2,
            observable: false,
            joinable: false,
            status: 0,
            start_time: None,
        }]),
        Message::GameStarted {
            table_id: 1,
            game_id: 7,
        },
        Message::Snapshot(common::snapshot()),
        Message::Question(common::choice(42)),
    ]
}
#[test]
fn bootstrap_is_consumable_and_each_truncation_fails() {
    let messages = lifecycle_messages();
    let mut encoder = ServerEncoder::new();
    for message in messages {
        let b = ServerEncoder::new()
            .encode_batch(std::slice::from_ref(&message))
            .unwrap();
        for n in 1..b.len() {
            assert!(Decoder::new().decode(&b[..n]).is_err(), "prefix {n}");
        }
        assert_eq!(Decoder::new().decode(&b).unwrap().len(), 1);
        encoder.encode_batch(&[message]).unwrap();
    }
}
#[test]
fn write_original_consumer_fixtures_on_explicit_request() {
    if let Ok(path) = std::env::var("DOMINION_LIFECYCLE_FIXTURE_OUTPUT") {
        let mut e = ServerEncoder::new();
        let output:Vec<_>=lifecycle_messages().into_iter().enumerate().map(|(i,m)|{let b=e.encode_batch(&[m]).unwrap();serde_json::json!({"name":format!("lifecycle-{i}"),"hex":b.iter().map(|v|format!("{v:02x}")).collect::<String>()})}).collect();
        std::fs::write(path, serde_json::to_string_pretty(&output).unwrap()).unwrap();
    }
}

#[test]
fn lifecycle_messages_keep_framing_across_membership_chat_and_failures() {
    let messages = vec![
        Message::Failed(t::Failure {
            reason: "ILLEGAL_REQUEST",
            card: None,
        }),
        Message::FormResult {
            form: "arena-compatibility".into(),
            succeeded: false,
            error: Some("unsupported synthetic feature".into()),
        },
        Message::Succeeded(3),
        Message::Preferences(
            t::LoginSuccess::new(10, "Alice".into(), "synthetic-only".into()).preferences,
        ),
        Message::HostChanged {
            table_id: 1,
            host: 20,
        },
        Message::PlayerJoined {
            table_id: 1,
            player: common::table().players[1].clone(),
            is_player: true,
        },
        Message::PlayerStatus {
            player_id: 20,
            table_id: 1,
            status: 3,
        },
        Message::RoleChanged {
            table_id: 1,
            player_id: 20,
            is_player: true,
            ready: true,
        },
        Message::PlayerLeft {
            table_id: 1,
            player_id: 20,
        },
        Message::Chat {
            receiver: "t1".into(),
            sender: "p10".into(),
            text: "synthetic chat".into(),
        },
        Message::Ancillary(t::ancillary::Message::PlayerAvatars(vec![])),
        Message::TableRemoved,
        Message::Pong,
    ];
    let bytes = ServerEncoder::new().encode_batch(&messages).unwrap();
    let decoded = Decoder::new().decode(&bytes).unwrap();
    assert_eq!(decoded.len(), messages.len());
    assert!(
        matches!(&decoded[1],Message::FormResult{form,error:Some(_),succeeded:false} if form=="arena-compatibility")
    );
}
#[test]
fn nested_rule_and_preference_lengths_are_bounded_before_allocation() {
    for (opcode, id) in [(9, 2), (9, 11), (12, 0)] {
        let mut w = Writer::new();
        w.int(opcode);
        if opcode == 9 {
            w.int(1);
            w.int(id);
            if id == 11 {
                w.int(0);
                w.int(0);
            }
        } else {
            w.int(id);
            w.int(2);
        }
        w.int(i32::MAX);
        assert!(decode_lifecycle_command(&w.finish()).is_err());
    }
    let command = Command::LoginWithSession {
        player: 10,
        version: "2.3.7".into(),
        session: "never-display-this-token".into(),
        language: -1,
    }
    .encode()
    .unwrap();
    assert!(
        !format!("{:?}", decode_lifecycle_command(&command).unwrap()).contains("never-display")
    );
    let command = Command::RequestServerState {
        version: "2.3.7".into(),
        player: Some(10),
        session: Some("never-display-probe-token".into()),
    }
    .encode()
    .unwrap();
    assert!(
        !format!("{:?}", decode_lifecycle_command(&command).unwrap()).contains("never-display")
    );
}

#[test]
fn export_original_and_number_consumer_fixture_on_request() {
    use dominion_online::questions::*;
    let Ok(path) = std::env::var("DOMINION_CONSUMER_AND_OUTPUT") else {
        return;
    };
    let number = |kind, default| Question {
        index: 42,
        description: QuestionDescription {
            question_type: kind,
            ..common::choice(42).description
        },
        kind: QuestionKind::Number {
            min: 0,
            max: 5,
            default,
            decline: -1,
        },
    };
    let question = Question {
        index: 42,
        description: QuestionDescription {
            question_type: 19,
            ..common::choice(42).description
        },
        kind: QuestionKind::Complex(vec![number(47, 2), number(48, 3)]),
    };
    let mut messages = lifecycle_messages();
    messages.pop();
    messages.push(Message::Question(question));
    let bytes = ServerEncoder::new().encode_batch(&messages).unwrap();
    let fixture = serde_json::json!([{"name":"original-and-number-send","messages":[{"hex":bytes.iter().map(|v|format!("{v:02x}")).collect::<String>()}],"answer":{"path":[1],"number":4},"expected":{"values":[1,2,1,4]}}]);
    std::fs::write(path, serde_json::to_string_pretty(&fixture).unwrap()).unwrap();
}

#[test]
fn server_rules_encode_canonical_fields_without_loosening_production_commands() {
    let mut table = common::table();
    table.rules = vec![
        (
            t::TableRule {
                id: 4,
                value: t::RuleValue::Group(t::Group::Nobody),
            },
            true,
        ),
        (
            t::TableRule {
                id: 15,
                value: t::RuleValue::Int(0),
            },
            true,
        ),
        (
            t::TableRule {
                id: 17,
                value: t::RuleValue::Group(t::Group::Players(table.players.clone())),
            },
            true,
        ),
    ];
    for (rule, _) in &table.rules {
        assert!(t::command::change_rule(1, rule).is_err());
    }
    let bytes = ServerEncoder::new()
        .encode_batch(&[Message::TableJoined {
            table: table.clone(),
            show: true,
        }])
        .unwrap();
    assert!(
        matches!(Decoder::new().decode(&bytes).unwrap().as_slice(),[Message::TableJoined{table:actual,show:true}] if actual==&table)
    );
}
