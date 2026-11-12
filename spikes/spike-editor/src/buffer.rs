//! Text storage for the editor spike. Every text mutation goes through `Buffer`, so the
//! IME handler (Task 7) and the key handler share one edit path.

use std::ops::Range;

use ropey::{Rope, RopeSlice};

pub struct Buffer {
    rope: Rope,
    /// Bumped on every edit. Also the source of per-line versions, so a version value is
    /// never reused: a `(line, version_of_line)` key can never match stale content, even
    /// after lines shift.
    version: u64,
    /// Version of each line's current content, indexed by line.
    line_versions: Vec<u64>,
}

impl From<&str> for Buffer {
    fn from(text: &str) -> Self {
        let rope = Rope::from_str(text);
        let lines = rope.len_lines() as u64;
        Self {
            rope,
            version: lines,
            line_versions: (0..lines).collect(),
        }
    }
}

impl Buffer {
    pub fn insert(&mut self, char_idx: usize, text: &str) {
        let char_idx = char_idx.min(self.rope.len_chars());
        self.edit(char_idx..char_idx, text);
    }

    pub fn delete(&mut self, range: Range<usize>) {
        let len = self.rope.len_chars();
        let range = range.start.min(len)..range.end.min(len);
        if range.start < range.end {
            self.edit(range, "");
        }
    }

    fn edit(&mut self, range: Range<usize>, text: &str) {
        let first_line = self.rope.char_to_line(range.start);
        let old_last_line = self.rope.char_to_line(range.end);
        self.rope.remove(range.clone());
        self.rope.insert(range.start, text);
        let new_last_line = self.rope.char_to_line(range.start + text.chars().count());

        let new_lines = new_last_line - first_line + 1;
        let fresh = self.version + 1..self.version + 1 + new_lines as u64;
        self.version = fresh.end - 1;
        self.line_versions.splice(first_line..=old_last_line, fresh);

        // A CR/LF pair can merge or split across the edit boundary; fall back to fresh
        // versions for every line rather than keep a misaligned table.
        if self.line_versions.len() != self.rope.len_lines() {
            let lines = self.rope.len_lines() as u64;
            self.line_versions = (self.version + 1..self.version + 1 + lines).collect();
            self.version += lines;
        }
    }

    pub fn line_count(&self) -> usize {
        self.rope.len_lines()
    }

    /// The line's content without its trailing line break.
    pub fn line_slice(&self, line: usize) -> RopeSlice<'_> {
        let slice = self.rope.line(line);
        let mut len = slice.len_chars();
        while len > 0 && matches!(slice.char(len - 1), '\n' | '\r') {
            len -= 1;
        }
        slice.slice(..len)
    }

    pub fn line_len_chars(&self, line: usize) -> usize {
        self.line_slice(line).len_chars()
    }

    pub fn version_of_line(&self, line: usize) -> u64 {
        self.line_versions
            .get(line)
            .copied()
            .unwrap_or(self.version)
    }

    pub fn len_chars(&self) -> usize {
        self.rope.len_chars()
    }

    pub fn char_to_line(&self, char_idx: usize) -> usize {
        self.rope.char_to_line(char_idx.min(self.rope.len_chars()))
    }

    pub fn line_to_char(&self, line: usize) -> usize {
        self.rope.line_to_char(line.min(self.rope.len_lines()))
    }

    pub fn char(&self, char_idx: usize) -> char {
        self.rope.char(char_idx)
    }

    // The UTF-16 conversions serve the IME handler (Task 7), which speaks UTF-16 offsets.
    #[allow(dead_code)]
    pub fn char_to_utf16(&self, char_idx: usize) -> usize {
        self.rope
            .char_to_utf16_cu(char_idx.min(self.rope.len_chars()))
    }

    #[allow(dead_code)]
    pub fn utf16_to_char(&self, u16_idx: usize) -> usize {
        self.rope
            .utf16_cu_to_char(u16_idx.min(self.rope.len_utf16_cu()))
    }

    /// Start of the identifier (`[A-Za-z0-9_]` or any non-ASCII letter) that ends at
    /// `char_idx`, scanning back within the line.
    pub fn identifier_start(&self, char_idx: usize) -> usize {
        let line_start = self.line_to_char(self.char_to_line(char_idx));
        let mut start = char_idx;
        while start > line_start && is_identifier_char(self.rope.char(start - 1)) {
            start -= 1;
        }
        start
    }
}

pub fn is_identifier_char(c: char) -> bool {
    c == '_' || c.is_alphanumeric()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf16_roundtrip_with_turkish_and_emoji() {
        let b = Buffer::from("İı😀x");
        assert_eq!(b.char_to_utf16(3), 4);
        assert_eq!(b.utf16_to_char(4), 3);
    }

    #[test]
    fn insert_bumps_version() {
        let mut b = Buffer::from("a");
        let v = b.version;
        b.insert(1, "b");
        assert!(b.version > v);
    }

    #[test]
    fn edit_bumps_only_the_edited_line() {
        let mut b = Buffer::from("one\ntwo\nthree\n");
        let before: Vec<u64> = (0..4).map(|l| b.version_of_line(l)).collect();
        b.insert(b.line_to_char(1) + 3, "!");
        assert_eq!(b.line_slice(1).to_string(), "two!");
        assert_ne!(b.version_of_line(1), before[1]);
        for l in [0, 2, 3] {
            assert_eq!(b.version_of_line(l), before[l]);
        }
    }

    #[test]
    fn newline_keeps_line_versions_unique_and_aligned() {
        let mut b = Buffer::from("aa\nbb\ncc");
        let cc = b.version_of_line(2);
        b.insert(1, "\nx\n");
        assert_eq!(b.line_count(), 5);
        assert_eq!(b.line_slice(4).to_string(), "cc");
        // Shifted lines keep their version; new lines get versions never used before.
        assert_eq!(b.version_of_line(4), cc);
        let mut v: Vec<u64> = (0..5).map(|l| b.version_of_line(l)).collect();
        v.sort();
        v.dedup();
        assert_eq!(v.len(), 5);
    }

    #[test]
    fn delete_joins_lines() {
        let mut b = Buffer::from("ab\ncd");
        b.delete(1..4);
        assert_eq!(b.line_count(), 1);
        assert_eq!(b.line_slice(0).to_string(), "ad");
    }

    #[test]
    fn identifier_start_stops_at_punctuation_and_line_start() {
        let b = Buffer::from("x\nWHERE c.İl_ad");
        let end = b.len_chars();
        let start = b.identifier_start(end);
        assert_eq!((end - start, b.char(start)), (5, 'İ'));
        assert_eq!(b.identifier_start(b.line_to_char(1) + 3), b.line_to_char(1));
    }
}
