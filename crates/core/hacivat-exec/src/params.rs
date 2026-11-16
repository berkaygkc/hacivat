// ## m3b-t01c-core-seams
//! Parameter preparation (HV-EXEC-03 binding, SQL-PARAM-01..03; Task 15): each unit's
//! placeholders found by `sql_split::scan_params`, its values asked for (`NeedsValues`) or
//! remembered per `(profile, name)` for the app session (R-M3-79), the text rewritten to the
//! engine's placeholders (`final_text`, which is exactly what is classified and sent:
//! P-M3b-2), the values masked for the log and the history, and a map from the rewritten text
//! back to the document (`OffsetMap`, for error positions).
//!
//! STUB until Task 15 (fail closed, F31): a unit holding any `:`, `$` or `?` byte is refused
//! (`ParamDiag::TooManyBinds`: "parameters arrive in a later build"); every other unit is
//! `Ready` as it is, with an identity map. Nothing is wired into Run before Task 24 (R-M3-52).

use std::ops::Range;
use std::sync::Arc;

use hacivat_proto::ProfileId;
use hacivat_values::BindValue;
use sql_dialect::DialectCtx;
use sql_split::{ParamDiag, ParamSettings};

use crate::units::ExecUnit;

/// A piecewise-linear map from offsets in a unit's final text to document offsets. Each piece
/// starts at a unit offset; an offset maps by its piece's shift.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct OffsetMap {
    /// `(unit_start, doc_start)` per piece, by `unit_start`; never empty.
    pieces: Vec<(usize, usize)>,
}

impl OffsetMap {
    /// The map of a unit sent as written, starting at document offset `start`.
    pub fn identity(start: usize) -> Self {
        OffsetMap {
            pieces: vec![(0, start)],
        }
    }

    /// The document offset of `unit_offset`.
    pub fn to_doc(&self, unit_offset: usize) -> usize {
        let (unit_start, doc_start) = self
            .pieces
            .iter()
            .rev()
            .find(|(u, _)| *u <= unit_offset)
            .copied()
            .unwrap_or(self.pieces[0]);
        doc_start + unit_offset.saturating_sub(unit_start)
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
    /// The value remembered for this profile in this app session.
    pub remembered: Option<BindValue>,
}

/// What preparing a run's units gave.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Prepared {
    Ready(Vec<PreparedUnit>),
    /// Ask for these, then prepare again.
    NeedsValues(Vec<ParamPrompt>),
    /// Unit `unit` (its index) cannot be sent: `at` in the document, `diag` why.
    Refused {
        unit: u32,
        at: Range<usize>,
        diag: ParamDiag,
    },
}

/// The values remembered per `(profile, name)` for the app session only (never persisted,
/// R-M3-79).
pub trait ParamValueStore {
    fn get(&self, profile: &ProfileId, name: &str) -> Option<BindValue>;
    fn put(&self, profile: &ProfileId, name: &str, v: BindValue);
}

/// Prepares `units` of a run on `profile` (the module docs).
///
/// STUB until Task 15: `Refused { diag: TooManyBinds }` at the first `:`, `$` or `?` byte of
/// the first unit holding one; else `Ready`, every unit as written with an identity map.
pub fn prepare_units(
    units: Vec<ExecUnit>,
    ctx: &DialectCtx,
    s: &ParamSettings,
    values: &dyn ParamValueStore,
    profile: &ProfileId,
) -> Prepared {
    let _ = (ctx, s, values, profile);
    for (i, unit) in units.iter().enumerate() {
        if let Some(at) = unit
            .text
            .bytes()
            .position(|b| matches!(b, b':' | b'$' | b'?'))
        {
            let start = unit.doc_range.start + at;
            return Prepared::Refused {
                unit: u32::try_from(i).unwrap_or(u32::MAX),
                at: start..start + 1,
                diag: ParamDiag::TooManyBinds,
            };
        }
    }
    Prepared::Ready(
        units
            .into_iter()
            .map(|unit| PreparedUnit {
                final_text: unit.text.clone(),
                binds: Arc::from(Vec::new()),
                masked: unit.text.clone(),
                offsets: OffsetMap::identity(unit.doc_range.start),
                unit,
            })
            .collect(),
    )
}
