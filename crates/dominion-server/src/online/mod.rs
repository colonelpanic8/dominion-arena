//! The official-server backend has its own connection lifecycle and account.
//! Hosted Arena games continue to use `serve` and the local engine.
mod mcp;
mod runtime;

use crate::{config::Config, store::Store};
use dominion_online::{
    catalog::{BODY_URL, Catalog, VERSION},
    transport::{Endpoint, Message, ServerState, Transport, command},
};
use rmcp::ServiceExt;
use std::{path::PathBuf, sync::Arc, time::Duration};

pub use runtime::{Client, NewGame};

pub fn cache_file(config: &Config) -> Result<PathBuf, String> {
    Ok(config
        .online_cache_dir()
        .ok_or("set online_cache_dir or HOME")?
        .join(format!("body-{VERSION}.js")))
}

pub async fn sync(config: &Config) -> Result<(), String> {
    let path = cache_file(config)?;
    let bytes = tokio::task::spawn_blocking(|| {
        ureq::get(BODY_URL)
            .call()
            .map_err(|e| e.to_string())?
            .body_mut()
            .with_config()
            .limit(8 * 1024 * 1024)
            .read_to_vec()
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())??;
    Catalog::from_bundle(&bytes).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
    std::fs::write(&path, &bytes).map_err(|e| e.to_string())?;
    eprintln!(
        "dominion: verified official client {VERSION} catalog at {}",
        path.display()
    );
    Ok(())
}

fn catalog(config: &Config) -> Result<Arc<Catalog>, String> {
    let path = cache_file(config)?;
    let bytes = std::fs::read(&path)
        .map_err(|e| format!("{}: {e}; run dominion online sync", path.display()))?;
    Catalog::from_bundle(&bytes)
        .map(Arc::new)
        .map_err(|e| e.to_string())
}

async fn endpoint(config: &Config) -> Result<Endpoint, String> {
    match config.online_endpoint.as_deref().unwrap_or("auto") {
        "alpha" => Ok(Endpoint::Alpha),
        "beta" => Ok(Endpoint::Beta),
        "auto" => {
            let mut errors = Vec::new();
            for endpoint in [Endpoint::Alpha, Endpoint::Beta] {
                match probe_one(endpoint).await {
                    Ok(ServerState::Normal) => return Ok(endpoint),
                    Ok(state) => errors.push(format!("{endpoint:?}: {state:?}")),
                    Err(e) => errors.push(e),
                }
            }
            Err(format!(
                "no official server accepting new clients: {}",
                errors.join("; ")
            ))
        }
        other => Err(format!(
            "unknown online_endpoint {other:?}; use auto, alpha or beta"
        )),
    }
}
async fn probe_one(endpoint: Endpoint) -> Result<ServerState, String> {
    tokio::time::timeout(Duration::from_secs(15), async {
        let mut socket = Transport::connect_probe(endpoint)
            .await
            .map_err(|e| e.to_string())?;
        socket
            .send(command::server_state(VERSION, None))
            .await
            .map_err(|e| e.to_string())?;
        let messages = socket.receive().await.map_err(|e| e.to_string())?;
        let _ = socket.disconnect().await;
        messages
            .into_iter()
            .find_map(|m| {
                if let Message::ServerState(s) = m {
                    Some(s)
                } else {
                    None
                }
            })
            .ok_or_else(|| "probe returned no server state".into())
    })
    .await
    .map_err(|_| "official server probe timed out".to_string())?
}
/// An unauthenticated server-state request; no browser or saved authority is used.
pub async fn probe() -> Result<(), String> {
    for endpoint in [Endpoint::Alpha, Endpoint::Beta] {
        eprintln!("{endpoint:?}: {:?}", probe_one(endpoint).await?);
    }
    Ok(())
}
async fn start(config: &Config) -> Result<Client, String> {
    if config.online_exclusive != Some(true) {
        return Err("REFUSED: set online_exclusive = true only while the account is idle and exclusive to this client; existing sessions are never removed".into());
    }
    let account = config.online_account.clone().ok_or("set online_account")?;
    let entry = config
        .online_password_entry
        .clone()
        .ok_or("set online_password_entry to an existing pass entry")?;
    if entry.starts_with('-') || entry.is_empty() {
        return Err("invalid online_password_entry".into());
    }
    let catalog = catalog(config)?;
    let state = Store::open(
        &config
            .online_state_dir()
            .ok_or("set online_state_dir or HOME")?,
    )
    .map_err(|e| format!("official client state lock: {e}"))?;
    let endpoint = endpoint(config).await?;
    let output = tokio::process::Command::new("pass")
        .args(["show", &entry])
        .output()
        .await
        .map_err(|e| format!("reading password-store entry: {e}"))?;
    if !output.status.success() {
        return Err("password-store lookup failed".into());
    }
    let password = std::str::from_utf8(&output.stdout)
        .map_err(|_| "password-store entry is not UTF-8")?
        .lines()
        .next()
        .filter(|s| !s.is_empty())
        .ok_or("password-store entry has no password")?;
    let result = Client::login(
        catalog,
        endpoint,
        &account,
        password,
        state,
        config.online_resume.unwrap_or(false),
    )
    .await;
    drop(output);
    result
}

pub async fn login_check(config: &Config) -> Result<(), String> {
    let client = start(config).await?;
    eprintln!("dominion: authenticated; checking application heartbeat for 31 seconds");
    client.idle_check(Duration::from_secs(31)).await?;
    client.disconnect().await?;
    eprintln!("dominion: login and heartbeat verified; disconnected without changing any game");
    Ok(())
}

pub async fn serve(config: &Config) -> Result<(), String> {
    let client = start(config).await?;
    let service = mcp::Session::new(client.clone())
        .serve(rmcp::transport::stdio())
        .await
        .map_err(|e| e.to_string())?;
    let result = service.waiting().await.map_err(|e| e.to_string());
    let _ = client.disconnect().await;
    result.map(|_| ())
}
