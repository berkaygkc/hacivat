//! The role probe: its SQL and how its answers are read (MCP-09 step 1, Q21).

use hacivat_guard::{RoleFindings, RoleWarning, interpret_probe, role_probe_sql};
use hacivat_proto::DbmsKind;

type RowSet = Vec<Vec<Option<String>>>;

fn b(v: bool) -> Option<String> {
    Some(if v { "t" } else { "f" }.to_owned())
}

fn names(ns: &[&str]) -> RowSet {
    ns.iter().map(|n| vec![Some((*n).to_owned())]).collect()
}

/// The five PostgreSQL row sets: `rolsuper` (the other three role flags false), the
/// predefined roles, the write check, the TEMP check and the egress extensions.
fn pg_rows(superuser: bool, roles: &[&str], write: bool, temp: bool, exts: &[&str]) -> Vec<RowSet> {
    pg_flags([superuser, false, false, false], roles, write, temp, exts)
}

fn pg_flags(
    flags: [bool; 4],
    roles: &[&str],
    write: bool,
    temp: bool,
    exts: &[&str],
) -> Vec<RowSet> {
    vec![
        vec![flags.iter().map(|f| b(*f)).collect()],
        names(roles),
        vec![vec![b(write)]],
        vec![vec![b(temp)]],
        names(exts),
    ]
}

/// `SHOW GRANTS`: one row set, one line per row.
fn grants(lines: &[&str]) -> Vec<RowSet> {
    vec![names(lines)]
}

fn unparsed(f: &RoleFindings) -> bool {
    !f.least_privilege && f.warnings.contains(&RoleWarning::Unparsed)
}

#[test]
fn probe_sql_per_engine() {
    let pg = role_probe_sql(DbmsKind::Postgres);
    assert_eq!(
        pg,
        [
            "SELECT rolsuper, rolbypassrls, rolreplication, rolcreaterole FROM pg_roles WHERE rolname = current_user",
            "SELECT r.rolname FROM pg_roles r WHERE pg_has_role(current_user, r.oid, 'MEMBER') AND r.rolname IN ('pg_read_server_files','pg_write_server_files','pg_execute_server_program','pg_signal_backend','pg_write_all_data')",
            "SELECT EXISTS (SELECT 1 FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace WHERE c.relkind IN ('r','p','v','m','f') AND n.nspname NOT IN ('pg_catalog','information_schema') AND (pg_has_role(current_user, c.relowner, 'USAGE') OR has_table_privilege(c.oid, 'INSERT, UPDATE, DELETE, TRUNCATE')))",
            "SELECT has_database_privilege(current_database(), 'TEMP')",
            "SELECT extname FROM pg_extension WHERE extname IN ('dblink','postgres_fdw','pg_net','http')",
        ]
    );
    for k in [DbmsKind::MySql, DbmsKind::MariaDb] {
        assert_eq!(role_probe_sql(k), ["SHOW GRANTS"], "{k:?}");
    }
    for k in [DbmsKind::Sqlite, DbmsKind::MsSql, DbmsKind::Redis] {
        assert!(role_probe_sql(k).is_empty(), "{k:?}");
    }
}

#[test]
fn pg_probe_least_privilege_and_warnings() {
    assert!(
        interpret_probe(DbmsKind::Postgres, &pg_rows(false, &[], false, false, &[]))
            .least_privilege
    );
    let f = interpret_probe(
        DbmsKind::Postgres,
        &pg_rows(true, &["pg_signal_backend"], true, true, &["dblink"]),
    );
    assert!(!f.least_privilege);
    for w in [
        RoleWarning::Superuser,
        RoleWarning::SignalBackend,
        RoleWarning::WritePrivileges,
        RoleWarning::TempPrivilege,
        RoleWarning::EgressExtension("dblink".into()),
    ] {
        assert!(f.warnings.contains(&w), "{w:?}");
    }
    let t = interpret_probe(DbmsKind::Postgres, &pg_rows(false, &[], false, true, &[])); // TEMP alone (Q21)
    assert!(t.least_privilege);
    assert_eq!(t.warnings, [RoleWarning::TempPrivilege]);
}

#[test]
fn pg_clean_role_has_no_warnings() {
    let f = interpret_probe(DbmsKind::Postgres, &pg_rows(false, &[], false, false, &[]));
    assert_eq!(
        f,
        RoleFindings {
            least_privilege: true,
            warnings: vec![]
        }
    );
}

#[test]
fn pg_each_role_flag_has_its_warning() {
    for (i, w) in [
        RoleWarning::Superuser,
        RoleWarning::BypassRls,
        RoleWarning::Replication,
        RoleWarning::CreateRole,
    ]
    .into_iter()
    .enumerate()
    {
        let mut flags = [false; 4];
        flags[i] = true;
        let f = interpret_probe(DbmsKind::Postgres, &pg_flags(flags, &[], false, false, &[]));
        assert_eq!(
            f,
            RoleFindings {
                least_privilege: false,
                warnings: vec![w]
            }
        );
    }
}

#[test]
fn pg_each_predefined_role_has_its_warning() {
    for (role, w) in [
        ("pg_read_server_files", RoleWarning::ServerFileRoles),
        ("pg_write_server_files", RoleWarning::ServerFileRoles),
        (
            "pg_execute_server_program",
            RoleWarning::ExecuteServerProgram,
        ),
        ("pg_signal_backend", RoleWarning::SignalBackend),
        ("pg_write_all_data", RoleWarning::WritePrivileges),
    ] {
        let f = interpret_probe(
            DbmsKind::Postgres,
            &pg_rows(false, &[role], false, false, &[]),
        );
        assert_eq!(
            f,
            RoleFindings {
                least_privilege: false,
                warnings: vec![w]
            },
            "{role}"
        );
    }
    // Both file roles and the write check report each warning once.
    let f = interpret_probe(
        DbmsKind::Postgres,
        &pg_rows(
            false,
            &[
                "pg_read_server_files",
                "pg_write_server_files",
                "pg_write_all_data",
            ],
            true,
            false,
            &[],
        ),
    );
    assert_eq!(
        f.warnings,
        [RoleWarning::ServerFileRoles, RoleWarning::WritePrivileges]
    );
}

#[test]
fn pg_egress_extensions_use_the_fixed_names() {
    let f = interpret_probe(
        DbmsKind::Postgres,
        &pg_rows(
            false,
            &[],
            false,
            false,
            &["DBLINK", "postgres_fdw", "pg_net", "http"],
        ),
    );
    assert!(!f.least_privilege);
    assert_eq!(
        f.warnings,
        ["dblink", "postgres_fdw", "pg_net", "http"]
            .map(|n| RoleWarning::EgressExtension(n.into()))
    );
}

#[test]
fn pg_unexpected_values_fail_closed() {
    let ok = || pg_rows(false, &[], false, false, &[]);
    let mut cases: Vec<Vec<RowSet>> = Vec::new();
    // Wrong number of row sets.
    cases.push(vec![]);
    cases.push(ok()[..4].to_vec());
    let mut extra = ok();
    extra.push(vec![]);
    cases.push(extra);
    // Row 1: missing, doubled, short, NULL, non-boolean.
    for r1 in [
        vec![],
        vec![vec![b(false); 4], vec![b(false); 4]],
        vec![vec![b(false); 3]],
        vec![vec![b(false), None, b(false), b(false)]],
        vec![vec![b(false), Some("no".into()), b(false), b(false)]],
    ] {
        let mut c = ok();
        c[0] = r1;
        cases.push(c);
    }
    // Rows 3 and 4: missing, NULL, non-boolean, two columns.
    for i in [2, 3] {
        for r in [
            vec![],
            vec![vec![None]],
            vec![vec![Some("yes".into())]],
            vec![vec![b(false), b(false)]],
        ] {
            let mut c = ok();
            c[i] = r;
            cases.push(c);
        }
    }
    // Rows 2 and 5: an unknown name, a NULL, two columns.
    for i in [1, 4] {
        for r in [
            names(&["pg_monitor"]),
            vec![vec![None]],
            vec![vec![Some("dblink".into()), None]],
        ] {
            let mut c = ok();
            c[i] = r;
            cases.push(c);
        }
    }
    for c in cases {
        let f = interpret_probe(DbmsKind::Postgres, &c);
        assert!(unparsed(&f), "{c:?} -> {f:?}");
    }
}

#[test]
fn pg_booleans_accept_text_spellings() {
    let mut c = pg_rows(false, &[], false, false, &[]);
    c[0] = vec![
        ["false", "FALSE", "f", "F"]
            .map(|s| Some(s.to_owned()))
            .to_vec(),
    ];
    c[3] = vec![vec![Some("true".into())]];
    let f = interpret_probe(DbmsKind::Postgres, &c);
    assert_eq!(
        f,
        RoleFindings {
            least_privilege: true,
            warnings: vec![RoleWarning::TempPrivilege]
        }
    );
}

#[test]
fn mysql_grants() {
    assert!(
        interpret_probe(
            DbmsKind::MySql,
            &grants(&[
                "GRANT USAGE ON *.* TO `hv_ro`@`%`",
                "GRANT SELECT, SHOW VIEW ON `hv_test`.* TO `hv_ro`@`%`"
            ])
        )
        .least_privilege
    );
    assert!(
        !interpret_probe(
            DbmsKind::MySql,
            &grants(&["GRANT SELECT, INSERT ON `d`.* TO `u`@`%`"])
        )
        .least_privilege
    );
    assert!(
        !interpret_probe(DbmsKind::MySql, &grants(&["GRANT FILE ON *.* TO `u`@`%`"]))
            .least_privilege
    );
    assert!(
        interpret_probe(DbmsKind::MySql, &grants(&["GRANT `r`@`%` TO `u`@`%`"]))
            .warnings
            .contains(&RoleWarning::RoleGrantNotExpanded)
    );
}

#[test]
fn mysql_least_privilege_shapes() {
    for lines in [
        &["GRANT USAGE ON *.* TO `hv_ro`@`%`"][..],
        &["grant  select ,show   view on `d`.* to 'u'@'%'"],
        &["GRANT SELECT (`a`, `b`) ON `d`.`t` TO `u`@`%`"],
        &["GRANT\tSELECT\nON `d`.* TO `u`@`localhost`"],
        &["GRANT USAGE ON *.* TO 'hv_ro'@'%' IDENTIFIED BY PASSWORD '*00'"],
        &[
            "GRANT USAGE ON *.* TO `u`@`%`",
            "REVOKE SELECT ON `mysql`.* FROM `u`@`%`",
        ],
        &["GRANT SELECT ON `to`.`on` TO `u`@`%`"],
    ] {
        let f = interpret_probe(DbmsKind::MySql, &grants(lines));
        assert_eq!(
            f,
            RoleFindings {
                least_privilege: true,
                warnings: vec![]
            },
            "{lines:?}"
        );
    }
}

#[test]
fn mysql_privileges_map_to_warnings() {
    for (line, w) in [
        (
            "GRANT SELECT, INSERT ON `d`.* TO `u`@`%`",
            RoleWarning::WritePrivileges,
        ),
        ("GRANT FILE ON *.* TO `u`@`%`", RoleWarning::ServerFileRoles),
        (
            "GRANT ALL PRIVILEGES ON *.* TO `u`@`%`",
            RoleWarning::Superuser,
        ),
        (
            "GRANT ALL ON `d`.* TO `u`@`%`",
            RoleWarning::WritePrivileges,
        ),
        ("GRANT SUPER ON *.* TO `u`@`%`", RoleWarning::Superuser),
        (
            "GRANT REPLICATION SLAVE ON *.* TO `u`@`%`",
            RoleWarning::Replication,
        ),
        (
            "GRANT CREATE USER ON *.* TO `u`@`%`",
            RoleWarning::CreateRole,
        ),
        (
            "GRANT CONNECTION_ADMIN ON *.* TO `u`@`%`",
            RoleWarning::SignalBackend,
        ),
        (
            "GRANT CREATE TEMPORARY TABLES ON `d`.* TO `u`@`%`",
            RoleWarning::TempPrivilege,
        ),
        (
            "GRANT EXECUTE, LOCK TABLES ON `d`.* TO `u`@`%`",
            RoleWarning::WritePrivileges,
        ),
        // A privilege without a warning of its own is one Hacivat can't read.
        (
            "GRANT SHOW DATABASES ON *.* TO `u`@`%`",
            RoleWarning::Unparsed,
        ),
        (
            "GRANT BACKUP_ADMIN ON *.* TO `u`@`%`",
            RoleWarning::Unparsed,
        ),
        ("GRANT PROXY ON ``@`` TO `u`@`%`", RoleWarning::CreateRole),
        (
            "GRANT SELECT ON `d`.* TO `u`@`%` WITH GRANT OPTION",
            RoleWarning::CreateRole,
        ),
        (
            "GRANT USAGE ON *.* TO `u`@`%` with  grant\toption",
            RoleWarning::CreateRole,
        ),
    ] {
        let f = interpret_probe(DbmsKind::MySql, &grants(&[line]));
        assert!(!f.least_privilege, "{line}");
        assert!(f.warnings.contains(&w), "{line}: {f:?}");
    }
}

#[test]
fn mysql_role_grants_are_not_expanded() {
    for line in [
        "GRANT `r`@`%` TO `u`@`%`",
        "GRANT `r1`@`%`,`r2`@`%` TO `u`@`%`",
        "GRANT 'r'@'%' TO 'u'@'%' WITH ADMIN OPTION",
    ] {
        let f = interpret_probe(
            DbmsKind::MySql,
            &grants(&["GRANT USAGE ON *.* TO `u`@`%`", line]),
        );
        assert!(!f.least_privilege, "{line}");
        assert!(
            f.warnings.contains(&RoleWarning::RoleGrantNotExpanded),
            "{line}"
        );
    }
}

#[test]
fn mysql_odd_lines_fail_closed() {
    for lines in [
        &[][..],
        &["GRANT `SELECT` ON `d`.* TO `u`@`%`"],
        &["GRANT \"SELECT\" ON `d`.* TO `u`@`%`"],
        &["GRANT SELECT ON `d`.* TO `u`@`%"],
        &["GRANT SELECT ON `d`.* TO 'u'@'%"],
        &["GRANT SELECT (`a` ON `d`.* TO `u`@`%`"],
        &["GRANT SELECT) ON `d`.* TO `u`@`%`"],
        &["GRANT ON `d`.* TO `u`@`%`"],
        &["GRANT SELECT,, SHOW VIEW ON `d`.* TO `u`@`%`"],
        &["GRANT SELECT ON `d`.*"],
        &["GRANT SELECT"],
        &["GRANT"],
        &[""],
        &["Grants for u@%"],
        &["SELECT 1"],
        &["GRANT SELECT ON `d`.* TO `u`@`%`; GRANT ALL ON *.* TO `u`@`%`"],
    ] {
        let f = interpret_probe(DbmsKind::MySql, &grants(lines));
        assert!(!f.least_privilege, "{lines:?}: {f:?}");
        assert!(!f.warnings.is_empty(), "{lines:?}");
    }
    // NULLs, extra columns and extra row sets.
    for rows in [
        vec![vec![vec![None]]],
        vec![vec![vec![
            Some("GRANT USAGE ON *.* TO `u`@`%`".into()),
            None,
        ]]],
        vec![grants(&["GRANT USAGE ON *.* TO `u`@`%`"]).remove(0), vec![]],
        vec![],
    ] {
        assert!(
            unparsed(&interpret_probe(DbmsKind::MySql, &rows)),
            "{rows:?}"
        );
    }
}

#[test]
fn unparsable_probe_is_guarded() {
    assert!(
        !interpret_probe(DbmsKind::Postgres, &[vec![vec![Some("weird".into())]]]).least_privilege
    );
}

#[test]
fn engines_without_a_probe_are_unparsed() {
    for k in [DbmsKind::Sqlite, DbmsKind::MsSql, DbmsKind::Redshift] {
        assert!(unparsed(&interpret_probe(k, &[])), "{k:?}");
    }
}
