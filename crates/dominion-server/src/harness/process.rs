use super::{Sandbox, antigravity, claude, codex, instructions, sandbox};
use crate::archive::Archive;
use crate::runner::{AgentSpec, Program};
use serde_json::{Value, json};
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
    /// The seat starts in the playbook and loads it itself (Claude and Codex), rather than
    /// being sent it in its first message.
    native_playbook: bool,
    forwarder: PathBuf,
    sockets: sandbox::Sockets,
    _proxy: sandbox::Proxy,
}

pub fn write_json(path: &Path, value: &Value) -> Result<(), String> {
    write(path, &serde_json::to_vec(value).map_err(|e| e.to_string())?)
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
        if spec.program == Program::T3Code {
            return Err("T3 Code agents have no native harness".into());
        }
        if !cfg!(target_os = "linux") {
            return Err(
                "native game harness isolation currently requires Linux and bubblewrap".into(),
            );
        }
        let work = archive.join("work");
        std::fs::create_dir(&work).map_err(|e| e.to_string())?;
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .ok_or("HOME is unset")?;
        let auth = spec.auth_dir.clone().unwrap_or_else(|| match spec.program {
            Program::Claude => std::env::var_os("CLAUDE_CONFIG_DIR")
                .map(PathBuf::from)
                .unwrap_or_else(|| home.join(".claude")),
            Program::Codex => std::env::var_os("CODEX_HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| home.join(".codex")),
            Program::T3Code => unreachable!("T3 Code agents have no native harness"),
            Program::Antigravity => std::env::var_os("GEMINI_HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| home.join(".gemini")),
        });
        let auth = auth
            .canonicalize()
            .map_err(|e| format!("provider credentials {}: {e}", auth.display()))?;
        let native_playbook =
            settings.playbook.is_some() && matches!(spec.program, Program::Claude | Program::Codex);
        write(&work.join("empty"), b"")?;
        write(
            &work.join("instructions.txt"),
            instructions(native_playbook).as_bytes(),
        )?;
        let sockets = sandbox::Sockets::new()?;
        let hosts = (sandbox::default_hosts(spec.program).iter())
            .map(|h| h.to_string())
            .chain(settings.allow_hosts.iter().cloned())
            .collect();
        let proxy = sandbox::Proxy::start(&sockets.proxy(), hosts, archive.join("network.jsonl"))?;
        let mut launch = match spec.program {
            Program::Claude => claude::launch(spec, native_playbook),
            Program::Codex => {
                let model = codex::catalog(&auth, &work, spec.model.as_deref(), native_playbook)?;
                let mut resolved = spec.clone();
                resolved.model = Some(model);
                codex::launch(&resolved, native_playbook)
            }
            Program::Antigravity => {
                antigravity::settings(&work)?;
                antigravity::launch(spec)
            }
            Program::T3Code => unreachable!("T3 Code agents have no native harness"),
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
            native_playbook,
            forwarder,
            sockets,
            _proxy: proxy,
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
                if self.native_playbook {
                    (self.launch.args).extend(claude::PLAYBOOK_TOOLS.map(String::from));
                }
            }
            Program::Antigravity => {
                write_json(
                    &self.work.join("config/mcp_config.json"),
                    &json!({"mcpServers":{"dominion":{"serverUrl":url,"disabled":false,"tools":super::bridge::TOOLS.iter().map(|name| (name.to_string(),json!({"eager":true}))).collect::<serde_json::Map<String,Value>>()}}}),
                )?;
                for path in [".agents/agents/dominion.md", "config/agents/dominion.md"] {
                    let path = self.work.join(path);
                    let text = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
                    write(
                        &path,
                        text.replace("mcpServers: []", "mcpServers: []\ninheritMcp: true")
                            .as_bytes(),
                    )?;
                }
                write_json(
                    &self.work.join("settings.json"),
                    &json!({"permissions":{"deny":["read_file(*)","write_file(*)","command(*)","read_url(*)","execute_url(*)"],"allow":["mcp(dominion/*)"],"ask":[]},"toolPermission":"request-review"}),
                )?;
            }
            Program::T3Code => unreachable!("T3 Code agents have no native harness"),
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
            Program::T3Code => unreachable!("T3 Code agents have no native harness"),
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
            Some(playbook) if self.native_playbook => {
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
        let brief = instructions(self.native_playbook);
        let mut player = spec.instructions.clone().unwrap_or_default();
        if !self.native_playbook
            && let Some(playbook) = &self.sandbox.playbook
        {
            player = format!("{}\n\n{player}", super::inline_playbook(playbook)?);
        }
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
        archive.record("native_request",json!({"program":spec.program,"executable":self.launch.program,"arguments":self.launch.args,"prompt":prompt,"stdout":stdout.file_name(),"stderr":stderr.file_name()}))?;
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
                if event["type"] == "system"
                    && event["subtype"] == "init"
                    && event["tools"].as_array().is_some_and(|v| {
                        v.iter()
                            .any(|t| !allowed_tool(t.as_str().unwrap_or(""), self.native_playbook))
                    })
                {
                    return Err("native harness exposed forbidden tools; see its archive".into());
                }
                if tool_event(&event, self.native_playbook) {
                    return Err("native harness attempted a forbidden tool; see its archive".into());
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
                Program::T3Code => unreachable!("T3 Code agents have no native harness"),
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
            Program::T3Code => unreachable!("T3 Code agents have no native harness"),
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

/// A tool a seat may use: the game's, and in the playbook Claude's tools for reading it and
/// running its lookups.
fn allowed_tool(name: &str, playbook: bool) -> bool {
    name.strip_prefix("mcp__dominion__")
        .or_else(|| name.strip_prefix("mcp_dominion_"))
        .is_some_and(|tool| super::bridge::TOOLS.contains(&tool))
        || name == "call_mcp_tool"
        || name == "finish"
        || (playbook && claude::PLAYBOOK_TOOLS.contains(&name))
}

/// Whether an event shows a seat using a tool it may not.
fn tool_event(event: &Value, playbook: bool) -> bool {
    if matches!(event["type"].as_str(), Some("tool_use" | "tool_result")) {
        return !allowed_tool(event["name"].as_str().unwrap_or(""), playbook);
    }
    if event["message"]["content"].as_array().is_some_and(|xs| {
        xs.iter().any(|x| {
            x["type"] == "tool_use" && !allowed_tool(x["name"].as_str().unwrap_or(""), playbook)
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
