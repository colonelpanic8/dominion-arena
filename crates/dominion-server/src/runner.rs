//! `dominion agents`: plays the server's own agents on this machine, with the coding
//! agents installed here and their logins (Claude Code, Codex). It connects to the
//! server's `agent_socket` ([`crate::house`]), offers its agents, and keeps one process
//! playing each seat they hold in a running game: started when the seat appears, started
//! again (a few times) if it stops while the game goes on, stopped once the game is over.
//! A process connects to the server's MCP endpoint and joins its seat with the seat's
//! token, which it is given on its standard input.

use crate::house::{FromRunner, Job, Offer, ToRunner};
use serde::Deserialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, SystemTime};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

/// Starts of one seat's process after the first, before it is left to forfeit.
const RESTARTS: u32 = 4;
const RESTART_AFTER: Duration = Duration::from_secs(15);
/// How long a process whose game is over may take to finish by itself.
const GRACE: Duration = Duration::from_secs(30);
const RECONNECT: Duration = Duration::from_secs(5);
/// Logs older than this are deleted when the runner starts.
const KEEP_LOGS: Duration = Duration::from_secs(14 * 86_400);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Program {
    /// Claude Code (`claude -p`).
    Claude,
    /// Codex (`codex exec`).
    Codex,
}

/// One agent the runner offers: an `[[agents]]` entry of the settings file.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentSpec {
    /// Its account's name on the server.
    pub name: String,
    pub program: Program,
    /// The model to ask for (default: the program's own default).
    pub model: Option<String>,
    /// The program to run (default `claude` or `codex` on the PATH).
    pub command: Option<String>,
    /// What the lobby says plays it (default from `program` and `model`).
    pub about: Option<String>,
}

impl AgentSpec {
    fn about(&self) -> String {
        if let Some(about) = &self.about {
            return about.clone();
        }
        let program = match self.program {
            Program::Claude => "Claude Code",
            Program::Codex => "Codex",
        };
        match &self.model {
            Some(model) => format!("{program} ({model})"),
            None => program.to_string(),
        }
    }
}

/// The agents offered when the settings name none.
pub fn default_agents() -> Vec<AgentSpec> {
    let agent = |name: &str, program, model: Option<&str>| AgentSpec {
        name: name.into(),
        program,
        model: model.map(String::from),
        command: None,
        about: None,
    };
    vec![
        agent("Claude Opus", Program::Claude, Some("opus")),
        agent("Claude Sonnet", Program::Claude, Some("sonnet")),
        agent("Codex", Program::Codex, None),
    ]
}

#[derive(Clone, Debug)]
pub struct Settings {
    /// The server's `agent_socket`.
    pub socket: PathBuf,
    /// The MCP endpoint the agents play through.
    pub server: String,
    pub agents: Vec<AgentSpec>,
    /// Games played at once.
    pub capacity: u32,
    /// Where the agents run and their logs go.
    pub dir: PathBuf,
}

const INSTRUCTIONS: &str = "\
You play one seat of a game of Dominion on a Dominion server, through the tools of its \
`dominion` MCP server. You have no other tools, and nobody reads or answers anything you \
write outside them. Play to win, by the rules, until the game is over.

- Start with `join_game`, with the game id and seat token below. Then loop: call \
`wait_for_decision` until your view has a decision (call it again when it comes back \
without one), read the view, and `answer` the decision with option indices. \
`play_treasures` plays all your Treasures in a Buy phase in one call; `card` gives a \
card's text by name.
- The game is timed: a seat that takes too long over a decision, or stops calling, \
forfeits. Think briefly, and don't stop between decisions.
- When a call says the game is over, or that the token is no longer valid, you are done: \
stop there.
- Never resign.";

/// The tools an agent plays with; it is refused the others (no resigning, no games of
/// its own).
const PLAYING_TOOLS: [&str; 6] = [
    "join_game",
    "view",
    "wait_for_decision",
    "answer",
    "play_treasures",
    "card",
];

fn job_text(job: &Job) -> String {
    format!(
        "Game {}, seat {} ({}), seat token {}. Join it and play.",
        job.game_id, job.seat, job.name, job.token
    )
}

/// What runs to play a seat.
#[derive(Clone, Debug)]
pub struct Launch {
    pub program: String,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
    /// Its standard input: the instructions and the seat.
    pub input: String,
}

/// How long one tool call may take: a `wait_for_decision` may wait up to the server's
/// longest wait.
const TOOL_TIMEOUT: Duration = Duration::from_secs(crate::mcp::MAX_WAIT.as_secs() + 60);

/// What runs to play `job`.
pub fn command(spec: &AgentSpec, job: &Job, server: &str, work: &Path) -> Launch {
    let mut args: Vec<String> = vec![];
    let mut arg = |a: &str| args.push(a.to_string());
    match spec.program {
        Program::Claude => {
            let mcp = serde_json::json!({
                "mcpServers": { "dominion": { "type": "http", "url": server } }
            });
            // Nothing of the machine's own setup: no settings, CLAUDE.md, hooks, plugins
            // or other MCP servers.
            for a in ["-p", "--no-chrome", "--no-session-persistence"] {
                arg(a);
            }
            arg("--setting-sources");
            arg("");
            if let Some(model) = &spec.model {
                arg("--model");
                arg(model);
            }
            arg("--strict-mcp-config");
            arg("--mcp-config");
            arg(&mcp.to_string());
            arg("--tools");
            arg("");
            arg("--allowedTools");
            arg(&PLAYING_TOOLS
                .map(|t| format!("mcp__dominion__{t}"))
                .join(","));
            arg("--permission-prompts");
            arg("none");
            arg("--system-prompt");
            arg(INSTRUCTIONS);
            arg("--output-format");
            arg("stream-json");
            arg("--verbose");
            Launch {
                program: spec.command.clone().unwrap_or_else(|| "claude".into()),
                args,
                env: vec![(
                    "MCP_TOOL_TIMEOUT".into(),
                    TOOL_TIMEOUT.as_millis().to_string(),
                )],
                input: job_text(job),
            }
        }
        Program::Codex => {
            for a in [
                "exec",
                "--ephemeral",
                "--skip-git-repo-check",
                "--ignore-user-config",
            ] {
                arg(a);
            }
            arg("-C");
            arg(&work.display().to_string());
            arg("--sandbox");
            arg("read-only");
            arg("-c");
            arg("approval_policy=\"never\"");
            arg("-c");
            arg(&format!("mcp_servers.dominion.url={}", toml_string(server)));
            arg("-c");
            arg(&format!(
                "mcp_servers.dominion.tool_timeout_sec={}",
                TOOL_TIMEOUT.as_secs()
            ));
            arg("-c");
            arg("mcp_servers.dominion.default_tools_approval_mode=\"approve\"");
            arg("-c");
            let tools: Vec<String> = PLAYING_TOOLS.iter().map(|t| toml_string(t)).collect();
            arg(&format!(
                "mcp_servers.dominion.enabled_tools=[{}]",
                tools.join(",")
            ));
            if let Some(model) = &spec.model {
                arg("--model");
                arg(model);
            }
            arg("--json");
            arg("-");
            Launch {
                program: spec.command.clone().unwrap_or_else(|| "codex".into()),
                args,
                env: vec![],
                input: format!("{INSTRUCTIONS}\n\n{}", job_text(job)),
            }
        }
    }
}

fn toml_string(s: &str) -> String {
    toml::Value::String(s.to_string()).to_string()
}

type Key = (String, u8);

struct Playing {
    stop: CancellationToken,
    task: JoinHandle<()>,
}

/// Runs until the process is stopped, reconnecting to a server that goes away; the
/// agents playing meanwhile go on.
pub async fn run(settings: Settings) -> Result<(), String> {
    let logs = settings.dir.join("logs");
    let work = settings.dir.join("work");
    for dir in [&logs, &work] {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    prune_logs(&logs);
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
                Ok(ToRunner::Jobs { jobs }) => {
                    reconcile(&settings, &mut playing, jobs, &logs, &work)
                }
                Ok(ToRunner::Refused { agent, reason }) => {
                    eprintln!("dominion agents: the server refused {agent}: {reason}");
                }
                Err(_) => eprintln!("dominion agents: the server said {line}"),
            }
        }
        eprintln!("dominion agents: lost the server; reconnecting");
        tokio::time::sleep(RECONNECT).await;
    }
}

fn reconcile(
    settings: &Settings,
    playing: &mut HashMap<Key, Playing>,
    jobs: Vec<Job>,
    logs: &Path,
    work: &Path,
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
        let log = logs.join(format!("{}-{}.log", job.game_id, job.seat));
        let task = tokio::spawn(keep_playing(
            spec.clone(),
            job,
            settings.server.clone(),
            work.to_path_buf(),
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
    work: PathBuf,
    log: PathBuf,
    stop: CancellationToken,
) {
    let launch = command(&spec, &job, &server, &work);
    for start in 0..=RESTARTS {
        let file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log);
        let (out, err) = match file.and_then(|f| Ok((f.try_clone()?, f))) {
            Ok((out, err)) => (Stdio::from(out), Stdio::from(err)),
            Err(e) => {
                eprintln!("dominion agents: {}: {e}", log.display());
                (Stdio::null(), Stdio::null())
            }
        };
        let child = tokio::process::Command::new(&launch.program)
            .args(&launch.args)
            .envs(launch.env.iter().cloned())
            .current_dir(&work)
            .stdin(Stdio::piped())
            .stdout(out)
            .stderr(err)
            .kill_on_drop(true)
            .spawn();
        let mut child = match child {
            Ok(child) => child,
            Err(e) => {
                eprintln!("dominion agents: starting {}: {e}", launch.program);
                return;
            }
        };
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(launch.input.as_bytes()).await;
        }
        tokio::select! {
            status = child.wait() => {
                let status = status.map_or_else(|e| e.to_string(), |s| s.to_string());
                eprintln!(
                    "dominion agents: {} in game {} stopped ({status})",
                    spec.name, job.game_id
                );
            }
            _ = stop.cancelled() => {
                if tokio::time::timeout(GRACE, child.wait()).await.is_err() {
                    let _ = child.kill().await;
                }
                return;
            }
        }
        if start == RESTARTS {
            break;
        }
        tokio::select! {
            _ = stop.cancelled() => return,
            _ = tokio::time::sleep(RESTART_AFTER) => {}
        }
    }
    eprintln!(
        "dominion agents: {} gave up on game {} after {} starts",
        spec.name,
        job.game_id,
        RESTARTS + 1
    );
}

fn prune_logs(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let old = (entry.metadata().and_then(|m| m.modified()).ok())
            .and_then(|t| SystemTime::now().duration_since(t).ok())
            .is_some_and(|age| age > KEEP_LOGS);
        if old {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn job() -> Job {
        Job {
            agent: "Claude Opus".into(),
            game_id: "g7".into(),
            seat: 1,
            name: "Claude Opus".into(),
            token: "secret".into(),
        }
    }

    #[test]
    fn the_token_goes_on_standard_input_only() {
        for spec in default_agents() {
            let launch = command(&spec, &job(), "http://127.0.0.1:1/mcp", Path::new("/w"));
            assert!(
                launch.args.iter().all(|a| !a.contains("secret")),
                "{launch:?}"
            );
            assert!(launch.input.contains("Game g7, seat 1"));
            assert!(launch.input.contains("seat token secret"));
        }
    }

    #[test]
    fn claude_gets_only_the_server_and_its_tools() {
        let spec = &default_agents()[0];
        let Launch { program, args, .. } =
            command(spec, &job(), "http://127.0.0.1:1/mcp", Path::new("/w"));
        assert_eq!(program, "claude");
        let after = |flag: &str| {
            let i = args.iter().position(|a| a == flag).unwrap();
            args[i + 1].clone()
        };
        assert_eq!(after("--model"), "opus");
        assert_eq!(after("--tools"), "");
        assert!(after("--allowedTools").contains("mcp__dominion__answer"));
        assert!(!after("--allowedTools").contains("resign"));
        assert_eq!(after("--setting-sources"), "");
        let mcp: serde_json::Value = serde_json::from_str(&after("--mcp-config")).unwrap();
        assert_eq!(
            mcp["mcpServers"]["dominion"]["url"],
            "http://127.0.0.1:1/mcp"
        );
        assert!(args.contains(&"--strict-mcp-config".to_string()));
    }

    #[test]
    fn codex_gets_the_server_as_a_config_override() {
        let spec = &default_agents()[2];
        let Launch {
            program,
            args,
            input,
            ..
        } = command(spec, &job(), "http://h/mcp", Path::new("/w"));
        assert_eq!(program, "codex");
        assert!(args.contains(&"mcp_servers.dominion.url=\"http://h/mcp\"".to_string()));
        let enabled = args.iter().find(|a| a.contains("enabled_tools")).unwrap();
        assert!(enabled.contains("\"answer\"") && !enabled.contains("resign"));
        assert_eq!(args.last().map(String::as_str), Some("-"));
        assert!(input.starts_with(INSTRUCTIONS));
    }
}
