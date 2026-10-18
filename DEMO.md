# Demo — a reviewer's walkthrough

Everything below is written for somebody who did not build this and has no reason to trust it.

Three deliverables have to be shown: a registry contract, a signature service, and a testnet round
trip. Each section below ends with **what to check** — the observation that would fail if the claim
were false. Reading the output is not the point; knowing what would have looked different is.

| I have | Do | Proves |
|---|---|---|
| **2 minutes** | [Read the evidence](#0--two-minutes-nothing-installed) — nothing to install | The round trip already happened, on a chain you can query |
| **10 minutes** | [Run the stack](#1--ten-minutes-the-stack) and [exercise every API path](#3--every-api-path) | The service does what the evidence claims, on your machine |
| **30 minutes** | [The browser walkthrough](#4--the-browser-walkthrough) | A person can drive it, not just a script |
| **an hour** | [Reproduce the chain round trip](#5--the-on-chain-round-trip) from nothing | The evidence is reproducible, not a one-off |

---

## 0 · Two minutes, nothing installed

Open [`.deploy/evidence.md`](.deploy/evidence.md). Twenty-six steps, each with a transaction hash
linking to Stellar Expert.

The three that carry the weight:

| Claim | Where | Why it is not self-reported |
|---|---|---|
| The treasury paid the vendor on **2 of 3** signatures | Case A, step 10 | Step 8 submits the *same envelope* with **one** signature and the network answers `tx_bad_auth`. The threshold is enforced by Stellar, not by us |
| A rejected payment moves **no money** | Case B, step 21 | The treasury's transaction count is read from Horizon before and after — `37 → 37`. A negative claim proved by counting on the chain, not by querying our own database |
| `paid` is written only after the ledger confirms | Case A, steps 11–13 | Step 11 reads the transaction *result*; `mark_paid` is step 13. A successful submit is not a confirmed payment |

Every hash is on public testnet. Open one and check the source account, the destination and the
amount yourself.

**What to check:** step 8. A demo that only shows the happy path proves the system works when
nothing goes wrong, which is not the interesting half. If the one-signature submission had
*succeeded*, the multi-signature gate would be decorative.

---

## 1 · Ten minutes, the stack

### Before you start

| Need | Check | If missing |
|---|---|---|
| Docker, running | `docker info` | Start Docker Desktop |
| Docker Compose v2 | `docker compose version` | Ships with Docker Desktop |
| `jq` | `jq --version` | `brew install jq` |
| Python 3 | `python3 --version` | Ships with macOS |
| Stellar CLI | `stellar --version` | [install](https://developers.stellar.org/docs/tools/cli) — needed for §3 and §5, not for §1 |

You do **not** need Rust, Node, or a Stellar account to run the stack. The contract and the backend
are already built and deployed to testnet; the addresses are in [`.env.example`](.env.example) and
[`.deploy/env`](.deploy/env).

### One command

```bash
make dev
```

[`scripts/dev.sh`](scripts/dev.sh) is three steps and stops at the first that is not as specified:

| # | Stage | What it is for |
|---|---|---|
| 1-2 | registry and treasury | [`chain_up.sh`](scripts/chain_up.sh) deploys each only if it is missing, and reuses whatever still **answers** on chain — so a testnet reset is rebuilt rather than reported later as `REGISTRY is not set` |
| 3 | docker compose up | postgres, backend, website, admin, nginx — then **polls** `/ready` and `/healthz` rather than sleeping, printing the log of whichever did not answer |
| 4 | policy | Enrols the treasury's signer roster, read from the chain. Without it `prepare` has no policy version to pin and the approver screen shows INTERNAL |
| 5 | console account | Seeds `admin@settlement.local` / `123456`. The console has no sign-up page, so without this there is no way in |

Every stage is idempotent: the second run reuses the deployment, says *the active policy already
matches the chain*, and leaves the account alone.

Everything is behind one origin, the same routing `make prod` serves:

| Path | Goes to |
|---|---|
| `/` | the website |
| `/admin` | the operations console |
| `/api/…` | the backend, with the `/api` prefix stripped |

`make dev` also publishes each service on its own port, which `make prod` does not.

`make prod` runs stage 4 as well, but **not** stage 5: the demo password is weak and published,
which is right for a laptop and wrong for a server. There, `make admin-user` makes the first account
deliberately.

`make dev` ends by printing:

```
  website   http://localhost:5173   create and approve payments
  admin     http://localhost:5174/admin   admin@settlement.local / 123456
  backend   http://localhost:8088   (/health · /ready)
```

Stage 4 polls rather than sleeps, and stage 5 is why. The backend runs its migrations at start-up;
enrolling a policy against a database that has no tables yet fails, and on a cold machine that is
what a fixed `sleep 5` produces. `/ready` is the backend's own answer to *have the migrations
finished*, so the script asks it instead of guessing.

Stage 5 is not optional and is easy to forget. The treasury account knows who its signers are; the
backend does not, until the roster is enrolled. Skip it and the approver screen shows an INTERNAL
error where the signature progress should be. It reads the roster **from Horizon**, not from an
argument, so the account and the database cannot drift into disagreeing about who may approve a
payment.

**What to check:** run `make dev` a second time. Stage 4 should say *the active policy
(version 1) already matches the chain — nothing to do*. An enrolment that wrote a new version every
run would mean the roster comparison is not actually comparing anything.

### The password is 123456, on purpose

It is printed in the [Makefile](Makefile), in this file, and in the repository, for anyone to read.
`make fresh` takes the database volume and the account with it, so it is rebuilt several times an
hour — a strong password here would be a strong password for nobody. It still requires
`ALLOW_WEAK_PASSWORD=1`, which nothing in the deployment sets, and the console has no sign-up page
and no bootstrap-by-environment-variable: both are ways for an account to exist that nobody decided
to create.

---

## 2 · Is it actually up

```bash
curl -s localhost:8088/health   # {"status":"ok"}
curl -s localhost:8088/ready    # {"status":"ready"}
```

The two are different questions. `/health` says the process is alive; `/ready` says the migrations
finished and the database answers. A load balancer that conflates them sends traffic to a backend
that cannot serve it.

---

## 3 · Every API path

```bash
make verify-api
```

Thirty-eight cases against the running stack — validation, idempotency, both assets, the sequence
lock, signature collection and its refusals, settlement, the six evidence milestones, and the reject
branch. Expect:

```
══ result
  38 passed   0 failed   0 skipped
```

The ones worth reading as they scroll past:

| Case | Why it is in here |
|---|---|
| `a second payment from the same treasury is refused` | Two payments contending for one treasury sequence number is the defect this system exists to not have. It is refused by a **database constraint**, so two backend instances are equally safe |
| `the same key twice yields one payment` | Idempotency. A retried request is not a second payment |
| `the same key with a different body is refused` | The other half of idempotency: reusing a key for different content is a bug, not a retry |
| `a wallet outside the policy cannot sign` | The roster is enforced server-side, not merely rendered |
| `the same signer cannot sign twice` | Otherwise one approver reaches a threshold of two alone |
| `preparing before the chain write is refused` | A request that is not yet on chain has nothing to pin a signature to |

### Why this is separate from `make verify`

`make verify` covers what can be checked without a network: the contract, the schema, the linters,
both front ends. `make verify-api` covers what cannot.

The split is not tidiness. Every defect that reached a person during this project — an asset with no
contract, a request signed before it existed on chain, a settlement that held the treasury for ever
— was reachable from the API and invisible to a unit test.

**What to check:** `0 skipped`. A skip here means a precondition was missing and the case did not
run, which reads identically to a pass in a summary that only counts failures.

---

## 4 · The browser walkthrough

This is the part a script cannot show: that a person can drive it.

### It needs a real wallet

The website has no password login. Sign-in is **SEP-10**: the backend issues a challenge
transaction, the wallet signs it, the backend verifies the signature. So the browser path needs a
Stellar wallet extension — [Freighter](https://www.freighter.app/) — set to **testnet**.

The challenge is a transaction with **sequence number 0**, which means it can never be submitted.
The sign-in screen says so next to the button, because a person approving a transaction deserves to
know why it is safe.

To approve a payment you must hold a key that is on the treasury's roster. Import one into
Freighter:

```bash
stellar keys show settle-signer-1    # prints the secret; import it into Freighter as testnet
```

These are throwaway testnet keys generated by `make treasury`. They hold no real value.

To watch the threshold work rather than take it on faith, import **two** — `settle-signer-1` and
`settle-signer-2` — as separate Freighter accounts.

### The four screens

| # | Where | Do | Expect |
|---|---|---|---|
| 1 | http://localhost:5173 | Connect Freighter, sign the challenge | A session. Nothing is submitted to the network |
| 2 | `/requests/new` | Vendor `GBA2FFCMIU6EJKELZKDRRG6BC5IWBRLWLDPJMOC7M4QQZZAVMBATTH4X`, amount `10`, asset `XLM`, any reference | The button stays **disabled** until a ✓ confirms the account exists and can receive the asset |
| 3 | `/requests` | The list | Your payment, `● Pending`, with signature progress `0/2` |
| 4 | `/approve/:id` | Sign as signer 1 | `1 of 2`, and **not** "paid". A submitted transaction is not a confirmed payment |
| 5 | `/approve/:id` | Switch Freighter to signer 2, sign again | The threshold is met, the transaction submits, the status reaches `Paid` |
| 6 | `/evidence/:reference` | The reference from step 2 | Six milestone rows, each hash linking to Stellar Expert |

Things worth doing deliberately:

- **Paste a vendor address that does not exist** (change one character). The field refuses it before
  you can submit. Checking the trustline here rather than at submit time is what stops a payment
  failing at the ledger *after two people have already signed it*.
- **Try `0` as an amount, or an eighth decimal place.** Both refused at the field. Money is never a
  float anywhere in this system — `numeric(30,7)` in Postgres, `i128` stroops in Rust, `String` on
  the wire, because JSON numbers are IEEE doubles.
- **Click Create twice, fast.** One payment. The idempotency key is generated when the form opens.
- **Use Reject instead.** It is visually equal to Sign and never hidden. The request reaches
  `Cancelled` and nothing leaves the treasury.

**What to check:** step 4. If signing once showed "Paid", the threshold would be cosmetic — the
screen would be reporting our intention rather than the ledger's answer.

### The operations console — http://localhost:5174/admin

Sign in with `admin@settlement.local` / `123456`. One route for every table in the catalogue, read
**directly from Postgres**, never through the API.

That difference is the whole design. The website is wallet-facing and holds no credential of its
own; the console holds the database password and never leaves the operator's network. There is no
file per table either — the list comes from `pg_catalog` at request time, so a table added by a
migration is browsable the moment it runs, rather than quietly missing until somebody notices.

Worth opening after the walkthrough above:

| Table | Shows |
|---|---|
| `payment_requests` | Your payment, and its status |
| `settlements` | The reserved treasury sequence number — the row the uniqueness constraint protects |
| `settlement_approvals` | One row per signature, with the signer address |
| `governance_policies` · `policy_signers` | The roster, version 1, threshold 2 |
| `chain_events` | What the chain sync ingested |
| `audit_events` | Who did what |

Full detail: [`docs/11-admin.md`](docs/11-admin.md).

---

## 5 · The on-chain round trip

This spends testnet lumens and **rewrites [`.deploy/evidence.md`](.deploy/evidence.md)**. The
committed file is the submission artefact, so run this when you want to reproduce the evidence, not
casually.

See the steps without touching the network or the evidence file:

```bash
make demo-plan                     # 16 steps listed, no network touched
```

Then, for real:

```bash
make demo                          # the chain round trip, writing .deploy/evidence.md
DEMO_ASSET=USDC make demo          # the same round trip paying an issued asset
```

Three cases: **A** happy path, **B** reject branch, **C** abuse resistance — duplicate id, zero
amount, past expiry, second `mark_paid`, each of which must be refused.

Each case establishes a **positive control** first. If the registry does not answer, the case
abandons itself rather than letting every "must be refused" check pass against a broken network —
which is the failure mode where a demo looks greenest exactly when nothing is working.

The evidence file is written **as each step happens**, not at the end. A hash not captured at the
moment it happens is a hash gone, so a run that dies half way still leaves behind everything it did
prove. A step that misbehaves is marked *NOT as specified* and the script exits non-zero, so a
broken run cannot be submitted as a green one.

### From nothing

If testnet has reset and the deployed contract went with it:

```bash
make deploy KEY=<your stellar key name>   # deploys AND initialises in one transaction
make treasury                             # 3 signers, thresholds 2, masterWeight 0, USDC trustline
make verify-treasury                      # reads it back from Horizon and ASSERTS
make demo
```

Under ten minutes, no manual step. `make treasury` also issues the treasury a working USDC balance
from an issuer it generates, so a USDC demo needs no browser faucet — set `USDC_ISSUER` first to use
a real issuer's asset instead. Both targets sync what they deployed into `.env`, so the console is
never pointed at a contract that is no longer there.

**What to check:** `make verify-treasury` reads the account back from Horizon and asserts
`masterWeight` is **0**. If the master key still had weight, the treasury could pay with one
signature and the entire approval gate would be optional.

---

## 6 · What runs without a network

```bash
make verify      # 21 checks: both Rust workspaces, both front ends, the schema, and the docs
```

It starts its own throwaway Postgres and fails on the first thing that is not as specified. The
checks worth naming:

- The registry contains **no token call** — and the gate's own self-test proves it can detect one.
  The contract never holds a single unit of anything; it records and verifies. That is what lets it
  reach testnet without a security audit.
- Every migration **reverses**, leaving no table, enum, domain or function behind.
- Every constraint **refuses** what it claims to refuse, including the one that stops two payments
  contending for one treasury sequence number.
- The docs and the code still agree: error numbers, routed endpoints, cited tests, translations.
- The wasm stays within its size ceiling, and no float appears anywhere in the source.

**What to check:** the no-token-call gate's self-test. A grep that has never been shown to fail is a
grep that might be matching nothing.

---

## 7 · When something is wrong

| Symptom | Cause | Fix |
|---|---|---|
| `the Docker daemon is not running` | Docker Desktop is not started | Start it |
| The backend never becomes ready | Migrations failed; the script prints the last 40 log lines | `docker compose logs backend` |
| Port 5173 or 5174 is already taken | Another stack, or a container under an old service name | `make stop`, or set `WEBSITE_PORT` / `ADMIN_PORT` |
| `TREASURY_ADDRESS is not set` | The treasury has never been created here | `make treasury` |
| The approver screen shows INTERNAL | The policy was never enrolled | `make policy` |
| Sign-in fails in the browser | Freighter is on mainnet, or the key is not on the roster | Switch to testnet; import `settle-signer-1` |
| A case says *NOT as specified* | Read that row of `.deploy/evidence.md` | It records what was expected and what happened |

Every host port is overridable — `BACKEND_PORT`, `WEBSITE_PORT`, `ADMIN_PORT`, `POSTGRES_PORT`,
`ADMINER_PORT` — so a second stack on the same machine does not fail to start over a port number.

Need to look at the database directly:

```bash
docker compose --profile sql up -d adminer     # http://localhost:8081
```

---

## 8 · Teardown

```bash
make stop                          # stop, keeping the database
make fresh                         # destroy the volume and rebuild from nothing
```

`make fresh` takes the console account with it, and the next `make dev` recreates it. That is the
point: the
account exists because somebody ran the command, which is the property the console's design cares
about.

---

## What this deliberately does not do

Out of scope, quoted from the statement of work: a full dashboard, vendor-facing views, a
reconciliation indexer, accounting export, a security audit, and mainnet deployment. Four more were
added to protect the schedule: no escrow holding funds, no fiat off-ramp, no internationalisation,
no mobile application.

**The contract holds no money.** Custody is a different design, a different security posture, and a
different conversation about liability — [`docs/16-risks.md`](docs/16-risks.md).

## Where to read next

| Question | Document |
|---|---|
| What is being built, and what is deliberately not? | [`docs/00-overview.md`](docs/00-overview.md) |
| What does the contract do, exactly? | [`docs/04-contract-interface.md`](docs/04-contract-interface.md) |
| Why is the sequence number such a problem? | [`docs/06-signature-service.md`](docs/06-signature-service.md) |
| How is the evidence defined? | [`docs/15-evidence.md`](docs/15-evidence.md) |
| What could go wrong? | [`docs/16-risks.md`](docs/16-risks.md) |
