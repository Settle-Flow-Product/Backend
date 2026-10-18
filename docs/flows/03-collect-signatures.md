# 03 · Collecting signatures

```mermaid
flowchart TB
  A["POST /requests/:id/prepare"] --> B{"status payable?"}
  B -->|"no"| E1["NOT_PAYABLE"]
  B --> C{"settlement already awaiting?"}
  C -->|"yes"| RET["Return it · rebuilding would VOID every signature"]
  C --> D(["Horizon: read the treasury sequence"])
  D --> E["Build payment<br/>memo = request id<br/>timebounds 1 hour"]
  E --> F["INSERT settlements"]
  F -->|"unique violation"| E2["409 RELEASE_IN_FLIGHT"]
  F --> G["200 · unsigned XDR + tx hash + valid_until"]

  G --> H["POST /requests/:id/signatures"]
  H --> I{"window still open?"}
  I -->|"no"| E3["TX_EXPIRED"]
  I --> J{"signer in the PINNED policy?"}
  J -->|"no"| E4["NOT_A_SIGNER"]
  J --> K{"ed25519 over THIS hash?"}
  K -->|"no"| E5["BAD_SIGNATURE"]
  K --> L["INSERT settlement_approvals"]
  L -->|"primary key violation"| E6["ALREADY_SIGNED"]
  L --> M["Recompute collected weight"]
  M --> N{"collected >= required?"}
  N -->|"no"| W["Wait for the next approver"]
  N --> S["Hand off to the submitter"]
```

## Three refusals, three codes

`NOT_A_SIGNER`, `BAD_SIGNATURE` and `ALREADY_SIGNED` are distinct because a caller who cannot tell
them apart cannot fix the problem. The last one is enforced by a primary key rather than by
application code — one signer contributing weight twice would reach a threshold of two alone,
which is the entire failure mode multisig exists to prevent.

## The pinned policy

`J` checks the policy version stored **on the settlement**, not the current one. A signer added
after authorisation does not get to approve work that was already in flight.

Source: `backend/crates/settle-signature/src/collector.rs`, `backend/migrations/0006_one_signer_signs_once.up.sql`.
