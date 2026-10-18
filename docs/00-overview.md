# 00 · Overview

What SOW 1 builds, what it refuses to build, and the constraints the Instaward imposes.

This document does not describe how anything works. Features are [01-features.md](01-features.md),
the work breakdown is [02-modules.md](02-modules.md), and the contract surface is
[04-contract-interface.md](04-contract-interface.md).

## The problem

Companies holding stablecoin treasury on Stellar have no purpose-built way to pay vendors with
internal approval control.

Zebec already covers payroll on Stellar. COINQVEST already covers merchant checkout. Neither handles
accounts payable — paying suppliers with a budget approval step before funds move. Teams are left
with two bad options: send payments straight from a wallet with no approval trail, or fall back on a
bank-focused tool like Bill.com that cannot touch stablecoins at all.

Stellar accounts already support native multi-signature with weighted signers and thresholds. It is
a mature primitive with years of production use. Nothing wires it into an actual payment-request
workflow. SOW 1 builds exactly that missing layer, and deliberately does not touch custody — which
is what makes twenty days without a security audit an honest schedule rather than a hopeful one.

## Objective

At the end of twenty working days: a Payment Request Registry contract deployed on Stellar testnet,
alongside a treasury account configured with three weighted signers and a two-signer threshold. A
payment request created against the registry can be approved by two of the three signers and paid
out to a vendor address, with the registry marking the request paid once the transaction settles.

## The three deliverables

| | Share | What |
|---|---|---|
| **1** | 35% · $1,750 | A Soroban contract that creates a payment request carrying vendor address, amount, asset and reference, and tracks it through `pending → approved → paid`. The contract never holds funds — it records and verifies status. |
| **2** | 35% · $1,750 | A backend service that builds an unsigned Stellar transaction from an approved request, receives each approver's signature, and submits once the total weight reaches the treasury account's threshold. |
| **3** | 30% · $1,500 | Registry and a three-signer treasury deployed on testnet. A demonstrated request collecting two of three signatures, paying a vendor address, and the registry marking it paid — with verifiable transaction hashes. |

## Scope

The single largest risk to a twenty-day Instaward is scope creep. The right-hand column is a hard
boundary: a pull request that touches it is closed, even when it "only takes an afternoon".

### In scope

- A payment request registry contract, no custody.
- The lifecycle `pending → approved → paid`, plus the `cancelled` branch.
- A treasury G-account: three signers at weight 1, threshold 2.
- Building an unsigned transaction from an approved request.
- Collecting each approver's signature, checking the threshold, submitting.
- Reading the chain back to confirm before writing `paid`.
- A minimum interface — enough to demonstrate the deliverables.
- Testnet deployment, a demonstrated round trip, and the evidence package.

### Out of scope — quoted from the SOW

| Not built | |
|---|---|
| A full dashboard | Only the minimum screens the demo needs |
| Vendor-facing views | No supplier portal |
| A reconciliation indexer | No separate reconciliation service |
| Accounting export | No accounting data output |
| A security audit | Not commissioned within these 30 days |
| Mainnet deployment | Testnet only |

Four more, added to protect the schedule rather than because the SOW named them: **no escrow holding
funds, no fiat off-ramp, no internationalisation, no mobile application.**

## Instaward constraints

| | Constraint | Meaning |
|---|---|---|
| 01 | ≤ 30 days | The whole scope completes within 30 calendar days, about 20 working days |
| 02 | Execution, not exploration | Instawards fund something that runs, not open-ended research |
| 03 | At most 2 follow-ons | A project receives no more than two subsequent Instawards |
| 04 | $5,000 / $15,000 ceiling | Each Instaward caps at $5,000; the cumulative total may not exceed $15,000 |

## One thing to agree before Day 1

The SOW text describes paying vendors — accounts payable. The commercial SettleFlow product develops
in the opposite direction, collecting from customers. **Technically the registry and the approval
mechanism are the same thing for both directions**, so the code delivered to the Instaward remains
correct either way.

This documentation follows the SOW text. Everything below describes vendor payout.

## Assets

| | USDC | XLM |
|---|---|---|
| Form | Stellar Asset Contract | Native |
| Decimals | 7 | 7 |
| Amount tolerance | 0 — exact only | 0 — exact only |
| Trustline required on the destination | Yes | No |
| Minimum balance reserve | — | 1 XLM base, plus 0.5 per subentry |
| Transaction fee paid in | XLM | XLM |
| Confirmation depth | 1 ledger, about 5 s | 1 ledger, about 5 s |
| Reorg risk | None — SCP does not fork | None |

Two consequences that have produced real incidents in systems of this shape:

- **Fees are always XLM, even when paying USDC.** A treasury holding a million USDC and 0.9 XLM
  passes every balance check and then fails at the ledger — after two approvers have already signed.
  The asset check and the fee check are independent gates.
- **A wallet holding 10 XLM cannot send 10 XLM.** The minimum balance reserve is not spendable.

Readback is still mandatory despite the absence of reorg risk. The reason is infrastructural — a
lost HTTP response, a dropped connection — not consensual.

## Trust model

| Party | Trusted for | Not trusted for |
|---|---|---|
| The business | Creating and cancelling its own requests | Marking its own request paid — only the settler may |
| An approver | Contributing signature weight | Anything else; a signature is worth its weight and nothing more |
| Our backend | Building transactions, collecting signatures, reading the chain | Being the source of truth for a request's status |
| The wallet | Holding keys and signing | Reporting outcomes; an optimistic response is not a confirmation |
| Stellar | Finality, and the definition of "now" for expiry | Being reachable — every call needs an unresolved branch |

## The rules this build follows

- **The registry never moves a token.** Enforced by a grep gate in CI, not by memory. This is the
  first item on the Day 4 review checklist.
- **Chain is the source of truth.** `paid` is written only after a readback. A successful submit is
  not a confirmed payment.
- **Unknown is a state, not an error.** `tx_bad_seq` and a timeout are handled identically: read the
  chain first, resubmit the same envelope, rebuild only when certain nothing landed.
- **Ledger time decides expiry.** `env.ledger().timestamp()`, never a server clock.
- **Money is never a float.** `numeric(30,7)` in Postgres, `i128` stroops in Rust, `String` on the
  wire — JSON numbers are IEEE doubles.
- **Every chain-touching endpoint returns 202**, not 200, so no client can read "API OK" as "money
  moved".
- **Colour is never the only carrier.** Every status pill also carries text.
