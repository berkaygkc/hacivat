//! The live explorer (Task 24): a [`CatalogHost`] per app over `CatalogHub` — the cached tree at
//! start, cached-then-validated on connect, the engine stopped on disconnect, and the expanded
//! state kept in `workspace.json` (mocks 1–3).
//!
//! ## m4-t24-live
//!
//! **A thin wrapper** (R-M4-31). Engine, cache and key live in the hub; the host keeps, per
//! profile, the [`ProfileCatalog`] the hub made, what it last told the engine (scope, filters,
//! levels, current schema), the engine's progress, the mass deletions it holds and what the user
//! asked to refresh. Every hub call ([`CatalogSource`]) runs on the `CoreRuntime`; the UI only
//! awaits `tokio::sync` channels (spec §5.4 rule 2, the Q4 ruling). The explorer reads the
//! current snapshot synchronously ([`crate::explorer::adapter::set_catalog`]).
//!
//! **Coalesced notify** (rule 3). Per open profile a forwarder on the runtime collects the
//! catalog's events, the engine's events and its progress into an inbox and wakes the UI only
//! when the inbox was clean (one wake per batch). The UI drains the inbox, reads
//! `Catalog::current()` once and calls `set_catalog` only when the generation moved: however
//! many publishes a batch held, the explorer is notified once. Progress repaints only when it
//! changed (a `watch` with `send_if_modified` on the engine's side, an equality check here).
//!
//! **App start** (R-M4-15, PO-Q19, review I10). [`start`] (called by the workspace once
//! `workspace.json` was restored) looks, on the blocking pool, for each profile's cache
//! file. A plaintext profile with one is opened at once (its cached tree paints when the hub
//! answers). An encrypted profile is opened only when its root is expanded in the restored
//! workspace; every other encrypted profile stays collapsed with the key glyph and makes no
//! keyring call until the user expands or connects it ([`on_expand`]).
//!
//! **Connect / disconnect** (`connections::live` mirror). Connected: the catalog is opened (if it
//! was not; the cache, no server query) and `IntroHandle::on_connect` runs after the cached
//! namespaces were published stale, **as soon as the profile's root is expanded** (at once when
//! it already is, e.g. restored expanded; else at its first expansion). Deviation D-1 (task
//! report): the plan says "on connect"; reading the catalog of a profile whose subtree nobody
//! looks at opened metadata connections in every connect of the M2/M3 tests and consumed their
//! scripted driver answers. `connect_engine` holds the one condition to drop.
//! The saved scope, filters (`set_filters`, R-W3-2) and level pills (`set_level_override`) are
//! forwarded, and the current database and schema come from the **user's** session (the focused
//! console of the profile, `SessionSnapshot::schema`), never from the metadata connection
//! (`set_current`, T15 carry). Any state that is not connected stops the engine through the hub
//! (`CatalogHub::shutdown`); the cache file stays and the explorer keeps showing the last
//! snapshot ("Showing cached metadata from …").
//!
//! **Deltas.** Renames move expansion (Task 14). Dropped objects and `CatalogDelta::out_of_scope`
//! objects lose their expansion: an out-of-scope object is a removal from the tree, never a
//! deletion notice (T15 fix round 1 carry). `NamespaceDropped` toasts (R-M4-22).
//!
//! **Mass deletion** (D-039, R-M4-22, mock 8). The engine holds a guarded merge and announces
//! `PendingConfirmation`. When the user asked for that namespace's refresh, the confirmation
//! opens ([`super::mass_drop`]); otherwise nothing is removed and the namespace keeps its
//! "out of date" badge with the pending count in its tooltip until the user refreshes.
//!
//! **Expanded state** (PO-Q15). The tree's expanded keys are kept per profile in
//! [`ExpandedState`] and saved in `workspace.json` as `explorer.expanded: { <profile uuid>:
//! [TreeKey as hex] }`. At start they are restored only for keys present in the cached snapshot
//! (walked from the profile's root through restored keys only) or provided by a contributor;
//! the `Expand` events those restored rows raise start no introspection ([`on_expand`] ignores
//! them once): only what on-connect schedules runs.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use futures::future::{BoxFuture, Either, select};
use hacivat_introspect::ddl_effects::Target;
use hacivat_introspect::engine::{EngineEvent, Filters, Phase, Progress};
use hacivat_introspect::filter::{FilterRole, parse_filter};
use hacivat_introspect::hub::{
    CacheMode, CatalogHub, HubError, HubSettings, MemoryOnlyReason, ProfileCatalog, default_scope,
};
use hacivat_introspect::scope::{CurrentCtx, parse_scope};
use hacivat_l10n::{ArgValue, WhenValue};
use hacivat_meta::codec::FORMAT_MAJOR;
use hacivat_meta::merge::Trigger;
use hacivat_meta::snapshot::{CatalogEvent, ContainerKey};
use hacivat_meta::{
    CatalogDelta, CatalogSnapshot, Level, LoadState, Name, NsKey, NsKind, ObjectKey, ObjectKind,
};
use hacivat_proto::{DbmsKind, ProfileId};
use hacivat_session::LiveState;
use hacivat_session::profile::Profile;
use hacivat_settings::LocalPaths;
use hacivat_ui::text::TextFn;
use hacivat_ui::toast::{Toast, ToastKind, ToastQueue};
use hacivat_ui::tree::{ChildState, KeySet, TreeKey, TreeModel};
use hacivat_ui_core::gpui_surface::{App, Global, SharedString, Task, Window};
use sql_dialect::DialectId;
use tokio::runtime::Handle;
use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;

use super::adapter::{self, NodeInfo};
use super::level_pill;
use super::userdata;
use crate::connections::ConnectionsModel;
use crate::console::Consoles;
use crate::shell::AppState;

// ---- the seams M5 and M4b use ----------------------------------------------------------------

/// One database of a profile as the explorer shows it (R-M4-19).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DatabaseRow {
    /// The name, verbatim.
    pub name: String,
    /// In the profile's scope (read), or listed greyed with "Add to scope" (R-M4-14).
    pub in_scope: bool,
    /// The database the profile's connection is on.
    pub connected: bool,
}

/// What [`reveal`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RevealOutcome {
    /// The object's row is expanded to, selected and scrolled into view.
    Revealed,
    /// An ancestor is loading; the row is selected when it arrives.
    Pending,
    /// The loaded catalog has no such object.
    Missing,
    /// The explorer cannot reveal objects (no catalog for the profile, or not built yet).
    Unavailable,
}

// ---- the source ------------------------------------------------------------------------------

/// Where the host gets each profile's catalog: [`CatalogHub`] in the app; a test double over the
/// engine fakes in tests. Every method is called on the `CoreRuntime` (blocking ones on its
/// blocking pool), never on the UI thread.
pub trait CatalogSource: Send + Sync + 'static {
    /// The profile's catalog (made once; the hub fetches the key and opens the cache).
    fn ensure(&self, profile: Arc<Profile>) -> BoxFuture<'_, Result<ProfileCatalog, HubError>>;
    /// Stops the profile's engine and forgets its catalog (the cache file stays).
    fn shutdown(&self, profile: ProfileId) -> BoxFuture<'_, ()>;
    /// Whether the profile has a cache file (blocking: a file stat).
    fn has_cache(&self, profile: ProfileId) -> bool;
    // ## m4-t27-refresh-ui (named exception, ruling R-W5-11)
    /// The `[metadata]` settings changed: catalogs made from now on use them (a lock, no I/O).
    fn set_metadata(&self, _metadata: hacivat_settings::MetadataSettings) {}
}

/// The app's source: the hub and where the caches live.
pub struct HubSource {
    pub hub: Arc<CatalogHub>,
    pub paths: LocalPaths,
}

impl CatalogSource for HubSource {
    fn ensure(&self, profile: Arc<Profile>) -> BoxFuture<'_, Result<ProfileCatalog, HubError>> {
        self.hub.ensure(profile)
    }

    fn shutdown(&self, profile: ProfileId) -> BoxFuture<'_, ()> {
        self.hub.shutdown(profile)
    }

    fn has_cache(&self, profile: ProfileId) -> bool {
        self.paths
            .catalog_db(&profile.0.to_string(), u32::from(FORMAT_MAJOR))
            .exists()
    }

    // ## m4-t27-refresh-ui (named exception, ruling R-W5-11)
    fn set_metadata(&self, metadata: hacivat_settings::MetadataSettings) {
        self.hub.set_metadata(metadata);
    }
}

// ## m4-t27-refresh-ui (named exception, ruling R-W5-11)
/// Settings › Metadata changed the background check or a cache cap: the next catalog made (the
/// next connect) uses the saved values.
pub fn metadata_changed(cx: &App) {
    let Some(metadata) = cx.try_global::<AppState>().map(|s| s.settings.metadata) else {
        return;
    };
    if let Some(source) = cx
        .try_global::<CatalogHost>()
        .and_then(|h| h.source.clone())
    {
        source.set_metadata(metadata);
    }
}
// ## m4-t27-refresh-ui

// ---- the host --------------------------------------------------------------------------------

/// What the explorer's rows read from the live host for one profile (the adapter's
/// `ProfileInput::live`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LiveRow {
    /// The engine's running tasks (the progress rows).
    pub progress: Arc<Progress>,
    /// Mass deletions held by a background pass: the namespace and how many objects vanished.
    pub pending: BTreeMap<NsKey, u32>,
    /// The engine's automatic level of each database row and the major objects it counted (the
    /// grey pill of a database that has no explicit level; T15 review m8, T19 carry).
    pub db_levels: BTreeMap<Name, (Level, usize)>,
    /// Connected and the engine told (`on_connect`): a first load shows "Loading…".
    pub live: bool,
    /// An encrypted cache exists that was not opened (the root's key glyph, PO-Q19).
    pub locked: bool,
    // ## m4-t27-refresh-ui (named exception)
    /// Changes of open transactions that wait for COMMIT, per namespace (mock 9's stamp).
    pub after_commit: super::refresh_prompt::Changes,
    /// Refresh after DDL marked the schema lists stale (`CoordEvent::Stale { scope: true }`):
    /// a stale list is badged until it is read again.
    pub scope_stale: bool,
    /// A stale schema list was seen since `scope_stale` was set (review m-3: only a list that
    /// goes from stale to read clears the mark, whichever channel came first).
    pub scope_seen: bool,
    /// Objects a state check found changed (not read again) or gone, for the namespace
    /// generation it saw (mock 11's explorer tooltip; `super::stale`).
    pub stale_objects: BTreeMap<ObjectKey, super::stale::StaleObject>,
}

/// The catalog of one profile, while open. `Opening` and `Closing` exist only while the
/// profile's lifecycle task waits on the source.
enum Open {
    Closed,
    Opening,
    Ready(ProfileCatalog),
    Closing,
}

/// What the host last told one engine.
#[derive(Default, Clone, PartialEq)]
struct Sent {
    scope: Option<Option<String>>,
    filters: Option<String>,
    levels: BTreeMap<ObjectKey, Level>,
    current: Option<(Option<String>, Option<String>)>,
    // ## m4-t27-refresh-ui (named exception; T26 carry)
    /// The user data handed to the overlays (virtual FKs, annotations, "Introspect live").
    user_data: Option<Arc<hacivat_meta::userdata::ProfileUserData>>,
}

/// One profile's live state.
struct Entry {
    open: Open,
    // ## fix round 1 (I-1): one lifecycle task per profile serializes opens and closes; the
    // latest wish wins.
    /// The catalog should be open (the latest of `open` and `close`).
    want_open: bool,
    /// The lifecycle task runs.
    busy: bool,
    /// T20's Save is deleting the cache files: nothing opens until `reopen`.
    deleting: bool,
    /// `close` callers waiting for the lifecycle task to settle.
    closers: Vec<oneshot::Sender<()>>,
    /// The profile is connected: the engine runs `on_connect` once it is ready.
    want_connect: bool,
    /// `on_connect` was sent to the engine that is open now.
    connected: bool,
    mode: Option<CacheMode>,
    row: Arc<LiveRow>,
    /// Namespaces whose refresh the user asked for (a mass deletion then asks).
    user_asked: HashSet<NsKey>,
    /// The catalog generation the explorer shows.
    last_gen: u64,
    /// How many times the explorer was given a new snapshot (tests: coalescing).
    applied: u64,
    pump: Option<Arc<Pump>>,
    sent: Sent,
    /// Other PostgreSQL databases the last scope refresh could not list (`DatabaseUnlisted`).
    unlisted: BTreeSet<Name>,
    /// The session whose schema is followed (`set_current`), and its watcher.
    watched: Option<hacivat_session::session::SessionId>,
    watcher: Option<Task<()>>,
    _ui: Option<Task<()>>,
}

impl Default for Entry {
    fn default() -> Self {
        Entry {
            open: Open::Closed,
            want_open: false,
            busy: false,
            deleting: false,
            closers: Vec::new(),
            want_connect: false,
            connected: false,
            mode: None,
            row: Arc::default(),
            user_asked: HashSet::new(),
            last_gen: 0,
            applied: 0,
            pump: None,
            sent: Sent::default(),
            unlisted: BTreeSet::new(),
            watched: None,
            watcher: None,
            _ui: None,
        }
    }
}

/// The live explorer's state (a global).
#[derive(Default)]
pub struct CatalogHost {
    source: Option<Arc<dyn CatalogSource>>,
    runtime: Option<Handle>,
    entries: HashMap<ProfileId, Entry>,
    /// Moves whenever a row's live parts change (the explorer rebuilds its model).
    generation: u64,
    /// Expanded keys restored from `workspace.json`, not applied yet.
    restored: HashMap<ProfileId, Vec<TreeKey>>,
    /// Restored keys whose `Expand` event must not load anything.
    restoring: HashSet<TreeKey>,
    /// Profiles with an encrypted cache file that was not opened at start.
    locked: HashSet<ProfileId>,
    /// `ensure` calls sent to the source (tests).
    ensures: u64,
}

impl Global for CatalogHost {}

impl CatalogHost {
    fn entry(&mut self, id: ProfileId) -> &mut Entry {
        self.entries.entry(id).or_default()
    }
}

/// The expanded rows per profile, as `workspace.json` keeps them (a global; the workspace saves
/// when it changes).
#[derive(Default, Clone, PartialEq)]
pub struct ExpandedState {
    pub by_profile: BTreeMap<ProfileId, Vec<TreeKey>>,
}

impl Global for ExpandedState {}

/// Runs `fut` on `rt`; the receiver answers its output (the UI awaits it, never the I/O).
fn on_runtime<T: Send + 'static>(
    rt: &Handle,
    fut: impl std::future::Future<Output = T> + Send + 'static,
) -> oneshot::Receiver<T> {
    let (tx, rx) = oneshot::channel();
    rt.spawn(async move {
        let _ = tx.send(fut.await);
    });
    rx
}

fn source_and_runtime(cx: &App) -> Option<(Arc<dyn CatalogSource>, Handle)> {
    let host = cx.try_global::<CatalogHost>()?;
    Some((host.source.clone()?, host.runtime.clone()?))
}

/// The SQL dialect of a profile's engine.
pub fn dialect_of(dbms: DbmsKind) -> DialectId {
    match dbms {
        DbmsKind::MySql => DialectId::MySql,
        DbmsKind::MariaDb => DialectId::MariaDb,
        DbmsKind::Sqlite => DialectId::Sqlite,
        _ => DialectId::Postgres,
    }
}

/// Renders Fluent message `id` with `args` in the UI language.
pub fn text(cx: &App, id: &str, args: Vec<(&'static str, ArgValue)>) -> SharedString {
    match cx.try_global::<crate::l10n_global::L10nGlobal>() {
        Some(g) => g.l10n.t_dynamic(id, args).into(),
        None => SharedString::from(id.to_string()),
    }
}

fn toast(cx: &mut App, kind: ToastKind, id: &'static str, args: Vec<(&'static str, ArgValue)>) {
    let label: TextFn = std::rc::Rc::new(move |cx: &App| text(cx, id, args.clone()));
    ToastQueue::push(cx, Toast::new(kind, label));
}

fn profile(id: ProfileId, cx: &App) -> Option<Arc<Profile>> {
    cx.try_global::<ConnectionsModel>()?.get(id)
}

fn is_connected(id: ProfileId, cx: &App) -> bool {
    cx.try_global::<ConnectionsModel>()
        .is_some_and(|m| matches!(m.live(id), LiveState::Connected(_)))
}

/// Installs the host over the app's hub (`connections::boot`, after the model is set). Nothing
/// without a model or settings (P17).
pub fn install(cx: &mut App) {
    let Some(state) = cx.try_global::<AppState>() else {
        return;
    };
    let Some(model) = cx.try_global::<ConnectionsModel>() else {
        return;
    };
    let paths = state.paths.clone();
    let settings = HubSettings {
        paths: paths.clone(),
        metadata: state.settings.metadata,
    };
    let rt = model.runtime();
    let sessions: Arc<dyn hacivat_introspect::hub::SessionLookup> = model.manager().clone();
    let secrets = model.secrets().clone();
    // The embedded system catalogs (Task 23 fills them); decoding is lazy.
    let system = Arc::new(hacivat_meta::system::SystemCatalogs::from_embedded());
    // The hub builds SQLite's introspector with its per-schema readers (R-W3-10).
    let hub = CatalogHub::new_on(rt.clone(), sessions, settings, secrets, system);
    set_source(Arc::new(HubSource { hub, paths }), cx);
    // A changed explicit level, scope or filter reaches the engine (T20 carry).
    cx.observe_global::<userdata::UserData>(|cx| {
        let ids: Vec<ProfileId> = cx
            .try_global::<CatalogHost>()
            .map(|h| h.entries.keys().copied().collect())
            .unwrap_or_default();
        for id in ids {
            forward(id, cx);
        }
    })
    .detach();
    // The user's session (the focused console of a profile) decides the current schema.
    cx.observe_global::<Consoles>(|cx| {
        let ids: Vec<ProfileId> = cx
            .try_global::<CatalogHost>()
            .map(|h| h.entries.keys().copied().collect())
            .unwrap_or_default();
        for id in ids {
            follow_session(id, cx);
        }
    })
    .detach();
}

/// Replaces the source (tests; a later milestone's broker). Profiles already open keep theirs
/// until they close.
pub fn set_source(source: Arc<dyn CatalogSource>, cx: &mut App) {
    let rt = cx.try_global::<ConnectionsModel>().map(|m| m.runtime());
    let host = cx.default_global::<CatalogHost>();
    host.source = Some(source);
    if rt.is_some() {
        host.runtime = rt;
    }
}

/// The host's change counter (the explorer rebuilds when it moves).
pub fn generation(cx: &App) -> u64 {
    cx.try_global::<CatalogHost>().map_or(0, |h| h.generation)
}

/// What `profile`'s rows show of the live host.
pub fn row(profile: ProfileId, cx: &App) -> Option<Arc<LiveRow>> {
    let host = cx.try_global::<CatalogHost>()?;
    let mut row = host.entries.get(&profile).map(|e| e.row.clone());
    if host.locked.contains(&profile) {
        let mut r = row.as_deref().cloned().unwrap_or_default();
        r.locked = true;
        row = Some(Arc::new(r));
    }
    row
}

/// The handle of `profile`'s engine, while its catalog is open.
pub fn handle(profile: ProfileId, cx: &App) -> Option<hacivat_introspect::IntroHandle> {
    match &cx.try_global::<CatalogHost>()?.entries.get(&profile)?.open {
        Open::Ready(pc) => Some(pc.handle.clone()),
        _ => None,
    }
}

/// How `profile`'s catalog is persisted, once it was opened.
pub fn cache_mode(profile: ProfileId, cx: &App) -> Option<CacheMode> {
    cx.try_global::<CatalogHost>()?.entries.get(&profile)?.mode
}

/// How many snapshots the explorer was given for `profile` (tests: coalescing).
pub fn applied(profile: ProfileId, cx: &App) -> u64 {
    cx.try_global::<CatalogHost>()
        .and_then(|h| h.entries.get(&profile))
        .map_or(0, |e| e.applied)
}

/// How many `ensure` calls the host sent (tests: no keyring call at start).
pub fn ensures(cx: &App) -> u64 {
    cx.try_global::<CatalogHost>().map_or(0, |h| h.ensures)
}

/// Whether `profile`'s catalog is open (ready).
pub fn is_open(profile: ProfileId, cx: &App) -> bool {
    handle(profile, cx).is_some()
}

/// The mass deletions held for `profile` (background passes).
pub fn pending(profile: ProfileId, cx: &App) -> BTreeMap<NsKey, u32> {
    row(profile, cx)
        .map(|r| r.pending.clone())
        .unwrap_or_default()
}

/// Other databases the last scope refresh could not list.
pub fn unlisted(profile: ProfileId, cx: &App) -> BTreeSet<Name> {
    cx.try_global::<CatalogHost>()
        .and_then(|h| h.entries.get(&profile))
        .map(|e| e.unlisted.clone())
        .unwrap_or_default()
}

fn bump(cx: &mut App) {
    cx.default_global::<CatalogHost>().generation += 1;
}

// ---- app start --------------------------------------------------------------------------------

/// Keeps the expanded keys `workspace.json` restored (the workspace, before the explorer starts).
pub fn set_restored(map: BTreeMap<ProfileId, Vec<TreeKey>>, cx: &mut App) {
    cx.set_global(ExpandedState {
        by_profile: map.clone(),
    });
    cx.default_global::<CatalogHost>().restored = map.into_iter().collect();
}

/// Reads the `explorer` part of a saved workspace (`{ "expanded": { uuid: [hex] } }`).
pub fn parse_saved(value: &serde_json::Value) -> BTreeMap<ProfileId, Vec<TreeKey>> {
    let mut out = BTreeMap::new();
    let Some(map) = value.get("expanded").and_then(|v| v.as_object()) else {
        return out;
    };
    for (uuid, keys) in map {
        let Ok(id) = uuid.parse::<ProfileId>() else {
            continue;
        };
        let keys: Vec<TreeKey> = keys
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|k| k.as_str())
            .filter_map(|k| u128::from_str_radix(k, 16).ok())
            .map(TreeKey)
            .collect();
        out.insert(id, keys);
    }
    out
}

/// Adds the `explorer` part to a saved workspace `value` (the workspace's writer).
pub fn add_saved(value: &mut serde_json::Value, cx: &App) {
    let Some(state) = cx.try_global::<ExpandedState>() else {
        return;
    };
    let expanded: serde_json::Map<String, serde_json::Value> = state
        .by_profile
        .iter()
        .filter(|(_, keys)| !keys.is_empty())
        .map(|(id, keys)| {
            let mut hex: Vec<String> = keys.iter().map(|k| format!("{:032x}", k.0)).collect();
            hex.sort();
            (
                id.0.to_string(),
                serde_json::Value::Array(hex.into_iter().map(serde_json::Value::String).collect()),
            )
        })
        .collect();
    if expanded.is_empty() {
        return;
    }
    if let Some(obj) = value.as_object_mut() {
        obj.insert(
            "explorer".to_string(),
            serde_json::json!({ "expanded": expanded }),
        );
    }
}

/// App start (the workspace, once `workspace.json` was restored): opens
/// the cached catalog of every plaintext profile with a cache file, and of every encrypted one
/// whose root the restored workspace shows expanded (R-M4-15). The files are looked for on the
/// blocking pool.
pub fn start(cx: &mut App) {
    let Some((source, rt)) = source_and_runtime(cx) else {
        return;
    };
    let Some(model) = cx.try_global::<ConnectionsModel>() else {
        return;
    };
    let restored: HashSet<TreeKey> = cx
        .try_global::<CatalogHost>()
        .map(|h| h.restored.values().flatten().copied().collect())
        .unwrap_or_default();
    let profiles: Vec<(ProfileId, bool, bool)> = model
        .profiles()
        .iter()
        .filter(|p| {
            matches!(
                p.engine,
                DbmsKind::Postgres | DbmsKind::MySql | DbmsKind::MariaDb | DbmsKind::Sqlite
            )
        })
        .map(|p| {
            (
                p.id,
                p.catalog_sensitive(),
                restored.contains(&adapter::profile_key(p.id)),
            )
        })
        .collect();
    if profiles.is_empty() {
        return;
    }
    let ids: Vec<ProfileId> = profiles.iter().map(|p| p.0).collect();
    let rx = on_runtime(&rt, {
        let rt = rt.clone();
        async move {
            rt.spawn_blocking(move || {
                ids.into_iter()
                    .map(|id| (id, source.has_cache(id)))
                    .collect::<HashMap<_, _>>()
            })
            .await
            .unwrap_or_default()
        }
    });
    cx.spawn(async move |cx| {
        let Ok(found) = rx.await else {
            return;
        };
        cx.update(|cx| {
            for (id, sensitive, expanded) in profiles {
                if !found.get(&id).copied().unwrap_or(false) {
                    continue;
                }
                if !sensitive || expanded {
                    open(id, cx);
                } else {
                    // Collapsed with the key glyph; no keyring call (PO-Q19).
                    cx.default_global::<CatalogHost>().locked.insert(id);
                    bump(cx);
                }
            }
        });
    })
    .detach();
}

// ---- open, connect, close ---------------------------------------------------------------------

/// Asks for `id`'s catalog to be open (through the source, on the runtime). Opens and closes of
/// one profile run one at a time in its lifecycle task, and the latest wish wins (fix round 1,
/// I-1): an open answered after a close is closed again by the same task, never by a stale
/// shutdown that could stop a newer engine.
pub fn open(id: ProfileId, cx: &mut App) {
    if profile(id, cx).is_none() || source_and_runtime(cx).is_none() {
        return;
    }
    let host = cx.default_global::<CatalogHost>();
    host.locked.remove(&id);
    host.entry(id).want_open = true;
    kick(id, cx);
    bump(cx);
}

/// One step of a profile's lifecycle task.
enum Step {
    Open(Arc<dyn CatalogSource>, Handle, Arc<Profile>),
    Close(Arc<dyn CatalogSource>, Handle),
    Done,
}

/// What `id`'s lifecycle task does next: open when wished (and not being deleted) and closed,
/// close when not wished and open, else nothing.
fn next_step(id: ProfileId, cx: &mut App) -> Step {
    let src = source_and_runtime(cx);
    let p = profile(id, cx);
    let host = cx.default_global::<CatalogHost>();
    let entry = host.entry(id);
    let wanted = entry.want_open && !entry.deleting;
    let (opening, closing) = (
        matches!(entry.open, Open::Closed) && wanted,
        matches!(entry.open, Open::Ready(_)) && !wanted,
    );
    if opening {
        let Some(((s, rt), p)) = src.zip(p) else {
            entry.want_open = false;
            return Step::Done;
        };
        entry.open = Open::Opening;
        host.ensures += 1;
        return Step::Open(s, rt, p);
    }
    if closing {
        teardown(entry);
        entry.open = if src.is_some() {
            Open::Closing
        } else {
            Open::Closed
        };
        host.generation += 1;
        return match src {
            Some((s, rt)) => Step::Close(s, rt),
            None => Step::Done,
        };
    }
    Step::Done
}

/// Stops what an open entry runs in the UI (its pump, its session watch) and forgets what it
/// told the engine.
fn teardown(entry: &mut Entry) {
    entry.connected = false;
    entry.user_asked.clear();
    entry.watched = None;
    entry.watcher = None;
    if let Some(pump) = entry.pump.take() {
        pump.stop.cancel();
    }
    entry._ui = None;
    let mut row = (*entry.row).clone();
    row.progress = Arc::default();
    row.live = false;
    row.db_levels.clear();
    // ## m4-t27-refresh-ui (named exception): the coordinator goes with the engine.
    row.after_commit.clear();
    row.scope_stale = false;
    row.scope_seen = false;
    row.stale_objects.clear();
    entry.row = Arc::new(row);
}

/// Starts `id`'s lifecycle task unless it runs (it reads the latest wish at every step).
fn kick(id: ProfileId, cx: &mut App) {
    let entry = cx.default_global::<CatalogHost>().entry(id);
    if entry.busy {
        return;
    }
    entry.busy = true;
    cx.spawn(async move |cx| {
        loop {
            match cx.update(|cx| next_step(id, cx)) {
                Step::Done => {
                    cx.update(|cx| {
                        let entry = cx.default_global::<CatalogHost>().entry(id);
                        entry.busy = false;
                        for tx in entry.closers.drain(..) {
                            let _ = tx.send(());
                        }
                        bump(cx);
                    });
                    break;
                }
                Step::Open(source, rt, p) => {
                    let rx = on_runtime(&rt, async move { source.ensure(p).await });
                    let answer = rx.await.unwrap_or(Err(HubError::Shutdown));
                    cx.update(|cx| opened(id, answer, cx));
                }
                Step::Close(source, rt) => {
                    // ## m4-t27-refresh-ui (named exception): the coordinator goes with the
                    // engine; what it owes waits for the next one.
                    cx.update(|cx| super::refresh_prompt::catalog_closed(id, cx));
                    let rx = on_runtime(&rt, async move { source.shutdown(id).await });
                    let _ = rx.await;
                    cx.update(|cx| {
                        cx.default_global::<CatalogHost>().entry(id).open = Open::Closed;
                    });
                }
            }
        }
    })
    .detach();
}

/// The source answered `id`'s open.
fn opened(id: ProfileId, answer: Result<ProfileCatalog, HubError>, cx: &mut App) {
    let pc = match answer {
        Ok(pc) => pc,
        Err(e) => {
            tracing::debug!(?e, "explorer: no catalog for the profile");
            let entry = cx.default_global::<CatalogHost>().entry(id);
            entry.open = Open::Closed;
            // A failed open is not retried in a loop: the next connect or expansion asks again.
            entry.want_open = false;
            bump(cx);
            return;
        }
    };
    let still = cx
        .try_global::<CatalogHost>()
        .and_then(|h| h.entries.get(&id))
        .is_some_and(|e| e.want_open && !e.deleting);
    if !still {
        // Closed meanwhile: the lifecycle task closes it next; nothing of it is shown.
        cx.default_global::<CatalogHost>().entry(id).open = Open::Ready(pc);
        return;
    }
    let name = profile(id, cx).map(|p| p.name.clone()).unwrap_or_default();
    match pc.cache {
        CacheMode::MemoryOnly(MemoryOnlyReason::KeyringUnavailable) => toast(
            cx,
            ToastKind::Error,
            "hv-explorer-memory-only",
            vec![("name", ArgValue::Text(name))],
        ),
        CacheMode::MemoryOnly(MemoryOnlyReason::CacheUnavailable) => {
            toast(
                cx,
                ToastKind::Info,
                "model-cache-disabled-by-policy",
                Vec::new(),
            );
        }
        CacheMode::Persistent {
            rebuilt: Some(reason),
            ..
        } => toast(
            cx,
            ToastKind::Info,
            "model-cache-rebuilt",
            vec![("reason", ArgValue::Text(reason.ftl_reason().to_string()))],
        ),
        CacheMode::Persistent { rebuilt: None, .. } => {}
    }
    let pump = Pump::start(id, &pc, cx);
    // ## m4-t27-refresh-ui (named exception): the profile's refresh-after-DDL coordinator.
    if let Some(p) = profile(id, cx) {
        super::refresh_prompt::catalog_opened(id, pc.handle.clone(), p.engine, cx);
    }
    {
        let host = cx.default_global::<CatalogHost>();
        let entry = host.entry(id);
        entry.mode = Some(pc.cache);
        entry.open = Open::Ready(pc);
        entry.connected = false;
        entry.sent = Sent::default();
        // A new catalog: any published generation of it is shown, whatever the old one's
        // number was; an empty one (generation 0) leaves the last snapshot in place.
        entry.last_gen = 0;
        entry.pump = Some(pump.0);
        entry._ui = Some(pump.1);
    }
    // The cached namespaces were published (stale) before the hub answered.
    flush(id, cx);
    forward(id, cx);
    follow_session(id, cx);
    let want = cx
        .try_global::<CatalogHost>()
        .and_then(|h| h.entries.get(&id))
        .is_some_and(|e| e.want_connect);
    if want {
        connect_engine(id, cx);
    }
    bump(cx);
}

/// Tells `id`'s open engine the profile is connected (on-connect, HV-INTRO-01 steps 1–3).
fn connect_engine(id: ProfileId, cx: &mut App) {
    // Deviation D-1 (see the module docs): the catalog is read once the profile's subtree is in
    // sight.
    if !root_expanded(id, cx) {
        return;
    }
    let host = cx.default_global::<CatalogHost>();
    let entry = host.entry(id);
    let Open::Ready(pc) = &entry.open else {
        return;
    };
    if entry.connected {
        return;
    }
    entry.connected = true;
    let mut row = (*entry.row).clone();
    row.live = true;
    entry.row = Arc::new(row);
    drop(pc.handle.on_connect());
    bump(cx);
    // ## m4-t27-refresh-ui (named exception, T21 m6): refreshes owed for want of a connection.
    super::refresh_prompt::reconnected(id, cx);
}

/// Whether `id`'s root row is expanded in the explorer.
fn root_expanded(id: ProfileId, cx: &App) -> bool {
    tree_of(cx).is_some_and(|t| t.read(cx).is_expanded(adapter::profile_key(id)))
}

/// The connections mirror saw `state` for `id` (`connections::live`, named lines).
pub fn on_live_state(id: ProfileId, state: &LiveState, cx: &mut App) {
    if cx.try_global::<CatalogHost>().is_none() {
        return;
    }
    match state {
        LiveState::Connected(_) => {
            cx.default_global::<CatalogHost>().entry(id).want_connect = true;
            let ready = matches!(
                cx.try_global::<CatalogHost>()
                    .and_then(|h| h.entries.get(&id))
                    .map(|e| &e.open),
                Some(Open::Ready(_))
            );
            if ready {
                connect_engine(id, cx);
                forward(id, cx);
                follow_session(id, cx);
                // ## m4-t27-refresh-ui (named exception, T21 m6): a reconnect that kept the
                // engine (connect_engine did nothing) runs what the coordinator owes.
                super::refresh_prompt::reconnected(id, cx);
            } else {
                open(id, cx);
            }
        }
        // A connect or a reconnect in progress changes nothing yet.
        LiveState::Connecting(_) | LiveState::Reconnecting { .. } => {}
        LiveState::Disconnected | LiveState::Failed(_) | LiveState::Lost { .. } => {
            let was = cx
                .try_global::<CatalogHost>()
                .and_then(|h| h.entries.get(&id))
                .is_some_and(|e| e.want_connect);
            if was {
                cx.default_global::<CatalogHost>().entry(id).want_connect = false;
                drop(close(id, cx));
            }
        }
    }
}

/// Stops `id`'s engine through the source (pending cache saves are written); the explorer
/// keeps the last snapshot and the cache file stays. The task resolves when the profile's
/// lifecycle task has settled (after an open still in flight, which it then closes).
pub fn close(id: ProfileId, cx: &mut App) -> Task<()> {
    let Some(entry) = cx.default_global::<CatalogHost>().entries.get_mut(&id) else {
        return Task::ready(());
    };
    entry.want_open = false;
    let (tx, rx) = oneshot::channel();
    entry.closers.push(tx);
    kick(id, cx);
    cx.spawn(async move |_| {
        let _ = rx.await;
    })
}

/// Closes `id`'s cache store before its files are deleted (T20's Save, T19 carry): nothing
/// opens it (a connect, an expansion, an open in flight) until [`reopen`]. The task resolves
/// once the store is closed.
pub fn release_cache(id: ProfileId, cx: &mut App) -> Task<()> {
    cx.default_global::<CatalogHost>().entry(id).deleting = true;
    close(id, cx)
}

/// Ends [`release_cache`]: a connected profile's catalog opens again, made from the profile as
/// stored now (after Save, so an "encrypt on" save never recreates a plaintext cache). When the
/// files were `deleted` and the profile is not reopened, the explorer drops its old snapshot
/// (it would claim a cache that no longer exists; review m-9).
pub fn reopen(id: ProfileId, deleted: bool, cx: &mut App) {
    let host = cx.default_global::<CatalogHost>();
    let entry = host.entry(id);
    entry.deleting = false;
    let again = entry.want_connect;
    if again && is_connected(id, cx) {
        cx.default_global::<CatalogHost>().entry(id).want_open = true;
        kick(id, cx);
    } else if deleted {
        cx.default_global::<CatalogHost>().entry(id).last_gen = 0;
        adapter::set_catalog(id, None, None, cx);
    }
    bump(cx);
}

/// Whether `id`'s engine was told the profile is connected (tests: D-1).
pub fn engine_connected(id: ProfileId, cx: &App) -> bool {
    cx.try_global::<CatalogHost>()
        .and_then(|h| h.entries.get(&id))
        .is_some_and(|e| e.connected)
}

/// Whether `id`'s lifecycle task has settled (tests: a processed signal).
pub fn settled(id: ProfileId, cx: &App) -> bool {
    cx.try_global::<CatalogHost>()
        .and_then(|h| h.entries.get(&id))
        .is_none_or(|e| !e.busy)
}

// ---- what the engine is told ------------------------------------------------------------------

fn ns_of_level_key(key: &ObjectKey) -> Option<NsKey> {
    match key.kind {
        ObjectKind::Database => key.path.first().map(|d| NsKey::database(d.clone())),
        ObjectKind::Schema => Some(NsKey::schema(key.path.iter().cloned())),
        _ => None,
    }
}

/// Forwards the profile's saved scope, filters and level pills to its engine when they changed
/// since last sent (T20 and T15 carries).
fn forward(id: ProfileId, cx: &mut App) {
    let Some(h) = handle(id, cx) else {
        return;
    };
    let Some(p) = profile(id, cx) else {
        return;
    };
    let Some(data) = userdata::get(id, cx) else {
        return;
    };
    let dialect = dialect_of(p.engine);
    let sent = cx
        .try_global::<CatalogHost>()
        .and_then(|h| h.entries.get(&id))
        .map(|e| e.sent.clone())
        .unwrap_or_default();
    let mut next = sent.clone();
    // The scope (HV-INTRO-02): `None` is the default (R-M4-14).
    if sent.scope.as_ref() != Some(&data.scope) {
        let text = data.scope.clone();
        let pattern = parse_scope(text.as_deref().unwrap_or(default_scope(dialect)), dialect);
        match pattern {
            Ok(pattern) => {
                h.set_scope(pattern);
                next.scope = Some(text);
            }
            Err(e) => tracing::debug!(?e, "explorer: the saved scope does not parse"),
        }
    }
    // The scope filters, one profile-wide filter (R-W3-2). Review m-7: joining the entries of
    // several containers with `||` unions their positive items (T12's meaning), so the result
    // reads as one profile-wide filter only while no writer keys entries per container (none
    // does in M4). A per-container writer must route each entry to its container instead.
    let filter_text = data
        .scope_filters
        .values()
        .filter(|t| !t.trim().is_empty())
        .cloned()
        .collect::<Vec<_>>()
        .join("||");
    if sent.filters.as_ref() != Some(&filter_text) {
        let filters = if filter_text.is_empty() {
            Some(Filters::default())
        } else {
            parse_filter(&filter_text, dialect, FilterRole::Introspect)
                .ok()
                .map(|f| Filters { scope: Some(f) })
        };
        if let Some(filters) = filters {
            h.set_filters(filters);
            next.filters = Some(filter_text);
        }
    }
    // The pills (T19 carry): every change since last sent.
    if sent.levels != data.level_overrides {
        for (key, level) in &data.level_overrides {
            if sent.levels.get(key) != Some(level)
                && let Some(ns) = ns_of_level_key(key)
            {
                h.set_level_override(ContainerKey::Namespace(ns), Some(*level));
            }
        }
        for key in sent.levels.keys() {
            if !data.level_overrides.contains_key(key)
                && let Some(ns) = ns_of_level_key(key)
            {
                h.set_level_override(ContainerKey::Namespace(ns), None);
            }
        }
        next.levels = data.level_overrides.clone();
    }
    // ## m4-t27-refresh-ui (named exception; T26 carry): the overlays' user layers and the
    // persisted "Introspect live" choices (`CatalogChange::SetUserData`).
    // Only what the overlays read, and only when it changed (none sent counts as empty), so a
    // profile without virtual foreign keys, annotations or live choices publishes nothing.
    let overlay_part = |d: &hacivat_meta::userdata::ProfileUserData| {
        (
            d.virtual_fks.clone(),
            d.annotations.clone(),
            d.live_system.clone(),
        )
    };
    let before = sent
        .user_data
        .as_deref()
        .map_or_else(|| overlay_part(&Default::default()), overlay_part);
    if overlay_part(&data) != before {
        h.set_user_data(data.clone());
        next.user_data = Some(data.clone());
    }
    if next != sent {
        cx.default_global::<CatalogHost>().entry(id).sent = next;
    }
}

/// The session of `id`'s user: the focused console's when it is bound to `id`, else the first
/// console of `id` with a session.
fn user_session(id: ProfileId, cx: &App) -> Option<hacivat_session::session::SessionHandle> {
    let focused = Consoles::focused(cx)
        .and_then(|c| Consoles::get(cx, c))
        .filter(|s| s.profile == Some(id))
        .and_then(|s| s.session.clone());
    focused.or_else(|| {
        Consoles::ids(cx)
            .into_iter()
            .filter_map(|c| Consoles::get(cx, c))
            .filter(|s| s.profile == Some(id))
            .find_map(|s| s.session.clone())
    })
}

/// The database `id`'s user session is on, when a session of it has read its context
/// (`SessionSnapshot::schema`).
pub fn session_database(id: ProfileId, cx: &App) -> Option<String> {
    let session = user_session(id, cx)?;
    let snapshot = session.state();
    let db = snapshot.borrow().schema.as_ref()?.database.clone();
    db.filter(|d| !d.is_empty())
}

/// Follows the current database and schema of `id`'s user session (`set_current`; T15 carry:
/// from the user's session, never the metadata connection).
fn follow_session(id: ProfileId, cx: &mut App) {
    if handle(id, cx).is_none() {
        return;
    }
    let session = user_session(id, cx);
    let sid = session.as_ref().map(|s| s.id());
    let same = cx
        .try_global::<CatalogHost>()
        .and_then(|h| h.entries.get(&id))
        .is_some_and(|e| e.watched == sid);
    if same {
        return;
    }
    let watcher = session.map(|s| {
        let mut rx = s.state();
        cx.spawn(async move |cx| {
            loop {
                let schema = rx.borrow_and_update().schema.clone();
                cx.update(|cx| set_current(id, schema, cx));
                if rx.changed().await.is_err() {
                    break;
                }
            }
        })
    });
    let entry = cx.default_global::<CatalogHost>().entry(id);
    entry.watched = sid;
    entry.watcher = watcher;
}

/// Tells `id`'s engine the user's current database and schema when they changed. Unknown parts
/// stay unknown (`None`), never an empty name.
fn set_current(id: ProfileId, schema: Option<hacivat_session::session::SchemaCtx>, cx: &mut App) {
    let Some(schema) = schema else {
        return;
    };
    let Some(h) = handle(id, cx) else {
        return;
    };
    let Some(p) = profile(id, cx) else {
        return;
    };
    let nonempty = |s: Option<String>| s.filter(|s| !s.is_empty());
    let pair = (nonempty(schema.database), nonempty(schema.schema));
    let sent = cx
        .try_global::<CatalogHost>()
        .and_then(|h| h.entries.get(&id))
        .and_then(|e| e.sent.current.clone());
    if sent.as_ref() == Some(&pair) {
        return;
    }
    let (database, schema_name) = pair.clone();
    let current = match p.engine {
        // A MySQL database is its schema.
        DbmsKind::MySql | DbmsKind::MariaDb => CurrentCtx {
            database: database.as_deref().map(Name::from),
            schema: database.as_deref().map(Name::from),
            ident: None,
        },
        DbmsKind::Sqlite => CurrentCtx {
            database: None,
            schema: Some(Name::from(schema_name.as_deref().unwrap_or("main"))),
            ident: None,
        },
        _ => CurrentCtx {
            database: database
                .as_deref()
                .or(p.database.as_deref())
                .map(Name::from),
            schema: schema_name.as_deref().map(Name::from),
            ident: None,
        },
    };
    h.set_current(current);
    cx.default_global::<CatalogHost>().entry(id).sent.current = Some(pair);
}

// ---- the pump -----------------------------------------------------------------------------

/// What one batch of the forwarder holds for the UI.
#[derive(Default)]
struct Inbox {
    deltas: Vec<Arc<CatalogDelta>>,
    // ## m4-t33b-carries (T24 carry): with the event's sample of names.
    pending: Vec<(NsKey, u32, Arc<[ObjectKey]>)>,
    events: Vec<EngineEvent>,
    progress: Option<Progress>,
}

/// One open profile's forwarder: an inbox and a wake that is sent only when it was clean.
struct Pump {
    inbox: Mutex<Inbox>,
    dirty: AtomicBool,
    wake: mpsc::UnboundedSender<()>,
    stop: CancellationToken,
    /// Wakes sent (tests).
    wakes: AtomicU64,
    /// The newest catalog generation whose `Published` event was taken in (tests; a lagging
    /// receiver still takes in the newest events).
    received: AtomicU64,
}

impl Pump {
    fn push(&self, f: impl FnOnce(&mut Inbox)) {
        f(&mut self.inbox.lock().unwrap_or_else(PoisonError::into_inner));
        if !self.dirty.swap(true, Ordering::SeqCst) {
            self.wakes.fetch_add(1, Ordering::Relaxed);
            let _ = self.wake.send(());
        }
    }

    /// Starts the forwarder on the runtime and the UI task that drains it.
    fn start(profile: ProfileId, pc: &ProfileCatalog, cx: &mut App) -> (Arc<Pump>, Task<()>) {
        let (wake, mut woken) = mpsc::unbounded_channel();
        let pump = Arc::new(Pump {
            inbox: Mutex::new(Inbox::default()),
            dirty: AtomicBool::new(false),
            wake,
            stop: CancellationToken::new(),
            wakes: AtomicU64::new(0),
            received: AtomicU64::new(0),
        });
        if let Some(rt) = cx
            .try_global::<CatalogHost>()
            .and_then(|h| h.runtime.clone())
        {
            let mut catalog = pc.catalog.subscribe();
            let mut events = pc.handle.events();
            let mut progress = pc.handle.progress();
            let p = pump.clone();
            rt.spawn(async move {
                loop {
                    let r = select(Box::pin(p.stop.cancelled()), Box::pin(catalog.recv())).await;
                    match r {
                        Either::Left(_) => break,
                        Either::Right((Ok(CatalogEvent::Published { delta, generation }), _)) => {
                            p.push(|i| i.deltas.push(delta));
                            p.received.fetch_max(generation, Ordering::SeqCst);
                        }
                        Either::Right((
                            Ok(CatalogEvent::PendingConfirmation { ns, count, sample }),
                            _,
                        )) => p.push(|i| i.pending.push((ns, count, sample))),
                        // A lagging receiver re-reads `current()` (the flush does anyway); the
                        // lagged deltas' renames and drops do not reach the expansion then.
                        Either::Right((
                            Ok(_) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)),
                            _,
                        )) => p.push(|_| {}),
                        // Fix round 1 (m-4): the catalog is gone; never spin.
                        Either::Right((Err(_), _)) => break,
                    }
                    if p.stop.is_cancelled() {
                        break;
                    }
                }
            });
            let p = pump.clone();
            rt.spawn(async move {
                loop {
                    match select(Box::pin(p.stop.cancelled()), Box::pin(events.recv())).await {
                        Either::Left(_) => break,
                        Either::Right((Ok(e), _)) => p.push(|i| i.events.push(e)),
                        Either::Right((
                            Err(tokio::sync::broadcast::error::RecvError::Lagged(_)),
                            _,
                        )) => {}
                        Either::Right((Err(_), _)) => break,
                    }
                }
            });
            let p = pump.clone();
            rt.spawn(async move {
                loop {
                    let changed = matches!(
                        select(Box::pin(p.stop.cancelled()), Box::pin(progress.changed())).await,
                        Either::Right((Ok(()), _))
                    );
                    if !changed {
                        break;
                    }
                    let now = progress.borrow_and_update().clone();
                    p.push(|i| i.progress = Some(now));
                }
            });
        }
        let me = pump.clone();
        let task = cx.spawn(async move |cx| {
            while woken.recv().await.is_some() {
                if me.stop.is_cancelled() {
                    break;
                }
                cx.update(|cx| flush(profile, cx));
            }
        });
        (pump, task)
    }
}

/// Puts engine event `e` into `profile`'s inbox as the engine's forwarder would (test seam,
/// feature `test-support`: events the fakes cannot produce, such as `CacheWriteFailed`).
#[cfg(feature = "test-support")]
pub fn inject_event(profile: ProfileId, e: EngineEvent, cx: &App) -> bool {
    let Some(pump) = cx
        .try_global::<CatalogHost>()
        .and_then(|h| h.entries.get(&profile))
        .and_then(|e| e.pump.clone())
    else {
        return false;
    };
    pump.push(|i| i.events.push(e));
    true
}

/// The newest catalog generation the forwarder of `profile` has taken in (tests: a processed
/// signal).
pub fn forwarded_generation(profile: ProfileId, cx: &App) -> u64 {
    cx.try_global::<CatalogHost>()
        .and_then(|h| h.entries.get(&profile))
        .and_then(|e| e.pump.as_ref())
        .map_or(0, |p| p.received.load(Ordering::SeqCst))
}

/// Wakes the UI task of `profile` (tests).
pub fn wakes(profile: ProfileId, cx: &App) -> u64 {
    cx.try_global::<CatalogHost>()
        .and_then(|h| h.entries.get(&profile))
        .and_then(|e| e.pump.as_ref())
        .map_or(0, |p| p.wakes.load(Ordering::Relaxed))
}

/// Applies one batch of `id`'s inbox: one `set_catalog` when the generation moved.
fn flush(id: ProfileId, cx: &mut App) {
    let (pc, pump) = {
        let Some(e) = cx
            .try_global::<CatalogHost>()
            .and_then(|h| h.entries.get(&id))
        else {
            return;
        };
        let Open::Ready(pc) = &e.open else {
            return;
        };
        (pc.clone(), e.pump.clone())
    };
    let inbox = match &pump {
        Some(p) => {
            p.dirty.store(false, Ordering::SeqCst);
            std::mem::take(&mut *p.inbox.lock().unwrap_or_else(PoisonError::into_inner))
        }
        None => Inbox::default(),
    };
    let snap = pc.catalog.current();
    let mut changed = false;
    let last = cx
        .try_global::<CatalogHost>()
        .and_then(|h| h.entries.get(&id))
        .map_or(0, |e| e.last_gen);
    if snap.generation != last {
        let mut combined = CatalogDelta::default();
        let mut gone: Vec<ObjectKey> = Vec::new();
        for d in &inbox.deltas {
            combined.renamed.extend(d.renamed.iter().cloned());
            gone.extend(d.dropped.iter().cloned());
            // A scope change: removed from the tree, never a deletion notice.
            gone.extend(d.out_of_scope.iter().cloned());
        }
        {
            let entry = cx.default_global::<CatalogHost>().entry(id);
            entry.last_gen = snap.generation;
            entry.applied += 1;
        }
        adapter::set_catalog(id, Some(snap.clone()), Some(&combined), cx);
        forget_expansion(id, &gone, cx);
        // After the explorer rebuilt its model from the new catalog (its observer runs first).
        cx.defer(move |cx| apply_restored(id, cx));
        changed = true;
    }
    let mut dialogs: Vec<(NsKey, u32, Arc<[ObjectKey]>)> = Vec::new();
    {
        let host = cx.default_global::<CatalogHost>();
        let entry = host.entry(id);
        let mut row = (*entry.row).clone();
        if let Some(p) = inbox.progress
            && *row.progress != p
        {
            row.progress = Arc::new(p);
        }
        for (ns, count, sample) in inbox.pending {
            if entry.user_asked.remove(&ns) {
                dialogs.push((ns, count, sample));
            } else {
                row.pending.insert(ns, count);
            }
        }
        for e in &inbox.events {
            match e {
                EngineEvent::PendingWithdrawn { ns } => {
                    row.pending.remove(ns);
                }
                EngineEvent::DatabaseUnlisted { db } => {
                    entry.unlisted.insert(db.clone());
                }
                _ => {}
            }
        }
        // A pending deletion of a namespace that is no longer stale was answered.
        // ## m4-t27-refresh-ui (named exception, R-W4-8): a `Partial` namespace carries no stale
        // bit, so its held deletion stays until the engine withdraws it.
        row.pending.retain(|ns, _| {
            matches!(
                snap.states.get(&ContainerKey::Namespace(ns.clone())),
                Some(LoadState::Loaded { stale: true, .. } | LoadState::Partial { .. })
            )
        });
        // ## m4-t27-refresh-ui (named exception): a schema list read again clears the mark
        // (review m-3: once a stale list was seen, whichever channel came first).
        if row.scope_stale {
            if any_stale_list(&snap) {
                row.scope_seen = true;
            } else if row.scope_seen {
                row.scope_stale = false;
                row.scope_seen = false;
            }
        }
        // Review m-8: a stale-object mark of a namespace published since is dropped.
        if changed && !row.stale_objects.is_empty() {
            row.stale_objects.retain(|k, m| {
                snap.find(k)
                    .is_some_and(|h| h.ns.generation() == m.generation())
            });
        }
        // Fix round 1 (m-5): only when the catalog moved (a level change re-reads, so it moves).
        if changed {
            row.db_levels = db_levels(&pc, &snap);
        }
        if row != *entry.row {
            entry.row = Arc::new(row);
            changed = true;
        }
        if changed {
            host.generation += 1;
        }
    }
    for e in inbox.events {
        notice(id, e, cx);
    }
    for (ns, count, sample) in dialogs {
        super::mass_drop::ask(id, ns, count, sample, cx);
    }
}

// ## m4-t27-refresh-ui (named exception)
/// Whether a schema list of `snap` is loaded and marked stale.
fn any_stale_list(snap: &CatalogSnapshot) -> bool {
    snap.states.iter().any(|(k, s)| {
        matches!(k, ContainerKey::SchemaList { .. })
            && matches!(s, LoadState::Loaded { stale: true, .. })
    })
}

/// Sets what refresh after DDL shows on `id`'s rows: the namespaces waiting for COMMIT and,
/// when `scope_stale` is given, the stale schema-list mark.
pub fn set_refresh_marks(
    id: ProfileId,
    after_commit: Option<super::refresh_prompt::Changes>,
    scope_stale: Option<bool>,
    cx: &mut App,
) {
    let host = cx.default_global::<CatalogHost>();
    let entry = host.entry(id);
    let mut row = (*entry.row).clone();
    if let Some(a) = after_commit {
        row.after_commit = a;
    }
    if let Some(s) = scope_stale {
        // Review m-3: the engine's stale publish may have come first.
        let seen = s
            && match &entry.open {
                Open::Ready(pc) => any_stale_list(&pc.catalog.current()),
                _ => false,
            };
        row.scope_seen |= seen;
        if !s {
            row.scope_seen = false;
        }
        row.scope_stale = s;
    }
    if row != *entry.row {
        entry.row = Arc::new(row);
        host.generation += 1;
    }
}
/// Sets (`Some`) or clears what a state check found about `key` of `id` (`super::stale`).
pub fn set_stale_object(
    id: ProfileId,
    key: ObjectKey,
    mark: Option<super::stale::StaleObject>,
    cx: &mut App,
) {
    let host = cx.default_global::<CatalogHost>();
    let entry = host.entry(id);
    if entry.row.stale_objects.get(&key) == mark.as_ref() {
        return;
    }
    let mut row = (*entry.row).clone();
    match mark {
        Some(m) => {
            row.stale_objects.insert(key, m);
        }
        None => {
            row.stale_objects.remove(&key);
        }
    }
    entry.row = Arc::new(row);
    host.generation += 1;
}
// ## m4-t27-refresh-ui

/// The automatic level of each database of `snap` with at least one loaded schema (m8).
fn db_levels(pc: &ProfileCatalog, snap: &CatalogSnapshot) -> BTreeMap<Name, (Level, usize)> {
    let mut out: BTreeMap<Name, (Level, usize)> = BTreeMap::new();
    if hacivat_meta::kind::database_level(snap.server.dbms) != hacivat_meta::kind::DbLevel::Database
    {
        return out;
    }
    let mut counts: BTreeMap<Name, usize> = BTreeMap::new();
    for (key, ns) in snap.namespaces.iter() {
        if key.kind == NsKind::Schema && key.path.len() == 2 {
            *counts.entry(key.path[0].clone()).or_default() += level_pill::major_objects(ns);
        }
    }
    for (db, count) in counts {
        let choice = pc.handle.level_of(&NsKey::database(db.clone()));
        if choice.source == hacivat_introspect::engine::LevelSource::Auto {
            out.insert(db, (choice.level, count));
        }
    }
    out
}

/// Shows what an engine event means to the user (codes only from the engine).
fn notice(id: ProfileId, e: EngineEvent, cx: &mut App) {
    match e {
        EngineEvent::NamespaceDropped { ns } => toast(
            cx,
            ToastKind::Info,
            "hv-metadata-ns-dropped",
            vec![("schema", ArgValue::Text(ns.root_name().to_string()))],
        ),
        EngineEvent::LevelLowered { ns, level } => toast(
            cx,
            ToastKind::Info,
            "intro-hint-level-lowered",
            vec![
                ("name", ArgValue::Text(ns.root_name().to_string())),
                (
                    "level",
                    ArgValue::Text(level_pill::level_code(level).to_string()),
                ),
            ],
        ),
        EngineEvent::CacheWriteFailed => {
            toast(cx, ToastKind::Error, "model-cache-write-failed", Vec::new());
        }
        EngineEvent::CatalogRaceRetried { ns } => {
            tracing::debug!(schema = %ns.root_name(), "explorer: catalog race retried");
        }
        EngineEvent::DatabaseUnlisted { db } => {
            tracing::debug!(database = %db, ?id, "explorer: database not listed");
        }
        EngineEvent::PendingWithdrawn { ns } => {
            super::mass_drop::withdrawn(id, &ns, cx);
        }
        EngineEvent::ScopeNarrowed { .. } => {}
        EngineEvent::PublishFailed => tracing::debug!("explorer: an engine publish failed"),
    }
}

// ---- expansion ----------------------------------------------------------------------------

fn tree_of(
    cx: &App,
) -> Option<hacivat_ui_core::gpui_surface::Entity<hacivat_ui::tree::TreeView<super::ExplorerModel>>>
{
    crate::connections::explorer::tree(cx)
}

/// Removes the expansion of objects that left `id`'s catalog.
fn forget_expansion(id: ProfileId, gone: &[ObjectKey], cx: &mut App) {
    if gone.is_empty() {
        return;
    }
    let Some(tree) = tree_of(cx) else {
        return;
    };
    let keys: HashSet<TreeKey> = gone
        .iter()
        .map(|k| adapter::object_tree_key(id, k))
        .collect();
    tree.update(cx, |t, cx| {
        if t.expanded().iter().any(|k| keys.contains(k)) {
            let next: Vec<TreeKey> = t
                .expanded()
                .iter()
                .copied()
                .filter(|k| !keys.contains(k))
                .collect();
            t.set_expanded_keys(next, cx);
        }
    });
}

/// Applies `id`'s restored expansion once its catalog shows: only keys present in it, walked
/// from the profile's root through restored keys (PO-Q15). Their `Expand` events load nothing.
fn apply_restored(id: ProfileId, cx: &mut App) {
    let Some(keys) = cx
        .try_global::<CatalogHost>()
        .and_then(|h| h.restored.get(&id).cloned())
    else {
        return;
    };
    let Some(tree) = tree_of(cx) else {
        return;
    };
    let want: HashSet<TreeKey> = keys.iter().copied().collect();
    let model = tree.read(cx).model().clone();
    let root = adapter::profile_key(id);
    if !want.contains(&root) || model.info(root).is_none() {
        return;
    }
    let mut present = vec![root];
    let mut i = 0;
    while i < present.len() {
        if let ChildState::Loaded(kids) = model.children(present[i]) {
            for k in kids.iter() {
                if want.contains(k) {
                    present.push(*k);
                }
            }
        }
        i += 1;
    }
    {
        let host = cx.default_global::<CatalogHost>();
        host.restored.remove(&id);
        // Fix round 1 (m-2): only the rows the tree announces (expanded with children not
        // loaded) raise an `Expand` to swallow; any other key would eat a later user expansion.
        host.restoring.extend(
            present
                .iter()
                .copied()
                .filter(|k| matches!(model.children(*k), ChildState::NotLoaded)),
        );
    }
    tree.update(cx, |t, cx| {
        let mut all: Vec<TreeKey> = t.expanded().iter().copied().collect();
        all.extend(present.iter().copied());
        t.set_expanded_keys(all, cx);
    });
}

/// The tree's expanded keys changed: keeps them per profile for `workspace.json`.
pub fn note_expanded(expanded: &KeySet, model: &super::ExplorerModel, cx: &mut App) {
    let mut next = cx
        .try_global::<ExpandedState>()
        .cloned()
        .unwrap_or_default();
    let mut seen: HashMap<ProfileId, Vec<TreeKey>> = HashMap::new();
    let mut unknown = false;
    for k in expanded.iter() {
        match model.profile_of(*k) {
            Some(p) => seen.entry(p).or_default().push(*k),
            None => unknown = true,
        }
    }
    // Profiles whose rows the model shows are replaced; the others keep what was restored.
    let shown: Vec<ProfileId> = model
        .roots()
        .iter()
        .filter_map(|k| model.profile_root(*k))
        .collect();
    let restored_left: HashSet<ProfileId> = cx
        .try_global::<CatalogHost>()
        .map(|h| h.restored.keys().copied().collect())
        .unwrap_or_default();
    for p in shown {
        if restored_left.contains(&p) {
            continue;
        }
        let mut keys = seen.remove(&p).unwrap_or_default();
        keys.sort_by_key(|k| k.0);
        if keys.is_empty() {
            next.by_profile.remove(&p);
        } else {
            next.by_profile.insert(p, keys);
        }
    }
    let _ = unknown;
    if cx.try_global::<ExpandedState>() != Some(&next) {
        cx.set_global(next);
    }
    // A connected profile whose root opened: its catalog is read now (D-1).
    let waiting: Vec<ProfileId> = cx
        .try_global::<CatalogHost>()
        .map(|h| {
            h.entries
                .iter()
                .filter(|(id, e)| {
                    e.want_connect
                        && !e.connected
                        && matches!(e.open, Open::Ready(_))
                        && expanded.contains(&adapter::profile_key(**id))
                })
                .map(|(id, _)| *id)
                .collect()
        })
        .unwrap_or_default();
    for id in waiting {
        // After this update: the tree that called us is still being updated.
        cx.defer(move |cx| connect_engine(id, cx));
    }
}

/// The tree asked to load `key`'s children: a namespace (its first expansion), an object's
/// children (bulk policy), or an encrypted profile's root (the only keyring call before a
/// connect, PO-Q19). Restored rows load nothing (PO-Q15).
pub fn on_expand(key: TreeKey, cx: &mut App) {
    if cx
        .try_global::<CatalogHost>()
        .is_some_and(|h| h.restoring.contains(&key))
    {
        cx.default_global::<CatalogHost>().restoring.remove(&key);
        return;
    }
    let Some(tree) = tree_of(cx) else {
        return;
    };
    let model = tree.read(cx).model().clone();
    match model.info(key) {
        Some(NodeInfo::Profile(id)) => {
            let closed = cx.try_global::<CatalogHost>().is_none_or(|h| {
                h.entries
                    .get(&id)
                    .is_none_or(|e| matches!(e.open, Open::Closed))
            });
            let locked = cx
                .try_global::<CatalogHost>()
                .is_some_and(|h| h.locked.contains(&id));
            if closed && locked {
                open(id, cx);
            }
        }
        Some(NodeInfo::Schema { profile, ns }) => {
            if let Some(h) = handle(profile, cx)
                && is_connected(profile, cx)
            {
                drop(h.expand(ns));
            }
        }
        Some(NodeInfo::Object {
            profile,
            key: object,
        }) => {
            if matches!(
                model.children(key),
                ChildState::NotLoaded | ChildState::Loading { .. }
            ) && let Some(h) = handle(profile, cx)
                && is_connected(profile, cx)
            {
                drop(h.load_children(object));
            }
        }
        _ => {}
    }
}

// ---- refresh ------------------------------------------------------------------------------

/// The namespace `key` of `profile` lives in.
fn ns_of_object(profile: ProfileId, key: &ObjectKey, cx: &App) -> Option<NsKey> {
    let snap = adapter::catalog_of(profile, cx)?;
    snap.find(key).map(|h| h.ns.key().clone())
}

/// Refresh on tree row `key` (all `Trigger::User`): an object re-reads that object (a
/// fragment); a schema or a kind group its namespace (incremental); a database the scope
/// listing and its loaded schemas; a profile the scope listing and every loaded namespace.
pub fn refresh_key(key: TreeKey, cx: &mut App) {
    let Some(tree) = tree_of(cx) else {
        return;
    };
    let (model, owner) = {
        let t = tree.read(cx);
        let flat = t.flat();
        let owner = flat
            .index_of(key)
            .and_then(|ix| flat.parent_of(ix))
            .and_then(|ix| flat.get(ix))
            .map(|r| r.key);
        (t.model().clone(), owner)
    };
    let info = match model.info(key) {
        // A kind group refreshes what owns it.
        Some(NodeInfo::Group { .. }) => owner.and_then(|o| model.info(o)),
        other => other,
    };
    let Some(info) = info else {
        return;
    };
    let profile = match &info {
        NodeInfo::Profile(p)
        | NodeInfo::Database { profile: p, .. }
        | NodeInfo::Schema { profile: p, .. }
        | NodeInfo::BuiltIn { profile: p, .. }
        | NodeInfo::Group { profile: p, .. }
        | NodeInfo::Object { profile: p, .. }
        | NodeInfo::Contributed { profile: p } => *p,
        NodeInfo::Environment(_) => return,
    };
    let Some(h) = handle(profile, cx) else {
        return;
    };
    let snap = adapter::catalog_of(profile, cx);
    let loaded_ns = |db: Option<&Name>| -> Vec<NsKey> {
        snap.as_ref()
            .map(|s| {
                s.namespaces
                    .keys()
                    .filter(|k| db.is_none_or(|d| k.path.first() == Some(d)))
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    };
    let mut asked: Vec<NsKey> = Vec::new();
    match info {
        NodeInfo::Object { key: object, .. } => {
            if let Some(ns) = ns_of_object(profile, &object, cx) {
                drop(h.refresh(
                    Target::Objects {
                        ns,
                        keys: vec![object],
                    },
                    Trigger::User,
                ));
            }
        }
        NodeInfo::Schema { ns, .. } => {
            asked.push(ns);
        }
        NodeInfo::Database { name, .. } => {
            drop(h.refresh(Target::ScopeRefresh, Trigger::User));
            asked.extend(loaded_ns(Some(&name)));
        }
        NodeInfo::Profile(_) => {
            drop(h.refresh(Target::ScopeRefresh, Trigger::User));
            asked.extend(loaded_ns(None));
        }
        NodeInfo::BuiltIn { .. }
        | NodeInfo::Group { .. }
        | NodeInfo::Contributed { .. }
        | NodeInfo::Environment(_) => {}
    }
    if asked.is_empty() {
        return;
    }
    {
        let host = cx.default_global::<CatalogHost>();
        let entry = host.entry(profile);
        let mut row = (*entry.row).clone();
        for ns in &asked {
            // A held background deletion is asked about now (the refresh re-runs the guard).
            row.pending.remove(ns);
            entry.user_asked.insert(ns.clone());
        }
        entry.row = Arc::new(row);
        host.generation += 1;
    }
    for ns in asked {
        let ticket = h.refresh(Target::Namespace(ns.clone()), Trigger::User);
        let catalog = h.catalog().clone();
        cx.spawn(async move |cx| {
            let answered = ticket.await;
            // The guard publishes the namespace stale before it answers: a pass that is not
            // stale held nothing, so the user's question about it ends here (a later background
            // guard only marks the namespace, R-M4-22). A held one keeps it until its
            // `PendingConfirmation` is drained. A failed refresh held nothing either (m-3).
            let stale = answered.is_ok()
                && matches!(
                    catalog
                        .current()
                        .states
                        .get(&ContainerKey::Namespace(ns.clone())),
                    Some(LoadState::Loaded { stale: true, .. })
                );
            if !stale {
                cx.update(|cx| {
                    if let Some(e) = cx
                        .try_global::<CatalogHost>()
                        .and_then(|h| h.entries.get(&profile))
                        && e.user_asked.contains(&ns)
                    {
                        cx.default_global::<CatalogHost>()
                            .entry(profile)
                            .user_asked
                            .remove(&ns);
                    }
                });
            }
        })
        .detach();
    }
}

/// "Add to scope" on an out-of-scope database or schema row: the profile's scope pattern gains
/// a clause for it (`db:*` for a database, the schema's own clause otherwise) and the engine
/// gets the new scope.
pub fn add_to_scope(key: TreeKey, cx: &mut App) {
    let Some(tree) = tree_of(cx) else {
        return;
    };
    let model = tree.read(cx).model().clone();
    let (profile, clause) = match model.info(key) {
        Some(NodeInfo::Database { profile, name }) => (profile, format!("{}:*", glob_name(&name))),
        Some(NodeInfo::Schema { profile, ns }) => {
            let clause = match ns.path.as_slice() {
                [db, schema] => format!("{}:{}", glob_name(db), glob_name(schema)),
                [schema] => glob_name(schema),
                _ => return,
            };
            (profile, clause)
        }
        _ => return,
    };
    let Some(p) = profile_data_scope(profile, cx) else {
        return;
    };
    let dialect = profile_dialect(profile, cx);
    let base = p.unwrap_or_else(|| default_scope(dialect).to_string());
    let next = format!("{base}|{clause}");
    if parse_scope(&next, dialect).is_err() {
        return;
    }
    drop(userdata::set_scope(profile, Some(next), cx));
}

fn profile_dialect(id: ProfileId, cx: &App) -> DialectId {
    profile(id, cx).map_or(DialectId::Postgres, |p| dialect_of(p.engine))
}

/// The saved scope of `id` (`Some(None)`: the default), once its data file was read.
fn profile_data_scope(id: ProfileId, cx: &App) -> Option<Option<String>> {
    userdata::get(id, cx).map(|d| d.scope.clone())
}

/// A name as a literal scope-pattern term: a name of word characters as is, anything else as an
/// anchored, escaped `{…}` regular expression.
fn glob_name(name: &str) -> String {
    if !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return name.to_string();
    }
    let mut out = String::from("{");
    for c in name.chars() {
        if !c.is_alphanumeric() && c != '_' && c != ' ' {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('}');
    out
}

// ---- the offline strip ------------------------------------------------------------------------

/// When `profile`'s cached snapshot was read: the newest `Loaded.at` of its namespaces.
pub fn cached_at(profile: ProfileId, cx: &App) -> Option<std::time::SystemTime> {
    let snap = adapter::catalog_of(profile, cx).filter(|c| c.generation > 0)?;
    snap.states
        .values()
        .filter_map(|s| match s {
            LoadState::Loaded { at, .. } => Some(*at),
            _ => None,
        })
        .max()
}

/// The `DATETIME` argument of `at`.
pub fn when(at: std::time::SystemTime) -> ArgValue {
    let ms = match at.duration_since(std::time::UNIX_EPOCH) {
        Ok(d) => i64::try_from(d.as_millis()).unwrap_or(i64::MAX),
        Err(e) => -i64::try_from(e.duration().as_millis()).unwrap_or(i64::MAX),
    };
    ArgValue::When(WhenValue::Instant { unix_ms: ms })
}

/// "Showing cached metadata from {date}" for the selected profile, while it is not connected
/// and shows a cached tree (mock 3).
pub fn offline_note(cx: &App) -> Option<SharedString> {
    let m = cx.try_global::<ConnectionsModel>()?;
    let id = m.selected()?;
    if matches!(
        m.live(id),
        LiveState::Connected(_) | LiveState::Connecting(_) | LiveState::Reconnecting { .. }
    ) {
        return None;
    }
    let at = cached_at(id, cx)?;
    Some(text(cx, "hv-explorer-cached", vec![("date", when(at))]))
}

// ---- reveal and database rows -----------------------------------------------------------------

/// The databases `profile`'s explorer lists, in display order: the profile's database first,
/// then the others by name (R-M4-19).
pub fn database_rows(profile: ProfileId, cx: &App) -> Vec<DatabaseRow> {
    let Some(snap) = adapter::catalog_of(profile, cx).filter(|c| c.generation > 0) else {
        return Vec::new();
    };
    let Some(p) = self::profile(profile, cx) else {
        return Vec::new();
    };
    let connected = is_connected(profile, cx);
    let mut out: BTreeMap<Name, bool> = BTreeMap::new();
    for (key, state) in snap.states.iter() {
        if let ContainerKey::Namespace(ns) = key
            && let Some(db) = ns.path.first()
            && matches!(
                hacivat_meta::kind::database_level(p.engine),
                hacivat_meta::kind::DbLevel::Database
            )
        {
            let in_scope = !matches!(state, LoadState::OutOfScope);
            let e = out.entry(db.clone()).or_insert(false);
            *e |= in_scope;
        }
    }
    for key in snap.namespaces.keys() {
        if let Some(db) = key.path.first()
            && matches!(
                hacivat_meta::kind::database_level(p.engine),
                hacivat_meta::kind::DbLevel::Database
            )
        {
            out.insert(db.clone(), true);
        }
    }
    // Fix round 1 (m-8): the database the user's session is on, else the profile's.
    let own = session_database(profile, cx).or_else(|| p.database.clone());
    let mut rows: Vec<DatabaseRow> = out
        .into_iter()
        .map(|(name, in_scope)| DatabaseRow {
            connected: connected && own.as_deref() == Some(&*name),
            name: name.to_string(),
            in_scope,
        })
        .collect();
    rows.sort_by_key(|r| own.as_deref() != Some(r.name.as_str()));
    rows
}

/// Expands the ancestors of `key` of `profile` (loading them lazily), selects its row and
/// scrolls it into view (M5 "reveal in explorer", M4b).
pub fn reveal(
    profile: ProfileId,
    key: &ObjectKey,
    _window: &mut Window,
    cx: &mut App,
) -> RevealOutcome {
    let Some(tree) = tree_of(cx) else {
        return RevealOutcome::Unavailable;
    };
    let Some(snap) = adapter::catalog_of(profile, cx).filter(|c| c.generation > 0) else {
        return RevealOutcome::Unavailable;
    };
    let Some(found) = snap.find(key) else {
        // Not in a loaded namespace: missing only when its namespace is loaded.
        let ns_loaded = snap.namespaces.keys().any(|ns| {
            key.path.len() > ns.path.len()
                && key.path.iter().zip(ns.path.iter()).all(|(a, b)| a == b)
        });
        return if ns_loaded {
            RevealOutcome::Missing
        } else {
            RevealOutcome::Pending
        };
    };
    let ns = found.ns.key().clone();
    let target = adapter::object_tree_key(profile, key);
    // Walk down from the profile's root, expanding the row on the way to the target.
    let mut chain: Vec<TreeKey> = vec![adapter::profile_key(profile)];
    let mut at = chain[0];
    for _ in 0..16 {
        let model = tree.read(cx).model().clone();
        let ChildState::Loaded(kids) = model.children(at) else {
            break;
        };
        if kids.contains(&target) {
            break;
        }
        let next = kids
            .iter()
            .copied()
            .find(|k| leads_to(&model, *k, &ns, key));
        let Some(next) = next else {
            break;
        };
        chain.push(next);
        at = next;
    }
    let model = tree.read(cx).model().clone();
    let reached = matches!(model.children(at), ChildState::Loaded(ref k) if k.contains(&target));
    tree.update(cx, |t, cx| {
        let mut all: Vec<TreeKey> = t.expanded().iter().copied().collect();
        all.extend(chain.iter().copied());
        t.set_expanded_keys(all, cx);
        if reached {
            t.select(target, cx);
            t.scroll_to(target, cx);
        }
    });
    if reached {
        RevealOutcome::Revealed
    } else {
        RevealOutcome::Pending
    }
}

/// Whether row `k` is on the way down to `key` (in namespace `ns`).
fn leads_to(model: &super::ExplorerModel, k: TreeKey, ns: &NsKey, key: &ObjectKey) -> bool {
    match model.info(k) {
        Some(NodeInfo::Database { name, .. }) => ns.path.first() == Some(&name),
        Some(NodeInfo::Schema { ns: s, .. }) => &s == ns,
        Some(NodeInfo::Group { group, .. }) => {
            super::rows::DisplayGroup::of(key.kind) == Some(group)
                || key.path.len() > ns.path.len() + 1 && ancestor_group(model, k, key)
        }
        Some(NodeInfo::Object { key: o, .. }) => {
            o.path.len() < key.path.len() && key.path.iter().zip(o.path.iter()).all(|(a, b)| a == b)
        }
        _ => false,
    }
}

/// A group that holds an ancestor of `key` (a column's table under "Tables").
fn ancestor_group(model: &super::ExplorerModel, group: TreeKey, key: &ObjectKey) -> bool {
    let ChildState::Loaded(kids) = model.children(group) else {
        return false;
    };
    kids.iter().any(|k| match model.info(*k) {
        Some(NodeInfo::Object { key: o, .. }) => {
            o.path.len() < key.path.len() && key.path.iter().zip(o.path.iter()).all(|(a, b)| a == b)
        }
        _ => false,
    })
}

/// The progress of one namespace, when the engine reads it now.
pub fn progress_of(row: &LiveRow, ns: &NsKey) -> Option<(Phase, u64, Option<u64>)> {
    row.progress
        .tasks
        .iter()
        .find(|t| &t.ns == ns)
        .map(|t| (t.phase, t.done, t.total))
}
