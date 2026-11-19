//! M4 Task 6: the catalog effects of each statement (`StmtReport.ddl`, the input of
//! HV-INTRO-12's refresh planning), on both backends, and the proof that filling them changed
//! no verdict.
//!
//! - The named cases of the plan's test list, then the data-driven corpus
//!   `sql-testkit/corpus/ddl_effects/{pg,mysql,sqlite}.toml`.
//! - `ddl_effects_never_change_the_verdict`: every statement of the existing corpora (the read
//!   corpus, the escaped literals, the SQL-SAFE-06 bypass catalogue, the classifier seeds, the
//!   T10 run-A rows and this task's corpus) is classified and its report, `ddl` left out, is
//!   compared with the golden `corpus/ddl_effects/verdict-golden.tsv`, written once from the
//!   `m4` base before this task changed a line of `src/` (`UPDATE_GOLDEN=1`).

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::PathBuf;

use serde::Deserialize;
use sql_dialect::{ClientFlavor, DialectCtx, DialectId, PgWireFlavor};
use sql_ident::{Ident, QualifiedName, parse_qualified};
use sql_safety::{
    ClassifyCtx, DdlAction, DdlEffect, DdlKind, Expect, SafetyReport, StmtReport, classify,
};

// -- helpers -----------------------------------------------------------------------------------

fn ctx(d: DialectId) -> ClassifyCtx {
    ClassifyCtx::new(DialectCtx::new(d, None), ClientFlavor::None, Expect::Script)
}

fn one(d: DialectId, sql: &str) -> StmtReport {
    let r = classify(sql, &ctx(d));
    assert!(r.pre_decision.is_none(), "{sql}: {:?}", r.pre_decision);
    assert_eq!(r.items.len(), 1, "{sql}");
    r.items.into_iter().next().unwrap()
}

fn pg(sql: &str) -> StmtReport {
    one(DialectId::Postgres, sql)
}

fn my(sql: &str) -> StmtReport {
    one(DialectId::MySql, sql)
}

fn sqlite(sql: &str) -> StmtReport {
    one(DialectId::Sqlite, sql)
}

fn qn(text: &str) -> QualifiedName {
    parse_qualified(DialectId::Postgres, text).unwrap()
}

/// One named effect with nothing else set.
fn eff(action: DdlAction, kind: DdlKind, name: &str) -> DdlEffect {
    DdlEffect {
        action,
        kind,
        name: Some(qn(name)),
        on: None,
        referenced: Vec::new(),
        temp: false,
        concurrently: false,
        opaque: false,
    }
}

/// A name as written: parts joined by `.`, a quoted part in `"…"` with `"` doubled (the
/// corpus notation; the plan's `to_string()`: `QualifiedName` has no `Display`).
fn show(name: &QualifiedName) -> String {
    name.parts
        .iter()
        .map(show_ident)
        .collect::<Vec<_>>()
        .join(".")
}

fn show_ident(i: &Ident) -> String {
    if i.quoted {
        format!("\"{}\"", i.text.replace('"', "\"\""))
    } else {
        i.text.to_string()
    }
}

/// An effect in the corpus notation (`corpus/ddl_effects/pg.toml`'s header).
fn render(e: &DdlEffect) -> String {
    if e.opaque {
        assert_eq!(
            *e,
            DdlEffect::opaque(),
            "an opaque effect names nothing: {e:?}"
        );
        return "opaque".to_string();
    }
    let action = match &e.action {
        DdlAction::Create => "create".to_string(),
        DdlAction::Alter => "alter".to_string(),
        DdlAction::Drop { cascade: false } => "drop".to_string(),
        DdlAction::Drop { cascade: true } => "drop-cascade".to_string(),
        DdlAction::Rename { to } => format!("rename-to:{}", show_ident(to)),
        DdlAction::SetSchema { to } => format!("set-schema:{}", show_ident(to)),
        DdlAction::Comment => "comment".to_string(),
        DdlAction::Grant => "grant".to_string(),
        DdlAction::Revoke => "revoke".to_string(),
        DdlAction::Other => "other".to_string(),
    };
    let kind = match e.kind {
        DdlKind::Database => "database",
        DdlKind::Schema => "schema",
        DdlKind::Table => "table",
        DdlKind::View => "view",
        DdlKind::MaterializedView => "matview",
        DdlKind::Index => "index",
        DdlKind::Sequence => "sequence",
        DdlKind::Routine => "routine",
        DdlKind::Trigger => "trigger",
        DdlKind::Type => "type",
        DdlKind::Domain => "domain",
        DdlKind::Extension => "extension",
        DdlKind::Policy => "policy",
        DdlKind::Rule => "rule",
        DdlKind::Event => "event",
        DdlKind::Role => "role",
        DdlKind::Other => "other",
    };
    let mut out = format!(
        "{action} {kind} {}",
        e.name.as_ref().map_or_else(|| "-".to_string(), show)
    );
    if let Some(on) = &e.on {
        let _ = write!(out, " on={}", show(on));
    }
    if !e.referenced.is_empty() {
        let refs: Vec<String> = e.referenced.iter().map(show).collect();
        let _ = write!(out, " refs={}", refs.join(","));
    }
    if e.temp {
        out.push_str(" temp");
    }
    if e.concurrently {
        out.push_str(" concurrently");
    }
    out
}

fn rendered(r: &StmtReport) -> Vec<String> {
    r.ddl.iter().map(render).collect()
}

// -- the plan's named cases ----------------------------------------------------------------------

#[test]
fn create_table_gives_one_create_table_effect() {
    let r = pg("create table s1.t(x int)");
    assert_eq!(r.ddl, [eff(DdlAction::Create, DdlKind::Table, "s1.t")]);
}

#[test]
fn create_index_names_its_table() {
    let mut want = eff(DdlAction::Create, DdlKind::Index, "i");
    want.on = Some(qn("s.t"));
    assert_eq!(pg("create index i on s.t (a)").ddl, [want.clone()]);
    assert_eq!(my("CREATE INDEX i ON s.t (a)").ddl, [want.clone()]);
    assert_eq!(sqlite("CREATE INDEX i ON t (a)").ddl[0].on, Some(qn("t")));
}

#[test]
fn pg_drop_index_has_no_table() {
    assert_eq!(pg("drop index i").ddl[0].on, None);
    assert_eq!(
        pg("drop index i").ddl,
        [eff(DdlAction::Drop { cascade: false }, DdlKind::Index, "i")]
    );
    // MySQL names it.
    assert_eq!(my("DROP INDEX i ON t").ddl[0].on, Some(qn("t")));
}

#[test]
fn create_index_concurrently_flagged() {
    let e = &pg("create index concurrently i on t (a)").ddl[0];
    assert!(e.concurrently && !e.opaque);
    assert!(!pg("create index i on t (a)").ddl[0].concurrently);
}

#[test]
fn alter_add_fk_lists_referenced() {
    let e = &pg("alter table t add foreign key (a) references s.u(b)").ddl[0];
    assert_eq!(e.action, DdlAction::Alter);
    assert_eq!(e.name, Some(qn("t")));
    assert_eq!(e.referenced, [qn("s.u")]);
    let e = &my("ALTER TABLE t ADD FOREIGN KEY (a) REFERENCES u (b)").ddl[0];
    assert_eq!(e.referenced, [qn("u")]);
}

#[test]
fn comment_on_column_targets_its_table() {
    let e = &pg("comment on column s.t.c is 'x'").ddl[0];
    assert_eq!(e.action, DdlAction::Comment);
    assert_eq!(e.on, Some(qn("s.t")));
    assert_eq!(e.name, Some(qn("c")));
}

#[test]
fn grant_on_all_tables_in_schema_targets_the_schema() {
    let r = pg("grant select on all tables in schema s to r");
    assert_eq!(r.ddl, [eff(DdlAction::Grant, DdlKind::Schema, "s")]);
}

#[test]
fn ctas_and_select_into_create() {
    for s in ["create table t as select 1", "select 1 into t"] {
        assert_eq!(
            pg(s).ddl,
            [eff(DdlAction::Create, DdlKind::Table, "t")],
            "{s}"
        );
    }
}

#[test]
fn mysql_rename_table_multi_gives_two_renames() {
    let r = my("RENAME TABLE a TO b, c TO d");
    assert_eq!(r.ddl.len(), 2);
    assert_eq!(rendered(&r), ["rename-to:b table a", "rename-to:d table c"]);
}

#[test]
fn mysql_drop_table_list_gives_three() {
    assert_eq!(my("DROP TABLE a, b, c").ddl.len(), 3);
}

#[test]
fn drop_schema_cascade() {
    assert_eq!(
        pg("drop schema s cascade").ddl,
        [eff(DdlAction::Drop { cascade: true }, DdlKind::Schema, "s")]
    );
}

#[test]
fn create_database() {
    assert_eq!(
        pg("create database d").ddl,
        [eff(DdlAction::Create, DdlKind::Database, "d")]
    );
    // A MySQL database is the model's schema.
    assert_eq!(
        my("CREATE DATABASE d").ddl,
        [eff(DdlAction::Create, DdlKind::Schema, "d")]
    );
}

#[test]
fn alter_type_targets_the_type() {
    for s in [
        "alter type s.mood add value 'x'",
        "alter type s.mood owner to r",
    ] {
        assert_eq!(
            pg(s).ddl,
            [eff(DdlAction::Alter, DdlKind::Type, "s.mood")],
            "{s}"
        );
    }
}

#[test]
fn set_schema_carries_the_new_schema() {
    let e = &pg("alter table s.t set schema s2").ddl[0];
    assert_eq!(e.name, Some(qn("s.t")));
    assert_eq!(
        e.action,
        DdlAction::SetSchema {
            to: Ident {
                text: "s2".into(),
                quoted: false
            }
        }
    );
}

#[test]
fn temp_objects_flagged_temp() {
    for s in [
        "create temp table t(a int)",
        "create temporary table t(a int)",
    ] {
        assert!(pg(s).ddl[0].temp, "{s}");
    }
    assert!(sqlite("create table temp.t(a)").ddl[0].temp);
    assert!(my("CREATE TEMPORARY TABLE t (a int)").ddl[0].temp);
    assert!(!pg("create table t(a int)").ddl[0].temp);
}

#[test]
fn do_call_execute_are_opaque() {
    for s in [
        "do $$ begin end $$",
        "call p()",
        "execute s",
        "prepare p as select 1",
    ] {
        assert_eq!(pg(s).ddl, [DdlEffect::opaque()], "{s}");
    }
    for s in ["CALL p()", "EXECUTE s", "PREPARE s FROM 'DROP TABLE t'"] {
        assert_eq!(my(s).ddl, [DdlEffect::opaque()], "{s}");
    }
}

#[test]
fn drop_owned_and_reassign_owned_are_opaque() {
    for s in [
        "drop owned by r",
        "drop owned by r cascade",
        "reassign owned by r to s",
    ] {
        assert!(
            pg(s).ddl.iter().all(|e| e.opaque) && !pg(s).ddl.is_empty(),
            "{s}"
        );
    }
}

#[test]
fn mysql_executable_comment_ddl_is_opaque() {
    assert!(my("/*!50000 CREATE TABLE z(a int) */").ddl[0].opaque);
}

#[test]
fn unparseable_ddl_is_opaque() {
    assert_eq!(pg("create table t (a int,,)").ddl, [DdlEffect::opaque()]);
    assert_eq!(my("CREATE TABLE t (a int,,)").ddl, [DdlEffect::opaque()]);
    assert_eq!(
        sqlite("CREATE TABLE t (a int,,)").ddl,
        [DdlEffect::opaque()]
    );
}

#[test]
fn dml_and_select_have_no_ddl() {
    for s in [
        "select 1",
        "insert into t values (1)",
        "update t set a = 1 where id = 1",
        "delete from t where id = 1",
    ] {
        assert!(pg(s).ddl.is_empty(), "{s}");
        assert!(my(s).ddl.is_empty(), "{s}");
        assert!(sqlite(s).ddl.is_empty(), "{s}");
    }
}

#[test]
fn quoted_mixed_case_names_kept_exact() {
    let e = &pg(r#"alter table "Sales"."Ürün" rename to "Ürünler""#).ddl[0];
    assert_eq!(show(e.name.as_ref().unwrap()), r#""Sales"."Ürün""#);
    assert_eq!(
        e.action,
        DdlAction::Rename {
            to: Ident {
                text: "Ürünler".into(),
                quoted: true
            }
        }
    );
    // Unquoted names are never folded (PostgreSQL would fold `S1.T` to `s1.t`; the consumer
    // folds with `sql_ident::fold`).
    let e = &pg("CREATE TABLE S1.T (x int)").ddl[0];
    assert_eq!(show(e.name.as_ref().unwrap()), "S1.T");
    assert!(e.name.as_ref().unwrap().parts.iter().all(|p| !p.quoted));
}

/// U-3 (controller ruling): TRUNCATE changes no catalog object; the named exception to "a DDL
/// statement yields an effect or is opaque".
#[test]
fn truncate_and_truncate_cascade_have_no_ddl_effect() {
    for s in [
        "truncate t",
        "truncate table a, b cascade",
        "truncate t restart identity",
    ] {
        assert!(pg(s).ddl.is_empty(), "{s}");
    }
    for s in ["TRUNCATE TABLE t", "TRUNCATE t"] {
        assert!(my(s).ddl.is_empty(), "{s}");
    }
}

/// T1c review H1 (carry): these never come back empty.
#[test]
fn review_h1_statements_are_never_empty() {
    let cases = [
        (DialectId::MySql, "PREPARE s FROM 'DROP TABLE t'"),
        (
            DialectId::MySql,
            "CREATE EVENT e ON SCHEDULE EVERY 1 HOUR DO DELETE FROM t",
        ),
        (DialectId::MySql, "ALTER USER 'u'@'%' ACCOUNT LOCK"),
        (DialectId::MySql, "RENAME USER 'a'@'%' TO 'b'@'%'"),
        (DialectId::MariaDb, "ALTER USER 'u'@'%' ACCOUNT LOCK"),
        (
            DialectId::Postgres,
            "EXPLAIN ANALYZE CREATE TABLE t3 AS SELECT 1",
        ),
        (DialectId::Postgres, "EXPLAIN (ANALYZE) SELECT 1 INTO t4"),
        (DialectId::Sqlite, "ATTACH DATABASE 'other.db' AS other"),
        (DialectId::Sqlite, "DETACH DATABASE other"),
    ];
    for (d, sql) in cases {
        let r = one(d, sql);
        assert!(
            r.ddl.first().is_some_and(|e| e.opaque),
            "{d:?}: {sql}: {:?}",
            r.ddl
        );
    }
}

/// A statement after one that may change what names mean (here: DDL) is classified against the
/// old context: its effects stay, and an opaque one is added (refresh more, never less).
#[test]
fn a_tainted_statement_keeps_its_names_and_is_opaque_too() {
    let r = classify(
        "create table t(a int); create index i on t (a)",
        &ctx(DialectId::Postgres),
    );
    assert_eq!(rendered(&r.items[0]), ["create table t"]);
    assert_eq!(rendered(&r.items[1]), ["opaque", "create index i on=t"]);
}

/// The PostgreSQL scanner truncates names to 63 bytes; the effect keeps the name as written.
#[test]
fn long_names_are_kept_as_written() {
    let long = "a".repeat(70);
    let e = &pg(&format!("create table {long}(x int)")).ddl[0];
    assert_eq!(show(e.name.as_ref().unwrap()), long);
}

/// A name written with `U&"…"` escapes is not re-read from the text: it is the server's
/// reading of it, quoted (exact identity, never folded twice).
#[test]
fn unicode_escaped_names_are_the_servers_reading() {
    let e = &pg(r#"create table U&"d\0061t" (x int)"#).ddl[0];
    assert_eq!(show(e.name.as_ref().unwrap()), r#""dat""#);
}

/// A `RENAME TO` target is the last spelling of its name in the statement, so a quoted target
/// stays quoted even when an unquoted name earlier reads the same.
#[test]
fn rename_target_keeps_its_own_quoting() {
    let e = &pg(r#"alter table T rename to "t""#).ddl[0];
    assert_eq!(show(e.name.as_ref().unwrap()), "T");
    assert_eq!(
        e.action,
        DdlAction::Rename {
            to: Ident {
                text: "t".into(),
                quoted: true
            }
        }
    );
}

/// A name the text does not write as a plain name (a comment between its parts) is the
/// server's reading of it, quoted: never another object, never folded twice.
#[test]
fn a_name_split_by_a_comment_is_the_servers_reading() {
    let e = &pg("create table S /* x */ . T (a int)").ddl[0];
    assert_eq!(show(e.name.as_ref().unwrap()), r#""s"."t""#);
}

/// Names inside a body or a string are never taken for the statement's own.
#[test]
fn bodies_and_strings_name_nothing() {
    assert_eq!(
        pg("do $$ begin create table x(a int); end $$").ddl,
        [DdlEffect::opaque()]
    );
    let r = pg("comment on table t is 'drop table u'");
    assert_eq!(rendered(&r), ["comment table t"]);
}

#[test]
fn quoted_names_of_triggers_and_their_tables_are_kept() {
    let r = pg(r#"drop trigger "Trg" on "S"."T""#);
    assert_eq!(rendered(&r), [r#"drop trigger "Trg" on="S"."T""#]);
    let r = sqlite("CREATE TABLE [My T](a)");
    assert_eq!(rendered(&r), [r#"create table "My T""#]);
}

/// MySQL `GRANT … ON db.*` targets the schema; `ON *.*` names nothing.
#[test]
fn mysql_grant_on_a_database_wildcard() {
    assert_eq!(
        rendered(&my("GRANT SELECT ON db.* TO 'u'@'%'")),
        ["grant schema db"]
    );
    assert_eq!(
        my("GRANT SELECT ON *.* TO 'u'@'%'").ddl,
        [DdlEffect::opaque()]
    );
}

// -- the corpus ----------------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CorpusFile {
    case: Vec<Case>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Case {
    sql: String,
    ddl: Vec<String>,
}

fn testkit() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../sql-testkit")
}

fn corpus(file: &str) -> Vec<Case> {
    let path = testkit().join("corpus/ddl_effects").join(file);
    let text = std::fs::read_to_string(&path).unwrap();
    toml::from_str::<CorpusFile>(&text).unwrap().case
}

/// The dialects each corpus file runs on.
const CORPUS: [(&str, &[DialectId]); 3] = [
    ("pg.toml", &[DialectId::Postgres]),
    ("mysql.toml", &[DialectId::MySql, DialectId::MariaDb]),
    ("sqlite.toml", &[DialectId::Sqlite]),
];

#[test]
fn corpus_ddl_effects() {
    let mut failures = Vec::new();
    let mut count = 0;
    for (file, dialects) in CORPUS {
        for case in corpus(file) {
            for d in dialects {
                count += 1;
                let r = classify(&case.sql, &ctx(*d));
                let got: Vec<String> = match (&r.pre_decision, r.items.as_slice()) {
                    (None, [item]) => rendered(item),
                    _ => vec![format!("<{} items, {:?}>", r.items.len(), r.pre_decision)],
                };
                if got != case.ddl {
                    failures.push(format!(
                        "{file} {d:?}: {}\n  want {:?}\n  got  {:?}",
                        case.sql, case.ddl, got
                    ));
                }
            }
        }
    }
    assert!(count > 200, "{count} cases");
    assert!(
        failures.is_empty(),
        "{} of {count} cases:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

/// Every DDL-class statement of the corpus has an effect or is opaque; only TRUNCATE (U-3) and
/// statements of no DDL class have none.
#[test]
fn every_ddl_class_corpus_statement_has_an_effect() {
    use sql_safety::StmtKind as K;
    for (file, dialects) in CORPUS {
        for case in corpus(file) {
            for d in dialects {
                let r = classify(&case.sql, &ctx(*d));
                for item in &r.items {
                    let ddl_class = matches!(
                        item.kind,
                        K::CreateTable
                            | K::CreateOther
                            | K::Alter
                            | K::Drop
                            | K::DropOwned
                            | K::Grant
                            | K::Revoke
                            | K::Do
                            | K::Call
                            | K::Execute
                            | K::Prepare
                            | K::Other
                    );
                    if ddl_class {
                        assert!(!item.ddl.is_empty(), "{file} {d:?}: {}", case.sql);
                    }
                    for e in &item.ddl {
                        assert!(
                            e.opaque || e.name.is_some() || e.on.is_some(),
                            "{file} {d:?}: {}: {e:?}",
                            case.sql
                        );
                    }
                }
            }
        }
    }
}

// -- no verdict changes --------------------------------------------------------------------------

/// Everything `classify` reported before this task, `ddl` left out (it did not exist at the
/// `m4` base `fb98cf1`; the T1c stub at `f52034b` changed nothing else).
fn projection(r: &SafetyReport) -> String {
    let items: Vec<_> = r
        .items
        .iter()
        .map(|i| {
            (
                i.range,
                i.kind,
                i.effects,
                i.flags,
                i.class,
                &i.touched,
                &i.reasons,
                &i.destructive,
                i.confidence,
                &i.set_targets,
            )
        })
        .collect();
    format!(
        "{:?}",
        (
            items,
            r.request_class,
            r.request_effects,
            r.flags,
            &r.fingerprint,
            &r.backends,
            &r.pre_decision,
            r.classified_len,
            r.classified_hash,
            r.dialect,
        )
    )
}

fn hash(text: &str) -> u64 {
    twox_hash::XxHash3_64::oneshot(text.as_bytes())
}

/// One statement of an existing corpus: where it comes from, its dialect, its text.
struct Stmt {
    source: String,
    dialect: DialectId,
    sql: String,
}

fn dialect_named(name: &str) -> Option<DialectId> {
    Some(match name {
        "pg" | "postgres" => DialectId::Postgres,
        "cr" => DialectId::PgWire(PgWireFlavor::Cockroach),
        "rs" => DialectId::PgWire(PgWireFlavor::Redshift),
        "my" | "mysql" => DialectId::MySql,
        "ma" | "mariadb" => DialectId::MariaDb,
        "sl" | "sqlite" => DialectId::Sqlite,
        "ms" => DialectId::SqlServer,
        "gen" => DialectId::Generic,
        _ => return None,
    })
}

/// Entries of a read-corpus `.sql` file (`tests/read_corpus.rs`'s rule).
fn sql_entries(text: &str) -> Vec<String> {
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

#[derive(Deserialize)]
struct Rows {
    row: Vec<toml::Table>,
}

fn rows(path: PathBuf) -> Vec<toml::Table> {
    let text = std::fs::read_to_string(&path).unwrap();
    toml::from_str::<Rows>(&text).unwrap().row
}

fn str_field<'t>(row: &'t toml::Table, key: &str) -> Option<&'t str> {
    row.get(key).and_then(|v| v.as_str())
}

fn existing_corpora() -> Vec<Stmt> {
    let mut out = Vec::new();
    let read = testkit().join("corpus/read");
    for (file, d) in [
        ("pg.sql", DialectId::Postgres),
        ("mysql.sql", DialectId::MySql),
        ("mariadb.sql", DialectId::MariaDb),
        ("sqlite.sql", DialectId::Sqlite),
    ] {
        let text = std::fs::read_to_string(read.join(file)).unwrap();
        for sql in sql_entries(&text) {
            out.push(Stmt {
                source: format!("read/{file}"),
                dialect: d,
                sql,
            });
        }
    }
    for row in rows(read.join("escaped_literals.toml")) {
        let d = dialect_named(str_field(&row, "dialect").unwrap()).unwrap();
        out.push(Stmt {
            source: "read/escaped_literals.toml".into(),
            dialect: d,
            sql: str_field(&row, "sql").unwrap().into(),
        });
    }
    for (file, d) in [
        ("pg.toml", DialectId::Postgres),
        ("mysql.toml", DialectId::MySql),
        ("mariadb.toml", DialectId::MariaDb),
        ("sqlite.toml", DialectId::Sqlite),
    ] {
        for row in rows(testkit().join("bypass").join(file)) {
            out.push(Stmt {
                source: format!("bypass/{file}"),
                dialect: d,
                sql: str_field(&row, "sql").unwrap().into(),
            });
        }
    }
    for row in rows(testkit().join("corpus/fuzz/classifier_seeds.toml")) {
        let tag = str_field(&row, "tag").unwrap();
        let d = dialect_named(tag.split(',').next().unwrap()).unwrap();
        out.push(Stmt {
            source: "fuzz/classifier_seeds.toml".into(),
            dialect: d,
            sql: str_field(&row, "sql").unwrap().into(),
        });
    }
    let safe06 = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/data/safe06_interim.toml");
    for row in rows(safe06) {
        let d = dialect_named(str_field(&row, "dialect").unwrap()).unwrap();
        out.push(Stmt {
            source: "tests/data/safe06_interim.toml".into(),
            dialect: d,
            sql: str_field(&row, "sql").unwrap().into(),
        });
    }
    for (file, dialects) in CORPUS {
        for case in corpus(file) {
            for d in dialects {
                out.push(Stmt {
                    source: format!("ddl_effects/{file}"),
                    dialect: *d,
                    sql: case.sql.clone(),
                });
            }
        }
    }
    out
}

fn golden_path() -> PathBuf {
    testkit().join("corpus/ddl_effects/verdict-golden.tsv")
}

/// The golden's key of a statement: its source, dialect and text (hashed).
fn key(s: &Stmt) -> String {
    format!("{}\t{:?}\t{:016x}", s.source, s.dialect, hash(&s.sql))
}

#[test]
fn ddl_effects_never_change_the_verdict() {
    let stmts = existing_corpora();
    assert!(stmts.len() > 3000, "{} statements", stmts.len());
    let mut now: BTreeMap<String, (u64, String)> = BTreeMap::new();
    for s in &stmts {
        let p = projection(&classify(&s.sql, &ctx(s.dialect)));
        now.insert(key(s), (hash(&p), p));
    }
    if std::env::var_os("UPDATE_GOLDEN").is_some() {
        let mut text = String::from(
            "# owner: Task 6 (M4). Golden of `classify`'s reports, `ddl` left out, over the\n\
             # existing corpora (`sql-safety/tests/ddl_effects.rs`,\n\
             # `ddl_effects_never_change_the_verdict`),\n\
             # written at the `m4` base before Task 6 changed `src/`. Columns: source, dialect,\n\
             # XXH3-64 of the statement, XXH3-64 of the report's projection.\n",
        );
        for (k, (h, _)) in &now {
            let _ = writeln!(text, "{k}\t{h:016x}");
        }
        std::fs::write(golden_path(), text).unwrap();
        return;
    }
    let golden = std::fs::read_to_string(golden_path()).unwrap();
    let rows: Vec<&str> = golden.lines().filter(|l| !l.starts_with('#')).collect();
    let mut checked = 0;
    let mut missing = Vec::new();
    let mut diffs = Vec::new();
    for line in &rows {
        let (k, want) = line.rsplit_once('\t').unwrap();
        // Fix round 1 (m3): every golden row must still be a corpus statement; a statement added
        // to a corpus after the base has no golden row and is not checked here.
        let Some((got, p)) = now.get(k) else {
            missing.push(k);
            continue;
        };
        checked += 1;
        if format!("{got:016x}") != want {
            let src = stmts.iter().find(|s| key(s) == k).unwrap();
            diffs.push(format!("{k}: {}\n  now {p}", src.sql));
        }
    }
    assert!(
        missing.is_empty(),
        "{} golden rows have no corpus statement:\n{}",
        missing.len(),
        missing.join("\n")
    );
    assert_eq!(checked, 3360, "golden statements checked");
    assert_eq!(checked, rows.len());
    assert!(
        diffs.is_empty(),
        "{} reports changed:\n{}",
        diffs.len(),
        diffs.join("\n")
    );
}
