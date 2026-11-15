//! The console view (Task 32): the `console_view` hook's body, the focused console and the
//! editor status segment.
//!
//! **Binding (R-M3-37).** An unbound console renders exactly today's M2 home (`editor_empty`).
//! The first successful connect binds the focused unbound console (the focused pane's active
//! tab); `workspace.new_console_tab` (and a split) binds the new console to the capsule's
//! connected profile (`console_binding`); a restored console keeps its saved profile and
//! reopens its autosaved text. A bound console is its profile's `slot`-th console (the smallest
//! free number at binding): the first uses the profile's default session (`ClientId::DEFAULT`,
//! B4), the others open their own (`SessionManager::open`, `Console(ClientId(n))`), read-only
//! whenever `effective_read_only` says so. While that open waits for the budget the toolbar
//! says "Waiting for a free connection … (in use of max in use)" with Cancel (R-M3-43).
//!
//! **Read-only is never weaker than the profile (R-M3-6, R-M3-29).** A session that is
//! read-write where `effective_read_only` now says read-only (the profile was tagged production
//! or marked read-only while connected) is dropped from every console of the profile at once,
//! and the profile reconnects, so its new sessions open read-only. The console never takes such
//! a session either (`acquire`).
//!
//! **View.** The editor (`EditorView` with the `SqlLanguageService`) is embedded through
//! `a11y::embed` (cached while no assistive technology is attached), under the console toolbar;
//! `exec_ui::attach` is called once, when the editor is made. The console's SQL context
//! (`set_sql_context`, the dialect's defaults until Task 33's probe) is set before the editor
//! hears it is connected (`set_doc_context`), on every bind and reconnect; the tier follows
//! `TierChanged`. The host is notified only by its own state (session, waiting, the run
//! clock), never by typing.
//!
//! A console registered for its tab by someone else (`register_test_console`) is shown as is:
//! its editor and session are the registrant's, and the host only draws it (no session, no
//! autosave, no binding).

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use hacivat_exec::sqlctx::dialect_ctx;
use hacivat_l10n::{NoArgs, Text, ids};
use hacivat_proto::{AccessLabel, Department, EnvTag, ProfileId};
use hacivat_session::LiveState;
use hacivat_session::session::{
    ClientId, ClosePolicy, SessionHandle, SessionKind, SessionSnapshot, SessionSpec, SessionState,
    effective_read_only,
};
use hacivat_sql_editor::{SqlLanguageService, effective_flavor};
use hacivat_ui::panes::TabId;
use hacivat_ui::text::{TextSource, paragraph};
use hacivat_ui::toast::{Toast, ToastKind, ToastQueue};
use hacivat_ui_core::gpui_surface::{
    AnyView, App, AppContext as _, Bounds, Context, Entity, Focusable as _, Global,
    InteractiveElement as _, IntoElement, ParentElement as _, Pixels, Render, SharedString,
    StyleRefinement, Styled as _, Subscription, Task, WeakEntity, Window, div, point, px,
};
use hacivat_ui_core::theme::{Theme, popover_surface};
use hacivat_ui_editor::large_file::SizeText;
use hacivat_ui_editor::{
    CaretBlink, DocContext, DocId, EDITOR_CONTEXT, EditorConfig, EditorEvent, EditorView,
    LanguageService, a11y,
};
use sql_dialect::ClientFlavor;
use std::time::Instant;

use crate::connections::capsule::{access_label, current_profile, env_kind};
use crate::connections::model::ConnectionsModel;
use crate::console::autosave::{self, Autosave, RESTORE_MAX_BYTES, Restored};
use crate::console::decor::{console_title, profile_of};
use crate::console::registry::{ConsoleId, ConsoleState, Consoles};
use crate::console::status::EditorSegment;
use crate::console::toolbar::{self, RunUx, ToolbarActions, ToolbarModel};
use crate::l10n_global::tr;
use crate::results::ConsoleResults;
use crate::shell::{AppState, Workspace, register_focus_context};

// ---- the hosts ------------------------------------------------------------------------------

/// What the app knows of one console host.
struct HostEntry {
    host: WeakEntity<ConsoleHost>,
    /// The profile and the console's number among its consoles (from 1).
    binding: Option<(ProfileId, u32)>,
    /// The host owns its console (not a console someone else registered for the tab).
    owned: bool,
}

/// Every console host, by tab; and the profiles new consoles were just bound to.
#[derive(Default)]
struct Hosts {
    by_tab: HashMap<TabId, HostEntry>,
    /// Profiles `console_binding` handed to a new console tab whose view is not built yet: that
    /// console starts empty, where a restored one reopens its text. With a text: a shots
    /// fixture's console, over that text, autosave off.
    fresh: RefCell<Vec<(ProfileId, Option<String>)>>,
}

impl Global for Hosts {}

/// The binding of console `tab`'s host: its profile and its number among that profile's
/// consoles.
pub(crate) fn host_binding(cx: &App, tab: TabId) -> Option<(ProfileId, u32)> {
    cx.try_global::<Hosts>()?.by_tab.get(&tab)?.binding
}

/// The smallest console number of `profile` no other host holds.
fn free_slot(cx: &App, profile: ProfileId, tab: TabId) -> u32 {
    let used: HashSet<u32> = cx
        .try_global::<Hosts>()
        .map(|h| {
            h.by_tab
                .iter()
                .filter(|(t, e)| **t != tab && e.host.upgrade().is_some())
                .filter_map(|(_, e)| e.binding)
                .filter(|(p, _)| *p == profile)
                .map(|(_, slot)| slot)
                .collect()
        })
        .unwrap_or_default();
    (1..).find(|n| !used.contains(n)).unwrap_or(1)
}

/// The live host of every owned console bound to `profile`.
fn hosts_of(cx: &App, profile: ProfileId) -> Vec<Entity<ConsoleHost>> {
    cx.try_global::<Hosts>()
        .map(|h| {
            h.by_tab
                .values()
                .filter(|e| e.owned && e.binding.is_some_and(|(p, _)| p == profile))
                .filter_map(|e| e.host.upgrade())
                .collect()
        })
        .unwrap_or_default()
}

/// The app-wide parts of the console host: the hosts' registry, the size text of the editor's
/// banner, the read-only rule's observer and the autosave at quit. Called once from
/// `commands::register` (Task 1b's `console::install`).
pub(crate) fn install(cx: &mut App) {
    cx.default_global::<Hosts>();
    cx.set_global(SizeText(Rc::new(|bytes, cx: &App| size_text(cx, bytes))));
    cx.observe_global::<ConnectionsModel>(enforce_read_only)
        .detach();
    cx.on_app_quit(|cx| {
        save_all_now(cx);
        async {}
    })
    .detach();
}

/// `bytes` in megabytes, in the regional format ("18,4 MB" in Turkey).
pub fn size_text(cx: &App, bytes: u64) -> SharedString {
    let tenths = (u128::from(bytes) * 10 + (1 << 19)) >> 20;
    tr(
        cx,
        ids::hv::CONSOLE_SIZE_MB,
        ids::hv::HvConsoleSizeMb {
            mb: hacivat_l10n::ExactDecimal(format!("{}.{}", tenths / 10, tenths % 10)),
        },
    )
}

/// The production policy of the app's sessions (R-M3-29), read from the manager.
fn production_read_only(cx: &App) -> bool {
    cx.try_global::<ConnectionsModel>()
        .is_some_and(|m| m.manager().policy().production_read_only)
}

/// Whether a session of `profile` must be read-only now.
fn must_be_read_only(cx: &App, profile: ProfileId) -> bool {
    profile_of(cx, profile).is_some_and(|p| effective_read_only(&p, production_read_only(cx)))
}

/// The read-only rule (R-M3-6, ledger T32 note): a console session that is read-write where
/// the profile now says read-only does not survive. Every console of that profile drops its
/// session at once, and the profile reconnects (its new sessions open read-only).
fn enforce_read_only(cx: &mut App) {
    let stale: Vec<ProfileId> = Consoles::ids(cx)
        .into_iter()
        .filter_map(|id| {
            let s = Consoles::get(cx, id)?;
            let (profile, session) = (s.profile?, s.session.as_ref()?);
            let owned = cx
                .try_global::<Hosts>()
                .and_then(|h| h.by_tab.get(&s.tab))
                .is_some_and(|e| e.owned);
            (owned && !session.state().borrow().read_only && must_be_read_only(cx, profile))
                .then_some(profile)
        })
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    for profile in stale {
        reconnect_read_only(cx, profile);
    }
}

/// Drops every console session of `profile`, then reconnects the profile.
fn reconnect_read_only(cx: &mut App, profile: ProfileId) {
    for host in hosts_of(cx, profile) {
        host.update(cx, |h, cx| h.release_session(cx));
    }
    ConnectionsModel::disconnect(cx, profile);
    ConnectionsModel::connect(cx, profile, None);
}

/// Writes every console with unsaved edits now (quit).
fn save_all_now(cx: &mut App) {
    let hosts: Vec<Entity<ConsoleHost>> = cx
        .try_global::<Hosts>()
        .map(|h| h.by_tab.values().filter_map(|e| e.host.upgrade()).collect())
        .unwrap_or_default();
    for host in hosts {
        let h = host.read(cx);
        if let (Some(save), Some(editor)) = (&h.autosave, h.editor()) {
            let snap = editor.read(cx).snapshot();
            if let Err(e) = save.write_now(&snap) {
                tracing::warn!(error = %e, "console autosave at quit failed");
            }
        }
    }
}

// ---- the session ----------------------------------------------------------------------------

/// Where the console's session is.
enum Acquire {
    /// None asked for (unbound, disconnected, or the session is here).
    Idle,
    /// `SessionManager::open` is pending on the core runtime.
    Waiting {
        abort: tokio::task::AbortHandle,
        _task: Task<()>,
    },
    /// The user cancelled the wait; Run asks again.
    Cancelled,
}

/// What the toolbar reads of the session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SessionUx {
    read_only: bool,
    access: AccessLabel,
    /// The request running now (0: the idle cursor release, which runs nothing).
    busy: Option<u64>,
    cancelling: bool,
}

impl SessionUx {
    fn of(snap: &SessionSnapshot) -> Self {
        let (busy, cancelling) = match snap.state {
            SessionState::Busy { req, .. } if req != 0 => (Some(req), false),
            SessionState::Cancelling { req, .. } => (Some(req), true),
            _ => (None, false),
        };
        SessionUx {
            read_only: snap.read_only,
            access: snap.access,
            busy,
            cancelling,
        }
    }
}

// ---- the host -------------------------------------------------------------------------------

enum Mode {
    /// Today's M2 home.
    Unbound,
    /// A restored console reading its text.
    Loading,
    Ready {
        console: ConsoleId,
        editor: Entity<EditorView>,
        /// Ours (`false`: registered by someone else; shown only).
        owned: bool,
    },
}

/// The view of one console tab.
pub struct ConsoleHost {
    tab: TabId,
    n: u32,
    binding: Option<(ProfileId, u32)>,
    mode: Mode,
    /// Every profile's state when the host last looked (a connect binds).
    seen: HashMap<ProfileId, LiveKind>,
    acquire: Acquire,
    /// The session the host watches, and what it shows.
    watched: Option<(hacivat_session::session::SessionId, Task<()>)>,
    ux: Option<SessionUx>,
    /// The request the run clock counts, and when (the executor clock) it was first seen.
    clock: Option<(u64, Instant)>,
    clock_task: Option<Task<()>>,
    segment: Option<Entity<EditorSegment>>,
    autosave: Option<Autosave>,
    autosave_task: Option<Task<()>>,
    /// The connected state the editor was last told.
    told: Option<bool>,
    /// The editor's failed paints already logged.
    paint_errors: u64,
    /// The toolbar's state forced by a shots fixture (feature `shots`).
    fixture: Option<FixtureUx>,
    /// The workspace's focused pane when the host last looked.
    pane: Option<hacivat_ui::panes::PaneId>,
    _subscriptions: Vec<Subscription>,
}

/// The `console_view` hook: the view of console `tab` (`Console <n>`, bound to `profile`),
/// built once per tab.
pub fn build(
    tab: TabId,
    n: u32,
    profile: Option<ProfileId>,
    window: &mut Window,
    cx: &mut App,
) -> AnyView {
    cx.new(|cx| ConsoleHost::new(tab, n, profile, window, cx))
        .into()
}

impl ConsoleHost {
    fn new(
        tab: TabId,
        n: u32,
        profile: Option<ProfileId>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut subscriptions = vec![
            cx.observe_global_in::<ConnectionsModel>(window, Self::on_connections),
            cx.on_release(Self::released),
        ];
        // The workspace is the window's root; it is rendering this tab now, so only its handle
        // is taken here.
        let workspace = window.root::<Workspace>().flatten();
        if let Some(ws) = &workspace {
            subscriptions.push(cx.observe_in(ws, window, Self::on_workspace));
        }
        let mut this = ConsoleHost {
            tab,
            n,
            binding: None,
            mode: Mode::Unbound,
            seen: live_kinds(cx),
            acquire: Acquire::Idle,
            watched: None,
            ux: None,
            clock: None,
            clock_task: None,
            segment: None,
            autosave: None,
            autosave_task: None,
            told: None,
            paint_errors: 0,
            fixture: None,
            pane: None,
            _subscriptions: subscriptions,
        };
        let me = cx.weak_entity();
        let registered = Consoles::by_tab(cx, tab);
        cx.default_global::<Hosts>().by_tab.insert(
            tab,
            HostEntry {
                host: me,
                binding: None,
                owned: registered.is_none(),
            },
        );
        if let Some(console) = registered {
            this.adopt(console, cx);
        } else if let Some(profile) = profile {
            this.set_binding(profile, cx);
            match take_fresh(cx, profile) {
                Some(None) => this.open_editor(String::new(), Some(None), true, window, cx),
                Some(Some(text)) => this.open_editor(text, None, false, window, cx),
                None => this.restore(window, cx),
            }
        }
        this
    }

    /// Shows the console someone else registered for this tab.
    fn adopt(&mut self, console: ConsoleId, cx: &mut Context<Self>) {
        let Some(state) = Consoles::get(cx, console).cloned() else {
            return;
        };
        let Some(editor) = state.editor else {
            return;
        };
        if let Some(profile) = state.profile {
            self.binding = Some((profile, 1));
        }
        // Its keys reach the editor's commands like a console of ours.
        let focus = editor.focus_handle(cx);
        register_focus_context(cx, focus, EDITOR_CONTEXT);
        self.segment = Some(cx.new(|cx| EditorSegment::new(&editor, cx)));
        self.mode = Mode::Ready {
            console,
            editor,
            owned: false,
        };
    }

    fn set_binding(&mut self, profile: ProfileId, cx: &mut Context<Self>) {
        let slot = free_slot(cx, profile, self.tab);
        self.binding = Some((profile, slot));
        if let Some(e) = cx.default_global::<Hosts>().by_tab.get_mut(&self.tab) {
            e.binding = self.binding;
        }
    }

    /// The editor, once there is one.
    fn editor(&self) -> Option<&Entity<EditorView>> {
        match &self.mode {
            Mode::Ready { editor, .. } => Some(editor),
            _ => None,
        }
    }

    fn console(&self) -> Option<ConsoleId> {
        match &self.mode {
            Mode::Ready { console, .. } => Some(*console),
            _ => None,
        }
    }

    fn owned(&self) -> bool {
        matches!(self.mode, Mode::Ready { owned: true, .. })
    }

    /// The editor status segment.
    pub fn segment(&self) -> Option<&Entity<EditorSegment>> {
        self.segment.as_ref()
    }

    /// The autosave file, once the console has one.
    pub fn autosave_path(&self) -> Option<PathBuf> {
        self.autosave.as_ref().map(|a| a.path().to_path_buf())
    }

    // ---- binding ----------------------------------------------------------------------------

    /// The connections changed: an unbound console in focus binds to a profile that a connect
    /// just connected; a bound one follows its profile's state.
    ///
    /// "A connect" is a profile this host saw disconnected, connecting or failed that is now
    /// connected: a recovery (lost, reconnecting) binds nothing, and neither does a profile
    /// that appears already connected (the host never saw it otherwise).
    fn on_connections(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let now = live_kinds(cx);
        match self.mode {
            Mode::Unbound => {
                let newly: Vec<ProfileId> = now
                    .iter()
                    .filter(|(id, kind)| {
                        **kind == LiveKind::Connected && self.seen.get(id) == Some(&LiveKind::Idle)
                    })
                    .map(|(id, _)| *id)
                    .collect();
                let current = cx
                    .try_global::<ConnectionsModel>()
                    .and_then(|m| m.current());
                let pick = current
                    .filter(|c| newly.contains(c))
                    .or_else(|| newly.first().copied());
                if let Some(profile) = pick
                    && self.is_focused_tab(window, cx)
                {
                    self.bind_fresh(profile, window, cx);
                }
            }
            Mode::Ready { owned: true, .. } => self.sync_session(window, cx),
            _ => {}
        }
        self.seen = now;
    }

    /// The workspace changed. When the focus moved to another pane (`workspace.focus_next_pane`,
    /// a click in the pane) and this console is that pane's active tab, its editor takes the
    /// keyboard focus, so moving between panes moves the typing too (ledger N2).
    fn on_workspace(&mut self, ws: Entity<Workspace>, window: &mut Window, cx: &mut Context<Self>) {
        let (pane, active, layers) = {
            let ws = ws.read(cx);
            (
                ws.layout().0.focused,
                ws.active_tab() == Some(self.tab),
                !ws.layer_ids().is_empty(),
            )
        };
        let moved = self.pane.replace(pane).is_some_and(|before| before != pane);
        if !(moved && active) || layers {
            return;
        }
        if let Some(editor) = self.editor() {
            let focus = editor.focus_handle(cx);
            if !focus.contains_focused(window, cx) {
                window.focus(&focus, cx);
            }
        }
    }

    /// This console is the focused pane's active tab.
    fn is_focused_tab(&self, window: &mut Window, cx: &App) -> bool {
        window
            .root::<Workspace>()
            .flatten()
            .is_some_and(|ws| ws.read(cx).active_tab() == Some(self.tab))
    }

    /// Binds this unbound console to `profile` (the first connect, R-M3-37).
    fn bind_fresh(&mut self, profile: ProfileId, window: &mut Window, cx: &mut Context<Self>) {
        self.set_binding(profile, cx);
        let tab = self.tab;
        if let Some(ws) = window.root::<Workspace>().flatten() {
            ws.update(cx, |ws, cx| ws.set_console_profile(tab, Some(profile), cx));
        }
        // The focus stays where the connect came from (the explorer, the capsule).
        self.open_editor(String::new(), Some(None), false, window, cx);
    }

    /// Reads the restored console's text on the core runtime, then opens it.
    fn restore(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((profile, _)) = self.binding else {
            return;
        };
        let Some(paths) = cx.try_global::<AppState>().map(|s| s.paths.clone()) else {
            self.open_editor(String::new(), None, false, window, cx);
            return;
        };
        let path = autosave::console_path(&paths, profile, self.n);
        self.mode = Mode::Loading;
        let read = path.clone();
        let reading: Task<Restored> = match cx.try_global::<ConnectionsModel>() {
            Some(m) => {
                let (tx, rx) = tokio::sync::oneshot::channel();
                m.runtime().spawn_blocking(move || {
                    let _ = tx.send(autosave::read(&read));
                });
                cx.background_spawn(async move { rx.await.unwrap_or(Restored::Unreadable) })
            }
            None => cx.background_spawn(async move { autosave::read(&read) }),
        };
        cx.spawn_in(window, async move |this, cx| {
            let found = reading.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.restored(found, path, window, cx);
            });
        })
        .detach();
    }

    fn restored(
        &mut self,
        found: Restored,
        path: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match found {
            Restored::Missing => self.open_editor(String::new(), Some(None), false, window, cx),
            Restored::Text { text, hash } => {
                self.open_editor(text, Some(Some(hash)), false, window, cx);
            }
            Restored::TooLarge { bytes } => {
                // R-M3-49: not opened, left on disk; the console's autosave stays off so the
                // file is never written over.
                let name = self.title(cx);
                let (size, limit) = (size_text(cx, bytes), size_text(cx, RESTORE_MAX_BYTES));
                let mut toast = Toast::new(
                    ToastKind::Info,
                    Rc::new(move |cx: &App| {
                        tr(
                            cx,
                            ids::hv::CONSOLE_RESTORE_REFUSED_BODY,
                            ids::hv::HvConsoleRestoreRefusedBody {
                                size: Text(size.to_string()),
                                limit: Text(limit.to_string()),
                            },
                        )
                    }),
                );
                toast.title = Some(Rc::new(move |cx: &App| {
                    tr(
                        cx,
                        ids::hv::CONSOLE_RESTORE_REFUSED_TITLE,
                        ids::hv::HvConsoleRestoreRefusedTitle {
                            name: Text(name.to_string()),
                        },
                    )
                }));
                toast.wide = true;
                ToastQueue::push(cx, toast);
                tracing::info!(path = %path.display(), bytes, "console not restored: too large");
                self.open_editor(String::new(), None, false, window, cx);
            }
            Restored::Unreadable => {
                tracing::warn!(path = %path.display(), "console not restored: unreadable");
                self.open_editor(String::new(), None, false, window, cx);
            }
        }
    }

    /// The console's title (the tab's).
    fn title(&self, cx: &App) -> SharedString {
        match self
            .binding
            .and_then(|(p, slot)| Some((profile_of(cx, p)?, slot)))
        {
            Some((profile, slot)) => console_title(cx, &profile, slot),
            None => SharedString::default(),
        }
    }

    /// Opens the console's editor over `text`. `autosave`: `None` keeps autosave off (a refused
    /// restore), `Some(on_disk)` saves to the console's file whose content hash is `on_disk`.
    /// `focus`: the user just made this console (it takes the keyboard focus).
    fn open_editor(
        &mut self,
        text: String,
        autosave: Option<Option<u128>>,
        focus: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((profile_id, slot)) = self.binding else {
            return;
        };
        let profile = profile_of(cx, profile_id);
        let doc = DocId(u64::from(self.tab.0));
        let service = SqlLanguageService::new();
        let language: Rc<dyn LanguageService> = service.clone();
        let label = a11y::label(self.title(cx));
        let cfg = editor_config(cx);
        let editor = cx.new(|cx| EditorView::new(doc, &text, cfg, language, label, window, cx));
        let log = hacivat_exec::SessionLog::new();
        let results = cx.new(|_| ConsoleResults::new(log.clone()));
        let client = if slot <= 1 {
            ClientId::DEFAULT
        } else {
            ClientId(u64::from(self.n))
        };
        let console = Consoles::register(
            cx,
            ConsoleState {
                id: ConsoleId(0),
                tab: self.tab,
                n: self.n,
                profile: Some(profile_id),
                client,
                session: None,
                editor: Some(editor.clone()),
                results: Some(results.clone()),
                sql: None,
                log,
                service: Some(service),
            },
        );
        exec_attach(console, &editor, window, cx);
        let focus_handle = editor.focus_handle(cx);
        register_focus_context(cx, focus_handle.clone(), EDITOR_CONTEXT);
        let subscriptions = [
            cx.subscribe_in(&editor, window, Self::on_editor_event),
            cx.on_focus(&focus_handle, window, move |_, _, cx| {
                Consoles::set_focused(cx, Some(console));
            }),
        ];
        self._subscriptions.extend(subscriptions);
        self.segment = Some(cx.new(|cx| EditorSegment::new(&editor, cx)));
        self.autosave = autosave.and_then(|on_disk| {
            let paths = cx.try_global::<AppState>()?.paths.clone();
            Some(Autosave::new(
                autosave::console_path(&paths, profile_id, self.n),
                on_disk,
            ))
        });
        self.mode = Mode::Ready {
            console,
            editor: editor.clone(),
            owned: true,
        };
        let _ = (profile, results);
        self.sync_session(window, cx);
        if focus && window.is_window_active() {
            window.defer(cx, move |window, cx| window.focus(&focus_handle, cx));
        }
        cx.notify();
    }

    fn on_editor_event(
        &mut self,
        editor: &Entity<EditorView>,
        event: &EditorEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // The editor counts failed paints; the app logs each new one (never their text).
        let errors = editor.read(cx).layout_stats().paint_errors;
        if errors > self.paint_errors {
            tracing::warn!(
                errors,
                new = errors - self.paint_errors,
                "console editor: paints failed"
            );
            self.paint_errors = errors;
        }
        match event {
            EditorEvent::Changed { .. } => self.schedule_autosave(cx),
            EditorEvent::TierChanged(tier) => {
                let connected = self.told == Some(true);
                let tier = *tier;
                editor.update(cx, |v, cx| {
                    v.set_doc_context(DocContext { connected, tier }, cx);
                });
            }
            _ => {}
        }
    }

    /// Saves 1 s after the last edit (a newer edit restarts the wait).
    fn schedule_autosave(&mut self, cx: &mut Context<Self>) {
        if self.autosave.is_none() {
            return;
        }
        let timer = cx.background_executor().timer(autosave::AUTOSAVE_DELAY);
        self.autosave_task = Some(cx.spawn(async move |this, cx| {
            timer.await;
            let _ = this.update(cx, |this, cx| this.save_now(cx));
        }));
    }

    /// Hands the editor's snapshot to the core runtime (never awaited here).
    fn save_now(&mut self, cx: &mut Context<Self>) {
        let (Some(save), Some(editor)) = (&self.autosave, self.editor()) else {
            return;
        };
        let snap = editor.read(cx).snapshot();
        match cx.try_global::<ConnectionsModel>() {
            Some(m) => save.save_off_thread(snap, &m.runtime()),
            None => {
                let save = save.clone();
                cx.background_spawn(async move {
                    let _ = save.write_now(&snap);
                })
                .detach();
            }
        }
    }

    // ---- the session ------------------------------------------------------------------------

    /// Follows the profile's state: a connected profile gives the console a session (and the
    /// editor its connected context); anything else takes it away.
    fn sync_session(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (Some((profile, _)), Some(console)) = (self.binding, self.console()) else {
            return;
        };
        if !self.owned() {
            return;
        }
        let live = cx
            .try_global::<ConnectionsModel>()
            .map(|m| m.live(profile))
            .unwrap_or_default();
        match live {
            LiveState::Connected(_) => {
                let has = Consoles::get(cx, console).is_some_and(|s| s.session.is_some());
                if !has && matches!(self.acquire, Acquire::Idle) {
                    self.acquire(profile, window, cx);
                }
            }
            _ => {
                self.release_session(cx);
                if self.told.is_none_or(|c| c) {
                    self.tell_editor(false, cx);
                }
            }
        }
    }

    fn tell_editor(&mut self, connected: bool, cx: &mut Context<Self>) {
        self.told = Some(connected);
        if let Some(editor) = self.editor().cloned() {
            editor.update(cx, |v, cx| {
                let tier = v.status().tier;
                v.set_doc_context(DocContext { connected, tier }, cx);
            });
        }
    }

    /// Asks for the console's session: the profile's default one for its first console, else
    /// a session of its own (which may wait for the budget).
    fn acquire(&mut self, profile: ProfileId, window: &mut Window, cx: &mut Context<Self>) {
        let (Some((_, slot)), Some(console)) = (self.binding, self.console()) else {
            return;
        };
        let Some(m) = cx.try_global::<ConnectionsModel>() else {
            return;
        };
        let Some(sessions) = m.manager().sessions(profile) else {
            return;
        };
        let read_only = must_be_read_only(cx, profile);
        if slot <= 1 {
            let Some(handle) = sessions.default_session() else {
                return;
            };
            if read_only && !handle.state().borrow().read_only {
                // Never a read-write session where the profile says read-only. Deferred: the
                // reconnect updates every host of the profile, this one included.
                cx.defer(move |cx| reconnect_read_only(cx, profile));
                return;
            }
            self.take_session(console, handle, window, cx);
            return;
        }
        let spec = SessionSpec {
            kind: SessionKind::Console(ClientId(u64::from(self.n))),
            read_only,
            department: Department::Console,
        };
        let ix = crate::connections::interaction::interaction(cx);
        let open = sessions.open(spec, ix);
        let (tx, rx) = tokio::sync::oneshot::channel();
        let join = m.runtime().spawn(async move {
            let opened = open.await;
            if let Err(Ok(handle)) = tx.send(opened) {
                // Nobody waits any more (the console closed): give the session back.
                let _ = handle.close(ClosePolicy::RollBack).await;
            }
        });
        let task = cx.spawn_in(window, async move |this, cx| {
            let opened = rx.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.acquire = Acquire::Idle;
                match opened {
                    Ok(Ok(handle)) => {
                        if handle.state().borrow().read_only || !read_only {
                            this.take_session(console, handle, window, cx);
                        } else {
                            let close = handle.close(ClosePolicy::RollBack);
                            if let Some(m) = cx.try_global::<ConnectionsModel>() {
                                m.runtime().spawn(close);
                            }
                        }
                    }
                    Ok(Err(_)) => {
                        tracing::info!("console session not opened");
                    }
                    Err(_) => {}
                }
                cx.notify();
            });
        });
        self.acquire = Acquire::Waiting {
            abort: join.abort_handle(),
            _task: task,
        };
        cx.notify();
    }

    fn take_session(
        &mut self,
        console: ConsoleId,
        handle: SessionHandle,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        Consoles::update(cx, console, |s| s.session = Some(handle));
        exec_rebind(console, window, cx);
        self.tell_editor(true, cx);
        self.watch_session(cx);
    }

    /// Takes the session away from the console; a session of its own closes.
    fn release_session(&mut self, cx: &mut Context<Self>) {
        if let Acquire::Waiting { abort, .. } = std::mem::replace(&mut self.acquire, Acquire::Idle)
        {
            abort.abort();
        }
        let Some(console) = self.console() else {
            return;
        };
        let mut released = None;
        Consoles::update(cx, console, |s| released = s.session.take());
        let own = self.binding.is_some_and(|(_, slot)| slot > 1);
        if let (Some(handle), true) = (released, own)
            && let Some(m) = cx.try_global::<ConnectionsModel>()
        {
            m.runtime().spawn(handle.close(ClosePolicy::RollBack));
        }
        self.watched = None;
        self.ux = None;
        self.clock = None;
        self.clock_task = None;
        cx.notify();
    }

    /// Watches the console's session for the toolbar (a `tokio::sync::watch` awaited on the
    /// foreground, Q4).
    fn watch_session(&mut self, cx: &mut Context<Self>) {
        let session = self
            .console()
            .and_then(|c| Consoles::get(cx, c))
            .and_then(|s| s.session.clone());
        let Some(session) = session else {
            self.watched = None;
            return;
        };
        if self
            .watched
            .as_ref()
            .is_some_and(|(id, _)| *id == session.id())
        {
            return;
        }
        let mut rx = session.state();
        let task = cx.spawn(async move |this, cx| {
            loop {
                let ux = SessionUx::of(&rx.borrow_and_update());
                if this.update(cx, |this, cx| this.on_session(ux, cx)).is_err() {
                    break;
                }
                if rx.changed().await.is_err() {
                    break;
                }
            }
        });
        self.watched = Some((session.id(), task));
    }

    fn on_session(&mut self, ux: SessionUx, cx: &mut Context<Self>) {
        if self.ux == Some(ux) {
            return;
        }
        let busy = ux.busy;
        self.ux = Some(ux);
        match busy {
            Some(req) if self.clock.is_none_or(|(r, _)| r != req) => {
                self.clock = Some((req, cx.background_executor().now()));
                // Once a second, no faster (no ambient motion).
                self.clock_task = Some(cx.spawn(async move |this, cx| {
                    loop {
                        cx.background_executor().timer(Duration::from_secs(1)).await;
                        if this.update(cx, |_, cx| cx.notify()).is_err() {
                            break;
                        }
                    }
                }));
            }
            Some(_) => {}
            None => {
                self.clock = None;
                self.clock_task = None;
            }
        }
        cx.notify();
    }

    // ---- the toolbar's actions --------------------------------------------------------------

    /// Run: ends a composition first (the platform commits it on the click; this covers a
    /// click it did not see), gives the editor the focus back and runs `editor.run` on it.
    fn run(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if matches!(self.acquire, Acquire::Cancelled) {
            self.acquire = Acquire::Idle;
            self.sync_session(window, cx);
            return;
        }
        let Some(editor) = self.editor().cloned() else {
            return;
        };
        if let Some(console) = self.console() {
            Consoles::set_focused(cx, Some(console));
        }
        let focus = editor.focus_handle(cx);
        window.focus(&focus, cx);
        editor.update(cx, |v, cx| {
            if v.status().composing {
                hacivat_ui_core::gpui_surface::EntityInputHandler::unmark_text(v, window, cx);
            }
            v.run_command("editor.run", window, cx);
        });
    }

    /// Stop: `exec.cancel` on this console (Task 33's command).
    fn stop(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(console) = self.console() {
            Consoles::set_focused(cx, Some(console));
        }
        if let Some(editor) = self.editor() {
            let focus = editor.focus_handle(cx);
            window.focus(&focus, cx);
        }
        window.defer(cx, |window, cx| {
            hacivat_ui::actions::ActionRegistry::run("exec.cancel", window, cx);
        });
    }

    fn cancel_wait(&mut self, cx: &mut Context<Self>) {
        if let Acquire::Waiting { abort, .. } =
            std::mem::replace(&mut self.acquire, Acquire::Cancelled)
        {
            abort.abort();
        }
        cx.notify();
    }

    /// The production explanation, under the notice.
    fn explain(&mut self, at: Option<Bounds<Pixels>>, window: &mut Window, cx: &mut Context<Self>) {
        let anchor = at.map_or_else(|| window.mouse_position(), |b| b.bottom_left());
        let anchor = point(anchor.x, anchor.y + px(6.0));
        let view: AnyView = cx.new(ProdPopover::new).into();
        if let Some(ws) = window.root::<Workspace>().flatten() {
            ws.update(cx, |ws, cx| {
                ws.open_anchored("console-prod-popover", anchor, view, window, cx);
            });
        }
    }

    // ---- render -----------------------------------------------------------------------------

    fn toolbar_model(&self, cx: &App) -> ToolbarModel {
        let mut model = self.live_toolbar_model(cx);
        if let Some(f) = &self.fixture {
            if let Some(run) = f.run {
                model.run = run;
            }
            if let Some((in_use, max)) = f.waiting {
                let name = self
                    .binding
                    .and_then(|(p, _)| profile_of(cx, p))
                    .map(|p| SharedString::from(p.name.clone()))
                    .unwrap_or_default();
                model.waiting = Some((name, in_use, max));
                model.run_enabled = false;
            }
            if let Some(access) = &f.read_only {
                model.read_only = Some(access.clone());
            }
        }
        model
    }

    fn live_toolbar_model(&self, cx: &App) -> ToolbarModel {
        let profile = self.binding.and_then(|(p, _)| profile_of(cx, p));
        let production = production_read_only(cx)
            && profile
                .as_ref()
                .is_some_and(|p| p.environment == Some(EnvTag::Production));
        let name = self.title(cx);
        let waiting = match (&self.acquire, self.binding) {
            (Acquire::Waiting { .. }, Some((p, _))) => cx
                .try_global::<ConnectionsModel>()
                .and_then(|m| m.manager().sessions(p))
                .map(|s| s.budget_use())
                .filter(|(in_use, max)| in_use >= max)
                .map(|(in_use, max)| {
                    let name = profile
                        .as_ref()
                        .map(|p| SharedString::from(p.name.clone()))
                        .unwrap_or_default();
                    (name, in_use, max)
                }),
            _ => None,
        };
        let run = match (self.ux, self.clock) {
            (Some(ux), _) if ux.cancelling => RunUx::Cancelling,
            (Some(ux), Some((req, since))) if ux.busy == Some(req) => {
                let now = cx.background_executor().now();
                RunUx::Running(now.saturating_duration_since(since).as_secs())
            }
            _ => RunUx::Idle,
        };
        let read_only = self
            .ux
            .filter(|ux| ux.read_only && !production)
            .map(|ux| access_label(cx, ux.access));
        ToolbarModel {
            run,
            run_enabled: !matches!(self.acquire, Acquire::Waiting { .. }),
            name,
            env: profile
                .as_ref()
                .and_then(|p| p.environment)
                .map(|tag| env_kind(Some(tag))),
            waiting,
            read_only,
            production,
        }
    }

    fn actions(&self, cx: &mut Context<Self>) -> ToolbarActions {
        let me = cx.weak_entity();
        let (a, b, c, d) = (me.clone(), me.clone(), me.clone(), me);
        ToolbarActions {
            run: Rc::new(move |window, cx| {
                let _ = a.update(cx, |h, cx| h.run(window, cx));
            }),
            stop: Rc::new(move |window, cx| {
                let _ = b.update(cx, |h, cx| h.stop(window, cx));
            }),
            cancel_wait: Rc::new(move |_, cx| {
                let _ = c.update(cx, |h, cx| h.cancel_wait(cx));
            }),
            explain: Rc::new(move |at, window, cx| {
                let _ = d.update(cx, |h, cx| h.explain(at, window, cx));
            }),
        }
    }

    /// The tab closed: the console leaves the registry, a session of its own closes, and
    /// unsaved edits are written.
    fn released(&mut self, cx: &mut App) {
        if let Some(e) = cx.default_global::<Hosts>().by_tab.get(&self.tab)
            && e.host.upgrade().is_none()
        {
            cx.default_global::<Hosts>().by_tab.remove(&self.tab);
        }
        let Mode::Ready {
            console,
            editor,
            owned,
        } = std::mem::replace(&mut self.mode, Mode::Unbound)
        else {
            return;
        };
        if let Acquire::Waiting { abort, .. } = std::mem::replace(&mut self.acquire, Acquire::Idle)
        {
            abort.abort();
        }
        if owned {
            if let Some(save) = &self.autosave {
                let snap = editor.read(cx).snapshot();
                match cx.try_global::<ConnectionsModel>() {
                    Some(m) => save.save_off_thread(snap, &m.runtime()),
                    None => {
                        let _ = save.write_now(&snap);
                    }
                }
            }
            let own = self.binding.is_some_and(|(_, slot)| slot > 1);
            let session = Consoles::get(cx, console).and_then(|s| s.session.clone());
            if let (true, Some(handle), Some(m)) =
                (own, session, cx.try_global::<ConnectionsModel>())
            {
                m.runtime().spawn(handle.close(ClosePolicy::RollBack));
            }
        }
        Consoles::remove(cx, console);
    }
}

impl Render for ConsoleHost {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *Theme::get(cx);
        let editor = match &self.mode {
            Mode::Unbound => return crate::shell::workspace::editor_empty(&theme, cx),
            Mode::Loading => {
                return div()
                    .size_full()
                    .bg(theme.tokens.colors.panel.to_hsla())
                    .into_any_element();
            }
            Mode::Ready { editor, .. } => editor.clone(),
        };
        // A registrant may swap the console's session (Task 34's tests do).
        self.watch_session(cx);
        let model = self.toolbar_model(cx);
        let actions = self.actions(cx);
        let bar = toolbar::build(&model, &actions, cx);
        div()
            .id(("console", self.tab.0))
            .flex()
            .flex_col()
            .size_full()
            .bg(theme.tokens.colors.panel.to_hsla())
            .child(bar)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h(px(0.0))
                    .child(a11y::embed(
                        &editor,
                        StyleRefinement::default().size_full(),
                        window,
                    )),
            )
            .into_any_element()
    }
}

// ---- helpers --------------------------------------------------------------------------------

/// What binding reads of a profile's state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LiveKind {
    Connected,
    /// Lost or reconnecting: a recovery, not a connect.
    Recovering,
    /// Disconnected, connecting or failed: a connect may follow.
    Idle,
}

/// Every profile's state now.
fn live_kinds(cx: &App) -> HashMap<ProfileId, LiveKind> {
    cx.try_global::<ConnectionsModel>()
        .map(|m| {
            m.profiles()
                .iter()
                .map(|p| {
                    let kind = match m.live(p.id) {
                        LiveState::Connected(_) => LiveKind::Connected,
                        LiveState::Lost { .. } | LiveState::Reconnecting { .. } => {
                            LiveKind::Recovering
                        }
                        _ => LiveKind::Idle,
                    };
                    (p.id, kind)
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Whether `console_binding` (or a shots fixture, with its text) just handed `profile` to a
/// new console tab (consumed).
fn take_fresh(cx: &App, profile: ProfileId) -> Option<Option<String>> {
    let hosts = cx.try_global::<Hosts>()?;
    let mut fresh = hosts.fresh.borrow_mut();
    let at = fresh.iter().position(|(p, _)| *p == profile)?;
    Some(fresh.remove(at).1)
}

/// The editor's configuration from the `[editor]` settings.
fn editor_config(cx: &App) -> EditorConfig {
    let s = cx
        .try_global::<AppState>()
        .map(|s| s.settings.editor)
        .unwrap_or_default();
    EditorConfig {
        tab_size: s.tab_size.get(),
        insert_spaces: s.insert_spaces,
        copy_line_without_selection: s.copy_line_without_selection,
        caret_blink: match s.caret_blink {
            hacivat_settings::CaretBlinkPref::System => CaretBlink::System,
            hacivat_settings::CaretBlinkPref::Off => CaretBlink::Off,
        },
        ..EditorConfig::default()
    }
}

// ## m3a-t32: exec_ui attach/rebind (T33)
// Task 33's `exec_ui::attach` (once per console) and `exec_ui::rebind` (after the console's
// session changes) own the SQL context (`set_sql_context`, its probe), the remembered chooser
// kind and the results display. Task 33 is not on this base, so the two functions below stand
// in for them: at the merge `exec_attach` keeps only its `attach` line and `exec_rebind`
// becomes `exec_ui::rebind`; everything else in them goes. The console host calls nothing else
// of the SQL context.

/// Attaches the run UI to console `console` (once, when its editor is made).
fn exec_attach(console: ConsoleId, editor: &Entity<EditorView>, window: &mut Window, cx: &mut App) {
    crate::exec_ui::attach(console, editor, window, cx);
    // Stand-in until Task 33: the dialect's defaults, no remembered kind, the display.
    let Some(state) = Consoles::get(cx, console).cloned() else {
        return;
    };
    let Some(profile) = state.profile.and_then(|p| profile_of(cx, p)) else {
        return;
    };
    let doc = editor.read(cx).doc();
    let ctx = dialect_ctx(profile.engine, None, None);
    if let Some(service) = &state.service {
        service.set_sql_context(doc, ctx, effective_flavor(&ctx, ClientFlavor::None), cx);
        service.set_remembered_kind(doc, None, cx);
    }
    Consoles::update(cx, console, |s| s.sql = Some(ctx));
    if let Some(results) = &state.results {
        set_result_display(results, profile.engine, cx);
    }
}

/// The console's session changed (a new one taken).
fn exec_rebind(console: ConsoleId, window: &mut Window, cx: &mut App) {
    let _ = window;
    // Stand-in until Task 33: the connected server's version in the SQL context.
    let Some(state) = Consoles::get(cx, console).cloned() else {
        return;
    };
    let Some(profile) = state.profile.and_then(|p| profile_of(cx, p)) else {
        return;
    };
    let version = cx
        .try_global::<ConnectionsModel>()
        .and_then(|m| match m.live(profile.id) {
            LiveState::Connected(info) => info.server.version.clone(),
            _ => None,
        });
    let ctx = dialect_ctx(profile.engine, version.as_ref(), None);
    if let (Some(service), Some(editor)) = (&state.service, &state.editor) {
        let doc = editor.read(cx).doc();
        service.set_sql_context(doc, ctx, effective_flavor(&ctx, ClientFlavor::None), cx);
    }
    Consoles::update(cx, console, |s| s.sql = Some(ctx));
}
// ## end m3a-t32: exec_ui attach/rebind (T33)

/// The results display of a console on `engine`: the format locale and the zone of the
/// localization, the zone loaded on the core runtime (ledger T26 note).
fn set_result_display(
    results: &Entity<ConsoleResults>,
    engine: hacivat_proto::DbmsKind,
    cx: &mut App,
) {
    let Some(format) = cx
        .try_global::<crate::l10n_global::L10nGlobal>()
        .map(|g| g.l10n.snapshot().format.clone())
    else {
        return;
    };
    let Some(rt) = cx.try_global::<ConnectionsModel>().map(|m| m.runtime()) else {
        return;
    };
    let (tx, rx) = tokio::sync::oneshot::channel();
    let tz = format.time_zone.clone();
    rt.spawn_blocking(move || {
        let _ = tx.send(hacivat_values::valfmt::DisplayZone::load(Some(&tz)));
    });
    let results = results.downgrade();
    cx.spawn(async move |cx| {
        if let Ok(zone) = rx.await {
            let display =
                crate::results::adapter::ResultDisplay::new(&format.format_locale, zone, engine);
            let _ = results.update(cx, |r, cx| r.set_display(display, cx));
        }
    })
    .detach();
}

/// The console whose editor has the focus, else the one last focused.
pub fn focused_console(window: &Window, cx: &App) -> Option<ConsoleId> {
    let by_editor = Consoles::ids(cx).into_iter().find(|id| {
        Consoles::get(cx, *id)
            .and_then(|s| s.editor.as_ref())
            .is_some_and(|editor| {
                hacivat_ui_core::gpui_surface::Focusable::focus_handle(editor.read(cx), cx)
                    .contains_focused(window, cx)
            })
    });
    by_editor.or_else(|| Consoles::focused(cx))
}

/// The `console_binding` hook: the profile a new console tab binds to (R-M3-37): the capsule's
/// profile while it is connected. The new tab's console starts empty.
pub fn console_binding(cx: &App) -> Option<ProfileId> {
    let m = cx.try_global::<ConnectionsModel>()?;
    let profile = current_profile(cx)?;
    if !matches!(m.live(profile.id), LiveState::Connected(_)) {
        return None;
    }
    if let Some(hosts) = cx.try_global::<Hosts>() {
        hosts.fresh.borrow_mut().push((profile.id, None));
    }
    Some(profile.id)
}

/// The status bar's editor segment of `console` ("Ln 14, Col 49"), its own entity (R-M3-11;
/// Task 34 places it).
pub fn editor_segment(console: ConsoleId, cx: &App) -> Option<AnyView> {
    let tab = Consoles::get(cx, console)?.tab;
    let host = cx.try_global::<Hosts>()?.by_tab.get(&tab)?.host.upgrade()?;
    host.read(cx).segment.clone().map(Into::into)
}

/// The host of console tab `tab` (tests, the shots fixtures).
pub fn host_of(cx: &App, tab: TabId) -> Option<Entity<ConsoleHost>> {
    cx.try_global::<Hosts>()?.by_tab.get(&tab)?.host.upgrade()
}

// ---- shots fixtures -------------------------------------------------------------------------

/// The toolbar states a shot shows without a live run or a full budget (feature `shots`).
#[derive(Debug, Clone, Default)]
struct FixtureUx {
    run: Option<RunUx>,
    waiting: Option<(u8, u8)>,
    read_only: Option<SharedString>,
}

/// Shots fixtures for NOTES surfaces 1, 2, 6 and 7 (Task 35 renders them): a console bound to
/// a saved profile with given text (a 5 MB text opens in reduced mode: the T1 banner and the
/// segment's "Reduced mode"), and the toolbar's run, waiting and lock states.
#[cfg(any(feature = "shots", feature = "test-support"))]
pub mod fixtures {
    use super::*;

    /// Opens a console tab bound to `profile` in the focused pane, over `text` (autosave off,
    /// nothing read from disk); its tab. The console is built when the tab first draws; the
    /// `show_*` fixtures apply after that. The profile's connect is the caller's.
    pub fn open(
        profile: ProfileId,
        text: &str,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<TabId> {
        let ws = window.root::<Workspace>().flatten()?;
        cx.default_global::<Hosts>()
            .fresh
            .borrow_mut()
            .push((profile, Some(text.to_string())));
        ws.update(cx, |ws, cx| ws.open_console_tab(Some(profile), window, cx))
    }

    /// Shows `run` in console `tab`'s toolbar (Running with its clock, Cancelling).
    pub fn show_run(tab: TabId, run: RunUx, cx: &mut App) {
        with_fixture(tab, cx, |f| f.run = Some(run));
    }

    /// Shows the waiting state, `in_use` of `max`.
    pub fn show_waiting(tab: TabId, in_use: u8, max: u8, cx: &mut App) {
        with_fixture(tab, cx, |f| f.waiting = Some((in_use, max)));
    }

    /// Shows the lock chip of a read-only connection with `access` (an M2b access label).
    pub fn show_lock(tab: TabId, access: AccessLabel, cx: &mut App) {
        let label = access_label(cx, access);
        with_fixture(tab, cx, |f| f.read_only = Some(label.clone()));
    }

    fn with_fixture(tab: TabId, cx: &mut App, f: impl FnOnce(&mut FixtureUx)) {
        if let Some(host) = host_of(cx, tab) {
            host.update(cx, |h, cx| {
                f(h.fixture.get_or_insert_with(FixtureUx::default));
                cx.notify();
            });
        }
    }
}

// ---- the production explanation -------------------------------------------------------------

/// "Why this console is read-only" (NOTES ruling 6): a 300 px popover with the title, why, and
/// what still works.
struct ProdPopover {
    focus: hacivat_ui_core::gpui_surface::FocusHandle,
    focused_once: bool,
}

impl ProdPopover {
    fn new(cx: &mut Context<Self>) -> Self {
        ProdPopover {
            focus: cx.focus_handle(),
            focused_once: false,
        }
    }
}

impl Render for ProdPopover {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.focused_once {
            self.focused_once = true;
            window.focus(&self.focus, cx);
        }
        let theme = *Theme::get(cx);
        let t = theme.tokens;
        let c = &t.colors;
        let title = tr(cx, ids::hv::CONSOLE_PROD_POP_TITLE, NoArgs);
        popover_surface(&theme)
            .id("console-prod-popover")
            .track_focus(&self.focus)
            .w(px(300.0))
            .p(px(t.spacing.lg))
            .flex()
            .flex_col()
            .gap(px(t.spacing.sm))
            .child(
                hacivat_ui::text::Label::new(
                    title,
                    TextSource::Fluent,
                    t.type_ramp.body_strong,
                    c.text.to_hsla(),
                )
                .id("console-prod-pop-title"),
            )
            .child(paragraph(
                tr(cx, ids::hv::CONSOLE_PROD_POP_BODY, NoArgs),
                TextSource::Fluent,
                t.type_ramp.body,
                c.text.to_hsla(),
            ))
            .child(paragraph(
                tr(cx, ids::hv::CONSOLE_PROD_POP_READS, NoArgs),
                TextSource::Fluent,
                t.type_ramp.body,
                c.text_2.to_hsla(),
            ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_busy_session_runs_unless_it_is_the_idle_release() {
        let mut snap = SessionSnapshot {
            state: SessionState::Idle,
            tx: Default::default(),
            schema: None,
            cancel: Default::default(),
            read_only: true,
            access: AccessLabel::ReadOnlyGuarded,
            generation: 0,
            cursor: None,
        };
        assert_eq!(SessionUx::of(&snap).busy, None);
        snap.state = SessionState::Busy {
            req: 0,
            since: std::time::Instant::now(),
        };
        assert_eq!(SessionUx::of(&snap).busy, None, "the idle cursor release");
        snap.state = SessionState::Busy {
            req: 4,
            since: std::time::Instant::now(),
        };
        assert_eq!(SessionUx::of(&snap).busy, Some(4));
        snap.state = SessionState::Cancelling {
            req: 4,
            since: std::time::Instant::now(),
        };
        let ux = SessionUx::of(&snap);
        assert!(ux.cancelling && ux.read_only);
    }
}
