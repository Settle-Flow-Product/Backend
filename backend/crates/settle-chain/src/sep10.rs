//! SEP-10 style challenge transactions.
//!
//! # Why a transaction and not a message
//!
//! `signMessage` is not interoperable. Wallets disagree about what they sign — the raw bytes,
//! `sha256` of them, or the bytes with a domain-separating prefix — and Freighter routes it through
//! a legacy "sign blob" path that treats the payload as opaque. A backend cannot verify a signature
//! when it cannot know what was signed, and the failure looks identical to a wrong key.
//!
//! `signTransaction` is the operation every wallet implements identically, because getting it wrong
//! breaks payments.
//!
//! # Why signing this is safe
//!
//! **The challenge carries sequence number 0, so it can never be submitted.** A transaction's
//! sequence must be its source account's current sequence plus one; zero is never that. The wallet
//! displays a transaction, the user approves it, and the result is an unspendable artefact that
//! proves key possession and nothing else.
//!
//! That is the whole design of SEP-10, and it is why "never make somebody sign a transaction to log
//! in" is too blunt a rule: what must never be signed is a *submittable* one.

use sha2::{Digest, Sha256};
use stellar_xdr::curr::{
    BytesM, Limits, ManageDataOp, Memo, Operation, OperationBody, Preconditions, ReadXdr,
    SequenceNumber, String64, TimeBounds, TimePoint, Transaction, TransactionEnvelope,
    TransactionExt, TransactionV1Envelope, VecM, WriteXdr,
};

use crate::error::ChainError;
use crate::keys::muxed_account;
use crate::network::Network;
use crate::tx::signature_payload_hash;

/// How long a challenge is valid for.
pub const CHALLENGE_SECONDS: u64 = 300;

/// The `manage_data` key. Naming the domain puts it in front of the person approving it.
pub const DATA_NAME: &str = "settleflow auth";

/// Build the challenge a wallet is asked to sign.
///
/// Deterministic: the same address, nonce and window always produce the same transaction, so
/// verification rebuilds it rather than trusting anything the client sends back.
pub fn build_challenge(
    address: &str,
    nonce: &[u8; 32],
    issued_at: u64,
    network: &Network,
) -> Result<(TransactionEnvelope, [u8; 32]), ChainError> {
    let data_name: String64 = String64(
        DATA_NAME
            .as_bytes()
            .to_vec()
            .try_into()
            .map_err(|_| ChainError::Encode("data name too long".into()))?,
    );

    // The nonce, hex encoded: manage_data values are at most 64 bytes, and hex keeps it printable
    // in whatever the wallet shows the user.
    let value: BytesM<64> = hex::encode(nonce)
        .into_bytes()
        .try_into()
        .map_err(|_| ChainError::Encode("nonce value too long".into()))?;

    let op = Operation {
        source_account: None,
        body: OperationBody::ManageData(ManageDataOp {
            data_name,
            data_value: Some(stellar_xdr::curr::DataValue(value)),
        }),
    };

    let operations: VecM<Operation, 100> = vec![op]
        .try_into()
        .map_err(|_| ChainError::Encode("too many operations".into()))?;

    let tx = Transaction {
        source_account: muxed_account(address)?,
        fee: 100,
        // ZERO. This is what makes the challenge unsubmittable, and it is the entire safety
        // argument for asking a wallet to sign it.
        seq_num: SequenceNumber(0),
        cond: Preconditions::Time(TimeBounds {
            min_time: TimePoint(issued_at),
            max_time: TimePoint(issued_at + CHALLENGE_SECONDS),
        }),
        memo: Memo::None,
        operations,
        ext: TransactionExt::V0,
    };

    let hash = signature_payload_hash(&tx, network)?;
    Ok((
        TransactionEnvelope::Tx(TransactionV1Envelope {
            tx,
            signatures: VecM::default(),
        }),
        hash,
    ))
}

/// Encode a challenge for the wire.
pub fn to_xdr(envelope: &TransactionEnvelope) -> Result<String, ChainError> {
    envelope
        .to_xdr_base64(Limits::none())
        .map_err(|e| ChainError::Encode(e.to_string()))
}

/// Pull every signature out of a signed envelope a wallet returned.
///
/// The transaction inside is deliberately ignored: verification rebuilds the expected challenge
/// from the stored nonce and checks the signature against *that* hash. Trusting the returned
/// transaction would let a caller sign something of their own choosing and have it accepted.
pub fn signatures_from(signed_xdr: &str) -> Result<Vec<Vec<u8>>, ChainError> {
    let envelope = TransactionEnvelope::from_xdr_base64(signed_xdr, Limits::none())
        .map_err(|e| ChainError::Decode(e.to_string()))?;

    let sigs = match envelope {
        TransactionEnvelope::Tx(v1) => v1.signatures.to_vec(),
        TransactionEnvelope::TxV0(v0) => v0.signatures.to_vec(),
        TransactionEnvelope::TxFeeBump(fb) => fb.signatures.to_vec(),
    };
    Ok(sigs.into_iter().map(|s| s.signature.to_vec()).collect())
}

/// The hash a signer signed, for a challenge we rebuilt ourselves.
pub fn challenge_hash(
    address: &str,
    nonce: &[u8; 32],
    issued_at: u64,
    network: &Network,
) -> Result<[u8; 32], ChainError> {
    let (_, hash) = build_challenge(address, nonce, issued_at, network)?;
    Ok(hash)
}

/// `sha256` of arbitrary bytes, for callers that need it alongside the above.
pub fn sha256(bytes: &[u8]) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(bytes);
    let d = h.finalize();
    let mut out = [0u8; 32];
    out.copy_from_slice(&d);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};

    fn key(seed: u8) -> SigningKey {
        SigningKey::from_bytes(&[seed; 32])
    }

    fn address(seed: u8) -> String {
        crate::keys::encode_account(key(seed).verifying_key().to_bytes())
    }

    #[test]
    fn the_challenge_carries_sequence_number_zero() {
        // The entire safety argument. If this ever becomes non-zero, the challenge becomes a
        // submittable transaction and asking a wallet to sign it becomes dangerous.
        let (env, _) =
            build_challenge(&address(1), &[7u8; 32], 1_700_000_000, &Network::testnet()).unwrap();
        let TransactionEnvelope::Tx(v1) = &env else {
            panic!("expected a v1 envelope");
        };
        assert_eq!(v1.tx.seq_num.0, 0);
    }

    #[test]
    fn the_challenge_is_deterministic() {
        // Verification rebuilds it rather than trusting what comes back, which only works if the
        // same inputs always produce the same transaction.
        let a =
            challenge_hash(&address(1), &[7u8; 32], 1_700_000_000, &Network::testnet()).unwrap();
        let b =
            challenge_hash(&address(1), &[7u8; 32], 1_700_000_000, &Network::testnet()).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn a_different_nonce_is_a_different_challenge() {
        let n = &Network::testnet();
        assert_ne!(
            challenge_hash(&address(1), &[7u8; 32], 1_700_000_000, n).unwrap(),
            challenge_hash(&address(1), &[8u8; 32], 1_700_000_000, n).unwrap()
        );
    }

    #[test]
    fn a_different_address_is_a_different_challenge() {
        let n = &Network::testnet();
        assert_ne!(
            challenge_hash(&address(1), &[7u8; 32], 1_700_000_000, n).unwrap(),
            challenge_hash(&address(2), &[7u8; 32], 1_700_000_000, n).unwrap()
        );
    }

    #[test]
    fn a_challenge_signed_by_its_owner_verifies() {
        let sk = key(3);
        let addr = address(3);
        let nonce = [9u8; 32];
        let (env, hash) =
            build_challenge(&addr, &nonce, 1_700_000_000, &Network::testnet()).unwrap();

        // What a wallet does: sign the transaction and hand back the envelope.
        let TransactionEnvelope::Tx(v1) = env else {
            panic!("expected a v1 envelope");
        };
        let signed = TransactionEnvelope::Tx(TransactionV1Envelope {
            tx: v1.tx,
            signatures: vec![stellar_xdr::curr::DecoratedSignature {
                hint: stellar_xdr::curr::SignatureHint(crate::keys::signature_hint(
                    sk.verifying_key().to_bytes(),
                )),
                signature: stellar_xdr::curr::Signature(
                    sk.sign(&hash).to_bytes().to_vec().try_into().unwrap(),
                ),
            }]
            .try_into()
            .unwrap(),
        });

        let xdr = to_xdr(&signed).unwrap();
        let sigs = signatures_from(&xdr).unwrap();
        assert_eq!(sigs.len(), 1);
        assert!(
            crate::keys::verify_signature(sk.verifying_key().to_bytes(), &hash, &sigs[0]).is_ok()
        );
    }

    #[test]
    fn a_challenge_signed_by_somebody_else_does_not_verify() {
        let owner = address(3);
        let stranger = key(4);
        let nonce = [9u8; 32];
        let hash = challenge_hash(&owner, &nonce, 1_700_000_000, &Network::testnet()).unwrap();
        let sig = stranger.sign(&hash).to_bytes().to_vec();

        let owner_key = crate::keys::decode_account(&owner).unwrap();
        assert!(crate::keys::verify_signature(owner_key, &hash, &sig).is_err());
    }

    #[test]
    fn a_malformed_envelope_is_refused_rather_than_panicking() {
        assert!(signatures_from("not base64 xdr").is_err());
        assert!(signatures_from("").is_err());
    }
}
