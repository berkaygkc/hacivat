//! B3 in the real app (Task 36, R-M3-11): typing latency in a console's editor, in the whole
//! app, over two fixtures (review I4, a plan correction):
//! - **T0** `cargo xtask gen-sql --lines 50000 --out target/fixtures/b3-50k-t0.sql` (≈ 2 MB, no
//!   long line: the editor's full tier) for `typing`, `typing-with-results`, `open` and
//!   `scroll`. `typing`, `typing-with-results` and `open` refuse (INVALID, exit 3) unless the
//!   editor opened it in the full tier, and `open` unless the file is 1.5–2.5 MB.
//! - **T1** `cargo xtask gen-sql --lines 50000 --long-line 100000 --out
//!   target/fixtures/b3-50k.sql` for `typing-long-line` and `ime-bounds`: its 101,491-character
//!   line (over 64 KB) opens the file in Reduced mode, which is the product's truth there.
//!
//! Every result line prints the editor's tier.
//!
//! The console is bound to a PostgreSQL profile served by a `FakeDriver` (no socket) and opened
//! through the console host's restore path. It is connected, so every keystroke pays for what a
//! connected console does: the off-thread analysis, the preview band, the status segment.
//!
//! Modes (`--bench <mode>`):
//! - `typing`: 2,000 keystrokes at 60 Hz at the end of line 25,000 (a SQL fragment with Turkish
//!   text, typed and erased);
//! - `typing-long-line`: the same in the middle of the longest line (≥ 100,000 characters);
//! - `typing-with-results`: `typing` while a 1M-row × 50-column `FakeDriver` result, fetched in
//!   full ("Fetch all", then "Fetch more anyway" past the 256 MiB cap), is open in the grid
//!   under the editor;
//! - `scroll`: 1,000 PageDown/PageUp keystrokes at 60 Hz from the top of the file (keyboard
//!   paging, not the wheel: `gpui_surface` exports no wheel event to synthesise; label
//!   `b3-scroll-pagedown-app`);
//! - `open`: five opens of the file in new console tabs: first paint of the editor and the first
//!   full analysis (lex and split of the whole file, off the UI thread), each from the moment the
//!   console's file is on disk and its tab opens;
//! - `ime-bounds`: the editor's `bounds_for_range` (the IME candidate window's question) at line
//!   49,000, 200 times (UI-EDIT-12 test 8).
//!
//!
//! M3c Task 44 adds three typing modes (graded against 16 ms p95 like `typing`):
//! - `typing-multi-caret`: `typing` with [`MULTI_CARETS`] carets: one at the end of line 25,000,
//!   and one on each of the 9 lines under it at the same column (`editor.add_caret_below`), so
//!   at a line's end only where that line is no longer (the column is clamped to the line);
//!   every keystroke is typed at each (T0 fixture);
//! - `typing-with-folds`: `typing` with [`FOLDS`] folds of five hidden lines, one every fifty
//!   lines ([`fold_plan`]; the caret's line is in none), kept through the run (T0 fixture);
//! - `typing-19mb`: `typing` at the middle line of a 19–20 MiB file (Reduced mode, the
//!   product's truth there; ledger 298): `gen-sql --lines 520000`.
//!
//! **R-M3-9 in the real app's worker.** Every typing mode also arms the language worker's own
//! timer (`hacivat_sql_editor::bench`) for its measured keystrokes and prints the incremental
//! relex figure (the scan for the one edit plus `SqlDocument::apply`, on the worker) with
//! p50/p95/p99/max, and each part on its own; `typing` grades the sum against 2 ms p95 (the
//! 50k-line fixture R-M3-9 names) over at least [`MIN_RELEX_SAMPLES`] incremental analyses
//! (fewer: INVALID). The figures go to `target/bench/relex/`.
//!
//! **Conditions (M3c Task 44).** A graded run needs a refresh of at least 118 Hz (no upper
//! bound, any scale; every result line records the actual Hz and scale) and the `tr-TR` format
//! applied by the app.
//! - A display under 118 Hz, or reporting none, is INVALID (exit 3, no JSON) with the
//!   `PENDING-PO` note, or ungraded with `--any-display` (label `b3-offpanel-…`).
//! - Another format runs ungraded (`b3-offlocale-…`).
//! - A window that cannot become key (the display asleep, the session locked), keystrokes to an
//!   inactive window mid-run, or a display that stops qualifying by the end, is INVALID too;
//!   `--any-display` does not change that.
//! - `--allow-inactive` (the CI harness check) goes on and is always ungraded.
//!
//! Every result line gives p50/p95/p99/max.
//!
//! Keys pass the app's dispatcher (`Window::dispatch_keystroke`: the keymap, then the editor's
//! input handler). The latency probe is the editor's `FrameProbe`: marked at the editor's handler
//! entry, recorded once the frame that shows the input is presented ([`super::PresentProbe`]).
//! Before measuring, unmeasured keystrokes (the phrase typed and erased once; 20 pages down and up
//! for `scroll`) warm the editor and GPUI's keyboard/mouse input
//! modality switch, which redraws the whole window once (ledger T36 note). JSON goes to
//! `bench_dir()` with `stamp_run`'s build and display; `cargo xtask bench-report` reads it.

use std::cell::Cell;
use std::path::PathBuf;
use std::process::ExitCode;
use std::rc::Rc;
use std::time::{Duration, Instant};

use anyhow::{Context as _, bail};
use hacivat_driver::fake::{FakeDriver, FakeQuery, FakeResultSet, FakeRows};
use hacivat_proto::{DbmsKind, ProfileId};
use hacivat_secrets::AuthProviderId;
use hacivat_session::profile::{Profile, ProfileEndpoint};
use hacivat_ui::panes::TabId;
use hacivat_ui_core::app::{Boot, run_app};
use hacivat_ui_core::frame_loop::{bench_dir, stamp_run};
use hacivat_ui_core::frames::{LatencyProbe, RunInfo, verdict};
use hacivat_ui_core::gpui_surface::{
    App, AppContext, EntityInputHandler, Focusable, Keystroke, Modifiers, WindowHandle,
};
use hacivat_ui_editor::{EditorView, FrameProbe, Tier};
use hacivat_values::{DecimalVal, LogicalType, TsVal, Value};

use super::{
    BenchRoot, EXIT_INVALID, EXIT_USAGE, Figures, Launch, Outcome, PENDING_PO, PresentProbe, Sleep,
    activate_or_refuse, connect, connected, console_editor, console_ready, end_run, fake_registry,
    flag, open_console, percentiles, print_result, save_profile, start_app, verdict_text,
    wait_until,
};
use crate::console::{ConsoleId, Consoles};
use crate::shell::workspace::Workspace;

/// 60 Hz.
const PERIOD: Duration = Duration::from_micros(16_667);
const WARMUP: Duration = Duration::from_millis(1_000);
const SETTLE: Duration = Duration::from_millis(500);
const TYPING_KEYSTROKES: usize = 2_000;
const SCROLL_KEYSTROKES: usize = 1_000;
/// Unmeasured keystrokes before the measured ones (the input-modality redraw, first shapes).
const WARM_KEYSTROKES: usize = 40;
/// "Line 25,000", 1-based.
const TYPING_LINE: usize = 25_000;
const LONG_LINE_MIN_CHARS: usize = 100_000;
/// UI-EDIT-12 test 8: the caret at line 49,000.
const IME_LINE: usize = 49_000;
const IME_SAMPLES: usize = 200;
const OPEN_RUNS: usize = 5;
/// The budgets the gate reads for `open` (spec ALG 25 §2.1): first paint, full analysis.
const OPEN_FIRST_PAINT_MS: f64 = 150.0;
const OPEN_ANALYSIS_MS: f64 = 600.0;
/// UI-EDIT-12 test 8.
const IME_BOUNDS_MS: f64 = 1.0;
/// The result `typing-with-results` keeps open.
pub const RESULT_ROWS: u64 = 1_000_000;
pub const RESULT_COLS: usize = 50;
/// The statement `typing-with-results` runs (the first line of its text).
const RESULT_SQL: &str = "SELECT * FROM hv_bench_rows";
const PROFILE_NAME: &str = "b3-bench";
const LOAD_TIMEOUT: Duration = Duration::from_secs(600);
// ## m3c-t44: the M3c modes.
/// The carets `typing-multi-caret` types at (line 25,000 and the lines under it).
pub const MULTI_CARETS: usize = 10;
/// The folds `typing-with-folds` keeps (the `hacivat-text` perf test's shape).
pub const FOLDS: usize = 1_000;
/// One fold every this many lines...
const FOLD_EVERY: usize = 50;
/// ...hiding this many lines under its header.
const FOLD_HIDDEN: usize = 5;
/// The smallest file `typing-19mb` measures (ledger 298: "19 MB", the editor's perf test).
pub const LARGE_MIN_BYTES: usize = 19 << 20;
/// R-M3-9: the worker's incremental relex, p95.
pub const RELEX_BUDGET_MS: f64 = 2.0;
// ## m3c-t44 (fix round 2, M1)
/// The fewest incremental analyses that support an R-M3-9 figure: a quarter of the 2,000
/// measured keystrokes (at 60 Hz the worker analyses nearly every one; far fewer means it
/// coalesced most jobs, and the figure would not describe typing).
pub const MIN_RELEX_SAMPLES: usize = TYPING_KEYSTROKES / 4;
/// The format a graded run needs.
pub const GRADED_FORMAT: &str = "tr-TR";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Typing,
    TypingLongLine,
    TypingWithResults,
    Scroll,
    Open,
    ImeBounds,
    // ## m3c-t44
    TypingMultiCaret,
    TypingWithFolds,
    Typing19Mb,
}

impl Mode {
    pub fn parse(s: &str) -> Option<Mode> {
        Some(match s {
            "typing" => Mode::Typing,
            "typing-long-line" => Mode::TypingLongLine,
            "typing-with-results" => Mode::TypingWithResults,
            "scroll" => Mode::Scroll,
            "open" => Mode::Open,
            "ime-bounds" => Mode::ImeBounds,
            // ## m3c-t44
            "typing-multi-caret" => Mode::TypingMultiCaret,
            "typing-with-folds" => Mode::TypingWithFolds,
            "typing-19mb" => Mode::Typing19Mb,
            _ => return None,
        })
    }

    // ## m3c-t44
    /// The modes that type the phrase and are graded against 16 ms p95.
    pub fn is_typing(self) -> bool {
        matches!(
            self,
            Mode::Typing
                | Mode::TypingLongLine
                | Mode::TypingWithResults
                | Mode::TypingMultiCaret
                | Mode::TypingWithFolds
                | Mode::Typing19Mb
        )
    }

    /// The JSON label. The typing labels start with `b3-typing`, so `hacivat-bench` grades them
    /// against 16 ms p95; the others have no shared threshold (their budgets are printed here).
    pub fn label(self) -> &'static str {
        match self {
            Mode::Typing => "b3-typing-app",
            Mode::TypingLongLine => "b3-typing-long-line-app",
            Mode::TypingWithResults => "b3-typing-with-results-app",
            Mode::Scroll => "b3-scroll-pagedown-app",
            Mode::Open => "b3-open-app",
            Mode::ImeBounds => "b3-ime-bounds-app",
            // ## m3c-t44
            Mode::TypingMultiCaret => "b3-typing-multi-caret-app",
            Mode::TypingWithFolds => "b3-typing-with-folds-app",
            Mode::Typing19Mb => "b3-typing-19mb-app",
        }
    }

    /// The tier the mode measures, when it must be that one (review I4): the 50k-line file
    /// without a long line opens in the full tier.
    pub fn required_tier(self) -> Option<Tier> {
        match self {
            Mode::Typing | Mode::TypingWithResults | Mode::Open => Some(Tier::Full),
            Mode::TypingLongLine | Mode::Scroll | Mode::ImeBounds => None,
            // ## m3c-t44: the 19 MiB file is over the full tier's 4 MB: Reduced is its truth.
            Mode::TypingMultiCaret | Mode::TypingWithFolds => Some(Tier::Full),
            Mode::Typing19Mb => Some(Tier::Reduced),
        }
    }
}

// ## m3c-t44
/// Whether `typing-19mb` may judge a file of `bytes` (at least [`LARGE_MIN_BYTES`]).
pub fn large_size_ok(bytes: usize) -> Result<(), String> {
    // ## m3c-t44 (fix round 2, M3): below the Reduced ceiling, which is also the largest
    // console file the host restores (both 20 MiB); a larger file would not open at all.
    let max = large_max_bytes();
    if (LARGE_MIN_BYTES..max).contains(&bytes) {
        Ok(())
    } else {
        Err(format!(
            "typing-19mb is judged on a file of at least {LARGE_MIN_BYTES} bytes (19 MiB) and under {max} (the 20 MiB Reduced and restore ceiling); this one has {bytes} (gen-sql --lines 520000)"
        ))
    }
}

/// The first size `typing-19mb` refuses: the console's restore ceiling, which equals the
/// editor's Reduced ceiling (`TierLimits::reduced_max_bytes`).
pub fn large_max_bytes() -> usize {
    let restore = crate::console::autosave::RESTORE_MAX_BYTES;
    let reduced = hacivat_ui_editor::TierLimits::default().reduced_max_bytes;
    usize::try_from(restore.min(reduced)).unwrap_or(usize::MAX)
}

/// The folds of `typing-with-folds` in a file of `lines` lines: `(header, last hidden line)`,
/// 0-based, one every [`FOLD_EVERY`] lines from line 1, each hiding [`FOLD_HIDDEN`] lines, at
/// most [`FOLDS`]. None holds the typing line (index 24,999).
pub fn fold_plan(lines: usize) -> Vec<(usize, usize)> {
    (0..FOLDS)
        .map(|i| {
            let header = i * FOLD_EVERY + 1;
            (header, header + FOLD_HIDDEN)
        })
        .take_while(|&(_, last)| last < lines)
        .collect()
}

/// Whether the window's display runs at B3's reference refresh: at least 118 Hz (B1-real's
/// `PANEL_MIN_HZ`), with no upper bound and at any scale (controller ruling, T44 fix round 1:
/// the PO's graded M3a B3 ran on a 120 Hz 1× display, the M3b gate's on 240 Hz 1×). Every
/// result line records the actual refresh and scale. Unknown refresh (the display asleep) is
/// refused.
pub fn display_ok(hz: Option<f64>, scale: Option<f32>) -> Result<(), String> {
    let min = super::b1_real::PANEL_MIN_HZ;
    match hz {
        Some(hz) if hz >= min => Ok(()),
        _ => Err(format!(
            "B3 is graded at >= {min} Hz; the display reports {hz:?} at scale {scale:?}"
        )),
    }
}

/// The worker's relex samples of a run, as R-M3-9 reads them.
#[derive(Debug, Clone, PartialEq)]
pub struct RelexReport {
    /// Analyses that took one edit incrementally (the R-M3-9 case).
    pub incremental: usize,
    /// Analyses with nothing to relex, and fresh builds (not relex figures).
    pub unchanged: usize,
    pub fresh: usize,
    /// The incremental analyses' scan + `SqlDocument::apply`: what R-M3-9 grades.
    pub relex: super::Figures,
    // ## m3c-t44 (fix round 2, M1): the two parts, printed so a figure near 2 ms says which.
    /// The scan for the one edit (`change_between`; grows with the document).
    pub diff: super::Figures,
    /// `SqlDocument::apply` (the incremental relex and resplit).
    pub apply: super::Figures,
    /// The incremental analyses' whole analysis (with split, highlight, statements, folds).
    pub analysis: super::Figures,
}

impl RelexReport {
    pub fn of(samples: &[hacivat_sql_editor::bench::WorkerSample]) -> RelexReport {
        use hacivat_sql_editor::bench::Built;
        let ms = |d: Duration| d.as_secs_f64() * 1000.0;
        let inc: Vec<_> = samples
            .iter()
            .filter(|s| s.kind == Built::Incremental)
            .collect();
        let count = |k: Built| samples.iter().filter(|s| s.kind == k).count();
        let figures = |f: &dyn Fn(&hacivat_sql_editor::bench::WorkerSample) -> Duration| {
            super::Figures::of(&inc.iter().map(|s| ms(f(s))).collect::<Vec<_>>())
        };
        RelexReport {
            incremental: inc.len(),
            unchanged: count(Built::Unchanged),
            fresh: count(Built::Fresh),
            relex: figures(&|s| s.relex()),
            diff: figures(&|s| s.diff),
            apply: figures(&|s| s.apply),
            analysis: figures(&|s| s.analysis),
        }
    }

    /// R-M3-9's outcome: graded only when `graded` (the `typing` run on the reference panel).
    /// Fewer than [`MIN_RELEX_SAMPLES`] incremental analyses cannot support the figure: the
    /// run is INVALID (fix round 2, M1), never a PASS or a FAIL.
    pub fn outcome(&self, graded: bool) -> Outcome {
        if !graded {
            Outcome::NotGraded
        } else if self.incremental < MIN_RELEX_SAMPLES {
            Outcome::Invalid
        } else if self.relex.p95 > RELEX_BUDGET_MS {
            Outcome::Fail
        } else {
            Outcome::Pass
        }
    }

    /// The log line.
    pub fn line(&self, label: &str, outcome: Outcome) -> String {
        format!(
            "{label} R-M3-9 worker relex (scan + SqlDocument::apply, incremental analyses, at least {MIN_RELEX_SAMPLES}): {} (budget {RELEX_BUDGET_MS} ms p95: {}); scan {}; apply {}; whole analysis {}; unchanged={} fresh={}",
            self.relex.text(3),
            outcome.text(),
            self.diff.text(3),
            self.apply.text(3),
            self.analysis.text(3),
            self.unchanged,
            self.fresh,
        )
    }

    pub fn json(&self, label: &str, outcome: Outcome) -> serde_json::Value {
        serde_json::json!({
            "label": label,
            "requirement": "R-M3-9",
            "budget_p95_ms": RELEX_BUDGET_MS,
            "min_incremental": MIN_RELEX_SAMPLES,
            "outcome": outcome.text(),
            "incremental": self.incremental,
            "unchanged": self.unchanged,
            "fresh": self.fresh,
            "relex_ms": self.relex.json(),
            "scan_ms": self.diff.json(),
            "apply_ms": self.apply.json(),
            "analysis_ms": self.analysis.json(),
        })
    }
}

/// The size `open` is judged at ("open 2 MB", E3).
const OPEN_BYTES: std::ops::RangeInclusive<usize> = 1_500_000..=2_500_000;

/// Whether `tier` is the one `mode` measures; the reason when it is not.
pub fn tier_ok(mode: Mode, tier: Tier) -> Result<(), String> {
    match mode.required_tier() {
        // ## m3c-t44 (fix round 2, M3): the hint names the mode's own fixture.
        Some(want) if want != tier => Err(format!(
            "{} measures the {want:?} tier but the editor opened the file in {tier:?}; {}",
            mode.label(),
            if mode == Mode::Typing19Mb {
                "use the 19 MiB fixture (gen-sql --lines 520000, no --long-line)"
            } else {
                "use the T0 fixture (gen-sql --lines 50000, no --long-line)"
            }
        )),
        _ => Ok(()),
    }
}

/// Whether `open` may judge a file of `bytes` against the 2 MB budgets.
pub fn open_size_ok(bytes: usize) -> Result<(), String> {
    if OPEN_BYTES.contains(&bytes) {
        Ok(())
    } else {
        Err(format!(
            "open is judged on a ≈ 2 MB file; this one has {bytes} bytes (1,500,000–2,500,000)"
        ))
    }
}

/// `b3 --file <path> --bench <mode> [--lang <tag>] [--format <locale>]`.
#[derive(Debug, Clone)]
pub struct Args {
    pub mode: Mode,
    pub file: PathBuf,
    pub lang: String,
    pub format: String,
    /// Go on when the window cannot become frontmost (a harness check; the verdict fails).
    pub allow_inactive: bool,
    // ## m3c-t44
    /// Run on whatever display the window is on, ungraded (`b3-offpanel-…`).
    pub any_display: bool,
}

pub const USAGE: &str = "usage: b3 --file <fixture> --bench typing|typing-long-line|typing-with-results|typing-multi-caret|typing-with-folds|typing-19mb|scroll|open|ime-bounds [--lang tr] [--format tr-TR] [--any-display] [--allow-inactive]\n\
  typing, typing-with-results, typing-multi-caret, typing-with-folds, open, scroll: --file target/fixtures/b3-50k-t0.sql (gen-sql --lines 50000)\n\
  typing-long-line, ime-bounds: --file target/fixtures/b3-50k.sql (gen-sql --lines 50000 --long-line 100000)\n\
  typing-19mb: --file target/fixtures/b3-19mb.sql (gen-sql --lines 520000; 19-20 MiB, Reduced mode)\n\
  Graded only at >= 118 Hz (any scale) with the tr-TR format applied:\n\
  - a display under 118 Hz or reporting none: INVALID (exit 3, no JSON), or ungraded with --any-display (b3-offpanel-*);\n\
  - another format: ungraded (b3-offlocale-*, JSON written);\n\
  - a window that cannot become frontmost, or keystrokes to an inactive window mid-run: INVALID (PENDING-PO); --any-display does not change this;\n\
  - --allow-inactive (the CI harness check): goes on, always ungraded (b3-offpanel-*).\n\
  The last line is `RESULT: PASS|FAIL|INVALID|n/a <label>`; exit 0 = PASS or ungraded, 1 = FAIL or aborted, 3 = INVALID (exit 0 is never a PASS by itself: read RESULT).";

pub fn parse_args(args: &[String]) -> Result<Args, String> {
    let mode = flag(args, "--bench").ok_or("missing --bench <mode>")?;
    let mode = Mode::parse(mode).ok_or_else(|| format!("unknown mode {mode:?}"))?;
    let file = PathBuf::from(flag(args, "--file").ok_or("missing --file <path>")?);
    Ok(Args {
        mode,
        file,
        lang: flag(args, "--lang").unwrap_or("tr").to_string(),
        format: flag(args, "--format").unwrap_or(GRADED_FORMAT).to_string(),
        allow_inactive: args.iter().any(|a| a == "--allow-inactive"),
        any_display: args.iter().any(|a| a == "--any-display"),
    })
}

// ## m3c-t44: graded only on the reference panel and locale (Task 44's review line).
impl Args {
    /// The run's label: the mode's own (graded) on the reference display in `tr-TR`; with
    /// `on_panel == false` `b3-offpanel-…`, with another format `b3-offlocale-…` (neither
    /// starts with `b3-typing`, so `hacivat-bench` grades neither). `format` is the format the
    /// app applied (fix round 2, M5), not the one asked for.
    pub fn label(&self, on_panel: bool, format: &str) -> String {
        let base = self.mode.label();
        let rest = base.strip_prefix("b3-").unwrap_or(base);
        if !on_panel {
            format!("b3-offpanel-{rest}")
        } else if format != GRADED_FORMAT {
            format!("b3-offlocale-{rest}")
        } else {
            base.to_string()
        }
    }

    /// Whether a run with this label is graded (`format`: the format the app applied).
    pub fn graded(&self, on_panel: bool, format: &str) -> bool {
        on_panel && format == GRADED_FORMAT
    }
}

/// The run's display decision: graded on the reference display; ungraded with `--any-display`
/// and, whatever the display, under `--allow-inactive` (a harness check, with a warning: fix
/// round 2, I1); otherwise refused (`Err`: INVALID).
pub fn display_decision(args: &Args, display: Result<(), String>) -> Result<bool, String> {
    if args.any_display {
        return Ok(false);
    }
    if args.allow_inactive {
        eprintln!(
            "warning: --allow-inactive is a harness check: the run is ungraded ({})",
            match &display {
                Ok(()) => "the display would qualify".to_string(),
                Err(why) => why.clone(),
            }
        );
        return Ok(false);
    }
    display.map(|()| true)
}

/// An ungraded run never claims PASS or FAIL (INVALID stays INVALID).
pub fn ungraded(outcome: Outcome) -> Outcome {
    match outcome {
        Outcome::Pass | Outcome::Fail => Outcome::NotGraded,
        other => other,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Char(char),
    Backspace,
    PageDown,
    PageUp,
}

impl Key {
    fn keystroke(self) -> Keystroke {
        let (key, key_char) = match self {
            Key::Char(' ') => ("space".to_string(), Some(" ".to_string())),
            Key::Char(c) => (c.to_string(), Some(c.to_string())),
            Key::Backspace => ("backspace".to_string(), None),
            Key::PageDown => ("pagedown".to_string(), None),
            Key::PageUp => ("pageup".to_string(), None),
        };
        Keystroke {
            modifiers: Modifiers::default(),
            key,
            key_char,
        }
    }
}

/// What the typing scripts type (and erase).
const PHRASE: &str = " AND c.city = 'İzmir' OR o.total > 4200";

/// The unmeasured keystrokes before a run: the phrase typed and erased once (typing modes), or
/// 20 pages down and 20 up (`scroll`), so the text and the caret end where they began.
pub fn warm_script(mode: Mode) -> Vec<Key> {
    match mode {
        Mode::Scroll => std::iter::repeat_n(Key::PageDown, WARM_KEYSTROKES / 2)
            .chain(std::iter::repeat_n(Key::PageUp, WARM_KEYSTROKES / 2))
            .collect(),
        _ => typing_script(2 * PHRASE.chars().count()),
    }
}

/// Types a SQL fragment (with Turkish text) and erases it again, until `n` keystrokes (the
/// spike's script).
pub fn typing_script(n: usize) -> Vec<Key> {
    let len = PHRASE.chars().count();
    PHRASE
        .chars()
        .map(Key::Char)
        .chain(std::iter::repeat_n(Key::Backspace, len))
        .cycle()
        .take(n)
        .collect()
}

/// Fifty pages down, fifty up, until `n` keystrokes.
pub fn scroll_script(n: usize) -> Vec<Key> {
    std::iter::repeat_n(Key::PageDown, 50)
        .chain(std::iter::repeat_n(Key::PageUp, 50))
        .cycle()
        .take(n)
        .collect()
}

/// The text a mode opens: the fixture, with the result's statement on a first line of its own
/// for `typing-with-results` (the typing line stays the fixture's line 25,000).
pub fn console_text(mode: Mode, fixture: &str) -> String {
    match mode {
        Mode::TypingWithResults => format!("{RESULT_SQL};\n{fixture}"),
        _ => fixture.to_string(),
    }
}

/// The cell at (`row`, `col`) of the `typing-with-results` result: int8, text with Turkish
/// words, numeric(14,2) and timestamptz, cycling (the B1 shape).
fn result_cell(row: u64, col: usize) -> Value {
    const WORDS: [&str; 8] = [
        "çiçek",
        "ağaç",
        "Iğdır",
        "İzmir",
        "şehir",
        "öğrenci",
        "gümüş",
        "ılık",
    ];
    let k = row.wrapping_mul(col as u64 + 1);
    match col % 4 {
        0 => Value::Int(k as i64),
        1 => Value::Text(format!("{} {k:x}", WORDS[(row as usize + col) % WORDS.len()]).into()),
        2 => DecimalVal::parse(&format!("{}.{:02}", k % 1_000_000, k % 100))
            .map_or(Value::Null, Value::Decimal),
        _ => Value::TimestampTz {
            utc: TsVal {
                secs: 1_704_067_200 + (k % 31_536_000) as i64,
                nanos: 0,
            },
            offset: None,
        },
    }
}

fn result_columns() -> Vec<(&'static str, LogicalType)> {
    (0..RESULT_COLS)
        .map(|c| {
            let name: &'static str = Box::leak(format!("c{c:02}").into_boxed_str());
            let logical = match c % 4 {
                0 => LogicalType::Int {
                    bits: 64,
                    signed: true,
                },
                1 => LogicalType::Text {
                    max_chars: None,
                    fixed: false,
                },
                2 => LogicalType::Decimal {
                    precision: Some(14),
                    scale: Some(2),
                },
                _ => LogicalType::Timestamp {
                    precision: 6,
                    tz: true,
                },
            };
            (name, logical)
        })
        .collect()
}

/// The `typing-with-results` result: `rows` rows of [`RESULT_COLS`] columns (the B1 shape).
pub(crate) fn result_query(rows: u64) -> FakeQuery {
    FakeQuery {
        prefix: "SELECT",
        sets: vec![FakeResultSet {
            columns: result_columns(),
            rows: FakeRows::Generated {
                count: rows,
                cell: result_cell,
            },
        }],
        ..FakeQuery::default()
    }
}

/// The PostgreSQL `FakeDriver` of `mode`: the 1M-row result for `typing-with-results`.
fn bench_fake(mode: Mode) -> FakeDriver {
    super::fake_pg(if mode == Mode::TypingWithResults {
        vec![result_query(RESULT_ROWS)]
    } else {
        Vec::new()
    })
}

pub(crate) fn profile() -> Profile {
    Profile {
        name: PROFILE_NAME.to_string(),
        engine: DbmsKind::Postgres,
        endpoint: ProfileEndpoint::Server {
            host: "127.0.0.1".to_string(),
            port: None,
        },
        user: Some("bench".to_string()),
        auth: AuthProviderId::None,
        ..Profile::default()
    }
}

/// Runs the bench; the process exits with its code.
pub fn run(args: Args) -> ExitCode {
    let fixture = match std::fs::read_to_string(&args.file) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("b3: cannot read {}: {e}\n{USAGE}", args.file.display());
            return ExitCode::from(EXIT_USAGE);
        }
    };
    // ## m3c-t44: `typing-19mb` checks its file's size as `open` does.
    let size = match args.mode {
        Mode::Open => open_size_ok(fixture.len()),
        Mode::Typing19Mb => large_size_ok(fixture.len()),
        _ => Ok(()),
    };
    if let Err(why) = size {
        println!("INVALID: {why}");
        print_result(Outcome::Invalid, args.mode.label());
        return ExitCode::from(EXIT_INVALID as u8);
    }
    let text = console_text(args.mode, &fixture);
    let root = match BenchRoot::new("b3") {
        Ok(r) => r,
        Err(e) => {
            eprintln!("b3: {e:#}");
            return ExitCode::FAILURE;
        }
    };
    let profile = profile();
    let profile_id = profile.id;
    if let Err(e) = save_profile(&root, &[profile]) {
        eprintln!("b3: {e:#}");
        root.remove();
        return ExitCode::FAILURE;
    }
    let launch = Launch {
        root: root.clone(),
        lang: args.lang.clone(),
        format: args.format.clone(),
        zone: "Europe/Istanbul".to_string(),
        drivers: fake_registry(bench_fake(args.mode)),
    };
    println!(
        "b3 {}: {} ({} bytes, {} lines)",
        args.mode.label(),
        args.file.display(),
        text.len(),
        text.lines().count()
    );
    run_app(Boot::default(), move |cx| {
        let ws = match start_app(cx, launch) {
            Ok(ws) => ws,
            Err(e) => {
                eprintln!("b3: opening the workspace failed: {e:#}");
                end_run(cx, &root, 1);
            }
        };
        let exec = cx.background_executor().clone();
        cx.spawn(async move |cx| {
            let sleep = move |d: Duration| exec.timer(d);
            let code = match script(cx, ws, &sleep, &args, &root, profile_id, &text).await {
                // ## m3c-t44: the label the run was measured under (ungraded off the panel).
                Ok((outcome, label)) => {
                    print_result(outcome, &label);
                    outcome.exit_code(args.allow_inactive)
                }
                Err(e) => {
                    eprintln!("b3 aborted: {e:#}");
                    print_result(Outcome::Fail, &format!("{} (aborted)", args.mode.label()));
                    1
                }
            };
            cx.update(|cx| end_run(cx, &root, code));
        })
        .detach();
    });
    ExitCode::SUCCESS
}

async fn script<C: AppContext>(
    cx: &mut C,
    ws: WindowHandle<Workspace>,
    sleep: &impl Sleep,
    args: &Args,
    root: &BenchRoot,
    profile: ProfileId,
    text: &str,
) -> anyhow::Result<(Outcome, String)> {
    // ## m3c-t44: a window that cannot become key (the display asleep, the session locked) and
    // a display other than the reference panel refuse the run: INVALID, no JSON, PENDING-PO.
    let refused = args.mode.label().to_string();
    if let Some(why) = activate_or_refuse(cx, ws, sleep, args.allow_inactive).await? {
        println!("INVALID: {why}; no JSON written. {PENDING_PO}");
        return Ok((Outcome::Invalid, refused));
    }
    let (hz, scale) = print_display(cx, ws, args.mode.label())?;
    let on_panel = match display_decision(args, display_ok(hz, scale)) {
        Ok(on_panel) => on_panel,
        Err(why) => {
            println!(
                "INVALID display: {why}; no JSON written (--any-display runs ungraded). {PENDING_PO}"
            );
            return Ok((Outcome::Invalid, refused));
        }
    };
    // ## m3c-t44 (fix round 2, M5): graded on the format the app applied, as B1-real does.
    let Some((format, _zone)) = ws.update(cx, |_, _, cx| super::format_locale(cx))? else {
        bail!("no localization");
    };
    if format != args.format {
        bail!("the format locale is {format}, not {}", args.format);
    }
    let label = args.label(on_panel, &format);
    let graded = args.graded(on_panel, &format);
    println!("{label}: format={format} graded={graded}");
    let outcome = measure(cx, ws, sleep, args, &label, graded, root, profile, text).await?;
    Ok((if graded { outcome } else { ungraded(outcome) }, label))
}

#[allow(clippy::too_many_arguments)]
async fn measure<C: AppContext>(
    cx: &mut C,
    ws: WindowHandle<Workspace>,
    sleep: &impl Sleep,
    args: &Args,
    label: &str,
    graded: bool,
    root: &BenchRoot,
    profile: ProfileId,
    text: &str,
) -> anyhow::Result<Outcome> {
    if args.mode == Mode::Open {
        return open_bench(cx, ws, sleep, root, profile, text).await;
    }
    let paths = root.paths().clone();
    let (tab, _) = ws.update(cx, |ws, window, cx| {
        open_console(ws, profile, text, window, cx, &paths)
    })??;
    let (console, editor) = wait_console(cx, ws, sleep, tab).await?;
    let tier = ws.update(cx, |_, _, cx| editor.read(cx).status().tier)?;
    if let Err(why) = tier_ok(args.mode, tier) {
        println!("INVALID: {why}");
        return Ok(Outcome::Invalid);
    }
    ws.update(cx, |_, _, cx| connect(cx, profile, None))?;
    wait_until(
        cx,
        ws,
        sleep,
        "the connection",
        Duration::from_secs(30),
        |_, _, cx| connected(cx, profile),
    )
    .await?;
    wait_until(
        cx,
        ws,
        sleep,
        "the console's SQL context",
        Duration::from_secs(30),
        |_, _, cx| console_ready(cx, console),
    )
    .await?;
    if args.mode == Mode::TypingWithResults {
        let loaded = super::run_and_load(
            cx,
            ws,
            sleep,
            console,
            &editor,
            0..RESULT_SQL.len(),
            LOAD_TIMEOUT,
        )
        .await?;
        println!(
            "b3 result: {} rows × {RESULT_COLS} columns complete in {:.2} s (first rows {:.0} ms), store {}, RSS {}, cap raised: {}",
            loaded.rows,
            loaded.complete.as_secs_f64(),
            loaded.first_rows.as_secs_f64() * 1000.0,
            super::mib(loaded.bytes),
            super::rss_bytes().map_or("unknown".into(), super::mib),
            loaded.raised,
        );
        if loaded.rows != RESULT_ROWS {
            bail!("the result has {} rows, not {RESULT_ROWS}", loaded.rows);
        }
        wait_until(
            cx,
            ws,
            sleep,
            "the grid of the result",
            Duration::from_secs(10),
            |_, _, cx| super::result_grid(cx, console).is_some(),
        )
        .await?;
    }
    ws.update(cx, |_, window, cx| {
        let handle = editor.focus_handle(cx);
        window.focus(&handle, cx);
    })?;
    if args.mode == Mode::ImeBounds {
        return ime_bounds(cx, ws, sleep, &editor, tier).await;
    }
    let (script, expected) = place_caret(cx, ws, args.mode, &editor)?;
    // Let the caret's line paint.
    wait_until(
        cx,
        ws,
        sleep,
        "the caret's frame",
        Duration::from_secs(5),
        |_, _, cx| editor.read(cx).caret_bounds().is_some(),
    )
    .await?;
    let probe = PresentProbe::new(label);
    ws.update(cx, |_, _, cx| {
        editor.update(cx, |e, _| e.set_probe(Some(probe.as_frame_probe())));
    })?;
    // Warm-up: the input-modality redraw and the first shapes, unmeasured; it leaves the text
    // and the caret where they were.
    let warm = warm_script(args.mode);
    replay(cx, ws, sleep, &warm).await;
    sleep(WARMUP).await;
    let shapes_before = editor_shapes(cx, ws, &editor)?;
    // ## m3c-t44: the language worker's own relex timer runs with the probe (R-M3-9).
    // Fix round 2 (M8): a guard, so an aborted run never leaves the timer armed.
    let worker_timer = hacivat_sql_editor::bench::armed_until_dropped();
    probe.arm(true);
    let (inactive, aborted) = replay(cx, ws, sleep, &script).await;
    sleep(SETTLE).await;
    probe.arm(false);
    drop(worker_timer);
    let worker = hacivat_sql_editor::bench::take();
    let shapes = editor_shapes(cx, ws, &editor)?.saturating_sub(shapes_before);
    // ## m3c-t44: a multi-caret or folds run measured what it claims only if its carets and
    // folds were there to the end.
    let (carets, folds) = ws.update(cx, |_, _, cx| {
        let e = editor.read(cx);
        (
            e.snapshot().selection.ranges().len(),
            e.folds().folded().len(),
        )
    })?;
    if let Err(why) = kept_through_run(args.mode, carets, folds) {
        println!("INVALID: {why}; no JSON written");
        return Ok(Outcome::Invalid);
    }
    let (inputs, frames) = probe.counts();
    let mut run = ws.update(cx, |_, window, cx| stamp_run(window, cx))?;
    run.expected_samples = expected;
    run.inactive_keys = inactive;
    run.aborted = aborted;
    let latency = probe.latency();
    let stats = latency.stats();
    let os = std::env::consts::OS;
    let v = verdict(label, os, &stats, &run);
    if args.mode == Mode::Scroll {
        println!("{label}: keyboard paging (PageDown/PageUp), not the wheel");
    }
    // ## m3c-t44: p99 next to p50/p95/max; the carets and folds the run kept.
    let fig = Figures::of(latency.samples_ms());
    println!(
        "{label}: tier={tier:?} samples={}/{} p50={:.2}ms p95={:.2}ms p99={:.2}ms max={:.2}ms verdict={} inputs_marked={inputs} frames_showing_input={frames} inactive_keys={} aborted={} lines_shaped={shapes} carets={carets} folds={folds} build={} display_hz={:?} display_scale={:?}",
        stats.samples,
        run.expected_samples,
        fig.p50,
        fig.p95,
        fig.p99,
        fig.max,
        verdict_text(v),
        run.inactive_keys,
        run.aborted,
        run.build,
        run.display_hz,
        run.display_scale,
    );
    // ## m3c-t44 (fix round 2, M2): focus lost or the display gone mid-run is a run that could
    // not measure: INVALID with the PENDING-PO note, no JSON (a harness check goes on).
    if let Err(why) = held_through_run(
        inactive,
        args.allow_inactive,
        graded,
        display_ok(run.display_hz, run.display_scale),
    ) {
        println!("INVALID: {why}; no JSON written. {PENDING_PO}");
        return Ok(Outcome::Invalid);
    }
    if inactive > 0 {
        eprintln!(
            "warning: {inactive} keystrokes went to an inactive window; GPUI throttles inactive windows, so this run is not representative"
        );
    }
    let dumped = dump(&latency, &run);
    let b3 = if run.aborted || dumped.is_none() {
        Outcome::Fail
    } else {
        Outcome::of(v)
    };
    if !args.mode.is_typing() {
        return Ok(b3);
    }
    // ## m3c-t44: R-M3-9 in the real app's worker; graded in `typing` on the panel only.
    let relex_graded = graded && args.mode == Mode::Typing;
    let relex = RelexReport::of(&worker);
    let relex_outcome = relex.outcome(relex_graded);
    println!("{}", relex.line(label, relex_outcome));
    if let Some(path) = &dumped {
        write_relex(path, &relex.json(label, relex_outcome));
    }
    Ok(if relex_graded {
        worse(b3, relex_outcome)
    } else {
        b3
    })
}

// ## m3c-t44 (fix round 2, M2)
/// Whether the run held its conditions to the end: no keystroke to an inactive window (focus
/// lost, the display asleep, the screen saver) unless `--allow-inactive` (a harness check), and
/// on a graded run the display still qualifying at the end (`display_end`).
pub fn held_through_run(
    inactive_keys: usize,
    allow_inactive: bool,
    graded: bool,
    display_end: Result<(), String>,
) -> Result<(), String> {
    if inactive_keys > 0 && !allow_inactive {
        return Err(format!(
            "{inactive_keys} keystrokes went to an inactive window (focus lost or the display slept mid-run)"
        ));
    }
    match display_end {
        Err(why) if graded => Err(format!("at the end of the run: {why}")),
        _ => Ok(()),
    }
}

// ## m3c-t44
/// Whether a run kept what its mode measures: every caret of `typing-multi-caret` and every
/// fold of `typing-with-folds` to the end.
pub fn kept_through_run(mode: Mode, carets: usize, folds: usize) -> Result<(), String> {
    match mode {
        Mode::TypingMultiCaret if carets != MULTI_CARETS => Err(format!(
            "{carets} of {MULTI_CARETS} carets were left at the end of the run"
        )),
        Mode::TypingWithFolds if folds != FOLDS => Err(format!(
            "{folds} of {FOLDS} folds were left at the end of the run"
        )),
        _ => Ok(()),
    }
}

/// The R-M3-9 figures of a run: `target/bench/relex/<the latency dump's name>`, outside
/// `target/bench/*.json` (which `bench-report` reads as frames or latency dumps).
fn write_relex(dump: &std::path::Path, body: &serde_json::Value) {
    let dir = bench_dir().join("relex");
    let path = dir.join(dump.file_name().unwrap_or_default());
    match serde_json::to_vec_pretty(body)
        .map_err(std::io::Error::other)
        .and_then(|b| std::fs::create_dir_all(&dir).and_then(|()| std::fs::write(&path, b)))
    {
        Ok(()) => println!("wrote {}", path.display()),
        Err(e) => eprintln!("could not write {}: {e}", path.display()),
    }
}

/// Prints the window's viewport, refresh and scale, and returns the refresh and the scale (the
/// run's display decision reads them; the JSON records them either way).
fn print_display<C: AppContext>(
    cx: &mut C,
    ws: WindowHandle<Workspace>,
    label: &str,
) -> anyhow::Result<(Option<f64>, Option<f32>)> {
    let (run, w, h) = ws.update(cx, |_, window, cx| {
        let v = window.viewport_size();
        (
            stamp_run(window, cx),
            f32::from(v.width),
            f32::from(v.height),
        )
    })?;
    println!(
        "{label}: viewport={w}x{h} display_hz={:?} display_scale={:?}",
        run.display_hz, run.display_scale
    );
    Ok((run.display_hz, run.display_scale))
}

/// Waits for the console host to open `tab`'s editor.
async fn wait_console<C: AppContext>(
    cx: &mut C,
    ws: WindowHandle<Workspace>,
    sleep: &impl Sleep,
    tab: TabId,
) -> anyhow::Result<(ConsoleId, hacivat_ui_core::gpui_surface::Entity<EditorView>)> {
    wait_until(
        cx,
        ws,
        sleep,
        "the console's editor",
        Duration::from_secs(30),
        |_, _, cx| console_editor(cx, tab).is_some(),
    )
    .await?;
    ws.update(cx, |_, _, cx| console_editor(cx, tab))?
        .context("the console's editor went away")
}

fn editor_shapes<C: AppContext>(
    cx: &mut C,
    ws: WindowHandle<Workspace>,
    editor: &hacivat_ui_core::gpui_surface::Entity<EditorView>,
) -> anyhow::Result<u64> {
    ws.update(cx, |_, _, cx| editor.read(cx).layout_stats().shapes)
}

/// Places the caret for `mode` (and, for the M3c modes, the carets or the folds); the measured
/// script and its expected samples.
pub(crate) fn place_caret<C: AppContext>(
    cx: &mut C,
    ws: WindowHandle<Workspace>,
    mode: Mode,
    editor: &hacivat_ui_core::gpui_surface::Entity<EditorView>,
) -> anyhow::Result<(Vec<Key>, usize)> {
    ws.update(cx, |_, window, cx| {
        let snap = editor.read(cx).snapshot();
        let buffer = &snap.buffer;
        // ## m3c-t44: 1,000 folds before the caret is placed (none holds the typing line).
        if mode == Mode::TypingWithFolds {
            let plan = fold_plan(buffer.len_lines());
            if plan.len() != FOLDS {
                bail!(
                    "the file has {} lines, too few for {FOLDS} folds",
                    buffer.len_lines()
                );
            }
            let ranges: Vec<std::ops::Range<usize>> = plan
                .iter()
                .map(|&(header, last)| {
                    buffer.line_range(header, false).start..buffer.line_range(last, false).end
                })
                .collect();
            editor.update(cx, |e, cx| {
                for r in ranges {
                    hacivat_ui_editor::fold_gutter::fold_range(e, r, cx);
                }
            });
            let made = editor.read(cx).folds().folded().len();
            if made != FOLDS {
                bail!("{made} of {FOLDS} folds were made");
            }
        }
        let (at, script) = match mode {
            // ## m3c-t44: the multi-caret and folds runs type where `typing` does.
            Mode::Typing
            | Mode::TypingWithResults
            | Mode::TypingMultiCaret
            | Mode::TypingWithFolds => {
                // The fixture's line 25,000 (one line lower under the result's statement).
                let line = TYPING_LINE - 1 + usize::from(mode == Mode::TypingWithResults);
                if line >= buffer.len_lines() {
                    bail!("the file has fewer than {TYPING_LINE} lines");
                }
                (
                    buffer.line_range(line, false).end,
                    typing_script(TYPING_KEYSTROKES),
                )
            }
            Mode::TypingLongLine => {
                let (line, chars) = (0..buffer.len_lines())
                    .map(|l| (l, buffer.slice(buffer.line_range(l, false)).chars().count()))
                    .max_by_key(|&(_, n)| n)
                    .context("empty file")?;
                if chars < LONG_LINE_MIN_CHARS {
                    bail!(
                        "the longest line has {chars} characters; generate the fixture with --long-line {LONG_LINE_MIN_CHARS}"
                    );
                }
                let r = buffer.line_range(line, false);
                (r.start + (r.end - r.start) / 2, typing_script(TYPING_KEYSTROKES))
            }
            // ## m3c-t44: the end of the 19 MiB file's middle line.
            Mode::Typing19Mb => (
                buffer.line_range(buffer.len_lines() / 2, false).end,
                typing_script(TYPING_KEYSTROKES),
            ),
            Mode::Scroll => (0, scroll_script(SCROLL_KEYSTROKES)),
            Mode::Open | Mode::ImeBounds => bail!("{mode:?} places no caret for typing"),
        };
        editor.update(cx, |e, cx| e.select_range(at..at, cx));
        // ## m3c-t44: the carets under the first one, through the editor's own command.
        if mode == Mode::TypingMultiCaret {
            editor.update(cx, |e, cx| {
                for _ in 1..MULTI_CARETS {
                    e.run_command("editor.add_caret_below", window, cx);
                }
            });
            let carets = editor.read(cx).snapshot().selection.ranges().len();
            if carets != MULTI_CARETS {
                bail!("{carets} of {MULTI_CARETS} carets were placed");
            }
        }
        let expected = script.len();
        Ok((script, expected))
    })?
}

/// Sends `keys` at 60 Hz through the window's dispatcher; the keys sent to an inactive window,
/// and whether the replay stopped early.
async fn replay<C: AppContext>(
    cx: &mut C,
    ws: WindowHandle<Workspace>,
    sleep: &impl Sleep,
    keys: &[Key],
) -> (usize, bool) {
    let start = Instant::now();
    let mut inactive = 0;
    for (i, key) in keys.iter().enumerate() {
        let due = start + PERIOD * i as u32;
        if let Some(wait) = due.checked_duration_since(Instant::now()) {
            sleep(wait).await;
        }
        match send_key(cx, ws, *key) {
            Ok(true) => {}
            Ok(false) => inactive += 1,
            Err(e) => {
                eprintln!("replay aborted: {e:#}");
                return (inactive, true);
            }
        }
    }
    (inactive, false)
}

/// Sends one key through the window's dispatcher; whether the window was active.
///
/// It updates the window only, as the platform's key events do, never the root `Workspace`:
/// a dispatch draws a dirty window first, and that draw renders the root view, which must not
/// be leased then (CI run 43779216264: "cannot update Workspace while it is already being
/// updated").
pub(crate) fn send_key<C: AppContext>(
    cx: &mut C,
    ws: WindowHandle<Workspace>,
    key: Key,
) -> anyhow::Result<bool> {
    cx.update_window(ws.into(), |_, window, cx| {
        window.dispatch_keystroke(key.keystroke(), cx);
        window.is_window_active()
    })
}

// ## m3c-t44 (named lines): the dump's path, for the relex sidecar.
fn dump(latency: &LatencyProbe, run: &RunInfo) -> Option<PathBuf> {
    match latency.dump_json(&bench_dir(), std::env::consts::OS, run) {
        Ok(path) => {
            println!("wrote {}", path.display());
            Some(path)
        }
        Err(e) => {
            eprintln!("could not write bench json: {e}");
            None
        }
    }
}

/// A probe that only records the first frame the editor presents.
struct FirstPaint(Rc<Cell<Option<Instant>>>);

impl FrameProbe for FirstPaint {
    fn input(&self, _: Instant) {}

    fn painted(&self, cx: &mut App) {
        let first = self.0.clone();
        cx.defer(move |_| {
            if first.get().is_none() {
                first.set(Some(Instant::now()));
            }
        });
    }
}

/// `open`: five opens of the file in new console tabs, each timed from the file being in place
/// and its tab opening to the editor's first presented frame and to the first full analysis.
async fn open_bench<C: AppContext>(
    cx: &mut C,
    ws: WindowHandle<Workspace>,
    sleep: &impl Sleep,
    root: &BenchRoot,
    profile: ProfileId,
    text: &str,
) -> anyhow::Result<Outcome> {
    let paths = root.paths().clone();
    let mut first_paint = LatencyProbe::new("b3-open-first-paint-app");
    let mut analysis = LatencyProbe::new("b3-open-analysis-app");
    let (mut fp_ms, mut an_ms) = (Vec::new(), Vec::new());
    // Review minor 8: the window must be key throughout, counted at every look.
    let mut inactive = 0usize;
    for i in 0..OPEN_RUNS {
        let (tab, opened) = ws.update(cx, |ws, window, cx| {
            open_console(ws, profile, text, window, cx, &paths)
        })??;
        let painted: Rc<Cell<Option<Instant>>> = Rc::default();
        // Every frame until the editor exists: attach the first-paint probe before it draws.
        watch_for_editor(cx, ws, tab, painted.clone())?;
        // Both ends are watched in one loop: the analysis may land before the first paint.
        let mut analysed_at = None;
        wait_until(
            cx,
            ws,
            sleep,
            "the editor's first frame and the first full analysis",
            Duration::from_secs(30),
            |_, window, cx| {
                inactive += usize::from(!window.is_window_active());
                if analysed_at.is_none()
                    && Consoles::by_tab(cx, tab)
                        .and_then(|id| Consoles::get(cx, id)?.service.clone())
                        .is_some_and(|s| s.analyses() >= 1)
                {
                    analysed_at = Some(Instant::now());
                }
                analysed_at.is_some() && painted.get().is_some()
            },
        )
        .await?;
        let paint_at = painted.get().context("no first paint")?;
        let analysed_at = analysed_at.context("no analysis")?;
        let tier = ws.update(cx, |_, _, cx| {
            console_editor(cx, tab).map(|(_, e)| e.read(cx).status().tier)
        })?;
        let tier = tier.context("the console's editor went away")?;
        if let Err(why) = tier_ok(Mode::Open, tier) {
            println!("INVALID: {why}");
            return Ok(Outcome::Invalid);
        }
        first_paint.mark_input(opened);
        first_paint.on_frame(paint_at);
        analysis.mark_input(opened);
        analysis.on_frame(analysed_at);
        let fp = paint_at.duration_since(opened).as_secs_f64() * 1000.0;
        let an = analysed_at.duration_since(opened).as_secs_f64() * 1000.0;
        println!(
            "open {}: tier={tier:?} first paint {fp:.1} ms, full analysis {an:.1} ms",
            i + 1
        );
        fp_ms.push(fp);
        an_ms.push(an);
        sleep(Duration::from_millis(300)).await;
    }
    let mut run = ws.update(cx, |_, window, cx| stamp_run(window, cx))?;
    run.expected_samples = OPEN_RUNS;
    run.inactive_keys = inactive;
    let (_, fp50, _, _, fp_max) = percentiles(&fp_ms);
    let (_, an50, _, _, an_max) = percentiles(&an_ms);
    let fp_outcome = budget_outcome(inactive, fp_max, OPEN_FIRST_PAINT_MS);
    let an_outcome = budget_outcome(inactive, an_max, OPEN_ANALYSIS_MS);
    println!(
        "b3-open-app: tier=Full first paint p50={fp50:.1}ms max={fp_max:.1}ms (budget {OPEN_FIRST_PAINT_MS} ms: {}), full analysis p50={an50:.1}ms max={an_max:.1}ms (budget {OPEN_ANALYSIS_MS} ms: {}); runs={OPEN_RUNS} inactive_looks={inactive} file={} bytes build={} display_hz={:?} display_scale={:?}",
        fp_outcome.text(),
        an_outcome.text(),
        text.len(),
        run.build,
        run.display_hz,
        run.display_scale,
    );
    let dumped = dump(&first_paint, &run).is_some() & dump(&analysis, &run).is_some();
    if !dumped {
        return Ok(Outcome::Fail);
    }
    Ok(worse(fp_outcome, an_outcome))
}

/// A budget's outcome: INVALID when the window was not key at some look, else PASS within it.
pub fn budget_outcome(inactive: usize, value_ms: f64, budget_ms: f64) -> Outcome {
    if inactive > 0 {
        Outcome::Invalid
    } else if value_ms <= budget_ms {
        Outcome::Pass
    } else {
        Outcome::Fail
    }
}

/// The worse of two outcomes (INVALID, then FAIL, then PASS).
pub fn worse(a: Outcome, b: Outcome) -> Outcome {
    let rank = |o: Outcome| match o {
        Outcome::Invalid => 3,
        Outcome::Fail => 2,
        Outcome::NotGraded => 1,
        Outcome::Pass => 0,
    };
    if rank(a) >= rank(b) { a } else { b }
}

/// Watches every frame (before it draws) until `tab`'s editor exists, then attaches the
/// first-paint probe, so the editor's first draw is the first frame recorded.
fn watch_for_editor<C: AppContext>(
    cx: &mut C,
    ws: WindowHandle<Workspace>,
    tab: TabId,
    painted: Rc<Cell<Option<Instant>>>,
) -> anyhow::Result<()> {
    fn arm(
        window: &mut hacivat_ui_core::gpui_surface::Window,
        tab: TabId,
        painted: Rc<Cell<Option<Instant>>>,
    ) {
        window.on_next_frame(move |window, cx| {
            if let Some((_, editor)) = console_editor(cx, tab) {
                let probe: Rc<dyn FrameProbe> = Rc::new(FirstPaint(painted));
                editor.update(cx, |e, _| e.set_probe(Some(probe)));
            } else {
                arm(window, tab, painted);
            }
        });
    }
    ws.update(cx, |_, window, _| arm(window, tab, painted))
}

/// `ime-bounds`: the caret at line 49,000; 200 `bounds_for_range` questions, one per frame.
async fn ime_bounds<C: AppContext>(
    cx: &mut C,
    ws: WindowHandle<Workspace>,
    sleep: &impl Sleep,
    editor: &hacivat_ui_core::gpui_surface::Entity<EditorView>,
    tier: Tier,
) -> anyhow::Result<Outcome> {
    ws.update(cx, |_, _, cx| {
        let snap = editor.read(cx).snapshot();
        if snap.buffer.len_lines() < IME_LINE {
            bail!("the file has fewer than {IME_LINE} lines");
        }
        let at = snap.buffer.line_range(IME_LINE - 1, false).start;
        editor.update(cx, |e, cx| e.select_range(at..at, cx));
        Ok(())
    })??;
    wait_until(
        cx,
        ws,
        sleep,
        "the caret's frame",
        Duration::from_secs(5),
        |_, _, cx| editor.read(cx).caret_bounds().is_some(),
    )
    .await?;
    sleep(WARMUP).await;
    let mut probe = LatencyProbe::new(Mode::ImeBounds.label());
    let mut samples = Vec::with_capacity(IME_SAMPLES);
    // Review minor 8: counted at every question.
    let mut inactive = 0usize;
    for _ in 0..IME_SAMPLES {
        let (took, active) = ws.update(cx, |_, window, cx| {
            let active = window.is_window_active();
            let took = editor.update(cx, |e, cx| {
                let bounds = e.last_layout()?.rows().bounds;
                let snap = e.snapshot();
                let at16 = snap.buffer.byte_to_utf16(snap.selection.main().head);
                let t = Instant::now();
                let answer = e.bounds_for_range(at16..at16 + 6, bounds, window, cx);
                let took = t.elapsed();
                answer.map(|_| (t, took))
            });
            (took, active)
        })?;
        inactive += usize::from(!active);
        let (t, took) = took.context("bounds_for_range answered None at line 49,000")?;
        probe.mark_input(t);
        probe.on_frame(t + took);
        samples.push(took.as_secs_f64() * 1000.0);
        sleep(PERIOD).await;
    }
    let mut run = ws.update(cx, |_, window, cx| stamp_run(window, cx))?;
    run.expected_samples = IME_SAMPLES;
    run.inactive_keys = inactive;
    let (n, p50, p95, _, max) = percentiles(&samples);
    let outcome = budget_outcome(inactive, p95, IME_BOUNDS_MS);
    println!(
        "b3-ime-bounds-app: tier={tier:?} samples={n}/{IME_SAMPLES} p50={p50:.4}ms p95={p95:.4}ms max={max:.4}ms (budget {IME_BOUNDS_MS} ms: {}) inactive_questions={inactive} build={} display_hz={:?} display_scale={:?}",
        outcome.text(),
        run.build,
        run.display_hz,
        run.display_scale,
    );
    Ok(if dump(&probe, &run).is_some() {
        outcome
    } else {
        Outcome::Fail
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typing_script_has_2000_keystrokes_and_stays_balanced() {
        let s = typing_script(TYPING_KEYSTROKES);
        assert_eq!(s.len(), 2_000);
        let net: isize = s
            .iter()
            .map(|k| if *k == Key::Backspace { -1 } else { 1 })
            .sum();
        assert!((0..=40).contains(&net), "line grows by {net}");
    }

    #[test]
    fn warm_scripts_are_balanced() {
        for mode in [
            Mode::Typing,
            Mode::TypingLongLine,
            Mode::TypingWithResults,
            Mode::Scroll,
        ] {
            let net: isize = warm_script(mode)
                .iter()
                .map(|k| match k {
                    Key::Backspace | Key::PageUp => -1,
                    _ => 1,
                })
                .sum();
            assert_eq!(net, 0, "{mode:?}");
        }
    }

    #[test]
    fn scroll_script_comes_back_to_the_top() {
        let s = scroll_script(SCROLL_KEYSTROKES);
        assert_eq!(s.len(), 1_000);
        let net: isize = s
            .iter()
            .map(|k| if *k == Key::PageDown { 1 } else { -1 })
            .sum();
        assert_eq!(net, 0);
    }

    #[test]
    fn typing_labels_get_the_b3_threshold() {
        for mode in [Mode::Typing, Mode::TypingLongLine, Mode::TypingWithResults] {
            assert!(mode.label().starts_with("b3-typing"), "{mode:?}");
        }
        for mode in [
            "typing",
            "typing-long-line",
            "typing-with-results",
            "scroll",
            "open",
            "ime-bounds",
        ] {
            assert!(Mode::parse(mode).is_some(), "{mode}");
        }
        assert_eq!(Mode::Scroll.label(), "b3-scroll-pagedown-app");
    }

    #[test]
    fn the_full_tier_modes_refuse_reduced_mode() {
        for mode in [Mode::Typing, Mode::TypingWithResults, Mode::Open] {
            assert_eq!(tier_ok(mode, Tier::Full), Ok(()), "{mode:?}");
            let err = tier_ok(mode, Tier::Reduced).unwrap_err();
            assert!(err.contains("T0 fixture"), "{err}");
        }
        // The long-line modes measure Reduced mode, the product's truth for that file.
        for mode in [Mode::TypingLongLine, Mode::ImeBounds, Mode::Scroll] {
            assert_eq!(tier_ok(mode, Tier::Reduced), Ok(()), "{mode:?}");
            assert_eq!(tier_ok(mode, Tier::Full), Ok(()), "{mode:?}");
        }
    }

    #[test]
    fn open_is_judged_on_about_two_megabytes() {
        assert!(open_size_ok(2_095_686).is_ok(), "the T1 fixture's size");
        assert!(open_size_ok(1_990_000).is_ok());
        assert!(open_size_ok(1_499_999).is_err());
        assert!(open_size_ok(2_500_001).is_err());
        assert!(open_size_ok(20_000).is_err());
    }

    #[test]
    fn budgets_are_invalid_whenever_the_window_was_not_key() {
        assert_eq!(budget_outcome(0, 100.0, 150.0), Outcome::Pass);
        assert_eq!(budget_outcome(0, 150.1, 150.0), Outcome::Fail);
        assert_eq!(budget_outcome(1, 10.0, 150.0), Outcome::Invalid);
        assert_eq!(worse(Outcome::Pass, Outcome::Fail), Outcome::Fail);
        assert_eq!(worse(Outcome::Invalid, Outcome::Fail), Outcome::Invalid);
        assert_eq!(worse(Outcome::Pass, Outcome::Pass), Outcome::Pass);
    }

    #[test]
    fn exit_codes_follow_the_result() {
        assert_eq!(Outcome::Pass.exit_code(false), 0);
        assert_eq!(Outcome::NotGraded.exit_code(false), 0);
        assert_eq!(Outcome::Fail.exit_code(false), 1);
        assert_eq!(Outcome::Invalid.exit_code(false), EXIT_INVALID);
        // A harness check under `--allow-inactive` fails its verdict by construction.
        assert_eq!(Outcome::Fail.exit_code(true), 0);
        assert_eq!(Outcome::Invalid.exit_code(true), EXIT_INVALID);
    }

    #[test]
    fn the_result_statement_leads_the_text_only_with_results() {
        let with = console_text(Mode::TypingWithResults, "SELECT 1;\n");
        assert!(with.starts_with(RESULT_SQL));
        assert_eq!(with.lines().count(), 2);
        assert_eq!(console_text(Mode::Typing, "SELECT 1;\n"), "SELECT 1;\n");
    }

    #[test]
    fn result_cells_cycle_the_four_types() {
        assert!(matches!(result_cell(3, 0), Value::Int(_)));
        assert!(matches!(result_cell(3, 1), Value::Text(_)));
        assert!(matches!(result_cell(3, 2), Value::Decimal(_)));
        assert!(matches!(result_cell(3, 3), Value::TimestampTz { .. }));
        assert_eq!(result_columns().len(), RESULT_COLS);
    }

    #[test]
    fn parse_needs_a_mode_and_a_file() {
        let a = |s: &[&str]| s.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        assert!(parse_args(&a(&["--bench", "typing"])).is_err());
        assert!(parse_args(&a(&["--file", "f.sql"])).is_err());
        let ok = parse_args(&a(&["--file", "f.sql", "--bench", "scroll"])).unwrap();
        assert_eq!(
            (ok.mode, ok.lang.as_str(), ok.format.as_str()),
            (Mode::Scroll, "tr", "tr-TR")
        );
    }

    // ---- ## m3c-t44 -------------------------------------------------------------------------

    fn args(s: &[&str]) -> Args {
        parse_args(&s.iter().map(|x| x.to_string()).collect::<Vec<_>>()).unwrap()
    }

    #[test]
    fn the_m3c_modes_parse_and_are_graded_typing_modes() {
        let cases = [
            (
                "typing-multi-caret",
                Mode::TypingMultiCaret,
                "b3-typing-multi-caret-app",
            ),
            (
                "typing-with-folds",
                Mode::TypingWithFolds,
                "b3-typing-with-folds-app",
            ),
            ("typing-19mb", Mode::Typing19Mb, "b3-typing-19mb-app"),
        ];
        for (name, mode, label) in cases {
            assert_eq!(Mode::parse(name), Some(mode), "{name}");
            assert_eq!(mode.label(), label);
            assert!(mode.is_typing(), "{name}");
            // `hacivat-bench` keys the 16 ms p95 threshold on this prefix.
            assert!(label.starts_with("b3-typing"), "{label}");
            assert_eq!(args(&["--file", "f.sql", "--bench", name]).mode, mode);
        }
        for mode in [Mode::Scroll, Mode::Open, Mode::ImeBounds] {
            assert!(!mode.is_typing(), "{mode:?}");
        }
        assert!(Mode::parse("typing-folds").is_none());
    }

    #[test]
    fn the_m3c_modes_need_their_tier() {
        for mode in [Mode::TypingMultiCaret, Mode::TypingWithFolds] {
            assert_eq!(tier_ok(mode, Tier::Full), Ok(()), "{mode:?}");
            assert!(tier_ok(mode, Tier::Reduced).is_err(), "{mode:?}");
        }
        // 19 MiB is over the full tier's 4 MB: Reduced mode is what the product does there.
        assert_eq!(tier_ok(Mode::Typing19Mb, Tier::Reduced), Ok(()));
        assert!(tier_ok(Mode::Typing19Mb, Tier::Full).is_err());
    }

    #[test]
    fn typing_19mb_needs_19_mib_under_the_reduced_ceiling() {
        assert_eq!(LARGE_MIN_BYTES, 19_922_944);
        // Fix round 2 (M3): the restore and Reduced ceiling, both 20 MiB.
        assert_eq!(large_max_bytes(), 20_971_520);
        assert!(large_size_ok(19_922_944).is_ok());
        assert!(large_size_ok(20_765_805).is_ok(), "gen-sql --lines 520000");
        assert!(large_size_ok(20_971_519).is_ok());
        let over = large_size_ok(20_971_520).unwrap_err();
        assert!(over.contains("under 20971520"), "{over}");
        let err = large_size_ok(19_922_943).unwrap_err();
        assert!(err.contains("--lines 520000"), "{err}");
        assert!(large_size_ok(2_000_000).is_err());
        // The tier hint names the mode's own fixture.
        let tier = tier_ok(Mode::Typing19Mb, Tier::Full).unwrap_err();
        assert!(
            tier.contains("19 MiB fixture") && !tier.contains("T0"),
            "{tier}"
        );
    }

    #[test]
    fn the_fold_plan_is_a_thousand_five_line_folds_clear_of_the_typing_line() {
        let plan = fold_plan(50_000);
        assert_eq!(plan.len(), FOLDS);
        assert_eq!(plan[0], (1, 6));
        assert_eq!(plan[999], (49_951, 49_956));
        let typing = TYPING_LINE - 1;
        for w in plan.windows(2) {
            assert!(w[0].1 < w[1].0, "{w:?} overlap");
        }
        for &(header, last) in &plan {
            assert_eq!(last - header, FOLD_HIDDEN);
            assert!(!(header..=last).contains(&typing), "{header}..={last}");
        }
        // A short file gets fewer folds (the run then refuses: fewer than FOLDS).
        assert_eq!(fold_plan(1_000).len(), 20);
        assert!(fold_plan(3).is_empty());
    }

    #[test]
    fn b3_is_graded_at_118_hz_or_faster_at_any_scale() {
        // The built-in panel, the PO's M3a 120 Hz 1x display, the M3b gate's 240 Hz 1x display.
        assert_eq!(display_ok(Some(120.0), Some(2.0)), Ok(()));
        assert_eq!(display_ok(Some(118.0), Some(2.0)), Ok(()));
        assert_eq!(display_ok(Some(120.000_004_8), Some(1.0)), Ok(()));
        assert_eq!(display_ok(Some(240.0), Some(1.0)), Ok(()));
        assert_eq!(display_ok(Some(120.0), None), Ok(()));
        // A 60 Hz display, just under the floor, an unknown refresh (asleep).
        let slow = display_ok(Some(60.0), Some(2.0)).unwrap_err();
        assert!(
            slow.contains(">= 118 Hz") && slow.contains("60.0"),
            "{slow}"
        );
        assert!(display_ok(Some(117.9), Some(2.0)).is_err());
        assert!(display_ok(None, Some(2.0)).is_err());
    }

    #[test]
    fn off_the_panel_a_run_is_refused_unless_any_display() {
        let off = Err("not the panel".to_string());
        let plain = args(&["--file", "f.sql", "--bench", "typing"]);
        assert_eq!(display_decision(&plain, Ok(())), Ok(true));
        assert!(display_decision(&plain, off.clone()).is_err());
        let anywhere = args(&["--file", "f.sql", "--bench", "typing", "--any-display"]);
        assert_eq!(display_decision(&anywhere, off.clone()), Ok(false));
        // `--any-display` is ungraded even on the panel.
        assert_eq!(display_decision(&anywhere, Ok(())), Ok(false));
        // The CI harness check (xvfb) goes on ungraded.
        let harness = args(&["--file", "f.sql", "--bench", "typing", "--allow-inactive"]);
        assert_eq!(display_decision(&harness, off), Ok(false));
    }

    // Fix round 2 (I1): `--allow-inactive` is ungraded even on a qualifying display.
    #[test]
    fn allow_inactive_is_never_graded() {
        let harness = args(&["--file", "f.sql", "--bench", "typing", "--allow-inactive"]);
        let on_panel = display_decision(&harness, Ok(())).unwrap();
        assert!(!on_panel);
        assert!(!harness.graded(on_panel, "tr-TR"));
        let label = harness.label(on_panel, "tr-TR");
        assert_eq!(label, "b3-offpanel-typing-app");
        assert!(!label.starts_with("b3-typing"));
    }

    #[test]
    fn only_a_panel_run_in_tr_tr_keeps_the_graded_label() {
        let typing = args(&["--file", "f.sql", "--bench", "typing-with-folds"]);
        assert_eq!(typing.label(true, "tr-TR"), "b3-typing-with-folds-app");
        assert!(typing.graded(true, "tr-TR"));
        assert_eq!(
            typing.label(false, "tr-TR"),
            "b3-offpanel-typing-with-folds-app"
        );
        assert!(!typing.graded(false, "tr-TR"));
        // Fix round 2 (M5): the format the app applied decides, not the one asked for.
        assert_eq!(
            typing.label(true, "en-US"),
            "b3-offlocale-typing-with-folds-app"
        );
        assert!(!typing.graded(true, "en-US"));
        let en = args(&["--file", "f.sql", "--bench", "typing", "--format", "en-US"]);
        assert_eq!(en.label(true, "en-US"), "b3-offlocale-typing-app");
        assert!(!en.graded(true, "en-US"));
        for ungraded_label in [typing.label(false, "tr-TR"), en.label(true, "en-US")] {
            assert!(!ungraded_label.starts_with("b3-typing"), "{ungraded_label}");
        }
        assert_eq!(ungraded(Outcome::Pass), Outcome::NotGraded);
        assert_eq!(ungraded(Outcome::Fail), Outcome::NotGraded);
        assert_eq!(ungraded(Outcome::Invalid), Outcome::Invalid);
    }

    // Fix round 2 (M2): focus lost or the display gone mid-run cannot measure.
    #[test]
    fn a_run_that_lost_focus_or_its_display_mid_run_is_invalid() {
        let ok = || Ok(());
        let asleep = || Err("no refresh".to_string());
        assert_eq!(held_through_run(0, false, true, ok()), Ok(()));
        let lost = held_through_run(3, false, true, ok()).unwrap_err();
        assert!(
            lost.contains("3 keystrokes went to an inactive window"),
            "{lost}"
        );
        // The harness check goes on.
        assert_eq!(held_through_run(3, true, false, asleep()), Ok(()));
        let gone = held_through_run(0, false, true, asleep()).unwrap_err();
        assert!(gone.contains("at the end of the run"), "{gone}");
        // An ungraded run does not need the reference display at the end.
        assert_eq!(held_through_run(0, false, false, asleep()), Ok(()));
    }

    /// A sample whose scan and apply share `relex_us` one to three.
    fn worker(
        kind: hacivat_sql_editor::bench::Built,
        relex_us: u64,
    ) -> hacivat_sql_editor::bench::WorkerSample {
        hacivat_sql_editor::bench::WorkerSample {
            kind,
            diff: Duration::from_micros(relex_us / 4),
            apply: Duration::from_micros(relex_us - relex_us / 4),
            analysis: Duration::from_micros(relex_us * 3),
        }
    }

    #[test]
    fn r_m3_9_reads_the_incremental_analyses_only() {
        use hacivat_sql_editor::bench::Built;
        // 1,000 incremental relexes of 0.001..=1.000 ms, one fresh full lex of 50 ms, two
        // unchanged.
        let mut samples: Vec<_> = (1..=1_000).map(|i| worker(Built::Incremental, i)).collect();
        samples.push(worker(Built::Fresh, 50_000));
        samples.push(worker(Built::Unchanged, 5));
        samples.push(worker(Built::Unchanged, 5));
        let r = RelexReport::of(&samples);
        assert_eq!((r.incremental, r.unchanged, r.fresh), (1_000, 2, 1));
        assert!((r.relex.p95 - 0.95).abs() < 1e-9, "{}", r.relex.p95);
        assert!((r.relex.p99 - 0.99).abs() < 1e-9, "{}", r.relex.p99);
        assert!(
            (r.relex.max - 1.0).abs() < 1e-9,
            "the fresh build is not a relex"
        );
        // Fix round 2 (M1): the two parts are reported on their own.
        assert!((r.diff.max - 0.25).abs() < 1e-9, "{}", r.diff.max);
        assert!((r.apply.max - 0.75).abs() < 1e-9, "{}", r.apply.max);
        assert!((r.analysis.max - 3.0).abs() < 1e-9);
        assert_eq!(r.outcome(true), Outcome::Pass);
        assert_eq!(r.outcome(false), Outcome::NotGraded);
        let line = r.line("b3-typing-app", r.outcome(true));
        assert!(
            line.contains("scan n=1000") && line.contains("apply n=1000"),
            "{line}"
        );
        let json = r.json("b3-typing-app", r.outcome(true));
        assert_eq!(json["requirement"], "R-M3-9");
        assert_eq!(json["outcome"], "PASS");
        assert_eq!(json["incremental"], 1_000);
        assert_eq!(json["min_incremental"], MIN_RELEX_SAMPLES);
        assert_eq!(json["scan_ms"]["max"], 0.25);
    }

    #[test]
    fn r_m3_9_fails_over_2_ms_p95_and_needs_enough_samples() {
        use hacivat_sql_editor::bench::Built;
        assert_eq!(MIN_RELEX_SAMPLES, 500);
        // 950 of 1,000 at 1 ms and 50 at 3 ms: p95 is 1 ms (PASS); 51 at 3 ms: 3 ms (FAIL).
        let at = |slow: usize| {
            (0..1_000)
                .map(|i| {
                    worker(
                        Built::Incremental,
                        if i < 1_000 - slow { 1_000 } else { 3_000 },
                    )
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(RelexReport::of(&at(50)).outcome(true), Outcome::Pass);
        assert_eq!(RelexReport::of(&at(51)).outcome(true), Outcome::Fail);
        // Exactly 2 ms passes, with exactly the minimum of samples.
        let edge: Vec<_> = (0..MIN_RELEX_SAMPLES)
            .map(|_| worker(Built::Incremental, 2_000))
            .collect();
        assert_eq!(RelexReport::of(&edge).outcome(true), Outcome::Pass);
        // One short of the minimum, or nothing relexed: no figure supports R-M3-9.
        assert_eq!(RelexReport::of(&edge[1..]).outcome(true), Outcome::Invalid);
        let none = [worker(Built::Fresh, 10), worker(Built::Unchanged, 1)];
        assert_eq!(RelexReport::of(&none).outcome(true), Outcome::Invalid);
        assert_eq!(RelexReport::of(&[]).outcome(true), Outcome::Invalid);
    }

    #[test]
    fn a_run_that_lost_its_carets_or_folds_is_refused() {
        assert_eq!(
            kept_through_run(Mode::TypingMultiCaret, MULTI_CARETS, 0),
            Ok(())
        );
        assert!(kept_through_run(Mode::TypingMultiCaret, 1, 0).is_err());
        assert_eq!(kept_through_run(Mode::TypingWithFolds, 1, FOLDS), Ok(()));
        let err = kept_through_run(Mode::TypingWithFolds, 1, 999).unwrap_err();
        assert!(err.contains("999 of 1000 folds"), "{err}");
        // Other modes keep nothing in particular.
        assert_eq!(kept_through_run(Mode::Typing, 1, 0), Ok(()));
    }
}
