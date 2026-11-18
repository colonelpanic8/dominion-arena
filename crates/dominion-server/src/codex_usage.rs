//! What an agent signed in with Codex's ChatGPT login spends, written as a Codex rollout
//! under that login's `sessions`, so what counts Codex's usage (T3 Code) counts it too.
//! The rollouts hold only usage: no `cwd` and no messages, and they sit outside the dated
//! folders Codex lists its own sessions from.

use serde_json::{Value, json};
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::time::SystemTime;

pub struct UsageLog {
    file: Option<File>,
    session: String,
    requests: u64,
}

impl UsageLog {
    /// A rollout for one seat's use of `model`, in `<codex_home>/sessions/dominion-arena`.
    /// One that can't be written is reported once and the seat plays on without it.
    pub fn open(codex_home: &Path, model: &str) -> Self {
        let session = uuid();
        let file = create(codex_home, &session, model)
            .inspect_err(|e| {
                eprintln!(
                    "dominion agents: usage log in {}: {e}",
                    codex_home.display()
                )
            })
            .ok();
        Self {
            file,
            session,
            requests: 0,
        }
    }

    /// One model request's usage, as the Responses API or `codex exec` reports it.
    pub fn record(&mut self, usage: &Value) {
        let Some(file) = &mut self.file else {
            return;
        };
        let count = |v: &Value| v.as_u64().unwrap_or(0);
        let input = count(&usage["input_tokens"]);
        let output = count(&usage["output_tokens"]);
        let cached = count(&usage["input_tokens_details"]["cached_tokens"])
            .max(count(&usage["cached_input_tokens"]))
            .min(input);
        let reasoning = count(&usage["output_tokens_details"]["reasoning_tokens"])
            .max(count(&usage["reasoning_output_tokens"]))
            .min(output);
        if input + output == 0 {
            return;
        }
        self.requests += 1;
        let line = json!({
            "timestamp": crate::openai::rfc3339(SystemTime::now()),
            "type": "token_usage_record",
            "payload": {
                "response_id": format!("{}:{}", self.session, self.requests),
                "usage": {
                    "input_tokens": input,
                    "cached_input_tokens": cached,
                    "output_tokens": output,
                    "reasoning_output_tokens": reasoning,
                    "total_tokens": count(&usage["total_tokens"]).max(input + output),
                },
            },
        });
        if let Err(e) = writeln!(file, "{line}") {
            eprintln!("dominion agents: usage log: {e}");
            self.file = None;
        }
    }
}

fn create(codex_home: &Path, session: &str, model: &str) -> std::io::Result<File> {
    let now = crate::openai::rfc3339(SystemTime::now());
    let dir = codex_home
        .join("sessions/dominion-arena")
        .join(&now[0..4])
        .join(&now[5..7])
        .join(&now[8..10]);
    std::fs::create_dir_all(&dir)?;
    let stamp = now.trim_end_matches('Z').replace(':', "-");
    let mut file = OpenOptions::new()
        .create_new(true)
        .append(true)
        .open(dir.join(format!("rollout-{stamp}-{session}.jsonl")))?;
    let meta = json!({"timestamp": now, "type": "session_meta",
        "payload": {"id": session, "originator": "dominion_arena", "model_provider": "openai"}});
    let context = json!({"timestamp": now, "type": "turn_context", "payload": {"model": model}});
    writeln!(file, "{meta}\n{context}")?;
    Ok(file)
}

fn uuid() -> String {
    let n =
        (rand::random::<u128>() & !(0xf000 << 64) & !(0xc << 60)) | (0x4000 << 64) | (0x8 << 60);
    let hex = format!("{n:032x}");
    format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_requests_as_codex_usage_ledger_entries() {
        let home = tempfile::tempdir().unwrap();
        let mut log = UsageLog::open(home.path(), "gpt-test");
        log.record(
            &json!({"input_tokens": 100, "input_tokens_details": {"cached_tokens": 40},
                "output_tokens": 30, "output_tokens_details": {"reasoning_tokens": 20},
                "total_tokens": 130}),
        );
        log.record(&json!({"input_tokens": 0, "output_tokens": 0}));
        log.record(&json!({"input_tokens": 10, "cached_input_tokens": 5, "output_tokens": 2}));
        let files: Vec<_> = walkdir(&home.path().join("sessions/dominion-arena"));
        assert_eq!(files.len(), 1);
        let name = files[0].file_name().unwrap().to_string_lossy().into_owned();
        assert!(name.starts_with("rollout-") && name.ends_with(&format!("{}.jsonl", log.session)));
        let lines: Vec<Value> = std::fs::read_to_string(&files[0])
            .unwrap()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(lines.len(), 4);
        assert_eq!(lines[0]["payload"]["id"], log.session);
        assert_eq!(lines[1]["payload"]["model"], "gpt-test");
        assert_eq!(
            lines[2]["payload"]["usage"],
            json!({"input_tokens": 100, "cached_input_tokens": 40, "output_tokens": 30,
                "reasoning_output_tokens": 20, "total_tokens": 130})
        );
        assert_eq!(lines[3]["payload"]["usage"]["total_tokens"], 12);
        assert_ne!(
            lines[2]["payload"]["response_id"],
            lines[3]["payload"]["response_id"]
        );
    }

    fn walkdir(dir: &Path) -> Vec<std::path::PathBuf> {
        let mut out = vec![];
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                out.extend(walkdir(&path));
            } else {
                out.push(path);
            }
        }
        out
    }

    #[test]
    fn session_ids_are_uuids() {
        let id = uuid();
        assert_eq!(id.len(), 36);
        assert_eq!(&id[14..15], "4");
        assert!(matches!(&id[19..20], "8" | "9" | "a" | "b"));
    }
}
