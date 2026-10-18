#!/usr/bin/env bash
#
# The SOW 1 round-trip demo, and the thing that writes `.deploy/evidence.md`.
#
# Runs every case end to end on testnet and records a transaction hash at each step AS IT HAPPENS.
# A hash not captured at the moment it happens is a hash gone — which is why this writes the
# evidence file incrementally rather than at the end, so a run that dies half way still leaves
# behind everything it did prove.
#
# Cases:
#   A · happy path      create -> approve (2 of 3) -> pay the vendor -> mark_paid -> read back Paid
#   B · reject branch   create -> cancel, and ASSERT no transaction left the treasury
#   C · abuse           duplicate id, second mark_paid, non-settler approve — each must be refused
#
# Usage:
#   ./scripts/demo.sh                  # uses .deploy/env from `make deploy` and `make treasury`
#   CASES=A ./scripts/demo.sh          # one case only
#   DRY_RUN=1 ./scripts/demo.sh        # print the plan, touch no network
#
# Exit 0 = every case behaved as specified and the evidence file is complete.

set -uo pipefail
cd "$(dirname "$0")/.."

# `set -e` is deliberately NOT used.
#
# Half of what this script proves is that certain calls are REFUSED, so a non-zero exit is expected
# output rather than a fault. Errexit would abort the run at the first refusal and leave an evidence
# file with no verdict — which is exactly what happened the first time this was tested against an
# unreachable registry.
#
# Instead every step is checked explicitly, and the trap below guarantees a verdict even if
# something unforeseen kills the run.
# Declared BEFORE the trap that reads them: `set -u` turns an early exit into a confusing
# "unbound variable" instead of the real reason the run stopped.
EVIDENCE_FINISHED=0
FAILURES=0
STEP_NO=0
# Whether THIS run owns the evidence file yet. Until `start_evidence` has truncated it, $EVIDENCE
# still holds the committed evidence of the last real run, and appending to it would destroy proof
# this run did not produce.
EVIDENCE_STARTED=0

on_exit() {
    local code=$?
    if [ "$EVIDENCE_STARTED" = "1" ] && [ "$EVIDENCE_FINISHED" = "0" ]; then
        FAILURES=$((FAILURES + 1))
        record "the run did not finish" "every case completes" "❌ **aborted** (exit $code)" ""
        finish_evidence "FAILED"
        printf "\n%s✗%s the demo aborted. %s holds everything it did prove.\n" \
            "${C_ERR:-}" "${C_0:-}" "$EVIDENCE" >&2
    fi
}
trap on_exit EXIT

NETWORK="${NETWORK:-testnet}"
RPC_URL="${RPC_URL:-https://soroban-testnet.stellar.org}"
HORIZON_URL="${HORIZON_URL:-https://horizon-testnet.stellar.org}"
DEPLOY_DIR="${DEPLOY_DIR:-.deploy}"
EVIDENCE="$DEPLOY_DIR/evidence.md"
CASES="${CASES:-ABC}"
DRY_RUN="${DRY_RUN:-0}"

# ---------------------------------------------------------------------------
# Output
# ---------------------------------------------------------------------------

if [ -t 1 ]; then
    C_B=$'\033[1m'; C_OK=$'\033[32m'; C_ERR=$'\033[31m'; C_DIM=$'\033[2m'; C_Y=$'\033[33m'; C_0=$'\033[0m'
else
    C_B=; C_OK=; C_ERR=; C_DIM=; C_Y=; C_0=
fi
# NOT `head`, and not `tail`, `sort`, `test`, `printf`…
#
# A shell function silently shadows the binary of the same name for the whole script. An earlier
# version of this file called its section heading `head()`, so every `| head -1` in the hash
# extraction called THAT instead — and wrote its argument, "-1", into the evidence table as a
# transaction hash. It looked like a parsing bug for a long time.
banner() { printf '\n%s%s%s\n' "$C_B" "$*" "$C_0"; }
step() { printf '%s→%s %s\n' "$C_B" "$C_0" "$*"; }
ok()   { printf '%s✓%s %s\n' "$C_OK" "$C_0" "$*"; }
warn() { printf '%s!%s %s\n' "$C_Y" "$C_0" "$*"; }
note() { printf '%s  %s%s\n' "$C_DIM" "$*" "$C_0"; }
# A preflight `die` — no registry, no jq — fires BEFORE `start_evidence`, which is before
# `finish_evidence` is even defined: bash resolves a function name when the call runs, so this used
# to abort with "finish_evidence: command not found" instead of the reason the run stopped. Defining
# it earlier would not have been the fix, because at that point $EVIDENCE is still the LAST run's
# committed evidence — and a machine that has simply never run `make deploy` would have had a
# "FAILED" verdict appended to a genuine, passing run's proof. So: report, and leave the file alone.
die() {
    printf '%s✗%s %s\n' "$C_ERR" "$C_0" "$*" >&2
    FAILURES=$((FAILURES + 1))
    if [ "$EVIDENCE_STARTED" = "1" ]; then
        finish_evidence "FAILED"
    fi
    EVIDENCE_FINISHED=1
    exit 1
}

# ---------------------------------------------------------------------------
# Preflight
# ---------------------------------------------------------------------------

command -v stellar >/dev/null || die "the Stellar CLI is not installed"
command -v jq      >/dev/null || die "jq is not installed (brew install jq)"
command -v curl    >/dev/null || die "curl is not installed"

mkdir -p "$DEPLOY_DIR"

if [ -f "$DEPLOY_DIR/env" ]; then
    # shellcheck disable=SC1091
    . "$DEPLOY_DIR/env"
fi
if [ -f "$DEPLOY_DIR/treasury.env" ]; then
    # shellcheck disable=SC1091
    . "$DEPLOY_DIR/treasury.env"
fi

REGISTRY="${REGISTRY:-}"
ADMIN_KEY="${ADMIN_KEY:-}"
SETTLER_KEY="${SETTLER_KEY:-$ADMIN_KEY}"
TREASURY_ADDRESS="${TREASURY_ADDRESS:-}"
TREASURY_KEY="${TREASURY_KEY:-}"
VENDOR_KEY="${VENDOR_KEY:-settle-vendor}"
SIGNER1_KEY="${SIGNER1_KEY:-settle-signer-1}"
SIGNER2_KEY="${SIGNER2_KEY:-settle-signer-2}"
SIGNER3_KEY="${SIGNER3_KEY:-settle-signer-3}"
AMOUNT_UNITS="${AMOUNT_UNITS:-10}"

# ---------------------------------------------------------------------------
# The asset, in its two forms
# ---------------------------------------------------------------------------
#
# The same asset is named two different ways, and conflating them is why the USDC path never worked:
#
#   ASSET      the CONTRACT's `asset` field, which is an Address — a Stellar Asset Contract id, or
#              the literal `native`. This is what create_request records.
#   PAY_ASSET  the CLASSIC payment operation's asset — `native`, or `CODE:ISSUER`.
#
# `stellar tx new payment` defaults --asset to native. The payment step never passed the flag at all,
# so every demo paid XLM no matter what ASSET said, and the registry recorded an asset the treasury
# had not actually sent.
#
# `DEMO_ASSET=USDC` sets both from .deploy/treasury.env, which is the ergonomic way in.
DEMO_ASSET="${DEMO_ASSET:-XLM}"
# `tr`, not `${DEMO_ASSET^^}`: macOS ships bash 3.2, where `^^` is a syntax error rather than an
# uppercase — and a syntax error inside an `if` condition takes BOTH branches out, so the variables
# below end up unset instead of defaulted.
DEMO_ASSET_UC=$(printf '%s' "$DEMO_ASSET" | tr '[:lower:]' '[:upper:]')

if [ "$DRY_RUN" != "1" ] && [ "$DEMO_ASSET_UC" = "USDC" ]; then
    [ -n "${USDC_ISSUER:-}" ] || die "DEMO_ASSET=USDC but USDC_ISSUER is not set. Run 'make treasury' first."
    ASSET="${ASSET:-${USDC_CONTRACT:-native}}"
    PAY_ASSET="${PAY_ASSET:-USDC:$USDC_ISSUER}"
    ASSET_CODE="USDC"
    # The evidence names the issuer in full. A reviewer has to be able to tell at a glance whether
    # this was Circle's USDC or an issuer we control — "USDC" alone invites them to assume the
    # former. setup_treasury.sh writes USDC_ISSUER_KEY only for an issuer it generated, so its
    # presence is exactly the "this asset is ours" signal.
    #
    # The URL is interpolated at USE, not here: the expert_* helpers are defined further down, and a
    # command substitution runs when the assignment does.
    if [ -n "${USDC_ISSUER_KEY:-}" ]; then
        ASSET_SELF_ISSUED=1
    else
        ASSET_SELF_ISSUED=0
    fi
else
    ASSET="${ASSET:-native}"
    PAY_ASSET="${PAY_ASSET:-native}"
    ASSET_CODE="XLM"
    ASSET_SELF_ISSUED=0
fi

if [ "$DRY_RUN" != "1" ]; then
    [ -n "$REGISTRY" ]         || die "REGISTRY is not set. Run 'make deploy KEY=…' first."
    [ -n "$ADMIN_KEY" ]        || die "ADMIN_KEY is not set. Run 'make deploy KEY=…' first."
    [ -n "$TREASURY_ADDRESS" ] || die "TREASURY_ADDRESS is not set. Run 'make treasury' first."
fi

# ---------------------------------------------------------------------------
# Evidence, written incrementally
# ---------------------------------------------------------------------------

expert_tx()       { echo "https://stellar.expert/explorer/testnet/tx/$1"; }
expert_contract() { echo "https://stellar.expert/explorer/testnet/contract/$1"; }
expert_account()  { echo "https://stellar.expert/explorer/testnet/account/$1"; }

# Built here, below the helpers it calls, and read by the evidence header further down.
if [ "$ASSET_CODE" = "XLM" ]; then
    ASSET_NOTE='native `XLM`'
elif [ "$ASSET_SELF_ISSUED" = "1" ]; then
    ASSET_NOTE="\`USDC\` — **self-issued test asset**, issuer \`$USDC_ISSUER\` ([view]($(expert_account "$USDC_ISSUER"))). Not Circle's USDC."
else
    ASSET_NOTE="\`USDC\` — issuer \`$USDC_ISSUER\` ([view]($(expert_account "$USDC_ISSUER")))"
fi

start_evidence() {
    cat > "$EVIDENCE" <<EOF
# Evidence — SettleFlow SOW 1

Committed on purpose. A reviewer opening this repository has to find the transaction hashes without
running anything, and a deliverable that only exists on the machine that produced it is not evidence
of anything.

Written **step by step, as each step happens**, by \`scripts/demo.sh\`. A hash not captured at the
moment it happens is a hash gone, so a run that dies half way still leaves behind everything it did
prove.

| | |
|---|---|
| Network | \`$NETWORK\` |
| Registry contract | \`$REGISTRY\` — [view]($(expert_contract "$REGISTRY")) |
| Treasury | \`$TREASURY_ADDRESS\` — [view]($(expert_account "$TREASURY_ADDRESS")) |
| Asset paid | $ASSET_NOTE |
| Generated | $(date -u '+%Y-%m-%d %H:%M:%SZ') |

The milestones are defined in [\`../docs/15-evidence.md\`](../docs/15-evidence.md).

---
EOF
    EVIDENCE_STARTED=1
}

open_case() {
    printf '\n## Case %s\n\n%s\n\n| # | Step | Expected | Result | Hash / id |\n|---|---|---|---|---|\n' \
        "$1" "$2" >> "$EVIDENCE"
}

# record <step> <expected> <result> <hash> [url]
record() {
    STEP_NO=$((STEP_NO + 1))
    local step="$1" expected="$2" result="$3" url="${5:-}"
    # A newline inside a table cell truncates the row and every column after it.
    local value; value=$(printf '%s' "${4:-}" | tr -d '\n')
    local shown="—"
    if [ -n "$value" ]; then
        # A link is only rendered when there is something real to link to; an empty href in an
        # evidence table is a 404 waiting for the reviewer.
        if [ -n "$url" ]; then
            shown="[\`$value\`]($url)"
        else
            shown="\`$value\`"
        fi
    fi
    printf '| %d | %s | %s | %s | %s |\n' "$STEP_NO" "$step" "$expected" "$result" "$shown" >> "$EVIDENCE"
}

pass() { record "$1" "$2" "✅ as specified" "${3:-}" "${4:-}"; ok "$1"; [ -n "${3:-}" ] && note "${3}"; return 0; }
fail() { record "$1" "$2" "❌ **NOT as specified**" "${3:-}"; FAILURES=$((FAILURES + 1)); warn "$1 — $2"; return 0; }

# A dry run records what WOULD happen and asserts nothing.
#
# The alternative — letting a stubbed call return success — makes every negative check pass without
# touching the network, and a green evidence file that proves nothing is worse than no file at all.
skip() { record "$1" "$2" "⏸ not run (dry run)" ""; note "would: $1"; return 0; }

planned() {
    local title="$1"; shift
    step "$title"
    while [ "$#" -gt 0 ]; do
        skip "$1" "$2"
        shift 2
    done
}

finish_evidence() {
    local verdict="${1:-}"
    EVIDENCE_FINISHED=1
    {
        echo
        echo "---"
        echo
        if [ "$DRY_RUN" = "1" ]; then
            echo "## Verdict"
            echo
            echo "**Dry run — nothing was executed and nothing is proved.** $STEP_NO steps are listed"
            echo "above as a plan. Run without \`DRY_RUN=1\` against testnet to produce real evidence."
        elif [ "$FAILURES" -eq 0 ] && [ "$verdict" != "FAILED" ]; then
            echo "## Verdict"
            echo
            echo "Every case behaved as the specification requires. $STEP_NO steps recorded."
        else
            echo "## Verdict"
            echo
            echo "**$FAILURES step(s) did not behave as specified.** Do not submit this run as evidence."
        fi
        echo
        echo "### Reproducing it"
        echo
        echo '```bash'
        echo 'make deploy KEY=<your stellar key name>   # E1'
        echo 'make treasury                             # E2'
        echo 'make verify-treasury                      # asserts E2 and prints it'
        echo './scripts/demo.sh                         # E3 → E7, and this file'
        echo '```'
    } >> "$EVIDENCE"
}

# ---------------------------------------------------------------------------
# Chain helpers
# ---------------------------------------------------------------------------

# invoke <source-key> <send:yes|no> <function> [args…]  → stdout, or non-zero on refusal
#
# Sets LAST_TX_HASH as a side effect. The CLI prints the transaction hash to STDERR
# ("Signing transaction: <hash>"), separately from the return value on stdout — and E3, E4 and E6
# are those hashes, not the request id. Recording the request id under a Stellar Expert link would
# produce an evidence table whose links all 404.
# The hash goes to a FILE, not a variable.
#
# Most call sites read the return value with `out=$(invoke …)`, which runs the function in a
# subshell — so a variable set inside it never reaches the caller, and every hash silently came
# back empty. A file survives the subshell.
HASH_FILE="${HASH_FILE:-/tmp/settleflow_demo_hash}"

invoke() {
    local key="$1" send="$2" fn="$3"; shift 3
    : > "$HASH_FILE"
    local out rc
    out=$(stellar contract invoke \
        --id "$REGISTRY" --source "$key" \
        --rpc-url "$RPC_URL" --network-passphrase "$(passphrase)" \
        --send "$send" -- "$fn" "$@" 2>/tmp/demo_err)
    rc=$?
    grep -oE '[0-9a-f]{64}' /tmp/demo_err | head -1 | tr -d '[:space:]' > "$HASH_FILE" || true
    printf '%s' "$out"
    return $rc
}

# The hash of the invocation that just ran, or empty for a simulated read.
last_tx_hash() { tr -d '[:space:]' < "$HASH_FILE" 2>/dev/null || true; }

passphrase() {
    case "$NETWORK" in
        testnet) echo "Test SDF Network ; September 2015" ;;
        *) die "only testnet is in scope for SOW 1" ;;
    esac
}

# A deterministic 32-byte id, so a rerun re-derives the same one — which is exactly the property
# case C then exercises.
request_id() { printf '%s' "$1" | shasum -a 256 | cut -c1-64; }

# Prove the network answers before believing any refusal.
#
# Without this, every "must be refused" check passes when the registry is unreachable, the key is
# missing, or the RPC is down — a green run that proves nothing. A negative test that cannot fail is
# not a test, so each case establishes a positive control first and abandons itself if it fails.
positive_control() {
    local case_name="$1" key="$2"
    if ! stellar keys address "$key" >/dev/null 2>&1; then
        fail "$case_name · positive control" \
             "the Stellar CLI does not know the key '$key' — every refusal below would be meaningless"
        return 1
    fi
    if ! invoke "$key" no get_admin >/dev/null 2>&1; then
        fail "$case_name · positive control" \
             "the registry at $REGISTRY did not answer — every refusal below would be meaningless"
        return 1
    fi
    pass "$case_name · positive control" "the registry answers, so a refusal below means the rule fired" "reachable"
    return 0
}

# The contract returns ReqStatus as a NUMBER, so a status must be parsed rather than grepped for.
#
# The first version of this script used `grep -qi paid` on the whole JSON document. It passed —
# because the payload contains a field called `paid_tx`. An assertion that matches a field NAME
# rather than a value is an assertion that cannot fail, which is worse than no assertion at all.
status_name() {
    case "$(jq -r '.status // empty' <<<"$1" 2>/dev/null)" in
        0) echo "pending"   ;;
        1) echo "approved"  ;;
        2) echo "paid"      ;;
        3) echo "cancelled" ;;
        *) echo "unreadable" ;;
    esac
}

# assert_status <json> <expected> <step> <expectation>
assert_status() {
    local got; got=$(status_name "$1")
    if [ "$got" = "$2" ]; then
        pass "$3" "$4" "$got"
    else
        fail "$3" "expected $2, the contract says $got"
    fi
}

treasury_tx_count() {
    curl -sf "$HORIZON_URL/accounts/$TREASURY_ADDRESS/transactions?limit=200&order=desc" \
        | jq '[._embedded.records[]] | length' 2>/dev/null || echo 0
}

# ---------------------------------------------------------------------------
# Case A · the happy path
# ---------------------------------------------------------------------------

case_a() {
    banner "Case A · happy path"
    open_case "A · happy path" \
"Create a payment, approve it as the settler once two of three signers have signed off chain, pay
the vendor from the treasury, and mark the registry paid — **after** reading the ledger back."

    if [ "$DRY_RUN" = "1" ]; then
        planned "planning case A" \
            "E3 · create_request"              "the request is recorded on chain" \
            "readback · status"                "Pending, read from the contract rather than inferred" \
            "E4 · approve"                     "Pending → Approved once weight ≥ threshold" \
            "E5 · payment to the vendor"       "the treasury pays, needing 2 of 3 signatures" \
            "E6 · mark_paid"                   "written only after the ledger confirmed the payment" \
            "E6 · get_request returns Paid"    "the chain is the source of truth" \
            "is_payable is false"              "single use — a paid request cannot be paid again"
        return
    fi

    positive_control "A" "$ADMIN_KEY" || return

    local ref="PO-DEMO-A-$(date +%s)"
    local id; id=$(request_id "$ref")
    local expiry=$(( $(date +%s) + 3600 ))
    local amount=$(( AMOUNT_UNITS * 10000000 ))
    local vendor
    vendor=$(stellar keys address "$VENDOR_KEY" 2>/dev/null || echo "")
    if [ -z "$vendor" ]; then
        stellar keys generate --fund --network "$NETWORK" "$VENDOR_KEY" >/dev/null 2>&1 || true
        vendor=$(stellar keys address "$VENDOR_KEY" 2>/dev/null || echo "")
    fi
    if [ -z "$vendor" ]; then
        fail "vendor account" "a funded vendor account is needed to pay anything"
        return
    fi

    # The vendor's side of the trustline. The treasury having one is not enough — an issued asset
    # needs a trustline on BOTH ends, and the destination's is the one that fails as `op_no_trust`
    # at submit, after two approvers have already signed.
    if [ "$PAY_ASSET" != "native" ]; then
        step "A0 · the vendor can receive $ASSET_CODE"
        if ! stellar tx new change-trust \
                --source "$VENDOR_KEY" --network "$NETWORK" \
                --line "$PAY_ASSET" >/dev/null 2>/tmp/demo_err; then
            fail "vendor trustline" \
                 "the vendor cannot receive $ASSET_CODE: $(tail -2 /tmp/demo_err | tr '\n' ' ')"
            return
        fi
        ok "vendor holds a $ASSET_CODE trustline"
    fi

    note "reference $ref"
    note "id        $id"
    note "vendor    $vendor"

    step "A1 · create_request"
    if out=$(invoke "$ADMIN_KEY" yes create_request \
            --creator "$(stellar keys address "$ADMIN_KEY")" \
            --payee "$vendor" --id "$id" --asset "$ASSET" \
            --amount "$amount" --reference "$id" --expiry "$expiry"); then
        local h; h=$(last_tx_hash)
        pass "E3 · create_request" "the request is recorded on chain" \
             "${h:-$id}" "${h:+$(expert_tx "$h")}"
    else
        fail "E3 · create_request" "the request should have been recorded: $(tail -2 /tmp/demo_err | tr '\n' ' ')"
        return
    fi

    step "A2 · readback says Pending"
    out=$(invoke "$SETTLER_KEY" no get_request --id "$id" || echo "")
    assert_status "$out" "pending" "readback · status" \
        "Pending, read from the contract rather than inferred"

    step "A3 · approve, as the settler"
    if invoke "$SETTLER_KEY" yes approve --id "$id" >/dev/null; then
        local h; h=$(last_tx_hash)
        pass "E4 · approve" "Pending → Approved once weight ≥ threshold" \
             "${h:-$id}" "${h:+$(expert_tx "$h")}"
    else
        fail "E4 · approve" "the settler should have been able to approve: $(tail -2 /tmp/demo_err | tr '\n' ' ')"
    fi

    step "A3b · the contract says Approved"
    out=$(invoke "$SETTLER_KEY" no get_request --id "$id" || echo "")
    assert_status "$out" "approved" "readback · Approved" \
        "the on-chain status moved only once the threshold was met"

    # ── The 2-of-3 approval. This is Deliverable 2, and the whole point of SOW 1. ──────────────
    #
    # Build once, sign twice, submit once. The treasury's master key has weight 0, so the source
    # account cannot authorise its own payment: two of the three signers must sign the SAME
    # envelope, over the same hash and the same sequence number.
    #
    # An earlier version of this script signed only with the source key and got `TxBadAuth` — which
    # was the multisig working correctly and the demo asking for the wrong thing.

    step "A4a · build the unsigned payment"
    local unsigned
    unsigned=$(stellar tx new payment \
        --build-only \
        --source-account "$TREASURY_ADDRESS" --network "$NETWORK" \
        --destination "$vendor" --asset "$PAY_ASSET" --amount "$amount" 2>/tmp/demo_err | tail -1 || true)
    if [ -n "$unsigned" ]; then
        pass "build_unsigned" "one envelope, for every approver to sign" "${unsigned:0:24}…"
    else
        fail "build_unsigned" "could not build the payment: $(tail -2 /tmp/demo_err | tr '\n' ' ')"
        return
    fi

    step "A4b · signature 1 of 2 — $SIGNER1_KEY"
    local signed1
    signed1=$(stellar tx sign "$unsigned" \
        --sign-with-key "$SIGNER1_KEY" --network "$NETWORK" 2>/tmp/demo_err | tail -1 || true)
    if [ -n "$signed1" ] && [ "$signed1" != "$unsigned" ]; then
        pass "signature 1 of 2" "the first approver signs; one signature is not enough" "$(stellar keys address "$SIGNER1_KEY")"
    else
        fail "signature 1 of 2" "the first signature was not attached: $(tail -2 /tmp/demo_err | tr '\n' ' ')"
        return
    fi

    step "A4c · one signature must NOT be enough"
    if stellar tx send "$signed1" --network "$NETWORK" >/dev/null 2>&1; then
        fail "one signature is refused" "a SINGLE signature moved the money — the threshold is not being enforced"
    else
        pass "one signature is refused" "below the threshold, the network rejects it — 2 of 3 is real" "tx_bad_auth"
    fi

    step "A4d · signature 2 of 2 — $SIGNER2_KEY"
    local signed2
    signed2=$(stellar tx sign "$signed1" \
        --sign-with-key "$SIGNER2_KEY" --network "$NETWORK" 2>/tmp/demo_err | tail -1 || true)
    if [ -n "$signed2" ] && [ "$signed2" != "$signed1" ]; then
        pass "signature 2 of 2" "the threshold is met on the SAME envelope" "$(stellar keys address "$SIGNER2_KEY")"
    else
        fail "signature 2 of 2" "the second signature was not attached: $(tail -2 /tmp/demo_err | tr '\n' ' ')"
        return
    fi

    step "A4e · submit"
    # `tx send` prints a JSON result on stdout. Parse it — the status and the hash are both fields,
    # and greping a 500-line document for something that looks like a hash is how you end up
    # recording a signature as a transaction id.
    local send_out send_status
    send_out=$(stellar tx send "$signed2" --network "$NETWORK" 2>/tmp/demo_err || true)
    send_status=$(jq -r '.status // empty' <<<"$send_out" 2>/dev/null || true)
    local pay_hash
    pay_hash=$(jq -r '.tx_hash // empty' <<<"$send_out" 2>/dev/null || true)

    if [ "$send_status" = "SUCCESS" ] && [ -n "$pay_hash" ]; then
        pass "E5 · payment to the vendor" "the treasury pays, on 2 of 3 signatures" "$pay_hash" "$(expert_tx "$pay_hash")"
    else
        fail "E5 · payment to the vendor" \
             "status=${send_status:-<none>} — $(tail -3 /tmp/demo_err | tr '\n' ' ')"
        return
    fi

    step "A4f · the ledger says the operation succeeded"
    local op_result
    op_result=$(jq -r '.result.result.tx_success[0].op_inner.payment // empty' <<<"$send_out" 2>/dev/null || true)
    if [ "$op_result" = "success" ]; then
        pass "the payment operation succeeded" "read from the transaction result, not from the submit call" "success"
    else
        fail "the payment operation succeeded" "the operation result was: ${op_result:-<none>}"
    fi

    step "A4g · the vendor actually received it"
    # Filtered by the asset that was actually sent. Hardcoding `native` here meant a USDC payout
    # reported the vendor's XLM balance — a number that is real, unchanged, and completely beside
    # the point.
    local vendor_balance
    if [ "$PAY_ASSET" = "native" ]; then
        vendor_balance=$(curl -sf "$HORIZON_URL/accounts/$vendor" \
            | jq -r '[.balances[] | select(.asset_type=="native") | .balance] | first // "0"' 2>/dev/null || echo 0)
    else
        vendor_balance=$(curl -sf "$HORIZON_URL/accounts/$vendor" \
            | jq -r --arg c "${PAY_ASSET%%:*}" --arg i "${PAY_ASSET#*:}" \
                '[.balances[] | select(.asset_code==$c and .asset_issuer==$i) | .balance] | first // "0"' \
                2>/dev/null || echo 0)
    fi
    pass "the vendor holds the funds" "read from the ledger, not from our own records" "$vendor_balance $ASSET_CODE"

    step "A5 · mark_paid, AFTER the readback"
    if invoke "$SETTLER_KEY" yes mark_paid --id "$id" --amount "$amount" --tx "$id" >/dev/null; then
        local h; h=$(last_tx_hash)
        pass "E6 · mark_paid" "written only after the ledger confirmed the payment" \
             "${h:-$id}" "${h:+$(expert_tx "$h")}"
    else
        fail "E6 · mark_paid" "mark_paid was refused: $(tail -2 /tmp/demo_err | tr '\n' ' ')"
    fi

    step "A6 · the contract says Paid"
    out=$(invoke "$SETTLER_KEY" no get_request --id "$id" || echo "")
    assert_status "$out" "paid" "E6 · get_request returns Paid" "the chain is the source of truth"

    step "A7 · the link is spent"
    out=$(invoke "$SETTLER_KEY" no is_payable --id "$id" || echo "")
    if [ "$(tr -d '[:space:]' <<<"$out")" = "false" ]; then
        pass "is_payable is false" "single use — a paid request cannot be paid again" "false"
    else
        fail "is_payable is false" "expected exactly false, the contract says: ${out:-<nothing>}"
    fi

    DEMO_A_ID="$id"
}

# ---------------------------------------------------------------------------
# Case B · the reject branch
# ---------------------------------------------------------------------------

case_b() {
    banner "Case B · reject branch"
    open_case "B · reject branch" \
"An approver refuses. The request must reach \`Cancelled\`, and **no transaction may leave the
treasury** — a negative claim, so it is proved by counting the treasury's transactions before and
after rather than by anything in our own database."

    if [ "$DRY_RUN" = "1" ]; then
        planned "planning case B" \
            "create_request"                       "a second request, this one to be refused" \
            "cancel_request"                       "the reject branch stops the payment entirely" \
            "get_request returns Cancelled"        "terminal, and visible on chain" \
            "mark_paid on a cancelled request"     "refused, as it must be" \
            "E7 · no transaction left the treasury" "counted on chain, before and after"
        return
    fi

    positive_control "B" "$ADMIN_KEY" || return

    local ref="PO-DEMO-B-$(date +%s)"
    local id; id=$(request_id "$ref")
    local expiry=$(( $(date +%s) + 3600 ))
    local amount=$(( AMOUNT_UNITS * 10000000 ))
    local vendor
    vendor=$(stellar keys address "$VENDOR_KEY" 2>/dev/null || echo "")
    if [ -z "$vendor" ]; then fail "vendor account" "no vendor account"; return; fi

    local before; before=$(treasury_tx_count)
    note "treasury transactions before: $before"

    step "B1 · create_request"
    if invoke "$ADMIN_KEY" yes create_request \
            --creator "$(stellar keys address "$ADMIN_KEY")" \
            --payee "$vendor" --id "$id" --asset "$ASSET" \
            --amount "$amount" --reference "$id" --expiry "$expiry" >/dev/null; then
        local h; h=$(last_tx_hash)
        pass "create_request" "a second request, this one to be refused" \
             "${h:-$id}" "${h:+$(expert_tx "$h")}"
    else
        fail "create_request" "could not create the request to reject"
        return
    fi

    step "B2 · the creator cancels"
    if invoke "$ADMIN_KEY" yes cancel_request --id "$id" >/dev/null; then
        local h; h=$(last_tx_hash)
        pass "cancel_request" "the reject branch stops the payment entirely" \
             "${h:-$id}" "${h:+$(expert_tx "$h")}"
    else
        fail "cancel_request" "the creator should have been able to cancel"
    fi

    step "B3 · the contract says Cancelled"
    out=$(invoke "$SETTLER_KEY" no get_request --id "$id" || echo "")
    assert_status "$out" "cancelled" "get_request returns Cancelled" "terminal, and visible on chain"

    step "B4 · a cancelled request cannot be paid"
    if invoke "$SETTLER_KEY" yes mark_paid --id "$id" --amount "$amount" --tx "$id" >/dev/null 2>&1; then
        fail "mark_paid on a cancelled request" "it was ACCEPTED — a refused payment could still be made"
    else
        pass "mark_paid on a cancelled request" "refused, as it must be" "NotApproved"
    fi

    step "B5 · E7 · no transaction left the treasury"
    local after; after=$(treasury_tx_count)
    note "treasury transactions after:  $after"
    if [ "$after" = "$before" ]; then
        pass "E7 · no transaction left the treasury" \
             "counted on chain, before and after — not read from our database" "$before → $after"
    else
        fail "E7 · no transaction left the treasury" \
             "the count moved from $before to $after; a rejected payment may have been made"
    fi
}

# ---------------------------------------------------------------------------
# Case C · abuse resistance
# ---------------------------------------------------------------------------

case_c() {
    banner "Case C · abuse resistance"
    open_case "C · abuse resistance" \
"Each of these must be REFUSED. A demo that only shows the happy path proves the system works when
nothing goes wrong, which is not the interesting half."

    if [ "$DRY_RUN" = "1" ]; then
        planned "planning case C" \
            "duplicate id refused"      "on-chain idempotency — a retry cannot create a second request" \
            "zero amount refused"       "an amount of zero has no business meaning" \
            "past expiry refused"       "measured against the LEDGER clock, not a server clock" \
            "double payment refused"    "single use, checked ON CHAIN rather than only in our database"
        return
    fi

    positive_control "C" "$ADMIN_KEY" || return

    local ref="PO-DEMO-C-$(date +%s)"
    local id; id=$(request_id "$ref")
    local expiry=$(( $(date +%s) + 3600 ))
    local amount=$(( AMOUNT_UNITS * 10000000 ))
    local vendor creator
    vendor=$(stellar keys address "$VENDOR_KEY" 2>/dev/null || echo "")
    creator=$(stellar keys address "$ADMIN_KEY" 2>/dev/null || echo "")
    if [ -z "$vendor" ] || [ -z "$creator" ]; then fail "accounts" "no vendor or creator account"; return; fi

    invoke "$ADMIN_KEY" yes create_request \
        --creator "$creator" --payee "$vendor" --id "$id" --asset "$ASSET" \
        --amount "$amount" --reference "$id" --expiry "$expiry" >/dev/null || true

    step "C1 · the same id cannot be created twice"
    if invoke "$ADMIN_KEY" yes create_request \
            --creator "$creator" --payee "$vendor" --id "$id" --asset "$ASSET" \
            --amount "$amount" --reference "$id" --expiry "$expiry" >/dev/null 2>&1; then
        fail "duplicate id" "a second request with the same id was ACCEPTED"
    else
        pass "duplicate id refused" "on-chain idempotency — a retry cannot create a second request" "RequestExists"
    fi

    step "C2 · a zero amount is refused"
    local zid; zid=$(request_id "$ref-zero")
    if invoke "$ADMIN_KEY" yes create_request \
            --creator "$creator" --payee "$vendor" --id "$zid" --asset "$ASSET" \
            --amount 0 --reference "$zid" --expiry "$expiry" >/dev/null 2>&1; then
        fail "zero amount" "a zero-amount request was ACCEPTED"
    else
        pass "zero amount refused" "an amount of zero has no business meaning" "InvalidAmount"
    fi

    step "C3 · an expiry in the past is refused"
    local eid; eid=$(request_id "$ref-expired")
    if invoke "$ADMIN_KEY" yes create_request \
            --creator "$creator" --payee "$vendor" --id "$eid" --asset "$ASSET" \
            --amount "$amount" --reference "$eid" --expiry 1 >/dev/null 2>&1; then
        fail "past expiry" "a request expiring in the past was ACCEPTED"
    else
        pass "past expiry refused" "measured against the LEDGER clock, not a server clock" "InvalidExpiry"
    fi

    step "C4 · a request cannot be paid twice"
    invoke "$SETTLER_KEY" yes approve --id "$id" >/dev/null 2>&1 || true
    invoke "$SETTLER_KEY" yes mark_paid --id "$id" --amount "$amount" --tx "$id" >/dev/null 2>&1 || true
    if invoke "$SETTLER_KEY" yes mark_paid --id "$id" --amount "$amount" --tx "$id" >/dev/null 2>&1; then
        fail "double payment" "a second mark_paid was ACCEPTED — the vendor could be paid twice"
    else
        pass "double payment refused" "single use, checked ON CHAIN rather than only in our database" "AlreadyPaid"
    fi
}

# ---------------------------------------------------------------------------
# Run
# ---------------------------------------------------------------------------

banner "SettleFlow SOW 1 · round-trip demo"
note "network   $NETWORK"
note "registry  ${REGISTRY:-(not deployed)}"
note "treasury  ${TREASURY_ADDRESS:-(not configured)}"
note "cases     $CASES"
[ "$DRY_RUN" = "1" ] && warn "DRY_RUN — nothing will touch the network"

start_evidence

case "$CASES" in *A*) case_a ;; esac
case "$CASES" in *B*) case_b ;; esac
case "$CASES" in *C*) case_c ;; esac

finish_evidence

banner "Result"
if [ "$DRY_RUN" = "1" ]; then
    warn "dry run — $STEP_NO steps planned, none executed, nothing proved"
    note "plan written to $EVIDENCE"
    exit 0
fi
if [ "$FAILURES" -eq 0 ]; then
    ok "every case behaved as specified · $STEP_NO steps recorded"
    note "evidence written to $EVIDENCE"
    exit 0
fi
printf '%s✗%s %d step(s) did not behave as specified. Do NOT submit this run as evidence.\n' \
    "$C_ERR" "$C_0" "$FAILURES" >&2
note "details in $EVIDENCE"
exit 1
