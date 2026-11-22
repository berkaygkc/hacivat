//! Script tabs (HV-SCRIPT-06, R-M4b-4, R-M4b-22, R-M4b-29): a script tab is the tab kind
//! [`TabKind::Script`](crate::shell::TabKind) whose handle ([`ScriptTabId`]) names the
//! (library, file, connection, database) the tab was opened under, for its whole life.
//!
//! ## m4b-t01b-seams: the table of handles and the tab's title. `TabKind` is `Copy` (its users
//! copy it everywhere), so the four values live behind a handle, as M4's DDL view tabs do
//! (`DdlObjectId`, ruling R-W0-1); `workspace.json` stores the values themselves (`kind:
//! "script"`, `library`, `rel_path`, `profile`, `database`). The same values always get the same
//! handle, so "one tab per (file, connection)" (R-M4b-4) compares handles.
//!
//! ## m4b-t14-tabs
//! A script tab is a console whose text comes from a file (`console::view` builds its host with
//! [`ScriptDoc`] beside the editor). The doc owns what the file adds to a console:
//! - **The binding never changes** (R-M4b-4, R-M4b-30). The doc reads its binding from the
//!   handle and nothing here or in the host writes another profile into it; a connect never
//!   binds an unbound script tab (`ConsoleHost::on_connections` skips it), and a tab whose
//!   profile is gone runs nothing and says so ([`Notice::Removed`]).
//! - **Run checks the database first** ([`run_gate`], R-M4b-27): before the gate, the session's
//!   database must be the tab's. MySQL/MariaDB (a session of the tab's own) switches back once
//!   with `Request::SwitchSchema` and runs only when the re-probe reads the tab's database;
//!   everything else refuses with a notice naming both databases. Markdown, a T2 preview, an
//!   unbound tab and a tab whose open switch did not land never run.
//! - **Dirty is the tab's** (R-M4b-38): the editor's version against `clean_version`, the
//!   version after the load, a reload or a save.
//! - **Disk changes** (HV-SCRIPT-08) come from a `Subscription` for the tab's environment
//!   (R-T10-2): a clean tab reloads in place (`replace_from_disk(text, clean_version)`, R-T11-1;
//!   a preview through `reload_preview`); a dirty one shows "Changed on disk"; a delete shows
//!   "Deleted on disk"; a followed rename re-checks the scope first and shows "Moved to …"; a
//!   move out of the connection's scope shows "No longer listed under this connection".
//! - **Recovery** (R-M4b-5, R-T5-3): 5 s after the last edit of a dirty tab its text is copied
//!   under `<data>/script-recovery/` (never into the library), and the copy goes when the tab
//!   is clean again, saved, or closed with "Don't save".
//! - **The last tab of a file** to close tells the service to forget the file (I4).

use std::collections::{BTreeMap, HashMap, HashSet};
use std::future::Future;
use std::rc::Rc;
use std::time::Duration;

use crate::shell::hooks::TabDecor;
use hacivat_l10n::ArgValue;
use hacivat_proto::{EnvTag, ProfileId};
use hacivat_scripts::fileio::{DiskStamp, FileMeta, ReadError, SizeTier, TextEncoding};
use hacivat_scripts::recovery::{self, CopyMeta, RecoveryEntry, RecoveryKey};
use hacivat_scripts::{EntryKind, LibraryEvent};
use hacivat_session::session::{Request, SessionHandle};
use hacivat_ui::panes::TabId;
use hacivat_ui_core::gpui_surface::{
    App, Context, Entity, EventEmitter, Global, SharedString, Task, WeakEntity,
};
use hacivat_ui_editor::EditorView;

use crate::connections::ConnectionsModel;
use crate::console::{ConsoleId, ConsoleSource, Consoles};

use crate::scripts::open::{self, Refusal, SessionPlan};
use crate::scripts::{DatabaseName, LibraryId, RelPath};

/// What a script tab is bound to (R-M4b-4: never changed while the tab lives).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ScriptTab {
    pub library: LibraryId,
    pub rel: RelPath,
    /// `None`: an unbound script tab ("Open without a connection", or a restored tab whose
    /// profile was deleted). A connect never binds it (R-M4b-30).
    pub profile: Option<ProfileId>,
    /// ## m4b-t14-tabs (R-T1b-1): for a bound tab, the database it was opened under, resolved
    /// at open (`open::resolve_database`); never `None` meaning "the profile's database".
    pub database: Option<DatabaseName>,
}

/// A script tab's handle in [`TabKind::Script`](crate::shell::TabKind).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ScriptTabId(pub u64);

/// The handles handed out so far (a global; `crate::scripts::install` sets it).
#[derive(Debug, Default)]
pub struct ScriptTabs {
    by_id: BTreeMap<u64, ScriptTab>,
    next: u64,
}

impl Global for ScriptTabs {}

/// The handle of `tab`: the same values get the same handle.
pub fn intern(tab: ScriptTab, cx: &mut App) -> ScriptTabId {
    let tabs = cx.default_global::<ScriptTabs>();
    if let Some((id, _)) = tabs.by_id.iter().find(|(_, t)| **t == tab) {
        return ScriptTabId(*id);
    }
    tabs.next += 1;
    let id = tabs.next;
    tabs.by_id.insert(id, tab);
    ScriptTabId(id)
}

/// What handle `id` names.
pub fn spec(id: ScriptTabId, cx: &App) -> Option<ScriptTab> {
    cx.try_global::<ScriptTabs>()?.by_id.get(&id.0).cloned()
}

// ## m4b-t14-tabs
/// The tab follows its file to `to` (a rename on disk the service paired, after the scope was
/// checked again): the path changes, never the connection or the database.
pub(crate) fn follow_rename(id: ScriptTabId, to: RelPath, cx: &mut App) {
    if let Some(tab) = cx.default_global::<ScriptTabs>().by_id.get_mut(&id.0) {
        tab.rel = to;
    }
}

/// The tab's title: "file · connection" (NOTES surface 5, `hv-scripts-tab-title`), "file ·
/// connection · database" under a MySQL/MariaDB database other than the profile's, or "file ·
/// no connection" when it is unbound or its profile is gone.
pub fn tab_title(id: ScriptTabId, cx: &App) -> SharedString {
    let Some(tab) = spec(id, cx) else {
        return SharedString::default();
    };
    let file = display_name(crate::scripts::file_name(&tab.rel));
    let profile = tab.profile.and_then(|p| open::profile_of(p, cx));
    match profile {
        Some(p) => {
            let conn = display_name(&p.name);
            let other_db = tab.database.as_ref().filter(|db| {
                hacivat_meta::kind::database_level(p.engine)
                    == hacivat_meta::kind::DbLevel::SchemaIsDatabase
                    && p.database.as_deref() != Some(db.as_str())
            });
            match other_db {
                Some(db) => msg(
                    cx,
                    "hv-scripts-tab-title-db",
                    vec![
                        ("file", ArgValue::Text(file)),
                        ("conn", ArgValue::Text(conn)),
                        ("database", ArgValue::Text(display_name(db.as_str()))),
                    ],
                ),
                None => msg(
                    cx,
                    "hv-scripts-tab-title",
                    vec![
                        ("file", ArgValue::Text(file)),
                        ("conn", ArgValue::Text(conn)),
                    ],
                ),
            }
        }
        None => msg(
            cx,
            "hv-scripts-tab-title-unbound",
            vec![("file", ArgValue::Text(file))],
        ),
    }
}

/// `name` for display (R-M4b-23): NFC, with control characters, bidi controls and invisible
/// characters shown as `<U+XXXX>` (## m4b-t14-tabs: Task 4's `display_name`, carries-w1).
pub fn display_name(name: &str) -> String {
    hacivat_scripts::pathsafe::display_name(name)
}

// ## m4b-t14-tabs ------------------------------------------------------------------------------

/// A Fluent message by its ID with arguments.
pub(crate) fn msg(cx: &App, id: &str, args: Vec<(&'static str, ArgValue)>) -> SharedString {
    match cx.try_global::<crate::l10n_global::L10nGlobal>() {
        Some(g) => g.l10n.t_dynamic(id, args).into(),
        None => SharedString::from(id.to_string()),
    }
}

/// The client number of a script tab's own session (`SessionKind::Console(ClientId(n))`): far
/// above any console's number, one per handle.
pub(crate) fn client_number(id: ScriptTabId) -> u32 {
    u32::try_from(id.0)
        .unwrap_or(u32::MAX - SCRIPT_CLIENT_BASE)
        .saturating_add(SCRIPT_CLIENT_BASE)
}

const SCRIPT_CLIENT_BASE: u32 = 1 << 30;

/// The app-wide parts of script tabs (`scripts::install`): the save commands.
pub(crate) fn install(cx: &mut App) {
    crate::scripts::save::register_commands(cx);
    cx.set_global(open::RecoveryInstance::default());
    // The library service stops with the app, so the core runtime's shutdown never waits for
    // its actor.
    cx.on_app_quit(|cx| {
        if let Some(service) = open::service(cx) {
            service.shutdown();
        }
        // ## m4b-t14-tabs fix round 2 (m-fr1-5): the quit waits for "Don't save"'s removals.
        let discards = std::mem::take(&mut cx.default_global::<QuitDiscards>().0);
        async move {
            for discard in discards {
                discard.await;
            }
        }
    })
    .detach();
}

/// The recovery copies a quit's "Don't save" is removing (fix round 2, m-fr1-5). Its futures
/// are `!Send`: only `unsaved_prompt::quit_guard` pushes to it, when the quit goes on, and the
/// quit hook in [`install`] drains it (fix round 3, m-fr2-3); nothing else may.
#[derive(Default)]
struct QuitDiscards(Vec<std::pin::Pin<Box<dyn Future<Output = ()>>>>);

impl Global for QuitDiscards {}

/// Runs `fut` on the core runtime and answers its output on the calling executor (`None`
/// when the runtime dropped it): file and service calls never run on the UI thread.
pub(crate) fn on_core<T: Send + 'static>(
    cx: &App,
    fut: impl Future<Output = T> + Send + 'static,
) -> impl Future<Output = Option<T>> + 'static {
    let (tx, rx) = tokio::sync::oneshot::channel();
    match cx.try_global::<ConnectionsModel>() {
        Some(m) => {
            m.runtime().spawn(async move {
                let _ = tx.send(fut.await);
            });
        }
        None => drop(tx),
    }
    async move { rx.await.ok() }
}

// ---- the doc of one tab --------------------------------------------------------------------------

/// Where the tab's file stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Load {
    /// Being read ("Opening…").
    Opening,
    /// Not UTF-8 and no BOM: the user names the encoding (NOTES surface 10).
    NeedsEncoding {
        guess: Option<TextEncoding>,
    },
    /// Not opened; the reason selects `hv-scripts-open-failed-body`.
    Failed(&'static str),
    /// The tab may not run where it is bound (a restored tab; R-M4b-25).
    Refused(Refusal),
    Ready,
}

/// Why a Run was refused (R-M4b-27, R-M4b-30), as the notice says it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunRefused {
    /// The session is on `current`; MySQL switched back once and it still is.
    Mysql { bound: String, current: String },
    /// PostgreSQL/SQLite: the connection now opens `current` (it was edited).
    Edited { bound: String, current: String },
    /// Nothing tells which database the session is on.
    Unknown { bound: String },
}

/// Why the tab's file is no longer listed under its connection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Unlisted {
    /// It moved to a place the connection does not show; its new path is not shown.
    Moved,
    /// It lies in `folder`, an environment folder the connection does not show (its
    /// environment changed).
    Env { folder: String },
}

/// One notice under the toolbar (NOTES surfaces 5, 6, 12).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Notice {
    /// "Changed on disk": the tab is dirty and the file changed.
    Changed,
    Deleted,
    Unlisted(Unlisted),
    Moved {
        path: String,
    },
    RunRefused(RunRefused),
    /// The MySQL switch made at open did not land.
    SwitchFailed {
        database: String,
    },
    /// The tab's profile no longer exists (R-M4b-30).
    Removed,
    /// Unsaved text recovered from an earlier run.
    Recovered {
        at: std::time::SystemTime,
        unknown: bool,
    },
    /// A Markdown note: it does not run.
    Markdown,
    /// A T2 preview: read-only, not saved, not run.
    Preview,
    /// ## m4b-t14-tabs fix round 1 (I3, R-T14-3): the connection could not give this MySQL tab
    /// a session of its own; it runs nothing.
    OneSession,
}

impl Notice {
    /// Notices that replace each other (one per kind).
    fn same_kind(&self, other: &Notice) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }
}

/// The USE switch of a MySQL tab under a database other than the profile's.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Switch {
    #[default]
    None,
    /// Sent; Run waits until the session reads the database back.
    Pending,
    Done,
    Failed,
}

/// What the doc tells its host.
#[derive(Debug, Clone)]
pub enum ScriptDocEvent {
    /// The file was read: the host opens the editor over `text`.
    Read(Box<Opened>),
    /// Something the host draws changed (a notice, the Run state).
    Changed,
}

/// A file as the host opens it.
#[derive(Clone)]
pub struct Opened {
    pub text: String,
    pub meta: FileMeta,
    pub stamp: Option<DiskStamp>,
    pub tier: SizeTier,
    /// The whole file's size (the preview's total).
    pub total: u64,
    /// Recovered unsaved text that replaces `text` once the editor is open (dirty).
    pub recovered: Option<RecoveryEntry>,
}

impl std::fmt::Debug for Opened {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Opened")
            .field("text_len", &self.text.len())
            .field("tier", &self.tier)
            .field("recovered", &self.recovered.is_some())
            .finish_non_exhaustive()
    }
}

/// One script tab's file side (beside its console host).
pub struct ScriptDoc {
    pub(crate) id: ScriptTabId,
    pub(crate) tab: TabId,
    pub(crate) load: Load,
    console: Option<ConsoleId>,
    editor: Option<Entity<EditorView>>,
    pub(crate) meta: FileMeta,
    pub(crate) tier: SizeTier,
    /// The file as it was last read, reloaded or saved (`None`: no file yet, or deleted).
    pub(crate) stamp: Option<DiskStamp>,
    pub(crate) clean_version: u64,
    pub(crate) notices: Vec<Notice>,
    /// "Keep mine": the next save asks before it overwrites.
    pub(crate) keep_mine: bool,
    /// Deleted on disk: the next save writes the file again (no `expect`).
    pub(crate) deleted: bool,
    pub(crate) plan: Result<SessionPlan, Refusal>,
    pub(crate) switch: Switch,
    /// The Run being re-run after its one switch back (R-M4b-27).
    switched_once: bool,
    /// ## m4b-t14-tabs fix round 1: the switch back is under way (a second Run press waits).
    switching_back: bool,
    /// The Run being re-run after its database was asked, and the answer (C1, R-T14-2: asked
    /// before every Run, never read from a cache).
    probing: bool,
    checked: Option<Checked>,
    pub(crate) recovered: Option<RecoveryEntry>,
    /// The stamp a running reload read.
    pending_reload: Option<DiskStamp>,
    /// "Save anyway" answers for this tab, by finding fingerprint (R-M4b-17).
    pub(crate) secret_ok: HashSet<u64>,
    /// "Don't save" was answered (close or quit): not asked again.
    pub(crate) discarded: bool,
    /// The one-line status message ("Saved …", "Reloaded …"), for 3 s.
    pub(crate) status: Option<SharedString>,
    status_task: Option<Task<()>>,
    recovery_task: Option<Task<()>>,
    events_task: Option<(Option<EnvTag>, Task<()>)>,
    load_task: Option<Task<()>>,
    switch_task: Option<Task<()>>,
    _subscriptions: Vec<hacivat_ui_core::gpui_surface::Subscription>,
}

impl EventEmitter<ScriptDocEvent> for ScriptDoc {}

/// Every live doc, by tab.
#[derive(Default)]
struct Docs(HashMap<TabId, WeakEntity<ScriptDoc>>);

impl Global for Docs {}

/// The doc of script tab `tab`.
pub fn doc_of_tab(tab: TabId, cx: &App) -> Option<Entity<ScriptDoc>> {
    cx.try_global::<Docs>()?.0.get(&tab)?.upgrade()
}

/// The doc of console `console`, when it is a script tab's.
pub fn doc_of_console(console: ConsoleId, cx: &App) -> Option<Entity<ScriptDoc>> {
    doc_of_tab(Consoles::get(cx, console)?.tab, cx)
}

/// The doc of the open tab with handle `id`.
pub fn doc_of_script(id: ScriptTabId, cx: &App) -> Option<Entity<ScriptDoc>> {
    cx.try_global::<Docs>()?
        .0
        .values()
        .filter_map(WeakEntity::upgrade)
        .find(|d| d.read(cx).id == id)
}

/// Every live doc, by tab order.
pub fn all_docs(cx: &App) -> Vec<Entity<ScriptDoc>> {
    let mut docs: Vec<(TabId, Entity<ScriptDoc>)> = cx
        .try_global::<Docs>()
        .map(|d| {
            d.0.iter()
                .filter_map(|(t, w)| Some((*t, w.upgrade()?)))
                .collect()
        })
        .unwrap_or_default();
    docs.sort_by_key(|(t, _)| t.0);
    docs.into_iter().map(|(_, d)| d).collect()
}

/// Unsaved recovered copies waiting for their tab, by handle (the launch recovery).
#[derive(Default)]
pub(crate) struct PendingRecovered(pub(crate) HashMap<ScriptTabId, RecoveryEntry>);

impl Global for PendingRecovered {}

impl ScriptDoc {
    pub(crate) fn new(id: ScriptTabId, tab: TabId, cx: &mut Context<Self>) -> Self {
        let plan = Self::plan_of(id, cx);
        let observe = cx.observe_global::<ConnectionsModel>(Self::on_connections);
        let mut doc = ScriptDoc {
            id,
            tab,
            load: match &plan {
                Err(r) => Load::Refused(r.clone()),
                Ok(_) => Load::Opening,
            },
            console: None,
            editor: None,
            meta: FileMeta::default(),
            tier: SizeTier::Full,
            stamp: None,
            clean_version: 0,
            notices: Vec::new(),
            keep_mine: false,
            deleted: false,
            plan,
            switch: Switch::None,
            switched_once: false,
            switching_back: false,
            probing: false,
            checked: None,
            recovered: None,
            pending_reload: None,
            secret_ok: HashSet::new(),
            discarded: false,
            status: None,
            status_task: None,
            recovery_task: None,
            events_task: None,
            load_task: None,
            switch_task: None,
            _subscriptions: vec![observe],
        };
        let me = cx.weak_entity();
        cx.default_global::<Docs>().0.insert(tab, me);
        if doc.bound_profile_missing(cx) {
            doc.notices.push(Notice::Removed);
        }
        doc
    }

    /// The session plan of a bound tab, or why it may not run (an unbound tab has none to
    /// check: it never runs).
    fn plan_of(id: ScriptTabId, cx: &App) -> Result<SessionPlan, Refusal> {
        let Some(tab) = spec(id, cx) else {
            return Err(Refusal::NoProfile);
        };
        let Some(profile) = tab.profile else {
            return Ok(SessionPlan::Shared);
        };
        let Some(p) = open::profile_of(profile, cx) else {
            // The connection is gone: the tab shows its text and runs nothing (Removed).
            return Ok(SessionPlan::Shared);
        };
        let Some(db) = tab.database else {
            // R-T1b-1: a bound tab without its database (never written by this build) is
            // never given "the profile's database".
            return Err(Refusal::DatabaseUnknown);
        };
        // ## m4b-t14-tabs fix round 2 (m-fr1-2): a SQLite tab without the file it was opened on
        // (launch recovery without a layout entry) takes the profile's file at its first Run,
        // as a fresh open would; every Run compares it (C2, R-T14-1).
        open::session_plan(&p, &db, cx)
    }

    /// ## m4b-t14-tabs fix round 1 (I6, R-T14-4): the file is no longer listed under the tab's
    /// connection: Run and Save are off (fail closed).
    pub fn unlisted(&self) -> bool {
        self.notices
            .iter()
            .any(|n| matches!(n, Notice::Unlisted(_)))
    }

    pub fn spec(&self, cx: &App) -> Option<ScriptTab> {
        spec(self.id, cx)
    }

    /// The tab's handle.
    pub fn script_id(&self) -> ScriptTabId {
        self.id
    }

    pub fn console(&self) -> Option<ConsoleId> {
        self.console
    }

    /// Where the tab's file stands.
    pub fn load(&self) -> &Load {
        &self.load
    }

    /// The notices under the toolbar, in the order they came.
    pub fn notices(&self) -> &[Notice] {
        &self.notices
    }

    /// The open switch of a MySQL tab under another database.
    pub fn switch_state(&self) -> Switch {
        self.switch
    }

    /// The status bar's message ("Saved …", "Reloaded …"), while it shows.
    pub fn status_text(&self) -> Option<SharedString> {
        self.status.clone()
    }

    /// The editor's tier for the file (a T2 preview is read-only, never saved or run).
    pub fn tier(&self) -> SizeTier {
        self.tier
    }

    /// "Keep mine" on the "Changed on disk" bar: the bar goes, the next save asks before it
    /// overwrites.
    pub fn keep_mine(&mut self, cx: &mut Context<Self>) {
        self.keep_mine = true;
        self.hide(|n| matches!(n, Notice::Changed), cx);
    }

    pub fn editor(&self) -> Option<&Entity<EditorView>> {
        self.editor.as_ref()
    }

    /// The tab's file is a Markdown note (R-M4b-12: it opens, it never runs).
    pub fn is_markdown(&self, cx: &App) -> bool {
        self.spec(cx).is_some_and(|t| {
            hacivat_scripts::scan::script_kind(crate::scripts::file_name(&t.rel))
                == Some(EntryKind::Markdown)
        })
    }

    /// The tab is bound to a profile that no longer exists.
    pub fn bound_profile_missing(&self, cx: &App) -> bool {
        self.spec(cx)
            .and_then(|t| t.profile)
            .is_some_and(|p| open::profile_of(p, cx).is_none())
    }

    /// The tab runs nowhere: unbound, or its connection is gone (R-M4b-30).
    pub fn runs_nowhere(&self, cx: &App) -> bool {
        self.spec(cx).is_none_or(|t| t.profile.is_none()) || self.bound_profile_missing(cx)
    }

    /// The text has edits that are not on disk.
    pub fn is_dirty(&self, cx: &App) -> bool {
        self.editor
            .as_ref()
            .is_some_and(|e| e.read(cx).snapshot().version != self.clean_version)
    }

    /// Whether Run can be pressed (the toolbar; `run_gate` checks again on every Run).
    pub fn run_enabled(&self, cx: &App) -> bool {
        self.load == Load::Ready
            && self.plan.is_ok()
            && !self.unlisted()
            && !self.runs_nowhere(cx)
            && !self.is_markdown(cx)
            && self.tier != SizeTier::Preview
            && !matches!(self.switch, Switch::Pending | Switch::Failed)
    }

    /// The environment file calls take now (R-T5-6).
    pub(crate) fn env(&self, cx: &App) -> Option<EnvTag> {
        if self.runs_nowhere(cx) {
            return None;
        }
        open::env_of(self.spec(cx).and_then(|t| t.profile), cx)
    }

    /// The recovery key of this tab (its binding, R-T5-3).
    pub(crate) fn recovery_key(&self, cx: &App) -> Option<RecoveryKey> {
        let t = self.spec(cx)?;
        Some(RecoveryKey {
            library: t.library,
            rel: t.rel,
            profile: t.profile,
            database: t.database,
        })
    }

    pub(crate) fn show(&mut self, notice: Notice, cx: &mut Context<Self>) {
        self.notices.retain(|n| !n.same_kind(&notice));
        self.notices.push(notice);
        cx.emit(ScriptDocEvent::Changed);
        cx.notify();
    }

    pub(crate) fn hide(&mut self, pred: impl Fn(&Notice) -> bool, cx: &mut Context<Self>) {
        let before = self.notices.len();
        self.notices.retain(|n| !pred(n));
        if self.notices.len() != before {
            cx.emit(ScriptDocEvent::Changed);
            cx.notify();
        }
    }

    /// Shows `text` in the status bar for 3 s (NOTES cross-cutting 2: no motion).
    pub(crate) fn say(&mut self, text: SharedString, cx: &mut Context<Self>) {
        self.status = Some(text);
        let timer = cx.background_executor().timer(Duration::from_secs(3));
        self.status_task = Some(cx.spawn(async move |this, cx| {
            timer.await;
            let _ = this.update(cx, |d, cx| {
                d.status = None;
                cx.emit(ScriptDocEvent::Changed);
                cx.notify();
            });
        }));
        cx.emit(ScriptDocEvent::Changed);
        cx.notify();
    }

    // ---- load ---------------------------------------------------------------------------------

    /// Reads the file off the UI thread (`encoding`: the user's choice after `NeedsEncoding`).
    pub(crate) fn start_load(&mut self, encoding: Option<&'static str>, cx: &mut Context<Self>) {
        if matches!(self.load, Load::Refused(_)) {
            return;
        }
        let Some(tab) = self.spec(cx) else {
            self.load = Load::Failed("other");
            cx.notify();
            return;
        };
        let Some(rt) = open::runtime(cx) else {
            self.load = Load::Failed("library");
            cx.notify();
            return;
        };
        self.load = Load::Opening;
        let env = self.env(cx);
        let service = rt.service.clone();
        let (lib, rel) = (tab.library, tab.rel.clone());
        // ## m4b-t14-tabs fix round 2 (m-fr1-7): only a tab restored at launch (its layout or
        // launch recovery) waits; a tab opened from a listed row reads at once.
        let restored = take_restored(self.id, cx);
        let read = on_core(cx, async move {
            // ## m4b-t14-tabs fix round 1 (I1): a tab restored at launch waits (bounded) until
            // the service has read its registry and listed the library, so a file that exists
            // is never "not in the library" because the first scan was still running.
            if restored {
                let lister = service.clone();
                let _ = tokio::task::spawn_blocking(move || listed(&lister, lib)).await;
            }
            match encoding {
                Some(label) => service.read_with(lib, &rel, env, label).await,
                None => service.read(lib, &rel, env).await,
            }
        });
        self.load_task = Some(cx.spawn(async move |this, cx| {
            let read = read
                .await
                .unwrap_or(Err(ReadError::Io(std::io::ErrorKind::Interrupted)));
            let _ = this.update(cx, |d, cx| d.read_done(read, cx));
        }));
        cx.notify();
    }

    fn read_done(
        &mut self,
        read: Result<hacivat_scripts::Decoded, ReadError>,
        cx: &mut Context<Self>,
    ) {
        let recovered = cx
            .try_global::<PendingRecovered>()
            .is_some_and(|p| p.0.contains_key(&self.id))
            .then(|| cx.default_global::<PendingRecovered>().0.remove(&self.id))
            .flatten();
        match read {
            Ok(decoded) => {
                let total = decoded.stamp.len;
                self.load = Load::Ready;
                cx.emit(ScriptDocEvent::Read(Box::new(Opened {
                    text: decoded.text,
                    meta: decoded.meta,
                    stamp: Some(decoded.stamp),
                    tier: decoded.tier,
                    total,
                    recovered,
                })));
            }
            Err(ReadError::NeedsEncoding { guess }) => {
                if let Some(r) = recovered {
                    cx.default_global::<PendingRecovered>().0.insert(self.id, r);
                }
                self.load = Load::NeedsEncoding { guess };
            }
            Err(e) => {
                let library_gone = self.spec(cx).is_none_or(|t| {
                    open::service(cx).is_none_or(|s| s.snapshot().library(t.library).is_none())
                });
                let reason = match e {
                    ReadError::Io(std::io::ErrorKind::NotFound) if library_gone => "library",
                    ReadError::Io(std::io::ErrorKind::NotFound) => "missing",
                    ReadError::OutOfScope => "scope",
                    ReadError::Binary => "binary",
                    ReadError::PathRefused => "refused",
                    ReadError::Lossy => "lossy",
                    ReadError::Undecodable | ReadError::UnknownEncoding => "encoding",
                    _ => "other",
                };
                match recovered {
                    // The recovered text opens even when the file or library is gone: Save
                    // writes it back to the same place (NOTES surface 12).
                    Some(r) if matches!(reason, "missing" | "library") => {
                        self.load = Load::Ready;
                        self.deleted = true;
                        cx.emit(ScriptDocEvent::Read(Box::new(Opened {
                            text: String::new(),
                            meta: r.copy.meta,
                            stamp: None,
                            tier: SizeTier::Full,
                            total: 0,
                            recovered: Some(r),
                        })));
                    }
                    other => {
                        if let Some(r) = other {
                            cx.default_global::<PendingRecovered>().0.insert(self.id, r);
                        }
                        self.load = Load::Failed(reason);
                    }
                }
            }
        }
        cx.emit(ScriptDocEvent::Changed);
        cx.notify();
    }

    /// The host opened the editor over `opened` as console `console`.
    pub(crate) fn attach(
        &mut self,
        console: ConsoleId,
        editor: Entity<EditorView>,
        opened: Opened,
        cx: &mut Context<Self>,
    ) {
        let Some(tab) = self.spec(cx) else {
            return;
        };
        Consoles::set_source(
            cx,
            console,
            ConsoleSource::Script {
                library: tab.library,
                rel: tab.rel.clone(),
                database: tab.database.clone(),
            },
        );
        self.console = Some(console);
        self.meta = opened.meta;
        self.tier = opened.tier;
        self.stamp = opened.stamp;
        self.clean_version = editor.read(cx).snapshot().version;
        if opened.tier == SizeTier::Preview {
            self.show(Notice::Preview, cx);
        }
        if self.is_markdown(cx) {
            self.show(Notice::Markdown, cx);
        }
        if let Some(entry) = opened.recovered {
            // The recovered text replaces the file's (one undo step back to the disk text).
            editor.update(cx, |e, cx| e.set_text(&entry.text, cx));
            self.show(
                Notice::Recovered {
                    at: entry.saved_at,
                    unknown: entry.liveness_unknown,
                },
                cx,
            );
            self.recovered = Some(entry);
        }
        self.editor = Some(editor);
        self.subscribe(cx);
        if self.is_dirty(cx) {
            self.schedule_recovery(Duration::ZERO, cx);
        }
        cx.emit(ScriptDocEvent::Changed);
        cx.notify();
    }

    // ---- edits and recovery -------------------------------------------------------------------

    /// The editor's text changed.
    pub(crate) fn on_edit(&mut self, cx: &mut Context<Self>) {
        if self.is_dirty(cx) {
            self.discarded = false;
            self.schedule_recovery(RECOVERY_IDLE, cx);
        } else {
            self.recovery_task = None;
            self.discard_copy(cx);
        }
        cx.emit(ScriptDocEvent::Changed);
    }

    /// Copies the unsaved text after `after` of quiet (R-M4b-5).
    fn schedule_recovery(&mut self, after: Duration, cx: &mut Context<Self>) {
        let timer = cx.background_executor().timer(after);
        self.recovery_task = Some(cx.spawn(async move |this, cx| {
            timer.await;
            let _ = this.update(cx, |d, cx| d.store_copy(cx));
        }));
    }

    /// Writes this tab's recovery copy on the core runtime (never into the library).
    fn store_copy(&mut self, cx: &mut Context<Self>) {
        if !self.is_dirty(cx) || self.tier == SizeTier::Preview {
            return;
        }
        let (Some(rt), Some(key), Some(editor)) = (
            open::current(cx),
            self.recovery_key(cx),
            self.editor.clone(),
        ) else {
            return;
        };
        let snap = editor.read(cx).snapshot();
        let copy = CopyMeta {
            meta: self.meta,
            base: self.stamp,
            secret_suspected: false,
            damaged: false,
            // ## m4b-t14-tabs fix round 3 (m-fr2-1): the SQLite file the tab is bound to.
            db_file: sqlite_file(self.id, cx).map(|f| hacivat_scripts::recovery::DbFile {
                path: f.path,
                dev_ino: f.id,
            }),
        };
        let old = self
            .recovered
            .as_ref()
            .filter(|e| !e.liveness_unknown)
            .cloned();
        let task = on_core(cx, async move {
            tokio::task::spawn_blocking(move || {
                let text = snap.buffer.to_string();
                let Some(instance) = rt.instance.get_or_init(|| {
                    recovery::Instance::start(&rt.paths)
                        .inspect_err(|e| tracing::warn!(?e, "scripts: no recovery instance"))
                        .ok()
                }) else {
                    return false;
                };
                let stored = recovery::store(&rt.paths, instance, &key, &text, &copy);
                if let Err(e) = &stored {
                    tracing::warn!(?e, "scripts: the recovery copy was not written");
                }
                // An offered copy of an instance that is gone is replaced by this one.
                if stored.is_ok()
                    && let Some(old) = old
                {
                    let _ = recovery::discard_entry(&rt.paths, &old);
                }
                stored.is_ok()
            })
            .await
            .unwrap_or(false)
        });
        cx.spawn(async move |this, cx| {
            if task.await == Some(true) {
                let _ = this.update(cx, |d, _| {
                    if d.recovered.as_ref().is_some_and(|e| !e.liveness_unknown) {
                        d.recovered = None;
                    }
                });
            }
        })
        .detach();
    }

    /// Removes this tab's own recovery copy (clean again, saved, "Don't save").
    pub(crate) fn discard_copy(&mut self, cx: &mut Context<Self>) {
        let (Some(rt), Some(key)) = (open::current(cx), self.recovery_key(cx)) else {
            return;
        };
        let task = on_core(cx, async move {
            tokio::task::spawn_blocking(move || {
                if let Some(Some(instance)) = rt.instance.get() {
                    let _ = recovery::discard(&rt.paths, instance, &key);
                }
            })
            .await
        });
        cx.spawn(async move |_, _| {
            let _ = task.await;
        })
        .detach();
    }

    /// Removes the offered copy this tab was recovered from (Save, or the explicit Discard).
    pub(crate) fn discard_recovered(&mut self, cx: &mut Context<Self>) {
        let Some(entry) = self.recovered.take() else {
            return;
        };
        let Some(rt) = open::current(cx) else {
            return;
        };
        let task = on_core(cx, async move {
            tokio::task::spawn_blocking(move || recovery::discard_entry(&rt.paths, &entry)).await
        });
        cx.spawn(async move |_, _| {
            let _ = task.await;
        })
        .detach();
        self.hide(|n| matches!(n, Notice::Recovered { .. }), cx);
    }

    /// ## m4b-t14-tabs fix round 1 (I2): unsaved text recovered at launch for this tab, which
    /// may have been read already (it was in the restored layout): the recovered text replaces
    /// the file's (one undo step back to the disk text), the tab is dirty, and the notice says
    /// so. A tab still reading takes it when its file is read.
    pub(crate) fn apply_recovered(&mut self, entry: RecoveryEntry, cx: &mut Context<Self>) {
        match (&self.load, self.editor.clone()) {
            (Load::Ready, Some(editor)) => {
                editor.update(cx, |e, cx| e.set_text(&entry.text, cx));
                self.show(
                    Notice::Recovered {
                        at: entry.saved_at,
                        unknown: entry.liveness_unknown,
                    },
                    cx,
                );
                self.recovered = Some(entry);
                self.discarded = false;
                self.schedule_recovery(Duration::ZERO, cx);
                cx.emit(ScriptDocEvent::Changed);
                cx.notify();
            }
            (Load::Failed("missing" | "library"), _) => {
                self.load = Load::Ready;
                self.deleted = true;
                cx.emit(ScriptDocEvent::Read(Box::new(Opened {
                    text: String::new(),
                    meta: entry.copy.meta,
                    stamp: None,
                    tier: SizeTier::Full,
                    total: 0,
                    recovered: Some(entry),
                })));
                cx.emit(ScriptDocEvent::Changed);
                cx.notify();
            }
            _ => {
                cx.default_global::<PendingRecovered>()
                    .0
                    .insert(self.id, entry);
            }
        }
    }

    /// Removes this tab's own recovery copy for a quit that goes on (I4): on the core runtime's
    /// blocking pool (fix round 2, m-fr1-5); the quit waits for it (`install`'s quit hook).
    pub(crate) fn discard_copy_at_quit(&mut self, cx: &mut Context<Self>) {
        let (Some(rt), Some(key)) = (open::current(cx), self.recovery_key(cx)) else {
            return;
        };
        self.recovery_task = None;
        let task = on_core(cx, async move {
            tokio::task::spawn_blocking(move || {
                if let Some(Some(instance)) = rt.instance.get() {
                    let _ = recovery::discard(&rt.paths, instance, &key);
                }
            })
            .await
            .ok()
        });
        cx.default_global::<QuitDiscards>()
            .0
            .push(Box::pin(async move {
                let _ = task.await;
            }));
    }

    /// The text is clean at `version` with the file at `stamp` (a save or a reload).
    pub(crate) fn clean_at(
        &mut self,
        version: u64,
        stamp: Option<DiskStamp>,
        cx: &mut Context<Self>,
    ) {
        self.clean_version = version;
        self.stamp = stamp;
        self.deleted = stamp.is_none() && self.deleted;
        self.keep_mine = false;
        if !self.is_dirty(cx) {
            self.recovery_task = None;
            self.discard_copy(cx);
        }
        self.hide(
            |n| {
                matches!(
                    n,
                    Notice::Changed | Notice::Deleted | Notice::Recovered { .. }
                )
            },
            cx,
        );
        cx.emit(ScriptDocEvent::Changed);
        cx.notify();
    }

    // ---- the profile ----------------------------------------------------------------------------

    /// The connections changed: a deleted profile shows the Removed notice (and the tab runs
    /// nothing); a new environment re-subscribes and checks the scope again. The binding never
    /// changes.
    fn on_connections(&mut self, cx: &mut Context<Self>) {
        if self.bound_profile_missing(cx) {
            if !self.notices.contains(&Notice::Removed) {
                self.show(Notice::Removed, cx);
            }
            return;
        }
        if self.load != Load::Ready {
            return;
        }
        let env = self.env(cx);
        if self.events_task.as_ref().is_some_and(|(e, _)| *e != env) {
            self.subscribe(cx);
            self.recheck_scope(cx);
        }
    }

    /// Whether the file is still shown under the tab's environment (R-T4-2, R-T10-2).
    fn recheck_scope(&mut self, cx: &mut Context<Self>) {
        let (Some(tab), Some(service)) = (self.spec(cx), open::service(cx)) else {
            return;
        };
        let env = self.env(cx);
        let snap = service.snapshot();
        let Some(lib) = snap.view(env).library(tab.library) else {
            return;
        };
        if lib.entry(&tab.rel).is_none() {
            let folder = tab
                .rel
                .components()
                .find(|c| c.starts_with('@'))
                .map(display_name)
                .unwrap_or_default();
            self.show(Notice::Unlisted(Unlisted::Env { folder }), cx);
        } else {
            self.hide(|n| matches!(n, Notice::Unlisted(_)), cx);
        }
    }

    // ---- events from disk -----------------------------------------------------------------------

    /// Listens to the service for the tab's environment (R-T10-2).
    fn subscribe(&mut self, cx: &mut Context<Self>) {
        let Some(service) = open::service(cx) else {
            return;
        };
        let env = self.env(cx);
        let mut sub = service.subscribe(env);
        let task = cx.spawn(async move |this, cx| {
            loop {
                match sub.recv().await {
                    Ok(event) => {
                        if this.update(cx, |d, cx| d.on_event(event, cx)).is_err() {
                            break;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        });
        self.events_task = Some((env, task));
    }

    fn on_event(&mut self, event: LibraryEvent, cx: &mut Context<Self>) {
        let Some(tab) = self.spec(cx) else {
            return;
        };
        match event {
            LibraryEvent::FileChanged { id, rel, stamp } if id == tab.library && rel == tab.rel => {
                self.changed_on_disk(stamp, cx);
            }
            LibraryEvent::FileDeleted { id, rel } if id == tab.library && rel == tab.rel => {
                self.deleted = true;
                self.show(Notice::Deleted, cx);
            }
            LibraryEvent::FileRenamed { id, from, to } if id == tab.library && from == tab.rel => {
                self.renamed(to, cx);
            }
            LibraryEvent::FileLeftScope { id, rel } if id == tab.library && rel == tab.rel => {
                self.show(Notice::Unlisted(Unlisted::Moved), cx);
            }
            _ => {}
        }
    }

    /// The file changed on disk (not this tab's own write).
    pub(crate) fn changed_on_disk(&mut self, stamp: DiskStamp, cx: &mut Context<Self>) {
        if self.stamp == Some(stamp) || self.pending_reload == Some(stamp) {
            return;
        }
        self.deleted = false;
        self.hide(|n| matches!(n, Notice::Deleted), cx);
        if self.is_dirty(cx) || self.keep_mine {
            // A dirty tab is never reloaded (R-M4b-16).
            self.show(Notice::Changed, cx);
            return;
        }
        self.reload(cx);
    }

    /// Reads the file again and replaces a clean tab's text (R-T11-1); a dirty tab gets the
    /// bar instead.
    pub(crate) fn reload(&mut self, cx: &mut Context<Self>) {
        let (Some(tab), Some(service)) = (self.spec(cx), open::service(cx)) else {
            return;
        };
        let env = self.env(cx);
        let label = (self.meta.encoding != TextEncoding::UTF_8 && !self.meta.bom)
            .then_some(self.meta.encoding.name());
        let (lib, rel) = (tab.library, tab.rel.clone());
        let read = on_core(cx, async move {
            match label {
                Some(label) => service.read_with(lib, &rel, env, label).await,
                None => service.read(lib, &rel, env).await,
            }
        });
        cx.spawn(async move |this, cx| {
            let read = read.await;
            let _ = this.update(cx, |d, cx| match read {
                Some(Ok(decoded)) => d.apply_reload(decoded, cx),
                Some(Err(ReadError::Io(std::io::ErrorKind::NotFound))) => {
                    d.deleted = true;
                    d.show(Notice::Deleted, cx);
                }
                Some(Err(ReadError::OutOfScope)) => {
                    d.show(Notice::Unlisted(Unlisted::Moved), cx);
                }
                _ => d.show(Notice::Changed, cx),
            });
        })
        .detach();
    }

    fn apply_reload(&mut self, decoded: hacivat_scripts::Decoded, cx: &mut Context<Self>) {
        let Some(editor) = self.editor.clone() else {
            return;
        };
        if self.tier == SizeTier::Preview || decoded.tier == SizeTier::Preview {
            if self.tier == SizeTier::Preview && decoded.tier == SizeTier::Preview {
                let spec = hacivat_ui_editor::PreviewSpec {
                    shown_bytes: decoded.text.len() as u64,
                    total_bytes: decoded.stamp.len,
                };
                let done = editor.update(cx, |e, cx| e.reload_preview(&decoded.text, spec, cx));
                if done.is_ok() {
                    let v = editor.read(cx).snapshot().version;
                    self.clean_at(v, Some(decoded.stamp), cx);
                    return;
                }
            }
            self.show(Notice::Changed, cx);
            return;
        }
        let clean = self.clean_version;
        let accepted = editor.update(cx, |e, cx| e.replace_from_disk(decoded.text, clean, cx));
        match accepted {
            Ok(()) => {
                // The editor says when it is done (`Reloaded`, through the host).
                self.pending_reload = Some(decoded.stamp);
                self.meta = decoded.meta;
            }
            Err(_) => self.show(Notice::Changed, cx),
        }
    }

    /// The editor finished a `replace_from_disk`: `Some(version)` is the new clean version.
    pub(crate) fn on_reloaded(&mut self, version: Option<u64>, cx: &mut Context<Self>) {
        let Some(stamp) = self.pending_reload.take() else {
            return;
        };
        match version {
            Some(v) => {
                self.clean_at(v, Some(stamp), cx);
                if let Some(t) = self.spec(cx) {
                    let file = display_name(crate::scripts::file_name(&t.rel));
                    let text = msg(
                        cx,
                        "hv-scripts-disk-reloaded",
                        vec![("file", ArgValue::Text(file))],
                    );
                    self.say(text, cx);
                }
            }
            // An edit came first: the bar, never the reload (R-M4b-16).
            None => self.show(Notice::Changed, cx),
        }
    }

    /// A rename the service paired: the tab follows the path (never the connection), once the
    /// new path is checked to be shown under the tab's environment (carries-w1 T14).
    fn renamed(&mut self, to: RelPath, cx: &mut Context<Self>) {
        let (Some(tab), Some(service)) = (self.spec(cx), open::service(cx)) else {
            return;
        };
        let env = self.env(cx);
        let shown = service
            .snapshot()
            .view(env)
            .library(tab.library)
            .and_then(|l| l.entry(&to))
            .is_some();
        if !shown {
            self.show(Notice::Unlisted(Unlisted::Moved), cx);
            return;
        }
        // ## m4b-t14-tabs fix round 1 (minor 3): the copy kept under the old path goes; the
        // next quiet spell writes one under the new path.
        self.discard_copy(cx);
        follow_rename(self.id, to.clone(), cx);
        if self.is_dirty(cx) {
            self.schedule_recovery(RECOVERY_IDLE, cx);
        }
        if let Some(console) = self.console {
            Consoles::set_source(
                cx,
                console,
                ConsoleSource::Script {
                    library: tab.library,
                    rel: to.clone(),
                    database: tab.database.clone(),
                },
            );
        }
        self.show(
            Notice::Moved {
                path: to.display_text(),
            },
            cx,
        );
        let text = msg(
            cx,
            "hv-scripts-disk-renamed",
            vec![
                (
                    "file",
                    ArgValue::Text(display_name(crate::scripts::file_name(&tab.rel))),
                ),
                (
                    "new",
                    ArgValue::Text(display_name(crate::scripts::file_name(&to))),
                ),
            ],
        );
        self.say(text, cx);
    }

    // ---- the session -----------------------------------------------------------------------------

    /// The host took `session` for the tab: a MySQL tab under another database switches it
    /// with `USE` before Run is enabled (R-M4b-25).
    pub(crate) fn session_taken(&mut self, session: SessionHandle, cx: &mut Context<Self>) {
        if self.plan != Ok(SessionPlan::Own { switch: true })
            || matches!(self.switch, Switch::Done | Switch::Pending)
        {
            return;
        }
        let Some(db) = self.spec(cx).and_then(|t| t.database) else {
            return;
        };
        self.switch = Switch::Pending;
        let ticket = session.submit(Request::SwitchSchema(db.as_str().to_string()));
        self.switch_task = Some(cx.spawn(async move |this, cx| {
            let replied = ticket.reply.await;
            let _ = this.update(cx, |d, cx| {
                let landed = replied.is_ok_and(|r| r.is_ok())
                    && session_database(&session).as_deref() == Some(db.as_str());
                d.switch = if landed { Switch::Done } else { Switch::Failed };
                if landed {
                    d.hide(|n| matches!(n, Notice::SwitchFailed { .. }), cx);
                } else {
                    d.show(
                        Notice::SwitchFailed {
                            database: db.as_str().to_string(),
                        },
                        cx,
                    );
                }
                cx.emit(ScriptDocEvent::Changed);
                cx.notify();
            });
        }));
        cx.emit(ScriptDocEvent::Changed);
        cx.notify();
    }

    /// The tab closed: the service forgets the file when no other tab has it open (I4).
    pub(crate) fn closed(&mut self, cx: &mut App) {
        let Some(tab) = self.spec(cx) else {
            return;
        };
        let me = self.tab;
        if let Some(docs) = cx.try_global::<Docs>() {
            let others = docs
                .0
                .iter()
                .filter(|(t, _)| **t != me)
                .filter_map(|(_, w)| w.upgrade())
                .any(|d| {
                    d.read(cx)
                        .spec(cx)
                        .is_some_and(|s| s.library == tab.library && s.rel == tab.rel)
                });
            if !others && let Some(service) = open::service(cx) {
                // ## m4b-t14-tabs fix round 1 (minor 13): queued now, before any later
                // reopen's read can remember the file again.
                service.forget_now(tab.library, &tab.rel);
            }
        }
        cx.default_global::<Docs>().0.remove(&me);
        // ## m4b-t14-tabs fix round 1 (minor 14): the handle goes with its tab.
        unintern(self.id, cx);
    }
}

/// Tab `saver` saved its file at `stamp`: every other tab of the file (under another
/// connection, R-M4b-4) sees an external change (reloaded if clean, the bar if dirty). The
/// service suppresses the save's own echo, so they are told here.
pub(crate) fn saved_elsewhere(saver: &Entity<ScriptDoc>, stamp: DiskStamp, cx: &mut App) {
    let Some(tab) = saver.read(cx).spec(cx) else {
        return;
    };
    let others: Vec<Entity<ScriptDoc>> = all_docs(cx)
        .into_iter()
        .filter(|d| d.entity_id() != saver.entity_id())
        .filter(|d| {
            d.read(cx)
                .spec(cx)
                .is_some_and(|s| s.library == tab.library && s.rel == tab.rel)
        })
        .collect();
    for doc in others {
        doc.update(cx, |d, cx| d.changed_on_disk(stamp, cx));
    }
}

// ## m4b-t14-tabs fix round 1 ---------------------------------------------------------------------

/// What one Run's check found on the tab's own session (C1, R-T14-2), with the session's
/// context generation read right after it on the same queue: the run is sent only under that
/// generation (`generation_holds`).
#[derive(Debug, Clone, PartialEq, Eq)]
struct Checked {
    session: hacivat_session::session::SessionId,
    generation: Option<u64>,
    /// The database the server says the session is on (`None`: it could not say).
    database: Option<String>,
    /// SQLite: the files the check found.
    file: Option<FileCheck>,
}

/// What a SQLite tab's Run check found (C2, R-T14-1; fix round 2, m-fr1-1).
#[derive(Debug, Clone, PartialEq, Eq)]
struct FileCheck {
    /// The profile's database file now.
    profile: Option<SqliteFile>,
    /// The file the tab's session has open (`PRAGMA database_list`, `main`).
    session: Option<SqliteFile>,
}

/// The `main` database's file in a `PRAGMA database_list` answer (`seq`, `name`, `file`).
fn main_file(answer: &hacivat_driver::ExecSummary) -> Option<String> {
    answer
        .rows
        .iter()
        .find(|r| r.get(1).and_then(Option::as_deref) == Some("main"))
        .and_then(|r| r.get(2).cloned().flatten())
        .filter(|p| !p.is_empty())
}

/// Whether `now` is `stored` after a remount: the same path and inode, another device.
fn remounted(stored: &SqliteFile, now: &SqliteFile) -> bool {
    stored.path == now.path
        && stored
            .id
            .zip(now.id)
            .is_some_and(|((d1, i1), (d2, i2))| i1 == i2 && d1 != d2)
}

/// The context generation each script console's last passed check was read under; the
/// orchestrator sends a run only under it.
#[derive(Default)]
struct RunChecks(HashMap<ConsoleId, (hacivat_session::session::SessionId, Option<u64>)>);

impl Global for RunChecks {}

/// Whether a run of console `console` classified under the session's context generation
/// `generation` may be sent (C1, R-T14-2): always for a console of its own; for a script tab
/// only under the generation its last check read (taken: one check, one run). Anything that
/// moved the context in between (a reconnect, a replay, a queued `USE`) refuses, and the next
/// Run checks again. The check must also have been made on `session`, the session the run goes
/// to (fix round 2, m-fr1-4).
pub(crate) fn generation_holds(
    console: ConsoleId,
    session: hacivat_session::session::SessionId,
    generation: Option<u64>,
    cx: &mut App,
) -> bool {
    if !is_script_console(console, cx) {
        return true;
    }
    let checked = cx.default_global::<RunChecks>().0.remove(&console);
    checked.is_some_and(|(on, read)| on == session && read.is_some() && read == generation)
}

/// The SQLite file a SQLite script tab was opened on (C2, R-T14-1): its canonical path and,
/// on Unix, its device and inode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SqliteFile {
    pub path: String,
    pub id: Option<(u64, u64)>,
}

impl SqliteFile {
    /// `"dev:ino"` (`workspace.json`).
    pub fn id_text(&self) -> Option<String> {
        self.id.map(|(d, i)| format!("{d}:{i}"))
    }

    /// The file from what `workspace.json` holds.
    pub fn parse(path: &str, id: Option<&str>) -> Self {
        let id = id.and_then(|t| {
            let (d, i) = t.split_once(':')?;
            Some((d.parse().ok()?, i.parse().ok()?))
        });
        SqliteFile {
            path: path.to_string(),
            id,
        }
    }
}

/// The file each SQLite script handle was opened on.
#[derive(Default)]
struct SqliteFiles(HashMap<u64, SqliteFile>);

impl Global for SqliteFiles {}

/// The SQLite file script handle `id` was opened on.
pub fn sqlite_file(id: ScriptTabId, cx: &App) -> Option<SqliteFile> {
    cx.try_global::<SqliteFiles>()?.0.get(&id.0).cloned()
}

/// Records the SQLite file handle `id` was opened on (the open, a restore).
pub(crate) fn set_sqlite_file(id: ScriptTabId, file: SqliteFile, cx: &mut App) {
    cx.default_global::<SqliteFiles>().0.insert(id.0, file);
}

/// Forgets the SQLite file handle `id` was opened on, as a tab opened by launch recovery
/// without a layout entry has none (tests; fix round 2, m-fr1-2).
#[cfg(feature = "test-support")]
pub fn forget_sqlite_file_for_tests(id: ScriptTabId, cx: &mut App) {
    if cx.has_global::<SqliteFiles>() {
        cx.default_global::<SqliteFiles>().0.remove(&id.0);
    }
}

/// The canonical path and identity of the file at `path` now. Blocking: call it on the core
/// runtime's blocking pool.
pub fn sqlite_identity(path: &std::path::Path) -> Option<SqliteFile> {
    let canonical = std::fs::canonicalize(path).ok()?;
    let meta = std::fs::metadata(&canonical).ok()?;
    #[cfg(unix)]
    let id = {
        use std::os::unix::fs::MetadataExt;
        Some((meta.dev(), meta.ino()))
    };
    #[cfg(not(unix))]
    let id = {
        let _ = &meta;
        None
    };
    Some(SqliteFile {
        path: canonical.to_string_lossy().into_owned(),
        id,
    })
}

/// The database file of a SQLite profile.
pub(crate) fn sqlite_path(p: &hacivat_session::profile::Profile) -> Option<std::path::PathBuf> {
    match &p.endpoint {
        hacivat_session::profile::ProfileEndpoint::File { path, .. } => Some(path.clone()),
        _ => None,
    }
}
// ## m4b-t14-tabs fix round 1

/// Whether `service` has read its registry and listed library `lib` (its first scan done),
/// waiting at most 10 s. Blocking: the core runtime's blocking pool.
fn listed(service: &hacivat_scripts::service::LibraryService, lib: LibraryId) -> bool {
    use hacivat_scripts::LibraryState;
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        let snap = service.snapshot();
        if snap.state != LibraryState::Scanning {
            match snap.library(lib) {
                None => return false,
                Some(l) if l.state != LibraryState::Scanning => return true,
                Some(_) => {}
            }
        }
        if std::time::Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

// ## m4b-t14-tabs fix round 2 (m-fr1-7)
/// The handles restored at launch whose first read has not started.
#[derive(Default)]
struct RestoredAtLaunch(std::collections::HashSet<u64>);

impl Global for RestoredAtLaunch {}

/// Marks handle `id` as restored at launch (the layout, launch recovery): its first read waits
/// for the library's first listing, and its first Run may follow a remounted SQLite file
/// (fix round 3, R-T14-5).
pub(crate) fn mark_restored(id: ScriptTabId, cx: &mut App) {
    cx.default_global::<RestoredAtLaunch>().0.insert(id.0);
    cx.default_global::<FirstRunAfterRestore>().0.insert(id.0);
}

// ## m4b-t14-tabs fix round 3 (m-fr2-2, R-T14-5)
/// The handles restored at launch that have not been run since.
#[derive(Default)]
struct FirstRunAfterRestore(std::collections::HashSet<u64>);

impl Global for FirstRunAfterRestore {}

/// Whether this is handle `id`'s first Run check since it was restored at launch (taken).
fn take_first_run(id: ScriptTabId, cx: &mut App) -> bool {
    cx.has_global::<FirstRunAfterRestore>()
        && cx.default_global::<FirstRunAfterRestore>().0.remove(&id.0)
}

/// Marks handle `id` as restored at launch (tests).
#[cfg(feature = "test-support")]
pub fn mark_restored_for_tests(id: ScriptTabId, cx: &mut App) {
    mark_restored(id, cx);
}

/// Records `file` as the SQLite file handle `id` was opened on (tests: a remount).
#[cfg(feature = "test-support")]
pub fn set_sqlite_file_for_tests(id: ScriptTabId, file: SqliteFile, cx: &mut App) {
    set_sqlite_file(id, file, cx);
}
// ## m4b-t14-tabs fix round 3

/// Whether handle `id` was restored at launch and has not read yet (taken).
fn take_restored(id: ScriptTabId, cx: &mut App) -> bool {
    cx.has_global::<RestoredAtLaunch>() && cx.default_global::<RestoredAtLaunch>().0.remove(&id.0)
}
// ## m4b-t14-tabs fix round 2

/// Forgets handle `id` (its tab closed): the same binding opened again gets a new handle.
fn unintern(id: ScriptTabId, cx: &mut App) {
    if let Some(tabs) = cx.try_global::<ScriptTabs>()
        && tabs.by_id.contains_key(&id.0)
    {
        cx.default_global::<ScriptTabs>().by_id.remove(&id.0);
    }
    if cx.has_global::<SqliteFiles>() {
        cx.default_global::<SqliteFiles>().0.remove(&id.0);
    }
}

/// How long a dirty tab is quiet before its text is copied (R-M4b-5).
pub const RECOVERY_IDLE: Duration = Duration::from_secs(5);

/// The database `session` is on now, as Run compares it (R-M4b-27): PostgreSQL and MySQL
/// report it, SQLite's is `main` (its first schema).
pub(crate) fn session_database(session: &SessionHandle) -> Option<String> {
    let state = session.state();
    let snap = state.borrow();
    let schema = snap.schema.as_ref()?;
    schema
        .database
        .clone()
        .or_else(|| schema.schema.clone().filter(|s| s == "main"))
        .filter(|d| !d.is_empty())
}

// ---- Run (R-M4b-27) -------------------------------------------------------------------------------

/// The script tab's check before every Run on console `console` (the orchestrator's single
/// submit point): `true` lets the run go on. A refused Run shows its notice and sends nothing;
/// a MySQL tab whose session is on another database switches it back once and runs `rerun`
/// after the switch, which checks again.
pub(crate) fn run_gate(console: ConsoleId, rerun: Box<dyn FnOnce(&mut App)>, cx: &mut App) -> bool {
    let Some(doc) = doc_of_console(console, cx) else {
        return !is_script_console(console, cx);
    };
    doc.update(cx, |d, cx| d.gate(console, rerun, false, cx))
}

// ## m4b-t18-run-as-script (named exception; R-M4b-27, R-T14-2)
/// [`run_gate`] for Run as script (`scripts::run_as_script`, between its two passes): the same
/// check on the tab's own session, except that a T2 preview is what it streams. A pass inserts
/// the check's context generation, which [`take_checked_generation`] hands to the runner.
pub(crate) fn run_gate_streamed(
    console: ConsoleId,
    rerun: Box<dyn FnOnce(&mut App)>,
    cx: &mut App,
) -> bool {
    let Some(doc) = doc_of_console(console, cx) else {
        return !is_script_console(console, cx);
    };
    doc.update(cx, |d, cx| d.gate(console, rerun, true, cx))
}

/// The context generation console `console`'s last passed check read on `session` (taken:
/// one check, one run); `None` when no check passed there, or its generation is unknown.
pub(crate) fn take_checked_generation(
    console: ConsoleId,
    session: hacivat_session::session::SessionId,
    cx: &mut App,
) -> Option<u64> {
    let (on, generation) = cx.default_global::<RunChecks>().0.remove(&console)?;
    (on == session).then_some(generation).flatten()
}
// ## m4b-t18-run-as-script

/// Whether `console` is a script tab's (registered with its source), whatever its doc.
fn is_script_console(console: ConsoleId, cx: &App) -> bool {
    matches!(Consoles::source(cx, console), ConsoleSource::Script { .. })
}

impl ScriptDoc {
    fn gate(
        &mut self,
        console: ConsoleId,
        rerun: Box<dyn FnOnce(&mut App)>,
        // ## m4b-t18-run-as-script (named exception): Run as script streams a T2 preview.
        streamed: bool,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.load != Load::Ready || self.plan.is_err() {
            return false;
        }
        if self.runs_nowhere(cx) {
            let text = msg(cx, "hv-scripts-run-refused-unbound", Vec::new());
            toast(text, cx);
            return false;
        }
        if self.is_markdown(cx) {
            toast(msg(cx, "hv-scripts-run-md", Vec::new()), cx);
            return false;
        }
        if self.tier == SizeTier::Preview && !streamed {
            toast(msg(cx, "hv-scripts-preview-no-run", Vec::new()), cx);
            return false;
        }
        // ## m4b-t14-tabs fix round 1 (I6, R-T14-4): off once the file left the connection.
        if self.unlisted() {
            toast(msg(cx, "hv-scripts-unlisted-off", Vec::new()), cx);
            return false;
        }
        // A check or a switch back is under way: this press waits for it (minor 10).
        if matches!(self.switch, Switch::Pending | Switch::Failed)
            || self.probing
            || self.switching_back
        {
            return false;
        }
        let Some(session) = Consoles::get(cx, console).and_then(|s| s.session.clone()) else {
            // Not connected: the orchestrator connects first and runs nothing.
            return true;
        };
        let (Some(bound), Some(profile)) = (
            self.spec(cx).and_then(|t| t.database),
            self.spec(cx)
                .and_then(|t| t.profile)
                .and_then(|p| open::profile_of(p, cx)),
        ) else {
            return false;
        };
        let bound = bound.as_str().to_string();
        let engine = profile.engine;
        let file_level =
            hacivat_meta::kind::database_level(engine) == hacivat_meta::kind::DbLevel::File;
        // ## m4b-t14-tabs fix round 1 (C1, R-T14-2; C2, R-T14-1): every Run asks the server, on
        // this tab's own session, which database it is on (a SQLite tab: which file it has
        // open, and whether the profile's file is still the tab's), never a cached value. The
        // session's context generation is read after the probe is queued (fix round 2,
        // m-fr1-3), and a move between the two bumps it, which the orchestrator refuses. This
        // Run then runs again with the answer.
        let checked = self.checked.take().filter(|c| c.session == session.id());
        let Some(checked) = checked else {
            self.probing = true;
            let sid = session.id();
            let ticket = session.submit(Request::Exec {
                sql: hacivat_session::session::schema_probe_sql(engine).to_string(),
                opts: hacivat_driver::ExecOpts::values(16),
                origin: hacivat_session::session::ExecOrigin::Internal { idempotent: true },
            });
            let generation = session.run_context();
            let found: std::pin::Pin<Box<dyn Future<Output = Checked>>> = if file_level {
                let path = sqlite_path(&profile);
                let files = on_core(cx, async move {
                    let open = ticket
                        .reply
                        .await
                        .ok()
                        .and_then(Result::ok)
                        .and_then(|answer| main_file(&answer));
                    tokio::task::spawn_blocking(move || FileCheck {
                        profile: path.as_deref().and_then(sqlite_identity),
                        session: open.and_then(|p| sqlite_identity(std::path::Path::new(&p))),
                    })
                    .await
                    .ok()
                });
                Box::pin(async move {
                    let files = files.await.flatten().unwrap_or(FileCheck {
                        profile: None,
                        session: None,
                    });
                    let (_, generation) = generation.await;
                    Checked {
                        session: sid,
                        generation,
                        database: None,
                        file: Some(files),
                    }
                })
            } else {
                Box::pin(async move {
                    let database = ticket
                        .reply
                        .await
                        .ok()
                        .and_then(Result::ok)
                        .map(|s| hacivat_session::session::parse_schema_probe(engine, &s))
                        .and_then(|ctx| ctx.database)
                        .filter(|d| !d.is_empty());
                    let (_, generation) = generation.await;
                    Checked {
                        session: sid,
                        generation,
                        database,
                        file: None,
                    }
                })
            };
            cx.spawn(async move |this, cx| {
                let checked = found.await;
                let _ = this.update(cx, |d, _| {
                    d.probing = false;
                    d.checked = Some(checked);
                });
                cx.update(rerun);
                let _ = this.update(cx, |d, _| d.checked = None);
            })
            .detach();
            return false;
        };
        let pass = |d: &mut Self, cx: &mut Context<Self>| {
            d.switched_once = false;
            d.hide(|n| matches!(n, Notice::RunRefused(_)), cx);
            cx.default_global::<RunChecks>()
                .0
                .insert(console, (checked.session, checked.generation));
            true
        };
        if let Some(found) = &checked.file {
            // ## m4b-t14-tabs fix round 2 (m-fr1-2): a tab without its file takes the profile's
            // file now (as a fresh open would); after a remount (same path and inode, another
            // device number) the tab follows its file.
            //
            // ## m4b-t14-tabs fix round 3 (m-fr2-2, R-T14-5): a remount is followed only on the
            // first Run after a launch restore, and said; mid-session a device change refuses
            // (FAT/exFAT inodes are synthesized, so another stick can match; ADR 0009).
            let stored = sqlite_file(self.id, cx);
            let first_after_restore = take_first_run(self.id, cx);
            let adopted = match (&stored, &found.profile) {
                (None, Some(now)) => Some(now.clone()),
                (Some(was), Some(now)) if first_after_restore && remounted(was, now) => {
                    let path = display_name(&now.path);
                    toast(
                        msg(
                            cx,
                            "hv-scripts-sqlite-remounted",
                            vec![("path", ArgValue::Text(path))],
                        ),
                        cx,
                    );
                    Some(now.clone())
                }
                _ => None,
            };
            if let Some(file) = adopted.clone() {
                set_sqlite_file(self.id, file, cx);
            }
            let stored = adopted.or(stored);
            // The profile's file is the tab's, and it is the file the session has open
            // (m-fr1-1).
            if stored.is_some() && stored == found.profile && found.session == found.profile {
                return pass(self, cx);
            }
            let name = |f: Option<&SqliteFile>| {
                f.and_then(|f| {
                    std::path::Path::new(&f.path)
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                })
                .unwrap_or_default()
            };
            let current = if stored != found.profile {
                found.profile.as_ref()
            } else {
                found.session.as_ref()
            };
            tracing::info!("scripts: Run refused: the SQLite file is not the tab's");
            self.show(
                Notice::RunRefused(RunRefused::Edited {
                    bound: name(stored.as_ref()),
                    current: name(current),
                }),
                cx,
            );
            return false;
        }
        let current = checked.database.clone();
        if current.as_deref() == Some(bound.as_str()) {
            return pass(self, cx);
        }
        let refused = match (&self.plan, current) {
            (Ok(SessionPlan::Own { .. }), Some(_)) if !self.switched_once => {
                // One switch back, then the same Run again (which checks again).
                self.switched_once = true;
                self.switching_back = true;
                let ticket = session.submit(Request::SwitchSchema(bound.clone()));
                cx.spawn(async move |this, cx| {
                    let _ = ticket.reply.await;
                    // The re-run checks again (asynchronously): `switched_once` stays set until
                    // that check passes or refuses.
                    let _ = this.update(cx, |d, _| d.switching_back = false);
                    cx.update(rerun);
                })
                .detach();
                return false;
            }
            (Ok(SessionPlan::Own { .. }), Some(current)) => RunRefused::Mysql { bound, current },
            (Ok(SessionPlan::Shared), Some(current)) => RunRefused::Edited { bound, current },
            (_, None) => RunRefused::Unknown { bound },
            (Err(_), _) => return false,
        };
        tracing::info!("scripts: Run refused: the session is not on the tab's database");
        self.switched_once = false;
        self.show(Notice::RunRefused(refused), cx);
        false
    }
}

/// "Run refused: this tab has no connection." (a Run of a tab that runs nowhere).
pub(crate) fn say_runs_nowhere(cx: &mut App) {
    let text = msg(cx, "hv-scripts-run-refused-unbound", Vec::new());
    toast(text, cx);
}

/// An info toast.
fn toast(text: SharedString, cx: &mut App) {
    hacivat_ui::toast::ToastQueue::push(
        cx,
        hacivat_ui::toast::Toast::new(
            hacivat_ui::toast::ToastKind::Info,
            Rc::new(move |_: &App| text.clone()),
        ),
    );
}

// ---- what the shell reads ----------------------------------------------------------------------------

/// The decoration of script tab `tab` (the `tab_decor` hook): its title, its connection's
/// environment mark and the production hairline, from the profile as it is now.
pub fn tab_decor(tab: TabId, script: ScriptTabId, cx: &App) -> TabDecor {
    let _ = tab;
    let profile = spec(script, cx)
        .and_then(|t| t.profile)
        .and_then(|p| open::profile_of(p, cx));
    let env = profile.as_ref().and_then(|p| p.environment);
    TabDecor {
        title: Some(tab_title(script, cx)),
        title_source: hacivat_ui::text::TextSource::Data,
        env: env.map(|tag| crate::connections::capsule::env_kind(Some(tag))),
        hairline: env == Some(EnvTag::Production),
    }
}

/// Whether script tab `tab` has unsaved edits (the tab strip's dirty dot).
pub fn tab_dirty(tab: TabId, cx: &App) -> bool {
    doc_of_tab(tab, cx).is_some_and(|d| d.read(cx).is_dirty(cx))
}

/// Whether console `console` is a script tab that runs nowhere (the capsule shows no
/// connection for it, R-M4b-30).
pub fn runs_nowhere(console: ConsoleId, cx: &App) -> bool {
    match doc_of_console(console, cx) {
        Some(doc) => doc.read(cx).runs_nowhere(cx),
        None => false,
    }
}

/// The status bar's one-line message of the doc of `tab` ("Saved …", "Reloaded …").
pub fn status_message(tab: TabId, cx: &App) -> Option<SharedString> {
    doc_of_tab(tab, cx)?.read(cx).status.clone()
}

/// The tab's file name as shown.
pub(crate) fn file_label(doc: &ScriptDoc, cx: &App) -> String {
    doc.spec(cx)
        .map(|t| display_name(crate::scripts::file_name(&t.rel)))
        .unwrap_or_default()
}

/// The tab's connection name as shown (empty when unbound).
pub(crate) fn conn_label(doc: &ScriptDoc, cx: &App) -> String {
    doc.spec(cx)
        .and_then(|t| t.profile)
        .and_then(|p| open::profile_of(p, cx))
        .map(|p| display_name(&p.name))
        .unwrap_or_default()
}

/// Shots fixtures (feature `shots`): a notice on a script tab without the disk or session
/// event behind it (NOTES surfaces 5, 6, 12).
#[cfg(any(feature = "shots", feature = "test-support"))]
pub mod fixtures {
    use super::*;

    /// Shows `notice` on script tab `tab`.
    pub fn show(tab: TabId, notice: Notice, cx: &mut App) {
        if let Some(doc) = doc_of_tab(tab, cx) {
            doc.update(cx, |d, cx| d.show(notice, cx));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hidden_characters_show_as_code_points() {
        assert_eq!(display_name("a\u{202E}lqs.sql"), "a<U+202E>lqs.sql");
        assert_eq!(display_name("a\u{0}b\u{7}"), "a<U+0000>b<U+0007>");
        assert_eq!(display_name("x\u{200B}\u{2066}y"), "x<U+200B><U+2066>y");
        assert_eq!(display_name("müşteri_raporu.sql"), "müşteri_raporu.sql");
    }

    #[test]
    fn script_client_numbers_never_meet_console_numbers() {
        assert!(client_number(ScriptTabId(1)) > 1_000_000);
        assert_ne!(client_number(ScriptTabId(1)), client_number(ScriptTabId(2)));
        assert_eq!(client_number(ScriptTabId(u64::MAX)), u32::MAX);
    }
}
