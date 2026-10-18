ALTER TABLE payment_requests
    DROP COLUMN IF EXISTS created_by,
    DROP COLUMN IF EXISTS trace_id;
