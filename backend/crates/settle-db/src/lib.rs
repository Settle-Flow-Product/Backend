//! # settle-db
//!
//! Every SQL statement in the system, one module per concern. No other crate writes SQL.
//!
//! Two conventions worth stating, because both are deliberate and both look odd at first:
//!
//! * **Runtime `sqlx::query`, never the `query!` macro.** The macro needs a live database at
//!   compile time or a checked-in cache that goes stale silently. A build that fails on a machine
//!   with no Postgres is a build that fails in CI for a reason nobody can act on.
//! * **Columns are read by name, never by tuple position.** Adding a column to a `SELECT` should
//!   not silently shift every field after it.
//!
//! Amounts are read as `text` and stay strings all the way to the wire. No floating point ever
//! touches money.
//!
//! The schema is `../../../docs/08-data-model.md`.

pub mod audit;
pub mod chain;
pub mod challenges;
pub mod error;
pub mod idempotency;
pub mod policy;
pub mod requests;
pub mod settlements;

pub use error::DbError;

use std::time::Duration;

use sqlx::postgres::PgPoolOptions;

/// The migrations, embedded at compile time.
///
/// `build.rs` re-runs the build when the directory changes, so a new migration is picked up
/// without a clean build.
pub static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../migrations");

/// Connect and bring the schema up to date.
///
/// Migrations run at start-up, before the port is bound. A backend that starts serving and then
/// discovers its schema is wrong reports itself healthy while being useless.
pub async fn connect(url: &str) -> Result<sqlx::PgPool, DbError> {
    let pool = PgPoolOptions::new()
        .max_connections(32)
        .acquire_timeout(Duration::from_secs(5))
        .connect(url)
        .await
        .map_err(DbError::classify)?;

    MIGRATOR.run(&pool).await?;
    Ok(pool)
}
