// ## m4-t32-shots (named exception; PO ruling R-W5-14 b)
//! "Clear query history…" for one connection: from the connection context menu
//! (`connection.clear_history`, the selected profile) and from the history dialog's ⋯ menu
//! (`history.clear`, the profile of the console the dialog is over).
//!
//! Both ask first. The question's default button is Cancel (Enter and Escape cancel); only the
//! Clear history button clears. The store deletes the profile's entries, pinned ones too, on
//! its writer (the core runtime's blocking pool); the UI thread only awaits a channel. An open
//! history dialog searches again afterwards, and a toast says what happened.

use std::rc::Rc;

use hacivat_l10n::{ArgValue, ids};
use hacivat_proto::ProfileId;
use hacivat_ui::button::ButtonKind;
use hacivat_ui::dialog::{DialogButton, DialogEvent, DialogSize, DialogSpec};
use hacivat_ui::text::TextFn;
use hacivat_ui::toast::{Toast, ToastKind, ToastQueue};
use hacivat_ui_core::gpui_surface::{App, Global, Task, WeakEntity};

use crate::connections::ConnectionsModel;
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

/// Whether `profile`'s history can be cleared: the history is open and the profile exists.
pub fn can_clear(profile: Option<ProfileId>, cx: &App) -> bool {
    super::store(cx).is_some() && profile.is_some_and(|p| profile_name(p, cx).is_some())
}

/// The open question (tests).
struct OpenQuestion(Option<(ProfileId, WeakEntity<hacivat_ui::dialog::Dialog>)>);

impl Global for OpenQuestion {}

/// The profile whose clear question is open (tests, shots).
pub fn question(cx: &App) -> Option<ProfileId> {
    let (p, d) = cx.try_global::<OpenQuestion>()?.0.as_ref()?;
    d.upgrade().map(|_| *p)
}

/// Asks whether to clear `profile`'s query history (Cancel is the default); Clear history
/// clears it.
pub fn ask(profile: ProfileId, cx: &mut App) {
    if !can_clear(Some(profile), cx) {
        return;
    }
    let Some(name) = profile_name(profile, cx) else {
        return;
    };
    let button = |label, kind| DialogButton {
        label,
        kind,
        accesskey: None,
        english_accesskey: None,
    };
    let spec = DialogSpec {
        title: text(
            "hv-history-clear-title",
            vec![("name", ArgValue::Text(name))],
        ),
        body: text("hv-history-clear-body", Vec::new()),
        buttons: vec![
            button(text("hv-history-clear-btn", Vec::new()), ButtonKind::Danger),
            button(text_fn(ids::hv::CONN_DLG_CANCEL), ButtonKind::Primary),
        ],
        cancel: CANCEL,
        primary: CANCEL,
        size: DialogSize::Standard,
    };
    crate::exec_ui::in_workspace_window(cx, move |ws, window, cx| {
        let dialog = ws.update(cx, |ws, cx| ws.open_dialog(spec, window, cx));
        cx.set_global(OpenQuestion(Some((profile, dialog.downgrade()))));
        // Held until an event comes (the workspace removes the layer on the same event).
        let mut keep = Some(dialog.clone());
        cx.subscribe(&dialog, move |_, event: &DialogEvent, cx| {
            keep.take();
            cx.set_global(OpenQuestion(None));
            if *event == DialogEvent::Pressed(CLEAR) {
                clear(profile, cx).detach();
            }
        })
        .detach();
    });
}

/// Clears `profile`'s query history now (no question): the store deletes its entries, an open
/// history dialog searches again, and a toast says whether it worked. The task answers how
/// many entries were deleted (`None`: it failed).
pub fn clear(profile: ProfileId, cx: &mut App) -> Task<Option<u64>> {
    let Some(store) = super::store(cx) else {
        return Task::ready(None);
    };
    let name = profile_name(profile, cx).unwrap_or_default();
    let work = super::on_core(store.clear(profile), cx);
    cx.spawn(async move |cx| {
        let result = work.await;
        cx.update(|cx| {
            let (kind, id) = match result {
                Ok(_) => (ToastKind::Info, "hv-history-cleared"),
                Err(_) => (ToastKind::Error, "hv-history-clear-failed"),
            };
            ToastQueue::push(
                cx,
                Toast::new(kind, text(id, vec![("name", ArgValue::Text(name))])),
            );
            super::dialog::search_again(cx);
            cx.refresh_windows();
        });
        result.ok()
    })
}
