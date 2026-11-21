//! Format minor 1 (M4 Task 13, R-W2-8): the rare parts of keys, foreign keys and checks
//! round-trip through the cache codec; a minor-0 payload reads `rare: None`; an object without
//! a rare part encodes to the same body as in minor 0; the name cap holds for the new names.

#![cfg(feature = "persist")]

use std::sync::Arc;

use hacivat_meta::codec::testing::{decode_as_build, encode_as_minor, encode_unchecked};
use hacivat_meta::codec::{
    DecodeError, FORMAT_MINOR, HEADER_LEN, MAX_NAME_BYTES, decode, encode_namespace,
};
use hacivat_meta::ids::{NsKey, Ref, ServerId};
use hacivat_meta::kind::ObjectKind;
use hacivat_meta::name::{FoldRule, Name};
use hacivat_meta::namespace::{NamespaceSnapshot, NsMeta, Obj, Subtree};
use hacivat_meta::props::{
    CheckProps, CheckRare, ColumnProps, FkAction, FkMatch, FkProps, FkRare, IndexProps, KeyColumn,
    KeyProps, KeyRare, Props, TableProps,
};
use hacivat_proto::DbmsKind;

fn n(s: &str) -> Name {
    Arc::from(s)
}

fn key(cols: &[(u32, &str)], rare: Option<KeyRare>) -> KeyProps {
    KeyProps {
        columns: cols
            .iter()
            .map(|(o, c)| KeyColumn {
                ordinal: *o,
                name: Some(n(c)),
            })
            .collect(),
        desc: vec![false; cols.len()],
        rare: rare.map(Arc::new),
        ..KeyProps::default()
    }
}

/// `v` when `on`.
fn opt<T>(on: bool, v: T) -> Option<T> {
    on.then_some(v)
}

/// A PostgreSQL table with an index, a unique key, an FK, a check and an exclusion constraint;
/// with or without their rare parts.
fn ns(with_rare: bool, opclass: &str) -> NamespaceSnapshot {
    let mut t = Subtree::new(Obj::new(ObjectKind::Schema, "public"));
    let tab = t.add(
        t.root(),
        Obj::new(ObjectKind::Table, "t")
            .with_server_id(ServerId::Oid(16_400))
            .with_props(Props::Table(Arc::new(TableProps::default()))),
    );
    for (i, c) in ["a", "b"].iter().enumerate() {
        t.add(
            tab,
            Obj::new(ObjectKind::Column, *c).with_props(Props::Column(Arc::new(ColumnProps {
                ordinal: i as u32 + 1,
                raw_type: n("text"),
                ..Default::default()
            }))),
        );
    }
    t.add(
        tab,
        Obj::new(ObjectKind::Index, "t_ab_idx")
            .with_server_id(ServerId::Oid(16_401))
            .with_props(Props::Index(Arc::new(IndexProps {
                key: key(
                    &[(1, "a"), (2, "b")],
                    opt(
                        with_rare,
                        KeyRare {
                            nulls_first: vec![false, true],
                            opclass: vec![Some(n(opclass)), None],
                            collation: vec![None, Some(n("C"))],
                        },
                    ),
                ),
                method: Some(n("gin")),
                ..Default::default()
            }))),
    );
    t.add(
        tab,
        Obj::new(ObjectKind::UniqueKey, "t_a_key")
            .with_server_id(ServerId::Oid(16_402))
            .with_props(Props::Key(Arc::new(key(
                &[(1, "a")],
                opt(
                    with_rare,
                    KeyRare {
                        collation: vec![Some(n("tr-TR-x-icu"))],
                        ..KeyRare::default()
                    },
                ),
            )))),
    );
    t.add(
        tab,
        Obj::new(ObjectKind::ForeignKey, "t_b_fkey")
            .with_server_id(ServerId::Oid(16_403))
            .with_props(Props::ForeignKey(Arc::new(FkProps {
                columns: vec![n("b")],
                target: Ref {
                    kind: ObjectKind::Table,
                    path: ["db", "other", "p"].into_iter().map(n).collect(),
                    server_id: Some(ServerId::Oid(9_000)),
                },
                target_columns: vec![n("id")],
                on_update: FkAction::NoAction,
                on_delete: FkAction::Cascade,
                match_kind: FkMatch::Simple,
                deferrable: false,
                initially_deferred: false,
                not_valid: false,
                enforced: true,
                rare: opt(
                    with_rare,
                    FkRare {
                        fk_ref_index: Some(Ref {
                            kind: ObjectKind::Index,
                            path: ["db", "other", "p", "p_pkey"].into_iter().map(n).collect(),
                            server_id: Some(ServerId::Oid(9_001)),
                        }),
                    },
                )
                .map(Arc::new),
            }))),
    );
    t.add(
        tab,
        Obj::new(ObjectKind::Check, "t_a_check").with_props(Props::Check(Arc::new(CheckProps {
            expr: n("a <> ''::text"),
            rare: opt(
                with_rare,
                CheckRare {
                    no_inherit: true,
                    ..CheckRare::default()
                },
            )
            .map(Arc::new),
            ..CheckProps::default()
        }))),
    );
    t.add(
        tab,
        Obj::new(ObjectKind::Exclusion, "t_excl").with_props(Props::Check(Arc::new(CheckProps {
            expr: n("USING gist (a WITH =)"),
            rare: opt(
                with_rare,
                CheckRare {
                    excl_ops: vec![n("=")],
                    excl_deferrable: true,
                    excl_initially_deferred: true,
                    ..CheckRare::default()
                },
            )
            .map(Arc::new),
            ..CheckProps::default()
        }))),
    );
    NamespaceSnapshot::build(
        NsKey::schema(["db", "public"]),
        DbmsKind::Postgres,
        FoldRule::Exact,
        NsMeta::default(),
        t,
    )
    .expect("namespace")
}

#[test]
fn rare_parts_round_trip_at_the_current_minor() {
    let with = ns(true, "public.gin_trgm_ops(siglen=32)");
    let back = decode(&encode_namespace(&with).unwrap()).unwrap();
    assert_eq!(back.minor, FORMAT_MINOR);
    assert!(!back.from_newer_minor);
    assert_eq!(back.ns.objs(), with.objs());
    let ix = back
        .ns
        .objs()
        .iter()
        .find(|o| o.kind() == ObjectKind::Index)
        .unwrap();
    let Props::Index(p) = ix.props() else {
        panic!("index props")
    };
    assert_eq!(p.key.nulls_first(), &[false, true][..]);
    assert_eq!(
        p.key.opclass(),
        &[Some(n("public.gin_trgm_ops(siglen=32)")), None][..]
    );
}

/// A payload written by minor 0 has no rare field: it reads `rare: None` everywhere, is not
/// from a newer minor, and decodes to the namespace without rare parts.
#[test]
fn an_older_minor_payload_reads_no_rare_part() {
    let with = ns(true, "public.gin_trgm_ops");
    let old = encode_as_minor(&with, 0, &[]).unwrap();
    let back = decode(&old).unwrap();
    assert_eq!(back.minor, 0);
    assert!(!back.from_newer_minor);
    assert_eq!(back.ns.objs(), ns(false, "public.gin_trgm_ops").objs());
}

/// An object without a rare part encodes to the same body in minor 1 as in minor 0 (only the
/// header's minor and checksum differ), so cached rows and fingerprints of such objects do not
/// change with the bump.
#[test]
fn objects_without_rare_parts_encode_as_in_minor_zero() {
    let without = ns(false, "x");
    let now = encode_namespace(&without).unwrap();
    let old = encode_as_minor(&without, 0, &[]).unwrap();
    assert_eq!(now[HEADER_LEN..], old[HEADER_LEN..]);
    assert_ne!(now, old);
}

/// A build of minor 0 reading a minor-1 payload with rare parts: it is from a newer minor
/// (never written back), and its objects still load.
#[test]
fn an_older_build_reads_rare_parts_as_a_newer_minor() {
    let with = ns(true, "public.gin_trgm_ops");
    let bytes = encode_namespace(&with).unwrap();
    let back = decode_as_build(&bytes, 0).unwrap();
    assert!(back.from_newer_minor);
    assert_eq!(back.ns.objs().len(), with.objs().len());
}

/// The name cap covers the new names: the encoder refuses an operator class over
/// `MAX_NAME_BYTES`, and the decoder refuses a crafted payload that holds one.
#[test]
fn rare_names_are_capped() {
    let long = "o".repeat(MAX_NAME_BYTES + 1);
    let bad = ns(true, &long);
    assert!(matches!(
        encode_namespace(&bad),
        Err(DecodeError::Malformed(_))
    ));
    let crafted = encode_unchecked(&bad).unwrap();
    assert!(matches!(decode(&crafted), Err(DecodeError::Malformed(_))));
}
