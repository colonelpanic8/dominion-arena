use super::{Sandbox, antigravity, claude, codex, instructions, sandbox};
use crate::archive::Archive;
use crate::codex_usage::UsageLog;
use crate::runner::{AgentSpec, Program};
use serde_json::{Value, json};
use std::collections::HashSet;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

#[derive(Debug)]
pub struct Launch {
    pub program: String,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
}

impl Launch {
    pub fn new(spec: &AgentSpec, default: &str) -> Self {
        Self {
            program: spec.command.clone().unwrap_or_else(|| default.into()),
            args: vec![],
            env: vec![],
        }
    }
}

/// Where a forwarder from outside the Nix store is mounted in the sandbox.
const FORWARDER: &str = "/run/dominion-forward";

pub struct Harness {
    work: PathBuf,
    auth: PathBuf,
    launch: Launch,
    responses: u32,
    records: Option<tokio::sync::mpsc::Receiver<super::bridge::Record>>,
    sandbox: Sandbox,
    /// The seat starts in the playbook and loads it itself.
    playbook: bool,
    forwarder: PathBuf,
    sockets: sandbox::Sockets,
    _proxy: sandbox::Proxy,
    /// Codex's usage, for what counts it: `--ephemeral` keeps no rollout of its own.
    usage: Option<UsageLog>,
}

pub fn write_json(path: &Path, value: &Value) -> Result<(), String> {
    write(path, &serde_json::to_vec(value).map_err(|e| e.to_string())?)
}

/// Writes a file only its owner may read.
pub fn write_private(path: &Path, data: &[u8]) -> Result<(), String> {
    write(path, data)
}

fn write(path: &Path, data: &[u8]) -> Result<(), String> {
    let mut options = OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options
        .open(path)
        .and_then(|mut f| f.write_all(data))
        .map_err(|e| format!("{}: {e}", path.display()))
}

pub fn executable(name: &str) -> Result<PathBuf, String> {
    let path = Path::new(name);
    let found = if path.components().count() > 1 {
        Some(path.to_path_buf())
    } else {
        std::env::var_os("PATH").and_then(|paths| {
            std::env::split_paths(&paths)
                .map(|p| p.join(name))
                .find(|p| p.is_file())
        })
    };
    found.and_then(|p| p.canonicalize().ok()).ok_or_else(|| {
        format!("native harness {name:?} is not installed; set its command to an executable path")
    })
}

impl Harness {
    pub fn new(spec: &AgentSpec, settings: &Sandbox, archive: &Path) -> Result<Self, String> {
        if matches!(
            spec.program,
            Program::T3Code | Program::OpenAi | Program::Anthropic
        ) {
            return Err("only CLI agents have a native harness".into());
        }
        if !cfg!(target_os = "linux") {
            return Err(
                "native game harness isolation currently requires Linux and bubblewrap".into(),
            );
        }
        let work = archive.join("work");
        std::fs::create_dir(&work).map_err(|e| e.to_string())?;
        let auth = (spec.auth_dir.clone())
            .or_else(|| spec.program.native_dir())
            .ok_or("HOME is unset")?;
        let auth = auth
            .canonicalize()
            .map_err(|e| format!("provider credentials {}: {e}", auth.display()))?;
        let playbook = settings.playbook.is_some();
        write(&work.join("empty"), b"")?;
        write(
            &work.join("instructions.txt"),
            instructions(playbook).as_bytes(),
        )?;
        let sockets = sandbox::Sockets::new()?;
        let hosts = (sandbox::default_hosts(spec.program).iter())
            .map(|h| h.to_string())
            .chain(settings.allow_hosts.iter().cloned())
            .collect();
        let proxy = sandbox::Proxy::start(&sockets.proxy(), hosts, archive.join("network.jsonl"))?;
        let mut usage = None;
        let mut launch = match spec.program {
            Program::Claude => claude::launch(spec, playbook),
            Program::Codex => {
                let model = codex::catalog(&auth, &work, spec.model.as_deref(), playbook)?;
                usage = Some(UsageLog::open(&auth, &model));
                let mut resolved = spec.clone();
                resolved.model = Some(model);
                codex::launch(&resolved, playbook)
            }
            Program::Antigravity => {
                antigravity::settings(&work, playbook)?;
                antigravity::launch(spec)
            }
            Program::T3Code | Program::OpenAi | Program::Anthropic => {
                unreachable!("only CLI agents have a native harness")
            }
        };
        launch.program = executable(&launch.program)?.to_string_lossy().into_owned();
        executable("bwrap")?;
        let forwarder = match &settings.forwarder {
            Some(path) => path.clone(),
            None => std::env::current_exe().map_err(|e| e.to_string())?,
        };
        Ok(Self {
            work,
            auth,
            launch,
            responses: 0,
            records: None,
            sandbox: settings.clone(),
            playbook,
            forwarder,
            sockets,
            _proxy: proxy,
            usage,
        })
    }

    /// Where the seat bridge listens, for the sandbox to reach it.
    pub fn bridge_socket(&self) -> PathBuf {
        self.sockets.bridge()
    }

    pub fn enable_mcp(
        &mut self,
        spec: &AgentSpec,
        url: &str,
        records: tokio::sync::mpsc::Receiver<super::bridge::Record>,
    ) -> Result<(), String> {
        write_json(&self.work.join("seat-mcp.json"), &json!({"url":url}))?;
        match spec.program {
            Program::Codex => {
                self.launch.args.extend([
                    "-c".into(),
                    format!(
                        "mcp_servers.dominion.url={}",
                        toml::Value::String(url.into())
                    ),
                    "-c".into(),
                    "mcp_servers.dominion.required=true".into(),
                    "-c".into(),
                    "mcp_servers.dominion.default_tools_approval_mode=\"approve\"".into(),
                ]);
            }
            Program::Claude => {
                write_json(
                    &self.work.join("mcp.json"),
                    &json!({"mcpServers":{"dominion":{"type":"http","url":url}}}),
                )?;
                let index = self
                    .launch
                    .args
                    .iter()
                    .position(|a| a == "--mcp-config")
                    .expect("MCP configuration")
                    + 1;
                self.launch.args[index] = "/work/mcp.json".into();
                self.launch
                    .args
                    .extend(["--allowedTools".into(), "mcp__dominion__*".into()]);
                if self.playbook {
                    (self.launch.args).extend(claude::PLAYBOOK_TOOLS.map(String::from));
                }
            }
            Program::Antigravity => antigravity::enable_mcp(&self.work, url, self.playbook)?,
            Program::T3Code | Program::OpenAi | Program::Anthropic => {
                unreachable!("only CLI agents have a native harness")
            }
        }
        self.records = Some(records);
        Ok(())
    }

    fn command(&self, spec: &AgentSpec) -> Result<tokio::process::Command, String> {
        let mut cmd = tokio::process::Command::new(executable("bwrap")?);
        cmd.env_clear();
        cmd.args([
            "--die-with-parent",
            "--new-session",
            // Only loopback: the proxy and the bridge are forwarded in (`sandbox`).
            "--unshare-net",
            "--unshare-pid",
            "--unshare-ipc",
            "--unshare-uts",
            "--proc",
            "/proc",
            "--dev",
            "/dev",
            "--tmpfs",
            "/tmp",
            "--dir",
            "/home/agent",
        ]);
        for path in [
            "/nix/store",
            "/usr",
            "/bin",
            "/lib",
            "/lib64",
            "/run/current-system/sw",
            "/etc/ssl",
            "/etc/static/ssl",
            "/etc/pki",
            "/etc/static/pki",
            "/etc/resolv.conf",
            "/etc/hosts",
            "/etc/nsswitch.conf",
        ] {
            if Path::new(path).exists() {
                cmd.args(["--ro-bind", path, path]);
            }
        }
        if !self.launch.program.starts_with("/nix/store/") {
            cmd.args(["--ro-bind", &self.launch.program, &self.launch.program]);
        }
        // At a fixed path: outside the store it would be somewhere under the host's home.
        let forwarder = if self.forwarder.starts_with("/nix/store/") {
            self.forwarder.clone()
        } else {
            cmd.arg("--ro-bind").arg(&self.forwarder).arg(FORWARDER);
            PathBuf::from(FORWARDER)
        };
        cmd.arg("--bind").arg(&self.work).arg("/work");
        cmd.arg("--bind")
            .arg(&self.sockets.dir)
            .arg(sandbox::SOCKETS);
        // Only the credential files: not the rest of the provider's directory (session
        // histories, settings) or ~/.claude.json (the account's email).
        let (home, files): (&str, &[&str]) = match spec.program {
            Program::Codex => ("/home/agent/.codex", &["auth.json"]),
            Program::Claude => ("/home/agent/.claude", &[".credentials.json"]),
            Program::Antigravity => (
                "/home/agent/.gemini",
                &[
                    "antigravity-cli/antigravity-oauth-token",
                    "antigravity-cli/installation_id",
                ],
            ),
            Program::T3Code | Program::OpenAi | Program::Anthropic => {
                unreachable!("only CLI agents have a native harness")
            }
        };
        cmd.args(["--dir", home]);
        for file in files {
            let source = self.auth.join(file);
            if source.is_file() {
                cmd.arg("--bind")
                    .arg(source)
                    .arg(Path::new(home).join(file));
            }
        }
        if spec.program == Program::Codex {
            cmd.env("CODEX_HOME", home);
        }
        if spec.program == Program::Antigravity {
            antigravity::mounts(&mut cmd, &self.work);
        }
        let workdir = match &self.sandbox.playbook {
            Some(playbook) => {
                cmd.arg("--ro-bind").arg(playbook).arg("/playbook");
                "/playbook"
            }
            _ => "/work",
        };
        cmd.args(["--chdir", workdir, "--"]);
        cmd.arg(&forwarder).args([
            "sandbox-forward".into(),
            "--port".into(),
            format!("{}={}/proxy.sock", sandbox::PROXY_PORT, sandbox::SOCKETS),
            "--port".into(),
            format!("{}={}/bridge.sock", sandbox::BRIDGE_PORT, sandbox::SOCKETS),
            "--".into(),
        ]);
        cmd.arg(&self.launch.program).args(&self.launch.args);
        let mut path = self.sandbox.path.clone();
        path.extend(["/run/current-system/sw/bin", "/usr/bin", "/bin"].map(PathBuf::from));
        let proxy = format!("http://127.0.0.1:{}", sandbox::PROXY_PORT);
        cmd.env("HOME", "/home/agent")
            .env(
                "PATH",
                std::env::join_paths(path).map_err(|e| e.to_string())?,
            )
            .env("HTTPS_PROXY", &proxy)
            .env("HTTP_PROXY", &proxy)
            .env("https_proxy", &proxy)
            .env("http_proxy", &proxy)
            .env("NO_PROXY", "127.0.0.1,localhost")
            .env("no_proxy", "127.0.0.1,localhost")
            .env("LANG", "C.UTF-8")
            .env("TERM", "dumb");
        for key in [
            "SSL_CERT_FILE",
            "SSL_CERT_DIR",
            "NIX_SSL_CERT_FILE",
            "CODEX_CA_CERTIFICATE",
        ] {
            if let Some(value) = std::env::var_os(key) {
                cmd.env(key, value);
            }
        }
        cmd.envs(self.launch.env.iter().cloned());
        cmd.kill_on_drop(true);
        Ok(cmd)
    }

    pub async fn reply(
        &mut self,
        spec: &AgentSpec,
        message: &str,
        archive: &mut Archive,
    ) -> Result<String, String> {
        self.responses += 1;
        let brief = instructions(self.playbook);
        let player = spec.instructions.clone().unwrap_or_default();
        let prompt = if player.trim().is_empty() {
            format!("{brief}\n\n{message}")
        } else {
            format!(
                "{brief}\n\nPlayer instructions:\n{}\n\n{message}",
                player.trim()
            )
        };
        let stem = format!("response-{:06}", self.responses);
        let stdout = archive.dir.join(format!("{stem}.stdout.jsonl"));
        let stderr = archive.dir.join(format!("{stem}.stderr"));
        archive.record("native_request",json!({"program":spec.program,"system":brief,"executable":self.launch.program,"arguments":self.launch.args,"prompt":prompt,"stdout":stdout.file_name(),"stderr":stderr.file_name()}))?;
        let mut output = private_file(&stdout)?;
        let error = private_file(&stderr)?;
        let mut cmd = self.command(spec)?;
        cmd.stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::from(error));
        let mut child = cmd
            .spawn()
            .map_err(|e| format!("starting native harness: {e}"))?;
        let input = if spec.program == Program::Antigravity {
            format!(
                "{}\n",
                json!({"event":"user", "message":{"content":prompt}})
            )
        } else {
            prompt.clone()
        };
        let mut events = vec![];
        let idle = Duration::from_secs(u64::from(spec.response_seconds.unwrap_or(300)));
        let mut deadline = tokio::time::Instant::now() + idle;
        let mut pending_tools = 0i32;
        // The tools the CLI said it has: a call to any other name can't run, and the CLI
        // answers it with an error.
        let mut exposed: Option<HashSet<String>> = None;
        let result = async {
            let mut stdin = child.stdin.take().ok_or("native harness has no stdin")?;
            tokio::time::timeout(idle, stdin.write_all(input.as_bytes()))
                .await
                .map_err(|_| "native harness response timed out from inactivity".to_string())?
                .map_err(|e| format!("sending native prompt: {e}"))?;
            drop(stdin);
            let stream = child.stdout.take().ok_or("native harness has no stdout")?;
            let mut stream = BufReader::new(stream);
            let mut line = vec![];
            loop {
                line.clear();
                let count = if let Some(records) = &mut self.records {
                    loop {
                        tokio::select! {
                            read = stream.read_until(b'\n', &mut line) => break read,
                            Some(record) = records.recv() => {
                                pending_tools += record.pending_change();
                                record.save(archive)?;
                                deadline = tokio::time::Instant::now() + idle;
                            },
                            _ = tokio::time::sleep_until(deadline), if pending_tools == 0 => return Err("native harness response timed out from inactivity".into()),
                        }
                    }
                } else {
                    tokio::time::timeout(idle, stream.read_until(b'\n', &mut line))
                        .await
                        .map_err(|_| {
                            "native harness response timed out from inactivity".to_string()
                        })?
                };
                if count.map_err(|e| e.to_string())? == 0 {
                    break;
                }
                deadline = tokio::time::Instant::now() + idle;
                output
                    .write_all(&line)
                    .and_then(|()| output.sync_data())
                    .map_err(|e| format!("saving native output: {e}"))?;
                if line.iter().all(u8::is_ascii_whitespace) {
                    continue;
                }
                let event: Value = serde_json::from_slice(&line).map_err(|e| {
                    format!(
                        "native harness emitted invalid JSON: {e}; see {}",
                        stdout.display()
                    )
                })?;
                archive.record("native_event", event.clone())?;
                if event["type"] == "system" && event["subtype"] == "init" {
                    let tools = event["tools"].as_array().into_iter().flatten();
                    exposed = Some(tools.filter_map(|t| t.as_str().map(String::from)).collect());
                    if exposed
                        .iter()
                        .flatten()
                        .any(|t| !allowed_tool(t, self.playbook))
                    {
                        return Err(
                            "native harness exposed forbidden tools; see its archive".into()
                        );
                    }
                }
                if tool_event(&event, self.playbook, exposed.as_ref()) {
                    return Err("native harness attempted a forbidden tool; see its archive".into());
                }
                if let Some(log) = &mut self.usage
                    && event["type"] == "turn.completed"
                {
                    log.record(&event["usage"]);
                }
                events.push(event);
            }
            tokio::time::timeout(idle, child.wait())
                .await
                .map_err(|_| "native harness response timed out from inactivity".to_string())?
                .map_err(|e| e.to_string())
        };
        let status = match result.await {
            Ok(status) => status,
            Err(error) => {
                let _ = child.kill().await;
                if error.contains("timed out from inactivity") {
                    archive.record("native_timeout", json!({"response":self.responses}))?;
                }
                return Err(error);
            }
        };
        for path in [&stdout, &stderr] {
            File::open(path)
                .and_then(|f| f.sync_all())
                .map_err(|e| format!("saving native output: {e}"))?;
        }
        archive.record(
            "native_exit",
            json!({"response":self.responses,"status":status.code()}),
        )?;
        if !status.success() {
            let detail = match spec.program {
                Program::Claude => claude::response(&events),
                Program::Codex => codex::response(&events),
                Program::Antigravity => antigravity::response(&events),
                Program::T3Code | Program::OpenAi | Program::Anthropic => {
                    unreachable!("only CLI agents have a native harness")
                }
            }
            .err()
            .unwrap_or_else(|| format!("exited with {status}"));
            return Err(format!(
                "{:?}: {detail}; see {}",
                spec.program,
                stderr.display()
            ));
        }
        let answer = match spec.program {
            Program::Claude => claude::response(&events),
            Program::Codex => codex::response(&events),
            Program::Antigravity => antigravity::response(&events),
            Program::T3Code | Program::OpenAi | Program::Anthropic => {
                unreachable!("only CLI agents have a native harness")
            }
        }?;
        archive.record("native_response", json!({"response":answer}))?;
        Ok(answer)
    }
}

fn private_file(path: &Path) -> Result<File, String> {
    let mut options = OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options
        .open(path)
        .map_err(|e| format!("{}: {e}", path.display()))
}

/// A tool a seat may use: the game's, and in the playbook Claude's or Antigravity's tools
/// for reading it and running its lookups.
fn allowed_tool(name: &str, playbook: bool) -> bool {
    name.strip_prefix("mcp__dominion__")
        .or_else(|| name.strip_prefix("mcp_dominion_"))
        .is_some_and(|tool| super::bridge::TOOLS.contains(&tool))
        || name == "call_mcp_tool"
        || name == "finish"
        || (playbook
            && (claude::PLAYBOOK_TOOLS.contains(&name)
                || antigravity::PLAYBOOK_TOOLS.contains(&name)))
}

/// Whether an event shows a seat using a tool it may not. A Claude Code call to a name
/// missing from `exposed` (a misspelt tool) runs nothing, so it isn't one.
fn tool_event(event: &Value, playbook: bool, exposed: Option<&HashSet<String>>) -> bool {
    // Antigravity refused something its permissions don't allow, and ended the turn.
    if event["result"]["denied_actions"]
        .as_array()
        .is_some_and(|denied| !denied.is_empty())
    {
        return true;
    }
    if matches!(event["type"].as_str(), Some("tool_use" | "tool_result")) {
        return !allowed_tool(event["name"].as_str().unwrap_or(""), playbook);
    }
    if event["message"]["content"].as_array().is_some_and(|xs| {
        xs.iter().any(|x| {
            let name = x["name"].as_str().unwrap_or("");
            x["type"] == "tool_use"
                && !allowed_tool(name, playbook)
                && exposed.is_none_or(|tools| tools.contains(name))
        })
    }) {
        return true;
    }
    if event["item"]["type"] == "mcp_tool_call" {
        let tool = event["item"]["tool"].as_str().unwrap_or("");
        // Codex's own tools for browsing MCP resources reach only the configured servers:
        // the seat bridge, which has none.
        if event["item"]["server"] == "codex" {
            return !matches!(
                tool,
                "list_mcp_resources" | "list_mcp_resource_templates" | "read_mcp_resource"
            );
        }
        return event["item"]["server"] != "dominion" || !super::bridge::TOOLS.contains(&tool);
    }
    if event["item"]["type"] == "command_execution" {
        return !playbook;
    }
    if matches!(
        event["item"]["type"].as_str(),
        Some("web_search" | "file_change")
    ) {
        return true;
    }
    event["step_update"]["step_type"] == "tool"
        && !allowed_tool(
            event["step_update"]["tool_name"]
                .as_str()
                .or_else(|| event["step_update"]["tool_info"]["name"].as_str())
                .unwrap_or(""),
            playbook,
        )
}

#[cfg(test)]
mod tests {
    use super::tool_event;
    use serde_json::json;
    use std::collections::HashSet;

    #[test]
    fn antigravity_playbook_tools_and_refusals() {
        let step = |tool: &str| json!({"event":"step_update","step_update":{"state":"ACTIVE","step_type":"tool","tool_name":tool,"tool_info":{"name":tool}}});
        for tool in ["read_file", "run_command", "manage_task"] {
            assert!(!tool_event(&step(tool), true, None), "{tool}");
            assert!(tool_event(&step(tool), false, None), "{tool}");
        }
        assert!(tool_event(&step("write_to_file"), true, None));
        let result = |denied| json!({"event":"result","result":{"status":"SUCCESS","response":"","denied_actions":denied}});
        assert!(!tool_event(&result(json!([])), true, None));
        assert!(tool_event(
            &result(json!([{"action":"command","display_name":"RunCommand"}])),
            true,
            None
        ));
    }

    #[test]
    fn a_misspelt_claude_tool_runs_nothing() {
        let call = |name: &str| json!({"type":"assistant","message":{"content":[{"type":"tool_use","name":name,"input":{}}]}});
        let exposed: HashSet<String> = ["Read", "mcp__dominion__wait", "WebFetch"]
            .map(String::from)
            .into();
        assert!(!tool_event(
            &call("mcp__dominion::wait"),
            true,
            Some(&exposed)
        ));
        assert!(!tool_event(
            &call("mcp__dominion__wait"),
            true,
            Some(&exposed)
        ));
        assert!(tool_event(&call("WebFetch"), true, Some(&exposed)));
        assert!(tool_event(&call("mcp__dominion::wait"), true, None));
    }
}
