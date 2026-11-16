//! The rope as `sql_text::TextChunks` (`RopeChunks` over a snapshot's buffer; Task 24).
//!
//! The worker reads a snapshot's rope through this, chunk by chunk, on the background executor
//! (R-M3-22): the UI thread hands over an O(1) clone of the buffer and never copies the text.

use std::borrow::Cow;
use std::ops::Range;

use hacivat_text::TextBuffer;
use sql_text::TextChunks;

/// A snapshot's buffer read as the lexer's chunked input (the text without a byte-order mark,
/// as the editor's offsets count it).
#[derive(Debug, Clone)]
pub struct RopeChunks(pub TextBuffer);

impl TextChunks for RopeChunks {
    fn len(&self) -> usize {
        self.0.len_bytes()
    }

    /// Borrowed when `r` lies inside one chunk. Panics when `r` is out of bounds or not on
    /// character boundaries, as `str` indexing does.
    fn slice(&self, r: Range<usize>) -> Cow<'_, str> {
        self.0.slice(r)
    }

    fn chunks(&self) -> Box<dyn Iterator<Item = &str> + '_> {
        Box::new(self.0.chunks_in(0..self.0.len_bytes()))
    }
}
