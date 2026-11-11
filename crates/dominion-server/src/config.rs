//! Settings: a flat TOML file (`$XDG_CONFIG_HOME/dominion/config.toml`), which command
//! line flags override.

use serde::Deserialize;
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::time::Duration;

pub const DEFAULT_PORT: u16 = 7317;
pub const DEFAULT_KEEP_FINISHED_DAYS: u32 = 7;
pub const DEFAULT_SESSION_IDLE_MINUTES: u32 = 24 * 60;
pub const DEFAULT_RECONNECT_SECONDS: u32 = 30;

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// Address `dominion serve` listens on (default 127.0.0.1).
    pub listen: Option<IpAddr>,
    /// Port `dominion serve` listens on (default 7317).
    pub port: Option<u16>,
    /// Host names (or host:port) the server accepts in the Host header, beyond loopback
    /// and the listen address.
    #[serde(default)]
    pub allowed_hosts: Vec<String>,
    /// Minutes an MCP session may go without a call before the server closes it, which
    /// also forgets the seat it joined (default 1440; 0 never closes one). Under twenty
    /// minutes it also shortens the longest `wait_for_decision`, to half of it.
    pub session_idle_minutes: Option<u32>,
    /// The MCP endpoint `dominion play` connects to (default the local server).
    pub server: Option<String>,
    /// Seconds `dominion play` keeps trying to reach a server it lost before it gives up
    /// (default 30).
    pub reconnect_seconds: Option<u32>,
    /// Whether `dominion serve` saves its games and resumes them on restart (default true).
    pub persist: Option<bool>,
    /// Where saved games live (default `$XDG_STATE_HOME/dominion/games`).
    pub state_dir: Option<PathBuf>,
    /// Days a finished game is kept before it is deleted, checked at startup (default 7).
    pub keep_finished_days: Option<u32>,
}

fn xdg_dir(var: &str, fallback: &str) -> Option<PathBuf> {
    std::env::var_os(var)
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(fallback)))
}

impl Config {
    pub fn default_path() -> Option<PathBuf> {
        Some(xdg_dir("XDG_CONFIG_HOME", ".config")?.join("dominion/config.toml"))
    }

    /// The directory games are saved in, unless persistence is off.
    pub fn state_dir(&self) -> Option<PathBuf> {
        if self.persist == Some(false) {
            return None;
        }
        self.state_dir
            .clone()
            .or_else(|| Some(xdg_dir("XDG_STATE_HOME", ".local/state")?.join("dominion/games")))
    }

    pub fn keep_finished(&self) -> Duration {
        let days = self
            .keep_finished_days
            .unwrap_or(DEFAULT_KEEP_FINISHED_DAYS);
        Duration::from_secs(u64::from(days) * 24 * 60 * 60)
    }

    /// Reads `path`, or the default file if it exists; no file means default settings.
    pub fn load(path: Option<&Path>) -> Result<Config, String> {
        let (path, required) = match path {
            Some(p) => (p.to_path_buf(), true),
            None => match Config::default_path() {
                Some(p) => (p, false),
                None => return Ok(Config::default()),
            },
        };
        match std::fs::read_to_string(&path) {
            Ok(text) => toml::from_str(&text).map_err(|e| format!("{}: {e}", path.display())),
            Err(e) if !required && e.kind() == std::io::ErrorKind::NotFound => {
                Ok(Config::default())
            }
            Err(e) => Err(format!("{}: {e}", path.display())),
        }
    }

    pub fn listen(&self) -> IpAddr {
        self.listen.unwrap_or(IpAddr::from([127, 0, 0, 1]))
    }

    pub fn port(&self) -> u16 {
        self.port.unwrap_or(DEFAULT_PORT)
    }

    /// How long an idle MCP session is kept; `None` is until the server stops.
    pub fn session_idle(&self) -> Option<Duration> {
        let minutes = self
            .session_idle_minutes
            .unwrap_or(DEFAULT_SESSION_IDLE_MINUTES);
        (minutes > 0).then(|| Duration::from_secs(u64::from(minutes) * 60))
    }

    pub fn server(&self) -> String {
        self.server
            .clone()
            .unwrap_or_else(|| format!("http://127.0.0.1:{}/mcp", self.port()))
    }

    pub fn reconnect(&self) -> Duration {
        let seconds = self.reconnect_seconds.unwrap_or(DEFAULT_RECONNECT_SECONDS);
        Duration::from_secs(seconds.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_a_flat_file_and_rejects_unknown_keys() {
        let config: Config = toml::from_str(
            "listen = \"0.0.0.0\"\nport = 9000\nallowed_hosts = [\"box.tailnet\"]\n",
        )
        .unwrap();
        assert_eq!(config.listen(), IpAddr::from([0, 0, 0, 0]));
        assert_eq!(config.port(), 9000);
        assert_eq!(config.server(), "http://127.0.0.1:9000/mcp");
        assert_eq!(config.session_idle(), Some(Duration::from_secs(86_400)));
        assert!(toml::from_str::<Config>("prot = 1").is_err());
        let off: Config = toml::from_str("persist = false\nstate_dir = \"/x\"").unwrap();
        assert_eq!(off.state_dir(), None);
        let on: Config = toml::from_str("state_dir = \"/x\"").unwrap();
        assert_eq!(on.state_dir(), Some(PathBuf::from("/x")));
        let sessions: Config = toml::from_str("session_idle_minutes = 0").unwrap();
        assert_eq!(sessions.session_idle(), None);
    }
}
