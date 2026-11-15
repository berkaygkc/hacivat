//! Statement spans from the editor-mode split, rebased (Task 24).
//!
//! The worker turns the editor split into `StatementSpan`s once per analysis. A statement is
//! `Exact` only when execution would send it as it is: `Exact` in the editor split with no
//! uncertain boundary inside, and the same certain statement in the execution split (the
//! agreement `exec_range` uses, R-M3-8). When the editor is ahead of the analysis, the spans
//! are mapped through the changes since as containers (typing at an edge grows them), and
//! every statement from the earliest change on is `Heuristic` until the next analysis: lexer
//! state flows forward, so a quote or `/*` typed above a statement may have turned it into
//! string or comment text (review M2).

use std::ops::Range;
use std::sync::Arc;

use hacivat_text::{Bias, MapMode, RangeKind};
use hacivat_ui_editor::{Confidence, StatementSpan};
use sql_split::{DocSnapshot, Item, Split};
use sql_text::TextRange;

use crate::highlight::back_map;
use crate::service::DocShared;

fn range(r: TextRange) -> Range<usize> {
    usize::from(r.start())..usize::from(r.end())
}

/// Whether the execution split holds a certain statement with exactly the strict range
/// `strict` (`exec_range`'s `View::certain`, second half). Under Task 13's agreement
/// (`editor_and_execution_agree_on_exact_items`) an editor-certain statement always has one, so
/// this check is likely redundant (dropping it is an equivalent mutant: T24 M8); it is kept as
/// defence in depth.
fn certain_in_execution(exec: &Split, strict: TextRange) -> bool {
    let i = exec
        .items
        .partition_point(|it| it.smart().end() < strict.start());
    exec.items[i..]
        .iter()
        .take_while(|it| it.smart().start() <= strict.start())
        .filter_map(Item::statement)
        .any(|x| x.strict == strict && exec.is_certain(x))
}

/// The editor split's statements as the gutter draws them (worker side).
pub fn spans(doc: &DocSnapshot) -> Vec<StatementSpan> {
    let (editor, exec) = (doc.editor_split(), doc.execution_split());
    editor
        .statements()
        .map(|st| {
            let certain = editor.is_certain(st) && certain_in_execution(exec, st.strict);
            StatementSpan {
                smart: range(st.smart),
                strict: range(st.strict),
                header: usize::from(st.head.first_token.map_or(st.strict.start(), |t| t.start())),
                confidence: if certain {
                    Confidence::Exact
                } else {
                    Confidence::Heuristic
                },
            }
        })
        .collect()
}

/// `x` is in view of `r` (an empty `r` is a point: the statements holding it, ends included).
fn in_view(x: &Range<usize>, r: &Range<usize>) -> bool {
    if r.is_empty() {
        x.start <= r.start && r.start <= x.end
    } else {
        x.start < r.end && r.start < x.end
    }
}

/// The statements of a sorted list whose smart ranges are in view of `r`.
fn select<'a>(
    list: &'a [StatementSpan],
    r: &Range<usize>,
) -> impl Iterator<Item = &'a StatementSpan> {
    let from = list.partition_point(|s| s.smart.end < r.start);
    list[from..]
        .iter()
        .take_while(move |s| s.smart.start <= r.end)
        .filter(move |s| in_view(&s.smart, r))
}

/// The statements in view of `r` at `version` (module docs); empty when the analysis is newer
/// than `version` or the changes since it are no longer known.
pub(crate) fn at(sh: &DocShared, version: u64, r: Range<usize>) -> Arc<[StatementSpan]> {
    let Some(a) = sh.latest() else {
        return Arc::from(Vec::new());
    };
    if let Some(hit) = sh.cached_statements(&a, version, &r) {
        return hit;
    }
    // Under an older SQL context nothing is certain.
    let stale = a.generation != sh.generation();
    let demote = |mut s: StatementSpan| {
        if stale {
            s.confidence = Confidence::Heuristic;
        }
        s
    };
    let out: Vec<StatementSpan> = if a.version == version {
        select(&a.statements, &r).cloned().map(demote).collect()
    } else if a.version > version {
        Vec::new()
    } else {
        sh.with_descs(a.version, version, |descs| {
            let old = descs.iter().rev().fold(r.clone(), |r, d| back_map(r, d));
            select(&a.statements, &old)
                .filter_map(|s| {
                    let mut s = s.clone();
                    for d in descs {
                        s.smart = d.map_range(s.smart.clone(), RangeKind::Container)?;
                        s.strict = d.map_range(s.strict.clone(), RangeKind::Container)?;
                        s.header = d
                            .map_pos(s.header, Bias::After, MapMode::Simple)?
                            .clamp(s.strict.start, s.strict.end);
                        // Touched, or after the change: the lexer state there may differ.
                        if let Some(ch) = d.changed_range()
                            && ch.start <= s.smart.end
                        {
                            s.confidence = Confidence::Heuristic;
                        }
                    }
                    in_view(&s.smart, &r).then(|| demote(s))
                })
                .collect()
        })
        .unwrap_or_default()
    };
    let out: Arc<[StatementSpan]> = Arc::from(out);
    sh.cache_statements(a, version, r, out.clone());
    out
}
