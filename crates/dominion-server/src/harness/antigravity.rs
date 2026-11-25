use super::process::{Launch, write_json, write_private};
use crate::runner::AgentSpec;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

/// Where `agy` keeps its conversations under `GEMINI_HOME`, one database each: what counts
/// Antigravity's usage (T3 Code) reads them there.
const CONVERSATIONS: &str = "antigravity-cli/conversations";

/// The tools a seat started in the playbook may use beyond the game's: reading it and
/// running its lookups. Antigravity offers `manage_task` with `run_command`, for a command
/// still running.
pub const PLAYBOOK_TOOLS: [&str; 3] = ["read_file", "run_command", "manage_task"];

/// The playbook's lookup command, the only one a seat may run, as given in its AGENTS.md
/// and with the playbook's path.
const LOOKUPS: [&str; 2] = [
    "command(python3 data/card-ratings.py)",
    "command(python3 /playbook/data/card-ratings.py)",
];

/// The seat's agent (its system prompt and tools) and permissions. A playbook seat starts
/// in it and loads it as a workspace: AGENTS.md as a rule, `.agents/skills`. It may read
/// it and run its lookups; Antigravity refuses anything else outside the playbook (other
/// commands, reads elsewhere) and ends the turn.
pub fn settings(work: &Path, playbook: bool, brief: &str) -> Result<(), String> {
    write_agent(work, playbook, brief)?;
    permissions(work, playbook, false)
}

/// Writes the seat's Antigravity agent definition with `brief` as its system prompt.
pub fn write_agent(work: &Path, playbook: bool, brief: &str) -> Result<(), String> {
    let tools = if playbook {
        "[read_file, run_command, finish]"
    } else {
        "[finish]"
    };
    // Global config (`GEMINI_HOME/config`) holds the agent and MCP servers, wherever the
    // seat starts; `.agents/` serves a seat started in /work.
    let agent = format!(
        "---\nname: dominion\ndescription: Play a Dominion seat through structured replies\ntools: {tools}\nmainAgent: true\nsubagent: false\ncommandExecutionPolicy: off\nmcpServers: []\nskills: []\nplugins: []\n---\n{brief}\n",
    );
    for dir in [".agents/agents", "config/agents"] {
        let dir = work.join(dir);
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        write_private(&dir.join("dominion.md"), agent.as_bytes())?;
    }
    Ok(())
}

/// The game tools by the names Antigravity gives them. Its own `wait` only lets time pass,
/// so a seat told to "call wait" must know the game's is another tool.
pub fn tool_names() -> String {
    let names: Vec<String> = (super::bridge::TOOLS.iter())
        .map(|tool| format!("mcp_dominion_{tool}"))
        .collect();
    format!(
        " Your game tools are named {}: call them by these names. Your own wait tool does not wait for the game; mcp_dominion_wait does.",
        names.join(", ")
    )
}

/// Lets the seat use the bridge's game tools at `url`.
pub fn enable_mcp(work: &Path, url: &str, playbook: bool) -> Result<(), String> {
    write_json(
        &work.join("config/mcp_config.json"),
        &json!({"mcpServers":{"dominion":{"serverUrl":url,"disabled":false,"tools":super::bridge::TOOLS.iter().map(|name| (name.to_string(),json!({"eager":true}))).collect::<serde_json::Map<String,Value>>()}}}),
    )?;
    for path in [".agents/agents/dominion.md", "config/agents/dominion.md"] {
        let path = work.join(path);
        let text = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
        write_private(
            &path,
            text.replace("mcpServers: []", "mcpServers: []\ninheritMcp: true")
                .as_bytes(),
        )?;
    }
    permissions(work, playbook, true)
}

/// Deny rules beat allow rules, so a playbook seat's commands and reads aren't denied
/// outright: under `request-review`, print mode refuses whatever no allow rule covers.
/// (`strict` refuses allowed reads and commands too.)
fn permissions(work: &Path, playbook: bool, mcp: bool) -> Result<(), String> {
    let mut deny = vec!["write_file(*)", "read_url(*)", "execute_url(*)"];
    let mut allow = vec![];
    if playbook {
        allow.push("read_file(/playbook)");
        allow.extend(LOOKUPS);
    } else {
        deny.extend(["read_file(*)", "command(*)"]);
    }
    if mcp {
        allow.push("mcp(dominion/*)");
    } else {
        deny.push("mcp(*)");
    }
    write_json(
        &work.join("settings.json"),
        &json!({
            "permissions": {"deny": deny, "ask": [], "allow": allow},
            "toolPermission": "request-review"
        }),
    )
}

/// The seat's own folder for its conversations, named after its archive and kept after it
/// ends. One that can't be made is reported and the seat plays on without it.
pub fn conversations(spec: &AgentSpec, auth: &Path, archive: &Path) -> Option<PathBuf> {
    let base = (spec.conversations_dir.clone())
        .unwrap_or_else(|| auth.join(CONVERSATIONS).join("dominion-arena"));
    let dir = base.join(archive.file_name()?);
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
    builder
        .create(&dir)
        .inspect_err(|e| eprintln!("dominion agents: conversations in {}: {e}", dir.display()))
        .ok()?;
    Some(dir)
}

pub fn mounts(cmd: &mut tokio::process::Command, work: &Path, conversations: Option<&Path>) {
    let target = "/home/agent/.gemini";
    cmd.env("GEMINI_HOME", target)
        .env("AGY_ACP_FORCE_FILE_STORAGE", "1");
    cmd.arg("--bind")
        .arg(work.join("config"))
        .arg(format!("{target}/config"));
    if let Some(dir) = conversations {
        cmd.arg("--bind")
            .arg(dir)
            .arg(format!("{target}/{CONVERSATIONS}"));
    }
}

pub fn launch(spec: &AgentSpec) -> Launch {
    let mut launch = Launch::new(spec, "agy");
    launch.args = [
        "-p",
        "",
        "--settings",
        "/work/settings.json",
        "--input-format",
        "stream-json",
        "--output-format",
        "stream-json",
        "--disable-slash-commands",
        "--agent",
        "dominion",
    ]
    .map(String::from)
    .to_vec();
    if let Some(model) = &spec.model {
        launch.args.extend(["--model".into(), model.clone()]);
    }
    if let Some(effort) = &spec.effort {
        launch.args.extend(["--effort".into(), effort.clone()]);
    }
    launch
}

pub fn response(events: &[Value]) -> Result<String, String> {
    let result = events
        .iter()
        .rev()
        .find(|e| e["event"] == "result")
        .ok_or("Antigravity returned no result")?;
    if result["result"]["status"] != "SUCCESS" {
        return Err(format!("Antigravity failed: {result}"));
    }
    result["result"]["response"]
        .as_str()
        .map(String::from)
        .ok_or_else(|| "Antigravity returned no answer".into())
}
