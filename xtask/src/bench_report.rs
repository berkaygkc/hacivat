//! `cargo xtask bench-report <files...> --milestone m0`: appends one markdown row per
//! recorder dump to `docs/benchmarks/<milestone>.md`.

// Shares statistics and thresholds with the recorders (`hacivat-bench` has no GPUI dependency).
use hacivat_bench::{
    FrameStats, LatencyStats, RunInfo, Verdict, latency_stats_of, stats_of, verdict,
};
use std::fmt::Write as _;
use std::path::PathBuf;
use std::process::ExitCode;

const HEADER: &str = "# Benchmarks\n\n| label | OS | host | build | display | frames | mean fps | p95 ms | p99 ms | max gap ms | gaps > 100 ms | verdict |\n|---|---|---|---|---|---|---|---|---|---|---|---|\n";

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
    let milestone = parse_milestone(&milestone.ok_or("missing --milestone <name>")?)?;
    if files.is_empty() {
        return Err("no input files".into());
    }
    let root = crate::workspace_root();
    let mut rows = String::new();
    for f in &files {
        let text = std::fs::read_to_string(f).map_err(|e| format!("{}: {e}", f.display()))?;
        let source = f
            .file_name()
            .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
        rows.push_str(&row_for(&text, &source).map_err(|e| format!("{}: {e}", f.display()))?);
    }
    let out = root.join("docs/benchmarks").join(format!("{milestone}.md"));
    let doc = std::fs::read_to_string(&out).unwrap_or_else(|_| HEADER.to_string());
    let doc = insert_rows(&doc, &rows);
    std::fs::create_dir_all(out.parent().unwrap()).map_err(|e| e.to_string())?;
    std::fs::write(&out, doc).map_err(|e| e.to_string())?;
    Ok(out)
}

/// A milestone name (`m0`, `m1`, ...): it becomes a file name, so no path syntax is accepted.
pub fn parse_milestone(s: &str) -> Result<String, String> {
    let digits = s.strip_prefix('m').unwrap_or("");
    if !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()) {
        Ok(s.to_string())
    } else {
        Err(format!(
            "bad milestone \"{s}\" (expected m<number>, e.g. m1)"
        ))
    }
}

/// Makes `s` safe inside one markdown table cell.
pub fn escape_cell(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('|', "\\|")
        .replace(['\n', '\r'], " ")
}

/// The hidden trailing cell that identifies the dump a row came from.
fn source_marker(source_file: &str) -> String {
    // `--` cannot appear inside an HTML comment body.
    format!(
        "<!-- src:{} -->",
        escape_cell(source_file).replace("--", "- -")
    )
}

/// Adds `rows` right after the last table row, so notes written under the table stay below it.
/// A row whose source marker is already in `doc` replaces the old row in place instead.
pub fn insert_rows(doc: &str, rows: &str) -> String {
    let mut lines: Vec<String> = doc.lines().map(str::to_string).collect();
    for row in rows.lines() {
        let existing = row
            .find("<!-- src:")
            .map(|i| &row[i..])
            .and_then(|m| m.find(" -->").map(|j| &m[..j + 4]))
            .and_then(|marker| {
                lines
                    .iter()
                    .position(|l| l.starts_with('|') && l.contains(marker))
            });
        match existing {
            Some(i) => lines[i] = row.to_string(),
            None => {
                let at = lines
                    .iter()
                    .rposition(|l| l.starts_with('|'))
                    .map_or(lines.len(), |i| i + 1);
                lines.insert(at, row.to_string());
            }
        }
    }
    let mut out = lines.join("\n");
    out.push('\n');
    out
}

fn verdict_text(v: Verdict) -> &'static str {
    match v {
        Verdict::Pass => "PASS",
        Verdict::Fail => "FAIL",
        Verdict::NoThreshold => "n/a",
    }
}

/// A number, or `None` for the string `"unknown"`; anything else is an error.
fn num_or_unknown(v: &serde_json::Value, k: &str) -> Result<Option<f64>, String> {
    match &v[k] {
        serde_json::Value::Number(n) => Ok(n.as_f64()),
        serde_json::Value::String(s) if s == "unknown" => Ok(None),
        _ => Err(format!("missing \"{k}\" (a number or \"unknown\")")),
    }
}

/// The run metadata the recorder wrote at bench time. Dumps without it cannot be graded.
fn run_info(v: &serde_json::Value) -> Result<RunInfo, String> {
    let count = |k: &str| {
        v[k].as_u64()
            .map(|n| n as usize)
            .ok_or(format!("missing \"{k}\""))
    };
    let painted_frames = match &v["painted_frames"] {
        serde_json::Value::Null => None,
        p => Some(p.as_u64().ok_or("bad \"painted_frames\"")? as usize),
    };
    Ok(RunInfo {
        expected_samples: count("expected_samples")?,
        inactive_keys: count("inactive_keys")?,
        painted_frames,
        aborted: v["aborted"].as_bool().ok_or("missing \"aborted\"")?,
        build: v["build"].as_str().ok_or("missing \"build\"")?.to_string(),
        display_hz: num_or_unknown(v, "display_hz")?,
        display_scale: num_or_unknown(v, "display_scale")?.map(|s| s as f32),
    })
}

/// `240 Hz, 2x`; `unknown` when neither is known.
fn display_text(run: &RunInfo) -> String {
    match (run.display_hz, run.display_scale) {
        (None, None) => "unknown".into(),
        (hz, scale) => format!(
            "{} Hz, {}x",
            hz.map_or("unknown".into(), |h| format!("{}", h.round())),
            scale.map_or("unknown".into(), |s| format!("{s}"))
        ),
    }
}

/// Renders one markdown table row (with trailing newline) from a recorder dump. The build and
/// display columns come from the dump (stamped at bench time), not from the current checkout.
pub fn row_for(json: &str, source_file: &str) -> Result<String, String> {
    let v: serde_json::Value = serde_json::from_str(json).map_err(|e| e.to_string())?;
    let s = |k: &str| {
        v[k].as_str()
            .ok_or(format!("missing \"{k}\""))
            .map(str::to_string)
    };
    let (label, os, host, kind) = (s("label")?, s("os")?, s("host")?, s("kind")?);
    let (label, os, host) = (escape_cell(&label), escape_cell(&os), escape_cell(&host));
    let src = source_marker(source_file);
    let run = run_info(&v)?;
    let (build, display) = (escape_cell(&run.build), display_text(&run));
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
            let ver = verdict_text(verdict(&label, &os, &st, &run));
            let _ = writeln!(
                row,
                "| {label} | {os} | {host} | {build} | {display} | {} | {:.1} | {:.1} | {:.1} | {:.1} | {} | {ver} | {src} |",
                st.frames, st.mean_fps, st.p95_ms, st.p99_ms, st.max_gap_ms, st.gaps_over_100ms
            );
        }
        "latency" => {
            // Latency dumps have no fps/p99/gap columns; p95 and max fill their slots.
            let st: LatencyStats = latency_stats_of(&values);
            let ver = verdict_text(verdict(&label, &os, &st, &run));
            let _ = writeln!(
                row,
                "| {label} | {os} | {host} | {build} | {display} | {} | - | {:.1} | - | {:.1} | - | {ver} | {src} |",
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

    /// A dump with complete-run metadata; `extra` overrides or adds JSON fields.
    fn dump(label: &str, os: &str, kind: &str, values: &str, extra: &str) -> String {
        let n = values.split(',').filter(|v| !v.is_empty()).count();
        let base = format!(
            r#""label":"{label}","os":"{os}","host":"mbp","kind":"{kind}","build":"abc1234","display_hz":240,"display_scale":2,"expected_samples":{n},"inactive_keys":0,"painted_frames":{n},"aborted":false"#
        );
        let mut v: serde_json::Value = serde_json::from_str(&format!("{{{base}}}")).unwrap();
        if !extra.is_empty() {
            let e: serde_json::Value = serde_json::from_str(&format!("{{{extra}}}")).unwrap();
            for (k, x) in e.as_object().unwrap() {
                v[k] = x.clone();
            }
        }
        v["values_ms"] = serde_json::from_str(&format!("[{values}]")).unwrap();
        v.to_string()
    }

    #[test]
    fn frames_row_passes_b1_on_smooth_macos() {
        let vals = vec!["16"; 600].join(",");
        let row = row_for(&dump("b1-fling", "macos", "frames", &vals, ""), "t.json").unwrap();
        assert!(
            row.starts_with("| b1-fling | macos | mbp | abc1234 | 240 Hz, 2x | 600 | 62.5 |"),
            "{row}"
        );
        assert!(body(&row).ends_with("| 0 | PASS |"), "{row}");
    }

    #[test]
    fn build_and_display_come_from_the_dump_not_head() {
        let vals = vec!["16"; 600].join(",");
        let extra = r#""build":"feedbee-dirty","display_hz":"unknown","display_scale":"unknown""#;
        let row = row_for(&dump("b1-fling", "macos", "frames", &vals, extra), "t.json").unwrap();
        assert!(row.contains("| feedbee-dirty | unknown |"), "{row}");
    }

    #[test]
    fn frames_row_fails_b1_on_long_gap() {
        let mut vals = vec!["16"; 600];
        vals[3] = "150";
        let row = row_for(
            &dump("b1-fling", "linux", "frames", &vals.join(","), ""),
            "t.json",
        )
        .unwrap();
        assert!(body(&row).ends_with("| 1 | FAIL |"), "{row}");
    }

    #[test]
    fn latency_row_uses_b3_threshold() {
        let row = row_for(
            &dump("b3-completion", "macos", "latency", "10,20,60", ""),
            "t.json",
        )
        .unwrap();
        assert!(
            row.contains("| 3 | - | 60.0 | - | 60.0 | - | FAIL |"),
            "{row}"
        );
    }

    #[test]
    fn partial_inactive_or_aborted_runs_fail() {
        let ok = row_for(
            &dump("b3-typing", "macos", "latency", "5,5,5", ""),
            "t.json",
        )
        .unwrap();
        assert!(body(&ok).ends_with("| PASS |"), "{ok}");
        for extra in [
            r#""expected_samples":2000"#,
            r#""inactive_keys":1"#,
            r#""aborted":true"#,
        ] {
            let row = row_for(
                &dump("b3-typing", "macos", "latency", "5,5,5", extra),
                "t.json",
            )
            .unwrap();
            assert!(body(&row).ends_with("| FAIL |"), "{extra}: {row}");
        }
        let vals = vec!["16"; 600].join(",");
        let row = row_for(
            &dump(
                "b1-fling",
                "macos",
                "frames",
                &vals,
                r#""painted_frames":10"#,
            ),
            "t.json",
        )
        .unwrap();
        assert!(body(&row).ends_with("| FAIL |"), "{row}");
    }

    #[test]
    fn empty_dump_fails() {
        let row = row_for(&dump("b3-typing", "macos", "latency", "", ""), "t.json").unwrap();
        assert!(body(&row).ends_with("| FAIL |"), "{row}");
    }

    #[test]
    fn dumps_without_run_metadata_are_rejected() {
        for field in [
            "build",
            "expected_samples",
            "inactive_keys",
            "aborted",
            "display_hz",
        ] {
            let mut v: serde_json::Value =
                serde_json::from_str(&dump("b3-typing", "macos", "latency", "5", "")).unwrap();
            v.as_object_mut().unwrap().remove(field);
            assert!(row_for(&v.to_string(), "t.json").is_err(), "{field}");
        }
    }

    #[test]
    fn rows_go_after_the_table_not_after_trailing_notes() {
        let doc = "# B\n\n| a |\n|---|\n| r1 |\n\n## Notes\n- n\n";
        assert_eq!(
            insert_rows(doc, "| r2 |\n"),
            "# B\n\n| a |\n|---|\n| r1 |\n| r2 |\n\n## Notes\n- n\n"
        );
        assert_eq!(
            insert_rows("| a |\n|---|", "| r |\n"),
            "| a |\n|---|\n| r |\n"
        );
    }

    #[test]
    fn rejects_unknown_kind() {
        assert!(row_for(&dump("a", "b", "zzz", "", ""), "t.json").is_err());
    }

    /// The row without its trailing hidden source cell for `t.json`.
    fn body(row: &str) -> &str {
        row.trim_end()
            .strip_suffix(" <!-- src:t.json --> |")
            .unwrap_or_else(|| panic!("no source cell: {row}"))
    }

    fn b1_fixture_json() -> String {
        dump(
            "b1-fling",
            "macos",
            "frames",
            &vec!["16"; 600].join(","),
            "",
        )
    }

    const DOC_FIXTURE: &str = "# Benchmarks\n\n| label |\n|---|\n| old |\n\n## Notes\n- n\n";

    #[test]
    fn pipe_in_label_is_escaped() {
        assert_eq!(escape_cell("b1|x"), "b1\\|x");
    }

    #[test]
    fn milestone_rejects_paths() {
        assert!(parse_milestone("../m0").is_err());
        assert!(parse_milestone("m1/x").is_err());
        assert!(parse_milestone("M1").is_err());
        assert!(parse_milestone("m").is_err());
        assert_eq!(parse_milestone("m1").unwrap(), "m1");
    }

    #[test]
    fn rerun_does_not_duplicate_rows() {
        let r = row_for(&b1_fixture_json(), "b1-fling-macos-1.json").unwrap();
        let doc = insert_rows(DOC_FIXTURE, &r);
        assert_eq!(insert_rows(&doc, &r), doc);
        assert_eq!(doc.matches("src:b1-fling-macos-1.json").count(), 1);
    }

    #[test]
    fn rerun_replaces_the_old_row_in_place() {
        let r1 = row_for(&b1_fixture_json(), "a.json").unwrap();
        let other = row_for(&b1_fixture_json(), "b.json").unwrap();
        let doc = insert_rows(&insert_rows(DOC_FIXTURE, &r1), &other);
        let newer = row_for(
            &dump(
                "b1-fling",
                "linux",
                "frames",
                &vec!["16"; 600].join(","),
                "",
            ),
            "a.json",
        )
        .unwrap();
        let doc = insert_rows(&doc, &newer);
        assert_eq!(doc.matches("src:a.json").count(), 1);
        assert_eq!(doc.matches("src:b.json").count(), 1);
        let line = doc.lines().find(|l| l.contains("src:a.json")).unwrap();
        assert!(line.starts_with("| b1-fling | linux |"), "{line}");
    }

    #[test]
    fn row_ends_with_hidden_source_cell() {
        let r = row_for(&b1_fixture_json(), "x.json").unwrap();
        assert!(r.trim_end().ends_with("| <!-- src:x.json --> |"), "{r}");
    }

    #[test]
    fn row_cells_escape_pipes() {
        let j = dump("b1|x", "macos", "frames", &vec!["16"; 600].join(","), "");
        let r = row_for(&j, "x.json").unwrap();
        assert!(r.starts_with("| b1\\|x |"), "{r}");
    }
}
