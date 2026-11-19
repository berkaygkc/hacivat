//! Fixture catalogs for the DDL producer tests (M4 Task 19; alg/03 HV-DDL-07 edge cases):
//! one catalog per dialect, published through a real `Catalog` and read through a
//! `SnapshotView` (the `children`/`lookup` contract), plus an in-memory [`DdlSources`] for
//! source texts and facets. Server texts are written the way each server prints them
//! (`pg_get_*def`, `SHOW CREATE …`, `sqlite_schema.sql`).
//!
//! The golden cases ([`CASES`]) are shared by `golden.rs` (one test per case) and the
//! byte-stability tests.

#![allow(dead_code)]

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::SystemTime;

use hacivat_ddl::produce::{DdlSources, create_script_with};
use hacivat_ddl::script::DelimStyle;
use hacivat_ddl::{DdlError, DdlTarget, GenOptions, Script};
use hacivat_meta::ids::{NsKey, ObjectKey, Ref, SourceRef};
use hacivat_meta::kind::ObjectKind;
use hacivat_meta::name::{FoldRule, IdentCtx, Name, NameClass};
use hacivat_meta::namespace::{
    Level, NamespaceSnapshot, NsMeta, Obj, ObjFlags, Subtree, Watermark,
};
use hacivat_meta::overlay::Overlays;
use hacivat_meta::props::*;
use hacivat_meta::snapshot::{
    Catalog, CatalogChange, ContainerKey, IntroErrorCode, LoadState, ServerFacts,
};
use hacivat_meta::view::SnapshotView;
use hacivat_proto::DbmsKind;
use smallvec::SmallVec;
use sql_dialect::{DialectId, ServerVersion};

#[allow(unused_imports)]
pub use hacivat_ddl::options::{Placement, Qualify};
pub use hacivat_meta::kind::ObjectKind as K;

pub const DB: &str = "app";

pub fn n(s: &str) -> Name {
    Name::from(s)
}

pub fn names(v: &[&str]) -> Vec<Name> {
    v.iter().map(|s| n(s)).collect()
}

// ---- sources and facets ----------------------------------------------------------------------

/// Source texts by `SourceRef`, `Sources`-facet texts and grants facets by object key.
#[derive(Default)]
pub struct MemSources {
    texts: BTreeMap<u128, Name>,
    server: BTreeMap<ObjectKey, Name>,
    grants: BTreeMap<ObjectKey, Facet<GrantsFacet>>,
}

impl MemSources {
    /// Stores `text` in the content-addressed store; returns its reference.
    pub fn add(&mut self, text: &str) -> SourceRef {
        let r = SourceRef::of(text);
        self.texts.insert(r.hash, n(text));
        r
    }

    /// The `Sources` facet text of `key`.
    pub fn server(&mut self, key: ObjectKey, text: &str) {
        self.server.insert(key, n(text));
    }

    /// The grants facet of `key`.
    pub fn grants_of(&mut self, key: ObjectKey, state: LoadState, entries: Vec<Grant>) {
        self.grants.insert(
            key,
            Facet {
                state,
                value: Some(Arc::new(GrantsFacet { entries })),
            },
        );
    }
}

impl DdlSources for MemSources {
    fn text(&self, r: &SourceRef) -> Option<Name> {
        self.texts.get(&r.hash).cloned()
    }

    fn server_text(&self, object: &ObjectKey) -> Option<Name> {
        self.server.get(object).cloned()
    }

    fn grants(&self, object: &ObjectKey) -> Facet<GrantsFacet> {
        self.grants
            .get(object)
            .cloned()
            .unwrap_or_else(Facet::not_loaded)
    }
}

pub fn loaded() -> LoadState {
    LoadState::Loaded {
        level: Level::Sources,
        at: SystemTime::UNIX_EPOCH,
        watermark: Watermark::None,
        stale: false,
    }
}

pub fn partial() -> LoadState {
    LoadState::Partial {
        reason: IntroErrorCode::PermissionDenied,
    }
}

pub fn grant(grantee: &str, privilege: &str, grantable: bool) -> Grant {
    Grant {
        grantee: n(grantee),
        privilege: n(privilege),
        columns: Vec::new(),
        grantable,
    }
}

// ---- model builders --------------------------------------------------------------------------

pub fn obj(kind: ObjectKind, name: &str, props: Props) -> Obj {
    Obj::new(kind, name).with_props(props)
}

/// A column: its raw type, nullability and the rest of its props.
pub fn col(name: &str, ty: &str, nullable: bool) -> ColumnProps {
    let _ = name;
    ColumnProps {
        raw_type: n(ty),
        nullable,
        ..ColumnProps::default()
    }
}

pub fn column(name: &str, p: ColumnProps) -> Obj {
    obj(ObjectKind::Column, name, Props::Column(Arc::new(p)))
}

pub fn rare(r: ColumnRare) -> Option<Arc<ColumnRare>> {
    Some(Arc::new(r))
}

pub fn key_cols(cols: &[&str]) -> Vec<KeyColumn> {
    cols.iter()
        .enumerate()
        .map(|(i, c)| KeyColumn {
            ordinal: i as u32 + 1,
            name: Some(n(c)),
        })
        .collect()
}

/// A primary or unique key on `cols`; `server_named` marks a server-generated name.
pub fn key(kind: ObjectKind, name: &str, cols: &[&str], server_named: bool) -> Obj {
    let o = obj(
        kind,
        name,
        Props::Key(Arc::new(KeyProps {
            columns: key_cols(cols),
            unique: true,
            enforced: true,
            ..KeyProps::default()
        })),
    );
    if server_named {
        o.add_flags(ObjFlags::SERVER_GENERATED_NAME)
    } else {
        o
    }
}

pub fn target(path: &[&str]) -> Ref {
    Ref {
        kind: ObjectKind::Table,
        path: path.iter().map(|p| n(p)).collect(),
        server_id: None,
    }
}

/// Foreign key props (the one literal of `FkProps`, which has no `Default`).
pub fn fk(cols: &[&str], to: Ref, to_cols: &[&str]) -> FkProps {
    FkProps {
        columns: names(cols),
        target: to,
        target_columns: names(to_cols),
        on_update: FkAction::NoAction,
        on_delete: FkAction::NoAction,
        match_kind: FkMatch::Simple,
        deferrable: false,
        initially_deferred: false,
        not_valid: false,
        enforced: true,
    }
}

pub fn fk_obj(name: &str, p: FkProps) -> Obj {
    obj(ObjectKind::ForeignKey, name, Props::ForeignKey(Arc::new(p)))
}

pub fn check(kind: ObjectKind, name: &str, expr: &str) -> Obj {
    obj(
        kind,
        name,
        Props::Check(Arc::new(CheckProps {
            expr: n(expr),
            not_valid: false,
            enforced: true,
        })),
    )
}

/// Index props on plain columns (`desc` per part, all ascending when empty).
pub fn ix(cols: &[&str], desc: &[bool]) -> IndexProps {
    IndexProps {
        key: KeyProps {
            columns: key_cols(cols),
            desc: if desc.is_empty() {
                vec![false; cols.len()]
            } else {
                desc.to_vec()
            },
            enforced: true,
            ..KeyProps::default()
        },
        ..IndexProps::default()
    }
}

/// Index props of one expression part.
pub fn ix_expr(expr: &str) -> IndexProps {
    IndexProps {
        key: KeyProps {
            columns: vec![KeyColumn {
                ordinal: 0,
                name: None,
            }],
            expressions: vec![n(expr)],
            desc: vec![false],
            enforced: true,
            ..KeyProps::default()
        },
        ..IndexProps::default()
    }
}

pub fn index(name: &str, p: IndexProps) -> Obj {
    obj(ObjectKind::Index, name, Props::Index(Arc::new(p)))
}

pub fn trigger(name: &str, src: SourceRef) -> Obj {
    obj(
        ObjectKind::Trigger,
        name,
        Props::Trigger(Arc::new(TriggerProps {
            source: Some(src),
            enabled: true,
            ..TriggerProps::default()
        })),
    )
}

pub fn table(name: &str, p: TableProps) -> Subtree {
    Subtree::new(obj(ObjectKind::Table, name, Props::Table(Arc::new(p))))
}

pub fn add(t: &mut Subtree, o: Obj) {
    let root = t.root();
    t.add(root, o);
}

pub fn leaf(o: Obj) -> Subtree {
    Subtree::new(o)
}

fn ident_ctx(d: DialectId) -> IdentCtx {
    IdentCtx {
        encoding: (d == DialectId::Postgres).then(|| n("UTF8")),
        ..IdentCtx::new(d)
    }
}

/// A schema namespace of `dbms` at `path` holding `children`.
pub fn namespace(
    dbms: DbmsKind,
    path: &[&str],
    root: Props,
    children: Vec<Subtree>,
) -> NamespaceSnapshot {
    let d = dialect_of(dbms);
    let fold = FoldRule::for_names(d, &ident_ctx(d), NameClass::Table).expect("a fold rule");
    let mut tree = Subtree::new(obj(ObjectKind::Schema, path[path.len() - 1], root));
    let r = tree.root();
    for c in children {
        tree.graft_under(r, c);
    }
    NamespaceSnapshot::build(
        NsKey::schema(path.iter().copied()),
        dbms,
        fold,
        NsMeta {
            level: Level::Sources,
            ..NsMeta::default()
        },
        tree,
    )
    .expect("a valid fixture namespace")
}

pub fn dialect_of(dbms: DbmsKind) -> DialectId {
    match dbms {
        DbmsKind::MySql => DialectId::MySql,
        DbmsKind::MariaDb => DialectId::MariaDb,
        DbmsKind::Sqlite => DialectId::Sqlite,
        _ => DialectId::Postgres,
    }
}

/// A published catalog holding `nss`, each loaded at `Sources`, with its schema list loaded.
pub fn view_of(dbms: DbmsKind, nss: Vec<NamespaceSnapshot>, overlays: Overlays) -> SnapshotView {
    view_at(dbms, nss, overlays, loaded())
}

/// As [`view_of`], every namespace in `state`.
pub fn view_at(
    dbms: DbmsKind,
    nss: Vec<NamespaceSnapshot>,
    overlays: Overlays,
    state: LoadState,
) -> SnapshotView {
    let catalog = Catalog::new(Arc::new(ServerFacts::new(
        dbms,
        ident_ctx(dialect_of(dbms)),
    )));
    let publisher = catalog.publisher().expect("the only publisher");
    let mut changes = Vec::new();
    for ns in nss {
        changes.push(CatalogChange::SetState(
            ContainerKey::Namespace(ns.key().clone()),
            state.clone(),
        ));
        changes.push(CatalogChange::Put(Arc::new(ns)));
    }
    let db = n(DB);
    changes.push(CatalogChange::SetState(
        ContainerKey::schema_list((dbms == DbmsKind::Postgres).then_some(&db)),
        loaded(),
    ));
    publisher.publish(changes).expect("publish");
    SnapshotView::new(catalog.current()).with_layers(Arc::new(overlays))
}

/// A catalog and its sources.
pub struct Fx {
    pub view: SnapshotView,
    pub src: MemSources,
    pub dbms: DbmsKind,
}

impl Fx {
    /// The key of `path` (PostgreSQL paths get the database in front).
    pub fn key(&self, kind: ObjectKind, path: &[&str]) -> ObjectKey {
        let mut p: SmallVec<[Name; 4]> = SmallVec::new();
        if self.dbms == DbmsKind::Postgres {
            p.push(n(DB));
        }
        p.extend(path.iter().map(|s| n(s)));
        ObjectKey {
            kind,
            path: p,
            server_id: None,
            overload_sig: None,
            raw_kind: None,
        }
    }

    /// The key of a routine overload.
    pub fn routine(&self, kind: ObjectKind, path: &[&str], sig: &str) -> ObjectKey {
        ObjectKey {
            overload_sig: Some(n(sig)),
            ..self.key(kind, path)
        }
    }

    pub fn try_script(
        &self,
        keys: &[ObjectKey],
        t: &DdlTarget,
        o: &GenOptions,
    ) -> Result<Script, DdlError> {
        create_script_with(&self.view, &self.src, keys, t, o)
    }

    pub fn script(&self, keys: &[ObjectKey], t: &DdlTarget, o: &GenOptions) -> Script {
        self.try_script(keys, t, o).expect("a script")
    }

    pub fn text(&self, keys: &[ObjectKey], t: &DdlTarget, o: &GenOptions) -> String {
        self.script(keys, t, o).text(DelimStyle::Semicolon)
    }
}

// ---- targets ---------------------------------------------------------------------------------

pub fn pg_target() -> DdlTarget {
    let mut t = DdlTarget::new(DialectId::Postgres, ServerVersion::new(17, 2, 0));
    t.current_schema = Some(n("shop"));
    t.quote.standard_conforming_strings = Some(true);
    t
}

pub fn mysql_target(d: DialectId) -> DdlTarget {
    let v = if d == DialectId::MariaDb {
        ServerVersion::new(11, 4, 0)
    } else {
        ServerVersion::new(8, 4, 0)
    };
    let mut t = DdlTarget::new(d, v);
    t.current_schema = Some(n("shop"));
    t.quote.no_backslash_escapes = Some(false);
    t
}

pub fn sqlite_target() -> DdlTarget {
    let mut t = DdlTarget::new(DialectId::Sqlite, ServerVersion::new(3, 46, 0));
    t.current_schema = Some(n("main"));
    t
}

pub fn opts() -> GenOptions {
    GenOptions::default()
}

pub fn no_grants() -> GenOptions {
    GenOptions {
        skip_grants: true,
        ..GenOptions::default()
    }
}

// ---- PostgreSQL --------------------------------------------------------------------------------

fn pg_table(owner: Option<&str>, comment: Option<&str>) -> TableProps {
    TableProps {
        owner: owner.map(n),
        comment: comment.map(n),
        ext: TableExt::Pg(PgTableExt {
            relkind: b'r',
            ..PgTableExt::default()
        }),
        ..TableProps::default()
    }
}

fn not_null(ty: &str) -> ColumnProps {
    col("", ty, false)
}

fn pg_seq(name: &str, p: SeqProps) -> Subtree {
    leaf(obj(
        ObjectKind::Sequence,
        name,
        Props::Sequence(Arc::new(p)),
    ))
}

fn pg_type(kind: ObjectKind, name: &str, body: TypeBody, comment: Option<&str>) -> Subtree {
    leaf(obj(
        kind,
        name,
        Props::Type(Arc::new(TypeProps {
            owner: Some(n("app_owner")),
            comment: comment.map(n),
            body,
        })),
    ))
}

fn pg_routine(name: &str, sig: &str, src: SourceRef, comment: Option<&str>) -> Subtree {
    leaf(obj(
        ObjectKind::Routine,
        name,
        Props::Routine(Arc::new(RoutineProps {
            overload_sig: Some(n(sig)),
            owner: Some(n("app_owner")),
            source: Some(src),
            comment: comment.map(n),
            ..RoutineProps::default()
        })),
    ))
}

/// The PostgreSQL catalog: database `app`, schemas `shop` (current) and `sales`.
pub fn pg() -> Fx {
    pg_with(Overlays::default())
}

pub fn pg_with(overlays: Overlays) -> Fx {
    let mut src = MemSources::default();
    let mut shop: Vec<Subtree> = Vec::new();
    let shop_ref = |t: &str| target(&[DB, "shop", t]);

    // shop.customers (sales has one too: an ambiguous name).
    let mut customers = table("customers", pg_table(None, None));
    add(&mut customers, column("id", not_null("bigint")));
    add(
        &mut customers,
        key(ObjectKind::PrimaryKey, "customers_pkey", &["id"], true),
    );
    add(
        &mut customers,
        index(
            "customers_pkey",
            IndexProps {
                key: KeyProps {
                    unique: true,
                    ..ix(&["id"], &[]).key
                },
                ..IndexProps::default()
            },
        )
        .add_flags(ObjFlags::IMPLICIT),
    );
    shop.push(customers);

    // shop.orders: the full table.
    let mut orders = table(
        "orders",
        pg_table(Some("app_owner"), Some("Customer orders")),
    );
    add(
        &mut orders,
        column(
            "id",
            ColumnProps {
                rare: rare(ColumnRare {
                    identity: Some(Identity::Always),
                    ..ColumnRare::default()
                }),
                ..not_null("bigint")
            },
        ),
    );
    add(&mut orders, column("customer_id", not_null("bigint")));
    add(
        &mut orders,
        column(
            "code",
            ColumnProps {
                rare: rare(ColumnRare {
                    collation: Some(n("C")),
                    ..ColumnRare::default()
                }),
                ..not_null("character varying(32)")
            },
        ),
    );
    add(
        &mut orders,
        column(
            "status",
            ColumnProps {
                default_raw: Some(n("'new'::text")),
                ..not_null("text")
            },
        ),
    );
    add(
        &mut orders,
        column(
            "amount",
            ColumnProps {
                default_raw: Some(n("0")),
                ..not_null("numeric(12,2)")
            },
        ),
    );
    add(
        &mut orders,
        column(
            "total",
            ColumnProps {
                rare: rare(ColumnRare {
                    generated: Some(Generated {
                        kind: GeneratedKind::Stored,
                        expr: n("(amount * 1.2)"),
                    }),
                    ..ColumnRare::default()
                }),
                ..col("", "numeric", true)
            },
        ),
    );
    add(
        &mut orders,
        column(
            "note",
            ColumnProps {
                comment: Some(n("PM's approver of record")),
                ..col("", "text", true)
            },
        ),
    );
    add(
        &mut orders,
        column(
            "created_at",
            ColumnProps {
                default_raw: Some(n("now()")),
                ..not_null("timestamp with time zone")
            },
        ),
    );
    add(
        &mut orders,
        key(ObjectKind::PrimaryKey, "orders_pkey", &["id"], true),
    );
    add(
        &mut orders,
        key(ObjectKind::UniqueKey, "orders_code_uq", &["code"], false),
    );
    add(
        &mut orders,
        fk_obj(
            "orders_customer_fk",
            FkProps {
                on_delete: FkAction::Cascade,
                ..fk(&["customer_id"], shop_ref("customers"), &["id"])
            },
        ),
    );
    add(
        &mut orders,
        check(
            ObjectKind::Check,
            "orders_amount_check",
            "(amount >= (0)::numeric)",
        )
        .add_flags(ObjFlags::SERVER_GENERATED_NAME),
    );
    // The indexes backing the keys are in the model; they are never re-created (one flagged
    // IMPLICIT, one found by its key's name).
    add(
        &mut orders,
        index("orders_pkey", ix(&["id"], &[])).add_flags(ObjFlags::IMPLICIT),
    );
    add(&mut orders, index("orders_code_uq", ix(&["code"], &[])));
    add(
        &mut orders,
        index(
            "orders_status_idx",
            IndexProps {
                method: Some(n("btree")),
                ..ix(&["status"], &[])
            },
        ),
    );
    let mut open = ix(&["created_at"], &[true]);
    open.method = Some(n("btree"));
    open.key.include = names(&["id"]);
    open.key.predicate = Some(n("(status <> 'done'::text)"));
    open.comment = Some(n("Open orders, newest first"));
    add(&mut orders, index("orders_open_idx", open));
    add(
        &mut orders,
        index(
            "orders_lower_note_idx",
            IndexProps {
                method: Some(n("btree")),
                ..ix_expr("lower(note)")
            },
        ),
    );
    add(
        &mut orders,
        index(
            "orders_code_hash",
            IndexProps {
                method: Some(n("hash")),
                ..ix(&["code"], &[])
            },
        ),
    );
    let audit = src.add(
        "CREATE TRIGGER orders_audit AFTER INSERT OR UPDATE ON shop.orders FOR EACH ROW EXECUTE FUNCTION shop.audit_row()",
    );
    add(&mut orders, trigger("orders_audit", audit));
    shop.push(orders);
    // The identity column's own sequence: implicit, never emitted for the table.
    shop.push(leaf(
        obj(
            ObjectKind::Sequence,
            "orders_id_seq",
            Props::Sequence(Arc::new(SeqProps {
                owned_by: Some((shop_ref("orders"), n("id"))),
                ..SeqProps::default()
            })),
        )
        .add_flags(ObjFlags::IMPLICIT),
    ));

    // shop.empty: zero columns.
    shop.push(table("empty", pg_table(None, None)));

    // shop.events, partitioned, and one partition.
    let mut events = table(
        "events",
        TableProps {
            partitioned: true,
            partition_key: Some(n("RANGE (created_at)")),
            ..pg_table(None, None)
        },
    );
    add(&mut events, column("id", not_null("bigint")));
    add(
        &mut events,
        column("created_at", not_null("timestamp with time zone")),
    );
    add(&mut events, column("payload", col("", "jsonb", true)));
    add(
        &mut events,
        key(
            ObjectKind::PrimaryKey,
            "events_pkey",
            &["id", "created_at"],
            true,
        ),
    );
    shop.push(events);
    let mut ev26 = table(
        "events_2026",
        TableProps {
            partition_of: Some(shop_ref("events")),
            partition_bound: Some(n(
                "FOR VALUES FROM ('2026-01-01 00:00:00+00') TO ('2027-01-01 00:00:00+00')",
            )),
            ext: TableExt::Pg(PgTableExt {
                relkind: b'r',
                options: names(&["fillfactor=90"]),
                tablespace: Some(n("fast_ssd")),
                ..PgTableExt::default()
            }),
            ..TableProps::default()
        },
    );
    for (c, ty) in [
        ("id", "bigint"),
        ("created_at", "timestamp with time zone"),
        ("payload", "jsonb"),
    ] {
        add(
            &mut ev26,
            column(c, col("", ty, c == "payload")).add_flags(ObjFlags::INHERITED),
        );
    }
    add(
        &mut ev26,
        key(
            ObjectKind::PrimaryKey,
            "events_2026_pkey",
            &["id", "created_at"],
            true,
        )
        .add_flags(ObjFlags::INHERITED),
    );
    add(
        &mut ev26,
        index("events_2026_pkey", ix(&["id", "created_at"], &[]))
            .add_flags(ObjFlags::INHERITED | ObjFlags::IMPLICIT),
    );
    add(
        &mut ev26,
        check(
            ObjectKind::Check,
            "events_2026_payload_check",
            "(payload IS NOT NULL)",
        ),
    );
    shop.push(ev26);

    // An inheritance child (not a partition): its parent list is not in the model yet.
    let mut child = table("child_t", pg_table(None, None));
    add(
        &mut child,
        column("id", not_null("bigint")).add_flags(ObjFlags::INHERITED),
    );
    add(&mut child, column("extra", col("", "text", true)));

    // Serial vs identity: `tickets.id` owns `tickets_id_seq`; `invoice_no` stands alone.
    let mut tickets = table("tickets", pg_table(None, None));
    add(
        &mut tickets,
        column(
            "id",
            ColumnProps {
                default_raw: Some(n("nextval('shop.tickets_id_seq'::regclass)")),
                ..not_null("integer")
            },
        ),
    );
    add(&mut tickets, column("title", col("", "text", true)));
    shop.push(tickets);
    shop.push(pg_seq(
        "tickets_id_seq",
        SeqProps {
            data_type: Some(n("integer")),
            start: Some(1),
            increment: Some(1),
            min: Some(1),
            max: Some(2_147_483_647),
            cache: Some(1),
            owned_by: Some((shop_ref("tickets"), n("id"))),
            ..SeqProps::default()
        },
    ));
    shop.push(pg_seq(
        "invoice_no",
        SeqProps {
            owner: Some(n("app_owner")),
            data_type: Some(n("bigint")),
            start: Some(1000),
            increment: Some(10),
            min: Some(1),
            max: Some(i64::MAX),
            cache: Some(20),
            comment: Some(n("Invoice numbers")),
            ..SeqProps::default()
        },
    ));

    // Views.
    let active = src.add(
        " SELECT orders.id,\n    orders.status\n   FROM shop.orders\n  WHERE (orders.status <> 'done'::text);",
    );
    let mut active_orders = Subtree::new(obj(
        ObjectKind::View,
        "active_orders",
        Props::View(Arc::new(ViewProps {
            owner: Some(n("app_owner")),
            comment: Some(n("Open orders")),
            source: Some(active),
            check_option: Some(n("local")),
            ..ViewProps::default()
        })),
    ));
    add(&mut active_orders, column("id", col("", "bigint", true)));
    add(
        &mut active_orders,
        column(
            "status",
            ColumnProps {
                comment: Some(n("Never 'done'")),
                ..col("", "text", true)
            },
        ),
    );
    shop.push(active_orders);
    let totals = src.add(
        " SELECT orders.customer_id,\n    sum(orders.amount) AS total\n   FROM shop.orders\n  GROUP BY orders.customer_id;",
    );
    let mut order_totals = Subtree::new(obj(
        ObjectKind::MaterializedView,
        "order_totals",
        Props::View(Arc::new(ViewProps {
            owner: Some(n("app_owner")),
            source: Some(totals),
            ..ViewProps::default()
        })),
    ));
    add(
        &mut order_totals,
        column("customer_id", col("", "bigint", true)),
    );
    add(&mut order_totals, column("total", col("", "numeric", true)));
    add(
        &mut order_totals,
        index("order_totals_customer_idx", ix(&["customer_id"], &[])),
    );
    shop.push(order_totals);

    // Types.
    shop.push(pg_type(
        ObjectKind::EnumType,
        "mood",
        TypeBody::Enum {
            labels: vec![n("sad"), n("ok"), n("it's fine")].into(),
        },
        Some("How it went"),
    ));
    shop.push(pg_type(
        ObjectKind::Domain,
        "posint",
        TypeBody::Domain {
            base: n("integer"),
            not_null: true,
            not_null_constraint: None,
            default_raw: Some(n("1")),
            checks: names(&["(VALUE > 0)"]),
        },
        None,
    ));
    shop.push(pg_type(
        ObjectKind::RangeType,
        "floatrange",
        TypeBody::Range {
            subtype: n("double precision"),
        },
        None,
    ));
    shop.push(pg_type(
        ObjectKind::CompositeType,
        "address",
        TypeBody::Composite {
            attrs: vec![
                TypeAttr {
                    name: n("street"),
                    raw_type: n("text"),
                },
                TypeAttr {
                    name: n("city"),
                    raw_type: n("character varying(64)"),
                },
            ],
        },
        None,
    ));

    // Functions and a trigger.
    let touch = src.add(
        "CREATE OR REPLACE FUNCTION shop.touch()\n RETURNS trigger\n LANGUAGE plpgsql\nAS $function$\nBEGIN\n  NEW.updated_at := now(); -- keep it fresh\n  RETURN NEW;\nEND\n$function$\n",
    );
    shop.push(pg_routine("touch", "", touch, Some("Sets updated_at")));
    let add_tax = src.add(
        "CREATE OR REPLACE FUNCTION shop.add_tax(amount numeric, rate numeric)\n RETURNS numeric\n LANGUAGE sql\n IMMUTABLE\nBEGIN ATOMIC\n SELECT (add_tax.amount * ((1)::numeric + add_tax.rate));\nEND\n",
    );
    shop.push(pg_routine("add_tax", "numeric, numeric", add_tax, None));
    let mut items = table("items", pg_table(None, None));
    add(&mut items, column("id", not_null("integer")));
    add(
        &mut items,
        column("updated_at", col("", "timestamp with time zone", true)),
    );
    let items_touch = src.add(
        "CREATE TRIGGER items_touch BEFORE UPDATE ON shop.items FOR EACH ROW EXECUTE FUNCTION shop.touch()",
    );
    add(&mut items, trigger("items_touch", items_touch));
    shop.push(items);

    // Turkish and mixed-case names.
    let mut tr = table(
        "MÜŞTERİ",
        pg_table(None, Some("Müşteri's table -- don't\nedit")),
    );
    add(&mut tr, column("Ad", col("", "text", true)));
    add(&mut tr, column("order", col("", "integer", true)));
    add(&mut tr, column("müşteri_no", not_null("integer")));
    add(&mut tr, column("ılık", col("", "boolean", true)));
    add(
        &mut tr,
        key(
            ObjectKind::PrimaryKey,
            "MÜŞTERİ_pkey",
            &["müşteri_no"],
            true,
        ),
    );
    shop.push(tr);

    // Row-level security.
    let mut secrets = table(
        "secrets",
        TableProps {
            row_security: true,
            ..pg_table(None, None)
        },
    );
    add(&mut secrets, column("id", not_null("integer")));
    shop.push(secrets);

    // An object of a kind this build does not know (a newer relkind).
    shop.push(leaf(obj(
        ObjectKind::Unknown,
        "future_thing",
        Props::Generic(Arc::new(GenericProps {
            raw_kind: Some(n("relkind:z")),
            ..GenericProps::default()
        })),
    )));

    let mut sales_customers = table("customers", pg_table(None, None));
    add(&mut sales_customers, column("id", not_null("bigint")));
    let shop_ns = namespace(
        DbmsKind::Postgres,
        &[DB, "shop"],
        Props::Schema(Arc::new(SchemaProps {
            owner: Some(n("app_owner")),
            comment: Some(n("The shop")),
        })),
        shop,
    );
    let sales_ns = namespace(
        DbmsKind::Postgres,
        &[DB, "sales"],
        Props::None,
        vec![sales_customers, child],
    );
    let mut fx = Fx {
        view: view_of(DbmsKind::Postgres, vec![shop_ns, sales_ns], overlays),
        src,
        dbms: DbmsKind::Postgres,
    };
    // `pg_get_indexdef` for the indexes whose definition text matters.
    let k = fx.key(ObjectKind::Index, &["shop", "orders", "orders_open_idx"]);
    fx.src.server(
        k,
        "CREATE INDEX orders_open_idx ON shop.orders USING btree (created_at DESC NULLS LAST) INCLUDE (id) WHERE (status <> 'done'::text)",
    );
    let k = fx.key(
        ObjectKind::Index,
        &["shop", "orders", "orders_lower_note_idx"],
    );
    fx.src.server(
        k,
        "CREATE INDEX orders_lower_note_idx ON shop.orders USING btree (lower(note) text_pattern_ops)",
    );
    // Grants of `orders`: loaded.
    let k = fx.key(ObjectKind::Table, &["shop", "orders"]);
    fx.src.grants_of(
        k,
        loaded(),
        vec![
            grant("app_reader", "SELECT", false),
            grant("app_writer", "UPDATE", true),
            grant("app_writer", "INSERT", true),
            Grant {
                columns: names(&["status", "code"]),
                ..grant("PUBLIC", "SELECT", false)
            },
        ],
    );
    fx
}

// ---- MySQL / MariaDB -----------------------------------------------------------------------------

fn my_col(ty: &str, column_type: &str, extra: Option<&str>, nullable: bool) -> ColumnProps {
    ColumnProps {
        ext: Some(Arc::new(ColumnExt::MySql {
            column_type: Some(n(column_type)),
            extra: extra.map(n),
            charset: None,
        })),
        ..col("", ty, nullable)
    }
}

fn implicit_index(name: &str, cols: &[&str]) -> Obj {
    index(name, ix(cols, &[])).add_flags(ObjFlags::IMPLICIT)
}

/// The MySQL (8.4) or MariaDB (11.4) catalog: schema `shop` (current).
pub fn mysql(dbms: DbmsKind) -> Fx {
    let maria = dbms == DbmsKind::MariaDb;
    let now = if maria {
        "current_timestamp()"
    } else {
        "CURRENT_TIMESTAMP"
    };
    let mut src = MemSources::default();
    let mut shop: Vec<Subtree> = Vec::new();
    let shop_ref = |t: &str| target(&["shop", t]);

    let mut customers = table("customers", TableProps::default());
    add(
        &mut customers,
        column("id", my_col("int", "int", None, false)),
    );
    add(
        &mut customers,
        key(ObjectKind::PrimaryKey, "PRIMARY", &["id"], false),
    );
    add(&mut customers, implicit_index("PRIMARY", &["id"]));
    shop.push(customers);

    let mut orders = table(
        "orders",
        TableProps {
            comment: Some(n("Orders")),
            ext: TableExt::MySql {
                engine: Some(n("InnoDB")),
                collation: Some(n(if maria {
                    "utf8mb4_uca1400_ai_ci"
                } else {
                    "utf8mb4_0900_ai_ci"
                })),
                options: Some(n("row_format=DYNAMIC")),
            },
            ..TableProps::default()
        },
    );
    add(
        &mut orders,
        column(
            "id",
            ColumnProps {
                rare: rare(ColumnRare {
                    identity: Some(Identity::AutoIncrement),
                    ..ColumnRare::default()
                }),
                ..my_col("bigint", "bigint unsigned", Some("auto_increment"), false)
            },
        ),
    );
    add(
        &mut orders,
        column("customer_id", my_col("int", "int", None, false)),
    );
    add(
        &mut orders,
        column(
            "code",
            ColumnProps {
                rare: rare(ColumnRare {
                    collation: Some(n("utf8mb4_bin")),
                    ..ColumnRare::default()
                }),
                ..my_col("varchar", "varchar(32)", None, false)
            },
        ),
    );
    add(
        &mut orders,
        column(
            "status",
            ColumnProps {
                default_raw: Some(n("'new'")),
                ..my_col("enum", "enum('new','done','')", None, false)
            },
        ),
    );
    add(
        &mut orders,
        column(
            "amount",
            ColumnProps {
                default_raw: Some(n("'0.00'")),
                ..my_col("decimal", "decimal(12,2)", None, false)
            },
        ),
    );
    add(
        &mut orders,
        column(
            "total",
            ColumnProps {
                rare: rare(ColumnRare {
                    generated: Some(Generated {
                        kind: GeneratedKind::Virtual,
                        expr: n("(`amount` * 1.2)"),
                    }),
                    ..ColumnRare::default()
                }),
                ..my_col("decimal", "decimal(14,2)", Some("VIRTUAL GENERATED"), true)
            },
        ),
    );
    let on_update = format!("DEFAULT_GENERATED on update {now}");
    add(
        &mut orders,
        column(
            "updated_at",
            ColumnProps {
                default_raw: Some(n(now)),
                ..my_col("timestamp", "timestamp", Some(&on_update), true)
            },
        ),
    );
    add(
        &mut orders,
        column(
            "note",
            ColumnProps {
                comment: Some(n("Approver's path: C:\\orders")),
                ..my_col("text", "text", None, true)
            },
        ),
    );
    add(
        &mut orders,
        column(
            "secret",
            ColumnProps {
                hidden: true,
                ..my_col("int", "int", Some("INVISIBLE"), true)
            },
        ),
    );
    add(
        &mut orders,
        key(ObjectKind::PrimaryKey, "PRIMARY", &["id"], false),
    );
    add(
        &mut orders,
        key(ObjectKind::UniqueKey, "code_uq", &["code"], false),
    );
    add(
        &mut orders,
        key(ObjectKind::UniqueKey, "note_uq", &["note"], false),
    );
    add(
        &mut orders,
        fk_obj(
            "fk_customer",
            FkProps {
                on_delete: FkAction::Cascade,
                ..fk(&["customer_id"], shop_ref("customers"), &["id"])
            },
        ),
    );
    add(
        &mut orders,
        check(ObjectKind::Check, "orders_chk_1", "(`amount` >= 0)")
            .add_flags(ObjFlags::SERVER_GENERATED_NAME),
    );
    add(&mut orders, implicit_index("PRIMARY", &["id"]));
    add(&mut orders, implicit_index("code_uq", &["code"]));
    add(
        &mut orders,
        index(
            "note_uq",
            IndexProps {
                prefix_len: vec![Some(20)],
                ..ix(&["note"], &[])
            },
        )
        .add_flags(ObjFlags::IMPLICIT),
    );
    add(&mut orders, implicit_index("fk_customer", &["customer_id"]));
    add(
        &mut orders,
        index(
            "idx_status",
            IndexProps {
                method: Some(n("BTREE")),
                ..ix(&["status"], &[])
            },
        ),
    );
    add(
        &mut orders,
        index(
            "idx_customer_status",
            IndexProps {
                method: Some(n("BTREE")),
                comment: Some(n("Per-customer queue")),
                ..ix(&["customer_id", "status"], &[false, true])
            },
        ),
    );
    add(
        &mut orders,
        index(
            "idx_note_prefix",
            IndexProps {
                method: Some(n("BTREE")),
                prefix_len: vec![Some(10)],
                ..ix(&["note"], &[])
            },
        ),
    );
    add(
        &mut orders,
        index(
            "ft_note",
            IndexProps {
                method: Some(n("FULLTEXT")),
                ..ix(&["note"], &[])
            },
        ),
    );
    add(
        &mut orders,
        index(
            "idx_lower_code",
            IndexProps {
                method: Some(n("BTREE")),
                ..ix_expr("lower(`code`)")
            },
        ),
    );
    let mut hidden = ix(&["amount"], &[]);
    hidden.method = Some(n("BTREE"));
    hidden.key.invisible = true;
    add(&mut orders, index("idx_amount_hidden", hidden));
    let bi = src.add(
        "CREATE DEFINER=`root`@`%` TRIGGER `orders_bi` BEFORE INSERT ON `orders` FOR EACH ROW SET NEW.code = UPPER(NEW.code)",
    );
    add(&mut orders, trigger("orders_bi", bi));
    shop.push(orders);

    // An FK whose name is also an explicit (non-implicit) index's name.
    let mut lines = table("lines", TableProps::default());
    add(
        &mut lines,
        column("order_id", my_col("bigint", "bigint unsigned", None, false)),
    );
    add(&mut lines, column("no", my_col("int", "int", None, false)));
    add(
        &mut lines,
        fk_obj(
            "lines_order",
            fk(&["order_id"], shop_ref("orders"), &["id"]),
        ),
    );
    add(
        &mut lines,
        index(
            "lines_order",
            IndexProps {
                method: Some(n("BTREE")),
                ..ix(&["order_id", "no"], &[])
            },
        ),
    );
    shop.push(lines);

    // A trigger whose server text holds an executable comment.
    let mut audit = table("audit", TableProps::default());
    add(
        &mut audit,
        column("at", my_col("datetime", "datetime", None, true)),
    );
    let audit_bi = src.add(
        "CREATE /*!50017 DEFINER=`root`@`%`*/ TRIGGER `audit_bi` BEFORE INSERT ON `audit` FOR EACH ROW SET NEW.at = NOW()",
    );
    add(&mut audit, trigger("audit_bi", audit_bi));
    shop.push(audit);

    // A view and routines (`SHOW CREATE` texts).
    let v = src.add(if maria {
        "CREATE ALGORITHM=UNDEFINED DEFINER=`root`@`%` SQL SECURITY DEFINER VIEW `v_open` AS select `orders`.`id` AS `id` from `orders` where `orders`.`status` <> 'done'"
    } else {
        "CREATE ALGORITHM=UNDEFINED DEFINER=`root`@`%` SQL SECURITY DEFINER VIEW `v_open` AS select `orders`.`id` AS `id` from `orders` where (`orders`.`status` <> 'done')"
    });
    shop.push(leaf(obj(
        ObjectKind::View,
        "v_open",
        Props::View(Arc::new(ViewProps {
            source: Some(v),
            definer: Some(n("root@%")),
            ..ViewProps::default()
        })),
    )));
    let p = src.add(
        "CREATE DEFINER=`app`@`%` PROCEDURE `p_close`(IN p_id BIGINT)\nBEGIN\n  UPDATE orders SET status = 'done' WHERE id = p_id;\nEND",
    );
    shop.push(leaf(obj(
        ObjectKind::Procedure,
        "p_close",
        Props::Routine(Arc::new(RoutineProps {
            kind: sql_catalog::types::FnKind::Procedure,
            source: Some(p),
            ..RoutineProps::default()
        })),
    )));
    let f = src.add(
        "CREATE DEFINER=`app`@`%` FUNCTION `f_tax`(x DECIMAL(12,2)) RETURNS decimal(12,2)\n    DETERMINISTIC\nRETURN x * 1.2",
    );
    shop.push(leaf(obj(
        ObjectKind::Routine,
        "f_tax",
        Props::Routine(Arc::new(RoutineProps {
            source: Some(f),
            ..RoutineProps::default()
        })),
    )));
    if maria {
        shop.push(leaf(obj(
            ObjectKind::Sequence,
            "seq_invoice",
            Props::Sequence(Arc::new(SeqProps {
                start: Some(1000),
                increment: Some(1),
                min: Some(1),
                max: Some(i64::MAX - 1),
                cache: Some(1000),
                ..SeqProps::default()
            })),
        )));
    }

    let shop_ns = namespace(dbms, &["shop"], Props::None, shop);
    let mut fx = Fx {
        view: view_of(dbms, vec![shop_ns], Overlays::default()),
        src,
        dbms,
    };
    let k = fx.key(ObjectKind::Table, &["shop", "orders"]);
    fx.src.grants_of(
        k.clone(),
        partial(),
        vec![
            grant("'app'@'%'", "SELECT", false),
            grant("'app'@'%'", "INSERT", false),
            Grant {
                columns: names(&["code"]),
                ..grant("'o''brien'@'localhost'", "SELECT", false)
            },
        ],
    );
    fx.src.server(
        k,
        "CREATE TABLE `orders` (\n  `id` bigint unsigned NOT NULL AUTO_INCREMENT,\n  PRIMARY KEY (`id`)\n) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_0900_ai_ci",
    );
    fx
}

// ---- SQLite ----------------------------------------------------------------------------------------

/// The SQLite catalog: schema `main`; every table, index, view and trigger has its
/// `sqlite_schema.sql` text in the `Sources` facet or its `SourceRef`.
pub fn sqlite() -> Fx {
    let mut src = MemSources::default();
    let mut main: Vec<Subtree> = Vec::new();

    let mut authors = table(
        "authors",
        TableProps {
            ext: TableExt::Sqlite {
                without_rowid: false,
                strict: false,
                module: None,
            },
            ..TableProps::default()
        },
    );
    add(
        &mut authors,
        column(
            "id",
            ColumnProps {
                rare: rare(ColumnRare {
                    identity: Some(Identity::AutoIncrement),
                    ..ColumnRare::default()
                }),
                ..col("", "INTEGER", true)
            },
        ),
    );
    add(&mut authors, column("name", col("", "TEXT", false)));
    add(
        &mut authors,
        key(ObjectKind::UniqueKey, "authors_name_uq", &["name"], true),
    );
    add(
        &mut authors,
        index("sqlite_autoindex_authors_1", ix(&["name"], &[])).add_flags(ObjFlags::IMPLICIT),
    );
    main.push(authors);

    let mut notes = table(
        "notes",
        TableProps {
            comment: Some(n("A Hacivat annotation, never DDL")),
            ext: TableExt::Sqlite {
                without_rowid: true,
                strict: true,
                module: None,
            },
            ..TableProps::default()
        },
    );
    add(&mut notes, column("id", col("", "INTEGER", false)));
    add(
        &mut notes,
        column(
            "author_id",
            ColumnProps {
                comment: Some(n("Never written: SQLite has no comments")),
                ..col("", "INTEGER", true)
            },
        ),
    );
    add(&mut notes, column("body", col("", "TEXT", true)));
    add(
        &mut notes,
        fk_obj(
            "notes_author_id_authors_fkey",
            fk(&["author_id"], target(&["main", "authors"]), &["id"]),
        )
        .add_flags(ObjFlags::SERVER_GENERATED_NAME),
    );
    let mut partial_ix = ix(&["author_id"], &[]);
    partial_ix.key.predicate = Some(n("author_id IS NOT NULL"));
    partial_ix.comment = Some(n("Never written either"));
    add(&mut notes, index("notes_author_idx", partial_ix));
    let ai = src.add(
        "CREATE TRIGGER notes_ai AFTER INSERT ON notes BEGIN\n  UPDATE authors SET name = name WHERE id = NEW.author_id;\nEND",
    );
    add(&mut notes, trigger("notes_ai", ai));
    main.push(notes);

    let v = src.add("CREATE VIEW v_notes AS SELECT id, body FROM notes");
    main.push(leaf(obj(
        ObjectKind::View,
        "v_notes",
        Props::View(Arc::new(ViewProps {
            source: Some(v),
            comment: Some(n("No comment reaches SQLite")),
            ..ViewProps::default()
        })),
    )));

    let main_ns = namespace(DbmsKind::Sqlite, &["main"], Props::None, main);
    let mut fx = Fx {
        view: view_of(DbmsKind::Sqlite, vec![main_ns], Overlays::default()),
        src,
        dbms: DbmsKind::Sqlite,
    };
    let k = fx.key(ObjectKind::Table, &["main", "authors"]);
    fx.src.server(
        k,
        "CREATE TABLE authors (\n  id INTEGER PRIMARY KEY AUTOINCREMENT,\n  name TEXT NOT NULL COLLATE NOCASE UNIQUE\n)",
    );
    let k = fx.key(ObjectKind::Table, &["main", "notes"]);
    fx.src.server(
        k,
        "CREATE TABLE notes (\n  id INTEGER NOT NULL,\n  author_id INTEGER REFERENCES authors(id) ON DELETE SET NULL,\n  body TEXT CHECK (length(body) < 1000),\n  words INTEGER GENERATED ALWAYS AS (length(body) - length(replace(body, ' ', '')) + 1) VIRTUAL,\n  PRIMARY KEY (id)\n) STRICT, WITHOUT ROWID",
    );
    let k = fx.key(ObjectKind::Index, &["main", "notes", "notes_author_idx"]);
    fx.src.server(
        k,
        "CREATE INDEX notes_author_idx ON notes (author_id) WHERE author_id IS NOT NULL",
    );
    fx
}

// ---- goldens ---------------------------------------------------------------------------------------

pub fn golden_path(rel: &str) -> PathBuf {
    [env!("CARGO_MANIFEST_DIR"), "tests", "golden"]
        .iter()
        .collect::<PathBuf>()
        .join(format!("{rel}.sql"))
}

/// Compares `actual` with the golden `rel`; `UPDATE_GOLDEN=1` writes it instead (R-M4-16).
pub fn check_golden(rel: &str, actual: &str) {
    let path = golden_path(rel);
    if std::env::var_os("UPDATE_GOLDEN").is_some_and(|v| v == "1") {
        std::fs::create_dir_all(path.parent().expect("golden dir")).expect("mkdir");
        std::fs::write(&path, actual.as_bytes()).expect("write golden");
        return;
    }
    let expected = std::fs::read(&path).unwrap_or_else(|_| {
        panic!(
            "missing golden {}; bless with UPDATE_GOLDEN=1 and review it",
            path.display()
        )
    });
    assert!(
        expected == actual.as_bytes(),
        "golden {rel} differs\n--- expected\n{}\n--- actual\n{actual}",
        String::from_utf8_lossy(&expected)
    );
}

/// One golden case: its file and how to produce its text.
pub struct Case {
    pub rel: &'static str,
    pub make: fn() -> String,
}

fn pg_text(paths: &[(ObjectKind, &[&str])], o: GenOptions) -> String {
    let fx = pg();
    let keys: Vec<ObjectKey> = paths.iter().map(|(k, p)| fx.key(*k, p)).collect();
    fx.text(&keys, &pg_target(), &o)
}

fn my_text(
    dbms: DbmsKind,
    paths: &[(ObjectKind, &[&str])],
    o: GenOptions,
    style: DelimStyle,
) -> String {
    let fx = mysql(dbms);
    let keys: Vec<ObjectKey> = paths.iter().map(|(k, p)| fx.key(*k, p)).collect();
    fx.script(&keys, &mysql_target(dialect_of(dbms)), &o)
        .text(style)
}

fn sqlite_text(paths: &[(ObjectKind, &[&str])], o: GenOptions) -> String {
    let fx = sqlite();
    let keys: Vec<ObjectKey> = paths.iter().map(|(k, p)| fx.key(*k, p)).collect();
    fx.text(&keys, &sqlite_target(), &o)
}

/// Every golden of `golden.rs`, by file.
pub const CASES: &[Case] = &[
    Case {
        rel: "pg/table_full",
        make: || pg_text(&[(K::Table, &["shop", "orders"])], opts()),
    },
    Case {
        rel: "pg/zero_column_table",
        make: || pg_text(&[(K::Table, &["shop", "empty"])], no_grants()),
    },
    Case {
        rel: "pg/partitioned",
        make: || {
            pg_text(
                &[
                    (K::Table, &["shop", "events_2026"]),
                    (K::Table, &["shop", "events"]),
                ],
                no_grants(),
            )
        },
    },
    Case {
        rel: "pg/partition_inherited_skipped",
        make: || pg_text(&[(K::Table, &["shop", "events_2026"])], no_grants()),
    },
    Case {
        rel: "pg/identity_vs_serial",
        make: || {
            pg_text(
                &[
                    (K::Table, &["shop", "tickets"]),
                    (K::Sequence, &["shop", "invoice_no"]),
                    (K::Table, &["shop", "orders"]),
                ],
                no_grants(),
            )
        },
    },
    Case {
        rel: "pg/views_or_replace",
        make: || {
            pg_text(
                &[
                    (K::MaterializedView, &["shop", "order_totals"]),
                    (K::View, &["shop", "active_orders"]),
                ],
                GenOptions {
                    or_replace: true,
                    ..no_grants()
                },
            )
        },
    },
    Case {
        rel: "pg/indexes_server_text",
        make: || {
            pg_text(
                &[
                    (K::Index, &["shop", "orders", "orders_lower_note_idx"]),
                    (K::Index, &["shop", "orders", "orders_open_idx"]),
                    (K::Index, &["shop", "orders", "orders_code_hash"]),
                ],
                no_grants(),
            )
        },
    },
    Case {
        rel: "pg/types",
        make: || {
            pg_text(
                &[
                    (K::CompositeType, &["shop", "address"]),
                    (K::Domain, &["shop", "posint"]),
                    (K::RangeType, &["shop", "floatrange"]),
                    (K::EnumType, &["shop", "mood"]),
                ],
                no_grants(),
            )
        },
    },
    Case {
        rel: "pg/function_and_trigger",
        make: || {
            let fx = pg();
            let keys = [
                fx.key(K::Table, &["shop", "items"]),
                fx.routine(K::Routine, &["shop", "touch"], ""),
                fx.routine(K::Routine, &["shop", "add_tax"], "numeric, numeric"),
            ];
            fx.text(
                &keys,
                &pg_target(),
                &GenOptions {
                    or_replace: true,
                    ..no_grants()
                },
            )
        },
    },
    Case {
        rel: "pg/turkish_mixed_case",
        make: || {
            pg_text(
                &[(K::Table, &["shop", "MÜŞTERİ"])],
                GenOptions {
                    surrogate_names: true,
                    ..no_grants()
                },
            )
        },
    },
    Case {
        rel: "pg/schema_qualify_always",
        make: || {
            pg_text(
                &[(K::Table, &["shop", "orders"])],
                GenOptions {
                    qualify: Qualify::Always,
                    skip_owner: true,
                    ..no_grants()
                },
            )
        },
    },
    Case {
        rel: "pg/row_security",
        make: || pg_text(&[(K::Table, &["shop", "secrets"])], no_grants()),
    },
    Case {
        rel: "mysql/table_full",
        make: || {
            my_text(
                DbmsKind::MySql,
                &[(K::Table, &["shop", "orders"])],
                opts(),
                DelimStyle::Semicolon,
            )
        },
    },
    Case {
        rel: "mysql/fk_after_same_named_index",
        make: || {
            my_text(
                DbmsKind::MySql,
                &[(K::Table, &["shop", "lines"])],
                no_grants(),
                DelimStyle::Semicolon,
            )
        },
    },
    Case {
        rel: "mysql/view_routine_definer",
        make: || {
            my_text(
                DbmsKind::MySql,
                &[
                    (K::View, &["shop", "v_open"]),
                    (K::Procedure, &["shop", "p_close"]),
                    (K::Routine, &["shop", "f_tax"]),
                ],
                no_grants(),
                DelimStyle::Semicolon,
            )
        },
    },
    Case {
        rel: "mysql/view_routine_skip_definer",
        make: || {
            my_text(
                DbmsKind::MySql,
                &[
                    (K::View, &["shop", "v_open"]),
                    (K::Procedure, &["shop", "p_close"]),
                    (K::Routine, &["shop", "f_tax"]),
                ],
                GenOptions {
                    skip_definer: true,
                    or_replace: true,
                    ..no_grants()
                },
                DelimStyle::Semicolon,
            )
        },
    },
    Case {
        rel: "mysql/view_routine_file",
        make: || {
            my_text(
                DbmsKind::MySql,
                &[
                    (K::View, &["shop", "v_open"]),
                    (K::Procedure, &["shop", "p_close"]),
                    (K::Routine, &["shop", "f_tax"]),
                ],
                no_grants(),
                DelimStyle::MySqlDelimiter,
            )
        },
    },
    Case {
        rel: "mysql/executable_comment",
        make: || {
            my_text(
                DbmsKind::MySql,
                &[(K::Table, &["shop", "audit"])],
                no_grants(),
                DelimStyle::Semicolon,
            )
        },
    },
    Case {
        rel: "mysql/server_definition",
        make: || {
            my_text(
                DbmsKind::MySql,
                &[(K::Table, &["shop", "orders"])],
                GenOptions {
                    server_definitions: true,
                    ..no_grants()
                },
                DelimStyle::Semicolon,
            )
        },
    },
    Case {
        rel: "mariadb/table",
        make: || {
            my_text(
                DbmsKind::MariaDb,
                &[(K::Table, &["shop", "orders"])],
                no_grants(),
                DelimStyle::Semicolon,
            )
        },
    },
    Case {
        rel: "mariadb/sequence_or_replace",
        make: || {
            my_text(
                DbmsKind::MariaDb,
                &[
                    (K::Sequence, &["shop", "seq_invoice"]),
                    (K::View, &["shop", "v_open"]),
                    (K::Procedure, &["shop", "p_close"]),
                ],
                GenOptions {
                    skip_definer: true,
                    or_replace: true,
                    ..no_grants()
                },
                DelimStyle::MySqlDelimiter,
            )
        },
    },
    Case {
        rel: "sqlite/server_text",
        make: || {
            sqlite_text(
                &[
                    (K::View, &["main", "v_notes"]),
                    (K::Table, &["main", "notes"]),
                    (K::Table, &["main", "authors"]),
                ],
                opts(),
            )
        },
    },
    Case {
        rel: "sqlite/schema_expanded",
        make: || sqlite_text(&[(K::Schema, &["main"])], opts()),
    },
];

/// The text of golden case `rel`.
pub fn case_text(rel: &str) -> String {
    let case = CASES
        .iter()
        .find(|c| c.rel == rel)
        .unwrap_or_else(|| panic!("no case {rel}"));
    (case.make)()
}
