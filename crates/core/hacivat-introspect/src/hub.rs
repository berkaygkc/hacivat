// ## m4-t01c-core
//! `CatalogHub` (R-M4-31; Task 15): one engine, catalog, cache and key per profile, reachable
//! without the UI (the explorer, M6, M7's apply, M8's broker).
//!
//! **Fail-closed stub (M4 Task 1c).** [`CatalogHub::ensure`] answers a catalog with no engine:
//! the cache is not opened (`CacheStore::open` is `Disabled` until Task 11), no key is fetched
//! (no keyring call at all), nothing is read, so every namespace is `NotLoaded` and every
//! request on the handle answers `IntroError::NotImplemented`.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use futures::future::BoxFuture;
use hacivat_meta::persist::RowKey;
use hacivat_meta::system::SystemCatalogs;
use hacivat_meta::{Catalog, ServerFacts};
use hacivat_proto::{DbmsKind, ProfileId};
use hacivat_secrets::SecretStore;
use hacivat_session::ConnectionManager;
use hacivat_session::profile::Profile;
use hacivat_session::session::SessionManager;
use hacivat_settings::{LocalPaths, MetadataSettings};
use sql_dialect::DialectId;
use sql_ident::key::IdentCtx;

use crate::engine::IntroHandle;

/// Where the hub finds a profile's sessions (the app's `ConnectionManager`; a test double).
pub trait SessionLookup: Send + Sync {
    /// The live sessions of `profile`, when it is connected.
    fn sessions(&self, profile: ProfileId) -> Option<Arc<SessionManager>>;
}

impl SessionLookup for ConnectionManager {
    fn sessions(&self, profile: ProfileId) -> Option<Arc<SessionManager>> {
        ConnectionManager::sessions(self, profile)
    }
}

/// What the hub reads from the settings: where the caches live and the `[metadata]` keys.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HubSettings {
    pub paths: LocalPaths,
    pub metadata: MetadataSettings,
}

/// One profile's catalog and its engine handle.
#[derive(Debug, Clone)]
pub struct ProfileCatalog {
    pub catalog: Arc<Catalog>,
    pub handle: IntroHandle,
}

/// Why the hub could not serve a profile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HubError {
    /// M4 introspects PostgreSQL, MySQL, MariaDB and SQLite only (R-M4-4).
    UnsupportedEngine(DbmsKind),
    /// The hub was shut down.
    Shutdown,
}

/// The per-profile catalogs.
pub struct CatalogHub {
    sessions: Arc<dyn SessionLookup>,
    settings: HubSettings,
    secrets: Arc<SecretStore>,
    system: Arc<SystemCatalogs>,
    profiles: Mutex<HashMap<ProfileId, ProfileCatalog>>,
}

impl std::fmt::Debug for CatalogHub {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let profiles = self.lock().len();
        f.debug_struct("CatalogHub")
            .field("profiles", &profiles)
            .finish_non_exhaustive()
    }
}

impl CatalogHub {
    pub fn new(
        sessions: Arc<dyn SessionLookup>,
        settings: HubSettings,
        secrets: Arc<SecretStore>,
        system: Arc<SystemCatalogs>,
    ) -> Arc<CatalogHub> {
        Arc::new(CatalogHub {
            sessions,
            settings,
            secrets,
            system,
            profiles: Mutex::new(HashMap::new()),
        })
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<ProfileId, ProfileCatalog>> {
        self.profiles.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The catalog of `profile`, when [`ensure`](Self::ensure) made one.
    pub fn get(&self, profile: ProfileId) -> Option<ProfileCatalog> {
        self.lock().get(&profile).cloned()
    }

    /// The catalog of `profile`, made on first use (opens the cache, fetches the
    /// `CatalogKey` when `catalog_sensitive`, starts the engine: Task 15). Never called at app
    /// start for a profile the UI did not ask for (R-M4-15).
    ///
    /// Stub: an empty catalog with a detached handle (cache only, nothing loaded).
    pub fn ensure(&self, profile: Arc<Profile>) -> BoxFuture<'_, Result<ProfileCatalog, HubError>> {
        Box::pin(async move {
            if let Some(found) = self.get(profile.id) {
                return Ok(found);
            }
            let dialect =
                dialect_of(profile.engine).ok_or(HubError::UnsupportedEngine(profile.engine))?;
            let _ = (&self.sessions, &self.settings, &self.secrets, &self.system);
            let server = ServerFacts::new(profile.engine, IdentCtx::new(dialect));
            let catalog = Arc::new(Catalog::new(Arc::new(server)));
            let made = ProfileCatalog {
                handle: IntroHandle::detached(catalog.clone()),
                catalog,
            };
            Ok(self.lock().entry(profile.id).or_insert(made).clone())
        })
    }

    /// The key of `profile`'s encrypted cache rows, when one was fetched. Stub: never.
    pub fn row_key(&self, profile: ProfileId) -> Option<RowKey> {
        let _ = profile;
        None
    }

    /// Stops `profile`'s engine and forgets its catalog.
    pub fn shutdown(&self, profile: ProfileId) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            if let Some(gone) = self.lock().remove(&profile) {
                gone.handle.shutdown();
            }
        })
    }
}

/// The SQL dialect of an engine M4 introspects.
fn dialect_of(dbms: DbmsKind) -> Option<DialectId> {
    match dbms {
        DbmsKind::Postgres => Some(DialectId::Postgres),
        DbmsKind::MySql => Some(DialectId::MySql),
        DbmsKind::MariaDb => Some(DialectId::MariaDb),
        DbmsKind::Sqlite => Some(DialectId::Sqlite),
        _ => None,
    }
}
