# The root Makefile delegates. It builds nothing itself, so there is exactly one place that knows
# how to build each part.
#
# Three commands do everything:
#
#   make dev     the stack in Docker, every service on its own port
#   make prod    the same stack behind nginx on one port, as it runs on a VM
#   make demo    the on-chain round trip against the registry contract, and the evidence file
#
# Everything else below is one of those taken apart: a single stage, run on its own.

.PHONY: help all contract backend frontend test lint fmt verify verify-guarantees verify-api \
        verify-wallet-auth verify-treasury clean fresh logs ps env deploy treasury policy chain \
        dev prod stop restart nginx-check demo demo-plan evidence admin-user demo-admin

CONTRACT := smart-contract
BACKEND  := backend
WEBSITE  := frontend/website
ADMIN    := frontend/admin

NETWORK ?= testnet

## Compose names its project after the directory it is run from, and this directory is called
## `monorepo` — a name shared with every other checkout called that on the same machine, and one
## that renames this stack's volumes the moment the tree moves. Pinned so the containers, the
## network and the database keep their identity wherever the checkout lives.
export COMPOSE_PROJECT_NAME ?= settleflow

## The VM layer: nginx in front, every other port on loopback. Both files, in this order, wherever
## the production stack is addressed — `docker compose down` against the base file alone would not
## know nginx exists and would leave the front door running after the command that stopped
## everything else.
COMPOSE_PROD := docker compose -f docker-compose.yml -f docker-compose.prod.yml

.DEFAULT_GOAL := help

help:  ## Print every target with its purpose.
	@grep -E '^[a-zA-Z_-]+:.*?## .*$$' $(MAKEFILE_LIST) \
		| awk 'BEGIN {FS = ":.*?## "}; {printf "  %-18s %s\n", $$1, $$2}'

# ---------------------------------------------------------------------------
# Running it
# ---------------------------------------------------------------------------
#
# `dev` and `prod` are the same three steps — deploy the registry if it is not deployed, configure
# the treasury if it is not configured, then bring the stack up. The only difference is the front
# door: `dev` publishes every service on its own port, `prod` puts nginx on one port and moves the
# rest to loopback.
#
# The first two steps are skipped whenever what they would build already ANSWERS on chain, so both
# are a single command on a machine that has never seen this project and a plain `compose up` on
# every run after.

## Five steps: deploy the registry and the treasury if they are missing, `docker compose up`, enrol
## the signer roster, seed the demo console account. Every one of them is idempotent, so the second
## run is close to a plain `compose up`.
dev: ## Run locally: everything, including the signer roster and a console login.
	@./scripts/dev.sh

## PUBLIC_HOST is the address a browser types. It reaches the backend's SEP-10 challenge and its
## CORS rules, so a stack published under a domain must say so here or wallet sign-in refuses every
## attempt with a domain mismatch — which reads like a wallet fault rather than a configuration one.
##
##   make prod
##   make prod PUBLIC_HOST=pay.example.com
##   make prod HTTP_PORT=8080          when 80 is taken
prod: ## Run the whole stack in Docker behind nginx, as it runs on a VM.
	@./scripts/prod.sh

stop: ## Stop everything, keeping its data.
	$(COMPOSE_PROD) down --remove-orphans

restart: stop prod ## Stop and start the production stack again.

ps: ## What is running.
	@$(COMPOSE_PROD) ps

logs: ## Follow the backend log.
	@$(COMPOSE_PROD) logs -f backend

## nginx fails closed on a bad configuration, so a typo in the front door takes the whole stack down
## at start-up rather than degrading one route. This checks it without starting anything.
nginx-check: ## Validate deploy/nginx/nginx.conf.
	@docker run --rm -v "$(CURDIR)/deploy/nginx/nginx.conf:/etc/nginx/conf.d/default.conf:ro" \
		nginx:1.27-alpine nginx -t

# ---------------------------------------------------------------------------
# The demo
# ---------------------------------------------------------------------------
#
# The registry contract, exercised on testnet. It needs no stack — only the deployed contract and
# the treasury — which is why it is a command of its own rather than a stage of `make dev`.

## Every case end to end, writing .deploy/evidence.md AS IT GOES: a hash not captured at the moment
## it happens is a hash gone, so a run that dies half way still leaves behind what it did prove.
##
## Three cases. A, the happy path: create, approve on 2 of 3, pay the vendor, mark paid. B, the
## reject branch, which ASSERTS no transaction left the treasury. C, abuse — a duplicate id, a
## second payment, a non-settler approving — each of which must be refused.
demo: ## Run the contract's round-trip demo on testnet, and write the evidence file.
	@./scripts/demo.sh

demo-plan: ## List what the demo would do, without touching the network.
	@DRY_RUN=1 ./scripts/demo.sh

evidence: ## Show the evidence table a reviewer opens.
	@cat .deploy/evidence.md

# ---------------------------------------------------------------------------
# The chain deployment
# ---------------------------------------------------------------------------
#
# `make dev` and `make prod` both call `chain`, so none of this normally has to be run by hand. It
# is here for the times it does: rebuilding after a testnet reset, or deploying from a key you
# already hold rather than the one that gets generated.

## Reuses whatever still answers and rebuilds what does not — a `.deploy/env` naming a contract is
## not evidence the contract exists, and testnet resets take every contract and funded account with
## them while the file stays exactly as it was.
chain: ## Deploy the registry and treasury if missing, reuse them if they answer.
	@./scripts/chain_up.sh $(ARGS)

## Both targets sync the addresses they produced into .env afterwards. A contract deployed but never
## written to the environment the backend reads is a deployment the console cannot see.
deploy: ## Deploy the registry to testnet. KEY=<stellar key name> required.
	$(MAKE) -C $(CONTRACT) deploy KEY=$(KEY) NETWORK=$(NETWORK)
	@./scripts/env.sh

treasury: ## Create and configure the treasury: 3 signers, threshold 2, masterWeight 0.
	cd $(BACKEND) && ./scripts/setup_treasury.sh
	@./scripts/env.sh

## Reads the roster from the chain rather than taking it as an argument, so the database and the
## account cannot disagree about who may approve a payment. Without it `prepare` has no policy
## version to pin and the approver screen shows INTERNAL where the signature progress should be.
policy: ## Enrol the treasury's signers as a governance policy. Needed before the first approval.
	cd $(BACKEND) && ./scripts/enrol_policy.sh

env: ## Write .env if it does not exist.
	@./scripts/env.sh

# ---------------------------------------------------------------------------
# Console accounts
# ---------------------------------------------------------------------------
#
# The console has no sign-up page and no bootstrap-by-environment-variable: both are ways for an
# account to exist that nobody decided to create.

## Run inside the admin container, not on the host. The script needs `pg`, and the host has
## frontend/admin/node_modules only if somebody has developed there — so on a server the host
## spelling fails with a module-not-found error about a package the container has had all along.
##
## No `-T`, so the interactive password prompt still gets a terminal.
admin-user: ## Manage console accounts. ARGS="create ops@example.com operator"
	@docker compose exec -e DATABASE_URL="postgres://settle:settle@postgres:5432/settle" \
		admin node scripts/admin-user.mjs $(ARGS)

## The demo console account. Deliberately weak, deliberately local: the password is in this file, in
## the repository, for anyone to read. `make fresh` takes the database volume and this account with
## it, so it is rebuilt several times an hour and a strong one would be a strong one for nobody.
##
## It still requires ALLOW_WEAK_PASSWORD=1, which nothing in the deployment sets.
DEMO_ADMIN_EMAIL    ?= admin@settlement.local
DEMO_ADMIN_PASSWORD ?= 123456

## `make dev` already does this. It is here for after a `make fresh`, which takes the database
## volume and this account with it.
demo-admin: ## Seed the local demo console account (weak password, local only).
	@docker compose exec -T -e DATABASE_URL="postgres://settle:settle@postgres:5432/settle" \
		-e ADMIN_PASSWORD="$(DEMO_ADMIN_PASSWORD)" -e ALLOW_WEAK_PASSWORD=1 \
		admin node scripts/admin-user.mjs create $(DEMO_ADMIN_EMAIL) superadmin \
		|| echo "  (already exists — leaving it alone)"
	@echo "  console  http://localhost:$${ADMIN_PORT:-5174}/admin   $(DEMO_ADMIN_EMAIL) / $(DEMO_ADMIN_PASSWORD)"

# ---------------------------------------------------------------------------
# Building and verifying
# ---------------------------------------------------------------------------

all: contract backend frontend  ## Build everything.

contract: ## Build and test the registry contract.
	$(MAKE) -C $(CONTRACT) all

backend: ## Build and test the backend workspace.
	$(MAKE) -C $(BACKEND) all

frontend: ## Typecheck, test and build both front ends.
	cd $(WEBSITE) && npm ci --ignore-scripts && npm run typecheck && npm test && npm run build
	cd $(ADMIN) && npm ci --ignore-scripts && npm run typecheck && npm test && npm run build

test: ## Every test suite.
	$(MAKE) -C $(CONTRACT) test
	$(MAKE) -C $(BACKEND) test
	cd $(WEBSITE) && npm test
	cd $(ADMIN) && npm test

lint: ## Every linter, at CI strictness.
	$(MAKE) -C $(CONTRACT) lint
	$(MAKE) -C $(BACKEND) lint
	cd $(WEBSITE) && npm run typecheck
	cd $(ADMIN) && npm run typecheck

fmt: ## Format everything.
	$(MAKE) -C $(CONTRACT) fmt
	$(MAKE) -C $(BACKEND) fmt

## Everything, in one command: both Rust workspaces, the console, the schema, and the claims the
## documentation makes. Starts its own throwaway Postgres and fails on the first thing that is not
## as specified.
verify: ## Verify every part of the project.
	@./scripts/verify_all.sh

## The properties that are worth nothing unless something has watched them fail.
verify-guarantees: ## Prove the guarantees rather than asserting them.
	$(MAKE) -C $(CONTRACT) gate
	$(MAKE) -C $(BACKEND) verify-migrations
	$(MAKE) -C $(BACKEND) verify-constraints

## Every path through the API against a running stack: validation, idempotency, both assets, the
## sequence lock, signature collection, settlement, evidence, and the reject branch.
##
## This is where an API-reachable bug is supposed to be found. Each one that instead reached a person
## — an asset with no contract, a request signed before it existed on chain, a settlement that held
## the treasury for ever — was reachable from here.
verify-api: ## Exercise every API path end to end. Needs `make dev`.
	@VENDOR=$$(stellar keys address settle-vendor 2>/dev/null) \
	 SIGNER_SECRETS=$$(stellar keys show settle-signer-1 2>/dev/null | tail -1),$$(stellar keys show settle-signer-2 2>/dev/null | tail -1) \
	 BASE=$${BASE:-http://localhost:$${BACKEND_PORT:-8088}} \
	 python3 $(BACKEND)/scripts/verify_api.py

## Needs a running backend, which is why it is not part of `make verify`. It signs with its own
## ed25519 implementation rather than the library the server verifies with, so the two agreeing
## means something.
verify-wallet-auth: ## Sign in over SEP-10 for real, and assert replay and a wrong key are refused.
	@BASE=$${BASE:-http://localhost:$${BACKEND_PORT:-8088}} python3 $(BACKEND)/scripts/verify_wallet_auth.py

verify-treasury: ## Read the treasury back from Horizon and ASSERT its configuration. Evidence E2.
	cd $(BACKEND) && ./scripts/verify_treasury.sh

# ---------------------------------------------------------------------------
# Housekeeping
# ---------------------------------------------------------------------------

clean: ## Remove build output.
	$(MAKE) -C $(CONTRACT) clean
	$(MAKE) -C $(BACKEND) clean
	rm -rf $(WEBSITE)/.next $(ADMIN)/.next

## Separate from `clean`, and named so it cannot be run by accident: this one takes the database.
fresh: ## Stop everything AND destroy the database volume.
	$(COMPOSE_PROD) down -v --remove-orphans
	@echo "gone: containers and the database. The chain deployment in .deploy/ is untouched."
