//! The synthetic B1 bench (Task 20; features `bench`, `test-support`): the M0 fling over the
//! real `GridView` and `SyntheticSource::spike(10_000_000, 50)` in a 1440 × 900 window.
//!
//! - `b1_synthetic` runs interactively (trackpad and wheel).
//! - `b1_synthetic --bench fling` waits out a 750 ms warm-up, jumps to row 5,000,000 (an
//!   absolute `f32` pixel offset would have lost sub-pixel precision there; the row-anchored
//!   model does not care), then drives the M0 fling from `on_next_frame` for 20 s: velocity
//!   `V0 · exp(−t / τ)`, V0 = 12,000 px/s, τ = 0.325 s, restarted every 1.5 s, each frame's
//!   delta the exact integral of that curve between two frame timestamps. Frames are recorded
//!   with `FrameRecorder("b1-fling-synthetic")`; it prints the frame statistics, the grid's
//!   paint and frame CPU percentiles, shapes and fast-scroll frames, and writes
//!   `<workspace>/target/bench/b1-fling-synthetic-<os>-<ts>.json` (exit 1 if it cannot).

use std::cell::{Cell, RefCell};
use std::process::ExitCode;
use std::rc::Rc;
use std::time::{Duration, Instant};

use hacivat_ui_core::app::{Boot, open_window, run_app};
use hacivat_ui_core::frame_loop::{bench_dir, record_frames, stamp_run};
use hacivat_ui_core::frames::{FrameRecorder, RunInfo, Verdict, b1_expected_frames, verdict};
use hacivat_ui_core::gpui_surface::{App, AppContext as _, Entity, Window};
use hacivat_ui_grid::GridView;
use hacivat_ui_grid::testing::{SyntheticSource, grid_config};

const ROWS: u64 = 10_000_000;
const COLS: u32 = 50;
/// DESIGN.md's reference window.
const WINDOW: (f32, f32) = (1440.0, 900.0);
const LABEL: &str = "b1-fling-synthetic";

/// Initial fling velocity, px/s.
const V0: f64 = 12_000.0;
/// Exponential decay time constant, s.
const TAU: f64 = 0.325;
/// A new fling starts every `REFLING_S` seconds.
const REFLING_S: f64 = 1.5;
/// Recorded fling time, s.
const DURATION_S: f64 = 20.0;
/// Idle time before recording, so first-frame font loading and atlas warm-up are not counted.
const WARMUP: Duration = Duration::from_millis(750);

/// Distance scrolled `t` seconds after the first fling started, in px.
fn fling_distance(t: f64) -> f64 {
    let t = t.max(0.0);
    let flings = (t / REFLING_S).floor();
    let local = t - flings * REFLING_S;
    let per_fling = V0 * TAU * (1.0 - (-REFLING_S / TAU).exp());
    flings * per_fling + V0 * TAU * (1.0 - (-local / TAU).exp())
}

fn start(view: Entity<GridView>, window: &mut Window) {
    let opened = Instant::now();
    warm_up(view, window, opened, false);
}

fn warm_up(view: Entity<GridView>, window: &mut Window, opened: Instant, jumped: bool) {
    window.on_next_frame(move |window, cx| {
        if !jumped {
            view.update(cx, |v, cx| v.scroll_to_row(ROWS / 2, cx));
        }
        if opened.elapsed() < WARMUP {
            warm_up(view, window, opened, true);
            return;
        }
        let recorder = Rc::new(RefCell::new(FrameRecorder::new(LABEL)));
        view.update(cx, |v, _| {
            *v.bench_stats() = Default::default();
            v.reset_frame_log();
        });
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
        view.update(cx, |v, cx| v.scroll_px(d - travelled, 0.0, cx));
        drive(view, window, recorder, inactive, started, d);
    });
}

fn percentiles(samples: &[f64]) -> (usize, f64, f64, f64, f64) {
    let mut s = samples.to_vec();
    s.sort_by(f64::total_cmp);
    let pct = |p: f64| {
        if s.is_empty() {
            0.0
        } else {
            s[(((p / 100.0) * s.len() as f64).ceil() as usize).clamp(1, s.len()) - 1]
        }
    };
    let max = s.last().copied().unwrap_or(0.0);
    (s.len(), pct(50.0), pct(95.0), pct(99.0), max)
}

fn print_cpu(what: &str, samples: &[f64]) {
    let (n, p50, p95, p99, max) = percentiles(samples);
    let over_3 = samples.iter().filter(|v| **v > 3.0).count();
    let over_6 = samples.iter().filter(|v| **v > 6.0).count();
    println!(
        "{LABEL} {what}: n={n} p50={p50:.2}ms p95={p95:.2}ms p99={p99:.2}ms max={max:.2}ms over-3ms={over_3} over-6ms={over_6}"
    );
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
    println!("{LABEL} FrameStats: {:?}", rec.stats());
    let (painted, log, top) = view.update(cx, |v, _| {
        let b = v.bench_stats().clone();
        print_cpu("paint (grid paint)", &b.paint_ms);
        print_cpu("frame CPU (grid prepaint start -> paint end)", &b.frame_cpu_ms);
        let n = b.shapes.len().max(1) as f64;
        let shapes: u64 = b.shapes.iter().map(|s| u64::from(*s)).sum();
        let cells: u64 = b.cells.iter().map(|s| u64::from(*s)).sum();
        let fast = b.fast.iter().filter(|f| **f).count();
        println!(
            "{LABEL} grid frames painted: {}, mean visible cells/frame: {:.0}, mean shapes/frame: {:.1}, fast-scroll frames: {fast}",
            b.paint_ms.len(),
            cells as f64 / n,
            shapes as f64 / n,
        );
        (b.paint_ms.len(), *v.frame_log(), v.scroll_state().top_row)
    });
    println!("{LABEL} frame log: {log:?}; final top row: {top}");
    let os = std::env::consts::OS;
    let stamp = stamp_run(window, cx);
    let run = RunInfo {
        build: stamp.build,
        display_hz: stamp.display_hz,
        display_scale: stamp.display_scale,
        expected_samples: b1_expected_frames(os, DURATION_S),
        inactive_keys: inactive,
        painted_frames: Some(painted),
        ..RunInfo::unstamped()
    };
    let ver = match verdict(LABEL, os, &rec.stats(), &run) {
        Verdict::Pass => "PASS",
        Verdict::Fail => "FAIL",
        Verdict::NoThreshold => "n/a",
    };
    println!(
        "{LABEL} verdict={ver} frames={}/{} painted={painted} inactive_steps={inactive} build={} display_hz={:?} display_scale={:?}",
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
    match rec.dump_json(&bench_dir(), os, &run) {
        Ok(path) => println!("{LABEL} wrote {}", path.display()),
        Err(e) => {
            eprintln!("{LABEL}: failed to write JSON: {e}");
            std::process::exit(1);
        }
    }
    cx.quit();
}

enum Mode {
    Interactive,
    BenchFling,
}

fn parse_args() -> Result<Mode, String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .as_slice()
    {
        [] => Ok(Mode::Interactive),
        ["--bench", "fling"] => Ok(Mode::BenchFling),
        other => Err(format!(
            "unknown arguments {other:?}; usage: b1_synthetic [--bench fling]"
        )),
    }
}

fn main() -> ExitCode {
    let mode = match parse_args() {
        Ok(m) => m,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::from(2);
        }
    };
    run_app(Boot::default(), move |cx| {
        let handle = match open_window(cx, "Hacivat B1 synthetic", WINDOW, |_, cx| {
            cx.new(|cx| {
                let source = Rc::new(SyntheticSource::spike(ROWS, COLS));
                GridView::new(source, grid_config("B1 synthetic"), cx)
            })
        }) {
            Ok(h) => h,
            Err(e) => {
                eprintln!("failed to open window: {e:#}");
                cx.quit();
                return;
            }
        };
        cx.activate(true);
        if let Mode::BenchFling = mode
            && let Err(e) = handle.update(cx, |_, window, cx| start(cx.entity(), window))
        {
            // Without the driver the app would idle forever and never write a result.
            eprintln!("could not start the fling bench: {e:#}");
            std::process::exit(1);
        }
    });
    ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_fling_starts_at_v0_and_decays() {
        let dt = 1e-6;
        let v = fling_distance(dt) / dt;
        assert!((v - V0).abs() / V0 < 1e-3, "{v}");
        let at_tau = (fling_distance(TAU + dt) - fling_distance(TAU)) / dt;
        assert!((at_tau - V0 / std::f64::consts::E).abs() / V0 < 1e-3);
    }

    #[test]
    fn the_distance_grows_over_the_run() {
        let total = fling_distance(DURATION_S);
        assert!((50_000.0..55_000.0).contains(&total), "{total}");
    }
}
