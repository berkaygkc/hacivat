//! Frame-interval and input-latency recording plus pass/fail verdicts.
//!
//! This file is deliberately free of GPUI types: `xtask` includes it via `#[path]`
//! so the report uses exactly the same statistics and thresholds as the recorders.

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

/// Something a verdict can be computed for.
pub trait Judge {
    fn judge(&self, label: &str, os: &str) -> Verdict;
}

impl From<bool> for Verdict {
    fn from(ok: bool) -> Self {
        if ok { Verdict::Pass } else { Verdict::Fail }
    }
}

impl Judge for FrameStats {
    fn judge(&self, label: &str, os: &str) -> Verdict {
        if label.starts_with("b1-fling") {
            let min_fps = if os == "linux" { 45.0 } else { 55.0 };
            return (self.mean_fps >= min_fps && self.gaps_over_100ms == 0).into();
        }
        Verdict::NoThreshold
    }
}

impl Judge for LatencyStats {
    fn judge(&self, label: &str, _os: &str) -> Verdict {
        if label.starts_with("b3-completion") {
            (self.p95_ms <= 50.0).into()
        } else if label.starts_with("b3-typing") {
            (self.p95_ms <= 16.0).into()
        } else {
            Verdict::NoThreshold
        }
    }
}

pub fn verdict(label: &str, os: &str, stats: &impl Judge) -> Verdict {
    stats.judge(label, os)
}

fn host_name() -> String {
    std::env::var("HOSTNAME")
        .or_else(|_| std::env::var("COMPUTERNAME"))
        .ok()
        .filter(|h| !h.is_empty())
        .or_else(|| {
            let out = std::process::Command::new("hostname").output().ok()?;
            let h = String::from_utf8(out.stdout).ok()?.trim().to_string();
            (!h.is_empty()).then_some(h)
        })
        .unwrap_or_else(|| "unknown".into())
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
fn write_dump(dir: &Path, label: &str, os: &str, kind: &str, values_ms: &[f64]) -> io::Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let ts = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let path = dir.join(format!("{label}-{os}-{ts}.json"));
    let values: Vec<String> = values_ms.iter().map(|v| format!("{v}")).collect();
    let body = format!(
        "{{\"label\":{},\"os\":{},\"host\":{},\"kind\":{},\"values_ms\":[{}]}}\n",
        json_str(label),
        json_str(os),
        json_str(&host_name()),
        json_str(kind),
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
        Self { label: label.to_string(), last: None, intervals_ms: Vec::new(), recording: true }
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

    pub fn dump_json(&self, dir: &Path, os: &str) -> io::Result<PathBuf> {
        write_dump(dir, &self.label, os, "frames", &self.intervals_ms)
    }
}

pub struct LatencyProbe {
    label: String,
    pending: Option<Instant>,
    samples_ms: Vec<f64>,
}

impl LatencyProbe {
    pub fn new(label: &str) -> Self {
        Self { label: label.to_string(), pending: None, samples_ms: Vec::new() }
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

    pub fn dump_json(&self, dir: &Path, os: &str) -> io::Result<PathBuf> {
        write_dump(dir, &self.label, os, "latency", &self.samples_ms)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn ok() -> FrameStats {
        stats_of(&[16.0; 10])
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
        let s = FrameStats { mean_fps: 54.9, ..ok() };
        assert_eq!(verdict("b1-fling", "macos", &s), Verdict::Fail);
    }

    #[test]
    fn report_marks_b1_pass_45fps_on_linux() {
        let s = FrameStats { mean_fps: 45.0, ..ok() };
        assert_eq!(verdict("b1-fling", "linux", &s), Verdict::Pass);
    }

    #[test]
    fn b1_fails_on_long_gap() {
        let s = FrameStats { gaps_over_100ms: 1, ..ok() };
        assert_eq!(verdict("b1-fling", "macos", &s), Verdict::Fail);
    }

    #[test]
    fn b3_thresholds() {
        let l = |p95| LatencyStats { samples: 1, p50_ms: 1.0, p95_ms: p95, max_ms: p95 };
        assert_eq!(verdict("b3-typing-small", "macos", &l(16.0)), Verdict::Pass);
        assert_eq!(verdict("b3-typing-small", "macos", &l(16.1)), Verdict::Fail);
        assert_eq!(verdict("b3-completion", "macos", &l(50.0)), Verdict::Pass);
        assert_eq!(verdict("b3-completion", "macos", &l(50.1)), Verdict::Fail);
        assert_eq!(verdict("other", "macos", &l(1.0)), Verdict::NoThreshold);
    }

    #[test]
    fn dump_json_writes_raw_intervals() {
        let dir = std::env::temp_dir().join(format!("hacivat-frames-{}", std::process::id()));
        let t0 = Instant::now();
        let mut r = FrameRecorder::new("b1-fling");
        r.on_frame(t0);
        r.on_frame(t0 + Duration::from_millis(16));
        let path = r.dump_json(&dir, "macos").unwrap();
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        assert!(name.starts_with("b1-fling-macos-") && name.ends_with(".json"));
        let body = std::fs::read_to_string(&path).unwrap();
        assert!(body.contains("\"kind\":\"frames\"") && body.contains("\"values_ms\":[16"));
        std::fs::remove_dir_all(&dir).ok();
    }
}
