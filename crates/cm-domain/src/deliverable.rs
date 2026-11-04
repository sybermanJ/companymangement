//! Required deliverables that supervisors attach to routine or project items.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeliverableKind {
    Text,
    Image,
    Audio,
    Video,
    File,
    GitCommit,
    Link,
    Checklist,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeliverableSpec {
    pub kind: DeliverableKind,
    pub min_count: u32,
    pub required: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Shortfall {
    pub kind: DeliverableKind,
    pub needed: u32,
    pub have: u32,
}

/// Required specs that are not yet met, given the submitted count for each spec
/// (same order as `specs`).
pub fn shortfalls(specs: &[DeliverableSpec], submitted: &[u32]) -> Vec<Shortfall> {
    specs
        .iter()
        .zip(submitted.iter().copied().chain(std::iter::repeat(0)))
        .filter(|(s, have)| s.required && *have < s.min_count.max(1))
        .map(|(s, have)| Shortfall {
            kind: s.kind,
            needed: s.min_count.max(1),
            have,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_missing_required_items_only() {
        let specs = vec![
            DeliverableSpec {
                kind: DeliverableKind::Text,
                min_count: 1,
                required: true,
            },
            DeliverableSpec {
                kind: DeliverableKind::GitCommit,
                min_count: 2,
                required: true,
            },
            DeliverableSpec {
                kind: DeliverableKind::Image,
                min_count: 1,
                required: false,
            },
        ];
        let s = shortfalls(&specs, &[1, 1]);
        assert_eq!(
            s,
            vec![Shortfall {
                kind: DeliverableKind::GitCommit,
                needed: 2,
                have: 1
            }]
        );
        assert!(shortfalls(&specs, &[1, 2, 0]).is_empty());
    }

    #[test]
    fn zero_min_count_means_at_least_one() {
        let specs = vec![DeliverableSpec {
            kind: DeliverableKind::Text,
            min_count: 0,
            required: true,
        }];
        assert_eq!(shortfalls(&specs, &[]).len(), 1);
    }
}
