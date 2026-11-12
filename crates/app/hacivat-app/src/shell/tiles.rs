//! The tiled workspace (DESIGN.md Layout): a 276 px explorer over a 205 px connection panel on
//! the left; a 322 px editor area over the results pane in the centre. Tiles are separated by a
//! 1 px gap that shows the seam colour beneath, and the seams can be dragged.

use hacivat_l10n::ids;
use hacivat_ui::icon::{Icon, IconName, IconSize};
use hacivat_ui::text::{Label, TextSource};
use hacivat_ui_core::gpui_surface::{
    App, Div, ParentElement, Pixels, Point, SharedString, Styled, div, px,
};
use hacivat_ui_core::theme::Theme;
use serde::{Deserialize, Serialize};

use crate::l10n_global::tr;

/// The sizes of the fixed tiles, in logical pixels. Saved in `workspace.json`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct TileLayout {
    pub explorer_w: f32,
    pub conn_panel_h: f32,
    pub editor_h: f32,
    pub inspector_w: f32,
    pub explorer_visible: bool,
}

impl Default for TileLayout {
    fn default() -> Self {
        Self {
            explorer_w: 276.0,
            conn_panel_h: 205.0,
            editor_h: 322.0,
            inspector_w: 252.0,
            explorer_visible: true,
        }
    }
}

/// The smallest a tile gets by dragging a seam.
pub const MIN_TILE_PX: f32 = 120.0;
/// The widest the explorer gets.
pub const MAX_EXPLORER_PX: f32 = 560.0;
/// The invisible grab area around a 1 px seam.
const SEAM_GRAB_PX: f32 = 6.0;

/// A draggable seam.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Seam {
    /// Between the left column and the centre column (sets `explorer_w`).
    Explorer,
    /// Between the explorer and the connection panel (sets `conn_panel_h`).
    Connection,
    /// Between the editor area and the results pane (sets `editor_h`).
    Editor,
}

impl TileLayout {
    /// Moves `seam` to the pointer at `pos` (window coordinates) in a window of `size`, with the
    /// tiles starting `top` px down and ending `bottom` px above the window's bottom edge.
    pub fn drag(
        &mut self,
        seam: Seam,
        pos: Point<Pixels>,
        window: (f32, f32),
        top: f32,
        bottom: f32,
    ) {
        let (w, h) = window;
        let (x, y) = (f32::from(pos.x), f32::from(pos.y));
        let tiles_h = (h - top - bottom).max(0.0);
        match seam {
            Seam::Explorer => {
                let max = MAX_EXPLORER_PX.min(w - MIN_TILE_PX).max(MIN_TILE_PX);
                self.explorer_w = x.clamp(MIN_TILE_PX, max);
            }
            Seam::Connection => {
                let max = (tiles_h - MIN_TILE_PX).max(MIN_TILE_PX);
                self.conn_panel_h = (h - bottom - y).clamp(MIN_TILE_PX, max);
            }
            Seam::Editor => {
                let max = (tiles_h - MIN_TILE_PX).max(MIN_TILE_PX);
                self.editor_h = (y - top).clamp(MIN_TILE_PX, max);
            }
        }
    }
}

/// A 30 px panel header on `head` with a bottom seam (DESIGN.md: every panel opens with one).
pub fn panel_header(theme: &Theme) -> Div {
    let t = theme.tokens;
    let c = &t.colors;
    div()
        .flex()
        .flex_row()
        .items_center()
        .w_full()
        .h(px(t.spacing.panel_header_height))
        .flex_none()
        .pl(px(t.spacing.xl))
        .pr(px(t.spacing.lg))
        .gap(px(t.spacing.lg))
        .border_b_1()
        .border_color(c.seam.to_hsla())
        .bg(c.head.to_hsla())
}

/// A small-caps panel title (pre-cased by Fluent) with element ID `id`.
pub fn panel_title(theme: &Theme, text: SharedString, id: &'static str) -> Label {
    let t = theme.tokens;
    Label::new(
        text,
        TextSource::Fluent,
        t.type_ramp.panel_title,
        t.colors.text_2.to_hsla(),
    )
    .id(id)
}

/// A centred empty-state line at `text_3`.
pub fn empty_state(theme: &Theme, text: SharedString, id: &'static str) -> Div {
    let t = theme.tokens;
    div()
        .flex()
        .flex_1()
        .items_center()
        .justify_center()
        .px(px(t.spacing.xl))
        .child(
            Label::new(
                text,
                TextSource::Fluent,
                t.type_ramp.body,
                t.colors.text_3.to_hsla(),
            )
            .id(id),
        )
}

/// The explorer: header, filter field and the empty state (no connections in M1).
pub fn explorer(theme: &Theme, cx: &App) -> Div {
    let t = theme.tokens;
    let c = &t.colors;
    let filter = div()
        .flex()
        .flex_row()
        .items_center()
        .h(px(28.0))
        .flex_none()
        .mx(px(t.spacing.md))
        .mt(px(t.spacing.md))
        .mb(px(t.spacing.xs))
        .pl(px(t.spacing.md))
        .pr(px(t.spacing.sm))
        .gap(px(7.0))
        .border_1()
        .border_color(c.seam.to_hsla())
        .rounded(px(t.radii.sm))
        .child(
            Icon::new(IconName::Search)
                .size(IconSize::Chrome)
                .color(c.text_3.to_hsla()),
        )
        .child(
            div().flex_1().min_w(px(0.0)).child(
                Label::new(
                    tr(cx, ids::hv::EXPLORER_FILTER, hacivat_l10n::NoArgs),
                    TextSource::Fluent,
                    t.type_ramp.body,
                    c.text_3.to_hsla(),
                )
                .id("explorer-filter"),
            ),
        );
    div()
        .flex()
        .flex_col()
        .size_full()
        .bg(c.side.to_hsla())
        .child(panel_header(theme).child(panel_title(
            theme,
            tr(cx, ids::hv::PANEL_EXPLORER, hacivat_l10n::NoArgs),
            "panel-explorer-title",
        )))
        .child(filter)
        .child(empty_state(
            theme,
            tr(cx, ids::hv::EXPLORER_EMPTY, hacivat_l10n::NoArgs),
            "explorer-empty",
        ))
}

/// The connection panel with no connection.
pub fn connection_panel(theme: &Theme, cx: &App) -> Div {
    let c = &theme.tokens.colors;
    div()
        .flex()
        .flex_col()
        .size_full()
        .bg(c.side.to_hsla())
        .child(panel_header(theme).child(panel_title(
            theme,
            tr(cx, ids::hv::PANEL_CONNECTION, hacivat_l10n::NoArgs),
            "panel-connection-title",
        )))
        .child(empty_state(
            theme,
            tr(cx, ids::hv::CAPSULE_NO_CONNECTION, hacivat_l10n::NoArgs),
            "connection-empty",
        ))
}

/// The results pane before a query has run.
pub fn results(theme: &Theme, cx: &App) -> Div {
    let c = &theme.tokens.colors;
    div()
        .flex()
        .flex_col()
        .size_full()
        .bg(c.panel.to_hsla())
        .child(empty_state(
            theme,
            tr(cx, ids::hv::RESULTS_EMPTY, hacivat_l10n::NoArgs),
            "results-empty",
        ))
}

/// The grab areas of the visible seams, positioned inside the tiles row (which is `relative`).
/// Each is a 6 px band centred on its 1 px seam; the caller adds the drag handlers.
pub fn seam_grabs(l: &TileLayout) -> Vec<(Seam, Div)> {
    let half = SEAM_GRAB_PX / 2.0;
    let mut grabs = Vec::new();
    let centre_left = if l.explorer_visible {
        // The vertical seam is the 1 px gap right of the explorer.
        grabs.push((
            Seam::Explorer,
            div()
                .absolute()
                .top_0()
                .bottom_0()
                .left(px(l.explorer_w + 0.5 - half))
                .w(px(SEAM_GRAB_PX))
                .cursor_col_resize(),
        ));
        grabs.push((
            Seam::Connection,
            div()
                .absolute()
                .left_0()
                .w(px(l.explorer_w))
                .bottom(px(l.conn_panel_h + 0.5 - half))
                .h(px(SEAM_GRAB_PX))
                .cursor_row_resize(),
        ));
        l.explorer_w + 1.0
    } else {
        0.0
    };
    grabs.push((
        Seam::Editor,
        div()
            .absolute()
            .left(px(centre_left))
            .right_0()
            .top(px(l.editor_h + 0.5 - half))
            .h(px(SEAM_GRAB_PX))
            .cursor_row_resize(),
    ));
    grabs
}

#[cfg(test)]
mod tests {
    use super::*;
    use hacivat_ui_core::gpui_surface::{point, px};

    fn at(x: f32, y: f32) -> Point<Pixels> {
        point(px(x), px(y))
    }

    #[test]
    fn default_layout_matches_design_md() {
        let l = TileLayout::default();
        assert_eq!(
            (
                l.explorer_w,
                l.conn_panel_h,
                l.editor_h,
                l.inspector_w,
                l.explorer_visible
            ),
            (276.0, 205.0, 322.0, 252.0, true)
        );
    }

    #[test]
    fn dragging_a_seam_moves_it_within_bounds() {
        let mut l = TileLayout::default();
        l.drag(Seam::Explorer, at(300.0, 0.0), (1440.0, 900.0), 40.0, 24.0);
        assert_eq!(l.explorer_w, 300.0);
        l.drag(Seam::Explorer, at(10.0, 0.0), (1440.0, 900.0), 40.0, 24.0);
        assert_eq!(l.explorer_w, MIN_TILE_PX);
        l.drag(Seam::Editor, at(0.0, 440.0), (1440.0, 900.0), 40.0, 24.0);
        assert_eq!(l.editor_h, 400.0);
        l.drag(
            Seam::Connection,
            at(0.0, 576.0),
            (1440.0, 900.0),
            40.0,
            24.0,
        );
        assert_eq!(l.conn_panel_h, 300.0);
        l.drag(Seam::Editor, at(0.0, 899.0), (1440.0, 900.0), 40.0, 24.0);
        assert_eq!(l.editor_h, 900.0 - 40.0 - 24.0 - MIN_TILE_PX);
    }
}
