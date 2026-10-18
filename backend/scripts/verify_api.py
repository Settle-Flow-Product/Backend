"""
Every path through the API, end to end, against a running stack.

This exists because the alternative was finding these one at a time by clicking. Each bug that
reached a person — an asset with no contract, a request signed before it existed on chain, a
settlement that held the treasury for ever — was reachable from the API and would have been caught
here on the first run.

It talks HTTP to a real backend and signs with real ed25519, so it exercises the shipped path
rather than a mock of it. It needs the stack up and the policy enrolled:

    make demo-up
    make verify-api

Cases are grouped, and every one states what it PROVES rather than what it calls. A case that
cannot say what would be wrong if it failed is not worth running.
"""
import json
import os
import sys
import time
import urllib.error
import urllib.request
import uuid

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import _stellar_crypto as ed  # noqa: E402

BASE = os.environ.get("BASE", "http://localhost:8088")
VENDOR = os.environ.get("VENDOR", "")
SIGNER_SECRETS = [s for s in os.environ.get("SIGNER_SECRETS", "").split(",") if s]

TTY = sys.stdout.isatty()
OK, ERR, DIM, BOLD, RESET = (
    ("\033[32m", "\033[31m", "\033[2m", "\033[1m", "\033[0m") if TTY else ("",) * 5
)

passed, failed, skipped = 0, 0, 0


def section(title):
    print(f"\n{BOLD}══ {title}{RESET}")


def check(name, proves, condition, detail=""):
    global passed, failed
    if condition:
        passed += 1
        print(f"  {OK}✓{RESET} {name}")
    else:
        failed += 1
        print(f"  {ERR}✗{RESET} {name}")
        print(f"      proves: {proves}")
        if detail:
            print(f"      got:    {detail}")


def skip(name, why):
    global skipped
    skipped += 1
    print(f"  {DIM}· {name} — {why}{RESET}")


def call(path, method="GET", payload=None, token=None, headers=None):
    hdr = {"content-type": "application/json"}
    if token:
        hdr["authorization"] = f"Bearer {token}"
    if headers:
        hdr.update(headers)
    data = json.dumps(payload).encode() if payload is not None else None
    req = urllib.request.Request(BASE + path, method=method, data=data, headers=hdr)
    try:
        with urllib.request.urlopen(req, timeout=90) as r:
            return r.status, json.loads(r.read() or b"null")
    except urllib.error.HTTPError as e:
        try:
            return e.code, json.loads(e.read() or b"null")
        except json.JSONDecodeError:
            return e.code, None
    except urllib.error.URLError as e:
        return 0, {"error": {"code": "UNREACHABLE", "message": str(e)}}


def code_of(body):
    return (body or {}).get("error", {}).get("code", "")


def sign_in(seed_material):
    """A full SEP-10 round trip, returning (token, address)."""
    seed = ed.hashlib.sha256(seed_material).digest()
    address = ed.strkey(6 << 3, ed.public_key(seed))
    status, ch = call("/auth/wallet/challenge", "POST", {"address": address})
    if status != 200:
        return None, address
    signed = ed.sign_challenge(seed, ch["transaction_xdr"], ch["network_passphrase"])
    status, out = call(
        "/auth/wallet/verify", "POST", {"nonce": ch["nonce"], "signed_xdr": signed}
    )
    return (out.get("token") if status == 200 else None), address


# ---------------------------------------------------------------------------
section("reachability")
# ---------------------------------------------------------------------------
status, _ = call("/health")
check("the backend answers", "nothing below can run against a dead service", status == 200)
if status != 200:
    print(f"\n{ERR}✗ {BASE} is not answering. Run `make demo-up`.{RESET}")
    sys.exit(1)

status, _ = call("/ready")
check("the database answers", "readiness reflects the database, not just the process", status == 200)

# ---------------------------------------------------------------------------
section("authentication")
# ---------------------------------------------------------------------------
status, body = call("/requests")
check(
    "an unauthenticated read is refused",
    "session enforcement, not merely a login page",
    status == 401,
    f"{status} {code_of(body)}",
)

token, wallet = sign_in(b"settleflow verify_api primary")
check("a wallet can sign in over SEP-10", "the shipped auth path works with a real signature", token is not None)
if not token:
    print(f"\n{ERR}✗ cannot continue without a session{RESET}")
    sys.exit(1)

status, ch = call("/auth/wallet/challenge", "POST", {"address": wallet})
seed = ed.hashlib.sha256(b"settleflow verify_api primary").digest()
signed = ed.sign_challenge(seed, ch["transaction_xdr"], ch["network_passphrase"])
call("/auth/wallet/verify", "POST", {"nonce": ch["nonce"], "signed_xdr": signed})
status, body = call("/auth/wallet/verify", "POST", {"nonce": ch["nonce"], "signed_xdr": signed})
check(
    "a replayed nonce is refused",
    "a captured challenge cannot be reused to impersonate a wallet",
    status == 401,
    f"{status} {code_of(body)}",
)

# ---------------------------------------------------------------------------
section("destination preflight")
# ---------------------------------------------------------------------------
status, body = call("/accounts/nonsense/can-receive?asset=USDC", token=token)
check(
    "a malformed address is refused at the field",
    "the error names the field rather than the network",
    status == 400 and code_of(body) == "INVALID_ADDRESS",
    f"{status} {code_of(body)}",
)

status, body = call(f"/accounts/{wallet}/can-receive?asset=USDC", token=token)
check(
    "an unfunded account reports exists=false",
    "a vendor that does not exist is caught before anybody signs",
    status == 200 and body.get("exists") is False,
    json.dumps(body),
)

status, body = call(f"/accounts/{wallet}/can-receive?asset=DOGE", token=token)
check(
    "an unsupported asset is refused",
    "only the assets this deployment can actually pay are accepted",
    status == 400,
    f"{status} {code_of(body)}",
)

if VENDOR:
    status, body = call(f"/accounts/{VENDOR}/can-receive?asset=USDC", token=token)
    check(
        "the demo vendor can receive USDC",
        "the trustline exists, so a USDC demo will not fail at the ledger",
        status == 200 and body.get("can_receive") is True,
        json.dumps(body),
    )
else:
    skip("the demo vendor can receive USDC", "VENDOR not set")

# ---------------------------------------------------------------------------
section("creating a payment · validation")
# ---------------------------------------------------------------------------
def create(body, key=None, token_=None):
    return call(
        "/requests",
        "POST",
        body,
        token=token_ or token,
        headers={"idempotency-key": key or str(uuid.uuid4())},
    )


good = {
    "payee_address": VENDOR or wallet,
    "amount": "1.5",
    "asset_code": "USDC",
    "reference": "PO-VERIFY-BASE",
}

status, body = call("/requests", "POST", good, token=token)
check(
    "a create without an idempotency key is refused",
    "retry safety is mandatory, not optional",
    status == 400 and code_of(body) == "IDEMPOTENCY_KEY_REQUIRED",
    f"{status} {code_of(body)}",
)

for label, patch, proves in [
    ("a bad vendor address", {"payee_address": "GXXX"}, "the address is validated server-side, not only in the form"),
    ("a zero amount", {"amount": "0"}, "an amount of zero has no business meaning"),
    ("a negative amount", {"amount": "-5"}, "a negative payment is a withdrawal nobody authorised"),
    ("an eighth decimal place", {"amount": "1.12345678"}, "stroops have seven decimals; more is a rounding decision"),
    ("an empty reference", {"reference": ""}, "every payment is traceable to a business document"),
    ("an unsupported asset", {"asset_code": "DOGE"}, "only assets this deployment can pay are accepted"),
]:
    b = dict(good)
    b.update(patch)
    status, body = create(b)
    check(f"{label} is refused", proves, status in (400, 422), f"{status} {code_of(body)}")

# ---------------------------------------------------------------------------
section("creating a payment · idempotency")
# ---------------------------------------------------------------------------
key = str(uuid.uuid4())
first = dict(good)
first["reference"] = f"PO-VERIFY-IDEM-{int(time.time())}"
s1, b1 = create(first, key=key)
s2, b2 = create(first, key=key)
check(
    "the same key twice yields one payment",
    "a retry after a timeout cannot create a second payment",
    s1 == 202 and b1.get("request_id") == (b2 or {}).get("request_id"),
    f"{s1}/{s2} {b1.get('request_id')} vs {(b2 or {}).get('request_id')}",
)

changed = dict(first)
changed["amount"] = "999"
s3, b3 = create(changed, key=key)
check(
    "the same key with a different body is refused",
    "an idempotency key cannot be reused to smuggle different terms",
    s3 == 409 and code_of(b3) == "IDEMPOTENCY_CONFLICT",
    f"{s3} {code_of(b3)}",
)

# ---------------------------------------------------------------------------
section("both assets reach the chain")
# ---------------------------------------------------------------------------
# The bug this catches: XLM had no asset contract configured, so `create_request` failed on a
# missing argument and the payment never reached the registry — while the interface showed Pending.
def wait_on_chain(rid, seconds=90):
    for _ in range(seconds // 3):
        time.sleep(3)
        _, d = call(f"/requests/{rid}", token=token)
        r = (d or {}).get("request", {})
        if r.get("create_tx_hash"):
            return r["create_tx_hash"]
        if r.get("status") in ("cancelled", "failed"):
            return None
    return None


on_chain = {}
for asset in ("USDC", "XLM"):
    b = dict(good)
    b["asset_code"] = asset
    b["reference"] = f"PO-VERIFY-{asset}-{int(time.time())}"
    status, body = create(b)
    if status != 202:
        check(f"a {asset} payment is accepted", "both supported assets can be paid", False, f"{status} {code_of(body)}")
        continue
    rid = body["request_id"]
    tx = wait_on_chain(rid)
    on_chain[asset] = (rid, tx)
    check(
        f"a {asset} payment reaches the registry",
        "the asset has a contract configured, so the chain write can succeed",
        tx is not None,
        "never got a create_tx_hash",
    )

# ---------------------------------------------------------------------------
section("preparing a payment")
# ---------------------------------------------------------------------------
b = dict(good)
b["reference"] = f"PO-VERIFY-EARLY-{int(time.time())}"
status, body = create(b)
early_rid = body.get("request_id") if status == 202 else None
if early_rid:
    status, body = call(f"/requests/{early_rid}/prepare", "POST", token=token)
    check(
        "preparing before the chain write is refused",
        "nobody signs a payment for a request the registry has never seen",
        status == 409 and code_of(body) == "NOT_ON_CHAIN_YET",
        f"{status} {code_of(body)}",
    )
else:
    skip("preparing before the chain write is refused", "could not create a request")

usdc_rid, usdc_tx = on_chain.get("USDC", (None, None))
prepared = None
if usdc_rid and usdc_tx:
    status, prepared = call(f"/requests/{usdc_rid}/prepare", "POST", token=token)
    check(
        "a request on chain can be prepared",
        "the unsigned transaction and the signer roster are available to approvers",
        status == 200 and prepared.get("tx_hash"),
        f"{status} {code_of(prepared)}",
    )
    if status == 200:
        check(
            "the roster comes from the enrolled policy",
            "the approver screen can show who may sign",
            len(prepared.get("signers", [])) >= 2 and prepared.get("required_weight", 0) >= 2,
            json.dumps({k: prepared.get(k) for k in ("required_weight", "collected_weight")}),
        )
        check(
            "the treasury is named as the paying account",
            "the approver sees which account the money leaves, not who raised it",
            bool(prepared.get("treasury_address")),
        )
else:
    skip("a request on chain can be prepared", "no USDC request reached the chain")

xlm_rid, xlm_tx = on_chain.get("XLM", (None, None))
if xlm_rid and xlm_tx and prepared:
    status, body = call(f"/requests/{xlm_rid}/prepare", "POST", token=token)
    check(
        "a second payment from the same treasury is refused",
        "one payment in flight per treasury, refused cleanly rather than dying at tx_bad_seq",
        status == 409 and code_of(body) == "RELEASE_IN_FLIGHT",
        f"{status} {code_of(body)}",
    )
else:
    skip("a second payment from the same treasury is refused", "needs two on-chain requests")

# ---------------------------------------------------------------------------
section("collecting signatures")
# ---------------------------------------------------------------------------
if not (prepared and prepared.get("tx_hash")):
    skip("signature cases", "nothing prepared to sign")
elif not SIGNER_SECRETS:
    skip("signature cases", "SIGNER_SECRETS not set")
else:
    tx_hash = prepared["tx_hash"]

    status, body = call(
        f"/requests/{usdc_rid}/signatures",
        "POST",
        {"signer_address": wallet, "signature": "aa" * 64},
        token=token,
    )
    check(
        "a wallet outside the policy cannot sign",
        "only enrolled approvers contribute weight",
        status in (403, 422) and code_of(body) in ("NOT_A_SIGNER", "BAD_SIGNATURE"),
        f"{status} {code_of(body)}",
    )

    first_secret = SIGNER_SECRETS[0]
    seed0 = ed.secret_to_seed(first_secret)
    addr0 = ed.strkey(6 << 3, ed.public_key(seed0))
    status, body = call(
        f"/requests/{usdc_rid}/signatures",
        "POST",
        {"signer_address": addr0, "signature": "bb" * 64},
        token=token,
    )
    check(
        "a wrong signature is refused",
        "the signature is verified against the transaction hash, not merely stored",
        status in (400, 422),
        f"{status} {code_of(body)}",
    )

    progress = []
    for i, secret in enumerate(SIGNER_SECRETS[:2]):
        s = ed.secret_to_seed(secret)
        a = ed.strkey(6 << 3, ed.public_key(s))
        sig = ed.sign(s, bytes.fromhex(tx_hash))
        status, body = call(
            f"/requests/{usdc_rid}/signatures",
            "POST",
            {"signer_address": a, "signature": sig.hex()},
            token=token,
        )
        progress.append((status, body))
        if i == 0:
            check(
                "the first signature is accepted but below threshold",
                "one approver is not enough — the threshold is real",
                status == 200 and body.get("threshold_met") is False,
                f"{status} {json.dumps(body)}",
            )
            status, dup = call(
                f"/requests/{usdc_rid}/signatures",
                "POST",
                {"signer_address": a, "signature": sig.hex()},
                token=token,
            )
            check(
                "the same signer cannot sign twice",
                "one approver cannot reach the threshold alone by signing again",
                status in (409, 422) and code_of(dup) == "ALREADY_SIGNED",
                f"{status} {code_of(dup)}",
            )
        else:
            check(
                "the second signature meets the threshold",
                "2 of 3 releases the payment, and only then",
                status == 200 and body.get("threshold_met") is True,
                f"{status} {json.dumps(body)}",
            )

# ---------------------------------------------------------------------------
section("settlement and evidence")
# ---------------------------------------------------------------------------
if usdc_rid and SIGNER_SECRETS and prepared:
    final = None
    for _ in range(40):
        time.sleep(4)
        _, d = call(f"/requests/{usdc_rid}", token=token)
        st = (d or {}).get("request", {}).get("status")
        if st in ("paid", "cancelled"):
            final = st
            break
    check(
        "the payment settles",
        "the whole path completes without anybody touching the database",
        final == "paid",
        f"final status {final}",
    )

    status, ev = call(f"/requests/{usdc_rid}/evidence", token=token)
    milestones = {m["milestone"]: m["value"] for m in (ev or {}).get("milestones", [])}
    check(
        "every evidence milestone has a value",
        "a reviewer can verify all six against the public ledger",
        status == 200 and all(milestones.get(m) for m in ("E1", "E2", "E3", "E4", "E5", "E6")),
        json.dumps({k: bool(v) for k, v in milestones.items()}),
    )
    for m in ("E3", "E4", "E5", "E6"):
        v = milestones.get(m) or ""
        check(
            f"{m} is a transaction hash",
            "the evidence page links to the chain rather than restating our own records",
            len(v) == 64 and all(c in "0123456789abcdef" for c in v),
            f"{m} = {v!r}",
        )
else:
    skip("settlement and evidence", "no signed payment to settle")

# ---------------------------------------------------------------------------
section("the reject branch")
# ---------------------------------------------------------------------------
if SIGNER_SECRETS and xlm_rid and xlm_tx:
    status, prep = call(f"/requests/{xlm_rid}/prepare", "POST", token=token)
    if status == 200:
        s = ed.secret_to_seed(SIGNER_SECRETS[0])
        a = ed.strkey(6 << 3, ed.public_key(s))
        status, body = call(
            f"/requests/{xlm_rid}/reject", "POST", {"signer_address": a}, token=token
        )
        check(
            "an approver can reject",
            "the reject branch is reachable, not merely designed",
            status in (200, 202),
            f"{status} {code_of(body)}",
        )
        cancelled = False
        for _ in range(20):
            time.sleep(3)
            _, d = call(f"/requests/{xlm_rid}", token=token)
            if (d or {}).get("request", {}).get("status") == "cancelled":
                cancelled = True
                break
        check(
            "a rejected payment reaches Cancelled",
            "rejection stops the payment for good rather than pausing it",
            cancelled,
        )
    else:
        skip("the reject branch", f"could not prepare ({status} {code_of(prep)})")
else:
    skip("the reject branch", "needs a second on-chain request and signer secrets")

# ---------------------------------------------------------------------------
print(f"\n{BOLD}══ result{RESET}")
print(f"  {OK}{passed} passed{RESET}   {ERR if failed else DIM}{failed} failed{RESET}   {DIM}{skipped} skipped{RESET}")
sys.exit(1 if failed else 0)
