# 09 · API contract

Nine endpoints carry the flow, plus two read-only helpers the screens need. Frontend and backend can
proceed in parallel from Day 1 once this is agreed, which is the only reason it is written before
either exists.

The screens that consume it are [10-frontend-ux.md](10-frontend-ux.md); the flow it serves is
[03-flow.md](03-flow.md).

## The endpoints

| Method | Path | Description | Returns |
|---|---|---|---|
| `POST` | `/auth/wallet/challenge` | Issue a nonce challenge for an address and network | `200` |
| `POST` | `/auth/wallet/verify` | Verify the signature → session | `200` |
| `POST` | `/requests` | Create a payment request. **`Idempotency-Key` header mandatory** | `202` |
| `GET` | `/requests` | The list, with status and signature progress | `200` |
| `GET` | `/requests/:id` | One request in detail | `200` |
| `POST` | `/requests/:id/prepare` | Build the unsigned transaction, lock the sequence, return XDR + transaction hash + `valid_until` | `200` |
| `POST` | `/requests/:id/signatures` | An approver submits a signature over the transaction hash | `200` |
| `POST` | `/requests/:id/reject` | Refuse. Releases the sequence, calls `cancel_request` | `202` |
| `GET` | `/requests/:id/evidence` | The per-milestone transaction hash table for the evidence page | `200` |

Two read-only helpers sit outside the request lifecycle. Neither writes anything.

| Method | Path | Description | Returns |
|---|---|---|---|
| `GET` | `/accounts/{address}/can-receive` | Can this address be paid in `?asset=`? Existence plus trustline | `200` |
| `GET` | `/treasury` | The treasury's signers and thresholds, read back from Horizon — evidence E2 | `200` |

`can-receive` answers `200` with a verdict — `{ exists, can_receive }` — rather than an error status.
"This address cannot receive USDC" is a successful answer to the question asked, and reserving the
error envelope for genuine failures keeps a highlighted field from looking like a broken API. It is
called from the address field behind a 500 ms debounce, because a missing trustline otherwise
surfaces as `op_no_trust` at SUBMIT — after both approvers have already been interrupted to sign.

Everything hangs off `/requests/:id`. There is no separate settlement resource in the API, because
in SOW 1 a request has at most one settlement and exposing both would invite the interface to track
two identifiers for one thing.

## Why chain writes return 202

**Every endpoint that touches the chain returns `202 Accepted`, not `200 OK`.**

This is how the API contract pushes "the chain is the source of truth" all the way down to the
button. There is no path by which a client can read "the API returned OK" as "the money moved". If
an endpoint that touches the chain returns `200`, that is a design defect, not a preference.

`POST /requests` and `POST /requests/:id/reject` return `202`. `POST /requests/:id/prepare` and
`/signatures` return `200`, because neither submits anything — `prepare` reads the network and
`signatures` writes a row.

## Authentication

| Prefix | Posture |
|---|---|
| `/auth/*` | Public. It is where a session comes from |
| everything else | A session from `/auth/wallet/verify`, sent as `Authorization: Bearer …` |

The challenge is a message, not a submittable transaction. A design that makes somebody sign a real
transaction to log in is one where a wallet can be tricked into signing a payment.

Every login failure returns the same message with the same latency. Nothing reveals whether an
address is enrolled.

## Request and response shapes

### `POST /auth/wallet/challenge`

```jsonc
// Request
{ "address": "GDX7XQFP...P4LM" }

// 200 OK
{
  "nonce": "79f8dbc1…",           // 32 bytes, hex. Single use.
  "transaction_xdr": "AAAAAgAAA…", // sequence number 0 — cannot be submitted
  "domain": "app.settleflow.internal",
  "network_passphrase": "Test SDF Network ; September 2015",
  "expires_at": 1787169547
}
```

### `POST /auth/wallet/verify`

```jsonc
// Request — the envelope exactly as the wallet returned it
{ "nonce": "79f8dbc1…", "signed_xdr": "AAAAAgAAA…" }

// 200 OK
{ "token": "eyJhbGciOi…", "wallet": "GDX7…P4LM", "expires_at": 1787198347 }
```

### `POST /requests`

```jsonc
// Request. Idempotency-Key: 6f1c… (UUID v4, generated when the form opens)
{
  "payee_address": "GDX7XQFP...P4LM",
  "amount":        "4500.0000000",   // STRING — JSON numbers are IEEE doubles
  "asset_code":    "USDC",
  "reference":     "PO-2026-0417"
}
```

```jsonc
// 202 Accepted
{
  "request_id":  "3f9a7b1e...",      // hex, 32 bytes — the SAME id as on chain
  "status":      "pending",
  "create_tx_hash": null,            // filled once the readback confirms (S-10)
  "server_time": "2026-08-19T14:22:10Z"
}
```

The response is `202` with `status: "pending"` and a null transaction hash, because at that moment
the write has been submitted and not yet confirmed. The interface shows "writing to chain" until a
subsequent `GET` returns a hash — which is step S-10, expressed in the contract rather than left to
the client's discretion.

### `POST /requests/:id/prepare`

```jsonc
// 200 OK
{
  "settlement_id":   "0193f2b8-...",
  "unsigned_xdr":    "AAAAAgAAAAB...",
  "tx_hash":         "c204ab...88af",   // approvers sign EXACTLY this
  "network_passphrase": "Test SDF Network ; September 2015",
  "valid_until":     "2026-08-19T15:22:10Z",   // the timebounds maxTime
  "required_weight": 2,
  "collected_weight": 0,
  "signers": [
    { "address": "GCFO...", "role": "CFO", "signed": false },
    { "address": "GCEO...", "role": "CEO", "signed": false },
    { "address": "GAUD...", "role": "Internal control", "signed": false }
  ]
}
```

`prepare` is the one endpoint that must reach the network — the sequence number exists only there
and cannot be cached. Its p95 is about 600 ms and is published rather than disguised.

A second concurrent `prepare` against a treasury that already has a settlement in flight receives
`409 RELEASE_IN_FLIGHT`. That is feature F-09 surfacing in the API: refused cleanly at the start,
rather than accepted and killed later by `tx_bad_seq`.

### `POST /requests/:id/signatures`

```jsonc
// Request
{ "signer_address": "GCFO...", "signature": "base64..." }

// 200 OK
{ "collected_weight": 1, "required_weight": 2, "threshold_met": false }
```

Three rejections, three codes: a signer outside the pinned policy is `NOT_A_SIGNER`; a signature over
any other hash is `BAD_SIGNATURE`; a second signature from the same signer is `ALREADY_SIGNED`.

When `threshold_met` becomes `true`, submission happens in a worker. The approver's request returns
as soon as the signature is stored — it does not wait for Horizon.

## The error envelope

Every error, from every endpoint, has the same shape.

```jsonc
{
  "error": {
    "code":      "RELEASE_IN_FLIGHT",   // machine-readable, stable, never renamed
    "message":   "A payment from this treasury is already awaiting signatures.",
    "field":     "amount",              // so the UI highlights the right input
    "details":   { "settlement_id": "0193f2b8-..." },
    "retriable": false,
    "trace_id":  "01J8XKQ2..."          // give this to support
  }
}
```

`code` is stable and machine-readable; `message` is for a person and may be translated. `field` is
what makes S-08 possible — the error lands under the input that caused it rather than in a generic
toast.

## Status codes

| Code | When | What the client does |
|---|---|---|
| `200` | A read, or a write that touches nothing on chain | Render |
| `202` | Accepted, outcome not yet known — creating a request, rejecting one | Move to a pending state, poll. **Never show success** |
| `400` | Validation failed | Highlight the field. Do not retry |
| `401` | No session, or it expired | Re-authenticate |
| `403` | Not a signer on this settlement | Explain; do not offer a retry |
| `404` | Not found — including exists-but-not-yours | Terminal. Do not probe further |
| `409` | Idempotency in flight, or `RELEASE_IN_FLIGHT` | Wait and poll. **Do not change the idempotency key** |
| `422` | The current state does not permit this action | Re-read the entity and update the UI |
| `429` | Rate limited | Back off per `Retry-After` |
| `503` | Read-model lag beyond threshold | Show "synchronising", block writes |

`404` covers "exists but is not yours" deliberately. Distinguishing the two tells an unauthorised
caller that a request id is real.

`409` on idempotency means the same key is currently in flight. The correct client behaviour is to
wait and poll with the **same** key; generating a new one is how a duplicate gets created.

## Error codes

| Code | Meaning |
|---|---|
| `INVALID_ADDRESS` | The vendor address is not a valid Stellar public key |
| `INVALID_AMOUNT` | Zero, negative, or more than seven decimal places |
| `ASSET_NOT_SUPPORTED` | Not USDC or XLM |
| `INVALID_REFERENCE` | Empty, or longer than 64 characters |
| `IDEMPOTENCY_CONFLICT` | The same key, a different payload |
| `DESTINATION_NO_TRUSTLINE` | The vendor cannot receive this asset |
| `RELEASE_IN_FLIGHT` | A payment from this treasury is already awaiting signatures |
| `NOT_A_SIGNER` | The address is not in the policy pinned to this settlement |
| `BAD_SIGNATURE` | The signature does not verify against this transaction hash |
| `ALREADY_SIGNED` | This signer has already signed this settlement |
| `TX_EXPIRED` | The timebounds have lapsed; rebuild and collect signatures again |
| `NOT_PAYABLE` | The request is paid, cancelled or expired |
| `SYNCING` | The read-model is behind; writes are blocked |

Codes are stable. Renaming one breaks the interface's copy table and any operator runbook that maps
codes to actions.

## The rules this document imposes

- **Chain writes return 202.** An endpoint touching the chain that returns 200 is a defect.
- **Amounts cross the wire as strings.** JSON numbers are doubles.
- **`Idempotency-Key` is mandatory on `POST /requests`**, and a `409` never means "use a new key".
- **The error shape is identical everywhere**, including `trace_id`.
- **`404` hides existence.** Not-yours and not-found are the same response.
- **Error codes are stable identifiers**, not prose.
