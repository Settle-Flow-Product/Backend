//! The background loops.
//!
//! Three of them, and each has one job:
//!
//! * **chain writes** — submit the contract calls the API accepted with a 202.
//! * **submitter** — assemble signatures, submit, and reconcile anything unresolved.
//! * **sweeps** — materialise `expired` and clear lapsed challenges.
//!
//! All three are cancellable, so shutdown is orderly rather than a kill.

use std::sync::Arc;
use std::time::Duration;

use settle_domain::{decide_submit, SubmitAction};
use settle_signature::reconcile::{resolve, ChainLookup, Resolution};
use tokio::task::JoinSet;
use tokio_util_shim::CancellationToken;

use crate::state::AppState;

/// A tiny cancellation token, so the whole `tokio-util` dependency is not pulled in for one type.
mod tokio_util_shim {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    #[derive(Clone, Default)]
    pub struct CancellationToken(Arc<AtomicBool>);

    impl CancellationToken {
        pub fn new() -> Self {
            Self::default()
        }
        pub fn cancel(&self) {
            self.0.store(true, Ordering::SeqCst);
        }
        pub fn is_cancelled(&self) -> bool {
            self.0.load(Ordering::SeqCst)
        }
    }
}

pub struct Supervisor {
    token: CancellationToken,
    tasks: JoinSet<()>,
}

impl Supervisor {
    pub async fn shutdown(mut self) {
        self.token.cancel();
        while self.tasks.join_next().await.is_some() {}
    }
}

pub fn spawn_all(state: Arc<AppState>) -> Supervisor {
    let token = CancellationToken::new();
    let mut tasks = JoinSet::new();

    tasks.spawn(chain_writes(
        state.clone(),
        token.clone(),
        Duration::from_secs(state.settings.indexer_interval_seconds),
    ));
    tasks.spawn(submitter(
        state.clone(),
        token.clone(),
        Duration::from_secs(state.settings.submitter_interval_seconds),
    ));
    tasks.spawn(ingest(
        state.clone(),
        token.clone(),
        Duration::from_secs(state.settings.indexer_interval_seconds),
    ));
    tasks.spawn(sweeps(
        state.clone(),
        token.clone(),
        Duration::from_secs(state.settings.sweep_interval_seconds),
    ));

    Supervisor { token, tasks }
}

async fn every(token: &CancellationToken, interval: Duration) -> bool {
    if token.is_cancelled() {
        return false;
    }
    tokio::time::sleep(interval).await;
    !token.is_cancelled()
}

/// Write requests to the registry, and read them back before believing anything.
///
/// The readback is the point. A successful contract invocation means the CLI got a response; the
/// stored status changes only once `get_request` says so.
async fn chain_writes(state: Arc<AppState>, token: CancellationToken, interval: Duration) {
    while every(&token, interval).await {
        let pending = match settle_db::requests::list(&state.pool, Some("pending"), 20).await {
            Ok(r) => r,
            Err(e) => {
                tracing::error!(error = %e, "could not list pending requests");
                continue;
            }
        };

        for row in pending {
            if row.create_tx_hash.is_some() {
                continue;
            }
            let Some(id) = settle_domain::RequestId::parse_hex(&row.id) else {
                continue;
            };

            // Read first. If it is already on chain — because a previous attempt succeeded and we
            // never saw the answer — there is nothing to write, and writing again would be
            // refused by RequestExists anyway.
            let existing = state
                .registry
                .get_request(&state.settings.settler_key, &row.id)
                .await;

            if existing.is_ok() {
                // Already on chain from an earlier attempt whose answer we never saw. We do not know
                // which transaction wrote it, and "confirmed" is the honest placeholder — the
                // indexer fills the real hash in from the `req_new` event.
                let _ = settle_db::requests::set_create_tx(&state.pool, id.as_bytes(), "confirmed")
                    .await;
                continue;
            }

            let amount = match settle_domain::Stroops::parse(&row.amount) {
                Ok(a) => a.as_i128(),
                Err(e) => {
                    tracing::error!(error = %e, request = %row.id, "a stored amount does not parse");
                    continue;
                }
            };

            // A request with no asset contract can never be written: the contract's `asset` field
            // is an Address, and there is no address meaning "none". Retrying cannot fix it, so
            // cancel it and say so instead of warning every few seconds for ever.
            //
            // The API refuses this at creation now. Rows predating that fix still exist, and a loop
            // that cannot make progress must stop rather than stay busy.
            if row.asset_contract.trim().is_empty() {
                tracing::error!(request = %row.id, asset = %row.asset_code,
                    "no asset contract configured — cancelling rather than retrying for ever");
                let _ = settle_db::requests::set_status(
                    &state.pool,
                    id.as_bytes(),
                    "cancelled",
                    &["pending"],
                )
                .await;
                continue;
            }

            let expiry = row.expiry_at.timestamp() as u64;
            let reference_hash = hex::encode(settle_domain::reference_hash(&row.reference));

            match state
                .registry
                .create_request(
                    &state.settings.business_key,
                    &row.creator_address,
                    &row.payee_address,
                    &row.id,
                    &row.asset_contract,
                    amount,
                    &reference_hash,
                    expiry,
                )
                .await
            {
                Ok(invocation) => {
                    // Readback before believing it — step S-10.
                    match state
                        .registry
                        .get_request(&state.settings.settler_key, &row.id)
                        .await
                    {
                        Ok(_) => {
                            // The REAL transaction hash, which is evidence milestone E3. Writing the
                            // word "confirmed" here left the evidence page with nothing to link to,
                            // and "a transaction hash a reviewer can look up" is the deliverable.
                            let create_tx = invocation.tx_hash.as_deref().unwrap_or("confirmed");
                            let _ = settle_db::requests::set_create_tx(
                                &state.pool,
                                id.as_bytes(),
                                create_tx,
                            )
                            .await;
                            tracing::info!(request = %row.id, tx = %create_tx,
                                trace_id = row.trace_id.as_deref().unwrap_or("-"),
                                "request recorded on chain");
                            let _ = settle_db::audit::record(
                                &state.pool,
                                "system",
                                None,
                                "payment_request",
                                &row.id,
                                "written_on_chain",
                                None,
                                None,
                                row.trace_id.as_deref(),
                            )
                            .await;
                        }
                        Err(e) => {
                            // Unresolved, not failed. Do NOT create a second request — the next
                            // pass reads by the deterministic id and finds it if it landed.
                            tracing::warn!(request = %row.id, error = %e,
                                "create submitted, readback unresolved — will re-read, not re-create");
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!(request = %row.id, error = %e, "could not write the request on chain");
                }
            }
        }
    }
}

/// Modules D1 and D2 — read the registry's events back off the chain into the ingest log.
///
/// **This is the only loop that treats the chain as the source of truth rather than as somewhere it
/// just wrote to.** Every other task records what it did; this one records what HAPPENED, including
/// anything a person did by calling the contract directly, and anything this service did before it
/// last restarted.
///
/// Three properties, in the order they matter:
///
/// * **Deduplication is a database constraint**, not a check in this function. `UNIQUE (tx_hash,
///   op_index)` means reprocessing is a no-op, two instances can run at once, and a rolling deploy
///   needs no leader election.
/// * **The cursor advances only after the batch is committed.** Dying in between replays events,
///   which deduplication makes harmless. Advancing first would lose them permanently.
/// * **An unrecognised event is quarantined, never dropped.** A silently swallowed event is money
///   nobody can account for, and it surfaces only when somebody telephones.
async fn ingest(state: Arc<AppState>, token: CancellationToken, interval: Duration) {
    const BATCH: u32 = 200;

    while every(&token, interval).await {
        let cursor = match settle_db::chain::cursor(&state.pool).await {
            Ok(c) => c,
            Err(e) => {
                tracing::error!(error = %e, "could not read the ingest cursor");
                continue;
            }
        };

        // Soroban RPC keeps only a rolling window of history, so ledger 1 is not "everything" — it
        // is an error. On a cold start, and whenever the cursor has fallen off the back of that
        // window, begin near the present rather than refusing to start at all.
        let start = if cursor > 0 {
            cursor + 1
        } else {
            match state.events.latest_ledger().await {
                Ok(latest) => (latest - 8_000).max(1),
                Err(e) => {
                    tracing::warn!(error = %e, "could not ask the network where it is");
                    continue;
                }
            }
        };

        let events = match state.events.events(start, BATCH).await {
            Ok(e) => e,
            Err(crate::events::EventsError::Refused(detail)) => {
                // The usual cause is a start ledger the RPC no longer holds. Jumping the cursor
                // forward loses history, so say so loudly rather than papering over it.
                tracing::warn!(start, detail = %detail, "the RPC refused the event range");
                continue;
            }
            Err(e) => {
                tracing::warn!(error = %e, "could not read events");
                continue;
            }
        };

        if events.is_empty() {
            continue;
        }

        let mut written = 0usize;
        let mut quarantined = 0usize;
        let mut highest = cursor;
        let mut failed = false;

        for e in &events {
            let topic = settle_indexer::Topic::parse(&e.topic);
            let is_unknown = topic == settle_indexer::Topic::Unknown || e.request_id.is_none();

            let row = settle_db::chain::NewChainEvent {
                tx_hash: e.tx_hash.clone(),
                op_index: e.op_index,
                ledger_seq: e.ledger_seq,
                ledger_time: e.ledger_time,
                contract_id: Some(e.contract_id.clone()),
                topic: e.topic.clone(),
                request_id: e.request_id,
                payload: serde_json::json!({
                    "amount": e.amount.map(|a| a.to_string()),
                    "paid_tx": e.paid_tx,
                }),
                quarantined: is_unknown,
            };

            match settle_db::chain::record(&state.pool, &row).await {
                Ok(true) => {
                    written += 1;
                    if is_unknown {
                        quarantined += 1;
                        tracing::warn!(tx = %e.tx_hash, topic = %e.topic,
                            "unrecognised event quarantined, not dropped");
                    }
                }
                // Already recorded. The constraint did its job.
                Ok(false) => {}
                Err(err) => {
                    // Stop at the first failure and DO NOT advance past it. Carrying on would leave
                    // a hole that the cursor then skips over for good.
                    tracing::error!(error = %err, tx = %e.tx_hash, "could not record an event");
                    failed = true;
                    break;
                }
            }
            highest = highest.max(e.ledger_seq);
        }

        if failed {
            continue;
        }

        // Only now.
        if highest > cursor {
            if let Err(e) = settle_db::chain::commit_cursor(&state.pool, highest).await {
                tracing::error!(error = %e, "could not advance the ingest cursor");
                continue;
            }
        }

        if written > 0 {
            tracing::info!(
                written,
                quarantined,
                cursor = highest,
                "ingested chain events"
            );
        }
    }
}

/// Assemble signatures, submit, and reconcile anything whose outcome is unknown.
async fn submitter(state: Arc<AppState>, token: CancellationToken, interval: Duration) {
    while every(&token, interval).await {
        // 1 · anything that reached its threshold
        match settle_db::settlements::ready_to_submit(&state.pool, 5).await {
            Ok(rows) => {
                for row in rows {
                    submit_one(&state, row).await;
                }
            }
            Err(e) => tracing::error!(error = %e, "could not list settlements ready to submit"),
        }

        // 2 · anything whose outcome we do not know
        match settle_db::settlements::needing_reconciliation(&state.pool, 5).await {
            Ok(rows) => {
                for row in rows {
                    reconcile_then_decide(&state, row).await;
                }
            }
            Err(e) => {
                tracing::error!(error = %e, "could not list settlements needing reconciliation")
            }
        }
    }
}

async fn submit_one(state: &AppState, row: settle_db::settlements::SettlementRow) {
    // A settlement whose request never reached the registry can never settle: `approve` and
    // `mark_paid` both act on a record the contract has never seen. Retrying is not resilience
    // here, it is a loop — and while it spins, the settlement holds the treasury's sequence and
    // every other payment is refused.
    //
    // Give the sequence back instead, and say why. The request is still there to be re-raised once
    // whatever stopped the chain write is fixed.
    if let Some(rid) = settle_domain::RequestId::parse_hex(&row.request_id) {
        match settle_db::requests::get(&state.pool, rid.as_bytes()).await {
            Ok(r) if r.create_tx_hash.is_none() => {
                tracing::error!(settlement = %row.id, request = %row.request_id,
                    "the request never reached the registry — releasing the treasury rather than retrying");
                let _ = settle_db::settlements::mark_rejected(
                    &state.pool,
                    row.id,
                    &row.treasury_address,
                )
                .await;
                return;
            }
            Ok(_) => {}
            Err(e) => {
                tracing::error!(error = %e, settlement = %row.id, "could not read the request");
                return;
            }
        }
    }

    // Claim the right to submit. Returns false if a submit has already been attempted, which is
    // the CORRECT idempotency guard — a status check would let a redelivered message through.
    match settle_db::settlements::claim_submit(&state.pool, row.id).await {
        Ok(true) => {}
        Ok(false) => return,
        Err(e) => {
            tracing::error!(error = %e, settlement = %row.id, "could not claim the submit");
            return;
        }
    }

    // S-24 · move the request to Approved ON CHAIN before any money moves.
    //
    // The registry does not count signatures; it trusts the settler, and the settler's key is used
    // only here, after the threshold has been met and verified. Doing this before the payment also
    // means a demo that fails at submit still shows an on-chain approval, which is evidence E4.
    match state
        .registry
        .approve(&state.settings.settler_key, &row.request_id)
        .await
    {
        Ok(invocation) => {
            // Evidence E4.
            if let Some(tx) = invocation.tx_hash.as_deref() {
                let _ = settle_db::settlements::set_approve_tx(&state.pool, row.id, tx).await;
            }
            let _ = settle_db::requests::set_status(
                &state.pool,
                &request_id_bytes(&row.request_id),
                "approved",
                &["pending"],
            )
            .await;
            tracing::info!(request = %row.request_id, "approved on chain");
        }
        Err(e) => {
            // Already approved is fine — this loop is idempotent by design. Anything else means
            // the chain is not in the state we think it is, and paying would be reckless.
            let text = e.to_string();
            if !text.contains("NotPending") && !text.contains("#20") {
                tracing::error!(error = %e, settlement = %row.id,
                    "could not approve on chain — refusing to submit a payment");
                let _ = settle_db::settlements::mark_unresolved(&state.pool, row.id).await;
                return;
            }
        }
    }

    // Ask the contract whether this is still payable. A request cancelled on chain by its creator
    // must not be paid because our copy had not caught up yet.
    match state
        .registry
        .is_payable(&state.settings.settler_key, &row.request_id)
        .await
    {
        Ok(true) => {}
        Ok(false) => {
            tracing::warn!(request = %row.request_id,
                "the contract says this is no longer payable — not submitting");
            let _ = settle_db::settlements::mark_rejected(&state.pool, row.id, "chain").await;
            return;
        }
        Err(e) => {
            tracing::error!(error = %e, settlement = %row.id,
                "could not ask the contract whether this is payable — not submitting");
            let _ = settle_db::settlements::mark_unresolved(&state.pool, row.id).await;
            return;
        }
    }

    let Some(xdr) = row.unsigned_xdr.clone() else {
        tracing::error!(settlement = %row.id, "a settlement has no unsigned transaction");
        return;
    };

    let approvals = match settle_db::settlements::approvals(&state.pool, row.id).await {
        Ok(a) => a,
        Err(e) => {
            tracing::error!(error = %e, settlement = %row.id, "could not read approvals");
            return;
        }
    };

    let signed = match assemble(&xdr, &approvals) {
        Ok(s) => s,
        Err(e) => {
            tracing::error!(error = %e, settlement = %row.id, "could not assemble the signatures");
            return;
        }
    };

    let result = state.horizon.submit(&signed).await;
    tracing::info!(settlement = %row.id, outcome = ?result.outcome, "submitted");

    match decide_submit(result.outcome) {
        SubmitAction::RecordAndWait => {
            if let Some(hash) = result.tx_hash {
                let _ = settle_db::settlements::mark_submitted(&state.pool, row.id, &hash).await;
                confirm_and_mark_paid(state, &row, &hash).await;
            }
        }
        // tx_bad_seq and timeout both land here. NEVER a rebuild.
        SubmitAction::Reconcile => {
            let _ = settle_db::settlements::mark_unresolved(&state.pool, row.id).await;
        }
        SubmitAction::FeeBump => {
            // The already-signed envelope is wrapped; approver signatures stay valid.
            tracing::warn!(settlement = %row.id, "fee too low — the envelope needs a fee bump");
            let _ = settle_db::settlements::mark_unresolved(&state.pool, row.id).await;
        }
        SubmitAction::MarkFailed => {
            tracing::error!(settlement = %row.id, detail = ?result.detail, "submit failed definitively");
            let _ = settle_db::settlements::mark_unresolved(&state.pool, row.id).await;
        }
    }
}

/// The only path that may conclude anything about an unknown outcome.
async fn reconcile_then_decide(state: &AppState, row: settle_db::settlements::SettlementRow) {
    let memo = row.request_id.clone();

    match state
        .horizon
        .find_by_memo(&row.treasury_address, &memo)
        .await
    {
        // It already landed. Record it; do not resend, and above all do not rebuild.
        Ok(Some(hash)) => {
            tracing::info!(settlement = %row.id, tx = %hash, "reconciled by memo — the payment already settled");
            let _ = settle_db::settlements::mark_submitted(&state.pool, row.id, &hash).await;
            confirm_and_mark_paid(state, &row, &hash).await;
        }
        // Nothing under this memo. That is NOT the end of it — the question is now whether the
        // reserved sequence is still available, and only the chain can answer that.
        //
        // Leaving it here was a deadlock: the settlement stayed `Unresolved`, `Unresolved` holds the
        // treasury's sequence, and so every later payment was refused with RELEASE_IN_FLIGHT for as
        // long as the service ran. A recovery path that never recovers is an outage with a log line.
        Ok(None) => {
            let Some(reserved) = row.reserved_seq else {
                tracing::error!(settlement = %row.id, "unresolved with no reserved sequence");
                return;
            };

            let account_seq = match state.horizon.sequence(&row.treasury_address).await {
                Ok(s) => s,
                Err(e) => {
                    tracing::warn!(error = %e, settlement = %row.id,
                        "cannot check the treasury sequence — staying unresolved, which is correct");
                    return;
                }
            };

            // A transaction with sequence N needs the account to be at N-1. Anything at or past N
            // means the sequence was consumed by SOMETHING ELSE, since we already know it was not
            // consumed by us — the memo lookup above found nothing.
            let lookup = if account_seq < reserved {
                ChainLookup::NotFoundSequenceUnused
            } else {
                ChainLookup::NotFoundSequenceConsumed
            };

            match resolve(lookup) {
                // The same envelope, byte for byte. The hash is unchanged, so the network's own
                // replay protection makes a genuine duplicate harmless, and no approver is asked
                // for anything again.
                Resolution::ResubmitSameEnvelope => {
                    tracing::info!(settlement = %row.id, reserved, account_seq,
                        "sequence still unused — resubmitting the identical envelope");
                    submit_one(state, row).await;
                }
                // The expensive path, and the only one that can pay twice if reached in error. It is
                // safe HERE and only here: the payment is not under the memo, and the sequence it
                // was built on is gone, so that envelope can never be applied.
                Resolution::RebuildAndReCollect => {
                    tracing::warn!(settlement = %row.id, reserved, account_seq,
                        "the reserved sequence was consumed by something else — releasing for rebuild");
                    let _ = settle_db::settlements::mark_rejected(
                        &state.pool,
                        row.id,
                        &row.treasury_address,
                    )
                    .await;
                }
                Resolution::RecordSettled { .. } => {
                    unreachable!("handled by the memo branch above")
                }
            }
        }
        Err(e) => {
            tracing::error!(error = %e, settlement = %row.id, "reconciliation could not reach Horizon");
        }
    }
}

/// Mark the registry paid — **after** a readback, never on the strength of a submit.
async fn confirm_and_mark_paid(
    state: &AppState,
    row: &settle_db::settlements::SettlementRow,
    tx_hash: &str,
) {
    let Some(rid) = settle_domain::RequestId::parse_hex(&row.request_id) else {
        return;
    };
    let amount = match settle_domain::Stroops::parse(&row.amount) {
        Ok(a) => a.as_i128(),
        Err(_) => return,
    };

    match state
        .registry
        .mark_paid(
            &state.settings.settler_key,
            &row.request_id,
            amount,
            tx_hash,
        )
        .await
    {
        Ok(invocation) => {
            // Evidence E6.
            if let Some(tx) = invocation.tx_hash.as_deref() {
                let _ = settle_db::settlements::set_mark_paid_tx(&state.pool, row.id, tx).await;
            }
            // Read it back once more before recording it locally.
            if let Ok(onchain) = state
                .registry
                .get_request(&state.settings.settler_key, &row.request_id)
                .await
            {
                if onchain.status_name().contains("paid") {
                    let _ =
                        settle_db::settlements::mark_settled(&state.pool, row.id, tx_hash).await;
                    let _ = settle_db::requests::set_status(
                        &state.pool,
                        rid.as_bytes(),
                        "paid",
                        &["pending", "approved"],
                    )
                    .await;
                    // The originating trace, so one id runs from the button to this chain event.
                    let trace = settle_db::requests::get(&state.pool, rid.as_bytes())
                        .await
                        .ok()
                        .and_then(|r| r.trace_id);
                    let _ = settle_db::audit::record(
                        &state.pool,
                        "system",
                        None,
                        "payment_request",
                        &row.request_id,
                        "marked_paid",
                        Some("approved"),
                        Some("paid"),
                        trace.as_deref(),
                    )
                    .await;
                    tracing::info!(request = %row.request_id, tx = %tx_hash, "paid");
                }
            }
        }
        Err(e) => {
            tracing::error!(error = %e, request = %row.request_id, "could not mark the request paid");
        }
    }
}

/// A request id's raw bytes, for the database layer.
fn request_id_bytes(hex_id: &str) -> [u8; 32] {
    settle_domain::RequestId::parse_hex(hex_id)
        .map(|r| *r.as_bytes())
        .unwrap_or([0u8; 32])
}

fn assemble(
    xdr: &str,
    approvals: &[settle_db::settlements::ApprovalRow],
) -> Result<String, String> {
    use stellar_xdr_reexport::{Limits, ReadXdr, TransactionEnvelope, WriteXdr};

    let envelope =
        TransactionEnvelope::from_xdr_base64(xdr, Limits::none()).map_err(|e| e.to_string())?;

    let TransactionEnvelope::Tx(v1) = &envelope else {
        return Err("expected a v1 transaction envelope".into());
    };

    let unsigned = settle_chain::UnsignedTx {
        envelope: envelope.clone(),
        tx_hash: [0u8; 32],
        sequence: v1.tx.seq_num.0,
        max_time: 0,
    };

    let mut collected = Vec::with_capacity(approvals.len());
    for a in approvals {
        let public_key =
            settle_chain::decode_account(&a.signer_address).map_err(|e| e.to_string())?;
        let signature: [u8; 64] = a
            .signature
            .clone()
            .try_into()
            .map_err(|_| "a stored signature is not 64 bytes".to_string())?;
        collected.push(settle_chain::CollectedSignature {
            signer_public_key: public_key,
            signature,
        });
    }

    let signed =
        settle_chain::attach_signatures(&unsigned, &collected).map_err(|e| e.to_string())?;
    signed
        .to_xdr_base64(Limits::none())
        .map_err(|e| e.to_string())
}

/// Re-export so `assemble` does not need a direct stellar-xdr dependency in this crate's manifest.
mod stellar_xdr_reexport {
    pub use settle_chain::xdr::{Limits, ReadXdr, TransactionEnvelope, WriteXdr};
}

/// Push rejections onto the chain — step S-19.
///
/// A request cancelled in our database is not cancelled until the registry says so. Until then a
/// second operator reading the chain would still see it as payable.
async fn cancel_on_chain(state: &AppState) {
    let cancelled = match settle_db::requests::list(&state.pool, Some("cancelled"), 20).await {
        Ok(r) => r,
        Err(e) => {
            tracing::error!(error = %e, "could not list cancelled requests");
            return;
        }
    };

    for row in cancelled {
        // Already cancelled on chain? Then there is nothing to do. Reading first keeps this loop
        // idempotent, which matters because it runs every tick.
        match state
            .registry
            .get_request(&state.settings.settler_key, &row.id)
            .await
        {
            Ok(onchain) if onchain.status_name().contains("cancel") => continue,
            Ok(_) => {}
            // Not on chain at all — nothing to cancel.
            Err(_) => continue,
        }

        match state
            .registry
            .cancel_request(&state.settings.business_key, &row.id)
            .await
        {
            Ok(_) => tracing::info!(request = %row.id, "cancelled on chain"),
            Err(e) => tracing::warn!(request = %row.id, error = %e, "could not cancel on chain"),
        }
    }
}

/// Materialise `expired`, and clear lapsed challenges.
async fn sweeps(state: Arc<AppState>, token: CancellationToken, interval: Duration) {
    while every(&token, interval).await {
        match settle_db::requests::sweep_expired(&state.pool).await {
            Ok(n) if n > 0 => tracing::info!(count = n, "requests marked expired"),
            Ok(_) => {}
            Err(e) => tracing::error!(error = %e, "the expiry sweep failed"),
        }
        match settle_db::challenges::sweep(&state.pool).await {
            Ok(n) if n > 0 => tracing::debug!(count = n, "lapsed challenges cleared"),
            Ok(_) => {}
            Err(e) => tracing::error!(error = %e, "the challenge sweep failed"),
        }
        cancel_on_chain(&state).await;
    }
}
