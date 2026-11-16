//! Native harnesses have no game credentials or host tools. This bridge alone
//! validates and executes their structured game requests.

mod antigravity;
mod bridge;
mod claude;
mod codex;
mod process;
pub mod sandbox;

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

/// The playbook as one text, for a CLI that can't start in it: its AGENTS.md, then every
/// context file and skill, which an agent started in the playbook reads as it needs them.
pub fn inline_playbook(dir: &std::path::Path) -> Result<String, String> {
    let read = |path: &std::path::Path| {
        std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))
    };
    let files = |sub: &str, file: Option<&str>| -> Result<Vec<String>, String> {
        let root = dir.join(sub);
        let mut names: Vec<String> = std::fs::read_dir(&root)
            .map_err(|e| format!("{}: {e}", root.display()))?
            .filter_map(|entry| entry.ok()?.file_name().into_string().ok())
            .map(|name| match file {
                Some(file) => format!("{sub}/{name}/{file}"),
                None => format!("{sub}/{name}"),
            })
            .filter(|path| path.ends_with(".md") && dir.join(path).is_file())
            .collect();
        names.sort();
        Ok(names)
    };
    let mut paths = vec!["AGENTS.md".to_string()];
    paths.extend(files("context", None)?);
    paths.extend(files("skills", Some("SKILL.md"))?);
    let mut text = vec![];
    for path in paths {
        text.push(format!("# {path}\n{}", read(&dir.join(&path))?));
    }
    Ok(text.join("\n\n"))
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
    let final_view = call(
        &client,
        archive,
        "dominion",
        if job.official { "state" } else { "view" },
        if job.official {
            json!({})
        } else {
            json!({"token":job.token})
        },
    )
    .await;
    if let Ok(view) = final_view
        && let Some(state) = view.json
        && game_state(&state)["game"]["running"] == false
    {
        archive.record("result", state)?;
    }
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

pub(super) fn endpoint(server: &str, job: &Job) -> Result<String, String> {
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
    #[test]
    fn the_inlined_playbook_is_agents_md_then_context_then_skills() {
        let dir = tempfile::tempdir().unwrap();
        for (path, text) in [
            ("AGENTS.md", "always"),
            ("context/strategy.md", "strategy"),
            ("context/combos.md", "combos"),
            ("skills/play-game/SKILL.md", "plan"),
            ("data/card-ratings.py", "print()"),
        ] {
            let path = dir.path().join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }
        let text = super::inline_playbook(dir.path()).unwrap();
        let headings: Vec<&str> = text.lines().filter(|l| l.starts_with("# ")).collect();
        assert_eq!(
            headings,
            [
                "# AGENTS.md",
                "# context/combos.md",
                "# context/strategy.md",
                "# skills/play-game/SKILL.md"
            ]
        );
    }
}
