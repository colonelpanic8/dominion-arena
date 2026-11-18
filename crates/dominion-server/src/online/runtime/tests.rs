use super::*;
use dominion_online::{
    model,
    questions::{Question, QuestionDescription, QuestionKind, Story},
    transport::{LoginSuccess, NamedId},
    wire::{Reader, Writer},
};
use dominion_protocol::LogEntry;
use std::collections::BTreeMap;
use tempfile::TempDir;

/// The requests a hosted account's backend gets, without those for its game's players'
/// levels.
fn without_levels(mut requests: mpsc::Receiver<Request>) -> mpsc::Receiver<Request> {
    let (tx, rest) = mpsc::channel(16);
    tokio::spawn(async move {
        while let Some(request) = requests.recv().await {
            if !matches!(request, Request::Profile(Some(_), ..)) && tx.send(request).await.is_err()
            {
                break;
            }
        }
    });
    rest
}

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
        completed_table: None,
        resuming: false,
        endpoint: Endpoint::Alpha,
        session: None,
        table: Some(table()),
        bots: vec![-2],
        human: None,
        search: None,
        queue_status: None,
        outbox: Vec::new(),
        pending_match: Vec::new(),
        profiles: Waiting::default(),
        results: Waiting::default(),
        update: Update {
            reducer: Reducer::new(catalog(), 42),
            error: None,
            connected: true,
            searching: false,
            queue_feedback: Vec::new(),
            mode: OnlineMode::Bots,
            timers: Vec::new(),
            finished_at: None,
            bots: Vec::new(),
        },
        recorder: Recorder::new(recordings::dir(directory.path())),
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
/// An account in a game with a decision open and `logs` behind it. Its answers reach no
/// server: one the client accepts is consumed, then fails to send.
pub(crate) fn fixture_client(logs: Vec<LogEntry>) -> (TempDir, Client) {
    let (directory, mut runtime) = active_runtime();
    runtime.update.reducer.logs = logs;
    let (publish, updates) = watch::channel(runtime.update.clone());
    let (tx, mut requests) = mpsc::channel(16);
    tokio::spawn(async move {
        while let Some(request) = requests.recv().await {
            if let Request::Answer(answer, expect, reply) = request {
                let result = runtime.answer(answer, expect).await;
                publish.send_replace(runtime.update.clone());
                let _ = reply.send(result);
            }
        }
    });
    (directory, Client { tx, updates })
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
fn player_joins_update_the_table_without_disconnecting() {
    for is_player in [false, true] {
        let (_directory, mut runtime) = runtime();
        let joined = Message::PlayerJoined {
            table_id: 10,
            player: named(99),
            is_player,
        };
        runtime.process(joined.clone()).unwrap();
        runtime.process(joined).unwrap();
        let table = runtime.table.as_ref().unwrap();
        let members = if is_player {
            &table.players
        } else {
            &table.spectators
        };
        assert_eq!(members.iter().filter(|p| p.id == 99).count(), 1);
        assert_eq!(table.players[0].id, 42);
        assert_eq!(table.ready.len(), table.players.len());
        assert_eq!(runtime.update.reducer.names.get(&99), Some(&named(99).name));
        assert!(runtime.safe_table().is_err());
        runtime
            .process(Message::PlayerLeft {
                table_id: 10,
                player_id: 99,
            })
            .unwrap();
        let table = runtime.safe_table().unwrap();
        assert_eq!(table.ready.len(), table.players.len());
    }
}

#[test]
fn table_updates_with_humans_do_not_disconnect() {
    for joined in [false, true] {
        let (_directory, mut runtime) = runtime();
        let table = human_table(false);
        let message = if joined {
            Message::TableJoined {
                table: table.clone(),
                show: true,
            }
        } else {
            Message::TableDetails(table.clone())
        };
        runtime.process(message).unwrap();
        assert_eq!(runtime.table.as_ref().unwrap().players, table.players);
        assert!(runtime.safe_table().is_err());
    }
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
async fn hosted_watching_is_public_but_management_and_mcp_require_owner() {
    use dominion_protocol::{api::OnlineObservation, lobby::Credentials};
    use futures_util::StreamExt;
    use std::net::{IpAddr, Ipv4Addr};
    let (_directory, runtime) = active_runtime();
    let (publish, updates) = watch::channel(runtime.update.clone());
    let (tx, requests) = mpsc::channel(16);
    let mut requests = without_levels(requests);
    let client = Client { tx, updates };
    let hosted = crate::online::Hosted::new(
        client.clone(),
        crate::online::LEGACY_ID.into(),
        "fixture-42".into(),
        "observer-owner".into(),
    );
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
    let router = crate::online::router(
        &[hosted],
        arena.clone(),
        vec!["127.0.0.1".into()],
        None,
        stop.clone(),
    );
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
    for path in ["/api/online/manage", "/mcp/online"] {
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
    for token in [None, Some(&outsider)] {
        for path in ["/api/online/view", "/api/online/events"] {
            let mut request = http.get(format!("{base}{path}"));
            if let Some(token) = token {
                request = request.bearer_auth(token);
            }
            let response = request.send().await.unwrap();
            assert_eq!(response.status(), reqwest::StatusCode::OK);
            assert_eq!(response.headers()["cache-control"], "no-store");
        }
        for (method, path) in [
            (reqwest::Method::POST, "start"),
            (reqwest::Method::POST, "agent"),
            (reqwest::Method::DELETE, "agent"),
            (reqwest::Method::DELETE, "matchmaking"),
            (reqwest::Method::POST, "reconnect"),
            (reqwest::Method::POST, "resign"),
        ] {
            let mut request = http.request(method, format!("{base}/api/online/{path}"));
            if let Some(token) = token {
                request = request.bearer_auth(token);
            }
            let response = request.send().await.unwrap();
            assert_eq!(
                response.status(),
                if token.is_some() {
                    reqwest::StatusCode::FORBIDDEN
                } else {
                    reqwest::StatusCode::UNAUTHORIZED
                }
            );
        }
    }
    let response = http
        .get(format!("{base}/api/online/view"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    assert_eq!(response.headers()["cache-control"], "no-store");
    let observed: OnlineObservation =
        serde_json::from_str(&response.text().await.unwrap()).unwrap();
    let watched = observed.game.unwrap();
    assert!(watched.view.spectating && watched.view.decision.is_none());
    // The account is the one asked, and a snapshot has come.
    assert_eq!((watched.deciding, watched.undos), (Some(0), 1));
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
        ["fixture-agent", "other-agent"]
            .map(|name| crate::house::Offer {
                name: name.into(),
                about: "fixture".into(),
                ..Default::default()
            })
            .into(),
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
    let first = arena.official_job(crate::online::LEGACY_ID).unwrap();
    assert!(first.official);
    assert!(first.transcript);
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
    // Only the playing agent adds to the game's transcript, which anyone may read.
    let entry = serde_json::json!({
        "attempt": "attempt-1", "offset": 0, "time": 1, "kind": "message",
        "block": "b1", "append": false, "text": "Opening with Silver.",
    });
    for (token, status) in [
        (&owner, reqwest::StatusCode::FORBIDDEN),
        (&first.token, reqwest::StatusCode::NO_CONTENT),
    ] {
        let response = http
            .post(format!("{manager}/transcript"))
            .bearer_auth(token)
            .json(&entry)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), status);
    }
    let kept = dominion_protocol::lobby::official_game_id(&first.game_id);
    assert!(crate::arena::is_official(&kept));
    let page = arena.transcript(&kept, 0).unwrap();
    assert_eq!(page.entries.len(), 1);
    assert_eq!(page.entries[0].agent, "fixture-agent");
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
    assert!(arena.official_job(crate::online::LEGACY_ID).is_none());
    let response = http
        .get(format!("{base}/mcp/online"))
        .bearer_auth(&first.token)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::UNAUTHORIZED);
    capability.close().await;
    // The paused game keeps its agent: no other may take it over.
    let response = http
        .post(format!("{manager}/agent"))
        .bearer_auth(&owner)
        .json(&serde_json::json!({"agent":"other-agent"}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::CONFLICT);
    assert!(arena.official_job(crate::online::LEGACY_ID).is_none());
    let managed: dominion_protocol::api::OnlineManagement = http
        .get(format!("{manager}/manage"))
        .bearer_auth(&owner)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(managed.game_agent.as_deref(), Some("fixture-agent"));
    let response = http
        .post(format!("{manager}/agent"))
        .bearer_auth(&owner)
        .json(&serde_json::json!({"agent":"fixture-agent"}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    assert_ne!(
        arena.official_job(crate::online::LEGACY_ID).unwrap().token,
        first.token
    );
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
    assert!(
        arena
            .official_job(crate::online::LEGACY_ID)
            .unwrap()
            .game_id
            .is_empty()
    );
    // The reserved slot cannot be reused while waiting for the public opponent.
    assert!(
        arena
            .reserve_official("observer-owner", crate::online::LEGACY_ID, "fixture-agent")
            .is_err()
    );
    let mut between = client.current();
    between.searching = false;
    publish.send_replace(between);
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert!(
        arena.official_job(crate::online::LEGACY_ID).is_some(),
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
    let assigned = arena.official_job(crate::online::LEGACY_ID).unwrap();
    assert!(
        !assigned.game_id.is_empty(),
        "a cancellation race must assign the game instead of abandoning it"
    );
    assert!(!assigned.token.is_empty());
    let response = http
        .get(format!("{base}/api/online/events"))
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
            .unwrap()
            .is_ok()
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

#[tokio::test]
async fn official_accounts_keep_their_own_owners_routes_and_agent_tokens() {
    use dominion_protocol::{api::OnlineAccountInfo, lobby::Credentials};
    use std::net::{IpAddr, Ipv4Addr};
    let (_a_directory, a_runtime) = active_runtime();
    let (_b_directory, b_runtime) = active_runtime();
    let (_a_publish, a_updates) = watch::channel(a_runtime.update.clone());
    let (_b_publish, b_updates) = watch::channel(b_runtime.update.clone());
    let (a_tx, a_requests) = mpsc::channel(16);
    let mut a_requests = without_levels(a_requests);
    let (b_tx, b_requests) = mpsc::channel(16);
    let mut b_requests = without_levels(b_requests);
    let hosted = [
        crate::online::Hosted::new(
            Client {
                tx: a_tx,
                updates: a_updates,
            },
            crate::online::LEGACY_ID.into(),
            "fixture-a".into(),
            "owner-a".into(),
        ),
        crate::online::Hosted::new(
            Client {
                tx: b_tx,
                updates: b_updates,
            },
            "sol".into(),
            "fixture-sol".into(),
            "owner-sol".into(),
        ),
    ];
    let arena = crate::arena::Arena::open(Arc::new(crate::hub::Hub::default()), Default::default())
        .unwrap();
    let ip = IpAddr::V4(Ipv4Addr::LOCALHOST);
    let register = |name: &str| {
        let credentials = Credentials {
            name: name.into(),
            password: "fixture-password".into(),
        };
        arena.register(ip, &credentials, None).unwrap().1.unwrap()
    };
    let (owner_a, owner_sol) = (register("owner-a"), register("owner-sol"));
    let (offering, mut jobs) = mpsc::unbounded_channel();
    arena.runner_connected(
        vec![crate::house::Offer {
            name: "fixture-agent".into(),
            about: "fixture".into(),
            ..Default::default()
        }],
        2,
        offering,
    );
    let stop = tokio_util::sync::CancellationToken::new();
    let router = crate::online::router(
        &hosted,
        arena.clone(),
        vec!["127.0.0.1".into()],
        None,
        stop.clone(),
    );
    // As in `serve`: Arena's MCP and API, with its fallback, beside the official routes.
    let router = axum::Router::new()
        .nest_service(
            "/mcp",
            axum::Router::new().fallback(|| async { axum::http::StatusCode::IM_A_TEAPOT }),
        )
        .nest(
            "/api",
            axum::Router::new()
                .route("/settings", axum::routing::get(|| async { "" }))
                .fallback(|| async { axum::http::StatusCode::IM_A_TEAPOT }),
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
    let listed: Vec<OnlineAccountInfo> = http
        .get(format!("{base}/api/online/accounts"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(
        listed
            .iter()
            .map(|a| (a.id.as_str(), a.account.as_str(), a.primary))
            .collect::<Vec<_>>(),
        [
            ("default", "fixture-a", true),
            ("sol", "fixture-sol", false)
        ]
    );
    let status = |path: &'static str, token: Option<&String>| {
        let mut request = http.get(format!("{base}{path}"));
        if let Some(token) = token {
            request = request.bearer_auth(token);
        }
        async move { request.send().await.unwrap().status() }
    };
    use reqwest::StatusCode as S;
    assert_eq!(status("/api/online/sol/view", None).await, S::OK);
    assert_eq!(
        status("/api/online/sol/manage", None).await,
        S::UNAUTHORIZED
    );
    assert_eq!(
        status("/api/online/sol/manage", Some(&owner_a)).await,
        S::FORBIDDEN
    );
    assert_eq!(
        status("/api/online/sol/manage", Some(&owner_sol)).await,
        S::OK
    );
    assert_eq!(
        status("/api/online/manage", Some(&owner_sol)).await,
        S::FORBIDDEN
    );
    assert_eq!(status("/api/online/manage", Some(&owner_a)).await, S::OK);
    assert_eq!(
        status("/api/online/default/manage", Some(&owner_a)).await,
        S::OK
    );
    assert_eq!(
        status("/mcp/online/sol", Some(&owner_a)).await,
        S::FORBIDDEN
    );
    let response = http
        .post(format!("{base}/api/online/sol/start"))
        .bearer_auth(&owner_a)
        .json(&serde_json::json!({"agent":"fixture-agent"}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), S::FORBIDDEN);
    let backend = tokio::spawn(async move {
        match b_requests.recv().await.unwrap() {
            Request::NewGame(_, reply) => reply.send(Ok(())).unwrap(),
            _ => panic!("expected a bot game request"),
        }
        b_requests
    });
    let management: serde_json::Value = http
        .post(format!("{base}/api/online/sol/start"))
        .bearer_auth(&owner_sol)
        .json(&serde_json::json!({"agent":"fixture-agent"}))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(management["id"], "sol");
    let mut b_requests = backend.await.unwrap();
    assert!(
        a_requests.try_recv().is_err(),
        "the other account was driven"
    );
    assert!(arena.official_job(crate::online::LEGACY_ID).is_none());
    let sol = arena.official_job("sol").unwrap();
    assert_eq!(sol.official_account.as_deref(), Some("sol"));
    assert_eq!(
        crate::harness::endpoint("http://127.0.0.1:1/mcp", &sol).unwrap(),
        "http://127.0.0.1:1/mcp/online/sol"
    );
    loop {
        let crate::house::ToRunner::Jobs { jobs } =
            tokio::time::timeout(Duration::from_secs(2), jobs.recv())
                .await
                .unwrap()
                .unwrap()
        else {
            panic!("job dispatch")
        };
        if jobs.contains(&sol) {
            break;
        }
    }
    // One account's job token opens only that account's endpoint.
    for path in ["/mcp/online", "/mcp/online/default"] {
        assert_eq!(status(path, Some(&sol.token)).await, S::UNAUTHORIZED);
    }
    let agent = crate::client::Client::connect_authenticated(
        &format!("{base}/mcp/online/sol"),
        Some(sol.token.clone()),
    )
    .await
    .unwrap();
    assert!(agent.call("state", serde_json::json!({})).await.unwrap().ok);
    agent.close().await;
    // The other account takes its own agent alongside.
    let backend = tokio::spawn(async move {
        match a_requests.recv().await.unwrap() {
            Request::NewGame(_, reply) => reply.send(Ok(())).unwrap(),
            _ => panic!("expected a bot game request"),
        }
    });
    http.post(format!("{base}/api/online/start"))
        .bearer_auth(&owner_a)
        .json(&serde_json::json!({"agent":"fixture-agent"}))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap();
    backend.await.unwrap();
    assert!(b_requests.try_recv().is_err());
    let default = arena.official_job(crate::online::LEGACY_ID).unwrap();
    assert_eq!(default.official_account.as_deref(), Some("default"));
    assert_ne!(default.token, sol.token);
    assert_eq!(arena.official_job("sol").unwrap(), sol);
    assert_eq!(
        status("/mcp/online/sol", Some(&default.token)).await,
        S::UNAUTHORIZED
    );
    http.delete(format!("{base}/api/online/sol/agent"))
        .bearer_auth(&owner_sol)
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap();
    assert!(arena.official_job("sol").is_none());
    assert_eq!(
        arena.official_job(crate::online::LEGACY_ID).unwrap(),
        default
    );
    stop.cancel();
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
    assert!(runtime.process(failure()).is_err());
    runtime.completed_table = Some(10);
    runtime.process(failure()).unwrap();
    assert!(runtime.table.is_none());
    assert!(runtime.bots.is_empty());
    assert!(!runtime.leaving_finished);
    assert!(runtime.update.reducer.result.is_some());
}

#[test]
fn public_match_initialization_waits_for_confirmation_after_early_table_and_game_messages() {
    for mode in [OnlineMode::Rated, OnlineMode::Unrated] {
        let (_directory, mut runtime) = runtime();
        runtime.table = None;
        runtime.process(Message::Login(login(false))).unwrap();
        runtime.search = Some(OnlineMatching::default());
        runtime.update.mode = mode;
        runtime.update.searching = true;
        let matched = human_table(mode == OnlineMode::Rated);
        let mut provisional = matched.clone();
        provisional.players.truncate(1);
        provisional.ready.truncate(1);
        runtime
            .process(Message::TableJoined {
                table: provisional,
                show: true,
            })
            .unwrap();
        runtime
            .process(Message::PlayerJoined {
                table_id: matched.table_id,
                player: named(99),
                is_player: true,
            })
            .unwrap();
        runtime
            .process(Message::RoleChanged {
                table_id: matched.table_id,
                player_id: 99,
                is_player: true,
                ready: true,
            })
            .unwrap();
        let mut snapshot = snapshot();
        snapshot.state.players[1].player_id = 99;
        runtime.process(Message::Snapshot(snapshot)).unwrap();
        runtime.process(Message::Question(question(7))).unwrap();
        assert!(runtime.update.view().unwrap().is_none());
        assert!(
            runtime
                .safe_table()
                .unwrap_err()
                .contains("waiting for match confirmation")
        );
        runtime.process(match_found(matched)).unwrap();
        let view = runtime.update.view().unwrap().unwrap();
        assert!(view.game.running && view.decision.is_some());
        assert!(runtime.safe_table().is_ok());
        assert!(runtime.pending_match.is_empty());
    }
}
#[tokio::test]
async fn unconfirmed_gameplay_is_bounded_and_discarded_when_the_connection_closes() {
    let (_directory, mut runtime) = runtime();
    runtime.search = Some(OnlineMatching::default());
    runtime.update.mode = OnlineMode::Rated;
    for _ in 0..256 {
        runtime.process(Message::Question(question(7))).unwrap();
    }
    assert!(
        runtime
            .process(Message::Question(question(7)))
            .unwrap_err()
            .contains("before match confirmation")
    );
    assert!(runtime.update.reducer.game_id.is_none());
    runtime.close().await;
    assert!(runtime.pending_match.is_empty());
}

#[tokio::test]
async fn an_old_result_never_authorizes_leaving_a_new_provisional_table() {
    for completed_table in [None, Some(9)] {
        let (_directory, mut runtime) = active_runtime();
        runtime.update.reducer.running = false;
        runtime.update.reducer.result = Some(dominion_protocol::GameResult {
            reason: None,
            players: vec![],
        });
        runtime.completed_table = completed_table;
        let error = runtime.new_game(NewGame::default()).await.unwrap_err();
        assert!(error.contains("unfinished"), "{error}");
        assert_eq!(runtime.table.as_ref().unwrap().table_id, 10);
        assert!(runtime.update.error.is_none());
    }
}

#[test]
fn early_gameplay_does_not_bypass_rating_or_roster_confirmation() {
    for wrong_rating in [true, false] {
        let (_directory, mut runtime) = runtime();
        runtime.process(Message::Login(login(false))).unwrap();
        runtime.search = Some(OnlineMatching::default());
        runtime.update.mode = OnlineMode::Rated;
        let mut snapshot = snapshot();
        snapshot.state.players[1].player_id = if wrong_rating { 99 } else { 101 };
        runtime.process(Message::Snapshot(snapshot)).unwrap();
        runtime.process(Message::Question(question(7))).unwrap();
        assert!(runtime.update.view().unwrap().is_none());
        assert!(
            runtime
                .process(match_found(human_table(!wrong_rating)))
                .is_err()
        );
        assert!(runtime.update.reducer.game_id.is_none());
        assert!(runtime.update.view().unwrap().is_none());
    }
}

#[test]
fn match_confirmation_can_arrive_at_each_initialization_boundary() {
    for mode in [OnlineMode::Rated, OnlineMode::Unrated] {
        for confirmation_at in 0..=5 {
            let (_directory, mut runtime) = runtime();
            runtime.table = None;
            runtime.process(Message::Login(login(false))).unwrap();
            runtime.search = Some(OnlineMatching::default());
            runtime.update.mode = mode;
            runtime.update.searching = true;
            let matched = human_table(mode == OnlineMode::Rated);
            let mut snapshot = snapshot();
            snapshot.state.players[1].player_id = 99;
            let mut messages = vec![
                Message::TableJoined {
                    table: matched.clone(),
                    show: true,
                },
                Message::PlayerJoined {
                    table_id: matched.table_id,
                    player: named(99),
                    is_player: true,
                },
                Message::RoleChanged {
                    table_id: matched.table_id,
                    player_id: 99,
                    is_player: true,
                    ready: true,
                },
                Message::Snapshot(snapshot),
                Message::Question(question(7)),
            ];
            messages.insert(confirmation_at, match_found(matched));
            for (index, message) in messages.into_iter().enumerate() {
                runtime.process(message).unwrap();
                if index < confirmation_at {
                    assert!(runtime.update.view().unwrap().is_none());
                }
            }
            let view = runtime.update.view().unwrap().unwrap();
            assert!(view.game.running && view.decision.is_some());
            assert!(runtime.safe_table().is_ok());
            assert!(!runtime.update.searching);
            assert!(runtime.pending_match.is_empty());
        }
    }
}
#[test]
fn a_timeout_offer_for_an_opponent_resigns_them() {
    let (_directory, mut runtime) = active_runtime();
    let offer = |player| Message::Metagame {
        kind: TIMEOUT_OFFER,
        player,
        decision: 3,
    };
    runtime.process(offer(0)).unwrap();
    assert!(runtime.outbox.is_empty());
    runtime.process(offer(1)).unwrap();
    let bytes = runtime.outbox.pop().unwrap();
    let mut reader = Reader::new(&bytes);
    assert_eq!(reader.int().unwrap(), 40);
    assert_eq!(reader.int().unwrap(), -1);
    assert_eq!(reader.int().unwrap(), 1);
    reader.finish().unwrap();
    runtime.update.reducer.running = false;
    runtime.process(offer(1)).unwrap();
    assert!(runtime.outbox.is_empty());
}

#[test]
fn official_timers_are_kept_by_seat_and_counted_down_while_running() {
    let (_directory, mut runtime) = active_runtime();
    assert_eq!(runtime.update.clock(0), None);
    let timer = |player, time, running, free_time| TimerUpdate {
        player,
        time,
        running,
        free_time,
        maximum: 0,
    };
    runtime
        .process(Message::Timers(vec![
            timer(-2, 300_000, true, 5_000),
            timer(42, 200_000, false, 0),
        ]))
        .unwrap();
    let at = runtime.update.timers[0].1;
    let clock = runtime.update.clock(at + 8_000).unwrap();
    assert_eq!(clock.running, Some(1));
    assert_eq!(clock.left, [200_000, 297_000]);
    assert!(clock.free.is_empty(), "free time used up is none");
    // A later update for one player replaces only theirs; a new game drops them all.
    runtime
        .process(Message::Timers(vec![timer(42, 150_000, true, 0)]))
        .unwrap();
    assert_eq!(runtime.update.timers.len(), 2);
    let mut next = snapshot();
    next.game_id = 124;
    runtime.process(Message::Snapshot(next)).unwrap();
    assert!(runtime.update.timers.is_empty());
}

fn profile(player: i32, kind: i32, skill: f64) -> PlayerProfile {
    #[derive(serde::Deserialize)]
    struct Fixture {
        opcode: i32,
        hex: String,
    }
    let fixtures: Vec<Fixture> = serde_json::from_str(include_str!(
        "../../../../dominion-online/tests/fixtures/ancillary.json"
    ))
    .unwrap();
    let hex = &fixtures.iter().find(|f| f.opcode == 54).unwrap().hex;
    let bytes: Vec<u8> = (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
        .collect();
    let mut profile = PlayerProfile::parse(&mut Reader::new(&bytes)).unwrap();
    profile.player = named(player);
    profile.rating_type = kind;
    profile.summary.current_skill = skill;
    profile.period.available = false;
    profile
}

fn result(rank: i32, opponent_rank: i32) -> ProfileResult {
    use dominion_online::transport::ancillary::ResultOpponent;
    ProfileResult {
        game_id: 186_039_786,
        time_millis: 1_794_900_000_000,
        opponents: vec![ResultOpponent {
            player: named(7),
            rank: opponent_rank,
            score: 30,
            resigned: false,
        }],
        my_rank: rank,
        my_score: 40,
        rated: true,
        my_level: 48.5,
        opponent_level: 47.0,
        expected_score: 0.55,
        kingdom: vec![],
        players: vec![named(42), named(7)],
        my_resigned: false,
    }
}

#[test]
fn player_page_replies_answer_the_requests_waiting_for_their_player_and_rating_type() {
    use dominion_online::transport::{Failure, ancillary};
    let (_directory, mut runtime) = active_runtime();
    let (two, mut two_rx) = oneshot::channel();
    let (three, mut three_rx) = oneshot::channel();
    let (opponent, mut opponent_rx) = oneshot::channel();
    let (results, mut results_rx) = oneshot::channel();
    assert!(runtime.profiles.wait((42, 0), two));
    assert!(runtime.profiles.wait((42, 1), three));
    assert!(runtime.profiles.wait((7, 0), opponent));
    assert!(runtime.results.wait(1, results));
    let reply = |p| Message::Ancillary(ancillary::Message::PlayerProfile(p));
    runtime.process(reply(profile(99, 0, 30.0))).unwrap();
    assert!(two_rx.try_recv().is_err(), "someone else's profile");
    runtime.process(reply(profile(42, 0, 30.0))).unwrap();
    assert_eq!(
        two_rx.try_recv().unwrap().unwrap().summary.current_skill,
        30.0
    );
    assert!(opponent_rx.try_recv().is_err());
    runtime.process(reply(profile(7, 0, 25.0))).unwrap();
    assert_eq!(
        opponent_rx
            .try_recv()
            .unwrap()
            .unwrap()
            .summary
            .current_skill,
        25.0
    );
    assert!(three_rx.try_recv().is_err());
    runtime
        .process(Message::Ancillary(ancillary::Message::PlayerResults {
            rating_type: 0,
            results: vec![],
        }))
        .unwrap();
    assert!(results_rx.try_recv().is_err(), "another rating type's");
    // A rating service outage answers the waiting requests instead of closing the
    // connection, but stays fatal when nothing asked.
    let down = || {
        Message::Failed(Failure {
            reason: "RATING_SERVICE_DOWN",
            card: None,
        })
    };
    assert!(!runtime.process(down()).unwrap());
    assert!(three_rx.try_recv().unwrap().is_err());
    assert!(results_rx.try_recv().unwrap().is_err());
    assert!(runtime.process(down()).is_err());
}

#[tokio::test]
async fn ratings_are_public_and_asked_of_the_server_once_a_minute() {
    use dominion_protocol::api::{OnlineRating, OnlineRatingType};
    let (_directory, runtime) = active_runtime();
    let (_publish, updates) = watch::channel(runtime.update.clone());
    let (tx, mut requests) = mpsc::channel(16);
    let hosted = crate::online::Hosted::new(
        Client { tx, updates },
        crate::online::LEGACY_ID.into(),
        "fixture-42".into(),
        "rating-owner".into(),
    );
    let backend = tokio::spawn(async move {
        let mut asked = vec![];
        while let Some(request) = requests.recv().await {
            match request {
                Request::Profile(None, kind, reply) => {
                    asked.push(kind);
                    reply.send(Ok(profile(42, kind, 41.5))).unwrap();
                }
                Request::Results(_, reply) => reply.send(Ok(vec![result(1, 2)])).unwrap(),
                _ => {}
            }
        }
        asked
    });
    let arena = crate::arena::Arena::open(Arc::new(crate::hub::Hub::default()), Default::default())
        .unwrap();
    let stop = tokio_util::sync::CancellationToken::new();
    let router = crate::online::router(
        &[hosted],
        arena,
        vec!["127.0.0.1".into()],
        None,
        stop.clone(),
    );
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
    for path in ["rating?type=3p", "default/rating?type=3p", "rating"] {
        let response = http
            .get(format!("{base}/api/online/{path}"))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::OK, "{path}");
        let rating: OnlineRating = response.json().await.unwrap();
        assert_eq!(rating.skill, 41.5);
        assert!(!rating.provisional);
        assert_eq!(rating.account, "fixture-42");
        assert_eq!(rating.results[0].opponents[0].name, "fixture-7");
    }
    let bad = http
        .get(format!("{base}/api/online/rating?type=4p"))
        .send()
        .await
        .unwrap();
    assert_eq!(bad.status(), reqwest::StatusCode::BAD_REQUEST);
    stop.cancel();
    serving.await.unwrap();
    assert_eq!(
        backend.await.unwrap(),
        [
            OnlineRatingType::ThreePlayer.ordinal(),
            OnlineRatingType::TwoPlayer.ordinal()
        ]
    );
}

#[tokio::test]
async fn watchers_and_the_lobby_see_the_players_levels_once_asked() {
    use dominion_online::transport::ancillary::PlayerRatingPoint;
    let (_directory, mut runtime) = active_runtime();
    runtime.update.bots = vec![-2];
    let (_publish, updates) = watch::channel(runtime.update.clone());
    let (tx, mut requests) = mpsc::channel(16);
    let hosted = crate::online::Hosted::new(
        Client { tx, updates },
        crate::online::LEGACY_ID.into(),
        "fixture-42".into(),
        "level-owner".into(),
    );
    let (asked_tx, mut asked) = mpsc::unbounded_channel();
    tokio::spawn(async move {
        while let Some(request) = requests.recv().await {
            if let Request::Profile(Some(player), kind, reply) = request {
                asked_tx.send((player, kind)).unwrap();
                let mut profile = profile(player, kind, 30.0);
                profile.rating_history = vec![PlayerRatingPoint {
                    date_millis: 1,
                    level: 47.5,
                    skill: 30.0,
                    deviation: 2.0,
                    games: 12,
                }];
                profile.summary.current_level = 47.5;
                reply.send(Ok(profile)).unwrap();
            }
        }
    });
    let arena = crate::arena::Arena::open(Arc::new(crate::hub::Hub::default()), Default::default())
        .unwrap();
    let stop = tokio_util::sync::CancellationToken::new();
    let router = crate::online::router(
        &[hosted],
        arena.clone(),
        vec!["127.0.0.1".into()],
        None,
        stop.clone(),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let stopped = stop.clone();
    let serving = tokio::spawn(async move {
        axum::serve(listener, router)
            .with_graceful_shutdown(stopped.cancelled_owned())
            .await
            .unwrap()
    });
    assert_eq!(
        asked.recv().await.unwrap(),
        (
            42,
            dominion_protocol::api::OnlineRatingType::TwoPlayer.ordinal()
        ),
        "the bot's level is never asked"
    );
    let http = reqwest::Client::new();
    let mut levels = vec![];
    for _ in 0..100 {
        let observed: dominion_protocol::api::OnlineObservation = http
            .get(format!("{base}/api/online/view"))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        levels = observed.game.unwrap().meta.levels;
        if levels.first().copied().flatten().is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(levels, [Some(47.5), None]);
    let listed = arena.lobby(None).official;
    assert_eq!(listed[0].game.as_ref().unwrap().levels, [Some(47.5), None]);
    assert!(
        asked.try_recv().is_err(),
        "each game's levels are asked once"
    );
    stop.cancel();
    serving.await.unwrap();
}

#[test]
fn the_open_period_is_the_latest_rating() {
    use crate::online::hosted::{rating_of, result_of};
    use dominion_protocol::api::{OnlineOutcome, OnlineRatingType};
    let mut profile = profile(42, 0, 41.5);
    let last = profile.rating_history.iter().map(|p| p.date_millis).max();
    let date = last.unwrap_or(0) + 1;
    let period = &mut profile.period;
    period.available = true;
    period.results = 3;
    period.period_date_millis = date;
    period.new_skill = 43.25;
    period.new_games = 18;
    let rating = rating_of(&profile, &[], &catalog(), OnlineRatingType::TwoPlayer, 0);
    assert!(rating.provisional);
    assert_eq!((rating.skill, rating.games), (43.25, 18));
    assert_eq!(rating.period.unwrap().results, 3);
    let latest = rating.history.last().unwrap();
    assert_eq!((latest.skill, latest.date), (43.25, date));
    // Once the history holds the period, its summary is the rating again.
    profile.period.period_date_millis = last.unwrap_or(0);
    let rating = rating_of(&profile, &[], &catalog(), OnlineRatingType::TwoPlayer, 0);
    assert!(!rating.provisional);
    assert_eq!(rating.skill, 41.5);

    let outcome = |me, them| result_of(&result(me, them), &catalog()).outcome;
    assert_eq!(outcome(1, 2), OnlineOutcome::Win);
    assert_eq!(outcome(1, 1), OnlineOutcome::Tie);
    assert_eq!(outcome(2, 1), OnlineOutcome::Loss);
}

#[test]
fn a_rating_is_asked_again_soon_after_a_game_finishes() {
    use crate::online::hosted::fresh;
    let now = 1_000_000;
    assert!(fresh(now - 50_000, None, now));
    assert!(!fresh(now - 70_000, None, now));
    // Read before the game finished, or while it may still be being counted.
    assert!(!fresh(now - 10_000, Some(now - 5_000), now));
    assert!(fresh(now - 2_000, Some(now - 5_000), now));
    // Read once it has been counted.
    assert!(fresh(now - 30_000, Some(now - 60_000), now));
}

/// An account resuming its human match, saved as it would be, waiting for the server.
fn resuming_human_match() -> (TempDir, Runtime) {
    let (directory, mut runtime) = runtime();
    let human = HumanTable {
        table: human_table(true),
        mode: OnlineMode::Rated,
        owner: 42,
    };
    runtime.store.save("own-match", &human).unwrap();
    runtime.table = Some(human.table.clone());
    runtime.bots.clear();
    runtime.human = Some(human);
    runtime.update.mode = OnlineMode::Rated;
    runtime.resuming = true;
    (directory, runtime)
}

#[test]
fn another_tables_details_are_not_checked_against_the_match_being_resumed() {
    let (_directory, mut runtime) = resuming_human_match();
    let mut reconnect = login(false);
    reconnect.previous_reconnects = 1;
    runtime.process(Message::Login(reconnect)).unwrap();
    assert!(runtime.needs_snapshot);
    let mut other = human_table(false);
    other.table_id = 11;
    other.host_id = 14424;
    other.players = vec![named(14424), named(15589)];
    runtime.process(Message::TableDetails(other)).unwrap();
    assert_eq!(runtime.table.as_ref().unwrap().table_id, 10);
    let mut changed = human_table(true);
    changed.players[1].id = 101;
    assert!(runtime.process(Message::TableDetails(changed)).is_err());
}

#[test]
fn a_resumed_match_the_server_no_longer_has_is_forgotten() {
    let (directory, mut runtime) = resuming_human_match();
    runtime.process(Message::Login(login(false))).unwrap();
    assert!(runtime.human.is_none() && runtime.table.is_none());
    assert!(!directory.path().join("own-match.json").exists());
}

#[tokio::test]
async fn a_finished_game_replays_at_each_step_as_the_account_saw_it() {
    let (directory, mut runtime) = runtime();
    runtime.process(Message::Login(login(false))).unwrap();
    let replays =
        recordings::Replays::new(vec![recordings::dir(directory.path())], None, catalog());
    let game = "dominion.games-dominion.games:123";
    let seen = |runtime: &Runtime| {
        let mut view = runtime.update.reducer.view().unwrap();
        view.decision = None;
        view.spectating = true;
        serde_json::to_value(view).unwrap()
    };
    let mut views = vec![];
    runtime.process(Message::Snapshot(snapshot())).unwrap();
    runtime.published();
    views.push(seen(&runtime));
    runtime.process(Message::Question(question(7))).unwrap();
    runtime.published();
    views.push(seen(&runtime));
    let id = decision_id(&runtime);
    // There is no server to send it to, but the client has taken it.
    let _ = runtime.answer(Answer::indices(id, vec![0]), None).await;
    runtime.published();
    let turn = model::TurnDescription {
        owner_id: 1,
        turn_number: 1,
        turn_type: 0,
        controller_id: 1,
    };
    runtime
        .process(Message::Event(model::Event::TurnDescription(turn)))
        .unwrap();
    runtime.published();
    runtime
        .process(Message::Log(model::LogChange {
            start_index: 0,
            entries: vec![model::LogEntry::Decision {
                decision_index: 1,
                player: 1,
                decision: vec![0],
                autoplayed: false,
            }],
        }))
        .unwrap();
    runtime.published();
    views.push(seen(&runtime));
    assert!(
        !replays.has(game),
        "a game still being played isn't replayed"
    );
    let player = |player_id, rank| model::PlayerResult {
        player_id,
        rank,
        score: model::Score {
            total_points: 3,
            used_turns: 1,
            parts: vec![],
        },
        card_names: vec![],
        resign_index: -1,
        resignation_type: None,
    };
    runtime
        .process(Message::GameFinished {
            table: table(),
            result: model::GameResult {
                table_id: 10,
                game_id: 123,
                rating_type: None,
                empty_piles: vec![],
                players: vec![player(42, 1), player(-2, 2)],
                auto_continue: false,
            },
            continue_allowed: false,
            match_completed: true,
        })
        .unwrap();
    runtime.published();
    views.push(seen(&runtime));
    assert!(replays.has(game));
    for (step, view) in views.iter().enumerate() {
        let shown = replays.view(game, step, 0).unwrap();
        assert_eq!(
            serde_json::to_value(&shown.view).unwrap(),
            *view,
            "step {step}"
        );
        let replay = shown.replay.unwrap();
        assert_eq!((replay.step, replay.steps, replay.seat), (step, 3, Some(0)));
        assert_eq!(
            replay
                .turns
                .iter()
                .map(|t| (t.step, t.player, t.turn))
                .collect::<Vec<_>>(),
            [(0, 0, 1), (2, 1, 1)]
        );
    }
    assert_eq!(replays.view(game, 1, 0).unwrap().deciding, Some(0));
    assert!(replays.view(game, 4, 0).is_err());
}
