//! Facet loading and fragment introspection on a caller's session (M4 Task 15b; R-M4-29,
//! R-M4-30): the engine's `load_facet` over a scripted introspector, the catalog's facet and
//! source side tables, and `fragment::on_session` on a real SQLite file (no Docker).

#![cfg(feature = "session")]
#![allow(
    clippy::result_large_err,
    reason = "the seam's `IntroError` carries the server's message (T1c's type)"
)]

mod common;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use common::*;
use futures::future::{BoxFuture, join_all};
use hacivat_driver::{Driver, DriverConnect, Endpoint, EngineCatalog, InsecureAuth};
use hacivat_introspect::ddl_effects::Target;
use hacivat_introspect::engine::{
    Engine, EngineConfig, EngineLimits, Filters, IntroHandle, LevelPolicy, MetaSource,
};
use hacivat_introspect::filter::CompiledFilter;
use hacivat_introspect::freshness::NsCheck;
use hacivat_introspect::introspector::{
    FacetRows, IntroCaps, IntroError, Introspector, ServerIdentity, Staleness,
};
use hacivat_introspect::pack::Mode;
use hacivat_introspect::scope::{CurrentCtx, NsInfo, parse_scope};
use hacivat_introspect::testing::FakeIntrospector;
use hacivat_introspect::testing::fake::{FakeCall, FakeObject};
use hacivat_meta::merge::{NamespaceResult, Trigger};
use hacivat_meta::name::FoldRule;
use hacivat_meta::name::IdentCtx;
use hacivat_meta::namespace::NsMeta;
use hacivat_meta::props::{DepsFacet, DepsKind, FacetKind, Grant, GrantsFacet, Props, ViewProps};
use hacivat_meta::snapshot::{
    CatalogChange, CatalogEvent, ContainerKey, FacetValue, IntroErrorCode, LoadState,
};
use hacivat_meta::{
    Catalog, Level, NamespaceSnapshot, NsKey, Obj, ObjectKey, ObjectKind, Ref, ServerFacts,
    ServerId, SourceRef, StateMark, Subtree,
};
use hacivat_net::ssh::{KnownHosts, TunnelManager};
use hacivat_proto::{DbmsKind, Department, ProfileId};
use hacivat_secrets::{
    AuthProviderId, CredentialFiles, MemoryBackend, Retention, SecretBackend, SecretStore,
};
use hacivat_session::budget_for;
use hacivat_session::meta::{MetaConn, MetaConnSpec, MetaError};
use hacivat_session::pipeline::{ApprovedRoots, ConnectDeps, Interaction};
use hacivat_session::profile::{Profile, ProfileEndpoint};
use hacivat_session::session::{
    ExecOrigin, JobKind, Request, SessionHandle, SessionKind, SessionManager, SessionSpec,
};
use sql_dialect::DialectId;
use tokio::sync::Semaphore;
use tokio_util::sync::CancellationToken;

// ---------------------------------------------------------------------------------------------
// A scripted facet reader over `FakeIntrospector`

type Answer =
    Box<dyn Fn(&NsKey, &[ObjectKey], FacetKind) -> Result<FacetRows, IntroError> + Send + Sync>;

/// One facet read the engine asked for.
type Call = (NsKey, Vec<ObjectKey>, FacetKind);

/// `FakeIntrospector` with facets: `facets` are its `IntroCaps::facets`, `answer` scripts
/// every facet read, `calls` records them, and `gate` (when set) holds each read until a
/// permit is added.
#[derive(Clone)]
struct FacetFake {
    inner: FakeIntrospector,
    facets: Vec<FacetKind>,
    answer: Arc<Mutex<Answer>>,
    calls: Arc<Mutex<Vec<Call>>>,
    entered: Arc<AtomicUsize>,
    gate: Arc<Mutex<Option<Arc<Semaphore>>>>,
}

impl FacetFake {
    fn new(inner: FakeIntrospector, facets: &[FacetKind]) -> Self {
        FacetFake {
            inner,
            facets: facets.to_vec(),
            answer: Arc::new(Mutex::new(Box::new(|_, _, _| {
                Err(IntroError::NotImplemented)
            }))),
            calls: Arc::default(),
            entered: Arc::default(),
            gate: Arc::default(),
        }
    }

    fn answer(
        &self,
        f: impl Fn(&NsKey, &[ObjectKey], FacetKind) -> Result<FacetRows, IntroError>
        + Send
        + Sync
        + 'static,
    ) {
        *self.answer.lock().unwrap_or_else(PoisonError::into_inner) = Box::new(f);
    }

    fn calls(&self) -> Vec<Call> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Holds every later facet read until [`Self::open`].
    fn hold(&self) -> Arc<Semaphore> {
        let s = Arc::new(Semaphore::new(0));
        *self.gate.lock().unwrap_or_else(PoisonError::into_inner) = Some(s.clone());
        s
    }
}

impl Introspector for FacetFake {
    fn list_namespaces<'a>(
        &'a self,
        c: &'a mut MetaConn,
    ) -> BoxFuture<'a, Result<Vec<NsInfo>, IntroError>> {
        self.inner.list_namespaces(c)
    }

    fn introspect<'a>(
        &'a self,
        c: &'a mut MetaConn,
        ns: &'a NsKey,
        mode: Mode,
        level: Level,
        filter: &'a CompiledFilter,
        prev: Option<&'a NamespaceSnapshot>,
        cancel: &'a CancellationToken,
    ) -> BoxFuture<'a, Result<NamespaceResult, IntroError>> {
        self.inner
            .introspect(c, ns, mode, level, filter, prev, cancel)
    }

    fn fragment<'a>(
        &'a self,
        c: &'a mut MetaConn,
        ns: &'a NsKey,
        keys: &'a [ObjectKey],
        level: Level,
        filter: &'a CompiledFilter,
        cancel: &'a CancellationToken,
    ) -> BoxFuture<'a, Result<NamespaceResult, IntroError>> {
        self.inner.fragment(c, ns, keys, level, filter, cancel)
    }

    fn stamps<'a>(
        &'a self,
        c: &'a mut MetaConn,
        keys: &'a [ObjectKey],
    ) -> BoxFuture<'a, Result<Vec<Option<StateMark>>, IntroError>> {
        self.inner.stamps(c, keys)
    }

    fn facet<'a>(
        &'a self,
        _c: &'a mut MetaConn,
        ns: &'a NsKey,
        keys: &'a [ObjectKey],
        kind: FacetKind,
    ) -> BoxFuture<'a, Result<FacetRows, IntroError>> {
        Box::pin(async move {
            self.calls
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push((ns.clone(), keys.to_vec(), kind));
            self.entered.fetch_add(1, Ordering::SeqCst);
            let gate = self
                .gate
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone();
            if let Some(g) = gate {
                g.acquire().await.expect("the gate stays open").forget();
            }
            (self.answer.lock().unwrap_or_else(PoisonError::into_inner))(ns, keys, kind)
        })
    }

    fn check_state<'a>(
        &'a self,
        c: &'a mut MetaConn,
        keys: &'a [ObjectKey],
    ) -> BoxFuture<'a, Result<Vec<Staleness>, IntroError>> {
        self.inner.check_state(c, keys)
    }

    fn check_namespace<'a>(
        &'a self,
        c: &'a mut MetaConn,
        ns: &'a NsKey,
        snap: &'a NamespaceSnapshot,
    ) -> BoxFuture<'a, Result<NsCheck, IntroError>> {
        self.inner.check_namespace(c, ns, snap)
    }

    fn capabilities(&self) -> IntroCaps {
        IntroCaps {
            facets: self.facets.clone(),
            ..self.inner.capabilities()
        }
    }

    fn cached_check(&self, snap: &NamespaceSnapshot) -> Option<NsCheck> {
        self.inner.cached_check(snap)
    }

    fn identity<'a>(
        &'a self,
        c: &'a mut MetaConn,
    ) -> BoxFuture<'a, Result<ServerIdentity, IntroError>> {
        self.inner.identity(c)
    }

    fn server_facts<'a>(
        &'a self,
        c: &'a mut MetaConn,
    ) -> BoxFuture<'a, Result<Option<ServerFacts>, IntroError>> {
        self.inner.server_facts(c)
    }
}

/// The PostgreSQL facets (`pg::facets::FACETS`: no `Sources`, a level there).
const PG_FACETS: [FacetKind; 3] = [
    FacetKind::Grants,
    FacetKind::Dependencies,
    FacetKind::Statistics,
];

struct FacetRig {
    fake: FacetFake,
    catalog: Arc<Catalog>,
    handle: IntroHandle,
}

/// An engine over `fake` (PostgreSQL, scope `@:*`).
fn facet_rig(fake: FacetFake) -> FacetRig {
    let meta = hacivat_introspect::testing::fake::FakeMetaSource::new(DbmsKind::Postgres);
    fake.inner.link_meta(meta.clone());
    let catalog = Arc::new(Catalog::new(Arc::new(ServerFacts::new(
        DbmsKind::Postgres,
        IdentCtx::new(DialectId::Postgres),
    ))));
    let handle = Engine::start(EngineConfig {
        profile: ProfileId::new(),
        dbms: DbmsKind::Postgres,
        meta,
        introspector: Arc::new(fake.clone()),
        catalog: catalog.clone(),
        catalog_publisher: catalog.publisher().expect("the only writer"),
        cache: None,
        levels: LevelPolicy::default(),
        scope: parse_scope("@:*", DialectId::Postgres).expect("a scope"),
        current: pg_current("db", "s"),
        filters: Filters::default(),
        validate_every: None,
        limits: EngineLimits::default(),
        runtime: tokio::runtime::Handle::current(),
    });
    FacetRig {
        fake,
        catalog,
        handle,
    }
}

fn s() -> NsKey {
    pg_ns("db", "s")
}

/// A rig whose schema `db.s` holds tables `t` (oid 100) and `u` (oid 101), loaded at
/// `Structure`.
async fn loaded(facets: &[FacetKind]) -> FacetRig {
    let inner = FakeIntrospector::new(DbmsKind::Postgres);
    inner.put_ns(
        s(),
        vec![
            FakeObject::table("t", &["a", "b"]).with_oid(100),
            FakeObject::table("u", &["x"]).with_oid(101),
        ],
    );
    let r = facet_rig(FacetFake::new(inner, facets));
    bounded(r.handle.expand(s())).await.expect("expanded");
    let catalog = r.catalog.clone();
    wait_until(Duration::from_secs(60), move || {
        matches!(
            catalog.current().state(&ContainerKey::Namespace(s())),
            LoadState::Loaded {
                level: Level::Structure,
                ..
            }
        )
    })
    .await;
    idle(&r.handle).await;
    r
}

fn t_key() -> ObjectKey {
    table_key(&s(), "t", Some(100))
}

fn u_key() -> ObjectKey {
    table_key(&s(), "u", Some(101))
}

fn grant(grantee: &str, privilege: &str) -> GrantsFacet {
    GrantsFacet {
        entries: vec![Grant {
            grantee: name(grantee),
            privilege: name(privilege),
            columns: Vec::new(),
            grantable: false,
        }],
    }
}

fn grants_of(r: &FacetRig, k: &ObjectKey) -> (LoadState, Option<GrantsFacet>) {
    let snap = r.catalog.current();
    match snap.facet(k, FacetKind::Grants) {
        (state, Some(FacetValue::Grants(g))) => (state, Some(GrantsFacet::clone(g))),
        (state, _) => (state, None),
    }
}

fn is_loaded(s: &LoadState) -> bool {
    matches!(s, LoadState::Loaded { stale: false, .. })
}

// ---------------------------------------------------------------------------------------------
// load_facet

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn load_facet_publishes_grants_without_touching_structure() {
    let r = loaded(&PG_FACETS).await;
    r.fake.answer(|_, keys, kind| {
        assert_eq!(kind, FacetKind::Grants);
        Ok(FacetRows::Grants(
            keys.iter()
                .map(|k| (k.clone(), grant("alice", "SELECT")))
                .collect(),
        ))
    });
    let before = r.catalog.current();
    let arena = before.namespaces.get(&s()).expect("loaded").clone();
    let mut events = r.catalog.subscribe();
    let g = bounded(r.handle.load_facet(vec![t_key()], FacetKind::Grants))
        .await
        .expect("loaded");
    let after = r.catalog.current();
    assert_eq!(g, before.generation + 1, "one side-table publish");
    assert_eq!(after.generation, g);
    // No arena moved: the namespace is the same `Arc`, still stamped with its own generation.
    let now = after.namespaces.get(&s()).expect("still there");
    assert!(
        Arc::ptr_eq(&arena, now),
        "the structure was not republished"
    );
    assert_eq!(now.generation(), arena.generation());
    assert_eq!(
        after.states.get(&ContainerKey::Namespace(s())),
        before.states.get(&ContainerKey::Namespace(s())),
        "the namespace's own state is unchanged"
    );
    let (state, value) = grants_of(&r, &t_key());
    assert!(is_loaded(&state), "{state:?}");
    assert_eq!(value, Some(grant("alice", "SELECT")));
    // What a DDL producer reads.
    let f = after.grants_facet(&t_key());
    assert!(is_loaded(&f.state));
    assert_eq!(f.value.as_deref(), Some(&grant("alice", "SELECT")));
    // Only the asked object: `u` stays not loaded.
    assert_eq!(grants_of(&r, &u_key()), (LoadState::NotLoaded, None));
    // Subscribers hear one publish with an empty delta and no replaced namespace.
    let mut heard = Vec::new();
    while let Ok(e) = events.try_recv() {
        heard.push(e);
    }
    assert_eq!(heard.len(), 1, "{heard:?}");
    match &heard[0] {
        CatalogEvent::Published { generation, delta } => {
            assert_eq!(*generation, g);
            assert!(delta.is_empty(), "{delta:?}");
        }
        e => panic!("{e:?}"),
    }
    // One read, of the object's own key (with its oid), on its namespace.
    assert_eq!(
        r.fake.calls(),
        vec![(s(), vec![t_key()], FacetKind::Grants)]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn load_facet_is_single_flight_per_object_and_facet() {
    let r = loaded(&PG_FACETS).await;
    r.fake.answer(|_, keys, kind| match kind {
        FacetKind::Grants => Ok(FacetRows::Grants(
            keys.iter()
                .map(|k| (k.clone(), grant("alice", "SELECT")))
                .collect(),
        )),
        _ => Ok(FacetRows::Dependencies(
            keys.iter()
                .map(|k| (k.clone(), DepsFacet::default()))
                .collect(),
        )),
    });
    let gate = r.fake.hold();
    let first = tokio::spawn(r.handle.load_facet(vec![t_key()], FacetKind::Grants));
    let entered = r.fake.entered.clone();
    wait_until(Duration::from_secs(60), move || {
        entered.load(Ordering::SeqCst) == 1
    })
    .await;
    // Two more asks while the read runs: they join it (a key spelled without its oid too).
    let more = [
        r.handle.load_facet(vec![t_key()], FacetKind::Grants),
        r.handle
            .load_facet(vec![table_key(&s(), "t", None)], FacetKind::Grants),
    ];
    gate.add_permits(1);
    let g = bounded(first).await.expect("joined").expect("loaded");
    for got in bounded(join_all(more)).await {
        assert_eq!(got, Ok(g), "the joiners resolve with the same generation");
    }
    assert_eq!(r.fake.calls().len(), 1, "{:?}", r.fake.calls());
    // Loaded and fresh: answered at once, with nothing read and nothing published.
    gate.add_permits(16);
    assert_eq!(
        bounded(r.handle.load_facet(vec![t_key()], FacetKind::Grants)).await,
        Ok(g)
    );
    assert_eq!(r.catalog.current().generation, g);
    assert_eq!(r.fake.calls().len(), 1);
    // Another facet of the same object is another read.
    bounded(r.handle.load_facet(vec![t_key()], FacetKind::Dependencies))
        .await
        .expect("loaded");
    assert_eq!(
        r.fake.calls()[1..],
        [(s(), vec![t_key()], FacetKind::Dependencies)]
    );
    // A key no arena holds is never read.
    let before = r.fake.calls().len();
    let gone = table_key(&s(), "nope", Some(999));
    assert!(
        bounded(r.handle.load_facet(vec![gone.clone()], FacetKind::Grants))
            .await
            .is_ok()
    );
    assert_eq!(r.fake.calls().len(), before);
    assert_eq!(grants_of(&r, &gone), (LoadState::NotLoaded, None));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn partial_facet_is_published_as_partial() {
    // R-W4-3: a read that could vouch for only part of the keys publishes that part loaded and
    // leaves the rest not loaded (never an empty facet that looks complete).
    let r = loaded(&PG_FACETS).await;
    r.fake.answer(|_, keys, _| {
        Ok(FacetRows::Grants(
            keys.iter()
                .filter(|k| k.name() == "t")
                .map(|k| (k.clone(), grant("alice", "SELECT")))
                .collect(),
        ))
    });
    bounded(
        r.handle
            .load_facet(vec![t_key(), u_key()], FacetKind::Grants),
    )
    .await
    .expect("read");
    assert_eq!(
        r.fake.calls(),
        vec![(s(), vec![t_key(), u_key()], FacetKind::Grants)],
        "one read for both keys"
    );
    assert!(is_loaded(&grants_of(&r, &t_key()).0));
    assert_eq!(grants_of(&r, &u_key()), (LoadState::NotLoaded, None));
    let snap = r.catalog.current();
    assert_eq!(snap.grants_facet(&u_key()).state, LoadState::NotLoaded);
    assert!(snap.grants_facet(&u_key()).value.is_none());

    // A refused read (42501) is published `Failed` for every key it was asked, with no value.
    r.fake.answer(|_, _, _| {
        Err(IntroError::Permission {
            kind: ObjectKind::Table,
        })
    });
    assert_eq!(
        bounded(
            r.handle
                .load_facet(vec![t_key(), u_key()], FacetKind::Dependencies)
        )
        .await,
        Err(IntroError::Permission {
            kind: ObjectKind::Table
        })
    );
    let snap = r.catalog.current();
    for k in [t_key(), u_key()] {
        assert!(
            matches!(
                snap.facet(&k, FacetKind::Dependencies),
                (
                    LoadState::Failed {
                        error: IntroErrorCode::PermissionDenied,
                        ..
                    },
                    None
                )
            ),
            "{:?}",
            snap.facet(&k, FacetKind::Dependencies)
        );
    }
    // The grants already loaded are untouched.
    assert!(is_loaded(&grants_of(&r, &t_key()).0));

    // A facet the introspector does not read is `Unsupported`, and nothing is asked of it.
    let calls = r.fake.calls().len();
    let r2 = loaded(&[FacetKind::Grants]).await;
    assert_eq!(
        bounded(r2.handle.load_facet(vec![t_key()], FacetKind::Statistics)).await,
        Err(IntroError::NotImplemented)
    );
    assert!(r2.fake.calls().is_empty());
    assert!(matches!(
        r2.catalog
            .current()
            .facet(&t_key(), FacetKind::Statistics)
            .0,
        LoadState::Failed {
            error: IntroErrorCode::Unsupported,
            ..
        }
    ));
    // So is one it answers `NotImplemented` for (MariaDB and SQLite dependencies).
    r.fake.answer(|_, _, _| Err(IntroError::NotImplemented));
    assert_eq!(
        bounded(r.handle.load_facet(vec![u_key()], FacetKind::Statistics)).await,
        Err(IntroError::NotImplemented)
    );
    assert_eq!(r.fake.calls().len(), calls + 1);
    assert!(matches!(
        r.catalog.current().facet(&u_key(), FacetKind::Statistics).0,
        LoadState::Failed {
            error: IntroErrorCode::Unsupported,
            ..
        }
    ));
    // A transient failure decides nothing: no state is published.
    let g = r.catalog.current().generation;
    r.fake
        .answer(|_, _, _| Err(IntroError::Meta(MetaError::ConnectionLost)));
    assert!(
        bounded(r.handle.load_facet(vec![t_key()], FacetKind::Statistics))
            .await
            .is_err()
    );
    assert_eq!(r.catalog.current().generation, g);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_re_read_drops_the_objects_facets_and_the_dependencies_that_name_it() {
    let r = loaded(&PG_FACETS).await;
    let t_ref = Ref {
        kind: ObjectKind::Table,
        path: t_key().path,
        server_id: Some(ServerId::Oid(100)),
    };
    r.fake.answer(move |_, keys, kind| match kind {
        FacetKind::Grants => Ok(FacetRows::Grants(
            keys.iter()
                .map(|k| (k.clone(), grant("alice", "SELECT")))
                .collect(),
        )),
        _ => Ok(FacetRows::Dependencies(
            keys.iter()
                .map(|k| {
                    (
                        k.clone(),
                        DepsFacet {
                            // `u` depends on `t`; `t` on nothing.
                            depends_on: if k.name() == "u" {
                                vec![t_ref.clone()]
                            } else {
                                Vec::new()
                            },
                            kind: DepsKind::Catalog,
                        },
                    )
                })
                .collect(),
        )),
    });
    let both = vec![t_key(), u_key()];
    bounded(r.handle.load_facet(both.clone(), FacetKind::Grants))
        .await
        .unwrap();
    bounded(r.handle.load_facet(both.clone(), FacetKind::Dependencies))
        .await
        .unwrap();
    assert!(is_loaded(
        &r.catalog
            .current()
            .facet(&u_key(), FacetKind::Dependencies)
            .0
    ));
    // `t` changes on the server and is read again.
    r.fake.inner.touch(&s(), "t");
    bounded(r.handle.refresh(
        Target::Objects {
            ns: s(),
            keys: vec![t_key()],
        },
        Trigger::User,
    ))
    .await
    .expect("re-read");
    let snap = r.catalog.current();
    // `t`'s grants and dependencies are dropped; `u`'s grants stay; `u`'s dependencies named
    // `t` (by oid) and are dropped too.
    assert_eq!(grants_of(&r, &t_key()), (LoadState::NotLoaded, None));
    assert_eq!(
        snap.facet(&t_key(), FacetKind::Dependencies).0,
        LoadState::NotLoaded
    );
    assert!(is_loaded(&grants_of(&r, &u_key()).0));
    assert_eq!(
        snap.facet(&u_key(), FacetKind::Dependencies).0,
        LoadState::NotLoaded
    );
    // The next ask reads them again.
    let calls = r.fake.calls().len();
    bounded(r.handle.load_facet(vec![t_key()], FacetKind::Grants))
        .await
        .unwrap();
    assert_eq!(r.fake.calls().len(), calls + 1);
    assert!(is_loaded(&grants_of(&r, &t_key()).0));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sources_on_an_engine_without_a_sources_facet_is_a_fragment_at_sources() {
    let r = loaded(&PG_FACETS).await;
    let before = r.fake.inner.calls().len();
    bounded(r.handle.load_facet(vec![t_key()], FacetKind::Sources))
        .await
        .expect("read");
    let after: Vec<FakeCall> = r.fake.inner.calls()[before..].to_vec();
    assert_eq!(
        after,
        vec![FakeCall::Fragment {
            ns: s(),
            keys: vec![t_key()],
            level: Level::Sources,
        }],
        "a fragment of the key at Sources, even though nothing asked for the level before"
    );
    assert!(r.fake.calls().is_empty(), "no facet read");
}

// ## m4-t15b-facets (fix round 1)

/// Review I-2 (T17b m-6): `Sources` reads are per key. A key the login may not `SHOW CREATE`
/// (MySQL 1142, `Permission`) fails alone: neither the other key of the same request nor
/// another caller's queued request for the namespace is marked denied.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_refused_sources_key_fails_alone() {
    let r = loaded(&[FacetKind::Sources, FacetKind::Grants]).await;
    r.fake.answer(|_, keys, kind| match kind {
        FacetKind::Sources if keys.iter().any(|k| k.name() == "u") => Err(IntroError::Permission {
            kind: ObjectKind::Table,
        }),
        FacetKind::Sources => Ok(FacetRows::Sources(
            keys.iter()
                .map(|k| (k.clone(), name(&format!("CREATE TABLE {}", k.name()))))
                .collect(),
        )),
        _ => Ok(FacetRows::Grants(Vec::new())),
    });
    // One request for both keys: one read per key.
    let got = bounded(
        r.handle
            .load_facet(vec![t_key(), u_key()], FacetKind::Sources),
    )
    .await;
    assert!(got.is_err(), "the refused key's error is reported: {got:?}");
    let calls = r.fake.calls();
    assert_eq!(
        calls,
        vec![
            (s(), vec![t_key()], FacetKind::Sources),
            (s(), vec![u_key()], FacetKind::Sources),
        ],
        "one read per key"
    );
    let snap = r.catalog.current();
    assert_eq!(
        snap.server_text(&t_key()).as_deref(),
        Some("CREATE TABLE t")
    );
    assert!(matches!(
        snap.facet(&u_key(), FacetKind::Sources).0,
        LoadState::Failed {
            error: IntroErrorCode::PermissionDenied,
            ..
        }
    ));

    // Two callers queued behind a running read of the namespace: they never coalesce.
    let r = loaded(&[FacetKind::Sources, FacetKind::Grants]).await;
    r.fake.answer(|_, keys, kind| match kind {
        FacetKind::Sources if keys.iter().any(|k| k.name() == "u") => Err(IntroError::Permission {
            kind: ObjectKind::Table,
        }),
        FacetKind::Sources => Ok(FacetRows::Sources(
            keys.iter()
                .map(|k| (k.clone(), name("CREATE TABLE t")))
                .collect(),
        )),
        _ => Ok(FacetRows::Grants(
            keys.iter()
                .map(|k| (k.clone(), GrantsFacet::default()))
                .collect(),
        )),
    });
    let gate = r.fake.hold();
    let busy = tokio::spawn(r.handle.load_facet(vec![t_key()], FacetKind::Grants));
    let entered = r.fake.entered.clone();
    wait_until(Duration::from_secs(60), move || {
        entered.load(Ordering::SeqCst) == 1
    })
    .await;
    let mine = r.handle.load_facet(vec![t_key()], FacetKind::Sources);
    let theirs = r.handle.load_facet(vec![u_key()], FacetKind::Sources);
    gate.add_permits(16);
    bounded(busy).await.expect("joined").expect("grants");
    assert!(
        bounded(mine).await.is_ok(),
        "the other caller's refusal is not mine"
    );
    assert!(bounded(theirs).await.is_err());
    let snap = r.catalog.current();
    assert_eq!(
        snap.server_text(&t_key()).as_deref(),
        Some("CREATE TABLE t")
    );
    assert!(matches!(
        snap.facet(&u_key(), FacetKind::Sources).0,
        LoadState::Failed { .. }
    ));
}

/// Review m-3: a facet that failed is not read again before its `retry_after`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_failed_facet_waits_for_its_retry_after() {
    let r = loaded(&PG_FACETS).await;
    r.fake.answer(|_, _, _| {
        Err(IntroError::Permission {
            kind: ObjectKind::Table,
        })
    });
    assert!(
        bounded(r.handle.load_facet(vec![t_key()], FacetKind::Grants))
            .await
            .is_err()
    );
    assert_eq!(r.fake.calls().len(), 1);
    let g = r.catalog.current().generation;
    // Asked again at once (a re-render): answered from the catalog, nothing read.
    assert_eq!(
        bounded(r.handle.load_facet(vec![t_key()], FacetKind::Grants)).await,
        Ok(g)
    );
    assert_eq!(r.fake.calls().len(), 1, "the server is not asked again yet");
    assert!(matches!(
        grants_of(&r, &t_key()).0,
        LoadState::Failed {
            error: IntroErrorCode::PermissionDenied,
            ..
        }
    ));
}

/// Review m-2: a grants read that ran while a role was renamed (Task 22 marks loaded grants
/// stale) publishes its answer stale, never fresh with the old grantee names.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_grants_read_overtaken_by_a_role_rename_is_published_stale() {
    let r = loaded(&PG_FACETS).await;
    let srv = NsKey::server();
    r.fake
        .inner
        .put_ns(srv.clone(), vec![FakeObject::role("alice").with_oid(500)]);
    bounded(
        r.handle
            .refresh(Target::Namespace(srv.clone()), Trigger::User),
    )
    .await
    .expect("roles");
    r.fake.answer(|_, keys, _| {
        Ok(FacetRows::Grants(
            keys.iter()
                .map(|k| (k.clone(), grant("alice", "SELECT")))
                .collect(),
        ))
    });
    let gate = r.fake.hold();
    let read = tokio::spawn(r.handle.load_facet(vec![t_key()], FacetKind::Grants));
    let entered = r.fake.entered.clone();
    wait_until(Duration::from_secs(60), move || {
        entered.load(Ordering::SeqCst) == 1
    })
    .await;
    // `ALTER ROLE alice RENAME TO alicia` while the read holds the old name.
    r.fake.inner.edit_ns(&srv, |n| {
        let o = n.objects.iter_mut().find(|o| &*o.name == "alice").unwrap();
        o.name = name("alicia");
        o.version += 10;
    });
    bounded(
        r.handle
            .refresh(Target::Namespace(srv.clone()), Trigger::User),
    )
    .await
    .expect("roles again");
    gate.add_permits(16);
    bounded(read).await.expect("joined").expect("read");
    let snap = r.catalog.current();
    assert!(
        matches!(
            snap.facet(&t_key(), FacetKind::Grants).0,
            LoadState::Loaded { stale: true, .. }
        ),
        "{:?}",
        snap.facet(&t_key(), FacetKind::Grants).0
    );
    // Never handed to a DDL producer; the next ask reads it again.
    assert_eq!(snap.grants_facet(&t_key()).state, LoadState::NotLoaded);
    let calls = r.fake.calls().len();
    bounded(r.handle.load_facet(vec![t_key()], FacetKind::Grants))
        .await
        .expect("again");
    assert_eq!(r.fake.calls().len(), calls + 1);
    assert!(is_loaded(&grants_of(&r, &t_key()).0));
}
// ## m4-t15b-facets

// ---------------------------------------------------------------------------------------------
// The side tables (the catalog's publisher, directly)

#[test]
fn set_facet_is_refused_on_another_container_and_texts_follow_the_arena() {
    let catalog = Catalog::new(Arc::new(ServerFacts::new(
        DbmsKind::Postgres,
        IdentCtx::new(DialectId::Postgres),
    )));
    let p = catalog.publisher().unwrap();
    assert!(
        p.publish(vec![CatalogChange::SetFacet {
            key: ContainerKey::Namespace(s()),
            state: LoadState::NotLoaded,
            value: None,
        }])
        .is_err()
    );
    let wrong = ContainerKey::Facet {
        ns: s(),
        object: t_key(),
        facet: FacetKind::Grants,
    };
    assert!(
        p.publish(vec![CatalogChange::SetFacet {
            key: wrong,
            state: LoadState::NotLoaded,
            value: Some(FacetValue::Sources(name("x"))),
        }])
        .is_err(),
        "a value of another facet"
    );
    assert_eq!(catalog.current().generation, 0, "nothing was published");
    // A text whose hash is not its reference is dropped; a namespace no arena holds keeps
    // none.
    p.publish(vec![CatalogChange::PutSources {
        ns: s(),
        texts: vec![
            (SourceRef::of("SELECT 1"), name("SELECT 1")),
            (SourceRef::of("SELECT 1"), name("SELECT 2")),
        ],
    }])
    .unwrap();
    assert_eq!(
        catalog.current().source_text(&SourceRef::of("SELECT 1")),
        None
    );

    // An arena with a view `v` whose text is `SELECT 1` and a table `t`.
    let text = "SELECT 1";
    let arena = |with_view: bool| {
        let mut tree = Subtree::new(Obj::new(ObjectKind::Schema, "s"));
        let root = tree.root();
        tree.add(
            root,
            Obj::new(ObjectKind::Table, "t").with_server_id(ServerId::Oid(100)),
        );
        if with_view {
            tree.add(
                root,
                Obj::new(ObjectKind::View, "v").with_props(Props::View(Arc::new(ViewProps {
                    source: Some(SourceRef::of(text)),
                    ..ViewProps::default()
                }))),
            );
        }
        Arc::new(
            NamespaceSnapshot::build(
                s(),
                DbmsKind::Postgres,
                FoldRule::Exact,
                NsMeta::default(),
                tree,
            )
            .expect("an arena"),
        )
    };
    p.publish(vec![
        CatalogChange::Put(arena(true)),
        CatalogChange::PutSources {
            ns: s(),
            texts: vec![(SourceRef::of(text), name(text))],
        },
    ])
    .unwrap();
    let snap = catalog.current();
    assert_eq!(
        snap.source_text(&SourceRef::of(text)).as_deref(),
        Some(text)
    );
    // A stale grants facet is never handed to a DDL producer; a fresh one is.
    let key = snap
        .facet_key(&t_key(), FacetKind::Grants)
        .expect("t is held");
    let loaded = |stale| LoadState::Loaded {
        level: Level::Structure,
        at: std::time::SystemTime::UNIX_EPOCH,
        watermark: hacivat_meta::Watermark::None,
        stale,
    };
    for (stale, shown) in [(true, false), (false, true)] {
        p.publish(vec![CatalogChange::SetFacet {
            key: key.clone(),
            state: loaded(stale),
            value: Some(FacetValue::Grants(Arc::new(grant("alice", "SELECT")))),
        }])
        .unwrap();
        let f = catalog.current().grants_facet(&t_key());
        assert_eq!(f.value.is_some(), shown, "stale: {stale}");
        assert_eq!(is_loaded(&f.state), shown, "stale: {stale}");
    }
    // The view leaves the arena: its text is no longer kept; `NotLoaded` removes the facet.
    p.publish(vec![
        CatalogChange::Put(arena(false)),
        CatalogChange::SetFacet {
            key: key.clone(),
            state: LoadState::NotLoaded,
            value: None,
        },
    ])
    .unwrap();
    let snap = catalog.current();
    assert_eq!(snap.source_text(&SourceRef::of(text)), None);
    assert!(snap.facet_value(&key).is_none());
    assert!(!snap.states.contains_key(&key));
    // Removing the namespace removes its side tables.
    p.publish(vec![CatalogChange::SetFacet {
        key: key.clone(),
        state: loaded(false),
        value: Some(FacetValue::Grants(Arc::new(grant("bob", "SELECT")))),
    }])
    .unwrap();
    p.publish(vec![CatalogChange::Remove(s())]).unwrap();
    assert_eq!(catalog.current().loaded_facets().count(), 0);
}

// ---------------------------------------------------------------------------------------------
// The joined sections (R-M4-29, R-W3-9: every rendered section passes Task 5's guard)

fn text_bind(s: &str) -> hacivat_values::BindValue {
    hacivat_values::BindValue::Typed {
        hint: hacivat_values::LogicalType::Text {
            max_chars: None,
            fixed: false,
        },
        text: Arc::from(s),
    }
}

#[test]
fn every_joined_section_renders_joined_and_passes_the_metadata_guard() {
    use hacivat_introspect::fragment::{mysql_name, pg_name};
    use hacivat_introspect::pack::{Pack, PackCtx, PackFlags, Params};
    let hostile = "hvm4_x'); COMMIT; SET search_path = evil; -- \"q\"`";
    let params = Params {
        scalars: vec![(name("schema"), text_bind(hostile))],
        lists: vec![
            (name("oids"), vec![text_bind("555001"), text_bind("555002")]),
            (name("names"), vec![text_bind(hostile), text_bind("t")]),
        ],
    };
    let cases: [(DbmsKind, DialectId, &[&str]); 3] = [
        (DbmsKind::Postgres, DialectId::Postgres, &pg_name::ALL),
        (DbmsKind::MySql, DialectId::MySql, &mysql_name::ALL),
        (DbmsKind::MariaDb, DialectId::MariaDb, &mysql_name::ALL),
    ];
    for (dbms, dialect, sections) in cases {
        let pack = Pack::builtin(dialect).expect("the pack");
        let mut flags = PackFlags::JOINED;
        flags.set(PackFlags::MARIADB, dbms == DbmsKind::MariaDb);
        let ctx = PackCtx {
            dialect,
            version: 0,
            flavor: dbms,
            mode: Mode::Full,
            level: Level::Structure,
            flags,
            params: params.clone(),
        };
        for section in sections {
            assert!(
                pack.section(section).expect("in the pack").joinable(),
                "{dbms:?} {section}"
            );
            let queries = pack.render(section, &ctx).expect("renders joined");
            assert!(!queries.is_empty(), "{dbms:?} {section}");
            for q in queries {
                let sql = q.sql();
                assert!(q.joinable());
                assert_eq!(
                    hacivat_session::meta::check_read_text_for(sql, dbms),
                    Ok(()),
                    "{dbms:?} {section}: the joined session's guard refuses the text"
                );
                assert!(sql.trim_start().starts_with("SELECT"), "{dbms:?} {section}");
                for needle in ["hvm4_", "555001", "COMMIT", "evil"] {
                    assert!(
                        !sql.contains(needle),
                        "{dbms:?} {section}: {needle} spliced"
                    );
                }
            }
        }
        // Only joinable sections render on a joined connection.
        let other = if dbms == DbmsKind::Postgres {
            "RetrieveRelations"
        } else {
            "Tables"
        };
        assert!(pack.render(other, &ctx).is_err(), "{dbms:?} {other}");
    }
}

// ---------------------------------------------------------------------------------------------
// SQLite: real files (no Docker)

fn sqlite_pid() -> ProfileId {
    "6f1c2a4e-1d16-4a16-8a16-0000000015b0".parse().unwrap()
}

fn deps() -> ConnectDeps {
    let backend: Arc<dyn SecretBackend> = Arc::new(MemoryBackend::new());
    let known_hosts = KnownHosts::load(
        Vec::new(),
        PathBuf::from("/nonexistent/hacivat/known_hosts"),
    );
    let tunnels = TunnelManager::new(known_hosts.clone(), Default::default());
    ConnectDeps {
        drivers: Arc::new(hacivat_session::default_drivers()),
        catalog: EngineCatalog::embedded(),
        secrets: Arc::new(SecretStore::new(Some(backend))),
        files: CredentialFiles {
            pgpass: None,
            pg_service: None,
        },
        transport_wrap: None,
        extra_steps: Vec::new(),
        approved_roots: ApprovedRoots::for_user(),
        timeout_override: None,
        tunnels,
        known_hosts,
    }
}

fn sqlite_profile(path: &Path) -> Profile {
    Profile {
        id: sqlite_pid(),
        name: "lite".into(),
        engine: DbmsKind::Sqlite,
        endpoint: ProfileEndpoint::File {
            path: path.to_path_buf(),
            create_if_missing: false,
        },
        user: None,
        auth: AuthProviderId::None,
        retention: Retention::Never,
        ..Profile::default()
    }
}

/// Creates `dir/main.db` with `sql` through the real driver (before any manager).
async fn sqlite_file(dir: &Path, sql: &str) -> PathBuf {
    let path = dir.join("main.db");
    let driver: Arc<dyn Driver> = hacivat_session::default_drivers()
        .resolve(DbmsKind::Sqlite)
        .expect("the SQLite driver");
    let mut conn = driver
        .connect(DriverConnect {
            endpoint: Endpoint::File {
                path: path.clone(),
                create_if_missing: true,
                read_only: false,
            },
            tls: None,
            user: None,
            password: None,
            database: None,
            application_name: "fixture".into(),
            read_only: false,
            options: Vec::new(),
            connect_timeout: Duration::from_secs(10),
            events: Arc::new(|_| {}),
            insecure_auth: InsecureAuth::default(),
        })
        .await
        .expect("the fixture file opens");
    conn.batch(sql).await.expect("the fixture SQL runs");
    conn.close().await;
    path
}

fn manager(profile: &Profile) -> Arc<SessionManager> {
    SessionManager::new(
        tokio::runtime::Handle::current(),
        Arc::new(profile.clone()),
        Arc::new(deps()),
        Arc::new(budget_for(profile)),
    )
}

async fn open(sm: &Arc<SessionManager>, kind: SessionKind) -> SessionHandle {
    let department = match kind {
        SessionKind::Job(..) => Department::Executor,
        _ => Department::Console,
    };
    sm.open(
        SessionSpec {
            kind,
            read_only: false,
            department,
        },
        Interaction::Headless,
    )
    .await
    .unwrap_or_else(|e| panic!("the session opens: {:?}", e.class))
}

async fn exec(h: &SessionHandle, sql: &str) {
    let got = h
        .submit(Request::Exec {
            sql: sql.to_string(),
            opts: hacivat_driver::ExecOpts::default(),
            origin: ExecOrigin::User,
        })
        .reply
        .await
        .expect("the actor replies");
    if let Err(e) = got {
        panic!("{sql}: {:?}", e.kind);
    }
}

/// The facts of a catalog of `dbms` (what `on_session`'s caller holds).
fn engine_facts(dbms: DbmsKind) -> ServerFacts {
    let d = match dbms {
        DbmsKind::MySql => DialectId::MySql,
        DbmsKind::MariaDb => DialectId::MariaDb,
        DbmsKind::Sqlite => DialectId::Sqlite,
        _ => DialectId::Postgres,
    };
    ServerFacts::new(dbms, IdentCtx::new(d))
}

/// Waits until `h` reports a transaction.
async fn in_tx(h: &SessionHandle) {
    let mut st = h.state();
    let _ = tokio::time::timeout(
        Duration::from_secs(10),
        st.wait_for(|s| s.tx.status == hacivat_driver::TxStatus::InTx),
    )
    .await;
}

/// The engine's metadata connections from a session manager.
struct SmMeta {
    sm: Arc<SessionManager>,
    profile: ProfileId,
}

impl MetaSource for SmMeta {
    fn acquire<'a>(
        &'a self,
        database: Option<hacivat_meta::Name>,
        cancel: &'a CancellationToken,
    ) -> BoxFuture<'a, Result<MetaConn, MetaError>> {
        Box::pin(async move {
            self.sm
                .meta_conn(
                    MetaConnSpec {
                        profile: self.profile,
                        database: database.map(|d| d.to_string()),
                        statement_timeout: Duration::from_secs(60),
                    },
                    cancel,
                )
                .await
        })
    }
}

/// An engine over the real SQLite introspector on `sm`.
fn sqlite_engine(
    sm: &Arc<SessionManager>,
    cache: Option<hacivat_introspect::engine::CacheHandle>,
) -> (Arc<Catalog>, IntroHandle) {
    let catalog = Arc::new(Catalog::new(Arc::new(ServerFacts::new(
        DbmsKind::Sqlite,
        IdentCtx::new(DialectId::Sqlite),
    ))));
    let handle = Engine::start(EngineConfig {
        profile: sqlite_pid(),
        dbms: DbmsKind::Sqlite,
        meta: Arc::new(SmMeta {
            sm: sm.clone(),
            profile: sqlite_pid(),
        }),
        introspector: Arc::new(hacivat_introspect::sqlite::SqliteIntrospector::default()),
        catalog: catalog.clone(),
        catalog_publisher: catalog.publisher().expect("the only writer"),
        cache,
        levels: LevelPolicy::default(),
        scope: parse_scope("*", DialectId::Sqlite).expect("a scope"),
        current: CurrentCtx {
            database: None,
            schema: Some(name("main")),
            ident: None,
        },
        filters: Filters::default(),
        validate_every: None,
        limits: EngineLimits {
            save_debounce: Duration::ZERO,
            ..EngineLimits::default()
        },
        runtime: tokio::runtime::Handle::current(),
    });
    (catalog, handle)
}

fn main_ns() -> NsKey {
    NsKey::schema([name("main")])
}

fn sqlite_key(kind: ObjectKind, path: &[&str]) -> ObjectKey {
    ObjectKey {
        kind,
        path: path.iter().map(|p| name(p)).collect(),
        server_id: None,
        overload_sig: None,
        raw_kind: None,
    }
}

fn columns_of(ns: &NamespaceSnapshot, table: &str) -> Vec<String> {
    let Some(t) = ns.find(&sqlite_key(ObjectKind::Table, &["main", table])) else {
        return Vec::new();
    };
    ns.children(t)
        .filter(|(_, o)| o.kind() == ObjectKind::Column)
        .map(|(_, o)| o.name().to_string())
        .collect()
}

async fn sqlite_loaded(catalog: &Arc<Catalog>, handle: &IntroHandle) {
    bounded(handle.expand(main_ns())).await.expect("expanded");
    let c = catalog.clone();
    wait_until(Duration::from_secs(60), move || {
        matches!(
            c.current().state(&ContainerKey::Namespace(main_ns())),
            LoadState::Loaded {
                level: Level::Structure | Level::Sources,
                ..
            }
        )
    })
    .await;
    idle(handle).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn on_session_result_is_never_published_or_cached() {
    let dir = tempfile::tempdir().unwrap();
    let path = sqlite_file(dir.path(), "CREATE TABLE t(a)").await;
    let profile = sqlite_profile(&path);
    let sm = manager(&profile);
    let cache_path = dir.path().join("cache.sqlite");
    let store = hacivat_meta::persist::CacheStore::open(
        &cache_path,
        hacivat_meta::persist::Encryption::None,
        hacivat_meta::persist::WriterId(15),
    )
    .expect("a cache");
    let cache = Arc::new(Mutex::new(store.store));
    let (catalog, handle) = sqlite_engine(&sm, Some(cache.clone()));
    sqlite_loaded(&catalog, &handle).await;
    let published = catalog.current();
    assert_eq!(
        columns_of(published.namespaces.get(&main_ns()).unwrap(), "t"),
        ["a"]
    );
    let cached_before = cache
        .lock()
        .unwrap()
        .load_all()
        .expect("the cache reads")
        .into_changes()
        .len();
    let mut events = catalog.subscribe();

    // The job's transaction: SQLite's one session is the job's (no console is open).
    let job = open(&sm, SessionKind::Job(1, JobKind::DdlApply)).await;
    exec(&job, "BEGIN").await;
    exec(&job, "ALTER TABLE t ADD COLUMN c INTEGER").await;
    in_tx(&job).await;
    let cancel = CancellationToken::new();
    let got = bounded(hacivat_introspect::fragment::on_session(
        &job,
        &engine_facts(DbmsKind::Sqlite),
        &main_ns(),
        &[sqlite_key(ObjectKind::Table, &["main", "t"])],
        Level::Structure,
        &cancel,
    ))
    .await;
    // A SQLite profile has one session: the job asked for gets the console, and T5 refuses to
    // join it (M7 adds the writer seam, `MetaConn::join` docs). So nothing is read here: this
    // test proves only that the catalog and the cache are untouched (review m-5). The positive
    // joined read is covered by `fragment`'s unit test `a_joined_read_is_a_standalone_snapshot_no_publisher_stamped`
    // (generation 0), by the review's scripted PG job probes (they need FakeDriver, a
    // controller manifest amendment) and by the Docker tests (`docker_fragment_on_session`).
    assert!(
        matches!(job.kind(), SessionKind::Console(_)),
        "{:?}",
        job.kind()
    );
    assert_eq!(got.map(drop), Err(IntroError::Meta(MetaError::NotJoinable)));
    exec(&job, "ROLLBACK").await;
    // Nothing reached the catalog or the cache.
    let now = catalog.current();
    assert_eq!(now.generation, published.generation);
    assert!(Arc::ptr_eq(&now, &published));
    assert!(events.try_recv().is_err(), "nothing was announced");
    assert_eq!(
        columns_of(now.namespaces.get(&main_ns()).unwrap(), "t"),
        ["a"]
    );
    handle.shutdown();
    bounded(handle.closed()).await;
    let cached_after = cache
        .lock()
        .unwrap()
        .load_all()
        .expect("the cache reads")
        .into_changes()
        .len();
    assert_eq!(cached_after, cached_before);
    assert!(
        hacivat_introspect::fragment::on_session(
            &open(&sm, SessionKind::Job(2, JobKind::DdlApply)).await,
            &engine_facts(DbmsKind::Sqlite),
            &main_ns(),
            &[],
            Level::Structure,
            &cancel,
        )
        .await
        .is_err(),
        "a job outside a transaction is not joinable"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn on_session_refuses_a_console_session() {
    let dir = tempfile::tempdir().unwrap();
    let path = sqlite_file(dir.path(), "CREATE TABLE t(a)").await;
    let sm = manager(&sqlite_profile(&path));
    let console = open(
        &sm,
        SessionKind::Console(hacivat_session::session::ClientId::DEFAULT),
    )
    .await;
    exec(&console, "BEGIN").await;
    let got = hacivat_introspect::fragment::on_session(
        &console,
        &engine_facts(DbmsKind::Sqlite),
        &main_ns(),
        &[sqlite_key(ObjectKind::Table, &["main", "t"])],
        Level::Structure,
        &CancellationToken::new(),
    )
    .await;
    assert_eq!(got.map(drop), Err(IntroError::Meta(MetaError::NotJoinable)));
    exec(&console, "ROLLBACK").await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sqlite_texts_read_at_sources_are_served_and_follow_the_arena() {
    let dir = tempfile::tempdir().unwrap();
    let path = sqlite_file(
        dir.path(),
        "CREATE TABLE t(a); CREATE VIEW v AS SELECT a FROM t",
    )
    .await;
    let sm = manager(&sqlite_profile(&path));
    let (catalog, handle) = sqlite_engine(&sm, None);
    sqlite_loaded(&catalog, &handle).await;
    let v = sqlite_key(ObjectKind::View, &["main", "v"]);
    // T25's flow: the view's namespace below Sources is elevated with one fragment.
    bounded(handle.elevate(v.clone(), Level::Sources))
        .await
        .expect("elevated");
    let snap = catalog.current();
    let h = snap.find(&v).expect("the view");
    let Props::View(p) = h.obj().props() else {
        panic!("view props");
    };
    let r = p.source.expect("a source reference at Sources");
    let text = snap.source_text(&r).expect("the view's text is served");
    assert_eq!(SourceRef::of(&text), r);
    assert!(text.contains("CREATE VIEW v"), "{text}");

    // A table's server text is the `Sources` facet (SQLite reads it).
    let t = sqlite_key(ObjectKind::Table, &["main", "t"]);
    bounded(handle.load_facet(vec![t.clone(), v.clone()], FacetKind::Sources))
        .await
        .expect("read");
    let snap = catalog.current();
    let table_text = snap.server_text(&t).expect("the table's text");
    assert!(table_text.contains("CREATE TABLE t"), "{table_text}");
    assert!(snap.server_text(&v).is_some());

    // The view is redefined: the next read replaces its text; the old one is no longer kept.
    let console = open(
        &sm,
        SessionKind::Console(hacivat_session::session::ClientId::DEFAULT),
    )
    .await;
    exec(&console, "DROP VIEW v").await;
    exec(&console, "CREATE VIEW v AS SELECT a AS b FROM t").await;
    bounded(handle.refresh(
        Target::Objects {
            ns: main_ns(),
            keys: vec![v.clone()],
        },
        Trigger::User,
    ))
    .await
    .expect("re-read");
    bounded(handle.elevate(v.clone(), Level::Sources))
        .await
        .expect("elevated");
    let snap = catalog.current();
    let Props::View(p2) = snap.find(&v).expect("the view").obj().props().clone() else {
        panic!("view props");
    };
    let r2 = p2.source.expect("a new reference");
    assert_ne!(r2, r);
    assert!(snap.source_text(&r2).expect("served").contains("AS b"));
    assert_eq!(
        snap.source_text(&r),
        None,
        "the old text left with its reference"
    );
    // The view's `Sources` facet went with its re-read; the table, not re-read, keeps its.
    assert_eq!(snap.server_text(&v), None);
    assert_eq!(snap.server_text(&t), Some(table_text));
    handle.shutdown();
    bounded(handle.closed()).await;
}
