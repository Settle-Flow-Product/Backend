//! Authentication failures.
//!
//! **Every one of these is reported to the caller identically**, with the same message and the
//! same latency. The variants exist so operators can tell from a log why a sign-in failed;
//! nothing distinguishing them reaches the client, because that would reveal whether an address is
//! enrolled.

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum AuthError {
    #[error("authentication failed")]
    ChallengeExpired,
    #[error("authentication failed")]
    ChallengeNotFound,
    #[error("authentication failed")]
    DomainMismatch,
    #[error("authentication failed")]
    NetworkMismatch,
    #[error("authentication failed")]
    InvalidAddress,
    #[error("authentication failed")]
    BadSignature,
    #[error("authentication failed")]
    SessionExpired,
    #[error("authentication failed")]
    SessionMalformed,
    #[error("authentication failed")]
    SessionBadSignature,
    #[error("the system has no source of entropy")]
    NoEntropy,
}

impl AuthError {
    /// What a client sees. One code for every failure above.
    pub fn public_code(self) -> &'static str {
        "AUTHENTICATION_FAILED"
    }

    /// What an operator sees in a log line. Never sent to a client.
    pub fn operator_reason(self) -> &'static str {
        match self {
            AuthError::ChallengeExpired => "challenge_expired",
            AuthError::ChallengeNotFound => "challenge_not_found_or_replayed",
            AuthError::DomainMismatch => "domain_mismatch",
            AuthError::NetworkMismatch => "network_mismatch",
            AuthError::InvalidAddress => "invalid_address",
            AuthError::BadSignature => "bad_signature",
            AuthError::SessionExpired => "session_expired",
            AuthError::SessionMalformed => "session_malformed",
            AuthError::SessionBadSignature => "session_bad_signature",
            AuthError::NoEntropy => "no_entropy",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_failure_looks_identical_to_a_client() {
        let all = [
            AuthError::ChallengeExpired,
            AuthError::ChallengeNotFound,
            AuthError::DomainMismatch,
            AuthError::NetworkMismatch,
            AuthError::InvalidAddress,
            AuthError::BadSignature,
            AuthError::SessionExpired,
        ];
        for e in all {
            assert_eq!(e.to_string(), "authentication failed");
            assert_eq!(e.public_code(), "AUTHENTICATION_FAILED");
        }
    }

    #[test]
    fn operators_can_still_tell_them_apart() {
        assert_ne!(
            AuthError::ChallengeExpired.operator_reason(),
            AuthError::BadSignature.operator_reason()
        );
    }
}
