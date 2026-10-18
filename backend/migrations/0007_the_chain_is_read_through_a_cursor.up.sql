-- The ingest log and the read position. The backbone of correctness.

CREATE TABLE chain_events (            -- APPEND-ONLY
    id          bigserial PRIMARY KEY,
    tx_hash     text NOT NULL,
    op_index    integer NOT NULL,
    ledger_seq  bigint NOT NULL,
    ledger_time timestamptz NOT NULL,  -- LEDGER time, not the time we happened to read it
    contract_id text,
    topic       text NOT NULL,         -- req_new | req_appr | req_paid | req_canc
    request_id  bytea,
    payload     jsonb NOT NULL DEFAULT '{}',

    -- An unrecognised event is flagged, never skipped. A silently swallowed event is money nobody
    -- can account for, and it only surfaces when somebody telephones.
    quarantined  boolean NOT NULL DEFAULT false,
    processed_at timestamptz,

    -- Absolute deduplication. This one line buys three things: reprocessing is a no-op, two
    -- daemon instances can run in parallel, and a rolling deploy needs no leader election —
    -- because correctness comes from the constraint rather than from only one process being alive.
    CONSTRAINT one_row_per_operation UNIQUE (tx_hash, op_index)
);

CREATE INDEX chain_events_by_request ON chain_events (request_id) WHERE request_id IS NOT NULL;
CREATE INDEX chain_events_quarantined ON chain_events (ledger_seq) WHERE quarantined;

CREATE TABLE ingest_cursor (
    -- A bool primary key defaulting to true, with CHECK (id), constrains this table to a single
    -- row. Two cursor rows would mean two readers disagreeing about where they are.
    id         boolean PRIMARY KEY DEFAULT true CHECK (id),
    ledger_seq bigint NOT NULL,
    updated_at timestamptz NOT NULL DEFAULT now()
);
