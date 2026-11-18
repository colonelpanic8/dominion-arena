//! Normalized model output and game activity, uploaded independently of the archive.

use crate::house::Job;
use dominion_protocol::transcript::{TranscriptInput, TranscriptKind as Kind};
use serde_json::Value;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

pub struct Live {
    normalizer: Normalizer,
    file: File,
    send: Option<mpsc::UnboundedSender<TranscriptInput>>,
    task: Option<JoinHandle<Result<(), String>>>,
}

impl Drop for Live {
    fn drop(&mut self) {
        if let Some(task) = &self.task {
            task.abort();
        }
    }
}

impl Live {
    pub fn new(dir: &Path, server: &str, job: &Job) -> Result<Self, String> {
        let base = server
            .trim_end_matches('/')
            .strip_suffix("/mcp")
            .ok_or("transcript upload requires the server's /mcp endpoint")?;
        let url = match (job.official, job.official_account.as_deref()) {
            (false, _) => format!("{base}/api/games/{}/transcript", job.game_id),
            (true, None) => format!("{base}/api/online/transcript"),
            (true, Some(id)) => {
                crate::harness::endpoint(server, job)?;
                format!("{base}/api/online/{id}/transcript")
            }
        };
        let client: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(10)))
            .http_status_as_error(false)
            .max_redirects(0)
            .build()
            .into();
        let token = job.token.clone();
        let (send, mut receive) = mpsc::unbounded_channel::<TranscriptInput>();
        let task = tokio::spawn(async move {
            while let Some(input) = receive.recv().await {
                loop {
                    let (client, url, token) = (client.clone(), url.clone(), token.clone());
                    let body = serde_json::to_vec(&input).map_err(|e| e.to_string())?;
                    let result = tokio::task::spawn_blocking(move || {
                        client
                            .post(&url)
                            .header("Authorization", &format!("Bearer {token}"))
                            .header("Content-Type", "application/json")
                            .send(body.as_slice())
                    })
                    .await
                    .map_err(|e| e.to_string())?;
                    match result {
                        Ok(reply) if reply.status().is_success() => break,
                        Ok(reply) if reply.status().is_client_error() => {
                            return Err(format!("transcript upload: HTTP {}", reply.status()));
                        }
                        _ => tokio::time::sleep(Duration::from_secs(1)).await,
                    }
                }
            }
            Ok(())
        });
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let file = options
            .open(dir.join("transcript.jsonl"))
            .map_err(|e| e.to_string())?;
        let attempt = dir
            .file_name()
            .and_then(|s| s.to_str())
            .ok_or("archive has no attempt name")?
            .to_string();
        Ok(Self {
            normalizer: Normalizer::new(attempt),
            file,
            send: Some(send),
            task: Some(task),
        })
    }

    pub fn record(&mut self, kind: &str, data: &Value, time: u64) -> Result<(), String> {
        for input in self.normalizer.record(kind, data, time) {
            serde_json::to_writer(&mut self.file, &input).map_err(|e| e.to_string())?;
            self.file
                .write_all(b"\n")
                .and_then(|()| self.file.sync_data())
                .map_err(|e| e.to_string())?;
            if let Some(send) = &self.send {
                let _ = send.send(input);
            }
        }
        Ok(())
    }

    pub async fn finish(mut self) -> Result<(), String> {
        self.send.take();
        let mut task = self
            .task
            .take()
            .ok_or("transcript uploader already closed")?;
        match tokio::time::timeout(Duration::from_secs(10), &mut task).await {
            Ok(result) => result.map_err(|e| e.to_string())?,
            Err(_) => {
                task.abort();
                Err(
                    "transcript upload did not finish; its complete local transcript is saved"
                        .into(),
                )
            }
        }
    }
}

struct Normalizer {
    attempt: String,
    offset: u64,
    response: u64,
    message: bool,
    view: Value,
    game_seq: Option<u64>,
    /// The system prompt last shown, so a harness that repeats it shows it once.
    system: Option<String>,
    /// Claude's API message and content block being streamed: its thinking and text
    /// blocks are one transcript block each.
    claude_message: String,
    claude_block: u64,
}

/// A tool call as the model made it, without the MCP server's name prefix.
fn call_text(name: &str, args: &Value) -> String {
    let name = name
        .strip_prefix("mcp__dominion__")
        .or_else(|| name.strip_prefix("mcp_dominion_"))
        .unwrap_or(name);
    let parsed;
    let args = match args.as_str() {
        Some(text) => match serde_json::from_str::<Value>(text) {
            Ok(value) => {
                parsed = value;
                &parsed
            }
            Err(_) => return format!("{name} {text}"),
        },
        None => args,
    };
    match args {
        Value::Null => name.to_string(),
        Value::Object(map) if map.is_empty() => name.to_string(),
        _ => format!("{name} {args}"),
    }
}

/// The text of tool-result content: a string, or MCP / Anthropic content blocks.
fn content_text(content: &Value) -> String {
    match content {
        Value::String(text) => text.clone(),
        Value::Array(parts) => parts
            .iter()
            .map(|part| match part["text"].as_str() {
                Some(text) => text.to_string(),
                None => format!("[{}]", part["type"].as_str().unwrap_or("content")),
            })
            .collect::<Vec<_>>()
            .join("\n"),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

impl Normalizer {
    fn new(attempt: String) -> Self {
        Self {
            attempt,
            offset: 0,
            response: 0,
            message: false,
            view: Value::Null,
            game_seq: None,
            system: None,
            claude_message: String::new(),
            claude_block: 0,
        }
    }

    fn emit(
        &mut self,
        out: &mut Vec<TranscriptInput>,
        time: u64,
        kind: Kind,
        block: String,
        mut append: bool,
        mut text: &str,
    ) {
        while !text.is_empty() {
            let mut end = text.len().min(8192);
            while !text.is_char_boundary(end) {
                end -= 1;
            }
            self.offset += 1;
            out.push(TranscriptInput {
                attempt: self.attempt.clone(),
                offset: self.offset,
                time,
                kind,
                block: block.clone(),
                append,
                text: text[..end].into(),
            });
            append = true;
            text = &text[end..];
        }
    }

    fn system(&mut self, out: &mut Vec<TranscriptInput>, time: u64, text: Option<&str>) {
        if let Some(text) = text.filter(|t| !t.trim().is_empty())
            && self.system.as_deref() != Some(text)
        {
            self.system = Some(text.to_string());
            let block = format!("system-{}", self.offset + 1);
            self.emit(out, time, Kind::System, block, false, text);
        }
    }

    fn said(
        &mut self,
        out: &mut Vec<TranscriptInput>,
        time: u64,
        kind: Kind,
        block: String,
        append: bool,
        text: &str,
    ) {
        if kind == Kind::Message {
            self.message = true;
        }
        self.emit(out, time, kind, block, append, text);
    }

    fn claude(&mut self, data: &Value, time: u64, out: &mut Vec<TranscriptInput>) {
        let response = self.response;
        if data["type"] == "stream_event" {
            let event = &data["event"];
            match event["type"].as_str() {
                Some("message_start") => {
                    self.claude_message = event["message"]["id"].as_str().unwrap_or("").into();
                }
                Some("content_block_start") => {
                    self.claude_block = event["index"].as_u64().unwrap_or(0);
                }
                _ => {}
            }
            let delta = &event["delta"];
            let (kind, text) = match delta["type"].as_str() {
                Some("thinking_delta") => (Kind::Reasoning, delta["thinking"].as_str()),
                Some("text_delta") => (Kind::Message, delta["text"].as_str()),
                _ => return,
            };
            if let Some(text) = text {
                let block = format!(
                    "r{response}/claude/{}/{}",
                    self.claude_message,
                    event["index"].as_u64().unwrap_or(0)
                );
                self.said(out, time, kind, block, true, text);
            }
            return;
        }
        let content = &data["message"]["content"];
        if data["type"] == "user" {
            match content {
                Value::String(text) => {
                    let block = format!("user-{}", self.offset + 1);
                    self.emit(out, time, Kind::Observation, block, false, text);
                }
                Value::Array(parts) => {
                    for part in parts {
                        if part["type"] == "tool_result" {
                            let mut text = content_text(&part["content"]);
                            if part["is_error"] == true {
                                text = format!("Error: {text}");
                            }
                            let block = format!(
                                "result/{}",
                                part["tool_use_id"].as_str().unwrap_or("tool")
                            );
                            self.emit(out, time, Kind::Result, block, false, &text);
                        } else if let Some(text) = part["text"].as_str() {
                            let block = format!("user-{}", self.offset + 1);
                            self.emit(out, time, Kind::Observation, block, false, text);
                        }
                    }
                }
                _ => {}
            }
            return;
        }
        let Some(blocks) = content.as_array() else {
            return;
        };
        let id = data["message"]["id"].as_str().unwrap_or("");
        let streamed = blocks.len() == 1 && id == self.claude_message;
        for (i, block) in blocks.iter().enumerate() {
            let index = if streamed {
                self.claude_block
            } else {
                i as u64
            };
            let key = format!("r{response}/claude/{id}/{index}");
            match block["type"].as_str() {
                Some("thinking") => {
                    if let Some(text) = block["thinking"].as_str() {
                        self.said(out, time, Kind::Reasoning, key, false, text);
                    }
                }
                Some("text") => {
                    if let Some(text) = block["text"].as_str() {
                        self.said(out, time, Kind::Message, key, false, text);
                    }
                }
                Some("tool_use") => {
                    let text = call_text(block["name"].as_str().unwrap_or("tool"), &block["input"]);
                    let key = format!("call/{}", block["id"].as_str().unwrap_or(&key));
                    self.emit(out, time, Kind::Call, key, false, &text);
                }
                _ => {}
            }
        }
    }

    fn codex(&mut self, data: &Value, time: u64, out: &mut Vec<TranscriptInput>) {
        let item = &data["item"];
        let id = item["id"].as_str().unwrap_or("item");
        let completed = data["type"] == "item.completed";
        match item["type"].as_str() {
            Some(kind @ ("reasoning" | "agent_message")) => {
                let kind = if kind == "reasoning" {
                    Kind::Reasoning
                } else {
                    Kind::Message
                };
                if let Some(text) = item["text"].as_str() {
                    let block = format!("r{}/codex/{kind:?}/{id}", self.response);
                    self.said(out, time, kind, block, false, text);
                }
            }
            Some("mcp_tool_call") => {
                let text = call_text(item["tool"].as_str().unwrap_or("tool"), &item["arguments"]);
                self.emit(out, time, Kind::Call, format!("call/{id}"), false, &text);
                if completed {
                    let text = match item["error"]["message"].as_str() {
                        Some(error) => format!("Error: {error}"),
                        None => content_text(&item["result"]["content"]),
                    };
                    self.emit(
                        out,
                        time,
                        Kind::Result,
                        format!("result/{id}"),
                        false,
                        &text,
                    );
                }
            }
            Some("command_execution") => {
                let text = format!("$ {}", item["command"].as_str().unwrap_or(""));
                self.emit(out, time, Kind::Call, format!("call/{id}"), false, &text);
                if completed {
                    let mut text = item["aggregated_output"].as_str().unwrap_or("").to_string();
                    if let Some(code) = item["exit_code"].as_i64().filter(|c| *c != 0) {
                        text.push_str(&format!("\n[exit code {code}]"));
                    }
                    self.emit(
                        out,
                        time,
                        Kind::Result,
                        format!("result/{id}"),
                        false,
                        &text,
                    );
                }
            }
            Some("error") => {
                let text = item["message"].as_str().unwrap_or("error");
                self.emit(out, time, Kind::Status, format!("error/{id}"), false, text);
            }
            Some(other) if completed => {
                let mut rest = item.clone();
                if let Some(map) = rest.as_object_mut() {
                    map.remove("id");
                    map.remove("type");
                }
                let text = format!("{other} {rest}");
                self.emit(out, time, Kind::Call, format!("call/{id}"), false, &text);
            }
            _ => {}
        }
    }

    fn antigravity(&mut self, data: &Value, time: u64, out: &mut Vec<TranscriptInput>) {
        let step = &data["step_update"];
        let index = step["step_index"].as_u64().unwrap_or(0);
        if step["step_type"] == "tool" {
            let info = &step["tool_info"];
            let name = info["name"].as_str().unwrap_or("tool");
            if !info["parameters"].is_null() || step["state"] == "ACTIVE" {
                let text = call_text(name, &info["parameters"]);
                let block = format!("r{}/call/{index}", self.response);
                self.emit(out, time, Kind::Call, block, false, &text);
            }
            if let Some(output) = info["output"].as_str() {
                let block = format!("r{}/result/{index}", self.response);
                self.emit(out, time, Kind::Result, block, false, output);
            }
            return;
        }
        let kind = if matches!(
            step["step_type"].as_str(),
            Some("thinking" | "thought" | "agent_thought" | "reasoning")
        ) || step["thinking_delta"].is_string()
            || step["thought_delta"].is_string()
        {
            Kind::Reasoning
        } else if step["step_type"] == "agent_response" {
            Kind::Message
        } else {
            return;
        };
        if let Some(text) = step["thinking_delta"]
            .as_str()
            .or_else(|| step["thought_delta"].as_str())
            .or_else(|| step["text_delta"].as_str())
        {
            let block = format!("r{}/antigravity/{kind:?}/{index}", self.response);
            self.said(out, time, kind, block, true, text);
        }
    }

    fn native(&mut self, data: &Value, time: u64, out: &mut Vec<TranscriptInput>) {
        if matches!(
            data["type"].as_str(),
            Some("stream_event" | "assistant" | "user")
        ) {
            self.claude(data, time, out);
        } else if matches!(
            data["type"].as_str(),
            Some("item.started" | "item.updated" | "item.completed")
        ) {
            if data["item"]["type"] == "function_call" {
                let item = &data["item"];
                let text = call_text(item["name"].as_str().unwrap_or("tool"), &item["arguments"]);
                let block = format!("call/{}", item["call_id"].as_str().unwrap_or("call"));
                self.emit(out, time, Kind::Call, block, false, &text);
            } else {
                self.codex(data, time, out);
            }
        } else if data["event"] == "step_update" {
            self.antigravity(data, time, out);
        } else if matches!(data["type"].as_str(), Some("error" | "turn.failed")) {
            let text = data["message"]
                .as_str()
                .or_else(|| data["error"]["message"].as_str())
                .unwrap_or("model error");
            let block = format!("error-{}", self.offset + 1);
            self.emit(out, time, Kind::Status, block, false, text);
        } else if data["type"] == "reasoning"
            && let Some(text) = data["text"].as_str()
        {
            self.emit(
                out,
                time,
                Kind::Reasoning,
                format!("r{}/reasoning", self.response),
                false,
                text,
            );
        }
    }

    /// An item the direct OpenAI agent adds to the model's input.
    fn direct_input(&mut self, item: &Value, time: u64, out: &mut Vec<TranscriptInput>) {
        if item["type"] == "function_call_output" {
            let block = format!("result/{}", item["call_id"].as_str().unwrap_or("call"));
            let text = content_text(&item["output"]);
            self.emit(out, time, Kind::Result, block, false, &text);
        } else {
            let text = content_text(&item["content"]);
            let block = format!("input-{}", self.offset + 1);
            self.emit(out, time, Kind::Observation, block, false, &text);
        }
    }

    fn record(&mut self, kind: &str, data: &Value, time: u64) -> Vec<TranscriptInput> {
        let mut out = vec![];
        match kind {
            "seat" => self.emit(
                &mut out,
                time,
                Kind::Status,
                "seat".into(),
                false,
                "Agent connected",
            ),
            "native_request" => {
                self.response += 1;
                self.message = false;
                self.system(&mut out, time, data["system"].as_str());
                if let Some(prompt) = data["prompt"].as_str() {
                    self.emit(
                        &mut out,
                        time,
                        Kind::Observation,
                        format!("r{}/input", self.response),
                        false,
                        prompt,
                    );
                }

                self.emit(
                    &mut out,
                    time,
                    Kind::Status,
                    format!("r{}/status", self.response),
                    false,
                    "Waiting for model",
                );
            }
            "direct_instructions" => self.system(&mut out, time, data["instructions"].as_str()),
            "direct_input" => self.direct_input(data, time, &mut out),
            "direct_pruned" => {
                let count = data["call_ids"].as_array().map_or(0, Vec::len);
                let text = format!(
                    "{count} earlier tool results replaced in the model's context with: {}",
                    data["output"].as_str().unwrap_or("")
                );
                let block = format!("pruned-{}", self.offset + 1);
                self.emit(&mut out, time, Kind::Status, block, false, &text);
            }
            "native_event" => self.native(data, time, &mut out),
            "native_response" => {
                if !self.message
                    && let Some(text) = data["response"].as_str()
                {
                    self.emit(
                        &mut out,
                        time,
                        Kind::Message,
                        format!("r{}/reply", self.response),
                        false,
                        text,
                    );
                }
                self.emit(
                    &mut out,
                    time,
                    Kind::Status,
                    format!("r{}/complete", self.response),
                    false,
                    "Model replied",
                );
            }
            "request" => {
                let args = &data["arguments"];
                let text = match data["tool"].as_str() {
                    Some("play_treasures") => Some("Play Treasures".to_string()),
                    Some("card") => Some(format!(
                        "Look up {}",
                        args["name"].as_str().unwrap_or("card")
                    )),
                    Some("cards") => Some(format!(
                        "Look up {}",
                        args["names"]
                            .as_array()
                            .map(|names| names
                                .iter()
                                .filter_map(Value::as_str)
                                .collect::<Vec<_>>()
                                .join(", "))
                            .unwrap_or_default()
                    )),
                    Some("answer" | "step") => {
                        let mut labels = vec![];
                        if let Some(indices) = args["indices"].as_array() {
                            for i in indices {
                                let option = self.view["decision"]["options"]
                                    .as_array()
                                    .and_then(|xs| xs.iter().find(|o| o["index"] == *i));
                                labels.push(
                                    option
                                        .and_then(|o| o["label"].as_str())
                                        .map(String::from)
                                        .unwrap_or_else(|| format!("Option {i}")),
                                );
                            }
                        }
                        if let Some(name) = args["name"].as_str() {
                            labels.push(format!("Name {name}"));
                        }
                        if let Some(way) = args["way"].as_str() {
                            labels.push(format!("Way: {way}"));
                        }
                        Some(if labels.is_empty() {
                            "Submit empty selection".into()
                        } else {
                            labels.join("; ")
                        })
                    }
                    _ => None,
                };
                if let Some(text) = text {
                    let block = format!("action-{}", self.offset + 1);
                    self.emit(&mut out, time, Kind::Action, block, false, &text);
                }
            }
            "response" => {
                let reply = &data["reply"];
                // What a game or playbook tool returned is shown from the model's own
                // stream, as it saw it. T3 Code's tools are the runner's, not the model's.
                if reply["ok"] == false && data["source"] == "t3" {
                    self.emit(
                        &mut out,
                        time,
                        Kind::Status,
                        format!("error-{}", self.offset + 1),
                        false,
                        reply["text"].as_str().unwrap_or("Request rejected"),
                    );
                }
                if let Some(view) = reply
                    .get("json")
                    .map(|v| v.get("view").unwrap_or(v))
                    .filter(|v| v["decision"].is_object() || v["game"].is_object())
                {
                    if let Some(events) = view["sinceLastDecision"].as_array() {
                        for event in events {
                            if let (Some(seq), Some(text)) =
                                (event["seq"].as_u64(), event["text"].as_str())
                                && self.game_seq.is_none_or(|last| seq > last)
                            {
                                self.game_seq = Some(seq);
                                self.emit(
                                    &mut out,
                                    time,
                                    Kind::Game,
                                    format!("game/{seq}"),
                                    false,
                                    text,
                                );
                            }
                        }
                    }
                    self.view = view.clone();
                }
            }
            "seat_finished" => {
                let text = data["result"]["Err"]
                    .as_str()
                    .map(|e| format!("Stopped: {e}"))
                    .unwrap_or_else(|| "Seat finished".into());
                self.emit(
                    &mut out,
                    time,
                    Kind::Status,
                    "seat/end".into(),
                    false,
                    &text,
                );
            }
            "native_timeout" | "transport_error" => self.emit(
                &mut out,
                time,
                Kind::Status,
                format!("error-{}", self.offset + 1),
                false,
                data["error"].as_str().unwrap_or("Model response timed out"),
            ),
            _ => {}
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn streamed_blocks_keep_identity_and_actions_follow_model_output() {
        let mut n = Normalizer::new("attempt".into());
        n.record(
            "native_request",
            &json!({"prompt":"Your legal observation"}),
            1,
        );
        let first=n.record("native_event",&json!({"type":"stream_event","event":{"index":0,"delta":{"type":"thinking_delta","thinking":"Buy "}}}),2);
        let second=n.record("native_event",&json!({"type":"stream_event","event":{"index":0,"delta":{"type":"thinking_delta","thinking":"Silver"}}}),3);
        let full=n.record("native_event",&json!({"type":"assistant","message":{"content":[{"type":"thinking","thinking":"Buy Silver"},{"type":"text","text":"a reply"}]}}),4);
        assert_eq!(first[0].block, second[0].block);
        assert_eq!(first[0].block, full[0].block);
        assert!(first[0].append && second[0].append && !full[0].append);
        assert_eq!(first[0].kind, Kind::Reasoning);
        assert!(
            !n.record("native_response", &json!({"response":"a reply"}), 5)
                .iter()
                .any(|e| e.kind == Kind::Message)
        );
        n.record("response",&json!({"reply":{"ok":true,"json":{"decision":{"options":[{"index":3,"label":"Buy Silver ($3)"}]},"sinceLastDecision":[]}}}),6);
        let action = n.record(
            "request",
            &json!({"tool":"answer","arguments":{"indices":[3],"token":"never included"}}),
            7,
        );
        assert_eq!(action[0].text, "Buy Silver ($3)");
        assert!(action[0].offset > full[1].offset);
    }

    #[test]
    fn common_envelopes_keep_game_events_and_step_action_labels() {
        let mut n = Normalizer::new("attempt".into());
        let events = n.record(
            "response",
            &json!({"tool":"state","reply":{"ok":true,"json":{"view":{
                "decision":{"options":[{"index":2,"label":"Buy Silver ($3)"}]},
                "sinceLastDecision":[{"seq":7,"text":"Ada plays Copper"}]
            }}}}),
            1,
        );
        assert_eq!(events[0].kind, Kind::Game);
        assert_eq!(events[0].text, "Ada plays Copper");
        let action = n.record("request", &json!({"tool":"step","arguments":{"decision":"d1","indices":[2],"expect":["Buy Silver"]}}), 2);
        assert_eq!(action[0].text, "Buy Silver ($3)");
        let lookup = n.record(
            "request",
            &json!({"tool":"cards","arguments":{"names":["Silver","Village"]}}),
            3,
        );
        assert_eq!(lookup[0].text, "Look up Silver, Village");
    }

    fn texts(out: &[TranscriptInput]) -> Vec<(Kind, &str, &str)> {
        out.iter()
            .map(|e| (e.kind, e.block.as_str(), e.text.as_str()))
            .collect()
    }

    #[test]
    fn claude_messages_keep_their_own_blocks_and_show_tool_calls_and_results() {
        let mut n = Normalizer::new("attempt".into());
        let request = n.record(
            "native_request",
            &json!({"system":"You play Dominion.","prompt":"Your view"}),
            1,
        );
        assert_eq!(request[0].kind, Kind::System);
        assert_eq!(request[0].text, "You play Dominion.");
        let again = n.record(
            "native_request",
            &json!({"system":"You play Dominion.","prompt":"Next"}),
            2,
        );
        assert!(again.iter().all(|e| e.kind != Kind::System));
        let mut blocks = vec![];
        for (time, id) in [(3, "m1"), (6, "m2")] {
            n.record("native_event", &json!({"type":"stream_event","event":{"type":"message_start","message":{"id":id}}}), time);
            n.record(
                "native_event",
                &json!({"type":"stream_event","event":{"type":"content_block_start","index":0}}),
                time,
            );
            let delta = n.record("native_event", &json!({"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"Hmm"}}}), time);
            let full = n.record("native_event", &json!({"type":"assistant","message":{"id":id,"content":[{"type":"thinking","thinking":"Hmm."}]}}), time);
            assert_eq!(delta[0].block, full[0].block);
            blocks.push(full[0].block.clone());
        }
        assert_ne!(blocks[0], blocks[1]);
        n.record(
            "native_event",
            &json!({"type":"stream_event","event":{"type":"content_block_start","index":1}}),
            7,
        );
        let call = n.record("native_event", &json!({"type":"assistant","message":{"id":"m2","content":[
            {"type":"tool_use","id":"toolu_1","name":"mcp__dominion__step","input":{"decision":"d1","indices":[2]}}]}}), 7);
        assert_eq!(
            texts(&call),
            [(
                Kind::Call,
                "call/toolu_1",
                r#"step {"decision":"d1","indices":[2]}"#
            )]
        );
        let result = n.record("native_event", &json!({"type":"user","message":{"content":[
            {"type":"tool_result","tool_use_id":"toolu_1","content":[{"type":"text","text":"GAME g1: turn 2"}]}]}}), 8);
        assert_eq!(
            texts(&result),
            [(Kind::Result, "result/toolu_1", "GAME g1: turn 2")]
        );
        let failed = n.record("native_event", &json!({"type":"user","message":{"content":[
            {"type":"tool_result","tool_use_id":"toolu_2","is_error":true,"content":"no such decision"}]}}), 9);
        assert_eq!(failed[0].text, "Error: no such decision");
    }

    #[test]
    fn direct_agents_show_instructions_inputs_calls_and_outputs() {
        let mut n = Normalizer::new("attempt".into());
        let system = n.record(
            "direct_instructions",
            &json!({"instructions":"Play well."}),
            1,
        );
        assert_eq!(texts(&system)[0].0, Kind::System);
        let prompt = n.record(
            "direct_input",
            &json!({"type":"message","role":"user",
            "content":[{"type":"input_text","text":"Initial view"}]}),
            2,
        );
        assert_eq!(prompt[0].kind, Kind::Observation);
        assert_eq!(prompt[0].text, "Initial view");
        let call = n.record(
            "native_event",
            &json!({"type":"item.completed","item":{"id":"fc_1",
            "type":"function_call","call_id":"call_1","name":"state","arguments":"{}"}}),
            3,
        );
        assert_eq!(texts(&call), [(Kind::Call, "call/call_1", "state")]);
        let output = n.record(
            "direct_input",
            &json!({"type":"function_call_output","call_id":"call_1","output":"GAME g1"}),
            4,
        );
        assert_eq!(texts(&output), [(Kind::Result, "result/call_1", "GAME g1")]);
        assert!(
            n.record("direct_request", &json!({"model":"m","items":4}), 5)
                .is_empty()
        );
    }

    #[test]
    fn codex_and_antigravity_tool_calls_have_their_results() {
        let mut n = Normalizer::new("attempt".into());
        let started = n.record("native_event", &json!({"type":"item.started","item":{"id":"item_1",
            "type":"mcp_tool_call","server":"dominion","tool":"wait","arguments":{"timeout_s":30}}}), 1);
        let done = n.record(
            "native_event",
            &json!({"type":"item.completed","item":{"id":"item_1",
            "type":"mcp_tool_call","server":"dominion","tool":"wait","arguments":{"timeout_s":30},
            "result":{"content":[{"type":"text","text":"Your turn"}]}}}),
            2,
        );
        assert_eq!(
            texts(&started),
            [(Kind::Call, "call/item_1", r#"wait {"timeout_s":30}"#)]
        );
        assert_eq!(done[1].kind, Kind::Result);
        assert_eq!(done[1].text, "Your turn");
        let call = n.record("native_event", &json!({"event":"step_update","step_update":{"state":"ACTIVE",
            "step_index":2,"step_type":"tool","tool_info":{"name":"mcp_dominion_answer","parameters":{"indices":[3]}}}}), 3);
        let output = n.record("native_event", &json!({"event":"step_update","step_update":{"state":"DONE",
            "step_index":2,"step_type":"tool","tool_info":{"name":"mcp_dominion_answer","output":"GAME g5"}}}), 4);
        assert_eq!(call[0].text, r#"answer {"indices":[3]}"#);
        assert_eq!(
            (output[0].kind, output[0].text.as_str()),
            (Kind::Result, "GAME g5")
        );
    }

    #[test]
    fn provider_reasoning_and_unicode_chunks_are_preserved_without_credentials() {
        let mut n = Normalizer::new("attempt".into());
        let text = "é🃏".repeat(4000);
        let chunks = n.record(
            "native_event",
            &json!({"type":"item.completed","item":{"id":"reason","type":"reasoning","text":text}}),
            1,
        );
        assert_eq!(
            chunks.iter().map(|e| e.text.as_str()).collect::<String>(),
            text
        );
        assert!(
            chunks
                .iter()
                .all(|e| e.text.len() <= 8192 && e.kind == Kind::Reasoning)
        );
        assert!(!chunks[0].append);
        assert!(chunks[1..].iter().all(|e| e.append));
        let gemini=n.record("native_event",&json!({"event":"step_update","step_update":{"step_type":"agent_thought","step_index":2,"text_delta":"Evaluate engines"}}),2);
        assert_eq!(gemini[0].kind, Kind::Reasoning);
        let raw = n.record("seat", &json!({"job":{"token":"secret"}}), 3);
        assert!(!serde_json::to_string(&raw).unwrap().contains("secret"));
    }
}
