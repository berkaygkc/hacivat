//! Frame-interval and input-latency recording plus pass/fail verdicts.
//!
//! This crate is deliberately free of GPUI types: `xtask bench-report` depends on it, so the
//! report uses exactly the same statistics and thresholds as the recorders, which reach it as
//! `hacivat_ui_core::frames` (feature `bench`).

use std::io;
use std::path::{Path, PathBuf};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, PartialEq)]
pub struct FrameStats {
    /// Number of frame intervals (deltas between consecutive frames).
    pub frames: usize,
    pub mean_fps: f64,
    pub p50_ms: f64,
    pub p95_ms: f64,
    pub p99_ms: f64,
    pub max_gap_ms: f64,
    pub gaps_over_100ms: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LatencyStats {
    pub samples: usize,
    pub p50_ms: f64,
    pub p95_ms: f64,
    pub max_ms: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Pass,
    Fail,
    /// No threshold is defined for this label.
    NoThreshold,
}

/// Nearest-rank percentile of an ascending-sorted slice.
fn nearest_rank(sorted: &[f64], pct: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let rank = ((pct / 100.0) * sorted.len() as f64).ceil() as usize;
    sorted[rank.clamp(1, sorted.len()) - 1]
}

fn sorted(values: &[f64]) -> Vec<f64> {
    let mut v = values.to_vec();
    v.sort_by(|a, b| a.total_cmp(b));
    v
}

/// Statistics over frame intervals in milliseconds.
pub fn stats_of(intervals_ms: &[f64]) -> FrameStats {
    if intervals_ms.is_empty() {
        return FrameStats {
            frames: 0,
            mean_fps: 0.0,
            p50_ms: 0.0,
            p95_ms: 0.0,
            p99_ms: 0.0,
            max_gap_ms: 0.0,
            gaps_over_100ms: 0,
        };
    }
    let s = sorted(intervals_ms);
    let mean_ms = intervals_ms.iter().sum::<f64>() / intervals_ms.len() as f64;
    FrameStats {
        frames: intervals_ms.len(),
        mean_fps: if mean_ms > 0.0 { 1000.0 / mean_ms } else { 0.0 },
        p50_ms: nearest_rank(&s, 50.0),
        p95_ms: nearest_rank(&s, 95.0),
        p99_ms: nearest_rank(&s, 99.0),
        max_gap_ms: *s.last().unwrap(),
        gaps_over_100ms: intervals_ms.iter().filter(|&&g| g > 100.0).count(),
    }
}

/// Statistics over latency samples in milliseconds.
pub fn latency_stats_of(samples_ms: &[f64]) -> LatencyStats {
    let s = sorted(samples_ms);
    LatencyStats {
        samples: s.len(),
        p50_ms: nearest_rank(&s, 50.0),
        p95_ms: nearest_rank(&s, 95.0),
        max_ms: s.last().copied().unwrap_or(0.0),
    }
}

/// Provenance and completeness of one bench run, recorded at bench time and written into the
/// JSON dump, so the report grades what was measured instead of what HEAD is later.
#[derive(Debug, Clone, PartialEq)]
pub struct RunInfo {
    /// Samples a complete run produces (keystrokes measured, or B1's minimum frame count).
    pub expected_samples: usize,
    /// Synthetic inputs (keystrokes or fling steps) delivered while the window was inactive;
    /// GPUI throttles inactive windows, so any such input invalidates the run.
    pub inactive_keys: usize,
    /// Grid canvas paints during the run (B1); `None` where it does not apply.
    pub painted_frames: Option<usize>,
    /// The replay stopped before the script ended.
    pub aborted: bool,
    /// Short commit hash, with `-dirty` when the tree differed (see [`build_label`]).
    pub build: String,
    /// Refresh rate of the window's display; `None` when the platform does not report it.
    pub display_hz: Option<f64>,
    /// Backing scale factor of the window; `None` when unknown.
    pub display_scale: Option<f32>,
}

impl RunInfo {
    /// Zero expectations and unknown provenance; fill the fields that apply.
    pub fn unstamped() -> Self {
        Self {
            expected_samples: 0,
            inactive_keys: 0,
            painted_frames: None,
            aborted: false,
            build: "unknown".into(),
            display_hz: None,
            display_scale: None,
        }
    }
}

/// Minimum mean fps B1 must reach on `os` (spec §6 M0).
pub fn b1_min_fps(os: &str) -> f64 {
    if os == "linux" { 45.0 } else { 55.0 }
}

/// Frame intervals a complete B1 run of `duration_s` yields at the minimum passing fps. A run
/// that ends early (or records only part of the fling) falls below 95 % of this and fails.
pub fn b1_expected_frames(os: &str, duration_s: f64) -> usize {
    (duration_s * b1_min_fps(os)).floor() as usize
}

/// Fewest frame intervals a B1 run may have and still be graded.
pub const B1_MIN_FRAMES: usize = 600;
/// A run must deliver at least this share of its expected samples (and B1 must paint the grid
/// on at least this share of its frames).
pub const MIN_COMPLETENESS: f64 = 0.95;

/// Something a verdict can be computed for.
pub trait Judge {
    /// Verdict from the statistics' thresholds alone.
    fn threshold(&self, label: &str, os: &str) -> Verdict;
    /// Number of samples (frame intervals or latency samples) the statistics cover.
    fn samples(&self) -> usize;
    /// Label-specific completeness rules beyond the shared ones in [`verdict`].
    fn complete(&self, _label: &str, _run: &RunInfo) -> bool {
        true
    }
}

impl From<bool> for Verdict {
    fn from(ok: bool) -> Self {
        if ok { Verdict::Pass } else { Verdict::Fail }
    }
}

impl Judge for FrameStats {
    fn threshold(&self, label: &str, os: &str) -> Verdict {
        if label.starts_with("b1-fling") {
            return (self.mean_fps >= b1_min_fps(os) && self.gaps_over_100ms == 0).into();
        }
        Verdict::NoThreshold
    }

    fn samples(&self) -> usize {
        self.frames
    }

    fn complete(&self, label: &str, run: &RunInfo) -> bool {
        if !label.starts_with("b1-fling") {
            return true;
        }
        let painted = run.painted_frames.unwrap_or(0) as f64;
        self.frames >= B1_MIN_FRAMES && painted >= MIN_COMPLETENESS * self.frames as f64
    }
}

impl Judge for LatencyStats {
    fn threshold(&self, label: &str, _os: &str) -> Verdict {
        if label.starts_with("b3-completion") {
            (self.p95_ms <= 50.0).into()
        } else if label.starts_with("b3-typing") {
            (self.p95_ms <= 16.0).into()
        } else {
            Verdict::NoThreshold
        }
    }

    fn samples(&self) -> usize {
        self.samples
    }
}

/// The threshold verdict, downgraded to `Fail` unless the run is complete: not aborted, at
/// least one sample, at least 95 % of the expected samples, no input to an inactive window, and
/// the label's own rules (B1: ≥ 600 frames, grid painted on ≥ 95 % of them).
pub fn verdict(label: &str, os: &str, stats: &impl Judge, run: &RunInfo) -> Verdict {
    let v = stats.threshold(label, os);
    if v == Verdict::NoThreshold {
        return v;
    }
    let n = stats.samples();
    let complete = !run.aborted
        && n > 0
        && n as f64 >= MIN_COMPLETENESS * run.expected_samples as f64
        && run.inactive_keys == 0
        && stats.complete(label, run);
    if complete { v } else { Verdict::Fail }
}

/// `head`, plus `-dirty` if `git status --porcelain -z` output lists any change other than
/// untracked files under `target/` or `.superpowers/` (bench output and agent reports) or any
/// change under `docs/benchmarks/` (the report this run feeds).
///
/// Entries are NUL-terminated `XY path`. A rename or copy (`R`/`C` in either status column) is
/// followed by a second NUL-terminated path with no status prefix; the entry is clean only if
/// both paths are ignorable.
pub fn build_label(head: &str, porcelain_z: &str) -> String {
    let mut entries = porcelain_z.split('\0');
    let mut dirty = false;
    while let Some(entry) = entries.next() {
        if entry.is_empty() {
            continue;
        }
        let status = entry.get(..2).unwrap_or(entry);
        let path = entry.get(3..).unwrap_or("");
        let mut ignorable = (status == "??"
            && (path.starts_with("target/") || path.starts_with(".superpowers/")))
            || path.starts_with("docs/benchmarks/");
        if status.contains(['R', 'C']) {
            // The other path of a rename or copy is a change too: clean only if both are.
            let other = entries.next().unwrap_or("");
            ignorable &= other.starts_with("docs/benchmarks/");
        }
        dirty |= !ignorable;
    }
    if dirty {
        format!("{head}-dirty")
    } else {
        head.to_string()
    }
}

/// [`build_label`] for the git checkout at `root`; `"nogit"` outside one.
pub fn build_stamp(root: &Path) -> String {
    let git = |args: &[&str]| {
        std::process::Command::new("git")
            .args(args)
            .current_dir(root)
            .output()
            .ok()
            .filter(|o| o.status.success())
            .and_then(|o| String::from_utf8(o.stdout).ok())
    };
    let head = git(&["rev-parse", "--short", "HEAD"])
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    match head {
        Some(head) => build_label(
            &head,
            &git(&["status", "--porcelain", "-z"]).unwrap_or_default(),
        ),
        None => "nogit".into(),
    }
}

/// Host label written into public benchmark rows. The machine's real hostname often contains the
/// owner's name, so it is never used; set `HACIVAT_BENCH_HOST` to label a run (e.g. `po-mac`).
fn host_name() -> String {
    std::env::var("HACIVAT_BENCH_HOST")
        .ok()
        .filter(|h| !h.trim().is_empty())
        .unwrap_or_else(|| "unlabelled".into())
}

fn json_str(s: &str) -> String {
    let mut o = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            c if (c as u32) < 0x20 => o.push_str(&format!("\\u{:04x}", c as u32)),
            c => o.push(c),
        }
    }
    o.push('"');
    o
}

/// Writes `<dir>/<label>-<os>-<unix_ts>.json`. `kind` is "frames" or "latency".
fn write_dump(
    dir: &Path,
    label: &str,
    os: &str,
    kind: &str,
    values_ms: &[f64],
    run: &RunInfo,
) -> io::Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let path = dir.join(format!("{label}-{os}-{ts}.json"));
    let values: Vec<String> = values_ms.iter().map(|v| format!("{v}")).collect();
    let num_or_unknown = |v: Option<f64>| match v {
        Some(v) if v.is_finite() => format!("{v}"),
        _ => json_str("unknown"),
    };
    let painted = run
        .painted_frames
        .map_or_else(|| "null".to_string(), |p| p.to_string());
    let body = format!(
        "{{\"label\":{},\"os\":{},\"host\":{},\"kind\":{},\"build\":{},\"display_hz\":{},\"display_scale\":{},\"expected_samples\":{},\"inactive_keys\":{},\"painted_frames\":{},\"aborted\":{},\"values_ms\":[{}]}}\n",
        json_str(label),
        json_str(os),
        json_str(&host_name()),
        json_str(kind),
        json_str(&run.build),
        num_or_unknown(run.display_hz),
        num_or_unknown(run.display_scale.map(f64::from)),
        run.expected_samples,
        run.inactive_keys,
        painted,
        run.aborted,
        values.join(",")
    );
    std::fs::write(&path, body)?;
    Ok(path)
}

fn ms_between(later: Instant, earlier: Instant) -> f64 {
    later.saturating_duration_since(earlier).as_secs_f64() * 1000.0
}

pub struct FrameRecorder {
    label: String,
    last: Option<Instant>,
    intervals_ms: Vec<f64>,
    recording: bool,
}

impl FrameRecorder {
    pub fn new(label: &str) -> Self {
        Self {
            label: label.to_string(),
            last: None,
            intervals_ms: Vec::new(),
            recording: true,
        }
    }

    /// Collects one frame. Called from a `window.on_next_frame` loop.
    pub fn on_frame(&mut self, now: Instant) {
        if let Some(prev) = self.last {
            self.intervals_ms.push(ms_between(now, prev));
        }
        self.last = Some(now);
    }

    pub fn is_recording(&self) -> bool {
        self.recording
    }

    /// Stops the re-arming loop in `record_frames`.
    pub fn stop(&mut self) {
        self.recording = false;
    }

    pub fn intervals_ms(&self) -> &[f64] {
        &self.intervals_ms
    }

    pub fn stats(&self) -> FrameStats {
        stats_of(&self.intervals_ms)
    }

    pub fn dump_json(&self, dir: &Path, os: &str, run: &RunInfo) -> io::Result<PathBuf> {
        write_dump(dir, &self.label, os, "frames", &self.intervals_ms, run)
    }
}

pub struct LatencyProbe {
    label: String,
    pending: Option<Instant>,
    samples_ms: Vec<f64>,
}

impl LatencyProbe {
    pub fn new(label: &str) -> Self {
        Self {
            label: label.to_string(),
            pending: None,
            samples_ms: Vec::new(),
        }
    }

    /// Marks an input event. If one is already pending the earliest mark is kept.
    pub fn mark_input(&mut self, at: Instant) {
        self.pending.get_or_insert(at);
    }

    /// If an input is pending, records `now - input_at` as one sample.
    pub fn on_frame(&mut self, now: Instant) {
        if let Some(at) = self.pending.take() {
            self.samples_ms.push(ms_between(now, at));
        }
    }

    pub fn stats(&self) -> LatencyStats {
        latency_stats_of(&self.samples_ms)
    }

    pub fn dump_json(&self, dir: &Path, os: &str, run: &RunInfo) -> io::Result<PathBuf> {
        write_dump(dir, &self.label, os, "latency", &self.samples_ms, run)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn ok() -> FrameStats {
        stats_of(&[16.0; 1_200])
    }

    /// A complete B1 run over `ok()`.
    fn b1_run() -> RunInfo {
        RunInfo {
            expected_samples: 1_100,
            painted_frames: Some(1_200),
            ..RunInfo::unstamped()
        }
    }

    fn lat(samples: usize) -> LatencyStats {
        LatencyStats {
            samples,
            p50_ms: 1.0,
            p95_ms: 5.0,
            max_ms: 9.0,
        }
    }

    fn lat_run(expected: usize) -> RunInfo {
        RunInfo {
            expected_samples: expected,
            ..RunInfo::unstamped()
        }
    }

    #[test]
    fn stats_from_uniform_16ms() {
        let s = stats_of(&[16.0; 600]);
        assert!((s.mean_fps - 62.5).abs() < 0.01);
        assert_eq!(s.gaps_over_100ms, 0);
    }

    #[test]
    fn detects_long_gap() {
        let mut v = vec![16.0; 100];
        v[50] = 140.0;
        let s = stats_of(&v);
        assert_eq!(s.gaps_over_100ms, 1);
        assert_eq!(s.max_gap_ms, 140.0);
    }

    #[test]
    fn p95_nearest_rank() {
        let v: Vec<f64> = (1..=100).map(|i| i as f64).collect();
        assert_eq!(stats_of(&v).p95_ms, 95.0);
    }

    #[test]
    fn recorder_uses_deltas_between_frames() {
        let t0 = Instant::now();
        let mut r = FrameRecorder::new("x");
        for i in 0..4 {
            r.on_frame(t0 + Duration::from_millis(16 * i));
        }
        assert_eq!(r.stats().frames, 3);
        assert!((r.stats().p50_ms - 16.0).abs() < 0.01);
    }

    #[test]
    fn latency_pairs_input_with_next_frame() {
        let t0 = Instant::now();
        let mut p = LatencyProbe::new("b3-typing");
        p.on_frame(t0); // no input pending: no sample
        p.mark_input(t0);
        p.on_frame(t0 + Duration::from_millis(5));
        p.on_frame(t0 + Duration::from_millis(20));
        let s = p.stats();
        assert_eq!(s.samples, 1);
        assert!((s.max_ms - 5.0).abs() < 0.01);
    }

    #[test]
    fn report_marks_b1_fail_below_55fps_on_macos() {
        let s = FrameStats {
            mean_fps: 54.9,
            ..ok()
        };
        assert_eq!(verdict("b1-fling", "macos", &s, &b1_run()), Verdict::Fail);
    }

    #[test]
    fn report_marks_b1_pass_45fps_on_linux() {
        let s = FrameStats {
            mean_fps: 45.0,
            ..ok()
        };
        assert_eq!(verdict("b1-fling", "linux", &s, &b1_run()), Verdict::Pass);
    }

    #[test]
    fn b1_fails_on_long_gap() {
        let s = FrameStats {
            gaps_over_100ms: 1,
            ..ok()
        };
        assert_eq!(verdict("b1-fling", "macos", &s, &b1_run()), Verdict::Fail);
    }

    #[test]
    fn b3_thresholds() {
        let l = |p95| LatencyStats {
            samples: 1,
            p50_ms: 1.0,
            p95_ms: p95,
            max_ms: p95,
        };
        let run = lat_run(1);
        assert_eq!(
            verdict("b3-typing-small", "macos", &l(16.0), &run),
            Verdict::Pass
        );
        assert_eq!(
            verdict("b3-typing-small", "macos", &l(16.1), &run),
            Verdict::Fail
        );
        assert_eq!(
            verdict("b3-completion", "macos", &l(50.0), &run),
            Verdict::Pass
        );
        assert_eq!(
            verdict("b3-completion", "macos", &l(50.1), &run),
            Verdict::Fail
        );
        assert_eq!(
            verdict("other", "macos", &l(1.0), &run),
            Verdict::NoThreshold
        );
    }

    #[test]
    fn dump_json_writes_raw_intervals() {
        let dir = std::env::temp_dir().join(format!("hacivat-frames-{}", std::process::id()));
        let t0 = Instant::now();
        let mut r = FrameRecorder::new("b1-fling");
        r.on_frame(t0);
        r.on_frame(t0 + Duration::from_millis(16));
        let run = RunInfo {
            expected_samples: 1_100,
            inactive_keys: 2,
            painted_frames: Some(1),
            aborted: true,
            build: "abc1234-dirty".into(),
            display_hz: Some(240.0),
            display_scale: None,
        };
        let path = r.dump_json(&dir, "macos", &run).unwrap();
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        assert!(name.starts_with("b1-fling-macos-") && name.ends_with(".json"));
        let body = std::fs::read_to_string(&path).unwrap();
        assert!(body.contains("\"kind\":\"frames\"") && body.contains("\"values_ms\":[16"));
        for field in [
            "\"expected_samples\":1100",
            "\"inactive_keys\":2",
            "\"painted_frames\":1",
            "\"aborted\":true",
            "\"build\":\"abc1234-dirty\"",
            "\"display_hz\":240",
            "\"display_scale\":\"unknown\"",
        ] {
            assert!(body.contains(field), "{field} missing in {body}");
        }
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn complete_runs_keep_their_threshold_verdict() {
        assert_eq!(
            verdict("b1-fling", "macos", &ok(), &b1_run()),
            Verdict::Pass
        );
        assert_eq!(
            verdict("b3-typing", "macos", &lat(2_000), &lat_run(2_000)),
            Verdict::Pass
        );
        // 95 % of the expected samples is still complete.
        assert_eq!(
            verdict("b3-typing", "macos", &lat(1_900), &lat_run(2_000)),
            Verdict::Pass
        );
    }

    #[test]
    fn zero_samples_fail() {
        assert_eq!(
            verdict("b3-typing", "macos", &lat(0), &lat_run(0)),
            Verdict::Fail
        );
        let empty = stats_of(&[]);
        let run = RunInfo {
            painted_frames: Some(0),
            ..b1_run()
        };
        assert_eq!(verdict("b1-fling", "macos", &empty, &run), Verdict::Fail);
    }

    #[test]
    fn partial_runs_fail_below_95_percent_of_expected() {
        assert_eq!(
            verdict("b3-typing", "macos", &lat(1_899), &lat_run(2_000)),
            Verdict::Fail
        );
        let run = RunInfo {
            expected_samples: 1_300,
            ..b1_run()
        };
        assert_eq!(verdict("b1-fling", "macos", &ok(), &run), Verdict::Fail);
    }

    #[test]
    fn inactive_keys_fail() {
        let run = RunInfo {
            inactive_keys: 1,
            ..lat_run(2_000)
        };
        assert_eq!(
            verdict("b3-typing", "macos", &lat(2_000), &run),
            Verdict::Fail
        );
        let run = RunInfo {
            inactive_keys: 1,
            ..b1_run()
        };
        assert_eq!(verdict("b1-fling", "macos", &ok(), &run), Verdict::Fail);
    }

    #[test]
    fn aborted_runs_fail() {
        let run = RunInfo {
            aborted: true,
            ..lat_run(2_000)
        };
        assert_eq!(
            verdict("b3-completion", "macos", &lat(2_000), &run),
            Verdict::Fail
        );
    }

    #[test]
    fn b1_needs_painted_frames_for_95_percent_of_intervals() {
        let run = RunInfo {
            painted_frames: Some(1_139),
            ..b1_run()
        };
        assert_eq!(verdict("b1-fling", "macos", &ok(), &run), Verdict::Fail);
        let run = RunInfo {
            painted_frames: Some(1_140),
            ..b1_run()
        };
        assert_eq!(verdict("b1-fling", "macos", &ok(), &run), Verdict::Pass);
        let run = RunInfo {
            painted_frames: None,
            ..b1_run()
        };
        assert_eq!(verdict("b1-fling", "macos", &ok(), &run), Verdict::Fail);
    }

    #[test]
    fn b1_needs_at_least_600_frames() {
        let s = stats_of(&[16.0; 599]);
        let run = RunInfo {
            expected_samples: 0,
            painted_frames: Some(599),
            ..b1_run()
        };
        assert_eq!(verdict("b1-fling", "macos", &s, &run), Verdict::Fail);
        let s = stats_of(&[16.0; 600]);
        let run = RunInfo {
            expected_samples: 0,
            painted_frames: Some(600),
            ..b1_run()
        };
        assert_eq!(verdict("b1-fling", "macos", &s, &run), Verdict::Pass);
    }

    #[test]
    fn b1_expected_frames_follow_the_os_threshold() {
        assert_eq!(b1_expected_frames("macos", 20.0), 1_100);
        assert_eq!(b1_expected_frames("windows", 20.0), 1_100);
        assert_eq!(b1_expected_frames("linux", 20.0), 900);
    }

    #[test]
    fn build_label_marks_dirty_trees() {
        assert_eq!(build_label("abc1234", ""), "abc1234");
        assert_eq!(
            build_label("abc1234", " M xtask/src/bench_report.rs\0"),
            "abc1234-dirty"
        );
        assert_eq!(
            build_label("abc1234", "?? spikes/new.rs\0"),
            "abc1234-dirty"
        );
        let ignored = "?? target/bench/b3-typing.json\0?? .superpowers/sdd/x-report.md\0";
        assert_eq!(build_label("abc1234", ignored), "abc1234");
        // Only untracked files there are ignored; a tracked change still counts.
        assert_eq!(
            build_label("abc1234", " M .superpowers/x.md\0"),
            "abc1234-dirty"
        );
    }

    #[test]
    fn build_label_ignores_benchmark_docs() {
        // Writing results into docs/benchmarks must not make the next run look dirty.
        assert_eq!(
            build_label("abc1234", " M docs/benchmarks/m0.md\0"),
            "abc1234"
        );
        assert_eq!(
            build_label("abc1234", "?? docs/benchmarks/m1.md\0"),
            "abc1234"
        );
        assert_eq!(
            build_label("abc1234", " M docs/other.md\0"),
            "abc1234-dirty"
        );
    }

    #[test]
    fn bench_doc_edits_are_not_dirty() {
        assert_eq!(
            build_label("abc1234", " M docs/benchmarks/m1.md\0"),
            "abc1234"
        );
    }

    #[test]
    fn non_ascii_path_is_dirty_without_quoting() {
        assert_eq!(
            build_label("abc1234", " M crates/ui/İçerik.rs\0"),
            "abc1234-dirty"
        );
    }

    #[test]
    fn rename_source_path_is_not_parsed_as_an_entry() {
        assert_eq!(
            build_label(
                "abc1234",
                "R  docs/benchmarks/new.md\0docs/benchmarks/old.md\0 M docs/benchmarks/m1.md\0"
            ),
            "abc1234"
        );
        assert_eq!(
            build_label("abc1234", "R  crates/new.rs\0docs/benchmarks/old.md\0"),
            "abc1234-dirty"
        );
    }

    #[test]
    fn rename_is_clean_only_if_both_paths_are_ignorable() {
        assert_eq!(
            build_label("abc1234", "R  docs/benchmarks/new.md\0crates/old.rs\0"),
            "abc1234-dirty"
        );
        assert_eq!(
            build_label(
                "abc1234",
                "R  docs/benchmarks/new.md\0docs/benchmarks/old.md\0"
            ),
            "abc1234"
        );
    }

    #[test]
    fn entries_are_split_on_nul() {
        assert_eq!(
            build_label("abc1234", "?? target/x.json\0 M crates/a.rs\0"),
            "abc1234-dirty"
        );
        assert_eq!(
            build_label("abc1234", "?? target/x.json\0?? .superpowers/y.md\0"),
            "abc1234"
        );
    }
}
