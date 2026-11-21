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

#![allow(dead_code, unused_imports)]
use std::fmt::Write as _;

use sql_dialect::{ClientFlavor, DialectCtx, DialectId};
use sql_ident::{Ident, QualifiedName, parse_qualified};
use sql_safety::{ClassifyCtx, DdlAction, DdlEffect, DdlKind, Expect, StmtReport, classify};

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

// -- reviewer hostile tests (M4 T6 review; not part of the task) ---------------------------------

fn dump(d: DialectId, sql: &str) -> String {
    let r = classify(sql, &ctx(d));
    let items: Vec<String> = r
        .items
        .iter()
        .map(|i| format!("{:?} {:?} {:?}", i.kind, rendered(i), i.effects))
        .collect();
    format!(
        "{d:?} | {sql} | pre={:?} | {}",
        r.pre_decision,
        items.join(" || ")
    )
}

#[test]
fn hostile_dump() {
    use DialectId::*;
    let cases: &[(DialectId, &str)] = &[
        (
            Postgres,
            "create schema s create table t (a int) create view v as select 1",
        ),
        (Postgres, "create schema s grant select on other.t to r"),
        (Postgres, "alter table s.t drop column c cascade"),
        (Postgres, "alter table s.t drop constraint t_pkey cascade"),
        (Postgres, "alter extension hstore set schema s2"),
        (Postgres, "security label for selinux on table t is 'x'"),
        (Postgres, "alter table v rename to w"),
        (Postgres, "alter table v alter column a set default 1"),
        (Postgres, "drop table s.parent"),
        (
            Postgres,
            "import foreign schema r from server srv into local_s",
        ),
        (
            Postgres,
            "alter default privileges in schema s grant select on tables to r",
        ),
        (
            Postgres,
            "update pg_catalog.pg_class set relname = 'x' where oid = 1",
        ),
        (Postgres, "create temp table t (a int references s.u)"),
        (Postgres, "create index i on s.parent (a)"),
        (
            Postgres,
            "create table s.p1 partition of other.p for values in (1)",
        ),
        (Postgres, "create or replace view s.v as select 1"),
        (
            Postgres,
            "create or replace function s.f() returns int language sql as 'select 1'",
        ),
        (Postgres, "grant usage on schema s to r"),
        (Postgres, "revoke all on all tables in schema s from r"),
        (Postgres, "create extension hstore schema s"),
        (Postgres, "drop extension hstore cascade"),
        (Postgres, "drop function f cascade"),
        (Postgres, "drop type s.ty cascade"),
        (Postgres, "alter sequence other.seq owned by s.t.c"),
        (Postgres, "alter type s.comp add attribute x int cascade"),
        (Postgres, "alter table s.t set schema s2"),
        (Postgres, "alter function s.f() set schema s2"),
        (Postgres, "do $$ begin execute 'drop table x'; end $$"),
        (Postgres, "call s.p()"),
        (Postgres, "execute st"),
        (Postgres, "prepare st as select 1"),
        (Postgres, "select s.f()"),
        (Postgres, "select pg_catalog.lo_unlink(1)"),
        (
            Postgres,
            "create table a(x int); alter table a rename to b; drop table b",
        ),
        (Postgres, "set search_path = other; create table t(a int)"),
        (Postgres, "create table pg_temp.t(a int)"),
        (Postgres, "create unlogged table t(a int)"),
        (Postgres, "refresh materialized view s.mv"),
        (Postgres, "reindex table s.t"),
        (Postgres, "cluster s.t using i"),
        (Postgres, "create statistics st on a, b from s.t"),
        (Postgres, "create publication p for tables in schema s"),
        (
            Postgres,
            "create event trigger et on ddl_command_start execute function f()",
        ),
        (
            Postgres,
            "alter table s.t attach partition other.p1 for values in (2)",
        ),
        (
            Postgres,
            "create rule r as on insert to s.t do instead nothing",
        ),
        (Postgres, "alter index s.i rename to j"),
        (Postgres, "comment on column s.t.c is 'x'"),
        (Postgres, "comment on extension hstore is 'x'"),
        (Postgres, "lock table s.t"),
        (Postgres, "vacuum full s.t"),
        (Postgres, "copy s.t from stdin"),
        (MySql, "RENAME TABLE db1.a TO b"),
        (MySql, "ALTER TABLE db1.a RENAME TO b"),
        (MySql, "ALTER TABLE db1.a RENAME TO db1.b"),
        (MySql, "RENAME TABLE a TO db2.b"),
        (MySql, "CREATE TABLE t SELECT 1 AS a"),
        (MySql, "CREATE TABLE t2 LIKE t"),
        (
            MySql,
            "/*!50001 CREATE ALGORITHM=UNDEFINED VIEW v AS SELECT 1 */",
        ),
        (MySql, "CREATE TABLE t (a int) /*!50100 ENGINE=InnoDB */"),
        (MySql, "SELECT 1 /*!50000 , (SELECT 1 FROM t) */"),
        (MySql, "CREATE TEMPORARY TABLE db1.t (a int)"),
        (MySql, "DROP TABLE temp.t"),
        (MySql, "CREATE TABLE temp.t (a int)"),
        (MySql, "ALTER TABLE t ADD COLUMN b int, RENAME TO u"),
        (
            MySql,
            "CREATE DEFINER=CURRENT_USER TRIGGER trg BEFORE INSERT ON t FOR EACH ROW SET NEW.a = 1",
        ),
        (MySql, "CREATE PROCEDURE p() BEGIN DROP TABLE t; END"),
        (MySql, "DROP EVENT e"),
        (MySql, "ALTER DATABASE db1 CHARACTER SET utf8mb4"),
        (MySql, "INSTALL PLUGIN x SONAME 'x.so'"),
        (MySql, "CREATE TABLESPACE ts ADD DATAFILE 'ts.ibd'"),
        (MySql, "USE db2"),
        (MySql, "OPTIMIZE TABLE t"),
        (MySql, "REPAIR TABLE t"),
        (MySql, "ALTER TABLE t TRUNCATE PARTITION p0"),
        (MySql, "TRUNCATE t"),
        (MySql, "LOAD DATA INFILE 'x' INTO TABLE t"),
        (MySql, "HANDLER t OPEN"),
        (MySql, "GRANT SELECT ON db.t TO 'u'@'%'"),
        (MySql, "GRANT EXECUTE ON PROCEDURE db.p TO 'u'@'%'"),
        (MySql, "CREATE USER 'u'@'%'"),
        (MySql, "DROP USER 'u'@'%'"),
        (MySql, "SET PASSWORD FOR 'u'@'%' = 'x'"),
        (MySql, "CREATE TABLE a (x int); RENAME TABLE a TO b"),
        (MariaDb, "/*M!100100 DROP TABLE x */"),
        (MariaDb, "SELECT 1 /*M!100100 , 2 */"),
        (MariaDb, "CREATE OR REPLACE TABLE t (a int)"),
        (MariaDb, "RENAME TABLE db1.a TO b"),
        (MariaDb, "CREATE SEQUENCE s"),
        (MariaDb, "ALTER TABLE db1.a RENAME TO b"),
        (Sqlite, "ATTACH 'x.db' AS aux"),
        (Sqlite, "ALTER TABLE aux.t RENAME TO u"),
        (Sqlite, "ALTER TABLE t RENAME COLUMN a TO b"),
        (Sqlite, "ALTER TABLE t DROP COLUMN a"),
        (
            Sqlite,
            "CREATE TEMP TRIGGER trg AFTER INSERT ON main.t BEGIN SELECT 1; END",
        ),
        (
            Sqlite,
            "UPDATE sqlite_master SET sql = 'x' WHERE name = 't'",
        ),
        (
            Sqlite,
            "UPDATE sqlite_schema SET sql = 'x' WHERE name = 't'",
        ),
        (Sqlite, "PRAGMA writable_schema = 1"),
        (Sqlite, "CREATE TABLE \"TEMP\".t(a)"),
        (Sqlite, "CREATE VIRTUAL TABLE f USING fts5(a)"),
        (Sqlite, "VACUUM"),
        (Sqlite, "DROP TABLE temp.t"),
        (Sqlite, "CREATE TEMP VIEW v AS SELECT 1"),
        (Sqlite, "REINDEX"),
        (Sqlite, "PRAGMA main.user_version = 3"),
    ];
    for (d, sql) in cases {
        eprintln!("{}", dump(*d, sql));
    }
}

/// H-1: `ALTER TABLE … DROP COLUMN/CONSTRAINT … CASCADE` drops dependents in other tables and
/// schemas (views, foreign keys); the effect must say so (cascade) or be opaque.
#[test]
fn hostile_alter_drop_cascade_is_not_narrow() {
    for s in [
        "alter table s.t drop column c cascade",
        "alter table s.t drop constraint t_pkey cascade",
    ] {
        let r = pg(s);
        let ok = r
            .ddl
            .iter()
            .any(|e| e.opaque || matches!(e.action, DdlAction::Drop { cascade: true }));
        assert!(ok, "{s}: {:?}", rendered(&r));
    }
}

/// H-2: MySQL resolves an unqualified rename target in the current database, so
/// `RENAME TABLE db1.a TO b` moves `a` out of `db1`: the effect must name the target schema
/// (an unqualified create the consumer resolves with the current database) or be opaque.
#[test]
fn hostile_mysql_rename_to_unqualified_moves_to_current_db() {
    for d in [DialectId::MySql, DialectId::MariaDb] {
        for s in ["RENAME TABLE db1.a TO b", "ALTER TABLE db1.a RENAME TO b"] {
            let r = one(d, s);
            let ok = r.ddl.iter().any(|e| {
                e.opaque
                    || (e.action == DdlAction::Create
                        && e.name.as_ref().is_some_and(|n| n.parts.len() == 1))
            });
            assert!(ok, "{d:?} {s}: {:?}", rendered(&r));
        }
    }
}

/// H-3: `ALTER EXTENSION … SET SCHEMA` moves every member object between two schemas; the
/// implementer's own rule makes extension statements opaque.
#[test]
fn hostile_alter_extension_set_schema_is_opaque() {
    let r = pg("alter extension hstore set schema s2");
    assert!(
        r.ddl.first().is_some_and(|e| e.opaque),
        "{:?}",
        rendered(&r)
    );
}

/// H-4: `CREATE SCHEMA … GRANT … ON other.t` changes an object outside the new schema.
#[test]
fn hostile_create_schema_with_an_outside_grant() {
    let r = pg("create schema s grant select on other.t to r");
    let ok = r
        .ddl
        .iter()
        .any(|e| e.opaque || e.name.as_ref().is_some_and(|n| n.parts.len() == 2));
    assert!(ok, "{:?}", rendered(&r));
}
