//! Money.
//!
//! Every amount in this system is an integer count of stroops. Stellar uses seven decimal places
//! for every asset, so one whole unit is 10,000,000 stroops.
//!
//! There is no floating point anywhere in this module, and there must not be one anywhere in a
//! money path. `numeric(30,7)` in Postgres, `i128` here, `String` on the wire — JSON numbers are
//! IEEE doubles, which silently lose precision at magnitudes this system reaches routinely.

use core::fmt;

use crate::error::{DomainError, Field};

/// Stroops in one whole unit. Stellar's decimal places, stated once.
pub const STROOPS_PER_UNIT: i128 = 10_000_000;

/// Stellar's decimal places.
pub const DECIMALS: u32 = 7;

/// An amount of some asset, in stroops.
///
/// Construction is fallible and goes through parsing or an explicit constructor, so a negative or
/// absurd value cannot be conjured by arithmetic on a bare integer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Stroops(i128);

impl Stroops {
    /// The largest amount accepted anywhere.
    ///
    /// Stellar amounts are `i64` at the classic protocol layer, so nothing beyond that can ever
    /// settle. Rejecting it here means the failure lands on a form field rather than at the ledger
    /// after two people have signed.
    pub const MAX: i128 = i64::MAX as i128;

    /// A positive amount, in stroops.
    pub fn new(stroops: i128) -> Result<Self, DomainError> {
        if stroops <= 0 {
            return Err(DomainError::AmountNotPositive);
        }
        if stroops > Self::MAX {
            return Err(DomainError::AmountTooLarge);
        }
        Ok(Self(stroops))
    }

    /// Whole units, for tests and fixtures.
    pub fn from_units(whole: i64) -> Result<Self, DomainError> {
        Self::new(i128::from(whole) * STROOPS_PER_UNIT)
    }

    pub fn as_i128(self) -> i128 {
        self.0
    }

    pub fn as_i64(self) -> i64 {
        // Construction caps at i64::MAX, so this cannot truncate.
        self.0 as i64
    }

    /// Parse the decimal string the API and the interface use.
    ///
    /// Accepts at most seven decimal places, and no exponent, no sign, no separators. The
    /// permissiveness people expect from a number parser is exactly what turns "4.5e3" into a
    /// payment nobody intended.
    pub fn parse(input: &str) -> Result<Self, DomainError> {
        let s = input.trim();
        if s.is_empty() {
            return Err(DomainError::AmountMalformed);
        }
        if !s
            .bytes()
            .all(|b| b.is_ascii_digit() || b == b'.' || b == b'_' || b == b',')
        {
            return Err(DomainError::AmountMalformed);
        }
        // Thousands separators are common in pasted values and mean nothing numerically.
        let s: String = s.chars().filter(|c| *c != ',' && *c != '_').collect();

        let mut parts = s.splitn(3, '.');
        let whole = parts.next().unwrap_or_default();
        let frac = parts.next().unwrap_or("");
        if parts.next().is_some() {
            return Err(DomainError::AmountMalformed);
        }
        if whole.is_empty() && frac.is_empty() {
            return Err(DomainError::AmountMalformed);
        }
        if frac.len() > DECIMALS as usize {
            return Err(DomainError::AmountTooPrecise);
        }

        let whole_v: i128 = if whole.is_empty() {
            0
        } else {
            whole.parse().map_err(|_| DomainError::AmountTooLarge)?
        };
        // Right-pad the fraction to exactly seven places, so "1.5" is 15,000,000 stroops.
        let mut frac_padded = frac.to_string();
        while frac_padded.len() < DECIMALS as usize {
            frac_padded.push('0');
        }
        let frac_v: i128 = if frac_padded.is_empty() {
            0
        } else {
            frac_padded
                .parse()
                .map_err(|_| DomainError::AmountMalformed)?
        };

        let total = whole_v
            .checked_mul(STROOPS_PER_UNIT)
            .and_then(|w| w.checked_add(frac_v))
            .ok_or(DomainError::AmountTooLarge)?;

        Self::new(total)
    }
}

impl fmt::Display for Stroops {
    /// Always seven decimal places, so the wire format is unambiguous and columns line up.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let whole = self.0 / STROOPS_PER_UNIT;
        let frac = self.0 % STROOPS_PER_UNIT;
        write!(f, "{whole}.{frac:07}")
    }
}

impl serde::Serialize for Stroops {
    /// A string on the wire. JSON numbers are doubles.
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string())
    }
}

impl<'de> serde::Deserialize<'de> for Stroops {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        Stroops::parse(&s).map_err(serde::de::Error::custom)
    }
}

/// The two assets SOW 1 supports.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum Asset {
    Usdc,
    Xlm,
}

impl Asset {
    pub fn code(self) -> &'static str {
        match self {
            Asset::Usdc => "USDC",
            Asset::Xlm => "XLM",
        }
    }

    /// A trustline is required to receive an issued asset. Native XLM needs none.
    ///
    /// This is what makes a destination check meaningful: paying USDC to an address without a USDC
    /// trustline fails at the ledger, and it fails at submit — after both approvers have signed.
    pub fn requires_trustline(self) -> bool {
        matches!(self, Asset::Usdc)
    }

    pub fn parse(code: &str) -> Result<Self, DomainError> {
        match code.trim().to_ascii_uppercase().as_str() {
            "USDC" => Ok(Asset::Usdc),
            "XLM" => Ok(Asset::Xlm),
            _ => Err(DomainError::AssetNotSupported {
                field: Field::AssetCode,
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn whole_units_convert_to_stroops() {
        assert_eq!(
            Stroops::from_units(4_500).unwrap().as_i128(),
            45_000_000_000
        );
        assert_eq!(Stroops::from_units(1).unwrap().as_i128(), 10_000_000);
    }

    #[test]
    fn parsing_accepts_up_to_seven_decimal_places() {
        assert_eq!(Stroops::parse("4500").unwrap().as_i128(), 45_000_000_000);
        assert_eq!(Stroops::parse("4500.0").unwrap().as_i128(), 45_000_000_000);
        assert_eq!(Stroops::parse("0.0000001").unwrap().as_i128(), 1);
        assert_eq!(Stroops::parse("1.5").unwrap().as_i128(), 15_000_000);
        assert_eq!(Stroops::parse(".5").unwrap().as_i128(), 5_000_000);
    }

    #[test]
    fn parsing_refuses_an_eighth_decimal_place() {
        assert_eq!(
            Stroops::parse("1.00000001").unwrap_err(),
            DomainError::AmountTooPrecise
        );
    }

    #[test]
    fn parsing_refuses_zero_and_negative() {
        assert_eq!(
            Stroops::parse("0").unwrap_err(),
            DomainError::AmountNotPositive
        );
        assert_eq!(
            Stroops::parse("0.0000000").unwrap_err(),
            DomainError::AmountNotPositive
        );
        // A minus sign is not a digit, so it is malformed rather than negative.
        assert_eq!(
            Stroops::parse("-1").unwrap_err(),
            DomainError::AmountMalformed
        );
    }

    #[test]
    fn parsing_refuses_what_a_permissive_number_parser_would_accept() {
        for bad in ["4.5e3", "1 000", "abc", "", "1.2.3", "0x10", "＄5"] {
            assert!(
                Stroops::parse(bad).is_err(),
                "expected {bad:?} to be refused"
            );
        }
    }

    #[test]
    fn separators_people_paste_are_tolerated() {
        assert_eq!(
            Stroops::parse("4,500.50").unwrap().as_i128(),
            45_005_000_000
        );
        assert_eq!(Stroops::parse("4_500").unwrap().as_i128(), 45_000_000_000);
    }

    #[test]
    fn an_amount_beyond_the_classic_protocol_limit_is_refused() {
        assert_eq!(
            Stroops::new(Stroops::MAX + 1).unwrap_err(),
            DomainError::AmountTooLarge
        );
        assert!(Stroops::new(Stroops::MAX).is_ok());
    }

    #[test]
    fn display_always_shows_seven_places_so_columns_line_up() {
        assert_eq!(
            Stroops::from_units(4_500).unwrap().to_string(),
            "4500.0000000"
        );
        assert_eq!(Stroops::new(1).unwrap().to_string(), "0.0000001");
        assert_eq!(Stroops::new(15_000_000).unwrap().to_string(), "1.5000000");
    }

    #[test]
    fn a_parsed_amount_round_trips_through_its_own_display() {
        for s in ["4500.0000000", "0.0000001", "1.5000000", "999999.9999999"] {
            assert_eq!(Stroops::parse(s).unwrap().to_string(), s);
        }
    }

    #[test]
    fn assets_know_whether_they_need_a_trustline() {
        assert!(Asset::Usdc.requires_trustline());
        assert!(!Asset::Xlm.requires_trustline());
        assert_eq!(Asset::parse("usdc").unwrap(), Asset::Usdc);
        assert!(Asset::parse("EURC").is_err());
    }
}
