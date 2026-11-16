//! The lexer's chunked input (R-M3-9): a rope hands its chunks without copying the document.

use std::borrow::Cow;
use std::ops::Range;

/// Text the lexer reads in chunks. Offsets are UTF-8 bytes; slices fall on char boundaries.
pub trait TextChunks {
    /// The text's length in bytes.
    fn len(&self) -> usize;

    /// Whether the text is empty.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The text of `r`: borrowed when it lies inside one chunk.
    fn slice(&self, r: Range<usize>) -> Cow<'_, str>;

    /// The chunks in order; their concatenation is the text.
    fn chunks(&self) -> Box<dyn Iterator<Item = &str> + '_>;
}

/// One chunk: the whole string (working since Task 1b).
///
/// `slice` panics when `r` is out of bounds or not on char boundaries, as `str` indexing does.
impl TextChunks for str {
    fn len(&self) -> usize {
        str::len(self)
    }

    fn slice(&self, r: Range<usize>) -> Cow<'_, str> {
        Cow::Borrowed(&self[r])
    }

    fn chunks(&self) -> Box<dyn Iterator<Item = &str> + '_> {
        Box::new(std::iter::once(self))
    }
}

/// A `String` is its `str` (and, unlike `str`, can be a `&dyn TextChunks`).
impl TextChunks for String {
    fn len(&self) -> usize {
        self.as_str().len()
    }

    fn slice(&self, r: Range<usize>) -> Cow<'_, str> {
        TextChunks::slice(self.as_str(), r)
    }

    fn chunks(&self) -> Box<dyn Iterator<Item = &str> + '_> {
        TextChunks::chunks(self.as_str())
    }
}

/// A reference reads through (`&str` as a `&dyn TextChunks`: `&"text"`).
impl<T: TextChunks + ?Sized> TextChunks for &T {
    fn len(&self) -> usize {
        (**self).len()
    }

    fn slice(&self, r: Range<usize>) -> Cow<'_, str> {
        (**self).slice(r)
    }

    fn chunks(&self) -> Box<dyn Iterator<Item = &str> + '_> {
        (**self).chunks()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_str_is_one_borrowed_chunk() {
        let text = "SELECT 'çğ';";
        let chunks: &dyn TextChunks = &text;
        assert_eq!(chunks.len(), 14);
        assert!(!chunks.is_empty());
        assert!(matches!(chunks.slice(7..13), Cow::Borrowed("'çğ'")));
        assert_eq!(chunks.chunks().collect::<Vec<_>>(), [text]);
        assert!(TextChunks::is_empty(""));
        let owned: &dyn TextChunks = &String::from(text);
        assert_eq!(owned.slice(0..6), "SELECT");
        assert_eq!(owned.len(), 14);
    }
}
