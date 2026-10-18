//! Session tokens.
//!
//! A hand-rolled HS256 JWT, deliberately not a JWT library. There is nothing to negotiate here:
//! one algorithm, one key, one issuer. A library brings an `alg` header field that a caller can
//! set to `none`, and the entire class of algorithm-confusion vulnerabilities that follows from
//! it. This implementation ignores the header's `alg` and verifies HMAC-SHA256 unconditionally.

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;

use crate::error::AuthError;

type HmacSha256 = Hmac<Sha256>;

/// How long a session lasts. Long enough for a working day of approvals; short enough that a
/// stolen token is not indefinite.
pub const SESSION_SECONDS: u64 = 8 * 3_600;

/// What a session asserts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Session {
    /// The address that proved control of itself.
    pub wallet: String,
    /// Issued at, unix seconds.
    pub iat: u64,
    /// Expires at, unix seconds.
    pub exp: u64,
}

#[derive(Serialize, Deserialize)]
struct Header {
    alg: &'static str,
    typ: &'static str,
}

/// Issue a token for an address that has just proved control of itself.
pub fn issue(wallet: &str, key: &[u8], now: u64) -> String {
    let header = Header {
        alg: "HS256",
        typ: "JWT",
    };
    let session = Session {
        wallet: wallet.to_string(),
        iat: now,
        exp: now + SESSION_SECONDS,
    };

    let h = URL_SAFE_NO_PAD.encode(serde_json::to_vec(&header).expect("header serialises"));
    let p = URL_SAFE_NO_PAD.encode(serde_json::to_vec(&session).expect("session serialises"));
    let signing_input = format!("{h}.{p}");
    let sig = URL_SAFE_NO_PAD.encode(mac(key, signing_input.as_bytes()));
    format!("{signing_input}.{sig}")
}

/// Verify a token and return what it asserts.
///
/// The signature is checked **before** the payload is parsed, and before expiry is considered.
/// Trusting any field of an unverified token — even to decide whether to bother verifying it — is
/// how a forged token gets a foothold.
pub fn verify(token: &str, key: &[u8], now: u64) -> Result<Session, AuthError> {
    let mut parts = token.split('.');
    let (Some(h), Some(p), Some(s), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return Err(AuthError::SessionMalformed);
    };

    let signing_input = format!("{h}.{p}");
    let expected = mac(key, signing_input.as_bytes());
    let provided = URL_SAFE_NO_PAD
        .decode(s)
        .map_err(|_| AuthError::SessionMalformed)?;

    if !constant_time_eq(&expected, &provided) {
        return Err(AuthError::SessionBadSignature);
    }

    let payload = URL_SAFE_NO_PAD
        .decode(p)
        .map_err(|_| AuthError::SessionMalformed)?;
    let session: Session =
        serde_json::from_slice(&payload).map_err(|_| AuthError::SessionMalformed)?;

    if now >= session.exp {
        return Err(AuthError::SessionExpired);
    }
    Ok(session)
}

fn mac(key: &[u8], message: &[u8]) -> Vec<u8> {
    let mut m = HmacSha256::new_from_slice(key).expect("HMAC accepts a key of any length");
    m.update(message);
    m.finalize().into_bytes().to_vec()
}

/// Compare in time independent of where the first difference falls.
///
/// A short-circuiting comparison leaks the position of the first wrong byte, which is enough to
/// reconstruct a signature one byte at a time.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: &[u8] = b"a-development-session-key-never-a-default-in-production";
    const WALLET: &str = "GABCDEFGHIJKLMNOPQRSTUVWXYZ234567ABCDEFGHIJKLMNOPQRSTUV";

    #[test]
    fn a_token_round_trips() {
        let now = 1_700_000_000;
        let t = issue(WALLET, KEY, now);
        let s = verify(&t, KEY, now).unwrap();
        assert_eq!(s.wallet, WALLET);
        assert_eq!(s.exp, now + SESSION_SECONDS);
    }

    #[test]
    fn a_token_signed_with_another_key_is_refused() {
        let now = 1_700_000_000;
        let t = issue(WALLET, KEY, now);
        assert!(matches!(
            verify(&t, b"a different key entirely", now),
            Err(AuthError::SessionBadSignature)
        ));
    }

    #[test]
    fn a_tampered_payload_is_refused() {
        let now = 1_700_000_000;
        let t = issue(WALLET, KEY, now);
        let mut parts: Vec<&str> = t.split('.').collect();

        // Re-encode the payload claiming a different wallet, keeping the original signature.
        let forged = Session {
            wallet: "GSOMEONEELSE".into(),
            iat: now,
            exp: now + SESSION_SECONDS,
        };
        let encoded = URL_SAFE_NO_PAD.encode(serde_json::to_vec(&forged).unwrap());
        parts[1] = &encoded;
        let tampered = parts.join(".");

        assert!(matches!(
            verify(&tampered, KEY, now),
            Err(AuthError::SessionBadSignature)
        ));
    }

    #[test]
    fn an_alg_none_token_is_refused() {
        // The classic JWT library vulnerability. This implementation never reads `alg`, so a
        // token claiming `none` fails on its signature like anything else.
        let now = 1_700_000_000;
        let header = URL_SAFE_NO_PAD.encode(br#"{"alg":"none","typ":"JWT"}"#);
        let payload = URL_SAFE_NO_PAD.encode(
            serde_json::to_vec(&Session {
                wallet: WALLET.into(),
                iat: now,
                exp: now + SESSION_SECONDS,
            })
            .unwrap(),
        );
        let forged = format!("{header}.{payload}.");
        assert!(verify(&forged, KEY, now).is_err());
    }

    #[test]
    fn an_expired_token_is_refused() {
        let now = 1_700_000_000;
        let t = issue(WALLET, KEY, now);
        assert!(verify(&t, KEY, now + SESSION_SECONDS - 1).is_ok());
        assert!(matches!(
            verify(&t, KEY, now + SESSION_SECONDS),
            Err(AuthError::SessionExpired)
        ));
    }

    #[test]
    fn a_malformed_token_is_refused_rather_than_panicking() {
        let now = 1_700_000_000;
        for bad in ["", ".", "a.b", "a.b.c.d", "not-a-token", "a.b.!!!"] {
            assert!(
                verify(bad, KEY, now).is_err(),
                "expected {bad:?} to be refused"
            );
        }
    }

    #[test]
    fn comparison_does_not_short_circuit() {
        assert!(constant_time_eq(b"abcd", b"abcd"));
        assert!(!constant_time_eq(b"abcd", b"abce"));
        assert!(!constant_time_eq(b"abcd", b"zbcd"));
        assert!(!constant_time_eq(b"abcd", b"abcde"));
    }
}
