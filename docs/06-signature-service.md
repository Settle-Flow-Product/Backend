# 06 · Signature service

Modules C1–C8 — Deliverable 2, part two. The service builds an unsigned Stellar payment from an
approved request, holds the treasury's sequence number while approvers sign, verifies each
signature, and submits once the collected weight reaches the threshold.

This is where SOW 1's value sits, and it is also where a mishandled exception pays a vendor twice.
The account it acts on is [05-treasury-multisig.md](05-treasury-multisig.md); the tables it writes
are [08-data-model.md](08-data-model.md).

## C1 · Build the unsigned transaction

```rust
async fn build_unsigned(req: &PaymentRequest) -> Result<UnsignedTx> {
    let acc = horizon.load_account(TREASURY).await?;

    // C2 · THE SEQUENCE NUMBER IS AN EXCLUSIVE RESOURCE.
    // Two payments taking sequence N means one of them certainly fails after
    // signatures have been collected. Lock it in the database; do not assume.
    let seq = db.reserve_sequence(TREASURY, acc.sequence).await?;

    let tx = TransactionBuilder::new(acc.with_seq(seq), BASE_FEE, NETWORK)
        .add_operation(Operation::payment(req.payee, req.asset, req.amount))

        // THE RECONCILIATION KEY. memo = request id, so C6 can find this transaction
        // on chain later without knowing its hash.
        .add_memo(Memo::hash(req.id))

        // MANDATORY. Without maxTime, a fully-signed transaction stays usable
        // indefinitely by anyone holding a copy. That is a real security defect,
        // not a hygiene preference.
        .set_timebounds(now, now + 3600)

        .build();

    Ok(UnsignedTx { xdr: tx.to_xdr(), hash: tx.hash(NETWORK), seq })
    // hash = sha256(networkId ‖ ENVELOPE_TYPE_TX ‖ txBody) — signers sign EXACTLY this.
}
```

| Element | Value | Why |
|---|---|---|
| Source | The treasury G-account | It supplies the sequence number and the signatures |
| Operation | `payment(payee, asset, amount)` | Straight from the approved request |
| Memo | `Memo::hash(request_id)` | The reverse-lookup key. Without it, C6 cannot find a transaction whose hash was never received |
| Timebounds | one hour | A signed, unsubmitted transaction is a signed cheque |
| Fee | base fee, bumped later if needed | See C7 — never rebuild for fee reasons |

The returned transaction hash is what approvers sign. **It changes if the sequence number changes**,
which is why the sequence is reserved before the hash is computed and held until the settlement
resolves.

## C2 · Sequence locking — feature F-09

One in-flight payment per treasury account, enforced by the database rather than by care.

```sql
-- Exactly one settlement in flight per treasury.
CREATE UNIQUE INDEX ux_settle_inflight ON settlements (treasury_address)
    WHERE status IN ('awaiting_approval', 'releasing');

-- A reserved sequence belongs to exactly one settlement.
CREATE UNIQUE INDEX ux_settle_seq ON settlements (treasury_address, reserved_seq)
    WHERE reserved_seq IS NOT NULL;
```

**The key is the treasury account, not the request and not the asset.** A USDC payment and an XLM
payment drawn on the same treasury contend for the same sequence number. Keying the lock on anything
narrower produces exactly the failure the lock exists to prevent, and produces it only under
concurrency — which is to say, at the demo.

A second concurrent request receives `409 RELEASE_IN_FLIGHT` immediately. That is the whole point:
refused cleanly at the start, rather than accepted, signed by two people, and then killed by
`tx_bad_seq`.

The concurrency test is not optional. Two simultaneous requests, one `202`, one `409`, and exactly
one reserved sequence in the table afterwards.

## C3 · Accepting a signature — feature F-10

```rust
async fn submit_signature(sid: Uuid, signer: Address, sig: Bytes) -> Result<Progress> {
    let s  = db.lock_settlement(sid).await?;      // SELECT ... FOR UPDATE
    let tx = db.unsigned_tx(sid).await?;

    // V1 · the signer must belong to the policy PINNED to this settlement,
    //      not to whatever policy is current.
    let w = db.signer_weight(s.policy_version, &signer).ok_or(NotASigner)?;

    // V2 · the signature must verify against THIS transaction hash, not another.
    if !ed25519_verify(&signer, &tx.hash, &sig) { return Err(BadSignature) }

    // V3 · one signer, one signature — PRIMARY KEY (settlement_id, signer_address).
    db.insert_approval(sid, signer, sig, w).await?;

    let collected = db.sum_weight(sid).await?;
    db.set_collected_weight(sid, collected).await?;
    if collected >= s.required_weight {
        queue.publish(SUBMIT_SETTLEMENT, sid);   // C5, asynchronously
    }
    Ok(Progress { collected, required: s.required_weight })
}
```

Three checks, three distinct failures:

| Check | Rejects | Error |
|---|---|---|
| V1 | A signer not in the pinned policy — including one added after this settlement was authorised | `NOT_A_SIGNER` |
| V2 | A signature over any hash other than this one | `BAD_SIGNATURE` |
| V3 | The same signer signing twice to reach the threshold alone | blocked by the primary key |

V3 is enforced by `PRIMARY KEY (settlement_id, signer_address)` rather than by an application check,
because an application can have a bug and an index cannot.

## C4 · Weight and threshold

Weights are summed from `settlement_approvals`, compared against `required_weight` pinned on the
settlement, and returned to the interface as `collected / required` so the approver screen can show
"1 of 2" (step S-23 in [03-flow.md](03-flow.md)).

Reaching the threshold publishes an event rather than submitting inline. The approver's HTTP request
returns as soon as their signature is stored; submission is a worker's job, and it is allowed to be
slow and allowed to fail.

## C5 · Assemble and submit

```rust
async fn submit_settlement(sid: Uuid) -> Result<()> {
    let s = db.lock_settlement(sid).await?;

    // A status guard is NOT idempotent here: status stays `releasing` for the whole
    // submit, so a redelivered queue message walks straight through and submits twice.
    // Guard on the SUBMIT ATTEMPT, not on the status.
    if s.submit_attempted_at.is_some() && s.tx_hash.is_none() {
        return reconcile_then_decide(sid).await;   // read the chain first, always
    }
    db.mark_submit_attempted(sid).await?;

    let mut env = XdrEnvelope::from(db.unsigned_tx(sid).await?);
    for a in db.approvals(sid).await? {
        env.attach(DecoratedSignature { hint: a.hint(), signature: a.sig });
    }

    match horizon.submit(env).await {
        Ok(r)                  => db.mark_releasing_with_tx(sid, r.hash).await,
        Err(TxBadSeq)          => db.mark_unresolved(sid).await,   // C6 — NOT a rebuild
        Err(Timeout)           => db.mark_unresolved(sid).await,   // C6 — same handling
        Err(InsufficientFee)   => horizon.submit_fee_bump(env, FEE_SOURCE).await, // C7
        Err(e)                 => db.mark_failed(sid, e).await,
    }
}
```

`submit_attempted_at` exists because the obvious guard is wrong. Checking `status == 'releasing'`
looks like it prevents a double submit; it does not, because the status is `releasing` throughout the
submit, so a redelivered message passes the check and submits again.

## C6 · Unresolved outcomes — feature F-13

**`tx_bad_seq` and a timeout are the same situation and get the same handling.** `tx_bad_seq` says
the sequence has been consumed. It does not say by whom, and the most common cause is that *this
envelope* already reached the ledger — a duplicate submit, a redelivered queue message, a lost HTTP
response. Building a new transaction with a new sequence and collecting signatures again pays the
vendor a second time.

```rust
// The ONLY path permitted to reach a rebuild.
async fn reconcile_then_decide(sid: Uuid) -> Result<()> {
    // 1. Look the treasury's transactions up BY MEMO — the memo is the request id,
    //    so this works without ever having received the transaction hash.
    match horizon.find_tx_by_memo(TREASURY, sid).await? {
        Some(tx) => db.mark_releasing_with_tx(sid, tx.hash).await,   // it already landed

        None => {
            // 2. Resubmit the SAME envelope first. Same hash; the network's own
            //    replay protection makes this safe and cheap.
            if horizon.resubmit_same_envelope(sid).await.is_ok() { return Ok(()) }

            // 3. Only once the sequence is demonstrably consumed by a DIFFERENT
            //    transaction may we rebuild — and that means interrupting the
            //    approvers again.
            db.reset_for_rebuild(sid).await
        }
    }
}
```

The memo is what makes step 1 possible. A settlement whose transaction hash was never received is
still findable, because the request id is written into the transaction itself.

## C7 · Fee bump

A transaction that is not being included because its fee is too low does **not** get rebuilt.
Stellar's fee-bump wraps an already-signed envelope: only the fee source signs the wrapper, and every
approver signature inside stays valid.

Rebuilding for fee reasons would discard valid signatures and interrupt two people for a problem the
protocol already solves.

## C8 · The reject branch — feature F-12

When an approver rejects:

1. Stop submission entirely. No envelope is assembled, nothing is sent.
2. **Release the reserved sequence** so the treasury is not blocked by a settlement that will never
   proceed.
3. Call `cancel_request` on the registry so the on-chain status matches.
4. Write an `audit_events` row naming who rejected and when.

The acceptance test is a negative assertion against the treasury's transaction list on chain — not
against our own database. Our database saying "cancelled" proves nothing about whether money moved.

## Failure handling summary

| Outcome | Handling | Never |
|---|---|---|
| Success | Record the hash, move to `releasing`, wait for readback | Write `paid` here |
| `tx_bad_seq` | `unresolved` → reconcile by memo | Rebuild and re-collect signatures |
| Timeout | `unresolved` → reconcile by memo | Assume failure |
| Insufficient fee | Fee-bump the signed envelope | Rebuild the transaction |
| Definitive failure | `failed`, with the reason recorded | Retry automatically |
| Timebounds lapsed | Cancel, rebuild, collect signatures again | Submit a transaction past its `maxTime` |

## The rules this document imposes

- **The sequence lock keys on the treasury account.** Not the request, not the asset.
- **The memo is the request id, always.** It is the only reverse-lookup key that survives losing a
  transaction hash.
- **Timebounds are mandatory and narrow.** One hour.
- **`tx_bad_seq` and timeout take the same path**, and that path reads the chain first.
- **The idempotency guard is `submit_attempted_at`**, never the status.
- **A rejection releases the sequence.** Otherwise one refusal blocks the treasury indefinitely.
- **Never rebuild a transaction for a fee problem.** Fee-bump it.
