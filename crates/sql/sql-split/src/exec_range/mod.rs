//! Caret or selection to execution units, with the D-011 policies and chooser candidates
//! (SQL-SPLIT-04 aligned to D-011, R-M3-8; Task 13).
//!
//! One function, [`resolve`], serves the Run preview (`SplitMode::Editor`) and execution
//! (`SplitMode::Execution`): they agree on every exact statement, and execution never returns
//! a unit that crosses a heuristic boundary (an uncertain one is `Ambiguous`).
//!
//! **One agreed split (R-M3-8).** Both modes read the same statements: the editor's split,
//! which is what the preview draws. They differ in one thing only: execution refuses a unit
//! that is not certain, where the preview returns it to be drawn as uncertain. A unit is
//! certain when its statement is `Exact` in the editor's split and the execution split holds
//! a certain statement with the same strict range. So what Run sends is always a range the
//! preview showed as exact, and an uncertain preview never runs: Run asks.
//!
//! **What a unit is.** Its range is the statement's strict range: the text sent, with no
//! terminator and no leading comments (those are the unit's `title_comment`).
//!
//! **The statement at a caret** (`alg/10` SQL-SPLIT-04 step 2), in this order:
//! 1. the statement whose smart range holds the caret, ends included: its attached leading
//!    comments (no blank line between), its terminator, and a comment after the terminator on
//!    the same line (the split layer's ownership, SQL-SPLIT-02). So a caret right after the
//!    `;`, at the statement's first character, inside a string or a dollar-quoted body is
//!    inside. Where two statements touch (`SELECT 1;┃SELECT 2;`) the earlier one is meant;
//! 2. a client command Hacivat does not run (`\echo`, `source`, `.mode`) at the caret is
//!    `UnsupportedClientCommand`;
//! 3. a caret after a statement on its line, with only spaces or tabs between them and
//!    nothing but white space after it on that line, means that statement;
//! 4. otherwise the caret is outside every statement (a blank line, a comment set apart by a
//!    blank line, an empty `;`, a `DELIMITER` line) and the outside policy decides.
//!
//! **Refusals** (fail closed; nothing is sent):
//! - Execution mode answers `Ambiguous` when a unit it would return is not certain
//!   ([`Split::is_certain`] in both splits). Under `Ask`, an uncertain statement at the caret is `Ambiguous`,
//!   and so is a refused candidate the chooser would open on (the remembered kind): Run
//!   never runs a smaller range in its place. Any other candidate it would refuse is left
//!   out, so the chooser offers only what can run.
//! - Editor mode returns heuristic statements as units: the preview draws them as uncertain.
//! - A run of several statements (whole script, from the caret, below the caret, several
//!   carets, an expanded selection) that spans an unsupported client command answers
//!   `UnsupportedClientCommand` in both modes: Hacivat would not run the script as its client
//!   does. A whole-script run spans the whole text.
//! - A selection has one answer for both modes, and it never holds a heuristic statement.
//!   `AsSeparate` splits it on its own from the lexer state and client delimiter at its
//!   start. Any uncertain boundary or unsupported client command in it refuses the whole
//!   selection. A selection that starts inside a token is refused: the server reads the sent
//!   text from a clean state, so the rest of a string, a quoted name, a dollar-quoted body or
//!   `COPY` data would run as code. (The rest of a plain comment is trivia and never sent;
//!   inside an executable comment or an optimizer hint the server reads code, so that is
//!   refused too.) A selection that starts inside a client command's line is
//!   `UnsupportedClientCommand`.
//! - `AsOne` (and `SmartExpand` without a boundary inside) returns one `Verbatim` unit, the
//!   selection without its outer white space, only when that text, read alone, is certain and
//!   holds no client command and no `COPY` data. No M3a Run action chooses it.
//!
//! **Several carets** each resolve as a statement (`Ask` runs the statement: a chooser per
//! caret has no meaning); the units are united by range, in text order.

mod candidates;
mod policy;

use sql_lexer::LexState;
/// The offsets and ranges this API speaks in (`sql-text`'s), for callers that reach the text
/// layer only through this crate.
pub use sql_text::{TextRange, TextSize};

pub use candidates::{
    Candidate, CandidateKind, CaretOrSelection, LABEL_MAX_CHARS, RangeResolution, SubqueryOracle,
    UnitKind, UnitSpan,
};
pub use policy::{ExecPolicy, InsidePolicy, OutsidePolicy, SelectionPolicy};

use crate::document::DocSnapshot;
use crate::item::{
    AmbiguityReason, AmbiguousBoundary, ClientSegment, Item, Split, SplitMode, Stmt, overlaps,
};
use crate::splitter::{SplitStart, size, split_from};
use candidates::{dedupe, label_for, preselect};

/// What `at` runs under `policy` in `mode` (Task 13); `remembered` is the console's last
/// chosen kind, preselected in the chooser.
///
/// Pure and deterministic; O(statements) at worst (a whole-script candidate lists every
/// unit). It blocks on nothing, but the caller keeps it off the UI thread for large texts.
pub fn resolve(
    doc: &DocSnapshot,
    at: &[CaretOrSelection],
    policy: &ExecPolicy,
    mode: SplitMode,
    subq: Option<&dyn SubqueryOracle>,
    remembered: Option<CandidateKind>,
) -> RangeResolution {
    match at {
        [] => RangeResolution::NothingToRun,
        [one] => resolve_one(doc, *one, policy, mode, subq, remembered),
        many => resolve_many(doc, many, policy, mode, subq),
    }
}

fn resolve_one(
    doc: &DocSnapshot,
    at: CaretOrSelection,
    policy: &ExecPolicy,
    mode: SplitMode,
    subq: Option<&dyn SubqueryOracle>,
    remembered: Option<CandidateKind>,
) -> RangeResolution {
    match at {
        CaretOrSelection::Selection(r) if !r.is_empty() => selection(doc, r, policy),
        CaretOrSelection::Selection(r) => caret(doc, r.start(), policy, mode, subq, remembered),
        CaretOrSelection::Caret(c) => caret(doc, c, policy, mode, subq, remembered),
    }
}

/// Several carets or selections: each resolves on its own (a caret under `Ask` as its
/// statement), and the units are united by range in text order.
fn resolve_many(
    doc: &DocSnapshot,
    many: &[CaretOrSelection],
    policy: &ExecPolicy,
    mode: SplitMode,
    subq: Option<&dyn SubqueryOracle>,
) -> RangeResolution {
    let mut each = *policy;
    if each.inside == InsidePolicy::Ask {
        each.inside = InsidePolicy::SmallestStatement;
    }
    let (mut units, mut ambiguous) = (Vec::new(), Vec::new());
    for at in many {
        match resolve_one(doc, *at, &each, mode, subq, None) {
            RangeResolution::Units(u) => units.extend(u),
            RangeResolution::NothingToRun => {}
            RangeResolution::Ambiguous(b) => ambiguous.extend(b),
            refused @ RangeResolution::UnsupportedClientCommand { .. } => return refused,
            // No policy asks here; were one to, nothing runs.
            RangeResolution::Ask { .. } => return RangeResolution::NothingToRun,
        }
    }
    if !ambiguous.is_empty() {
        return RangeResolution::Ambiguous(sorted(ambiguous));
    }
    units.sort_by_key(|u: &UnitSpan| (u.range.start(), u.range.end()));
    units.dedup_by_key(|u| u.range);
    let (Some(first), Some(last)) = (units.first(), units.last()) else {
        return RangeResolution::NothingToRun;
    };
    if units.len() > 1 {
        let span = TextRange::new(first.range.start(), last.range.end().max(first.range.end()));
        if let Some(at) = View::new(doc, mode).unsupported_in(span) {
            return RangeResolution::UnsupportedClientCommand { at };
        }
    }
    RangeResolution::Units(units)
}

/// Appends the boundaries of `split` that start inside `strict` (every boundary starts
/// inside the one statement it makes uncertain; `ambiguous` is sorted by start).
fn starting_in(split: &Split, strict: TextRange, out: &mut Vec<AmbiguousBoundary>) {
    let all = &split.ambiguous;
    let from = all.partition_point(|a| a.at.start() < strict.start());
    out.extend(
        all[from..]
            .iter()
            .take_while(|a| a.at.start() < strict.end())
            .copied(),
    );
}

fn sorted(mut b: Vec<AmbiguousBoundary>) -> Vec<AmbiguousBoundary> {
    b.sort_by_key(|a| (a.at.start(), a.at.end(), a.reason as u8));
    b.dedup();
    b
}

/// `at` inside `text`, moved back to a character boundary.
fn clamp(text: &str, at: TextSize) -> TextSize {
    let mut i = usize::from(at).min(text.len());
    while !text.is_char_boundary(i) {
        i -= 1;
    }
    size(i)
}

fn whole(text: &str) -> TextRange {
    TextRange::up_to(size(text.len()))
}

/// A document's statements as the preview draws them (the editor's split), and what `mode`
/// refuses of them.
struct View<'a> {
    doc: &'a DocSnapshot,
    split: &'a Split,
    mode: SplitMode,
}

impl<'a> View<'a> {
    fn new(doc: &'a DocSnapshot, mode: SplitMode) -> Self {
        View {
            doc,
            split: doc.editor_split(),
            mode,
        }
    }

    /// Whether execution may send `s`: `Exact` in the editor's split with no uncertain
    /// boundary inside, and the same certain statement in the execution split.
    fn certain(&self, s: &Stmt) -> bool {
        if !self.split.is_certain(s) {
            return false;
        }
        let exec = self.doc.execution_split();
        let i = exec
            .items
            .partition_point(|it| it.smart().end() < s.strict.start());
        exec.items[i..]
            .iter()
            .take_while(|it| it.smart().start() <= s.strict.start())
            .filter_map(Item::statement)
            .any(|x| x.strict == s.strict && exec.is_certain(x))
    }

    fn stmts(&self) -> Vec<&'a Stmt> {
        self.split.statements().collect()
    }

    /// The first client command inside `span` that Hacivat does not run.
    fn unsupported_in(&self, span: TextRange) -> Option<TextRange> {
        self.split
            .client
            .iter()
            .find(|c| !c.supported && overlaps(c.range, span))
            .map(|c| c.range)
    }

    /// The uncertain boundaries of the statements of `stmts` that are not certain: the ones
    /// inside each, else the ones of the execution statements it overlaps (a heuristic part
    /// of the editor's is a piece of one uncertain execution statement).
    fn boundaries(&self, stmts: &[&Stmt]) -> Vec<AmbiguousBoundary> {
        let mut out = Vec::new();
        for s in stmts {
            if self.certain(s) {
                continue;
            }
            let before = out.len();
            starting_in(self.split, s.strict, &mut out);
            if out.len() == before {
                let exec = self.doc.execution_split();
                let from = exec
                    .items
                    .partition_point(|it| it.smart().end() < s.strict.start());
                for x in exec.items[from..]
                    .iter()
                    .take_while(|it| it.smart().start() <= s.strict.end())
                    .filter_map(Item::statement)
                    .filter(|x| overlaps(x.strict, s.strict))
                {
                    starting_in(exec, x.strict, &mut out);
                }
            }
            if out.len() == before {
                // Uncertain with no boundary listed: still refused, at the statement.
                out.push(AmbiguousBoundary {
                    at: s.strict,
                    reason: AmbiguityReason::UnterminatedConstruct,
                });
            }
        }
        sorted(out)
    }

    /// Why `stmts` do not run, if they do not: an unsupported client command inside `span`
    /// (a run of several statements names its span), or, in execution mode, an uncertain
    /// statement.
    fn refusal(&self, stmts: &[&Stmt], span: Option<TextRange>) -> Option<RangeResolution> {
        if let Some(at) = span.and_then(|s| self.unsupported_in(s)) {
            return Some(RangeResolution::UnsupportedClientCommand { at });
        }
        if self.mode == SplitMode::Execution {
            let b = self.boundaries(stmts);
            if !b.is_empty() {
                return Some(RangeResolution::Ambiguous(b));
            }
        }
        None
    }

    /// `stmts` as units, or why they do not run.
    fn units(&self, stmts: &[&Stmt], span: Option<TextRange>) -> RangeResolution {
        if stmts.is_empty() {
            return RangeResolution::NothingToRun;
        }
        match self.refusal(stmts, span) {
            Some(refused) => refused,
            None => RangeResolution::Units(stmts.iter().map(|s| UnitSpan::of(s)).collect()),
        }
    }

    /// The statement whose smart range holds `caret` (the earlier one where two touch).
    fn holding(&self, caret: TextSize) -> Option<&'a Stmt> {
        let items = &self.split.items;
        let first = items.partition_point(|it| it.smart().end() < caret);
        items[first..]
            .iter()
            .take_while(|it| it.smart().start() <= caret)
            .find_map(|it| match it {
                Item::Statement(s) if s.smart.contains_inclusive(caret) => Some(s),
                _ => None,
            })
    }

    /// The statement that ends before `caret` on the caret's line, with only spaces or tabs
    /// between them and only white space after the caret on that line.
    fn trailed(&self, caret: TextSize) -> Option<&'a Stmt> {
        let items = &self.split.items;
        let next = items.partition_point(|it| it.smart().end() < caret);
        let Item::Statement(s) = items.get(next.checked_sub(1)?)? else {
            return None;
        };
        let text = self.doc.text();
        let between = text.get(usize::from(s.smart.end())..usize::from(caret))?;
        let rest = &text[usize::from(caret)..];
        let line_rest = rest.split(['\n', '\r']).next().unwrap_or_default();
        let blank = |t: &str| t.chars().all(|c| c == ' ' || c == '\t');
        (blank(between) && line_rest.trim().is_empty()).then_some(s)
    }

    /// The runnable subquery at `caret` inside `stmt`, when an oracle names a valid one.
    fn subquery(
        &self,
        stmt: &Stmt,
        caret: TextSize,
        subq: Option<&dyn SubqueryOracle>,
    ) -> Option<UnitSpan> {
        let r = subq?.innermost_runnable_subquery(self.doc, stmt, caret)?;
        let text = self.doc.text();
        let valid = !r.is_empty()
            && stmt.strict.contains_range(r)
            && text.is_char_boundary(usize::from(r.start()))
            && text.is_char_boundary(usize::from(r.end()));
        valid.then_some(UnitSpan {
            range: r,
            kind: UnitKind::Subquery,
            key: None,
            title_comment: None,
        })
    }

    /// The chooser at the statement `stmts[i]` (HV-EXEC-02): candidates in resolve order,
    /// duplicates removed; one candidate runs directly (D-011).
    fn ask(
        &self,
        stmts: &[&'a Stmt],
        i: usize,
        caret: TextSize,
        subq: Option<&dyn SubqueryOracle>,
        remembered: Option<CandidateKind>,
    ) -> RangeResolution {
        let text = self.doc.text();
        let stmt = stmts[i];
        if let Some(refused) = self.refusal(&[stmt], None) {
            return refused;
        }
        let units = |s: &[&Stmt]| -> Vec<UnitSpan> { s.iter().map(|s| UnitSpan::of(s)).collect() };
        let mut tried = Vec::with_capacity(5);
        if let Some(u) = self.subquery(stmt, caret, subq) {
            tried.push((CandidateKind::Subquery, vec![u]));
        }
        tried.push((CandidateKind::Statement, vec![UnitSpan::of(stmt)]));
        tried.push((CandidateKind::LargestOrBatch, vec![UnitSpan::of(stmt)]));
        // From the first statement, "from the caret" is the whole script.
        let from_caret = TextRange::new(stmt.smart.start(), whole(text).end());
        if i > 0 {
            tried.push((CandidateKind::FromCaret, units(&stmts[i..])));
        }
        tried.push((CandidateKind::WholeScript, units(stmts)));
        let kept = dedupe(tried);
        let kinds: Vec<CandidateKind> = kept.iter().map(|(k, _)| *k).collect();
        let chosen = kinds[preselect(&kinds, remembered)];
        // Why each candidate would not run. Only a run of several statements can be refused
        // here: the statement at the caret is not.
        let refusal = |kind: CandidateKind| match kind {
            CandidateKind::FromCaret => self.refusal(&stmts[i..], Some(from_caret)),
            CandidateKind::WholeScript => self.refusal(stmts, Some(whole(text))),
            _ => None,
        };
        // The candidate the chooser would open on does not run: Run says why, and never
        // runs a smaller range in its place.
        if let Some(refused) = refusal(chosen) {
            return refused;
        }
        let mut candidates: Vec<Candidate> = kept
            .into_iter()
            .filter(|(kind, _)| refusal(*kind).is_none())
            .map(|(kind, units)| Candidate {
                kind,
                label: label_for(text, &units),
                units,
            })
            .collect();
        if candidates.len() == 1 {
            return RangeResolution::Units(candidates.remove(0).units);
        }
        RangeResolution::Ask {
            preselect: candidates
                .iter()
                .position(|c| c.kind == chosen)
                .unwrap_or(0),
            candidates,
        }
    }
}

fn caret(
    doc: &DocSnapshot,
    caret: TextSize,
    policy: &ExecPolicy,
    mode: SplitMode,
    subq: Option<&dyn SubqueryOracle>,
    remembered: Option<CandidateKind>,
) -> RangeResolution {
    let view = View::new(doc, mode);
    let text = doc.text();
    let caret = clamp(text, caret);
    let stmts = view.stmts();

    let mut stmt = view.holding(caret);
    if stmt.is_none() {
        let on_command = view
            .split
            .client
            .iter()
            .find(|c| !c.supported && c.range.contains_inclusive(caret));
        if let Some(c) = on_command {
            return RangeResolution::UnsupportedClientCommand { at: c.range };
        }
        stmt = view.trailed(caret);
    }

    let Some(stmt) = stmt else {
        return match policy.outside {
            OutsidePolicy::Nothing => RangeResolution::NothingToRun,
            OutsidePolicy::WholeScript => view.units(&stmts, Some(whole(text))),
            OutsidePolicy::EverythingBelow => {
                let from = stmts.partition_point(|s| s.strict.start() < caret);
                let span = TextRange::new(caret, whole(text).end());
                view.units(&stmts[from..], Some(span))
            }
        };
    };
    let i = stmts.partition_point(|s| s.strict.start() < stmt.strict.start());
    match policy.inside {
        InsidePolicy::Ask => view.ask(&stmts, i, caret, subq, remembered),
        InsidePolicy::SmallestSubqueryOrStatement => {
            match (
                view.refusal(&[stmt], None),
                view.subquery(stmt, caret, subq),
            ) {
                (Some(refused), _) => refused,
                (None, Some(unit)) => RangeResolution::Units(vec![unit]),
                (None, None) => view.units(&[stmt], None),
            }
        }
        InsidePolicy::SmallestStatement
        | InsidePolicy::LargestStatement
        | InsidePolicy::LargestStatementOrBatch => view.units(&[stmt], None),
        InsidePolicy::WholeScript => view.units(&stmts, Some(whole(text))),
        InsidePolicy::EverythingFromCaret => {
            let span = TextRange::new(stmt.smart.start(), whole(text).end());
            view.units(&stmts[i..], Some(span))
        }
    }
}

fn selection(doc: &DocSnapshot, r: TextRange, policy: &ExecPolicy) -> RangeResolution {
    let text = doc.text();
    let end = size(text.len());
    let r = TextRange::new(r.start().min(end), r.end().min(end));
    // A selection that splits a character is no text: nothing runs.
    let Some(piece) = text.get(usize::from(r.start())..usize::from(r.end())) else {
        return RangeResolution::NothingToRun;
    };
    if piece.trim().is_empty() {
        return RangeResolution::NothingToRun;
    }
    match policy.selection {
        SelectionPolicy::AsSeparate => separate(doc, r),
        SelectionPolicy::AsOne => verbatim(doc, r),
        SelectionPolicy::SmartExpand => {
            // Like every selection, read through execution's split in both modes: a
            // selection has one answer, and it never holds a heuristic statement.
            let view = View::new(doc, SplitMode::Execution);
            let stmts = view.stmts();
            let a = stmts.partition_point(|s| s.strict.end() <= r.start());
            let b = stmts.partition_point(|s| s.strict.start() < r.end());
            if b.saturating_sub(a) >= 2 {
                let span = TextRange::new(stmts[a].smart.start(), stmts[b - 1].smart.end());
                view.units(&stmts[a..b], Some(span))
            } else {
                verbatim(doc, r)
            }
        }
    }
}

fn shifted(b: &[AmbiguousBoundary], by: TextSize) -> Vec<AmbiguousBoundary> {
    b.iter()
        .map(|a| AmbiguousBoundary {
            at: a.at + by,
            ..*a
        })
        .collect()
}

fn refused_at(at: TextRange) -> RangeResolution {
    RangeResolution::Ambiguous(vec![AmbiguousBoundary {
        at,
        reason: AmbiguityReason::UnterminatedConstruct,
    }])
}

/// The selection `r` as separate statements (D-011), split from the state at its start.
fn separate(doc: &DocSnapshot, r: TextRange) -> RangeResolution {
    // Inside a client command's line the text is that command's argument, not SQL.
    let inside = |c: &&ClientSegment| c.range.start() < r.start() && r.start() < c.range.end();
    if let Some(c) = doc.editor_split().client.iter().find(inside) {
        return RangeResolution::UnsupportedClientCommand { at: c.range };
    }
    // The server reads what is sent from a clean state. A selection that starts inside a
    // token would send that token's tail as code: a string's or a quoted name's rest, a
    // dollar-quoted body, `COPY` data. In a plain comment the tail is trivia and never sent;
    // in an executable comment or a hint the server reads code. `split_selection` marks
    // such a tail uncertain itself; this refusal does not depend on it.
    let start = doc.split_start_at(r.start());
    let reason = match start.lex {
        LexState::Initial | LexState::InBlockComment { .. } => None,
        LexState::InExecutableComment { .. } | LexState::InHintComment { .. } => {
            Some(AmbiguityReason::AmbiguousCommentEnd)
        }
        _ => Some(AmbiguityReason::UnterminatedConstruct),
    };
    if let Some(reason) = reason {
        let lexed = doc.lexed();
        let tail = lexed
            .token_at(r.start())
            .map_or(r.end(), |i| lexed.range(i).end().min(r.end()));
        return RangeResolution::Ambiguous(vec![AmbiguousBoundary {
            at: TextRange::new(r.start(), tail.max(r.start())),
            reason,
        }]);
    }
    let split = doc.split_selection(r, SplitMode::Execution);
    if let Some(c) = split.client.iter().find(|c| !c.supported) {
        return RangeResolution::UnsupportedClientCommand { at: c.range };
    }
    if !split.ambiguous.is_empty() {
        return RangeResolution::Ambiguous(split.ambiguous.clone());
    }
    let stmts: Vec<&Stmt> = split.statements().collect();
    if let Some(s) = stmts.iter().find(|s| !split.is_certain(s)) {
        return refused_at(s.strict);
    }
    if stmts.is_empty() {
        return RangeResolution::NothingToRun;
    }
    RangeResolution::Units(stmts.iter().map(|s| UnitSpan::of(s)).collect())
}

/// The selection `r` as one unit sent as written: its text without the outer white space,
/// when that text read alone is certain and holds nothing a server cannot take.
fn verbatim(doc: &DocSnapshot, r: TextRange) -> RangeResolution {
    let piece = &doc.text()[r];
    let lead = piece.len() - piece.trim_start().len();
    let trimmed = piece.trim();
    let by = r.start() + size(lead);
    let range = TextRange::at(by, size(trimmed.len()));
    let clean = SplitStart {
        lex: LexState::Initial,
        delimiter: doc.split_start_at(by).delimiter,
    };
    let alone = split_from(
        trimmed,
        doc.ctx(),
        doc.flavor(),
        SplitMode::Execution,
        &clean,
    );
    // Sent as written, a client command or COPY data would reach the server as SQL.
    if let Some(c) = alone.client.first() {
        return RangeResolution::UnsupportedClientCommand { at: c.range + by };
    }
    if let Some((s, data)) = alone.statements().find_map(|s| s.copy_data.map(|d| (s, d))) {
        return RangeResolution::UnsupportedClientCommand {
            at: TextRange::new(s.strict.start(), data.rows.end().max(s.strict.end())) + by,
        };
    }
    if !alone.ambiguous.is_empty() {
        return RangeResolution::Ambiguous(shifted(&alone.ambiguous, by));
    }
    if let Some(s) = alone.statements().find(|s| !alone.is_certain(s)) {
        return refused_at(s.strict + by);
    }
    if alone.statements().next().is_none() {
        return RangeResolution::NothingToRun;
    }
    RangeResolution::Units(vec![UnitSpan {
        range,
        kind: UnitKind::Verbatim,
        key: None,
        title_comment: None,
    }])
}
