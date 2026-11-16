//! Durable records of native harness and game interactions.

use crate::client::{Client, Reply};
use crate::hub::now_ms;
use serde_json::{Value, json};
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Append-only interaction journal. A failed write stops play before the next action.
pub struct Archive {
    pub dir: PathBuf,
    file: File,
    live: Option<crate::transcript::Live>,
    result: bool,
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
            live: None,
            result: false,
        })
    }

    pub fn publish(&mut self, server: &str, job: &crate::house::Job) -> Result<(), String> {
        if job.transcript {
            self.live = Some(crate::transcript::Live::new(&self.dir, server, job)?);
        }
        Ok(())
    }

    pub async fn finish_publish(&mut self) {
        if let Some(live) = self.live.take()
            && let Err(error) = live.finish().await
        {
            eprintln!("dominion agents: {error}");
            let _ = self.record("transcript_delivery_error", json!({"error":error}));
        }
    }

    /// Whether a `result` record holds the finished game.
    pub fn has_result(&self) -> bool {
        self.result
    }

    /// Appends a record. The first game reply that shows the game over is also recorded
    /// as its `result`: an official seat's access ends with its game, so a later fetch
    /// may be refused.
    pub fn record(&mut self, kind: &str, mut data: Value) -> Result<(), String> {
        redact(&mut data);
        let finished = (kind == "response" && !self.result)
            .then(|| finished_game(&data).cloned())
            .flatten();
        self.result |= kind == "result";
        let time = now_ms();
        let line = json!({"time": time, "kind": kind, "data": data});
        serde_json::to_writer(&mut self.file, &line).map_err(|e| e.to_string())?;
        self.file.write_all(b"\n").map_err(|e| e.to_string())?;
        self.file
            .sync_data()
            .map_err(|e| format!("saving interaction archive: {e}"))?;
        if let Some(live) = &mut self.live {
            live.record(kind, &line["data"], time)?;
        }
        if let Some(state) = finished {
            self.record("result", state)?;
        }
        Ok(())
    }
}

/// The structured reply (`{view, glossary?}`, or a bare view) of a game tool's `response`
/// record, if it shows the game over.
fn finished_game(data: &Value) -> Option<&Value> {
    let state = data["reply"]
        .get("json")
        .filter(|_| data["source"] == "dominion")?;
    (state.get("view").unwrap_or(state)["game"]["running"] == false).then_some(state)
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

#[cfg(test)]
mod tests {
    use super::*;

    fn records(archive: &Archive) -> Vec<Value> {
        std::fs::read_to_string(archive.dir.join("interactions.jsonl"))
            .unwrap()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect()
    }

    #[test]
    fn the_first_game_over_reply_is_the_result() {
        let dir = tempfile::tempdir().unwrap();
        let mut archive = Archive::open(dir.path()).unwrap();
        let reply = |source: &str, running: bool, vp: u32| {
            json!({"source": source, "tool": "step", "reply": {"ok": true, "text": "",
                "json": {"view": {"game": {"running": running, "vp": vp}}, "glossary": {}}}})
        };
        archive
            .record("response", reply("dominion", true, 1))
            .unwrap();
        archive.record("response", reply("t3", false, 2)).unwrap();
        assert!(!archive.has_result());
        archive
            .record("response", reply("dominion", false, 3))
            .unwrap();
        archive
            .record("response", reply("dominion", false, 4))
            .unwrap();
        assert!(archive.has_result());
        let results: Vec<Value> = records(&archive)
            .into_iter()
            .filter(|r| r["kind"] == "result")
            .map(|r| r["data"].clone())
            .collect();
        assert_eq!(
            results,
            [json!({"view": {"game": {"running": false, "vp": 3}}, "glossary": {}})]
        );
    }
}
