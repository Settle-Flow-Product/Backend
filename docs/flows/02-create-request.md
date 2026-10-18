# 02 · Creating a payment

> *"Kết quả chưa rõ không phải là lỗi để hiện toast rồi cho bấm lại."* — the deck, on S-11

```mermaid
flowchart TB
  A["POST /requests"] --> B{"Idempotency-Key present?"}
  B -->|"no"| E1["IDEMPOTENCY_KEY_REQUIRED"]
  B --> C{"validate"}
  C -->|"bad address"| E2["INVALID_ADDRESS"]
  C -->|"bad amount"| E3["INVALID_AMOUNT"]
  C -->|"bad reference"| E4["INVALID_REFERENCE"]
  C --> D["Claim the key BEFORE any work"]
  D -->|"in flight"| E5["409 · wait and poll with the SAME key"]
  D -->|"same key, other body"| E6["IDEMPOTENCY_CONFLICT"]
  D -->|"already done"| R["Return the ORIGINAL result"]
  D --> F["id = sha256(business ‖ key)"]
  F --> G["INSERT payment_requests"]
  G --> H["202 Accepted · status pending · no tx hash yet"]
  H --> I["background: create_request() on chain"]
  I --> J{"readback: does get_request find it?"}
  J -->|"yes"| K["record the hash"]
  J -.->|"unresolved"| I
```

## The dashed edge is the whole point

`J` back to `I` is a **readback** loop, not a resubmit loop. On an unresolved outcome the next pass
re-reads by the deterministic id; it does not create a second request. And it cannot: the id is
derived from the same inputs, so the contract's `RequestExists` gate catches a second attempt even
if the backend tried.

## Why 202 and not 200

At the moment the API answers, the chain write has been accepted and not confirmed. `200` would
let a client read "OK" as "recorded".

Source: `backend/crates/settle-backend/src/http.rs`, `backend/crates/settle-backend/src/tasks.rs`.
