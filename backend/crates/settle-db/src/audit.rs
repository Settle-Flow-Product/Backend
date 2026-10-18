//! The audit trail. Append-only, enforced by permission rather than by promise.

use sqlx::PgExecutor;

use crate::error::DbError;

/// Record a state change.
///
/// Called for **every** transition. `trace_id` is what connects a click in the interface to this
/// row to a chain event; the Definition of Done requires that chain to be followable end to end.
#[allow(clippy::too_many_arguments)]
pub async fn record<'c, E: PgExecutor<'c>>(
    conn: E,
    actor_type: &str,
    actor_id: Option<&str>,
    entity: &str,
    entity_id: &str,
    action: &str,
    from_state: Option<&str>,
    to_state: Option<&str>,
    trace_id: Option<&str>,
) -> Result<(), DbError> {
    sqlx::query(
        r#"
        INSERT INTO audit_events
            (actor_type, actor_id, entity, entity_id, action, from_state, to_state, trace_id)
        VALUES ($1::actor_type, $2, $3, $4, $5, $6, $7, $8)
        "#,
    )
    .bind(actor_type)
    .bind(actor_id)
    .bind(entity)
    .bind(entity_id)
    .bind(action)
    .bind(from_state)
    .bind(to_state)
    .bind(trace_id)
    .execute(conn)
    .await
    .map_err(DbError::classify)?;
    Ok(())
}
