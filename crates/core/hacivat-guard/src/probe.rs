//! The role probe: what the signed-in role could do if the guard failed (MCP-09 step 1). Task 5
//! fills the bodies.

use hacivat_proto::DbmsKind;

/// A privilege of the signed-in role worth a warning.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum RoleWarning {
    Superuser,
    BypassRls,
    Replication,
    CreateRole,
    ServerFileRoles,
    ExecuteServerProgram,
    SignalBackend,
    WritePrivileges,
    TempPrivilege,
    /// An extension that can reach other servers (`dblink`, `postgres_fdw`, ...).
    EgressExtension(String),
    /// A MySQL role grant the probe does not expand (fails closed).
    RoleGrantNotExpanded,
    /// A probe answer the interpreter could not read (fails closed).
    Unparsed,
}

/// What the probe found.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct RoleFindings {
    pub least_privilege: bool,
    pub warnings: Vec<RoleWarning>,
}

/// The probe statements of `kind`, in order. Task 1 stub: none, so no probe runs and no
/// connection is labelled least-privilege until Task 5.
pub fn role_probe_sql(_kind: DbmsKind) -> &'static [&'static str] {
    &[]
}

/// Reads the probe's answers, one row set per probe statement. Task 1 stub: fails closed (not
/// least privilege, `Unparsed`) until Task 5.
pub fn interpret_probe(_kind: DbmsKind, _results: &[Vec<Vec<Option<String>>>]) -> RoleFindings {
    RoleFindings {
        least_privilege: false,
        warnings: vec![RoleWarning::Unparsed],
    }
}
