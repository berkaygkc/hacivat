//! Bind parameters (SQL-PARAM-01, -02; Task 6): placeholders in one unit, their bind plan and
//! the rewrite to the driver's syntax (PG `:name` → `$k`, MySQL/SQLite `:name` → `?`).
//!
//! STUB until Task 6 (fail closed): every `:`, `$` and `?` byte is reported as an unbindable
//! placeholder and nothing is rewritten, so Run refuses any unit holding one ("parameters
//! arrive in a later build").

use std::sync::Arc;

use smallvec::SmallVec;
use sql_dialect::DialectCtx;
use sql_text::{TextRange, TextSize};

use crate::item::StmtHead;
use crate::templates::TemplateVar;

/// The user's parameter settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ParamSettings {
    /// `?` is a parameter on PostgreSQL (off by default, D-08: it is also a jsonb operator).
    pub pg_question_marks: bool,
    /// `:name` is a parameter.
    pub colon_params: bool,
    /// Placeholders inside DDL statements are bindable.
    pub in_ddl: bool,
}

impl Default for ParamSettings {
    fn default() -> Self {
        ParamSettings {
            pg_question_marks: false,
            colon_params: true,
            in_ddl: false,
        }
    }
}

/// How a placeholder is written. Names compare exactly (`:Id` ≠ `:id`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum PlaceholderKind {
    /// `$n`
    Positional(u32),
    /// `?`, numbered from 1 in text order
    Question(u32),
    /// `:name`
    Named(Arc<str>),
}

/// One placeholder in a unit.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Placeholder {
    pub range: TextRange,
    pub kind: PlaceholderKind,
    pub bindable: bool,
}

/// Name `i` binds the driver slots `slots[i]` (MySQL repeats a name once per occurrence).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct BindPlan {
    pub names: Vec<Arc<str>>,
    pub slots: Vec<SmallVec<[u32; 2]>>,
}

/// The unit as the driver receives it, with the plan and every replacement as (range in the
/// unit, range in the rewritten text).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RewrittenSql {
    pub text: String,
    pub plan: BindPlan,
    pub edits: Vec<(TextRange, TextRange)>,
}

/// A parameter diagnostic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ParamDiag {
    TemplateVariable,
    QuestionMarkOff,
    ParamInDdl,
    MixedStyles,
    TooManyBinds,
}

/// What [`scan_params`] found in one unit.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct ParamScan {
    pub placeholders: Vec<Placeholder>,
    pub templates: Vec<TemplateVar>,
    /// `None` when nothing is bindable (or, in the stub, always).
    pub rewritten: Option<RewrittenSql>,
    pub diags: Vec<(TextRange, ParamDiag)>,
}

/// The placeholders, templates and rewrite of `unit` (one statement) in `ctx`.
///
/// STUB: every `:`, `$` and `?` byte is an unbindable placeholder (`Named("")`,
/// `Positional(0)`, `Question(0)`); `rewritten` is `None`.
pub fn scan_params(unit: &str, ctx: &DialectCtx, head: &StmtHead, s: &ParamSettings) -> ParamScan {
    let _ = (ctx, head, s);
    let placeholders = unit
        .bytes()
        .enumerate()
        .filter_map(|(at, b)| {
            let kind = match b {
                b':' => PlaceholderKind::Named(Arc::from("")),
                b'$' => PlaceholderKind::Positional(0),
                b'?' => PlaceholderKind::Question(0),
                _ => return None,
            };
            let start = TextSize::try_from(at).ok()?;
            Some(Placeholder {
                range: TextRange::at(start, TextSize::from(1)),
                kind,
                bindable: false,
            })
        })
        .collect();
    ParamScan {
        placeholders,
        ..ParamScan::default()
    }
}

#[cfg(test)]
mod tests {
    use sql_dialect::DialectId;

    use super::*;

    #[test]
    fn stub_reports_every_marker_byte_unbindable() {
        let ctx = DialectCtx::new(DialectId::Postgres, None);
        let s = scan_params(
            "SELECT :a, $1, ?, a::int",
            &ctx,
            &StmtHead::default(),
            &ParamSettings::default(),
        );
        assert_eq!(s.placeholders.len(), 5);
        assert!(s.placeholders.iter().all(|p| !p.bindable));
        assert_eq!(s.placeholders[0].range, TextRange::at(7.into(), 1.into()));
        assert!(s.rewritten.is_none());
        assert!(
            scan_params(
                "SELECT 1",
                &ctx,
                &StmtHead::default(),
                &ParamSettings::default()
            )
            .placeholders
            .is_empty()
        );
    }

    #[test]
    fn default_settings_follow_d08() {
        let s = ParamSettings::default();
        assert!(!s.pg_question_marks && s.colon_params && !s.in_ddl);
    }
}
