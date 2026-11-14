//! Column layout (Task 17): view order, widths, `f64` prefix sums over the visible columns,
//! hidden columns; a width change updates only the suffix.

use crate::source::ColId;

/// The columns as the grid lays them out.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ColumnLayout {
    order: Vec<ColId>,
    widths: Vec<f32>,
    prefix: Vec<f64>,
    hidden: Vec<ColId>,
}

impl ColumnLayout {
    /// The columns in view order (Task 17 fills the layout).
    pub fn order(&self) -> &[ColId] {
        &self.order
    }

    /// Hidden columns.
    pub fn hidden(&self) -> &[ColId] {
        &self.hidden
    }

    /// The number of laid-out widths and prefix sums (equal once Task 17 builds them).
    pub fn len(&self) -> usize {
        self.widths.len().min(self.prefix.len())
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}
