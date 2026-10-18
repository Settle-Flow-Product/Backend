# 02 · Modules

The official work list: 42 modules in seven groups. Each has a code, a deliverable, an owner and a
completion condition. This table is meant to be copied straight into an issue tracker.

Which feature each module serves is [01-features.md](01-features.md); the schedule that orders them
is [14-roadmap.md](14-roadmap.md).

| Group | Modules | Deliverable |
|---|---|---|
| A · Smart contract — registry | 9 | 1 · 35% · $1,750 |
| B · Treasury and multisig | 5 | 2, part one |
| C · Signature collection service | 8 | 2, part two |
| D · Chain synchronisation, minimal | 4 | supports 2 and 3 |
| E · Minimum interface | 5 | supports all three |
| F · Data | 6 | supports 2 and 3 |
| G · Deploy and demo | 5 | 3 · 30% · $1,500 |

> **The deck says 37; its own group counts sum to 42.**
>
> A (9) + B (5) + C (8) + D (4) + E (5) + F (6) + G (5) = 42. Every per-group number here matches
> the deck exactly, so the total is an arithmetic slip in the source rather than a missing group.
> Recorded here so nobody reconciles the two by quietly deleting five rows.

Groups **C** and **D** concentrate the technical risk. Group **G** concentrates the acceptance risk:
finishing every other module and arriving without transaction hashes still scores as missing
evidence.

---

## A · Smart contract — registry

*Deliverable 1 · 35% · $1,750 · Days 1–4*

| Code | Module | Content | Owner |
|---|---|---|---|
| **A1** | Storage layout and types | `DataKey` (`Admin`, `Settler`, `Req`); `PaymentRequest` carrying creator, payee, asset, amount, reference, expiry, status, `paid_tx`; `ReqStatus` with four values and twelve error codes | Contract dev |
| **A2** | `create_request()` | Record a new payment. `require_auth` on the creator; check `amount > 0` and expiry in the future; reject a duplicate id | Contract dev |
| **A3** | `approve()` | `Pending → Approved`. Settler only, after the backend has collected weight ≥ threshold | Contract dev |
| **A4** | `mark_paid()` | `Approved → Paid` with the transaction hash. Refuse a second call, refuse past expiry, check the amount matches | Contract dev |
| **A5** | `cancel_request()` | The reject branch. `require_auth` on the creator; cancellable only while not yet paid | Contract dev |
| **A6** | `get_request()` · `is_payable()` | Read functions, called through RPC simulation. Never submitted as a transaction | Contract dev |
| **A7** | Events | `req_new`, `req_appr`, `req_paid`, `req_canc` — the source for chain synchronisation | Contract dev |
| **A8** | TTL management | Extend TTL on both persistent and instance storage at every entry point. Forgetting instance kills the entire contract when it expires | Contract dev |
| **A9** | Unit test suite | 16 cases: full lifecycle, abuse resistance, boundaries. Line coverage ≥ 90% for the registry crate | Contract dev + Tester |

## B · Treasury and multisig

*Deliverable 2, part one · Days 5–8*

| Code | Module | Content | Owner |
|---|---|---|---|
| **B1** | Treasury creation script | Fund via friendbot, establish the testnet USDC trustline, re-runnable from scratch in under ten minutes | Backend |
| **B2** | Three-signer configuration | Three signers at weight 1, thresholds low/med/high = 2, and `masterWeight = 0`. Without the last step the multisig is decoration | Backend |
| **B3** | Lock-out guard | Before every `setOptions`: check `Σ weight(active signers) + masterWeight ≥ max(low, med, high)` against the **new** values | Backend |
| **B4** | Policy and signer enrolment | Create a new policy version rather than editing in place. A settlement pins `policy_version` at authorisation | Backend |
| **B5** | Configuration verification script | Read the account back from Horizon and assert three signers, the correct thresholds, master weight 0. Its output is submitted as evidence | Tester |

## C · Signature collection service

*Deliverable 2, part two · Days 9–17*

| Code | Module | Content | Owner |
|---|---|---|---|
| **C1** | `build_unsigned()` | Build the treasury → vendor payment from an approved request. Memo = request id, for reverse lookup. Timebounds of one hour, mandatory | Backend |
| **C2** | Sequence number locking | Hold the sequence in the database, unique index on `(treasury_address, reserved_seq)`. One in-flight payment per treasury | Backend + Tech lead |
| **C3** | `submit_signature()` | Verify ed25519 over the exact transaction hash; the signer must belong to the pinned policy; the primary key blocks a duplicate signature | Backend |
| **C4** | Weight accumulation and threshold detection | Return `collected / required` progress to the interface; emit an event when the threshold is met | Backend |
| **C5** | Assemble and submit | `DecoratedSignature` with the correct hint, send to Horizon, record the transaction hash | Backend |
| **C6** | Unresolved outcome handling | `tx_bad_seq` and timeout are handled identically: read the chain by memo first, resubmit the same envelope, rebuild only when certain nothing landed | Tech lead |
| **C7** | Fee bump | On a low fee, wrap the already-signed envelope in a fee bump — approver signatures stay valid and nobody is asked again | Backend |
| **C8** | The reject branch | A rejected request stops submission entirely, releases the reserved sequence, and writes an audit row | Backend |

## D · Chain synchronisation — minimal

*Supports Deliverables 2 and 3 · Days 9–13*

| Code | Module | Content | Owner |
|---|---|---|---|
| **D1** | Cursor ingest | Read events from Horizon and RPC by cursor. The cursor advances only after a successful commit | Tech lead |
| **D2** | Absolute deduplication | `UNIQUE (tx_hash, op_index)` at the database layer. Two daemon instances running in parallel remain safe | Tech lead |
| **D3** | Readback before writing paid | Never infer `paid` from a submit result. Write only after reading the ledger back | Tech lead |
| **D4** | Read-model updates | The interface reads the read-model; no synchronous Horizon call sits inside a request path | Backend |

## E · Minimum interface

*Enough to demonstrate — not a dashboard*

| Code | Module | Content | Owner |
|---|---|---|---|
| **E1** | Wallet sign-in | Connect a wallet, sign the nonce challenge, open an internal session | Frontend |
| **E2** | Create-request screen | The four SOW fields, validated at the field, confirmed before writing to chain | Frontend |
| **E3** | Request list | `Pending` / `Approved` / `Paid` / `Cancelled`, signature progress, basic filtering | Frontend |
| **E4** | Approver signing screen | Show vendor, amount, reference, who has signed, and the transaction's deadline. Connect a wallet and sign | Frontend |
| **E5** | Evidence page | The per-milestone transaction hash table with Stellar Expert links — this *is* the evidence submitted to the Chapter Lead | Frontend |

## F · Data — only the tables that earn their place

| Code | Table | Why it is needed in SOW 1 | Owner |
|---|---|---|---|
| **F1** | `payment_requests` | An off-chain copy for fast filtering and display. The chain remains the source of truth | Backend |
| **F2** | `settlements` | Carries `request_id`, `treasury_address`, `reserved_seq`, `submit_attempted_at`. Without these four columns there is no link between a request and its transaction | Backend |
| **F3** | `settlement_approvals` | Each approver's signature with its weight. The primary key blocks a duplicate signature | Backend |
| **F4** | `chain_events` + `ingest_cursor` | An append-only ingest log and the read position. The backbone of correctness | Tech lead |
| **F5** | `idempotency_keys` | Prevents a duplicate request when the network is unreliable | Backend |
| **F6** | `audit_events` | Who did what, when, from which state. Append-only, with `UPDATE`/`DELETE` revoked | Backend |

Two supporting tables not in the deck's list but required by **B4** and **F-01**:
`governance_policies` + `policy_signers` (a settlement cannot pin a policy version that is not
stored), and `auth_challenges` (a nonce that is not stored cannot be single-use). Both are argued
for in [08-data-model.md](08-data-model.md).

## G · Deploy and demo

*Deliverable 3 · 30% · $1,500 · Days 18–20*

| Code | Module | Content | Owner |
|---|---|---|---|
| **G1** | Testnet deploy script | Deploy contract and treasury from scratch with no manual step. Re-runnable if testnet resets | Contract dev |
| **G2** | Round-trip demo scenario | Create a request → collect 2 of 3 signatures → pay the vendor → registry marks paid | Tech lead |
| **G3** | Transaction hash collection | Record the hash at each milestone into the evidence table, with Stellar Expert links | Tester |
| **G4** | README and deploy guide | An outsider reads it and redeploys from scratch unaided | Tech lead |
| **G5** | Demo video, 3–5 minutes | So the Chapter Lead can review without deep technical knowledge | Tech lead |

---

## How the modules map onto the repository

| Group | Lives in |
|---|---|
| A | `smart-contract/src/` |
| B | `backend/scripts/`, `backend/crates/settle-chain` |
| C | `backend/crates/settle-signature`, `settle-chain` |
| D | `backend/crates/settle-indexer`, `settle-backend/src/indexer.rs` |
| E | `frontend/website/` |
| F | `backend/migrations/`, `backend/crates/settle-db` |
| G | `smart-contract/scripts/`, `.deploy/evidence.md`, `README.md` |

## The rules this document imposes

- **A module is done when its completion condition is observable**, not when its code compiles.
- **C2 lands on Day 9.** It is the one module whose absence is only discovered when it has already
  wasted two approvers' time.
- **C6 and C8 are owned by the Technical Lead**, because both are paths where a mistake spends money
  twice.
- **G3 runs during the demo, not after it.** A hash not captured at the moment is a hash gone.
