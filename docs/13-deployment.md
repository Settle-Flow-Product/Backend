# 13 · Deployment

Modules G1 and G4. Build, deploy, configure, verify — and be able to do the whole thing again from
nothing in under ten minutes.

The evidence the deployment produces is [15-evidence.md](15-evidence.md). The treasury configuration
it applies is [05-treasury-multisig.md](05-treasury-multisig.md).

## Read first

- **Testnet resets.** When it does, the deployed contract and every demo transaction go with it.
  That is why the ten-minute rebuild is a requirement rather than a nicety — it is the difference
  between a lost afternoon and a lost deliverable.
- **No manual step.** Not one address typed by hand, not one configuration set through a UI. A step
  a human performs is a step that is skipped under demo-day pressure, and `masterWeight = 0` is
  exactly the step that gets skipped.
- **Testnet only.** Mainnet is explicitly out of scope.
- **Pin the SDK on Day 1.** `Cargo.lock` is committed and not updated mid-sprint. A Soroban SDK
  breaking change costs one to two days, and it is a named project risk.

## Prerequisites

| Tool | Version |
|---|---|
| Rust | pinned in `rust-toolchain.toml`, target `wasm32v1-none` |
| `stellar` CLI | current |
| `jq` | any |
| Docker + Compose | for Postgres and the backend |
| Node | 22, for the two frontends |

## Build

```bash
make -C smart-contract build      # fmt, clippy -D warnings, test, then the release wasm
make -C backend all               # fmt, clippy, test, build
cd frontend/website && npm ci && npm run build
cd frontend/admin   && npm ci && npm run build
```

The contract build runs against the release profile — `opt-level = "z"`, LTO, symbols stripped. That
profile only applies at a workspace root, which is why the contract and the backend are two separate
Cargo workspaces and are never merged.

CI additionally asserts the wasm size and runs the no-custody grep gate from
[04-contract-interface.md](04-contract-interface.md).

## Deploy

```bash
make -C smart-contract deploy KEY=deployer NETWORK=testnet
```

`scripts/deploy.sh` deploys and configures the contract **in one transaction** —
`stellar contract deploy … -- --admin … --settler …`, which invokes the contract's
`__constructor`.

The contract has no `initialize` entry point, deliberately. A contract deployed in one transaction
and configured in another has a window between the two in which anyone may configure it. There is
no such window here, and the script reads the configuration back from the chain before it writes
anything to `.deploy/env`.

It writes `.deploy/env`, which holds addresses and never secrets:

```
NETWORK=testnet
REGISTRY=CBIE………………………
TREASURY=GTRE………………………
SETTLER=GSET………………………
ADMIN=GADM………………………
USDC_SAC=CBIELTK6………………
```

## Configure the treasury

```bash
./backend/scripts/setup_treasury.sh     # B1 + B2: fund, trustline, 3 signers, thresholds, master 0
./backend/scripts/verify_treasury.sh    # B5: read back from Horizon and ASSERT
```

`verify_treasury.sh` asserts rather than prints. A script that prints the configuration and leaves a
human to check it is a script that passes on demo day while the master key still has weight 1. Its
output is evidence milestone **E2**.

| Asserted | Expected |
|---|---|
| Signers, excluding master | 3, each weight 1 |
| `low_threshold` / `med_threshold` / `high_threshold` | 2 / 2 / 2 |
| Master key weight | **0** |
| USDC trustline | present, **when the demo pays USDC**. Native XLM needs none, so the check follows the asset rather than failing a perfectly good XLM treasury |

## Run the stack

```bash
make dev           # postgres, backend, website, admin
make migrate       # apply every migration
```

| Service | Port | Override with |
|---|---|---|
| backend | 8088 → 8080 (`/health`, `/ready`) | `BACKEND_PORT` |
| website | 5173 | `WEBSITE_PORT` |
| admin | 5174 | `ADMIN_PORT` |
| postgres | 5432 | `POSTGRES_PORT` |
| adminer | 8081 (compose profile `sql`) | `ADMINER_PORT` |

Only the host side of each mapping is overridable. They exist because a second stack on the same
machine should not fail to start over a port number — a developer running another project's Postgres
on 5432 is the normal case, not a misconfiguration.

## Configuration

The backend reads everything from the environment at start-up. An `Option` sub-struct means a
subsystem is off; half-configured is a boot failure rather than a surprise at runtime.

| Variable | Purpose | Default |
|---|---|---|
| `DATABASE_URL` | Postgres | **required** |
| `BIND` | Listen address | `0.0.0.0:8080` |
| `SOROBAN_RPC_URL` | Soroban RPC | `https://soroban-testnet.stellar.org` |
| `HORIZON_URL` | Horizon | `https://horizon-testnet.stellar.org` |
| `NETWORK_PASSPHRASE` | Network binding | `Test SDF Network ; September 2015` |
| `REGISTRY_CONTRACT_ID` | The deployed registry | **required** |
| `TREASURY_ADDRESS` | The G-account payments are drawn on | **required** |
| `BUSINESS_KEY` / `SETTLER_KEY` | Stellar CLI key **names**. The CLI holds the secrets in its own keystore, so no private key passes through this process or its logs | **required** |
| `SESSION_KEY` | Signs the website's session tokens | **required** |
| `HOME_DOMAIN` | Bound into the auth challenge | `localhost:5173` |
| `CHALLENGE_TTL_SECONDS` | Challenge lifetime | `300` |
| `TX_TIMEBOUNDS_SECONDS` | Signing window | `3600` |
| `INDEXER_INTERVAL_SECONDS` | Ingest poll | `5` |
| `ALLOWED_ORIGINS` | CORS | `http://localhost:5173` |
| `RUST_LOG` | Log filter | `info,sqlx=warn` |

`SESSION_KEY`, `BUSINESS_KEY` and `SETTLER_KEY` are never defaulted. A default secret is a secret
in production.

### The two frontends

| Variable | Read by | Purpose |
|---|---|---|
| `API_TARGET` | website | The backend it proxies to, server-side |
| `DATABASE_URL` | admin | The console reads Postgres directly — see [11-admin.md](11-admin.md) |
| `DATABASE_POOL_MAX` | admin | Connections the console may hold (`6`) |
| `ADMIN_SESSION_KEY` | admin | Signs the console's session cookie. A **different** value from `SESSION_KEY`: a token minted for one app must not open the other |
| `COOKIE_SECURE` | admin | `true` behind TLS |

**There is no `NEXT_PUBLIC_*` variable in either app**, so one image runs against any environment. In
the console that is also a security property: a database URL baked in at build time would be a
credential sitting in a browser bundle.

Neither app's config is a `.ts` file. Next reaches for the TypeScript compiler to load
`next.config.ts` at start-up, and the production image has no devDependencies to find one in — so
both use `next.config.mjs`.

## Start-up order

The backend's start-up sequence is load-bearing and fails fast:

```
1  connect to Postgres and run migrations
2  verify the registry contract responds and the treasury account exists
3  build the chain client
4  spawn the indexer and submitter tasks
5  bind the port
6  serve, with graceful shutdown
```

Step 2 happens **before** step 5 deliberately. A backend that binds its port and then discovers it
cannot reach the chain reports itself healthy while being useless.

## Post-deployment checklist

- [ ] `verify_treasury.sh` passes every assertion.
- [ ] `get_request` on a freshly created request returns `Pending` via RPC simulation.
- [ ] The registry contract id is pinned in the root `README.md`.
- [ ] `.deploy/evidence.md` carries the contract id and every hash captured so far.
- [ ] `curl localhost:8088/ready` returns 200.
- [ ] `make verify-wallet-auth` passes — a real SEP-10 signature is accepted, a replayed nonce and a
      signature from a different key are both refused.
- [ ] The website signs in with a wallet and lists requests.
- [ ] The admin console signs in with an account made by `make admin-user`, lists a table, and
      returns 404 for `/admin/admin_users`.
- [ ] No secret appears anywhere in the repository or in a log line.
- [ ] The whole sequence has been re-run from scratch and timed at under ten minutes.

## Redeploying after a testnet reset

```bash
make -C smart-contract deploy KEY=deployer     # new contract id
./backend/scripts/setup_treasury.sh            # new treasury, new signers
./backend/scripts/verify_treasury.sh
make migrate && make dev
```

Every identifier changes. Nothing in the code has an address hard-coded; they all come from
`.deploy/env` and the environment, which is what makes the rebuild a script rather than a search.

## The rules this document imposes

- **No manual step.** If a human types an address, the script is unfinished.
- **Deploy and configure in one transaction**, through `__constructor`.
- **Verification asserts**, it does not print.
- **`Cargo.lock` is pinned on Day 1** and not updated mid-sprint.
- **Secrets are never defaulted** and never logged.
- **Fail before binding the port**, not after.
- **The rebuild is timed**, and ten minutes is the number.
