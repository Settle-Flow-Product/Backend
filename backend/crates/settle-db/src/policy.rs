//! The signer roster, versioned rather than edited.

use sqlx::Row;
use uuid::Uuid;

use crate::error::DbError;

#[derive(Debug, Clone)]
pub struct PolicyRow {
    pub id: Uuid,
    pub treasury_address: String,
    pub version: i32,
    pub threshold: i32,
    pub signers: Vec<SignerRow>,
}

#[derive(Debug, Clone)]
pub struct SignerRow {
    pub address: String,
    pub weight: i32,
    pub role: String,
}

/// The policy currently in force for a treasury.
pub async fn active(pool: &sqlx::PgPool, treasury: &str) -> Result<PolicyRow, DbError> {
    let row = sqlx::query(
        r#"
        SELECT id, treasury_address, version, threshold
        FROM governance_policies
        WHERE treasury_address = $1 AND active
        "#,
    )
    .bind(treasury)
    .fetch_optional(pool)
    .await
    .map_err(DbError::classify)?
    .ok_or(DbError::NotFound)?;

    let id: Uuid = row.get("id");
    Ok(PolicyRow {
        id,
        treasury_address: row.get("treasury_address"),
        version: row.get("version"),
        threshold: row.get("threshold"),
        signers: signers_of(pool, id).await?,
    })
}

/// A specific version — **what a settlement is judged against**.
///
/// Signature checks read this, never `active`. A signer added after a settlement was authorised
/// does not get to approve work that was already in flight, and months later this is what answers
/// "under which rule was this payment approved?".
pub async fn version(
    pool: &sqlx::PgPool,
    treasury: &str,
    version: i32,
) -> Result<PolicyRow, DbError> {
    let row = sqlx::query(
        r#"
        SELECT id, treasury_address, version, threshold
        FROM governance_policies
        WHERE treasury_address = $1 AND version = $2
        "#,
    )
    .bind(treasury)
    .bind(version)
    .fetch_optional(pool)
    .await
    .map_err(DbError::classify)?
    .ok_or(DbError::NotFound)?;

    let id: Uuid = row.get("id");
    Ok(PolicyRow {
        id,
        treasury_address: row.get("treasury_address"),
        version: row.get("version"),
        threshold: row.get("threshold"),
        signers: signers_of(pool, id).await?,
    })
}

async fn signers_of(pool: &sqlx::PgPool, policy_id: Uuid) -> Result<Vec<SignerRow>, DbError> {
    let rows = sqlx::query(
        "SELECT signer_address, weight, role FROM policy_signers WHERE policy_id = $1 ORDER BY role",
    )
    .bind(policy_id)
    .fetch_all(pool)
    .await
    .map_err(DbError::classify)?;

    Ok(rows
        .iter()
        .map(|r| SignerRow {
            address: r.get("signer_address"),
            weight: r.get("weight"),
            role: r.get("role"),
        })
        .collect())
}

/// Enrol a new policy version. **Never an edit** — the previous version stays readable, which is
/// what makes a pinned `policy_version` meaningful.
pub async fn enrol(
    pool: &sqlx::PgPool,
    treasury: &str,
    threshold: i32,
    signers: &[(String, i32, String)],
) -> Result<PolicyRow, DbError> {
    let mut tx = pool.begin().await.map_err(DbError::classify)?;

    let next: i32 = sqlx::query_scalar(
        "SELECT COALESCE(MAX(version), 0) + 1 FROM governance_policies WHERE treasury_address = $1",
    )
    .bind(treasury)
    .fetch_one(&mut *tx)
    .await
    .map_err(DbError::classify)?;

    let id: Uuid = sqlx::query_scalar(
        r#"
        INSERT INTO governance_policies (treasury_address, version, threshold, active)
        VALUES ($1, $2, $3, false)
        RETURNING id
        "#,
    )
    .bind(treasury)
    .bind(next)
    .bind(threshold)
    .fetch_one(&mut *tx)
    .await
    .map_err(DbError::classify)?;

    for (address, weight, role) in signers {
        sqlx::query(
            "INSERT INTO policy_signers (policy_id, signer_address, weight, role) VALUES ($1,$2,$3,$4)",
        )
        .bind(id)
        .bind(address)
        .bind(weight)
        .bind(role)
        .execute(&mut *tx)
        .await
        .map_err(DbError::classify)?;
    }

    // Deactivate the previous one first: the partial unique index permits exactly one active
    // policy per treasury, so the order matters.
    sqlx::query(
        "UPDATE governance_policies SET active = false WHERE treasury_address = $1 AND active",
    )
    .bind(treasury)
    .execute(&mut *tx)
    .await
    .map_err(DbError::classify)?;

    // The trigger refuses this if the enrolled weight cannot reach the threshold — a policy that
    // can never authorise anything is a trap the business discovers at the worst moment.
    sqlx::query("UPDATE governance_policies SET active = true WHERE id = $1")
        .bind(id)
        .execute(&mut *tx)
        .await
        .map_err(DbError::classify)?;

    tx.commit().await.map_err(DbError::classify)?;
    active(pool, treasury).await
}
