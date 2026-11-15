use super::process::Launch;
use crate::runner::AgentSpec;
use serde_json::{Value, json};
use std::path::Path;

/// Use the installed provider's catalog, retaining model access and context limits
/// while removing the native capabilities that expose local tools.
pub fn catalog(auth: &Path, work: &Path, requested: Option<&str>) -> Result<String, String> {
    let path = auth.join("models_cache.json");
    let cached = std::fs::read_to_string(&path).map_err(|e| {
        format!(
            "{}: {e}; start Codex once to populate its model catalog",
            path.display()
        )
    })?;
    let mut catalog: Value = serde_json::from_str(&cached).map_err(|e| e.to_string())?;
    let models = catalog["models"]
        .as_array_mut()
        .ok_or("Codex model catalog has no models")?;
    let selected = match requested {
        Some(name) => models.iter().find(|m| m["slug"] == name),
        None => models
            .iter()
            .filter(|m| m["visibility"] == "list")
            .min_by_key(|m| m["priority"].as_i64().unwrap_or(i64::MAX)),
    }
    .and_then(|m| m["slug"].as_str())
    .ok_or("Codex model is absent from its local catalog; start Codex to refresh it")?
    .to_string();
    for model in models {
        model["shell_type"] = json!("disabled");
        model["apply_patch_tool_type"] = Value::Null;
        model["experimental_supported_tools"] = json!([]);
        model["tool_mode"] = json!("direct");
        model["supports_search_tool"] = json!(false);
        model["node_repl_disabled"] = json!(true);
        model["include_skills_usage_instructions"] = json!(false);
        model["include_plugin_usage_instructions"] = json!(false);
        model["include_apps_usage_instructions"] = json!(false);
    }
    super::process::write_json(
        &work.join("models.json"),
        &json!({"models":catalog["models"]}),
    )?;
    Ok(selected)
}

pub fn launch(spec: &AgentSpec) -> Launch {
    let mut launch = Launch::new(spec, "codex");
    launch.args = [
        "exec",
        "--ephemeral",
        "--skip-git-repo-check",
        "--ignore-user-config",
        "--sandbox",
        "read-only",
        "--json",
    ]
    .map(String::from)
    .to_vec();
    for config in [
        "approval_policy=\"never\"",
        "agents.enabled=false",
        "web_search=\"disabled\"",
        "tools.update_plan.enabled=false",
        "tools.experimental_request_user_input.enabled=false",
        "log_dir=\"/work/logs\"",
        "model_catalog_json=\"/work/models.json\"",
        "model_instructions_file=\"/work/instructions.txt\"",
    ] {
        launch.args.extend(["-c".into(), config.into()]);
    }
    for feature in [
        "shell_tool",
        "apps",
        "plugins",
        "hooks",
        "multi_agent",
        "multi_agent_v2",
        "browser_use",
        "computer_use",
        "view_image",
        "image_generation",
        "goals",
        "sleep_tool",
        "tool_suggest",
        "code_mode",
        "code_mode_only",
        "current_time_reminder",
        "token_budget",
        "request_permissions_tool",
        "send_message_to_user_async",
        "skill_search",
        "skill_mcp_dependency_install",
        "remote_plugin",
        "memories",
    ] {
        launch.args.extend(["--disable".into(), feature.into()]);
    }
    launch
        .args
        .extend(["--enable".into(), "skip_host_skill_discovery".into()]);
    if let Some(model) = &spec.model {
        launch.args.extend(["--model".into(), model.clone()]);
    }
    if let Some(effort) = &spec.effort {
        launch.args.extend([
            "-c".into(),
            format!(
                "model_reasoning_effort={}",
                toml::Value::String(effort.clone())
            ),
        ]);
    }
    launch.args.push("-".into());
    launch
}

pub fn response(events: &[Value]) -> Result<String, String> {
    if let Some(error) = events.iter().rev().find(|e| e["type"] == "turn.failed") {
        return Err(format!("Codex failed: {error}"));
    }
    events
        .iter()
        .rev()
        .find(|e| e["type"] == "item.completed" && e["item"]["type"] == "agent_message")
        .and_then(|e| e["item"]["text"].as_str())
        .map(String::from)
        .ok_or_else(|| "Codex returned no answer".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_removes_host_tools_and_rejects_models_that_would_use_fallback_capabilities() {
        let auth = tempfile::tempdir().unwrap();
        let work = tempfile::tempdir().unwrap();
        std::fs::write(
            auth.path().join("models_cache.json"),
            json!({"models":[{
                "slug":"fixture", "visibility":"list", "priority":0,
                "shell_type":"unified_exec", "apply_patch_tool_type":"freeform",
                "tool_mode":"code_mode_only", "experimental_supported_tools":["clock"],
                "context_window":100000
            }]})
            .to_string(),
        )
        .unwrap();
        assert!(catalog(auth.path(), work.path(), Some("unknown")).is_err());
        assert_eq!(catalog(auth.path(), work.path(), None).unwrap(), "fixture");
        let sanitized: Value =
            serde_json::from_slice(&std::fs::read(work.path().join("models.json")).unwrap())
                .unwrap();
        let model = &sanitized["models"][0];
        assert_eq!(model["shell_type"], "disabled");
        assert!(model["apply_patch_tool_type"].is_null());
        assert_eq!(model["experimental_supported_tools"], json!([]));
        assert_eq!(model["tool_mode"], "direct");
        assert_eq!(model["context_window"], 100000);
    }

    #[test]
    fn recovered_transport_errors_do_not_discard_a_completed_answer() {
        let events = [
            json!({"type":"error","message":"retrying"}),
            json!({"type":"item.completed","item":{"type":"agent_message","text":"answer"}}),
            json!({"type":"turn.completed"}),
        ];
        assert_eq!(response(&events).unwrap(), "answer");
        assert!(response(&[json!({"type":"turn.failed","error":{"message":"failed"}})]).is_err());
    }
}
