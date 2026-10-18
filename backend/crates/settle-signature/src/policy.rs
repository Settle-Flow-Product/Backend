//! The signer roster a settlement is judged against.
//!
//! A policy is **versioned, never edited**. Changing a signer or a threshold writes a new version;
//! the old one stays. A settlement pins the version it was authorised under, and every signature is
//! checked against *that* version rather than against whatever is current.
//!
//! This answers a question that otherwise has no answer months later: under which rule was this
//! payment approved?

use settle_domain::Progress;

/// One approver.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Signer {
    pub address: String,
    pub weight: i32,
    pub role: String,
}

/// A frozen roster.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Policy {
    pub version: i32,
    pub threshold: i32,
    pub signers: Vec<Signer>,
}

impl Policy {
    pub fn new(version: i32, threshold: i32, signers: Vec<Signer>) -> Self {
        Self {
            version,
            threshold,
            signers,
        }
    }

    pub fn weight_of(&self, address: &str) -> Option<i32> {
        self.signers
            .iter()
            .find(|s| s.address == address)
            .map(|s| s.weight)
    }

    pub fn total_weight(&self) -> i32 {
        self.signers.iter().map(|s| s.weight).sum()
    }

    /// Whether this policy can ever authorise anything.
    ///
    /// A policy whose signers cannot reach its own threshold is a trap: the business can never pay
    /// anybody and finds out at the worst moment. This is the database-side counterpart of the B3
    /// lock-out guard.
    pub fn is_reachable(&self) -> bool {
        self.threshold >= 1 && self.total_weight() >= self.threshold
    }

    /// Whether losing one signer would make the policy unusable.
    ///
    /// A 3-of-3 configuration means one lost key ends the business's ability to pay. SOW 1 uses
    /// 2 of 3 precisely so that one lost key is survivable.
    pub fn survives_losing_one_signer(&self) -> bool {
        let Some(largest) = self.signers.iter().map(|s| s.weight).max() else {
            return false;
        };
        self.total_weight() - largest >= self.threshold
    }

    pub fn progress(&self, collected_weight: i32) -> Progress {
        Progress::new(collected_weight, self.threshold)
    }
}

/// The SOW 1 treasury configuration: three signers at weight 1, threshold 2.
pub fn sow1_policy(signers: [&str; 3]) -> Policy {
    Policy::new(
        1,
        2,
        signers
            .iter()
            .enumerate()
            .map(|(i, a)| Signer {
                address: (*a).to_string(),
                weight: 1,
                role: format!("approver-{}", i + 1),
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn three() -> Policy {
        sow1_policy(["GA", "GB", "GC"])
    }

    #[test]
    fn the_sow_configuration_is_two_of_three() {
        let p = three();
        assert_eq!(p.threshold, 2);
        assert_eq!(p.total_weight(), 3);
        assert_eq!(p.weight_of("GB"), Some(1));
        assert_eq!(p.weight_of("GZ"), None);
    }

    #[test]
    fn two_of_three_survives_one_lost_key() {
        assert!(three().survives_losing_one_signer());
    }

    #[test]
    fn three_of_three_does_not() {
        let mut p = three();
        p.threshold = 3;
        assert!(p.is_reachable());
        // Reachable today, unusable the moment anyone loses a key. This is why the threshold is
        // never set equal to the total weight.
        assert!(!p.survives_losing_one_signer());
    }

    #[test]
    fn a_threshold_beyond_the_available_weight_is_a_trap() {
        let mut p = three();
        p.threshold = 4;
        assert!(!p.is_reachable());
    }

    /// Feature F-07, as a predicate.
    ///
    /// The on-chain counterpart lives in `backend/scripts/setup_treasury.sh`, which refuses to
    /// submit a `setOptions` that would leave the account unable to reach its own threshold. That
    /// state is UNRECOVERABLE: `setOptions` is itself a high-threshold operation, so once the high
    /// threshold is out of reach nobody can ever change the configuration back.
    #[test]
    fn a_locking_threshold_configuration_is_refused() {
        // Σ weight = 3. Anything above it can never be met.
        for threshold in [4, 5, 100] {
            let mut p = three();
            p.threshold = threshold;
            assert!(
                !p.is_reachable(),
                "threshold {threshold} against total weight 3 must be refused"
            );
        }

        // Exactly reachable, but one lost key ends it. Refused for a different reason.
        let mut exact = three();
        exact.threshold = 3;
        assert!(exact.is_reachable());
        assert!(
            !exact.survives_losing_one_signer(),
            "a threshold equal to the total weight must be refused"
        );

        // The SOW configuration passes both checks.
        let sow = three();
        assert!(sow.is_reachable());
        assert!(sow.survives_losing_one_signer());
    }

    #[test]
    fn a_policy_with_no_signers_cannot_authorise_anything() {
        let p = Policy::new(1, 1, vec![]);
        assert!(!p.is_reachable());
        assert!(!p.survives_losing_one_signer());
    }

    #[test]
    fn progress_is_reported_against_the_pinned_threshold() {
        let p = three();
        assert!(!p.progress(1).threshold_met());
        assert!(p.progress(2).threshold_met());
        assert_eq!(p.progress(1).remaining(), 1);
    }
}
