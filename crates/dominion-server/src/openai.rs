//! The direct OpenAI agent: plays a seat by calling the Responses API itself, with no CLI
//! between the model and the game. Its only tools are the game's and, with a playbook,
//! reading it and its ratings lookup, so it needs no sandbox: it runs nothing the model
//! chooses. It signs in with Codex's ChatGPT login (`auth.json`) or an API key.

use crate::archive::{Archive, call};
use crate::client::Client;
use crate::harness::Sandbox;
use crate::house::Job;
use crate::runner::AgentSpec;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio_util::sync::CancellationToken;

const CHATGPT_URL: &str = "https://chatgpt.com/backend-api/codex/responses";
const API_URL: &str = "https://api.openai.com/v1/responses";
const TOKEN_URL: &str = "https://auth.openai.com/oauth/token";
/// Codex's OAuth client: refreshing its login keeps `auth.json` usable by Codex too.
const CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
/// Game results kept verbatim; older ones are replaced by a note, a batch at a time so
/// the cached prefix of the conversation changes rarely.
const DEFAULT_KEEP_RESULTS: usize = 60;
const PRUNE_BATCH: usize = 20;
/// Times the model may stop calling tools while the game still runs.
const MAX_NUDGES: u32 = 3;

const BRIEF: &str = "You play one assigned seat of Dominion. Play to win with the game tools. step answers your decision: its decision id, indices (ordered: top first) or name only when nameAny is true, and expect with each chosen option's label or card, in index order. When there is no decision for you (WAITING), call wait. Results are text: what changed since your last decision, the text of cards new to you, then your next decision. state shows the whole game; cards looks up card text and current costs; log rereads log entries; play_treasures plays every Treasure offered in one call. Keep playing until GAME OVER, then write a short review of your plan, the turns that decided the game and what you would do differently. Never resign or ask a person for help.";
const PLAYBOOK: &str = "The Dominion playbook is open to you, read-only: its AGENTS.md follows. read_file reads its other files and card_ratings is its `python3 data/card-ratings.py` lookup. Before your first decision, do what skills/play-game/SKILL.md says under \"Make the plan\": look up the ratings of every card on the board, read the context it names, run its kingdom analysis, and write the plan out in your reply; then play by its skills.";

enum Auth {
    ChatGpt { file: PathBuf },
    ApiKey(String),
}

struct Session {
    auth: Auth,
    agent: ureq::Agent,
    id: String,
}

impl Session {
    fn new(spec: &AgentSpec) -> Result<Self, String> {
        let auth = match &spec.api_key_env {
            Some(var) => Auth::ApiKey(
                std::env::var(var).map_err(|_| format!("{var} is unset: it names the API key"))?,
            ),
            None => {
                let dir = spec.auth_dir.clone().map(Ok).unwrap_or_else(|| {
                    std::env::var_os("CODEX_HOME")
                        .map(PathBuf::from)
                        .or_else(|| {
                            std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".codex"))
                        })
                        .ok_or_else(|| "HOME is unset".to_string())
                })?;
                Auth::ChatGpt {
                    file: dir.join("auth.json"),
                }
            }
        };
        let seconds = u64::from(spec.response_seconds.unwrap_or(600));
        let agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(seconds)))
            .http_status_as_error(false)
            .build()
            .into();
        Ok(Self {
            auth,
            agent,
            id: format!("{:032x}", rand::random::<u128>()),
        })
    }

    /// One model turn: its output items, and the usage it reports.
    async fn respond(&self, body: &Value) -> Result<(Vec<Value>, Value), String> {
        let mut refreshed = false;
        let mut delay = Duration::from_secs(2);
        let mut attempt = 0;
        loop {
            attempt += 1;
            let (url, headers) = match &self.auth {
                Auth::ApiKey(key) => (API_URL, vec![("Authorization", format!("Bearer {key}"))]),
                Auth::ChatGpt { file } => {
                    let tokens = read_tokens(file)?;
                    (
                        CHATGPT_URL,
                        vec![
                            ("Authorization", format!("Bearer {}", tokens.0)),
                            ("chatgpt-account-id", tokens.1),
                            ("OpenAI-Beta", "responses=experimental".into()),
                            ("originator", "dominion_arena".into()),
                            ("session_id", self.id.clone()),
                        ],
                    )
                }
            };
            let agent = self.agent.clone();
            let payload = serde_json::to_vec(body).map_err(|e| e.to_string())?;
            let reply = tokio::task::spawn_blocking(move || {
                let mut request = agent
                    .post(url)
                    .header("Content-Type", "application/json")
                    .header("Accept", "text/event-stream");
                for (name, value) in &headers {
                    request = request.header(*name, value);
                }
                let mut reply = request
                    .send(payload.as_slice())
                    .map_err(|e| e.to_string())?;
                let status = reply.status().as_u16();
                let text = reply
                    .body_mut()
                    .read_to_string()
                    .map_err(|e| e.to_string())?;
                Ok::<_, String>((status, text))
            })
            .await
            .map_err(|e| e.to_string())?;
            let (status, text) = match reply {
                Ok(reply) => reply,
                Err(e) if attempt < 5 => {
                    eprintln!("dominion agents: OpenAI request failed ({e}); retrying");
                    tokio::time::sleep(delay).await;
                    delay *= 2;
                    continue;
                }
                Err(e) => return Err(format!("OpenAI: {e}")),
            };
            match status {
                200 => return parse_stream(&text),
                401 if !refreshed => {
                    if let Auth::ChatGpt { file } = &self.auth {
                        refresh(file).await?;
                        refreshed = true;
                        continue;
                    }
                    return Err(format!("OpenAI: HTTP 401: {}", snippet(&text)));
                }
                429 | 500..=599 if attempt < 6 => {
                    tokio::time::sleep(delay).await;
                    delay *= 2;
                }
                _ => return Err(format!("OpenAI: HTTP {status}: {}", snippet(&text))),
            }
        }
    }
}

fn snippet(text: &str) -> String {
    text.chars().take(500).collect()
}

/// The access token and account id in Codex's `auth.json`.
fn read_tokens(file: &Path) -> Result<(String, String), String> {
    let text = std::fs::read_to_string(file).map_err(|e| format!("{}: {e}", file.display()))?;
    let auth: Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    let tokens = &auth["tokens"];
    match (
        tokens["access_token"].as_str(),
        tokens["account_id"].as_str(),
    ) {
        (Some(access), Some(account)) => Ok((access.into(), account.into())),
        _ => Err(format!(
            "{}: no ChatGPT login; sign in with `codex login`",
            file.display()
        )),
    }
}

static REFRESH: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Exchanges the refresh token for new tokens and writes them back for Codex to use too.
/// One refresh at a time: the refresh token is used up by it.
async fn refresh(file: &Path) -> Result<(), String> {
    let _held = REFRESH.lock().await;
    let file = file.to_path_buf();
    tokio::task::spawn_blocking(move || {
        let text = std::fs::read_to_string(&file).map_err(|e| e.to_string())?;
        let mut auth: Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
        let refresh = auth["tokens"]["refresh_token"]
            .as_str()
            .ok_or("auth.json has no refresh token")?
            .to_string();
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(30)))
            .http_status_as_error(false)
            .build()
            .into();
        let body = json!({"client_id": CLIENT_ID, "grant_type": "refresh_token",
            "refresh_token": refresh, "scope": "openid profile email"});
        let mut reply = agent
            .post(TOKEN_URL)
            .header("Content-Type", "application/json")
            .send(
                serde_json::to_vec(&body)
                    .map_err(|e| e.to_string())?
                    .as_slice(),
            )
            .map_err(|e| e.to_string())?;
        let status = reply.status().as_u16();
        let text = reply
            .body_mut()
            .read_to_string()
            .map_err(|e| e.to_string())?;
        if status != 200 {
            return Err(format!("refreshing the ChatGPT login: HTTP {status}"));
        }
        let fresh: Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
        for key in ["access_token", "id_token", "refresh_token"] {
            if let Some(value) = fresh[key].as_str() {
                auth["tokens"][key] = json!(value);
            }
        }
        auth["last_refresh"] = json!(rfc3339(std::time::SystemTime::now()));
        let tmp = file.with_extension("json.dominion");
        crate::harness::write_private(
            &tmp,
            &serde_json::to_vec_pretty(&auth).map_err(|e| e.to_string())?,
        )?;
        std::fs::rename(&tmp, &file).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// `2026-11-16T09:44:00Z`.
fn rfc3339(time: std::time::SystemTime) -> String {
    let secs = time
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let (days, rest) = (secs / 86_400, secs % 86_400);
    // Days since 1970-01-01 to a civil date (Howard Hinnant's algorithm).
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rest / 3600,
        rest / 60 % 60,
        rest % 60
    )
}

/// The output items and usage of a streamed response.
fn parse_stream(text: &str) -> Result<(Vec<Value>, Value), String> {
    let mut items = vec![];
    let mut usage = Value::Null;
    for line in text.lines() {
        let Some(data) = line.strip_prefix("data:") else {
            continue;
        };
        let Ok(event) = serde_json::from_str::<Value>(data.trim()) else {
            continue;
        };
        match event["type"].as_str() {
            Some("response.output_item.done") => items.push(event["item"].clone()),
            Some("response.completed") => usage = event["response"]["usage"].clone(),
            Some("response.failed" | "error") => {
                return Err(format!("OpenAI: {}", snippet(&event.to_string())));
            }
            _ => {}
        }
    }
    if usage.is_null() {
        return Err(format!(
            "OpenAI: the response didn't complete: {}",
            snippet(text)
        ));
    }
    Ok((items, usage))
}

/// The function tools the model gets: the game's, and the playbook's when there is one.
fn tools(playbook: bool) -> Vec<Value> {
    let mut tools: Vec<Value> = crate::harness::GAME_TOOLS
        .iter()
        .map(|name| {
            let definition = crate::mcp::tool_definition(name).expect("game tool");
            let mut parameters = Value::Object((*definition.input_schema).clone());
            if let Some(properties) = parameters["properties"].as_object_mut() {
                properties.remove("token");
            }
            if let Some(required) = parameters["required"].as_array_mut() {
                required.retain(|v| v != "token");
            }
            json!({"type": "function", "name": name, "parameters": parameters,
                "description": description(name)})
        })
        .collect();
    if playbook {
        tools.push(json!({"type": "function", "name": "read_file",
            "description": "Read a file of the Dominion playbook (a path relative to it, such as skills/play-game/SKILL.md), optionally from line `offset` (1-based) for `limit` lines.",
            "parameters": {"type": "object", "properties": {
                "path": {"type": "string"},
                "offset": {"type": "integer"},
                "limit": {"type": "integer"}},
                "required": ["path"], "additionalProperties": false}}));
        tools.push(json!({"type": "function", "name": "list_files",
            "description": "List the files under a directory of the playbook (default: all of it).",
            "parameters": {"type": "object", "properties": {"dir": {"type": "string"}},
                "additionalProperties": false}}));
        tools.push(json!({"type": "function", "name": "card_ratings",
            "description": "The playbook's ratings lookup (`python3 data/card-ratings.py NAME ...`): the community rating of each named card or landscape, matched exactly, ignoring case.",
            "parameters": {"type": "object", "properties": {
                "names": {"type": "array", "items": {"type": "string"}}},
                "required": ["names"], "additionalProperties": false}}));
    }
    tools
}

fn description(tool: &str) -> &'static str {
    match tool {
        "state" => "The whole game as your seat sees it, and your decision.",
        "step" => {
            "Answer your decision, then wait for your next one (timeout_s defaults to 60, at most 600). Returns what changed and the next decision."
        }
        "wait" => {
            "Wait without answering until your next decision, an undo request (answer_undo) or the game's end. WAITING means call it again."
        }
        "cards" => "Card text and current costs, by name.",
        "log" => "Reread log entries after a sequence number.",
        "turns" => {
            "What each turn did (played, bought, gained, trashed, ...), by player and turn number; detail adds its log entries."
        }
        "play_treasures" => {
            "In a Buy phase, play every Treasure the decision offers, one after another."
        }
        _ => "",
    }
}

/// A path inside the playbook, never outside it.
fn inside(playbook: &Path, path: &str) -> Result<PathBuf, String> {
    let root = playbook.canonicalize().map_err(|e| e.to_string())?;
    let full = root
        .join(path.trim_start_matches('/').trim_start_matches("playbook/"))
        .canonicalize()
        .map_err(|_| format!("{path}: no such file in the playbook"))?;
    if full.starts_with(&root) {
        Ok(full)
    } else {
        Err(format!("{path}: outside the playbook"))
    }
}

fn read_file(playbook: &Path, args: &Value) -> Result<String, String> {
    let path = inside(playbook, args["path"].as_str().unwrap_or(""))?;
    let text = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
    let offset = args["offset"].as_u64().unwrap_or(1).max(1) as usize - 1;
    let limit = args["limit"]
        .as_u64()
        .map(|l| l as usize)
        .unwrap_or(usize::MAX);
    Ok(text
        .lines()
        .skip(offset)
        .take(limit)
        .collect::<Vec<_>>()
        .join("\n"))
}

fn list_files(playbook: &Path, args: &Value) -> Result<String, String> {
    let root = playbook.canonicalize().map_err(|e| e.to_string())?;
    let start = inside(playbook, args["dir"].as_str().unwrap_or("."))?;
    let mut found = vec![];
    let mut stack = vec![start];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir)
            .map_err(|e| e.to_string())?
            .flatten()
        {
            let path = entry.path();
            let name = entry.file_name();
            if name.to_string_lossy().starts_with('.') {
                continue;
            }
            if path.is_dir() {
                stack.push(path);
            } else if let Ok(relative) = path.strip_prefix(&root) {
                found.push(relative.display().to_string());
            }
        }
    }
    found.sort();
    Ok(found.join("\n"))
}

async fn card_ratings(sandbox: &Sandbox, playbook: &Path, args: &Value) -> Result<String, String> {
    let names: Vec<String> = (args["names"].as_array().into_iter().flatten())
        .filter_map(|n| n.as_str().map(String::from))
        .collect();
    let mut path: Vec<PathBuf> = sandbox.path.clone();
    path.extend(
        std::env::var_os("PATH")
            .map(|p| std::env::split_paths(&p).collect::<Vec<_>>())
            .unwrap_or_default(),
    );
    let run = tokio::process::Command::new("python3")
        .arg("data/card-ratings.py")
        .args(&names)
        .current_dir(playbook)
        .env(
            "PATH",
            std::env::join_paths(path).map_err(|e| e.to_string())?,
        )
        .kill_on_drop(true)
        .output();
    let out = tokio::time::timeout(Duration::from_secs(30), run)
        .await
        .map_err(|_| "the ratings lookup timed out".to_string())?
        .map_err(|e| format!("running the ratings lookup: {e}"))?;
    let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    Ok(text)
}

pub async fn play(
    spec: &AgentSpec,
    sandbox: &Sandbox,
    job: &Job,
    server: &str,
    archive: &mut Archive,
    stop: CancellationToken,
) -> Result<(), String> {
    spec.validate()?;
    archive.publish(server, job)?;
    archive.record("seat", json!({"agent":spec,"job":job,"server":server}))?;
    let endpoint = crate::harness::endpoint(server, job)?;
    let client =
        match Client::connect_authenticated(&endpoint, job.official.then(|| job.token.clone()))
            .await
        {
            Ok(client) => client,
            Err(error) => {
                archive.record("seat_finished", json!({"result":{"Err":error}}))?;
                archive.finish_publish().await;
                return Err(error);
            }
        };
    let result = tokio::select! {
        biased;
        _ = stop.cancelled() => Err("seat runner cancelled".into()),
        result = play_inner(&client, spec, sandbox, job, archive) => result,
    };
    crate::harness::record_final_result(&client, archive, job).await?;
    archive.record("seat_finished", json!({"result": result}))?;
    client.close().await;
    archive.finish_publish().await;
    result
}

async fn play_inner(
    client: &Client,
    spec: &AgentSpec,
    sandbox: &Sandbox,
    job: &Job,
    archive: &mut Archive,
) -> Result<(), String> {
    let (tool, args) = if job.official {
        ("state", json!({}))
    } else {
        (
            "join_game",
            json!({"game_id": job.game_id, "token": job.token}),
        )
    };
    let joined = call(client, archive, "dominion", tool, args).await?;
    if !joined.ok {
        return Err(joined.text);
    }
    let session = Session::new(spec)?;
    let playbook = sandbox.playbook.as_deref();
    let mut instructions = BRIEF.to_string();
    if let Some(dir) = playbook {
        let agents = std::fs::read_to_string(dir.join("AGENTS.md"))
            .map_err(|e| format!("{}: {e}", dir.display()))?;
        instructions.push_str(&format!("\n\n{PLAYBOOK}\n\n# AGENTS.md\n{agents}"));
    }
    let mut first = String::new();
    if let Some(text) = spec
        .instructions
        .as_deref()
        .filter(|t| !t.trim().is_empty())
    {
        first.push_str(&format!("Player instructions:\n{}\n\n", text.trim()));
    }
    first.push_str(&format!(
        "Initial view and card reference:\n{}",
        joined.text
    ));
    let mut input = vec![json!({"type": "message", "role": "user",
        "content": [{"type": "input_text", "text": first}]})];
    let tools = tools(playbook.is_some());
    let keep = spec.context_results.unwrap_or(DEFAULT_KEEP_RESULTS);
    let mut game_results: Vec<usize> = vec![];
    let mut nudges = 0;
    let mut over = false;
    loop {
        let mut reasoning = json!({"summary": "detailed"});
        if let Some(effort) = &spec.effort {
            reasoning["effort"] = json!(effort);
        }
        let body = json!({
            "model": spec.model.as_deref().ok_or("an openai agent needs a model")?,
            "instructions": instructions,
            "input": input,
            "tools": tools,
            "tool_choice": "auto",
            "parallel_tool_calls": false,
            "reasoning": reasoning,
            "store": false,
            "stream": true,
            "include": ["reasoning.encrypted_content"],
            "prompt_cache_key": session.id,
        });
        archive.record(
            "direct_request",
            json!({"model": spec.model, "items": input.len()}),
        )?;
        let (items, usage) = session.respond(&body).await?;
        archive.record("direct_usage", usage)?;
        let mut calls = vec![];
        let mut said = String::new();
        for item in items {
            match item["type"].as_str() {
                Some("reasoning") => {
                    let text: Vec<&str> = (item["summary"].as_array().into_iter().flatten())
                        .filter_map(|s| s["text"].as_str())
                        .collect();
                    if !text.is_empty() {
                        archive.record("native_event", json!({"type": "item.completed",
                            "item": {"id": item["id"], "type": "reasoning", "text": text.join("\n\n")}}))?;
                    }
                }
                Some("message") => {
                    let text: Vec<&str> = (item["content"].as_array().into_iter().flatten())
                        .filter_map(|c| c["text"].as_str())
                        .collect();
                    said = text.join("\n");
                    archive.record(
                        "native_event",
                        json!({"type": "item.completed",
                        "item": {"id": item["id"], "type": "agent_message", "text": said}}),
                    )?;
                }
                Some("function_call") => calls.push(item.clone()),
                _ => {}
            }
            input.push(item);
        }
        if calls.is_empty() {
            if over || archive.has_result() {
                archive.record("native_response", json!({"response": said}))?;
                return Ok(());
            }
            let view = call(client, archive, "dominion", "state", token(job, json!({}))).await?;
            if view.text.contains("GAME OVER") {
                archive.record("native_response", json!({"response": said}))?;
                return Ok(());
            }
            nudges += 1;
            if nudges > MAX_NUDGES {
                return Err("the model stopped playing before the game finished".into());
            }
            input.push(json!({"type": "message", "role": "user", "content": [{"type": "input_text",
                "text": "The game isn't over: keep playing (step to answer your decision, wait when there is none)."}]}));
            continue;
        }
        for item in calls {
            let name = item["name"].as_str().unwrap_or("");
            let args: Value = serde_json::from_str(item["arguments"].as_str().unwrap_or("{}"))
                .unwrap_or(json!({}));
            let output = match (name, playbook) {
                ("read_file", Some(dir)) => {
                    playbook_call(archive, name, &args, read_file(dir, &args))?
                }
                ("list_files", Some(dir)) => {
                    playbook_call(archive, name, &args, list_files(dir, &args))?
                }
                ("card_ratings", Some(dir)) => {
                    let result = card_ratings(sandbox, dir, &args).await;
                    playbook_call(archive, name, &args, result)?
                }
                _ => match crate::harness::GAME_TOOLS.iter().find(|t| **t == name) {
                    Some(tool) => {
                        let args = if matches!(*tool, "cards" | "log" | "turns") {
                            args
                        } else {
                            token(job, args)
                        };
                        let mut args = args;
                        // The archive's call gives up after 90 seconds.
                        if args["timeout_s"].as_u64().is_some_and(|t| t > 60) {
                            args["timeout_s"] = json!(60);
                        }
                        let reply = match call(client, archive, "dominion", tool, args).await {
                            Ok(reply) => reply,
                            // An official seat's access ends with its game.
                            Err(_) if archive.has_result() => return Ok(()),
                            Err(error) => return Err(error),
                        };
                        if reply.text.contains("GAME OVER") {
                            over = true;
                        }
                        game_results.push(input.len());
                        if reply.ok {
                            reply.text
                        } else {
                            format!("ERROR: {}", reply.text)
                        }
                    }
                    None => format!("ERROR: no tool named {name:?}"),
                },
            };
            input.push(json!({"type": "function_call_output", "call_id": item["call_id"], "output": output}));
        }
        if game_results.len() > keep + PRUNE_BATCH {
            for index in game_results.drain(..PRUNE_BATCH) {
                input[index]["output"] = json!(
                    "[an earlier game update, left out to save room: state shows the whole game, log rereads its entries]"
                );
            }
        }
    }
}

/// Adds the seat's token to a game tool's arguments, except in an official game, where
/// the connection is the seat.
fn token(job: &Job, mut args: Value) -> Value {
    if !job.official {
        args["token"] = json!(job.token);
    }
    args
}

fn playbook_call(
    archive: &mut Archive,
    tool: &str,
    args: &Value,
    result: Result<String, String>,
) -> Result<String, String> {
    archive.record(
        "request",
        json!({"source": "playbook", "tool": tool, "arguments": args}),
    )?;
    let (ok, text) = match result {
        Ok(text) => (true, text),
        Err(e) => (false, format!("ERROR: {e}")),
    };
    archive.record(
        "response",
        json!({"source": "playbook", "tool": tool,
        "reply": {"ok": ok, "text": text}}),
    )?;
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn streamed_responses_give_their_items_and_usage() {
        let text = "event: x\ndata: {\"type\":\"response.output_item.done\",\"item\":{\"type\":\"message\"}}\n\ndata: {\"type\":\"response.completed\",\"response\":{\"usage\":{\"input_tokens\":3}}}\n";
        let (items, usage) = parse_stream(text).unwrap();
        assert_eq!(items, [json!({"type":"message"})]);
        assert_eq!(usage["input_tokens"], 3);
        assert!(parse_stream("data: {\"type\":\"response.failed\"}\n").is_err());
        assert!(parse_stream("data: {\"type\":\"response.created\"}\n").is_err());
    }

    #[test]
    fn playbook_reads_stay_inside_it() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("skills/a")).unwrap();
        std::fs::write(dir.path().join("skills/a/SKILL.md"), "one\ntwo\nthree").unwrap();
        std::fs::write(dir.path().join("AGENTS.md"), "x").unwrap();
        let read = |args: Value| read_file(dir.path(), &args);
        assert_eq!(
            read(json!({"path":"skills/a/SKILL.md","offset":2,"limit":1})).unwrap(),
            "two"
        );
        assert_eq!(read(json!({"path":"/playbook/AGENTS.md"})).unwrap(), "x");
        assert!(read(json!({"path":"../../etc/passwd"})).is_err());
        assert!(read(json!({"path":"/etc/passwd"})).is_err());
        assert_eq!(
            list_files(dir.path(), &json!({})).unwrap(),
            "AGENTS.md\nskills/a/SKILL.md"
        );
    }

    #[test]
    fn timestamps_are_rfc3339() {
        let t = std::time::UNIX_EPOCH + Duration::from_secs(1_794_863_720);
        assert_eq!(rfc3339(t), "2026-11-16T21:15:20Z");
    }
}
