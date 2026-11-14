//! `TextBuffer`: a rope with exact line endings and the BOM in its metadata (UI-EDIT-02;
//! working since Task 1b, Task 5 extends it).
//!
//! Byte offsets are canonical. CR, LF and CRLF each end a line and round-trip byte for byte;
//! U+2028 and U+0085 are not line breaks (ropey without `unicode_lines`). A leading U+FEFF
//! moves into [`BufferMeta::bom`].

use std::borrow::Cow;
use std::fmt;
use std::ops::Range;
use std::sync::Arc;

use ropey::Rope;

/// What the buffer knows about its text besides the text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct BufferMeta {
    /// The text began with U+FEFF (not part of the buffer's text).
    pub bom: bool,
    pub line_endings: LineEndingStats,
}

/// A line ending.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LineEnding {
    Lf,
    CrLf,
    Cr,
}

/// How many line endings of each kind the text had when the buffer was made.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct LineEndingStats {
    pub lf: u32,
    pub crlf: u32,
    pub cr: u32,
}

/// Which side of a position a mapped or snapped offset keeps to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Bias {
    Before,
    After,
}

/// The editor's text: O(1) to clone, Send + Sync.
#[derive(Clone)]
pub struct TextBuffer {
    rope: Rope,
    meta: Arc<BufferMeta>,
}

impl fmt::Debug for TextBuffer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TextBuffer")
            .field("len_bytes", &self.rope.len_bytes())
            .field("len_lines", &self.rope.len_lines())
            .field("meta", &self.meta)
            .finish()
    }
}

/// The text, without the BOM: for the worker, Run and autosave only (never per keystroke).
impl fmt::Display for TextBuffer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for chunk in self.rope.chunks() {
            f.write_str(chunk)?;
        }
        Ok(())
    }
}

impl From<&str> for TextBuffer {
    fn from(text: &str) -> Self {
        TextBuffer::from_str(text)
    }
}

fn stats_of(text: &str) -> LineEndingStats {
    let mut stats = LineEndingStats::default();
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'\r' if bytes.get(i + 1) == Some(&b'\n') => {
                stats.crlf += 1;
                i += 1;
            }
            b'\r' => stats.cr += 1,
            b'\n' => stats.lf += 1,
            _ => {}
        }
        i += 1;
    }
    stats
}

impl TextBuffer {
    /// A buffer over `text`; one leading U+FEFF moves into `meta().bom`.
    #[allow(
        clippy::should_implement_trait,
        reason = "infallible: any text is a buffer, while `FromStr` is fallible"
    )]
    pub fn from_str(text: &str) -> Self {
        let (bom, body) = match text.strip_prefix('\u{FEFF}') {
            Some(rest) => (true, rest),
            None => (false, text),
        };
        TextBuffer {
            rope: Rope::from_str(body),
            meta: Arc::new(BufferMeta {
                bom,
                line_endings: stats_of(body),
            }),
        }
    }

    /// The buffer's metadata.
    pub fn meta(&self) -> &BufferMeta {
        &self.meta
    }

    pub fn len_bytes(&self) -> usize {
        self.rope.len_bytes()
    }

    /// The length in UTF-16 code units (what the OS input methods count).
    pub fn len_utf16(&self) -> usize {
        self.rope.len_utf16_cu()
    }

    /// Lines: one more than the line breaks (an empty buffer has one line).
    pub fn len_lines(&self) -> usize {
        self.rope.len_lines()
    }

    /// The text of the byte range `r` (borrowed when it lies in one chunk).
    ///
    /// # Panics
    /// When `r` is out of bounds or not on char boundaries.
    pub fn slice(&self, r: Range<usize>) -> Cow<'_, str> {
        let s = self.rope.byte_slice(r);
        match s.as_str() {
            Some(text) => Cow::Borrowed(text),
            None => Cow::Owned(s.to_string()),
        }
    }

    /// The rope's chunks over the byte range `r`.
    pub fn chunks_in(&self, r: Range<usize>) -> impl Iterator<Item = &str> + '_ {
        self.rope.byte_slice(r).chunks()
    }

    /// The UTF-16 offset of byte `b` (a char boundary).
    pub fn byte_to_utf16(&self, b: usize) -> usize {
        let char_idx = self.rope.byte_to_char(b.min(self.rope.len_bytes()));
        self.rope.char_to_utf16_cu(char_idx)
    }

    /// The byte offset of UTF-16 offset `u`; inside a surrogate pair, the pair's start.
    pub fn utf16_to_byte(&self, u: usize) -> usize {
        let char_idx = self.rope.utf16_cu_to_char(u.min(self.rope.len_utf16_cu()));
        self.rope.char_to_byte(char_idx)
    }

    /// The line holding byte `b`.
    pub fn byte_to_line(&self, b: usize) -> usize {
        self.rope.byte_to_line(b.min(self.rope.len_bytes()))
    }

    /// The first byte of line `l` (the end of the text past the last line).
    pub fn line_to_byte(&self, l: usize) -> usize {
        self.rope.line_to_byte(l.min(self.rope.len_lines()))
    }

    /// The bytes of line `l`, with or without its line break.
    pub fn line_range(&self, l: usize, with_break: bool) -> Range<usize> {
        let start = self.line_to_byte(l);
        let end = self.line_to_byte(l + 1);
        if with_break {
            return start..end;
        }
        let line = self.slice(start..end);
        let body = line
            .strip_suffix("\r\n")
            .or_else(|| line.strip_suffix('\n'))
            .or_else(|| line.strip_suffix('\r'))
            .unwrap_or(&line);
        start..start + body.len()
    }

    /// `b` moved to a char boundary on `bias`'s side, never between the CR and LF of a CRLF.
    pub fn snap(&self, b: usize, bias: Bias) -> usize {
        let len = self.rope.len_bytes();
        let b = b.min(len);
        let char_idx = self.rope.byte_to_char(b);
        let mut at = self.rope.char_to_byte(char_idx);
        if at != b && bias == Bias::After {
            at = self.rope.char_to_byte(char_idx + 1);
        }
        let inside_crlf =
            at > 0 && at < len && self.rope.byte(at - 1) == b'\r' && self.rope.byte(at) == b'\n';
        match (inside_crlf, bias) {
            (true, Bias::Before) => at - 1,
            (true, Bias::After) => at + 1,
            (false, _) => at,
        }
    }

    /// The most common line ending (LF when there is none; ties go LF, CRLF, CR).
    pub fn dominant_line_ending(&self) -> LineEnding {
        let s = self.meta.line_endings;
        if s.crlf > s.lf && s.crlf >= s.cr {
            LineEnding::CrLf
        } else if s.cr > s.lf && s.cr > s.crlf {
            LineEnding::Cr
        } else {
            LineEnding::Lf
        }
    }

    /// The XXH3-128 of the text (twox-hash, streamed over the chunks; R-M3-4).
    pub fn content_hash(&self) -> u128 {
        let mut h = twox_hash::XxHash3_128::new();
        for chunk in self.rope.chunks() {
            h.write(chunk.as_bytes());
        }
        h.finish_128()
    }

    /// The rope (for the model's edits).
    pub(crate) fn rope_mut(&mut self) -> &mut Rope {
        &mut self.rope
    }
}
