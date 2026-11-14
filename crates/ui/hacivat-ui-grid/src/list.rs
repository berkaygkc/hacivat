//! `VirtualList` (Task 20): one column, the grid's scroll model, a fixed row height (the
//! output log).

use hacivat_ui_core::gpui_surface::{AnyElement, App, Window};

use crate::model::ScrollState;

/// The rows of a virtual list.
pub trait RowRenderer: 'static {
    fn len(&self) -> u64;
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
    fn row(&self, ix: u64, window: &mut Window, cx: &mut App) -> AnyElement;
}

/// A virtual list over `R` (Task 20 makes it a view).
pub struct VirtualList<R: RowRenderer> {
    rows: R,
    row_height: f32,
    scroll: ScrollState,
}

impl<R: RowRenderer> std::fmt::Debug for VirtualList<R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VirtualList")
            .field("len", &self.rows.len())
            .field("row_height", &self.row_height)
            .field("scroll", &self.scroll)
            .finish()
    }
}

impl<R: RowRenderer> VirtualList<R> {
    /// A list of `rows`, each `row_height` px tall.
    pub fn new(rows: R, row_height: f32) -> Self {
        VirtualList {
            rows,
            row_height,
            scroll: ScrollState::default(),
        }
    }

    pub fn rows(&self) -> &R {
        &self.rows
    }

    pub fn row_height(&self) -> f32 {
        self.row_height
    }
}
