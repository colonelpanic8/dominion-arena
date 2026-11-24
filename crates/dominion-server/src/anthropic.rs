//! The direct Anthropic agent: plays a seat by calling the Messages API itself, with no CLI
//! between the model and the game. It has the direct OpenAI agent's brief and tools
//! (`crate::openai`), and signs in with an API key.
//!
//! Claude's thinking blocks are bound to the conversation before them, so the history is
//! only ever appended to: the API clears old game updates itself (`clear_tool_uses`)
//! where the OpenAI agent replaces them in its input.

use crate::archive::{Archive, call};
use crate::client::Client;
use crate::harness::Sandbox;
use crate::house::Job;
use crate::openai::{
    BRIEF, DEFAULT_KEEP_RESULTS, MAX_NUDGES, PLAYBOOK, card_ratings, list_files, playbook_call,
    read_file, token, user_message,
};
use crate::runner::AgentSpec;
use serde_json::{Value, json};
use std::time::Duration;
use tokio_util::sync::CancellationToken;

const API_URL: &str = "https://api.anthropic.com/v1/messages";
const VERSION: &str = "2026-10-21";
const BETAS: &str = "context-management-2025-06-27";
const DEFAULT_KEY_ENV: &str = "ANTHROPIC_API_KEY";
/// The workspace a key that isn't scoped to one bills to (a user key, `sk-ant-usr-`).
const WORKSPACE_ENV: &str = "ANTHROPIC_WORKSPACE_ID";
const MAX_TOKENS: u64 = 64_000;
/// Old game updates are cleared this many at a time, as the OpenAI agent prunes them.
const CLEAR_BATCH: usize = 20;
/// The playbook's tools: reference the model read once, never cleared.
const PLAYBOOK_TOOLS: [&str; 3] = ["read_file", "list_files", "card_ratings"];

struct Session {
    key: String,
    workspace: Option<String>,
    agent: ureq::Agent,
}

impl Session {
    fn new(spec: &AgentSpec) -> Result<Self, String> {
        let var = spec.api_key_env.as_deref().unwrap_or(DEFAULT_KEY_ENV);
        let key =
            std::env::var(var).map_err(|_| format!("{var} is unset: it names the API key"))?;
        let seconds = u64::from(spec.response_seconds.unwrap_or(900));
        let agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(seconds)))
            .http_status_as_error(false)
            .build()
            .into();
        let workspace = std::env::var(WORKSPACE_ENV).ok().filter(|w| !w.is_empty());
        Ok(Self {
            key,
            workspace,
            agent,
        })
    }

    async fn respond(&self, body: &Value) -> Result<Reply, String> {
        let mut delay = Duration::from_secs(2);
        let mut attempt = 0;
        loop {
            attempt += 1;
            let agent = self.agent.clone();
            let key = self.key.clone();
            let workspace = self.workspace.clone();
            let payload = serde_json::to_vec(body).map_err(|e| e.to_string())?;
            let reply = tokio::task::spawn_blocking(move || {
                let mut request = agent
                    .post(API_URL)
                    .header("Content-Type", "application/json")
                    .header("Accept", "text/event-stream")
                    .header("x-api-key", &key)
                    .header("anthropic-version", VERSION)
                    .header("anthropic-beta", BETAS);
                if let Some(workspace) = &workspace {
                    request = request.header("anthropic-workspace-id", workspace);
                }
                let mut reply = request
                    .send(payload.as_slice())
                    .map_err(|e| e.to_string())?;
                let status = reply.status().as_u16();
                let text = reply
                    .body_mut()
                    .with_config()
                    .limit(256 * 1024 * 1024)
                    .read_to_string()
                    .map_err(|e| e.to_string())?;
                Ok::<_, String>((status, text))
            })
            .await
            .map_err(|e| e.to_string())?;
            let (status, text) = match reply {
                Ok(reply) => reply,
                Err(e) if attempt < 5 => {
                    eprintln!("dominion agents: Anthropic request failed ({e}); retrying");
                    tokio::time::sleep(delay).await;
                    delay *= 2;
                    continue;
                }
                Err(e) => return Err(format!("Anthropic: {e}")),
            };
            match status {
                200 => match parse_stream(&text) {
                    Err(e) if e.overloaded && attempt < 6 => {}
                    result => return result.map_err(|e| e.message),
                },
                429 | 500..=599 if attempt < 6 => {}
                _ => return Err(format!("Anthropic: HTTP {status}: {}", snippet(&text))),
            }
            tokio::time::sleep(delay).await;
            delay *= 2;
        }
    }
}

fn snippet(text: &str) -> String {
    text.chars().take(500).collect()
}

/// A streamed message, put back together.
#[derive(Debug)]
struct Reply {
    content: Vec<Value>,
    stop_reason: String,
    stop_details: Value,
    usage: Value,
    context_management: Value,
}

#[derive(Debug)]
struct StreamError {
    message: String,
    /// A transient failure reported in the stream, worth retrying.
    overloaded: bool,
}

fn parse_stream(text: &str) -> Result<Reply, StreamError> {
    let fail = |message: String| StreamError {
        message,
        overloaded: false,
    };
    let mut content: Vec<Value> = vec![];
    let mut partial: Vec<Option<String>> = vec![];
    let mut reply = Reply {
        content: vec![],
        stop_reason: String::new(),
        stop_details: Value::Null,
        usage: json!({}),
        context_management: Value::Null,
    };
    let mut stopped = false;
    for line in text.lines() {
        let Some(data) = line.strip_prefix("data:") else {
            continue;
        };
        let Ok(event) = serde_json::from_str::<Value>(data.trim()) else {
            continue;
        };
        let index = event["index"].as_u64().unwrap_or(0) as usize;
        match event["type"].as_str() {
            Some("message_start") => {
                if let Some(usage) = event["message"]["usage"].as_object() {
                    reply.usage = Value::Object(usage.clone());
                }
            }
            Some("content_block_start") => {
                if content.len() <= index {
                    content.resize(index + 1, Value::Null);
                    partial.resize(index + 1, None);
                }
                content[index] = event["content_block"].clone();
                if content[index]["type"] == "tool_use" {
                    partial[index] = Some(String::new());
                }
            }
            Some("content_block_delta") => {
                let Some(block) = content.get_mut(index) else {
                    return Err(fail(format!(
                        "Anthropic: a delta for block {index} before it"
                    )));
                };
                let delta = &event["delta"];
                let append = |block: &mut Value, field: &str, more: &Value| {
                    let text = format!(
                        "{}{}",
                        block[field].as_str().unwrap_or(""),
                        more.as_str().unwrap_or("")
                    );
                    block[field] = json!(text);
                };
                match delta["type"].as_str() {
                    Some("text_delta") => append(block, "text", &delta["text"]),
                    Some("thinking_delta") => append(block, "thinking", &delta["thinking"]),
                    Some("signature_delta") => append(block, "signature", &delta["signature"]),
                    Some("input_json_delta") => {
                        if let Some(Some(json)) = partial.get_mut(index) {
                            json.push_str(delta["partial_json"].as_str().unwrap_or(""));
                        }
                    }
                    _ => {}
                }
            }
            Some("content_block_stop") => {
                if let Some(Some(json)) = partial.get_mut(index).map(Option::take) {
                    let input = if json.trim().is_empty() {
                        json!({})
                    } else {
                        serde_json::from_str(&json).map_err(|e| {
                            fail(format!(
                                "Anthropic: tool input isn't JSON ({e}): {}",
                                snippet(&json)
                            ))
                        })?
                    };
                    content[index]["input"] = input;
                }
            }
            Some("message_delta") => {
                if let Some(reason) = event["delta"]["stop_reason"].as_str() {
                    reply.stop_reason = reason.into();
                }
                reply.stop_details = event["delta"]["stop_details"].clone();
                if let (Some(usage), Some(more)) =
                    (reply.usage.as_object_mut(), event["usage"].as_object())
                {
                    for (key, value) in more {
                        if !value.is_null() {
                            usage.insert(key.clone(), value.clone());
                        }
                    }
                }
                if !event["context_management"].is_null() {
                    reply.context_management = event["context_management"].clone();
                }
            }
            Some("message_stop") => stopped = true,
            Some("error") => {
                return Err(StreamError {
                    overloaded: event["error"]["type"] == "overloaded_error",
                    message: format!("Anthropic: {}", snippet(&event.to_string())),
                });
            }
            _ => {}
        }
    }
    if !stopped {
        return Err(fail(format!(
            "Anthropic: the response didn't complete: {}",
            snippet(text)
        )));
    }
    reply.content = content.into_iter().filter(|b| !b.is_null()).collect();
    Ok(reply)
}

/// The direct OpenAI agent's tools, as Messages API tools.
fn tools(playbook: bool) -> Vec<Value> {
    crate::openai::tools(playbook)
        .into_iter()
        .map(|t| json!({"name": t["name"], "description": t["description"], "input_schema": t["parameters"]}))
        .collect()
}

/// Once more than `keep` + `CLEAR_BATCH` game updates are in the conversation, clears all
/// but the newest `keep`.
fn context_management(keep: usize) -> Value {
    json!({"edits": [{
        "type": "clear_tool_uses_20250919",
        "trigger": {"type": "tool_uses", "value": keep + CLEAR_BATCH},
        "keep": {"type": "tool_uses", "value": keep},
        "exclude_tools": PLAYBOOK_TOOLS,
    }]})
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
    let result = match result {
        Ok(answer) => {
            crate::harness::debrief(&client, archive, job, answer).await;
            Ok(())
        }
        Err(e) => Err(e),
    };
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
) -> Result<Result<String, String>, String> {
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
    if (joined.json.as_ref())
        .is_some_and(|v| crate::harness::game_state(v)["game"]["running"] == false)
    {
        return Ok(Err(crate::harness::JOINED_OVER.into()));
    }
    let session = Session::new(spec)?;
    let model = spec
        .model
        .as_deref()
        .ok_or("an anthropic agent needs a model")?;
    let playbook = sandbox.playbook.as_deref();
    let mut instructions = BRIEF.to_string();
    if let Some(dir) = playbook {
        let agents = std::fs::read_to_string(dir.join("AGENTS.md"))
            .map_err(|e| format!("{}: {e}", dir.display()))?;
        instructions.push_str(&format!("\n\n{PLAYBOOK}\n\n# AGENTS.md\n{agents}"));
    }
    let mut first = crate::harness::opening(job, false);
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
    archive.record("direct_instructions", json!({"instructions": instructions}))?;
    let mut messages = vec![];
    say(archive, &mut messages, &first)?;
    let system =
        json!([{"type": "text", "text": instructions, "cache_control": {"type": "ephemeral"}}]);
    let tools = tools(playbook.is_some());
    let context = context_management(spec.context_results.unwrap_or(DEFAULT_KEEP_RESULTS));
    let mut nudges = 0;
    let mut over = false;
    let mut debrief = crate::harness::debrief_request(job).filter(|_| !job.official);
    let mut asked = false;
    loop {
        let mut body = json!({
            "model": model,
            "max_tokens": MAX_TOKENS,
            "stream": true,
            "system": system,
            "tools": tools,
            "tool_choice": {"type": "auto", "disable_parallel_tool_use": true},
            "thinking": {"type": "adaptive", "display": "summarized"},
            "cache_control": {"type": "ephemeral"},
            "context_management": context,
            "messages": messages,
        });
        if let Some(effort) = &spec.effort {
            body["output_config"] = json!({"effort": effort});
        }
        archive.record(
            "direct_request",
            json!({"model": model, "items": messages.len()}),
        )?;
        let reply = session.respond(&body).await?;
        archive.record("direct_usage", reply.usage.clone())?;
        if (reply.context_management["applied_edits"].as_array()).is_some_and(|e| !e.is_empty()) {
            archive.record("direct_context", reply.context_management.clone())?;
        }
        let mut calls = vec![];
        let mut said = String::new();
        for (n, block) in reply.content.iter().enumerate() {
            let id = format!("{}-{n}", messages.len());
            match block["type"].as_str() {
                Some("thinking") => {
                    if let Some(text) = block["thinking"].as_str().filter(|t| !t.is_empty()) {
                        archive.record(
                            "native_event",
                            json!({"type": "item.completed",
                            "item": {"id": id, "type": "reasoning", "text": text}}),
                        )?;
                    }
                }
                Some("text") => {
                    let text = block["text"].as_str().unwrap_or("");
                    said = if said.is_empty() {
                        text.into()
                    } else {
                        format!("{said}\n{text}")
                    };
                    archive.record(
                        "native_event",
                        json!({"type": "item.completed",
                        "item": {"id": id, "type": "agent_message", "text": text}}),
                    )?;
                }
                Some("tool_use") => {
                    archive.record(
                        "native_event",
                        json!({"type": "item.completed",
                        "item": {"id": id, "type": "function_call", "call_id": block["id"],
                        "name": block["name"], "arguments": block["input"].to_string()}}),
                    )?;
                    calls.push(block.clone());
                }
                _ => {}
            }
        }
        messages.push(json!({"role": "assistant", "content": reply.content}));
        match reply.stop_reason.as_str() {
            "refusal" => {
                return Err(format!("Anthropic declined: {}", reply.stop_details));
            }
            "max_tokens" if calls.is_empty() => {
                return Err("Anthropic: the reply reached max_tokens".into());
            }
            _ => {}
        }
        if calls.is_empty() {
            let ended = over
                || archive.has_result()
                || call(client, archive, "dominion", "state", token(job, json!({})))
                    .await?
                    .game_over();
            if ended {
                archive.record("native_response", json!({"response": said}))?;
                if let Some(request) = debrief.take() {
                    over = true;
                    asked = true;
                    say(archive, &mut messages, &request)?;
                    continue;
                }
                return Ok(if asked { Ok(said) } else { Ok(String::new()) });
            }
            nudges += 1;
            if nudges > MAX_NUDGES {
                return Err("the model stopped playing before the game finished".into());
            }
            say(
                archive,
                &mut messages,
                "The game isn't over: keep playing (step to answer your decision, wait when there is none).",
            )?;
            continue;
        }
        let mut results = vec![];
        for block in calls {
            let name = block["name"].as_str().unwrap_or("");
            let args = block["input"].clone();
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
                        let mut args = if matches!(*tool, "cards" | "log" | "turns") {
                            args
                        } else {
                            token(job, args)
                        };
                        // The archive's call gives up after 90 seconds.
                        if args["timeout_s"].as_u64().is_some_and(|t| t > 60) {
                            args["timeout_s"] = json!(60);
                        }
                        let reply = match call(client, archive, "dominion", tool, args).await {
                            Ok(reply) => reply,
                            // An official seat's access ends with its game.
                            Err(_) if archive.has_result() => return Ok(Ok(String::new())),
                            Err(error) => return Err(error),
                        };
                        if reply.game_over() {
                            over = true;
                        }
                        if reply.ok {
                            reply.text
                        } else {
                            format!("ERROR: {}", reply.text)
                        }
                    }
                    None => format!("ERROR: no tool named {name:?}"),
                },
            };
            archive.record(
                "direct_input",
                json!({"type": "function_call_output", "call_id": block["id"], "output": output}),
            )?;
            results.push(
                json!({"type": "tool_result", "tool_use_id": block["id"], "content": output}),
            );
        }
        messages.push(json!({"role": "user", "content": results}));
    }
}

/// Adds a user message to the conversation, recorded as the direct OpenAI agent records
/// its input.
fn say(archive: &mut Archive, messages: &mut Vec<Value>, text: &str) -> Result<(), String> {
    archive.record("direct_input", user_message(text))?;
    messages.push(json!({"role": "user", "content": [{"type": "text", "text": text}]}));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stream(events: &[Value]) -> String {
        events
            .iter()
            .map(|e| format!("event: {}\ndata: {e}\n\n", e["type"].as_str().unwrap()))
            .collect()
    }

    #[test]
    fn streamed_messages_are_put_back_together() {
        let text = stream(&[
            json!({"type":"message_start","message":{"usage":{"input_tokens":10,"cache_read_input_tokens":90,"output_tokens":1}}}),
            json!({"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":"","signature":""}}),
            json!({"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"Buy "}}),
            json!({"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"Lab."}}),
            json!({"type":"content_block_delta","index":0,"delta":{"type":"signature_delta","signature":"sig"}}),
            json!({"type":"content_block_stop","index":0}),
            json!({"type":"content_block_start","index":1,"content_block":{"type":"text","text":""}}),
            json!({"type":"content_block_delta","index":1,"delta":{"type":"text_delta","text":"Laboratory."}}),
            json!({"type":"content_block_stop","index":1}),
            json!({"type":"content_block_start","index":2,"content_block":{"type":"tool_use","id":"t1","name":"step","input":{}}}),
            json!({"type":"content_block_delta","index":2,"delta":{"type":"input_json_delta","partial_json":"{\"indices\":"}}),
            json!({"type":"content_block_delta","index":2,"delta":{"type":"input_json_delta","partial_json":"[3]}"}}),
            json!({"type":"content_block_stop","index":2}),
            json!({"type":"message_delta","delta":{"stop_reason":"tool_use"},"usage":{"output_tokens":40},
                "context_management":{"applied_edits":[]}}),
            json!({"type":"message_stop"}),
        ]);
        let reply = parse_stream(&text).unwrap();
        assert_eq!(
            reply.content,
            [
                json!({"type":"thinking","thinking":"Buy Lab.","signature":"sig"}),
                json!({"type":"text","text":"Laboratory."}),
                json!({"type":"tool_use","id":"t1","name":"step","input":{"indices":[3]}}),
            ]
        );
        assert_eq!(reply.stop_reason, "tool_use");
        assert_eq!(reply.usage["cache_read_input_tokens"], 90);
        assert_eq!(reply.usage["output_tokens"], 40);
        assert_eq!(reply.context_management["applied_edits"], json!([]));
    }

    #[test]
    fn incomplete_or_failed_streams_are_errors() {
        let start = json!({"type":"message_start","message":{"usage":{}}});
        assert!(
            !parse_stream(&stream(std::slice::from_ref(&start)))
                .unwrap_err()
                .overloaded
        );
        let overloaded = json!({"type":"error","error":{"type":"overloaded_error"}});
        assert!(
            parse_stream(&stream(&[start, overloaded]))
                .unwrap_err()
                .overloaded
        );
    }

    #[test]
    fn tools_take_the_messages_api_shape_and_clearing_spares_the_playbook() {
        let tools = tools(true);
        let step = tools.iter().find(|t| t["name"] == "step").unwrap();
        assert!(step["input_schema"]["properties"]["token"].is_null());
        assert!(step.get("parameters").is_none());
        let names: Vec<&str> = tools.iter().filter_map(|t| t["name"].as_str()).collect();
        let edit = &context_management(60)["edits"][0];
        for tool in edit["exclude_tools"].as_array().unwrap() {
            assert!(names.contains(&tool.as_str().unwrap()));
        }
        assert_eq!(edit["keep"]["value"], 60);
    }
}
