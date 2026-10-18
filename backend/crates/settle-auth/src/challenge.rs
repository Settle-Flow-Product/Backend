//! The wallet challenge — a SEP-10 style transaction with sequence number 0.
//!
//! # Why a transaction rather than a message
//!
//! `signMessage` is not interoperable. Wallets disagree about what they actually sign — the raw
//! bytes, `sha256` of them, or the bytes behind a domain-separating prefix — and Freighter routes
//! it through a legacy "sign blob" path that treats the payload as opaque. A backend cannot verify
//! a signature when it cannot know what was signed, and the failure is indistinguishable from a
//! wrong key. This was not a theory: it is how sign-in failed against a real wallet.
//!
//! `signTransaction` is the one operation every wallet implements identically, because getting it
//! wrong breaks payments.
//!
//! # Why signing it is safe
//!
//! The challenge carries **sequence number 0**, so it can never be submitted — a transaction's
//! sequence must be its source account's current sequence plus one, and zero is never that. The
//! user approves a transaction that is, by construction, unspendable.
//!
//! "Never make somebody sign a transaction to log in" is too blunt a rule. What must never be
//! signed is a *submittable* one.

use serde::{Deserialize, Serialize};

use crate::error::AuthError;

/// How long a challenge lives. Not extended — a lapsed one is discarded and a new one issued.
pub const CHALLENGE_TTL_SECONDS: u64 = 300;

/// What the server remembers between issuing a challenge and verifying it.
///
/// Everything needed to rebuild the transaction byte for byte. Verification never trusts what the
/// client sends back; it reconstructs the expected challenge from these stored values and checks
/// the signature against that.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Challenge {
    pub nonce: [u8; 32],
    pub address: String,
    pub domain: String,
    pub network_passphrase: String,
    pub issued_at: u64,
    pub expires_at: u64,
}

impl Challenge {
    pub fn new(
        nonce: [u8; 32],
        address: impl Into<String>,
        domain: impl Into<String>,
        network_passphrase: impl Into<String>,
        now: u64,
    ) -> Self {
        Self {
            nonce,
            address: address.into(),
            domain: domain.into(),
            network_passphrase: network_passphrase.into(),
            issued_at: now,
            expires_at: now + CHALLENGE_TTL_SECONDS,
        }
    }

    /// The transaction the wallet is asked to sign, base64 XDR.
    pub fn transaction_xdr(&self) -> Result<String, AuthError> {
        let network = settle_chain::Network::new(self.network_passphrase.clone());
        let (envelope, _) =
            settle_chain::build_challenge(&self.address, &self.nonce, self.issued_at, &network)
                .map_err(|_| AuthError::InvalidAddress)?;
        settle_chain::challenge_to_xdr(&envelope).map_err(|_| AuthError::InvalidAddress)
    }

    /// The hash a signer must have signed.
    pub fn expected_hash(&self) -> Result<[u8; 32], AuthError> {
        let network = settle_chain::Network::new(self.network_passphrase.clone());
        settle_chain::challenge_hash(&self.address, &self.nonce, self.issued_at, &network)
            .map_err(|_| AuthError::InvalidAddress)
    }

    /// Everything that does not involve cryptography, cheapest first.
    pub fn check(
        &self,
        expected_domain: &str,
        expected_network: &str,
        now: u64,
    ) -> Result<(), AuthError> {
        if now > self.expires_at {
            return Err(AuthError::ChallengeExpired);
        }
        if self.domain != expected_domain {
            return Err(AuthError::DomainMismatch);
        }
        if self.network_passphrase != expected_network {
            return Err(AuthError::NetworkMismatch);
        }
        Ok(())
    }
}

/// A 32-byte nonce from the operating system's entropy source.
pub fn generate_nonce() -> Result<[u8; 32], AuthError> {
    let mut buf = [0u8; 32];
    getrandom::fill(&mut buf).map_err(|_| AuthError::NoEntropy)?;
    Ok(buf)
}

/// Verify the envelope a wallet returned against a challenge we rebuilt ourselves.
///
/// The transaction inside the returned envelope is **ignored**. Only its signatures are read, and
/// they are checked against the hash of the challenge reconstructed from stored values — otherwise
/// a caller could sign a transaction of their own choosing and have it accepted.
pub fn verify_signed_challenge(challenge: &Challenge, signed_xdr: &str) -> Result<(), AuthError> {
    let public_key =
        settle_chain::decode_account(&challenge.address).map_err(|_| AuthError::InvalidAddress)?;
    let expected = challenge.expected_hash()?;

    let signatures =
        settle_chain::signatures_from(signed_xdr).map_err(|_| AuthError::SessionMalformed)?;
    if signatures.is_empty() {
        return Err(AuthError::BadSignature);
    }

    // A wallet may attach more than one signature. Any that verifies for this address proves
    // possession; the rest are somebody else's business.
    for signature in &signatures {
        if settle_chain::verify_signature(public_key, &expected, signature).is_ok() {
            return Ok(());
        }
    }
    Err(AuthError::BadSignature)
}

#[cfg(test)]
mod tests {
    use ed25519_dalek::{Signer, SigningKey};
    use stellar_xdr::curr::{
        DecoratedSignature, Limits, ReadXdr, Signature, SignatureHint, TransactionEnvelope,
        TransactionV1Envelope, WriteXdr,
    };

    use super::*;

    const DOMAIN: &str = "app.settleflow.internal";
    const NETWORK: &str = settle_chain::TESTNET;
    const NOW: u64 = 1_700_000_000;

    fn key(seed: u8) -> SigningKey {
        SigningKey::from_bytes(&[seed; 32])
    }

    fn challenge_for(seed: u8) -> Challenge {
        Challenge::new(
            [1u8; 32],
            settle_chain::encode_account(key(seed).verifying_key().to_bytes()),
            DOMAIN,
            NETWORK,
            NOW,
        )
    }

    /// Exactly what a wallet does: sign the transaction, return the envelope.
    fn wallet_signs(challenge: &Challenge, sk: &SigningKey) -> String {
        let xdr = challenge.transaction_xdr().unwrap();
        let envelope = TransactionEnvelope::from_xdr_base64(&xdr, Limits::none()).unwrap();
        let TransactionEnvelope::Tx(v1) = envelope else {
            panic!("expected a v1 envelope");
        };
        let hash = challenge.expected_hash().unwrap();
        let signed = TransactionEnvelope::Tx(TransactionV1Envelope {
            tx: v1.tx,
            signatures: vec![DecoratedSignature {
                hint: SignatureHint(settle_chain::signature_hint(sk.verifying_key().to_bytes())),
                signature: Signature(sk.sign(&hash).to_bytes().to_vec().try_into().unwrap()),
            }]
            .try_into()
            .unwrap(),
        });
        signed.to_xdr_base64(Limits::none()).unwrap()
    }

    #[test]
    fn a_wallet_signature_over_the_challenge_verifies() {
        let c = challenge_for(9);
        let signed = wallet_signs(&c, &key(9));
        assert!(verify_signed_challenge(&c, &signed).is_ok());
    }

    #[test]
    fn a_signature_from_another_wallet_is_refused() {
        let c = challenge_for(9);
        // The stranger signs the same transaction — it still is not their address.
        let signed = wallet_signs(&c, &key(8));
        assert_eq!(
            verify_signed_challenge(&c, &signed).unwrap_err(),
            AuthError::BadSignature
        );
    }

    #[test]
    fn a_signature_over_a_different_nonce_is_refused() {
        let issued = challenge_for(9);
        let signed = wallet_signs(&issued, &key(9));

        let mut tampered = issued.clone();
        tampered.nonce = [2u8; 32];
        assert_eq!(
            verify_signed_challenge(&tampered, &signed).unwrap_err(),
            AuthError::BadSignature
        );
    }

    #[test]
    fn an_envelope_with_no_signatures_is_refused() {
        let c = challenge_for(9);
        let unsigned = c.transaction_xdr().unwrap();
        assert_eq!(
            verify_signed_challenge(&c, &unsigned).unwrap_err(),
            AuthError::BadSignature
        );
    }

    #[test]
    fn rubbish_is_refused_rather_than_panicking() {
        let c = challenge_for(9);
        for bad in ["", "not xdr", "AAAA"] {
            assert!(verify_signed_challenge(&c, bad).is_err());
        }
    }

    #[test]
    fn the_challenge_transaction_can_never_be_submitted() {
        // The whole safety argument for asking a wallet to sign this.
        let c = challenge_for(9);
        let xdr = c.transaction_xdr().unwrap();
        let TransactionEnvelope::Tx(v1) =
            TransactionEnvelope::from_xdr_base64(&xdr, Limits::none()).unwrap()
        else {
            panic!("expected a v1 envelope");
        };
        assert_eq!(
            v1.tx.seq_num.0, 0,
            "sequence 0 is what makes it unspendable"
        );
    }

    #[test]
    fn a_lapsed_challenge_is_refused() {
        let c = challenge_for(9);
        assert!(c.check(DOMAIN, NETWORK, NOW).is_ok());
        assert!(c
            .check(DOMAIN, NETWORK, NOW + CHALLENGE_TTL_SECONDS)
            .is_ok());
        assert_eq!(
            c.check(DOMAIN, NETWORK, NOW + CHALLENGE_TTL_SECONDS + 1)
                .unwrap_err(),
            AuthError::ChallengeExpired
        );
    }

    #[test]
    fn a_challenge_issued_for_another_site_is_refused() {
        let c = challenge_for(9);
        assert_eq!(
            c.check("evil.example", NETWORK, NOW).unwrap_err(),
            AuthError::DomainMismatch
        );
    }

    #[test]
    fn a_challenge_issued_for_another_network_is_refused() {
        let c = challenge_for(9);
        assert_eq!(
            c.check(
                DOMAIN,
                "Public Global Stellar Network ; September 2015",
                NOW
            )
            .unwrap_err(),
            AuthError::NetworkMismatch
        );
    }

    #[test]
    fn generated_nonces_differ() {
        assert_ne!(generate_nonce().unwrap(), generate_nonce().unwrap());
    }
}
