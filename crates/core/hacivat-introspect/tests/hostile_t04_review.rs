//! T4 security review: the reviewer's hostile probes (H1–H10), committed in fix round 1 and
//! flipped so each finding test asserts the fix (refused, not accepted). H10 held already.

use std::sync::Arc;

use hacivat_introspect::pack::{
    BindValue, DialectId, DraftPack, Flavor, LintKind, LogicalType, Mode, PackCtx, PackErrorKind,
    PackFlags, Params, RenderError, lint,
};
use hacivat_meta::Level;

fn text(s: &str) -> BindValue {
    BindValue::Typed {
        hint: LogicalType::Text {
            max_chars: None,
            fixed: false,
        },
        text: Arc::from(s),
    }
}

fn ctx(d: DialectId) -> PackCtx {
    PackCtx {
        dialect: d,
        version: 170000,
        flavor: match d {
            DialectId::Postgres => Flavor::Postgres,
            DialectId::Sqlite => Flavor::Sqlite,
            DialectId::MariaDb => Flavor::MariaDb,
            _ => Flavor::MySql,
        },
        mode: Mode::Full,
        level: Level::Structure,
        flags: PackFlags::empty(),
        params: Params::default(),
    }
}

fn sec(body: &str) -> String {
    format!("--- name: S ---\n--- returns: x text ---\n{body}\n")
}

fn ids(n: usize) -> Vec<BindValue> {
    (0..n).map(|i| text(&i.to_string())).collect()
}

fn bind_texts(b: &[BindValue]) -> Vec<String> {
    b.iter()
        .map(|v| match v {
            BindValue::Typed { text, .. } => text.to_string(),
            _ => "<null>".into(),
        })
        .collect()
}

fn refused(d: DialectId, body: &str) -> PackErrorKind {
    DraftPack::parse(d, &sec(body)).expect_err(body).kind
}

// H1. PostgreSQL does not quote with backticks: a backtick in a PG pack is refused.
#[test]
fn h1_pg_backtick_is_refused() {
    assert_eq!(
        refused(
            DialectId::Postgres,
            "SELECT c.relname AS x FROM pg_catalog.pg_class c WHERE c.oid = :oid OR `$1; x` IS NULL"
        ),
        PackErrorKind::Backtick
    );
    assert_eq!(
        refused(DialectId::Postgres, "SELECT 1 AS x WHERE $1 IS NULL"),
        PackErrorKind::RawPlaceholder
    );
}

// H2. `/* a /*/` opens a nested PG comment; the lexer refuses it, so no code hides from the
// lint.
#[test]
fn h2_pg_overlapping_nested_comment_is_refused() {
    let trick = sec(
        "SELECT c.oid AS x /* a /*/\n, 'q */ */ , upper(c.relname) AS y -- ' AS z\nFROM pg_catalog.pg_class c",
    );
    let e = DraftPack::parse(DialectId::Postgres, &trick).unwrap_err();
    assert_eq!((e.kind, e.line), (PackErrorKind::NestedComment, 3));
    // Control: the code PG would run is linted when written plainly.
    let plain = sec("SELECT c.oid AS x , upper(c.relname) AS y\nFROM pg_catalog.pg_class c");
    let p = DraftPack::parse(DialectId::Postgres, &plain).unwrap();
    assert!(
        lint(&p, DialectId::Postgres)
            .iter()
            .any(|e| matches!(e.kind, LintKind::CaseFoldingOnCatalogName(_)))
    );
    assert_eq!(
        refused(DialectId::Postgres, "SELECT 1 AS x /* a /* b */ */"),
        PackErrorKind::NestedComment
    );
}

// H3. MySQL executable comments are code to MySQL: a parse error, not only a lint finding.
#[test]
fn h3_mysql_executable_comment_is_refused_at_parse() {
    assert_eq!(
        refused(
            DialectId::MySql,
            "SELECT TABLE_NAME AS x FROM INFORMATION_SCHEMA.TABLES WHERE TABLE_SCHEMA = :s /*! OR TABLE_NAME = ? ; */"
        ),
        PackErrorKind::ExecutableComment
    );
    assert_eq!(
        refused(DialectId::MariaDb, "SELECT 1 AS x /*M!100100 OR 1 */"),
        PackErrorKind::ExecutableComment
    );
}

// H4. SQLite parameters `#name`, `:1`, `:é` are refused like `@x`.
#[test]
fn h4_sqlite_hash_and_numeric_colon_parameters_are_refused() {
    for body in [
        "SELECT name AS x FROM sqlite_schema WHERE name = #x",
        "SELECT name AS x FROM sqlite_schema WHERE name = :1",
        "SELECT name AS x FROM sqlite_schema WHERE name = :\u{e9}",
        "SELECT 1 AS x WHERE @x IS NULL",
        "SELECT 1 AS x WHERE ?1 IS NULL",
        "SELECT 1 AS x WHERE $x IS NULL",
    ] {
        assert_eq!(
            refused(DialectId::Sqlite, body),
            PackErrorKind::RawPlaceholder,
            "{body}"
        );
    }
}

// H5. List-bind forms whose chunked result differs from the unchunked one are refused, and the
// empty-list "no query" answer is left only to forms where it is right.
#[test]
fn h5_list_bind_forms_that_chunking_breaks_are_refused() {
    let pg = [
        "SELECT c.relname AS x FROM pg_catalog.pg_class c WHERE NOT c.oid IN :[ids]",
        "SELECT c.relname AS x FROM pg_catalog.pg_class c WHERE NOT (c.oid IN :[ids])",
        "SELECT c.relname AS x FROM pg_catalog.pg_class c WHERE (c.oid IN :[ids]) IS NOT TRUE",
        "SELECT c.relname AS x FROM pg_catalog.pg_class c WHERE NOT EXISTS (SELECT 1 FROM pg_catalog.pg_depend d WHERE d.objid = c.oid AND d.refobjid IN :[ids])",
        "SELECT count(*) AS x FROM pg_catalog.pg_class c WHERE c.oid IN :[ids]",
        "SELECT c.relname AS x FROM pg_catalog.pg_class c WHERE c.oid IN :[ids] ORDER BY c.relname LIMIT 10",
        "SELECT c.relname AS x FROM pg_catalog.pg_class c WHERE c.relkind = 'r' OR c.oid IN :[ids]",
    ];
    for body in pg {
        let k = refused(DialectId::Postgres, body);
        assert!(
            matches!(
                k,
                PackErrorKind::ListBindNotConjunct | PackErrorKind::ListBindUnchunkable(_)
            ),
            "{body}: {k:?}"
        );
    }
    assert_eq!(
        refused(
            DialectId::MySql,
            "SELECT TABLE_NAME AS x FROM INFORMATION_SCHEMA.TABLES WHERE !(TABLE_NAME IN :[ids])"
        ),
        PackErrorKind::ListBindNotConjunct
    );
    // The positive form with an empty list: no query, which is the right answer there.
    let p = DraftPack::parse(
        DialectId::Postgres,
        &sec("SELECT c.relname AS x FROM pg_catalog.pg_class c WHERE c.oid IN :[ids]"),
    )
    .unwrap();
    let mut c = ctx(DialectId::Postgres);
    c.params.lists.push((Arc::from("ids"), Vec::new()));
    assert_eq!(p.render("S", &c), Ok(Vec::new()));
}

// H6. The same list used twice is refused (each use would get the same chunk).
#[test]
fn h6_same_list_twice_is_refused() {
    assert_eq!(
        refused(
            DialectId::Postgres,
            "SELECT d.objid AS x FROM pg_catalog.pg_depend d WHERE d.objid IN :[ids] AND d.refobjid IN :[ids]"
        ),
        PackErrorKind::ListBindReused
    );
}

// H7. Duplicate items are removed before chunking: a duplicate never lands in two chunks.
#[test]
fn h7_duplicate_list_items_are_removed_before_chunking() {
    let p = DraftPack::parse(
        DialectId::Postgres,
        &sec("SELECT c.relname AS x FROM pg_catalog.pg_class c WHERE c.oid IN :[ids]"),
    )
    .unwrap();
    let mut items = ids(1001);
    items[1000] = text("0");
    let mut c = ctx(DialectId::Postgres);
    c.params.lists.push((Arc::from("ids"), items));
    let qs = p.render("S", &c).unwrap();
    assert_eq!(qs.len(), 1);
    let b = bind_texts(qs[0].binds());
    assert_eq!(b.len(), 1000);
    assert_eq!(b.iter().filter(|t| *t == "0").count(), 1);
}

// H8. `age()` outside the BETWEEN shape is linted through casts, parentheses and arithmetic;
// PG 18 `casefold()`/`initcap()` are folds.
#[test]
fn h8_lint_catches_age_through_cast_parens_arithmetic_and_other_folds() {
    let ordering = |body: &str| {
        let p = DraftPack::parse(DialectId::Postgres, &sec(body)).unwrap();
        lint(&p, DialectId::Postgres)
            .iter()
            .any(|e| e.kind == LintKind::OrderingOnAge)
    };
    for body in [
        "SELECT c.oid AS x FROM pg_catalog.pg_class c WHERE pg_catalog.age(c.xmin) <= :w",
        "SELECT c.oid AS x FROM pg_catalog.pg_class c WHERE pg_catalog.age(c.xmin)::int8 <= :w",
        "SELECT c.oid AS x FROM pg_catalog.pg_class c WHERE (pg_catalog.age(c.xmin)) <= :w",
        "SELECT c.oid AS x FROM pg_catalog.pg_class c WHERE :w >= (pg_catalog.age(c.xmin))",
        "SELECT c.oid AS x FROM pg_catalog.pg_class c WHERE pg_catalog.age(c.xmin) + 0 < :w",
    ] {
        assert!(ordering(body), "missed: {body}");
    }
    let folds = |body: &str| {
        let p = DraftPack::parse(DialectId::Postgres, &sec(body)).unwrap();
        lint(&p, DialectId::Postgres)
            .iter()
            .any(|e| matches!(e.kind, LintKind::CaseFoldingOnCatalogName(_)))
    };
    for body in [
        "SELECT c.oid AS x FROM pg_catalog.pg_class c WHERE pg_catalog.casefold(c.relname) = :n",
        "SELECT c.oid AS x FROM pg_catalog.pg_class c WHERE pg_catalog.initcap(c.relname) = :n",
    ] {
        assert!(folds(body), "missed: {body}");
    }
}

// H9. Runtime text parses only into a `DraftPack`, whose queries have no `as_meta_query`
// (the `compile_fail` doctests in `pack::render` prove both that and that `Pack::parse`
// refuses a non-`'static` text); check-deps `PackParseOnlyFromPacks` limits `Pack::parse` to
// the packs, the introspectors, `tests/` and `fuzz/meta/`.
#[test]
fn h9_runtime_text_renders_only_as_a_draft() {
    let user_text: String = [
        "WITH d AS (DELETE FROM public.t RETURNING 1) ",
        "SELECT 1 AS x FROM d",
    ]
    .concat();
    let pack_text =
        format!("--- name: S ---\n--- returns: x text ---\n--- joinable ---\n{user_text}\n");
    // The reviewer's text names DELETE in a joinable section: refused at parse (T5 carry).
    assert_eq!(
        DraftPack::parse(DialectId::Postgres, &pack_text)
            .unwrap_err()
            .kind,
        PackErrorKind::JoinableWriteWord("DELETE".into())
    );
    // Any runtime text that does parse renders only `DraftQuery`s: readable, never runnable.
    let runtime: String = ["SELECT 1 AS x ", "FROM pg_catalog.pg_class c"].concat();
    let p = DraftPack::parse(
        DialectId::Postgres,
        &format!("--- name: S ---\n--- returns: x text ---\n--- joinable ---\n{runtime}\n"),
    )
    .expect("parses as a draft");
    let mut c = ctx(DialectId::Postgres);
    c.flags = PackFlags::JOINED;
    let q: Vec<hacivat_introspect::pack::DraftQuery> = p.render("S", &c).unwrap();
    assert_eq!(q[0].sql(), runtime);
}

// H10. Show-create identifier break-out attempts through the bind value (all must hold).
#[test]
fn h10_show_create_ident_hostile_values_stay_one_identifier() {
    let pack = "--- name: S ---\n--- returns: x text ---\n--- show-create ---\nSHOW CREATE TABLE #{ident:s}.#{ident:t}\n";
    let p = DraftPack::parse(DialectId::MySql, pack).unwrap();
    let hostile = [
        "`; SELECT 1; -- ",
        "a`.`b",
        "```",
        "x`/*",
        "x`#",
        "\u{FF40}x",
        "\u{2018}x\u{2019}",
        "\u{0300}`",
        "中`x",
        "x\u{FEFF}`",
        "x\r\n`y",
        "\u{1F600}`",
    ];
    for h in hostile {
        let mut c = ctx(DialectId::MySql);
        c.params.scalars.push((Arc::from("s"), text("db")));
        c.params.scalars.push((Arc::from("t"), text(h)));
        let q = p.render("S", &c).unwrap();
        let sql = q[0].sql();
        let inner = sql
            .strip_prefix("SHOW CREATE TABLE `db`.`")
            .and_then(|s| s.strip_suffix('`'))
            .unwrap_or_else(|| panic!("{sql}"));
        assert!(!inner.replace("``", "").contains('`'), "{sql}");
        assert_eq!(inner.replace("``", "`"), h);
        assert!(q[0].binds().is_empty());
    }
    // An absent bind is an error, and (fix round 1, m5) a value not hinted as text is not a
    // name.
    let mut c = ctx(DialectId::MySql);
    c.params.scalars.push((Arc::from("s"), text("db")));
    assert!(matches!(
        p.render("S", &c),
        Err(RenderError::MissingBind(_))
    ));
    c.params.scalars.push((
        Arc::from("t"),
        BindValue::Typed {
            hint: LogicalType::Int {
                bits: 32,
                signed: true,
            },
            text: Arc::from("1"),
        },
    ));
    assert_eq!(
        p.render("S", &c),
        Err(RenderError::IdentNotText(Arc::from("t")))
    );
    // A crafted section header never reaches SQL text.
    for hdr in [
        "--- name: S; DROP ---",
        "--- name: S` ---",
        "--- name: S --- x",
        "--- name: \u{130} ---",
    ] {
        assert!(
            DraftPack::parse(
                DialectId::MySql,
                &format!("{hdr}\n--- returns: x text ---\nSELECT 1 AS x\n")
            )
            .is_err(),
            "{hdr}"
        );
    }
}
