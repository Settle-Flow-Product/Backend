# 14 · Roadmap

Twenty working days in five stages, following the Daily Breakdown committed to in the SOW. Each
stage has a measurable output, not a completion percentage.

The modules referenced are [02-modules.md](02-modules.md); the acceptance criteria are
[15-evidence.md](15-evidence.md).

## Budget

| Deliverable | Share | Amount |
|---|---|---|
| 1 · Registry contract | 35% | $1,750 |
| 2 · Signature service | 35% | $1,750 |
| 3 · Testnet deploy and demo | 30% | $1,500 |
| | | **$5,000** |

## Roles

| Role | Focus |
|---|---|
| Contract dev | Days 1–4, supporting Days 14–17 |
| Backend | Days 5–17, the centre of gravity |
| Frontend | Days 5–20, in parallel |
| Tech lead | Throughout — sequence locking, reconcile, the reject branch, documentation |
| Tester | Verification scripts, integration, chaos, evidence capture |

---

## Days 1–4 · Registry contract and unit tests

*Module group A*

| Task | Detail | Owner |
|---|---|---|
| Repository and CI | `fmt`, `clippy`, `test` on every push. **Pin the Soroban SDK in `Cargo.lock` on Day 1** | Tech lead |
| Storage and types | `DataKey`, `PaymentRequest`, `ReqStatus`, twelve error codes. Settle the TTL strategy | Contract dev |
| `create_request` + `approve` + `mark_paid` + `cancel_request` | The full `pending → approved → paid` lifecycle plus the cancelled branch | Contract dev |
| 16 unit tests | Lifecycle, abuse resistance, boundaries. Coverage ≥ 90% | Contract dev + Tester |

**Output:** the contract compiles; unit tests pass for creating a request, approving it, and moving
it through its statuses.

The no-custody grep gate is the first item on the Day 4 review checklist.

## Days 5–8 · Treasury multisig on testnet

*Module group B*

| Task | Detail | Owner |
|---|---|---|
| Treasury and trustline | Fund via friendbot, establish the testnet USDC trustline | Backend |
| Three signers, threshold 2, `masterWeight` 0 | The `masterWeight` step is the one most often forgotten — a reviewer checks it first | Backend |
| Lock-out guard | With a test for the configuration that would lock the account permanently | Backend |
| Verification script from Horizon | Prints the real configuration, to be submitted as evidence | Tester |
| In parallel: interface shell | Routing, design tokens, the state components | Frontend |

**Output:** a treasury deployed on testnet with a verified three-signer configuration and an enforced
threshold.

## Days 9–13 · Signature collection service

*Module groups C and D*

| Task | Detail | Owner |
|---|---|---|
| `build_unsigned()` | Payment transaction with `memo = request id` and one-hour timebounds | Backend |
| **Sequence locking in the database** | Two concurrent requests: the second is refused cleanly | Backend + Tech lead |
| `submit_signature()` | ed25519 over the exact transaction hash; a duplicate blocked by the primary key | Backend |
| Weight accumulation and threshold detection | Return `collected / required` to the interface | Backend |
| Minimal chain synchronisation | Cursor ingest, deduplication on `(tx_hash, op_index)` | Tech lead |
| Create and list screens | E2 and E3 | Frontend |

**Output:** the service receives and stores approver signatures, and detects precisely when the
threshold is met.

**The sequence lock lands here, on Day 9.** Not Day 14. It is the one piece of work whose absence is
only discovered once it has already wasted two approvers' time, and by then the demo is a week away.

## Days 14–17 · Submit, update the registry, the reject branch

| Task | Detail | Owner |
|---|---|---|
| Assemble signatures and submit | `tx_bad_seq` and timeout handled separately from real failures — both route through reconcile | Tech lead |
| Wire the registry: `approve` and `mark_paid` | Called only after a confirming readback, never after a submit | Backend + Contract dev |
| The reject branch | A test asserting that no transaction left the treasury | Backend + Tester |
| Integration on real testnet | Approve-then-settle, and reject. No mocks | Tester |
| Chaos test | Kill the service mid-submit; assert no double submit after restart | Tester + Tech lead |
| Approver signing screen | E4 | Frontend |

**Output:** the approve-and-settle integration test passes in full; the reject branch sets the correct
status and blocks submission.

## Days 18–20 · Round-trip demo and documentation

*Module group G*

| Task | Detail | Owner |
|---|---|---|
| Full flow on testnet | Create a request → 2 of 3 signatures → pay the vendor → registry marks paid | Whole team |
| Evidence page and hash capture | Six milestones, every Stellar Expert link present | Frontend + Tester |
| README and deploy guide | An outsider redeploys from scratch unaided | Tech lead |
| Demo video, 3–5 minutes | Reviewable without deep technical knowledge | Tech lead |
| Public repo and contract address | Clear secrets, add a LICENSE, pin the contract id in the README | Tech lead |

**Output:** a request that collected 2 of 3 signatures, paid a vendor address, and was marked paid by
the registry — with transaction hashes and a deploy guide committed.

---

## Critical path

```
  Day 1   4        8            13              17         20
   |------|--------|-------------|---------------|----------|
   A: contract
          B: treasury
                   C: signatures ....................
                   D: chain sync
          E: interface ..............................
                                                   G: demo
```

Three dependencies are hard, and each is a place where slipping one thing slips everything after it:

| This | Cannot start before | Because |
|---|---|---|
| C1 `build_unsigned` | B2 complete | There is no treasury to draw a sequence number from |
| C5 submit | C2 complete | Submitting without a sequence lock is the failure the lock exists to prevent |
| G2 demo | C6 and C8 complete | An unhandled `tx_bad_seq` during a demo pays twice in front of the reviewer |

E runs in parallel from Day 5 against the agreed API contract
([09-api-contract.md](09-api-contract.md)), which is why that document is written before either side
exists.

## The rules this document imposes

- **The SDK is pinned on Day 1** and not touched again during the sprint.
- **The sequence lock lands on Day 9.**
- **The chaos test runs before Day 18**, not during the demo.
- **The evidence page is built during Days 18–20**, and hashes are captured as they happen — a hash
  not captured at the moment is a hash gone.
- **A stage is finished when its output is observable**, not when its tasks are ticked.
