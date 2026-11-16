use super::process::{Launch, write_json, write_private};
use crate::runner::AgentSpec;
use serde_json::{Value, json};
use std::path::Path;

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
pub fn settings(work: &Path, playbook: bool) -> Result<(), String> {
    let tools = if playbook {
        "[read_file, run_command, finish]"
    } else {
        "[finish]"
    };
    // Global config (`GEMINI_HOME/config`) holds the agent and MCP servers, wherever the
    // seat starts; `.agents/` serves a seat started in /work.
    let agent = format!(
        "---\nname: dominion\ndescription: Play a Dominion seat through structured replies\ntools: {tools}\nmainAgent: true\nsubagent: false\ncommandExecutionPolicy: off\nmcpServers: []\nskills: []\nplugins: []\n---\n{}\n",
        super::instructions(playbook)
    );
    for dir in [".agents/agents", "config/agents"] {
        let dir = work.join(dir);
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        write_private(&dir.join("dominion.md"), agent.as_bytes())?;
    }
    permissions(work, playbook, false)
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

pub fn mounts(cmd: &mut tokio::process::Command, work: &Path) {
    let target = "/home/agent/.gemini";
    cmd.env("GEMINI_HOME", target)
        .env("AGY_ACP_FORCE_FILE_STORAGE", "1");
    cmd.arg("--bind")
        .arg(work.join("config"))
        .arg(format!("{target}/config"));
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
