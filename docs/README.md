# SettleFlow — SOW 1 build documentation

SettleFlow SOW 1 is a vendor payout workflow with an internal approval gate, on Stellar. A business
records a payment request against a Soroban registry contract — vendor address, amount, asset,
reference — and tracks it through `pending → approved → paid`. A backend builds an unsigned Stellar
payment transaction from an approved request, collects approver signatures, and submits once the
collected weight meets the treasury account's threshold.

**The contract never holds a single unit of anything.** It records and verifies. That is what lets
it reach testnet in twenty working days without a security audit.

Everything here derives from one file:
[`../resources/specs/settleflow-sow1-deck.html`](../resources/specs/settleflow-sow1-deck.html) —
the SettleFlow SOW 1 Delivery Deck, in Vietnamese. Where a document cites *slide N*, that is the
deck in display order.

## Scope of this documentation

This covers **SOW 1 only** — the twenty working days under contract, three deliverables, $5,000.
It does not describe the full SettleFlow product; the escrow vault, off-ramp routing, dashboards and
vendor portals are all explicitly out of scope and appear here only in the "not this" column.

## Source of truth

| Question | Answered by | Not by |
|---|---|---|
| What status is this request in? | `get_request` on the registry contract | The database, which mirrors it |
| Is this approval sufficient? | The treasury account's thresholds on chain | Our `policy_signers` table, which records intent |
| Did the payment settle? | A readback from the ledger | The result of `submit` — those are different facts |
| Everything else | PostgreSQL | — |

## Two document sets

| Set | Language | Purpose | Precedence |
|---|---|---|---|
| [`docs/`](.) | English (en-GB) | The specification the code is checked against | **Authoritative** |
| [`docs/translate/vi/`](translate/vi/) | Vietnamese | A line-parallel translation for the team | Loses to the English on any conflict |
| [`../resources/flows/`](../resources/flows/) | English + mermaid | Diagrams drawn *from the shipped code* | Loses to `src/` on any conflict |

## Reading order

| # | Document | Covers |
|---|---|---|
| 00 | [00-overview.md](00-overview.md) | The problem, the three deliverables, and the hard scope boundary |
| 01 | [01-features.md](01-features.md) | F-01…F-13, each with an observable acceptance test |
| 02 | [02-modules.md](02-modules.md) | The 42 modules in seven groups — the issue tracker, in table form |
| 03 | [03-flow.md](03-flow.md) | S-01…S-30, and the payment request state machine |
| 04 | [04-contract-interface.md](04-contract-interface.md) | `payment_request_registry`: types, gate order, errors, events, TTL |
| 05 | [05-treasury-multisig.md](05-treasury-multisig.md) | Treasury setup, `masterWeight = 0`, and the lock-out guard |
| 06 | [06-signature-service.md](06-signature-service.md) | Build unsigned, reserve the sequence, collect, submit, reconcile |
| 07 | [07-chain-sync.md](07-chain-sync.md) | Cursor ingest, deduplication, and readback before `paid` |
| 08 | [08-data-model.md](08-data-model.md) | The tables that actually earn their place, and the indexes that matter |
| 09 | [09-api-contract.md](09-api-contract.md) | Nine endpoints, the error envelope, and why chain writes return 202 |
| 10 | [10-frontend-ux.md](10-frontend-ux.md) | The website: four screens, design tokens, component states |
| 11 | [11-admin.md](11-admin.md) | The admin console: one route per catalogue, and what it refuses to write |
| 12 | [12-testing.md](12-testing.md) | The 16 contract cases, integration on real testnet, the chaos test |
| 13 | [13-deployment.md](13-deployment.md) | Deploy, configure, verify, and do it again in under ten minutes |
| 14 | [14-roadmap.md](14-roadmap.md) | Twenty days in five stages, with measurable output per stage |
| 15 | [15-evidence.md](15-evidence.md) | The seven hashes, and the Definition of Done |
| 16 | [16-risks.md](16-risks.md) | Six risks with mitigations, and three questions to settle before Day 1 |
| — | [backend/README.md](backend/README.md) | How the backend runs, in the order the code executes |

## Shape of SOW 1 in one page

```
  Deliverable 1   35% · $1,750    payment_request_registry            Soroban · no custody
                                  create_request     creator signs
                                  approve            settler signs · pending -> approved
                                  mark_paid          settler signs · approved -> paid
                                  cancel_request     creator signs · the reject branch
                                  get_request        read-only, via RPC simulation
                                  is_payable

  Deliverable 2   35% · $1,750    treasury            G-account · 3 signers weight 1
                                                      low/med/high threshold 2 · masterWeight 0
                                  signature service   build_unsigned  reserves ONE treasury sequence
                                                      submit_signature  ed25519 over the exact hash
                                                      threshold met -> assemble -> submit
                                                      reconcile_then_decide  the only rebuild path

  Deliverable 3   30% · $1,500    testnet round trip  request -> 2 of 3 signatures -> vendor paid
                                                      -> registry marks paid · every hash recorded

  Assets          USDC (Stellar Asset Contract) and native XLM · 7 decimals · i128 stroops
  Money           numeric(30,7) in Postgres · i128 in Rust · String on the wire · float nowhere
  Time            env.ledger().timestamp() decides expiry. The server clock decides nothing.
  Success         only after readback. A submitted transaction is not a paid request.
  Scale           13 features · 42 modules · 30 flow steps · 4 screens + a console · 7 evidence milestones
```

## Decisions that override the deck

| Deck says | We build | Why |
|---|---|---|
| The commercial product will collect money from customers; the SOW describes paying vendors (slide 2) | Vendor payout only | The deck settles this itself: the registry and the approval mechanism are the same in both directions, so the delivered code stays correct either way. Evidence is checked against the signed text. |
| `app.settleflow.internal` with four screens | Two Next.js applications: `frontend/website` and `frontend/admin` | The split follows the trust boundary, not taste. The website is wallet-facing and holds no credential; the console holds the database password and never leaves the operator's network. One codebase would mean one bundle that does both. |
| A backend, language unspecified | A Rust Cargo workspace under `backend/crates/` | One toolchain with the contract, one lint gate, one test command. |
| `PaymentRequest` carries nine fields | The same nine, and no more | Every extra field on chain is storage rent and a migration nobody planned. |

## Gaps in the deck this specification fills

| Gap | Filled in |
|---|---|
| Twelve error codes are named but never numbered or ordered | [04-contract-interface.md](04-contract-interface.md) |
| Gate evaluation order is unstated, but tests assert exact codes | [04-contract-interface.md](04-contract-interface.md), [12-testing.md](12-testing.md) |
| The lock-out guard is prose, never a checkable predicate | [05-treasury-multisig.md](05-treasury-multisig.md) |
| Six tables are named but not defined, indexed or ordered into migrations | [08-data-model.md](08-data-model.md) |
| The error response shape is never specified | [09-api-contract.md](09-api-contract.md) |
| "16 unit tests" is a count, not a list | [12-testing.md](12-testing.md) |

## Current repository state

- The specification is written; the code is scaffolded.
- Nothing has been deployed to testnet. [15-evidence.md](15-evidence.md) holds the empty
  transaction-hash table the demo fills in.
- The Vietnamese translation under [`translate/vi/`](translate/vi/) tracks the English set.
