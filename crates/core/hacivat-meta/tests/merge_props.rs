//! Properties of the existence-diff merge (HV-INTRO-11; M4 Task 10): the merge is pure and
//! leaves `prev` untouched; an incremental merge and the merge of a full result built from the
//! same final state give the same namespace (and both equal that state built directly); the
//! delta, after a serde round trip, turns the previous top-level keys into the next ones.

use std::sync::Arc;

use hacivat_meta::delta::{CatalogDelta, apply_delta, top_level_keys};
use hacivat_meta::fixture::{ns_mysql, ns_pg};
use hacivat_meta::merge::{ChangedObject, ExistSet, Identity, MergePolicy, NamespaceResult, merge};
use hacivat_meta::name::Name;
use hacivat_meta::namespace::{Level, NamespaceSnapshot, Obj, Subtree, Watermark};
use hacivat_meta::props::{ColumnProps, IndexProps, Props, TableProps, ViewProps};
use hacivat_meta::{ObjectKey, ObjectKind, ServerId, StateMark};
use proptest::prelude::*;

const TYPES: [&str; 8] = [
    "integer", "text", "bigint", "date", "uuid", "jsonb", "boolean", "numeric",
];

/// One object's life across the two states.
#[derive(Debug, Clone)]
struct Ent {
    before: bool,
    after: bool,
    view: bool,
    renamed: bool,
    /// PostgreSQL: dropped and created again under the same name (a new OID).
    recreated: bool,
    cols0: Vec<u8>,
    cols1: Vec<u8>,
    comment0: Option<u8>,
    comment1: Option<u8>,
    /// PostgreSQL tables: an index child before and after.
    idx0: bool,
    idx1: bool,
}

#[derive(Debug, Clone)]
struct Case {
    pg: bool,
    ents: Vec<Ent>,
    /// Whether the full result also carries existence sets (a full pass may leave them implied).
    full_with_sets: bool,
}

fn cols() -> impl Strategy<Value = Vec<u8>> {
    prop::sample::subsequence((0u8..8).collect::<Vec<_>>(), 1..5)
}

fn ent() -> impl Strategy<Value = Ent> {
    (
        (any::<bool>(), any::<bool>(), any::<bool>()),
        (prop::bool::weighted(0.25), prop::bool::weighted(0.15)),
        (cols(), cols(), prop::bool::weighted(0.6)),
        (
            prop::option::of(0u8..3),
            prop::option::of(0u8..3),
            any::<bool>(),
            any::<bool>(),
        ),
    )
        .prop_map(
            |(
                (before, after, view),
                (renamed, recreated),
                (cols0, cols1, same),
                (c0, c1, i0, i1),
            )| {
                Ent {
                    before,
                    after,
                    view,
                    renamed,
                    recreated,
                    cols1: if same { cols0.clone() } else { cols1 },
                    cols0,
                    comment0: c0,
                    comment1: c1,
                    idx0: i0,
                    idx1: i1,
                }
            },
        )
}

fn merge_case() -> impl Strategy<Value = Case> {
    (
        any::<bool>(),
        prop::collection::vec(ent(), 0..24),
        any::<bool>(),
    )
        .prop_map(|(pg, ents, full_with_sets)| Case {
            pg,
            ents,
            full_with_sets,
        })
}

/// One entity in one state.
struct Shape {
    kind: ObjectKind,
    name: String,
    sid: Option<u32>,
    cols: Vec<u8>,
    comment: Option<u8>,
    index: Option<(String, u32)>,
}

impl Case {
    fn shape(&self, i: usize, after: bool) -> Option<Shape> {
        let e = &self.ents[i];
        if !(if after { e.after } else { e.before }) {
            return None;
        }
        let base = 100 + 10 * i as u32;
        let recreated = after && e.recreated;
        let sid = self.pg.then_some(base + if recreated { 5 } else { 0 });
        let has_index = self.pg && !e.view && if after { e.idx1 } else { e.idx0 };
        Some(Shape {
            kind: if e.view {
                ObjectKind::View
            } else {
                ObjectKind::Table
            },
            name: if after && e.renamed {
                format!("r{i}")
            } else {
                format!("e{i}")
            },
            sid,
            cols: if after {
                e.cols1.clone()
            } else {
                e.cols0.clone()
            },
            comment: if after { e.comment1 } else { e.comment0 },
            index: has_index.then(|| (format!("x{i}"), base + if recreated { 6 } else { 1 })),
        })
    }

    fn subtree(s: &Shape) -> Subtree {
        let comment = s.comment.map(|c| Name::from(format!("note {c}").as_str()));
        let props = match s.kind {
            ObjectKind::View => Props::View(Arc::new(ViewProps {
                comment,
                ..Default::default()
            })),
            _ => Props::Table(Arc::new(TableProps {
                comment,
                ..Default::default()
            })),
        };
        let mut root = Obj::new(s.kind, s.name.as_str()).with_props(props);
        if let Some(sid) = s.sid {
            root = root.with_server_id(ServerId::Oid(sid));
        }
        let mut t = Subtree::new(root);
        for (k, &c) in s.cols.iter().enumerate() {
            t.add(
                t.root(),
                Obj::new(ObjectKind::Column, format!("c{c}").as_str()).with_props(Props::Column(
                    Arc::new(ColumnProps {
                        ordinal: k as u32 + 1,
                        raw_type: Name::from(TYPES[c as usize]),
                        nullable: c % 2 == 0,
                        ..Default::default()
                    }),
                )),
            );
        }
        if let Some((name, sid)) = &s.index {
            t.add(
                t.root(),
                Obj::new(ObjectKind::Index, name.as_str())
                    .with_server_id(ServerId::Oid(*sid))
                    .with_props(Props::Index(Arc::new(IndexProps::default()))),
            );
        }
        t
    }

    /// The namespace in one state, built directly.
    fn ns(&self, after: bool) -> NamespaceSnapshot {
        let mut b = if self.pg {
            ns_pg("db", "public")
        } else {
            ns_mysql("shop", 0)
        };
        for i in 0..self.ents.len() {
            if let Some(s) = self.shape(i, after) {
                b = b.subtree(Case::subtree(&s));
            }
        }
        b.finish()
    }

    fn sets(&self) -> Vec<(ObjectKind, ExistSet)> {
        let mut tables = Vec::new();
        let mut views = Vec::new();
        let mut indexes = Vec::new();
        for i in 0..self.ents.len() {
            let Some(s) = self.shape(i, true) else {
                continue;
            };
            let id = Identity {
                path: vec![Name::from(s.name.as_str())],
                server_id: s.sid.map(ServerId::Oid),
                overload_sig: None,
                state: StateMark::None,
            };
            if let Some((name, sid)) = &s.index {
                indexes.push(Identity {
                    path: vec![Name::from(s.name.as_str()), Name::from(name.as_str())],
                    server_id: Some(ServerId::Oid(*sid)),
                    overload_sig: None,
                    state: StateMark::None,
                });
            }
            match s.kind {
                ObjectKind::View => views.push(id),
                _ => tables.push(id),
            }
        }
        let mut out = vec![
            (ObjectKind::Table, ExistSet::Ok(tables)),
            (ObjectKind::View, ExistSet::Ok(views)),
        ];
        if self.pg {
            out.push((ObjectKind::Index, ExistSet::Ok(indexes)));
        }
        out
    }

    fn result(&self, prev: &NamespaceSnapshot, full: bool) -> NamespaceResult {
        let root = prev.object_key(0).expect("root");
        let mut changed = Vec::new();
        for i in 0..self.ents.len() {
            let Some(after) = self.shape(i, true) else {
                continue;
            };
            let same = self.shape(i, false).is_some_and(|before| {
                before.name == after.name
                    && before.sid == after.sid
                    && before.cols == after.cols
                    && before.comment == after.comment
                    // An index dropped from an unchanged table is the index existence set's job;
                    // any other child change re-reads the table (edge 8).
                    && (before.index == after.index || after.index.is_none())
            });
            if full || !same {
                changed.push(ChangedObject {
                    parent: root.clone(),
                    subtree: Case::subtree(&after),
                });
            }
        }
        NamespaceResult {
            existent: if full && !self.full_with_sets {
                Default::default()
            } else {
                self.sets().into_iter().collect()
            },
            changed,
            categories_to_reload: Default::default(),
            new_watermark: Watermark::None,
            full,
            level: Level::Structure,
            read_at_ms: 1,
        }
    }
}

/// A namespace's top level, sorted, with each object's children in order (layout aside).
type Canon = Vec<(
    ObjectKind,
    String,
    Option<ServerId>,
    Props,
    Vec<(ObjectKind, String, Option<ServerId>, Props)>,
)>;

fn canon(ns: &NamespaceSnapshot) -> Canon {
    let mut v: Canon = ns
        .children(0)
        .map(|(i, o)| {
            (
                o.kind(),
                o.name().to_string(),
                o.server_id(),
                o.props().clone(),
                ns.children(i)
                    .map(|(_, c)| {
                        (
                            c.kind(),
                            c.name().to_string(),
                            c.server_id(),
                            c.props().clone(),
                        )
                    })
                    .collect(),
            )
        })
        .collect();
    v.sort_by(|a, b| (a.0, &a.1).cmp(&(b.0, &b.1)));
    v
}

fn top_keys(keys: &[ObjectKey], depth: usize) -> Vec<ObjectKey> {
    let mut v: Vec<ObjectKey> = keys
        .iter()
        .filter(|k| k.path.len() == depth)
        .cloned()
        .collect();
    v.sort();
    v
}

#[cfg(feature = "system")]
fn round_trip(d: &CatalogDelta) -> CatalogDelta {
    serde_json::from_str(&serde_json::to_string(d).expect("serialize")).expect("deserialize")
}

#[cfg(not(feature = "system"))]
fn round_trip(d: &CatalogDelta) -> CatalogDelta {
    d.clone()
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// The merge is a pure function of its inputs: two runs agree, and `prev` is unchanged.
    #[test]
    fn merge_is_pure_and_prev_untouched(case in merge_case()) {
        let prev = case.ns(false);
        let before = prev.objs().to_vec();
        let r = case.result(&prev, false);
        let a = merge(Some(&prev), r.clone(), &MergePolicy::default()).expect("merge");
        let b = merge(Some(&prev), r, &MergePolicy::default()).expect("merge");
        prop_assert_eq!(prev.objs(), &before[..]);
        prop_assert_eq!(a.next().objs(), b.next().objs());
        prop_assert_eq!(a.delta(), b.delta());
        prop_assert_eq!(a.report(), b.report());
        prop_assert_eq!(a.is_pending(), b.is_pending());
    }

    /// An incremental merge equals the merge of a full result built from the same final state,
    /// and both equal that state; the delta (after a serde round trip) maps the previous
    /// top-level keys to the next ones, and both paths name the same top-level changes.
    #[test]
    fn merge_then_full_equals_full(case in merge_case()) {
        let prev = case.ns(false);
        let expected = canon(&case.ns(true));
        let inc = merge(Some(&prev), case.result(&prev, false), &MergePolicy::default())
            .expect("incremental merge");
        let full = merge(Some(&prev), case.result(&prev, true), &MergePolicy::default())
            .expect("full merge");
        prop_assert_eq!(&canon(inc.next()), &expected);
        prop_assert_eq!(&canon(full.next()), &expected);
        for o in [&inc, &full] {
            let keys = apply_delta(prev.key(), &top_level_keys(&prev), &round_trip(o.delta()))
                .expect("the delta applies");
            prop_assert_eq!(keys, top_level_keys(o.next()));
            prop_assert!(o.report().fragments.is_empty(), "{:?}", o.report().fragments);
        }
        let depth = prev.key().path.len() + 1;
        prop_assert_eq!(
            top_keys(&inc.delta().created, depth),
            top_keys(&full.delta().created, depth)
        );
        prop_assert_eq!(
            top_keys(&inc.delta().dropped, depth),
            top_keys(&full.delta().dropped, depth)
        );
        prop_assert_eq!(&inc.delta().renamed, &full.delta().renamed);
    }
}
