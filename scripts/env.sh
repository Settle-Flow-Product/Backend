#!/usr/bin/env bash
#
# Write .env with generated development secrets, and top up anything a newer compose file needs.
#
# Existing values are never overwritten: regenerating SESSION_KEY signs everybody out, and doing it
# silently during a routine `make dev` is a confusing way to discover that.
#
# But "leave it alone" cannot mean "do nothing". A .env written before the admin console existed has
# no ADMIN_SESSION_KEY, so `docker compose` refuses to start and says "run 'make env' first" — and
# the older version of this script then exited without adding it, which made the instruction in the
# error message a no-op. A generated secret that is MISSING is appended; one that is PRESENT is left
# exactly as it is.
#
# The values here are for local development. Production secrets come from the deployment's own
# secret store and never from a generator in a repository.

set -euo pipefail
cd "$(dirname "$0")/.."

generate() { LC_ALL=C tr -dc 'A-Za-z0-9' </dev/urandom | head -c 48; }

# has <key> — is the key already set in .env, ignoring comments?
has() { [ -f .env ] && grep -qE "^[[:space:]]*$1=" .env; }

if [ ! -f .env ]; then
    cat > .env <<EOF
# Written by scripts/env.sh. Local development only — not committed, and not production secrets.
# Regenerating SESSION_KEY signs everybody out.

SESSION_KEY=$(generate)

# Signs the admin console's session cookie. A different key from the backend's on purpose:
# the two apps authenticate different people against different things.
ADMIN_SESSION_KEY=$(generate)

# Stellar CLI key NAMES, not secrets. The CLI holds the private keys in its own keystore, so no
# private key passes through the backend's environment or its logs.
BUSINESS_KEY=settle-business
SETTLER_KEY=settle-settler

# Filled in by \`make deploy\` and \`make treasury\`.
REGISTRY_CONTRACT_ID=
TREASURY_ADDRESS=

# Testnet USDC.
USDC_ISSUER=GBBD47IF6LWK7P7MDEVSCWR7DPUWV3NY3DTQEVFL4NAT4AQH3ZLLFLA5
USDC_CONTRACT=CBIELTK6YBZJU5UP2WWQEUCYKLPU6AUNZ2BQ4WWFEIE3USCIHMXQDAMA
EOF
    chmod 600 .env
    echo "wrote .env (mode 600)"
    echo "fill in REGISTRY_CONTRACT_ID and TREASURY_ADDRESS after 'make deploy' and 'make treasury'"
    exit 0
fi

# The file exists. Add only what compose requires and this file does not have.
added=0
for key in SESSION_KEY ADMIN_SESSION_KEY; do
    if ! has "$key"; then
        printf '\n# Added by scripts/env.sh — this key was required but missing.\n%s=%s\n' \
            "$key" "$(generate)" >> .env
        echo "added $key"
        added=$((added + 1))
    fi
done

for pair in BUSINESS_KEY=settle-business SETTLER_KEY=settle-settler; do
    key="${pair%%=*}"
    if ! has "$key"; then
        printf '%s\n' "$pair" >> .env
        echo "added $key"
        added=$((added + 1))
    fi
done

# ---------------------------------------------------------------------------
# Chain addresses, synced from what was actually deployed
# ---------------------------------------------------------------------------
#
# These four ARE overwritten, and the distinction from the secrets above is the point: a secret is a
# value somebody chose and regenerating it destroys sessions, while a contract id is a derived fact
# about the current deployment. A stale one is not "preserved", it is wrong.
#
# The failure this prevents is quiet. `.env` shipped with Circle's testnet USDC issuer as a
# placeholder; `make treasury` deployed against a different one; and the console then told operators
# a perfectly good vendor address could not receive USDC, because it was checking the trustline
# against an issuer nobody was using.
sync() {
    local key="$1" value="$2"
    [ -n "$value" ] || return 0
    if grep -qE "^[[:space:]]*$key=" .env; then
        local current
        current=$(grep -E "^[[:space:]]*$key=" .env | head -1 | cut -d= -f2-)
        [ "$current" = "$value" ] && return 0
        # `.env.tmp` then move, rather than `sed -i`: the in-place flag differs between GNU and BSD
        # sed, and the portable spelling is not to use it.
        grep -vE "^[[:space:]]*$key=" .env > .env.tmp
        printf '%s=%s\n' "$key" "$value" >> .env.tmp
        mv .env.tmp .env
    else
        printf '%s=%s\n' "$key" "$value" >> .env
    fi
    echo "synced $key"
    synced=$((synced + 1))
}

synced=0
if [ -f .deploy/env ]; then
    # shellcheck disable=SC1091
    . .deploy/env
    sync REGISTRY_CONTRACT_ID "${REGISTRY:-}"
fi
if [ -f .deploy/treasury.env ]; then
    # shellcheck disable=SC1091
    . .deploy/treasury.env
    sync TREASURY_ADDRESS "${TREASURY_ADDRESS:-}"
    sync BUSINESS_ADDRESS "${BUSINESS_ADDRESS:-}"
    sync BUSINESS_KEY     "${BUSINESS_KEY:-}"
    sync USDC_ISSUER      "${USDC_ISSUER:-}"
    sync USDC_CONTRACT    "${USDC_CONTRACT:-}"
fi

chmod 600 .env
if [ "$added" -eq 0 ] && [ "$synced" -eq 0 ]; then
    echo ".env already has everything compose requires — leaving it alone"
else
    echo "topped up .env with $added missing value(s) and synced $synced chain address(es)"
fi
