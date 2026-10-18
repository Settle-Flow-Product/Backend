# 11 · Admin console

The second application: [`frontend/admin`](../frontend/admin/), port 5174. It is the screen
operations looks at when something needs explaining — a payment stuck in `unresolved`, a request
nobody approved, a chain event that never arrived.

It reads **PostgreSQL directly**. The website in [10-frontend-ux.md](10-frontend-ux.md) reads the
backend API. That difference is the whole design.

## Why it does not go through the API

The backend's nine endpoints exist to serve the payment workflow, and each one is a promise the
website depends on. An operations console asks open-ended questions — *show me every settlement
whose sequence was reserved more than an hour ago* — and answering those through an HTTP contract
means either a new endpoint per question, or one endpoint general enough to be a query language
with a URL in front of it. The first never keeps up; the second is a database connection with extra
steps and a worse security story.

So the console has a database connection and no API surface of its own. Nothing it does is reachable
from the wallet-facing app, because the two are separate deployments on separate ports with separate
credentials.

| | Website | Admin console |
|---|---|---|
| Audience | Requesters and approvers | Operations |
| Authenticates | A Stellar wallet, by SEP-10 | An email and a password |
| Reads through | The backend API | `pg`, in-process |
| Holds | No credential | The database password |
| Reachable from | The office network | The operator's network only |
| Session lives in | A bearer token in `localStorage` (`settleflow.session.v1`) | An HTTP-only cookie (`settleflow_admin_session`) |

The two session keys are different values on purpose (`SESSION_KEY`, `ADMIN_SESSION_KEY`). A token
minted for one app must not open the other, and the cheapest way to guarantee that is for neither to
be able to verify the other's signature.

The storage differs because the threat does. The website's token is held by a page that already
talks to a browser wallet extension, and it is sent as an `Authorization` header the code writes
deliberately. The console's is an HTTP-only cookie the page cannot read at all — there is no wallet
in the console, nothing there needs to see the token, and a cookie the script cannot touch is one
fewer thing an injected script could take.

## One route for every table

There is no file per table. `app/admin/[table]/page.tsx` is the only module screen, and the list of
tables comes from `pg_catalog` at request time.

A hand-written module per table is correct on the day it is written and quietly incomplete after the
next migration: the new table simply does not appear, and nothing reports it missing. Reading the
catalogue means a table added by a migration is browsable the moment `make migrate` runs.

That generality is also the risk. The one part of a SQL statement that cannot be a bound parameter
is an identifier, so a generic browser is an injection hazard by construction. The rule, enforced in
[`lib/schema.ts`](../frontend/admin/lib/schema.ts) and
[`lib/crud.ts`](../frontend/admin/lib/crud.ts):

> **Every identifier that reaches a statement was first read from the catalogue and compared for
> exact equality. Every value is bound.**

Nothing else in the console is allowed to build SQL.

## What the console will not show

| Table | Withheld because |
|---|---|
| `auth_challenges` | Live single-use nonces. Listing them is displaying unspent credentials. |
| `admin_users` | A password must be hashed on the way in, and roles decide who may edit roles. Through a generic writer an `admin` could write a `password_hash` of their choosing into a `superadmin`'s row and become one. Managed from the command line instead. |
| `_sqlx_migrations` | The migration runner's bookkeeping. Editing it makes the schema and its record disagree. |

Asking for one of these by URL returns **404**, the same answer as a misspelled table name — so the
status code cannot be used to work out which tables exist.

## What the console will not write

Read access is broad; write access is narrow, and narrowed by the *table*, not by the person. No
role writes to the money path.

| Table | Writability | Because |
|---|---|---|
| `settlements`, `settlement_approvals`, `payment_requests` | Read-only | The money path. A row edited here changes what the submitter believes about a payment that may already be in flight — including its reserved sequence number, the one value two payments must never share. |
| `chain_events`, `ingest_cursor` | Read-only | Projections, rebuilt from the chain. Editing one does not change what happened; it changes what the console *says* happened, until the next ingest overwrites it. |
| `audit_events`, `chain_events` | Append-only | `REVOKE UPDATE, DELETE` in migration 0008 is the real enforcement. The console refusing is only so it does not offer a button that would fail. |
| `governance_policies`, `policy_signers`, `idempotency_keys` | Writable | Configuration and bookkeeping. Nothing in flight depends on them mid-payment. |

A table with no primary key is read-only whatever else is true of it: without one there is no way to
name the row an edit is meant to change.

## Who may do what

Four roles, collapsed to three grants in [`types/role.ts`](../frontend/admin/types/role.ts) once,
rather than re-derived at each call site.

| | `viewer` | `operator` | `admin` | `superadmin` |
|---|:--:|:--:|:--:|:--:|
| Read any visible table | ✓ | ✓ | ✓ | ✓ |
| Write a writable table | | ✓ | ✓ | ✓ |
| Manage accounts | | | | ✓ |

The check happens **inside each server action**, per call. Not in the layout, and not in the
component that hides a button. A server action is a POST endpoint with a generated name: anything
callable is callable by somebody who should not, so each one asks who is calling before it does
anything. The layout's check stops a browser being shown a screen it cannot fill; hiding a button is
a courtesy to the person, not a control.

`middleware.ts` checks only that a session cookie is *present*. It runs on the Edge runtime, which
has no database — so it cannot ask whether the account is still active, and does not pretend to. The
real check is `currentCaller()` in the layout and in every action.

## Sessions

A hand-rolled HS256 token, in [`lib/token.ts`](../frontend/admin/lib/token.ts) — no library, and no
`alg` field read from the token. The algorithm is decided by the code, so `alg: none` is not a
sentence this verifier can be told.

Revocation is a `token_epoch` integer on the account. Every token carries the epoch it was minted
under; `currentCaller()` compares it against the row. Disabling an account or changing its password
bumps the epoch, and every token issued before that stops verifying — without a session table, and
without waiting for an expiry.

Passwords are scrypt. An unknown email is compared against a hash that cannot match, so a wrong
password and an unknown address cost the same work and return the same words. Otherwise the sign-in
form becomes a way to enumerate who has an account.

## Accounts

```
make admin-user ARGS="create ops@settleflow.internal operator"
make admin-user ARGS="list"
make admin-user ARGS="passwd ops@settleflow.internal"
make admin-user ARGS="roles ops@settleflow.internal admin"
make admin-user ARGS="disable ops@settleflow.internal"
```

Roles are positional; an argument that is not one of the four role names is ignored, and an account
created with none is a `viewer`. `create` and `passwd` prompt for the password rather than taking it
as an argument, so it does not end up in the shell history.

There is no sign-up page and no bootstrap-by-environment-variable. Both are ways for an account to
exist that nobody decided to create; the first account is made by somebody with database access, on
purpose.

`passwd`, `roles` and `disable` all bump `token_epoch`, so each one signs the account out
everywhere.

## Money on screen

`numeric(30,7)` does not fit in a JavaScript number, and `bigint` does not either. The `pg` driver is
told to leave both as strings ([`lib/db.ts`](../frontend/admin/lib/db.ts)), and wide numerics are
cast to `text` in the query rather than parsed and re-rendered.

The rule is the same one the backend and the contract keep: **float nowhere**. A console that rounds
a stroop while displaying it is a console that will eventually be quoted in an argument about how
much was paid.

## Testing

`npm test` — 46 cases, no database and no network. The pure pieces are the ones worth testing:
token encode/verify, epoch revocation, scrypt comparison, role collapse, identifier quoting, wide
numeric handling, find-option parsing, and the 404 refusal.

`server-only` throws when imported outside a server component, which would fail the whole suite, so
`vitest.config.ts` aliases it to a stub. That is why the token codec lives in `token.ts` with no
`server-only` import and the I/O lives in `session.ts` with one.

The rest is covered by [12-testing.md](12-testing.md) and by the console being pointed at a real
database during the demo.
