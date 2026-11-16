//! Network isolation for Claude and Codex seats. The CLI runs in its own network
//! namespace with nothing but loopback; `dominion sandbox-forward` (run inside it)
//! exposes two unix sockets there as loopback ports: an allowlisting CONNECT proxy to
//! the provider's API, and the seat bridge. Nothing else on the host or the network,
//! the arena server included, is reachable.

use serde_json::json;
use std::io::Write;
use std::path::{Path, PathBuf};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream, UnixListener, UnixStream};

/// Inside the sandbox: where the CLI finds the proxy (`HTTPS_PROXY`) and the bridge.
pub const PROXY_PORT: u16 = 3128;
pub const BRIDGE_PORT: u16 = 3129;
/// Where the seat's sockets are mounted inside the sandbox.
pub const SOCKETS: &str = "/seat";

/// The hosts a program's CLI may reach. Anything else is refused and logged.
pub fn default_hosts(program: crate::runner::Program) -> &'static [&'static str] {
    use crate::runner::Program;
    match program {
        Program::Claude => &[
            "api.anthropic.com",
            "console.anthropic.com",
            "platform.claude.com",
        ],
        Program::Codex => &["chatgpt.com", "auth.openai.com", "api.openai.com"],
        Program::Antigravity => &[
            "daily-cloudcode-pa.googleapis.com",
            "cloudcode-pa.googleapis.com",
            "aicode.googleapis.com",
            "oauth2.googleapis.com",
            "www.googleapis.com",
        ],
        Program::T3Code | Program::OpenAi => &[],
    }
}

/// A host matches an entry equal to it, or an entry `*.suffix` it ends in.
pub fn allowed(host: &str, allow: &[String]) -> bool {
    let host = host.to_ascii_lowercase();
    allow.iter().any(|entry| match entry.strip_prefix("*.") {
        Some(suffix) => host.ends_with(&format!(".{suffix}")),
        None => host == entry.to_ascii_lowercase(),
    })
}

/// A short private directory for the seat's sockets (unix socket paths are limited to
/// 108 bytes), removed when dropped.
pub struct Sockets {
    pub dir: PathBuf,
}

impl Sockets {
    pub fn new() -> Result<Self, String> {
        let dir = std::env::temp_dir().join(format!("ds-{:016x}", rand::random::<u64>()));
        let mut builder = std::fs::DirBuilder::new();
        #[cfg(unix)]
        std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
        builder
            .create(&dir)
            .map_err(|e| format!("{}: {e}", dir.display()))?;
        Ok(Self { dir })
    }

    pub fn proxy(&self) -> PathBuf {
        self.dir.join("proxy.sock")
    }

    pub fn bridge(&self) -> PathBuf {
        self.dir.join("bridge.sock")
    }
}

impl Drop for Sockets {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// The allowlisting CONNECT proxy, serving until dropped. Each request is appended to
/// `log` (JSON lines: host, port, allowed).
pub struct Proxy {
    task: tokio::task::JoinHandle<()>,
}

impl Proxy {
    pub fn start(socket: &Path, allow: Vec<String>, log: PathBuf) -> Result<Self, String> {
        let listener =
            UnixListener::bind(socket).map_err(|e| format!("{}: {e}", socket.display()))?;
        let allow = std::sync::Arc::new(allow);
        let task = tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let allow = allow.clone();
                let log = log.clone();
                tokio::spawn(async move {
                    let _ = connect(stream, &allow, &log).await;
                });
            }
        });
        Ok(Self { task })
    }
}

impl Drop for Proxy {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// The target of a `CONNECT host:port HTTP/1.1` request head.
pub fn connect_target(head: &str) -> Option<(String, u16)> {
    let line = head.lines().next()?;
    let mut words = line.split_whitespace();
    if words.next()? != "CONNECT" {
        return None;
    }
    let (host, port) = words.next()?.rsplit_once(':')?;
    let host = host.trim_start_matches('[').trim_end_matches(']');
    Some((host.to_string(), port.parse().ok()?))
}

async fn connect(mut client: UnixStream, allow: &[String], log: &Path) -> std::io::Result<()> {
    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        if head.len() > 8192 || client.read(&mut byte).await? == 0 {
            return Ok(());
        }
        head.push(byte[0]);
    }
    let target = connect_target(&String::from_utf8_lossy(&head));
    let ok = target
        .as_ref()
        .is_some_and(|(host, port)| *port == 443 && allowed(host, allow));
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log)
    {
        let (host, port) = target.clone().unwrap_or_default();
        let _ = writeln!(file, "{}", json!({"host":host,"port":port,"allowed":ok}));
    }
    let Some((host, port)) = target.filter(|_| ok) else {
        client
            .write_all(b"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\n\r\n")
            .await?;
        return Ok(());
    };
    let mut upstream = match TcpStream::connect((host.as_str(), port)).await {
        Ok(upstream) => upstream,
        Err(_) => {
            client
                .write_all(b"HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\n\r\n")
                .await?;
            return Ok(());
        }
    };
    client
        .write_all(b"HTTP/1.1 200 Connection established\r\n\r\n")
        .await?;
    tokio::io::copy_bidirectional(&mut client, &mut upstream).await?;
    Ok(())
}

/// Inside the sandbox: listen on each loopback port and pass its connections to its
/// unix socket, run `command`, and return its exit code.
pub async fn forward(ports: Vec<(u16, PathBuf)>, command: Vec<String>) -> Result<i32, String> {
    for (port, socket) in ports {
        let listener = TcpListener::bind(("127.0.0.1", port))
            .await
            .map_err(|e| format!("127.0.0.1:{port}: {e}"))?;
        tokio::spawn(async move {
            while let Ok((mut stream, _)) = listener.accept().await {
                let socket = socket.clone();
                tokio::spawn(async move {
                    if let Ok(mut unix) = UnixStream::connect(&socket).await {
                        let _ = tokio::io::copy_bidirectional(&mut stream, &mut unix).await;
                    }
                });
            }
        });
    }
    let (program, args) = command.split_first().ok_or("no command to run")?;
    let status = tokio::process::Command::new(program)
        .args(args)
        .status()
        .await
        .map_err(|e| format!("{program}: {e}"))?;
    Ok(status.code().unwrap_or(1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_listed_hosts_are_allowed() {
        let allow = vec!["api.anthropic.com".to_string(), "*.example.com".into()];
        assert!(allowed("api.anthropic.com", &allow));
        assert!(allowed("API.Anthropic.com", &allow));
        assert!(allowed("a.b.example.com", &allow));
        assert!(!allowed("example.com", &allow));
        assert!(!allowed("api.anthropic.com.evil.net", &allow));
        assert!(!allowed("127.0.0.1", &allow));
    }

    #[test]
    fn connect_requests_name_their_target() {
        let head = "CONNECT api.anthropic.com:443 HTTP/1.1\r\nHost: x\r\n\r\n";
        assert_eq!(
            connect_target(head),
            Some(("api.anthropic.com".into(), 443))
        );
        assert_eq!(connect_target("GET http://x/ HTTP/1.1\r\n\r\n"), None);
        assert_eq!(
            connect_target("CONNECT [::1]:443 HTTP/1.1"),
            Some(("::1".into(), 443))
        );
    }

    #[tokio::test]
    async fn the_proxy_refuses_hosts_off_the_list_and_logs_them() {
        let dir = std::env::temp_dir().join(format!("dsp-{:08x}", rand::random::<u32>()));
        std::fs::create_dir(&dir).unwrap();
        let socket = dir.join("p.sock");
        let log = dir.join("net.jsonl");
        let _proxy = Proxy::start(&socket, vec!["allowed.invalid".into()], log.clone()).unwrap();
        let mut stream = UnixStream::connect(&socket).await.unwrap();
        stream
            .write_all(b"CONNECT 127.0.0.1:47317 HTTP/1.1\r\n\r\n")
            .await
            .unwrap();
        let mut reply = String::new();
        stream.read_to_string(&mut reply).await.unwrap();
        assert!(reply.starts_with("HTTP/1.1 403"), "{reply}");
        let logged = std::fs::read_to_string(&log).unwrap();
        assert!(logged.contains("\"allowed\":false"), "{logged}");
        std::fs::remove_dir_all(dir).unwrap();
    }
}
