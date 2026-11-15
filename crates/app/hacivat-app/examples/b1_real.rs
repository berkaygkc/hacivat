//! B1-real (Task 36; feature `bench`): the M0 fling over a 1M-row × 50-column result from
//! PostgreSQL in the real app's results grid, in the `tr-TR` format locale. See
//! `hacivat_app::bench::b1_real`.
//!
//! ```text
//! HACIVAT_DOCKER_SET=pr cargo run --release -p hacivat-app --features bench --example b1_real
//! HACIVAT_DOCKER_SET=pr cargo run --release -p hacivat-app --features bench --example b1_real -- --size 2560x1440
//! HACIVAT_DOCKER_SET=pr cargo run --release -p hacivat-app --features bench --example b1_real -- --mode streaming
//! cargo run --release -p hacivat-app --features bench --example b1_real -- --engine sqlite
//! ```
//!
//! The PostgreSQL server is the Docker matrix's `pg17` (`hacivat_testkit::require_target`, which
//! answers only when `HACIVAT_DOCKER_SET` names a set holding it: `pr` or `full`). Without it
//! the bench exits 2 and names the variable. Keep the window frontmost and do not touch the
//! mouse or keyboard while it runs.

use std::process::ExitCode;

use hacivat_app::bench::EXIT_USAGE;
use hacivat_app::bench::b1_real::{self, Engine, ServerTarget};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let args = match b1_real::parse_args(&args) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("b1_real: {e}\n{}", b1_real::USAGE);
            return ExitCode::from(EXIT_USAGE);
        }
    };
    let target = match args.engine {
        Engine::Sqlite => None,
        Engine::Pg => match hacivat_testkit::require_target("pg17") {
            Some(t) => Some(ServerTarget {
                host: t.host,
                port: t.port,
                user: t.user,
                password: t.password.into(),
                database: t.database,
            }),
            None => {
                eprintln!(
                    "b1_real: the pg17 target is not available; set HACIVAT_DOCKER_SET=pr (the Docker matrix's pr set, with pg17 up)\n{}",
                    b1_real::USAGE
                );
                return ExitCode::from(EXIT_USAGE);
            }
        },
    };
    b1_real::run(args, target)
}
