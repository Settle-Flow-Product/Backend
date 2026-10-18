# 04 · Submitting, and the unclear answer

> *"tx_bad_seq chỉ nói seq đã bị tiêu, không nói ai tiêu."* — the deck, on S-28

```mermaid
flowchart TB
  A["Threshold met"] --> B{"claim_submit: attempted already?"}
  B -->|"yes, and no tx hash"| RC["Reconcile"]
  B --> C(["approve() on chain"])
  C -->|"fails"| U1["unresolved · do NOT pay"]
  C --> D{"is_payable() on chain?"}
  D -->|"no"| STOP["Stop · the contract says no"]
  D --> E["Attach DecoratedSignatures"]
  E --> F(["Horizon: submit"])

  F -->|"accepted"| G["Record hash · releasing"]
  F -->|"tx_bad_seq"| RC
  F -->|"timeout"| RC
  F -->|"insufficient fee"| FB["Fee-bump the SIGNED envelope"]
  F -->|"definitive failure"| U2["unresolved · a human looks"]

  RC --> H(["Horizon: find by MEMO"])
  H -->|"found"| G
  H -->|"not found, seq unused"| RS["Resubmit the SAME envelope"]
  H -->|"not found, seq consumed by another tx"| RB["Rebuild · ask the approvers again"]

  G --> I(["readback: did it settle?"])
  I --> J["mark_paid() on chain"]
  J --> K(["readback: does get_request say Paid?"])
  K --> L["Paid"]
```

## The two edges that decide whether a vendor is paid once

`tx_bad_seq` and `timeout` both go to `RC`, never to `RB`. `tx_bad_seq` says the sequence has been
consumed; it does not say by whom, and the most common cause is that *this envelope* already
reached the ledger. Rebuilding and re-collecting signatures pays twice.

Only `H → RB` reaches a rebuild, and only once the sequence is demonstrably consumed by a
different transaction.

## Why the memo lookup works at all

The memo is the request id, written into the transaction when it was built. A settlement whose
transaction hash was never received is still findable.

## Why the guard is the attempt, not the status

`B` checks `submit_attempted_at`. The status is `releasing` for the whole duration of a submit, so
a status check would let a redelivered queue message through and submit a second time.

Source: `backend/crates/settle-signature/src/reconcile.rs`, `backend/crates/settle-backend/src/tasks.rs`.
