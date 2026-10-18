//! Shared state, built once at start-up.

use std::sync::Arc;

use crate::config::Settings;
use crate::events::Reader;
use crate::horizon::Horizon;
use crate::registry::Registry;

pub struct AppState {
    pub pool: sqlx::PgPool,
    pub settings: Settings,
    pub registry: Registry,
    pub horizon: Horizon,
    /// Reads the registry's events back off the chain — the ingest half of chain synchronisation.
    pub events: Reader,
    pub network: settle_chain::Network,
}

impl AppState {
    pub fn new(pool: sqlx::PgPool, settings: Settings) -> Arc<Self> {
        let registry = Registry::new(
            settings.registry_contract_id.clone(),
            settings.network_passphrase.clone(),
            settings.rpc_url.clone(),
        );
        let horizon = Horizon::new(settings.horizon_url.clone());
        let events = Reader::new(
            settings.rpc_url.clone(),
            settings.registry_contract_id.clone(),
        );
        let network = settings.network();

        Arc::new(Self {
            pool,
            settings,
            registry,
            horizon,
            events,
            network,
        })
    }
}

/// Unix seconds. The server clock drives countdowns and nothing else — expiry is decided by the
/// ledger.
pub fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_default()
}
