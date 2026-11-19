//! Properties of the existence-diff merge (HV-INTRO-11; M4 Task 10, fix round 1 M-5):
//!
//! - the merge is pure and leaves `prev` untouched;
//! - an incremental merge and the merge of a full result of the same final state agree, and
//!   both equal that state built directly; the delta (after a serde round trip) turns the
//!   previous top-level keys into the next ones;
//! - a chain of two incremental passes equals one full pass to the final state;
//! - with a failed existence set nothing of that kind is deleted, and a pass below the cached
//!   level deletes no child and keeps the level — while incremental and full still agree.
//!
//! The generator covers PostgreSQL (server ids, OID reuse), MySQL under
//! `lower_case_table_names` 0 and 2 and SQLite (names only), case-only respelling, renames,
//! failed sets, a lower pass level, and child sets scoped to the re-read objects or
//! namespace-wide.

use std::sync::Arc;

use hacivat_meta::delta::{CatalogDelta, apply_delta, top_level_keys};
use hacivat_meta::merge::{
    ChangedObject, ExistSet, Identity, MergeOutcome, MergePolicy, NamespaceResult, merge,
};
use hacivat_meta::name::{DialectId, FoldRule, IdentCtx, Name, NameClass};
use hacivat_meta::namespace::{Level, NamespaceSnapshot, NsMeta, Obj, Subtree, Watermark};
use hacivat_meta::props::{ColumnProps, IndexProps, Props, TableProps, ViewProps};
use hacivat_meta::snapshot::IntroErrorCode;
use hacivat_meta::{NsKey, ObjectKey, ObjectKind, ServerId};
use hacivat_proto::DbmsKind;
use proptest::prelude::*;

const TYPES: [&str; 8] = [
    "integer", "text", "bigint", "date", "uuid", "jsonb", "boolean", "numeric",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Engine {
    Pg,
    MySql0,
    MySql2,
    Sqlite,
}

/// One version of an object.
#[derive(Debug, Clone, PartialEq)]
struct Ver {
    renamed: bool,
    /// Case-only respelling (`E3` for `e3`).
    upper: bool,
    /// PostgreSQL: which OID generation (a new one is a drop + create under one name).
    generation: u8,
    cols: Vec<u8>,
    comment: Option<u8>,
    /// PostgreSQL tables: an index child.
    index: bool,
}

/// One object's life across three states.
#[derive(Debug, Clone)]
struct Ent {
    view: bool,
    vers: [Option<Ver>; 3],
}

#[derive(Debug, Clone)]
struct Case {
    engine: Engine,
    ents: Vec<Ent>,
    /// A kind whose existence query fails in every pass.
    failed: Option<ObjectKind>,
    /// The pass reads `Names` over a `Structure` cache.
    names_level: bool,
    /// Incremental column sets scoped to the re-read objects (else namespace-wide).
    scoped: bool,
}

fn ver() -> impl Strategy<Value = Ver> {
    (
        prop::bool::weighted(0.25),
        prop::bool::weighted(0.2),
        prop::sample::select(vec![0u8, 0, 0, 1, 2]),
        prop::sample::subsequence((0u8..8).collect::<Vec<_>>(), 1..5),
        prop::option::of(0u8..3),
        any::<bool>(),
    )
        .prop_map(|(renamed, upper, generation, cols, comment, index)| Ver {
            renamed,
            upper,
            generation,
            cols,
            comment,
            index,
        })
}

fn ent() -> impl Strategy<Value = Ent> {
    (
        any::<bool>(),
        [any::<bool>(), any::<bool>(), any::<bool>()],
        [ver(), ver(), ver()],
        [prop::bool::weighted(0.5), prop::bool::weighted(0.5)],
    )
        .prop_map(|(view, present, mut vs, same)| {
            // Often an object does not change between two states.
            if same[0] {
                vs[1] = vs[0].clone();
            }
            if same[1] {
                vs[2] = vs[1].clone();
            }
            let [v0, v1, v2] = vs;
            Ent {
                view,
                vers: [
                    present[0].then_some(v0),
                    present[1].then_some(v1),
                    present[2].then_some(v2),
                ],
            }
        })
}

fn merge_case() -> impl Strategy<Value = Case> {
    (
        prop::sample::select(vec![
            Engine::Pg,
            Engine::MySql0,
            Engine::MySql2,
            Engine::Sqlite,
        ]),
        prop::collection::vec(ent(), 0..20),
        prop::option::weighted(
            0.3,
            prop::sample::select(vec![
                ObjectKind::Table,
                ObjectKind::View,
                ObjectKind::Index,
                ObjectKind::Column,
            ]),
        ),
        prop::bool::weighted(0.2),
        any::<bool>(),
    )
        .prop_map(|(engine, ents, failed, names_level, scoped)| Case {
            engine,
            ents,
            failed,
            names_level,
            scoped,
        })
}

/// One object in one state.
#[derive(Debug, Clone, PartialEq)]
struct Shape {
    kind: ObjectKind,
    name: String,
    sid: Option<u32>,
    cols: Vec<u8>,
    comment: Option<u8>,
    index: Option<(String, u32)>,
}

impl Case {
    fn pg(&self) -> bool {
        self.engine == Engine::Pg
    }

    fn shape(&self, i: usize, state: usize) -> Option<Shape> {
        let e = &self.ents[i];
        let v = e.vers[state].as_ref()?;
        let base = 100 + 10 * i as u32;
        let stem = match (v.renamed, v.upper) {
            (false, false) => "e",
            (false, true) => "E",
            (true, false) => "r",
            (true, true) => "R",
        };
        Some(Shape {
            kind: if e.view {
                ObjectKind::View
            } else {
                ObjectKind::Table
            },
            name: format!("{stem}{i}"),
            sid: self.pg().then_some(base + u32::from(v.generation)),
            cols: v.cols.clone(),
            comment: v.comment,
            index: (self.pg() && !e.view && v.index)
                .then(|| (format!("x{i}"), base + 5 + u32::from(v.generation))),
        })
    }

    fn shapes(&self, state: usize) -> Vec<Shape> {
        (0..self.ents.len())
            .filter_map(|i| self.shape(i, state))
            .collect()
    }

    fn object(s: &Shape) -> Obj {
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
        let o = Obj::new(s.kind, s.name.as_str()).with_props(props);
        match s.sid {
            Some(sid) => o.with_server_id(ServerId::Oid(sid)),
            None => o,
        }
    }

    /// The object with its children (`children = false`: a `Names` read).
    fn subtree(s: &Shape, children: bool) -> Subtree {
        let mut t = Subtree::new(Case::object(s));
        if !children {
            return t;
        }
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

    fn key(&self) -> NsKey {
        match self.engine {
            Engine::Pg => NsKey::schema(["db", "public"]),
            Engine::Sqlite => NsKey::schema(["main"]),
            Engine::MySql0 | Engine::MySql2 => NsKey::schema(["shop"]),
        }
    }

    fn dbms_fold(&self) -> (DbmsKind, FoldRule) {
        let mysql = |lctn| {
            let ctx = IdentCtx {
                lower_case_table_names: lctn,
                ..IdentCtx::new(DialectId::MySql)
            };
            FoldRule::for_names(DialectId::MySql, &ctx, NameClass::Table).expect("pinned")
        };
        match self.engine {
            Engine::Pg => (DbmsKind::Postgres, FoldRule::Exact),
            Engine::Sqlite => (DbmsKind::Sqlite, FoldRule::AsciiCaseless),
            Engine::MySql0 => (DbmsKind::MySql, mysql(0)),
            Engine::MySql2 => (DbmsKind::MySql, mysql(2)),
        }
    }

    /// The namespace in one state, built directly, at `Structure`.
    fn ns(&self, state: usize) -> NamespaceSnapshot {
        let key = self.key();
        let mut t = Subtree::new(Obj::new(ObjectKind::Schema, key.root_name()));
        for s in self.shapes(state) {
            t.graft_under(t.root(), Case::subtree(&s, true));
        }
        let (dbms, fold) = self.dbms_fold();
        let meta = NsMeta {
            level: Level::Structure,
            ..NsMeta::default()
        };
        NamespaceSnapshot::build(key, dbms, fold, meta, t).expect("a valid namespace")
    }

    /// Whether a pass at this case's level sees no change between two versions (so an
    /// incremental pass need not re-read it). A dropped index is the index set's job.
    fn same(&self, a: &Shape, b: &Shape) -> bool {
        let names = a.name == b.name && a.sid == b.sid && a.comment == b.comment;
        if self.names_level {
            return names;
        }
        names && a.cols == b.cols && (a.index == b.index || b.index.is_none())
    }

    /// The pass from `from` to `to`, incremental or full.
    fn result(
        &self,
        prev: &NamespaceSnapshot,
        from: usize,
        to: usize,
        full: bool,
    ) -> NamespaceResult {
        let root = prev.object_key(0).expect("root");
        let children = !self.names_level;
        let mut changed = Vec::new();
        let mut reread = Vec::new();
        for i in 0..self.ents.len() {
            let Some(cur) = self.shape(i, to) else {
                continue;
            };
            let same = self.shape(i, from).is_some_and(|old| self.same(&old, &cur));
            if full || !same {
                changed.push(ChangedObject {
                    parent: root.clone(),
                    subtree: Case::subtree(&cur, children),
                });
                reread.push(cur);
            }
        }
        let id = |path: Vec<&str>, sid: Option<u32>| {
            Identity::of(
                path.into_iter().map(Name::from).collect(),
                sid.map(ServerId::Oid),
            )
        };
        let all = self.shapes(to);
        let mut existent: Vec<(ObjectKind, ExistSet)> = vec![
            (
                ObjectKind::Table,
                ExistSet::Ok(
                    all.iter()
                        .filter(|s| s.kind == ObjectKind::Table)
                        .map(|s| id(vec![&s.name], s.sid))
                        .collect(),
                ),
            ),
            (
                ObjectKind::View,
                ExistSet::Ok(
                    all.iter()
                        .filter(|s| s.kind == ObjectKind::View)
                        .map(|s| id(vec![&s.name], s.sid))
                        .collect(),
                ),
            ),
        ];
        if children {
            if self.pg() {
                existent.push((
                    ObjectKind::Index,
                    ExistSet::Ok(
                        all.iter()
                            .filter_map(|s| {
                                s.index
                                    .as_ref()
                                    .map(|(x, sid)| id(vec![&s.name, x], Some(*sid)))
                            })
                            .collect(),
                    ),
                ));
            }
            let cols = |shapes: &[Shape]| -> Vec<Identity> {
                shapes
                    .iter()
                    .flat_map(|s| {
                        s.cols
                            .iter()
                            .map(|c| id(vec![&s.name, &format!("c{c}")], None))
                            .collect::<Vec<_>>()
                    })
                    .collect()
            };
            // A full pass's column set covers the objects it re-read (all of them); it claims
            // the whole namespace only when no section failed (a pass whose table query failed
            // cannot know the columns of tables it could not list).
            existent.push((
                ObjectKind::Column,
                if self.scoped || self.failed.is_some() {
                    ExistSet::Scoped {
                        scope: reread
                            .iter()
                            .map(|s| vec![Name::from(s.name.as_str())])
                            .collect(),
                        ids: cols(&reread),
                    }
                } else {
                    ExistSet::Ok(cols(&all))
                },
            ));
        }
        if let Some(k) = self.failed {
            existent.retain(|(kind, _)| *kind != k);
            existent.push((k, ExistSet::Failed(IntroErrorCode::PermissionDenied)));
        }
        NamespaceResult {
            existent: existent.into_iter().collect(),
            changed,
            categories_to_reload: Default::default(),
            new_watermark: Watermark::None,
            full,
            level: if self.names_level {
                Level::Names
            } else {
                Level::Structure
            },
            read_at_ms: 1,
        }
    }

    /// Whether the final state is what every pass should reproduce exactly.
    fn clean(&self) -> bool {
        self.failed.is_none() && !self.names_level
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
    v.sort_by(|a, b| (a.0, &a.1, a.2).cmp(&(b.0, &b.1, b.2)));
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

fn run(prev: &NamespaceSnapshot, r: NamespaceResult) -> MergeOutcome {
    merge(Some(prev), r, &MergePolicy::default()).expect("merge")
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// The merge is a pure function of its inputs: two runs agree, and `prev` is unchanged.
    #[test]
    fn merge_is_pure_and_prev_untouched(case in merge_case()) {
        let prev = case.ns(0);
        let before = prev.objs().to_vec();
        let r = case.result(&prev, 0, 1, false);
        let a = run(&prev, r.clone());
        let b = run(&prev, r);
        prop_assert_eq!(prev.objs(), &before[..]);
        prop_assert_eq!(a.next().objs(), b.next().objs());
        prop_assert_eq!(a.delta(), b.delta());
        prop_assert_eq!(a.report(), b.report());
        prop_assert_eq!(a.is_pending(), b.is_pending());
    }

    /// An incremental merge equals the merge of a full result built from the same final state,
    /// and (for a clean pass) both equal that state; the delta maps the previous top-level keys
    /// to the next ones; a failed kind loses nothing; a lower pass deletes no child.
    #[test]
    fn merge_then_full_equals_full(case in merge_case()) {
        let prev = case.ns(0);
        let inc = run(&prev, case.result(&prev, 0, 1, false));
        let full = run(&prev, case.result(&prev, 0, 1, true));
        prop_assert_eq!(&canon(inc.next()), &canon(full.next()));
        if case.clean() {
            prop_assert_eq!(&canon(inc.next()), &canon(&case.ns(1)));
        }
        for o in [&inc, &full] {
            let keys = apply_delta(prev.key(), &top_level_keys(&prev), &round_trip(o.delta()))
                .expect("the delta applies");
            prop_assert_eq!(keys, top_level_keys(o.next()));
            prop_assert!(o.report().fragments.is_empty(), "{:?}", o.report().fragments);
            if let Some(k) = case.failed {
                prop_assert!(o.delta().dropped.iter().all(|d| d.kind != k));
                prop_assert!(o.report().stale_kinds.contains_key(&k));
            }
            if case.names_level {
                prop_assert_eq!(o.next().meta().level, Level::Structure);
                prop_assert!(o
                    .delta()
                    .dropped
                    .iter()
                    .all(|d| d.kind != ObjectKind::Column && d.kind != ObjectKind::Index));
            }
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

    /// Two incremental passes (S0 → S1 → S2) equal one full pass S0 → S2, and the final state.
    #[test]
    fn incremental_chain_equals_one_full_pass(case in merge_case()) {
        let case = Case { failed: None, names_level: false, ..case };
        let s0 = case.ns(0);
        let s1 = run(&s0, case.result(&s0, 0, 1, false)).into_parts().0;
        let s2 = run(&s1, case.result(&s1, 1, 2, false)).into_parts().0;
        let direct = run(&s0, case.result(&s0, 0, 2, true)).into_parts().0;
        let expected = canon(&case.ns(2));
        prop_assert_eq!(&canon(&s2), &expected);
        prop_assert_eq!(&canon(&direct), &expected);
    }
}
