-- The signer roster, versioned rather than edited.
--
-- Changing a signer or a threshold writes a new version; the previous row stays. A settlement pins
-- the version it was authorised under, and every signature is checked against THAT version rather
-- than against whatever is current.
--
-- This answers a question that otherwise has no answer months later: under which rule was this
-- payment approved?

CREATE TABLE governance_policies (
    id               uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    treasury_address text NOT NULL,
    version          integer NOT NULL,
    threshold        integer NOT NULL CHECK (threshold >= 1),
    active           boolean NOT NULL DEFAULT false,
    created_at       timestamptz NOT NULL DEFAULT now(),

    CONSTRAINT one_row_per_treasury_and_version UNIQUE (treasury_address, version)
);

-- Exactly one active policy per treasury. Enforced here rather than in application code, because
-- two active policies means two answers to "who may approve this".
CREATE UNIQUE INDEX governance_policies_one_active_per_treasury
    ON governance_policies (treasury_address) WHERE active;

CREATE TABLE policy_signers (
    policy_id      uuid NOT NULL REFERENCES governance_policies(id) ON DELETE CASCADE,
    signer_address stellar_addr NOT NULL,
    weight         integer NOT NULL CHECK (weight BETWEEN 1 AND 255),
    role           text NOT NULL DEFAULT 'approver',

    PRIMARY KEY (policy_id, signer_address)
);

-- A policy whose signers cannot reach its own threshold is a trap: the business can never pay
-- anybody, and finds out at the worst possible moment. This is the database-side counterpart of
-- the B3 lock-out guard on the treasury account itself.
CREATE OR REPLACE FUNCTION assert_policy_threshold_is_reachable() RETURNS trigger AS $$
DECLARE
    total integer;
    required integer;
BEGIN
    IF NEW.active IS NOT TRUE THEN
        RETURN NEW;
    END IF;

    SELECT COALESCE(SUM(weight), 0) INTO total
      FROM policy_signers WHERE policy_id = NEW.id;
    required := NEW.threshold;

    IF total < required THEN
        RAISE EXCEPTION
            'policy % cannot be activated: signer weight % is below its threshold %',
            NEW.id, total, required;
    END IF;

    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER governance_policies_threshold_is_reachable
    BEFORE INSERT OR UPDATE ON governance_policies
    FOR EACH ROW EXECUTE FUNCTION assert_policy_threshold_is_reachable();
