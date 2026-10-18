# 12 · Testing

Module A9 and the integration work in Days 14–17. Sixteen mandatory contract cases, line coverage
≥ 90% for the registry crate, integration on real testnet without mocks, and one chaos test.

The interface being tested is [04-contract-interface.md](04-contract-interface.md); the features
these cases accept are [01-features.md](01-features.md).

## The standard

**Assert the exact error code.** `assert_eq!(err, Error::AlreadyPaid)`, never
`assert!(result.is_err())`. A test that only checks *that* a call failed passes when the contract
fails for the wrong reason, which is precisely the bug worth catching. This is why every entry point
returns `Result<_, Error>` — `try_*` on the generated client yields the code.

**`require_auth` failures panic; they do not return a code.** That is Soroban host behaviour. Those
cases assert a panic, and they say so in the test name so nobody later "fixes" them into code
assertions.

**Test names are full sentences.** `a_request_cannot_be_paid_twice`, not `test_mark_paid_2`. The
name is what a reviewer reads in the `cargo test` output, and it is also what the snapshot file is
named.

## The sixteen mandatory cases

### Lifecycle — 5

| # | Test | Asserts |
|---|---|---|
| 1 | `a_created_request_is_pending` | `create_request` → `get_request().status == Pending` |
| 2 | `the_settler_moves_a_pending_request_to_approved` | `create` → `approve` → `Approved` |
| 3 | `an_approved_request_becomes_paid_with_its_transaction_hash` | `approve` → `mark_paid` → `Paid`, and `paid_tx` is set |
| 4 | `the_creator_can_cancel_a_request_nobody_has_paid` | `create` → `cancel_request` → `Cancelled` |
| 5 | `an_expired_request_is_not_payable` | Advance the ledger past expiry → `is_payable() == false` |

### Abuse resistance — 7

| # | Test | Asserts |
|---|---|---|
| 6 | `the_same_request_id_cannot_be_created_twice` | `RequestExists` — the on-chain half of F-04 |
| 7 | `a_request_cannot_be_paid_twice` | Second `mark_paid` → `AlreadyPaid` |
| 8 | `only_the_settler_can_mark_a_request_paid` | `mark_paid` from another address → **panics** |
| 9 | `only_the_settler_can_approve_a_request` | `approve` from another address → **panics** |
| 10 | `a_request_cannot_be_approved_twice` | Second `approve` → `NotPending` |
| 11 | `only_the_creator_can_cancel_a_request` | `cancel_request` from another address → **panics** |
| 12 | `paying_a_different_amount_is_refused` | `mark_paid` with a mismatched amount → `InvalidAmount` |

Cases 8, 9 and 11 are the authority matrix. Without them, an authorisation regression is silent — the
contract still works for the legitimate caller, which is the only path anyone tests by hand.

### Boundaries — 4

| # | Test | Asserts |
|---|---|---|
| 13 | `a_zero_or_negative_amount_is_refused` | Both `0` and `-1` → `InvalidAmount` |
| 14 | `an_expiry_in_the_past_is_refused` | `expiry <= ledger.timestamp()` → `InvalidExpiry` |
| 15 | `a_payment_landing_on_the_expiry_second_is_refused` | `mark_paid` at exactly `expiry` → `Expired` |
| 16 | `an_entry_survives_across_ledgers_after_its_ttl_is_extended` | Advance many ledgers; both the request and every instance-storage function still work — module A8 |

Case 15 is the boundary that decides whether the comparison is `>=` or `>`. The specification says a
payment landing exactly on the expiry second is late, and the test is what keeps it that way through
a refactor.

Case 16 covers the failure mode that is invisible until it is catastrophic. If instance TTL is not
extended, every function in the contract stops working at once, some weeks after deployment, and no
test that runs inside a single ledger will ever notice.

## Error code coverage

Every code in [04-contract-interface.md](04-contract-interface.md) is reached by a test. The
sixteen mandatory cases cover most of them; the remainder get their own.

| Code | Covered by |
|---|---|
| `AlreadyInitialized` | Reserved. **No test — unreachable by construction** |
| `NotInitialized` | Reserved. **No test — unreachable by construction** |
| `SettlerNotSet` | Reserved. **No test — unreachable by construction** |
| `RequestExists` | 6 |
| `RequestNotFound` | `reading_a_request_that_never_existed_is_refused` |
| `InvalidAmount` | 12, 13 |
| `InvalidExpiry` | 14 |
| `NotPending` | 10, `a_cancelled_request_cannot_be_approved`, and `a_paid_request_cannot_be_cancelled` |
| `NotApproved` | `a_cancelled_request_cannot_be_paid` |
| `AlreadyPaid` | 7 |
| `Expired` | 15, and `approving_after_expiry_is_refused` |
| `Unauthorized` | Reserved. **No test — see below** |

Four codes are deliberately untested, and each has a reason.

`AlreadyInitialized`, `NotInitialized` and `SettlerNotSet` all describe a contract that exists
without being configured. Configuration happens in `__constructor`, which the host runs inside the
deployment transaction, so that state is unrepresentable — the codes became unreachable because the
design improved, not because anybody forgot them. `the_reserved_codes_keep_their_numbers` pins their
numbers so a future change cannot quietly reuse one.

`Unauthorized` is reserved for an authority check the host cannot express; today every one of them
routes through `require_auth`, which panics rather than returning a code.

Listing them here rather than quietly leaving a gap is the point — an untested code that nobody has
decided about is a hole; an untested code with a written reason is a decision.

## Coverage rule

Line coverage ≥ 90% for the registry crate, measured by `cargo llvm-cov`, excluding `src/test/` and
`mock-token/`. The report is written to `.deploy/coverage.txt` and committed, because it is part of
the Deliverable 1 evidence.

Coverage is a floor, not a goal. The sixteen cases above are mandatory whatever the percentage says.

## Backend tests

The four marked *(database)* need a real Postgres, because the property under test **is** the
database's: a partial unique index refusing a second in-flight settlement, a primary key refusing a
second signature. A mock would only assert that our mock behaves like our mock.

They skip with a printed reason when `DATABASE_URL` is unset, so `cargo test` still works on a
laptop with no database. CI always sets it.

```bash
DATABASE_URL=postgres://settle:settle@localhost:5432/settle cargo test -p settle-db --tests
```

| Test | Proves | Feature |
|---|---|---|
| `two_concurrent_prepares_leave_one_reserved_sequence` *(database)* | Two simultaneous `prepare` calls: one `200`, one `409`, exactly one `reserved_seq` row afterwards | F-09 |
| `a_signature_over_a_different_hash_is_refused` | ed25519 verification is against the transaction hash actually built | F-10 |
| `a_signer_outside_the_pinned_policy_is_refused` | The policy version pinned at authorisation is the one that governs | F-10 |
| `the_same_signer_cannot_sign_twice` *(database)* | The primary key blocks it, not application code | F-10 |
| `the_same_idempotency_key_returns_the_same_request_id` *(database)* | Calling `POST /requests` twice creates one request | F-04 |
| `a_different_body_under_the_same_key_is_refused` *(database)* | `IDEMPOTENCY_CONFLICT` rather than silently returning someone else's result | F-04 |
| `a_locking_threshold_configuration_is_refused` | The B3 guard, evaluated against the new values | F-07 |

Three more checks are **scripts rather than cargo tests**, because what they exercise is a database
and a live account, not a function:

| Check | Proves | Run by |
|---|---|---|
| `backend/scripts/verify_migrations.sh` | Every `up` then every `down`; no table, enum, domain or function of ours survives | `make -C backend verify-migrations` |
| `backend/scripts/verify_constraints.sh` | Every constraint refuses what it claims to refuse, F-09 included | `make -C backend verify-constraints` |
| `backend/scripts/verify_treasury.sh` | Three signers, thresholds 2, `masterWeight` 0 — evidence E2 | `make verify-treasury` |

## Integration — on real testnet, not mocked

Days 14–17. Mocks do not have sequence numbers, do not have trustlines, and do not time out.

| Scenario | Steps | Passes when |
|---|---|---|
| **Approve and settle** | `create_request` → `prepare` → signature 1 → signature 2 → threshold → `approve` → submit → readback → `mark_paid` | `get_request` returns `Paid`, and every milestone has a transaction hash |
| **Reject** | `create_request` → `prepare` → approver rejects | The request is `Cancelled`, the reserved sequence is released, **and the treasury's transaction list contains nothing for this settlement** |

The reject scenario's assertion is negative and is made against the **treasury account on chain**,
not against our database. Our database saying "cancelled" proves nothing about whether money moved,
and this is feature F-12, one of the three where money is lost.

### The concurrency test has been shown to fail

A test that has never been watched reject something is not a test. `two_concurrent_prepares_leave_one_reserved_sequence`
was run against a deliberately mis-keyed index — `(treasury_address, asset_code)` instead of
`treasury_address` — and it failed, as it must: two payments in different assets on the same
treasury were both accepted, and both would then have contended for one sequence number.

Repeat it before trusting any change to that index.

## Chaos test

Kill the service mid-submit. Restart it. Assert that no second transaction was submitted.

This is the test that proves the `submit_attempted_at` guard rather than the status guard. Under a
status guard the service restarts, finds the settlement in `releasing`, and submits again — paying
the vendor twice. Under the attempt guard it finds `submit_attempted_at` set with no `tx_hash`, and
routes into `reconcile_then_decide`, which reads the chain by memo before doing anything.

Run it before Day 18, not during the demo.

## Pre-demo checklist

Every item has failed a live demo somewhere.

- [ ] The treasury holds a USDC trustline.
- [ ] The vendor address holds a USDC trustline.
- [ ] Every test wallet holds a USDC trustline.
- [ ] The treasury holds enough XLM for fees **and** its minimum balance reserve.
- [ ] `masterWeight` reads back as 0 from Horizon.
- [ ] The deploy script has been run from scratch, timed, and finishes under ten minutes.
- [ ] The evidence page shows six ✓ and no ○.

## The rules this document imposes

- **Assert exact error codes.** `is_err()` is not an assertion.
- **The sixteen cases are mandatory** regardless of the coverage number.
- **Auth failures assert a panic**, and the test name says so.
- **Integration runs on testnet without mocks.**
- **The reject test asserts against the treasury on chain**, not against our database.
- **The chaos test runs before Day 18.**
- **An untested error code carries a written reason**, or it is a hole.
