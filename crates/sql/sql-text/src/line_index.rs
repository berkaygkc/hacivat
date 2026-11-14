//! Line starts of a text (Task 3): byte offset ↔ line and byte column, and PostgreSQL's
//! 1-based character error cursors to bytes.

use text_size::TextSize;

/// A position as line and byte column, both 0-based.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub struct LineCol {
    pub line: u32,
    pub col: u32,
}

/// The line starts of one text (`\n`, `\r\n` and a lone `\r` end a line; Task 3).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct LineIndex {
    starts: Vec<TextSize>,
}

impl LineIndex {
    /// The index of `text` (Task 3).
    pub fn new(text: &str) -> Self {
        let _ = text;
        unimplemented!("LineIndex::new: Task 3")
    }

    /// The line and byte column of `at`.
    pub fn line_col(&self, at: TextSize) -> LineCol {
        let _ = at;
        unimplemented!("LineIndex::line_col ({} lines): Task 3", self.starts.len())
    }

    /// The offset of `lc`, when the line exists and the column is inside it.
    pub fn offset(&self, lc: LineCol) -> Option<TextSize> {
        let _ = lc;
        unimplemented!("LineIndex::offset: Task 3")
    }

    /// The byte offset of the `char_idx`-th character (0-based) of `text`; PostgreSQL's error
    /// cursor is 1-based, so the caller subtracts 1.
    pub fn char_to_byte(&self, text: &str, char_idx: usize) -> Option<TextSize> {
        let _ = (text, char_idx);
        unimplemented!("LineIndex::char_to_byte: Task 3")
    }

    pub fn len_lines(&self) -> u32 {
        unimplemented!("LineIndex::len_lines: Task 3")
    }
}
