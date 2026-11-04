//! Free-flow reassignment ("hand-off") of work items.
//!
//! Anyone can pass an open item to any active person. Hand-offs inside a
//! department go straight to the target for acceptance; hand-offs across
//! departments first need approval from the target's department head (or a
//! deputy), and for project items the project PM may also approve.
//! Logged time never moves with a hand-off.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HandoffState {
    PendingApproval,
    PendingAccept,
    Accepted,
    Declined,
    Rejected,
    Cancelled,
}

impl HandoffState {
    pub fn is_open(self) -> bool {
        matches!(self, Self::PendingApproval | Self::PendingAccept)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HandoffAction {
    Approve,
    Reject,
    Accept,
    Decline,
    Cancel,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum HandoffError {
    #[error("the work item is closed")]
    ItemClosed,
    #[error("the target person is not active in this company")]
    TargetInactive,
    #[error("the item is already held by that person")]
    SameHolder,
    #[error("a reason is required")]
    MissingReason,
    #[error("this person may not approve the hand-off")]
    NotAnApprover,
    #[error("only the target can accept or decline")]
    NotTheTarget,
    #[error("only the sender can cancel")]
    NotTheSender,
    #[error("cannot {action:?} a hand-off that is {state:?}")]
    InvalidTransition {
        state: HandoffState,
        action: HandoffAction,
    },
}

#[derive(Clone, Copy, Debug)]
pub struct Person {
    pub user_id: Uuid,
    pub department_id: Uuid,
    pub active: bool,
}

/// Who is allowed to approve a cross-department hand-off.
#[derive(Clone, Debug, Default)]
pub struct Approvers {
    pub target_department_head: Option<Uuid>,
    pub target_department_deputies: Vec<Uuid>,
    /// Set only when the item belongs to a project.
    pub project_pm: Option<Uuid>,
}

impl Approvers {
    pub fn allows(&self, user: Uuid) -> bool {
        self.target_department_head == Some(user)
            || self.target_department_deputies.contains(&user)
            || self.project_pm == Some(user)
    }
}

pub fn needs_approval(from: &Person, to: &Person) -> bool {
    from.department_id != to.department_id
}

/// Validate a new hand-off request and return its initial state.
pub fn request(
    item_open: bool,
    current_holder: Uuid,
    from: &Person,
    to: &Person,
    reason: &str,
) -> Result<HandoffState, HandoffError> {
    if !item_open {
        return Err(HandoffError::ItemClosed);
    }
    if !to.active {
        return Err(HandoffError::TargetInactive);
    }
    if to.user_id == current_holder {
        return Err(HandoffError::SameHolder);
    }
    if reason.trim().is_empty() {
        return Err(HandoffError::MissingReason);
    }
    Ok(if needs_approval(from, to) {
        HandoffState::PendingApproval
    } else {
        HandoffState::PendingAccept
    })
}

/// Apply an action by `actor` to a hand-off from `sender` to `target`.
pub fn apply(
    state: HandoffState,
    action: HandoffAction,
    actor: Uuid,
    sender: Uuid,
    target: Uuid,
    approvers: &Approvers,
) -> Result<HandoffState, HandoffError> {
    use HandoffAction as A;
    use HandoffState as S;
    let invalid = || HandoffError::InvalidTransition { state, action };
    match (state, action) {
        (S::PendingApproval, A::Approve | A::Reject) => {
            if !approvers.allows(actor) {
                return Err(HandoffError::NotAnApprover);
            }
            Ok(if action == A::Approve {
                S::PendingAccept
            } else {
                S::Rejected
            })
        }
        (S::PendingAccept, A::Accept | A::Decline) => {
            if actor != target {
                return Err(HandoffError::NotTheTarget);
            }
            Ok(if action == A::Accept {
                S::Accepted
            } else {
                S::Declined
            })
        }
        (S::PendingApproval | S::PendingAccept, A::Cancel) => {
            if actor != sender {
                return Err(HandoffError::NotTheSender);
            }
            Ok(S::Cancelled)
        }
        _ => Err(invalid()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids() -> (Uuid, Uuid, Uuid, Uuid, Uuid) {
        (
            Uuid::now_v7(),
            Uuid::now_v7(),
            Uuid::now_v7(),
            Uuid::now_v7(),
            Uuid::now_v7(),
        )
    }

    #[test]
    fn same_department_goes_straight_to_accept() {
        let (a, b, dept, _, _) = ids();
        let from = Person {
            user_id: a,
            department_id: dept,
            active: true,
        };
        let to = Person {
            user_id: b,
            department_id: dept,
            active: true,
        };
        assert_eq!(
            request(true, a, &from, &to, "needs review"),
            Ok(HandoffState::PendingAccept)
        );
    }

    #[test]
    fn cross_department_needs_target_head_approval() {
        let (a, b, d1, d2, head) = ids();
        let from = Person {
            user_id: a,
            department_id: d1,
            active: true,
        };
        let to = Person {
            user_id: b,
            department_id: d2,
            active: true,
        };
        let s = request(true, a, &from, &to, "testing").expect("ok");
        assert_eq!(s, HandoffState::PendingApproval);

        let approvers = Approvers {
            target_department_head: Some(head),
            ..Default::default()
        };
        // the sender cannot approve their own cross-department hand-off
        assert_eq!(
            apply(s, HandoffAction::Approve, a, a, b, &approvers),
            Err(HandoffError::NotAnApprover)
        );
        // the target cannot skip approval by accepting
        assert!(matches!(
            apply(s, HandoffAction::Accept, b, a, b, &approvers),
            Err(HandoffError::InvalidTransition { .. })
        ));
        let s = apply(s, HandoffAction::Approve, head, a, b, &approvers).expect("ok");
        assert_eq!(s, HandoffState::PendingAccept);
        assert_eq!(
            apply(s, HandoffAction::Accept, b, a, b, &approvers),
            Ok(HandoffState::Accepted)
        );
    }

    #[test]
    fn pm_and_deputy_can_approve() {
        let (a, b, pm, deputy, other) = ids();
        let ap = Approvers {
            target_department_head: None,
            target_department_deputies: vec![deputy],
            project_pm: Some(pm),
        };
        let s = HandoffState::PendingApproval;
        assert!(apply(s, HandoffAction::Approve, pm, a, b, &ap).is_ok());
        assert!(apply(s, HandoffAction::Reject, deputy, a, b, &ap).is_ok());
        assert!(apply(s, HandoffAction::Approve, other, a, b, &ap).is_err());
    }

    #[test]
    fn guards() {
        let (a, b, d, _, _) = ids();
        let from = Person {
            user_id: a,
            department_id: d,
            active: true,
        };
        let inactive = Person {
            user_id: b,
            department_id: d,
            active: false,
        };
        let active = Person {
            active: true,
            ..inactive
        };
        assert_eq!(
            request(false, a, &from, &active, "x"),
            Err(HandoffError::ItemClosed)
        );
        assert_eq!(
            request(true, a, &from, &inactive, "x"),
            Err(HandoffError::TargetInactive)
        );
        assert_eq!(
            request(true, b, &from, &active, "x"),
            Err(HandoffError::SameHolder)
        );
        assert_eq!(
            request(true, a, &from, &active, "  "),
            Err(HandoffError::MissingReason)
        );
    }

    #[test]
    fn only_sender_cancels_and_closed_states_are_final() {
        let (a, b, _, _, _) = ids();
        let ap = Approvers::default();
        assert_eq!(
            apply(
                HandoffState::PendingAccept,
                HandoffAction::Cancel,
                b,
                a,
                b,
                &ap
            ),
            Err(HandoffError::NotTheSender)
        );
        assert_eq!(
            apply(
                HandoffState::PendingAccept,
                HandoffAction::Cancel,
                a,
                a,
                b,
                &ap
            ),
            Ok(HandoffState::Cancelled)
        );
        assert!(apply(HandoffState::Accepted, HandoffAction::Decline, b, a, b, &ap).is_err());
        assert!(!HandoffState::Rejected.is_open());
    }
}
