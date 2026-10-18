#!/usr/bin/env bash
#
# Modules B1 and B2 — create the treasury and configure its multisig.
#
# Three signers at weight 1, every threshold at 2, and masterWeight = 0.
#
# That last step is the one most often forgotten and the first thing a reviewer checks. Without it
# whoever holds the treasury's secret key can sign alone, and the entire approval process is
# decoration. It is done here by script and asserted again by verify_treasury.sh, never by hand.
#
# Re-runnable from nothing in under ten minutes. Testnet resets, and when it does everything here
# goes with it.
#
#   ./scripts/setup_treasury.sh

set -euo pipefail
cd "$(dirname "$0")/.."

NETWORK="${NETWORK:-testnet}"
RPC_URL="${RPC_URL:-https://soroban-testnet.stellar.org}"
HORIZON_URL="${HORIZON_URL:-https://horizon-testnet.stellar.org}"
DEPLOY_DIR="${DEPLOY_DIR:-../.deploy}"

TREASURY_KEY="${TREASURY_KEY:-settle-treasury}"

# The account that CREATES payment requests.
#
# A SEPARATE account from the treasury, and that separation is load-bearing. `create_request` calls
# `creator.require_auth()`, and the treasury cannot authorise anything on its own — masterWeight 0 is
# the entire point of it. Using the treasury as the creator makes recording a request need two of
# three approvals, which is not what recording a request means.
BUSINESS_KEY="${BUSINESS_KEY:-settle-business}"

SIGNER_KEYS=("${SIGNER1_KEY:-settle-signer-1}" "${SIGNER2_KEY:-settle-signer-2}" "${SIGNER3_KEY:-settle-signer-3}")
THRESHOLD="${THRESHOLD:-2}"

# The USDC issuer.
#
# Left unset, this script generates and funds its OWN issuer and issues the treasury a working
# balance — so a USDC demo runs unattended and survives a testnet reset, which is the whole point of
# "re-runnable from nothing in under ten minutes". Circle's testnet USDC would mean a manual visit to
# a browser faucet, and a deploy step nobody can script is a deploy step that fails at the worst
# moment.
#
# Set USDC_ISSUER to a real issuer address to use that asset instead; the trustline is then
# established against it and no issuance is attempted, because we cannot issue somebody else's asset.
USDC_ISSUER_KEY="${USDC_ISSUER_KEY:-settle-usdc-issuer}"
USDC_ISSUE_UNITS="${USDC_ISSUE_UNITS:-1000000}"

if [ -t 1 ]; then C_B=$'\033[1m'; C_OK=$'\033[32m'; C_ERR=$'\033[31m'; C_DIM=$'\033[2m'; C_0=$'\033[0m'
else C_B=; C_OK=; C_ERR=; C_DIM=; C_0=; fi
step() { printf '%s→%s %s\n' "$C_B" "$C_0" "$*"; }
ok()   { printf '%s✓%s %s\n' "$C_OK" "$C_0" "$*"; }
note() { printf '%s  %s%s\n' "$C_DIM" "$*" "$C_0"; }
die()  { printf '%s✗%s %s\n' "$C_ERR" "$C_0" "$*" >&2; exit 1; }

command -v stellar >/dev/null || die "the Stellar CLI is not installed"
command -v jq >/dev/null      || die "jq is not installed (brew install jq)"
mkdir -p "$DEPLOY_DIR"

# Prints the address on stdout and NOTHING ELSE, because every caller captures it with `$(…)`.
#
# The progress line goes to stderr for that reason. On stdout it was captured along with the
# address, and the caller ended up holding "→ creating and funding settle-usdc-issuer GAXU…" as
# though it were an account — which only happens for a key that does not exist yet, so it hid until
# the first genuinely fresh run.
ensure_key() {
    local name="$1"
    if ! stellar keys address "$name" >/dev/null 2>&1; then
        step "creating and funding $name" >&2
        stellar keys generate --fund --network "$NETWORK" "$name" >/dev/null 2>&1
    fi
    stellar keys address "$name"
}

step "Treasury and signers"
TREASURY=$(ensure_key "$TREASURY_KEY")
note "treasury  $TREASURY"

BUSINESS=$(ensure_key "$BUSINESS_KEY")
note "business  $BUSINESS  ($BUSINESS_KEY)"

SIGNERS=()
for k in "${SIGNER_KEYS[@]}"; do
    a=$(ensure_key "$k")
    SIGNERS+=("$a")
    note "signer    $a  ($k)"
done

# ---------------------------------------------------------------------------
# The lock-out guard — module B3
# ---------------------------------------------------------------------------
#
# Setting a threshold above the reachable signer weight locks the account PERMANENTLY: setOptions
# is itself a high-threshold operation, so once the high threshold is unreachable nobody can ever
# change the configuration back. There is no recovery path.
#
# The predicate, evaluated against the NEW values:
#
#     Σ weight(active signers) + masterWeight  ≥  max(low, med, high)
#
# masterWeight is about to become 0, so it contributes nothing.
TOTAL_WEIGHT=${#SIGNERS[@]}
step "Lock-out guard"
if [ "$THRESHOLD" -gt "$TOTAL_WEIGHT" ]; then
    die "refusing: threshold $THRESHOLD exceeds the total signer weight $TOTAL_WEIGHT. This would lock the account permanently."
fi
if [ "$THRESHOLD" -eq "$TOTAL_WEIGHT" ]; then
    die "refusing: threshold $THRESHOLD equals the total weight $TOTAL_WEIGHT. Losing one key would end the ability to pay. SOW 1 uses 2 of 3 precisely so one lost key is survivable."
fi
ok "threshold $THRESHOLD of $TOTAL_WEIGHT is reachable and survives losing one key"

# ---------------------------------------------------------------------------
# The trustline — BEFORE the master key is disabled
# ---------------------------------------------------------------------------
#
# Sending USDC to an account without a USDC trustline fails at the ledger, and it fails AT SUBMIT —
# after both approvers have already signed.
#
# **Order is the whole correctness argument here.** `changeTrust` is a medium-threshold operation,
# so once masterWeight is 0 and the thresholds are 2, the treasury key can no longer authorise one
# alone. This step used to run LAST, after that had already happened, where it could not succeed —
# and its failure was swallowed by an `|| note`, so the script reported "treasury can receive USDC"
# over a trustline that was never created. That is why the deployed treasury had none.
#
# Establishing it here, while the treasury is still a plain single-key account, is what makes it
# work at all. Nothing below this point is authorised by the treasury key alone.

if [ -z "${USDC_ISSUER:-}" ]; then
    step "USDC issuer"
    USDC_ISSUER=$(ensure_key "$USDC_ISSUER_KEY")
    ISSUER_IS_OURS=1
    note "issuer    $USDC_ISSUER  ($USDC_ISSUER_KEY)"
    note "self-issued test USDC — set USDC_ISSUER to use somebody else's asset"
else
    ISSUER_IS_OURS=0
    note "using the configured USDC issuer $USDC_ISSUER"
fi

# How the treasury is authorised right now decides how this transaction gets signed. A treasury that
# has never been configured still has a weight-1 master key and signs for itself; one that has
# already been through this script does not, and needs two of its three signers — the same 2-of-3 a
# payment needs. Reading it rather than assuming is what lets this script run against BOTH, so a
# trustline can still be added to a treasury that is already locked down.
MASTER_WEIGHT=$(curl -sf "$HORIZON_URL/accounts/$TREASURY" \
    | jq -r --arg a "$TREASURY" '[.signers[] | select(.key == $a) | .weight] | first // 1' \
    2>/dev/null || echo 1)

step "Establishing the USDC trustline"
# No `|| note`: a trustline that failed to establish must stop the script here, not surface as a
# failed payment in front of a reviewer. An already-existing trustline is a success, not an error —
# changeTrust to the same limit is a no-op the network accepts.
if [ "$MASTER_WEIGHT" != "0" ]; then
    if ! stellar tx new change-trust \
            --source "$TREASURY_KEY" --network "$NETWORK" \
            --line "USDC:$USDC_ISSUER" >/dev/null 2>/tmp/trustline_err; then
        die "could not establish the USDC trustline: $(tail -3 /tmp/trustline_err | tr '\n' ' ')"
    fi
else
    note "master key has weight 0 — signing the trustline with $THRESHOLD of ${#SIGNER_KEYS[@]}"
    TRUST_XDR=$(stellar tx new change-trust --build-only \
        --source-account "$TREASURY" --network "$NETWORK" \
        --line "USDC:$USDC_ISSUER" 2>/tmp/trustline_err | tail -1 || true)
    [ -n "$TRUST_XDR" ] || die "could not build the trustline transaction: $(tail -3 /tmp/trustline_err | tr '\n' ' ')"

    for ((i = 0; i < THRESHOLD; i++)); do
        SIGNED=$(stellar tx sign "$TRUST_XDR" \
            --sign-with-key "${SIGNER_KEYS[$i]}" --network "$NETWORK" 2>/tmp/trustline_err | tail -1 || true)
        [ -n "$SIGNED" ] || die "${SIGNER_KEYS[$i]} could not sign: $(tail -3 /tmp/trustline_err | tr '\n' ' ')"
        TRUST_XDR="$SIGNED"
        note "signed by ${SIGNER_KEYS[$i]}"
    done

    SEND_OUT=$(stellar tx send "$TRUST_XDR" --network "$NETWORK" 2>/tmp/trustline_err || true)
    if [ "$(jq -r '.status // empty' <<<"$SEND_OUT" 2>/dev/null)" != "SUCCESS" ]; then
        die "the trustline transaction was refused: $(tail -3 /tmp/trustline_err | tr '\n' ' ')"
    fi
fi
ok "treasury can receive USDC"

if [ "$ISSUER_IS_OURS" = "1" ]; then
    step "Issuing $USDC_ISSUE_UNITS USDC to the treasury"
    # Issuance IS a payment from the issuing account: the asset comes into existence as it is sent.
    # `--amount` is in stroops, so units are scaled by 10^7 — the same seven decimals the contract
    # stores, and never a float.
    ISSUE_STROOPS=$((USDC_ISSUE_UNITS * 10000000))
    if ! stellar tx new payment \
            --source "$USDC_ISSUER_KEY" --network "$NETWORK" \
            --destination "$TREASURY" --asset "USDC:$USDC_ISSUER" \
            --amount "$ISSUE_STROOPS" >/dev/null 2>/tmp/issue_err; then
        die "could not issue USDC to the treasury: $(tail -3 /tmp/issue_err | tr '\n' ' ')"
    fi
    ok "treasury holds $USDC_ISSUE_UNITS USDC"
fi

# ---------------------------------------------------------------------------
# The Stellar Asset Contract
# ---------------------------------------------------------------------------
#
# The registry's `asset` field is an ADDRESS — the asset's Stellar Asset Contract, not its
# `CODE:ISSUER` string. Without this the demo fell back to `native` and recorded a request for XLM
# while the treasury paid USDC: an on-chain record that disagrees with the on-chain payment it
# describes, which is the exact class of defect the readback discipline exists to catch.
#
# The id is derived deterministically from the asset, so this is idempotent — deploying an already
# deployed SAC is not an error worth stopping for.
step "Stellar Asset Contract for USDC"
USDC_CONTRACT=$(stellar contract id asset \
    --asset "USDC:$USDC_ISSUER" --network "$NETWORK" 2>/dev/null | tail -1 || true)
[ -n "$USDC_CONTRACT" ] || die "could not derive the USDC asset contract id"

# Deployed only for an issuer we control. Somebody else's asset already has its SAC on the network —
# Circle's does — and we have no key to deploy it with anyway. The id above is derived either way,
# which is the part the registry needs.
if [ "$ISSUER_IS_OURS" = "1" ]; then
    if ! stellar contract asset deploy \
            --asset "USDC:$USDC_ISSUER" --source-account "$USDC_ISSUER_KEY" \
            --network "$NETWORK" >/dev/null 2>/tmp/sac_err; then
        # The only expected failure is "already deployed", which is a success for our purposes. Any
        # other cause surfaces immediately in the demo, where create_request cannot resolve the id.
        note "asset contract not newly deployed (already exists, or see /tmp/sac_err)"
    fi
fi
ok "USDC asset contract $USDC_CONTRACT"

# Both steps below are authorised by the treasury's master key, so they only apply to a treasury
# that still has one. Re-running against a configured treasury skips them rather than failing:
# setOptions is a HIGH-threshold operation, and the master key it would need is exactly the one this
# script already set to zero.
if [ "$MASTER_WEIGHT" != "0" ]; then
    step "Adding signers at weight 1"
    for a in "${SIGNERS[@]}"; do
        stellar tx new set-options \
            --source "$TREASURY_KEY" --network "$NETWORK" \
            --signer "$a" --signer-weight 1 >/dev/null
        note "added $a"
    done

    # masterWeight LAST. Adding signers first means the account is never in a state where it has a
    # zero-weight master key and no other signer — which would be the locked account this whole guard
    # exists to prevent.
    step "Setting thresholds to $THRESHOLD and masterWeight to 0"
    stellar tx new set-options \
        --source "$TREASURY_KEY" --network "$NETWORK" \
        --master-weight 0 \
        --low-threshold "$THRESHOLD" \
        --med-threshold "$THRESHOLD" \
        --high-threshold "$THRESHOLD" >/dev/null
    ok "master key disabled; payment now needs $THRESHOLD of $TOTAL_WEIGHT"
else
    step "Multisig configuration"
    ok "already configured — master key has weight 0, leaving it alone"
fi

{
    echo "TREASURY_ADDRESS=$TREASURY"
    echo "TREASURY_KEY=$TREASURY_KEY"
    echo "BUSINESS_ADDRESS=$BUSINESS"
    echo "BUSINESS_KEY=$BUSINESS_KEY"
    for i in "${!SIGNERS[@]}"; do
        echo "SIGNER$((i + 1))_ADDRESS=${SIGNERS[$i]}"
        echo "SIGNER$((i + 1))_KEY=${SIGNER_KEYS[$i]}"
    done
    echo "THRESHOLD=$THRESHOLD"
    echo "USDC_ISSUER=$USDC_ISSUER"
    echo "USDC_CONTRACT=$USDC_CONTRACT"
    # An `if`, not `[ … ] && echo`: a false test makes the && the last command in this group, the
    # group exits non-zero, and `set -e` kills the script one line before it reports success.
    if [ "$ISSUER_IS_OURS" = "1" ]; then
        echo "USDC_ISSUER_KEY=$USDC_ISSUER_KEY"
    fi
} > "$DEPLOY_DIR/treasury.env"

echo
ok "Treasury configured"
note "addresses written to $DEPLOY_DIR/treasury.env"
note "verify with ./scripts/verify_treasury.sh"
