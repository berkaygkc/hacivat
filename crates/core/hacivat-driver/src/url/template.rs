//! URL templates: compile DataGrip-style template text into nodes (HV-DRV-02). Body and the
//! private node type in Task 3.

use std::collections::BTreeSet;

use super::FieldId;

/// A compiled URL template (HV-DRV-02).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Template {
    pub id: String,
    pub field_set: BTreeSet<FieldId>,
    /// The template ends in a pass-through of extra parameters.
    pub has_passthrough: bool,
    #[allow(dead_code, reason = "Task 3")]
    nodes: Vec<Node>,
}

/// A template node (Task 3).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum Node {}

impl Template {
    /// The template's default for `f` (`{host::localhost}` gives `localhost`) (Task 3).
    pub fn default_of(&self, _f: FieldId) -> Option<&str> {
        None
    }
}

/// Why a template failed to compile (HV-DRV-02).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum TemplateError {
    Unbalanced { pos: usize },
    BadRegex { pos: usize },
    UnknownField { name: String },
    TooManyFields { pos: usize },
}

/// Compiles template text `src` as template `id` (Task 3).
pub fn compile(_src: &str, _id: &str) -> Result<Template, TemplateError> {
    Err(TemplateError::Unbalanced { pos: 0 })
}
