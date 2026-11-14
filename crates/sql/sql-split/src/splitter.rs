//! The block-stack splitter with editor and execution modes (SQL-SPLIT-01, -02; Task 8).
//!
//! One forward loop over the tokens (`alg/10` SPLIT-02, DBeaver's shape) does the client
//! pre-pass and the split together: it recognises client commands (`client`), keeps a block
//! stack per statement (the `sql_dialect::SplitProfile` tables), ends a statement at the
//! delimiter when the stack is empty, and records the editor's heuristic boundaries (`recover`)
//! and every uncertain place. That structural pass (`Analysis`) is the same for both modes;
//! `Analysis::to_split` turns it into the editor or the execution view.
//!
//! **Tokens.** The loop reads the document's own lexing until a client command interrupts it;
//! from there it lexes again in chunks that double in size (`Stream`), so a script with many
//! commands or `COPY` blocks costs O(n), not O(n × commands). A chunk ends at a line start where
//! one is near, and a token that spans that boundary (a string, comment, dollar quote or `COPY`
//! data across lines) is resumed from the lexer's end state and joined, which is exact at line
//! starts (`sql_lexer::LexState`); on a long line it ends inside the line, after the last token
//! the cut cannot have changed.

use std::borrow::Cow;
use std::sync::Arc;

use sql_dialect::split_profile::{BlockFamily, BlockKind, ClientProfile, client_profile};
use sql_dialect::{
    ClientFlavor, DialectCtx, DialectId, KwEntry, LexDialect, LexModes, SplitProfile, lex_profile,
    split_profile,
};
use sql_lexer::{LexErrorCode, LexState, Lexed, TokenFlags, TokenKind, lex};
use sql_text::{TextRange, TextSize};

use crate::client::{self, Backslash};
use crate::item::{
    AmbiguityReason, AmbiguousBoundary, ClientKind, ClientSegment, Confidence, CopyData, DiagCode,
    Diagnostic, Item, Split, SplitMode, Stmt, StmtHead,
};
use crate::key::StmtKey;
use crate::recover::{self, Cut, PendingWith, Trivia};

/// Where a split starts: the lexer's state and the client delimiter in force there (a
/// selection that starts inside a dollar quote, or after `DELIMITER //`).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct SplitStart {
    pub lex: LexState,
    /// The client delimiter (`None`: `;`).
    pub delimiter: Option<Arc<str>>,
}

/// Splits `text` (lexed from `LexState::Initial` as `lexed`) in `mode`.
pub fn split(
    text: &str,
    lexed: &Lexed,
    ctx: &DialectCtx,
    flavor: ClientFlavor,
    mode: SplitMode,
) -> Split {
    analyze(text, Some(lexed), ctx, flavor, &SplitStart::default()).to_split(text, mode)
}

/// Splits `text` from `start` (it lexes `text` itself), in `mode`.
pub fn split_from(
    text: &str,
    ctx: &DialectCtx,
    flavor: ClientFlavor,
    mode: SplitMode,
    start: &SplitStart,
) -> Split {
    analyze(text, None, ctx, flavor, start).to_split(text, mode)
}

/// The lexer profile for `ctx`; a dialect without one lexes with the generic profile.
pub(crate) fn lexer_for(ctx: &DialectCtx) -> &'static LexDialect {
    match lex_profile(ctx) {
        Ok(p) => p,
        Err(_) => match lex_profile(&DialectCtx::new(DialectId::Generic, None)) {
            Ok(p) => p,
            Err(_) => unreachable!("the generic dialect always has a lexer profile"),
        },
    }
}

pub(crate) fn size(n: usize) -> TextSize {
    TextSize::from(u32::try_from(n).expect("a SQL text is under 4 GiB"))
}

pub(crate) fn range(a: usize, b: usize) -> TextRange {
    TextRange::new(size(a), size(b))
}

// ------------------------------------------------------------------------------------------
// Tokens
// ------------------------------------------------------------------------------------------

/// One token with its absolute range.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Tok {
    pub kind: TokenKind,
    pub range: TextRange,
    pub flags: TokenFlags,
    pub keyword: Option<KwEntry>,
    /// The token's end depends on how the server reads it (`AmbiguousCommentEnd`).
    pub ambiguous_end: bool,
}

impl Tok {
    pub(crate) fn start(&self) -> usize {
        usize::from(self.range.start())
    }

    pub(crate) fn end(&self) -> usize {
        usize::from(self.range.end())
    }

    /// Trivia: whitespace, line breaks and comments (an executable comment is not, and neither
    /// is a comment whose end is ambiguous: it belongs to a statement).
    pub(crate) fn is_trivia(&self) -> bool {
        matches!(
            self.kind,
            TokenKind::Whitespace
                | TokenKind::Newline
                | TokenKind::LineComment
                | TokenKind::BlockComment
                | TokenKind::HintComment
        ) && !self.ambiguous_end
    }

    pub(crate) fn is_comment(&self) -> bool {
        matches!(
            self.kind,
            TokenKind::LineComment | TokenKind::BlockComment | TokenKind::HintComment
        )
    }

    /// A token in which a client delimiter may start (not a string, identifier, comment or
    /// space).
    fn is_code(&self) -> bool {
        !matches!(
            self.kind,
            TokenKind::String
                | TokenKind::QuotedIdent
                | TokenKind::UnicodeQuotedIdent
                | TokenKind::BitString
                | TokenKind::HexString
                | TokenKind::DollarString
                | TokenKind::LineComment
                | TokenKind::BlockComment
                | TokenKind::HintComment
                | TokenKind::ExecutableComment
                | TokenKind::Whitespace
                | TokenKind::Newline
                | TokenKind::InlineData
        )
    }
}

/// The starts of a lexing's tokens whose end is ambiguous.
fn ambiguous_starts(lexed: &Lexed) -> Vec<TextSize> {
    lexed
        .errors()
        .iter()
        .filter(|e| e.code == LexErrorCode::AmbiguousCommentEnd)
        .map(|e| e.range.start())
        .collect()
}

/// The tokens of `text[base..]`, lexed lazily in chunks.
struct Stream<'a> {
    text: &'a str,
    profile: &'static LexDialect,
    modes: LexModes,
    run: Cow<'a, Lexed>,
    /// How many of `run`'s tokens the stream hands out: all of them, or, after a cut inside a
    /// line, those that end `CUT_MARGIN` before the cut.
    usable: usize,
    /// The lexer's state at `end`.
    state: LexState,
    /// Where `run`'s text starts, and where its usable tokens end.
    base: usize,
    end: usize,
    next: usize,
    amb: Vec<TextSize>,
    grow: usize,
    /// The first chunk's length after a restart.
    first: usize,
    /// `run` was lexed from `Initial`, so its first token is an `Initial`-state token too.
    initial: bool,
    /// Bytes lexed so far (the complexity counter; the document's own lexing is not counted).
    lexed: u64,
}

/// After a restart the first chunk is short (a command or delimiter usually comes back within a
/// few lines); each further chunk doubles.
const FIRST_CHUNK: usize = 64;
const MAX_CHUNK: usize = 1 << 20;
const BOM: &[u8] = b"\xEF\xBB\xBF";

impl<'a> Stream<'a> {
    fn new(
        text: &'a str,
        lexed: Option<&'a Lexed>,
        profile: &'static LexDialect,
        modes: LexModes,
        start: &LexState,
        first: usize,
    ) -> Stream<'a> {
        let first = first.max(1);
        let mut s = Stream {
            text,
            profile,
            modes,
            run: Cow::Owned(lex("", profile, modes, &LexState::Initial)),
            usable: 0,
            state: LexState::Initial,
            base: 0,
            end: 0,
            next: 0,
            amb: Vec::new(),
            grow: first,
            first,
            initial: true,
            lexed: 0,
        };
        match lexed {
            Some(l) if usize::from(l.range_end()) == text.len() => {
                s.amb = ambiguous_starts(l);
                s.usable = l.len();
                s.state = l.end_state().clone();
                s.run = Cow::Borrowed(l);
                s.end = text.len();
            }
            _ => s.load(0, start.clone()),
        }
        s
    }

    /// Lexes the chunk starting at `at` in `state`.
    ///
    /// The chunk ends at a line start when one comes within twice the chunk's length: the
    /// lexer's state is exact there, and a token the cut splits is resumed and joined (`next`).
    /// On a longer line (a one-line script) it is cut inside the line instead. A token the
    /// lexer settles by looking a few bytes ahead reads as in the whole text once it ends
    /// `CUT_MARGIN` before the cut, so those are kept, and the next chunk starts after them, in
    /// `Initial`. One scan can settle a token early by meeting the cut: a dollar tag read to
    /// the cut leaves a lone `$`, and the tag's bytes end the chunk as one word that reaches
    /// the cut. So when a word ends the chunk, the token before it waits for a longer chunk too.
    /// (`COPY` data, which the lexer reads line by line, is never cut so.) Either way a chunk
    /// costs its length, and the chunks double, so a run of commands on one line still costs
    /// O(n).
    fn load(&mut self, at: usize, state: LexState) {
        let n = self.text.len();
        loop {
            let want = at.saturating_add(self.grow).min(n);
            let line = cut_after(self.text, want);
            let mid = line > at.saturating_add(self.grow.saturating_mul(2))
                && state != LexState::InCopyData;
            let end = if mid {
                ceil_char(self.text, want)
            } else {
                line
            };
            let lexed = lex(&self.text[at..end], self.profile, self.modes, &state);
            self.lexed += (end - at) as u64;
            let (usable, stop, next) = if mid {
                let cut = end - at;
                let b = self.text.as_bytes();
                let mut k = lexed.len();
                while k > 0 {
                    let e = usize::from(lexed.range(k - 1).end());
                    if e + CUT_MARGIN <= cut
                        && !lexed.flags(k - 1).contains(TokenFlags::UNTERMINATED)
                        && !b[at + e..].starts_with(BOM)
                    {
                        break;
                    }
                    k -= 1;
                }
                let last = lexed.len().saturating_sub(1);
                if k > 0
                    && k == last
                    && lexed.kind(last) == TokenKind::Word
                    && usize::from(lexed.range(last).end()) == cut
                {
                    k -= 1;
                    while k > 0 && b[at + usize::from(lexed.range(k - 1).end())..].starts_with(BOM)
                    {
                        k -= 1;
                    }
                }
                if k == 0 {
                    // The first token runs past the cut: lex a longer chunk.
                    self.grow = self.grow.saturating_mul(2);
                    continue;
                }
                let e = usize::from(lexed.range(k - 1).end());
                (k, at + e, LexState::Initial)
            } else {
                (lexed.len(), end, lexed.end_state().clone())
            };
            self.amb = ambiguous_starts(&lexed);
            self.run = Cow::Owned(lexed);
            self.usable = usable;
            self.state = next;
            self.initial = state == LexState::Initial;
            self.base = at;
            self.end = stop;
            self.next = 0;
            self.grow = self.grow.saturating_mul(2).min(MAX_CHUNK.max(self.grow));
            return;
        }
    }

    /// Continues at `at` in `state`, dropping whatever was lexed past it.
    ///
    /// When the current run has an `Initial`-state token starting exactly at `at`, its tokens
    /// from there are what a fresh lex from `at` gives (the lexer reads no byte before a token's
    /// start), so they are kept; otherwise lexing restarts at `at`.
    fn restart(&mut self, at: usize, state: LexState) {
        let n = self.text.len();
        if state == LexState::Initial && at >= self.base && (at > self.base || self.initial) {
            if at < self.end {
                let rel = size(at - self.base);
                if let Some(i) = self.run.token_at(rel)
                    && i < self.usable
                    && self.run.range(i).start() == rel
                {
                    self.next = i;
                    return;
                }
            } else if at == self.end && self.state == LexState::Initial {
                self.next = self.usable;
                return;
            }
        }
        self.grow = self.first;
        if at >= n {
            self.base = n;
            self.end = n;
            self.next = 0;
            self.run = Cow::Owned(lex("", self.profile, self.modes, &LexState::Initial));
            self.usable = 0;
            self.state = LexState::Initial;
            self.amb.clear();
            return;
        }
        self.load(at, state);
    }

    fn tok(&self, i: usize) -> Tok {
        let r = self.run.range(i);
        let shift = size(self.base);
        Tok {
            kind: self.run.kind(i),
            range: TextRange::new(r.start() + shift, r.end() + shift),
            flags: self.run.flags(i),
            keyword: self.run.keyword(i),
            ambiguous_end: self.amb.binary_search(&r.start()).is_ok(),
        }
    }

    fn next(&mut self) -> Option<Tok> {
        let n = self.text.len();
        loop {
            if self.next < self.usable {
                let mut i = self.next;
                self.next += 1;
                let mut tok = self.tok(i);
                // A token a line cut split: resume it from the lexer's state and join.
                while i + 1 == self.usable
                    && self.end < n
                    && tok.flags.contains(TokenFlags::UNTERMINATED)
                {
                    let state = self.state.clone();
                    self.load(self.end, state);
                    if self.usable == 0 {
                        break;
                    }
                    let cont = self.tok(0);
                    self.next = 1;
                    i = 0;
                    tok = Tok {
                        range: TextRange::new(tok.range.start(), cont.range.end()),
                        flags: cont.flags,
                        ambiguous_end: tok.ambiguous_end || cont.ambiguous_end,
                        ..tok
                    };
                }
                return Some(tok);
            }
            if self.end >= n {
                return None;
            }
            let state = self.state.clone();
            self.load(self.end, state);
        }
    }
}

/// The first char boundary at or after `at`.
fn ceil_char(text: &str, mut at: usize) -> usize {
    while at < text.len() && !text.is_char_boundary(at) {
        at += 1;
    }
    at.min(text.len())
}

/// The first line start at or after `at` that does not begin with a byte-order mark (which a
/// chunk would read as whitespace), else the text's end.
fn cut_after(text: &str, at: usize) -> usize {
    let n = text.len();
    if at >= n {
        return n;
    }
    let mut cut = client::next_line_start(text, at.saturating_sub(1));
    while cut < n && text.as_bytes()[cut..].starts_with(BOM) {
        cut = client::next_line_start(text, cut);
    }
    cut
}

/// The end of a lexing's text (the end of its last token).
trait RangeEnd {
    fn range_end(&self) -> TextSize;
}

impl RangeEnd for Lexed {
    fn range_end(&self) -> TextSize {
        if self.is_empty() {
            TextSize::from(0)
        } else {
            self.range(self.len() - 1).end()
        }
    }
}

// ------------------------------------------------------------------------------------------
// The structural pass
// ------------------------------------------------------------------------------------------

/// A statement as the delimiter rules end it, before either mode's view.
#[derive(Debug, Clone)]
pub(crate) struct RawStmt {
    pub strict: TextRange,
    /// The end of the smart range: the terminator or a same-line trailing comment.
    pub smart_end: TextSize,
    pub terminator: Option<TextRange>,
    pub leading: Option<TextRange>,
    pub head: StmtHead,
    pub delimiter: Option<Arc<str>>,
    pub copy_data: Option<CopyData>,
    /// Heuristic boundaries inside (editor mode splits here).
    pub cuts: Vec<Cut>,
    /// A structural ambiguity overlaps it (editor mode marks it `Heuristic`).
    pub uncertain: bool,
    /// The key of its strict text (computed once, shared by both views).
    pub key: StmtKey,
}

#[derive(Debug, Clone)]
pub(crate) enum RawItem {
    Stmt(RawStmt),
    Empty(TextRange),
}

/// The structural pass over one text: the same for both modes.
#[derive(Debug, Clone, Default)]
pub(crate) struct Analysis {
    pub items: Vec<RawItem>,
    pub client: Vec<ClientSegment>,
    pub ambiguous: Vec<AmbiguousBoundary>,
    pub diagnostics: Vec<Diagnostic>,
    /// Where the earliest construct still open at the text's end begins (the editor's
    /// stability rule re-splits after its line).
    pub unterminated: Option<TextSize>,
    /// Where each client delimiter takes effect.
    pub delimiters: Vec<(TextSize, Option<Arc<str>>)>,
    /// Where lexing restarted after a command, a delimiter or data, and in which state.
    pub restarts: Vec<(TextSize, LexState)>,
    pub ctx: Option<DialectCtx>,
    pub flavor: ClientFlavor,
    /// The delimiter in force at the text's start.
    pub start_delimiter: Option<Arc<str>>,
    /// Bytes the pass lexed itself (chunks, re-lexed token prefixes, executable-comment bodies,
    /// server-side comment reads): the complexity counter.
    pub lexed: u64,
}

/// Runs the structural pass over `text` (its lexing from `start.lex` is `lexed`, or the pass
/// lexes it).
pub(crate) fn analyze(
    text: &str,
    lexed: Option<&Lexed>,
    ctx: &DialectCtx,
    flavor: ClientFlavor,
    start: &SplitStart,
) -> Analysis {
    analyze_chunked(text, lexed, ctx, flavor, start, FIRST_CHUNK)
}

/// [`analyze`], lexing in chunks that start at `first` bytes after a restart.
pub(crate) fn analyze_chunked(
    text: &str,
    lexed: Option<&Lexed>,
    ctx: &DialectCtx,
    flavor: ClientFlavor,
    start: &SplitStart,
    first: usize,
) -> Analysis {
    let lp = lexer_for(ctx);
    let lexed = lexed.filter(|_| start.lex == LexState::Initial);
    let mut w = Walker {
        text,
        ctx,
        sp: split_profile(ctx),
        cp: client_profile(flavor),
        lp,
        stream: Stream::new(text, lexed, lp, ctx.modes, &start.lex, first),
        delim: start.delimiter.clone(),
        out: Analysis {
            ctx: Some(*ctx),
            flavor,
            start_delimiter: start.delimiter.clone(),
            ..Analysis::default()
        },
        cur: None,
        trivia: Trivia::default(),
        after_item: false,
        open_trailing: None,
        copy: None,
        last_block_item: None,
        if_depth: 0,
        server_to_eof: false,
        server_read: None,
        lexed: 0,
    };
    w.run();
    w.out.lexed = w.lexed + w.stream.lexed;
    w.out
}

/// One open block and the token that opened it.
#[derive(Debug, Clone, Copy)]
struct Open {
    kind: BlockKind,
    at: TextRange,
    /// A `CASE` in statement position (a CASE statement, closed by `END CASE`).
    statement: bool,
    /// A `REPEAT` whose `UNTIL` has been read (its `END` follows the condition).
    until: bool,
}

/// Whether a statement follows its family's block rules.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// Reading the head: `scanned` tokens so far; `after_eq` right after `DEFINER =` (or the
    /// `@` of `user@host`), whose value is skipped whatever it is.
    Undecided {
        scanned: u8,
        after_eq: bool,
    },
    Plain,
    Routine,
}

/// The statement being read.
#[derive(Debug)]
struct Cur {
    start: TextSize,
    last_end: TextSize,
    leading: Option<TextRange>,
    head: StmtHead,
    /// The first token of the current (heuristic) sub-statement: head continuations read it.
    sub_head: TextRange,
    sub_once_used: bool,
    mode: Mode,
    stack: Vec<Open>,
    after_end: Option<(Open, TextRange)>,
    pending_begin: Option<TextRange>,
    /// A MariaDB `FOR` in statement position: a loop, unless `EACH` follows (a trigger's
    /// `FOR EACH ROW`).
    pending_for: Option<TextRange>,
    pending_with: Option<PendingWith>,
    cuts: Vec<Cut>,
    uncertain: bool,
    /// psql `COPY … FROM STDIN`: 1 = head is COPY, 2 = FROM seen, 3 = FROM STDIN.
    copy: u8,
    sig: usize,
    prev: Option<Tok>,
    /// Client commands between the statement's tokens.
    inside: Vec<TextRange>,
    /// Inside a block: the current inner statement is a `DECLARE`, and it is a handler
    /// declaration (`DECLARE … HANDLER FOR …`), whose action may be a `BEGIN … END`.
    inner_declare: bool,
    handler: bool,
    /// A block other than a parenthesis was opened in this statement.
    had_block: bool,
    /// The statement stands in (or across) a psql `\if` branch.
    in_branch: bool,
}

struct Walker<'a, 's> {
    text: &'a str,
    ctx: &'s DialectCtx,
    sp: &'static SplitProfile,
    cp: &'static ClientProfile,
    lp: &'static LexDialect,
    stream: Stream<'a>,
    delim: Option<Arc<str>>,
    out: Analysis,
    cur: Option<Cur>,
    /// Trivia since the last significant token, item or command.
    trivia: Trivia,
    /// The trivia run started right after an item ended (its same-line comments may trail it).
    after_item: bool,
    /// The last statement, which may still take same-line trailing comments.
    open_trailing: Option<usize>,
    /// psql `COPY … FROM STDIN` data due at this offset, for this item.
    copy: Option<(usize, usize)>,
    /// The last item whose statement tracked a block (the MySQL stray-`END` backstop).
    last_block_item: Option<usize>,
    /// psql `\if` nesting: statements in a branch run only if psql takes it.
    if_depth: u32,
    /// A server-side comment read already ran to the text's end (every later one would too).
    server_to_eof: bool,
    /// The last comment read server-side: its start and where the server ends it.
    server_read: Option<(usize, Option<usize>)>,
    /// Bytes lexed outside the stream.
    lexed: u64,
}

impl<'a> Walker<'a, '_> {
    fn eq(&self, t: &Tok, word: &str) -> bool {
        t.kind == TokenKind::Word && self.text[t.range].eq_ignore_ascii_case(word)
    }

    fn word(&self, t: &Tok) -> Option<&'a str> {
        (t.kind == TokenKind::Word).then(|| &self.text[t.range])
    }

    fn in_table(&self, t: &Tok, table: &[&str]) -> bool {
        self.word(t)
            .is_some_and(|w| table.iter().any(|x| w.eq_ignore_ascii_case(x)))
    }

    /// A token's text as the tables spell punctuation (`,` `(` `.` `;` `:` `)`), or its word.
    fn spelled(&self, t: &Tok) -> &'a str {
        match t.kind {
            TokenKind::Word
            | TokenKind::Comma
            | TokenKind::LParen
            | TokenKind::RParen
            | TokenKind::Dot
            | TokenKind::Semicolon
            | TokenKind::Op => &self.text[t.range],
            _ => "",
        }
    }

    fn run(&mut self) {
        while let Some(mut t) = self.stream.next() {
            self.reread_comment_end(&mut t);
            if let Some((ds, _)) = self.copy
                && t.end() > ds
            {
                if t.start() < ds {
                    self.sub_lex(t.start(), ds);
                }
                self.copy_data();
                continue;
            }
            if let Some(d) = self.delim.clone()
                && self.client_delimiter(&t, &d)
            {
                continue;
            }
            if !t.is_trivia() && self.client_command(&t) {
                continue;
            }
            self.token(t);
        }
        if self.copy.is_some() {
            self.copy_data();
        }
        self.finish();
    }

    /// The lexer's flag and the splitter's own server-side reading of a comment's end are
    /// joined: a terminated executable comment or hint is ambiguous when either says so. They
    /// differ where the lexer's reading is coarser than its own tokens (it reads `x'…'` and
    /// `b'…'` as escaping strings, where the lexer, like the server, reads raw literals); taking
    /// both only ever refuses more.
    fn reread_comment_end(&mut self, t: &mut Tok) {
        if t.ambiguous_end
            || !matches!(
                t.kind,
                TokenKind::ExecutableComment | TokenKind::HintComment
            )
            || t.flags.contains(TokenFlags::UNTERMINATED)
        {
            return;
        }
        if self.server_end_of(t) != Some(t.end()) {
            t.ambiguous_end = true;
        }
    }

    /// [`server_end`] of the comment `t`, read once per token.
    fn server_end_of(&mut self, t: &Tok) -> Option<usize> {
        if let Some((at, end)) = self.server_read
            && at == t.start()
        {
            return end;
        }
        let (end, lexed) = server_end(self.text, t, self.lp, self.ctx.modes);
        self.lexed += lexed;
        self.server_read = Some((t.start(), end));
        end
    }

    /// Feeds the tokens of `text[a..b]` lexed on their own (a token cut by a delimiter or by
    /// `COPY` data).
    fn sub_lex(&mut self, a: usize, b: usize) {
        self.lexed += (b - a) as u64;
        let lexed = lex(
            &self.text[a..b],
            self.lp,
            self.ctx.modes,
            &LexState::Initial,
        );
        let amb = ambiguous_starts(&lexed);
        let shift = size(a);
        for i in 0..lexed.len() {
            let r = lexed.range(i);
            let mut t = Tok {
                kind: lexed.kind(i),
                range: TextRange::new(r.start() + shift, r.end() + shift),
                flags: lexed.flags(i),
                keyword: lexed.keyword(i),
                ambiguous_end: amb.binary_search(&r.start()).is_ok(),
            };
            self.reread_comment_end(&mut t);
            self.token(t);
        }
    }

    fn restart(&mut self, at: usize, state: LexState) {
        self.out.restarts.push((size(at), state.clone()));
        self.stream.restart(at, state);
    }

    // ---- client delimiter (`DELIMITER $$`) -------------------------------------------------

    /// Ends the statement at the client delimiter `d` when it starts inside `t`.
    fn client_delimiter(&mut self, t: &Tok, d: &str) -> bool {
        if t.kind == TokenKind::ExecutableComment {
            // The client delimiter inside an executable comment: a server running the comment
            // would not see it, a client would cut there.
            if let Some(p) = find(self.text, t.start(), t.end(), d) {
                self.ambiguity(
                    range(p, p + d.len()),
                    AmbiguityReason::SemicolonInExecutableComment,
                );
            }
            return false;
        }
        if !t.is_code() {
            return false;
        }
        // A match starts inside the token and may run past it (`$$;` over `1$$` and `;`).
        let Some(p) = find(self.text, t.start(), t.end(), d) else {
            return false;
        };
        if p > t.start() {
            self.sub_lex(t.start(), p);
        }
        let term = range(p, p + d.len());
        if self.cur.is_some() {
            self.terminate(Some(term));
        } else {
            self.empty_item(term);
        }
        self.restart(p + d.len(), LexState::Initial);
        true
    }

    // ---- client commands --------------------------------------------------------------------

    /// Handles a client command starting at `t`; `false` when `t` is SQL.
    fn client_command(&mut self, t: &Tok) -> bool {
        let cp = self.cp;
        let at = t.start();
        if cp.backslash_commands && t.kind == TokenKind::Unknown && &self.text[t.range] == "\\" {
            match client::backslash(self.text, at, cp) {
                Backslash::InlineSemicolon { end } => {
                    self.segment(at, end, ClientKind::InlineSemicolon, false);
                }
                Backslash::Terminator { end, supported } => {
                    self.segment(at, end, ClientKind::Terminator, supported);
                }
                Backslash::Clear { end } => self.segment(at, end, ClientKind::ClearBuffer, false),
                Backslash::Conditional { end, nesting } => {
                    if let Some(cur) = &mut self.cur {
                        cur.in_branch = true;
                    }
                    self.if_depth = self.if_depth.saturating_add_signed(nesting.into());
                    self.segment(at, end, ClientKind::Command, false);
                }
                Backslash::Delimiter { arg, end }
                    if self.cur.is_none() && client::only_blanks_before(self.text, at) =>
                {
                    self.delimiter_command(at, arg, end);
                }
                Backslash::Delimiter { end, .. } | Backslash::Command { end } => {
                    self.segment(at, end, ClientKind::Command, false);
                }
            }
            return true;
        }
        let idle = self.cur.is_none();
        if idle
            && let Some(cmd) = cp.delimiter_command
            && self.eq(t, cmd)
            && client::only_blanks_before(self.text, at)
        {
            let arg = client::delimiter_arg(self.text, t.end());
            let end = client::line_end(self.text, at);
            self.delimiter_command(at, arg, end);
            return true;
        }
        if idle && self.in_table(t, cp.line_commands) && client::only_blanks_before(self.text, at) {
            let end = client::line_end(self.text, at);
            self.segment(at, end, ClientKind::Command, false);
            return true;
        }
        if idle
            && cp.dot_commands
            && self.text.as_bytes()[at] == b'.'
            && client::at_left_margin(self.text, at)
        {
            let end = client::line_end(self.text, at);
            self.segment(at, end, ClientKind::Command, false);
            return true;
        }
        false
    }

    /// Records the client command `[at, end)` and continues lexing after it.
    fn segment(&mut self, at: usize, end: usize, kind: ClientKind, supported: bool) {
        let seg_range = range(at, end);
        let seg = ClientSegment {
            range: seg_range,
            kind,
            supported,
        };
        if self.cur.is_some() {
            match kind {
                ClientKind::Terminator | ClientKind::ClearBuffer => {
                    let start = self.cur.as_ref().map_or(size(at), |c| c.start);
                    self.terminate(Some(seg_range));
                    if !supported {
                        self.ambiguity(
                            TextRange::new(start, seg_range.end()),
                            AmbiguityReason::ClientCommand,
                        );
                        self.mark_last_uncertain();
                    }
                }
                _ => {
                    // The statement goes on around the command, which Hacivat cannot send as
                    // written: uncertain once SQL follows it inside the statement (`terminate`).
                    if let Some(cur) = &mut self.cur {
                        cur.inside.push(seg_range);
                    }
                }
            }
        } else {
            self.flush_trailing(false);
            self.trivia = Trivia::default();
            self.after_item = false;
            let seg = ClientSegment {
                supported: supported && kind != ClientKind::Terminator,
                ..seg
            };
            self.out.client.push(seg);
            self.restart(end, LexState::Initial);
            return;
        }
        self.out.client.push(seg);
        self.restart(end, LexState::Initial);
    }

    /// `DELIMITER x` (or `\d x`) at statement start: the line `[at, end)`, the argument `arg`.
    fn delimiter_command(&mut self, at: usize, arg: (usize, usize), end: usize) {
        let arg_text = &self.text[arg.0..arg.1];
        let arg_range = range(arg.0, arg.1);
        let ok = if arg_text.is_empty() {
            self.diag(range(at, end), DiagCode::DelimiterWithoutArgument);
            false
        } else if arg_text.contains('\\') {
            self.diag(arg_range, DiagCode::DelimiterInvalid);
            false
        } else {
            if arg_text.len() > 1 && arg_text.ends_with(';') {
                self.diag(arg_range, DiagCode::DelimiterEndsWithSemicolon);
            }
            self.delim = (arg_text != ";").then(|| Arc::from(arg_text));
            self.out.delimiters.push((size(end), self.delim.clone()));
            true
        };
        self.segment(at, end, ClientKind::Delimiter, ok);
    }

    // ---- COPY … FROM STDIN data -------------------------------------------------------------

    fn copy_data(&mut self) {
        let Some((ds, item)) = self.copy.take() else {
            return;
        };
        if let Some(cur) = &self.cur {
            // Text after the COPY's terminator, on its line, that the data cuts short.
            let open = TextRange::new(cur.start, cur.last_end);
            self.ambiguity(open, AmbiguityReason::UnterminatedConstruct);
            self.terminate(None);
            self.mark_last_uncertain();
        }
        self.flush_trailing(false);
        self.trivia = Trivia::default();
        self.after_item = false;
        let n = self.text.len();
        self.restart(ds, LexState::InCopyData);
        let data = self
            .stream
            .next()
            .filter(|d| d.kind == TokenKind::InlineData && d.start() == ds);
        let (rows, end, data_end) = match data {
            Some(d) if !d.flags.contains(TokenFlags::UNTERMINATED) && d.end() >= ds + 2 => (
                range(ds, d.end() - 2),
                Some(range(d.end() - 2, d.end())),
                d.end(),
            ),
            Some(d) => (d.range, None, d.end()),
            None => (range(ds.min(n), ds.min(n)), None, ds.min(n)),
        };
        self.out.restarts.push((size(data_end), LexState::Initial));
        if let Some(RawItem::Stmt(s)) = self.out.items.get_mut(item) {
            s.copy_data = Some(CopyData { rows, end });
            if end.is_none() {
                s.uncertain = true;
                let at = TextRange::new(s.strict.start(), size(data_end));
                self.out.ambiguous.push(AmbiguousBoundary {
                    at,
                    reason: AmbiguityReason::UnterminatedConstruct,
                });
                self.diag(rows, DiagCode::UnterminatedCopyData);
            }
        }
    }

    // ---- tokens -----------------------------------------------------------------------------

    fn token(&mut self, t: Tok) {
        if t.is_trivia() {
            if t.flags.contains(TokenFlags::UNTERMINATED) {
                self.unterminated_at(t.range.start());
            }
            self.trivia.push(self.text, &t);
            return;
        }
        if self.cur.is_none() {
            let is_delim = t.kind == TokenKind::Semicolon && self.delim.is_none();
            if is_delim {
                self.empty_item(t.range);
                return;
            }
            self.start_statement(&t);
        } else {
            self.heuristics(&t);
        }
        self.blocks(&t);
        self.checks(&t);
        let ends = t.kind == TokenKind::Semicolon
            && self.delim.is_none()
            && self.cur.as_ref().is_some_and(|c| c.stack.is_empty());
        if ends {
            self.terminate(Some(t.range));
            return;
        }
        if t.kind == TokenKind::Semicolon
            && self.delim.is_some()
            && self.cur.as_ref().is_some_and(|c| c.stack.is_empty())
        {
            self.ambiguity(t.range, AmbiguityReason::DelimiterChange);
        }
        if let Some(cur) = &mut self.cur {
            cur.prev = Some(t);
            cur.last_end = t.range.end();
            cur.sig += 1;
        }
        self.trivia = Trivia::default();
        self.after_item = false;
    }

    fn start_statement(&mut self, t: &Tok) {
        let same_line = self.trivia.breaks == 0;
        let (trailing, leading) = recover::attach(&self.trivia, self.after_item, same_line);
        self.extend_trailing(trailing);
        self.open_trailing = None;
        self.cur = Some(Cur {
            start: t.range.start(),
            last_end: t.range.end(),
            leading,
            head: StmtHead {
                first_keyword: t.keyword.map(|k| k.id),
                first_token: Some(t.range),
            },
            sub_head: t.range,
            sub_once_used: false,
            mode: Mode::Undecided {
                scanned: 0,
                after_eq: false,
            },
            stack: Vec::new(),
            after_end: None,
            pending_begin: None,
            pending_for: None,
            pending_with: None,
            cuts: Vec::new(),
            uncertain: false,
            copy: 0,
            sig: 0,
            prev: None,
            inside: Vec::new(),
            inner_declare: false,
            handler: false,
            had_block: false,
            in_branch: self.if_depth > 0,
        });
        self.stray_end(t);
        // A statement that starts with a client line command's word away from a line start
        // (`SELECT 1; source f.sql`, sqlite3 `x; .mode csv`): the client may read it as the
        // command; Hacivat cannot tell (fail closed).
        let cp = self.cp;
        let command_word = cp.delimiter_command.is_some_and(|c| self.eq(t, c))
            || self.in_table(t, cp.line_commands)
            || (cp.dot_commands && self.text.as_bytes()[t.start()] == b'.');
        if command_word {
            self.ambiguity(t.range, AmbiguityReason::ClientCommand);
        }
    }

    /// The MySQL/MariaDB backstop: no statement starts with `END` there, so one that does means
    /// a block's opener went unseen and its `END` closed an outer block early. Every statement
    /// back to the last block-tracked one, and this one, is uncertain.
    fn stray_end(&mut self, t: &Tok) {
        if self.sp.family != BlockFamily::MySql || !self.eq(t, "END") {
            return;
        }
        let from = self.last_block_item.unwrap_or(self.out.items.len());
        let mut marked = Vec::new();
        for item in &mut self.out.items[from..] {
            if let RawItem::Stmt(s) = item {
                s.uncertain = true;
                marked.push(s.strict);
            }
        }
        for at in marked {
            self.out.ambiguous.push(AmbiguousBoundary {
                at,
                reason: AmbiguityReason::UnknownBlock,
            });
        }
        self.anomaly(t.range);
    }

    fn empty_item(&mut self, r: TextRange) {
        self.flush_trailing(true);
        self.out.items.push(RawItem::Empty(r));
        self.open_trailing = None;
        self.trivia = Trivia::default();
        self.after_item = true;
    }

    /// Gives the last statement the same-line comments of the trivia run (when the run follows
    /// it and the next thing is on a later line).
    fn flush_trailing(&mut self, next_on_same_line: bool) {
        let same = next_on_same_line && self.trivia.breaks == 0;
        let (trailing, _) = recover::attach(&self.trivia, self.after_item, same);
        self.extend_trailing(trailing);
        self.open_trailing = None;
    }

    fn extend_trailing(&mut self, trailing: Option<TextSize>) {
        if let (Some(end), Some(i)) = (trailing, self.open_trailing)
            && let Some(RawItem::Stmt(s)) = self.out.items.get_mut(i)
        {
            s.smart_end = s.smart_end.max(end);
        }
    }

    /// The editor heuristics before `t` (SQL-SPLIT-03): a pending `WITH`, then `t` itself.
    fn heuristics(&mut self, t: &Tok) {
        let text = self.text;
        let sp = self.sp;
        let cp = self.cp;
        let first_on_line = self.trivia.breaks > 0;
        let Some(cur) = self.cur.as_mut() else {
            return;
        };
        if let Some(p) = cur.pending_with.take() {
            let names_a_cte = (t.kind == TokenKind::Word
                && (t.keyword.is_none() || text[t.range].eq_ignore_ascii_case("RECURSIVE")))
                || t.kind == TokenKind::QuotedIdent;
            if names_a_cte {
                let with = p.cut.at;
                cur.cuts.push(p.cut);
                cur.sub_head = with;
                cur.sub_once_used = false;
                // A WITH statement is never a routine.
                cur.mode = Mode::Plain;
            }
        }
        if !(cur.sig > 0 && cur.stack.is_empty()) {
            return;
        }
        let word = (t.kind == TokenKind::Word).then(|| &text[t.range]);
        // A `DELIMITER` line inside an open statement (the mysql client would take it).
        let delimiter_line = first_on_line
            && cp
                .delimiter_command
                .zip(word)
                .is_some_and(|(c, w)| c.eq_ignore_ascii_case(w))
            && client::only_blanks_before(text, t.start());
        if !delimiter_line {
            let starts_word = word.is_some_and(|w| {
                sp.statement_starts
                    .iter()
                    .any(|x| x.eq_ignore_ascii_case(w))
            });
            if !starts_word {
                return;
            }
            // 1. The head's continuation rule, at the first start word wherever it stands
            //    (`INSERT INTO t SELECT …` uses up INSERT's one continuation on the same line).
            let head = &text[cur.sub_head];
            let w = word.unwrap_or("");
            let rule = sp.head_continuations.iter().find(|h| {
                h.head.eq_ignore_ascii_case(head)
                    && (h.words.is_empty() || h.words.iter().any(|x| x.eq_ignore_ascii_case(w)))
            });
            if let Some(rule) = rule {
                if !rule.once {
                    return;
                }
                if !cur.sub_once_used {
                    cur.sub_once_used = true;
                    return;
                }
            }
            // 2. Only the first significant token on its line.
            if !first_on_line {
                return;
            }
            // 3. The token before it.
            let prev = cur.prev.map_or("", |p| match p.kind {
                TokenKind::Word | TokenKind::Comma | TokenKind::Dot => &text[p.range],
                _ => "",
            });
            if sp
                .prev_exceptions
                .iter()
                .any(|x| x.eq_ignore_ascii_case(prev))
            {
                return;
            }
        }
        let cut = recover::cut_at(&self.trivia, cur.last_end, t);
        if word.is_some_and(|w| w.eq_ignore_ascii_case("WITH")) && !delimiter_line {
            cur.pending_with = Some(PendingWith { cut });
            return;
        }
        cur.cuts.push(cut);
        cur.sub_head = t.range;
        cur.sub_once_used = false;
        cur.mode = Mode::Undecided {
            scanned: 0,
            after_eq: false,
        };
    }

    // ---- blocks -----------------------------------------------------------------------------

    fn blocks(&mut self, t: &Tok) {
        let family = self.sp.family;
        // An `END`'s follower names the block it closed.
        let mut consumed = false;
        if let Some((popped, end_tok)) = self.cur.as_mut().and_then(|c| c.after_end.take()) {
            let follower = self
                .sp
                .end_followers
                .iter()
                .find(|(w, _)| self.eq(t, w))
                .map(|(_, k)| *k);
            match follower {
                Some(kind) => {
                    consumed = true;
                    if kind != popped.kind {
                        self.anomaly(t.range);
                    }
                }
                None => {
                    if needs_follower(&popped) {
                        self.anomaly(end_tok);
                    }
                }
            }
        }
        if let Some(at) = self.cur.as_mut().and_then(|c| c.pending_for.take())
            && !self.eq(t, "EACH")
        {
            self.push(BlockKind::For, at, true);
        }
        if let Some(begin) = self.cur.as_mut().and_then(|c| c.pending_begin.take()) {
            let opens = match family {
                BlockFamily::Postgres => self.eq(t, "ATOMIC").then_some(BlockKind::BeginAtomic),
                BlockFamily::MySql => self
                    .in_table(t, self.sp.top_level_block_begins)
                    .then_some(BlockKind::BeginEnd),
                _ => None,
            };
            if let Some(kind) = opens {
                self.push(kind, begin, false);
            }
        }
        self.head_scan(t);
        self.copy_scan(t);
        if t.kind == TokenKind::Semicolon
            && let Some(cur) = &mut self.cur
        {
            cur.inner_declare = false;
            cur.handler = false;
        }
        match t.kind {
            TokenKind::LParen | TokenKind::LBracket => self.push(BlockKind::Paren, t.range, false),
            TokenKind::RParen | TokenKind::RBracket => self.close_paren(t),
            TokenKind::Word if !consumed => self.block_word(t, family),
            _ => {}
        }
    }

    /// Whether `prev` puts a name next: `BEGIN` and `END` after it are names, not block words
    /// (`t.end`, `AS end`, `SET end = 1`, `SELECT begin`, `x > end`). Any operator does, but not
    /// a label's `:` (`lbl: BEGIN`).
    fn names_next(&self, prev: Option<Tok>) -> bool {
        prev.is_some_and(|p| {
            let w = self.spelled(&p);
            (p.kind == TokenKind::Op && w != ":")
                || self.sp.name_after.iter().any(|x| x.eq_ignore_ascii_case(w))
        })
    }

    fn block_word(&mut self, t: &Tok, family: BlockFamily) {
        let Some(cur) = &self.cur else {
            return;
        };
        let routine = cur.mode == Mode::Routine;
        let top = cur.stack.last().copied();
        let top_is_paren = top.is_some_and(|o| o.kind == BlockKind::Paren);
        let in_block = cur.stack.iter().any(|o| o.kind != BlockKind::Paren);
        let prev = cur.prev;
        let handler = cur.handler;
        // The start of a top-level statement, or right after its label (`lbl: LOOP`).
        let top_start =
            cur.sig == 0 || (cur.sig == 2 && prev.is_some_and(|p| &self.text[p.range] == ":"));
        let names = self.names_next(prev);
        let case_name = prev.is_some_and(|p| {
            p.kind == TokenKind::Dot || self.text[p.range].eq_ignore_ascii_case("AS")
        });
        let word = self.word(t).unwrap_or("");
        let is = |w: &str| word.eq_ignore_ascii_case(w);
        match family {
            BlockFamily::Plain => {}
            BlockFamily::Postgres => {
                if is("BEGIN") {
                    if routine && !in_block && !top_is_paren && !names {
                        self.set_pending_begin(t.range);
                    }
                } else if is("CASE") {
                    if in_block && !case_name {
                        self.push(BlockKind::Case, t.range, false);
                    }
                } else if is("END") {
                    self.end(t, names);
                }
            }
            BlockFamily::Sqlite => {
                if is("BEGIN") {
                    // The trigger body: the first `BEGIN` that is no name (no nesting in SQLite).
                    if routine && !in_block && !top_is_paren && !names {
                        self.push(BlockKind::BeginEnd, t.range, false);
                    }
                } else if is("CASE") {
                    if routine && !case_name {
                        self.push(BlockKind::Case, t.range, false);
                    }
                } else if is("END") && routine {
                    self.end(t, names);
                }
            }
            BlockFamily::MySql => {
                let body = routine || in_block;
                if is("BEGIN") {
                    if names || top_is_paren {
                        // A name (`SELECT begin FROM t`).
                    } else if in_block {
                        if self.statement_position() || handler {
                            self.push(BlockKind::BeginEnd, t.range, false);
                        }
                    } else if routine {
                        // The routine's body.
                        self.push(BlockKind::BeginEnd, t.range, false);
                    } else if top_start {
                        self.set_pending_begin(t.range);
                    }
                } else if is("END") {
                    if body {
                        self.end(t, names);
                    }
                } else if is("UNTIL") {
                    if let Some(cur) = &mut self.cur
                        && let Some(open) = cur.stack.last_mut()
                        && open.kind == BlockKind::Repeat
                    {
                        open.until = true;
                    }
                } else {
                    let position = if body {
                        self.statement_position()
                    } else {
                        self.sp.compound_at_top && top_start
                    };
                    if let Some((declare, marker)) = self.sp.handler
                        && in_block
                    {
                        if position && is(declare) {
                            self.set_inner(|c| c.inner_declare = true);
                        } else if is(marker) && self.cur.as_ref().is_some_and(|c| c.inner_declare) {
                            self.set_inner(|c| c.handler = true);
                        }
                    }
                    let opener = self
                        .sp
                        .statement_openers
                        .iter()
                        .find(|(w, _)| is(w))
                        .map(|(_, k)| *k);
                    match opener {
                        Some(BlockKind::Case) if body && !case_name => {
                            self.push(BlockKind::Case, t.range, position);
                        }
                        Some(BlockKind::For) if position && !case_name => {
                            self.set_inner(|c| c.pending_for = Some(t.range));
                        }
                        Some(kind) if position && !case_name => self.push(kind, t.range, true),
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }

    fn set_inner(&mut self, f: impl FnOnce(&mut Cur)) {
        if let Some(cur) = &mut self.cur {
            f(cur);
        }
    }

    /// Whether the next token stands where a stored-program statement may start (MySQL).
    fn statement_position(&self) -> bool {
        let Some(cur) = &self.cur else {
            return false;
        };
        match cur.stack.last() {
            Some(o) if o.kind == BlockKind::Paren => return false,
            Some(o) if o.kind == BlockKind::Case && !o.statement => return false,
            _ => {}
        }
        let Some(prev) = cur.prev else {
            return false;
        };
        let spelled = self.spelled(&prev);
        if self
            .sp
            .statement_position_after
            .iter()
            .any(|w| w.eq_ignore_ascii_case(spelled))
        {
            return true;
        }
        let header =
            cur.mode == Mode::Routine && !cur.stack.iter().any(|o| o.kind != BlockKind::Paren);
        header
            && (prev.kind == TokenKind::String
                || self
                    .sp
                    .body_start_after
                    .iter()
                    .any(|w| w.eq_ignore_ascii_case(spelled)))
    }

    fn set_pending_begin(&mut self, at: TextRange) {
        if let Some(cur) = &mut self.cur {
            cur.pending_begin = Some(at);
        }
    }

    fn push(&mut self, kind: BlockKind, at: TextRange, statement: bool) {
        if let Some(cur) = &mut self.cur {
            if kind != BlockKind::Paren {
                cur.had_block = true;
                cur.handler = false;
            }
            cur.stack.push(Open {
                kind,
                at,
                statement,
                until: false,
            });
        }
    }

    /// `END` (erratum (e)): after a name position (`names`) it is a name. Otherwise it closes
    /// the innermost block when the grammar can end it here: a `CASE` expression anywhere, a
    /// `REPEAT` after its `UNTIL`, any other block only in statement position
    /// (`block_end_after`); its follower is checked next. An `END` that can close nothing here
    /// is a mismatch (`UnknownBlock`), never an early end. With no block open it is a name.
    fn end(&mut self, t: &Tok, names: bool) {
        if names {
            return;
        }
        let Some(cur) = &self.cur else {
            return;
        };
        let Some(open) = cur.stack.last().copied() else {
            return;
        };
        if open.kind == BlockKind::Paren {
            return;
        }
        let statement_end = cur.prev.is_some_and(|p| {
            let w = self.spelled(&p);
            self.sp
                .block_end_after
                .iter()
                .any(|x| x.eq_ignore_ascii_case(w))
        });
        // A `CASE` expression's `END` follows an expression, never its `THEN` or `ELSE`: there
        // `end` is a name (`CASE a WHEN 1 THEN end END`).
        let after_branch = cur.prev.is_some_and(|p| {
            let w = self.spelled(&p);
            w.eq_ignore_ascii_case("THEN") || w.eq_ignore_ascii_case("ELSE")
        });
        let closes = match open.kind {
            BlockKind::Case if !open.statement => {
                if after_branch {
                    return;
                }
                true
            }
            BlockKind::Repeat => open.until || statement_end,
            _ => statement_end,
        };
        if closes {
            if let Some(cur) = &mut self.cur {
                cur.stack.pop();
                cur.after_end = Some((open, t.range));
            }
        } else {
            self.diag(t.range, DiagCode::EndWithoutBlock);
            self.anomaly(t.range);
        }
    }

    fn close_paren(&mut self, t: &Tok) {
        let Some(cur) = &mut self.cur else {
            return;
        };
        match cur.stack.last() {
            Some(o) if o.kind == BlockKind::Paren => {
                cur.stack.pop();
            }
            _ => {
                if let Some(i) = cur.stack.iter().rposition(|o| o.kind == BlockKind::Paren) {
                    cur.stack.truncate(i);
                    self.anomaly(t.range);
                } else {
                    let in_block = cur.stack.iter().any(|o| o.kind != BlockKind::Paren);
                    self.diag(t.range, DiagCode::UnbalancedClose);
                    if in_block {
                        self.anomaly(t.range);
                    }
                }
            }
        }
    }

    /// Reads a statement's head to switch on the family's block rules for a routine.
    fn head_scan(&mut self, t: &Tok) {
        let sp = self.sp;
        let text = self.text;
        let Some(cur) = &mut self.cur else {
            return;
        };
        let Mode::Undecided { scanned, after_eq } = cur.mode else {
            return;
        };
        let word = (t.kind == TokenKind::Word).then(|| &text[t.range]);
        let is_in =
            |table: &[&str]| word.is_some_and(|w| table.iter().any(|x| x.eq_ignore_ascii_case(w)));
        cur.mode = if scanned == 0 {
            if is_in(&["CREATE", "ALTER"]) && !sp.routine_kinds.is_empty() {
                Mode::Undecided {
                    scanned: 1,
                    after_eq: false,
                }
            } else {
                Mode::Plain
            }
        } else if scanned > 16 {
            Mode::Plain
        } else if after_eq {
            // The value of `DEFINER = …`: a user, quoted or not, keyword or not.
            Mode::Undecided {
                scanned: scanned + 1,
                after_eq: false,
            }
        } else if is_in(sp.routine_kinds) {
            Mode::Routine
        } else if is_in(sp.routine_head_skips)
            || matches!(
                t.kind,
                TokenKind::String
                    | TokenKind::QuotedIdent
                    | TokenKind::UserVar
                    | TokenKind::LParen
                    | TokenKind::RParen
            )
        {
            Mode::Undecided {
                scanned: scanned + 1,
                after_eq: false,
            }
        } else if t.kind == TokenKind::Op && matches!(&text[t.range], "=" | "@") {
            // `DEFINER = user`, and the host of `user@host`.
            Mode::Undecided {
                scanned: scanned + 1,
                after_eq: true,
            }
        } else {
            Mode::Plain
        };
    }

    /// psql `COPY … FROM STDIN` at depth 0.
    fn copy_scan(&mut self, t: &Tok) {
        if !self.cp.copy_from_stdin || t.kind != TokenKind::Word {
            return;
        }
        let word = &self.text[t.range];
        let Some(cur) = &mut self.cur else {
            return;
        };
        let depth0 = cur.stack.is_empty();
        cur.copy = match cur.copy {
            0 if cur.sig == 0 && word.eq_ignore_ascii_case("COPY") => 1,
            1 if depth0 && word.eq_ignore_ascii_case("FROM") => 2,
            2 if word.eq_ignore_ascii_case("STDIN") => 3,
            2 => 1,
            c => c,
        };
    }

    /// Per-token uncertainty: executable-comment bodies, ambiguous ends, unterminated tokens.
    fn checks(&mut self, t: &Tok) {
        if t.ambiguous_end {
            // Every statement up to where the server ends the comment is uncertain.
            let end = if self.server_to_eof {
                self.text.len()
            } else {
                let end = refusal_end(self.text.len(), t, self.server_end_of(t));
                self.server_to_eof = end == self.text.len();
                end
            };
            self.ambiguity(
                range(t.start(), end.max(t.end())),
                AmbiguityReason::AmbiguousCommentEnd,
            );
        }
        if t.kind == TokenKind::ExecutableComment {
            self.executable_body(t);
        }
        if t.flags.contains(TokenFlags::UNTERMINATED) && t.kind != TokenKind::InlineData {
            self.ambiguity(t.range, AmbiguityReason::UnterminatedConstruct);
            self.unterminated_at(t.range.start());
        }
    }

    /// A `;` inside an executable comment's body, or a body that does not lex to its end.
    ///
    /// Any `;` byte counts, a quoted one too (`alg/10` SQL-LEX-05 step 3 and its FX-SPLIT-MY-04
    /// case: `/*!80000 SELECT 'versioned;' */` is `Ambiguous` in execution mode): the body's
    /// own quoting is exactly what a server reads differently when the comment's end is in
    /// doubt, so the splitter does not rely on it.
    fn executable_body(&mut self, t: &Tok) {
        let tok = &self.text[t.range];
        let open = if tok.starts_with("/*M!") { 4 } else { 3 };
        let digits = tok.as_bytes()[open.min(tok.len())..]
            .iter()
            .take(6)
            .take_while(|b| b.is_ascii_digit())
            .count();
        let skip = open + if digits >= 5 { digits } else { 0 };
        let closed = !t.flags.contains(TokenFlags::UNTERMINATED)
            && tok.len() >= skip + 2
            && tok.ends_with("*/");
        let body_end = if closed { tok.len() - 2 } else { tok.len() };
        let body_start = skip.min(body_end);
        let a = t.start() + body_start;
        let body_text = &tok[body_start..body_end];
        self.lexed += body_text.len() as u64;
        let body = lex(body_text, self.lp, self.ctx.modes, &LexState::Initial);
        if let Some(i) = body_text.bytes().position(|b| b == b';') {
            let at = range(a + i, a + i + 1);
            self.ambiguity(at, AmbiguityReason::SemicolonInExecutableComment);
            self.diag(at, DiagCode::SemicolonInExecutableComment);
        } else if *body.end_state() != LexState::Initial && !t.ambiguous_end {
            // A body that does not lex to its end: where the server ends the comment is
            // uncertain (the lexer flags this itself; this keeps the splitter closed on its own).
            self.ambiguity(t.range, AmbiguityReason::AmbiguousCommentEnd);
        }
    }

    fn unterminated_at(&mut self, at: TextSize) {
        let u = &mut self.out.unterminated;
        *u = Some(u.map_or(at, |old| old.min(at)));
    }

    fn ambiguity(&mut self, at: TextRange, reason: AmbiguityReason) {
        self.out.ambiguous.push(AmbiguousBoundary { at, reason });
        if let Some(cur) = &mut self.cur {
            cur.uncertain = true;
        }
    }

    /// A block-stack mismatch.
    fn anomaly(&mut self, at: TextRange) {
        self.ambiguity(at, AmbiguityReason::UnknownBlock);
    }

    fn diag(&mut self, r: TextRange, code: DiagCode) {
        self.out.diagnostics.push(Diagnostic { range: r, code });
    }

    fn mark_last_uncertain(&mut self) {
        if let Some(RawItem::Stmt(s)) = self.out.items.last_mut() {
            s.uncertain = true;
        }
    }

    /// Ends the statement being read (`terminator`: what ended it).
    fn terminate(&mut self, terminator: Option<TextRange>) {
        let Some(mut cur) = self.cur.take() else {
            return;
        };
        if let Some((popped, end_tok)) = cur.after_end.take()
            && needs_follower(&popped)
        {
            self.out.ambiguous.push(AmbiguousBoundary {
                at: end_tok,
                reason: AmbiguityReason::UnknownBlock,
            });
            cur.uncertain = true;
        }
        if let Some(open) = cur.stack.first() {
            self.out.ambiguous.push(AmbiguousBoundary {
                at: open.at,
                reason: AmbiguityReason::UnterminatedConstruct,
            });
            self.out.diagnostics.push(Diagnostic {
                range: open.at,
                code: DiagCode::UnterminatedBlock,
            });
            cur.uncertain = true;
        }
        let strict = TextRange::new(cur.start, cur.last_end);
        if cur.in_branch || self.if_depth > 0 {
            // psql runs it only if it takes the branch.
            self.out.ambiguous.push(AmbiguousBoundary {
                at: strict,
                reason: AmbiguityReason::ClientCommand,
            });
            cur.uncertain = true;
        }
        for seg in cur.inside.iter().filter(|s| s.start() < strict.end()) {
            self.out.ambiguous.push(AmbiguousBoundary {
                at: *seg,
                reason: AmbiguityReason::ClientCommand,
            });
            cur.uncertain = true;
        }
        let smart_end = terminator.map_or(strict.end(), |t| t.end().max(strict.end()));
        let copy = cur.copy == 3 && terminator.is_some();
        self.out.items.push(RawItem::Stmt(RawStmt {
            strict,
            smart_end,
            terminator,
            leading: cur.leading,
            head: cur.head,
            delimiter: self.delim.clone(),
            copy_data: None,
            cuts: cur.cuts,
            uncertain: cur.uncertain,
            key: StmtKey::of(self.ctx, self.delim.as_deref(), &self.text[strict]),
        }));
        let idx = self.out.items.len() - 1;
        if cur.had_block {
            self.last_block_item = Some(idx);
        }
        self.open_trailing = Some(idx);
        self.trivia = Trivia::default();
        self.after_item = true;
        if copy && let Some(t) = terminator {
            let ds = client::next_line_start(self.text, usize::from(t.end()));
            self.copy = Some((ds, idx));
        }
    }

    fn finish(&mut self) {
        if let Some(open) = self.cur.as_ref().and_then(|c| c.stack.first()) {
            // A block still open at the text's end: the editor's stability rule applies.
            let at = open.at.start();
            self.unterminated_at(at);
        }
        if self.cur.is_some() {
            // Same-line comments after the last statement trail it.
            let (trailing, _) = recover::attach(&self.trivia, true, false);
            self.terminate(None);
            self.extend_trailing(trailing);
        } else {
            self.flush_trailing(false);
        }
        self.clip_ambiguities();
        self.out.diagnostics.sort_by_key(|d| d.range.start());
    }

    /// Lists each uncertain boundary once per statement it overlaps, cut to that statement's
    /// strict range, sorted and without repeats: every boundary then lies inside the one
    /// execution statement it makes uncertain (`Split::is_certain` relies on it). One that
    /// overlaps no statement refuses nothing and is dropped.
    fn clip_ambiguities(&mut self) {
        let strict: Vec<TextRange> = self
            .out
            .items
            .iter()
            .filter_map(|it| match it {
                RawItem::Stmt(s) => Some(s.strict),
                RawItem::Empty(_) => None,
            })
            .collect();
        let mut clipped = Vec::with_capacity(self.out.ambiguous.len());
        for a in &self.out.ambiguous {
            let mut i = strict.partition_point(|r| r.end() <= a.at.start());
            while i < strict.len()
                && strict[i].start() < a.at.end().max(a.at.start() + TextSize::from(1))
            {
                if crate::item::overlaps(a.at, strict[i])
                    && let Some(at) = a.at.intersect(strict[i]).filter(|r| !r.is_empty())
                {
                    clipped.push(AmbiguousBoundary { at, ..*a });
                }
                i += 1;
            }
        }
        clipped.sort_by_key(|a| (a.at.start(), a.at.end(), a.reason as u8));
        clipped.dedup();
        self.out.ambiguous = clipped;
    }
}

/// How far past a token the lexer may look before it settles that token: `-- ` and `/*M!`, an
/// `x'`/`b'` prefix, a version number. A token that ends this far before a prefix's cut reads as
/// it does in the whole text.
const CUT_MARGIN: usize = 16;

/// Where a MySQL/MariaDB server that runs (or reads) the comment token `t` ends it, and the
/// bytes lexed to find out: the comment's body is lexed as code with the session's profile and
/// modes (quotes, identifiers, `#` and `-- ` comments, one nested `/* … */` all come from the
/// lexer, the reading `sql_lexer` flags `AmbiguousCommentEnd` against), and the first `*/` in
/// code ends it. `None` when nothing does, or when the body holds an inner executable comment,
/// which the lexer does not follow: no end is certain (the caller fails closed to the text's
/// end).
///
/// The body is lexed in doubling prefixes cut anywhere (not at line starts, which a one-line
/// script would not have), each read only up to `CUT_MARGIN` before its cut: the cost is a
/// small multiple of the distance to that end, never the rest of the text.
pub(crate) fn server_end(
    text: &str,
    t: &Tok,
    profile: &'static LexDialect,
    modes: LexModes,
) -> (Option<usize>, u64) {
    server_end_from(text, t, profile, modes, None)
}

/// [`server_end`], its first prefix `first` bytes long (`None`: just past the token's end).
fn server_end_from(
    text: &str,
    t: &Tok,
    profile: &'static LexDialect,
    modes: LexModes,
    first: Option<usize>,
) -> (Option<usize>, u64) {
    let open = if text[t.range].starts_with("/*M!") {
        4
    } else {
        3
    };
    let from = (t.start() + open).min(text.len());
    let body = &text[from..];
    let b = body.as_bytes();
    let mut lexed = 0u64;
    // The server usually ends the comment where the lexer does: start just past that.
    let mut want = first
        .unwrap_or((t.end().saturating_sub(from) + 2 * CUT_MARGIN).max(FIRST_CHUNK / 2))
        .max(1);
    loop {
        let mut cut = want.min(body.len());
        while !body.is_char_boundary(cut) {
            cut += 1;
        }
        let whole = cut == body.len();
        let run = lex(&body[..cut], profile, modes, &LexState::Initial);
        lexed += cut as u64;
        let settled = |at: usize| whole || at + CUT_MARGIN <= cut;
        for i in 0..run.len() {
            let r = run.range(i);
            let (s, e) = (usize::from(r.start()), usize::from(r.end()));
            match run.kind(i) {
                TokenKind::ExecutableComment if settled(s) => return (None, lexed),
                TokenKind::Op if settled(e) && b[s] == b'*' && b.get(s + 1) == Some(&b'/') => {
                    return (Some(from + s + 2), lexed);
                }
                _ if !settled(e) => break,
                _ => {}
            }
        }
        if whole {
            return (None, lexed);
        }
        want = want.saturating_mul(2);
    }
}

/// How far the refusal of an ambiguous comment `t` reaches: to where the server ends it, or to
/// the text's end when no end is certain, or when that reading ends exactly at the token's own
/// end although the lexer flagged it (the two readings disagree: fail closed).
fn refusal_end(n: usize, t: &Tok, server: Option<usize>) -> usize {
    match server {
        Some(e) if e != t.end() => e.max(t.end()),
        _ => n,
    }
}

/// [`refusal_end`] of a flagged comment token at `at` (the agreement property's probe).
#[doc(hidden)]
pub fn comment_refusal_end_at(text: &str, ctx: &DialectCtx, at: usize, len: usize) -> usize {
    let t = Tok {
        kind: TokenKind::ExecutableComment,
        range: range(at, at + len),
        flags: TokenFlags::empty(),
        keyword: None,
        ambiguous_end: true,
    };
    refusal_end(
        text.len(),
        &t,
        server_end(text, &t, lexer_for(ctx), ctx.modes).0,
    )
}

/// [`server_end`] of the comment token starting at `at`, its first prefix `first` bytes long
/// when given (the agreement property's probe).
#[doc(hidden)]
pub fn comment_server_end(
    text: &str,
    ctx: &DialectCtx,
    at: usize,
    len: usize,
    first: Option<usize>,
) -> Option<usize> {
    let t = Tok {
        kind: TokenKind::ExecutableComment,
        range: range(at, at + len),
        flags: TokenFlags::empty(),
        keyword: None,
        ambiguous_end: true,
    };
    server_end_from(text, &t, lexer_for(ctx), ctx.modes, first).0
}

/// [`split_from`] lexing in chunks of `first` bytes after each restart (the chunking
/// property's probe: any chunking gives the same split).
#[doc(hidden)]
pub fn split_chunked(
    text: &str,
    ctx: &DialectCtx,
    flavor: ClientFlavor,
    mode: SplitMode,
    start: &SplitStart,
    first: usize,
) -> Split {
    analyze_chunked(text, None, ctx, flavor, start, first).to_split(text, mode)
}

/// The bytes the structural pass lexes for `text` on its own (the complexity counter).
#[doc(hidden)]
pub fn lexed_bytes(text: &str, ctx: &DialectCtx, flavor: ClientFlavor) -> u64 {
    analyze(text, None, ctx, flavor, &SplitStart::default()).lexed
}

/// Whether a block needs `END <kind>` rather than a bare `END`.
fn needs_follower(o: &Open) -> bool {
    matches!(
        o.kind,
        BlockKind::If | BlockKind::Loop | BlockKind::While | BlockKind::Repeat | BlockKind::For
    ) || (o.kind == BlockKind::Case && o.statement)
}

/// The first `needle` in `text` starting in `[from, to)` (it may run past `to`).
fn find(text: &str, from: usize, to: usize, needle: &str) -> Option<usize> {
    let b = text.as_bytes();
    let n = needle.as_bytes();
    let to = to.min(b.len());
    if from >= to || n.is_empty() {
        return None;
    }
    let window = &b[from..(to + n.len() - 1).min(b.len())];
    window
        .windows(n.len())
        .position(|w| w == n)
        .map(|i| from + i)
        .filter(|p| *p < to)
}

// ------------------------------------------------------------------------------------------
// The two views
// ------------------------------------------------------------------------------------------

impl Analysis {
    /// The split in `mode`.
    pub(crate) fn to_split(&self, text: &str, mode: SplitMode) -> Split {
        match mode {
            SplitMode::Execution => self.execution(text),
            SplitMode::Editor => recover::editor(self, text, 0),
        }
    }

    fn ctx(&self) -> DialectCtx {
        self.ctx
            .unwrap_or_else(|| DialectCtx::new(DialectId::Generic, None))
    }

    /// One statement of `raw` with `strict`, `leading`, `smart_end`, `terminator`.
    pub(crate) fn stmt(
        &self,
        text: &str,
        raw: &RawStmt,
        part: Part,
        confidence: Confidence,
    ) -> Stmt {
        let strict = part.strict;
        let smart_start = part.leading.map_or(strict.start(), |l| l.start());
        let smart_end = part.smart_end.max(strict.end());
        Stmt {
            strict,
            smart: TextRange::new(smart_start, smart_end),
            terminator: part.terminator,
            leading_comments: part.leading,
            confidence,
            head: part.head,
            key: if strict == raw.strict {
                raw.key
            } else {
                StmtKey::of(&self.ctx(), raw.delimiter.as_deref(), &text[strict])
            },
            copy_data: part.copy_data,
        }
    }

    /// The execution view: one `Exact` item per delimiter-ended statement; every heuristic
    /// boundary is an `AmbiguousBoundary`.
    fn execution(&self, text: &str) -> Split {
        let mut split = self.base_split();
        let overlapped = self.overlapped_items(&split);
        for (n, item) in self.items.iter().enumerate() {
            split.items.push(match item {
                RawItem::Empty(r) => Item::Empty { range: *r },
                RawItem::Stmt(raw) => {
                    // `Exact` only when certain: a refused statement never reads as runnable.
                    let confidence = if raw.uncertain || overlapped[n] {
                        Confidence::Heuristic
                    } else {
                        Confidence::Exact
                    };
                    Item::Statement(self.stmt(text, raw, Part::whole(raw), confidence))
                }
            });
        }
        split
    }

    /// Which items an ambiguity of `split` overlaps (a sweep: O((A + marks) log N)).
    pub(crate) fn overlapped_items(&self, split: &Split) -> Vec<bool> {
        let ranges: Vec<TextRange> = self
            .items
            .iter()
            .map(|it| match it {
                RawItem::Stmt(s) => s.strict,
                RawItem::Empty(r) => *r,
            })
            .collect();
        let mut marked = vec![false; ranges.len()];
        for amb in &split.ambiguous {
            let mut i = ranges.partition_point(|r| r.end() < amb.at.start());
            while i < ranges.len() && ranges[i].start() <= amb.at.end() {
                if crate::item::overlaps(amb.at, ranges[i]) {
                    marked[i] = true;
                }
                i += 1;
            }
        }
        marked
    }

    /// The ambiguities, client segments and diagnostics both views share: the structural ones
    /// and one `MissingSemicolon` per heuristic boundary.
    pub(crate) fn base_split(&self) -> Split {
        let mut ambiguous = self.ambiguous.clone();
        let mut diagnostics = self.diagnostics.clone();
        for item in &self.items {
            if let RawItem::Stmt(raw) = item {
                for cut in &raw.cuts {
                    ambiguous.push(AmbiguousBoundary {
                        at: cut.at,
                        reason: AmbiguityReason::MissingSemicolon,
                    });
                    diagnostics.push(Diagnostic {
                        range: cut.at,
                        code: DiagCode::MissingSemicolonSuspected,
                    });
                }
            }
        }
        ambiguous.sort_by_key(|a| (a.at.start(), a.at.end(), a.reason as u8));
        ambiguous.dedup();
        diagnostics.sort_by_key(|d| d.range.start());
        Split {
            items: Vec::new(),
            ambiguous,
            client: self.client.clone(),
            diagnostics,
        }
    }

    /// The delimiter in force at `at`.
    pub(crate) fn delimiter_at(&self, at: TextSize) -> Option<Arc<str>> {
        let i = self.delimiters.partition_point(|(p, _)| *p <= at);
        if i == 0 {
            self.start_delimiter.clone()
        } else {
            self.delimiters[i - 1].1.clone()
        }
    }
}

/// One editor or execution item cut from a raw statement.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Part {
    pub strict: TextRange,
    pub leading: Option<TextRange>,
    pub smart_end: TextSize,
    pub terminator: Option<TextRange>,
    pub head: StmtHead,
    pub copy_data: Option<CopyData>,
}

impl Part {
    pub(crate) fn whole(raw: &RawStmt) -> Part {
        Part {
            strict: raw.strict,
            leading: raw.leading,
            smart_end: raw.smart_end,
            terminator: raw.terminator,
            head: raw.head,
            copy_data: raw.copy_data,
        }
    }
}
