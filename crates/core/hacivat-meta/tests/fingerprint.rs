//! Structural fingerprints (M4 Task 11, HV-MODEL-03, R-M4-26): XXH3-128 over a subtree's
//! canonical encoding, blind to state marks, statistics and server ids.

use std::sync::Arc;

use hacivat_meta::codec::{decode_namespace, encode_namespace};
use hacivat_meta::fingerprint::{Fingerprint, subtree};
use hacivat_meta::fixture::{DB, pg_catalog};
use hacivat_meta::ids::{NsKey, ObjId, Ref, ServerId, StateMark};
use hacivat_meta::kind::ObjectKind;
use hacivat_meta::name::{FoldRule, Name};
use hacivat_meta::namespace::{NamespaceSnapshot, NsMeta, Obj, Subtree, Watermark};
use hacivat_meta::props::*;
use hacivat_proto::DbmsKind;

fn col(name: &str, ordinal: u32, ty: &str) -> Obj {
    Obj::new(ObjectKind::Column, name).with_props(Props::Column(Arc::new(ColumnProps {
        ordinal,
        raw_type: Name::from(ty),
        nullable: true,
        ..Default::default()
    })))
}

/// `app.public` with table `sipariş` (ids, state marks and the FK target id as given; columns
/// in the given insertion order) and a referenced table `İl`.
fn shop(
    table_oid: u32,
    xmin: u32,
    fk_target_oid: Option<u32>,
    order: &[usize],
    ty: &str,
) -> NamespaceSnapshot {
    let mut t = Subtree::new(Obj::new(ObjectKind::Schema, "public"));
    let root = t.root();
    let tab = t.add(
        root,
        Obj::new(ObjectKind::Table, "sipariş")
            .with_server_id(ServerId::Oid(table_oid))
            .with_state(StateMark::Xmin(xmin))
            .with_props(Props::Table(Arc::new(TableProps {
                comment: Some(Name::from("Iı İi")),
                ..Default::default()
            }))),
    );
    let cols = [
        col("id", 1, "bigint"),
        col("il_id", 2, ty),
        col("ılık", 3, "text"),
    ];
    for &i in order {
        t.add(tab, cols[i].clone());
    }
    t.add(
        tab,
        Obj::new(ObjectKind::ForeignKey, "sipariş_il_fkey")
            .with_server_id(ServerId::Oid(table_oid + 1))
            .with_state(StateMark::Xmin(xmin + 1))
            .with_props(Props::ForeignKey(Arc::new(FkProps {
                columns: vec![Name::from("il_id")],
                target: Ref {
                    kind: ObjectKind::Table,
                    path: [DB, "public", "İl"].into_iter().map(Name::from).collect(),
                    server_id: fk_target_oid.map(ServerId::Oid),
                },
                target_columns: vec![Name::from("id")],
                on_update: FkAction::NoAction,
                on_delete: FkAction::Cascade,
                match_kind: FkMatch::Simple,
                deferrable: false,
                initially_deferred: false,
                not_valid: false,
                enforced: true,
            }))),
    );
    let il = t.add(
        root,
        Obj::new(ObjectKind::Table, "İl").with_server_id(ServerId::Oid(table_oid + 100)),
    );
    t.add(il, col("id", 1, "bigint"));
    NamespaceSnapshot::build(
        NsKey::schema([DB, "public"]),
        DbmsKind::Postgres,
        FoldRule::Exact,
        NsMeta {
            watermark: Watermark::PgXid {
                w: u64::from(xmin),
                identity: Default::default(),
            },
            loaded_at_ms: i64::from(xmin),
            ..Default::default()
        },
        t,
    )
    .unwrap()
}

/// The arena index of the first object named `name`.
fn id_of(ns: &NamespaceSnapshot, name: &str) -> ObjId {
    let idx = ns.objs().iter().position(|o| &**o.name() == name).unwrap();
    ObjId {
        ns: 0,
        idx: idx as u32,
    }
}

fn fp(ns: &NamespaceSnapshot, name: &str) -> Fingerprint {
    subtree(ns, id_of(ns, name)).unwrap()
}

#[test]
fn fingerprint_ignores_state_marks_statistics_and_oids() {
    let a = shop(16_400, 10, Some(16_500), &[0, 1, 2], "bigint");
    // Other OIDs, other xmins, no FK target id, another watermark and load time.
    let b = shop(20_000, 99, None, &[0, 1, 2], "bigint");
    assert_eq!(fp(&a, "sipariş"), fp(&b, "sipariş"));
    assert_eq!(fp(&a, "public"), fp(&b, "public"));
    // Statistics are a facet, never part of an `Obj`; the order a server listed children in
    // does not matter (the column ordinals do).
    let c = shop(16_400, 10, Some(16_500), &[2, 0, 1], "bigint");
    assert_eq!(fp(&a, "sipariş"), fp(&c, "sipariş"));
    // Out of range: no fingerprint, never a constant.
    assert!(subtree(&a, ObjId { ns: 0, idx: 9_999 }).is_none());
}

#[test]
fn fingerprint_changes_on_column_type_change() {
    let a = shop(16_400, 10, None, &[0, 1, 2], "bigint");
    let b = shop(16_400, 10, None, &[0, 1, 2], "integer");
    assert_ne!(fp(&a, "sipariş"), fp(&b, "sipariş"));
    assert_ne!(fp(&a, "public"), fp(&b, "public"));
    // The column itself, and nothing outside the changed subtree.
    assert_ne!(fp(&a, "il_id"), fp(&b, "il_id"));
    assert_eq!(fp(&a, "İl"), fp(&b, "İl"));
}

#[test]
fn fingerprint_stable_across_encode_decode() {
    for ns in pg_catalog(&[("public", 120), ("sales", 30)], 1_400) {
        let back = decode_namespace(&encode_namespace(&ns).unwrap()).unwrap();
        for idx in 0..ns.len() as u32 {
            let id = ObjId { ns: 0, idx };
            assert_eq!(subtree(&ns, id), subtree(&back, id), "{idx}");
        }
    }
}

/// Bytes only: no casing, no collation. The value is pinned, so the same test under `tr_TR`
/// (CI's `turkey-test`, or `LC_ALL=tr_TR.UTF-8` locally) proves the locale does not reach it.
/// Dotted and dotless I stay four different letters.
#[test]
fn fingerprint_identical_under_tr_locale() {
    let ns = shop(16_400, 10, Some(1), &[0, 1, 2], "bigint");
    assert_eq!(fp(&ns, "sipariş"), Fingerprint(PINNED_SIPARIS));
    let names = ["I", "ı", "İ", "i"];
    let mut seen = std::collections::HashSet::new();
    for n in names {
        let mut t = Subtree::new(Obj::new(ObjectKind::Schema, "s"));
        t.add(t.root(), Obj::new(ObjectKind::Table, n));
        let ns = NamespaceSnapshot::build(
            NsKey::schema(["s"]),
            DbmsKind::MySql,
            FoldRule::AsciiCaseless,
            NsMeta::default(),
            t,
        )
        .unwrap();
        assert!(seen.insert(fp(&ns, n)), "{n}");
    }
}

/// The fingerprint of `sipariş` in [`fingerprint_identical_under_tr_locale`], computed once
/// under the C locale.
const PINNED_SIPARIS: u128 = 3_585_153_592_469_755_192_533_275_119_297_880_807;
