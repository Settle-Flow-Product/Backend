# 10 · Frontend

Four screens, and no more. The SOW puts a dashboard out of scope. Each screen exists because
something in the evidence package needs it, not because it looks finished.

Modules E1–E5. The API they consume is [09-api-contract.md](09-api-contract.md).

## The application

Two Next.js App Router applications:

| App | Port | Audience | How it reads data |
|---|---|---|---|
| [`frontend/website`](../frontend/website/) | 5173 | The people who create and approve payments | The backend API, through a same-origin proxy |
| [`frontend/admin`](../frontend/admin/) | 5174 | Operations — looking at what the system did | **Postgres directly**, through server actions |

The split follows the trust boundary rather than taste. The website is wallet-facing and holds no
credential of its own; the admin console is server-owned, never leaves the operator's network, and
has no business making a person sign anything. Giving them one codebase would mean one bundle that
both talks to wallets and holds a database password.

The four screens below are the **website**. The admin console is [11-admin.md](11-admin.md).

**Next.js 15 + TypeScript + Tailwind v4 + shadcn/ui.** The shadcn primitives live in
`components/ui/` as source the project owns rather than a dependency it upgrades — which is what
shadcn is for, and what lets a `Badge` grow an `ok` / `warn` / `err` / `info` variant that means
something in this domain. Variants come from `cva`, merged through `cn()`.

There is no `tailwind.config.js`: Tailwind v4 is CSS-first, and the token contract lives in
`app/globals.css`.

| Route | Screen | Module |
|---|---|---|
| `/` | Wallet sign-in | E1 |
| `/requests/new` | Create a payment | E2 |
| `/requests` | The request list | E3 |
| `/approve/:id` | Approver signing | E4 |
| `/evidence/:reference` | On-chain evidence | E5 |

---

## E1 · Wallet sign-in

Connect a Stellar wallet, sign the nonce challenge, open an internal session.

- The wallet kit loads on first use, not at page load — it is the largest dependency in the bundle
  and most page loads do not need it.
- The challenge is a **SEP-10 transaction with sequence number 0**, so it can never be submitted.
  The screen says so plainly, near the button — a person approving a transaction deserves to know
  why it is safe.
- The wallet selection is kept alongside the session token, because the kit holds it in memory only
  and a page refresh otherwise loses which wallet to re-open.
- Every failure — wrong signature, unknown address, lapsed challenge — shows the same text after the
  same delay.

## E2 · Create a payment · `/requests/new`

The four fields the SOW names: vendor address, amount, asset, reference.

```
  ┌─────────────────────────────────────────────────┐
  │  Create payment                        Testnet  │
  ├─────────────────────────────────────────────────┤
  │  VENDOR ADDRESS                                 │
  │  GDX7…P4LM                                      │
  │  ✓ Valid address · USDC trustline present       │
  │                                                 │
  │  AMOUNT                    ASSET                │
  │  4,500.0000000             USDC                 │
  │                                                 │
  │  REFERENCE                                      │
  │  PO-2026-0417                                   │
  │                                                 │
  │  Approval required            2 / 3 signatures  │
  │  Written to                          CBIE…9K2Q  │
  │                                                 │
  │             [ Create payment ]                  │
  └─────────────────────────────────────────────────┘
```

| Spec | |
|---|---|
| **Validate at the field** | The vendor address is checked against `^G[A-Z2-7]{55}$`, then after a 500 ms debounce an API call confirms the account exists and holds the trustline. The button stays disabled until the ✓ appears |
| **Amount** | A text input, never `<input type="number">`. At most seven decimal places. Rendered with tabular figures |
| **Idempotency** | A UUID v4 is generated when the form opens and sent as `Idempotency-Key`. Clicking twice creates one payment |
| **After the click** | The screen moves to "writing to chain". **Success is not shown until the readback returns Pending** — step S-10 |

Checking the trustline here rather than at submit time is what stops a payment failing at the ledger
after two people have already signed it. It costs one API call — `GET /accounts/{address}/can-receive`
— and saves a demo.

One deliberate exception to "disabled until the ✓ appears": if the check cannot reach the network at
all, the button re-enables on the local shape check alone. An unreachable Horizon is not evidence
that the address is wrong, and letting it freeze the form would turn a degraded network into an
unusable one — the payment would simply fail later, which is exactly where it failed before this
check existed.

## E3 · The request list · `/requests`

```
  ┌─────────────────────────────────────────────────┐
  │  Payments                             4 items   │
  ├─────────────────────────────────────────────────┤
  │  PO-2026-0417   GDX7…P4LM · 4,500 USDC          │
  │  ● Pending                    1/2 signatures    │
  ├─────────────────────────────────────────────────┤
  │  PO-2026-0416   GBQK…4T7M · 1,200 USDC          │
  │  ● Approved                       submitting    │
  ├─────────────────────────────────────────────────┤
  │  PO-2026-0412   GA5Z…8H3N ·   890 USDC          │
  │  ● Paid                        a91f…2c8d  ↗     │
  ├─────────────────────────────────────────────────┤
  │  PO-2026-0409   GCTY…1W9P · 3,000 USDC          │
  │  ● Cancelled                 rejected by CFO    │
  └─────────────────────────────────────────────────┘
```

| Spec | |
|---|---|
| **Four statuses, four colours** | Pending amber, Approved blue, Paid green, Cancelled red — **always with the word**. Colour is never the only carrier |
| **The second line** | Pending shows signature progress; Paid shows a shortened transaction hash linking to Stellar Expert; Cancelled shows who rejected |
| **Data source** | The read-model. No synchronous Horizon call. If sync lag exceeds 60 seconds, show a "synchronising" banner and disable the create button |
| **Nothing else** | No charts, no statistics, no export. That is a dashboard, and it is out of scope |

## E4 · Approver signing · `/approve/:id`

```
  ┌─────────────────────────────────────────────────┐
  │  Approve payment            Expires in 47:12    │
  ├─────────────────────────────────────────────────┤
  │  PAY TO VENDOR                                  │
  │                                                 │
  │       4,500.00 USDC                             │
  │       GDX7…P4LM                                 │
  │                                                 │
  │  Reference           PO-2026-0417               │
  │  Created by          Head of Finance            │
  │  From account        GTRE…5X8B                  │
  ├─────────────────────────────────────────────────┤
  │  SIGNATURE PROGRESS · 2 OF 3 REQUIRED           │
  │  CFO                            ✓ Signed        │
  │  You (CEO)                        Waiting       │
  │  Internal control              Not needed       │
  ├─────────────────────────────────────────────────┤
  │      [ Sign in wallet ]      [ Reject ]         │
  └─────────────────────────────────────────────────┘
```

| Spec | |
|---|---|
| **Three things largest** | Amount, vendor address, reference. An approver must confirm all three within five seconds without scrolling |
| **The countdown** | It is the **transaction's** timebounds — one hour — not the request's expiry. At zero the sign button disables and the screen says the transaction must be rebuilt |
| **Reject** | Visually equal to Sign, never hidden. One confirmation step, then `POST /requests/:id/reject` |
| **After signing** | Below threshold → the waiting screen showing 1 of 2 (step S-23). At threshold → "submitting transaction". **Never "paid"** |

The countdown distinction matters and is easy to get wrong. A request may be valid for days while
the transaction built from it is valid for an hour. Showing the wrong one lets an approver believe
they have time they do not have.

## E5 · Evidence · `/evidence/:reference`

```
  ┌─────────────────────────────────────────────────┐
  │  On-chain evidence                   Complete   │
  ├─────────────────────────────────────────────────┤
  │  ✓  Deploy registry contract        CBIE…9K2Q ↗ │
  │  ✓  Treasury · 3 signers · thr 2    GTRE…5X8B ↗ │
  │  ✓  create_request                  3f9a…7b1e ↗ │
  │  ✓  approve · 2 of 3 signatures     c204…88af ↗ │
  │  ✓  Payment to vendor               a91f…2c8d ↗ │
  │  ✓  mark_paid                       de55…1a09 ↗ │
  ├─────────────────────────────────────────────────┤
  │            [ Copy all hashes ]                  │
  └─────────────────────────────────────────────────┘
```

**This is the most important screen in SOW 1.** The Chapter Lead accepts the work by opening it. Six
rows correspond to the six mandatory milestones in [15-evidence.md](15-evidence.md).

| Spec | |
|---|---|
| **Every hash is a link** | To Stellar Expert testnet, in a new tab. Shortened first-4–last-4, full value on hover |
| **Copy button** | Emits plain text in table form, ready to paste into the Instawards submission form |
| **Missing rows** | A grey ○ instead of a ✓, so a glance shows what evidence is still absent |

## Design tokens

Two families, kept apart on purpose. The shadcn contract (`--background`, `--foreground`,
`--primary`, `--border`, `--ring`, …) styles the furniture and has a light and a dark value. The
four below carry **meaning**, and each keeps one meaning everywhere it appears.

| Token | Means | Statuses |
|---|---|---|
| `--ok` | done | Paid · settled · confirmed |
| `--warn` | waiting | Pending · awaiting signatures · unresolved |
| `--err` | stopped | Cancelled · rejected · failed |
| `--info` | in progress | Approved · submitting |
| `--muted` | not applicable | Expired · not started |

Each has a `-bg` and a `-foreground` companion, so a badge has a readable pair in both themes.
Values are `oklch`, which is what keeps the light and dark pairs perceptually even.

```css
/* Money columns line up or they are not read. */
.tabular {
  font-variant-numeric: tabular-nums;
  font-family: var(--font-mono);
}
```

| Role | Size / weight / tracking |
|---|---|
| display | 22 / 750 / −0.6px — the amount |
| title | 13 / 700 — screen headings |
| body | 11 / 450 / 1.55 |
| label | 8.5 / 700 / .7px, uppercase |
| mono | 9.5 / 500 — addresses, hashes, ids |

Spacing scale 4 · 8 · 12 · 16 · 24. Breakpoints 360 · 768 · 1280.

**Colour is never the only channel.** Every status pill carries its word as well as its colour —
required for colour-blind readers, and it also survives a screenshot in a submission form.

## Component states

| State | When | Show | Never |
|---|---|---|---|
| Loading | Awaiting the first response | A skeleton shaped like the content that is coming | A full-screen spinner |
| Empty | No data yet | An explanation and one clear action | Forcing creation immediately after sign-in |
| Pending | Sent, no final outcome | `--warn`, what is being waited on, the hash if there is one | `--ok`, or the word "success" |
| Unresolved | Timeout or lost response | That the outcome is unknown, a Check again button, the hash | A Send again button. Any automatic retry |
| Error | A definitive failure | A reason a person understands, the next step, the trace id | A stack trace or a bare error code |
| Terminal | Paid or Cancelled | The state and an explanation. Hide every action | A disabled button — it implies it could be unlocked |
| Degraded | Read-model lag > 60 s | A "synchronising" banner; figures shown with their timestamp | Allowing a write |

The unresolved row is the one that is usually wrong. Offering "Send again" after a timeout is how a
vendor gets paid twice, which is why the only affordance is "Check again" and it re-reads rather
than re-sends.

## Error copy

Use verbatim.

| Code | Shown to the user | Action offered |
|---|---|---|
| `WALLET_REJECTED` | You declined the connection in your wallet. | Try again · Change wallet |
| `NETWORK_MISMATCH` | Your wallet is on a different network. Switch it to Stellar {network} and try again. | Check again |
| `INVALID_ADDRESS` | That is not a valid Stellar address. | Highlight the field |
| `DESTINATION_NO_TRUSTLINE` | This address is not set up to receive {asset}. | Highlight the field |
| `RELEASE_IN_FLIGHT` | Another payment from this treasury is still awaiting signatures. Wait for it to finish. | View the payment in progress |
| `NOT_A_SIGNER` | This wallet is not an approver for this payment. | No action |
| `TX_EXPIRED` | The signing window has closed. The transaction needs to be rebuilt. | Rebuild |
| `PAYMENT_UNRESOLVED` | We have not been able to confirm the outcome. Do not send it again — reopen this page in a few minutes. | Check again · View transaction |
| `SYNCING` | Synchronising with Stellar. Figures shown are current as of {time}. | Writes blocked |

Three rules for writing these: say what happened rather than what broke; always give a concrete next
step, even when that step is "contact the person who created this"; and when something is still
valid, say so — that is the single line that most reduces panic.

## Accessibility

| | |
|---|---|
| Contrast | Primary text ≥ 7:1, secondary ≥ 4.5:1. Measured, not eyeballed |
| Touch targets | 44 × 44 px minimum |
| Keyboard | Every screen fully operable; the focus ring is clearly visible |

## The rule that governs all four screens

**No screen shows success before a readback from the chain.** Every chain-touching action returns
`202` and moves the interface into a waiting state. That is how the API contract pushes "the chain is
the source of truth" all the way down to the button.

## The rules this document imposes

- **Four screens.** A fifth needs a line in the evidence package to justify it.
- **Money is a string, rendered with tabular figures.** Never a `number` input.
- **Colour always has a word beside it.**
- **The approver countdown is the transaction's timebounds**, not the request's expiry.
- **Unresolved offers "Check again", never "Send again".**
- **The evidence page is the acceptance surface.** It gets finished, not left until Day 20.
