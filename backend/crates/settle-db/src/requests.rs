//! Payment requests — the off-chain copy.

use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::{PgExecutor, Row};
use uuid::Uuid;

use crate::error::DbError;

/// One request, as the interface needs it.
#[derive(Debug, Clone, Serialize)]
pub struct RequestRow {
    /// Hex-encoded. The same 32 bytes as the id on chain.
    pub id: String,
    /// The BUSINESS the request belongs to — the `creator` the contract authorises as.
    pub creator_address: String,
    /// The PERSON who raised it: the wallet that was signed in. `None` for rows written before
    /// this was recorded.
    pub created_by: Option<String>,
    /// The trace id of the interface action that raised this, carried through to the chain write.
    pub trace_id: Option<String>,
    pub payee_address: String,
    pub asset_code: String,
    pub asset_contract: String,
    /// A string on the wire. JSON numbers are IEEE doubles.
    pub amount: String,
    pub reference: String,
    pub expiry_at: DateTime<Utc>,
    pub status: String,
    pub contract_id: String,
    pub create_tx_hash: Option<String>,
    pub created_at: DateTime<Utc>,
}

/// What is needed to record a new request.
#[derive(Debug, Clone)]
pub struct NewRequestRow {
    pub id: [u8; 32],
    pub creator_address: String,
    pub created_by: Option<String>,
    pub trace_id: Option<String>,
    pub payee_address: String,
    pub asset_code: String,
    pub asset_contract: String,
    pub amount: String,
    pub reference: String,
    pub reference_hash: [u8; 32],
    pub expiry_at: DateTime<Utc>,
    pub contract_id: String,
    pub idempotency_key: Uuid,
}

fn row_to_request(row: &sqlx::postgres::PgRow) -> RequestRow {
    let id: Vec<u8> = row.get("id");
    RequestRow {
        id: hex::encode(id),
        creator_address: row.get("creator_address"),
        created_by: row.get("created_by"),
        trace_id: row.get("trace_id"),
        payee_address: row.get("payee_address"),
        asset_code: row.get("asset_code"),
        asset_contract: row.get("asset_contract"),
        // numeric is read as text so no floating point ever touches an amount.
        amount: row.get("amount"),
        reference: row.get("reference"),
        expiry_at: row.get("expiry_at"),
        status: row.get("status"),
        contract_id: row.get("contract_id"),
        create_tx_hash: row.get("create_tx_hash"),
        created_at: row.get("created_at"),
    }
}

/// Record a request. The unique index on `(creator_address, idempotency_key)` is what makes the
/// second of two concurrent clicks lose the race here rather than at the contract.
pub async fn insert<'c, E: PgExecutor<'c>>(
    conn: E,
    new: &NewRequestRow,
) -> Result<RequestRow, DbError> {
    let row = sqlx::query(
        r#"
        INSERT INTO payment_requests
            (id, creator_address, created_by, trace_id, payee_address, asset_code, asset_contract,
             amount, reference, reference_hash, expiry_at, contract_id, idempotency_key)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8::numeric, $9, $10, $11, $12, $13)
        RETURNING id, creator_address, created_by, trace_id, payee_address, asset_code, asset_contract,
                  amount::text AS amount, reference, expiry_at, status::text AS status,
                  contract_id, create_tx_hash, created_at
        "#,
    )
    .bind(new.id.as_slice())
    .bind(&new.creator_address)
    .bind(&new.created_by)
    .bind(&new.trace_id)
    .bind(&new.payee_address)
    .bind(&new.asset_code)
    .bind(&new.asset_contract)
    .bind(&new.amount)
    .bind(&new.reference)
    .bind(new.reference_hash.as_slice())
    .bind(new.expiry_at)
    .bind(&new.contract_id)
    .bind(new.idempotency_key)
    .fetch_one(conn)
    .await
    .map_err(DbError::classify)?;

    Ok(row_to_request(&row))
}

pub async fn get<'c, E: PgExecutor<'c>>(conn: E, id: &[u8; 32]) -> Result<RequestRow, DbError> {
    let row = sqlx::query(
        r#"
        SELECT id, creator_address, created_by, trace_id, payee_address, asset_code, asset_contract,
               amount::text AS amount, reference, expiry_at, status::text AS status,
               contract_id, create_tx_hash, created_at
        FROM payment_requests WHERE id = $1
        "#,
    )
    .bind(id.as_slice())
    .fetch_optional(conn)
    .await
    .map_err(DbError::classify)?
    .ok_or(DbError::NotFound)?;

    Ok(row_to_request(&row))
}

/// The list the interface shows, newest first.
pub async fn list<'c, E: PgExecutor<'c>>(
    conn: E,
    status: Option<&str>,
    limit: i64,
) -> Result<Vec<RequestRow>, DbError> {
    let rows = sqlx::query(
        r#"
        SELECT id, creator_address, created_by, trace_id, payee_address, asset_code, asset_contract,
               amount::text AS amount, reference, expiry_at, status::text AS status,
               contract_id, create_tx_hash, created_at
        FROM payment_requests
        WHERE ($1::text IS NULL OR status::text = $1)
        ORDER BY created_at DESC
        LIMIT $2
        "#,
    )
    .bind(status)
    .bind(limit)
    .fetch_all(conn)
    .await
    .map_err(DbError::classify)?;

    Ok(rows.iter().map(row_to_request).collect())
}

/// Record the creation transaction hash, once the readback has confirmed it.
///
/// Separate from the insert on purpose: the row exists in `pending` with a null hash from the
/// moment the request is submitted, and the hash arrives only after the chain has been asked.
/// That gap is exactly step S-10, made visible in the schema.
pub async fn set_create_tx<'c, E: PgExecutor<'c>>(
    conn: E,
    id: &[u8; 32],
    tx_hash: &str,
) -> Result<(), DbError> {
    sqlx::query(
        "UPDATE payment_requests SET create_tx_hash = $2, version = version + 1 WHERE id = $1",
    )
    .bind(id.as_slice())
    .bind(tx_hash)
    .execute(conn)
    .await
    .map_err(DbError::classify)?;
    Ok(())
}

/// Move a request's status, refusing a transition the state machine forbids.
///
/// The `WHERE status = ANY($3)` clause is what makes this safe under concurrency: two workers
/// racing to mark the same request paid produce one update and one no-op, rather than two updates
/// and a lost history.
pub async fn set_status<'c, E: PgExecutor<'c>>(
    conn: E,
    id: &[u8; 32],
    to: &str,
    allowed_from: &[&str],
) -> Result<bool, DbError> {
    let from: Vec<String> = allowed_from.iter().map(|s| (*s).to_string()).collect();
    let result = sqlx::query(
        r#"
        UPDATE payment_requests
        SET status = $2::request_status, version = version + 1
        WHERE id = $1 AND status::text = ANY($3)
        "#,
    )
    .bind(id.as_slice())
    .bind(to)
    .bind(&from)
    .execute(conn)
    .await
    .map_err(DbError::classify)?;

    Ok(result.rows_affected() == 1)
}

/// Materialise `expired` for requests whose window has closed.
///
/// The contract has no `Expired` status — it compares the ledger clock on every call. This sweep
/// exists so the interface can filter on it.
pub async fn sweep_expired<'c, E: PgExecutor<'c>>(conn: E) -> Result<u64, DbError> {
    let result = sqlx::query(
        r#"
        UPDATE payment_requests
        SET status = 'expired', version = version + 1
        WHERE status IN ('pending', 'approved') AND expiry_at <= now()
        "#,
    )
    .execute(conn)
    .await
    .map_err(DbError::classify)?;

    Ok(result.rows_affected())
}
