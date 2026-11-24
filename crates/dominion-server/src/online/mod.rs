//! The official-server backend has its own connection lifecycle and account.
//! Hosted Arena games continue to use `serve` and the local engine.
mod hosted;
mod mcp;
pub mod recordings;
mod runtime;
pub use hosted::{Hosted, router};

use crate::{config::Config, store::Store};
use dominion_online::{
    catalog::{BODY_URL, Catalog, VERSION},
    transport::{Endpoint, Message, ServerState, Transport, command},
};
use rmcp::ServiceExt;
use std::{path::PathBuf, sync::Arc, time::Duration};

pub use runtime::{Client, NewGame, PairTable};
#[cfg(test)]
pub(crate) use {mcp::Session, runtime::tests::fixture_client};

/// The id of the account the flat `online_*` settings configure.
pub const LEGACY_ID: &str = "default";

/// One official account, from the settings ([`Config::online_accounts`]).
#[derive(Clone, Debug, PartialEq)]
pub struct Account {
    pub id: String,
    pub account: String,
    pub owner: Option<String>,
    pub password_entry: Option<String>,
    pub password_file: Option<PathBuf>,
    pub exclusive: bool,
    pub resume: bool,
    pub endpoint: Option<String>,
    pub state_dir: PathBuf,
    /// Kept by `dominion serve`.
    pub serve: bool,
    /// The lowest dominion.games level its public searches match.
    pub min_opponent_level: Option<f64>,
}

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

fn configured_endpoint(setting: Option<&str>) -> Result<Option<Endpoint>, String> {
    match setting.unwrap_or("auto") {
        "alpha" => Ok(Some(Endpoint::Alpha)),
        "beta" => Ok(Some(Endpoint::Beta)),
        "auto" => Ok(None),
        other => Err(format!(
            "unknown online_endpoint {other:?}; use auto, alpha or beta"
        )),
    }
}

async fn auto_endpoint() -> Result<Endpoint, String> {
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
async fn probe_one(endpoint: Endpoint) -> Result<ServerState, String> {
    tokio::time::timeout(Duration::from_secs(5), async {
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
async fn start(config: &Config, account: &Account) -> Result<Client, String> {
    if !account.exclusive {
        return Err("REFUSED: set online_exclusive = true (exclusive = true in online_accounts) only while the account is idle and exclusive to this client; existing sessions are never removed".into());
    }
    let catalog = catalog(config)?;
    let state =
        Store::open(&account.state_dir).map_err(|e| format!("official client state lock: {e}"))?;
    let endpoint = configured_endpoint(account.endpoint.as_deref())?;
    let secret = match (&account.password_entry, &account.password_file) {
        (Some(entry), None) if !entry.is_empty() && !entry.starts_with('-') => {
            let output = tokio::process::Command::new("pass")
                .args(["show", entry])
                .output()
                .await
                .map_err(|e| format!("reading password-store entry: {e}"))?;
            if !output.status.success() {
                return Err("password-store lookup failed".into());
            }
            output.stdout
        }
        (None, Some(path)) => tokio::fs::read(path)
            .await
            .map_err(|e| format!("reading runtime password credential: {e}"))?,
        _ => return Err("set exactly one password entry or password file".into()),
    };
    let password = std::str::from_utf8(&secret)
        .map_err(|_| "password credential is not UTF-8")?
        .lines()
        .next()
        .filter(|s| !s.is_empty())
        .ok_or("password credential has no password")?;
    let result = Client::login(
        catalog,
        endpoint,
        &account.account,
        password,
        state,
        account.resume,
    )
    .await;
    drop(secret);
    result
}

/// How long the account joining a paired table may take to see its game start.
const PAIR_START: Duration = Duration::from_secs(30);

/// Starts a private unrated game between two accounts: `host` opens the table, `guest`
/// joins it, and once both see the game running it is theirs to play. A table that
/// didn't get that far is left by both.
pub(crate) async fn pair(host: &Client, guest: &Client, options: PairTable) -> Result<(), String> {
    let (host_id, guest_id) = (
        host.current().reducer.player_id,
        guest.current().reducer.player_id,
    );
    let timer = options.timer;
    let table = host.pair_open(guest_id, options).await?;
    let started: Result<(), String> = async {
        guest
            .pair_join(table, host_id, timer)
            .await
            .map_err(|e| format!("the other account could not join: {e}"))?;
        host.pair_start(guest_id).await?;
        running(guest, PAIR_START)
            .await
            .map_err(|e| format!("the other account: {e}"))?;
        let (host, guest) = (host.current(), guest.current());
        if host.paired
            && guest.paired
            && host.reducer.running
            && host.reducer.game_id.is_some()
            && host.reducer.game_id == guest.reducer.game_id
        {
            Ok(())
        } else {
            Err("the two accounts did not confirm the same running paired game".into())
        }
    }
    .await;
    if let Err(mut error) = started {
        let _ = tokio::join!(host.pair_abandon(), guest.pair_abandon());
        for (side, client) in [("host", host), ("guest", guest)] {
            let update = client.current();
            if update.paired
                && update.reducer.running
                && let Some(id) = update.reducer.game_id
            {
                error.push_str(&format!(". The {side} still reports paired game {id} running; check its management page before starting another game"));
            }
        }
        return Err(error);
    }
    Ok(())
}
/// Waits up to `limit` for `client` to see its game running.
async fn running(client: &Client, limit: Duration) -> Result<(), String> {
    let mut updates = client.updates.clone();
    let deadline = tokio::time::Instant::now() + limit;
    loop {
        let update = updates.borrow_and_update().clone();
        update.view()?;
        if update.reducer.running {
            return Ok(());
        }
        match tokio::time::timeout_at(deadline, updates.changed()).await {
            Err(_) => return Err("its game did not start".into()),
            Ok(Err(_)) => return Err("official client stopped".into()),
            Ok(Ok(())) => {}
        }
    }
}

/// What `pair_check` does once the game has started.
#[derive(Clone, Copy, Debug)]
pub enum PairCheckEnd {
    /// The host resigns after watching.
    Resign,
    /// Nobody moves: the game runs until dominion.games ends it, or the limit.
    Idle,
}

/// Logs in accounts `host` and `guest`, starts a private unrated game between them with
/// `options`, reports what each sees of it (the table, the timer, both clocks) every few
/// seconds for `watch`, then ends it as `end` says, and disconnects both. Nobody plays a
/// move: it checks the setup and dominion.games' clocks, not an agent.
pub async fn pair_check(
    config: &Config,
    host: &str,
    guest: &str,
    options: PairTable,
    watch: Duration,
    end: PairCheckEnd,
) -> Result<(), String> {
    let (host_account, guest_account) = (
        config.online_account(Some(host))?,
        config.online_account(Some(guest))?,
    );
    if host_account
        .account
        .eq_ignore_ascii_case(&guest_account.account)
    {
        return Err("choose two different official accounts".into());
    }
    let (host, guest) =
        tokio::try_join!(start(config, &host_account), start(config, &guest_account))?;
    let clients = [
        (&host_account.account, &host),
        (&guest_account.account, &guest),
    ];
    let result = pair_watched(&clients, options, watch, end).await;
    for (name, client) in clients {
        if let Err(e) = client.disconnect().await {
            eprintln!("{name}: disconnecting: {e}");
        }
    }
    result
}
async fn pair_watched(
    clients: &[(&String, &Client); 2],
    options: PairTable,
    watch: Duration,
    end: PairCheckEnd,
) -> Result<(), String> {
    for (name, client) in clients {
        let update = client.current();
        if update.view()?.is_some_and(|v| v.game.running) || update.searching {
            return Err(format!("{name} is already playing or searching"));
        }
    }
    let [(_, host), (_, guest)] = *clients;
    let began = std::time::Instant::now();
    pair(host, guest, options).await?;
    eprintln!("game started after {:.1}s", began.elapsed().as_secs_f64());
    let report = || {
        for (name, client) in clients {
            let update = client.current();
            let (clock, info) = update.clock_now();
            eprintln!(
                "  {name}: game {:?} seat {:?} running {} paired {} timer {:?}\n    clock {} info {}",
                update.reducer.game_id,
                update.reducer.seat(),
                update.reducer.running,
                update.paired,
                update.timer_settings,
                serde_json::to_string(&clock).unwrap_or_default(),
                serde_json::to_string(&info).unwrap_or_default(),
            );
        }
    };
    report();
    let watched = tokio::time::Instant::now() + watch;
    let over = |client: &Client| !client.current().reducer.running;
    while tokio::time::Instant::now() < watched && !clients.iter().any(|(_, c)| over(c)) {
        tokio::time::sleep_until(watched.min(tokio::time::Instant::now() + Duration::from_secs(5)))
            .await;
        eprintln!("after {:.0}s:", began.elapsed().as_secs_f64());
        report();
    }
    if matches!(end, PairCheckEnd::Idle) && clients.iter().all(|(_, c)| !over(c)) {
        for (_, client) in clients {
            client.current().view()?;
        }
        eprintln!("observation period ended while the game was still running");
        return Ok(());
    }
    if matches!(end, PairCheckEnd::Resign) && !over(host) {
        host.resign().await?;
    }
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    for (name, client) in clients {
        let mut updates = client.updates.clone();
        loop {
            let update = updates.borrow_and_update().clone();
            let view = update
                .view()?
                .ok_or_else(|| format!("{name}: game disappeared"))?;
            if let Some(result) = view.game.result {
                eprintln!(
                    "{name}: finished after {:.0}s, result {}",
                    began.elapsed().as_secs_f64(),
                    serde_json::to_string(&result).map_err(|e| e.to_string())?
                );
                break;
            }
            tokio::time::timeout_at(deadline, updates.changed())
                .await
                .map_err(|_| format!("{name}: no official result within 30 seconds"))?
                .map_err(|_| format!("{name}: client stopped before reporting the result"))?;
        }
    }
    Ok(())
}

/// Prints each configured account: id, official name, owner, served, state directory and
/// where its password comes from (never the password).
pub fn list(config: &Config) -> Result<(), String> {
    for a in config.online_accounts()? {
        let password = match (&a.password_entry, &a.password_file) {
            (Some(entry), None) => format!("pass {entry}"),
            (None, Some(file)) => format!("file {}", file.display()),
            _ => "UNSET: set exactly one password entry or file".into(),
        };
        println!(
            "{}\t{}\towner={}\tserve={}\texclusive={}\tresume={}\tstate={}\tpassword={password}",
            a.id,
            a.account,
            a.owner.as_deref().unwrap_or("-"),
            a.serve,
            a.exclusive,
            a.resume,
            a.state_dir.display(),
        );
    }
    Ok(())
}

pub async fn login_check(config: &Config, id: Option<&str>) -> Result<(), String> {
    let client = start(config, &config.online_account(id)?).await?;
    eprintln!("dominion: authenticated; checking application heartbeat for 31 seconds");
    client.idle_check(Duration::from_secs(31)).await?;
    client.disconnect().await?;
    eprintln!("dominion: login and heartbeat verified; disconnected without changing any game");
    Ok(())
}

pub async fn serve(config: &Config, id: Option<&str>) -> Result<(), String> {
    let client = start(config, &config.online_account(id)?).await?;
    let service = mcp::Session::new(client.clone())
        .serve(rmcp::transport::stdio())
        .await
        .map_err(|e| e.to_string())?;
    let result = service.waiting().await.map_err(|e| e.to_string());
    let _ = client.disconnect().await;
    result.map(|_| ())
}
