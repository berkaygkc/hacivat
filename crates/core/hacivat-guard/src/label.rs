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
/// enforced (Q21). `CREATE` on `public` is an optional comment too, for the same reason: it is
/// needed only where `PUBLIC` still holds it (clusters created before PostgreSQL 15 or
/// upgraded from one), and there the probe reads every role as `WritePrivileges` until it is
/// revoked (fix round 2 ruling).
const PG_REMEDIATION: &str = "CREATE ROLE <role> LOGIN PASSWORD '<password>';\n\
GRANT pg_read_all_data TO <role>;\n\
-- optional: REVOKE TEMP ON DATABASE <database> FROM PUBLIC;\n\
-- optional: REVOKE CREATE ON SCHEMA public FROM PUBLIC;";

/// PostgreSQL before 14, which has no `pg_read_all_data` (round 3, m-7): `SELECT` on one
/// schema's tables and sequences, and on the tables created there later (default privileges of
/// the role that runs it), with the same two optional comments. It adds a `<schema>`
/// placeholder.
const PG13_REMEDIATION: &str = "CREATE ROLE <role> LOGIN PASSWORD '<password>';\n\
GRANT USAGE ON SCHEMA <schema> TO <role>;\n\
GRANT SELECT ON ALL TABLES IN SCHEMA <schema> TO <role>;\n\
GRANT SELECT ON ALL SEQUENCES IN SCHEMA <schema> TO <role>;\n\
ALTER DEFAULT PRIVILEGES IN SCHEMA <schema> GRANT SELECT ON TABLES TO <role>;\n\
-- optional: REVOKE TEMP ON DATABASE <database> FROM PUBLIC;\n\
-- optional: REVOKE CREATE ON SCHEMA public FROM PUBLIC;";

/// The first PostgreSQL major version with `pg_read_all_data`.
const PG_READ_ALL_DATA_SINCE: u32 = 14;

/// MySQL: a user with `SELECT` and `SHOW VIEW` on one database, which the probe reads as least
/// privilege.
const MYSQL_REMEDIATION: &str = "CREATE USER '<role>'@'%' IDENTIFIED BY '<password>';\n\
GRANT SELECT, SHOW VIEW ON <database>.* TO '<role>'@'%';";

/// SQL that creates a least-privilege role, with `<role>`, `<password>` and `<database>`
/// placeholders and nothing else to fill in. `None` where no role change makes the label
/// enforced: MariaDB (C19) and SQLite, and every other engine. PostgreSQL gets the form for
/// version 14 and later; [`remediation_for`] takes the server's major version.
pub fn remediation(kind: DbmsKind) -> Option<&'static str> {
    remediation_for(kind, None)
}

/// [`remediation`] for a server of major version `major`. PostgreSQL before 14 gets the
/// schema-grant form, which also uses a `<schema>` placeholder; an unknown version (`None`)
/// gets the 14+ form. Other engines ignore `major`.
pub fn remediation_for(kind: DbmsKind, major: Option<u32>) -> Option<&'static str> {
    match kind {
        DbmsKind::Postgres => match major {
            Some(m) if m < PG_READ_ALL_DATA_SINCE => Some(PG13_REMEDIATION),
            _ => Some(PG_REMEDIATION),
        },
        DbmsKind::MySql => Some(MYSQL_REMEDIATION),
        _ => None,
    }
}
