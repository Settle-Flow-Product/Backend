//! Module C6 — the only path permitted to reach a rebuild.
//!
//! `tx_bad_seq` says the sequence has been consumed. **It does not say by whom**, and the most
//! common cause is that this very envelope already reached the ledger — a duplicate submit, a
//! redelivered queue message, a lost HTTP response. Building a new transaction with a new sequence
//! and collecting signatures again pays the vendor a second time.
//!
//! So a timeout and `tx_bad_seq` take the same path, and that path asks the chain first.

use settle_domain::SubmitOutcome;

/// What the chain says when we go looking.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChainLookup {
    /// Found by memo. The payment already settled; there is nothing to resend.
    Found { tx_hash: String },
    /// Nothing under this memo, and the reserved sequence is still unused.
    NotFoundSequenceUnused,
    /// Nothing under this memo, but the sequence has been consumed by something else. Only here
    /// is a rebuild justified.
    NotFoundSequenceConsumed,
}

/// What to do next.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolution {
    /// It landed. Record the hash and wait for the readback to confirm settlement.
    RecordSettled { tx_hash: String },
    /// Resend the identical envelope. Same hash, so the network's own replay protection makes it
    /// safe, and no approver is troubled.
    ResubmitSameEnvelope,
    /// Build a new transaction and collect signatures again. **The expensive path**, and the only
    /// one that can pay twice if reached in error.
    RebuildAndReCollect,
}

/// Decide what to do about an unresolved settlement.
///
/// The memo lookup comes first because it is the one that can prove the payment already happened.
/// Any ordering that resends before looking is an ordering that can pay twice.
pub fn resolve(lookup: ChainLookup) -> Resolution {
    match lookup {
        ChainLookup::Found { tx_hash } => Resolution::RecordSettled { tx_hash },
        ChainLookup::NotFoundSequenceUnused => Resolution::ResubmitSameEnvelope,
        ChainLookup::NotFoundSequenceConsumed => Resolution::RebuildAndReCollect,
    }
}

/// Whether an outcome must go through reconciliation before anything else happens.
pub fn needs_reconciliation(outcome: SubmitOutcome) -> bool {
    matches!(outcome, SubmitOutcome::BadSequence | SubmitOutcome::Timeout)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_payment_found_by_memo_is_recorded_not_resent() {
        assert_eq!(
            resolve(ChainLookup::Found {
                tx_hash: "a91f2c8d".into()
            }),
            Resolution::RecordSettled {
                tx_hash: "a91f2c8d".into()
            }
        );
    }

    #[test]
    fn an_unused_sequence_resends_the_identical_envelope() {
        // Same hash, so the network refuses a genuine duplicate itself. No approver is troubled.
        assert_eq!(
            resolve(ChainLookup::NotFoundSequenceUnused),
            Resolution::ResubmitSameEnvelope
        );
    }

    #[test]
    fn only_a_sequence_consumed_by_something_else_justifies_a_rebuild() {
        assert_eq!(
            resolve(ChainLookup::NotFoundSequenceConsumed),
            Resolution::RebuildAndReCollect
        );
    }

    #[test]
    fn a_rebuild_is_never_the_answer_when_the_payment_may_have_landed() {
        // The single most important property in this module: neither of the two states where the
        // payment might already exist may reach a rebuild.
        for lookup in [
            ChainLookup::Found {
                tx_hash: "x".into(),
            },
            ChainLookup::NotFoundSequenceUnused,
        ] {
            assert_ne!(resolve(lookup), Resolution::RebuildAndReCollect);
        }
    }

    #[test]
    fn a_bad_sequence_and_a_timeout_both_reconcile() {
        assert!(needs_reconciliation(SubmitOutcome::BadSequence));
        assert!(needs_reconciliation(SubmitOutcome::Timeout));
    }

    #[test]
    fn a_clean_outcome_does_not_reconcile() {
        assert!(!needs_reconciliation(SubmitOutcome::Accepted));
        assert!(!needs_reconciliation(SubmitOutcome::Failed));
        assert!(!needs_reconciliation(SubmitOutcome::InsufficientFee));
    }
}
