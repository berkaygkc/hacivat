//! The orchestrator (`alg/13` SQL-SAFE-02 steps; Task 14): limits, lexing, splitting, the
//! backend for the dialect, the union with the lexer pre-scan, and the request's report.
//!
//! [`classify`] reads exactly the bytes it is given (the bytes Run sends, P-M3b-2) and never a
//! normalised copy. Its steps, each failing closed to `Unknown` with a reason:
//!
//! 1. **L0, before any parser** (R-M3-3). Longer than `Limits.max_bytes` → `TooComplex`, first;
//!    no other check runs. A dialect without a backend → `UnsupportedDialect`. The text is
//!    lexed once (with the client flavor, so a psql `\set` is one client-command token); any
//!    lexer error is `LexError` (a NUL byte, an unterminated string or comment, an unknown
//!    character, trailing junk, …), except `AmbiguousCommentEnd`, which is
//!    `AmbiguousBoundary`. More than `Limits.max_tokens` significant tokens, or brackets
//!    (`(`, `[`, `{`, counted together) plus `CASE … END` blocks nested deeper than
//!    `Limits.max_nesting`, → `TooComplex`.
//! 2. **Pre-pass.** Any client command (psql `\cmd`, mysql `DELIMITER`, sqlite3 `.cmd`) or
//!    `COPY … FROM STDIN` data → `ClientCommand`: those bytes are never what Run sends. A
//!    template variable (`${x}`, `%x%`, `<x>`, SQL-PARAM-02) → `TemplateVariable`.
//! 3. **Execution-mode split.** No statement → `NoStatement`; more than one when
//!    `Expect::SingleStatement` → `MultiStatement`; an uncertain boundary anywhere →
//!    `AmbiguousBoundary`.
//!
//!    Steps 1–3 collect every reason they find; any reason makes the report pre-decided
//!    (`pre_decision`, no items) and no backend runs.
//! 4. **Threshold views** (MySQL, MariaDB) are built by the interim backend per statement
//!    (Task 10): every executable-comment reading, up to `Limits.max_exec_comment_views`.
//! 5. **Dispatch** by [`plan_for`](crate::backend::plan::plan_for): PostgreSQL and the PG-wire
//!    flavours → libpg_query (Task 9, on the oracle's serial thread); MySQL, MariaDB, SQLite →
//!    the interim backend (Task 10). Each statement's strict range (the text sent: no leading
//!    comment, no terminator) is one item.
//! 6. **Combine.** Per item: exact duplicate reasons dropped; a PostgreSQL `COPY … TO STDOUT`
//!    is `Unknown` (`alg/13` S-12: exports go through a dedicated path, never a query tool;
//!    Run also refuses the unit by kind, ADR 0005 `m3a-f1`); and after an item that may
//!    change how later items read ([`changes_context`]: the search path, the lexing modes,
//!    the role, code the classifier cannot see, a cursor or prepared statement that now runs),
//!    every later item is `Unknown` (`EffectsUnknown`). The request's class, effects and flags
//!    are [`merge`]d; the fingerprint is libpg_query's on PostgreSQL (`HvNorm1` when it
//!    cannot, and on every other dialect); the backends' versions are recorded.
//!
//! **Producer contract** (T4 review): every item a backend returns has `class ==
//! class_of(effects)`, and an `Unknown` item carries `Unknown` evidence (`EffectSet::UNKNOWN`
//! or a reason the policy reads as doubt or unknown). A debug build asserts both; a release
//! build that meets a violation makes the item `Unknown` (`EffectsUnknown`), never lower.
//!
//! **Units classified one at a time** ([`classify_units`]): the caller classifies a run's
//! units before any of them runs (R-M3-84), so a unit that changes the context cannot be
//! followed by a refreshed one: every later unit is `Unknown` instead (ADR 0005 `m3b-t09`
//! carry, R1-M5). Across runs, [`changes_context`] tells the caller when to re-read the
//! session's search path and lexing modes before it classifies again.

use std::fmt;
use std::sync::Arc;

use sql_dialect::{ClientFlavor, DialectCtx, DialectId, lex_profile};
use sql_lexer::{LexErrorCode, LexState, Lexed, TokenKind, lex_with_client};
use sql_split::item::StmtHead;
use sql_split::{ParamSettings, SplitMode, scan_params};
use sql_text::{TextRange, TextSize};

use crate::backend::Backend;
use crate::backend::plan::{Plan, plan_for};
use crate::effects::{CoarseClass, EffectSet, RiskFlags, class_of};
use crate::fingerprint::request_fingerprint;
use crate::functions::{FnCatalog, set_allowed};
use crate::limits::Limits;
use crate::policy::{Group, ReasonRule, reason_rule};
use crate::reason::{Reason, ReasonCode};
use crate::report::{SafetyReport, StmtKind, StmtReport, merge};

/// What the caller expects the text to hold.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Expect {
    /// One execution unit (Run); more than one statement is `MultiStatement`.
    SingleStatement,
    /// A script of statements.
    Script,
}

/// Everything classification depends on besides the text.
#[derive(Clone)]
pub struct ClassifyCtx {
    pub dialect: DialectCtx,
    pub flavor: ClientFlavor,
    pub expect: Expect,
    pub catalog: Option<Arc<dyn FnCatalog>>,
    /// The profile's read-only function allowlist (P-M3b-10).
    pub fn_allowlist: Arc<[Arc<str>]>,
    // ## m3b-t09-pg-backend (named exception, fix round 1, I2b): the field's contract.
    /// The session's search path, one schema per element, each its name exactly as the server
    /// stores it (folded: what `current_schemas(true)` returns, or `SHOW search_path` split and
    /// folded), `$user` kept literally. Empty means unknown (`pg_catalog` assumed first, as the
    /// server searches it first unless it is listed). An element that cannot be such a name
    /// (a raw `SHOW` string, quotes, a comma, edge spaces, `pg_catalog` in another case) is
    /// read fail-closed: unqualified builtins are then unknown (PG backend, P-M3b-T3-1).
    pub search_path: Arc<[Arc<str>]>,
    pub limits: Limits,
}

impl ClassifyCtx {
    /// A context with no catalog, empty allowlist and search path, and the default limits.
    pub fn new(dialect: DialectCtx, flavor: ClientFlavor, expect: Expect) -> Self {
        ClassifyCtx {
            dialect,
            flavor,
            expect,
            catalog: None,
            fn_allowlist: Arc::from(Vec::new()),
            search_path: Arc::from(Vec::new()),
            limits: Limits::default(),
        }
    }
}

impl fmt::Debug for ClassifyCtx {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ClassifyCtx")
            .field("dialect", &self.dialect)
            .field("flavor", &self.flavor)
            .field("expect", &self.expect)
            .field("catalog", &self.catalog.is_some())
            .field("fn_allowlist", &self.fn_allowlist)
            .field("search_path", &self.search_path)
            .field("limits", &self.limits)
            .finish()
    }
}

// ## m3b-t14-orchestrator

/// Classifies `sql` in `ctx`: pure, synchronous, never more permissive than its evidence (the
/// module docs). Every libpg_query call it makes runs on the oracle's own serial parser
/// thread.
///
/// **Call it on `hacivat-exec::classify_pool`'s 16 MiB threads only** (R-M3-3, R-M3-53): the
/// token limit lets a left-deep operator chain of ~100,000 terms through (`SELECT 1 + 1 + …`),
/// which `sqlparser`'s derived walk and drop recurse through; measured at Task 14, a 2 MiB
/// thread (a tokio worker's default) aborts the process on it, in release too.
pub fn classify(sql: &str, ctx: &ClassifyCtx) -> SafetyReport {
    let plan = plan_for(ctx.dialect.id);
    classify_with(sql, ctx, plan, plan.backend())
}

/// [`classify`] with the plan's backend given (the unit tests inject failing ones).
fn classify_with(
    sql: &str,
    ctx: &ClassifyCtx,
    plan: Plan,
    backend: Option<&dyn Backend>,
) -> SafetyReport {
    let decided = |reasons: Vec<Reason>| {
        let mut r = SafetyReport::pre_decided(sql, sql_split::hv_norm1(sql, &ctx.dialect), reasons);
        r.dialect = Some(ctx.dialect.id);
        r.backends = plan.versions();
        r
    };

    // Step 1: L0. The size first, before any other check.
    if sql.len() > ctx.limits.max_bytes {
        return decided(vec![Reason::new(ReasonCode::TooComplex)]);
    }
    let Some(backend) = backend else {
        return decided(vec![Reason::new(ReasonCode::UnsupportedDialect)]);
    };
    let Ok(profile) = lex_profile(&ctx.dialect) else {
        return decided(vec![Reason::new(ReasonCode::UnsupportedDialect)]);
    };
    let lexed = lex_with_client(
        sql,
        profile,
        ctx.dialect.modes,
        &LexState::Initial,
        ctx.flavor,
    );
    let mut reasons = Vec::new();
    limits_and_lexing(sql, &lexed, &ctx.limits, &mut reasons);

    // Step 2: the pre-pass; step 3: the execution-mode split.
    let split = sql_split::split(sql, &lexed, &ctx.dialect, ctx.flavor, SplitMode::Execution);
    if let Some(c) = split.client.first() {
        push_once(&mut reasons, ReasonCode::ClientCommand, Some(c.range));
    }
    let stmts: Vec<_> = split.statements().collect();
    if let Some(data) = stmts.iter().find_map(|s| s.copy_data) {
        push_once(&mut reasons, ReasonCode::ClientCommand, Some(data.rows));
    }
    if let Some(at) = template_at(sql, &stmts, &ctx.dialect) {
        push_once(&mut reasons, ReasonCode::TemplateVariable, Some(at));
    }
    if let Some(at) = exec_comment_outside(sql, &lexed, &stmts, ctx.dialect.id) {
        push_once(&mut reasons, ReasonCode::ExecutableComment, Some(at));
    }
    if stmts.is_empty() {
        push_once(&mut reasons, ReasonCode::NoStatement, None);
    }
    if ctx.expect == Expect::SingleStatement && stmts.len() > 1 {
        push_once(
            &mut reasons,
            ReasonCode::MultiStatement,
            stmts.get(1).map(|s| s.strict),
        );
    }
    if let Some(a) = split.ambiguous.first() {
        push_once(&mut reasons, ReasonCode::AmbiguousBoundary, Some(a.at));
    } else if let Some(s) = stmts.iter().find(|s| !split.is_certain(s)) {
        push_once(&mut reasons, ReasonCode::AmbiguousBoundary, Some(s.strict));
    }
    if !reasons.is_empty() {
        return decided(reasons);
    }

    // Step 5: the backend, one item per statement (its strict range: the bytes sent).
    let ranges: Vec<TextRange> = stmts.iter().map(|s| s.strict).collect();
    // The lexing is lent only when it is exactly `sql_lexer::lex`'s (no client flavor): a
    // backend that lexes the same text again does not (T10 C1 carry).
    let plain = (ctx.flavor == ClientFlavor::None).then_some(&lexed);
    let mut items = backend.analyse(sql, &ranges, ctx, plain);
    if items.len() != ranges.len() || items.iter().zip(&ranges).any(|(i, r)| i.range != *r) {
        // A backend that does not answer one report per statement proves nothing.
        return decided(vec![Reason::new(ReasonCode::EffectsUnknown)]);
    }

    // Step 6: combine.
    for item in &mut items {
        enforce_producer_contract(item);
        dedup_reasons(item);
        copy_to_client(item, plan);
    }
    taint_after_context_changes(sql, &mut items, ctx.dialect.id);
    let (request_class, request_effects, flags) = merge(&items);
    // A text the parser already refused is not sent to it again for a fingerprint.
    let refused = items
        .iter()
        .any(|i| i.reasons.iter().any(|r| r.code == ReasonCode::ParseError));
    let fingerprint = if refused {
        sql_split::hv_norm1(sql, &ctx.dialect)
    } else {
        request_fingerprint(sql, &ctx.dialect)
    };
    SafetyReport {
        items,
        request_class,
        request_effects,
        flags,
        fingerprint,
        backends: plan.versions(),
        pre_decision: None,
        classified_len: u32::try_from(sql.len()).unwrap_or(u32::MAX),
        classified_hash: crate::fingerprint::classified_hash(sql),
        dialect: Some(ctx.dialect.id),
    }
}

/// Classifies a run's units in order, each with `ctx` (the module docs): once a unit may change
/// the context ([`changes_context`]), every later unit is `Unknown` (`EffectsUnknown` on every
/// item; a pre-decided report stays as it is). One report per unit, in order.
pub fn classify_units<S: AsRef<str>>(units: &[S], ctx: &ClassifyCtx) -> Vec<SafetyReport> {
    let mut stale = false;
    units
        .iter()
        .map(|u| {
            let mut r = classify(u.as_ref(), ctx);
            if stale {
                mark_stale(&mut r);
            }
            stale |= changes_context(u.as_ref(), &r);
            r
        })
        .collect()
}

/// Whether running the request `r`, the report of `sql`, may change how a later request must
/// be classified: its search path, its lexing modes, its role, or anything the classifier
/// could not see (the module docs). A pre-decided report (nothing was proved), a report
/// without a dialect, and a report of other bytes than `sql` always may.
///
/// The caller that classifies units one at a time either treats every later unit as unknown
/// ([`classify_units`]) or, across runs, re-reads `ClassifyCtx.search_path` and
/// `ClassifyCtx.dialect.modes` from the session before classifying again.
pub fn changes_context(sql: &str, r: &SafetyReport) -> bool {
    let Some(dialect) = r.dialect else {
        return true;
    };
    if r.pre_decision.is_some() || r.items.is_empty() || !r.matches_text(sql) {
        return true;
    }
    r.items
        .iter()
        .any(|i| item_changes_context(i, sql, dialect))
}

/// `r`'s items made `Unknown` after a context change earlier in the run, and its request
/// fields merged again.
fn mark_stale(r: &mut SafetyReport) {
    if r.pre_decision.is_some() {
        return;
    }
    for item in &mut r.items {
        doubt(item, ReasonCode::EffectsUnknown, None);
    }
    let (class, effects, flags) = merge(&r.items);
    r.request_class = class;
    r.request_effects = effects;
    r.flags = flags;
}

/// Effects after which later statements are read against a context that may be gone: code the
/// classifier cannot see, a routine, DDL (a new function, operator, cast, type or temp view a
/// later name may resolve to), and every administrative effect (`set_config`, `UPDATE
/// pg_settings`, `pg_reload_conf()`, `LOAD`, a privilege or role change, external code: the
/// report names no setting the session keeps, so any may have changed).
const TAINTING_EFFECTS: EffectSet = EffectSet::UNKNOWN
    .union(EffectSet::ROUTINE_CALL)
    .union(EffectSet::DDL)
    .union(EffectSet::DDL_TEMP)
    .union(EffectSet::ADMIN)
    .union(EffectSet::PRIVILEGE)
    .union(EffectSet::PROCESS_EXEC)
    .union(EffectSet::EXTERNAL_IO);

/// Words whose statement runs code defined earlier, perhaps in another run: `FETCH`/`MOVE` run
/// a cursor's query, `EXECUTE` a prepared statement (ADR 0005 `m3b-t09`, R1-M5).
const RUNS_DEFINED_CODE: [&str; 3] = ["FETCH", "MOVE", "EXECUTE"];

/// Whether running `item` may change how later statements read (the module docs); `sql` is the
/// text `item.range` points into.
fn item_changes_context(item: &StmtReport, sql: &str, dialect: DialectId) -> bool {
    if item.effects.intersects(TAINTING_EFFECTS)
        || class_of(item.effects) == CoarseClass::Unknown
        || item
            .flags
            .intersects(RiskFlags::LEX_MODE_CHANGE | RiskFlags::DYNAMIC_SQL)
        || item.kind == StmtKind::Execute
    {
        return true;
    }
    let doubtful = item.reasons.iter().any(|r| {
        matches!(
            r.code,
            ReasonCode::LexModeChange
                | ReasonCode::PrivilegeChange
                | ReasonCode::SetNotAllowlisted
                | ReasonCode::DynamicSql
                | ReasonCode::ProceduralCode
                | ReasonCode::RoutineCall
                | ReasonCode::EffectsUnknown
        )
    });
    if doubtful {
        return true;
    }
    // A `SET` that the dialect's harmless list does not hold may change the search path, the
    // lexing or anything else a later statement depends on (P-M3b-T3-1).
    if item
        .set_targets
        .iter()
        .any(|t| !set_allowed(dialect, &t.name, t.value.as_deref()))
    {
        return true;
    }
    if item.kind == StmtKind::Set && item.set_targets.is_empty() {
        return true;
    }
    let word = first_word(sql, item.range);
    RUNS_DEFINED_CODE
        .iter()
        .any(|w| word.eq_ignore_ascii_case(w))
}

/// The leading ASCII letters of `sql[range]`.
fn first_word(sql: &str, range: TextRange) -> &str {
    let s = usize::from(range.start()).min(sql.len());
    let rest = sql.get(s..).unwrap_or("");
    let n = rest.bytes().take_while(u8::is_ascii_alphabetic).count();
    &rest[..n]
}

/// After an item that may change the context, every later item of the same request is
/// `Unknown` (the module docs). The PostgreSQL backend applies its own rule first (Task 9);
/// this one adds the orchestrator's (`FETCH`/`MOVE`, refused `SET`s on every dialect, the
/// interim backend's items) without repeating a reason.
fn taint_after_context_changes(sql: &str, items: &mut [StmtReport], dialect: DialectId) {
    let mut stale = false;
    for item in items {
        if stale {
            doubt(item, ReasonCode::EffectsUnknown, None);
        }
        stale |= item_changes_context(item, sql, dialect);
    }
}

/// A PostgreSQL `COPY` that would be a read (`TO STDOUT`: a `COPY` to a server file or a
/// program is external I/O or process execution, `FROM` a write) is `Unknown`: `alg/13` S-12
/// denies it in query tools, whose exports have their own path.
fn copy_to_client(item: &mut StmtReport, plan: Plan) {
    if plan == Plan::Pg
        && item.kind == StmtKind::Copy
        && class_of(item.effects) == CoarseClass::Read
    {
        doubt(item, ReasonCode::EffectsUnknown, Some("COPY TO STDOUT"));
    }
}

/// Adds `code` (and `UNKNOWN`) to `item` unless an equal reason is there already.
fn doubt(item: &mut StmtReport, code: ReasonCode, arg: Option<&str>) {
    let reason = Reason {
        code,
        range: Some(item.range),
        arg: arg.map(Arc::from),
    };
    if !item.reasons.contains(&reason) {
        item.reasons.push(reason);
    }
    item.effects |= EffectSet::UNKNOWN;
    item.class = class_of(item.effects);
}

/// The producer contract (the module docs).
fn enforce_producer_contract(item: &mut StmtReport) {
    let consistent = item.class == class_of(item.effects);
    let evidenced = item.class != CoarseClass::Unknown
        || item.effects.contains(EffectSet::UNKNOWN)
        || item.reasons.iter().any(|r| is_unknown_evidence(r.code));
    debug_assert!(
        consistent,
        "producer contract: class {:?} != class_of({:?})",
        item.class, item.effects
    );
    debug_assert!(
        evidenced,
        "producer contract: an Unknown item without Unknown evidence ({:?})",
        item.reasons
    );
    if !consistent || !evidenced {
        doubt(item, ReasonCode::EffectsUnknown, None);
    }
}

/// A reason the policy reads as doubt or as unknown effects.
fn is_unknown_evidence(code: ReasonCode) -> bool {
    matches!(
        reason_rule(code),
        ReasonRule::DenyAlways | ReasonRule::Group(Group::Unknown | Group::Routine)
    )
}

/// Drops exact duplicate reasons, keeping the first of each in order.
fn dedup_reasons(item: &mut StmtReport) {
    let mut seen: Vec<Reason> = Vec::with_capacity(item.reasons.len());
    item.reasons.retain(|r| {
        if seen.contains(r) {
            false
        } else {
            seen.push(r.clone());
            true
        }
    });
}

/// Pushes `code` at `range` unless a reason with that code is there already.
fn push_once(reasons: &mut Vec<Reason>, code: ReasonCode, range: Option<TextRange>) {
    if !reasons.iter().any(|r| r.code == code) {
        reasons.push(Reason {
            code,
            range,
            arg: None,
        });
    }
}

/// Step 1's checks on the lexing: errors, the token count, the nesting.
fn limits_and_lexing(sql: &str, lexed: &Lexed, limits: &Limits, reasons: &mut Vec<Reason>) {
    for e in lexed.errors() {
        let code = if e.code == LexErrorCode::AmbiguousCommentEnd {
            ReasonCode::AmbiguousBoundary
        } else {
            ReasonCode::LexError
        };
        push_once(reasons, code, Some(e.range));
    }
    let mut significant: u32 = 0;
    let mut brackets: u32 = 0;
    let mut cases: u32 = 0;
    let mut deepest: u32 = 0;
    let mut i = 0;
    while i < lexed.len() {
        if lexed.is_trivia(i) {
            i += 1;
            continue;
        }
        significant = significant.saturating_add(1);
        match lexed.kind(i) {
            TokenKind::LParen | TokenKind::LBracket | TokenKind::LBrace => brackets += 1,
            TokenKind::RParen | TokenKind::RBracket | TokenKind::RBrace => {
                brackets = brackets.saturating_sub(1);
            }
            TokenKind::Word => {
                let word = token_text(sql, lexed, i);
                if word.eq_ignore_ascii_case("CASE") {
                    cases += 1;
                } else if word.eq_ignore_ascii_case("END") && !ends_a_program_block(sql, lexed, i) {
                    cases = cases.saturating_sub(1);
                }
            }
            _ => {}
        }
        deepest = deepest.max(brackets.saturating_add(cases));
        i += 1;
    }
    let too_deep = deepest > u32::from(limits.max_nesting);
    if significant > limits.max_tokens || too_deep {
        push_once(reasons, ReasonCode::TooComplex, None);
    }
}

/// Whether the `END` at token `i` closes a stored-program block (`END IF`, `END LOOP`, …)
/// rather than a `CASE`.
fn ends_a_program_block(sql: &str, lexed: &Lexed, i: usize) -> bool {
    let next = (i + 1..lexed.len()).find(|&j| !lexed.is_trivia(j));
    next.is_some_and(|j| {
        lexed.kind(j) == TokenKind::Word
            && ["IF", "LOOP", "WHILE", "REPEAT", "FOR"]
                .iter()
                .any(|w| token_text(sql, lexed, j).eq_ignore_ascii_case(w))
    })
}

fn token_text<'a>(sql: &'a str, lexed: &Lexed, i: usize) -> &'a str {
    let r = lexed.range(i);
    sql.get(usize::from(r.start())..usize::from(r.end()))
        .unwrap_or("")
}

/// On MySQL and MariaDB, the first comment outside every statement's strict range that a
/// server may run (`/*!…*/`, `/*M!…*/`, any case): the bytes passed are the bytes sent, and a
/// backend only reads the statements, so such a comment (a MariaDB server behind a MySQL
/// profile runs a trailing `/*M! INTO OUTFILE … */`; T10 carry) is never left unread.
fn exec_comment_outside(
    sql: &str,
    lexed: &Lexed,
    stmts: &[&sql_split::Stmt],
    d: DialectId,
) -> Option<TextRange> {
    if !matches!(d, DialectId::MySql | DialectId::MariaDb) {
        return None;
    }
    (0..lexed.len()).find_map(|i| {
        let r = lexed.range(i);
        let text = token_text(sql, lexed, i).as_bytes();
        let runs = text.starts_with(b"/*!")
            || (text.len() >= 4 && text[..4].eq_ignore_ascii_case(b"/*m!"));
        let inside = stmts.iter().any(|s| s.strict.contains_range(r));
        (runs && !inside).then_some(r)
    })
}

/// The first template variable in any statement, as a range in `sql` (SQL-PARAM-02). Only a
/// statement holding a byte a template can start with is scanned.
fn template_at(sql: &str, stmts: &[&sql_split::Stmt], dialect: &DialectCtx) -> Option<TextRange> {
    for s in stmts {
        let start = usize::from(s.strict.start());
        let text = sql.get(start..usize::from(s.strict.end()))?;
        if !text.contains("${") && !text.contains('%') && !text.contains('<') {
            continue;
        }
        let scan = scan_params(
            text,
            dialect,
            &StmtHead::default(),
            &ParamSettings::default(),
        );
        if let Some(t) = scan.templates.first() {
            let base = TextSize::from(u32::try_from(start).ok()?);
            return Some(t.range + base);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use smallvec::SmallVec;

    use super::*;
    use crate::report::{Confidence, TouchedObjects};

    /// A backend that answers whatever its function makes of the items.
    struct Fake(fn(&[TextRange]) -> Vec<StmtReport>);

    impl Backend for Fake {
        fn analyse(
            &self,
            _: &str,
            items: &[TextRange],
            _: &ClassifyCtx,
            _: Option<&Lexed>,
        ) -> Vec<StmtReport> {
            (self.0)(items)
        }
    }

    fn item(range: TextRange, effects: EffectSet, class: CoarseClass) -> StmtReport {
        StmtReport {
            range,
            kind: StmtKind::Select,
            effects,
            flags: RiskFlags::empty(),
            class,
            touched: TouchedObjects::default(),
            reasons: Vec::new(),
            destructive: Vec::new(),
            confidence: Confidence::Exact,
            set_targets: Vec::new(),
        }
    }

    fn reads(items: &[TextRange]) -> Vec<StmtReport> {
        items
            .iter()
            .map(|r| item(*r, EffectSet::READ, CoarseClass::Read))
            .collect()
    }

    fn ctx() -> ClassifyCtx {
        ClassifyCtx::new(
            DialectCtx::new(DialectId::Postgres, None),
            ClientFlavor::Psql,
            Expect::Script,
        )
    }

    fn run(sql: &str, f: fn(&[TextRange]) -> Vec<StmtReport>) -> SafetyReport {
        classify_with(sql, &ctx(), Plan::Pg, Some(&Fake(f)))
    }

    fn codes(r: &SafetyReport) -> Vec<ReasonCode> {
        r.pre_decision
            .iter()
            .flatten()
            .chain(r.items.iter().flat_map(|i| &i.reasons))
            .map(|x| x.code)
            .collect()
    }

    #[test]
    fn an_honest_fake_backend_reads() {
        let r = run("SELECT 1; SELECT 2", reads);
        assert_eq!(r.request_class, CoarseClass::Read);
        assert_eq!(r.items.len(), 2);
    }

    #[test]
    fn a_backend_with_too_few_or_too_many_reports_proves_nothing() {
        let too_few: fn(&[TextRange]) -> Vec<StmtReport> = |items| reads(&items[..1]);
        let none: fn(&[TextRange]) -> Vec<StmtReport> = |_| Vec::new();
        let too_many: fn(&[TextRange]) -> Vec<StmtReport> = |items| {
            let mut v = reads(items);
            v.push(v[0].clone());
            v
        };
        for f in [none, too_few, too_many] {
            let r = run("SELECT 1; SELECT 2", f);
            assert_eq!(r.request_class, CoarseClass::Unknown);
            assert!(r.items.is_empty());
            assert_eq!(codes(&r), [ReasonCode::EffectsUnknown]);
        }
    }

    #[test]
    fn a_backend_reporting_other_ranges_proves_nothing() {
        let r = run("SELECT 1; SELECT 2", |items| {
            let mut v = reads(items);
            v.swap(0, 1);
            v
        });
        assert_eq!(r.request_class, CoarseClass::Unknown);
        assert!(r.pre_decision.is_some());
        let r = run("SELECT 1", |items| {
            reads(&[TextRange::new(
                items[0].start(),
                items[0].end() - TextSize::from(1),
            )])
        });
        assert_eq!(r.request_class, CoarseClass::Unknown);
    }

    #[test]
    fn no_backend_runs_after_a_failed_precheck() {
        // A backend that would call everything a read never gets the text.
        fn never(_: &[TextRange]) -> Vec<StmtReport> {
            panic!("a backend ran past a failed precheck")
        }
        for s in ["SELECT 'x", "", "SELECT ${x}", "\\set x 1", "SELECT 1\0"] {
            let r = run(s, never);
            assert!(r.pre_decision.is_some(), "{s:?}");
        }
        let mut c = ctx();
        c.limits.max_nesting = 2;
        assert!(
            classify_with("SELECT ((( 1 )))", &c, Plan::Pg, Some(&Fake(never)))
                .pre_decision
                .is_some()
        );
        // An unsupported plan never reaches a backend either.
        let r = classify_with("SELECT 1", &ctx(), Plan::Unsupported, None);
        assert_eq!(codes(&r), [ReasonCode::UnsupportedDialect]);
    }

    #[test]
    fn producer_contract_violations_never_lower_the_class() {
        fn lying(items: &[TextRange]) -> Vec<StmtReport> {
            vec![item(items[0], EffectSet::WRITE_DATA, CoarseClass::Read)]
        }
        fn empty(items: &[TextRange]) -> Vec<StmtReport> {
            vec![item(items[0], EffectSet::empty(), CoarseClass::Unknown)]
        }
        for f in [lying as fn(&[TextRange]) -> Vec<StmtReport>, empty] {
            let outcome = std::panic::catch_unwind(|| run("SELECT 1", f));
            if cfg!(debug_assertions) {
                // A debug build stops at the contract (the pool answers Unknown for a panic).
                assert!(outcome.is_err());
            } else {
                let r = outcome.unwrap();
                assert_eq!(r.request_class, CoarseClass::Unknown);
                assert!(r.request_effects.contains(EffectSet::UNKNOWN));
            }
        }
    }

    #[test]
    fn contract_enforcement_adds_unknown_evidence() {
        let mut lying = item(
            TextRange::default(),
            EffectSet::WRITE_DATA,
            CoarseClass::Read,
        );
        let mut empty = item(
            TextRange::default(),
            EffectSet::empty(),
            CoarseClass::Unknown,
        );
        for i in [&mut lying, &mut empty] {
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                enforce_producer_contract(i)
            }));
        }
        if !cfg!(debug_assertions) {
            for i in [&lying, &empty] {
                assert_eq!(i.class, CoarseClass::Unknown);
                assert!(i.effects.contains(EffectSet::UNKNOWN));
            }
        }
        // An honest Unknown (a routine call with its reason) passes untouched.
        let mut call = item(
            TextRange::default(),
            EffectSet::ROUTINE_CALL,
            CoarseClass::Unknown,
        );
        call.reasons.push(Reason::new(ReasonCode::ProceduralCode));
        let before = call.clone();
        enforce_producer_contract(&mut call);
        assert_eq!(call, before);
    }

    #[test]
    fn duplicate_reasons_are_dropped_in_order() {
        let mut i = item(TextRange::default(), EffectSet::READ, CoarseClass::Read);
        let a = Reason::new(ReasonCode::FnUnknown);
        let b = Reason::new(ReasonCode::CastUnknown);
        i.reasons = vec![a.clone(), b.clone(), a.clone(), b.clone()];
        dedup_reasons(&mut i);
        assert_eq!(i.reasons, [a, b]);
    }

    #[test]
    fn the_versions_follow_the_plan() {
        let r = run("SELECT 1", reads);
        assert_eq!(
            r.backends,
            SmallVec::<[_; 3]>::from_slice(&[crate::backend::pg::BACKEND])
        );
    }
}
