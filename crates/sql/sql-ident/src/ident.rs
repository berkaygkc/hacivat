//! An identifier as written and a qualified name (Task 3 fills the parsing).

use std::sync::Arc;

use smallvec::SmallVec;

/// One identifier: its text (without the quotes, doubled quotes undone) and whether it was
/// quoted. The text is never case-folded here; [`crate::fold`] does that per dialect.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Ident {
    pub text: Arc<str>,
    pub quoted: bool,
}

/// `name`, `schema.name` or `db.schema.name`, outermost part first.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct QualifiedName {
    pub parts: SmallVec<[Ident; 3]>,
}
