//! Module C2 — the sequence lock, as a decision.
//!
//! A signature binds tightly to the treasury's sequence number. Two payments taking the same
//! sequence means the second dies with `tx_bad_seq` **after** two people have already been
//! interrupted to sign it.
//!
//! The lock itself is a unique index in Postgres, because a constraint stays correct with two
//! service instances running and application logic does not. What lives here is the rule the index
//! encodes, so it can be tested without a database and stated in one place.

use settle_domain::SettlementStatus;

/// The answer to "may this treasury start another payment?"
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reservation {
    /// Nothing in flight. Reserve `sequence` and build.
    Granted { sequence: i64 },
    /// Something already holds the sequence. **Refused cleanly, at the start** — not accepted and
    /// killed later, after signatures have been collected.
    InFlight { settlement_id: String },
}

impl Reservation {
    pub fn is_granted(&self) -> bool {
        matches!(self, Reservation::Granted { .. })
    }
}

/// A settlement that may or may not be holding the treasury.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InFlightSettlement {
    pub settlement_id: String,
    pub status: SettlementStatus,
}

/// Decide whether a new payment may reserve the treasury's next sequence number.
///
/// **The key is the treasury account.** Not the request, not the asset: a USDC payment and an XLM
/// payment drawn on the same treasury contend for the same sequence number. Keying this on
/// anything narrower reproduces exactly the failure it exists to prevent, and reproduces it only
/// under concurrency — which is to say, at the demo.
pub fn reserve(account_sequence: i64, existing: Option<&InFlightSettlement>) -> Reservation {
    if let Some(s) = existing {
        if s.status.holds_sequence() {
            return Reservation::InFlight {
                settlement_id: s.settlement_id.clone(),
            };
        }
    }
    // A transaction's sequence must be the account's current sequence plus one.
    Reservation::Granted {
        sequence: account_sequence + 1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn holding(status: SettlementStatus) -> InFlightSettlement {
        InFlightSettlement {
            settlement_id: "s-1".into(),
            status,
        }
    }

    #[test]
    fn a_free_treasury_grants_the_next_sequence() {
        assert_eq!(reserve(41, None), Reservation::Granted { sequence: 42 });
    }

    #[test]
    fn a_treasury_awaiting_signatures_refuses_a_second_payment() {
        let r = reserve(41, Some(&holding(SettlementStatus::AwaitingApproval)));
        assert_eq!(
            r,
            Reservation::InFlight {
                settlement_id: "s-1".into()
            }
        );
        assert!(!r.is_granted());
    }

    #[test]
    fn a_treasury_mid_submit_refuses_a_second_payment() {
        assert!(!reserve(41, Some(&holding(SettlementStatus::Releasing))).is_granted());
    }

    #[test]
    fn an_unresolved_settlement_still_holds_the_treasury() {
        // Releasing it would let a second payment take a sequence an in-flight envelope may still
        // consume. Unresolved means unknown, and unknown is not free.
        assert!(!reserve(41, Some(&holding(SettlementStatus::Unresolved))).is_granted());
    }

    #[test]
    fn a_finished_settlement_releases_the_treasury() {
        for status in [SettlementStatus::Settled, SettlementStatus::Failed] {
            assert!(
                reserve(41, Some(&holding(status))).is_granted(),
                "{status:?} must not block the treasury"
            );
        }
    }

    #[test]
    fn a_rejection_releases_the_treasury_immediately() {
        // Module C8. Otherwise one refusal blocks the treasury indefinitely.
        assert!(reserve(41, Some(&holding(SettlementStatus::Failed))).is_granted());
    }

    #[test]
    fn a_draft_has_not_reserved_anything_yet() {
        assert!(reserve(41, Some(&holding(SettlementStatus::Draft))).is_granted());
    }
}
