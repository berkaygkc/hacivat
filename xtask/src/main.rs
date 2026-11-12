mod bench_report;
mod check_deps;
mod gen_sql;
mod l10n;
mod shot_diff;
mod tokens;
mod update_key;
mod vf;

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
        other => {
            eprintln!(
                "unknown xtask command: {other:?}\navailable: bench-report, check-deps, check-update-key, gen-sql, l10n, shot-diff, tokens, vf"
            );
            ExitCode::from(2)
        }
    }
}
