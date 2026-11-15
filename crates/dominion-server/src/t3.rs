//! T3 Code runs the conversation; this bridge gives it only its seat's observations
//! and applies its requested Dominion tool calls.

use crate::client::{Client, Reply};
use crate::house::Job;
use crate::hub::now_ms;
use crate::runner::{AgentSpec, Program};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio_util::sync::CancellationToken;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum OptionValue {
    String(String),
    Bool(bool),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    /// T3 Code's streamable HTTP MCP endpoint.
    pub url: String,
    /// A dedicated project for game conversations, with the desired agent environment.
    pub project_id: String,
    /// Environment variable holding an MCP OAuth access token.
    pub token_env: Option<String>,
    /// Alternatively, a file holding the MCP OAuth access token.
    pub token_file: Option<PathBuf>,
    /// A linked T3 environment, if the project and providers live there.
    pub environment_id: Option<String>,
    /// Maximum time for one agent response (default 600 seconds).
    pub response_seconds: Option<u32>,
}

impl Settings {
    pub async fn connect(&self) -> Result<Client, String> {
        if self.token_env.is_some() && self.token_file.is_some() {
            return Err("set t3code.token_env or t3code.token_file, not both".into());
        }
        let token = if let Some(path) = &self.token_file {
            std::fs::read_to_string(path)
                .map_err(|e| format!("reading T3 MCP token file {}: {e}", path.display()))?
        } else {
            let name = self.token_env.as_deref().unwrap_or("T3CODE_MCP_TOKEN");
            std::env::var(name).map_err(|_| format!("set {name} to a T3 MCP OAuth access token"))?
        };
        if token.trim().is_empty() {
            return Err("the T3 MCP access token is empty".into());
        }
        Client::connect_authenticated(&self.url, Some(token.trim().to_string())).await
    }

    fn scoped(&self, mut args: Value) -> Value {
        if let Some(id) = &self.environment_id {
            args["environmentId"] = json!(id);
        }
        args
    }
}

/// Append-only interaction journal. A failed write stops play before the next action.
pub struct Archive {
    pub dir: PathBuf,
    file: File,
}

impl Archive {
    pub fn create(root: &Path) -> Result<Self, String> {
        private_dir(root)?;
        let dir = root.join(format!("{}-{:016x}", now_ms(), rand::random::<u64>()));
        Self::open(&dir)
    }

    pub fn open(dir: &Path) -> Result<Self, String> {
        private_dir(dir)?;
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let file = options
            .open(dir.join("interactions.jsonl"))
            .map_err(|e| format!("opening archive {}: {e}", dir.display()))?;
        Ok(Self {
            dir: dir.into(),
            file,
        })
    }

    pub fn record(&mut self, kind: &str, mut data: Value) -> Result<(), String> {
        redact(&mut data);
        let line = json!({"time": now_ms(), "kind": kind, "data": data});
        serde_json::to_writer(&mut self.file, &line).map_err(|e| e.to_string())?;
        self.file.write_all(b"\n").map_err(|e| e.to_string())?;
        self.file
            .sync_data()
            .map_err(|e| format!("saving interaction archive: {e}"))
    }
}

fn private_dir(dir: &Path) -> Result<(), String> {
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder
        .create(dir)
        .map_err(|e| format!("{}: {e}", dir.display()))
}

fn redact(value: &mut Value) {
    match value {
        Value::Object(map) => {
            for (key, value) in map {
                if matches!(key.as_str(), "token" | "authorization" | "access_token") {
                    *value = json!("[redacted]");
                } else {
                    redact(value);
                }
            }
        }
        Value::Array(values) => values.iter_mut().for_each(redact),
        _ => {}
    }
}

pub async fn call(
    client: &Client,
    archive: &mut Archive,
    source: &str,
    tool: &'static str,
    args: Value,
) -> Result<Reply, String> {
    archive.record(
        "request",
        json!({"source": source, "tool": tool, "arguments": args}),
    )?;
    let result = tokio::time::timeout(Duration::from_secs(90), client.call(tool, args)).await;
    match result {
        Ok(Ok(reply)) => {
            archive.record(
                "response",
                json!({"source": source, "tool": tool, "reply": reply}),
            )?;
            Ok(reply)
        }
        result => {
            let error = match result {
                Ok(Err(e)) => e.to_string(),
                Err(_) => format!("{source} {tool}: no reply within 90 seconds"),
                _ => unreachable!(),
            };
            archive.record(
                "transport_error",
                json!({"source": source, "tool": tool, "error": error}),
            )?;
            Err(error)
        }
    }
}

fn structured(reply: Reply) -> Result<Value, String> {
    if !reply.ok {
        return Err(reply.text);
    }
    reply
        .json
        .or_else(|| serde_json::from_str(&reply.text).ok())
        .ok_or_else(|| format!("MCP returned no structured result: {}", reply.text))
}

/// Check the actual provider catalog before creating a game or starting an agent.
pub async fn validate(
    settings: &Settings,
    client: &Client,
    spec: &AgentSpec,
    archive: &mut Archive,
) -> Result<Value, String> {
    if spec.program != Program::T3Code {
        return Err(format!(
            "{}: simulations need program = \"t3code\"",
            spec.name
        ));
    }
    if spec.command.is_some() {
        return Err(format!(
            "{}: T3 Code owns the executable; omit command",
            spec.name
        ));
    }
    let provider = spec
        .provider_instance
        .as_deref()
        .ok_or("a T3 agent needs provider_instance")?;
    let model = spec.model.as_deref().ok_or("a T3 agent needs model")?;
    let caps = structured(
        call(
            client,
            archive,
            "t3",
            "orchestrator_capabilities",
            settings.scoped(json!({"search": model})),
        )
        .await?,
    )?;
    if caps["runtimeMode"] != "full-access" {
        return Err("the T3 MCP credential must have Full access; run dominion t3-login".into());
    }
    validate_catalog(&caps, provider, model, spec)?;
    let driver = caps["providers"]
        .as_array()
        .and_then(|ps| ps.iter().find(|p| p["providerInstanceId"] == provider))
        .and_then(|p| p["driverKind"].as_str())
        .ok_or("T3 provider returned no driverKind")?;
    let project = structured(
        call(
            client,
            archive,
            "t3",
            "t3_project_read",
            settings.scoped(json!({"projectId": settings.project_id})),
        )
        .await?,
    )?;
    if project["id"].as_str() != Some(&settings.project_id) {
        return Err("T3 did not return the configured project".into());
    }
    if settings.response_seconds == Some(0) {
        return Err("t3code.response_seconds must be positive".into());
    }
    let options: Vec<_> = spec
        .options
        .iter()
        .map(|(id, value)| json!({"id": id, "value": value}))
        .collect();
    Ok(json!({"provider": driver, "instanceId": provider, "model": model, "options": options}))
}

fn validate_catalog(
    caps: &Value,
    provider: &str,
    model: &str,
    spec: &AgentSpec,
) -> Result<(), String> {
    let providers = caps["providers"]
        .as_array()
        .ok_or("T3 returned no provider catalog")?;
    let entry = providers
        .iter()
        .find(|p| p["providerInstanceId"] == provider)
        .ok_or_else(|| format!("T3 provider instance {provider:?} is unavailable"))?;
    if entry["canRunChildTask"] == false {
        return Err(format!(
            "T3 provider {provider:?} is unavailable: {}",
            entry["constraints"]
        ));
    }
    if entry["runtimeModes"]
        .as_array()
        .is_some_and(|modes| !modes.iter().any(|m| m == "full-access"))
    {
        return Err(format!(
            "T3 provider {provider:?} does not support full-access"
        ));
    }
    let models = entry["models"].as_array().ok_or("T3 returned no models")?;
    let model = models
        .iter()
        .find(|m| m["id"] == model)
        .ok_or_else(|| format!("T3 model {model:?} is unavailable on {provider:?}"))?;
    for (id, value) in &spec.options {
        let value = serde_json::to_value(value).expect("serializable");
        let valid = model["options"]
            .as_array()
            .and_then(|opts| opts.iter().find(|o| o["id"] == *id))
            .and_then(|o| o["values"].as_array())
            .is_some_and(|values| values.contains(&value));
        if !valid {
            return Err(format!(
                "{}: unsupported T3 model option {id}={value}",
                spec.name
            ));
        }
    }
    Ok(())
}

const INSTRUCTIONS: &str = "You play one seat of Dominion. Play to win. The bridge supplies your legal view and executes one requested game tool per reply. Use only the observations supplied here for game information. Do not inspect files, run commands, delegate, or use other tools. Never resign or ask a person for help.
Reply with exactly one JSON object (no prose), using one of these forms:
{\"tool\":\"answer\",\"decision_id\":\"d1\",\"indices\":[0]}
{\"tool\":\"answer\",\"decision_id\":\"d1\",\"name\":\"Copper\"}
{\"tool\":\"play_treasures\",\"decision_id\":\"d1\"}
{\"tool\":\"card\",\"name\":\"Village\"}
An answer may also include \"way\":\"none\" or a Way's name. Use the actual decision id and option indices in your view; ordered choices are top first. A card lookup returns official card text, then you may answer. The conversation continues throughout the game. When a tool returns an error, correct your request.";

#[derive(Debug, Deserialize)]
#[serde(tag = "tool", rename_all = "snake_case", deny_unknown_fields)]
enum Action {
    Answer {
        decision_id: String,
        indices: Option<Vec<u32>>,
        name: Option<String>,
        way: Option<String>,
    },
    PlayTreasures {
        decision_id: String,
    },
    Card {
        name: String,
    },
}

fn action(text: &str) -> Result<Action, String> {
    let text = text.trim();
    let text = text
        .strip_prefix("```json")
        .or_else(|| text.strip_prefix("```"))
        .and_then(|t| t.trim().strip_suffix("```"))
        .unwrap_or(text)
        .trim();
    serde_json::from_str(text).map_err(|e| format!("reply with one game-tool JSON object: {e}"))
}

struct Conversation<'a> {
    settings: &'a Settings,
    client: Client,
    thread: Option<String>,
    position: Option<u64>,
    selection: Value,
    run: Option<String>,
}

impl Conversation<'_> {
    async fn call(
        &self,
        archive: &mut Archive,
        tool: &'static str,
        args: Value,
    ) -> Result<Value, String> {
        structured(
            call(
                &self.client,
                archive,
                "t3",
                tool,
                self.settings.scoped(args),
            )
            .await?,
        )
    }

    async fn reply(
        &mut self,
        spec: &AgentSpec,
        job: &Job,
        message: String,
        archive: &mut Archive,
    ) -> Result<String, String> {
        let started = if let Some(thread) = &self.thread {
            self.call(
                archive,
                "t3_thread_send",
                json!({
                    "threadId": thread, "message": message, "mode": "queue",
                    "clientRequestId": format!("dominion-{:016x}", rand::random::<u64>())
                }),
            )
            .await?
        } else {
            let started = self.call(archive, "t3_thread_launch", json!({
                "projectId": self.settings.project_id,
                "title": format!("Dominion {} — {} (seat {})", job.game_id, spec.name, job.seat),
                "modelSelection": self.selection,
                "runtimeMode": "full-access", "interactionMode": "default",
                "workspaceStrategy": {"type": "root"},
                "message": format!("{INSTRUCTIONS}\n\n{}\n\nYou are {} (seat {}) in game {}.\n\n{message}",
                    spec.instructions.as_deref().unwrap_or(""), job.name, job.seat, job.game_id),
                "clientRequestId": format!("dominion-{:016x}", rand::random::<u64>())
            })).await?;
            let id = started["threadId"]
                .as_str()
                .ok_or("T3 launch returned no threadId")?
                .to_string();
            self.thread = Some(id);
            archive.record("thread", started.clone())?;
            if let Some(link) = started["link"].as_str() {
                println!("{}: {link}", spec.name);
            }
            started
        };
        let run = started["runId"].as_str().ok_or("T3 returned no runId")?;
        self.run = Some(run.to_string());
        let thread = self.thread.as_deref().expect("launched");
        let wait = async {
            loop {
                let waited = self
                    .call(
                        archive,
                        "t3_thread_wait",
                        json!({"threadId": thread, "runId": run, "timeoutMs": 60_000}),
                    )
                    .await?;
                if waited["timedOut"] == true {
                    continue;
                }
                return match waited["status"].as_str() {
                    Some("completed") => Ok(()),
                    status => Err(format!("T3 agent run stopped: {status:?}")),
                };
            }
        };
        let outcome = tokio::time::timeout(
            Duration::from_secs(u64::from(self.settings.response_seconds.unwrap_or(600))),
            wait,
        )
        .await;
        // Read history even on a failed run, including reasoning and failed tool calls.
        let text = self.history(run, archive).await?;
        outcome.map_err(|_| "T3 agent response timed out".to_string())??;
        text.ok_or_else(|| "T3 run completed without an assistant response".into())
    }

    async fn history(
        &mut self,
        run: &str,
        archive: &mut Archive,
    ) -> Result<Option<String>, String> {
        let mut answer = None;
        loop {
            let mut args = json!({"threadId": self.thread, "view": "activity", "limit": 100, "maxCharsPerItem": 50_000});
            if let Some(position) = self.position {
                args["afterPosition"] = json!(position);
            }
            let page = self.call(archive, "t3_thread_read", args).await?;
            let items = page["items"]
                .as_array()
                .ok_or("T3 history returned no items")?;
            for item in items {
                let mut part = item.clone();
                let mut text = part["text"].as_str().unwrap_or("").to_string();
                while part["textTruncated"] == true {
                    let offset = part["nextTextOffset"]
                        .as_u64()
                        .ok_or("T3 truncated history without a continuation")?;
                    let page = self.call(archive, "t3_thread_read", json!({
                        "threadId": self.thread, "view": "activity", "itemId": item["itemId"],
                        "textOffset": offset, "maxCharsPerItem": 50_000, "limit": 1
                    })).await?;
                    let next = page["items"][0].clone();
                    if next["itemId"] != item["itemId"]
                        || next["nextTextOffset"].as_u64().is_some_and(|n| n <= offset)
                    {
                        return Err("T3 history continuation did not advance".into());
                    }
                    text.push_str(next["text"].as_str().unwrap_or(""));
                    part = next;
                }
                let mut complete = item.clone();
                if !complete["text"].is_null() {
                    complete["text"] = json!(text);
                }
                complete["textTruncated"] = json!(false);
                complete["nextTextOffset"] = Value::Null;
                archive.record("activity", complete)?;
                if item["runId"] == run && item["type"] == "assistant_message" {
                    answer = Some(text);
                }
            }
            if page["hasMore"] != true {
                self.position = page["nextPosition"].as_u64().or(self.position);
                break;
            }
            let next = page["nextPosition"]
                .as_u64()
                .ok_or("T3 history returned no cursor")?;
            if self.position.is_some_and(|p| next <= p) {
                return Err("T3 history cursor did not advance".into());
            }
            self.position = Some(next);
        }
        Ok(answer)
    }

    async fn stop(&mut self, archive: &mut Archive) -> Result<(), String> {
        if let Some(thread) = &self.thread {
            let stopped = self
                .call(
                    archive,
                    "t3_thread_interrupt",
                    json!({"threadId": thread, "reason": "Dominion seat runner stopped"}),
                )
                .await?;
            if stopped["status"] == "interrupt_requested" {
                let waited = self
                    .call(
                        archive,
                        "t3_thread_wait",
                        json!({"threadId": thread, "runId": self.run, "timeoutMs": 60_000}),
                    )
                    .await?;
                if waited["timedOut"] == true {
                    return Err("T3 did not stop the agent run".into());
                }
            }
            self.position = None;
            let run = self.run.clone().unwrap_or_default();
            self.history(&run, archive).await?;
        }
        Ok(())
    }
}

pub async fn play(
    settings: &Settings,
    spec: &AgentSpec,
    job: &Job,
    server: &str,
    archive: &mut Archive,
    stop: CancellationToken,
) -> Result<(), String> {
    archive.record(
        "seat",
        json!({"agent": spec, "job": job, "server": server, "t3code": settings}),
    )?;
    let client = settings.connect().await?;
    let selection = validate(settings, &client, spec, archive).await?;
    let dominion = Client::connect(server).await?;
    let mut chat = Conversation {
        settings,
        client,
        thread: None,
        position: None,
        selection,
        run: None,
    };
    let result = tokio::select! {
        biased;
        _ = stop.cancelled() => Err("seat runner cancelled".into()),
        result = play_inner(&mut chat, &dominion, spec, job, archive) => result,
    };
    let cleanup = chat.stop(archive).await;
    let final_view = call(
        &dominion,
        archive,
        "dominion",
        "view",
        json!({"token":job.token}),
    )
    .await;
    if let Ok(view) = &final_view
        && let Some(state) = &view.json
        && state["game"]["running"] == false
    {
        archive.record("result", state.clone())?;
    }
    archive.record(
        "seat_finished",
        json!({"result": result, "cleanup": cleanup, "final_view_error": final_view.as_ref().err()}),
    )?;
    result.and(cleanup)
}

async fn play_inner(
    chat: &mut Conversation<'_>,
    dominion: &Client,
    spec: &AgentSpec,
    job: &Job,
    archive: &mut Archive,
) -> Result<(), String> {
    let mut view = call(
        dominion,
        archive,
        "dominion",
        "join_game",
        json!({"game_id": job.game_id, "token": job.token}),
    )
    .await?;
    if !view.ok {
        return Err(view.text);
    }
    let mut errors = 0;
    loop {
        let state = view.json.as_ref().ok_or("Dominion returned no game view")?;
        if state["game"]["running"] == false {
            archive.record("result", state.clone())?;
            return Ok(());
        }
        if state["decision"].is_null() {
            view = call(
                dominion,
                archive,
                "dominion",
                "wait_for_decision",
                json!({"token": job.token, "timeout_ms": 60_000}),
            )
            .await?;
            if !view.ok {
                return Err(view.text);
            }
            continue;
        }
        let mut message = format!(
            "Current view:\n{}",
            serde_json::to_string(state).expect("serializable")
        );
        for interaction in 0..32 {
            if interaction == 31 {
                return Err("agent exceeded 32 requests for one decision".into());
            }
            let response = chat.reply(spec, job, message, archive).await?;
            let (tool, mut args) = match action(&response) {
                Ok(Action::Answer {
                    decision_id,
                    indices,
                    name,
                    way,
                }) => (
                    "answer",
                    json!({"decision_id":decision_id,"indices":indices,"name":name,"way":way}),
                ),
                Ok(Action::PlayTreasures { decision_id }) => {
                    ("play_treasures", json!({"decision_id":decision_id}))
                }
                Ok(Action::Card { name }) => ("card", json!({"name":name})),
                Err(e) => {
                    errors += 1;
                    if errors >= 3 {
                        return Err(e);
                    }
                    message = e;
                    continue;
                }
            };
            if tool != "card" {
                args["token"] = json!(job.token);
            }
            let reply = call(dominion, archive, "dominion", tool, args).await?;
            if tool == "card" || !reply.ok {
                if !reply.ok {
                    errors += 1;
                    if errors >= 3 {
                        return Err(reply.text);
                    }
                }
                message = serde_json::to_string(&reply).expect("serializable");
                continue;
            }
            errors = 0;
            view = reply;
            break;
        }
    }
}

/// OAuth authorization with an ephemeral loopback callback. Credentials stay in
/// memory; the caller can send the returned token to its credential manager.
pub async fn login(url: &str, show_url: impl FnOnce(&str)) -> Result<String, String> {
    use axum::extract::{Query, State};
    use rmcp::transport::auth::{AuthorizationManager, AuthorizationRequest, AuthorizationSession};
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};
    use tokio::sync::oneshot;
    type CallbackSender = Arc<Mutex<Option<oneshot::Sender<HashMap<String, String>>>>>;

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|e| e.to_string())?;
    let redirect = format!(
        "http://{}/callback",
        listener.local_addr().map_err(|e| e.to_string())?
    );
    let mut manager = AuthorizationManager::new_allowing_private_addresses(url)
        .await
        .map_err(|e| e.to_string())?;
    let metadata = manager
        .resolve_metadata()
        .await
        .map_err(|e| e.to_string())?;
    manager.set_metadata(metadata.metadata);
    let request = AuthorizationRequest::new(&redirect)
        .with_client_name("Dominion agent runner")
        .with_scopes(["orchestration:read", "orchestration:operate"]);
    let session = AuthorizationSession::new(manager, request)
        .await
        .map_err(|(_, e)| e.to_string())?;
    let (send, received) = oneshot::channel::<HashMap<String, String>>();
    let sender = Arc::new(Mutex::new(Some(send)));
    let router = axum::Router::new()
        .route(
            "/callback",
            axum::routing::get(
                async |State(sender): State<CallbackSender>,
                       Query(query): Query<HashMap<String, String>>| {
                    if let Some(send) = sender.lock().expect("callback lock").take() {
                        let _ = send.send(query);
                    }
                    "Authorization received. Return to the terminal."
                },
            ),
        )
        .with_state(sender);
    let stop = CancellationToken::new();
    let stopped = stop.clone();
    let task = tokio::spawn(async move {
        axum::serve(listener, router)
            .with_graceful_shutdown(stopped.cancelled_owned())
            .await
    });
    show_url(session.get_authorization_url());
    let callback = tokio::time::timeout(Duration::from_secs(300), received).await;
    stop.cancel();
    let _ = task.await;
    let query = callback
        .map_err(|_| "T3 sign-in timed out")?
        .map_err(|_| "T3 callback closed")?;
    if let Some(error) = query.get("error") {
        return Err(format!("T3 sign-in refused: {error}"));
    }
    let code = query.get("code").ok_or("T3 callback had no code")?;
    let state = query.get("state").ok_or("T3 callback had no state")?;
    session
        .handle_callback_with_issuer(code, state, query.get("iss").map(String::as_str))
        .await
        .map_err(|e| e.to_string())?;
    session
        .auth_manager
        .get_access_token()
        .await
        .map_err(|e| e.to_string())
}
