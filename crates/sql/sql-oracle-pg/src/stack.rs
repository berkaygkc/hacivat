//! The parser's own thread with a 16 MiB stack (R-M3-3 on M3a's one parser call, R-M3-35;
//! Task 9).

/// The deepest lexer nesting (`Lexed::max_paren_depth`) a unit may have to be sent to the
/// oracle (R-M3-35).
pub const MAX_ORACLE_NESTING: u16 = 256;

/// Runs `f` on a fresh thread with a 16 MiB stack and joins it; `None` if it panicked (Task 9).
///
/// Task 1b stub (fail closed): `None`, without running `f`.
pub fn on_parser_stack<R: Send + 'static>(f: impl FnOnce() -> R + Send + 'static) -> Option<R> {
    drop(f);
    None
}
