//! `gen-fn-catalog --check`: the offline freshness check of `sql-safety`'s function data
//! against `data/functions.digest` (Task 3). The catalog read itself is a Docker test,
//! `hacivat-exec/tests/gen_fn_catalog.rs` (P-M3b-13); xtask has no database client.

use std::process::ExitCode;

/// `cargo xtask gen-fn-catalog --check`. STUB: exits 2, naming the task that fills it.
pub fn run() -> ExitCode {
    crate::not_yet("gen-fn-catalog", "Task 3")
}
