//! The payment request: its identifier, its validation, and its state machine.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::address::StellarAddress;
use crate::amount::{Asset, Stroops};
use crate::error::{DomainError, Field};

/// The shortest expiry a request may carry.
pub const MIN_EXPIRY_SECONDS: u64 = 300;
/// The longest. Beyond this a request is clutter that somebody eventually pays by mistake.
pub const MAX_EXPIRY_DAYS: u32 = 30;

/// A request identifier: 32 bytes, and the SAME value on chain and in Postgres.
///
/// One value for both eliminates an entire class of mapping bugs. It is derived rather than
/// random, which is what makes retry-after-timeout idempotent at the contract level: the same
/// inputs derive the same id, the contract's `RequestExists` gate catches the second attempt, and
/// the backend reads the first one back instead of creating a duplicate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RequestId([u8; 32]);

impl RequestId {
    /// `sha256(business_id ‖ idempotency_key)`.
    ///
    /// Both halves matter. Without the business, two tenants generating the same UUID would
    /// collide; without the idempotency key, a business could only ever have one request.
    pub fn derive(business_id: Uuid, idempotency_key: Uuid) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(business_id.as_bytes());
        hasher.update(idempotency_key.as_bytes());
        let digest = hasher.finalize();
        let mut out = [0u8; 32];
        out.copy_from_slice(&digest);
        Self(out)
    }

    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    pub fn to_hex(self) -> String {
        hex::encode(self.0)
    }

    pub fn parse_hex(s: &str) -> Option<Self> {
        let raw = hex::decode(s.trim()).ok()?;
        let arr: [u8; 32] = raw.try_into().ok()?;
        Some(Self(arr))
    }
}

impl core::fmt::Display for RequestId {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&hex::encode(self.0))
    }
}

impl Serialize for RequestId {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_hex())
    }
}

impl<'de> Deserialize<'de> for RequestId {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        RequestId::parse_hex(&s)
            .ok_or_else(|| serde::de::Error::custom("a request id is 32 hex-encoded bytes"))
    }
}

/// The hash of a reference, which is what goes on chain.
///
/// The plaintext — a purchase order or invoice number — stays in Postgres. On the ledger it would
/// be permanently public and permanently linked to a vendor and an amount, which publishes the
/// business's supplier relationships and payment sizes to anyone who looks.
pub fn reference_hash(reference: &str) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(reference.as_bytes());
    let digest = hasher.finalize();
    let mut out = [0u8; 32];
    out.copy_from_slice(&digest);
    out
}

/// A request's status. Mirrors `ReqStatus` on the contract, plus `Expired`, which is derived.
///
/// The contract has no `Expired`: it compares `ledger.timestamp()` against `expiry` on every call.
/// Materialising it here is what lets the interface filter on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RequestStatus {
    Pending,
    Approved,
    Paid,
    Cancelled,
    Expired,
}

impl RequestStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            RequestStatus::Pending => "pending",
            RequestStatus::Approved => "approved",
            RequestStatus::Paid => "paid",
            RequestStatus::Cancelled => "cancelled",
            RequestStatus::Expired => "expired",
        }
    }

    /// `Paid` and `Cancelled` are terminal on chain. A mistake after that point is a new request,
    /// never an edit.
    pub fn is_terminal(self) -> bool {
        matches!(self, RequestStatus::Paid | RequestStatus::Cancelled)
    }

    /// The transitions the contract permits, mirrored here so the backend refuses an impossible
    /// call before spending a network round trip on it.
    pub fn can_transition_to(self, next: RequestStatus) -> bool {
        use RequestStatus::*;
        matches!(
            (self, next),
            (Pending, Approved)
                | (Pending, Paid)
                | (Pending, Cancelled)
                | (Approved, Paid)
                | (Approved, Cancelled)
        )
    }

    pub fn transition(self, next: RequestStatus) -> Result<RequestStatus, DomainError> {
        if self.can_transition_to(next) {
            Ok(next)
        } else {
            Err(DomainError::IllegalTransition {
                from: self.as_str(),
                to: next.as_str(),
            })
        }
    }
}

/// What the interface sends to create a payment.
#[derive(Debug, Clone, Deserialize)]
pub struct NewRequest {
    pub payee_address: String,
    pub amount: String,
    pub asset_code: String,
    pub reference: String,
}

/// The same thing, validated. Constructing one of these is the only way past the rules.
#[derive(Debug, Clone)]
pub struct ValidatedRequest {
    pub payee: StellarAddress,
    pub amount: Stroops,
    pub asset: Asset,
    pub reference: String,
    pub reference_hash: [u8; 32],
    pub expiry: u64,
}

/// Validate a submitted request against every rule in one pass.
///
/// `now` and `expiry` are unix seconds. The caller supplies both, because this crate performs no
/// I/O and has no clock of its own — which is also what makes it trivially testable at a boundary.
pub fn validate(
    input: &NewRequest,
    now: u64,
    expiry: u64,
) -> Result<ValidatedRequest, DomainError> {
    let payee = StellarAddress::parse(&input.payee_address, Field::PayeeAddress)?;
    let amount = Stroops::parse(&input.amount)?;
    let asset = Asset::parse(&input.asset_code)?;
    let reference = validate_reference(&input.reference)?;
    validate_expiry(now, expiry)?;

    Ok(ValidatedRequest {
        payee,
        amount,
        asset,
        reference_hash: reference_hash(&reference),
        reference,
        expiry,
    })
}

/// 1 to 64 characters, no control characters.
///
/// The length is an accounting constraint. The control-character rule is not cosmetic: a reference
/// is rendered on the approver screen and written into logs, and a stray newline or escape
/// sequence there is a way to make one payment look like another.
pub fn validate_reference(reference: &str) -> Result<String, DomainError> {
    let s = reference.trim();
    let count = s.chars().count();
    if count == 0 || count > 64 {
        return Err(DomainError::ReferenceLength);
    }
    if s.chars().any(|c| c.is_control()) {
        return Err(DomainError::ReferenceControlCharacter);
    }
    Ok(s.to_string())
}

/// The expiry window.
pub fn validate_expiry(now: u64, expiry: u64) -> Result<(), DomainError> {
    if expiry <= now.saturating_add(MIN_EXPIRY_SECONDS) {
        return Err(DomainError::ExpiryNotInFuture);
    }
    let max = now.saturating_add(u64::from(MAX_EXPIRY_DAYS) * 86_400);
    if expiry > max {
        return Err(DomainError::ExpiryTooFar {
            max_days: MAX_EXPIRY_DAYS,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn address() -> String {
        let mut s = String::from("G");
        let alphabet: Vec<char> = "ABCDEFGHIJKLMNOPQRSTUVWXYZ234567".chars().collect();
        for i in 0..55 {
            s.push(alphabet[i % alphabet.len()]);
        }
        s
    }

    fn good(now: u64) -> (NewRequest, u64) {
        (
            NewRequest {
                payee_address: address(),
                amount: "4500.0000000".into(),
                asset_code: "USDC".into(),
                reference: "PO-2026-0417".into(),
            },
            now + 3_600,
        )
    }

    // ---- the identifier ---------------------------------------------------

    #[test]
    fn the_same_inputs_always_derive_the_same_id() {
        let biz = Uuid::from_u128(1);
        let key = Uuid::from_u128(2);
        assert_eq!(RequestId::derive(biz, key), RequestId::derive(biz, key));
    }

    #[test]
    fn a_different_idempotency_key_derives_a_different_id() {
        let biz = Uuid::from_u128(1);
        assert_ne!(
            RequestId::derive(biz, Uuid::from_u128(2)),
            RequestId::derive(biz, Uuid::from_u128(3))
        );
    }

    #[test]
    fn a_different_business_derives_a_different_id() {
        let key = Uuid::from_u128(2);
        assert_ne!(
            RequestId::derive(Uuid::from_u128(1), key),
            RequestId::derive(Uuid::from_u128(9), key)
        );
    }

    #[test]
    fn an_id_round_trips_through_hex() {
        let id = RequestId::derive(Uuid::from_u128(1), Uuid::from_u128(2));
        assert_eq!(RequestId::parse_hex(&id.to_hex()).unwrap(), id);
        assert_eq!(id.to_hex().len(), 64);
    }

    #[test]
    fn a_malformed_id_does_not_parse() {
        assert!(RequestId::parse_hex("").is_none());
        assert!(RequestId::parse_hex("abcd").is_none());
        assert!(RequestId::parse_hex(&"z".repeat(64)).is_none());
    }

    // ---- validation -------------------------------------------------------

    #[test]
    fn a_well_formed_request_validates() {
        let now = 1_700_000_000;
        let (req, expiry) = good(now);
        let v = validate(&req, now, expiry).unwrap();
        assert_eq!(v.amount.as_i128(), 45_000_000_000);
        assert_eq!(v.asset, Asset::Usdc);
        assert_eq!(v.reference, "PO-2026-0417");
        assert_eq!(v.reference_hash, reference_hash("PO-2026-0417"));
    }

    #[test]
    fn every_constraint_has_its_own_refusal() {
        let now = 1_700_000_000;

        let (mut r, e) = good(now);
        r.payee_address = "not-an-address".into();
        assert_eq!(validate(&r, now, e).unwrap_err().code(), "INVALID_ADDRESS");

        let (mut r, e) = good(now);
        r.amount = "0".into();
        assert_eq!(validate(&r, now, e).unwrap_err().code(), "INVALID_AMOUNT");

        let (mut r, e) = good(now);
        r.amount = "1.00000001".into();
        assert_eq!(validate(&r, now, e).unwrap_err().code(), "INVALID_AMOUNT");

        let (mut r, e) = good(now);
        r.asset_code = "EURC".into();
        assert_eq!(
            validate(&r, now, e).unwrap_err().code(),
            "ASSET_NOT_SUPPORTED"
        );

        let (mut r, e) = good(now);
        r.reference = String::new();
        assert_eq!(
            validate(&r, now, e).unwrap_err().code(),
            "INVALID_REFERENCE"
        );

        let (r, _) = good(now);
        assert_eq!(
            validate(&r, now, now + 60).unwrap_err().code(),
            "INVALID_EXPIRY"
        );
    }

    #[test]
    fn a_reference_of_exactly_sixty_four_characters_is_accepted() {
        assert!(validate_reference(&"A".repeat(64)).is_ok());
        assert_eq!(
            validate_reference(&"A".repeat(65)).unwrap_err(),
            DomainError::ReferenceLength
        );
    }

    #[test]
    fn a_reference_carrying_a_control_character_is_refused() {
        assert_eq!(
            validate_reference("PO-2026\n0417").unwrap_err(),
            DomainError::ReferenceControlCharacter
        );
        assert_eq!(
            validate_reference("PO\u{0}0417").unwrap_err(),
            DomainError::ReferenceControlCharacter
        );
    }

    #[test]
    fn the_expiry_window_is_closed_at_both_ends() {
        let now = 1_700_000_000;
        assert!(validate_expiry(now, now + MIN_EXPIRY_SECONDS).is_err());
        assert!(validate_expiry(now, now + MIN_EXPIRY_SECONDS + 1).is_ok());
        assert!(validate_expiry(now, now + 30 * 86_400).is_ok());
        assert!(validate_expiry(now, now + 30 * 86_400 + 1).is_err());
    }

    #[test]
    fn an_expiry_that_would_overflow_the_clock_is_refused_not_wrapped() {
        assert!(validate_expiry(u64::MAX - 1, u64::MAX).is_err());
    }

    // ---- the state machine ------------------------------------------------

    #[test]
    fn the_lifecycle_the_statement_of_work_names_is_permitted() {
        use RequestStatus::*;
        assert!(Pending.can_transition_to(Approved));
        assert!(Approved.can_transition_to(Paid));
        assert!(Pending.can_transition_to(Cancelled));
        assert!(Approved.can_transition_to(Cancelled));
    }

    #[test]
    fn terminal_states_have_no_way_out() {
        use RequestStatus::*;
        for terminal in [Paid, Cancelled] {
            assert!(terminal.is_terminal());
            for next in [Pending, Approved, Paid, Cancelled] {
                assert!(
                    !terminal.can_transition_to(next),
                    "{terminal:?} must not reach {next:?}"
                );
            }
        }
    }

    #[test]
    fn a_status_cannot_go_backwards() {
        use RequestStatus::*;
        assert!(!Approved.can_transition_to(Pending));
    }

    #[test]
    fn an_illegal_transition_names_both_ends() {
        let e = RequestStatus::Paid
            .transition(RequestStatus::Pending)
            .unwrap_err();
        assert_eq!(
            e,
            DomainError::IllegalTransition {
                from: "paid",
                to: "pending"
            }
        );
    }
}
