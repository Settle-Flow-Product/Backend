//! The ingest log and the read position.

use chrono::{DateTime, Utc};
use sqlx::{PgExecutor, Row};

use crate::error::DbError;

#[derive(Debug, Clone)]
pub struct NewChainEvent {
    pub tx_hash: String,
    pub op_index: i32,
    pub ledger_seq: i64,
    pub ledger_time: DateTime<Utc>,
    pub contract_id: Option<String>,
    pub topic: String,
    pub request_id: Option<[u8; 32]>,
    pub payload: serde_json::Value,
    pub quarantined: bool,
}

/// Record an event, or discover it was already recorded.
///
/// Returns `false` when the event is a duplicate. `ON CONFLICT DO NOTHING` on
/// `UNIQUE (tx_hash, op_index)` is the single line that makes reprocessing a no-op, lets two
/// ingest instances run in parallel, and removes any need for leader election — correctness comes
/// from the constraint rather than from only one process being alive.
pub async fn record<'c, E: PgExecutor<'c>>(
    conn: E,
    event: &NewChainEvent,
) -> Result<bool, DbError> {
    let inserted = sqlx::query(
        r#"
        INSERT INTO chain_events
            (tx_hash, op_index, ledger_seq, ledger_time, contract_id, topic,
             request_id, payload, quarantined)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
        ON CONFLICT (tx_hash, op_index) DO NOTHING
        RETURNING id
        "#,
    )
    .bind(&event.tx_hash)
    .bind(event.op_index)
    .bind(event.ledger_seq)
    .bind(event.ledger_time)
    .bind(&event.contract_id)
    .bind(&event.topic)
    .bind(event.request_id.map(|r| r.to_vec()))
    .bind(&event.payload)
    .bind(event.quarantined)
    .fetch_optional(conn)
    .await
    .map_err(DbError::classify)?;

    Ok(inserted.is_some())
}

/// Where the reader has got to. Zero when it has never run.
pub async fn cursor<'c, E: PgExecutor<'c>>(conn: E) -> Result<i64, DbError> {
    let row = sqlx::query("SELECT ledger_seq FROM ingest_cursor WHERE id")
        .fetch_optional(conn)
        .await
        .map_err(DbError::classify)?;
    Ok(row.map(|r| r.get::<i64, _>("ledger_seq")).unwrap_or(0))
}

/// Advance the cursor — **after** the batch has been committed, never before.
///
/// `GREATEST` rather than a plain assignment: a late batch from further back must not rewind the
/// read position and cause the same range to be re-read forever.
pub async fn commit_cursor<'c, E: PgExecutor<'c>>(conn: E, ledger_seq: i64) -> Result<(), DbError> {
    sqlx::query(
        r#"
        INSERT INTO ingest_cursor (id, ledger_seq) VALUES (true, $1)
        ON CONFLICT (id) DO UPDATE
        SET ledger_seq = GREATEST(ingest_cursor.ledger_seq, EXCLUDED.ledger_seq),
            updated_at = now()
        "#,
    )
    .bind(ledger_seq)
    .execute(conn)
    .await
    .map_err(DbError::classify)?;
    Ok(())
}

/// Events flagged as unrecognised, for an operator to look at. Never silently dropped.
pub async fn quarantined<'c, E: PgExecutor<'c>>(
    conn: E,
    limit: i64,
) -> Result<Vec<(String, i32, String)>, DbError> {
    let rows = sqlx::query(
        "SELECT tx_hash, op_index, topic FROM chain_events WHERE quarantined ORDER BY ledger_seq DESC LIMIT $1",
    )
    .bind(limit)
    .fetch_all(conn)
    .await
    .map_err(DbError::classify)?;

    Ok(rows
        .iter()
        .map(|r| (r.get("tx_hash"), r.get("op_index"), r.get("topic")))
        .collect())
}
