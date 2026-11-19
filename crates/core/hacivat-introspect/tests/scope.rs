//! Schema scope grammar and matcher (M4 Task 12; alg/02 HV-INTRO-02 test cases 1–8 and 10,
//! edge cases 1–13; MSSQL rows excluded per R-M4-4). Test 10 (the round trip) is
//! `scope_props.rs`.

use std::collections::BTreeSet;
use std::sync::Arc;

use hacivat_introspect::scope::{
    CurrentCtx, NsInfo, ScopeErrorKind, ScopeWarning, ScopeWarningKind, Term, parse_scope,
};
use hacivat_meta::name::{IdentCtx, Name};
use hacivat_meta::{NsKey, NsKind};
use sql_dialect::DialectId;

const PG: DialectId = DialectId::Postgres;
const MY: DialectId = DialectId::MySql;

fn n(s: &str) -> Name {
    Arc::from(s)
}

/// PostgreSQL schemas from `"db.schema"`; a schema named `pg_catalog` or
/// `information_schema` is a system namespace.
fn pg(dotted: &[&str]) -> Vec<NsInfo> {
    dotted
        .iter()
        .map(|d| {
            let (db, s) = d.split_once('.').expect("db.schema");
            NsInfo {
                key: NsKey::schema([db, s]),
                system: matches!(s, "pg_catalog" | "information_schema"),
            }
        })
        .collect()
}

/// One-level namespaces (MySQL databases, SQLite schemas).
fn one(names: &[&str]) -> Vec<NsInfo> {
    names
        .iter()
        .map(|s| NsInfo {
            key: NsKey::schema([*s]),
            system: matches!(*s, "mysql" | "information_schema" | "sys"),
        })
        .collect()
}

fn ctx(db: &str, schema: &str) -> CurrentCtx {
    CurrentCtx {
        database: Some(n(db)),
        schema: Some(n(schema)),
        ident: None,
    }
}

fn keys(dotted: &[&str]) -> BTreeSet<NsKey> {
    dotted.iter().map(|d| NsKey::schema(d.split('.'))).collect()
}

fn select(p: &str, d: DialectId, infos: &[NsInfo], c: &CurrentCtx) -> BTreeSet<NsKey> {
    parse_scope(p, d)
        .unwrap_or_else(|e| panic!("{p:?} parses: {e:?}"))
        .select(infos, c)
}

fn cluster() -> Vec<NsInfo> {
    pg(&[
        "db.public",
        "db.app",
        "db.dbo",
        "db.pg_catalog",
        "db1.s1",
        "db1.s2",
        "db1.s3",
        "db1.s4",
        "db1.dbo",
        "t1.public",
        "t1.other",
        "at1.public",
    ])
}

// ---- test cases ------------------------------------------------------------------------------

#[test]
fn all_all_selects_every_pair() {
    // Test 1. System schemas come from the snapshots, so `*` skips them (R-M4-14).
    let got = select("*:*", PG, &cluster(), &ctx("db", "public"));
    let mut want: Vec<&str> = vec![
        "db.public",
        "db.app",
        "db.dbo",
        "db1.s1",
        "db1.s2",
        "db1.s3",
        "db1.s4",
        "db1.dbo",
        "t1.public",
        "t1.other",
        "at1.public",
    ];
    assert_eq!(got, keys(&want));
    // A system schema is selected only by its name.
    want.push("db.pg_catalog");
    assert_eq!(
        select("*:*|db:pg_catalog", PG, &cluster(), &ctx("db", "public")),
        keys(&want)
    );
}

#[test]
fn current_current_selects_only_current() {
    // Test 2.
    assert_eq!(
        select("@:@", PG, &cluster(), &ctx("db", "app")),
        keys(&["db.app"])
    );
    // The empty pattern keeps `@:@` too.
    assert_eq!(
        select("", PG, &cluster(), &ctx("db", "app")),
        keys(&["db.app"])
    );
    assert_eq!(
        select("  ", PG, &cluster(), &ctx("db", "app")),
        keys(&["db.app"])
    );
}

#[test]
fn union_of_three_clauses() {
    // Test 3.
    assert_eq!(
        select(
            "*:dbo|@:@|db1:s1,s2,s3",
            PG,
            &cluster(),
            &ctx("db", "public")
        ),
        keys(&[
            "db.dbo",
            "db1.dbo",
            "db.public",
            "db1.s1",
            "db1.s2",
            "db1.s3"
        ])
    );
    // Whitespace around separators is ignored; the canonical text has none.
    let p = parse_scope(" *:dbo | @:@ | db1: s1 , s2,s3 ", PG).unwrap();
    assert_eq!(p.to_text(), "*:dbo|@:@|db1:s1,s2,s3");
}

#[test]
fn regex_is_full_match() {
    // Test 4: the default schema of every database whose name fully matches `t.*`.
    assert_eq!(
        select("{t.*}:@", PG, &cluster(), &ctx("db", "public")),
        keys(&["t1.public", "db.public"])
    );
    // `at1` contains a match but does not fully match.
    let explicit = parse_scope("{t.*}:@", PG)
        .unwrap()
        .select_explicit(&cluster(), &ctx("db", "public"));
    assert_eq!(explicit, keys(&["t1.public"]));
}

#[test]
fn negative_clause_removes_tmp_schemas() {
    // Test 5.
    let infos = pg(&[
        "db.public",
        "db.tmp_1",
        "db.tmp_x",
        "db.tmpx",
        "db.app_tmp_1",
    ]);
    assert_eq!(
        select("*:*|!*:{tmp_.*}", PG, &infos, &ctx("db", "public")),
        keys(&["db.public", "db.tmpx", "db.app_tmp_1"])
    );
    // Order-independent; a duplicate clause is idempotent (edge 12).
    assert_eq!(
        select("!*:{tmp_.*}|*:*|*:*", PG, &infos, &ctx("db", "public")),
        keys(&["db.public", "db.tmpx", "db.app_tmp_1"])
    );
}

#[test]
fn unclosed_brace_errors_at_column_3() {
    // Test 6.
    let e = parse_scope("db{1:x", PG).unwrap_err();
    assert_eq!(e.column, 3);
    assert_eq!(e.kind, ScopeErrorKind::UnclosedBrace);
}

#[test]
fn quoted_names_with_separators_round_trip() {
    // Test 7 (and edge 1): `:`, `,`, `|`, `{`, `!`, spaces and quotes inside quotes.
    let p = parse_scope(r#""a:b":s"#, PG).unwrap();
    let c = &p.clauses()[0];
    assert!(matches!(&c.db, Some(Term::Name(x)) if &*x.text == "a:b" && x.quoted));
    assert!(matches!(&c.schemas[..], [Term::Name(x)] if &*x.text == "s" && !x.quoted));
    assert_eq!(p.to_text(), r#""a:b":s"#);
    let infos = pg(&["a:b.s", "a.b", "a:b.t"]);
    assert_eq!(
        p.select_explicit(&infos, &ctx("x", "y")),
        BTreeSet::from([NsKey::schema(["a:b", "s"])])
    );
    let hostile = r#""a,b|c{d}!e f""g":"x""y""#;
    let p = parse_scope(hostile, PG).unwrap();
    assert_eq!(p.to_text(), hostile);
    let c = &p.clauses()[0];
    assert!(matches!(&c.db, Some(Term::Name(x)) if &*x.text == r#"a,b|c{d}!e f"g"#));
    assert!(matches!(&c.schemas[..], [Term::Name(x)] if &*x.text == r#"x"y"#));
    assert_eq!(parse_scope(p.to_text(), PG).unwrap(), p);
}

#[test]
fn mysql_regex_tenants() {
    // Test 8.
    let infos = one(&["tenant_1", "tenant_x", "tenant_22", "shop"]);
    assert_eq!(
        select(r"{tenant_\d+}", MY, &infos, &CurrentCtx::default()),
        keys(&["tenant_1", "tenant_22"])
    );
}

// ---- edge cases -------------------------------------------------------------------------------

#[test]
fn regex_keeps_bars_and_commas_inside_braces() {
    // Edge 2.
    let p = parse_scope("db:{a|b,c},d", PG).unwrap();
    assert_eq!(p.clauses().len(), 1);
    assert_eq!(p.clauses()[0].schemas.len(), 2);
    let infos = pg(&["db.a", "db.b,c", "db.d", "db.b"]);
    assert_eq!(
        p.select_explicit(&infos, &ctx("db", "x")),
        keys(&["db.a", "db.b,c", "db.d"])
    );
}

#[test]
fn invalid_regex_errors_at_its_brace() {
    // Edge 4 (and 3 at another place).
    let e = parse_scope("*:{(}", PG).unwrap_err();
    assert_eq!((e.column, e.kind), (3, ScopeErrorKind::BadRegex));
    let e = parse_scope("*:x|*:{a", PG).unwrap_err();
    assert_eq!((e.column, e.kind), (7, ScopeErrorKind::UnclosedBrace));
    // A regex cannot escape its anchors (`a)|(b` alone is unbalanced).
    let e = parse_scope("{a)|(b}", MY).unwrap_err();
    assert_eq!(e.kind, ScopeErrorKind::BadRegex);
}

#[test]
fn error_columns_count_characters_not_bytes() {
    // `ş` and `İ` are two bytes each; the error is at character 6, byte 8.
    let e = parse_scope("şİ:a,{", PG).unwrap_err();
    assert_eq!((e.column, e.kind), (6, ScopeErrorKind::UnclosedBrace));
    let e = parse_scope("ğü:x y", PG).unwrap_err();
    assert_eq!((e.column, e.kind), (6, ScopeErrorKind::Syntax));
    let e = parse_scope("😀:\"ab", PG).unwrap_err();
    assert_eq!((e.column, e.kind), (3, ScopeErrorKind::Syntax));
}

#[test]
fn syntax_errors_point_at_the_offending_character() {
    for (text, column) in [
        ("a|", 3),
        ("|a", 1),
        ("a::b", 3),
        ("app_*", 5),
        ("a:b,", 5),
        ("a b", 3),
        ("}", 1),
        ("a:\"\"", 3),
        ("!", 2),
        ("a!b", 2),
    ] {
        let e = parse_scope(text, PG).unwrap_err();
        assert_eq!(
            (e.column, e.kind),
            (column, ScopeErrorKind::Syntax),
            "{text:?}"
        );
    }
    // A comma list is a database:schema list on PostgreSQL.
    assert_eq!(parse_scope("a,b", PG).unwrap_err().column, 2);
    assert!(parse_scope("a,b", MY).is_ok());
}

#[test]
fn current_unknown_selects_nothing_for_that_term() {
    // Edge 5.
    let p = parse_scope("@:*|db1:s1", PG).unwrap();
    assert!(p.mentions_current());
    let none = CurrentCtx::default();
    assert_eq!(p.select(&cluster(), &none), keys(&["db1.s1"]));
    assert!(!parse_scope("db1:s1", PG).unwrap().mentions_current());
    assert!(parse_scope("db1", PG).unwrap().mentions_current());
}

#[test]
fn pg_quoted_mixed_case_schema_differs_from_bare() {
    // Edge 6: bare `Sales` folds to `sales`; quoted `"Sales"` is exact.
    let infos = pg(&["db.Sales", "db.sales", "db.SALES"]);
    let c = ctx("db", "x");
    assert_eq!(select("db:\"Sales\"", PG, &infos, &c), keys(&["db.Sales"]));
    assert_eq!(
        parse_scope("db:Sales", PG)
            .unwrap()
            .select_explicit(&infos, &c),
        keys(&["db.sales"])
    );
    // A regex on PostgreSQL is case-sensitive (the name class is exact).
    assert_eq!(
        parse_scope("db:{sales}", PG)
            .unwrap()
            .select_explicit(&infos, &c),
        keys(&["db.sales"])
    );
}

#[test]
fn mysql_lctn_decides_name_and_regex_case() {
    // Edge 8, with HV-INTRO-14's ASCII-only rule (R-M4-41): never Unicode or Turkish folding.
    let infos = one(&["Shop", "shop", "İtem", "item"]);
    let lctn = |l: u8| CurrentCtx {
        ident: Some(IdentCtx {
            lower_case_table_names: l,
            ..IdentCtx::new(MY)
        }),
        ..CurrentCtx::default()
    };
    let explicit = |p: &str, c: &CurrentCtx| parse_scope(p, MY).unwrap().select_explicit(&infos, c);
    assert_eq!(explicit("shop", &lctn(0)), keys(&["shop"]));
    assert_eq!(explicit("shop", &lctn(1)), keys(&["Shop", "shop"]));
    assert_eq!(explicit("{SH.P}", &lctn(0)), keys(&[]));
    assert_eq!(explicit("{SH.P}", &lctn(2)), keys(&["Shop", "shop"]));
    // A non-ASCII cased letter is not pinned under a caseless rule: exact text only.
    assert_eq!(explicit("İtem", &lctn(1)), keys(&["İtem"]));
    assert_eq!(explicit("item", &lctn(1)), keys(&["item"]));
    assert_eq!(explicit("{ıtem}", &lctn(1)), keys(&[]));
}

#[test]
fn sqlite_names_are_ascii_caseless() {
    let infos = one(&["main", "temp", "Aux"]);
    let none = CurrentCtx::default();
    let p = |s: &str| {
        parse_scope(s, DialectId::Sqlite)
            .unwrap()
            .select_explicit(&infos, &none)
    };
    assert_eq!(p("MAIN,aux"), keys(&["main", "Aux"]));
    assert_eq!(p("{A.X}"), keys(&["Aux"]));
}

#[test]
fn negative_on_current_schema_keeps_it_resolvable() {
    // Edge 9: still resolvable (in `select`), not shown (`select_explicit`).
    let p = parse_scope("*:*|!@:@", PG).unwrap();
    let c = ctx("db", "public");
    assert!(
        p.select(&cluster(), &c)
            .contains(&NsKey::schema(["db", "public"]))
    );
    assert!(
        !p.select_explicit(&cluster(), &c)
            .contains(&NsKey::schema(["db", "public"]))
    );
}

#[test]
fn single_level_dialect_ignores_db_term_with_warning() {
    // Edge 11.
    let infos = one(&["orders", "shop", "billing"]);
    let p = parse_scope("shop:orders,billing", MY).unwrap();
    assert_eq!(
        p.warnings(),
        &[ScopeWarning {
            column: 1,
            kind: ScopeWarningKind::DbTermIgnored
        }]
    );
    assert_eq!(
        p.select_explicit(&infos, &CurrentCtx::default()),
        keys(&["orders", "billing"])
    );
    assert_eq!(p.to_text(), "shop:orders,billing");
    // `*:` is DataGrip's own form: no warning.
    let p = parse_scope("*:orders|x|!{z}:billing", MY).unwrap();
    assert_eq!(
        p.warnings(),
        &[ScopeWarning {
            column: 13,
            kind: ScopeWarningKind::DbTermIgnored
        }]
    );
    assert_eq!(
        p.select_explicit(&infos, &CurrentCtx::default()),
        keys(&["orders"])
    );
}

#[test]
fn database_only_clause_is_the_database_and_its_default_schema() {
    let mut infos = cluster();
    infos.push(NsInfo {
        key: NsKey::database("db1"),
        system: false,
    });
    infos.push(NsInfo {
        key: NsKey::database("t1"),
        system: false,
    });
    let c = ctx("db", "s1");
    let got = parse_scope("db1", PG).unwrap().select_explicit(&infos, &c);
    assert_eq!(
        got,
        BTreeSet::from([NsKey::schema(["db1", "s1"]), NsKey::database("db1")])
    );
    // A database node comes with any of its selected schemas.
    let got = parse_scope("t1:other", PG)
        .unwrap()
        .select_explicit(&infos, &c);
    assert_eq!(
        got,
        BTreeSet::from([NsKey::schema(["t1", "other"]), NsKey::database("t1")])
    );
    assert!(got.iter().all(|k| k.kind != NsKind::Server));
}

#[test]
fn a_database_that_no_longer_exists_matches_nothing() {
    // Edge 13.
    assert_eq!(
        select("gone:*|gone", PG, &cluster(), &CurrentCtx::default()),
        keys(&[])
    );
}

#[test]
fn many_databases_select_lazily_by_list() {
    // Edge 10's scale: 2,000 databases × 3 schemas, one pass over the list.
    let dotted: Vec<String> = (0..2000)
        .flat_map(|d| ["public", "app", "pg_catalog"].map(move |s| format!("d{d}.{s}")))
        .collect();
    let refs: Vec<&str> = dotted.iter().map(String::as_str).collect();
    let infos = pg(&refs);
    let got = select("*:*|!{d1.*}:app", PG, &infos, &CurrentCtx::default());
    let removed = (0..2000).filter(|d| d.to_string().starts_with('1')).count();
    assert_eq!(got.len(), 2000 * 2 - removed);
}

#[test]
fn matching_is_locale_free() {
    // HV-INTRO-14: the process locale never enters (run under `LC_ALL=tr_TR.UTF-8` too).
    let infos = one(&["ITEMS", "items", "İTEMS"]);
    let c = CurrentCtx {
        ident: Some(IdentCtx {
            lower_case_table_names: 1,
            ..IdentCtx::new(MY)
        }),
        ..CurrentCtx::default()
    };
    assert_eq!(
        parse_scope("items", MY)
            .unwrap()
            .select_explicit(&infos, &c),
        keys(&["ITEMS", "items"])
    );
}
