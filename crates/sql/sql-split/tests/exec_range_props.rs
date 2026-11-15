//! `resolve` properties (Task 13) over generated SQL-ish text in every dialect and client
//! flavor, at every caret and selection, under every policy:
//! - execution mode never returns a unit that is not a certain statement, and every unit it
//!   returns, read alone from a clean state as the server reads it, is exactly itself;
//! - the preview and execution agree on every exact statement (R-M3-8);
//! - every answer is well formed (the chooser's dedupe rule included).

use proptest::prelude::*;
use sql_dialect::{ClientFlavor, DialectCtx, DialectId};
use sql_lexer::{LexState, TokenKind};
use sql_split::exec_range::{
    Candidate, CandidateKind, CaretOrSelection, ExecPolicy, InsidePolicy, LABEL_MAX_CHARS,
    OutsidePolicy, RangeResolution, SelectionPolicy, UnitKind, UnitSpan, resolve,
};
use sql_split::{Confidence, DocSnapshot, SplitMode, SplitStart, SqlDocument, split_from};
use sql_text::{TextRange, TextSize};

const FRAGMENTS: &[&str] = &[
    // Review round 2: a `COPY` whose line holds another statement or an empty `;`.
    "\nCOPY t FROM STDIN; SELECT 1;\n",
    "COPY t FROM STDIN; ;\n",
    // Review round 1: an escape cut in two, and a quoted `*/` inside each comment the server
    // reads as code.
    "E'a\\'",
    "'a\\'",
    "E'a\\'; x; --'",
    "'a\\'; x; -- '",
    "\"a\\\"; x; -- \"",
    "/*! , \"*/ x; -- \" */",
    "/*+ \"*/ x; -- \" */",
    "/*M! , '*/' x; */",
    "U&'a\\'",
    "-- don't\n",
    " ",
    " ",
    "\n",
    "\n",
    "\n\n",
    "\t",
    ";",
    ";",
    ";",
    "SELECT 1",
    "SELECT 1;",
    "select a from t",
    "UPDATE t SET a = 1",
    "DELETE FROM t",
    "INSERT INTO t VALUES (1)",
    "WITH x AS (SELECT 1) SELECT * FROM x",
    "SELECT",
    "1",
    "a",
    ",",
    "(",
    ")",
    "'a;b'",
    "'",
    "\"x;y\"",
    "\"",
    "$$",
    "$$ a; b $$",
    "$q$ x; $q$",
    "$q$",
    "-- c\n",
    "-- ;\n",
    "/* c */",
    "/*",
    "*/",
    "/* ; */",
    "/*! ; */",
    "/*!50000 x */",
    "/*+ h */",
    "# c\n",
    "`q;`",
    "BEGIN",
    "END",
    "CASE WHEN a THEN 1 END",
    "IF",
    "LOOP",
    "DO",
    "CREATE PROCEDURE p() BEGIN SELECT 1; END",
    "CREATE FUNCTION f() RETURNS int LANGUAGE sql BEGIN ATOMIC SELECT 1; END",
    "CREATE TRIGGER tr AFTER INSERT ON t BEGIN SELECT 1; END",
    "\\g",
    "\\G",
    "\\;",
    "\\gset",
    "\n\\echo x\n",
    "\n\\set a 1\n",
    "\n\\if x\n",
    "\n\\endif\n",
    "\nDELIMITER $$\n",
    "\nDELIMITER ;\n",
    "\nDELIMITER //\n",
    "$$",
    "//",
    "\nCOPY t FROM STDIN;\n",
    "1\t2\n",
    "\n\\.\n",
    "\n.mode csv\n",
    "\nsource f\n",
    "İ",
    "é",
    "x\r\n",
    "\u{202E}",
];

fn any_sql() -> impl Strategy<Value = String> {
    prop::collection::vec(prop::sample::select(FRAGMENTS), 0..40).prop_map(|v| v.join(" "))
}

fn dialects() -> impl Strategy<Value = (DialectId, ClientFlavor)> {
    prop::sample::select(vec![
        (DialectId::Postgres, ClientFlavor::Psql),
        (DialectId::Postgres, ClientFlavor::Psql),
        (DialectId::Postgres, ClientFlavor::None),
        (DialectId::MySql, ClientFlavor::MySqlCli),
        (DialectId::MySql, ClientFlavor::MySqlCli),
        (DialectId::MySql, ClientFlavor::None),
        (DialectId::MariaDb, ClientFlavor::MySqlCli),
        (DialectId::Sqlite, ClientFlavor::Sqlite3),
        (DialectId::Sqlite, ClientFlavor::None),
        (DialectId::Generic, ClientFlavor::None),
    ])
}

/// A caret as a seed: `offset(seed, text)` puts it somewhere in any text.
fn caret_s() -> impl Strategy<Value = usize> {
    0usize..100_000
}

fn offset(seed: usize, text: &str) -> usize {
    let mut i = seed % (text.len() + 1);
    while !text.is_char_boundary(i) {
        i -= 1;
    }
    i
}

fn size(n: usize) -> TextSize {
    TextSize::try_from(n).unwrap()
}

fn doc(s: &str, id: DialectId, flavor: ClientFlavor) -> DocSnapshot {
    SqlDocument::new(s.to_owned(), DialectCtx::new(id, None), flavor).snapshot()
}

fn policies() -> Vec<ExecPolicy> {
    let mut out = Vec::new();
    for inside in [
        InsidePolicy::Ask,
        InsidePolicy::SmallestSubqueryOrStatement,
        InsidePolicy::SmallestStatement,
        InsidePolicy::LargestStatement,
        InsidePolicy::LargestStatementOrBatch,
        InsidePolicy::WholeScript,
        InsidePolicy::EverythingFromCaret,
    ] {
        out.push(ExecPolicy {
            inside,
            ..ExecPolicy::default()
        });
    }
    for outside in [OutsidePolicy::WholeScript, OutsidePolicy::EverythingBelow] {
        out.push(ExecPolicy {
            outside,
            ..ExecPolicy::default()
        });
    }
    for selection in [SelectionPolicy::AsOne, SelectionPolicy::SmartExpand] {
        out.push(ExecPolicy {
            selection,
            ..ExecPolicy::default()
        });
    }
    out
}

/// A caret, a selection, and two carets from the two seeds.
fn places(s: &str, a: usize, b: usize) -> Vec<Vec<CaretOrSelection>> {
    let (a, b) = (offset(a, s), offset(b, s));
    let (lo, hi) = (a.min(b), a.max(b));
    vec![
        vec![CaretOrSelection::Caret(size(a))],
        vec![CaretOrSelection::Selection(TextRange::new(
            size(lo),
            size(hi),
        ))],
        vec![
            CaretOrSelection::Caret(size(a)),
            CaretOrSelection::Caret(size(b)),
        ],
    ]
}

/// Every unit an answer could run: its units, or every candidate's.
fn all_units(r: &RangeResolution) -> Vec<UnitSpan> {
    match r {
        RangeResolution::Units(u) => u.clone(),
        RangeResolution::Ask { candidates, .. } => {
            candidates.iter().flat_map(|c| c.units.clone()).collect()
        }
        _ => Vec::new(),
    }
}

/// `piece` split alone from a clean lexer state under `delimiter`, as the server reads it.
fn alone(d: &DocSnapshot, piece: &str, delimiter: Option<std::sync::Arc<str>>) -> sql_split::Split {
    let clean = SplitStart {
        lex: LexState::Initial,
        delimiter,
    };
    split_from(piece, d.ctx(), d.flavor(), SplitMode::Execution, &clean)
}

/// Whether `piece` alone is exactly one certain statement (or, `verbatim`, certain statements
/// with no client command and no COPY data).
fn certain_alone(
    d: &DocSnapshot,
    piece: &str,
    delimiter: Option<std::sync::Arc<str>>,
    verbatim: bool,
) -> Result<(), String> {
    let split = alone(d, piece, delimiter);
    let whole = TextRange::up_to(size(piece.len()));
    if !split.ambiguous.is_empty() || split.statements().any(|s| !split.is_certain(s)) {
        return Err(format!(
            "{piece:?} is uncertain alone: {:?}",
            split.ambiguous
        ));
    }
    if verbatim {
        if !split.client.is_empty() || split.statements().any(|s| s.copy_data.is_some()) {
            return Err(format!(
                "verbatim {piece:?} holds a client command or COPY data"
            ));
        }
        return Ok(());
    }
    let stmts: Vec<TextRange> = split.statements().map(|s| s.strict).collect();
    if stmts != [whole] {
        return Err(format!(
            "{piece:?} alone is {:?}",
            stmts.iter().map(|r| &piece[*r]).collect::<Vec<_>>()
        ));
    }
    Ok(())
}

/// The document-level check (Task 13 review round 1): a certain unit of the selection `r` is
/// what the *document* shows there, whatever the unit reads as alone.
/// - It never overlaps the rows of a `COPY … FROM STDIN`: they are data.
/// - It never overlaps a document token that an edge of the selection is strictly inside
///   (a string or name entered after its first byte, openers and prefixes included; a word
///   or number cut in two; an executable comment or hint). White space and plain comments
///   are not tokens a selection cuts.
/// - It starts and ends on a token boundary of the whole-document lexing, or lies wholly
///   inside one `--` comment (a statement kept in a comment). A unit sent as written (`verbatim`) may also
///   have an edge inside a plain comment.
///
/// The whole-document lexing is the document's truth up to its first client command or
/// `COPY` data. After one the splitter lexes again, and the one lexing can read what follows
/// differently (a quote in `COPY` data opens a "string" there); the token checks stop at that
/// place, the `COPY` check never does.
fn in_step(
    snap: &DocSnapshot,
    r: TextRange,
    unit: TextRange,
    verbatim: bool,
) -> Result<(), String> {
    let (text, lexed) = (snap.text(), snap.lexed());
    let overlap = |a: TextRange, b: TextRange| a.start() < b.end() && b.start() < a.end();
    // Where the splitter first lexes again.
    let mut relexed = snap
        .execution_split()
        .client
        .iter()
        .map(|c| c.range.start())
        .min()
        .unwrap_or(TextSize::from(u32::MAX));
    for st in snap.execution_split().statements() {
        if let Some(d) = st.copy_data {
            relexed = relexed.min(st.strict.start());
            if overlap(d.rows, unit) {
                return Err(format!(
                    "unit {:?} overlaps COPY rows {:?}",
                    &text[unit], d.rows
                ));
            }
        }
    }
    let plain = |k: TokenKind| {
        matches!(
            k,
            TokenKind::Whitespace
                | TokenKind::Newline
                | TokenKind::LineComment
                | TokenKind::BlockComment
        )
    };
    for edge in [r.start(), r.end()] {
        if edge > relexed {
            continue;
        }
        if let Some(i) = lexed.token_at(edge) {
            let t = lexed.range(i);
            if t.start() < edge && edge < t.end() && !plain(lexed.kind(i)) && overlap(t, unit) {
                return Err(format!(
                    "unit {:?} overlaps the {:?} token {:?} the selection cuts",
                    &text[unit],
                    lexed.kind(i),
                    &text[t]
                ));
            }
        }
    }
    let inside = |pos: TextSize| {
        lexed.token_at(pos).filter(|i| {
            let t = lexed.range(*i);
            t.start() < pos && pos < t.end()
        })
    };
    for pos in [unit.start(), unit.end()] {
        if pos > relexed {
            continue;
        }
        let Some(i) = inside(pos) else { continue };
        let (kind, t) = (lexed.kind(i), lexed.range(i));
        let kept_in_comment = kind == TokenKind::LineComment && t.contains_range(unit);
        let written = verbatim && matches!(kind, TokenKind::LineComment | TokenKind::BlockComment);
        if !kept_in_comment && !written {
            return Err(format!(
                "unit {:?} has an edge inside the {kind:?} token {:?}",
                &text[unit], &text[t]
            ));
        }
    }
    Ok(())
}

/// A unit execution mode returned is a certain statement, alone and in its document.
///
/// Alone, it is read under the client delimiter in force where it starts. A selection is its
/// own script (it may cut a `DELIMITER` line away), so there the delimiter is the document's
/// at the unit, or the one the selection starts under, or none.
fn unit_is_certain(
    d: &DocSnapshot,
    u: &UnitSpan,
    selection: Option<TextRange>,
) -> Result<(), String> {
    let text = d.text();
    let piece = &text[u.range];
    let verbatim = u.kind == UnitKind::Verbatim;
    let mut delimiters = vec![d.split_start_at(u.range.start()).delimiter];
    if let Some(sel) = selection {
        delimiters.push(d.split_start_at(sel.start()).delimiter);
        delimiters.push(None);
    }
    let mut last = Ok(());
    for delimiter in delimiters {
        last = certain_alone(d, piece, delimiter, verbatim);
        if last.is_ok() {
            break;
        }
    }
    last?;
    if selection.is_none() {
        for split in [d.execution_split(), d.editor_split()] {
            let Some(s) = split.statements().find(|s| s.strict == u.range) else {
                return Err(format!("{piece:?} is no statement of both splits"));
            };
            if !split.is_certain(s) || s.confidence != Confidence::Exact {
                return Err(format!("{piece:?} is not certain"));
            }
            if let Some(a) = split.ambiguities_in(u.range).next() {
                return Err(format!("{piece:?} is crossed by {a:?}"));
            }
        }
    }
    Ok(())
}

fn no_unit_crosses_heuristic(
    s: &str,
    id: DialectId,
    flavor: ClientFlavor,
    a: usize,
    b: usize,
) -> Result<(), String> {
    let d = doc(s, id, flavor);
    for at in places(s, a, b) {
        let selected = at.iter().find_map(|x| match x {
            CaretOrSelection::Selection(r) if !r.is_empty() => Some(*r),
            _ => None,
        });
        for p in policies() {
            let r = resolve(&d, &at, &p, SplitMode::Execution, None, None);
            for u in all_units(&r) {
                // An expanded selection returns statements of the document.
                let own_script = selected.filter(|_| {
                    p.selection != SelectionPolicy::SmartExpand || u.kind == UnitKind::Verbatim
                });
                unit_is_certain(&d, &u, own_script).map_err(|e| format!("{at:?} {p:?}: {e}"))?;
                if let Some(r) = own_script.filter(|_| at.len() == 1) {
                    in_step(&d, r, u.range, u.kind == UnitKind::Verbatim)
                        .map_err(|e| format!("{at:?} {p:?}: {e}"))?;
                }
            }
        }
    }
    Ok(())
}

/// The editor statements of `units` are all `Exact`.
fn all_exact(d: &DocSnapshot, units: &[UnitSpan]) -> bool {
    let editor = d.editor_split();
    units.iter().all(|u| {
        editor
            .statements()
            .any(|s| s.strict == u.range && s.confidence == Confidence::Exact)
    })
}

/// What an answer is, without its details: the ranges it runs, or how it refuses.
#[derive(Debug, PartialEq, Eq)]
enum Answer {
    Runs(Vec<TextRange>),
    Asks,
    Nothing,
    Refuses,
}

fn answer_kind(r: &RangeResolution) -> Answer {
    match r {
        RangeResolution::Units(u) => Answer::Runs(u.iter().map(|u| u.range).collect()),
        RangeResolution::Ask { .. } => Answer::Asks,
        RangeResolution::NothingToRun => Answer::Nothing,
        RangeResolution::Ambiguous(_) | RangeResolution::UnsupportedClientCommand { .. } => {
            Answer::Refuses
        }
    }
}

/// What "as separate statements" must answer for the selection `r`, from
/// `DocSnapshot::split_selection` and `DocSnapshot::selection_cuts` alone.
fn separate_by_hand(d: &DocSnapshot, r: TextRange) -> Answer {
    if d.text()[r].trim().is_empty() {
        return Answer::Nothing;
    }
    let split = d.split_selection(r, SplitMode::Execution);
    // An edge inside a token refuses the selection even when the cut token is in no
    // statement (a client command's first bytes, say).
    if !d.selection_cuts(r).is_empty()
        || split.client.iter().any(|c| !c.supported)
        || !split.ambiguous.is_empty()
        || split.statements().any(|s| !split.is_certain(s))
    {
        return Answer::Refuses;
    }
    let ranges: Vec<TextRange> = split.statements().map(|s| s.strict).collect();
    if ranges.is_empty() {
        Answer::Nothing
    } else {
        Answer::Runs(ranges)
    }
}

fn chosen(r: &RangeResolution) -> Option<&[UnitSpan]> {
    match r {
        RangeResolution::Units(u) => Some(u),
        RangeResolution::Ask {
            candidates,
            preselect,
        } => candidates.get(*preselect).map(|c| c.units.as_slice()),
        _ => None,
    }
}

fn agree_on_exact(
    s: &str,
    id: DialectId,
    flavor: ClientFlavor,
    a: usize,
    b: usize,
) -> Result<(), String> {
    let d = doc(s, id, flavor);
    for at in places(s, a, b) {
        let selected = at
            .iter()
            .any(|x| matches!(x, CaretOrSelection::Selection(r) if !r.is_empty()));
        for p in policies() {
            for remembered in [
                None,
                Some(CandidateKind::WholeScript),
                Some(CandidateKind::FromCaret),
            ] {
                let x = resolve(&d, &at, &p, SplitMode::Execution, None, remembered);
                let e = resolve(&d, &at, &p, SplitMode::Editor, None, remembered);
                let ctx = |what: &str| {
                    format!("{at:?} {p:?} {remembered:?}: {what}: editor {e:?}, execution {x:?}")
                };
                // A selection is split on its own: one answer for both modes. A verbatim
                // unit is the selection, not a statement.
                let verbatim = all_units(&x)
                    .iter()
                    .chain(&all_units(&e))
                    .any(|u| u.kind == UnitKind::Verbatim);
                if selected && at.len() == 1 {
                    if e != x {
                        return Err(ctx("a selection differs between the modes"));
                    }
                    // And the answer is the selection's own split, read independently: as
                    // separate statements, Run runs exactly its statements when every one
                    // is certain and nothing in it is uncertain or unsupported, and refuses
                    // otherwise.
                    if p.selection == SelectionPolicy::AsSeparate
                        && let CaretOrSelection::Selection(r) = at[0]
                    {
                        let (want, got) = (separate_by_hand(&d, r), answer_kind(&x));
                        // One difference is allowed, and it is a refusal: `resolve` also
                        // reads the client commands the editor's split finds (after an
                        // unterminated construct, inside a statement with no `;`), and a
                        // selection that starts inside one of those lines is unsupported.
                        let stricter =
                            matches!(x, RangeResolution::UnsupportedClientCommand { .. });
                        if want != got && !stricter {
                            return Err(ctx(&format!("the selection's split says {want:?}")));
                        }
                    }
                    continue;
                }
                if verbatim {
                    continue;
                }
                // What execution runs is what the preview shows: the units, or the
                // candidate the chooser opens on.
                if let Some(xu) = chosen(&x)
                    && chosen(&e) != Some(xu)
                {
                    return Err(ctx("execution runs what the preview does not show"));
                }
                // Where Run runs or asks, the preview is the same answer: never a chooser
                // over an immediate run, never a candidate Run does not offer.
                if chosen(&x).is_some() && e != x {
                    return Err(ctx("the preview differs where Run runs or asks"));
                }
                // Every candidate execution offers, the preview's chooser offers too.
                if let RangeResolution::Ask { candidates: xc, .. } = &x {
                    let RangeResolution::Ask { candidates: ec, .. } = &e else {
                        return Err(ctx("execution asks, the preview does not"));
                    };
                    if !xc.iter().all(|c| ec.contains(c)) {
                        return Err(ctx("execution offers what the preview does not"));
                    }
                }
                // What the preview shows as certain, execution runs: whatever execution
                // refuses or finds empty, the preview never shows as an exact range.
                if let Some(eu) = chosen(&e)
                    && all_exact(&d, eu)
                    && chosen(&x) != Some(eu)
                {
                    return Err(ctx(
                        "the preview shows exact statements execution does not run",
                    ));
                }
            }
        }
    }
    Ok(())
}

fn ranges(c: &Candidate) -> Vec<TextRange> {
    c.units.iter().map(|u| u.range).collect()
}

fn well_formed(
    s: &str,
    id: DialectId,
    flavor: ClientFlavor,
    a: usize,
    b: usize,
) -> Result<(), String> {
    let d = doc(s, id, flavor);
    let len = size(s.len());
    let check_units = |units: &[UnitSpan], many: bool, exact: bool| -> Result<(), String> {
        if units.is_empty() {
            return Err("no unit".into());
        }
        let mut last = TextSize::from(0);
        for (n, u) in units.iter().enumerate() {
            let Some(piece) = s.get(usize::from(u.range.start())..usize::from(u.range.end()))
            else {
                return Err(format!("{:?} is not a text range", u.range));
            };
            // A heuristic item of the editor's may end in white space; what runs never does.
            if piece.is_empty() || (exact && piece.trim() != piece) {
                return Err(format!("unit {piece:?} is empty or carries white space"));
            }
            if n > 0 && u.range.start() < last && !many {
                return Err(format!("unit {piece:?} overlaps the one before"));
            }
            if n > 0 && u.range.start() < units[n - 1].range.start() {
                return Err("units are not in text order".into());
            }
            last = u.range.end();
            if let Some(t) = u.title_comment
                && (t.end() > u.range.start()
                    || s.get(usize::from(t.start())..usize::from(t.end()))
                        .is_none())
            {
                return Err(format!("title {t:?} is not before {piece:?}"));
            }
            if let UnitKind::CopyIn { data } = u.kind
                && (data.start() < u.range.end() || data.end() > len)
            {
                return Err(format!("COPY data {data:?} is not after {piece:?}"));
            }
        }
        Ok(())
    };
    for at in places(s, a, b) {
        for p in policies() {
            for mode in [SplitMode::Execution, SplitMode::Editor] {
                let r = resolve(&d, &at, &p, mode, None, None);
                if r != resolve(&d, &at, &p, mode, None, None) {
                    return Err("resolve is not deterministic".into());
                }
                let ctx = |e: String| format!("{at:?} {p:?} {mode:?}: {e} in {r:?}");
                let exact = mode == SplitMode::Execution;
                match &r {
                    RangeResolution::Units(u) => {
                        check_units(u, at.len() > 1, exact).map_err(ctx)?
                    }
                    RangeResolution::Ask {
                        candidates,
                        preselect,
                    } => {
                        if candidates.len() < 2 || *preselect >= candidates.len() {
                            return Err(ctx(
                                "a chooser needs two candidates and a valid preselect".into(),
                            ));
                        }
                        if at.len() != 1 || p.inside != InsidePolicy::Ask {
                            return Err(ctx("only one caret under Ask opens the chooser".into()));
                        }
                        if candidates[*preselect].kind != CandidateKind::Statement {
                            return Err(ctx("the statement is preselected".into()));
                        }
                        let order = |k: CandidateKind| k as u8;
                        for (n, c) in candidates.iter().enumerate() {
                            check_units(&c.units, false, exact).map_err(ctx)?;
                            if n > 0 && order(candidates[n - 1].kind) >= order(c.kind) {
                                return Err(ctx("candidates are not in resolve order".into()));
                            }
                            if candidates[..n].iter().any(|o| ranges(o) == ranges(c)) {
                                return Err(ctx("two candidates run the same ranges".into()));
                            }
                            if c.label.is_empty()
                                || c.label.chars().count() > LABEL_MAX_CHARS
                                || c.label
                                    .chars()
                                    .any(|ch| ch.is_control() || ch == '\u{202E}')
                                || c.label.contains("  ")
                            {
                                return Err(ctx(format!("label {:?}", c.label)));
                            }
                            if c.kind == CandidateKind::Statement && c.units.len() != 1 {
                                return Err(ctx("the statement candidate is one unit".into()));
                            }
                        }
                        let of = |k| candidates.iter().find(|c| c.kind == k);
                        if let (Some(f), Some(w)) =
                            (of(CandidateKind::FromCaret), of(CandidateKind::WholeScript))
                        {
                            let (f, w) = (ranges(f), ranges(w));
                            if f.len() >= w.len() || !w.ends_with(&f) {
                                return Err(ctx(
                                    "from-caret is not a strict tail of the whole script".into(),
                                ));
                            }
                        }
                    }
                    RangeResolution::NothingToRun => {}
                    RangeResolution::Ambiguous(b) => {
                        if b.is_empty() || b.iter().any(|a| a.at.end() > len) {
                            return Err(ctx(
                                "an ambiguous answer names boundaries in the text".into()
                            ));
                        }
                    }
                    RangeResolution::UnsupportedClientCommand { at } => {
                        if at.end() > len || at.is_empty() {
                            return Err(ctx("the command is in the text".into()));
                        }
                    }
                }
            }
        }
    }
    Ok(())
}

proptest! {
    #![proptest_config(ProptestConfig { cases: ProptestConfig::default().cases.max(512), ..ProptestConfig::default() })]

    #[test]
    fn execution_never_crosses_a_heuristic_boundary(s in any_sql(), (id, flavor) in dialects(), a in caret_s(), b in caret_s()) {
        let r = no_unit_crosses_heuristic(&s, id, flavor, a, b);
        prop_assert!(r.is_ok(), "{:?} in {:?} {:?}: {}", s, id, flavor, r.unwrap_err());
    }

    // R-M3-8
    #[test]
    fn editor_and_execution_agree_on_exact_items(s in any_sql(), (id, flavor) in dialects(), a in caret_s(), b in caret_s()) {
        let r = agree_on_exact(&s, id, flavor, a, b);
        prop_assert!(r.is_ok(), "{:?} in {:?} {:?}: {}", s, id, flavor, r.unwrap_err());
    }

    #[test]
    fn every_answer_is_well_formed(s in any_sql(), (id, flavor) in dialects(), a in caret_s(), b in caret_s()) {
        let r = well_formed(&s, id, flavor, a, b);
        prop_assert!(r.is_ok(), "{:?} in {:?} {:?}: {}", s, id, flavor, r.unwrap_err());
    }
}
