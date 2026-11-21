//! M3b Task 14: the orchestrator `classify` (`alg/13` SQL-SAFE-02 steps; R-M3-3): the L0
//! limits and the pre-pass before any backend, the execution-mode split, the dispatch per
//! dialect, the combine (producer contract, `COPY … TO STDOUT`, context changes), the
//! fingerprint, units classified one at a time, the read corpora through the real entry, the
//! T9/T10 regression probes, hostile scripts, and the 10 KB budget.

use std::path::PathBuf;
use std::time::Instant;
// ## m3b-t24-exec-ui (named exception)
use std::sync::Arc;

use sql_dialect::{ClientFlavor, DialectCtx, DialectId, LexModes, PgWireFlavor, ServerVersion};
use sql_safety::{
    Actor, ClassifyCtx, CoarseClass, EffectSet, Expect, GateClass, Limits, MAX_CLONING_WORK,
    MAX_OPERATOR_CHAIN, Profile, ReasonCode, SafetyReport, Verdict, changes_context, classify,
    classify_interim, classify_units, decide, gate_class, pg_backend_hooks,
};
use sql_split::FingerprintAlgo;

// ---------------------------------------------------------------------------------------------
// Fixtures

fn ctx_of(d: DialectId, flavor: ClientFlavor, expect: Expect) -> ClassifyCtx {
    ClassifyCtx::new(DialectCtx::new(d, None), flavor, expect)
}

fn pg() -> ClassifyCtx {
    ctx_of(
        DialectId::Postgres,
        ClientFlavor::Psql,
        Expect::SingleStatement,
    )
}

fn pg_single() -> ClassifyCtx {
    ctx_of(
        DialectId::Postgres,
        ClientFlavor::None,
        Expect::SingleStatement,
    )
}

fn pg_script() -> ClassifyCtx {
    ctx_of(DialectId::Postgres, ClientFlavor::None, Expect::Script)
}

fn script(d: DialectId) -> ClassifyCtx {
    ctx_of(d, ClientFlavor::None, Expect::Script)
}

fn single(d: DialectId) -> ClassifyCtx {
    ctx_of(d, ClientFlavor::None, Expect::SingleStatement)
}

/// Every reason of `r`: the pre-decision's and every item's.
fn codes(r: &SafetyReport) -> Vec<ReasonCode> {
    r.pre_decision
        .iter()
        .flatten()
        .chain(r.items.iter().flat_map(|i| &i.reasons))
        .map(|x| x.code)
        .collect()
}

fn has_reason(r: &SafetyReport, code: ReasonCode) -> bool {
    codes(r).contains(&code)
}

fn read_only_allows(r: &SafetyReport) -> bool {
    decide(r, Profile::ReadOnlyPlusExplain, Actor::Human).allows()
}

/// `r` proves nothing: `Unknown`, the read-only profiles deny it, and no item is a read at the
/// gate.
fn assert_failed_closed(r: &SafetyReport, what: &str) {
    assert_eq!(r.request_class, CoarseClass::Unknown, "{what}: {r:?}");
    assert!(
        r.request_effects.contains(EffectSet::UNKNOWN)
            || r.items.iter().any(|i| i.class == CoarseClass::Unknown),
        "{what}"
    );
    for p in [Profile::ReadOnly, Profile::ReadOnlyPlusExplain] {
        assert!(
            matches!(decide(r, p, Actor::Human).verdict, Verdict::Deny(_)),
            "{what}: {p:?}"
        );
    }
    for i in &r.items {
        assert_ne!(gate_class(i), GateClass::Read, "{what}");
    }
}

// ---------------------------------------------------------------------------------------------
// Step 1–3: before any backend

#[test]
fn prechecks_deny_before_any_backend() {
    for (s, code) in [
        ("\0", ReasonCode::LexError),
        ("\\set x 1", ReasonCode::ClientCommand),
        ("SELECT ${x}", ReasonCode::TemplateVariable),
        ("", ReasonCode::NoStatement),
    ] {
        let r = classify(s, &pg());
        assert!(
            r.pre_decision
                .as_ref()
                .unwrap()
                .iter()
                .any(|r| r.code == code),
            "{s:?}: {r:?}"
        );
        assert!(r.items.is_empty(), "{s:?}: a backend ran");
        assert_failed_closed(&r, s);
        assert_eq!(r.dialect, Some(DialectId::Postgres), "{s:?}");
        assert!(r.matches_text(s), "{s:?}");
    }
}

#[test]
fn more_prechecks_each_with_its_reason() {
    let cases: &[(&str, ClassifyCtx, ReasonCode)] = &[
        ("SELECT 'abc", pg_single(), ReasonCode::LexError),
        ("SELECT /* open", pg_single(), ReasonCode::LexError),
        ("SELECT 1x", pg_single(), ReasonCode::LexError),
        (
            "   -- only a comment\n",
            pg_single(),
            ReasonCode::NoStatement,
        ),
        (";;", pg_script(), ReasonCode::NoStatement),
        (
            "COPY t FROM STDIN;\n1\t2\n\\.\n",
            ctx_of(DialectId::Postgres, ClientFlavor::Psql, Expect::Script),
            ReasonCode::ClientCommand,
        ),
        (
            "DELIMITER $$\nSELECT 1$$\n",
            ctx_of(DialectId::MySql, ClientFlavor::MySqlCli, Expect::Script),
            ReasonCode::ClientCommand,
        ),
        (
            ".tables\nSELECT 1;",
            ctx_of(DialectId::Sqlite, ClientFlavor::Sqlite3, Expect::Script),
            ReasonCode::ClientCommand,
        ),
        (
            "SELECT * FROM t WHERE a = %v%",
            single(DialectId::MySql),
            ReasonCode::TemplateVariable,
        ),
        (
            "SELECT 1; SELECT ${x}",
            pg_script(),
            ReasonCode::TemplateVariable,
        ),
        (
            "SELECT 1; DELETE FROM t",
            pg_single(),
            ReasonCode::MultiStatement,
        ),
    ];
    for (s, ctx, code) in cases {
        let r = classify(s, ctx);
        assert!(r.pre_decision.is_some(), "{s:?}: {r:?}");
        assert!(has_reason(&r, *code), "{s:?}: {:?}", codes(&r));
        assert!(r.items.is_empty(), "{s:?}");
        assert_failed_closed(&r, s);
    }
}

#[test]
fn oversized_input_is_too_complex_and_never_analysed() {
    let mut ctx = pg_single();
    ctx.limits = Limits {
        max_bytes: 64,
        ..Limits::default()
    };
    let s = format!("SELECT {}", "1 + ".repeat(20)) + "1";
    assert!(s.len() > 64);
    let r = classify(&s, &ctx);
    assert_eq!(
        r.pre_decision
            .as_deref()
            .map(|v| v.iter().map(|x| x.code).collect::<Vec<_>>()),
        Some(vec![ReasonCode::TooComplex])
    );
    assert!(r.matches_text(&s));
    assert_failed_closed(&r, "oversized");
    // The default cap: 1 MiB.
    let big = format!("SELECT 1{}", " ".repeat(1 << 20));
    assert!(has_reason(
        &classify(&big, &pg_single()),
        ReasonCode::TooComplex
    ));
    // Exactly at the cap is analysed.
    ctx.limits.max_bytes = s.len();
    assert!(classify(&s, &ctx).pre_decision.is_none());
}

#[test]
fn too_many_tokens_is_too_complex() {
    let mut ctx = pg_single();
    ctx.limits.max_tokens = 10;
    // 11 significant tokens: SELECT 1 , 2 , 3 , 4 , 5 , 6 → 12.
    let r = classify("SELECT 1, 2, 3, 4, 5, 6", &ctx);
    assert!(has_reason(&r, ReasonCode::TooComplex), "{r:?}");
    assert!(r.items.is_empty());
    // Comments and spaces are not tokens that count.
    let ok = classify("SELECT /* a b c d e f g h i j k */ 1, 2", &ctx);
    assert!(ok.pre_decision.is_none(), "{ok:?}");
}

#[test]
fn nesting_over_256_is_too_complex() {
    let s = format!("SELECT {}1{}", "(".repeat(257), ")".repeat(257));
    assert!(has_reason(&classify(&s, &pg()), ReasonCode::TooComplex));
    let r = classify(&s, &pg());
    assert!(r.items.is_empty(), "no backend runs past the nesting limit");
    // Brackets of every kind count together, and so do `CASE … END` blocks.
    let mixed = format!(
        "SELECT {}{}1{}{}",
        "ARRAY[".repeat(129),
        "(".repeat(128),
        ")".repeat(128),
        "]".repeat(129)
    );
    assert!(has_reason(&classify(&mixed, &pg()), ReasonCode::TooComplex));
    let deep = format!(
        "SELECT {}{}1{}{}",
        "(".repeat(57),
        "CASE WHEN true THEN ".repeat(200),
        " END".repeat(200),
        ")".repeat(57)
    );
    assert!(
        has_reason(&classify(&deep, &pg()), ReasonCode::TooComplex),
        "{deep:.80}"
    );
    // `END IF` closes a stored-program block, not a `CASE`; 256 deep is within the limit.
    let at_limit = format!("SELECT {}1{}", "(".repeat(256), ")".repeat(256));
    assert!(!has_reason(
        &classify(&at_limit, &pg()),
        ReasonCode::TooComplex
    ));
    // MySQL: the same limit before sqlparser.
    let my = format!("SELECT {}1{}", "(".repeat(300), ")".repeat(300));
    let r = classify(&my, &single(DialectId::MySql));
    assert!(r.items.is_empty() && has_reason(&r, ReasonCode::TooComplex));
}

#[test]
fn ambiguous_comment_end_is_unknown() {
    // An executable comment whose end depends on how the server reads its quote.
    for d in [DialectId::MySql, DialectId::MariaDb] {
        let s = "SELECT 1 /*!50000 '*/ , 2 -- ' */";
        let r = classify(s, &single(d));
        assert!(
            has_reason(&r, ReasonCode::AmbiguousBoundary),
            "{d:?}: {:?}",
            codes(&r)
        );
        assert!(r.items.is_empty(), "{d:?}: no backend runs");
        assert_failed_closed(&r, s);
    }
}

#[test]
fn single_statement_expect_denies_two() {
    assert!(has_reason(
        &classify("SELECT 1; SELECT 2", &pg_single()),
        ReasonCode::MultiStatement
    ));
    for d in [DialectId::MySql, DialectId::MariaDb, DialectId::Sqlite] {
        let r = classify("SELECT 1; SELECT 2", &single(d));
        assert!(has_reason(&r, ReasonCode::MultiStatement), "{d:?}");
        assert_failed_closed(&r, "two");
    }
    // A trailing terminator and comment are one statement.
    let one = classify("SELECT 1; -- done", &pg_single());
    assert_eq!(one.request_class, CoarseClass::Read, "{one:?}");
}

#[test]
fn script_expect_classifies_each_item() {
    assert_eq!(
        classify("SELECT 1; DELETE FROM t", &pg_script())
            .items
            .len(),
        2
    );
    let r = classify("SELECT 1; DELETE FROM t", &pg_script());
    assert_eq!(r.items[0].class, CoarseClass::Read);
    assert_eq!(r.items[1].class, CoarseClass::Write);
    assert_eq!(r.request_class, CoarseClass::Write);
    // Each item is its strict range: the bytes sent.
    assert_eq!(
        usize::from(r.items[1].range.start())..usize::from(r.items[1].range.end()),
        10..23
    );
    for d in [DialectId::MySql, DialectId::MariaDb, DialectId::Sqlite] {
        let r = classify("SELECT 1; DELETE FROM t", &script(d));
        assert_eq!(r.items.len(), 2, "{d:?}");
        assert_eq!(r.request_class, CoarseClass::Write, "{d:?}");
    }
}

#[test]
fn unsupported_dialects_are_unknown() {
    for d in [
        DialectId::Generic,
        DialectId::SqlServer,
        DialectId::Oracle,
        DialectId::ClickHouse,
    ] {
        let r = classify("SELECT 1", &single(d));
        assert!(has_reason(&r, ReasonCode::UnsupportedDialect), "{d:?}");
        assert_eq!(r.dialect, Some(d));
        assert!(r.backends.is_empty(), "{d:?}");
        assert_failed_closed(&r, "unsupported");
    }
    // The PG-wire flavours take the PostgreSQL backend.
    let r = classify(
        "SELECT 1",
        &single(DialectId::PgWire(PgWireFlavor::Cockroach)),
    );
    assert_eq!(r.request_class, CoarseClass::Read);
    assert_eq!(r.backends[0].name, "libpg_query");
}

#[test]
fn reports_record_dialect_backends_and_the_exact_bytes() {
    let s = "  SELECT 1 ;  ";
    let r = classify(s, &pg_single());
    assert_eq!(r.dialect, Some(DialectId::Postgres));
    assert_eq!(
        r.backends.iter().map(|b| b.name).collect::<Vec<_>>(),
        ["libpg_query"]
    );
    assert!(r.matches_text(s) && !r.matches_text("SELECT 1"));
    assert_eq!(r.classified_len, 14);
    let m = classify("SELECT 1", &single(DialectId::MySql));
    assert_eq!(
        m.backends
            .iter()
            .map(|b| (b.name, b.version))
            .collect::<Vec<_>>(),
        [("sqlparser", "0.64.0"), ("lexer-prescan", "1")]
    );
}

// ---------------------------------------------------------------------------------------------
// Turkish process locale

#[test]
fn turkish_process_locale_changes_nothing() {
    // Run under LC_ALL=tr_TR.UTF-8 in turkey-test: every comparison is ASCII, so the dotted and
    // dotless i never fold into a keyword and `insert` is INSERT whatever the locale.
    let cases: &[(&str, DialectId, CoarseClass)] = &[
        (
            "insert into t values (1)",
            DialectId::Postgres,
            CoarseClass::Write,
        ),
        (
            "select * from müşteri where İd = 1",
            DialectId::Postgres,
            CoarseClass::Read,
        ),
        (
            "delete from ışık where id = 1",
            DialectId::Postgres,
            CoarseClass::Write,
        ),
        (
            "insert into t values (1)",
            DialectId::MySql,
            CoarseClass::Write,
        ),
        (
            "select upper('ı') from t",
            DialectId::MySql,
            CoarseClass::Read,
        ),
        ("SELECT lower('İ')", DialectId::Sqlite, CoarseClass::Read),
    ];
    for (s, d, class) in cases {
        assert_eq!(
            classify(s, &single(*d)).request_class,
            *class,
            "{s} on {d:?}"
        );
    }
    // A dotless `ınsert` is no keyword: never a known write, never a read.
    for d in [DialectId::Postgres, DialectId::MySql, DialectId::Sqlite] {
        let r = classify("ınsert into t values (1)", &single(d));
        assert_failed_closed(&r, "ınsert");
    }
    // The fingerprint upper-cases keywords in ASCII only.
    let f = classify("select a from t", &single(DialectId::MySql)).fingerprint;
    assert_eq!(&*f.normalized, "SELECT a FROM t");
}

// ---------------------------------------------------------------------------------------------
// Fingerprints

#[test]
fn fingerprint_falls_back_on_parse_errors() {
    let ok = classify("SELECT a FROM t WHERE id = 1", &pg_single());
    assert_eq!(ok.fingerprint.algo, FingerprintAlgo::PgQuery);
    let same = classify("select a from t where id = 42", &pg_single());
    assert_eq!(ok.fingerprint.hash, same.fingerprint.hash);
    let bad = classify("SELEC a FROM t", &pg_single());
    assert!(
        has_reason(&bad, ReasonCode::ParseError),
        "{:?}",
        codes(&bad)
    );
    assert_eq!(bad.fingerprint.algo, FingerprintAlgo::HvNorm1);
    assert_eq!(
        bad.fingerprint,
        sql_split::hv_norm1(
            "SELEC a FROM t",
            &DialectCtx::new(DialectId::Postgres, None)
        )
    );
    // Too deep for the protobuf decode of the tree: Unknown, though libpg_query's fingerprint
    // (no tree decode) still reads it.
    let deep = format!("SELECT {}1{}", "ARRAY[".repeat(60), "]".repeat(60));
    let r = classify(&deep, &pg_single());
    assert!(has_reason(&r, ReasonCode::TooComplex), "{:?}", codes(&r));
    assert_failed_closed(&r, "deep arrays");
    // Pre-decided requests never reach the parser.
    let pre = classify("SELECT 'x", &pg_single());
    assert_eq!(pre.fingerprint.algo, FingerprintAlgo::HvNorm1);
    // Other dialects: HvNorm1.
    let m = classify("SELECT 1", &single(DialectId::MySql));
    assert_eq!(m.fingerprint.algo, FingerprintAlgo::HvNorm1);
}

// ---------------------------------------------------------------------------------------------
// Every failure path

#[test]
fn every_failure_path_is_unknown_or_denied() {
    let mut tiny = pg_single();
    tiny.limits.max_bytes = 4;
    let mut few = single(DialectId::MySql);
    few.limits.max_tokens = 2;
    let mut no_views = single(DialectId::MySql);
    no_views.limits.max_exec_comment_views = 1;
    let mut pg19 = pg_single();
    pg19.dialect.version = Some(ServerVersion {
        major: 19,
        minor: 0,
        patch: 0,
    });
    let mut oracle_mode = single(DialectId::MariaDb);
    oracle_mode.dialect.modes = LexModes {
        mariadb_oracle_mode: true,
        ..LexModes::default_for(DialectId::MariaDb)
    };
    let mut bad_path = pg_single();
    bad_path.search_path = vec!["\"public\", pg_catalog".into()].into();
    let mut catalog_late = pg_single();
    catalog_late.search_path = vec!["public".into(), "pg_catalog".into()].into();
    let deep = format!("SELECT {}1{}", "(SELECT ".repeat(120), ")".repeat(120));
    let unions = (0..120)
        .map(|i| format!("SELECT {i}"))
        .collect::<Vec<_>>()
        .join(" UNION ALL ");
    let mysql_deep = format!("SELECT {}1{}", "(".repeat(80), ")".repeat(80));
    let cases: Vec<(String, ClassifyCtx, &str)> = vec![
        ("SELECT 1".into(), tiny, "max_bytes"),
        ("SELECT 1, 2".into(), few, "max_tokens"),
        (
            "SELECT 1 /*!50000 , 2 */ /*!50100 , 3 */".into(),
            no_views,
            "threshold views",
        ),
        ("SELECT 1".into(), pg19, "a server newer than the grammar"),
        ("SELECT 1".into(), oracle_mode, "MariaDB sql_mode=ORACLE"),
        (
            "SELECT lower('a')".into(),
            bad_path,
            "an unreadable search_path",
        ),
        (
            "SELECT lower('a')".into(),
            catalog_late,
            "pg_catalog after public",
        ),
        ("SELEC 1".into(), pg_single(), "a PG syntax error"),
        (
            "SELEC 1".into(),
            single(DialectId::MySql),
            "a MySQL syntax error",
        ),
        (
            "SELEC 1".into(),
            single(DialectId::Sqlite),
            "a SQLite syntax error",
        ),
        (deep, pg_single(), "prost's decode depth"),
        (unions, pg_single(), "100 UNIONs"),
        (
            mysql_deep,
            single(DialectId::MySql),
            "sqlparser's recursion limit",
        ),
        (
            "SELECT 1 FROM t FOR SYSTEM_TIME ALL".into(),
            single(DialectId::MariaDb),
            "MariaDB-only syntax",
        ),
        ("SELECT f(1)".into(), pg_single(), "an unknown function"),
        (
            "SELECT f(1)".into(),
            single(DialectId::MySql),
            "an unknown MySQL function",
        ),
        ("EXECUTE p".into(), pg_single(), "EXECUTE"),
        ("DO $$ BEGIN END $$".into(), pg_single(), "DO"),
        ("CALL p()".into(), pg_single(), "CALL"),
        ("COPY t TO STDOUT".into(), pg_single(), "COPY TO STDOUT"),
        (
            "COPY (SELECT 1) TO STDOUT".into(),
            pg_single(),
            "COPY query TO STDOUT",
        ),
    ];
    for (s, ctx, what) in &cases {
        let r = classify(s, ctx);
        assert_failed_closed(&r, what);
    }
    // Nothing in this table runs a backend twice into a panic, and every report names its
    // bytes and dialect.
    for (s, ctx, what) in &cases {
        let r = classify(s, ctx);
        assert!(r.matches_text(s), "{what}");
        assert_eq!(r.dialect, Some(ctx.dialect.id), "{what}");
    }
}

// ---------------------------------------------------------------------------------------------
// COPY … TO STDOUT (alg/13 S-12)

#[test]
fn copy_to_stdout_is_never_a_read_and_other_copies_keep_their_class() {
    for s in [
        "COPY t TO STDOUT",
        "copy t (a, b) to stdout with (format csv)",
        "COPY (SELECT * FROM t) TO STDOUT",
    ] {
        let r = classify(s, &pg_single());
        assert!(!read_only_allows(&r), "{s}");
        assert!(
            r.items[0]
                .reasons
                .iter()
                .any(|x| x.code == ReasonCode::EffectsUnknown
                    && x.arg.as_deref() == Some("COPY TO STDOUT")),
            "{s}: {:?}",
            r.items[0].reasons
        );
    }
    assert_eq!(
        classify("COPY t TO PROGRAM 'x'", &pg_single()).request_class,
        CoarseClass::Admin
    );
    assert_eq!(
        classify("COPY t FROM STDIN", &pg_single()).request_class,
        CoarseClass::Write
    );
}

// ---------------------------------------------------------------------------------------------
// Context changes inside one request and across units

#[test]
fn a_context_change_makes_every_later_item_unknown() {
    // Inside one PostgreSQL script (the backend's rule, Task 9, and the orchestrator's).
    for s in [
        "SET search_path = evil, pg_catalog; SELECT lower('a')",
        "SELECT set_config('search_path', 'evil, pg_catalog', false); SELECT 1",
        "SET standard_conforming_strings = off; SELECT 'a\\'; DROP TABLE t; --'",
        "SET ROLE admin; SELECT 1",
        "CREATE FUNCTION lower(text) RETURNS text AS 'select 1' LANGUAGE sql; SELECT lower('a')",
        "FETCH c; SELECT 1",
        "MOVE c; SELECT 1",
        "EXECUTE p; SELECT 1",
        "CALL p(); SELECT 1",
        "RESET ALL; SELECT 1",
    ] {
        let r = classify(s, &pg_script());
        assert!(!read_only_allows(&r), "{s}");
        if r.pre_decision.is_some() {
            // Fix round 1, C2: the two `standard_conforming_strings` readings cut the
            // backslash script differently.
            assert!(has_reason(&r, ReasonCode::AmbiguousBoundary), "{s}: {r:?}");
            continue;
        }
        let last = r.items.last().unwrap();
        assert_eq!(last.class, CoarseClass::Unknown, "{s}: {r:?}");
        assert!(
            last.reasons
                .iter()
                .any(|x| x.code == ReasonCode::EffectsUnknown),
            "{s}"
        );
        assert!(!read_only_allows(&r), "{s}");
    }
    // MySQL, MariaDB, SQLite: the orchestrator's rule (the interim backend reads statements
    // alone).
    for (d, s) in [
        (
            DialectId::MySql,
            "SET sql_mode = 'ANSI_QUOTES'; SELECT \"a\" FROM t",
        ),
        (DialectId::MySql, "SET NAMES gbk; SELECT 1"),
        (DialectId::MariaDb, "SET SESSION sql_mode = ''; SELECT 1"),
        (DialectId::MySql, "CALL p(); SELECT 1"),
        (DialectId::MySql, "EXECUTE s; SELECT 1"),
        (DialectId::MySql, "CREATE TABLE t (a int); SELECT a FROM t"),
        (
            DialectId::Sqlite,
            "CREATE TEMP VIEW t AS SELECT 1; SELECT * FROM t",
        ),
    ] {
        let r = classify(s, &script(d));
        let last = r.items.last().unwrap();
        assert_eq!(last.class, CoarseClass::Unknown, "{d:?} {s}: {r:?}");
        assert!(!read_only_allows(&r), "{d:?} {s}");
    }
    // A harmless SET and plain reads change nothing.
    let r = classify(
        "SET statement_timeout = 5000; SELECT 1; SELECT 2",
        &pg_script(),
    );
    assert_eq!(r.items[2].class, CoarseClass::Read, "{r:?}");
    let r = classify("SELECT 1; SELECT lower('a')", &pg_script());
    assert_eq!(r.request_class, CoarseClass::Read);
}

#[test]
fn units_classified_one_at_a_time_inherit_the_context_change() {
    let ctx = pg_single();
    // The carry (ADR 0005 m3b-t09, R1-M5): the unit that *runs* the code is the one after
    // which the context may differ.
    for first in [
        "SET search_path = evil, pg_catalog",
        "FETCH c",
        "MOVE NEXT IN c",
        "EXECUTE p",
        "CALL p()",
        "DO $$ BEGIN PERFORM set_config('search_path', 'evil', false); END $$",
        "SET standard_conforming_strings = off",
        "CREATE FUNCTION f() RETURNS int AS 'select 1' LANGUAGE sql",
        "SELECT 'unterminated",
        // Every context changer Task 9's in-request rule knows (I2a, R1-I1, R1-I2, R1-M1).
        "SELECT set_config('search_path', 'evil, pg_catalog', false)",
        "SELECT set_config('application_name', 'x', false)",
        "UPDATE pg_settings SET setting = 'evil' WHERE name = 'search_path'",
        "SET LOCAL search_path = evil, pg_catalog",
        "RESET search_path",
        "RESET ALL",
        "DISCARD ALL",
        "SET ROLE admin",
        "SET SESSION AUTHORIZATION admin",
        "SELECT pg_reload_conf()",
        "LOAD 'x'",
        "CREATE TEMP VIEW pg_class AS SELECT 1",
        "CREATE OPERATOR === (LEFTARG = int, RIGHTARG = int, FUNCTION = f)",
        "ALTER ROLE CURRENT_USER SET search_path = evil, pg_catalog",
        "SET client_encoding = 'SJIS'",
    ] {
        let reports = classify_units(&[first, "SELECT lower('a')", "SELECT 1"], &ctx);
        assert_eq!(reports.len(), 3);
        for r in &reports[1..] {
            assert_eq!(
                r.request_class,
                CoarseClass::Unknown,
                "after {first}: {r:?}"
            );
            assert!(!read_only_allows(r), "after {first}");
            assert!(
                changes_context("x", r),
                "an Unknown unit changes the context"
            );
        }
        assert!(changes_context(first, &reports[0]), "{first}");
    }
    // A cursor declared over `set_config` runs it at `FETCH`: the units after the `FETCH` are
    // stale, whatever the `DECLARE` was judged.
    let reports = classify_units(
        &[
            "DECLARE c CURSOR FOR SELECT set_config('search_path', 'evil, pg_catalog', false)",
            "FETCH c",
            "SELECT lower('a')",
        ],
        &ctx,
    );
    assert_eq!(
        reports[2].request_class,
        CoarseClass::Unknown,
        "{:?}",
        reports[2]
    );
    // The same across runs: a `FETCH` alone tells the caller to re-read the context.
    let fetch = classify("FETCH NEXT FROM c", &ctx);
    assert!(changes_context("FETCH NEXT FROM c", &fetch));
    // Reads keep later units reads, and their reports are exactly `classify`'s.
    let units = [
        "SELECT 1",
        "SET statement_timeout = 1000",
        "SELECT lower('a')",
    ];
    let reports = classify_units(&units, &ctx);
    for (u, r) in units.iter().zip(&reports) {
        assert_eq!(*r, classify(u, &ctx), "{u}");
        assert!(!changes_context(u, r), "{u}");
    }
    // MySQL: a lexing-mode change.
    let my = single(DialectId::MySql);
    let reports = classify_units(
        &["SET sql_mode = 'ANSI_QUOTES'", "SELECT \"a\" FROM t"],
        &my,
    );
    assert_eq!(reports[1].request_class, CoarseClass::Unknown);
    // A report of other bytes, or with no dialect, is a change.
    let r = classify("SELECT 1", &ctx);
    assert!(changes_context("SELECT 2", &r));
    let mut nameless = r.clone();
    nameless.dialect = None;
    assert!(changes_context("SELECT 1", &nameless));
    // The stale report still classifies its own bytes (the gate's stale check holds).
    let reports = classify_units(&["CALL p()", "SELECT 1"], &ctx);
    assert!(reports[1].matches_text("SELECT 1"));
}

/// The cross-unit rule covers the PostgreSQL backend's in-request one: whenever the backend
/// makes the statement after `x` stale inside one script, `changes_context` says `x` changes
/// the context on its own (else a run that sends `x` and the read as two units would lose it).
#[test]
fn the_cross_unit_rule_covers_the_pg_backends_in_request_rule() {
    let ctx = pg_script();
    let mut probes: Vec<String> = [
        "SELECT set_config('search_path', 'evil, pg_catalog', false)",
        "SELECT set_config('application_name', 'x', true)",
        "SELECT pg_catalog.set_config('a.b', 'x', false)",
        "SELECT set_config(current_user, 'x', false)",
        "UPDATE pg_settings SET setting = 'x' WHERE name = 'search_path'",
        "UPDATE public.pg_settings SET setting = 'x'",
        "SET search_path = evil, pg_catalog",
        "SET search_path TO DEFAULT",
        "SET LOCAL search_path = \"$user\", public, pg_catalog",
        "RESET search_path",
        "RESET ALL",
        "DISCARD ALL",
        "SET ROLE admin",
        "RESET ROLE",
        "SET SESSION AUTHORIZATION admin",
        "SET standard_conforming_strings = off",
        "SET client_encoding = 'SJIS'",
        "SET backslash_quote = on",
        "SELECT pg_reload_conf()",
        "LOAD 'x'",
        "CALL lower('a')",
        "DO $$ BEGIN END $$",
        "EXECUTE p",
        "CREATE TEMP VIEW t AS SELECT 1",
        "CREATE FUNCTION f() RETURNS int AS 'select 1' LANGUAGE sql",
        "SELECT f()",
        "SELECT \"lower\"('a')",
        "COPY t TO STDOUT",
        "SET statement_timeout = 1000",
        "SET search_path = public",
        "SELECT 1",
        "SELECT lower('a')",
        "DELETE FROM t WHERE id = 1",
        "BEGIN",
        "COMMIT",
        "LISTEN ch",
        "PREPARE p AS SELECT 1",
        "DECLARE c CURSOR FOR SELECT 1",
        "FETCH c",
        "LOCK TABLE t",
        "SELECT pg_advisory_lock(1)",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    probes.extend(corpus("pg.sql").into_iter().take(120));
    let mut stale_seen = 0;
    for x in &probes {
        let script = format!("{x}; SELECT 1");
        let backend = pg_backend_hooks::classify_pg(&script, &ctx);
        if backend.items.len() != 2 {
            continue;
        }
        let backend_stale = backend.items[1]
            .reasons
            .iter()
            .any(|r| r.code == ReasonCode::EffectsUnknown);
        if backend_stale {
            stale_seen += 1;
            assert!(
                changes_context(x, &classify(x, &single(DialectId::Postgres))),
                "{x}: the backend taints what follows it, the unit rule does not"
            );
        }
    }
    assert!(stale_seen >= 25, "{stale_seen}");
}

// ---------------------------------------------------------------------------------------------
// Hostile scripts through the full entry point

#[test]
fn hostile_scripts_mixing_engine_tricks_are_never_reads() {
    let pg_cases = [
        // Comment and string tricks around a write.
        "SELECT 1 /* ; */ ; DELETE FROM t",
        "SELECT $$;$$; DROP TABLE t",
        "SELECT 'a\\'; DROP TABLE t; --'",
        "SELECT 1; COMMIT; DROP TABLE t",
        "WITH d AS (DELETE FROM t RETURNING *) SELECT * FROM d",
        "SELECT * FROM t FOR UPDATE",
        "SELECT * INTO new_t FROM t",
        "EXPLAIN ANALYZE DELETE FROM t",
        "SELECT pg_sleep(10)",
        "SELECT lo_export(1, '/tmp/x')",
        "SELECT dblink_exec('x', 'DROP TABLE t')",
        "SELECT * FROM (SELECT * FROM (SELECT nextval('s')) a) b",
        "SELECT * FROM t, LATERAL (SELECT txid_current()) x",
        "SELECT \"lower\"('a')",
        "SELECT public.lower('a')",
        "SELECT 1 \\g",
        "UPDATE pg_settings SET setting = 'evil' WHERE name = 'search_path'; SELECT lower('a')",
        "SELECT * FROM pg_stat_activity",
        "LOCK TABLE t",
        "LISTEN ch",
        "NOTIFY ch",
        "DECLARE c CURSOR WITH HOLD FOR SELECT 1",
        "SET SESSION AUTHORIZATION 'x'",
        "\u{feff}DELETE FROM t",
    ];
    for s in pg_cases {
        for ctx in [pg_script(), pg()] {
            let r = classify(s, &ctx);
            // Denied on read-only; a denylisted builtin such as `pg_sleep` keeps a coarse read.
            assert!(!read_only_allows(&r), "{s}: {r:?}");
        }
    }
    let mysql_cases = [
        "SELECT 1 /*!50000 ; DELETE FROM t */",
        "SELECT 1 /*!99999 , (SELECT 1 FROM t INTO OUTFILE '/tmp/x') */",
        "SELECT 1 /*M! , 2 */ INTO OUTFILE '/tmp/x'",
        "SELECT '/*M!' , 1 /*M! INTO OUTFILE '/tmp/x' */",
        "/*M! DELETE FROM t */ SELECT 1",
        "SELECT 1; /*M! DELETE FROM t */",
        "SELECT 1 /*m! , 2 INTO OUTFILE '/tmp/x' */",
        "SELECT 1--1;DROP TABLE t",
        "SELECT 1 # ; DROP TABLE t\n; DELETE FROM t",
        "SELECT * FROM t INTO OUTFILE '/tmp/x'",
        "SELECT * FROM t INTO @v",
        "SELECT SLEEP(10)",
        "SELECT BENCHMARK(1000000, MD5('a'))",
        "SELECT /*+ SET_VAR(transaction_read_only = 0) */ 1",
        "SET @@SESSION.transaction_read_only = 0",
        "SET SESSION TRANSACTION READ WRITE",
        "LOAD DATA LOCAL INFILE '/etc/passwd' INTO TABLE t",
        "HANDLER t OPEN",
        "LOCK TABLES t WRITE",
        "SHOW PROCESSLIST",
        "SELECT * FROM information_schema.PROCESSLIST",
        "DO SLEEP(1)",
        "PREPARE s FROM 'DELETE FROM t'",
        "REPLACE INTO t VALUES (1)",
        "INSERT IGNORE INTO t VALUES (1)",
        "SELECT * FROM t FOR UPDATE",
        "SELECT * FROM t LOCK IN SHARE MODE",
    ];
    for s in mysql_cases {
        for d in [DialectId::MySql, DialectId::MariaDb] {
            for ctx in [script(d), single(d)] {
                let r = classify(s, &ctx);
                assert!(!read_only_allows(&r), "{d:?} {s}: {r:?}");
                assert!(
                    r.items.iter().all(|i| gate_class(i) != GateClass::Read)
                        || r.request_class != CoarseClass::Read,
                    "{d:?} {s}"
                );
            }
        }
    }
    let sqlite_cases = [
        "ATTACH DATABASE '/tmp/x.db' AS x",
        "VACUUM INTO '/tmp/x.db'",
        "PRAGMA journal_mode = WAL",
        "PRAGMA query_only = 0",
        "SELECT load_extension('x')",
        "SELECT writefile('/tmp/x', 'y')",
        "INSERT OR REPLACE INTO t VALUES (1)",
        "SELECT 1; DETACH x",
        "WITH x AS (SELECT 1) INSERT INTO t SELECT * FROM x",
    ];
    for s in sqlite_cases {
        for ctx in [script(DialectId::Sqlite), single(DialectId::Sqlite)] {
            let r = classify(s, &ctx);
            assert!(!read_only_allows(&r), "{s}: {r:?}");
        }
    }
}

#[test]
fn t9_and_t10_regression_probes_hold_through_classify() {
    // T9 review I1: user-type coercions outside :: and CAST.
    for s in [
        "SELECT JSON_VALUE('1', '$' RETURNING mytype)",
        "SELECT * FROM JSON_TABLE('[]', '$[*]' COLUMNS (a mytype PATH '$'))",
        "SELECT XMLSERIALIZE(CONTENT '<a/>' AS mytype)",
        "SELECT * FROM f() AS x(a mytype)",
        // T9 R1-I1: a procedure is never a builtin.
        "CALL lower('a')",
        // T9 I2b: the role and session resets.
        "DISCARD ALL",
        "SELECT pg_reload_conf()",
        "LOAD 'x'",
    ] {
        let r = classify(s, &pg_single());
        assert!(!read_only_allows(&r), "{s}: {r:?}");
    }
    // T9 R1-I2 in a script: UPDATE pg_settings, then a read.
    let r = classify(
        "UPDATE pg_settings SET setting = 'x' WHERE name = 'search_path'; SELECT lower('a')",
        &pg_script(),
    );
    assert_eq!(r.items[1].class, CoarseClass::Unknown);
    // T10 C1: an unversioned /*M! on MySQL hides INTO OUTFILE behind a quote.
    let r = classify(
        "SELECT 1 /*M! , ' */ INTO OUTFILE '/tmp/x' -- ' */",
        &single(DialectId::MySql),
    );
    assert!(!read_only_allows(&r), "{r:?}");
    // T10 C2: many executable comments are TooComplex quickly, never a hang.
    let many = format!(
        "SELECT 1 {}",
        (0..63)
            .map(|i| format!("/*!{} , {i} */", 50000 + i))
            .collect::<Vec<_>>()
            .join(" ")
    );
    let start = Instant::now();
    let r = classify(&many, &single(DialectId::MySql));
    assert!(start.elapsed().as_secs() < 2);
    assert!(has_reason(&r, ReasonCode::TooComplex), "{:?}", codes(&r));
    // T10 RR-I1: SHOW EXPLAIN FOR another connection, SHOW REPLICA 'x' STATUS.
    for s in [
        "SHOW EXPLAIN FORMAT=JSON FOR 5",
        "SHOW REPLICA 'x' STATUS",
        "SHOW SLAVE 'x' STATUS",
    ] {
        let r = classify(s, &single(DialectId::MariaDb));
        assert!(!read_only_allows(&r), "{s}: {r:?}");
    }
    // T10 I1: SET GLOBAL / PERSIST are Admin.
    for s in [
        "SET GLOBAL max_connections = 1",
        "SET PERSIST max_connections = 1",
    ] {
        let r = classify(s, &single(DialectId::MySql));
        assert!(r.request_class >= CoarseClass::Admin, "{s}: {r:?}");
    }
}

#[test]
fn classify_is_never_more_permissive_than_the_backend_harnesses() {
    let pg_inputs = [
        "SELECT 1",
        "SELECT lower(a) FROM t",
        "DELETE FROM t",
        "SET search_path = public; SELECT 1",
        "COPY t TO STDOUT",
        "FETCH c; SELECT 1",
        "SELECT 1; SELECT 2",
    ];
    for s in pg_inputs {
        for ctx in [pg_single(), pg_script()] {
            let ours = classify(s, &ctx);
            let theirs = pg_backend_hooks::classify_pg(s, &ctx);
            if read_only_allows(&ours) {
                assert!(read_only_allows(&theirs), "{s}");
            }
            assert!(
                ours.request_class >= theirs.request_class,
                "{s}: {ours:?} vs {theirs:?}"
            );
        }
    }
    for d in [DialectId::MySql, DialectId::MariaDb, DialectId::Sqlite] {
        for s in [
            "SELECT 1",
            "DELETE FROM t",
            "SELECT 1; SELECT 2",
            "SET NAMES gbk; SELECT 1",
        ] {
            for ctx in [single(d), script(d)] {
                let ours = classify(s, &ctx);
                let theirs = classify_interim(s, &ctx);
                if read_only_allows(&ours) {
                    assert!(read_only_allows(&theirs), "{d:?} {s}");
                }
                assert!(ours.request_class >= theirs.request_class, "{d:?} {s}");
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------
// The read corpora through the real entry

fn corpus(file: &str) -> Vec<String> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../sql-testkit/corpus/read")
        .join(file);
    let text = std::fs::read_to_string(path).unwrap();
    let mut out = Vec::new();
    let mut cur = String::new();
    for line in text.lines() {
        if cur.is_empty() && (line.starts_with("--") || line.trim().is_empty()) {
            continue;
        }
        if !cur.is_empty() {
            cur.push('\n');
        }
        cur.push_str(line);
        if let Some(stmt) = cur.strip_suffix(';') {
            out.push(stmt.to_string());
            cur.clear();
        }
    }
    out
}

/// The PostgreSQL corpus entries that read-only consoles refuse by an earlier ruling: none
/// may join them silently.
const PG_RULED_DENIALS: [&str; 5] = [
    // Fix round 1, C2; round 2, ruling 4: `U&''` lexes differently with
    // `standard_conforming_strings` off, which is unknown here, so the read is `Unknown`.
    "SELECT E'tab\\there', U&'d\\0061t\\+000061'",
    // T3 ruling (5): `version()` stays unknown (PO list).
    "SELECT version()",
    // T9 fix round 1, I1: a coercion to a user type may run a domain CHECK or a user cast.
    "SELECT status::text FROM orders WHERE status = 'paid'::order_status",
    "SELECT enum_range(NULL::order_status)",
    // T3 fix round 1, I3: the json(b)_populate/to_record rows are unreviewed (domain CHECK).
    "SELECT x FROM jsonb_to_recordset('[{\"x\":1}]'::jsonb) AS t(x int)",
];

#[test]
fn read_corpora_have_no_false_denial_through_classify() {
    let mut failures = Vec::new();
    let mut total = 0;
    for (file, d) in [
        ("pg.sql", DialectId::Postgres),
        ("mysql.sql", DialectId::MySql),
        ("mariadb.sql", DialectId::MariaDb),
        ("sqlite.sql", DialectId::Sqlite),
    ] {
        for q in corpus(file) {
            total += 1;
            let r = classify(&q, &single(d));
            let ruled = d == DialectId::Postgres && PG_RULED_DENIALS.contains(&q.as_str());
            if read_only_allows(&r) == ruled {
                failures.push(format!("{d:?}: {q}: {:?}", codes(&r)));
            }
        }
    }
    assert!(total >= 1200, "{total}");
    assert!(
        failures.is_empty(),
        "{} false denials:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

/// The orchestrator lends its lexing to the backends only without a client flavor (T10 C1
/// carry); with one, each backend lexes on its own. Both ways give the same items.
#[test]
fn a_lent_lexing_classifies_exactly_like_the_backends_own() {
    let hostile = [
        "SELECT 1 /*!50000 , 2 */",
        "SELECT 'a\\'; DROP TABLE t; --'",
        "SET search_path = evil, pg_catalog; SELECT lower('a')",
        "SELECT * FROM t WHERE a = 'İ' AND b = $$x$$",
    ];
    for (file, d, cli) in [
        ("pg.sql", DialectId::Postgres, ClientFlavor::Psql),
        ("mysql.sql", DialectId::MySql, ClientFlavor::MySqlCli),
        ("mariadb.sql", DialectId::MariaDb, ClientFlavor::MySqlCli),
        ("sqlite.sql", DialectId::Sqlite, ClientFlavor::Sqlite3),
    ] {
        let inputs = corpus(file)
            .into_iter()
            .chain(hostile.iter().map(|s| s.to_string()));
        for q in inputs {
            let lent = classify(&q, &script(d));
            let own = classify(&q, &ctx_of(d, cli, Expect::Script));
            assert_eq!(lent.items, own.items, "{d:?}: {q}");
            assert_eq!(lent.request_class, own.request_class, "{d:?}: {q}");
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Fix round 1

/// Every infix operator of each interim dialect, as `(operator, right operand)`.
fn infix_operators(d: DialectId) -> Vec<(&'static str, &'static str)> {
    let shared: &[(&str, &str)] = &[
        ("+", "a"),
        ("-", "a"),
        ("*", "a"),
        ("/", "a"),
        ("%", "a"),
        ("=", "a"),
        ("<>", "a"),
        ("!=", "a"),
        ("<", "a"),
        (">", "a"),
        ("<=", "a"),
        (">=", "a"),
        ("||", "a"),
        ("AND", "a"),
        ("OR", "a"),
        ("LIKE", "a"),
        ("NOT LIKE", "a"),
        ("LIKE a ESCAPE", "'x'"),
        ("REGEXP", "a"),
        ("NOT REGEXP", "a"),
        ("|", "a"),
        ("&", "a"),
        ("<<", "a"),
        (">>", "a"),
        ("->", "'$'"),
        ("->>", "'$'"),
        ("COLLATE", "binary"),
        ("IS", "NULL"),
        ("IS NOT", "NULL"),
        ("IN", "(a)"),
        ("NOT IN", "(a)"),
        ("BETWEEN a AND", "a"),
    ];
    let own: &[(&str, &str)] = match d {
        DialectId::Sqlite => &[
            ("==", "a"),
            ("GLOB", "a"),
            ("NOT GLOB", "a"),
            ("MATCH", "a"),
            ("NOT MATCH", "a"),
            ("IS DISTINCT FROM", "a"),
        ],
        _ => &[
            ("DIV", "a"),
            ("MOD", "a"),
            ("XOR", "a"),
            ("&&", "a"),
            ("<=>", "a"),
            ("^", "a"),
            ("RLIKE", "a"),
            ("SOUNDS LIKE", "a"),
            ("MEMBER OF", "(a)"),
        ],
    };
    shared.iter().chain(own).copied().collect()
}

/// Whether `op` is an operator `sqlparser` parses by cloning its left operand in `d`
/// (`MAX_CLONING_WORK`): MySQL/MariaDB `DIV`, SQLite `REGEXP`/`GLOB`/`MATCH`.
fn cloning(d: DialectId, op: &str) -> bool {
    let words: &[&str] = if d == DialectId::Sqlite {
        &["REGEXP", "GLOB", "MATCH"]
    } else {
        &["DIV"]
    };
    words.iter().any(|w| op.ends_with(w))
}

/// Significant tokens of `text` in `d`.
fn significant_tokens(text: &str, d: DialectId) -> usize {
    let dctx = DialectCtx::new(d, None);
    let profile = sql_dialect::lex_profile(&dctx).unwrap();
    let lexed = sql_lexer::lex(text, profile, dctx.modes, &sql_lexer::LexState::Initial);
    (0..lexed.len()).filter(|&i| !lexed.is_trivia(i)).count()
}

/// The most links of `SELECT a <op> <rhs> …` the cloning cap allows: the work is the sum, over
/// the cloning operators, of the significant tokens before each (`SELECT a` is 2, a link
/// `link_tokens`, of which `lead` come before the cloning word: `NOT` in `NOT REGEXP`).
fn cloning_links(link_tokens: usize, lead: usize) -> usize {
    let (mut n, mut work) = (0usize, 0u64);
    loop {
        let before = (2 + n * link_tokens + lead) as u64;
        if work + before > MAX_CLONING_WORK {
            return n;
        }
        work += before;
        n += 1;
    }
}

/// C1: every infix operator of every interim dialect, chained as long as the caps allow,
/// classifies on the test thread (2 MiB: `classify` hops to its own stack) in a bounded time,
/// and one more link is `TooComplex` before any parser runs.
#[test]
fn every_infix_operator_chain_is_capped_and_classifies_in_bounded_time() {
    // n cloning links of `per_link` counted operators each: n × n·per_link ≤ MAX_CLONING_WORK.
    let mut slowest = (0.0f64, String::new());
    for d in [DialectId::MySql, DialectId::MariaDb, DialectId::Sqlite] {
        for (op, rhs) in infix_operators(d) {
            // Operators per link (`NOT LIKE` is two counted words, `BETWEEN a AND` two, …).
            let per_link = op
                .split(' ')
                .filter(|w| {
                    !w.is_empty()
                        && ([
                            "AND", "OR", "XOR", "NOT", "DIV", "MOD", "LIKE", "RLIKE", "REGEXP",
                            "GLOB", "MATCH", "SOUNDS", "IS", "IN", "BETWEEN", "COLLATE", "ESCAPE",
                            "MEMBER",
                        ]
                        .contains(w)
                            || !w.chars().all(|c| c.is_ascii_alphabetic()))
                })
                .count()
                .max(1);
            let by_chain = MAX_OPERATOR_CHAIN as usize / per_link;
            let links = if cloning(d, op) {
                // The cloning operator is the link's last counted operator.
                by_chain.min(cloning_links(
                    significant_tokens(&format!(" {op} {rhs}"), d),
                    significant_tokens(op, d) - 1,
                ))
            } else {
                by_chain
            };
            let chain = |n: usize| format!("SELECT a{}", format!(" {op} {rhs}").repeat(n));
            let at = chain(links);
            let start = Instant::now();
            let r = classify(&at, &single(d));
            let took = start.elapsed().as_secs_f64();
            assert!(
                !has_reason(&r, ReasonCode::TooComplex) || r.pre_decision.is_none(),
                "{d:?} `{op}` ×{links}: refused at the cap ({:?})",
                codes(&r)
            );
            assert!(took < 2.0, "{d:?} `{op}` ×{links}: {took:.2} s");
            if std::env::var("T14_CHAIN_TIMES").is_ok() {
                eprintln!("chain {d:?} `{op}` ×{links}: {:.1} ms", took * 1e3);
            }
            if took > slowest.0 {
                slowest = (took, format!("{d:?} `{op}` ×{links}"));
            }
            let past = classify(&chain(links + 1), &single(d));
            assert!(
                past.pre_decision
                    .as_deref()
                    .is_some_and(|p| p.iter().any(|x| x.code == ReasonCode::TooComplex)),
                "{d:?} `{op}` ×{}: not capped ({:?})",
                links + 1,
                codes(&past)
            );
        }
    }
    eprintln!(
        "slowest chain at the cap: {} in {:.0} ms",
        slowest.1,
        slowest.0 * 1e3
    );
    // A long linear chain followed by cloning operators: the clones copy the whole chain.
    for d in [DialectId::MySql, DialectId::Sqlite] {
        let op = if d == DialectId::Sqlite {
            "REGEXP"
        } else {
            "DIV"
        };
        let s = format!(
            "SELECT a{}{}",
            " + a".repeat(1_000),
            format!(" {op} a").repeat(23)
        );
        let start = Instant::now();
        let r = classify(&s, &single(d));
        assert!(start.elapsed().as_secs_f64() < 2.0, "{d:?}");
        assert!(r.matches_text(&s));
        assert!(
            !has_reason(&r, ReasonCode::TooComplex),
            "{d:?}: {:?}",
            codes(&r)
        );
        let more = format!("{s}{}", format!(" {op} a").repeat(8));
        assert!(
            has_reason(&classify(&more, &single(d)), ReasonCode::TooComplex),
            "{d:?}"
        );
    }
}

/// C1: the reviewer's crashing inputs are refused before `sqlparser` sees them, quickly.
#[test]
fn the_reviewers_quadratic_chains_are_too_complex_at_once() {
    for (d, s) in [
        (
            DialectId::Sqlite,
            format!("SELECT a{}", " REGEXP a".repeat(5_000)),
        ),
        (
            DialectId::Sqlite,
            format!("SELECT a{}", " REGEXP a".repeat(20_000)),
        ),
        (
            DialectId::Sqlite,
            format!("SELECT a{}", " GLOB a".repeat(99_994)),
        ),
        (
            DialectId::Sqlite,
            format!("SELECT a{}", " MATCH a".repeat(99_994)),
        ),
        (
            DialectId::MySql,
            format!("SELECT a{}", " DIV a".repeat(20_000)),
        ),
        (
            DialectId::MariaDb,
            format!("SELECT a{}", " DIV a".repeat(5_000)),
        ),
        (
            DialectId::MySql,
            format!("SELECT 1{}", " + 1".repeat(99_000)),
        ),
        (
            DialectId::Sqlite,
            format!(
                "SELECT 1 FROM t WHERE {}",
                vec!["a = 1"; 49_000].join(" AND ")
            ),
        ),
    ] {
        let start = Instant::now();
        let r = classify(&s, &single(d));
        assert!(start.elapsed().as_secs_f64() < 2.0, "{d:?}");
        assert!(
            r.items.is_empty() && has_reason(&r, ReasonCode::TooComplex),
            "{d:?}"
        );
    }
    // Fix round 2, R1-C1: the same chains inside executable comments, where L0 sees one
    // comment token; every reading is capped before `sqlparser`. R1-M1: a long left operand
    // counts by its size. (Run in debug by default and in release with `--release`.)
    let ones = vec!["1"; 90_000].join(",");
    for (d, s) in [
        (
            DialectId::MySql,
            format!("SELECT a{}", " /*!DIV a*/".repeat(5_000)),
        ),
        (
            DialectId::MySql,
            format!("SELECT a{}", " /*!50000 DIV a*/".repeat(5_000)),
        ),
        (
            DialectId::MariaDb,
            format!("SELECT a{}", " /*M!DIV a*/".repeat(5_000)),
        ),
        (
            DialectId::MariaDb,
            format!("SELECT a{}", " /*!DIV a*/".repeat(5_000)),
        ),
        (
            DialectId::MySql,
            format!("SELECT a /*!{} */", " DIV a".repeat(5_000)),
        ),
        (
            DialectId::MySql,
            format!("SELECT a /*!{} */", " DIV a".repeat(20_000)),
        ),
        (
            DialectId::MariaDb,
            format!("SELECT a /*M!{} */", " DIV a".repeat(20_000)),
        ),
        (
            DialectId::MySql,
            format!("SELECT a /*!{} */", " + a".repeat(99_000)),
        ),
        (
            DialectId::MySql,
            format!("SELECT f({ones}){}", " DIV a".repeat(250)),
        ),
        (
            DialectId::Sqlite,
            format!("SELECT ({ones}){}", " REGEXP a".repeat(250)),
        ),
    ] {
        let start = Instant::now();
        let r = classify(&s, &single(d));
        let took = start.elapsed().as_secs_f64();
        assert!(took < 2.0, "{d:?} {:.60}: {took:.2} s", s);
        assert!(
            has_reason(&r, ReasonCode::TooComplex),
            "{d:?} {s:.60}: {:?}",
            codes(&r)
        );
        assert!(!read_only_allows(&r), "{d:?} {s:.60}");
    }
    // A chain split by commas or statements is many short chains.
    let items = vec!["a REGEXP a"; 2_000].join(", ");
    let r = classify(&format!("SELECT {items}"), &single(DialectId::Sqlite));
    assert!(
        has_reason(&r, ReasonCode::TooComplex),
        "cloning work is per statement"
    );
    let list = vec!["a + a"; 3_000].join(", ");
    let r = classify(&format!("SELECT {list}"), &single(DialectId::Sqlite));
    assert!(!has_reason(&r, ReasonCode::TooComplex), "{:?}", codes(&r));
}

/// I2: `classify`, `classify_units` and the harnesses run on their own 16 MiB stack when the
/// calling thread is small. Before the fix these aborted the process on a 2 MiB thread.
#[test]
fn every_entry_is_safe_on_a_two_mib_thread() {
    let at_cap = format!("SELECT a{}", " + a".repeat(MAX_OPERATOR_CHAIN as usize));
    let deep = format!("SELECT 1{}", " + 1".repeat(99_000));
    let h = std::thread::Builder::new()
        .stack_size(2 << 20)
        .spawn(move || {
            let my = single(DialectId::MySql);
            let a = classify(&at_cap, &my).request_class;
            let b = classify_units(&[at_cap.as_str(), "SELECT 1"], &my).len();
            // The harness has no L0: its tree is 99,000 deep.
            let c = classify_interim(&deep, &my).request_class;
            let d = pg_backend_hooks::classify_pg(&deep, &pg_single()).request_class;
            (a, b, c, d)
        })
        .unwrap();
    let (a, b, c, _) = h.join().expect("no stack overflow");
    assert_eq!(a, CoarseClass::Read);
    assert_eq!(b, 2);
    // Since fix round 2 the interim harness caps the chain in each reading too.
    assert_eq!(c, CoarseClass::Unknown);
}

/// C2 (controller ruling): the session's lexing modes are not known to `classify`, so every
/// reading the bytes allow is taken and the union denies what any reading denies. The
/// reviewer's repros, with the dialect's default modes.
#[test]
fn every_lexing_mode_reading_must_allow() {
    for (d, s) in [
        (DialectId::MySql, "SELECT 'a\\' INTO OUTFILE '/tmp/x' -- '"),
        (
            DialectId::MariaDb,
            "SELECT 'a\\' INTO OUTFILE '/tmp/x' -- '",
        ),
        (DialectId::MySql, "SELECT \"a\\\" , SLEEP(1) -- \""),
        (
            DialectId::Postgres,
            "SELECT 'a\\' || ' , dblink_exec($q$h$q$, $q$DELETE FROM t$q$) --'",
        ),
        (
            DialectId::Postgres,
            "SELECT 'a\\' || ' , pg_terminate_backend(1) --'",
        ),
        // Round 2: the NBE reading ends unterminated, but after a `;`: over a multi-statement
        // `COM_QUERY` the `DELETE` would run first, so that reading is kept.
        (DialectId::MySql, "SELECT 'a\\'; DELETE FROM t; --'"),
        (DialectId::MySql, "SET @a = 'x\\'; DELETE FROM t; --'"),
        // Round 3, R2-C1: the `;` hidden in an executable comment (its body runs as code).
        (DialectId::MySql, "SELECT 'a\\' /*!; DO SLEEP(5); */ x'"),
        (DialectId::MySql, "SELECT 'a\\' /*!; KILL 5; */ x'"),
        (
            DialectId::MySql,
            "SELECT 'a\\' /*!50000 ; DO GET_LOCK(0x61,1); */ x'",
        ),
        (DialectId::MariaDb, "SELECT 'a\\' /*M!; DO SLEEP(5); */ x'"),
        (DialectId::MySql, "SELECT \"a\\\" /*!; DO SLEEP(5); */ x\""),
    ] {
        let r = classify(s, &single(d));
        assert!(!read_only_allows(&r), "{d:?} {s}: {r:?}");
        assert!(r.matches_text(s));
    }
    // The other way round: a session the caller says is NO_BACKSLASH_ESCAPES still denies
    // what the default reading denies.
    let mut nbe = single(DialectId::MySql);
    nbe.dialect.modes = LexModes::from_mysql_sql_mode("NO_BACKSLASH_ESCAPES");
    let r = classify("SELECT 'a\\' , ' INTO OUTFILE ''/tmp/x'' -- '", &nbe);
    assert!(!read_only_allows(&r), "{r:?}");
    // Texts the modes read alike keep one reading and their verdicts.
    for (d, s) in [
        (DialectId::MySql, "SELECT 'a' FROM t WHERE b = 1"),
        (DialectId::MySql, "SELECT 'it\\'s' FROM t"),
        (DialectId::Postgres, "SELECT E'a\\nb' FROM t"),
        (DialectId::Postgres, "SELECT a FROM t WHERE b = 'x'"),
    ] {
        let r = classify(s, &single(d));
        if !s.contains('\\') {
            assert!(read_only_allows(&r), "{d:?} {s}: {r:?}");
        }
        assert!(r.matches_text(s));
    }
}

#[derive(serde::Deserialize)]
struct EscapedRows {
    row: Vec<EscapedRow>,
}

#[derive(serde::Deserialize)]
struct EscapedRow {
    dialect: String,
    kind: String,
    sql: String,
}

/// Fix round 2, R1-I1 ruling 5: honest statements with escaped literals are never denied for
/// their escapes. On the unknown-modes path every read is allowed on read-only and nothing is
/// denied under `Admin`; with the dialect's default modes known, the same.
#[test]
fn escaped_literals_are_no_false_denials_with_unknown_or_known_modes() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../sql-testkit/corpus/read/escaped_literals.toml");
    let rows: EscapedRows = toml::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    assert!(rows.row.len() >= 20);
    let mut failures = Vec::new();
    for row in &rows.row {
        let d = match row.dialect.as_str() {
            "pg" => DialectId::Postgres,
            "my" => DialectId::MySql,
            "ma" => DialectId::MariaDb,
            "sl" => DialectId::Sqlite,
            x => panic!("{x}"),
        };
        let unknown = single(d);
        let known = single(d).with_known_modes(LexModes::default_for(d));
        for (path, ctx) in [("unknown", &unknown), ("known", &known)] {
            let r = classify(&row.sql, ctx);
            let admin = decide(&r, Profile::Admin, Actor::Human).verdict;
            if matches!(admin, Verdict::Deny(_)) {
                failures.push(format!(
                    "{path} {d:?} Admin denies {}: {:?}",
                    row.sql,
                    codes(&r)
                ));
            }
            if row.kind == "read" && !read_only_allows(&r) {
                failures.push(format!(
                    "{path} {d:?} read denied {}: {:?}",
                    row.sql,
                    codes(&r)
                ));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// Fix round 2, R1-I1 rulings 1 and 4: known modes are read once, exactly; PostgreSQL with
/// `standard_conforming_strings` off is never parsed so, and a different cut is `Unknown`.
#[test]
fn known_modes_read_once_and_pg_scs_off_is_unknown_not_denied_always() {
    let mysql_trick = "SELECT 'a\\' INTO OUTFILE '/tmp/x' -- '";
    let default_known =
        single(DialectId::MySql).with_known_modes(LexModes::default_for(DialectId::MySql));
    // The server's modes are known to be the defaults: the text is a string and a comment.
    assert!(read_only_allows(&classify(mysql_trick, &default_known)));
    // Known NO_BACKSLASH_ESCAPES: the file write is code.
    let nbe = single(DialectId::MySql)
        .with_known_modes(LexModes::from_mysql_sql_mode("NO_BACKSLASH_ESCAPES"));
    assert!(!read_only_allows(&classify(mysql_trick, &nbe)));
    // Unknown: every reading.
    assert!(!read_only_allows(&classify(
        mysql_trick,
        &single(DialectId::MySql)
    )));

    let pg_trick = "SELECT 'a\\' || ' , dblink_exec($q$h$q$, $q$DELETE FROM t$q$) --'";
    let on = pg_single().with_known_modes(LexModes::default_for(DialectId::Postgres));
    assert!(read_only_allows(&classify(pg_trick, &on)));
    let off = pg_single().with_known_modes(LexModes {
        standard_conforming_strings: false,
        ..LexModes::default_for(DialectId::Postgres)
    });
    for ctx in [&off, &pg_single()] {
        let r = classify(pg_trick, ctx);
        assert_eq!(r.request_class, CoarseClass::Unknown, "{r:?}");
        assert!(
            !has_reason(&r, ReasonCode::ParserDisagreement),
            "{:?}",
            codes(&r)
        );
        assert!(
            r.items[0]
                .reasons
                .iter()
                .any(|x| x.arg.as_deref() == Some("standard_conforming_strings"))
        );
        assert!(!read_only_allows(&r));
        // Unknown, not deny-always: the Admin profile confirms rather than denies.
        assert!(!matches!(
            decide(&r, Profile::Admin, Actor::Human).verdict,
            Verdict::Deny(_)
        ));
    }
    // Round 3, R2-M1: the `off` reading ends a statement where the `on` reading does not:
    // another boundary, pre-decided `AmbiguousBoundary`, known `off` or unknown.
    let split = "SELECT 'a\\' || ' ; DELETE FROM t; --'";
    for ctx in [&off, &pg_single()] {
        let r = classify(split, ctx);
        assert!(r.pre_decision.is_some() && r.items.is_empty(), "{r:?}");
        assert!(
            has_reason(&r, ReasonCode::AmbiguousBoundary),
            "{:?}",
            codes(&r)
        );
        assert!(matches!(
            decide(&r, Profile::Admin, Actor::Human).verdict,
            Verdict::Deny(_)
        ));
    }
    // The same on MySQL with NO_BACKSLASH_ESCAPES unknown.
    let r = classify(
        "SELECT 'a\\'; DELETE FROM t; --'",
        &single(DialectId::MySql),
    );
    assert!(
        has_reason(&r, ReasonCode::AmbiguousBoundary),
        "{:?}",
        codes(&r)
    );
    // scs off and the same cut: the reading adds nothing.
    assert!(read_only_allows(&classify(
        "SELECT a FROM t WHERE b ~ '\\d+'",
        &off
    )));
}

/// Minors M2–M4 of the review.
#[test]
fn round_one_minors() {
    // M2: an oversized text is never lexed for its fingerprint.
    let mut tiny = single(DialectId::MySql);
    tiny.limits.max_bytes = 16;
    let s = "SELECT 1, 2, 3, 4, 5";
    let r = classify(s, &tiny);
    assert!(r.fingerprint.normalized.is_empty());
    assert_eq!(r.fingerprint.hash, sql_safety::classified_hash(s));
    let u = sql_safety::unknown_report(s, &tiny, ReasonCode::EffectsUnknown);
    assert!(u.fingerprint.normalized.is_empty() && u.matches_text(s));
    // M3: a PostgreSQL rollback undoes transactional SETs.
    for rb in [
        "ROLLBACK",
        "ABORT",
        "ROLLBACK TO SAVEPOINT s",
        "rollback prepared 'x'",
    ] {
        assert!(changes_context(rb, &classify(rb, &pg_single())), "{rb}");
    }
    let reports = classify_units(
        &[
            "BEGIN",
            "SET LOCAL search_path = public",
            "ROLLBACK",
            "SELECT lower('a')",
        ],
        &pg_single(),
    );
    assert_eq!(reports[3].request_class, CoarseClass::Unknown);
    // M4: `USE db` on MySQL and MariaDB.
    // ## m3b-t24-exec-ui (named exception, the T24 I5 ruling): a `USE` of a literal name
    // still changes the context, and the later unit is now read in it (`sys` current, so the
    // unqualified `session` is that schema's table) instead of being `Unknown`; one the
    // classifier can't read exactly still makes it `Unknown`.
    for d in [DialectId::MySql, DialectId::MariaDb] {
        let reports = classify_units(&["USE sys", "SELECT * FROM session"], &single(d));
        assert!(changes_context("USE sys", &reports[0]), "{d:?}");
        let alone = classify("SELECT * FROM session", &{
            let mut c = single(d);
            c.search_path = Arc::from(vec![Arc::<str>::from("sys")]);
            c
        });
        assert_eq!(reports[1].request_class, alone.request_class, "{d:?}");
        assert_eq!(codes(&reports[1]), codes(&alone), "{d:?}");
    }
    // M7 lives in `hacivat-exec` (`read_only_check`).
}

// ## m3b-t24-exec-ui (named exception: the T24 I5 ruling)
/// A context change whose new value is read exactly from the text sets the context of the
/// later units of the run; anything else still makes them `Unknown`.
#[test]
fn literal_context_switches_carry_into_later_units() {
    let path_of =
        |c: &ClassifyCtx| -> Vec<String> { c.search_path.iter().map(|s| s.to_string()).collect() };
    let with_path = |mut c: ClassifyCtx, p: &[&str]| {
        c.search_path = Arc::from(p.iter().map(|s| Arc::<str>::from(*s)).collect::<Vec<_>>());
        c
    };
    // MySQL/MariaDB `USE`: plain and backquoted names.
    for d in [DialectId::MySql, DialectId::MariaDb] {
        for (use_stmt, db) in [("USE shop", "shop"), ("use `my``db`", "my`db")] {
            let reports = classify_units(&[use_stmt, "SELECT id FROM orders"], &single(d));
            let alone = classify("SELECT id FROM orders", &with_path(single(d), &[db]));
            assert_eq!(
                reports[1].request_class,
                CoarseClass::Read,
                "{d:?} {use_stmt}"
            );
            assert_eq!(codes(&reports[1]), codes(&alone), "{d:?} {use_stmt}");
        }
        assert!(path_of(&with_path(single(d), &["shop"])) == ["shop"]);
    }
    // PostgreSQL `SET search_path`: names folded, quoted names and literals kept.
    for set in [
        "SET search_path = app, public",
        "SET search_path TO app, \"public\"",
        "set session search_path to 'app', public",
    ] {
        let reports = classify_units(&[set, "SELECT lower('a') FROM t"], &pg_single());
        let alone = classify(
            "SELECT lower('a') FROM t",
            &with_path(pg_single(), &["app", "public"]),
        );
        assert_eq!(reports[1].request_class, alone.request_class, "{set}");
        assert_eq!(codes(&reports[1]), codes(&alone), "{set}");
        assert_ne!(reports[1].request_class, CoarseClass::Unknown, "{set}");
    }
    // A path the harmless-SET list refuses (`pg_catalog` no longer first) does change the
    // context: the later unit is read in exactly that path, where an unqualified builtin may
    // be shadowed (the backend's `catalog_first` rule), never in the old one.
    let set = "SET search_path = evil, pg_catalog";
    let reports = classify_units(&[set, "SELECT lower('a') FROM t"], &pg_single());
    assert!(changes_context(set, &reports[0]));
    let alone = classify(
        "SELECT lower('a') FROM t",
        &with_path(pg_single(), &["evil", "pg_catalog"]),
    );
    assert_eq!(reports[1].request_class, alone.request_class);
    assert_eq!(codes(&reports[1]), codes(&alone));
    assert_ne!(
        reports[1].request_class,
        classify("SELECT lower('a') FROM t", &pg_single()).request_class,
        "never the old path's reading"
    );
    // Still `Unknown`: anything not read exactly.
    let pg_still = [
        "SET LOCAL search_path = app",
        "SET search_path = DEFAULT",
        "SET search_path TO app, $user",
        "SELECT set_config('search_path', 'app', false)",
        "SET search_path = app /* x */",
        "SET role app_user",
        "CALL p()",
        "SET search_path = 'a,b'",
    ];
    for first in pg_still {
        let reports = classify_units(&[first, "SELECT lower('a') FROM t"], &pg_single());
        if changes_context(first, &reports[0]) {
            assert_eq!(
                reports[1].request_class,
                CoarseClass::Unknown,
                "{first}: {:?}",
                codes(&reports[1])
            );
        }
    }
    for d in [DialectId::MySql, DialectId::MariaDb] {
        for first in ["USE `shop", "USE shop; SELECT 1", "CALL p()", "SET @a = 1"] {
            let reports = classify_units(&[first, "SELECT id FROM orders"], &single(d));
            if changes_context(first, &reports[0]) {
                assert_eq!(
                    reports[1].request_class,
                    CoarseClass::Unknown,
                    "{d:?} {first}"
                );
            }
        }
    }
    // Once a run is `Unknown` it stays so, a later literal switch notwithstanding.
    let reports = classify_units(
        &[
            "CALL p()",
            "SET search_path = app",
            "SELECT lower('a') FROM t",
        ],
        &pg_single(),
    );
    assert_eq!(reports[2].request_class, CoarseClass::Unknown);
}

// ---------------------------------------------------------------------------------------------
// The budget (release, HACIVAT_PERF=1)

/// One dense ~10 KB read unit: a wide select list over a join.
fn ten_kb_unit() -> String {
    let mut cols = Vec::new();
    let mut k = 0;
    while cols.iter().map(|c: &String| c.len() + 2).sum::<usize>() < 10_000 {
        cols.push(format!(
            "CASE WHEN o.id > {k} THEN UPPER(c.full_name) ELSE LOWER(c.email) END AS c{k}"
        ));
        k += 1;
    }
    format!(
        "SELECT {} FROM customers c JOIN orders o ON o.customer_id = c.id WHERE c.city = 'İstanbul'",
        cols.join(", ")
    )
}

/// `classify` per 10 KB unit (SQL-SAFE-02's ≤ 1 ms): the p50 and p95 of 200 runs per dialect
/// are printed against the spec's 1 ms. Neither path reaches 1 ms (Task 14 report: libpg_query's
/// parse and protobuf decode, and its fingerprint, are ~1.4 ms of PostgreSQL's ~2 ms;
/// `sqlparser`'s tokenize and parse ~0.85 ms of the interim path's ~1.5 ms), so the assertion
/// is T10's interim 2 ms, taken as T10 took it: on the fastest run, so a busy machine does not
/// fail it (ledger: C1 ruling; T20 measures, T30 decides against the budget).
#[test]
#[ignore = "perf"]
fn classify_10kb_unit_under_1ms() {
    if std::env::var("HACIVAT_PERF").as_deref() != Ok("1") {
        return;
    }
    // Measured on a classifier thread, as the pool runs it (round 2): elsewhere each call
    // first moves to a scoped thread of its own, which this budget is not about.
    sql_safety::spawn_classifier_thread("classify-budget".to_string(), budget)
        .unwrap()
        .join()
        .unwrap();
}

fn budget() {
    const RUNS: usize = 200;
    let unit = ten_kb_unit();
    for d in [
        DialectId::Postgres,
        DialectId::MySql,
        DialectId::MariaDb,
        DialectId::Sqlite,
    ] {
        let ctx = single(d);
        assert_eq!(
            classify(&unit, &ctx).request_class,
            CoarseClass::Read,
            "{d:?}"
        );
        let (best, p50, p95, rounds) = measure(&unit, &ctx, RUNS, 2.0);
        eprintln!(
            "{d:?}: classify best {best:.3}, p50 {p50:.3}, p95 {p95:.3} ms per 10 KB, {rounds} round(s) (budget 1 ms; interim assertion 2 ms on the best)"
        );
        assert!(best < 2.0, "{d:?}: {best:.3} ms per 10 KB");
    }
    // Fix round 1, C2: a unit whose bytes the lexing modes read differently is classified once
    // per reading (PostgreSQL 2, MySQL/MariaDB up to 4). Printed for the report; bounded by
    // the readings × the interim 2 ms.
    let marked = unit.replacen("'İstanbul'", "'İst\\anbul' AND c.note = \"x\"", 1);
    // Round 2: known modes read once; unknown modes lex the other readings and classify only
    // those that cut other tokens (here MySQL's `ANSI_QUOTES`: `"x"` becomes an identifier).
    for (d, readings) in [
        (DialectId::Postgres, 2.0),
        (DialectId::MySql, 4.0),
        (DialectId::MariaDb, 4.0),
        (DialectId::Sqlite, 1.0),
    ] {
        for (path, ctx, bound) in [
            (
                "known modes",
                single(d).with_known_modes(LexModes::default_for(d)),
                2.0,
            ),
            ("unknown modes", single(d), 2.0 * readings),
        ] {
            let (best, p50, _, rounds) = measure(&marked, &ctx, RUNS / 4, bound);
            eprintln!(
                "{d:?} with a backslash and a double quote, {path}: classify best {best:.3}, p50 {p50:.3} ms per 10 KB, {rounds} round(s)"
            );
            assert!(best < bound, "{d:?} {path}: {best:.3} ms per 10 KB");
        }
    }
}

/// Load-tolerant timing (round 3): up to three rounds of `runs` classifications of `text`,
/// stopping once a round's best is under `bound` ms per 10 KB. Answers the best over the
/// rounds (the classifier's own cost on a busy machine), the last round's p50 and p95, and the
/// rounds taken.
fn measure(text: &str, ctx: &ClassifyCtx, runs: usize, bound: f64) -> (f64, f64, f64, usize) {
    let per = |t: f64| t / (text.len() as f64 / 10_240.0) * 1e3;
    let mut best = f64::INFINITY;
    let (mut p50, mut p95) = (0.0, 0.0);
    for round in 1..=3 {
        let mut times: Vec<f64> = (0..runs)
            .map(|_| {
                let start = Instant::now();
                std::hint::black_box(classify(text, ctx));
                start.elapsed().as_secs_f64()
            })
            .collect();
        times.sort_by(f64::total_cmp);
        best = best.min(per(times[0]));
        p50 = per(times[runs / 2]);
        p95 = per(times[runs * 95 / 100]);
        if best < bound {
            return (best, p50, p95, round);
        }
    }
    (best, p50, p95, 3)
}

// ## m3b-t24-exec-ui (fix round 2, R1-C1): which units' switches were carried.
#[test]
fn carried_switches_names_the_units_the_later_ones_depend_on() {
    let units = ["USE app", "SELECT * FROM session", "USE shop", "SELECT 1"];
    let reports = classify_units(&units, &single(DialectId::MySql));
    assert_eq!(sql_safety::carried_switches(&units, &reports), [0, 2]);
    // After a change that is not read exactly, nothing later is carried (the rest is
    // `Unknown`).
    let units = ["SET @@session.sql_mode = 'ANSI'", "USE app", "SELECT 1"];
    let reports = classify_units(&units, &single(DialectId::MySql));
    assert!(sql_safety::carried_switches(&units, &reports).is_empty());
    // PostgreSQL: a path without `pg_catalog` first is carried; one with it first is the
    // harmless `SET` (no context change, nothing carried).
    let units = [
        "SET search_path TO app, pg_catalog",
        "SELECT lower('a') FROM t",
    ];
    let reports = classify_units(&units, &pg_single());
    assert_eq!(sql_safety::carried_switches(&units, &reports), [0]);
    let units = ["SET search_path TO app", "SELECT lower('a') FROM t"];
    let reports = classify_units(&units, &pg_single());
    assert!(sql_safety::carried_switches(&units, &reports).is_empty());
}
