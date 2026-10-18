ALTER TABLE settlements
    DROP COLUMN IF EXISTS approve_tx_hash,
    DROP COLUMN IF EXISTS mark_paid_tx_hash;
