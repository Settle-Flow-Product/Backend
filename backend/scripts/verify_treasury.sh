#!/usr/bin/env bash
#
# Module B5 — read the treasury back from Horizon and ASSERT its configuration.
#
# It asserts rather than prints. A script that displays the numbers and leaves a human to check
# them is a script that passes on demo day while the master key still has weight 1.
#
# Its output is evidence milestone E2. See ../../docs/14-evidence.md.

set -euo pipefail
cd "$(dirname "$0")/.."

NETWORK="${NETWORK:-testnet}"
HORIZON_URL="${HORIZON_URL:-https://horizon-testnet.stellar.org}"
DEPLOY_DIR="${DEPLOY_DIR:-../.deploy}"
EXPECTED_SIGNERS="${EXPECTED_SIGNERS:-3}"
EXPECTED_THRESHOLD="${EXPECTED_THRESHOLD:-2}"

if [ -t 1 ]; then C_B=$'\033[1m'; C_OK=$'\033[32m'; C_ERR=$'\033[31m'; C_DIM=$'\033[2m'; C_0=$'\033[0m'
else C_B=; C_OK=; C_ERR=; C_DIM=; C_0=; fi
ok()   { printf '%s✓%s %s\n' "$C_OK" "$C_0" "$*"; }
bad()  { printf '%s✗%s %s\n' "$C_ERR" "$C_0" "$*"; FAILED=1; }
note() { printf '%s  %s%s\n' "$C_DIM" "$*" "$C_0"; }
die()  { printf '%s✗%s %s\n' "$C_ERR" "$C_0" "$*" >&2; exit 1; }

command -v jq >/dev/null || die "jq is not installed"

TREASURY="${TREASURY_ADDRESS:-}"
if [ -z "$TREASURY" ] && [ -f "$DEPLOY_DIR/treasury.env" ]; then
    # shellcheck disable=SC1090
    . "$DEPLOY_DIR/treasury.env"
    TREASURY="$TREASURY_ADDRESS"
fi
[ -n "$TREASURY" ] || die "TREASURY_ADDRESS is not set and $DEPLOY_DIR/treasury.env does not exist"

printf '%sTreasury configuration%s  %s\n' "$C_B" "$C_0" "$TREASURY"
echo

ACCOUNT=$(curl -sf "$HORIZON_URL/accounts/$TREASURY") \
    || die "Horizon does not know account $TREASURY on $NETWORK"

FAILED=0

MASTER_WEIGHT=$(jq -r --arg a "$TREASURY" '[.signers[] | select(.key == $a) | .weight] | first // 0' <<<"$ACCOUNT")
OTHER_COUNT=$(jq -r --arg a "$TREASURY" '[.signers[] | select(.key != $a)] | length' <<<"$ACCOUNT")
LOW=$(jq -r '.thresholds.low_threshold'  <<<"$ACCOUNT")
MED=$(jq -r '.thresholds.med_threshold'  <<<"$ACCOUNT")
HIGH=$(jq -r '.thresholds.high_threshold' <<<"$ACCOUNT")
TOTAL=$(jq -r --arg a "$TREASURY" '[.signers[] | select(.key != $a) | .weight] | add // 0' <<<"$ACCOUNT")

# The first thing a reviewer checks.
if [ "$MASTER_WEIGHT" = "0" ]; then
    ok "masterWeight is 0 — the master key cannot sign alone"
else
    bad "masterWeight is $MASTER_WEIGHT — whoever holds the treasury secret can pay ALONE. The approval process is decoration."
fi

if [ "$OTHER_COUNT" = "$EXPECTED_SIGNERS" ]; then
    ok "$OTHER_COUNT signers besides the master key"
else
    bad "expected $EXPECTED_SIGNERS signers, found $OTHER_COUNT"
fi

for pair in "low:$LOW" "med:$MED" "high:$HIGH"; do
    name="${pair%%:*}"; value="${pair##*:}"
    if [ "$value" = "$EXPECTED_THRESHOLD" ]; then
        ok "${name}_threshold is $value"
    else
        bad "${name}_threshold is $value, expected $EXPECTED_THRESHOLD"
    fi
done

# Payment is a MEDIUM operation, so med_threshold is the one that actually gates a payout.
if [ "$MED" -gt "$TOTAL" ] 2>/dev/null; then
    bad "med_threshold $MED exceeds the total signer weight $TOTAL — no payment can ever be made"
fi
if [ "$HIGH" -gt "$TOTAL" ] 2>/dev/null; then
    bad "high_threshold $HIGH exceeds the total signer weight $TOTAL — the account is LOCKED, its configuration can never be changed again"
fi
if [ "$HIGH" = "$TOTAL" ] 2>/dev/null; then
    bad "high_threshold equals the total weight — losing one key locks administration permanently"
fi

echo
jq -r --arg a "$TREASURY" '
    "signers:", (.signers[] | "  \(.key)  weight \(.weight)" +
        (if .key == $a then "   ← master key" else "" end))
' <<<"$ACCOUNT"

# The trustline requirement follows the ASSET being demonstrated, not the calendar.
#
# XLM is native and needs no trustline; USDC is issued and does. Failing unconditionally would
# refuse a perfectly good XLM treasury, and passing unconditionally would let a USDC demo reach the
# ledger and fail there — after both approvers have already signed. So it is checked when, and only
# when, it matters.
ASSET="${ASSET:-native}"

# Matched on the ISSUER as well as the code when we know which issuer to expect. "USDC" is a ticker,
# not an identity: any account may issue an asset by that name, and a trustline to the wrong one
# passes a code-only check while paying the vendor in something that merely looks like dollars.
if [ -n "${USDC_ISSUER:-}" ]; then
    TRUSTLINE=$(jq -r --arg i "$USDC_ISSUER" \
        '[.balances[] | select(.asset_code == "USDC" and .asset_issuer == $i)] | length' <<<"$ACCOUNT")
    USDC_BALANCE=$(jq -r --arg i "$USDC_ISSUER" \
        '[.balances[] | select(.asset_code == "USDC" and .asset_issuer == $i) | .balance] | first // "0"' <<<"$ACCOUNT")
else
    TRUSTLINE=$(jq -r '[.balances[] | select(.asset_code == "USDC")] | length' <<<"$ACCOUNT")
    USDC_BALANCE=$(jq -r '[.balances[] | select(.asset_code == "USDC") | .balance] | first // "0"' <<<"$ACCOUNT")
fi

echo
if [ "$ASSET" = "native" ] && [ -z "${USDC_ISSUER:-}" ]; then
    if [ "$TRUSTLINE" -gt 0 ]; then
        ok "USDC trustline present (not required — this treasury is demonstrating native XLM)"
    else
        note "no USDC trustline. Not required for native XLM."
        note "Set USDC_ISSUER and rerun setup_treasury.sh before demonstrating a USDC payment."
    fi
elif [ "$TRUSTLINE" -gt 0 ]; then
    ok "the treasury holds a USDC trustline from ${USDC_ISSUER:-<any issuer>}"
    # A trustline with nothing behind it still fails the payment, just one step later.
    if [ "${USDC_BALANCE%%.*}" -gt 0 ] 2>/dev/null; then
        ok "spendable USDC: $USDC_BALANCE"
    else
        bad "the USDC trustline is empty — a USDC payment will fail for want of funds, not want of a trustline"
    fi
else
    bad "no USDC trustline${USDC_ISSUER:+ from $USDC_ISSUER} — a USDC payment will fail AT THE LEDGER, after both approvers have signed"
fi

# Native payments still need spendable XLM: the minimum balance reserve is not spendable, so an
# account holding exactly its reserve can pay nobody.
XLM=$(jq -r '[.balances[] | select(.asset_type == "native") | .balance] | first // "0"' <<<"$ACCOUNT")
SUBENTRIES=$(jq -r '.subentry_count // 0' <<<"$ACCOUNT")
RESERVE=$(awk "BEGIN {printf \"%.1f\", 1 + 0.5 * $SUBENTRIES}")
SPENDABLE=$(awk "BEGIN {printf \"%.4f\", $XLM - $RESERVE}")
if awk "BEGIN {exit !($SPENDABLE > 0)}"; then
    ok "spendable XLM: $SPENDABLE (balance $XLM, reserve $RESERVE for $SUBENTRIES subentries)"
else
    bad "no spendable XLM: balance $XLM is at or below the $RESERVE reserve. A wallet holding 10 XLM cannot send 10 XLM."
fi

echo
if [ "$FAILED" -ne 0 ]; then
    printf '%sThe treasury is NOT correctly configured. Do not demo this.%s\n' "$C_ERR" "$C_0"
    exit 1
fi
ok "Treasury configuration verified — this output is evidence E2"
