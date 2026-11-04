//! Finance cost thresholds: when to open and resolve an alert.
//!
//! An alert fires once when the observed value crosses the limit from below,
//! stays open while the value remains at or above it, and resolves itself when
//! the value drops back below (e.g. entries corrected). A later crossing opens
//! a new alert.

use rust_decimal::prelude::ToPrimitive;
use rust_decimal::{Decimal, RoundingStrategy};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum ThresholdRule {
    /// Limit in the project currency's minor units.
    Absolute { amount_minor: i64 },
    /// Limit as a percentage of the project's labour budget, e.g. 80.
    PercentOfBudget { percent: Decimal },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AlertLevel {
    Info,
    Warn,
    Critical,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Evaluation {
    /// Rule cannot be evaluated (percent rule without a positive budget).
    Inactive,
    /// Value crossed the limit and no alert is open: open one and notify.
    Fire { limit_minor: i64 },
    /// Value fell below the limit while an alert was open: resolve it.
    Resolve,
    /// Nothing to do.
    NoChange,
}

/// The limit in project-currency minor units, or `None` if it cannot be computed.
pub fn limit_minor(rule: ThresholdRule, budget_minor: Option<i64>) -> Option<i64> {
    match rule {
        ThresholdRule::Absolute { amount_minor } => Some(amount_minor),
        ThresholdRule::PercentOfBudget { percent } => {
            let budget = budget_minor.filter(|b| *b > 0)?;
            if percent <= Decimal::ZERO {
                return None;
            }
            (Decimal::from(budget) * percent / Decimal::from(100))
                .round_dp_with_strategy(0, RoundingStrategy::ToPositiveInfinity)
                .to_i64()
        }
    }
}

/// Decide what to do for one threshold given the current project total.
pub fn evaluate(
    rule: ThresholdRule,
    budget_minor: Option<i64>,
    observed_minor: i64,
    alert_open: bool,
) -> Evaluation {
    let Some(limit) = limit_minor(rule, budget_minor) else {
        return Evaluation::Inactive;
    };
    let over = observed_minor >= limit;
    match (over, alert_open) {
        (true, false) => Evaluation::Fire { limit_minor: limit },
        (false, true) => Evaluation::Resolve,
        _ => Evaluation::NoChange,
    }
}

/// Share of budget used, in whole percent (rounded down), for hours-only views.
/// PMs see this instead of amounts.
pub fn budget_used_percent(spent_minor: i64, budget_minor: Option<i64>) -> Option<u32> {
    let budget = budget_minor.filter(|b| *b > 0)?;
    let pct = i128::from(spent_minor.max(0)) * 100 / i128::from(budget);
    u32::try_from(pct).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    const PCT80: ThresholdRule = ThresholdRule::PercentOfBudget { percent: dec!(80) };

    #[test]
    fn percent_limit_from_budget() {
        assert_eq!(limit_minor(PCT80, Some(6_000_000)), Some(4_800_000));
        assert_eq!(limit_minor(PCT80, Some(0)), None);
        assert_eq!(limit_minor(PCT80, None), None);
        // fractional limits round up so "80%" is never undercut
        assert_eq!(limit_minor(PCT80, Some(1)), Some(1));
    }

    #[test]
    fn fires_once_then_holds_then_resolves_then_refires() {
        let b = Some(10_000);
        assert_eq!(evaluate(PCT80, b, 7_999, false), Evaluation::NoChange);
        assert_eq!(
            evaluate(PCT80, b, 8_000, false),
            Evaluation::Fire { limit_minor: 8_000 }
        );
        assert_eq!(evaluate(PCT80, b, 9_500, true), Evaluation::NoChange);
        assert_eq!(evaluate(PCT80, b, 7_000, true), Evaluation::Resolve);
        assert_eq!(
            evaluate(PCT80, b, 8_100, false),
            Evaluation::Fire { limit_minor: 8_000 }
        );
    }

    #[test]
    fn absolute_rule_ignores_budget() {
        let r = ThresholdRule::Absolute {
            amount_minor: 7_000_000,
        };
        assert_eq!(
            evaluate(r, None, 7_000_000, false),
            Evaluation::Fire {
                limit_minor: 7_000_000
            }
        );
    }

    #[test]
    fn percent_rule_without_budget_is_inactive() {
        assert_eq!(
            evaluate(PCT80, None, 1_000_000, false),
            Evaluation::Inactive
        );
    }

    #[test]
    fn budget_percent_for_pm_view() {
        assert_eq!(budget_used_percent(4_560_000, Some(6_000_000)), Some(76));
        assert_eq!(budget_used_percent(1, None), None);
        assert_eq!(budget_used_percent(-5, Some(100)), Some(0));
    }
}
