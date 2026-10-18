//! # settle-signature
//!
//! Deliverable 2: reserve the treasury's sequence, collect approver signatures, detect the
//! threshold, and decide what to do when the outcome is unclear.
//!
//! Everything here is a pure decision over data the caller supplies. Persistence and network calls
//! live in `settle-backend`. That split is not architectural tidiness — it is what makes the three
//! decisions where money is lost testable without a chain: the sequence lock, the signature checks,
//! and the response to `tx_bad_seq`.
//!
//! The specification is `../../../docs/06-signature-service.md`.

pub mod collector;
pub mod policy;
pub mod reconcile;
pub mod sequence;

pub use collector::{Approval, Collector};
pub use policy::{sow1_policy, Policy, Signer};
pub use reconcile::{needs_reconciliation, resolve, ChainLookup, Resolution};
pub use sequence::{reserve, InFlightSettlement, Reservation};
