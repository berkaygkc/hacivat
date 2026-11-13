//! The session environment (HV-SESS-06; Task 7): the statements a session runs after connect
//! and replays after a reset or a reconnect (client identity, read-only settings, time zone,
//! the last schema).

use hacivat_proto::DbmsKind;

use crate::profile::Profile;
use crate::session::SchemaCtx;

/// One environment step.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum EnvStep {
    /// The application name the server shows (PostgreSQL `application_name`).
    ClientIdentity(String),
    /// The engine's read-only session settings.
    ReadOnly(Vec<String>),
    TimeZone(String),
    RestoreSchema(String),
    /// A user startup script (M3).
    StartupScript,
}

/// A session's environment, in order.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct EnvPlan {
    pub steps: Vec<EnvStep>,
}

impl EnvPlan {
    /// The plan as statements. Task 1 stub: none (Task 7).
    pub fn statements(&self) -> Vec<String> {
        Vec::new()
    }
}

/// The environment of a `kind` session for `profile`. Task 1 stub: empty (Task 7).
pub fn build_env_plan(
    _kind: DbmsKind,
    _profile: &Profile,
    _read_only: bool,
    _last_schema: Option<&SchemaCtx>,
) -> EnvPlan {
    EnvPlan::default()
}
