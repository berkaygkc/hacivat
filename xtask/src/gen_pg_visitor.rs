//! `gen-pg-visitor`: writes `sql-safety/src/visit/pg_generated.rs` from the vendored
//! `third_party/pg_query/protobuf/pg_query.proto`, without `protoc` (R-M3-2; Task 9).

use std::process::ExitCode;

/// `cargo xtask gen-pg-visitor`. STUB: exits 2, naming the task that fills it.
pub fn run() -> ExitCode {
    crate::not_yet("gen-pg-visitor", "Task 9")
}
