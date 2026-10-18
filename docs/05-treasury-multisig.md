# 05 · Treasury and multisig

Modules B1–B5. The treasury is an ordinary Stellar G-account with three signers at weight 1 and
every threshold set to 2. No contract holds the money; Stellar's native multi-signature does the
work, because it has been in production for years and needs no audit from us.

This document covers the account. Building and submitting transactions against it is
[06-signature-service.md](06-signature-service.md).

## Why native multisig rather than a contract

A custody contract would need a security review, an upgrade story, an escape hatch, and an answer to
who is legally responsible for the funds it holds. None of those fit in twenty days, and none of
them are what the SOW asked for. Stellar accounts already support weighted signers and per-operation
thresholds. The gap in the market is not the primitive — it is that nothing wires the primitive into
an approval workflow. That wiring is the deliverable.

## B1 · Creating the treasury

```bash
# Re-runnable from scratch in under ten minutes, with no manual step.
stellar keys generate --fund treasury --network testnet
# Establish the testnet USDC trustline before anything tries to pay in USDC.
stellar tx new change-trust --source treasury --line USDC:$USDC_ISSUER --network testnet
```

Testnet resets, and when it does the deployed contract and the demo data go with it. The script must
rebuild everything from nothing in under ten minutes without a human typing an address. That is a
schedule mitigation, not tidiness — see [16-risks.md](16-risks.md).

**The trustline is a demo-day failure waiting to happen.** Sending USDC to an account without a USDC
trustline fails at the ledger, and it fails *at submit* — after both approvers have signed. The
pre-demo checklist verifies the trustline on the treasury, the vendor address and every test wallet.

### Order is the correctness argument

The trustline is established **before** the master key is disabled, and that ordering is load-bearing
rather than stylistic. `changeTrust` is a medium-threshold operation: once `masterWeight` is 0 and
the thresholds are 2, the treasury key can no longer authorise one on its own. Doing it afterwards
cannot work.

`setup_treasury.sh` reads the treasury's current master weight and picks its path from that, so it
runs correctly against a fresh account *and* against one already locked down — in the second case it
builds the `changeTrust`, collects two of three signatures, and submits, exactly as a payment does.

### Where the USDC comes from

Left unset, the script generates and funds its **own** issuer and issues the treasury a working
balance. That keeps "re-runnable from nothing in under ten minutes, with no manual step" literally
true: Circle's testnet USDC comes from a browser faucet, and a deploy step nobody can script is a
deploy step that fails at the worst moment. The evidence file names the issuer in full and says
plainly that it is not Circle's asset.

Set `USDC_ISSUER` to a real issuer address to use that asset instead. The trustline is then
established against it and no issuance is attempted, because we cannot issue somebody else's asset —
funding the treasury becomes your problem.

### Two names for one asset

The same asset is named two different ways, and conflating them is a real defect rather than a
pedantic distinction:

| | |
|---|---|
| `USDC:GAXU…` | the **classic** payment operation's asset |
| `CBJ4…` | the **Stellar Asset Contract** address, which is what the registry's `asset` field stores |

`setup_treasury.sh` derives the second from the first and records both, so `create_request` cannot
end up describing a payment in an asset the treasury did not send.

## B2 · Three signers, thresholds 2, master weight 0

```ts
const tx = new TransactionBuilder(treasuryAccount, { fee, networkPassphrase })
  .addOperation(Operation.setOptions({ signer: { ed25519PublicKey: S1, weight: 1 } }))
  .addOperation(Operation.setOptions({ signer: { ed25519PublicKey: S2, weight: 1 } }))
  .addOperation(Operation.setOptions({ signer: { ed25519PublicKey: S3, weight: 1 } }))
  .addOperation(Operation.setOptions({
    masterWeight:  0,   // <- MANDATORY. Without it the master key still signs alone.
    lowThreshold:  2,
    medThreshold:  2,   // payment is a medium operation, so this is the one that gates payouts
    highThreshold: 2,   // changing the signer set also needs two people
  }))
  .setTimeout(120)
  .build();
```

| Setting | Value | Why |
|---|---|---|
| Three signers | weight 1 each | Total weight 3, so any two of three reach the threshold |
| `lowThreshold` | 2 | |
| `medThreshold` | 2 | **Payment is a medium operation.** This is the threshold that actually gates a payout |
| `highThreshold` | 2 | Changing the signer configuration needs the same two people |
| `masterWeight` | **0** | Without this, whoever holds the treasury secret key signs alone and the entire approval process is decoration |

**`masterWeight = 0` is the step most often forgotten, and the first thing a reviewer checks.** It
belongs in the B2 script and is asserted again by the B5 script. It is never done by hand.

## B3 · The lock-out guard

Setting a threshold higher than the reachable signer weight locks the account permanently. `setOptions`
and `accountMerge` are high-threshold operations, so once the high threshold is unreachable nobody
can ever change the configuration back. There is no recovery path.

The guard, as a predicate, evaluated against the **new** values before any `setOptions` is submitted:

```
Σ weight(active signers) + masterWeight  ≥  max(lowThreshold, medThreshold, highThreshold)
```

Two refinements that matter:

- **Precision about what is lost.** Payment is a medium operation. If `medThreshold` stays within
  reachable weight but `highThreshold` does not, money still moves — what is lost is administrative
  control of the account. That is a slower disaster, not a smaller one.
- **Do not set the threshold equal to the total weight.** With three signers, a 3-of-3 configuration
  means losing one key loses administrative control forever. The configuration here is 2 of 3
  precisely so one lost key is survivable.

Module B3 has a test that constructs a locking configuration and asserts it is refused (feature
F-07). Every `setOptions` in the codebase routes through the guard; none constructs the operation
directly.

## B4 · Policy and signer enrolment

Signers and thresholds are recorded off chain as well, because the signature service needs to know
whose signature counts for how much before it has anything to submit.

**A policy is versioned, never edited in place.** Changing a signer or a threshold writes a new
`governance_policies` row with `version + 1`; the previous row stays. A settlement pins
`policy_version` at the moment it is authorised, and every signature is checked against **that**
version, not against whatever is current.

This answers a question that otherwise has no answer several months later: *under which rule was this
payment approved?*

It also settles what happens when the policy changes while signatures are being collected. There are
two kinds of revocation and they behave differently:

| Revocation | Effect on signatures already collected |
|---|---|
| In our policy table only | They remain valid on chain — Stellar knows nothing about our internal records |
| On chain via `setOptions` | Weight is evaluated at apply time, so an old signature contributes 0 and the transaction fails `tx_bad_auth` |

Both lead to the same operational conclusion: **when the policy changes, every in-flight settlement
is cancelled and rebuilt.** Pinning the version is what makes that detectable rather than a surprise
at submit time.

## B5 · Verification, which is also evidence

A script reads the account back from Horizon and asserts the configuration. Its output is submitted
as evidence milestone **E2** ([15-evidence.md](15-evidence.md)).

| Assertion | Expected |
|---|---|
| Signer count, excluding the master key | 3 |
| Each signer's weight | 1 |
| `low_threshold` | 2 |
| `med_threshold` | 2 |
| `high_threshold` | 2 |
| Master key weight | **0** |
| USDC trustline present | yes |

It asserts rather than prints. A script that prints the configuration and leaves a human to read it
is a script that passes on demo day while the master key still has weight 1.

## The five traps of native multisig

Read before writing code against the treasury.

**1 · Sequence number contention.** A signature binds tightly to the sequence number. Two payments
taking the same sequence means the second fails `tx_bad_seq` after two people have already signed.
The sequence is locked in the database, and only one payment is in flight per treasury account. See
[06-signature-service.md](06-signature-service.md).

**2 · Missing timebounds.** A transaction with enough signatures but not yet submitted is a signed
cheque. Without `maxTime` it stays usable indefinitely by anyone holding a copy. Always set a narrow
window — one hour — and collect signatures again if it lapses.

**3 · The master key still has weight.** Covered above. It is the first thing a reviewer verifies.

**4 · Revoking a signer mid-flight.** Covered under B4. Both kinds of revocation lead to cancel and
rebuild.

**5 · The destination has no trustline.** Sending USDC to an account without a USDC trustline fails
at the ledger — but only at submit, after both approvers have been interrupted. The destination's
trustline is checked when the request is created and again before the transaction is built, not
after the signatures have been collected.

## The rules this document imposes

- **`masterWeight = 0` is set by script and asserted by script.** Never by hand, never only once.
- **Every `setOptions` passes the B3 guard**, evaluated against the new values.
- **Thresholds never equal total signer weight.** Losing one key must be survivable.
- **A policy is versioned, never edited.** A settlement pins its version at authorisation.
- **A policy change cancels every in-flight settlement.** Both kinds of revocation lead there.
- **The trustline is verified before signatures are requested**, not after.
