//! `dominion`: an MCP server hosting Dominion games over streamable HTTP, and a terminal
//! client for it.

pub mod assets;
pub mod client;
pub mod config;
pub mod http;
pub mod hub;
pub mod mcp;

use dominion_agent::GlossaryEntry;
use rmcp::transport::streamable_http_server::{
    StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;

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
    /// The settings file the web client's edits are written to.
    pub config_file: Option<PathBuf>,
}

impl Default for Options {
    fn default() -> Options {
        Options {
            allowed_hosts: vec![],
            session_idle: config::Config::default().session_idle(),
            assets_dir: None,
            web_dir: None,
            web: Default::default(),
            config_file: None,
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
    let site = http::router(hub.clone(), &options, hosts.clone(), stop.clone());
    let config = StreamableHttpServerConfig::default()
        .with_allowed_hosts(hosts)
        .with_cancellation_token(stop.child_token());
    let mut sessions = LocalSessionManager::default();
    sessions.session_config.keep_alive = options.session_idle;
    // A call in progress doesn't keep its session alive, so no wait may outlast it.
    let max_wait = options.session_idle.map_or(mcp::MAX_WAIT, |idle| idle / 2);
    let service = StreamableHttpService::new(
        move || Ok(mcp::Session::new(hub.clone(), max_wait)),
        Arc::new(sessions),
        config,
    );
    let router = axum::Router::new()
        .nest_service("/mcp", service)
        .merge(site);
    axum::serve(listener, router)
        .with_graceful_shutdown(async move { stop.cancelled_owned().await })
        .await
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
