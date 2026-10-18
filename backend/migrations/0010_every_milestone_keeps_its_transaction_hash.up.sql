-- Evidence milestones E4 and E6 had nowhere to live.
--
-- The acceptance criteria require a transaction hash for all six milestones, each one looked up on
-- a public explorer. Five of them had somewhere to be recorded; `approve` and `mark_paid` did not,
-- so the interface's evidence page showed "2/2" and "paid" — true statements that a reviewer cannot
-- verify against the chain, which is the entire purpose of that page.
--
-- On `settlements` rather than `payment_requests` because both are settlement events, and SOW 1 has
-- exactly one settlement per request.

ALTER TABLE settlements
    ADD COLUMN approve_tx_hash   text,
    ADD COLUMN mark_paid_tx_hash text;

COMMENT ON COLUMN settlements.approve_tx_hash IS
    'Evidence E4 — the registry transaction that moved Pending to Approved.';
COMMENT ON COLUMN settlements.mark_paid_tx_hash IS
    'Evidence E6 — the registry transaction that recorded the payment, written only after readback.';
