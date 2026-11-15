//! B1-real (Task 36, R-M3-12): the M0 fling over a real 1M-row × 50-column result in the
//! results grid of the real app, with the `tr-TR` format locale.
//!
//! - **Server.** `--engine pg` (default): the Docker matrix's `pg17` (the example resolves it
//!   with `hacivat_testkit::require_target`, so it runs with `HACIVAT_DOCKER_SET=pr`), through
//!   the real PostgreSQL driver. `--engine sqlite`: the same shape from a recursive CTE over a
//!   temp-file database (the CI regression signal; no server). Nothing is created on the server:
//!   the rows come from `generate_series`.
//! - **Dataset.** `SELECT … FROM generate_series(1, 1000000)` with 50 columns cycling int8, text
//!   (md5 mixed with a 16-word Turkish array), numeric(14,2) and timestamptz.
//! - **Layout.** A `workspace.json` written before boot: explorer hidden, the editor tile at
//!   `MIN_TILE_PX`; the window at `--size` (1440 × 900 by default; the gate's second run is
//!   2560 × 1440 on a 1× display). No inspector exists in M3a.
//! - **Load.** Run, then **Fetch all** and **Fetch more anyway** (`ConsoleResults::raise_cap`)
//!   until `Complete` (ledger T36 note; R-M3-19). Load time (Run → Complete), first rows, the
//!   store's bytes and the process RSS are printed and kept in `target/bench/load/`.
//! - **Fling.** The M0 fling (V0 = 12,000 px/s, τ = 0.325 s, refling every 1.5 s, 20 s after a
//!   750 ms warm-up that jumps to the middle row) recorded with `FrameRecorder`; the verdict is
//!   `hacivat_ui_core::frames::verdict` (≥ 55 fps on macOS and Windows, ≥ 45 on Linux, no gap
//!   over 100 ms, the grid painted on ≥ 95 % of frames, no step to an inactive window). The
//!   label starts with `b1-fling` so that verdict applies (`hacivat-bench` keys its threshold on
//!   that prefix).
//! - **Paint.** The window's CPU per frame ([`super::FrameCpu`]) is reported against 6 ms and
//!   3 ms, not gating: an upper bound on the grid's paint (the grid's own paint timer is behind
//!   `hacivat-ui-grid/bench`, which the app's `bench` feature does not enable).
//! - `--mode streaming`: a third, non-gating run that starts the fling once 10,000 rows are in
//!   while the rest still streams (label `b1-streaming-real-*`, no threshold).

use std::cell::{Cell, RefCell};
use std::process::ExitCode;
use std::rc::Rc;
use std::time::{Duration, Instant};

use anyhow::{Context as _, bail};
use hacivat_proto::{DbmsKind, ProfileId};
use hacivat_secrets::AuthProviderId;
use hacivat_secrets::Retention;
use hacivat_session::profile::{Profile, ProfileEndpoint};
use hacivat_ui_core::app::{Boot, run_app};
use hacivat_ui_core::frame_loop::{bench_dir, record_frames, stamp_run};
use hacivat_ui_core::frames::{FrameRecorder, RunInfo, b1_expected_frames, verdict};
use hacivat_ui_core::gpui_surface::{AppContext, Entity, Window, WindowHandle, px, size};
use hacivat_ui_grid::GridView;
use secrecy::SecretString;

use super::{
    BenchRoot, EXIT_USAGE, FrameCpu, Launch, Sleep, activate, connect, connected, console_editor,
    console_ready, flag, format_locale, open_console, percentiles, result_grid, save_layout,
    save_profile, start_app, verdict_text, wait_until,
};
use crate::console::ConsoleId;
use crate::shell::TileLayout;
use crate::shell::tiles::MIN_TILE_PX;
use crate::shell::workspace::Workspace;

pub const ROWS: u64 = 1_000_000;
pub const COLS: usize = 50;
/// Initial fling velocity, px/s.
const V0: f64 = 12_000.0;
/// Exponential decay time constant, s.
const TAU: f64 = 0.325;
/// A new fling starts every `REFLING_S` seconds.
const REFLING_S: f64 = 1.5;
/// Recorded fling time, s.
const DURATION_S: f64 = 20.0;
/// Idle time before recording (first shapes, atlas warm-up).
const WARMUP: Duration = Duration::from_millis(750);
/// The streaming run starts its fling once this many rows are in.
const STREAMING_START_ROWS: u64 = 10_000;
const LOAD_TIMEOUT: Duration = Duration::from_secs(900);
/// The paint budgets reported (not gating): 120 Hz half-frame and the 6 ms ALG budget.
const PAINT_BUDGETS_MS: [f64; 2] = [6.0, 3.0];
/// The 16-word Turkish array the text columns mix in.
const WORDS: [&str; 16] = [
    "çiçek",
    "ağaç",
    "Iğdır",
    "İzmir",
    "şehir",
    "öğrenci",
    "gümüş",
    "ılık",
    "ışık",
    "üzüm",
    "kuşkonmaz",
    "Çanakkale",
    "Şırnak",
    "dağ",
    "göğüs",
    "yağmur",
];

/// The server a `pg` run connects to (the example fills it from `require_target("pg17")`).
pub struct ServerTarget {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub password: SecretString,
    pub database: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Engine {
    Pg,
    Sqlite,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Fetch everything, then fling (the gate's run).
    Full,
    /// Fling from the first 10,000 rows while the rest streams (not gating).
    Streaming,
}

/// `b1_real [--engine pg|sqlite] [--size WxH] [--mode full|streaming] [--lang tr]
/// [--format tr-TR]`.
#[derive(Debug, Clone)]
pub struct Args {
    pub engine: Engine,
    pub size: (f32, f32),
    pub mode: Mode,
    pub lang: String,
    pub format: String,
    /// Go on when the window cannot become frontmost (a harness check; the verdict fails).
    pub allow_inactive: bool,
}

pub const USAGE: &str = "usage: HACIVAT_DOCKER_SET=pr b1_real [--engine pg|sqlite] [--size 1440x900] [--mode full|streaming] [--lang tr] [--format tr-TR] [--allow-inactive]";

pub fn parse_args(args: &[String]) -> Result<Args, String> {
    let engine = match flag(args, "--engine").unwrap_or("pg") {
        "pg" => Engine::Pg,
        "sqlite" => Engine::Sqlite,
        other => return Err(format!("unknown engine {other:?}")),
    };
    let size = match flag(args, "--size") {
        None => (1440.0, 900.0),
        Some(s) => {
            let (w, h) = s.split_once('x').ok_or(format!("bad size {s:?}"))?;
            (
                w.parse().map_err(|_| format!("bad width {w:?}"))?,
                h.parse().map_err(|_| format!("bad height {h:?}"))?,
            )
        }
    };
    let mode = match flag(args, "--mode").unwrap_or("full") {
        "full" => Mode::Full,
        "streaming" => Mode::Streaming,
        other => return Err(format!("unknown mode {other:?}")),
    };
    Ok(Args {
        engine,
        size,
        mode,
        lang: flag(args, "--lang").unwrap_or("tr").to_string(),
        format: flag(args, "--format").unwrap_or("tr-TR").to_string(),
        allow_inactive: args.iter().any(|a| a == "--allow-inactive"),
    })
}

impl Args {
    /// The JSON label: `b1-fling-real-<engine>-<w>x<h>` (graded), or `b1-streaming-real-…`.
    pub fn label(&self) -> String {
        let engine = match self.engine {
            Engine::Pg => "pg",
            Engine::Sqlite => "sqlite",
        };
        let kind = match self.mode {
            Mode::Full => "b1-fling-real",
            Mode::Streaming => "b1-streaming-real",
        };
        format!("{kind}-{engine}-{}x{}", self.size.0, self.size.1)
    }
}

/// The 50-column PostgreSQL dataset over `generate_series(1, rows)`.
pub fn pg_sql(rows: u64, cols: usize) -> String {
    let words = WORDS
        .iter()
        .map(|w| format!("'{w}'"))
        .collect::<Vec<_>>()
        .join(",");
    let columns: Vec<String> = (0..cols)
        .map(|c| {
            let k = c + 1;
            match c % 4 {
                0 => format!("(g * {k})::int8 AS c{c:02}"),
                1 => format!(
                    "(ARRAY[{words}])[1 + (g + {c}) % 16] || ' ' || md5((g + {c})::text) AS c{c:02}"
                ),
                2 => format!("((g * {k}) % 1000000000000 / 100.0)::numeric(14,2) AS c{c:02}"),
                _ => format!(
                    "timestamptz '2024-01-01 00:00:00+00' + (g * {k}) * interval '1 second' AS c{c:02}"
                ),
            }
        })
        .collect();
    format!(
        "SELECT {} FROM generate_series(1, {rows}) AS g",
        columns.join(", ")
    )
}

/// The same shape on SQLite: a recursive CTE (no table), text from a JSON array of the words
/// and a hex hash-like string, decimals rounded to two places, ISO timestamps.
pub fn sqlite_sql(rows: u64, cols: usize) -> String {
    let words = WORDS
        .iter()
        .map(|w| format!("\"{w}\""))
        .collect::<Vec<_>>()
        .join(",");
    let columns: Vec<String> = (0..cols)
        .map(|c| {
            let k = c + 1;
            match c % 4 {
                0 => format!("g * {k} AS c{c:02}"),
                1 => format!(
                    "json_extract('[{words}]', '$[' || ((g + {c}) % 16) || ']') || ' ' || printf('%08x%08x', (g * 2654435761 + {c}) % 4294967296, (g * {k} * 40503) % 4294967296) AS c{c:02}"
                ),
                2 => format!("round(((g * {k}) % 1000000000000) / 100.0, 2) AS c{c:02}"),
                _ => format!(
                    "strftime('%Y-%m-%dT%H:%M:%SZ', 1704067200 + g * {k}, 'unixepoch') AS c{c:02}"
                ),
            }
        })
        .collect();
    format!(
        "WITH RECURSIVE s(g) AS (SELECT 1 UNION ALL SELECT g + 1 FROM s WHERE g < {rows}) SELECT {} FROM s",
        columns.join(", ")
    )
}

/// Distance scrolled `t` seconds after the first fling started, in px (the M0 fling).
pub fn fling_distance(t: f64) -> f64 {
    let t = t.max(0.0);
    let flings = (t / REFLING_S).floor();
    let local = t - flings * REFLING_S;
    let per_fling = V0 * TAU * (1.0 - (-REFLING_S / TAU).exp());
    flings * per_fling + V0 * TAU * (1.0 - (-local / TAU).exp())
}

pub(crate) fn profile(engine: Engine, target: Option<&ServerTarget>, root: &BenchRoot) -> Profile {
    match (engine, target) {
        (Engine::Pg, Some(t)) => {
            let mut p = Profile {
                name: "b1-real-pg17".to_string(),
                engine: DbmsKind::Postgres,
                endpoint: ProfileEndpoint::Server {
                    host: t.host.clone(),
                    port: Some(t.port),
                },
                database: Some(t.database.clone()),
                user: Some(t.user.clone()),
                auth: AuthProviderId::Password,
                retention: Retention::Never,
                ..Profile::default()
            };
            p.tls.mode = hacivat_net::tls::TlsMode::Disable;
            p
        }
        _ => Profile {
            name: "b1-real-sqlite".to_string(),
            engine: DbmsKind::Sqlite,
            endpoint: ProfileEndpoint::File {
                path: root.dir().join("b1-real.sqlite"),
                create_if_missing: true,
            },
            user: None,
            auth: AuthProviderId::None,
            max_connections: 1,
            ..Profile::default()
        },
    }
}

/// Runs the bench; the process exits with its code. `target` is required for `--engine pg`.
pub fn run(args: Args, target: Option<ServerTarget>) -> ExitCode {
    if args.engine == Engine::Pg && target.is_none() {
        eprintln!("b1_real: --engine pg needs the pg17 target; {USAGE}");
        return ExitCode::from(EXIT_USAGE);
    }
    let root = match BenchRoot::new("b1-real") {
        Ok(r) => r,
        Err(e) => {
            eprintln!("b1_real: {e:#}");
            return ExitCode::FAILURE;
        }
    };
    let profile = profile(args.engine, target.as_ref(), &root);
    let profile_id = profile.id;
    let tiles = TileLayout {
        editor_h: MIN_TILE_PX,
        explorer_visible: false,
        ..TileLayout::default()
    };
    if let Err(e) = save_profile(&root, &[profile]).and_then(|()| save_layout(&root, tiles)) {
        eprintln!("b1_real: {e:#}");
        root.remove();
        return ExitCode::FAILURE;
    }
    let password = target.map(|t| t.password);
    let launch = Launch {
        root: root.clone(),
        lang: args.lang.clone(),
        format: args.format.clone(),
        zone: "Europe/Istanbul".to_string(),
        drivers: hacivat_session::default_drivers(),
    };
    run_app(Boot::default(), move |cx| {
        let ws = match start_app(cx, launch) {
            Ok(ws) => ws,
            Err(e) => {
                eprintln!("b1_real: opening the workspace failed: {e:#}");
                root.remove();
                std::process::exit(1);
            }
        };
        let exec = cx.background_executor().clone();
        cx.spawn(async move |cx| {
            let sleep = move |d: Duration| exec.timer(d);
            let code = match script(cx, ws, &sleep, &args, &root, profile_id, password, tiles).await
            {
                Ok(code) => code,
                Err(e) => {
                    eprintln!("b1_real aborted: {e:#}");
                    1
                }
            };
            root.remove();
            if code != 0 {
                std::process::exit(code);
            }
            cx.update(|cx| cx.quit());
        })
        .detach();
    });
    ExitCode::SUCCESS
}

#[allow(clippy::too_many_arguments)]
async fn script<C: AppContext>(
    cx: &mut C,
    ws: WindowHandle<Workspace>,
    sleep: &impl Sleep,
    args: &Args,
    root: &BenchRoot,
    profile: ProfileId,
    password: Option<SecretString>,
    tiles: TileLayout,
) -> anyhow::Result<i32> {
    // The run's conditions: the layout restored, the format locale, the window's size.
    let (restored, locale) = ws.update(cx, |ws, window, cx| {
        window.resize(size(px(args.size.0), px(args.size.1)));
        (ws.layout().1, format_locale(cx))
    })?;
    if restored.explorer_visible || restored.editor_h != tiles.editor_h {
        bail!("the bench layout was not restored: {restored:?}");
    }
    let Some((format, zone)) = locale else {
        bail!("no localization");
    };
    if format != args.format {
        bail!("the format locale is {format}, not {}", args.format);
    }
    activate(cx, ws, sleep, args.allow_inactive).await?;
    let sql = match args.engine {
        Engine::Pg => pg_sql(ROWS, COLS),
        Engine::Sqlite => sqlite_sql(ROWS, COLS),
    };
    let paths = root.paths().clone();
    let (tab, _) = ws.update(cx, |ws, window, cx| {
        open_console(ws, profile, &sql, window, cx, &paths)
    })??;
    wait_until(
        cx,
        ws,
        sleep,
        "the console's editor",
        Duration::from_secs(30),
        |_, _, cx| console_editor(cx, tab).is_some(),
    )
    .await?;
    let (console, editor) = ws
        .update(cx, |_, _, cx| console_editor(cx, tab))?
        .context("the console's editor went away")?;
    ws.update(cx, |_, _, cx| connect(cx, profile, password))?;
    wait_until(
        cx,
        ws,
        sleep,
        "the connection",
        Duration::from_secs(60),
        |_, _, cx| connected(cx, profile),
    )
    .await?;
    wait_until(
        cx,
        ws,
        sleep,
        "the console's SQL context",
        Duration::from_secs(60),
        |_, _, cx| console_ready(cx, console),
    )
    .await?;
    let rss_before = super::rss_bytes();
    let ran = super::start_run(cx, ws, &editor, 0..sql.len())?;
    let mut fetching = super::Fetching::default();
    let streaming = Rc::new(Cell::new(None::<FlingOutcome>));
    if args.mode == Mode::Streaming {
        wait_until(
            cx,
            ws,
            sleep,
            "the first 10,000 rows",
            LOAD_TIMEOUT,
            |_, _, cx| {
                fetching.step(cx, console);
                fetching.failed.is_some() || fetching.rows >= STREAMING_START_ROWS
            },
        )
        .await?;
        let grid = wait_grid(cx, ws, sleep, console).await?;
        start_fling(cx, ws, grid, args.label(), streaming.clone())?;
    }
    wait_until(
        cx,
        ws,
        sleep,
        "the result to complete",
        LOAD_TIMEOUT,
        |_, _, cx| fetching.step(cx, console),
    )
    .await?;
    let loaded = super::finish_load(cx, ws, console, ran, fetching)?;
    let rss_after = super::rss_bytes();
    let complete = loaded.rows == ROWS;
    println!(
        "{} load: rows={} (complete={complete}) columns={COLS} load={:.2}s first_rows={:.0}ms store={} rss_before={} rss_after={} fetch_all_asked={} cap_raised={} format={format} zone={zone}",
        args.label(),
        loaded.rows,
        loaded.complete.as_secs_f64(),
        loaded.first_rows.as_secs_f64() * 1000.0,
        super::mib(loaded.bytes),
        rss_before.map_or("unknown".into(), super::mib),
        rss_after.map_or("unknown".into(), super::mib),
        loaded.fetch_alls,
        loaded.raised,
    );
    if !complete {
        bail!("the result has {} rows, not {ROWS}", loaded.rows);
    }
    let outcome = match args.mode {
        Mode::Streaming => {
            wait_until(
                cx,
                ws,
                sleep,
                "the fling",
                Duration::from_secs(60),
                |_, _, _| {
                    // Peek without taking.
                    let o = streaming.take();
                    let done = o.is_some();
                    streaming.set(o);
                    done
                },
            )
            .await?;
            streaming.take().context("no fling outcome")?
        }
        Mode::Full => {
            let grid = wait_grid(cx, ws, sleep, console).await?;
            let done = Rc::new(Cell::new(None::<FlingOutcome>));
            start_fling(cx, ws, grid, args.label(), done.clone())?;
            wait_until(
                cx,
                ws,
                sleep,
                "the fling",
                Duration::from_secs(60),
                |_, _, _| {
                    let o = done.take();
                    let ok = o.is_some();
                    done.set(o);
                    ok
                },
            )
            .await?;
            done.take().context("no fling outcome")?
        }
    };
    report(cx, ws, args, &outcome, &loaded, rss_after, &format)
}

async fn wait_grid<C: AppContext>(
    cx: &mut C,
    ws: WindowHandle<Workspace>,
    sleep: &impl Sleep,
    console: ConsoleId,
) -> anyhow::Result<Entity<GridView>> {
    wait_until(
        cx,
        ws,
        sleep,
        "the result's grid",
        Duration::from_secs(30),
        |_, _, cx| result_grid(cx, console).is_some(),
    )
    .await?;
    ws.update(cx, |_, _, cx| result_grid(cx, console))?
        .context("the grid went away")
}

/// What the fling recorded.
struct FlingOutcome {
    recorder: Rc<RefCell<FrameRecorder>>,
    inactive: usize,
    painted: u64,
    fast_frames: u64,
    max_shapes: u32,
    cpu: Vec<f64>,
    final_top: u64,
}

/// Starts the warm-up and the fling over `grid` on the window's frames; `done` receives the
/// outcome.
fn start_fling<C: AppContext>(
    cx: &mut C,
    ws: WindowHandle<Workspace>,
    grid: Entity<GridView>,
    label: String,
    done: Rc<Cell<Option<FlingOutcome>>>,
) -> anyhow::Result<()> {
    ws.update(cx, move |_, window, _| {
        warm_up(grid, window, Instant::now(), false, label, done);
    })
}

fn warm_up(
    grid: Entity<GridView>,
    window: &mut Window,
    opened: Instant,
    jumped: bool,
    label: String,
    done: Rc<Cell<Option<FlingOutcome>>>,
) {
    window.on_next_frame(move |window, cx| {
        if !jumped {
            // The middle of what is loaded (all of it for the full run).
            let loaded = grid.read(cx).scroll_state().row_count();
            grid.update(cx, |g, cx| g.scroll_to_row(loaded / 2, cx));
        }
        if opened.elapsed() < WARMUP {
            warm_up(grid, window, opened, true, label, done);
            return;
        }
        let recorder = Rc::new(RefCell::new(FrameRecorder::new(&label)));
        let painted_before = grid.update(cx, |g, _| {
            g.reset_frame_log();
            g.frame_log().frames
        });
        record_frames(window, recorder.clone());
        let fling = Fling {
            grid,
            recorder,
            inactive: Rc::default(),
            cpu: FrameCpu::default(),
            started: Instant::now(),
            painted_before,
            done,
        };
        drive(fling, window, 0.0);
    });
}

struct Fling {
    grid: Entity<GridView>,
    recorder: Rc<RefCell<FrameRecorder>>,
    inactive: Rc<Cell<usize>>,
    cpu: FrameCpu,
    started: Instant,
    painted_before: u64,
    done: Rc<Cell<Option<FlingOutcome>>>,
}

fn drive(f: Fling, window: &mut Window, travelled: f64) {
    window.on_next_frame(move |window, cx| {
        f.cpu.frame_started(cx);
        let t = f.started.elapsed().as_secs_f64();
        if t >= DURATION_S {
            f.recorder.borrow_mut().stop();
            let (log, top) = f
                .grid
                .read_with(cx, |g, _| (*g.frame_log(), g.scroll_state().top_row));
            f.done.set(Some(FlingOutcome {
                recorder: f.recorder.clone(),
                inactive: f.inactive.get(),
                painted: log.frames.saturating_sub(f.painted_before),
                fast_frames: log.fast_frames,
                max_shapes: log.max_shapes,
                cpu: f.cpu.samples(),
                final_top: top,
            }));
            return;
        }
        if !window.is_window_active() {
            f.inactive.set(f.inactive.get() + 1);
        }
        let d = fling_distance(t);
        f.grid
            .update(cx, |g, cx| g.scroll_px(d - travelled, 0.0, cx));
        drive(f, window, d);
    });
}

fn report<C: AppContext>(
    cx: &mut C,
    ws: WindowHandle<Workspace>,
    args: &Args,
    o: &FlingOutcome,
    loaded: &super::Loaded,
    rss: Option<u64>,
    format: &str,
) -> anyhow::Result<i32> {
    let label = args.label();
    let os = std::env::consts::OS;
    let (mut run, viewport) = ws.update(cx, |_, window, cx| {
        (stamp_run(window, cx), window.viewport_size())
    })?;
    run.expected_samples = b1_expected_frames(os, DURATION_S);
    run.inactive_keys = o.inactive;
    run.painted_frames = Some(usize::try_from(o.painted).unwrap_or(usize::MAX));
    let rec = o.recorder.borrow();
    let stats = rec.stats();
    let v = verdict(&label, os, &stats, &run);
    println!("{label} FrameStats: {stats:?}");
    let (n, p50, p95, p99, max) = percentiles(&o.cpu);
    let over = |ms: f64| o.cpu.iter().filter(|v| **v > ms).count();
    println!(
        "{label} window frame CPU (frame callback -> next main-thread turn; an upper bound on the grid's paint): n={n} p50={p50:.2}ms p95={p95:.2}ms p99={p99:.2}ms max={max:.2}ms over-{}ms={} over-{}ms={}",
        PAINT_BUDGETS_MS[0],
        over(PAINT_BUDGETS_MS[0]),
        PAINT_BUDGETS_MS[1],
        over(PAINT_BUDGETS_MS[1]),
    );
    println!(
        "{label} grid frames painted={} fast-scroll frames={} max shapes/frame={} final top row={}",
        o.painted, o.fast_frames, o.max_shapes, o.final_top
    );
    println!(
        "{label} verdict={} frames={}/{} inactive_steps={} build={} display_hz={:?} display_scale={:?} window={}x{} format={format} rows={} load={:.2}s store={} rss={}",
        verdict_text(v),
        stats.frames,
        run.expected_samples,
        o.inactive,
        run.build,
        run.display_hz,
        run.display_scale,
        f32::from(viewport.width),
        f32::from(viewport.height),
        loaded.rows,
        loaded.complete.as_secs_f64(),
        super::mib(loaded.bytes),
        rss.map_or("unknown".into(), super::mib),
    );
    if o.inactive > 0 {
        eprintln!(
            "warning: {} fling steps ran while the window was inactive; GPUI throttles inactive windows, so this run is not representative",
            o.inactive
        );
    }
    match rec.dump_json(&bench_dir(), os, &run) {
        Ok(path) => {
            println!("{label} wrote {}", path.display());
            write_sidecar(&path, args, o, loaded, rss, format, &run);
            Ok(0)
        }
        Err(e) => {
            eprintln!("{label}: failed to write JSON: {e}");
            Ok(1)
        }
    }
}

/// The load and paint figures of a run, for the gate's report: `target/bench/load/<the frames
/// dump's name>`, outside `target/bench/*.json` because `bench-report` reads every file it is
/// given as a frames or latency dump.
fn write_sidecar(
    dump: &std::path::Path,
    args: &Args,
    o: &FlingOutcome,
    loaded: &super::Loaded,
    rss: Option<u64>,
    format: &str,
    run: &RunInfo,
) {
    let (_, p50, p95, p99, max) = percentiles(&o.cpu);
    let body = serde_json::json!({
        "label": args.label(),
        "format_locale": format,
        "window": [args.size.0, args.size.1],
        "display_hz": run.display_hz,
        "display_scale": run.display_scale,
        "build": run.build,
        "rows": loaded.rows,
        "columns": COLS,
        "load_ms": loaded.complete.as_millis() as u64,
        "first_rows_ms": loaded.first_rows.as_millis() as u64,
        "store_bytes": loaded.bytes,
        "rss_bytes": rss,
        "cap_raised": loaded.raised,
        "frame_cpu_ms": {"p50": p50, "p95": p95, "p99": p99, "max": max},
        "grid_painted_frames": o.painted,
        "fast_scroll_frames": o.fast_frames,
    });
    let dir = bench_dir().join("load");
    let path = dir.join(dump.file_name().unwrap_or_default());
    match serde_json::to_vec_pretty(&body)
        .map_err(std::io::Error::other)
        .and_then(|b| std::fs::create_dir_all(&dir).and_then(|()| std::fs::write(&path, b)))
    {
        Ok(()) => println!("{} wrote {}", args.label(), path.display()),
        Err(e) => eprintln!("could not write {}: {e}", path.display()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_fling_starts_at_v0_and_decays() {
        let dt = 1e-6;
        let v = fling_distance(dt) / dt;
        assert!((v - V0).abs() / V0 < 1e-3, "{v}");
        let total = fling_distance(DURATION_S);
        assert!((50_000.0..55_000.0).contains(&total), "{total}");
    }

    #[test]
    fn the_datasets_have_fifty_columns_cycling_four_types() {
        let pg = pg_sql(ROWS, COLS);
        assert_eq!(pg.matches(" AS c").count(), COLS);
        assert_eq!(pg.matches("::int8").count(), 13);
        assert_eq!(pg.matches("md5(").count(), 13);
        assert_eq!(pg.matches("::numeric(14,2)").count(), 12);
        assert_eq!(pg.matches("timestamptz '").count(), 12);
        assert!(pg.contains("generate_series(1, 1000000)"));
        assert!(pg.contains("'Iğdır'") && pg.contains("'İzmir'"));
        let lite = sqlite_sql(ROWS, COLS);
        assert_eq!(lite.matches(" AS c").count(), COLS);
        assert!(lite.contains("WHERE g < 1000000"));
    }

    #[test]
    fn labels_keep_the_graded_prefix_only_for_the_full_run() {
        let a = |s: &[&str]| s.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        let full = parse_args(&a(&[])).unwrap();
        assert_eq!(full.label(), "b1-fling-real-pg-1440x900");
        assert_eq!((full.lang.as_str(), full.format.as_str()), ("tr", "tr-TR"));
        let wide = parse_args(&a(&["--size", "2560x1440", "--engine", "sqlite"])).unwrap();
        assert_eq!(wide.label(), "b1-fling-real-sqlite-2560x1440");
        let streaming = parse_args(&a(&["--mode", "streaming"])).unwrap();
        assert!(!streaming.label().starts_with("b1-fling"));
        assert!(parse_args(&a(&["--engine", "oracle"])).is_err());
        assert!(parse_args(&a(&["--size", "big"])).is_err());
    }
}
