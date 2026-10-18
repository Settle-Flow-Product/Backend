# 01 · Features

Thirteen features that must run. Each has a code, a responsible party, and an acceptance test you
can observe. **No feature is accepted on the grounds that the code is written.**

The work breakdown that implements them is [02-modules.md](02-modules.md); the flow they compose
into is [03-flow.md](03-flow.md).

## The thirteen

| Code | Feature | Description | Accepted when |
|---|---|---|---|
| **F-01** | Wallet connection and authentication | An internal user connects a Stellar wallet and signs a nonce challenge to prove control of the address. No asset is transferred. | Sign-in works with a wallet; a bad signature is refused |
| **F-02** | Create a payment request | Enter vendor address, amount, asset, reference. Written to the registry contract. | A `create_request` transaction hash on Stellar Expert |
| **F-03** | Validate request data | `amount > 0` and at most 7 decimals; the asset is supported; the vendor is a valid Stellar address; the reference is 1–64 characters. | Every constraint has a negative unit test |
| **F-04** | Duplicate protection | The request id derives deterministically from the idempotency key. Clicking twice, or retrying after a timeout, does not create a second request. | Calling the API twice with the same key yields the same request id |
| **F-05** | Lifecycle tracking | `Pending → Approved → Paid`, plus the `Cancelled` branch. Status is authoritative on chain. | Reading `get_request` at each milestone shows the correct status |
| **F-06** | Treasury multisig configuration | Three signers at weight 1, thresholds low/med/high = 2, and `masterWeight = 0`. | Reading the account from Horizon asserts the exact configuration |
| **F-07** | Account lock-out guard | Refuse any `setOptions` that would leave total signer weight below a threshold — an unrecoverable state. | A test asserts that a locking configuration is rejected |
| **F-08** | Build an unsigned transaction | From an approved request, build a treasury → vendor payment, memo carrying the request id, with timebounds. | The returned XDR decodes, and the memo matches the request id |
| **F-09** | Sequence number locking | One in-flight payment per treasury. A second concurrent request is refused cleanly — not left to fail after signatures have been collected. | Two concurrent requests: the second receives `409` |
| **F-10** | Collect approver signatures | Verify ed25519 over the exact transaction hash; the signer must belong to the pinned policy; one signer signs once. | A wrong signature and an unknown signer are both refused |
| **F-11** | Threshold detection and submit | Sum the weights; at 2, assemble the `DecoratedSignature` set and send to Horizon. | The payment transaction hash, viewable on Stellar Expert |
| **F-12** | The reject branch | An approver refuses: the request moves to `Cancelled` and submission stops entirely. | A test asserts that no transaction left the treasury |
| **F-13** | Mark paid after readback | Call `mark_paid` only after reading the ledger back. Timeout and `tx_bad_seq` both route through reconcile. | The `mark_paid` transaction hash, and a readback showing `Paid` |

## The three where money is lost

These get the most careful code review in the project.

**F-09 — sequence contention.** Signatures bind to the treasury's sequence number. If two payments
take the same sequence, the second dies with `tx_bad_seq` *after* two people have already been
interrupted to sign it. The lock belongs in the database from Day 9, not discovered on Day 14.

**F-12 — the reject branch.** If this is wrong, a payment somebody explicitly refused still leaves
the treasury. The test is a negative assertion: no transaction left the account. Negative assertions
are easy to write badly, so this one names the account and checks its transaction list.

**F-13 — handling `tx_bad_seq`.** If the response to `tx_bad_seq` is "rebuild the transaction and
collect signatures again", the system pays the same vendor twice. `tx_bad_seq` says only that the
sequence has been consumed; it does not say by whom, and the most common cause is that this very
envelope already reached the ledger. Read the chain first. Always.

## Coverage against the deliverables

| Deliverable | Features |
|---|---|
| 1 — registry contract | F-02, F-03, F-04, F-05 |
| 2 — treasury and signature service | F-06, F-07, F-08, F-09, F-10, F-11, F-12 |
| 3 — testnet deploy and demo | F-13, and the round trip that exercises all of the above |
| Supporting | F-01 |

## The rules this document imposes

- **No feature is accepted on "the code is written".** Every row above names something observable.
- **F-03 has one negative test per constraint.** Not one test covering "invalid input".
- **F-12's test is a negative assertion about the treasury**, not about our own database.
- **F-09 and F-13 are reviewed by the Technical Lead**, not merged on a single approval.
