//! Template variables (D-053; Task 6): `${var}`, `%var%` and `<var>` are detected, never
//! substituted; [`crate::params::scan_params`] reports them with `ParamDiag::TemplateVariable`.

use std::sync::Arc;

use sql_text::TextRange;

/// The template syntax a variable was written in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TemplatePattern {
    /// `${var}`
    DollarBrace,
    /// `%var%`
    Percent,
    /// `<var>`
    Angle,
}

/// One template variable in a unit.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TemplateVar {
    pub range: TextRange,
    pub name: Arc<str>,
    pub pattern: TemplatePattern,
}
