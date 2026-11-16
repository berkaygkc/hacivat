//! Functions, operators, casts and types (`alg/13` SAFE-03; Task 3): the per-dialect allow and
//! deny data under `data/`, the harmless-`SET` allowlist (P-M3b-6) and the catalog seam (M4).
//!
//! STUB until Task 3 (fail closed): no function is listed as denied (the guard equality test
//! of P-M3b-13 compares against Task 3's data) and no `SET` is allowed.

pub mod data;

use sql_dialect::DialectId;
use sql_ident::QualifiedName;

/// A server function catalog (M4); `ClassifyCtx.catalog` is `None` in M3b.
pub trait FnCatalog: Send + Sync {
    fn lookup(&self, f: &QualifiedName, arity: Option<u16>) -> FnLookup;
}

/// What a catalog knows about a function.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FnLookup {
    Known { volatile: bool, writes: bool },
    Unknown,
}

/// The names on `d`'s function denylist, for the guard equality test (P-M3b-13). STUB: none.
pub fn denied_function_names(d: DialectId) -> Box<dyn Iterator<Item = &'static str>> {
    let _ = d;
    Box::new(std::iter::empty())
}

/// Whether a read-only session may run `SET name …` on `d` (P-M3b-6). STUB: `false`.
pub fn set_allowed(d: DialectId, name: &str) -> bool {
    let _ = (d, name);
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stubs_fail_closed() {
        assert_eq!(denied_function_names(DialectId::Postgres).count(), 0);
        assert!(!set_allowed(DialectId::Postgres, "search_path"));
        assert!(!set_allowed(DialectId::MySql, "time_zone"));
    }
}
