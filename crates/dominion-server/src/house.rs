//! The server's own agents, as seen from both ends of the `agent_socket`: a runner
//! (`dominion agents`, see [`crate::runner`]) connects, says which agents it offers, and
//! is told the seats they hold in running games; it keeps one agent playing each. One
//! JSON message a line. Whoever may open the socket is trusted to run agents: its file
//! mode is the only access control.

use crate::arena::Arena;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Offer {
    /// The agent's account name.
    pub name: String,
    /// What plays it, for the lobby.
    pub about: String,
    /// Its model and effort, which the lobby picks agents by.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<OfferedModel>,
    /// Its account is made when it is first seated, not when it is offered.
    #[serde(default)]
    pub on_demand: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OfferedModel {
    /// What runs it ("Claude Code").
    pub program: String,
    /// The provider's id for the model.
    pub id: String,
    /// The model's name ("Opus 5.5").
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum FromRunner {
    /// The first line: the agents it offers, and how many seats it plays at once.
    Hello { agents: Vec<Offer>, capacity: u32 },
    /// The agents it offers now, in place of those it offered before.
    Offers { agents: Vec<Offer> },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ToRunner {
    /// Every seat its agents hold in a running game, in place of the last list.
    Jobs { jobs: Vec<Job> },
    /// An agent it offered that the server won't seat.
    Refused { agent: String, reason: String },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Job {
    #[serde(default)]
    pub official: bool,
    /// The official account an official job plays as; its MCP endpoint is
    /// `/mcp/online/<id>`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub official_account: Option<String>,
    #[serde(default)]
    pub transcript: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub configuration: Option<dominion_engine::GameConfig>,
    pub agent: String,
    pub game_id: String,
    pub seat: u8,
    /// The seat's name in the game.
    pub name: String,
    pub token: String,
}

/// Accepts runners on `path` until `stop`: owner and group may connect.
pub async fn listen(
    arena: Arc<Arena>,
    path: &Path,
    stop: CancellationToken,
) -> std::io::Result<()> {
    if std::fs::symlink_metadata(path).is_ok() {
        std::fs::remove_file(path)?;
    }
    let listener = UnixListener::bind(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o660))?;
    }
    loop {
        let stream = tokio::select! {
            _ = stop.cancelled() => return Ok(()),
            accepted = listener.accept() => match accepted {
                Ok((stream, _)) => stream,
                Err(e) => {
                    eprintln!("dominion: agent socket: {e}");
                    continue;
                }
            },
        };
        tokio::spawn(serve_runner(arena.clone(), stream, stop.clone()));
    }
}

async fn serve_runner(arena: Arc<Arena>, stream: UnixStream, stop: CancellationToken) {
    let (read, mut write) = stream.into_split();
    let mut lines = BufReader::new(read).lines();
    let hello = match lines.next_line().await {
        Ok(Some(line)) => serde_json::from_str::<FromRunner>(&line),
        _ => return,
    };
    let Ok(FromRunner::Hello { agents, capacity }) = hello else {
        let _ = write
            .write_all(b"{\"error\":\"the first line is a hello\"}\n")
            .await;
        return;
    };
    let names: Vec<&str> = agents.iter().map(|a| a.name.as_str()).collect();
    eprintln!(
        "dominion: an agent runner connected: {} ({capacity} at once)",
        names.join(", ")
    );
    let (send, mut told) = mpsc::unbounded_channel();
    let arena2 = arena.clone();
    let id = tokio::task::spawn_blocking(move || arena2.runner_connected(agents, capacity, send))
        .await
        .expect("registering a runner");
    loop {
        tokio::select! {
            _ = stop.cancelled() => break,
            message = told.recv() => {
                // None: a newer runner took its place.
                let Some(message) = message else { break };
                let mut line = serde_json::to_string(&message).expect("serializable");
                line.push('\n');
                if write.write_all(line.as_bytes()).await.is_err() {
                    break;
                }
            }
            line = lines.next_line() => match line {
                Ok(Some(line)) => {
                    if let Ok(FromRunner::Offers { agents }) = serde_json::from_str(&line) {
                        let arena = arena.clone();
                        let _ = tokio::task::spawn_blocking(move || arena.offers_changed(id, agents))
                            .await;
                    }
                }
                _ => break,
            },
        }
    }
    eprintln!("dominion: the agent runner went away");
    let _ = tokio::task::spawn_blocking(move || arena.runner_gone(id)).await;
}
