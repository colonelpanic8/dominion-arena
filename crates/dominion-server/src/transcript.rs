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
        let url = format!("{base}/api/games/{}/transcript", job.game_id);
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

    fn native(&mut self, data: &Value, time: u64, out: &mut Vec<TranscriptInput>) {
        if data["type"] == "stream_event" {
            let event = &data["event"];
            let delta = &event["delta"];
            let (kind, text) = if delta["type"] == "thinking_delta" {
                (Kind::Reasoning, delta["thinking"].as_str())
            } else {
                (Kind::Message, delta["text"].as_str())
            };
            if let Some(text) = text {
                if kind == Kind::Message {
                    self.message = true;
                }
                self.emit(
                    out,
                    time,
                    kind,
                    format!(
                        "r{}/claude/{}",
                        self.response,
                        event["index"].as_u64().unwrap_or(0)
                    ),
                    true,
                    text,
                );
            }
        } else if data["type"] == "assistant" {
            if let Some(blocks) = data["message"]["content"].as_array() {
                for (i, block) in blocks.iter().enumerate() {
                    let (kind, text) = if block["type"] == "thinking" {
                        (Kind::Reasoning, block["thinking"].as_str())
                    } else {
                        (Kind::Message, block["text"].as_str())
                    };
                    if let Some(text) = text {
                        if kind == Kind::Message {
                            self.message = true;
                        }
                        self.emit(
                            out,
                            time,
                            kind,
                            format!("r{}/claude/{i}", self.response),
                            false,
                            text,
                        );
                    }
                }
            }
        } else if matches!(
            data["type"].as_str(),
            Some("item.started" | "item.updated" | "item.completed")
        ) {
            let item = &data["item"];
            let kind = match item["type"].as_str() {
                Some("reasoning") => Kind::Reasoning,
                Some("agent_message") => Kind::Message,
                _ => return,
            };
            if let Some(text) = item["text"].as_str() {
                if kind == Kind::Message {
                    self.message = true;
                }
                self.emit(
                    out,
                    time,
                    kind,
                    format!(
                        "r{}/codex/{:?}/{}",
                        self.response,
                        kind,
                        item["id"].as_str().unwrap_or("message")
                    ),
                    false,
                    text,
                );
            }
        } else if data["event"] == "step_update" {
            let step = &data["step_update"];
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
                if kind == Kind::Message {
                    self.message = true;
                }
                self.emit(
                    out,
                    time,
                    kind,
                    format!(
                        "r{}/antigravity/{:?}/{}",
                        self.response,
                        kind,
                        step["step_index"].as_u64().unwrap_or(0)
                    ),
                    true,
                    text,
                );
            }
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
                    Some("answer") => {
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
                if reply["ok"] == false {
                    self.emit(
                        &mut out,
                        time,
                        Kind::Result,
                        format!("error-{}", self.offset + 1),
                        false,
                        reply["text"].as_str().unwrap_or("Request rejected"),
                    );
                } else if data["tool"] == "card" {
                    self.emit(
                        &mut out,
                        time,
                        Kind::Result,
                        format!("card-{}", self.offset + 1),
                        false,
                        reply["text"].as_str().unwrap_or(""),
                    );
                }
                if let Some(view) = reply
                    .get("json")
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
