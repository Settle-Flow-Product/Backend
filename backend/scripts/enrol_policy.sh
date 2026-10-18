#!/usr/bin/env bash
#
# Module B4 — enrol the treasury's signer roster as a governance policy.
#
# This is the step that answers "who may approve this payment". Without it `prepare` has no policy to
# pin, returns 500, and the approver screen shows an INTERNAL error instead of a signature roster —
# because the roster does not exist.
#
# ## The roster is READ FROM THE CHAIN, never typed
#
# The signers and the threshold come from Horizon, not from an argument. Two rosters that disagree —
# one on the account, one in Postgres — is the worst outcome available here: the backend would accept
# a signature the network will refuse, or refuse one it would have accepted, and the mismatch only
# shows up at submit. Deriving the policy from the account makes the two incapable of drifting.
#
# The master key is excluded. It has weight 0 by construction, and a zero-weight signer violates the
# `weight BETWEEN 1 AND 255` check on policy_signers — correctly, since a key that cannot contribute
# to a threshold is not an approver.
#
# ## Versions, not edits
#
# A change writes a NEW version and deactivates the old one. The previous row stays, because a
# settlement pins the version it was authorised under, and "under which rule was this payment
# approved" must still have an answer months later.
#
#   ./scripts/enrol_policy.sh

set -euo pipefail
cd "$(dirname "$0")/.."

HORIZON_URL="${HORIZON_URL:-https://horizon-testnet.stellar.org}"
DEPLOY_DIR="${DEPLOY_DIR:-../.deploy}"

if [ -t 1 ]; then C_B=$'\033[1m'; C_OK=$'\033[32m'; C_ERR=$'\033[31m'; C_DIM=$'\033[2m'; C_0=$'\033[0m'
else C_B=; C_OK=; C_ERR=; C_DIM=; C_0=; fi
step() { printf '%s→%s %s\n' "$C_B" "$C_0" "$*"; }
ok()   { printf '%s✓%s %s\n' "$C_OK" "$C_0" "$*"; }
note() { printf '%s  %s%s\n' "$C_DIM" "$*" "$C_0"; }
die()  { printf '%s✗%s %s\n' "$C_ERR" "$C_0" "$*" >&2; exit 1; }

command -v jq >/dev/null || die "jq is not installed (brew install jq)"

if [ -f "$DEPLOY_DIR/treasury.env" ]; then
    # shellcheck disable=SC1091
    . "$DEPLOY_DIR/treasury.env"
fi
TREASURY="${TREASURY_ADDRESS:-}"
[ -n "$TREASURY" ] || die "TREASURY_ADDRESS is not set. Run 'make treasury' first."

# Same resolution as verify_constraints.sh, so there is one way to reach the database.
if [ -z "${PSQL:-}" ]; then
    command -v docker >/dev/null 2>&1 || die "docker is required, or set PSQL"

    # ASKED FOR, not guessed. Compose derives its project name from the directory, so a container
    # called `settlement-postgres-1` in one checkout is `monorepo-postgres-1` in another — and a
    # hardcoded name turns "the repository was moved" into "the database is not running".
    CONTAINER="${PG_CONTAINER:-$(cd .. && docker compose ps -q postgres 2>/dev/null | head -1)}"
    [ -n "$CONTAINER" ] \
        || die "no running postgres container for this project. Start it with 'make up', or set PSQL."
    docker inspect "$CONTAINER" >/dev/null 2>&1 \
        || die "container $CONTAINER is not running. Start it with 'make up', or set PSQL."
    PSQL="docker exec -i $CONTAINER psql"
fi
run() { $PSQL -v ON_ERROR_STOP=1 -qtAX -U settle -d settle; }

step "Reading the signer roster from the chain"
ACCOUNT=$(curl -sf "$HORIZON_URL/accounts/$TREASURY") \
    || die "could not read $TREASURY from Horizon"

# Payment is a MEDIUM-threshold operation, so med_threshold is the one that actually gates a payout.
THRESHOLD=$(jq -r '.thresholds.med_threshold' <<<"$ACCOUNT")
SIGNERS=$(jq -r --arg a "$TREASURY" \
    '[.signers[] | select(.key != $a and .weight > 0) | "\(.key) \(.weight)"] | .[]' <<<"$ACCOUNT")

[ -n "$SIGNERS" ] || die "no signers with weight above zero on $TREASURY"

TOTAL=$(awk '{s += $2} END {print s + 0}' <<<"$SIGNERS")
COUNT=$(wc -l <<<"$SIGNERS" | tr -d ' ')

note "treasury  $TREASURY"
note "threshold $THRESHOLD (med) · $COUNT signers · total weight $TOTAL"
while read -r key weight; do note "signer    $key  weight $weight"; done <<<"$SIGNERS"

# The same lock-out guard as B3, applied before writing rather than discovered after. The database
# trigger refuses this too; checking here means the message names the problem.
[ "$TOTAL" -ge "$THRESHOLD" ] \
    || die "signer weight $TOTAL is below the threshold $THRESHOLD — this policy could never approve anything"

# Already correct? Then do nothing. Re-running must not churn version numbers, because every bump
# invalidates nothing but makes the audit trail harder to read.
CURRENT=$(run <<SQL
SELECT COALESCE(
  (SELECT p.version || ':' || p.threshold || ':' ||
          COALESCE((SELECT string_agg(s.signer_address || '=' || s.weight, ',' ORDER BY s.signer_address)
                      FROM policy_signers s WHERE s.policy_id = p.id), '')
     FROM governance_policies p
    WHERE p.treasury_address = '$TREASURY' AND p.active), '');
SQL
)
WANT_SIGNERS=$(while read -r key weight; do printf '%s=%s\n' "$key" "$weight"; done <<<"$SIGNERS" | sort | paste -sd, -)
if [ -n "$CURRENT" ] && [ "${CURRENT#*:}" = "$THRESHOLD:$WANT_SIGNERS" ]; then
    ok "the active policy (version ${CURRENT%%:*}) already matches the chain — nothing to do"
    exit 0
fi

step "Writing a new policy version"
VALUES=$(while read -r key weight; do printf "(pid, '%s', %s, 'approver')," "$key" "$weight"; done <<<"$SIGNERS" | sed '$ s/,$//')

# One transaction. A policy that is half written is a roster nobody can trust.
#
# Order is forced by the trigger: it checks that the signers reach the threshold, so the row cannot
# be inserted active before its signers exist. Insert inactive, add signers, then activate.
run <<SQL
BEGIN;

DO \$\$
DECLARE
    pid uuid;
    next_version integer;
BEGIN
    SELECT COALESCE(MAX(version), 0) + 1 INTO next_version
      FROM governance_policies WHERE treasury_address = '$TREASURY';

    UPDATE governance_policies SET active = false
     WHERE treasury_address = '$TREASURY' AND active;

    INSERT INTO governance_policies (treasury_address, version, threshold, active)
    VALUES ('$TREASURY', next_version, $THRESHOLD, false)
    RETURNING id INTO pid;

    INSERT INTO policy_signers (policy_id, signer_address, weight, role) VALUES
    $VALUES;

    UPDATE governance_policies SET active = true WHERE id = pid;

    RAISE NOTICE 'policy version % active: % of %', next_version, $THRESHOLD, $TOTAL;
END \$\$;

COMMIT;
SQL

ACTIVE=$(run <<SQL
SELECT version FROM governance_policies WHERE treasury_address = '$TREASURY' AND active;
SQL
)
echo
ok "policy version $ACTIVE is active — $THRESHOLD of $TOTAL"
note "the approver screen can now show who may sign"
