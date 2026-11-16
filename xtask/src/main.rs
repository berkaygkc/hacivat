mod bench_report;
mod check_deps;
mod gen_sql;
mod l10n;
mod shot_diff;
mod tokens;
mod update_key;
mod vf;
// ## m2a-t11-docker
mod docker;
// ## m3a-t03-sql-tables
mod gen_sql_tables;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

/// The cargo workspace root: the parent of this crate's manifest directory. Every xtask test
/// that reads repository files resolves paths from here (F20).
pub fn workspace_root() -> PathBuf {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    manifest.parent().unwrap_or(manifest).to_path_buf()
}

fn main() -> ExitCode {
    match std::env::args().nth(1).as_deref() {
        Some("bench-report") => bench_report::run(),
        Some("check-deps") => check_deps::run(),
        Some("gen-sql") => gen_sql::run(),
        Some("l10n") => l10n::run(),
        Some("tokens") => tokens::run(),
        Some("check-update-key") => update_key::run(),
        Some("vf") => vf::run(),
        Some("shot-diff") => shot_diff::run(),
        // ## m2a-t11-docker
        Some("docker") => docker::run(),
        // ## m3a-t03-sql-tables
        Some("gen-sql-tables") => gen_sql_tables::run(),
        other => {
            eprintln!(
                "unknown xtask command: {other:?}\navailable: bench-report, check-deps, check-update-key, docker, gen-sql, l10n, shot-diff, tokens, vf"
            );
            // ## m3a-t03-sql-tables
            eprintln!("also available: gen-sql-tables");
            ExitCode::from(2)
        }
    }
}
