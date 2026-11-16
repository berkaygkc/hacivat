//! Form layout (NOTES "NEW primitives", mock `.frow` / `.hintrow`): a 140 px label column and a
//! control, a 12 px column gap and a 26 px minimum row; a hint row aligned with the control
//! column; and the field well that restores text-3 inside dialogs (NOTES cross-cutting #4).
//! Rows carry no outer spacing: the form body stacks them with its own gap.

use hacivat_ui_core::gpui_surface::{
    Div, IntoElement, ParentElement, SharedString, Styled, div, px,
};
use hacivat_ui_core::theme::{Theme, ThemeMode};

use crate::text::{Label, TextSource};

/// The label column of a form row.
pub const FORM_LABEL_PX: f32 = 140.0;

/// A labelled form row: the label at 13 px `text-2` (the Raised Ink Rule on the dialog), then
/// the control cell (a row of controls with an 8 px gap).
pub fn form_row(theme: &Theme, label: SharedString, control: impl IntoElement) -> Div {
    let t = theme.tokens;
    div()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(t.spacing.xl))
        .w_full()
        .min_h(px(t.spacing.field_height))
        // m2a-t17-dialog: a label longer than the column truncates (never clips).
        .child(
            div().flex_none().w(px(FORM_LABEL_PX)).child(
                Label::new(
                    label,
                    TextSource::Fluent,
                    t.type_ramp.body,
                    t.colors.text_2.to_hsla(),
                )
                .truncate(true),
            ),
        )
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(t.spacing.md))
                .flex_1()
                .min_w_0()
                .child(control),
        )
}

/// A hint under a control, aligned with the control column. The caller's content uses 12 px
/// `text-2` (`caption`).
pub fn hint_row(theme: &Theme, content: impl IntoElement) -> Div {
    let t = theme.tokens;
    div()
        .flex()
        .flex_row()
        .gap(px(t.spacing.xl))
        .w_full()
        .child(div().flex_none().w(px(FORM_LABEL_PX)))
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(t.spacing.sm))
                .flex_1()
                .min_w_0()
                .child(content),
        )
}

/// The well behind a field: `panel` (dark) or `ground` (light) with the field radius, so text-3
/// is allowed on it inside a dialog.
pub fn field_well(theme: &Theme) -> Div {
    let t = theme.tokens;
    let fill = match theme.mode {
        ThemeMode::Dark => t.colors.panel,
        ThemeMode::Light => t.colors.ground,
    };
    div().bg(fill.to_hsla()).rounded(px(t.radii.sm))
}

#[cfg(test)]
mod tests {
    use super::*;
    use hacivat_ui_core::theme::{DARK, LIGHT};

    #[test]
    fn field_well_is_panel_in_dark_and_ground_in_light() {
        let mut dark = field_well(&Theme::new(ThemeMode::Dark));
        assert_eq!(
            dark.style().background,
            Some(DARK.colors.panel.to_hsla().into())
        );
        let mut light = field_well(&Theme::new(ThemeMode::Light));
        assert_eq!(
            light.style().background,
            Some(LIGHT.colors.ground.to_hsla().into())
        );
    }
}
