// ## m4-t01c-core
//! Object filters (HV-INTRO-03): which objects of a namespace are read (`Introspect`) or shown
//! (`View`). Task 12 writes the grammar, the matcher and the server-side pushdown (positive
//! globs only); this module builds without the `session` feature (the fuzzed grammar).
//!
//! **Fail-closed stub (M4 Task 1c).** [`parse_filter`] accepts any text and the compiled
//! filter matches every object (nothing is hidden or skipped by mistake) and pushes nothing
//! down to the server.

use hacivat_meta::{Name, ObjectKind};
use hacivat_values::BindValue;
use sql_dialect::DialectId;
use sql_ident::key::IdentCtx;

/// What a filter decides.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FilterRole {
    /// Objects that are not read from the server at all.
    Introspect,
    /// Objects that are read but hidden in the explorer.
    View,
}

/// A compiled object filter.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CompiledFilter {
    text: String,
    dialect: DialectId,
    role: FilterRole,
}

/// A `WHERE` fragment a filter pushes down to the catalog query (Task 12; binds only).
#[derive(Debug, Clone, PartialEq)]
pub struct SqlPredicate {
    pub sql_fragment: &'static str,
    pub binds: Vec<BindValue>,
}

impl CompiledFilter {
    /// Whether an object of `kind` named `name` passes the filter.
    ///
    /// Stub: every object passes.
    pub fn matches(&self, kind: ObjectKind, name: &Name) -> bool {
        let _ = (kind, name);
        true
    }

    /// The server-side form of the filter's positive globs for `kind`, when it has one.
    ///
    /// Stub: never (the client filters everything).
    pub fn pushdown(
        &self,
        kind: ObjectKind,
        dialect: DialectId,
        ident: &IdentCtx,
    ) -> Option<SqlPredicate> {
        let _ = (kind, dialect, ident);
        None
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn role(&self) -> FilterRole {
        self.role
    }

    pub fn dialect(&self) -> DialectId {
        self.dialect
    }
}

/// Why a filter does not parse (`intro-filter-*`, with the offending item).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum FilterError {
    UnknownKind { item: String },
    BadRegex { item: String },
    BadGlob { item: String },
}

/// Parses an object filter of `dialect` for `role`.
///
/// Stub: accepts any text; the result matches everything.
pub fn parse_filter(
    text: &str,
    dialect: DialectId,
    role: FilterRole,
) -> Result<CompiledFilter, FilterError> {
    Ok(CompiledFilter {
        text: text.to_owned(),
        dialect,
        role,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_stub_filter_hides_nothing_and_pushes_nothing_down() {
        let f = parse_filter("!tmp_*", DialectId::Postgres, FilterRole::View).unwrap();
        assert!(f.matches(ObjectKind::Table, &Name::from("tmp_1")));
        assert_eq!(
            f.pushdown(
                ObjectKind::Table,
                DialectId::Postgres,
                &IdentCtx::new(DialectId::Postgres)
            ),
            None
        );
    }
}
