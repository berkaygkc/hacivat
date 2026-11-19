//! `CatalogHub` (M4 Task 15; R-M4-31, R-M4-15): one engine, catalog, cache and key per
//! profile, usable without the UI; the key only on `ensure`, never on `get`.

mod common;

use std::sync::Arc;

use common::*;
use hacivat_introspect::IntroError;
use hacivat_introspect::hub::{
    CacheMode, CatalogHub, HubSettings, MemoryOnlyReason, SessionLookup,
};
use hacivat_meta::codec::FORMAT_MAJOR;
use hacivat_meta::persist::{CacheStore, Encryption, WriterId};
use hacivat_meta::snapshot::{ContainerKey, LoadState};
use hacivat_proto::{DbmsKind, ProfileId};
use hacivat_secrets::{MemoryBackend, SecretBackend, SecretStore};
use hacivat_session::meta::MetaError;
use hacivat_session::profile::Profile;
use hacivat_session::session::SessionManager;
use hacivat_settings::{LocalPaths, MetadataSettings};

/// No profile is connected (a hub without a UI or a connection).
struct NoSessions;

impl SessionLookup for NoSessions {
    fn sessions(&self, _: ProfileId) -> Option<Arc<SessionManager>> {
        None
    }
}

fn hub_with(secrets: Arc<SecretStore>) -> (Arc<CatalogHub>, tempfile::TempDir) {
    let dir = tempfile::tempdir().expect("a temp dir");
    let hub = CatalogHub::new(
        Arc::new(NoSessions),
        HubSettings {
            paths: LocalPaths::under(dir.path()),
            metadata: MetadataSettings::default(),
        },
        secrets,
        Arc::new(hacivat_meta::system::SystemCatalogs::empty()),
    );
    (hub, dir)
}

fn sensitive(engine: DbmsKind) -> Arc<Profile> {
    Arc::new(Profile {
        catalog_sensitive: true,
        ..(*profile(engine)).clone()
    })
}

fn cache_path(dir: &tempfile::TempDir, p: &Profile) -> std::path::PathBuf {
    LocalPaths::under(dir.path()).catalog_db(&p.id.0.to_string(), u32::from(FORMAT_MAJOR))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ensure_starts_one_engine_per_profile() {
    let (hub, _dir) = hub_with(Arc::new(SecretStore::new(None)));
    let p = profile(DbmsKind::Postgres);
    let q = profile(DbmsKind::MySql);
    // Concurrent first uses make one catalog.
    let (a, b) = bounded(futures::future::join(
        hub.ensure(p.clone()),
        hub.ensure(p.clone()),
    ))
    .await;
    let (a, b) = (a.unwrap(), b.unwrap());
    assert!(Arc::ptr_eq(&a.catalog, &b.catalog));
    // Its engine is the catalog's one writer.
    assert!(a.catalog.publisher().is_none());
    let other = bounded(hub.ensure(q.clone())).await.unwrap();
    assert!(!Arc::ptr_eq(&other.catalog, &a.catalog));
    assert_eq!(other.catalog.current().server.dbms, DbmsKind::MySql);
    assert!(hub.ensure(profile(DbmsKind::MsSql)).await.is_err());
    // Shutdown stops it (requests answer Cancelled) and forgets the catalog.
    bounded(hub.shutdown(p.id)).await;
    assert!(hub.get(p.id).is_none());
    assert_eq!(
        bounded(a.handle.expand(pg_ns("db", "public"))).await,
        Err(IntroError::Cancelled)
    );
    let again = bounded(hub.ensure(p)).await.unwrap();
    assert!(!Arc::ptr_eq(&again.catalog, &a.catalog));
}

/// A hub with no UI and no connection serves the cached catalog (stale) as soon as `ensure`
/// answers; reading needs a connection, and its absence changes nothing.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hub_without_ui_serves_a_cached_catalog() {
    let (hub, dir) = hub_with(Arc::new(SecretStore::new(None)));
    let p = profile(DbmsKind::Postgres);
    let ns = pg_ns("db", "public");
    {
        let mut store = CacheStore::open(&cache_path(&dir, &p), Encryption::None, WriterId(1))
            .unwrap()
            .store;
        let arena = hacivat_meta::fixture::ns_pg("db", "public")
            .table("orders", &["id", "total"])
            .finish();
        store.save_namespace(&arena).unwrap();
    }
    let pc = bounded(hub.ensure(p.clone())).await.unwrap();
    assert!(matches!(
        pc.cache,
        CacheMode::Persistent {
            encrypted: false,
            ..
        }
    ));
    let snap = pc.catalog.current();
    assert!(snap.namespaces.contains_key(&ns));
    assert!(matches!(
        snap.state(&ContainerKey::Namespace(ns.clone())),
        LoadState::Loaded { stale: true, .. }
    ));
    // Not connected: a read fails and the cached rows stay as they are.
    assert_eq!(
        bounded(pc.handle.on_connect()).await,
        Err(IntroError::Meta(MetaError::Unavailable))
    );
    let after = pc.catalog.current();
    assert!(Arc::ptr_eq(&after.namespaces[&ns], &snap.namespaces[&ns]));
}

/// "Encrypt cached metadata" with no keyring: memory-only, never a plaintext file.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sensitive_profile_without_keyring_is_memory_only() {
    let (hub, dir) = hub_with(Arc::new(SecretStore::new(None)));
    let p = sensitive(DbmsKind::Postgres);
    let pc = bounded(hub.ensure(p.clone())).await.unwrap();
    assert_eq!(
        pc.cache,
        CacheMode::MemoryOnly(MemoryOnlyReason::KeyringUnavailable)
    );
    assert!(!cache_path(&dir, &p).exists(), "no cache file was made");
    assert!(hub.row_key(p.id).is_none());
    // With a keyring the same profile is encrypted at rest, its key made once.
    let backend = Arc::new(MemoryBackend::new());
    let (hub, dir) = hub_with(Arc::new(SecretStore::new(Some(
        backend.clone() as Arc<dyn SecretBackend>
    ))));
    let p = sensitive(DbmsKind::Postgres);
    let (a, b) = bounded(futures::future::join(
        hub.ensure(p.clone()),
        hub.ensure(p.clone()),
    ))
    .await;
    assert!(Arc::ptr_eq(&a.unwrap().catalog, &b.unwrap().catalog));
    let pc = hub.get(p.id).unwrap();
    assert!(matches!(
        pc.cache,
        CacheMode::Persistent {
            encrypted: true,
            ..
        }
    ));
    assert!(cache_path(&dir, &p).exists());
    assert!(hub.row_key(p.id).is_some());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn get_never_fetches_a_key() {
    let backend = Arc::new(MemoryBackend::new());
    let (hub, _dir) = hub_with(Arc::new(SecretStore::new(Some(
        backend.clone() as Arc<dyn SecretBackend>
    ))));
    let p = sensitive(DbmsKind::Postgres);
    // Before `ensure`: nothing, and no keyring call.
    assert!(hub.get(p.id).is_none());
    assert!(hub.row_key(p.id).is_none());
    assert_eq!(backend.calls(), 0);
    bounded(hub.ensure(p.clone())).await.unwrap();
    let after_ensure = backend.calls();
    assert!(after_ensure > 0, "ensure fetched (made) the key");
    for _ in 0..10 {
        assert!(hub.get(p.id).is_some());
        assert!(hub.row_key(p.id).is_some());
    }
    assert_eq!(backend.calls(), after_ensure, "get never touches the keyring");
}
