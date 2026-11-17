//! Generated catalogs for tests and benches (`test-support` only).
//!
//! [`large_pg_catalog`] is the DBeaver #42465 scale the HV-MODEL-03 budget names: 15,000
//! tables and 160,000 columns in one PostgreSQL database, spread over five schemas (one of
//! 10,000 tables, the worst case for a single-namespace reload), with a primary key and an
//! index per table, a foreign key on every third table, and a view and a function every 100
//! and 50 tables. Names, types, defaults and comments repeat the way real schemas do (shared
//! column and type names, per-table defaults), so string interning is measured honestly.

use std::sync::Arc;

use hacivat_proto::DbmsKind;

use crate::ids::{NsKey, Ref, ServerId, SourceRef, StateMark};
use crate::kind::ObjectKind;
use crate::name::{FoldPolicy, Name};
use crate::namespace::{Level, NamespaceSnapshot, NsMeta, Obj, ObjFlags, Subtree, Watermark};
use crate::props::*;

/// The database name of the generated catalogs.
pub const DB: &str = "app";

const COLUMN_NAMES: [&str; 12] = [
    "id",
    "created_at",
    "updated_at",
    "name",
    "status",
    "owner_id",
    "amount",
    "currency",
    "note",
    "external_ref",
    "deleted_at",
    "version",
];

const TYPES: [&str; 10] = [
    "bigint",
    "integer",
    "text",
    "character varying(255)",
    "timestamp with time zone",
    "numeric(12,2)",
    "boolean",
    "uuid",
    "jsonb",
    "date",
];

/// Schemas and table counts of [`large_pg_catalog`].
pub const LARGE_SCHEMAS: [(&str, usize); 5] = [
    ("public", 10_000),
    ("sales", 2_000),
    ("hr", 1_500),
    ("audit", 1_000),
    ("reference", 500),
];

/// 15,000 tables, 160,000 columns (see the module docs), plus the database and server
/// namespaces.
pub fn large_pg_catalog() -> Vec<NamespaceSnapshot> {
    pg_catalog(&LARGE_SCHEMAS, 160_000)
}

/// A PostgreSQL catalog with `schemas` (name, table count) and `total_columns` columns spread
/// as evenly as possible (the first tables get the remainder).
pub fn pg_catalog(schemas: &[(&str, usize)], total_columns: usize) -> Vec<NamespaceSnapshot> {
    let tables: usize = schemas.iter().map(|s| s.1).sum();
    let base = total_columns / tables.max(1);
    let extra = total_columns % tables.max(1);
    let mut out = vec![server_ns(), database_ns()];
    let mut global = 0usize;
    let mut oid = 16_384u32;
    for &(schema, count) in schemas {
        out.push(schema_ns(schema, count, base, extra, &mut global, &mut oid));
    }
    out
}

fn meta() -> NsMeta {
    NsMeta {
        level: Level::Structure,
        watermark: Watermark::PgXid {
            xid: 1_234_567,
            system_id: 7_301_234_567_890_123_456,
        },
        loaded_at_ms: 1_763_337_600_000,
    }
}

fn server_ns() -> NamespaceSnapshot {
    let mut t = Subtree::new(Obj::new(ObjectKind::Root, ""));
    for r in ["postgres", "app_owner", "app_reader", "app_writer"] {
        t.add(t.root(), Obj::new(ObjectKind::Role, r));
    }
    t.add(t.root(), Obj::new(ObjectKind::Tablespace, "pg_default"));
    NamespaceSnapshot::build(
        NsKey::server(),
        DbmsKind::Postgres,
        FoldPolicy::for_dbms(DbmsKind::Postgres),
        meta(),
        t,
    )
    .expect("server namespace")
}

fn database_ns() -> NamespaceSnapshot {
    let mut t = Subtree::new(
        Obj::new(ObjectKind::Database, DB).with_props(Props::Database(Arc::new(DatabaseProps {
            owner: Some(Arc::from("app_owner")),
            encoding: Some(Arc::from("UTF8")),
            collation: Some(Arc::from("en_US.UTF-8")),
            ctype: Some(Arc::from("en_US.UTF-8")),
            comment: None,
        }))),
    );
    for (e, v) in [("plpgsql", "1.0"), ("pgcrypto", "1.3"), ("pg_trgm", "1.6")] {
        t.add(
            t.root(),
            Obj::new(ObjectKind::Extension, e).with_props(Props::Generic(Arc::new(GenericProps {
                owner: None,
                version: Some(Arc::from(v)),
                comment: None,
            }))),
        );
    }
    NamespaceSnapshot::build(
        NsKey::database(DB),
        DbmsKind::Postgres,
        FoldPolicy::for_dbms(DbmsKind::Postgres),
        meta(),
        t,
    )
    .expect("database namespace")
}

/// One generated table subtree: `name` with `columns` columns, a primary key, an index and
/// (when `fk_to` is set) a foreign key. Exposed for the incremental-update tests.
pub fn table_subtree(
    schema: &str,
    name: &str,
    columns: usize,
    oid: u32,
    fk_to: Option<&str>,
) -> Subtree {
    let interned = |s: &str| -> Name { Arc::from(s) };
    let mut t = Subtree::with_capacity(
        Obj::new(ObjectKind::Table, name)
            .with_server_id(ServerId::Oid(oid))
            .with_state(StateMark::Xmin(oid.wrapping_mul(7)))
            .with_props(Props::Table(Arc::new(TableProps {
                owner: Some(interned("app_owner")),
                comment: oid.is_multiple_of(10).then(|| interned("Business records")),
                row_estimate: Some(i64::from(oid % 100_000)),
                ext: TableExt::Pg(PgTableExt {
                    relkind: b'r',
                    persistence: b'p',
                    ..Default::default()
                }),
                ..Default::default()
            }))),
        columns + 4,
    );
    let root = t.root();
    for k in 0..columns {
        let col_name: Name = if k < 6 {
            Arc::from(COLUMN_NAMES[k % COLUMN_NAMES.len()])
        } else {
            Arc::from(
                format!(
                    "{}_{}",
                    COLUMN_NAMES[k % COLUMN_NAMES.len()],
                    k / 12 + oid as usize % 50
                )
                .as_str(),
            )
        };
        let default_expr = match k {
            0 => Some(Arc::from(
                format!("nextval('{name}_id_seq'::regclass)").as_str(),
            )),
            1 | 2 => Some(Arc::from("now()")),
            _ => None,
        };
        t.add(
            root,
            Obj::new(ObjectKind::Column, col_name).with_props(Props::Column(Arc::new(
                ColumnProps {
                    ordinal: k as u32 + 1,
                    type_text: Arc::from(TYPES[(k * 7 + oid as usize) % TYPES.len()]),
                    nullable: k != 0,
                    default_expr,
                    comment: (k % 10 == 3).then(|| Arc::from("Free-form note")),
                    ext: ColumnExt::Pg(PgColumnExt {
                        attnum: k as i16 + 1,
                        ..Default::default()
                    }),
                    ..Default::default()
                },
            ))),
        );
    }
    t.add(
        root,
        Obj::new(ObjectKind::PrimaryKey, format!("{name}_pkey").as_str())
            .with_server_id(ServerId::Oid(oid + 1))
            .with_flags(ObjFlags::SERVER_GENERATED_NAME)
            .with_props(Props::Key(Arc::new(KeyProps {
                columns: vec![Arc::from("id")],
                index_name: Some(Arc::from(format!("{name}_pkey").as_str())),
                deferrable: Deferrable::No,
            }))),
    );
    t.add(
        root,
        Obj::new(ObjectKind::Index, format!("{name}_created_at_idx").as_str())
            .with_server_id(ServerId::Oid(oid + 2))
            .with_props(Props::Index(Arc::new(IndexProps {
                method: Some(Arc::from("btree")),
                parts: vec![IndexPart {
                    column: Some(Arc::from("created_at")),
                    ..Default::default()
                }],
                ..Default::default()
            }))),
    );
    if let Some(target) = fk_to {
        t.add(
            root,
            Obj::new(
                ObjectKind::ForeignKey,
                format!("{name}_owner_id_fkey").as_str(),
            )
            .with_server_id(ServerId::Oid(oid + 3))
            .with_props(Props::ForeignKey(Arc::new(ForeignKeyProps {
                columns: vec![Arc::from("owner_id")],
                target: Ref {
                    kind: ObjectKind::Table,
                    path: [DB, schema, target].iter().map(|s| Arc::from(*s)).collect(),
                    server_id: None,
                },
                target_columns: vec![Arc::from("id")],
                on_update: FkAction::NoAction,
                on_delete: FkAction::Cascade,
                match_kind: FkMatch::Simple,
                deferrable: Deferrable::No,
            }))),
        );
    }
    t
}

fn schema_ns(
    schema: &str,
    count: usize,
    base: usize,
    extra: usize,
    global: &mut usize,
    oid: &mut u32,
) -> NamespaceSnapshot {
    let mut t = Subtree::new(
        Obj::new(ObjectKind::Schema, schema).with_props(Props::Schema(Arc::new(SchemaProps {
            owner: Some(Arc::from("app_owner")),
            comment: None,
        }))),
    );
    let root = t.root();
    let mut prev: Option<String> = None;
    for i in 0..count {
        let name = format!("tbl_{i:05}");
        let columns = base + usize::from(*global < extra);
        let fk = (i % 3 == 2).then_some(()).and(prev.as_deref());
        let sub = table_subtree(schema, &name, columns, *oid, fk);
        t.graft_under(root, sub);
        *oid += 4;
        *global += 1;
        if i % 100 == 99 {
            t.add(
                root,
                Obj::new(ObjectKind::View, format!("v_{i:05}").as_str())
                    .with_server_id(ServerId::Oid(*oid))
                    .with_props(Props::View(Arc::new(ViewProps {
                        source: Some(SourceRef::of(&format!("SELECT * FROM {name}"))),
                        ..Default::default()
                    }))),
            );
            *oid += 1;
        }
        if i % 50 == 49 {
            t.add(
                root,
                Obj::new(ObjectKind::Routine, format!("fn_{i:05}").as_str())
                    .with_server_id(ServerId::Oid(*oid))
                    .with_props(Props::Routine(Arc::new(RoutineProps {
                        args: vec![Arg {
                            name: Some(Arc::from("p_id")),
                            type_text: Arc::from("bigint"),
                            ..Default::default()
                        }],
                        returns: Some(Arc::from("integer")),
                        language: Some(Arc::from("plpgsql")),
                        volatility: Volatility::Stable,
                        overload_sig: Some(Arc::from("bigint")),
                        ..Default::default()
                    }))),
            );
            *oid += 1;
        }
        prev = Some(name);
    }
    NamespaceSnapshot::build(
        NsKey::schema([DB, schema]),
        DbmsKind::Postgres,
        FoldPolicy::for_dbms(DbmsKind::Postgres),
        meta(),
        t,
    )
    .expect("schema namespace")
}
