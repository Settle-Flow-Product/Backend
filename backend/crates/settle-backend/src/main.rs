//! # settle-backend
//!
//! The SettleFlow SOW 1 service. One binary, seven crates, one database.
//!
//! The start-up order below is load-bearing: everything that can fail is given the chance to fail
//! **before** the port is bound. A backend that starts serving and then discovers it cannot reach
//! its database reports itself healthy while being useless, and a load balancer believes it.

mod config;
mod events;
mod horizon;
mod http;
mod registry;
mod state;
mod tasks;

use std::process::ExitCode;

use tracing_subscriber::EnvFilter;

#[derive(Debug, thiserror::Error)]
enum StartupError {
    #[error(transparent)]
    Config(#[from] config::ConfigError),
    #[error("could not reach the database: {0}")]
    Database(#[from] settle_db::DbError),
    #[error("could not bind {bind}: {source}")]
    Bind {
        bind: String,
        #[source]
        source: std::io::Error,
    },
    #[error("the server stopped: {0}")]
    Serve(std::io::Error),
}

#[tokio::main]
async fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| "info,sqlx=warn".into()),
        )
        .init();

    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            // Also printed plainly: a container log reader may not have the filter set, and a
            // boot failure nobody can read is a boot failure nobody can fix.
            eprintln!("settle-backend failed to start: {e}");
            tracing::error!(error = %e, "startup failed");
            ExitCode::FAILURE
        }
    }
}

async fn run() -> Result<(), StartupError> {
    // 1 · configuration. Half-configured fails here, with a sentence naming what is missing.
    let settings = config::Settings::from_env()?;
    tracing::info!(
        network = %settings.network_passphrase,
        registry = %settings.registry_contract_id,
        treasury = %settings.treasury_address,
        "configuration loaded"
    );

    // 2 · database and migrations, before anything else.
    let pool = settle_db::connect(&settings.database_url).await?;
    tracing::info!("database connected and migrations applied");

    let bind = settings.bind.clone();
    let state = state::AppState::new(pool, settings);

    // 3 · report what the chain says, without making it fatal.
    //
    // The registry being unreachable at boot is worth a loud warning, not a refusal to start: the
    // read paths still work, and a transient RPC outage should not keep the service down.
    match state
        .horizon
        .sequence(&state.settings.treasury_address)
        .await
    {
        Ok(seq) => tracing::info!(sequence = seq, "treasury reachable"),
        Err(e) => tracing::warn!(error = %e,
            "the treasury is not reachable — read paths will work, payments will not"),
    }

    // 4 · background loops.
    let supervisor = tasks::spawn_all(state.clone());

    // 5 · bind, last.
    let listener = tokio::net::TcpListener::bind(&bind)
        .await
        .map_err(|source| StartupError::Bind {
            bind: bind.clone(),
            source,
        })?;
    tracing::info!(%bind, "listening");

    axum::serve(listener, http::router(state))
        .with_graceful_shutdown(shutdown())
        .await
        .map_err(StartupError::Serve)?;

    // 6 · let the loops finish what they are doing.
    supervisor.shutdown().await;
    tracing::info!("stopped cleanly");
    Ok(())
}

/// Stop on SIGINT or SIGTERM. A submit in flight gets to finish rather than being killed
/// mid-write, which is what keeps `submit_attempted_at` meaningful across a restart.
async fn shutdown() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };

    #[cfg(unix)]
    let terminate = async {
        if let Ok(mut sig) =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        {
            sig.recv().await;
        }
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
    tracing::info!("shutdown signal received");
}
