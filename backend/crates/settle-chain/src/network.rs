//! Network identity.
//!
//! The network id is hashed into every transaction's signature payload. That is what stops a
//! signature collected on testnet from being replayable on mainnet — and it is why the passphrase
//! is configuration rather than a constant.

use sha2::{Digest, Sha256};

/// The testnet passphrase. SOW 1 is testnet only; mainnet is explicitly out of scope.
pub const TESTNET: &str = "Test SDF Network ; September 2015";

/// A Stellar network, identified by its passphrase.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Network {
    passphrase: String,
    id: [u8; 32],
}

impl Network {
    pub fn new(passphrase: impl Into<String>) -> Self {
        let passphrase = passphrase.into();
        let mut hasher = Sha256::new();
        hasher.update(passphrase.as_bytes());
        let digest = hasher.finalize();
        let mut id = [0u8; 32];
        id.copy_from_slice(&digest);
        Self { passphrase, id }
    }

    pub fn testnet() -> Self {
        Self::new(TESTNET)
    }

    pub fn passphrase(&self) -> &str {
        &self.passphrase
    }

    /// `sha256(passphrase)` — the value mixed into every signature payload.
    pub fn id(&self) -> [u8; 32] {
        self.id
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The published testnet network id. If this ever changes, every signature the service builds
    /// is being computed against the wrong network and nothing will settle.
    #[test]
    fn the_testnet_network_id_matches_the_published_value() {
        let n = Network::testnet();
        assert_eq!(
            hex::encode(n.id()),
            "cee0302d59844d32bdca915c8203dd44b33fbb7edc19051ea37abedf28ecd472"
        );
    }

    #[test]
    fn a_different_passphrase_is_a_different_network() {
        assert_ne!(
            Network::testnet().id(),
            Network::new("Public Global Stellar Network ; September 2015").id()
        );
    }
}
