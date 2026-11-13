//! The honest access label and the remediation offered for an over-privileged role (D-035,
//! MCP-09 step 1). Task 5 fills the bodies.

use hacivat_proto::{AccessLabel, DbmsKind};

use crate::probe::RoleFindings;

/// The label a connection shows. Task 1 stub: never claims `ReadOnlyEnforced` (not read-only
/// is `ReadWrite`, read-only is `ReadOnlyGuarded`) until Task 5.
pub fn access_label(
    _kind: DbmsKind,
    read_only: bool,
    _findings: Option<&RoleFindings>,
) -> AccessLabel {
    if read_only {
        AccessLabel::ReadOnlyGuarded
    } else {
        AccessLabel::ReadWrite
    }
}

/// SQL that creates a least-privilege role, with `<role>`, `<password>` and `<database>`
/// placeholders. Task 1 stub: none until Task 5.
pub fn remediation(_kind: DbmsKind) -> Option<&'static str> {
    None
}
