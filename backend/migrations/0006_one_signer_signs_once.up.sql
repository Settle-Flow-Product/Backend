-- Each approver's signature, with the weight it carries.

CREATE TABLE settlement_approvals (
    settlement_id  uuid NOT NULL REFERENCES settlements(id) ON DELETE CASCADE,
    signer_address stellar_addr NOT NULL,
    signature      bytea NOT NULL CHECK (octet_length(signature) = 64),
    weight         integer NOT NULL CHECK (weight >= 1),

    -- Stored rather than assumed. If a settlement is ever rebuilt, signatures against the old hash
    -- are visibly stale rather than silently wrong.
    tx_hash_signed text NOT NULL,

    signed_at timestamptz NOT NULL DEFAULT now(),

    -- One signer, one signature. Expressed where it cannot be forgotten: a single signer
    -- contributing their weight twice would reach a threshold of 2 alone, which is the entire
    -- failure mode multisig exists to prevent. Application code has bugs; a primary key does not.
    PRIMARY KEY (settlement_id, signer_address)
);

CREATE INDEX settlement_approvals_by_signer ON settlement_approvals (signer_address);
