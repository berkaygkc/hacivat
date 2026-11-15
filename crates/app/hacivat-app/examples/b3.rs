//! B3 in the real app (Task 36; feature `bench`): typing, scrolling, opening and the IME's
//! bounds question in a console's editor over the B3 fixture. See `hacivat_app::bench::b3`.
//!
//! ```text
//! cargo xtask gen-sql --lines 50000 --long-line 100000 --out target/fixtures/b3-50k.sql
//! cargo run --release -p hacivat-app --features bench --example b3 -- \
//!     --file target/fixtures/b3-50k.sql --bench typing
//! ```
//!
//! Modes: `typing`, `typing-long-line`, `typing-with-results`, `scroll`, `open`, `ime-bounds`.
//! Keep the window frontmost and do not touch the mouse or keyboard while it runs.

use std::process::ExitCode;

use hacivat_app::bench::b3;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match b3::parse_args(&args) {
        Ok(args) => b3::run(args),
        Err(e) => {
            eprintln!("b3: {e}\n{}", b3::USAGE);
            ExitCode::from(hacivat_app::bench::EXIT_USAGE)
        }
    }
}
