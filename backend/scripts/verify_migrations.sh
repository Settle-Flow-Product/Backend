#!/usr/bin/env bash
#
# Prove every migration reverses.
#
# Applies every *.up.sql in order, asserts the schema exists, applies every *.down.sql in reverse,
# then asserts that NOTHING of ours survives — not a table, not an enum, not a domain, not a
# function. A down migration that forgets an enum type is the classic version of this bug: the
# next `up` fails with "type already exists" and the failure lands on whoever is redeploying under
# pressure, not on whoever wrote it.
#
# Extension-owned objects are excluded from the survivor check: pgcrypto is shared infrastructure,
# not ours to drop.
#
# Usage:  ./scripts/verify_migrations.sh                 (starts a throwaway container)
#         PSQL="docker exec -i mypg psql" ./scripts/verify_migrations.sh   (use an existing one)

set -euo pipefail
cd "$(dirname "$0")/.."

CONTAINER="${CONTAINER:-settle-migrations-verify}"
OWN_CONTAINER=0

if [ -z "${PSQL:-}" ]; then
    command -v docker >/dev/null 2>&1 || { echo "docker is required, or set PSQL"; exit 1; }
    docker rm -f "$CONTAINER" >/dev/null 2>&1 || true
    docker run -d --name "$CONTAINER" \
        -e POSTGRES_USER=settle -e POSTGRES_PASSWORD=settle -e POSTGRES_DB=settle \
        postgres:17-alpine >/dev/null
    OWN_CONTAINER=1
    trap 'docker rm -f "$CONTAINER" >/dev/null 2>&1 || true' EXIT

    # `pg_isready` is not a sufficient signal here.
    #
    # The official image starts a temporary server for initdb, SHUTS IT DOWN, then starts the real
    # one — and the temporary server answers pg_isready over the unix socket. Accepting it means the
    # first statement can land on a database that is on its way down, which fails with "the database
    # system is shutting down". The window is short, so it hits intermittently, which is worse than
    # always: it teaches whoever sees it to re-run until green.
    #
    # The temporary server is started with `listen_addresses=''` and accepts no TCP connection at
    # all, so a query forced over TCP only succeeds against the real one. Twice running, to cover
    # the moment the real server is itself still starting.
    ready=0
    for _ in $(seq 1 90); do
        if docker exec "$CONTAINER" \
             psql -qtAX -h 127.0.0.1 -U settle -d settle -c 'select 1' >/dev/null 2>&1; then
            ready=$((ready + 1))
            [ "$ready" -ge 2 ] && break
        else
            ready=0
        fi
        sleep 1
    done
    [ "$ready" -ge 2 ] || { echo "postgres in $CONTAINER never accepted a query"; exit 1; }
    PSQL="docker exec -i $CONTAINER psql"
fi

run() { $PSQL -v ON_ERROR_STOP=1 -q -U settle -d settle; }
query() { $PSQL -tA -U settle -d settle -c "$1" | tr -d '[:space:]'; }

echo "→ applying every migration, in order"
for f in $(ls migrations/*.up.sql | sort); do
    printf '  %-56s' "$(basename "$f")"
    run < "$f"
    echo "ok"
done

tables=$(query "SELECT count(*) FROM pg_tables WHERE schemaname='public';")
[ "$tables" -gt 0 ] || { echo "the up migrations created no tables"; exit 1; }
echo "  schema present: $tables tables"

echo "→ reversing every migration"
for f in $(ls migrations/*.down.sql | sort -r); do
    printf '  %-56s' "$(basename "$f")"
    run < "$f"
    echo "ok"
done

echo "→ asserting nothing of ours survives"
fail=0
check() {
    local what="$1" count="$2"
    printf '  %-28s %s\n' "$what" "$count"
    [ "$count" = "0" ] || fail=1
}
check "tables"            "$(query "SELECT count(*) FROM pg_tables WHERE schemaname='public';")"
check "enum/domain types" "$(query "SELECT count(*) FROM pg_type WHERE typnamespace='public'::regnamespace AND typtype IN ('e','d');")"
check "our functions"     "$(query "SELECT count(*) FROM pg_proc p WHERE p.pronamespace='public'::regnamespace AND NOT EXISTS (SELECT 1 FROM pg_depend d WHERE d.objid=p.oid AND d.deptype='e');")"
check "sequences"         "$(query "SELECT count(*) FROM pg_sequences WHERE schemaname='public';")"

if [ "$fail" -ne 0 ]; then
    echo
    echo "A down migration is incomplete. The next deploy will fail with 'already exists'."
    exit 1
fi

echo
echo "migrations reverse cleanly"
[ "$OWN_CONTAINER" = 1 ] || echo "(reused an existing database; it is now empty)"
