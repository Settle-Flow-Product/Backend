#!/usr/bin/env bash
#
# The registry contract and the treasury — created if they are missing, reused if they answer.
#
# `make dev` and `make prod` both start with this, which is what makes either of them a single
# command on a machine that has never seen this project. It used to be two commands somebody had to
# know to run first, `make deploy KEY=…` and `make treasury`, and forgetting either produced
# "REGISTRY is not set" from a script three stages later.
#
# ## Reuse is decided by ASKING THE CHAIN, not by reading a file
#
# `.deploy/env` saying a contract exists is not evidence that it does. Testnet resets, and when it
# does every contract and every funded account goes with it while the file stays exactly as it was.
# So each check here is a call: the registry is reused if it answers `get_admin`, and the treasury
# if Horizon still knows the account. A stale file is a rebuild, not a confusing failure later.
#
# Usage:
#   ./scripts/chain_up.sh                # reuse what answers, create what does not
#   ./scripts/chain_up.sh --redeploy     # a NEW registry and treasury, whatever .deploy says
#   KEY=my-key ./scripts/chain_up.sh     # deploy from an existing Stellar CLI key
#
# Exit 0 = .deploy/env and .deploy/treasury.env describe a deployment that is live right now, and
# .env has been synced to match.

set -uo pipefail
cd "$(dirname "$0")/.."

NETWORK="${NETWORK:-testnet}"
HORIZON_URL="${HORIZON_URL:-https://horizon-testnet.stellar.org}"
FRIENDBOT_URL="${FRIENDBOT_URL:-https://friendbot.stellar.org}"
DEPLOY_DIR="${DEPLOY_DIR:-.deploy}"

# The key the contract is deployed FROM, and its admin.
#
# Generated and friendbot-funded on demand, so nothing has to exist first. `KEY=` names one you
# already have — the same spelling `make deploy KEY=…` takes.
DEPLOY_KEY="${KEY:-${DEPLOY_KEY:-settle-deployer}}"

REDEPLOY=0
while [ $# -gt 0 ]; do
    case "$1" in
        --redeploy) REDEPLOY=1 ;;
        *) echo "unknown option: $1" >&2; exit 1 ;;
    esac
    shift
done

if [ -t 1 ]; then
    C_B=$'\033[1m'; C_OK=$'\033[32m'; C_ERR=$'\033[31m'; C_DIM=$'\033[2m'; C_Y=$'\033[33m'; C_0=$'\033[0m'
else
    C_B=; C_OK=; C_ERR=; C_DIM=; C_Y=; C_0=
fi
step() { printf '%s→%s %s\n' "$C_B" "$C_0" "$*"; }
ok()   { printf '%s✓%s %s\n' "$C_OK" "$C_0" "$*"; }
warn() { printf '%s!%s %s\n' "$C_Y" "$C_0" "$*"; }
note() { printf '%s  %s%s\n' "$C_DIM" "$*" "$C_0"; }
die()  { printf '%s✗%s %s\n' "$C_ERR" "$C_0" "$*" >&2; exit 1; }

command -v stellar >/dev/null || die "the Stellar CLI is not installed — https://developers.stellar.org/docs/tools/cli"
command -v jq      >/dev/null || die "jq is not installed (brew install jq)"
command -v curl    >/dev/null || die "curl is not installed"

mkdir -p "$DEPLOY_DIR"

# ---------------------------------------------------------------------------
# The deployer key
# ---------------------------------------------------------------------------
#
# Two separate conditions, because they fail on different days: a key the CLI has never heard of,
# and a key it knows whose account testnet has since forgotten. The second is the one that looks
# like a bug — every command works until one needs a fee.

funded() { curl -fsS -m 15 "$HORIZON_URL/accounts/$1" >/dev/null 2>&1; }

step "deployer key · $DEPLOY_KEY"
if ! stellar keys address "$DEPLOY_KEY" >/dev/null 2>&1; then
    note "not in the keystore — generating one"
    stellar keys generate --fund --network "$NETWORK" "$DEPLOY_KEY" >/dev/null 2>&1 \
        || die "could not generate $DEPLOY_KEY"
fi

DEPLOY_ADDRESS=$(stellar keys address "$DEPLOY_KEY" 2>/dev/null)
[ -n "$DEPLOY_ADDRESS" ] || die "the Stellar CLI knows $DEPLOY_KEY but will not print its address"

if ! funded "$DEPLOY_ADDRESS"; then
    note "the account does not exist on $NETWORK — asking friendbot"
    curl -fsS -m 30 "$FRIENDBOT_URL/?addr=$DEPLOY_ADDRESS" >/dev/null 2>&1 || true
    funded "$DEPLOY_ADDRESS" \
        || die "friendbot did not fund $DEPLOY_ADDRESS. Fund it by hand and run this again."
fi
ok "$DEPLOY_ADDRESS"

# ---------------------------------------------------------------------------
# The registry contract
# ---------------------------------------------------------------------------

REGISTRY=""
if [ -f "$DEPLOY_DIR/env" ]; then
    # shellcheck disable=SC1091
    . "$DEPLOY_DIR/env"
    REGISTRY="${REGISTRY:-}"
fi

# The source of the read-only probe. `.deploy/env` names the key the contract was deployed with,
# and reusing it keeps the probe honest on a machine that holds several.
PROBE_KEY="${ADMIN_KEY:-$DEPLOY_KEY}"
stellar keys address "$PROBE_KEY" >/dev/null 2>&1 || PROBE_KEY="$DEPLOY_KEY"

step "registry contract"
NEED_DEPLOY=1
if [ "$REDEPLOY" = "1" ]; then
    warn "--redeploy: deploying a new registry, ignoring $DEPLOY_DIR/env"
elif [ -n "$REGISTRY" ]; then
    note "$DEPLOY_DIR/env names $REGISTRY — asking whether it is still there"
    if stellar contract invoke --id "$REGISTRY" --source "$PROBE_KEY" --network "$NETWORK" \
        -- get_admin >/dev/null 2>&1; then
        NEED_DEPLOY=0
        ok "$REGISTRY (reused)"
    else
        warn "it does not answer — testnet has probably reset. Deploying a new one."
    fi
else
    note "nothing deployed yet"
fi

if [ "$NEED_DEPLOY" = "1" ]; then
    # Checked here rather than at the top: a run that reuses an existing contract has no reason to
    # need a Rust toolchain, and demanding one would make every `make prod` on a plain VM fail.
    command -v cargo >/dev/null \
        || die "a registry must be deployed and cargo is not installed — https://rustup.rs"
    "${MAKE:-make}" -C smart-contract deploy KEY="$DEPLOY_KEY" NETWORK="$NETWORK" \
        || die "deploying the registry failed"
    # shellcheck disable=SC1091
    . "$DEPLOY_DIR/env"
    REGISTRY="${REGISTRY:-}"
    [ -n "$REGISTRY" ] || die "the deploy reported success but wrote no REGISTRY to $DEPLOY_DIR/env"
    ok "$REGISTRY (deployed)"
fi

# ---------------------------------------------------------------------------
# The treasury
# ---------------------------------------------------------------------------
#
# setup_treasury.sh is already unattended — it generates and funds every key it needs — so there is
# nothing to arrange here beyond deciding whether it has to run at all.

TREASURY=""
if [ -f "$DEPLOY_DIR/treasury.env" ]; then
    # shellcheck disable=SC1091
    . "$DEPLOY_DIR/treasury.env"
    TREASURY="${TREASURY_ADDRESS:-}"
fi

step "treasury"
NEED_TREASURY=1
if [ "$REDEPLOY" = "1" ]; then
    warn "--redeploy: configuring a new treasury"
elif [ -n "$TREASURY" ]; then
    note "$DEPLOY_DIR/treasury.env names $TREASURY — asking Horizon"
    if funded "$TREASURY"; then
        NEED_TREASURY=0
        ok "$TREASURY (reused)"
    else
        warn "Horizon does not know that account — testnet has probably reset. Rebuilding it."
    fi
else
    note "no treasury yet"
fi

if [ "$NEED_TREASURY" = "1" ]; then
    note "three signers, threshold 2, masterWeight 0 — this takes a minute"
    ( cd backend && ./scripts/setup_treasury.sh ) || die "configuring the treasury failed"
    # shellcheck disable=SC1091
    . "$DEPLOY_DIR/treasury.env"
    TREASURY="${TREASURY_ADDRESS:-}"
    [ -n "$TREASURY" ] || die "setup_treasury.sh wrote no TREASURY_ADDRESS"
    ok "$TREASURY (configured)"
fi

# ---------------------------------------------------------------------------
# Into .env, where the services read it
# ---------------------------------------------------------------------------
#
# A contract deployed but never written to the environment the backend reads is a deployment the
# console cannot see.

step "syncing .env"
./scripts/env.sh || die "scripts/env.sh failed"
