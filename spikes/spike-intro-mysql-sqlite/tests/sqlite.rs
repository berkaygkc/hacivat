//! The SQLite introspector against real files through `SqliteDriver` (no Docker): every
//! HV-MODEL-01/HV-INTRO-13 shape the catalog names (WITHOUT ROWID, STRICT, generated columns,
//! composite and expression keys, unnamed constraints, virtual tables, broken views), the
//! ver/01 `#sqlite-schema-version` matrix as change detection, `data_version`, the digest
//! path, case rules (`#sqlite-case`) and the read-only profile's authorizer.

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use hacivat_driver::{Driver, DriverConnect, Endpoint, RawConn};
use hacivat_driver_sqlite::SqliteDriver;
use spike_intro_mysql_sqlite::conn::TypedCatalog;
use spike_intro_mysql_sqlite::model::*;
use spike_intro_mysql_sqlite::sqlite::{self, RefreshOutcome};

fn request(path: &Path, read_only: bool) -> DriverConnect {
    DriverConnect {
        endpoint: Endpoint::File {
            path: path.to_path_buf(),
            create_if_missing: false,
            read_only: false,
        },
        tls: None,
        user: None,
        password: None,
        database: None,
        application_name: "Hacivat (metadata)".into(),
        read_only,
        options: vec![],
        connect_timeout: Duration::from_secs(15),
        events: Arc::new(|_| {}),
        insecure_auth: Default::default(),
    }
}

async fn open(path: &Path, read_only: bool) -> Box<dyn RawConn> {
    SqliteDriver::new()
        .connect(request(path, read_only))
        .await
        .unwrap()
}

const FIXTURE: &str = r#"
PRAGMA foreign_keys = ON;
CREATE TABLE parent (
    id INTEGER PRIMARY KEY,
    code TEXT NOT NULL UNIQUE,
    region TEXT,
    note TEXT DEFAULT 'n/a'
);
CREATE TABLE child (
    a INTEGER NOT NULL,
    b TEXT NOT NULL,
    parent_id INTEGER REFERENCES parent(id) ON DELETE CASCADE,
    code TEXT,
    total REAL GENERATED ALWAYS AS (a * 2) VIRTUAL,
    label TEXT AS (upper(b)) STORED,
    PRIMARY KEY (b, a),
    CONSTRAINT fk_code FOREIGN KEY (code, b) REFERENCES pair(x, y) ON UPDATE SET NULL,
    CHECK (a > 0)
);
CREATE TABLE pair (x TEXT, y TEXT, PRIMARY KEY (x, y)) WITHOUT ROWID;
CREATE TABLE strict_t (k INTEGER PRIMARY KEY, v TEXT NOT NULL) STRICT;
CREATE TABLE "Müşteri" (a);
CREATE TABLE "MÜŞTERİ" (a);
CREATE INDEX ix_child_desc ON child (parent_id DESC, code COLLATE NOCASE);
CREATE INDEX ix_expr ON parent (lower(code), region);
CREATE UNIQUE INDEX ix_partial ON parent (region) WHERE region IS NOT NULL;
CREATE VIEW v_child AS SELECT a, b, total FROM child;
CREATE TABLE doomed (z);
CREATE VIEW v_broken AS SELECT z FROM doomed;
CREATE TRIGGER trg_child AFTER INSERT ON child BEGIN SELECT 1; END;
CREATE VIRTUAL TABLE docs USING fts5(title, body);
"#;

fn fixture(dir: &Path) -> std::path::PathBuf {
    let path = dir.join("intro.sqlite");
    let c = rusqlite::Connection::open(&path).unwrap();
    c.execute_batch(FIXTURE).unwrap();
    // A view over a dropped table stays in the schema (legacy behaviour SQLite keeps).
    c.execute_batch("PRAGMA legacy_alter_table = ON; DROP TABLE doomed;")
        .unwrap();
    path
}

fn names(s: &SchemaSnapshot) -> Vec<&str> {
    s.relations.iter().map(|r| r.name.as_str()).collect()
}

#[tokio::test]
async fn full_introspection_reads_every_shape() {
    let dir = tempfile::tempdir().unwrap();
    let path = fixture(dir.path());
    let mut raw = open(&path, false).await;
    let mut c = TypedCatalog::new(&mut *raw);

    let ns = sqlite::list_namespaces(&mut c).await.unwrap();
    assert_eq!(ns[0].name, "main");
    assert!(ns[0].detail.as_deref().unwrap().ends_with("intro.sqlite"));

    let (s, token) = sqlite::introspect(&mut c, "main", Level::Sources)
        .await
        .unwrap();
    assert!(token.schema_version > 0);
    assert_eq!(token.digest.len(), 64);

    // Exact names, never folded (#sqlite-case: ASCII-only folding, so `Ü` and `ü` differ).
    let n = names(&s);
    assert!(n.contains(&"Müşteri") && n.contains(&"MÜŞTERİ"), "{n:?}");
    assert!(!n.contains(&"doomed"));

    let parent = s.relation("parent").unwrap();
    assert_eq!(parent.kind, Some(RelationKind::Table));
    let id = parent.column("id").unwrap();
    assert_eq!(
        (id.ordinal, id.pk_position, id.data_type.as_str()),
        (1, 1, "INTEGER")
    );
    assert_eq!(
        parent.column("note").unwrap().default.as_deref(),
        Some("'n/a'")
    );
    assert!(!parent.column("code").unwrap().nullable);
    // INTEGER PRIMARY KEY: no index, PK from `pk`.
    let pk = parent
        .constraints
        .iter()
        .find(|c| c.kind == ConstraintKind::PrimaryKey)
        .unwrap();
    assert_eq!(pk.columns, ["id"]);
    // UNIQUE column → unnamed unique constraint backed by `sqlite_autoindex_parent_1`.
    let auto = parent.index("sqlite_autoindex_parent_1").unwrap();
    assert!(auto.unique && auto.origin.as_deref() == Some("u") && auto.sql.is_none());
    assert!(
        parent
            .constraints
            .iter()
            .any(|c| c.kind == ConstraintKind::Unique && c.columns == ["code"] && c.name.is_none())
    );
    // Expression index: key part without a name; text in the index SQL.
    let ix = parent.index("ix_expr").unwrap();
    assert_eq!(ix.columns.len(), 2);
    assert_eq!(ix.columns[0].name, None);
    assert_eq!(ix.columns[1].name.as_deref(), Some("region"));
    assert!(ix.sql.as_deref().unwrap().contains("lower(code)"));
    // Partial unique index.
    let p = parent.index("ix_partial").unwrap();
    assert!(p.partial && p.unique && p.origin.as_deref() == Some("c"));

    let child = s.relation("child").unwrap();
    // Composite PK in key order (b, a), not table order.
    let pk = child
        .constraints
        .iter()
        .find(|c| c.kind == ConstraintKind::PrimaryKey)
        .unwrap();
    assert_eq!(pk.columns, ["b", "a"]);
    // Generated columns: hidden 2 (virtual), 3 (stored).
    assert_eq!(
        child.column("total").unwrap().generated,
        Some((GeneratedKind::Virtual, None))
    );
    assert_eq!(
        child.column("label").unwrap().generated,
        Some((GeneratedKind::Stored, None))
    );
    assert_eq!(
        child.columns.len(),
        6,
        "table_xinfo lists generated columns"
    );
    // FKs: single-column with ON DELETE, composite with ON UPDATE; names are not reported.
    let fks: Vec<&Constraint> = child
        .constraints
        .iter()
        .filter(|c| matches!(c.kind, ConstraintKind::ForeignKey(_)))
        .collect();
    assert_eq!(fks.len(), 2);
    let to_parent = fks.iter().find(|c| c.columns == ["parent_id"]).unwrap();
    let ConstraintKind::ForeignKey(fk) = &to_parent.kind else {
        unreachable!()
    };
    assert_eq!(fk.ref_table, "parent");
    assert_eq!(fk.ref_columns, [Some("id".to_owned())]);
    assert_eq!(fk.on_delete.as_deref(), Some("CASCADE"));
    let to_pair = fks.iter().find(|c| c.columns == ["code", "b"]).unwrap();
    assert_eq!(to_pair.name, None, "pragma_foreign_key_list has no names");
    let ConstraintKind::ForeignKey(fk) = &to_pair.kind else {
        unreachable!()
    };
    assert_eq!(fk.ref_columns, [Some("x".to_owned()), Some("y".to_owned())]);
    assert_eq!(fk.on_update.as_deref(), Some("SET NULL"));
    // Descending + collated key parts.
    let d = child.index("ix_child_desc").unwrap();
    assert!(d.columns[0].descending && !d.columns[1].descending);
    assert_eq!(d.columns[1].collation.as_deref(), Some("NOCASE"));
    // CHECK is only in the SQL text (no pragma reports it).
    assert!(child.sql.as_deref().unwrap().contains("CHECK (a > 0)"));

    let pair = s.relation("pair").unwrap();
    assert!(pair.without_rowid && !pair.strict);
    let st = s.relation("strict_t").unwrap();
    assert!(st.strict && !st.without_rowid);

    let v = s.relation("v_child").unwrap();
    assert_eq!(v.kind, Some(RelationKind::View));
    let cols: Vec<&str> = v.columns.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(cols, ["a", "b", "total"]);
    assert!(v.view.as_ref().unwrap().definition.is_some());
    // The broken view is kept with its error (HV-INTRO-13 broken).
    let b = s.relation("v_broken").unwrap();
    assert!(b.columns.is_empty());
    assert!(
        b.broken.as_deref().unwrap().contains("doomed"),
        "{:?}",
        b.broken
    );

    let docs = s.relation("docs").unwrap();
    assert_eq!(docs.kind, Some(RelationKind::VirtualTable));
    assert_eq!(docs.engine.as_deref(), Some("fts5"));
    // FTS5 shadow tables are listed and flagged by `pragma_table_list`.
    let shadow = s.relation("docs_data").unwrap();
    assert_eq!(shadow.kind, Some(RelationKind::Shadow));

    let t = s.trigger("trg_child").unwrap();
    assert_eq!(t.table, "child");
    assert!(t.body.as_deref().unwrap().starts_with("CREATE TRIGGER"));

    // Connection-level functions (HV-MODEL-01 SQLite `Routine`).
    let fs = sqlite::functions(&mut c).await.unwrap();
    assert!(fs.iter().any(|(n, builtin, _, _)| n == "upper" && *builtin));
    // `Names` level: relations only, no columns.
    let (names_only, _) = sqlite::introspect(&mut c, "main", Level::Names)
        .await
        .unwrap();
    assert_eq!(names_only.relations.len(), s.relations.len());
    assert_eq!(names_only.column_count(), 0);
    drop(c);
    raw.close().await;
}

#[tokio::test]
async fn read_only_profile_introspects_through_the_authorizer() {
    let dir = tempfile::tempdir().unwrap();
    let path = fixture(dir.path());
    let mut rw = open(&path, false).await;
    let (expected, _) =
        sqlite::introspect(&mut TypedCatalog::new(&mut *rw), "main", Level::Sources)
            .await
            .unwrap();
    rw.close().await;

    let mut ro = open(&path, true).await;
    let mut c = TypedCatalog::new(&mut *ro);
    let (got, _) = sqlite::introspect(&mut c, "main", Level::Sources)
        .await
        .unwrap();
    // Finding: `pragma_table_list` is not on the read-only allowlist (Q16), so the read-only
    // pass falls back to the SQL text scan: WITHOUT ROWID/STRICT still right, but FTS5 shadow
    // tables read as plain tables.
    let shadow = got.relation("docs_data").unwrap();
    assert_eq!(shadow.kind, Some(RelationKind::Table));
    let mut got_fixed = got.clone();
    for r in &mut got_fixed.relations {
        if let Some(e) = expected.relation(&r.name)
            && e.kind == Some(RelationKind::Shadow)
        {
            r.kind = Some(RelationKind::Shadow);
        }
    }
    assert_same(
        &got_fixed,
        &expected,
        "read-only = read-write but for shadow flags",
    );
    // `data_version` is denied by the read-only authorizer (not in SQLITE_READ_PRAGMAS).
    assert!(sqlite::data_version(&mut c, "main").await.is_err());
    assert!(sqlite::schema_version(&mut c, "main").await.is_ok());
    drop(c);
    ro.close().await;
}

/// ver/01 `#sqlite-schema-version` as change detection: DDL from another connection bumps
/// `schema_version` and re-reads; INSERT, a rolled-back CREATE and temp DDL do not touch
/// `main`; VACUUM re-reads (spurious, harmless).
#[tokio::test]
async fn schema_version_matrix_drives_refresh() {
    let dir = tempfile::tempdir().unwrap();
    let path = fixture(dir.path());
    let other = rusqlite::Connection::open(&path).unwrap();
    let mut raw = open(&path, false).await;
    let mut c = TypedCatalog::new(&mut *raw);
    let (mut snap, mut token) = sqlite::introspect(&mut c, "main", Level::Structure)
        .await
        .unwrap();

    // (step, sql run by another connection, expected outcome, check)
    type Check = fn(&SchemaSnapshot) -> bool;
    let steps: Vec<(&str, &str, RefreshOutcome, Check)> = vec![
        (
            "insert",
            "INSERT INTO parent(code) VALUES ('x1')",
            RefreshOutcome::Unchanged,
            |_| true,
        ),
        (
            "create index",
            "CREATE INDEX ix_new ON parent(note)",
            RefreshOutcome::Reread,
            |s| s.relation("parent").unwrap().index("ix_new").is_some(),
        ),
        (
            "add column",
            "ALTER TABLE parent ADD COLUMN extra INT",
            RefreshOutcome::Reread,
            |s| s.relation("parent").unwrap().column("extra").is_some(),
        ),
        (
            "create view",
            "CREATE VIEW v2 AS SELECT 1 AS one",
            RefreshOutcome::Reread,
            |s| s.relation("v2").is_some(),
        ),
        ("drop view", "DROP VIEW v2", RefreshOutcome::Reread, |s| {
            s.relation("v2").is_none()
        }),
        (
            "rolled-back create",
            "BEGIN; CREATE TABLE z(a); ROLLBACK;",
            RefreshOutcome::Unchanged,
            |s| s.relation("z").is_none(),
        ),
        ("vacuum", "VACUUM", RefreshOutcome::Reread, |_| true),
    ];
    for (step, sql, outcome, check) in steps {
        other.execute_batch(sql).unwrap();
        let before = c.log.len();
        let (next, next_token, got) =
            sqlite::refresh(&mut c, &snap, &token, Level::Structure, false)
                .await
                .unwrap();
        assert_eq!(got, outcome, "{step}");
        assert!(check(&next), "{step}");
        let xinfo = c.log[before..]
            .iter()
            .filter(|s| s.contains("pragma_table_xinfo"))
            .count();
        assert_eq!(
            xinfo > 0,
            outcome == RefreshOutcome::Reread,
            "{step}: {xinfo} column reads"
        );
        if outcome == RefreshOutcome::Reread {
            assert!(next_token.schema_version > token.schema_version, "{step}");
            // The re-read equals a fresh full pass.
            let (fresh, _) = sqlite::introspect(&mut c, "main", Level::Structure)
                .await
                .unwrap();
            assert_eq!(next, fresh, "{step}");
        }
        snap = next;
        token = next_token;
    }

    // Temp DDL on this connection bumps only `temp`'s counter.
    let main_before = sqlite::schema_version(&mut c, "main").await.unwrap();
    raw_batch(&mut c, "CREATE TEMP TABLE tt(a)").await;
    assert_eq!(
        sqlite::schema_version(&mut c, "main").await.unwrap(),
        main_before
    );
    assert!(sqlite::schema_version(&mut c, "temp").await.unwrap() >= 1);
    let (temp, _) = sqlite::introspect(&mut c, "temp", Level::Structure)
        .await
        .unwrap();
    assert_eq!(names(&temp), ["tt"]);
    let (_, _, got) = sqlite::refresh(&mut c, &snap, &token, Level::Structure, false)
        .await
        .unwrap();
    assert_eq!(got, RefreshOutcome::Unchanged);

    // An attached database is its own namespace.
    let aux = dir.path().join("aux.sqlite");
    rusqlite::Connection::open(&aux)
        .unwrap()
        .execute_batch("CREATE TABLE in_aux(q)")
        .unwrap();
    let attach = format!("ATTACH '{}' AS aux", aux.display());
    raw_batch(&mut c, &attach).await;
    let ns: Vec<String> = sqlite::list_namespaces(&mut c)
        .await
        .unwrap()
        .into_iter()
        .map(|n| n.name)
        .collect();
    assert!(ns.contains(&"aux".to_owned()), "{ns:?}");
    let (a, _) = sqlite::introspect(&mut c, "aux", Level::Structure)
        .await
        .unwrap();
    assert_eq!(names(&a), ["in_aux"]);
    drop(c);
    raw.close().await;
}

/// `a == b`, failing with the first differing relation (snapshots are large).
fn assert_same(a: &SchemaSnapshot, b: &SchemaSnapshot, what: &str) {
    if a == b {
        return;
    }
    let an: Vec<&str> = names(a);
    let bn: Vec<&str> = names(b);
    assert_eq!(an, bn, "{what}: relation names");
    for (x, y) in a.relations.iter().zip(&b.relations) {
        assert_eq!(x, y, "{what}: relation {}", x.name);
    }
    assert_eq!(a.triggers, b.triggers, "{what}: triggers");
    assert_eq!(a, b, "{what}");
}

async fn raw_batch(c: &mut TypedCatalog<'_>, sql: &str) {
    use spike_intro_mysql_sqlite::conn::CatalogConn;
    c.control(sql).await.unwrap();
}

/// `data_version` (ver/01 shows 1 on a fresh connection): another connection's commit moves
/// it — data or schema — and this connection's own commits never do. It answers "did anyone
/// else write?", not "did the schema change?".
#[tokio::test]
async fn data_version_moves_on_other_connections_commits_only() {
    let dir = tempfile::tempdir().unwrap();
    let path = fixture(dir.path());
    let other = rusqlite::Connection::open(&path).unwrap();
    let mut raw = open(&path, false).await;
    let mut c = TypedCatalog::new(&mut *raw);
    let d0 = sqlite::data_version(&mut c, "main").await.unwrap();
    raw_batch(&mut c, "INSERT INTO parent(code) VALUES ('own')").await;
    assert_eq!(sqlite::data_version(&mut c, "main").await.unwrap(), d0);
    other
        .execute_batch("INSERT INTO parent(code) VALUES ('theirs')")
        .unwrap();
    let d1 = sqlite::data_version(&mut c, "main").await.unwrap();
    assert!(d1 > d0);
    let sv = sqlite::schema_version(&mut c, "main").await.unwrap();
    other.execute_batch("CREATE TABLE t_more(a)").unwrap();
    assert!(sqlite::data_version(&mut c, "main").await.unwrap() > d1);
    assert_eq!(
        sqlite::schema_version(&mut c, "main").await.unwrap(),
        sv + 1
    );
    drop(c);
    raw.close().await;
}

/// HV-INTRO-08 test 5: a file replaced in place by another database whose `schema_version`
/// matches is missed by the fast path and caught by the digest once the identity changed.
#[tokio::test]
async fn digest_catches_a_replaced_file_with_the_same_schema_version() {
    let dir = tempfile::tempdir().unwrap();
    let a = dir.path().join("a.sqlite");
    let b = dir.path().join("b.sqlite");
    rusqlite::Connection::open(&a)
        .unwrap()
        .execute_batch("CREATE TABLE one(x)")
        .unwrap();
    rusqlite::Connection::open(&b)
        .unwrap()
        .execute_batch("CREATE TABLE two(y)")
        .unwrap();
    let mut raw = open(&a, false).await;
    let (snap, token) =
        sqlite::introspect(&mut TypedCatalog::new(&mut *raw), "main", Level::Structure)
            .await
            .unwrap();
    raw.close().await;
    std::fs::copy(&b, &a).unwrap();
    let mut raw = open(&a, false).await;
    let mut c = TypedCatalog::new(&mut *raw);
    assert_eq!(
        sqlite::schema_version(&mut c, "main").await.unwrap(),
        token.schema_version,
        "the crafted collision"
    );
    let (_, _, fast) = sqlite::refresh(&mut c, &snap, &token, Level::Structure, false)
        .await
        .unwrap();
    assert_eq!(fast, RefreshOutcome::Unchanged, "documented limitation");
    let (next, _, slow) = sqlite::refresh(&mut c, &snap, &token, Level::Structure, true)
        .await
        .unwrap();
    assert_eq!(slow, RefreshOutcome::Reread);
    assert_eq!(names(&next), ["two"]);
    // Identity changed but same content → digest matches, nothing re-read.
    let (_, tok2) = sqlite::introspect(&mut c, "main", Level::Structure)
        .await
        .unwrap();
    let (_, _, same) = sqlite::refresh(&mut c, &next, &tok2, Level::Structure, true)
        .await
        .unwrap();
    assert_eq!(same, RefreshOutcome::SameDigest);
    drop(c);
    raw.close().await;
}

/// The 10k-table / 160k-column measurement (HV-INTRO-01 scale target). `#[ignore]`: run with
/// `cargo test -p spike-intro-mysql-sqlite --release --test sqlite -- --ignored --nocapture`.
#[tokio::test]
#[ignore = "perf"]
async fn perf_10k_tables_160k_columns() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("big.sqlite");
    let t0 = Instant::now();
    {
        let mut c = rusqlite::Connection::open(&path).unwrap();
        let tx = c.transaction().unwrap();
        for t in 0..10_000 {
            let cols: Vec<String> = (0..16)
                .map(|i| match i {
                    0 => "id INTEGER PRIMARY KEY".to_owned(),
                    1 => format!("ref_{t} INTEGER REFERENCES t{:05}(id)", t / 2),
                    _ => format!("c{i} TEXT DEFAULT 'x' NOT NULL"),
                })
                .collect();
            tx.execute_batch(&format!(
                "CREATE TABLE t{t:05} ({}); CREATE INDEX t{t:05}_ix ON t{t:05}(c2, c3);",
                cols.join(", ")
            ))
            .unwrap();
        }
        tx.commit().unwrap();
    }
    let build = t0.elapsed();
    let mut raw = open(&path, true).await;
    let mut c = TypedCatalog::new(&mut *raw);
    for level in [Level::Names, Level::Structure, Level::Sources] {
        let t = Instant::now();
        let (s, _) = sqlite::introspect(&mut c, "main", level).await.unwrap();
        let took = t.elapsed();
        if level >= Level::Structure {
            assert_eq!(s.relations.len(), 10_000);
            assert_eq!(s.column_count(), 160_000);
        }
        println!(
            "PERF sqlite {level:?}: {} relations, {} columns, {} rows read, {:?} (fixture build {:?})",
            s.relations.len(),
            s.column_count(),
            c.rows_read,
            took,
            build
        );
        c.rows_read = 0;
    }
    let (snap, token) = sqlite::introspect(&mut c, "main", Level::Structure)
        .await
        .unwrap();
    let t = Instant::now();
    let (_, _, o) = sqlite::refresh(&mut c, &snap, &token, Level::Structure, false)
        .await
        .unwrap();
    println!("PERF sqlite unchanged check: {o:?} in {:?}", t.elapsed());
    let t = Instant::now();
    let (_, _, o) = sqlite::refresh(&mut c, &snap, &token, Level::Structure, true)
        .await
        .unwrap();
    println!("PERF sqlite digest check: {o:?} in {:?}", t.elapsed());
    drop(c);
    raw.close().await;
}
