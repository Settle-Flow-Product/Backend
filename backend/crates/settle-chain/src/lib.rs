//! # settle-chain
//!
//! Everything Stellar: decode an address, build the vendor payment, compute the hash approvers
//! sign, verify their signatures, attach them, and wrap the result in a fee bump when the network
//! wants more.
//!
//! **Nothing in this crate submits anything.** Building and submitting are separate so the guard
//! against double submission lives in exactly one place — see `settle-signature`.
//!
//! The specification is `../../../docs/06-signature-service.md`.

pub mod error;
pub mod keys;

/// The XDR types callers need to decode and re-encode an envelope this crate built.
///
/// Re-exported rather than left to the caller's own dependency, so there is exactly one
/// `stellar-xdr` version in the build and a transaction can never be encoded by one version and
/// decoded by another.
pub mod xdr {
    pub use stellar_xdr::curr::{
        FeeBumpTransactionInnerTx, Limits, ReadXdr, ScMap, ScVal, Transaction, TransactionEnvelope,
        TransactionV1Envelope, WriteXdr,
    };
}
pub mod network;
pub mod sep10;
pub mod tx;

pub use error::ChainError;
pub use keys::{decode_account, encode_account, signature_hint, verify_message, verify_signature};
pub use network::{Network, TESTNET};
pub use sep10::{
    build_challenge, challenge_hash, signatures_from, to_xdr as challenge_to_xdr, CHALLENGE_SECONDS,
};
pub use tx::{
    attach_signatures, build_payment, fee_bump, signature_payload_hash, CollectedSignature,
    PaymentSpec, UnsignedTx, BASE_FEE, TIMEBOUNDS_SECONDS,
};
