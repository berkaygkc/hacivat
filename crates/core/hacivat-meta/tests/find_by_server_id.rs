// ## m4-t28-routine-lookup (named exception, ruling R-W5-9)
//! `NamespaceSnapshot::find_by_server_id`: the lookup Task 16's incremental pass uses for an
//! existence row, which carries the object's oid but not a routine's overload signature.

use std::sync::Arc;

use hacivat_meta::name::FoldRule;
use hacivat_meta::namespace::NsMeta;
use hacivat_meta::props::{Props, RoutineProps};
use hacivat_meta::{Name, NamespaceSnapshot, NsKey, Obj, ObjectKey, ObjectKind, ServerId, Subtree};
use hacivat_proto::DbmsKind;

fn routine(name: &str, oid: u32, sig: &str) -> Obj {
    Obj::new(ObjectKind::Routine, name)
        .with_server_id(ServerId::Oid(oid))
        .with_props(Props::Routine(Arc::new(RoutineProps {
            overload_sig: Some(Name::from(sig)),
            ..RoutineProps::default()
        })))
}

fn ns(objs: Vec<Obj>) -> NamespaceSnapshot {
    let mut tree = Subtree::new(Obj::new(ObjectKind::Schema, "s"));
    let r = tree.root();
    for o in objs {
        tree.add(r, o);
    }
    NamespaceSnapshot::build(
        NsKey::schema(["db", "s"]),
        DbmsKind::Postgres,
        FoldRule::Exact,
        NsMeta::default(),
        tree,
    )
    .expect("a namespace")
}

/// The key the incremental pass builds from an existence row: no overload signature.
fn key_without_sig(name: &str, oid: u32) -> ObjectKey {
    ObjectKey {
        kind: ObjectKind::Routine,
        path: ["db", "s", name].into_iter().map(Name::from).collect(),
        server_id: Some(ServerId::Oid(oid)),
        overload_sig: None,
        raw_kind: None,
    }
}

#[test]
fn an_overloaded_routine_is_found_by_its_oid_alone() {
    let n = ns(vec![
        routine("add", 16_500, "a integer, b integer"),
        routine("add", 16_501, "a text, b text"),
        routine("f0", 16_502, ""),
    ]);
    for (name, oid) in [("add", 16_500), ("add", 16_501), ("f0", 16_502)] {
        // The old lookup: a key without the signature never matches a routine.
        assert_eq!(n.find(&key_without_sig(name, oid)), None, "{name} {oid}");
        let i = n
            .find_by_server_id(ObjectKind::Routine, ServerId::Oid(oid))
            .unwrap_or_else(|| panic!("{name} {oid}"));
        let o = n.obj(i).unwrap();
        assert_eq!(
            (&**o.name(), o.server_id()),
            (name, Some(ServerId::Oid(oid)))
        );
    }
}

#[test]
fn another_kind_an_unknown_oid_and_a_shared_oid_are_none() {
    let n = ns(vec![routine("f", 16_500, "")]);
    assert_eq!(
        n.find_by_server_id(ObjectKind::Procedure, ServerId::Oid(16_500)),
        None
    );
    assert_eq!(
        n.find_by_server_id(ObjectKind::Routine, ServerId::Oid(16_999)),
        None
    );
    // Two cached routines with one oid are ambiguous, as in `find`.
    let twins = ns(vec![
        routine("f", 16_500, "a int"),
        routine("g", 16_500, ""),
    ]);
    assert_eq!(twins.duplicate_server_ids(), 2);
    assert_eq!(
        twins.find_by_server_id(ObjectKind::Routine, ServerId::Oid(16_500)),
        None
    );
}
