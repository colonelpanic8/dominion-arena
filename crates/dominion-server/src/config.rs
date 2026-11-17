//! Settings: a flat TOML file (`$XDG_CONFIG_HOME/dominion/config.toml`), which command
//! line flags override.

use dominion_protocol::api::{AnimationSpeed, WebSettings};
use serde::Deserialize;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::time::Duration;

pub const DEFAULT_PORT: u16 = 7317;
pub const DEFAULT_KEEP_FINISHED_DAYS: u32 = 7;
pub const DEFAULT_SESSION_IDLE_MINUTES: u32 = 24 * 60;
pub const DEFAULT_RECONNECT_SECONDS: u32 = 30;
pub const DEFAULT_KEEP_IDLE_DAYS: u32 = 30;
pub const DEFAULT_AWAY_MINUTES: u32 = 5;
pub const DEFAULT_DECISION_MINUTES: u32 = 30;
pub const DEFAULT_AGENT_SEATS_PER_DAY: u32 = 5;
pub const DEFAULT_AGENTS_AT_ONCE: u32 = 2;
/// The environment variable naming the web client to serve when `web_dir` is unset.
pub const WEB_DIR_ENV: &str = "DOMINION_WEB_DIR";

/// How long a game is hosted, from a number of days: 0 is for as long as the server runs.
fn keep_days(days: u32) -> Duration {
    match days {
        0 => Duration::MAX,
        days => Duration::from_secs(u64::from(days) * 24 * 60 * 60),
    }
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// Serve fixed Arena seats using the pinned binary gameplay protocol (default false).
    pub official_server_enabled: Option<bool>,
    /// Hash-pinned client bundle; defaults to the existing online catalog cache.
    pub official_server_catalog_file: Option<PathBuf>,
    /// Enable the original client's host/port-only root WebSocket alias.
    pub official_server_root_socket: Option<bool>,
    /// Per-account flat preference files, separate from game state and asset cache.
    pub official_preferences_dir: Option<PathBuf>,
    /// Separately populated original application cache; never downloaded by the host.
    pub official_client_assets_dir: Option<PathBuf>,
    /// Official-server account (separate from Arena's own accounts).
    pub online_account: Option<String>,
    /// Keep one official connection in `dominion serve`.
    pub online_enabled: Option<bool>,
    /// Registered Arena account allowed to observe/control the official connection.
    pub online_owner: Option<String>,
    /// Password-store entry; only its first line is read at runtime.
    pub online_password_entry: Option<String>,
    /// Runtime credential file; mutually exclusive with online_password_entry.
    pub online_password_file: Option<PathBuf>,
    /// Operator assertion that this account is idle and exclusive to this client.
    pub online_exclusive: Option<bool>,
    /// Resume only a bot table previously created by this state directory.
    pub online_resume: Option<bool>,
    /// Official endpoint: auto (default), alpha, or beta.
    pub online_endpoint: Option<String>,
    /// Official-client process lock and non-secret session metadata.
    pub online_state_dir: Option<PathBuf>,
    /// Downloaded hash-pinned public protocol catalog bundle.
    pub online_cache_dir: Option<PathBuf>,
    /// More official accounts, as `[[online_accounts]]` tables, each with its own owner,
    /// credential and state directory. The `online_*` account above is the one with id
    /// `default`.
    pub online_accounts: Option<Vec<OnlineAccountSettings>>,
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
    /// The MCP endpoint `dominion play` connects to (default the local server, at
    /// `listen` and `port`).
    pub server: Option<String>,
    /// Seconds `dominion play` keeps trying to reach a server it lost before it gives up
    /// (default 30).
    pub reconnect_seconds: Option<u32>,
    /// Whether `dominion serve` saves its games and resumes them on restart (default true).
    pub persist: Option<bool>,
    /// Where saved games live (default `$XDG_STATE_HOME/dominion/games`).
    pub state_dir: Option<PathBuf>,
    /// Days a finished game is hosted before it is moved to `expired/` (default 7; 0: no
    /// limit).
    pub keep_finished_days: Option<u32>,
    /// Where `dominion assets sync` keeps dominion.games' images, fonts and sounds, and
    /// `dominion serve` serves them from (default `$XDG_CACHE_HOME/dominion/assets`).
    pub assets_dir: Option<PathBuf>,
    /// The built web client `dominion serve` serves at `/` (default `$DOMINION_WEB_DIR`,
    /// which a packaged build sets to its own client; otherwise none).
    pub web_dir: Option<PathBuf>,
    /// The web client shows cards moving (default true).
    pub web_animations: Option<bool>,
    /// How fast cards move: `slow`, `standard` (default), `sprightly`, `swift`, `speedy`
    /// or `none`, as dominion.games' Animation Type.
    pub web_animation_speed: Option<AnimationSpeed>,
    /// On other players' turns the kingdom drops and their play area goes above it
    /// (default true).
    pub web_move_kingdom: Option<bool>,
    /// A single card to trash, discard, exile or topdeck waits for a confirming button
    /// (default false: it is sent on the click).
    pub web_confirm_single: Option<bool>,
    /// The painting under the table, by its name in the asset cache's `images/large`, or
    /// "" for none (default `inheritance`).
    pub web_background: Option<String>,
    /// The web client plays sounds (default true).
    pub web_sounds: Option<bool>,
    /// The web client's sound volume, 0.0 to 1.0 (default 0.5).
    pub web_volume: Option<f32>,
    /// Cards a table never deals at random while it respects the Banned list (the web
    /// client's Cardlists tab writes it).
    pub banned_cards: Option<Vec<String>>,
    /// The URL people reach the server at, when it isn't `listen` and `port` (behind a
    /// reverse proxy): its host is accepted in the Host header, and an `https` one makes
    /// session cookies HTTPS-only.
    pub public_url: Option<String>,
    /// Take a client's address from the `X-Forwarded-For` header, for the limits on what
    /// one address may do (default false: set it only behind a reverse proxy that sets
    /// the header).
    pub trust_proxy: Option<bool>,
    /// Players rated here and on dominion.games, as `[[official_skill_anchors]]` tables:
    /// `player` (the account here) and `skill` (its dominion.games skill, mu, not level).
    /// With two or more, ratings are also shown on dominion.games' scale.
    pub official_skill_anchors: Option<Vec<crate::arena::SkillAnchor>>,
    /// Whether new accounts can be registered (default true).
    pub registration: Option<bool>,
    /// Whether people can play without an account, as guests (default true).
    pub guests: Option<bool>,
    /// The most games in progress at once (default 1000).
    pub max_games: Option<usize>,
    /// Days an unfinished game nobody plays is hosted before it is moved to `expired/`
    /// (default 30; 0: no limit).
    pub keep_idle_days: Option<u32>,
    /// In a lobby game, minutes the player who must decide may be disconnected before
    /// they forfeit (default 5; 0 never).
    pub away_minutes: Option<u32>,
    /// In a lobby game, minutes a player may take over one decision before they forfeit
    /// (default 30; 0 never).
    pub decision_minutes: Option<u32>,
    /// The Unix socket where `dominion serve` takes a runner of its own agents and
    /// `dominion agents` connects (default: none, and no such agents).
    pub agent_socket: Option<PathBuf>,
    /// The Unix socket where `dominion serve` answers `dominion games` (default: none).
    /// Only its owner may connect.
    pub control_socket: Option<PathBuf>,
    /// How many seats one registered player may fill with the server's agents a day
    /// (default 5; 0: no limit).
    pub agent_seats_per_day: Option<u32>,
    /// The agents `dominion agents` offers, as `[[agents]]` tables: `name`, `program`
    /// (`claude`, `codex`, or `antigravity`), and optionally `model`, `command`, `about` (default: Claude
    /// Opus, Claude Sonnet and Codex).
    pub agents: Option<Vec<crate::runner::AgentSpec>>,
    /// Providers whose models `dominion agents` offers, each at every effort allowed, as
    /// `[[catalogs]]` tables: `program`, and optionally `models` and `exclude` (patterns
    /// of model ids), `efforts`, `tag` and the agents' other settings. An `[[agents]]`
    /// profile of the same program, model and effort is offered in its place.
    pub catalogs: Option<Vec<crate::catalog::CatalogSpec>>,
    /// How many agent seats `dominion agents` plays at once (default 2).
    pub agents_at_once: Option<u32>,
    /// Where `dominion agents` runs its agents and keeps their logs (default
    /// `$XDG_STATE_HOME/dominion/agents`).
    pub agents_dir: Option<PathBuf>,
    /// A dominion-playbook checkout. Claude and Codex seats start in it, read-only, and
    /// load it natively; Antigravity seats are sent it in their first message.
    pub playbook_dir: Option<PathBuf>,
    /// Directories put first on PATH in the agents' sandbox (a shell, python3, coreutils).
    pub sandbox_path: Option<Vec<PathBuf>>,
    /// Hosts agents may reach beyond their provider's API (`*.example.com` allowed).
    pub network_allow: Option<Vec<String>>,
    /// Where `dominion solo` keeps its games (default `$XDG_STATE_HOME/dominion/solo`).
    pub solo_dir: Option<PathBuf>,
    /// Connection and workspace used by T3 Code agents.
    pub t3code: Option<crate::t3::Settings>,
}

/// One `[[online_accounts]]` table.
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct OnlineAccountSettings {
    /// Lowercase letters, digits and `-`: names the account in routes.
    pub id: String,
    /// The official-server account name.
    pub account: String,
    /// Registered Arena account allowed to control it.
    pub owner: Option<String>,
    pub password_entry: Option<String>,
    pub password_file: Option<PathBuf>,
    pub exclusive: Option<bool>,
    pub resume: Option<bool>,
    pub endpoint: Option<String>,
    /// Default `<online_state_dir>/accounts/<id>`.
    pub state_dir: Option<PathBuf>,
    /// Kept by `dominion serve` (default true).
    pub enabled: Option<bool>,
}

fn xdg_dir(var: &str, fallback: &str) -> Option<PathBuf> {
    std::env::var_os(var)
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(fallback)))
}

impl Config {
    pub fn online_state_dir(&self) -> Option<PathBuf> {
        self.online_state_dir
            .clone()
            .or_else(|| Some(xdg_dir("XDG_STATE_HOME", ".local/state")?.join("dominion/online")))
    }

    /// Every configured official account: the `online_*` one first (id `default`), then
    /// `online_accounts`.
    pub fn online_accounts(&self) -> Result<Vec<crate::online::Account>, String> {
        use crate::online::{Account, LEGACY_ID};
        let state = || {
            self.online_state_dir()
                .ok_or_else(|| "set online_state_dir or HOME".to_string())
        };
        let mut all = Vec::new();
        if self.online_account.is_some() || self.online_enabled == Some(true) {
            all.push(Account {
                id: LEGACY_ID.into(),
                account: self.online_account.clone().ok_or("set online_account")?,
                owner: self.online_owner.clone(),
                password_entry: self.online_password_entry.clone(),
                password_file: self.online_password_file.clone(),
                exclusive: self.online_exclusive == Some(true),
                resume: self.online_resume.unwrap_or(false),
                endpoint: self.online_endpoint.clone(),
                state_dir: state()?,
                serve: self.online_enabled.unwrap_or(false),
            });
        }
        for a in self.online_accounts.iter().flatten() {
            if a.id.is_empty()
                || a.id == LEGACY_ID
                || !(a.id.bytes())
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
            {
                return Err(format!(
                    "online_accounts id {:?}: use lowercase letters, digits and -, not {LEGACY_ID:?}",
                    a.id
                ));
            }
            if a.account.trim().is_empty() {
                return Err(format!("online_accounts {}: set account", a.id));
            }
            all.push(Account {
                id: a.id.clone(),
                account: a.account.clone(),
                owner: a.owner.clone(),
                password_entry: a.password_entry.clone(),
                password_file: a.password_file.clone(),
                exclusive: a.exclusive == Some(true),
                resume: a.resume.unwrap_or(false),
                endpoint: a.endpoint.clone(),
                state_dir: match &a.state_dir {
                    Some(dir) => dir.clone(),
                    None => state()?.join("accounts").join(&a.id),
                },
                serve: a.enabled.unwrap_or(true),
            });
        }
        for (i, a) in all.iter().enumerate() {
            for b in &all[..i] {
                if a.id == b.id {
                    return Err(format!("duplicate online account id {:?}", a.id));
                }
                if a.account.eq_ignore_ascii_case(&b.account) {
                    return Err(format!(
                        "official account {:?} is configured twice ({} and {})",
                        a.account, b.id, a.id
                    ));
                }
                if a.state_dir == b.state_dir {
                    return Err(format!(
                        "online accounts {} and {} share the state directory {}",
                        b.id,
                        a.id,
                        a.state_dir.display()
                    ));
                }
            }
        }
        Ok(all)
    }

    /// The account `id` names, or by default the `online_*` one (else the only one).
    pub fn online_account(&self, id: Option<&str>) -> Result<crate::online::Account, String> {
        let mut all = self.online_accounts()?;
        let index = match id {
            Some(id) => all.iter().position(|a| a.id == id),
            None if all.len() == 1 => Some(0),
            None => all.iter().position(|a| a.id == crate::online::LEGACY_ID),
        };
        match index {
            Some(i) => Ok(all.swap_remove(i)),
            None if all.is_empty() => Err("no official account: set online_account".into()),
            None => Err(format!(
                "choose an account with --account: {}",
                all.iter()
                    .map(|a| a.id.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )),
        }
    }

    pub fn online_cache_dir(&self) -> Option<PathBuf> {
        self.online_cache_dir
            .clone()
            .or_else(|| Some(xdg_dir("XDG_CACHE_HOME", ".cache")?.join("dominion/online")))
    }
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

    /// The directory solo games are kept in.
    pub fn solo_dir(&self) -> Option<PathBuf> {
        self.solo_dir
            .clone()
            .or_else(|| Some(xdg_dir("XDG_STATE_HOME", ".local/state")?.join("dominion/solo")))
    }

    pub fn keep_finished(&self) -> Duration {
        keep_days(
            self.keep_finished_days
                .unwrap_or(DEFAULT_KEEP_FINISHED_DAYS),
        )
    }

    /// The asset cache directory.
    pub fn assets_dir(&self) -> Option<PathBuf> {
        self.assets_dir
            .clone()
            .or_else(|| Some(xdg_dir("XDG_CACHE_HOME", ".cache")?.join("dominion/assets")))
    }

    /// The built web client to serve, if any.
    pub fn web_dir(&self) -> Option<PathBuf> {
        self.web_dir_or(std::env::var_os(WEB_DIR_ENV))
    }

    fn web_dir_or(&self, env: Option<std::ffi::OsString>) -> Option<PathBuf> {
        self.web_dir
            .clone()
            .or_else(|| env.filter(|v| !v.is_empty()).map(PathBuf::from))
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

    /// What the settings say about accounts and the lobby.
    pub fn arena(&self) -> crate::arena::Settings {
        let minutes = |m: u32| Duration::from_secs(u64::from(m) * 60);
        crate::arena::Settings {
            registration: self.registration.unwrap_or(true),
            guests: self.guests.unwrap_or(true),
            banned_cards: self.banned_cards.clone().unwrap_or_default(),
            timing: crate::hub::Timing {
                away: minutes(self.away_minutes.unwrap_or(DEFAULT_AWAY_MINUTES)),
                decision: minutes(self.decision_minutes.unwrap_or(DEFAULT_DECISION_MINUTES)),
            },
            max_games: self.max_games.unwrap_or(crate::hub::DEFAULT_MAX_GAMES),
            keep_finished: self.keep_finished(),
            keep_idle: keep_days(self.keep_idle_days.unwrap_or(DEFAULT_KEEP_IDLE_DAYS)),
            agent_seats_per_day: self
                .agent_seats_per_day
                .unwrap_or(DEFAULT_AGENT_SEATS_PER_DAY),
            skill_anchors: self.official_skill_anchors.clone().unwrap_or_default(),
        }
    }

    /// What `dominion agents` runs with, if the settings say where the server is.
    pub fn runner(&self) -> Result<crate::runner::Settings, String> {
        let socket = (self.agent_socket.clone())
            .ok_or("no agent_socket: set it to the server's in the settings file")?;
        let dir = (self.agents_dir.clone())
            .or_else(|| Some(xdg_dir("XDG_STATE_HOME", ".local/state")?.join("dominion/agents")))
            .ok_or("no place for the agents: set agents_dir (or XDG_STATE_HOME or HOME)")?;
        let catalogs = self.catalogs.clone().unwrap_or_default();
        for catalog in &catalogs {
            catalog.validate()?;
        }
        let agents = self
            .agents
            .clone()
            .unwrap_or_else(|| match catalogs.is_empty() {
                true => crate::runner::default_agents(),
                false => vec![],
            });
        for (i, agent) in agents.iter().enumerate() {
            agent.validate()?;
            if agents[..i]
                .iter()
                .any(|a| a.name.eq_ignore_ascii_case(&agent.name))
            {
                return Err(format!("duplicate agent profile name {:?}", agent.name));
            }
        }
        Ok(crate::runner::Settings {
            socket,
            server: self.server(),
            agents,
            catalogs,
            capacity: self.agents_at_once.unwrap_or(DEFAULT_AGENTS_AT_ONCE),
            dir,
            t3code: self.t3code.clone(),
            sandbox: crate::harness::Sandbox {
                playbook: self.playbook_dir.clone(),
                path: self.sandbox_path.clone().unwrap_or_default(),
                allow_hosts: self.network_allow.clone().unwrap_or_default(),
                forwarder: None,
            },
        })
    }

    /// The host of `public_url`, as a Host header has it.
    pub fn public_host(&self) -> Option<String> {
        let url = self.public_url.as_deref()?;
        let rest = url.split_once("://").map_or(url, |(_, rest)| rest);
        let host = rest.split('/').next().unwrap_or(rest);
        (!host.is_empty()).then(|| host.to_string())
    }

    /// What `GET /api/settings` serves.
    pub fn web(&self) -> WebSettings {
        let d = WebSettings::default();
        WebSettings {
            web_animations: self.web_animations.unwrap_or(d.web_animations),
            web_animation_speed: self.web_animation_speed.unwrap_or(d.web_animation_speed),
            web_move_kingdom: self.web_move_kingdom.unwrap_or(d.web_move_kingdom),
            web_confirm_single: self.web_confirm_single.unwrap_or(d.web_confirm_single),
            web_background: self
                .web_background
                .clone()
                .filter(|b| {
                    b.is_empty() || b.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
                })
                .unwrap_or(d.web_background),
            web_sounds: self.web_sounds.unwrap_or(d.web_sounds),
            web_volume: self
                .web_volume
                .filter(|v| v.is_finite())
                .map_or(d.web_volume, |v| v.clamp(0.0, 1.0)),
            banned_cards: self.banned_cards.clone().unwrap_or_default(),
            registration: self.registration.unwrap_or(d.registration),
            guests: self.guests.unwrap_or(d.guests),
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
        self.server.clone().unwrap_or_else(|| {
            // A server listening on every address is reached through loopback.
            let host = match self.listen() {
                IpAddr::V4(ip) if ip.is_unspecified() => Ipv4Addr::LOCALHOST.into(),
                IpAddr::V6(ip) if ip.is_unspecified() => Ipv6Addr::LOCALHOST.into(),
                ip => ip,
            };
            format!("http://{}/mcp", SocketAddr::new(host, self.port()))
        })
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
    fn official_host_is_opt_in_and_independent_of_official_accounts() {
        assert!(
            crate::official_host::Host::from_config(&Config::default())
                .unwrap()
                .is_none()
        );
        let config: Config = toml::from_str("official_server_enabled = false\nofficial_server_catalog_file = \"/missing\"\nonline_enabled = false").unwrap();
        assert!(
            crate::official_host::Host::from_config(&config)
                .unwrap()
                .is_none()
        );
        let config: Config = toml::from_str(
            "official_server_enabled = true\nofficial_server_catalog_file = \"/missing\"",
        )
        .unwrap();
        let error = crate::official_host::Host::from_config(&config).unwrap_err();
        assert!(error.contains("/missing") && error.contains("dominion online sync"));
        let file = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(file.path(), b"untrusted bundle").unwrap();
        let config = Config {
            official_server_enabled: Some(true),
            official_server_catalog_file: Some(file.path().into()),
            ..Config::default()
        };
        assert!(
            crate::official_host::Host::from_config(&config)
                .unwrap_err()
                .contains("hash")
        );
    }

    #[test]
    fn official_accounts_keep_the_legacy_one_and_isolate_the_rest() {
        let config: Config = toml::from_str(
            r#"
            online_state_dir = "/state/online"
            online_enabled = true
            online_owner = "Arena Matches"
            online_account = "colonelpanic8"
            online_exclusive = true
            online_password_file = "/run/credentials/dominion.service/online-password"

            [[online_accounts]]
            id = "sol"
            account = "SolDominion"
            owner = "Arena Matches"
            password_file = "/run/credentials/dominion.service/online-password-sol"
            exclusive = true
            resume = true

            [[online_accounts]]
            id = "spare"
            account = "Spare"
            password_entry = "dominion.games/Spare"
            state_dir = "/elsewhere"
            enabled = false
            "#,
        )
        .unwrap();
        let all = config.online_accounts().unwrap();
        let summary: Vec<_> = (all.iter())
            .map(|a| {
                (
                    a.id.as_str(),
                    a.account.as_str(),
                    a.state_dir.to_str().unwrap(),
                    a.serve,
                )
            })
            .collect();
        assert_eq!(
            summary,
            [
                ("default", "colonelpanic8", "/state/online", true),
                ("sol", "SolDominion", "/state/online/accounts/sol", true),
                ("spare", "Spare", "/elsewhere", false),
            ]
        );
        let sol = config.online_account(Some("sol")).unwrap();
        assert!(sol.exclusive && sol.resume);
        assert_eq!(sol.owner.as_deref(), Some("Arena Matches"));
        assert_eq!(config.online_account(None).unwrap().id, "default");
        assert!(config.online_account(Some("nobody")).is_err());

        let only: Config = toml::from_str(
            "[[online_accounts]]\nid = \"sol\"\naccount = \"SolDominion\"\nstate_dir = \"/s\"",
        )
        .unwrap();
        assert_eq!(only.online_account(None).unwrap().id, "sol");
        assert!(Config::default().online_accounts().unwrap().is_empty());

        let error = |extra: &str| {
            toml::from_str::<Config>(&format!(
                "online_account = \"a\"\nonline_state_dir = \"/s\"\n{extra}"
            ))
            .unwrap()
            .online_accounts()
            .unwrap_err()
        };
        let entry = |id: &str, account: &str, dir: &str| {
            format!(
                "[[online_accounts]]\nid = \"{id}\"\naccount = \"{account}\"\nstate_dir = \"{dir}\"\n"
            )
        };
        assert!(error(&entry("Sol", "b", "/t")).contains("lowercase"));
        assert!(error(&entry("default", "b", "/t")).contains("lowercase"));
        assert!(error(&entry("../x", "b", "/t")).contains("lowercase"));
        assert!(error(&entry("b", "A", "/t")).contains("configured twice"));
        assert!(error(&entry("b", "b", "/s")).contains("state directory"));
        assert!(error(&(entry("b", "b", "/t") + &entry("b", "c", "/u"))).contains("duplicate"));
        assert!(
            toml::from_str::<Config>(
                "[[online_accounts]]\nid = \"x\"\naccount = \"x\"\npassword = \"p\""
            )
            .is_err()
        );
    }

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

    #[test]
    fn web_settings_default_and_stay_in_range() {
        let web = |toml: &str| toml::from_str::<Config>(toml).unwrap().web();
        assert_eq!(web(""), WebSettings::default());
        let set = web(
            "web_animations = false\nweb_animation_speed = \"swift\"\nweb_move_kingdom = false\n\
             web_confirm_single = true\nweb_background = \"dark-wood\"\nweb_sounds = false\n\
             web_volume = 0.2\nbanned_cards = [\"Witch\"]",
        );
        assert_eq!(
            set,
            WebSettings {
                web_animations: false,
                web_animation_speed: AnimationSpeed::Swift,
                web_move_kingdom: false,
                web_confirm_single: true,
                web_background: "dark-wood".into(),
                web_sounds: false,
                web_volume: 0.2,
                banned_cards: vec!["Witch".into()],
                registration: true,
                guests: true,
            }
        );
        assert_eq!(
            web("web_background = \"../x\"").web_background,
            "inheritance"
        );
        assert!(toml::from_str::<Config>("web_animation_speed = \"fast\"").is_err());
        assert_eq!(web("web_volume = 3.0").web_volume, 1.0);
        assert!(toml::from_str::<Config>("web_volume = \"loud\"").is_err());
    }

    #[test]
    fn the_web_dir_setting_wins_over_the_environment() {
        let env = || Some(std::ffi::OsString::from("/nix/store/web"));
        let set: Config = toml::from_str("web_dir = \"/srv/web\"").unwrap();
        assert_eq!(set.web_dir_or(env()), Some(PathBuf::from("/srv/web")));
        let unset = Config::default();
        assert_eq!(
            unset.web_dir_or(env()),
            Some(PathBuf::from("/nix/store/web"))
        );
        assert_eq!(unset.web_dir_or(Some("".into())), None);
        assert_eq!(unset.web_dir_or(None), None);
    }

    #[test]
    fn the_default_play_endpoint_is_where_the_server_listens() {
        let server = |toml: &str| toml::from_str::<Config>(toml).unwrap().server();
        assert_eq!(server(""), "http://127.0.0.1:7317/mcp");
        assert_eq!(
            server("listen = \"::1\"\nport = 9000"),
            "http://[::1]:9000/mcp"
        );
        assert_eq!(server("listen = \"::\""), "http://[::1]:7317/mcp");
        assert_eq!(server("listen = \"10.1.2.3\""), "http://10.1.2.3:7317/mcp");
        assert_eq!(
            server("listen = \"::1\"\nserver = \"http://box/mcp\""),
            "http://box/mcp"
        );
    }
}
