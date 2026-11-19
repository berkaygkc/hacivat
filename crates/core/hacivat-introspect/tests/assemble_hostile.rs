//! M4 Task 13 opus review: hostile probes the implementer did not write
//! (`task-m4-13-review-hostile.rs`), committed with fix round 1. Each asserts the behaviour the
//! review asks for: h1–h4 were findings (I1, I2, I3, m1) and pass with the fix; h5 and h6 are
//! sanity probes. Row constructors gained the fix round's new row fields.

use std::sync::Arc;

use hacivat_introspect::assemble::{ColRef, ColumnIndex, KeyObj, TableId, mysql, pg, sqlite};
use hacivat_meta::merge::{
    ChangedObject, ExistSet, Identity, MergePolicy, NamespaceResult, base, merge,
};
use hacivat_meta::name::{DialectId, FoldRule, IdentCtx, Name};
use hacivat_meta::namespace::{Level, Obj, Subtree, Watermark};
use hacivat_meta::props::{ColumnProps, Props, TableProps};
use hacivat_meta::{NsKey, ObjectKind};
use hacivat_proto::DbmsKind;

fn n(s: &str) -> Name {
    Arc::from(s)
}

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

fn lite_fk(table: &str, id: u32, ref_table: &str, from: &str, to: &str) -> sqlite::FkRow {
    sqlite::FkRow {
        table: n(table),
        id,
        seq: 0,
        ref_table: n(ref_table),
        from: n(from),
        to: Some(n(to)),
        on_update: n("NO ACTION"),
        on_delete: n("NO ACTION"),
        match_kind: n("NONE"),
        deferrable: false,
        initially_deferred: false,
    }
}

fn fk_named_for<'a>(objs: &'a [KeyObj], from: &str) -> &'a KeyObj {
    objs.iter()
        .find(|o| {
            o.fk()
                .is_some_and(|f| f.columns.iter().any(|c| &**c == from))
        })
        .unwrap_or_else(|| panic!("no fk on {from}: {objs:#?}"))
}

/// H1. `CREATE TABLE t(a REFERENCES p(id), b REFERENCES q(id))`, then the usual SQLite
/// "drop a constraint" (rebuild `t` without the FK on `a`). The FK on `b` is the same FK before
/// and after; its identity (the display name, which is the sibling identity) must not change,
/// and the name it had must not be handed to another FK.
#[test]
fn h1_sqlite_fk_identity_survives_dropping_an_earlier_fk() {
    let cols = lite_cols(&[("t", &["a", "b"]), ("p", &["id"]), ("q", &["id"])]);
    // SQLite numbers `id` from the last declared FK.
    let before = sqlite::assemble_keys(
        sqlite::KeyRows {
            foreign_keys: vec![
                lite_fk("t", 0, "q", "b", "id"),
                lite_fk("t", 1, "p", "a", "id"),
            ],
            ..Default::default()
        },
        &cols,
    );
    let after = sqlite::assemble_keys(
        sqlite::KeyRows {
            foreign_keys: vec![lite_fk("t", 0, "q", "b", "id")],
            ..Default::default()
        },
        &cols,
    );
    let b_before = fk_named_for(&before, "b").obj.name().clone();
    let a_before = fk_named_for(&before, "a").obj.name().clone();
    let b_after = fk_named_for(&after, "b").obj.name().clone();
    eprintln!("before: a={a_before} b={b_before}; after: b={b_after}");
    assert_eq!(
        b_after, b_before,
        "the FK on b changed identity after an unrelated FK was dropped"
    );
    assert_ne!(
        b_after, a_before,
        "the dropped FK's identity was handed to the FK on b"
    );
}

/// H1b. The same drop through `merge`: the delta must say "the FK on a was dropped", not "t_fk1
/// was altered and t_fk2 dropped" (which moves any user data keyed on t_fk1 to another FK).
///
/// Fix round 1: the pass reports the FK existence set scoped to the re-read table (the T15–T18
/// contract, R-W2-5: no set, no deletion), and only FK entries are checked in `altered` (the
/// table itself is altered with `children` when one of its FKs is dropped).
#[test]
fn h1b_sqlite_fk_drop_through_merge_drops_the_right_key() {
    let cols = lite_cols(&[("t", &["a", "b"]), ("p", &["id"]), ("q", &["id"])]);
    let before = sqlite::assemble_keys(
        sqlite::KeyRows {
            foreign_keys: vec![
                lite_fk("t", 0, "q", "b", "id"),
                lite_fk("t", 1, "p", "a", "id"),
            ],
            ..Default::default()
        },
        &cols,
    );
    let after = sqlite::assemble_keys(
        sqlite::KeyRows {
            foreign_keys: vec![lite_fk("t", 0, "q", "b", "id")],
            ..Default::default()
        },
        &cols,
    );
    let subtree = |keys: &[KeyObj]| {
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
            t.add(t.root(), o.obj.clone());
        }
        t
    };
    let fk_set = |keys: &[KeyObj]| ExistSet::Scoped {
        scope: vec![vec![n("t")]],
        ids: keys
            .iter()
            .filter(|o| o.obj.kind() == ObjectKind::ForeignKey)
            .map(|o| Identity::of(vec![n("t"), o.obj.name().clone()], None))
            .collect(),
    };
    let result = |ns: &hacivat_meta::NamespaceSnapshot, keys: &[KeyObj]| NamespaceResult {
        existent: [(ObjectKind::ForeignKey, fk_set(keys))]
            .into_iter()
            .collect(),
        changed: vec![ChangedObject {
            parent: ns.object_key(0).expect("root"),
            subtree: subtree(keys),
        }],
        categories_to_reload: Default::default(),
        new_watermark: Watermark::None,
        full: true,
        level: Level::Structure,
        read_at_ms: 1_700_000_000_000,
    };
    let empty = base(
        NsKey::schema(["main"]),
        DbmsKind::Sqlite,
        FoldRule::AsciiCaseless,
    )
    .expect("base");
    let first = merge(
        Some(&empty),
        result(&empty, &before),
        &MergePolicy::default(),
    )
    .expect("first");
    let cached = first.next().clone();
    let second = merge(
        Some(&cached),
        result(&cached, &after),
        &MergePolicy::default(),
    )
    .expect("second");
    let d = second.delta();
    let dropped: Vec<String> = d.dropped.iter().map(|k| format!("{:?}", k.path)).collect();
    let altered: Vec<String> = d
        .altered
        .iter()
        .filter(|(k, _)| k.kind == ObjectKind::ForeignKey)
        .map(|(k, _)| format!("{:?}", k.path))
        .collect();
    eprintln!("dropped {dropped:?}; altered {altered:?}");
    let a_name = fk_named_for(&before, "a").obj.name().clone();
    assert!(
        d.dropped.iter().any(|k| k.path.last() == Some(&a_name)),
        "the delta does not drop the FK on a ({a_name}); dropped {dropped:?}, altered {altered:?}"
    );
    assert!(
        altered.is_empty(),
        "an untouched FK is reported altered: {altered:?}"
    );
}

/// H2. SQLite reports `foreign_key_list.table` as the REFERENCES clause spelled it. A parent
/// spelled `P` for the stored table `p` resolves (SQLite names fold ASCII), so the target `Ref`
/// must name the stored table: `locate_ref` matches names exactly, and a `[main, P]` target
/// reads as a dangling FK and drops out of `fk_referencing(p)`.
#[test]
fn h2_sqlite_fk_target_uses_the_stored_parent_name() {
    let cols = lite_cols(&[("t", &["a"]), ("p", &["id"])]);
    let out = sqlite::assemble_keys(
        sqlite::KeyRows {
            foreign_keys: vec![lite_fk("t", 0, "P", "a", "ID")],
            ..Default::default()
        },
        &cols,
    );
    let fk = fk_named_for(&out, "a").fk().expect("fk");
    eprintln!(
        "target {:?} target_columns {:?}",
        fk.target.path, fk.target_columns
    );
    assert_eq!(
        &*fk.target_columns[0], "id",
        "target column is the stored name"
    );
    assert_eq!(
        fk.target.path.last().map(|s| &**s),
        Some("p"),
        "target table is the REFERENCES clause's spelling, not the stored name"
    );
}

/// H3. The DDL contract (DDL spike §"Contracts", `hacivat-ddl` renders `CHECK ({expr})` and
/// `EXCLUDE {raw}`): `CheckProps.expr` is the bare expression, and an exclusion's is the text
/// after `EXCLUDE`. T13 stores `pg_get_constraintdef` whole.
#[test]
fn h3_pg_check_and_exclusion_expr_follow_the_ddl_contract() {
    let mut c = ColumnIndex::new(
        NsKey::schema(["db", "s"]),
        DialectId::Postgres,
        IdentCtx::new(DialectId::Postgres),
    );
    c.add_table(TableId::with_oid(100, "t"), [(1, n("a")), (2, n("r"))]);
    let con = |oid: u32, name: &str, contype: u8, def: &str| pg::ConstraintRow {
        table: TableId::with_oid(100, "t"),
        oid,
        name: n(name),
        contype,
        columns: vec![ColRef::at(1, "a")],
        definition: n(def),
        check_expr: (contype == b'c').then(|| n("a > 0")),
        deferrable: false,
        deferred: false,
        validated: false,
        enforced: true,
        no_inherit: false,
        local: true,
        parent: None,
        index: None,
        foreign: None,
        operators: vec![n("=")],
    };
    let out = pg::assemble_keys(
        pg::KeyRows {
            indexes: vec![],
            constraints: vec![
                con(1, "ck", b'c', "CHECK ((a > 0)) NOT VALID"),
                con(2, "ex", b'x', "EXCLUDE USING gist (a WITH =)"),
            ],
        },
        &c,
    );
    for o in &out {
        let e = &o.check().expect("check").expr;
        eprintln!("{} expr = {e:?}", o.obj.name());
        assert!(
            !e.starts_with("CHECK") && !e.starts_with("EXCLUDE") && !e.contains("NOT VALID"),
            "{} keeps the constraintdef wrapper: {e:?} (DDL would print CHECK (CHECK ...))",
            o.obj.name()
        );
    }
}

/// H4. A PK whose `conindid` index row did not come with the pass (a fragment, a name filter
/// on indexes): its INCLUDE list and NULLS NOT DISTINCT are unknown, so the key must not read
/// as complete (no backing index, no include, not broken).
#[test]
fn h4_pg_pk_with_a_missing_backing_index_row_is_not_silently_complete() {
    let mut c = ColumnIndex::new(
        NsKey::schema(["db", "s"]),
        DialectId::Postgres,
        IdentCtx::new(DialectId::Postgres),
    );
    c.add_table(TableId::with_oid(100, "t"), [(1, n("a"))]);
    let out = pg::assemble_keys(
        pg::KeyRows {
            indexes: vec![],
            constraints: vec![pg::ConstraintRow {
                table: TableId::with_oid(100, "t"),
                oid: 7,
                name: n("t_pkey"),
                contype: b'p',
                columns: vec![ColRef::at(1, "a")],
                definition: n("PRIMARY KEY (a) INCLUDE (b)"),
                check_expr: None,
                deferrable: false,
                deferred: false,
                validated: true,
                enforced: true,
                no_inherit: false,
                local: true,
                parent: None,
                index: Some(4242),
                foreign: None,
                operators: vec![],
            }],
        },
        &c,
    );
    let pk = &out[0];
    eprintln!("flags {:?} key {:?}", pk.obj.flags(), pk.key());
    assert!(
        pk.is_broken() || pk.key().is_some_and(|k| k.backing_index.is_some()),
        "a PK whose backing index is unknown reads as complete"
    );
}

/// H5 (sanity, expected to pass). MySQL FK to another database: kept, unresolved, not broken,
/// the target path names the other schema, and multi-column parts follow ORDINAL_POSITION.
#[test]
fn h5_mysql_cross_database_fk_in_ordinal_order() {
    let mut c = ColumnIndex::new(
        NsKey::schema(["shop"]),
        DialectId::MySql,
        IdentCtx::new(DialectId::MySql),
    );
    c.add_table(TableId::named("o"), [(1, n("x")), (2, n("y"))]);
    let mut rows = mysql::KeyRows::new(mysql::Flavor::MySql);
    rows.foreign_keys = vec![mysql::FkRow {
        table: n("o"),
        name: n("fk_o"),
        referenced_schema: n("crm"),
        referenced_table: n("c"),
        unique_constraint_name: Some(n("PRIMARY")),
        update_rule: n("NO ACTION"),
        delete_rule: n("CASCADE"),
        match_option: n("NONE"),
    }];
    let part = |col: &str, pos: u32, rc: &str| mysql::FkColumnRow {
        table: n("o"),
        constraint: n("fk_o"),
        column: n(col),
        ordinal_position: pos,
        referenced_column: Some(n(rc)),
    };
    rows.fk_columns = vec![part("x", 2, "k2"), part("y", 1, "k1")];
    let out = mysql::assemble_keys(rows, &c);
    let fk = out[0].fk().expect("fk");
    assert!(!out[0].is_broken());
    assert_eq!(
        fk.target.path.iter().map(|s| &**s).collect::<Vec<_>>(),
        ["crm", "c"]
    );
    assert_eq!(
        fk.columns.iter().map(|s| &**s).collect::<Vec<_>>(),
        ["y", "x"]
    );
    assert_eq!(
        fk.target_columns.iter().map(|s| &**s).collect::<Vec<_>>(),
        ["k1", "k2"]
    );
}

/// H6 (sanity, expected to pass). Objects whose parts are missing are kept, broken: a SQLite
/// index with no `index_xinfo` rows and a MySQL index with a SEQ_IN_INDEX gap.
#[test]
fn h6_partless_objects_are_kept_broken() {
    let cols = lite_cols(&[("t", &["a"])]);
    let out = sqlite::assemble_keys(
        sqlite::KeyRows {
            indexes: vec![sqlite::IndexRow {
                table: n("t"),
                name: n("i"),
                unique: false,
                origin: n("c"),
                partial: false,
                sql: Some(n("CREATE INDEX i ON t(a)")),
            }],
            ..Default::default()
        },
        &cols,
    );
    assert_eq!(out.len(), 1);
    assert!(out[0].is_broken());

    let mut c = ColumnIndex::new(
        NsKey::schema(["shop"]),
        DialectId::MySql,
        IdentCtx::new(DialectId::MySql),
    );
    c.add_table(TableId::named("o"), [(1, n("x")), (2, n("y"))]);
    let stat = |seq: u32, col: &str| mysql::StatRow {
        table: n("o"),
        index: n("ix"),
        seq_in_index: seq,
        column: Some(n(col)),
        expression: None,
        collation: Some(n("A")),
        sub_part: None,
        non_unique: true,
        index_type: Some(n("BTREE")),
        visible: true,
        comment: None,
    };
    let mut rows = mysql::KeyRows::new(mysql::Flavor::MySql);
    rows.indexes = vec![stat(1, "x"), stat(3, "y")];
    let out = mysql::assemble_keys(rows, &c);
    assert_eq!(out.len(), 1);
    assert!(out[0].is_broken());
}
