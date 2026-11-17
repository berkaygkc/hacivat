//! Producer behaviour (plan-m4 Task 19 tests that are not goldens): refusals, notices,
//! determinism, option effects.

mod fixtures;

use std::sync::Arc;

use fixtures::*;
use hacivat_ddl::{
    DdlError, DdlTarget, DelimStyle, GenOptions, GrantsFacet, NoSources, NoticeCode, Qualify,
    RenderError, create_script,
};
use hacivat_meta::props::*;
use hacivat_meta::{
    Catalog, CatalogChange, FoldPolicy, Level, NamespaceSnapshot, NsKey, NsMeta, Obj, ObjFlags,
    ObjectKind as K, Props, Subtree,
};
use hacivat_proto::DbmsKind;
use sql_dialect::DialectId;

fn has(s: &hacivat_ddl::Script, code: NoticeCode) -> bool {
    s.notices.iter().any(|n| n.code == code)
}

/// A one-table PostgreSQL catalog at `level` with `root` objects under schema `s`.
fn tiny_pg(level: Level, objs: Vec<Subtree>) -> Arc<hacivat_meta::CatalogSnapshot> {
    let mut t = Subtree::new(Obj::new(K::Schema, "s"));
    for o in objs {
        t.graft_under(t.root(), o);
    }
    let ns = NamespaceSnapshot::build(
        NsKey::schema(["db", "s"]),
        DbmsKind::Postgres,
        FoldPolicy::for_dbms(DbmsKind::Postgres),
        NsMeta {
            level,
            ..Default::default()
        },
        t,
    )
    .unwrap();
    let c = Catalog::new(DbmsKind::Postgres);
    c.publish([CatalogChange::Put(ns)]).unwrap();
    c.current()
}

fn key(path: &[&str], kind: K) -> hacivat_meta::ObjectKey {
    hacivat_meta::ObjectKey {
        kind,
        path: path.iter().map(|s| n(s)).collect(),
        server_id: None,
        overload_sig: None,
    }
}

#[test]
fn unknown_kind_is_refused() {
    let snap = tiny_pg(
        Level::Sources,
        vec![Subtree::new(Obj::new(K::Unknown, "x"))],
    );
    let r = create_script(
        &snap,
        &NoSources,
        &[key(&["db", "s", "x"], K::Unknown)],
        &pg_target(),
        &GenOptions::default(),
    );
    assert_eq!(
        r,
        Err(DdlError::Unsupported {
            kind: K::Unknown,
            dialect: DialectId::Postgres
        })
    );
}

#[test]
fn missing_object_is_not_found() {
    let snap = tiny_pg(Level::Sources, vec![]);
    let k = key(&["db", "s", "nope"], K::Table);
    let r = create_script(
        &snap,
        &NoSources,
        std::slice::from_ref(&k),
        &pg_target(),
        &GenOptions::default(),
    );
    assert_eq!(r, Err(DdlError::NotFound(Box::new(k))));
}

#[test]
fn lossy_name_is_refused() {
    let t = Subtree::new(
        Obj::new(K::Table, "bad\u{FFFD}name")
            .with_flags(ObjFlags::LOSSY_NAME)
            .with_props(Props::Table(Arc::new(TableProps::default()))),
    );
    let snap = tiny_pg(Level::Sources, vec![t]);
    let r = create_script(
        &snap,
        &NoSources,
        &[key(&["db", "s", "bad\u{FFFD}name"], K::Table)],
        &pg_target(),
        &GenOptions::default(),
    );
    assert!(matches!(r, Err(DdlError::LossyName(_))), "{r:?}");
}

#[test]
fn names_level_table_is_not_turned_into_an_empty_table() {
    // A namespace loaded at `Names` has no columns yet: no `CREATE TABLE t ()`.
    let t = Subtree::new(
        Obj::new(K::Table, "t").with_props(Props::Table(Arc::new(TableProps::default()))),
    );
    let snap = tiny_pg(Level::Names, vec![t]);
    let r = create_script(
        &snap,
        &NoSources,
        &[key(&["db", "s", "t"], K::Table)],
        &pg_target(),
        &GenOptions::default(),
    );
    assert!(matches!(r, Err(DdlError::SourceNotLoaded(_))), "{r:?}");
}

#[test]
fn view_without_its_source_is_an_error_not_an_empty_view() {
    let fx = pg();
    let r = create_script(
        &fx.snap,
        &NoSources,
        &[fx.key(K::View, &["shop", "active_orders"])],
        &pg_target(),
        &GenOptions::default(),
    );
    assert!(matches!(r, Err(DdlError::SourceNotLoaded(_))), "{r:?}");
}

#[test]
fn nul_in_comment_is_refused() {
    let t = Subtree::new(
        Obj::new(K::Table, "t").with_props(Props::Table(Arc::new(TableProps {
            comment: Some(n("a\0b")),
            ..Default::default()
        }))),
    );
    let snap = tiny_pg(Level::Sources, vec![t]);
    let r = create_script(
        &snap,
        &NoSources,
        &[key(&["db", "s", "t"], K::Table)],
        &pg_target(),
        &GenOptions {
            skip_grants: true,
            ..Default::default()
        },
    );
    assert_eq!(
        r,
        Err(DdlError::Render(RenderError::Nul { what: "comment" }))
    );
}

#[test]
fn producer_without_grants_facet_emits_notice_not_silence() {
    let fx = pg();
    let s = fx.script(
        &[fx.key(K::Table, &["shop", "tickets"])],
        &pg_target(),
        &GenOptions::default(),
    );
    assert!(has(&s, NoticeCode::GrantsNotLoaded));
    assert!(!s.text(DelimStyle::Semicolon).contains("GRANT"));
    let skipped = fx.script(
        &[fx.key(K::Table, &["shop", "tickets"])],
        &pg_target(),
        &GenOptions {
            skip_grants: true,
            ..Default::default()
        },
    );
    assert!(!has(&skipped, NoticeCode::GrantsNotLoaded));
}

#[test]
fn sqlite_never_asks_for_grants() {
    let fx = sqlite();
    let s = fx.script(
        &[fx.key(K::Table, &["main", "notes"])],
        &sqlite_target(),
        &GenOptions::default(),
    );
    assert!(!has(&s, NoticeCode::GrantsNotLoaded));
}

#[test]
fn grants_not_applicable_is_silent() {
    let mut fx = pg();
    fx.sources.default_grants = GrantsFacet::NotApplicable;
    let s = fx.script(
        &[fx.key(K::Table, &["shop", "tickets"])],
        &pg_target(),
        &GenOptions::default(),
    );
    assert!(s.notices.is_empty(), "{:?}", s.notices);
}

#[test]
fn unnamed_constraints_get_no_synthetic_names() {
    let fx = pg();
    let k = [fx.key(K::Table, &["tiny", "child"])];
    let opts = GenOptions {
        skip_grants: true,
        ..Default::default()
    };
    let text = fx.text(&k, &pg_target(), &opts);
    assert!(!text.contains("child_pkey"), "{text}");
    assert!(!text.contains("child_parent_id_fkey"), "{text}");
    let named = fx.text(
        &k,
        &pg_target(),
        &GenOptions {
            surrogate_names: true,
            ..opts
        },
    );
    assert!(
        named.contains("CONSTRAINT child_parent_id_fkey FOREIGN KEY"),
        "{named}"
    );
}

#[test]
fn selection_order_does_not_change_the_output() {
    let fx = pg();
    let a = [
        fx.key(K::Table, &["shop", "orders"]),
        fx.key(K::View, &["shop", "active_orders"]),
        fx.key(K::Table, &["shop", "a"]),
        fx.key(K::Table, &["shop", "b"]),
        fx.key(K::EnumType, &["shop", "mood"]),
    ];
    let mut b = a.clone();
    b.reverse();
    let opts = GenOptions::default();
    assert_eq!(
        fx.text(&a, &pg_target(), &opts),
        fx.text(&b, &pg_target(), &opts)
    );
}

#[test]
fn goldens_byte_stable_twice() {
    for _ in 0..2 {
        let fx = pg();
        let k = [fx.key(K::Schema, &["shop"])];
        let one = fx.text(&k, &pg_target(), &GenOptions::default());
        let two = fx.text(&k, &pg_target(), &GenOptions::default());
        assert_eq!(one, two);
    }
}

#[test]
fn or_replace_applies_to_pg_view_not_table() {
    let fx = pg();
    let text = fx.text(
        &[
            fx.key(K::Table, &["shop", "logs"]),
            fx.key(K::View, &["shop", "active_orders"]),
            fx.key(K::MaterializedView, &["shop", "order_totals"]),
        ],
        &pg_target(),
        &GenOptions {
            or_replace: true,
            skip_grants: true,
            ..Default::default()
        },
    );
    assert!(text.contains("CREATE UNLOGGED TABLE logs"), "{text}");
    assert!(
        text.contains("CREATE OR REPLACE VIEW active_orders"),
        "{text}"
    );
    assert!(
        text.contains("CREATE MATERIALIZED VIEW order_totals"),
        "{text}"
    );
}

#[test]
fn pg_trigger_or_replace_needs_14() {
    let fx = pg();
    let k = [fx.key(K::Table, &["shop", "items"])];
    let opts = GenOptions {
        or_replace: true,
        skip_grants: true,
        ..Default::default()
    };
    let old = pg_target().with_version(sql_dialect::ServerVersion::new(13, 9, 0));
    assert!(
        fx.text(&k, &old, &opts)
            .contains("\nCREATE TRIGGER items_touch")
    );
    assert!(
        fx.text(&k, &pg_target(), &opts)
            .contains("\nCREATE OR REPLACE TRIGGER items_touch")
    );
}

#[test]
fn mysql_skip_definer_drops_definer() {
    let fx = mysql(DbmsKind::MySql);
    let t = mysql_target(DialectId::MySql);
    let k = [fx.key(K::Table, &["shop", "orders"])];
    let opts = GenOptions {
        skip_definer: true,
        skip_grants: true,
        ..Default::default()
    };
    let text = fx.text(&k, &t, &opts);
    assert!(
        text.contains("CREATE TRIGGER `orders_bi` BEFORE INSERT"),
        "{text}"
    );
    assert!(!text.contains("DEFINER"), "{text}");
}

#[test]
fn unexpected_mysql_head_is_kept_with_a_notice() {
    let fx = mysql(DbmsKind::MySql);
    let t = mysql_target(DialectId::MySql);
    let s = fx.script(
        &[fx.key(K::View, &["shop", "v_legacy"])],
        &t,
        &GenOptions {
            skip_definer: true,
            skip_grants: true,
            ..Default::default()
        },
    );
    assert!(has(&s, NoticeCode::HeadNotEdited));
    assert!(has(&s, NoticeCode::WarnExecutableComment));
    assert!(
        s.text(DelimStyle::Semicolon)
            .contains("/*!50013 DEFINER=`root`@`%`")
    );
}

#[test]
fn unsupported_children_and_kinds_are_left_out_with_a_notice() {
    let fx = pg();
    let s = fx.script(
        &[
            fx.key(K::Table, &["shop", "logs"]),
            fx.routine(K::Aggregate, &["shop", "median"], "numeric"),
        ],
        &pg_target(),
        &GenOptions {
            skip_grants: true,
            ..Default::default()
        },
    );
    let kinds: Vec<_> = s
        .notices
        .iter()
        .filter(|n| n.code == NoticeCode::UnsupportedKind)
        .map(|n| n.kind)
        .collect();
    assert_eq!(kinds, vec![Some(K::Aggregate), Some(K::Policy)]);
    assert!(!s.text(DelimStyle::Semicolon).contains("median"));
}

#[test]
fn server_definitions_fall_back_with_a_notice() {
    let fx = mysql(DbmsKind::MySql);
    let s = fx.script(
        &[fx.key(K::Table, &["shop", "customers"])],
        &mysql_target(DialectId::MySql),
        &GenOptions {
            server_definitions: true,
            skip_grants: true,
            ..Default::default()
        },
    );
    assert!(has(&s, NoticeCode::ServerDefinitionUnavailable));
    assert!(
        s.text(DelimStyle::Semicolon)
            .starts_with("CREATE TABLE shop.customers (")
    );
}

#[test]
fn comments_with_quote_dashes_newline_are_escaped() {
    let fx = pg();
    let text = fx.text(
        &[fx.key(K::Table, &["shop", "MÜŞTERİ"])],
        &pg_target(),
        &GenOptions {
            skip_grants: true,
            ..Default::default()
        },
    );
    assert!(
        text.contains("IS 'Sıcaklık \"derece\" -- yorum değil\nikinci satır';"),
        "{text}"
    );
}

#[test]
fn pg_without_standard_strings_writes_an_e_literal_for_backslashes() {
    let t = Subtree::new(
        Obj::new(K::Table, "t").with_props(Props::Table(Arc::new(TableProps {
            comment: Some(n("C:\\tmp")),
            ..Default::default()
        }))),
    );
    let snap = tiny_pg(Level::Sources, vec![t]);
    let mut target = pg_target();
    target.pg_standard_conforming_strings = false;
    let s = create_script(
        &snap,
        &NoSources,
        &[key(&["db", "s", "t"], K::Table)],
        &target,
        &GenOptions {
            skip_grants: true,
            ..Default::default()
        },
    )
    .unwrap();
    assert!(
        s.text(DelimStyle::Semicolon)
            .contains("COMMENT ON TABLE s.t IS E'C:\\\\tmp';"),
        "{}",
        s.text(DelimStyle::Semicolon)
    );
}

#[test]
fn qualification_auto_follows_the_current_schema() {
    let fx = pg();
    let k = [fx.key(K::Table, &["tiny", "parent"])];
    let opts = GenOptions {
        skip_grants: true,
        skip_owner: true,
        ..Default::default()
    };
    let in_tiny = pg_target().with_current_schema("tiny");
    assert!(
        fx.text(&k, &in_tiny, &opts)
            .starts_with("CREATE TABLE parent (")
    );
    assert!(
        fx.text(&k, &pg_target(), &opts)
            .starts_with("CREATE TABLE tiny.parent (")
    );
    let no_current = DdlTarget::new(DialectId::Postgres);
    assert!(
        fx.text(&k, &no_current, &opts)
            .starts_with("CREATE TABLE tiny.parent (")
    );
    let never = GenOptions {
        qualify: Qualify::Never,
        ..opts
    };
    assert!(
        fx.text(&k, &pg_target(), &never)
            .starts_with("CREATE TABLE parent (")
    );
}

#[test]
fn dialect_without_a_renderer_is_refused() {
    let fx = pg();
    let r = create_script(
        &fx.snap,
        &fx.sources,
        &[fx.key(K::Table, &["shop", "empty"])],
        &DdlTarget::new(DialectId::SqlServer),
        &GenOptions::default(),
    );
    assert!(matches!(r, Err(DdlError::Unsupported { .. })), "{r:?}");
}

#[test]
fn a_selected_index_trigger_or_constraint_alone() {
    let fx = pg();
    let text = fx.text(
        &[
            fx.key(K::Index, &["shop", "orders", "orders_status_idx"]),
            fx.key(K::ForeignKey, &["shop", "orders", "orders_customer_fk"]),
            fx.key(K::Trigger, &["shop", "items", "items_touch"]),
        ],
        &pg_target(),
        &GenOptions::default(),
    );
    assert_eq!(
        text,
        "CREATE INDEX orders_status_idx ON orders (status);\n\n\
         CREATE TRIGGER items_touch BEFORE UPDATE ON shop.items FOR EACH ROW EXECUTE FUNCTION shop.touch();\n\n\
         ALTER TABLE orders ADD CONSTRAINT orders_customer_fk FOREIGN KEY (customer_id) REFERENCES shop.customers (id) ON DELETE CASCADE;\n"
    );
}
