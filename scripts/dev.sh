#!/usr/bin/env bash
#
# Local development, in one command:
#
#   1. deploy the registry contract, if it is not deployed
#   2. configure the treasury, if it is not configured
#   3. docker compose up
#   4. enrol the treasury's signer roster as a governance policy
#   5. seed the demo console account
#
# Steps 1 and 2 are skipped when what they would build already answers on chain, and 4 and 5 are
# idempotent — so this is a full build on a machine that has never seen the project and close to a
# plain `docker compose up` on every run after.
#
# 4 and 5 are here because without them the stack is up and unusable: `prepare` has no policy
# version to pin and the approver screen shows INTERNAL, and the console has no sign-up page so
# there is no way in at all. `make prod` does 4 as well, but NOT 5 — the demo account's password is
# weak and published, which is right for a laptop and wrong for a server.
#
# nginx is in front here exactly as it is on the VM — same file, same routes — so an origin is never
# something that only exists in production. The difference from `make prod` is that this ALSO
# publishes every service on its own port, which is what you want while you are changing one; there
# they are on loopback and the front door is the only way in.

set -uo pipefail
cd "$(dirname "$0")/.."

# The same default the Makefile exports, so a stack started by `make dev` and one started by
# `./scripts/dev.sh` are the same stack rather than two with different names.
export COMPOSE_PROJECT_NAME="${COMPOSE_PROJECT_NAME:-settleflow}"

if [ -t 1 ]; then
    C_B=$'\033[1m'; C_OK=$'\033[32m'; C_ERR=$'\033[31m'; C_DIM=$'\033[2m'; C_0=$'\033[0m'
else
    C_B=; C_OK=; C_ERR=; C_DIM=; C_0=
fi
banner() { printf '\n%s── %s %s\n' "$C_B" "$*" "$C_0"; }
ok()     { printf '%s✓%s %s\n' "$C_OK" "$C_0" "$*"; }
note()   { printf '%s  %s%s\n' "$C_DIM" "$*" "$C_0"; }
die()    { printf '\n%s✗%s %s\n' "$C_ERR" "$C_0" "$*" >&2; exit 1; }

command -v docker >/dev/null || die "docker is not installed"
docker compose version >/dev/null 2>&1 || die "docker compose v2 is not available"
docker info >/dev/null 2>&1 || die "the Docker daemon is not running — start Docker Desktop"

# A bind mount whose source does not exist is not an error: Docker creates the path as an empty
# DIRECTORY and mounts that, and nginx then fails with "not a directory", which names the symptom
# rather than the cause — and leaves a directory sitting where the configuration used to be.
[ -f deploy/nginx/nginx.conf ] || die "deploy/nginx/nginx.conf is missing or is not a file.
  Restore it:  git checkout deploy/nginx/nginx.conf"

# ── 1 and 2 · the registry and the treasury ─────────────────────────────────
#
# Before the build, not after. The backend refuses to start without REGISTRY_CONTRACT_ID and
# TREASURY_ADDRESS — neither has a default, on purpose — so a stack brought up first would restart
# in a loop on a missing variable rather than say what is missing.

banner "1-2 · registry and treasury"
./scripts/chain_up.sh || die "the chain deployment did not complete"

# chain_up.sh has just synced what it deployed into .env, which is where compose reads it.
set -a
# shellcheck disable=SC1091
. ./.env
set +a

# ── 3 · the stack ───────────────────────────────────────────────────────────
#
# `--remove-orphans` is not tidiness. The console service was split into `website` and `admin`, and
# a container left behind under the old name still holds 5173.

banner "3 · docker compose up"
docker compose up -d --build --remove-orphans || die "docker compose up failed"

BACKEND_PORT="${BACKEND_PORT:-8088}"
WEBSITE_PORT="${WEBSITE_PORT:-5173}"
ADMIN_PORT="${ADMIN_PORT:-5174}"
HTTP_PORT="${HTTP_PORT:-80}"

SUFFIX=""
[ "$HTTP_PORT" = "80" ] || SUFFIX=":$HTTP_PORT"
ORIGIN="http://localhost$SUFFIX"

# Polled rather than slept. On a cold machine the backend is still running migrations well after the
# container is "up", and every instruction printed below would be wrong for another minute.
printf '  waiting for the backend'
DEADLINE=$(( $(date +%s) + 180 ))
until curl -fsS -m 3 "http://localhost:$BACKEND_PORT/ready" >/dev/null 2>&1; do
    if [ "$(date +%s)" -ge "$DEADLINE" ]; then
        printf '\n'
        docker compose logs --tail 40 backend
        die "the backend did not become ready within 180s (log above)"
    fi
    printf '.'
    sleep 2
done
printf '\n'
ok "http://localhost:$BACKEND_PORT/ready"

# nginx answers /healthz itself, so this reports the front door rather than an upstream — two
# different faults, worth telling apart.
printf '  waiting for the front door'
DEADLINE=$(( $(date +%s) + 60 ))
until curl -fsS -m 3 "$ORIGIN/healthz" >/dev/null 2>&1; do
    if [ "$(date +%s)" -ge "$DEADLINE" ]; then
        printf '\n'
        docker compose logs --tail 20 nginx
        die "nginx did not answer within 60s (log above). Is port $HTTP_PORT already taken?"
    fi
    printf '.'
    sleep 2
done
printf '\n'
ok "$ORIGIN/healthz"

# ── 4 · the governance policy ───────────────────────────────────────────────
#
# Read from the chain, never typed: the signers and the threshold come from Horizon, so the account
# and the database cannot drift into disagreeing about who may approve a payment. Idempotent —
# enrolling an unchanged roster writes no new version — which is why it is unconditional rather than
# guarded by a query.

banner "4 · governance policy"
( cd backend && ./scripts/enrol_policy.sh ) || die "enrolling the signer roster failed"

# ── 5 · the demo console account ────────────────────────────────────────────
#
# Run INSIDE the admin container rather than on the host. The script needs `pg`, and the host has
# frontend/admin/node_modules only if somebody has developed there — so on a clean machine the host
# spelling fails with a module-not-found error about a package the container has had all along.
#
# Deliberately weak, deliberately local: the password is in the Makefile, in this file, and in the
# repository, for anyone to read. `make fresh` takes the database volume and this account with it,
# so it is rebuilt several times an hour and a strong one would be a strong one for nobody. It
# still requires ALLOW_WEAK_PASSWORD=1, which nothing in the deployment sets.

banner "5 · console account"
DEMO_ADMIN_EMAIL="${DEMO_ADMIN_EMAIL:-admin@settlement.local}"
DEMO_ADMIN_PASSWORD="${DEMO_ADMIN_PASSWORD:-123456}"
if docker compose exec -T \
        -e DATABASE_URL="postgres://settle:settle@postgres:5432/settle" \
        -e ADMIN_PASSWORD="$DEMO_ADMIN_PASSWORD" \
        -e ALLOW_WEAK_PASSWORD=1 \
        admin node scripts/admin-user.mjs create "$DEMO_ADMIN_EMAIL" superadmin >/dev/null 2>&1; then
    ok "$DEMO_ADMIN_EMAIL created"
else
    note "$DEMO_ADMIN_EMAIL already exists — leaving it alone"
fi

banner "up"
printf '  %-9s %s\n' "website" "$ORIGIN"
printf '  %-9s %s\n' "admin"   "$ORIGIN/admin   $DEMO_ADMIN_EMAIL / $DEMO_ADMIN_PASSWORD"
printf '  %-9s %s\n' "api"     "$ORIGIN/api   → the backend, prefix stripped"
printf '\n'
note "and each service on its own port, which prod does not publish:"
note "  website $WEBSITE_PORT · admin $ADMIN_PORT/admin · backend $BACKEND_PORT (/health · /ready)"
printf '\n'
note "registry  ${REGISTRY_CONTRACT_ID:-—}"
note "treasury  ${TREASURY_ADDRESS:-—}"
printf '\n'
printf '  %smake demo%s        the on-chain round trip\n' "$C_B" "$C_0"
printf '  %smake verify-api%s  38 API cases against this stack\n' "$C_B" "$C_0"
printf '  %smake stop%s        stop everything\n' "$C_B" "$C_0"
printf '\n'
