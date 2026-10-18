//! Settlements, approvals, and the sequence reservation that makes F-09 structural.

use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::{PgExecutor, Row};
use uuid::Uuid;

use crate::error::DbError;

#[derive(Debug, Clone, Serialize)]
pub struct SettlementRow {
    pub id: Uuid,
    pub request_id: String,
    pub treasury_address: String,
    pub destination_address: String,
    pub asset_code: String,
    pub amount: String,
    pub policy_version: i32,
    pub required_weight: i32,
    pub collected_weight: i32,
    pub status: String,
    pub unsigned_xdr: Option<String>,
    pub tx_hash_to_sign: Option<String>,
    pub reserved_seq: Option<i64>,
    pub valid_until: Option<DateTime<Utc>>,
    pub submit_attempted_at: Option<DateTime<Utc>>,
    pub tx_hash: Option<String>,
    pub settled_at: Option<DateTime<Utc>>,
    pub rejected_by: Option<String>,
    /// Evidence E4 — the registry transaction that moved Pending to Approved.
    pub approve_tx_hash: Option<String>,
    /// Evidence E6 — the registry transaction that recorded the payment.
    pub mark_paid_tx_hash: Option<String>,
}

const COLUMNS: &str = r#"
    id, encode(request_id, 'hex') AS request_id, treasury_address, destination_address,
    asset_code, amount::text AS amount, policy_version, required_weight, collected_weight,
    status::text AS status, unsigned_xdr, tx_hash_to_sign, reserved_seq, valid_until,
    submit_attempted_at, tx_hash, settled_at, rejected_by, approve_tx_hash, mark_paid_tx_hash
"#;

fn row_to_settlement(row: &sqlx::postgres::PgRow) -> SettlementRow {
    SettlementRow {
        id: row.get("id"),
        request_id: row.get("request_id"),
        treasury_address: row.get("treasury_address"),
        destination_address: row.get("destination_address"),
        asset_code: row.get("asset_code"),
        amount: row.get("amount"),
        policy_version: row.get("policy_version"),
        required_weight: row.get("required_weight"),
        collected_weight: row.get("collected_weight"),
        status: row.get("status"),
        unsigned_xdr: row.get("unsigned_xdr"),
        tx_hash_to_sign: row.get("tx_hash_to_sign"),
        reserved_seq: row.get("reserved_seq"),
        valid_until: row.get("valid_until"),
        submit_attempted_at: row.get("submit_attempted_at"),
        tx_hash: row.get("tx_hash"),
        settled_at: row.get("settled_at"),
        rejected_by: row.get("rejected_by"),
        approve_tx_hash: row.get("approve_tx_hash"),
        mark_paid_tx_hash: row.get("mark_paid_tx_hash"),
    }
}

#[derive(Debug, Clone)]
pub struct NewSettlement {
    pub request_id: [u8; 32],
    pub treasury_address: String,
    pub destination_address: String,
    pub asset_code: String,
    pub amount: String,
    pub policy_version: i32,
    pub required_weight: i32,
    pub unsigned_xdr: String,
    pub tx_hash_to_sign: String,
    pub reserved_seq: i64,
    pub valid_until: DateTime<Utc>,
}

/// Reserve the treasury and record the built transaction, in one statement.
///
/// **This is where F-09 is enforced.** `settlements_one_in_flight_per_treasury` is a partial unique
/// index on `treasury_address`, so a second concurrent attempt fails here and surfaces as
/// `RELEASE_IN_FLIGHT` — refused cleanly at the start, rather than accepted, signed by two people,
/// and killed later by `tx_bad_seq`.
///
/// A constraint rather than a `SELECT` then `INSERT`, because the check-then-act version has a
/// window between the two and two service instances will find it.
pub async fn reserve<'c, E: PgExecutor<'c>>(
    conn: E,
    new: &NewSettlement,
) -> Result<SettlementRow, DbError> {
    let sql = format!(
        r#"
        INSERT INTO settlements
            (request_id, treasury_address, destination_address, asset_code, amount,
             policy_version, required_weight, status, unsigned_xdr, tx_hash_to_sign,
             reserved_seq, valid_until)
        VALUES ($1, $2, $3, $4, $5::numeric, $6, $7, 'awaiting_approval', $8, $9, $10, $11)
        RETURNING {COLUMNS}
        "#
    );

    let row = sqlx::query(&sql)
        .bind(new.request_id.as_slice())
        .bind(&new.treasury_address)
        .bind(&new.destination_address)
        .bind(&new.asset_code)
        .bind(&new.amount)
        .bind(new.policy_version)
        .bind(new.required_weight)
        .bind(&new.unsigned_xdr)
        .bind(&new.tx_hash_to_sign)
        .bind(new.reserved_seq)
        .bind(new.valid_until)
        .fetch_one(conn)
        .await
        .map_err(DbError::classify)?;

    Ok(row_to_settlement(&row))
}

pub async fn get<'c, E: PgExecutor<'c>>(conn: E, id: Uuid) -> Result<SettlementRow, DbError> {
    let sql = format!("SELECT {COLUMNS} FROM settlements WHERE id = $1");
    let row = sqlx::query(&sql)
        .bind(id)
        .fetch_optional(conn)
        .await
        .map_err(DbError::classify)?
        .ok_or(DbError::NotFound)?;
    Ok(row_to_settlement(&row))
}

/// The settlement currently holding a request, if any.
pub async fn for_request<'c, E: PgExecutor<'c>>(
    conn: E,
    request_id: &[u8; 32],
) -> Result<Option<SettlementRow>, DbError> {
    let sql = format!(
        "SELECT {COLUMNS} FROM settlements WHERE request_id = $1 ORDER BY created_at DESC LIMIT 1"
    );
    let row = sqlx::query(&sql)
        .bind(request_id.as_slice())
        .fetch_optional(conn)
        .await
        .map_err(DbError::classify)?;
    Ok(row.as_ref().map(row_to_settlement))
}

/// Record a signature and recompute the collected weight in the same transaction.
///
/// The primary key `(settlement_id, signer_address)` is what refuses a second signature from the
/// same signer — one signer contributing weight twice would reach a threshold of 2 alone, which is
/// the entire failure mode multisig exists to prevent. Application code has bugs; a primary key
/// does not.
pub async fn add_approval(
    pool: &sqlx::PgPool,
    settlement_id: Uuid,
    signer_address: &str,
    signature: &[u8; 64],
    weight: i32,
    tx_hash_signed: &str,
) -> Result<i32, DbError> {
    let mut tx = pool.begin().await.map_err(DbError::classify)?;

    sqlx::query(
        r#"
        INSERT INTO settlement_approvals
            (settlement_id, signer_address, signature, weight, tx_hash_signed)
        VALUES ($1, $2, $3, $4, $5)
        "#,
    )
    .bind(settlement_id)
    .bind(signer_address)
    .bind(signature.as_slice())
    .bind(weight)
    .bind(tx_hash_signed)
    .execute(&mut *tx)
    .await
    .map_err(DbError::classify)?;

    // Recomputed from the rows rather than incremented. A counter that is incremented separately
    // is a counter that drifts the first time a transaction is retried.
    let collected: i32 = sqlx::query_scalar(
        r#"
        UPDATE settlements
        SET collected_weight = (
            SELECT COALESCE(SUM(weight), 0)::int FROM settlement_approvals
            WHERE settlement_id = $1
        ), version = version + 1
        WHERE id = $1
        RETURNING collected_weight
        "#,
    )
    .bind(settlement_id)
    .fetch_one(&mut *tx)
    .await
    .map_err(DbError::classify)?;

    tx.commit().await.map_err(DbError::classify)?;
    Ok(collected)
}

#[derive(Debug, Clone)]
pub struct ApprovalRow {
    pub signer_address: String,
    pub signature: Vec<u8>,
    pub weight: i32,
    pub tx_hash_signed: String,
    pub signed_at: DateTime<Utc>,
}

pub async fn approvals<'c, E: PgExecutor<'c>>(
    conn: E,
    settlement_id: Uuid,
) -> Result<Vec<ApprovalRow>, DbError> {
    let rows = sqlx::query(
        r#"
        SELECT signer_address, signature, weight, tx_hash_signed, signed_at
        FROM settlement_approvals WHERE settlement_id = $1 ORDER BY signed_at
        "#,
    )
    .bind(settlement_id)
    .fetch_all(conn)
    .await
    .map_err(DbError::classify)?;

    Ok(rows
        .iter()
        .map(|r| ApprovalRow {
            signer_address: r.get("signer_address"),
            signature: r.get("signature"),
            weight: r.get("weight"),
            tx_hash_signed: r.get("tx_hash_signed"),
            signed_at: r.get("signed_at"),
        })
        .collect())
}

/// Claim the right to submit — module C5.
///
/// **Returns false if a submit has already been attempted and no hash came back.** That is the
/// correct idempotency guard. The obvious one — "is the status `releasing`?" — is not idempotent,
/// because the status is `releasing` for the whole duration of a submit, so a redelivered queue
/// message passes it and submits a second time.
pub async fn claim_submit<'c, E: PgExecutor<'c>>(conn: E, id: Uuid) -> Result<bool, DbError> {
    let result = sqlx::query(
        r#"
        UPDATE settlements
        SET submit_attempted_at = now(), status = 'releasing', version = version + 1
        WHERE id = $1 AND submit_attempted_at IS NULL
        "#,
    )
    .bind(id)
    .execute(conn)
    .await
    .map_err(DbError::classify)?;

    Ok(result.rows_affected() == 1)
}

pub async fn mark_submitted<'c, E: PgExecutor<'c>>(
    conn: E,
    id: Uuid,
    tx_hash: &str,
) -> Result<(), DbError> {
    sqlx::query(
        "UPDATE settlements SET tx_hash = $2, status = 'releasing', version = version + 1 WHERE id = $1",
    )
    .bind(id)
    .bind(tx_hash)
    .execute(conn)
    .await
    .map_err(DbError::classify)?;
    Ok(())
}

/// Confirmed by a ledger readback — the only path to `settled`.
pub async fn mark_settled<'c, E: PgExecutor<'c>>(
    conn: E,
    id: Uuid,
    tx_hash: &str,
) -> Result<(), DbError> {
    sqlx::query(
        r#"
        UPDATE settlements
        SET status = 'settled', tx_hash = $2, settled_at = now(), version = version + 1
        WHERE id = $1
        "#,
    )
    .bind(id)
    .bind(tx_hash)
    .execute(conn)
    .await
    .map_err(DbError::classify)?;
    Ok(())
}

/// The outcome is unknown. Nothing leaves this state except a readback.
pub async fn mark_unresolved<'c, E: PgExecutor<'c>>(conn: E, id: Uuid) -> Result<(), DbError> {
    sqlx::query(
        "UPDATE settlements SET status = 'unresolved', version = version + 1 WHERE id = $1",
    )
    .bind(id)
    .execute(conn)
    .await
    .map_err(DbError::classify)?;
    Ok(())
}

/// A rejection — module C8.
///
/// Marking it `failed` releases the treasury, because the in-flight index does not cover `failed`.
/// Without that, one refusal would block the treasury indefinitely.
pub async fn mark_rejected<'c, E: PgExecutor<'c>>(
    conn: E,
    id: Uuid,
    rejected_by: &str,
) -> Result<(), DbError> {
    sqlx::query(
        r#"
        UPDATE settlements
        SET status = 'failed', rejected_by = $2, rejected_at = now(),
            -- Give the sequence back. `settlements_one_settlement_per_reserved_sequence` is
            -- partial on `reserved_seq IS NOT NULL`, so a failed settlement that keeps the number
            -- keeps the slot — and since a refused payment never advances the treasury's sequence,
            -- the NEXT attempt computes the same number and collides with a corpse. Module C8 says
            -- a rejection releases the sequence; this is where that happens.
            reserved_seq = NULL,
            version = version + 1
        WHERE id = $1
        "#,
    )
    .bind(id)
    .bind(rejected_by)
    .execute(conn)
    .await
    .map_err(DbError::classify)?;
    Ok(())
}

/// Settlements that reached their threshold and are waiting for a submitter.
pub async fn ready_to_submit<'c, E: PgExecutor<'c>>(
    conn: E,
    limit: i64,
) -> Result<Vec<SettlementRow>, DbError> {
    let sql = format!(
        r#"
        SELECT {COLUMNS} FROM settlements
        WHERE status = 'awaiting_approval'
          AND collected_weight >= required_weight
          AND submit_attempted_at IS NULL
        ORDER BY created_at
        LIMIT $1
        "#
    );
    let rows = sqlx::query(&sql)
        .bind(limit)
        .fetch_all(conn)
        .await
        .map_err(DbError::classify)?;
    Ok(rows.iter().map(row_to_settlement).collect())
}

/// Settlements whose outcome we do not know — the reconciliation queue.
pub async fn needing_reconciliation<'c, E: PgExecutor<'c>>(
    conn: E,
    limit: i64,
) -> Result<Vec<SettlementRow>, DbError> {
    let sql = format!(
        r#"
        SELECT {COLUMNS} FROM settlements
        WHERE status = 'unresolved'
           OR (status = 'releasing' AND submit_attempted_at IS NOT NULL AND tx_hash IS NULL)
        ORDER BY created_at
        LIMIT $1
        "#
    );
    let rows = sqlx::query(&sql)
        .bind(limit)
        .fetch_all(conn)
        .await
        .map_err(DbError::classify)?;
    Ok(rows.iter().map(row_to_settlement).collect())
}

/// Record evidence milestone E4 — the registry transaction that approved this request.
///
/// Separate from `set_status`: the status can move for reasons that produce no transaction (a
/// replay that found the request already approved), and a hash column that sometimes holds a hash
/// and sometimes holds a word is a column nobody can link to.
pub async fn set_approve_tx<'c, E: PgExecutor<'c>>(
    conn: E,
    id: Uuid,
    tx_hash: &str,
) -> Result<(), DbError> {
    sqlx::query("UPDATE settlements SET approve_tx_hash = $2 WHERE id = $1")
        .bind(id)
        .bind(tx_hash)
        .execute(conn)
        .await?;
    Ok(())
}

/// Record evidence milestone E6 — the registry transaction that marked this request paid.
pub async fn set_mark_paid_tx<'c, E: PgExecutor<'c>>(
    conn: E,
    id: Uuid,
    tx_hash: &str,
) -> Result<(), DbError> {
    sqlx::query("UPDATE settlements SET mark_paid_tx_hash = $2 WHERE id = $1")
        .bind(id)
        .bind(tx_hash)
        .execute(conn)
        .await?;
    Ok(())
}
