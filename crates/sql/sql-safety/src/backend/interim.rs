//! The interim backend for MySQL, MariaDB and SQLite (Task 10; R-M3-54): `sqlparser`'s derived
//! `Visit`, unioned with the lexer pre-scan. Combine rule `MostDangerousOfAll { deny_if_any_fails:
//! true }` (`alg/11` :66).
//!
//! For each statement range:
//!
//! 1. **Lex** it with our lexer. A lexing error is `LexError`; an `AmbiguousCommentEnd` comment
//!    is read both ways and is `AmbiguousBoundary` (ledger 126/138).
//! 2. **Threshold views.** Each executable comment (`/*!…*/`; MariaDB also `/*M!…*/`) is read
//!    active (its markers blanked, its body code) and inactive (blanked whole) at every version
//!    threshold: with `k` distinct gate versions there are `k + 1` readings, and an unversioned
//!    comment is always active. An ambiguous end doubles the active readings (the skipped and
//!    the running end). Blanking keeps every byte offset. More than
//!    `Limits.max_exec_comment_views` (16) readings is `TooComplex`. Any executable comment
//!    sets `EXEC_COMMENT` with `ExecutableComment` (the gate's `Unknown`, D-058).
//! 3. **Each view** is lexed again and tokenized by `sqlparser`; the two must agree on which
//!    bytes are code, quoted text or trivia, and on every quoted token's extent (spans mapped
//!    from `sqlparser`'s line and character column through [`LineIndex`]), else
//!    `ParserDisagreement`. Both must count the same statements (else `ParserDisagreement`;
//!    more than one is `MultiStatement`). The pre-scan inventory runs on the view. Then the
//!    view must parse in strict mode (`MySqlDialect` for MySQL and MariaDB, `SQLiteDialect`;
//!    recursion limit 50): a failure is `ParseError` (`TooComplex` at the limit), except the
//!    narrow `SHOW`/`DESCRIBE`/`EXPLAIN` grammars (P-M3b-35) and SQLite's `PRAGMA` grammar.
//! 4. The **walk** (derived `Visit`) maps every statement, top-level or nested, through
//!    `data/<engine>/statements.toml`, and adds locking clauses, `INTO`, data-modifying CTEs,
//!    `EXPLAIN ANALYZE`, every function call (SAFE-03, `lookup_fn` then the allowlist and the
//!    catalog), touched objects and the SAFE-07 destructive patterns.
//! 5. **Union** over the views: effects, flags, reasons, destructive patterns and targets; the
//!    kind of the most critical view. Any doubt code (`reason_rule` `DenyAlways` or the
//!    `unknown` group) adds `UNKNOWN` and makes the class `Unknown`: the class field is never
//!    the only carrier of an Unknown outcome (the T4 producer contract).
//!
//! No `sqlparser` type leaves this module: the public items take and give `sql-safety` types.

use std::collections::HashSet;
use std::fmt::{self, Write as _};
use std::ops::ControlFlow;
use std::sync::{Arc, OnceLock};

use serde::Deserialize;
use smallvec::{SmallVec, smallvec};
use sql_dialect::{DialectCtx, DialectId, lex_profile};
use sql_ident::{Ident, QualifiedName};
use sql_lexer::{LexErrorCode, LexState, Lexed, TokenKind, exec_comment, lex};
use sql_split::SplitMode;
use sql_text::{LineCol, LineIndex, TextRange};
use sqlparser::ast::{
    Expr, FunctionArguments, ObjectName, ObjectNamePart, Query, Select, Statement, TableFactor,
    TableObject, TableWithJoins, Value, Visit, Visitor,
};
use sqlparser::dialect::{Dialect, MySqlDialect, SQLiteDialect};
use sqlparser::parser::{Parser, ParserError, ParserOptions};
use sqlparser::tokenizer::{Location, Token, TokenWithSpan, Tokenizer};

use super::Backend;
use super::prescan::{self, Acc, Describe, Toks, range_of};
use crate::classify::{ClassifyCtx, Expect};
use crate::destructive::{DestructiveKind, DestructiveReason};
use crate::effects::{CoarseClass, EffectSet, RiskFlags, class_of};
use crate::gate_class::gate_class;
use crate::policy::{Group, ReasonRule, reason_rule};
use crate::reason::{Reason, ReasonCode};
use crate::report::{
    BackendVersion, Confidence, ObjectKind, ObjectRef, SafetyReport, StmtKind, StmtReport, merge,
};

/// The parser this backend runs (R-M3-54), recorded in `SafetyReport.backends`.
pub(crate) const SQLPARSER: BackendVersion = BackendVersion {
    name: "sqlparser",
    version: "0.64.0",
};

/// The lexer pre-scan's version (`prescan`), recorded in `SafetyReport.backends`.
pub(crate) const PRESCAN: BackendVersion = BackendVersion {
    name: "lexer-prescan",
    version: "1",
};

/// `sqlparser`'s recursion limit (R-M3-54).
const RECURSION_LIMIT: usize = 50;

/// How deep a narrow `EXPLAIN` may nest before the inner statement is not classified.
const MAX_EXPLAIN_DEPTH: u8 = 2;

/// The interim backend (R-M3-54): MySQL and MariaDB through `MySqlDialect`, SQLite through
/// `SQLiteDialect`; every other dialect is `UnsupportedDialect`.
pub(crate) struct InterimBackend;

impl Backend for InterimBackend {
    fn analyse(&self, sql: &str, items: &[TextRange], ctx: &ClassifyCtx) -> Vec<StmtReport> {
        items.iter().map(|r| analyse_stmt(sql, *r, ctx)).collect()
    }
}

// ---------------------------------------------------------------------------------------------
// The Task 10 harness

/// Classifies `sql` with the interim backend alone: the splitter's statements (execution mode)
/// each analysed, an uncertain boundary or more statements than `ctx.expect` allows marked
/// `Unknown`, merged as `classify` merges. Task 10's tests and corpus run through it; Task 14's
/// [`crate::classify`] is the production entry and supersedes it (it adds the limits, the
/// templates and the per-dialect plan).
pub fn classify_interim(sql: &str, ctx: &ClassifyCtx) -> SafetyReport {
    let fingerprint = sql_split::hv_norm1(sql, &ctx.dialect);
    let decided = |code: ReasonCode| {
        let mut r = SafetyReport::pre_decided(sql, fingerprint.clone(), vec![Reason::new(code)]);
        r.dialect = Some(ctx.dialect.id);
        r.backends = smallvec![SQLPARSER, PRESCAN];
        r
    };
    if !supported(ctx.dialect.id) {
        return decided(ReasonCode::UnsupportedDialect);
    }
    let Ok(profile) = lex_profile(&ctx.dialect) else {
        return decided(ReasonCode::UnsupportedDialect);
    };
    let lexed = lex(sql, profile, ctx.dialect.modes, &LexState::Initial);
    let split = sql_split::split(sql, &lexed, &ctx.dialect, ctx.flavor, SplitMode::Execution);
    if !split.client.is_empty() {
        return decided(ReasonCode::ClientCommand);
    }
    let stmts: Vec<_> = split.statements().collect();
    if stmts.is_empty() {
        return decided(ReasonCode::NoStatement);
    }
    // The strict range is the text sent (no leading comments, no terminator): classification
    // sees exactly the bytes sent (`alg/13` :150).
    let ranges: Vec<TextRange> = stmts.iter().map(|s| s.strict).collect();
    let mut items = InterimBackend.analyse(sql, &ranges, ctx);
    let any_ambiguity = !split.ambiguous.is_empty();
    let several = ctx.expect == Expect::SingleStatement && items.len() > 1;
    for (item, stmt) in items.iter_mut().zip(&stmts) {
        if any_ambiguity || !split.is_certain(stmt) {
            mark_unknown(item, ReasonCode::AmbiguousBoundary);
        }
        if several {
            mark_unknown(item, ReasonCode::MultiStatement);
        }
    }
    let (request_class, request_effects, flags) = merge(&items);
    SafetyReport {
        items,
        request_class,
        request_effects,
        flags,
        fingerprint,
        backends: smallvec![SQLPARSER, PRESCAN],
        pre_decision: None,
        classified_len: u32::try_from(sql.len()).unwrap_or(u32::MAX),
        classified_hash: crate::fingerprint::classified_hash(sql),
        dialect: Some(ctx.dialect.id),
    }
}

fn mark_unknown(item: &mut StmtReport, code: ReasonCode) {
    item.effects |= EffectSet::UNKNOWN;
    item.class = CoarseClass::Unknown;
    let reason = Reason {
        code,
        range: Some(item.range),
        arg: None,
    };
    if !item.reasons.contains(&reason) {
        item.reasons.push(reason);
    }
}

fn supported(d: DialectId) -> bool {
    matches!(d, DialectId::MySql | DialectId::MariaDb | DialectId::Sqlite)
}

// ---------------------------------------------------------------------------------------------
// One statement

/// One executable comment of a statement, as the dialect's server reads it.
struct ExecComment {
    /// The whole token.
    token: (usize, usize),
    /// The opener with the version digits the server reads (`/*!50000`; MySQL reads five of
    /// six digits, the sixth is body text).
    opener: (usize, usize),
    /// The closing `*/` the skipping reading ends at.
    close: Option<(usize, usize)>,
    /// The server version from which the body runs; `None`: always.
    version: Option<u32>,
    /// The two readings of the comment's end differ: the running reading's `*/` (or `None`
    /// when it does not close inside the statement).
    running: Option<Option<(usize, usize)>>,
}

fn analyse_stmt(sql: &str, range: TextRange, ctx: &ClassifyCtx) -> StmtReport {
    let start = usize::from(range.start());
    let end = usize::from(range.end()).min(sql.len());
    let text = sql.get(start..end).unwrap_or("");
    let mut base = Acc::default();
    let views = statement_views(text, ctx, &mut base);
    let outs: Vec<Acc> = match views {
        Some((views, lexed)) => {
            if views.len() > 1 {
                base.conservative = true;
            }
            // The one reading of a statement without executable comments is its text: its
            // lexing is reused.
            let same = views.len() == 1 && views[0] == text;
            views
                .iter()
                .map(|v| analyse_view(v, ctx, 0, same.then_some(&lexed)))
                .collect()
        }
        None => Vec::new(),
    };
    finalize(range, text, base, outs)
}

/// The readings of `text` and its lexing, or `None` when the statement is decided by `base`
/// already (a lexing error, an unsupported mode, too many readings).
fn statement_views(text: &str, ctx: &ClassifyCtx, base: &mut Acc) -> Option<(Vec<String>, Lexed)> {
    let d = ctx.dialect.id;
    let whole = Some(range_of(0, text.len()));
    if !supported(d) {
        base.doubt(ReasonCode::UnsupportedDialect, whole);
        return None;
    }
    if ctx.dialect.modes.mariadb_oracle_mode {
        // `sql_mode=ORACLE` changes MariaDB's grammar: no parser here reads it.
        base.doubt(ReasonCode::UnsupportedDialect, whole);
        return None;
    }
    let Ok(profile) = lex_profile(&ctx.dialect) else {
        base.doubt(ReasonCode::UnsupportedDialect, whole);
        return None;
    };
    let lexed = lex(text, profile, ctx.dialect.modes, &LexState::Initial);
    let mut ambiguous_tokens = Vec::new();
    for e in lexed.errors() {
        if e.code == LexErrorCode::AmbiguousCommentEnd {
            ambiguous_tokens.push(e.range);
        } else {
            base.doubt(ReasonCode::LexError, Some(e.range));
        }
    }
    if base.effects.contains(EffectSet::UNKNOWN) {
        return None;
    }
    let comments = exec_comments(text, &lexed, &ctx.dialect, &ambiguous_tokens);
    for c in &comments {
        base.flags |= RiskFlags::EXEC_COMMENT;
        base.reason(
            ReasonCode::ExecutableComment,
            Some(range_of(c.token.0, c.token.1)),
            None,
        );
        if c.running.is_some() {
            base.reason(
                ReasonCode::AmbiguousBoundary,
                Some(range_of(c.token.0, c.token.1)),
                None,
            );
            base.effects |= EffectSet::UNKNOWN;
        }
    }
    build_views(text, &comments, ctx, base).map(|v| (v, lexed))
}

fn exec_comments(
    text: &str,
    lexed: &Lexed,
    ctx: &DialectCtx,
    ambiguous: &[TextRange],
) -> Vec<ExecComment> {
    let mut out = Vec::new();
    for i in 0..lexed.len() {
        if ctx.id == DialectId::MySql
            && let Some(c) = mariadb_comment_on_mysql(text, lexed, i)
        {
            out.push(c);
            continue;
        }
        let Some(c) = exec_comment(text, lexed, i) else {
            continue;
        };
        let token = lexed.range(i);
        let (t0, t1) = (usize::from(token.start()), usize::from(token.end()));
        let mut opener = (usize::from(c.opener.start()), usize::from(c.opener.end()));
        let mut version = c.version;
        // MySQL reads five version digits; a sixth is body text (`exec_comment_gate`).
        if ctx.id == DialectId::MySql && opener.1 - opener.0 == 3 + 6 {
            opener.1 -= 1;
            version = version.map(|v| v / 10);
        }
        let body_end = usize::from(c.body.end());
        let close = (body_end < t1).then_some((body_end, t1));
        let running = ambiguous.contains(&token).then(|| {
            let mariadb = ctx.id == DialectId::MariaDb;
            prescan::running_close(text, opener.1, text.len(), ctx.modes, mariadb)
                .map(|e| (e - 2, e))
        });
        out.push(ExecComment {
            token: (t0, t1),
            opener,
            close,
            version,
            running,
        });
    }
    out
}

/// A MariaDB `/*M!…*/` comment on a MySQL connection. MySQL skips it, but a MariaDB server
/// behind a MySQL profile runs it, so its body is read as a threshold view too (the most
/// dangerous reading wins; P-M3b-8). Our MySQL lexer reads it as an ordinary block comment,
/// which ends at the first `*/`.
fn mariadb_comment_on_mysql(text: &str, lexed: &Lexed, i: usize) -> Option<ExecComment> {
    if lexed.kind(i) != TokenKind::BlockComment {
        return None;
    }
    let r = lexed.range(i);
    let (t0, t1) = (usize::from(r.start()), usize::from(r.end()));
    let token = text.get(t0..t1)?;
    let rest = token.strip_prefix("/*M!")?;
    let digits = rest.bytes().take(6).take_while(u8::is_ascii_digit).count();
    let (version, opener_len) = if digits >= 5 {
        let v = rest[..digits].parse::<u32>().ok()?;
        (Some(v), 4 + digits)
    } else {
        (None, 4)
    };
    let closed = token.len() >= opener_len + 2 && token.ends_with("*/");
    Some(ExecComment {
        token: (t0, t1),
        opener: (t0, t0 + opener_len),
        close: closed.then_some((t1 - 2, t1)),
        version,
        running: None,
    })
}

/// Every threshold reading of `text` (deduplicated), or `None` past the limit.
fn build_views(
    text: &str,
    comments: &[ExecComment],
    ctx: &ClassifyCtx,
    base: &mut Acc,
) -> Option<Vec<String>> {
    let mut thresholds: Vec<u32> = comments.iter().filter_map(|c| c.version).collect();
    thresholds.sort_unstable();
    thresholds.dedup();
    let ambiguous: Vec<usize> = comments
        .iter()
        .enumerate()
        .filter_map(|(k, c)| c.running.is_some().then_some(k))
        .collect();
    let limit = usize::from(ctx.limits.max_exec_comment_views);
    let raw = (thresholds.len() + 1).checked_shl(u32::try_from(ambiguous.len()).ok()?);
    if raw.is_none_or(|n| n > limit.saturating_mul(4)) {
        base.doubt(ReasonCode::TooComplex, Some(range_of(0, text.len())));
        return None;
    }
    let mut seen = HashSet::new();
    let mut views = Vec::new();
    for t in 0..=thresholds.len() {
        for mask in 0u32..(1 << ambiguous.len()) {
            let mut bytes = text.as_bytes().to_vec();
            for (k, c) in comments.iter().enumerate() {
                let active = c.version.is_none_or(|v| {
                    thresholds
                        .iter()
                        .position(|x| *x == v)
                        .is_some_and(|p| p < t)
                });
                let running = ambiguous
                    .iter()
                    .position(|a| *a == k)
                    .is_some_and(|bit| mask & (1 << bit) != 0);
                if !active {
                    blank(&mut bytes, c.token.0, c.token.1);
                    continue;
                }
                blank(&mut bytes, c.opener.0, c.opener.1);
                match (running, c.running) {
                    (true, Some(Some((a, b)))) => blank(&mut bytes, a, b),
                    (true, _) => {}
                    (false, _) => {
                        if let Some((a, b)) = c.close {
                            blank(&mut bytes, a, b);
                        }
                    }
                }
            }
            // Blanking whole ASCII bytes inside comment markers keeps the text UTF-8; a
            // multi-byte character inside a blanked inactive body becomes spaces as a whole.
            let Ok(view) = String::from_utf8(bytes) else {
                base.doubt(ReasonCode::LexError, Some(range_of(0, text.len())));
                return None;
            };
            if seen.insert(view.clone()) {
                views.push(view);
            }
        }
    }
    if views.len() > limit {
        base.doubt(ReasonCode::TooComplex, Some(range_of(0, text.len())));
        return None;
    }
    Some(views)
}

/// Replaces `bytes[a..b]` with spaces, keeping line breaks (so lines and offsets stay).
fn blank(bytes: &mut [u8], a: usize, b: usize) {
    let b = b.min(bytes.len());
    for x in &mut bytes[a.min(b)..b] {
        if *x != b'\n' && *x != b'\r' {
            *x = b' ';
        }
    }
}

// ---------------------------------------------------------------------------------------------
// One view

/// One reading; `pre` is its lexing when the caller has it (the reading is the statement).
fn analyse_view(view: &str, ctx: &ClassifyCtx, depth: u8, pre: Option<&Lexed>) -> Acc {
    let mut acc = Acc::default();
    let d = ctx.dialect.id;
    let Ok(profile) = lex_profile(&ctx.dialect) else {
        acc.doubt(ReasonCode::UnsupportedDialect, None);
        return acc;
    };
    let owned;
    let lexed = match pre {
        Some(l) => l,
        None => {
            owned = lex(view, profile, ctx.dialect.modes, &LexState::Initial);
            &owned
        }
    };
    let t = Toks::new(view, lexed);
    let whole = t.whole();
    if t.len() == 0 {
        return acc;
    }
    if !lexed.errors().is_empty() {
        acc.doubt(ReasonCode::LexError, Some(lexed.errors()[0].range));
        return acc;
    }
    if (0..lexed.len()).any(|i| lexed.kind(i) == TokenKind::ExecutableComment) {
        // An executable comment inside a body: a reading this backend does not follow.
        acc.doubt(ReasonCode::TooComplex, whole);
        return acc;
    }
    prescan::lexical(&t, d, &mut acc);
    let dialect = sql_dialect_for(d);
    // One tokenization serves the agreement check and the parse.
    let tokens = Tokenizer::new(dialect, view)
        .with_unescape(false)
        .tokenize_with_location();
    let agreed = match &tokens {
        Ok(tokens) => agreement(view, lexed, tokens),
        // Our lexer read the view without an error; a tokenizer that cannot disagrees.
        Err(_) => Err(ReasonCode::ParserDisagreement),
    };
    if let Err(code) = agreed {
        acc.doubt(code, whole);
    }
    prescan::inventory(&t, ctx, &mut acc);
    prescan::hints(&t, d, &mut acc);
    if d == DialectId::Sqlite && t.is(0, "PRAGMA") {
        // SQLite's `PRAGMA` grammar decides (sqlparser rejects `PRAGMA table_info(t)`).
        prescan::pragma(&t, &mut acc);
        if count_statements(&t) != 1 {
            acc.doubt(ReasonCode::MultiStatement, whole);
        }
        return acc;
    }
    if d != DialectId::Sqlite && t.is(0, "SET") {
        prescan::set_statement(&t, &mut acc);
    }
    let ours = count_statements(&t);
    let parsed = match tokens {
        Ok(tokens) => parse(tokens, dialect),
        Err(e) => Err(ParserError::TokenizerError(e.to_string())),
    };
    match parsed {
        Ok(stmts) => {
            if stmts.len() != ours {
                acc.doubt(ReasonCode::ParserDisagreement, whole);
            }
            if stmts.len() > 1 || ours > 1 {
                acc.doubt(ReasonCode::MultiStatement, whole);
            }
            let mut walk = Walk::new(ctx, whole, &mut acc);
            for s in &stmts {
                let _ = s.visit(&mut walk);
            }
        }
        Err(ParserError::RecursionLimitExceeded) => acc.doubt(ReasonCode::TooComplex, whole),
        Err(_) => narrow(view, &t, ctx, depth, &mut acc),
    }
    acc
}

/// The narrow read grammars (P-M3b-35) for a view `sqlparser` rejected; else `ParseError`.
fn narrow(view: &str, t: &Toks<'_>, ctx: &ClassifyCtx, depth: u8, acc: &mut Acc) {
    let whole = t.whole();
    let mysql = ctx.dialect.id != DialectId::Sqlite;
    if mysql && prescan::narrow_show(t) {
        acc.kind = Some(StmtKind::Show);
        acc.effects |= EffectSet::READ;
        return;
    }
    let inner_at = match prescan::narrow_describe(t) {
        Describe::Table if mysql => {
            acc.kind = Some(StmtKind::Show);
            acc.effects |= EffectSet::READ;
            return;
        }
        Describe::Explain(at) => Some(at),
        _ => prescan::narrow_explain(t),
    };
    match inner_at {
        Some(at) if mysql && depth < MAX_EXPLAIN_DEPTH => {
            let mut bytes = view.as_bytes().to_vec();
            blank(&mut bytes, 0, at);
            let Ok(inner) = String::from_utf8(bytes) else {
                acc.doubt(ReasonCode::ParseError, whole);
                return;
            };
            let inner = analyse_view(&inner, ctx, depth + 1, None);
            acc.absorb(inner);
            acc.kind = Some(StmtKind::Explain { analyze: false });
            acc.effects |= EffectSet::READ;
        }
        _ => acc.doubt(ReasonCode::ParseError, whole),
    }
}

/// Statements in a view by our lexer: segments between top-level `;` holding a token.
fn count_statements(t: &Toks<'_>) -> usize {
    let mut count = 0;
    let mut open = false;
    for j in 0..t.len() {
        if t.kind(j) == Some(TokenKind::Semicolon) {
            open = false;
        } else if !open {
            open = true;
            count += 1;
        }
    }
    count
}

fn sql_dialect_for(d: DialectId) -> &'static dyn Dialect {
    static MYSQL: MySqlDialect = MySqlDialect {};
    static SQLITE: SQLiteDialect = SQLiteDialect {};
    match d {
        DialectId::Sqlite => &SQLITE,
        _ => &MYSQL,
    }
}

/// Strict parse of a view's tokens: no trailing commas, `;` between statements, recursion
/// limit 50.
fn parse(tokens: Vec<TokenWithSpan>, dialect: &dyn Dialect) -> Result<Vec<Statement>, ParserError> {
    let options = ParserOptions::new()
        .with_trailing_commas(false)
        .with_unescape(false);
    Parser::new(dialect)
        .with_recursion_limit(RECURSION_LIMIT)
        .with_options(options)
        .with_tokens_with_locations(tokens)
        .parse_statements()
}

// ---------------------------------------------------------------------------------------------
// Token agreement

const UNSET: u8 = 0;
const TRIVIA: u8 = 1;
const CODE: u8 = 2;
const QUOTED: u8 = 3;

/// Marks `classes[a..b]` for one token: trivia, code, or quoted from its first quote byte on
/// (a prefix such as `x`, `N`, `_utf8mb4` or `@` stays code). Returns the quoted extent.
fn mark(classes: &mut [u8], text: &str, a: usize, b: usize, class: u8) -> Option<(usize, usize)> {
    let b = b.min(classes.len());
    if a >= b {
        return None;
    }
    if class != QUOTED {
        classes[a..b].fill(class);
        return None;
    }
    let q = text[a..b]
        .bytes()
        .position(|c| matches!(c, b'\'' | b'"' | b'`' | b'[' | b'$'))
        .map(|p| a + p);
    match q {
        Some(q) => {
            classes[a..q].fill(CODE);
            classes[q..b].fill(QUOTED);
            Some((q, b))
        }
        None => {
            classes[a..b].fill(CODE);
            None
        }
    }
}

fn our_class(k: TokenKind, text: &str) -> u8 {
    match k {
        TokenKind::Whitespace
        | TokenKind::Newline
        | TokenKind::LineComment
        | TokenKind::BlockComment
        | TokenKind::HintComment => TRIVIA,
        TokenKind::String
        | TokenKind::BitString
        | TokenKind::HexString
        | TokenKind::QuotedIdent
        | TokenKind::UnicodeQuotedIdent
        | TokenKind::DollarString => QUOTED,
        TokenKind::UserVar | TokenKind::SystemVar
            if text.bytes().any(|c| matches!(c, b'\'' | b'"' | b'`')) =>
        {
            QUOTED
        }
        _ => CODE,
    }
}

fn their_class(t: &Token) -> u8 {
    match t {
        Token::Whitespace(_) => TRIVIA,
        Token::Word(w) if w.quote_style.is_some() => QUOTED,
        Token::SingleQuotedString(_)
        | Token::DoubleQuotedString(_)
        | Token::TripleSingleQuotedString(_)
        | Token::TripleDoubleQuotedString(_)
        | Token::DollarQuotedString(_)
        | Token::SingleQuotedByteStringLiteral(_)
        | Token::DoubleQuotedByteStringLiteral(_)
        | Token::TripleSingleQuotedByteStringLiteral(_)
        | Token::TripleDoubleQuotedByteStringLiteral(_)
        | Token::SingleQuotedRawStringLiteral(_)
        | Token::DoubleQuotedRawStringLiteral(_)
        | Token::TripleSingleQuotedRawStringLiteral(_)
        | Token::TripleDoubleQuotedRawStringLiteral(_)
        | Token::NationalStringLiteral(_)
        | Token::QuoteDelimitedStringLiteral(_)
        | Token::NationalQuoteDelimitedStringLiteral(_)
        | Token::EscapedStringLiteral(_)
        | Token::UnicodeStringLiteral(_)
        | Token::HexStringLiteral(_) => QUOTED,
        _ => CODE,
    }
}

/// Whether our lexer and `sqlparser`'s tokenizer read `view` alike: the same class (trivia,
/// code, quoted) for every byte and the same extent for every quoted token. Code tokens may
/// split differently (`@v` vs `@` `v`): the parse reads its own; what may not differ is where a
/// string, identifier or comment starts and ends (the differential that hides code, `alg/13`
/// M3/M15).
fn agreement(view: &str, lexed: &Lexed, tokens: &[TokenWithSpan]) -> Result<(), ReasonCode> {
    let n = view.len();
    // `sqlparser` counts only `\n` as a line break; `LineIndex` also counts a lone `\r`.
    let b = view.as_bytes();
    if (0..n).any(|i| b[i] == b'\r' && b.get(i + 1) != Some(&b'\n')) {
        return Err(ReasonCode::ParserDisagreement);
    }
    let mut ours = vec![UNSET; n];
    let mut ours_q = Vec::new();
    for i in 0..lexed.len() {
        let r = lexed.range(i);
        let (a, e) = (usize::from(r.start()), usize::from(r.end()));
        let class = our_class(lexed.kind(i), &view[a..e]);
        ours_q.extend(mark(&mut ours, view, a, e, class));
    }
    let index = LineIndex::new(view);
    let mut map = PosMap::new(view, &index);
    let mut theirs = vec![UNSET; n];
    let mut theirs_q = Vec::new();
    for tok in tokens {
        if tok.token == Token::EOF {
            continue;
        }
        let a = map
            .at(tok.span.start)
            .ok_or(ReasonCode::ParserDisagreement)?;
        let e = map.at(tok.span.end).ok_or(ReasonCode::ParserDisagreement)?;
        if e < a {
            return Err(ReasonCode::ParserDisagreement);
        }
        // `0x0A` is a hex literal without a quote: `mark` makes it code, as ours says.
        theirs_q.extend(mark(&mut theirs, view, a, e, their_class(&tok.token)));
    }
    if ours == theirs && ours_q == theirs_q {
        Ok(())
    } else {
        Err(ReasonCode::ParserDisagreement)
    }
}

/// Maps `sqlparser` locations (1-based line, 1-based character column) to byte offsets,
/// through the view's [`LineIndex`] for line starts. Linear over a forward sequence.
struct PosMap<'a> {
    text: &'a str,
    index: &'a LineIndex,
    line: u64,
    col: u64,
    byte: usize,
}

impl<'a> PosMap<'a> {
    fn new(text: &'a str, index: &'a LineIndex) -> Self {
        PosMap {
            text,
            index,
            line: 1,
            col: 1,
            byte: 0,
        }
    }

    fn at(&mut self, loc: Location) -> Option<usize> {
        if loc.line == 0 || loc.column == 0 {
            return None;
        }
        if loc.line != self.line || loc.column < self.col {
            let line = u32::try_from(loc.line - 1).ok()?;
            let start = self.index.offset(LineCol { line, col: 0 })?;
            self.line = loc.line;
            self.col = 1;
            self.byte = usize::from(start);
        }
        while self.col < loc.column {
            let c = self.text.get(self.byte..)?.chars().next()?;
            if c == '\n' {
                return None;
            }
            self.byte += c.len_utf8();
            self.col += 1;
        }
        Some(self.byte)
    }
}

// ---------------------------------------------------------------------------------------------
// The statement tables

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct TableFile {
    version: u32,
    rows: Vec<RowFile>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RowFile {
    variant: String,
    kind: String,
    effects: Vec<String>,
    #[serde(default)]
    reasons: Vec<String>,
    #[serde(default)]
    flags: Vec<String>,
}

/// One statement table row, resolved.
#[derive(Debug, Clone)]
pub(crate) struct Row {
    pub(crate) kind: StmtKind,
    pub(crate) effects: EffectSet,
    pub(crate) reasons: Vec<ReasonCode>,
    pub(crate) flags: RiskFlags,
}

pub(crate) type Table = Vec<(String, Row)>;

const MYSQL_TABLE: &str = include_str!("../../data/mysql/statements.toml");
const SQLITE_TABLE: &str = include_str!("../../data/sqlite/statements.toml");

fn kind_named(name: &str) -> Option<StmtKind> {
    use StmtKind as K;
    Some(match name {
        "Select" => K::Select,
        "Values" => K::Values,
        "Table" => K::Table,
        "Insert" => K::Insert,
        "Update" => K::Update,
        "Delete" => K::Delete,
        "Merge" => K::Merge,
        "Copy" => K::Copy,
        "Call" => K::Call,
        "Do" => K::Do,
        "Explain" => K::Explain { analyze: false },
        "CreateTable" => K::CreateTable,
        "CreateOther" => K::CreateOther,
        "Alter" => K::Alter,
        "Drop" => K::Drop,
        "Truncate" => K::Truncate,
        "Grant" => K::Grant,
        "Revoke" => K::Revoke,
        "Set" => K::Set,
        "Show" => K::Show,
        "Begin" => K::Begin,
        "Commit" => K::Commit,
        "Rollback" => K::Rollback,
        "Savepoint" => K::Savepoint,
        "Lock" => K::Lock,
        "Prepare" => K::Prepare,
        "Execute" => K::Execute,
        "Listen" => K::Listen,
        "Notify" => K::Notify,
        "Use" => K::Use,
        "Pragma" => K::Pragma,
        "Other" => K::Other,
        _ => return None,
    })
}

fn code_named(name: &str) -> Option<ReasonCode> {
    ReasonCode::ALL
        .into_iter()
        .find(|c| format!("{c:?}") == name)
}

/// Parses one statement table; an error names the first bad entry.
pub(crate) fn load_table(src: &str) -> Result<Table, String> {
    let file: TableFile = toml::from_str(src).map_err(|e| e.to_string())?;
    if file.version != 1 {
        return Err(format!("version {}", file.version));
    }
    let mut out = Vec::with_capacity(file.rows.len());
    for r in file.rows {
        let kind = kind_named(&r.kind).ok_or_else(|| format!("kind {}", r.kind))?;
        let mut effects = EffectSet::empty();
        for e in &r.effects {
            effects |= EffectSet::from_name(e).ok_or_else(|| format!("effect {e}"))?;
        }
        if effects.is_empty() {
            return Err(format!("{}: no effect", r.variant));
        }
        let mut flags = RiskFlags::empty();
        for f in &r.flags {
            flags |= RiskFlags::from_name(f).ok_or_else(|| format!("flag {f}"))?;
        }
        let reasons = r
            .reasons
            .iter()
            .map(|c| code_named(c).ok_or_else(|| format!("reason {c}")))
            .collect::<Result<Vec<_>, _>>()?;
        if out.iter().any(|(v, _): &(String, Row)| *v == r.variant) {
            return Err(format!("{} twice", r.variant));
        }
        out.push((
            r.variant,
            Row {
                kind,
                effects,
                reasons,
                flags,
            },
        ));
    }
    Ok(out)
}

/// The statement table for `d` (MySQL's serves MariaDB); `Err` when it does not load (every
/// statement is then `UNKNOWN`).
pub(crate) fn table_for(d: DialectId) -> Result<&'static Table, &'static str> {
    static MYSQL: OnceLock<Result<Table, String>> = OnceLock::new();
    static SQLITE: OnceLock<Result<Table, String>> = OnceLock::new();
    let cell = match d {
        DialectId::Sqlite => SQLITE.get_or_init(|| load_table(SQLITE_TABLE)),
        _ => MYSQL.get_or_init(|| load_table(MYSQL_TABLE)),
    };
    cell.as_ref().map_err(String::as_str)
}

/// The variant's name as `Debug` prints it (`Query`, `ShowVariable`, …), without formatting the
/// rest of the value.
fn variant_name<T: fmt::Debug>(v: &T) -> String {
    struct Head(String, bool);
    impl fmt::Write for Head {
        fn write_str(&mut self, s: &str) -> fmt::Result {
            for c in s.chars() {
                if c.is_ascii_alphanumeric() || c == '_' {
                    self.0.push(c);
                } else {
                    self.1 = true;
                    return Err(fmt::Error);
                }
            }
            Ok(())
        }
    }
    let mut h = Head(String::new(), false);
    let _ = write!(h, "{v:?}");
    h.0
}

// ---------------------------------------------------------------------------------------------
// The walk

struct Walk<'c, 'a> {
    ctx: &'c ClassifyCtx,
    range: Option<TextRange>,
    acc: &'a mut Acc,
    top: Option<StmtKind>,
    /// Inside a plain `EXPLAIN` (its inner statement does not run: no destructive pattern).
    plain_explain: u32,
}

impl<'c, 'a> Walk<'c, 'a> {
    fn new(ctx: &'c ClassifyCtx, range: Option<TextRange>, acc: &'a mut Acc) -> Self {
        Walk {
            ctx,
            range,
            acc,
            top: None,
            plain_explain: 0,
        }
    }

    fn destructive(&mut self, kind: DestructiveKind, target: Option<&ObjectName>) {
        if self.plain_explain > 0 {
            return;
        }
        let Some(range) = self.range else {
            return;
        };
        let target = target.and_then(qualified).map(|name| ObjectRef {
            name,
            kind: ObjectKind::Table,
        });
        self.acc.destructive.push(DestructiveReason {
            kind,
            range,
            target,
        });
    }

    fn write_target(&mut self, name: &ObjectName) {
        if let Some(name) = qualified(name) {
            self.acc.touched.writes.push(ObjectRef {
                name,
                kind: ObjectKind::Table,
            });
        }
    }

    fn call(&mut self, name: &ObjectName, arity: Option<u16>) {
        match qualified(name) {
            Some(q) => prescan::assess_call(&q, self.range, arity, self.ctx, false, self.acc),
            // A computed name (`IDENTIFIER(…)`): nothing is known.
            None => self.acc.doubt(ReasonCode::FnUnknown, self.range),
        }
    }

    fn statement(&mut self, s: &Statement) {
        let name = variant_name(s);
        let row = match table_for(self.ctx.dialect.id) {
            Ok(t) => t.iter().find(|(v, _)| *v == name).map(|(_, r)| r.clone()),
            Err(_) => None,
        };
        let Some(row) = row else {
            if self.top.is_none() {
                self.top = Some(StmtKind::Other);
                self.acc.kind = self.acc.kind.or(Some(StmtKind::Other));
            }
            self.acc.doubt(ReasonCode::EffectsUnknown, self.range);
            return;
        };
        let mut kind = row.kind;
        let mut effects = row.effects;
        let nested = self.top.is_some();
        match s {
            Statement::Insert(i) => {
                if let TableObject::TableName(n) = &i.table {
                    self.write_target(n);
                }
            }
            Statement::Update(u) => {
                self.unbounded(u.selection.as_ref(), &u.limit, u.order_by.is_empty(), None);
                self.joins_without_on(std::slice::from_ref(&u.table));
                if let TableFactor::Table { name, .. } = &u.table.relation {
                    self.write_target(name);
                }
            }
            Statement::Delete(d) => {
                let from = match &d.from {
                    sqlparser::ast::FromTable::WithFromKeyword(v)
                    | sqlparser::ast::FromTable::WithoutKeyword(v) => v.as_slice(),
                };
                let target = d.tables.first().cloned().or_else(|| match from.first() {
                    Some(TableWithJoins {
                        relation: TableFactor::Table { name, .. },
                        ..
                    }) => Some(name.clone()),
                    _ => None,
                });
                self.unbounded(
                    d.selection.as_ref(),
                    &d.limit,
                    d.order_by.is_empty(),
                    target.as_ref(),
                );
                self.joins_without_on(from);
                if let Some(t) = &target {
                    self.write_target(t);
                }
            }
            Statement::Drop {
                object_type,
                cascade,
                names,
                ..
            } => {
                if matches!(variant_name(object_type).as_str(), "User" | "Role") {
                    effects |= EffectSet::PRIVILEGE;
                }
                for n in names {
                    self.destructive(DestructiveKind::Drop { cascade: *cascade }, Some(n));
                    self.write_target(n);
                }
            }
            Statement::DropFunction(_)
            | Statement::DropProcedure { .. }
            | Statement::DropTrigger(_) => {
                self.destructive(DestructiveKind::Drop { cascade: false }, None);
            }
            Statement::Truncate(t) => {
                for n in &t.table_names {
                    self.destructive(DestructiveKind::Truncate, Some(&n.name));
                    self.write_target(&n.name);
                }
            }
            Statement::AlterTable(a) => {
                for op in &a.operations {
                    let k = match variant_name(op).as_str() {
                        "DropColumn" => Some(DestructiveKind::DropColumn),
                        "DropConstraint" | "DropPrimaryKey" | "DropForeignKey" | "DropIndex" => {
                            Some(DestructiveKind::DropConstraint)
                        }
                        "ChangeColumn" | "ModifyColumn" => Some(DestructiveKind::AlterType),
                        "AlterColumn" if format!("{op:?}").contains("SetDataType") => {
                            Some(DestructiveKind::AlterType)
                        }
                        "DropPartitions" => Some(DestructiveKind::Drop { cascade: false }),
                        _ => None,
                    };
                    if let Some(k) = k {
                        self.destructive(k, Some(&a.name));
                    }
                }
                self.write_target(&a.name);
            }
            Statement::CreateTable(ct) => {
                if ct.temporary {
                    effects = EffectSet::DDL_TEMP | EffectSet::WRITE_TEMP;
                }
                if ct.query.is_some() {
                    effects |= if ct.temporary {
                        EffectSet::WRITE_TEMP
                    } else {
                        EffectSet::WRITE_DATA
                    };
                }
            }
            Statement::Explain {
                analyze, options, ..
            } => {
                let analyze = *analyze
                    || options.iter().flatten().any(|o| {
                        o.name.value.eq_ignore_ascii_case("ANALYZE")
                            || o.name.value.eq_ignore_ascii_case("ANALYSE")
                    });
                kind = StmtKind::Explain { analyze };
                if analyze {
                    self.acc.flags |= RiskFlags::EXPLAIN_ANALYZE;
                    self.acc
                        .reason(ReasonCode::ExplainAnalyze, self.range, None);
                } else {
                    self.plain_explain += 1;
                }
            }
            Statement::Call(f) => self.call(&f.name, None),
            _ => {}
        }
        if nested
            && matches!(
                kind,
                StmtKind::Insert | StmtKind::Update | StmtKind::Delete | StmtKind::Merge
            )
            && self.top == Some(StmtKind::Select)
        {
            self.acc
                .reason(ReasonCode::DataModifyingCte, self.range, None);
        }
        if self.top.is_none() {
            self.top = Some(kind);
            if self.acc.kind.is_none() || self.acc.kind == Some(StmtKind::Set) {
                self.acc.kind = Some(kind);
            }
        }
        self.acc.effects |= effects;
        self.acc.flags |= row.flags;
        for code in row.reasons {
            if effects.contains(EffectSet::UNKNOWN) {
                self.acc.doubt(code, self.range);
            } else {
                self.acc.reason(code, self.range, None);
            }
        }
    }

    /// `UPDATE`/`DELETE` without a bounding `WHERE` (SAFE-07).
    fn unbounded(
        &mut self,
        selection: Option<&Expr>,
        limit: &Option<Expr>,
        no_order: bool,
        target: Option<&ObjectName>,
    ) {
        match selection {
            None => self.destructive(DestructiveKind::NoWhere, target),
            Some(e) if always_true(e) => self.destructive(DestructiveKind::AlwaysTrue, target),
            Some(_) => {}
        }
        if limit.is_some() && no_order {
            self.destructive(DestructiveKind::UpdateLimitNoOrder, target);
        }
    }

    /// A join without a condition in an `UPDATE`/`DELETE` target list (SAFE-07).
    fn joins_without_on(&mut self, tables: &[TableWithJoins]) {
        for t in tables {
            for j in &t.joins {
                let op = format!("{:?}", j.join_operator);
                if op.starts_with("CrossJoin") || op.ends_with("(None)") {
                    self.destructive(DestructiveKind::JoinWithoutOn, None);
                }
            }
        }
    }
}

impl Visitor for Walk<'_, '_> {
    type Break = ();

    fn pre_visit_statement(&mut self, s: &Statement) -> ControlFlow<()> {
        self.statement(s);
        ControlFlow::Continue(())
    }

    fn post_visit_statement(&mut self, s: &Statement) -> ControlFlow<()> {
        if let Statement::Explain {
            analyze, options, ..
        } = s
        {
            let analyze = *analyze
                || options.iter().flatten().any(|o| {
                    o.name.value.eq_ignore_ascii_case("ANALYZE")
                        || o.name.value.eq_ignore_ascii_case("ANALYSE")
                });
            if !analyze {
                self.plain_explain = self.plain_explain.saturating_sub(1);
            }
        }
        ControlFlow::Continue(())
    }

    fn pre_visit_query(&mut self, q: &Query) -> ControlFlow<()> {
        if !q.locks.is_empty() {
            self.acc.effects |= EffectSet::LOCK_ROW;
            self.acc.reason(ReasonCode::LockingClause, self.range, None);
        }
        ControlFlow::Continue(())
    }

    fn pre_visit_select(&mut self, s: &Select) -> ControlFlow<()> {
        if let Some(into) = &s.into {
            let vars = !into.targets.is_empty()
                && into.targets.iter().all(|t| match t {
                    Expr::Identifier(i) => i.value.starts_with('@'),
                    _ => false,
                });
            if vars {
                self.acc.effects |= EffectSet::SESSION_STATE;
                self.acc.reason(ReasonCode::SessionState, self.range, None);
            } else {
                self.acc.effects |= EffectSet::DDL | EffectSet::WRITE_DATA;
                self.acc.reason(ReasonCode::SchemaChange, self.range, None);
            }
        }
        ControlFlow::Continue(())
    }

    fn pre_visit_expr(&mut self, e: &Expr) -> ControlFlow<()> {
        if let Expr::Function(f) = e {
            let arity = match &f.args {
                FunctionArguments::List(l) => u16::try_from(l.args.len()).ok(),
                FunctionArguments::None => Some(0),
                FunctionArguments::Subquery(_) => None,
            };
            self.call(&f.name, arity);
        }
        ControlFlow::Continue(())
    }

    fn pre_visit_table_factor(&mut self, t: &TableFactor) -> ControlFlow<()> {
        match t {
            TableFactor::Table {
                name,
                args: Some(args),
                ..
            } => self.call(name, u16::try_from(args.args.len()).ok()),
            TableFactor::Function { name, args, .. } => {
                self.call(name, u16::try_from(args.len()).ok())
            }
            _ => {}
        }
        ControlFlow::Continue(())
    }

    fn pre_visit_relation(&mut self, n: &ObjectName) -> ControlFlow<()> {
        if let Some(name) = qualified(n) {
            self.acc.touched.reads.push(ObjectRef {
                name,
                kind: ObjectKind::Table,
            });
        }
        ControlFlow::Continue(())
    }
}

/// `n` as a [`QualifiedName`]; `None` for a computed part.
fn qualified(n: &ObjectName) -> Option<QualifiedName> {
    let mut parts: SmallVec<[Ident; 3]> = SmallVec::new();
    for p in &n.0 {
        match p {
            ObjectNamePart::Identifier(i) => parts.push(Ident {
                text: Arc::from(i.value.as_str()),
                quoted: i.quote_style.is_some(),
            }),
            ObjectNamePart::Function(_) => return None,
        }
    }
    (!parts.is_empty()).then_some(QualifiedName { parts })
}

/// A `WHERE` that holds for every row: `TRUE`, a non-zero number, `x = x` for one literal, or
/// an `OR` with such a side.
fn always_true(e: &Expr) -> bool {
    use sqlparser::ast::BinaryOperator as B;
    match e {
        Expr::Nested(inner) => always_true(inner),
        Expr::Value(v) => match &v.value {
            Value::Boolean(b) => *b,
            Value::Number(n, _) => n.parse::<f64>().is_ok_and(|x| x != 0.0),
            _ => false,
        },
        Expr::BinaryOp { left, op, right } => match op {
            B::Or => always_true(left) || always_true(right),
            B::Eq => matches!(**left, Expr::Value(_)) && left == right,
            _ => false,
        },
        _ => false,
    }
}

// ---------------------------------------------------------------------------------------------
// The statement's report

fn finalize(range: TextRange, text: &str, base: Acc, outs: Vec<Acc>) -> StmtReport {
    let shift = range.start();
    let mut views: Vec<Acc> = outs;
    let any = views.iter().any(|v| {
        v.kind.is_some() || v.kind_hint.is_some() || !v.effects.is_empty() || !v.reasons.is_empty()
    });
    // The kind of the most critical reading.
    let mut best: Option<(crate::gate_class::GateClass, StmtKind)> = None;
    for v in &views {
        // A reading with no statement (an inactive comment alone) names no kind.
        if v.kind.is_none() && v.kind_hint.is_none() && v.effects.is_empty() && v.reasons.is_empty()
        {
            continue;
        }
        let kind = v.kind.or(v.kind_hint).unwrap_or(StmtKind::Other);
        let probe = StmtReport {
            range,
            kind,
            effects: v.effects,
            flags: v.flags,
            class: class_of(v.effects),
            touched: Default::default(),
            reasons: v.reasons.clone(),
            destructive: v.destructive.clone(),
            confidence: Confidence::Exact,
            set_targets: Vec::new(),
        };
        let g = gate_class(&probe);
        if best.is_none_or(|(bg, _)| g > bg) {
            best = Some((g, kind));
        }
    }
    let mut acc = base;
    let kind = best.map(|(_, k)| k).unwrap_or(StmtKind::Other);
    for v in views.drain(..) {
        acc.absorb(v);
    }
    let whole = Some(range_of(0, text.len()));
    if !any && !acc.effects.contains(EffectSet::UNKNOWN) {
        acc.doubt(ReasonCode::NoStatement, whole);
    }
    if acc.effects.is_empty() {
        acc.doubt(ReasonCode::EffectsUnknown, whole);
    }
    let doubt = acc.reasons.iter().any(|r| {
        matches!(
            reason_rule(r.code),
            ReasonRule::DenyAlways | ReasonRule::Group(Group::Unknown)
        )
    });
    if doubt {
        acc.effects |= EffectSet::UNKNOWN;
    }
    let mut class = class_of(acc.effects);
    if doubt || acc.effects.contains(EffectSet::UNKNOWN) {
        class = CoarseClass::Unknown;
    }
    let move_range = |r: TextRange| TextRange::new(r.start() + shift, r.end() + shift);
    let mut reasons: Vec<Reason> = Vec::new();
    for mut r in acc.reasons {
        r.range = r.range.map(move_range);
        // One reason per code, argument and range; the pre-scan's call reason (at the name)
        // and the walk's (at the statement) for the same name are one.
        let nested = |a: Option<TextRange>, b: Option<TextRange>| match (a, b) {
            (Some(a), Some(b)) => a.contains_range(b) || b.contains_range(a),
            (a, b) => a == b,
        };
        let dup = reasons.iter().any(|x| {
            x.code == r.code
                && x.arg == r.arg
                && (x.range == r.range || (r.arg.is_some() && nested(x.range, r.range)))
        });
        if !dup {
            reasons.push(r);
        }
    }
    let mut destructive: Vec<DestructiveReason> = Vec::new();
    for mut d in acc.destructive {
        d.range = move_range(d.range);
        if !destructive.contains(&d) {
            destructive.push(d);
        }
    }
    // One target per name range (each reading names the same bytes); readings that disagree on
    // the value leave it unknown (`None`, never more permissive). Each target has exactly one
    // `SessionState` reason at its range, so the two lists match (the T4 contract).
    let mut by_range: Vec<(TextRange, crate::report::SetTarget)> = Vec::new();
    for (t, r) in acc.set_targets {
        match by_range.iter_mut().find(|(x, _)| *x == r) {
            Some((_, seen)) if seen.value != t.value => seen.value = None,
            Some(_) => {}
            None => by_range.push((r, t)),
        }
    }
    let set_targets: Vec<crate::report::SetTarget> = by_range.into_iter().map(|(_, t)| t).collect();
    let mut touched = acc.touched;
    for list in [&mut touched.reads, &mut touched.writes, &mut touched.calls] {
        let mut seen = HashSet::new();
        list.retain(|o| seen.insert(o.clone()));
    }
    StmtReport {
        range,
        kind,
        effects: acc.effects,
        flags: acc.flags,
        class,
        touched,
        reasons,
        destructive,
        confidence: if acc.conservative {
            Confidence::Conservative
        } else {
            Confidence::Exact
        },
        set_targets,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn variant_names_stop_at_the_first_field() {
        let s = Parser::parse_sql(&MySqlDialect {}, "SHOW TABLES").unwrap();
        assert_eq!(variant_name(&s[0]), "ShowTables");
        let s = Parser::parse_sql(&MySqlDialect {}, "SELECT 1").unwrap();
        assert_eq!(variant_name(&s[0]), "Query");
    }

    #[test]
    fn both_tables_load() {
        assert!(table_for(DialectId::MySql).is_ok());
        assert!(table_for(DialectId::Sqlite).is_ok());
        assert!(
            load_table(
                "version = 1\nrows = [{ variant = \"X\", kind = \"Nope\", effects = [\"READ\"] }]"
            )
            .is_err()
        );
        assert!(
            load_table("version = 1\nrows = [{ variant = \"X\", kind = \"Other\", effects = [] }]")
                .is_err()
        );
        assert!(load_table("version = 1\nrows = [{ variant = \"X\", kind = \"Other\", effects = [\"READ\"], reasons = [\"Nope\"] }]").is_err());
    }

    #[test]
    fn positions_map_by_character_column() {
        let text = "SELECT 'ş'\n  , ü";
        let index = LineIndex::new(text);
        let mut m = PosMap::new(text, &index);
        assert_eq!(m.at(Location { line: 1, column: 8 }), Some(7));
        assert_eq!(
            m.at(Location {
                line: 1,
                column: 11
            }),
            Some(11)
        );
        assert_eq!(m.at(Location { line: 2, column: 5 }), Some(16));
        assert_eq!(m.at(Location { line: 2, column: 6 }), Some(18));
    }
}
