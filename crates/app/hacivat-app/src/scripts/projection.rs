//! The Scripts node in the explorer (HV-SCRIPT-05, R-M4b-19, R-M4b-25, R-M4b-26; NOTES surface
//! 1): an [`ExplorerContributor`] that puts a "Scripts" node under every database node a bound
//! library shows under, above its schemas, with the library's folders and files below it.
//!
//! ## m4b-t01b-seams: the stub contributed no node and answered generation 0.
//!
//! ## m4b-t13-explorer
//! - **One source of truth.** Every row, count, badge and diagnostic comes from the library
//!   service's snapshot as the connection's environment sees it, `WorkspaceSnapshot::view(env)`
//!   (T10, R-T10-2), with `env` read from the profile **at answer time**: changing a profile's
//!   environment re-filters at once, without a rescan. Nothing here reads a file.
//! - **Placement** (R-M4b-25, R-M4b-31, R-M4b-32): a PostgreSQL database only when it is the one
//!   the profile's session is on or, without one, the profile's database; every MySQL/MariaDB
//!   database M4 asks about; SQLite only `main`, bound by the main file's stem. M4 asks only for
//!   profiles whose catalog has loaded, so a profile never expanded shows no Scripts node.
//! - **Keys** (R-M4b-26): `xxh3_128("hv-scripts\0" ‖ profile ‖ database ‖ library ‖ kind ‖ rel)`,
//!   each part length-prefixed. The same file under orders-prod and orders-staging has two keys,
//!   so expansion, selection and activation never cross connections; the contributor maps every
//!   key it listed back to its [`Place`] for `children`, `child_node`, `activate` and
//!   `context_menu`, and forgets the map when its generation moves.
//! - **Generation** (C-M4-7): the snapshot's generation and the profiles' environment
//!   generation. The service's events (one subscription, `Changed`/`GitChanged` carry no path)
//!   are drained in one foreground task, which calls `contributors_changed` once per drained
//!   batch.
//! - **The service** opens on the core runtime when `libraries.toml` exists (someone attached a
//!   library once; Task 16's attach opens it with [`ensure_service`]); with no library nothing
//!   starts, so every M4 tree and shot is unchanged.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;

use hacivat_git::FileStatus;
use hacivat_l10n::ArgValue;
use hacivat_meta::kind::{DbLevel, database_level};
use hacivat_proto::{EnvTag, ProfileId};
use hacivat_scripts::binding::{self, DbNode};
use hacivat_scripts::pathsafe::display_name;
use hacivat_scripts::{
    EntryKind, LibraryId, LibraryService, LibrarySnapshot, LibraryState, LibraryView, LinkState,
    LogicalDb, ScriptEntry, UnavailableReason, WorkspaceSnapshot,
};
use hacivat_session::profile::{Profile, ProfileEndpoint};
use hacivat_ui::icon::IconName;
use hacivat_ui::menu::MenuItemSpec;
use hacivat_ui::text::TextSource;
use hacivat_ui::tree::{ChildState, MetaMark, RowFlag, TreeKey, TreeRowData};
use hacivat_ui_core::gpui_surface::{App, Global, SharedString, Window};
use twox_hash::XxHash3_128;

use crate::AppState;
use crate::connections::ConnectionsModel;
use crate::explorer::contrib::{
    ContributedChildren, ContributedNode, ExplorerContributor, contributors_changed,
};
use crate::explorer::rows::NameOrder;
use crate::l10n_global::{tr_id, ui_language};

use super::git_badges;

/// The domain of every Scripts key (never equal to an object key, `"hv-object\0"`).
const DOMAIN: &[u8] = b"hv-scripts\0";

// ---- the service ---------------------------------------------------------------------------

/// The app's library service and the contributor's state (one per app).
pub(crate) struct ScriptsHost(Rc<Inner>);

impl Global for ScriptsHost {}

/// The contributor's state, shared by the registered contributor and the host.
#[derive(Default)]
pub(crate) struct Inner {
    service: RefCell<Option<Arc<LibraryService>>>,
    /// Moves whenever a profile's environment (or the set of profiles) changes.
    env_gen: Cell<u64>,
    /// The `(profile, environment)` pairs `env_gen` was counted for.
    env_seen: RefCell<Vec<(ProfileId, Option<EnvTag>)>>,
    /// The generation the maps below belong to.
    seen: Cell<Option<u64>>,
    places: RefCell<HashMap<TreeKey, Place>>,
    /// Each library's listing as one environment sees it, built once per generation.
    index: RefCell<HashMap<IndexKey, Rc<LibIndex>>>,
    /// The libraries, as `binding::resolve` takes them, once per generation.
    libs: RefCell<Option<Rc<Vec<LibrarySnapshot>>>>,
    /// The display order of the UI language, kept while the language stays.
    order: RefCell<Option<Rc<NameOrder>>>,
    /// The foreground task that turns the service's events into redraws.
    relay: RefCell<Option<hacivat_ui_core::gpui_surface::Task<()>>>,
    /// Whether the profiles' environments are being followed.
    following: Cell<bool>,
    // Tests: what activation asked `open_script` for.
    #[cfg(feature = "test-support")]
    opens: RefCell<Vec<OpenAsked>>,
}

/// A library as one environment sees it.
type IndexKey = (LibraryId, Option<EnvTag>);

/// What a file row's activation asked `open_script` for (tests).
#[cfg(feature = "test-support")]
pub type OpenAsked = (LibraryId, String, Option<ProfileId>, Option<String>);

fn host(cx: &App) -> Option<Rc<Inner>> {
    cx.try_global::<ScriptsHost>().map(|h| h.0.clone())
}

fn host_or_new(cx: &mut App) -> Rc<Inner> {
    if let Some(inner) = host(cx) {
        return inner;
    }
    let inner = Rc::new(Inner::default());
    cx.set_global(ScriptsHost(inner.clone()));
    inner
}

/// The library service, when one is open.
pub fn service(cx: &App) -> Option<Arc<LibraryService>> {
    host(cx)?.service.borrow().clone()
}

/// Opens the library service on the core runtime when a library was ever attached (its
/// registry exists); nothing otherwise (`connections::boot`, after the model is set).
///
/// ## m4b-t13-explorer fix round 1 (review m9): the registry is looked for on the core
/// runtime (never a stat on the UI thread), and without one the recovery folders of libraries
/// no registry names are swept there (T10's launch hygiene, which the service would otherwise
/// do; copies of a live instance are spared).
pub fn install_service(cx: &mut App) {
    let Some(paths) = cx.try_global::<AppState>().map(|s| s.paths.clone()) else {
        return;
    };
    let Some(rt) = cx.try_global::<ConnectionsModel>().map(|m| m.runtime()) else {
        return;
    };
    let (tx, rx) = tokio::sync::oneshot::channel();
    rt.spawn_blocking(move || {
        let plan = boot_plan(paths.libraries_toml().try_exists());
        if plan == BootPlan::SweepOrphans {
            hacivat_scripts::recovery::sweep_unregistered(&paths, &|_| false);
        }
        let _ = tx.send(plan);
    });
    cx.spawn(async move |cx| {
        if rx.await == Ok(BootPlan::OpenService) {
            cx.update(|cx| {
                ensure_service(cx);
            });
        }
    })
    .detach();
}

// ## m4b-t13-explorer (re-review n6, safety)
/// What the boot does about the library registry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BootPlan {
    /// The registry is there, or its presence cannot be told: the service opens and reads it
    /// (it sweeps nothing over a registry it could not read cleanly, T10).
    OpenService,
    /// No registry, proven: no library was ever attached, so every recovery folder is an
    /// orphan and is swept (copies of a live instance are spared).
    SweepOrphans,
}

/// Recovery copies are swept only when the registry is *known* to be absent: an I/O error
/// (an unmounted or unreadable config folder) never leads to a sweep.
fn boot_plan(registry: std::io::Result<bool>) -> BootPlan {
    match registry {
        Ok(false) => BootPlan::SweepOrphans,
        Ok(true) | Err(_) => BootPlan::OpenService,
    }
}

/// The library service, opened now if it is not open yet (Task 16's attach calls this before
/// its first attach). `None` without settings or a connections model.
pub fn ensure_service(cx: &mut App) -> Option<Arc<LibraryService>> {
    if let Some(s) = service(cx) {
        return Some(s);
    }
    let paths = cx.try_global::<AppState>()?.paths.clone();
    // The core runtime's handle (`CoreRuntime::handle`, carries-w1 T10).
    let rt = cx.try_global::<ConnectionsModel>()?.runtime();
    let svc = Arc::new(LibraryService::open(&paths, &rt));
    set_service(svc.clone(), cx);
    Some(svc)
}

/// Makes `service` the app's library service (tests pass one over fake event sources): the
/// Scripts nodes read its snapshot, and its events redraw the explorer.
pub fn set_service(service: Arc<LibraryService>, cx: &mut App) {
    let inner = host_or_new(cx);
    let mut events = service.subscribe(None);
    *inner.service.borrow_mut() = Some(service);
    inner.seen.set(None);
    follow_environments(&inner, cx);
    let task = cx.spawn(async move |cx| {
        use tokio::sync::broadcast::error::RecvError;
        // `Changed` and `GitChanged` reach every subscriber; a lagged one re-reads. The loop
        // ends when the service is dropped.
        while let Ok(_) | Err(RecvError::Lagged(_)) = events.recv().await {
            // One redraw per drained batch (at most one per frame).
            while events.try_recv().is_ok() {}
            cx.update(contributors_changed);
        }
    });
    *inner.relay.borrow_mut() = Some(task);
    contributors_changed(cx);
}

/// Follows the profiles' environments: a change moves the generation (the explorer then drops
/// and re-asks the loaded Scripts children, so a retagged profile is filtered again at once).
fn follow_environments(inner: &Rc<Inner>, cx: &mut App) {
    if inner.following.replace(true) {
        return;
    }
    // At quit the service is dropped first, so its actor stops before the core runtime does.
    let at_quit = Rc::downgrade(inner);
    cx.on_app_quit(move |_| {
        if let Some(inner) = at_quit.upgrade() {
            inner.relay.borrow_mut().take();
            drop(inner.service.borrow_mut().take());
        }
        async {}
    })
    .detach();
    note_environments(inner, cx);
    let weak = Rc::downgrade(inner);
    cx.observe_global::<ConnectionsModel>(move |cx| {
        let Some(inner) = weak.upgrade() else {
            return;
        };
        if note_environments(&inner, cx) {
            contributors_changed(cx);
        }
    })
    .detach();
    // ## m4b-t13-explorer fix round 1 (review m11): a catalog that lands or changes can add or
    // drop a place a library shows under, so the loaded rows' "shared" is asked again.
    let weak = Rc::downgrade(inner);
    let seen = Cell::new(crate::explorer::adapter::catalogs_generation(cx));
    cx.observe_global::<crate::explorer::adapter::ExplorerCatalogs>(move |cx| {
        let Some(inner) = weak.upgrade() else {
            return;
        };
        let now = crate::explorer::adapter::catalogs_generation(cx);
        if seen.replace(now) != now {
            inner.env_gen.set(inner.env_gen.get() + 1);
            contributors_changed(cx);
        }
    })
    .detach();
}

/// Records the profiles' environments; whether they changed.
fn note_environments(inner: &Inner, cx: &App) -> bool {
    let now: Vec<(ProfileId, Option<EnvTag>)> = cx
        .try_global::<ConnectionsModel>()
        .map(|m| m.profiles().iter().map(|p| (p.id, p.environment)).collect())
        .unwrap_or_default();
    if *inner.env_seen.borrow() == now {
        return false;
    }
    *inner.env_seen.borrow_mut() = now;
    inner.env_gen.set(inner.env_gen.get() + 1);
    true
}

// ---- places and keys -----------------------------------------------------------------------

/// Where a Scripts key belongs: the connection and database it is listed under, and what it is.
#[derive(Debug, Clone)]
pub(crate) struct Place {
    pub profile: ProfileId,
    pub database: String,
    pub node: Node,
}

/// What a Scripts key is.
#[derive(Debug, Clone)]
pub(crate) enum Node {
    /// The Scripts node, over the libraries bound here.
    Scripts { libraries: Vec<LibraryId> },
    /// One library's row (when several are bound here), with its display name.
    Library { id: LibraryId, label: String },
    /// A folder or a file of a library.
    Entry {
        library: LibraryId,
        entry: ScriptEntry,
        status: Option<FileStatus>,
        contains_changes: bool,
        // ## m4b-t13-explorer fix round 1 (review m11): the library shows elsewhere too.
        shared: bool,
    },
    /// A library's problems row (surface 11).
    Problems { library: LibraryId, count: usize },
}

impl Place {
    /// The library a key under a library belongs to.
    pub fn library(&self) -> Option<LibraryId> {
        match &self.node {
            Node::Scripts { libraries } if libraries.len() == 1 => libraries.first().copied(),
            Node::Scripts { .. } => None,
            Node::Library { id, .. } => Some(*id),
            Node::Entry { library, .. } | Node::Problems { library, .. } => Some(*library),
        }
    }

    /// The listed entry of a folder or file row.
    pub fn entry(&self) -> Option<&ScriptEntry> {
        match &self.node {
            Node::Entry { entry, .. } => Some(entry),
            _ => None,
        }
    }
}

struct KeyBytes(Vec<u8>);

impl KeyBytes {
    fn part(&mut self, bytes: &[u8]) {
        let len = u32::try_from(bytes.len()).unwrap_or(u32::MAX);
        self.0.extend_from_slice(&len.to_le_bytes());
        self.0.extend_from_slice(bytes);
    }
}

/// A Scripts key (R-M4b-26): profile, database, library, kind and path, each length-prefixed.
pub(crate) fn scripts_key(
    profile: ProfileId,
    database: &str,
    library: Option<LibraryId>,
    kind: u8,
    rel: &str,
) -> TreeKey {
    let mut k = KeyBytes(Vec::with_capacity(96 + rel.len()));
    k.part(DOMAIN);
    k.part(profile.0.as_bytes());
    k.part(database.as_bytes());
    match library {
        Some(id) => k.part(id.0.as_bytes()),
        None => k.part(b""),
    }
    k.part(&[kind]);
    k.part(rel.as_bytes());
    TreeKey(XxHash3_128::oneshot(&k.0))
}

const K_SCRIPTS: u8 = b'S';
const K_LIBRARY: u8 = b'L';
const K_FOLDER: u8 = b'D';
const K_FILE: u8 = b'F';
const K_PROBLEMS: u8 = b'P';

fn entry_key(profile: ProfileId, database: &str, library: LibraryId, e: &ScriptEntry) -> TreeKey {
    let kind = if e.kind == EntryKind::Folder {
        K_FOLDER
    } else {
        K_FILE
    };
    scripts_key(profile, database, Some(library), kind, e.rel.as_str())
}

// ---- one library as an environment sees it ---------------------------------------------------

/// A library's shown listing, built once per generation and environment: the children of each
/// folder in display order, each file's badge and the folders that hold a change.
struct LibIndex {
    children: HashMap<String, Vec<ScriptEntry>>,
    status: HashMap<String, FileStatus>,
    contains: HashSet<String>,
    files: usize,
    problems: usize,
    state: LibraryState,
}

/// The folder that holds `rel` (`""` for the root).
fn parent(rel: &str) -> &str {
    rel.rfind('/').map_or("", |i| &rel[..i])
}

impl Inner {
    fn snapshot(&self) -> Option<Arc<WorkspaceSnapshot>> {
        self.service.borrow().as_ref().map(|s| s.snapshot())
    }

    fn generation(&self) -> u64 {
        let snap = self
            .service
            .borrow()
            .as_ref()
            .map_or(0, |s| s.snapshot().generation);
        let env = self.env_gen.get();
        if snap == 0 {
            return 0;
        }
        snap ^ env.rotate_left(40)
    }

    /// Forgets the maps of an older generation.
    fn sync(&self) {
        let now = self.generation();
        if self.seen.get() != Some(now) {
            self.seen.set(Some(now));
            self.places.borrow_mut().clear();
            self.index.borrow_mut().clear();
            *self.libs.borrow_mut() = None;
        }
    }

    fn order(&self, cx: &App) -> Rc<NameOrder> {
        let lang = ui_language(cx).map(|l| l.to_string()).unwrap_or_default();
        let mut slot = self.order.borrow_mut();
        match slot.as_ref() {
            Some(o) if o.lang() == lang || (lang.is_empty() && o.lang() == "und") => o.clone(),
            _ => {
                let o = Rc::new(NameOrder::new(&lang));
                *slot = Some(o.clone());
                o
            }
        }
    }

    fn libs(&self, snap: &WorkspaceSnapshot) -> Rc<Vec<LibrarySnapshot>> {
        let mut slot = self.libs.borrow_mut();
        if let Some(l) = slot.as_ref() {
            return l.clone();
        }
        let l: Rc<Vec<LibrarySnapshot>> =
            Rc::new(snap.libraries.iter().map(|l| (**l).clone()).collect());
        *slot = Some(l.clone());
        l
    }

    fn lib_index(
        &self,
        snap: &WorkspaceSnapshot,
        id: LibraryId,
        env: Option<EnvTag>,
        cx: &App,
    ) -> Option<Rc<LibIndex>> {
        if let Some(i) = self.index.borrow().get(&(id, env)) {
            return Some(i.clone());
        }
        let view = snap.view(env);
        let lib = view.library(id)?;
        let order = self.order(cx);
        let mut children: HashMap<String, Vec<ScriptEntry>> = HashMap::new();
        let mut files = 0;
        for e in lib.entries() {
            if e.kind != EntryKind::Folder {
                files += 1;
            }
            children
                .entry(parent(e.rel.as_str()).to_owned())
                .or_default()
                .push(e.clone());
        }
        for list in children.values_mut() {
            // Folders before files, each by the UI collation (NOTES surface 1).
            list.sort_by(|a, b| {
                (b.kind == EntryKind::Folder)
                    .cmp(&(a.kind == EntryKind::Folder))
                    .then_with(|| order.cmp(a.rel.name(), b.rel.name()))
            });
        }
        let mut status = HashMap::new();
        let mut contains = HashSet::new();
        for (rel, st) in lib.git_statuses() {
            if git_badges::is_change(st) {
                let mut p = parent(rel.as_str());
                while !p.is_empty() {
                    if !contains.insert(p.to_owned()) {
                        break;
                    }
                    p = parent(p);
                }
            }
            status.insert(rel.as_str().to_owned(), st);
        }
        let idx = Rc::new(LibIndex {
            children,
            status,
            contains,
            files,
            problems: super::diagnostics::count(&lib),
            state: lib.library().state.clone(),
        });
        self.index.borrow_mut().insert((id, env), idx.clone());
        Some(idx)
    }

    fn place(&self, key: TreeKey) -> Option<Place> {
        self.places.borrow().get(&key).cloned()
    }

    fn remember(&self, key: TreeKey, place: Place) {
        self.places.borrow_mut().insert(key, place);
    }
}

// ---- placement -------------------------------------------------------------------------------

fn profile(id: ProfileId, cx: &App) -> Option<Arc<Profile>> {
    cx.try_global::<ConnectionsModel>()?.get(id)
}

/// The libraries bound to `database` of `p` when a Scripts node belongs there (R-M4b-25,
/// R-M4b-32).
fn libraries_at(
    inner: &Inner,
    p: &Profile,
    database: &str,
    snap: &WorkspaceSnapshot,
    cx: &App,
) -> Vec<LibraryId> {
    let logical = match database_level(p.engine) {
        DbLevel::File => {
            if database != "main" {
                return Vec::new();
            }
            match &p.endpoint {
                ProfileEndpoint::File { path, .. } => binding::logical_for_sqlite(path),
                _ => None,
            }
        }
        DbLevel::Database => {
            // Only the database a script tab can run on: the session's, else the profile's.
            let current =
                crate::explorer::live::session_database(p.id, cx).or_else(|| p.database.clone());
            if current.as_deref() != Some(database) {
                return Vec::new();
            }
            LogicalDb::parse(database).ok()
        }
        DbLevel::SchemaIsDatabase => LogicalDb::parse(database).ok(),
    };
    let exact = hacivat_scripts::DatabaseName::new(database);
    let libs = inner.libs(snap);
    match logical {
        Some(logical) => binding::resolve(
            &DbNode {
                profile: p.id,
                database: exact,
                logical,
            },
            &libs,
            &snap.mappings,
        ),
        // No logical name (an in-memory SQLite database): a local mapping only.
        None => snap
            .mappings
            .iter()
            .find(|m| m.profile == p.id && m.database == exact)
            .filter(|m| libs.iter().any(|l| l.id == m.library))
            .map(|m| m.library)
            .into_iter()
            .collect(),
    }
}

/// The databases M4 asks contributors about for `p` (`connections/explorer.rs`
/// `contributed_places`, read from the same catalog): none before its catalog loaded.
fn asked_databases(p: &Profile, cx: &App) -> Vec<String> {
    let Some(cat) = crate::explorer::adapter::catalog_of(p.id, cx).filter(|c| c.generation > 0)
    else {
        return Vec::new();
    };
    let mut dbs: Vec<String> = Vec::new();
    match database_level(p.engine) {
        DbLevel::File => dbs.push("main".to_string()),
        level => {
            for k in cat
                .namespaces
                .keys()
                .chain(cat.states.keys().filter_map(|c| match c {
                    hacivat_meta::snapshot::ContainerKey::Namespace(k) => Some(k),
                    _ => None,
                }))
            {
                let name = match (level, k.kind) {
                    (DbLevel::Database, _) => k.path.first(),
                    (_, hacivat_meta::NsKind::Schema) => k.path.last(),
                    _ => None,
                };
                if let Some(n) = name
                    && !dbs.iter().any(|d| d == n.as_ref())
                {
                    dbs.push(n.to_string());
                }
            }
            if let Some(db) = &p.database
                && level == DbLevel::Database
                && !dbs.contains(db)
            {
                dbs.push(db.clone());
            }
        }
    }
    dbs
}

/// The other connections that show one of `libs` (the "shared" badge and its tooltip).
fn shown_elsewhere(
    inner: &Inner,
    libs: &[LibraryId],
    here: (ProfileId, &str),
    snap: &WorkspaceSnapshot,
    cx: &App,
) -> Vec<String> {
    let profiles: Vec<Arc<Profile>> = cx
        .try_global::<ConnectionsModel>()
        .map(|m| m.profiles().to_vec())
        .unwrap_or_default();
    let mut names: Vec<String> = Vec::new();
    for q in &profiles {
        for db in asked_databases(q, cx) {
            if (q.id, db.as_str()) == here {
                continue;
            }
            let there = libraries_at(inner, q, &db, snap, cx);
            if there.iter().any(|l| libs.contains(l)) {
                let name = q.name.clone();
                if !names.contains(&name) {
                    names.push(name);
                }
            }
        }
    }
    names
}

// ---- rows ------------------------------------------------------------------------------------

fn text(cx: &App, id: &str, args: Vec<(&'static str, ArgValue)>) -> SharedString {
    crate::explorer::live::text(cx, id, args)
}

fn count(n: usize) -> ArgValue {
    ArgValue::Count(u64::try_from(n).unwrap_or(u64::MAX))
}

/// The preformatted count of a row's meta (format locale, L3).
fn count_text(cx: &App, n: usize) -> SharedString {
    text(cx, "hv-explorer-count", vec![("n", count(n))])
}

/// The row tooltip's facts, in one line: the tree's tooltip is a single-line label (a newline
/// in it panics GPUI's text layout; found by the m7 hover shot, fix round 1).
fn join_lines(lines: &[SharedString]) -> Option<SharedString> {
    (!lines.is_empty()).then(|| {
        lines
            .iter()
            .map(|l| l.as_ref())
            .collect::<Vec<_>>()
            .join(" · ")
            .into()
    })
}

fn join_words(words: &[SharedString]) -> Option<SharedString> {
    (!words.is_empty()).then(|| {
        words
            .iter()
            .map(|w| w.as_ref())
            .collect::<Vec<_>>()
            .join(", ")
            .into()
    })
}

/// The environment an `@<tag>` folder name names (ASCII case-insensitive, R-M4b-6).
fn env_folder_tag(name: &str) -> Option<EnvTag> {
    let rest = name.strip_prefix('@')?;
    EnvTag::ALL
        .iter()
        .copied()
        .find(|t| t.id().eq_ignore_ascii_case(rest))
}

/// The Scripts node's row.
fn scripts_row(files: usize, others: &[String], cx: &App) -> TreeRowData {
    let mut row = TreeRowData::new(tr_id(cx, "hv-scripts-node"), TextSource::Fluent);
    row.icon = Some(IconName::ScrollText);
    row.meta = Some(count_text(cx, files));
    let mut tip = vec![text(
        cx,
        "hv-scripts-node-row-tip",
        vec![("n", count(files))],
    )];
    let mut words = vec![text(cx, "hv-scripts-set-files", vec![("n", count(files))])];
    if !others.is_empty() {
        // Outlined, lowercase, glyph-less (controller ruling b); never red, never filled.
        row.badge = Some(tr_id(cx, "hv-scripts-node-shared"));
        // ## m4b-t13-explorer fix round 1: the tree's tooltip is one line (`Tip`), so a shared
        // node's tooltip is the shared sentence alone (the count is the row's meta and its
        // accessible description); both sentences together ran past a 960 px window.
        tip = vec![text(
            cx,
            "hv-scripts-node-shared-tip",
            vec![("conns", ArgValue::List(others.to_vec()))],
        )];
        words.push(tr_id(cx, "hv-scripts-node-shared"));
    }
    row.tooltip = join_lines(&tip);
    row.a11y_description = join_words(&words);
    row
}

/// The label of each library row: its folder name, a duplicate numbered ` (2)` (NOTES surface
/// 1), escaped.
fn library_labels(snap: &WorkspaceSnapshot, ids: &[LibraryId]) -> Vec<String> {
    let mut seen: HashMap<String, usize> = HashMap::new();
    ids.iter()
        .map(|id| {
            let name = snap
                .library(*id)
                .and_then(|l| {
                    l.root
                        .path
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                })
                .unwrap_or_default();
            let name = display_name(&name);
            let n = seen.entry(name.clone()).or_insert(0);
            *n += 1;
            if *n == 1 {
                name
            } else {
                format!("{name} ({n})")
            }
        })
        .collect()
}

fn library_row(
    snap: &WorkspaceSnapshot,
    id: LibraryId,
    label: &str,
    files: usize,
    cx: &App,
) -> TreeRowData {
    let mut row = TreeRowData::new(SharedString::from(label.to_string()), TextSource::Data);
    row.icon = Some(IconName::Folder);
    row.meta = Some(count_text(cx, files));
    if let Some(lib) = snap.library(id) {
        let path = display_name(&lib.root.path.to_string_lossy());
        row.tooltip = Some(text(
            cx,
            "hv-scripts-lib-tip",
            vec![
                ("path", ArgValue::Text(path)),
                ("database", ArgValue::Text(lib.db.display_escaped())),
            ],
        ));
    }
    row.a11y_description =
        join_words(&[text(cx, "hv-scripts-set-files", vec![("n", count(files))])]);
    row
}

fn entry_row(
    entry: &ScriptEntry,
    status: Option<FileStatus>,
    contains_changes: bool,
    shared: bool,
    cx: &App,
) -> TreeRowData {
    let name = display_name(entry.rel.name());
    let mut row = TreeRowData::new(SharedString::from(name), TextSource::Data);
    let mut tip: Vec<SharedString> = Vec::new();
    let mut words: Vec<SharedString> = Vec::new();
    match entry.kind {
        EntryKind::Folder => {
            row.icon = Some(IconName::Folder);
            if let Some(tag) = env_folder_tag(entry.rel.name()) {
                row.flags
                    .push(RowFlag::Env(crate::connections::capsule::env_kind(Some(
                        tag,
                    ))));
                tip.push(text(
                    cx,
                    "hv-scripts-env-folder-tip",
                    vec![("env", ArgValue::Text(tag.id().to_string()))],
                ));
            }
            if contains_changes {
                let st = git_badges::contains_changes(cx);
                tip.push(git_badges::tip_line(&st));
                words.push(tr_id(cx, "hv-scripts-a11y-contains"));
                row.status = Some(st);
            }
            if entry.dataless {
                row.meta_mark = Some(MetaMark::Glyph(IconName::Download));
                tip.push(tr_id(cx, "hv-scripts-dataless-row-tip"));
                words.push(tr_id(cx, "hv-scripts-a11y-dataless"));
            }
        }
        EntryKind::Sql | EntryKind::Markdown => {
            row.icon = Some(if entry.kind == EntryKind::Sql {
                IconName::FileCode
            } else {
                IconName::FileText
            });
            if let Some(st) = status {
                let s = git_badges::file_status(st, cx);
                tip.push(git_badges::tip_line(&s));
                words.push(git_badges::a11y_word(st, cx));
                row.status = Some(s);
            }
            match entry.link {
                LinkState::OutsideLink | LinkState::Loop => {
                    row.icon = Some(IconName::Link);
                    row.dim = true;
                    tip.insert(0, tr_id(cx, "hv-scripts-outside-link-tip"));
                    words.push(tr_id(cx, "hv-scripts-a11y-outside"));
                }
                LinkState::Plain | LinkState::InsideLink => {}
            }
            if entry.dataless {
                // The meta mark only: no words in the meta slot (review I4).
                row.meta_mark = Some(MetaMark::Glyph(IconName::Download));
                tip.insert(0, tr_id(cx, "hv-scripts-dataless-row-tip"));
                words.push(tr_id(cx, "hv-scripts-a11y-dataless"));
            }
        }
    }
    // ## m4b-t13-explorer fix round 1 (review m11): "orders_report.sql, modified, shared".
    if shared && entry.kind != EntryKind::Folder {
        words.push(tr_id(cx, "hv-scripts-node-shared"));
    }
    row.tooltip = join_lines(&tip);
    row.a11y_description = join_words(&words);
    row
}

fn problems_row(count_n: usize, cx: &App) -> TreeRowData {
    let mut row = TreeRowData::new(
        text(cx, "hv-scripts-diag-row", vec![("n", count(count_n))]),
        TextSource::Fluent,
    );
    row.icon = Some(IconName::CircleAlert);
    row
}

/// What a library that shows nothing shows under its node.
fn unavailable(reason: UnavailableReason, cx: &App) -> ChildState {
    let label = match reason {
        UnavailableReason::Missing | UnavailableReason::NotImplemented => {
            tr_id(cx, "hv-scripts-missing-row")
        }
        UnavailableReason::NotResponding => text(
            cx,
            "hv-scripts-unavailable-row",
            vec![("reason", ArgValue::Text("responding".into()))],
        ),
        UnavailableReason::RootChanged => text(
            cx,
            "hv-scripts-unavailable-row",
            vec![("reason", ArgValue::Text("moved".into()))],
        ),
        UnavailableReason::Unreadable(_) | UnavailableReason::InEnvFolder => text(
            cx,
            "hv-scripts-unavailable-row",
            vec![("reason", ArgValue::Text("unreadable".into()))],
        ),
    };
    // The retry is the menu's Refresh (the tree's retry button is M4's catalog retry).
    ChildState::Failed { label, retry: None }
}

// ---- the contributor -------------------------------------------------------------------------

/// The Scripts contributor. The registered one shares the app's [`ScriptsHost`]; a default one
/// has no service and contributes nothing.
#[derive(Default)]
pub struct ScriptsContributor {
    inner: Rc<Inner>,
}

impl std::fmt::Debug for ScriptsContributor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ScriptsContributor")
            .field("generation", &self.inner.generation())
            .finish_non_exhaustive()
    }
}

impl ScriptsContributor {
    /// The contributor over the app's host (`scripts::install` registers it).
    pub(crate) fn for_app(cx: &mut App) -> Self {
        ScriptsContributor {
            inner: host_or_new(cx),
        }
    }
}

/// The lazy children of `key`, asked through the app's host (the closure holds the key only).
fn lazy(key: TreeKey) -> ContributedChildren {
    ContributedChildren::Lazy(Arc::new(move |cx: &App| match host(cx) {
        Some(inner) => children_of(&inner, key, cx),
        None => ChildState::Leaf,
    }))
}

fn children_of(inner: &Inner, key: TreeKey, cx: &App) -> ChildState {
    inner.sync();
    let (Some(snap), Some(place)) = (inner.snapshot(), inner.place(key)) else {
        return ChildState::Leaf;
    };
    let Some(p) = profile(place.profile, cx) else {
        return ChildState::Leaf;
    };
    let env = p.environment;
    match &place.node {
        Node::Scripts { libraries } if libraries.len() == 1 => {
            library_children(inner, &place, libraries[0], env, &snap, cx)
        }
        Node::Scripts { libraries } => {
            let labels = library_labels(&snap, libraries);
            let keys: Vec<TreeKey> = libraries
                .iter()
                .zip(labels)
                .map(|(id, label)| {
                    let k = scripts_key(place.profile, &place.database, Some(*id), K_LIBRARY, "");
                    inner.remember(
                        k,
                        Place {
                            profile: place.profile,
                            database: place.database.clone(),
                            node: Node::Library { id: *id, label },
                        },
                    );
                    k
                })
                .collect();
            ChildState::Loaded(Arc::from(keys))
        }
        Node::Library { id, .. } => library_children(inner, &place, *id, env, &snap, cx),
        Node::Entry { library, entry, .. } if entry.kind == EntryKind::Folder => {
            let Some(idx) = inner.lib_index(&snap, *library, env, cx) else {
                return ChildState::Leaf;
            };
            let shared = is_shared(inner, &place, *library, &snap, cx);
            ChildState::Loaded(Arc::from(entries_under(
                inner,
                &place,
                *library,
                &idx,
                entry.rel.as_str(),
                shared,
            )))
        }
        Node::Entry { .. } | Node::Problems { .. } => ChildState::Leaf,
    }
}

fn library_children(
    inner: &Inner,
    place: &Place,
    id: LibraryId,
    env: Option<EnvTag>,
    snap: &WorkspaceSnapshot,
    cx: &App,
) -> ChildState {
    let Some(idx) = inner.lib_index(snap, id, env, cx) else {
        return unavailable(UnavailableReason::Missing, cx);
    };
    match &idx.state {
        LibraryState::Unavailable(r) => return unavailable(*r, cx),
        LibraryState::Scanning if idx.children.is_empty() => {
            return ChildState::Loading {
                label: tr_id(cx, "hv-scripts-loading"),
                running: true,
                count: None,
            };
        }
        _ => {}
    }
    let mut keys = Vec::new();
    // The problems row is the library's first child (NOTES ruling 7).
    if idx.problems > 0 {
        let k = scripts_key(place.profile, &place.database, Some(id), K_PROBLEMS, "");
        inner.remember(
            k,
            Place {
                profile: place.profile,
                database: place.database.clone(),
                node: Node::Problems {
                    library: id,
                    count: idx.problems,
                },
            },
        );
        keys.push(k);
    }
    let shared = is_shared(inner, place, id, snap, cx);
    keys.extend(entries_under(inner, place, id, &idx, "", shared));
    ChildState::Loaded(Arc::from(keys))
}

// ## m4b-t13-explorer fix round 1 (review m11)
/// Whether `library` also shows under a connection other than `place`'s ("shared").
fn is_shared(
    inner: &Inner,
    place: &Place,
    library: LibraryId,
    snap: &WorkspaceSnapshot,
    cx: &App,
) -> bool {
    !shown_elsewhere(
        inner,
        &[library],
        (place.profile, &place.database),
        snap,
        cx,
    )
    .is_empty()
}

fn entries_under(
    inner: &Inner,
    place: &Place,
    library: LibraryId,
    idx: &LibIndex,
    folder: &str,
    shared: bool,
) -> Vec<TreeKey> {
    let Some(list) = idx.children.get(folder) else {
        return Vec::new();
    };
    list.iter()
        .map(|e| {
            let k = entry_key(place.profile, &place.database, library, e);
            inner.remember(
                k,
                Place {
                    profile: place.profile,
                    database: place.database.clone(),
                    node: Node::Entry {
                        library,
                        entry: e.clone(),
                        status: idx.status.get(e.rel.as_str()).copied(),
                        contains_changes: idx.contains.contains(e.rel.as_str()),
                        shared,
                    },
                },
            );
            k
        })
        .collect()
}

impl ExplorerContributor for ScriptsContributor {
    fn database_children(
        &self,
        profile_id: ProfileId,
        database: &str,
        cx: &App,
    ) -> Vec<ContributedNode> {
        let inner = &self.inner;
        inner.sync();
        let Some(snap) = inner.snapshot() else {
            return Vec::new();
        };
        let Some(p) = profile(profile_id, cx) else {
            return Vec::new();
        };
        let libraries = libraries_at(inner, &p, database, &snap, cx);
        if libraries.is_empty() {
            return Vec::new();
        }
        let files: usize = libraries
            .iter()
            .filter_map(|id| inner.lib_index(&snap, *id, p.environment, cx))
            .map(|i| i.files)
            .sum();
        let others = shown_elsewhere(inner, &libraries, (profile_id, database), &snap, cx);
        let key = scripts_key(profile_id, database, None, K_SCRIPTS, "");
        inner.remember(
            key,
            Place {
                profile: profile_id,
                database: database.to_string(),
                node: Node::Scripts { libraries },
            },
        );
        vec![ContributedNode {
            key,
            row: scripts_row(files, &others, cx),
            children: lazy(key),
            filter_text: None,
        }]
    }

    fn children(&self, key: TreeKey, cx: &App) -> ChildState {
        children_of(&self.inner, key, cx)
    }

    fn child_node(&self, key: TreeKey, cx: &App) -> Option<ContributedNode> {
        let inner = &self.inner;
        inner.sync();
        let place = inner.place(key)?;
        let snap = inner.snapshot()?;
        let p = profile(place.profile, cx)?;
        Some(match &place.node {
            Node::Scripts { .. } => return None,
            Node::Library { id, label } => {
                let files = inner
                    .lib_index(&snap, *id, p.environment, cx)
                    .map_or(0, |i| i.files);
                ContributedNode {
                    key,
                    row: library_row(&snap, *id, label, files, cx),
                    children: lazy(key),
                    filter_text: None,
                }
            }
            Node::Entry {
                entry,
                status,
                contains_changes,
                shared,
                ..
            } => {
                let folder = entry.kind == EntryKind::Folder;
                ContributedNode {
                    key,
                    row: entry_row(entry, *status, *contains_changes, *shared, cx),
                    children: if folder {
                        lazy(key)
                    } else {
                        ContributedChildren::Leaf
                    },
                    // The explorer filter matches loaded file names (X-2).
                    filter_text: (!folder).then(|| entry.rel.name().to_string()),
                }
            }
            Node::Problems { count, .. } => ContributedNode {
                key,
                row: problems_row(*count, cx),
                children: ContributedChildren::Leaf,
                filter_text: None,
            },
        })
    }

    /// A file opens under the connection and database its row is under (Task 14's
    /// `open_script`); the problems row opens the diagnostics popover (surface 11); a link
    /// pointing outside the library opens nothing.
    fn activate(&self, key: TreeKey, window: &mut Window, cx: &mut App) {
        let Some(place) = self.inner.place(key) else {
            return;
        };
        match &place.node {
            Node::Entry { library, entry, .. } if entry.kind != EntryKind::Folder => {
                if matches!(entry.link, LinkState::OutsideLink | LinkState::Loop) {
                    return;
                }
                open_entry(&self.inner, &place, *library, entry, cx);
            }
            Node::Problems { library, .. } => {
                super::diagnostics::open_popover(&place, *library, key, window, cx);
            }
            _ => {}
        }
    }

    fn context_menu(&self, key: TreeKey, cx: &App) -> Vec<MenuItemSpec> {
        match self.inner.place(key) {
            Some(place) => super::file_ops::menu(&place, cx),
            None => Vec::new(),
        }
    }

    fn generation(&self) -> u64 {
        self.inner.generation()
    }
}

/// Opens a listed file under the place's connection and database (R-M4b-4).
fn open_entry(inner: &Inner, place: &Place, library: LibraryId, entry: &ScriptEntry, cx: &mut App) {
    let _ = inner;
    let Ok(rel) = super::RelPath::parse(entry.rel.as_str()) else {
        return;
    };
    #[cfg(feature = "test-support")]
    inner.opens.borrow_mut().push((
        library,
        entry.rel.as_str().to_owned(),
        Some(place.profile),
        Some(place.database.clone()),
    ));
    super::open_script(
        cx,
        library,
        rel,
        Some(place.profile),
        Some(super::DatabaseName::new(&place.database)),
    );
}

// ---- queries for the menus and commands -------------------------------------------------------

/// The place of tree row `key`, when it is a Scripts row.
pub(crate) fn place_of(key: TreeKey, cx: &App) -> Option<Place> {
    host(cx)?.place(key)
}

/// The place of the explorer's cursor row, when it is a Scripts row.
pub(crate) fn cursor_place(cx: &App) -> Option<(TreeKey, Place)> {
    let tree = crate::connections::explorer::tree(cx)?;
    let key = tree.read(cx).cursor()?;
    Some((key, place_of(key, cx)?))
}

// ## m4b-t13-explorer fix round 1 (review m10)
/// Whether a connection other than `place`'s shows file `rel` of `library`, by the chooser's
/// candidate rule (R-M4b-9, R-M4b-31): a place the explorer lists or a profile's configured
/// database, where the library is bound and the profile's environment shows the file.
pub(crate) fn shown_under_another_connection(
    place: &Place,
    library: LibraryId,
    rel: &hacivat_scripts::RelPath,
    cx: &App,
) -> bool {
    let Some(inner) = host(cx) else {
        return false;
    };
    let Some(snap) = inner.snapshot() else {
        return false;
    };
    let profiles: Vec<Arc<Profile>> = cx
        .try_global::<ConnectionsModel>()
        .map(|m| m.profiles().to_vec())
        .unwrap_or_default();
    profiles.iter().any(|q| {
        let mut dbs = asked_databases(q, cx);
        let configured = match database_level(q.engine) {
            DbLevel::File => Some("main".to_string()),
            _ => q.database.clone(),
        };
        if let Some(db) = configured
            && !dbs.contains(&db)
        {
            dbs.push(db);
        }
        dbs.iter().any(|db| {
            (q.id, db.as_str()) != (place.profile, place.database.as_str())
                && libraries_at(&inner, q, db, &snap, cx).contains(&library)
                && snap
                    .view(q.environment)
                    .library(library)
                    .is_some_and(|l| l.entry(rel).is_some())
        })
    })
}

/// The environment a place's connection has now.
pub(crate) fn env_of(place: &Place, cx: &App) -> Option<EnvTag> {
    profile(place.profile, cx).and_then(|p| p.environment)
}

/// The entries `place`'s connection shows in `folder` of `library` (sibling names for the
/// name field's live check).
pub(crate) fn shown_names(
    place: &Place,
    library: LibraryId,
    folder: &str,
    cx: &App,
) -> Vec<String> {
    let Some(inner) = host(cx) else {
        return Vec::new();
    };
    let Some(snap) = inner.snapshot() else {
        return Vec::new();
    };
    let env = env_of(place, cx);
    inner
        .lib_index(&snap, library, env, cx)
        .and_then(|i| i.children.get(folder).cloned())
        .map(|l| l.iter().map(|e| e.rel.name().to_string()).collect())
        .unwrap_or_default()
}

/// `f` over library `library` as `place`'s connection sees it now (`view(env)`, R-T10-2).
pub(crate) fn with_view<R>(
    place: &Place,
    library: LibraryId,
    cx: &App,
    f: impl FnOnce(&LibraryView<'_>) -> R,
) -> Option<R> {
    let snap = host(cx)?.snapshot()?;
    let env = env_of(place, cx);
    let view = snap.view(env);
    let lib = view.library(library)?;
    Some(f(&lib))
}

/// A library's folder name for display (escaped, NFC).
pub(crate) fn library_name(lib: &LibraryView<'_>) -> String {
    lib.library()
        .root
        .path
        .file_name()
        .map(|n| display_name(&n.to_string_lossy()))
        .unwrap_or_default()
}

/// The canonical path of an entry of `library` (Reveal, Copy path); `rel` `None` is the root.
pub(crate) fn entry_path(
    place: &Place,
    library: LibraryId,
    rel: Option<&str>,
    cx: &App,
) -> Option<std::path::PathBuf> {
    with_view(place, library, cx, |lib| {
        let mut p = lib.library().root.canonical.clone();
        if let Some(rel) = rel {
            for part in rel.split('/') {
                p.push(part);
            }
        }
        p
    })
}

// ---- tests ------------------------------------------------------------------------------------

/// What file activations asked `open_script` for, in order (tests).
#[cfg(feature = "test-support")]
pub fn opens_asked(cx: &App) -> Vec<OpenAsked> {
    host(cx)
        .map(|h| h.opens.borrow().clone())
        .unwrap_or_default()
}

/// Takes the library service out of the app (tests drop it before the core runtime stops, so
/// its actor ends first).
#[cfg(feature = "test-support")]
pub fn take_service(cx: &mut App) -> Option<Arc<LibraryService>> {
    let inner = host(cx)?;
    inner.relay.borrow_mut().take();
    let svc = inner.service.borrow_mut().take();
    inner.seen.set(None);
    contributors_changed(cx);
    svc
}

/// The key of a Scripts node (`rel` `None`), or of a folder (`folder`) or file row of
/// `library` (tests: a saved `workspace.json` names them).
#[cfg(feature = "test-support")]
pub fn key_for(
    profile: ProfileId,
    database: &str,
    library: Option<LibraryId>,
    rel: Option<(&str, bool)>,
) -> TreeKey {
    match (library, rel) {
        (None, _) => scripts_key(profile, database, None, K_SCRIPTS, ""),
        (Some(l), None) => scripts_key(profile, database, Some(l), K_LIBRARY, ""),
        (Some(l), Some((rel, folder))) => scripts_key(
            profile,
            database,
            Some(l),
            if folder { K_FOLDER } else { K_FILE },
            rel,
        ),
    }
}

/// The place behind a Scripts key, as `(profile, database, library, rel)` (tests).
#[cfg(feature = "test-support")]
pub fn describe_key(
    key: TreeKey,
    cx: &App,
) -> Option<(ProfileId, String, Option<LibraryId>, Option<String>)> {
    let place = place_of(key, cx)?;
    let rel = place.entry().map(|e| e.rel.as_str().to_owned());
    Some((place.profile, place.database.clone(), place.library(), rel))
}

#[cfg(test)]
mod tests {
    use super::*;

    // ## m4b-t13-explorer (re-review n6)
    #[test]
    fn the_boot_sweeps_only_when_the_registry_is_known_absent() {
        assert_eq!(boot_plan(Ok(false)), BootPlan::SweepOrphans);
        assert_eq!(boot_plan(Ok(true)), BootPlan::OpenService);
        for kind in [
            std::io::ErrorKind::PermissionDenied,
            std::io::ErrorKind::NotFound,
            std::io::ErrorKind::Other,
        ] {
            assert_eq!(
                boot_plan(Err(kind.into())),
                BootPlan::OpenService,
                "{kind:?} never sweeps"
            );
        }
    }

    /// On a real folder the registry's parent cannot be searched in: `try_exists` answers an
    /// error, and the boot plans no sweep.
    #[cfg(unix)]
    #[test]
    fn an_unsearchable_config_folder_never_leads_to_a_sweep() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let locked = dir.path().join("config");
        std::fs::create_dir(&locked).unwrap();
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
        let probe = locked.join("libraries.toml").try_exists();
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o700)).unwrap();
        // Root may search anything; the rule under test is the plan for an error.
        if probe.is_err() {
            assert_eq!(boot_plan(probe), BootPlan::OpenService);
        }
    }

    #[test]
    fn keys_are_scoped_by_profile_database_library_kind_and_path() {
        let (a, b) = (ProfileId::new(), ProfileId::new());
        let lib = LibraryId::new();
        let k = |p, db: &str, kind, rel: &str| scripts_key(p, db, Some(lib), kind, rel);
        let base = k(a, "orders", K_FILE, "a.sql");
        assert_eq!(base, k(a, "orders", K_FILE, "a.sql"));
        assert_ne!(base, k(b, "orders", K_FILE, "a.sql"));
        assert_ne!(base, k(a, "orders2", K_FILE, "a.sql"));
        assert_ne!(base, k(a, "orders", K_FOLDER, "a.sql"));
        assert_ne!(base, k(a, "orders", K_FILE, "b.sql"));
        assert_ne!(
            base,
            scripts_key(a, "orders", Some(LibraryId::new()), K_FILE, "a.sql")
        );
        // Length prefixes: moving a byte between parts gives another key.
        assert_ne!(
            scripts_key(a, "ab", Some(lib), K_FILE, "c"),
            scripts_key(a, "a", Some(lib), K_FILE, "bc")
        );
    }

    #[test]
    fn only_the_five_tags_name_an_environment_folder() {
        assert_eq!(env_folder_tag("@production"), Some(EnvTag::Production));
        assert_eq!(env_folder_tag("@PRODUCTION"), Some(EnvTag::Production));
        assert_eq!(env_folder_tag("@Staging"), Some(EnvTag::Staging));
        for no in ["@prod", "@productİon", "@production ", "production", "@"] {
            assert_eq!(env_folder_tag(no), None, "{no}");
        }
    }

    #[test]
    fn parents_of_paths() {
        assert_eq!(parent("a.sql"), "");
        assert_eq!(parent("x/a.sql"), "x");
        assert_eq!(parent("x/y/a.sql"), "x/y");
    }
}
