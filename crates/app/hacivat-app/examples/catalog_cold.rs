//! G1 in the real app (Task 29; feature `bench`): the cached catalog of the 160k-column fixture
//! on screen in < 300 ms (spec §3.3, M4 exit). See `docs/benchmarks/m4-bench-notes.md` once
//! Task 29 writes it.
//!
//! ```text
//! cargo run --release -p hacivat-app --features bench --example catalog_cold -- …
//! ```
//!
//! M4 Task 1a stub (P-M4-19, the m3a-t01b-seams precedent): fail-closed. It measures nothing,
//! prints `RESULT: n/a` and exits 2, so no run can mistake it for a pass.

use std::process::ExitCode;

fn main() -> ExitCode {
    eprintln!("catalog_cold: not implemented (M4 Task 29)");
    println!("RESULT: n/a");
    ExitCode::from(2)
}
