-- Sign-in for an internal tool where nobody has a password.
--
-- A nonce that is not stored cannot be single use, which is the whole reason this table exists.
-- Verification TAKES the row — reads and deletes in one statement — so a captured challenge cannot
-- be replayed against a slow client.

CREATE TABLE auth_challenges (
    nonce      bytea PRIMARY KEY CHECK (octet_length(nonce) = 32),
    address    stellar_addr NOT NULL,
    -- Bound to a network, so a testnet proof cannot open a mainnet session.
    network    text NOT NULL,
    -- Bound to a domain, so a challenge issued by another site is not accepted here.
    domain     text NOT NULL,
    issued_at  timestamptz NOT NULL DEFAULT now(),
    expires_at timestamptz NOT NULL,

    CONSTRAINT challenge_expires_after_it_is_issued CHECK (expires_at > issued_at)
);

-- Lapsed challenges are swept, not extended. A challenge that can be renewed is a challenge with
-- no expiry.
CREATE INDEX auth_challenges_by_expiry ON auth_challenges (expires_at);
