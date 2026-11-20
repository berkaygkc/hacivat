//! The mass-deletion guard's UI (Task 24; D-039, R-M4-22, mock 8): a user-triggered refresh asks
//! (Keep cached is the default), a background pass marks the namespace stale.
//!
//! ## m4-t24-live
//!
//! - **User refresh** ([`ask`]): an alertdialog "{count} objects in {schema} are no longer
//!   visible", the body says removing them changes nothing on the server. Buttons: **Remove from
//!   cache** (Default, first) and **Keep cached** (Primary, the initial focus). Neither is
//!   Danger: nothing on the server changes. Escape and the close icon answer Keep cached. The
//!   answer goes to the engine (`IntroHandle::confirm_pending`): only "Remove from cache"
//!   applies the deletions.
//! - **Background pass**: nothing is removed and no dialog opens; the namespace keeps its "out of
//!   date" badge, whose tooltip counts the objects that disappeared (the adapter, from
//!   [`super::live::LiveRow::pending`]).
//! - **Withdrawn** ([`withdrawn`]): a later pass of the namespace replaced the held merge; an
//!   open question about it closes as Keep cached would (nothing is applied).

use std::rc::Rc;

use hacivat_l10n::ArgValue;
use hacivat_meta::NsKey;
use hacivat_proto::ProfileId;
use hacivat_ui::button::ButtonKind;
use hacivat_ui::dialog::{Dialog, DialogButton, DialogEvent, DialogSize, DialogSpec};
use hacivat_ui::text::TextFn;
use hacivat_ui_core::gpui_surface::{App, Entity, Global, WeakEntity};

use crate::l10n_global::L10nGlobal;

/// The dialog's buttons, in order.
pub const REMOVE: usize = 0;
pub const KEEP: usize = 1;

/// The open question: its profile, namespace and dialog.
#[derive(Default)]
struct Open(Vec<(ProfileId, NsKey, WeakEntity<Dialog>)>);

impl Global for Open {}

fn text(id: &'static str, args: Vec<(&'static str, ArgValue)>) -> TextFn {
    Rc::new(move |cx: &App| match cx.try_global::<L10nGlobal>() {
        Some(g) => g.l10n.t_dynamic(id, args.clone()).into(),
        None => id.into(),
    })
}

/// The confirmation for `count` objects of `ns`.
pub fn spec(ns: &NsKey, count: u32) -> DialogSpec {
    let schema = ns.root_name().to_string();
    let button = |label, kind| DialogButton {
        label,
        kind,
        accesskey: None,
        english_accesskey: None,
    };
    DialogSpec {
        title: text(
            "hv-metadata-mass-title",
            vec![
                ("count", ArgValue::Count(u64::from(count))),
                ("schema", ArgValue::Text(schema)),
            ],
        ),
        body: text("hv-metadata-mass-body", Vec::new()),
        buttons: vec![
            button(
                text("hv-metadata-mass-remove", Vec::new()),
                ButtonKind::Default,
            ),
            button(
                text("hv-metadata-mass-keep", Vec::new()),
                ButtonKind::Primary,
            ),
        ],
        cancel: KEEP,
        primary: KEEP,
        size: DialogSize::Wide,
    }
}

/// The open question's dialog for `ns` of `profile` (tests).
pub fn open_dialog(profile: ProfileId, ns: &NsKey, cx: &App) -> Option<Entity<Dialog>> {
    cx.try_global::<Open>()?
        .0
        .iter()
        .find(|(p, n, _)| *p == profile && n == ns)
        .and_then(|(_, _, d)| d.upgrade())
}

/// Asks whether to remove `count` vanished objects of `ns` from `profile`'s cache (a refresh the
/// user asked for).
pub fn ask(profile: ProfileId, ns: NsKey, count: u32, cx: &mut App) {
    if open_dialog(profile, &ns, cx).is_some() {
        return;
    }
    crate::exec_ui::in_workspace_window(cx, move |ws, window, cx| {
        let dialog = ws.update(cx, |ws, cx| ws.open_dialog(spec(&ns, count), window, cx));
        cx.default_global::<Open>()
            .0
            .push((profile, ns.clone(), dialog.downgrade()));
        cx.subscribe(&dialog, move |_, event: &DialogEvent, cx| {
            let accept = *event == DialogEvent::Pressed(REMOVE);
            answer(profile, &ns, accept, cx);
        })
        .detach();
    });
}

/// Sends the answer to the engine and forgets the question.
fn answer(profile: ProfileId, ns: &NsKey, accept: bool, cx: &mut App) {
    if let Some(open) = cx.try_global::<Open>()
        && open.0.iter().any(|(p, n, _)| *p == profile && n == ns)
    {
        cx.default_global::<Open>()
            .0
            .retain(|(p, n, _)| !(*p == profile && n == ns));
    }
    if let Some(h) = super::live::handle(profile, cx) {
        drop(h.confirm_pending(ns.clone(), accept));
    }
}

/// The engine withdrew the held deletion of `ns`: an open question closes, applying nothing.
pub fn withdrawn(profile: ProfileId, ns: &NsKey, cx: &mut App) {
    let Some(dialog) = open_dialog(profile, ns, cx) else {
        return;
    };
    cx.default_global::<Open>()
        .0
        .retain(|(p, n, _)| !(*p == profile && n == ns));
    crate::exec_ui::in_workspace_window(cx, move |ws, window, cx| {
        let id = dialog.entity_id();
        ws.update(cx, |ws, cx| ws.remove_layer(id, window, cx));
    });
}
