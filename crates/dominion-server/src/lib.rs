//! `dominion`: an MCP server hosting Dominion games over streamable HTTP, a terminal
//! client for it, and solo games kept in files.

pub mod anthropic;
pub mod archive;
pub mod arena;
pub mod assets;
pub mod backfill;
pub mod catalog;
pub mod client;
mod codex_usage;
pub mod config;
pub mod control;
pub mod harness;
pub mod house;
pub mod http;
pub mod hub;
pub mod limits;
pub mod mcp;
pub mod official_host;
pub mod online;
pub mod openai;
pub mod rating;
pub mod runner;
pub mod setup_cards;
pub mod solo;
pub mod steps;
pub mod store;
pub mod t3;
mod transcript;

/// The commit this build is from, where that is known.
pub fn commit() -> Option<String> {
    let commit = env!("DOMINION_COMMIT");
    (!commit.is_empty()).then(|| commit.to_string())
}

use dominion_agent::GlossaryEntry;
use rmcp::transport::streamable_http_server::{
    StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;

/// How often forfeits, departures and matches are looked for.
const SWEEP: Duration = Duration::from_secs(5);

/// How [`serve`] serves.
#[derive(Clone, Debug)]
pub struct Options {
    /// Accepted in the Host header besides loopback.
    pub allowed_hosts: Vec<String>,
    /// How long an MCP session may go without a call before it is closed, and the seat
    /// it joined forgotten. `None` keeps sessions until the server stops.
    pub session_idle: Option<Duration>,
    /// The asset cache, served at `/assets`.
    pub assets_dir: Option<PathBuf>,
    /// The built web client, served at `/`.
    pub web_dir: Option<PathBuf>,
    /// What `GET /api/settings` tells the web client.
    pub web: dominion_protocol::api::WebSettings,
    /// Accounts and the lobby.
    pub arena: arena::Settings,
    /// The URL people reach the server at, when a proxy stands in front of it.
    pub public_url: Option<String>,
    /// A client's address is the last one in `X-Forwarded-For`.
    pub trust_proxy: bool,
    /// Where a runner of the server's own agents connects ([`house`]).
    pub agent_socket: Option<PathBuf>,
    /// Where `dominion games` connects ([`control`]).
    pub control_socket: Option<PathBuf>,
    /// The official accounts kept connected.
    pub online: Vec<online::Hosted>,
    pub official_server: Option<Arc<official_host::Host>>,
    /// What dominion.games draws on each card, by name ([`online::online_cards`]).
    pub online_cards: Arc<online::OnlineCards>,
}

impl Default for Options {
    fn default() -> Options {
        Options {
            allowed_hosts: vec![],
            session_idle: config::Config::default().session_idle(),
            assets_dir: None,
            web_dir: None,
            web: Default::default(),
            arena: Default::default(),
            public_url: None,
            trust_proxy: false,
            agent_socket: None,
            control_socket: None,
            online: Vec::new(),
            official_server: None,
            online_cards: Default::default(),
        }
    }
}

/// Serves the MCP endpoint at `/mcp`, the HTTP API at `/api`, the asset cache at
/// `/assets` and the web client at `/` on `listener` until `stop` is cancelled.
pub async fn serve(
    listener: TcpListener,
    hub: Arc<hub::Hub>,
    options: Options,
    stop: CancellationToken,
) -> std::io::Result<()> {
    let mut hosts: Vec<String> = ["localhost", "127.0.0.1", "::1"]
        .into_iter()
        .map(String::from)
        .collect();
    hosts.extend(options.allowed_hosts.iter().cloned());
    let arena = arena::Arena::open(hub, options.arena.clone())?;
    if let Some(online) = options.online.iter().find(|online| {
        !arena
            .profile(&online.owner)
            .is_ok_and(|p| p.user.kind == dominion_protocol::lobby::UserKind::Registered)
    }) {
        let id = online.id.clone();
        for online in &options.online {
            let _ = online.client.disconnect().await;
        }
        return Err(std::io::Error::other(format!(
            "the owner of official account {id} must name an existing registered Arena account"
        )));
    }
    let site = http::router(arena.clone(), &options, hosts.clone(), stop.clone());
    if let Some(path) = &options.agent_socket {
        let (arena, path, stop) = (arena.clone(), path.clone(), stop.clone());
        tokio::spawn(async move {
            if let Err(e) = house::listen(arena, &path, stop).await {
                eprintln!("dominion: agent socket {}: {e}", path.display());
            }
        });
    }
    if let Some(path) = &options.control_socket {
        let (arena, path, stop) = (arena.clone(), path.clone(), stop.clone());
        tokio::spawn(async move {
            if let Err(e) = control::listen(arena, &path, stop).await {
                eprintln!("dominion: control socket {}: {e}", path.display());
            }
        });
    }
    let (sweeper, stopped) = (arena.clone(), stop.clone());
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(SWEEP);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                _ = stopped.cancelled() => return,
                _ = tick.tick() => {}
            }
            let arena = sweeper.clone();
            let _ = tokio::task::spawn_blocking(move || arena.sweep()).await;
        }
    });
    let config = StreamableHttpServerConfig::default()
        .with_allowed_hosts(hosts.clone())
        .with_cancellation_token(stop.child_token());
    let mut sessions = LocalSessionManager::default();
    sessions.session_config.keep_alive = options.session_idle;
    // A call in progress doesn't keep its session alive, so no wait may outlast it.
    let max_wait = options.session_idle.map_or(mcp::MAX_WAIT, |idle| idle / 2);
    let trust_proxy = options.trust_proxy;
    let online = (!options.online.is_empty()).then(|| {
        online::router(
            &options.online,
            arena.clone(),
            hosts.clone(),
            options.session_idle,
            stop.clone(),
        )
    });
    let service = StreamableHttpService::new(
        move || Ok(mcp::Session::new(arena.clone(), max_wait, trust_proxy)),
        Arc::new(sessions),
        config,
    );
    let router = axum::Router::new()
        .nest_service("/mcp", service)
        .merge(site);
    let router = if let Some(online) = online {
        router.merge(online)
    } else {
        router
    };
    let service = router.into_make_service_with_connect_info::<std::net::SocketAddr>();
    let result = axum::serve(listener, service)
        .with_graceful_shutdown(async move { stop.cancelled_owned().await })
        .await;
    for online in options.online {
        let _ = online.client.disconnect().await;
    }
    result
}

/// A glossary entry as plain text.
pub fn card_text(entry: &GlossaryEntry) -> String {
    let mut head = vec![entry.name.clone(), entry.types.join(" - ")];
    if let Some(cost) = &entry.cost {
        let cost = dominion_engine::Cost {
            coins: cost.coins as u8,
            potions: cost.potions as u8,
            debt: cost.debt as u8,
        };
        head.push(cost.to_string());
    }
    if let Some(set) = &entry.set {
        head.push(set.clone());
    }
    format!("{}\n{}", head.join(" | "), entry.text)
}
