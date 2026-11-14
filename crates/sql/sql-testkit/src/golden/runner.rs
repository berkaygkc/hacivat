//! Running a golden case through `resolve` and diffing the result (Task 13).
//!
//! Every case resolves in execution mode. When the whole document is certain it resolves in
//! editor mode too, and the two answers must be equal (R-M3-8: the preview shows what Run
//! runs).

use sql_dialect::{ClientFlavor, DialectId};
use sql_split::exec_range::{
    Candidate, CandidateKind, CaretOrSelection, ExecPolicy, InsidePolicy, OutsidePolicy,
    RangeResolution, SelectionPolicy, TextRange, TextSize, UnitKind, UnitSpan, resolve,
};
use sql_split::{DocSnapshot, SplitMode, SqlDocument, Stmt};

use super::markers::extract;
use super::{Case, Expect};
use crate::lex::dialect_ctx;
use crate::split::{fixture, flavor};

/// How a case ran.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Its `expect` held.
    Ran,
    /// It needs a parse tree (`requires = ["cst"]`): its `expect_without_cst` held.
    RanWithoutCst,
    /// It is skipped, with its reason (one of its `requires`).
    Skipped(String),
}

/// Runs `case`: `Err` names the case and what differs.
///
/// # Errors
/// When the case is malformed, or `resolve` answers something else than it expects.
pub fn run(case: &Case) -> Result<Outcome, String> {
    run_inner(case).map_err(|e| format!("{}: {e}", case.id))
}

fn size(n: usize) -> TextSize {
    TextSize::try_from(n).expect("a corpus text is far below 4 GiB")
}

fn range(r: &std::ops::Range<usize>) -> TextRange {
    TextRange::new(size(r.start), size(r.end))
}

fn inside(name: &str) -> Result<InsidePolicy, String> {
    Ok(match name {
        "ask" => InsidePolicy::Ask,
        "smallest_subquery_or_statement" => InsidePolicy::SmallestSubqueryOrStatement,
        "smallest_statement" => InsidePolicy::SmallestStatement,
        "largest_statement" => InsidePolicy::LargestStatement,
        "largest_statement_or_batch" => InsidePolicy::LargestStatementOrBatch,
        "whole_script" => InsidePolicy::WholeScript,
        "everything_from_caret" => InsidePolicy::EverythingFromCaret,
        other => return Err(format!("unknown inside policy {other:?}")),
    })
}

fn outside(name: &str) -> Result<OutsidePolicy, String> {
    Ok(match name {
        "nothing" => OutsidePolicy::Nothing,
        "whole_script" => OutsidePolicy::WholeScript,
        "everything_below" => OutsidePolicy::EverythingBelow,
        other => return Err(format!("unknown outside policy {other:?}")),
    })
}

fn selection(name: &str) -> Result<SelectionPolicy, String> {
    Ok(match name {
        "as_one" => SelectionPolicy::AsOne,
        "as_separate" => SelectionPolicy::AsSeparate,
        "smart_expand" => SelectionPolicy::SmartExpand,
        other => return Err(format!("unknown selection policy {other:?}")),
    })
}

fn kind(name: &str) -> Result<CandidateKind, String> {
    Ok(match name {
        "subquery" => CandidateKind::Subquery,
        "statement" => CandidateKind::Statement,
        "largest_or_batch" => CandidateKind::LargestOrBatch,
        "from_caret" => CandidateKind::FromCaret,
        "whole_script" => CandidateKind::WholeScript,
        other => return Err(format!("unknown candidate kind {other:?}")),
    })
}

fn unit_kind_name(k: UnitKind) -> &'static str {
    match k {
        UnitKind::Statement => "statement",
        UnitKind::Subquery => "subquery",
        UnitKind::Batch => "batch",
        UnitKind::Verbatim => "verbatim",
        UnitKind::CopyIn { .. } => "copy_in",
    }
}

fn policy(case: &Case) -> Result<ExecPolicy, String> {
    let mut p = ExecPolicy::default();
    if let Some(n) = &case.policy.inside {
        p.inside = inside(n)?;
    }
    if let Some(n) = &case.policy.outside {
        p.outside = outside(n)?;
    }
    if let Some(n) = &case.policy.selection {
        p.selection = selection(n)?;
    }
    Ok(p)
}

/// The client a dialect's scripts are written for, when the case names none.
fn default_flavor(id: DialectId) -> ClientFlavor {
    match id {
        DialectId::Postgres => ClientFlavor::Psql,
        DialectId::MySql | DialectId::MariaDb => ClientFlavor::MySqlCli,
        DialectId::Sqlite => ClientFlavor::Sqlite3,
        _ => ClientFlavor::None,
    }
}

/// The case's text and where Run is asked, in text order.
fn text_and_at(case: &Case) -> Result<(String, Vec<CaretOrSelection>), String> {
    let (text, mut at) = match (&case.input, &case.fixture) {
        (Some(input), None) => {
            if case.caret.is_some() || !case.carets.is_empty() || case.select.is_some() {
                return Err("an `input` marks its carets and selections inline".into());
            }
            let m = extract(input)?;
            let mut at: Vec<CaretOrSelection> = m
                .carets
                .iter()
                .map(|c| CaretOrSelection::Caret(size(*c)))
                .collect();
            at.extend(
                m.selections
                    .iter()
                    .map(|s| CaretOrSelection::Selection(range(s))),
            );
            (m.text, at)
        }
        (None, Some(f)) => {
            let text = fixture(f);
            let mut at = Vec::new();
            for a in case.caret.iter().chain(&case.carets) {
                at.push(CaretOrSelection::Caret(size(a.offset(&text)?)));
            }
            if let Some(s) = &case.select {
                at.push(CaretOrSelection::Selection(range(&s.range(&text)?)));
            }
            (text, at)
        }
        _ => return Err("exactly one of `input` and `fixture`".into()),
    };
    if at.is_empty() {
        return Err("no caret and no selection".into());
    }
    at.sort_by_key(|a| match a {
        CaretOrSelection::Caret(c) => *c,
        CaretOrSelection::Selection(r) => r.start(),
    });
    Ok((text, at))
}

fn run_inner(case: &Case) -> Result<Outcome, String> {
    if let Some(reason) = &case.skip {
        if !case.requires.contains(reason) {
            return Err(format!("skipped for {reason:?}, which it does not require"));
        }
        return Ok(Outcome::Skipped(reason.clone()));
    }
    let needs_cst = case.requires.iter().any(|r| r == "cst");
    if let Some(other) = case.requires.iter().find(|r| *r != "cst") {
        return Err(format!(
            "requires {other:?}, which this runner never has: skip it with that reason"
        ));
    }
    let expect = match (needs_cst, &case.expect, &case.expect_without_cst) {
        (false, Some(e), None) => e,
        (true, Some(_), Some(e)) => e,
        (false, _, _) => return Err("needs `expect` and no `expect_without_cst`".into()),
        (true, _, _) => {
            return Err("a `cst` case needs `expect` (M5) and `expect_without_cst` (M3a)".into());
        }
    };
    if case.sources.is_empty() {
        return Err("names no source for its expectation".into());
    }

    let (text, at) = text_and_at(case)?;
    let ctx = dialect_ctx(
        case.dialect.as_deref().unwrap_or("postgres"),
        case.version.as_deref(),
    );
    let flavor = match case.flavor.as_deref() {
        None => default_flavor(ctx.id),
        some => flavor(some),
    };
    let policy = policy(case)?;
    let remembered = case.remembered.as_deref().map(kind).transpose()?;
    let doc = SqlDocument::new(text, ctx, flavor).snapshot();

    let got = resolve(&doc, &at, &policy, SplitMode::Execution, None, remembered);
    compare(expect, &got, &doc)?;

    // R-M3-8: on a document with no uncertain boundary, the preview is the execution.
    let split = doc.execution_split();
    if split.ambiguous.is_empty() && split.statements().all(|s| split.is_certain(s)) {
        let preview = resolve(&doc, &at, &policy, SplitMode::Editor, None, remembered);
        if preview != got {
            return Err(format!(
                "editor mode differs on a certain document: {} against {}",
                describe(&preview, &doc),
                describe(&got, &doc)
            ));
        }
    }
    Ok(if needs_cst {
        Outcome::RanWithoutCst
    } else {
        Outcome::Ran
    })
}

fn unit_texts<'a>(units: &[UnitSpan], doc: &'a DocSnapshot) -> Vec<&'a str> {
    units.iter().map(|u| &doc.text()[u.range]).collect()
}

/// A resolution as an error message shows it.
fn describe(r: &RangeResolution, doc: &DocSnapshot) -> String {
    match r {
        RangeResolution::Units(u) => format!("units {:?}", unit_texts(u, doc)),
        RangeResolution::Ask {
            candidates,
            preselect,
        } => {
            let list: Vec<(CandidateKind, Vec<&str>)> = candidates
                .iter()
                .map(|c| (c.kind, unit_texts(&c.units, doc)))
                .collect();
            format!("ask {list:?}, preselect {preselect}")
        }
        RangeResolution::NothingToRun => "nothing to run".into(),
        RangeResolution::Ambiguous(b) => format!("ambiguous {b:?}"),
        RangeResolution::UnsupportedClientCommand { at } => {
            format!("unsupported client command {:?}", &doc.text()[*at])
        }
    }
}

fn stmt_ranges(stmts: &[Stmt], refs: &[usize]) -> Result<Vec<TextRange>, String> {
    refs.iter()
        .map(|n| {
            stmts
                .get(*n)
                .map(|s| s.strict)
                .ok_or_else(|| format!("the document has no statement {n}"))
        })
        .collect()
}

fn compare(expect: &Expect, got: &RangeResolution, doc: &DocSnapshot) -> Result<(), String> {
    let forms = [
        expect.units.is_some(),
        expect.units_text.is_some(),
        expect.candidates.is_some(),
        expect.resolution.is_some(),
    ];
    if forms.iter().filter(|f| **f).count() != 1 {
        return Err(
            "expects exactly one of `units`, `units_text`, `candidates`, `resolution`".into(),
        );
    }
    let wrong = |what: &str| Err(format!("expected {what}, got {}", describe(got, doc)));
    let stmts: Vec<Stmt> = doc.execution_split().statements().copied().collect();

    if let Some(name) = &expect.resolution {
        let ok = match name.as_str() {
            "nothing" => matches!(got, RangeResolution::NothingToRun),
            "ambiguous" => matches!(got, RangeResolution::Ambiguous(b) if !b.is_empty()),
            "unsupported_client_command" => {
                matches!(got, RangeResolution::UnsupportedClientCommand { .. })
            }
            other => return Err(format!("unknown resolution {other:?}")),
        };
        if !ok {
            return wrong(name);
        }
        return no_unit_checks(expect);
    }

    if let Some(specs) = &expect.candidates {
        let RangeResolution::Ask {
            candidates,
            preselect,
        } = got
        else {
            return wrong("a chooser");
        };
        let mut want = Vec::new();
        for s in specs {
            want.push((kind(&s.kind)?, stmt_ranges(&stmts, &s.stmts)?));
        }
        let have: Vec<(CandidateKind, Vec<TextRange>)> = candidates
            .iter()
            .map(|c| (c.kind, c.units.iter().map(|u| u.range).collect()))
            .collect();
        if want != have {
            return wrong(&format!("the candidates {specs:?}"));
        }
        let Some(chosen) = candidates.get(*preselect) else {
            return Err(format!("preselect {preselect} is not a candidate"));
        };
        return candidate_checks(expect, candidates, chosen);
    }

    let RangeResolution::Units(units) = got else {
        return wrong("units");
    };
    if let Some(refs) = &expect.units {
        let refs: Vec<usize> = refs.iter().map(|r| r.stmt).collect();
        let want = stmt_ranges(&stmts, &refs)?;
        let have: Vec<TextRange> = units.iter().map(|u| u.range).collect();
        if want != have {
            return wrong(&format!("the statements {refs:?}"));
        }
    }
    if let Some(want) = &expect.units_text
        && unit_texts(units, doc) != *want
    {
        return wrong(&format!("units {want:?}"));
    }
    if let Some(want) = &expect.unit_kinds {
        let have: Vec<&str> = units.iter().map(|u| unit_kind_name(u.kind)).collect();
        if have != *want {
            return Err(format!("expected the unit kinds {want:?}, got {have:?}"));
        }
    }
    if let Some(want) = &expect.titles {
        let have: Vec<&str> = units
            .iter()
            .map(|u| u.title_comment.map_or("", |t| &doc.text()[t]))
            .collect();
        if have != *want {
            return Err(format!("expected the titles {want:?}, got {have:?}"));
        }
    }
    if expect.preselect.is_some() || expect.labels.is_some() || expect.label_starts.is_some() {
        return Err("`preselect`, `labels` and `label_starts` belong to `candidates`".into());
    }
    Ok(())
}

fn no_unit_checks(expect: &Expect) -> Result<(), String> {
    if expect.unit_kinds.is_some()
        || expect.titles.is_some()
        || expect.preselect.is_some()
        || expect.labels.is_some()
        || expect.label_starts.is_some()
    {
        return Err("a `resolution` has no units, candidates or labels to check".into());
    }
    Ok(())
}

fn candidate_checks(
    expect: &Expect,
    candidates: &[Candidate],
    chosen: &Candidate,
) -> Result<(), String> {
    if expect.unit_kinds.is_some() || expect.titles.is_some() {
        return Err("`unit_kinds` and `titles` belong to `units`".into());
    }
    if let Some(want) = &expect.preselect
        && kind(want)? != chosen.kind
    {
        return Err(format!(
            "expected {want} preselected, got {:?}",
            chosen.kind
        ));
    }
    if let Some(want) = &expect.labels {
        let have: Vec<&str> = candidates.iter().map(|c| c.label.as_str()).collect();
        if have != *want {
            return Err(format!("expected the labels {want:?}, got {have:?}"));
        }
    }
    if let Some(want) = &expect.label_starts
        && !chosen.label.starts_with(want.as_str())
    {
        return Err(format!(
            "expected a label starting {want:?}, got {:?}",
            chosen.label
        ));
    }
    Ok(())
}
