// ## m4-t01c-core
//! Glob patterns of the scope and filter grammars (Task 12): `*`, `?` and literal text,
//! translated to an anchored regular expression; case handling follows the server's identifier
//! rule (HV-INTRO-14), never locale folding.
//!
//! **Fail-closed stub (M4 Task 1c).** [`Glob::new`] answers [`GlobError::NotImplemented`]; the
//! stub scope and filter never call it.

/// How a glob compares letters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GlobCase {
    /// Byte for byte (PostgreSQL quoted names, MySQL table names under
    /// `lower_case_table_names = 0`).
    Exact,
    /// ASCII letters only, the server's rule (never Unicode or locale folding).
    AsciiInsensitive,
}

/// A compiled glob.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Glob {
    pattern: String,
    case: GlobCase,
}

/// Why a glob does not compile.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GlobError {
    /// Not written yet (M4 Task 1c stub).
    NotImplemented,
    /// A trailing escape character.
    TrailingEscape,
}

impl Glob {
    /// Compiles `pattern`.
    ///
    /// Stub: [`GlobError::NotImplemented`].
    pub fn new(pattern: &str, case: GlobCase) -> Result<Glob, GlobError> {
        let _ = (pattern, case);
        Err(GlobError::NotImplemented)
    }

    /// Whether `name` matches.
    ///
    /// Stub: never (and no `Glob` is built until Task 12).
    pub fn is_match(&self, name: &str) -> bool {
        let _ = (&self.pattern, self.case, name);
        false
    }
}
