//! M2b Task 7: the environment plan and schema tracking (HV-SESS-06, -07, Q8, Q15, Q27).

mod common;

use common::*;
use hacivat_driver::ExecSummary;
use hacivat_net::TlsMode;
use hacivat_proto::DbmsKind;
use hacivat_session::profile::Profile;
use hacivat_session::session::env::{quote_my_ident, quote_pg_ident, quote_pg_literal};
use hacivat_session::session::{
    EnvStep, ExecErrorKind, Request, SchemaCtx, SessionEvent, TxCommand, TxMode, build_env_plan,
    needs_schema_reprobe, parse_schema_probe, schema_probe_sql, switch_schema_sql,
};

#[test]
fn env_plan_order() {
    let p = build_env_plan(
        DbmsKind::Postgres,
        &ro_profile(),
        true,
        Some(&schema("app")),
    );
    assert_eq!(
        p.statements(),
        [
            "SET application_name = 'Hacivat'",
            "SET default_transaction_read_only = on",
            "SET standard_conforming_strings = on",
            "SET client_encoding = 'UTF8'",
            "SET idle_in_transaction_session_timeout = '30s'",
            "SET search_path TO \"app\""
        ]
    );
}

#[test]
fn env_plan_of_mysql_and_without_a_schema() {
    // MySQL's identity travels in the connect attributes; no schema known, none restored.
    let p = build_env_plan(
        DbmsKind::MySql,
        &mysql_profile(TlsMode::Disable),
        true,
        Some(&SchemaCtx::default()),
    );
    assert_eq!(
        p.statements(),
        ["SET SESSION TRANSACTION READ ONLY", "SET NAMES utf8mb4"]
    );
    let rw = build_env_plan(
        DbmsKind::MySql,
        &mysql_profile(TlsMode::Disable),
        false,
        None,
    );
    assert!(rw.statements().is_empty());
    let mut app = schema("shop");
    app.database = Some("shop".into());
    let with = build_env_plan(
        DbmsKind::MySql,
        &mysql_profile(TlsMode::Disable),
        false,
        Some(&app),
    );
    assert_eq!(with.statements(), ["USE `shop`"]);
}

#[test]
fn the_client_identity_is_quoted_as_a_literal() {
    let profile = Profile {
        application_name: "it's \\ mine".into(),
        ..pg_profile(TlsMode::Disable)
    };
    let p = build_env_plan(DbmsKind::Postgres, &profile, false, None);
    assert_eq!(p.steps, [EnvStep::ClientIdentity("it's \\ mine".into())]);
    assert_eq!(
        p.statements(),
        ["SET application_name = E'it''s \\\\ mine'"]
    );
}

#[test]
fn quoting_functions() {
    assert_eq!(quote_pg_ident("a\"b"), "\"a\"\"b\"");
    assert_eq!(quote_my_ident("a`b"), "`a``b`");
    assert_eq!(quote_pg_literal("o'k"), "'o''k'");
    assert_eq!(quote_pg_literal("a\\b"), "E'a\\\\b'");
    // NUL never reaches the server: it would end the C string early.
    assert!(!quote_pg_literal("a\0b").contains('\0'));
}

#[test]
fn switch_schema_quotes_exactly() {
    assert_eq!(
        switch_schema_sql(DbmsKind::Postgres, "Müşteri"),
        "SET search_path TO \"Müşteri\""
    );
    assert_eq!(
        switch_schema_sql(DbmsKind::Postgres, "a\"b"),
        "SET search_path TO \"a\"\"b\""
    );
    assert_eq!(switch_schema_sql(DbmsKind::MySql, "an`x"), "USE `an``x`");
    // Refused (empty): a NUL in a name, an empty name, and SQLite, which has no current schema.
    assert_eq!(switch_schema_sql(DbmsKind::Postgres, "a\0b"), "");
    assert_eq!(switch_schema_sql(DbmsKind::MySql, ""), "");
    assert_eq!(switch_schema_sql(DbmsKind::Sqlite, "main"), "");
}

#[test]
fn reprobe_rule() {
    for s in ["SET search_path=x", "  /* c */ use db", "rollback", "Begin"] {
        assert!(needs_schema_reprobe(s), "{s}");
    }
    for s in ["select 1", "settings", "user"] {
        assert!(!needs_schema_reprobe(s), "{s}");
    }
    for s in [
        "-- c\nRESET ALL",
        "# c\nUSE db",
        "\tSTART TRANSACTION",
        "commit;",
        "END",
        "abort",
        "discard all",
    ] {
        assert!(needs_schema_reprobe(s), "{s}");
    }
    for s in ["", "   ", "/* unterminated use db", "update t set a = 1"] {
        assert!(!needs_schema_reprobe(s), "{s}");
    }
}

#[test]
fn probe_statements_and_their_parse() {
    assert_eq!(
        schema_probe_sql(DbmsKind::Postgres),
        "SELECT current_database(), current_schema(), array_to_string(current_schemas(true), ','), current_user, current_setting('search_path')"
    );
    assert_eq!(
        schema_probe_sql(DbmsKind::MySql),
        "SELECT DATABASE(), CURRENT_USER()"
    );
    assert_eq!(schema_probe_sql(DbmsKind::Sqlite), "PRAGMA database_list");
    let row = |v: &[Option<&str>]| ExecSummary {
        rows: vec![v.iter().map(|x| x.map(String::from)).collect()],
        ..ExecSummary::default()
    };
    assert_eq!(
        parse_schema_probe(
            DbmsKind::Postgres,
            &row(&[
                Some("d"),
                Some("app"),
                Some("app,public"),
                Some("u"),
                Some("\"$user\", public")
            ])
        ),
        SchemaCtx {
            database: Some("d".into()),
            schema: Some("app".into()),
            search_path: vec!["app".into(), "public".into()],
            user: Some("u".into()),
            search_path_setting: vec!["$user".into(), "public".into()],
        }
    );
    assert_eq!(
        parse_schema_probe(DbmsKind::MySql, &row(&[None, Some("u@%")])),
        SchemaCtx {
            user: Some("u@%".into()),
            ..SchemaCtx::default()
        }
    );
    assert_eq!(
        parse_schema_probe(
            DbmsKind::Sqlite,
            &row(&[Some("0"), Some("main"), Some("/tmp/x.db")])
        ),
        SchemaCtx {
            schema: Some("main".into()),
            search_path: vec!["main".into()],
            ..SchemaCtx::default()
        }
    );
    assert_eq!(
        parse_schema_probe(DbmsKind::Postgres, &ExecSummary::default()),
        SchemaCtx::default()
    );
}

#[tokio::test]
async fn schema_reprobed_after_rollback() {
    let (h, f) = session(fake_pg().exec(vec![rows(
        "SELECT current_database()",
        &[&["d", "app", "app,public", "u"]],
    )]))
    .await;
    let mut ev = h.events();
    run(&h, "ROLLBACK").await.unwrap(); // a user statement
    assert!(
        f.executed()
            .iter()
            .any(|s| s.starts_with("SELECT current_database()"))
    );
    // Q32
    assert!(matches!(
        next_schema_changed(&mut ev).await,
        SessionEvent::SchemaChanged(_)
    ));
    assert_eq!(
        snap(&h).schema.and_then(|s| s.schema).as_deref(),
        Some("app")
    );
}

#[tokio::test]
async fn internal_control_statements_never_reprobe() {
    // Q27
    let (h, f) = session(fake_mysql()).await;
    tx_cmd(&h, TxCommand::SetMode(TxMode::Manual))
        .await
        .unwrap();
    assert!(
        !f.executed()
            .iter()
            .any(|s| s.starts_with("SELECT DATABASE()"))
    );
}

#[tokio::test]
async fn plain_statements_and_reset_never_reprobe() {
    let (h, f) =
        session(fake_pg().exec(vec![rows("SELECT current_user = session_user", &[&["t"]])])).await;
    run(&h, "select 1").await.unwrap();
    h.submit(Request::Reset).reply.await.unwrap().unwrap();
    assert!(
        !f.executed()
            .iter()
            .any(|s| s.starts_with("SELECT current_database()")),
        "{:?}",
        f.executed()
    );
}

#[tokio::test]
async fn a_user_commit_reprobes() {
    let (h, f) = session(fake_pg()).await;
    tx_cmd(&h, TxCommand::Commit).await.unwrap();
    assert_eq!(f.executed()[0], "COMMIT");
    assert!(f.executed()[1].starts_with("SELECT current_database()"));
}

#[tokio::test]
async fn switch_schema_runs_the_switch_then_reprobes_and_reset_restores_it() {
    let (h, f) = session(fake_pg().exec(vec![
        rows(
            "SELECT current_database()",
            &[&["d", "Müşteri", "Müşteri,public", "u"]],
        ),
        rows("SELECT current_user = session_user", &[&["t"]]),
    ]))
    .await;
    let mut ev = h.events();
    h.submit(Request::SwitchSchema("Müşteri".into()))
        .reply
        .await
        .unwrap()
        .unwrap();
    assert_eq!(f.executed()[0], "SET search_path TO \"Müşteri\"");
    assert!(matches!(
        next_schema_changed(&mut ev).await,
        SessionEvent::SchemaChanged(SchemaCtx { schema: Some(s), .. }) if s == "Müşteri"
    ));
    h.submit(Request::Reset).reply.await.unwrap().unwrap();
    assert_eq!(
        &f.executed()[f.executed().len() - 2..],
        [
            "SET application_name = 'Hacivat'",
            "SET search_path TO \"Müşteri\""
        ]
    );
}

#[tokio::test]
async fn a_schema_name_with_nul_is_refused() {
    let (h, f) = session(fake_pg()).await;
    let r = h
        .submit(Request::SwitchSchema("a\0b".into()))
        .reply
        .await
        .unwrap();
    assert!(matches!(r.unwrap_err().kind, ExecErrorKind::Internal));
    assert!(f.executed().is_empty());
}

// ## m2b-t07 M11 ruling: PostgreSQL restores the whole `search_path` setting

#[test]
fn the_search_path_setting_is_restored_whole_with_user_kept() {
    let ctx = SchemaCtx {
        schema: Some("public".into()),
        search_path_setting: vec!["$user".into(), "public".into()],
        ..SchemaCtx::default()
    };
    let p = build_env_plan(DbmsKind::Postgres, &ro_profile(), false, Some(&ctx));
    assert_eq!(
        p.statements(),
        [
            "SET application_name = 'Hacivat'",
            "SET search_path TO \"$user\", \"public\""
        ]
    );
    // A name with a quote and a space is quoted exactly; one with NUL is never sent.
    let odd = SchemaCtx {
        search_path_setting: vec!["My \"Schema\"".into(), "public".into()],
        ..SchemaCtx::default()
    };
    let p = build_env_plan(DbmsKind::Postgres, &ro_profile(), false, Some(&odd));
    assert_eq!(
        p.statements()[1],
        "SET search_path TO \"My \"\"Schema\"\"\", \"public\""
    );
    let nul = SchemaCtx {
        schema: Some("app".into()),
        search_path_setting: vec!["a\0b".into()],
        ..SchemaCtx::default()
    };
    let p = build_env_plan(DbmsKind::Postgres, &ro_profile(), false, Some(&nul));
    // The setting with NUL is never sent; the current schema is restored instead.
    assert_eq!(p.statements()[1], "SET search_path TO \"app\"");
    assert!(!p.statements().iter().any(|s| s.contains('\0')));
}

#[test]
fn the_setting_is_parsed_like_postgresql_reads_it() {
    let parse = |setting: &str| {
        parse_schema_probe(
            DbmsKind::Postgres,
            &ExecSummary {
                rows: vec![vec![None, None, None, None, Some(setting.to_string())]],
                ..ExecSummary::default()
            },
        )
        .search_path_setting
    };
    assert_eq!(parse("\"$user\", public"), ["$user", "public"]);
    // Unquoted names fold to ASCII lower case; quoted ones keep case, commas and `""`.
    assert_eq!(
        parse(" Foo ,\"My \"\"S\"\"\",\"x,y\""),
        ["foo", "My \"S\"", "x,y"]
    );
    assert!(parse("").is_empty());
}

#[tokio::test]
async fn a_two_element_search_path_survives_a_reset_and_a_reprobe() {
    let probe_answer: &[&str] = &[
        "d",
        "My Schema",
        "My Schema,public",
        "u",
        "\"My Schema\", public",
    ];
    let (h, f) = session(fake_pg().exec(vec![
        rows("SELECT current_database()", &[probe_answer]),
        rows("SELECT current_user = session_user", &[&["t"]]),
        rows("SELECT current_database()", &[probe_answer]),
    ]))
    .await;
    run(&h, "SET search_path TO \"My Schema\", public")
        .await
        .unwrap();
    let want = ["My Schema".to_string(), "public".to_string()];
    assert_eq!(snap(&h).schema.unwrap().search_path_setting, want);
    h.submit(Request::Reset).reply.await.unwrap().unwrap();
    assert_eq!(
        f.executed().last().map(String::as_str),
        Some("SET search_path TO \"My Schema\", \"public\"")
    );
    run(&h, "ROLLBACK").await.unwrap(); // a re-probe
    assert_eq!(snap(&h).schema.unwrap().search_path_setting, want);
}
