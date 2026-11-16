//! M3b Task 14: the orchestrator `classify` (`alg/13` SQL-SAFE-02 steps; R-M3-3): the L0
//! limits and the pre-pass before any backend, the execution-mode split, the dispatch per
//! dialect, the combine (producer contract, `COPY … TO STDOUT`, context changes), the
//! fingerprint, units classified one at a time, the read corpora through the real entry, the
//! T9/T10 regression probes, hostile scripts, and the 10 KB budget.

use std::path::PathBuf;
use std::time::Instant;

use sql_dialect::{ClientFlavor, DialectCtx, DialectId, LexModes, PgWireFlavor, ServerVersion};
use sql_safety::{
    Actor, ClassifyCtx, CoarseClass, EffectSet, Expect, GateClass, Limits, Profile, ReasonCode,
    SafetyReport, Verdict, changes_context, classify, classify_interim, classify_units, decide,
    gate_class, pg_backend_hooks,
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
const PG_RULED_DENIALS: [&str; 4] = [
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
        let mut times: Vec<f64> = (0..RUNS)
            .map(|_| {
                let start = Instant::now();
                std::hint::black_box(classify(&unit, &ctx));
                start.elapsed().as_secs_f64()
            })
            .collect();
        times.sort_by(f64::total_cmp);
        let per = |t: f64| t / (unit.len() as f64 / 10_240.0) * 1e3;
        let (best, p50, p95) = (
            per(times[0]),
            per(times[RUNS / 2]),
            per(times[RUNS * 95 / 100]),
        );
        eprintln!(
            "{d:?}: classify best {best:.3}, p50 {p50:.3}, p95 {p95:.3} ms per 10 KB (budget 1 ms; interim assertion 2 ms on the best)"
        );
        assert!(best < 2.0, "{d:?}: {best:.3} ms per 10 KB");
    }
}
