//! Native harnesses have no game credentials or host tools. This bridge alone
//! validates and executes their structured game requests.

mod antigravity;
mod bridge;
mod claude;
mod codex;
mod process;
pub mod sandbox;

pub use bridge::TOOLS as GAME_TOOLS;
pub use process::write_private;

use crate::archive::{Archive, call};
use crate::client::Client;
use crate::house::Job;
use crate::runner::AgentSpec;
use serde_json::json;
use tokio_util::sync::CancellationToken;

const INSTRUCTIONS: &str = "You play one assigned seat of Dominion. Play to win using only the dominion MCP tools. Follow dominion/play@1: read your observation, then call step with decision and indices (ordered: top first) or name only when nameAny is true. Give expect with each chosen option's label or card, in index order. step answers and waits for your next decision. When there is no decision for you (WAITING), call wait and continue. Results are text: what changed since your last decision, the text of cards new to you, then your next decision. Finish only at GAME OVER. The initial observation includes the full view and official card reference; use cards with names for more text/current costs or log to reread log entries by seq. state shows the whole game again when needed. Do not refresh after a tool already returned the current view. play_treasures plays every Treasure offered in one call. Keep playing this seat for the entire game in this conversation. Never print a JSON action in place of calling a tool. The server supplies your secret seat credentials and binds the bridge to this seat; do not override them or create another game. When a tool rejects a request, correct it using the returned error and open decision.";
const NO_PLAYBOOK: &str =
    "Do not inspect files, run commands, delegate, resign, or ask a person for help.";
const PLAYBOOK: &str = "Your working directory is the Dominion playbook, read-only. Before your first decision, do what skills/play-game/SKILL.md says under \"Make the plan\": look up the ratings of every card on the board with its command, run its kingdom analysis, and write the plan out in your reply; then play by its skills. Read its files and run its lookup commands as it says, and run nothing else. Do not delegate, resign, or ask a person for help.";

/// What a native seat runs with beyond its profile: the playbook it starts in, the tools
/// on its PATH and the hosts it may reach.
#[derive(Clone, Debug, Default)]
pub struct Sandbox {
    /// A dominion-playbook checkout. Seats start in it, read-only, and load it the way
    /// their CLI loads a project; they may read it and run its lookup commands.
    pub playbook: Option<std::path::PathBuf>,
    /// Directories put first on PATH inside the sandbox (a shell, python3, coreutils).
    pub path: Vec<std::path::PathBuf>,
    /// Hosts the CLIs may reach beyond their programs' defaults (`*.example.com` too).
    pub allow_hosts: Vec<String>,
    /// The `dominion` binary run inside the sandbox to forward its ports (default: this
    /// executable).
    pub forwarder: Option<std::path::PathBuf>,
}

/// The game brief: the seat's system prompt.
pub fn instructions(playbook: bool) -> String {
    format!(
        "{INSTRUCTIONS} {}",
        if playbook { PLAYBOOK } else { NO_PLAYBOOK }
    )
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
    let endpoint = endpoint(server, job)?;
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
    let result = async {
        let mut harness = process::Harness::new(spec, sandbox, &archive.dir)?;
        tokio::select! {
            biased;
            _ = stop.cancelled() => Err("seat runner cancelled".into()),
            result = play_inner(&mut harness, &client, spec, job, server, archive) => result,
        }
    }
    .await;
    record_final_result(&client, archive, job).await?;
    archive.record("seat_finished", json!({"result":result}))?;
    client.close().await;
    archive.finish_publish().await;
    result
}

async fn play_inner(
    harness: &mut process::Harness,
    client: &Client,
    spec: &AgentSpec,
    job: &Job,
    server: &str,
    archive: &mut Archive,
) -> Result<(), String> {
    let view = call(
        client,
        archive,
        "dominion",
        if job.official { "state" } else { "join_game" },
        if job.official {
            json!({})
        } else {
            json!({"game_id":job.game_id,"token":job.token})
        },
    )
    .await?;
    if !view.ok {
        return Err(view.text);
    }
    let mut bridge = bridge::Bridge::start(server, job, Some(&harness.bridge_socket())).await?;
    harness.enable_mcp(
        spec,
        &bridge.url,
        bridge.records.take().expect("new bridge"),
    )?;
    let state = view.json.as_ref().ok_or("Dominion returned no game view")?;
    if game_state(state)["game"]["running"] == false {
        return Ok(());
    }
    let message = format!("Initial view and card reference:\n{}", view.text);
    harness.reply(spec, &message, archive).await?;
    session_end(client, archive, job).await
}

/// Whether the game is over once the CLI has finished: shown by a reply during play, else
/// by a fresh view. An official seat's access ends with its game, so a reply that showed
/// it settles it without asking.
async fn session_end(client: &Client, archive: &mut Archive, job: &Job) -> Result<(), String> {
    if archive.has_result() {
        return Ok(());
    }
    let view = call(
        client,
        archive,
        "dominion",
        if job.official { "state" } else { "view" },
        if job.official {
            json!({})
        } else {
            json!({"token":job.token})
        },
    )
    .await?;
    if !view.ok {
        return Err(view.text);
    }
    if view
        .json
        .as_ref()
        .is_some_and(|v| game_state(v)["game"]["running"] == false)
    {
        Ok(())
    } else {
        Err("agent session ended before the game finished".into())
    }
}

/// Asks for the finished game when no reply during play showed it (the archive records
/// the `result` from the reply). An official seat's access ends with its game, so a
/// refusal there is no transport error.
pub async fn record_final_result(
    client: &Client,
    archive: &mut Archive,
    job: &Job,
) -> Result<(), String> {
    if archive.has_result() {
        return Ok(());
    }
    if !job.official {
        let _ = call(
            client,
            archive,
            "dominion",
            "view",
            json!({"token":job.token}),
        )
        .await;
        return Ok(());
    }
    let reply = tokio::time::timeout(
        std::time::Duration::from_secs(90),
        client.call("state", json!({})),
    )
    .await;
    match reply {
        Ok(Ok(reply)) => archive.record(
            "response",
            json!({"source":"dominion", "tool":"state", "reply":reply}),
        ),
        Ok(Err(e)) => archive.record("final_view_unavailable", json!({"error":e.to_string()})),
        Err(_) => archive.record(
            "final_view_unavailable",
            json!({"error":"state: no reply within 90 seconds"}),
        ),
    }
}

pub fn endpoint(server: &str, job: &Job) -> Result<String, String> {
    if !job.official {
        return Ok(server.into());
    }
    let root = server
        .trim_end_matches('/')
        .strip_suffix("/mcp")
        .ok_or("agent server must end in /mcp")?;
    Ok(format!("{root}/mcp/online"))
}
fn game_state(value: &serde_json::Value) -> &serde_json::Value {
    value.get("view").unwrap_or(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::response::IntoResponse as _;
    use rmcp::transport::streamable_http_server::{
        StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
    };
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    #[derive(Clone)]
    struct Server;
    impl rmcp::ServerHandler for Server {}

    /// A client of an MCP server that answers 401, as dominion.games' seat endpoint does
    /// once the game is over, after `revoked` is set.
    async fn revocable() -> (Client, Arc<AtomicBool>) {
        let revoked = Arc::new(AtomicBool::new(false));
        let service = StreamableHttpService::new(
            || Ok(Server),
            Arc::new(LocalSessionManager::default()),
            StreamableHttpServerConfig::default(),
        );
        let gate = revoked.clone();
        let router =
            axum::Router::new()
                .nest_service("/mcp", service)
                .layer(axum::middleware::from_fn(
                    move |request: axum::extract::Request, next: axum::middleware::Next| {
                        let revoked = gate.load(Ordering::SeqCst);
                        async move {
                            if revoked {
                                axum::http::StatusCode::UNAUTHORIZED.into_response()
                            } else {
                                next.run(request).await
                            }
                        }
                    },
                ));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/mcp", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, router).await });
        (Client::connect(&url).await.unwrap(), revoked)
    }

    fn kinds(archive: &Archive) -> Vec<String> {
        std::fs::read_to_string(archive.dir.join("interactions.jsonl"))
            .unwrap()
            .lines()
            .map(|l| serde_json::from_str::<serde_json::Value>(l).unwrap()["kind"].to_string())
            .collect()
    }

    #[tokio::test]
    async fn a_finished_official_seat_ends_well_once_its_access_is_revoked() {
        let (client, revoked) = revocable().await;
        revoked.store(true, Ordering::SeqCst);
        let job = Job {
            official: true,
            transcript: false,
            configuration: None,
            agent: "a".into(),
            game_id: "dominion.games:1".into(),
            seat: 0,
            name: "a".into(),
            token: "t".into(),
        };
        let dir = tempfile::tempdir().unwrap();
        let mut archive = Archive::open(&dir.path().join("over")).unwrap();
        archive
            .record(
                "response",
                json!({"source":"dominion", "tool":"step", "reply":{"ok":true, "text":"GAME OVER",
                    "json":{"view":{"game":{"running":false}}, "glossary":{}}}}),
            )
            .unwrap();
        assert_eq!(session_end(&client, &mut archive, &job).await, Ok(()));
        record_final_result(&client, &mut archive, &job)
            .await
            .unwrap();
        assert_eq!(kinds(&archive), ["\"response\"", "\"result\""]);

        // No reply showed the game over: the seat can't tell, and says so without a
        // transport error.
        let mut archive = Archive::open(&dir.path().join("unknown")).unwrap();
        record_final_result(&client, &mut archive, &job)
            .await
            .unwrap();
        assert_eq!(kinds(&archive), ["\"final_view_unavailable\""]);
        assert!(session_end(&client, &mut archive, &job).await.is_err());
        client.close().await;
    }
}
