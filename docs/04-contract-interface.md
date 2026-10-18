# 04 · Contract interface

`payment_request_registry` — the Soroban contract for Deliverable 1. It records and verifies the
lifecycle of a payment request. It never touches a token.

This is the hard interface. **Gate order is part of the specification, not an implementation
detail.** Tests assert the exact error code, never merely that a call failed, so reordering two
gates is a breaking change even when both still reject. The test matrix is
[12-testing.md](12-testing.md); the treasury and signature flow around this contract are
[05-treasury-multisig.md](05-treasury-multisig.md) and
[06-signature-service.md](06-signature-service.md).

## Why this can ship without an audit

There is no function here that can move an asset. No `token::Client`, no `transfer`, no call to a
Stellar Asset Contract anywhere in the crate. The custody risk surface is zero, which is what makes
twenty working days to testnet an honest estimate.

That property is checked mechanically, not remembered. It is the **first item on the Day 4 review
checklist**, and CI enforces it on every push:

```bash
# Any hit fails the build.
grep -rn -e 'token::Client' -e 'TokenClient' -e '\.transfer(' smart-contract/src/ && exit 1
```

If a token call ever appears, the contract has left the no-custody scope of SOW 1 and needs a
security review before it reaches any network.

## Surface at a glance

| Function | Who may call | Condition | Touches assets |
|---|---|---|---|
| `__constructor` | The deployer, inside the deployment transaction | Runs exactly once, by construction. Extends the instance TTL here | No |
| `create_request` | The business, for itself | `amount > 0`, expiry in the future, id not already present | No |
| `approve` | Settler only | Currently `Pending`, not expired. The backend calls it once weight suffices | No |
| `mark_paid` | Settler only | No `paid_tx` yet, currently `Approved` *or* `Pending`, not expired, amount matches | No |
| `cancel_request` | The business that created it | Not yet `Paid` | No |
| `get_request` · `is_payable` | Anyone — read through RPC simulation | — | No |

`get_request` and `is_payable` are read-only, but Soroban has no view functions — every entry point
goes through `InvokeHostFunction`. They are free only when called off chain via `simulateTransaction`
or `getLedgerEntries`. **Never submit them as transactions.**

## Types

```rust
// The SOW names three statuses directly: pending -> approved -> paid.
pub enum ReqStatus { Pending = 0, Approved = 1, Paid = 2, Cancelled = 3 }

pub struct PaymentRequest {
    pub id:        BytesN<32>,          // sha256(business_id ‖ idempotency_key)
    pub creator:   Address,             // the business that created it — the party that may cancel
    pub payee:     Address,             // THE VENDOR ADDRESS, per SOW Deliverable 1
    pub asset:     Address,             // the SAC of USDC, or of native XLM
    pub amount:    i128,                // stroops · 7 decimals · never a float
    pub reference: BytesN<32>,          // a HASH of the reference; the raw string never goes on chain
    pub expiry:    u64,                 // ledger timestamp, not a server clock
    pub status:    ReqStatus,
    pub paid_tx:   Option<BytesN<32>>,
}

pub enum DataKey {
    Admin,              // Instance    · the administrative address
    Settler,            // Instance    · the ONLY address that may approve or mark_paid
    Req(BytesN<32>),    // Persistent  · one entry per request
}
```

Nine fields, and no more. Every extra field on chain is storage rent nobody budgeted and a migration
nobody planned.

Three of them are worth arguing for explicitly.

**`payee` is the vendor.** The creator is the business paying; the payee is the supplier being paid.
This is the field that makes the contract an accounts-payable instrument rather than a receivable
one, and it is the field the SOW names.

**`reference` is a hash.** The raw string is a purchase order or invoice number — commercially
sensitive, and permanently public if written to the ledger. The plaintext stays in Postgres; the
chain holds `sha256` of it, which is enough to prove correspondence during reconciliation. This is
one of the three questions to settle before Day 1; see [16-risks.md](16-risks.md).

**`expiry` is a ledger timestamp.** It is compared against `env.ledger().timestamp()` and against
nothing else, ever.

## Errors

`#[contracterror] #[repr(u32)]`, numbered in banded decades so a code's range names the subsystem
that refused. **A code number never changes after the first public testnet deployment**, and a
retired code leaves its number permanently vacant — never recycled. Backends that map codes to
operator actions cannot survive renumbering.

| Band | Code | Name | Raised when |
|---|---|---|---|
| System 1–9 | 1 | `AlreadyInitialized` | **Reserved, unreachable** — the constructor runs once, by construction |
| | 2 | `NotInitialized` | **Reserved, unreachable** — there is no deployed-but-unconfigured state |
| | 3 | `SettlerNotSet` | **Reserved, unreachable** — the constructor always sets the settler |
| | 4–9 | *(vacant)* | Reserved |
| Request 10–19 | 10 | `RequestExists` | `create_request` with an id already in persistent storage |
| | 11 | `RequestNotFound` | No entry for this id — it never existed |
| | 12 | `InvalidAmount` | `amount <= 0` at create, or `amount != r.amount` at `mark_paid` |
| | 13 | `InvalidExpiry` | `expiry <= env.ledger().timestamp()` at create |
| | 14–19 | *(vacant)* | Reserved |
| Lifecycle 20–29 | 20 | `NotPending` | A transition requiring `Pending` found another status |
| | 21 | `NotApproved` | `mark_paid` on a request that is neither `Pending` nor `Approved` |
| | 22 | `AlreadyPaid` | `paid_tx` is already set |
| | 23 | `Expired` | `env.ledger().timestamp() >= r.expiry` |
| | 24–29 | *(vacant)* | Reserved |
| Authority 30–39 | 30 | `Unauthorized` | Authority checks the host does not already panic on |
| | 31–39 | *(vacant)* | Reserved |

Twelve codes, matching module **A1**. Four of them — 1, 2, 3 and 30 — are reserved and
unreachable; their numbers stay occupied because a code number is never reused. See
[12-testing.md](12-testing.md) for why each one cannot fire.

`require_auth` failures panic in the host rather than returning a code — Soroban behaviour, not a
choice. Tests for those cases assert a panic, not a code.

`Error` derives `Debug`. This is not cosmetic: without it, `.unwrap()` on a `Result<_, Error>` does
not compile in tests, and the entire exact-code assertion style becomes unavailable.

## Storage and TTL — module A8

```rust
const BUMP_THRESHOLD: u32 = 518_400;   // ~30 days IF a ledger closes every 5s
const BUMP_AMOUNT:    u32 = 725_760;   // ~42 days IF a ledger closes every 5s
```

Both are counts of **ledgers**, not seconds. Ledger close time is a network parameter and it can
fall; at 4 seconds, "42 days" becomes about 33.6. Recompute at build time rather than copying the
numbers forward.

| Key | Durability | Extended by |
|---|---|---|
| `Admin`, `Settler` | Instance | **Every entry point** |
| `Req(id)` | Persistent | Every function that reads or writes that request |

**Instance storage shares its TTL with the Wasm code, and if it expires every function fails** until
a `RestoreFootprint` operation runs. That is why A8 says extending the instance TTL is required at
every entry point, not only in the ones that write.

**An archived entry is not an absent one.** A transaction touching an archived key fails at apply
time, before it reaches the contract — so `get_request` does not return `RequestNotFound` and
`is_payable` does not return `false`. The call aborts. The backend must distinguish "never existed"
from "archived", because only the second is recoverable.

## The write discipline

Every mutating entry point follows the same order. Deviating is how a contract emits an event for a
change it then fails to make.

```
 1  require_auth on the correct role
 2  validate parameters
 3  load state
 4  guards, cheapest first
 5  apply changes
 6  emit the event
 7  extend TTL                 always last, always bounded
```

---

## `initialize(env, admin, settler)`

| | |
|---|---|
| Auth | `admin.require_auth()` |
| Gates | 1. instance already has `Admin` → `AlreadyInitialized` |
| Writes | `Admin`, `Settler`; extends the instance TTL |
| Events | none |
| Returns | `Result<(), Error>` |

The settler is a single address and is the only party that can move a request past `Pending`. Which
key that is — a dedicated backend key or the treasury account itself — is question 01 in
[16-risks.md](16-risks.md), and it must be settled before Day 1.

## `create_request(env, creator, payee, id, asset, amount, reference, expiry)`

| | |
|---|---|
| Auth | `creator.require_auth()` — a business creates only for itself |
| Gates, in order | 1. `amount <= 0` → `InvalidAmount`<br>2. `expiry <= env.ledger().timestamp()` → `InvalidExpiry`<br>3. `Req(id)` already present → `RequestExists` |
| Writes | `Req(id)` with `status = Pending`, `paid_tx = None` |
| Events | `("req_new", id)` → `(amount, expiry)` |
| Returns | `Result<PaymentRequest, Error>` |

**Gate 3 is on-chain idempotency, and it is why the id is derived rather than random.** A retry after
a network timeout re-derives the same id from the same `(business_id, idempotency_key)` pair, hits
`RequestExists`, and cannot produce a second request. The backend treats that error as "already
done" and reads the existing entry back — which is exactly step S-11 in [03-flow.md](03-flow.md).

## `approve(env, id)`

`Pending → Approved`. The step the SOW calls the budget approval gate.

| | |
|---|---|
| Auth | `settler.require_auth()` |
| Gates, in order | 1. `Settler` missing → `SettlerNotSet`<br>2. no `Req(id)` → `RequestNotFound`<br>3. `status != Pending` → `NotPending`<br>4. `ledger.timestamp() >= expiry` → `Expired` |
| Writes | `status = Approved` |
| Events | `("req_appr", id)` → `()` |
| Returns | `Result<(), Error>` |

**The contract does not count signatures.** It trusts the settler's authorisation, and the settler's
key is used only after the signature service has verified every signature against the pinned policy
version and summed the weights. Putting weight arithmetic on chain would mean putting the signer
roster on chain, which changes what a policy change costs and what it leaks.

## `mark_paid(env, id, amount, tx)`

`Approved → Paid`, after the payment has settled.

| | |
|---|---|
| Auth | `settler.require_auth()` — a business cannot mark its own request paid |
| Gates, in order | 1. `Settler` missing → `SettlerNotSet`<br>2. no `Req(id)` → `RequestNotFound`<br>3. `paid_tx.is_some()` → `AlreadyPaid`<br>4. `status` not in `{Pending, Approved}` → `NotApproved`<br>5. `ledger.timestamp() >= expiry` → `Expired`<br>6. `amount != r.amount` → `InvalidAmount` |
| Writes | `status = Paid`, `paid_tx = Some(tx)` |
| Events | `("req_paid", id)` → `(amount, tx)` |
| Returns | `Result<(), Error>` |

Gate 3 precedes gate 4 deliberately. A second `mark_paid` on an already-paid request must report
`AlreadyPaid` — specific and actionable — rather than `NotApproved`, which would be technically true
and diagnostically useless.

Gate 4 accepts `Pending` alongside `Approved` so a single-authority configuration can complete
without a no-op `approve`. In the configuration being demonstrated, `approve` always runs first.

**The backend calls this only after reading the settled transaction back from the ledger.** Calling
it on the strength of a successful submit is the defect this whole design exists to prevent, and it
is the third of the three money-losing features in [01-features.md](01-features.md).

## `cancel_request(env, id)`

The reject branch — module A5, feature F-12.

| | |
|---|---|
| Auth | `r.creator.require_auth()`, read from **stored state**, not from a parameter |
| Gates, in order | 1. no `Req(id)` → `RequestNotFound`<br>2. `creator.require_auth()`<br>3. `status` not in `{Pending, Approved}` → `NotPending` |
| Writes | `status = Cancelled` |
| Events | `("req_canc", id)` → `()` |
| Returns | `Result<(), Error>` |

Authority comes from stored state rather than a caller-supplied address. Taking it as a parameter
would let anyone pass their own address, authorise as themselves, and cancel a request they do not
own.

Cancelling after `Approved` is permitted, and necessary: an approved-but-unpaid request is exactly
the thing a business most needs to withdraw.

## `get_request(env, id) -> Result<PaymentRequest, Error>`

Returns the stored entry, or `RequestNotFound`. No authorisation — the data is not secret to anyone
holding the 32-byte id, and the id is not guessable.

## `is_payable(env, id) -> bool`

```rust
matches!(r.status, ReqStatus::Pending | ReqStatus::Approved)
    && env.ledger().timestamp() < r.expiry
    && r.paid_tx.is_none()
```

Returns `false` for a request that never existed. It does **not** return `false` for an archived
entry — that call aborts at apply time. The two cases need different handling.

## Events — module A7

Topic 0 is always `"settle"`. Topic 1 is abbreviated to fit `symbol_short!`, which caps at nine
characters — a constraint that surprises people at exactly the wrong moment. These four events are
the sole input to chain synchronisation ([07-chain-sync.md](07-chain-sync.md)).

| Topic 1 | Topic 2 | Data | Emitted by |
|---|---|---|---|
| `req_new` | `id` | `(amount, expiry)` | `create_request` |
| `req_appr` | `id` | `()` | `approve` |
| `req_paid` | `id` | `(amount, tx)` | `mark_paid` |
| `req_canc` | `id` | `()` | `cancel_request` |

Three properties every event has, and keeps: it names its request in topic 2, so an indexer filters
without decoding data; its fields never change after deployment, because an indexer in production is
parsing them; and it is emitted at step 6, after the state write, never before.

## The rules this document imposes

- **No function in this crate may move a token.** Enforced by a grep gate in CI.
- **Gate order is specified.** Tests assert exact codes; reordering gates is a breaking change.
- **Error numbers freeze at first public deployment.** Retired numbers stay vacant forever.
- **Ledger time decides expiry.** `env.ledger().timestamp()`, never a server clock.
- **Every entry point extends the instance TTL**, not only the entry it touched.
- **Archived is not absent.** The backend handles the two cases differently.
