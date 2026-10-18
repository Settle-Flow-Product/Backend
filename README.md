# SettleFlow — SOW 1

Vendor payments with an internal approval gate, on Stellar.

A business records a payment request against a Soroban registry contract — vendor address, amount,
asset, reference — and tracks it through `pending → approved → paid`. A backend builds an unsigned
Stellar payment from an approved request, collects approver signatures, and submits once the
collected weight meets the treasury account's threshold.

**The contract never holds a single unit of anything.** It records and verifies. That is what lets
it reach testnet in twenty working days without a security audit, and it is checked mechanically on
every push rather than remembered.

## The problem

Companies holding stablecoin treasury on Stellar have no purpose-built way to pay vendors with
internal approval control. Zebec covers payroll, COINQVEST covers merchant checkout — neither
covers accounts payable. Teams either send payments straight from a wallet with no approval trail,
or fall back on a bank-focused tool that cannot touch stablecoins at all.

Stellar accounts have supported weighted-signer multi-signature for years. Nothing wires that
primitive into a payment-request workflow. SOW 1 builds exactly that layer.

## Evidence

| Deliverable | Proof | Where |
|---|---|---|
| 1 · Registry contract, 35% | Public repository, testnet contract id, `cargo test` output | [`.deploy/evidence.md`](.deploy/evidence.md) · E1 |
| 2 · Signature service, 35% | Source, and a submitted transaction showing threshold-based approval | E2 → E5 |
| 3 · Testnet deploy and demo, 30% | The round trip from request to payout, verifiable on chain | E6, E7 |

**Deployed on Stellar testnet:**

| | |
|---|---|
| Registry contract | [`CAYZWEVGGANJO3EZTGIIX6VJEOYXQ2EZNTB5YYXTXKQTLNXE74RVGVBQ`](https://stellar.expert/explorer/testnet/contract/CAYZWEVGGANJO3EZTGIIX6VJEOYXQ2EZNTB5YYXTXKQTLNXE74RVGVBQ) |
| Treasury | [`GBBA7JI4Y3P6KWXK2OZ66SJXOUJUFZ6TKZI3Q5SGFCPSGNHPUCQ2HHUN`](https://stellar.expert/explorer/testnet/account/GBBA7JI4Y3P6KWXK2OZ66SJXOUJUFZ6TKZI3Q5SGFCPSGNHPUCQ2HHUN) — 3 signers, thresholds 2, `masterWeight` 0 |

The full round trip has been run: 26 steps, every one as specified, with every transaction hash in
[`.deploy/evidence.md`](.deploy/evidence.md) and verifiable on Stellar Expert.

## Layout

| Path | Contents | State |
|---|---|---|
| [`DEMO.md`](DEMO.md) | The reviewer's walkthrough — run it, click it, check it | — |
| [`docs/`](docs/) | The specification. English; [`docs/translate/vi/`](docs/translate/vi/) mirrors it | Written |
| [`smart-contract/`](smart-contract/) | `payment_request_registry` — Soroban, Rust, no custody | Built and tested |
| [`backend/`](backend/) | Seven Rust crates and one binary | Built and tested |
| [`frontend/website/`](frontend/website/) | The internal tool. Four screens, wallet sign-in | Built and tested |
| [`frontend/admin/`](frontend/admin/) | The operations console. One route for every table in the catalogue | Built and tested |
| [`resources/`](resources/) | The source deck, and flow diagrams drawn from the code | — |
| [`.deploy/evidence.md`](.deploy/evidence.md) | The transaction hashes a reviewer opens | Awaiting the demo |

## Running it

**Reviewing this?** [`DEMO.md`](DEMO.md) is the step-by-step walkthrough — what to run, what to
click, and for each claim, the observation that would have looked different if it were false. It is
tiered by how much time you have, starting at two minutes with nothing installed.

Two ways to run it, and the difference is where the code we wrote runs:

```bash
make dev     # behind nginx, and every service also on its own port
make prod    # behind nginx, everything else on loopback — as it runs on a VM
```

Both are three steps: deploy the registry contract if it is not deployed, configure the treasury if
it is not configured, then bring the stack up. The first two are
[`scripts/chain_up.sh`](scripts/chain_up.sh), and it skips whatever already **answers on chain** —
so `make deploy` and `make treasury` are no longer steps to remember, and a testnet reset is noticed
and rebuilt rather than reported three stages later as `REGISTRY is not set`.

`make prod` is what a fresh VM needs — `make prod PUBLIC_HOST=pay.example.com` when there is a
domain. `make stop` takes it down, `make ps` says what is running.

The third command is the contract's own demo, which needs no stack at all — only the deployed
registry and the treasury:

```bash
make demo         # the testnet round trip, writing .deploy/evidence.md as it goes
make demo-plan    # the same 16 steps listed, touching no network
make evidence     # print the table a reviewer opens
```

`make dev` leaves a stack you can click through — it enrols the signer roster and seeds
`admin@settlement.local` / `123456` as its last two steps, because a stack that is up but has
neither is a stack with no way in and an approver screen that errors. Then:

```bash
make verify-api   # 38 API cases end to end against the running stack
```

| Service | URL |
|---|---|
| backend | http://localhost:8088 (`/health`, `/ready`) |
| website | http://localhost:5173 — create and approve payments |
| admin | http://localhost:5174/admin — operations; see [docs/11-admin.md](docs/11-admin.md) |
| adminer | http://localhost:8081 (`docker compose --profile sql up -d adminer`) |

The console lives under `/admin`, in development as well as behind nginx. Two Next.js apps at one
origin cannot both own `/_next`, so it is built with `basePath: '/admin'` — and an app reachable at
`/` locally but `/admin` in production is an app whose links are only ever tested in one of them.

nginx is in front of both `make dev` and `make prod`, so there is one origin rather than four ports:

| Path | Goes to |
|---|---|
| `/` | the website |
| `/admin` | the operations console |
| `/api/…` | the backend, with the `/api` prefix stripped |
| `/healthz` | nginx itself — what `dev` and `prod` poll |

Same file and same routes in both, deliberately: an origin that only exists in production is an
origin whose links are only ever tested there. What `prod` adds is moving every other published
port to loopback, so the front door becomes the only way in; `dev` publishes them as well, which is
how the demo and verification scripts reach the backend directly.

The routing is [`deploy/nginx/nginx.conf`](deploy/nginx/nginx.conf), and `make nginx-check`
validates it without starting anything. `HTTP_PORT=8080 make dev` when 80 is taken.

Each host port can be overridden (`BACKEND_PORT`, `WEBSITE_PORT`, `ADMIN_PORT`, `POSTGRES_PORT`,
`ADMINER_PORT`) so a second stack on the same machine does not fail to start over a port number.

The admin console has no sign-up page. Make the first account deliberately:

```bash
make admin-user ARGS="create you@example.com superadmin"
```

`make dev` seeds `admin@settlement.local` / `123456` for you; `make demo-admin` recreates it after a
`make fresh`. That password is weak on purpose and printed in the Makefile for anyone to read: the
account is rebuilt several times an hour and a strong one would be a strong one for nobody. It still
requires `ALLOW_WEAK_PASSWORD=1`, which nothing in the deployment sets — and `make prod` does not
seed it at all.

## Deploying to testnet

`make dev` and `make prod` both do this for you, and skip whatever already answers. By hand, or to
rebuild after a testnet reset:

```bash
make chain                                # both of the two below, only where they are needed
make deploy KEY=<your stellar key name>   # deploys AND initialises in one transaction
make treasury                             # 3 signers, thresholds 2, masterWeight 0, USDC trustline
make verify-treasury                      # reads it back from Horizon and ASSERTS
make policy                               # enrol that roster as the governance policy
```

`make policy` is not optional and it is easy to forget. The treasury account knows who its signers
are; the backend does not, until the roster is enrolled as a policy. Skip it and `prepare` has no
policy version to pin, so the approver screen shows an INTERNAL error where the signature progress
should be. It reads the roster from Horizon rather than taking it as an argument, so the account and
the database cannot drift into disagreeing about who may approve a payment.

`make treasury` also establishes the USDC trustline and issues the treasury a working balance from an
issuer it generates, so a USDC demo needs no browser faucet. Set `USDC_ISSUER` first to use a real
issuer's asset instead. Both targets sync what they deployed into `.env`, so the console is never
pointed at a contract that is no longer there.

The whole sequence re-runs from nothing in under ten minutes, with no manual step. Testnet resets,
and when it does the deployed contract goes with it — so the rebuild is a script rather than a
search. Full runbook: [`docs/13-deployment.md`](docs/13-deployment.md).

## Testing every API path

```bash
make dev          # the stack, the signer roster, and a console account
make verify-api   # 38 cases end to end against the running stack
```

`make verify` covers what can be checked without a network: the contract, the schema, the linters,
both front ends. `make verify-api` covers what cannot — validation, idempotency, both assets reaching
the registry, the sequence lock, signature collection and its refusals, settlement, the six evidence
milestones, and the reject branch.

The split matters. Every defect that reached a person during this project — an asset with no
contract, a request signed before it existed on chain, a settlement that held the treasury for ever —
was reachable from the API and invisible to a unit test.

## Running the demo

```bash
make demo                     # every case on testnet, writing .deploy/evidence.md as it goes
make demo-plan                # list the steps without touching the network
DEMO_ASSET=USDC make demo     # the same round trip paying an issued asset instead of native XLM
```

Three cases, because a demo that only shows the happy path proves the system works when nothing goes
wrong — which is not the interesting half.

| Case | Covers |
|---|---|
| **A · happy path** | create → approve → pay the vendor → `mark_paid` after readback → `Paid` |
| **B · reject branch** | create → cancel, with the treasury's transaction count **counted on chain** before and after |
| **C · abuse resistance** | duplicate id, zero amount, past expiry, second `mark_paid` — each must be refused |

Each case establishes a **positive control** first: if the registry does not answer, the case
abandons itself rather than letting every "must be refused" check pass against a broken network. A
step that misbehaves is marked *NOT as specified* and the script exits non-zero, so a broken run
cannot be submitted as a green one.

## Verifying

```bash
make verify      # both Rust workspaces, both frontends, the schema, and the docs — one command
make test        # every suite: contract, backend, website, admin
make lint        # clippy -D warnings, and tsc
```

`make verify` starts its own throwaway Postgres and checks 21 things. The ones worth naming:

- The registry contains **no token call** — and the gate's own self-test proves it can detect one.
- Every migration **reverses**, leaving no table, enum, domain or function behind.
- Every constraint **refuses** what it claims to refuse, including the one that stops two payments
  contending for one treasury sequence number.
- The docs and the code still agree: error numbers, routed endpoints, cited tests, translations.
- The wasm stays within its size ceiling, and no float appears anywhere in the source.

## What this does not do

Out of scope, quoted from the statement of work: a full dashboard, vendor-facing views, a
reconciliation indexer, accounting export, a security audit, and mainnet deployment. Four more were
added to protect the schedule: no escrow holding funds, no fiat off-ramp, no internationalisation,
no mobile application.

The contract holds no money. Custody is a different design, a different security posture, and a
different conversation about liability — see [`docs/16-risks.md`](docs/16-risks.md).

## Where to start

| Question | Document |
|---|---|
| What is being built, and what is deliberately not? | [`docs/00-overview.md`](docs/00-overview.md) |
| What must run, and how is it accepted? | [`docs/01-features.md`](docs/01-features.md) |
| What are the actual work items? | [`docs/02-modules.md`](docs/02-modules.md) |
| What does the contract do, exactly? | [`docs/04-contract-interface.md`](docs/04-contract-interface.md) |
| Why is the sequence number such a problem? | [`docs/06-signature-service.md`](docs/06-signature-service.md) |
| How does the backend actually run? | [`docs/backend/README.md`](docs/backend/README.md) |
| What could go wrong in twenty days? | [`docs/16-risks.md`](docs/16-risks.md) |

## The rules this repository follows

- **The registry never moves a token.** A CI gate greps for it, and the gate has a self-test.
- **The chain is the source of truth.** `paid` is written only after a ledger readback; a
  successful submit is not a confirmed payment.
- **`tx_bad_seq` and a timeout are the same situation.** Both read the chain first. Rebuilding a
  transaction instead is how the same vendor gets paid twice.
- **Money is never a float.** `numeric(30,7)` in Postgres, `i128` stroops in Rust, `String` on the
  wire — JSON numbers are IEEE doubles.
- **Ledger time decides expiry.** `env.ledger().timestamp()`, never a server clock.
- **Correctness comes from constraints**, so two service instances are always safe.
- **Secrets are never defaulted and never committed.** A CI job checks.
