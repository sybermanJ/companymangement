//! Work-item lifecycle. Projects tasks, routine tasks and requests share it.

use crate::deliverable::Shortfall;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkItemKind {
    Phase,
    Task,
    Milestone,
    Routine,
    Request,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Todo,
    InProgress,
    Blocked,
    InReview,
    Done,
    Cancelled,
}

impl Status {
    pub fn is_open(self) -> bool {
        !matches!(self, Status::Done | Status::Cancelled)
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum StatusError {
    #[error("cannot move from {from:?} to {to:?}")]
    NotAllowed { from: Status, to: Status },
    #[error("required deliverables are missing")]
    DeliverablesMissing(Vec<Shortfall>),
}

/// Validate a status change. `shortfalls` are the unmet required deliverables;
/// a supervisor override (with a logged reason) skips that check.
pub fn transition(
    from: Status,
    to: Status,
    shortfalls: Vec<Shortfall>,
    supervisor_override: bool,
) -> Result<Status, StatusError> {
    use Status::*;
    let allowed = match from {
        Todo => matches!(to, InProgress | Blocked | Done | Cancelled),
        InProgress => matches!(to, Todo | Blocked | InReview | Done | Cancelled),
        Blocked => matches!(to, InProgress | Todo | Cancelled),
        InReview => matches!(to, InProgress | Done | Cancelled),
        // Reopen is allowed; it is recorded as an event like any other change.
        Done => matches!(to, InProgress),
        Cancelled => matches!(to, Todo),
    };
    if !allowed {
        return Err(StatusError::NotAllowed { from, to });
    }
    if to == Done && !shortfalls.is_empty() && !supervisor_override {
        return Err(StatusError::DeliverablesMissing(shortfalls));
    }
    Ok(to)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::deliverable::DeliverableKind;

    #[test]
    fn normal_flow() {
        assert_eq!(
            transition(Status::Todo, Status::InProgress, vec![], false),
            Ok(Status::InProgress)
        );
        assert_eq!(
            transition(Status::InProgress, Status::Done, vec![], false),
            Ok(Status::Done)
        );
        assert!(transition(Status::Blocked, Status::Done, vec![], false).is_err());
    }

    #[test]
    fn done_requires_deliverables_unless_overridden() {
        let missing = vec![Shortfall {
            kind: DeliverableKind::Text,
            needed: 1,
            have: 0,
        }];
        assert!(matches!(
            transition(Status::InProgress, Status::Done, missing.clone(), false),
            Err(StatusError::DeliverablesMissing(_))
        ));
        assert_eq!(
            transition(Status::InProgress, Status::Done, missing, true),
            Ok(Status::Done)
        );
    }
}
