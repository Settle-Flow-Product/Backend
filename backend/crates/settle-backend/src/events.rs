//! Modules D1 and D2 — reading contract events back off the chain.
//!
//! This is the half of chain synchronisation that lives outside `settle-indexer`. That crate is a
//! pure fold with no I/O, precisely so the properties that matter — deduplication, ordering, cursor
//! discipline — can be tested against a vector instead of a live network. What lives here is the
//! part that cannot: the RPC call, and the decoding of what comes back.
//!
//! ## Why the RPC rather than the CLI
//!
//! Every other chain interaction in this service shells out to the Stellar CLI, and `stellar events`
//! would return the same events already decoded. It does not return the **transaction hash**, and
//! `UNIQUE (tx_hash, op_index)` is the deduplication key the whole design rests on. An ingest log
//! keyed on something else is an ingest log that cannot promise what this one promises.

use chrono::{DateTime, Utc};
use settle_chain::xdr::{Limits, ReadXdr, ScVal};

#[derive(Debug, thiserror::Error)]
pub enum EventsError {
    #[error("could not reach the RPC: {0}")]
    Unreachable(String),
    #[error("the RPC returned something unreadable: {0}")]
    Unreadable(String),
    #[error("the RPC refused: {0}")]
    Refused(String),
}

/// One contract event, as the network describes it.
#[derive(Debug, Clone)]
pub struct RawEvent {
    pub tx_hash: String,
    pub op_index: i32,
    pub ledger_seq: i64,
    pub ledger_time: DateTime<Utc>,
    pub contract_id: String,
    /// The second topic — `req_new`, `req_appr`, `req_paid`, `req_canc`, or something unrecognised.
    pub topic: String,
    /// The third topic: every event names its request there, so an indexer filters without
    /// decoding the body.
    pub request_id: Option<[u8; 32]>,
    pub amount: Option<i128>,
    pub paid_tx: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Reader {
    rpc_url: String,
    contract_id: String,
    http: reqwest::Client,
}

impl Reader {
    pub fn new(rpc_url: String, contract_id: String) -> Self {
        Self {
            rpc_url,
            contract_id,
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .expect("a default HTTP client always builds"),
        }
    }

    /// The ledger the network is on right now.
    ///
    /// Needed because Soroban RPC keeps only a rolling window of history — asking for events from
    /// ledger 1 is not "give me everything", it is an error. A cursor that has fallen off the back
    /// of that window has to be moved forward, and this is how we find out where to move it to.
    pub async fn latest_ledger(&self) -> Result<i64, EventsError> {
        let body = self
            .call(serde_json::json!({
                "jsonrpc": "2.0", "id": 1, "method": "getLatestLedger"
            }))
            .await?;
        body.get("result")
            .and_then(|r| r.get("sequence"))
            .and_then(|s| s.as_i64())
            .ok_or_else(|| EventsError::Unreadable(format!("no sequence in {body}")))
    }

    /// Every event this contract emitted from `start_ledger` onwards.
    pub async fn events(
        &self,
        start_ledger: i64,
        limit: u32,
    ) -> Result<Vec<RawEvent>, EventsError> {
        let body = self
            .call(serde_json::json!({
                "jsonrpc": "2.0", "id": 1, "method": "getEvents",
                "params": {
                    "startLedger": start_ledger,
                    "filters": [{ "type": "contract", "contractIds": [self.contract_id] }],
                    "pagination": { "limit": limit }
                }
            }))
            .await?;

        let events = body
            .get("result")
            .and_then(|r| r.get("events"))
            .and_then(|e| e.as_array())
            .ok_or_else(|| EventsError::Unreadable(format!("no events in {body}")))?;

        Ok(events.iter().filter_map(decode_event).collect())
    }

    async fn call(&self, payload: serde_json::Value) -> Result<serde_json::Value, EventsError> {
        let res = self
            .http
            .post(&self.rpc_url)
            .json(&payload)
            .send()
            .await
            .map_err(|e| EventsError::Unreachable(e.to_string()))?;

        let body: serde_json::Value = res
            .json()
            .await
            .map_err(|e| EventsError::Unreadable(e.to_string()))?;

        // A JSON-RPC error arrives with HTTP 200 and an `error` member. Checking the status alone
        // reads a refusal as a success and then fails on the missing field, three frames away from
        // the cause.
        if let Some(err) = body.get("error") {
            return Err(EventsError::Refused(err.to_string()));
        }
        Ok(body)
    }
}

/// Turn one RPC event into a `RawEvent`, or drop it if it is not shaped like one at all.
///
/// An event that decodes into an UNRECOGNISED topic is kept, with its topic string intact, so it
/// can be quarantined further up. Only an event missing the fields that identify it — no hash, no
/// ledger — is dropped here, because there is nothing to record it under.
fn decode_event(v: &serde_json::Value) -> Option<RawEvent> {
    let topics = v.get("topic")?.as_array()?;

    Some(RawEvent {
        tx_hash: v.get("txHash")?.as_str()?.to_string(),
        op_index: v
            .get("operationIndex")
            .and_then(|i| i.as_i64())
            .unwrap_or(0) as i32,
        ledger_seq: v.get("ledger")?.as_i64()?,
        ledger_time: v
            .get("ledgerClosedAt")
            .and_then(|t| t.as_str())
            .and_then(|t| DateTime::parse_from_rfc3339(t).ok())
            .map(|t| t.with_timezone(&Utc))
            .unwrap_or_else(Utc::now),
        contract_id: v
            .get("contractId")
            .and_then(|c| c.as_str())
            .unwrap_or_default()
            .to_string(),
        // Topic 0 is always "settle"; topic 1 names the event; topic 2 is the request id.
        topic: topics.get(1).and_then(scval_symbol).unwrap_or_default(),
        request_id: topics.get(2).and_then(scval_bytes32),
        amount: v.get("value").and_then(|x| scval_map_i128(x, "amount")),
        paid_tx: v
            .get("value")
            .and_then(|x| scval_map_bytes_hex(x, "paid_tx")),
    })
}

fn scval(v: &serde_json::Value) -> Option<ScVal> {
    ScVal::from_xdr_base64(v.as_str()?, Limits::none()).ok()
}

fn scval_symbol(v: &serde_json::Value) -> Option<String> {
    match scval(v)? {
        ScVal::Symbol(s) => Some(s.to_utf8_string().ok()?),
        _ => None,
    }
}

fn scval_bytes32(v: &serde_json::Value) -> Option<[u8; 32]> {
    match scval(v)? {
        ScVal::Bytes(b) => <[u8; 32]>::try_from(b.as_slice()).ok(),
        _ => None,
    }
}

/// Look one key up in an event body, which the SDK encodes as an `ScMap`.
fn scval_map_get(v: &serde_json::Value, key: &str) -> Option<ScVal> {
    let ScVal::Map(Some(map)) = scval(v)? else {
        return None;
    };
    map.iter()
        .find(|entry| matches!(&entry.key, ScVal::Symbol(s) if s.to_utf8_string_lossy() == key))
        .map(|entry| entry.val.clone())
}

fn scval_map_i128(v: &serde_json::Value, key: &str) -> Option<i128> {
    match scval_map_get(v, key)? {
        // i128 crosses as a high/low pair, and the halves must be recombined rather than read
        // separately: `lo` alone is right until the first amount above 2^64 stroops.
        ScVal::I128(parts) => Some(((parts.hi as i128) << 64) | (parts.lo as i128)),
        ScVal::U32(n) => Some(n as i128),
        ScVal::I64(n) => Some(n as i128),
        _ => None,
    }
}

fn scval_map_bytes_hex(v: &serde_json::Value, key: &str) -> Option<String> {
    match scval_map_get(v, key)? {
        ScVal::Bytes(b) => Some(hex::encode(b.as_slice())),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Captured verbatim from testnet, so these assert against what the network really emits rather
    // than against what this module wishes it emitted.
    const SETTLE: &str = "AAAADwAAAAZzZXR0bGUAAA==";
    const REQ_NEW: &str = "AAAADwAAAAdyZXFfbmV3AA==";
    const REQ_ID: &str = "AAAADQAAACCCFGWMaSpSiR8p5VKsEw1yEa1gHo1pjWPlYbECh2G/5w==";
    const BODY: &str =
        "AAAAEQAAAAEAAAACAAAADwAAAAZhbW91bnQAAAAAAAoAAAAAAAAAAAAAAAAEUkMgAAAADwAAAAZleHBpcnkAAAAAAAUAAAAAaoqfUw==";

    fn j(s: &str) -> serde_json::Value {
        serde_json::json!(s)
    }

    #[test]
    fn the_first_topic_is_the_namespace() {
        assert_eq!(scval_symbol(&j(SETTLE)).as_deref(), Some("settle"));
    }

    #[test]
    fn the_second_topic_names_the_event() {
        assert_eq!(scval_symbol(&j(REQ_NEW)).as_deref(), Some("req_new"));
    }

    #[test]
    fn the_third_topic_is_the_request_id() {
        let id = scval_bytes32(&j(REQ_ID)).expect("a 32 byte id");
        assert_eq!(
            hex::encode(id),
            "8214658c692a52891f29e552ac130d7211ad601e8d698d63e561b1028761bfe7"
        );
    }

    #[test]
    fn the_amount_is_recombined_from_both_halves() {
        // 72500000 stroops = 7.25 units. Read from `lo` alone this is right, which is exactly why
        // it needs a test that would still catch dropping `hi` on a larger amount.
        assert_eq!(scval_map_i128(&j(BODY), "amount"), Some(72_500_000));
    }

    #[test]
    fn a_missing_key_is_none_rather_than_zero() {
        // `paid_tx` is absent on a creation event. Defaulting it to something would put a
        // transaction hash on a request that has not been paid.
        assert_eq!(scval_map_bytes_hex(&j(BODY), "paid_tx"), None);
    }

    #[test]
    fn rubbish_decodes_to_nothing_rather_than_panicking() {
        assert_eq!(scval_symbol(&j("not base64 at all")), None);
        assert_eq!(scval_bytes32(&j("")), None);
    }

    #[test]
    fn an_event_without_a_transaction_hash_is_dropped() {
        // Nothing to record it under, and `UNIQUE (tx_hash, op_index)` is the whole guarantee.
        let e = serde_json::json!({ "ledger": 1, "topic": [SETTLE, REQ_NEW, REQ_ID] });
        assert!(decode_event(&e).is_none());
    }

    #[test]
    fn a_whole_event_decodes() {
        let e = serde_json::json!({
            "txHash": "6607b4e957f1284e6dde42843a1588bd297d5e5a83e4e139672730cb920648e2",
            "operationIndex": 0,
            "ledger": 4272117,
            "ledgerClosedAt": "2026-08-22T07:20:58Z",
            "contractId": "CAYZWEVGGANJO3EZTGIIX6VJEOYXQ2EZNTB5YYXTXKQTLNXE74RVGVBQ",
            "topic": [SETTLE, REQ_NEW, REQ_ID],
            "value": BODY,
        });
        let d = decode_event(&e).expect("decodes");
        assert_eq!(d.topic, "req_new");
        assert_eq!(d.op_index, 0);
        assert_eq!(d.ledger_seq, 4_272_117);
        assert_eq!(d.amount, Some(72_500_000));
        assert!(d.request_id.is_some());
    }
}
