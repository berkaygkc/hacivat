//! Server names and lookup keys (HV-MODEL-01 "Names", HV-INTRO-14).
//!
//! A [`Name`] is the exact text the server reported: never normalized, never case-mapped.
//! An [`IdentKey`] is the separate lookup key, built only by a [`FoldRule`]. The only folding
//! here is ASCII (INV-3): locale casing never runs, so `İ`, `ı` and every other non-ASCII
//! letter stay exactly as written.

use std::sync::Arc;

use hacivat_proto::DbmsKind;

use crate::kind::KindGroup;

/// An object name exactly as the server reported it.
///
/// `Arc<str>` (as `sql-ident` uses): the cache decoder interns every string of a namespace,
/// so the 160k columns of a large schema share their few dozen type names.
pub type Name = Arc<str>;

/// How a catalog name turns into its [`IdentKey`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FoldRule {
    /// Byte equality: PostgreSQL (catalog names are already folded; the caller folds unquoted
    /// input with `sql-ident::fold` first), MySQL tables with `lower_case_table_names = 0`.
    Exact,
    /// ASCII letters compare case-insensitively; every other character compares exactly.
    AsciiCaseless,
}

/// The fold rules of one catalog: relation names (tables, views; MySQL's
/// `lower_case_table_names` decides them) and every other name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FoldPolicy {
    pub relations: FoldRule,
    pub other: FoldRule,
}

impl FoldPolicy {
    /// The default policy of `dbms`. MySQL and MariaDB relations default to [`FoldRule::Exact`]
    /// (`lower_case_table_names = 0`, the Linux default); the introspector overrides it from
    /// the server variable.
    pub fn for_dbms(dbms: DbmsKind) -> Self {
        use FoldRule::*;
        match dbms {
            DbmsKind::MySql | DbmsKind::MariaDb | DbmsKind::TiDb => FoldPolicy {
                relations: Exact,
                other: AsciiCaseless,
            },
            DbmsKind::Sqlite => FoldPolicy {
                relations: AsciiCaseless,
                other: AsciiCaseless,
            },
            _ => FoldPolicy {
                relations: Exact,
                other: Exact,
            },
        }
    }

    /// The rule for names in `group`.
    pub fn rule(&self, group: KindGroup) -> FoldRule {
        match group {
            KindGroup::Relations => self.relations,
            _ => self.other,
        }
    }
}

/// A lookup key: a [`Name`] folded by a [`FoldRule`]. Never displayed, never persisted.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct IdentKey(Arc<str>);

impl IdentKey {
    /// The key of the catalog name `name` under `rule`. Shares `name`'s allocation unless
    /// folding changes the text.
    pub fn new(rule: FoldRule, name: &Name) -> Self {
        match rule {
            FoldRule::AsciiCaseless if name.bytes().any(|b| b.is_ascii_uppercase()) => {
                IdentKey(Arc::from(name.to_ascii_lowercase()))
            }
            _ => IdentKey(name.clone()),
        }
    }

    /// The key of borrowed text (a lookup probe); allocates.
    pub fn of_str(rule: FoldRule, text: &str) -> Self {
        match rule {
            FoldRule::AsciiCaseless => IdentKey(Arc::from(text.to_ascii_lowercase())),
            FoldRule::Exact => IdentKey(Arc::from(text)),
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

// `Hash`/`Eq` of `IdentKey` are those of the `str` it holds, so map lookups can borrow.
impl std::borrow::Borrow<str> for IdentKey {
    fn borrow(&self) -> &str {
        &self.0
    }
}

/// Whether `a` and `b` are the same name under `rule`, without allocating.
pub fn name_eq(rule: FoldRule, a: &str, b: &str) -> bool {
    match rule {
        FoldRule::Exact => a == b,
        FoldRule::AsciiCaseless => a.eq_ignore_ascii_case(b),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ascii_caseless_never_touches_non_ascii() {
        let n: Name = Arc::from("MÜŞTERİ_Id");
        assert_eq!(
            IdentKey::new(FoldRule::AsciiCaseless, &n).as_str(),
            "mÜŞterİ_id"
        );
        assert_eq!(IdentKey::new(FoldRule::Exact, &n).as_str(), "MÜŞTERİ_Id");
        assert!(name_eq(FoldRule::AsciiCaseless, "Iı", "iı"));
        assert!(!name_eq(FoldRule::AsciiCaseless, "İ", "i"));
    }

    #[test]
    fn exact_key_shares_the_name() {
        let n: Name = Arc::from("users");
        let k = IdentKey::new(FoldRule::AsciiCaseless, &n);
        assert!(std::ptr::eq(k.as_str().as_ptr(), n.as_ptr()));
    }
}
