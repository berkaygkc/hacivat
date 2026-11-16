// ## m3b-t01c-core-seams
//! Parameter preparation (HV-EXEC-03 binding, SQL-PARAM-01..03; Task 15): each unit's
//! placeholders found by `sql_split::scan_params`, its values asked for (`NeedsValues`) or
//! remembered per `(profile, name)` for the app session (R-M3-79), the text rewritten to the
//! engine's placeholders (`final_text`, which is exactly what is classified and sent:
//! P-M3b-2), the values masked for the log and the history, and a map from the rewritten text
//! back to the document (`OffsetMap`, for error positions).
//!
//! ## m3b-t15-gate
//! Since Task 15 ([`prepare_units`] states the rules). Nothing is wired into Run before Task
//! 24 (R-M3-52). Fail closed: any placeholder Task 6's scan cannot bind refuses the unit
//! (`ParamDiag::Unbindable`: SQLite would bind NULL to a parameter nobody filled), and so does
//! every other diagnostic but the informational `QuestionMarkOff`; nothing of the run is sent.

use std::collections::HashMap;
use std::ops::Range;
use std::sync::Arc;

use hacivat_proto::ProfileId;
use hacivat_values::BindValue;
use sql_dialect::{DialectCtx, lex_profile};
use sql_lexer::{LexState, TokenFlags, TokenKind, lex};
use sql_split::{
    BindPlan, ParamDiag, ParamScan, ParamSettings, PlaceholderKind, StmtHead, scan_params,
};
use sql_text::TextRange;

use crate::units::ExecUnit;

/// A piecewise map from offsets in a unit's final text to document offsets. Each piece starts
/// at a unit offset; a linear piece maps by its shift, a constant one (a rewritten placeholder)
/// maps every offset inside it to the placeholder's start in the document.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct OffsetMap {
    /// `(unit_start, doc_start, linear)` per piece, by `unit_start`; never empty.
    pieces: Vec<(usize, usize, bool)>,
}

impl OffsetMap {
    /// The map of a unit sent as written, starting at document offset `start`.
    pub fn identity(start: usize) -> Self {
        OffsetMap {
            pieces: vec![(0, start, true)],
        }
    }

    // ## m3b-t15-gate
    /// The map of a unit at document offset `start` rewritten by `edits` (`RewrittenSql.edits`:
    /// each placeholder's range in the unit and its replacement's range in the final text, in
    /// text order). Text outside the placeholders maps back byte for byte.
    pub fn from_edits(start: usize, edits: &[(TextRange, TextRange)]) -> Self {
        let mut pieces = vec![(0, start, true)];
        for (unit, rewritten) in edits {
            pieces.push((
                usize::from(rewritten.start()),
                start + usize::from(unit.start()),
                false,
            ));
            pieces.push((
                usize::from(rewritten.end()),
                start + usize::from(unit.end()),
                true,
            ));
        }
        OffsetMap { pieces }
    }

    /// The document offset of `unit_offset`.
    pub fn to_doc(&self, unit_offset: usize) -> usize {
        let (unit_start, doc_start, linear) = self
            .pieces
            .iter()
            .rev()
            .find(|(u, _, _)| *u <= unit_offset)
            .copied()
            .unwrap_or(self.pieces[0]);
        if linear {
            doc_start + unit_offset.saturating_sub(unit_start)
        } else {
            doc_start
        }
    }
}

/// One unit ready to classify and send.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PreparedUnit {
    pub unit: ExecUnit,
    /// The exact text classified and sent (placeholders rewritten for the engine).
    pub final_text: Arc<str>,
    pub binds: Arc<[BindValue]>,
    /// `final_text` with the bind values masked (the log, the history).
    pub masked: Arc<str>,
    pub offsets: OffsetMap,
}

/// A parameter the user must give a value for.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ParamPrompt {
    pub name: Arc<str>,
    pub occurrences: u32,
    /// The value remembered for this profile in this app session: it pre-fills the prompt and
    /// never answers it (Task 15 fix round 1, D8 ruling). Always `None` for an ordinal or
    /// anonymous parameter (I4).
    pub remembered: Option<BindValue>,
    // ## m3b-t15-gate (fix round 1, I4, M2)
    /// `None` for a named parameter (`:name`, SQLite `@name`/`$name`), one value for every unit
    /// of the run that names it; `Some(i)` for an ordinal or anonymous one (`?`, `?NNN`, `$n`),
    /// which belongs to unit `i` only: two units never share it.
    pub unit: Option<u32>,
}

impl ParamPrompt {
    /// Whether the answer may be remembered (`ParamValueStore::put`): named parameters only. An
    /// ordinal name (`?1`, `$2`) means a different thing in every statement (I4).
    pub fn rememberable(&self) -> bool {
        self.unit.is_none()
    }
}

/// The values the user gave for this run's prompts (keyed like the prompts: name and, for an
/// ordinal or anonymous parameter, its unit).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ParamAnswers {
    values: HashMap<(Option<u32>, Arc<str>), BindValue>,
}

impl ParamAnswers {
    /// The answer to `prompt`.
    pub fn insert(&mut self, prompt: &ParamPrompt, v: BindValue) {
        self.values.insert((prompt.unit, prompt.name.clone()), v);
    }

    fn get(&self, unit: Option<u32>, name: &str) -> Option<&BindValue> {
        self.values.get(&(unit, Arc::from(name)))
    }
}

/// What preparing a run's units gave.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Prepared {
    Ready(Vec<PreparedUnit>),
    /// Ask for these, then prepare again with the answers ([`prepare_units_with`]).
    NeedsValues(Vec<ParamPrompt>),
    /// Unit `unit` (its index) cannot be sent: `at` in the document, `diag` why.
    Refused {
        unit: u32,
        at: Range<usize>,
        diag: ParamDiag,
    },
}

/// The values remembered per `(profile, name)` for the app session only (never persisted,
/// R-M3-79). Named parameters only ([`ParamPrompt::rememberable`]).
pub trait ParamValueStore {
    fn get(&self, profile: &ProfileId, name: &str) -> Option<BindValue>;
    fn put(&self, profile: &ProfileId, name: &str, v: BindValue);
}

// ## m3b-t15-gate

/// The Fluent ID that says why a unit's parameters refuse it (`hacivat-core.ftl`, block
/// `m3b-t15-gate`), a literal per arm.
pub fn param_diag_message_id(d: ParamDiag) -> &'static str {
    match d {
        ParamDiag::TemplateVariable => "exec-param-template-variable",
        ParamDiag::QuestionMarkOff => "exec-param-question-mark-off",
        ParamDiag::ParamInDdl => "exec-param-in-ddl",
        ParamDiag::MixedStyles => "exec-param-mixed-styles",
        ParamDiag::TooManyBinds => "exec-param-too-many-binds",
        ParamDiag::Unbindable => "exec-param-unbindable",
    }
}

/// [`prepare_units_with`] with no answers yet: `Ready` only when no unit has a parameter.
pub fn prepare_units(
    units: Vec<ExecUnit>,
    ctx: &DialectCtx,
    s: &ParamSettings,
    values: &dyn ParamValueStore,
    profile: &ProfileId,
) -> Prepared {
    prepare_units_with(units, ctx, s, values, profile, &ParamAnswers::default())
}

/// Prepares `units` of a run on `profile` with the user's `answers` (HV-EXEC-03):
///
/// 1. Each unit is scanned (`scan_params` with the unit's own head, `s`, `ctx`). The first unit
///    with a diagnostic other than `QuestionMarkOff` (a template variable, a placeholder in DDL
///    without `s.in_ddl`, mixed styles, more than 65,535 binds, any unbindable marker), with a
///    placeholder that does not bind, or with a driver slot no placeholder fills (`$2` alone)
///    is `Refused` at the diagnostic's document range, before any prompt (M5).
/// 2. Every parameter of the run must have an answer: a named one once for the whole run, an
///    ordinal or anonymous one (`?`, `?NNN`, `$n`) once per unit (M2). When one has none, the
///    answer is `NeedsValues` with one prompt per parameter in first-use order, each with its
///    occurrence count and, for a named one, `values`' remembered value to pre-fill it.
///    Remembered values never skip the prompt (D8 ruling): a statement with parameters always
///    asks. The caller asks, puts the answers of rememberable prompts into `values`, and
///    prepares again with `answers`.
/// 3. Otherwise `Ready`: each unit's `final_text` is the rewrite (exactly what is classified
///    and sent, P-M3b-2), `binds` hold one value per driver slot in slot order, `offsets` map
///    the rewrite back to the document, and `masked` is `final_text` with every secret literal
///    masked ([`mask_secrets`]). A unit without placeholders is sent as written.
pub fn prepare_units_with(
    units: Vec<ExecUnit>,
    ctx: &DialectCtx,
    s: &ParamSettings,
    values: &dyn ParamValueStore,
    profile: &ProfileId,
    answers: &ParamAnswers,
) -> Prepared {
    let mut scans = Vec::with_capacity(units.len());
    for (i, unit) in units.iter().enumerate() {
        let scan = scan_params(&unit.text, ctx, &StmtHead::default(), s);
        if let Some((at, diag)) = refusal(&scan) {
            return refused(i, unit, at, diag);
        }
        scans.push(scan);
    }

    // The run's parameters, in first-use order, with their occurrences.
    let mut prompts: Vec<ParamPrompt> = Vec::new();
    let mut at: HashMap<(Option<u32>, Arc<str>), usize> = HashMap::new();
    for (i, (unit, scan)) in units.iter().zip(&scans).enumerate() {
        for p in &scan.placeholders {
            let name = plan_name(&unit.text, &p.kind, p.range);
            let scope = prompt_unit(&p.kind, i);
            match at.get(&(scope, name.clone())) {
                Some(&k) => prompts[k].occurrences = prompts[k].occurrences.saturating_add(1),
                None => {
                    at.insert((scope, name.clone()), prompts.len());
                    prompts.push(ParamPrompt {
                        name,
                        occurrences: 1,
                        remembered: None,
                        unit: scope,
                    });
                }
            }
        }
    }
    let answered = prompts
        .iter()
        .all(|q| answers.get(q.unit, &q.name).is_some());
    if !answered {
        for q in &mut prompts {
            if q.rememberable() {
                q.remembered = values.get(profile, &q.name);
            }
        }
        return Prepared::NeedsValues(prompts);
    }

    let mut ready = Vec::with_capacity(units.len());
    for (i, (unit, scan)) in units.into_iter().zip(scans).enumerate() {
        let start = unit.doc_range.start;
        let Some(rw) = scan.rewritten else {
            let masked = mask_secrets(&unit.text, ctx);
            ready.push(PreparedUnit {
                final_text: unit.text.clone(),
                binds: Arc::from(Vec::new()),
                masked,
                offsets: OffsetMap::identity(start),
                unit,
            });
            continue;
        };
        // Each plan name's answer, scoped like its prompt.
        let mut known: HashMap<Arc<str>, BindValue> = HashMap::new();
        for p in &scan.placeholders {
            let name = plan_name(&unit.text, &p.kind, p.range);
            if let Some(v) = answers.get(prompt_unit(&p.kind, i), &name) {
                known.insert(name, v.clone());
            }
        }
        let Some(binds) = binds_of(&rw.plan, &known) else {
            let at = scan
                .placeholders
                .first()
                .map_or(TextRange::default(), |p| p.range);
            return refused(i, &unit, at, ParamDiag::Unbindable);
        };
        let final_text: Arc<str> = Arc::from(rw.text);
        ready.push(PreparedUnit {
            masked: mask_secrets(&final_text, ctx),
            final_text,
            binds: Arc::from(binds),
            offsets: OffsetMap::from_edits(start, &rw.edits),
            unit,
        });
    }
    Prepared::Ready(ready)
}

/// The prompt scope of a placeholder of unit `i`: shared for a named one, the unit's own for an
/// ordinal or anonymous one.
fn prompt_unit(kind: &PlaceholderKind, i: usize) -> Option<u32> {
    match kind {
        PlaceholderKind::Named(_) => None,
        PlaceholderKind::Question(_) | PlaceholderKind::Positional(_) => {
            Some(u32::try_from(i).unwrap_or(u32::MAX))
        }
    }
}

fn refused(i: usize, unit: &ExecUnit, at: TextRange, diag: ParamDiag) -> Prepared {
    let start = unit.doc_range.start;
    Prepared::Refused {
        unit: u32::try_from(i).unwrap_or(u32::MAX),
        at: start + usize::from(at.start())..start + usize::from(at.end()),
        diag,
    }
}

/// Why a scanned unit cannot be sent, if it cannot: its first diagnostic but the informational
/// `QuestionMarkOff`, a placeholder without a rewrite, a bind plan with a driver slot no
/// placeholder fills, or a template variable.
fn refusal(scan: &ParamScan) -> Option<(TextRange, ParamDiag)> {
    if let Some((at, d)) = scan
        .diags
        .iter()
        .find(|(_, d)| *d != ParamDiag::QuestionMarkOff)
    {
        return Some((*at, *d));
    }
    let first = scan.placeholders.first().map(|p| p.range);
    match &scan.rewritten {
        None => {
            if let Some(at) = first {
                return Some((at, ParamDiag::Unbindable));
            }
        }
        Some(rw) if !dense(&rw.plan) => {
            return Some((first.unwrap_or_default(), ParamDiag::Unbindable));
        }
        Some(_) => {}
    }
    scan.templates
        .first()
        .map(|t| (t.range, ParamDiag::TemplateVariable))
}

/// Whether the plan's driver slots are `0..n` with none missing.
fn dense(plan: &BindPlan) -> bool {
    let mut seen: Vec<bool> = Vec::new();
    for &k in plan.slots.iter().flat_map(|s| s.iter()) {
        let Ok(k) = usize::try_from(k) else {
            return false;
        };
        if k > usize::from(u16::MAX) {
            return false;
        }
        if seen.len() <= k {
            seen.resize(k + 1, false);
        }
        seen[k] = true;
    }
    seen.iter().all(|s| *s)
}

/// The bind plan's name of a placeholder (`RewrittenSql.plan.names`): a `:name`'s name, `?k`
/// for the k-th `?`, a positional placeholder's own text.
fn plan_name(text: &str, kind: &PlaceholderKind, at: TextRange) -> Arc<str> {
    match kind {
        PlaceholderKind::Named(n) => n.clone(),
        PlaceholderKind::Question(q) => Arc::from(format!("?{q}")),
        PlaceholderKind::Positional(_) => Arc::from(
            text.get(usize::from(at.start())..usize::from(at.end()))
                .unwrap_or(""),
        ),
    }
}

/// One value per driver slot, in slot order; `None` when a name has no value or a slot below
/// the highest is bound by no name (the server would expect a value nobody gave).
fn binds_of(plan: &BindPlan, known: &HashMap<Arc<str>, BindValue>) -> Option<Vec<BindValue>> {
    let total = plan
        .slots
        .iter()
        .flat_map(|s| s.iter())
        .map(|&k| usize::try_from(k).unwrap_or(usize::MAX))
        .max()
        .map_or(0, |m| m.saturating_add(1));
    if total > usize::from(u16::MAX) {
        return None;
    }
    let mut out: Vec<Option<BindValue>> = vec![None; total];
    for (name, slots) in plan.names.iter().zip(&plan.slots) {
        let v = known.get(name)?;
        for &k in slots.iter() {
            *out.get_mut(usize::try_from(k).ok()?)? = Some(v.clone());
        }
    }
    out.into_iter().collect()
}

/// The words after which a literal may be a secret (HV-EXEC-03; the redaction patterns of
/// HV-EXEC-12: `PASSWORD '…'`, `IDENTIFIED BY …`, `WITH PASSWORD = '…'`, `SET PASSWORD`,
/// connection strings with `password=`/`pwd=`), compared ASCII-case-insensitively.
const SECRET_MARKERS: [&str; 4] = ["password", "identified", "passwd", "pwd="];

/// `text` for the log and the history: when it mentions a password anywhere (code, string or
/// comment), every string literal becomes `'***'` and every comment `/* *** */` (`-- ***` for
/// a line comment); a token the lexer cannot read cleanly masks the rest of the text. Text
/// that mentions no password is returned as it is. Bind values never appear in `final_text`
/// (they travel as binds), so masking literals is all that is left. Over-masking is accepted:
/// the masked text is never sent.
pub fn mask_secrets(text: &str, ctx: &DialectCtx) -> Arc<str> {
    let lower = text.to_ascii_lowercase();
    let Some(first) = SECRET_MARKERS.iter().filter_map(|m| lower.find(m)).min() else {
        return Arc::from(text);
    };
    const MASK: &str = "***";
    let Ok(profile) = lex_profile(ctx) else {
        // No lexer for the dialect: nothing after the first marker is shown.
        return Arc::from(format!("{}{MASK}", &text[..first]));
    };
    let lexed = lex(text, profile, ctx.modes, &LexState::Initial);
    let mut out = String::with_capacity(text.len());
    let mut at = 0usize;
    for i in 0..lexed.len() {
        let r = lexed.range(i);
        let (s, e) = (usize::from(r.start()), usize::from(r.end()));
        let unclean = lexed
            .flags(i)
            .intersects(TokenFlags::UNTERMINATED | TokenFlags::ERROR)
            || lexed.kind(i) == TokenKind::Unknown;
        let replacement = match lexed.kind(i) {
            _ if unclean => {
                out.push_str(&text[at..s]);
                out.push_str(MASK);
                return Arc::from(out);
            }
            TokenKind::String
            | TokenKind::DollarString
            | TokenKind::BitString
            | TokenKind::HexString
            | TokenKind::InlineData => Some("'***'"),
            TokenKind::LineComment => Some("-- ***"),
            TokenKind::BlockComment | TokenKind::HintComment | TokenKind::ExecutableComment => {
                Some("/* *** */")
            }
            _ => None,
        };
        if let Some(rep) = replacement {
            out.push_str(&text[at..s]);
            out.push_str(rep);
            at = e;
        }
    }
    out.push_str(&text[at..]);
    Arc::from(out)
}
