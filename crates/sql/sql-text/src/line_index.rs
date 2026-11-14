//! Line starts of a text (Task 3): byte offset ↔ line and byte column, and PostgreSQL's
//! 1-based character error cursors to bytes.

use text_size::TextSize;

/// A position as line and byte column, both 0-based.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub struct LineCol {
    pub line: u32,
    pub col: u32,
}

/// The lines of one text: `\n`, `\r\n` and a lone `\r` each end a line, so a text with `n`
/// breaks has `n + 1` lines (the last one may be empty).
///
/// Offsets are UTF-8 bytes. Texts in `crates/sql` stay below 4 GiB ([`TextSize`] is a `u32`);
/// a longer text saturates at `u32::MAX` instead of panicking.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct LineIndex {
    /// The first byte of each line; `starts[0] == 0`.
    starts: Vec<TextSize>,
    /// The end of each line's content, before its break (the text's end on the last line).
    ends: Vec<TextSize>,
}

fn size(n: usize) -> TextSize {
    TextSize::from(u32::try_from(n).unwrap_or(u32::MAX))
}

impl LineIndex {
    /// The index of `text`, in one pass over its bytes.
    pub fn new(text: &str) -> Self {
        let bytes = text.as_bytes();
        let mut starts = vec![TextSize::from(0)];
        let mut ends = Vec::new();
        let mut i = 0;
        while i < bytes.len() {
            let break_len = match bytes[i] {
                b'\n' => 1,
                b'\r' if bytes.get(i + 1) == Some(&b'\n') => 2,
                b'\r' => 1,
                _ => 0,
            };
            if break_len == 0 {
                i += 1;
                continue;
            }
            ends.push(size(i));
            i += break_len;
            starts.push(size(i));
        }
        ends.push(size(bytes.len()));
        LineIndex { starts, ends }
    }

    /// The line and byte column of `at`. Offsets past the end clamp to the end; an offset
    /// between the `\r` and `\n` of a CRLF names the line that break ends, one column past its
    /// content.
    pub fn line_col(&self, at: TextSize) -> LineCol {
        let end = self.ends.last().copied().unwrap_or_default();
        let at = at.min(end);
        // The last line starting at or before `at`; `starts[0] == 0`, so there is one.
        let line = self.starts.partition_point(|&s| s <= at).saturating_sub(1);
        LineCol {
            line: u32::try_from(line).unwrap_or(u32::MAX),
            col: u32::from(at - self.starts[line]),
        }
    }

    /// The offset of `lc`, when the line exists and the column lies within its content (the
    /// column just past the content, before the break, included). The column is in bytes:
    /// without the text the index cannot tell a column inside a multi-byte character.
    pub fn offset(&self, lc: LineCol) -> Option<TextSize> {
        let line = usize::try_from(lc.line).ok()?;
        let (start, end) = (*self.starts.get(line)?, *self.ends.get(line)?);
        let at = start.checked_add(TextSize::from(lc.col))?;
        (at <= end).then_some(at)
    }

    /// The byte offset of the `char_idx`-th character (0-based) of `text` (the text PostgreSQL
    /// was sent, normally the one this index was built from). PostgreSQL's error cursor is
    /// 1-based, so the caller subtracts 1. One past the last character is the text's end (an
    /// error at the end of the input); further is `None`. Linear in `char_idx`.
    pub fn char_to_byte(&self, text: &str, char_idx: usize) -> Option<TextSize> {
        text.char_indices()
            .map(|(at, _)| at)
            .chain(std::iter::once(text.len()))
            .nth(char_idx)
            .map(size)
    }

    /// The number of lines: one more than the number of line breaks.
    pub fn len_lines(&self) -> u32 {
        u32::try_from(self.starts.len()).unwrap_or(u32::MAX)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starts_and_ends_pair_up() {
        let ix = LineIndex::new("ab\r\n\rc");
        assert_eq!(ix.starts.len(), ix.ends.len());
        let pairs: Vec<(u32, u32)> = ix
            .starts
            .iter()
            .zip(&ix.ends)
            .map(|(&s, &e)| (s.into(), e.into()))
            .collect();
        assert_eq!(pairs, [(0, 2), (4, 4), (5, 6)]);
    }
}
