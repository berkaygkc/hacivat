//! B3 in the real app (Task 36; feature `bench`): typing, scrolling, opening and the IME's
//! bounds question in a console's editor over the B3 fixtures. See `hacivat_app::bench::b3`.
//!
//! ```text
//! cargo xtask gen-sql --lines 50000 --out target/fixtures/b3-50k-t0.sql                    # T0
//! cargo xtask gen-sql --lines 50000 --long-line 100000 --out target/fixtures/b3-50k.sql    # T1
//! cargo xtask gen-sql --lines 520000 --out target/fixtures/b3-19mb.sql                     # 19 MiB
//! cargo run --release -p hacivat-app --features bench --example b3 -- \
//!     --file target/fixtures/b3-50k-t0.sql --bench typing
//! ```
//!
//! Modes on the T0 fixture: `typing`, `typing-with-results`, `typing-multi-caret`,
//! `typing-with-folds` (M3c), `open`, `scroll`; on the T1 fixture: `typing-long-line`,
//! `ime-bounds`; on the 19 MiB file: `typing-19mb` (M3c).
//!
//! Graded only at ≥ 118 Hz (any scale) with the `tr-TR` format applied:
//! - a display under 118 Hz (or reporting none) is INVALID (PENDING-PO, no JSON), or ungraded
//!   with `--any-display` (`b3-offpanel-…`);
//! - another format runs ungraded (`b3-offlocale-…`, JSON written);
//! - a window that cannot become frontmost (display asleep, session locked), or keystrokes to an
//!   inactive window mid-run, is INVALID (PENDING-PO); `--any-display` does not change that;
//! - `--allow-inactive` (the CI harness check) goes on, always ungraded.
//!
//! Run under `caffeinate -d` so the display does not sleep. The last line is `RESULT: PASS|FAIL|INVALID|n/a`; exit 0 is PASS or ungraded, 1 FAIL or
//! aborted, 3 INVALID. Keep the window frontmost and do not touch the mouse or keyboard while it
//! runs.

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
