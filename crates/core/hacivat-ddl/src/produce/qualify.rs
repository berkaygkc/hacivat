//! Schema qualification (HV-DDL-10): `Auto (default) | Never | Always` for display, copy and
//! file output; scripts executed against a live target are always fully qualified (the caller
//! passes `Always` for them). M4 Task 7 body (spike origin `da358e4`).
//!
//! - `Always`: every name that has a schema keeps it.
//! - `Never`: no name does.
//! - `Auto`: a name stays bare when its schema is the current schema **and** the name is not
//!   ambiguous in the scope (another schema of the scope has an object of that name);
//!   otherwise it is qualified. With no current schema every name is qualified. On SQLite the
//!   current schema plays no part: `temp` and attached schemas are always qualified, and a
//!   `main` name stays bare only when it is not ambiguous (the plan's rule "qualify names that
//!   exist in more than one schema of the scope"; SQLite resolves a bare name in `temp` first,
//!   so a bare `t` would name `temp.t` when both exist).
//!
//! Only names Hacivat writes are qualified; server text is never rewritten.

use std::collections::BTreeSet;

use hacivat_meta::name::Name;
use smallvec::SmallVec;
use sql_dialect::DialectId;
use sql_ident::QualifiedName;
use sql_ident::ident::Ident;

use crate::DdlTarget;
use crate::options::Qualify;

/// What qualification depends on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QualifyCtx {
    pub mode: Qualify,
    pub dialect: DialectId,
    /// The session's current schema (PostgreSQL: the first `search_path` entry; MySQL: the
    /// current database).
    pub current_schema: Option<Name>,
    /// Names that exist in more than one schema of the scope.
    pub ambiguous: BTreeSet<Name>,
}

impl QualifyCtx {
    pub fn new(mode: Qualify, dialect: DialectId, current_schema: Option<Name>) -> Self {
        QualifyCtx {
            mode,
            dialect,
            current_schema,
            ambiguous: BTreeSet::new(),
        }
    }

    /// `mode` for `target`: its current schema, else the first `search_path` entry.
    pub fn for_target(target: &DdlTarget, mode: Qualify) -> Self {
        let current = target
            .current_schema
            .clone()
            .or_else(|| target.search_path.first().cloned());
        QualifyCtx::new(mode, target.dialect, current)
    }

    /// Adds names that exist in more than one schema of the scope.
    pub fn with_ambiguous<I: IntoIterator<Item = Name>>(mut self, names: I) -> Self {
        self.ambiguous.extend(names);
        self
    }

    fn show_schema(&self, schema: &Name, name: &Name) -> bool {
        match self.mode {
            Qualify::Always => true,
            Qualify::Never => false,
            Qualify::Auto if self.dialect == DialectId::Sqlite => {
                &**schema != "main" || self.ambiguous.contains(name)
            }
            Qualify::Auto => {
                self.current_schema.as_ref() != Some(schema) || self.ambiguous.contains(name)
            }
        }
    }
}

/// A name as generated text writes it: with its schema or bare.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct QualifiedRef {
    pub schema: Option<Name>,
    pub name: Name,
}

impl QualifiedRef {
    pub fn is_qualified(&self) -> bool {
        self.schema.is_some()
    }

    /// The IR form: `[schema, name]` or `[name]`.
    pub fn to_qualified_name(&self) -> QualifiedName {
        let part = |text: &Name| Ident {
            text: text.clone(),
            quoted: false,
        };
        let mut parts: SmallVec<[Ident; 3]> = SmallVec::new();
        if let Some(s) = &self.schema {
            parts.push(part(s));
        }
        parts.push(part(&self.name));
        QualifiedName { parts }
    }
}

/// `name` (its last part the object, the one before it the schema; a database part before
/// the schema is never written) as `ctx` wants it written.
pub fn qualify(name: &QualifiedName, ctx: &QualifyCtx) -> QualifiedRef {
    let mut parts = name.parts.iter().rev();
    let object = parts
        .next()
        .map_or_else(|| Name::from(""), |p| p.text.clone());
    let schema = parts
        .next()
        .map(|p| p.text.clone())
        .filter(|s| ctx.show_schema(s, &object));
    QualifiedRef {
        schema,
        name: object,
    }
}
