//! Server versions and the lexer profile's version string (Task 3).

use crate::id::DialectCtx;

/// A server version; versions order by major, minor, patch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub struct ServerVersion {
    pub major: u16,
    pub minor: u16,
    pub patch: u16,
}

/// The version of the lexer profile `ctx` selects, e.g. `"lex:pg-18@2026.11.1"`: caches of
/// lexed text key on it (Task 3).
pub fn profile_version(ctx: &DialectCtx) -> &'static str {
    let _ = ctx;
    unimplemented!("profile_version: Task 3")
}
