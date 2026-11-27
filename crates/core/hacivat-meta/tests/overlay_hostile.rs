//! T26 opus review: hostile tests the implementer did not write (`task-m4-26-review-hostile.rs`),
//! committed in fix round 1. H1-H5 failed at `6eb409f`; they now assert the fixes (I-1..I-4,
//! m-2; H2 tightened from "not Missing" to "Found from the live arena", ruling R-W5-6). H6
//! asserts the race; H7 and H8 only measure.
#![cfg(feature = "system")]

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::SystemTime;

use hacivat_meta::fixture::{ns_mysql, ns_pg};
use hacivat_meta::ids::{ObjectKey, ServerId};
use hacivat_meta::kind::{KindGroup, ObjectKind};
use hacivat_meta::name::{DialectId, IdentCtx, IdentKey, Name, NameClass, NamePart, match_key};
use hacivat_meta::namespace::{Level, NamespaceSnapshot, Watermark};
use hacivat_meta::overlay::{Overlays, VirtualFk};
use hacivat_meta::search_path::SearchPath;
use hacivat_meta::snapshot::{
    Catalog, CatalogChange, CatalogSnapshot, ContainerKey, LoadState, ServerFacts,
};
use hacivat_meta::system::{SystemCatalogs, miss_is_fact};
use hacivat_meta::view::{Lookup, ModelView, SnapshotView};
use hacivat_proto::DbmsKind;
use sql_catalog::view::CatalogView;

// ## m4-t33c-facts (named exception)
/// Prints one `HV-FACT …` line (`format!` arguments) straight to the process's stdout, on one
/// line: libtest captures only the `print!` family, so the line reaches the CI log of a passing
/// test too (R-W3-14). Lines are catalog facts, never a secret.
macro_rules! fact {
    ($($arg:tt)*) => {{
        use std::io::Write as _;
        let line = format!($($arg)*).replace('\n', " ");
        let mut out = std::io::stdout().lock();
        let _ = writeln!(out, "{line}");
        let _ = out.flush();
    }};
}

fn name(s: &str) -> Name {
    Arc::from(s)
}

fn loaded(level: Level) -> LoadState {
    LoadState::Loaded {
        level,
        at: SystemTime::UNIX_EPOCH,
        watermark: Watermark::None,
        stale: false,
    }
}

fn shipped() -> SystemCatalogs {
    let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../assets/system-catalogs");
    SystemCatalogs::open(&dir).unwrap()
}

fn parts(dotted: &str) -> Vec<NamePart> {
    dotted
        .split('.')
        .map(|t| NamePart {
            text: name(t),
            quoted: false,
        })
        .collect()
}

fn k(ctx: &IdentCtx, text: &str, class: NameClass) -> IdentKey {
    match_key(ctx.dialect, ctx, text, false, class).expect("a pinned key")
}

fn snapshot(
    dbms: DbmsKind,
    ctx: IdentCtx,
    version: &str,
    nss: Vec<(NamespaceSnapshot, LoadState)>,
    lists: Vec<ContainerKey>,
) -> Arc<CatalogSnapshot> {
    let mut facts = ServerFacts::new(dbms, ctx);
    facts.version = name(version);
    let catalog = Catalog::new(Arc::new(facts));
    let publisher = catalog.publisher().unwrap();
    let mut changes = Vec::new();
    for (ns, state) in nss {
        changes.push(CatalogChange::SetState(
            ContainerKey::Namespace(ns.key().clone()),
            state,
        ));
        changes.push(CatalogChange::Put(Arc::new(ns)));
    }
    for l in lists {
        changes.push(CatalogChange::SetState(l, loaded(Level::Names)));
    }
    publisher.publish(changes).unwrap();
    catalog.current()
}

fn not_missing<T: std::fmt::Debug>(what: &str, l: &Lookup<T>) -> Option<String> {
    matches!(l, Lookup::Missing(_)).then(|| format!("{what}: false Missing ({l:?})"))
}

/// H1. MySQL (lctn = 0, a patch the 8.4 snapshot covers): `information_schema` names are
/// case-insensitive on every MySQL server, whatever `lower_case_table_names` is; the most
/// common catalog query there is, `SELECT … FROM information_schema.tables`, must never be
/// told the table does not exist.
#[test]
fn h1_mysql_information_schema_any_case_is_never_missing() {
    let ctx = IdentCtx::new(DialectId::MySql);
    let shop = ns_mysql("shop", 0).table("orders", &["id"]).finish();
    let snap = snapshot(
        DbmsKind::MySql,
        ctx.clone(),
        "8.4.3",
        vec![(shop, loaded(Level::Structure))],
        vec![ContainerKey::schema_list(None)],
    );
    let view = SnapshotView::new(snap)
        .with_layers(Arc::new(shipped().overlays_for(DbmsKind::MySql, "8.4.3")))
        .with_search_path(SearchPath::for_mysql(Some(&name("shop"))));
    let mut bad = Vec::new();
    for (s, t) in [
        ("information_schema", "TABLES"),
        ("information_schema", "tables"),
        ("INFORMATION_SCHEMA", "TABLES"),
        ("Information_Schema", "Columns"),
    ] {
        bad.extend(not_missing(
            &format!("ModelView {s}.{t}"),
            &view.lookup(&parts(&format!("{s}.{t}")), KindGroup::Relations),
        ));
        bad.extend(not_missing(
            &format!("CatalogView relation({s}, {t})"),
            &view.relation(
                &k(&ctx, s, NameClass::Schema),
                &k(&ctx, t, NameClass::Table),
            ),
        ));
    }
    assert!(bad.is_empty(), "{bad:#?}");
    // Fix round 1 (I-1): every case is found.
    for (s, t) in [
        ("information_schema", "tables"),
        ("INFORMATION_SCHEMA", "TABLES"),
        ("Information_Schema", "Columns"),
    ] {
        assert!(
            matches!(
                view.lookup(&parts(&format!("{s}.{t}")), KindGroup::Relations),
                Lookup::Found(_)
            ),
            "{s}.{t}"
        );
        assert!(
            matches!(
                view.relation(
                    &k(&ctx, s, NameClass::Schema),
                    &k(&ctx, t, NameClass::Table)
                ),
                Lookup::Found(_)
            ),
            "relation({s}, {t})"
        );
    }
}

/// H2. PostgreSQL: an object in `pg_catalog` with an OID >= 16384 (an extension whose control
/// file says `schema = pg_catalog`, e.g. adminpack or a PL handler; a managed service's
/// additions) is not in any snapshot. The live catalog here even holds it, loaded at
/// `Structure`; the view must not answer `Missing`.
#[test]
fn h2_pg_catalog_object_the_snapshot_lacks_is_not_missing_when_live_holds_it() {
    let ctx = IdentCtx {
        encoding: Some(Arc::from("UTF8")),
        ..IdentCtx::new(DialectId::Postgres)
    };
    let public = ns_pg("app", "public").table("orders", &["id"]).finish();
    let pgc = ns_pg("app", "pg_catalog")
        .table("hv_ext_tbl", &["id"])
        .finish();
    let snap = snapshot(
        DbmsKind::Postgres,
        ctx,
        "17.2",
        vec![
            (public, loaded(Level::Structure)),
            (pgc, loaded(Level::Structure)),
        ],
        vec![ContainerKey::schema_list(Some(&name("app")))],
    );
    // Without the layer the live, complete pg_catalog answers Found.
    let plain = SnapshotView::new(snap.clone()).with_search_path(SearchPath::for_pg(
        &name("app"),
        &[name("public")],
        None,
        None,
    ));
    assert!(matches!(
        plain.lookup(&parts("pg_catalog.hv_ext_tbl"), KindGroup::Relations),
        Lookup::Found(_)
    ));
    let view = SnapshotView::new(snap)
        .with_layers(Arc::new(shipped().overlays_for(DbmsKind::Postgres, "17.2")))
        .with_search_path(SearchPath::for_pg(
            &name("app"),
            &[name("public")],
            None,
            None,
        ));
    let mut bad = Vec::new();
    bad.extend(not_missing(
        "qualified pg_catalog.hv_ext_tbl",
        &view.lookup(&parts("pg_catalog.hv_ext_tbl"), KindGroup::Relations),
    ));
    bad.extend(not_missing(
        "unqualified hv_ext_tbl",
        &view.lookup(&parts("hv_ext_tbl"), KindGroup::Relations),
    ));
    assert!(bad.is_empty(), "{bad:#?}");
    // Fix round 1 (R-W5-6): the live, loaded pg_catalog arena answers after the snapshot miss.
    for path in ["pg_catalog.hv_ext_tbl", "hv_ext_tbl"] {
        assert!(
            matches!(
                view.lookup(&parts(path), KindGroup::Relations),
                Lookup::Found(_)
            ),
            "{path}"
        );
    }
}

/// H3. `settings()` against a MySQL patch newer than the snapshot's: a patch release may add
/// a system variable, so the snapshot's list is not the server's complete list.
#[test]
fn h3_settings_list_is_not_complete_for_a_newer_mysql_patch() {
    let ctx = IdentCtx::new(DialectId::MySql);
    let shop = ns_mysql("shop", 0).table("orders", &["id"]).finish();
    let snap = snapshot(
        DbmsKind::MySql,
        ctx,
        "8.4.99",
        vec![(shop, loaded(Level::Structure))],
        vec![ContainerKey::schema_list(None)],
    );
    let view = SnapshotView::new(snap)
        .with_layers(Arc::new(shipped().overlays_for(DbmsKind::MySql, "8.4.99")))
        .with_search_path(SearchPath::for_mysql(Some(&name("shop"))));
    let s = view.settings();
    assert!(
        !matches!(s, Lookup::Found(_)),
        "settings() on 8.4.99 against the 8.4 snapshot answered a complete list"
    );
}

/// H4. A user virtual FK keyed by a PostgreSQL OID must not attach to another database's
/// relation that happens to have the same OID (OIDs are per database).
#[test]
fn h4_user_virtual_fk_does_not_cross_databases_by_oid() {
    let ctx = IdentCtx {
        encoding: Some(Arc::from("UTF8")),
        ..IdentCtx::new(DialectId::Postgres)
    };
    let app = ns_pg("app", "public")
        .table_oid("orders", 100, &["id", "customer_id"])
        .table_oid("customers", 200, &["id"])
        .finish();
    let other = ns_pg("other", "public")
        .table_oid("invoices", 100, &["id", "customer_id"])
        .finish();
    let snap = snapshot(
        DbmsKind::Postgres,
        ctx,
        "17.2",
        vec![
            (app, loaded(Level::Structure)),
            (other, loaded(Level::Structure)),
        ],
        vec![
            ContainerKey::schema_list(Some(&name("app"))),
            ContainerKey::schema_list(Some(&name("other"))),
        ],
    );
    let key = |db: &str, t: &str, oid: u32| ObjectKey {
        kind: ObjectKind::Table,
        path: [name(db), name("public"), name(t)].into_iter().collect(),
        server_id: Some(ServerId::Oid(oid)),
        overload_sig: None,
        raw_kind: None,
    };
    let layers = Overlays {
        virtual_fks: vec![VirtualFk {
            from: key("app", "orders", 100),
            from_cols: vec!["customer_id".into()],
            to: key("app", "customers", 200),
            to_cols: vec!["id".into()],
            name: None,
        }],
        ..Overlays::default()
    };
    let view = SnapshotView::new(snap)
        .with_layers(Arc::new(layers))
        .with_search_path(SearchPath::for_pg(
            &name("other"),
            &[name("public")],
            None,
            None,
        ));
    assert_eq!(view.virtual_fks_of(&key("app", "orders", 100)).len(), 1);
    let leaked = view.virtual_fks_of(&key("other", "invoices", 100));
    assert!(
        leaked.is_empty(),
        "app.public.orders' virtual FK shows on other.public.invoices: {:?}",
        leaked.iter().map(|l| &l.fk.from.path).collect::<Vec<_>>()
    );
}

/// H5. Version-string fuzz into the same-major rule: a miss is a fact only for a release
/// `M.m` of the snapshot's major (PostgreSQL), or the same line at a patch up to the
/// snapshot's (MySQL/MariaDB). Never for dev/beta/rc/alpha, another major, or garbage.
#[test]
fn h5_version_fuzz_never_makes_an_uncertain_miss_a_fact() {
    let pg = DbmsKind::Postgres;
    let tails = [
        "",
        " (Debian 17.2-1.pgdg120+1)",
        "devel",
        "beta1",
        "rc1",
        "alpha",
        ".1",
        ".x",
        "-YB",
        "\n",
        " ",
        "abc",
        ".0.1",
    ];
    let heads = [
        "17",
        "17.0",
        "17.2",
        "17.99",
        "16.9",
        "18.0",
        "170",
        "1.7",
        "017.2",
        "-17.2",
        "+17.2",
        "",
        "x17.2",
        "17.",
        ".17",
        "4294967296.1",
        "17.4294967296",
    ];
    let mut facts = Vec::new();
    for h in heads {
        for t in tails {
            let v = format!("{h}{t}");
            if miss_is_fact(pg, &v, "17.6") {
                facts.push(v);
            }
        }
    }
    fact!("HV-FACT t26-review pg facts: {facts:?}");
    for v in &facts {
        let lower = v.to_ascii_lowercase();
        assert!(
            !["devel", "beta", "rc", "alpha"]
                .iter()
                .any(|w| lower.contains(w)),
            "a pre-release counted as a fact: {v:?}"
        );
        let t = v.trim_start();
        assert!(
            t.starts_with("17.") || t.starts_with("017."),
            "another major counted as a fact: {v:?}"
        );
    }
    fact!("HV-FACT t26-review pg facts: {facts:?}");
    let my = DbmsKind::MySql;
    for v in [
        "8.4.14", "8.4.99", "8.5.0", "9.0.0", "8.4", "8", "8.4.x", "", "8.4.13a",
    ] {
        if miss_is_fact(my, v, "8.4.13") {
            assert!(
                v.starts_with("8.4.") && !v.starts_with("8.4.14") && !v.starts_with("8.4.99"),
                "mysql {v:?} counted as a fact"
            );
        }
    }
}

/// H6. Race: the writer flips the server between PostgreSQL 16 and 17 while readers take
/// generations; every generation pairs the version with the snapshot of its own major
/// (never a pg16 layer under a pg17 server), and a pinned old generation keeps its pair.
#[test]
fn h6_facts_publish_and_lookups_never_see_a_mismatched_layer() {
    let catalog = Arc::new(Catalog::with_system(
        Arc::new(ServerFacts::new(
            DbmsKind::Postgres,
            IdentCtx::new(DialectId::Postgres),
        )),
        Arc::new(shipped()),
    ));
    assert!(catalog.current().overlays.system.is_none());
    let publisher = catalog.publisher().unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    let readers: Vec<_> = (0..4)
        .map(|_| {
            let c = Arc::clone(&catalog);
            let stop = Arc::clone(&stop);
            std::thread::spawn(move || {
                let mut seen = 0u64;
                while !stop.load(Ordering::Relaxed) {
                    let s = c.current();
                    let label = s.overlays.system_label().map(|l| l.to_string());
                    let major = s.server.version.split('.').next().unwrap_or("").to_string();
                    match label {
                        Some(l) => {
                            assert_eq!(l, major, "layer {l} under server {}", s.server.version)
                        }
                        None => assert!(
                            s.server.version.is_empty(),
                            "no layer under {}",
                            s.server.version
                        ),
                    }
                    seen += 1;
                }
                seen
            })
        })
        .collect();
    let mut pinned = None;
    for i in 0..2000u32 {
        let v = if i % 2 == 0 { "16.4" } else { "17.2" };
        let mut f = ServerFacts::new(DbmsKind::Postgres, IdentCtx::new(DialectId::Postgres));
        f.version = name(v);
        publisher
            .publish(vec![CatalogChange::SetServer(Arc::new(f))])
            .unwrap();
        if i == 0 {
            pinned = Some(catalog.current());
        }
    }
    stop.store(true, Ordering::Relaxed);
    let total: u64 = readers.into_iter().map(|r| r.join().unwrap()).sum();
    let pinned = pinned.unwrap();
    assert_eq!(&*pinned.server.version, "16.4");
    assert_eq!(pinned.overlays.system_label().map(|l| &**l), Some("16"));
    // A pg17-only view, through the pinned pg16 generation: not found there, and certain.
    let view = SnapshotView::new(pinned).with_search_path(SearchPath::for_pg(
        &name("app"),
        &[name("public")],
        None,
        None,
    ));
    fact!(
        "HV-FACT t26-review race reads={total} pg16 pg_stat_checkpointer={:?}",
        matches!(
            view.lookup(
                &parts("pg_catalog.pg_stat_checkpointer"),
                KindGroup::Relations
            ),
            Lookup::Missing(_)
        )
    );
}

/// H7 (measurement, no assert): first-use decode cost, which `Publisher::apply` pays inside
/// the publish that carries the connect's `SetServer`.
#[test]
fn h7_first_decode_cost() {
    let set = shipped();
    for (d, v) in [
        (DbmsKind::Postgres, "17.2"),
        (DbmsKind::Postgres, "18.1"),
        (DbmsKind::MySql, "8.4.3"),
    ] {
        let t = std::time::Instant::now();
        assert!(set.for_server(d, v).is_some());
        let first = t.elapsed();
        let t = std::time::Instant::now();
        let _ = set.for_server(d, v);
        fact!(
            "HV-FACT t26-review decode {d:?} {v}: first={first:?} again={:?}",
            t.elapsed()
        );
    }
}

/// H8 (measurement, no assert): the first lookup into `pg_catalog` of a database re-keys the
/// snapshot arena on the caller's thread (`LayerCache::system_arena`).
#[test]
fn h8_first_rekey_cost() {
    let set = shipped();
    let _ = set.for_server(DbmsKind::Postgres, "17.2");
    let mut facts = ServerFacts::new(
        DbmsKind::Postgres,
        IdentCtx {
            encoding: Some(Arc::from("UTF8")),
            ..IdentCtx::new(DialectId::Postgres)
        },
    );
    facts.version = name("17.2");
    let catalog = Catalog::with_system(Arc::new(facts), Arc::new(set));
    for db in ["app", "other", "app"] {
        let view = SnapshotView::new(catalog.current()).with_search_path(SearchPath::for_pg(
            &name(db),
            &[name("public")],
            None,
            None,
        ));
        let t = std::time::Instant::now();
        let hit = view.lookup(&parts("pg_catalog.pg_class"), KindGroup::Relations);
        let first = t.elapsed();
        let t = std::time::Instant::now();
        let hit2 = view.lookup(&parts("information_schema.tables"), KindGroup::Relations);
        fact!(
            "HV-FACT t26-review rekey db={db} pg_catalog first={first:?} ({}) information_schema first={:?} ({})",
            matches!(hit, Lookup::Found(_)),
            t.elapsed(),
            matches!(hit2, Lookup::Found(_)),
        );
    }
}
