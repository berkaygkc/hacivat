//! A generated script: its statements and the notices shown beside it (Fluent ids, never
//! English text; never a script that silently lacks part of the object).

use crate::render::Stmt;

/// A notice attached to a script; the UI maps `id` to Fluent with `args`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notice {
    /// The Fluent message id (`ddl-grants-not-loaded`, `ddl-policies-not-loaded`, …).
    pub id: &'static str,
    /// Named arguments, preformatted (P31).
    pub args: Vec<(&'static str, String)>,
}

impl Notice {
    /// A notice without arguments.
    pub fn new(id: &'static str) -> Self {
        Notice {
            id,
            args: Vec::new(),
        }
    }
}

/// The statements of a script, in order, and its notices.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Script {
    pub stmts: Vec<Stmt>,
    pub notices: Vec<Notice>,
}
