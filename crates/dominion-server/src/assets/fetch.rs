//! Polite HTTP: a plain User-Agent, timeouts, retries with backoff on network errors
//! and 5xx, and a stop on 429 or repeated 403.

use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

pub const IN_FLIGHT: usize = 4;
const ATTEMPTS: u32 = 3;
const MAX_FORBIDDEN: usize = 3;
const MAX_UNREACHABLE: usize = 8;
const MAX_BODY: u64 = 64 << 20;

pub enum Fetched {
    Body(Vec<u8>),
    /// Not obtainable; the reason goes in the manifest.
    Missing(String),
}

/// One request's result.
enum Reply {
    Body(Vec<u8>),
    Status(u16),
    /// No response at all.
    Unreachable(String),
    /// A response whose body broke off.
    Broken(String),
}

/// Uses its own TLS stack with built-in roots: enabling TLS on reqwest would also make
/// the MCP client's default reqwest client require the system's certificates.
pub struct Fetcher {
    agent: ureq::Agent,
    base: String,
    forbidden: AtomicUsize,
    /// Requests in a row that failed without a response.
    unreachable: AtomicUsize,
}

impl Fetcher {
    pub fn new(base: &str) -> Fetcher {
        let agent = ureq::Agent::config_builder()
            .user_agent(concat!(
                "dominion-engine-assets/",
                env!("CARGO_PKG_VERSION")
            ))
            .timeout_connect(Some(Duration::from_secs(15)))
            .timeout_global(Some(Duration::from_secs(60)))
            .http_status_as_error(false)
            .build()
            .into();
        Fetcher {
            agent,
            base: base.trim_end_matches('/').to_string(),
            forbidden: AtomicUsize::new(0),
            unreachable: AtomicUsize::new(0),
        }
    }

    pub fn url(&self, path: &str) -> String {
        url(&self.base, path)
    }

    /// Fetches `path` (relative to the site root). `Err` means the sync must stop.
    pub async fn get(&self, path: &str) -> Result<Fetched, String> {
        let url = self.url(path);
        let mut last = String::new();
        for attempt in 0..ATTEMPTS {
            if attempt > 0 {
                tokio::time::sleep(Duration::from_secs(1 << attempt)).await;
            }
            let agent = self.agent.clone();
            let target = url.clone();
            let reply = tokio::task::spawn_blocking(move || request(&agent, &target))
                .await
                .map_err(|e| e.to_string())?;
            if !matches!(reply, Reply::Unreachable(_)) {
                self.unreachable.store(0, Ordering::Relaxed);
            }
            match reply {
                Reply::Unreachable(e) => {
                    last = e;
                    if self.unreachable.fetch_add(1, Ordering::Relaxed) + 1 >= MAX_UNREACHABLE {
                        return Err(format!("can't reach dominion.games: {last}"));
                    }
                }
                Reply::Body(body) => return Ok(Fetched::Body(body)),
                Reply::Broken(e) => last = e,
                Reply::Status(429) => {
                    return Err(
                        "dominion.games is limiting our requests (429); try again later".into(),
                    );
                }
                Reply::Status(403) => {
                    if self.forbidden.fetch_add(1, Ordering::Relaxed) + 1 >= MAX_FORBIDDEN {
                        return Err(format!(
                            "dominion.games refused {MAX_FORBIDDEN} requests (403), last {url}"
                        ));
                    }
                    return Ok(Fetched::Missing("403".into()));
                }
                Reply::Status(code @ 500..=599) => last = code.to_string(),
                Reply::Status(code) => return Ok(Fetched::Missing(code.to_string())),
            }
        }
        Ok(Fetched::Missing(format!(
            "{last} after {ATTEMPTS} attempts"
        )))
    }

    /// Fetches a page or bundle as text; anything but a body stops the sync.
    pub async fn text(&self, path: &str) -> Result<String, String> {
        match self.get(path).await? {
            Fetched::Body(b) => Ok(String::from_utf8_lossy(&b).into_owned()),
            Fetched::Missing(reason) => Err(format!("fetching {}: {reason}", self.url(path))),
        }
    }
}

fn request(agent: &ureq::Agent, url: &str) -> Reply {
    let mut response = match agent.get(url).call() {
        Ok(r) => r,
        Err(e) => return Reply::Unreachable(e.to_string()),
    };
    let status = response.status().as_u16();
    if !(200..300).contains(&status) {
        return Reply::Status(status);
    }
    match response
        .body_mut()
        .with_config()
        .limit(MAX_BODY)
        .read_to_vec()
    {
        Ok(body) => Reply::Body(body),
        Err(e) => Reply::Broken(e.to_string()),
    }
}

/// The URL of `path` (relative to the site root) under `base`.
pub fn url(base: &str, path: &str) -> String {
    format!("{base}/{}", path.replace('\'', "%27"))
}

/// Writes through a temporary file in the same directory and renames it into place, so
/// an interrupted write never leaves a truncated file at `path`.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let dir = path.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir)?;
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let tmp = dir.join(format!(".{name}.{}.part", std::process::id()));
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })
}
