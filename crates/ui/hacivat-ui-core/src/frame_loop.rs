//! Wires a `FrameRecorder` to GPUI's per-frame callback, and stamps bench runs with their
//! build and display.

use crate::frames::{FrameRecorder, RunInfo, build_stamp};
use crate::gpui_surface::{App, Window};
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant};

/// The cargo workspace root (`crates/ui/hacivat-ui-core` -> `../../..`).
pub fn workspace_root() -> PathBuf {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    manifest.ancestors().nth(3).unwrap_or(manifest).to_path_buf()
}

/// Where benches write their JSON dumps: `<workspace>/target/bench`, independent of the cwd.
pub fn bench_dir() -> PathBuf {
    workspace_root().join("target").join("bench")
}

fn hz_of(interval: Option<Duration>) -> Option<f64> {
    interval
        .filter(|d| !d.is_zero())
        .map(|d| 1.0 / d.as_secs_f64())
}

/// A [`RunInfo`] with this checkout's build stamp and `window`'s display refresh rate and scale,
/// taken now (at bench time). The caller fills in the completeness fields.
pub fn stamp_run(window: &Window, cx: &App) -> RunInfo {
    RunInfo {
        build: build_stamp(&workspace_root()),
        display_hz: hz_of(window.refresh_interval(cx)),
        display_scale: Some(window.scale_factor()),
        ..RunInfo::unstamped()
    }
}

/// Records every frame via `Window::on_next_frame`, re-arming itself until
/// `recorder.stop()` is called.
pub fn record_frames(window: &mut Window, recorder: Rc<RefCell<FrameRecorder>>) {
    window.on_next_frame(move |window, _cx| {
        let recording = {
            let mut r = recorder.borrow_mut();
            if r.is_recording() {
                r.on_frame(Instant::now());
            }
            r.is_recording()
        };
        if recording {
            record_frames(window, recorder);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bench_dir_is_under_the_workspace_target() {
        let root = workspace_root();
        let manifest = std::fs::read_to_string(root.join("Cargo.toml")).unwrap();
        assert!(manifest.contains("[workspace]"), "{}", root.display());
        assert_eq!(bench_dir(), root.join("target").join("bench"));
    }

    #[test]
    fn display_hz_comes_from_the_refresh_interval() {
        assert_eq!(hz_of(Some(std::time::Duration::from_micros(4_167))).map(f64::round), Some(240.0));
        assert_eq!(hz_of(None), None);
        assert_eq!(hz_of(Some(std::time::Duration::ZERO)), None);
    }
}
