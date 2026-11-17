//! HV-DDL-10 schema qualification: whether a generated reference carries its schema.
//!
//! - `Always`: every name that has a schema is qualified.
//! - `Never`: no name is.
//! - `Auto`: a name stays bare when its schema is the current schema **and** the name is not
//!   ambiguous in the scope (no other schema has an object of that name); otherwise it is
//!   qualified. With no current schema, every name is qualified. SQLite is the exception:
//!   `main` stays bare and attached schemas (and `temp`) are qualified, whatever the
//!   current schema.
//!
//! Server text is never re-qualified: the plan only applies to names Hacivat writes.

use std::collections::BTreeSet;

use hacivat_meta::Name;
use sql_dialect::DialectId;

use crate::ir::QName;
use crate::options::Qualify;

/// The qualification decision for one script.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QualifyPlan {
    pub mode: Qualify,
    pub dialect: DialectId,
    pub current_schema: Option<Name>,
    /// Names that exist in more than one schema of the scope.
    pub ambiguous: BTreeSet<Name>,
}

impl QualifyPlan {
    pub fn new(mode: Qualify, dialect: DialectId, current_schema: Option<Name>) -> Self {
        QualifyPlan {
            mode,
            dialect,
            current_schema,
            ambiguous: BTreeSet::new(),
        }
    }

    /// Every name qualified (what an executed migration uses, HV-DDL-10).
    pub fn always(dialect: DialectId) -> Self {
        QualifyPlan::new(Qualify::Always, dialect, None)
    }

    /// No name qualified.
    pub fn never(dialect: DialectId) -> Self {
        QualifyPlan::new(Qualify::Never, dialect, None)
    }

    pub fn with_ambiguous<I: IntoIterator<Item = Name>>(mut self, names: I) -> Self {
        self.ambiguous.extend(names);
        self
    }

    /// Whether `q` is printed with its schema.
    pub fn show_schema(&self, q: &QName) -> bool {
        let Some(schema) = &q.schema else {
            return false;
        };
        match self.mode {
            Qualify::Always => true,
            Qualify::Never => false,
            Qualify::Auto if self.dialect == DialectId::Sqlite => &**schema != "main",
            Qualify::Auto => {
                self.current_schema.as_ref() != Some(schema) || self.ambiguous.contains(&q.name)
            }
        }
    }
}
