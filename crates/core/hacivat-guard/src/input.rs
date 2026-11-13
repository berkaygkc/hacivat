//! Input limits checked before a statement reaches the server (SQL-SAFE-05). Task 5 fills
//! [`check_input`].

/// The longest statement a session accepts, in bytes (1 MiB).
pub const MAX_STATEMENT_BYTES: usize = 1 << 20;

/// Why a statement is refused before it is sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum InputViolation {
    TooLong { limit: usize },
    Nul,
}

/// Whether `sql` may be sent. Task 1 stub: fails closed (every statement is refused as too
/// long) until Task 5.
pub fn check_input(_sql: &str) -> Result<(), InputViolation> {
    Err(InputViolation::TooLong {
        limit: MAX_STATEMENT_BYTES,
    })
}
