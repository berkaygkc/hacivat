//! Bind parameters (SQL-PARAM-01, -02; Task 6): placeholders in one unit, their bind plan and
//! the rewrite to the driver's syntax (PG `:name` → `$k`, MySQL/SQLite `:name` → `?`).
//!
//! The scan reads the unit's own lexing, from the initial state, exactly as the server
//! receives it. It fails closed: a marker inside data or commentary is never a placeholder,
//! and a marker the server may read as a parameter that Hacivat cannot bind makes the whole
//! unit unbindable, so Run refuses it and nothing is sent ([`scan_params`]).

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use smallvec::SmallVec;
use sql_dialect::{DialectCtx, DialectId, KeywordId, LexDialect, kw, lex_profile};
use sql_lexer::{LexState, Lexed, TokenFlags, TokenKind, exec_comment, lex};
use sql_text::{TextRange, TextSize};

use crate::item::StmtHead;
use crate::templates::{self, TemplateVar};

/// The user's parameter settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ParamSettings {
    /// `?` is a parameter on PostgreSQL (off by default, D-08: it is also a jsonb operator).
    pub pg_question_marks: bool,
    /// `:name` is a parameter.
    pub colon_params: bool,
    /// Placeholders inside DDL statements are bindable.
    pub in_ddl: bool,
}

impl Default for ParamSettings {
    fn default() -> Self {
        ParamSettings {
            pg_question_marks: false,
            colon_params: true,
            in_ddl: false,
        }
    }
}

/// How a placeholder is written. Names compare exactly (`:Id` ≠ `:id`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum PlaceholderKind {
    /// `$n` (and SQLite's `?NNN`)
    Positional(u32),
    /// `?`, numbered from 1 in text order
    Question(u32),
    /// `:name` (and psql `:'name'`; SQLite's `@name`, `$name`, `#name` keep their sigil)
    Named(Arc<str>),
}

/// One placeholder in a unit.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Placeholder {
    pub range: TextRange,
    pub kind: PlaceholderKind,
    pub bindable: bool,
}

/// Name `i` binds the driver slots `slots[i]` (MySQL repeats a name once per occurrence).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct BindPlan {
    pub names: Vec<Arc<str>>,
    pub slots: Vec<SmallVec<[u32; 2]>>,
}

/// The unit as the driver receives it, with the plan and every replacement as (range in the
/// unit, range in the rewritten text).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RewrittenSql {
    pub text: String,
    pub plan: BindPlan,
    pub edits: Vec<(TextRange, TextRange)>,
}

/// A parameter diagnostic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ParamDiag {
    TemplateVariable,
    QuestionMarkOff,
    ParamInDdl,
    MixedStyles,
    TooManyBinds,
    // ## m3b-t06-params
    /// A marker the server may read as a parameter that Hacivat cannot bind.
    Unbindable,
}

/// What [`scan_params`] found in one unit.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct ParamScan {
    pub placeholders: Vec<Placeholder>,
    pub templates: Vec<TemplateVar>,
    /// `Some` exactly when there is at least one placeholder and every one binds.
    pub rewritten: Option<RewrittenSql>,
    /// In text order.
    pub diags: Vec<(TextRange, ParamDiag)>,
}

// ## m3b-t06-params

/// The placeholders, templates and rewrite of `unit` (one statement) in `ctx`.
///
/// Fail closed. Only code is read: a marker inside a string, quoted identifier, comment, dollar
/// body or the data after a `COPY`'s `;` is never a placeholder. The lexer does not recognise
/// psql meta-commands (`\set x :y` reaches this scan as an unknown `\` and code), so a unit
/// holding any token the lexer could not read cleanly (an unknown character such as `\`, an
/// error or junk, an unterminated string or comment) binds nothing: its placeholders are
/// refused with [`ParamDiag::Unbindable`] at that token. So is a marker the server may read as
/// a parameter but Hacivat cannot bind (inside an executable comment, `?` under `CALL`/`EXEC`,
/// psql `:"name"`/`:{?name}`, `$0`, `$01` and other leading zeros, a malformed `$n`, any marker
/// in a dialect without a lexer profile, any placeholder of a `COPY`, a `PREPARE` or a unit of
/// several statements (a `;` inside an executable comment counts), a placeholder whose
/// replacement would merge with a neighbouring identifier byte, a PostgreSQL `?` whose reading
/// is ambiguous once `?` binds); so
/// are placeholders in DDL ([`ParamDiag::ParamInDdl`], unless `in_ddl`; the head is the
/// caller's or the unit's own first word, and an `EXPLAIN` of DDL counts), styles mixed in one
/// unit ([`ParamDiag::MixedStyles`]), more than 65,535 driver slots
/// ([`ParamDiag::TooManyBinds`]) and any placeholder beside a template variable
/// ([`ParamDiag::TemplateVariable`]). One unbindable placeholder makes the whole unit
/// unbindable: every placeholder is then `bindable: false` and `rewritten` is `None`, so Run
/// refuses the unit and nothing is sent.
///
/// The rewrite replaces each placeholder's range and nothing else (`edits`),
/// deterministically: PostgreSQL `:name` → `$k` by first occurrence (a repeated name reuses
/// `$k`), `?` (with `pg_question_marks`) → `$k` in text order, `$n` unchanged; MySQL,
/// MariaDB, SQLite and the generic dialect `:name` (SQLite also `@name`, `$name`, `#name`) →
/// `?`, one slot per occurrence, `?` and `?NNN` unchanged. Plan names: a `:name`'s name, `?k`
/// for the k-th `?`, a positional placeholder's own text (`$2`, `?2`).
pub fn scan_params(unit: &str, ctx: &DialectCtx, head: &StmtHead, s: &ParamSettings) -> ParamScan {
    if u32::try_from(unit.len()).is_err() {
        let at = TextRange::empty(TextSize::from(0));
        return ParamScan {
            placeholders: vec![Placeholder {
                range: at,
                kind: PlaceholderKind::Named(Arc::from("")),
                bindable: false,
            }],
            diags: vec![(at, ParamDiag::Unbindable)],
            ..ParamScan::default()
        };
    }
    let Ok(profile) = lex_profile(ctx) else {
        return without_profile(unit);
    };
    let lexed = lex(unit, profile, ctx.modes, &LexState::Initial);
    Scanner::new(unit, ctx, head, s, profile, &lexed).run()
}

/// The most driver slots a unit may bind (PostgreSQL's protocol limit, applied everywhere).
const MAX_SLOTS: u64 = 65_535;

/// PostgreSQL operators that contain `?` (jsonb, geometric, jsonpath).
const PG_QUESTION_OPS: &[&str] = &["?|", "?&", "?-", "?-|", "?||", "?#", "@?"];

/// Keywords after which an expression (an operand) starts.
const EXPRESSION_KEYWORDS: &[&str] = &[
    "ALL",
    "AND",
    "ANY",
    "BETWEEN",
    "BY",
    "CASE",
    "DISTINCT",
    "ELSE",
    "EXISTS",
    "FETCH",
    "FIRST",
    "HAVING",
    "ILIKE",
    "IN",
    "IS",
    "LIKE",
    "LIMIT",
    "NEXT",
    "NOT",
    "OFFSET",
    "ON",
    "OR",
    "RETURN",
    "RETURNING",
    "SELECT",
    "SET",
    "SIMILAR",
    "SOME",
    "THEN",
    "USING",
    "VALUES",
    "WHEN",
    "WHERE",
];

/// Token `j` (a word of `text`) is a keyword after which an operand starts.
pub(crate) fn expression_keyword(text: &str, lexed: &Lexed, j: usize) -> bool {
    let r = lexed.range(j);
    let word = &text[usize::from(r.start())..usize::from(r.end())];
    lexed.keyword(j).is_some()
        && EXPRESSION_KEYWORDS
            .iter()
            .any(|k| word.eq_ignore_ascii_case(k))
}

/// `n` as a `TextSize` (callers have checked that the text is under 4 GiB).
pub(crate) fn size_of(n: usize) -> TextSize {
    TextSize::from(u32::try_from(n).unwrap_or(u32::MAX))
}

/// A unit in a dialect without a lexer profile: every `:`, `$`, `?` and `@` byte is an
/// unbindable placeholder (nothing can say which of them the server reads).
fn without_profile(unit: &str) -> ParamScan {
    let placeholders: Vec<Placeholder> = unit
        .bytes()
        .enumerate()
        .filter_map(|(at, b)| {
            let kind = match b {
                b':' | b'@' => PlaceholderKind::Named(Arc::from("")),
                b'$' => PlaceholderKind::Positional(0),
                b'?' => PlaceholderKind::Question(0),
                _ => return None,
            };
            Some(Placeholder {
                range: TextRange::at(size_of(at), TextSize::from(1)),
                kind,
                bindable: false,
            })
        })
        .collect();
    let diags = placeholders
        .first()
        .map(|p| vec![(p.range, ParamDiag::Unbindable)])
        .unwrap_or_default();
    ParamScan {
        placeholders,
        diags,
        ..ParamScan::default()
    }
}

/// An identifier byte in any dialect (`$` included: PostgreSQL and MySQL identifiers hold it,
/// and it opens a dollar quote).
fn ident_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'$' || b >= 0x80
}

/// Neither neighbour of `r` in `unit` is an identifier byte, so a replacement cannot merge
/// with them.
fn isolated(unit: &[u8], r: TextRange) -> bool {
    let (s, e) = (usize::from(r.start()), usize::from(r.end()));
    let before = s.checked_sub(1).map(|p| unit[p]);
    let after = unit.get(e).copied();
    !before.is_some_and(ident_byte) && !after.is_some_and(ident_byte)
}

/// A placeholder style; a unit binds one style only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Style {
    Positional,
    Question,
    Named,
}

/// A placeholder found, and why it cannot bind (`None`: it can, so far).
#[derive(Debug)]
struct Candidate {
    range: TextRange,
    kind: PlaceholderKind,
    style: Style,
    reason: Option<ParamDiag>,
}

struct Scanner<'a> {
    unit: &'a str,
    ctx: &'a DialectCtx,
    s: &'a ParamSettings,
    profile: &'static LexDialect,
    lexed: &'a Lexed,
    pg: bool,
    ddl: bool,
    /// `CALL`/`EXEC`/`EXECUTE`: `?` keeps ODBC escape semantics and never binds.
    call: bool,
    /// `COPY`, `PREPARE`, a unit of several statements (a `;` inside an executable comment
    /// included) take no bind values.
    no_binds: bool,
    /// The first code token the lexer could not read cleanly.
    malformed: Option<TextRange>,
    /// Where code ends: after a `COPY`'s first `;` the rest is data.
    code_end: usize,
    question: u32,
    cands: Vec<Candidate>,
    diags: Vec<(TextRange, ParamDiag)>,
}

impl<'a> Scanner<'a> {
    fn new(
        unit: &'a str,
        ctx: &'a DialectCtx,
        head: &StmtHead,
        s: &'a ParamSettings,
        profile: &'static LexDialect,
        lexed: &'a Lexed,
    ) -> Self {
        // The caller's head and the unit's own first word both count, so a stale head cannot
        // unlock binding in DDL.
        let first = (0..lexed.len())
            .find(|i| !lexed.is_trivia(*i))
            .filter(|i| lexed.kind(*i) == TokenKind::Word);
        let own = first.and_then(|i| lexed.keyword(i)).map(|k| k.id);
        let own_text = first.map_or("", |i| {
            let r = lexed.range(i);
            &unit[usize::from(r.start())..usize::from(r.end())]
        });
        let is = |id: KeywordId| head.first_keyword == Some(id) || own == Some(id);
        // `EXPLAIN [ANALYZE] <statement>`: the inner statement decides DDL. Fail closed: any
        // `CREATE`/`ALTER`/`DROP` keyword in an `EXPLAIN` counts (all three are reserved, so
        // none is a bare column name).
        let explain_ddl = is(kw::EXPLAIN)
            && (0..lexed.len()).any(|i| {
                lexed.kind(i) == TokenKind::Word
                    && lexed
                        .keyword(i)
                        .is_some_and(|k| [kw::CREATE, kw::ALTER, kw::DROP].contains(&k.id))
            });
        let copy = is(kw::COPY) || head.copy_stream;
        let code_end = if copy {
            (0..lexed.len())
                .find(|i| lexed.kind(*i) == TokenKind::Semicolon)
                .map_or(unit.len(), |i| usize::from(lexed.range(i).start()))
        } else {
            unit.len()
        };
        // Several statements in one unit (a verbatim "Run as one" unit): no protocol binds
        // values across statements, and the head says nothing about the later ones.
        let multi = (0..lexed.len())
            .skip_while(|i| lexed.kind(*i) != TokenKind::Semicolon)
            .skip(1)
            .any(|i| !lexed.is_trivia(i) && lexed.kind(i) != TokenKind::Semicolon);
        // A code token the lexer could not read cleanly: an unknown character (psql's `\`
        // meta-commands: `\set x :y`, `:a\gset`, `\g`), an error or junk, or a construct that
        // runs to the end (`SELECT :a '`, `SELECT :a /* …`). Nothing here can say what the
        // server reads as code, so no placeholder of the unit binds.
        let malformed = (0..lexed.len())
            .take_while(|i| usize::from(lexed.range(*i).start()) < code_end)
            .find(|i| {
                lexed.kind(*i) == TokenKind::Unknown
                    || lexed
                        .flags(*i)
                        .intersects(TokenFlags::ERROR | TokenFlags::JUNK | TokenFlags::UNTERMINATED)
            })
            .map(|i| lexed.range(i));
        Scanner {
            unit,
            ctx,
            s,
            profile,
            lexed,
            pg: matches!(ctx.id, DialectId::Postgres | DialectId::PgWire(_)),
            ddl: is(kw::CREATE) || is(kw::ALTER) || is(kw::DROP) || explain_ddl,
            call: is(kw::CALL) || is(kw::EXECUTE) || own_text.eq_ignore_ascii_case("EXEC"),
            no_binds: copy || multi || is(kw::PREPARE),
            malformed,
            code_end,
            question: 0,
            cands: Vec::new(),
            diags: Vec::new(),
        }
    }

    fn text(&self, r: TextRange) -> &'a str {
        &self.unit[usize::from(r.start())..usize::from(r.end())]
    }

    fn run(mut self) -> ParamScan {
        let lexed = self.lexed;
        let mut depth = 0u32;
        for i in 0..lexed.len() {
            let r = lexed.range(i);
            if usize::from(r.start()) >= self.code_end {
                break;
            }
            match lexed.kind(i) {
                TokenKind::LBracket => depth += 1,
                TokenKind::RBracket => depth = depth.saturating_sub(1),
                TokenKind::PositionalParam => self.positional(i, r),
                TokenKind::QuestionParam => self.question_param(r, None),
                TokenKind::NamedParam => self.named(i, r, depth),
                TokenKind::ClientVar => self.client_var(r, depth),
                TokenKind::Op if self.pg => self.pg_op(i, r),
                TokenKind::Op => self.colon_word(lexed, i, 0, None),
                TokenKind::ExecutableComment => self.exec_body(i),
                _ => {}
            }
        }
        self.finish()
    }

    fn push(
        &mut self,
        range: TextRange,
        kind: PlaceholderKind,
        style: Style,
        reason: Option<ParamDiag>,
    ) {
        let reason = if self.call && style == Style::Question {
            reason.or(Some(ParamDiag::Unbindable))
        } else {
            reason
        };
        self.cands.push(Candidate {
            range,
            kind,
            style,
            reason,
        });
    }

    /// A slot number written as `digits` (`$n`, SQLite `?NNN`): the kind's number and why it
    /// cannot bind. Leading zeros are refused: `$01` is the server's `$1`, and two spellings of
    /// one slot would become two plan names (two prompts, two values for one parameter).
    fn slot_number(digits: &str) -> (u32, Option<ParamDiag>) {
        if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
            return (0, Some(ParamDiag::Unbindable));
        }
        if digits.len() > 1 && digits.starts_with('0') {
            let n = digits.parse::<u64>().unwrap_or(u64::MAX);
            return (
                u32::try_from(n).unwrap_or(u32::MAX),
                Some(ParamDiag::Unbindable),
            );
        }
        match digits.parse::<u64>() {
            Ok(0) => (0, Some(ParamDiag::Unbindable)),
            Ok(n) if n <= MAX_SLOTS => (u32::try_from(n).unwrap_or(u32::MAX), None),
            Ok(n) => (
                u32::try_from(n).unwrap_or(u32::MAX),
                Some(ParamDiag::TooManyBinds),
            ),
            Err(_) => (u32::MAX, Some(ParamDiag::TooManyBinds)),
        }
    }

    /// PostgreSQL `$n`.
    fn positional(&mut self, i: usize, r: TextRange) {
        let text = self.text(r);
        let (n, mut reason) = Self::slot_number(text.get(1..).unwrap_or(""));
        if self
            .lexed
            .flags(i)
            .intersects(TokenFlags::ERROR | TokenFlags::JUNK)
        {
            reason = Some(ParamDiag::Unbindable);
        }
        self.push(r, PlaceholderKind::Positional(n), Style::Positional, reason);
    }

    /// `?` (MySQL, MariaDB, SQLite, generic) or SQLite `?NNN`; `forced` marks it unbindable.
    fn question_param(&mut self, r: TextRange, forced: Option<ParamDiag>) {
        let text = self.text(r);
        if text.len() == 1 {
            self.question += 1;
            self.push(
                r,
                PlaceholderKind::Question(self.question),
                Style::Question,
                forced,
            );
        } else {
            let (n, reason) = Self::slot_number(&text[1..]);
            self.push(
                r,
                PlaceholderKind::Positional(n),
                Style::Positional,
                forced.or(reason),
            );
        }
    }

    /// The byte before a `:` at `at` allows a parameter: not an identifier character (so
    /// `a:b` and `$:a` are none), not `:`, `?`, `\`, `/` or `[` (DBeaver's prefix guard).
    fn colon_guard(&self, at: usize) -> bool {
        match at.checked_sub(1).map(|p| self.unit.as_bytes()[p]) {
            None => true,
            Some(c) => !(ident_byte(c) || matches!(c, b':' | b'?' | b'\\' | b'/' | b'[')),
        }
    }

    /// A `NamedParam` token: SQLite's variables (every one read by the server) or `:name`
    /// (PostgreSQL, generic: outside `[…]`, after the prefix guard).
    fn named(&mut self, i: usize, r: TextRange, depth: u32) {
        let text = self.text(r);
        if self.ctx.id == DialectId::Sqlite {
            let colon = text.starts_with(':');
            let name = if colon { &text[1..] } else { text };
            let malformed = self.lexed.flags(i).contains(TokenFlags::ERROR);
            // SQLite binds every variable it reads (NULL when unbound): one Hacivat may not
            // bind is refused, never sent.
            let reason =
                (malformed || (colon && !self.s.colon_params)).then_some(ParamDiag::Unbindable);
            self.push(
                r,
                PlaceholderKind::Named(Arc::from(name)),
                Style::Named,
                reason,
            );
            return;
        }
        let at = usize::from(r.start());
        if self.s.colon_params && depth == 0 && self.colon_guard(at) && text.len() > 1 {
            self.push(
                r,
                PlaceholderKind::Named(Arc::from(&text[1..])),
                Style::Named,
                None,
            );
        }
    }

    /// psql `:'name'` (a value: it binds), `:"name"` (an identifier) and `:{?name}` (a test):
    /// the last two cannot bind.
    fn client_var(&mut self, r: TextRange, depth: u32) {
        let text = self.text(r);
        let at = usize::from(r.start());
        if !(self.s.colon_params && depth == 0 && self.colon_guard(at)) || text.len() < 4 {
            return;
        }
        let inner = &text[2..text.len() - 1];
        let (name, reason) = match text.as_bytes()[1] {
            b'\'' => (inner, None),
            b'"' => (inner, Some(ParamDiag::Unbindable)),
            _ => (
                inner.strip_prefix('?').unwrap_or(inner),
                Some(ParamDiag::Unbindable),
            ),
        };
        self.push(
            r,
            PlaceholderKind::Named(Arc::from(name)),
            Style::Named,
            reason,
        );
    }

    /// A PostgreSQL operator token: `?` binds only with `pg_question_marks` and in operand
    /// position (else it is the jsonb operator; a hint when it looks like a parameter). Once
    /// `?` binds, these are ambiguous and refused: an operator that merely contains `?`
    /// (`=?`); a `?`-operator in operand position (`a = ?-1`: `$1 - 1` or the `?-` operator);
    /// a `?` beside another `?`-operator (`WHERE ? ? 'k'`, `(j) ? ?`); a `?` with a literal
    /// right after it (`first ? 'k'`: `first` may be a column, the `?` the jsonb operator).
    fn pg_op(&mut self, i: usize, r: TextRange) {
        let text = self.text(r);
        if text == "?" {
            let operand = self.operand_position(i);
            if self.s.pg_question_marks {
                if operand {
                    let ambiguous = self.prev_is_question_op(i) || self.next_reads_ambiguous(i);
                    self.question += 1;
                    self.push(
                        r,
                        PlaceholderKind::Question(self.question),
                        Style::Question,
                        ambiguous.then_some(ParamDiag::Unbindable),
                    );
                } else if self.next_is_question_op(i) {
                    // `j ? ?|…`: which `?` is the operator cannot be told.
                    self.question += 1;
                    self.push(
                        r,
                        PlaceholderKind::Question(self.question),
                        Style::Question,
                        Some(ParamDiag::Unbindable),
                    );
                }
            } else if operand {
                self.diags.push((r, ParamDiag::QuestionMarkOff));
            }
        } else if self.s.pg_question_marks
            && text.contains('?')
            && (!PG_QUESTION_OPS.contains(&text) || self.operand_position(i))
        {
            self.question += 1;
            self.push(
                r,
                PlaceholderKind::Question(self.question),
                Style::Question,
                Some(ParamDiag::Unbindable),
            );
        }
    }

    fn prev_significant(&self, i: usize) -> Option<usize> {
        (0..i).rev().find(|j| !self.lexed.is_trivia(*j))
    }

    fn next_significant(&self, i: usize) -> Option<usize> {
        (i + 1..self.lexed.len()).find(|j| !self.lexed.is_trivia(*j))
    }

    /// Token `j` is an operator containing `?` (`?` itself, `?|`, `?&`, `@?`, `=?`, …).
    fn question_op(&self, j: usize) -> bool {
        self.lexed.kind(j) == TokenKind::Op && self.text(self.lexed.range(j)).contains('?')
    }

    fn prev_is_question_op(&self, i: usize) -> bool {
        self.prev_significant(i)
            .is_some_and(|j| self.question_op(j))
    }

    fn next_is_question_op(&self, i: usize) -> bool {
        self.next_significant(i)
            .is_some_and(|j| self.question_op(j))
    }

    /// After a `?` taken as a placeholder: another `?`-operator, or a literal (the `?` would
    /// sit between two operands, where it is the jsonb operator).
    fn next_reads_ambiguous(&self, i: usize) -> bool {
        self.next_significant(i).is_some_and(|j| {
            self.question_op(j)
                || matches!(
                    self.lexed.kind(j),
                    TokenKind::String
                        | TokenKind::DollarString
                        | TokenKind::Number
                        | TokenKind::BitString
                        | TokenKind::HexString
                )
        })
    }

    /// The previous significant token leaves an operand position.
    fn operand_position(&self, i: usize) -> bool {
        match (0..i).rev().find(|j| !self.lexed.is_trivia(*j)) {
            None => true,
            Some(j) => match self.lexed.kind(j) {
                TokenKind::Op | TokenKind::LParen | TokenKind::Comma | TokenKind::LBracket => true,
                TokenKind::Word => expression_keyword(self.unit, self.lexed, j),
                _ => false,
            },
        }
    }

    /// MySQL/MariaDB `:name`: a `:` operator token directly followed by a word, in `lexed`
    /// (the unit, or an executable comment body at `base`). `forced` marks it unbindable.
    fn colon_word(&mut self, lexed: &Lexed, i: usize, base: usize, forced: Option<ParamDiag>) {
        let r = lexed.range(i);
        let (s, e) = (base + usize::from(r.start()), base + usize::from(r.end()));
        if !matches!(self.ctx.id, DialectId::MySql | DialectId::MariaDb)
            || &self.unit[s..e] != ":"
            || !self.s.colon_params
            || !self.colon_guard(s)
            || i + 1 >= lexed.len()
        {
            return;
        }
        let wr = lexed.range(i + 1);
        if lexed.kind(i + 1) != TokenKind::Word || base + usize::from(wr.start()) != e {
            return;
        }
        let we = base + usize::from(wr.end());
        self.push(
            TextRange::new(size_of(s), size_of(we)),
            PlaceholderKind::Named(Arc::from(&self.unit[e..we])),
            Style::Named,
            forced,
        );
    }

    /// An executable comment's body is code to the server: every marker in it is found, and
    /// none binds.
    fn exec_body(&mut self, i: usize) {
        let Some(ec) = exec_comment(self.unit, self.lexed, i) else {
            return;
        };
        let base = usize::from(ec.body.start());
        let body = &self.unit[base..usize::from(ec.body.end())];
        let sub = lex(body, self.profile, self.ctx.modes, &LexState::Initial);
        // A `;` the server runs inside the comment starts a second statement: refused like
        // any unit of several statements.
        if (0..sub.len()).any(|j| sub.kind(j) == TokenKind::Semicolon) {
            self.no_binds = true;
        }
        let forced = Some(ParamDiag::Unbindable);
        for j in 0..sub.len() {
            let r = sub.range(j);
            let at = TextRange::new(
                size_of(base + usize::from(r.start())),
                size_of(base + usize::from(r.end())),
            );
            match sub.kind(j) {
                TokenKind::QuestionParam => self.question_param(at, forced),
                TokenKind::NamedParam | TokenKind::PositionalParam => self.push(
                    at,
                    PlaceholderKind::Named(Arc::from(self.text(at))),
                    Style::Named,
                    forced,
                ),
                TokenKind::Op => self.colon_word(&sub, j, base, forced),
                _ => {}
            }
        }
    }

    fn finish(mut self) -> ParamScan {
        let mut templates = Vec::new();
        templates::find(
            self.unit,
            self.lexed,
            self.profile,
            self.ctx.modes,
            0,
            &mut templates,
        );
        templates.retain(|t| usize::from(t.range.start()) < self.code_end);
        templates.sort_by_key(|t| (t.range.start(), t.range.end()));

        let unit = self.unit.as_bytes();
        let pg = self.pg;
        for c in &mut self.cands {
            // A replacement must not run into its neighbours: `:'v'2` → `$12`, `SELECT?` →
            // `SELECT$1` (one identifier), SQLite `$a(x)1` → `?1` would change what the server
            // reads, so such a placeholder does not bind.
            let replaced = matches!(c.kind, PlaceholderKind::Named(_))
                || (pg && matches!(c.kind, PlaceholderKind::Question(_)));
            if replaced && c.reason.is_none() && !isolated(unit, c.range) {
                c.reason = Some(ParamDiag::Unbindable);
            }
            if self.ddl && !self.s.in_ddl {
                c.reason = Some(ParamDiag::ParamInDdl);
            } else if self.no_binds {
                c.reason = c.reason.or(Some(ParamDiag::Unbindable));
            }
        }
        let mut blocked = !templates.is_empty();
        let mut diags = std::mem::take(&mut self.diags);
        for c in &self.cands {
            if let Some(d) = c.reason {
                diags.push((c.range, d));
                blocked = true;
            }
        }
        for t in &templates {
            diags.push((t.range, ParamDiag::TemplateVariable));
        }
        if let Some(first) = self.cands.first() {
            // A token the lexer could not read cleanly (this includes a comment whose end the
            // server decides, `AmbiguousCommentEnd`, which is flagged `ERROR`): nothing here
            // can say what is code.
            if let Some(r) = self.malformed {
                diags.push((r, ParamDiag::Unbindable));
                blocked = true;
            }
            if let Some(c) = self.cands.iter().find(|c| c.style != first.style) {
                diags.push((c.range, ParamDiag::MixedStyles));
                blocked = true;
            }
            if let Some(r) = self.first_over_limit() {
                if !diags.iter().any(|(_, d)| *d == ParamDiag::TooManyBinds) {
                    diags.push((r, ParamDiag::TooManyBinds));
                }
                blocked = true;
            }
        }
        diags.sort_by_key(|(r, _)| r.start());

        let bindable = !blocked && !self.cands.is_empty();
        let rewritten = bindable.then(|| self.rewrite());
        let placeholders = self
            .cands
            .into_iter()
            .map(|c| Placeholder {
                range: c.range,
                kind: c.kind,
                bindable,
            })
            .collect();
        ParamScan {
            placeholders,
            templates,
            rewritten,
            diags,
        }
    }

    /// The first placeholder that takes the unit past [`MAX_SLOTS`] driver slots.
    fn first_over_limit(&self) -> Option<TextRange> {
        let mut names: HashSet<&str> = HashSet::new();
        let mut count = 0u64;
        let mut max = 0u64;
        for c in &self.cands {
            let slots = match &c.kind {
                PlaceholderKind::Named(n) if self.pg => {
                    names.insert(n);
                    names.len() as u64
                }
                PlaceholderKind::Named(_) | PlaceholderKind::Question(_) => {
                    count += 1;
                    count
                }
                PlaceholderKind::Positional(n) => {
                    max = max.max(u64::from(*n));
                    max
                }
            };
            if slots > MAX_SLOTS {
                return Some(c.range);
            }
        }
        None
    }

    /// The driver's text and bind plan (every placeholder binds, in one style).
    fn rewrite(&self) -> RewrittenSql {
        let mut plan = BindPlan::default();
        let mut index: HashMap<Arc<str>, usize> = HashMap::new();
        let mut text = String::with_capacity(self.unit.len());
        let mut edits = Vec::new();
        let mut at = 0usize;
        let mut occurrence = 0u32;
        for c in &self.cands {
            let name: Arc<str> = match &c.kind {
                PlaceholderKind::Named(n) => n.clone(),
                PlaceholderKind::Question(q) => Arc::from(format!("?{q}")),
                PlaceholderKind::Positional(_) => Arc::from(self.text(c.range)),
            };
            let (i, new) = match index.get(&name) {
                Some(i) => (*i, false),
                None => {
                    let i = plan.names.len();
                    index.insert(name.clone(), i);
                    plan.names.push(name);
                    plan.slots.push(SmallVec::new());
                    (i, true)
                }
            };
            let k = u32::try_from(i).unwrap_or(u32::MAX);
            let replacement = match (&c.kind, self.pg) {
                (PlaceholderKind::Named(_), true) => {
                    if new {
                        plan.slots[i].push(k);
                    }
                    Some(format!("${}", k + 1))
                }
                (PlaceholderKind::Named(_), false) => {
                    plan.slots[i].push(occurrence);
                    occurrence += 1;
                    Some("?".to_string())
                }
                (PlaceholderKind::Question(q), pg) => {
                    plan.slots[i].push(q - 1);
                    pg.then(|| format!("${q}"))
                }
                (PlaceholderKind::Positional(n), _) => {
                    if new {
                        plan.slots[i].push(n - 1);
                    }
                    None
                }
            };
            if let Some(rep) = replacement {
                let (s, e) = (usize::from(c.range.start()), usize::from(c.range.end()));
                text.push_str(&self.unit[at..s]);
                let to = text.len();
                text.push_str(&rep);
                edits.push((c.range, TextRange::new(size_of(to), size_of(text.len()))));
                at = e;
            }
        }
        text.push_str(&self.unit[at..]);
        RewrittenSql { text, plan, edits }
    }
}

#[cfg(test)]
mod tests {
    use sql_dialect::DialectId;

    use super::*;

    fn pg() -> DialectCtx {
        DialectCtx::new(DialectId::Postgres, None)
    }

    #[test]
    fn a_unit_without_markers_has_nothing_to_bind() {
        let s = scan_params(
            "SELECT 1",
            &pg(),
            &StmtHead::default(),
            &ParamSettings::default(),
        );
        assert_eq!(s, ParamScan::default());
    }

    #[test]
    fn edits_map_unit_ranges_to_rewritten_ranges() {
        let s = scan_params(
            "SELECT :long_name, :b",
            &pg(),
            &StmtHead::default(),
            &ParamSettings::default(),
        );
        let r = s.rewritten.unwrap();
        assert_eq!(r.text, "SELECT $1, $2");
        assert_eq!(
            r.edits,
            [
                (
                    TextRange::at(7.into(), 10.into()),
                    TextRange::at(7.into(), 2.into())
                ),
                (
                    TextRange::at(19.into(), 2.into()),
                    TextRange::at(11.into(), 2.into())
                ),
            ]
        );
    }

    #[test]
    fn slot_numbers_fail_closed() {
        assert_eq!(Scanner::slot_number("1"), (1, None));
        assert_eq!(Scanner::slot_number("0"), (0, Some(ParamDiag::Unbindable)));
        assert_eq!(
            Scanner::slot_number("65536"),
            (65_536, Some(ParamDiag::TooManyBinds))
        );
        assert_eq!(
            Scanner::slot_number("99999999999999999999999"),
            (u32::MAX, Some(ParamDiag::TooManyBinds))
        );
        assert_eq!(Scanner::slot_number(""), (0, Some(ParamDiag::Unbindable)));
    }

    #[test]
    fn default_settings_follow_d08() {
        let s = ParamSettings::default();
        assert!(!s.pg_question_marks && s.colon_params && !s.in_ddl);
    }
}
