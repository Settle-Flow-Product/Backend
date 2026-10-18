//! Wallet challenges — stored so they can be single use.

use chrono::{DateTime, Utc};
use sqlx::{PgExecutor, Row};

use crate::error::DbError;

#[derive(Debug, Clone)]
pub struct ChallengeRow {
    pub nonce: Vec<u8>,
    pub address: String,
    pub network: String,
    pub domain: String,
    pub issued_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}

pub async fn issue<'c, E: PgExecutor<'c>>(
    conn: E,
    nonce: &[u8; 32],
    address: &str,
    network: &str,
    domain: &str,
    ttl_seconds: i64,
) -> Result<ChallengeRow, DbError> {
    let row = sqlx::query(
        r#"
        INSERT INTO auth_challenges (nonce, address, network, domain, expires_at)
        VALUES ($1, $2, $3, $4, now() + ($5::int * interval '1 second'))
        RETURNING nonce, address, network, domain, issued_at, expires_at
        "#,
    )
    .bind(nonce.as_slice())
    .bind(address)
    .bind(network)
    .bind(domain)
    // An integer count of seconds. `make_interval(secs => …)` wants a double, and a float
    // has no business in this codebase even where it would be harmless.
    .bind(i32::try_from(ttl_seconds).unwrap_or(300))
    .fetch_one(conn)
    .await
    .map_err(DbError::classify)?;

    Ok(ChallengeRow {
        nonce: row.get("nonce"),
        address: row.get("address"),
        network: row.get("network"),
        domain: row.get("domain"),
        issued_at: row.get("issued_at"),
        expires_at: row.get("expires_at"),
    })
}

/// Read a challenge **and delete it**, in one statement.
///
/// This is what makes a nonce single use. A `SELECT` followed by a `DELETE` has a window between
/// them, and a captured challenge replayed into that window authenticates twice. `DELETE …
/// RETURNING` has no window.
pub async fn take<'c, E: PgExecutor<'c>>(
    conn: E,
    nonce: &[u8; 32],
) -> Result<ChallengeRow, DbError> {
    let row = sqlx::query(
        r#"
        DELETE FROM auth_challenges WHERE nonce = $1
        RETURNING nonce, address, network, domain, issued_at, expires_at
        "#,
    )
    .bind(nonce.as_slice())
    .fetch_optional(conn)
    .await
    .map_err(DbError::classify)?
    .ok_or(DbError::NotFound)?;

    Ok(ChallengeRow {
        nonce: row.get("nonce"),
        address: row.get("address"),
        network: row.get("network"),
        domain: row.get("domain"),
        issued_at: row.get("issued_at"),
        expires_at: row.get("expires_at"),
    })
}

/// Remove lapsed challenges. They are swept, never extended — a challenge that can be renewed is
/// a challenge with no expiry.
pub async fn sweep<'c, E: PgExecutor<'c>>(conn: E) -> Result<u64, DbError> {
    let r = sqlx::query("DELETE FROM auth_challenges WHERE expires_at <= now()")
        .execute(conn)
        .await
        .map_err(DbError::classify)?;
    Ok(r.rows_affected())
}
