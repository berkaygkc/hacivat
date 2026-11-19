//! Generated catalogs for tests and benches (feature `test-support`; enabled only by test
//! dev-dependencies). M4 Task 3, spike origin `d86313b`.
//!
//! - [`ns_pg`], [`ns_mysql`], [`ns_sqlite`]: a small namespace builder for model tests
//!   (`ns_pg("db", "public").table_oid("t", 16400, &["a", "b"]).finish()`).
//! - [`large_pg_catalog`]: the DBeaver #42465 scale the HV-MODEL-03 budget names: 15,000 tables
//!   and 160,000 columns in one PostgreSQL database over five schemas (one of 10,000 tables,
//!   the worst case for a single-namespace reload), with a primary key and an index per table,
//!   a foreign key on every third table, a view every 100 and a function every 50 tables.
//!   Names, types, defaults and comments repeat the way real schemas do (shared column and type
//!   names, per-table defaults), so string interning is measured honestly.
//! - [`pg_catalog`] (any shape), [`table_subtree`] (one table, for the incremental tests).

use std::sync::Arc;

use hacivat_proto::DbmsKind;
use sql_catalog::types::{FnArg, Volatility};

use crate::ids::{NsKey, Ref, ServerId, SourceRef, StateMark};
use crate::kind::ObjectKind;
use crate::name::{DialectId, FoldRule, IdentCtx, Name, NameClass};
use crate::namespace::{
    Level, NamespaceSnapshot, NsMeta, Obj, ObjFlags, PgIdentity, Subtree, Watermark,
};
use crate::props::*;

/// The database name of the generated PostgreSQL catalogs.
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

/// A namespace being built by a test: tables with columns, or any subtree, under its root.
#[derive(Debug, Clone)]
pub struct NsBuilder {
    key: NsKey,
    dbms: DbmsKind,
    fold: FoldRule,
    tree: Subtree,
}

/// A PostgreSQL schema namespace `[db, schema]` (stored rule exact).
pub fn ns_pg(db: &str, schema: &str) -> NsBuilder {
    NsBuilder {
        key: NsKey::schema([db, schema]),
        dbms: DbmsKind::Postgres,
        fold: FoldRule::Exact,
        tree: Subtree::new(Obj::new(ObjectKind::Schema, schema)),
    }
}

/// A MySQL schema namespace `[schema]` under `lower_case_table_names` (0, 1 or 2).
pub fn ns_mysql(schema: &str, lower_case_table_names: u8) -> NsBuilder {
    let ctx = IdentCtx {
        lower_case_table_names,
        ..IdentCtx::new(DialectId::MySql)
    };
    NsBuilder {
        key: NsKey::schema([schema]),
        dbms: DbmsKind::MySql,
        fold: FoldRule::for_names(DialectId::MySql, &ctx, NameClass::Table)
            .expect("lower_case_table_names is 0, 1 or 2"),
        tree: Subtree::new(Obj::new(ObjectKind::Schema, schema)),
    }
}

/// A SQLite schema namespace (`main`, `temp` or an attached name; stored rule ASCII-caseless).
pub fn ns_sqlite(schema: &str) -> NsBuilder {
    NsBuilder {
        key: NsKey::schema([schema]),
        dbms: DbmsKind::Sqlite,
        fold: FoldRule::AsciiCaseless,
        tree: Subtree::new(Obj::new(ObjectKind::Schema, schema)),
    }
}

impl NsBuilder {
    /// Adds table `name` with server id `oid` and `columns` (ordinals from 1).
    pub fn table_oid(self, name: &str, oid: u32, columns: &[&str]) -> Self {
        let root = self.table_root(name).with_server_id(ServerId::Oid(oid));
        self.add_table(root, columns)
    }

    /// Adds table `name` without a server id and with `columns` (ordinals from 1).
    pub fn table(self, name: &str, columns: &[&str]) -> Self {
        let root = self.table_root(name);
        self.add_table(root, columns)
    }

    /// Adds `subtree` under the namespace root.
    pub fn subtree(mut self, subtree: Subtree) -> Self {
        let root = self.tree.root();
        self.tree.graft_under(root, subtree);
        self
    }

    /// The built namespace. Panics on a kind-schema or layout error (a test bug).
    pub fn finish(self) -> NamespaceSnapshot {
        NamespaceSnapshot::build(self.key, self.dbms, self.fold, NsMeta::default(), self.tree)
            .expect("a valid fixture namespace")
    }

    fn table_root(&self, name: &str) -> Obj {
        Obj::new(ObjectKind::Table, name).with_props(Props::Table(Arc::new(TableProps::default())))
    }

    fn add_table(self, root: Obj, columns: &[&str]) -> Self {
        let raw_type: Name = Arc::from(match self.dbms {
            DbmsKind::MySql | DbmsKind::MariaDb => "int",
            DbmsKind::Sqlite => "INTEGER",
            _ => "integer",
        });
        let mut t = Subtree::with_capacity(root, columns.len() + 1);
        for (i, c) in columns.iter().enumerate() {
            t.add(
                t.root(),
                Obj::new(ObjectKind::Column, *c).with_props(Props::Column(Arc::new(ColumnProps {
                    ordinal: i as u32 + 1,
                    raw_type: raw_type.clone(),
                    nullable: true,
                    ..Default::default()
                }))),
            );
        }
        self.subtree(t)
    }
}

/// 15,000 tables, 160,000 columns (see the module docs), plus the database and server
/// namespaces.
pub fn large_pg_catalog() -> Vec<NamespaceSnapshot> {
    pg_catalog(&LARGE_SCHEMAS, 160_000)
}

/// A PostgreSQL catalog in database [`DB`] with `schemas` (name, table count) and
/// `total_columns` columns spread as evenly as possible (the first tables get the remainder),
/// plus the server and database namespaces.
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
            w: 1_234_567,
            identity: PgIdentity {
                system_identifier: 7_301_234_567_890_123_456,
                start_time_micros: 1_763_337_600_000_000,
            },
        },
        loaded_at_ms: 1_763_337_600_000,
    }
}

fn build_pg(key: NsKey, tree: Subtree) -> NamespaceSnapshot {
    NamespaceSnapshot::build(key, DbmsKind::Postgres, FoldRule::Exact, meta(), tree)
        .expect("a valid generated namespace")
}

fn server_ns() -> NamespaceSnapshot {
    let mut t = Subtree::new(Obj::new(ObjectKind::Root, ""));
    for r in ["postgres", "app_owner", "app_reader", "app_writer"] {
        t.add(t.root(), Obj::new(ObjectKind::Role, r));
    }
    t.add(t.root(), Obj::new(ObjectKind::Tablespace, "pg_default"));
    build_pg(NsKey::server(), t)
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
                version: Some(Arc::from(v)),
                ..Default::default()
            }))),
        );
    }
    build_pg(NsKey::database(DB), t)
}

/// One generated table subtree: `name` with `columns` columns, a primary key, an index and
/// (when `fk_to` is set) a foreign key to `fk_to` in the same schema of database [`DB`].
pub fn table_subtree(
    schema: &str,
    name: &str,
    columns: usize,
    oid: u32,
    fk_to: Option<&str>,
) -> Subtree {
    let mut t = Subtree::with_capacity(
        Obj::new(ObjectKind::Table, name)
            .with_server_id(ServerId::Oid(oid))
            .with_state(StateMark::Xmin(oid.wrapping_mul(7)))
            .with_props(Props::Table(Arc::new(TableProps {
                owner: Some(Arc::from("app_owner")),
                comment: oid
                    .is_multiple_of(10)
                    .then(|| Arc::from("Business records")),
                ext: TableExt::Pg(PgTableExt {
                    relkind: b'r',
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
        let default_raw = match k {
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
                    raw_type: Arc::from(TYPES[(k * 7 + oid as usize) % TYPES.len()]),
                    nullable: k != 0,
                    default_raw,
                    comment: (k % 10 == 3).then(|| Arc::from("Free-form note")),
                    ..Default::default()
                },
            ))),
        );
    }
    let pkey: Name = Arc::from(format!("{name}_pkey").as_str());
    t.add(
        root,
        Obj::new(ObjectKind::PrimaryKey, pkey.clone())
            .with_server_id(ServerId::Oid(oid + 1))
            .add_flags(ObjFlags::SERVER_GENERATED_NAME)
            .with_props(Props::Key(Arc::new(KeyProps {
                columns: vec![KeyColumn {
                    ordinal: 1,
                    name: Some(Arc::from("id")),
                }],
                desc: vec![false],
                unique: true,
                enforced: true,
                backing_index: Some(pkey),
                ..Default::default()
            }))),
    );
    t.add(
        root,
        Obj::new(ObjectKind::Index, format!("{name}_created_at_idx").as_str())
            .with_server_id(ServerId::Oid(oid + 2))
            .with_props(Props::Index(Arc::new(IndexProps {
                key: KeyProps {
                    columns: vec![KeyColumn {
                        ordinal: 2,
                        name: Some(Arc::from("created_at")),
                    }],
                    desc: vec![false],
                    enforced: true,
                    ..Default::default()
                },
                method: Some(Arc::from("btree")),
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
            .with_props(Props::ForeignKey(Arc::new(FkProps {
                columns: vec![Arc::from("owner_id")],
                target: Ref {
                    kind: ObjectKind::Table,
                    path: [DB, schema, target].into_iter().map(Name::from).collect(),
                    server_id: None,
                },
                target_columns: vec![Arc::from("id")],
                on_update: FkAction::NoAction,
                on_delete: FkAction::Cascade,
                match_kind: FkMatch::Simple,
                deferrable: false,
                initially_deferred: false,
                not_valid: false,
                enforced: true,
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
    let mut t = Subtree::with_capacity(
        Obj::new(ObjectKind::Schema, schema).with_props(Props::Schema(Arc::new(SchemaProps {
            owner: Some(Arc::from("app_owner")),
            comment: None,
        }))),
        count * 16,
    );
    let root = t.root();
    let mut prev: Option<String> = None;
    for i in 0..count {
        let name = format!("tbl_{i:05}");
        let columns = base + usize::from(*global < extra);
        let fk = if i % 3 == 2 { prev.as_deref() } else { None };
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
                        args: Some(vec![FnArg::input(
                            Some(Arc::from("p_id")),
                            Arc::from("bigint"),
                        )]),
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
    build_pg(NsKey::schema([DB, schema]), t)
}
