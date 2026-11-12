//! The settings schema and store, the §5.6 local paths and the workspace-state file (R2).

pub mod fsio;
pub mod paths;
pub mod schema;
pub mod store;
pub mod workspace_state;

pub use paths::LocalPaths;
pub use schema::*;
pub use store::{LoadOutcome, SettingsDiag, load, save_changes};
