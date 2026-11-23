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
//! read-write where `effective_read_only` now says read-only (the profile was marked read-only
//! while connected) is dropped from every console of the profile at once, and the profile
//! reconnects, so its new sessions open read-only. The console never takes such a session
//! either (`acquire`).
//!
//! ## m3b-t26-prod-unlock (R-M3-78): a production tag no longer makes a console read-only, so
//! retagging a profile as production while its console is open reconnects nothing: the gate
//! reads the live profile's environment on every Run (`orchestrator::conn_safety`), so the
//! confirmations apply from the next Run; the session keeps its transaction mode (D-036's
//! Manual default applies to new sessions). Marking a profile read-only keeps the reconnect
//! above (`pending_read_only`).
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
//!
//! ## m4b-t14-tabs (named exception: the script tab's host, R-M4b-4, R-M4b-25, R-M4b-30)
//! A script tab's host ([`build_script`]) is this host with a [`ScriptDoc`] beside the editor:
//! its text comes from the file (read off the UI thread while "Opening…" shows), it never
//! autosaves into the consoles folder (R-M4b-5), and its binding is the one its handle names:
//! - **never bound by a connect** (R-M3-37's auto-binding skips it, R-M4b-30), and a tab whose
//!   profile is gone gets no binding at all;
//! - **its session** (C-M4-5, R-M4b-27): PostgreSQL and SQLite share the profile's sessions as a
//!   console does; a MySQL/MariaDB tab always opens a session of its own (slot 0, never the
//!   profile's default one, which consoles and the explorer read) and, under a database other
//!   than the profile's, switches it with `USE` before Run is enabled (`ScriptDoc::session_taken`);
//!   a single-session profile that hands back its one session is refused for such a tab.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use hacivat_l10n::{NoArgs, Text, ids};
use hacivat_proto::{AccessLabel, Department, ProfileId};
use hacivat_session::LiveState;
use hacivat_session::session::{
    ClientId, ClosePolicy, SessionHandle, SessionKind, SessionSnapshot, SessionSpec, SessionState,
    effective_read_only,
};
use hacivat_sql_editor::SqlLanguageService;
use hacivat_ui::focus_home::{HomeSlot, focus_home};
use hacivat_ui::panes::TabId;
use hacivat_ui::toast::{Toast, ToastKind, ToastQueue};
use hacivat_ui_core::gpui_surface::{
    AnyView, App, AppContext as _, Context, Entity, Focusable as _, Global,
    InteractiveElement as _, IntoElement, ParentElement as _, Render, SharedString,
    StyleRefinement, Styled as _, Subscription, Task, WeakEntity, Window, div, px,
};
use hacivat_ui_core::theme::Theme;
use hacivat_ui_editor::large_file::SizeText;
use hacivat_ui_editor::{
    CaretBlink, DocContext, DocId, EDITOR_CONTEXT, EditorConfig, EditorEvent, EditorView,
    LanguageService, Tier, a11y,
};
use std::time::Instant;

use crate::connections::capsule::{access_label, current_profile, env_kind};
use crate::connections::model::ConnectionsModel;
use crate::console::autosave::{self, Autosave, RESTORE_MAX_BYTES, Restored};
use crate::console::decor::{console_title, profile_of};
use crate::console::registry::{ConsoleId, ConsoleState, Consoles};
use crate::console::status::EditorSegment;
// ## m3c-t41-chrome
use crate::console::status::{SegmentCell, SegmentPart};
use crate::console::toolbar::{self, RunUx, ToolbarActions, ToolbarModel};
use crate::l10n_global::tr;
use crate::results::ConsoleResults;
use crate::shell::{AppState, TabKind, Workspace, register_focus_context};
// ## m4b-t14-tabs (named exception)
use crate::scripts::tabs::{ScriptDoc, ScriptDocEvent, ScriptTabId};

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
    /// Profiles whose read-only reconnect waits for a user transaction to end.
    pending: HashSet<ProfileId>,
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
    // ## m3a-f5-demo (DEMO-1)
    cx.observe_global::<ConnectionsModel>(follow_explorer_connects)
        .detach();
    cx.on_app_quit(|cx| {
        save_all_now(cx);
        async {}
    })
    .detach();
}

// ## m3a-f5-demo (DEMO-1): a connect from the Explorer ends at a console of that profile.

/// Profiles the user connected from the Explorer (double-click, Connect) whose connect has not
/// ended yet.
#[derive(Default)]
struct ExplorerConnects(HashMap<ProfileId, bool>);

impl Global for ExplorerConnects {}

/// The user asked the Explorer to connect `profile` (`connection.toggle_connect`): once it is
/// connected, a console of that profile gets the focus ([`console_for_connect`]).
pub(crate) fn note_explorer_connect(cx: &mut App, profile: ProfileId) {
    cx.default_global::<ExplorerConnects>()
        .0
        .insert(profile, false);
}

/// The user disconnected or cancelled `profile`: its pending Explorer connect is dropped.
pub(crate) fn forget_explorer_connect(cx: &mut App, profile: ProfileId) {
    if cx.has_global::<ExplorerConnects>() {
        cx.global_mut::<ExplorerConnects>().0.remove(&profile);
    }
}

/// The connections changed: an Explorer connect that connected is followed by its console
/// (after every host has seen the change, so an unbound focused console binds first). One that
/// failed, or that went back to disconnected after it was seen connecting (cancelled from
/// anywhere: the capsule, the dialog, a delete; review m-8), or whose profile is gone, is
/// dropped.
fn follow_explorer_connects(cx: &mut App) {
    let Some(wanted) = cx
        .try_global::<ExplorerConnects>()
        .filter(|w| !w.0.is_empty())
        .map(|w| w.0.iter().map(|(p, seen)| (*p, *seen)).collect::<Vec<_>>())
    else {
        return;
    };
    let Some(m) = cx.try_global::<ConnectionsModel>() else {
        return;
    };
    let states: Vec<(ProfileId, bool, Option<LiveState>)> = wanted
        .into_iter()
        .map(|(p, seen)| (p, seen, m.get(p).map(|_| m.live(p))))
        .collect();
    for (profile, seen_connecting, state) in states {
        match state {
            Some(LiveState::Connected(_)) => {
                forget_explorer_connect(cx, profile);
                cx.defer(move |cx| console_for_connect(profile, cx));
            }
            Some(LiveState::Connecting(_) | LiveState::Reconnecting { .. }) => {
                if let Some(seen) = cx.global_mut::<ExplorerConnects>().0.get_mut(&profile) {
                    *seen = true;
                }
            }
            // Not started yet (the state the intent is noted in).
            Some(LiveState::Disconnected) if !seen_connecting => {}
            Some(LiveState::Disconnected | LiveState::Failed(_) | LiveState::Lost { .. })
            | None => forget_explorer_connect(cx, profile),
        }
    }
}

/// `profile` connected from the Explorer (the ruling on PO demo finding 1): the focused pane's
/// console stays when it is bound to `profile` (an unbound one has just bound to it, R-M3-37);
/// else a console of `profile` that is open is activated and focused; else a new console bound
/// to it opens, active and focused.
fn console_for_connect(profile: ProfileId, cx: &mut App) {
    let Some(window) = cx
        .active_window()
        .into_iter()
        .chain(cx.windows())
        .find_map(|w| w.downcast::<Workspace>())
    else {
        return;
    };
    let _ = window.update(cx, |ws, window, cx| {
        let active = ws.active_tab();
        let bound: Vec<TabId> = cx
            .try_global::<Hosts>()
            .map(|h| {
                let mut tabs: Vec<TabId> = h
                    .by_tab
                    .iter()
                    .filter(|(_, e)| {
                        e.binding.is_some_and(|(p, _)| p == profile) && e.host.upgrade().is_some()
                    })
                    .map(|(tab, _)| *tab)
                    .collect();
                tabs.sort_by_key(|t| t.0);
                tabs
            })
            .unwrap_or_default();
        if active.is_some_and(|a| bound.contains(&a)) {
            return;
        }
        // An unbound focused console that did not bind on its own (it never saw the profile
        // disconnected) binds now: R-M3-37's rule comes first.
        if let Some(tab) = active
            && matches!(
                ws.tab_kind(tab),
                Some(TabKind::Console { profile: None, .. })
            )
            && let Some(host) = host_of(cx, tab)
        {
            // After this update: the binding updates the workspace.
            window.defer(cx, move |window, cx| {
                host.update(cx, |h, cx| {
                    if matches!(h.mode, Mode::Unbound) {
                        h.bind_fresh(profile, window, cx);
                    }
                });
            });
            return;
        }
        if let Some(tab) = bound.into_iter().find(|t| ws.tab_kind(*t).is_some()) {
            ws.focus_tab(tab, cx);
            let editor = Consoles::by_tab(cx, tab)
                .and_then(|id| Consoles::get(cx, id))
                .and_then(|s| s.editor.clone());
            if let Some(editor) = editor {
                let focus = editor.focus_handle(cx);
                window.defer(cx, move |window, cx| window.focus(&focus, cx));
            }
            return;
        }
        // A new console starts empty and takes the focus (as `console_binding`'s).
        if let Some(hosts) = cx.try_global::<Hosts>() {
            hosts.fresh.borrow_mut().push((profile, None));
        }
        ws.open_console_tab(Some(profile), window, cx);
    });
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

/// Whether a session of `profile` must be read-only now (## m3b-t26-prod-unlock: the profile's
/// own read-only flag; its environment no longer decides, R-M3-78).
fn must_be_read_only(cx: &App, profile: ProfileId) -> bool {
    profile_of(cx, profile).is_some_and(|p| effective_read_only(&p))
}

/// The read-only rule (R-M3-6, ledger T32 note): a console session that is read-write where
/// the profile now says read-only does not survive. Every console of that profile drops its
/// session at once, and the profile reconnects (its new sessions open read-only).
fn enforce_read_only(cx: &mut App) {
    // A pending reconnect ends with its connection (fix round 1, M14): the next pending
    // window of that profile says so again.
    let connected = live_kinds(cx);
    cx.default_global::<Hosts>()
        .pending
        .retain(|p| connected.get(p) == Some(&LiveKind::Connected));
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
///
/// A console whose session holds the user's open transaction asks first, once per pending
/// window (Task 33's prompt: "Commit and continue" / "Roll back and continue" / Cancel). The
/// reconnect runs again after a Commit or Roll back; on Cancel nothing is released and the
/// console stays pending read-only (C1: Run sends nothing on it); the rule also runs again when
/// that transaction ends on its own (the host's session watch).
fn reconnect_read_only(cx: &mut App, profile: ProfileId) {
    let holding = hosts_of(cx, profile).into_iter().find_map(|host| {
        let console = host.read(cx).console()?;
        let session = Consoles::get(cx, console)?.session.clone()?;
        (!can_release_quietly(&session)).then_some(console)
    });
    if let Some(console) = holding {
        let first = cx.default_global::<Hosts>().pending.insert(profile);
        if first {
            ask_release(console, cx, move |cx| reconnect_read_only(cx, profile));
        }
        return;
    }
    for host in hosts_of(cx, profile) {
        host.update(cx, |h, cx| h.release_session(cx));
    }
    cx.default_global::<Hosts>().pending.remove(&profile);
    ConnectionsModel::disconnect(cx, profile);
    ConnectionsModel::connect(cx, profile, None);
}

// ## m3a-t32: release gate (T33 confirm_release)
/// Asks before console `console` lets go of its session (a rebind, a swap, the read-only
/// reconnect, a Disconnect or Delete of its profile): Task 33's
/// `close_prompt::confirm_release`. With no user transaction `then` runs now and this answers
/// `true`; with one, the prompt asks ("Commit and continue" / "Roll back and continue" / Cancel)
/// and `then` runs only after the transaction ended (`false` now); Cancel runs nothing, so
/// nothing is released. A session is never dropped with a silent rollback. (Tab close and quit
/// ask through Task 33's own guards.)
fn ask_release(console: ConsoleId, cx: &mut App, then: impl FnOnce(&mut App) + 'static) -> bool {
    crate::exec_ui::close_prompt::confirm_release(console, cx, then)
        == crate::shell::CloseDecision::Proceed
}

/// Whether `session` can go without asking: it ended (nothing to lose), or it holds no user
/// transaction. The paths that may not prompt (the profile already disconnected, a session
/// ended under the console) release only then.
fn can_release_quietly(session: &SessionHandle) -> bool {
    let snap = session.state().borrow().clone();
    matches!(
        snap.state,
        SessionState::Disconnected | SessionState::Broken(_)
    ) || !user_tx_open(&snap)
}

/// A Disconnect or Delete of `profile` asks first (fix round 1, C2): when a console of ours
/// holds the user's open transaction on one of the profile's sessions, Task 33's prompt asks and
/// `then` (the Disconnect or Delete again) runs after Commit or Roll back; `true` means the
/// caller does nothing now. The session manager's close would roll it back silently.
pub fn release_refused(
    cx: &mut App,
    profile: ProfileId,
    then: impl FnOnce(&mut App) + 'static,
) -> bool {
    // The holder as Task 33 sees it (its prompt asks about that transaction; fix round 2, M15).
    let holding = hosts_of(cx, profile).into_iter().find_map(|host| {
        let console = host.read(cx).console()?;
        crate::exec_ui::tx::user_tx(console, cx).map(|_| console)
    });
    let Some(console) = holding else {
        return false;
    };
    if crate::exec_ui::close_prompt::prompt_open(cx) {
        // Another question is open: Task 33 would drop `then` without asking. Never silently:
        // the user is told to answer it first and try again.
        ToastQueue::push(
            cx,
            Toast::new(
                ToastKind::Info,
                Rc::new(|cx: &App| tr(cx, ids::hv::CONSOLE_RELEASE_BUSY, NoArgs)),
            ),
        );
        return true;
    }
    // `then` runs now (Proceed) or after Commit or Roll back: either way the caller is done.
    ask_release(console, cx, then);
    true
}

/// The user's own transaction is open (or failed) on the session.
fn user_tx_open(snap: &SessionSnapshot) -> bool {
    snap.tx.owner == hacivat_session::session::TxOwner::User
        && snap.tx.status != hacivat_driver::TxStatus::Idle
}
// ## end m3a-t32: release gate

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
    /// The user's own transaction is open (or failed).
    user_tx: bool,
    /// The session ended (disconnected or broken).
    ended: bool,
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
            user_tx: user_tx_open(snap),
            ended: matches!(
                snap.state,
                SessionState::Disconnected | SessionState::Broken(_)
            ),
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
    // ## m3c-t41-chrome: one status bar cell per part of the segment.
    segment_cells: Vec<Entity<SegmentCell>>,
    autosave: Option<Autosave>,
    autosave_task: Option<Task<()>>,
    /// The console's session is its own (not the profile's default session): only such a
    /// session is ever closed by the console (fix round 1, I4).
    own_session: bool,
    /// The profile must be read-only but the session is still read-write (C1).
    pending_ro: bool,
    /// The editor's failed paints already logged.
    paint_errors: u64,
    /// The toolbar's state forced by a shots fixture (feature `shots`).
    fixture: Option<FixtureUx>,
    /// The workspace's focused pane when the host last looked.
    pane: Option<hacivat_ui::panes::PaneId>,
    // ## m4-t33d-fr1 (ruling R-T33d-5): the console is a focus home; its primary is the editor.
    home: HomeSlot,
    // ## m4b-t14-tabs (named exception): a script tab's file side, and the home of its
    // not-yet-shown states.
    script: Option<Entity<ScriptDoc>>,
    script_home: HomeSlot,
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
    cx.new(|cx| ConsoleHost::new(tab, n, profile, None, window, cx))
        .into()
}

// ## m4b-t14-tabs (named exception)
/// The view of script tab `tab`, bound for its life as handle `script` names, built once.
pub fn build_script(tab: TabId, script: ScriptTabId, window: &mut Window, cx: &mut App) -> AnyView {
    let n = crate::scripts::tabs::client_number(script);
    cx.new(|cx| ConsoleHost::new(tab, n, None, Some(script), window, cx))
        .into()
}

impl ConsoleHost {
    fn new(
        tab: TabId,
        n: u32,
        profile: Option<ProfileId>,
        // ## m4b-t14-tabs (named exception)
        script: Option<ScriptTabId>,
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
            segment_cells: Vec::new(),
            autosave: None,
            autosave_task: None,
            paint_errors: 0,
            own_session: false,
            pending_ro: false,
            fixture: None,
            pane: None,
            home: HomeSlot::new(),
            // ## m4b-t14-tabs
            script: None,
            script_home: HomeSlot::new(),
            _subscriptions: subscriptions,
        };
        // A binding the hook handed out is for the tab built in this frame; whatever no host
        // took by the end of it is stale (fix round 1, M10).
        cx.defer(|cx| {
            if let Some(h) = cx.try_global::<Hosts>() {
                h.fresh.borrow_mut().clear();
            }
        });
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
        // ## m4b-t14-tabs (named exception): a script tab's host reads its file first.
        if let Some(script) = script {
            this.open_script(script, window, cx);
        } else if let Some(console) = registered {
            this.adopt(console, cx);
            // ## m3c-t33-find (named exception): the registrant's editor gets its find bar.
            if let Some(editor) = this.editor().cloned() {
                crate::console::find_bar::attach(console, &editor, window, cx);
            }
        } else if let Some(profile) = profile {
            this.set_binding(profile, cx);
            // ## m4b-t18-run-as-script fix round 1 (named exception; I3, R-T14-3): a preview
            // pasted into a MySQL script tab takes a session of its own (slot 0), as the tab.
            if crate::scripts::run_as_script::pending_wants_own_session(tab, cx) {
                this.binding = Some((profile, 0));
                if let Some(e) = cx.default_global::<Hosts>().by_tab.get_mut(&tab) {
                    e.binding = this.binding;
                }
            }
            match take_fresh(cx, profile) {
                Some(None) => this.open_editor(String::new(), Some(None), true, window, cx),
                Some(Some(text)) => {
                    this.open_editor(text, None, false, window, cx);
                    // ## m4b-t18-run-as-script (named exception): a pasted preview's tab.
                    if let (Some(console), Some(editor)) = (this.console(), this.editor().cloned())
                    {
                        crate::scripts::run_as_script::adopt_pending(tab, console, &editor, cx);
                    }
                }
                None => this.restore(window, cx),
            }
            // ## m4b-t18-run-as-script fix round 1 (m6): a pasted text not adopted goes.
            crate::scripts::run_as_script::forget_pending(tab, cx);
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
        self.set_segment(&editor, cx);
        self.mode = Mode::Ready {
            console,
            editor,
            owned: false,
        };
    }

    // ## m4b-t14-tabs (named exception)
    /// Starts script tab `script`: its doc, its binding as its handle names it (none when the
    /// profile is gone or the tab may not run there), then the file's read.
    fn open_script(&mut self, script: ScriptTabId, window: &mut Window, cx: &mut Context<Self>) {
        let tab = self.tab;
        let doc = cx.new(|cx| ScriptDoc::new(script, tab, cx));
        self._subscriptions
            .push(cx.subscribe_in(&doc, window, Self::on_script_doc));
        let (profile, plan) = {
            let d = doc.read(cx);
            (d.spec(cx).and_then(|t| t.profile), d.plan.clone())
        };
        if let Some(p) = profile.filter(|p| profile_of(cx, *p).is_some()) {
            match plan {
                Ok(crate::scripts::open::SessionPlan::Shared) => self.set_binding(p, cx),
                Ok(crate::scripts::open::SessionPlan::Own { .. }) => {
                    // Slot 0: a session of its own, never the profile's default one.
                    self.binding = Some((p, 0));
                    if let Some(e) = cx.default_global::<Hosts>().by_tab.get_mut(&self.tab) {
                        e.binding = self.binding;
                    }
                }
                // Refused where it is bound: no binding, nothing runs.
                Err(_) => {}
            }
        }
        self.script = Some(doc.clone());
        self.mode = Mode::Loading;
        doc.update(cx, |d, cx| d.start_load(None, cx));
    }

    /// The script doc's events: the file was read (the editor opens over it), or something the
    /// host draws changed.
    fn on_script_doc(
        &mut self,
        doc: &Entity<ScriptDoc>,
        event: &ScriptDocEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            ScriptDocEvent::Read(opened) => {
                if !matches!(self.mode, Mode::Loading) {
                    return;
                }
                let opened = (**opened).clone();
                self.open_editor(opened.text.clone(), None, true, window, cx);
                let (Some(console), Some(editor)) = (self.console(), self.editor().cloned()) else {
                    return;
                };
                if opened.tier == hacivat_scripts::fileio::SizeTier::Preview {
                    let spec = hacivat_ui_editor::PreviewSpec {
                        shown_bytes: opened.text.len() as u64,
                        total_bytes: opened.total,
                    };
                    editor.update(cx, |e, cx| e.set_preview(spec, cx));
                }
                doc.update(cx, |d, cx| d.attach(console, editor, opened, cx));
                // A session taken before the doc knew its console.
                if let Some(session) = Consoles::get(cx, console).and_then(|s| s.session.clone()) {
                    doc.update(cx, |d, cx| d.session_taken(session, cx));
                }
                cx.notify();
            }
            ScriptDocEvent::Changed => cx.notify(),
        }
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

    // ## m3c-t41-chrome
    /// Gives the console's results the format locale and time zone again when they changed
    /// from what the attach or the last refresh applied (a settings change refreshes the
    /// windows).
    fn follow_display(&mut self, cx: &mut Context<Self>) {
        let Some(console) = self.console() else {
            return;
        };
        let Some(format) = cx
            .try_global::<crate::l10n_global::L10nGlobal>()
            .map(|g| g.l10n.snapshot().format.clone())
        else {
            return;
        };
        let key = (format.format_locale.clone(), format.time_zone.clone());
        // ## fix round 1 (m8): against what the attach (or the last refresh) applied, so a
        // change made before this console's first frame is followed too.
        match crate::exec_ui::orchestrator::display_applied(console, cx) {
            Some(applied) if applied != key => {
                cx.defer(move |cx| crate::exec_ui::orchestrator::refresh_display(console, cx));
            }
            _ => {}
        }
    }

    // ## m3c-t41-chrome
    /// The editor's status segment and a status bar cell for each of its parts; and the
    /// placeholder an empty console shows ("Write SQL here. ⌘↵ runs the statement at the
    /// caret.", design/m3a-query surface 1, m3b-safe surface 24).
    fn set_segment(&mut self, editor: &Entity<EditorView>, cx: &mut Context<Self>) {
        let placeholder: hacivat_ui::text::TextFn = Rc::new(|cx: &App| {
            let key =
                crate::shell::keymap_state::shortcut_label(cx, "editor.run").unwrap_or_default();
            tr(
                cx,
                ids::hv::EDITOR_PLACEHOLDER,
                ids::hv::HvEditorPlaceholder {
                    key: hacivat_l10n::Text(key),
                },
            )
        });
        editor.update(cx, |e, cx| e.set_placeholder(Some(placeholder), cx));
        let segment = cx.new(|cx| EditorSegment::new(editor, cx));
        self.segment_cells = SegmentPart::ALL
            .into_iter()
            .map(|part| cx.new(|cx| SegmentCell::new(&segment, part, cx)))
            .collect();
        self.segment = Some(segment);
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
            // ## m4b-t14-tabs (named exception, R-M4b-30): a script tab is never bound by a
            // connect.
            Mode::Unbound | Mode::Loading if self.script.is_some() => {}
            Mode::Ready { owned: true, .. } if self.script.is_some() && self.binding.is_none() => {}
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
            Mode::Ready { owned: true, .. } => {
                self.sync_session(window, cx);
                self.update_pending(cx);
            }
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
                cx.background_spawn(async move {
                    rx.await.unwrap_or(Restored::Unreadable { kept: None })
                })
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
            Restored::TooLarge { bytes, kept } => {
                // R-M3-49: not opened. The file was moved aside unchanged (fix round 1, I6),
                // so the console saves its new text as usual; were it still in place, autosave
                // stays off rather than write over it.
                let name = self.title(cx);
                let (size, limit) = (size_text(cx, bytes), size_text(cx, RESTORE_MAX_BYTES));
                let file = kept_name(kept.as_deref(), &path);
                let body: hacivat_ui::text::TextFn = Rc::new(move |cx: &App| {
                    tr(
                        cx,
                        ids::hv::CONSOLE_RESTORE_REFUSED_BODY,
                        ids::hv::HvConsoleRestoreRefusedBody {
                            size: Text(size.to_string()),
                            limit: Text(limit.to_string()),
                            file: Text(file.clone()),
                        },
                    )
                });
                restore_toast(name, body, cx);
                tracing::info!(path = %path.display(), bytes, "console not restored: too large");
                let save = kept.is_some().then_some(None);
                self.open_editor(String::new(), save, false, window, cx);
            }
            Restored::Unreadable { kept } => {
                let name = self.title(cx);
                let file = kept_name(kept.as_deref(), &path);
                let body: hacivat_ui::text::TextFn = Rc::new(move |cx: &App| {
                    tr(
                        cx,
                        ids::hv::CONSOLE_RESTORE_UNREADABLE_BODY,
                        ids::hv::HvConsoleRestoreUnreadableBody {
                            file: Text(file.clone()),
                        },
                    )
                });
                restore_toast(name, body, cx);
                tracing::warn!(path = %path.display(), "console not restored: unreadable");
                let save = kept.is_some().then_some(None);
                self.open_editor(String::new(), save, false, window, cx);
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
        // ## m4b-t14-tabs (named exception): a script tab opens its editor unbound too (it
        // then runs nothing); a console needs its binding.
        let binding = self.binding;
        if binding.is_none() && self.script.is_none() {
            return;
        }
        let slot = binding.map_or(0, |(_, slot)| slot);
        let profile = binding.and_then(|(p, _)| profile_of(cx, p));
        let doc = DocId(u64::from(self.tab.0));
        let service = SqlLanguageService::new();
        let language: Rc<dyn LanguageService> = service.clone();
        // ## m4b-t14-tabs (named exception): a script editor is named by its tab's title.
        let label = a11y::label(match &self.script {
            Some(doc) => crate::scripts::tabs::tab_title(doc.read(cx).id, cx),
            None => self.title(cx),
        });
        let cfg = editor_config(cx);
        let editor = cx.new(|cx| EditorView::new(doc, &text, cfg, language, label, window, cx));
        let log = hacivat_exec::SessionLog::new();
        let results = cx.new(|_| ConsoleResults::new(log.clone()));
        // ## m4b-t14-tabs (named exception): slot 0 (a script tab's own session) is never the
        // default client.
        let client = if slot == 1 {
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
                profile: binding.map(|(p, _)| p),
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
        // ## m4b-t21-shots (named exception): while a Run-as-script strip shows, this host
        // draws the T2 banner above it (see `render`).
        editor.update(cx, |e, cx| {
            e.set_host_banner(
                Some(std::rc::Rc::new(move |cx: &App| {
                    crate::scripts::run_as_script::strip(console, cx).is_some()
                })),
                cx,
            );
        });
        // ## m3c-t33-find (named exception): the console's find bar.
        crate::console::find_bar::attach(console, &editor, window, cx);
        let focus_handle = editor.focus_handle(cx);
        register_focus_context(cx, focus_handle.clone(), EDITOR_CONTEXT);
        let subscriptions = [
            cx.subscribe_in(&editor, window, Self::on_editor_event),
            cx.on_focus(&focus_handle, window, move |_, _, cx| {
                Consoles::set_focused(cx, Some(console));
            }),
        ];
        self._subscriptions.extend(subscriptions);
        self.set_segment(&editor, cx);
        // ## m4b-t14-tabs (named exception, R-M4b-5): a script tab never autosaves into the
        // consoles folder.
        let autosave = autosave.filter(|_| autosave::allowed(self.script.is_some()));
        self.autosave = autosave.and_then(|on_disk| {
            let paths = cx.try_global::<AppState>()?.paths.clone();
            Some(Autosave::new(
                autosave::console_path(&paths, binding?.0, self.n),
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
        window: &mut Window,
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
            EditorEvent::Changed { .. } => {
                self.schedule_autosave(cx);
                // ## m4b-t14-tabs (named exception): dirty and recovery are the doc's.
                if let Some(doc) = &self.script {
                    doc.update(cx, |d, cx| d.on_edit(cx));
                }
            }
            EditorEvent::TierChanged(tier) => {
                exec_tier(editor, self.console(), *tier, cx);
            }
            // ## m4b-t14-tabs (named exception)
            EditorEvent::Reloaded { version } => {
                if let Some(doc) = &self.script {
                    let version = *version;
                    doc.update(cx, |d, cx| d.on_reloaded(version, cx));
                }
            }
            // ## m4b-t18-run-as-script (named exception; Task 11's question, R-T11-2): a
            // 20-100 MB paste asks; "Paste as preview" opens the document as a preview tab.
            EditorEvent::PasteNeedsConfirm { bytes, total } => {
                crate::scripts::run_as_script::ask_paste(editor, *bytes, *total, window, cx);
            }
            EditorEvent::PasteAsPreview { text } => {
                if let Some(console) = self.console() {
                    crate::scripts::run_as_script::open_pasted(console, text, window, cx);
                }
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
                // A held session that ended, or a first console's that is no longer the
                // profile's default (a reconnect replaced it), is let go (the gate passes an
                // ended session) and a new one taken (fix round 1, I5).
                if let Some(held) = Consoles::get(cx, console).and_then(|s| s.session.clone()) {
                    let ended = matches!(
                        held.state().borrow().state,
                        SessionState::Disconnected | SessionState::Broken(_)
                    );
                    let replaced = !self.own_session
                        && cx
                            .try_global::<ConnectionsModel>()
                            .and_then(|m| m.manager().sessions(profile))
                            .and_then(|s| s.default_session())
                            .is_some_and(|d| d.id() != held.id());
                    if ended || replaced {
                        self.release_session(cx);
                    }
                }
                let has = Consoles::get(cx, console).is_some_and(|s| s.session.is_some());
                if !has && matches!(self.acquire, Acquire::Idle) {
                    self.acquire(profile, window, cx);
                }
            }
            _ => {
                self.release_session(cx);
            }
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
        // ## m4b-t14-tabs (named exception): slot 0 is a script tab's own session.
        if slot == 1 {
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
                        // Said once, then no retry until Run (fix round 1, M13).
                        tracing::info!("console session not opened");
                        this.acquire = Acquire::Cancelled;
                        let name = this.title(cx);
                        ToastQueue::push(
                            cx,
                            Toast::new(
                                ToastKind::Info,
                                Rc::new(move |cx: &App| {
                                    tr(
                                        cx,
                                        ids::hv::CONSOLE_SESSION_FAILED,
                                        ids::hv::HvConsoleSessionFailed {
                                            name: Text(name.to_string()),
                                        },
                                    )
                                }),
                            ),
                        );
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
        // Own = not the profile's default session (in single-session mode `open` hands a later
        // console the default one; fix round 1, I4).
        let default = self.binding.and_then(|(p, _)| {
            cx.try_global::<ConnectionsModel>()?
                .manager()
                .sessions(p)?
                .default_session()
        });
        self.own_session = default.is_none_or(|d| d.id() != handle.id());
        // ## m4b-t14-tabs (named exception, R-M4b-27): a script tab that needs a session of its
        // own never takes the profile's default one (a single-session profile hands it back).
        if !self.own_session
            && self.binding.is_some_and(|(_, slot)| slot == 0)
            && let Some(doc) = self.script.clone()
        {
            // ## m4b-t14-tabs fix round 1 (I3, R-T14-3): never a shared session.
            doc.update(cx, |d, cx| {
                d.show(crate::scripts::tabs::Notice::OneSession, cx);
                d.switch = crate::scripts::tabs::Switch::Failed;
            });
            self.acquire = Acquire::Cancelled;
            return;
        }
        // The registry's client says whose session it is (Task 33 closes a closed console's
        // session only when it is not the default one).
        let client = if self.own_session {
            ClientId(u64::from(self.n))
        } else {
            ClientId::DEFAULT
        };
        Consoles::update(cx, console, |s| {
            s.session = Some(handle);
            s.client = client;
        });
        exec_rebind(console, window, cx);
        self.watch_session(window, cx);
        // ## m4b-t14-tabs (named exception): a MySQL script tab switches its own session.
        if let Some(doc) = self.script.clone()
            && let Some(session) = Consoles::get(cx, console).and_then(|s| s.session.clone())
        {
            doc.update(cx, |d, cx| d.session_taken(session, cx));
        }
    }

    /// Takes the session away from the console; a session of its own closes.
    ///
    /// Every release goes through [`confirm_release`] first: while the user's transaction is
    /// open the console keeps its session (`false`), so nothing is ever rolled back silently.
    fn release_session(&mut self, cx: &mut Context<Self>) -> bool {
        if let Acquire::Waiting { abort, .. } = std::mem::replace(&mut self.acquire, Acquire::Idle)
        {
            abort.abort();
        }
        let Some(console) = self.console() else {
            return true;
        };
        // No prompt here (the callers that may prompt ask first, `ask_release`): a session
        // holding the user's transaction is kept.
        if let Some(session) = Consoles::get(cx, console).and_then(|s| s.session.clone())
            && !can_release_quietly(&session)
        {
            return false;
        }
        let mut released = None;
        Consoles::update(cx, console, |s| released = s.session.take());
        let own = std::mem::take(&mut self.own_session);
        if let (Some(handle), true) = (released, own)
            && let Some(m) = cx.try_global::<ConnectionsModel>()
        {
            // No user transaction is open (the gate said so): this ends only Hacivat's own
            // (a cursor's).
            m.runtime().spawn(handle.close(ClosePolicy::RollBack));
        }
        self.watched = None;
        self.ux = None;
        self.clock = None;
        self.clock_task = None;
        self.update_pending(cx);
        cx.notify();
        true
    }

    /// C1: the profile must be read-only while this console still holds a read-write session
    /// (the user's open transaction holds the reconnect back). Published on the registry, where
    /// every Run entry point reads it; the toolbar says what is true.
    fn update_pending(&mut self, cx: &mut Context<Self>) {
        let pending = self.owned()
            && self.ux.is_some_and(|u| !u.read_only && !u.ended)
            && self.binding.is_some_and(|(p, _)| must_be_read_only(cx, p));
        if pending == self.pending_ro {
            return;
        }
        self.pending_ro = pending;
        if let Some(console) = self.console() {
            crate::exec_ui::set_pending_read_only(console, pending, cx);
        }
        cx.notify();
    }

    /// Watches the console's session for the toolbar (a `tokio::sync::watch` awaited on the
    /// foreground, Q4).
    fn watch_session(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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
        let task = cx.spawn_in(window, async move |this, cx| {
            loop {
                let ux = SessionUx::of(&rx.borrow_and_update());
                if this
                    .update_in(cx, |this, window, cx| this.on_session(ux, window, cx))
                    .is_err()
                {
                    break;
                }
                if rx.changed().await.is_err() {
                    break;
                }
            }
        });
        self.watched = Some((session.id(), task));
    }

    fn on_session(&mut self, ux: SessionUx, window: &mut Window, cx: &mut Context<Self>) {
        if self.ux == Some(ux) {
            return;
        }
        if self.ux.is_some_and(|before| before.user_tx) && !ux.user_tx {
            // A read-only reconnect may have waited for this transaction to end.
            cx.defer(enforce_read_only);
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
        self.update_pending(cx);
        if ux.ended && self.owned() {
            // The session ended under the console: let it go and take a new one if the
            // profile is still connected (I5).
            self.sync_session(window, cx);
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
            // ## m3a-f4-ime: a composition still open here (the button usually took the focus
            // on the mouse-down, and the blur ended it) is ended as the app, so the platform
            // drops its copy too (IME-1); before, this called the platform's own `unmark_text`.
            v.end_composition(window, cx);
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
        // ## m3b-t26-prod-unlock: no production read-only notice (R-M3-6 REVISED ends): a
        // production console shows its environment (the swatch, the hairline, the capsule) and,
        // when the session is read-only because the profile is, the lock chip like any other.
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
            .filter(|ux| ux.read_only && !self.pending_ro)
            .map(|ux| access_label(cx, ux.access));
        let pending_read_only = self.pending_ro.then(|| {
            profile
                .as_ref()
                .map(|p| SharedString::from(p.name.clone()))
                .unwrap_or_default()
        });
        // ## m4b-t14-tabs (named exception): a script tab runs only where it may.
        let script_runs = self
            .script
            .as_ref()
            .is_none_or(|d| d.read(cx).run_enabled(cx));
        ToolbarModel {
            run,
            run_enabled: !matches!(self.acquire, Acquire::Waiting { .. }) && script_runs,
            name,
            env: profile
                .as_ref()
                .and_then(|p| p.environment)
                .map(|tag| env_kind(Some(tag))),
            waiting,
            read_only,
            pending_read_only,
        }
    }

    fn actions(&self, cx: &mut Context<Self>) -> ToolbarActions {
        let me = cx.weak_entity();
        let (a, b, c) = (me.clone(), me.clone(), me);
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
        // ## m4b-t14-tabs (named exception): the last tab of a file lets the service forget it.
        if let Some(doc) = self.script.take() {
            doc.update(cx, |d, cx| d.closed(cx));
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
        // ## m3c-t33-find (named exception)
        crate::console::find_bar::console_closed(console, cx);
        // ## m4b-t18-run-as-script (named exception): its run stops, its pasted text goes.
        crate::scripts::run_as_script::console_closed(console, cx);
        // ## m3c-t41-chrome: the editor's key context leaves with the console (its handle may
        // outlive the tab, held by a pending task).
        let handle = editor.read(cx).focus_handle(cx);
        crate::shell::unregister_focus_context(cx, &handle);
        if owned {
            // The last edits are written now, on the closing thread (fix round 2, M16): a new
            // console that reuses this number starts after it, and its autosave takes the file
            // over, so no late write of this one lands on it.
            if let Some(save) = &self.autosave {
                let snap = editor.read(cx).snapshot();
                if let Err(e) = save.write_now(&snap) {
                    tracing::warn!(error = %e, "console autosave at close failed");
                }
            }
            let own = self.own_session;
            let session = Consoles::get(cx, console).and_then(|s| s.session.clone());
            if let (true, Some(handle)) = (own, session) {
                // The tab close guard (Task 33's prompt) asked before; should a user
                // transaction still be open here, the session is closed refusing to roll it
                // back (it stays open until its profile disconnects), never silently.
                let policy = if can_release_quietly(&handle) {
                    ClosePolicy::RollBack
                } else {
                    ClosePolicy::RefuseIfOpenTx
                };
                if let Some(m) = cx.try_global::<ConnectionsModel>() {
                    m.runtime().spawn(handle.close(policy));
                }
            }
        }
        Consoles::remove(cx, console);
    }
}

impl Render for ConsoleHost {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *Theme::get(cx);
        let editor = match &self.mode {
            // ## m4b-t14-tabs (named exception): a script tab whose text is not shown says why.
            Mode::Unbound | Mode::Loading if self.script.is_some() => {
                let doc = self.script.clone().expect("checked");
                return crate::scripts::disk_bars::render_unloaded(&doc, &self.script_home, cx);
            }
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
        self.watch_session(window, cx);
        // ## m3c-t41-chrome: a regional format or time zone change reaches the results
        // (`ConsoleResults::set_display` again; the console's attach applied the first).
        self.follow_display(cx);
        let model = self.toolbar_model(cx);
        let actions = self.actions(cx);
        // ## m4b-t21-shots: the Run-as-script strip, if any (below).
        let strip = self
            .console()
            .and_then(|c| crate::scripts::run_as_script::strip(c, cx));
        let strip_shown = strip.is_some();
        // ## m3b-t22-tx-ui (named exception): with the console's transaction items.
        let bar = toolbar::build_for(&model, &actions, self.console(), cx);
        let root = div()
            .id(("console", self.tab.0))
            .flex()
            .flex_col()
            .size_full()
            .bg(theme.tokens.colors.panel.to_hsla())
            .child(bar)
            // ## m4b-t14-tabs (named exception): a script tab's notices, under its toolbar.
            // ## m4b-t21-shots fix round 1 (m1): rows that never give up their height.
            .children(
                self.script
                    .as_ref()
                    .and_then(|doc| crate::scripts::disk_bars::render(doc, cx))
                    .map(|n| div().flex_none().child(n)),
            )
            // ## m4b-t18-run-as-script (named exception; NOTES surface 9): the Run-as-script
            // strip, under the notices (never over a trust signal).
            // ## m4b-t21-shots (named exception, T18 concern): and under the T2 banner, as the
            // mock shows: while a strip shows, this host draws the editor's banner above it
            // (`EditorView::set_host_banner`, set at attach).
            .children(strip.as_ref().and_then(|_| {
                editor
                    .clone()
                    .update(cx, |e, cx| e.banner_element(window, cx))
            }))
            .children(strip.map(|s| div().flex_none().child(s)))
            // ## m3c-t33-find (named exception): the find bar, a row under the toolbar
            // (design/m3b-safe surface 16, M3c-9).
            .children(
                self.console()
                    .and_then(|c| crate::console::find_bar::render(c, cx)),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    // ## m4b-t21-shots fix round 1 (m1, named exception): while a strip shows,
                    // the code keeps two lines; the condensed banner's body gives way first.
                    .min_h(if strip_shown {
                        // ## m4b-t21-shots fix round 2 (m-a): with the code's top inset.
                        hacivat_ui_editor::EditorView::code_height(2, cx)
                    } else {
                        px(0.0)
                    })
                    .child(a11y::embed(
                        &editor,
                        StyleRefinement::default().size_full(),
                        window,
                    )),
            )
            // ## m3b-t23-escalation (named exception): the error-policy bar at the foot of the
            // editor tile, above the seam to the results (design/m3b-safe surface 7).
            .children(
                self.console()
                    .and_then(|c| crate::exec_ui::error_bar::render(c, cx)),
            );
        // ## m4-t33d-fr1 (ruling R-T33d-5)
        focus_home(&self.home, editor.focus_handle(cx), root).into_any_element()
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

// ## m4-t24-live (named exception): Open data's console (R-M4-13).
/// The next console tab built for `profile` opens over `text` (as a shots fixture's does:
/// nothing restored, autosave off), not empty.
pub(crate) fn queue_text(profile: ProfileId, text: String, cx: &mut App) {
    cx.default_global::<Hosts>()
        .fresh
        .borrow_mut()
        .push((profile, Some(text)));
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
        // ## m4b-t18-run-as-script (named exception; carries-w1 T11/T18): the host takes the
        // 20-100 MB paste question (`run_as_script::ask_paste`).
        confirm_large_paste: true,
        ..EditorConfig::default()
    }
}

// ## m3a-t32: exec_ui attach/rebind (T33)
// Task 33's `exec_ui::attach` (once per console) and `exec_ui::rebind` (after each session
// take) own the SQL context (its probe, `set_sql_context`), the remembered chooser kind, the
// results display and the editor's `connected` (set after the probe). The console host calls
// nothing else of them.

/// Attaches the run UI to console `console` (once, when its editor is made).
fn exec_attach(console: ConsoleId, editor: &Entity<EditorView>, window: &mut Window, cx: &mut App) {
    crate::exec_ui::attach(console, editor, window, cx);
}

/// The console took a session.
fn exec_rebind(console: ConsoleId, window: &mut Window, cx: &mut App) {
    let _ = window;
    crate::exec_ui::rebind(console, cx);
}

/// The editor's tier changed (`TierChanged`, ledger T29/T32): the language service hears it,
/// with `connected` as Task 33 set it (its SQL context read, the session live).
fn exec_tier(editor: &Entity<EditorView>, console: Option<ConsoleId>, tier: Tier, cx: &mut App) {
    // Task 33's own rule (fix round 2, M18).
    let connected = console.is_some_and(|c| crate::exec_ui::connected(c, cx));
    editor.update(cx, |v, cx| {
        v.set_doc_context(DocContext { connected, tier }, cx)
    });
}
// ## end m3a-t32: exec_ui attach/rebind (T33)

/// The kept file's name as the restore notice shows it (machine text).
fn kept_name(kept: Option<&std::path::Path>, path: &std::path::Path) -> String {
    kept.unwrap_or(path)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default()
}

/// The wide notice of a console that was not reopened.
fn restore_toast(name: SharedString, body: hacivat_ui::text::TextFn, cx: &mut App) {
    let mut toast = Toast::new(ToastKind::Info, body);
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

// ## m3c-t41-chrome (F2 M8; M3c-35)
/// The status bar's cells of `console`'s editor segment: one per part it shows now, left to
/// right ("Reduced mode", "Mixed line endings", the carets, the selected count, Ln/Col).
pub fn editor_segment_cells(console: ConsoleId, cx: &App) -> Vec<(SegmentPart, AnyView)> {
    let Some(tab) = Consoles::get(cx, console).map(|s| s.tab) else {
        return Vec::new();
    };
    let Some(host) = cx
        .try_global::<Hosts>()
        .and_then(|h| h.by_tab.get(&tab))
        .and_then(|e| e.host.upgrade())
    else {
        return Vec::new();
    };
    let host = host.read(cx);
    let Some(segment) = &host.segment else {
        return Vec::new();
    };
    let shown = segment.read(cx).parts().to_vec();
    host.segment_cells
        .iter()
        .filter_map(|cell| {
            let part = cell.read(cx).part();
            shown.contains(&part).then(|| (part, cell.clone().into()))
        })
        .collect()
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

    // ## m3a-t35-shots (fix round 1, I1; named exception): what the fixtures set, read back
    // so a shot fails when a fixture did nothing.
    /// The toolbar states the fixtures set on console `tab` (`None`: none set).
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct Shown {
        pub run: Option<RunUx>,
        pub waiting: Option<(u8, u8)>,
        pub read_only: Option<SharedString>,
    }

    /// What the fixtures set on console `tab`'s toolbar.
    pub fn shown(tab: TabId, cx: &App) -> Option<Shown> {
        let host = host_of(cx, tab)?;
        let f = host.read(cx).fixture.clone()?;
        Some(Shown {
            run: f.run,
            waiting: f.waiting,
            read_only: f.read_only,
        })
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
