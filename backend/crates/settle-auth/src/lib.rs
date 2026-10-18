//! # settle-auth
//!
//! Sign-in for an internal tool where nobody has a password. Somebody connects a Stellar wallet,
//! signs a nonce, and receives a session token.
//!
//! The specification is `../../../docs/09-api-contract.md`.

pub mod challenge;
pub mod error;
pub mod session;

pub use challenge::{generate_nonce, verify_signed_challenge, Challenge, CHALLENGE_TTL_SECONDS};
pub use error::AuthError;
pub use session::{issue, verify, Session, SESSION_SECONDS};
