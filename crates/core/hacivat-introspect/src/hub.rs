// ## m4-t01c-core
//! `CatalogHub` (R-M4-31; Task 15): one engine, catalog, cache and key per profile, reachable
//! without the UI (the explorer, M6, M7's apply, M8's broker).
//!
//! ## m4-t15-hub (M4 Task 15; ADR 0008 `## m4-t15 — engine and hub`)
//!
//! - [`CatalogHub::ensure`] makes a profile's catalog once (concurrent callers share one
//!   making: a per-profile lock), in this order, every blocking step on the runtime's blocking
//!   pool:
//!   1. **The key** (`Profile::catalog_sensitive()`: the explicit choice, else production; never the raw field): `SecretStore::catalog_key(profile, true)`,
//!      which reads the key or creates it on the first encrypted use. It is created here and
//!      only here, under the per-profile lock, so two first-time creators never race (T11
//!      carry). A refused or unavailable keyring makes the profile **memory-only** for the
//!      session ([`CacheMode::MemoryOnly`], `KeyringUnavailable`): a sensitive profile is never
//!      opened with `Encryption::None` (the store would answer `Disabled` anyway).
//!   2. **The cache** (`CacheStore::open` at `catalog_db(uuid, FORMAT_MAJOR)`, caps from the
//!      settings); any error: memory-only.
//!   3. **The engine** ([`Engine::start`]): it publishes the cached namespaces (stale) before
//!      `ensure` answers, so a hub without a UI serves the cached catalog at once.
//! - [`CatalogHub::get`] only reads the map: it never opens a file and never touches the
//!   keyring. `ensure` is never called at app start for a profile the UI did not ask for
//!   (R-M4-15): that is the caller's rule, and the reason `get` exists.
//! - **Sessions** ([`HubMeta`]): every metadata connection comes from
//!   `SessionManager::meta_conn` with a spec naming the requested profile; `meta_conn` refuses a
//!   spec of another profile (`Unavailable`, Task 5), so a lookup that answered another
//!   profile's manager can never serve this one (T1c review m-B2). Nothing else of the manager
//!   is used.
//! - **Defaults** for a profile the app has not configured yet: the R-M4-14 scope (PostgreSQL
//!   `@:*`, MySQL/MariaDB and SQLite `*`), the profile's database as the current one, D-038's
//!   levels, periodic validation per R-M4-20 (`metadata.validate_interval_min`: `auto` is 10
//!   minutes for `local`/`dev` profiles, off otherwise). The app forwards the saved scope, the
//!   current schema and the pills through the handle.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use futures::future::BoxFuture;
use hacivat_meta::codec::FORMAT_MAJOR;
use hacivat_meta::name::Name;
use hacivat_meta::persist::{CacheCaps, CacheStore, Encryption, RebuildReason, RowKey, WriterId};
use hacivat_meta::system::SystemCatalogs;
use hacivat_meta::{Catalog, ServerFacts};
use hacivat_proto::{DbmsKind, EnvTag, ProfileId};
use hacivat_secrets::SecretStore;
use hacivat_session::ConnectionManager;
use hacivat_session::meta::{MetaConn, MetaConnSpec, MetaError};
use hacivat_session::profile::Profile;
use hacivat_session::session::SessionManager;
use hacivat_settings::{LocalPaths, MetadataSettings, ValidateInterval};
use sql_dialect::DialectId;
use sql_ident::key::IdentCtx;
use tokio_util::sync::CancellationToken;

use crate::engine::{
    Engine, EngineConfig, EngineLimits, Filters, IntroHandle, LevelPolicy, MetaSource,
};
use crate::introspector::introspector_for;
use crate::scope::{CurrentCtx, parse_scope};

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

// ## m4-t15-hub
/// Whether a profile's catalog is persisted, and why not.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CacheMode {
    /// Kept in the cache file; `rebuilt` says why it started empty, if it did
    /// (`model-cache-rebuilt`).
    Persistent {
        encrypted: bool,
        rebuilt: Option<RebuildReason>,
    },
    /// In memory for this session only.
    MemoryOnly(MemoryOnlyReason),
}

/// Why a profile's catalog is memory-only (the UI's notice; alg/02 HV-MODEL-03 edge 9).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryOnlyReason {
    /// "Encrypt cached metadata" is on and the keyring refused or is unavailable.
    KeyringUnavailable,
    /// The cache file could not be opened (a read-only directory, a locked file, …).
    CacheUnavailable,
}
// ## m4-t15-hub

/// One profile's catalog and its engine handle.
#[derive(Debug, Clone)]
pub struct ProfileCatalog {
    pub catalog: Arc<Catalog>,
    pub handle: IntroHandle,
    // ## m4-t15-hub
    pub cache: CacheMode,
}

/// Why the hub could not serve a profile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HubError {
    /// M4 introspects PostgreSQL, MySQL, MariaDB and SQLite only (R-M4-4).
    UnsupportedEngine(DbmsKind),
    /// The hub was shut down.
    Shutdown,
    // ## m4-t15-hub
    /// The hub was made outside a Tokio runtime and given none (`CatalogHub::new_on`).
    NoRuntime,
}

// ## m4-t15-hub
/// One profile's entry: its catalog and the key its cache rows are sealed with.
struct Entry {
    catalog: ProfileCatalog,
    key: Option<RowKey>,
}
// ## m4-t15-hub

/// The per-profile catalogs.
pub struct CatalogHub {
    sessions: Arc<dyn SessionLookup>,
    settings: HubSettings,
    secrets: Arc<SecretStore>,
    system: Arc<SystemCatalogs>,
    // ## m4-t15-hub
    profiles: Mutex<HashMap<ProfileId, Entry>>,
    /// Per-profile locks: one making per profile at a time.
    making: Mutex<HashMap<ProfileId, Arc<tokio::sync::Mutex<()>>>>,
    runtime: Option<tokio::runtime::Handle>,
    limits: EngineLimits,
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
    /// A hub on the current Tokio runtime (call it on the `CoreRuntime`); see
    /// [`CatalogHub::new_on`].
    pub fn new(
        sessions: Arc<dyn SessionLookup>,
        settings: HubSettings,
        secrets: Arc<SecretStore>,
        system: Arc<SystemCatalogs>,
    ) -> Arc<CatalogHub> {
        let rt = tokio::runtime::Handle::try_current().ok();
        Self::make(
            sessions,
            settings,
            secrets,
            system,
            rt,
            EngineLimits::default(),
        )
    }

    // ## m4-t15-hub
    /// A hub whose engines and blocking work run on `rt` (the `CoreRuntime`).
    pub fn new_on(
        rt: tokio::runtime::Handle,
        sessions: Arc<dyn SessionLookup>,
        settings: HubSettings,
        secrets: Arc<SecretStore>,
        system: Arc<SystemCatalogs>,
    ) -> Arc<CatalogHub> {
        Self::make(
            sessions,
            settings,
            secrets,
            system,
            Some(rt),
            EngineLimits::default(),
        )
    }

    /// As [`CatalogHub::new_on`] with the engines' knobs (tests).
    pub fn with_limits(
        rt: tokio::runtime::Handle,
        sessions: Arc<dyn SessionLookup>,
        settings: HubSettings,
        secrets: Arc<SecretStore>,
        system: Arc<SystemCatalogs>,
        limits: EngineLimits,
    ) -> Arc<CatalogHub> {
        Self::make(sessions, settings, secrets, system, Some(rt), limits)
    }

    fn make(
        sessions: Arc<dyn SessionLookup>,
        settings: HubSettings,
        secrets: Arc<SecretStore>,
        system: Arc<SystemCatalogs>,
        runtime: Option<tokio::runtime::Handle>,
        limits: EngineLimits,
    ) -> Arc<CatalogHub> {
        Arc::new(CatalogHub {
            sessions,
            settings,
            secrets,
            system,
            profiles: Mutex::new(HashMap::new()),
            making: Mutex::new(HashMap::new()),
            runtime,
            limits,
        })
    }

    /// The system catalogs the hub serves (Task 26 layers them over each catalog).
    pub fn system(&self) -> &Arc<SystemCatalogs> {
        &self.system
    }
    // ## m4-t15-hub

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<ProfileId, Entry>> {
        self.profiles.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The catalog of `profile`, when [`ensure`](Self::ensure) made one. Reads a map only: no
    /// file, no keyring, no engine.
    pub fn get(&self, profile: ProfileId) -> Option<ProfileCatalog> {
        self.lock().get(&profile).map(|e| e.catalog.clone())
    }

    /// The catalog of `profile`, made on first use: the `CatalogKey` when `Profile::catalog_sensitive()`
    /// (blocking pool; created here once), the cache, the engine (its cached namespaces are
    /// published before this answers). Never called at app start for a profile the UI did not
    /// ask for (R-M4-15).
    pub fn ensure(&self, profile: Arc<Profile>) -> BoxFuture<'_, Result<ProfileCatalog, HubError>> {
        Box::pin(async move {
            if let Some(found) = self.get(profile.id) {
                return Ok(found);
            }
            let dialect =
                dialect_of(profile.engine).ok_or(HubError::UnsupportedEngine(profile.engine))?;
            let rt = self.runtime.clone().ok_or(HubError::NoRuntime)?;
            let gate = self.gate(profile.id);
            let made = {
                let _making = gate.lock().await;
                self.make_locked(profile.clone(), dialect, rt).await
            };
            self.prune(profile.id, gate);
            made
        })
    }

    // ## m4-t15-hub (fix round 1, m3)
    /// The per-profile lock of `ensure` and `shutdown` (one making or stopping at a time).
    fn gate(&self, profile: ProfileId) -> Arc<tokio::sync::Mutex<()>> {
        self.making
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .entry(profile)
            .or_default()
            .clone()
    }

    /// Forgets the per-profile lock when nobody else holds it (the map and `gate` are the only
    /// owners): the map stays as small as the set of profiles being made or stopped.
    fn prune(&self, profile: ProfileId, gate: Arc<tokio::sync::Mutex<()>>) {
        let mut making = self.making.lock().unwrap_or_else(PoisonError::into_inner);
        if Arc::strong_count(&gate) == 2 {
            making.remove(&profile);
        }
    }

    /// The number of per-profile locks kept (tests: they are pruned).
    pub fn pending_locks(&self) -> usize {
        self.making
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len()
    }

    /// `ensure`'s making, under the profile's lock.
    async fn make_locked(
        &self,
        profile: Arc<Profile>,
        dialect: DialectId,
        rt: tokio::runtime::Handle,
    ) -> Result<ProfileCatalog, HubError> {
        {
            if let Some(found) = self.get(profile.id) {
                return Ok(found);
            }
            let (key, store, cache) = self.open_cache(&rt, &profile).await;
            let introspector = introspector_for(profile.engine)
                .ok_or(HubError::UnsupportedEngine(profile.engine))?;
            // ## m4-t17-mysql (named exception; review I-2): a MySQL-family placeholder does not
            // know `lower_case_table_names` until the server says, so no table rule is pinned
            // (lookups answer `ServerDependent`, never a wrong `Missing`); the scope refresh
            // replaces it with the real facts.
            let mut ident = IdentCtx::new(dialect);
            if matches!(dialect, DialectId::MySql | DialectId::MariaDb) {
                ident.lower_case_table_names = u8::MAX;
            }
            let server = ServerFacts::new(profile.engine, ident);
            let catalog = Arc::new(Catalog::new(Arc::new(server)));
            let publisher = catalog.publisher().ok_or(HubError::Shutdown)?;
            let scope =
                parse_scope(default_scope(dialect), dialect).expect("the default scopes parse");
            let handle = Engine::start(EngineConfig {
                profile: profile.id,
                dbms: profile.engine,
                meta: Arc::new(HubMeta {
                    sessions: self.sessions.clone(),
                    profile: profile.id,
                    single_session: profile.single_session,
                    timeout: META_STATEMENT_TIMEOUT,
                    verified: Mutex::new(std::sync::Weak::new()),
                }),
                introspector,
                catalog: catalog.clone(),
                catalog_publisher: publisher,
                cache: store,
                levels: LevelPolicy::default(),
                scope,
                current: current_of(&profile, dialect),
                filters: Filters::default(),
                validate_every: validate_every(&self.settings.metadata, &profile),
                limits: self.limits,
                runtime: rt,
            });
            // The cached namespaces are published before the catalog is handed out.
            let _ = handle.ready().await;
            let made = ProfileCatalog {
                catalog,
                handle,
                cache,
            };
            self.lock().insert(
                profile.id,
                Entry {
                    catalog: made.clone(),
                    key,
                },
            );
            Ok(made)
        }
    }

    // ## m4-t15-hub
    /// The key and the cache of `profile` (blocking steps on `rt`'s blocking pool).
    async fn open_cache(
        &self,
        rt: &tokio::runtime::Handle,
        profile: &Profile,
    ) -> (
        Option<RowKey>,
        Option<crate::engine::CacheHandle>,
        CacheMode,
    ) {
        let encryption = if profile.catalog_sensitive() {
            let secrets = self.secrets.clone();
            let id = profile.id;
            let fetched = rt
                .spawn_blocking(move || {
                    secrets
                        .catalog_key(id, true)
                        .ok()
                        .flatten()
                        .map(|bytes| RowKey::from_bytes(&bytes))
                })
                .await
                .ok()
                .flatten();
            match fetched {
                Some(k) => Encryption::Row(k),
                None => {
                    return (
                        None,
                        None,
                        CacheMode::MemoryOnly(MemoryOnlyReason::KeyringUnavailable),
                    );
                }
            }
        } else {
            Encryption::None
        };
        let key = match &encryption {
            Encryption::Row(k) => Some(k.clone()),
            Encryption::None => None,
        };
        let encrypted = key.is_some();
        let path = self
            .settings
            .paths
            .catalog_db(&profile.id.0.to_string(), u32::from(FORMAT_MAJOR));
        let caps = CacheCaps::from_mib(
            self.settings.metadata.cache_mb_per_profile.get(),
            self.settings.metadata.cache_mb_total.get(),
        );
        let opened = rt
            .spawn_blocking(move || {
                CacheStore::open(&path, encryption, WriterId(std::process::id().into())).map(
                    |mut o| {
                        o.store.set_caps(caps);
                        o
                    },
                )
            })
            .await;
        match opened {
            Ok(Ok(o)) => (
                key,
                Some(Arc::new(Mutex::new(o.store))),
                CacheMode::Persistent {
                    encrypted,
                    rebuilt: o.rebuilt,
                },
            ),
            _ => (
                None,
                None,
                CacheMode::MemoryOnly(MemoryOnlyReason::CacheUnavailable),
            ),
        }
    }
    // ## m4-t15-hub

    /// The key of `profile`'s encrypted cache rows, when [`ensure`](Self::ensure) fetched one
    /// (never fetched here).
    pub fn row_key(&self, profile: ProfileId) -> Option<RowKey> {
        self.lock().get(&profile).and_then(|e| e.key.clone())
    }

    /// Stops `profile`'s engine (pending cache saves are written) and forgets its catalog.
    pub fn shutdown(&self, profile: ProfileId) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            // Under the profile's lock: a shutdown during a first `ensure` waits for it and then
            // stops what it made (review m3).
            let gate = self.gate(profile);
            {
                let _stopping = gate.lock().await;
                let gone = self.lock().remove(&profile);
                if let Some(gone) = gone {
                    let handle = gone.catalog.handle;
                    handle.shutdown();
                    handle.closed().await;
                }
            }
            self.prune(profile, gate);
        })
    }
}

// ## m4-t15-hub
/// The statement timeout of a catalog statement (alg/02 HV-INTRO-01 step 4: 60 s).
pub const META_STATEMENT_TIMEOUT: Duration = Duration::from_secs(60);

/// The engine's metadata connections from the profile's `SessionManager`.
pub struct HubMeta {
    sessions: Arc<dyn SessionLookup>,
    profile: ProfileId,
    single_session: bool,
    timeout: Duration,
    /// The manager a `meta_conn` last succeeded on: `meta_conn` checked that it is this
    /// profile's (review m5).
    verified: Mutex<std::sync::Weak<SessionManager>>,
}

impl HubMeta {
    /// The spec of a connection to `database`: always the hub's own profile, so `meta_conn`
    /// refuses it on another profile's manager.
    pub fn spec(&self, database: Option<Name>) -> MetaConnSpec {
        MetaConnSpec {
            profile: self.profile,
            database: database.map(|d| d.to_string()),
            statement_timeout: self.timeout,
        }
    }
}

impl MetaSource for HubMeta {
    fn acquire<'a>(
        &'a self,
        database: Option<Name>,
        cancel: &'a CancellationToken,
    ) -> BoxFuture<'a, Result<MetaConn, MetaError>> {
        Box::pin(async move {
            let Some(manager) = self.sessions.sessions(self.profile) else {
                return Err(MetaError::Unavailable);
            };
            let conn = manager.meta_conn(self.spec(database), cancel).await?;
            *self.verified.lock().unwrap_or_else(PoisonError::into_inner) =
                Arc::downgrade(&manager);
            Ok(conn)
        })
    }

    /// Asks only a manager `meta_conn` has accepted the hub's profile on (review m5); any
    /// other answer on a single-connection profile is "yes" (validation waits: the safe side).
    fn user_in_transaction(&self) -> bool {
        if !self.single_session {
            return false;
        }
        let verified = self
            .verified
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .upgrade();
        match (self.sessions.sessions(self.profile), verified) {
            (Some(m), Some(v)) if Arc::ptr_eq(&m, &v) => !m.open_transactions().is_empty(),
            _ => true,
        }
    }
}

/// The scope of a profile the user has not scoped (R-M4-14).
pub fn default_scope(dialect: DialectId) -> &'static str {
    match dialect {
        DialectId::Postgres | DialectId::PgWire(_) => "@:*",
        _ => "*",
    }
}

/// What `@` means before the app reports the session's own context: the profile's database
/// (SQLite: `main`). The PostgreSQL schema is the session's to tell (`set_current`).
fn current_of(profile: &Profile, dialect: DialectId) -> CurrentCtx {
    let database = profile.database.as_deref().map(Name::from);
    match dialect {
        DialectId::Sqlite => CurrentCtx {
            database: None,
            schema: Some(Name::from("main")),
            ident: None,
        },
        DialectId::MySql | DialectId::MariaDb => CurrentCtx {
            database: database.clone(),
            schema: database,
            ident: None,
        },
        _ => CurrentCtx {
            database,
            schema: None,
            ident: None,
        },
    }
}

/// Periodic validation (R-M4-20): `auto` is every 10 minutes for `local`/`dev` profiles.
fn validate_every(m: &MetadataSettings, profile: &Profile) -> Option<Duration> {
    match m.validate_interval_min {
        ValidateInterval::Off => None,
        ValidateInterval::Minutes(n) => Some(Duration::from_secs(u64::from(n.get()) * 60)),
        ValidateInterval::Auto => matches!(profile.environment, Some(EnvTag::Local | EnvTag::Dev))
            .then(|| Duration::from_secs(10 * 60)),
    }
}
// ## m4-t15-hub

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

#[cfg(test)]
mod tests {
    use super::*;

    struct NoSessions;
    impl SessionLookup for NoSessions {
        fn sessions(&self, _: ProfileId) -> Option<Arc<SessionManager>> {
            None
        }
    }

    #[test]
    fn every_connection_spec_names_the_hubs_own_profile() {
        let profile = ProfileId::new();
        let meta = HubMeta {
            sessions: Arc::new(NoSessions),
            profile,
            single_session: false,
            timeout: META_STATEMENT_TIMEOUT,
            verified: Mutex::new(std::sync::Weak::new()),
        };
        let spec = meta.spec(Some(Name::from("app")));
        assert_eq!(spec.profile, profile);
        assert_eq!(spec.database.as_deref(), Some("app"));
        assert!(!meta.user_in_transaction());
    }

    #[test]
    fn default_scopes_parse_and_validation_follows_r_m4_20() {
        for d in [
            DialectId::Postgres,
            DialectId::MySql,
            DialectId::MariaDb,
            DialectId::Sqlite,
        ] {
            assert!(parse_scope(default_scope(d), d).is_ok(), "{d:?}");
        }
        let m = MetadataSettings::default();
        let mut p = Profile::default();
        assert_eq!(validate_every(&m, &p), None);
        p.environment = Some(EnvTag::Dev);
        assert_eq!(validate_every(&m, &p), Some(Duration::from_secs(600)));
        p.environment = Some(EnvTag::Production);
        assert_eq!(validate_every(&m, &p), None);
    }
}
