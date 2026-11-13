//! The toasts of live sessions (NOTES M2a §7 "Session lost", M2b surface 7; Task 22): a session
//! lost with state, an idle disconnect, an idle transaction, a read-only enforcement gap.
//!
//! - **Session lost** (D-085, B19): a wide sticky toast. Its title and first line say whether
//!   Hacivat opened a new session: a loss with no open transaction reconnects (the reconnect
//!   has happened when the event arrives), one inside a transaction never does (Task 17, C1;
//!   `LostState::open_tx`). The transaction line shows only when one was open, so the toast
//!   never claims a rollback otherwise; the other losses are one line, a locale list ("and",
//!   Turkish "ve"). Show details selects the connection (its sheet); Dismiss closes the toast.
//! - **Idle disconnect** (B18): one toast with the threshold the profile's `LivenessPolicy`
//!   gives (whole hours, else minutes, each message with its unit, INV-1). The profile is then
//!   idle: the capsule, the status bar and the explorer say so until it connects again.
//! - **Idle transaction**: a notice (M3 adds the prompt to end it).
//! - **Enforcement gap** (Q35): a sticky alert whose line depends on the `GapReason`.
//!
//! One toast of each kind per profile: a newer one replaces the older (two quick reconnects
//! show one toast). Every text follows a language switch.
//!
//! Fix round 1: a gap alert is urgent (it never waits behind two session-lost toasts, M7),
//! and a gap that changed data downgrades the profile's "enforced" label to guarded until its
//! connection changes (M8: a write got past what the label promised).

use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use hacivat_l10n::{Count, NoArgs, Text, ids};
use hacivat_proto::{AccessLabel, ProfileId};
use hacivat_session::GapReason;
use hacivat_session::LiveState;
use hacivat_session::session::{LivenessPolicy, LostState};
use hacivat_ui::icon::IconName;
use hacivat_ui::text::TextFn;
use hacivat_ui::toast::{Toast, ToastAction, ToastKind, ToastQueue};
use hacivat_ui_core::gpui_surface::{App, Global, SharedString};

use crate::connections::model::ConnectionsModel;
use crate::l10n_global::{text_fn, tr};

/// Which session toast of a profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Kind {
    Lost,
    Idle,
    IdleTx,
    Gap,
}

/// The session toasts on screen, by profile and kind, and the profiles disconnected while
/// idle.
#[derive(Default)]
struct SessionToasts {
    shown: HashMap<(ProfileId, Kind), u64>,
    idle: HashSet<ProfileId>,
    /// Profiles whose read-only session let a write through (M8).
    downgraded: HashSet<ProfileId>,
    /// What each toast says, for the tests (feature `test-support`).
    #[cfg(feature = "test-support")]
    record: Vec<(u64, &'static str, Toast)>,
}

impl Global for SessionToasts {}

/// Shows `toast` as `profile`'s toast of `kind`, replacing the one shown before.
fn show(cx: &mut App, profile: ProfileId, kind: Kind, toast: Toast) {
    let old = cx
        .default_global::<SessionToasts>()
        .shown
        .remove(&(profile, kind));
    if let Some(old) = old {
        ToastQueue::dismiss(cx, old);
    }
    #[cfg(feature = "test-support")]
    let recorded = toast.clone();
    let id = ToastQueue::push_id(cx, toast);
    let toasts = cx.default_global::<SessionToasts>();
    toasts.shown.insert((profile, kind), id);
    #[cfg(feature = "test-support")]
    toasts.record.push((id, kind_name(kind), recorded));
}

/// The profile's name, while it exists.
fn name_of(cx: &App, profile: ProfileId) -> Option<String> {
    cx.try_global::<ConnectionsModel>()?
        .get(profile)
        .map(|p| p.name.clone())
}

/// What a session lost besides its transaction, as list items (`hv-conn-lost-item-*`).
fn lost_items(cx: &App, lost: &LostState) -> Vec<String> {
    let mut items = Vec::new();
    if lost.temp_objects {
        items.push(tr(cx, ids::hv::CONN_LOST_ITEM_TEMP, NoArgs).to_string());
    }
    if lost.session_vars {
        items.push(tr(cx, ids::hv::CONN_LOST_ITEM_VARS, NoArgs).to_string());
    }
    if lost.prepared > 0 {
        items.push(
            tr(
                cx,
                ids::hv::CONN_LOST_ITEM_PREPARED,
                ids::hv::HvConnLostItemPrepared {
                    count: Count(u64::from(lost.prepared)),
                },
            )
            .to_string(),
        );
    }
    if lost.locks {
        items.push(tr(cx, ids::hv::CONN_LOST_ITEM_LOCKS, NoArgs).to_string());
    }
    items
}

/// A session of `profile` was lost and a reconnect would lose `lost` (see the module docs).
pub(crate) fn lost(profile: ProfileId, lost: LostState, cx: &mut App) {
    let Some(name) = name_of(cx, profile) else {
        return;
    };
    // Task 17 (C1): only a session without an open (or unknown) transaction reconnects.
    let reconnected = !lost.open_tx;
    let (title_state, body_state) = if reconnected {
        (
            ids::hv::HvConnLostTitleState::Reconnected,
            ids::hv::HvConnLostBodyState::Reconnected,
        )
    } else {
        (
            ids::hv::HvConnLostTitleState::Lost,
            ids::hv::HvConnLostBodyState::Lost,
        )
    };
    let title: TextFn = Rc::new(move |cx: &App| {
        tr(
            cx,
            ids::hv::CONN_LOST_TITLE,
            ids::hv::HvConnLostTitle {
                name: Text(name.clone()),
                state: title_state,
            },
        )
    });
    let body: TextFn = Rc::new(move |cx: &App| {
        tr(
            cx,
            ids::hv::CONN_LOST_BODY,
            ids::hv::HvConnLostBody { state: body_state },
        )
    });
    let mut bullets: Vec<TextFn> = Vec::new();
    if lost.open_tx {
        bullets.push(text_fn(ids::hv::CONN_LOST_TX));
    }
    if !lost_items(cx, &lost).is_empty() {
        let lost = lost.clone();
        bullets.push(Rc::new(move |cx: &App| {
            tr(
                cx,
                ids::hv::CONN_LOST_OTHER,
                ids::hv::HvConnLostOther {
                    items: lost_items(cx, &lost),
                },
            )
        }));
    }
    let mut toast = Toast::new(ToastKind::Info, body);
    toast.sticky = true;
    toast.wide = true;
    toast.title = Some(title);
    toast.bullets = bullets;
    toast.icon = Some(if reconnected {
        IconName::RotateCw
    } else {
        IconName::Unplug
    });
    toast.actions = vec![
        ToastAction {
            label: text_fn(ids::hv::CONN_LOST_DETAILS),
            // The connection's sheet follows the explorer's selection.
            run: Rc::new(move |_, cx| ConnectionsModel::select(cx, Some(profile))),
        },
        ToastAction {
            label: text_fn(ids::hv::CONN_LOST_DISMISS),
            // An action dismisses its toast after it runs.
            run: Rc::new(|_, _| {}),
        },
    ];
    show(cx, profile, Kind::Lost, toast);
}

/// The idle threshold of `profile` (`LivenessPolicy::for_profile`, B18) as the toast's text:
/// whole hours, else minutes (rounded up).
fn idle_text(cx: &App, name: &str, profile: ProfileId) -> Option<SharedString> {
    let p = cx.try_global::<ConnectionsModel>()?.get(profile)?;
    let secs = LivenessPolicy::for_profile(&p).idle_disconnect?.as_secs();
    let name = Text(name.to_string());
    Some(if secs > 0 && secs % 3600 == 0 {
        tr(
            cx,
            ids::hv::CONN_IDLE_DISCONNECTED,
            ids::hv::HvConnIdleDisconnected {
                name,
                hours: Count(secs / 3600),
            },
        )
    } else {
        tr(
            cx,
            ids::hv::CONN_IDLE_DISCONNECTED_MINUTES,
            ids::hv::HvConnIdleDisconnectedMinutes {
                name,
                minutes: Count(secs.div_ceil(60)),
            },
        )
    })
}

/// `profile` was disconnected after being idle (see the module docs). An event that arrives
/// after the profile connected again is stale and shows nothing.
pub(crate) fn idle(profile: ProfileId, cx: &mut App) {
    let Some(m) = cx.try_global::<ConnectionsModel>() else {
        return;
    };
    if m.manager().state(profile) != LiveState::Disconnected {
        return;
    }
    let Some(name) = name_of(cx, profile) else {
        return;
    };
    cx.default_global::<SessionToasts>().idle.insert(profile);
    cx.refresh_windows();
    if idle_text(cx, &name, profile).is_none() {
        return;
    }
    let text: TextFn = Rc::new(move |cx: &App| idle_text(cx, &name, profile).unwrap_or_default());
    let mut toast = Toast::new(ToastKind::Info, text);
    toast.icon = Some(IconName::Unplug);
    show(cx, profile, Kind::Idle, toast);
}

/// Whether `profile` was disconnected after being idle and has not connected since.
pub fn is_idle(cx: &App, profile: ProfileId) -> bool {
    cx.try_global::<SessionToasts>()
        .is_some_and(|t| t.idle.contains(&profile))
}

/// `profile` left `Disconnected` (the live mirror): it is no longer idle.
pub(crate) fn clear_idle(cx: &mut App, profile: ProfileId) {
    if let Some(t) = cx.try_global::<SessionToasts>()
        && t.idle.contains(&profile)
    {
        cx.global_mut::<SessionToasts>().idle.remove(&profile);
    }
}

/// A transaction of `profile` has been open and idle for `minutes`: a notice (M3 adds the
/// prompt that ends it).
pub(crate) fn idle_tx(profile: ProfileId, minutes: u32, cx: &mut App) {
    let Some(name) = name_of(cx, profile) else {
        return;
    };
    let text: TextFn = Rc::new(move |cx: &App| {
        tr(
            cx,
            ids::hv::CONN_IDLE_TX,
            ids::hv::HvConnIdleTx {
                name: Text(name.clone()),
                minutes: Count(u64::from(minutes)),
            },
        )
    });
    show(cx, profile, Kind::IdleTx, Toast::new(ToastKind::Info, text));
}

/// A read-only session of `profile` saw a statement escape the guard: a sticky alert whose line
/// depends on `reason` (Q35).
pub(crate) fn gap(profile: ProfileId, reason: GapReason, cx: &mut App) {
    let Some(name) = name_of(cx, profile) else {
        return;
    };
    let line = match reason {
        GapReason::TransactionEndedByStatement => ids::hv::CONN_GAP_TX_ENDED,
        GapReason::CanaryAssigned | GapReason::WriteSucceeded => ids::hv::CONN_GAP_CHANGED_DATA,
    };
    let title: TextFn = Rc::new(move |cx: &App| {
        tr(
            cx,
            ids::hv::CONN_GAP_TITLE,
            ids::hv::HvConnGapTitle {
                name: Text(name.clone()),
            },
        )
    });
    let mut toast = Toast::new(ToastKind::Info, text_fn(line));
    toast.sticky = true;
    toast.title = Some(title);
    // No new hue (NOTES M2b cross-cutting #1): an ink alert glyph.
    toast.icon = Some(IconName::TriangleAlert);
    // Fix round 1 (M7): a security alert is seen now, never queued behind sticky toasts.
    toast.urgent = true;
    show(cx, profile, Kind::Gap, toast);
    // Fix round 1 (M8): data changed on a read-only connection, so "enforced by the server"
    // no longer holds for it.
    if matches!(
        reason,
        GapReason::CanaryAssigned | GapReason::WriteSucceeded
    ) {
        cx.default_global::<SessionToasts>()
            .downgraded
            .insert(profile);
        cx.refresh_windows();
    }
}

/// The access label `profile`'s live connection shows: `access`, except that "enforced" reads
/// guarded after a gap that changed data (M8).
pub fn effective_access(cx: &App, profile: ProfileId, access: AccessLabel) -> AccessLabel {
    let downgraded = cx
        .try_global::<SessionToasts>()
        .is_some_and(|t| t.downgraded.contains(&profile));
    if downgraded && access == AccessLabel::ReadOnlyEnforced {
        AccessLabel::ReadOnlyGuarded
    } else {
        access
    }
}

/// `profile`'s connection changed (the live mirror): a new one is probed anew.
pub(crate) fn clear_downgrade(cx: &mut App, profile: ProfileId) {
    if let Some(t) = cx.try_global::<SessionToasts>()
        && t.downgraded.contains(&profile)
    {
        cx.global_mut::<SessionToasts>().downgraded.remove(&profile);
    }
}

// ## test seam (feature `test-support`)

#[cfg(feature = "test-support")]
fn kind_name(kind: Kind) -> &'static str {
    match kind {
        Kind::Lost => "lost",
        Kind::Idle => "idle",
        Kind::IdleTx => "idle-tx",
        Kind::Gap => "gap",
    }
}

/// A visible session toast as it reads now (feature `test-support`).
#[cfg(feature = "test-support")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShownToast {
    /// `lost`, `idle`, `idle-tx` or `gap`.
    pub kind: &'static str,
    pub title: Option<String>,
    /// The body line (when not empty), then each bullet.
    pub lines: Vec<String>,
    pub actions: Vec<String>,
    pub sticky: bool,
    pub urgent: bool,
}

/// The session toasts still visible, oldest first, read in the current language (feature
/// `test-support`).
#[cfg(feature = "test-support")]
pub fn shown(cx: &App) -> Vec<ShownToast> {
    let visible = ToastQueue::visible_ids(cx);
    let Some(t) = cx.try_global::<SessionToasts>() else {
        return Vec::new();
    };
    t.record
        .iter()
        .filter(|(id, _, _)| visible.contains(id))
        .map(|(_, kind, toast)| {
            let body = (toast.text)(cx).to_string();
            let mut lines: Vec<String> = Vec::new();
            if !body.is_empty() {
                lines.push(body);
            }
            lines.extend(toast.bullets.iter().map(|b| b(cx).to_string()));
            ShownToast {
                kind,
                title: toast.title.as_ref().map(|f| f(cx).to_string()),
                lines,
                actions: toast
                    .actions
                    .iter()
                    .map(|a| (a.label)(cx).to_string())
                    .collect(),
                sticky: toast.sticky,
                urgent: toast.urgent,
            }
        })
        .collect()
}
