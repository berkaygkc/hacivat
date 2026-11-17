//! Bind parameters and template variables (SQL-PARAM-01, -02; M3b Task 6): which markers are
//! placeholders, which are bindable, the rewrite the driver receives and its bind plan. Fail
//! closed: a marker inside a string, comment, dollar body or `COPY` data is never a
//! parameter, and a marker Hacivat cannot bind makes the whole unit unbindable.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use proptest::prelude::*;
use sql_dialect::{ClientFlavor, DialectCtx, DialectId, kw};
use sql_lexer::{LexState, TokenFlags, TokenKind, lex};
use sql_split::{
    ParamDiag, ParamScan, ParamSettings, PlaceholderKind, SplitMode, SplitStart, StmtHead,
    TemplatePattern, scan_params, split_from,
};
use sql_text::TextRange;

fn pg() -> DialectCtx {
    DialectCtx::new(DialectId::Postgres, None)
}

fn mysql() -> DialectCtx {
    DialectCtx::new(DialectId::MySql, None)
}

fn mariadb() -> DialectCtx {
    DialectCtx::new(DialectId::MariaDb, None)
}

fn sqlite() -> DialectCtx {
    DialectCtx::new(DialectId::Sqlite, None)
}

/// MySQL for the corpus rows that use its variables (`@x`), else PostgreSQL.
fn pg_or_mysql(s: &str) -> DialectCtx {
    if s.contains('@') { mysql() } else { pg() }
}

fn head_select() -> StmtHead {
    StmtHead {
        first_keyword: Some(kw::SELECT),
        ..StmtHead::default()
    }
}

fn head_of(id: sql_dialect::KeywordId) -> StmtHead {
    StmtHead {
        first_keyword: Some(id),
        ..StmtHead::default()
    }
}

fn scan(s: &str, ctx: &DialectCtx) -> ParamScan {
    scan_params(s, ctx, &head_select(), &ParamSettings::default())
}

fn text_of(s: &str, r: TextRange) -> &str {
    &s[usize::from(r.start())..usize::from(r.end())]
}

fn bindable_texts<'a>(s: &'a str, p: &ParamScan) -> Vec<&'a str> {
    p.placeholders
        .iter()
        .filter(|x| x.bindable)
        .map(|x| text_of(s, x.range))
        .collect()
}

fn unbindable_texts<'a>(s: &'a str, p: &ParamScan) -> Vec<&'a str> {
    p.placeholders
        .iter()
        .filter(|x| !x.bindable)
        .map(|x| text_of(s, x.range))
        .collect()
}

fn diag_names(p: &ParamScan) -> Vec<String> {
    p.diags.iter().map(|(_, d)| format!("{d:?}")).collect()
}

fn final_text(p: &ParamScan) -> Option<&str> {
    p.rewritten.as_ref().map(|r| r.text.as_str())
}

// ---------------------------------------------------------------------------------------------
// alg/20 HV-EXEC-03 test cases 1–8

#[test]
fn hv_exec_03_cases_1_to_8() {
    // 1. PG `:a`, `:b`; the cast and the jsonb `?` stay.
    let s = "select * from t where a = :a and b::int = :b and c ? 'k'";
    let p = scan(s, &pg());
    assert_eq!(bindable_texts(s, &p), [":a", ":b"]);
    assert_eq!(
        final_text(&p),
        Some("select * from t where a = $1 and b::int = $2 and c ? 'k'")
    );

    // 2. PG with `?` placeholders on: only the first `?`.
    let s = "where a = ? and j ? 'k'";
    let on = ParamSettings {
        pg_question_marks: true,
        ..ParamSettings::default()
    };
    let p = scan_params(s, &pg(), &head_select(), &on);
    assert_eq!(bindable_texts(s, &p), ["?"]);
    assert_eq!(p.placeholders[0].kind, PlaceholderKind::Question(1));
    assert_eq!(final_text(&p), Some("where a = $1 and j ? 'k'"));

    // 3. T-SQL has no lexer profile in M3b: every marker is unbindable, nothing is sent
    //    (alg/20 expects `[@y]`; divergence: SQL Server arrives with its driver).
    let s = "DECLARE @x int = 1; SELECT * FROM t WHERE id = @x AND y = @y";
    let p = scan(s, &DialectCtx::new(DialectId::SqlServer, None));
    assert!(!p.placeholders.is_empty());
    assert!(p.placeholders.iter().all(|x| !x.bindable));
    assert!(p.rewritten.is_none());
    assert!(
        p.diags
            .iter()
            .any(|(_, d)| matches!(d, ParamDiag::Unbindable))
    );

    // 4. SQLite: four placeholders, three distinct names and a positional; mixing `?NNN` with
    //    names is refused (the numbering of the rewrite would be ambiguous).
    let s = "select ?2, :a, @a, $a";
    let p = scan(s, &sqlite());
    assert_eq!(p.placeholders.len(), 4);
    let kinds: Vec<PlaceholderKind> = p.placeholders.iter().map(|x| x.kind.clone()).collect();
    assert_eq!(
        kinds,
        [
            PlaceholderKind::Positional(2),
            PlaceholderKind::Named("a".into()),
            PlaceholderKind::Named("@a".into()),
            PlaceholderKind::Named("$a".into()),
        ]
    );
    let names: BTreeSet<_> = kinds
        .iter()
        .filter_map(|k| match k {
            PlaceholderKind::Named(n) => Some(n.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(names.len(), 3);
    assert!(diag_names(&p).contains(&"MixedStyles".to_string()));
    assert!(p.rewritten.is_none());

    // 5. Templates: `${col}` in code is detected; `'${id}'` is string data (strings off).
    let s = "SELECT ${col} FROM t WHERE id='${id}'";
    let p = scan(s, &pg());
    let t: Vec<&str> = p.templates.iter().map(|t| &*t.name).collect();
    assert_eq!(t, ["col"]);
    assert_eq!(p.templates[0].pattern, TemplatePattern::DollarBrace);
    assert_eq!(text_of(s, p.templates[0].range), "${col}");

    // 6. DataGrip's broad patterns (`$name`, `$name$`, `?NN`) are not Hacivat's (alg/10
    //    SQL-PARAM-02 decision): nothing is a template.
    for s in [
        "SELECT $ACCESS FROM t",
        "SELECT * FROM V$ACCESS",
        "SELECT $x1$ a $x1$",
        "SELECT ?69",
    ] {
        let p = scan(s, &pg());
        assert!(p.templates.is_empty(), "{s}");
        assert!(p.placeholders.iter().all(|x| !x.bindable), "{s}");
    }

    // 7. `ALTER ROLE app PASSWORD :pwd`: DDL is refused by default; with placeholders in DDL
    //    turned on the driver receives `PASSWORD $1` (masking is Task 15's).
    let s = "ALTER ROLE app PASSWORD :pwd";
    let p = scan_params(s, &pg(), &head_of(kw::ALTER), &ParamSettings::default());
    assert_eq!(unbindable_texts(s, &p), [":pwd"]);
    assert_eq!(diag_names(&p), ["ParamInDdl"]);
    assert!(p.rewritten.is_none());
    let in_ddl = ParamSettings {
        in_ddl: true,
        ..ParamSettings::default()
    };
    let p = scan_params(s, &pg(), &head_of(kw::ALTER), &in_ddl);
    assert_eq!(final_text(&p), Some("ALTER ROLE app PASSWORD $1"));

    // 8. PG `select arr[1:2]`: none.
    assert!(scan("select arr[1:2]", &pg()).placeholders.is_empty());
}

// ---------------------------------------------------------------------------------------------
// The PARAM corpus (`sql-testkit/corpus/datagrip/param.toml`)

#[derive(Debug, Default)]
struct Case {
    id: String,
    dialect: String,
    input: String,
    pg_question_marks: bool,
    bindable: Vec<String>,
    unbindable: Vec<String>,
    final_text: Option<String>,
    templates: Vec<String>,
    diags: Vec<String>,
    divergence: Option<String>,
    sources: Vec<String>,
}

/// One basic string at the start of `s` (after its opening quote); the rest after the closing
/// quote.
fn basic_string(s: &str) -> (String, &str) {
    let mut out = String::new();
    let mut chars = s.char_indices();
    while let Some((i, c)) = chars.next() {
        match c {
            '"' => return (out, &s[i + 1..]),
            '\\' => match chars.next().map(|(_, e)| e) {
                Some('\\') => out.push('\\'),
                Some('"') => out.push('"'),
                Some('n') => out.push('\n'),
                other => panic!("unsupported escape {other:?} in {s:?}"),
            },
            c => out.push(c),
        }
    }
    panic!("unterminated string: {s:?}")
}

fn string_value(v: &str) -> String {
    let v = v.trim();
    let rest = v
        .strip_prefix('"')
        .unwrap_or_else(|| panic!("not a string: {v}"));
    let (s, tail) = basic_string(rest);
    assert!(tail.trim().is_empty(), "trailing text after {v}");
    s
}

fn array_value(v: &str) -> Vec<String> {
    let mut rest = v
        .trim()
        .strip_prefix('[')
        .unwrap_or_else(|| panic!("not an array: {v}"))
        .trim_start();
    let mut out = Vec::new();
    loop {
        if let Some(tail) = rest.strip_prefix(']') {
            assert!(tail.trim().is_empty(), "trailing text after {v}");
            return out;
        }
        let body = rest
            .strip_prefix('"')
            .unwrap_or_else(|| panic!("not a string array: {v}"));
        let (s, tail) = basic_string(body);
        out.push(s);
        rest = tail.trim_start();
        rest = rest.strip_prefix(',').unwrap_or(rest).trim_start();
    }
}

fn load_param_corpus() -> Vec<Case> {
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../sql-testkit/corpus/datagrip/param.toml");
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let mut cases = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line == "[[case]]" {
            cases.push(Case::default());
            continue;
        }
        let (key, value) = line
            .split_once(" = ")
            .unwrap_or_else(|| panic!("not a key line: {line}"));
        let c = cases.last_mut().expect("a key before the first [[case]]");
        match key {
            "id" => c.id = string_value(value),
            "dialect" => c.dialect = string_value(value),
            "input" => c.input = string_value(value),
            "pg_question_marks" => c.pg_question_marks = value.trim() == "true",
            "bindable" => c.bindable = array_value(value),
            "unbindable" => c.unbindable = array_value(value),
            "final" => c.final_text = Some(string_value(value)),
            "templates" => c.templates = array_value(value),
            "diags" => c.diags = array_value(value),
            "divergence" => c.divergence = Some(string_value(value)),
            "sources" => c.sources = array_value(value),
            other => panic!("unknown key {other}"),
        }
    }
    cases
}

#[test]
fn param_goldens() {
    let cases = load_param_corpus();
    let want: BTreeSet<&str> = [
        "PARAM-01", "PARAM-02", "PARAM-03", "PARAM-04", "PARAM-05", "PARAM-06", "PARAM-12",
        "PARAM-13", "PARAM-14", "PARAM-15", "PARAM-18", "PARAM-19", "PARAM-20",
    ]
    .into_iter()
    .collect();
    let mut seen: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for c in &cases {
        seen.entry(c.id.as_str())
            .or_default()
            .insert(c.dialect.as_str());
        assert!(!c.sources.is_empty(), "{} names its sources", c.id);
        if let Some(d) = &c.divergence {
            assert!(!d.is_empty(), "{}", c.id);
        }
        let ctx = match c.dialect.as_str() {
            "postgres" => pg(),
            "mysql" => mysql(),
            other => panic!("{}: dialect {other}", c.id),
        };
        let settings = ParamSettings {
            pg_question_marks: c.pg_question_marks,
            ..ParamSettings::default()
        };
        let p = scan_params(&c.input, &ctx, &head_select(), &settings);
        let tag = format!("{} {} {:?}", c.id, c.dialect, c.input);
        assert_eq!(bindable_texts(&c.input, &p), c.bindable, "{tag}: bindable");
        assert_eq!(
            unbindable_texts(&c.input, &p),
            c.unbindable,
            "{tag}: unbindable"
        );
        assert_eq!(
            final_text(&p).map(str::to_string),
            c.final_text,
            "{tag}: final text"
        );
        let names: Vec<String> = p.templates.iter().map(|t| t.name.to_string()).collect();
        assert_eq!(names, c.templates, "{tag}: templates");
        assert_eq!(diag_names(&p), c.diags, "{tag}: diags");
    }
    let ids: BTreeSet<&str> = seen.keys().copied().collect();
    assert_eq!(ids, want, "the corpus rows 01–06, 12–15, 18–20");
    for (id, dialects) in &seen {
        assert!(
            dialects.contains("postgres") && dialects.contains("mysql"),
            "{id} runs for PostgreSQL and MySQL"
        );
    }
    // D-053 divergences are recorded: the template rows say why Hacivat refuses.
    for c in cases.iter().filter(|c| c.id == "PARAM-04") {
        assert!(
            c.divergence
                .as_deref()
                .is_some_and(|d| d.contains("refuse"))
        );
    }
}

// ---------------------------------------------------------------------------------------------
// The plan's named cases

#[test]
fn pg_colon_name_rewrites_to_dollar_with_reuse() {
    let s = scan_params(
        "SELECT :a, :b, :a",
        &pg(),
        &head_select(),
        &ParamSettings::default(),
    );
    let r = s.rewritten.unwrap();
    assert_eq!(r.text, "SELECT $1, $2, $1");
    assert_eq!(r.plan.names.len(), 2);
    assert_eq!(&*r.plan.names[0], "a");
    assert_eq!(&*r.plan.names[1], "b");
    assert_eq!(&r.plan.slots[0][..], &[0]);
    assert_eq!(&r.plan.slots[1][..], &[1]);
    assert_eq!(
        r.edits,
        [
            (
                TextRange::at(7.into(), 2.into()),
                TextRange::at(7.into(), 2.into())
            ),
            (
                TextRange::at(11.into(), 2.into()),
                TextRange::at(11.into(), 2.into())
            ),
            (
                TextRange::at(15.into(), 2.into()),
                TextRange::at(15.into(), 2.into())
            ),
        ]
    );
}

#[test]
fn mysql_repeats_a_name_per_slot() {
    let r = scan_params(
        "SELECT :a, :a",
        &mysql(),
        &head_select(),
        &ParamSettings::default(),
    )
    .rewritten
    .unwrap();
    assert_eq!(r.text, "SELECT ?, ?");
    assert_eq!(&r.plan.slots[0][..], &[0, 1]);
    assert_eq!(r.plan.names.len(), 1);
    // Each edit shrinks `:a` to `?`; later offsets shift by one per edit.
    assert_eq!(
        r.edits,
        [
            (
                TextRange::at(7.into(), 2.into()),
                TextRange::at(7.into(), 1.into())
            ),
            (
                TextRange::at(11.into(), 2.into()),
                TextRange::at(10.into(), 1.into())
            ),
        ]
    );
}

#[test]
fn false_positives_are_not_params() {
    for s in [
        "SELECT a::int",
        "SELECT a[1:2]",
        "SELECT '{}'::jsonb ?| array['a']",
        "SET @x := 1",
        "SELECT ':a'",
        "-- :a\nSELECT 1",
    ] {
        assert!(
            scan_params(
                s,
                &pg_or_mysql(s),
                &head_select(),
                &ParamSettings::default()
            )
            .placeholders
            .iter()
            .all(|p| !p.bindable),
            "{s}"
        );
    }
    // More of alg/10's edge cases, on PostgreSQL: none is a placeholder at all.
    for s in [
        "SELECT a[lo:hi] FROM t",
        "SELECT a[1:], a[:2], a[:] FROM t",
        "SELECT '{}'::jsonb ?& array['a'], '{}'::jsonb @> '{}'",
        "SELECT x FROM t WHERE y = 'it''s :q'",
        "/* :a ? $1 */ SELECT 1",
        "SELECT $$ :a $1 $$",
        "SELECT $f$ :a $f$",
        "SELECT E'\\' :a'",
        "SELECT \":a\" FROM t",
        "SELECT a:b FROM t",
        "DO $$ BEGIN PERFORM :x; END $$",
    ] {
        let p = scan(s, &pg());
        assert!(p.placeholders.is_empty(), "{s}: {:?}", p.placeholders);
        assert!(p.rewritten.is_none(), "{s}");
    }
    // `:x::int` is a parameter followed by a cast.
    let p = scan("SELECT :x::int", &pg());
    assert_eq!(final_text(&p), Some("SELECT $1::int"));
    // MySQL: `@a`, `@@x`, `:=`, a label and a backslash-escaped quote.
    for s in [
        "SELECT @a, @@sql_mode",
        "SELECT @x := 1",
        "SELECT 'a\\':b' FROM t",
        "SELECT `:a` FROM t",
        "SELECT \":a\" FROM t",
    ] {
        let p = scan(s, &mysql());
        assert!(p.placeholders.is_empty(), "{s}: {:?}", p.placeholders);
    }
}

#[test]
fn names_compare_exactly() {
    assert_eq!(
        scan_params(
            "SELECT :Id, :id",
            &pg(),
            &head_select(),
            &ParamSettings::default()
        )
        .rewritten
        .unwrap()
        .plan
        .names
        .len(),
        2
    );
    // Turkish dotted and dotless I never collapse.
    let r = scan("SELECT :İd, :id, :ıd, :Id", &pg()).rewritten.unwrap();
    assert_eq!(r.plan.names.len(), 4);
    assert_eq!(r.text, "SELECT $1, $2, $3, $4");
}

#[test]
fn pg_question_mark_is_off_by_default() {
    let s = "SELECT * FROM t WHERE a = ?";
    let p = scan(s, &pg());
    assert!(p.placeholders.is_empty());
    assert!(p.rewritten.is_none());
    assert_eq!(
        p.diags,
        [(
            TextRange::at(26.into(), 1.into()),
            ParamDiag::QuestionMarkOff
        )]
    );
    // The jsonb operators get no hint: they are operators wherever they sit.
    for s in [
        "SELECT j ? 'k' FROM t",
        "SELECT j ?| array['a'] FROM t",
        "SELECT j ?& array['a'] FROM t",
    ] {
        let p = scan(s, &pg());
        assert!(p.placeholders.is_empty() && p.diags.is_empty(), "{s}");
    }
    // Turned on: `?` in operand position binds, renumbered as `$k`.
    let on = ParamSettings {
        pg_question_marks: true,
        ..ParamSettings::default()
    };
    let p = scan_params(
        "SELECT ? , j ? 'k' FROM t WHERE a IN (?, ?)",
        &pg(),
        &head_select(),
        &on,
    );
    assert_eq!(
        final_text(&p),
        Some("SELECT $1 , j ? 'k' FROM t WHERE a IN ($2, $3)")
    );
    // An operator that only contains `?` (`=?`) is ambiguous once `?` binds: refused.
    let p = scan_params("SELECT * FROM t WHERE a =? 1", &pg(), &head_select(), &on);
    assert_eq!(p.placeholders.len(), 1);
    assert!(!p.placeholders[0].bindable);
    assert!(p.rewritten.is_none());
}

#[test]
fn ddl_placeholders_are_not_bindable() {
    for (s, ctx, head) in [
        (
            "CREATE TABLE t (id INT DEFAULT :value)",
            pg(),
            head_of(kw::CREATE),
        ),
        (
            "ALTER TABLE t ALTER c SET DEFAULT $1",
            pg(),
            head_of(kw::ALTER),
        ),
        ("DROP TABLE :t", mysql(), head_of(kw::DROP)),
        ("CREATE VIEW v AS SELECT ?", mysql(), head_of(kw::CREATE)),
        (
            "CREATE TABLE t (a DEFAULT :v)",
            sqlite(),
            head_of(kw::CREATE),
        ),
    ] {
        let p = scan_params(s, &ctx, &head, &ParamSettings::default());
        assert_eq!(p.placeholders.len(), 1, "{s}");
        assert!(!p.placeholders[0].bindable, "{s}");
        assert_eq!(diag_names(&p), ["ParamInDdl"], "{s}");
        assert!(p.rewritten.is_none(), "{s}");
    }
    // The head is also read from the unit itself: a caller's stale head cannot unlock DDL.
    let p = scan_params(
        "  -- c\n create table t (a int default :v)",
        &pg(),
        &head_select(),
        &ParamSettings::default(),
    );
    assert_eq!(diag_names(&p), ["ParamInDdl"]);
    // `in_ddl` turns binding on.
    let in_ddl = ParamSettings {
        in_ddl: true,
        ..ParamSettings::default()
    };
    let p = scan_params(
        "CREATE TABLE t (id INT DEFAULT :value)",
        &pg(),
        &head_of(kw::CREATE),
        &in_ddl,
    );
    assert_eq!(final_text(&p), Some("CREATE TABLE t (id INT DEFAULT $1)"));
}

#[test]
fn template_variable_is_detected_not_substituted() {
    let s = scan_params(
        "SELECT * FROM ${t}",
        &pg(),
        &head_select(),
        &ParamSettings::default(),
    );
    assert_eq!(s.templates.len(), 1);
    assert!(
        s.diags
            .iter()
            .any(|(_, d)| matches!(d, ParamDiag::TemplateVariable))
    );
    assert!(s.rewritten.is_none());
    // The three patterns, in operand position only; never in strings or comments.
    let s = "SELECT %name%, <id>, ${x y} FROM t WHERE a = <b> AND c = '%z%' -- ${q}";
    let p = scan(s, &pg());
    let got: Vec<(&str, TemplatePattern, &str)> = p
        .templates
        .iter()
        .map(|t| (&*t.name, t.pattern, text_of(s, t.range)))
        .collect();
    assert_eq!(
        got,
        [
            ("name", TemplatePattern::Percent, "%name%"),
            ("id", TemplatePattern::Angle, "<id>"),
            ("x y", TemplatePattern::DollarBrace, "${x y}"),
            ("b", TemplatePattern::Angle, "<b>"),
        ]
    );
    // Arithmetic and comparisons are not templates.
    for s in [
        "SELECT 10%n%3",
        "SELECT a %b% c FROM t",
        "SELECT * FROM t WHERE a<b>c",
        "SELECT * FROM t WHERE a < b AND c > d",
        "SELECT '${x}', \"${y}\"",
        "SELECT $$ ${x} $$",
    ] {
        assert!(scan(s, &pg()).templates.is_empty(), "{s}");
    }
    // A placeholder beside a template does not bind: the whole unit is refused.
    let p = scan("SELECT :a, ${b}", &pg());
    assert!(p.placeholders.iter().all(|x| !x.bindable));
    assert!(p.rewritten.is_none());
    // An empty name is kept as written (the dialog shows "(unnamed)").
    assert_eq!(&*scan("SELECT ${}", &pg()).templates[0].name, "");
}

// ---------------------------------------------------------------------------------------------
// Fail closed

#[test]
fn mixing_dollar_n_and_colon_names_is_refused() {
    let p = scan("SELECT $1, :a", &pg());
    assert_eq!(p.placeholders.len(), 2);
    assert!(p.placeholders.iter().all(|x| !x.bindable));
    assert!(p.rewritten.is_none());
    assert_eq!(
        p.diags,
        [(TextRange::at(11.into(), 2.into()), ParamDiag::MixedStyles)]
    );
    let p = scan("SELECT ?, :a", &mysql());
    assert_eq!(diag_names(&p), ["MixedStyles"]);
    assert!(p.rewritten.is_none());
}

#[test]
fn more_than_65535_binds_is_refused() {
    let many = |n: usize| {
        let mut s = String::from("SELECT ");
        for i in 0..n {
            if i > 0 {
                s.push(',');
            }
            s.push_str(&format!(":p{i}"));
        }
        s
    };
    let ok = scan(&many(65_535), &pg());
    assert!(ok.rewritten.is_some());
    assert_eq!(ok.rewritten.unwrap().plan.names.len(), 65_535);
    let p = scan(&many(65_536), &pg());
    assert!(p.rewritten.is_none());
    assert_eq!(diag_names(&p), ["TooManyBinds"]);
    // MySQL counts slots, not names.
    let s = format!("SELECT {}", vec![":a"; 65_536].join(","));
    let p = scan(&s, &mysql());
    assert_eq!(diag_names(&p), ["TooManyBinds"]);
    // `$70000` names a slot past the limit.
    assert_eq!(diag_names(&scan("SELECT $70000", &pg())), ["TooManyBinds"]);
}

#[test]
fn markers_the_server_reads_but_hacivat_cannot_bind_refuse_the_unit() {
    // A `?` or `:name` inside a MySQL executable comment is code to the server.
    for (s, ctx) in [
        ("SELECT 1 /*! , ? */", mysql()),
        ("SELECT :a /*!80000 , :b */", mysql()),
        ("SELECT ? /*M! , ? */", mariadb()),
    ] {
        let p = scan(s, &ctx);
        assert!(!p.placeholders.is_empty(), "{s}");
        assert!(p.placeholders.iter().all(|x| !x.bindable), "{s}");
        assert!(p.rewritten.is_none(), "{s}");
        assert!(diag_names(&p).contains(&"Unbindable".to_string()), "{s}");
    }
    // A hint comment is not code.
    assert!(scan("SELECT /*+ ? */ 1", &mysql()).placeholders.is_empty());
    // `?` in CALL (ODBC escape semantics).
    let p = scan_params(
        "CALL p(?)",
        &mysql(),
        &head_of(kw::CALL),
        &ParamSettings::default(),
    );
    assert_eq!(diag_names(&p), ["Unbindable"]);
    assert!(p.rewritten.is_none());
    // psql `:"name"` substitutes an identifier and `:{?name}` tests a variable: neither binds.
    for s in ["SELECT * FROM t WHERE :\"col\" = 1", "SELECT :{?v}"] {
        let p = scan(s, &pg());
        assert_eq!(p.placeholders.len(), 1, "{s}");
        assert!(!p.placeholders[0].bindable, "{s}");
        assert!(p.rewritten.is_none(), "{s}");
    }
    // `:'name'` is a value: it binds, the quotes included in the replaced range.
    let p = scan("SELECT :'v', :v", &pg());
    assert_eq!(final_text(&p), Some("SELECT $1, $1"));
    // `$0` and `$1abc` are not parameters the server accepts.
    for s in ["SELECT $0", "SELECT $1abc"] {
        let p = scan(s, &pg());
        assert!(p.placeholders.iter().all(|x| !x.bindable), "{s}");
        assert!(p.rewritten.is_none(), "{s}");
    }
    // SQLite binds every variable it reads (NULL when unbound): with `:name` turned off, a
    // `:a` is refused rather than sent.
    let off = ParamSettings {
        colon_params: false,
        ..ParamSettings::default()
    };
    let p = scan_params("SELECT :a", &sqlite(), &head_select(), &off);
    assert!(p.placeholders.iter().all(|x| !x.bindable));
    assert!(p.rewritten.is_none());
    // On PostgreSQL the server never reads `:a`: turned off, it is simply not a placeholder.
    assert!(
        scan_params("SELECT :a", &pg(), &head_select(), &off)
            .placeholders
            .is_empty()
    );
}

#[test]
fn a_replacement_never_merges_with_its_neighbours() {
    // `$12.5`, `SELECT$1` (one identifier), `$1abc`, SQLite `?1`: each would change what the
    // server reads, so the placeholder is refused instead.
    let on = ParamSettings {
        pg_question_marks: true,
        ..ParamSettings::default()
    };
    for (s, ctx, settings) in [
        ("SELECT :'v'2.5", pg(), ParamSettings::default()),
        ("SELECT :'v'abc", pg(), ParamSettings::default()),
        ("SELECT?", pg(), on),
        ("SELECT a FROM t WHERE x = ?b", pg(), on),
        ("SELECT $a(x)1", sqlite(), ParamSettings::default()),
    ] {
        let p = scan_params(s, &ctx, &head_select(), &settings);
        assert!(!p.placeholders.is_empty(), "{s}");
        assert!(p.placeholders.iter().all(|x| !x.bindable), "{s}");
        assert!(p.rewritten.is_none(), "{s}");
    }
    // Neighbouring punctuation is fine.
    let p = scan("SELECT (:a),:b;", &pg());
    assert_eq!(final_text(&p), Some("SELECT ($1),$2;"));
}

fn refused(p: &ParamScan) -> bool {
    !p.placeholders.is_empty()
        && p.placeholders.iter().all(|x| !x.bindable)
        && p.rewritten.is_none()
        && diag_names(p).contains(&"Unbindable".to_string())
}

/// Review I1: `$01` is the server's `$1`; two spellings would be two plan names for one slot.
#[test]
fn leading_zero_positionals_are_refused() {
    for (s, ctx) in [
        ("SELECT $1, $01", pg()),
        ("SELECT $01", pg()),
        ("SELECT ?1, ?01", sqlite()),
        ("SELECT ?007", sqlite()),
    ] {
        assert!(refused(&scan(s, &ctx)), "{s}");
    }
    // One spelling binds.
    let r = scan("SELECT $1, $1, $10", &pg()).rewritten.unwrap();
    let names: Vec<&str> = r.plan.names.iter().map(|n| &**n).collect();
    assert_eq!(names, ["$1", "$10"]);
}

/// Review I2: the lexer reads psql meta-commands as an unknown `\` and code; a unit holding an
/// unreadable or unterminated token binds nothing.
#[test]
fn unreadable_or_unterminated_tokens_refuse_the_unit() {
    for (s, ctx) in [
        ("\\set x :y", pg()),
        ("SELECT \\x :a", pg()),
        ("SELECT :a\\gset", pg()),
        ("SELECT :a \\g", pg()),
        ("SELECT :a\n\\gset", pg()),
        ("SELECT :a '", pg()),
        ("SELECT :a /* unterminated", pg()),
        ("SELECT :a, \"x", pg()),
        ("SELECT :a, $q$ x", pg()),
        ("SELECT ? '", mysql()),
        ("SELECT :a /*! x", mysql()),
        ("SELECT :a '", sqlite()),
    ] {
        let p = scan(s, &ctx);
        assert!(refused(&p), "{s}: {p:?}");
    }
    // The diagnostic points at the unreadable token.
    let p = scan("SELECT :a \\g", &pg());
    assert!(
        p.diags
            .contains(&(TextRange::at(10.into(), 1.into()), ParamDiag::Unbindable))
    );
}

/// Review M1: a `;` the server runs inside an executable comment starts a second statement.
#[test]
fn a_semicolon_inside_an_executable_comment_refuses_the_unit() {
    for (s, ctx) in [
        ("SELECT :a /*!; DROP TABLE t */", mysql()),
        ("SELECT ? /*!80000 ; DELETE FROM t */", mysql()),
        ("SELECT ? /*M! ;SELECT 1 */", mariadb()),
    ] {
        assert!(refused(&scan(s, &ctx)), "{s}");
    }
    // A `;` inside a string in the body is data: the unit still binds.
    let p = scan("SELECT :a /*! , ';' */", &mysql());
    assert_eq!(final_text(&p), Some("SELECT ? /*! , ';' */"));
}

/// Review M2/M3: once `?` binds on PostgreSQL, a `?` whose reading is ambiguous is refused.
#[test]
fn ambiguous_pg_question_marks_are_refused() {
    let on = ParamSettings {
        pg_question_marks: true,
        ..ParamSettings::default()
    };
    for s in [
        "SELECT * FROM t WHERE ? ? 'k'",
        "SELECT * FROM t WHERE (j) ? ?",
        "SELECT first ? 'k' FROM t",
        "SELECT next ? 'k', all ? 'k' FROM t",
        "SELECT * FROM t WHERE a = ?-1",
        "SELECT j ? ?| array['a'] FROM t",
        "SELECT * FROM t WHERE a = ? ?& array['a']",
    ] {
        let p = scan_params(s, &pg(), &head_select(), &on);
        assert!(refused(&p), "{s}: {p:?}");
    }
    // Unambiguous ones still bind.
    for (s, want) in [
        ("SELECT ?, ? FROM t", "SELECT $1, $2 FROM t"),
        (
            "SELECT * FROM t WHERE j ? 'k' AND a = ?",
            "SELECT * FROM t WHERE j ? 'k' AND a = $1",
        ),
        (
            "SELECT * FROM t WHERE a = ? - 1",
            "SELECT * FROM t WHERE a = $1 - 1",
        ),
        (
            "SELECT * FROM t LIMIT ? OFFSET ?",
            "SELECT * FROM t LIMIT $1 OFFSET $2",
        ),
    ] {
        let p = scan_params(s, &pg(), &head_select(), &on);
        assert_eq!(final_text(&p), Some(want), "{s}");
    }
}

/// Review M4: `EXPLAIN [ANALYZE]` of DDL is DDL.
#[test]
fn an_explain_of_ddl_is_ddl() {
    let explain = scan_params(
        "EXPLAIN ANALYZE CREATE TABLE t AS SELECT :a",
        &pg(),
        &head_of(kw::EXPLAIN),
        &ParamSettings::default(),
    );
    assert_eq!(diag_names(&explain), ["ParamInDdl"]);
    assert!(explain.rewritten.is_none());
    // Read from the unit too, whatever head the caller passes.
    let p = scan("explain (analyze, verbose) drop table t", &pg());
    assert!(p.placeholders.is_empty());
    let p = scan("EXPLAIN ALTER TABLE t ADD c int DEFAULT :v", &pg());
    assert_eq!(diag_names(&p), ["ParamInDdl"]);
    // An EXPLAIN of DML binds.
    let p = scan("EXPLAIN ANALYZE DELETE FROM t WHERE id = :a", &pg());
    assert_eq!(
        final_text(&p),
        Some("EXPLAIN ANALYZE DELETE FROM t WHERE id = $1")
    );
}

#[test]
fn a_unit_of_several_statements_never_binds() {
    // The head is the first statement's: a later `DROP` must not ride on a `SELECT`'s binds.
    for (s, ctx) in [
        ("SELECT :a; DROP TABLE t", pg()),
        ("SELECT ?; DELETE FROM t WHERE id = ?", mysql()),
        ("SELECT 1; SELECT :a", sqlite()),
    ] {
        let p = scan(s, &ctx);
        assert!(!p.placeholders.is_empty(), "{s}");
        assert!(p.placeholders.iter().all(|x| !x.bindable), "{s}");
        assert!(p.rewritten.is_none(), "{s}");
        assert!(diag_names(&p).contains(&"Unbindable".to_string()), "{s}");
    }
    // A trailing terminator (or several) is still one statement.
    assert_eq!(
        final_text(&scan("SELECT :a; ;", &pg())),
        Some("SELECT $1; ;")
    );
    assert_eq!(
        final_text(&scan("SELECT :a; -- c", &pg())),
        Some("SELECT $1; -- c")
    );
}

#[test]
fn copy_data_is_never_a_parameter() {
    let s = "COPY t FROM STDIN;\n:a\t?\t$1\n\\.\n";
    let p = scan_params(s, &pg(), &head_of(kw::COPY), &ParamSettings::default());
    assert!(p.placeholders.is_empty(), "{:?}", p.placeholders);
    // The head is read from the unit too, and a `COPY` (a utility statement) never binds.
    let p = scan("COPY t FROM STDIN;\n:a\n\\.\n", &pg());
    assert!(p.placeholders.is_empty(), "{:?}", p.placeholders);
    let p = scan("COPY (SELECT :a) TO STDOUT", &pg());
    assert_eq!(p.placeholders.len(), 1);
    assert!(!p.placeholders[0].bindable);
    assert_eq!(diag_names(&p), ["Unbindable"]);
}

#[test]
fn sqlite_and_mysql_rewrite_names_to_question_marks() {
    let s = "SELECT :a, @b, $c, :a";
    let r = scan(s, &sqlite()).rewritten.unwrap();
    assert_eq!(r.text, "SELECT ?, ?, ?, ?");
    let names: Vec<&str> = r.plan.names.iter().map(|n| &**n).collect();
    assert_eq!(names, ["a", "@b", "$c"]);
    assert_eq!(&r.plan.slots[0][..], &[0, 3]);
    // Positional-only units pass through unchanged, one slot per number.
    let r = scan("SELECT $2, $1, $2", &pg()).rewritten.unwrap();
    assert_eq!(r.text, "SELECT $2, $1, $2");
    assert!(r.edits.is_empty());
    let names: Vec<&str> = r.plan.names.iter().map(|n| &**n).collect();
    assert_eq!(names, ["$2", "$1"]);
    assert_eq!(&r.plan.slots[0][..], &[1]);
    assert_eq!(&r.plan.slots[1][..], &[0]);
    let r = scan("SELECT ?, ?", &mysql()).rewritten.unwrap();
    assert_eq!(r.text, "SELECT ?, ?");
    assert_eq!(r.plan.slots.len(), 2);
}

#[test]
fn rewrite_is_deterministic() {
    let s = "SELECT :b, :a, :b FROM t WHERE x = :c";
    let a = scan(s, &pg());
    let b = scan(s, &pg());
    assert_eq!(a, b);
    assert_eq!(
        final_text(&a),
        Some("SELECT $1, $2, $1 FROM t WHERE x = $3")
    );
}

// ## m3b-fuzz (controller fix): the nightly `params_rewrite` crash — a rewrite must be a
// fixpoint, so a name the rewrite turns into `?` under `CALL`/`EXEC`/`EXECUTE` (where a `?`
// never binds) is refused like the `?` itself.

/// A rewrite is final: scanning its text again binds every placeholder and changes nothing.
fn assert_rewrite_is_a_fixpoint(unit: &str, ctx: &DialectCtx, head: &StmtHead, s: &ParamSettings) {
    let p = scan_params(unit, ctx, head, s);
    let Some(r) = &p.rewritten else {
        return;
    };
    let again = scan_params(&r.text, ctx, head, s);
    assert_eq!(
        again.rewritten.map(|x| x.text),
        Some(r.text.clone()),
        "{:?} {unit:?}: the rewritten text rewrites again",
        ctx.id
    );
}

#[test]
fn fuzz_params_rewrite_names_under_exec_rewrite_to_a_refused_question_mark() {
    // The crash input (knob byte `U`: `?` binds on PG, `:name` on, DDL binds), every dialect.
    let knobs = ParamSettings {
        pg_question_marks: true,
        colon_params: true,
        in_ddl: true,
    };
    let crash = "exec($q$h$q$, $q$D1 */";
    for ctx in [pg(), mysql(), mariadb(), sqlite()] {
        assert_rewrite_is_a_fixpoint(crash, &ctx, &StmtHead::default(), &knobs);
    }
    // SQLite: `$q$h$q$` and `$q$D1` are two variables; under `exec` their `?` would not bind.
    let p = scan_params(crash, &sqlite(), &StmtHead::default(), &knobs);
    assert_eq!(unbindable_texts(crash, &p), ["$q$h$q$", "$q$D1"]);
    assert!(p.rewritten.is_none());
    assert!(diag_names(&p).contains(&"Unbindable".to_string()));

    // Close variants: names under `CALL`/`EXEC`/`EXECUTE` (head from the unit or the caller),
    // `$name$`-style SQLite variables, a stray `*/`, a `?` already present.
    let refused = [
        ("exec($q$h$q$, $q$D1 */", sqlite(), StmtHead::default()),
        ("EXEC p(:a)", sqlite(), StmtHead::default()),
        ("exec p(@a, #b, $c$)", sqlite(), StmtHead::default()),
        ("exec($a$) */", sqlite(), StmtHead::default()),
        ("exec(?, $a$)", sqlite(), StmtHead::default()),
        ("exec(?, ? */", sqlite(), StmtHead::default()),
        ("CALL p(:a)", mysql(), head_of(kw::CALL)),
        ("call p(:a, :b) */", mysql(), StmtHead::default()),
        ("EXEC p(:a)", mysql(), StmtHead::default()),
        ("EXECUTE s USING :a", mysql(), StmtHead::default()),
        ("CALL p(:a)", mariadb(), StmtHead::default()),
        ("p(:a)", mysql(), head_of(kw::CALL)),
    ];
    for (s, ctx, head) in &refused {
        for knobs in [knobs, ParamSettings::default()] {
            let p = scan_params(s, ctx, head, &knobs);
            assert!(!p.placeholders.is_empty(), "{:?} {s}", ctx.id);
            assert!(
                p.placeholders.iter().all(|x| !x.bindable),
                "{:?} {s}",
                ctx.id
            );
            assert!(p.rewritten.is_none(), "{:?} {s}", ctx.id);
            assert_rewrite_is_a_fixpoint(s, ctx, head, &knobs);
        }
    }
    // Outside `CALL`/`EXEC` the same variables bind, and the rewrite is final.
    let binds = [
        ("SELECT $q$h$q$, $q$D1 */", sqlite(), "SELECT ?, ? */"),
        ("SELECT $a$ */", sqlite(), "SELECT ? */"),
        ("SELECT :a, :b */", mysql(), "SELECT ?, ? */"),
        ("SELECT ?, ? */", sqlite(), "SELECT ?, ? */"),
        // PostgreSQL rewrites names to `$k`, which bind under `CALL`: nothing to refuse.
        ("CALL p(:a, :b, :a)", pg(), "CALL p($1, $2, $1)"),
        ("exec(:a) */", pg(), "exec($1) */"),
    ];
    for (s, ctx, want) in &binds {
        let p = scan_params(s, ctx, &StmtHead::default(), &knobs);
        assert_eq!(final_text(&p), Some(*want), "{:?} {s}", ctx.id);
        assert_rewrite_is_a_fixpoint(s, ctx, &StmtHead::default(), &knobs);
    }
}
// ## m3b-fuzz (controller fix) end

// ---------------------------------------------------------------------------------------------
// Properties

const PG_FRAGMENTS: &[&str] = &[
    "SELECT",
    "FROM",
    "WHERE",
    "t",
    "a",
    "b1",
    "1",
    "2.5",
    ",",
    "(",
    ")",
    "=",
    "+",
    "-",
    " ",
    " ",
    "\n",
    ":p",
    ":q",
    ":p",
    ":müşteri",
    ":Id",
    ":'v'",
    "a::int",
    "::",
    "arr[1:2]",
    "arr[lo:hi]",
    "'x:y'",
    "'it''s :q'",
    "-- :c ? $1\n",
    "/* :d ? */",
    "$$ :z $$",
    "$f$ :y $f$",
    "j ? 'k'",
    "?|",
    "@>",
    ":=",
    "\"q:x\"",
    "E'\\' :e'",
    "$",
    "$1",
    "${t}",
    "'",
    "\"",
    ";",
    "?",
    "2",
    "x$",
    ":\"v\"",
    "[",
    "]",
    "IN",
    "_",
    "/*",
    "*/",
    "$2",
    ":{?v}",
    // Fix round 1: leading zeros, a psql meta-command, a `?` beside a `?`-operator.
    "$01",
    "\\gset",
    "?-",
];

const MYSQL_FRAGMENTS: &[&str] = &[
    "SELECT",
    "FROM",
    "WHERE",
    "t",
    "a",
    "1",
    ",",
    "(",
    ")",
    "=",
    " ",
    " ",
    "\n",
    ":p",
    ":q",
    ":p",
    ":müşteri",
    "@v",
    "@@x",
    ":=",
    "'x:y'",
    "'a\\':b'",
    "# :h\n",
    "-- :c\n",
    "/* :d */",
    "/*+ :e */",
    "`:f`",
    "\":g\"",
    "?",
    "x:y",
    "'",
    ";",
    "/*! :p ? */",
    "2",
    "$x",
    "_utf8mb4'a'",
    "*/",
    ":",
];

const SQLITE_FRAGMENTS: &[&str] = &[
    "SELECT", "FROM", "t", "a", "1", ",", "(", ")", "=", " ", " ", "\n", ":p", ":q", ":p", "@p",
    "$p", "$a(x)", "#h", "?", "?2", "'x:y'", "-- :c\n", "/* :d */", "\":g\"", "[:i]", "2", "'",
    ";", "::", "?01", "?1",
];

// ## m3b-params-proptest: every dialect (MariaDB and the generic dialect too), every placeholder
// form and sigil in each, units that start with the statement words the scan treats specially,
// every settings knob and several caller heads.

/// The dialects the properties run on; a generated case's `d` indexes this.
const DIALECTS: [DialectId; 5] = [
    DialectId::Postgres,
    DialectId::MySql,
    DialectId::MariaDb,
    DialectId::Sqlite,
    DialectId::Generic,
];

/// Every placeholder form and sigil of every dialect, offered in each one: what one engine
/// binds is code, data or an error to another, and each reading must keep the invariants.
const SIGIL_FRAGMENTS: &[&str] = &[
    ":name",
    ":a",
    "@name",
    "@a",
    "$name",
    "$a",
    "#name",
    "#a",
    "$1",
    "$2",
    "$0",
    "$01",
    "$65536",
    "?",
    "?",
    "?1",
    "?2",
    "?01",
    "?0",
    "?65536",
    ":'v'",
    ":\"v\"",
    ":{?v}",
    "$q$",
    "$q$ :a ? $1 $q$",
    "$q$h$q$",
    "$$",
    "$$ :z ? $$",
    "$a$",
    "/*!",
    "/*! :p ? $1 */",
    "/*!50000 :p */",
    "/*M! :p */",
    "/*+ :h ? */",
    "/*",
    "*/",
    "--",
    "# :h\n",
    "\\gset",
    "\\set x :y\n",
    ":a\\gset",
    "\\",
    "${t}",
    "${",
    "%t%",
    "<t>",
    "{{t}}",
    "@@x",
    "::",
    ":=",
    "?|",
    "?-",
    "=?",
    "@?",
    "`:b`",
    "[:c]",
    "\":g\"",
    "'x:y'",
    "E'\\' :e'",
    "'",
    "\"",
    "`",
    ";",
    "CALL",
    "EXEC",
    "EXECUTE",
    "PREPARE",
    "COPY",
    "EXPLAIN",
    "CREATE",
    "STDIN",
    "USING",
];

/// Statement starts a unit may get: plain code, and the words after which the scan refuses
/// (`CALL`/`EXEC`/`EXECUTE`, `PREPARE`, `COPY`, an `EXPLAIN` of DDL) or reads DDL.
const HEADS: &[&str] = &[
    "",
    "SELECT ",
    "CALL p(",
    "call p(",
    "CALL ",
    "EXEC ",
    "exec(",
    "EXEC p ",
    "EXECUTE s USING ",
    "execute ",
    "PREPARE s AS SELECT ",
    "PREPARE s FROM ",
    "prepare ",
    "COPY t FROM STDIN;\n",
    "COPY (SELECT ",
    "copy t TO STDOUT ",
    "EXPLAIN ",
    "EXPLAIN ANALYZE SELECT ",
    "explain CREATE TABLE t AS SELECT ",
    "CREATE TABLE t AS SELECT ",
    "INSERT INTO t VALUES (",
];

/// A generated case: a unit in a dialect, with settings and the caller's head.
#[derive(Debug, Clone)]
struct PropCase {
    /// An index into [`DIALECTS`].
    d: usize,
    settings: ParamSettings,
    head: StmtHead,
    /// The index into [`HEADS`] the unit starts with.
    start: usize,
    sql: String,
}

impl PropCase {
    fn ctx(&self) -> DialectCtx {
        DialectCtx::new(DIALECTS[self.d], None)
    }
}

/// The dialect's own fragments.
fn fragments_of(d: DialectId) -> &'static [&'static str] {
    match d {
        DialectId::Postgres => PG_FRAGMENTS,
        DialectId::MySql | DialectId::MariaDb => MYSQL_FRAGMENTS,
        DialectId::Sqlite => SQLITE_FRAGMENTS,
        _ => GENERIC_FRAGMENTS,
    }
}

/// The generic dialect reads standard SQL only: `:name` and `?` bind, the sigils probe the rest.
const GENERIC_FRAGMENTS: &[&str] = &[
    "SELECT",
    "FROM",
    "WHERE",
    "t",
    "a",
    "1",
    ",",
    "(",
    ")",
    "=",
    " ",
    " ",
    "\n",
    ":p",
    ":q",
    ":p",
    ":Id",
    "?",
    "'x:y'",
    "'it''s :q'",
    "-- :c ?\n",
    "/* :d ? */",
    "\"q:x\"",
    "a::int",
    "arr[1:2]",
    "2",
    "'",
    ";",
    "_",
    "IN",
];

/// `frag` alone lexes cleanly in `d` and holds no `;`. A unit with a token the lexer cannot
/// read cleanly (or of several statements) binds nothing, so half the units are built of clean
/// fragments only: else almost every unit is refused and few rewrites are checked.
fn clean_in(d: DialectId, frag: &str) -> bool {
    let ctx = DialectCtx::new(d, None);
    let profile = sql_dialect::lex_profile(&ctx).unwrap();
    let l = lex(frag, profile, ctx.modes, &LexState::Initial);
    (0..l.len()).all(|i| {
        !matches!(l.kind(i), TokenKind::Unknown | TokenKind::Semicolon)
            && !l
                .flags(i)
                .intersects(TokenFlags::ERROR | TokenFlags::JUNK | TokenFlags::UNTERMINATED)
    })
}

/// A generated unit: an optional statement start, then fragments of its dialect and sigils
/// (about one in seven; in half the units clean ones only), with every settings combination
/// and a caller head (none, `SELECT`, `CALL`, `EXECUTE`).
fn sql_with_params() -> impl Strategy<Value = PropCase> {
    (
        0..DIALECTS.len(),
        0u8..8,
        0u8..8,
        (0..HEADS.len() * 3),
        any::<bool>(),
        proptest::collection::vec(0usize..4096, 0..28),
    )
        .prop_map(|(d, knobs, head, start, clean, picks)| {
            let own = fragments_of(DIALECTS[d]);
            // A third of the units start with a statement word, the rest with a fragment.
            let start = if start < HEADS.len() { start } else { 0 };
            let picks = &picks[..picks.len().min(if start == 0 { 20 } else { 10 })];
            let mut s = String::from(HEADS[start]);
            for (i, p) in picks.iter().enumerate() {
                let frag = if p % 7 == 0 {
                    SIGIL_FRAGMENTS[(p / 7) % SIGIL_FRAGMENTS.len()]
                } else {
                    own[(p / 7) % own.len()]
                };
                if clean && !clean_in(DIALECTS[d], frag) {
                    continue;
                }
                if i > 0 && p % 4 == 1 {
                    s.push(' ');
                }
                s.push_str(frag);
            }
            let settings = ParamSettings {
                pg_question_marks: knobs & 1 != 0,
                colon_params: knobs & 2 == 0,
                in_ddl: knobs & 4 != 0,
            };
            let head = match head {
                0..3 => StmtHead::default(),
                3..6 => head_select(),
                6 => head_of(kw::CALL),
                _ => head_of(kw::EXECUTE),
            };
            PropCase {
                d,
                settings,
                head,
                start,
                sql: s,
            }
        })
}
// ## m3b-params-proptest end

/// The texts of the tokens of `text` that overlap none of `holes`.
fn tokens_outside(ctx: &DialectCtx, text: &str, holes: &[TextRange]) -> Vec<String> {
    let profile = sql_dialect::lex_profile(ctx).unwrap();
    let l = lex(text, profile, ctx.modes, &LexState::Initial);
    (0..l.len())
        .filter(|i| {
            let r = l.range(*i);
            holes
                .iter()
                .all(|h| h.end() <= r.start() || r.end() <= h.start())
        })
        .map(|i| text_of(text, l.range(i)).to_string())
        .collect()
}

// ## m3b-params-proptest: `PROPTEST_CASES` raises the count (a local or nightly run).
/// The proptest case count: `PROPTEST_CASES` when set, else 8,192 (PR).
fn cases() -> u32 {
    std::env::var("PROPTEST_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(8192)
}

// ## m3b-params-proptest: the properties' checks, shared by the generated units and the
// statements the execution split finds in them (each with its own head, as the fuzz target).

/// The rewrite replaces only bindable placeholders, copies every other byte, and is final.
fn check_rewrite(
    sql: &str,
    ctx: &DialectCtx,
    head: &StmtHead,
    settings: &ParamSettings,
) -> Result<(), TestCaseError> {
    let pg = ctx.id == DialectId::Postgres;
    let p = scan_params(sql, ctx, head, settings);
    if let Some(r) = &p.rewritten {
        // Edits are ordered, disjoint, each one a bindable placeholder's range, and the
        // bytes between them are copied unchanged.
        let mut src_at = 0usize;
        let mut dst_at = 0usize;
        for (from, to) in &r.edits {
            let (fs, fe) = (usize::from(from.start()), usize::from(from.end()));
            let (ts, te) = (usize::from(to.start()), usize::from(to.end()));
            prop_assert!(fs >= src_at && ts >= dst_at);
            prop_assert_eq!(&sql[src_at..fs], &r.text[dst_at..ts]);
            prop_assert!(
                p.placeholders
                    .iter()
                    .any(|x| x.bindable && x.range == *from)
            );
            let rep = &r.text[ts..te];
            if pg {
                prop_assert!(
                    rep.starts_with('$') && rep[1..].parse::<u32>().is_ok(),
                    "{rep}"
                );
            } else {
                prop_assert_eq!(rep, "?");
            }
            src_at = fe;
            dst_at = te;
        }
        prop_assert_eq!(&sql[src_at..], &r.text[dst_at..]);
        // Every token outside a placeholder lexes the same, byte for byte.
        let from: Vec<TextRange> = r.edits.iter().map(|e| e.0).collect();
        let to: Vec<TextRange> = r.edits.iter().map(|e| e.1).collect();
        prop_assert_eq!(
            tokens_outside(ctx, sql, &from),
            tokens_outside(ctx, &r.text, &to)
        );
        // Every replacement lexes as one placeholder token of the engine.
        let profile = sql_dialect::lex_profile(ctx).unwrap();
        let l = lex(&r.text, profile, ctx.modes, &LexState::Initial);
        for t in &to {
            let i = l.token_at(t.start()).unwrap();
            prop_assert_eq!(l.range(i), *t);
            let want = if pg {
                TokenKind::PositionalParam
            } else {
                TokenKind::QuestionParam
            };
            prop_assert_eq!(l.kind(i), want);
        }
        // Scanning the rewritten text again finds exactly the engine's placeholders, one per
        // replaced or kept placeholder, all bindable, and rewrites nothing more.
        let again = scan_params(&r.text, ctx, head, settings);
        prop_assert_eq!(again.placeholders.len(), p.placeholders.len());
        prop_assert!(again.placeholders.iter().all(|x| x.bindable));
        prop_assert_eq!(again.rewritten.map(|x| x.text), Some(r.text.clone()));
    } else {
        // Nothing is rewritten: either there is nothing to bind or the unit is refused.
        prop_assert!(p.placeholders.is_empty() || p.placeholders.iter().all(|x| !x.bindable));
    }
    // A refused unit says why.
    if p.placeholders.iter().any(|x| !x.bindable) {
        prop_assert!(!p.diags.is_empty());
        prop_assert!(p.placeholders.iter().all(|x| !x.bindable));
    }
    Ok(())
}

/// The bind plan covers each bindable placeholder once.
fn check_plan(
    sql: &str,
    ctx: &DialectCtx,
    head: &StmtHead,
    settings: &ParamSettings,
) -> Result<(), TestCaseError> {
    let pg = ctx.id == DialectId::Postgres;
    let p = scan_params(sql, ctx, head, settings);
    let Some(r) = &p.rewritten else {
        return Ok(());
    };
    prop_assert_eq!(r.plan.names.len(), r.plan.slots.len());
    // Names are distinct.
    let distinct: BTreeSet<&str> = r.plan.names.iter().map(|n| &**n).collect();
    prop_assert_eq!(distinct.len(), r.plan.names.len());
    // Slots are distinct; without `$n`/`?NNN` they are dense, 0..n.
    let mut all: Vec<u32> = r
        .plan
        .slots
        .iter()
        .flat_map(|s| s.iter().copied())
        .collect();
    all.sort_unstable();
    let n = u32::try_from(all.len()).unwrap();
    let mut dedup = all.clone();
    dedup.dedup();
    prop_assert_eq!(&dedup, &all);
    let positional = p
        .placeholders
        .iter()
        .any(|x| matches!(x.kind, PlaceholderKind::Positional(_)));
    if !positional {
        prop_assert_eq!(&all, &(0..n).collect::<Vec<_>>());
    }
    // Each bindable placeholder is covered by exactly one name, and its replacement (or its
    // own `$n`/`?`) names a slot of that name.
    for x in p.placeholders.iter().filter(|x| x.bindable) {
        let name = match &x.kind {
            PlaceholderKind::Named(n) => n.to_string(),
            PlaceholderKind::Question(q) => format!("?{q}"),
            PlaceholderKind::Positional(_) => text_of(sql, x.range).to_string(),
        };
        let owners: Vec<usize> = r
            .plan
            .names
            .iter()
            .enumerate()
            .filter(|(_, n)| ***n == *name)
            .map(|(i, _)| i)
            .collect();
        prop_assert_eq!(owners.len(), 1, "{}", name);
        let slots = &r.plan.slots[owners[0]];
        match r.edits.iter().position(|(from, _)| *from == x.range) {
            Some(e) => {
                let rep = text_of(&r.text, r.edits[e].1);
                if pg {
                    let k: u32 = rep[1..].parse().unwrap();
                    prop_assert!(slots.contains(&(k - 1)));
                } else {
                    prop_assert!(slots.contains(&u32::try_from(e).unwrap()));
                }
            }
            None => match &x.kind {
                PlaceholderKind::Positional(k) => prop_assert_eq!(&slots[..], &[k - 1]),
                PlaceholderKind::Question(q) => prop_assert_eq!(&slots[..], &[q - 1]),
                PlaceholderKind::Named(_) => prop_assert!(false, "a name is always replaced"),
            },
        }
    }
    // MySQL and SQLite: one slot per occurrence.
    if !pg && !positional {
        prop_assert_eq!(n as usize, p.placeholders.len());
    }
    Ok(())
}

proptest! {
    #![proptest_config(ProptestConfig { cases: cases(), ..ProptestConfig::default() })]

    #[test]
    fn rewrite_preserves_every_other_byte(case in sql_with_params()) {
        check_rewrite(&case.sql, &case.ctx(), &case.head, &case.settings)?;
    }

    #[test]
    fn bind_plan_covers_each_placeholder_once(case in sql_with_params()) {
        check_plan(&case.sql, &case.ctx(), &case.head, &case.settings)?;
    }

    /// Each statement of the execution split, scanned with the head the splitter gives it.
    #[test]
    fn split_statements_keep_the_properties(case in sql_with_params()) {
        let ctx = case.ctx();
        let split = split_from(
            &case.sql,
            &ctx,
            ClientFlavor::None,
            SplitMode::Execution,
            &SplitStart::default(),
        );
        for st in split.statements() {
            let unit = &case.sql[st.strict];
            check_rewrite(unit, &ctx, &st.head, &case.settings)?;
            check_plan(unit, &ctx, &st.head, &case.settings)?;
        }
    }
}
// ## m3b-params-proptest end

/// The properties are not vacuous: the generator yields rewritten units with edits in every
/// dialect, refused units, and units with nothing to bind; every statement start is generated,
/// and the starts the scan refuses under (`CALL`/`EXEC`/`EXECUTE`, `PREPARE`, `COPY`) carry
/// placeholders.
#[test]
fn generator_reaches_every_outcome() {
    use proptest::strategy::ValueTree;
    use proptest::test_runner::TestRunner;
    let mut runner = TestRunner::deterministic();
    let strategy = sql_with_params();
    let mut edited = [0u32; DIALECTS.len()];
    let mut starts = vec![0u32; HEADS.len()];
    let (mut refused, mut nothing) = (0u32, 0u32);
    for _ in 0..32_768 {
        let case = strategy.new_tree(&mut runner).unwrap().current();
        let p = scan_params(&case.sql, &case.ctx(), &case.head, &case.settings);
        if !p.placeholders.is_empty() {
            starts[case.start] += 1;
        }
        match &p.rewritten {
            Some(r) if !r.edits.is_empty() => edited[case.d] += 1,
            Some(_) => {}
            None if p.placeholders.is_empty() => nothing += 1,
            None => refused += 1,
        }
    }
    assert!(edited.iter().all(|n| *n >= 100), "{edited:?}");
    assert!(starts.iter().all(|n| *n >= 50), "{starts:?}");
    assert!(refused >= 100 && nothing >= 100, "{refused} {nothing}");
}

/// alg/20 HV-EXEC-03 budget: ≤ 1 ms per 10 KB unit (release, `HACIVAT_PERF=1`; the GC perf
/// line runs `-p sql-split -- --ignored`).
#[test]
#[ignore = "perf"]
fn scan_params_10kb_unit_p95_under_1ms() {
    if std::env::var_os("HACIVAT_PERF").is_none() {
        return;
    }
    let mut unit = String::from("SELECT ");
    let mut i = 0;
    while unit.len() < 10 * 1024 {
        unit.push_str(&format!(
            "a{i} = :p{} AND b::int = $x$ :q $x$ AND c ? 'k:{i}' -- :z\n OR ",
            i % 50
        ));
        i += 1;
    }
    unit.push('1');
    for (ctx, s) in [(pg(), unit.clone()), (mysql(), unit.replace("$x$", "'"))] {
        let mut times: Vec<std::time::Duration> = (0..200)
            .map(|_| {
                let t = std::time::Instant::now();
                let p = scan(&s, &ctx);
                let e = t.elapsed();
                assert!(!p.placeholders.is_empty());
                e
            })
            .collect();
        times.sort();
        let p95 = times[189];
        eprintln!("scan_params 10 KiB p95 {p95:?}");
        assert!(p95 <= std::time::Duration::from_millis(1), "p95 {p95:?}");
    }
}
