//! # settle-indexer
//!
//! Modules D1–D4, as a pure fold: an ordered stream of contract events in, a projection out.
//!
//! **There is no I/O here.** The loop that reads from the network and writes to Postgres lives in
//! `settle-backend`; what is testable in isolation lives here, because the properties that matter —
//! deduplication, ordering, cursor discipline — are exactly the ones that are impossible to test
//! against a live network and trivial to test against a vector.
//!
//! The specification is `../../../docs/07-chain-sync.md`.

use std::collections::{HashMap, HashSet};

use settle_domain::{RequestId, RequestStatus};

/// The four topics the registry emits. Anything else is quarantined, never dropped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Topic {
    RequestCreated,
    RequestApproved,
    RequestPaid,
    RequestCancelled,
    /// A topic this build does not recognise.
    ///
    /// It is recorded and flagged rather than skipped. **A silently swallowed event is money
    /// nobody can account for**, and it only surfaces when somebody telephones.
    Unknown,
}

impl Topic {
    pub fn parse(topic: &str) -> Self {
        match topic {
            "req_new" => Topic::RequestCreated,
            "req_appr" => Topic::RequestApproved,
            "req_paid" => Topic::RequestPaid,
            "req_canc" => Topic::RequestCancelled,
            _ => Topic::Unknown,
        }
    }
}

/// One event as read from the chain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChainEvent {
    pub tx_hash: String,
    pub op_index: i32,
    pub ledger_seq: i64,
    /// **Ledger** time, not the time we happened to read it.
    pub ledger_time: i64,
    pub topic: Topic,
    pub request_id: Option<RequestId>,
    pub amount: Option<i128>,
    /// The settled payment's transaction hash, on a `req_paid` event.
    pub paid_tx: Option<String>,
}

impl ChainEvent {
    /// The deduplication key. `UNIQUE (tx_hash, op_index)` in Postgres, the same pair here.
    pub fn key(&self) -> (String, i32) {
        (self.tx_hash.clone(), self.op_index)
    }
}

/// What one request looks like after folding every event about it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestProjection {
    pub id: RequestId,
    pub status: RequestStatus,
    pub amount: Option<i128>,
    pub paid_tx: Option<String>,
    /// The ledger sequence of the most recent event applied.
    pub last_ledger: i64,
}

/// How an event was handled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    Applied,
    /// Already seen. Reprocessing is a no-op, which is what makes replay after a crash safe.
    Duplicate,
    /// Out of order or otherwise inapplicable — a `req_appr` for a request already paid, say.
    /// Recorded, never silently dropped.
    OutOfOrder,
    /// Unrecognised, or missing the request id it needs.
    Quarantined,
}

/// The accumulated view.
#[derive(Debug, Default)]
pub struct Projection {
    seen: HashSet<(String, i32)>,
    requests: HashMap<RequestId, RequestProjection>,
    quarantined: Vec<ChainEvent>,
    /// The last ledger safely committed. **Advanced only after the work is done.**
    cursor: i64,
}

impl Projection {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cursor(&self) -> i64 {
        self.cursor
    }

    pub fn get(&self, id: &RequestId) -> Option<&RequestProjection> {
        self.requests.get(id)
    }

    pub fn len(&self) -> usize {
        self.requests.len()
    }

    pub fn is_empty(&self) -> bool {
        self.requests.is_empty()
    }

    pub fn quarantined(&self) -> &[ChainEvent] {
        &self.quarantined
    }

    /// Apply one event.
    pub fn apply(&mut self, event: &ChainEvent) -> Decision {
        // 1 · deduplicate before anything else. This is the line that makes two parallel ingest
        //     instances safe, and it is why no leader election is needed.
        if !self.seen.insert(event.key()) {
            return Decision::Duplicate;
        }

        let Some(id) = event.request_id else {
            self.quarantined.push(event.clone());
            return Decision::Quarantined;
        };

        let next = match event.topic {
            Topic::RequestCreated => RequestStatus::Pending,
            Topic::RequestApproved => RequestStatus::Approved,
            Topic::RequestPaid => RequestStatus::Paid,
            Topic::RequestCancelled => RequestStatus::Cancelled,
            Topic::Unknown => {
                self.quarantined.push(event.clone());
                return Decision::Quarantined;
            }
        };

        match self.requests.get_mut(&id) {
            None => {
                // A first sighting must be a creation. Anything else means we started reading
                // part-way through a request's life, which is a gap worth noticing.
                if next != RequestStatus::Pending {
                    self.quarantined.push(event.clone());
                    return Decision::Quarantined;
                }
                self.requests.insert(
                    id,
                    RequestProjection {
                        id,
                        status: next,
                        amount: event.amount,
                        paid_tx: event.paid_tx.clone(),
                        last_ledger: event.ledger_seq,
                    },
                );
                Decision::Applied
            }
            Some(existing) => {
                // The status machine in settle-domain is the authority. Nothing is re-implemented
                // here, so a rule change lands in one place.
                if !existing.status.can_transition_to(next) {
                    return Decision::OutOfOrder;
                }
                existing.status = next;
                existing.last_ledger = event.ledger_seq;
                if let Some(tx) = &event.paid_tx {
                    existing.paid_tx = Some(tx.clone());
                }
                Decision::Applied
            }
        }
    }

    /// Apply a batch, then advance the cursor.
    ///
    /// **The cursor advances after the work, never before.** Dying between the two replays events,
    /// which deduplication makes harmless. Advancing first would lose them permanently.
    pub fn ingest(&mut self, batch: &[ChainEvent]) -> Vec<Decision> {
        let decisions: Vec<Decision> = batch.iter().map(|e| self.apply(e)).collect();
        if let Some(last) = batch.iter().map(|e| e.ledger_seq).max() {
            self.cursor = self.cursor.max(last);
        }
        decisions
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use settle_domain::RequestId;

    fn id(seed: u8) -> RequestId {
        RequestId::from_bytes([seed; 32])
    }

    fn event(tx: &str, op: i32, ledger: i64, topic: Topic, req: Option<RequestId>) -> ChainEvent {
        ChainEvent {
            tx_hash: tx.into(),
            op_index: op,
            ledger_seq: ledger,
            ledger_time: 1_700_000_000 + ledger,
            topic,
            request_id: req,
            amount: Some(45_000_000_000),
            paid_tx: None,
        }
    }

    #[test]
    fn the_happy_path_folds_to_paid() {
        let mut p = Projection::new();
        let r = id(1);
        let mut paid = event("t3", 0, 3, Topic::RequestPaid, Some(r));
        paid.paid_tx = Some("a91f2c8d".into());

        assert_eq!(
            p.apply(&event("t1", 0, 1, Topic::RequestCreated, Some(r))),
            Decision::Applied
        );
        assert_eq!(
            p.apply(&event("t2", 0, 2, Topic::RequestApproved, Some(r))),
            Decision::Applied
        );
        assert_eq!(p.apply(&paid), Decision::Applied);

        let view = p.get(&r).unwrap();
        assert_eq!(view.status, RequestStatus::Paid);
        assert_eq!(view.paid_tx.as_deref(), Some("a91f2c8d"));
        assert_eq!(view.last_ledger, 3);
    }

    #[test]
    fn the_same_event_twice_is_a_no_op() {
        let mut p = Projection::new();
        let r = id(2);
        let e = event("t1", 0, 1, Topic::RequestCreated, Some(r));

        assert_eq!(p.apply(&e), Decision::Applied);
        assert_eq!(p.apply(&e), Decision::Duplicate);
        assert_eq!(p.len(), 1);
    }

    #[test]
    fn two_operations_in_one_transaction_are_distinct_events() {
        // The key is the PAIR. Keying on tx_hash alone would silently drop the second.
        let mut p = Projection::new();
        assert_eq!(
            p.apply(&event("t1", 0, 1, Topic::RequestCreated, Some(id(3)))),
            Decision::Applied
        );
        assert_eq!(
            p.apply(&event("t1", 1, 1, Topic::RequestCreated, Some(id(4)))),
            Decision::Applied
        );
        assert_eq!(p.len(), 2);
    }

    #[test]
    fn replaying_an_entire_batch_changes_nothing() {
        // This is what makes "advance the cursor after the commit" safe, and therefore what makes
        // a crash mid-batch survivable.
        let r = id(5);
        let batch = vec![
            event("t1", 0, 1, Topic::RequestCreated, Some(r)),
            event("t2", 0, 2, Topic::RequestApproved, Some(r)),
        ];

        let mut p = Projection::new();
        p.ingest(&batch);
        let after_first = p.get(&r).cloned();

        let second = p.ingest(&batch);
        assert!(second.iter().all(|d| *d == Decision::Duplicate));
        assert_eq!(p.get(&r).cloned(), after_first);
    }

    #[test]
    fn two_instances_reading_the_same_stream_agree() {
        // Correctness comes from the deduplication key, not from only one process being alive.
        let r = id(6);
        let batch = vec![
            event("t1", 0, 1, Topic::RequestCreated, Some(r)),
            event("t2", 0, 2, Topic::RequestApproved, Some(r)),
            event("t3", 0, 3, Topic::RequestPaid, Some(r)),
        ];

        let mut a = Projection::new();
        let mut b = Projection::new();
        for e in &batch {
            a.apply(e);
            b.apply(e);
            // Interleaved re-delivery, as happens during a rolling deploy.
            a.apply(e);
        }
        assert_eq!(a.get(&r), b.get(&r));
    }

    #[test]
    fn a_status_moving_backwards_is_recorded_not_applied() {
        let mut p = Projection::new();
        let r = id(7);
        p.apply(&event("t1", 0, 1, Topic::RequestCreated, Some(r)));
        p.apply(&event("t2", 0, 2, Topic::RequestPaid, Some(r)));

        assert_eq!(
            p.apply(&event("t3", 0, 3, Topic::RequestApproved, Some(r))),
            Decision::OutOfOrder
        );
        assert_eq!(p.get(&r).unwrap().status, RequestStatus::Paid);
    }

    #[test]
    fn an_unknown_topic_is_quarantined_never_dropped() {
        let mut p = Projection::new();
        assert_eq!(
            p.apply(&event("t1", 0, 1, Topic::Unknown, Some(id(8)))),
            Decision::Quarantined
        );
        assert_eq!(p.quarantined().len(), 1);
    }

    #[test]
    fn an_event_with_no_request_id_is_quarantined() {
        let mut p = Projection::new();
        assert_eq!(
            p.apply(&event("t1", 0, 1, Topic::RequestCreated, None)),
            Decision::Quarantined
        );
        assert_eq!(p.quarantined().len(), 1);
    }

    #[test]
    fn a_request_first_seen_part_way_through_its_life_is_quarantined() {
        // Starting from a cursor that skipped the creation is a gap, and a gap is worth an alert
        // rather than a projection that looks complete.
        let mut p = Projection::new();
        assert_eq!(
            p.apply(&event("t9", 0, 9, Topic::RequestPaid, Some(id(9)))),
            Decision::Quarantined
        );
        assert!(p.get(&id(9)).is_none());
    }

    #[test]
    fn the_cursor_only_moves_forward() {
        let mut p = Projection::new();
        p.ingest(&[event("t5", 0, 5, Topic::RequestCreated, Some(id(10)))]);
        assert_eq!(p.cursor(), 5);

        // A late batch from further back must not rewind the read position.
        p.ingest(&[event("t2", 0, 2, Topic::RequestCreated, Some(id(11)))]);
        assert_eq!(p.cursor(), 5);
    }

    #[test]
    fn an_empty_batch_leaves_the_cursor_alone() {
        let mut p = Projection::new();
        p.ingest(&[event("t4", 0, 4, Topic::RequestCreated, Some(id(12)))]);
        p.ingest(&[]);
        assert_eq!(p.cursor(), 4);
    }

    #[test]
    fn topics_parse_from_their_on_chain_symbols() {
        assert_eq!(Topic::parse("req_new"), Topic::RequestCreated);
        assert_eq!(Topic::parse("req_appr"), Topic::RequestApproved);
        assert_eq!(Topic::parse("req_paid"), Topic::RequestPaid);
        assert_eq!(Topic::parse("req_canc"), Topic::RequestCancelled);
        assert_eq!(Topic::parse("something_else"), Topic::Unknown);
    }
}
