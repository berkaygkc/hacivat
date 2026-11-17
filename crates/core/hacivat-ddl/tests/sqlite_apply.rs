//! SQLite apply-then-reread, in process (the SQLite leg of alg/03 HV-DDL-07 test 10 and plan
//! Task 31 `sqlite_roundtrip`): every SQLite golden runs against an empty database (with
//! `aux` attached), then the server's own description of each table is compared with the
//! fixture model the golden came from: columns (name, declared type, NOT NULL, default,
//! primary-key membership, generated kind), foreign keys, indexes, triggers and views.
//!
//! Re-rendering from the reread model needs the SQLite introspector (Task 18); until then the
//! comparison is against the model fields directly.

mod fixtures;

use std::collections::BTreeSet;

use fixtures::{golden_path, sqlite};
use hacivat_meta::props::{FkAction, Generated};
use hacivat_meta::{ObjectKind, Props};
use rusqlite::Connection;

fn apply(rel: &str) -> Connection {
    let sql = std::fs::read_to_string(golden_path(rel)).expect("golden");
    let c = Connection::open_in_memory().unwrap();
    c.execute_batch("ATTACH DATABASE ':memory:' AS aux")
        .unwrap();
    c.execute_batch(&sql)
        .unwrap_or_else(|e| panic!("{rel} does not apply: {e}\n{sql}"));
    c
}

fn objects(c: &Connection, schema: &str) -> BTreeSet<(String, String)> {
    let mut st = c
        .prepare(&format!(
            "SELECT type, name FROM \"{schema}\".sqlite_schema WHERE name NOT LIKE 'sqlite_%'"
        ))
        .unwrap();
    st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

fn fk_action(a: FkAction) -> &'static str {
    match a {
        FkAction::NoAction => "NO ACTION",
        FkAction::Restrict => "RESTRICT",
        FkAction::Cascade => "CASCADE",
        FkAction::SetNull => "SET NULL",
        FkAction::SetDefault => "SET DEFAULT",
    }
}

/// Compares every table the golden created with the fixture model.
fn reread_matches_model(c: &Connection, rel: &str) {
    let fx = sqlite();
    for ns in fx.snap.namespaces() {
        let schema = ns.key().root_name().to_string();
        let created = objects(c, &schema);
        for (ti, t) in ns.children_of_kind(0, ObjectKind::Table) {
            if !created.contains(&("table".to_string(), t.name.to_string())) {
                continue;
            }
            let pk_cols: Vec<String> = ns
                .children_of_kind(ti, ObjectKind::PrimaryKey)
                .flat_map(|(_, k)| match &k.props {
                    Props::Key(kp) => kp.columns.iter().map(|c| c.to_string()).collect(),
                    _ => vec![],
                })
                .collect();
            let mut st = c
                .prepare("SELECT name, type, \"notnull\", dflt_value, pk, hidden FROM pragma_table_xinfo(?1, ?2) ORDER BY cid")
                .unwrap();
            type Row = (String, String, bool, Option<String>, i64, i64);
            let rows: Vec<Row> = st
                .query_map([&*t.name, &schema], |r| {
                    Ok((
                        r.get(0)?,
                        r.get(1)?,
                        r.get(2)?,
                        r.get(3)?,
                        r.get(4)?,
                        r.get(5)?,
                    ))
                })
                .unwrap()
                .map(Result::unwrap)
                .collect();
            let model: Vec<_> = ns.children_of_kind(ti, ObjectKind::Column).collect();
            assert_eq!(rows.len(), model.len(), "{rel}: {} column count", t.name);
            for ((name, ty, notnull, dflt, pk, hidden), (_, m)) in rows.iter().zip(&model) {
                let cp = m.props.as_column().unwrap();
                let what = format!("{rel}: {}.{name}", t.name);
                assert_eq!(name.as_str(), &*m.name, "{what}");
                assert_eq!(ty.as_str(), &*cp.type_text, "{what} type");
                assert_eq!(*notnull, !cp.nullable, "{what} notnull");
                assert_eq!(
                    dflt.as_deref(),
                    cp.default_expr.as_deref(),
                    "{what} default"
                );
                assert_eq!(*pk > 0, pk_cols.contains(name), "{what} pk");
                let want_hidden = match cp.generated {
                    Generated::No => 0,
                    Generated::Virtual => 2,
                    Generated::Stored => 3,
                };
                assert_eq!(*hidden, want_hidden, "{what} hidden");
            }
            // Foreign keys: parent table, columns and ON DELETE.
            let mut st = c
                .prepare("SELECT \"table\", \"from\", \"to\", on_delete FROM pragma_foreign_key_list(?1, ?2) ORDER BY id, seq")
                .unwrap();
            let fks: BTreeSet<(String, String, String, String)> = st
                .query_map([&*t.name, &schema], |r| {
                    Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
                })
                .unwrap()
                .map(Result::unwrap)
                .collect();
            let model_fks: BTreeSet<(String, String, String, String)> = ns
                .children_of_kind(ti, ObjectKind::ForeignKey)
                .filter_map(|(_, k)| match &k.props {
                    Props::ForeignKey(f) => Some((
                        f.target.path.last().unwrap().to_string(),
                        f.columns[0].to_string(),
                        f.target_columns[0].to_string(),
                        fk_action(f.on_delete).to_string(),
                    )),
                    _ => None,
                })
                .collect();
            assert_eq!(fks, model_fks, "{rel}: {} foreign keys", t.name);
            // Indexes: every non-automatic model index exists, with its unique and partial flags.
            let mut st = c
                .prepare("SELECT name, \"unique\", origin, partial FROM pragma_index_list(?1, ?2)")
                .unwrap();
            let idx: BTreeSet<(String, bool, String, bool)> = st
                .query_map([&*t.name, &schema], |r| {
                    Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
                })
                .unwrap()
                .map(Result::unwrap)
                .collect();
            for (_, i) in ns.children_of_kind(ti, ObjectKind::Index) {
                let Props::Index(ip) = &i.props else { continue };
                if i.name.starts_with("sqlite_autoindex_") {
                    assert!(
                        idx.iter().any(|(_, u, o, _)| *u && o == "u"),
                        "{rel}: {} unique key index",
                        t.name
                    );
                    continue;
                }
                assert!(
                    idx.contains(&(
                        i.name.to_string(),
                        ip.unique,
                        "c".to_string(),
                        ip.predicate.is_some()
                    )),
                    "{rel}: index {} in {idx:?}",
                    i.name
                );
            }
            for (_, tr) in ns.children_of_kind(ti, ObjectKind::Trigger) {
                assert!(
                    created.contains(&("trigger".to_string(), tr.name.to_string())),
                    "{rel}: trigger {}",
                    tr.name
                );
            }
        }
    }
    let integrity: String = c
        .query_row("PRAGMA integrity_check", [], |r| r.get(0))
        .unwrap();
    assert_eq!(integrity, "ok");
}

#[test]
fn sqlite_roundtrip_every_golden() {
    let dir = golden_path("sqlite/x").parent().unwrap().to_path_buf();
    let mut files: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "sql"))
        .collect();
    files.sort();
    assert!(files.len() >= 5, "{files:?}");
    for f in files {
        let rel = format!("sqlite/{}", f.file_stem().unwrap().to_string_lossy());
        let c = apply(&rel);
        reread_matches_model(&c, &rel);
    }
}

#[test]
fn sqlite_table_full_creates_every_object() {
    let c = apply("sqlite/table_full");
    let got = objects(&c, "main");
    for (ty, name) in [
        ("table", "authors"),
        ("table", "notes"),
        ("index", "notes_author"),
        ("index", "notes_lower_body"),
        ("trigger", "notes_touch"),
        ("view", "v_notes"),
    ] {
        assert!(
            got.contains(&(ty.to_string(), name.to_string())),
            "{ty} {name}: {got:?}"
        );
    }
    // AUTOINCREMENT survived: SQLite keeps a sqlite_sequence table for it.
    let seq: i64 = c
        .query_row(
            "SELECT count(*) FROM main.sqlite_schema WHERE name = 'sqlite_sequence'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(seq, 1);
}

#[test]
fn sqlite_attached_objects_land_in_aux() {
    let c = apply("sqlite/attached");
    let got = objects(&c, "aux");
    assert!(
        got.contains(&("index".to_string(), "kv_v".to_string())),
        "{got:?}"
    );
    assert!(
        got.contains(&("trigger".to_string(), "kv_guard".to_string())),
        "{got:?}"
    );
    assert!(objects(&c, "main").is_empty());
}

#[test]
fn sqlite_strict_table_enforces_types() {
    let c = apply("sqlite/strict_without_rowid_generated");
    let err = c.execute(
        "INSERT INTO prices (sku, qty, price) VALUES ('a', 'many', 1.0)",
        [],
    );
    assert!(err.is_err(), "STRICT must refuse text in an INTEGER column");
    c.execute(
        "INSERT INTO prices (sku, qty, price) VALUES ('a', 2, 1.5)",
        [],
    )
    .unwrap();
    let (total, label): (f64, String) = c
        .query_row("SELECT total, label FROM prices", [], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .unwrap();
    assert_eq!((total, label.as_str()), (3.0, "A"));
}
