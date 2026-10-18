# How the backend runs

One binary, seven crates, one database. This file explains what happens when you start it and what
happens when someone creates a payment — in the order the code actually executes, rather than in the
order the modules are listed.

The specification these crates implement is the numbered documentation in [`../`](../). This is the
tour.

## The shape

```
                          settle-backend                 the binary
                   http · config · state · tasks
                     indexer · submitter · main
                                |
        +-------------+---------+---------+-------------+
        |             |                   |             |
  settle-signature  settle-auth    settle-indexer       |
        |             |                   |             |
        +------+------+---------+---------+-------------+
               |                |
          settle-db       settle-chain
               |                |
               +-------+--------+
                       |
                 settle-domain                 depends on nothing
```

Read it bottom-up. `settle-domain` has no dependencies at all — it is pure functions over plain
types, which is what lets its rules be unit-tested in milliseconds and differentially tested against
the compiled contract. Nothing above it re-implements a rule that lives in it.

## What each crate is for

| Crate | It holds |
|---|---|
| `settle-domain` | Validation rules, the status enums, the stroops amount type, the `request_id` derivation, the error envelope. No I/O, no async, no database |
| `settle-chain` | Everything Stellar: build a transaction, compute its hash, verify an ed25519 signature, attach a decorated signature, decode a contract event, call the registry, find a transaction by memo |
| `settle-db` | Every SQL statement in the system, one module per concern. Runtime `sqlx::query`, never the macro |
| `settle-auth` | The wallet challenge, its verification, and the session token |
| `settle-signature` | Deliverable 2: build unsigned, reserve the sequence, accept a signature, submit, reconcile |
| `settle-indexer` | A pure fold from an ordered event stream to a projection, plus the cursor rules |
| `settle-backend` | The axum server, configuration, shared state, and the background tasks |

## Starting up

`main` does six things, in an order that is load-bearing:

```
1  connect to Postgres and run migrations        vault of last resort: fail here, not later
2  check the registry responds, the treasury exists
3  build the chain client
4  spawn the indexer and submitter tasks
5  bind the port
6  serve, with graceful shutdown
```

Step 2 comes before step 5 on purpose. A backend that binds its port and then discovers it cannot
reach the chain reports itself healthy while being useless, and a load balancer believes it.

Configuration is read once, from the environment, into a `Settings` struct. An `Option` sub-struct
means a subsystem is switched off. Half-configured is a boot failure rather than a runtime surprise:
if `SETTLER_SECRET` is set but `REGISTRY_CONTRACT_ID` is not, the process exits with a sentence
saying so.

## Creating a payment, step by step

Somebody fills in the form and clicks. This is what runs.

**1 · `POST /requests` arrives at `http.rs`.** The `Idempotency-Key` header is mandatory; without it
the request is refused before anything else happens.

**2 · `settle-domain` validates.** The vendor address against `^G[A-Z2-7]{55}$`, the amount as a
decimal string with at most seven places, the asset against the supported set, the reference length.
Each failure names its field, so the interface highlights the right input rather than showing a
toast.

**3 · `settle-db` claims the idempotency key.** This happens *before* anything is written anywhere
else. Four outcomes: claimed and we continue; already done, so return the original result; in flight,
so return `409`; same key with a different body, so return `IDEMPOTENCY_CONFLICT`.

**4 · The request id is derived.** `sha256(business_id ‖ idempotency_key)`, 32 bytes. The same value
will be the key on chain. This is what makes a retry after a timeout land on the same request instead
of creating a second one.

**5 · `settle-chain` calls `create_request`.** The transaction is built, signed with the creator's
authorisation, and submitted.

**6 · Nothing is believed yet.** The submit result is not the answer. `settle-chain` reads the request
back from the contract by id. If it reads `Pending`, the creation succeeded. If the submit timed out
and the readback finds nothing, the response is *unresolved* — not failure, and not an invitation to
retry blindly.

**7 · `202 Accepted` goes back**, with `status: "pending"` and a null transaction hash. The interface
shows "writing to chain" and polls. It does not show success, because at this instant nobody knows.

## Then somebody approves it

**1 · `POST /requests/:id/prepare`.** `settle-signature` loads the treasury account from Horizon —
the one unavoidable network call in a user-facing path, because a sequence number exists only on the
network and cannot be cached.

**2 · The sequence is reserved in the database.** A unique index on `treasury_address` for in-flight
settlements means a second concurrent `prepare` gets `409` immediately. This is the single most
important line in the schema: without it, two payments take the same sequence, both collect
signatures, and one dies after wasting two people's time.

**3 · The transaction is built** — payment to the vendor, `memo = request id`, timebounds one hour.
Its hash is computed and stored. That hash is what approvers sign, and it changes if the sequence
changes, which is why the sequence is held until the settlement resolves.

**4 · `POST /requests/:id/signatures` arrives, twice.** Each time, three checks: the signer belongs to
the policy version pinned on this settlement; the signature verifies against this exact hash; and the
primary key `(settlement_id, signer_address)` refuses a second signature from the same signer. Then
the weights are summed.

**5 · At weight 2, an event is published** and the HTTP request returns. Submission is a worker's
job. The approver does not wait for Horizon.

## The background loops

**The indexer** reads contract events by cursor, inserts each into `chain_events`, and advances the
cursor *after* the commit. Dying between the two replays events, which is harmless because
`UNIQUE (tx_hash, op_index)` makes reprocessing a no-op. That one constraint is also why two
instances can run during a rolling deploy without leader election.

**The submitter** picks up settlements that have reached their threshold. It checks
`submit_attempted_at` — not `status`, which stays `releasing` for the whole submit and therefore
does not prevent a redelivered message from submitting twice. Then it assembles the decorated
signatures and sends the envelope.

Four outcomes, and only one of them is simple:

| Outcome | What happens |
|---|---|
| Success | Record the hash, move to `releasing`, wait for the indexer to see it settle |
| `tx_bad_seq` | **Unresolved.** Not a failure, not a rebuild |
| Timeout | **Unresolved.** Handled identically to `tx_bad_seq` |
| Insufficient fee | Fee-bump the already-signed envelope. Approver signatures stay valid |

`tx_bad_seq` says the sequence has been consumed. It does not say by whom, and the most common cause
is that this envelope already reached the ledger. Building a new transaction and asking for
signatures again pays the vendor twice. So both unresolved cases route into `reconcile_then_decide`,
which looks the treasury's transactions up **by memo** — the memo is the request id, so the lookup
works even though the transaction hash was never received — and only rebuilds once the sequence is
demonstrably consumed by something else.

## Writing `paid`

Only after a readback. The indexer sees the payment settle, the submitter confirms it against the
ledger, and only then does `settle-chain` call `mark_paid` on the registry. A successful submit is
never sufficient. This is the rule the entire architecture is arranged around, and it is the third of
the three places in [../01-features.md](../01-features.md) where getting it wrong costs money.

## Logging in

There is no password anywhere. Someone connects a Stellar wallet and signs a nonce:

1. `POST /auth/wallet/challenge` stores a 32-byte nonce bound to the address, the network, this
   domain, and a 300-second expiry — then builds a SEP-10 challenge transaction from those values.
2. The wallet signs it with `signTransaction`. The transaction carries **sequence number 0**, so it
   can never be submitted: a sequence must be the source account's current one plus one, and zero is
   never that. What gets approved is unspendable by construction.
3. `POST /auth/wallet/verify` **takes** the nonce — reads and deletes in one statement — so it cannot
   be replayed. It rebuilds the expected challenge from the stored values, reads only the signatures
   out of the returned envelope, and checks them against that hash. The returned transaction itself
   is ignored, or a caller could sign something of their own choosing.

`signMessage` was tried first and does not work: wallets disagree about what they sign, and
Freighter routes it through a legacy "sign blob" path. `signTransaction` is the one operation every
wallet implements identically.

Every failure returns the same message after the same delay. Nothing reveals whether an address is
enrolled.

## What is not wired

- Off-ramp, custody, balances, reconciliation jobs. All out of scope; none of them have a crate.
- Notifications. The flow's S-13 sends the approver a link; in SOW 1 that link is copied by hand.
- Any dashboard aggregate. That is the thing the SOW names as out of scope.

## Running it

```bash
make dev         # postgres, backend, console
make migrate     # every migration, in order
make -C backend test
```

`make -C backend test` warns rather than fails if Postgres is unreachable, and `test-offline`
excludes `settle-db` for when you are on a train.

## The rules this code follows

- **`settle-domain` depends on nothing**, and no rule it owns is written twice.
- **No crate reads another crate's tables.** `settle-db` modules are the boundary.
- **Money is never a `number`.** `i128` stroops in Rust, `numeric(30,7)` in Postgres, `String` on the
  wire.
- **The submit result is not an answer.** Every conclusion comes from a readback.
- **`tx_bad_seq` and timeout take the same path**, and that path reads the chain first.
- **Correctness comes from indexes**, so two instances are always safe.
- **Secrets are never defaulted.** A default secret is a secret in production.
