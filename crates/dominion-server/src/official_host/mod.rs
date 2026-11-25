//! Fixed-seat Arena transport using the pinned dominion.games gameplay layouts.

pub(crate) mod catalog;
pub(crate) mod identity;
pub(crate) mod lifecycle;
pub(crate) mod preferences;
pub(crate) mod profile;
pub(crate) mod projection;
pub(crate) mod questions;
pub(crate) mod session;

use crate::{config::Config, limits::Limiter};
use dominion_online::catalog::Catalog;
use rand::Rng;
use std::{
    collections::HashMap,
    net::IpAddr,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tokio::sync::Semaphore;

/// Validated catalog and bounded connection admission for the optional fixed-seat route.
pub struct Host {
    pub(crate) catalog: catalog::WireCatalog,
    pub(crate) upgrades: Limiter<IpAddr>,
    pub(crate) connections: Arc<Semaphore>,
    pub(crate) root_socket: bool,
    pub(crate) unreviewed_cards: bool,
    pub(crate) preferences: preferences::Store,
    codes: Mutex<HashMap<String, (Instant, String)>>,
    launches: Limiter<String>,
}

impl std::fmt::Debug for Host {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Host").finish_non_exhaustive()
    }
}

impl Host {
    /// Explicit catalog injection also lets offline tests use a synthetic catalog.
    pub fn new(catalog: Arc<Catalog>) -> Result<Arc<Self>, String> {
        Ok(Arc::new(Self {
            catalog: catalog::WireCatalog::new(catalog).map_err(|e| e.to_string())?,
            upgrades: Limiter::new(30, Duration::from_secs(60)),
            connections: Arc::new(Semaphore::new(256)),
            root_socket: true,
            unreviewed_cards: false,
            preferences: preferences::Store::new(None),
            codes: Mutex::default(),
            launches: Limiter::new(10, Duration::from_secs(60)),
        }))
    }

    pub fn compatibility_report(
        &self,
        config: &dominion_engine::GameConfig,
        meta: &crate::hub::Meta,
    ) -> serde_json::Value {
        serde_json::to_value(profile::assess_setup(config, meta, &self.catalog))
            .expect("compatibility report is JSON")
    }

    pub fn coverage_report(&self) -> serde_json::Value {
        serde_json::to_value(profile::coverage_report(&self.catalog))
            .expect("coverage report is JSON")
    }

    pub(crate) fn issue_code(
        &self,
        token: &str,
        arena: &crate::arena::Arena,
    ) -> Result<String, String> {
        let ident = arena
            .identify(token)
            .ok_or("Arena authentication required")?;
        self.launches.sweep();
        self.launches
            .take(ident.user)
            .map_err(|_| "too many launch-code requests")?;
        let mut codes = crate::hub::lock(&self.codes);
        codes.retain(|_, (expires, _)| *expires > Instant::now());
        if codes.len() >= 256 {
            return Err("too many pending launch codes".into());
        }
        let bytes: [u8; 32] = rand::rng().random();
        let code = bytes.iter().map(|b| format!("{b:02x}")).collect::<String>();
        codes.insert(
            code.clone(),
            (Instant::now() + Duration::from_secs(30), token.to_string()),
        );
        Ok(code)
    }

    pub(crate) fn redeem(&self, code: &str, arena: &crate::arena::Arena) -> Option<String> {
        let (expires, token) = crate::hub::lock(&self.codes).remove(code)?;
        (expires > Instant::now() && arena.identify(&token).is_some()).then_some(token)
    }

    /// Loading never logs in or downloads a bundle. Disabled servers need no catalog.
    pub fn from_config(config: &Config) -> Result<Option<Arc<Self>>, String> {
        if !config.official_server_enabled.unwrap_or(false) {
            return Ok(None);
        }
        let path = match &config.official_server_catalog_file {
            Some(path) => path.clone(),
            None => crate::online::cache_file(config)?,
        };
        let bytes = std::fs::read(&path).map_err(|e| format!(
            "official_server_catalog_file {}: {e}; run dominion online sync or set the file path",
            path.display(),
        ))?;
        let catalog = Catalog::from_bundle(&bytes)
            .map_err(|e| format!("official_server_catalog_file {}: {e}", path.display(),))?;
        let mut host = Self::new(Arc::new(catalog))?;
        let host_mut = Arc::get_mut(&mut host).expect("new host is unshared");
        host_mut.root_socket = config.official_server_root_socket.unwrap_or(false);
        host_mut.unreviewed_cards = config.official_server_unreviewed_cards.unwrap_or(false);
        let dir = config.official_preferences_dir.clone().or_else(|| {
            let root = std::env::var_os("XDG_CONFIG_HOME")
                .map(std::path::PathBuf::from)
                .or_else(|| {
                    std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".config"))
                })?;
            Some(root.join("dominion/official-preferences"))
        });
        host_mut.preferences = preferences::Store::new(dir);
        if config.official_client_assets_dir.is_some() {
            return Err("official_client_assets_dir requires a verified complete original distribution; no graphical launcher is available yet".into());
        }
        Ok(Some(host))
    }
}
