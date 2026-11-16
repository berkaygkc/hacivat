//! The Transposed view's axis swap (HV-GRID-12; M3b Task 38): while
//! `GridConfig::transposed` is on, the frame the element paints shows the source's rows as
//! columns and its columns as rows.
//!
//! ## m3b-t01b-seams: the hook (Task 1b), called once per frame by `grid::prepaint_frame`
//! right after `begin_frame`, so Task 38 edits neither `grid.rs` nor `view.rs`. STUB: the
//! frame unchanged, whatever the flag (`GridView::set_transposed` already stores it).

use std::rc::Rc;

use crate::source::GridFrame;
use crate::view::GridView;

/// The frame the element paints for `v` this frame. STUB: `frame` itself.
pub(crate) fn frame(v: &mut GridView, frame: Rc<dyn GridFrame>) -> Rc<dyn GridFrame> {
    let _ = v;
    frame
}
