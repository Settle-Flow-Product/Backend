//! Feature F-04 — clicking twice creates one payment.
//!
//! The protection is structural: the key is claimed before any work happens, and
//! `UNIQUE (creator_address, idempotency_key)` decides the race in the database rather than in
//! application code, where two instances would each believe they won.

mod support;

use chrono::{Duration, Utc};
use settle_db::idempotency::Claim;
use settle_domain::RequestId;
use uuid::Uuid;

// A marker PER TEST, never shared.
//
// cargo runs tests in the same file concurrently, and `reset` deletes by marker. Two tests sharing
// one marker means each can delete the other's rows mid-run — which is exactly how the first
// version of this file produced a spurious failure that looked like a primary key not holding.
const MARKER_DERIVE: &str = "test_idem_derive";
const MARKER_CLAIM: &str = "test_idem_claim";

fn new_row(
    marker: &str,
    id: [u8; 32],
    key: Uuid,
    amount: &str,
) -> settle_db::requests::NewRequestRow {
    settle_db::requests::NewRequestRow {
        id,
        creator_address: support::address("creator"),
        created_by: None,
        trace_id: None,
        payee_address: support::address("vendor"),
        asset_code: "USDC".into(),
        asset_contract: "CBIE".into(),
        amount: amount.into(),
        reference: "PO-2026-0417".into(),
        reference_hash: [0xB2; 32],
        expiry_at: Utc::now() + Duration::hours(1),
        contract_id: marker.into(),
        idempotency_key: key,
    }
}

#[tokio::test]
async fn the_same_idempotency_key_returns_the_same_request_id() {
    let Some(pool) = support::pool().await else {
        eprintln!("{}", support::SKIP_NOTE);
        return;
    };
    support::reset(&pool, MARKER_DERIVE).await;

    let business = Uuid::from_u128(1);
    let key = Uuid::new_v4();

    // The id is DERIVED, so both attempts compute the same one before touching the database.
    let first = RequestId::derive(business, key);
    let second = RequestId::derive(business, key);
    assert_eq!(first, second, "a derived id must be stable");

    settle_db::requests::insert(
        &pool,
        &new_row(MARKER_DERIVE, *first.as_bytes(), key, "4500.0000000"),
    )
    .await
    .expect("the first insert succeeds");

    // The second attempt, exactly as a retry after a timeout would arrive.
    let repeat = settle_db::requests::insert(
        &pool,
        &new_row(MARKER_DERIVE, *second.as_bytes(), key, "4500.0000000"),
    )
    .await;
    let err = repeat.expect_err("a second insert with the same key must be refused");
    assert!(err.is_duplicate(), "expected a duplicate, got {err:?}");

    // Exactly one row exists.
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM payment_requests WHERE contract_id = $1")
            .bind(MARKER_DERIVE)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(count, 1, "two clicks must create one payment");

    support::reset(&pool, MARKER_DERIVE).await;
}

#[tokio::test]
async fn a_different_body_under_the_same_key_is_refused() {
    let Some(pool) = support::pool().await else {
        eprintln!("{}", support::SKIP_NOTE);
        return;
    };

    let scope = MARKER_CLAIM;
    let key = Uuid::new_v4();
    let body_a = [0xAAu8; 32];
    let body_b = [0xBBu8; 32];

    assert_eq!(
        settle_db::idempotency::claim(&pool, scope, key, &body_a)
            .await
            .unwrap(),
        Claim::Claimed,
        "the first claim succeeds"
    );

    // The same call again, still in flight: wait and poll — with the SAME key.
    assert_eq!(
        settle_db::idempotency::claim(&pool, scope, key, &body_a)
            .await
            .unwrap(),
        Claim::InFlight
    );

    // A DIFFERENT payment reusing the key. Returning the first payment's result here would be
    // worse than refusing: the caller would believe a payment they never made had gone through.
    assert_eq!(
        settle_db::idempotency::claim(&pool, scope, key, &body_b)
            .await
            .unwrap(),
        Claim::BodyMismatch
    );

    // Once finished, a retry gets the original result rather than doing the work again.
    settle_db::idempotency::finish(&pool, scope, key, "the-original-request-id")
        .await
        .unwrap();
    assert_eq!(
        settle_db::idempotency::claim(&pool, scope, key, &body_a)
            .await
            .unwrap(),
        Claim::AlreadyDone {
            result_ref: "the-original-request-id".into()
        }
    );

    support::reset(&pool, MARKER_CLAIM).await;
}
