use super::process::{Launch, write_json};
use crate::runner::AgentSpec;
use serde_json::{Value, json};
use std::path::Path;

pub fn settings(work: &Path) -> Result<(), String> {
    let agents = work.join(".agents/agents");
    std::fs::create_dir_all(&agents).map_err(|e| e.to_string())?;
    std::fs::write(agents.join("dominion.md"), format!("---\nname: dominion\ndescription: Play a Dominion seat through structured replies\ntools: [finish]\nmainAgent: true\nsubagent: false\ncommandExecutionPolicy: off\nmcpServers: []\nskills: []\nplugins: []\n---\n{}\n", super::instructions(false))).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(work.join("config/agents")).map_err(|e| e.to_string())?;
    std::fs::copy(
        agents.join("dominion.md"),
        work.join("config/agents/dominion.md"),
    )
    .map_err(|e| e.to_string())?;
    write_json(
        &work.join("settings.json"),
        &json!({
            "permissions": {"deny": ["read_file(*)", "write_file(*)", "command(*)", "mcp(*)", "read_url(*)", "execute_url(*)"], "ask": [], "allow": []},
            "toolPermission": "strict"
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
