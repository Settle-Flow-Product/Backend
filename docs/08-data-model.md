# 08 · Data model

PostgreSQL 17. Only the tables that earn their place — modules F1–F6, plus two the deck's list
implies but does not name.

The chain is the source of truth for a request's status; these tables are how the interface filters
and displays without waiting on Horizon, and how the signature service holds state that has nowhere
else to live. Contract storage is [04-contract-interface.md](04-contract-interface.md).

## Four design decisions

**1 · `numeric(30,7)`, never a float.** Stellar uses seven decimal places. `numeric` compares exactly,
which is what an exact-amount check requires. A `double precision` anywhere in a money path is a bug
waiting for the right value. In Rust the same number is an `i128` count of stroops.

**2 · `request_id` is `bytea(32)`, not a `uuid`.** It is `sha256(business_id ‖ idempotency_key)` and
it is *also* the key on chain. One value for both eliminates an entire class of mapping bugs, and it
makes retry-after-timeout idempotent at the contract level — see F-04 and S-11.

**3 · A policy is versioned, never edited.** Changing a signer or a threshold writes a new row with
`version + 1`. A settlement pins the version it was authorised under, so months later the question
"under which rule was this approved?" has an answer.

**4 · Three tables are append-only, enforced by permissions.** `payments`-style history, the ingest
log and the audit trail are never updated or deleted, and the application role does not hold the
grant to try.

## The tables

| Migration | Table | Module | Why it exists |
|---|---|---|---|
| 0001 | *(types and domains)* | — | One place where every enum and money type is defined |
| 0002 | `auth_challenges` | F-01 | A nonce that is not stored cannot be single-use |
| 0003 | `governance_policies`, `policy_signers` | B4 | A settlement cannot pin a version that is not stored |
| 0004 | `payment_requests`, `idempotency_keys` | F1, F5 | Fast filtering and display; duplicate protection |
| 0005 | `settlements` | F2 | Links a request to its transaction, and holds the reserved sequence |
| 0006 | `settlement_approvals` | F3 | Each approver's signature and weight |
| 0007 | `chain_events`, `ingest_cursor` | F4 | The ingest log and the read position |
| 0008 | `audit_events` | F6 | Who did what, when, from which state |

Migration files are named as prose sentences describing the change, in both directions:
`0005_a_settlement_reserves_one_sequence.up.sql` and `.down.sql`. CI applies every `up` in order,
then every `down` in reverse, and fails if any table **or enum type** survives.

---

## 0001 · Types and domains

```sql
CREATE EXTENSION IF NOT EXISTS "pgcrypto";
CREATE EXTENSION IF NOT EXISTS "citext";

-- Mirrors ReqStatus on the contract. 'expired' is DERIVED at this layer: the contract
-- has no Expired status, it compares ledger.timestamp() against expiry on every call.
CREATE TYPE request_status AS ENUM ('pending','approved','paid','cancelled','expired');

-- Backend-only. 'unresolved' has no on-chain counterpart and is a first-class state.
CREATE TYPE settle_status  AS ENUM ('draft','awaiting_approval','releasing',
                                    'settled','failed','unresolved');

CREATE TYPE idem_state     AS ENUM ('in_flight','done','released');
CREATE TYPE actor_type     AS ENUM ('business','approver','system','ops');

-- Every amount in the system passes through this domain.
CREATE DOMAIN stellar_amount AS numeric(30,7) CHECK (VALUE >= 0);
CREATE DOMAIN stellar_addr   AS text CHECK (VALUE ~ '^G[A-Z2-7]{55}$');
```

`stellar_addr` is the same regular expression the create-request form validates against
([10-frontend-ux.md](10-frontend-ux.md)), so a malformed address is refused at three layers: the
field, the API, and the column.

## 0002 · A wallet proves who you are

```sql
CREATE TABLE auth_challenges (
    nonce        bytea PRIMARY KEY CHECK (octet_length(nonce) = 32),
    address      stellar_addr NOT NULL,
    network      text NOT NULL,
    domain       text NOT NULL,
    issued_at    timestamptz NOT NULL DEFAULT now(),
    expires_at   timestamptz NOT NULL,
    CONSTRAINT ck_challenge_ttl CHECK (expires_at > issued_at)
);
CREATE INDEX ix_challenge_expiry ON auth_challenges (expires_at);
```

Verification **takes** the row — reads and deletes in one statement — so a nonce cannot be replayed
even against a slow client. Expiry is 300 seconds and is not extended; a lapsed challenge is
discarded and a new one issued.

The challenge is a message to sign, never a submittable transaction. A design that makes somebody
sign a real transaction to log in is a design where a wallet can be tricked into signing a payment.

## 0003 · A policy has versions, not edits

```sql
CREATE TABLE governance_policies (
    id           uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    version      int  NOT NULL,
    threshold    int  NOT NULL CHECK (threshold >= 1),
    treasury_address text NOT NULL,
    active       boolean NOT NULL DEFAULT false,
    created_at   timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT uq_policy_version UNIQUE (treasury_address, version)
);

-- Exactly one active policy per treasury — enforced here, not in application code.
CREATE UNIQUE INDEX ux_policy_active ON governance_policies (treasury_address) WHERE active;

CREATE TABLE policy_signers (
    policy_id       uuid NOT NULL REFERENCES governance_policies(id) ON DELETE CASCADE,
    signer_address  stellar_addr NOT NULL,
    weight          int  NOT NULL CHECK (weight BETWEEN 1 AND 255),
    role            text NOT NULL DEFAULT 'approver',
    PRIMARY KEY (policy_id, signer_address)
);
```

A trigger on activation asserts that the sum of signer weights is at least the threshold. A policy
whose signers cannot reach its own threshold is a trap: the business can never pay anybody, and
discovers it at the worst moment. This is the database-side counterpart of the B3 lock-out guard in
[05-treasury-multisig.md](05-treasury-multisig.md).

## 0004 · A request is created exactly once

```sql
CREATE TABLE payment_requests (
    id              bytea PRIMARY KEY CHECK (octet_length(id) = 32),
    -- = sha256(business_id ‖ idempotency_key), IDENTICAL to the id on chain

    creator_address stellar_addr NOT NULL,
    payee_address   stellar_addr NOT NULL,          -- THE VENDOR
    asset_code      text NOT NULL,                  -- 'USDC' | 'XLM'
    asset_contract  text NOT NULL,                  -- the SAC address
    amount          stellar_amount NOT NULL CHECK (amount > 0),

    reference       text  NOT NULL CHECK (char_length(reference) BETWEEN 1 AND 64),
    reference_hash  bytea NOT NULL CHECK (octet_length(reference_hash) = 32),
    -- The plaintext stays here. Only the hash goes on chain.

    expiry_at       timestamptz NOT NULL,
    status          request_status NOT NULL DEFAULT 'pending',

    contract_id     text NOT NULL,                  -- the registry it was written to
    create_tx_hash  text,
    idempotency_key uuid NOT NULL,
    created_at      timestamptz NOT NULL DEFAULT now(),
    version         int NOT NULL DEFAULT 1,         -- optimistic lock

    CONSTRAINT uq_idem UNIQUE (creator_address, idempotency_key),
    CONSTRAINT ck_expiry_future CHECK (expiry_at > created_at)
);

CREATE INDEX ix_req_status ON payment_requests (status, created_at DESC);
CREATE INDEX ix_req_payee  ON payment_requests (payee_address);
CREATE INDEX ix_req_expiry ON payment_requests (expiry_at) WHERE status IN ('pending','approved');

CREATE TABLE idempotency_keys (
    scope       text NOT NULL,               -- 'create_request' | ...
    key         uuid NOT NULL,
    body_hash   bytea NOT NULL,
    state       idem_state NOT NULL DEFAULT 'in_flight',
    result_ref  text,
    created_at  timestamptz NOT NULL DEFAULT now(),
    expires_at  timestamptz NOT NULL DEFAULT now() + interval '24 hours',
    PRIMARY KEY (scope, key)
);
```

`uq_idem` is feature F-04 made structural. The application claims the key before doing anything, so
the second of two concurrent clicks loses the race at the database rather than at the contract —
and the contract's `RequestExists` gate catches whatever slips past.

`body_hash` distinguishes "the same call again" from "a different call reusing a key". The first
returns the original result; the second is an error, because silently returning someone else's
result is worse than refusing.

## 0005 · A settlement reserves one sequence

```sql
CREATE TABLE settlements (
    id                  uuid PRIMARY KEY DEFAULT gen_random_uuid(),

    -- Deliverable 2 builds an unsigned tx FROM AN APPROVED REQUEST. Without this
    -- column there is no link between a request and its transaction, and the memo
    -- reconciliation key means nothing.
    request_id          bytea NOT NULL REFERENCES payment_requests(id),

    treasury_address    text NOT NULL,               -- supplies the sequence number
    destination_address stellar_addr NOT NULL,       -- copied from the request's payee
    asset_code          text NOT NULL,
    amount              stellar_amount NOT NULL CHECK (amount > 0),

    policy_version      int NOT NULL,                -- PINNED at authorisation
    required_weight     int NOT NULL,
    collected_weight    int NOT NULL DEFAULT 0,

    status              settle_status NOT NULL DEFAULT 'draft',

    unsigned_xdr        text,
    tx_hash_to_sign     text,                        -- what approvers sign
    reserved_seq        bigint,                      -- the held treasury sequence
    valid_until         timestamptz,                 -- the transaction's timebounds maxTime

    submit_attempted_at timestamptz,                 -- the CORRECT idempotency guard
    tx_hash             text,                        -- the submitted transaction
    settled_at          timestamptz,

    rejected_by         stellar_addr,
    rejected_at         timestamptz,

    created_at          timestamptz NOT NULL DEFAULT now(),
    version             int NOT NULL DEFAULT 1,

    CONSTRAINT ck_settled_needs_tx CHECK (
        status <> 'settled' OR (tx_hash IS NOT NULL AND settled_at IS NOT NULL))
);

-- F-09 · exactly ONE settlement in flight per TREASURY ACCOUNT.
-- Not per request, not per asset: signatures bind to the treasury's sequence number,
-- so any two payments on the same treasury contend for the same seq.
CREATE UNIQUE INDEX ux_settle_inflight ON settlements (treasury_address)
    WHERE status IN ('awaiting_approval','releasing');

-- A reserved sequence belongs to exactly one settlement.
CREATE UNIQUE INDEX ux_settle_seq ON settlements (treasury_address, reserved_seq)
    WHERE reserved_seq IS NOT NULL;

CREATE UNIQUE INDEX ux_settle_tx  ON settlements (tx_hash) WHERE tx_hash IS NOT NULL;
CREATE INDEX ix_settle_request    ON settlements (request_id);
```

**`ux_settle_inflight` is the single most important line in this schema.** It is feature F-09, and
keying it on anything narrower than the treasury address reproduces exactly the failure it exists to
prevent — a second payment accepted, signed by two people, and killed by `tx_bad_seq`.

`submit_attempted_at` is separate from `status` because the status guard is not idempotent: the
status is `releasing` for the whole duration of a submit, so a redelivered message would pass a
status check and submit twice. See C5 in [06-signature-service.md](06-signature-service.md).

## 0006 · One signer signs once

```sql
CREATE TABLE settlement_approvals (
    settlement_id   uuid NOT NULL REFERENCES settlements(id) ON DELETE CASCADE,
    signer_address  stellar_addr NOT NULL,
    signature       bytea NOT NULL,
    weight          int  NOT NULL,
    tx_hash_signed  text NOT NULL,          -- the hash this signature is over
    signed_at       timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (settlement_id, signer_address)
);
```

The primary key is check V3 from C3, expressed where it cannot be forgotten. One signer contributing
their weight twice would reach a threshold of 2 alone, which is the entire failure mode multisig
exists to prevent.

`tx_hash_signed` is stored rather than assumed. If a settlement is ever rebuilt, signatures against
the old hash are visibly stale rather than silently wrong.

## 0007 · The chain is read through a cursor

```sql
CREATE TABLE chain_events (            -- APPEND-ONLY
    id          bigserial PRIMARY KEY,
    tx_hash     text NOT NULL,
    op_index    int  NOT NULL,
    ledger_seq  bigint NOT NULL,
    ledger_time timestamptz NOT NULL,   -- LEDGER time, not now()
    contract_id text,
    topic       text NOT NULL,          -- req_new | req_appr | req_paid | req_canc
    request_id  bytea,
    payload     jsonb NOT NULL,
    quarantined boolean NOT NULL DEFAULT false,
    processed_at timestamptz,

    -- D2 · absolute deduplication. Two daemon instances stay safe on this line alone.
    CONSTRAINT uq_chain_ev UNIQUE (tx_hash, op_index)
);
CREATE INDEX ix_chain_req  ON chain_events (request_id) WHERE request_id IS NOT NULL;
CREATE INDEX ix_chain_quar ON chain_events (ledger_seq) WHERE quarantined;

CREATE TABLE ingest_cursor (           -- exactly one row
    id         bool PRIMARY KEY DEFAULT true CHECK (id),
    ledger_seq bigint NOT NULL,
    updated_at timestamptz NOT NULL DEFAULT now()
);
```

`CHECK (id)` on a `bool` primary key defaulting to `true` is how a table is constrained to a single
row. A second cursor row would mean two readers disagreeing about where they are.

## 0008 · Every state change leaves an audit row

```sql
CREATE TABLE audit_events (            -- APPEND-ONLY
    id          bigserial PRIMARY KEY,
    actor_type  actor_type NOT NULL,
    actor_id    text,
    entity      text NOT NULL,         -- 'payment_request' | 'settlement' | 'policy'
    entity_id   text NOT NULL,
    action      text NOT NULL,
    from_state  text,
    to_state    text,
    metadata    jsonb NOT NULL DEFAULT '{}',
    trace_id    text,
    at          timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX ix_audit_entity ON audit_events (entity, entity_id, at DESC);
```

`trace_id` is what connects a click in the interface to a row here to a chain event. The Definition
of Done requires it end to end.

## Append-only, enforced by grant

```sql
REVOKE UPDATE, DELETE ON chain_events, audit_events FROM app_rw;
```

Only the migration role can touch them. The application role does not hold the permission to try, so
append-only is a property of the system rather than a promise in a code review.

## The two indexes that carry the correctness

| Index | Prevents |
|---|---|
| `ux_settle_inflight ON settlements (treasury_address)` | Two payments contending for one sequence number — F-09 |
| `uq_chain_ev UNIQUE (tx_hash, op_index)` | An event being processed twice, and makes parallel ingest safe — D2 |

Both replace a great deal of defensive application logic, and — more importantly — both stay correct
when two instances of a service run at the same time, which application logic does not.

## The rules this document imposes

- **Money is `numeric(30,7)`.** Never `float`, never `double precision`.
- **`request_id` is one value shared with the chain**, derived, never random.
- **A policy is versioned and pinned**, never edited in place.
- **`ux_settle_inflight` keys on the treasury address.** Nothing narrower.
- **`submit_attempted_at` is the idempotency guard**, not `status`.
- **Append-only tables are enforced by `REVOKE`**, not by convention.
- **Every migration reverses**, and CI proves it on every push.
