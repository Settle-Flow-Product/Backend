-- WHO created the payment request — the person, not the company.
--
-- `creator_address` is the BUSINESS account, and it is correct that it is: the contract calls
-- `creator.require_auth()`, so the creator has to be an account the service can authorise as. The
-- specification agrees — `creator` is documented as "doanh nghiệp tạo lệnh", the business, and
-- `create_request` is "Doanh nghiệp, cho chính mình".
--
-- But that means the individual is invisible on chain BY DESIGN, and therefore has to be recorded
-- off chain, or nowhere. It was recorded nowhere: the wallet that signed in was verified and then
-- dropped, so every request looked identical no matter which employee raised it.
--
-- Two things in the specification need this column. `audit_events` is defined as "Ai làm gì, khi
-- nào" — who did what — and the approver screen shows a "Người tạo" line, which cannot be rendered
-- from data nobody kept.
--
-- Nullable, because rows written before this migration genuinely do not know the answer, and
-- inventing one would be worse than admitting it.

ALTER TABLE payment_requests
    ADD COLUMN created_by stellar_addr;

COMMENT ON COLUMN payment_requests.created_by IS
    'The wallet that was signed in when this request was raised. The PERSON; creator_address is the company.';

-- The trace id the request was raised under.
--
-- The acceptance criteria ask for "a trace_id that connects the interface to the chain event". The
-- interface's click and the chain write happen in different processes minutes apart, so the only
-- way to join them is to carry the id across — a trace that stops at the API boundary joins nothing.
ALTER TABLE payment_requests
    ADD COLUMN trace_id text;

COMMENT ON COLUMN payment_requests.trace_id IS
    'Carried from the interface action that raised this request, through to the chain write.';
