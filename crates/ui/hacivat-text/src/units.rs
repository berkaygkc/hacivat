//! Text units for movement (UI-EDIT-06; Task 10): graphemes (`unicode-segmentation`, CRLF one
//! unit), words from the SQL tokens when the language service has them, else character
//! classes; sub-words by ASCII case transitions; smart Home.

use std::ops::Range;

use smallvec::SmallVec;

use crate::buffer::TextBuffer;

/// How far a move goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Unit {
    Grapheme,
    Word,
    SubWord,
    Line,
    Document,
}

/// Which way a move goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Dir {
    Back,
    Forward,
    Up,
    Down,
}

/// Words from the language service: the token ranges on a line (`None` when it has no
/// analysis at the current version; the class fallback applies).
pub trait TokenOracle {
    fn words_on_line(&self, line: Range<usize>) -> Option<SmallVec<[Range<usize>; 16]>>;
}

/// Where a move of `unit` in `dir` from `at` lands (Task 10). Task 1b stub: it does not move.
pub fn move_offset(
    buf: &TextBuffer,
    at: usize,
    dir: Dir,
    unit: Unit,
    tok: &dyn TokenOracle,
) -> usize {
    let _ = (buf, dir, unit, tok);
    at
}
