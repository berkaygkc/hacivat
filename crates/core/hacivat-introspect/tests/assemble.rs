//! Composite object assembly (HV-INTRO-13; M4 Task 13): keys, indexes, foreign keys, checks and
//! exclusion constraints built from per-dialect catalog rows. Row fixtures for alg/02
//! HV-INTRO-13 test cases 1–5 and 7; the Docker variants run in Tasks 16–18.

use std::sync::Arc;

use hacivat_introspect::assemble::{ColRef, ColumnIndex, KeyObj, TableId, mysql, pg, sqlite};
use hacivat_meta::merge::{ChangedObject, MergePolicy, NamespaceResult, base, merge};
use hacivat_meta::name::{DialectId, FoldRule, IdentCtx, Name};
use hacivat_meta::namespace::{Level, Obj, ObjFlags, Subtree, Watermark};
use hacivat_meta::props::{ColumnProps, FkAction, FkMatch, KeyColumn, Props, TableProps};
use hacivat_meta::{NsKey, ObjectKind, Ref, ServerId};
use hacivat_proto::DbmsKind;

// ---- helpers ---------------------------------------------------------------------------------

fn n(s: &str) -> Name {
    Arc::from(s)
}

fn path(parts: &[&str]) -> Vec<Name> {
    parts.iter().map(|p| n(p)).collect()
}

fn kc(ordinal: u32, name: &str) -> KeyColumn {
    KeyColumn {
        ordinal,
        name: Some(n(name)),
    }
}

/// The expression slot of a key (`ordinal` 0, no name; its text is in `expressions`).
fn expr_slot() -> KeyColumn {
    KeyColumn {
        ordinal: 0,
        name: None,
    }
}

/// The PostgreSQL schema `db.s` with table `t` (OID 100): `a`, `b`, `name`, `id`, `active`,
/// `room`, `during` (attnums 1–7), and table `p` (OID 101): `id`, `k`.
fn pg_cols() -> ColumnIndex {
    let mut c = ColumnIndex::new(
        NsKey::schema(["db", "s"]),
        DialectId::Postgres,
        IdentCtx::new(DialectId::Postgres),
    );
    c.add_table(
        TableId::with_oid(100, "t"),
        ["a", "b", "name", "id", "active", "room", "during"]
            .iter()
            .enumerate()
            .map(|(i, c)| (i as u32 + 1, n(c))),
    );
    c.add_table(TableId::with_oid(101, "p"), [(1, n("id")), (2, n("k"))]);
    c
}

fn t_pg() -> TableId {
    TableId::with_oid(100, "t")
}

fn pg_part(attnum: i16, name: &str) -> pg::IndexPart {
    pg::IndexPart {
        attnum,
        name: Some(n(name)),
        expression: None,
        desc: false,
        ..Default::default()
    }
}

fn pg_index(oid: u32, name: &str, parts: Vec<pg::IndexPart>) -> pg::IndexRow {
    pg::IndexRow {
        table: t_pg(),
        oid,
        name: n(name),
        method: Some(n("btree")),
        unique: false,
        valid: true,
        nulls_not_distinct: false,
        parts,
        include: Vec::new(),
        predicate: None,
        comment: None,
    }
}

fn pg_con(oid: u32, name: &str, contype: u8, columns: Vec<ColRef>) -> pg::ConstraintRow {
    pg::ConstraintRow {
        table: t_pg(),
        oid,
        name: n(name),
        contype,
        columns,
        definition: n(""),
        check_expr: None,
        deferrable: false,
        deferred: false,
        validated: true,
        enforced: true,
        no_inherit: false,
        local: true,
        parent: None,
        index: None,
        foreign: None,
        operators: Vec::new(),
    }
}

fn pg_fk(
    oid: u32,
    name: &str,
    columns: Vec<ColRef>,
    target: TableId,
    schema: &str,
    target_columns: Vec<ColRef>,
) -> pg::ConstraintRow {
    let mut c = pg_con(oid, name, b'f', columns);
    c.foreign = Some(pg::FkTarget {
        table: target,
        schema: n(schema),
        columns: target_columns,
        on_update: b'a',
        on_delete: b'c',
        match_type: b's',
        index: None,
    });
    c
}

fn find<'a>(objs: &'a [KeyObj], kind: ObjectKind, name: &str) -> &'a KeyObj {
    objs.iter()
        .find(|o| o.obj.kind() == kind && &**o.obj.name() == name)
        .unwrap_or_else(|| panic!("no {} {name} in {objs:#?}", kind.id()))
}

fn find_in<'a>(objs: &'a [KeyObj], table: &str, kind: ObjectKind, name: &str) -> &'a KeyObj {
    objs.iter()
        .find(|o| &*o.table.name == table && o.obj.kind() == kind && &**o.obj.name() == name)
        .unwrap_or_else(|| panic!("no {} {table}.{name} in {objs:#?}", kind.id()))
}

/// MySQL schema `shop` with `cols` per table (ordinals 1..).
fn my_cols(tables: &[(&str, &[&str])]) -> ColumnIndex {
    let mut c = ColumnIndex::new(
        NsKey::schema(["shop"]),
        DialectId::MySql,
        IdentCtx::new(DialectId::MySql),
    );
    for (t, cols) in tables {
        c.add_table(
            TableId::named(*t),
            cols.iter().enumerate().map(|(i, c)| (i as u32 + 1, n(c))),
        );
    }
    c
}

fn stat(table: &str, index: &str, seq: u32, column: &str) -> mysql::StatRow {
    mysql::StatRow {
        table: n(table),
        index: n(index),
        seq_in_index: seq,
        column: Some(n(column)),
        expression: None,
        collation: Some(n("A")),
        sub_part: None,
        non_unique: true,
        index_type: Some(n("BTREE")),
        visible: true,
        comment: None,
    }
}

fn my_rows(indexes: Vec<mysql::StatRow>) -> mysql::KeyRows {
    mysql::KeyRows {
        indexes,
        ..mysql::KeyRows::new(mysql::Flavor::MySql)
    }
}

/// SQLite `main` with `cols` per table (ordinal = `cid + 1`).
fn lite_cols(tables: &[(&str, &[&str])]) -> ColumnIndex {
    let mut c = ColumnIndex::new(
        NsKey::schema(["main"]),
        DialectId::Sqlite,
        IdentCtx::new(DialectId::Sqlite),
    );
    for (t, cols) in tables {
        c.add_table(
            TableId::named(*t),
            cols.iter().enumerate().map(|(i, c)| (i as u32 + 1, n(c))),
        );
    }
    c
}

fn lite_index(table: &str, name: &str, origin: &str, unique: bool) -> sqlite::IndexRow {
    sqlite::IndexRow {
        table: n(table),
        name: n(name),
        unique,
        origin: n(origin),
        partial: false,
        sql: None,
    }
}

fn lite_part(
    index: &str,
    seqno: u32,
    cid: i64,
    name: Option<&str>,
    key: bool,
) -> sqlite::IndexPartRow {
    sqlite::IndexPartRow {
        index: n(index),
        seqno,
        cid,
        name: name.map(n),
        desc: false,
        coll: None,
        key,
    }
}

/// `CREATE TABLE t(a UNIQUE, b, PRIMARY KEY(b))` as SQLite's pragmas report it (alg/02
/// HV-INTRO-13 test 5).
fn lite_unique_and_pk() -> (sqlite::KeyRows, ColumnIndex) {
    let rows = sqlite::KeyRows {
        pk_columns: vec![sqlite::PkColumnRow {
            table: n("t"),
            cid: 1,
            name: n("b"),
            pk: 1,
        }],
        indexes: vec![
            lite_index("t", "sqlite_autoindex_t_1", "u", true),
            lite_index("t", "sqlite_autoindex_t_2", "pk", true),
        ],
        index_parts: vec![
            lite_part("sqlite_autoindex_t_1", 0, 0, Some("a"), true),
            lite_part("sqlite_autoindex_t_1", 1, -1, None, false),
            lite_part("sqlite_autoindex_t_2", 0, 1, Some("b"), true),
            lite_part("sqlite_autoindex_t_2", 1, -1, None, false),
        ],
        foreign_keys: Vec::new(),
    };
    (rows, lite_cols(&[("t", &["a", "b"])]))
}

fn lite_fk(
    table: &str,
    id: u32,
    seq: u32,
    ref_table: &str,
    from: &str,
    to: Option<&str>,
) -> sqlite::FkRow {
    sqlite::FkRow {
        table: n(table),
        id,
        seq,
        ref_table: n(ref_table),
        from: n(from),
        to: to.map(n),
        on_update: n("NO ACTION"),
        on_delete: n("CASCADE"),
        match_kind: n("NONE"),
        deferrable: false,
        initially_deferred: false,
    }
}

// ---- alg/02 HV-INTRO-13 test cases and the plan's list ---------------------------------------

/// Test 2: `PRIMARY KEY (b, a)` keeps key order `[b, a]`, never attnum or name order.
#[test]
fn key_columns_in_key_order() {
    let mut pk = pg_con(
        300,
        "t_pkey",
        b'p',
        vec![ColRef::at(2, "b"), ColRef::at(1, "a")],
    );
    pk.index = Some(200);
    let rows = pg::KeyRows {
        indexes: vec![{
            let mut i = pg_index(200, "t_pkey", vec![pg_part(2, "b"), pg_part(1, "a")]);
            i.unique = true;
            i
        }],
        constraints: vec![pk],
    };
    let out = pg::assemble_keys(rows, &pg_cols());
    let pk = find(&out, ObjectKind::PrimaryKey, "t_pkey");
    let key = pk.key().expect("key props");
    assert_eq!(key.columns, vec![kc(2, "b"), kc(1, "a")]);
    assert_eq!(key.desc, vec![false, false]);
    assert!(key.unique && key.enforced);
    assert!(!pk.obj.flags().contains(ObjFlags::BROKEN));
    assert_eq!(pk.obj.server_id(), Some(ServerId::Oid(300)));
    let idx = find(&out, ObjectKind::Index, "t_pkey");
    assert_eq!(idx.key().unwrap().columns, vec![kc(2, "b"), kc(1, "a")]);
}

/// Test 1: `CREATE INDEX idx_ab ON t(a, b DESC)` on MySQL 8.4 → `[a asc, b desc]`.
#[test]
fn mysql_index_with_a_descending_part() {
    let mut b = stat("t", "idx_ab", 2, "b");
    b.collation = Some(n("D"));
    let rows = my_rows(vec![b, stat("t", "idx_ab", 1, "a")]);
    let out = mysql::assemble_keys(rows, &my_cols(&[("t", &["a", "b"])]));
    let idx = find(&out, ObjectKind::Index, "idx_ab");
    let p = idx.index().expect("index props");
    assert_eq!(p.key.columns, vec![kc(1, "a"), kc(2, "b")]);
    assert_eq!(p.key.desc, vec![false, true]);
    assert!(!p.key.unique);
    assert_eq!(p.method.as_deref(), Some("BTREE"));
    assert_eq!(p.prefix_len, vec![None, None]);
    assert!(!idx.obj.flags().contains(ObjFlags::BROKEN));
}

/// Test 3: `CREATE INDEX ON t (lower(name)) INCLUDE (id) WHERE active`.
#[test]
fn expression_include_and_partial_index() {
    let mut i = pg_index(
        210,
        "t_lower_idx",
        vec![pg::IndexPart {
            attnum: 0,
            name: None,
            expression: Some(n("lower(name)")),
            ..Default::default()
        }],
    );
    i.include = vec![ColRef::at(4, "id")];
    i.predicate = Some(n("active"));
    let out = pg::assemble_keys(
        pg::KeyRows {
            indexes: vec![i],
            constraints: Vec::new(),
        },
        &pg_cols(),
    );
    let idx = find(&out, ObjectKind::Index, "t_lower_idx");
    let k = idx.key().unwrap();
    assert_eq!(k.columns, vec![expr_slot()]);
    assert_eq!(k.expressions, vec![n("lower(name)")]);
    assert_eq!(k.include, vec![n("id")]);
    assert_eq!(k.predicate.as_deref(), Some("active"));
    assert!(
        !idx.obj
            .flags()
            .intersects(ObjFlags::BROKEN | ObjFlags::IMPLICIT)
    );
}

/// Test 4: an FK to a table in an out-of-scope schema is kept, its target unresolved (never
/// broken for that); an FK to an in-scope table whose column does not resolve is broken.
#[test]
fn fk_to_an_out_of_scope_table_is_kept_unresolved() {
    let out = pg::assemble_keys(
        pg::KeyRows {
            indexes: Vec::new(),
            constraints: vec![
                pg_fk(
                    310,
                    "t_a_fkey",
                    vec![ColRef::at(1, "a")],
                    TableId::with_oid(900, "u"),
                    "other",
                    vec![ColRef::at(1, "uid")],
                ),
                pg_fk(
                    311,
                    "t_b_fkey",
                    vec![ColRef::at(2, "b")],
                    TableId::with_oid(101, "p"),
                    "s",
                    vec![ColRef::at(9, "gone")],
                ),
            ],
        },
        &pg_cols(),
    );
    let fk = find(&out, ObjectKind::ForeignKey, "t_a_fkey");
    assert!(!fk.obj.flags().contains(ObjFlags::BROKEN));
    let p = fk.fk().expect("fk props");
    assert_eq!(
        p.target,
        Ref {
            kind: ObjectKind::Table,
            path: path(&["db", "other", "u"]).into_iter().collect(),
            server_id: Some(ServerId::Oid(900)),
        }
    );
    assert_eq!(p.columns, vec![n("a")]);
    assert_eq!(p.target_columns, vec![n("uid")]);
    assert_eq!(p.on_update, FkAction::NoAction);
    assert_eq!(p.on_delete, FkAction::Cascade);
    assert_eq!(p.match_kind, FkMatch::Simple);

    let broken = find(&out, ObjectKind::ForeignKey, "t_b_fkey");
    assert!(broken.obj.flags().contains(ObjFlags::BROKEN));
    assert_eq!(broken.fk().unwrap().target_columns, vec![n("gone")]);
}

/// Test 5: `CREATE TABLE t(a UNIQUE, b, PRIMARY KEY(b))` → PK `[b]`, a unique key on `a` named
/// by the server `sqlite_autoindex_t_1`.
#[test]
fn sqlite_autoindex_unique_has_server_generated_name() {
    let (rows, cols) = lite_unique_and_pk();
    let out = sqlite::assemble_keys(rows, &cols);
    let uk = find(&out, ObjectKind::UniqueKey, "sqlite_autoindex_t_1");
    assert!(uk.obj.flags().contains(ObjFlags::SERVER_GENERATED_NAME));
    assert!(!uk.obj.flags().contains(ObjFlags::BROKEN));
    let k = uk.key().unwrap();
    assert_eq!(k.columns, vec![kc(1, "a")]);
    assert!(k.unique);
    assert_eq!(k.backing_index.as_deref(), Some("sqlite_autoindex_t_1"));
    let pk = find(&out, ObjectKind::PrimaryKey, "t_pk");
    assert_eq!(pk.key().unwrap().columns, vec![kc(2, "b")]);
}

/// A SQLite constraint the server leaves unnamed gets a display name flagged
/// `SERVER_GENERATED_NAME` (DDL never emits it): the primary key `<table>_pk`, and each foreign
/// key a distinct name derived from its content (`<table>_<from>_<parent>_fkey`), so siblings
/// never share an identity.
#[test]
fn sqlite_unnamed_pk_gets_a_display_name_only() {
    let (mut rows, _) = lite_unique_and_pk();
    rows.foreign_keys = vec![
        lite_fk("t", 0, 0, "p", "b", Some("id")),
        lite_fk("t", 1, 0, "q", "a", None),
    ];
    let cols = lite_cols(&[("t", &["a", "b"]), ("p", &["id"]), ("q", &["k"])]);
    let out = sqlite::assemble_keys(rows, &cols);
    let pk = find(&out, ObjectKind::PrimaryKey, "t_pk");
    assert!(pk.obj.flags().contains(ObjFlags::SERVER_GENERATED_NAME));
    assert!(!pk.obj.flags().contains(ObjFlags::IMPLICIT));
    assert!(
        !out.iter()
            .any(|o| o.obj.kind() == ObjectKind::Index && &**o.obj.name() == "t_pk"),
        "the display name names no index"
    );
    let first = find(&out, ObjectKind::ForeignKey, "t_a_q_fkey");
    let second = find(&out, ObjectKind::ForeignKey, "t_b_p_fkey");
    assert_eq!(first.fk().unwrap().columns, vec![n("a")]);
    assert_eq!(second.fk().unwrap().columns, vec![n("b")]);
    for o in &out {
        let real = o.obj.name().starts_with("sqlite_autoindex_");
        assert!(
            o.obj.flags().contains(ObjFlags::SERVER_GENERATED_NAME),
            "{} {} must never reach DDL as a real name (real server name: {real})",
            o.obj.kind().id(),
            o.obj.name()
        );
    }
}

/// An FK whose column count differs from its referenced column count is kept, flagged broken.
#[test]
fn fk_column_count_mismatch_is_broken_and_kept() {
    let out = pg::assemble_keys(
        pg::KeyRows {
            indexes: Vec::new(),
            constraints: vec![pg_fk(
                320,
                "t_ab_fkey",
                vec![ColRef::at(1, "a"), ColRef::at(2, "b")],
                TableId::with_oid(101, "p"),
                "s",
                vec![ColRef::at(1, "id")],
            )],
        },
        &pg_cols(),
    );
    let fk = find(&out, ObjectKind::ForeignKey, "t_ab_fkey");
    assert!(fk.obj.flags().contains(ObjFlags::BROKEN));
    let p = fk.fk().unwrap();
    assert_eq!(p.columns, vec![n("a"), n("b")]);
    assert_eq!(p.target_columns, vec![n("id")]);
}

/// A PK backed by an index of another name links it (`backing_index`), and the index is the
/// PK's implicit index.
#[test]
fn pk_backed_by_an_index_of_another_name_links_it() {
    let mut pk = pg_con(330, "pk", b'p', vec![ColRef::at(4, "id")]);
    pk.index = Some(230);
    let mut i = pg_index(230, "t_custom_idx", vec![pg_part(4, "id")]);
    i.unique = true;
    let out = pg::assemble_keys(
        pg::KeyRows {
            indexes: vec![i],
            constraints: vec![pk],
        },
        &pg_cols(),
    );
    let pk = find(&out, ObjectKind::PrimaryKey, "pk");
    assert_eq!(
        pk.key().unwrap().backing_index.as_deref(),
        Some("t_custom_idx")
    );
    let idx = find(&out, ObjectKind::Index, "t_custom_idx");
    assert!(idx.obj.flags().contains(ObjFlags::IMPLICIT));
    assert_eq!(
        idx.key().unwrap().implicit_for,
        Some(Ref {
            kind: ObjectKind::PrimaryKey,
            path: path(&["db", "s", "t", "pk"]).into_iter().collect(),
            server_id: Some(ServerId::Oid(330)),
        })
    );
}

/// Key parts arrive one row each, interleaved across tables and indexes, and are grouped
/// client-side by `(table, index)` in `SEQ_IN_INDEX` order (no `GROUP_CONCAT`).
#[test]
fn mysql_key_parts_grouped_client_side_in_seq_order() {
    let rows = my_rows(vec![
        stat("t2", "ix", 2, "y"),
        stat("t1", "ix", 3, "c"),
        stat("t1", "ix", 1, "a"),
        stat("t2", "ix", 1, "x"),
        stat("t1", "other", 1, "c"),
        stat("t1", "ix", 2, "b"),
    ]);
    let cols = my_cols(&[("t1", &["a", "b", "c"]), ("t2", &["x", "y"])]);
    let out = mysql::assemble_keys(rows, &cols);
    assert_eq!(out.len(), 3);
    assert_eq!(
        find_in(&out, "t1", ObjectKind::Index, "ix")
            .key()
            .unwrap()
            .columns,
        vec![kc(1, "a"), kc(2, "b"), kc(3, "c")]
    );
    assert_eq!(
        find_in(&out, "t2", ObjectKind::Index, "ix")
            .key()
            .unwrap()
            .columns,
        vec![kc(1, "x"), kc(2, "y")]
    );
    assert_eq!(
        find_in(&out, "t1", ObjectKind::Index, "other")
            .key()
            .unwrap()
            .columns,
        vec![kc(3, "c")]
    );
    // A gap in the sequence is a missing part: kept, broken.
    let gap = mysql::assemble_keys(
        my_rows(vec![stat("t1", "ix", 1, "a"), stat("t1", "ix", 3, "c")]),
        &cols,
    );
    assert!(gap[0].obj.flags().contains(ObjFlags::BROKEN));
}

/// A PostgreSQL exclusion constraint keeps its operator per key part and its raw definition;
/// its backing index is implicit for it.
#[test]
fn exclusion_constraint_keeps_operators() {
    let mut x = pg_con(
        340,
        "t_room_during_excl",
        b'x',
        vec![ColRef::at(6, "room"), ColRef::at(7, "during")],
    );
    x.definition = n("EXCLUDE USING gist (room WITH =, during WITH &&)");
    x.operators = vec![n("="), n("&&")];
    x.index = Some(240);
    let mut i = pg_index(
        240,
        "t_room_during_excl",
        vec![pg_part(6, "room"), pg_part(7, "during")],
    );
    i.method = Some(n("gist"));
    let out = pg::assemble_keys(
        pg::KeyRows {
            indexes: vec![i],
            constraints: vec![x],
        },
        &pg_cols(),
    );
    let x = find(&out, ObjectKind::Exclusion, "t_room_during_excl");
    assert_eq!(x.check().unwrap().excl_ops(), &[n("="), n("&&")][..]);
    assert_eq!(
        &*x.check().unwrap().expr,
        "USING gist (room WITH =, during WITH &&)"
    );
    assert!(!x.obj.flags().contains(ObjFlags::BROKEN));
    let idx = find(&out, ObjectKind::Index, "t_room_during_excl");
    assert!(idx.obj.flags().contains(ObjFlags::IMPLICIT));
    assert_eq!(
        idx.key().unwrap().implicit_for.as_ref().map(|r| r.kind),
        Some(ObjectKind::Exclusion)
    );

    // One operator short: kept, broken.
    let mut short = pg_con(
        341,
        "t_x2",
        b'x',
        vec![ColRef::at(6, "room"), ColRef::at(7, "during")],
    );
    short.operators = vec![n("=")];
    let out = pg::assemble_keys(
        pg::KeyRows {
            indexes: Vec::new(),
            constraints: vec![short],
        },
        &pg_cols(),
    );
    assert!(out[0].obj.flags().contains(ObjFlags::BROKEN));
}

/// Shuffled rows give the same objects in the same order: by table, then by name.
#[test]
fn output_order_is_deterministic() {
    let rows = || {
        let mut pk = pg_con(300, "t_pkey", b'p', vec![ColRef::at(4, "id")]);
        pk.index = Some(200);
        let mut ppk = pg_con(301, "p_pkey", b'p', vec![ColRef::at(1, "id")]);
        ppk.table = TableId::with_oid(101, "p");
        let mut uk = pg_con(302, "t_a_key", b'u', vec![ColRef::at(1, "a")]);
        uk.index = Some(201);
        let mut i0 = pg_index(200, "t_pkey", vec![pg_part(4, "id")]);
        i0.unique = true;
        let mut i1 = pg_index(201, "t_a_key", vec![pg_part(1, "a")]);
        i1.unique = true;
        let i2 = pg_index(202, "b_idx", vec![pg_part(2, "b")]);
        let fk = pg_fk(
            303,
            "t_b_fkey",
            vec![ColRef::at(2, "b")],
            TableId::with_oid(101, "p"),
            "s",
            vec![ColRef::at(1, "id")],
        );
        (vec![i0, i1, i2], vec![pk, ppk, uk, fk])
    };
    let (i, c) = rows();
    let a = pg::assemble_keys(
        pg::KeyRows {
            indexes: i,
            constraints: c,
        },
        &pg_cols(),
    );
    let (mut i, mut c) = rows();
    i.reverse();
    c.rotate_left(2);
    let b = pg::assemble_keys(
        pg::KeyRows {
            indexes: i,
            constraints: c,
        },
        &pg_cols(),
    );
    assert_eq!(a, b);
    let order: Vec<(String, String)> = a
        .iter()
        .map(|o| (o.table.name.to_string(), o.obj.name().to_string()))
        .collect();
    let mut sorted = order.clone();
    sorted.sort();
    assert_eq!(order, sorted);
    assert_eq!(order.first().map(|(t, _)| t.as_str()), Some("p"));
}

/// Test 7: 600 indexed columns across 40 indexes of one table assemble completely: every part,
/// in order, none broken (client-side grouping has no length limit).
#[test]
fn six_hundred_indexed_columns_assemble_without_truncation() {
    let names: Vec<String> = (1..=600)
        .map(|i| format!("column_with_a_long_name_{i:04}"))
        .collect();
    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let cols = my_cols(&[("wide", &refs)]);
    let mut rows = Vec::new();
    for ix in 0..40u32 {
        for part in 0..15u32 {
            let col = &names[(ix * 15 + part) as usize];
            rows.push(stat("wide", &format!("ix_{ix:02}"), part + 1, col));
        }
    }
    // A deterministic shuffle: rows arrive in no particular order.
    let len = rows.len();
    let mut shuffled = Vec::with_capacity(len);
    for i in 0..len {
        shuffled.push(rows[(i * 367) % len].clone());
    }
    let out = mysql::assemble_keys(my_rows(shuffled), &cols);
    assert_eq!(out.len(), 40);
    let mut total = 0;
    for (ix, o) in out.iter().enumerate() {
        assert_eq!(&**o.obj.name(), format!("ix_{ix:02}"));
        assert!(!o.obj.flags().contains(ObjFlags::BROKEN));
        let k = o.key().unwrap();
        assert_eq!(k.columns.len(), 15);
        for (part, c) in k.columns.iter().enumerate() {
            let at = ix * 15 + part;
            assert_eq!(c, &kc(at as u32 + 1, &names[at]));
        }
        total += k.columns.len();
    }
    assert_eq!(total, 600);
}

/// PostgreSQL: the index behind a PK (through `conindid`) is implicit for the PK.
#[test]
fn pg_pk_backing_index_is_implicit_for_the_pk() {
    let mut pk = pg_con(300, "t_pkey", b'p', vec![ColRef::at(4, "id")]);
    pk.index = Some(200);
    // An FK's `conindid` names the referenced table's index: never an implicit link.
    let mut fk = pg_fk(
        301,
        "t_b_fkey",
        vec![ColRef::at(2, "b")],
        TableId::with_oid(101, "p"),
        "s",
        vec![ColRef::at(1, "id")],
    );
    fk.index = Some(202);
    if let Some(f) = fk.foreign.as_mut() {
        f.index = Some((202, n("b_idx")));
    }
    let mut i = pg_index(200, "t_pkey", vec![pg_part(4, "id")]);
    i.unique = true;
    let plain = pg_index(202, "b_idx", vec![pg_part(2, "b")]);
    let out = pg::assemble_keys(
        pg::KeyRows {
            indexes: vec![i, plain],
            constraints: vec![pk, fk],
        },
        &pg_cols(),
    );
    let idx = find(&out, ObjectKind::Index, "t_pkey");
    assert!(idx.obj.flags().contains(ObjFlags::IMPLICIT));
    assert_eq!(
        idx.key().unwrap().implicit_for,
        Some(Ref {
            kind: ObjectKind::PrimaryKey,
            path: path(&["db", "s", "t", "t_pkey"]).into_iter().collect(),
            server_id: Some(ServerId::Oid(300)),
        })
    );
    assert_eq!(
        find(&out, ObjectKind::PrimaryKey, "t_pkey")
            .key()
            .unwrap()
            .backing_index
            .as_deref(),
        Some("t_pkey")
    );
    let plain = find(&out, ObjectKind::Index, "b_idx");
    assert!(!plain.obj.flags().contains(ObjFlags::IMPLICIT));
    assert_eq!(plain.key().unwrap().implicit_for, None);
    let fk = find(&out, ObjectKind::ForeignKey, "t_b_fkey");
    assert_eq!(
        fk.fk().unwrap().fk_ref_index().cloned(),
        Some(Ref {
            kind: ObjectKind::Index,
            path: path(&["db", "s", "p", "b_idx"]).into_iter().collect(),
            server_id: Some(ServerId::Oid(202)),
        })
    );
}

/// SQLite `sqlite_autoindex_*` indexes are implicit for their key (`origin` `u` or `pk`); a
/// `CREATE INDEX` (`origin` `c`) is not.
#[test]
fn sqlite_autoindex_is_implicit() {
    let (mut rows, cols) = lite_unique_and_pk();
    rows.indexes.push(lite_index("t", "t_b", "c", false));
    rows.index_parts
        .push(lite_part("t_b", 0, 1, Some("b"), true));
    let out = sqlite::assemble_keys(rows, &cols);
    let u = find(&out, ObjectKind::Index, "sqlite_autoindex_t_1");
    assert!(
        u.obj
            .flags()
            .contains(ObjFlags::IMPLICIT | ObjFlags::SERVER_GENERATED_NAME)
    );
    assert_eq!(
        u.key().unwrap().implicit_for,
        Some(Ref {
            kind: ObjectKind::UniqueKey,
            path: path(&["main", "t", "sqlite_autoindex_t_1"])
                .into_iter()
                .collect(),
            server_id: None,
        })
    );
    let pk_idx = find(&out, ObjectKind::Index, "sqlite_autoindex_t_2");
    assert!(pk_idx.obj.flags().contains(ObjFlags::IMPLICIT));
    assert_eq!(
        pk_idx.key().unwrap().implicit_for,
        Some(Ref {
            kind: ObjectKind::PrimaryKey,
            path: path(&["main", "t", "t_pk"]).into_iter().collect(),
            server_id: None,
        })
    );
    assert_eq!(
        find(&out, ObjectKind::PrimaryKey, "t_pk")
            .key()
            .unwrap()
            .backing_index
            .as_deref(),
        Some("sqlite_autoindex_t_2")
    );
    let plain = find(&out, ObjectKind::Index, "t_b");
    assert!(
        !plain
            .obj
            .flags()
            .intersects(ObjFlags::IMPLICIT | ObjFlags::SERVER_GENERATED_NAME)
    );
    assert_eq!(plain.key().unwrap().implicit_for, None);
}

/// PostgreSQL ≥ 15 `UNIQUE NULLS NOT DISTINCT`: the flag lives on the index and is carried to
/// the key it backs (with the index's `INCLUDE` columns).
#[test]
fn nulls_not_distinct_carried_to_key_and_index() {
    let mut uk = pg_con(350, "t_a_key", b'u', vec![ColRef::at(1, "a")]);
    uk.index = Some(250);
    let mut i = pg_index(250, "t_a_key", vec![pg_part(1, "a")]);
    i.unique = true;
    i.nulls_not_distinct = true;
    i.include = vec![ColRef::at(2, "b")];
    let out = pg::assemble_keys(
        pg::KeyRows {
            indexes: vec![i],
            constraints: vec![uk],
        },
        &pg_cols(),
    );
    let k = find(&out, ObjectKind::UniqueKey, "t_a_key")
        .key()
        .unwrap()
        .clone();
    assert!(k.nulls_not_distinct && k.unique);
    assert_eq!(k.include, vec![n("b")]);
    assert!(
        find(&out, ObjectKind::Index, "t_a_key")
            .key()
            .unwrap()
            .nulls_not_distinct
    );
}

/// MySQL 8.0.13+ functional key part: `COLUMN_NAME` NULL, `EXPRESSION` set → an expression
/// slot with the server's text; a part naming no column of the table is kept, broken.
#[test]
fn mysql_functional_key_part_keeps_its_expression() {
    let mut f = stat("t", "fx", 2, "unused");
    f.column = None;
    f.expression = Some(n("lower(`name`)"));
    let mut p = stat("t", "fx", 3, "name");
    p.sub_part = Some(10);
    let out = mysql::assemble_keys(
        my_rows(vec![f, stat("t", "fx", 1, "id"), p]),
        &my_cols(&[("t", &["id", "name"])]),
    );
    let idx = find(&out, ObjectKind::Index, "fx");
    let ip = idx.index().unwrap();
    assert_eq!(
        ip.key.columns,
        vec![kc(1, "id"), expr_slot(), kc(2, "name")]
    );
    assert_eq!(ip.key.expressions, vec![n("lower(`name`)")]);
    assert_eq!(ip.prefix_len, vec![None, None, Some(10)]);
    assert!(!idx.obj.flags().contains(ObjFlags::BROKEN));

    let out = mysql::assemble_keys(
        my_rows(vec![stat("t", "bad", 1, "nope")]),
        &my_cols(&[("t", &["id", "name"])]),
    );
    let bad = find(&out, ObjectKind::Index, "bad");
    assert!(bad.obj.flags().contains(ObjFlags::BROKEN));
    assert_eq!(
        bad.key().unwrap().columns,
        vec![KeyColumn {
            ordinal: 0,
            name: Some(n("nope"))
        }]
    );
}

/// MySQL CHECK names are unique per schema and joined by name; MariaDB's are per table and
/// joined by `(TABLE_NAME, CONSTRAINT_NAME)` (F9).
#[test]
fn mariadb_check_joined_by_table_and_name_mysql_by_name() {
    let cols = my_cols(&[("t1", &["a"]), ("t2", &["a"])]);
    let check = |table: &str, name: &str, enforced| mysql::CheckRow {
        table: n(table),
        name: n(name),
        enforced,
    };
    let clause = |table: Option<&str>, name: &str, text: &str| mysql::CheckClauseRow {
        table: table.map(n),
        name: n(name),
        clause: n(text),
    };

    let mut maria = mysql::KeyRows::new(mysql::Flavor::MariaDb);
    maria.checks = vec![check("t1", "c", true), check("t2", "c", true)];
    maria.check_clauses = vec![
        clause(Some("t2"), "c", "`a` < 10"),
        clause(Some("t1"), "c", "`a` > 0"),
    ];
    let out = mysql::assemble_keys(maria, &cols);
    assert_eq!(
        &*find_in(&out, "t1", ObjectKind::Check, "c")
            .check()
            .unwrap()
            .expr,
        "`a` > 0"
    );
    assert_eq!(
        &*find_in(&out, "t2", ObjectKind::Check, "c")
            .check()
            .unwrap()
            .expr,
        "`a` < 10"
    );
    assert!(
        out.iter()
            .all(|o| !o.obj.flags().contains(ObjFlags::BROKEN))
    );

    let mut my = mysql::KeyRows::new(mysql::Flavor::MySql);
    my.checks = vec![
        check("t1", "t1_chk_1", false),
        check("t2", "t2_chk_1", true),
    ];
    my.check_clauses = vec![
        clause(None, "t2_chk_1", "(`a` < 10)"),
        clause(None, "t1_chk_1", "(`a` > 0)"),
    ];
    let out = mysql::assemble_keys(my, &cols);
    let c1 = find_in(&out, "t1", ObjectKind::Check, "t1_chk_1");
    assert_eq!(&*c1.check().unwrap().expr, "(`a` > 0)");
    assert!(!c1.check().unwrap().enforced);
    assert_eq!(
        &*find_in(&out, "t2", ObjectKind::Check, "t2_chk_1")
            .check()
            .unwrap()
            .expr,
        "(`a` < 10)"
    );

    // MariaDB without a matching table: no clause is borrowed from another table.
    let mut orphan = mysql::KeyRows::new(mysql::Flavor::MariaDb);
    orphan.checks = vec![check("t1", "c", true)];
    orphan.check_clauses = vec![clause(Some("t2"), "c", "`a` < 10")];
    let out = mysql::assemble_keys(orphan, &cols);
    let c = find_in(&out, "t1", ObjectKind::Check, "c");
    assert!(c.obj.flags().contains(ObjFlags::BROKEN));
    assert_eq!(&*c.check().unwrap().expr, "");
}

// ---- beyond the plan's list ------------------------------------------------------------------

/// MySQL: `PRIMARY` and unique indexes are keys whose same-named index is implicit for them;
/// the PK's name is the server's own.
#[test]
fn mysql_primary_and_unique_are_keys_with_implicit_indexes() {
    let mut pk = stat("t", "PRIMARY", 1, "id");
    pk.non_unique = false;
    let mut uk = stat("t", "u_name", 1, "name");
    uk.non_unique = false;
    uk.sub_part = Some(20);
    uk.visible = false;
    let out = mysql::assemble_keys(my_rows(vec![uk, pk]), &my_cols(&[("t", &["id", "name"])]));
    let key = find(&out, ObjectKind::PrimaryKey, "PRIMARY");
    assert!(key.obj.flags().contains(ObjFlags::SERVER_GENERATED_NAME));
    assert_eq!(key.key().unwrap().columns, vec![kc(1, "id")]);
    assert_eq!(key.key().unwrap().backing_index.as_deref(), Some("PRIMARY"));
    let idx = find(&out, ObjectKind::Index, "PRIMARY");
    assert!(idx.obj.flags().contains(ObjFlags::IMPLICIT));
    let u = find(&out, ObjectKind::UniqueKey, "u_name");
    assert!(u.key().unwrap().invisible && u.key().unwrap().unique);
    let ui = find(&out, ObjectKind::Index, "u_name");
    assert_eq!(ui.index().unwrap().prefix_len, vec![Some(20)]);
    assert_eq!(
        ui.key().unwrap().implicit_for,
        Some(Ref {
            kind: ObjectKind::UniqueKey,
            path: path(&["shop", "t", "u_name"]).into_iter().collect(),
            server_id: None,
        })
    );
}

/// MySQL FKs: columns grouped by `ORDINAL_POSITION`, referenced columns by position, the parent
/// key named by `UNIQUE_CONSTRAINT_NAME`; the FK-created index is implicit only under the rule
/// Task 17's probe pins.
#[test]
fn mysql_foreign_key_and_its_created_index() {
    let cols = my_cols(&[("t", &["id", "a", "b"]), ("p", &["x", "y"])]);
    let rows = |rule| {
        let mut r = my_rows(vec![stat("t", "fk_ab", 1, "a"), stat("t", "fk_ab", 2, "b")]);
        r.fk_index = rule;
        r.foreign_keys = vec![mysql::FkRow {
            table: n("t"),
            name: n("fk_ab"),
            referenced_schema: n("shop"),
            referenced_table: n("p"),
            unique_constraint_name: Some(n("PRIMARY")),
            update_rule: n("RESTRICT"),
            delete_rule: n("SET NULL"),
            match_option: n("NONE"),
        }];
        r.fk_columns = vec![
            mysql::FkColumnRow {
                table: n("t"),
                constraint: n("fk_ab"),
                column: n("b"),
                ordinal_position: 2,
                referenced_column: Some(n("y")),
            },
            mysql::FkColumnRow {
                table: n("t"),
                constraint: n("fk_ab"),
                column: n("a"),
                ordinal_position: 1,
                referenced_column: Some(n("x")),
            },
        ];
        r
    };
    let out = mysql::assemble_keys(rows(mysql::FkIndexRule::NamedAfterConstraint), &cols);
    let fk = find(&out, ObjectKind::ForeignKey, "fk_ab");
    assert!(!fk.obj.flags().contains(ObjFlags::BROKEN));
    let p = fk.fk().unwrap();
    assert_eq!(p.columns, vec![n("a"), n("b")]);
    assert_eq!(p.target_columns, vec![n("x"), n("y")]);
    assert_eq!(p.target.path.to_vec(), path(&["shop", "p"]));
    assert_eq!(
        (p.on_update, p.on_delete),
        (FkAction::Restrict, FkAction::SetNull)
    );
    assert_eq!(
        fk.fk().unwrap().fk_ref_index().cloned(),
        Some(Ref {
            kind: ObjectKind::Index,
            path: path(&["shop", "p", "PRIMARY"]).into_iter().collect(),
            server_id: None,
        })
    );
    let idx = find(&out, ObjectKind::Index, "fk_ab");
    assert!(idx.obj.flags().contains(ObjFlags::IMPLICIT));
    assert_eq!(
        idx.key().unwrap().implicit_for.as_ref().map(|r| r.kind),
        Some(ObjectKind::ForeignKey)
    );

    let out = mysql::assemble_keys(rows(mysql::FkIndexRule::None), &cols);
    assert!(
        !find(&out, ObjectKind::Index, "fk_ab")
            .obj
            .flags()
            .contains(ObjFlags::IMPLICIT)
    );
    // ## m4-t17b-mysql-facets (named exception): an index named like the FK with more parts
    // than the FK's columns was the user's (InnoDB created none): never implicit.
    let mut longer = rows(mysql::FkIndexRule::NamedAfterConstraint);
    longer.indexes.push(stat("t", "fk_ab", 3, "id"));
    let out = mysql::assemble_keys(longer, &cols);
    assert!(
        !find(&out, ObjectKind::Index, "fk_ab")
            .obj
            .flags()
            .contains(ObjFlags::IMPLICIT)
    );
}

/// SQLite: an expression part's text and a partial index's predicate come from the index's SQL
/// (quotes, brackets and comments do not confuse the scan); a `to` column spelled in another
/// ASCII case resolves; an FK naming no referenced column references the parent's key.
#[test]
fn sqlite_expression_text_predicate_and_fk_targets() {
    let cols = lite_cols(&[("t", &["a", "b", "c"]), ("p", &["id"])]);
    let mut ix = lite_index("t", "ix(1)", "c", false);
    ix.partial = true;
    ix.sql = Some(n(
        "CREATE INDEX \"ix(1)\" ON [t] /* (no) */ (lower(a) COLLATE nocase DESC, 'x,)' || b, b) WHERE c > 0 AND a <> ')'",
    ));
    let mut desc = lite_part("ix(1)", 0, -2, None, true);
    desc.desc = true;
    let rows = sqlite::KeyRows {
        pk_columns: Vec::new(),
        indexes: vec![ix],
        index_parts: vec![
            desc,
            lite_part("ix(1)", 1, -2, None, true),
            lite_part("ix(1)", 2, 1, Some("b"), true),
            lite_part("ix(1)", 3, -1, None, false),
        ],
        foreign_keys: vec![
            lite_fk("t", 0, 0, "p", "c", Some("ID")),
            lite_fk("t", 1, 0, "p", "a", None),
        ],
    };
    let out = sqlite::assemble_keys(rows, &cols);
    let idx = find(&out, ObjectKind::Index, "ix(1)");
    assert!(!idx.obj.flags().contains(ObjFlags::BROKEN), "{idx:#?}");
    let k = idx.key().unwrap();
    assert_eq!(k.columns, vec![expr_slot(), expr_slot(), kc(2, "b")]);
    assert_eq!(k.expressions, vec![n("lower(a)"), n("'x,)' || b")]);
    assert_eq!(k.desc, vec![true, false, false]);
    assert_eq!(k.predicate.as_deref(), Some("c > 0 AND a <> ')'"));

    let by_id = find(&out, ObjectKind::ForeignKey, "t_c_p_fkey");
    assert_eq!(by_id.fk().unwrap().target_columns, vec![n("id")]);
    assert_eq!(by_id.fk().unwrap().on_delete, FkAction::Cascade);
    let to_pk = find(&out, ObjectKind::ForeignKey, "t_a_p_fkey");
    assert!(to_pk.fk().unwrap().target_columns.is_empty());
    assert!(!to_pk.obj.flags().contains(ObjFlags::BROKEN));

    // A partial index whose SQL has no predicate is kept, broken.
    let mut bad = lite_index("t", "bad", "c", false);
    bad.partial = true;
    bad.sql = Some(n("CREATE INDEX bad ON t(a)"));
    let out = sqlite::assemble_keys(
        sqlite::KeyRows {
            pk_columns: Vec::new(),
            indexes: vec![bad],
            index_parts: vec![lite_part("bad", 0, 0, Some("a"), true)],
            foreign_keys: Vec::new(),
        },
        &cols,
    );
    assert!(out[0].obj.flags().contains(ObjFlags::BROKEN));
}

/// Column references resolve by ordinal first, then by name; an ordinal whose column has
/// another name than the row says is not resolved (an inconsistent read), and is broken.
#[test]
fn column_refs_resolve_by_ordinal_then_name() {
    let cols = pg_cols();
    let t = t_pg();
    assert_eq!(cols.resolve(&t, &ColRef::at(2, "b")), Some(kc(2, "b")));
    assert_eq!(cols.resolve(&t, &ColRef::named("b")), Some(kc(2, "b")));
    assert_eq!(cols.resolve(&t, &ColRef::at(2, "a")), None);
    assert_eq!(cols.resolve(&t, &ColRef::at(99, "zz")), None);
    // PostgreSQL names are exact; SQLite and MySQL columns fold ASCII case only.
    assert_eq!(cols.resolve(&t, &ColRef::named("B")), None);
    let lite = lite_cols(&[("t", &["Name"])]);
    assert_eq!(
        lite.resolve(&TableId::named("t"), &ColRef::named("NAME")),
        Some(kc(1, "Name"))
    );
    let my = my_cols(&[("t", &["Äb"])]);
    assert_eq!(
        my.resolve(&TableId::named("t"), &ColRef::named("äb")),
        None,
        "a non-ASCII fold is not pinned (R-M4-41)"
    );
    assert_eq!(
        my.resolve(&TableId::named("t"), &ColRef::named("Äb")),
        Some(kc(1, "Äb"))
    );
    assert_eq!(
        cols.resolve(&TableId::with_oid(555, "x"), &ColRef::at(1, "a")),
        None
    );
}

/// The assembled objects fit the kind schema and keep distinct identities: built into a
/// namespace through `merge::base` and a full pass, every key survives (no sibling collapses
/// into another; edge 12 would keep only the last).
#[test]
fn assembled_keys_merge_into_a_namespace() {
    let (mut rows, _) = lite_unique_and_pk();
    rows.foreign_keys = vec![
        lite_fk("t", 0, 0, "p", "b", Some("id")),
        lite_fk("t", 1, 0, "p", "a", Some("id")),
        lite_fk("t", 2, 0, "q", "a", None),
    ];
    let cols = lite_cols(&[("t", &["a", "b"]), ("p", &["id"]), ("q", &["k"])]);
    let out = sqlite::assemble_keys(rows, &cols);
    let expected = out.len();
    let table_subtree = |keys: &[KeyObj]| {
        let mut t = Subtree::new(
            Obj::new(ObjectKind::Table, "t")
                .with_props(Props::Table(Arc::new(TableProps::default()))),
        );
        for (i, c) in ["a", "b"].iter().enumerate() {
            t.add(
                t.root(),
                Obj::new(ObjectKind::Column, *c).with_props(Props::Column(Arc::new(ColumnProps {
                    ordinal: i as u32 + 1,
                    raw_type: n(""),
                    nullable: true,
                    ..Default::default()
                }))),
            );
        }
        for o in keys {
            assert_eq!(&*o.table.name, "t");
            t.add(t.root(), o.obj.clone());
        }
        t
    };
    let result = |ns: &hacivat_meta::NamespaceSnapshot, subtree: Subtree| NamespaceResult {
        existent: Default::default(),
        changed: vec![ChangedObject {
            parent: ns.object_key(0).expect("root"),
            subtree,
        }],
        categories_to_reload: Default::default(),
        new_watermark: Watermark::None,
        full: true,
        level: Level::Structure,
        read_at_ms: 1_700_000_000_000,
    };
    let keys_of = |ns: &hacivat_meta::NamespaceSnapshot| {
        let table = ns
            .children(0)
            .find(|(_, o)| &**o.name() == "t")
            .map(|(i, _)| i)
            .expect("table");
        let all = ns
            .children(table)
            .filter(|(_, o)| o.kind() != ObjectKind::Column)
            .count();
        let fks = ns
            .children(table)
            .filter(|(_, o)| o.kind() == ObjectKind::ForeignKey)
            .count();
        (all, fks)
    };

    // A first load goes through `merge::base`.
    let empty = base(
        NsKey::schema(["main"]),
        DbmsKind::Sqlite,
        FoldRule::AsciiCaseless,
    )
    .expect("base");
    let first = merge(
        Some(&empty),
        result(&empty, table_subtree(&out)),
        &MergePolicy::default(),
    )
    .expect("first load");
    assert_eq!(first.report().duplicates, 0);
    let cached = first.next().clone();
    assert_eq!(keys_of(&cached), (expected, 3));

    // A re-read whose keys arrive in another order is matched sibling by sibling: identities
    // that collided would collapse into one (edge 12, the last wins).
    let mut reordered = out.clone();
    reordered.reverse();
    let again = merge(
        Some(&cached),
        result(&cached, table_subtree(&reordered)),
        &MergePolicy::default(),
    )
    .expect("re-read");
    assert_eq!(again.report().duplicates, 0);
    assert_eq!(keys_of(again.next()), (expected, 3));
}

// ---- fix round 1 (R-W2-8, I2, I3, m1, m2, m6) ------------------------------------------------

/// I2: a CHECK stores the bare `pg_get_expr` text (DDL writes `CHECK (expr)`); an EXCLUDE stores
/// the text after `EXCLUDE` without the deferral words, which go into the rare part with its
/// operators. `NO INHERIT` and a non-local constraint are carried.
#[test]
fn pg_check_and_exclusion_follow_the_ddl_contract() {
    let mut ck = pg_con(400, "t_a_check", b'c', vec![ColRef::at(1, "a")]);
    ck.definition = n("CHECK ((a > 0)) NO INHERIT NOT VALID");
    ck.check_expr = Some(n("a > 0"));
    ck.no_inherit = true;
    ck.validated = false;
    let mut inherited = pg_con(401, "t_b_check", b'c', vec![ColRef::at(2, "b")]);
    inherited.check_expr = Some(n("b > 0"));
    inherited.local = false;
    let missing = pg_con(402, "t_x_check", b'c', Vec::new());
    let mut x = pg_con(403, "t_excl", b'x', vec![ColRef::at(6, "room")]);
    x.definition = n("EXCLUDE USING gist (room WITH =) DEFERRABLE INITIALLY DEFERRED");
    x.operators = vec![n("=")];
    x.deferrable = true;
    x.deferred = true;
    let out = pg::assemble_keys(
        pg::KeyRows {
            indexes: Vec::new(),
            constraints: vec![ck, inherited, missing, x],
        },
        &pg_cols(),
    );
    let ck = find(&out, ObjectKind::Check, "t_a_check");
    let p = ck.check().unwrap();
    assert_eq!(&*p.expr, "a > 0");
    assert!(p.no_inherit() && p.not_valid);
    assert!(ck.obj.flags().contains(ObjFlags::INVALID));
    assert!(!ck.is_broken());
    assert!(
        find(&out, ObjectKind::Check, "t_b_check")
            .obj
            .flags()
            .contains(ObjFlags::INHERITED)
    );
    let missing = find(&out, ObjectKind::Check, "t_x_check");
    assert!(missing.is_broken());
    assert_eq!(&*missing.check().unwrap().expr, "");
    let x = find(&out, ObjectKind::Exclusion, "t_excl")
        .check()
        .unwrap()
        .clone();
    assert_eq!(&*x.expr, "USING gist (room WITH =)");
    let r = x.rare.as_deref().expect("exclusion rare part");
    assert!(r.excl_deferrable && r.excl_initially_deferred && !r.no_inherit);
    assert_eq!(r.excl_ops, vec![n("=")]);
}

/// R-W2-8: `NULLS FIRST`, a non-default operator class and a collation other than the column's
/// ride in the key's rare part, one entry per part (empty vectors when no part has the option),
/// and a key takes them from its backing index.
#[test]
fn pg_part_options_ride_in_the_rare_part() {
    let mut name = pg_part(3, "name");
    name.opclass = Some(n("public.gin_trgm_ops"));
    let mut b = pg_part(2, "b");
    b.desc = true;
    b.nulls_first = true;
    b.collation = Some(n("C"));
    let mut i = pg_index(260, "t_opts_idx", vec![name, b]);
    i.method = Some(n("gin"));
    let mut uk = pg_con(
        360,
        "t_ab_key",
        b'u',
        vec![ColRef::at(1, "a"), ColRef::at(2, "b")],
    );
    uk.index = Some(261);
    let mut ub = pg_part(2, "b");
    ub.collation = Some(n("C"));
    let mut ui = pg_index(261, "t_ab_key", vec![pg_part(1, "a"), ub]);
    ui.unique = true;
    let plain = pg_index(262, "t_plain_idx", vec![pg_part(1, "a")]);
    let out = pg::assemble_keys(
        pg::KeyRows {
            indexes: vec![i, ui, plain],
            constraints: vec![uk],
        },
        &pg_cols(),
    );
    let k = find(&out, ObjectKind::Index, "t_opts_idx")
        .key()
        .unwrap()
        .clone();
    assert_eq!(k.desc, vec![false, true]);
    assert_eq!(k.nulls_first(), &[false, true][..]);
    assert_eq!(k.opclass(), &[Some(n("public.gin_trgm_ops")), None][..]);
    assert_eq!(k.collation(), &[None, Some(n("C"))][..]);
    let key = find(&out, ObjectKind::UniqueKey, "t_ab_key")
        .key()
        .unwrap()
        .clone();
    assert_eq!(key.collation(), &[None, Some(n("C"))][..]);
    assert!(key.nulls_first().is_empty() && key.opclass().is_empty());
    assert_eq!(
        find(&out, ObjectKind::Index, "t_plain_idx")
            .key()
            .unwrap()
            .rare,
        None
    );
}

/// R-W2-8 on SQLite: a part's collation (`index_xinfo.coll`, as Task 18 passes it) rides in the
/// rare part (a `UNIQUE (email COLLATE NOCASE)` is another constraint than a plain one); an FK
/// carries its deferral.
#[test]
fn sqlite_collation_and_fk_deferral_are_carried() {
    let cols = lite_cols(&[("t", &["email", "p_id"]), ("p", &["id"])]);
    let mut part = lite_part("sqlite_autoindex_t_1", 0, 0, Some("email"), true);
    part.coll = Some(n("NOCASE"));
    let mut fk = lite_fk("t", 0, 0, "p", "p_id", Some("id"));
    fk.deferrable = true;
    fk.initially_deferred = true;
    let out = sqlite::assemble_keys(
        sqlite::KeyRows {
            pk_columns: Vec::new(),
            indexes: vec![lite_index("t", "sqlite_autoindex_t_1", "u", true)],
            index_parts: vec![part],
            foreign_keys: vec![fk],
        },
        &cols,
    );
    for kind in [ObjectKind::UniqueKey, ObjectKind::Index] {
        let k = find(&out, kind, "sqlite_autoindex_t_1")
            .key()
            .unwrap()
            .clone();
        assert_eq!(k.collation(), &[Some(n("NOCASE"))][..], "{}", kind.id());
    }
    let f = find(&out, ObjectKind::ForeignKey, "t_p_id_p_fkey")
        .fk()
        .unwrap()
        .clone();
    assert!(f.deferrable && f.initially_deferred);
}

/// I1: FK display names come from content. Two FKs on the same columns to the same parent take
/// their referenced columns too; exact duplicates are numbered in declaration order; a very
/// long name is hashed, the same way every time.
#[test]
fn sqlite_fk_names_are_content_derived() {
    let cols = lite_cols(&[("t", &["a", "b"]), ("p", &["id", "k"])]);
    let rows = |fks: Vec<sqlite::FkRow>| sqlite::KeyRows {
        foreign_keys: fks,
        ..Default::default()
    };
    // id 3 is declared first.
    let out = sqlite::assemble_keys(
        rows(vec![
            lite_fk("t", 3, 0, "p", "a", Some("id")),
            lite_fk("t", 2, 0, "p", "a", Some("k")),
            lite_fk("t", 1, 0, "p", "b", None),
            lite_fk("t", 0, 0, "p", "b", None),
        ]),
        &cols,
    );
    let mut names: Vec<String> = out.iter().map(|o| o.obj.name().to_string()).collect();
    names.sort();
    assert_eq!(
        names,
        [
            "t_a_p_id_fkey",
            "t_a_p_k_fkey",
            "t_b_p_pk_fkey",
            "t_b_p_pk_fkey_2"
        ]
    );
    let long = "c".repeat(150);
    let wide = lite_cols(&[("t", &[long.as_str()]), ("p", &["id"])]);
    let a = sqlite::assemble_keys(
        rows(vec![lite_fk("t", 0, 0, "p", &long, Some("id"))]),
        &wide,
    );
    let b = sqlite::assemble_keys(
        rows(vec![lite_fk("t", 4, 0, "p", &long, Some("id"))]),
        &wide,
    );
    let name = a[0].obj.name().to_string();
    assert!(
        name.starts_with("t_fk_") && name.len() == "t_fk_".len() + 16,
        "{name}"
    );
    assert_eq!(a[0].obj.name(), b[0].obj.name());
}

/// I3: an FK target that resolves is named by the stored table (MySQL with
/// `lower_case_table_names = 1`: `REFERENCES P` names table `p`).
#[test]
fn mysql_fk_target_uses_the_stored_table_name() {
    let mut cols = ColumnIndex::new(
        NsKey::schema(["shop"]),
        DialectId::MySql,
        IdentCtx {
            lower_case_table_names: 1,
            ..IdentCtx::new(DialectId::MySql)
        },
    );
    cols.add_table(TableId::named("t"), [(1, n("a"))]);
    cols.add_table(TableId::named("p"), [(1, n("id"))]);
    let mut r = mysql::KeyRows::new(mysql::Flavor::MySql);
    r.foreign_keys = vec![mysql::FkRow {
        table: n("t"),
        name: n("fk"),
        referenced_schema: n("shop"),
        referenced_table: n("P"),
        unique_constraint_name: Some(n("PRIMARY")),
        update_rule: n("NO ACTION"),
        delete_rule: n("NO ACTION"),
        match_option: n("NONE"),
    }];
    r.fk_columns = vec![mysql::FkColumnRow {
        table: n("t"),
        constraint: n("fk"),
        column: n("a"),
        ordinal_position: 1,
        referenced_column: Some(n("ID")),
    }];
    let out = mysql::assemble_keys(r, &cols);
    let f = out[0].fk().unwrap();
    assert!(!out[0].is_broken());
    assert_eq!(f.target.path.to_vec(), path(&["shop", "p"]));
    assert_eq!(f.target_columns, vec![n("id")]);
    assert_eq!(
        f.fk_ref_index().map(|r| r.path.to_vec()),
        Some(path(&["shop", "p", "PRIMARY"]))
    );
}

/// m6: in a complete index (a full pass) an FK whose target lies in this namespace but is not
/// there dangles and is broken; in a fragment the target is only unknown. A target in another
/// namespace is never broken for that.
#[test]
fn dangling_fk_in_a_complete_namespace_is_broken() {
    let fk = || sqlite::KeyRows {
        foreign_keys: vec![lite_fk("t", 0, 0, "gone", "a", Some("id"))],
        ..Default::default()
    };
    let mut cols = lite_cols(&[("t", &["a"])]);
    assert!(!sqlite::assemble_keys(fk(), &cols)[0].is_broken());
    cols.set_complete(true);
    assert!(sqlite::assemble_keys(fk(), &cols)[0].is_broken());

    let mut pg_complete = pg_cols();
    pg_complete.set_complete(true);
    let rows = |schema: &str| pg::KeyRows {
        indexes: Vec::new(),
        constraints: vec![pg_fk(
            500,
            "t_a_fkey",
            vec![ColRef::at(1, "a")],
            TableId::with_oid(990, "gone"),
            schema,
            vec![ColRef::at(1, "id")],
        )],
    };
    assert!(pg::assemble_keys(rows("s"), &pg_complete)[0].is_broken());
    assert!(!pg::assemble_keys(rows("other"), &pg_complete)[0].is_broken());
}

/// m2: the scanned SQL is used only when it has as many parts as the index; otherwise an
/// expression slot gets no text from another part and is broken.
#[test]
fn sqlite_scan_with_another_part_count_is_not_used() {
    let cols = lite_cols(&[("t", &["a", "b"])]);
    let mut ix = lite_index("t", "ix", "c", false);
    ix.sql = Some(n("CREATE INDEX ix ON t(lower(a))"));
    let out = sqlite::assemble_keys(
        sqlite::KeyRows {
            pk_columns: Vec::new(),
            indexes: vec![ix],
            index_parts: vec![
                lite_part("ix", 0, 1, Some("b"), true),
                lite_part("ix", 1, -2, None, true),
            ],
            foreign_keys: Vec::new(),
        },
        &cols,
    );
    assert!(out[0].is_broken());
    assert_eq!(out[0].key().unwrap().expressions, vec![n("")]);
}
