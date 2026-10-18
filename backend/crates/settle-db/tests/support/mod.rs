//! Shared fixture for the database tests.
//!
//! These need a real Postgres, because the properties under test *are* the database's: a partial
//! unique index refusing a second in-flight settlement, a primary key refusing a second signature.
//! A mock would only assert that our mock behaves like our mock.
//!
//! When `DATABASE_URL` is not set the tests skip with a printed reason rather than failing, so
//! `cargo test` still works on a laptop with no database. CI always sets it.

#![allow(dead_code)]

use sqlx::PgPool;

pub const SKIP_NOTE: &str =
    "skipped: DATABASE_URL is not set, so there is no database to test against";

/// Connect and migrate, or return None so the caller can skip.
pub async fn pool() -> Option<PgPool> {
    let url = std::env::var("DATABASE_URL")
        .ok()
        .filter(|u| !u.trim().is_empty())?;
    match settle_db::connect(&url).await {
        Ok(p) => Some(p),
        Err(e) => {
            eprintln!("skipped: could not reach {url}: {e}");
            None
        }
    }
}

/// A valid-shaped Stellar address, distinct per seed.
pub fn address(prefix: &str) -> String {
    let alphabet = "ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
    let seed: Vec<char> = prefix
        .to_uppercase()
        .chars()
        .filter(|c| alphabet.contains(*c))
        .collect();
    let mut s = String::from("G");
    for i in 0..55 {
        s.push(if i < seed.len() {
            seed[i]
        } else {
            alphabet.as_bytes()[(i * 7 + seed.len()) % alphabet.len()] as char
        });
    }
    s
}

/// Remove everything a test created, so a rerun starts clean.
pub async fn reset(pool: &PgPool, marker: &str) {
    let _ = sqlx::query(
        "DELETE FROM settlement_approvals WHERE settlement_id IN \
         (SELECT id FROM settlements WHERE treasury_address = $1)",
    )
    .bind(marker)
    .execute(pool)
    .await;
    let _ = sqlx::query("DELETE FROM settlements WHERE treasury_address = $1")
        .bind(marker)
        .execute(pool)
        .await;
    let _ = sqlx::query("DELETE FROM payment_requests WHERE contract_id = $1")
        .bind(marker)
        .execute(pool)
        .await;
    let _ = sqlx::query("DELETE FROM idempotency_keys WHERE scope = $1")
        .bind(marker)
        .execute(pool)
        .await;
    let _ = sqlx::query("DELETE FROM governance_policies WHERE treasury_address = $1")
        .bind(marker)
        .execute(pool)
        .await;
}
