# Flows

One diagram per file, drawn **from the shipped code** rather than from the specification.

## Precedence

Where a flow and [`docs/`](../../docs/) disagree, the documentation is the design intent and these
diagrams are what was built — which usually means a flow is out of date. Where a flow and the
source disagree, **the source wins**: these are descriptions, not requirements.

The place to look is named at the bottom of each file.

## Legend

| Shape | Means |
|---|---|
| Rectangle | A state write — something durable changed |
| Diamond | A gate. The refusing edge is labelled with the error code |
| Rounded | An external system: a wallet, Horizon, the ledger |
| Dashed edge | A path back to an earlier node — a readback loop, never a resubmit loop |

## Index

| # | Flow | Who starts it | File |
|---|---|---|---|
| 00 | The lifecycle in one picture | — | [00-lifecycle.md](00-lifecycle.md) |
| 01 | Wallet sign-in | A person | [01-sign-in.md](01-sign-in.md) |
| 02 | Creating a payment | The business | [02-create-request.md](02-create-request.md) |
| 03 | Collecting signatures | An approver | [03-collect-signatures.md](03-collect-signatures.md) |
| 04 | Submitting, and what happens when the answer is unclear | The submitter loop | [04-submit-and-reconcile.md](04-submit-and-reconcile.md) |
| 05 | Rejecting | An approver | [05-reject.md](05-reject.md) |
