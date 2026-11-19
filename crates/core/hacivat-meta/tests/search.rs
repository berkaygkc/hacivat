//! Object search over a snapshot (M4 Task 9; A-14, M8's `search_objects`): ASCII-caseless,
//! locale-free, bounded by `limit`, in a deterministic order.

use std::sync::Arc;

use hacivat_meta::fixture::ns_pg;
use hacivat_meta::ids::NsKey;
use hacivat_meta::kind::ObjectKind;
use hacivat_meta::name::{DialectId, IdentCtx};
use hacivat_meta::namespace::NamespaceSnapshot;
use hacivat_meta::search::{ObjHit, SearchMode, SearchQuery, search};
use hacivat_meta::snapshot::{Catalog, CatalogChange, CatalogSnapshot, ServerFacts};
use hacivat_proto::DbmsKind;

fn snapshot(nss: Vec<NamespaceSnapshot>) -> Arc<CatalogSnapshot> {
    let c = Catalog::new(Arc::new(ServerFacts::new(
        DbmsKind::Postgres,
        IdentCtx::new(DialectId::Postgres),
    )));
    c.publisher()
        .unwrap()
        .publish(
            nss.into_iter()
                .map(|n| CatalogChange::Put(Arc::new(n)))
                .collect(),
        )
        .unwrap();
    c.current()
}

fn q(text: &str, kinds: &[ObjectKind], limit: usize, mode: SearchMode) -> SearchQuery {
    SearchQuery {
        text: text.to_string(),
        kinds: kinds.to_vec(),
        limit,
        mode,
    }
}

fn names(hits: &[ObjHit]) -> Vec<String> {
    hits.iter()
        .map(|h| format!("{}.{}", h.ns.root_name(), h.name))
        .collect()
}

fn turkish() -> Arc<CatalogSnapshot> {
    snapshot(vec![
        ns_pg("db", "public")
            .table_oid("İndeks", 1, &["id", "ılık"])
            .table_oid("index_log", 2, &["Id"])
            .table_oid("ID_MAP", 3, &[])
            .table_oid("kişi", 4, &[])
            .finish(),
    ])
}

#[test]
fn search_is_locale_free_and_bounded() {
    let snap = turkish();
    let tables = [ObjectKind::Table];
    // ASCII letters fold both ways; `I` is never `ı` or `İ` (no locale, no Unicode folding).
    assert_eq!(
        names(&search(&snap, &q("i", &tables, 10, SearchMode::Substring))),
        ["public.index_log", "public.ID_MAP", "public.kişi"]
    );
    assert_eq!(
        names(&search(&snap, &q("İ", &tables, 10, SearchMode::Substring))),
        ["public.İndeks"]
    );
    assert_eq!(
        names(&search(
            &snap,
            &q("ı", &[ObjectKind::Column], 10, SearchMode::Prefix)
        )),
        ["public.ılık"]
    );
    assert!(
        search(
            &snap,
            &q("I", &[ObjectKind::Column], 10, SearchMode::Prefix)
        )
        .iter()
        .all(|h| h.name.as_ref() != "ılık")
    );
    assert_eq!(
        names(&search(&snap, &q("ŞI", &tables, 10, SearchMode::Substring))),
        Vec::<String>::new()
    );
    // Bounded: at most `limit` hits, and none for a zero limit.
    assert_eq!(
        search(&snap, &q("", &[], 3, SearchMode::Substring)).len(),
        3
    );
    assert!(search(&snap, &q("i", &tables, 0, SearchMode::Substring)).is_empty());
    // A prefix query matches the start only.
    assert_eq!(
        names(&search(&snap, &q("ID", &tables, 10, SearchMode::Prefix))),
        ["public.ID_MAP"]
    );
    // The hit carries the kind, the arena index and the exact name.
    let hit = &search(&snap, &q("kişi", &tables, 1, SearchMode::Substring))[0];
    assert_eq!(hit.kind, ObjectKind::Table);
    assert_eq!(hit.ns, NsKey::schema(["db", "public"]));
    assert_eq!(
        snap.namespaces[&hit.ns]
            .obj(hit.id)
            .unwrap()
            .name()
            .as_ref(),
        "kişi"
    );
}

#[test]
fn search_order_is_deterministic() {
    let snap = snapshot(vec![
        ns_pg("db", "sales")
            .table_oid("orders_archive", 10, &[])
            .table_oid("orders", 11, &[])
            .finish(),
        ns_pg("db", "public")
            .table_oid("old_orders", 1, &[])
            .table_oid("Orders", 2, &[])
            .table_oid("orders_v2", 3, &[])
            .finish(),
    ]);
    let tables = [ObjectKind::Table];
    // Exact (ASCII-caseless) matches first, then prefix, then substring; within each, the
    // namespace key order, then arena order.
    let all = names(&search(
        &snap,
        &q("orders", &tables, 10, SearchMode::Substring),
    ));
    assert_eq!(
        all,
        [
            "public.Orders",
            "sales.orders",
            "public.orders_v2",
            "sales.orders_archive",
            "public.old_orders"
        ]
    );
    // The bound keeps the best hits, the same ones every time.
    for _ in 0..3 {
        assert_eq!(
            names(&search(
                &snap,
                &q("ORDERS", &tables, 3, SearchMode::Substring)
            )),
            ["public.Orders", "sales.orders", "public.orders_v2"]
        );
    }
    let prefix = names(&search(
        &snap,
        &q("orders", &tables, 10, SearchMode::Prefix),
    ));
    assert_eq!(
        prefix,
        [
            "public.Orders",
            "sales.orders",
            "public.orders_v2",
            "sales.orders_archive"
        ]
    );
    // Kinds filter; an empty kind list searches every kind but the server root.
    assert!(
        search(
            &snap,
            &q("orders", &[ObjectKind::View], 10, SearchMode::Substring)
        )
        .is_empty()
    );
    assert_eq!(
        names(&search(&snap, &q("sales", &[], 10, SearchMode::Substring))),
        ["sales.sales"]
    );
}
