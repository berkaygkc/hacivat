//! The grid's AccessKit rows and cells (HV-GRID-16; M3b Task 40): zero-paint row and cell
//! children of the grid's node while assistive technology is active, the active-descendant
//! focus, and the live announcements.
//!
//! ## m3b-t01b-seams: the hook (Task 1b), called once per frame by `grid::prepaint_frame` once
//! the frame's geometry is known, so Task 40 never edits `grid.rs`. STUB: builds nothing; the
//! grid's node stays M3a's (`GridView::a11y_node`: role, label, counts).

use hacivat_ui_core::gpui_surface::{Context, Window};

use super::Geometry;
use crate::source::GridFrame;
use crate::view::GridView;

/// The frame's accessibility children for `v`. STUB: nothing.
pub(crate) fn prepaint(
    v: &mut GridView,
    frame: &dyn GridFrame,
    geom: &Geometry,
    window: &mut Window,
    cx: &mut Context<GridView>,
) {
    let _ = (v, frame, geom, window, cx);
}
