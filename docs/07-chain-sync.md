# 07 · Chain synchronisation

Modules D1–D4 — the minimal version. One component reads the chain; everything else reads what that
component wrote.

This is deliberately small. A full reconciliation indexer is out of scope
([00-overview.md](00-overview.md)); what is in scope is enough correctness that `paid` is never
written on a guess, and enough speed that the interface never waits on Horizon.

## Where it lives

The split is deliberate, and it is why the hard parts are testable at all:

| | |
|---|---|
| `settle-indexer` | The **fold**. No I/O, no async, no clock. Deduplication, ordering and cursor discipline tested against vectors rather than a live network |
| `settle-backend/src/events.rs` | The **RPC read**, and the XDR decoding of what comes back |
| `settle-backend/src/tasks.rs` · `ingest` | The **loop** that joins the two to Postgres |

Events are read from Soroban RPC's `getEvents` rather than from `stellar events`, even though every
other chain interaction in this service shells out to the CLI. The CLI does not return the
**transaction hash**, and `UNIQUE (tx_hash, op_index)` is the deduplication key the whole design
rests on. An ingest log keyed on anything else cannot promise what this one promises.

On a cold start the cursor begins near the present, not at ledger 1. Soroban RPC keeps a rolling
window of history, so asking for everything is an error rather than a large answer.

## D1 · Cursor ingest

```rust
loop {
    // Read BY CURSOR, never by time window. A time window silently skips events
    // when a clock drifts or a batch takes longer than the window.
    let batch = rpc.get_events(from: cursor, limit: 200).await?;

    for ev in batch {
        // D2 · deduplicate before anything else
        if db.insert_chain_event(&ev).is_duplicate() { continue }

        match ev.topic {
            "req_new"  => db.project_created(&ev)?,
            "req_appr" => db.project_approved(&ev)?,
            "req_paid" => db.project_paid(&ev)?,
            "req_canc" => db.project_cancelled(&ev)?,
            // An unrecognised event is recorded and flagged. It is NEVER skipped
            // silently — a swallowed event is money nobody can account for.
            _          => db.quarantine(&ev)?,
        }
        audit.emit(CHAIN_EVENT_PROCESSED, &ev);
    }

    // Advance the cursor AFTER the commit, never before. Dying between the two
    // means re-reading events we have already stored, which D2 makes harmless.
    cursor = db.commit_cursor(batch.last_ledger)?;
}
```

The ordering of the last two steps is the whole design. Committing the cursor before the work means a
crash loses events permanently. Committing it after means a crash replays events — and replay is safe
because of D2.

## D2 · Absolute deduplication

```sql
CONSTRAINT uq_chain_ev UNIQUE (tx_hash, op_index)
```

One constraint, and it buys three properties:

| Property | How |
|---|---|
| Reprocessing an event is a no-op | The insert conflicts and the loop moves on |
| Two instances can run in parallel | Both insert; one wins; neither corrupts anything |
| A rolling deploy is safe | No leader election, no lease, nothing to get wrong under partition |

**No leader election.** Correctness comes from the index. Anything that requires exactly one process
to be alive is a system that breaks during a deploy, and deploys happen more often than incidents.

## D3 · Readback before writing paid

The rule the whole system is arranged around: **never infer `paid` from a submit result.**

```
   submit returns Ok       ->  status: releasing        NOT paid
   ledger readback shows
   the transaction settled ->  mark_paid() on chain
                           ->  status: paid
```

A successful submit means the network accepted the envelope. It does not mean the transaction was
included, and it does not mean it succeeded. Those are three different facts, and only the last one
justifies telling somebody their vendor was paid.

The readback is what closes the loop from
[06-signature-service.md](06-signature-service.md)'s `reconcile_then_decide`: whether the outcome was
a clean success, a timeout or `tx_bad_seq`, the same question gets asked of the ledger, and the
ledger's answer is the only one recorded.

## D4 · The read-model

The interface reads a projection. **No synchronous Horizon call sits inside a request path.**

| Path | Reads | Budget |
|---|---|---|
| `GET /requests` | Read-model | < 45 ms p95 |
| `GET /requests/:id` | Read-model | < 45 ms p95 |
| `GET /requests/:id/evidence` | Read-model | < 45 ms p95 |
| `POST /requests/:id/prepare` | **Horizon, unavoidably** | ~600 ms p95 |

`prepare` is the exception and cannot be otherwise. A Stellar transaction carries a sequence number
that must equal the source account's current sequence plus one. That number exists only on the
network; it cannot be cached, because any other transaction from that account invalidates it
immediately. One RPC round trip is the honest cost, and its real p95 is published rather than
disguised.

### Staleness is shown, not hidden

If the read-model falls more than 60 seconds behind the latest ledger, the interface displays a
"synchronising" banner and disables the create button. Numbers stay on screen with the timestamp
they were true at. The alternative — showing stale numbers as though they were current — is how
somebody creates a second payment for an invoice that was already paid.

| Metric | Threshold |
|---|---|
| Read-model lag SLO | < 3 s |
| Lag warning shown in the interface | > 60 s |
| Ledger lag alert | > 60 s, about 12 ledgers |

## What this module is not

It is not a reconciliation indexer. It does not compare balances, it does not maintain a
double-entry ledger, and it does not detect drift between the chain and the database. Those belong
to a later phase and are explicitly out of scope. What it does is keep four event types flowing into
one table without loss and without duplication.

## The rules this document imposes

- **Read by cursor, never by time window.**
- **Advance the cursor after the commit, never before.**
- **Deduplicate at the database layer**, so two instances are always safe.
- **Never skip an event silently.** Unrecognised events are quarantined and recorded.
- **`paid` follows a ledger readback**, never a submit result.
- **No synchronous Horizon call in a read path.** `prepare` is the single documented exception.
