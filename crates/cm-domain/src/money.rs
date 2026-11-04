//! Money as integer minor units, and labour cost from minutes × hourly rate.

use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum MoneyError {
    #[error("currency code must be three uppercase ASCII letters, got {0:?}")]
    InvalidCurrency(String),
    #[error("amount overflow")]
    Overflow,
    #[error("hourly rate must not be negative")]
    NegativeRate,
    #[error("currency mismatch: {0} vs {1}")]
    CurrencyMismatch(Currency, Currency),
}

/// ISO 4217 alphabetic code.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Currency([u8; 3]);

impl Currency {
    pub fn parse(code: &str) -> Result<Self, MoneyError> {
        let b = code.as_bytes();
        if b.len() == 3 && b.iter().all(u8::is_ascii_uppercase) {
            Ok(Self([b[0], b[1], b[2]]))
        } else {
            Err(MoneyError::InvalidCurrency(code.to_owned()))
        }
    }

    pub fn as_str(&self) -> &str {
        // Constructed only from ASCII uppercase bytes.
        std::str::from_utf8(&self.0).unwrap_or("???")
    }

    /// Number of minor-unit digits (ISO 4217). Defaults to 2.
    pub fn minor_exponent(&self) -> u32 {
        match self.as_str() {
            "JPY" | "KRW" | "VND" | "CLP" | "ISK" | "UGX" | "XAF" | "XOF" | "PYG" => 0,
            "BHD" | "KWD" | "OMR" | "JOD" | "TND" | "IQD" | "LYD" => 3,
            _ => 2,
        }
    }
}

impl fmt::Display for Currency {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}
impl fmt::Debug for Currency {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Currency({})", self.as_str())
    }
}
impl TryFrom<String> for Currency {
    type Error = MoneyError;
    fn try_from(s: String) -> Result<Self, Self::Error> {
        Self::parse(&s)
    }
}
impl From<Currency> for String {
    fn from(c: Currency) -> Self {
        c.as_str().to_owned()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Money {
    pub minor: i64,
    pub currency: Currency,
}

impl Money {
    pub fn new(minor: i64, currency: Currency) -> Self {
        Self { minor, currency }
    }

    pub fn checked_add(self, other: Money) -> Result<Money, MoneyError> {
        if self.currency != other.currency {
            return Err(MoneyError::CurrencyMismatch(self.currency, other.currency));
        }
        let minor = self
            .minor
            .checked_add(other.minor)
            .ok_or(MoneyError::Overflow)?;
        Ok(Money::new(minor, self.currency))
    }
}

/// Divide with round-half-to-even (banker's rounding). `den` must be > 0.
pub(crate) fn div_round_half_even(num: i128, den: i128) -> i128 {
    debug_assert!(den > 0);
    let q = num.div_euclid(den);
    let r = num.rem_euclid(den);
    let twice = r * 2;
    if twice > den || (twice == den && q % 2 != 0) {
        q + 1
    } else {
        q
    }
}

/// Labour cost of `minutes` at `rate` per hour, rounded half-even to the
/// rate currency's minor unit. This is the value frozen on each work log.
pub fn labour_cost(minutes: u32, rate_per_hour: Money) -> Result<Money, MoneyError> {
    if rate_per_hour.minor < 0 {
        return Err(MoneyError::NegativeRate);
    }
    let raw = i128::from(minutes) * i128::from(rate_per_hour.minor);
    let minor = i64::try_from(div_round_half_even(raw, 60)).map_err(|_| MoneyError::Overflow)?;
    Ok(Money::new(minor, rate_per_hour.currency))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cny(minor: i64) -> Money {
        Money::new(minor, Currency::parse("CNY").expect("valid"))
    }

    #[test]
    fn parses_currency_codes() {
        assert!(Currency::parse("USD").is_ok());
        assert!(Currency::parse("usd").is_err());
        assert!(Currency::parse("US").is_err());
        assert_eq!(Currency::parse("JPY").expect("valid").minor_exponent(), 0);
        assert_eq!(Currency::parse("KWD").expect("valid").minor_exponent(), 3);
        assert_eq!(Currency::parse("EUR").expect("valid").minor_exponent(), 2);
    }

    #[test]
    fn three_hours_at_200_is_600() {
        assert_eq!(labour_cost(180, cny(20_000)).expect("ok"), cny(60_000));
    }

    #[test]
    fn rounds_half_to_even() {
        // 1 minute at 0.30/h = 0.5 minor -> 0 (even); 3 minutes at 0.30/h = 1.5 -> 2
        assert_eq!(labour_cost(1, cny(30)).expect("ok").minor, 0);
        assert_eq!(labour_cost(3, cny(30)).expect("ok").minor, 2);
        // 0.7 rounds up, 0.3 rounds down
        assert_eq!(labour_cost(7, cny(6)).expect("ok").minor, 1);
        assert_eq!(labour_cost(3, cny(6)).expect("ok").minor, 0);
    }

    #[test]
    fn rejects_negative_rate_and_mixed_currency() {
        assert_eq!(labour_cost(60, cny(-1)), Err(MoneyError::NegativeRate));
        let usd = Money::new(1, Currency::parse("USD").expect("valid"));
        assert!(cny(1).checked_add(usd).is_err());
    }

    #[test]
    fn half_even_helper_handles_negatives() {
        assert_eq!(div_round_half_even(-3, 2), -2); // -1.5 -> -2
        assert_eq!(div_round_half_even(-1, 2), 0); // -0.5 -> 0
        assert_eq!(div_round_half_even(5, 2), 2); // 2.5 -> 2
    }
}
