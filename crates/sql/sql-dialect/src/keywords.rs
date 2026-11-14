//! Keyword tables and lookup (SQL-LEX-01 tables, SQL-LEX-04 lookup; Task 3). Lookup is
//! ASCII-only through an ASCII-upper key of at most 32 bytes: Turkish `ı`/`İ` and look-alike
//! letters never match a keyword.

use crate::id::DialectId;

/// A keyword's ID, shared by every dialect's table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct KeywordId(pub u16);

/// PostgreSQL's keyword categories (`kwlist.h`); other dialects use `Reserved` and
/// `Unreserved`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum KwCategory {
    Unreserved,
    ColName,
    TypeFuncName,
    Reserved,
}

/// A keyword of a dialect's table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct KwEntry {
    pub id: KeywordId,
    pub category: KwCategory,
}

/// `word` as a keyword of `d`: ASCII-only, at most 32 bytes, else `None` (Task 3). Task 1b
/// stub: no table, so no word is a keyword.
pub fn keyword_lookup(d: DialectId, word: &str) -> Option<KwEntry> {
    let _ = (d, word);
    None
}

/// The canonical upper-case text of `id` (Task 3).
pub fn keyword_text(id: KeywordId) -> &'static str {
    unimplemented!("keyword_text({id:?}): Task 3")
}

/// The keywords other crates name. Task 1b gives them placeholder values (Rust needs values);
/// Task 3 generates the full set from the tables (P22).
pub mod kw {
    use super::KeywordId;

    pub const SELECT: KeywordId = KeywordId(1);
    pub const VALUES: KeywordId = KeywordId(2);
    pub const TABLE: KeywordId = KeywordId(3);
    pub const WITH: KeywordId = KeywordId(4);
    pub const SHOW: KeywordId = KeywordId(5);
    pub const INSERT: KeywordId = KeywordId(6);
    pub const UPDATE: KeywordId = KeywordId(7);
    pub const DELETE: KeywordId = KeywordId(8);
    pub const MERGE: KeywordId = KeywordId(9);
    pub const INTO: KeywordId = KeywordId(10);
    pub const FOR: KeywordId = KeywordId(11);
    pub const LOCK: KeywordId = KeywordId(12);
    pub const SET: KeywordId = KeywordId(13);
    pub const BEGIN: KeywordId = KeywordId(14);
    pub const END: KeywordId = KeywordId(15);
}
