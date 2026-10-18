//! # settle-domain
//!
//! Every business rule in SettleFlow, as pure functions over plain types.
//!
//! **This crate depends on nothing.** No I/O, no async, no database, no clock — a caller passes
//! `now` in. That is what lets these rules be unit-tested in microseconds, and it is why nothing
//! above this crate may re-implement a rule that lives here. When a rule changes, it changes in one
//! file and this crate's whole suite runs again.
//!
//! The specification is `../../../docs/02-modules.md` and `../../../docs/09-api-contract.md`.

pub mod address;
pub mod amount;
pub mod error;
pub mod request;
pub mod settlement;

pub use address::StellarAddress;
pub use amount::{Asset, Stroops, DECIMALS, STROOPS_PER_UNIT};
pub use error::{DomainError, ErrorBody, ErrorEnvelope, Field};
pub use request::{
    reference_hash, validate, validate_expiry, validate_reference, NewRequest, RequestId,
    RequestStatus, ValidatedRequest, MAX_EXPIRY_DAYS, MIN_EXPIRY_SECONDS,
};
pub use settlement::{
    decide_submit, may_submit, Progress, SettlementStatus, SignatureRejection, SubmitAction,
    SubmitOutcome,
};
