//! `fetch-pg-regress`: fetches the PostgreSQL regression corpus at a pinned tag into
//! `target-pg-regress/` (git-ignored; never vendored; third-party content is data; Task 20).

use std::process::ExitCode;

/// `cargo xtask fetch-pg-regress`. STUB: exits 2, naming the task that fills it.
pub fn run() -> ExitCode {
    crate::not_yet("fetch-pg-regress", "Task 20")
}
