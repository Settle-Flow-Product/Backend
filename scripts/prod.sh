#!/usr/bin/env bash
#
# The whole stack in containers, behind nginx, exactly as it runs on a VM.
#
# This is what a fresh server needs: it deploys the chain side if it is missing, builds the images,
# starts everything, waits for the front door, enrols the signer roster, and prints one URL.
#
#   make prod                        on the VM, reachable at http://<the VM's address>
#   PUBLIC_HOST=pay.example.com make prod
#
# ## One port, one origin
#
# docker-compose.yml publishes a port per service for the convenience of a local run. On a server
# that convenience is four open ports, each with its own idea of who may talk to it — so BIND_ADDR
# is set to loopback here and nginx is the only thing reachable from outside. The published ports
# still exist for the demo and verification scripts, which run ON the VM.
#
# Exit 0 = the front door answered and the stack is serving.

set -uo pipefail
cd "$(dirname "$0")/.."

if [ -t 1 ]; then
    C_B=$'\033[1m'; C_OK=$'\033[32m'; C_ERR=$'\033[31m'; C_DIM=$'\033[2m'; C_Y=$'\033[33m'; C_0=$'\033[0m'
else
    C_B=; C_OK=; C_ERR=; C_DIM=; C_Y=; C_0=
fi
banner() { printf '\n%s── %s %s\n' "$C_B" "$*" "$C_0"; }
step()   { printf '%s→%s %s\n' "$C_B" "$C_0" "$*"; }
ok()     { printf '%s✓%s %s\n' "$C_OK" "$C_0" "$*"; }
warn()   { printf '%s!%s %s\n' "$C_Y" "$C_0" "$*"; }
note()   { printf '%s  %s%s\n' "$C_DIM" "$*" "$C_0"; }
die()    { printf '\n%s✗%s %s\n' "$C_ERR" "$C_0" "$*" >&2; exit 1; }

COMPOSE=(docker compose -f docker-compose.yml -f docker-compose.prod.yml)

# The same default the Makefile exports, so this script and enrol_policy.sh address the same stack
# whether they were reached through `make prod` or run directly.
export COMPOSE_PROJECT_NAME="${COMPOSE_PROJECT_NAME:-settleflow}"

# Loopback, and exported so the compose files below see it. Overridable for the rare case of a VM
# behind a load balancer that reaches the services directly.
export BIND_ADDR="${BIND_ADDR:-127.0.0.1}"
export PUBLIC_HOST="${PUBLIC_HOST:-localhost}"
HTTP_PORT="${HTTP_PORT:-80}"
export HTTP_PORT

# The origin a browser types.
#
# Port 80 is elided because a browser elides it, and an instruction that does not match what ends up
# in the address bar is one the reader has to second-guess. A PUBLIC_HOST that already carries a
# port is left alone — it is the more specific statement of the two, and appending HTTP_PORT to it
# produced `localhost:8099:8099`.
case "$PUBLIC_HOST" in
    *:*) ORIGIN="http://$PUBLIC_HOST" ;;
    *)   if [ "$HTTP_PORT" = "80" ]; then
             ORIGIN="http://$PUBLIC_HOST"
         else
             ORIGIN="http://$PUBLIC_HOST:$HTTP_PORT"
         fi ;;
esac

# ---------------------------------------------------------------------------
# 1 · Preflight
# ---------------------------------------------------------------------------

banner "1 · preflight"
need() { command -v "$1" >/dev/null 2>&1 || die "$1 is not installed${2:+ — $2}"; }
need docker
need curl
docker compose version >/dev/null 2>&1 || die "docker compose v2 is not available"
docker info >/dev/null 2>&1 || die "the Docker daemon is not running"

# Checked as a FILE, and checked here rather than left to the bind mount.
#
# A bind mount whose source does not exist is not an error: Docker creates the path as an empty
# DIRECTORY and mounts that instead. nginx then fails with "not a directory ... are you trying to
# mount a directory onto a file", which names the symptom and not the cause, and the front door is
# left as a directory sitting where the configuration used to be — so the next run fails the same
# way for what now looks like a different reason.
[ -f deploy/nginx/nginx.conf ] || die "deploy/nginx/nginx.conf is missing or is not a file.
  Docker would silently mount an empty directory in its place. Restore it from git:
    git checkout deploy/nginx/nginx.conf"

ok "docker, compose, curl, nginx.conf"

# ---------------------------------------------------------------------------
# 2 · The chain
# ---------------------------------------------------------------------------
#
# Before the build, because the backend refuses to boot without a registry and a treasury, and a
# ten-minute image build that ends in a container restart-looping on a missing variable is a bad way
# to be told. Reuses a deployment that still answers, so this is a no-op on every run but the first.

banner "2 · registry and treasury"
./scripts/chain_up.sh || die "the chain deployment did not complete"

set -a
# shellcheck disable=SC1091
. ./.env
set +a

# ---------------------------------------------------------------------------
# 3 · Build and start
# ---------------------------------------------------------------------------
#
# `--remove-orphans` is not tidiness. The console service was split into `website` and `admin`, and
# a container left behind under the old name still holds 5173.

banner "3 · building and starting"
note "public origin  $ORIGIN"
"${COMPOSE[@]}" up -d --build --remove-orphans || die "docker compose up failed"
ok "postgres, backend, website, admin, nginx"

# ---------------------------------------------------------------------------
# 4 · The front door
# ---------------------------------------------------------------------------
#
# nginx answers /healthz itself, so this reports the front door being up and not merely the website
# having finished compiling — two different faults, worth telling apart.

banner "4 · waiting for the front door"
printf '  '
DEADLINE=$(( $(date +%s) + 180 ))
until curl -fsS -m 3 "http://localhost:$HTTP_PORT/healthz" >/dev/null 2>&1; do
    if [ "$(date +%s)" -ge "$DEADLINE" ]; then
        printf '\n'
        "${COMPOSE[@]}" ps
        die "the front door did not answer within 180s. Try: make logs"
    fi
    printf '.'
    sleep 2
done
printf '\n'
ok "http://localhost:$HTTP_PORT/healthz"

# The backend is behind the website's proxy rather than routed by nginx, so it gets its own wait:
# the front door is up long before migrations have finished.
printf '  waiting for the backend'
DEADLINE=$(( $(date +%s) + 180 ))
until curl -fsS -m 3 "http://localhost:${BACKEND_PORT:-8088}/ready" >/dev/null 2>&1; do
    if [ "$(date +%s)" -ge "$DEADLINE" ]; then
        printf '\n'
        "${COMPOSE[@]}" logs --tail 40 backend
        die "the backend did not become ready within 180s (log above)"
    fi
    printf '.'
    sleep 2
done
printf '\n'
ok "/ready"

# ---------------------------------------------------------------------------
# 5 · Governance policy
# ---------------------------------------------------------------------------
#
# Idempotent: enrolling an unchanged roster writes no new version, so this is unconditional rather
# than guarded by a query.

banner "5 · governance policy"
( cd backend && ./scripts/enrol_policy.sh ) || die "enrolling the signer roster failed"

# ---------------------------------------------------------------------------
# 6 · Console account
# ---------------------------------------------------------------------------
#
# Seeded only when asked for. The password here is weak and lives in the repository, which is right
# for a demo and wrong for a server — so on a VM the operator creates the account deliberately:
#
#   make admin-user ARGS="create ops@example.com operator"

banner "6 · console account"
if [ "${SEED_DEMO_ADMIN:-0}" = "1" ]; then
    DEMO_ADMIN_EMAIL="${DEMO_ADMIN_EMAIL:-admin@settlement.local}"
    DEMO_ADMIN_PASSWORD="${DEMO_ADMIN_PASSWORD:-123456}"
    warn "seeding $DEMO_ADMIN_EMAIL with a weak, published password — demo only"
    # Inside the container: a server has no reason to have frontend/admin/node_modules, and the
    # host spelling would fail there on a missing `pg` rather than on anything to do with accounts.
    "${COMPOSE[@]}" exec -T \
        -e DATABASE_URL="postgres://settle:settle@postgres:5432/settle" \
        -e ADMIN_PASSWORD="$DEMO_ADMIN_PASSWORD" -e ALLOW_WEAK_PASSWORD=1 \
        admin node scripts/admin-user.mjs create "$DEMO_ADMIN_EMAIL" superadmin >/dev/null 2>&1 \
      || note "(already exists — leaving it alone)"
else
    note "none seeded. Create one:  make admin-user ARGS=\"create you@example.com superadmin\""
    note "or re-run with SEED_DEMO_ADMIN=1 for the published demo account."
fi

# ---------------------------------------------------------------------------
# Where to go
# ---------------------------------------------------------------------------

banner "up"
printf '  %-9s %s\n' "website" "$ORIGIN"
printf '  %-9s %s\n' "admin"   "$ORIGIN/admin"
printf '\n'
note "registry  ${REGISTRY_CONTRACT_ID:-—}"
note "treasury  ${TREASURY_ADDRESS:-—}"
note "postgres, backend, website and admin are published on $BIND_ADDR only — nginx is the way in"
printf '\n'
printf '  %smake demo%s      the on-chain round trip, writing .deploy/evidence.md\n' "$C_B" "$C_0"
printf '  %smake logs%s      follow the backend\n' "$C_B" "$C_0"
printf '  %smake stop%s      stop everything, keeping its data\n' "$C_B" "$C_0"
printf '\n'
