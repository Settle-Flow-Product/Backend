#!/usr/bin/env bash
#
# Prove the constraints that carry the correctness actually refuse what they claim to refuse.
#
# A unique index nobody has watched reject something is a comment. Each block below plants the
# exact scenario the constraint exists to prevent and asserts the database says no — most
# importantly F-09, where a second payment on the same treasury must be refused CLEANLY at the
# start rather than accepted, signed by two people, and killed later by tx_bad_seq.
#
# Runs against a throwaway container by default.

set -euo pipefail
cd "$(dirname "$0")/.."

CONTAINER="${CONTAINER:-settle-constraints-verify}"

if [ -z "${PSQL:-}" ]; then
    command -v docker >/dev/null 2>&1 || { echo "docker is required, or set PSQL"; exit 1; }
    docker rm -f "$CONTAINER" >/dev/null 2>&1 || true
    docker run -d --name "$CONTAINER" \
        -e POSTGRES_USER=settle -e POSTGRES_PASSWORD=settle -e POSTGRES_DB=settle \
        postgres:17-alpine >/dev/null
    trap 'docker rm -f "$CONTAINER" >/dev/null 2>&1 || true' EXIT
    # `pg_isready` is not a sufficient signal here.
    #
    # The official image starts a temporary server for initdb, SHUTS IT DOWN, then starts the real
    # one — and the temporary server answers pg_isready over the unix socket. Accepting it means the
    # first statement can land on a database that is on its way down, which fails with "the database
    # system is shutting down". The window is short, so it hits intermittently, which is worse than
    # always: it teaches whoever sees it to re-run until green.
    #
    # The temporary server is started with `listen_addresses=''` and accepts no TCP connection at
    # all, so a query forced over TCP only succeeds against the real one. Twice running, to cover
    # the moment the real server is itself still starting.
    ready=0
    for _ in $(seq 1 90); do
        if docker exec "$CONTAINER" \
             psql -qtAX -h 127.0.0.1 -U settle -d settle -c 'select 1' >/dev/null 2>&1; then
            ready=$((ready + 1))
            [ "$ready" -ge 2 ] && break
        else
            ready=0
        fi
        sleep 1
    done
    [ "$ready" -ge 2 ] || { echo "postgres in $CONTAINER never accepted a query"; exit 1; }
    PSQL="docker exec -i $CONTAINER psql"
fi

run() { $PSQL -v ON_ERROR_STOP=1 -q -U settle -d settle; }

echo "→ applying the schema"
for f in $(ls migrations/*.up.sql | sort); do run < "$f"; done

echo "→ asserting behaviour"
run <<'SQL'
\set QUIET on
-- Assertions report through NOTICE, which goes to stderr. The empty result rows from each
-- SELECT are noise, so stdout is discarded.
\o /dev/null
\set treasury '''GTREASURYAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA'''
\set vendor   '''GVENDORAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA'''
\set signer   '''GSIGNERAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA'''

INSERT INTO payment_requests
 (id, creator_address, payee_address, asset_code, asset_contract, amount,
  reference, reference_hash, expiry_at, contract_id, idempotency_key)
VALUES
 (decode(repeat('a1',32),'hex'), :treasury, :vendor, 'USDC', 'CBIE', 4500.0000000,
  'PO-2026-0417', decode(repeat('b2',32),'hex'), now() + interval '1 hour',
  'CBIE', '00000000-0000-0000-0000-000000000001');

-- A helper so each assertion reads as one line: run `sql`, expect it to fail with `expected`.
CREATE OR REPLACE FUNCTION must_refuse(what text, stmt text, expected text) RETURNS void AS $fn$
BEGIN
    BEGIN
        EXECUTE stmt;
    EXCEPTION
        WHEN unique_violation THEN
            IF expected <> 'unique' THEN RAISE EXCEPTION 'REGRESSION [%]: expected %, got unique_violation', what, expected; END IF;
            RAISE NOTICE 'ok  %', what; RETURN;
        WHEN check_violation THEN
            IF expected <> 'check' THEN RAISE EXCEPTION 'REGRESSION [%]: expected %, got check_violation', what, expected; END IF;
            RAISE NOTICE 'ok  %', what; RETURN;
        WHEN raise_exception THEN
            IF expected <> 'raise' THEN RAISE EXCEPTION 'REGRESSION [%]: expected %, got raise', what, expected; END IF;
            RAISE NOTICE 'ok  %', what; RETURN;
    END;
    RAISE EXCEPTION 'REGRESSION [%]: the database ACCEPTED what it must refuse', what;
END;
$fn$ LANGUAGE plpgsql;

SELECT must_refuse(
  'a malformed vendor address is refused',
  $$INSERT INTO payment_requests (id, creator_address, payee_address, asset_code, asset_contract,
      amount, reference, reference_hash, expiry_at, contract_id, idempotency_key)
    VALUES (decode(repeat('a2',32),'hex'),
      'GTREASURYAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA', 'not-an-address',
      'USDC','CBIE',1,'x',decode(repeat('b2',32),'hex'), now()+interval '1 hour','CBIE',
      gen_random_uuid())$$,
  'check');

SELECT must_refuse(
  'F-04 the same idempotency key cannot create a second request',
  $$INSERT INTO payment_requests (id, creator_address, payee_address, asset_code, asset_contract,
      amount, reference, reference_hash, expiry_at, contract_id, idempotency_key)
    VALUES (decode(repeat('a3',32),'hex'),
      'GTREASURYAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA',
      'GVENDORAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA',
      'USDC','CBIE',1,'y',decode(repeat('b2',32),'hex'), now()+interval '1 hour','CBIE',
      '00000000-0000-0000-0000-000000000001')$$,
  'unique');

SELECT must_refuse(
  'a zero amount is refused',
  $$INSERT INTO payment_requests (id, creator_address, payee_address, asset_code, asset_contract,
      amount, reference, reference_hash, expiry_at, contract_id, idempotency_key)
    VALUES (decode(repeat('a4',32),'hex'),
      'GTREASURYAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA',
      'GVENDORAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA',
      'USDC','CBIE',0,'z',decode(repeat('b2',32),'hex'), now()+interval '1 hour','CBIE',
      gen_random_uuid())$$,
  'check');

-- F-09. The case that matters most.
INSERT INTO settlements
 (request_id, treasury_address, destination_address, asset_code, amount,
  policy_version, required_weight, status, reserved_seq)
VALUES (decode(repeat('a1',32),'hex'), 'GTREASURY1',
  'GVENDORAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA',
  'USDC', 4500, 1, 2, 'awaiting_approval', 42);

SELECT must_refuse(
  'F-09 a second payment on the same treasury is refused, even for a DIFFERENT asset',
  $$INSERT INTO settlements (request_id, treasury_address, destination_address, asset_code,
      amount, policy_version, required_weight, status, reserved_seq)
    VALUES (decode(repeat('a1',32),'hex'), 'GTREASURY1',
      'GVENDORAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA',
      'XLM', 100, 1, 2, 'awaiting_approval', 43)$$,
  'unique');

DO $$ BEGIN
  INSERT INTO settlements (request_id, treasury_address, destination_address, asset_code,
    amount, policy_version, required_weight, status, reserved_seq)
  VALUES (decode(repeat('a1',32),'hex'), 'GTREASURY2',
    'GVENDORAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA',
    'USDC', 10, 1, 2, 'awaiting_approval', 7);
  RAISE NOTICE 'ok  a different treasury is not blocked by the first';
END $$;

UPDATE settlements SET status='unresolved' WHERE treasury_address='GTREASURY1';
SELECT must_refuse(
  'an unresolved settlement still holds its treasury',
  $$INSERT INTO settlements (request_id, treasury_address, destination_address, asset_code,
      amount, policy_version, required_weight, status, reserved_seq)
    VALUES (decode(repeat('a1',32),'hex'), 'GTREASURY1',
      'GVENDORAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA',
      'USDC', 1, 1, 2, 'awaiting_approval', 44)$$,
  'unique');

UPDATE settlements SET status='failed', rejected_at=now() WHERE treasury_address='GTREASURY1';
DO $$ BEGIN
  INSERT INTO settlements (request_id, treasury_address, destination_address, asset_code,
    amount, policy_version, required_weight, status, reserved_seq)
  VALUES (decode(repeat('a1',32),'hex'), 'GTREASURY1',
    'GVENDORAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA',
    'USDC', 1, 1, 2, 'awaiting_approval', 44);
  RAISE NOTICE 'ok  C8 a rejection releases the treasury for the next payment';
END $$;

SELECT must_refuse(
  'settled without a transaction hash is refused',
  $$UPDATE settlements SET status='settled' WHERE treasury_address='GTREASURY2'$$,
  'check');

DO $$
DECLARE sid uuid;
BEGIN
  SELECT id INTO sid FROM settlements WHERE treasury_address='GTREASURY2';
  INSERT INTO settlement_approvals (settlement_id, signer_address, signature, weight, tx_hash_signed)
  VALUES (sid, 'GSIGNERAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA',
          decode(repeat('cc',64),'hex'), 1, 'c204');
  PERFORM must_refuse(
    'F-10 one signer cannot reach the threshold alone',
    format($f$INSERT INTO settlement_approvals (settlement_id, signer_address, signature, weight,
        tx_hash_signed) VALUES (%L,
        'GSIGNERAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA',
        decode(repeat('dd',64),'hex'), 1, 'c204')$f$, sid),
    'unique');
END $$;

INSERT INTO chain_events (tx_hash, op_index, ledger_seq, ledger_time, topic)
VALUES ('t1', 0, 1, now(), 'req_new');
DO $$ BEGIN
  INSERT INTO chain_events (tx_hash, op_index, ledger_seq, ledger_time, topic)
  VALUES ('t1', 1, 1, now(), 'req_new');
  RAISE NOTICE 'ok  D2 a second operation in one transaction is a distinct event';
END $$;
SELECT must_refuse(
  'D2 replaying an event is a no-op',
  $$INSERT INTO chain_events (tx_hash, op_index, ledger_seq, ledger_time, topic)
    VALUES ('t1', 0, 1, now(), 'req_new')$$,
  'unique');

INSERT INTO ingest_cursor (ledger_seq) VALUES (100);
SELECT must_refuse(
  'the ingest cursor is a single row',
  $$INSERT INTO ingest_cursor (ledger_seq) VALUES (200)$$,
  'unique');

DO $$
DECLARE pid uuid;
BEGIN
  INSERT INTO governance_policies (treasury_address, version, threshold, active)
  VALUES ('GTREASURY9', 1, 2, false) RETURNING id INTO pid;
  PERFORM must_refuse(
    'B3 a policy that can never reach its own threshold cannot be activated',
    format($f$UPDATE governance_policies SET active = true WHERE id = %L$f$, pid),
    'raise');

  INSERT INTO policy_signers (policy_id, signer_address, weight) VALUES
    (pid, 'GSIGNERAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA', 1),
    (pid, 'GSIGNERBAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA', 1),
    (pid, 'GSIGNERCAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA', 1);
  UPDATE governance_policies SET active = true WHERE id = pid;
  RAISE NOTICE 'ok  three signers at weight 1 activate a threshold of 2';
END $$;

DROP FUNCTION must_refuse(text, text, text);
SQL

echo
echo "every constraint refuses what it claims to refuse"
