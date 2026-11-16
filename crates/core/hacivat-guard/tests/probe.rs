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

/// The six PostgreSQL row sets: `rolsuper` (the other three role flags false), the
/// predefined roles, the write check, the TEMP check, the egress extensions and (F1) no
/// function a rollback cannot undo.
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
        names(&[]),
    ]
}

/// A clean PostgreSQL answer whose row 6 names `functions` (F1).
fn pg_functions(functions: &[&str]) -> Vec<RowSet> {
    let mut c = pg_rows(false, &[], false, false, &[]);
    c[5] = names(functions);
    c
}

/// Row 6's `IN` list (F1): the functions whose `EXECUTE` privilege is what lets a role call them.
const PG_EXECUTE_GATED: [&str; 18] = [
    "pg_logical_emit_message",
    "pg_switch_wal",
    "pg_create_restore_point",
    "pg_log_standby_snapshot",
    "pg_backup_start",
    "pg_backup_stop",
    "pg_start_backup",
    "pg_stop_backup",
    "pg_promote",
    "pg_wal_replay_pause",
    "pg_wal_replay_resume",
    "pg_replication_origin_create",
    "pg_replication_origin_drop",
    "pg_replication_origin_advance",
    "pg_replication_origin_session_setup",
    "pg_replication_origin_session_reset",
    "pg_replication_origin_xact_setup",
    "pg_replication_origin_xact_reset",
];

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
            // Fix round 2 (I-1): rows 1 and 3 cover every role current_user is a MEMBER of.
            // T18 review m-3: every function, aggregate and relation is `pg_catalog.`-qualified,
            // so a `search_path` naming another schema first cannot shadow them.
            "SELECT pg_catalog.bool_or(rolsuper), pg_catalog.bool_or(rolbypassrls), pg_catalog.bool_or(rolreplication), pg_catalog.bool_or(rolcreaterole) FROM pg_catalog.pg_roles WHERE pg_catalog.pg_has_role(current_user, oid, 'MEMBER')",
            "SELECT r.rolname FROM pg_catalog.pg_roles r WHERE pg_catalog.pg_has_role(current_user, r.oid, 'MEMBER') AND r.rolname IN ('pg_read_server_files','pg_write_server_files','pg_execute_server_program','pg_signal_backend','pg_write_all_data','pg_maintain')",
            // Round 3 (m-4): CREATEDB and database CREATE folded into row 3.
            "SELECT EXISTS (SELECT 1 FROM pg_catalog.pg_class c JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace JOIN pg_catalog.pg_roles r ON pg_catalog.pg_has_role(current_user, r.oid, 'MEMBER') WHERE c.relkind IN ('r','p','v','m','f','S') AND n.nspname !~ '^pg_' AND n.nspname <> 'information_schema' AND (c.relowner = r.oid OR CASE WHEN c.relkind = 'S' THEN pg_catalog.has_sequence_privilege(r.oid, c.oid, 'USAGE, UPDATE') ELSE pg_catalog.has_table_privilege(r.oid, c.oid, 'INSERT, UPDATE, DELETE, TRUNCATE') OR pg_catalog.has_any_column_privilege(r.oid, c.oid, 'INSERT, UPDATE') END)) OR EXISTS (SELECT 1 FROM pg_catalog.pg_namespace n JOIN pg_catalog.pg_roles r ON pg_catalog.pg_has_role(current_user, r.oid, 'MEMBER') WHERE n.nspname !~ '^pg_' AND n.nspname <> 'information_schema' AND pg_catalog.has_schema_privilege(r.oid, n.oid, 'CREATE')) OR EXISTS (SELECT 1 FROM pg_catalog.pg_roles r WHERE pg_catalog.pg_has_role(current_user, r.oid, 'MEMBER') AND (r.rolcreatedb OR pg_catalog.has_database_privilege(r.oid, pg_catalog.current_database(), 'CREATE')))",
            "SELECT pg_catalog.has_database_privilege(pg_catalog.current_database(), 'TEMP')",
            "SELECT extname FROM pg_catalog.pg_extension WHERE extname IN ('dblink','postgres_fdw','pg_net','http')",
            // F1: the functions a rollback cannot undo that any member role may execute. The
            // replication-origin ones check for a superuser themselves before PostgreSQL 14.
            "SELECT DISTINCT p.proname FROM pg_catalog.pg_proc p JOIN pg_catalog.pg_namespace n ON n.oid = p.pronamespace JOIN pg_catalog.pg_roles r ON pg_catalog.pg_has_role(current_user, r.oid, 'MEMBER') WHERE n.nspname = 'pg_catalog' AND p.proname IN ('pg_logical_emit_message','pg_switch_wal','pg_create_restore_point','pg_log_standby_snapshot','pg_backup_start','pg_backup_stop','pg_start_backup','pg_stop_backup','pg_promote','pg_wal_replay_pause','pg_wal_replay_resume','pg_replication_origin_create','pg_replication_origin_drop','pg_replication_origin_advance','pg_replication_origin_session_setup','pg_replication_origin_session_reset','pg_replication_origin_xact_setup','pg_replication_origin_xact_reset') AND (p.proname !~ '^pg_replication_origin_' OR pg_catalog.current_setting('server_version_num')::pg_catalog.int4 >= 140000) AND pg_catalog.has_function_privilege(r.oid, p.oid, 'EXECUTE')",
        ]
    );
    for k in [DbmsKind::MySql, DbmsKind::MariaDb] {
        assert_eq!(role_probe_sql(k), ["SHOW GRANTS"], "{k:?}");
    }
    for k in [DbmsKind::Sqlite, DbmsKind::MsSql, DbmsKind::Redis] {
        assert!(role_probe_sql(k).is_empty(), "{k:?}");
    }
}

/// What PostgreSQL 13, 15 and 18 returned for rows 1 and 3 of the corrected probe, run as
/// `hv_ro` in each scenario (fix round 2, I-1; throwaway containers; the versions agreed, and
/// `pg_read_all_data`, new in 14, ran on 15 and 18 only). Rows 2 and 5 were empty and row 4
/// (TEMP, granted to PUBLIC by default) true throughout. Round 3 (m-4, m-7) added the last four,
/// run again on 13, 15 and 18.
const PG_SCENARIOS: [(&str, [bool; 4], bool); 15] = [
    ("SELECT only", [false; 4], false),
    ("pg_read_all_data (the remediation)", [false; 4], false),
    ("member of a superuser role", [true; 4], true),
    ("column-level UPDATE grant", [false; 4], true),
    ("sequence USAGE", [false; 4], true),
    ("sequence UPDATE", [false; 4], true),
    ("CREATE on schema public", [false; 4], true),
    ("member of a role with INSERT", [false; 4], true),
    ("member of a table's owner", [false; 4], true),
    ("database owner", [false; 4], true),
    ("SELECT only, after revokes", [false; 4], false),
    ("CREATEDB", [false; 4], true),
    ("member of a CREATEDB role", [false; 4], true),
    ("CREATE on the database", [false; 4], true),
    ("the PostgreSQL 13 remediation", [false; 4], false),
];

#[test]
fn pg_scenarios_read_as_their_findings() {
    for (scenario, flags, write) in PG_SCENARIOS {
        let f = interpret_probe(DbmsKind::Postgres, &pg_flags(flags, &[], write, true, &[]));
        let mut want = Vec::new();
        if flags[0] {
            want.extend([
                RoleWarning::Superuser,
                RoleWarning::BypassRls,
                RoleWarning::Replication,
                RoleWarning::CreateRole,
            ]);
        }
        if write {
            want.push(RoleWarning::WritePrivileges);
        }
        want.push(RoleWarning::TempPrivilege);
        let least = !flags[0] && !write;
        assert_eq!(
            f,
            RoleFindings {
                least_privilege: least,
                warnings: want
            },
            "{scenario}"
        );
    }
}

#[test]
fn pg_superuser_membership_alone_is_not_least_privilege() {
    // Row 1 is aggregated over member roles: a superuser membership shows there even when
    // row 3 finds nothing to write (for example, no user tables yet).
    let f = interpret_probe(
        DbmsKind::Postgres,
        &pg_flags([true, false, false, false], &[], false, false, &[]),
    );
    assert_eq!(
        f,
        RoleFindings {
            least_privilege: false,
            warnings: vec![RoleWarning::Superuser]
        }
    );
}

#[test]
fn pg_null_aggregate_row_is_unparsed() {
    // `bool_or` over no rows is NULL. It cannot happen (a role is a member of itself), so it
    // fails closed.
    let mut c = pg_rows(false, &[], false, false, &[]);
    c[0] = vec![vec![None, None, None, None]];
    let f = interpret_probe(DbmsKind::Postgres, &c);
    assert_eq!(
        f,
        RoleFindings {
            least_privilege: false,
            warnings: vec![RoleWarning::Unparsed]
        }
    );
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
        ("pg_maintain", RoleWarning::WritePrivileges),
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
    // Wrong number of row sets (five is the answer of the probe before F1).
    cases.push(vec![]);
    cases.push(ok()[..4].to_vec());
    cases.push(ok()[..5].to_vec());
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
    // Rows 2, 5 and 6: an unknown name, a NULL, two columns.
    for i in [1, 4, 5] {
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
fn pg_non_transactional_functions_are_not_least_privilege() {
    // F1: `PUBLIC` may execute `pg_logical_emit_message` on PostgreSQL 13–18 until the
    // remediation's optional REVOKE, so a default role reads guarded.
    for name in PG_EXECUTE_GATED {
        assert_eq!(
            interpret_probe(DbmsKind::Postgres, &pg_functions(&[name])),
            RoleFindings {
                least_privilege: false,
                warnings: vec![RoleWarning::NonTransactionalWrite]
            },
            "{name}"
        );
    }
    // Several names, in any case, are one warning; with TEMP it is still guarded.
    let mut c = pg_functions(&["PG_LOGICAL_EMIT_MESSAGE", "pg_switch_wal", "pg_promote"]);
    c[3] = vec![vec![b(true)]];
    assert_eq!(
        interpret_probe(DbmsKind::Postgres, &c),
        RoleFindings {
            least_privilege: false,
            warnings: vec![
                RoleWarning::TempPrivilege,
                RoleWarning::NonTransactionalWrite
            ]
        }
    );
    // Row 6 names only functions of the guard's text layer.
    for name in PG_EXECUTE_GATED {
        assert!(
            hacivat_guard::PG_NON_TRANSACTIONAL_FUNCTIONS.contains(&name),
            "{name}"
        );
    }
}

#[test]
fn pg_row_6_lists_exactly_the_execute_gated_functions() {
    let row6 = role_probe_sql(DbmsKind::Postgres)[5];
    let list = format!(
        "p.proname IN ({})",
        PG_EXECUTE_GATED.map(|n| format!("'{n}'")).join(",")
    );
    assert!(row6.contains(&list), "{row6}");
    // Replication slots are executable by PUBLIC and refused at run time without REPLICATION
    // (row 1), so they are not in row 6.
    assert!(!row6.contains("replication_slot"), "{row6}");
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
fn mysql_non_ascii_outside_quotes_is_unparsed() {
    // Fix round 2, M-2: a no-break space or a look-alike letter outside quotes is Unparsed
    // (it used to read as a role grant; closed either way).
    for line in [
        "GRANT SELECT\u{a0}ON `d`.* TO `u`@`%`",
        "GRANT SELECT ON `d`.* TO\u{a0}`u`@`%`",
        "GRANT SELECT, SHOW V\u{130}EW ON `d`.* TO `u`@`%`",
    ] {
        let f = interpret_probe(DbmsKind::MySql, &grants(&[line]));
        assert_eq!(
            f,
            RoleFindings {
                least_privilege: false,
                warnings: vec![RoleWarning::Unparsed]
            },
            "{line:?}"
        );
    }
    // Non-ASCII inside quotes is a name, and fine.
    let f = interpret_probe(
        DbmsKind::MySql,
        &grants(&["GRANT SELECT ON `veri_tabanı`.* TO `kullanıcı`@`%`"]),
    );
    assert!(f.least_privilege, "{f:?}");
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
