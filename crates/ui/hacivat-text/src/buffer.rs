//! `TextBuffer`: a rope with exact line endings and the BOM in its metadata (UI-EDIT-02).
//!
//! Byte offsets are canonical. CR, LF and CRLF each end a line and round-trip byte for byte;
//! nothing is normalised. U+2028 and U+0085 are not line breaks (ropey without
//! `unicode_lines`; `cargo xtask check-deps` `RopeLineBreaks`). A leading U+FEFF moves into
//! [`BufferMeta::bom`] and comes back with [`TextBuffer::to_string_with_bom`].
//!
//! The text changes only through [`ChangeSet::apply`](crate::ChangeSet::apply) (the model's
//! dispatch), on character boundaries; the line-ending counts follow every edit in time
//! proportional to the edit, never to the document. No casing or normalisation is ever applied
//! to the text.

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
    /// The line endings the text has now.
    pub line_endings: LineEndingStats,
}

/// A line ending.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LineEnding {
    Lf,
    CrLf,
    Cr,
}

impl LineEnding {
    /// The ending's bytes: what a typed newline inserts.
    pub fn as_str(self) -> &'static str {
        match self {
            LineEnding::Lf => "\n",
            LineEnding::CrLf => "\r\n",
            LineEnding::Cr => "\r",
        }
    }
}

/// How many line endings of each kind the text has (a CRLF counts once, as `crlf`).
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

/// Counts line endings over byte slices fed in order (a CRLF may straddle two slices).
#[derive(Default)]
struct EndingCounter {
    stats: LineEndingStats,
    pending_cr: bool,
}

impl EndingCounter {
    fn feed(&mut self, bytes: &[u8]) {
        for &b in bytes {
            if self.pending_cr {
                self.pending_cr = false;
                if b == b'\n' {
                    self.stats.crlf += 1;
                    continue;
                }
                self.stats.cr += 1;
            }
            match b {
                b'\r' => self.pending_cr = true,
                b'\n' => self.stats.lf += 1,
                _ => {}
            }
        }
    }

    fn finish(mut self) -> LineEndingStats {
        if self.pending_cr {
            self.stats.cr += 1;
        }
        self.stats
    }
}

fn stats_of(text: &str) -> LineEndingStats {
    let mut c = EndingCounter::default();
    c.feed(text.as_bytes());
    c.finish()
}

/// The line endings inside the bytes `a..b` of `rope` (any byte offsets).
fn endings_in(rope: &Rope, a: usize, b: usize) -> LineEndingStats {
    let mut c = EndingCounter::default();
    if a < b {
        let (chunks, mut at, _, _) = rope.chunks_at_byte(a);
        for chunk in chunks {
            let bytes = chunk.as_bytes();
            let lo = a.saturating_sub(at);
            let hi = (b - at).min(bytes.len());
            if lo < hi {
                c.feed(&bytes[lo..hi]);
            }
            at += bytes.len();
            if at >= b {
                break;
            }
        }
    }
    c.finish()
}

/// `stats` with `gone` taken out and `came` put in.
fn adjust(stats: LineEndingStats, gone: LineEndingStats, came: LineEndingStats) -> LineEndingStats {
    let f = |s: u32, g: u32, c: u32| s.saturating_add(c).saturating_sub(g);
    LineEndingStats {
        lf: f(stats.lf, gone.lf, came.lf),
        crlf: f(stats.crlf, gone.crlf, came.crlf),
        cr: f(stats.cr, gone.cr, came.cr),
    }
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

    /// The text with its BOM, when it had one: what saving writes back.
    pub fn to_string_with_bom(&self) -> String {
        let mut out = String::with_capacity(self.len_bytes() + 3);
        if self.meta.bom {
            out.push('\u{FEFF}');
        }
        for chunk in self.rope.chunks() {
            out.push_str(chunk);
        }
        out
    }

    /// `b` falls on a character boundary of the text (the end of the text is one).
    pub fn is_char_boundary(&self, b: usize) -> bool {
        if b > self.rope.len_bytes() {
            return false;
        }
        let (chunk, start, _, _) = self.rope.chunk_at_byte(b);
        chunk.is_char_boundary(b - start)
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
        // The break is the line's last one or two bytes: read them, never the line (O(log n)).
        let byte_before = |at: usize| (at > start).then(|| self.rope.byte(at - 1));
        let body_end = match byte_before(end) {
            Some(b'\n') if end - 1 > start && self.rope.byte(end - 2) == b'\r' => end - 2,
            Some(b'\n' | b'\r') => end - 1,
            _ => end,
        };
        start..body_end
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

    /// `b` moved to a char boundary on `bias`'s side. Unlike [`TextBuffer::snap`] it may stop
    /// between the CR and LF of a CRLF: explicit byte edits may split one.
    pub(crate) fn snap_to_char(&self, b: usize, bias: Bias) -> usize {
        let b = b.min(self.rope.len_bytes());
        let char_idx = self.rope.byte_to_char(b);
        let at = self.rope.char_to_byte(char_idx);
        if at != b && bias == Bias::After {
            self.rope.char_to_byte(char_idx + 1)
        } else {
            at
        }
    }

    /// The character that ends just before byte `b` (a char boundary).
    pub(crate) fn char_before(&self, b: usize) -> Option<char> {
        let char_idx = self.rope.byte_to_char(b.min(self.rope.len_bytes()));
        char_idx.checked_sub(1).map(|i| self.rope.char(i))
    }

    /// Replaces the bytes `r` (on char boundaries) with `text`, keeping the line-ending counts
    /// exact in O(log n + |r| + |text|): only the edit and one byte on either side are counted
    /// (a CR before the edit may pair with a LF after it, or stop pairing).
    pub(crate) fn replace(&mut self, r: Range<usize>, text: &str) {
        let len = self.rope.len_bytes();
        let window_start = r.start.saturating_sub(1);
        let gone = endings_in(&self.rope, window_start, (r.end + 1).min(len));
        let start_char = self.rope.byte_to_char(r.start);
        let end_char = self.rope.byte_to_char(r.end);
        if end_char > start_char {
            self.rope.remove(start_char..end_char);
        }
        if !text.is_empty() {
            self.rope.insert(start_char, text);
        }
        let new_len = self.rope.len_bytes();
        let came = endings_in(
            &self.rope,
            window_start,
            (r.start + text.len() + 1).min(new_len),
        );
        let meta = Arc::make_mut(&mut self.meta);
        meta.line_endings = adjust(meta.line_endings, gone, came);
    }
}
