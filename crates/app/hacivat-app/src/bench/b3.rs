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
    BenchRoot, EXIT_INVALID, EXIT_USAGE, Launch, Outcome, PresentProbe, Sleep, activate, connect,
    connected, console_editor, console_ready, end_run, fake_registry, flag, open_console,
    percentiles, print_result, save_profile, start_app, verdict_text, wait_until,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Typing,
    TypingLongLine,
    TypingWithResults,
    Scroll,
    Open,
    ImeBounds,
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
            _ => return None,
        })
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
        }
    }

    /// The tier the mode measures, when it must be that one (review I4): the 50k-line file
    /// without a long line opens in the full tier.
    pub fn required_tier(self) -> Option<Tier> {
        match self {
            Mode::Typing | Mode::TypingWithResults | Mode::Open => Some(Tier::Full),
            Mode::TypingLongLine | Mode::Scroll | Mode::ImeBounds => None,
        }
    }
}

/// The size `open` is judged at ("open 2 MB", E3).
const OPEN_BYTES: std::ops::RangeInclusive<usize> = 1_500_000..=2_500_000;

/// Whether `tier` is the one `mode` measures; the reason when it is not.
pub fn tier_ok(mode: Mode, tier: Tier) -> Result<(), String> {
    match mode.required_tier() {
        Some(want) if want != tier => Err(format!(
            "{} measures the {want:?} tier but the editor opened the file in {tier:?}; use the T0 fixture (gen-sql --lines 50000, no --long-line)",
            mode.label()
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
}

pub const USAGE: &str = "usage: b3 --file <fixture> --bench typing|typing-long-line|typing-with-results|scroll|open|ime-bounds [--lang tr] [--format tr-TR] [--allow-inactive]\n\
  typing, typing-with-results, open, scroll: --file target/fixtures/b3-50k-t0.sql (gen-sql --lines 50000)\n\
  typing-long-line, ime-bounds: --file target/fixtures/b3-50k.sql (gen-sql --lines 50000 --long-line 100000)\n\
  The last line is `RESULT: PASS|FAIL|INVALID|n/a <label>`; exit 0 = PASS or ungraded, 1 = FAIL or aborted, 3 = INVALID (exit 0 is never a PASS by itself: read RESULT).";

pub fn parse_args(args: &[String]) -> Result<Args, String> {
    let mode = flag(args, "--bench").ok_or("missing --bench <mode>")?;
    let mode = Mode::parse(mode).ok_or_else(|| format!("unknown mode {mode:?}"))?;
    let file = PathBuf::from(flag(args, "--file").ok_or("missing --file <path>")?);
    Ok(Args {
        mode,
        file,
        lang: flag(args, "--lang").unwrap_or("tr").to_string(),
        format: flag(args, "--format").unwrap_or("tr-TR").to_string(),
        allow_inactive: args.iter().any(|a| a == "--allow-inactive"),
    })
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
    if args.mode == Mode::Open
        && let Err(why) = open_size_ok(fixture.len())
    {
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
                Ok(outcome) => {
                    print_result(outcome, args.mode.label());
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
) -> anyhow::Result<Outcome> {
    activate(cx, ws, sleep, args.allow_inactive).await?;
    print_display(cx, ws, args.mode.label())?;
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
    let probe = PresentProbe::new(args.mode.label());
    ws.update(cx, |_, _, cx| {
        editor.update(cx, |e, _| e.set_probe(Some(probe.as_frame_probe())));
    })?;
    // Warm-up: the input-modality redraw and the first shapes, unmeasured; it leaves the text
    // and the caret where they were.
    let warm = warm_script(args.mode);
    replay(cx, ws, sleep, &warm).await;
    sleep(WARMUP).await;
    let shapes_before = editor_shapes(cx, ws, &editor)?;
    probe.arm(true);
    let (inactive, aborted) = replay(cx, ws, sleep, &script).await;
    sleep(SETTLE).await;
    probe.arm(false);
    let shapes = editor_shapes(cx, ws, &editor)?.saturating_sub(shapes_before);
    let (inputs, frames) = probe.counts();
    let mut run = ws.update(cx, |_, window, cx| stamp_run(window, cx))?;
    run.expected_samples = expected;
    run.inactive_keys = inactive;
    run.aborted = aborted;
    let latency = probe.latency();
    let stats = latency.stats();
    let os = std::env::consts::OS;
    let v = verdict(args.mode.label(), os, &stats, &run);
    if args.mode == Mode::Scroll {
        println!(
            "{}: keyboard paging (PageDown/PageUp), not the wheel",
            args.mode.label()
        );
    }
    println!(
        "{}: tier={tier:?} samples={}/{} p50={:.2}ms p95={:.2}ms max={:.2}ms verdict={} inputs_marked={inputs} frames_showing_input={frames} inactive_keys={} aborted={} lines_shaped={shapes} build={} display_hz={:?} display_scale={:?}",
        args.mode.label(),
        stats.samples,
        run.expected_samples,
        stats.p50_ms,
        stats.p95_ms,
        stats.max_ms,
        verdict_text(v),
        run.inactive_keys,
        run.aborted,
        run.build,
        run.display_hz,
        run.display_scale,
    );
    if inactive > 0 {
        eprintln!(
            "warning: {inactive} keystrokes went to an inactive window; GPUI throttles inactive windows, so this run is not representative"
        );
    }
    let dumped = dump(&latency, &run);
    Ok(if run.aborted || !dumped {
        Outcome::Fail
    } else {
        Outcome::of(v)
    })
}

/// Prints the window's viewport, refresh and scale; warns (does not refuse) off the 120 Hz
/// panel (review C1.6). The JSON records the display either way.
fn print_display<C: AppContext>(
    cx: &mut C,
    ws: WindowHandle<Workspace>,
    label: &str,
) -> anyhow::Result<()> {
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
    if run.display_hz.is_none_or(|hz| hz < 118.0) {
        eprintln!(
            "warning: the display is not the 120 Hz panel ({:?} Hz); the gate's B3 runs on it",
            run.display_hz
        );
    }
    Ok(())
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

/// Places the caret for `mode`; the measured script and its expected samples.
fn place_caret<C: AppContext>(
    cx: &mut C,
    ws: WindowHandle<Workspace>,
    mode: Mode,
    editor: &hacivat_ui_core::gpui_surface::Entity<EditorView>,
) -> anyhow::Result<(Vec<Key>, usize)> {
    ws.update(cx, |_, _, cx| {
        let snap = editor.read(cx).snapshot();
        let buffer = &snap.buffer;
        let (at, script) = match mode {
            Mode::Typing | Mode::TypingWithResults => {
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
            Mode::Scroll => (0, scroll_script(SCROLL_KEYSTROKES)),
            Mode::Open | Mode::ImeBounds => bail!("{mode:?} places no caret for typing"),
        };
        editor.update(cx, |e, cx| e.select_range(at..at, cx));
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

fn dump(latency: &LatencyProbe, run: &RunInfo) -> bool {
    match latency.dump_json(&bench_dir(), std::env::consts::OS, run) {
        Ok(path) => {
            println!("wrote {}", path.display());
            true
        }
        Err(e) => {
            eprintln!("could not write bench json: {e}");
            false
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
    let dumped = dump(&first_paint, &run) & dump(&analysis, &run);
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
    Ok(if dump(&probe, &run) {
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
}
