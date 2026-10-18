-- One place where every enum and money type in the system is defined.
--
-- Conventions this file establishes, and every later migration follows:
--
--   * Every amount is `stellar_amount` — numeric(30,7). Stellar uses seven decimal places, and
--     numeric compares exactly, which is what an exact-amount check requires. A double precision
--     anywhere in a money path is a bug waiting for the right value.
--   * Every account address is `stellar_addr`, checked against the same regular expression the
--     form field and the API use. Three layers, deliberately: the form is for the person typing,
--     the API because a client cannot be trusted, the column because application code has bugs.
--   * Ids are uuid with a default; timestamps are timestamptz with a default. Never a bare
--     timestamp — a payment system that stores local time cannot reconcile across a border.
--   * Constraints are named in prose, so a violation reads as a sentence in the logs.

-- pgcrypto supplies gen_random_uuid(). Nothing else is enabled: an extension that is
-- installed but unused is surface area with no reader.
CREATE EXTENSION IF NOT EXISTS "pgcrypto";

-- Mirrors ReqStatus on the contract, plus 'expired', which is DERIVED here.
-- The contract has no Expired status: it compares ledger.timestamp() against expiry on every call.
-- Materialising it lets the interface filter on it.
CREATE TYPE request_status AS ENUM ('pending', 'approved', 'paid', 'cancelled', 'expired');

-- Backend-only. 'unresolved' has no on-chain counterpart and is a first-class state, not an edge
-- case: nothing leaves it except a readback.
CREATE TYPE settle_status AS ENUM ('draft', 'awaiting_approval', 'releasing',
                                   'settled', 'failed', 'unresolved');

CREATE TYPE idem_state AS ENUM ('in_flight', 'done', 'released');

CREATE TYPE actor_type AS ENUM ('business', 'approver', 'system', 'ops');

CREATE DOMAIN stellar_amount AS numeric(30, 7) CHECK (VALUE >= 0);

CREATE DOMAIN stellar_addr AS text CHECK (VALUE ~ '^G[A-Z2-7]{55}$');
