//! Addresses, keys and signatures.

use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use stellar_xdr::curr::{AccountId, Hash, MuxedAccount, PublicKey, Uint256};

use crate::error::ChainError;

/// Decode a `G…` address into its raw 32-byte ed25519 public key.
///
/// This is a real strkey decode, checksum included — unlike the shape check in `settle-domain`,
/// which deliberately performs no cryptography so that it can stay dependency-free. A transposed
/// character passes the shape check and fails here, which is the layering working as intended.
pub fn decode_account(address: &str) -> Result<[u8; 32], ChainError> {
    let parsed = stellar_strkey::ed25519::PublicKey::from_string(address.trim())
        .map_err(|_| ChainError::InvalidAddress(address.to_string()))?;
    Ok(parsed.0)
}

/// Encode a raw ed25519 public key back into a `G…` address.
pub fn encode_account(key: [u8; 32]) -> String {
    // strkey 0.0.18 returns a heapless string; the rest of the system speaks `String`.
    stellar_strkey::ed25519::PublicKey(key)
        .to_string()
        .as_str()
        .to_owned()
}

pub fn account_id(address: &str) -> Result<AccountId, ChainError> {
    Ok(AccountId(PublicKey::PublicKeyTypeEd25519(Uint256(
        decode_account(address)?,
    ))))
}

pub fn muxed_account(address: &str) -> Result<MuxedAccount, ChainError> {
    Ok(MuxedAccount::Ed25519(Uint256(decode_account(address)?)))
}

/// Verify an approver's signature against a transaction hash.
///
/// The hash is what a signer actually signed. It is **not** derived from the request or from the
/// settlement id — it is the signature payload of this exact transaction, including its sequence
/// number, so a signature collected for one transaction cannot be reused for a rebuilt one.
pub fn verify_signature(
    public_key: [u8; 32],
    tx_hash: &[u8; 32],
    signature: &[u8],
) -> Result<(), ChainError> {
    let key = VerifyingKey::from_bytes(&public_key).map_err(|_| ChainError::MalformedPublicKey)?;
    let sig_bytes: [u8; 64] = signature
        .try_into()
        .map_err(|_| ChainError::MalformedSignature)?;
    let sig = Signature::from_bytes(&sig_bytes);
    key.verify(tx_hash, &sig)
        .map_err(|_| ChainError::BadSignature)
}

/// Verify a signature over an arbitrary message.
///
/// Separate from `verify_signature`, which takes a 32-byte transaction hash. The distinction is
/// deliberate: a transaction hash is always 32 bytes and mixing the two would let a caller pass a
/// message where a hash belongs, which is how signatures end up being checked against the wrong
/// thing.
pub fn verify_message(
    public_key: [u8; 32],
    message: &[u8],
    signature: &[u8],
) -> Result<(), ChainError> {
    let key = VerifyingKey::from_bytes(&public_key).map_err(|_| ChainError::MalformedPublicKey)?;
    let sig_bytes: [u8; 64] = signature
        .try_into()
        .map_err(|_| ChainError::MalformedSignature)?;
    let sig = Signature::from_bytes(&sig_bytes);
    key.verify(message, &sig)
        .map_err(|_| ChainError::BadSignature)
}

/// The four-byte hint Stellar uses to match a signature to a signer.
///
/// It is the **last** four bytes of the public key, and it is only a hint: it narrows the search,
/// it does not authorise anything. Getting it wrong produces a transaction the network refuses
/// after every approver has already signed.
pub fn signature_hint(public_key: [u8; 32]) -> [u8; 4] {
    let mut hint = [0u8; 4];
    hint.copy_from_slice(&public_key[28..32]);
    hint
}

pub(crate) fn hash(bytes: [u8; 32]) -> Hash {
    Hash(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};

    fn keypair(seed: u8) -> SigningKey {
        SigningKey::from_bytes(&[seed; 32])
    }

    #[test]
    fn an_address_round_trips_through_strkey() {
        let key = keypair(7).verifying_key().to_bytes();
        let address = encode_account(key);
        assert!(address.starts_with('G'));
        assert_eq!(address.len(), 56);
        assert_eq!(decode_account(&address).unwrap(), key);
    }

    #[test]
    fn a_transposed_character_fails_the_checksum() {
        let address = encode_account(keypair(7).verifying_key().to_bytes());
        let mut broken: Vec<char> = address.chars().collect();
        broken.swap(10, 11);
        let broken: String = broken.into_iter().collect();
        // Same length, same alphabet — the shape check in settle-domain would accept this. The
        // checksum is what catches it.
        assert_eq!(broken.len(), 56);
        assert!(decode_account(&broken).is_err());
    }

    #[test]
    fn a_valid_signature_verifies() {
        let sk = keypair(3);
        let tx_hash = [0x11u8; 32];
        let sig = sk.sign(&tx_hash);
        assert!(verify_signature(sk.verifying_key().to_bytes(), &tx_hash, &sig.to_bytes()).is_ok());
    }

    #[test]
    fn a_signature_over_a_different_hash_is_refused() {
        let sk = keypair(3);
        let signed = sk.sign(&[0x11u8; 32]);
        let other_hash = [0x22u8; 32];
        assert!(matches!(
            verify_signature(
                sk.verifying_key().to_bytes(),
                &other_hash,
                &signed.to_bytes()
            ),
            Err(ChainError::BadSignature)
        ));
    }

    #[test]
    fn a_signature_from_another_signer_is_refused() {
        let signer = keypair(3);
        let stranger = keypair(4);
        let tx_hash = [0x11u8; 32];
        let sig = signer.sign(&tx_hash);
        assert!(matches!(
            verify_signature(
                stranger.verifying_key().to_bytes(),
                &tx_hash,
                &sig.to_bytes()
            ),
            Err(ChainError::BadSignature)
        ));
    }

    #[test]
    fn a_malformed_signature_is_refused_rather_than_panicking() {
        let sk = keypair(3);
        assert!(matches!(
            verify_signature(sk.verifying_key().to_bytes(), &[0u8; 32], &[0u8; 10]),
            Err(ChainError::MalformedSignature)
        ));
    }

    #[test]
    fn the_hint_is_the_last_four_bytes_of_the_key() {
        let mut key = [0u8; 32];
        key[28..32].copy_from_slice(&[0xDE, 0xAD, 0xBE, 0xEF]);
        assert_eq!(signature_hint(key), [0xDE, 0xAD, 0xBE, 0xEF]);
    }
}
