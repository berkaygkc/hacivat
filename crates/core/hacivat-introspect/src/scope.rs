// ## m4-t01c-core
//! Schema scope patterns (HV-INTRO-02): which namespaces a profile introspects
//! (`@:*`, `db1:public,app_*`, `*:!pg_temp*`). Task 12 writes the grammar, the matcher and the
//! server-side pushdown; this module builds without the `session` feature (the fuzzed grammar).
//!
//! **Fail-closed stub (M4 Task 1c).** [`parse_scope`] keeps the text and accepts it, and
//! [`ScopePattern::select`] selects only `@:@`, the current namespace, whatever the pattern
//! says: never more than the user can see is read, and never a namespace that is not there.

use std::collections::BTreeSet;

use hacivat_meta::{Name, NsKey, NsKind};
use sql_dialect::DialectId;

/// A namespace the server lists (`Introspector::list_namespaces`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct NsInfo {
    pub key: NsKey,
    /// A system schema (`pg_catalog`, `information_schema`, `mysql`, …): never introspected
    /// live by default; it comes from the snapshots (R-M4-6, R-M4-14).
    pub system: bool,
}

/// What `@` means: the session's current database and schema (MySQL/MariaDB: the database is
/// the schema; SQLite: `main`).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct CurrentCtx {
    pub database: Option<Name>,
    pub schema: Option<Name>,
}

/// A parsed scope pattern.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ScopePattern {
    text: String,
    dialect: DialectId,
}

impl ScopePattern {
    /// The namespaces of `infos` the pattern selects (the union of its positive clauses minus
    /// its negative ones; Task 12).
    ///
    /// Stub: only `@:@` — the schema namespace that is `ctx`'s current database and schema (one
    /// level, `[schema]`, on MySQL, MariaDB and SQLite) — when `infos` lists it; nothing when
    /// the current namespace is unknown (`intro-scope-current-unknown`).
    pub fn select(&self, infos: &[NsInfo], ctx: &CurrentCtx) -> BTreeSet<NsKey> {
        let current = match (self.dialect, &ctx.database, &ctx.schema) {
            (DialectId::Postgres, Some(db), Some(schema)) => {
                NsKey::schema([db.clone(), schema.clone()])
            }
            (DialectId::Postgres, _, _) => return BTreeSet::new(),
            (_, _, Some(schema)) => NsKey::schema([schema.clone()]),
            (_, Some(db), None) => NsKey::schema([db.clone()]),
            (_, None, None) => return BTreeSet::new(),
        };
        infos
            .iter()
            .filter(|i| i.key.kind == NsKind::Schema && i.key == current)
            .map(|i| i.key.clone())
            .collect()
    }

    /// The pattern as text (Task 12: a round trip through the syntax tree).
    pub fn to_text(&self) -> &str {
        &self.text
    }

    pub fn dialect(&self) -> DialectId {
        self.dialect
    }
}

/// Why a scope pattern does not parse.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ScopeError {
    /// The 1-based position, in Unicode scalar values (never bytes).
    pub column: u32,
    pub kind: ScopeErrorKind,
}

/// The closed set of scope errors (`intro-scope-err-*`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ScopeErrorKind {
    UnclosedBrace,
    BadRegex,
    /// Any other syntax error (`intro-scope-syntax-error`).
    Syntax,
}

/// Parses a scope pattern of `dialect`.
///
/// Stub: accepts any text and keeps it (the matcher selects only `@:@` until Task 12).
pub fn parse_scope(text: &str, dialect: DialectId) -> Result<ScopePattern, ScopeError> {
    Ok(ScopePattern {
        text: text.to_owned(),
        dialect,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(path: &[&str]) -> NsInfo {
        NsInfo {
            key: NsKey::schema(path.iter().copied()),
            system: false,
        }
    }

    fn ctx(db: Option<&str>, schema: Option<&str>) -> CurrentCtx {
        CurrentCtx {
            database: db.map(Name::from),
            schema: schema.map(Name::from),
        }
    }

    #[test]
    fn one_level_dialects_select_their_current_schema_only() {
        let infos = [info(&["shop"]), info(&["orders"])];
        let p = parse_scope("*", DialectId::MySql).unwrap();
        let got = p.select(&infos, &ctx(Some("shop"), Some("shop")));
        assert_eq!(got, BTreeSet::from([NsKey::schema(["shop"])]));
        assert!(p.select(&infos, &ctx(None, None)).is_empty());
        assert_eq!(p.to_text(), "*");
    }

    #[test]
    fn postgres_needs_both_the_database_and_the_schema() {
        let infos = [info(&["db", "public"])];
        let p = parse_scope("*:*", DialectId::Postgres).unwrap();
        assert!(p.select(&infos, &ctx(Some("db"), None)).is_empty());
        // A current namespace the server does not list is never invented.
        assert!(p.select(&infos, &ctx(Some("db"), Some("app"))).is_empty());
    }
}
