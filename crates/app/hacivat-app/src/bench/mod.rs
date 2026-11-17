//! The real-app benches (Task 36; feature `bench`, R-M3-40): B3 typing in a console's editor
//! ([`b3`]) and B1 over a real result in the results grid ([`b1_real`]). Both run the whole app
//! (`crate::start`): a throwaway local root, a saved profile, the workspace window, a console
//! opened through the console host's own restore path, the run UI and the results tile. Nothing
//! here changes product behaviour: the module exists only with the `bench` feature, and it
//! drives the app through the public seams the tests use (the editor's `FrameProbe`, the
//! workspace, the console registry, the results tile).
//!
//! Shared pieces:
//! - [`BenchRoot`]: the run's own `LocalPaths` under the system temp folder, removed at the end;
//!   [`save_profile`] and [`save_layout`] write `connections.toml` and `workspace.json` there
//!   before boot, so the app restores them as it would a user's.
//! - [`start_app`]: `crate::start` with the bench's language and OS locale (the format locale)
//!   and a memory secret store (`ConnectionsBoot::for_tests`: no keychain, no credential files).
//! - [`open_console`]: opens a console tab bound to a profile over a text through the host's
//!   restore path (`<data>/consoles/<profile>/<n>.sql`, read on the core runtime), the one way
//!   the M3a app opens a file.
//! - [`PresentProbe`]: the editor's B3 probe. The input is marked at the editor's handler entry
//!   (M0 addendum finding 6): `EditorView::commit` for commands such as Backspace and PageDown,
//!   and the input handler's `replace_text_in_range` / `replace_and_mark_text_in_range` for
//!   typed text and compositions (fix round 1, I2); the frame is recorded once the frame that
//!   shows it has been drawn and presented (a deferred effect after `draw` and `present`, the
//!   spike's `record_after_present`).
//! - [`FrameCpu`]: the whole window's CPU time per frame, from the frame callback to the next
//!   main-thread turn (draw, present, and anything queued before that turn): an upper bound on
//!   the grid's own paint, which `b1_real` reports from the grid's `bench_stats` (the app's
//!   `bench` feature enables `hacivat-ui-grid/bench`).
//! - [`Outcome`] / [`print_result`] / [`end_run`]: every run ends with `RESULT: …`, an exit code
//!   that follows it, and its root removed after the core runtime stopped.

pub mod b1_real;
pub mod b3;

use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context as _, anyhow, bail};
use futures::future::BoxFuture;
use hacivat_driver::fake::{FakeDriver, FakeExec, FakeQuery};
use hacivat_driver::{
    Driver, DriverConnect, DriverDescriptor, DriverRegistry, ExecSummary, RawConn, RawError,
};
use hacivat_l10n::OsLocale;
use hacivat_proto::{DbmsKind, ProfileId};
use hacivat_session::profile::{Profile, ProfileStore};
use hacivat_settings::{LanguagePref, LocalPaths, workspace_state};
use hacivat_ui::panes::TabId;
use hacivat_ui_core::frames::{LatencyProbe, Verdict};
use hacivat_ui_core::gpui_surface::{App, AppContext, Context, Entity, Task, Window, WindowHandle};
use hacivat_ui_editor::{EditorView, FrameProbe};

use crate::connections::{ConnectionsBoot, ConnectionsModel};
use crate::console::{ConsoleId, Consoles};
use crate::shell::TileLayout;
use crate::shell::workspace::{WORKSPACE_STATE_VERSION, Workspace};
use crate::{Startup, start};

/// How often a bench script looks again while it waits for the app.
pub const POLL: Duration = Duration::from_millis(2);

/// Exit code of a bench that could not run (bad arguments, a missing fixture or server).
pub const EXIT_USAGE: u8 = 2;

/// A bench script's sleep: the GPUI background executor's timer (a `Task<()>`).
pub trait Sleep: Fn(Duration) -> Task<()> {}
impl<F: Fn(Duration) -> Task<()>> Sleep for F {}

// ---- the local root --------------------------------------------------------------------------

/// The run's own local folders (config, data, cache) under the system temp folder. Removed by
/// [`BenchRoot::remove`] at the end of the run (a GPUI quit ends the process, so no `Drop`).
#[derive(Debug, Clone)]
pub struct BenchRoot {
    dir: PathBuf,
    paths: LocalPaths,
}

impl BenchRoot {
    /// A fresh root named after `tag`, the process, the time and a per-process sequence number.
    pub fn new(tag: &str) -> anyhow::Result<BenchRoot> {
        // ## m3c-t44 (fix of the M3a headless flake "timed out waiting for the connection"):
        // the wall clock has microsecond resolution on macOS, so two tests booting in the same
        // microsecond shared one root, and the later `connections.toml` dropped the earlier
        // test's profile (its connect then never happened). The sequence number keeps every
        // root of the process apart.
        static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        let dir = std::env::temp_dir().join(format!(
            "hacivat-bench-{tag}-{}-{nanos}-{seq}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
        let paths = LocalPaths::under(&dir);
        paths.ensure_dirs().context("creating the local folders")?;
        Ok(BenchRoot { dir, paths })
    }

    pub fn paths(&self) -> &LocalPaths {
        &self.paths
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Removes the root and everything in it.
    pub fn remove(&self) {
        if let Err(err) = std::fs::remove_dir_all(&self.dir) {
            eprintln!("could not remove {}: {err}", self.dir.display());
        }
    }
}

/// Saves `profiles` as the root's `connections.toml` (before boot).
pub fn save_profile(root: &BenchRoot, profiles: &[Profile]) -> anyhow::Result<()> {
    let (store, _) = ProfileStore::open(root.paths().connections_toml());
    store
        .save(profiles)
        .map_err(|e| anyhow!("saving connections.toml: {e:?}"))
}

/// Saves a `workspace.json` with `tiles` and one unbound `Console 1` (before boot), so the
/// workspace restores that layout as it would a user's.
pub fn save_layout(root: &BenchRoot, tiles: TileLayout) -> anyhow::Result<()> {
    let value = serde_json::json!({
        "tiles": serde_json::to_value(tiles)?,
        "panes": {"root": {"Leaf": 0}, "panes": {"0": {"tabs": [0], "active": 0}}, "focused": 0},
        "tabs": [{"id": 0, "kind": "console", "n": 1}],
    });
    workspace_state::save(
        &root.paths().workspace_json(),
        WORKSPACE_STATE_VERSION,
        &value,
    )
    .context("saving workspace.json")
}

// ---- the app -----------------------------------------------------------------------------------

/// What a bench app boots with.
pub struct Launch {
    pub root: BenchRoot,
    /// The UI language tag (`tr`, `en`).
    pub lang: String,
    /// The OS regional format, which the results format numbers and dates with (`tr-TR`).
    pub format: String,
    /// The OS time zone (IANA).
    pub zone: String,
    pub drivers: DriverRegistry,
}

/// Starts the app (`crate::start`) on `launch`: its settings with the bench's UI language, an OS
/// locale whose regional format is `launch.format`, and the connections over `launch.drivers`
/// with a memory secret store. The workspace window opens.
pub fn start_app(cx: &mut App, launch: Launch) -> anyhow::Result<WindowHandle<Workspace>> {
    let paths = launch.root.paths().clone();
    let mut outcome = hacivat_settings::load(&paths.settings_toml());
    outcome.settings.ui.language = LanguagePref::Tag(launch.lang);
    let os = OsLocale {
        languages: vec![launch.format.clone()],
        format: Some(launch.format),
        time_zone: Some(launch.zone),
        env: Vec::new(),
    };
    let mut startup = Startup::new(paths.clone(), outcome, os);
    startup.connections = Some(ConnectionsBoot::for_tests(&paths, launch.drivers));
    start(cx, startup, None)
}

/// The format locale and time zone the app formats results with (`L10n`'s snapshot).
pub fn format_locale(cx: &App) -> Option<(String, String)> {
    let g = cx.try_global::<crate::l10n_global::L10nGlobal>()?;
    let f = g.l10n.snapshot().format.clone();
    Some((f.format_locale, f.time_zone))
}

/// `FakeDriver` presented as a driver that dials its own target (MySQL's shape): the connect
/// pipeline then opens no stream, and the fake never dials, so the bench opens no socket.
struct DialsItself {
    fake: Arc<FakeDriver>,
    descriptor: DriverDescriptor,
}

impl Driver for DialsItself {
    fn descriptor(&self) -> &DriverDescriptor {
        &self.descriptor
    }

    fn connect(
        &self,
        req: DriverConnect,
    ) -> BoxFuture<'static, Result<Box<dyn RawConn>, RawError>> {
        Driver::connect(self.fake.as_ref(), req)
    }
}

/// A registry whose PostgreSQL driver is `fake` (dialing itself).
pub fn fake_registry(fake: FakeDriver) -> DriverRegistry {
    let fake = Arc::new(fake);
    let mut descriptor = fake.descriptor().clone();
    descriptor.caps.stream_injection = false;
    let mut drivers = DriverRegistry::new();
    drivers.register(Arc::new(DialsItself { fake, descriptor }));
    drivers
}

/// A PostgreSQL `FakeDriver` that answers the console's SQL-context probe and serves
/// `queries` (each used once, by the first typed query its prefix matches).
pub fn fake_pg(queries: Vec<FakeQuery>) -> FakeDriver {
    let probes: Vec<FakeExec> = (0..64)
        .map(|_| FakeExec {
            prefix: "SHOW standard_conforming_strings",
            result: Ok(ExecSummary {
                rows: vec![vec![Some("on".to_string())]],
                row_count: 1,
                ..ExecSummary::default()
            }),
            delay: Duration::ZERO,
            tx_after: None,
            panics: false,
        })
        .collect();
    FakeDriver::new(DbmsKind::Postgres)
        .tracks_transactions()
        .exec(probes)
        .queries(queries)
}

/// Connects profile `id` (with `password` for this connect only, never stored).
pub fn connect(cx: &mut App, id: ProfileId, password: Option<secrecy::SecretString>) {
    ConnectionsModel::connect(cx, id, password);
}

/// Whether profile `id` is connected.
pub fn connected(cx: &App, id: ProfileId) -> bool {
    cx.try_global::<ConnectionsModel>()
        .is_some_and(|m| matches!(m.live(id), hacivat_session::LiveState::Connected(_)))
}

/// Opens `Console <n>` bound to `profile` in the focused pane over `text`, through the console
/// host's restore path: the text is written to the console's file first, and the host reads it
/// on the core runtime when the tab is first drawn, then opens the editor. Returns the tab and
/// when the file was in place (the open starts there).
pub fn open_console(
    ws: &mut Workspace,
    profile: ProfileId,
    text: &str,
    window: &mut Window,
    cx: &mut Context<Workspace>,
    paths: &LocalPaths,
) -> anyhow::Result<(TabId, Instant)> {
    // The host is built when the tab first draws, which is after this update: the file is in
    // place before it reads it.
    let tab = ws
        .open_console_tab(Some(profile), window, cx)
        .context("no console tab opened")?;
    let Some(crate::shell::TabKind::Console { n, .. }) = ws.tab_kind(tab) else {
        bail!("the new tab is not a console");
    };
    let path = crate::console::autosave::console_path(paths, profile, n);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    std::fs::write(&path, text).with_context(|| format!("writing {}", path.display()))?;
    Ok((tab, Instant::now()))
}

/// The console of `tab` and its editor, once the host has opened it.
pub fn console_editor(cx: &App, tab: TabId) -> Option<(ConsoleId, Entity<EditorView>)> {
    let id = Consoles::by_tab(cx, tab)?;
    Some((id, Consoles::get(cx, id)?.editor.clone()?))
}

/// The console's SQL context has been read from its session (Run is possible).
pub fn console_ready(cx: &App, console: ConsoleId) -> bool {
    crate::exec_ui::sql_context(console, cx) != crate::exec_ui::context::SqlContext::Pending
}

/// Waits until `cond` holds (polled every [`POLL`] in the window's update), at most `timeout`;
/// how long it took.
pub async fn wait_until<C: AppContext>(
    cx: &mut C,
    ws: WindowHandle<Workspace>,
    sleep: &impl Sleep,
    what: &str,
    timeout: Duration,
    cond: impl FnMut(&mut Workspace, &mut Window, &mut App) -> bool,
) -> anyhow::Result<Duration> {
    wait_until_every(cx, ws, sleep, what, timeout, POLL, cond).await
}

/// [`wait_until`], looking every `poll` (a measured fling waits with a slow poll, so the wait
/// itself adds next to no work to the frames it measures).
pub async fn wait_until_every<C: AppContext>(
    cx: &mut C,
    ws: WindowHandle<Workspace>,
    sleep: &impl Sleep,
    what: &str,
    timeout: Duration,
    poll: Duration,
    mut cond: impl FnMut(&mut Workspace, &mut Window, &mut App) -> bool,
) -> anyhow::Result<Duration> {
    let started = Instant::now();
    loop {
        let done = ws
            .update(cx, |ws, window, cx| cond(ws, window, cx))
            .with_context(|| format!("waiting for {what}: the window is gone"))?;
        if done {
            return Ok(started.elapsed());
        }
        if started.elapsed() > timeout {
            bail!("timed out after {timeout:?} waiting for {what}");
        }
        sleep(poll).await;
    }
}

/// Activates the window and waits until it is the key window (synthetic input to an inactive
/// window is throttled by GPUI, so such a run is not representative). With `allow_inactive`
/// (`--allow-inactive`, a harness check on a machine whose screen is locked or asleep) the run
/// goes on after a warning; its inputs then count as inactive and the verdict fails.
pub async fn activate<C: AppContext>(
    cx: &mut C,
    ws: WindowHandle<Workspace>,
    sleep: &impl Sleep,
    allow_inactive: bool,
) -> anyhow::Result<()> {
    ws.update(cx, |_, window, cx| {
        window.activate_window();
        cx.activate(true);
    })?;
    let key = wait_until(
        cx,
        ws,
        sleep,
        "the window to be frontmost (key)",
        Duration::from_secs(5),
        |_, window, _| window.is_window_active(),
    )
    .await;
    match key {
        Ok(_) => Ok(()),
        Err(e) if allow_inactive => {
            eprintln!(
                "warning: {e:#}; going on (--allow-inactive): NOT a valid measurement, the verdict fails"
            );
            Ok(())
        }
        Err(e) => Err(e),
    }
}

// ## m3c-t44: a window that cannot become key refuses the run (PENDING-PO), never a FAIL.
/// What a refused run prints: the measurement waits for the PO's Mac.
pub const PENDING_PO: &str = "PENDING-PO: run it on the PO's Mac with the display awake, the session unlocked and the bench window frontmost";

/// [`activate`], except that a window that cannot become key (the display asleep, the session
/// locked, another window frontmost) refuses the run instead of aborting it: `Some(reason)`,
/// and the caller reports INVALID with [`PENDING_PO`] (M3a T36: no figure exists then, GPUI
/// throttles or stops an inactive window's frames). Under `--allow-inactive` (a harness check)
/// the run goes on as before, ungraded. Fix round 2 (M8): only the wait for the key window
/// running out refuses; any other error (the window gone) stays an error, as in [`activate`].
pub async fn activate_or_refuse<C: AppContext>(
    cx: &mut C,
    ws: WindowHandle<Workspace>,
    sleep: &impl Sleep,
    allow_inactive: bool,
) -> anyhow::Result<Option<String>> {
    ws.update(cx, |_, window, cx| {
        window.activate_window();
        cx.activate(true);
    })?;
    let started = Instant::now();
    loop {
        let key = ws
            .update(cx, |_, window, _| window.is_window_active())
            .context("waiting for the window to be frontmost: the window is gone")?;
        if key {
            return Ok(None);
        }
        if let Some(why) = activation_timed_out(started.elapsed(), allow_inactive) {
            return Ok(why);
        }
        sleep(POLL).await;
    }
}

/// How long a bench waits for its window to become key.
pub const ACTIVATE_TIMEOUT: Duration = Duration::from_secs(5);

/// The activation wait's verdict after `waited`: `None` while it may still wait;
/// `Some(Some(reason))` once it ran out (the run is refused); `Some(None)` once it ran out under
/// `--allow-inactive` (the harness check goes on, with a warning).
pub fn activation_timed_out(waited: Duration, allow_inactive: bool) -> Option<Option<String>> {
    if waited <= ACTIVATE_TIMEOUT {
        return None;
    }
    let why = format!(
        "the bench window did not become frontmost within {ACTIVATE_TIMEOUT:?}; the display may be asleep or the session locked"
    );
    if allow_inactive {
        eprintln!("warning: {why}; going on (--allow-inactive): NOT a valid measurement, ungraded");
        Some(None)
    } else {
        Some(Some(why))
    }
}

// ---- running a result --------------------------------------------------------------------------

/// A result fetched in full: when its first rows and its end came (from Run), its rows and the
/// store's bytes, whether "Fetch more anyway" was taken, and how often "Fetch all" was asked.
#[derive(Debug, Clone, Copy)]
pub struct Loaded {
    pub first_rows: Duration,
    pub complete: Duration,
    pub rows: u64,
    pub bytes: u64,
    pub raised: bool,
    pub fetch_alls: u32,
}

/// The grid of `console`'s first result tab, once the results tile has shown it.
pub fn result_grid(cx: &App, console: ConsoleId) -> Option<Entity<hacivat_ui_grid::GridView>> {
    crate::results::view::hosts(cx)
        .into_iter()
        .find_map(|h| h.read(cx).grid_of(console, 0, cx))
}

/// The state of a "Fetch all" until `Complete`.
#[derive(Debug, Default)]
pub struct Fetching {
    pub asked: Option<Instant>,
    pub fetch_alls: u32,
    pub raised: bool,
    pub first_rows: Option<Instant>,
    pub rows: u64,
    pub failed: Option<String>,
}

impl Fetching {
    /// One look at `console`'s first result: asks "Fetch all" while the cursor waits, takes
    /// "Fetch more anyway" when the result stopped at its memory cap (R-M3-19 and the ledger's
    /// T36 note: 1M × 50 passes the 256 MiB cap), and says whether the result ended. A result
    /// that ends any way other than `Complete` is a failure (`failed`).
    pub fn step(&mut self, cx: &mut App, console: ConsoleId) -> bool {
        use hacivat_results::Completeness;
        let Some(results) = Consoles::get(cx, console).and_then(|s| s.results.clone()) else {
            return false;
        };
        let Some((snapshot, banner, source)) = results
            .read(cx)
            .tab(0)
            .map(|t| (t.adapter().snapshot(), t.memory_banner(), t.source()))
        else {
            return false;
        };
        self.rows = snapshot.row_count();
        if self.first_rows.is_none() && self.rows > 0 {
            self.first_rows = Some(Instant::now());
        }
        match snapshot.completeness() {
            Completeness::Complete => true,
            Completeness::Streaming => false,
            Completeness::Suspended(_) => {
                if let Some(banner) = banner {
                    if banner.can_raise && !self.raised {
                        // "Fetch more anyway": the one path that records the raise.
                        results.update(cx, |r, cx| r.raise_cap(0, cx));
                        self.raised = true;
                        self.asked = Some(Instant::now());
                    } else if !banner.can_raise {
                        self.failed = Some(format!(
                            "the result stopped at {:?} with nothing left to raise",
                            banner.reason
                        ));
                        return true;
                    }
                } else if self
                    .asked
                    .is_none_or(|at| at.elapsed() > Duration::from_secs(2))
                {
                    // "Fetch all" (asked again only if the cursor still waits 2 s later).
                    source.fetch_all();
                    self.fetch_alls += 1;
                    self.asked = Some(Instant::now());
                }
                false
            }
            other => {
                self.failed = Some(format!("the result ended {other:?}"));
                true
            }
        }
    }
}

/// Selects `range` of the console's editor, runs it (`editor.run`), then fetches the whole
/// result ([`Fetching`]); what was loaded. Fails when it does not complete within `timeout`.
pub async fn run_and_load<C: AppContext>(
    cx: &mut C,
    ws: WindowHandle<Workspace>,
    sleep: &impl Sleep,
    console: ConsoleId,
    editor: &Entity<EditorView>,
    range: std::ops::Range<usize>,
    timeout: Duration,
) -> anyhow::Result<Loaded> {
    let ran = start_run(cx, ws, editor, range)?;
    let mut fetching = Fetching::default();
    wait_until(
        cx,
        ws,
        sleep,
        "the result to complete",
        timeout,
        |_, _, cx| fetching.step(cx, console),
    )
    .await?;
    finish_load(cx, ws, console, ran, fetching)
}

/// Selects `range` and runs it (`editor.run`); when Run was asked.
pub fn start_run<C: AppContext>(
    cx: &mut C,
    ws: WindowHandle<Workspace>,
    editor: &Entity<EditorView>,
    range: std::ops::Range<usize>,
) -> anyhow::Result<Instant> {
    ws.update(cx, |_, window, cx| {
        editor.update(cx, |e, cx| {
            e.select_range(range, cx);
            let ran = Instant::now();
            if e.run_command("editor.run", window, cx) {
                Ok(ran)
            } else {
                Err(anyhow!("editor.run did not run"))
            }
        })
    })?
}

/// What `fetching` loaded, once it ended.
pub fn finish_load<C: AppContext>(
    cx: &mut C,
    ws: WindowHandle<Workspace>,
    console: ConsoleId,
    ran: Instant,
    fetching: Fetching,
) -> anyhow::Result<Loaded> {
    if let Some(failed) = fetching.failed {
        bail!("{failed}");
    }
    let complete = ran.elapsed();
    let (rows, bytes) = ws
        .update(cx, |_, _, cx| {
            let results = Consoles::get(cx, console)?.results.clone()?;
            let snapshot = results.read(cx).tab(0)?.adapter().snapshot();
            Some((snapshot.row_count(), snapshot.bytes()))
        })?
        .context("the result tab went away")?;
    Ok(Loaded {
        first_rows: fetching
            .first_rows
            .map_or(complete, |t| t.duration_since(ran)),
        complete,
        rows,
        bytes,
        raised: fetching.raised,
        fetch_alls: fetching.fetch_alls,
    })
}

// ---- probes ------------------------------------------------------------------------------------

/// The B3 probe on a console's editor: input marked at the editor's handler entry, the frame
/// recorded after it was drawn and presented.
#[derive(Clone)]
pub struct PresentProbe(Rc<ProbeState>);

struct ProbeState {
    probe: RefCell<LatencyProbe>,
    /// Inputs are measured only while armed (warm-up keys are not).
    armed: Cell<bool>,
    last_mark: Cell<Option<Instant>>,
    inputs: Cell<u64>,
    frames: Cell<u64>,
}

impl PresentProbe {
    pub fn new(label: &str) -> PresentProbe {
        PresentProbe(Rc::new(ProbeState {
            probe: RefCell::new(LatencyProbe::new(label)),
            armed: Cell::new(false),
            last_mark: Cell::new(None),
            inputs: Cell::new(0),
            frames: Cell::new(0),
        }))
    }

    pub fn arm(&self, on: bool) {
        self.0.armed.set(on);
    }

    /// The measured inputs (marks) and the frames that showed one.
    pub fn counts(&self) -> (u64, u64) {
        (self.0.inputs.get(), self.0.frames.get())
    }

    pub fn latency(&self) -> std::cell::Ref<'_, LatencyProbe> {
        self.0.probe.borrow()
    }

    /// As an editor's `FrameProbe`.
    pub fn as_frame_probe(&self) -> Rc<dyn FrameProbe> {
        Rc::new(self.clone())
    }
}

impl FrameProbe for PresentProbe {
    /// The editor calls this at its handler entry. The probe stamps the wall clock itself: the
    /// editor's `at` is the GPUI executor clock, which is `Instant::now()` on a real platform
    /// but a fake clock under GPUI's test scheduler, and the frame side is always wall time.
    fn input(&self, _executor_at: Instant) {
        if !self.0.armed.get() {
            return;
        }
        let at = Instant::now();
        self.0.inputs.set(self.0.inputs.get() + 1);
        self.0.last_mark.set(Some(at));
        self.0.probe.borrow_mut().mark_input(at);
    }

    /// Called while the editor paints. GPUI also draws synchronously, without presenting, when
    /// a key arrives while the window is dirty; such a draw cannot show an input marked after
    /// it was painted, so that input pairs with the next frame (the spike's rule).
    fn painted(&self, cx: &mut App) {
        let painted_at = Instant::now();
        let state = self.0.clone();
        cx.defer(move |_| {
            if !state.armed.get() {
                return;
            }
            if state.last_mark.get().is_none_or(|mark| mark <= painted_at) {
                let mut probe = state.probe.borrow_mut();
                let before = probe.stats().samples;
                probe.on_frame(Instant::now());
                if probe.stats().samples > before {
                    state.frames.set(state.frames.get() + 1);
                }
            }
        });
    }
}

/// The window's CPU time per frame: from the frame callback (before the draw) to the next turn
/// of the main thread (after the draw and the present). An upper bound on any one element's
/// paint in that frame.
#[derive(Clone, Default)]
pub struct FrameCpu(Rc<RefCell<Vec<f64>>>);

impl FrameCpu {
    /// Call from a frame callback: the sample ends when the main thread next runs a task.
    pub fn frame_started(&self, cx: &mut App) {
        let started = Instant::now();
        let samples = self.0.clone();
        cx.spawn(async move |_| {
            samples
                .borrow_mut()
                .push(started.elapsed().as_secs_f64() * 1000.0);
        })
        .detach();
    }

    pub fn samples(&self) -> Vec<f64> {
        self.0.borrow().clone()
    }

    pub fn clear(&self) {
        self.0.borrow_mut().clear();
    }
}

// ---- numbers -----------------------------------------------------------------------------------

/// Nearest-rank percentiles: (count, p50, p95, p99, max).
pub fn percentiles(samples: &[f64]) -> (usize, f64, f64, f64, f64) {
    let mut s = samples.to_vec();
    s.sort_by(f64::total_cmp);
    let pct = |p: f64| {
        if s.is_empty() {
            0.0
        } else {
            s[(((p / 100.0) * s.len() as f64).ceil() as usize).clamp(1, s.len()) - 1]
        }
    };
    (
        s.len(),
        pct(50.0),
        pct(95.0),
        pct(99.0),
        s.last().copied().unwrap_or(0.0),
    )
}

// ## m3c-t44: one set of figures for every report (B3 latency, the worker's relex, B1 paint).
/// A run's figures in milliseconds: nearest-rank p50/p95/p99/max over `n` samples and how many
/// samples were over 3 ms (the 120 Hz half-frame) and over 6 ms (the ALG paint budget).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Figures {
    pub n: usize,
    pub p50: f64,
    pub p95: f64,
    pub p99: f64,
    pub max: f64,
    pub over_3ms: usize,
    pub over_6ms: usize,
}

impl Figures {
    pub fn of(samples_ms: &[f64]) -> Figures {
        let (n, p50, p95, p99, max) = percentiles(samples_ms);
        let over = |ms: f64| samples_ms.iter().filter(|v| **v > ms).count();
        Figures {
            n,
            p50,
            p95,
            p99,
            max,
            over_3ms: over(3.0),
            over_6ms: over(6.0),
        }
    }

    /// `n=… p50=…ms p95=…ms p99=…ms max=…ms` with `decimals` places (the log's text).
    pub fn text(&self, decimals: usize) -> String {
        format!(
            "n={} p50={:.d$}ms p95={:.d$}ms p99={:.d$}ms max={:.d$}ms",
            self.n,
            self.p50,
            self.p95,
            self.p99,
            self.max,
            d = decimals
        )
    }

    /// [`Figures::text`] with the counts over 3 ms and 6 ms (the paint lines).
    pub fn text_with_counts(&self) -> String {
        format!(
            "{} over-3ms={} over-6ms={}",
            self.text(2),
            self.over_3ms,
            self.over_6ms
        )
    }

    /// The figures as the sidecars write them.
    pub fn json(&self) -> serde_json::Value {
        serde_json::json!({
            "n": self.n, "p50": self.p50, "p95": self.p95, "p99": self.p99, "max": self.max,
            "over_3ms": self.over_3ms, "over_6ms": self.over_6ms,
        })
    }
}

pub fn verdict_text(v: Verdict) -> &'static str {
    match v {
        Verdict::Pass => "PASS",
        Verdict::Fail => "FAIL",
        Verdict::NoThreshold => "n/a",
    }
}

/// How a bench run ended, for its final `RESULT:` line and its exit code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Pass,
    Fail,
    /// The run's conditions were not met (display, size, tier, file size, an inactive window):
    /// nothing graded was written.
    Invalid,
    /// An ungraded label (streaming, off-panel, off-locale, no shared threshold).
    NotGraded,
}

impl Outcome {
    pub fn of(v: Verdict) -> Outcome {
        match v {
            Verdict::Pass => Outcome::Pass,
            Verdict::Fail => Outcome::Fail,
            Verdict::NoThreshold => Outcome::NotGraded,
        }
    }

    pub fn text(self) -> &'static str {
        match self {
            Outcome::Pass => "PASS",
            Outcome::Fail => "FAIL",
            Outcome::Invalid => "INVALID",
            Outcome::NotGraded => "n/a",
        }
    }

    /// 0 for PASS or ungraded, 1 for FAIL, [`EXIT_INVALID`] for INVALID. Under
    /// `--allow-inactive` (a harness check; CI under xvfb) a FAIL exits 0, because the verdict
    /// fails there by construction.
    pub fn exit_code(self, allow_inactive: bool) -> i32 {
        match self {
            Outcome::Pass | Outcome::NotGraded => 0,
            Outcome::Fail if allow_inactive => 0,
            Outcome::Fail => 1,
            Outcome::Invalid => EXIT_INVALID,
        }
    }
}

/// Exit code of a run whose conditions were not met (no graded JSON written).
pub const EXIT_INVALID: i32 = 3;

/// The run's last line: `RESULT: PASS|FAIL|INVALID|n/a <label>`.
pub fn print_result(outcome: Outcome, label: &str) {
    println!("RESULT: {} {label}", outcome.text());
}

/// Ends the run: stops the core runtime (no autosave or other write is left in flight), removes
/// the run's root, and exits with `code`. GPUI's quit hooks are skipped on purpose: they would
/// write the dirty console and the layout back into the removed root (review minor 15).
pub fn end_run(cx: &App, root: &BenchRoot, code: i32) -> ! {
    if let Some(m) = cx.try_global::<ConnectionsModel>() {
        m.core_runtime().shutdown(Duration::from_secs(10));
    }
    root.remove();
    std::process::exit(code)
}

/// The process's resident set size, from `ps` (no `unsafe`); `None` where `ps` is missing.
pub fn rss_bytes() -> Option<u64> {
    let out = std::process::Command::new("ps")
        .args(["-o", "rss=", "-p", &std::process::id().to_string()])
        .output()
        .ok()?;
    let kib: u64 = String::from_utf8_lossy(&out.stdout).trim().parse().ok()?;
    Some(kib * 1024)
}

/// Bytes as MiB with one decimal (machine text for the bench log).
pub fn mib(bytes: u64) -> String {
    format!("{:.1} MiB", bytes as f64 / (1024.0 * 1024.0))
}

/// The value of `--name <value>` in `args`.
pub fn flag<'a>(args: &'a [String], name: &str) -> Option<&'a str> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1))
        .map(String::as_str)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentiles_are_nearest_rank() {
        let v: Vec<f64> = (1..=100).map(f64::from).collect();
        assert_eq!(percentiles(&v), (100, 50.0, 95.0, 99.0, 100.0));
        assert_eq!(percentiles(&[]), (0, 0.0, 0.0, 0.0, 0.0));
    }

    // ## m3c-t44 (fix round 2, M8): only the key-window wait running out refuses a run.
    #[test]
    fn only_the_activation_timeout_refuses() {
        let short = ACTIVATE_TIMEOUT / 2;
        let past = ACTIVATE_TIMEOUT + Duration::from_millis(1);
        assert_eq!(activation_timed_out(short, false), None, "still waiting");
        assert_eq!(activation_timed_out(short, true), None);
        let refused = activation_timed_out(past, false).expect("ran out");
        let why = refused.expect("refused without --allow-inactive");
        assert!(why.contains("did not become frontmost"), "{why}");
        // The harness check goes on.
        assert_eq!(activation_timed_out(past, true), Some(None));
    }

    // ## m3c-t44
    #[test]
    fn roots_made_at_once_never_share_a_folder() {
        let roots: Vec<BenchRoot> = (0..64)
            .map(|_| BenchRoot::new("t44-seq").unwrap())
            .collect();
        let mut dirs: Vec<&Path> = roots.iter().map(BenchRoot::dir).collect();
        dirs.sort();
        dirs.dedup();
        assert_eq!(dirs.len(), 64);
        for r in &roots {
            r.remove();
        }
    }

    #[test]
    fn figures_give_p99_and_the_counts_over_3_and_6_ms() {
        // 1..=100 ms: p99 is the 99th sample; 97 samples are over 3 ms, 94 over 6 ms.
        let v: Vec<f64> = (1..=100).map(f64::from).collect();
        let f = Figures::of(&v);
        assert_eq!(
            (f.n, f.p50, f.p95, f.p99, f.max),
            (100, 50.0, 95.0, 99.0, 100.0)
        );
        assert_eq!((f.over_3ms, f.over_6ms), (97, 94));
        // Exactly 3 or 6 ms is not over.
        let edge = Figures::of(&[3.0, 6.0, 6.01]);
        assert_eq!((edge.over_3ms, edge.over_6ms), (2, 1));
        assert_eq!(
            Figures::of(&[1.5, 2.25]).text_with_counts(),
            "n=2 p50=1.50ms p95=2.25ms p99=2.25ms max=2.25ms over-3ms=0 over-6ms=0"
        );
        assert_eq!(
            Figures::of(&[0.1234]).text(3),
            "n=1 p50=0.123ms p95=0.123ms p99=0.123ms max=0.123ms"
        );
        let empty = Figures::of(&[]);
        assert_eq!((empty.n, empty.p95, empty.over_6ms), (0, 0.0, 0));
        assert_eq!(f.json()["p99"], 99.0);
        assert_eq!(f.json()["over_6ms"], 94);
    }

    #[test]
    fn flags_take_the_next_argument() {
        let args: Vec<String> = ["--bench", "typing", "--file", "x.sql"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(flag(&args, "--file"), Some("x.sql"));
        assert_eq!(flag(&args, "--size"), None);
    }

    #[test]
    fn the_layout_file_restores_as_a_workspace_layout() {
        let root = BenchRoot::new("t36-layout").unwrap();
        let tiles = TileLayout {
            editor_h: crate::shell::tiles::MIN_TILE_PX,
            explorer_visible: false,
            ..TileLayout::default()
        };
        save_layout(&root, tiles).unwrap();
        let value = workspace_state::load(&root.paths().workspace_json(), WORKSPACE_STATE_VERSION)
            .expect("a readable workspace.json");
        let back: TileLayout = serde_json::from_value(value["tiles"].clone()).unwrap();
        assert_eq!(back, tiles);
        root.remove();
        assert!(!root.dir().exists());
    }

    // ---- the harness, headless (the benches themselves need a real, active window) --------

    use gpui::{TestAppContext, VisualTestContext};
    use hacivat_ui_core::gpui_surface::Focusable as _;

    /// Waits until `pred` holds, at most `timeout`: core threads run (a short real sleep), the
    /// GPUI clock moves one frame, and the woken GPUI tasks run (the app tests' `settle_until`).
    fn settle(
        vcx: &mut VisualTestContext,
        what: &str,
        timeout: Duration,
        mut pred: impl FnMut(&mut VisualTestContext) -> bool,
    ) {
        let deadline = Instant::now() + timeout;
        loop {
            vcx.executor().allow_parking();
            vcx.update(|window, _| window.refresh());
            vcx.run_until_parked();
            if pred(vcx) {
                return;
            }
            assert!(Instant::now() < deadline, "timed out waiting for {what}");
            std::thread::sleep(Duration::from_millis(2));
            vcx.executor().advance_clock(Duration::from_millis(16));
        }
    }

    /// The real app on GPUI's test platform (headless), booted like the benches.
    struct Headless {
        root: BenchRoot,
        ws: WindowHandle<Workspace>,
        vcx: &'static mut VisualTestContext,
        profile: ProfileId,
    }

    fn boot(cx: &mut TestAppContext, profile: Profile, drivers: DriverRegistry) -> Headless {
        let root = BenchRoot::new("t36-headless").unwrap();
        let id = profile.id;
        save_profile(&root, &[profile]).unwrap();
        let launch = Launch {
            root: root.clone(),
            lang: "tr".to_string(),
            format: "tr-TR".to_string(),
            zone: "Europe/Istanbul".to_string(),
            drivers,
        };
        let ws = cx.update(|cx| start_app(cx, launch)).unwrap();
        cx.executor().allow_parking();
        let vcx = VisualTestContext::from_window(ws.into(), cx).into_mut();
        vcx.run_until_parked();
        Headless {
            root,
            ws,
            vcx,
            profile: id,
        }
    }

    impl Headless {
        /// Opens a console over `text` (the restore path), connects and waits for Run.
        fn console(
            &mut self,
            text: &str,
            password: Option<secrecy::SecretString>,
        ) -> (ConsoleId, Entity<EditorView>) {
            let (ws, id, paths) = (self.ws, self.profile, self.root.paths().clone());
            let (tab, _) = ws
                .update(self.vcx, |ws, window, cx| {
                    open_console(ws, id, text, window, cx, &paths)
                })
                .unwrap()
                .unwrap();
            let long = Duration::from_secs(60);
            settle(self.vcx, "the console's editor", long, |vcx| {
                vcx.cx.read(|cx| console_editor(cx, tab).is_some())
            });
            let (console, editor) = self.vcx.cx.read(|cx| console_editor(cx, tab)).unwrap();
            ws.update(self.vcx, |_, _, cx| connect(cx, id, password))
                .unwrap();
            settle(self.vcx, "the connection", long, |vcx| {
                vcx.cx.read(|cx| connected(cx, id))
            });
            settle(self.vcx, "the SQL context", long, |vcx| {
                vcx.cx.read(|cx| console_ready(cx, console))
            });
            (console, editor)
        }

        /// Runs `range` and fetches everything ([`Fetching`]), at most `timeout`.
        fn load(
            &mut self,
            console: ConsoleId,
            editor: &Entity<EditorView>,
            range: std::ops::Range<usize>,
            timeout: Duration,
        ) -> Loaded {
            let ws = self.ws;
            let ran = start_run(self.vcx, ws, editor, range).unwrap();
            let mut fetching = Fetching::default();
            settle(self.vcx, "the whole result", timeout, |vcx| {
                ws.update(vcx, |_, _, cx| fetching.step(cx, console))
                    .unwrap()
            });
            finish_load(self.vcx, ws, console, ran, fetching).unwrap()
        }

        fn stop(self) {
            self.vcx.cx.update(|cx| {
                cx.global::<ConnectionsModel>()
                    .core_runtime()
                    .shutdown(Duration::from_secs(10));
            });
            self.vcx.run_until_parked();
            self.root.remove();
        }
    }

    /// The bench's pieces over the real app, headless: the console opens from its file through
    /// the host's restore path, connects, runs, and "Fetch all" brings a result past the cursor's
    /// read-ahead to `Complete` in the grid; the editor's probe pairs each input with the frame
    /// that shows it; the format locale is the bench's.
    #[gpui::test]
    fn the_harness_opens_runs_fetches_all_and_probes(cx: &mut TestAppContext) {
        const ROWS: u64 = 25_000;
        let fake = fake_registry(fake_pg(vec![b3::result_query(ROWS)]));
        let mut h = boot(cx, b3::profile(), fake);
        let format = h.vcx.cx.read(format_locale).map(|f| f.0);
        assert_eq!(format.as_deref(), Some("tr-TR"));

        let text = "SELECT * FROM hv_bench_rows;\nSELECT 2;\n";
        let (console, editor) = h.console(text, None);
        let opened = h
            .vcx
            .cx
            .read(|cx| editor.read(cx).snapshot().buffer.to_string());
        assert_eq!(opened, text, "the host restored the console's file");

        let run = 0.."SELECT * FROM hv_bench_rows".len();
        let loaded = h.load(console, &editor, run, Duration::from_secs(20));
        assert_eq!(loaded.rows, ROWS);
        assert!(
            loaded.fetch_alls >= 1,
            "the cursor stopped at its read-ahead"
        );
        assert!(!loaded.raised, "a small result never reaches the cap");
        assert!(loaded.bytes > 0);
        settle(h.vcx, "the result's grid", Duration::from_secs(20), |vcx| {
            vcx.cx.read(|cx| result_grid(cx, console).is_some())
        });
        // Review I6: the app's `bench` feature turns on the grid's own paint timer, which
        // B1-real reports as the gate's paint figure.
        let grid = h.vcx.cx.read(|cx| result_grid(cx, console)).unwrap();
        settle(
            h.vcx,
            "the grid's paint samples",
            Duration::from_secs(20),
            |vcx| grid.update(vcx, |g, _| !g.bench_stats().paint_ms.is_empty()),
        );

        // The probe: one sample per input, each paired with a frame drawn after it.
        let probe = PresentProbe::new("b3-typing-harness");
        editor.update(h.vcx, |e, _| e.set_probe(Some(probe.as_frame_probe())));
        h.ws.update(h.vcx, |_, window, cx| {
            let handle = editor.focus_handle(cx);
            window.focus(&handle, cx);
            editor.update(cx, |e, cx| e.select_range(text.len()..text.len(), cx));
        })
        .unwrap();
        h.vcx.update(|window, _| window.refresh());
        h.vcx.run_until_parked();
        // Unarmed (warm-up) inputs are not measured.
        h.vcx.simulate_keystrokes("x");
        h.vcx.update(|window, _| window.refresh());
        h.vcx.run_until_parked();
        assert_eq!(probe.counts(), (0, 0));
        probe.arm(true);
        for key in ["a", "b", "c"] {
            h.vcx.simulate_keystrokes(key);
            h.vcx.update(|window, _| window.refresh());
            h.vcx.run_until_parked();
        }
        assert_eq!(probe.counts(), (3, 3));
        assert_eq!(probe.latency().stats().samples, 3);
        h.stop();
    }

    /// B3's replay types through the window's dispatcher as `b3::script` does: the console's
    /// editor focused, each key sent by `b3::send_key` (CI run 43779216264 panicked there with
    /// "cannot update Workspace while it is already being updated").
    #[gpui::test]
    fn b3_replay_types_into_the_console(cx: &mut TestAppContext) {
        let mut h = boot(cx, b3::profile(), fake_registry(fake_pg(Vec::new())));
        let text = "SELECT 1;\n";
        let (_, editor) = h.console(text, None);
        h.ws.update(h.vcx, |_, window, cx| {
            let handle = editor.focus_handle(cx);
            window.focus(&handle, cx);
            editor.update(cx, |e, cx| e.select_range(text.len()..text.len(), cx));
        })
        .unwrap();
        h.vcx.update(|window, _| window.refresh());
        h.vcx.run_until_parked();
        for key in b3::typing_script(4) {
            b3::send_key(h.vcx, h.ws, key).unwrap();
            h.vcx.update(|window, _| window.refresh());
            h.vcx.run_until_parked();
        }
        let typed = h
            .vcx
            .cx
            .read(|cx| editor.read(cx).snapshot().buffer.to_string());
        assert_eq!(typed, format!("{text} AND"));
        h.stop();
    }

    // ---- ## m3c-t44: the M3c modes' pieces, headless (smoke; no frames, no figures) --------

    /// Focuses the console's editor and draws.
    fn focus_editor(h: &mut Headless, editor: &Entity<EditorView>) {
        h.ws.update(h.vcx, |_, window, cx| {
            let handle = editor.focus_handle(cx);
            window.focus(&handle, cx);
        })
        .unwrap();
        h.vcx.update(|window, _| window.refresh());
        h.vcx.run_until_parked();
    }

    /// Types `keys` as the replay does, one dispatch per key. Backspace runs the command the
    /// keymap binds it to (`editor.delete_backward`): on GPUI's test platform a dispatched
    /// `backspace` reaches no binding (typed text goes through the input handler as on a real
    /// window; the real run's Backspace is the keymap's, M3a T36).
    fn type_keys(h: &mut Headless, editor: &Entity<EditorView>, keys: &[b3::Key]) {
        for key in keys {
            if *key == b3::Key::Backspace {
                let ran =
                    h.ws.update(h.vcx, |_, window, cx| {
                        editor.update(cx, |e, cx| {
                            e.run_command("editor.delete_backward", window, cx)
                        })
                    })
                    .unwrap();
                assert!(ran);
            } else {
                b3::send_key(h.vcx, h.ws, *key).unwrap();
            }
            h.vcx.update(|window, _| window.refresh());
            h.vcx.run_until_parked();
        }
    }

    fn text_of(h: &mut Headless, editor: &Entity<EditorView>) -> String {
        h.vcx
            .cx
            .read(|cx| editor.read(cx).snapshot().buffer.to_string())
    }

    /// `typing-multi-caret` through the real console: ten carets at the end of line 25,000 and
    /// the nine under it (the editor's own `add_caret_below`), each keystroke typed at every
    /// caret, and the warm-up phrase typed and erased leaves the text as it was.
    #[gpui::test]
    fn b3_multi_caret_types_at_ten_carets(cx: &mut TestAppContext) {
        let mut h = boot(cx, b3::profile(), fake_registry(fake_pg(Vec::new())));
        let text: String = (0..25_020).map(|i| format!("SELECT {i};\n")).collect();
        let (_, editor) = h.console(&text, None);
        focus_editor(&mut h, &editor);
        let (script, expected) =
            b3::place_caret(h.vcx, h.ws, b3::Mode::TypingMultiCaret, &editor).unwrap();
        assert_eq!(expected, 2_000);
        assert_eq!(script.len(), 2_000);
        let carets = h
            .vcx
            .cx
            .read(|cx| editor.read(cx).snapshot().selection.ranges().len());
        assert_eq!(carets, b3::MULTI_CARETS);
        type_keys(
            &mut h,
            &editor,
            &b3::warm_script(b3::Mode::TypingMultiCaret),
        );
        assert_eq!(text_of(&mut h, &editor), text, "the warm-up is balanced");
        type_keys(&mut h, &editor, &b3::typing_script(4));
        let typed = text_of(&mut h, &editor);
        for line in 24_999..25_009 {
            let want = format!("SELECT {line}; AND");
            assert_eq!(typed.lines().nth(line), Some(want.as_str()), "line {line}");
        }
        assert_eq!(typed.lines().nth(25_009), Some("SELECT 25009;"));
        let carets = h
            .vcx
            .cx
            .read(|cx| editor.read(cx).snapshot().selection.ranges().len());
        assert_eq!(
            b3::kept_through_run(b3::Mode::TypingMultiCaret, carets, 0),
            Ok(())
        );
        h.stop();
    }

    /// `typing-with-folds` through the real console: 1,000 folds made, the caret on line 25,000
    /// outside them, and typing keeps every fold.
    #[gpui::test]
    fn b3_folds_mode_keeps_a_thousand_folds_while_typing(cx: &mut TestAppContext) {
        let mut h = boot(cx, b3::profile(), fake_registry(fake_pg(Vec::new())));
        let text = "SELECT a FROM t;\n".repeat(50_000);
        let (_, editor) = h.console(&text, None);
        focus_editor(&mut h, &editor);
        b3::place_caret(h.vcx, h.ws, b3::Mode::TypingWithFolds, &editor).unwrap();
        let folds = |h: &mut Headless| h.vcx.cx.read(|cx| editor.read(cx).folds().folded().len());
        assert_eq!(folds(&mut h), b3::FOLDS);
        type_keys(&mut h, &editor, &b3::typing_script(11));
        let typed = text_of(&mut h, &editor);
        assert_eq!(
            typed.lines().nth(24_999),
            Some("SELECT a FROM t; AND c.city")
        );
        assert_eq!(
            folds(&mut h),
            b3::FOLDS,
            "typing outside the folds keeps them"
        );
        assert_eq!(
            b3::kept_through_run(b3::Mode::TypingWithFolds, 1, folds(&mut h)),
            Ok(())
        );
        h.stop();
    }

    /// `typing-19mb`'s caret goes to the middle line of a file over the full tier (Reduced).
    #[gpui::test]
    fn b3_large_mode_types_in_reduced_mode_at_the_middle_line(cx: &mut TestAppContext) {
        use hacivat_ui_editor::Tier;
        let mut h = boot(cx, b3::profile(), fake_registry(fake_pg(Vec::new())));
        // Over the full tier's 4 MB (the real run is 19 MiB; smaller here for a debug build).
        let text = "SELECT col FROM t;\n".repeat(225_000);
        let (_, editor) = h.console(&text, None);
        let tier = h.vcx.cx.read(|cx| editor.read(cx).status().tier);
        assert_eq!(tier, Tier::Reduced);
        assert!(b3::tier_ok(b3::Mode::Typing19Mb, tier).is_ok());
        focus_editor(&mut h, &editor);
        b3::place_caret(h.vcx, h.ws, b3::Mode::Typing19Mb, &editor).unwrap();
        type_keys(&mut h, &editor, &b3::typing_script(4));
        let typed = text_of(&mut h, &editor);
        assert_eq!(typed.lines().nth(112_500), Some("SELECT col FROM t; AND"));
        h.stop();
    }

    /// R-M3-9's probe: armed, the console's language worker records how long each analysis
    /// took to bring its document up to the typed text, incrementally.
    #[gpui::test]
    fn the_language_worker_times_its_incremental_relex(cx: &mut TestAppContext) {
        use hacivat_sql_editor::bench::{self as worker, Built};
        let mut h = boot(cx, b3::profile(), fake_registry(fake_pg(Vec::new())));
        let text = "SELECT 1;\nSELECT 2;\n";
        let (_, editor) = h.console(text, None);
        focus_editor(&mut h, &editor);
        h.ws.update(h.vcx, |_, _, cx| {
            editor.update(cx, |e, cx| e.select_range(text.len()..text.len(), cx));
        })
        .unwrap();
        let armed = worker::armed_until_dropped();
        assert!(worker::armed());
        type_keys(&mut h, &editor, &b3::typing_script(6));
        let mut samples = Vec::new();
        settle(
            h.vcx,
            "the worker's samples",
            Duration::from_secs(20),
            |_| {
                samples.extend(worker::take());
                samples.iter().any(|s| s.kind == Built::Incremental)
            },
        );
        drop(armed);
        let report = b3::RelexReport::of(&samples);
        assert!(report.incremental >= 1, "{report:?}");
        assert!(report.relex.max <= report.analysis.max, "{report:?}");
        assert!(!worker::armed(), "the guard disarmed the timer");
        h.stop();
    }

    /// `b1_real --sorted`'s step through the real app: the whole result sorted by `c01` through
    /// the grid, done once the grid shows that sort over every row.
    #[gpui::test]
    fn the_sorted_run_sorts_the_whole_result_through_the_grid(cx: &mut TestAppContext) {
        use hacivat_ui_grid::ColId;
        use hacivat_ui_grid::model::SortSpec;
        const ROWS: u64 = 20_000;
        let fake = fake_registry(fake_pg(vec![b3::result_query(ROWS)]));
        let mut h = boot(cx, b3::profile(), fake);
        let text = "SELECT * FROM hv_bench_rows;\n";
        let (console, editor) = h.console(text, None);
        let run = 0.."SELECT * FROM hv_bench_rows".len();
        let loaded = h.load(console, &editor, run, Duration::from_secs(30));
        assert_eq!(loaded.rows, ROWS);
        settle(h.vcx, "the result's grid", Duration::from_secs(20), |vcx| {
            vcx.cx.read(|cx| result_grid(cx, console).is_some())
        });
        let grid = h.vcx.cx.read(|cx| result_grid(cx, console)).unwrap();
        let spec = SortSpec::by(ColId(b1_real::SORT_COLUMN), false);
        h.ws.update(h.vcx, |_, _, cx| {
            grid.update(cx, |g, cx| g.request_sort(spec.clone(), cx));
        })
        .unwrap();
        settle(h.vcx, "the sort", Duration::from_secs(60), |vcx| {
            vcx.cx.read(|cx| {
                crate::results::sort::status(&grid, cx)
                    .is_some_and(|s| b1_real::sort_done(&s, &spec, ROWS))
            })
        });
        h.stop();
    }

    /// Review I4 through the real console path: a ≈ 2 MB file of short lines opens in the full
    /// tier (what `typing`, `typing-with-results` and `open` measure), and a line over 64 KB
    /// opens the file in Reduced mode, which those modes refuse.
    #[gpui::test]
    fn the_console_tier_follows_the_fixture_shape(cx: &mut TestAppContext) {
        use hacivat_ui_editor::Tier;
        let h = boot(cx, b3::profile(), fake_registry(fake_pg(Vec::new())));
        let short: String = "SELECT id, name FROM customers WHERE id = 42;\n".repeat(44_000);
        assert!(b3::open_size_ok(short.len()).is_ok(), "{}", short.len());
        let long = format!("SELECT 1;\nSELECT '{}';\n", "x".repeat(70_000));
        let (ws, id, paths) = (h.ws, h.profile, h.root.paths().clone());
        let mut tiers = Vec::new();
        for text in [&short, &long] {
            let (tab, _) = ws
                .update(h.vcx, |ws, window, cx| {
                    open_console(ws, id, text, window, cx, &paths)
                })
                .unwrap()
                .unwrap();
            settle(
                h.vcx,
                "the console's editor",
                Duration::from_secs(20),
                |vcx| vcx.cx.read(|cx| console_editor(cx, tab).is_some()),
            );
            let (_, editor) = h.vcx.cx.read(|cx| console_editor(cx, tab)).unwrap();
            tiers.push(h.vcx.cx.read(|cx| editor.read(cx).status().tier));
        }
        assert_eq!(tiers, [Tier::Full, Tier::Reduced]);
        assert!(b3::tier_ok(b3::Mode::Typing, tiers[0]).is_ok());
        assert!(b3::tier_ok(b3::Mode::Typing, tiers[1]).is_err());
        assert!(b3::tier_ok(b3::Mode::TypingLongLine, tiers[1]).is_ok());
        h.stop();
    }

    /// `b1_real --engine sqlite`'s dataset is valid SQLite and loads through the real SQLite
    /// driver with its 50 columns (a smaller row count than the bench's).
    #[gpui::test]
    fn the_sqlite_dataset_runs_in_the_app(cx: &mut TestAppContext) {
        use hacivat_ui_grid::GridDataSource as _;
        const ROWS: u64 = 20_000;
        let root = BenchRoot::new("t36-profile").unwrap();
        let profile = b1_real::profile(b1_real::Engine::Sqlite, None, &root);
        let mut h = boot(cx, profile, hacivat_session::default_drivers());
        let sql = b1_real::sqlite_sql(ROWS, b1_real::COLS);
        let (console, editor) = h.console(&sql, None);
        let loaded = h.load(console, &editor, 0..sql.len(), Duration::from_secs(60));
        assert_eq!(loaded.rows, ROWS);
        let columns = h.vcx.cx.read(|cx| {
            let results = Consoles::get(cx, console)?.results.clone()?;
            Some(results.read(cx).tab(0)?.adapter().columns().len())
        });
        assert_eq!(columns, Some(b1_real::COLS));
        h.stop();
        root.remove();
    }

    /// Prints a headless load (no window frames, no fling: NOT B1-real).
    fn print_headless(what: &str, loaded: &Loaded) {
        println!(
            "HEADLESS (gpui test platform; no window, no fling; not a panel measurement) {what}: rows={} load={:.2}s first_rows={:.0}ms store={} rss={} cap_raised={} fetch_all_asked={}",
            loaded.rows,
            loaded.complete.as_secs_f64(),
            loaded.first_rows.as_secs_f64() * 1000.0,
            mib(loaded.bytes),
            rss_bytes().map_or("unknown".into(), mib),
            loaded.raised,
            loaded.fetch_alls,
        );
    }

    /// The `typing-with-results` result (1M × 50 from the `FakeDriver`) passes the 256 MiB cap
    /// and completes through "Fetch more anyway". Heavy (≈ 1 GiB); run it in `--release`:
    /// `cargo test --release -p hacivat-app --features bench --lib -- --ignored headless_`.
    #[gpui::test]
    #[ignore = "heavy: 1M × 50 rows; run in --release"]
    fn headless_million_fake_rows_complete_past_the_cap(cx: &mut TestAppContext) {
        let fake = fake_registry(fake_pg(vec![b3::result_query(b3::RESULT_ROWS)]));
        let mut h = boot(cx, b3::profile(), fake);
        let text = "SELECT * FROM hv_bench_rows;\n";
        let (console, editor) = h.console(text, None);
        let run = 0.."SELECT * FROM hv_bench_rows".len();
        let loaded = h.load(console, &editor, run, Duration::from_secs(900));
        print_headless("FakeDriver 1M × 50", &loaded);
        assert_eq!(loaded.rows, b3::RESULT_ROWS);
        assert!(loaded.raised, "1M × 50 passes the 256 MiB cap");
        h.stop();
    }

    /// The B1-real dataset (1M × 50 from `pg17`, `tr-TR`) loads to `Complete` through "Fetch
    /// all" and "Fetch more anyway" in the real app, headless. Needs the Docker matrix:
    /// `HACIVAT_DOCKER_SET=pr cargo test --release -p hacivat-app --features bench --lib --
    /// --ignored headless_`. Its numbers are load figures only, never B1-real (no window).
    #[gpui::test]
    #[ignore = "docker; heavy: 1M × 50 rows; run in --release"]
    fn headless_b1_real_dataset_loads_from_pg17(cx: &mut TestAppContext) {
        let Some(t) = hacivat_testkit::require_target("pg17") else {
            return;
        };
        let password: secrecy::SecretString = t.password.clone().into();
        let target = b1_real::ServerTarget {
            host: t.host.clone(),
            port: t.port,
            user: t.user.clone(),
            password: password.clone(),
            database: t.database.clone(),
        };
        let root = BenchRoot::new("t36-profile").unwrap();
        let profile = b1_real::profile(b1_real::Engine::Pg, Some(&target), &root);
        root.remove();
        let mut h = boot(cx, profile, hacivat_session::default_drivers());
        let sql = b1_real::pg_sql(b1_real::ROWS, b1_real::COLS);
        let (console, editor) = h.console(&sql, Some(password));
        let loaded = h.load(console, &editor, 0..sql.len(), Duration::from_secs(900));
        print_headless("pg17 1M × 50 (the B1-real dataset)", &loaded);
        assert_eq!(loaded.rows, b1_real::ROWS);
        assert!(loaded.raised, "1M × 50 passes the 256 MiB cap");
        h.stop();
    }
}
