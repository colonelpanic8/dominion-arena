use super::process::Launch;
use crate::runner::AgentSpec;
use serde_json::{Value, json};

/// The built-in tools a seat started in the playbook may use: reading it, and running its
/// lookup commands (the sandbox confines what they can reach).
pub const PLAYBOOK_TOOLS: [&str; 5] = ["Bash", "Read", "Grep", "Glob", "Skill"];

pub fn launch(spec: &AgentSpec, playbook: bool) -> Launch {
    let mut launch = Launch::new(spec, "claude");
    // `--restricted` ignores project settings, so a playbook seat leaves it out: it loads
    // the playbook's CLAUDE.md and skills, and bwrap does the confining.
    launch.args = if playbook {
        vec![
            "-p".into(),
            "--setting-sources".into(),
            "project".into(),
            "--tools".into(),
            PLAYBOOK_TOOLS.join(","),
        ]
    } else {
        ["-p", "--restricted", "--setting-sources", "", "--tools", ""]
            .map(String::from)
            .to_vec()
    };
    launch.args.extend(
        [
            "--no-chrome",
            "--no-session-persistence",
            "--strict-mcp-config",
            "--mcp-config",
            "{\"mcpServers\":{}}",
            "--permission-prompts",
            "none",
            "--output-format",
            "stream-json",
            "--verbose",
            "--include-partial-messages",
            // The API omits the thinking text unless asked; summarized is all it returns.
            "--thinking-display",
            "summarized",
            // The game brief replaces Claude Code's software-engineering system prompt, as
            // `model_instructions_file` does for Codex.
            "--system-prompt-file",
            "/work/instructions.txt",
        ]
        .map(String::from),
    );
    if let Some(model) = &spec.model {
        launch.args.extend(["--model".into(), model.clone()]);
    }
    if let Some(effort) = &spec.effort {
        launch.args.extend(["--effort".into(), effort.clone()]);
    }
    launch
        .env
        .push(("CLAUDE_CODE_DISABLE_AUTO_MEMORY".into(), "1".into()));
    launch.env.push((
        "CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC".into(),
        "1".into(),
    ));
    launch
        .env
        .push(("ENABLE_CLAUDEAI_MCP_SERVERS".into(), "false".into()));
    // A game tool may wait up to MAX_WAIT for the other seats; Claude Code's own limit
    // must not end the call first.
    let limit = crate::mcp::MAX_WAIT + std::time::Duration::from_secs(60);
    launch
        .env
        .push(("MCP_TOOL_TIMEOUT".into(), limit.as_millis().to_string()));
    launch
}

pub fn response(events: &[Value]) -> Result<String, String> {
    let result = events
        .iter()
        .rev()
        .find(|e| e["type"] == "result")
        .ok_or("Claude returned no result")?;
    if result["is_error"] == true {
        return Err(format!("Claude failed: {result}"));
    }
    if let Some(output) = result.get("structured_output") {
        return Ok(output.to_string());
    }
    result["result"]
        .as_str()
        .map(String::from)
        .ok_or_else(|| format!("Claude returned no answer: {}", json!(result)))
}
