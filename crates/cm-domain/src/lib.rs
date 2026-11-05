//! Pure business rules for the company management framework.
//!
//! Nothing in this crate does I/O. The store and server crates load data,
//! call these functions, and persist the results, so every rule here is
//! unit-testable without a database.

pub mod approval;
pub mod deliverable;
pub mod fx;
pub mod gitref;
pub mod handoff;
pub mod money;
pub mod recurrence;
pub mod threshold;
pub mod work_item;

pub use money::{Currency, Money};
