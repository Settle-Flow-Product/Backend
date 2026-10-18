//! Stellar account addresses.
//!
//! One regular expression, checked at three layers: the form field, the API, and the database
//! column. Repeating it is deliberate — the form check is for the person typing, the API check is
//! because a client cannot be trusted, and the column check is because application code has bugs
//! and constraints do not.

use crate::error::{DomainError, Field};

/// A validated Stellar public key (`G…`).
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize)]
pub struct StellarAddress(String);

impl StellarAddress {
    /// Validate the shape of a `G…` account address.
    ///
    /// This checks the alphabet and the length — the same predicate the `stellar_addr` domain uses
    /// in Postgres. It does **not** verify the strkey checksum or that the account exists; those
    /// belong to `settle-chain`, which can decode, and to a live account lookup, which needs the
    /// network.
    pub fn parse(input: &str, field: Field) -> Result<Self, DomainError> {
        let s = input.trim();
        let err = || DomainError::InvalidAddress { field };

        if s.len() != 56 {
            return Err(err());
        }
        let mut chars = s.chars();
        if chars.next() != Some('G') {
            return Err(err());
        }
        // RFC 4648 base32: uppercase A–Z and digits 2–7. No 0, 1, 8 or 9, which is what makes a
        // transposed character detectable rather than silently valid.
        if !chars.all(|c| c.is_ascii_uppercase() || ('2'..='7').contains(&c)) {
            return Err(err());
        }
        Ok(Self(s.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The `GDX7…P4LM` form the interface shows.
    ///
    /// Never used for comparison — only for display. Two different accounts can share an
    /// abbreviation, which is exactly why the full value is what gets copied.
    pub fn abbreviated(&self) -> String {
        format!("{}…{}", &self.0[..4], &self.0[self.0.len() - 4..])
    }
}

impl core::fmt::Display for StellarAddress {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALID: &str = "GDX7ABCDEFGHIJKLMNOPQRSTUVWXYZ234567ABCDEFGHIJKLMNOP4LM";

    fn valid() -> String {
        // 56 characters exactly: 'G' plus 55 from the base32 alphabet.
        let mut s = String::from("G");
        let alphabet: Vec<char> = "ABCDEFGHIJKLMNOPQRSTUVWXYZ234567".chars().collect();
        for i in 0..55 {
            s.push(alphabet[i % alphabet.len()]);
        }
        s
    }

    #[test]
    fn a_well_formed_address_parses() {
        let a = StellarAddress::parse(&valid(), Field::PayeeAddress).unwrap();
        assert_eq!(a.as_str().len(), 56);
    }

    #[test]
    fn surrounding_whitespace_is_tolerated() {
        let padded = format!("  {}  ", valid());
        assert!(StellarAddress::parse(&padded, Field::PayeeAddress).is_ok());
    }

    #[test]
    fn the_wrong_length_is_refused() {
        assert!(StellarAddress::parse("G", Field::PayeeAddress).is_err());
        assert!(StellarAddress::parse(&valid()[..55], Field::PayeeAddress).is_err());
        assert!(StellarAddress::parse(&format!("{}A", valid()), Field::PayeeAddress).is_err());
    }

    #[test]
    fn a_contract_address_is_not_an_account_address() {
        // C… is a contract. SOW 1 pays vendor ACCOUNTS, and accepting a contract here would build
        // a payment operation that cannot succeed.
        let c: String = std::iter::once('C')
            .chain(valid().chars().skip(1))
            .collect();
        assert!(StellarAddress::parse(&c, Field::PayeeAddress).is_err());
    }

    #[test]
    fn characters_outside_the_base32_alphabet_are_refused() {
        // 0, 1, 8 and 9 are excluded from the alphabet precisely so a typo is detectable.
        for bad in ['0', '1', '8', '9', 'a', '-'] {
            let mut s = valid();
            s.replace_range(10..11, &bad.to_string());
            assert!(
                StellarAddress::parse(&s, Field::PayeeAddress).is_err(),
                "expected {bad} to be refused"
            );
        }
    }

    #[test]
    fn the_error_names_the_field_that_caused_it() {
        let e = StellarAddress::parse("nope", Field::PayeeAddress).unwrap_err();
        assert_eq!(e.field(), Some(Field::PayeeAddress));
        assert_eq!(e.code(), "INVALID_ADDRESS");
    }

    #[test]
    fn abbreviation_is_for_display_only() {
        let a = StellarAddress::parse(&valid(), Field::PayeeAddress).unwrap();
        let abbr = a.abbreviated();
        assert!(abbr.starts_with("GABC"));
        assert!(abbr.contains('…'));
        // The abbreviation is lossy, which is why it never round-trips back into an address.
        assert!(StellarAddress::parse(&abbr, Field::PayeeAddress).is_err());
        let _ = VALID;
    }
}
