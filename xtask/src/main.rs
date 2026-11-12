mod bench_report;
mod check_deps;

use std::process::ExitCode;

fn main() -> ExitCode {
    match std::env::args().nth(1).as_deref() {
        Some("bench-report") => bench_report::run(),
        Some("check-deps") => check_deps::run(),
        other => {
            eprintln!("unknown xtask command: {other:?}\navailable: bench-report, check-deps");
            ExitCode::from(2)
        }
    }
}
