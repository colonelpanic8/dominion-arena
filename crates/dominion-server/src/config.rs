//! Settings: a flat TOML file (`$XDG_CONFIG_HOME/dominion/config.toml`), which command
//! line flags override.

use serde::Deserialize;
use std::net::IpAddr;
use std::path::{Path, PathBuf};

pub const DEFAULT_PORT: u16 = 7317;

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
    /// The MCP endpoint `dominion play` connects to (default the local server).
    pub server: Option<String>,
}

impl Config {
    pub fn default_path() -> Option<PathBuf> {
        let base = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
        Some(base.join("dominion").join("config.toml"))
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

    pub fn server(&self) -> String {
        self.server
            .clone()
            .unwrap_or_else(|| format!("http://127.0.0.1:{}/mcp", self.port()))
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
        assert!(toml::from_str::<Config>("prot = 1").is_err());
    }
}
