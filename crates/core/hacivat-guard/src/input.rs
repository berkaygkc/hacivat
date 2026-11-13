//! Input limits checked before a statement reaches the server (SQL-SAFE-05).

/// The longest statement a session accepts, in bytes (1 MiB).
pub const MAX_STATEMENT_BYTES: usize = 1 << 20;

/// Why a statement is refused before it is sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum InputViolation {
    TooLong { limit: usize },
    Nul,
}

/// Whether `sql` may be sent: at most [`MAX_STATEMENT_BYTES`] bytes (UTF-8, not characters),
/// and no NUL, which C-string APIs (SQLite, libpq-style tools) would cut the statement at.
pub fn check_input(sql: &str) -> Result<(), InputViolation> {
    if sql.len() > MAX_STATEMENT_BYTES {
        return Err(InputViolation::TooLong {
            limit: MAX_STATEMENT_BYTES,
        });
    }
    if sql.as_bytes().contains(&0) {
        return Err(InputViolation::Nul);
    }
    Ok(())
}
