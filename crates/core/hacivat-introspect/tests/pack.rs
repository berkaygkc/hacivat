//! Catalog query packs (M4 Task 4; alg/02 HV-INTRO-04 test cases 1–6 and edge cases 1–8,
//! HV-INTRO-14 rules 1–2, R-M4-3, R-M4-7, R-M4-8, R-M4-29, R-M4-32, R-M4-37).

use std::collections::BTreeSet;
use std::sync::Arc;

use hacivat_introspect::pack::{
    DraftPack, DraftQuery, LintKind, MAX_IN_LIST, Mode, Pack, PackCtx, PackError, PackErrorKind,
    PackFlags, Params, RenderError, lint,
};
use hacivat_meta::Level;
use hacivat_proto::DbmsKind;
use hacivat_values::{BindValue, LogicalType};
use proptest::prelude::*;
use sql_dialect::DialectId;

// ---------------------------------------------------------------- helpers

fn text(s: &str) -> BindValue {
    BindValue::Typed {
        hint: LogicalType::Text {
            max_chars: None,
            fixed: false,
        },
        text: Arc::from(s),
    }
}

fn flavor_of(d: DialectId) -> DbmsKind {
    match d {
        DialectId::Postgres => DbmsKind::Postgres,
        DialectId::MariaDb => DbmsKind::MariaDb,
        DialectId::Sqlite => DbmsKind::Sqlite,
        _ => DbmsKind::MySql,
    }
}

fn ctx(d: DialectId, version: u32) -> PackCtx {
    PackCtx {
        dialect: d,
        version,
        flavor: flavor_of(d),
        mode: Mode::Full,
        level: Level::Structure,
        flags: PackFlags::empty(),
        params: Params::default(),
    }
}

fn ctx_pg(version: u32) -> PackCtx {
    ctx(DialectId::Postgres, version)
}

fn parse(d: DialectId, text: &str) -> DraftPack {
    DraftPack::parse(d, text).unwrap_or_else(|e| panic!("fixture parses: {e:?}"))
}

fn render(pack: &str, section: &str, c: PackCtx) -> Vec<DraftQuery> {
    parse(c.dialect, pack)
        .render(section, &c)
        .unwrap_or_else(|e| panic!("{section} renders: {e:?}"))
}

fn parse_err(d: DialectId, text: &str) -> PackError {
    DraftPack::parse(d, text).expect_err("the fixture must not parse")
}

const VERSION_GATED: &str = "\
-- header comment, ignored
--- name: S ---
--- returns: oid u32, name text ---
SELECT c.oid, c.relname
--# if version >= 150000
  -- new-branch-marker
  , c.relispartition
--# else
  -- old-branch-marker
  , false
--# end
FROM pg_catalog.pg_class c
";

const LIST_SECTION: &str = "\
--- name: L ---
--- returns: name text ---
SELECT t.name FROM t WHERE t.kind = :kind AND t.name IN :[names]
";

fn render_list(d: DialectId, names: &[&str]) -> Vec<DraftQuery> {
    let mut c = ctx(d, 0);
    c.params.scalars.push((Arc::from("kind"), text("r")));
    c.params
        .lists
        .push((Arc::from("names"), names.iter().map(|n| text(n)).collect()));
    render(LIST_SECTION, "L", c)
}

fn ids(n: usize) -> Vec<String> {
    (0..n).map(|i| format!("{i}")).collect()
}

const SHOW_CREATE: &str = "\
--- name: ShowCreateTable ---
--- returns: name text, ddl text ---
--- show-create ---
SHOW CREATE TABLE #{ident:schema}.#{ident:table}
";

fn try_render_show_create(table: &str) -> Result<Vec<DraftQuery>, RenderError> {
    let mut c = ctx(DialectId::MySql, 80046);
    c.params
        .scalars
        .push((Arc::from("schema"), text("hvm4_db")));
    c.params.scalars.push((Arc::from("table"), text(table)));
    parse(DialectId::MySql, SHOW_CREATE).render("ShowCreateTable", &c)
}

fn render_show_create(table: &str) -> DraftQuery {
    let mut q = try_render_show_create(table).expect("renders");
    assert_eq!(q.len(), 1);
    q.remove(0)
}

fn lint_kinds(d: DialectId, text: &str) -> Vec<LintKind> {
    lint(&parse(d, text), d)
        .into_iter()
        .map(|e| e.kind)
        .collect()
}

/// A one-section pack whose body is `body` (returns declared).
fn section(body: &str) -> String {
    format!("--- name: S ---\n--- returns: x text ---\n{body}\n")
}

// ---------------------------------------------------------------- directives (HV-INTRO-04 1, 2, 6)

#[test]
fn version_gate_picks_else_on_older() {
    let q = render(VERSION_GATED, "S", ctx_pg(140005));
    assert_eq!(q.len(), 1);
    assert!(
        q[0].sql().contains("-- old-branch-marker"),
        "{}",
        q[0].sql()
    );
    assert!(!q[0].sql().contains("new-branch-marker"));
    let q = render(VERSION_GATED, "S", ctx_pg(150000));
    assert!(q[0].sql().contains("-- new-branch-marker"));
    assert!(q[0].sql().contains("c.relispartition"));
    assert!(!q[0].sql().contains("old-branch-marker"));
    // Directive lines never reach the server.
    assert!(!q[0].sql().contains("--#"));
    assert!(!q[0].sql().contains("---"));
}

#[test]
fn incremental_lines_removed_in_full_mode() {
    let pack = section(
        "SELECT c.relname AS x FROM pg_catalog.pg_class c WHERE c.relnamespace = :ns\n\
         --# if mode == incremental\n\
         AND pg_catalog.age(c.xmin) BETWEEN 0 AND pg_catalog.age(:w::pg_catalog.xid)\n\
         --# end",
    );
    let mut c = ctx_pg(170000);
    c.params.scalars.push((Arc::from("ns"), text("2200")));
    c.params.scalars.push((Arc::from("w"), text("750")));
    let full = render(&pack, "S", c.clone());
    assert!(!full[0].sql().contains("age("), "{}", full[0].sql());
    assert_eq!(full[0].binds().len(), 1);
    c.mode = Mode::Incremental;
    let inc = render(&pack, "S", c);
    assert!(
        inc[0]
            .sql()
            .contains("BETWEEN 0 AND pg_catalog.age($2::pg_catalog.xid)")
    );
    assert_eq!(inc[0].binds(), &[text("2200"), text("750")]);
}

#[test]
fn unbalanced_if_is_parse_error_with_line() {
    const UNBALANCED: &str = "\
--- name: S ---
--- returns: x text ---
SELECT 1 AS x
--# if version >= 150000
, 2 AS y
";
    assert!(matches!(
        DraftPack::parse(DialectId::Postgres, UNBALANCED),
        Err(PackError { line: 4, .. })
    ));
    let e = parse_err(DialectId::Postgres, UNBALANCED);
    assert_eq!(e.kind, PackErrorKind::UnclosedIf);
    // The other unbalanced shapes, each at its own line.
    for (body, line) in [
        ("SELECT 1 AS x\n--# end", 4),
        ("SELECT 1 AS x\n--# else", 4),
        ("SELECT 1 AS x\n--# elif mode == full", 4),
        (
            "--# if mode == full\nSELECT 1 AS x\n--# else\nSELECT 2 AS x\n--# else\n--# end",
            7,
        ),
        (
            "--# if mode == full\nSELECT 1 AS x\n--# else\nSELECT 2 AS x\n--# elif mode == brief\n--# end",
            7,
        ),
    ] {
        let e = parse_err(DialectId::Postgres, &section(body));
        assert_eq!(e.line, line, "{body}");
        assert_eq!(e.kind, PackErrorKind::UnbalancedBlock, "{body}");
    }
    // A section that starts while an `if` is open reports the open `if`.
    let e = parse_err(
        DialectId::Postgres,
        &format!(
            "{}--- name: T ---\n",
            section("--# if mode == full\nSELECT 1 AS x")
        ),
    );
    assert_eq!((e.line, e.kind), (3, PackErrorKind::UnclosedIf));
}

#[test]
fn nesting_deeper_than_4_is_error() {
    let nest = |depth: usize| {
        let mut body = String::new();
        for _ in 0..depth {
            body.push_str("--# if mode == full\n");
        }
        body.push_str("SELECT 1 AS x\n");
        for _ in 0..depth {
            body.push_str("--# end\n");
        }
        section(&body)
    };
    let q = render(&nest(4), "S", ctx_pg(170000));
    assert_eq!(q[0].sql(), "SELECT 1 AS x");
    let e = parse_err(DialectId::Postgres, &nest(5));
    assert_eq!((e.line, e.kind), (7, PackErrorKind::NestingTooDeep));
}

#[test]
fn unknown_directives_and_bad_conditions_are_parse_errors() {
    for body in [
        "--# iff mode == full\nSELECT 1 AS x\n--# end",
        "--# if mode = full\nSELECT 1 AS x\n--# end",
        "--# if mode == fragment\nSELECT 1 AS x\n--# end",
        "--# if version >= 15.0\nSELECT 1 AS x\n--# end",
        "--# if version >= 99999999999\nSELECT 1 AS x\n--# end",
        "--# if flavor == db2\nSELECT 1 AS x\n--# end",
        "--# if flag(super)\nSELECT 1 AS x\n--# end",
        "--# if level >= everything\nSELECT 1 AS x\n--# end",
        "--# if (mode == full\nSELECT 1 AS x\n--# end",
        "--# if mode == full and\nSELECT 1 AS x\n--# end",
        "--# if\nSELECT 1 AS x\n--# end",
        "--# if ((((((((((mode == full))))))))))\nSELECT 1 AS x\n--# end",
    ] {
        let e = parse_err(DialectId::Postgres, &section(body));
        assert_eq!(e.line, 3, "{body}");
        assert!(
            matches!(
                e.kind,
                PackErrorKind::BadCondition
                    | PackErrorKind::UnknownDirective
                    | PackErrorKind::MalformedDirective
                    | PackErrorKind::ExprTooDeep
            ),
            "{body}: {:?}",
            e.kind
        );
    }
    let e = parse_err(
        DialectId::Postgres,
        &section("--# if mode == full\nSELECT 1 AS x\n--# end now"),
    );
    assert_eq!((e.line, e.kind), (5, PackErrorKind::MalformedDirective));
    // The condition language works with and/or/not and parentheses.
    let pack = section(
        "SELECT 1 AS x\n\
         --# if not (mode == brief or level < structure) and flavor != mariadb\n\
         , 2 AS y\n\
         --# end",
    );
    let q = render(&pack, "S", ctx_pg(170000));
    assert!(q[0].sql().contains("2 AS y"));
    let mut c = ctx_pg(170000);
    c.level = Level::Names;
    assert!(!render(&pack, "S", c)[0].sql().contains("2 AS y"));
}

#[test]
fn section_header_errors() {
    let cases: [(&str, PackErrorKind, u32); 9] = [
        ("SELECT 1\n", PackErrorKind::TextOutsideSection, 1),
        (
            "--- name: S ---\n--- returns: x text ---\n--- frobnicate ---\nSELECT 1 AS x\n",
            PackErrorKind::UnknownDirective,
            3,
        ),
        (
            "--- name: S ---\n--- returns: x text\nSELECT 1 AS x\n",
            PackErrorKind::MalformedDirective,
            2,
        ),
        (
            "--- name: 1S ---\n--- returns: x text ---\nSELECT 1 AS x\n",
            PackErrorKind::BadSectionName,
            1,
        ),
        (
            "--- name: S ---\nSELECT 1 AS x\n--- returns: x text ---\n",
            PackErrorKind::DirectiveOutOfPlace,
            3,
        ),
        (
            "--- name: S ---\n--- returns: x text ---\n--- joinable ---\n--- joinable ---\nSELECT 1 AS x\n",
            PackErrorKind::DuplicateFlag,
            4,
        ),
        (
            "--- name: S ---\n--- returns: x text, x u32 ---\nSELECT 1 AS x\n",
            PackErrorKind::BadReturns,
            2,
        ),
        (
            "--- name: S ---\n--- returns: x varchar ---\nSELECT 1 AS x\n",
            PackErrorKind::BadReturns,
            2,
        ),
        (
            "--- returns: x text ---\n",
            PackErrorKind::DirectiveOutOfPlace,
            1,
        ),
    ];
    for (text, kind, line) in cases {
        let e = parse_err(DialectId::Postgres, text);
        assert_eq!((e.kind, e.line), (kind, line), "{text}");
    }
}

// ---------------------------------------------------------------- binds (R-M4-3, edge cases 5–7)

#[test]
fn list_bind_expands_to_in_list_on_every_dialect() {
    let q = render_list(DialectId::MySql, &["t1", "t2"]);
    assert!(q[0].sql().contains("IN (?, ?)"), "{}", q[0].sql());
    assert_eq!(q[0].binds().len(), 3);
    let p = render_list(DialectId::Postgres, &["t1", "t2"]);
    assert!(
        p[0].sql().contains("t.kind = $1 AND t.name IN ($2, $3)"),
        "{}",
        p[0].sql()
    );
    assert_eq!(p[0].binds(), &[text("r"), text("t1"), text("t2")]);
    for d in [DialectId::MariaDb, DialectId::Sqlite] {
        let q = render_list(d, &["t1", "t2"]);
        assert!(
            q[0].sql().contains("t.kind = ? AND t.name IN (?, ?)"),
            "{d:?}"
        );
        assert_eq!(q[0].binds(), &[text("r"), text("t1"), text("t2")], "{d:?}");
    }
}

#[test]
fn list_bind_chunks_at_1000() {
    assert_eq!(MAX_IN_LIST, 1000);
    let all = ids(2500);
    let refs: Vec<&str> = all.iter().map(String::as_str).collect();
    let q = render_list(DialectId::Postgres, &refs);
    assert_eq!(q.len(), 3);
    // Every chunk carries the scalar first, then its slice of the list, in order.
    let mut seen = Vec::new();
    for (k, chunk) in q.iter().enumerate() {
        let n = if k < 2 { 1000 } else { 500 };
        assert_eq!(chunk.binds().len(), n + 1, "chunk {k}");
        assert_eq!(chunk.binds()[0], text("r"));
        assert!(chunk.sql().contains(&format!("${})", n + 1)), "chunk {k}");
        assert!(!chunk.sql().contains(&format!("${}", n + 2)), "chunk {k}");
        seen.extend_from_slice(&chunk.binds()[1..]);
    }
    let expect: Vec<BindValue> = refs.iter().map(|s| text(s)).collect();
    assert_eq!(seen, expect, "no item lost, repeated or reordered");
    // The boundaries: 1,000 is one chunk, 1,001 two.
    for (n, chunks, last) in [
        (1, 1, 1),
        (999, 1, 999),
        (1000, 1, 1000),
        (1001, 2, 1),
        (2000, 2, 1000),
        (2001, 3, 1),
    ] {
        let all = ids(n);
        let refs: Vec<&str> = all.iter().map(String::as_str).collect();
        let q = render_list(DialectId::MySql, &refs);
        assert_eq!(q.len(), chunks, "{n}");
        assert_eq!(q.last().unwrap().binds().len(), last + 1, "{n}");
        assert_eq!(
            q.last().unwrap().sql().matches('?').count(),
            last + 1,
            "{n}"
        );
    }
}

#[test]
fn control_characters_in_a_section_line_are_parse_errors() {
    // PostgreSQL ends a `--` comment at a bare CR; the pack lexer must never disagree.
    for body in [
        "SELECT 1 AS x -- c\r, :n AS y",
        "SELECT 1 AS x\u{b}",
        "SELECT 1 AS x\u{c}",
        "SELECT 1\u{0} AS x",
        "SELECT 1\u{7f} AS x",
    ] {
        let e = parse_err(DialectId::Postgres, &section(body));
        assert_eq!(
            (e.kind, e.line),
            (PackErrorKind::ControlChar, 3),
            "{body:?}"
        );
    }
    // A tab is fine, and a CRLF file reads like an LF one.
    let crlf = section("SELECT\t1 AS x").replace('\n', "\r\n");
    assert_eq!(
        render(&crlf, "S", ctx_pg(170000))[0].sql(),
        "SELECT\t1 AS x"
    );
}

#[test]
fn empty_list_skips_section() {
    assert!(render_list(DialectId::Sqlite, &[]).is_empty());
    assert!(render_list(DialectId::Postgres, &[]).is_empty());
    // An empty list never hides a missing scalar: the section is still checked.
    let mut c = ctx(DialectId::Sqlite, 0);
    c.params.lists.push((Arc::from("names"), Vec::new()));
    assert_eq!(
        parse(DialectId::Sqlite, LIST_SECTION).render("L", &c),
        Err(RenderError::MissingBind(Arc::from("kind")))
    );
}

#[test]
fn unknown_placeholder_is_a_render_error() {
    let pack = parse(DialectId::Postgres, LIST_SECTION);
    let c = ctx_pg(170000);
    assert_eq!(
        pack.render("L", &c),
        Err(RenderError::MissingBind(Arc::from("kind")))
    );
    let mut c = ctx_pg(170000);
    c.params.scalars.push((Arc::from("kind"), text("r")));
    assert_eq!(
        pack.render("L", &c),
        Err(RenderError::MissingBind(Arc::from("names")))
    );
    assert_eq!(pack.render("Nope", &c), Err(RenderError::UnknownSection));
    // A scalar given only as a list, or a list only as a scalar, is still missing.
    let mut c = ctx_pg(170000);
    c.params.lists.push((Arc::from("kind"), vec![text("r")]));
    c.params.scalars.push((Arc::from("names"), text("t1")));
    assert_eq!(
        pack.render("L", &c),
        Err(RenderError::MissingBind(Arc::from("kind")))
    );
}

#[test]
fn pg_casts_and_assignments_are_not_binds() {
    let pack = section(
        "SELECT 'pg_catalog.pg_class'::pg_catalog.regclass::pg_catalog.oid AS x, \
         pg_catalog.format_type(a.atttypid, NULL) AS t, :n::pg_catalog.int4 AS y \
         FROM pg_catalog.pg_attribute a WHERE a.attnum = ANY(ARRAY[1, 2]) AND a.attrelid IN :[ids]",
    );
    let mut c = ctx_pg(170000);
    c.params.scalars.push((Arc::from("n"), text("7")));
    c.params
        .lists
        .push((Arc::from("ids"), vec![text("1"), text("2")]));
    let q = render(&pack, "S", c);
    assert!(
        q[0].sql()
            .starts_with("SELECT 'pg_catalog.pg_class'::pg_catalog.regclass::pg_catalog.oid AS x"),
        "{}",
        q[0].sql()
    );
    assert!(q[0].sql().contains("$1::pg_catalog.int4 AS y"));
    assert!(q[0].sql().ends_with("a.attrelid IN ($2, $3)"));
    assert_eq!(q[0].binds().len(), 3);
}

#[test]
fn literals_and_comments_are_opaque_to_binds() {
    let pack =
        section("SELECT ':notabind' AS x, \"col:also\" AS y /* :nor_this */ FROM t -- :or_this");
    let q = render(&pack, "S", ctx_pg(170000));
    assert!(q[0].binds().is_empty());
    assert_eq!(
        q[0].sql(),
        "SELECT ':notabind' AS x, \"col:also\" AS y /* :nor_this */ FROM t -- :or_this"
    );
}

#[test]
fn raw_placeholders_and_statement_breaks_are_parse_errors() {
    use PackErrorKind as K;
    let cases: &[(DialectId, &str, PackErrorKind)] = &[
        (DialectId::Postgres, "SELECT $1 AS x", K::RawPlaceholder),
        (DialectId::Postgres, "SELECT $$x$$ AS x", K::RawPlaceholder),
        (DialectId::MySql, "SELECT ? AS x", K::RawPlaceholder),
        (DialectId::Sqlite, "SELECT ? AS x", K::RawPlaceholder),
        (DialectId::Sqlite, "SELECT @x AS x", K::RawPlaceholder),
        (DialectId::Sqlite, "SELECT $x AS x", K::RawPlaceholder),
        (
            DialectId::Sqlite,
            "SELECT [x] AS x FROM t",
            K::BracketIdentifier,
        ),
        (
            DialectId::Postgres,
            "SELECT 1 AS x; DROP TABLE t",
            K::Semicolon,
        ),
        (DialectId::MySql, "SELECT 1 AS x # trailing", K::HashComment),
        (DialectId::MySql, "SELECT 1--1 AS x", K::AmbiguousDashDash),
        (
            DialectId::Postgres,
            "SELECT 'a AS x",
            K::UnterminatedLiteral,
        ),
        (
            DialectId::Postgres,
            "SELECT \"a AS x",
            K::UnterminatedLiteral,
        ),
        (DialectId::MySql, "SELECT `a AS x", K::UnterminatedLiteral),
        (
            DialectId::Postgres,
            "SELECT 1 AS x /* open",
            K::UnterminatedLiteral,
        ),
        (
            DialectId::Postgres,
            "SELECT 1 AS x /* a /* b */ */",
            K::NestedComment,
        ),
        (
            DialectId::Postgres,
            "SELECT E'a\\'b' AS x",
            K::BackslashInLiteral,
        ),
        (DialectId::MySql, "SELECT 'a\\' AS x", K::BackslashInLiteral),
        (
            DialectId::Postgres,
            "SELECT :[ids] AS x",
            K::ListBindNotAfterIn,
        ),
        (
            DialectId::Postgres,
            "SELECT 1 AS x FROM t WHERE t.a NOT IN :[ids]",
            K::NotInListBind,
        ),
        (
            DialectId::Postgres,
            "SELECT 1 AS x FROM t WHERE t.a NOT /* c */ IN :[ids]",
            K::NotInListBind,
        ),
        (
            DialectId::Postgres,
            "SELECT 1 AS x FROM t WHERE t.a IN :[a] AND t.b IN :[b]",
            K::MultipleListBinds,
        ),
        (
            DialectId::Postgres,
            "SELECT 1 AS x FROM t WHERE t.a IN :[1x]",
            K::BadBind,
        ),
        // Fix round 1, I1: every form the server reads differently from the lexer.
        (
            DialectId::Postgres,
            "SELECT 1 AS x WHERE `$1; x` IS NULL",
            K::Backtick,
        ),
        (
            DialectId::Postgres,
            "SELECT 1 AS x /* a /*/",
            K::NestedComment,
        ),
        (
            DialectId::Postgres,
            "SELECT 1 AS x /* a /**/",
            K::NestedComment,
        ),
        (
            DialectId::Sqlite,
            "SELECT 1 AS x WHERE #x IS NULL",
            K::RawPlaceholder,
        ),
        (
            DialectId::Sqlite,
            "SELECT 1 AS x WHERE :1 IS NULL",
            K::RawPlaceholder,
        ),
        (
            DialectId::Sqlite,
            "SELECT 1 AS x WHERE :\u{e9} IS NULL",
            K::RawPlaceholder,
        ),
        (
            DialectId::Postgres,
            "SELECT 1 AS x WHERE :1 IS NULL",
            K::RawPlaceholder,
        ),
        (
            DialectId::MySql,
            "SELECT 1 AS x /*! OR 1 = ? */",
            K::ExecutableComment,
        ),
        (
            DialectId::MariaDb,
            "SELECT 1 AS x /*M!100100 OR 1 */",
            K::ExecutableComment,
        ),
        (
            DialectId::MySql,
            "SELECT 1 AS x /*!80000 1 */",
            K::ExecutableComment,
        ),
    ];
    for (d, body, kind) in cases {
        let e = parse_err(*d, &section(body));
        assert_eq!((e.kind.clone(), e.line), (kind.clone(), 3), "{d:?} {body}");
    }
    // `IN` must sit on the list's own line, after its operand.
    let e = parse_err(
        DialectId::Postgres,
        &section("SELECT 1 AS x FROM t WHERE t.a NOT\nIN :[ids]"),
    );
    assert_eq!((e.kind, e.line), (K::ListBindNotAfterIn, 4));
}

#[test]
fn session_refused_words_are_parse_errors() {
    // T5 carry: no PRAGMA statement in any pack; table-valued functions are fine.
    let e = parse_err(DialectId::Sqlite, &section("PRAGMA table_info(:t)"));
    assert_eq!((e.kind, e.line), (PackErrorKind::PragmaStatement, 3));
    render(
        &section("SELECT name AS x FROM pragma_table_info(:t, :s)"),
        "S",
        {
            let mut c = ctx(DialectId::Sqlite, 0);
            c.params.scalars.push((Arc::from("t"), text("t")));
            c.params.scalars.push((Arc::from("s"), text("main")));
            c
        },
    );
    // A joinable section never holds a write word, not even in a literal or comment.
    let joinable = |body: &str| {
        DraftPack::parse(
            DialectId::Postgres,
            &format!("--- name: S ---\n--- returns: x text ---\n--- joinable ---\n{body}\n"),
        )
    };
    for (body, word) in [
        (
            "SELECT 1 AS x FROM pg_catalog.pg_class c WHERE c.relname = 'insert'",
            "insert",
        ),
        ("SELECT 1 AS x -- UPDATE later", "UPDATE"),
        ("SELECT 1 AS x /* Delete */", "Delete"),
        ("SELECT 1 AS \"merge\"", "merge"),
        ("SELECT 1 AS x INTO t", "INTO"),
    ] {
        assert_eq!(
            joinable(body).unwrap_err().kind,
            PackErrorKind::JoinableWriteWord(word.into()),
            "{body}"
        );
    }
    // The same text in a section that is not joinable, and a word that only contains one.
    DraftPack::parse(
        DialectId::Postgres,
        &section("SELECT 1 AS x FROM pg_catalog.pg_class c WHERE c.relname = 'insert'"),
    )
    .unwrap();
    joinable("SELECT 1 AS update_count, 2 AS into_x").unwrap();
}

#[test]
fn comments_and_backticks_the_lexer_keeps() {
    // Not nested, so allowed: an empty comment, a run of stars, a slash after the opener.
    let q = render(
        &section("SELECT 1 AS x /**/ /***/ /*/ y */ FROM t"),
        "S",
        ctx_pg(170000),
    );
    assert_eq!(q[0].sql(), "SELECT 1 AS x /**/ /***/ /*/ y */ FROM t");
    // MySQL and SQLite quote identifiers with backticks; optimizer hints are comments.
    for d in [DialectId::MySql, DialectId::Sqlite] {
        let q = render(&section("SELECT `a``b` AS x"), "S", ctx(d, 0));
        assert_eq!(q[0].sql(), "SELECT `a``b` AS x", "{d:?}");
    }
    let q = render(
        &section("SELECT /*+ MAX_EXECUTION_TIME(1000) */ 1 AS x"),
        "S",
        ctx(DialectId::MySql, 80046),
    );
    assert!(q[0].sql().contains("/*+"));
}

// ---------------------------------------------------------------- list placement (I3)

/// A section whose WHERE clause is `body`, over `pg_catalog.pg_class c`.
fn list_section(select: &str, where_: &str) -> String {
    section(&format!(
        "SELECT {select} FROM pg_catalog.pg_class c WHERE {where_}"
    ))
}

#[test]
fn list_bind_is_only_a_top_level_where_conjunct() {
    use PackErrorKind as K;
    for (sel, wh, kind) in [
        (
            "c.relname AS x",
            "NOT c.oid IN :[ids]",
            K::ListBindNotConjunct,
        ),
        (
            "c.relname AS x",
            "NOT (c.oid IN :[ids])",
            K::ListBindNotConjunct,
        ),
        (
            "c.relname AS x",
            "(c.oid IN :[ids]) IS NOT TRUE",
            K::ListBindNotConjunct,
        ),
        (
            "c.relname AS x",
            "c.oid IN :[ids] IS NOT TRUE",
            K::ListBindNotConjunct,
        ),
        (
            "c.relname AS x",
            "NOT EXISTS (SELECT 1 FROM pg_catalog.pg_depend d WHERE d.objid = c.oid AND d.refobjid IN :[ids])",
            K::ListBindNotConjunct,
        ),
        (
            "c.relname AS x",
            "c.relkind = 'r' AND (c.relnamespace = :ns AND c.oid IN :[ids])",
            K::ListBindNotConjunct,
        ),
        (
            "c.relname AS x",
            "c.relkind = 'r' OR c.oid IN :[ids]",
            K::ListBindNotConjunct,
        ),
        (
            "c.relname AS x",
            "c.oid IN :[ids] OR c.relkind = 'r'",
            K::ListBindNotConjunct,
        ),
        (
            "c.relname AS x",
            "c.relkind = 'r' OR c.relkind = 'v' AND c.oid IN :[ids]",
            K::ListBindUnchunkable("OR".into()),
        ),
        (
            "count(*) AS x",
            "c.oid IN :[ids]",
            K::ListBindUnchunkable("count".into()),
        ),
        (
            "DISTINCT c.relkind AS x",
            "c.oid IN :[ids]",
            K::ListBindUnchunkable("DISTINCT".into()),
        ),
        (
            "c.relname AS x",
            "c.oid IN :[ids] ORDER BY c.relname LIMIT 10",
            K::ListBindUnchunkable("LIMIT".into()),
        ),
        (
            "c.relname AS x",
            "c.oid IN :[ids] OFFSET 5",
            K::ListBindNotConjunct,
        ),
        (
            "c.relkind AS x",
            "c.oid IN :[ids] GROUP BY c.relkind",
            K::ListBindNotConjunct,
        ),
        (
            "c.relname AS x",
            "c.relpages BETWEEN 1 AND c.oid IN :[ids]",
            K::ListBindNotConjunct,
        ),
        (
            "c.relname AS x",
            "CASE WHEN c.oid IN :[ids] THEN true END",
            K::ListBindNotConjunct,
        ),
        (
            "c.relname AS x, pg_catalog.row_number() OVER () AS n",
            "c.oid IN :[ids]",
            K::ListBindUnchunkable("OVER".into()),
        ),
        (
            "c.relname AS x",
            "c.oid IN :[ids] AND c.relam IN :[ids]",
            K::ListBindReused,
        ),
        ("c.relname AS x", "'a' IN :[ids]", K::ListBindNotConjunct),
    ] {
        let e = parse_err(DialectId::Postgres, &list_section(sel, wh));
        assert_eq!(e.kind, kind, "{sel} … WHERE {wh}");
        assert_eq!(e.line, 3, "{wh}");
    }
    // MySQL: `!` and `||` are NOT and OR there.
    for (wh, kind) in [
        ("!(TABLE_NAME IN :[ids])", K::ListBindNotConjunct),
        (
            "TABLE_SCHEMA = :s || TABLE_NAME IN :[ids]",
            K::ListBindNotConjunct,
        ),
        (
            "TABLE_SCHEMA = :s || TABLE_TYPE = :t AND TABLE_NAME IN :[ids]",
            K::ListBindUnchunkable("||".into()),
        ),
    ] {
        let e = parse_err(
            DialectId::MySql,
            &section(&format!(
                "SELECT TABLE_NAME AS x FROM INFORMATION_SCHEMA.TABLES WHERE {wh}"
            )),
        );
        assert_eq!(e.kind, kind, "{wh}");
    }
    // An ON clause is not the WHERE clause (a chunked outer join repeats its other rows).
    let e = parse_err(
        DialectId::Postgres,
        &section(
            "SELECT c.relname AS x FROM pg_catalog.pg_class c LEFT JOIN pg_catalog.pg_index i ON i.indrelid = c.oid AND i.indexrelid IN :[ids]",
        ),
    );
    assert_eq!(e.kind, K::ListBindNotConjunct);
    // Every legal form: first or later conjunct, qualified or not, ORDER BY after it, an
    // aggregate in a correlated subquery, the list gated in one variant only.
    for body in [
        "SELECT c.relname AS x FROM pg_catalog.pg_class c WHERE c.oid IN :[ids]",
        "SELECT c.relname AS x FROM pg_catalog.pg_class c WHERE c.relkind = 'r' AND NOT c.relispartition AND c.oid IN :[ids] AND c.relnamespace = :ns",
        "SELECT relname AS x FROM pg_catalog.pg_class WHERE oid IN :[ids] ORDER BY relname",
        "SELECT c.relname AS x, (SELECT count(*) FROM pg_catalog.pg_attribute a WHERE a.attrelid = c.oid) AS n FROM pg_catalog.pg_class c WHERE c.oid IN :[ids]",
        "SELECT c.relname AS x FROM pg_catalog.pg_class c WHERE c.relnamespace = :ns\n--# if mode == incremental\nAND c.oid IN :[ids]\n--# end",
        "SELECT c.relname AS x FROM pg_catalog.pg_class c\n--# if mode == incremental\nWHERE c.oid IN :[ids]\n--# else\nWHERE c.relnamespace = :ns\n--# end",
        "SELECT c.relname AS x FROM pg_catalog.pg_class c WHERE pg_catalog.age(c.xmin) BETWEEN 0 AND pg_catalog.age(:w::pg_catalog.xid) AND c.oid IN :[ids]",
    ] {
        DraftPack::parse(DialectId::Postgres, &section(body))
            .unwrap_or_else(|e| panic!("{body}: {e:?}"));
    }
}

#[test]
fn an_empty_list_selects_nothing_because_negation_is_refused() {
    // The legal form: an empty list renders no query, the right answer (no row has an id in
    // the empty set) …
    let ok = list_section("c.relname AS x", "c.relkind = 'r' AND c.oid IN :[ids]");
    let mut c = ctx_pg(170000);
    c.params.lists.push((Arc::from("ids"), Vec::new()));
    assert_eq!(
        parse(DialectId::Postgres, &ok).render("S", &c),
        Ok(Vec::new())
    );
    // … and the forms where "no query" would be wrong (every row matches a negated empty
    // set) never parse.
    for wh in [
        "NOT c.oid IN :[ids]",
        "NOT (c.oid IN :[ids])",
        "c.oid NOT IN :[ids]",
        "(c.oid IN :[ids]) IS NOT TRUE",
        "c.relkind = 'r' OR c.oid IN :[ids]",
    ] {
        assert!(
            DraftPack::parse(DialectId::Postgres, &list_section("c.relname AS x", wh)).is_err(),
            "{wh}"
        );
    }
}

#[test]
fn list_items_are_deduplicated_before_chunking() {
    let mut items: Vec<String> = ids(1001);
    items[1000] = "0".into();
    items.push("7".into());
    let refs: Vec<&str> = items.iter().map(String::as_str).collect();
    let q = render_list(DialectId::Postgres, &refs);
    // 1,002 items, 1,000 distinct: one chunk, each id once, first-seen order.
    assert_eq!(q.len(), 1);
    let expect: Vec<BindValue> = std::iter::once(text("r"))
        .chain((0..1000).map(|i| text(&i.to_string())))
        .collect();
    assert_eq!(q[0].binds(), &expect[..]);
}

// ---------------------------------------------------------------- macros (edge case 6, R-M4-37)

#[test]
fn names_are_binds_never_macros() {
    for m in [
        "#{schema}",
        "#{table_name}",
        "#{name}",
        "#{ident}",
        "#{ident:}",
        "#{ident:a b}",
        "#{ident:a;b}",
        "#{IDENT:t}",
        "#{ max_in_list }",
        "#{catalog_prefix:x}",
        "#{",
    ] {
        let e = parse_err(
            DialectId::MySql,
            &section(&format!("SELECT 1 AS x FROM {m}")),
        );
        assert!(
            matches!(e.kind, PackErrorKind::UnknownMacro),
            "{m}: {:?}",
            e.kind
        );
    }
    // A macro inside a literal or a comment never reaches the renderer.
    for body in [
        "SELECT '#{max_in_list}' AS x",
        "SELECT \"#{max_in_list}\" AS x",
        "SELECT 1 AS x -- #{max_in_list}",
        "SELECT 1 AS x /* #{ident:t} */",
    ] {
        let e = parse_err(DialectId::Postgres, &section(body));
        assert_eq!(e.kind, PackErrorKind::MacroInLiteral, "{body}");
    }
    // The numeric limit is the closed enum's constant.
    let q = render(
        &section("SELECT 1 AS x FROM t LIMIT #{max_in_list}"),
        "S",
        ctx_pg(170000),
    );
    assert_eq!(q[0].sql(), "SELECT 1 AS x FROM t LIMIT 1000");
}

#[test]
fn show_create_ident_macro_quotes_backtick_and_refuses_nul() {
    assert!(render_show_create("a`b").sql().contains("`a``b`"));
    assert!(render_show_create("İ").sql().contains("`İ`"));
    assert!(render_show_create("t ").sql().contains("`t `"));
    assert!(try_render_show_create("a\0b").is_err());
    assert_eq!(
        render_show_create("a`b").sql(),
        "SHOW CREATE TABLE `hvm4_db`.`a``b`"
    );
    // SHOW CREATE takes no binds (R-M4-37): the names are quoted text, nothing else travels.
    assert!(render_show_create("t").binds().is_empty());
    // Hostile names stay one quoted identifier.
    for hostile in [
        "x` FROM t; DROP TABLE t; -- ",
        "`",
        "``",
        "a``b",
        "\u{202E}t",
        "t\nu",
        "?",
        ":x",
        "$1",
        "'",
        "\"",
        "\\",
    ] {
        let q = render_show_create(hostile);
        let sql = q.sql();
        let tail = sql
            .strip_prefix("SHOW CREATE TABLE `hvm4_db`.`")
            .unwrap_or_else(|| panic!("{sql}"));
        let inner = tail.strip_suffix('`').unwrap_or_else(|| panic!("{sql}"));
        assert_eq!(inner.replace("``", "`"), hostile, "{sql}");
        assert!(!inner.replace("``", "").contains('`'), "{sql}");
    }
    assert_eq!(try_render_show_create("a\0b"), Err(RenderError::IdentNul));
    assert_eq!(try_render_show_create(""), Err(RenderError::IdentEmpty));
    // A NULL bind is not a name.
    let mut c = ctx(DialectId::MySql, 80046);
    c.params.scalars.push((Arc::from("schema"), text("db")));
    c.params.scalars.push((Arc::from("table"), BindValue::Null));
    assert_eq!(
        parse(DialectId::MySql, SHOW_CREATE).render("ShowCreateTable", &c),
        Err(RenderError::IdentNotText(Arc::from("table")))
    );
    // MariaDB uses the same pack and the same backtick quoting.
    let mut c = ctx(DialectId::MariaDb, 110402);
    c.params.scalars.push((Arc::from("schema"), text("d\"b")));
    c.params.scalars.push((Arc::from("table"), text("t")));
    let q = parse(DialectId::MariaDb, SHOW_CREATE)
        .render("ShowCreateTable", &c)
        .unwrap();
    assert_eq!(q[0].sql(), "SHOW CREATE TABLE `d\"b`.`t`");
}

#[test]
fn show_create_sections_hold_no_literal_comment_or_bind() {
    for body in [
        "SHOW CREATE TABLE #{ident:t} -- note",
        "SHOW CREATE TABLE #{ident:t} /* note */",
        "SHOW CREATE TABLE `x`.#{ident:t}",
        "SHOW CREATE TABLE #{ident:s}.'t'",
        "SHOW CREATE TABLE #{ident:t} LIKE :pattern",
        "SHOW CREATE TABLE #{ident:s}#{ident:t}",
        "SHOW CREATE TABLE x#{ident:t}",
        "SHOW CREATE TABLE #{ident:t}x",
    ] {
        let text =
            format!("--- name: S ---\n--- returns: x text ---\n--- show-create ---\n{body}\n");
        let e = parse_err(DialectId::MySql, &text);
        assert_eq!(e.kind, PackErrorKind::ShowCreateText, "{body}");
        assert_eq!(e.line, 4, "{body}");
    }
    // A comment line ahead of the flag counts too.
    let e = parse_err(
        DialectId::MySql,
        "--- name: S ---\n-- note\n--- returns: x text ---\n--- show-create ---\nSHOW CREATE TABLE #{ident:t}\n",
    );
    assert_eq!((e.kind, e.line), (PackErrorKind::ShowCreateText, 2));
    // `show-create` is MySQL's (R-M4-37): no other dialect quotes with backticks.
    for d in [DialectId::Postgres, DialectId::Sqlite] {
        let e = parse_err(d, SHOW_CREATE);
        assert_eq!(
            (e.kind, e.line),
            (PackErrorKind::ShowCreateNotMySql, 3),
            "{d:?}"
        );
    }
}

#[test]
fn show_create_macro_outside_show_create_section_is_a_parse_error() {
    // Fix round 1 (m2): a parse error, so a gated-away branch cannot hide it until runtime.
    for body in [
        "SELECT 1 AS x FROM INFORMATION_SCHEMA.TABLES WHERE TABLE_NAME = #{ident:t}",
        "SELECT 1 AS x\n--# if version >= 99999999\nFROM #{ident:t}\n--# end",
    ] {
        let e = parse_err(DialectId::MySql, &section(body));
        assert_eq!(e.kind, PackErrorKind::IdentOutsideShowCreate, "{body}");
    }
}

// ---------------------------------------------------------------- sections, files (R-M4-32)

#[test]
fn every_section_declares_returns() {
    let e = parse_err(
        DialectId::Postgres,
        "--- name: A ---\n--- returns: x text ---\nSELECT 1 AS x\n--- name: B ---\nSELECT 2 AS y\n",
    );
    assert_eq!((e.kind, e.line), (PackErrorKind::MissingReturns, 4));
    let e = parse_err(DialectId::Postgres, "--- name: A ---\nSELECT 1 AS x\n");
    assert_eq!((e.kind, e.line), (PackErrorKind::MissingReturns, 1));
    let pack = parse(DialectId::Postgres, VERSION_GATED);
    let cols: Vec<(&str, &str)> = pack
        .section("S")
        .unwrap()
        .returns()
        .iter()
        .map(|c| (&*c.name, c.ty.as_str()))
        .collect();
    assert_eq!(cols, [("oid", "u32"), ("name", "text")]);
}

const SEC_S: &str = "--- name: S ---\n--- returns: x text ---\nSELECT 1 AS x\n";
const SEC_T: &str = "--- name: T ---\n--- returns: x text ---\nSELECT 2 AS x\n";
const SEC_U: &str = "--- name: U ---\n--- returns: x text ---\nSELECT 3 AS x\n";

#[test]
fn duplicate_section_across_files_is_an_error() {
    assert!(matches!(
        Pack::parse_all(
            DialectId::Postgres,
            &[("10-a.hvq.sql", SEC_S), ("20-b.hvq.sql", SEC_S)]
        ),
        Err(PackError {
            file: "20-b.hvq.sql",
            ..
        })
    ));
    let e = Pack::parse_all(
        DialectId::Postgres,
        &[("20-b.hvq.sql", SEC_S), ("10-a.hvq.sql", SEC_S)],
    )
    .unwrap_err();
    assert_eq!(
        (e.file, e.line, e.kind),
        (
            "20-b.hvq.sql",
            1,
            PackErrorKind::DuplicateSection(Arc::from("S"))
        )
    );
    let e = Pack::parse_all(
        DialectId::Postgres,
        &[("10-a.hvq.sql", SEC_S), ("10-a.hvq.sql", SEC_T)],
    )
    .unwrap_err();
    assert_eq!(e.kind, PackErrorKind::DuplicateFile);
}

#[test]
fn files_merge_in_name_order() {
    let pack = Pack::parse_all(
        DialectId::Postgres,
        &[
            ("70-later.hvq.sql", SEC_U),
            ("10-core.hvq.sql", SEC_S),
            ("40-system.hvq.sql", SEC_T),
        ],
    )
    .unwrap();
    let names: Vec<&str> = pack.sections().iter().map(|s| &**s).collect();
    assert_eq!(names, ["S", "T", "U"]);
    assert_eq!(pack.section("T").unwrap().file(), "40-system.hvq.sql");
    assert_eq!(pack.dialect(), DialectId::Postgres);
    // A section never continues into the next file.
    let e = Pack::parse_all(
        DialectId::Postgres,
        &[
            ("10-a", "--- name: S ---\n"),
            ("20-b", "--- returns: x text ---\nSELECT 1\n"),
        ],
    )
    .unwrap_err();
    assert_eq!(
        (e.file, e.line, e.kind),
        ("10-a", 1, PackErrorKind::MissingReturns)
    );
}

#[test]
fn builtin_packs_parse_and_lint_clean() {
    for d in [
        DialectId::Postgres,
        DialectId::MySql,
        DialectId::MariaDb,
        DialectId::Sqlite,
    ] {
        let pack = Pack::builtin(d).unwrap_or_else(|e| panic!("{d:?}: {e:?}"));
        assert_eq!(pack.dialect(), d);
        assert_eq!(lint(&pack, d), Vec::new(), "{d:?}");
        for s in pack.sections() {
            assert!(!pack.reachable_variants(s).is_empty(), "{d:?} {s}");
        }
    }
    assert_eq!(
        Pack::builtin(DialectId::SqlServer).unwrap_err().kind,
        PackErrorKind::UnsupportedDialect
    );
    assert_eq!(
        Pack::parse(DialectId::Oracle, SEC_S).unwrap_err().kind,
        PackErrorKind::UnsupportedDialect
    );
}

#[test]
fn render_checks_dialect_and_produces_no_sql_for_an_empty_variant() {
    let pack = parse(DialectId::Postgres, SEC_S);
    assert_eq!(
        pack.render("S", &ctx(DialectId::MySql, 80046)),
        Err(RenderError::DialectMismatch)
    );
    // A MySQL pack serves MariaDB contexts (one pack, flavor gates inside).
    let my = parse(DialectId::MySql, SEC_S);
    assert_eq!(
        my.render("S", &ctx(DialectId::MariaDb, 110402))
            .unwrap()
            .len(),
        1
    );
    // Every line gated away: an error, never an empty result that reads as "no objects".
    let gated = section("--# if flavor == mariadb\nSELECT 1 AS x\n--# end");
    assert_eq!(
        parse(DialectId::MySql, &gated).render("S", &ctx(DialectId::MySql, 80046)),
        Err(RenderError::NoVariant)
    );
    let comment_only =
        section("-- nothing to run\n--# if flavor == mariadb\nSELECT 1 AS x\n--# end");
    assert_eq!(
        parse(DialectId::MySql, &comment_only).render("S", &ctx(DialectId::MySql, 80046)),
        Err(RenderError::NoVariant)
    );
}

// ---------------------------------------------------------------- joinable (R-M4-29)

#[test]
fn joinable_flag_reaches_the_meta_query() {
    let src = "\
--- name: J ---
--- returns: x text ---
--- joinable ---
SELECT c.relname AS x FROM pg_catalog.pg_class c WHERE c.oid = :oid
--- name: N ---
--- returns: x text ---
SELECT 1 AS x
";
    let pack = Pack::parse(DialectId::Postgres, src).unwrap();
    let mut c = ctx_pg(170000);
    c.params.scalars.push((Arc::from("oid"), text("1259")));
    let q = pack.render("J", &c).unwrap();
    assert!(q[0].joinable());
    let n = pack.render("N", &c).unwrap();
    assert!(!n[0].joinable());
    assert!(pack.section("J").unwrap().joinable());
    // The `MetaQuery` side exists with the `session` feature only.
    #[cfg(feature = "session")]
    {
        let m = q[0].as_meta_query();
        assert!(m.joinable());
        assert_eq!(m.sql(), q[0].sql());
        assert_eq!(m.binds(), &[text("1259")]);
        assert!(!n[0].as_meta_query().joinable());
    }
    // A joined connection renders only joinable sections.
    c.flags |= PackFlags::JOINED;
    assert!(pack.render("J", &c).unwrap()[0].joinable());
    #[cfg(feature = "session")]
    assert!(pack.render("J", &c).unwrap()[0].as_meta_query().joinable());
    assert_eq!(pack.render("N", &c), Err(RenderError::NotJoinable));
}

// ---------------------------------------------------------------- lint (HV-INTRO-14, R-M4-8)

#[test]
fn directive_text_inside_string_literal_is_refused_by_lint() {
    for body in [
        "SELECT '--# if version >= 1' AS x",
        "SELECT '--- name: T ---' AS x",
        "SELECT 1 AS \"--# end\"",
    ] {
        assert_eq!(
            lint_kinds(DialectId::Postgres, &section(body)),
            [LintKind::DirectiveInLiteral],
            "{body}"
        );
    }
    // A line that only looks like a directive after a literal is not one.
    let q = render(&section("SELECT '--# if' AS x"), "S", ctx_pg(170000));
    assert_eq!(q[0].sql(), "SELECT '--# if' AS x");
}

#[test]
fn canonical_catalog_case_is_linted() {
    use LintKind as L;
    let pg = |b: &str| lint_kinds(DialectId::Postgres, &section(b));
    assert_eq!(pg("SELECT c.relname AS x FROM pg_catalog.pg_class c"), []);
    assert_eq!(
        pg("SELECT c.relname AS x FROM pg_class c"),
        [L::UnqualifiedPgCatalog("pg_class".into())]
    );
    assert_eq!(
        pg("SELECT pg_get_userbyid(1) AS x"),
        [L::UnqualifiedPgCatalog("pg_get_userbyid".into())]
    );
    assert_eq!(
        pg("SELECT 1 AS x FROM PG_CATALOG.pg_class c"),
        [L::NonCanonicalCatalogName("PG_CATALOG".into())]
    );
    assert_eq!(
        pg("SELECT 1 AS x FROM pg_catalog.PG_CLASS c"),
        [L::NonCanonicalCatalogName("PG_CLASS".into())]
    );
    assert_eq!(
        pg("SELECT 1 AS x FROM INFORMATION_SCHEMA.columns"),
        [L::NonCanonicalCatalogName("INFORMATION_SCHEMA".into())]
    );
    assert_eq!(
        pg("SELECT 1 AS x FROM information_schema.COLUMNS"),
        [L::NonCanonicalCatalogName("COLUMNS".into())]
    );
    assert_eq!(pg("SELECT 1 AS x FROM information_schema.columns"), []);

    let my = |b: &str| lint_kinds(DialectId::MySql, &section(b));
    assert_eq!(
        my("SELECT TABLE_NAME AS x FROM INFORMATION_SCHEMA.TABLES"),
        []
    );
    assert_eq!(
        my("SELECT TABLE_NAME AS x FROM information_schema.TABLES"),
        [L::NonCanonicalCatalogName("information_schema".into())]
    );
    assert_eq!(
        my("SELECT TABLE_NAME AS x FROM INFORMATION_SCHEMA.tables"),
        [L::NonCanonicalCatalogName("tables".into())]
    );
    assert_eq!(
        my("SELECT 1 AS x FROM MYSQL.user"),
        [L::NonCanonicalCatalogName("MYSQL".into())]
    );
    assert_eq!(
        my("SELECT 1 AS x FROM mysql.user, performance_schema.threads"),
        []
    );

    let lite = |b: &str| lint_kinds(DialectId::Sqlite, &section(b));
    assert_eq!(lite("SELECT name AS x FROM sqlite_schema"), []);
    assert_eq!(
        lite("SELECT name AS x FROM sqlite_master"),
        [L::NonCanonicalCatalogName("sqlite_master".into())]
    );
    assert_eq!(
        lite("SELECT name AS x FROM SQLITE_SCHEMA"),
        [L::NonCanonicalCatalogName("SQLITE_SCHEMA".into())]
    );
    assert_eq!(
        lite("SELECT name AS x FROM Pragma_Table_Xinfo(:t)"),
        [L::NonCanonicalCatalogName("Pragma_Table_Xinfo".into())]
    );
}

#[test]
fn lower_upper_ilike_collate_on_catalog_names_are_linted() {
    use LintKind as L;
    let pg = |b: &str| lint_kinds(DialectId::Postgres, &section(b));
    for (body, word) in [
        (
            "SELECT 1 AS x FROM pg_catalog.pg_class c WHERE lower(c.relname) = :n",
            "lower",
        ),
        (
            "SELECT 1 AS x FROM pg_catalog.pg_class c WHERE UPPER (c.relname) = :n",
            "UPPER",
        ),
        (
            "SELECT 1 AS x FROM pg_catalog.pg_class c WHERE c.relname ILIKE :n",
            "ILIKE",
        ),
        (
            "SELECT 1 AS x FROM pg_catalog.pg_class c WHERE c.relname COLLATE \"C\" = :n",
            "COLLATE",
        ),
        (
            "SELECT 1 AS x FROM pg_catalog.pg_class c WHERE c.relname ~* :n",
            "~*",
        ),
        (
            "SELECT 1 AS x FROM pg_catalog.pg_class c WHERE c.relname ~~* :n",
            "~~*",
        ),
        (
            "SELECT 1 AS x FROM pg_catalog.pg_class c WHERE c.relname !~* :n",
            "!~*",
        ),
        (
            "SELECT 1 AS x FROM pg_catalog.pg_class c WHERE pg_catalog.lower(c.relname) = :n",
            "lower",
        ),
    ] {
        assert_eq!(
            pg(body),
            [L::CaseFoldingOnCatalogName(word.into())],
            "{body}"
        );
    }
    let my = |b: &str| lint_kinds(DialectId::MySql, &section(b));
    for (body, word) in [
        (
            "SELECT 1 AS x FROM INFORMATION_SCHEMA.TABLES WHERE LCASE(TABLE_NAME) = :n",
            "LCASE",
        ),
        (
            "SELECT 1 AS x FROM INFORMATION_SCHEMA.TABLES WHERE ucase(TABLE_NAME) = :n",
            "ucase",
        ),
        (
            "SELECT 1 AS x FROM INFORMATION_SCHEMA.TABLES WHERE TABLE_NAME COLLATE utf8mb4_tr_0900_ai_ci = :n",
            "COLLATE",
        ),
    ] {
        assert_eq!(
            my(body),
            [L::CaseFoldingOnCatalogName(word.into())],
            "{body}"
        );
    }
    // A column that merely contains the word is not a call.
    assert_eq!(
        pg("SELECT c.lower_bound AS x, lower_x AS y FROM pg_catalog.pg_class c"),
        []
    );
    assert_eq!(pg("SELECT 'lower(x)' AS x"), []);
    // A quoted function or catalog name is still read.
    assert_eq!(
        pg("SELECT 1 AS x FROM pg_catalog.pg_class c WHERE \"lower\"(c.relname) = :n"),
        [L::CaseFoldingOnCatalogName("lower".into())]
    );
    assert_eq!(
        pg("SELECT 1 AS x FROM \"pg_class\" c"),
        [L::UnqualifiedPgCatalog("pg_class".into())]
    );
    assert!(
        pg("SELECT 1 AS x FROM pg_catalog.pg_class c WHERE pg_catalog.\"age\"(c.xmin) <= :t")
            .contains(&L::OrderingOnAge)
    );
    assert_eq!(
        lint_kinds(
            DialectId::MySql,
            &section("SELECT 1 AS x FROM INFORMATION_SCHEMA.TABLES WHERE `LCASE`(TABLE_NAME) = :n")
        ),
        [L::CaseFoldingOnCatalogName("LCASE".into())]
    );
}

#[test]
fn update_time_is_linted_in_mysql_packs() {
    use LintKind as L;
    assert_eq!(
        lint_kinds(
            DialectId::MySql,
            &section("SELECT UPDATE_TIME AS x FROM INFORMATION_SCHEMA.TABLES")
        ),
        [L::UpdateTime]
    );
    assert_eq!(
        lint_kinds(
            DialectId::MariaDb,
            &section("SELECT t.update_time AS x FROM INFORMATION_SCHEMA.TABLES t")
        ),
        [L::UpdateTime]
    );
    assert_eq!(
        lint_kinds(
            DialectId::MySql,
            &section("SELECT CREATE_TIME AS x FROM INFORMATION_SCHEMA.TABLES")
        ),
        []
    );
}

#[test]
fn group_concat_and_le_on_age_and_txid_current_are_linted() {
    use LintKind as L;
    assert_eq!(
        lint_kinds(
            DialectId::MySql,
            &section("SELECT GROUP_CONCAT(COLUMN_NAME) AS x FROM INFORMATION_SCHEMA.COLUMNS")
        ),
        [L::GroupConcat]
    );
    let pg = |b: &str| lint_kinds(DialectId::Postgres, &section(b));
    for body in [
        "SELECT 1 AS x FROM pg_catalog.pg_class c WHERE pg_catalog.age(c.xmin) <= :t",
        "SELECT 1 AS x FROM pg_catalog.pg_class c WHERE age(c.xmin) <= :t",
        "SELECT 1 AS x FROM pg_catalog.pg_class c WHERE :t >= pg_catalog.age(c.xmin)",
        "SELECT 1 AS x FROM pg_catalog.pg_class c WHERE pg_catalog.age(c.xmin) < :t",
        "SELECT 1 AS x FROM pg_catalog.pg_class c WHERE pg_catalog.age((c.xmin)) > :t",
    ] {
        let k = pg(body);
        assert!(k.contains(&L::OrderingOnAge), "{body}: {k:?}");
    }
    assert_eq!(
        pg(
            "SELECT 1 AS x FROM pg_catalog.pg_class c WHERE pg_catalog.age(c.xmin) BETWEEN 0 AND pg_catalog.age(:w::pg_catalog.xid)"
        ),
        []
    );
    // Across lines too.
    assert!(
        pg("SELECT 1 AS x FROM pg_catalog.pg_class c WHERE pg_catalog.age(c.xmin)\n<= :t")
            .contains(&L::OrderingOnAge)
    );
    for (d, body, word) in [
        (
            DialectId::Postgres,
            "SELECT pg_catalog.txid_current() AS x",
            "txid_current",
        ),
        (
            DialectId::Postgres,
            "SELECT pg_catalog.pg_current_xact_id() AS x",
            "pg_current_xact_id",
        ),
        (
            DialectId::MySql,
            "SELECT txid_current() AS x",
            "txid_current",
        ),
    ] {
        assert!(
            lint_kinds(d, &section(body)).contains(&L::XidAssigningFunction(word.into())),
            "{body}"
        );
    }
    assert_eq!(
        pg("SELECT pg_catalog.pg_current_xact_id_if_assigned() IS NULL AS x"),
        []
    );
}

#[test]
fn age_outside_the_between_shape_is_linted() {
    use LintKind as L;
    let pg = |b: &str| lint_kinds(DialectId::Postgres, &section(b));
    let w = "SELECT c.oid AS x FROM pg_catalog.pg_class c WHERE";
    for cond in [
        "pg_catalog.age(c.xmin)::int8 <= :w",
        "pg_catalog.age(c.xmin)::pg_catalog.int8 <= :w",
        "(pg_catalog.age(c.xmin)) <= :w",
        ":w >= (pg_catalog.age(c.xmin))",
        "pg_catalog.age(c.xmin) + 0 < :w",
        "pg_catalog.age(c.xmin) = :w",
        "pg_catalog.age(c.xmin) BETWEEN 1 AND :w",
        "pg_catalog.age(c.xmin) IS NOT NULL",
    ] {
        let k = pg(&format!("{w} {cond}"));
        assert!(k.contains(&L::OrderingOnAge), "{cond}: {k:?}");
    }
    for cond in [
        "pg_catalog.age(c.xmin) BETWEEN 0 AND pg_catalog.age(:w::pg_catalog.xid)",
        "pg_catalog.age(c.xmin)::pg_catalog.int8 BETWEEN 0 AND pg_catalog.age(:w::pg_catalog.xid)",
    ] {
        assert_eq!(pg(&format!("{w} {cond}")), [], "{cond}");
    }
}

#[test]
fn pg_catalog_functions_are_qualified() {
    use LintKind as L;
    let pg = |b: &str| lint_kinds(DialectId::Postgres, &section(b));
    // Every section: the catalog functions without a `pg_` prefix.
    for (body, f) in [
        (
            "SELECT format_type(a.atttypid, a.atttypmod) AS x FROM pg_catalog.pg_attribute a",
            "format_type",
        ),
        (
            "SELECT obj_description(c.oid, 'pg_class') AS x FROM pg_catalog.pg_class c",
            "obj_description",
        ),
        (
            "SELECT has_table_privilege(c.oid, 'SELECT') AS x FROM pg_catalog.pg_class c",
            "has_table_privilege",
        ),
        (
            "SELECT 1 AS x FROM unnest(:a::pg_catalog.oid[]) u",
            "unnest",
        ),
    ] {
        assert_eq!(pg(body), [L::UnqualifiedPgCatalog(f.into())], "{body}");
    }
    assert_eq!(
        pg(
            "SELECT pg_catalog.format_type(a.atttypid, NULL) AS x, coalesce(a.attname, '') AS y FROM pg_catalog.pg_attribute a WHERE a.attnum IN (1, 2) AND EXISTS (SELECT 1)"
        ),
        []
    );
    // A joinable section (no pinned search_path, R-M4-46): every function call.
    let joinable = |body: &str| {
        lint_kinds(
            DialectId::Postgres,
            &format!("--- name: S ---\n--- returns: x text ---\n--- joinable ---\n{body}\n"),
        )
    };
    assert_eq!(
        joinable("SELECT myfunc(c.oid) AS x FROM pg_catalog.pg_class c"),
        [L::UnqualifiedPgCatalog("myfunc".into())]
    );
    assert_eq!(
        joinable("SELECT public.myfunc(c.oid) AS x FROM pg_catalog.pg_class c"),
        [L::UnqualifiedPgCatalog("myfunc".into())]
    );
    assert_eq!(
        joinable(
            "SELECT pg_catalog.length(c.relname) AS x, CAST(c.oid AS pg_catalog.int8) AS y, coalesce(c.reltuples, 0) AS z FROM pg_catalog.pg_class c WHERE c.oid = ANY (ARRAY[1]) AND EXISTS (SELECT 1)"
        ),
        []
    );
    // PG 18 folds.
    for f in ["casefold", "initcap"] {
        assert!(
            pg(&format!(
                "SELECT 1 AS x FROM pg_catalog.pg_class c WHERE pg_catalog.{f}(c.relname) = :n"
            ))
            .contains(&L::CaseFoldingOnCatalogName(f.into())),
            "{f}"
        );
    }
}

#[test]
fn lint_reads_every_branch() {
    let text = section(
        "SELECT 1 AS x\n--# if version >= 150000\nFROM pg_class\n--# else\nFROM pg_catalog.pg_class\n--# end",
    );
    let errors = lint(&parse(DialectId::Postgres, &text), DialectId::Postgres);
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].line, 5);
}

// ---------------------------------------------------------------- variants (HV-INTRO-04 step 4)

#[test]
fn reachable_variants_cover_a_fixed_pack() {
    let text = section(
        "SELECT 1 AS x\n\
         --# if version >= 150000\n-- m1\n\
         --# elif version >= 140000 and mode == incremental\n-- m2\n\
         --# else\n-- m3\n--# end\n\
         --# if flavor == mariadb\n-- m4\n--# end\n\
         --# if level >= sources\n-- m5\n--# end",
    );
    let pack = parse(DialectId::MySql, &text);
    let variants = pack.reachable_variants("S");
    let mut seen = BTreeSet::new();
    for v in &variants {
        assert_eq!(v.dialect, DialectId::MySql);
        assert_eq!(
            v.flags.contains(PackFlags::MARIADB),
            v.flavor == DbmsKind::MariaDb
        );
        let sql = pack.render("S", v).unwrap().remove(0);
        for m in 1..=5 {
            if sql.sql().contains(&format!("-- m{m}\n")) || sql.sql().ends_with(&format!("-- m{m}"))
            {
                seen.insert(m);
            }
        }
    }
    assert_eq!(seen, (1..=5).collect::<BTreeSet<_>>());
    assert!(pack.reachable_variants("Nope").is_empty());
}

/// A generated condition over small integer thresholds and the other dimensions.
#[derive(Debug, Clone)]
enum GenCond {
    Version(&'static str, u32),
    Mode(bool, Mode),
    Level(&'static str, Level),
    Flavor(bool, DbmsKind),
    Flag(&'static str),
    Not(Box<GenCond>),
    And(Box<GenCond>, Box<GenCond>),
    Or(Box<GenCond>, Box<GenCond>),
}

impl GenCond {
    fn text(&self) -> String {
        match self {
            GenCond::Version(op, n) => format!("version {op} {n}"),
            GenCond::Mode(eq, m) => format!(
                "mode {} {}",
                if *eq { "==" } else { "!=" },
                match m {
                    Mode::Full => "full",
                    Mode::Incremental => "incremental",
                    Mode::Brief => "brief",
                    Mode::System => "system",
                }
            ),
            GenCond::Level(op, l) => format!(
                "level {op} {}",
                match l {
                    Level::Names => "names",
                    Level::Structure => "structure",
                    Level::Sources => "sources",
                }
            ),
            GenCond::Flavor(eq, f) => {
                format!("flavor {} {}", if *eq { "==" } else { "!=" }, f.id())
            }
            GenCond::Flag(f) => format!("flag({f})"),
            GenCond::Not(c) => format!("not ({})", c.text()),
            GenCond::And(a, b) => format!("({}) and ({})", a.text(), b.text()),
            GenCond::Or(a, b) => format!("({}) or ({})", a.text(), b.text()),
        }
    }
}

fn gen_cond() -> impl Strategy<Value = GenCond> {
    let op = prop::sample::select(vec![">=", "<", ">", "<=", "==", "!="]);
    let leaf = prop_oneof![
        (op.clone(), (1u32..10).prop_map(|n| n * 10)).prop_map(|(o, n)| GenCond::Version(o, n)),
        (
            any::<bool>(),
            prop::sample::select(vec![
                Mode::Full,
                Mode::Incremental,
                Mode::Brief,
                Mode::System
            ])
        )
            .prop_map(|(e, m)| GenCond::Mode(e, m)),
        (
            op,
            prop::sample::select(vec![Level::Names, Level::Structure, Level::Sources])
        )
            .prop_map(|(o, l)| GenCond::Level(o, l)),
        (
            any::<bool>(),
            prop::sample::select(vec![DbmsKind::MySql, DbmsKind::MariaDb, DbmsKind::TiDb])
        )
            .prop_map(|(e, f)| GenCond::Flavor(e, f)),
        prop::sample::select(vec!["mariadb", "joined"]).prop_map(GenCond::Flag),
    ];
    leaf.prop_recursive(2, 6, 2, |inner| {
        prop_oneof![
            inner.clone().prop_map(|c| GenCond::Not(Box::new(c))),
            (inner.clone(), inner.clone())
                .prop_map(|(a, b)| GenCond::And(Box::new(a), Box::new(b))),
            (inner.clone(), inner).prop_map(|(a, b)| GenCond::Or(Box::new(a), Box::new(b))),
        ]
    })
}

#[derive(Debug, Clone)]
enum GenNode {
    Marker,
    If(Vec<(GenCond, Vec<GenNode>)>, Option<Vec<GenNode>>),
}

fn gen_body() -> impl Strategy<Value = Vec<GenNode>> {
    let leaf = Just(GenNode::Marker);
    let node = leaf.prop_recursive(3, 24, 3, |inner| {
        (
            prop::collection::vec(
                (gen_cond(), prop::collection::vec(inner.clone(), 1..3)),
                1..3,
            ),
            prop::option::of(prop::collection::vec(inner, 1..3)),
        )
            .prop_map(|(arms, else_)| GenNode::If(arms, else_))
    });
    prop::collection::vec(node, 1..4)
}

fn write_body(nodes: &[GenNode], out: &mut String, next: &mut u32) {
    for n in nodes {
        match n {
            GenNode::Marker => {
                out.push_str(&format!("-- m{next}\n"));
                *next += 1;
            }
            GenNode::If(arms, else_) => {
                for (k, (c, body)) in arms.iter().enumerate() {
                    let kw = if k == 0 { "if" } else { "elif" };
                    out.push_str(&format!("--# {kw} {}\n", c.text()));
                    write_body(body, out, next);
                }
                if let Some(body) = else_ {
                    out.push_str("--# else\n");
                    write_body(body, out, next);
                }
                out.push_str("--# end\n");
            }
        }
    }
}

fn markers(sql: &str) -> BTreeSet<u32> {
    sql.lines()
        .filter_map(|l| l.strip_prefix("-- m"))
        .map(|n| n.parse().unwrap())
        .collect()
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 128, ..ProptestConfig::default() })]

    /// `reachable_variants` reaches exactly the markers a brute-force sweep of every version
    /// near every threshold, mode, level, flavor and flag reaches.
    #[test]
    fn reachable_variants_cover_every_branch(body in gen_body(), joinable in any::<bool>()) {
        let mut text = String::from("--- name: S ---\n--- returns: x text ---\n");
        if joinable {
            text.push_str("--- joinable ---\n");
        }
        text.push_str("SELECT 1 AS x\n");
        let mut next = 0;
        write_body(&body, &mut text, &mut next);
        let pack = DraftPack::parse(DialectId::MySql, &text).expect("generated packs parse");

        let mut by_variants = BTreeSet::new();
        for v in pack.reachable_variants("S") {
            let q = pack.render("S", &v).expect("every variant renders");
            by_variants.extend(markers(q[0].sql()));
        }

        let mut brute = BTreeSet::new();
        for version in 0..=101u32 {
            for mode in [Mode::Full, Mode::Incremental, Mode::Brief, Mode::System] {
                for level in [Level::Names, Level::Structure, Level::Sources] {
                    for flavor in [DbmsKind::MySql, DbmsKind::MariaDb, DbmsKind::TiDb] {
                        for joined in [false, true] {
                            if joined && !joinable {
                                continue;
                            }
                            let mut flags = PackFlags::empty();
                            flags.set(PackFlags::MARIADB, flavor == DbmsKind::MariaDb);
                            flags.set(PackFlags::JOINED, joined);
                            let c = PackCtx {
                                dialect: DialectId::MySql,
                                version,
                                flavor,
                                mode,
                                level,
                                flags,
                                params: Params::default(),
                            };
                            let q = pack.render("S", &c).expect("renders");
                            brute.extend(markers(q[0].sql()));
                        }
                    }
                }
            }
        }
        prop_assert_eq!(by_variants, brute);
    }

    /// Parsing arbitrary text never panics (the fuzz target's property, in-process).
    #[test]
    fn parse_never_panics(text in "(--- name: S ---\n|--- returns: x text ---\n|--# if [a-z()=<>! 0-9]{0,20}\n|--# else\n|--# end\n|[ -~]{0,30}\n)*") {
        for d in [DialectId::Postgres, DialectId::MySql, DialectId::Sqlite] {
            if let Ok(p) = DraftPack::parse(d, &text) {
                let _ = lint(&p, d);
                for s in p.sections() {
                    let _ = p.render(s, &ctx(d, 150000));
                }
            }
        }
    }
}

// ---------------------------------------------------------------- locale (HV-INTRO-14 rule 5)

const LOCALE_PACK: &str = "\
--- name: S ---
--- returns: x text ---
SELECT 'İı' AS x, #{max_in_list} AS m, c.relname FROM pg_catalog.pg_class c
--# if version >= 150000 and level >= structure
WHERE c.relname = :name AND c.oid IN :[ids]
--# end
";

fn locale_render() -> String {
    let mut c = ctx_pg(170004);
    c.params.scalars.push((Arc::from("name"), text("İTEMS")));
    c.params.lists.push((
        Arc::from("ids"),
        (0..1500).map(|i| text(&i.to_string())).collect(),
    ));
    let qs = render(LOCALE_PACK, "S", c);
    let mut out = String::new();
    for q in &qs {
        out.push_str(q.sql());
        out.push('\u{1}');
        out.push_str(&q.binds().len().to_string());
        out.push('\u{2}');
    }
    out
}

#[test]
fn rendering_is_byte_identical_under_tr_locale() {
    let here = locale_render();
    assert!(here.contains("$1001)"), "the first chunk numbers on");
    if std::env::var_os("HACIVAT_T04_LOCALE_CHILD").is_some() {
        println!("PACK-RENDER:{}", here.escape_default());
        return;
    }
    let name = "rendering_is_byte_identical_under_tr_locale";
    for locale in ["tr_TR.UTF-8", "C"] {
        let out = std::process::Command::new(std::env::current_exe().expect("the test binary"))
            .args(["--exact", name, "--test-threads=1", "--nocapture"])
            .env("LANG", locale)
            .env("LC_ALL", locale)
            .env("LC_CTYPE", locale)
            .env("LC_COLLATE", locale)
            .env("HACIVAT_T04_LOCALE_CHILD", "1")
            .output()
            .expect("the child runs");
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(out.status.success(), "{locale}: {stdout}");
        let line = stdout
            .lines()
            .find_map(|l| l.split_once("PACK-RENDER:").map(|(_, r)| r))
            .unwrap_or_else(|| panic!("{locale}: no render line in {stdout}"));
        assert_eq!(line, here.escape_default().to_string(), "{locale}");
    }
}
