//! Feature F-09 — one in-flight payment per treasury, and one signature per signer.
//!
//! This is the test the whole schema exists for. Two payments taking the same treasury sequence
//! number means the second dies with `tx_bad_seq` *after* two people have already been interrupted
//! to sign it. The refusal has to happen at the start, cleanly, and it has to hold under genuine
//! concurrency — which is why this runs the two attempts at the same time rather than one after
//! the other.

mod support;

use chrono::{Duration, Utc};
use uuid::Uuid;

const MARKER: &str = "test_sequence_lock";

async fn seed_request(pool: &sqlx::PgPool, id: [u8; 32]) {
    let row = settle_db::requests::NewRequestRow {
        id,
        creator_address: support::address("creator"),
        created_by: None,
        trace_id: None,
        payee_address: support::address("vendor"),
        asset_code: "USDC".into(),
        asset_contract: "CBIE".into(),
        amount: "4500.0000000".into(),
        reference: "PO-2026-0417".into(),
        reference_hash: [0xB2; 32],
        expiry_at: Utc::now() + Duration::hours(1),
        contract_id: MARKER.into(),
        idempotency_key: Uuid::new_v4(),
    };
    let _ = settle_db::requests::insert(pool, &row).await;
}

fn settlement(
    request_id: [u8; 32],
    asset: &str,
    seq: i64,
) -> settle_db::settlements::NewSettlement {
    settle_db::settlements::NewSettlement {
        request_id,
        treasury_address: MARKER.into(),
        destination_address: support::address("vendor"),
        asset_code: asset.into(),
        amount: "4500.0000000".into(),
        policy_version: 1,
        required_weight: 2,
        unsigned_xdr: "AAAAAgAAAAB".into(),
        tx_hash_to_sign: format!("hash-{seq}"),
        reserved_seq: seq,
        valid_until: Utc::now() + Duration::hours(1),
    }
}

#[tokio::test]
async fn two_concurrent_prepares_leave_one_reserved_sequence() {
    let Some(pool) = support::pool().await else {
        eprintln!("{}", support::SKIP_NOTE);
        return;
    };
    support::reset(&pool, MARKER).await;

    let id = [0xA1u8; 32];
    seed_request(&pool, id).await;

    // Genuinely at the same time. A sequential test would pass against a check-then-act
    // implementation that has a window between the check and the insert.
    let usdc = settlement(id, "USDC", 42);
    let xlm = settlement(id, "XLM", 43);
    let (ra, rb) = tokio::join!(
        settle_db::settlements::reserve(&pool, &usdc),
        settle_db::settlements::reserve(&pool, &xlm),
    );

    let winners = [ra.is_ok(), rb.is_ok()].iter().filter(|x| **x).count();
    assert_eq!(
        winners, 1,
        "exactly one of two concurrent reservations may succeed"
    );

    // The loser must be told RELEASE_IN_FLIGHT specifically, so the API can answer 409 rather than
    // a generic error the client cannot act on.
    let loser = match (ra, rb) {
        (Err(e), Ok(_)) | (Ok(_), Err(e)) => e,
        _ => unreachable!("the count assertion above already ruled this out"),
    };
    assert!(
        matches!(loser, settle_db::DbError::ReleaseInFlight),
        "the loser must be ReleaseInFlight, got {loser:?}"
    );

    // Note the assets DIFFER. Keying the index on (business, asset) would have allowed both, and
    // both would then have contended for the same treasury sequence number.
    let held: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM settlements WHERE treasury_address = $1 \
         AND status IN ('awaiting_approval','releasing','unresolved')",
    )
    .bind(MARKER)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(held, 1, "one treasury holds one in-flight settlement");

    support::reset(&pool, MARKER).await;
}

#[tokio::test]
async fn a_rejection_releases_the_treasury_for_the_next_payment() {
    let Some(pool) = support::pool().await else {
        eprintln!("{}", support::SKIP_NOTE);
        return;
    };
    let marker = "test_release_after_reject";
    support::reset(&pool, marker).await;

    let id = [0xA2u8; 32];
    let row = settle_db::requests::NewRequestRow {
        id,
        creator_address: support::address("creator"),
        created_by: None,
        trace_id: None,
        payee_address: support::address("vendor"),
        asset_code: "USDC".into(),
        asset_contract: "CBIE".into(),
        amount: "10.0000000".into(),
        reference: "PO-REJECT".into(),
        reference_hash: [0xB3; 32],
        expiry_at: Utc::now() + Duration::hours(1),
        contract_id: marker.into(),
        idempotency_key: Uuid::new_v4(),
    };
    let _ = settle_db::requests::insert(&pool, &row).await;

    let mut first = settlement(id, "USDC", 100);
    first.treasury_address = marker.into();
    let s = settle_db::settlements::reserve(&pool, &first)
        .await
        .expect("first reserve");

    // While it is in flight, the treasury is blocked.
    let mut second = settlement(id, "USDC", 101);
    second.treasury_address = marker.into();
    assert!(settle_db::settlements::reserve(&pool, &second)
        .await
        .is_err());

    // A rejection frees it. Without this, one refusal would block the treasury indefinitely and
    // the business would discover it at the next invoice.
    settle_db::settlements::mark_rejected(&pool, s.id, &support::address("cfo"))
        .await
        .expect("mark rejected");

    settle_db::settlements::reserve(&pool, &second)
        .await
        .expect("the treasury must be free again after a rejection");

    support::reset(&pool, marker).await;
}

#[tokio::test]
async fn the_same_signer_cannot_sign_twice() {
    let Some(pool) = support::pool().await else {
        eprintln!("{}", support::SKIP_NOTE);
        return;
    };
    let marker = "test_one_signature";
    support::reset(&pool, marker).await;

    let id = [0xA3u8; 32];
    let row = settle_db::requests::NewRequestRow {
        id,
        creator_address: support::address("creator"),
        created_by: None,
        trace_id: None,
        payee_address: support::address("vendor"),
        asset_code: "USDC".into(),
        asset_contract: "CBIE".into(),
        amount: "10.0000000".into(),
        reference: "PO-SIGN".into(),
        reference_hash: [0xB4; 32],
        expiry_at: Utc::now() + Duration::hours(1),
        contract_id: marker.into(),
        idempotency_key: Uuid::new_v4(),
    };
    let _ = settle_db::requests::insert(&pool, &row).await;

    let mut spec = settlement(id, "USDC", 200);
    spec.treasury_address = marker.into();
    let s = settle_db::settlements::reserve(&pool, &spec)
        .await
        .expect("reserve");

    let signer = support::address("cfo");
    let collected =
        settle_db::settlements::add_approval(&pool, s.id, &signer, &[0xCC; 64], 1, "hash-200")
            .await
            .expect("the first signature is accepted");
    assert_eq!(collected, 1);

    // One signer contributing weight twice would reach a threshold of 2 ALONE — the entire failure
    // mode multisig exists to prevent. The primary key refuses it, not application code.
    let again =
        settle_db::settlements::add_approval(&pool, s.id, &signer, &[0xDD; 64], 1, "hash-200")
            .await;
    assert!(again
        .expect_err("a second signature from the same signer must be refused")
        .is_duplicate());

    // And the weight did not move.
    let weight: i32 = sqlx::query_scalar("SELECT collected_weight FROM settlements WHERE id = $1")
        .bind(s.id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(
        weight, 1,
        "a refused signature must not change the collected weight"
    );

    // A DIFFERENT signer does count, and that is what reaches the threshold.
    let second_signer = support::address("ceo");
    let total = settle_db::settlements::add_approval(
        &pool,
        s.id,
        &second_signer,
        &[0xEE; 64],
        1,
        "hash-200",
    )
    .await
    .expect("a second, different signer is accepted");
    assert_eq!(total, 2, "two distinct signers reach the threshold");

    support::reset(&pool, marker).await;
}
