# 15 · Evidence

What gets submitted, and the condition under which each deliverable is considered complete.

Finishing every module and arriving without transaction hashes still scores as missing evidence.
This is the acceptance risk concentrated in module group G, and it is why the evidence page is a
screen ([10-frontend-ux.md](10-frontend-ux.md)) rather than a spreadsheet somebody fills in
afterwards.

## Evidence per SOW section 6.1

| Deliverable | Kind of evidence | Specifically |
|---|---|---|
| **1** | Public repository + contract address | The Payment Request Registry source and the contract id deployed on Stellar testnet, with `cargo test` output |
| **2** | Repository + transaction hashes | The signature collection service source, and a submitted transaction demonstrating threshold-based approval |
| **3** | Transaction hashes on Stellar Expert | The complete round trip from request to payout, with verifiable on-chain links |

## The seven milestones

Filled in during the demo, not after it. The live copy lives in
[`../.deploy/evidence.md`](../.deploy/evidence.md) and is committed on purpose — a reviewer opening
the repository has to find these without running anything.

| # | Event | Verified by | Hash / id |
|---|---|---|---|
| **E1** | Deploy the registry contract | Stellar Expert · contract id | `C…` |
| **E2** | Treasury: 3 signers, threshold 2, master 0 | Horizon `/accounts`, compared against the B5 script output | `G…` |
| **E3** | `create_request` on the registry | Stellar Expert · transaction hash | |
| **E4** | `approve` after 2 of 3 signatures | Stellar Expert · transaction hash | |
| **E5** | Payment to the vendor address | Stellar Expert · transaction hash | |
| **E6** | `mark_paid`, and `get_request` returning `Paid` | Stellar Expert + an RPC read | |
| **E7** | Reject branch: no transaction left the treasury | A negative assertion — log and test, with an explanation for the reviewer | — |

**E7 is a negative claim and needs handling as one.** "No transaction was submitted" cannot be shown
with a link, so the evidence is the test itself plus a query of the treasury's transaction list over
the relevant window, presented with a sentence explaining what the reviewer is looking at. A reviewer
who cannot tell what an absence proves will read it as a gap.

**E2 is not a transaction hash** but an account state. The B5 verification script's output is what is
submitted, because it asserts the configuration rather than displaying it — including
`masterWeight = 0`, which is the item a reviewer checks first.

## Submission package

| Item | Where |
|---|---|
| Public repository, secrets cleared, LICENSE added | Git |
| Registry contract id, testnet | Pinned in the root `README.md` |
| `cargo test` output | `.deploy/coverage.txt` and CI |
| The seven milestones | `.deploy/evidence.md` and `/evidence/:reference` |
| Deploy guide | [13-deployment.md](13-deployment.md) and the root `README.md` |
| Demo video, 3–5 minutes | Linked from the README |

## Producing it

`scripts/demo.sh` runs every case end to end on testnet and writes `.deploy/evidence.md` **as it
goes**, recording each step the moment it happens. A run that dies half way still leaves behind
everything it did prove.

```bash
make deploy KEY=<your stellar key name>   # E1
make treasury                             # E2
make verify-treasury                      # asserts E2 and prints it
make demo                                 # E3 → E7, and the evidence file
make demo-plan                            # list the steps without touching the network
```

Three cases, because a demo that only shows the happy path proves the system works when nothing
goes wrong — which is not the interesting half.

| Case | Covers | Milestones |
|---|---|---|
| **A · happy path** | create → approve → pay the vendor → `mark_paid` after readback → `Paid` | E3, E4, E5, E6 |
| **B · reject branch** | create → cancel, and the treasury's transaction count **counted on chain** before and after | E7 |
| **C · abuse resistance** | duplicate id, zero amount, past expiry, and a second `mark_paid` — each must be refused | — |

Every step is recorded with what was expected and what happened. A step that did not behave as
specified is marked **NOT as specified** and the script exits non-zero, so a broken run cannot be
submitted as a green one by mistake.

`DRY_RUN=1` lists the plan and asserts nothing — and says so in the file, because a green evidence
file that never touched the network is worse than no file at all.

## Definition of Done

### Required

- An automated test for the happy path and at least one error branch.
- Every state change writes an `audit_events` row.
- Every chain-touching endpoint returns `202`, not `200`.
- Logs carry a `trace_id` that connects the interface to the chain event.
- Every migration runs both up and down.
- The deploy script re-runs from scratch in under ten minutes.

### Reject the merge if

- There is a `f32` or `f64` anywhere in a money path.
- A server clock decides an expiry.
- Any path shows "success" before a readback.
- Any retry happens automatically without a readback first.
- The registry contract contains a token transfer call.
- A secret or seed phrase appears in the repository or in a log line.

## Acceptance

SOW 1 is complete when a reviewer can, without assistance:

1. Open the repository and find the contract id.
2. Open Stellar Expert and see the registry contract deployed.
3. Read the treasury account from Horizon and confirm three signers, threshold 2, master weight 0.
4. Follow E3 → E4 → E5 → E6 and watch one request go from created to paid.
5. Read the reject-branch explanation and understand why no transaction exists for it.
6. Clone the repository and redeploy the whole thing from scratch.

Item 6 is the one most often left undone, and it is the one that turns a demo into a deliverable.

## The rules this document imposes

- **Hashes are captured as they happen.** A hash not recorded at the moment is a hash gone.
- **`.deploy/evidence.md` is committed**, not generated on demand.
- **E7 ships with an explanation**, because an absence proves nothing on its own.
- **The deploy guide is tested by someone who did not write it.**
- **Secrets are cleared before the repository is made public** — checked, not assumed.
