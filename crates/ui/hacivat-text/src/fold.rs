//! Folding (UI-EDIT-10; M3b Task 32): the foldable ranges a language service reports, and the
//! set of folded ranges, mapped through every change and turned into hidden lines for the
//! chunked height map (`display::HeightMap::chunked`).
//!
//! ## m3b-t01b-seams: types and stubs (Task 1b). The stub set folds nothing: every fold
//! request answers `false`, nothing is hidden, so the display stays M3a's. Task 32 fills the
//! bodies.

use std::ops::Range;

use crate::buffer::TextBuffer;
use crate::change::ChangeDesc;

/// What a foldable range is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FoldKind {
    /// One statement.
    Statement,
    /// A bracketed or `BEGIN … END` block.
    Block,
    /// A comment spanning lines.
    Comment,
    /// A `-- region` … `-- endregion` pair.
    Region,
}

/// A range the language service says can fold (byte offsets).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FoldRange {
    pub range: Range<usize>,
    pub kind: FoldKind,
}

/// The folded ranges of a document: sorted, each either disjoint from or nested in another,
/// mapped through every change.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FoldSet {
    folded: Vec<Range<usize>>,
}

impl FoldSet {
    /// Folds `r`; `true` when the set changed. STUB: `false` (nothing folds).
    pub fn fold(&mut self, r: Range<usize>) -> bool {
        let _ = r;
        false
    }

    /// Unfolds the innermost folded range holding `at`; `true` when the set changed.
    /// STUB: `false`.
    pub fn unfold_at(&mut self, at: usize) -> bool {
        let _ = at;
        false
    }

    /// Unfolds everything; `true` when anything was folded.
    pub fn unfold_all(&mut self) -> bool {
        let had = !self.folded.is_empty();
        self.folded.clear();
        had
    }

    /// Maps the folded ranges through `c` (a fold whose text is deleted goes). STUB: nothing is
    /// folded, so nothing moves.
    pub fn map(&mut self, c: &ChangeDesc) {
        let _ = c;
    }

    /// The folded ranges, sorted by start.
    pub fn folded(&self) -> &[Range<usize>] {
        &self.folded
    }

    /// The lines the folds hide (each fold keeps its first line), sorted and merged.
    /// STUB: none.
    pub fn hidden_lines(&self, buf: &TextBuffer) -> Vec<Range<u32>> {
        let _ = buf;
        Vec::new()
    }
}
