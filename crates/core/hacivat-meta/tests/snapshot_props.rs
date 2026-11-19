//! alg/02 HV-MODEL-02 test 7 (M4 Task 9): random publish/lookup interleavings on 8 threads
//! never show a torn read — every answer of one lookup comes from one generation.

use std::sync::Arc;
use std::time::SystemTime;

use hacivat_meta::fixture::ns_pg;
use hacivat_meta::ids::NsKey;
use hacivat_meta::kind::{KindGroup, ObjectKind};
use hacivat_meta::name::{DialectId, IdentCtx, Name, NamePart};
use hacivat_meta::namespace::{Level, Watermark};
use hacivat_meta::search_path::SearchPath;
use hacivat_meta::snapshot::{Catalog, CatalogChange, ContainerKey, LoadState, ServerFacts};
use hacivat_meta::view::{Lookup, ModelView, SnapshotView};
use hacivat_proto::DbmsKind;
use proptest::prelude::*;

const DB: &str = "db";
const READERS: usize = 7;

#[derive(Debug, Clone)]
enum Op {
    /// Publish generation-tagged `public` and `sales` with `n` tables each.
    Publish(u8),
    /// Look up table `t<n>` in `public` and in `sales` through one view.
    Lookup(u8),
}

fn ops_strategy(len: usize) -> impl Strategy<Value = Vec<Op>> {
    prop::collection::vec(
        prop_oneof![
            1 => (1u8..12).prop_map(Op::Publish),
            9 => (0u8..12).prop_map(Op::Lookup),
        ],
        len,
    )
}

fn part(s: &str) -> NamePart {
    NamePart {
        text: Name::from(s),
        quoted: false,
    }
}

/// Two namespaces whose table count is `n` and whose tables carry `tag` as their server id.
fn changes(n: u8, tag: u32) -> Vec<CatalogChange> {
    let mut out = Vec::new();
    for schema in ["public", "sales"] {
        let mut b = ns_pg(DB, schema);
        for i in 0..n {
            b = b.table_oid(&format!("t{i}"), tag, &["a"]);
        }
        let ns = b.finish();
        out.push(CatalogChange::SetState(
            ContainerKey::Namespace(ns.key().clone()),
            LoadState::Loaded {
                level: Level::Structure,
                at: SystemTime::UNIX_EPOCH,
                watermark: Watermark::None,
                stale: false,
            },
        ));
        out.push(CatalogChange::Put(Arc::new(ns)));
    }
    out
}

fn run(ops: Vec<Op>) {
    let catalog = Arc::new(Catalog::new(Arc::new(ServerFacts::new(
        DbmsKind::Postgres,
        IdentCtx {
            encoding: Some(Arc::from("UTF8")),
            ..IdentCtx::new(DialectId::Postgres)
        },
    ))));
    let publisher = catalog.publisher().unwrap();
    publisher.publish(changes(1, 1)).unwrap();
    let path = SearchPath::exact(
        Some(Name::from(DB)),
        [NsKey::schema([DB, "public"]), NsKey::schema([DB, "sales"])],
    );
    let (publishes, lookups): (Vec<Op>, Vec<Op>) =
        ops.into_iter().partition(|o| matches!(o, Op::Publish(_)));
    let lookups = Arc::new(lookups);
    let readers: Vec<_> = (0..READERS)
        .map(|r| {
            let catalog = catalog.clone();
            let path = path.clone();
            let lookups = lookups.clone();
            std::thread::spawn(move || {
                for k in (r..lookups.len()).step_by(READERS) {
                    let Op::Lookup(n) = lookups[k] else {
                        unreachable!()
                    };
                    let view = SnapshotView::new(catalog.current()).with_search_path(path.clone());
                    let g = view.snapshot().generation;
                    let t = format!("t{n}");
                    let hits: Vec<_> = ["public", "sales"]
                        .into_iter()
                        .map(|s| view.lookup(&[part(s), part(&t)], KindGroup::Relations))
                        .collect();
                    // Both schemas of one generation hold the same tables, published under
                    // that generation and tagged with the same server id.
                    match (&hits[0], &hits[1]) {
                        (Lookup::Found(a), Lookup::Found(b)) => {
                            assert_eq!(a.ns.generation(), g);
                            assert_eq!(b.ns.generation(), g);
                            assert_eq!(a.obj().server_id(), b.obj().server_id());
                        }
                        (Lookup::Missing(_), Lookup::Missing(_)) => {
                            for s in ["public", "sales"] {
                                let ns = &view.snapshot().namespaces[&NsKey::schema([DB, s])];
                                assert_eq!(ns.generation(), g);
                                assert!(
                                    ns.iter_named(&[ObjectKind::Table]).count() <= usize::from(n)
                                );
                            }
                        }
                        (a, b) => {
                            let brief = |l: &Lookup<hacivat_meta::snapshot::ObjHandle>| match l {
                                Lookup::Found(h) => {
                                    format!(
                                        "Found(gen {}, {:?})",
                                        h.ns.generation(),
                                        h.obj().server_id()
                                    )
                                }
                                other => format!("{other:?}"),
                            };
                            panic!("torn at generation {g}: {} / {}", brief(a), brief(b));
                        }
                    }
                }
            })
        })
        .collect();
    for (tag, op) in publishes.into_iter().enumerate() {
        let Op::Publish(n) = op else { unreachable!() };
        publisher.publish(changes(n, tag as u32 + 2)).unwrap();
    }
    for r in readers {
        r.join().unwrap();
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(4))]
    #[test]
    fn no_torn_reads_8_threads(ops in ops_strategy(10_000)) {
        run(ops);
    }
}
