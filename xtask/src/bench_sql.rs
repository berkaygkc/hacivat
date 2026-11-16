//! `bench-sql`: the SQL text and classifier benchmarks (SQL-T13; Task 20).
//!
//! `cargo xtask bench-sql [--out <file>]` runs `sql-safety`'s `bench_sql` test in release as a
//! subprocess (`HACIVAT_BENCH_SQL=1`; xtask takes no SQL crate, T1a I3), collects every
//! `bench-sql: {json}` line it prints (lexing, the worst relex, typing relex and resplit,
//! `resolve`, `classify` on the gen-sql 1 MB and 50k-line fixtures, the per-10 KB classifier,
//! the PG oracle's one call against three) and writes them as one JSON report, by default
//! `target/bench-sql.json`. The figures are a relative signal (L2): CI keeps the report as an
//! artifact and asserts nothing about a number.

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};

/// The prefix of a result line.
const PREFIX: &str = "bench-sql: ";

/// The report's format version.
const SCHEMA: u32 = 1;

/// `cargo xtask bench-sql [--out <file>]`.
pub fn run() -> ExitCode {
    match run_inner() {
        Ok(path) => {
            println!("bench-sql: wrote {}", path.display());
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("xtask bench-sql: {e:#}");
            ExitCode::FAILURE
        }
    }
}

fn run_inner() -> Result<PathBuf> {
    let root = crate::workspace_root();
    let mut out = root.join("target").join("bench-sql.json");
    let mut args = std::env::args().skip(2);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--out" => out = PathBuf::from(args.next().context("--out needs a path")?),
            other => bail!("unknown argument {other:?} (usage: bench-sql [--out <file>])"),
        }
    }
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let output = Command::new(cargo)
        .current_dir(&root)
        .args([
            "test",
            "--release",
            "-p",
            "sql-safety",
            "--test",
            "bench_sql",
            "--",
            "--ignored",
            "--exact",
            "bench_sql",
            "--nocapture",
        ])
        .env("HACIVAT_BENCH_SQL", "1")
        .output()
        .context("running the bench_sql test")?;
    if !output.status.success() {
        eprintln!("{}", String::from_utf8_lossy(&output.stderr));
        bail!("the bench_sql test failed ({})", output.status);
    }
    let results = collect(&String::from_utf8_lossy(&output.stdout))?;
    let commit = Command::new("git")
        .current_dir(&root)
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string());
    write_report(&out, &report(results, commit.as_deref()))?;
    Ok(out)
}

/// Every result line of the test's output, parsed; an error for a line that does not parse or
/// for an output with none.
pub(crate) fn collect(stdout: &str) -> Result<Vec<Value>> {
    let mut out = Vec::new();
    for line in stdout.lines() {
        let Some(body) = line.trim_start().strip_prefix(PREFIX) else {
            continue;
        };
        let v: Value =
            serde_json::from_str(body).with_context(|| format!("a result line: {line:?}"))?;
        if v.get("name").and_then(Value::as_str).is_none() {
            bail!("a result without a name: {line:?}");
        }
        out.push(v);
    }
    if out.is_empty() {
        bail!("the bench_sql test printed no result (was HACIVAT_BENCH_SQL=1 passed?)");
    }
    Ok(out)
}

/// The report: format version, tool, commit when known, and the results in order.
pub(crate) fn report(results: Vec<Value>, commit: Option<&str>) -> Value {
    json!({
        "schema": SCHEMA,
        "tool": "bench-sql",
        "commit": commit,
        "note": "relative signal only (L2); no figure is a budget",
        "results": results,
    })
}

pub(crate) fn write_report(path: &Path, report: &Value) -> Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    let text = serde_json::to_string_pretty(report)? + "\n";
    std::fs::write(path, text).with_context(|| format!("writing {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const OUTPUT: &str = "\
running 1 test
bench-sql: {\"name\":\"lex\",\"fixture\":\"gen-sql-1mb\",\"bytes\":1047939,\"runs\":10,\"best_ms\":4.8855,\"p50_ms\":5.1512,\"p95_ms\":5.9391}
some other line
bench-sql: {\"name\":\"classify-10kb-pg\",\"fixture\":\"10kb-unit\",\"bytes\":10163,\"runs\":200,\"best_ms\":1.65,\"p50_ms\":1.82,\"p95_ms\":2.59}
test bench_sql ... ok
";

    #[test]
    fn bench_sql_writes_json() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("bench-sql.json");
        let results = collect(OUTPUT).unwrap();
        assert_eq!(results.len(), 2);
        write_report(&path, &report(results, Some("abc123"))).unwrap();
        let back: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(back["schema"], 1);
        assert_eq!(back["tool"], "bench-sql");
        assert_eq!(back["commit"], "abc123");
        let names: Vec<&str> = back["results"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, ["lex", "classify-10kb-pg"]);
        assert_eq!(back["results"][1]["best_ms"], 1.65);
        assert_eq!(back["results"][0]["bytes"], 1_047_939);
    }

    #[test]
    fn bench_sql_refuses_bad_or_missing_results() {
        assert!(collect("running 1 test\ntest bench_sql ... ok\n").is_err());
        assert!(collect("bench-sql: {not json}\n").is_err());
        assert!(collect("bench-sql: {\"fixture\":\"x\"}\n").is_err());
        let r = report(collect(OUTPUT).unwrap(), None);
        assert!(r["commit"].is_null());
    }
}
