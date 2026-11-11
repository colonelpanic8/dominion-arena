//! `dominion`: an MCP server hosting Dominion games over streamable HTTP, and a terminal
//! client for it.

pub mod client;
pub mod config;
pub mod hub;
pub mod mcp;

use dominion_agent::GlossaryEntry;
use rmcp::transport::streamable_http_server::{
    StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
};
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;

/// Serves the MCP endpoint at `/mcp` on `listener` until `stop` is cancelled.
/// `allowed_hosts` are accepted in the Host header besides loopback.
pub async fn serve(
    listener: TcpListener,
    hub: Arc<hub::Hub>,
    allowed_hosts: Vec<String>,
    stop: CancellationToken,
) -> std::io::Result<()> {
    let mut hosts: Vec<String> = ["localhost", "127.0.0.1", "::1"]
        .into_iter()
        .map(String::from)
        .collect();
    hosts.extend(allowed_hosts);
    let config = StreamableHttpServerConfig::default()
        .with_allowed_hosts(hosts)
        .with_cancellation_token(stop.child_token());
    let service = StreamableHttpService::new(
        move || Ok(mcp::Session::new(hub.clone())),
        Arc::new(LocalSessionManager::default()),
        config,
    );
    let router = axum::Router::new().nest_service("/mcp", service);
    axum::serve(listener, router)
        .with_graceful_shutdown(async move { stop.cancelled_owned().await })
        .await
}

/// A glossary entry as plain text.
pub fn card_text(entry: &GlossaryEntry) -> String {
    let mut head = vec![entry.name.clone(), entry.types.join(" - ")];
    if let Some(cost) = &entry.cost {
        let mut c = format!("${}", cost.coins);
        if cost.potions > 0 {
            c.push_str(&"P".repeat(cost.potions as usize));
        }
        if cost.debt > 0 {
            c.push_str(&format!(" {}D", cost.debt));
        }
        head.push(c);
    }
    if let Some(set) = &entry.set {
        head.push(set.clone());
    }
    format!("{}\n{}", head.join(" | "), entry.text)
}
