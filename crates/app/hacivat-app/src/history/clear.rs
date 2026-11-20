// ## m4-t32-shots (named exception; PO ruling R-W5-14 b)
//! "Clear query history…" for one connection: from the connection context menu
//! (`connection.clear_history`, the selected profile) and from the history dialog's ⋯ menu
//! (`history.clear`, the profile of the console the dialog is over).
//!
//! Both ask first. The question's default button is Cancel (Enter and Escape cancel); only the
//! Clear history button clears. The store deletes the profile's entries, pinned ones too, on
//! its writer (the core runtime's blocking pool); the UI thread only awaits a channel. An open
//! history dialog searches again afterwards, and a toast says what happened.
//!
//! ## m4-t33b-carries
//!
//! - **Damaged backups** (the T32 carry, ruling 2026-11-20). A damaged history file is set
//!   aside as `history.sqlite.corrupt-*` when the history opens; such a copy holds every
//!   connection's statements as they were, and nothing can open it to clear one connection.
//!   Before the question opens, the folder is looked at on the core runtime: when copies
//!   exist, the question says so and offers "Also delete damaged history backups" (off by
//!   default; Tab reaches it first, Space checks it). Checked, Clear history deletes them too.
//! - **History off** (T32 re-review RR-2). The store opened just for the clear runs no
//!   retention (every limit 0), and when it finds the file damaged (it sets it aside, as a
//!   start does) the toast says the history was not cleared, never "cleared".
//! - **Recalls** (RR-1): the clear bumps the profile's "cleared" count when it starts and when
//!   it ends, so a console recall read that overlaps it shows nothing.

use std::cell::Cell;
use std::rc::Rc;

use hacivat_l10n::{ArgValue, ids};
use hacivat_proto::ProfileId;
use hacivat_ui::button::ButtonKind;
use hacivat_ui::checkbox::Checkbox;
use hacivat_ui::dialog::{DialogButton, DialogEvent, DialogSize, DialogSpec};
use hacivat_ui::text::TextFn;
use hacivat_ui::toast::{Toast, ToastKind, ToastQueue};
use hacivat_ui_core::gpui_surface::{
    App, AppContext as _, Context, FocusHandle, Global, IntoElement, ParentElement, Render, Task,
    WeakEntity, Window, div,
};

use crate::connections::ConnectionsModel;
use futures::FutureExt as _;
use hacivat_exec::{HistoryConfig, HistoryError, HistoryStore};

/// How long a store opened only for a clear may take to close.
const CLOSE_GRACE: std::time::Duration = std::time::Duration::from_secs(2);
use crate::l10n_global::text_fn;

/// The Clear history button's index; Cancel is the default.
const CLEAR: usize = 0;
const CANCEL: usize = 1;

fn text(id: &'static str, args: Vec<(&'static str, ArgValue)>) -> TextFn {
    Rc::new(move |cx: &App| crate::explorer::live::text(cx, id, args.clone()))
}

fn profile_name(profile: ProfileId, cx: &App) -> Option<String> {
    cx.try_global::<ConnectionsModel>()?
        .get(profile)
        .map(|p| p.name.clone())
}

/// Whether `profile`'s history can be cleared: the profile exists. Fix round 1 (review M-2):
/// with the history turned off the file may still hold its statements, so the clear opens the
/// file just for itself ([`clear`]).
pub fn can_clear(profile: Option<ProfileId>, cx: &App) -> bool {
    cx.try_global::<crate::AppState>().is_some()
        && profile.is_some_and(|p| profile_name(p, cx).is_some())
}

/// The open question (tests).
struct OpenQuestion(Option<(ProfileId, WeakEntity<hacivat_ui::dialog::Dialog>)>);

impl Global for OpenQuestion {}

/// The profile whose clear question is open (tests, shots).
pub fn question(cx: &App) -> Option<ProfileId> {
    let (p, d) = cx.try_global::<OpenQuestion>()?.0.as_ref()?;
    d.upgrade().map(|_| *p)
}

fn history_path(cx: &App) -> Option<std::path::PathBuf> {
    cx.try_global::<crate::AppState>()
        .map(|s| s.paths.history_db())
}

// ## m4-t33b-carries
/// The question's "Also delete damaged history backups" checkbox (shown only when copies
/// exist): a view in the dialog's content slot, with its own tab stop.
struct BackupsOffer {
    count: usize,
    checked: Rc<Cell<bool>>,
    focus: FocusHandle,
}

impl Render for BackupsOffer {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let checked = self.checked.clone();
        let me = cx.weak_entity();
        let label = crate::explorer::live::text(cx, "hv-history-clear-backups", Vec::new());
        let sub = crate::explorer::live::text(
            cx,
            "hv-history-clear-backups-note",
            vec![("count", ArgValue::Count(self.count as u64))],
        );
        div().child(
            Checkbox::new("history-clear-backups", label)
                .sub_label(sub)
                .checked(checked.get())
                .focus_handle(self.focus.clone())
                .on_toggle(move |on, _, cx| {
                    checked.set(on);
                    let _ = me.update(cx, |_, cx| cx.notify());
                }),
        )
    }
}

/// Asks whether to clear `profile`'s query history (Cancel is the default); Clear history
/// clears it. The question opens once the history folder was looked at for damaged backups
/// (on the core runtime).
pub fn ask(profile: ProfileId, cx: &mut App) {
    if !can_clear(Some(profile), cx) {
        return;
    }
    // ## m4-t33b-carries (fix round 1, review 7): one question per profile, also while its
    // folder listing is on its way (no modal layer holds the window yet).
    if question(cx) == Some(profile) || cx.default_global::<Asking>().0.contains(&profile) {
        return;
    }
    let listing = match history_path(cx) {
        Some(path) => super::on_core(HistoryStore::damaged_backups(path), cx),
        None => return,
    };
    cx.default_global::<Asking>().0.insert(profile);
    cx.spawn(async move |cx| {
        // A folder that can't be read offers nothing (the clear itself still works).
        let copies = listing.await.unwrap_or_default();
        cx.update(|cx| {
            open_question(profile, copies, cx);
            // Fix round 2 (re-review R-1): the guard covers only the listing. This runs after
            // the deferred open (FIFO), which has set the weak `OpenQuestion` that `ask` checks
            // while the question is open; a question dropped without an event, or never opened
            // (no workspace window), leaves nothing behind.
            cx.defer(move |cx| {
                cx.default_global::<Asking>().0.remove(&profile);
            });
        });
    })
    .detach();
}

// ## m4-t33b-carries (fix round 1, review 7)
/// The profiles whose clear question is being prepared (its folder listing runs; once it
/// opens, the weak [`question`] covers it).
#[derive(Default)]
struct Asking(std::collections::HashSet<ProfileId>);

impl Global for Asking {}

/// Whether `profile`'s clear question is being prepared (tests).
pub fn asking(profile: ProfileId, cx: &App) -> bool {
    cx.try_global::<Asking>()
        .is_some_and(|a| a.0.contains(&profile))
}

/// Opens the question (in the workspace window, after this update); `copies` are the damaged
/// backups listed for it (the only ones the clear may delete, review 3).
fn open_question(profile: ProfileId, copies: Vec<std::path::PathBuf>, cx: &mut App) {
    let backups = copies.len();
    let Some(name) = profile_name(profile, cx) else {
        return;
    };
    let button = |label, kind| DialogButton {
        label,
        kind,
        accesskey: None,
        english_accesskey: None,
    };
    let body = if backups == 0 {
        text("hv-history-clear-body", Vec::new())
    } else {
        text(
            "hv-history-clear-body-backups",
            vec![("count", ArgValue::Count(backups as u64))],
        )
    };
    let spec = DialogSpec {
        title: text(
            "hv-history-clear-title",
            vec![("name", ArgValue::Text(name))],
        ),
        body,
        buttons: vec![
            button(text("hv-history-clear-btn", Vec::new()), ButtonKind::Danger),
            button(text_fn(ids::hv::CONN_DLG_CANCEL), ButtonKind::Primary),
        ],
        cancel: CANCEL,
        primary: CANCEL,
        size: DialogSize::Standard,
    };
    let delete_backups = Rc::new(Cell::new(false));
    crate::exec_ui::in_workspace_window(cx, move |ws, window, cx| {
        let dialog = ws.update(cx, |ws, cx| ws.open_dialog(spec, window, cx));
        if backups > 0 {
            let checked = delete_backups.clone();
            let offer = cx.new(|cx| BackupsOffer {
                count: backups,
                checked,
                focus: cx.focus_handle(),
            });
            let focus = offer.read(cx).focus.clone();
            dialog.update(cx, |d, cx| {
                d.set_content(offer.into());
                d.set_content_focus(vec![focus]);
                cx.notify();
            });
        }
        cx.set_global(OpenQuestion(Some((profile, dialog.downgrade()))));
        // Fix round 2 (re-review R-1): no strong handle here. The workspace holds the layer
        // and keeps the dialog alive until its event reached every subscriber
        // (`Workspace::open_dialog`); a layer removed without an event (`pop_layer`, the
        // window closing) releases it, so the weak `OpenQuestion` reads closed.
        let mut copies = copies;
        cx.subscribe(&dialog, move |_, event: &DialogEvent, cx| {
            cx.set_global(OpenQuestion(None));
            if *event == DialogEvent::Pressed(CLEAR) {
                let delete = if delete_backups.get() {
                    std::mem::take(&mut copies)
                } else {
                    Vec::new()
                };
                clear_with(profile, delete, cx).detach();
            }
        })
        .detach();
    });
}

/// How a clear went.
enum Cleared {
    /// The entries were deleted (and this many damaged backups).
    Done { backups: u64 },
    /// The store opened just for the clear found the file damaged and set it aside: nothing
    /// was cleared from it (RR-2).
    Damaged,
    // ## m4-t33b-carries (fix round 1, review 6)
    /// The entries were deleted; the damaged backups could not be.
    BackupsFailed,
}

/// Clears `profile`'s query history now (no question): the store deletes its entries, an open
/// history dialog searches again, and a toast says whether it worked. The task answers how
/// many entries were deleted (`None`: it failed).
pub fn clear(profile: ProfileId, cx: &mut App) -> Task<Option<u64>> {
    clear_with(profile, Vec::new(), cx)
}

/// Clears `profile`'s history and deletes the damaged backups `backups` (the ones the question
/// listed; none: keep every backup).
fn clear_with(
    profile: ProfileId,
    backups: Vec<std::path::PathBuf>,
    cx: &mut App,
) -> Task<Option<u64>> {
    let name = profile_name(profile, cx).unwrap_or_default();
    let Some(path) = history_path(cx) else {
        return Task::ready(None);
    };
    // ## m4-t33b-carries (RR-1): a recall read from now on, or already on its way, is stale.
    crate::console::commands::note_cleared(profile, cx);
    let entries = match super::store(cx) {
        Some(store) => store.clear(profile).map(|r| r.map(|n| (n, false))).boxed(),
        // Fix round 1 (review M-2): the history is off, so no store is open: open the file
        // for this clear only (no file: nothing to clear), then close it. Nothing records.
        // ## m4-t33b-carries (RR-2): no retention, and a damaged file is not "cleared".
        None => {
            let path = path.clone();
            async move {
                if !path.exists() {
                    return Ok((0, false));
                }
                let no_retention = HistoryConfig {
                    per_source: 0,
                    max_age_days: 0,
                    ..HistoryConfig::default()
                };
                let store = HistoryStore::open(path, no_retention).await?;
                let damaged = store.recovered_from().is_some();
                let cleared = store.clear(profile).await;
                store.shutdown(CLOSE_GRACE).await;
                cleared.map(|n| (n, damaged))
            }
            .boxed()
        }
    };
    let work = super::on_core(
        async move {
            let (n, damaged) = entries.await?;
            // Review 3 and 6: only the listed copies; a failed delete leaves the clear done.
            let deleted = if backups.is_empty() {
                Ok(0)
            } else {
                HistoryStore::delete_damaged_backups(path, backups).await
            };
            Ok::<_, HistoryError>((
                n,
                match (damaged, deleted) {
                    (true, _) => Cleared::Damaged,
                    (false, Ok(backups)) => Cleared::Done { backups },
                    (false, Err(_)) => Cleared::BackupsFailed,
                },
            ))
        }
        .boxed(),
        cx,
    );
    cx.spawn(async move |cx| {
        let result = work.await;
        cx.update(|cx| {
            // Fix round 1 (review M-1): no console steps through cleared statements.
            crate::console::commands::forget_recalls(profile, cx);
            let named = ("name", ArgValue::Text(name));
            let (kind, label) = match &result {
                Ok((_, Cleared::Done { backups: 0 })) => {
                    (ToastKind::Info, text("hv-history-cleared", vec![named]))
                }
                Ok((_, Cleared::Done { backups })) => (
                    ToastKind::Info,
                    text(
                        "hv-history-cleared-backups",
                        vec![named, ("count", ArgValue::Count(*backups))],
                    ),
                ),
                Ok((_, Cleared::Damaged)) => (
                    ToastKind::Error,
                    text("hv-history-clear-damaged", vec![named]),
                ),
                Ok((_, Cleared::BackupsFailed)) => (
                    ToastKind::Error,
                    text("hv-history-cleared-backups-failed", vec![named]),
                ),
                Err(_) => (
                    ToastKind::Error,
                    text("hv-history-clear-failed", vec![named]),
                ),
            };
            ToastQueue::push(cx, Toast::new(kind, label));
            super::dialog::search_again(cx);
            cx.refresh_windows();
        });
        match result {
            Ok((n, Cleared::Done { .. } | Cleared::BackupsFailed)) => Some(n),
            _ => None,
        }
    })
}
