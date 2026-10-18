//! Domain errors, and the wire envelope they become.
//!
//! Every error the API returns has the same shape, from every endpoint. `code` is a stable
//! machine-readable identifier that never changes — renaming one breaks the interface's copy table
//! and any operator runbook that maps codes to actions. `message` is for a person and may be
//! translated. `field` is what lets the interface put the error under the input that caused it
//! rather than in a generic toast.
//!
//! See `../../../../docs/09-api-contract.md`.

use serde::{Deserialize, Serialize};

/// The input a validation error belongs to, so the interface can highlight it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Field {
    PayeeAddress,
    Amount,
    AssetCode,
    Reference,
    IdempotencyKey,
    Signature,
    SignerAddress,
}

impl Field {
    pub fn as_str(self) -> &'static str {
        match self {
            Field::PayeeAddress => "payee_address",
            Field::Amount => "amount",
            Field::AssetCode => "asset_code",
            Field::Reference => "reference",
            Field::IdempotencyKey => "idempotency_key",
            Field::Signature => "signature",
            Field::SignerAddress => "signer_address",
        }
    }
}

/// Everything the rules can refuse.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DomainError {
    #[error("the amount must be greater than zero")]
    AmountNotPositive,
    #[error("the amount has more than seven decimal places")]
    AmountTooPrecise,
    #[error("the amount is larger than Stellar can settle")]
    AmountTooLarge,
    #[error("the amount is not a plain decimal number")]
    AmountMalformed,
    #[error("that is not a valid Stellar address")]
    InvalidAddress { field: Field },
    #[error("that asset is not supported")]
    AssetNotSupported { field: Field },
    #[error("the reference must be between 1 and 64 characters")]
    ReferenceLength,
    #[error("the reference contains a control character")]
    ReferenceControlCharacter,
    #[error("the expiry must be in the future")]
    ExpiryNotInFuture,
    #[error("the expiry is further out than {max_days} days")]
    ExpiryTooFar { max_days: u32 },
    #[error("that transition is not allowed from the current status")]
    IllegalTransition {
        from: &'static str,
        to: &'static str,
    },
}

impl DomainError {
    /// The stable machine-readable code. **Never renamed.**
    pub fn code(&self) -> &'static str {
        match self {
            DomainError::AmountNotPositive
            | DomainError::AmountTooPrecise
            | DomainError::AmountTooLarge
            | DomainError::AmountMalformed => "INVALID_AMOUNT",
            DomainError::InvalidAddress { .. } => "INVALID_ADDRESS",
            DomainError::AssetNotSupported { .. } => "ASSET_NOT_SUPPORTED",
            DomainError::ReferenceLength | DomainError::ReferenceControlCharacter => {
                "INVALID_REFERENCE"
            }
            DomainError::ExpiryNotInFuture | DomainError::ExpiryTooFar { .. } => "INVALID_EXPIRY",
            DomainError::IllegalTransition { .. } => "ILLEGAL_TRANSITION",
        }
    }

    /// Which input to highlight, when there is one.
    pub fn field(&self) -> Option<Field> {
        match self {
            DomainError::AmountNotPositive
            | DomainError::AmountTooPrecise
            | DomainError::AmountTooLarge
            | DomainError::AmountMalformed => Some(Field::Amount),
            DomainError::InvalidAddress { field } | DomainError::AssetNotSupported { field } => {
                Some(*field)
            }
            DomainError::ReferenceLength | DomainError::ReferenceControlCharacter => {
                Some(Field::Reference)
            }
            _ => None,
        }
    }

    /// Whether the client may retry the same call unchanged.
    ///
    /// A validation failure never is: retrying identical bad input produces an identical refusal
    /// and teaches the user nothing.
    pub fn retriable(&self) -> bool {
        false
    }
}

/// The envelope every error crosses the wire in.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorBody {
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub field: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,
    pub retriable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trace_id: Option<String>,
}

/// The single-key wrapper, so a response body is `{"error": {...}}` and never bare.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorEnvelope {
    pub error: ErrorBody,
}

impl ErrorEnvelope {
    pub fn new(code: &str, message: impl Into<String>) -> Self {
        Self {
            error: ErrorBody {
                code: code.to_string(),
                message: message.into(),
                field: None,
                details: None,
                retriable: false,
                trace_id: None,
            },
        }
    }

    pub fn with_field(mut self, field: Field) -> Self {
        self.error.field = Some(field.as_str().to_string());
        self
    }

    pub fn with_details(mut self, details: serde_json::Value) -> Self {
        self.error.details = Some(details);
        self
    }

    pub fn retriable(mut self, retriable: bool) -> Self {
        self.error.retriable = retriable;
        self
    }

    /// Attach the identifier that connects this response to a log line and a chain event.
    pub fn with_trace(mut self, trace_id: impl Into<String>) -> Self {
        self.error.trace_id = Some(trace_id.into());
        self
    }
}

impl From<DomainError> for ErrorEnvelope {
    fn from(e: DomainError) -> Self {
        let mut env = ErrorEnvelope::new(e.code(), e.to_string()).retriable(e.retriable());
        if let Some(f) = e.field() {
            env = env.with_field(f);
        }
        env
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_amount_error_shares_one_stable_code() {
        for e in [
            DomainError::AmountNotPositive,
            DomainError::AmountTooPrecise,
            DomainError::AmountTooLarge,
            DomainError::AmountMalformed,
        ] {
            assert_eq!(e.code(), "INVALID_AMOUNT");
            assert_eq!(e.field(), Some(Field::Amount));
        }
    }

    #[test]
    fn an_envelope_serialises_under_a_single_error_key() {
        let env: ErrorEnvelope = DomainError::ReferenceLength.into();
        let json = serde_json::to_value(&env).unwrap();
        assert_eq!(json["error"]["code"], "INVALID_REFERENCE");
        assert_eq!(json["error"]["field"], "reference");
        assert_eq!(json["error"]["retriable"], false);
        // Absent optional members are omitted rather than sent as null.
        assert!(json["error"].get("details").is_none());
        assert!(json["error"].get("trace_id").is_none());
    }

    #[test]
    fn a_trace_id_survives_onto_the_envelope() {
        let env = ErrorEnvelope::new("SYNCING", "Synchronising").with_trace("01J8XKQ2");
        let json = serde_json::to_value(&env).unwrap();
        assert_eq!(json["error"]["trace_id"], "01J8XKQ2");
    }

    #[test]
    fn validation_failures_are_never_retriable() {
        assert!(!DomainError::AmountNotPositive.retriable());
        assert!(!DomainError::ReferenceLength.retriable());
    }
}
