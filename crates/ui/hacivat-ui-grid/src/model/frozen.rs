//! Frozen columns (HV-GRID-02 frozen; M3b Task 35): the first `n` visible columns stay put
//! while the body scrolls horizontally, painted in their own layer.
//!
//! ## m3b-t01b-seams: the seam (Task 1b). STUB: nothing freezes; [`ColumnLayout::frozen`]
//! answers 0, so the layout and the paint stay M3a's. Task 35 fills it.

use crate::model::ColumnLayout;

impl ColumnLayout {
    /// Freezes the first `n` visible columns (`0`: none). STUB: ignored.
    pub fn set_frozen(&mut self, n: usize) {
        let _ = n;
    }

    /// How many leading visible columns are frozen. STUB: 0.
    pub fn frozen(&self) -> usize {
        0
    }
}
