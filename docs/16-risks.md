# 16 · Risks and open questions

Six risks that could cost the twenty days, each with a mitigation that is a piece of work rather
than an intention. Three questions that must be answered before Day 1.

Where the mitigations land in the schedule is [14-roadmap.md](14-roadmap.md).

## Six risks

| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| **Sequence number contention** | High | Collected signatures are voided, the work is redone, and confidence is lost during the demo itself | Lock the sequence in the database on **Day 9**, with a concurrency test. Do not let this surface on Day 14 |
| **Scope creep** | High | The three committed deliverables do not land | The out-of-scope list in [00-overview.md](00-overview.md) is a hard boundary. A pull request touching it is closed immediately |
| **Forgetting `masterWeight = 0`** | Medium | Multisig becomes decoration — whoever holds the treasury key signs alone. A reviewer will find this | It goes in the B2 script and the B5 script asserts it back. Never done by hand |
| **Testnet reset or instability** | Medium | The deployed contract and the demo data are lost | The deploy script rebuilds everything from scratch in under ten minutes with no manual step |
| **Soroban SDK version change** | Medium | The contract stops building; one to two days lost | Pin the version in `Cargo.lock` on Day 1. No updates mid-stage |
| **Testnet USDC trustline** | Medium | The payment fails during the demo, in front of the reviewer | Pre-demo checklist: treasury, vendor address and every test wallet all hold the trustline |

Two of these have the same shape and are worth noticing together. **Sequence contention** and
**`masterWeight`** both fail silently under normal single-threaded, single-operator use and only
appear under the conditions of a demo — concurrency and scrutiny. Neither is caught by testing the
happy path by hand, which is why both have a specific automated check attached rather than a
reminder.

## Three questions to settle before Day 1

### 01 · Who is the settler?

Which address may call `approve` and `mark_paid` — a dedicated backend key, or the treasury account
itself?

This directly shapes the contract's authority design. A dedicated backend key means the registry
trusts one service, and compromising that service lets someone mark requests paid that were never
paid — bad, but no money moves, because the registry holds none. Using the treasury account means the
same two signatures that authorise the payment also authorise the status change, which is tighter but
puts `approve` and `mark_paid` behind the multisig and therefore behind a signing round each.

The design in [04-contract-interface.md](04-contract-interface.md) assumes a dedicated backend key.
If that changes, `approve` and `mark_paid` become multi-signature operations and the flow in
[03-flow.md](03-flow.md) gains two signing rounds.

### 02 · Is the contract upgradeable?

If deployment is immutable, a bug found after Day 20 means deploying a new contract and migrating
every reference to it. If it is upgradeable, somebody holds that power and that person is now the
system's largest single point of trust.

For a registry that holds no funds, immutable is defensible: the worst outcome is a stale registry
that gets replaced. The same question for a custody contract would have a different answer, which is
part of why custody is out of scope.

### 03 · Is the reference stored as a hash or as plaintext?

This specification stores a hash on chain and keeps the plaintext in Postgres.

The reference is a purchase order or invoice number. Written to the ledger in plaintext it is
permanently public and permanently linked to a vendor address and an amount — which is to say, the
business's supplier relationships and payment sizes become readable by anyone. The hash is enough to
prove correspondence during reconciliation without publishing the commercial relationship.

If the decision goes the other way, `PaymentRequest.reference` changes type and
[08-data-model.md](08-data-model.md) loses the `reference_hash` column.

## After this Instaward

SOW section 7.1 anticipates applying for a follow-on Instaward. The registry and the approval
mechanism built in these twenty days are the foundation that dashboards, reconciliation and a vendor
portal build directly on top of. In SettleFlow's six-month roadmap, this is month one.

The three questions the full product must answer before any custody scope opens — none of them
technical:

1. **Upgrade.** Who may upgrade a contract that holds funds, and if nobody may, how is a bug patched?
2. **Escape.** If the contract locks up, how does a business recover its money — and is that
   mechanism not itself the vulnerability?
3. **Liability.** Who is legally responsible for money the contract holds, and how does that change
   KYB and AML obligations in each market?

## SOW 1 on one page

| Building | Not building | Accepted by |
|---|---|---|
| A payment request registry, no custody | A full dashboard | A public repository |
| A treasury with 3 signers, 2 required | A vendor portal | A testnet contract id |
| A service that builds, collects and submits | A reconciliation indexer | Seven transaction hashes |
| Four screens, enough to demonstrate | Accounting export | A 3–5 minute demo video |
| Testnet deployment and evidence | A security audit | A deploy guide that re-runs |
| | Mainnet | |

| 13 | 42 | 30 | 4 | 7 | 6 |
|---|---|---|---|---|---|
| features | modules in 7 groups | flow steps | screens | evidence milestones | risks with mitigations |

## The closing line

Stellar has had native multi-signature in production for years, and nobody has wired it into a real
payment approval process. SOW 1 builds that layer in twenty days, does not touch custody, and
delivers evidence anyone can verify on a public blockchain.
