//! `cargo xtask bench-report <files...> --milestone m0`: appends one markdown row per
//! recorder dump to `docs/benchmarks/<milestone>.md`.

// Shares statistics and thresholds with the recorders (the file has no GPUI dependency).
#[path = "../../crates/ui/hacivat-ui-core/src/frames.rs"]
#[allow(dead_code)]
mod frames;

use frames::{FrameStats, LatencyStats, Verdict, latency_stats_of, stats_of, verdict};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

const HEADER: &str = "# Benchmarks\n\n| label | OS | host | build | frames | mean fps | p95 ms | p99 ms | max gap ms | gaps > 100 ms | verdict |\n|---|---|---|---|---|---|---|---|---|---|---|\n";

pub fn run() -> ExitCode {
    match run_inner(std::env::args().skip(2).collect()) {
        Ok(path) => {
            println!("updated {}", path.display());
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("bench-report: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run_inner(args: Vec<String>) -> Result<PathBuf, String> {
    let mut files = Vec::new();
    let mut milestone = None;
    let mut it = args.into_iter();
    while let Some(a) = it.next() {
        if a == "--milestone" {
            milestone = Some(it.next().ok_or("--milestone needs a value")?);
        } else {
            files.push(PathBuf::from(a));
        }
    }
    let milestone = milestone.ok_or("missing --milestone <name>")?;
    if files.is_empty() {
        return Err("no input files".into());
    }
    let build = build_hash();
    let mut rows = String::new();
    for f in &files {
        let text = std::fs::read_to_string(f).map_err(|e| format!("{}: {e}", f.display()))?;
        rows.push_str(&row_for(&text, &build).map_err(|e| format!("{}: {e}", f.display()))?);
    }
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let out = root.join("docs/benchmarks").join(format!("{milestone}.md"));
    let mut doc = std::fs::read_to_string(&out).unwrap_or_else(|_| HEADER.to_string());
    if !doc.ends_with('\n') {
        doc.push('\n');
    }
    doc.push_str(&rows);
    std::fs::create_dir_all(out.parent().unwrap()).map_err(|e| e.to_string())?;
    std::fs::write(&out, doc).map_err(|e| e.to_string())?;
    Ok(out)
}

fn build_hash() -> String {
    std::process::Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "nogit".into())
}

fn verdict_text(v: Verdict) -> &'static str {
    match v {
        Verdict::Pass => "PASS",
        Verdict::Fail => "FAIL",
        Verdict::NoThreshold => "n/a",
    }
}

/// Renders one markdown table row (with trailing newline) from a recorder dump.
fn row_for(json: &str, build: &str) -> Result<String, String> {
    let v: serde_json::Value = serde_json::from_str(json).map_err(|e| e.to_string())?;
    let s = |k: &str| v[k].as_str().ok_or(format!("missing \"{k}\"")).map(str::to_string);
    let (label, os, host, kind) = (s("label")?, s("os")?, s("host")?, s("kind")?);
    let values: Vec<f64> = v["values_ms"]
        .as_array()
        .ok_or("missing \"values_ms\"")?
        .iter()
        .map(|x| x.as_f64().ok_or("non-numeric value"))
        .collect::<Result<_, _>>()?;
    let mut row = String::new();
    match kind.as_str() {
        "frames" => {
            let st: FrameStats = stats_of(&values);
            let ver = verdict_text(verdict(&label, &os, &st));
            let _ = writeln!(
                row,
                "| {label} | {os} | {host} | {build} | {} | {:.1} | {:.1} | {:.1} | {:.1} | {} | {ver} |",
                st.frames, st.mean_fps, st.p95_ms, st.p99_ms, st.max_gap_ms, st.gaps_over_100ms
            );
        }
        "latency" => {
            // Latency dumps have no fps/p99/gap columns; p95 and max fill their slots.
            let st: LatencyStats = latency_stats_of(&values);
            let ver = verdict_text(verdict(&label, &os, &st));
            let _ = writeln!(
                row,
                "| {label} | {os} | {host} | {build} | {} | - | {:.1} | - | {:.1} | - | {ver} |",
                st.samples, st.p95_ms, st.max_ms
            );
        }
        other => return Err(format!("unknown kind \"{other}\"")),
    }
    Ok(row)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_row_passes_b1_on_smooth_macos() {
        let vals = vec!["16"; 600].join(",");
        let json = format!(r#"{{"label":"b1-fling","os":"macos","host":"mbp","kind":"frames","values_ms":[{vals}]}}"#);
        let row = row_for(&json, "abc1234").unwrap();
        assert!(row.starts_with("| b1-fling | macos | mbp | abc1234 | 600 | 62.5 |"), "{row}");
        assert!(row.trim_end().ends_with("| 0 | PASS |"), "{row}");
    }

    #[test]
    fn frames_row_fails_b1_on_long_gap() {
        let json = r#"{"label":"b1-fling","os":"linux","host":"h","kind":"frames","values_ms":[16,16,150,16]}"#;
        assert!(row_for(json, "x").unwrap().trim_end().ends_with("| 1 | FAIL |"));
    }

    #[test]
    fn latency_row_uses_b3_threshold() {
        let json = r#"{"label":"b3-completion","os":"macos","host":"h","kind":"latency","values_ms":[10,20,60]}"#;
        let row = row_for(json, "x").unwrap();
        assert!(row.contains("| 3 | - | 60.0 | - | 60.0 | - | FAIL |"), "{row}");
    }

    #[test]
    fn rejects_unknown_kind() {
        let json = r#"{"label":"a","os":"b","host":"c","kind":"zzz","values_ms":[]}"#;
        assert!(row_for(json, "x").is_err());
    }
}
