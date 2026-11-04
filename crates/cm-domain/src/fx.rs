//! Converting labour cost from a person's pay currency to a project's budget currency.

use crate::money::{Currency, Money, MoneyError};
use rust_decimal::prelude::ToPrimitive;
use rust_decimal::{Decimal, RoundingStrategy};

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum FxError {
    #[error("exchange rate must be positive")]
    NonPositiveRate,
    #[error("no exchange rate from {from} to {to} on the entry date")]
    MissingRate { from: Currency, to: Currency },
    #[error(transparent)]
    Money(#[from] MoneyError),
}

/// `rate` = units of `to` per one unit of `amount.currency` (e.g. CNY→USD 0.1405).
pub fn convert(amount: Money, to: Currency, rate: Decimal) -> Result<Money, FxError> {
    if amount.currency == to {
        return Ok(amount);
    }
    if rate <= Decimal::ZERO {
        return Err(FxError::NonPositiveRate);
    }
    let from_exp = amount.currency.minor_exponent();
    let to_exp = to.minor_exponent();
    let major = Decimal::from(amount.minor) / Decimal::from(10_i64.pow(from_exp));
    let target_minor = (major * rate * Decimal::from(10_i64.pow(to_exp)))
        .round_dp_with_strategy(0, RoundingStrategy::MidpointNearestEven);
    let minor = target_minor.to_i64().ok_or(MoneyError::Overflow)?;
    Ok(Money::new(minor, to))
}

/// Convert when a rate may be missing; a missing rate is a distinct, recoverable
/// state (the work log is saved as "pending FX" and Finance is asked for the rate).
pub fn convert_opt(amount: Money, to: Currency, rate: Option<Decimal>) -> Result<Money, FxError> {
    if amount.currency == to {
        return Ok(amount);
    }
    match rate {
        Some(r) => convert(amount, to, r),
        None => Err(FxError::MissingRate {
            from: amount.currency,
            to,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    fn c(s: &str) -> Currency {
        Currency::parse(s).expect("valid")
    }

    #[test]
    fn cny_to_usd() {
        // CNY 600.00 at 0.1405 = USD 84.30
        let m = convert(Money::new(60_000, c("CNY")), c("USD"), dec!(0.1405)).expect("ok");
        assert_eq!(m, Money::new(8_430, c("USD")));
    }

    #[test]
    fn handles_zero_decimal_currencies() {
        // JPY 6,500 at 0.0066 = USD 42.90
        let m = convert(Money::new(6_500, c("JPY")), c("USD"), dec!(0.0066)).expect("ok");
        assert_eq!(m, Money::new(4_290, c("USD")));
        // USD 10.00 at 151.37 = JPY 1,514 (1513.7 rounds to 1514)
        let m = convert(Money::new(1_000, c("USD")), c("JPY"), dec!(151.37)).expect("ok");
        assert_eq!(m, Money::new(1_514, c("JPY")));
    }

    #[test]
    fn same_currency_is_identity_even_without_rate() {
        let m = Money::new(123, c("EUR"));
        assert_eq!(convert_opt(m, c("EUR"), None).expect("ok"), m);
    }

    #[test]
    fn missing_and_bad_rates_are_errors() {
        let m = Money::new(100, c("EUR"));
        assert_eq!(
            convert_opt(m, c("USD"), None),
            Err(FxError::MissingRate {
                from: c("EUR"),
                to: c("USD")
            })
        );
        assert_eq!(convert(m, c("USD"), dec!(0)), Err(FxError::NonPositiveRate));
    }

    #[test]
    fn rounds_half_even() {
        // 0.05 USD * 0.5 = 0.025 -> 2 minor (2.5 -> 2)
        let m = convert(Money::new(5, c("USD")), c("EUR"), dec!(0.5)).expect("ok");
        assert_eq!(m.minor, 2);
    }
}
