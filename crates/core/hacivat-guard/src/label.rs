//! The honest access label and the remediation offered for an over-privileged role (D-035,
//! MCP-09 step 1).

use hacivat_proto::{AccessLabel, DbmsKind};

use crate::probe::{RoleFindings, RoleWarning};

/// The label a connection shows. Not read-only is `ReadWrite`. Read-only is
/// `ReadOnlyEnforced` only on PostgreSQL or MySQL with a least-privilege probe, and
/// `ReadOnlyGuarded` otherwise: MariaDB (C19), SQLite, any other engine, and a probe that did
/// not run, failed or found more.
///
/// The findings must agree with themselves: a finding that says least privilege but carries a
/// warning that denies it (any warning but PostgreSQL's `TempPrivilege`, Q21) is guarded.
pub fn access_label(
    kind: DbmsKind,
    read_only: bool,
    findings: Option<&RoleFindings>,
) -> AccessLabel {
    if !read_only {
        return AccessLabel::ReadWrite;
    }
    let neutral = |w: &RoleWarning| match kind {
        DbmsKind::Postgres => *w == RoleWarning::TempPrivilege,
        _ => false,
    };
    let enforced = matches!(kind, DbmsKind::Postgres | DbmsKind::MySql)
        && findings.is_some_and(|f| f.least_privilege && f.warnings.iter().all(neutral));
    if enforced {
        AccessLabel::ReadOnlyEnforced
    } else {
        AccessLabel::ReadOnlyGuarded
    }
}

/// PostgreSQL: a login role that inherits `pg_read_all_data`. No `NOINHERIT`: a non-inheriting
/// member would not get `pg_read_all_data` and could read nothing. TEMP is an optional comment,
/// because revoking it from `PUBLIC` affects every role, and TEMP alone keeps the label
/// enforced (Q21).
const PG_REMEDIATION: &str = "CREATE ROLE <role> LOGIN PASSWORD '<password>';\n\
GRANT pg_read_all_data TO <role>;\n\
-- optional: REVOKE TEMP ON DATABASE <database> FROM PUBLIC;";

/// MySQL: a user with `SELECT` and `SHOW VIEW` on one database, which the probe reads as least
/// privilege.
const MYSQL_REMEDIATION: &str = "CREATE USER '<role>'@'%' IDENTIFIED BY '<password>';\n\
GRANT SELECT, SHOW VIEW ON <database>.* TO '<role>'@'%';";

/// SQL that creates a least-privilege role, with `<role>`, `<password>` and `<database>`
/// placeholders and nothing else to fill in. `None` where no role change makes the label
/// enforced: MariaDB (C19) and SQLite, and every other engine.
pub fn remediation(kind: DbmsKind) -> Option<&'static str> {
    match kind {
        DbmsKind::Postgres => Some(PG_REMEDIATION),
        DbmsKind::MySql => Some(MYSQL_REMEDIATION),
        _ => None,
    }
}
