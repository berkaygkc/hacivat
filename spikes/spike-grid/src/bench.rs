//! `--bench fling`: a synthetic fling driven from `on_next_frame`, recorded with
//! `FrameRecorder("b1-fling")`.
//!
//! Velocity is `V0 * exp(-t / TAU)`, restarted every `REFLING_S`, for `DURATION_S`. The scroll
//! delta per frame is the exact integral of that curve between the two frame timestamps, so the
//! distance travelled does not depend on the frame rate.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::{Duration, Instant};

use hacivat_ui_core::frame_loop::{bench_dir, record_frames, stamp_run};
use hacivat_ui_core::frames::{FrameRecorder, RunInfo, Verdict, b1_expected_frames, verdict};
use hacivat_ui_core::gpui_surface::{App, Entity, Window};

use crate::GridView;

/// Initial fling velocity, px/s.
pub const V0: f64 = 12_000.0;
/// Exponential decay time constant, s.
pub const TAU: f64 = 0.325;
/// A new fling starts every `REFLING_S` seconds.
pub const REFLING_S: f64 = 1.5;
/// Total recorded fling time, s.
pub const DURATION_S: f64 = 20.0;
/// Idle time after the window opens before recording starts, so first-frame font loading and
/// atlas warm-up are not counted as a fling frame.
pub const WARMUP: Duration = Duration::from_millis(750);

/// Distance scrolled `t` seconds after the first fling started, in px.
pub fn fling_distance(t: f64) -> f64 {
    let t = t.max(0.0);
    let flings = (t / REFLING_S).floor();
    let local = t - flings * REFLING_S;
    let per_fling = V0 * TAU * (1.0 - (-REFLING_S / TAU).exp());
    flings * per_fling + V0 * TAU * (1.0 - (-local / TAU).exp())
}

/// Completeness fields of a B1 run on `os`: the fling expects `DURATION_S` at the minimum
/// passing fps, `inactive` fling steps ran while the window was inactive, and the grid canvas
/// painted `painted` times. Provenance (build, display) is added by `stamp_run`.
pub fn fling_run(os: &str, inactive: usize, painted: usize) -> RunInfo {
    RunInfo {
        expected_samples: b1_expected_frames(os, DURATION_S),
        inactive_keys: inactive,
        painted_frames: Some(painted),
        ..RunInfo::unstamped()
    }
}

/// Process exit code: non-zero when the JSON dump could not be written. A FAIL verdict on a
/// complete run still exits 0; the verdict is the result.
pub fn exit_code(dumped: bool) -> i32 {
    i32::from(!dumped)
}

/// Waits out the warm-up, then starts recording and drives the fling.
pub fn start(view: Entity<GridView>, window: &mut Window) {
    let opened = Instant::now();
    warm_up(view, window, opened);
}

fn warm_up(view: Entity<GridView>, window: &mut Window, opened: Instant) {
    window.on_next_frame(move |window, cx| {
        if opened.elapsed() < WARMUP {
            warm_up(view, window, opened);
            return;
        }
        let recorder = Rc::new(RefCell::new(FrameRecorder::new("b1-fling")));
        view.update(cx, |v, _| v.state.borrow_mut().stats = Default::default());
        record_frames(window, recorder.clone());
        drive(view, window, recorder, Rc::default(), Instant::now(), 0.0);
    });
}

fn drive(
    view: Entity<GridView>,
    window: &mut Window,
    recorder: Rc<RefCell<FrameRecorder>>,
    inactive: Rc<Cell<usize>>,
    started: Instant,
    travelled: f64,
) {
    window.on_next_frame(move |window, cx| {
        let t = started.elapsed().as_secs_f64();
        if t >= DURATION_S {
            finish(&view, &recorder, inactive.get(), window, cx);
            return;
        }
        if !window.is_window_active() {
            inactive.set(inactive.get() + 1);
        }
        let d = fling_distance(t);
        view.update(cx, |v, cx| {
            v.scroll_px(d - travelled, 0.0);
            cx.notify();
        });
        drive(view, window, recorder, inactive, started, d);
    });
}

fn finish(
    view: &Entity<GridView>,
    recorder: &Rc<RefCell<FrameRecorder>>,
    inactive: usize,
    window: &Window,
    cx: &mut App,
) {
    let mut rec = recorder.borrow_mut();
    rec.stop();
    println!("b1-fling FrameStats: {:?}", rec.stats());
    let pos = view.read(cx).pos;
    println!("b1-fling final position: {pos:?}");
    let painted = view.read(cx).state.borrow().stats.paint_ms.len();
    let os = std::env::consts::OS;
    let stamp = stamp_run(window, cx);
    let run = RunInfo {
        build: stamp.build,
        display_hz: stamp.display_hz,
        display_scale: stamp.display_scale,
        ..fling_run(os, inactive, painted)
    };
    {
        let v = view.read(cx);
        let state = v.state.borrow();
        print_cpu("paint (grid canvas paint closure)", &state.stats.paint_ms);
        print_cpu(
            "frame CPU (render start -> paint end)",
            &state.stats.frame_cpu_ms,
        );
        let n = state.stats.misses.len().max(1) as f64;
        let misses: u64 = state.stats.misses.iter().map(|&m| m as u64).sum();
        let cells: u64 = state.stats.cells.iter().map(|&m| m as u64).sum();
        println!(
            "b1-fling grid frames painted: {}, mean visible cells/frame: {:.0}, mean shape misses/frame: {:.1}",
            state.stats.paint_ms.len(),
            cells as f64 / n,
            misses as f64 / n
        );
    }
    let ver = match verdict("b1-fling", os, &rec.stats(), &run) {
        Verdict::Pass => "PASS",
        Verdict::Fail => "FAIL",
        Verdict::NoThreshold => "n/a",
    };
    println!(
        "b1-fling verdict={ver} frames={}/{} painted={painted} inactive_steps={inactive} build={} display_hz={:?} display_scale={:?}",
        rec.stats().frames,
        run.expected_samples,
        run.build,
        run.display_hz,
        run.display_scale,
    );
    if inactive > 0 {
        eprintln!(
            "warning: {inactive} fling steps ran while the window was inactive; GPUI throttles inactive windows, so this run is not representative"
        );
    }
    let dumped = match rec.dump_json(&bench_dir(), os, &run) {
        Ok(path) => {
            println!("b1-fling wrote {}", path.display());
            true
        }
        Err(e) => {
            eprintln!("b1-fling: failed to write JSON: {e}");
            false
        }
    };
    let code = exit_code(dumped);
    if code != 0 {
        std::process::exit(code);
    }
    cx.quit();
}

fn print_cpu(what: &str, samples: &[f64]) {
    let mut s = samples.to_vec();
    s.sort_by(|a, b| a.total_cmp(b));
    let pct = |p: f64| {
        if s.is_empty() {
            0.0
        } else {
            s[(((p / 100.0) * s.len() as f64).ceil() as usize).clamp(1, s.len()) - 1]
        }
    };
    let over = s.iter().filter(|&&v| v > 6.0).count();
    println!(
        "b1-fling {what}: n={} p50={:.2}ms p95={:.2}ms p99={:.2}ms max={:.2}ms over-6ms={}",
        s.len(),
        pct(50.0),
        pct(95.0),
        pct(99.0),
        s.last().copied().unwrap_or(0.0),
        over
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_complete_fling_expects_at_least_600_frames() {
        for os in ["macos", "windows", "linux"] {
            let run = fling_run(os, 0, 0);
            assert!(
                run.expected_samples >= hacivat_ui_core::frames::B1_MIN_FRAMES,
                "{os}"
            );
            assert_eq!(run.painted_frames, Some(0));
        }
        assert_eq!(fling_run("macos", 3, 7).inactive_keys, 3);
    }

    #[test]
    fn unwritten_dumps_exit_non_zero() {
        assert_eq!(exit_code(true), 0);
        assert_eq!(exit_code(false), 1);
    }

    #[test]
    fn fling_starts_at_v0_and_decays() {
        let dt = 1e-6;
        let v_start = fling_distance(dt) / dt;
        assert!((v_start - V0).abs() / V0 < 1e-3, "{v_start}");
        let v_tau = (fling_distance(TAU + dt) - fling_distance(TAU)) / dt;
        assert!(
            (v_tau - V0 / std::f64::consts::E).abs() / V0 < 1e-3,
            "{v_tau}"
        );
    }

    #[test]
    fn refling_restores_velocity_and_distance_is_continuous() {
        let dt = 1e-6;
        let before = fling_distance(REFLING_S - dt);
        let after = fling_distance(REFLING_S + dt);
        assert!(after - before < 1.0);
        let v = (fling_distance(REFLING_S + 2.0 * dt) - after) / dt;
        assert!((v - V0).abs() / V0 < 1e-3, "{v}");
    }

    #[test]
    fn distance_is_monotonic_over_the_run() {
        let mut prev = 0.0;
        for i in 1..=2_000 {
            let d = fling_distance(i as f64 * DURATION_S / 2_000.0);
            assert!(d >= prev);
            prev = d;
        }
        // ~13.3 flings of V0 * TAU * (1 - e^(-1.5/0.325)) px each.
        assert!((50_000.0..55_000.0).contains(&prev), "{prev}");
    }
}
