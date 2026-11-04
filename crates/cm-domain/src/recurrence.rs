//! When a routine template produces an item. Deliberately small: daily,
//! weekdays (Monday–Friday) or weekly on chosen ISO weekdays (1 = Monday).

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Recurrence {
    Daily,
    Weekdays,
    Weekly(Vec<u8>),
}

impl Recurrence {
    pub fn parse(kind: &str, weekdays: &[i16]) -> Option<Self> {
        Some(match kind {
            "daily" => Self::Daily,
            "weekdays" => Self::Weekdays,
            "weekly" => {
                let days: Vec<u8> = weekdays
                    .iter()
                    .filter_map(|d| u8::try_from(*d).ok())
                    .filter(|d| (1..=7).contains(d))
                    .collect();
                if days.is_empty() {
                    return None;
                }
                Self::Weekly(days)
            }
            _ => return None,
        })
    }

    /// Does the template fire on a day with this ISO weekday (1 = Monday … 7 = Sunday)?
    pub fn fires_on(&self, iso_weekday: u8) -> bool {
        match self {
            Self::Daily => true,
            Self::Weekdays => (1..=5).contains(&iso_weekday),
            Self::Weekly(days) => days.contains(&iso_weekday),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn weekdays_skip_weekend() {
        let r = Recurrence::parse("weekdays", &[]).expect("valid");
        assert!(r.fires_on(1) && r.fires_on(5));
        assert!(!r.fires_on(6) && !r.fires_on(7));
    }

    #[test]
    fn weekly_needs_days() {
        assert_eq!(Recurrence::parse("weekly", &[]), None);
        assert_eq!(Recurrence::parse("weekly", &[0, 9]), None);
        let r = Recurrence::parse("weekly", &[1, 3]).expect("valid");
        assert!(r.fires_on(3) && !r.fires_on(2));
        assert_eq!(Recurrence::parse("hourly", &[]), None);
    }
}
