//! The honest access label and the remediation SQL (D-035, MCP-09 step 1, C19, Q21).

use hacivat_guard::{
    RoleFindings, RoleWarning, access_label, interpret_probe, remediation, remediation_for,
    role_probe_sql,
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
    // F1: an unknown version gets the newest form, whose REVOKE names the 17+ signatures.
    assert_eq!(remediation(DbmsKind::Postgres), Some(PG17_REMEDIATION));
    // Every revoke from PUBLIC stays a comment: they affect every role (Q21, fix round 2, F1).
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

const PG13_REMEDIATION: &str = "CREATE ROLE <role> LOGIN PASSWORD '<password>';\nGRANT USAGE ON SCHEMA <schema> TO <role>;\nGRANT SELECT ON ALL TABLES IN SCHEMA <schema> TO <role>;\nGRANT SELECT ON ALL SEQUENCES IN SCHEMA <schema> TO <role>;\nALTER DEFAULT PRIVILEGES IN SCHEMA <schema> GRANT SELECT ON TABLES TO <role>;\n-- optional: REVOKE TEMP ON DATABASE <database> FROM PUBLIC;\n-- optional: REVOKE CREATE ON SCHEMA public FROM PUBLIC;\n-- optional: REVOKE EXECUTE ON FUNCTION pg_logical_emit_message(boolean, text, text), pg_logical_emit_message(boolean, text, bytea) FROM PUBLIC;";

/// PostgreSQL 14–16 (F1: the REVOKE names the three-argument signatures of 13–16).
const PG14_REMEDIATION: &str = "CREATE ROLE <role> LOGIN PASSWORD '<password>';\nGRANT pg_read_all_data TO <role>;\n-- optional: REVOKE TEMP ON DATABASE <database> FROM PUBLIC;\n-- optional: REVOKE CREATE ON SCHEMA public FROM PUBLIC;\n-- optional: REVOKE EXECUTE ON FUNCTION pg_logical_emit_message(boolean, text, text), pg_logical_emit_message(boolean, text, bytea) FROM PUBLIC;";

/// PostgreSQL 17 and later, and an unknown version (F1: 17 added the `flush` argument, so the
/// three-argument signatures no longer exist there).
const PG17_REMEDIATION: &str = "CREATE ROLE <role> LOGIN PASSWORD '<password>';\nGRANT pg_read_all_data TO <role>;\n-- optional: REVOKE TEMP ON DATABASE <database> FROM PUBLIC;\n-- optional: REVOKE CREATE ON SCHEMA public FROM PUBLIC;\n-- optional: REVOKE EXECUTE ON FUNCTION pg_logical_emit_message(boolean, text, text, boolean), pg_logical_emit_message(boolean, text, bytea, boolean) FROM PUBLIC;";

#[test]
fn postgres_remediation_follows_the_server_version() {
    // Round 3, m-7: `pg_read_all_data` exists from PostgreSQL 14. F1: the REVOKE of
    // `pg_logical_emit_message` names 13–16's signatures, or 17's (with `flush`).
    for (majors, want) in [
        (
            &[Some(13), Some(12), Some(9), Some(0)][..],
            PG13_REMEDIATION,
        ),
        (&[Some(14), Some(15), Some(16)], PG14_REMEDIATION),
        (
            &[None, Some(17), Some(18), Some(u32::MAX)],
            PG17_REMEDIATION,
        ),
    ] {
        for &major in majors {
            assert_eq!(
                remediation_for(DbmsKind::Postgres, major),
                Some(want),
                "{major:?}"
            );
        }
    }
    assert!(!PG13_REMEDIATION.contains("pg_read_all_data"));
    for s in [PG13_REMEDIATION, PG14_REMEDIATION, PG17_REMEDIATION] {
        assert!(!s.contains("NOINHERIT"));
        for line in s.lines().filter(|l| l.contains("PUBLIC")) {
            assert!(line.starts_with("-- optional: "), "{line}");
        }
        // The F1 line is the last one, after the two earlier optional comments.
        assert!(
            s.lines().last().is_some_and(|l| l
                .starts_with("-- optional: REVOKE EXECUTE ON FUNCTION pg_logical_emit_message(")),
            "{s}"
        );
    }
}

#[test]
fn other_engines_ignore_the_version() {
    for major in [None, Some(5), Some(8), Some(13)] {
        assert_eq!(
            remediation_for(DbmsKind::MySql, major),
            remediation(DbmsKind::MySql)
        );
        for k in [DbmsKind::MariaDb, DbmsKind::Sqlite, DbmsKind::MsSql] {
            assert_eq!(remediation_for(k, major), None, "{k:?} {major:?}");
        }
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

#[test]
fn a_postgres_role_that_may_emit_wal_messages_is_guarded_until_the_revoke() {
    // Task 23 finding F1 (controller ruling): the guard's text layer refuses the call, but SQL
    // built at run time gets past it, so only a revoked `EXECUTE` keeps the label enforced.
    let answer = |functions: &[&str]| {
        let rows: Vec<Vec<Vec<Option<String>>>> = vec![
            vec![vec![Some("f".into()); 4]],
            vec![],
            vec![vec![Some("f".into())]],
            vec![vec![Some("t".into())]],
            vec![],
            functions
                .iter()
                .map(|n| vec![Some((*n).to_owned())])
                .collect(),
        ];
        interpret_probe(DbmsKind::Postgres, &rows)
    };
    let default = answer(&["pg_logical_emit_message"]);
    assert_eq!(
        access_label(DbmsKind::Postgres, true, Some(&default)),
        AccessLabel::ReadOnlyGuarded
    );
    assert_eq!(
        default.warnings,
        [
            RoleWarning::TempPrivilege,
            RoleWarning::NonTransactionalWrite
        ]
    );
    let revoked = answer(&[]);
    assert_eq!(
        access_label(DbmsKind::Postgres, true, Some(&revoked)),
        AccessLabel::ReadOnlyEnforced
    );
}
