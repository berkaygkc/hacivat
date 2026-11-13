//! The honest access label and the remediation SQL (D-035, MCP-09 step 1, C19, Q21).

use hacivat_guard::{
    RoleFindings, RoleWarning, access_label, interpret_probe, remediation, role_probe_sql,
};
use hacivat_proto::{AccessLabel, DbmsKind};

#[test]
fn labels() {
    let lp = RoleFindings {
        least_privilege: true,
        warnings: vec![],
    };
    assert_eq!(
        access_label(DbmsKind::Postgres, true, Some(&lp)),
        AccessLabel::ReadOnlyEnforced
    );
    assert_eq!(
        access_label(DbmsKind::MariaDb, true, Some(&lp)),
        AccessLabel::ReadOnlyGuarded
    );
    assert_eq!(
        access_label(DbmsKind::Sqlite, true, None),
        AccessLabel::ReadOnlyGuarded
    );
    assert_eq!(
        access_label(DbmsKind::MySql, false, Some(&lp)),
        AccessLabel::ReadWrite
    );
}

#[test]
fn enforced_needs_a_least_privilege_probe() {
    let lp = RoleFindings {
        least_privilege: true,
        warnings: vec![],
    };
    let wide = RoleFindings {
        least_privilege: false,
        warnings: vec![RoleWarning::WritePrivileges],
    };
    for k in [DbmsKind::Postgres, DbmsKind::MySql] {
        assert_eq!(
            access_label(k, true, Some(&lp)),
            AccessLabel::ReadOnlyEnforced,
            "{k:?}"
        );
        assert_eq!(
            access_label(k, true, Some(&wide)),
            AccessLabel::ReadOnlyGuarded,
            "{k:?}"
        );
        assert_eq!(
            access_label(k, true, None),
            AccessLabel::ReadOnlyGuarded,
            "{k:?}"
        );
        assert_eq!(
            access_label(k, false, None),
            AccessLabel::ReadWrite,
            "{k:?}"
        );
    }
    // Engines outside PostgreSQL and MySQL are never enforced, whatever the findings say.
    for k in [
        DbmsKind::MariaDb,
        DbmsKind::Sqlite,
        DbmsKind::Redshift,
        DbmsKind::Timescale,
        DbmsKind::TiDb,
        DbmsKind::MsSql,
    ] {
        assert_eq!(
            access_label(k, true, Some(&lp)),
            AccessLabel::ReadOnlyGuarded,
            "{k:?}"
        );
        assert_eq!(
            access_label(k, false, Some(&lp)),
            AccessLabel::ReadWrite,
            "{k:?}"
        );
    }
}

#[test]
fn temp_privilege_is_label_neutral_on_postgres_only() {
    // Q21: TEMP alone keeps PostgreSQL enforced.
    let temp = RoleFindings {
        least_privilege: true,
        warnings: vec![RoleWarning::TempPrivilege],
    };
    assert_eq!(
        access_label(DbmsKind::Postgres, true, Some(&temp)),
        AccessLabel::ReadOnlyEnforced
    );
    // Findings that contradict themselves never earn the enforced label.
    let inconsistent = RoleFindings {
        least_privilege: true,
        warnings: vec![RoleWarning::Superuser],
    };
    for k in [DbmsKind::Postgres, DbmsKind::MySql] {
        assert_eq!(
            access_label(k, true, Some(&inconsistent)),
            AccessLabel::ReadOnlyGuarded,
            "{k:?}"
        );
    }
    assert_eq!(
        access_label(DbmsKind::MySql, true, Some(&temp)),
        AccessLabel::ReadOnlyGuarded
    );
}

#[test]
fn remediation_has_only_placeholders() {
    let s = remediation(DbmsKind::Postgres).unwrap();
    assert!(s.contains("<role>") && s.contains("<database>"));
    assert!(!s.contains("NOINHERIT"));
    assert!(remediation(DbmsKind::MariaDb).is_none());
} // Q21

#[test]
fn remediation_is_exact() {
    assert_eq!(
        remediation(DbmsKind::Postgres),
        Some(
            "CREATE ROLE <role> LOGIN PASSWORD '<password>';\nGRANT pg_read_all_data TO <role>;\n-- optional: REVOKE TEMP ON DATABASE <database> FROM PUBLIC;\n-- optional: REVOKE CREATE ON SCHEMA public FROM PUBLIC;"
        )
    );
    // Both revokes from PUBLIC stay comments: they affect every role (Q21, fix round 2).
    let pg = remediation(DbmsKind::Postgres).unwrap();
    for line in pg.lines().filter(|l| l.contains("PUBLIC")) {
        assert!(line.starts_with("-- optional: "), "{line}");
    }
    assert_eq!(
        remediation(DbmsKind::MySql),
        Some(
            "CREATE USER '<role>'@'%' IDENTIFIED BY '<password>';\nGRANT SELECT, SHOW VIEW ON <database>.* TO '<role>'@'%';"
        )
    );
    for k in [DbmsKind::MariaDb, DbmsKind::Sqlite, DbmsKind::MsSql] {
        assert_eq!(remediation(k), None, "{k:?}");
    }
}

#[test]
fn engines_with_remediation_are_the_probed_ones() {
    for k in [DbmsKind::Postgres, DbmsKind::MySql] {
        assert!(
            remediation(k).is_some() && !role_probe_sql(k).is_empty(),
            "{k:?}"
        );
    }
}

#[test]
fn mysql_remediation_role_probes_as_enforced() {
    // The grants MySQL shows for the remediation role.
    let rows = vec![vec![
        vec![Some("GRANT USAGE ON *.* TO `ro`@`%`".to_owned())],
        vec![Some(
            "GRANT SELECT, SHOW VIEW ON `db`.* TO `ro`@`%`".to_owned(),
        )],
    ]];
    let f = interpret_probe(DbmsKind::MySql, &rows);
    assert_eq!(
        access_label(DbmsKind::MySql, true, Some(&f)),
        AccessLabel::ReadOnlyEnforced
    );
}
