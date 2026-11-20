use dominion_server::archive::Archive;
use dominion_server::arena::{Arena, Ident};
use dominion_server::client::Client;
use dominion_server::config::Config;
use dominion_server::house::{Job, Offer, ToRunner};
use dominion_server::hub::Hub;
use dominion_server::hub::NewGame;
use dominion_server::t3;
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ServerCapabilities, ServerConfig,
};
use rmcp::service::RequestContext;
use rmcp::transport::streamable_http_server::{
    StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
};
use rmcp::{ErrorData, RoleServer, ServerHandler};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio_util::sync::CancellationToken;

#[derive(Default)]
struct Thread {
    items: Vec<Value>,
    view: Value,
    runs: u32,
    lookup: bool,
}

#[derive(Clone, Default)]
struct FakeT3 {
    threads: Arc<Mutex<HashMap<String, Thread>>>,
    calls: Arc<Mutex<Vec<(String, Value)>>>,
    unsupported: bool,
    failed: bool,
}

impl FakeT3 {
    fn response(&self, tool: &str, args: Value) -> Value {
        self.calls.lock().unwrap().push((tool.into(), args.clone()));
        match tool {
            "orchestrator_capabilities" => json!({"runtimeMode":"full-access", "providers":[{
                "providerInstanceId":"test-account", "driverKind":"codex", "canRunChildTask":true,
                "models":[{"id":"test-model", "options":[{"id":"reasoningEffort", "values":["low","high"]}]}]
            }]}),
            "t3_project_read" => json!({"id":"game-project"}),
            "t3_thread_launch" | "t3_thread_send" => {
                assert_eq!(args["environmentId"], "remote-test");
                let mut threads = self.threads.lock().unwrap();
                let id = if tool == "t3_thread_launch" {
                    assert_eq!(args["runtimeMode"], "full-access");
                    assert_eq!(args["modelSelection"]["instanceId"], "test-account");
                    assert_eq!(
                        args["modelSelection"]["options"],
                        json!([{"id":"reasoningEffort","value":"high"}])
                    );
                    format!("thread-{}", threads.len())
                } else {
                    args["threadId"].as_str().unwrap().into()
                };
                let thread = threads.entry(id.clone()).or_default();
                let message = args["message"].as_str().unwrap();
                assert!(!message.contains("test-credential"));
                if let Some((_, view)) = message.rsplit_once("Current view:\n") {
                    thread.view = serde_json::from_str(view).unwrap();
                }
                thread.runs += 1;
                let run = format!("run-{}", thread.runs);
                let decision = &thread.view["decision"];
                let response = if message.starts_with("The game is over. Reply now in prose") {
                    json!(format!("Debrief of {id}: the brief's plan held up."))
                } else if self.unsupported {
                    json!({"tool":"resign"})
                } else if !thread.lookup {
                    thread.lookup = true;
                    json!({"tool":"card","name":"Village"})
                } else {
                    let options = decision["options"].as_array().unwrap();
                    let end = options
                        .iter()
                        .find(|o| o["label"].as_str().unwrap().contains("End your Buy phase"));
                    let index = end.unwrap_or(&options[0])["index"].clone();
                    json!({"tool":"answer","decision_id":decision["id"],"indices":[index]})
                };
                for (kind, text) in [
                    ("user_message", message.to_string()),
                    (
                        "reasoning",
                        if thread.runs == 1 {
                            "r".repeat(120_003)
                        } else {
                            "Choose a legal move".into()
                        },
                    ),
                    (
                        "assistant_message",
                        response
                            .as_str()
                            .map_or_else(|| response.to_string(), String::from),
                    ),
                ] {
                    let position = thread.items.len();
                    thread.items.push(json!({"position":position,"itemId":format!("item-{position}"),
                        "runId":run,"type":kind,"text":text,"textTruncated":false,"nextTextOffset":null}));
                }
                json!({"threadId":id,"runId":run,"link":format!("t3://threads/{id}"),"status":"running"})
            }
            "t3_thread_wait" => {
                json!({"status":if self.failed {"failed"} else {"completed"}, "timedOut":false})
            }
            "t3_thread_read" => {
                let threads = self.threads.lock().unwrap();
                let thread = &threads[args["threadId"].as_str().unwrap()];
                let from = args["afterPosition"].as_u64().map_or(0, |n| n as usize + 1);
                let mut items = if let Some(id) = args["itemId"].as_str() {
                    vec![
                        thread
                            .items
                            .iter()
                            .find(|i| i["itemId"] == id)
                            .unwrap()
                            .clone(),
                    ]
                } else {
                    thread
                        .items
                        .iter()
                        .skip(from)
                        .take(2)
                        .cloned()
                        .collect::<Vec<_>>()
                };
                for item in &mut items {
                    let text = item["text"].as_str().unwrap().to_string();
                    let offset = args["textOffset"].as_u64().unwrap_or(0) as usize;
                    let end = (offset + 50_000).min(text.len());
                    item["text"] = json!(&text[offset..end]);
                    item["textTruncated"] = json!(end < text.len());
                    item["nextTextOffset"] = if end < text.len() {
                        json!(end)
                    } else {
                        Value::Null
                    };
                }
                let next = items
                    .last()
                    .map(|i| i["position"].clone())
                    .unwrap_or(Value::Null);
                let more = next
                    .as_u64()
                    .is_some_and(|n| (n as usize + 1) < thread.items.len());
                json!({"items":items,"nextPosition":next,"hasMore":more})
            }
            "t3_thread_interrupt" => json!({"status":"no_active_run"}),
            _ => panic!("unexpected tool {tool}"),
        }
    }
}

impl ServerHandler for FakeT3 {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
    }

    async fn call_tool(
        &self,
        req: CallToolRequestParams,
        _: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        Ok(CallToolResult::structured(
            self.response(&req.name, json!(req.arguments.unwrap_or_default())),
        )
        .into())
    }
}

async fn start_fake(fake: FakeT3, stop: CancellationToken) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/mcp", listener.local_addr().unwrap());
    let service = StreamableHttpService::new(
        move || Ok(fake.clone()),
        Arc::new(LocalSessionManager::default()),
        StreamableHttpServerConfig::default(),
    );
    let router = axum::Router::new().nest_service("/mcp", service);
    tokio::spawn(async move {
        axum::serve(listener, router)
            .with_graceful_shutdown(stop.cancelled_owned())
            .await
            .unwrap();
    });
    url
}

struct Harness {
    config: Config,
    arena: Arc<Arena>,
    who: Ident,
    token: String,
    jobs: tokio::sync::mpsc::UnboundedReceiver<ToRunner>,
    stop: CancellationToken,
    _dir: tempfile::TempDir,
}

async fn setup(fake: FakeT3) -> Harness {
    let dir = tempfile::tempdir().unwrap();
    let token_path = dir.path().join("fake-token");
    std::fs::write(&token_path, "test-credential").unwrap();
    let stop = CancellationToken::new();
    let url = start_fake(fake, stop.clone()).await;
    let hub = Arc::new(Hub::default());
    let arena = Arena::open(hub, Default::default()).unwrap();
    let (_, token) = arena
        .register(
            "127.0.0.1".parse().unwrap(),
            &dominion_protocol::lobby::Credentials {
                name: "Host".into(),
                password: "correct horse".into(),
                invite: None,
            },
            None,
        )
        .unwrap();
    let token = token.unwrap();
    let who = arena.identify(&token).unwrap();
    let (send, jobs) = tokio::sync::mpsc::unbounded_channel();
    arena.runner_connected(
        vec![Offer {
            name: "Test".into(),
            about: "T3 Code".into(),
            ..Default::default()
        }],
        2,
        send,
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let server = format!("http://{}/mcp", listener.local_addr().unwrap());
    let sessions = Arc::new(LocalSessionManager::default());
    let served = arena.clone();
    let service = StreamableHttpService::new(
        move || {
            Ok(dominion_server::mcp::Session::new(
                served.clone(),
                std::time::Duration::from_secs(600),
                false,
            ))
        },
        sessions,
        StreamableHttpServerConfig::default(),
    );
    let router = dominion_server::http::router(
        arena.clone(),
        &Default::default(),
        vec!["127.0.0.1".into()],
        stop.clone(),
    )
    .nest_service("/mcp", service);
    let stopped = stop.clone();
    tokio::spawn(async move {
        axum::serve(listener, router)
            .with_graceful_shutdown(stopped.cancelled_owned())
            .await
            .unwrap();
    });
    let config: Config = toml::from_str(&format!(
        r#"
        server = {server:?}
        agents_dir = {:?}
        [t3code]
        url = {url:?}
        token_file = {:?}
        project_id = "game-project"
        environment_id = "remote-test"
        response_seconds = 10
        [[agents]]
        name = "Test"
        program = "t3code"
        provider_instance = "test-account"
        model = "test-model"
        instructions = "Buy Provinces."
        [agents.options]
        reasoningEffort = "high"
    "#,
        dir.path().join("agents").to_str().unwrap(),
        token_path.to_str().unwrap()
    ))
    .unwrap();
    Harness {
        config,
        arena,
        who,
        token,
        jobs,
        stop,
        _dir: dir,
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        self.stop.cancel();
    }
}

fn request() -> NewGame {
    NewGame {
        players: 2,
        agents: vec![Some("Test".into()), Some("Test".into())],
        kingdom: vec!["Village".into()],
        seed: Some(42),
        turn_limit: Some(2),
        ..NewGame::default()
    }
}

fn dispatched(jobs: &mut tokio::sync::mpsc::UnboundedReceiver<ToRunner>) -> Vec<Job> {
    let mut result = vec![];
    while let Ok(message) = jobs.try_recv() {
        if let ToRunner::Jobs { jobs } = message {
            result = jobs;
        }
    }
    result
}

async fn play(h: &mut Harness, jobs: Vec<Job>) -> (std::path::PathBuf, Vec<Result<(), String>>) {
    let root = h._dir.path().join("archive");
    let mut tasks = tokio::task::JoinSet::new();
    for job in jobs {
        let mut archive = Archive::open(&root.join(format!("seat-{}", job.seat))).unwrap();
        let settings = h.config.t3code.clone().unwrap();
        let spec = h.config.agents.as_ref().unwrap()[0].clone();
        let server = h.config.server();
        tasks.spawn(async move {
            t3::play(
                &settings,
                &spec,
                &job,
                &server,
                &mut archive,
                CancellationToken::new(),
            )
            .await
        });
    }
    let mut results = vec![];
    while let Some(result) = tasks.join_next().await {
        results.push(result.unwrap());
    }
    (root, results)
}

fn records(path: &std::path::Path) -> Vec<Value> {
    std::fs::read_to_string(path.join("interactions.jsonl"))
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

#[tokio::test]
async fn normal_game_creation_runs_two_agents_through_t3_with_complete_archives() {
    let fake = FakeT3::default();
    let calls = fake.calls.clone();
    let mut h = setup(fake).await;
    let client = Client::connect_authenticated(&h.config.server(), Some(h.token.clone()))
        .await
        .unwrap();
    let created = client
        .call("create_game", serde_json::to_value(request()).unwrap())
        .await
        .unwrap();
    assert!(created.ok, "{}", created.text);
    let created = created.json.unwrap();
    assert_eq!(created["seats"], json!([]));
    let id = created["game_id"].as_str().unwrap();
    let jobs = dispatched(&mut h.jobs);
    assert_eq!(jobs.len(), 2);
    assert_eq!(jobs[0].configuration.as_ref().unwrap().seed, 42);
    assert!(h.arena.hub.watch_view(id, 0, None).unwrap().view.spectating);
    let (root, results) =
        tokio::time::timeout(std::time::Duration::from_secs(30), play(&mut h, jobs))
            .await
            .unwrap();
    assert!(results.iter().all(Result::is_ok), "{results:?}");
    assert!(!h.arena.hub.list()[0].running);
    assert!(h.arena.hub.list()[0].watchable);
    assert_eq!(h.arena.hub.list()[0].names, ["Test (1)", "Test (2)"]);
    for seat in 0..2 {
        let journal = records(&root.join(format!("seat-{seat}")));
        assert!(journal.iter().any(|r| r["kind"] == "result"));
        assert!(journal.iter().any(|r| r["kind"] == "activity"
            && r["data"]["type"] == "reasoning"
            && r["data"]["text"].as_str().unwrap().len() == 120_003));
        assert!(
            journal
                .iter()
                .any(|r| r["kind"] == "request" && r["data"]["tool"] == "card")
        );
        let text = serde_json::to_string(&journal).unwrap();
        assert!(!text.contains("test-credential"));
        let joined = journal
            .iter()
            .find(|r| r["kind"] == "request" && r["data"]["tool"] == "join_game")
            .unwrap();
        assert_eq!(joined["data"]["arguments"]["token"], "[redacted]");
        assert_eq!(journal[0]["data"]["job"]["configuration"]["seed"], 42);
    }
    let calls = calls.lock().unwrap();
    assert_eq!(
        calls
            .iter()
            .filter(|(t, _)| t == "t3_thread_launch")
            .count(),
        2
    );
    assert_eq!(
        calls
            .iter()
            .filter(|(t, _)| t == "t3_thread_interrupt")
            .count(),
        2
    );
}

#[tokio::test]
async fn invalid_options_are_rejected_without_starting_a_thread() {
    let fake = FakeT3::default();
    let calls = fake.calls.clone();
    let mut h = setup(fake).await;
    h.config.agents.as_mut().unwrap()[0]
        .options
        .insert("made-up".into(), t3::OptionValue::Bool(true));
    let settings = h.config.t3code.as_ref().unwrap();
    let client = settings.connect().await.unwrap();
    let mut archive = Archive::open(&h._dir.path().join("validation")).unwrap();
    let error = t3::validate(
        settings,
        &client,
        &h.config.agents.as_ref().unwrap()[0],
        &mut archive,
    )
    .await
    .unwrap_err();
    assert!(error.contains("unsupported T3 model option"), "{error}");
    assert!(
        !calls
            .lock()
            .unwrap()
            .iter()
            .any(|(t, _)| t == "t3_thread_launch")
    );
}

#[tokio::test]
async fn failed_runs_are_archived_and_threads_are_stopped() {
    let fake = FakeT3 {
        failed: true,
        ..FakeT3::default()
    };
    let calls = fake.calls.clone();
    let mut h = setup(fake).await;
    h.arena.create_game(Some(&h.who), &request()).unwrap();
    let jobs = dispatched(&mut h.jobs)
        .into_iter()
        .filter(|j| j.seat == 0)
        .collect();
    let (root, results) = play(&mut h, jobs).await;
    assert!(
        results
            .iter()
            .all(|r| r.as_ref().unwrap_err().contains("failed")),
        "{results:?}"
    );
    let journal = records(&root.join("seat-0"));
    assert!(journal.iter().any(|r| r["kind"] == "activity"));
    assert!(
        calls
            .lock()
            .unwrap()
            .iter()
            .any(|(t, _)| t == "t3_thread_interrupt")
    );
}

#[tokio::test]
async fn unsupported_actions_stop_after_corrections_without_resigning() {
    let mut h = setup(FakeT3 {
        unsupported: true,
        ..FakeT3::default()
    })
    .await;
    h.arena.create_game(Some(&h.who), &request()).unwrap();
    let mut jobs = dispatched(&mut h.jobs);
    jobs.retain(|j| j.seat == 0);
    let (_, results) = play(&mut h, jobs).await;
    assert!(results[0].as_ref().unwrap_err().contains("game-tool JSON"));
    assert!(h.arena.hub.list()[0].running);
}

#[tokio::test]
async fn http_creation_assigns_only_requested_seats_and_respects_privacy() {
    let mut h = setup(FakeT3::default()).await;
    let mut request = request();
    request.agents = vec![None, Some("Test".into())];
    request.watchable = Some(false);
    let base = h.config.server().trim_end_matches("/mcp").to_string();
    let response = reqwest::Client::new()
        .post(format!("{base}/api/games"))
        .bearer_auth(&h.token)
        .body(serde_json::to_string(&request).unwrap())
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::CREATED);
    let created: Value = serde_json::from_str(&response.text().await.unwrap()).unwrap();
    assert_eq!(created["seats"].as_array().unwrap().len(), 1);
    assert_eq!(created["seats"][0]["seat"], 0);
    assert_eq!(dispatched(&mut h.jobs).len(), 1);
    assert!(!h.arena.hub.list()[0].watchable);
}

#[tokio::test]
async fn creation_checks_permissions_capacity_and_seat_assignments() {
    let h = setup(FakeT3::default()).await;
    assert!(h.arena.create_game(None, &request()).is_err());
    let mut malformed = request();
    malformed.agents.pop();
    assert!(h.arena.create_game(Some(&h.who), &malformed).is_err());
    malformed.agents = vec![Some("Missing".into()), None];
    assert!(h.arena.create_game(Some(&h.who), &malformed).is_err());
    assert!(h.arena.hub.list().is_empty());
    h.arena.create_game(Some(&h.who), &request()).unwrap();
    assert!(h.arena.create_game(Some(&h.who), &request()).is_err());
}

#[tokio::test]
async fn the_existing_runner_dispatches_t3_seats_and_archives_game_completion() {
    let mut h = setup(FakeT3::default()).await;
    let socket = h._dir.path().join("agents.sock");
    h.config.agent_socket = Some(socket.clone());
    let arena = h.arena.clone();
    let server_stop = h.stop.clone();
    let mut listening = Box::pin(async move {
        dominion_server::house::listen(arena, &socket, server_stop)
            .await
            .unwrap();
    });
    assert!(futures_util::poll!(listening.as_mut()).is_pending());
    let listener_task = tokio::spawn(listening);
    let (mut changes, _) = h.arena.subscribe();
    let runner_stop = CancellationToken::new();
    let runner = tokio::spawn(dominion_server::runner::run_until(
        h.config.runner().unwrap(),
        runner_stop.clone(),
    ));
    tokio::time::timeout(std::time::Duration::from_secs(10), changes.changed())
        .await
        .unwrap()
        .unwrap();
    let created = h.arena.create_game(Some(&h.who), &request()).unwrap();
    let hub = h.arena.hub.clone();
    let mut game_changes = hub.subscribe(&created.game_id).unwrap().version;
    tokio::time::timeout(std::time::Duration::from_secs(15), async {
        while hub.list()[0].running {
            game_changes.changed().await.unwrap();
        }
    })
    .await
    .unwrap();
    runner_stop.cancel();
    tokio::time::timeout(std::time::Duration::from_secs(10), runner)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let root = h.config.agents_dir.as_ref().unwrap().join("archives");
    let mut finished = 0;
    for entry in std::fs::read_dir(root).unwrap() {
        let records = records(&entry.unwrap().path());
        if records.iter().any(|r| r["kind"] == "seat") {
            assert!(records.iter().any(|r| r["kind"] == "result"));
            assert!(records.iter().any(|r| r["kind"] == "seat_finished"));
            finished += 1;
        }
    }
    assert_eq!(finished, 2);
    listener_task.abort();
}

#[tokio::test]
async fn login_uses_oauth_pkce_and_returns_the_token_without_a_credential_file() {
    use axum::extract::{Form, Query, State};
    use axum::response::Redirect;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let root = base.clone();
    let issuer = base.clone();
    let router = axum::Router::new()
        .route("/.well-known/oauth-protected-resource/mcp", axum::routing::get(async move || axum::Json(json!({
            "resource":format!("{root}/mcp"), "authorization_servers":[root],
            "scopes_supported":["orchestration:read","orchestration:operate"]
        }))))
        .route("/.well-known/oauth-authorization-server", axum::routing::get(async move || axum::Json(json!({
            "issuer":issuer,"authorization_endpoint":format!("{issuer}/authorize"),
            "token_endpoint":format!("{issuer}/token"),"registration_endpoint":format!("{issuer}/register"),
            "response_types_supported":["code"],"code_challenge_methods_supported":["S256"],
            "grant_types_supported":["authorization_code"],"token_endpoint_auth_methods_supported":["none"]
        }))))
        .route("/register", axum::routing::post(async |axum::Json(req):axum::Json<Value>| {
            assert_eq!(req["client_name"], "Dominion agent runner");
            axum::Json(json!({"client_id":"client","redirect_uris":req["redirect_uris"],"token_endpoint_auth_method":"none"}))
        }))
        .route("/authorize", axum::routing::get(async |Query(req):Query<HashMap<String,String>>| {
            assert_eq!(req["code_challenge_method"], "S256");
            assert_eq!(req["code_challenge"].len(), 43);
            assert_eq!(req["scope"], "orchestration:read orchestration:operate");
            Redirect::temporary(&format!("{}?code=code&state={}", req["redirect_uri"], req["state"]))
        }))
        .route("/token", axum::routing::post(async |State(base):State<String>, Form(req):Form<HashMap<String,String>>| {
            assert_eq!(req["code"], "code");
            assert!(req["code_verifier"].len() >= 43);
            assert_eq!(req["resource"], format!("{base}/mcp"));
            axum::Json(json!({"access_token":"authorized-test-token","token_type":"Bearer","expires_in":3600}))
        })).with_state(base.clone());
    let task = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    let token = t3::login(&format!("{base}/mcp"), |url| {
        let url = url.to_string();
        tokio::spawn(async move {
            reqwest::Client::new()
                .get(url)
                .send()
                .await
                .unwrap()
                .error_for_status()
                .unwrap();
        });
    })
    .await
    .unwrap();
    assert_eq!(token, "authorized-test-token");
    task.abort();
}

#[tokio::test]
async fn briefed_agents_debrief_before_they_leave_and_the_report_collects_it() {
    let fake = FakeT3::default();
    let calls = fake.calls.clone();
    let mut h = setup(fake).await;
    let socket = h._dir.path().join("agents.sock");
    h.config.agent_socket = Some(socket.clone());
    let (arena, server_stop) = (h.arena.clone(), h.stop.clone());
    let listener =
        tokio::spawn(
            async move { dominion_server::house::listen(arena, &socket, server_stop).await },
        );
    let (mut changes, _) = h.arena.subscribe();
    let runner_stop = CancellationToken::new();
    let runner = tokio::spawn(dominion_server::runner::run_until(
        h.config.runner().unwrap(),
        runner_stop.clone(),
    ));
    tokio::time::timeout(std::time::Duration::from_secs(10), changes.changed())
        .await
        .unwrap()
        .unwrap();
    let client = Client::connect_authenticated(&h.config.server(), Some(h.token.clone()))
        .await
        .unwrap();
    let mut game = request();
    game.briefs = vec![Some("Play Villages.".into()), None];
    game.debrief = Some("What decided the game?".into());
    let created = client
        .call("create_game", serde_json::to_value(&game).unwrap())
        .await
        .unwrap();
    assert!(created.ok, "{}", created.text);
    let id = created.json.unwrap()["game_id"]
        .as_str()
        .unwrap()
        .to_string();
    let report = client
        .call("game_report", json!({"game": id, "wait_s": 30}))
        .await
        .unwrap();
    assert!(report.ok, "{}", report.text);
    let value = report.json.unwrap();
    assert_eq!(value["settled"], true, "{}", report.text);
    assert_eq!(value["seats"][0]["brief"], "Play Villages.");
    assert_eq!(value["seats"][1]["brief"], Value::Null);
    assert_eq!(value["debrief_question"], "What decided the game?");
    for seat in 0..2 {
        let debrief = value["seats"][seat]["debrief"].as_str().unwrap();
        assert!(debrief.contains("the brief's plan held up"), "{debrief}");
    }
    assert!(value["seats"][0]["deck"]["Copper"].as_u64().unwrap() >= 7);
    assert!(!value["turns"].as_array().unwrap().is_empty());
    assert!(report.text.contains("DEBRIEF seat 1 (Test (2))"));
    // Each agent read its own brief and nobody else's.
    let launches: Vec<String> = (calls.lock().unwrap().iter())
        .filter(|(tool, _)| tool == "t3_thread_launch")
        .map(|(_, args)| args["message"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(launches.len(), 2);
    let briefed: Vec<&String> = (launches.iter())
        .filter(|m| m.contains("Play Villages."))
        .collect();
    assert_eq!(briefed.len(), 1);
    assert!(briefed[0].contains("You are Test (1) (seat 0)"));
    assert!(
        launches
            .iter()
            .all(|m| m.contains("What decided the game?"))
    );
    // Their jobs ended with their debriefs.
    h.arena.sweep();
    assert!(dispatched(&mut h.jobs).is_empty());
    client.close().await;
    runner_stop.cancel();
    tokio::time::timeout(std::time::Duration::from_secs(10), runner)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    listener.abort();
}

#[tokio::test]
async fn a_token_seat_reads_its_brief_on_joining_and_debriefs_once_the_game_is_over() {
    let h = setup(FakeT3::default()).await;
    let client = Client::connect_authenticated(&h.config.server(), Some(h.token.clone()))
        .await
        .unwrap();
    let game = NewGame {
        agents: vec![],
        briefs: vec![Some("Open Silver/Silver.".into()), None],
        debrief: Some("What decided it?".into()),
        watchable: Some(false),
        ..request()
    };
    let created = client
        .call("create_game", serde_json::to_value(&game).unwrap())
        .await
        .unwrap();
    assert!(created.ok, "{}", created.text);
    let created = created.json.unwrap();
    let id = created["game_id"].as_str().unwrap().to_string();
    let token = |s: usize| created["seats"][s]["token"].as_str().unwrap().to_string();

    let player = Client::connect(&h.config.server()).await.unwrap();
    let joined = player
        .call("join_game", json!({"game": id, "token": token(0)}))
        .await
        .unwrap();
    assert!(joined.text.starts_with("BRIEF"), "{}", joined.text);
    assert_eq!(joined.json.unwrap()["brief"], "Open Silver/Silver.");
    let other = player
        .call("join_game", json!({"game": id, "token": token(1)}))
        .await
        .unwrap();
    assert!(!other.text.contains("BRIEF"));
    let early = player
        .call("debrief", json!({"token": token(0), "text": "x"}))
        .await
        .unwrap();
    assert!(!early.ok);
    // The game isn't watchable: only its creator and players may read its report.
    let stranger = Client::connect(&h.config.server()).await.unwrap();
    let refused = stranger
        .call("game_report", json!({"game": id}))
        .await
        .unwrap();
    assert!(!refused.ok);
    let running = client
        .call("game_report", json!({"game": id}))
        .await
        .unwrap();
    assert!(running.ok, "{}", running.text);
    assert!(!running.text.contains("Open Silver/Silver."));

    let resigned = player
        .call("resign", json!({"token": token(1)}))
        .await
        .unwrap();
    assert!(resigned.ok, "{}", resigned.text);
    let given = player
        .call(
            "debrief",
            json!({"token": token(0), "text": "They resigned."}),
        )
        .await
        .unwrap();
    assert!(given.ok, "{}", given.text);
    let report = client
        .call("game_report", json!({"game": id}))
        .await
        .unwrap();
    let value = report.json.unwrap();
    assert_eq!(value["settled"], true);
    assert_eq!(value["seats"][0]["debrief"], "They resigned.");
    assert_eq!(value["seats"][0]["brief"], "Open Silver/Silver.");
    for c in [client, player, stranger] {
        c.close().await;
    }
}
