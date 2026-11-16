use super::*;
use dominion_online::{
    questions::{Question, QuestionDescription, QuestionKind, Story},
    transport::{LoginSuccess, NamedId},
    wire::{Reader, Writer},
};
use std::collections::BTreeMap;
use tempfile::TempDir;

fn catalog() -> Arc<Catalog> {
    Arc::new(Catalog {
        cards: vec![],
        enums: BTreeMap::new(),
        labels: BTreeMap::new(),
    })
}
fn named(id: i32) -> NamedId {
    NamedId {
        id,
        name: format!("fixture-{id}"),
    }
}
fn table() -> TableDetails {
    TableDetails {
        table_id: 10,
        host_id: 42,
        players: vec![named(42)],
        ready: vec![false],
        bots: vec![named(-2)],
        spectators: vec![],
        reconnecting: vec![],
        rules: vec![
            (
                TableRule {
                    id: 5,
                    value: RuleValue::Group(Group::Nobody),
                },
                false,
            ),
            (
                TableRule {
                    id: 19,
                    value: RuleValue::Boolean(false),
                },
                false,
            ),
        ],
        division: String::new(),
        match_games: vec![],
        match_score: None,
        tournament: None,
    }
}
fn runtime() -> (TempDir, Runtime) {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::open(directory.path()).unwrap();
    let runtime = Runtime {
        socket: None,
        needs_snapshot: false,
        request_game: false,
        leaving_finished: false,
        resuming: false,
        endpoint: Endpoint::Alpha,
        session: None,
        table: Some(table()),
        bots: vec![-2],
        human: None,
        search: None,
        queue_status: None,
        outbox: Vec::new(),
        update: Update {
            reducer: Reducer::new(catalog(), 42),
            error: None,
            connected: true,
            searching: false,
            queue_feedback: Vec::new(),
            mode: OnlineMode::Bots,
        },
        store,
    };
    (directory, runtime)
}
fn login(reconnecting: bool) -> LoginSuccess {
    let mut w = Writer::new();
    w.int(42);
    w.string("fixture-account");
    w.boolean(reconnecting);
    w.int(0);
    w.int(0);
    w.string("private-fixture-authority");
    w.int(0);
    for _ in 0..4 {
        w.int(0);
    }
    w.int(0);
    w.int(0);
    w.int(0);
    let bytes = w.finish();
    let mut reader = Reader::new(&bytes);
    let login = LoginSuccess::parse(&mut reader).unwrap();
    reader.finish().unwrap();
    login
}
fn snapshot() -> model::Snapshot {
    let player = |player_id| model::Player {
        player_id,
        card_frequencies: vec![],
        reminders: vec![],
        bought_projects: vec![],
    };
    model::Snapshot {
        reconnect: false,
        game_id: 123,
        state: model::GameState {
            players: vec![player(42), player(-2)],
            card_names: vec![],
            pile_names: vec![],
            card_counts: vec![],
            cards: vec![],
            zones: vec![],
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
fn question(index: i32) -> Question {
    let story = Story {
        question_id: 0,
        arguments: vec![],
        context: None,
    };
    Question {
        index,
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
fn decision_id(runtime: &Runtime) -> String {
    runtime.update.reducer.view().unwrap().decision.unwrap().id
}
fn active_runtime() -> (TempDir, Runtime) {
    let (directory, mut runtime) = runtime();
    runtime.process(Message::Login(login(false))).unwrap();
    runtime.process(Message::Snapshot(snapshot())).unwrap();
    runtime.process(Message::Question(question(7))).unwrap();
    (directory, runtime)
}

#[test]
fn no_human_seats_or_silently_ignored_counts() {
    for (players, bots) in [(1, 0), (2, 0), (3, 1), (7, 6)] {
        assert!(
            NewGame {
                players: Some(players),
                bots: Some(bots),
                ..Default::default()
            }
            .rules(&catalog())
            .is_err()
        );
    }
    assert_eq!(NewGame::default().rules(&catalog()).unwrap().0, 1);
}
#[test]
fn table_gate_rejects_humans_visitors_reconnecting_players_and_foreign_host() {
    let (_directory, mut runtime) = runtime();
    assert!(runtime.safe_table().is_ok());
    let good = table();
    let mut bad = Vec::new();
    let mut human = good.clone();
    human.players.push(named(99));
    bad.push(human);
    let mut spectator = good.clone();
    spectator.spectators.push(named(99));
    bad.push(spectator);
    let mut reconnecting = good.clone();
    reconnecting.reconnecting.push(named(99));
    bad.push(reconnecting);
    let mut foreign_host = good.clone();
    foreign_host.host_id = 99;
    bad.push(foreign_host);
    let mut wrong_account = good.clone();
    wrong_account.players[0] = named(99);
    bad.push(wrong_account);
    for table in bad {
        runtime.table = Some(table);
        assert!(runtime.safe_table().unwrap_err().starts_with("REFUSED:"));
    }
}
#[test]
fn table_gate_requires_confirmed_unrated_and_no_spectator_rules() {
    let (_directory, mut runtime) = runtime();
    for id in [5, 19] {
        let mut missing = table();
        missing.rules.retain(|(r, _)| r.id != id);
        runtime.table = Some(missing);
        assert!(runtime.safe_table().is_err());
    }
    let mut rated = table();
    rated
        .rules
        .iter_mut()
        .find(|(r, _)| r.id == 19)
        .unwrap()
        .0
        .value = RuleValue::Boolean(true);
    runtime.table = Some(rated);
    assert!(runtime.safe_table().is_err());
    let mut observable = table();
    observable
        .rules
        .iter_mut()
        .find(|(r, _)| r.id == 5)
        .unwrap()
        .0
        .value = RuleValue::Group(Group::Everybody);
    runtime.table = Some(observable);
    assert!(runtime.safe_table().is_err());
}
#[test]
fn conflicting_duplicate_privacy_rules_do_not_prove_a_safe_table() {
    let (_directory, mut runtime) = runtime();
    runtime.table.as_mut().unwrap().rules.push((
        TableRule {
            id: 19,
            value: RuleValue::Boolean(true),
        },
        true,
    ));
    assert!(runtime.safe_table().is_err());
}
#[test]
fn initial_login_reporting_an_existing_game_does_not_acquire_authority() {
    let (directory, mut runtime) = runtime();
    let error = runtime.process(Message::Login(login(true))).unwrap_err();
    assert!(error.contains("no automatic takeover or reconnect"));
    assert!(runtime.session.is_none());
    assert!(runtime.socket.is_none());
    assert!(!directory.path().join("session.json").exists());
    assert!(runtime.update.reducer.game_id.is_none());
}
#[test]
fn persisted_login_metadata_does_not_contain_the_session_authority() {
    let (directory, mut runtime) = runtime();
    runtime.process(Message::Login(login(false))).unwrap();
    let metadata = std::fs::read_to_string(directory.path().join("session.json")).unwrap();
    assert!(metadata.contains("fixture-account"));
    assert!(!metadata.contains("private-fixture-authority"));
    assert!(runtime.session.is_some());
}
#[tokio::test]
async fn an_old_decision_cannot_consume_the_replacement_question() {
    let (_directory, mut runtime) = active_runtime();
    let stale = decision_id(&runtime);
    runtime.process(Message::Question(question(8))).unwrap();
    let current = decision_id(&runtime);
    assert_ne!(stale, current);
    let error = runtime
        .answer(Answer::indices(stale, vec![1]), None)
        .await
        .unwrap_err();
    assert!(error.contains("is not open"), "{error}");
    assert_eq!(decision_id(&runtime), current);
    assert!(runtime.update.error.is_none());
}
#[tokio::test]
async fn a_failed_send_does_not_leave_an_answer_available_for_replay() {
    let (_directory, mut runtime) = active_runtime();
    let answer = Answer::indices(decision_id(&runtime), vec![1]);
    let error = runtime.answer(answer.clone(), None).await.unwrap_err();
    assert!(error.contains("connection closed"), "{error}");
    assert!(runtime.update.reducer.view().unwrap().decision.is_none());
    let duplicate = runtime.answer(answer, None).await.unwrap_err();
    assert!(
        duplicate.contains("no open official decision"),
        "{duplicate}"
    );
}
#[tokio::test]
async fn snapshot_replacement_invalidates_an_answer_from_the_previous_epoch() {
    let (_directory, mut runtime) = active_runtime();
    let old_answer = Answer::indices(decision_id(&runtime), vec![1]);
    runtime.process(Message::Snapshot(snapshot())).unwrap();
    runtime.process(Message::Question(question(7))).unwrap();
    let current = decision_id(&runtime);
    let error = runtime.answer(old_answer, None).await.unwrap_err();
    assert!(error.contains("is not open"), "{error}");
    assert_eq!(decision_id(&runtime), current);
}

#[tokio::test]
async fn reconnect_barrier_does_not_consume_a_question_before_fresh_snapshot() {
    let (_directory, mut runtime) = active_runtime();
    let id = decision_id(&runtime);
    runtime.needs_snapshot = true;
    let error = runtime
        .answer(Answer::indices(id.clone(), vec![1]), None)
        .await
        .unwrap_err();
    assert!(error.contains("reconnect snapshot"), "{error}");
    assert_eq!(decision_id(&runtime), id);
}

#[tokio::test]
async fn changed_table_membership_blocks_an_existing_answer() {
    let (_directory, mut runtime) = active_runtime();
    let id = decision_id(&runtime);
    runtime.table.as_mut().unwrap().players.push(named(99));
    let error = runtime
        .answer(Answer::indices(id.clone(), vec![1]), None)
        .await
        .unwrap_err();
    assert!(error.starts_with("REFUSED:"), "{error}");
    assert_eq!(decision_id(&runtime), id);
}

#[tokio::test]
async fn receive_failure_closes_and_invalidates_before_a_later_action() {
    let (_directory, mut runtime) = active_runtime();
    let answer = Answer::indices(decision_id(&runtime), vec![1]);
    let error = runtime.receive().await.unwrap_err();
    assert!(error.contains("connection closed"), "{error}");
    assert!(!runtime.update.connected);
    assert!(runtime.update.error.is_some());
    assert!(runtime.update.reducer.view().unwrap().decision.is_none());
    assert!(runtime.answer(answer, None).await.is_err());
}

#[test]
fn automatic_reconnect_waits_for_the_server_without_requesting_a_second_snapshot() {
    let (_directory, mut runtime) = active_runtime();
    runtime.process(Message::Login(login(true))).unwrap();
    assert!(runtime.needs_snapshot);
    assert!(!runtime.request_game);
}

#[test]
fn interrupted_reconnect_requests_one_game_and_requires_owned_authority() {
    let (_directory, mut runtime) = runtime();
    let mut interrupted = login(false);
    interrupted.previous_reconnects = 1;
    assert!(
        runtime
            .process(Message::Login(interrupted.clone()))
            .is_err()
    );
    assert!(runtime.session.is_none());
    runtime.resuming = true;
    runtime.process(Message::Login(interrupted)).unwrap();
    assert!(runtime.needs_snapshot);
    assert!(runtime.request_game);
}

#[test]
fn leaving_own_finished_table_releases_membership_without_erasing_the_result() {
    let (_directory, mut runtime) = active_runtime();
    runtime.update.reducer.running = false;
    runtime
        .process(Message::PlayerLeft {
            table_id: 10,
            player_id: 42,
        })
        .unwrap();
    assert!(runtime.table.is_none());
    assert!(runtime.bots.is_empty());
    assert_eq!(runtime.update.reducer.game_id, Some(123));
}
#[tokio::test]
async fn new_game_does_not_leave_an_existing_unfinished_table() {
    let (_directory, mut runtime) = runtime();
    let error = runtime.new_game(NewGame::default()).await.unwrap_err();
    assert!(error.contains("unfinished"), "{error}");
    assert!(runtime.table.is_some());
}

#[test]
fn decision_log_replacement_does_not_erase_the_next_question() {
    let (_directory, mut runtime) = active_runtime();
    let id = decision_id(&runtime);
    runtime.update.reducer.logs.push(LogEntry {
        seq: 5,
        kind: "decision".into(),
        player: Some(1),
        card: None,
        text: "fixture".into(),
    });
    runtime
        .process(Message::Log(model::LogChange {
            start_index: 5,
            entries: vec![model::LogEntry::Decision {
                decision_index: 1,
                player: 1,
                decision: vec![0],
                autoplayed: false,
            }],
        }))
        .unwrap();
    assert_eq!(decision_id(&runtime), id);
    assert_eq!(runtime.update.reducer.logs.len(), 1);
}

#[tokio::test]
async fn resign_consumes_the_open_decision_even_if_transmission_fails() {
    let (_directory, mut runtime) = active_runtime();
    assert!(runtime.resign().await.is_err());
    assert!(runtime.update.reducer.view().unwrap().decision.is_none());
}

#[tokio::test]
async fn hosted_http_and_mcp_require_owner_and_stream_read_only_updates() {
    use dominion_protocol::{api::OnlineObservation, lobby::Credentials};
    use futures_util::StreamExt;
    use std::net::{IpAddr, Ipv4Addr};
    let (_directory, runtime) = active_runtime();
    let (publish, updates) = watch::channel(runtime.update.clone());
    let (tx, mut requests) = mpsc::channel(16);
    let client = Client { tx, updates };
    let hosted = crate::online::Hosted::new(client.clone(), "observer-owner".into());
    let arena = crate::arena::Arena::open(Arc::new(crate::hub::Hub::default()), Default::default())
        .unwrap();
    let ip = IpAddr::V4(Ipv4Addr::LOCALHOST);
    let (_, owner) = arena
        .register(
            ip,
            &Credentials {
                name: "observer-owner".into(),
                password: "fixture-password".into(),
            },
            None,
        )
        .unwrap();
    let owner = owner.unwrap();
    let (_, outsider) = arena
        .register(
            ip,
            &Credentials {
                name: "observer-outsider".into(),
                password: "fixture-password".into(),
            },
            None,
        )
        .unwrap();
    let outsider = outsider.unwrap();
    let ident = arena.identify(&owner).unwrap();
    let bot = arena.new_bot(&ident, "observer-agent").unwrap();
    let stop = tokio_util::sync::CancellationToken::new();
    let router = hosted.router(arena.clone(), vec!["127.0.0.1".into()], None, stop.clone());
    let router = axum::Router::new()
        .nest_service(
            "/mcp",
            axum::Router::new().fallback(|| async { axum::http::StatusCode::IM_A_TEAPOT }),
        )
        .merge(router);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let stopped = stop.clone();
    let serving = tokio::spawn(async move {
        axum::serve(listener, router)
            .with_graceful_shutdown(stopped.cancelled_owned())
            .await
            .unwrap()
    });
    let http = reqwest::Client::new();
    for path in ["/api/online/view", "/api/online/events", "/mcp/online"] {
        assert_eq!(
            http.get(format!("{base}{path}"))
                .send()
                .await
                .unwrap()
                .status(),
            reqwest::StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            http.get(format!("{base}{path}"))
                .bearer_auth(&outsider)
                .send()
                .await
                .unwrap()
                .status(),
            reqwest::StatusCode::FORBIDDEN
        );
    }
    let response = http
        .get(format!("{base}/api/online/view"))
        .bearer_auth(&bot.key)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    assert_eq!(response.headers()["cache-control"], "no-store");
    let observed: OnlineObservation =
        serde_json::from_str(&response.text().await.unwrap()).unwrap();
    assert!(observed.view.as_ref().unwrap().spectating);
    assert!(observed.view.as_ref().unwrap().decision.is_none());
    assert!(client.current().view().unwrap().unwrap().decision.is_some());
    let agent = crate::client::Client::connect_authenticated(
        &format!("{base}/mcp/online"),
        Some(bot.key.clone()),
    )
    .await
    .unwrap();
    let reply = agent.call("state", serde_json::json!({})).await.unwrap();
    assert!(reply.ok, "{}", reply.text);
    assert!(!reply.json.unwrap()["view"]["decision"].is_null());
    agent.close().await;
    // Management is operator-only, and hosted MCP cannot bypass the runner's lifecycle.
    let manager = format!("{base}/api/online");
    let response = http
        .post(format!("{manager}/start"))
        .bearer_auth(&owner)
        .json(&serde_json::json!({"agent":"fixture-agent"}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::CONFLICT);
    assert!(requests.try_recv().is_err());
    let (offering, mut jobs) = mpsc::unbounded_channel();
    arena.runner_connected(
        vec![crate::house::Offer {
            name: "fixture-agent".into(),
            about: "fixture".into(),
        }],
        1,
        offering,
    );
    let _ = jobs.recv().await.unwrap();
    let response = http
        .post(format!("{manager}/start"))
        .bearer_auth(&outsider)
        .json(&serde_json::json!({"agent":"fixture-agent"}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::FORBIDDEN);
    let response = http
        .post(format!("{manager}/start"))
        .bearer_auth(&owner)
        .header("origin", "https://elsewhere.example")
        .json(&serde_json::json!({"agent":"fixture-agent"}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::FORBIDDEN);
    assert!(requests.try_recv().is_err());
    let backend = tokio::spawn(async move {
        match requests.recv().await.unwrap() {
            Request::NewGame(options, reply) => {
                assert_eq!(options.bots, Some(1));
                assert_eq!(options.players, Some(2));
                reply.send(Ok(())).unwrap();
            }
            _ => panic!("expected a bot game request"),
        }
        requests
    });
    let response = http
        .post(format!("{manager}/start"))
        .bearer_auth(&owner)
        .json(&serde_json::json!({"agent":"fixture-agent"}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    let mut requests = backend.await.unwrap();
    let first = arena.official_job().unwrap();
    assert!(first.official);
    assert!(!first.transcript);
    let crate::house::ToRunner::Jobs { jobs } = jobs.recv().await.unwrap() else {
        panic!("job dispatch")
    };
    assert_eq!(jobs, vec![first.clone()]);
    let capability = crate::client::Client::connect_authenticated(
        &format!("{base}/mcp/online"),
        Some(first.token.clone()),
    )
    .await
    .unwrap();
    assert!(
        capability
            .call("state", serde_json::json!({}))
            .await
            .unwrap()
            .ok
    );
    for tool in ["new_game", "resign", "online_reconnect"] {
        let reply = capability.call(tool, serde_json::json!({})).await.unwrap();
        assert!(!reply.ok, "{tool} unexpectedly allowed");
    }
    assert!(requests.try_recv().is_err());
    let response = http
        .get(format!("{manager}/manage"))
        .bearer_auth(&first.token)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::UNAUTHORIZED);
    let response = http
        .delete(format!("{manager}/agent"))
        .bearer_auth(&owner)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    assert!(arena.official_job().is_none());
    let response = http
        .get(format!("{base}/mcp/online"))
        .bearer_auth(&first.token)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::UNAUTHORIZED);
    capability.close().await;
    let response = http
        .post(format!("{manager}/agent"))
        .bearer_auth(&owner)
        .json(&serde_json::json!({"agent":"fixture-agent"}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    assert_ne!(arena.official_job().unwrap().token, first.token);
    http.delete(format!("{manager}/agent"))
        .bearer_auth(&owner)
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap();
    let mut searching = client.current();
    searching.reducer.running = false;
    searching.reducer.invalidate();
    searching.searching = true;
    searching.mode = OnlineMode::Rated;
    let queue_publish = publish.clone();
    let backend = tokio::spawn(async move {
        match requests.recv().await.unwrap() {
            Request::Match(mode, options, reply) => {
                assert_eq!(mode, OnlineMode::Rated);
                assert_eq!(options.players, 2);
                queue_publish.send_replace(searching);
                reply.send(Ok(())).unwrap();
            }
            _ => panic!("expected public matchmaking request"),
        }
        requests
    });
    let response = http
        .post(format!("{manager}/start"))
        .bearer_auth(&owner)
        .json(&serde_json::json!({"agent":"fixture-agent","mode":"rated"}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    let mut requests = backend.await.unwrap();
    assert!(arena.official_job().unwrap().game_id.is_empty());
    // The reserved slot cannot be reused while waiting for the public opponent.
    assert!(
        arena
            .reserve_official("observer-owner", "fixture-agent")
            .is_err()
    );
    let mut between = client.current();
    between.searching = false;
    publish.send_replace(between);
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert!(
        arena.official_job().is_some(),
        "queue-to-match notification gap must keep the reservation"
    );
    let status: serde_json::Value = http
        .get(format!("{manager}/manage"))
        .bearer_auth(&owner)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(status["searching"], true);
    let response = http
        .post(format!("{manager}/reconnect"))
        .bearer_auth(&owner)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::CONFLICT);
    let matched = runtime.update.clone();
    let queue_publish = publish.clone();
    let backend = tokio::spawn(async move {
        match requests.recv().await.unwrap() {
            Request::CancelMatch(reply) => {
                queue_publish.send_replace(matched);
                reply.send(Ok(())).unwrap();
            }
            _ => panic!("expected matchmaking cancellation"),
        }
    });
    let response = http
        .delete(format!("{manager}/matchmaking"))
        .bearer_auth(&owner)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    backend.await.unwrap();
    let assigned = arena.official_job().unwrap();
    assert!(
        !assigned.game_id.is_empty(),
        "a cancellation race must assign the game instead of abandoning it"
    );
    assert!(!assigned.token.is_empty());
    let response = http
        .get(format!("{base}/api/online/events"))
        .header("cookie", format!("dominion_session={owner}"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    let mut stream = response.bytes_stream();
    let first = tokio::time::timeout(Duration::from_secs(2), stream.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(
        std::str::from_utf8(&first)
            .unwrap()
            .contains("event: change")
    );
    let mut next = runtime.update.clone();
    next.reducer.revision += 1;
    publish.send_replace(next.clone());
    let changed = tokio::time::timeout(Duration::from_secs(2), stream.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(
        std::str::from_utf8(&changed)
            .unwrap()
            .contains(&format!("\"version\":{}", next.reducer.revision))
    );
    arena.logout(&owner);
    next.reducer.revision += 1;
    publish.send_replace(next);
    assert!(
        tokio::time::timeout(Duration::from_secs(2), stream.next())
            .await
            .unwrap()
            .is_none()
    );
    let response = http
        .get(format!("{base}/api/online/events"))
        .bearer_auth(&bot.key)
        .send()
        .await
        .unwrap();
    let mut shutdown_stream = response.bytes_stream();
    let _ = tokio::time::timeout(Duration::from_secs(2), shutdown_stream.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    stop.cancel();
    assert!(
        tokio::time::timeout(Duration::from_secs(2), shutdown_stream.next())
            .await
            .unwrap()
            .is_none()
    );
    tokio::time::timeout(Duration::from_secs(2), serving)
        .await
        .unwrap()
        .unwrap();
}

fn human_table(rated: bool) -> TableDetails {
    let mut table = table();
    table.host_id = 99;
    table.players.push(named(99));
    table.ready.push(true);
    table.bots.clear();
    table.spectators.push(named(100));
    table
        .rules
        .iter_mut()
        .find(|(r, _)| r.id == 19)
        .unwrap()
        .0
        .value = RuleValue::Boolean(rated);
    table
}
fn match_found(table: TableDetails) -> Message {
    Message::Ancillary(
        dominion_online::transport::ancillary::Message::AutomatchFound {
            table,
            is_bot_game: false,
        },
    )
}
#[test]
fn human_matching_requires_explicit_search_and_checks_roster_rating_and_snapshot() {
    for mode in [OnlineMode::Rated, OnlineMode::Unrated] {
        let (_directory, mut runtime) = runtime();
        runtime.process(Message::Login(login(false))).unwrap();
        let matched = human_table(mode == OnlineMode::Rated);
        assert!(runtime.process(match_found(matched.clone())).is_err());
        runtime.search = Some(OnlineMatching::default());
        runtime.update.mode = mode;
        let mut wrong_rating = matched.clone();
        wrong_rating
            .rules
            .iter_mut()
            .find(|(r, _)| r.id == 19)
            .unwrap()
            .0
            .value = RuleValue::Boolean(mode != OnlineMode::Rated);
        assert!(runtime.process(match_found(wrong_rating)).is_err());
        runtime.search = Some(OnlineMatching::default());
        runtime.process(match_found(matched.clone())).unwrap();
        assert!(runtime.update.searching);
        assert!(runtime.safe_table().is_ok());
        let mut snapshot = snapshot();
        snapshot.state.players[1].player_id = 99;
        runtime
            .process(Message::Snapshot(snapshot.clone()))
            .unwrap();
        assert!(!runtime.update.searching);
        assert!(runtime.update.view().unwrap().unwrap().game.running);
        snapshot.state.players[1].player_id = 101;
        assert!(runtime.process(Message::Snapshot(snapshot)).is_err());
        let mut foreign = matched;
        foreign.players[1].id = 101;
        assert!(runtime.process(Message::TableDetails(foreign)).is_err());
        runtime
            .process(Message::Metagame {
                kind: 0,
                player: 99,
                decision: 7,
            })
            .unwrap();
        let bytes = runtime.outbox.pop().unwrap();
        let mut reader = Reader::new(&bytes);
        assert_eq!(reader.int().unwrap(), 42);
        assert_eq!(reader.int().unwrap(), 99);
        assert_eq!(reader.int().unwrap(), 7);
    }
}
#[test]
fn matching_criteria_match_official_player_rating_timer_and_cancellation_fields() {
    for (mode, players, rated) in [(OnlineMode::Rated, 2, 2), (OnlineMode::Unrated, 3, 0)] {
        for searching in [true, false] {
            let options = OnlineMatching {
                players,
                ..Default::default()
            };
            let bytes = matching::criteria(mode, &options, searching).unwrap();
            let mut reader = Reader::new(&bytes);
            assert_eq!(reader.int().unwrap(), 22);
            assert_eq!(reader.int().unwrap(), 0);
            assert_eq!(reader.boolean().unwrap(), searching);
            assert_eq!(reader.int().unwrap(), 10);
            assert_eq!(reader.int().unwrap(), -10);
            assert_eq!(reader.int().unwrap(), 4);
            for (key, value) in [
                (0, 0),
                (1, if players == 2 { 2 } else { 0 }),
                (2, rated),
                (3, 0),
            ] {
                assert_eq!(reader.int().unwrap(), key);
                assert_eq!(reader.int().unwrap(), value);
            }
            assert_eq!(reader.int().unwrap(), 4);
            for key in 0..4 {
                assert_eq!(reader.int().unwrap(), key);
                assert_eq!(reader.int().unwrap(), if key == 3 { 4 } else { -1 });
            }
        }
    }
    for options in [
        OnlineMatching {
            players: 1,
            ..Default::default()
        },
        OnlineMatching {
            timer: 4,
            ..Default::default()
        },
        OnlineMatching {
            min_rating: 20,
            ..Default::default()
        },
    ] {
        assert!(matching::criteria(OnlineMode::Rated, &options, true).is_err());
    }
}

#[test]
fn a_completed_table_already_left_by_the_server_does_not_block_the_next_game() {
    let (_directory, mut runtime) = active_runtime();
    let failure = || {
        Message::Failed(dominion_online::transport::Failure {
            reason: "COMMAND_REQUIRES_TABLE",
            card: None,
        })
    };
    runtime.leaving_finished = true;
    assert!(runtime.process(failure()).is_err());
    assert!(runtime.table.is_some());
    runtime.update.reducer.running = false;
    assert!(runtime.process(failure()).is_err());
    runtime.update.reducer.result = Some(dominion_protocol::GameResult {
        reason: None,
        players: vec![],
    });
    runtime.leaving_finished = false;
    assert!(runtime.process(failure()).is_err());
    runtime.leaving_finished = true;
    runtime.process(failure()).unwrap();
    assert!(runtime.table.is_none());
    assert!(runtime.bots.is_empty());
    assert!(!runtime.leaving_finished);
    assert!(runtime.update.reducer.result.is_some());
}
