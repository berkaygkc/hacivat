//! Option sanitizing tests (Task 2, HV-DRV-05): always-deny before the allow-list, ASCII-only
//! key folding, residual percent escapes, safe PostgreSQL GUCs and the path rule.

use std::path::PathBuf;

use hacivat_driver::{EngineCatalog, SanitizeInput, Violation, ViolationReason, sanitize};
use hacivat_proto::Origin::{self, UserTyped};
use hacivat_proto::{DbmsKind, ImportSource};

fn cat() -> &'static EngineCatalog {
    EngineCatalog::embedded()
}

fn imported() -> Origin {
    Origin::Imported {
        source: ImportSource::Url,
    }
}

fn owned(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

/// A `SanitizeInput` with no unsafe toggle and the approved roots `[~/.postgresql]`. The
/// borrowed parts are leaked: each test builds a handful.
fn input_with(
    kind: DbmsKind,
    origin: Origin,
    options: &[(&str, &str)],
    paths: &[(&str, &str)],
    unsafe_toggle: bool,
) -> SanitizeInput<'static> {
    let paths: Vec<(String, PathBuf)> = paths
        .iter()
        .map(|(k, p)| (k.to_string(), PathBuf::from(p)))
        .collect();
    SanitizeInput {
        kind,
        origin: Box::leak(Box::new(origin)),
        options: Box::leak(owned(options).into_boxed_slice()),
        paths: Box::leak(paths.into_boxed_slice()),
        unsafe_toggle,
        approved_roots: Box::leak(vec![PathBuf::from("~/.postgresql")].into_boxed_slice()),
    }
}

fn input(
    kind: DbmsKind,
    origin: Origin,
    options: &[(&str, &str)],
    paths: &[(&str, &str)],
) -> SanitizeInput<'static> {
    input_with(kind, origin, options, paths, false)
}

fn reasons(r: Result<Vec<(String, String)>, Vec<Violation>>) -> Vec<ViolationReason> {
    r.unwrap_err().into_iter().map(|v| v.reason).collect()
}

fn err_reasons(kind: DbmsKind, origin: Origin, options: &[(&str, &str)]) -> Vec<ViolationReason> {
    reasons(sanitize(&input(kind, origin, options, &[]), cat()))
}

fn err_paths(kind: DbmsKind, origin: Origin, paths: &[(&str, &str)]) -> Vec<ViolationReason> {
    reasons(sanitize(&input(kind, origin, &[], paths), cat()))
}

#[test]
fn mysql_local_infile_denied_even_user_typed() {
    assert_eq!(
        sanitize(
            &input(
                DbmsKind::MySql,
                UserTyped,
                &[("allowLoadLocalInfile", "true")],
                &[]
            ),
            cat()
        )
        .unwrap_err(),
        vec![Violation {
            key: "allowLoadLocalInfile".into(),
            reason: ViolationReason::AlwaysDenied
        }]
    );
}

#[test]
fn always_denied_keys_fold_case_and_yield_to_the_unsafe_toggle() {
    assert_eq!(
        err_reasons(
            DbmsKind::MySql,
            imported(),
            &[("ALLOWLOADLOCALINFILE", "1")]
        ),
        [ViolationReason::AlwaysDenied]
    );
    // MariaDB has the same always-deny list as MySQL.
    assert_eq!(
        err_reasons(DbmsKind::MariaDb, UserTyped, &[("autoDeserialize", "true")]),
        [ViolationReason::AlwaysDenied]
    );
    let toggled = input_with(
        DbmsKind::MySql,
        UserTyped,
        &[("allowLoadLocalInfile", "true")],
        &[],
        true,
    );
    assert_eq!(
        sanitize(&toggled, cat()).unwrap(),
        owned(&[("allowLoadLocalInfile", "true")])
    );
}

#[test]
fn double_encoded_value_rejected() {
    assert_eq!(
        err_reasons(DbmsKind::MySql, imported(), &[("sslMode", "%2552EQUIRED")]),
        [ViolationReason::DoubleEncoding]
    );
    // One decode leaves `%61`, which is still an escape.
    assert_eq!(
        err_reasons(DbmsKind::MySql, imported(), &[("sslMode", "%2561")]),
        [ViolationReason::DoubleEncoding]
    );
}

#[test]
fn values_arrive_decoded_once_so_any_escape_left_is_a_second_layer() {
    // The URL parser decodes once; `my%20app` here means the source encoded it twice.
    assert_eq!(
        err_reasons(
            DbmsKind::Postgres,
            UserTyped,
            &[("application_name", "my%20app")]
        ),
        [ViolationReason::DoubleEncoding]
    );
    assert_eq!(
        sanitize(
            &input(
                DbmsKind::Postgres,
                imported(),
                &[("application_name", "my app"), ("sslmode", "require")],
                &[]
            ),
            cat()
        )
        .unwrap(),
        owned(&[("application_name", "my app"), ("sslmode", "require")])
    );
    // A `%` that is not an escape stays as typed.
    assert_eq!(
        sanitize(
            &input(
                DbmsKind::Postgres,
                imported(),
                &[("application_name", "100%")],
                &[]
            ),
            cat()
        )
        .unwrap(),
        owned(&[("application_name", "100%")])
    );
}

#[test]
fn unknown_option_rejected_for_imports_kept_for_user_typed() {
    assert_eq!(
        err_reasons(DbmsKind::Postgres, imported(), &[("frobnicate", "1")]),
        [ViolationReason::UnknownOption]
    );
    assert_eq!(
        sanitize(
            &input(DbmsKind::Postgres, UserTyped, &[("frobnicate", "1")], &[]),
            cat()
        )
        .unwrap(),
        vec![("frobnicate".into(), "1".into())]
    );
    let toggled = input_with(
        DbmsKind::Postgres,
        imported(),
        &[("frobnicate", "1")],
        &[],
        true,
    );
    assert_eq!(
        sanitize(&toggled, cat()).unwrap(),
        owned(&[("frobnicate", "1")])
    );
}

#[test]
fn option_keys_fold_ascii_only() {
    assert!(
        sanitize(
            &input(
                DbmsKind::Postgres,
                imported(),
                &[("SSLMODE", "require")],
                &[]
            ),
            cat()
        )
        .is_ok()
    );
    assert_eq!(
        err_reasons(DbmsKind::Postgres, imported(), &[("applİcation_name", "x")]),
        [ViolationReason::UnknownOption]
    );
}

#[test]
fn allowed_keys_come_back_in_catalog_spelling() {
    assert_eq!(
        sanitize(
            &input(
                DbmsKind::MySql,
                imported(),
                &[("SSLMODE", "REQUIRED"), ("connecttimeout", "5")],
                &[]
            ),
            cat()
        )
        .unwrap(),
        owned(&[("sslMode", "REQUIRED"), ("connectTimeout", "5")])
    );
}

#[test]
fn every_violation_is_reported_in_order() {
    let r = sanitize(
        &input(
            DbmsKind::MySql,
            imported(),
            &[
                ("frobnicate", "1"),
                ("sslMode", "REQUIRED"),
                ("allowUrlInLocalInfile", "true"),
            ],
            &[],
        ),
        cat(),
    );
    assert_eq!(
        r.unwrap_err(),
        vec![
            Violation {
                key: "frobnicate".into(),
                reason: ViolationReason::UnknownOption
            },
            Violation {
                key: "allowUrlInLocalInfile".into(),
                reason: ViolationReason::AlwaysDenied
            },
        ]
    );
}

#[test]
fn pg_key_path_outside_roots_from_import() {
    assert_eq!(
        err_paths(DbmsKind::Postgres, imported(), &[("sslkey", "/etc/shadow")]),
        [ViolationReason::PathOutsideRoots]
    );
}

#[test]
fn pg_key_paths_inside_roots_pass_and_dot_dot_cannot_escape() {
    assert!(
        sanitize(
            &input(
                DbmsKind::Postgres,
                imported(),
                &[],
                &[("sslkey", "~/.postgresql/postgresql.key")]
            ),
            cat()
        )
        .is_ok()
    );
    assert_eq!(
        err_paths(
            DbmsKind::Postgres,
            imported(),
            &[("sslkey", "~/.postgresql/../.ssh/id_ed25519")]
        ),
        [ViolationReason::PathOutsideRoots]
    );
    // The user typed it: the path rule is for untrusted origins only.
    assert!(
        sanitize(
            &input(
                DbmsKind::Postgres,
                UserTyped,
                &[],
                &[("sslkey", "/etc/shadow")]
            ),
            cat()
        )
        .is_ok()
    );
}

#[cfg(unix)]
#[test]
fn a_symlink_inside_the_roots_pointing_outside_is_rejected() {
    let base = std::env::temp_dir().join(format!("hv-sanitize-{}", std::process::id()));
    let root = base.join("root");
    let outside = base.join("outside");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(outside.join("secret.key"), b"k").unwrap();
    let link = root.join("client.key");
    let _ = std::fs::remove_file(&link);
    std::os::unix::fs::symlink(outside.join("secret.key"), &link).unwrap();
    std::fs::write(root.join("real.key"), b"k").unwrap();

    let roots = [root.clone()];
    let origin = imported();
    let check = |p: PathBuf| {
        let paths = [("sslkey".to_string(), p)];
        sanitize(
            &SanitizeInput {
                kind: DbmsKind::Postgres,
                origin: &origin,
                options: &[],
                paths: &paths,
                unsafe_toggle: false,
                approved_roots: &roots,
            },
            cat(),
        )
    };
    assert_eq!(
        reasons(check(link.clone())),
        [ViolationReason::PathOutsideRoots]
    );
    assert!(check(root.join("real.key")).is_ok());
    std::fs::remove_dir_all(&base).unwrap();
}

#[test]
fn pg_options_allow_only_safe_gucs() {
    assert!(
        sanitize(
            &input(
                DbmsKind::Postgres,
                imported(),
                &[("options", "-c search_path=app")],
                &[]
            ),
            cat()
        )
        .is_ok()
    );
    assert_eq!(
        err_reasons(
            DbmsKind::Postgres,
            imported(),
            &[("options", "-c archive_command=x")]
        ),
        [ViolationReason::AlwaysDenied]
    );
}

#[test]
fn pg_options_forms() {
    let ok = |v: &str| {
        sanitize(
            &input(DbmsKind::Postgres, imported(), &[("options", v)], &[]),
            cat(),
        )
        .is_ok()
    };
    assert!(ok("-c search_path=app -c statement_timeout=5s"));
    assert!(ok("-csearch_path=app"));
    assert!(ok("--datestyle=ISO"));
    assert!(ok("-c TimeZone=UTC"));
    assert!(!ok("-c search_path=app -c session_preload_libraries=x"));
    assert!(!ok("-c"));
    assert!(!ok("search_path=app"));
    assert!(!ok("-c search_path"));
}

#[test]
fn sqlite_vfs_always_denied() {
    assert_eq!(
        err_reasons(DbmsKind::Sqlite, UserTyped, &[("vfs", "unix-none")]),
        [ViolationReason::AlwaysDenied]
    );
    assert_eq!(
        err_reasons(DbmsKind::Sqlite, imported(), &[("load_extension", "1")]),
        [ViolationReason::AlwaysDenied]
    );
}

#[test]
fn startup_scripts_from_untrusted_origins_are_quarantined() {
    assert_eq!(
        err_reasons(
            DbmsKind::Postgres,
            Origin::DeepLink,
            &[("init_sql", "COPY t TO PROGRAM 'x'")]
        ),
        [ViolationReason::QuarantinedScript]
    );
}

fn perde() -> Origin {
    Origin::PerdeManaged { org: "acme".into() }
}

#[test]
fn pg_options_split_like_the_server_vertical_tab_included() {
    // The server's pg_split_opts splits on C isspace, which includes \x0B and \x0C.
    for v in [
        "-c search_path=a\x0B-c\x0Bsession_preload_libraries=x",
        "-c search_path=a\x0C-c\x0Csession_preload_libraries=x",
        "-c search_path=a\r-c\rsession_preload_libraries=x",
    ] {
        assert_eq!(
            err_reasons(DbmsKind::Postgres, perde(), &[("options", v)]),
            [ViolationReason::AlwaysDenied],
            "{v:?}"
        );
    }
    // A backslash escape could glue words the way the server reads them: refused outright.
    assert_eq!(
        err_reasons(
            DbmsKind::Postgres,
            perde(),
            &[(
                "options",
                r"-c search_path=a\ -c\ session_preload_libraries=x"
            )]
        ),
        [ViolationReason::AlwaysDenied]
    );
    // `-` reads as `_` in a setting name, as the server's ParseLongOption does.
    assert!(
        sanitize(
            &input(
                DbmsKind::Postgres,
                perde(),
                &[("options", "--search-path=app")],
                &[]
            ),
            cat()
        )
        .is_ok()
    );
}

#[test]
fn control_characters_are_refused_for_every_origin_even_with_the_toggle() {
    for (k, v) in [
        ("application_name", "a\0b"),
        ("application_name", "a\x1Bb"),
        ("app\nname", "x"),
    ] {
        let toggled = input_with(DbmsKind::Postgres, UserTyped, &[(k, v)], &[], true);
        assert_eq!(
            reasons(sanitize(&toggled, cat())),
            [ViolationReason::AlwaysDenied],
            "{k:?}={v:?}"
        );
    }
}

#[test]
fn an_escape_in_a_key_is_a_second_encoding() {
    assert_eq!(
        err_reasons(DbmsKind::Postgres, UserTyped, &[("ssl%6Dode", "require")]),
        [ViolationReason::DoubleEncoding]
    );
}

#[test]
fn path_options_are_checked_without_being_listed_in_paths() {
    assert_eq!(
        err_reasons(DbmsKind::Postgres, imported(), &[("sslkey", "/etc/passwd")]),
        [ViolationReason::PathOutsideRoots]
    );
    assert_eq!(
        err_reasons(
            DbmsKind::Postgres,
            imported(),
            &[("SSLROOTCERT", "~/.postgresql/../../../etc/ssl/cert.pem")]
        ),
        [ViolationReason::PathOutsideRoots]
    );
    // Inside the approved roots it passes.
    assert!(
        sanitize(
            &input(
                DbmsKind::Postgres,
                imported(),
                &[("sslcert", "~/.postgresql/postgresql.crt")],
                &[]
            ),
            cat()
        )
        .is_ok()
    );
    // `sslrootcert=system` names the system trust store, not a file.
    assert!(
        sanitize(
            &input(
                DbmsKind::Postgres,
                imported(),
                &[("sslrootcert", "system")],
                &[]
            ),
            cat()
        )
        .is_ok()
    );
    // The user typed it: no path rule.
    assert!(
        sanitize(
            &input(
                DbmsKind::Postgres,
                UserTyped,
                &[("sslkey", "/etc/passwd")],
                &[]
            ),
            cat()
        )
        .is_ok()
    );
}

#[test]
fn mysql_async_native_options_are_always_denied() {
    for engine in [DbmsKind::MySql, DbmsKind::MariaDb] {
        for key in [
            "enable_cleartext_plugin",
            "socket",
            "prefer_socket",
            "require_ssl",
            "verify_ca",
            "verify_identity",
            "built_in_roots",
            "secure_auth",
            "reset_connection",
        ] {
            assert_eq!(
                err_reasons(engine, UserTyped, &[(key, "false")]),
                [ViolationReason::AlwaysDenied],
                "{engine:?} {key}"
            );
        }
    }
}

// ## m2b-t18-guarded-exec

/// Task 18 fix rounds 2 and 3 (I-3b, I-3c): a read-only profile's PostgreSQL `options` is
/// allowlisted by shape (`-c name=value`, `-cname=value`, `--name=value`, no `role` or
/// `session_authorization`), whatever its origin; anything else is a visible violation, and a
/// read-write profile keeps the option.
#[test]
fn a_read_only_profile_may_not_switch_the_role() {
    use hacivat_driver::sanitize::{pg_options_read_only_allowed, read_only_violations};
    for value in [
        "-c role=hv_ro",
        "-crole=hv_ro",
        "-c  search_path=app   -c Role=hv_ro",
        "--role=hv_ro",
        "-c session_authorization=hv_ro",
        "--session-authorization=hv_ro",
        "-c ro\\le=x",
        // Review round 2 (I-3c): `getopt` combines single-dash flags, so each of these sets the
        // role (or session authorization) on the server; none has an allowed shape.
        "-ncrole=x",
        "-nc role=x",
        "-n-role=x",
        "-n- role=x",
        "-ec role=x",
        "-Pcsession_authorization=x",
        "-TEjnc role=x",
        // Other switches, a dangling `-c`, a word that is not an option.
        "-e",
        "-c",
        "-c role",
        "role=x",
        "-c search_path=app -fs",
    ] {
        assert!(!pg_options_read_only_allowed(value), "{value:?}");
        assert_eq!(
            read_only_violations(DbmsKind::Postgres, &owned(&[("options", value)])),
            [Violation {
                key: "options".into(),
                reason: ViolationReason::AlwaysDenied,
            }],
            "{value:?}"
        );
        // The ordinary sanitizer still passes it from the user (a read-write profile keeps it).
        assert!(
            sanitize(
                &input(DbmsKind::Postgres, UserTyped, &[("options", value)], &[]),
                cat()
            )
            .is_ok(),
            "{value:?}"
        );
    }
    // The server's own splitting (a vertical tab separates words); such a value is refused by
    // the control-character rule anyway, for every origin.
    assert!(!pg_options_read_only_allowed(
        "-c\x0bsession_authorization=x"
    ));
    // The allowed shapes, with names that only look alike.
    for value in [
        "-c rolex=1",
        "-c search_path=role",
        "-csearch_path=app --statement-timeout=5s -c application_name=role=x",
        "",
        "   ",
    ] {
        assert!(pg_options_read_only_allowed(value), "{value:?}");
        assert!(
            read_only_violations(DbmsKind::Postgres, &owned(&[("options", value)])).is_empty(),
            "{value:?}"
        );
    }
    // Other engines and other keys are left alone.
    assert!(read_only_violations(DbmsKind::MySql, &owned(&[("options", "-c role=x")])).is_empty());
    assert!(
        read_only_violations(DbmsKind::Postgres, &owned(&[("search_path", "-c role=x")]))
            .is_empty()
    );
}
