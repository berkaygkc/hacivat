//! Sessions (HV-CONN-01, -07, CONN-04 subset, D-035): the core runtime, connection profiles
//! and their store, the connect pipeline, test connection, connection budgets and the
//! `ConnectionManager`. Types are complete since Task 1; bodies arrive in Tasks 12 and 16
//! unless an item says it is working.

// `ConnectError` is the pipeline's error interface (Task 1, HV-CONN-03): it carries the trail,
// the arguments and the server message by value and travels once per connect, so its size is
// by design.
#![allow(
    clippy::result_large_err,
    reason = "ConnectError is a fixed interface; one per connect"
)]

pub mod budget;
pub mod manager;
pub mod pipeline;
pub mod profile;
pub mod registry;
pub mod runtime;
pub mod test_conn;

pub use budget::{Budget, BudgetPermit, Priority, budget_for};
pub use manager::{ConnectionManager, LiveInfo, LiveState, ManagerConfig, PathKind, TestHandle};
pub use registry::default_drivers;
pub use runtime::CoreRuntime;
pub use test_conn::{TestReport, TestWarning, test_connection, test_connection_continuing};
