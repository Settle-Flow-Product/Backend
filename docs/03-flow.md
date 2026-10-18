# 03 · Flow

Thirty steps across six lanes, from sign-in to a complete evidence package, plus the payment
request's state machine.

This document describes what happens and in what order. What each function does is
[04-contract-interface.md](04-contract-interface.md) and
[06-signature-service.md](06-signature-service.md). The diagram version, drawn from the shipped code
rather than from this specification, lives in [`../resources/flows/`](../resources/flows/).

## Lanes

| Lane | Who | Node kind |
|---|---|---|
| Business | The internal user creating the request | user |
| Approver (2 of 3) | The people who sign | user |
| Minimum app | The four screens | interface |
| Backend | Everything that decides | system · decision · error |
| Registry contract | On-chain lifecycle | system · success · error |
| Stellar / treasury | Finality and the actual transfer | system |

## Part one — sign-in to a request sitting at Pending

Thirteen steps. The only hard part here is **S-10**: never believe a submit result; believe only the
status read back from the contract.

| Step | Lane · kind | Node | What |
|---|---|---|---|
| S-01 | app · interface | Open the SOW 1 app | An internal tool; only people inside the business reach it |
| S-02 | business · user | Connect the wallet and sign the challenge | Proves control of the address. No asset moves |
| S-03 | backend · system | Verify the signature | A single-use nonce bound to domain and network; replay-resistant |
| S-04 | app · interface | Open the request form | The four SOW fields: vendor, amount, asset, reference |
| S-05 | business · user | Enter the request data | Supplier address, amount, asset, reference code |
| S-06 | backend · system | Validate | `amount > 0`, the asset is supported, the vendor is a valid Stellar address |
| S-07 | backend · decision | Is the data valid? | No request is created while a field is in error |
| S-08 | app · error | Report the error on the field | Fixed in place and rechecked; never a generic toast |
| S-09 | contract · system | `create_request()` into the registry | Writes the request on chain. **The contract holds nothing** |
| S-10 | backend · decision | Readback: is it Pending? | Trust only the status read back from the contract, never the submit result |
| S-11 | backend · error | Unresolved · readback | Re-read by the deterministic id. Do not create a second request |
| S-12 | contract · success | Request is Pending | On chain, awaiting internal approval |
| S-13 | backend · system | Notify the approvers | Send the approval screen's link with the request id |

### S-10 and S-11 are the pair most often got wrong

"Unresolved" is not an error you show in a toast with a retry button. The correct procedure is:

1. Re-read by the deterministic request id.
2. If it reads back `Pending`, treat it as success.
3. Only after several backoff attempts return nothing may a new request be created, with a **new**
   idempotency key.

The return edge from S-11 to S-09 is a readback loop, not a resubmit loop. The distinction is the
whole point: resubmitting writes a second request; re-reading discovers the first one.

## Part two — from 2-of-3 approval to a complete evidence package

Seventeen steps. This is where all of SOW 1's value sits, and also where one mishandled exception
pays a vendor twice.

| Step | Lane · kind | Node | What |
|---|---|---|---|
| S-14 | app · interface | Approver opens the signing screen | Shows vendor, amount, reference and who has signed |
| S-15 | backend · system | `build_unsigned()` builds the transaction | Treasury → vendor payment, memo = request id |
| S-16 | backend · system | Lock the treasury sequence | One in-flight payment per treasury; prevents sequence contention |
| S-17 | approver · user | Review and decide | Check the vendor and the amount before signing |
| S-18 | backend · decision | Rejected? | The reject branch must stop submission entirely |
| S-19 | contract · error | Cancelled · no submit | Write the status on chain; assert no transaction left the treasury |
| S-20 | approver · user | Sign the transaction hash in the wallet | The signature is over the exact hash built; changing the sequence voids it |
| S-21 | backend · system | Verify the signature and add its weight | ed25519 over the transaction hash; the signer must belong to the pinned policy |
| S-22 | backend · decision | Threshold 2 of 3 reached? | Sum the weights of the signers who have signed |
| S-23 | app · interface | Wait for the next approver | Show 1 of 2 progress and the transaction's timebounds deadline |
| S-24 | contract · system | `approve()` → Approved | The on-chain status changes only once the weight suffices |
| S-25 | backend · system | Assemble the signatures and submit | `DecoratedSignature` with the correct hint, sent to Horizon |
| S-26 | Stellar · system | Stellar processes the payment | The treasury pays the vendor address |
| S-27 | backend · decision | Outcome? | Confirmed, definitively failed, or unresolved |
| S-28 | backend · error | Reconcile before retrying | `tx_bad_seq` and timeout are handled identically: read the chain first |
| S-29 | contract · success | `mark_paid()` → Paid | The registry records `Paid` with the transaction hash, after readback |
| S-30 | app · success | Complete evidence | Per-milestone hashes plus Stellar Expert links for the reviewer |

### S-16 · Lock the sequence early, not at the end

A signature binds tightly to the treasury's sequence number. Two payments taking the same sequence
means the second dies with `tx_bad_seq` **after** two people have already been interrupted to sign
it. The lock belongs in the database from Day 9. Discovering this on Day 14 costs a sprint.

### S-28 · Reconcile is the only exit from "unresolved"

`tx_bad_seq` says the sequence has been consumed. It does not say by whom, and the most common cause
is that this very transaction reached the ledger. Building a new transaction and collecting
signatures again means **paying twice**.

Handle it exactly as a timeout:

1. Look the treasury's transactions up by memo — the memo is the request id, so the lookup works
   without knowing the transaction hash.
2. If found, record the hash and move on; it already settled.
3. If not found, resubmit the **same envelope** first. It has the same hash, and the network's own
   replay protection makes that safe.
4. Only when the sequence has demonstrably been consumed by a *different* transaction may a rebuild
   happen — and that means asking the approvers again.

## State machine

```
                     create_request
                           |
                           v
                      +---------+   cancel_request    +-----------+
                      | Pending | ------------------> | Cancelled |   terminal
                      +---------+                     +-----------+
                           |                                ^
                           | approve()  (weight >= threshold)|
                           v                                 |
                      +----------+   cancel_request          |
                      | Approved | --------------------------+
                      +----------+
                           |
                           | mark_paid()  (after ledger readback)
                           v
                       +------+
                       | Paid |   terminal
                       +------+


  Backend-only, not on chain:

       submitted --> unresolved --> (readback) --> confirmed | definitively failed
                          ^                |
                          +----------------+
                            never auto-retries out of this state
```

**`Unresolved` exists only in the backend; there is no such status on chain.** It is a first-class
state in every state machine in the system, not an edge case. Nothing transitions out of it except a
readback.

Two things the diagram makes visible:

- **`Cancelled` is reachable from both `Pending` and `Approved`.** An approved-but-unpaid request is
  exactly the thing a business most needs to be able to withdraw.
- **There is no edge back from `Paid` or `Cancelled`.** Both are terminal on chain. A mistake after
  that point is a new request, not an edit.

## The rules this document imposes

- **S-11 is a readback loop, never a resubmit loop.**
- **S-16 lands on Day 9**, before anything depends on it.
- **S-19 is verified by a negative assertion** against the treasury's transaction list, not against
  our own database.
- **S-28 reads the chain before doing anything else**, without exception, for both `tx_bad_seq` and
  timeout.
- **S-29 follows a readback.** `mark_paid` is never called on the strength of a submit result.
