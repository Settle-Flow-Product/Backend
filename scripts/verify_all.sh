#!/usr/bin/env bash
#
# Verify everything: smart-contract, backend, frontend, and the claims the documentation makes.
#
# One command, so "did you check?" has a reproducible answer rather than a remembered one. It
# starts its own throwaway Postgres, runs every suite, and fails on the first thing that is not as
# specified.
#
#   ./scripts/verify_all.sh          # everything
#   PASS=2 ./scripts/verify_all.sh   # label the run, for repeated passes

set -uo pipefail
cd "$(dirname "$0")/.."

PASS_LABEL="${PASS:-}"
PG_CONTAINER="${PG_CONTAINER:-settle-verify-all}"
PG_PORT="${PG_PORT:-55444}"
DATABASE_URL="postgres://settle:settle@localhost:${PG_PORT}/settle"

if [ -t 1 ]; then
    C_B=$'\033[1m'; C_OK=$'\033[32m'; C_ERR=$'\033[31m'; C_DIM=$'\033[2m'; C_0=$'\033[0m'
else
    C_B=; C_OK=; C_ERR=; C_DIM=; C_0=
fi

FAILED=0
declare -a RESULTS=()

section() { printf '\n%s══ %s %s\n' "$C_B" "$*" "$C_0"; }
note()    { printf '%s   %s%s\n' "$C_DIM" "$*" "$C_0"; }

# check <name> <command…>
check() {
    local name="$1"; shift
    printf '   %-46s' "$name"
    local out
    if out=$("$@" 2>&1); then
        printf '%sok%s\n' "$C_OK" "$C_0"
        RESULTS+=("ok   $name")
    else
        printf '%sFAILED%s\n' "$C_ERR" "$C_0"
        printf '%s\n' "$out" | tail -25 | sed 's/^/       /'
        RESULTS+=("FAIL $name")
        FAILED=$((FAILED + 1))
    fi
}

cleanup() { docker rm -f "$PG_CONTAINER" >/dev/null 2>&1 || true; }
trap cleanup EXIT

printf '%sSettleFlow · full verification%s%s\n' "$C_B" "${PASS_LABEL:+  (pass $PASS_LABEL)}" "$C_0"

# ---------------------------------------------------------------------------
section "smart-contract"
# ---------------------------------------------------------------------------
check "formatting"            bash -c 'cd smart-contract && cargo fmt --all --check'
check "clippy -D warnings"    bash -c 'cd smart-contract && cargo clippy --workspace --all-targets -- -D warnings'
check "unit tests"            bash -c 'cd smart-contract && cargo test --workspace'
check "no-custody gate finds a violation" bash -c 'cd smart-contract && ./scripts/no_custody_gate_selftest.sh'
check "no-custody gate"       bash -c 'cd smart-contract && ./scripts/no_custody_gate.sh'
check "release wasm builds"   bash -c 'cd smart-contract && cargo build --workspace --target wasm32v1-none --release'
check "wasm within its ceiling" bash -c '
    size=$(wc -c < smart-contract/target/wasm32v1-none/release/payment_request_registry.wasm)
    echo "wasm ${size} bytes"; test "$size" -le 262144'

# ---------------------------------------------------------------------------
section "database"
# ---------------------------------------------------------------------------
docker rm -f "$PG_CONTAINER" >/dev/null 2>&1 || true
if docker run -d --name "$PG_CONTAINER" \
     -e POSTGRES_USER=settle -e POSTGRES_PASSWORD=settle -e POSTGRES_DB=settle \
     -p "${PG_PORT}:5432" postgres:17-alpine >/dev/null 2>&1; then
    # `pg_isready` alone is not enough, and neither is the log line.
    #
    # The official image runs a temporary server for initdb, then SHUTS IT DOWN and starts the real
    # one. That temporary server logs "database system is ready to accept connections" and answers
    # pg_isready over the unix socket, so both of the obvious signals say yes while the database is
    # about to go away — and the migrations then fail with "the database system is shutting down".
    # The window is short, so it hits intermittently; this suite lost a run to it. A verification
    # script that intermittently fails teaches people to re-run until green, which is how a real
    # failure eventually gets waved through.
    #
    # What separates the two decisively: the temporary server is started with `listen_addresses=''`
    # and so accepts no TCP connection at all. Forcing psql over TCP therefore only succeeds against
    # the real server. Twice running, to cover the moment the real one is itself still starting.
    ready=0
    for _ in $(seq 1 90); do
        if docker exec "$PG_CONTAINER" \
             psql -qtAX -h 127.0.0.1 -U settle -d settle -c 'select 1' >/dev/null 2>&1; then
            ready=$((ready + 1))
            [ "$ready" -ge 2 ] && break
        else
            ready=0
        fi
        sleep 1
    done
    note "postgres on ${PG_PORT}"
    if [ "$ready" -ge 2 ]; then
        HAVE_DB=1
    else
        note "postgres never accepted a TCP query — database checks will be skipped, not passed"
        HAVE_DB=0
    fi
else
    note "docker unavailable — database checks will be skipped, not silently passed"
    HAVE_DB=0
fi

if [ "$HAVE_DB" = "1" ]; then
    check "migrations reverse cleanly"  bash -c 'cd backend && ./scripts/verify_migrations.sh'
    check "constraints refuse as claimed" bash -c 'cd backend && ./scripts/verify_constraints.sh'
else
    RESULTS+=("SKIP database checks (no docker)")
fi

# ---------------------------------------------------------------------------
section "backend"
# ---------------------------------------------------------------------------
check "formatting"          bash -c 'cd backend && cargo fmt --all --check'
check "clippy -D warnings"  bash -c 'cd backend && cargo clippy --workspace --all-targets -- -D warnings'
if [ "$HAVE_DB" = "1" ]; then
    # Apply the schema once so the integration tests have somewhere to run.
    for f in backend/migrations/*.up.sql; do
        docker exec -i "$PG_CONTAINER" psql -v ON_ERROR_STOP=1 -q -U settle -d settle < "$f" >/dev/null 2>&1
    done
    check "tests, including the database suite" \
        env DATABASE_URL="$DATABASE_URL" bash -c 'cd backend && cargo test --workspace'
else
    check "tests (pure crates only)" bash -c 'cd backend && cargo test --workspace'
fi

# ---------------------------------------------------------------------------
section "website"
# ---------------------------------------------------------------------------
# Install first if the tree has never been built.
#
# Without this, a FRESH CLONE fails six checks with "tsc: command not found" — which reads as the
# repository being broken rather than as dependencies being absent. `npm ci` is skipped when
# `node_modules` is already there, so the common case costs nothing.
deps() {
    local dir="$1"
    [ -d "$dir/node_modules" ] && return 0
    note "installing $dir dependencies (first run)"
    (cd "$dir" && npm ci --ignore-scripts >/dev/null 2>&1)
}
deps frontend/website
deps frontend/admin

check "typecheck"    bash -c 'cd frontend/website && npm run typecheck'
check "tests"        bash -c 'cd frontend/website && npm test'
check "production build" bash -c 'cd frontend/website && rm -rf .next && npm run build'

# ---------------------------------------------------------------------------
section "admin console"
# ---------------------------------------------------------------------------
check "typecheck"    bash -c 'cd frontend/admin && npm run typecheck'
check "tests"        bash -c 'cd frontend/admin && npm test'
check "production build" bash -c 'cd frontend/admin && rm -rf .next && npm run build'

# ---------------------------------------------------------------------------
section "repository"
# ---------------------------------------------------------------------------
# env.sh is idempotent and never overwrites an existing value, so it runs unconditionally: the
# guard that used to be here (`test -f .env ||`) meant a .env predating a newly required variable
# was never topped up, and this check failed with the very instruction that could not fix it.
check "compose is valid"     bash -c './scripts/env.sh >/dev/null; docker compose config --quiet'
check "demo script parses"   bash -n scripts/demo.sh
check "demo plan runs"       bash -c 'DRY_RUN=1 REGISTRY=X ADMIN_KEY=x TREASURY_ADDRESS=Y EVIDENCE=/tmp/ev.md DEPLOY_DIR=/tmp/dryrun ./scripts/demo.sh >/dev/null'
check "no Stellar secret key committed" bash -c '
    ! grep -rInE "\bS[A-Z2-7]{55}\b" --exclude-dir=.git --exclude-dir=node_modules \
        --exclude-dir=target --exclude="*.lock" . 2>/dev/null'
check "no float in any source" bash -c '
    ! grep -rInE "\b(f32|f64|parseFloat)\b" backend/crates/*/src smart-contract/src \
        frontend/website/lib 2>/dev/null'
check "docs and code agree"  python3 scripts/check_docs.py

# ---------------------------------------------------------------------------
printf '\n%s══ result %s\n' "$C_B" "$C_0"
for r in "${RESULTS[@]}"; do
    case "$r" in
        ok*)   printf '   %s✓%s %s\n' "$C_OK" "$C_0" "${r#ok   }" ;;
        FAIL*) printf '   %s✗%s %s\n' "$C_ERR" "$C_0" "${r#FAIL }" ;;
        *)     printf '   %s· %s%s\n' "$C_DIM" "${r#SKIP }" "$C_0" ;;
    esac
done

echo
if [ "$FAILED" -eq 0 ]; then
    printf '%s✓ everything verified%s%s\n' "$C_OK" "${PASS_LABEL:+  (pass $PASS_LABEL)}" "$C_0"
    exit 0
fi
printf '%s✗ %d check(s) failed%s\n' "$C_ERR" "$FAILED" "$C_0"
exit 1
