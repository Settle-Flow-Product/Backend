//! Module C3 and C4 — accepting signatures and detecting the threshold.
//!
//! This is deliberately a pure state machine over data the caller supplies. The persistence and
//! the network calls live in `settle-backend`; what lives here is the logic that decides whether a
//! signature counts, and every one of those decisions is a place where money is lost if it is
//! wrong.

use std::collections::BTreeMap;

use settle_chain::verify_signature;
use settle_domain::{Progress, SignatureRejection};

use crate::policy::Policy;

/// One accepted signature.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Approval {
    pub signer_address: String,
    pub signature: [u8; 64],
    pub weight: i32,
    /// The hash this signature is over.
    ///
    /// Stored rather than assumed. If a settlement is ever rebuilt, signatures against the old
    /// hash are visibly stale rather than silently wrong.
    pub tx_hash_signed: [u8; 32],
}

/// Signatures collected so far for one settlement.
#[derive(Debug, Clone)]
pub struct Collector {
    policy: Policy,
    tx_hash: [u8; 32],
    /// A map, so a signer physically cannot appear twice. This mirrors
    /// `PRIMARY KEY (settlement_id, signer_address)` — the same rule, enforced in both places,
    /// because application code has bugs and a primary key does not.
    approvals: BTreeMap<String, Approval>,
    /// The transaction's timebounds `maxTime`.
    valid_until: u64,
}

impl Collector {
    pub fn new(policy: Policy, tx_hash: [u8; 32], valid_until: u64) -> Self {
        Self {
            policy,
            tx_hash,
            approvals: BTreeMap::new(),
            valid_until,
        }
    }

    pub fn policy_version(&self) -> i32 {
        self.policy.version
    }

    pub fn collected_weight(&self) -> i32 {
        self.approvals.values().map(|a| a.weight).sum()
    }

    pub fn progress(&self) -> Progress {
        self.policy.progress(self.collected_weight())
    }

    pub fn threshold_met(&self) -> bool {
        self.progress().threshold_met()
    }

    pub fn approvals(&self) -> impl Iterator<Item = &Approval> {
        self.approvals.values()
    }

    /// Whether a given signer has already contributed.
    pub fn has_signed(&self, address: &str) -> bool {
        self.approvals.contains_key(address)
    }

    /// Accept a signature, or say precisely why not.
    ///
    /// The checks run in this order for a reason: cheap before expensive, and specific before
    /// general. Verifying an ed25519 signature from somebody who is not a signer at all would be
    /// work done to reach a worse error message.
    pub fn submit(
        &mut self,
        signer_address: &str,
        signature: &[u8],
        now: u64,
    ) -> Result<Progress, SignatureRejection> {
        // V0 · a lapsed transaction cannot be signed into life. Rebuild it instead.
        if now >= self.valid_until {
            return Err(SignatureRejection::Expired);
        }

        // V1 · the signer must belong to the PINNED policy — not the current one. A signer added
        //      after this settlement was authorised does not get to approve it.
        let Some(weight) = self.policy.weight_of(signer_address) else {
            return Err(SignatureRejection::NotASigner);
        };

        // V3 before V2: an already-signed signer gets a specific answer without paying for a
        //     signature verification that would succeed anyway.
        if self.approvals.contains_key(signer_address) {
            return Err(SignatureRejection::AlreadySigned);
        }

        // V2 · the signature must verify against THIS transaction hash, not another.
        let public_key = settle_chain::decode_account(signer_address)
            .map_err(|_| SignatureRejection::NotASigner)?;
        let sig_bytes: [u8; 64] = signature
            .try_into()
            .map_err(|_| SignatureRejection::BadSignature)?;
        verify_signature(public_key, &self.tx_hash, signature)
            .map_err(|_| SignatureRejection::BadSignature)?;

        self.approvals.insert(
            signer_address.to_string(),
            Approval {
                signer_address: signer_address.to_string(),
                signature: sig_bytes,
                weight,
                tx_hash_signed: self.tx_hash,
            },
        );

        Ok(self.progress())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::policy::sow1_policy;
    use ed25519_dalek::{Signer as _, SigningKey};

    const NOW: u64 = 1_700_000_000;
    const VALID_UNTIL: u64 = NOW + 3_600;
    const TX_HASH: [u8; 32] = [0x42; 32];

    fn key(seed: u8) -> SigningKey {
        SigningKey::from_bytes(&[seed; 32])
    }

    fn address(seed: u8) -> String {
        settle_chain::encode_account(key(seed).verifying_key().to_bytes())
    }

    fn collector() -> Collector {
        let policy = sow1_policy([&address(1), &address(2), &address(3)]);
        Collector::new(policy, TX_HASH, VALID_UNTIL)
    }

    fn sign(seed: u8, hash: &[u8; 32]) -> Vec<u8> {
        key(seed).sign(hash).to_bytes().to_vec()
    }

    #[test]
    fn two_of_three_signatures_meet_the_threshold() {
        let mut c = collector();
        assert!(!c.threshold_met());

        let p = c.submit(&address(1), &sign(1, &TX_HASH), NOW).unwrap();
        assert_eq!((p.collected_weight, p.required_weight), (1, 2));
        assert!(!c.threshold_met());

        let p = c.submit(&address(2), &sign(2, &TX_HASH), NOW).unwrap();
        assert_eq!((p.collected_weight, p.required_weight), (2, 2));
        assert!(c.threshold_met());
    }

    #[test]
    fn the_third_signature_is_not_needed_but_is_still_accepted() {
        let mut c = collector();
        c.submit(&address(1), &sign(1, &TX_HASH), NOW).unwrap();
        c.submit(&address(2), &sign(2, &TX_HASH), NOW).unwrap();
        let p = c.submit(&address(3), &sign(3, &TX_HASH), NOW).unwrap();
        assert_eq!(p.collected_weight, 3);
        assert!(p.threshold_met());
    }

    #[test]
    fn one_signer_cannot_reach_the_threshold_alone() {
        // The entire failure mode multisig exists to prevent.
        let mut c = collector();
        c.submit(&address(1), &sign(1, &TX_HASH), NOW).unwrap();

        assert_eq!(
            c.submit(&address(1), &sign(1, &TX_HASH), NOW).unwrap_err(),
            SignatureRejection::AlreadySigned
        );
        assert_eq!(c.collected_weight(), 1);
        assert!(!c.threshold_met());
    }

    #[test]
    fn a_signer_outside_the_pinned_policy_is_refused() {
        let mut c = collector();
        assert_eq!(
            c.submit(&address(9), &sign(9, &TX_HASH), NOW).unwrap_err(),
            SignatureRejection::NotASigner
        );
        assert_eq!(c.collected_weight(), 0);
    }

    #[test]
    fn a_signer_added_after_authorisation_cannot_approve_this_settlement() {
        // The policy is pinned at authorisation. Adding a signer later does not retroactively let
        // them approve work that was already in flight.
        let mut c = collector();
        let current_policy_has_four = sow1_policy([&address(1), &address(2), &address(4)]);
        assert!(current_policy_has_four.weight_of(&address(4)).is_some());

        assert_eq!(
            c.submit(&address(4), &sign(4, &TX_HASH), NOW).unwrap_err(),
            SignatureRejection::NotASigner
        );
    }

    #[test]
    fn a_signature_over_a_different_hash_is_refused() {
        let mut c = collector();
        let other_hash = [0x99u8; 32];
        assert_eq!(
            c.submit(&address(1), &sign(1, &other_hash), NOW)
                .unwrap_err(),
            SignatureRejection::BadSignature
        );
    }

    #[test]
    fn one_signers_signature_presented_under_another_signers_name_is_refused() {
        let mut c = collector();
        // A real signer's address, but the signature belongs to somebody else.
        assert_eq!(
            c.submit(&address(1), &sign(2, &TX_HASH), NOW).unwrap_err(),
            SignatureRejection::BadSignature
        );
    }

    #[test]
    fn a_malformed_signature_is_refused_rather_than_panicking() {
        let mut c = collector();
        assert_eq!(
            c.submit(&address(1), &[0u8; 10], NOW).unwrap_err(),
            SignatureRejection::BadSignature
        );
    }

    #[test]
    fn signing_after_the_window_closes_is_refused() {
        let mut c = collector();
        assert_eq!(
            c.submit(&address(1), &sign(1, &TX_HASH), VALID_UNTIL)
                .unwrap_err(),
            SignatureRejection::Expired
        );
        // One second earlier is fine, so the boundary is where it is documented to be.
        assert!(c
            .submit(&address(1), &sign(1, &TX_HASH), VALID_UNTIL - 1)
            .is_ok());
    }

    #[test]
    fn each_approval_records_the_hash_it_signed() {
        let mut c = collector();
        c.submit(&address(1), &sign(1, &TX_HASH), NOW).unwrap();
        let a = c.approvals().next().unwrap();
        assert_eq!(a.tx_hash_signed, TX_HASH);
        assert_eq!(a.weight, 1);
    }

    #[test]
    fn a_rejected_signature_changes_nothing() {
        let mut c = collector();
        let before = c.collected_weight();
        let _ = c.submit(&address(9), &sign(9, &TX_HASH), NOW);
        let _ = c.submit(&address(1), &sign(2, &TX_HASH), NOW);
        assert_eq!(c.collected_weight(), before);
        assert!(!c.has_signed(&address(1)));
    }
}
