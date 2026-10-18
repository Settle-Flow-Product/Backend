//! Duplicate protection — feature F-04.

use sqlx::{PgExecutor, Row};
use uuid::Uuid;

use crate::error::DbError;

/// What happened when a caller tried to claim a key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Claim {
    /// First time. Carry on.
    Claimed,
    /// Already completed. Return the original result rather than doing the work again.
    AlreadyDone { result_ref: String },
    /// Someone else is doing it right now. The caller waits and polls **with the same key** —
    /// generating a new one is how a duplicate gets created.
    InFlight,
    /// The same key with a different payload. An error, because silently returning somebody else's
    /// result is worse than refusing.
    BodyMismatch,
}

/// Claim a key before doing any work.
///
/// The claim happens *first*, before anything is written anywhere else. `ON CONFLICT DO NOTHING`
/// makes the race resolve in the database, where it is decided once, rather than in application
/// code, where two instances would each believe they won.
pub async fn claim(
    pool: &sqlx::PgPool,
    scope: &str,
    key: Uuid,
    body_hash: &[u8; 32],
) -> Result<Claim, DbError> {
    let inserted = sqlx::query(
        r#"
        INSERT INTO idempotency_keys (scope, key, body_hash, state)
        VALUES ($1, $2, $3, 'in_flight')
        ON CONFLICT (scope, key) DO NOTHING
        RETURNING key
        "#,
    )
    .bind(scope)
    .bind(key)
    .bind(body_hash.as_slice())
    .fetch_optional(pool)
    .await
    .map_err(DbError::classify)?;

    if inserted.is_some() {
        return Ok(Claim::Claimed);
    }

    let existing = sqlx::query(
        "SELECT body_hash, state::text AS state, result_ref FROM idempotency_keys WHERE scope=$1 AND key=$2",
    )
    .bind(scope)
    .bind(key)
    .fetch_optional(pool)
    .await
    .map_err(DbError::classify)?
    .ok_or(DbError::NotFound)?;

    let stored: Vec<u8> = existing.get("body_hash");
    if stored != body_hash.as_slice() {
        return Ok(Claim::BodyMismatch);
    }

    let state: String = existing.get("state");
    match state.as_str() {
        "done" => Ok(Claim::AlreadyDone {
            result_ref: existing
                .get::<Option<String>, _>("result_ref")
                .unwrap_or_default(),
        }),
        _ => Ok(Claim::InFlight),
    }
}

/// The work succeeded. Record where the result lives so a retry can return it.
pub async fn finish<'c, E: PgExecutor<'c>>(
    conn: E,
    scope: &str,
    key: Uuid,
    result_ref: &str,
) -> Result<(), DbError> {
    sqlx::query(
        "UPDATE idempotency_keys SET state='done', result_ref=$3 WHERE scope=$1 AND key=$2",
    )
    .bind(scope)
    .bind(key)
    .bind(result_ref)
    .execute(conn)
    .await
    .map_err(DbError::classify)?;
    Ok(())
}

/// The work failed definitively. Release the key so the caller may try again.
///
/// **Only for definitive failures.** An unresolved outcome keeps its claim, because releasing a
/// key while the original attempt may still land is how a duplicate is created.
pub async fn release<'c, E: PgExecutor<'c>>(
    conn: E,
    scope: &str,
    key: Uuid,
) -> Result<(), DbError> {
    sqlx::query("UPDATE idempotency_keys SET state='released' WHERE scope=$1 AND key=$2")
        .bind(scope)
        .bind(key)
        .execute(conn)
        .await
        .map_err(DbError::classify)?;
    Ok(())
}
