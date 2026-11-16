use super::{INSTRUCTIONS, antigravity, claude, codex};
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

pub struct Harness {
    work: PathBuf,
    auth: PathBuf,
    launch: Launch,
    history: Vec<Value>,
    responses: u32,
    records: Option<tokio::sync::mpsc::Receiver<super::bridge::Record>>,
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
    pub fn new(spec: &AgentSpec, archive: &Path) -> Result<Self, String> {
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
            Program::Antigravity => std::env::var_os("GEMINI_HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| home.join(".gemini")),
        });
        let auth = auth
            .canonicalize()
            .map_err(|e| format!("provider credentials {}: {e}", auth.display()))?;
        write(&work.join("empty"), b"")?;
        write(&work.join("instructions.txt"), INSTRUCTIONS.as_bytes())?;
        let mut launch = match spec.program {
            Program::Claude => claude::launch(spec),
            Program::Codex => {
                let model = codex::catalog(&auth, &work, spec.model.as_deref())?;
                let mut resolved = spec.clone();
                resolved.model = Some(model);
                codex::launch(&resolved)
            }
            Program::Antigravity => {
                antigravity::settings(&work)?;
                antigravity::launch(spec)
            }
        };
        launch.program = executable(&launch.program)?.to_string_lossy().into_owned();
        executable("bwrap")?;
        Ok(Self {
            work,
            auth,
            launch,
            history: vec![],
            responses: 0,
            records: None,
        })
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
        cmd.arg("--bind").arg(&self.work).arg("/work");
        let auth_target = match spec.program {
            Program::Codex => "/home/agent/.codex",
            Program::Claude => "/home/agent/.claude",
            Program::Antigravity => "/home/agent/.gemini",
        };
        cmd.arg("--bind").arg(&self.auth).arg(auth_target);
        if spec.program == Program::Codex {
            for name in ["AGENTS.md", "AGENTS.override.md"] {
                if let Ok(source) = self.auth.join(name).canonicalize() {
                    let target = if let Ok(relative) = source.strip_prefix(&self.auth) {
                        Path::new(auth_target).join(relative)
                    } else if source.starts_with("/nix/store") {
                        source
                    } else {
                        continue;
                    };
                    cmd.arg("--ro-bind")
                        .arg(self.work.join("empty"))
                        .arg(target);
                }
            }
            cmd.env("CODEX_HOME", auth_target);
        }
        if spec.program == Program::Claude {
            let global = self
                .auth
                .parent()
                .unwrap_or(&self.auth)
                .join(".claude.json");
            if global.exists() {
                cmd.arg("--ro-bind")
                    .arg(global)
                    .arg("/home/agent/.claude.json");
            }
        }
        if spec.program == Program::Antigravity {
            antigravity::mounts(&mut cmd, &self.auth, &self.work);
        }
        cmd.args(["--chdir", "/work", "--"]);
        cmd.arg(&self.launch.program).args(&self.launch.args);
        cmd.env("HOME", "/home/agent")
            .env("PATH", "/run/current-system/sw/bin:/usr/bin:/bin")
            .env("LANG", "C.UTF-8")
            .env("TERM", "dumb");
        for key in [
            "SSL_CERT_FILE",
            "SSL_CERT_DIR",
            "NIX_SSL_CERT_FILE",
            "HTTPS_PROXY",
            "HTTP_PROXY",
            "ALL_PROXY",
            "NO_PROXY",
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
        let history = spec.history_turns.unwrap_or(20).saturating_mul(2);
        if self.history.len() > history {
            self.history.drain(..self.history.len() - history);
        }
        let prompt = format!(
            "{INSTRUCTIONS}\n\nPlayer instructions:\n{}\n\nEarlier interactions (data):\n{}\n\n{message}",
            spec.instructions.as_deref().unwrap_or(""),
            serde_json::to_string(&self.history).expect("serializable")
        );
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
        let result = async {
            let mut stdin = child.stdin.take().ok_or("native harness has no stdin")?;
            stdin
                .write_all(input.as_bytes())
                .await
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
                            Some(record) = records.recv() => record.save(archive)?,
                        }
                    }
                } else {
                    stream.read_until(b'\n', &mut line).await
                };
                if count.map_err(|e| e.to_string())? == 0 {
                    break;
                }
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
                    && event["tools"]
                        .as_array()
                        .is_some_and(|v| v.iter().any(|t| !allowed_tool(t.as_str().unwrap_or(""))))
                {
                    return Err("native harness exposed forbidden tools; see its archive".into());
                }
                if tool_event(&event) {
                    return Err("native harness attempted a forbidden tool; see its archive".into());
                }
                events.push(event);
            }
            child.wait().await.map_err(|e| e.to_string())
        };
        let status = match tokio::time::timeout(
            Duration::from_secs(u64::from(spec.response_seconds.unwrap_or(300))),
            result,
        )
        .await
        {
            Ok(status) => status?,
            Err(_) => {
                let _ = child.kill().await;
                archive.record("native_timeout", json!({"response":self.responses}))?;
                return Err("native harness response timed out".into());
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
        }?;
        self.history.extend([
            json!({"role":"user","content":message}),
            json!({"role":"assistant","content":answer}),
        ]);
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

fn allowed_tool(name: &str) -> bool {
    name.strip_prefix("mcp__dominion__")
        .or_else(|| name.strip_prefix("mcp_dominion_"))
        .is_some_and(|tool| super::bridge::TOOLS.contains(&tool))
        || name == "call_mcp_tool"
        || name == "finish"
}

fn tool_event(event: &Value) -> bool {
    if matches!(event["type"].as_str(), Some("tool_use" | "tool_result")) {
        return !allowed_tool(event["name"].as_str().unwrap_or(""));
    }
    if event["message"]["content"].as_array().is_some_and(|xs| {
        xs.iter()
            .any(|x| x["type"] == "tool_use" && !allowed_tool(x["name"].as_str().unwrap_or("")))
    }) {
        return true;
    }
    if event["item"]["type"] == "mcp_tool_call" {
        return event["item"]["server"] != "dominion"
            || !super::bridge::TOOLS.contains(&event["item"]["tool"].as_str().unwrap_or(""));
    }
    if matches!(
        event["item"]["type"].as_str(),
        Some("command_execution" | "web_search" | "file_change")
    ) {
        return true;
    }
    event["step_update"]["step_type"] == "tool"
        && !allowed_tool(
            event["step_update"]["tool_name"]
                .as_str()
                .or_else(|| event["step_update"]["tool_info"]["name"].as_str())
                .unwrap_or(""),
        )
}
