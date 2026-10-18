//! The settlement: its status, and the threshold arithmetic that decides when to submit.

use serde::{Deserialize, Serialize};

/// Where a settlement is. Backend-only — none of this exists on chain.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SettlementStatus {
    /// Built, but no sequence reserved yet.
    Draft,
    /// A sequence is held and signatures are being collected.
    AwaitingApproval,
    /// Enough weight collected; submitted or submitting.
    Releasing,
    /// Confirmed by a ledger readback.
    Settled,
    /// A definitive failure. Not a timeout.
    Failed,
    /// The outcome is unknown.
    ///
    /// **A first-class state, not an edge case.** Nothing leaves it except a readback. A timeout
    /// and `tx_bad_seq` both land here, and both mean the same thing: we do not know, so we ask
    /// the chain before we do anything at all.
    Unresolved,
}

impl SettlementStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            SettlementStatus::Draft => "draft",
            SettlementStatus::AwaitingApproval => "awaiting_approval",
            SettlementStatus::Releasing => "releasing",
            SettlementStatus::Settled => "settled",
            SettlementStatus::Failed => "failed",
            SettlementStatus::Unresolved => "unresolved",
        }
    }

    /// Whether this status holds the treasury's sequence number.
    ///
    /// This is the predicate behind `ux_settle_inflight`. It is keyed on the treasury account, not
    /// on the request and not on the asset: signatures bind to the treasury's sequence, so any two
    /// payments drawn on the same treasury contend for it.
    pub fn holds_sequence(self) -> bool {
        matches!(
            self,
            SettlementStatus::AwaitingApproval
                | SettlementStatus::Releasing
                | SettlementStatus::Unresolved
        )
    }

    pub fn is_terminal(self) -> bool {
        matches!(self, SettlementStatus::Settled | SettlementStatus::Failed)
    }
}

/// How far along signature collection is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Progress {
    pub collected_weight: i32,
    pub required_weight: i32,
}

impl Progress {
    pub fn new(collected_weight: i32, required_weight: i32) -> Self {
        Self {
            collected_weight,
            required_weight,
        }
    }

    /// Whether enough weight has been collected to submit.
    ///
    /// `>=`, not `==`: weights need not be 1 each, so the sum can overshoot the threshold. An
    /// equality check here would leave a fully-approved payment stuck forever the first time
    /// somebody configured a signer with weight 2.
    pub fn threshold_met(self) -> bool {
        self.collected_weight >= self.required_weight
    }

    pub fn remaining(self) -> i32 {
        (self.required_weight - self.collected_weight).max(0)
    }
}

/// Why a signature was refused. Three distinct causes, three distinct codes — a caller who cannot
/// tell them apart cannot fix the problem.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SignatureRejection {
    /// The address is not in the policy version pinned to this settlement — including a signer
    /// added *after* it was authorised. That is the point of pinning.
    #[error("this wallet is not an approver for this payment")]
    NotASigner,
    /// The signature does not verify against this transaction's hash.
    #[error("the signature does not match this transaction")]
    BadSignature,
    /// This signer has already signed. Blocked by a primary key, not by application code.
    #[error("this signer has already signed")]
    AlreadySigned,
    /// The transaction's timebounds have lapsed; it must be rebuilt.
    #[error("the signing window has closed")]
    Expired,
}

impl SignatureRejection {
    pub fn code(self) -> &'static str {
        match self {
            SignatureRejection::NotASigner => "NOT_A_SIGNER",
            SignatureRejection::BadSignature => "BAD_SIGNATURE",
            SignatureRejection::AlreadySigned => "ALREADY_SIGNED",
            SignatureRejection::Expired => "TX_EXPIRED",
        }
    }
}

/// What to do about a submit outcome.
///
/// This is the decision that separates a working system from one that pays twice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubmitOutcome {
    /// The network accepted it. **Not the same as settled** — that needs a readback.
    Accepted,
    /// The sequence has been consumed. It does NOT say by whom, and the most common cause is that
    /// this very envelope already reached the ledger.
    BadSequence,
    /// No answer.
    Timeout,
    /// The fee was too low to be included.
    InsufficientFee,
    /// A definitive, named failure.
    Failed,
}

/// What the submitter does next.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubmitAction {
    /// Record the hash and wait for the readback.
    RecordAndWait,
    /// Read the chain by memo before doing anything else.
    Reconcile,
    /// Wrap the already-signed envelope in a fee bump. Approver signatures stay valid.
    FeeBump,
    /// Record a definitive failure.
    MarkFailed,
}

/// Map an outcome to an action.
///
/// **`BadSequence` and `Timeout` map to the same action, deliberately.** `tx_bad_seq` means the
/// sequence was consumed; it does not say by whom. Treating it as a failure and rebuilding the
/// transaction is how the same vendor gets paid twice. Both go and ask the chain first.
///
/// `InsufficientFee` never rebuilds either: Stellar's fee-bump wraps a signed envelope, so the
/// approver signatures survive and nobody has to be interrupted again.
pub fn decide_submit(outcome: SubmitOutcome) -> SubmitAction {
    match outcome {
        SubmitOutcome::Accepted => SubmitAction::RecordAndWait,
        SubmitOutcome::BadSequence => SubmitAction::Reconcile,
        SubmitOutcome::Timeout => SubmitAction::Reconcile,
        SubmitOutcome::InsufficientFee => SubmitAction::FeeBump,
        SubmitOutcome::Failed => SubmitAction::MarkFailed,
    }
}

/// Whether a redelivered submit message may proceed.
///
/// The obvious guard — "is the status `Releasing`?" — is **not idempotent**: the status is
/// `Releasing` for the whole duration of a submit, so a redelivered queue message passes the check
/// and submits a second time. The guard is the submit *attempt*, not the status.
pub fn may_submit(submit_attempted: bool, has_tx_hash: bool) -> SubmitAction {
    match (submit_attempted, has_tx_hash) {
        // Never attempted: go ahead.
        (false, _) => SubmitAction::RecordAndWait,
        // Attempted, and we know where it landed: nothing to do.
        (true, true) => SubmitAction::RecordAndWait,
        // Attempted, and we do not know what happened. Ask the chain.
        (true, false) => SubmitAction::Reconcile,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_statuses_that_hold_a_sequence_block_a_treasury() {
        use SettlementStatus::*;
        assert!(AwaitingApproval.holds_sequence());
        assert!(Releasing.holds_sequence());
        // Unresolved holds it too: the sequence may still be consumed by an envelope in flight,
        // and releasing it would let a second payment take the same number.
        assert!(Unresolved.holds_sequence());

        assert!(!Draft.holds_sequence());
        assert!(!Settled.holds_sequence());
        // A rejection releases the sequence, or one refusal blocks the treasury indefinitely.
        assert!(!Failed.holds_sequence());
    }

    #[test]
    fn the_threshold_is_met_by_reaching_or_passing_it() {
        assert!(!Progress::new(0, 2).threshold_met());
        assert!(!Progress::new(1, 2).threshold_met());
        assert!(Progress::new(2, 2).threshold_met());
        // A weight-2 signer overshoots, and that must still count.
        assert!(Progress::new(3, 2).threshold_met());
    }

    #[test]
    fn remaining_weight_never_goes_negative() {
        assert_eq!(Progress::new(0, 2).remaining(), 2);
        assert_eq!(Progress::new(1, 2).remaining(), 1);
        assert_eq!(Progress::new(5, 2).remaining(), 0);
    }

    #[test]
    fn a_bad_sequence_reconciles_and_never_rebuilds() {
        // This single assertion is the difference between one payment and two.
        assert_eq!(
            decide_submit(SubmitOutcome::BadSequence),
            SubmitAction::Reconcile
        );
    }

    #[test]
    fn a_timeout_takes_exactly_the_same_path_as_a_bad_sequence() {
        assert_eq!(
            decide_submit(SubmitOutcome::Timeout),
            decide_submit(SubmitOutcome::BadSequence)
        );
    }

    #[test]
    fn a_low_fee_bumps_rather_than_rebuilds() {
        assert_eq!(
            decide_submit(SubmitOutcome::InsufficientFee),
            SubmitAction::FeeBump
        );
    }

    #[test]
    fn a_redelivered_message_after_an_attempt_with_no_hash_reconciles() {
        // The status guard would say "releasing, carry on" and submit twice. This does not.
        assert_eq!(may_submit(true, false), SubmitAction::Reconcile);
    }

    #[test]
    fn a_first_attempt_proceeds_and_a_settled_one_is_a_no_op() {
        assert_eq!(may_submit(false, false), SubmitAction::RecordAndWait);
        assert_eq!(may_submit(true, true), SubmitAction::RecordAndWait);
    }

    #[test]
    fn each_signature_rejection_carries_its_own_code() {
        assert_eq!(SignatureRejection::NotASigner.code(), "NOT_A_SIGNER");
        assert_eq!(SignatureRejection::BadSignature.code(), "BAD_SIGNATURE");
        assert_eq!(SignatureRejection::AlreadySigned.code(), "ALREADY_SIGNED");
        assert_eq!(SignatureRejection::Expired.code(), "TX_EXPIRED");
    }
}
