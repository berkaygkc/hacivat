//! The sampled column auto-size, wired to the frame (HV-GRID-04; M3a Task 26 over Task 17's
//! `model::autosize`).
//!
//! The first frame never waits: columns start at their source's default widths. Once the first
//! rows of a generation are loaded (or the result is complete without rows), the grid measures
//! each column's sample rows (`model::autosize::sample_rows`: the first 50, the last 10 and 20
//! between) with the same shapes its cells paint, and the header's name and type, then sets
//! every width in one move and fills the viewport when there are few columns.
//!
//! - **Time-sliced.** Shaping needs the window's text system, so the measurement runs on the UI
//!   thread, in prepaint, at most [`AUTOSIZE_MEASURES_PER_FRAME`] cells per frame (whole
//!   columns; at least one column per frame). A frame that leaves columns unmeasured asks for
//!   the next frame. The budget is a count, not a clock, so a frame's work does not depend on
//!   the machine.
//! - **Shared shapes.** A measured cell's shape lands in the grid's text cache, so the visible
//!   rows, which are among the samples, are not shaped twice.
//! - **Once per column set.** A re-run that keeps the column names keeps the widths and is not
//!   measured again; nothing resizes after the layout settled. A column the user sized (a
//!   header-edge drag, or the host's `set_column_width`) is never auto-sized.

use hacivat_ui_core::gpui_surface::{App, Window};

use super::layers::{
    CELL_PAD_PX, ERROR_GAP_PX, ERROR_ICON_PX, TRAIL_GAP_PX, TRAIL_PAD_PX, cell_text,
};
use super::text_cache::{Budget, with_shape};
use crate::model::autosize::{
    MAX_HEADER_WIDTH, MAX_INITIAL_WIDTH, column_width, fill, is_text_like, sample_rows,
};
use crate::source::{CellKind, ColId, GridFrame, ModelRow};
use crate::view::GridView;

/// The most cells one frame measures for the auto-size (whole columns; a column of 80 samples
/// is never split, and a frame always measures at least one column).
pub const AUTOSIZE_MEASURES_PER_FRAME: u32 = 240;

/// The auto-size of one column set.
#[derive(Debug, Default)]
pub(crate) struct AutosizeState {
    /// Columns are still to be measured.
    pending: bool,
    /// The sample rows, fixed when the measurement starts.
    samples: Option<Vec<u64>>,
    /// The next column to measure (an index into the source's columns).
    next: usize,
    /// The measured widths so far.
    widths: Vec<(ColId, f32)>,
    /// Columns the user sized: never auto-sized.
    user_sized: Vec<ColId>,
}

impl AutosizeState {
    /// A new column set (the first frame, or a re-run whose columns changed): measure it.
    pub fn restart(&mut self) {
        self.pending = true;
        self.samples = None;
        self.next = 0;
        self.widths.clear();
        self.user_sized.clear();
    }

    /// Columns are still to be measured.
    pub fn pending(&self) -> bool {
        self.pending
    }

    /// The user (or the host, restoring a width) sized `col`.
    pub fn mark_user_sized(&mut self, col: ColId) {
        if !self.user_sized.contains(&col) {
            self.user_sized.push(col);
        }
    }
}

/// One frame's share of the auto-size, before the frame's geometry is laid out: measures up to
/// the frame's budget and, once every column is measured, sets the widths. Answers the cells
/// measured in this frame.
pub(crate) fn step(
    v: &mut GridView,
    frame: &dyn GridFrame,
    body_w: f32,
    window: &mut Window,
    cx: &mut App,
) -> u32 {
    if !v.config.auto_size || !v.autosize.pending {
        return 0;
    }
    let count = v.data.count;
    if count.loaded == 0 && count.more {
        // The first rows are still arriving: the default widths stand until they are here.
        return 0;
    }
    let GridView {
        autosize: st,
        paint,
        columns,
        layout,
        scroll,
        ..
    } = v;
    let Some(roles) = paint.roles.as_ref() else {
        return 0;
    };
    let samples = st.samples.get_or_insert_with(|| sample_rows(count.loaded));
    let cell_avail = MAX_INITIAL_WIDTH - 2.0 * CELL_PAD_PX;
    let mut budget = Budget::new(u32::MAX);
    let mut measured = 0u32;
    let mut widths: Vec<f32> = Vec::with_capacity(samples.len());
    while st.next < columns.len() && (measured == 0 || measured < AUTOSIZE_MEASURES_PER_FRAME) {
        let spec = &columns[st.next];
        st.next += 1;
        if st.user_sized.contains(&spec.id) {
            continue;
        }
        let mut header_w = 0.0f32;
        for (text, role) in [
            (&spec.name, &roles.header_name),
            (&spec.type_label, &roles.header_type),
        ] {
            if text.is_empty() {
                continue;
            }
            let w = with_shape(
                &mut paint.cache,
                text,
                role,
                &roles.chip,
                MAX_HEADER_WIDTH,
                false,
                &mut budget,
                &mut paint.scratch,
                window,
                cx,
                |s, _, _| s.width,
            );
            header_w = header_w.max(w.unwrap_or(0.0));
        }
        widths.clear();
        for &row in samples.iter() {
            if row >= count.loaded {
                continue;
            }
            let cell = frame.cell(ModelRow(row), spec.id);
            if cell.kind == CellKind::Placeholder {
                continue;
            }
            let mut extra = 2.0 * CELL_PAD_PX;
            if cell.kind == CellKind::Error {
                extra += ERROR_ICON_PX + ERROR_GAP_PX;
            }
            if let Some(chip) = cell.chip.as_ref() {
                let w = with_shape(
                    &mut paint.cache,
                    chip,
                    &roles.chip,
                    &roles.chip,
                    cell_avail,
                    false,
                    &mut budget,
                    &mut paint.scratch,
                    window,
                    cx,
                    |s, _, _| s.width,
                );
                extra += w.unwrap_or(0.0) + 2.0 * TRAIL_PAD_PX + 2.0 + TRAIL_GAP_PX;
            }
            let text = cell_text(cell.kind, &cell.text);
            let role = roles.for_cell(cell.kind, spec.kind);
            let w = with_shape(
                &mut paint.cache,
                &text,
                role,
                &roles.chip,
                cell_avail,
                false,
                &mut budget,
                &mut paint.scratch,
                window,
                cx,
                |s, _, _| s.width,
            );
            widths.push(w.unwrap_or(0.0) + extra);
            measured += 1;
        }
        st.widths
            .push((spec.id, column_width(&widths, header_w + 2.0 * CELL_PAD_PX)));
    }
    if st.next < columns.len() {
        return measured;
    }

    // Every column is measured: one move to the new widths, then the fill.
    let order: Vec<ColId> = layout.order().to_vec();
    let mut new_widths: Vec<f32> = order
        .iter()
        .map(|id| {
            st.widths
                .iter()
                .find(|(col, _)| col == id)
                .map(|(_, w)| *w)
                .or_else(|| layout.width_of(*id))
                .unwrap_or(0.0)
        })
        .collect();
    if layout.hidden().is_empty() {
        let text_like: Vec<bool> = order
            .iter()
            .map(|id| {
                !st.user_sized.contains(id)
                    && columns
                        .iter()
                        .find(|s| s.id == *id)
                        .is_some_and(|s| is_text_like(s.kind))
            })
            .collect();
        fill(&mut new_widths, &text_like, body_w);
    }
    for (id, w) in order.iter().zip(new_widths) {
        if !st.user_sized.contains(id) {
            layout.set_width(*id, w);
        }
    }
    scroll.set_content_width(layout.total_width());
    st.pending = false;
    st.samples = None;
    st.widths.clear();
    measured
}
