use super::process::Launch;
use crate::runner::AgentSpec;
use serde_json::{Value, json};

pub fn launch(spec: &AgentSpec) -> Launch {
    let mut launch = Launch::new(spec, "claude");
    launch.args = [
        "-p",
        "--restricted",
        "--no-chrome",
        "--no-session-persistence",
        "--setting-sources",
        "",
        "--strict-mcp-config",
        "--mcp-config",
        "{\"mcpServers\":{}}",
        "--tools",
        "",
        "--permission-prompts",
        "none",
        "--output-format",
        "stream-json",
        "--verbose",
        "--include-partial-messages",
        // The API omits the thinking text unless asked; summarized is all it returns.
        "--thinking-display",
        "summarized",
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
        .env
        .push(("CLAUDE_CODE_DISABLE_AUTO_MEMORY".into(), "1".into()));
    launch.env.push((
        "CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC".into(),
        "1".into(),
    ));
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
