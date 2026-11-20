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
//!
//! ## m4-t33b-carries
//!
//! - **Names** (mock 8; the T24 carry): the question lists up to five of the vanished objects
//!   the engine named (`CatalogEvent::PendingConfirmation::sample`, in name order) in a well,
//!   each with its kind icon and its name verbatim in mono, then "and N more".

use std::rc::Rc;
use std::sync::Arc;

use hacivat_l10n::ArgValue;
use hacivat_meta::{NsKey, ObjectKey};
use hacivat_proto::ProfileId;
use hacivat_ui::button::ButtonKind;
use hacivat_ui::dialog::{Dialog, DialogButton, DialogEvent, DialogSize, DialogSpec};
use hacivat_ui::icon::{Icon, IconSize};
use hacivat_ui::text::{Label, TextFn, TextSource};
use hacivat_ui_core::gpui_surface::{
    App, AppContext as _, Context, Entity, Global, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, Styled as _, WeakEntity, Window, div, px,
};
use hacivat_ui_core::theme::Theme;

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

// ## m4-t33b-carries
/// The well of vanished names (mock 8): kind icon and mono name per row, then "and N more".
struct Vanished {
    sample: Arc<[ObjectKey]>,
    count: u32,
}

impl Render for Vanished {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *Theme::get(cx);
        let t = theme.tokens;
        let c = &t.colors;
        let mut well = div()
            .debug_selector(|| "mass-drop-names".to_string())
            .flex()
            .flex_col()
            .gap(px(t.spacing.s_2xs))
            .px(px(t.spacing.md))
            .py(px(t.spacing.sm))
            .rounded(px(t.radii.sm))
            .border_1()
            .border_color(c.seam.to_hsla())
            .bg(c.panel.to_hsla());
        for key in self.sample.iter() {
            well = well.child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(t.spacing.sm))
                    .min_w_0()
                    .child(
                        Icon::new(super::rows::kind_icon(key.kind))
                            .size(IconSize::Chrome)
                            .color(c.text_2.to_hsla()),
                    )
                    .child(
                        Label::new(
                            super::rows::isolate(key.name()),
                            TextSource::Catalog,
                            t.type_ramp.mono_data,
                            c.text.to_hsla(),
                        )
                        .truncate(true)
                        .id("mass-drop-name"),
                    ),
            );
        }
        let more = u64::from(self.count).saturating_sub(self.sample.len() as u64);
        if more > 0 {
            let text = super::live::text(
                cx,
                "hv-metadata-mass-more",
                vec![("n", ArgValue::Count(more))],
            );
            well = well.child(
                Label::new(
                    text,
                    TextSource::Fluent,
                    t.type_ramp.body_sm,
                    c.text_2.to_hsla(),
                )
                .id("mass-drop-more"),
            );
        }
        well
    }
}

/// Asks whether to remove `count` vanished objects of `ns` from `profile`'s cache (a refresh the
/// user asked for), naming the engine's `sample` of them.
pub fn ask(profile: ProfileId, ns: NsKey, count: u32, sample: Arc<[ObjectKey]>, cx: &mut App) {
    if open_dialog(profile, &ns, cx).is_some() {
        return;
    }
    crate::exec_ui::in_workspace_window(cx, move |ws, window, cx| {
        let dialog = ws.update(cx, |ws, cx| ws.open_dialog(spec(&ns, count), window, cx));
        // ## m4-t33b-carries: the names, when the engine gave some.
        if !sample.is_empty() {
            let names = cx.new(|_| Vanished { sample, count });
            dialog.update(cx, |d, cx| {
                d.set_content(names.into());
                cx.notify();
            });
        }
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
