use super::process::{Launch, write_json};
use crate::runner::AgentSpec;
use serde_json::{Value, json};
use std::path::Path;

pub fn settings(work: &Path) -> Result<(), String> {
    let agents = work.join(".agents/agents");
    std::fs::create_dir_all(&agents).map_err(|e| e.to_string())?;
    std::fs::write(agents.join("dominion.md"), format!("---\nname: dominion\ndescription: Play a Dominion seat through structured replies\ntools: [finish]\nmainAgent: true\nsubagent: false\ncommandExecutionPolicy: off\nmcpServers: []\nskills: []\nplugins: []\n---\n{}\n", super::INSTRUCTIONS)).map_err(|e| e.to_string())?;
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

pub fn mounts(cmd: &mut tokio::process::Command, auth: &Path, work: &Path) {
    let target = "/home/agent/.gemini";
    cmd.env("GEMINI_HOME", target)
        .env("AGY_ACP_FORCE_FILE_STORAGE", "1");
    cmd.arg("--bind")
        .arg(work.join("config"))
        .arg(format!("{target}/config"));
    for directory in ["skills", "plugins", "agents"] {
        if auth.join("antigravity-cli").join(directory).exists() {
            cmd.arg("--ro-bind")
                .arg(work.join("config"))
                .arg(format!("{target}/antigravity-cli/{directory}"));
        }
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

pub fn recover(events: &[Value]) -> Option<String> {
    let result = &events.iter().rev().find(|e| e["event"] == "result")?["result"];
    if result["status"] != "ERROR"
        || !result["error"].as_str()?.contains(
            "Malformed function call: Failed to parse function call: Function call is empty - no input to parse.",
        )
    {
        return None;
    }
    let mut replies =
        serde_json::Deserializer::from_str(result["response"].as_str()?).into_iter::<Value>();
    let first = replies.next()?.ok()?;
    for reply in replies {
        if reply.ok()? != first {
            return None;
        }
    }
    let answer = serde_json::to_string(&first).ok()?;
    super::action(&answer).ok()?;
    Some(answer)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn failed(response: &str, error: &str) -> Vec<Value> {
        vec![json!({"event":"result", "result":{
            "status":"ERROR", "response":response, "error":error
        }})]
    }

    const EMPTY_CALL: &str = "Malformed function call: Failed to parse function call: Function call is empty - no input to parse.";

    #[test]
    fn recovers_only_unambiguous_game_actions_after_an_empty_function_call() {
        let action = r#"{"tool":"card","name":"Gear"}"#;
        let duplicate = format!("{action}\n{action}\n");
        let expected = serde_json::to_string(&json!({"tool":"card","name":"Gear"})).unwrap();
        assert_eq!(
            recover(&failed(&duplicate, EMPTY_CALL)),
            Some(expected.clone())
        );
        assert_eq!(recover(&failed(action, EMPTY_CALL)), Some(expected));
        for output in [
            format!("{action}\n{{\"tool\":\"card\",\"name\":\"Village\"}}"),
            format!("{action}\n{{"),
            format!("{action}\nI meant to choose something else"),
            r#"{"tool":"run_command","command":"anything"}"#.into(),
            String::new(),
        ] {
            assert!(recover(&failed(&output, EMPTY_CALL)).is_none(), "{output}");
        }
        assert!(recover(&failed(action, "authentication failed or timed out")).is_none());
        assert!(recover(&failed(action, "quota exceeded")).is_none());
    }
}
