//! G1 in the real app (M4 Task 29; feature `bench`): the cached catalog of the 160k-column
//! fixture on screen in < 300 ms (spec §3.3, M4 exit), p95 over cold processes. See
//! `hacivat_app::bench::catalog` and `docs/benchmarks/m4-bench-notes.md`.
//!
//! ```text
//! HACIVAT_G1_FIXTURE=$PWD/target/bench/g1-160k.v1.sqlite \
//!   cargo test --release -p hacivat-meta --test bench_fixture -- --ignored write_g1_fixture
//! cargo run --release -p hacivat-app --features bench --example catalog_cold -- \
//!   --fixture target/bench/g1-160k.v1.sqlite --runs 10 --offline
//! ```
//!
//! Each run is a new process (the bench starts itself with `--child`). The plaintext variant is
//! graded (`RESULT: PASS|FAIL|INVALID g1-catalog-cold`, the last line); the encrypted variant is
//! recorded only (`--variant plain|encrypted|both`, `--key-delay-ms N` models a slow keyring).
//! Keep the window frontmost: a run whose window was not key at t1 makes the result INVALID
//! (exit 3) unless `--allow-inactive`.

use std::process::ExitCode;
use std::time::Instant;

use hacivat_app::bench::EXIT_USAGE;
use hacivat_app::bench::catalog;

fn main() -> ExitCode {
    // The process start, as close as the example gets to it (§3.3's "from process start").
    let started = Instant::now();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let args = match catalog::parse_args(&args) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("catalog_cold: {e}\n{}", catalog::USAGE);
            return ExitCode::from(EXIT_USAGE);
        }
    };
    match args.child {
        Some(variant) => catalog::run_child(&args, variant, started),
        None => catalog::run(&args),
    }
}
