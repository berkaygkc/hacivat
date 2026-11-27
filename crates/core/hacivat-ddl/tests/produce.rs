//! DDL producers (plan-m4 Task 19; alg/03 HV-DDL-07 edge cases): refusals, notices, naming
//! and ordering rules that a golden alone does not pin.

mod fixtures;

use std::sync::Arc;

use fixtures::*;
use hacivat_ddl::produce::{
    GRANTS_NOT_LOADED, GRANTS_PARTIAL, UNSUPPORTED_KIND, WARN_NOT_ENFORCED, WARN_UNKNOWN_TYPE,
    create_script, create_script_with,
};
use hacivat_ddl::render::RenderError;
use hacivat_ddl::script::{DelimStyle, Notice};
use hacivat_ddl::{DdlError, DdlTarget, GenOptions};
use hacivat_meta::ids::ObjectKey;
use hacivat_meta::namespace::{Level, Obj, ObjFlags, Subtree, Watermark};
use hacivat_meta::overlay::{Overlays, VirtualFk};
use hacivat_meta::props::{
    CheckProps, ColumnProps, IndexProps, KeyProps, Props, TableProps, TypeBody, TypeProps, TypeRef,
    ViewProps,
};
use hacivat_meta::snapshot::LoadState;
use hacivat_proto::DbmsKind;
use sql_dialect::{DialectId, ServerVersion};
use sql_ident::quote::QuoteError;

fn is_nul(e: &DdlError) -> bool {
    matches!(
        e,
        DdlError::Quote(QuoteError::Nul) | DdlError::Render(RenderError::Quote(QuoteError::Nul))
    )
}

/// A one-namespace PostgreSQL catalog `app.shop` holding `children`, in `state`.
fn tiny_pg(children: Vec<Subtree>, state: LoadState) -> Fx {
    let ns = namespace(DbmsKind::Postgres, &[DB, "shop"], Props::None, children);
    Fx {
        view: view_at(DbmsKind::Postgres, vec![ns], Overlays::default(), state),
        src: MemSources::default(),
        dbms: DbmsKind::Postgres,
    }
}

#[test]
fn comments_with_quote_dashes_newline_are_escaped() {
    let text = case_text("pg/turkish_mixed_case");
    assert!(
        text.contains("COMMENT ON TABLE \"MÜŞTERİ\" IS 'Müşteri''s table -- don''t\nedit';\n"),
        "{text}"
    );
    let my = case_text("mysql/table_full");
    assert!(
        my.contains("COMMENT 'Approver''s path: C:\\\\orders'"),
        "{my}"
    );
}

#[test]
fn unnamed_constraints_get_no_synthetic_names() {
    let fx = pg();
    let key = [fx.key(K::Table, &["shop", "orders"])];
    let text = fx.text(&key, &pg_target(), &no_grants());
    assert!(!text.contains("orders_pkey") && !text.contains("orders_amount_check"));
    assert!(
        text.contains("CONSTRAINT orders_code_uq UNIQUE (code)"),
        "a user name stays"
    );
    let named = fx.text(
        &key,
        &pg_target(),
        &GenOptions {
            surrogate_names: true,
            ..no_grants()
        },
    );
    assert!(named.contains("CONSTRAINT orders_pkey PRIMARY KEY (id)"));
    let my = case_text("mysql/table_full");
    assert!(!my.contains("orders_chk_1") && !my.contains("PRIMARY`"));
}

fn typed(ty: &str, schema: &str, name: &str) -> ColumnProps {
    ColumnProps {
        type_ref: Some(TypeRef {
            schema: Some(n(schema)),
            name: n(name),
        }),
        ..col("", ty, true)
    }
}

#[test]
fn unknown_type_emitted_as_is_with_a_warning() {
    // A type the loaded catalog does not hold (an extension's) is the server's spelling,
    // verbatim, with `ddl-warn-unknown-type`; built-in and known user types raise nothing.
    let mut t = table("tags", TableProps::default());
    add(
        &mut t,
        column("label", typed("public.citext", "public", "citext")),
    );
    add(&mut t, column("n", typed("integer", "pg_catalog", "int4")));
    add(&mut t, column("m", typed("mood", "shop", "mood")));
    let mood = leaf(
        Obj::new(K::EnumType, "mood").with_props(Props::Type(Arc::new(TypeProps {
            body: TypeBody::Enum {
                labels: vec![n("a")].into(),
            },
            ..TypeProps::default()
        }))),
    );
    let fx = tiny_pg(vec![t, mood], loaded());
    let s = fx.script(
        &[fx.key(K::Table, &["shop", "tags"])],
        &pg_target(),
        &no_grants(),
    );
    assert_eq!(
        s.text(DelimStyle::Semicolon),
        "CREATE TABLE tags (\n    label public.citext,\n    n integer,\n    m mood\n);\n"
    );
    assert_eq!(
        s.notices,
        vec![Notice {
            id: WARN_UNKNOWN_TYPE,
            args: vec![("type", "public.citext".to_string())],
        }]
    );
}

#[test]
fn constraints_default_to_enforced() {
    assert!(KeyProps::default().enforced && CheckProps::default().enforced);
    assert!(IndexProps::default().key.enforced);
}

#[test]
fn not_enforced_before_pg18_is_left_out_with_a_notice() {
    let fx = pg();
    let k = [fx.key(K::Table, &["shop", "loose"])];
    let s = fx.script(&k, &pg_target(), &no_grants());
    let text = s.text(DelimStyle::Semicolon);
    assert_eq!(text, "CREATE TABLE loose (\n    customer_id bigint\n);\n");
    let ids: Vec<_> = s
        .notices
        .iter()
        .filter(|n| n.id == WARN_NOT_ENFORCED)
        .flat_map(|n| n.args.iter().map(|(_, v)| v.clone()))
        .collect();
    assert_eq!(ids, ["loose_customer_check", "loose_customer_fk"]);
    // A selected constraint alone follows the same rule.
    let c = fx.key(K::Check, &["shop", "loose", "loose_customer_check"]);
    let s = fx.script(std::slice::from_ref(&c), &pg_target(), &opts());
    assert!(s.stmts.is_empty() && s.has_notice(WARN_NOT_ENFORCED));
    let mut t18 = pg_target();
    t18.version = ServerVersion::new(18, 0, 0);
    assert_eq!(
        fx.text(&[c], &t18, &opts()),
        "ALTER TABLE loose ADD CONSTRAINT loose_customer_check CHECK ((customer_id > 0)) NOT ENFORCED;\n"
    );
    // MySQL before 8.0.16 has no NOT ENFORCED either.
    let my = mysql(DbmsKind::MySql);
    let mut old = mysql_target(DialectId::MySql);
    old.version = ServerVersion::new(8, 0, 15);
    let s = my.script(&[my.key(K::Table, &["shop", "loose"])], &old, &no_grants());
    assert!(s.has_notice(WARN_NOT_ENFORCED));
    assert!(!s.text(DelimStyle::Semicolon).contains("loose_chk_1"));
}

#[test]
fn a_table_without_props_or_a_partition_key_is_not_loaded() {
    let none = leaf(Obj::new(K::Table, "bare"));
    let parted = table(
        "parted",
        TableProps {
            partitioned: true,
            ..TableProps::default()
        },
    );
    let fx = tiny_pg(vec![none, parted], loaded());
    for t in ["bare", "parted"] {
        assert_eq!(
            fx.try_script(
                &[fx.key(K::Table, &["shop", t])],
                &pg_target(),
                &no_grants()
            ),
            Err(DdlError::NotLoaded),
            "{t}"
        );
    }
}

#[test]
fn unknown_kind_is_refused() {
    let fx = pg();
    let k = ObjectKey::unknown(
        [DB, "shop", "future_thing"].iter().map(|s| n(s)).collect(),
        None,
        Some(n("relkind:z")),
    );
    assert_eq!(
        fx.try_script(&[k], &pg_target(), &opts()),
        Err(DdlError::UnsupportedKind)
    );
    // Inside a selected schema it is left out, with a notice.
    let s = fx.script(&[fx.key(K::Schema, &["shop"])], &pg_target(), &no_grants());
    assert!(
        s.notices
            .iter()
            .any(|n| n.id == UNSUPPORTED_KIND && n.args.contains(&("kind", "unknown".to_string())))
    );
}

#[test]
fn virtual_fk_is_never_emitted() {
    let base = pg();
    let from = base.key(K::Table, &["shop", "orders"]);
    let to = base.key(K::Table, &["shop", "items"]);
    let fx = pg_with(Overlays {
        virtual_fks: vec![VirtualFk {
            from: from.clone(),
            from_cols: vec!["customer_id".into()],
            to,
            to_cols: vec!["id".into()],
            name: Some("virt_orders_items".into()),
        }],
        ..Overlays::default()
    });
    let text = fx.text(&[from], &pg_target(), &no_grants());
    assert!(!text.contains("virt_orders_items") && !text.contains("REFERENCES items"));
    assert_eq!(
        text,
        base.text(
            &[base.key(K::Table, &["shop", "orders"])],
            &pg_target(),
            &no_grants()
        )
    );
}

#[test]
fn producer_without_grants_facet_emits_notice_not_silence() {
    let fx = pg();
    let customers = [fx.key(K::Table, &["shop", "customers"])];
    let s = fx.script(&customers, &pg_target(), &opts());
    assert!(s.has_notice(GRANTS_NOT_LOADED));
    assert!(!s.text(DelimStyle::Semicolon).contains("GRANT"));
    let s = fx.script(&customers, &pg_target(), &no_grants());
    assert!(s.notices.is_empty(), "skipped grants need no notice");
    // The seam entry point has no facets at all.
    let s = create_script(&fx.view, &customers, &pg_target(), &opts()).expect("script");
    assert!(s.has_notice(GRANTS_NOT_LOADED));
    // Loaded: the grants, no notice; partial: the grants read plus the notice.
    let orders = fx.script(
        &[fx.key(K::Table, &["shop", "orders"])],
        &pg_target(),
        &opts(),
    );
    assert!(!orders.has_notice(GRANTS_NOT_LOADED) && !orders.has_notice(GRANTS_PARTIAL));
    let my = mysql(DbmsKind::MySql);
    let s = my.script(
        &[my.key(K::Table, &["shop", "orders"])],
        &mysql_target(DialectId::MySql),
        &opts(),
    );
    assert!(s.has_notice(GRANTS_PARTIAL));
    assert!(
        s.text(DelimStyle::Semicolon)
            .contains("GRANT INSERT, SELECT ON orders TO 'app'@'%'")
    );
}

#[test]
fn a_view_without_its_source_is_not_loaded_never_empty() {
    let fx = pg();
    let v = [fx.key(K::View, &["shop", "active_orders"])];
    assert!(fx.try_script(&v, &pg_target(), &opts()).is_ok());
    assert_eq!(
        create_script(&fx.view, &v, &pg_target(), &opts()),
        Err(DdlError::NotLoaded)
    );
}

#[test]
fn a_namespace_below_structure_is_not_loaded() {
    let mut t = table("t", TableProps::default());
    add(&mut t, column("a", col("", "integer", true)));
    let names_only = LoadState::Loaded {
        level: Level::Names,
        at: std::time::SystemTime::UNIX_EPOCH,
        watermark: Watermark::None,
        stale: false,
    };
    let fx = tiny_pg(vec![t], names_only);
    assert_eq!(
        fx.try_script(&[fx.key(K::Table, &["shop", "t"])], &pg_target(), &opts()),
        Err(DdlError::NotLoaded),
        "never `CREATE TABLE t ()` from a Names-level load"
    );
    let fx = pg();
    assert_eq!(
        fx.try_script(
            &[fx.key(K::Table, &["shop", "nope"])],
            &pg_target(),
            &opts()
        ),
        Err(DdlError::NotLoaded)
    );
}

#[test]
fn a_lossy_name_is_refused() {
    let t = leaf(
        Obj::from_server_bytes(K::Table, b"bad\xff")
            .with_props(Props::Table(Arc::new(TableProps::default()))),
    );
    let fx = tiny_pg(vec![t], loaded());
    let k = fx.key(K::Table, &["shop", "bad\u{FFFD}"]);
    assert_eq!(
        fx.try_script(&[k], &pg_target(), &opts()),
        Err(DdlError::NotLoaded)
    );
}

#[test]
fn nul_is_refused_anywhere() {
    let t = table(
        "t",
        TableProps {
            comment: Some(n("a\0b")),
            ..TableProps::default()
        },
    );
    let mut fx = tiny_pg(vec![t], loaded());
    let e = fx
        .try_script(
            &[fx.key(K::Table, &["shop", "t"])],
            &pg_target(),
            &no_grants(),
        )
        .expect_err("a NUL in a comment");
    assert!(is_nul(&e), "{e:?}");
    // Server text and raw defaults are emitted verbatim, so a NUL there is refused as well.
    let mut d = table("d", TableProps::default());
    add(
        &mut d,
        column(
            "a",
            ColumnProps {
                default_raw: Some(n("'x\0'")),
                ..col("", "text", true)
            },
        ),
    );
    let src = fx.src.add("SELECT '\0' AS x");
    let v = leaf(
        Obj::new(K::View, "v").with_props(Props::View(Arc::new(ViewProps {
            source: Some(src),
            ..ViewProps::default()
        }))),
    );
    let mut fx2 = tiny_pg(vec![d, v], loaded());
    std::mem::swap(&mut fx2.src, &mut fx.src);
    for k in [
        fx2.key(K::Table, &["shop", "d"]),
        fx2.key(K::View, &["shop", "v"]),
    ] {
        let e = fx2
            .try_script(&[k], &pg_target(), &no_grants())
            .expect_err("a NUL in verbatim text");
        assert!(is_nul(&e), "{e:?}");
    }
}

#[test]
fn selection_order_does_not_change_the_output() {
    let fx = pg();
    let a = [
        fx.key(K::Table, &["shop", "orders"]),
        fx.key(K::Table, &["shop", "customers"]),
        fx.key(K::Sequence, &["shop", "invoice_no"]),
        fx.key(K::EnumType, &["shop", "mood"]),
    ];
    let mut b = a.clone();
    b.reverse();
    let ta = fx.text(&a, &pg_target(), &opts());
    assert_eq!(ta, fx.text(&b, &pg_target(), &opts()));
    let mut doubled = a.to_vec();
    doubled.extend(a.iter().cloned());
    assert_eq!(
        ta,
        fx.text(&doubled, &pg_target(), &opts()),
        "duplicates collapse"
    );
    // FK order: the referenced table first.
    assert!(
        ta.find("CREATE TABLE shop.customers").unwrap() < ta.find("CREATE TABLE orders").unwrap()
    );
}

#[test]
fn a_foreign_key_cycle_is_postponed_past_both_tables() {
    let mut a = table("a", TableProps::default());
    add(&mut a, column("id", col("", "integer", false)));
    add(&mut a, column("b_id", col("", "integer", true)));
    add(
        &mut a,
        fk_obj("a_b_fk", fk(&["b_id"], target(&[DB, "shop", "b"]), &["id"])),
    );
    let mut b = table("b", TableProps::default());
    add(&mut b, column("id", col("", "integer", false)));
    add(&mut b, column("a_id", col("", "integer", true)));
    add(
        &mut b,
        fk_obj("b_a_fk", fk(&["a_id"], target(&[DB, "shop", "a"]), &["id"])),
    );
    let fx = tiny_pg(vec![a, b], loaded());
    let text = fx.text(
        &[
            fx.key(K::Table, &["shop", "b"]),
            fx.key(K::Table, &["shop", "a"]),
        ],
        &pg_target(),
        &no_grants(),
    );
    assert_eq!(
        text,
        "CREATE TABLE a (\n    id integer NOT NULL,\n    b_id integer\n);\n\n\
         CREATE TABLE b (\n    id integer NOT NULL,\n    a_id integer,\n    CONSTRAINT b_a_fk FOREIGN KEY (a_id) REFERENCES a (id)\n);\n\n\
         ALTER TABLE a ADD CONSTRAINT a_b_fk FOREIGN KEY (b_id) REFERENCES b (id);\n"
    );
}

#[test]
fn an_expression_index_without_its_server_text_is_synthesized() {
    let fx = pg();
    let k = fx.key(K::Index, &["shop", "orders", "orders_lower_note_idx"]);
    let s = create_script(&fx.view, &[k], &pg_target(), &opts()).expect("script");
    assert_eq!(
        s.text(DelimStyle::Semicolon),
        "CREATE INDEX orders_lower_note_idx ON orders ((lower(note)));\n"
    );
}

#[test]
fn a_selected_constraint_alone_is_an_alter_table() {
    let fx = pg();
    let k = fx.key(K::ForeignKey, &["shop", "orders", "orders_customer_fk"]);
    assert_eq!(
        fx.text(&[k], &pg_target(), &opts()),
        "ALTER TABLE orders ADD CONSTRAINT orders_customer_fk FOREIGN KEY (customer_id) REFERENCES shop.customers (id) ON DELETE CASCADE;\n"
    );
    let sq = sqlite();
    let k = sq.key(
        K::ForeignKey,
        &["main", "notes", "notes_author_id_authors_fkey"],
    );
    let s = sq.script(&[k], &sqlite_target(), &opts());
    assert!(s.stmts.is_empty() && s.has_notice(UNSUPPORTED_KIND));
}

#[test]
fn dialects_without_a_renderer_are_refused() {
    let fx = pg();
    for d in [
        DialectId::SqlServer,
        DialectId::ClickHouse,
        DialectId::Oracle,
    ] {
        let t = DdlTarget::new(d, ServerVersion::new(1, 0, 0));
        assert_eq!(
            create_script_with(
                &fx.view,
                &fx.src,
                &[fx.key(K::Table, &["shop", "orders"])],
                &t,
                &opts()
            ),
            Err(DdlError::UnsupportedKind)
        );
    }
}

#[test]
fn temp_and_implicit_objects() {
    // A directly selected identity sequence is emitted; an implicit one is not otherwise.
    let fx = pg();
    let text = fx.text(
        &[fx.key(K::Sequence, &["shop", "orders_id_seq"])],
        &pg_target(),
        &no_grants(),
    );
    assert_eq!(text, "CREATE SEQUENCE orders_id_seq;\n");
    let temp = table(
        "scratch",
        TableProps {
            persistence: hacivat_meta::props::Persistence::Temporary,
            ..TableProps::default()
        },
    );
    let fx = tiny_pg(vec![temp], loaded());
    let text = fx.text(
        &[fx.key(K::Table, &["shop", "scratch"])],
        &pg_target(),
        &GenOptions {
            qualify: Qualify::Always,
            ..no_grants()
        },
    );
    assert_eq!(
        text, "CREATE TEMPORARY TABLE scratch ();\n",
        "never qualified"
    );
}

#[test]
fn system_objects_are_left_out_of_a_schema() {
    let mut shadow = Subtree::new(
        Obj::new(K::Table, "shadow")
            .with_props(Props::Table(Arc::new(TableProps::default())))
            .add_flags(ObjFlags::SYSTEM),
    );
    add(&mut shadow, column("x", col("", "integer", true)));
    let fx = tiny_pg(vec![shadow], loaded());
    let text = fx.text(&[fx.key(K::Schema, &["shop"])], &pg_target(), &no_grants());
    assert_eq!(text, "CREATE SCHEMA shop;\n");
}

#[test]
fn a_sqlite_virtual_table_is_its_server_text() {
    let vt = leaf(
        Obj::new(K::VirtualTable, "docs").with_props(Props::Table(Arc::new(TableProps {
            ext: hacivat_meta::props::TableExt::Sqlite {
                without_rowid: false,
                strict: false,
                module: Some(n("fts5")),
            },
            ..TableProps::default()
        }))),
    );
    let ns = namespace(DbmsKind::Sqlite, &["main"], Props::None, vec![vt]);
    let mut fx = Fx {
        view: view_of(DbmsKind::Sqlite, vec![ns], Overlays::default()),
        src: MemSources::default(),
        dbms: DbmsKind::Sqlite,
    };
    let k = fx.key(K::VirtualTable, &["main", "docs"]);
    fx.src.server(
        k.clone(),
        "CREATE VIRTUAL TABLE docs USING fts5(title, body)",
    );
    assert_eq!(
        fx.text(&[k], &sqlite_target(), &opts()),
        "CREATE VIRTUAL TABLE docs USING fts5(title, body);\n"
    );
}
