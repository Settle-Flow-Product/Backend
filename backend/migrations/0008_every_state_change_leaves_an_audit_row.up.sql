-- Who did what, when, and from which state.

CREATE TABLE audit_events (            -- APPEND-ONLY
    id         bigserial PRIMARY KEY,
    actor_type actor_type NOT NULL,
    actor_id   text,
    entity     text NOT NULL,          -- 'payment_request' | 'settlement' | 'policy'
    entity_id  text NOT NULL,
    action     text NOT NULL,
    from_state text,
    to_state   text,
    metadata   jsonb NOT NULL DEFAULT '{}',

    -- Connects a click in the interface to this row to a chain event. The Definition of Done
    -- requires it end to end.
    trace_id text,

    at timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX audit_events_by_entity ON audit_events (entity, entity_id, at DESC);
CREATE INDEX audit_events_by_trace  ON audit_events (trace_id) WHERE trace_id IS NOT NULL;

-- Append-only, enforced by permission rather than by promise.
--
-- The application role does not hold the grant to try, so this is a property of the system rather
-- than a rule somebody has to remember during a code review. The role is created only if the
-- deployment defines one; a local database run by a superuser has nothing to revoke.
DO $$
BEGIN
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'app_rw') THEN
        REVOKE UPDATE, DELETE ON chain_events, audit_events FROM app_rw;
    END IF;
END
$$;
