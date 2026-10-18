//! Failures in the chain layer.

/// Everything that can go wrong building, hashing or verifying a transaction.
#[derive(Debug, thiserror::Error)]
pub enum ChainError {
    #[error("{0} is not a valid Stellar account address")]
    InvalidAddress(String),
    #[error("an asset code must be 1 to 12 characters, got {0:?}")]
    InvalidAssetCode(String),
    #[error("the amount does not fit in the classic protocol's i64")]
    AmountOutOfRange,
    #[error("timebounds must span at least one second")]
    EmptyTimebounds,
    #[error("could not encode XDR: {0}")]
    Encode(String),
    #[error("could not decode XDR: {0}")]
    Decode(String),
    #[error("the signature is not 64 bytes")]
    MalformedSignature,
    #[error("the public key is not a valid ed25519 key")]
    MalformedPublicKey,
    #[error("the signature does not verify against this transaction hash")]
    BadSignature,
}
