mod check_deps;

use std::process::ExitCode;

fn main() -> ExitCode {
    match std::env::args().nth(1).as_deref() {
        Some("check-deps") => check_deps::run(),
        other => {
            eprintln!("unknown xtask command: {other:?}\navailable: check-deps");
            ExitCode::from(2)
        }
    }
}
