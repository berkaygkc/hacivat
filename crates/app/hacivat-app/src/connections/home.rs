//! The editor area's home when no connection is open (NOTES surface 8, Task 18).
//!
//! There is no query console in M2 (L15 c): the centre shows the keyboard entry points with
//! their shortcut labels from the active keymap. Each entry is a ghost button (a tab stop) that
//! runs its command, disabled while the command cannot run.

use hacivat_l10n::{MsgRef, NoArgs, ids};
use hacivat_ui::actions::ActionRegistry;
use hacivat_ui::button::{Button, ButtonKind};
use hacivat_ui::text::{Label, TextSource};
use hacivat_ui_core::gpui_surface::{AnyElement, App, IntoElement, ParentElement, Styled, div, px};
use hacivat_ui_core::theme::{Theme, TypeStyle};

use crate::connections::model::ConnectionsModel;
use crate::l10n_global::tr;
use crate::shell::keymap_state::shortcut_label;
use crate::shell::tiles;

/// The entry points, top down: the button ID, the label and the command it runs.
const ENTRIES: [(&str, MsgRef<NoArgs>, &str); 4] = [
    ("home-new", ids::hv::HOME_NEW, "connection.new"),
    ("home-paste", ids::hv::HOME_PASTE, "connection.paste_url"),
    (
        "home-connect",
        ids::hv::HOME_CONNECT,
        "connection.toggle_connect",
    ),
    (
        "home-palette",
        ids::hv::TITLEBAR_PALETTE,
        "workspace.command_palette",
    ),
];

/// Whether the registered command `action` is enabled now.
fn enabled(cx: &App, action: &str) -> bool {
    cx.try_global::<ActionRegistry>()
        .and_then(|r| r.get(action))
        .is_some_and(|d| (d.enabled)(cx))
}

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
        .items_start()
        .gap(px(t.spacing.xs))
        .pt(px(t.spacing.lg));
    for (id, label, action) in ENTRIES {
        // Real buttons: tab stops, Enter and Space, announced as buttons.
        let mut button = Button::new(id, tr(cx, label, NoArgs))
            .kind(ButtonKind::Ghost)
            .disabled(!enabled(cx, action))
            .on_click(move |_, window, cx| {
                ActionRegistry::run(action, window, cx);
            });
        if let Some(keys) = shortcut_label(cx, action) {
            button = button.key_hint(keys.into());
        }
        rows = rows.child(button);
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
