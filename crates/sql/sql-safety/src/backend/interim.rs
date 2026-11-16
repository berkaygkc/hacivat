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
use sql_ident::{Ident, QualifiedName, ident_eq};
use sql_lexer::{LexErrorCode, LexState, Lexed, TokenKind, exec_comment, lex};
use sql_split::SplitMode;
use sql_text::{LineCol, LineIndex, TextRange};
use sqlparser::ast::{
    Expr, FunctionArguments, ObjectName, ObjectNamePart, Query, Select, SetExpr, Statement,
    TableFactor, TableObject, TableWithJoins, Value, Visit, Visitor,
};
use sqlparser::dialect::{Dialect, MySqlDialect, SQLiteDialect};
use sqlparser::parser::{Parser, ParserError, ParserOptions};
use sqlparser::tokenizer::{Location, Token, TokenWithSpan, Tokenizer};

use super::Backend;
use super::prescan::{self, Acc, Describe, Toks, range_of};
use crate::classify::{ClassifyCtx, Expect};
use crate::destructive::{CmpOp, DestructiveKind, DestructiveReason, Predicate};
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

// ## m3b-t14-orchestrator (named exception, T10 C1 carry): a statement that is the whole text
// (one unit, the exec path) reads the orchestrator's lexing instead of lexing again.
impl Backend for InterimBackend {
    fn analyse(
        &self,
        sql: &str,
        items: &[TextRange],
        ctx: &ClassifyCtx,
        lexed: Option<&Lexed>,
    ) -> Vec<StmtReport> {
        let whole =
            |r: &TextRange| usize::from(r.start()) == 0 && usize::from(r.end()) == sql.len();
        // ## m3b-t20-differential (named exception): one reading budget for the request.
        let budget = std::cell::Cell::new(MAX_READING_BYTES);
        items
            .iter()
            .map(|r| analyse_stmt(sql, *r, ctx, lexed.filter(|_| whole(r)), &budget))
            .collect()
    }
}

// ---------------------------------------------------------------------------------------------
// The Task 10 harness

/// Classifies `sql` with the interim backend alone: the splitter's statements (execution mode)
/// each analysed, an uncertain boundary or more statements than `ctx.expect` allows marked
/// `Unknown`, merged as `classify` merges. Task 10's tests and corpus run through it; Task 14's
/// [`crate::classify`] is the production entry and supersedes it (it adds the limits, the
/// templates and the per-dialect plan).
// ## m3b-t14-orchestrator (named exception, fix round 1, I2): the harness runs on a classifier
// stack too (it has no L0 limits, so its trees can be deeper than `classify`'s).
pub fn classify_interim(sql: &str, ctx: &ClassifyCtx) -> SafetyReport {
    crate::classify::on_classifier_stack(
        || classify_interim_here(sql, ctx),
        || crate::classify::unknown_report(sql, ctx, ReasonCode::EffectsUnknown),
    )
}

fn classify_interim_here(sql: &str, ctx: &ClassifyCtx) -> SafetyReport {
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
    let mut items = InterimBackend.analyse(sql, &ranges, ctx, None);
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

/// One executable comment of a statement and every way a server may read it.
///
/// Each [`Reading`] is how a server that *runs* the comment reads it; a reading with a version
/// is skipped (the whole token is a comment) by a server older than that version, and the
/// threshold views include that skipped reading too (fix round 1, C1: the union of readings
/// covers every way the server may read the text).
struct ExecComment {
    /// The whole token (the skipping reading's extent).
    token: (usize, usize),
    /// The ways a server that runs it reads it (at least one).
    readings: Vec<Reading>,
    /// The comment's end depends on how its body reads (`AmbiguousCommentEnd`).
    ambiguous: bool,
}

/// One running reading of an executable comment.
#[derive(Clone, Copy)]
struct Reading {
    /// The opener with the version digits this reading takes (`/*!50000`).
    opener: (usize, usize),
    /// The `*/` this reading ends at (`None`: it runs to the end of the statement).
    close: Option<(usize, usize)>,
    /// The server version from which the body runs; `None`: every server runs it.
    version: Option<u32>,
}

/// The version a MySQL server never reaches: a MariaDB `/*M!` comment without digits, which
/// MySQL skips and a MariaDB server behind a MySQL profile runs.
const NEVER_ON_MYSQL: u32 = u32::MAX;

// ## m3b-t14-orchestrator (named exception, T10 C1 carry): `pre`, when given, is `text`'s
// lexing (the statement is the whole text the orchestrator lexed).
fn analyse_stmt(
    sql: &str,
    range: TextRange,
    ctx: &ClassifyCtx,
    pre: Option<&Lexed>,
    budget: &std::cell::Cell<usize>,
) -> StmtReport {
    let start = usize::from(range.start());
    let end = usize::from(range.end()).min(sql.len());
    let text = sql.get(start..end).unwrap_or("");
    let mut base = Acc::default();
    let views = statement_views(text, ctx, &mut base, pre, budget);
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
                .map(|v| analyse_view(v, ctx, 0, same.then_some(&*lexed)))
                .collect()
        }
        None => Vec::new(),
    };
    finalize(range, text, base, outs)
}

// ## m3b-t14-orchestrator (named exception, T10 C1 carry)
/// A lexing this module made, or one the orchestrator lent.
#[allow(
    clippy::large_enum_variant,
    reason = "one per statement analysis, never stored: boxing would add an allocation"
)]
enum Held<'a> {
    Own(Lexed),
    Lent(&'a Lexed),
}

impl std::ops::Deref for Held<'_> {
    type Target = Lexed;

    fn deref(&self) -> &Lexed {
        match self {
            Held::Own(l) => l,
            Held::Lent(l) => l,
        }
    }
}

/// The readings of `text` and its lexing (`pre` when given), or `None` when the statement is
/// decided by `base` already (a lexing error, an unsupported mode, too many readings).
fn statement_views<'a>(
    text: &str,
    ctx: &ClassifyCtx,
    base: &mut Acc,
    pre: Option<&'a Lexed>,
    budget: &std::cell::Cell<usize>,
) -> Option<(Vec<String>, Held<'a>)> {
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
    let lexed = match pre {
        Some(l) => Held::Lent(l),
        None => Held::Own(lex(text, profile, ctx.dialect.modes, &LexState::Initial)),
    };
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
        if c.ambiguous {
            base.reason(
                ReasonCode::AmbiguousBoundary,
                Some(range_of(c.token.0, c.token.1)),
                None,
            );
            base.effects |= EffectSet::UNKNOWN;
        }
    }
    build_views(text, &comments, ctx, base, budget).map(|v| (v, lexed))
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
        let opener = (usize::from(c.opener.start()), usize::from(c.opener.end()));
        let body_end = usize::from(c.body.end());
        let close = (body_end < t1).then_some((body_end, t1));
        // The version readings: the opener as written; on MySQL a six-digit gate is also read
        // as five digits and a body digit (`exec_comment_gate`), and both are kept (fix round
        // 1, M7: every distinct threshold is read).
        let mut openers = vec![(opener, c.version)];
        if ctx.id == DialectId::MySql && opener.1 - opener.0 == 3 + 6 {
            openers.push(((opener.0, opener.1 - 1), c.version.map(|v| v / 10)));
        }
        let is_ambiguous = ambiguous.contains(&token);
        let mut readings = Vec::new();
        for (opener, version) in openers {
            readings.push(Reading {
                opener,
                close,
                version,
            });
            if is_ambiguous {
                let mariadb = ctx.id == DialectId::MariaDb;
                let running =
                    prescan::running_close(text, opener.1, text.len(), ctx.modes, mariadb)
                        .map(|e| (e - 2, e));
                readings.push(Reading {
                    opener,
                    close: running,
                    version,
                });
            }
        }
        out.push(ExecComment {
            token: (t0, t1),
            readings,
            ambiguous: is_ambiguous,
        });
    }
    out
}

/// A MariaDB `/*M!…*/` comment on a MySQL connection. MySQL skips it (the whole token is a
/// comment); a MariaDB server behind a MySQL profile runs it. Both are read (fix round 1, C1):
/// without version digits its one running reading has the version [`NEVER_ON_MYSQL`], so the
/// threshold views hold the skipped reading too. Our MySQL lexer reads it as an ordinary block
/// comment, which ends at the first `*/`.
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
        (v, 4 + digits)
    } else {
        (NEVER_ON_MYSQL, 4)
    };
    let closed = token.len() >= opener_len + 2 && token.ends_with("*/");
    Some(ExecComment {
        token: (t0, t1),
        readings: vec![Reading {
            opener: (t0, t0 + opener_len),
            close: closed.then_some((t1 - 2, t1)),
            version: Some(version),
        }],
        ambiguous: false,
    })
}

/// How many readings `comments` make: `(distinct versions + 1) × Π readings per comment`,
/// counted with checked arithmetic and stopped as soon as it passes `limit` (`None`: too
/// many). No allocation per reading and no loop over readings (fix round 1, C2).
fn reading_count(thresholds: usize, comments: &[ExecComment], limit: usize) -> Option<usize> {
    let mut n = thresholds.checked_add(1)?;
    if n > limit {
        return None;
    }
    for c in comments {
        n = n.checked_mul(c.readings.len().max(1))?;
        if n > limit {
            return None;
        }
    }
    Some(n)
}

// ## m3b-t20-differential (named exception, T10 review: the readings × length perf cap)
/// The bytes a request's statements with more than one threshold reading may hand `sqlparser`
/// together: the sum, over those statements, of their readings × their length. Past it the
/// statement is `TooComplex` before any reading is built. The budget is per call of this
/// backend: the extra work executable comments cause is bounded by that of one 256 KiB
/// statement (≈ 35 ms in release) per lexing-mode reading, and `classify` makes up to four of
/// those when the modes are unknown (fix round 1, m4): up to 1 MiB of readings, not 16
/// readings × 1 MiB. 16 readings of a 16 KiB statement fit; a statement of more than 128 KiB
/// with any executable comment does not; and because the budget is the request's, a long
/// script run as one unit (a schema dump of some 450 tables with `/*!…*/` lines) runs out, and
/// its later commented statements are `TooComplex` (fail-closed; PO list).
pub(crate) const MAX_READING_BYTES: usize = 256 << 10;

/// Every threshold reading of `text` (deduplicated), or `None` past the limit
/// (`Limits.max_exec_comment_views`, checked before any reading is built) or past what is left
/// of the request's [`MAX_READING_BYTES`] (`budget`).
fn build_views(
    text: &str,
    comments: &[ExecComment],
    ctx: &ClassifyCtx,
    base: &mut Acc,
    budget: &std::cell::Cell<usize>,
) -> Option<Vec<String>> {
    let limit = usize::from(ctx.limits.max_exec_comment_views);
    let whole = Some(range_of(0, text.len()));
    // A cheap bound first: more distinct versions than the limit can never fit.
    let mut thresholds: Vec<u32> = Vec::new();
    for c in comments {
        for r in &c.readings {
            if let Some(v) = r.version
                && !thresholds.contains(&v)
            {
                if thresholds.len() >= limit {
                    base.doubt(ReasonCode::TooComplex, whole);
                    return None;
                }
                thresholds.push(v);
            }
        }
    }
    thresholds.sort_unstable();
    let Some(count) = reading_count(thresholds.len(), comments, limit) else {
        base.doubt(ReasonCode::TooComplex, whole);
        return None;
    };
    // ## m3b-t20-differential: readings × length within the request's budget.
    if count > 1 {
        match count
            .checked_mul(text.len())
            .and_then(|cost| budget.get().checked_sub(cost))
        {
            Some(left) => budget.set(left),
            None => {
                base.doubt(ReasonCode::TooComplex, whole);
                return None;
            }
        }
    }
    // The comments with a choice of readings, as a mixed-radix counter.
    let choosers: Vec<usize> = comments
        .iter()
        .enumerate()
        .filter_map(|(k, c)| (c.readings.len() > 1).then_some(k))
        .collect();
    let mut seen = HashSet::new();
    let mut views = Vec::new();
    for t in 0..=thresholds.len() {
        let mut choice = vec![0usize; comments.len()];
        loop {
            let mut bytes = text.as_bytes().to_vec();
            for (k, c) in comments.iter().enumerate() {
                let r = c.readings[choice[k]];
                let active = r.version.is_none_or(|v| {
                    thresholds
                        .iter()
                        .position(|x| *x == v)
                        .is_some_and(|p| p < t)
                });
                if !active {
                    blank(&mut bytes, c.token.0, c.token.1);
                    continue;
                }
                blank(&mut bytes, r.opener.0, r.opener.1);
                if let Some((a, b)) = r.close {
                    blank(&mut bytes, a, b);
                }
            }
            // Blanking whole ASCII bytes inside comment markers keeps the text UTF-8; a
            // multi-byte character inside a blanked inactive body becomes spaces as a whole.
            let Ok(view) = String::from_utf8(bytes) else {
                base.doubt(ReasonCode::LexError, whole);
                return None;
            };
            if seen.insert(view.clone()) {
                views.push(view);
            }
            // The next combination of readings.
            let mut carried = true;
            for &k in &choosers {
                choice[k] += 1;
                if choice[k] < comments[k].readings.len() {
                    carried = false;
                    break;
                }
                choice[k] = 0;
            }
            if carried {
                break;
            }
        }
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
    // ## m3b-t14-orchestrator (named exception, fix round 2, R1-C1): an executable comment's
    // body is one token to the orchestrator's L0; in a reading it is code, so the operator-chain
    // caps run here, on every reading, before `sqlparser` sees it.
    if crate::classify::chains_exceed(view, lexed, d) {
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
    // ## m3b-t26-prod-unlock (named exception, T26 carry MUST 1): MySQL/MariaDB's multi-table
    // `UPDATE a, b SET …` is read as `UPDATE a CROSS JOIN b SET …` (the same table reference to
    // the server), so it gets real targets instead of an unparsed `Unknown`; the `UPDATE` and
    // `DELETE` modifiers are never read as a table (`mysql_dml_as_parsed`).
    let mut comma_update = false;
    let parsed = match tokens {
        Ok(tokens) => {
            let tokens = if d == DialectId::Sqlite {
                tokens
            } else {
                let (tokens, rewrote) = mysql_dml_as_parsed(tokens);
                comma_update = rewrote;
                tokens
            };
            parse(tokens, dialect)
        }
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
            walk.comma_update = comma_update;
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
    // ## m3b-t28-matrix (named exception, T28 review M1 ruling): MariaDB's `ANALYZE <dml>`
    // (a data change) runs its statement (it is MariaDB's EXPLAIN ANALYZE, with no rollback). It is classified
    // as the inner statement itself, so it is gated exactly like the bare DML (never the D-062
    // wrap, never `Unknown`). `ANALYZE SELECT`, `ANALYZE FORMAT=…` and anything else stay
    // `ParseError`.
    if inner_at.is_none()
        && ctx.dialect.id == DialectId::MariaDb
        && depth < MAX_EXPLAIN_DEPTH
        && let Some(at) = mariadb_analyze_inner(t)
    {
        let mut bytes = view.as_bytes().to_vec();
        blank(&mut bytes, 0, at);
        let Ok(inner) = String::from_utf8(bytes) else {
            acc.doubt(ReasonCode::ParseError, whole);
            return;
        };
        let inner = analyse_view(&inner, ctx, depth + 1, None);
        acc.absorb(inner);
        return;
    }
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

// ## m3b-t28-matrix (named exception, T28 review M1 ruling)
/// Where the data change of a MariaDB `ANALYZE <dml>` starts (`ANALYZE INSERT|UPDATE|DELETE|
/// REPLACE …`), `None` for any other `ANALYZE`. `ANALYZE SELECT` stays T10's MariaDB-only form
/// (never a read).
fn mariadb_analyze_inner(t: &Toks<'_>) -> Option<usize> {
    const INNER: [&str; 4] = ["INSERT", "UPDATE", "DELETE", "REPLACE"];
    (t.is(0, "ANALYZE") && t.is_any(1, &INNER)).then(|| t.start(1))
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

// ## m3b-t26-prod-unlock (named exception, T26 carry MUST 1)
/// The MySQL/MariaDB `UPDATE`/`DELETE` forms `sqlparser` reads wrongly or not at all, as forms it
/// reads right with the same targets (spans stay the original tokens'):
/// - **Modifiers.** `UPDATE [LOW_PRIORITY] [IGNORE]` and `DELETE [LOW_PRIORITY] [QUICK]
///   [IGNORE]`: `sqlparser` reads a lone modifier as the *table* (`UPDATE LOW_PRIORITY orders`
///   writes "LOW_PRIORITY", alias `orders`), so a production confirmation would ask to type the
///   modifier while `orders` changes. They change locking and error handling, never which
///   tables are written: they are read as white space.
/// - **The comma form of a multi-table `UPDATE`** (`UPDATE a, b SET …`), which it does not
///   parse: every top-level comma between the `UPDATE` and its first top-level `SET` becomes
///   `CROSS JOIN` (the comma is an inner join without a condition to the server; MySQL's
///   `table_references`).
///
/// Whether a comma was rewritten (the walk then does not call those joins a missing `ON`); any
/// other statement is returned as it was.
fn mysql_dml_as_parsed(mut tokens: Vec<TokenWithSpan>) -> (Vec<TokenWithSpan>, bool) {
    use sqlparser::keywords::Keyword;
    use sqlparser::tokenizer::Whitespace;
    let code = |t: &Token| !matches!(t, Token::Whitespace(_));
    let is_kw = |t: &Token, k: Keyword| matches!(t, Token::Word(w) if w.keyword == k && w.quote_style.is_none());
    let is_word = |t: &Token, word: &str| matches!(t, Token::Word(w) if w.quote_style.is_none() && w.value.eq_ignore_ascii_case(word));
    let Some(start) = tokens.iter().position(|t| code(&t.token)) else {
        return (tokens, false);
    };
    let update = is_kw(&tokens[start].token, Keyword::UPDATE);
    let modifiers: &[&str] = if update {
        &["LOW_PRIORITY", "IGNORE"]
    } else if is_kw(&tokens[start].token, Keyword::DELETE) {
        &["LOW_PRIORITY", "QUICK", "IGNORE"]
    } else {
        return (tokens, false);
    };
    // The modifiers right after the keyword, in any order MySQL accepts them, each once.
    let mut i = start + 1;
    loop {
        while i < tokens.len() && !code(&tokens[i].token) {
            i += 1;
        }
        if i < tokens.len() && modifiers.iter().any(|m| is_word(&tokens[i].token, m)) {
            tokens[i].token = Token::Whitespace(Whitespace::Space);
            i += 1;
        } else {
            break;
        }
    }
    if !update {
        return (tokens, false);
    }
    let mut depth = 0usize;
    let mut commas = Vec::new();
    let mut set = false;
    for (i, t) in tokens.iter().enumerate().skip(start + 1) {
        match &t.token {
            Token::LParen => depth += 1,
            Token::RParen => depth = depth.saturating_sub(1),
            Token::SemiColon if depth == 0 => break,
            Token::Comma if depth == 0 => commas.push(i),
            tok if depth == 0 && is_kw(tok, Keyword::SET) => {
                set = true;
                break;
            }
            _ => {}
        }
    }
    if !set || commas.is_empty() {
        return (tokens, false);
    }
    let mut out = Vec::with_capacity(tokens.len() + 3 * commas.len());
    for (i, t) in tokens.into_iter().enumerate() {
        if commas.contains(&i) {
            let span = t.span;
            for token in [
                Token::Whitespace(Whitespace::Space),
                Token::make_keyword("CROSS"),
                Token::Whitespace(Whitespace::Space),
                Token::make_keyword("JOIN"),
            ] {
                out.push(TokenWithSpan { token, span });
            }
        } else {
            out.push(t);
        }
    }
    (out, true)
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

// ## m3b-t15-gate (named exception, T15 fix round 1, I3)
/// The object kind a `DROP`'s `ObjectType` (by its variant name) names; anything else is
/// `Other`.
fn drop_object_kind(variant: &str) -> ObjectKind {
    match variant {
        "Table" | "ForeignTable" => ObjectKind::Table,
        "View" | "MaterializedView" => ObjectKind::View,
        "Schema" => ObjectKind::Schema,
        "Database" => ObjectKind::Database,
        "Role" | "User" | "Group" => ObjectKind::Role,
        "Sequence" => ObjectKind::Sequence,
        _ => ObjectKind::Other,
    }
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
    // ## m3b-t26-prod-unlock (named exception, T26 carry MUST 1)
    /// The view was the comma form of a multi-table `UPDATE`, read with `CROSS JOIN`s
    /// ([`mysql_dml_as_parsed`]): those joins are the user's commas, not a join that forgot
    /// its `ON` (the comma form of a multi-table `DELETE` is not one either).
    comma_update: bool,
}

impl<'c, 'a> Walk<'c, 'a> {
    fn new(ctx: &'c ClassifyCtx, range: Option<TextRange>, acc: &'a mut Acc) -> Self {
        Walk {
            ctx,
            range,
            acc,
            top: None,
            plain_explain: 0,
            comma_update: false,
        }
    }

    fn destructive(&mut self, kind: DestructiveKind, target: Option<&ObjectName>) {
        self.destructive_of(kind, target, ObjectKind::Table);
    }

    // ## m3b-t15-gate (named exception, T15 fix round 1, I3): a destructive target carries the
    // kind of object it names (`DROP DATABASE d` → `Database`), so the gate asks for the typed
    // name on MySQL and MariaDB as it does on PostgreSQL.
    fn destructive_of(
        &mut self,
        kind: DestructiveKind,
        target: Option<&ObjectName>,
        object: ObjectKind,
    ) {
        if self.plain_explain > 0 {
            return;
        }
        let Some(range) = self.range else {
            return;
        };
        let target = target
            .and_then(qualified)
            .map(|name| ObjectRef { name, kind: object });
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
                // ## m3b-t26-prod-unlock (named exception, T26 carry MUST 1): every table a
                // `SET` target names is a target of the unbounded patterns and a write, so a
                // joined `UPDATE a JOIN b … SET b.y = 2` asks for `b` (the table that changes),
                // never the first table; more than one is several targets (the gate denies them
                // on production, as a multi-table `DELETE`).
                let targets = update_targets(u, self.ctx.dialect.id);
                let no_order = u.order_by.is_empty();
                if targets.is_empty() {
                    self.unbounded(u.selection.as_ref(), &u.limit, no_order, None);
                }
                for t in &targets {
                    self.unbounded(u.selection.as_ref(), &u.limit, no_order, Some(t));
                }
                self.joins_without_on(std::slice::from_ref(&u.table));
                for t in &targets {
                    self.write_target(t);
                }
            }
            Statement::Delete(d) => {
                let from = match &d.from {
                    sqlparser::ast::FromTable::WithFromKeyword(v)
                    | sqlparser::ast::FromTable::WithoutKeyword(v) => v.as_slice(),
                };
                // ## m3b-t28-matrix (named exception, T28 fix round 1 finding): a MySQL
                // multi-table `DELETE a, b FROM …` deletes from every listed table, so each one
                // is a target of its patterns: typing one name must never approve another (the
                // gate then denies the several targets on production).
                // `DELETE a, b FROM …` lists them before `FROM`; `DELETE FROM a, b USING …` (and
                // the single-table form) in `FROM`.
                let mut targets: Vec<ObjectName> = d.tables.clone();
                if targets.is_empty() {
                    targets.extend(from.iter().filter_map(|t| match &t.relation {
                        TableFactor::Table { name, .. } => Some(name.clone()),
                        _ => None,
                    }));
                }
                if targets.is_empty() {
                    self.unbounded(d.selection.as_ref(), &d.limit, d.order_by.is_empty(), None);
                }
                for t in &targets {
                    self.unbounded(
                        d.selection.as_ref(),
                        &d.limit,
                        d.order_by.is_empty(),
                        Some(t),
                    );
                }
                self.joins_without_on(from);
                for t in &targets {
                    self.write_target(t);
                }
            }
            Statement::Drop {
                object_type,
                cascade,
                names,
                ..
            } => {
                let variant = variant_name(object_type);
                if matches!(variant.as_str(), "User" | "Role") {
                    effects |= EffectSet::PRIVILEGE;
                }
                // ## m3b-t15-gate (named exception, T15 fix round 1, I3)
                let object = drop_object_kind(&variant);
                for n in names {
                    let kind = DestructiveKind::Drop { cascade: *cascade };
                    self.destructive_of(kind, Some(n), object);
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
            Statement::Call(f) => {
                // A procedure is never a builtin function (fix round 1, M1): its denial is
                // read from the deny list, and its body is always unknown.
                match qualified(&f.name) {
                    Some(q) => {
                        prescan::assess_call(&q, self.range, None, self.ctx, true, self.acc);
                        self.acc.doubt(ReasonCode::FnUnknown, self.range);
                        if let Some(r) = self.acc.reasons.last_mut() {
                            r.arg = Some(prescan::shown(&q));
                        }
                        self.acc.touched.calls.push(ObjectRef {
                            name: q,
                            kind: ObjectKind::Function,
                        });
                    }
                    None => self.acc.doubt(ReasonCode::FnUnknown, self.range),
                }
            }
            // `WITH … DELETE` is a DELETE (fix round 1, M5): the kind of a query whose body is
            // a data change is that change's.
            Statement::Query(q) => {
                if let SetExpr::Insert(inner)
                | SetExpr::Update(inner)
                | SetExpr::Delete(inner)
                | SetExpr::Merge(inner) = &*q.body
                    && let Ok(t) = table_for(self.ctx.dialect.id)
                    && let Some((_, r)) = t.iter().find(|(v, _)| *v == variant_name(inner))
                {
                    kind = r.kind;
                }
            }
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
            Some(e) if always_true(e, self.ctx.dialect.modes.ansi_quotes) => {
                self.destructive(DestructiveKind::AlwaysTrue, target)
            }
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
                // ## m3b-t26-prod-unlock (named exception): a comma of the comma form.
                if self.comma_update && op.starts_with("CrossJoin") {
                    continue;
                }
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

// ## m3b-t26-prod-unlock (named exception, T26 carry MUST 1)
/// The tables an `UPDATE` changes: the table of a single-table `UPDATE` (whatever its `SET`
/// says); in a joined (or comma) one, every table a `SET` target's qualifier names, by its alias
/// or by its name's last parts (`ident_eq`, so a qualifier that may name two tables names
/// both). Fail closed: an unqualified `SET` column, or a qualifier that names no listed table,
/// makes every listed table a target. Each name once, as the statement lists it.
fn update_targets(u: &sqlparser::ast::Update, d: DialectId) -> Vec<ObjectName> {
    let listed: Vec<(&ObjectName, Option<&sqlparser::ast::Ident>)> =
        std::iter::once(&u.table.relation)
            .chain(u.table.joins.iter().map(|j| &j.relation))
            .filter_map(|t| match t {
                TableFactor::Table { name, alias, .. } => {
                    Some((name, alias.as_ref().map(|a| &a.name)))
                }
                _ => None,
            })
            .collect();
    let all = || -> Vec<ObjectName> {
        let mut out: Vec<ObjectName> = Vec::new();
        for (n, _) in &listed {
            if !out.contains(n) {
                out.push((*n).clone());
            }
        }
        out
    };
    let single = u.table.joins.is_empty() && listed.len() == 1;
    if single {
        return all();
    }
    let ident = |i: &sqlparser::ast::Ident| Ident {
        text: Arc::from(i.value.as_str()),
        quoted: i.quote_style.is_some(),
    };
    let mut out: Vec<ObjectName> = Vec::new();
    let columns = u.assignments.iter().flat_map(|a| match &a.target {
        sqlparser::ast::AssignmentTarget::ColumnName(n) => vec![n],
        sqlparser::ast::AssignmentTarget::Tuple(ns) => ns.iter().collect(),
    });
    for column in columns {
        let Some(q) = qualified(column) else {
            return all();
        };
        if q.parts.len() < 2 {
            return all();
        }
        let qualifier = &q.parts[..q.parts.len() - 1];
        let mut hit = false;
        for (name, alias) in &listed {
            let by_alias = alias
                .is_some_and(|a| qualifier.len() == 1 && ident_eq(d, &qualifier[0], &ident(a)));
            let by_name = alias.is_none()
                && qualified(name).is_some_and(|n| {
                    let k = qualifier.len().min(n.parts.len());
                    k > 0
                        && qualifier[qualifier.len() - k..]
                            .iter()
                            .zip(&n.parts[n.parts.len() - k..])
                            .all(|(a, b)| ident_eq(d, a, b))
                });
            if by_alias || by_name {
                hit = true;
                if !out.contains(name) {
                    out.push((*name).clone());
                }
            }
        }
        if !hit {
            return all();
        }
    }
    if out.is_empty() { all() } else { out }
}

/// A `WHERE` that keeps every row (SAFE-07 `AlwaysTrue`, UNSAFE-06), by the shared rule
/// [`crate::destructive::always_true`] over this tree's [`Predicate`] (fix round 1, I2).
fn always_true(e: &Expr, ansi_quotes: bool) -> bool {
    crate::destructive::always_true(&predicate(e, ansi_quotes))
}

/// `e` as a SAFE-07 [`Predicate`]: literals and the logic between them; a constant-only
/// expression the rule does not model is `Const`; anything that reads the row is `Row`.
fn predicate(e: &Expr, ansi: bool) -> Predicate {
    use sqlparser::ast::{BinaryOperator as B, UnaryOperator as U};
    let b = |x: &Expr| Box::new(predicate(x, ansi));
    match e {
        Expr::Nested(inner) => predicate(inner, ansi),
        Expr::Value(v) => match &v.value {
            Value::Boolean(x) => Predicate::Bool(*x),
            Value::Null => Predicate::Null,
            Value::Number(n, _) => n.parse::<f64>().map_or(Predicate::Const, Predicate::Number),
            Value::SingleQuotedString(s) => Predicate::Text(s.clone()),
            // Under ANSI_QUOTES `"a"` is a column.
            Value::DoubleQuotedString(_) if ansi => Predicate::Row,
            Value::DoubleQuotedString(s) => Predicate::Text(s.clone()),
            _ => Predicate::Const,
        },
        Expr::UnaryOp { op: U::Not, expr } => Predicate::Not(b(expr)),
        Expr::BinaryOp { left, op, right } => {
            let cmp = |c| Predicate::Cmp(c, b(left), b(right));
            match op {
                B::And => Predicate::And(b(left), b(right)),
                B::Or => Predicate::Or(b(left), b(right)),
                B::Eq => cmp(CmpOp::Eq),
                B::NotEq => cmp(CmpOp::NotEq),
                B::Lt => cmp(CmpOp::Lt),
                B::LtEq => cmp(CmpOp::LtEq),
                B::Gt => cmp(CmpOp::Gt),
                B::GtEq => cmp(CmpOp::GtEq),
                B::Spaceship => cmp(CmpOp::NullSafeEq),
                _ => constness(e, ansi),
            }
        }
        Expr::IsNull(x) => Predicate::IsNull {
            expr: b(x),
            negated: false,
        },
        Expr::IsNotNull(x) => Predicate::IsNull {
            expr: b(x),
            negated: true,
        },
        _ => constness(e, ansi),
    }
}

/// `Const` when `e` reads nothing of the row (no column, call or subquery; a user variable
/// or a bound parameter is constant for the statement), else `Row`.
fn constness(e: &Expr, ansi: bool) -> Predicate {
    struct RowRef {
        ansi: bool,
    }
    impl Visitor for RowRef {
        type Break = ();
        fn pre_visit_expr(&mut self, e: &Expr) -> ControlFlow<()> {
            match e {
                Expr::Identifier(i) if i.quote_style.is_none() && i.value.starts_with('@') => {
                    ControlFlow::Continue(())
                }
                Expr::Identifier(_)
                | Expr::CompoundIdentifier(_)
                | Expr::Function(_)
                | Expr::Subquery(_)
                | Expr::Exists { .. }
                | Expr::Wildcard(_)
                | Expr::QualifiedWildcard(..) => ControlFlow::Break(()),
                _ => ControlFlow::Continue(()),
            }
        }
        fn pre_visit_value(&mut self, v: &sqlparser::ast::ValueWithSpan) -> ControlFlow<()> {
            if self.ansi && matches!(v.value, Value::DoubleQuotedString(_)) {
                ControlFlow::Break(())
            } else {
                ControlFlow::Continue(())
            }
        }
        fn pre_visit_query(&mut self, _q: &Query) -> ControlFlow<()> {
            ControlFlow::Break(())
        }
    }
    match e.visit(&mut RowRef { ansi }) {
        ControlFlow::Continue(()) => Predicate::Const,
        ControlFlow::Break(()) => Predicate::Row,
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

    // ## m3b-t28-matrix (named exception, T28 review M1 ruling)
    /// MariaDB's `ANALYZE <dml>` runs its statement: it is classified exactly as the bare
    /// statement (kind, effects, class, destructive patterns), never `Unknown` and never an
    /// `EXPLAIN ANALYZE`. On MySQL (no such statement) and for other `ANALYZE` forms
    /// (`ANALYZE SELECT`, `FORMAT=`) nothing changes: they stay `Unknown`.
    #[test]
    fn mariadb_analyze_dml_is_the_dml_itself() {
        let ctx = |d| {
            ClassifyCtx::new(
                DialectCtx::new(d, None),
                sql_dialect::ClientFlavor::None,
                crate::Expect::SingleStatement,
            )
        };
        let maria = ctx(DialectId::MariaDb);
        let shape = |r: &SafetyReport| {
            let s = &r.items[0];
            (
                s.kind,
                s.effects,
                s.class,
                r.request_class,
                s.destructive.iter().map(|d| d.kind).collect::<Vec<_>>(),
                s.flags.contains(RiskFlags::EXPLAIN_ANALYZE),
            )
        };
        for inner in [
            "DELETE FROM t WHERE a > 0",
            "DELETE FROM t",
            "UPDATE t SET a = 1 WHERE a > 0",
            "UPDATE t SET a = 1",
            "INSERT INTO t VALUES (1)",
            "REPLACE INTO t VALUES (1)",
        ] {
            let bare = classify_interim(inner, &maria);
            let analyzed = classify_interim(&format!("ANALYZE {inner}"), &maria);
            assert_eq!(shape(&analyzed), shape(&bare), "ANALYZE {inner}");
            assert_ne!(
                analyzed.request_class,
                CoarseClass::Unknown,
                "ANALYZE {inner}"
            );
            assert!(!shape(&analyzed).5, "ANALYZE {inner}: never the D-062 wrap");
        }
        let w = classify_interim("ANALYZE DELETE FROM t WHERE a > 0", &maria);
        assert!(w.items[0].effects.contains(EffectSet::WRITE_DATA));
        // Other forms stay as they were: MySQL has no `ANALYZE <dml>`, and `FORMAT=JSON` is
        // not followed.
        for (d, sql) in [
            (DialectId::MySql, "ANALYZE DELETE FROM t WHERE a > 0"),
            (
                DialectId::MariaDb,
                "ANALYZE FORMAT=JSON DELETE FROM t WHERE a > 0",
            ),
            (DialectId::MariaDb, "ANALYZE SELECT a FROM t"),
        ] {
            let r = classify_interim(sql, &ctx(d));
            assert_eq!(r.request_class, CoarseClass::Unknown, "{d:?} {sql}");
        }
    }

    // ## m3b-t28-matrix (named exception, T28 fix round 1 finding)
    /// A multi-table `DELETE` names every table it deletes from as a target of its unbounded
    /// pattern (MySQL and MariaDB, both spellings); a single-table one is unchanged.
    #[test]
    fn multi_table_delete_targets_every_table() {
        for d in [DialectId::MySql, DialectId::MariaDb] {
            let ctx = ClassifyCtx::new(
                DialectCtx::new(d, None),
                sql_dialect::ClientFlavor::None,
                crate::Expect::SingleStatement,
            );
            let targets = |sql: &str| -> Vec<String> {
                let r = classify_interim(sql, &ctx);
                r.items
                    .iter()
                    .flat_map(|s| &s.destructive)
                    .filter(|x| x.kind == DestructiveKind::NoWhere)
                    .filter_map(|x| x.target.as_ref().map(|t| format!("{:?}", t.name)))
                    .collect()
            };
            for sql in [
                "DELETE a, b FROM a JOIN b ON a.x = b.x",
                "DELETE FROM a, b USING a JOIN b ON a.x = b.x",
            ] {
                assert_eq!(targets(sql).len(), 2, "{d:?} {sql}: {:?}", targets(sql));
            }
            assert_eq!(targets("DELETE FROM a").len(), 1, "{d:?}");
            assert_eq!(
                targets("DELETE a FROM a JOIN b ON a.x = b.x").len(),
                1,
                "{d:?}"
            );
        }
    }

    // ## m3b-t26-prod-unlock (named exception, T26 carry MUST 1)
    /// The tables an `UPDATE` changes are its targets: a joined or comma `UPDATE` names the
    /// `SET` tables (an unqualified column, every listed table); the MySQL modifiers are never a
    /// table; the comma form parses (not `Unknown`) and its commas are no missing `ON`.
    #[test]
    fn update_targets_are_the_tables_that_change() {
        for d in [DialectId::MySql, DialectId::MariaDb] {
            let ctx = ClassifyCtx::new(
                DialectCtx::new(d, None),
                sql_dialect::ClientFlavor::None,
                crate::Expect::SingleStatement,
            );
            let names = |sql: &str| -> (Vec<String>, Vec<String>, Vec<DestructiveKind>) {
                let r = classify_interim(sql, &ctx);
                assert_ne!(r.request_class, CoarseClass::Unknown, "{d:?} {sql}: {r:?}");
                let text = |n: &QualifiedName| {
                    n.parts
                        .iter()
                        .map(|p| p.text.to_string())
                        .collect::<Vec<_>>()
                        .join(".")
                };
                let s = &r.items[0];
                (
                    s.destructive
                        .iter()
                        .filter(|x| x.kind == DestructiveKind::NoWhere)
                        .filter_map(|x| x.target.as_ref().map(|t| text(&t.name)))
                        .collect(),
                    s.touched.writes.iter().map(|w| text(&w.name)).collect(),
                    s.destructive.iter().map(|x| x.kind).collect(),
                )
            };
            for (sql, want) in [
                ("UPDATE a JOIN b ON a.id = b.id SET b.y = 2", &["b"][..]),
                (
                    "UPDATE a AS x JOIN b AS z ON x.id = z.id SET z.y = 2",
                    &["b"],
                ),
                (
                    "UPDATE a JOIN b ON a.id = b.id SET a.x = 1, b.y = 2",
                    &["a", "b"],
                ),
                ("UPDATE a JOIN b ON a.id = b.id SET y = 2", &["a", "b"]),
                ("UPDATE a, b SET b.y = 2", &["b"]),
                ("UPDATE a, b SET y = 2", &["a", "b"]),
                ("UPDATE a, b SET q.y = 2", &["a", "b"]),
                ("UPDATE a x, b z SET (z.y) = 2", &["b"]),
                ("UPDATE LOW_PRIORITY orders SET a = 1", &["orders"]),
                ("UPDATE IGNORE orders SET a = 1", &["orders"]),
                (
                    "UPDATE LOW_PRIORITY IGNORE orders o SET o.a = 1",
                    &["orders"],
                ),
                ("UPDATE LOW_PRIORITY a, b SET a.y = b.y", &["a"]),
                ("DELETE LOW_PRIORITY FROM orders", &["orders"]),
                ("DELETE QUICK FROM orders", &["orders"]),
                ("DELETE IGNORE FROM orders", &["orders"]),
                ("DELETE LOW_PRIORITY QUICK IGNORE FROM orders", &["orders"]),
            ] {
                let (unbounded, writes, kinds) = names(sql);
                assert_eq!(unbounded, want, "{d:?} {sql}: the unbounded targets");
                assert_eq!(writes, want, "{d:?} {sql}: the writes");
                assert!(
                    !kinds.contains(&DestructiveKind::JoinWithoutOn),
                    "{d:?} {sql}: {kinds:?}"
                );
            }
            // A real join without `ON` is still one.
            let (_, _, kinds) = names("UPDATE a JOIN b SET b.y = 2 WHERE a.id = b.id");
            assert!(
                kinds.contains(&DestructiveKind::JoinWithoutOn),
                "{d:?}: {kinds:?}"
            );
            // A quoted modifier word is a table.
            let (unbounded, _, _) = names("UPDATE `LOW_PRIORITY` SET a = 1");
            assert_eq!(unbounded, ["LOW_PRIORITY"], "{d:?}");
        }
    }

    // ## m3b-t15-gate (named exception, T15 fix round 1, I3)
    #[test]
    fn drop_targets_carry_their_object_kind() {
        for d in [DialectId::MySql, DialectId::MariaDb] {
            let ctx = ClassifyCtx::new(
                DialectCtx::new(d, None),
                sql_dialect::ClientFlavor::None,
                crate::Expect::SingleStatement,
            );
            for (sql, kind) in [
                ("DROP DATABASE shop", ObjectKind::Database),
                ("DROP SCHEMA shop", ObjectKind::Database),
                ("DROP TABLE t", ObjectKind::Table),
                ("DROP VIEW v", ObjectKind::View),
            ] {
                let r = classify_interim(sql, &ctx);
                let got: Vec<ObjectKind> = r
                    .items
                    .iter()
                    .flat_map(|s| &s.destructive)
                    .filter_map(|x| x.target.as_ref().map(|t| t.kind))
                    .collect();
                // MySQL parses `SCHEMA` as a synonym of `DATABASE` (either is a typed name).
                let ok = got == [kind] || (sql.contains("SCHEMA") && got == [ObjectKind::Schema]);
                assert!(ok, "{d:?} {sql}: {got:?}");
            }
        }
    }

    /// Fix round 1 (M8): the walk reads `Debug` text in three places (variant names, join
    /// operators, `ALTER COLUMN … TYPE`). The pin is `=0.64.0`; this golden fails when a bump
    /// changes any of that text, so the matches are revisited instead of silently dropping
    /// `JoinWithoutOn` or `AlterType`.
    #[test]
    fn sqlparser_is_pinned_and_its_debug_text_is_the_walks() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../Cargo.toml");
        let manifest = std::fs::read_to_string(root).unwrap();
        assert!(
            manifest.contains("sqlparser = { version = \"=0.64.0\""),
            "the workspace pins sqlparser =0.64.0"
        );
        assert_eq!(SQLPARSER.version, "0.64.0");
        let one = |sql: &str| Parser::parse_sql(&MySqlDialect {}, sql).unwrap().remove(0);
        let joins = |sql: &str| -> Vec<String> {
            let Statement::Query(q) = one(sql) else {
                panic!("{sql}")
            };
            let sqlparser::ast::SetExpr::Select(s) = &*q.body else {
                panic!("{sql}")
            };
            s.from[0]
                .joins
                .iter()
                .map(|j| format!("{:?}", j.join_operator))
                .collect()
        };
        assert_eq!(joins("SELECT * FROM a JOIN b"), ["Join(None)"]);
        assert_eq!(joins("SELECT * FROM a CROSS JOIN b"), ["CrossJoin(None)"]);
        assert!(joins("SELECT * FROM a JOIN b ON a.x = b.x")[0].starts_with("Join(On("));
        let Statement::AlterTable(a) = one("ALTER TABLE t ALTER COLUMN c SET DATA TYPE BIGINT")
        else {
            panic!("alter")
        };
        assert_eq!(variant_name(&a.operations[0]), "AlterColumn");
        assert!(format!("{:?}", a.operations[0]).contains("SetDataType"));
        for (sql, name) in [
            ("DELETE FROM t", "Delete"),
            ("UPDATE t SET a = 1", "Update"),
            ("SET time_zone = '+00:00'", "Set"),
            ("CALL p()", "Call"),
            ("EXPLAIN SELECT 1", "Explain"),
            ("DESCRIBE t", "ExplainTable"),
            ("ALTER TABLE t DROP COLUMN c", "AlterTable"),
        ] {
            assert_eq!(variant_name(&one(sql)), name, "{sql}");
        }
        let Statement::AlterTable(a) = one("ALTER TABLE t DROP COLUMN c, MODIFY d INT") else {
            panic!("alter")
        };
        let ops: Vec<String> = a.operations.iter().map(variant_name).collect();
        assert_eq!(ops, ["DropColumn", "ModifyColumn"]);
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
