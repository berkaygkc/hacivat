//! The editor area's home when no connection is open (NOTES surface 8, Task 18).
//!
//! There is no query console in M2 (L15 c): the centre shows the keyboard entry points with
//! their shortcut labels from the active keymap. Each row runs its command when clicked.

use hacivat_l10n::{MsgRef, NoArgs, ids};
use hacivat_ui::actions::ActionRegistry;
use hacivat_ui::text::{Label, TextSource};
use hacivat_ui_core::gpui_surface::{
    AnyElement, App, InteractiveElement, IntoElement, ParentElement, SharedString,
    StatefulInteractiveElement, Styled, div, px,
};
use hacivat_ui_core::theme::{Theme, TypeStyle};

use crate::connections::model::ConnectionsModel;
use crate::l10n_global::tr;
use crate::shell::keymap_state::shortcut_label;
use crate::shell::tiles;

/// The entry points, top down: the label and the command it runs.
const ENTRIES: [(MsgRef<NoArgs>, &str); 4] = [
    (ids::hv::HOME_NEW, "connection.new"),
    (ids::hv::HOME_PASTE, "connection.paste_url"),
    (ids::hv::HOME_CONNECT, "connection.toggle_connect"),
    (ids::hv::TITLEBAR_PALETTE, "workspace.command_palette"),
];

/// The `editor_empty` hook; without a model the M1 empty state (P17).
pub fn element(theme: &Theme, cx: &App) -> AnyElement {
    if !cx.has_global::<ConnectionsModel>() {
        return tiles::empty_state(theme, tr(cx, ids::hv::EDITOR_EMPTY, NoArgs), "editor-empty")
            .into_any_element();
    }
    let t = theme.tokens;
    let c = &t.colors;
    let mut rows = div()
        .flex()
        .flex_col()
        .gap(px(t.spacing.xs))
        .pt(px(t.spacing.lg));
    for (i, (label, action)) in ENTRIES.into_iter().enumerate() {
        let keys: SharedString = shortcut_label(cx, action).unwrap_or_default().into();
        rows = rows.child(
            div()
                .id(("home-entry", i))
                .flex()
                .flex_row()
                .items_center()
                .justify_between()
                .gap(px(t.spacing.s_3xl))
                .h(px(t.spacing.control_height))
                .px(px(t.spacing.md))
                .rounded(px(t.radii.sm))
                .cursor_pointer()
                .hover(|s| s.bg(c.hover.to_hsla()))
                .on_click(move |_, window, cx| {
                    ActionRegistry::run(action, window, cx);
                })
                .child(Label::new(
                    tr(cx, label, NoArgs),
                    TextSource::Fluent,
                    t.type_ramp.body,
                    c.text.to_hsla(),
                ))
                .child(Label::new(
                    keys,
                    TextSource::Shortcut,
                    t.type_ramp.mono_kbd,
                    c.text_2.to_hsla(),
                )),
        );
    }
    div()
        .flex()
        .flex_1()
        .items_center()
        .justify_center()
        .child(
            div()
                .flex()
                .flex_col()
                .w(px(360.0))
                .gap(px(t.spacing.xs))
                .child(
                    Label::new(
                        tr(cx, ids::hv::HOME_TITLE, NoArgs),
                        TextSource::Fluent,
                        TypeStyle {
                            weight: 600,
                            ..t.type_ramp.body
                        },
                        c.text.to_hsla(),
                    )
                    .id("home-title"),
                )
                .child(Label::new(
                    tr(cx, ids::hv::HOME_HINT, NoArgs),
                    TextSource::Fluent,
                    t.type_ramp.body_sm,
                    c.text_2.to_hsla(),
                ))
                .child(rows),
        )
        .into_any_element()
}
