//! Opening a script (HV-SCRIPT-06, R-M4b-4, R-M4b-25, R-M4b-27): context from location. The
//! tab is bound to the (connection, database) it is opened under and keeps it for its life.
//!
//! ## m4b-t01b-seams: the stub opened nothing.
//!
//! ## m4b-t14-tabs
//! - **One tab per (file, connection, database)** (R-M4b-4). [`open_script`] interns the
//!   binding (`tabs::intern`); `Workspace::open_script_tab` focuses the tab that already has it,
//!   or opens one in the focused pane. The file is read when the tab is first drawn, off the UI
//!   thread (`LibraryService::read`), so the tab shows "Opening…" first.
//! - **The database is resolved at open, never later** (R-T1b-1). `None` with a profile means
//!   the database the profile runs on now ([`resolve_database`]); the tab stores that name, so a
//!   later profile edit never moves it, and Run compares against it (R-M4b-27).
//! - **Where a script may run** ([`session_plan`], R-M4b-25): PostgreSQL only on the database
//!   its session (or, unconnected, its profile) is on; SQLite only on `main`; MySQL/MariaDB on
//!   any database, always through a session of the tab's own (never the profile's default one,
//!   which consoles and the explorer share) and switched with `USE` before Run is enabled when
//!   the database is not the profile's. A single-session MySQL profile shares its one session
//!   only on its own database; elsewhere the script is refused. Every refusal opens no tab and
//!   says why; a refused restored tab stays, runs nothing and says why.
//! - **The service** ([`ScriptsRuntime`]): one `LibraryService` for the app on the core runtime,
//!   started on first use (`service`), and this instance's recovery lock. Tests install their
//!   own ([`install_service`]).

use std::rc::Rc;
use std::sync::{Arc, OnceLock};

use hacivat_l10n::ArgValue;
use hacivat_meta::kind::{DbLevel, database_level};
use hacivat_proto::{EnvTag, ProfileId};
use hacivat_scripts::recovery::Instance;
use hacivat_scripts::service::LibraryService;
use hacivat_session::profile::Profile;
use hacivat_settings::LocalPaths;
use hacivat_ui::toast::{Toast, ToastKind, ToastQueue};
use hacivat_ui_core::gpui_surface::{App, Global};

use crate::connections::ConnectionsModel;
use crate::console::{ConsoleId, Consoles};
use crate::scripts::tabs::{self, ScriptTab};
use crate::scripts::{DatabaseName, LibraryId, RelPath};
use crate::shell::AppState;

// ---- the service -------------------------------------------------------------------------------

/// What the script tabs use of the app: the one library service (Task 13's,
/// `scripts::projection::service`; never a second one), the app's paths, the core runtime
/// every file and service call runs on, and this instance's recovery lock (taken on the
/// blocking pool the first time a copy is written).
#[derive(Clone)]
pub struct ScriptsRuntime {
    pub service: Arc<LibraryService>,
    pub paths: LocalPaths,
    pub handle: tokio::runtime::Handle,
    pub instance: Arc<OnceLock<Option<Instance>>>,
}

/// This app instance's recovery lock, once taken (set by `scripts::install`).
#[derive(Default)]
pub(crate) struct RecoveryInstance(Arc<OnceLock<Option<Instance>>>);

impl Global for RecoveryInstance {}

/// Makes `service` the app's library service (tests and the shots pass one over fake event
/// sources): Task 13's `projection::set_service`.
pub fn install_service(service: Arc<LibraryService>, cx: &mut App) {
    crate::scripts::projection::set_service(service, cx);
}

/// The script runtime, with the library service opened now if it is not open yet (Task 13's
/// `projection::ensure_service`); `None` before the connections are set up.
pub fn runtime(cx: &mut App) -> Option<ScriptsRuntime> {
    crate::scripts::projection::ensure_service(cx)?;
    current(cx)
}

/// The script runtime while the library service is open (never opens it).
pub fn current(cx: &App) -> Option<ScriptsRuntime> {
    Some(ScriptsRuntime {
        service: service(cx)?,
        paths: cx.try_global::<AppState>()?.paths.clone(),
        handle: cx.try_global::<ConnectionsModel>()?.runtime(),
        instance: cx.try_global::<RecoveryInstance>()?.0.clone(),
    })
}

/// The app's library service, when it is open (never opens it).
pub fn service(cx: &App) -> Option<Arc<LibraryService>> {
    crate::scripts::projection::service(cx)
}

/// The environment file calls take for a tab bound to `profile`, read now (R-T5-6): `None` for
/// an unbound tab or a profile without a tag, which sees no `@`-scoped file.
pub fn env_of(profile: Option<ProfileId>, cx: &App) -> Option<EnvTag> {
    let id = profile?;
    cx.try_global::<ConnectionsModel>()?.get(id)?.environment
}

/// The live profile `id`.
pub(crate) fn profile_of(id: ProfileId, cx: &App) -> Option<Arc<Profile>> {
    cx.try_global::<ConnectionsModel>()?.get(id)
}

// ---- where a script may run --------------------------------------------------------------------

/// How a script tab gets its session (R-M4b-25, R-M4b-27).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionPlan {
    /// As a console of the profile (PostgreSQL, SQLite, a single-session MySQL on its own
    /// database): a mismatch before Run refuses, never switches (others share the session).
    Shared,
    /// A session of the tab's own (MySQL/MariaDB); `switch`: `USE` the database before Run is
    /// enabled, because it is not the one the profile connects to.
    Own { switch: bool },
}

/// Why a script is not opened (or a restored one runs nothing).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// The connection no longer exists.
    NoProfile,
    /// PostgreSQL/SQLite: the connection runs on `current`, not on the script's database.
    OtherDatabase { current: String },
    /// Nothing tells which database the connection runs on.
    DatabaseUnknown,
    /// A single-session MySQL profile cannot give the script a session on another database.
    SingleSession,
}

/// The database a script opened under `profile` without one runs on: the database the
/// profile runs on now (R-T1b-1): the session's, else the profile's; `main` on SQLite.
pub fn resolve_database(profile: &Profile, cx: &App) -> Option<DatabaseName> {
    if database_level(profile.engine) == DbLevel::File {
        return Some(DatabaseName::new("main"));
    }
    crate::explorer::live::session_database(profile.id, cx)
        .or_else(|| profile.database.clone())
        .filter(|d| !d.is_empty())
        .map(|d| DatabaseName::new(&d))
}

/// How a script under `database` of `profile` runs, or why it may not.
pub fn session_plan(
    profile: &Profile,
    database: &DatabaseName,
    cx: &App,
) -> Result<SessionPlan, Refusal> {
    let db = database.as_str();
    match database_level(profile.engine) {
        DbLevel::File => {
            if db == "main" {
                Ok(SessionPlan::Shared)
            } else {
                Err(Refusal::OtherDatabase {
                    current: "main".to_string(),
                })
            }
        }
        DbLevel::Database => {
            // The I-3 rule of `console::open_with_text`: the session's database, else the
            // profile's; never a guess.
            let current = crate::explorer::live::session_database(profile.id, cx)
                .or_else(|| profile.database.clone());
            match current {
                Some(current) if current == db => Ok(SessionPlan::Shared),
                Some(current) => Err(Refusal::OtherDatabase { current }),
                None => Err(Refusal::DatabaseUnknown),
            }
        }
        DbLevel::SchemaIsDatabase => {
            let own_db = profile.database.as_deref() == Some(db);
            if profile.single_session {
                if own_db {
                    Ok(SessionPlan::Shared)
                } else {
                    Err(Refusal::SingleSession)
                }
            } else {
                Ok(SessionPlan::Own { switch: !own_db })
            }
        }
    }
}

/// The notice of a script that is not opened.
pub(crate) fn refusal_text(
    refusal: &Refusal,
    file: &str,
    conn: &str,
    database: &str,
    cx: &App,
) -> hacivat_ui_core::gpui_surface::SharedString {
    let file = ArgValue::Text(tabs::display_name(file));
    let conn = ArgValue::Text(tabs::display_name(conn));
    let database = ArgValue::Text(tabs::display_name(database));
    match refusal {
        Refusal::NoProfile => tabs::msg(cx, "hv-scripts-open-no-profile", vec![("file", file)]),
        Refusal::OtherDatabase { current } => tabs::msg(
            cx,
            "hv-scripts-open-other-db",
            vec![
                ("file", file),
                ("conn", conn),
                ("database", database),
                ("current", ArgValue::Text(tabs::display_name(current))),
            ],
        ),
        Refusal::DatabaseUnknown => tabs::msg(
            cx,
            "hv-scripts-open-db-unknown",
            vec![("file", file), ("conn", conn), ("database", database)],
        ),
        Refusal::SingleSession => tabs::msg(
            cx,
            "hv-scripts-open-single-session",
            vec![("file", file), ("conn", conn), ("database", database)],
        ),
    }
}

/// Says why a script was not opened (a toast: nothing opened, nothing ran).
fn refuse(refusal: Refusal, rel: &RelPath, conn: String, database: String, cx: &mut App) {
    let file = crate::scripts::file_name(rel).to_string();
    tracing::info!(?refusal, "scripts: a script was not opened");
    let text: hacivat_ui::text::TextFn =
        Rc::new(move |cx: &App| refusal_text(&refusal, &file, &conn, &database, cx));
    ToastQueue::push(cx, Toast::new(ToastKind::Info, text));
}

// ---- open ----------------------------------------------------------------------------------------

/// Opens `rel` of `library` under `profile` and `database` (`None` without a profile:
/// unbound, "Open without a connection"; `None` with one: the database the profile runs on now,
/// resolved here and stored, R-T1b-1).
///
/// Focuses the tab already open for the same (library, file, connection, database); otherwise
/// opens one in the focused pane, after the current update, whose file is read when it is first
/// drawn. Answers the console of a tab that was already open and loaded, else `None` (a new
/// tab's console is registered when its file is read). A script that may not run where asked
/// (R-M4b-25) opens nothing and a notice says why.
pub fn open_script(
    cx: &mut App,
    library: LibraryId,
    rel: RelPath,
    profile: Option<ProfileId>,
    database: Option<DatabaseName>,
) -> Option<ConsoleId> {
    // A library the service does not list opens nothing (a stale row, a detached library):
    // every caller found the file in a snapshot first (the tree, Go to script), so the service
    // is running already and is never started here.
    if service(cx).is_none_or(|s| s.snapshot().library(library).is_none()) {
        tracing::info!(%library, "scripts: not opened: the library is not attached");
        return None;
    }
    let database = match profile {
        None => None,
        Some(id) => {
            let Some(p) = profile_of(id, cx) else {
                refuse(Refusal::NoProfile, &rel, String::new(), String::new(), cx);
                return None;
            };
            let resolved = database.or_else(|| resolve_database(&p, cx));
            let Some(db) = resolved else {
                refuse(
                    Refusal::DatabaseUnknown,
                    &rel,
                    p.name.clone(),
                    String::new(),
                    cx,
                );
                return None;
            };
            if let Err(refusal) = session_plan(&p, &db, cx) {
                refuse(refusal, &rel, p.name.clone(), db.as_str().to_string(), cx);
                return None;
            }
            Some(db)
        }
    };
    let script = tabs::intern(
        ScriptTab {
            library,
            rel,
            profile,
            database,
        },
        cx,
    );
    // Already open and read: its console, focused after this update.
    let open = tabs::doc_of_script(script, cx).and_then(|doc| doc.read(cx).console());
    crate::exec_ui::in_workspace_window(cx, move |ws, window, cx| {
        let tab = ws.update(cx, |ws, cx| ws.open_script_tab(script, cx));
        if let Some(editor) = tab
            .and_then(|tab| Consoles::by_tab(cx, tab))
            .and_then(|c| Consoles::get(cx, c))
            .and_then(|s| s.editor.clone())
        {
            let focus = hacivat_ui_core::gpui_surface::Focusable::focus_handle(editor.read(cx), cx);
            window.focus(&focus, cx);
        }
    });
    open
}

// ---- open under another connection ----------------------------------------------------------------

/// The other places a script tab's file can be opened under (N2): every profile's configured
/// database (R-M4b-31) whose binding includes the library and whose environment shows the file
/// (R-M4b-6), except the tab's own. Production is listed like any other; the chooser never
/// preselects it (R-M4b-9, Task 15).
pub fn other_places(tab: &ScriptTab, cx: &App) -> Vec<(ProfileId, DatabaseName)> {
    let (Some(service), Some(m)) = (service(cx), cx.try_global::<ConnectionsModel>()) else {
        return Vec::new();
    };
    let snap = service.snapshot();
    let mut out = Vec::new();
    for p in m.profiles() {
        let Some(db) = resolve_database(p, cx) else {
            continue;
        };
        if tab.profile == Some(p.id) && tab.database.as_ref() == Some(&db) {
            continue;
        }
        if session_plan(p, &db, cx).is_err() {
            continue;
        }
        let logical = match database_level(p.engine) {
            DbLevel::File => sqlite_stem(p),
            _ => hacivat_scripts::LogicalDb::parse(db.as_str()).ok(),
        };
        let Some(logical) = logical else { continue };
        let node = hacivat_scripts::DbNode {
            profile: p.id,
            database: db.clone(),
            logical,
        };
        let libs: Vec<hacivat_scripts::LibrarySnapshot> =
            snap.libraries.iter().map(|l| (**l).clone()).collect();
        if !hacivat_scripts::resolve(&node, &libs, &snap.mappings).contains(&tab.library) {
            continue;
        }
        let shown = snap
            .view(p.environment)
            .library(tab.library)
            .and_then(|l| l.entry(&tab.rel))
            .is_some();
        if shown {
            out.push((p.id, db));
        }
    }
    out
}

/// The logical name of a SQLite profile: its file's stem (R-M4b-3, R-M4b-32).
fn sqlite_stem(p: &Profile) -> Option<hacivat_scripts::LogicalDb> {
    let hacivat_session::profile::ProfileEndpoint::File { path, .. } = &p.endpoint else {
        return None;
    };
    hacivat_scripts::LogicalDb::parse(path.file_stem()?.to_str()?).ok()
}

/// A hook Task 15's chooser fills: "Open under another connection…" of a script tab. Without
/// it, one place that is not production opens directly; anything else waits for the chooser
/// (production is never chosen for the user, R-M4b-9).
pub type OpenUnderFn = Rc<dyn Fn(&ScriptTab, Vec<(ProfileId, DatabaseName)>, &mut App)>;

#[derive(Clone)]
struct OpenUnderHook(OpenUnderFn);

impl Global for OpenUnderHook {}

/// Installs Task 15's chooser for "Open under another connection…".
pub fn set_open_under_another(hook: OpenUnderFn, cx: &mut App) {
    cx.set_global(OpenUnderHook(hook));
}

/// "Open under another connection…" of the tab bound as `tab`: a new tab, never a rebind.
pub fn open_under_another(tab: &ScriptTab, cx: &mut App) {
    let places = other_places(tab, cx);
    if let Some(hook) = cx.try_global::<OpenUnderHook>().cloned() {
        (hook.0)(tab, places, cx);
        return;
    }
    let production =
        |p: &ProfileId| profile_of(*p, cx).and_then(|p| p.environment) == Some(EnvTag::Production);
    if let [(profile, db)] = places.as_slice()
        && !production(profile)
    {
        open_script(
            cx,
            tab.library,
            tab.rel.clone(),
            Some(*profile),
            Some(db.clone()),
        );
    } else {
        tracing::info!(
            places = places.len(),
            "scripts: open under another connection waits for the chooser (Task 15)"
        );
    }
}
