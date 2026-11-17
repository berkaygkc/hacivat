//! Fixture catalogs for the producer and golden tests: one PostgreSQL, one MySQL/MariaDB and
//! one SQLite catalog built with the `hacivat-meta` builders, each object shaped the way the
//! introspector contract in `hacivat_ddl::produce::Sources` describes, plus an in-memory
//! source store and grants facet.
//!
//! Every value here is written by hand from vendor documentation of what the server prints
//! (`format_type`, `pg_get_viewdef`, `SHOW CREATE …`, `sqlite_schema.sql`); none is copied
//! from another tool's output.

#![allow(dead_code)]

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::Arc;

use hacivat_ddl::{
    DdlTarget, DelimStyle, GenOptions, Grant, GrantsFacet, Script, Sources, create_script,
};
use hacivat_meta::props::*;
use hacivat_meta::{
    Catalog, CatalogChange, CatalogSnapshot, FoldPolicy, Level, Name, NamespaceSnapshot, NodeId,
    NsKey, NsMeta, Obj, ObjFlags, ObjectKey, ObjectKind, Props, Ref, SourceRef, Subtree,
};
use hacivat_proto::DbmsKind;
use smallvec::SmallVec;
use sql_dialect::{DialectId, ServerVersion};

pub fn n(s: &str) -> Name {
    Name::from(s)
}

pub fn names(v: &[&str]) -> Vec<Name> {
    v.iter().map(|s| n(s)).collect()
}

/// Source texts and grants, keyed by content hash and object path.
#[derive(Default, Clone)]
pub struct MemSources {
    texts: HashMap<u128, Name>,
    grants: BTreeMap<(ObjectKind, Vec<Name>), GrantsFacet>,
    pub default_grants: GrantsFacet,
}

impl MemSources {
    pub fn add(&mut self, text: &str) -> SourceRef {
        let r = SourceRef::of(text);
        self.texts.insert(r.hash, n(text));
        r
    }

    pub fn set_grants(&mut self, kind: ObjectKind, path: &[&str], g: GrantsFacet) {
        self.grants.insert((kind, names(path)), g);
    }
}

impl Sources for MemSources {
    fn source(&self, r: &SourceRef) -> Option<Name> {
        self.texts.get(&r.hash).cloned()
    }

    fn grants(&self, key: &ObjectKey) -> GrantsFacet {
        self.grants
            .get(&(key.kind, key.path.to_vec()))
            .cloned()
            .unwrap_or_else(|| self.default_grants.clone())
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

/// A fixture catalog.
pub struct Fx {
    pub snap: Arc<CatalogSnapshot>,
    pub sources: MemSources,
    /// The path prefix of schema namespaces (`["app"]` on PostgreSQL, `[]` elsewhere).
    pub prefix: Vec<&'static str>,
}

impl Fx {
    pub fn key(&self, kind: ObjectKind, path: &[&str]) -> ObjectKey {
        let mut p: SmallVec<[Name; 4]> = self.prefix.iter().map(|s| n(s)).collect();
        p.extend(path.iter().map(|s| n(s)));
        ObjectKey {
            kind,
            path: p,
            server_id: None,
            overload_sig: None,
        }
    }

    pub fn routine(&self, kind: ObjectKind, path: &[&str], sig: &str) -> ObjectKey {
        let mut k = self.key(kind, path);
        k.overload_sig = Some(n(sig));
        k
    }

    pub fn script(&self, keys: &[ObjectKey], target: &DdlTarget, opts: &GenOptions) -> Script {
        create_script(&self.snap, &self.sources, keys, target, opts).expect("script")
    }

    pub fn text(&self, keys: &[ObjectKey], target: &DdlTarget, opts: &GenOptions) -> String {
        self.script(keys, target, opts).text(DelimStyle::Semicolon)
    }
}

// ---- object builders ---------------------------------------------------------------------

pub fn cp(ty: &str) -> ColumnProps {
    ColumnProps {
        type_text: n(ty),
        nullable: true,
        ..Default::default()
    }
}

pub fn column(name: &str, p: ColumnProps) -> Obj {
    Obj::new(ObjectKind::Column, name).with_props(Props::Column(Arc::new(p)))
}

pub fn key(kind: ObjectKind, name: &str, cols: &[&str], server_named: bool) -> Obj {
    let o = Obj::new(kind, name).with_props(Props::Key(Arc::new(KeyProps {
        columns: names(cols),
        ..Default::default()
    })));
    if server_named {
        o.with_flags(ObjFlags::SERVER_GENERATED_NAME)
    } else {
        o
    }
}

pub fn target(kind: ObjectKind, path: &[&str]) -> Ref {
    Ref {
        kind,
        path: path.iter().map(|s| n(s)).collect(),
        server_id: None,
    }
}

/// Foreign-key props (`_name` documents the constraint the props belong to).
pub fn fk(_name: &str, cols: &[&str], to: Ref, to_cols: &[&str]) -> ForeignKeyProps {
    ForeignKeyProps {
        columns: names(cols),
        target: to,
        target_columns: names(to_cols),
        on_update: FkAction::NoAction,
        on_delete: FkAction::NoAction,
        match_kind: FkMatch::Simple,
        deferrable: Deferrable::No,
    }
}

pub fn fk_obj(name: &str, p: ForeignKeyProps) -> Obj {
    Obj::new(ObjectKind::ForeignKey, name).with_props(Props::ForeignKey(Arc::new(p)))
}

pub fn check(kind: ObjectKind, name: &str, expr: &str) -> Obj {
    Obj::new(kind, name).with_props(Props::Check(Arc::new(CheckProps { expr: n(expr) })))
}

pub fn part(col: &str) -> IndexPart {
    IndexPart {
        column: Some(n(col)),
        ..Default::default()
    }
}

pub fn expr_part(e: &str) -> IndexPart {
    IndexPart {
        expr: Some(n(e)),
        ..Default::default()
    }
}

pub fn index(name: &str, p: IndexProps) -> Obj {
    Obj::new(ObjectKind::Index, name).with_props(Props::Index(Arc::new(p)))
}

pub fn trigger(name: &str, src: SourceRef) -> Obj {
    Obj::new(ObjectKind::Trigger, name).with_props(Props::Trigger(Arc::new(TriggerProps {
        source: Some(src),
        enabled: true,
        ..Default::default()
    })))
}

pub fn table(name: &str, p: TableProps) -> Subtree {
    Subtree::new(Obj::new(ObjectKind::Table, name).with_props(Props::Table(Arc::new(p))))
}

pub fn add(t: &mut Subtree, o: Obj) -> NodeId {
    t.add(t.root(), o)
}

fn namespace(dbms: DbmsKind, key: NsKey, root: Obj, children: Vec<Subtree>) -> NamespaceSnapshot {
    let mut t = Subtree::new(root);
    for c in children {
        t.graft_under(t.root(), c);
    }
    NamespaceSnapshot::build(
        key,
        dbms,
        FoldPolicy::for_dbms(dbms),
        NsMeta {
            level: Level::Sources,
            ..Default::default()
        },
        t,
    )
    .expect("fixture namespace")
}

fn catalog(dbms: DbmsKind, ns: Vec<NamespaceSnapshot>) -> Arc<CatalogSnapshot> {
    let c = Catalog::new(dbms);
    c.publish(ns.into_iter().map(CatalogChange::Put))
        .expect("publish");
    c.current()
}

fn leaf(o: Obj) -> Subtree {
    Subtree::new(o)
}

// ---- PostgreSQL ---------------------------------------------------------------------------

pub fn pg_target() -> DdlTarget {
    DdlTarget::new(DialectId::Postgres)
        .with_version(ServerVersion::new(17, 0, 0))
        .with_current_schema("shop")
}

fn pg_table_props(owner: Option<&str>, comment: Option<&str>) -> TableProps {
    TableProps {
        owner: owner.map(n),
        comment: comment.map(n),
        ext: TableExt::Pg(PgTableExt {
            relkind: b'r',
            persistence: b'p',
            ..Default::default()
        }),
        ..Default::default()
    }
}

fn pg_seq(name: &str, p: SequenceProps) -> Subtree {
    leaf(Obj::new(ObjectKind::Sequence, name).with_props(Props::Sequence(Arc::new(p))))
}

/// The PostgreSQL catalog: database `app`, schemas `shop`, `sales` and `tiny`.
pub fn pg() -> Fx {
    let mut src = MemSources::default();
    let mut shop = Vec::new();

    // shop.customers (also in sales: an ambiguous name).
    let mut customers = table("customers", pg_table_props(None, None));
    add(
        &mut customers,
        column(
            "id",
            ColumnProps {
                nullable: false,
                ..cp("bigint")
            },
        ),
    );
    add(
        &mut customers,
        key(ObjectKind::PrimaryKey, "customers_pkey", &["id"], true),
    );
    shop.push(customers);

    // shop.orders: the full table.
    let mut orders = table(
        "orders",
        pg_table_props(Some("app_owner"), Some("Customer orders")),
    );
    add(
        &mut orders,
        column(
            "id",
            ColumnProps {
                nullable: false,
                identity: Identity::Always,
                ..cp("bigint")
            },
        ),
    );
    add(
        &mut orders,
        column(
            "customer_id",
            ColumnProps {
                nullable: false,
                ..cp("bigint")
            },
        ),
    );
    add(
        &mut orders,
        column(
            "code",
            ColumnProps {
                nullable: false,
                collation: Some(n("C")),
                ..cp("character varying(32)")
            },
        ),
    );
    add(
        &mut orders,
        column(
            "status",
            ColumnProps {
                nullable: false,
                default_expr: Some(n("'new'::text")),
                ..cp("text")
            },
        ),
    );
    add(
        &mut orders,
        column(
            "amount",
            ColumnProps {
                nullable: false,
                default_expr: Some(n("0")),
                ..cp("numeric(12,2)")
            },
        ),
    );
    add(
        &mut orders,
        column(
            "total",
            ColumnProps {
                generated: Generated::Stored,
                generation_expr: Some(n("(amount * 1.2)")),
                ..cp("numeric")
            },
        ),
    );
    add(
        &mut orders,
        column(
            "note",
            ColumnProps {
                comment: Some(n("PM's approver of record")),
                ..cp("text")
            },
        ),
    );
    add(
        &mut orders,
        column(
            "created_at",
            ColumnProps {
                nullable: false,
                default_expr: Some(n("now()")),
                ..cp("timestamp with time zone")
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
            ForeignKeyProps {
                on_delete: FkAction::Cascade,
                ..fk(
                    "orders_customer_fk",
                    &["customer_id"],
                    target(ObjectKind::Table, &["app", "shop", "customers"]),
                    &["id"],
                )
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
        .with_flags(ObjFlags::SERVER_GENERATED_NAME),
    );
    // The indexes backing the keys are in the model too; they are never re-created.
    add(
        &mut orders,
        index(
            "orders_pkey",
            IndexProps {
                unique: true,
                primary: true,
                parts: vec![part("id")],
                ..Default::default()
            },
        ),
    );
    add(
        &mut orders,
        index(
            "orders_code_uq",
            IndexProps {
                unique: true,
                parts: vec![part("code")],
                ..Default::default()
            },
        ),
    );
    add(
        &mut orders,
        index(
            "orders_status_idx",
            IndexProps {
                method: Some(n("btree")),
                parts: vec![part("status")],
                ..Default::default()
            },
        ),
    );
    add(
        &mut orders,
        index(
            "orders_open_idx",
            IndexProps {
                method: Some(n("btree")),
                parts: vec![IndexPart {
                    desc: true,
                    nulls_first: Some(false),
                    ..part("created_at")
                }],
                include: names(&["id"]),
                predicate: Some(n("(status <> 'done'::text)")),
                comment: Some(n("Open orders, newest first")),
                ..Default::default()
            },
        ),
    );
    add(
        &mut orders,
        index(
            "orders_lower_note_idx",
            IndexProps {
                method: Some(n("btree")),
                parts: vec![expr_part("lower(note)")],
                ..Default::default()
            },
        ),
    );
    add(
        &mut orders,
        index(
            "orders_code_hash",
            IndexProps {
                method: Some(n("hash")),
                parts: vec![part("code")],
                ..Default::default()
            },
        ),
    );
    let audit = src.add(
        "CREATE TRIGGER orders_audit AFTER INSERT OR UPDATE ON shop.orders FOR EACH ROW EXECUTE FUNCTION shop.audit_row()",
    );
    add(&mut orders, trigger("orders_audit", audit));
    shop.push(orders);
    // The identity column's own sequence is in the model; it is never emitted for the table.
    shop.push(pg_seq(
        "orders_id_seq",
        SequenceProps {
            owned_by: Some((
                target(ObjectKind::Table, &["app", "shop", "orders"]),
                n("id"),
            )),
            ..Default::default()
        },
    ));

    // shop.empty: zero columns.
    shop.push(table("empty", pg_table_props(None, None)));

    // shop.events, partitioned, and one partition.
    let mut events = table(
        "events",
        TableProps {
            partition_key: Some(n("RANGE (created_at)")),
            ext: TableExt::Pg(PgTableExt {
                relkind: b'p',
                persistence: b'p',
                ..Default::default()
            }),
            ..Default::default()
        },
    );
    add(
        &mut events,
        column(
            "id",
            ColumnProps {
                nullable: false,
                ..cp("bigint")
            },
        ),
    );
    add(
        &mut events,
        column(
            "created_at",
            ColumnProps {
                nullable: false,
                ..cp("timestamp with time zone")
            },
        ),
    );
    add(&mut events, column("payload", cp("jsonb")));
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
            partition_of: Some(target(ObjectKind::Table, &["app", "shop", "events"])),
            partition_bound: Some(n(
                "FOR VALUES FROM ('2026-01-01 00:00:00+00') TO ('2027-01-01 00:00:00+00')",
            )),
            ext: TableExt::Pg(PgTableExt {
                relkind: b'r',
                persistence: b'p',
                options: names(&["fillfactor=90"]),
                tablespace: Some(n("fast_ssd")),
                ..Default::default()
            }),
            ..Default::default()
        },
    );
    for (c, ty) in [
        ("id", "bigint"),
        ("created_at", "timestamp with time zone"),
        ("payload", "jsonb"),
    ] {
        add(
            &mut ev26,
            column(
                c,
                ColumnProps {
                    nullable: c == "payload",
                    ..cp(ty)
                },
            )
            .with_flags(ObjFlags::INHERITED),
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
        .with_flags(ObjFlags::INHERITED),
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

    // shop.tickets with a serial-style sequence; shop.invoice_no standalone.
    let mut tickets = table("tickets", pg_table_props(None, None));
    add(
        &mut tickets,
        column(
            "id",
            ColumnProps {
                nullable: false,
                default_expr: Some(n("nextval('shop.tickets_id_seq'::regclass)")),
                ..cp("integer")
            },
        ),
    );
    add(&mut tickets, column("title", cp("text")));
    add(
        &mut tickets,
        key(ObjectKind::PrimaryKey, "tickets_pkey", &["id"], true),
    );
    shop.push(tickets);
    shop.push(pg_seq(
        "tickets_id_seq",
        SequenceProps {
            data_type: Some(n("integer")),
            start: Some(1),
            increment: Some(1),
            min: Some(1),
            max: Some(2_147_483_647),
            cache: Some(1),
            owned_by: Some((
                target(ObjectKind::Table, &["app", "shop", "tickets"]),
                n("id"),
            )),
            ..Default::default()
        },
    ));
    shop.push(pg_seq(
        "invoice_no",
        SequenceProps {
            data_type: Some(n("bigint")),
            start: Some(1000),
            increment: Some(10),
            min: Some(1),
            max: Some(i64::MAX),
            cache: Some(20),
            cycle: true,
            comment: Some(n("Invoice numbers")),
            ..Default::default()
        },
    ));

    // Views.
    let v = src.add(
        " SELECT o.id,\n    o.status\n   FROM shop.orders o\n  WHERE (o.status <> 'done'::text);",
    );
    let mut active = Subtree::new(Obj::new(ObjectKind::View, "active_orders").with_props(
        Props::View(Arc::new(ViewProps {
            owner: Some(n("app_owner")),
            comment: Some(n("Orders not done yet")),
            source: Some(v),
            check_option: Some(n("LOCAL")),
            ..Default::default()
        })),
    ));
    add(
        &mut active,
        column(
            "id",
            ColumnProps {
                comment: Some(n("Order id")),
                ..cp("bigint")
            },
        ),
    );
    add(&mut active, column("status", cp("text")));
    shop.push(active);
    let mv = src.add(
        " SELECT orders.customer_id,\n    sum(orders.amount) AS total\n   FROM shop.orders\n  GROUP BY orders.customer_id;",
    );
    let mut totals = Subtree::new(
        Obj::new(ObjectKind::MaterializedView, "order_totals").with_props(Props::View(Arc::new(
            ViewProps {
                source: Some(mv),
                ..Default::default()
            },
        ))),
    );
    add(&mut totals, column("customer_id", cp("bigint")));
    add(&mut totals, column("total", cp("numeric")));
    add(
        &mut totals,
        index(
            "order_totals_customer_idx",
            IndexProps {
                unique: true,
                parts: vec![part("customer_id")],
                ..Default::default()
            },
        ),
    );
    shop.push(totals);

    // Types.
    let ty = |kind, name: &str, p: TypeProps| {
        Subtree::new(Obj::new(kind, name).with_props(Props::Type(Arc::new(p))))
    };
    shop.push(ty(
        ObjectKind::EnumType,
        "mood",
        TypeProps {
            labels: names(&["sad", "ok", "it's fine"]),
            comment: Some(n("How it went")),
            ..Default::default()
        },
    ));
    shop.push(ty(
        ObjectKind::Domain,
        "posint",
        TypeProps {
            base: Some(n("integer")),
            not_null: true,
            default_expr: Some(n("1")),
            ..Default::default()
        },
    ));
    let mut address = ty(ObjectKind::CompositeType, "address", TypeProps::default());
    add(&mut address, column("street", cp("text")));
    add(
        &mut address,
        column(
            "city",
            ColumnProps {
                collation: Some(n("C")),
                ..cp("text")
            },
        ),
    );
    shop.push(address);
    shop.push(ty(
        ObjectKind::RangeType,
        "floatrange",
        TypeProps {
            base: Some(n("double precision")),
            ..Default::default()
        },
    ));

    // A trigger function and a table that uses it.
    let f = src.add(
        "CREATE OR REPLACE FUNCTION shop.touch()\n RETURNS trigger\n LANGUAGE plpgsql\nAS $function$\nBEGIN\n  NEW.updated_at := now();\n  RETURN NEW;\nEND;\n$function$\n",
    );
    shop.push(leaf(Obj::new(ObjectKind::Routine, "touch").with_props(
        Props::Routine(Arc::new(RoutineProps {
            kind: RoutineKind::Function,
            returns: Some(n("trigger")),
            language: Some(n("plpgsql")),
            overload_sig: Some(n("")),
            source: Some(f),
            comment: Some(n("Sets updated_at")),
            ..Default::default()
        })),
    )));
    let add_fn = src.add(
        "CREATE OR REPLACE FUNCTION shop.add_tax(amount numeric, rate numeric DEFAULT 0.18)\n RETURNS numeric\n LANGUAGE sql\n IMMUTABLE\nBEGIN ATOMIC\n SELECT (add_tax.amount * ((1)::numeric + add_tax.rate));\nEND\n",
    );
    shop.push(leaf(Obj::new(ObjectKind::Routine, "add_tax").with_props(
        Props::Routine(Arc::new(RoutineProps {
            kind: RoutineKind::Function,
            args: vec![
                Arg {
                    name: Some(n("amount")),
                    type_text: n("numeric"),
                    ..Default::default()
                },
                Arg {
                    name: Some(n("rate")),
                    type_text: n("numeric"),
                    default_expr: Some(n("0.18")),
                    ..Default::default()
                },
            ],
            returns: Some(n("numeric")),
            overload_sig: Some(n("numeric, numeric")),
            source: Some(add_fn),
            ..Default::default()
        })),
    )));
    let mut items = table("items", pg_table_props(None, None));
    add(
        &mut items,
        column(
            "id",
            ColumnProps {
                nullable: false,
                ..cp("integer")
            },
        ),
    );
    add(
        &mut items,
        column("updated_at", cp("timestamp with time zone")),
    );
    let tt = src.add(
        "CREATE TRIGGER items_touch BEFORE UPDATE ON shop.items FOR EACH ROW EXECUTE FUNCTION shop.touch()",
    );
    add(&mut items, trigger("items_touch", tt));
    shop.push(items);

    // Turkish, mixed-case and reserved identifiers.
    let mut tr = table(
        "MÜŞTERİ",
        pg_table_props(None, Some("Müşteri kayıtları — İstanbul şubesi")),
    );
    add(
        &mut tr,
        column(
            "Ad",
            ColumnProps {
                nullable: false,
                ..cp("text")
            },
        ),
    );
    add(&mut tr, column("order", cp("integer")));
    add(&mut tr, column("müşteri_no", cp("bigint")));
    add(
        &mut tr,
        column(
            "ılık",
            ColumnProps {
                comment: Some(n("Sıcaklık \"derece\" -- yorum değil\nikinci satır")),
                ..cp("boolean")
            },
        ),
    );
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

    // A foreign-key cycle and an unvalidated check.
    let mut a = table("a", pg_table_props(None, None));
    add(
        &mut a,
        column(
            "id",
            ColumnProps {
                nullable: false,
                ..cp("integer")
            },
        ),
    );
    add(&mut a, column("b_id", cp("integer")));
    add(&mut a, key(ObjectKind::PrimaryKey, "a_pkey", &["id"], true));
    add(
        &mut a,
        fk_obj(
            "a_b_fk",
            ForeignKeyProps {
                deferrable: Deferrable::InitiallyDeferred,
                ..fk(
                    "a_b_fk",
                    &["b_id"],
                    target(ObjectKind::Table, &["app", "shop", "b"]),
                    &["id"],
                )
            },
        ),
    );
    add(
        &mut a,
        check(ObjectKind::Check, "a_b_id_check", "(b_id > 0)").with_flags(ObjFlags::INVALID),
    );
    shop.push(a);
    let mut b = table("b", pg_table_props(None, None));
    add(
        &mut b,
        column(
            "id",
            ColumnProps {
                nullable: false,
                ..cp("integer")
            },
        ),
    );
    add(&mut b, column("a_id", cp("integer")));
    add(&mut b, key(ObjectKind::PrimaryKey, "b_pkey", &["id"], true));
    add(
        &mut b,
        fk_obj(
            "b_a_fk",
            fk(
                "b_a_fk",
                &["a_id"],
                target(ObjectKind::Table, &["app", "shop", "a"]),
                &["id"],
            ),
        ),
    );
    shop.push(b);

    // Inside-column placement candidates.
    let mut ic = table("inline_keys", pg_table_props(None, None));
    add(
        &mut ic,
        column(
            "id",
            ColumnProps {
                nullable: false,
                ..cp("integer")
            },
        ),
    );
    add(&mut ic, column("code", cp("text")));
    add(&mut ic, column("customer_id", cp("bigint")));
    add(&mut ic, column("lo", cp("integer")));
    add(&mut ic, column("hi", cp("integer")));
    add(&mut ic, key(ObjectKind::PrimaryKey, "pk", &["id"], false));
    add(
        &mut ic,
        key(
            ObjectKind::UniqueKey,
            "inline_keys_code_key",
            &["code"],
            true,
        ),
    );
    add(
        &mut ic,
        fk_obj(
            "inline_keys_customer_id_fkey",
            ForeignKeyProps {
                on_update: FkAction::Restrict,
                on_delete: FkAction::SetNull,
                match_kind: FkMatch::Full,
                ..fk(
                    "inline_keys_customer_id_fkey",
                    &["customer_id"],
                    target(ObjectKind::Table, &["app", "sales", "customers"]),
                    &["id"],
                )
            },
        )
        .with_flags(ObjFlags::SERVER_GENERATED_NAME),
    );
    add(
        &mut ic,
        key(ObjectKind::UniqueKey, "lo_hi", &["lo", "hi"], false),
    );
    add(&mut ic, check(ObjectKind::Check, "lo_le_hi", "(lo <= hi)"));
    add(
        &mut ic,
        check(
            ObjectKind::Exclusion,
            "no_overlap",
            "USING gist (int4range(lo, hi) WITH &&)",
        ),
    );
    shop.push(ic);

    // An unlogged table with storage clauses.
    let mut logs = table(
        "logs",
        TableProps {
            ext: TableExt::Pg(PgTableExt {
                relkind: b'r',
                persistence: b'u',
                access_method: Some(n("heap")),
                options: names(&["fillfactor=70", "autovacuum_enabled=false"]),
                ..Default::default()
            }),
            ..Default::default()
        },
    );
    add(&mut logs, column("line", cp("text")));
    // Row-level policies have no producer yet: left out with a notice.
    add(&mut logs, Obj::new(ObjectKind::Policy, "logs_owner_only"));
    shop.push(logs);
    shop.push(leaf(Obj::new(ObjectKind::Aggregate, "median").with_props(
        Props::Routine(Arc::new(RoutineProps {
            kind: RoutineKind::Aggregate,
            overload_sig: Some(n("numeric")),
            ..Default::default()
        })),
    )));

    let shop_ns = namespace(
        DbmsKind::Postgres,
        NsKey::schema(["app", "shop"]),
        Obj::new(ObjectKind::Schema, "shop").with_props(Props::Schema(Arc::new(SchemaProps {
            owner: Some(n("app_owner")),
            comment: Some(n("The shop")),
        }))),
        shop,
    );

    // sales: a second `customers` (makes the name ambiguous under Auto).
    let mut sc = table("customers", pg_table_props(None, None));
    add(
        &mut sc,
        column(
            "id",
            ColumnProps {
                nullable: false,
                ..cp("bigint")
            },
        ),
    );
    add(
        &mut sc,
        key(ObjectKind::PrimaryKey, "customers_pkey", &["id"], true),
    );
    let sales_ns = namespace(
        DbmsKind::Postgres,
        NsKey::schema(["app", "sales"]),
        Obj::new(ObjectKind::Schema, "sales"),
        vec![sc],
    );

    // tiny: a whole schema to expand.
    let mut tiny = Vec::new();
    let mut parent = table("parent", pg_table_props(Some("tiny_owner"), None));
    add(
        &mut parent,
        column(
            "id",
            ColumnProps {
                nullable: false,
                ..cp("integer")
            },
        ),
    );
    add(
        &mut parent,
        key(ObjectKind::PrimaryKey, "parent_pkey", &["id"], true),
    );
    let mut child = table("child", pg_table_props(None, None));
    add(
        &mut child,
        column(
            "id",
            ColumnProps {
                nullable: false,
                ..cp("integer")
            },
        ),
    );
    add(
        &mut child,
        column(
            "parent_id",
            ColumnProps {
                nullable: false,
                ..cp("integer")
            },
        ),
    );
    add(
        &mut child,
        key(ObjectKind::PrimaryKey, "child_pkey", &["id"], true),
    );
    add(
        &mut child,
        fk_obj(
            "child_parent_id_fkey",
            fk(
                "child_parent_id_fkey",
                &["parent_id"],
                target(ObjectKind::Table, &["app", "tiny", "parent"]),
                &["id"],
            ),
        )
        .with_flags(ObjFlags::SERVER_GENERATED_NAME),
    );
    // Insertion order puts the child first; the producer still creates the parent first.
    tiny.push(child);
    tiny.push(parent);
    let tiny_ns = namespace(
        DbmsKind::Postgres,
        NsKey::schema(["app", "tiny"]),
        Obj::new(ObjectKind::Schema, "tiny").with_props(Props::Schema(Arc::new(SchemaProps {
            owner: Some(n("tiny_owner")),
            comment: None,
        }))),
        tiny,
    );

    src.set_grants(
        ObjectKind::Table,
        &["app", "shop", "orders"],
        GrantsFacet::Loaded(vec![
            grant("app_reader", "SELECT", false),
            grant("app_writer", "UPDATE", true),
            grant("app_writer", "INSERT", true),
            grant("PUBLIC", "SELECT", false),
            Grant {
                columns: names(&["status", "note"]),
                ..grant("support", "UPDATE", false)
            },
        ]),
    );
    src.default_grants = GrantsFacet::NotLoaded;

    Fx {
        snap: catalog(DbmsKind::Postgres, vec![shop_ns, sales_ns, tiny_ns]),
        sources: src,
        prefix: vec!["app"],
    }
}

// ---- MySQL / MariaDB ---------------------------------------------------------------------

pub fn mysql_target(d: DialectId) -> DdlTarget {
    let v = if d == DialectId::MariaDb {
        ServerVersion::new(11, 4, 0)
    } else {
        ServerVersion::new(8, 4, 0)
    };
    DdlTarget::new(d)
        .with_version(v)
        .with_current_schema("shop")
}

fn my_col(ty: &str, column_type: &str, extra: Option<&str>) -> ColumnProps {
    ColumnProps {
        ext: ColumnExt::MySql(MySqlColumnExt {
            column_type: Some(n(column_type)),
            extra: extra.map(n),
            charset: None,
        }),
        ..cp(ty)
    }
}

/// The MySQL (or MariaDB) catalog: schemas `shop` and `crm`.
pub fn mysql(dbms: DbmsKind) -> Fx {
    let mut src = MemSources::default();
    let mut shop = Vec::new();

    let mut customers = table("customers", TableProps::default());
    add(
        &mut customers,
        column(
            "id",
            ColumnProps {
                nullable: false,
                ..my_col("bigint", "bigint unsigned", None)
            },
        ),
    );
    add(
        &mut customers,
        key(ObjectKind::PrimaryKey, "PRIMARY", &["id"], true),
    );
    shop.push(customers);

    let mut orders = table(
        "orders",
        TableProps {
            comment: Some(n("Orders")),
            ext: TableExt::MySql(MySqlTableExt {
                engine: Some(n("InnoDB")),
                collation: Some(n("utf8mb4_0900_ai_ci")),
                auto_increment: Some(1042),
                ..Default::default()
            }),
            ..Default::default()
        },
    );
    add(
        &mut orders,
        column(
            "id",
            ColumnProps {
                nullable: false,
                identity: Identity::AutoIncrement,
                ..my_col("bigint", "bigint unsigned", Some("auto_increment"))
            },
        ),
    );
    add(
        &mut orders,
        column(
            "customer_id",
            ColumnProps {
                nullable: false,
                ..my_col("bigint", "bigint unsigned", None)
            },
        ),
    );
    add(
        &mut orders,
        column(
            "code",
            ColumnProps {
                nullable: false,
                collation: Some(n("utf8mb4_bin")),
                ..my_col("varchar", "varchar(32)", None)
            },
        ),
    );
    add(
        &mut orders,
        column(
            "status",
            ColumnProps {
                nullable: false,
                default_expr: Some(n("'new'")),
                ..my_col("enum", "enum('new','paid','it''s done')", None)
            },
        ),
    );
    add(
        &mut orders,
        column(
            "note",
            ColumnProps {
                comment: Some(n("it's \"quoted\" and C:\\path")),
                ..my_col("varchar", "varchar(200)", None)
            },
        ),
    );
    add(
        &mut orders,
        column("price", my_col("decimal", "decimal(10,2)", None)),
    );
    add(
        &mut orders,
        column(
            "total",
            ColumnProps {
                generated: Generated::Virtual,
                generation_expr: Some(n("(`price` * 1.2)")),
                ..my_col("decimal", "decimal(12,2)", Some("VIRTUAL GENERATED"))
            },
        ),
    );
    add(
        &mut orders,
        column(
            "created_at",
            ColumnProps {
                nullable: false,
                default_expr: Some(n("CURRENT_TIMESTAMP")),
                ..my_col(
                    "timestamp",
                    "timestamp",
                    Some("DEFAULT_GENERATED on update CURRENT_TIMESTAMP"),
                )
            },
        ),
    );
    add(
        &mut orders,
        column("closed_at", my_col("timestamp", "timestamp", None)),
    );
    add(
        &mut orders,
        key(ObjectKind::PrimaryKey, "PRIMARY", &["id"], true),
    );
    add(
        &mut orders,
        key(ObjectKind::UniqueKey, "code_uq", &["code"], false),
    );
    add(
        &mut orders,
        fk_obj(
            "fk_customer",
            ForeignKeyProps {
                on_delete: FkAction::Cascade,
                ..fk(
                    "fk_customer",
                    &["customer_id"],
                    target(ObjectKind::Table, &["crm", "customers"]),
                    &["id"],
                )
            },
        ),
    );
    add(
        &mut orders,
        check(ObjectKind::Check, "chk_price", "(`price` >= 0)"),
    );
    // Indexes as `information_schema.STATISTICS` lists them: the primary and unique key
    // indexes, the foreign key's own index (named after the constraint), and secondary ones.
    add(
        &mut orders,
        index(
            "PRIMARY",
            IndexProps {
                unique: true,
                primary: true,
                method: Some(n("BTREE")),
                parts: vec![part("id")],
                ..Default::default()
            },
        ),
    );
    add(
        &mut orders,
        index(
            "code_uq",
            IndexProps {
                unique: true,
                method: Some(n("BTREE")),
                parts: vec![part("code")],
                ..Default::default()
            },
        ),
    );
    add(
        &mut orders,
        index(
            "fk_customer",
            IndexProps {
                method: Some(n("BTREE")),
                parts: vec![part("customer_id")],
                ..Default::default()
            },
        ),
    );
    add(
        &mut orders,
        index(
            "idx_status",
            IndexProps {
                method: Some(n("BTREE")),
                parts: vec![part("status")],
                comment: Some(n("for the board")),
                ..Default::default()
            },
        ),
    );
    add(
        &mut orders,
        index(
            "idx_note",
            IndexProps {
                method: Some(n("BTREE")),
                parts: vec![
                    IndexPart {
                        prefix_len: Some(10),
                        ..part("note")
                    },
                    IndexPart {
                        desc: true,
                        ..part("created_at")
                    },
                ],
                ..Default::default()
            },
        ),
    );
    add(
        &mut orders,
        index(
            "idx_lower_code",
            IndexProps {
                method: Some(n("BTREE")),
                parts: vec![expr_part("lower(`code`)")],
                ..Default::default()
            },
        ),
    );
    add(
        &mut orders,
        index(
            "ft_note",
            IndexProps {
                method: Some(n("FULLTEXT")),
                parts: vec![part("note")],
                ..Default::default()
            },
        ),
    );
    add(
        &mut orders,
        index(
            "idx_code_hash",
            IndexProps {
                method: Some(n("HASH")),
                parts: vec![part("code"), part("status")],
                ..Default::default()
            },
        ),
    );
    let trg = src.add(
        "CREATE DEFINER=`root`@`%` TRIGGER `orders_bi` BEFORE INSERT ON `orders` FOR EACH ROW BEGIN\n  IF NEW.price < 0 THEN\n    SET NEW.price = 0;\n  END IF;\nEND",
    );
    add(&mut orders, trigger("orders_bi", trg));
    shop.push(orders);

    // Views and routines with definers, and one with an executable comment.
    let view = |name: &str, r: SourceRef| {
        Subtree::new(
            Obj::new(ObjectKind::View, name).with_props(Props::View(Arc::new(ViewProps {
                source: Some(r),
                ext: ViewExt::MySql(MySqlViewExt {
                    algorithm: Some(n("UNDEFINED")),
                    definer: Some(n("root@%")),
                    sql_security: Some(n("DEFINER")),
                }),
                ..Default::default()
            }))),
        )
    };
    let v = src.add(
        "CREATE ALGORITHM=UNDEFINED DEFINER=`root`@`%` SQL SECURITY DEFINER VIEW `v_open` AS select `orders`.`id` AS `id`,`orders`.`status` AS `status` from `orders` where (`orders`.`status` <> 'paid')",
    );
    shop.push(view("v_open", v));
    let vx = src.add(
        "CREATE ALGORITHM=UNDEFINED DEFINER=`root`@`%` SQL SECURITY DEFINER VIEW `v_hint` AS select /*!80000 SQL_NO_CACHE */ `orders`.`id` AS `id` from `orders`",
    );
    shop.push(view("v_hint", vx));
    // A head this build does not recognise (an executable comment before the object word).
    let vl = src.add(
        "CREATE /*!50001 ALGORITHM=UNDEFINED */ /*!50013 DEFINER=`root`@`%` SQL SECURITY DEFINER */ VIEW `v_legacy` AS select 1 AS `one`",
    );
    shop.push(view("v_legacy", vl));
    let p = src.add(
        "CREATE DEFINER=`root`@`%` PROCEDURE `close_order`(IN p_id BIGINT)\n    COMMENT 'Closes one order'\nBEGIN\n  UPDATE orders SET status = 'paid' WHERE id = p_id;\n  SELECT ROW_COUNT();\nEND",
    );
    shop.push(leaf(
        Obj::new(ObjectKind::Procedure, "close_order").with_props(Props::Routine(Arc::new(
            RoutineProps {
                kind: RoutineKind::Procedure,
                args: vec![Arg {
                    name: Some(n("p_id")),
                    type_text: n("bigint"),
                    ..Default::default()
                }],
                source: Some(p),
                ..Default::default()
            },
        ))),
    ));
    let f = src.add(
        "CREATE DEFINER=`app`@`localhost` FUNCTION `tax`(amount DECIMAL(10,2)) RETURNS decimal(10,2)\n    DETERMINISTIC\nRETURN amount * 0.18",
    );
    shop.push(leaf(Obj::new(ObjectKind::Routine, "tax").with_props(
        Props::Routine(Arc::new(RoutineProps {
            kind: RoutineKind::Function,
            args: vec![Arg {
                name: Some(n("amount")),
                type_text: n("decimal(10,2)"),
                ..Default::default()
            }],
            returns: Some(n("decimal(10,2)")),
            source: Some(f),
            ..Default::default()
        })),
    )));

    // Reserved, Turkish and mixed-case names.
    let mut ord = table("order", TableProps::default());
    add(
        &mut ord,
        column(
            "Müşteri",
            ColumnProps {
                nullable: false,
                ..my_col("varchar", "varchar(64)", None)
            },
        ),
    );
    add(&mut ord, column("select", my_col("int", "int", None)));
    add(&mut ord, column("CamelCase", my_col("int", "int", None)));
    add(&mut ord, column("1st", my_col("int", "int", None)));
    add(&mut ord, column("status", my_col("int", "int", None)));
    add(
        &mut ord,
        key(ObjectKind::PrimaryKey, "PRIMARY", &["Müşteri"], true),
    );
    shop.push(ord);

    if dbms == DbmsKind::MariaDb {
        shop.push(leaf(
            Obj::new(ObjectKind::Sequence, "invoice_seq").with_props(Props::Sequence(Arc::new(
                SequenceProps {
                    start: Some(1000),
                    increment: Some(1),
                    min: Some(1),
                    max: Some(i64::MAX - 1),
                    cache: Some(1000),
                    cycle: false,
                    ..Default::default()
                },
            ))),
        ));
    }

    let shop_ns = namespace(
        dbms,
        NsKey::schema(["shop"]),
        Obj::new(ObjectKind::Schema, "shop"),
        shop,
    );
    let mut crm_c = table("customers", TableProps::default());
    add(
        &mut crm_c,
        column(
            "id",
            ColumnProps {
                nullable: false,
                ..my_col("bigint", "bigint unsigned", None)
            },
        ),
    );
    add(
        &mut crm_c,
        key(ObjectKind::PrimaryKey, "PRIMARY", &["id"], true),
    );
    let crm_ns = namespace(
        dbms,
        NsKey::schema(["crm"]),
        Obj::new(ObjectKind::Schema, "crm"),
        vec![crm_c],
    );

    src.set_grants(
        ObjectKind::Table,
        &["shop", "orders"],
        GrantsFacet::Partial(vec![
            grant("'app'@'%'", "SELECT", false),
            grant("'app'@'%'", "INSERT", false),
            grant("'o''brien'@'localhost'", "SELECT", true),
        ]),
    );

    Fx {
        snap: catalog(dbms, vec![shop_ns, crm_ns]),
        sources: src,
        prefix: vec![],
    }
}

// ---- SQLite --------------------------------------------------------------------------------

pub fn sqlite_target() -> DdlTarget {
    DdlTarget::new(DialectId::Sqlite)
}

fn sq_table(name: &str, strict: bool, without_rowid: bool) -> Subtree {
    table(
        name,
        TableProps {
            comment: Some(n("never written on SQLite")),
            ext: TableExt::Sqlite(SqliteTableExt {
                without_rowid,
                strict,
                virtual_module: None,
            }),
            ..Default::default()
        },
    )
}

/// The SQLite catalog: schemas `main` and the attached `aux`.
pub fn sqlite() -> Fx {
    let mut src = MemSources::default();
    let mut main = Vec::new();

    let mut authors = sq_table("authors", false, false);
    add(
        &mut authors,
        column(
            "id",
            ColumnProps {
                nullable: true,
                ..cp("INTEGER")
            },
        ),
    );
    add(
        &mut authors,
        column(
            "name",
            ColumnProps {
                nullable: false,
                collation: Some(n("NOCASE")),
                ..cp("TEXT")
            },
        ),
    );
    add(
        &mut authors,
        key(ObjectKind::PrimaryKey, "pk_authors", &["id"], true),
    );
    main.push(authors);

    let mut notes = sq_table("notes", false, false);
    add(
        &mut notes,
        column(
            "id",
            ColumnProps {
                identity: Identity::AutoIncrement,
                ..cp("INTEGER")
            },
        ),
    );
    add(
        &mut notes,
        column(
            "body",
            ColumnProps {
                nullable: false,
                comment: Some(n("never written")),
                ..cp("TEXT")
            },
        ),
    );
    add(&mut notes, column("author_id", cp("INTEGER")));
    add(
        &mut notes,
        column(
            "created_at",
            ColumnProps {
                default_expr: Some(n("CURRENT_TIMESTAMP")),
                ..cp("TEXT")
            },
        ),
    );
    add(
        &mut notes,
        column(
            "weight",
            ColumnProps {
                default_expr: Some(n("1.5")),
                ..cp("REAL")
            },
        ),
    );
    add(
        &mut notes,
        key(ObjectKind::PrimaryKey, "pk_notes", &["id"], true),
    );
    add(
        &mut notes,
        key(
            ObjectKind::UniqueKey,
            "uq_notes",
            &["body", "author_id"],
            true,
        ),
    );
    add(
        &mut notes,
        fk_obj(
            "fk_notes_0",
            ForeignKeyProps {
                on_delete: FkAction::SetNull,
                ..fk(
                    "fk_notes_0",
                    &["author_id"],
                    target(ObjectKind::Table, &["main", "authors"]),
                    &["id"],
                )
            },
        )
        .with_flags(ObjFlags::SERVER_GENERATED_NAME),
    );
    add(
        &mut notes,
        check(ObjectKind::Check, "body_not_empty", "length(body) > 0"),
    );
    add(
        &mut notes,
        index(
            "sqlite_autoindex_notes_1",
            IndexProps {
                unique: true,
                parts: vec![part("body"), part("author_id")],
                ..Default::default()
            },
        ),
    );
    add(
        &mut notes,
        index(
            "notes_author",
            IndexProps {
                parts: vec![
                    part("author_id"),
                    IndexPart {
                        desc: true,
                        ..part("created_at")
                    },
                ],
                predicate: Some(n("author_id IS NOT NULL")),
                comment: Some(n("never written")),
                ..Default::default()
            },
        ),
    );
    add(
        &mut notes,
        index(
            "notes_lower_body",
            IndexProps {
                unique: false,
                parts: vec![expr_part("lower(body)")],
                ..Default::default()
            },
        ),
    );
    let trg = src.add(
        "CREATE TRIGGER notes_touch AFTER UPDATE ON notes BEGIN\n  UPDATE notes SET created_at = CURRENT_TIMESTAMP WHERE id = NEW.id;\nEND",
    );
    add(&mut notes, trigger("notes_touch", trg));
    main.push(notes);

    let v = src.add("CREATE VIEW v_notes AS SELECT n.id, a.name FROM notes n JOIN authors a ON a.id = n.author_id");
    main.push(Subtree::new(
        Obj::new(ObjectKind::View, "v_notes").with_props(Props::View(Arc::new(ViewProps {
            source: Some(v),
            ..Default::default()
        }))),
    ));

    let mut prices = sq_table("prices", true, true);
    add(
        &mut prices,
        column(
            "sku",
            ColumnProps {
                nullable: false,
                ..cp("TEXT")
            },
        ),
    );
    add(
        &mut prices,
        column(
            "qty",
            ColumnProps {
                nullable: false,
                ..cp("INTEGER")
            },
        ),
    );
    add(
        &mut prices,
        column(
            "price",
            ColumnProps {
                nullable: false,
                ..cp("REAL")
            },
        ),
    );
    add(
        &mut prices,
        column(
            "total",
            ColumnProps {
                generated: Generated::Stored,
                generation_expr: Some(n("qty * price")),
                ext: ColumnExt::Sqlite(SqliteColumnExt { hidden: 3 }),
                ..cp("REAL")
            },
        ),
    );
    add(
        &mut prices,
        column(
            "label",
            ColumnProps {
                generated: Generated::Virtual,
                generation_expr: Some(n("upper(sku)")),
                ext: ColumnExt::Sqlite(SqliteColumnExt { hidden: 2 }),
                ..cp("TEXT")
            },
        ),
    );
    add(
        &mut prices,
        column(
            "meta",
            ColumnProps {
                nullable: true,
                ..cp("ANY")
            },
        ),
    );
    add(
        &mut prices,
        key(ObjectKind::PrimaryKey, "pk_prices", &["sku"], true),
    );
    main.push(prices);

    // A cycle (SQLite keeps both foreign keys inside the tables).
    let mut a = sq_table("a", false, false);
    add(
        &mut a,
        column(
            "id",
            ColumnProps {
                nullable: false,
                ..cp("INTEGER")
            },
        ),
    );
    add(&mut a, column("b_id", cp("INTEGER")));
    add(&mut a, key(ObjectKind::PrimaryKey, "pk_a", &["id"], true));
    add(
        &mut a,
        fk_obj(
            "fk_a_0",
            ForeignKeyProps {
                deferrable: Deferrable::InitiallyDeferred,
                ..fk(
                    "fk_a_0",
                    &["b_id"],
                    target(ObjectKind::Table, &["main", "b"]),
                    &["id"],
                )
            },
        )
        .with_flags(ObjFlags::SERVER_GENERATED_NAME),
    );
    main.push(a);
    let mut b = sq_table("b", false, false);
    add(
        &mut b,
        column(
            "id",
            ColumnProps {
                nullable: false,
                ..cp("INTEGER")
            },
        ),
    );
    add(&mut b, column("a_id", cp("INTEGER")));
    add(&mut b, key(ObjectKind::PrimaryKey, "pk_b", &["id"], true));
    add(
        &mut b,
        fk_obj(
            "fk_b_0",
            fk(
                "fk_b_0",
                &["a_id"],
                target(ObjectKind::Table, &["main", "a"]),
                &["id"],
            ),
        )
        .with_flags(ObjFlags::SERVER_GENERATED_NAME),
    );
    main.push(b);

    let main_ns = namespace(
        DbmsKind::Sqlite,
        NsKey::schema(["main"]),
        Obj::new(ObjectKind::Schema, "main"),
        main,
    );

    // aux: an attached schema.
    let mut kv = sq_table("kv", false, false);
    add(
        &mut kv,
        column(
            "k",
            ColumnProps {
                nullable: false,
                ..cp("TEXT")
            },
        ),
    );
    add(&mut kv, column("v", cp("BLOB")));
    add(&mut kv, key(ObjectKind::PrimaryKey, "pk_kv", &["k"], true));
    add(
        &mut kv,
        index(
            "kv_v",
            IndexProps {
                parts: vec![part("v")],
                ..Default::default()
            },
        ),
    );
    let kt = src.add(
        "CREATE TRIGGER aux.kv_guard BEFORE DELETE ON kv BEGIN\n  SELECT RAISE(ABORT, 'read-only');\nEND",
    );
    add(&mut kv, trigger("kv_guard", kt));
    let aux_ns = namespace(
        DbmsKind::Sqlite,
        NsKey::schema(["aux"]),
        Obj::new(ObjectKind::Schema, "aux"),
        vec![kv],
    );

    Fx {
        snap: catalog(DbmsKind::Sqlite, vec![main_ns, aux_ns]),
        sources: src,
        prefix: vec![],
    }
}

// ---- goldens -------------------------------------------------------------------------------

/// The golden file `tests/golden/<rel>.sql`.
pub fn golden_path(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("golden")
        .join(format!("{rel}.sql"))
}

/// Compares `actual` with the golden file byte for byte; `UPDATE_GOLDEN=1` writes it instead
/// (R-M4-16). Every blessed file is reviewed by hand.
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
