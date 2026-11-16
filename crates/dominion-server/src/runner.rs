//! The server's agent runner. Each native harness receives game observations and
//! returns structured actions; only Dominion's bridge holds the seat credentials.

use crate::house::{FromRunner, Job, Offer, ToRunner};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

const RECONNECT: Duration = Duration::from_secs(5);
const GRACE: Duration = Duration::from_secs(30);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Program {
    Claude,
    Codex,
    #[serde(alias = "gemini")]
    Antigravity,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentSpec {
    pub name: String,
    pub program: Program,
    pub model: Option<String>,
    pub command: Option<String>,
    pub about: Option<String>,
    pub instructions: Option<String>,
    /// Native reasoning effort.
    pub effort: Option<String>,
    /// Maximum model inactivity between events, excluding MCP calls (default 300 seconds).
    pub response_seconds: Option<u32>,
    /// Legacy setting, accepted for compatibility. Native sessions retain their own context.
    pub history_turns: Option<usize>,
    /// Provider's native credential/config directory (default its standard location).
    pub auth_dir: Option<PathBuf>,
}

impl AgentSpec {
    fn about(&self) -> String {
        self.about.clone().unwrap_or_else(|| {
            format!(
                "{:?}{}",
                self.program,
                self.model
                    .as_ref()
                    .map_or(String::new(), |m| format!(" ({m})"))
            )
        })
    }
    pub fn validate(&self) -> Result<(), String> {
        if self.name.trim().is_empty() || self.name.trim() != self.name {
            return Err("agent name must be nonempty without surrounding whitespace".into());
        }
        if self.response_seconds == Some(0) {
            return Err("response_seconds must be positive".into());
        }
        if let Some(effort) = &self.effort {
            let allowed = match self.program {
                Program::Claude => &["low", "medium", "high", "max"][..],
                Program::Codex => {
                    &["minimal", "low", "medium", "high", "xhigh", "max", "ultra"][..]
                }
                Program::Antigravity => &["low", "medium", "high", "xhigh", "max"][..],
            };
            if !allowed.contains(&effort.as_str()) {
                return Err(format!("unsupported {:?} effort {effort:?}", self.program));
            }
        }
        Ok(())
    }
}

pub fn default_agents() -> Vec<AgentSpec> {
    [
        ("Claude Opus", Program::Claude, Some("opus")),
        ("Claude Sonnet", Program::Claude, Some("sonnet")),
        ("Codex", Program::Codex, None),
    ]
    .into_iter()
    .map(|(name, program, model)| AgentSpec {
        name: name.into(),
        program,
        model: model.map(String::from),
        command: None,
        about: None,
        instructions: None,
        effort: None,
        response_seconds: None,
        history_turns: None,
        auth_dir: None,
    })
    .collect()
}

#[derive(Clone, Debug)]
pub struct Settings {
    pub socket: PathBuf,
    pub server: String,
    pub agents: Vec<AgentSpec>,
    /// Agent seats played at once.
    pub capacity: u32,
    /// Workspaces and durable archives.
    pub dir: PathBuf,
}

type Key = (String, u8);

struct Playing {
    stop: CancellationToken,
    task: JoinHandle<()>,
}

/// Runs until the process is stopped, reconnecting to a server that goes away; the
/// agents playing meanwhile go on.
pub async fn run(settings: Settings) -> Result<(), String> {
    let stop = CancellationToken::new();
    let signal = stop.clone();
    let handler = tokio::spawn(async move {
        let _ = tokio::signal::ctrl_c().await;
        signal.cancel();
    });
    let result = run_until(settings, stop).await;
    handler.abort();
    result
}

pub async fn run_until(settings: Settings, stop: CancellationToken) -> Result<(), String> {
    let logs = settings.dir.join("archives");
    std::fs::create_dir_all(&logs).map_err(|e| format!("{}: {e}", logs.display()))?;
    let hello = FromRunner::Hello {
        agents: (settings.agents.iter())
            .map(|a| Offer {
                name: a.name.clone(),
                about: a.about(),
            })
            .collect(),
        capacity: settings.capacity,
    };
    let mut hello = serde_json::to_string(&hello).expect("serializable");
    hello.push('\n');
    let mut playing: HashMap<Key, Playing> = HashMap::new();
    let mut said_unreachable = false;
    let connected = async {
        loop {
            let stream = match UnixStream::connect(&settings.socket).await {
                Ok(stream) => stream,
                Err(e) => {
                    if !said_unreachable {
                        eprintln!(
                            "dominion agents: {}: {e}; retrying",
                            settings.socket.display()
                        );
                        said_unreachable = true;
                    }
                    tokio::time::sleep(RECONNECT).await;
                    continue;
                }
            };
            said_unreachable = false;
            eprintln!(
                "dominion agents: connected to {}",
                settings.socket.display()
            );
            let (read, mut write) = stream.into_split();
            if write.write_all(hello.as_bytes()).await.is_err() {
                tokio::time::sleep(RECONNECT).await;
                continue;
            }
            let mut lines = BufReader::new(read).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                match serde_json::from_str::<ToRunner>(&line) {
                    Ok(ToRunner::Jobs { jobs }) => reconcile(&settings, &mut playing, jobs, &logs),
                    Ok(ToRunner::Refused { agent, reason }) => {
                        eprintln!("dominion agents: the server refused {agent}: {reason}");
                    }
                    Err(_) => eprintln!("dominion agents: the server said {line}"),
                }
            }
            eprintln!("dominion agents: lost the server; reconnecting");
            tokio::time::sleep(RECONNECT).await;
        }
    };
    tokio::select! {
        _ = stop.cancelled() => {},
        () = connected => {},
    }
    for p in playing.values() {
        p.stop.cancel();
    }
    for (_, p) in playing {
        let _ = p.task.await;
    }
    Ok(())
}

fn reconcile(
    settings: &Settings,
    playing: &mut HashMap<Key, Playing>,
    jobs: Vec<Job>,
    logs: &Path,
) {
    let wanted: HashMap<Key, Job> = (jobs.into_iter())
        .map(|j| ((j.game_id.clone(), j.seat), j))
        .collect();
    playing.retain(|key, p| {
        let keep = wanted.contains_key(key);
        if !keep {
            p.stop.cancel();
        }
        keep || !p.task.is_finished()
    });
    for (key, job) in wanted {
        if playing.contains_key(&key) {
            continue;
        }
        let Some(spec) = settings.agents.iter().find(|a| a.name == job.agent) else {
            eprintln!(
                "dominion agents: no agent {} here for game {}",
                job.agent, job.game_id
            );
            continue;
        };
        eprintln!(
            "dominion agents: {} plays seat {} of game {}",
            spec.name, job.seat, job.game_id
        );
        let stop = CancellationToken::new();
        let log = logs.to_path_buf();
        let task = tokio::spawn(keep_playing(
            spec.clone(),
            job,
            settings.server.clone(),
            log,
            stop.clone(),
        ));
        playing.insert(key, Playing { stop, task });
    }
}

async fn keep_playing(
    spec: AgentSpec,
    job: Job,
    server: String,
    logs: PathBuf,
    stop: CancellationToken,
) {
    let mut retry = RECONNECT;
    let mut failures = 0;
    while !stop.is_cancelled() {
        let result = async {
            let mut archive = crate::archive::Archive::create(&logs)?;
            eprintln!(
                "dominion agents: {} in {}: archive {}",
                spec.name,
                job.game_id,
                archive.dir.display()
            );
            let cancelled = CancellationToken::new();
            let play = crate::harness::play(&spec, &job, &server, &mut archive, cancelled.clone());
            tokio::pin!(play);
            tokio::select! {
                result = &mut play => result,
                _ = stop.cancelled() => match tokio::time::timeout(GRACE, &mut play).await {
                    Ok(result) => result,
                    Err(_) => { cancelled.cancel(); play.await }
                }
            }
        }
        .await;
        let Err(error) = result else { return };
        if stop.is_cancelled() {
            return;
        }
        failures += 1;
        if failures >= 3
            || error.contains("agent finished without submitting a move")
            || error.contains("native harness attempted a forbidden tool")
            || error.contains("native harness exposed forbidden tools")
        {
            eprintln!(
                "dominion agents: {} in {}: {error}; stopped",
                spec.name, job.game_id
            );
            return;
        }
        eprintln!(
            "dominion agents: {} in {}: {error}; retrying in {} seconds",
            spec.name,
            job.game_id,
            retry.as_secs()
        );
        tokio::select! {
            _ = stop.cancelled() => return,
            () = tokio::time::sleep(retry) => {},
        }
        retry = (retry * 2).min(Duration::from_secs(60));
    }
}
