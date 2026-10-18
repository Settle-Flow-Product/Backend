-- The link between a request and the transaction that pays it.
--
-- Without request_id, treasury_address, reserved_seq and submit_attempted_at there is no way to
-- connect a payment to its request, no way to prevent two payments contending for one sequence
-- number, and no way to tell a redelivered submit from a first one.

CREATE TABLE settlements (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),

    -- Deliverable 2 builds an unsigned transaction FROM AN APPROVED REQUEST.
    request_id bytea NOT NULL REFERENCES payment_requests(id),

    treasury_address    text NOT NULL,              -- supplies the sequence number
    destination_address stellar_addr NOT NULL,      -- copied from the request's payee
    asset_code          text NOT NULL,
    amount              stellar_amount NOT NULL CHECK (amount > 0),

    -- PINNED at authorisation. A signer added afterwards does not get to approve work that was
    -- already in flight, and months later this answers "under which rule was this approved?".
    policy_version   integer NOT NULL,
    required_weight  integer NOT NULL CHECK (required_weight >= 1),
    collected_weight integer NOT NULL DEFAULT 0,

    status settle_status NOT NULL DEFAULT 'draft',

    unsigned_xdr    text,
    tx_hash_to_sign text,                           -- what approvers actually sign
    reserved_seq    bigint,                         -- the held treasury sequence
    valid_until     timestamptz,                    -- the timebounds maxTime

    -- The CORRECT idempotency guard. A status guard is not idempotent: the status is 'releasing'
    -- for the whole duration of a submit, so a redelivered queue message would pass a status check
    -- and submit a second time.
    submit_attempted_at timestamptz,

    tx_hash    text,
    settled_at timestamptz,

    rejected_by stellar_addr,
    rejected_at timestamptz,

    created_at timestamptz NOT NULL DEFAULT now(),
    version    integer NOT NULL DEFAULT 1,

    CONSTRAINT settled_needs_a_transaction_hash CHECK (
        status <> 'settled' OR (tx_hash IS NOT NULL AND settled_at IS NOT NULL))
);

-- Feature F-09, and the single most important line in this schema.
--
-- Exactly ONE settlement in flight per TREASURY ACCOUNT. Not per request, not per asset:
-- signatures bind to the treasury's sequence number, so a USDC payment and an XLM payment drawn
-- on the same treasury contend for the same seq. Keying this on anything narrower reproduces
-- exactly the failure it exists to prevent, and reproduces it only under concurrency — which is
-- to say, at the demo.
CREATE UNIQUE INDEX settlements_one_in_flight_per_treasury
    ON settlements (treasury_address)
    WHERE status IN ('awaiting_approval', 'releasing', 'unresolved');

-- A reserved sequence belongs to exactly one settlement.
CREATE UNIQUE INDEX settlements_one_settlement_per_reserved_sequence
    ON settlements (treasury_address, reserved_seq) WHERE reserved_seq IS NOT NULL;

CREATE UNIQUE INDEX settlements_one_row_per_transaction
    ON settlements (tx_hash) WHERE tx_hash IS NOT NULL;

CREATE INDEX settlements_by_request ON settlements (request_id);
