//! Native harnesses have no game credentials or host tools. This bridge alone
//! validates and executes their structured game requests.

mod antigravity;
pub(crate) mod bridge;
mod claude;
mod codex;
pub(crate) mod process;
pub mod sandbox;

pub use bridge::TOOLS as GAME_TOOLS;
pub use process::write_private;

use crate::archive::{Archive, call};
use crate::client::Client;
use crate::house::Job;

use serde_json::json;

const INSTRUCTIONS: &str = "You play one assigned seat of Dominion. Play to win using only the dominion MCP tools. Follow dominion/play@1: read your observation, then call step with decision and indices (ordered: top first) or name only when nameAny is true. Give expect with each chosen option's label or card, in index order. step answers and waits for your next decision. When there is no decision for you (WAITING), call wait and continue. Results are text: what changed since your last decision, the text of cards new to you, then your next decision. Finish only at GAME OVER. The initial observation includes the full view and official card reference; use cards with names for more text/current costs or log to reread log entries by seq. state shows the whole game again when needed. Do not refresh after a tool already returned the current view. play_treasures plays every Treasure offered in one call. Keep playing this seat for the entire game in this conversation. Never print a JSON action in place of calling a tool. The server supplies your secret seat credentials and binds the bridge to this seat; do not override them or create another game. When a tool rejects a request, correct it using the returned error and open decision.";
const NO_PLAYBOOK: &str =
    "Do not inspect files, run commands, delegate, resign, or ask a person for help.";
const CLOCKED_NO_PLAYBOOK: &str = "This game has an active clock. Make timely legal moves and answer immediately when a CLOCK PRESSURE or LOW TIME cue appears. Do not inspect files, run commands, delegate, resign, or ask a person for help.";
const PLAYBOOK: &str = "Your working directory is the read-only Dominion playbook. Use the full opening analysis in skills/play-game/SKILL.md, including on a timed table. A BRIEF from the game's creator supplies the plan. Read its files and run its lookup commands as needed. Do not delegate, resign, or ask a person for help.";
pub(crate) const CLOCKED_OPENING: &str = "Your clock may already be running. Use the supplied opening budget and any pre-fetched card ratings. Answer LOW TIME immediately. A BRIEF from the game's creator supplies the plan.";

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

/// The playbook at `dir` as a seat is given it: its commit, where git can tell, and a
/// digest of what is in it.
pub fn playbook(dir: &std::path::Path) -> Result<dominion_protocol::lobby::Playbook, String> {
    use sha2::{Digest, Sha256};
    fn walk(
        root: &std::path::Path,
        dir: &std::path::Path,
        files: &mut Vec<std::path::PathBuf>,
    ) -> std::io::Result<()> {
        for entry in std::fs::read_dir(dir)? {
            let path = entry?.path();
            if path.file_name().is_some_and(|n| n == ".git") {
                continue;
            }
            let meta = std::fs::metadata(&path)?;
            if meta.is_dir() {
                walk(root, &path, files)?;
            } else if meta.is_file() {
                files.push(path.strip_prefix(root).unwrap_or(&path).to_path_buf());
            }
        }
        Ok(())
    }
    let failed = |e: std::io::Error| format!("reading the playbook {}: {e}", dir.display());
    let mut files = vec![];
    walk(dir, dir, &mut files).map_err(failed)?;
    files.sort();
    let mut hash = Sha256::new();
    for file in &files {
        let contents = std::fs::read(dir.join(file)).map_err(failed)?;
        hash.update(file.to_string_lossy().as_bytes());
        hash.update([0]);
        hash.update((contents.len() as u64).to_le_bytes());
        hash.update(&contents);
    }
    let git = |args: &[&str]| {
        let out = std::process::Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .output()
            .ok()?;
        out.status
            .success()
            .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
    };
    let commit = git(&["rev-parse", "HEAD"]).filter(|c| !c.is_empty());
    let dirty = commit.is_some()
        && git(&["status", "--porcelain", "--untracked-files=normal"])
            .is_some_and(|s| !s.is_empty());
    Ok(dominion_protocol::lobby::Playbook {
        path: dir.display().to_string(),
        commit,
        dirty,
        digest: format!("{:x}", hash.finalize()),
    })
}

/// The player brief for the opening selected by the runner strategy.
pub fn instructions(playbook: bool, clocked: bool) -> String {
    let guidance = match (playbook, clocked) {
        (true, true) => {
            return format!(
                "{INSTRUCTIONS} Your working directory is the read-only Dominion playbook. {CLOCKED_OPENING} Read its files and run its lookup commands as needed. Do not delegate, resign, or ask a person for help."
            );
        }
        (true, false) => PLAYBOOK,
        (false, true) => CLOCKED_NO_PLAYBOOK,
        (false, false) => NO_PLAYBOOK,
    };
    format!("{INSTRUCTIONS} {guidance}")
}

/// Supply the playbook's short table guidance without a model tool call on the clock.
pub(crate) fn append_live_guidance(
    text: &mut String,
    playbook: Option<&std::path::Path>,
) -> Result<(), String> {
    let Some(dir) = playbook else { return Ok(()) };
    let path = dir.join("skills/play-game/references/live-decisions.md");
    match std::fs::read_to_string(&path) {
        Ok(guide) => {
            text.push_str("\n\n");
            text.push_str(guide.trim());
            Ok(())
        }
        // Older and custom playbooks may only provide the general play-game skill.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("{}: {e}", path.display())),
    }
}

/// Whether a common play@1 observation indicates an active (or potentially active) clock.
/// Returns `false` for explicitly untimed games (`clockInfo.status == "off"` or absent
/// clock metadata with no `clock`).
pub(crate) fn is_clock_active(reply: &serde_json::Value) -> bool {
    match reply
        .get("clockInfo")
        .and_then(|v| v.get("status"))
        .and_then(|v| v.as_str())
    {
        Some("off") => false,
        Some("available" | "unavailable") => true,
        _ => reply.get("clock").is_some_and(|v| !v.is_null()),
    }
}

pub(crate) fn has_decision(reply: &serde_json::Value) -> bool {
    game_state(reply)
        .get("decision")
        .is_some_and(|v| !v.is_null())
}

pub(crate) fn is_running(reply: &serde_json::Value) -> bool {
    game_state(reply)["game"]["running"] != false
}

pub use crate::runner::provider::play;

/// Why a seat has no debrief when it joined a game already over.
pub const JOINED_OVER: &str = "the seat's agent started after the game was over";

/// What a seat's first message starts with: its brief, and the debrief it will owe.
/// `final_reply`: the debrief is the agent's last reply, not an answer to a later message.
pub fn opening(job: &Job, final_reply: bool) -> String {
    let mut text = String::new();
    if let Some(brief) = &job.brief {
        text.push_str(&format!(
            "BRIEF from the game's creator. It comes before your own planning: where it sets a plan, play that plan, and keep where you would have played differently for your debrief.\n{}\n\n",
            brief.trim()
        ));
    }
    if let Some(question) = &job.debrief {
        let when = if final_reply {
            "Once the game is over, your final reply is your debrief for the game's creator, in prose, answering"
        } else {
            "Once the game is over you will be asked for a debrief for the game's creator, answering"
        };
        text.push_str(&format!("DEBRIEF. {when}:\n{}\n\n", question.trim()));
    }
    text
}

/// The question a seat that plays reply by reply is asked once the game is over.
pub fn debrief_request(job: &Job) -> Option<String> {
    job.debrief.as_ref().map(|question| {
        format!(
            "The game is over. Reply now in prose (no tool call, no JSON) with your debrief for the game's creator, answering:\n{}",
            question.trim()
        )
    })
}

/// Records the seat's debrief, or why it has none, if its game asked for one: a server
/// agent's job lasts until it has.
pub async fn debrief(
    client: &Client,
    archive: &mut Archive,
    job: &Job,
    answer: Result<String, String>,
) {
    if job.official || job.debrief.is_none() {
        return;
    }
    let text = match answer {
        Ok(text) if !text.trim().is_empty() => text,
        Ok(_) => "No debrief: the agent's last reply was empty.".into(),
        Err(why) => format!("No debrief: {why}."),
    };
    let sent = call(
        client,
        archive,
        "dominion",
        "debrief",
        json!({"token": job.token, "text": text}),
    )
    .await;
    if let Err(e) = sent.and_then(|r| if r.ok { Ok(()) } else { Err(r.text) }) {
        let _ = archive.record("debrief_unsent", json!({"error": e}));
    }
}

/// Whether the game is over once the CLI has finished: shown by a reply during play, else
/// by a fresh view. An official seat's access ends with its game, so a reply that showed
/// it settles it without asking.
pub(crate) async fn session_end(
    client: &Client,
    archive: &mut Archive,
    job: &Job,
) -> Result<(), String> {
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
    match job.official_account.as_deref() {
        None => Ok(format!("{root}/mcp/online")),
        Some(id)
            if !id.is_empty()
                && (id.bytes())
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-') =>
        {
            Ok(format!("{root}/mcp/online/{id}"))
        }
        Some(id) => Err(format!("invalid official account id {id:?}")),
    }
}
pub(crate) fn game_state(value: &serde_json::Value) -> &serde_json::Value {
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
            options: Default::default(),
            official: true,
            official_account: None,
            transcript: false,
            configuration: None,
            agent: "a".into(),
            brief: None,
            debrief: None,
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

#[cfg(test)]
mod playbook_tests {
    #[test]
    fn a_playbooks_digest_follows_its_files_and_not_its_git_directory() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("skills")).unwrap();
        std::fs::write(dir.path().join("skills/a.md"), "plan").unwrap();
        let first = super::playbook(dir.path()).unwrap();
        std::fs::create_dir_all(dir.path().join(".git")).unwrap();
        std::fs::write(dir.path().join(".git/HEAD"), "ref").unwrap();
        assert_eq!(super::playbook(dir.path()).unwrap().digest, first.digest);
        std::fs::write(dir.path().join("skills/a.md"), "plan!").unwrap();
        assert_ne!(super::playbook(dir.path()).unwrap().digest, first.digest);
    }
}
