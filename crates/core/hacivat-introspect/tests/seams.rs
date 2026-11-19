//! M4 Task 1c Part B: the introspection seams fail closed (Global Constraints, "No silent
//! weakening"): refresh planning widens, the scope selects only the current namespace,
//! freshness is `Unknown`, introspectors and facet loads answer `NotImplemented`, and the hub
//! serves a cache-only catalog.

#![cfg(feature = "session")]

mod common;

use std::collections::BTreeSet;
use std::sync::Arc;

use common::{name, pg_ns, profile};
use hacivat_introspect::ddl_effects::{
    ExecutedStmt, SessionStateAtExec, Target, TxnState, plan_refresh,
};
use hacivat_introspect::freshness::check_state;
use hacivat_introspect::hub::{CatalogHub, HubSettings, SessionLookup};
use hacivat_introspect::scope::{CurrentCtx, NsInfo, parse_scope};
use hacivat_introspect::{IntroCaps, IntroError, Mode, Staleness, introspector_for};
use hacivat_meta::props::FacetKind;
use hacivat_meta::{Level, NsKey, ObjectKey, ObjectKind};
use hacivat_proto::{DbmsKind, ProfileId};
use hacivat_secrets::SecretStore;
use hacivat_session::meta::MetaConn;
use hacivat_session::session::SessionManager;
use hacivat_settings::{LocalPaths, MetadataSettings};
use sql_dialect::DialectId;
use sql_ident::{Ident, QualifiedName};
use sql_safety::{DdlAction, DdlEffect, DdlKind, StmtKind};
use tokio_util::sync::CancellationToken;

// ---- helpers ---------------------------------------------------------------------------------

fn qn(parts: &[&str]) -> QualifiedName {
    QualifiedName {
        parts: parts
            .iter()
            .map(|p| Ident {
                text: Arc::from(*p),
                quoted: false,
            })
            .collect(),
    }
}

/// A `CREATE TABLE schema.table` effect, as Task 6 will report it (a named, non-opaque one).
fn ddl_create_table(schema: &str, table: &str) -> DdlEffect {
    DdlEffect {
        action: DdlAction::Create,
        kind: DdlKind::Table,
        name: Some(qn(&[schema, table])),
        on: None,
        referenced: Vec::new(),
        temp: false,
        concurrently: false,
        opaque: false,
    }
}

fn executed(_sql: &str, effect: DdlEffect) -> ExecutedStmt {
    ExecutedStmt {
        kind: StmtKind::CreateTable,
        ddl: vec![effect],
    }
}

fn ns(schema: &str) -> NsKey {
    pg_ns("db", schema)
}

fn session_at(schema: &str) -> SessionStateAtExec {
    SessionStateAtExec {
        current: ns(schema),
        search_path: vec![ns(schema)],
    }
}

fn caps_pg() -> IntroCaps {
    IntroCaps::planned(DbmsKind::Postgres)
}

fn infos() -> Vec<NsInfo> {
    ["public", "app", "pg_catalog"]
        .into_iter()
        .map(|s| NsInfo {
            key: ns(s),
            system: s == "pg_catalog",
        })
        .collect()
}

fn ctx(db: &str, schema: &str) -> CurrentCtx {
    CurrentCtx {
        database: Some(name(db)),
        schema: Some(name(schema)),
        // ## m4-t12-scope (named exception): `CurrentCtx` carries the server's identifier rule.
        ident: None,
    }
}

/// `"db.public"` → the PostgreSQL namespace `[db, public]`.
fn set<const N: usize>(dotted: [&str; N]) -> BTreeSet<NsKey> {
    dotted
        .into_iter()
        .map(|d| NsKey::schema(d.split('.')))
        .collect()
}

fn table(schema: &str, t: &str) -> ObjectKey {
    ObjectKey {
        kind: ObjectKind::Table,
        path: ["db", schema, t].into_iter().map(name).collect(),
        server_id: None,
        overload_sig: None,
        raw_kind: None,
    }
}

/// No profile is connected.
struct NoSessions;

impl SessionLookup for NoSessions {
    fn sessions(&self, _: ProfileId) -> Option<Arc<SessionManager>> {
        None
    }
}

fn hub() -> (Arc<CatalogHub>, tempfile::TempDir) {
    let dir = tempfile::tempdir().expect("a temp dir");
    let hub = CatalogHub::new(
        Arc::new(NoSessions),
        HubSettings {
            paths: LocalPaths::under(dir.path()),
            metadata: MetadataSettings::default(),
        },
        Arc::new(SecretStore::new(None)),
        Arc::new(hacivat_meta::system::SystemCatalogs::empty()),
    );
    (hub, dir)
}

// ---- tests -----------------------------------------------------------------------------------

#[test]
fn stub_plan_refresh_widens_any_ddl_to_the_current_namespace() {
    let p = plan_refresh(
        &[executed(
            "CREATE TABLE s.t(a int)",
            ddl_create_table("s", "t"),
        )],
        &session_at("public"),
        TxnState::AutoCommit,
        &caps_pg(),
    );
    assert_eq!(p.now, vec![Target::Namespace(ns("public"))]);
    assert!(p.pending.is_empty());
}

#[test]
fn scope_selects_the_current_and_every_user_schema() {
    // ## m4-t12-scope (named exception; renamed from `stub_scope_selects_only_current` in fix
    // round 1, M8): the matcher landed. The empty pattern still selects
    // only `@:@`; `*:*` now selects every listed user schema, never the system one (R-M4-14).
    let scope = |p: &str| {
        parse_scope(p, DialectId::Postgres)
            .unwrap()
            .select(&infos(), &ctx("db", "public"))
    };
    assert_eq!(scope(""), set(["db.public"]));
    assert_eq!(scope("*:*"), set(["db.public", "db.app"]));
}

#[test]
fn stub_check_state_is_unknown() {
    let catalog = hacivat_meta::Catalog::new(Arc::new(hacivat_meta::ServerFacts::new(
        DbmsKind::Postgres,
        sql_ident::key::IdentCtx::new(DialectId::Postgres),
    )));
    let keys = [table("public", "orders"), table("public", "gone")];
    assert_eq!(
        check_state(&catalog.current(), &keys),
        vec![Staleness::Unknown, Staleness::Unknown]
    );
}

#[tokio::test]
async fn stub_stamps_and_facet_not_implemented() {
    let keys = [table("public", "orders")];
    let cancel = CancellationToken::new();
    for dbms in [
        DbmsKind::Postgres,
        DbmsKind::MySql,
        DbmsKind::MariaDb,
        DbmsKind::Sqlite,
    ] {
        let intro = introspector_for(dbms).expect("an M4 engine");
        let mut c = MetaConn::offline(dbms);
        let n = ns("public");
        assert_eq!(
            intro.stamps(&mut c, &keys).await,
            Err(IntroError::NotImplemented),
            "{dbms:?}"
        );
        for kind in [
            FacetKind::Sources,
            FacetKind::Grants,
            FacetKind::Dependencies,
            FacetKind::Statistics,
        ] {
            assert_eq!(
                intro.facet(&mut c, &n, &keys, kind).await,
                Err(IntroError::NotImplemented),
                "{dbms:?} {kind:?}"
            );
        }
        assert_eq!(
            intro.list_namespaces(&mut c).await,
            Err(IntroError::NotImplemented)
        );
        // ## m4-t15-engine (named exception, ruling R-W3-2): the filter argument; and the
        // identity default is `Unknown` (R-W3-3).
        let all = hacivat_introspect::filter::parse_filter(
            "",
            sql_dialect::DialectId::Postgres,
            hacivat_introspect::filter::FilterRole::Introspect,
        )
        .expect("the empty filter");
        assert!(matches!(
            intro
                .introspect(
                    &mut c,
                    &n,
                    Mode::Full,
                    Level::Structure,
                    &all,
                    None,
                    &cancel
                )
                .await,
            Err(IntroError::NotImplemented)
        ));
        assert!(matches!(
            intro
                .fragment(&mut c, &n, &keys, Level::Names, &all, &cancel)
                .await,
            Err(IntroError::NotImplemented)
        ));
        assert_eq!(
            intro.identity(&mut c).await,
            Ok(hacivat_introspect::introspector::ServerIdentity::Unknown)
        );
        // ## m4-t15-engine
        assert_eq!(
            intro.check_state(&mut c, &keys).await,
            Err(IntroError::NotImplemented)
        );
    }
}

#[tokio::test]
async fn stub_load_facet_leaves_the_facet_not_loaded() {
    let (hub, _dir) = hub();
    let pc = hub
        .ensure(profile(DbmsKind::Postgres))
        .await
        .expect("a catalog");
    let before = pc.catalog.current();
    let got = pc
        .handle
        .load_facet(vec![table("public", "orders")], FacetKind::Grants)
        .await;
    assert_eq!(got, Err(IntroError::NotImplemented));
    let after = pc.catalog.current();
    // Nothing was published: no generation, no namespace, so no facet is loaded anywhere.
    assert_eq!(after.generation, before.generation);
    assert!(after.namespaces.is_empty());
    // ## m4-t15-engine (named exception): the engine now runs the check; with no connection
    // it answers `Unavailable` and publishes nothing.
    assert_eq!(
        pc.handle.check_state(vec![table("public", "orders")]).await,
        Err(IntroError::Meta(
            hacivat_session::meta::MetaError::Unavailable
        ))
    );
    assert_eq!(pc.catalog.current().generation, before.generation);
    // ## m4-t15-engine
}

#[tokio::test]
async fn stub_hub_serves_cache_only() {
    let (hub, _dir) = hub();
    let p = profile(DbmsKind::Postgres);
    assert!(hub.get(p.id).is_none(), "nothing before ensure");
    let pc = hub.ensure(p.clone()).await.expect("a catalog");
    // Cache only: an empty catalog at generation 0, every namespace NotLoaded (none listed).
    let snap = pc.catalog.current();
    assert_eq!(snap.generation, 0);
    assert!(snap.namespaces.is_empty());
    assert_eq!(snap.server.dbms, DbmsKind::Postgres);
    // ## m4-t15-engine (named exception): an engine runs now; with no connection nothing is
    // read (the stub introspectors would answer `NotImplemented`), and no key is fetched.
    let unavailable = Err(IntroError::Meta(
        hacivat_session::meta::MetaError::Unavailable,
    ));
    assert_eq!(pc.handle.on_connect().await, unavailable);
    assert_eq!(pc.handle.expand(ns("public")).await, unavailable);
    assert!(pc.catalog.current().namespaces.is_empty());
    // ## m4-t15-engine
    assert!(hub.row_key(p.id).is_none());
    // One catalog per profile.
    let again = hub.ensure(p.clone()).await.expect("a catalog");
    assert!(Arc::ptr_eq(&again.catalog, &pc.catalog));
    assert!(Arc::ptr_eq(&hub.get(p.id).unwrap().catalog, &pc.catalog));
    // An engine M4 does not introspect is refused.
    assert!(hub.ensure(profile(DbmsKind::MsSql)).await.is_err());
    hub.shutdown(p.id).await;
    assert!(hub.get(p.id).is_none());
}
