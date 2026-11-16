//! Execution units (HV-EXEC-01; Task 23): `exec_range::resolve` in execution mode, each unit
//! with its strict text and cursor eligibility; PostgreSQL ambiguity through the oracle when
//! it covers the region exactly (R-M3-24).
//!
//! **What is sent.** A unit's text is `doc.text()[unit.range]`, byte for byte: the strict range
//! `resolve` returned (no terminator, no leading comments). Cursor eligibility reads that same
//! strict range (`head::cursor_eligible`), so a parenthesised `(SELECT …)` is never eligible.
//!
//! **Refusals** (fail closed: nothing is sent):
//! - `Ambiguous` answers [`Resolution::Uncertain`], except on PostgreSQL when the parser oracle
//!   splits every region holding an uncertain boundary exactly (below);
//! - any unit that is `COPY … FROM STDIN` (with data rows, or a `COPY` naming `STDIN` or
//!   `STDOUT`) refuses the whole run (`exec-copy-unsupported`): with `StopOnError` the units
//!   before it would run and the `COPY` then fail, so a script holding one runs nothing.
//!   Which unit is such a `COPY` is the splitter's answer (`UnitKind::CopyIn` and
//!   `UnitKind::CopyStream`, follow-up F1-a), the same one the Run preview reads, so the two
//!   never disagree on a `COPY` (`)COPY t FROM STDIN` is no `COPY` to the splitter or the
//!   server, which rejects it as a syntax error);
//! - a `Verbatim` unit (a selection run as one) is refused until M3b;
//! - a chooser candidate that would be refused is left out of `Ask`.
//!
//! **The PostgreSQL oracle (R-M3-24, R-M3-35).** Only for a single caret (a selection, or
//! several carets, stays uncertain: a selection's start inside a token or a `$$` body must
//! never be rescued by a parser that reads the selected text from a clean state), only on
//! PostgreSQL and Timescale (the same grammar), only while the session's
//! `standard_conforming_strings` is on (libpg_query lexes with it on; under off, `'a\'; …` is
//! one literal to the server and the editor), and never for a boundary from a client command
//! or overlapping units. The region of a boundary is the execution statement whose strict
//! range holds it. Every region must have a lexer parenthesis depth of at most
//! [`MAX_ORACLE_NESTING`] (deeper texts never reach the parser), must hold no `COPY` data nor
//! the word `COPY` (F1-a: the oracle's pieces carry no splitter kind, so a region that may
//! hold a `COPY` stays uncertain), and
//! the oracle's statements must cover it exactly: the first starts at the region's start, the
//! last ends at its end, and only white space and `;` lie between them. Its ranges are
//! relative to the region's text and trimmed, so the region's start is added. Then:
//! - Run 1 / Run 2 at a caret inside a region runs the oracle statement at the caret (the
//!   caret inside it, or after it with only spaces, tabs and `;` between); a caret that is
//!   not inside a region, or a remembered "from the caret" / "whole script", stays uncertain;
//! - Run 3 (whole script) runs every statement in order, each region replaced by its oracle
//!   statements; every other statement must be certain in both splits, and the script must
//!   hold no unsupported client command.
//!
//! The oracle blocks its caller; the caller runs resolution off the UI thread. Oracle requests
//! coalesce latest-wins through an [`OracleLane`]: a request a later resolve on the same lane
//! superseded before its parser job ran is skipped and answers `Uncertain`.

use std::ops::Range;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use sql_dialect::{DialectId, PgWireFlavor};
use sql_lexer::TokenKind;
use sql_oracle_pg::{MAX_ORACLE_NESTING, on_parser_stack, split_with_parser};
use sql_split::exec_range::{
    Candidate, CandidateKind, CaretOrSelection, ExecPolicy, RangeResolution, TextRange, UnitKind,
    UnitSpan, resolve,
};
use sql_split::{AmbiguityReason, AmbiguousBoundary, DocSnapshot, Item, SplitMode, Stmt, head};

/// Which Run the user asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RunAction {
    /// D-011: the statement at the caret (ask when several fit).
    Execute1,
    /// The largest statement at the caret.
    Execute2,
    /// The whole script.
    Execute3,
}

/// The policy of `a`: Execute1 is D-011's default, Execute2 `largest()`, Execute3
/// `whole_script()`.
pub fn policy_for(a: RunAction) -> ExecPolicy {
    match a {
        RunAction::Execute1 => ExecPolicy::default(),
        RunAction::Execute2 => ExecPolicy::largest(),
        RunAction::Execute3 => ExecPolicy::whole_script(),
    }
}

/// One unit to run: the exact text sent, where it is in the document, its kind and whether
/// it may stay suspended as a console cursor (R-M3-23).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ExecUnit {
    pub text: Arc<str>,
    pub doc_range: Range<usize>,
    pub kind: UnitKind,
    pub suspendable: bool,
    // ## m3b-t16-run-service (named exception): the result tabs' title (R-M3-66).
    /// The unit's title: its directly preceding comment without the comment markers
    /// (`UnitSpan::title_comment`, `titles::comment_title`), `None` without one.
    pub title: Option<Arc<str>>,
}

/// Why Run sends nothing although the range is certain.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RefuseReason {
    /// `COPY … FROM STDIN` / `TO STDOUT`: consoles do not stream data (`exec-copy-unsupported`).
    CopyStream,
    /// A selection run as one statement (`UnitKind::Verbatim`): M3b.
    RunAsOne,
}

impl RefuseReason {
    /// The Fluent ID that says why (`hacivat-core.ftl`).
    pub fn fluent_id(self) -> &'static str {
        match self {
            RefuseReason::CopyStream => "exec-copy-unsupported",
            RefuseReason::RunAsOne => "exec-run-as-one-unsupported",
        }
    }
}

/// What Run at a caret or selection does.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Resolution {
    Units(Vec<ExecUnit>),
    Ask {
        candidates: Vec<Candidate>,
        preselect: usize,
    },
    NothingToRun,
    /// A boundary is uncertain: nothing is sent (R-M3-24).
    Uncertain(Vec<AmbiguousBoundary>),
    UnsupportedClientCommand {
        at: Range<usize>,
    },
    /// A unit Run does not send: nothing is sent.
    Refused {
        at: Range<usize>,
        reason: RefuseReason,
    },
}

/// The Fluent ID that explains an uncertain boundary of `reason` (`exec-uncertain-*`), for the
/// chooser's "Boundary uncertain" state and the output log.
pub fn uncertain_message_id(reason: AmbiguityReason) -> &'static str {
    match reason {
        AmbiguityReason::SemicolonInExecutableComment => "exec-uncertain-exec-comment",
        AmbiguityReason::UnterminatedConstruct => "exec-uncertain-unterminated",
        AmbiguityReason::UnknownBlock => "exec-uncertain-unknown-block",
        AmbiguityReason::DelimiterChange => "exec-uncertain-delimiter",
        AmbiguityReason::MissingSemicolon => "exec-uncertain-missing-semicolon",
        AmbiguityReason::AmbiguousCommentEnd => "exec-uncertain-comment-end",
        AmbiguityReason::ClientCommand => "exec-uncertain-client-command",
        AmbiguityReason::OverlappingUnits => "exec-uncertain-overlapping-units",
    }
}

/// PostgreSQL's own split of a text: statement ranges relative to it, trimmed (no trivia, no
/// `;`), or `None` when the parser refuses it or the request was skipped.
pub trait SplitOracle {
    fn split(&self, text: &str) -> Option<Vec<TextRange>>;
}

/// A lane of oracle requests that coalesce latest-wins (one per console, or per caller): each
/// resolve takes a ticket, and a parser job whose ticket is no longer the newest is skipped.
#[derive(Debug, Clone, Default)]
pub struct OracleLane {
    latest: Arc<AtomicU64>,
}

impl OracleLane {
    pub fn new() -> Self {
        OracleLane::default()
    }

    /// [`resolve_units`] on this lane: a later call supersedes this one's oracle requests.
    pub fn resolve(
        &self,
        doc: &DocSnapshot,
        at: &[CaretOrSelection],
        action: RunAction,
        remembered: Option<CandidateKind>,
    ) -> Resolution {
        resolve_units_with(doc, at, action, remembered, &self.request())
    }

    /// A new request on this lane: it supersedes every earlier one.
    pub fn request(&self) -> LaneOracle {
        LaneOracle {
            latest: self.latest.clone(),
            ticket: self.latest.fetch_add(1, Ordering::SeqCst) + 1,
        }
    }
}

/// The parser oracle of one request on a lane: runs on the 16 MiB parser thread, unless a
/// later request on the lane was made before the job started (then `None`: uncertain).
#[derive(Debug)]
pub struct LaneOracle {
    latest: Arc<AtomicU64>,
    ticket: u64,
}

impl SplitOracle for LaneOracle {
    fn split(&self, text: &str) -> Option<Vec<TextRange>> {
        let (latest, ticket, text) = (self.latest.clone(), self.ticket, text.to_owned());
        on_parser_stack(move || {
            if latest.load(Ordering::SeqCst) != ticket {
                return None;
            }
            split_with_parser(&text).ok()
        })
        .flatten()
    }
}

/// The units Run `action` at `at` runs. The caller runs it off the UI thread (it may wait for
/// the parser thread); requests through this function never coalesce ([`OracleLane`] does).
pub fn resolve_units(
    doc: &DocSnapshot,
    at: &[CaretOrSelection],
    action: RunAction,
    remembered: Option<CandidateKind>,
) -> Resolution {
    OracleLane::new().resolve(doc, at, action, remembered)
}

/// [`resolve_units`] with `oracle` as PostgreSQL's split.
pub fn resolve_units_with(
    doc: &DocSnapshot,
    at: &[CaretOrSelection],
    action: RunAction,
    remembered: Option<CandidateKind>,
    oracle: &dyn SplitOracle,
) -> Resolution {
    let policy = policy_for(action);
    match resolve(doc, at, &policy, SplitMode::Execution, None, remembered) {
        RangeResolution::Units(spans) => units_of(doc, &spans),
        RangeResolution::Ask {
            candidates,
            preselect,
        } => ask(doc, candidates, preselect),
        RangeResolution::NothingToRun => Resolution::NothingToRun,
        RangeResolution::Ambiguous(boundaries) => {
            by_oracle(doc, at, action, remembered, &boundaries, oracle)
                .unwrap_or(Resolution::Uncertain(boundaries))
        }
        RangeResolution::UnsupportedClientCommand { at } => {
            Resolution::UnsupportedClientCommand { at: span(at) }
        }
    }
}

/// The units of the chooser's candidate `c`; none when Run would refuse one of them (the
/// chooser never offers such a candidate, [`resolve_units`]).
pub fn units_for_candidate(doc: &DocSnapshot, c: &Candidate) -> Vec<ExecUnit> {
    match units_of(doc, &c.units) {
        Resolution::Units(units) => units,
        _ => Vec::new(),
    }
}

fn span(r: TextRange) -> Range<usize> {
    usize::from(r.start())..usize::from(r.end())
}

/// Why `s` is not sent, if it is not.
// ## m3a-f1-followups (named exception, follow-up F1-a): the splitter's kind decides a `COPY`
// (it was a second, text-based reading of the unit's words that took `)COPY` for one).
fn refusal(s: &UnitSpan) -> Option<RefuseReason> {
    match s.kind {
        UnitKind::Verbatim => Some(RefuseReason::RunAsOne),
        UnitKind::CopyIn { .. } | UnitKind::CopyStream => Some(RefuseReason::CopyStream),
        // ## m3a-f1-followups (fix round 1, M2): no production `SubqueryOracle` exists and nothing
        // builds `Batch` today. A future oracle's range carries no splitter head, so (M3b) it
        // must pass `mentions_copy`, or be split again for its head, before it becomes a unit.
        UnitKind::Statement | UnitKind::Subquery | UnitKind::Batch => None,
    }
}

// ## m3a-f1-followups (named exception, follow-up F1-a)
/// Whether `range` holds the word `COPY` (a word in a string or a comment does not count):
/// such an oracle region stays uncertain, since the oracle's pieces carry no splitter kind.
fn mentions_copy(doc: &DocSnapshot, range: TextRange) -> bool {
    let (lexed, text) = (doc.lexed(), doc.text());
    lexed
        .tokens_in(range)
        .filter(|&i| lexed.kind(i) == TokenKind::Word)
        .filter_map(|i| text.get(span(lexed.range(i))))
        .any(|w| w.eq_ignore_ascii_case("COPY"))
}

/// One unit: its strict text and, for a statement, its cursor eligibility (strict range).
fn unit(doc: &DocSnapshot, range: TextRange, kind: UnitKind) -> ExecUnit {
    ExecUnit {
        text: doc.text()[span(range)].into(),
        doc_range: span(range),
        kind,
        suspendable: kind == UnitKind::Statement && head::cursor_eligible(doc, range),
        // ## m3b-t16-run-service (named exception): set by `units_of` from the span.
        title: None,
    }
}

fn units_of(doc: &DocSnapshot, spans: &[UnitSpan]) -> Resolution {
    if spans.is_empty() {
        return Resolution::NothingToRun;
    }
    if let Some((s, reason)) = spans
        .iter()
        .find_map(|s| refusal(s).map(|reason| (s, reason)))
    {
        return Resolution::Refused {
            at: span(s.range),
            reason,
        };
    }
    // ## m3b-t16-run-service (named exception): each unit's comment title (R-M3-66).
    Resolution::Units(
        spans
            .iter()
            .map(|s| ExecUnit {
                title: s
                    .title_comment
                    .and_then(|t| doc.text().get(span(t)))
                    .and_then(crate::titles::comment_title),
                ..unit(doc, s.range, s.kind)
            })
            .collect(),
    )
}

/// The chooser without the candidates Run would refuse; the preselected one stays selected
/// when it is kept.
fn ask(doc: &DocSnapshot, candidates: Vec<Candidate>, preselect: usize) -> Resolution {
    let mut first_refused = None;
    let mut kept = Vec::with_capacity(candidates.len());
    let mut chosen = 0;
    for (i, c) in candidates.into_iter().enumerate() {
        match units_of(doc, &c.units) {
            Resolution::Refused { at, reason } => {
                first_refused.get_or_insert(Resolution::Refused { at, reason });
            }
            _ => {
                if i == preselect {
                    chosen = kept.len();
                }
                kept.push(c);
            }
        }
    }
    match (kept.is_empty(), first_refused) {
        (true, Some(refused)) => refused,
        (true, None) => Resolution::NothingToRun,
        (false, _) => Resolution::Ask {
            candidates: kept,
            preselect: chosen,
        },
    }
}

/// Whether the oracle may speak for `id`: PostgreSQL's grammar (Timescale is an extension of
/// it). The other wire flavours parse differently.
fn oracle_dialect(id: DialectId) -> bool {
    matches!(
        id,
        DialectId::Postgres | DialectId::PgWire(PgWireFlavor::Timescale)
    )
}

/// The certain statement of `split` with the strict range `strict`, if there is one.
fn certain_in(split: &sql_split::Split, strict: TextRange) -> bool {
    let i = split
        .items
        .partition_point(|it| it.smart().end() < strict.start());
    split.items[i..]
        .iter()
        .take_while(|it| it.smart().start() <= strict.start())
        .filter_map(Item::statement)
        .any(|s| s.strict == strict && split.is_certain(s))
}

/// The oracle's statements for `region`, in document offsets, when they cover it exactly.
fn oracle_pieces(
    doc: &DocSnapshot,
    region: &Stmt,
    oracle: &dyn SplitOracle,
) -> Option<Vec<TextRange>> {
    // ## m3a-f1-followups: nor any region that may hold a `COPY` (F1-a).
    if region.copy_data.is_some() || mentions_copy(doc, region.strict) {
        return None;
    }
    let lexed = doc.lexed();
    if lexed.max_paren_depth(lexed.tokens_in(region.strict)) > MAX_ORACLE_NESTING {
        return None;
    }
    let text = doc.text();
    let pieces: Vec<TextRange> = oracle
        .split(&text[span(region.strict)])?
        .into_iter()
        .map(|r| r + region.strict.start())
        .collect();
    let (first, last) = (pieces.first()?, pieces.last()?);
    if first.start() != region.strict.start() || last.end() != region.strict.end() {
        return None;
    }
    let separators_only = |gap: &str| gap.chars().all(|c| c.is_whitespace() || c == ';');
    for r in &pieces {
        let (a, b) = (usize::from(r.start()), usize::from(r.end()));
        if r.is_empty() || !text.is_char_boundary(a) || !text.is_char_boundary(b) {
            return None;
        }
    }
    for w in pieces.windows(2) {
        if w[1].start() < w[0].end()
            || !separators_only(&text[span(TextRange::new(w[0].end(), w[1].start()))])
        {
            return None;
        }
    }
    Some(pieces)
}

/// R-M3-24: the units the PostgreSQL oracle settles, or `None` (uncertain).
fn by_oracle(
    doc: &DocSnapshot,
    at: &[CaretOrSelection],
    action: RunAction,
    remembered: Option<CandidateKind>,
    boundaries: &[AmbiguousBoundary],
    oracle: &dyn SplitOracle,
) -> Option<Resolution> {
    // libpg_query lexes with `standard_conforming_strings = on`; a session with it off (or not
    // known to be on) reads `'a\'; …` as one literal where the oracle would see a statement.
    if !oracle_dialect(doc.ctx().id)
        || !doc.ctx().modes.standard_conforming_strings
        || boundaries.is_empty()
    {
        return None;
    }
    let caret = match at {
        [CaretOrSelection::Caret(c)] => *c,
        [CaretOrSelection::Selection(r)] if r.is_empty() => r.start(),
        _ => return None,
    };
    if boundaries.iter().any(|b| {
        matches!(
            b.reason,
            AmbiguityReason::ClientCommand | AmbiguityReason::OverlappingUnits
        )
    }) {
        return None;
    }
    let exec = doc.execution_split();
    // The execution statement holding each boundary, in text order, once each.
    let mut regions: Vec<&Stmt> = Vec::new();
    for b in boundaries {
        let holder = exec.statements().find(|s| {
            s.strict.contains_range(b.at) && (!b.at.is_empty() || s.strict.contains(b.at.start()))
        })?;
        if !regions.iter().any(|r| r.strict == holder.strict) {
            regions.push(holder);
        }
    }
    regions.sort_by_key(|r| r.strict.start());
    let mut settled: Vec<(TextRange, Vec<TextRange>)> = Vec::with_capacity(regions.len());
    for r in &regions {
        settled.push((r.strict, oracle_pieces(doc, r, oracle)?));
    }
    let to_units = |ranges: &[TextRange]| -> Resolution {
        let spans: Vec<UnitSpan> = ranges
            .iter()
            .map(|r| UnitSpan {
                range: *r,
                kind: UnitKind::Statement,
                key: None,
                title_comment: None,
            })
            .collect();
        units_of(doc, &spans)
    };
    match action {
        RunAction::Execute1 | RunAction::Execute2 => {
            if matches!(
                remembered,
                Some(CandidateKind::FromCaret | CandidateKind::WholeScript)
            ) {
                return None;
            }
            let (_, pieces) = settled.iter().find(|(strict, _)| {
                regions
                    .iter()
                    .find(|r| r.strict == *strict)
                    .is_some_and(|r| r.smart.contains_inclusive(caret))
            })?;
            let at_caret = pieces.iter().rev().find(|p| p.start() <= caret)?;
            let text = doc.text();
            let after =
                text.get(usize::from(at_caret.end())..usize::from(caret.max(at_caret.end())))?;
            let blank = after.chars().all(|c| c == ' ' || c == '\t' || c == ';');
            (caret <= at_caret.end() || blank).then(|| to_units(&[*at_caret]))
        }
        RunAction::Execute3 => {
            if exec.client.iter().any(|c| !c.supported) {
                return None;
            }
            let ranges = whole_script(exec, doc.editor_split(), &settled)?;
            Some(to_units(&ranges))
        }
    }
}

/// Run 3 with the oracle: every execution statement in order, each settled region replaced by
/// its oracle statements; any other statement must be certain in both splits (R-M3-8: Run never
/// sends a range the preview drew as uncertain).
fn whole_script(
    exec: &sql_split::Split,
    editor: &sql_split::Split,
    settled: &[(TextRange, Vec<TextRange>)],
) -> Option<Vec<TextRange>> {
    let mut ranges = Vec::new();
    for s in exec.statements() {
        match settled.iter().find(|(strict, _)| *strict == s.strict) {
            Some((_, pieces)) => ranges.extend_from_slice(pieces),
            None if certain_in(exec, s.strict) && certain_in(editor, s.strict) => {
                ranges.push(s.strict);
            }
            None => return None,
        }
    }
    (!ranges.is_empty()).then_some(ranges)
}

#[cfg(test)]
mod tests {
    use sql_split::{Confidence, Item, Split, Stmt, StmtHead, StmtKey};
    use sql_text::{TextRange, TextSize};

    use super::whole_script;

    fn r(a: u32, b: u32) -> TextRange {
        TextRange::new(TextSize::from(a), TextSize::from(b))
    }

    fn stmt(range: TextRange, confidence: Confidence) -> Item {
        Item::Statement(Stmt {
            strict: range,
            smart: range,
            terminator: None,
            leading_comments: None,
            confidence,
            head: StmtHead::default(),
            key: StmtKey(0),
            copy_data: None,
        })
    }

    fn split(items: Vec<Item>) -> Split {
        Split {
            items,
            ..Split::default()
        }
    }

    /// Review I3 (2): a statement certain in the execution split but heuristic in the editor's
    /// (what the preview draws) is never part of a whole-script run the oracle settles. Real
    /// texts cannot build it today (both splits list the same boundaries, sql-split's R-M3-8
    /// rule), so the splits are built here.
    #[test]
    fn whole_script_refuses_a_statement_the_editor_split_finds_uncertain() {
        let settled = vec![(r(0, 20), vec![r(0, 8), r(10, 20)])];
        let exec = split(vec![
            stmt(r(0, 20), Confidence::Heuristic),
            stmt(r(22, 30), Confidence::Exact),
        ]);
        let editor_certain = split(vec![
            stmt(r(0, 8), Confidence::Heuristic),
            stmt(r(10, 20), Confidence::Heuristic),
            stmt(r(22, 30), Confidence::Exact),
        ]);
        assert_eq!(
            whole_script(&exec, &editor_certain, &settled),
            Some(vec![r(0, 8), r(10, 20), r(22, 30)])
        );
        let editor_uncertain = split(vec![
            stmt(r(0, 8), Confidence::Heuristic),
            stmt(r(10, 20), Confidence::Heuristic),
            stmt(r(22, 30), Confidence::Heuristic),
        ]);
        assert_eq!(whole_script(&exec, &editor_uncertain, &settled), None);
        // And one the editor split does not hold at all.
        let editor_missing = split(vec![stmt(r(0, 20), Confidence::Heuristic)]);
        assert_eq!(whole_script(&exec, &editor_missing, &settled), None);
    }
}
