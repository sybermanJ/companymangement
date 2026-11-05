//! Approval routes for expense requests and contracts.
//!
//! A route is a list of steps. At submission each step is resolved to the
//! people who may act on it (the phase manager, Finance, a named person...).
//! Approvers act one step at a time: approve (the next step takes over),
//! send back for changes, reject, pass the step to someone else, or add a
//! person who must approve after them.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Step {
    pub kind: String,
    #[serde(default)]
    pub label: String,
    pub users: Vec<Uuid>,
    /// Set when the step was passed without a decision (the applicant was its only approver).
    #[serde(default)]
    pub skipped: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Approve,
    /// Back to the applicant for changes; they resubmit and the route starts again.
    Return,
    Reject,
    /// Hand this step to someone else.
    Delegate(Uuid),
    /// Approve, and require this person's approval before the next step.
    Forward(Uuid),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// The step at this index is now waiting.
    Next(usize),
    Approved,
    Returned,
    Rejected,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ApprovalError {
    #[error("this approval is not waiting for you")]
    NotYourTurn,
    #[error("cannot pass the step to yourself")]
    SelfTarget,
    #[error("approval is finished")]
    Finished,
}

/// Removes the applicant from each step (nobody approves their own request).
/// A step left empty falls back to `fallback` minus the applicant; if that is
/// empty too and the applicant was the only possible approver, the step is
/// marked skipped. Returns the index of the first step that needs someone.
pub fn prepare(steps: &mut [Step], applicant: Uuid, fallback: &[Uuid]) -> Option<usize> {
    for s in steps.iter_mut() {
        s.users.sort();
        s.users.dedup();
        let had_applicant = s.users.contains(&applicant);
        s.users.retain(|u| *u != applicant);
        if s.users.is_empty() {
            let mut f: Vec<Uuid> = fallback
                .iter()
                .copied()
                .filter(|u| *u != applicant)
                .collect();
            f.sort();
            f.dedup();
            if f.is_empty() || had_applicant {
                // The applicant approves this level themselves: record it as passed.
                s.skipped = true;
            } else {
                s.users = f;
            }
        }
    }
    first_open(steps, 0)
}

fn first_open(steps: &[Step], from: usize) -> Option<usize> {
    (from..steps.len()).find(|&i| !steps[i].skipped)
}

/// Applies one action by `actor` to the waiting step `current`.
pub fn act(
    steps: &mut Vec<Step>,
    current: usize,
    actor: Uuid,
    action: Action,
) -> Result<Outcome, ApprovalError> {
    let step = steps.get(current).ok_or(ApprovalError::Finished)?;
    if !step.users.contains(&actor) {
        return Err(ApprovalError::NotYourTurn);
    }
    match action {
        Action::Approve => Ok(match first_open(steps, current + 1) {
            Some(i) => Outcome::Next(i),
            None => Outcome::Approved,
        }),
        Action::Return => Ok(Outcome::Returned),
        Action::Reject => Ok(Outcome::Rejected),
        Action::Delegate(to) => {
            if to == actor {
                return Err(ApprovalError::SelfTarget);
            }
            let s = &mut steps[current];
            s.users = vec![to];
            Ok(Outcome::Next(current))
        }
        Action::Forward(to) => {
            if to == actor {
                return Err(ApprovalError::SelfTarget);
            }
            steps.insert(
                current + 1,
                Step {
                    kind: "user".into(),
                    label: String::new(),
                    users: vec![to],
                    skipped: false,
                },
            );
            Ok(Outcome::Next(current + 1))
        }
    }
}

/// Picks the route for an amount: the highest minimum not above the amount.
pub fn pick_route<T>(routes: &[(i64, T)], amount_minor: i64) -> Option<&T> {
    routes
        .iter()
        .filter(|(min, _)| *min <= amount_minor)
        .max_by_key(|(min, _)| *min)
        .map(|(_, r)| r)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn u(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }
    fn step(kind: &str, users: &[u128]) -> Step {
        Step {
            kind: kind.into(),
            label: String::new(),
            users: users.iter().map(|x| u(*x)).collect(),
            skipped: false,
        }
    }

    #[test]
    fn pm_then_finance() {
        let mut s = vec![step("project_manager", &[2]), step("finance", &[3, 4])];
        assert_eq!(prepare(&mut s, u(1), &[u(9)]), Some(0));
        assert_eq!(
            act(&mut s, 0, u(3), Action::Approve),
            Err(ApprovalError::NotYourTurn)
        );
        assert_eq!(act(&mut s, 0, u(2), Action::Approve), Ok(Outcome::Next(1)));
        assert_eq!(act(&mut s, 1, u(4), Action::Approve), Ok(Outcome::Approved));
    }

    #[test]
    fn applicant_never_approves_own_request() {
        // The PM files their own claim: the PM step is passed, Finance decides.
        let mut s = vec![step("project_manager", &[1]), step("finance", &[1, 3])];
        assert_eq!(prepare(&mut s, u(1), &[u(9)]), Some(1));
        assert!(s[0].skipped);
        assert_eq!(s[1].users, vec![u(3)]);
    }

    #[test]
    fn empty_step_falls_back_to_admins() {
        let mut s = vec![step("department_head", &[]), step("finance", &[3])];
        assert_eq!(prepare(&mut s, u(1), &[u(9), u(1)]), Some(0));
        assert_eq!(s[0].users, vec![u(9)]);
    }

    #[test]
    fn forward_adds_an_approver_after_me() {
        let mut s = vec![step("project_manager", &[2]), step("finance", &[3])];
        prepare(&mut s, u(1), &[]);
        assert_eq!(
            act(&mut s, 0, u(2), Action::Forward(u(5))),
            Ok(Outcome::Next(1))
        );
        assert_eq!(s.len(), 3);
        assert_eq!(act(&mut s, 1, u(5), Action::Approve), Ok(Outcome::Next(2)));
        assert_eq!(act(&mut s, 2, u(3), Action::Approve), Ok(Outcome::Approved));
    }

    #[test]
    fn delegate_hands_the_step_over() {
        let mut s = vec![step("project_manager", &[2])];
        assert_eq!(
            act(&mut s, 0, u(2), Action::Delegate(u(6))),
            Ok(Outcome::Next(0))
        );
        assert_eq!(
            act(&mut s, 0, u(2), Action::Approve),
            Err(ApprovalError::NotYourTurn)
        );
        assert_eq!(act(&mut s, 0, u(6), Action::Approve), Ok(Outcome::Approved));
        assert_eq!(
            act(&mut s, 0, u(6), Action::Delegate(u(6))),
            Err(ApprovalError::SelfTarget)
        );
    }

    #[test]
    fn return_and_reject() {
        let mut s = vec![step("finance", &[3])];
        assert_eq!(act(&mut s, 0, u(3), Action::Return), Ok(Outcome::Returned));
        assert_eq!(act(&mut s, 0, u(3), Action::Reject), Ok(Outcome::Rejected));
        assert_eq!(
            act(&mut s, 1, u(3), Action::Approve),
            Err(ApprovalError::Finished)
        );
    }

    #[test]
    fn route_by_amount() {
        let routes = [(0, "basic"), (5_000_000, "big"), (50_000_000, "huge")];
        assert_eq!(pick_route(&routes, 100), Some(&"basic"));
        assert_eq!(pick_route(&routes, 5_000_000), Some(&"big"));
        assert_eq!(pick_route(&routes, 99_000_000), Some(&"huge"));
        assert_eq!(pick_route::<&str>(&[], 1), None);
    }
}
