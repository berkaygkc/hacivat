//! The status bar's connection segments (NOTES surface 7, Task 18).
//!
//! The status bar mirrors the capsule from the same source (`capsule::current_profile` and its
//! mirrored live state): the environment segment (`hv-status-env`), then the state or the path
//! with its latency, the encryption fact, the "unsupported" mark and the engine version. With
//! no current connection it shows M1's "No connection".
//!
//! M2b (Task 22): the environment segment carries the capsule's access label ("production ·
//! read-only (enforced by server role)", NOTES M2b surface 6); a reconnect shows its attempt,
//! an idle disconnect "Disconnected (idle)", and an SSH path `SSH via <bastion>` (label ID
//! `status-path`, the capsule's path text).
//!
//! M3a (Task 34): the bar follows the focused console and its active result tab (DESIGN.md
//! "Status Follows The Tab"); see the `m3a-t34-status-follows` block below.

use hacivat_driver::Support;
use hacivat_l10n::{NoArgs, Text, ids};
use hacivat_session::LiveState;
use hacivat_session::profile::Profile;
use hacivat_ui::statusbar::{env_segment, status_label};
use hacivat_ui::text::TextSource;
use hacivat_ui_core::gpui_surface::{AnyElement, App, IntoElement, SharedString};
use hacivat_ui_core::theme::Theme;

use crate::connections::capsule::{
    access_label, current_profile, encryption_key, encryption_label, env_kind, env_word,
    is_read_only, latency_label, path_label, reconnecting_label,
};
use crate::connections::model::ConnectionsModel;
use crate::connections::session_toasts;
use crate::l10n_global::tr;
// ## m3a-t34-status-follows
use std::collections::HashMap;
use std::sync::Arc;

use hacivat_exec::{TxBadge, console_ux};
use hacivat_l10n::{Count, ExactDecimal};
use hacivat_proto::{AccessLabel, EnvTag, ProfileId};
use hacivat_results::{CloseReason, ResultId};
use hacivat_session::session::{Request, SessionHandle, SessionId, SessionSnapshot, SessionState};
use hacivat_ui::button::{Button, ButtonKind};
// ## m3a-f2-visual
use hacivat_ui::give_way::{GiveWayRow, Yield};
use hacivat_ui::icon::{Icon, IconName, IconSize};
use hacivat_ui::panes::TabId;
use hacivat_ui::popover::Tip;
use hacivat_ui::statusbar::status_label_in;
use hacivat_ui::step_trail::run_marker;
use hacivat_ui::text::Label;
use hacivat_ui_core::gpui_surface::{
    Global, InteractiveElement as _, ParentElement as _, StatefulInteractiveElement as _,
    Styled as _, Task, div, px,
};
use hacivat_ui_editor::{EditorView, FocusedEditor};

use crate::console::{ConsoleId, ConsoleState, Consoles};
use crate::results::context::result_context_of;
use crate::results::{CompletenessView, ResultContext};
use crate::shell::workspace::ActiveTab;

/// The access word of a profile without a live connection (`hv-capsule-access-*`).
fn profile_access(cx: &App, read_only: bool) -> SharedString {
    // ## m2a-t21-shots: no second copy of the environment and access words.
    let access = if read_only {
        ids::hv::CAPSULE_ACCESS_READ_ONLY
    } else {
        ids::hv::CAPSULE_ACCESS_READ_WRITE
    };
    tr(cx, access, NoArgs)
}

/// `production · read-only` and the like: the environment word (`hv-conn-env-*`) and the
/// access text in `hv-status-env`.
fn env_text(cx: &App, p: &Profile, access: SharedString) -> SharedString {
    tr(
        cx,
        ids::hv::STATUS_ENV,
        ids::hv::HvStatusEnv {
            env: Text(env_word(cx, p.environment).to_string()),
            access: Text(access.to_string()),
        },
    )
}

// ## m3a-t34-status-follows: the M2 body moved into `connection_parts`, unchanged.
/// The `status_left` hook (see the module docs). While a bound console is followed
/// ([`followed_console`]), the bar describes that console (`console_segments`); otherwise it
/// is M2's, unchanged.
pub fn segments(theme: &Theme, cx: &App) -> Vec<AnyElement> {
    let no_connection = || {
        vec![
            status_label(
                theme,
                tr(cx, ids::hv::STATUS_NO_CONNECTION, NoArgs),
                TextSource::Fluent,
            )
            .into_any_element(),
        ]
    };
    if let (Some(m), Some((console, p))) = (
        cx.try_global::<ConnectionsModel>(),
        followed_bound_console(cx),
    ) {
        return console_segments(theme, cx, m, console, &p);
    }
    let (Some(m), Some(p)) = (cx.try_global::<ConnectionsModel>(), current_profile(cx)) else {
        return no_connection();
    };
    connection_parts(theme, cx, m, &p)
        .into_iter()
        .map(|(_, segment)| segment)
        .collect()
}

/// What an M2 connection segment is, so a console's bar can order and collapse it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Part {
    Env,
    /// "No connection", connecting, reconnecting, failed, idle.
    State,
    Path,
    Latency,
    Encryption,
    Unsupported,
    Version,
}

/// M2's segments of `p` (the environment, then the state or the path, latency, encryption,
/// the "unsupported" mark and the version), each with what it is.
fn connection_parts(
    theme: &Theme,
    cx: &App,
    m: &ConnectionsModel,
    p: &Profile,
) -> Vec<(Part, AnyElement)> {
    let no_connection = || {
        vec![(
            Part::State,
            status_label(
                theme,
                tr(cx, ids::hv::STATUS_NO_CONNECTION, NoArgs),
                TextSource::Fluent,
            )
            .into_any_element(),
        )]
    };
    let kind = env_kind(p.environment);
    // ## m2b-t22-live (fix round 1): every segment after the environment may ellipsize when
    // the bar is full (long locales at the minimum window; the tooltip keeps the whole text).
    let fluent = |text: SharedString| {
        status_label(theme, text, TextSource::Fluent)
            .truncate(true)
            .into_any_element()
    };
    let profile_env = || {
        env_segment(
            theme,
            kind,
            env_text(cx, p, profile_access(cx, p.read_only)),
            p.read_only,
        )
    };
    match m.live(p.id) {
        // ## m2b-t22-live: an idle disconnect keeps the profile (NOTES M2b surface 7).
        LiveState::Disconnected if session_toasts::is_idle(cx, p.id) => vec![
            (Part::Env, profile_env()),
            (Part::State, fluent(tr(cx, ids::hv::CONN_IDLE_CAP, NoArgs))),
        ],
        LiveState::Disconnected => no_connection(),
        LiveState::Connecting(_) => vec![
            (Part::Env, profile_env()),
            (
                Part::State,
                fluent(tr(cx, ids::hv::CAPSULE_CONNECTING, NoArgs)),
            ),
        ],
        // ## m2b-t22-live: the attempt, as the capsule says it.
        LiveState::Reconnecting { attempt, .. } => {
            vec![
                (Part::Env, profile_env()),
                (Part::State, fluent(reconnecting_label(cx, attempt))),
            ]
        }
        LiveState::Failed(_) | LiveState::Lost { .. } => vec![
            (Part::Env, profile_env()),
            (Part::State, fluent(tr(cx, ids::hv::CAPSULE_FAILED, NoArgs))),
        ],
        LiveState::Connected(info) => {
            let read_only = is_read_only(&info);
            // ## m2b-t22-live: the capsule's access label and path (B21, SSH).
            let (path, _) = path_label(cx, p, Some(&info));
            let mut out = vec![
                (
                    Part::Env,
                    env_segment(
                        theme,
                        kind,
                        env_text(
                            cx,
                            p,
                            access_label(
                                cx,
                                session_toasts::effective_access(cx, p.id, info.access),
                            ),
                        ),
                        read_only,
                    ),
                ),
                (
                    Part::Path,
                    status_label(theme, path, TextSource::Fluent)
                        .truncate(true)
                        .id("status-path")
                        .into_any_element(),
                ),
                (Part::Latency, fluent(latency_label(cx, &info))),
                // ## m2b-t24-shots (follow-up): which label it is, for the shots' markers.
                (
                    Part::Encryption,
                    status_label(theme, encryption_label(cx, p, &info), TextSource::Fluent)
                        .truncate(true)
                        .id(SharedString::from(format!(
                            "status-enc-{}",
                            encryption_key(p, &info)
                        )))
                        .into_any_element(),
                ),
            ];
            if matches!(info.support, Support::Unsupported { .. }) {
                out.push((
                    Part::Unsupported,
                    fluent(tr(cx, ids::hv::CAPSULE_UNSUPPORTED, NoArgs)),
                ));
            }
            // ## demofix-1 (PO demo F9): "PostgreSQL 16.16", never the product twice or the
            // whole build line; the full text is the segment's tooltip.
            let (short, full) = version_texts(&info.server.product, &info.server.raw_version);
            // ## m2b-t22-live (fix round 1): the last segment, it ellipsizes when the bar is full.
            let label = status_label(theme, short, TextSource::Data).truncate(true);
            out.push((
                Part::Version,
                match full {
                    Some(full) => div()
                        .id("status-version-tip")
                        .flex()
                        .flex_row()
                        .items_center()
                        .min_w(px(0.0))
                        .child(label)
                        .tooltip(move |_, cx| Tip::view(full.clone(), cx))
                        .into_any_element(),
                    None => label.into_any_element(),
                },
            ));
            out
        }
    }
}

// ## demofix-1 (PO demo F9)
/// The most characters of the version the status bar shows (a build line can be anything).
const VERSION_SHORT_MAX: usize = 24;

/// The status bar's version segment: `product` and the server's version number (the first
/// word of `raw` that starts with a digit, up to a `-` build suffix: "PostgreSQL 16.16",
/// "MySQL 8.0.40", "MariaDB 11.4.2"), and, when that leaves anything out, the server's whole
/// version text for the tooltip, its product named once (PostgreSQL's `version()` already
/// begins with it). Control characters in the server's text are shown as `<U+XXXX>`.
pub(crate) fn version_texts(product: &str, raw: &str) -> (String, Option<SharedString>) {
    let raw = raw.trim();
    if raw.is_empty() {
        return (product.to_string(), None);
    }
    let number = raw
        .split_whitespace()
        .find(|w| w.starts_with(|c: char| c.is_ascii_digit()))
        .map(|w| w.split('-').next().unwrap_or(w))
        .map(|w| w.trim_end_matches([',', ';']))
        .map(|w| w.chars().take(VERSION_SHORT_MAX).collect::<String>());
    let full = if raw.starts_with(product) {
        raw.to_string()
    } else {
        format!("{product} {raw}")
    };
    let full = escape_controls(&full);
    match number {
        Some(n) => {
            let short = escape_controls(&format!("{product} {n}"));
            let tip = (short != full).then(|| full.into());
            (short, tip)
        }
        None => (product.to_string(), Some(full.into())),
    }
}

/// `s` with every control or bidi character as `<U+XXXX>` (server text is data).
fn escape_controls(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if c.is_control()
            || matches!(c, '\u{200E}' | '\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}')
        {
            out.push_str(&format!("<U+{:04X}>", c as u32));
        } else {
            out.push(c);
        }
    }
    out
}
// ## demofix-1

// ## m3a-t34-status-follows
//
// The status bar and the capsule follow the console (DESIGN.md "Status Follows The Tab";
// design/m3a-query/NOTES.md surface 6).
//
// - **Which console** (fix round 1, C1). The focused pane's active tab decides
//   (`shell::workspace::ActiveTab`, mirrored by the workspace): the console whose editor has
//   the focus when its tab is the active one, else the active tab's console. A tab switch
//   moves no focus, so the active tab, not the focus, says which console the user sees.
//   Picking a connection in the explorer hands the capsule and the bar back to the M2 rule
//   until that console's tab is shown again or its editor takes the focus
//   ([`followed_console`]). Only a bound console is followed.
// - **The bar of a followed console**, left to right: the environment and access of its active
//   result tab (`ResultContext`, whose access fails closed), else the console's own; the
//   state, path, latency, encryption and "unsupported" segments; the user's transaction
//   ("Transaction open · N statements", or its Failed form); the active result's rows ·
//   timing; "Cursor open" (a button that closes the cursor); the transient message; the
//   console's editor segment (Task 32's entity); the server version.
// - **Collapse order at narrow widths** (first given way first): the version, the encryption
//   (and the "unsupported" mark), the editor segment, the path and latency, rows · timing,
//   the state words. ## m3c-t41-chrome (F2 M8, M3c-35; design/m3b-safe surface 24): the
//   editor segment's parts give way one by one: the selected count, the carets and "Mixed line
//   endings" before the path, Ln/Col after it; "Reduced mode" never. The segment and the
//   version sit right-aligned (the room left over goes to the cell before them). The environment, the transaction and the cursor never give way; the
//   message ellipsizes (its SQL is data) down to a floor. ## m3a-f2-visual (named exception):
//   a cell gives way whole (NOTES cross-cutting #7: never ellipsized chrome); the message
//   ellipsizes only once every other cell gave way (fix round 1, I1).
// - **Session state** is mirrored, never read on the paint path: one foreground task per
//   console session awaits its `watch` (a `tokio::sync` receiver) and repaints only when the
//   projection the bar shows ([`SessionView`]) changes. The projection is Task 23's
//   `console_ux` table (fix round 1), with one rule more: a session that ended shows no
//   transaction and no cursor (I3).
// - **Access** (fix round 1, I2): the console's own session is authoritative. The bar and the
//   capsule show the less protective of the session's label and the label they would show
//   otherwise (the live connection's, or a result tab's recorded one): they never claim
//   protection the console's session does not enforce.

/// The transaction the status bar shows: only the user's (`TxOwner::User`, R-M3-27). A
/// transaction Hacivat holds around a PostgreSQL cursor (`TxOwner::Cursor`) or the read-only
/// guard's plan (`TxOwner::Guard`) is not the user's and is never shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum TxSegment {
    #[default]
    None,
    Open {
        statements: u32,
    },
    /// An error or a server cancel stopped it (R-M3-16 amended, P16): only a rollback ends
    /// it. A PostgreSQL unit that cannot suspend, stopped inside the user's transaction after
    /// the 1 s grace, lands here (Task 18 ruling 4).
    Failed,
}

/// The open console cursor the bar offers to close.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CursorSegment {
    pub result: ResultId,
    /// `false` while the session releases the cursor itself (its idle bound ran out: `Busy`
    /// with request 0, which no ticket holds). The snapshot is held until that ends, so the
    /// segment stays, but a click would only queue a second release.
    pub closable: bool,
}

/// What the status bar shows of a console's session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SessionView {
    pub tx: TxSegment,
    pub cursor: Option<CursorSegment>,
    /// The session's own access label (fix round 1, I2).
    pub access: AccessLabel,
}

/// The status bar's projection of a session snapshot (see [`TxSegment`], [`CursorSegment`]).
///
/// The transaction, the cursor and the access are Task 23's `console_ux` table (the profile's
/// environment, the run and the pool do not reach those fields, so they are not
/// passed). One rule more, the safer one where the two differ: a session that is
/// `Disconnected` or `Broken` shows no transaction and no cursor (fix round 1, I3). Its close
/// and its break roll the transaction back but leave the last `tx` in the snapshot, so the
/// table alone would keep showing a transaction that is gone (session side: T18/M3b; the
/// table's `tx_badge`: T33).
pub fn session_view(snap: &SessionSnapshot) -> SessionView {
    let ux = console_ux(None, snap, None, None);
    let ended = matches!(
        snap.state,
        SessionState::Disconnected | SessionState::Broken(_)
    );
    let tx = match ux.tx {
        _ if ended => TxSegment::None,
        TxBadge::UserOpen { statements, .. } => TxSegment::Open { statements },
        TxBadge::Failed => TxSegment::Failed,
        TxBadge::Auto | TxBadge::CursorSnapshot { .. } => TxSegment::None,
    };
    let releasing = matches!(snap.state, SessionState::Busy { req: 0, .. });
    let cursor = ux.cursor.filter(|_| !ended).map(|c| CursorSegment {
        result: c.result,
        closable: !releasing,
    });
    SessionView {
        tx,
        cursor,
        access: ux.access,
    }
}

/// How much an access label protects: read-write, then guarded, then enforced.
fn protection(access: AccessLabel) -> u8 {
    match access {
        AccessLabel::ReadWrite => 0,
        AccessLabel::ReadOnlyGuarded => 1,
        AccessLabel::ReadOnlyEnforced => 2,
    }
}

/// The less protective of `a` and `b`: what the bar may claim when both describe the console.
pub fn less_protective(a: AccessLabel, b: AccessLabel) -> AccessLabel {
    if protection(a) <= protection(b) { a } else { b }
}

/// One console session the bar watches.
struct Watched {
    session: SessionId,
    view: SessionView,
    _task: Task<()>,
}

/// The follow state (a global): the console the user last showed or focused, with the
/// explorer's selection at that moment, and the mirrored session views.
#[derive(Default)]
struct StatusFollow {
    last: Option<(ConsoleId, Option<ProfileId>)>,
    watched: HashMap<ConsoleId, Watched>,
}

impl Global for StatusFollow {}

/// Starts following: the focused editor's console, and every console session's state.
/// Called from `connections::live::install`, beside the `status_left` hook.
pub fn install(cx: &mut App) {
    cx.default_global::<StatusFollow>();
    cx.observe_global::<FocusedEditor>(|cx| {
        note_focus(cx);
        cx.refresh_windows();
    })
    .detach();
    // ## fix round 1 (C1): a tab switch moves no focus; the active tab is what the user sees.
    cx.observe_global::<ActiveTab>(|cx| {
        note_active(cx);
        cx.refresh_windows();
    })
    .detach();
    cx.observe_global::<Consoles>(watch_sessions).detach();
    watch_sessions(cx);
}

/// The active tab changed: its console (if bound) is the one the user now sees; remember it
/// with the explorer's selection.
fn note_active(cx: &mut App) {
    let Some(id) = active_tab(cx)
        .flatten()
        .and_then(|tab| Consoles::by_tab(cx, tab))
    else {
        return;
    };
    let selected = cx
        .try_global::<ConnectionsModel>()
        .and_then(|m| m.selected());
    cx.default_global::<StatusFollow>().last = Some((id, selected));
}

/// The focused pane's active tab as the workspace mirrors it; `None` before any workspace
/// rendered.
fn active_tab(cx: &App) -> Option<Option<TabId>> {
    cx.try_global::<ActiveTab>().map(|a| a.0)
}

/// The console whose editor has the focus.
fn editor_console(cx: &App) -> Option<ConsoleId> {
    let editor = cx.try_global::<FocusedEditor>()?.0.as_ref()?.entity_id();
    Consoles::ids(cx).into_iter().find(|id| {
        Consoles::get(cx, *id)
            .and_then(|s| s.editor.as_ref())
            .is_some_and(|e| e.entity_id() == editor)
    })
}

/// A console editor took the focus: remember it, with the explorer's selection (and tell the
/// registry, so `console::focused_console` agrees).
fn note_focus(cx: &mut App) {
    let Some(id) = editor_console(cx) else {
        return;
    };
    let selected = cx
        .try_global::<ConnectionsModel>()
        .and_then(|m| m.selected());
    cx.default_global::<StatusFollow>().last = Some((id, selected));
    if Consoles::focused(cx) != Some(id) {
        Consoles::set_focused(cx, Some(id));
    }
}

/// The console the capsule and the status bar follow (see the block comment above):
/// 1. the console whose editor has the focus, when its tab is the active one;
/// 2. else the active tab's console, unless a connection was picked in the explorer since
///    that console was last shown or focused;
/// 3. else none (the M2 rule).
///
/// Before any workspace rendered (no active tab is known), the focused editor's console.
pub fn followed_console(cx: &App) -> Option<ConsoleId> {
    let Some(active) = active_tab(cx) else {
        return editor_console(cx);
    };
    let active = active?;
    if let Some(id) =
        editor_console(cx).filter(|id| Consoles::get(cx, *id).is_some_and(|s| s.tab == active))
    {
        return Some(id);
    }
    let id = Consoles::by_tab(cx, active)?;
    if let Some((last, selected)) = cx.try_global::<StatusFollow>().and_then(|f| f.last) {
        let now = cx
            .try_global::<ConnectionsModel>()
            .and_then(|m| m.selected());
        if last == id && now != selected {
            return None;
        }
    }
    Some(id)
}

/// The followed console when it is bound to a saved profile, with that profile.
pub(crate) fn followed_bound_console(cx: &App) -> Option<(ConsoleId, Arc<Profile>)> {
    let id = followed_console(cx)?;
    let profile = Consoles::get(cx, id)?.profile?;
    Some((id, cx.try_global::<ConnectionsModel>()?.get(profile)?))
}

/// The access of the followed console's own session (after a changed-data gap's downgrade),
/// when that console is bound to `profile` and its session is watched (fix round 1, I2): the
/// capsule shows no more protection than this.
pub(crate) fn followed_session_access(cx: &App, profile: ProfileId) -> Option<AccessLabel> {
    let (id, p) = followed_bound_console(cx)?;
    (p.id == profile).then_some(())?;
    session_access(cx, id, profile)
}

/// `access` capped at the followed console's session access when that console is bound to
/// `profile` (fix round 2, N1): what the capsule's explanation may claim.
pub(crate) fn capped_access(cx: &App, profile: ProfileId, access: AccessLabel) -> AccessLabel {
    followed_session_access(cx, profile).map_or(access, |s| less_protective(access, s))
}

/// The access of console `id`'s session (bound to `profile`), with the gap downgrade.
fn session_access(cx: &App, id: ConsoleId, profile: ProfileId) -> Option<AccessLabel> {
    let view = view_of(cx, id)?;
    Some(session_toasts::effective_access(cx, profile, view.access))
}

/// Keeps one watcher per console session: a new console (or a console's new session) gets
/// one, a closed console loses its own.
fn watch_sessions(cx: &mut App) {
    let wanted: Vec<(ConsoleId, SessionHandle)> = Consoles::ids(cx)
        .into_iter()
        .filter_map(|id| Some((id, Consoles::get(cx, id)?.session.clone()?)))
        .collect();
    let follow = cx.default_global::<StatusFollow>();
    let before = follow.watched.len();
    follow.watched.retain(|id, w| {
        wanted
            .iter()
            .any(|(want, h)| want == id && h.id() == w.session)
    });
    let dropped = follow.watched.len() != before;
    let new: Vec<(ConsoleId, SessionHandle)> = wanted
        .into_iter()
        .filter(|(id, _)| !follow.watched.contains_key(id))
        .collect();
    if new.is_empty() {
        if dropped {
            cx.refresh_windows();
        }
        return;
    }
    for (id, session) in new {
        let mut rx = session.state();
        let view = session_view(&rx.borrow_and_update());
        // The foreground task awaits a `tokio::sync::watch` receiver only (Q4 ruling).
        let task = cx.spawn(async move |cx| {
            while rx.changed().await.is_ok() {
                let view = session_view(&rx.borrow_and_update());
                cx.update(|cx| note_view(cx, id, view));
            }
        });
        cx.default_global::<StatusFollow>().watched.insert(
            id,
            Watched {
                session: session.id(),
                view,
                _task: task,
            },
        );
    }
    cx.refresh_windows();
}

/// A watched session's view changed: repaint when what the bar shows did.
fn note_view(cx: &mut App, id: ConsoleId, view: SessionView) {
    let Some(w) = cx.default_global::<StatusFollow>().watched.get_mut(&id) else {
        return;
    };
    if w.view != view {
        w.view = view;
        cx.refresh_windows();
    }
}

/// How many console sessions the bar watches (a test seam: a closed console's watcher goes).
#[cfg(feature = "test-support")]
pub fn watched_sessions(cx: &App) -> usize {
    cx.try_global::<StatusFollow>()
        .map_or(0, |f| f.watched.len())
}

/// The mirrored view of console `id`'s session; `None` when it has no session.
fn view_of(cx: &App, id: ConsoleId) -> Option<SessionView> {
    cx.try_global::<StatusFollow>()
        .and_then(|f| f.watched.get(&id))
        .map(|w| w.view)
}

// ## m3a-f2-visual (named exception in Task 34's block): a cell gives way whole, never
// ellipsized chrome ("ş…") and never a sliver (NOTES cross-cutting #7). `hacivat_ui::give_way`
// lays the row out; only the message, whose SQL is data, ellipsizes, down to a floor.
/// How a cell of a followed console's bar gives way when the bar is too narrow.
type Give = Yield;

/// NOTES surface 6, first given way first: the version, the encryption, the editor segment
/// (selected count, Ln/Col), the path (and latency), rows · timing, the state words.
fn give_of(part: Part) -> Give {
    match part {
        Part::Version => Yield::Whole(60),
        Part::Encryption | Part::Unsupported => Yield::Whole(50),
        Part::Path | Part::Latency => Yield::Whole(30),
        Part::Env | Part::State => Yield::Whole(10),
    }
}

// ## m3c-t41-chrome (F2 M8 = T34 M6; NOTES surface 24): the editor segment's parts, each with
// its own rank: the selected count, the carets, "Mixed line endings" after the encryption and
// before the path; Ln/Col after the path, before rows · timing; Reduced mode never.
fn give_editor(part: crate::console::status::SegmentPart) -> Give {
    use crate::console::status::SegmentPart;
    match part {
        SegmentPart::Reduced => Yield::Never,
        SegmentPart::Selected => Yield::Whole(46),
        SegmentPart::Carets => Yield::Whole(45),
        SegmentPart::LineEndings => Yield::Whole(44),
        SegmentPart::LnCol => Yield::Whole(25),
    }
}
/// Rows · timing gives way after the path.
const GIVE_ROWS: Give = Yield::Whole(20);
/// The message ellipsizes, down to a floor, only once every cell that can give way left
/// (rows · timing and the state words last): hide before truncate (NOTES #7; fix round 1, I1).
const GIVE_MESSAGE: Give = Yield::Shrink {
    floor: 160.0,
    rank: 5,
};

/// The bar of the followed console `console`, bound to `p` (see the block comment above).
fn console_segments(
    theme: &Theme,
    cx: &App,
    m: &ConnectionsModel,
    console: ConsoleId,
    p: &Profile,
) -> Vec<AnyElement> {
    let state = Consoles::get(cx, console);
    let rc = result_context_of(console, cx);
    let session = session_access(cx, console, p.id);
    let mut parts = connection_parts(theme, cx, m, p);
    // The env cell: the active result tab's environment and access (Status Follows The Tab),
    // else the console's own; never more protection than the console's session has (I2).
    let env = match &rc {
        Some(rc) => Some(result_env(
            theme,
            cx,
            rc.env,
            session.map_or(rc.access, |s| less_protective(rc.access, s)),
        )),
        None => session.and_then(|s| {
            let shown = shown_access(cx, m, p);
            (less_protective(shown, s) != shown).then(|| result_env(theme, cx, p.environment, s))
        }),
    };
    if let Some(env) = env {
        match parts.iter().position(|(part, _)| *part == Part::Env) {
            Some(ix) => parts[ix] = (Part::Env, env),
            None => parts.insert(0, (Part::Env, env)),
        }
    }
    let version = parts
        .iter()
        .position(|(part, _)| *part == Part::Version)
        .map(|ix| parts.remove(ix));
    // The path and its latency are one cell ("the path with latency", NOTES surface 6), so
    // they give way together.
    let mut cells: Vec<(Give, AnyElement)> = Vec::new();
    let mut parts = parts.into_iter().peekable();
    while let Some((part, segment)) = parts.next() {
        if part == Part::Path && parts.peek().is_some_and(|(next, _)| *next == Part::Latency) {
            let latency = parts.next().map(|(_, latency)| latency);
            cells.push((
                give_of(part),
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .min_w(px(0.0))
                    .gap(px(theme.tokens.spacing.sm))
                    .child(segment)
                    .children(latency)
                    .into_any_element(),
            ));
            continue;
        }
        cells.push((give_of(part), segment));
    }
    let view = view_of(cx, console);
    match view.map_or(TxSegment::None, |v| v.tx) {
        TxSegment::None => {}
        // ## m3b-t22-tx-ui (named exception): the segment is a button that opens the
        // transaction menu above itself (NOTES surface 4).
        TxSegment::Open { statements } => {
            cells.push((
                Give::Never,
                tx_segment_button(
                    theme,
                    cx,
                    console,
                    tx_open(theme, cx, statements),
                    Some(tr(cx, ids::hv::TX_OPEN_TIP, NoArgs)),
                ),
            ));
        }
        TxSegment::Failed => cells.push((
            Give::Never,
            tx_segment_button(
                theme,
                cx,
                console,
                tx_failed(theme, cx),
                Some(tr(cx, ids::hv::TX_FAILED_TIP, NoArgs)),
            ),
        )),
    }
    if let Some(rows) = rc.as_ref().map(|rc| rows_timing(cx, rc)) {
        cells.push((
            GIVE_ROWS,
            status_label_in(
                theme,
                rows,
                TextSource::Fluent,
                // ## m3a-f2-visual (named exception): mono 11.5, the bar's data size (DESIGN.md
                // status bar caption; NOTES surface 6 "in mono"), not the 15 px counter.
                theme.tokens.type_ramp.mono_data,
            )
            .truncate(true)
            .id("status-rows")
            .into_any_element(),
        ));
    }
    if let (Some(cursor), Some(session)) = (
        view.and_then(|v| v.cursor),
        state.and_then(|s| s.session.clone()),
    ) {
        cells.push((Give::Never, cursor_button(cx, cursor, session)));
    }
    if let Some(text) = state.and_then(|s| console_message(s, cx)) {
        let t = theme.tokens;
        cells.push((
            GIVE_MESSAGE,
            Label::new(
                text,
                TextSource::Fluent,
                t.type_ramp.caption,
                t.colors.text.to_hsla(),
            )
            .truncate(true)
            .id("status-message")
            .into_any_element(),
        ));
    }
    // ## m3c-t41-chrome: the right-aligned group: the editor segment's parts, then the
    // version; the cell before them takes the room left over.
    let right = cells.len();
    for (part, cell) in crate::console::editor_segment_cells(console, cx) {
        cells.push((give_editor(part), cell.into_any_element()));
    }
    if let Some((part, segment)) = version {
        // ## m3c-t41-chrome: its selector, for the order test.
        let segment = div()
            .debug_selector(|| "status-version".to_string())
            .flex()
            .flex_row()
            .items_center()
            .min_w(px(0.0))
            .child(segment)
            .into_any_element();
        cells.push((give_of(part), segment));
    }
    let mut cells = cells.into_iter();
    let Some((_, first)) = cells.next() else {
        return Vec::new();
    };
    let rest: Vec<(Give, AnyElement)> = cells.collect();
    if rest.is_empty() {
        return vec![first];
    }
    // In `rest`, the cell before the right group (none when the group starts the row).
    let grow = right.checked_sub(2);
    vec![first, give_way_row(theme, rest, grow)]
}

/// The cells after the first, as one bar segment that lays them out itself, so each gives
/// way by its own rank instead of by its place (the bar's own rule, M2b).
///
/// ## m3a-f2-visual (named exception in Task 34's block): the row is
/// `hacivat_ui::give_way::GiveWayRow`. A cell that gives way leaves whole, so no chrome
/// ellipsizes ("ş…") and no cell is ever narrower than its content: two seams are always a
/// cell's padding apart, at any width, with no rounding (Task 33's question: the row needs
/// none). The room left over goes to the last cell shown.
///
/// - A cell is a box that fills the width it is given, around an inner box that holds the
///   12 px padding, the content and the seam at its right edge. Only the message's cell is
///   ever narrower than its content (its SQL ellipsizes, down to its floor); it clips.
/// - The row covers its bar segment's padding; on the right it reaches one pixel further, so
///   the last cell's seam lies on the bar's own seam after the segment.
fn give_way_row(theme: &Theme, cells: Vec<(Give, AnyElement)>, grow: Option<usize>) -> AnyElement {
    let t = theme.tokens;
    let seam = t.colors.seam.to_hsla();
    // The least an inner box keeps: its padding and its seam.
    let inner_floor = 2.0 * t.spacing.lg + 1.0;
    let cells = cells
        .into_iter()
        .map(|(give, child)| {
            let inner = div()
                .relative()
                .flex()
                .flex_row()
                .items_center()
                .h_full()
                .flex_grow(1.0)
                .flex_shrink(1.0)
                .min_w(px(inner_floor))
                .px(px(t.spacing.lg))
                .gap(px(t.spacing.sm))
                .child(child)
                .child(
                    div()
                        .absolute()
                        .right_0()
                        .top_0()
                        .h_full()
                        .w(px(1.0))
                        .bg(seam),
                );
            let cell = div()
                .flex()
                .flex_row()
                .items_center()
                .w_full()
                .h_full()
                .child(inner);
            let cell = match give {
                Yield::Shrink { .. } => cell.overflow_hidden(),
                _ => cell,
            };
            (give, cell.into_any_element())
        })
        .collect();
    GiveWayRow::new(cells)
        .margins(-t.spacing.lg, -t.spacing.lg - 1.0)
        // ## m3c-t41-chrome (M3c-35)
        .grow(grow)
        .into_any_element()
}

/// The environment segment for environment `env` and access `access` (a result tab's, or the
/// console session's).
fn result_env(theme: &Theme, cx: &App, env: Option<EnvTag>, access: AccessLabel) -> AnyElement {
    let text = tr(
        cx,
        ids::hv::STATUS_ENV,
        ids::hv::HvStatusEnv {
            env: Text(env_word(cx, env).to_string()),
            access: Text(access_label(cx, access).to_string()),
        },
    );
    env_segment(theme, env_kind(env), text, access != AccessLabel::ReadWrite)
}

/// The access the M2 env cell of `p` claims: the live connection's (after the gap downgrade)
/// while connected, else the profile's read-only flag.
fn shown_access(cx: &App, m: &ConnectionsModel, p: &Profile) -> AccessLabel {
    match m.live(p.id) {
        LiveState::Connected(info) => session_toasts::effective_access(cx, p.id, info.access),
        _ if p.read_only => AccessLabel::ReadOnlyGuarded,
        _ => AccessLabel::ReadWrite,
    }
}

/// "Transaction open · 3 statements" with the amber marker (pending work); the tooltip says
/// how to end it.
fn tx_open(theme: &Theme, cx: &App, statements: u32) -> AnyElement {
    let t = theme.tokens;
    div()
        .id("status-tx")
        .debug_selector(|| "status-tx".to_string())
        .flex()
        .flex_row()
        .items_center()
        .gap(px(t.spacing.sm))
        .child(run_marker(theme))
        .child(
            Label::new(
                tr(
                    cx,
                    ids::hv::TX_OPEN,
                    ids::hv::HvTxOpen {
                        n: Count(u64::from(statements)),
                    },
                ),
                TextSource::Fluent,
                t.type_ramp.caption,
                t.colors.text.to_hsla(),
            )
            .id("status-tx-open"),
        )
        // ## m3b-t22-tx-ui (named exception, review m8): the button around it carries the
        // tooltip, so the segment's own no longer shadows it.
        .into_any_element()
}

// ## m3b-t22-tx-ui (named exception)
/// The transaction segment as a button (`segment`, M3a's open or failed form): a Tab stop
/// that Enter, Space or a click opens the console's transaction menu with, anchored 4 px above
/// the segment, left edges aligned (NOTES surface 4, M2); its tooltip says so, after `lead`
/// (the failed form's explanation). The segment draws no tooltip of its own.
fn tx_segment_button(
    theme: &Theme,
    cx: &App,
    console: ConsoleId,
    segment: AnyElement,
    lead: Option<SharedString>,
) -> AnyElement {
    use std::rc::Rc;

    let t = theme.tokens;
    let click = tr(cx, ids::hv::TX_SEGMENT_TIP, NoArgs);
    let tip: SharedString = match lead {
        Some(lead) => format!("{lead} {click}").into(),
        None => click,
    };
    let open = Rc::new(
        move |window: &mut hacivat_ui_core::gpui_surface::Window, cx: &mut App| {
            crate::console::tx_chip::open_segment_menu(console, window, cx);
        },
    );
    let (on_click, on_key) = (open.clone(), open);
    let button = div()
        .id("status-tx-button")
        .debug_selector(|| "status-tx-button".to_string())
        .relative()
        .flex()
        .flex_row()
        .items_center()
        .rounded(px(t.radii.sm))
        .cursor_pointer()
        .child(segment)
        .children(crate::console::tx_badge::record_bounds(
            console,
            crate::console::tx_badge::Focus::Segment,
            cx,
        ))
        .on_click(move |_, window, cx| on_click(window, cx))
        .on_key_down(move |e, window, cx| {
            if matches!(e.keystroke.key.as_str(), "enter" | "space")
                && !e.keystroke.modifiers.modified()
            {
                cx.stop_propagation();
                on_key(window, cx);
            }
        })
        .tooltip(move |_, cx| Tip::view(tip.clone(), cx));
    match crate::console::tx_badge::focus_of(console, crate::console::tx_badge::Focus::Segment, cx)
    {
        Some(focus) => button
            .track_focus(&focus)
            .focus_visible(|s| s.bg(t.colors.blue_tint.to_hsla()))
            .into_any_element(),
        None => button.into_any_element(),
    }
}

/// "Transaction failed · roll back to end it" with the ink alert glyph (an error takes no hue).
fn tx_failed(theme: &Theme, cx: &App) -> AnyElement {
    let t = theme.tokens;
    div()
        .id("status-tx")
        .debug_selector(|| "status-tx".to_string())
        .flex()
        .flex_row()
        .items_center()
        .gap(px(t.spacing.sm))
        .child(
            Icon::new(IconName::TriangleAlert)
                .size(IconSize::Chrome)
                .color(t.colors.text.to_hsla()),
        )
        .child(
            Label::new(
                tr(cx, ids::hv::TX_FAILED, NoArgs),
                TextSource::Fluent,
                t.type_ramp.caption,
                t.colors.text.to_hsla(),
            )
            .id("status-tx-failed"),
        )
        // ## m3b-t22-tx-ui (named exception, review m8): the button carries the tooltip.
        .into_any_element()
}

/// The active result's rows and time ("10,000+ rows · 131 ms"): the rows loaded, with "+"
/// while more may come (streaming, suspended, or cut short by a limit); the time is the
/// statement's execution plus its fetching, once the run UI has reported them.
fn rows_timing(cx: &App, rc: &ResultContext) -> SharedString {
    let more = matches!(
        rc.completeness,
        CompletenessView::Streaming | CompletenessView::Suspended | CompletenessView::Truncated
    );
    let rows = Count(rc.loaded);
    let duration = match (rc.exec_ms, rc.fetch_ms) {
        (None, None) => None,
        (exec, fetch) => Some(Text(
            duration_label(cx, exec.unwrap_or(0).saturating_add(fetch.unwrap_or(0))).to_string(),
        )),
    };
    match (more, duration) {
        (false, None) => tr(cx, ids::hv::RESULTS_ROWS, ids::hv::HvResultsRows { rows }),
        (true, None) => tr(
            cx,
            ids::hv::RESULTS_ROWS_MORE,
            ids::hv::HvResultsRowsMore { rows },
        ),
        (false, Some(duration)) => tr(
            cx,
            ids::hv::RESULTS_ROWS_TIMING,
            ids::hv::HvResultsRowsTiming { rows, duration },
        ),
        (true, Some(duration)) => tr(
            cx,
            ids::hv::RESULTS_ROWS_MORE_TIMING,
            ids::hv::HvResultsRowsMoreTiming { rows, duration },
        ),
    }
}

/// A duration: whole milliseconds under a second, else seconds with two decimals.
fn duration_label(cx: &App, ms: u64) -> SharedString {
    if ms < 1000 {
        return tr(
            cx,
            ids::hv::STATUS_DURATION_MS,
            ids::hv::HvStatusDurationMs { ms: Count(ms) },
        );
    }
    tr(
        cx,
        ids::hv::STATUS_DURATION_S,
        ids::hv::HvStatusDurationS {
            seconds: ExactDecimal(format!("{}.{:02}", ms / 1000, (ms % 1000) / 10)),
        },
    )
}

/// "Cursor open": a button with the lock-open glyph that closes the cursor (a Tab stop that
/// Enter or Space presses); the tooltip says what it holds.
fn cursor_button(cx: &App, cursor: CursorSegment, session: SessionHandle) -> AnyElement {
    let tip = tr(cx, ids::hv::RESULTS_STATUS_CURSOR_TIP, NoArgs);
    let result = cursor.result;
    div()
        .id("status-cursor-cell")
        .flex()
        .flex_row()
        .items_center()
        .child(
            Button::new(
                "status-cursor",
                tr(cx, ids::hv::RESULTS_STATUS_CURSOR, NoArgs),
            )
            .kind(ButtonKind::Ghost)
            .icon(IconName::LockOpen)
            .inert(!cursor.closable)
            .on_click(move |_, _, _| {
                // The release reaches the result through the run's observer
                // (`CursorReleased`); the ticket is dropped, as `FetchControl::close` does.
                let _ = session.submit(Request::CloseCursor {
                    result,
                    reason: CloseReason::UserClosed,
                });
            }),
        )
        .tooltip(move |_, cx| Tip::view(tip.clone(), cx))
        .into_any_element()
}

/// The followed console's transient sentence: the editor's (Describe execution range, the Tab
/// mode), else the Run preview's ("Nothing to run here", the boundary-uncertain line).
fn console_message(state: &ConsoleState, cx: &App) -> Option<SharedString> {
    let editor = state.editor.as_ref()?.read(cx);
    hacivat_ui_editor::commands::status_message(editor).or_else(|| preview_message(editor, cx))
}

/// The Run preview's status sentence (Task 28; fix round 1, M7).
fn preview_message(editor: &EditorView, cx: &App) -> Option<SharedString> {
    hacivat_ui_editor::preview::status_message(editor, cx)
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    // ## demofix-1 (PO demo F9)
    #[test]
    fn the_version_segment_is_short_and_names_its_product_once() {
        let pg = "PostgreSQL 16.16 (Debian 16.16-1.pgdg13+1) on aarch64-unknown-linux-gnu, \
                  compiled by gcc (Debian 14.2.0-19) 14.2.0, 64-bit";
        let (short, tip) = super::version_texts("PostgreSQL", pg);
        assert_eq!(short, "PostgreSQL 16.16");
        assert_eq!(tip.as_deref(), Some(pg), "the whole text, the product once");
        let (short, tip) = super::version_texts("MySQL", "8.0.40-0ubuntu0.22.04.1");
        assert_eq!(short, "MySQL 8.0.40");
        assert_eq!(tip.as_deref(), Some("MySQL 8.0.40-0ubuntu0.22.04.1"));
        let (short, tip) = super::version_texts("MariaDB", "11.4.2-MariaDB-ubu2404");
        assert_eq!(short, "MariaDB 11.4.2");
        assert_eq!(tip.as_deref(), Some("MariaDB 11.4.2-MariaDB-ubu2404"));
        // Nothing left out: no tooltip.
        assert_eq!(
            super::version_texts("MySQL", "8.4.3"),
            ("MySQL 8.4.3".to_string(), None)
        );
        assert_eq!(
            super::version_texts("SQLite", "3.45.0"),
            ("SQLite 3.45.0".to_string(), None)
        );
        assert_eq!(
            super::version_texts("PostgreSQL", ""),
            ("PostgreSQL".to_string(), None)
        );
        // A pre-release keeps its word; a text with no number keeps the product only.
        assert_eq!(
            super::version_texts("PostgreSQL", "PostgreSQL 18beta1 on x86_64").0,
            "PostgreSQL 18beta1"
        );
        let (short, tip) = super::version_texts("PostgreSQL", "EnterpriseDB build");
        assert_eq!(short, "PostgreSQL");
        assert_eq!(tip.as_deref(), Some("PostgreSQL EnterpriseDB build"));
        // Server text is data: controls are shown, never obeyed; a huge word is cut.
        let (short, tip) = super::version_texts("MySQL", "8.0.1\u{202E}x");
        assert_eq!(short, "MySQL 8.0.1<U+202E>x");
        assert_eq!(tip, None);
        let long = "9".repeat(80);
        assert_eq!(
            super::version_texts("MySQL", &long).0.len(),
            "MySQL ".len() + 24
        );
    }

    use hacivat_driver::TxStatus;
    use hacivat_proto::ErrClass;
    use hacivat_session::session::{CursorBadge, TxOwner, TxState};

    use super::*;

    fn snap(owner: TxOwner, status: TxStatus, statements: u32) -> SessionSnapshot {
        SessionSnapshot {
            state: SessionState::Idle,
            tx: TxState {
                owner,
                status,
                statements,
                ..TxState::default()
            },
            schema: None,
            cancel: Default::default(),
            read_only: false,
            access: AccessLabel::ReadWrite,
            generation: 1,
            cursor: None,
        }
    }

    #[test]
    fn only_the_users_transaction_is_shown() {
        assert_eq!(
            session_view(&snap(TxOwner::User, TxStatus::InTx, 3)).tx,
            TxSegment::Open { statements: 3 }
        );
        for owner in [TxOwner::Cursor, TxOwner::Guard, TxOwner::None] {
            for status in [TxStatus::InTx, TxStatus::Failed, TxStatus::Idle] {
                assert_eq!(
                    session_view(&snap(owner, status, 2)).tx,
                    TxSegment::None,
                    "{owner:?} {status:?}"
                );
            }
        }
        assert_eq!(
            session_view(&snap(TxOwner::User, TxStatus::Idle, 0)).tx,
            TxSegment::None
        );
    }

    // ## fix round 1 (I3)
    #[test]
    fn a_session_that_ended_shows_no_transaction_and_no_cursor() {
        for state in [
            SessionState::Disconnected,
            SessionState::Broken(ErrClass::ALL[0]),
        ] {
            for status in [TxStatus::InTx, TxStatus::Failed] {
                let mut s = snap(TxOwner::User, status, 3);
                s.state = state.clone();
                s.cursor = Some(CursorBadge {
                    result: ResultId(1),
                    rows: 10,
                    since: Instant::now(),
                    holds_tx: true,
                });
                let view = session_view(&s);
                assert_eq!(view.tx, TxSegment::None, "{state:?} {status:?}");
                assert_eq!(view.cursor, None, "{state:?} {status:?}");
            }
        }
    }

    // ## fix round 1 (I2)
    #[test]
    fn the_session_access_is_mirrored_and_the_less_protective_label_wins() {
        let mut s = snap(TxOwner::None, TxStatus::Idle, 0);
        s.access = AccessLabel::ReadOnlyEnforced;
        assert_eq!(session_view(&s).access, AccessLabel::ReadOnlyEnforced);
        use AccessLabel::{ReadOnlyEnforced as E, ReadOnlyGuarded as G, ReadWrite as W};
        for (a, b, want) in [
            (W, G, W),
            (G, W, W),
            (W, E, W),
            (E, G, G),
            (G, E, G),
            (E, E, E),
        ] {
            assert_eq!(less_protective(a, b), want, "{a:?} {b:?}");
        }
    }

    #[test]
    fn a_failed_user_transaction_is_shown_failed() {
        assert_eq!(
            session_view(&snap(TxOwner::User, TxStatus::Failed, 4)).tx,
            TxSegment::Failed
        );
    }

    #[test]
    fn a_cursor_released_by_its_idle_bound_is_not_closable() {
        let badge = CursorBadge {
            result: ResultId(7),
            rows: 10_000,
            since: Instant::now(),
            holds_tx: true,
        };
        let mut s = snap(TxOwner::Cursor, TxStatus::InTx, 0);
        s.cursor = Some(badge);
        assert_eq!(
            session_view(&s).cursor,
            Some(CursorSegment {
                result: ResultId(7),
                closable: true
            })
        );
        // The idle release: `Busy` with request 0, which no ticket holds.
        s.state = SessionState::Busy {
            req: 0,
            since: Instant::now(),
        };
        assert_eq!(
            session_view(&s).cursor,
            Some(CursorSegment {
                result: ResultId(7),
                closable: false
            })
        );
        // A request of the user's keeps it closable (the close queues behind it).
        s.state = SessionState::Busy {
            req: 3,
            since: Instant::now(),
        };
        assert_eq!(session_view(&s).cursor.map(|c| c.closable), Some(true));
    }
}
