-- The off-chain copy of a payment request, and the duplicate protection around creating one.
--
-- The chain remains the source of truth for status. This table exists so the interface can filter
-- and display without waiting on Horizon, and so the plaintext reference has somewhere to live
-- that is not a public ledger.

CREATE TABLE payment_requests (
    -- = sha256(business_id || idempotency_key). IDENTICAL to the id on chain.
    -- One value for both eliminates an entire class of mapping bugs, and it is what makes a retry
    -- after a timeout land on this row instead of creating a second request.
    id bytea PRIMARY KEY CHECK (octet_length(id) = 32),

    creator_address stellar_addr NOT NULL,
    payee_address   stellar_addr NOT NULL,          -- THE VENDOR

    asset_code     text NOT NULL CHECK (asset_code IN ('USDC', 'XLM')),
    asset_contract text NOT NULL,                   -- the Stellar Asset Contract address
    amount         stellar_amount NOT NULL CHECK (amount > 0),

    -- The plaintext stays HERE. Only reference_hash goes on chain: a purchase order number
    -- written to the ledger is permanently public and permanently linked to a vendor and an
    -- amount, which publishes the business's supplier relationships to anyone who looks.
    reference      text  NOT NULL CHECK (char_length(reference) BETWEEN 1 AND 64),
    reference_hash bytea NOT NULL CHECK (octet_length(reference_hash) = 32),

    expiry_at timestamptz NOT NULL,
    status    request_status NOT NULL DEFAULT 'pending',

    contract_id     text NOT NULL,                  -- the registry it was written to
    create_tx_hash  text,                           -- NULL until the readback confirms it
    idempotency_key uuid NOT NULL,

    created_at timestamptz NOT NULL DEFAULT now(),
    version    integer NOT NULL DEFAULT 1,          -- optimistic lock

    -- Feature F-04, made structural. The second of two concurrent clicks loses the race here,
    -- at the database, rather than at the contract.
    CONSTRAINT one_request_per_idempotency_key UNIQUE (creator_address, idempotency_key),
    CONSTRAINT expiry_is_after_creation CHECK (expiry_at > created_at)
);

CREATE INDEX payment_requests_by_status ON payment_requests (status, created_at DESC);
CREATE INDEX payment_requests_by_payee  ON payment_requests (payee_address);

-- Serves the sweep that materialises 'expired'. Partial, because the open set stays small while
-- the table grows without bound.
CREATE INDEX payment_requests_still_open
    ON payment_requests (expiry_at) WHERE status IN ('pending', 'approved');

CREATE TABLE idempotency_keys (
    scope      text NOT NULL,                       -- 'create_request' | ...
    key        uuid NOT NULL,

    -- Distinguishes "the same call again" from "a different call reusing a key". The first returns
    -- the original result; the second is an error, because silently returning somebody else's
    -- result is worse than refusing.
    body_hash  bytea NOT NULL,

    state      idem_state NOT NULL DEFAULT 'in_flight',
    result_ref text,
    created_at timestamptz NOT NULL DEFAULT now(),
    expires_at timestamptz NOT NULL DEFAULT now() + interval '24 hours',

    PRIMARY KEY (scope, key)
);

CREATE INDEX idempotency_keys_by_expiry ON idempotency_keys (expires_at);
