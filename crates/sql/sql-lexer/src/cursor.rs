//! The byte cursor and the lexing entry points (SQL-LEX-02).
//!
//! One pass over the bytes, first matching rule wins (`alg/10` LEX-02 rules 1–12), each rule
//! read from the `LexDialect` profile and the session's `LexModes`. Terminators are found with
//! `memchr`; nothing recurses (nesting is a counter). Every byte the cursor examines is counted
//! in `steps`, so the complexity tests can prove the pass linear.

use sql_dialect::lex_profiles::{At, Colon, Dollar, DoubleQuote, Question};
use sql_dialect::{LexDialect, LexModes, keyword_lookup};
use sql_text::{TextChunks, TextRange, TextSize};

use crate::state::LexState;
use crate::token::{LexErrorCode, Lexed, TokenFlags, TokenKind};
use crate::{comments, dollar, numbers, ops, params, strings, words};

/// Lexes `text` with `profile` and `modes`, starting in `start`. Total and lossless over any
/// text below 4 GiB: the tokens tile the text, every token is at least one byte, every boundary
/// is a char boundary, and malformed input becomes flagged tokens and errors.
///
/// Offset 0 of `text` is taken as the start of a document (a byte-order mark there is
/// whitespace; see `LexState` for resuming inside one).
///
/// # Panics
/// When `text` is 4 GiB or longer (offsets are `u32`); no editor document reaches that
/// (R-M3-49 caps one at 20 MB).
pub fn lex(text: &str, profile: &LexDialect, modes: LexModes, start: &LexState) -> Lexed {
    Cursor::new(text, profile, modes).run(start)
}

/// [`lex`] and the number of bytes the cursor examined (the complexity tests' counter).
#[doc(hidden)]
pub fn lex_counting_steps(
    text: &str,
    profile: &LexDialect,
    modes: LexModes,
    start: &LexState,
) -> (Lexed, u64) {
    let mut cursor = Cursor::new(text, profile, modes);
    let lexed = cursor.run_counted(start);
    (lexed, cursor.steps)
}

/// [`lex`] over chunked text. M3a materialises the chunks into one string (a single chunk is
/// read in place) and lexes it.
///
/// # Panics
/// When the text is 4 GiB or longer, as [`lex`].
pub fn lex_chunks(
    text: &dyn TextChunks,
    profile: &LexDialect,
    modes: LexModes,
    start: &LexState,
) -> Lexed {
    let mut chunks = text.chunks();
    match (chunks.next(), chunks.next()) {
        (None, _) => lex("", profile, modes, start),
        (Some(only), None) => lex(only, profile, modes, start),
        (Some(first), Some(second)) => {
            let mut whole = String::with_capacity(text.len());
            whole.push_str(first);
            whole.push_str(second);
            chunks.for_each(|c| whole.push_str(c));
            lex(&whole, profile, modes, start)
        }
    }
}

/// A byte length as a `TextSize`.
///
/// # Panics
/// Above 4 GiB, which no editor document reaches (R-M3-49 caps it at 20 MB).
pub(crate) fn text_size(len: usize) -> TextSize {
    TextSize::from(u32::try_from(len).expect("a SQL text is under 4 GiB"))
}

/// One token the rules found at a position.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Scan {
    pub(crate) kind: TokenKind,
    /// Where the token ends (exclusive).
    pub(crate) end: usize,
    pub(crate) flags: TokenFlags,
    /// Its errors, in order (at most two: an unterminated construct can also be ambiguous or
    /// rejected).
    pub(crate) errors: [Option<LexErrorCode>; 2],
    /// The state the text ends in when this token runs unterminated to its end.
    pub(crate) open: Option<LexState>,
}

impl Scan {
    pub(crate) fn new(kind: TokenKind, end: usize) -> Scan {
        Scan {
            kind,
            end,
            flags: TokenFlags::empty(),
            errors: [None, None],
            open: None,
        }
    }

    pub(crate) fn flagged(mut self, flags: TokenFlags) -> Scan {
        self.flags |= flags;
        self
    }

    pub(crate) fn with_error(mut self, code: LexErrorCode) -> Scan {
        let slot = self.errors.iter_mut().find(|e| e.is_none());
        *slot.expect("a token has at most two errors") = Some(code);
        self
    }

    /// The server rejects the token, or its reading is uncertain: `ERROR` and `code`.
    pub(crate) fn rejected(self, code: LexErrorCode) -> Scan {
        self.flagged(TokenFlags::ERROR).with_error(code)
    }

    /// An identifier follows directly: `JUNK` and `TrailingJunk`.
    pub(crate) fn junk(self) -> Scan {
        self.flagged(TokenFlags::JUNK)
            .with_error(LexErrorCode::TrailingJunk)
    }

    /// The token runs to the end of the text, unterminated, leaving lexing in `state`.
    pub(crate) fn unterminated(
        kind: TokenKind,
        end: usize,
        state: LexState,
        error: Option<LexErrorCode>,
    ) -> Scan {
        Scan {
            kind,
            end,
            flags: TokenFlags::UNTERMINATED,
            errors: [error, None],
            open: Some(state),
        }
    }
}

/// The cursor over one text.
pub(crate) struct Cursor<'a> {
    pub(crate) text: &'a str,
    pub(crate) b: &'a [u8],
    pub(crate) p: &'a LexDialect,
    pub(crate) modes: LexModes,
    /// Bytes examined so far.
    pub(crate) steps: u64,
    /// The PostgreSQL operator run being split (`ops`), so a long run is read once.
    pub(crate) op_run: ops::RunCache,
}

impl<'a> Cursor<'a> {
    pub(crate) fn new(text: &'a str, p: &'a LexDialect, modes: LexModes) -> Cursor<'a> {
        Cursor {
            text,
            b: text.as_bytes(),
            p,
            modes,
            steps: 0,
            op_run: ops::RunCache::default(),
        }
    }

    pub(crate) fn len(&self) -> usize {
        self.b.len()
    }

    /// The byte at `i`, `None` past the end (one step).
    pub(crate) fn at(&mut self, i: usize) -> Option<u8> {
        self.steps += 1;
        self.b.get(i).copied()
    }

    /// The first index at or after `from` whose byte fails `keep`, else the text's end.
    pub(crate) fn skip_while(&mut self, from: usize, keep: impl Fn(u8) -> bool) -> usize {
        let end = self.b[from.min(self.b.len())..]
            .iter()
            .position(|c| !keep(*c))
            .map_or(self.b.len(), |n| from + n);
        self.steps += (end.saturating_sub(from) + 1) as u64;
        end
    }

    /// The first `needle` at or after `from`.
    pub(crate) fn find(&mut self, from: usize, needle: u8) -> Option<usize> {
        let found = memchr::memchr(needle, self.b.get(from..)?).map(|n| from + n);
        self.count_scan(from, found);
        found
    }

    /// The first `a` or `b` at or after `from`.
    pub(crate) fn find2(&mut self, from: usize, a: u8, b: u8) -> Option<usize> {
        let found = memchr::memchr2(a, b, self.b.get(from..)?).map(|n| from + n);
        self.count_scan(from, found);
        found
    }

    /// The first occurrence of `needle` at or after `from`.
    pub(crate) fn find_seq(&mut self, from: usize, needle: &[u8]) -> Option<usize> {
        let found = memchr::memmem::find(self.b.get(from..)?, needle).map(|n| from + n);
        self.count_scan(from, found);
        found
    }

    fn count_scan(&mut self, from: usize, found: Option<usize>) {
        let to = found.map_or(self.b.len(), |f| f + 1);
        self.steps += (to.saturating_sub(from) + 1) as u64;
    }

    /// Whether `c` starts an unquoted identifier (non-ASCII bytes always do).
    pub(crate) fn ident_start(&self, c: u8) -> bool {
        c.is_ascii_alphabetic()
            || c == b'_'
            || c >= 0x80
            || (c == b'$' && self.p.idents.dollar_start)
    }

    /// Whether `c` continues an unquoted identifier.
    pub(crate) fn ident_continue(&self, c: u8) -> bool {
        c.is_ascii_alphanumeric()
            || c == b'_'
            || c >= 0x80
            || (c == b'$' && self.p.idents.dollar_continue)
    }

    /// Lexes the whole text from `start`.
    fn run(mut self, start: &LexState) -> Lexed {
        self.run_counted(start)
    }

    fn run_counted(&mut self, start: &LexState) -> Lexed {
        let len = self.len();
        // SQL averages about five bytes a token; the cap keeps a huge string or comment from
        // reserving arrays it never fills (the vectors grow past it as needed).
        let mut out = Lexed::with_capacity((len / 6).min(1 << 16) + 16, text_size(len));
        let mut pos = 0;
        if *start != LexState::Initial {
            if len == 0 {
                out.set_end_state(start.clone());
            } else {
                let scan = self.resume(start);
                pos = self.emit(&mut out, 0, scan);
            }
        }
        while pos < len {
            let scan = self.scan(pos);
            pos = self.emit(&mut out, pos, scan);
        }
        out
    }

    /// Appends `scan` (starting at `pos`) to `out`; returns where it ends.
    fn emit(&mut self, out: &mut Lexed, pos: usize, scan: Scan) -> usize {
        debug_assert!(
            scan.end > pos && scan.end <= self.len(),
            "{scan:?} at {pos}"
        );
        debug_assert!(self.text.is_char_boundary(scan.end), "{scan:?} at {pos}");
        let range = TextRange::new(text_size(pos), text_size(scan.end));
        let keyword = if scan.kind == TokenKind::Word {
            keyword_lookup(self.p.keywords, &self.text[pos..scan.end])
        } else {
            None
        };
        out.push(range.start(), scan.kind, scan.flags, keyword);
        for code in scan.errors.into_iter().flatten() {
            out.push_error(range, code);
        }
        if let Some(state) = scan.open {
            out.set_end_state(state);
        }
        scan.end
    }

    /// Finishes the construct `state` is inside of, from offset 0.
    fn resume(&mut self, state: &LexState) -> Scan {
        match state {
            LexState::Initial => self.scan(0),
            LexState::InBlockComment { depth } => comments::block_from(self, 0, *depth),
            LexState::InHintComment { body } => comments::hint_from(self, 0, *body),
            LexState::InExecutableComment { depth, body } => {
                comments::executable_from(self, 0, *depth, *body)
            }
            LexState::InString { rule, quote, kind } => {
                strings::string_from(self, 0, *quote, *rule, *kind)
            }
            LexState::InQuotedIdent { close, kind } => strings::ident_from(self, 0, *close, *kind),
            LexState::InDollar { tag } => dollar::body_from(self, 0, tag),
            LexState::InCopyData => comments::copy_data_from(self, 0),
        }
    }

    /// The token at `pos`, in the initial state.
    fn scan(&mut self, pos: usize) -> Scan {
        let c = self.b[pos];
        if c >= 0x80 {
            if self.bom_at(pos) {
                return Scan::new(TokenKind::Whitespace, pos + 3).flagged(TokenFlags::BOM);
            }
            return words::scan(self, pos);
        }
        if self.p.illegal.contains(&c) {
            return unknown(pos);
        }
        match c {
            b' ' | b'\t' | 0x0c => self.whitespace(pos),
            0x0b if self.p.space.vertical_tab => self.whitespace(pos),
            b'\n' | b'\r' => {
                let end = self.skip_while(pos, |c| c == b'\n' || c == b'\r');
                Scan::new(TokenKind::Newline, end)
            }
            b'-' if self.dash_comment_at(pos) => comments::line(self, pos),
            b'/' if self.b.get(pos + 1) == Some(&b'*') => {
                comments::block(self, pos).unwrap_or_else(|| ops::scan(self, pos))
            }
            b'#' if self.p.comments.hash => comments::line(self, pos),
            b'#' if self.p.sigils.hash_param => params::tcl(self, pos),
            b'\'' => strings::plain(self, pos),
            b'"' => match self.p.quotes.double {
                DoubleQuote::StringUnlessAnsiQuotes if !self.modes.ansi_quotes => {
                    let rule = strings::plain_rule(self);
                    strings::string_at(self, pos, b'"', rule)
                }
                _ => strings::ident_at(self, pos, 1, b'"', TokenKind::QuotedIdent),
            },
            b'`' if self.p.quotes.backtick_idents => {
                strings::ident_at(self, pos, 1, b'`', TokenKind::QuotedIdent)
            }
            b'[' if self.p.quotes.bracket_idents => {
                strings::ident_at(self, pos, 1, b']', TokenKind::QuotedIdent)
            }
            b'$' => match self.p.sigils.dollar {
                Dollar::Positional => dollar::scan(self, pos),
                Dollar::Tcl => params::tcl(self, pos),
                Dollar::Ident => words::scan(self, pos),
                Dollar::Illegal => unknown(pos),
            },
            b'0'..=b'9' => numbers::scan(self, pos),
            b'.' => self.dot(pos),
            b'?' => match self.p.sigils.question {
                Question::Param => Scan::new(TokenKind::QuestionParam, pos + 1),
                Question::NumberedParam => {
                    let end = self.skip_while(pos + 1, |c| c.is_ascii_digit());
                    Scan::new(TokenKind::QuestionParam, end)
                }
                Question::Operator => ops::scan(self, pos),
            },
            b':' => match self.p.sigils.colon {
                Colon::Postgres | Colon::NamedParam | Colon::Operator => params::colon(self, pos),
                Colon::Tcl => params::tcl(self, pos),
            },
            b'@' => match self.p.sigils.at {
                At::Operator => ops::scan(self, pos),
                At::Variables => params::mysql_var(self, pos),
                At::Tcl => params::tcl(self, pos),
                At::Illegal => unknown(pos),
            },
            b'(' => Scan::new(TokenKind::LParen, pos + 1),
            b')' => Scan::new(TokenKind::RParen, pos + 1),
            b'[' => Scan::new(TokenKind::LBracket, pos + 1),
            b']' => Scan::new(TokenKind::RBracket, pos + 1),
            b'{' => Scan::new(TokenKind::LBrace, pos + 1),
            b'}' => Scan::new(TokenKind::RBrace, pos + 1),
            b',' => Scan::new(TokenKind::Comma, pos + 1),
            b';' => Scan::new(TokenKind::Semicolon, pos + 1),
            0 => Scan::new(TokenKind::Unknown, pos + 1).rejected(LexErrorCode::Nul),
            c if c.is_ascii_alphabetic() || c == b'_' => words::scan(self, pos),
            _ => ops::scan(self, pos),
        }
    }

    fn whitespace(&mut self, pos: usize) -> Scan {
        let vt = self.p.space.vertical_tab;
        let end = self.skip_while(pos, |c| {
            c == b' ' || c == b'\t' || c == 0x0c || (vt && c == 0x0b)
        });
        Scan::new(TokenKind::Whitespace, end)
    }

    /// A UTF-8 byte-order mark at `pos` that is whitespace here: at offset 0, or at any token
    /// start in SQLite.
    ///
    /// Offset 0 is the start of the text *handed to `lex`*: the rule depends on the absolute
    /// offset. A caller that lexes a slice of a document from `Initial` (an incremental relex,
    /// a selection, a `COPY` segment) must not start the slice at a BOM that is not the
    /// document's start, or it reads as whitespace where the document's lexing reads a word.
    fn bom_at(&self, pos: usize) -> bool {
        (pos == 0 || self.p.space.bom_anywhere) && self.b[pos..].starts_with(b"\xEF\xBB\xBF")
    }

    /// `--` at `pos` starts a comment: in MySQL and MariaDB only before whitespace, a control
    /// character or the end.
    fn dash_comment_at(&mut self, pos: usize) -> bool {
        if self.at(pos + 1) != Some(b'-') {
            return false;
        }
        !self.p.comments.dash_needs_space
            || self
                .at(pos + 2)
                .is_none_or(|c| c.is_ascii_whitespace() || c.is_ascii_control())
    }

    /// `.`: a number (`.5`), PostgreSQL's `..`, or a dot.
    fn dot(&mut self, pos: usize) -> Scan {
        match self.at(pos + 1) {
            Some(d) if d.is_ascii_digit() => numbers::scan(self, pos),
            Some(b'.') if self.p.numbers.dot_dot => Scan::new(TokenKind::Op, pos + 2),
            _ => Scan::new(TokenKind::Dot, pos + 1),
        }
    }
}

/// One `Unknown` byte at `pos` (only ASCII bytes are ever unknown: non-ASCII characters are
/// identifier characters).
pub(crate) fn unknown(pos: usize) -> Scan {
    Scan::new(TokenKind::Unknown, pos + 1).rejected(LexErrorCode::UnknownCharacter)
}
