// ## m4-t29-g1-bench
//! G1 (M4 Task 29; spec §3.3 "Cached catalog load (160k columns) < 300 ms", §6 M4 exit,
//! HV-MODEL-03): the cached catalog of the 160k-column fixture on screen in the real app,
//! measured over cold processes. Definitions (verbatim in `docs/benchmarks/m4-bench-notes.md`):
//!
//! - **t0**: the app's `CatalogHost` begins opening the profile's cache file: the host's
//!   lifecycle task calls `CatalogSource::ensure` on the core runtime ([`TimedSource`]; the key
//!   fetch, the cache open, the load and the publish all happen inside it).
//! - **t1**: the first presented frame in which the explorer shows the schema's kind group with
//!   its count and the first visible table rows: after a frame's draw and present (the next
//!   main-thread turn after a `Window::on_next_frame` callback, the [`super::FrameCpu`] rule),
//!   the tree's rows built in that frame hold the `TABLES` group header of `app.public`, with its
//!   count, and at least one table row under it ([`Watch::check`]).
//! - **Conditions**: app start with the profile already expanded in the restored workspace
//!   (`workspace.json` holds the profile, database, schema and `TABLES` rows expanded), the
//!   server unreachable (`--offline`: the profile names `127.0.0.1:9` and its driver is a fake
//!   that never dials; nothing connects), a plaintext cache (the default for non-production
//!   profiles). **Cold process, warm disk**: every run is a new process, but the cache file was
//!   just copied into the run's folder, so the OS file cache holds it (dropping the file cache
//!   needs root on macOS).
//! - **Budget**: p95 of `t1 − t0` < 300 ms over `--runs` (10) cold processes, on the PO's Mac
//!   (L2, authoritative). The encrypted variant (an encrypted production profile expanded in the
//!   restored workspace, R-M4-15) is measured with the same t0 and reported separately, never
//!   graded, with its `CatalogKey` fetch broken out (`key_fetch_ms`; `--key-delay-ms` models a
//!   slow keyring, since the bench uses a memory secret store and never touches the Keychain).
//!
//! Phases per run: `key_fetch_ms` (the keyring read inside `ensure`), `cache_load_ms` (the rest
//! of `ensure`: open, read, decode, validate, publish the cached namespaces), `host_ms` (from
//! `ensure`'s answer until the explorer holds the snapshot, sampled per frame) and `paint_ms`
//! (from there to t1). Also recorded: `process_to_t1_ms` (§3.3 "≤ 1 s to an interactive
//! workspace with a cached catalog", from the example's `main`), the RSS at t1, the display.
//!
//! The parent process ([`run`]) checks the fixture's shape, prepares the sealed copy, starts one
//! child process per run (`--child <variant>`), reads each child's `G1RUN {json}` line and writes
//! one JSON dump per variant in the shared bench schema (`hacivat-bench`'s latency dump, read by
//! `cargo xtask bench-report --milestone m4`), extended with the definitions, the runs and the
//! phases.

use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use anyhow::{Context as _, anyhow, bail};
use futures::future::BoxFuture;
use hacivat_introspect::hub::{CatalogHub, HubError, HubSettings, ProfileCatalog, SessionLookup};
use hacivat_l10n::OsLocale;
use hacivat_meta::codec::FORMAT_MAJOR;
use hacivat_meta::persist::{CacheStore, Encryption, RowKey, WriterId};
use hacivat_meta::{NamespaceSnapshot, NsKey, ObjectKind};
use hacivat_proto::{DbmsKind, EnvTag, ProfileId};
use hacivat_secrets::{
    AuthProviderId, MemoryBackend, SecretBackend, SecretError, SecretKey, SecretSlot,
    SecretStore, SecretString,
};
use hacivat_session::profile::{Profile, ProfileEndpoint};
use hacivat_settings::{LanguagePref, workspace_state};
use hacivat_ui::tree::flat::RowKind;
use hacivat_ui::tree::{TreeKey, TreeModel};
use hacivat_ui_core::app::{Boot, run_app};
use hacivat_ui_core::frame_loop::{bench_dir, stamp_run, workspace_root};
use hacivat_ui_core::frames::{LatencyProbe, RunInfo, build_stamp};
use hacivat_ui_core::gpui_surface::{App, Window, WindowHandle};
use serde_json::{Value, json};

use super::{
    BenchRoot, EXIT_INVALID, EXIT_USAGE, Figures, Outcome, PENDING_PO, end_run, fake_pg,
    fake_registry, flag, print_result, rss_bytes,
};
use crate::connections::{ConnectionsBoot, ConnectionsModel};
use crate::explorer::adapter::{self, NodeInfo};
use crate::explorer::live::{self, CatalogSource, HubSource};
use crate::explorer::rows::DisplayGroup;
use crate::shell::workspace::{WORKSPACE_STATE_VERSION, Workspace};
use crate::shell::{AppState, TileLayout};
use crate::{Startup, start};

/// The gating label (plaintext cache).
pub const LABEL: &str = "g1-catalog-cold";
/// The encrypted variant's label (recorded, never graded).
pub const LABEL_ENCRYPTED: &str = "g1-catalog-cold-encrypted";
/// The G1 budget: p95 of `t1 − t0`.
pub const BUDGET_MS: f64 = 300.0;
/// The stated fixture scale (Task 11's generator in one schema; `hacivat-meta`
/// `tests/bench_fixture.rs`).
pub const TABLES: usize = 15_000;
pub const COLUMNS: usize = 160_000;
/// The database and the schema the fixture holds every table in.
pub const DB: &str = "app";
pub const SCHEMA: &str = "public";
/// The prefix of a child's one result line on stdout.
pub const RUN_LINE: &str = "G1RUN ";
/// The row key of the encrypted variant's cache: bench fixture data, not a user secret (the
/// bench's memory secret store holds it; the Keychain is never touched).
const BENCH_KEY: [u8; 32] = [71; 32];
/// A child that has not painted by then has failed.
const RUN_TIMEOUT: Duration = Duration::from_secs(60);

/// The t0 and t1 definitions, as the JSON dump states them.
pub const T0_DEF: &str = "CatalogHost begins opening the profile's cache file (CatalogSource::ensure entered on the core runtime)";
pub const T1_DEF: &str = "first presented frame whose built tree rows hold the schema's TABLES group header with its count and at least one table row";

pub const USAGE: &str = "usage: catalog_cold --fixture <cache-file> --offline [--runs N] [--variant plain|encrypted|both] [--key-delay-ms N] [--any-fixture] [--allow-inactive]
  write the fixture: HACIVAT_G1_FIXTURE=$PWD/target/bench/g1-160k.v1.sqlite cargo test --release -p hacivat-meta --test bench_fixture -- --ignored write_g1_fixture";

// ---- arguments ---------------------------------------------------------------------------------

/// Which cache the profile has.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Variant {
    /// A development profile, plaintext cache (graded).
    Plain,
    /// A production profile, row-encrypted cache, key from the secret store (recorded only).
    Encrypted,
}

impl Variant {
    pub fn label(self) -> &'static str {
        match self {
            Variant::Plain => LABEL,
            Variant::Encrypted => LABEL_ENCRYPTED,
        }
    }

    pub fn arg(self) -> &'static str {
        match self {
            Variant::Plain => "plain",
            Variant::Encrypted => "encrypted",
        }
    }

    fn parse(s: &str) -> Result<Variant, String> {
        match s {
            "plain" => Ok(Variant::Plain),
            "encrypted" => Ok(Variant::Encrypted),
            other => Err(format!("unknown variant \"{other}\"")),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Args {
    pub fixture: PathBuf,
    pub runs: usize,
    pub variants: Vec<Variant>,
    /// A fixed delay added to every `CatalogKey` read (a slow keyring).
    pub key_delay: Duration,
    /// Run on a fixture smaller than the stated scale (ungraded).
    pub any_fixture: bool,
    pub allow_inactive: bool,
    /// Internal: this process is one run of `variant`.
    pub child: Option<Variant>,
}

pub fn parse_args(args: &[String]) -> Result<Args, String> {
    let fixture = flag(args, "--fixture")
        .map(PathBuf::from)
        .ok_or("missing --fixture <cache-file>")?;
    if !args.iter().any(|a| a == "--offline") {
        return Err("the G1 bench runs offline only (the server unreachable); pass --offline".into());
    }
    let runs = match flag(args, "--runs") {
        Some(n) => n.parse::<usize>().map_err(|_| format!("bad --runs {n}"))?,
        None => 10,
    };
    if runs == 0 {
        return Err("--runs must be at least 1".into());
    }
    let variants = match flag(args, "--variant").unwrap_or("both") {
        "both" => vec![Variant::Encrypted, Variant::Plain],
        v => vec![Variant::parse(v)?],
    };
    let key_delay = match flag(args, "--key-delay-ms") {
        Some(n) => Duration::from_millis(
            n.parse::<u64>()
                .map_err(|_| format!("bad --key-delay-ms {n}"))?,
        ),
        None => Duration::ZERO,
    };
    let child = flag(args, "--child").map(Variant::parse).transpose()?;
    Ok(Args {
        fixture,
        runs,
        variants,
        key_delay,
        any_fixture: args.iter().any(|a| a == "--any-fixture"),
        allow_inactive: args.iter().any(|a| a == "--allow-inactive"),
        child,
    })
}

// ---- the fixture ---------------------------------------------------------------------------------

/// What a fixture cache holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FixtureShape {
    pub namespaces: usize,
    pub tables: usize,
    pub columns: usize,
    pub bytes: u64,
}

impl FixtureShape {
    /// At least the stated scale (review probe: a fixture smaller than stated).
    pub fn is_full_scale(&self) -> bool {
        self.tables >= TABLES && self.columns >= COLUMNS
    }

    fn json(&self) -> Value {
        json!({"namespaces": self.namespaces, "tables": self.tables, "columns": self.columns, "bytes": self.bytes})
    }
}

/// A cache file and its WAL (copied together; the shared-memory file is rebuilt).
fn copy_cache(from: &Path, to: &Path) -> anyhow::Result<()> {
    if let Some(dir) = to.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    std::fs::copy(from, to)
        .with_context(|| format!("copying {} to {}", from.display(), to.display()))?;
    let wal = |p: &Path| PathBuf::from(format!("{}-wal", p.display()));
    if wal(from).exists() {
        std::fs::copy(wal(from), wal(to)).context("copying the cache's WAL")?;
    }
    Ok(())
}

fn load_cache(path: &Path, encryption: Encryption) -> anyhow::Result<Vec<NamespaceSnapshot>> {
    let opened = CacheStore::open(path, encryption, WriterId(1))
        .map_err(|e| anyhow!("opening {}: {e:?}", path.display()))?;
    if let Some(reason) = opened.rebuilt {
        bail!("{} is not a readable catalog cache ({reason:?})", path.display());
    }
    let mut store = opened.store;
    let all = store
        .load_all()
        .map_err(|e| anyhow!("reading {}: {e:?}", path.display()))?;
    if !all.dropped.is_empty() {
        bail!("{} has {} unreadable rows", path.display(), all.dropped.len());
    }
    Ok(all.namespaces)
}

/// Copies `fixture` into `work` and reads its shape there (the user's file is never opened).
/// With `sealed`, also writes the row-encrypted copy the encrypted variant loads.
pub fn prepare_fixture(
    fixture: &Path,
    work: &Path,
    sealed: Option<&Path>,
) -> anyhow::Result<FixtureShape> {
    let bytes = std::fs::metadata(fixture)
        .with_context(|| format!("reading {}", fixture.display()))?
        .len();
    let copy = work.join("shape.v1.sqlite");
    copy_cache(fixture, &copy)?;
    let nss = load_cache(&copy, Encryption::None)?;
    let shape = FixtureShape {
        namespaces: nss.len(),
        tables: nss
            .iter()
            .map(|n| n.objs().iter().filter(|o| o.kind() == ObjectKind::Table).count())
            .sum(),
        columns: nss.iter().map(NamespaceSnapshot::column_count).sum(),
        bytes,
    };
    if let Some(sealed) = sealed {
        let mut store = CacheStore::open(
            sealed,
            Encryption::Row(RowKey::from_bytes(&BENCH_KEY)),
            WriterId(1),
        )
        .map_err(|e| anyhow!("opening {}: {e:?}", sealed.display()))?
        .store;
        for ns in &nss {
            store
                .save_namespace(ns)
                .map_err(|e| anyhow!("sealing {:?}: {e:?}", ns.key()))?;
        }
    }
    Ok(shape)
}

// ---- one run's app -------------------------------------------------------------------------------

/// The bench profile of `variant`: a development profile (plaintext cache) or a production one
/// (`catalog_sensitive`: encrypted cache). Its server is unreachable and never dialled.
pub fn profile(variant: Variant) -> Profile {
    Profile {
        name: "g1-bench".to_string(),
        engine: DbmsKind::Postgres,
        endpoint: ProfileEndpoint::Server {
            host: "127.0.0.1".to_string(),
            port: Some(9),
        },
        user: Some("bench".to_string()),
        auth: AuthProviderId::None,
        database: Some(DB.to_string()),
        environment: Some(match variant {
            Variant::Plain => EnvTag::Dev,
            Variant::Encrypted => EnvTag::Production,
        }),
        ..Profile::default()
    }
}

/// The schema of the fixture.
pub fn schema_ns() -> NsKey {
    NsKey::schema([DB, SCHEMA])
}

/// The rows the restored workspace shows expanded: the profile, its database, the schema and
/// the schema's `TABLES` group.
pub fn expanded_keys(id: ProfileId) -> Vec<TreeKey> {
    let schema = adapter::namespace_key(id, &schema_ns());
    vec![
        adapter::profile_key(id),
        adapter::database_key(id, DB),
        schema,
        adapter::group_key(schema, DisplayGroup::Tables),
    ]
}

/// A `workspace.json` with one console, the explorer visible and `keys` of profile `id`
/// expanded (the app restores it at start, as a user's).
fn save_layout(root: &BenchRoot, id: ProfileId, keys: &[TreeKey]) -> anyhow::Result<()> {
    let tiles = TileLayout {
        explorer_visible: true,
        ..TileLayout::default()
    };
    let hex: Vec<String> = keys.iter().map(|k| format!("{:032x}", k.0)).collect();
    let mut expanded = serde_json::Map::new();
    expanded.insert(id.0.to_string(), json!(hex));
    let value = json!({
        "tiles": serde_json::to_value(tiles)?,
        "panes": {"root": {"Leaf": 0}, "panes": {"0": {"tabs": [0], "active": 0}}, "focused": 0},
        "tabs": [{"id": 0, "kind": "console", "n": 1}],
        "explorer": {"expanded": expanded},
    });
    workspace_state::save(
        &root.paths().workspace_json(),
        WORKSPACE_STATE_VERSION,
        &value,
    )
    .context("saving workspace.json")
}

/// The memory secret store's backend, timing every `CatalogKey` read (with `delay` added: a
/// slow keyring). Other slots pass through.
pub struct TimedBackend {
    inner: MemoryBackend,
    delay: Duration,
    key_us: AtomicU64,
}

impl TimedBackend {
    pub fn new(delay: Duration) -> TimedBackend {
        TimedBackend {
            inner: MemoryBackend::new(),
            delay,
            key_us: AtomicU64::new(0),
        }
    }

    /// Time spent reading the `CatalogKey` so far.
    pub fn key_fetch(&self) -> Duration {
        Duration::from_micros(self.key_us.load(Ordering::SeqCst))
    }

    /// Stores the bench's row key as `profile`'s `CatalogKey`.
    fn store_bench_key(&self, profile: ProfileId) -> anyhow::Result<()> {
        let hex: String = BENCH_KEY.iter().map(|b| format!("{b:02x}")).collect();
        self.inner
            .put(
                &SecretKey {
                    profile,
                    slot: SecretSlot::CatalogKey,
                },
                &SecretString::from(hex),
            )
            .map_err(|e| anyhow!("storing the bench key: {e:?}"))
    }
}

impl SecretBackend for TimedBackend {
    fn get(&self, key: &SecretKey) -> Result<Option<SecretString>, SecretError> {
        if !matches!(key.slot, SecretSlot::CatalogKey) {
            return self.inner.get(key);
        }
        let started = Instant::now();
        std::thread::sleep(self.delay);
        let got = self.inner.get(key);
        let us = u64::try_from(started.elapsed().as_micros()).unwrap_or(u64::MAX);
        self.key_us.fetch_add(us, Ordering::SeqCst);
        got
    }

    fn put(&self, key: &SecretKey, value: &SecretString) -> Result<(), SecretError> {
        self.inner.put(key, value)
    }

    fn delete(&self, key: &SecretKey) -> Result<(), SecretError> {
        self.inner.delete(key)
    }

    fn name(&self) -> &'static str {
        self.inner.name()
    }

    fn choice(&self) -> Option<hacivat_secrets::BackendChoice> {
        self.inner.choice()
    }
}

/// When the host asked the source for the catalog (t0) and when the source answered.
#[derive(Debug, Default)]
pub struct Timings {
    t0: Mutex<Option<Instant>>,
    ensured: Mutex<Option<Instant>>,
}

impl Timings {
    fn mark(slot: &Mutex<Option<Instant>>) {
        slot.lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get_or_insert_with(Instant::now);
    }

    fn get(slot: &Mutex<Option<Instant>>) -> Option<Instant> {
        *slot.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub fn t0(&self) -> Option<Instant> {
        Self::get(&self.t0)
    }

    pub fn ensured(&self) -> Option<Instant> {
        Self::get(&self.ensured)
    }
}

/// The app's own source (the hub over the app's sessions, secrets and cache folder, exactly as
/// `live::install` builds it), stamping t0 when the host asks for a catalog.
pub struct TimedSource {
    inner: HubSource,
    timings: Arc<Timings>,
}

impl CatalogSource for TimedSource {
    fn ensure(&self, profile: Arc<Profile>) -> BoxFuture<'_, Result<ProfileCatalog, HubError>> {
        Timings::mark(&self.timings.t0);
        let made = self.inner.ensure(profile);
        let timings = self.timings.clone();
        Box::pin(async move {
            let answer = made.await;
            Timings::mark(&timings.ensured);
            answer
        })
    }

    fn shutdown(&self, profile: ProfileId) -> BoxFuture<'_, ()> {
        self.inner.shutdown(profile)
    }

    fn has_cache(&self, profile: ProfileId) -> bool {
        self.inner.has_cache(profile)
    }
}

/// One run's prepared folder: the profile, its cache file and the restored layout.
pub struct Prepared {
    pub root: BenchRoot,
    pub profile: ProfileId,
    pub backend: Arc<TimedBackend>,
    pub timings: Arc<Timings>,
}

/// Prepares one run of `variant` over `cache` (the plaintext fixture, or the sealed copy for
/// the encrypted variant): a fresh root, the profile, the cache file in the profile's place,
/// the restored layout and the secret store's backend.
pub fn prepare(variant: Variant, cache: &Path, key_delay: Duration) -> anyhow::Result<Prepared> {
    let root = BenchRoot::new("g1")?;
    let p = profile(variant);
    let id = p.id;
    let done = (|| {
        super::save_profile(&root, &[p])?;
        copy_cache(
            cache,
            &root
                .paths()
                .catalog_db(&id.0.to_string(), u32::from(FORMAT_MAJOR)),
        )?;
        save_layout(&root, id, &expanded_keys(id))
    })();
    if let Err(e) = done {
        root.remove();
        return Err(e);
    }
    let backend = Arc::new(TimedBackend::new(key_delay));
    if variant == Variant::Encrypted {
        backend.store_bench_key(id)?;
    }
    Ok(Prepared {
        root,
        profile: id,
        backend,
        timings: Arc::default(),
    })
}

/// Starts the app (`crate::start`) on `prep`'s root, its secret store over `prep.backend`, then
/// replaces the explorer's source with a [`TimedSource`] before any of its tasks ran (the
/// workspace's `live::start` asks the source again once it found the cache file).
pub fn boot(cx: &mut App, prep: &Prepared) -> anyhow::Result<WindowHandle<Workspace>> {
    let paths = prep.root.paths().clone();
    let mut outcome = hacivat_settings::load(&paths.settings_toml());
    outcome.settings.ui.language = LanguagePref::Tag("en".to_string());
    let os = OsLocale {
        languages: vec!["en-US".to_string()],
        format: Some("en-US".to_string()),
        time_zone: Some("UTC".to_string()),
        env: Vec::new(),
    };
    let mut startup = Startup::new(paths.clone(), outcome, os);
    let mut connections = ConnectionsBoot::for_tests(&paths, fake_registry(fake_pg(Vec::new())));
    let backend: Arc<dyn SecretBackend> = prep.backend.clone();
    connections.secrets = Arc::new(SecretStore::new(Some(backend)));
    startup.connections = Some(connections);
    let ws = start(cx, startup, None)?;
    let state = cx.try_global::<AppState>().context("no app state")?;
    let model = cx
        .try_global::<ConnectionsModel>()
        .context("no connections")?;
    let settings = HubSettings {
        paths: paths.clone(),
        metadata: state.settings.metadata,
    };
    let sessions: Arc<dyn SessionLookup> = model.manager().clone();
    let hub = CatalogHub::new_on(
        model.runtime(),
        sessions,
        settings,
        model.secrets().clone(),
        Arc::new(hacivat_meta::system::SystemCatalogs::from_embedded()),
    );
    live::set_source(
        Arc::new(TimedSource {
            inner: HubSource { hub, paths },
            timings: prep.timings.clone(),
        }),
        cx,
    );
    Ok(ws)
}

/// What the explorer painted of the schema.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Painted {
    /// The `TABLES` header's count, as shown.
    pub tables_count: String,
    /// Table rows built in the frame.
    pub table_rows: usize,
}

/// The `TABLES` group of `app.public` and its table rows, if the tree's last frame built them.
pub fn painted(id: ProfileId, cx: &App) -> Option<Painted> {
    let tree = crate::connections::explorer::tree(cx)?;
    let tree = tree.read(cx);
    let model = tree.model();
    let group = adapter::group_key(adapter::namespace_key(id, &schema_ns()), DisplayGroup::Tables);
    let built = tree.flat().rows().get(tree.built_rows())?;
    let at = built
        .iter()
        .position(|r| r.key == group && r.kind == RowKind::Node)?;
    let count = model.row(group)?.meta?.to_string();
    if count.is_empty() {
        return None;
    }
    let depth = built[at].depth + 1;
    let table_rows = built[at + 1..]
        .iter()
        .take_while(|r| r.depth >= depth)
        .filter(|r| r.depth == depth && r.kind == RowKind::Node)
        .filter(|r| {
            matches!(model.info(r.key), Some(NodeInfo::Object { key, .. }) if key.kind == ObjectKind::Table)
        })
        .count();
    (table_rows > 0).then_some(Painted {
        tables_count: count,
        table_rows,
    })
}

/// One run's watch over the app: samples when the explorer got the snapshot and, after each
/// presented frame, whether t1 was reached.
pub struct Watch {
    pub variant: Variant,
    pub profile: ProfileId,
    pub timings: Arc<Timings>,
    pub backend: Arc<TimedBackend>,
    /// The process start (the example's `main`).
    pub started: Instant,
    seen: Cell<Option<Instant>>,
    done: Cell<bool>,
}

impl Watch {
    pub fn new(variant: Variant, prep: &Prepared, started: Instant) -> Watch {
        Watch {
            variant,
            profile: prep.profile,
            timings: prep.timings.clone(),
            backend: prep.backend.clone(),
            started,
            seen: Cell::new(None),
            done: Cell::new(false),
        }
    }

    pub fn finished(&self) -> bool {
        self.done.get()
    }

    /// Notes when the explorer first holds the schema's cached namespace.
    pub fn sample(&self, cx: &App) {
        if self.seen.get().is_none()
            && adapter::catalog_of(self.profile, cx)
                .is_some_and(|c| c.namespaces.contains_key(&schema_ns()))
        {
            self.seen.set(Some(Instant::now()));
        }
    }

    /// Called after a frame was drawn and presented: the run's record once t1 is reached (once).
    pub fn check(&self, window: &Window, cx: &App) -> Option<Value> {
        if self.done.get() {
            return None;
        }
        self.sample(cx);
        let shown = painted(self.profile, cx)?;
        let t1 = Instant::now();
        let t0 = self.timings.t0()?;
        let ensured = self.timings.ensured().unwrap_or(t1);
        let seen = self.seen.get().unwrap_or(t1);
        self.done.set(true);
        let ms = |later: Instant, earlier: Instant| {
            later.saturating_duration_since(earlier).as_secs_f64() * 1000.0
        };
        let key = self.backend.key_fetch().as_secs_f64() * 1000.0;
        let run = stamp_run(window, cx);
        let size = window.viewport_size();
        Some(json!({
            "variant": self.variant.arg(),
            "t0_after_start_ms": ms(t0, self.started),
            "t1_ms": ms(t1, t0),
            "key_fetch_ms": key,
            "cache_load_ms": (ms(ensured, t0) - key).max(0.0),
            "host_ms": ms(seen, ensured),
            "paint_ms": ms(t1, seen),
            "process_to_t1_ms": ms(t1, self.started),
            "rss_bytes": rss_bytes(),
            "active": window.is_window_active(),
            "display_hz": run.display_hz,
            "display_scale": run.display_scale,
            "viewport": [f32::from(size.width), f32::from(size.height)],
            "tables_count": shown.tables_count,
            "table_rows": shown.table_rows,
        }))
    }
}

/// Looks after every frame until the watch is done: the `on_next_frame` callback samples, and a
/// task it spawns runs on the next main-thread turn, after the frame's draw and present.
fn watch_frames(
    window: &mut Window,
    ws: WindowHandle<Workspace>,
    watch: Rc<Watch>,
    root: BenchRoot,
) {
    window.on_next_frame(move |window, cx| {
        if watch.finished() {
            return;
        }
        watch.sample(cx);
        let (w, r) = (watch.clone(), root.clone());
        cx.spawn(async move |cx| {
            let record = ws
                .update(cx, |_, window, cx| w.check(window, cx))
                .ok()
                .flatten();
            if let Some(record) = record {
                println!("{RUN_LINE}{record}");
                cx.update(|cx| end_run(cx, &r, 0));
            }
        })
        .detach();
        watch_frames(window, ws, watch, root);
    });
}

/// One cold run (a child process): prints one `G1RUN {json}` line and exits.
pub fn run_child(args: &Args, variant: Variant, started: Instant) -> ExitCode {
    let prep = match prepare(variant, &args.fixture, args.key_delay) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("catalog_cold: {e:#}");
            return ExitCode::FAILURE;
        }
    };
    run_app(Boot::default(), move |cx| {
        let ws = match boot(cx, &prep) {
            Ok(ws) => ws,
            Err(e) => {
                eprintln!("catalog_cold: opening the workspace failed: {e:#}");
                end_run(cx, &prep.root, 1);
            }
        };
        let watch = Rc::new(Watch::new(variant, &prep, started));
        let root = prep.root.clone();
        let _ = ws.update(cx, |_, window, cx| {
            window.activate_window();
            cx.activate(true);
            watch_frames(window, ws, watch, root);
        });
        let exec = cx.background_executor().clone();
        let root = prep.root.clone();
        cx.spawn(async move |cx| {
            exec.timer(RUN_TIMEOUT).await;
            eprintln!("catalog_cold: the cached tree was not painted within {RUN_TIMEOUT:?}");
            cx.update(|cx| end_run(cx, &root, 1));
        })
        .detach();
    });
    ExitCode::SUCCESS
}

// ---- the parent ----------------------------------------------------------------------------------

/// One child process of `variant` over `cache`; its record.
fn spawn_child(args: &Args, variant: Variant, cache: &Path) -> anyhow::Result<Value> {
    let exe = std::env::current_exe().context("finding the bench's executable")?;
    let out = std::process::Command::new(exe)
        .arg("--child")
        .arg(variant.arg())
        .arg("--fixture")
        .arg(cache)
        .arg("--offline")
        .arg("--key-delay-ms")
        .arg(args.key_delay.as_millis().to_string())
        .stderr(std::process::Stdio::inherit())
        .output()
        .context("starting a run")?;
    let stdout = String::from_utf8_lossy(&out.stdout);
    let line = stdout
        .lines()
        .find_map(|l| l.strip_prefix(RUN_LINE))
        .with_context(|| format!("the run printed no result ({})", out.status))?;
    serde_json::from_str(line).context("reading a run's result")
}

fn field(records: &[Value], name: &str) -> Vec<f64> {
    records.iter().filter_map(|r| r[name].as_f64()).collect()
}

/// Writes `variant`'s dump into `dir` in the shared bench schema (`kind: latency`, one value per
/// run: `t1 − t0`) extended with the G1 fields; its path, outcome and figures. The plaintext
/// variant is graded (p95 < [`BUDGET_MS`]); the encrypted one never is.
pub fn report(
    variant: Variant,
    records: &[Value],
    runs: usize,
    shape: &FixtureShape,
    dir: &Path,
    allow_inactive: bool,
) -> anyhow::Result<(PathBuf, Outcome, Figures)> {
    let values = field(records, "t1_ms");
    let mut probe = LatencyProbe::new(variant.label());
    let base = Instant::now();
    for v in &values {
        probe.mark_input(base);
        probe.on_frame(base + Duration::from_secs_f64(v / 1000.0));
    }
    let inactive = records
        .iter()
        .filter(|r| r["active"].as_bool() != Some(true))
        .count();
    let first = records.first().cloned().unwrap_or(Value::Null);
    let run = RunInfo {
        expected_samples: runs,
        inactive_keys: inactive,
        painted_frames: None,
        aborted: records.len() < runs,
        build: build_stamp(&workspace_root()),
        display_hz: first["display_hz"].as_f64(),
        display_scale: first["display_scale"].as_f64().map(|s| s as f32),
    };
    let path = probe
        .dump_json(dir, std::env::consts::OS, &run)
        .context("writing the dump")?;
    let figures = Figures::of(probe.samples_ms());
    let outcome = if inactive > 0 && !allow_inactive {
        Outcome::Invalid
    } else if variant == Variant::Encrypted || !shape.is_full_scale() {
        Outcome::NotGraded
    } else if run.aborted || figures.p95 >= BUDGET_MS {
        Outcome::Fail
    } else {
        Outcome::Pass
    };
    let p95 = |name: &str| Figures::of(&field(records, name)).p95;
    let mut dump: Value =
        serde_json::from_str(&std::fs::read_to_string(&path).context("reading the dump")?)
            .context("parsing the dump")?;
    let extra = json!({
        "gate": "G1",
        "variant": variant.arg(),
        "budget_ms": BUDGET_MS,
        "graded": variant == Variant::Plain && shape.is_full_scale(),
        "outcome": outcome.text(),
        "t0": T0_DEF,
        "t1": T1_DEF,
        "conditions": "cold process, warm disk; profile expanded in the restored workspace; server unreachable (--offline); memory secret store",
        "fixture": shape.json(),
        "figures": figures.json(),
        "phases_p95": {
            "key_fetch_ms": p95("key_fetch_ms"),
            "cache_load_ms": p95("cache_load_ms"),
            "host_ms": p95("host_ms"),
            "paint_ms": p95("paint_ms"),
            "process_to_t1_ms": p95("process_to_t1_ms"),
        },
        "rss_bytes_p50": Figures::of(&field(records, "rss_bytes")).p50,
        "runs": records,
    });
    if let (Some(d), Some(e)) = (dump.as_object_mut(), extra.as_object()) {
        for (k, v) in e {
            d.insert(k.clone(), v.clone());
        }
    }
    std::fs::write(&path, format!("{dump}\n")).context("writing the dump")?;
    Ok((path, outcome, figures))
}

/// The bench: checks the fixture, runs `--runs` cold child processes per variant, writes the
/// dumps and prints the result (the plaintext variant's `RESULT` line is the last one).
pub fn run(args: &Args) -> ExitCode {
    let work = match BenchRoot::new("g1-prep") {
        Ok(r) => r,
        Err(e) => {
            eprintln!("catalog_cold: {e:#}");
            return ExitCode::FAILURE;
        }
    };
    let code = run_in(args, &work);
    work.remove();
    ExitCode::from(u8::try_from(code).unwrap_or(1))
}

fn run_in(args: &Args, work: &BenchRoot) -> i32 {
    let sealed = work.dir().join("sealed.v1.sqlite");
    let wants_sealed = args.variants.contains(&Variant::Encrypted);
    let shape = match prepare_fixture(
        &args.fixture,
        work.dir(),
        wants_sealed.then_some(sealed.as_path()),
    ) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("catalog_cold: {e:#}\n{USAGE}");
            return i32::from(EXIT_USAGE);
        }
    };
    println!(
        "catalog_cold: fixture {} ({} namespaces, {} tables, {} columns, {:.1} MB)",
        args.fixture.display(),
        shape.namespaces,
        shape.tables,
        shape.columns,
        shape.bytes as f64 / 1e6
    );
    if !shape.is_full_scale() {
        if !args.any_fixture {
            println!(
                "INVALID: the fixture is smaller than {TABLES} tables / {COLUMNS} columns; no JSON written"
            );
            print_result(Outcome::Invalid, LABEL);
            return EXIT_INVALID;
        }
        eprintln!("warning: a fixture below the stated scale (--any-fixture): ungraded");
    }
    let mut last = Outcome::NotGraded;
    let mut code = 0;
    for &variant in &args.variants {
        let cache = match variant {
            Variant::Plain => args.fixture.as_path(),
            Variant::Encrypted => sealed.as_path(),
        };
        let mut records = Vec::new();
        for i in 0..args.runs {
            match spawn_child(args, variant, cache) {
                Ok(r) => {
                    println!(
                        "{} run {}/{}: t1-t0 {:.1} ms (key {:.1}, load {:.1}, host {:.1}, paint {:.1}); process start to t1 {:.1} ms",
                        variant.label(),
                        i + 1,
                        args.runs,
                        r["t1_ms"].as_f64().unwrap_or(f64::NAN),
                        r["key_fetch_ms"].as_f64().unwrap_or(f64::NAN),
                        r["cache_load_ms"].as_f64().unwrap_or(f64::NAN),
                        r["host_ms"].as_f64().unwrap_or(f64::NAN),
                        r["paint_ms"].as_f64().unwrap_or(f64::NAN),
                        r["process_to_t1_ms"].as_f64().unwrap_or(f64::NAN),
                    );
                    records.push(r);
                }
                Err(e) => eprintln!("{} run {}: {e:#}", variant.label(), i + 1),
            }
        }
        let (path, outcome, figures) = match report(
            variant,
            &records,
            args.runs,
            &shape,
            &bench_dir(),
            args.allow_inactive,
        ) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("{}: {e:#}", variant.label());
                print_result(Outcome::Fail, variant.label());
                return 1;
            }
        };
        println!("{} wrote {}", variant.label(), path.display());
        println!(
            "{}: t1-t0 {} (budget p95 < {BUDGET_MS} ms{})",
            variant.label(),
            figures.text(1),
            if variant == Variant::Encrypted {
                "; recorded, not graded: over budget is a PO item"
            } else {
                ""
            }
        );
        if outcome == Outcome::Invalid {
            println!("INVALID: a run's window was not frontmost at t1. {PENDING_PO}");
        }
        print_result(outcome, variant.label());
        code = code.max(outcome.exit_code(args.allow_inactive));
        last = outcome;
    }
    if last == Outcome::Invalid {
        return EXIT_INVALID;
    }
    code
}

#[cfg(test)]
mod tests {
    use super::*;

    use gpui::{TestAppContext, VisualTestContext};
    use hacivat_meta::name::FoldRule;
    use hacivat_meta::namespace::{Level, NsMeta, Obj, Subtree};
    use hacivat_meta::props::{ColumnProps, Props, TableProps};
    use hacivat_meta::ServerId;

    /// A small catalog in the fixture's place: `tables` tables of two columns in `app.public`.
    fn small_catalog(tables: usize) -> NamespaceSnapshot {
        let mut t = Subtree::new(Obj::new(ObjectKind::Schema, SCHEMA));
        for i in 0..tables {
            let table = t.add(
                t.root(),
                Obj::new(ObjectKind::Table, format!("tbl_{i:03}").as_str())
                    .with_server_id(ServerId::Oid(20_000 + i as u32))
                    .with_props(Props::Table(Arc::new(TableProps::default()))),
            );
            for (k, c) in ["id", "name"].iter().enumerate() {
                t.add(
                    table,
                    Obj::new(ObjectKind::Column, *c).with_props(Props::Column(Arc::new(
                        ColumnProps {
                            ordinal: k as u32 + 1,
                            raw_type: Arc::from("integer"),
                            ..Default::default()
                        },
                    ))),
                );
            }
        }
        let meta = NsMeta {
            level: Level::Structure,
            ..NsMeta::default()
        };
        NamespaceSnapshot::build(schema_ns(), DbmsKind::Postgres, FoldRule::Exact, meta, t)
            .expect("a valid namespace")
    }

    fn write_fixture(path: &Path, tables: usize) {
        let mut store = CacheStore::open(path, Encryption::None, WriterId(1))
            .unwrap()
            .store;
        store.save_namespace(&small_catalog(tables)).unwrap();
    }

    /// One headless run of `variant`: the real app booted as a child boots, frames drawn by the
    /// test (a refresh and a park stand for a presented frame), the watch checked after each.
    fn headless_run(cx: &mut TestAppContext, variant: Variant, delay: Duration) -> Value {
        let dir = tempfile::tempdir().unwrap();
        let fixture = dir.path().join("fixture.v1.sqlite");
        write_fixture(&fixture, 40);
        let sealed = dir.path().join("sealed.v1.sqlite");
        let shape = prepare_fixture(&fixture, dir.path(), Some(&sealed)).unwrap();
        assert_eq!((shape.tables, shape.columns), (40, 80));
        assert!(!shape.is_full_scale());
        let cache = match variant {
            Variant::Plain => fixture.as_path(),
            Variant::Encrypted => sealed.as_path(),
        };
        let started = Instant::now();
        let prep = prepare(variant, cache, delay).unwrap();
        let ws = cx.update(|cx| boot(cx, &prep)).unwrap();
        cx.executor().allow_parking();
        let vcx = VisualTestContext::from_window(ws.into(), cx).into_mut();
        let watch = Watch::new(variant, &prep, started);
        let deadline = Instant::now() + Duration::from_secs(60);
        let record = loop {
            vcx.executor().allow_parking();
            vcx.update(|window, _| window.refresh());
            vcx.run_until_parked();
            vcx.cx.update(|cx| watch.sample(cx));
            if let Some(r) = ws.update(vcx, |_, window, cx| watch.check(window, cx)).unwrap() {
                break r;
            }
            assert!(Instant::now() < deadline, "the cached tree was not painted");
            std::thread::sleep(Duration::from_millis(2));
            vcx.executor().advance_clock(Duration::from_millis(16));
        };
        // t1 is taken only once the rows were built in a frame.
        let shown = vcx.cx.read(|cx| painted(prep.profile, cx)).expect("painted");
        assert_eq!(shown.tables_count, "40");
        assert!(shown.table_rows > 0);
        assert!(watch.finished());
        let mode = vcx.cx.read(|cx| live::cache_mode(prep.profile, cx));
        assert!(
            matches!(
                mode,
                Some(hacivat_introspect::hub::CacheMode::Persistent { encrypted, rebuilt: None })
                    if encrypted == (variant == Variant::Encrypted)
            ),
            "{mode:?}"
        );
        vcx.cx.update(|cx| {
            cx.global::<ConnectionsModel>()
                .core_runtime()
                .shutdown(Duration::from_secs(10));
        });
        vcx.run_until_parked();
        prep.root.remove();
        drop(dir);
        record
    }

    fn assert_phases(r: &Value) {
        for k in [
            "t0_after_start_ms",
            "t1_ms",
            "key_fetch_ms",
            "cache_load_ms",
            "host_ms",
            "paint_ms",
            "process_to_t1_ms",
        ] {
            let v = r[k].as_f64().unwrap_or_else(|| panic!("{k} in {r}"));
            assert!(v >= 0.0, "{k} = {v}");
        }
        let t1 = r["t1_ms"].as_f64().unwrap();
        let parts = ["key_fetch_ms", "cache_load_ms", "host_ms", "paint_ms"]
            .iter()
            .map(|k| r[*k].as_f64().unwrap())
            .sum::<f64>();
        // The phases split t0..t1 (the key fetch is inside `ensure`).
        assert!((parts - t1).abs() < 0.5, "{parts} vs {t1}: {r}");
        assert!(r["process_to_t1_ms"].as_f64().unwrap() >= t1);
    }

    #[gpui::test]
    fn catalog_cold_bench_emits_json_with_t0_t1_and_phases(cx: &mut TestAppContext) {
        let r = headless_run(cx, Variant::Plain, Duration::ZERO);
        assert_eq!(r["variant"], "plain");
        assert_phases(&r);
        assert_eq!(r["key_fetch_ms"].as_f64(), Some(0.0), "no key for a plaintext cache");

        // The dump: the shared bench schema (what `xtask bench-report` reads) plus G1's fields.
        let out = tempfile::tempdir().unwrap();
        let shape = FixtureShape {
            namespaces: 1,
            tables: TABLES,
            columns: COLUMNS,
            bytes: 1,
        };
        let records = vec![r.clone(), r];
        let (path, outcome, figures) =
            report(Variant::Plain, &records, 2, &shape, out.path(), true).unwrap();
        let v: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        for k in ["label", "os", "host", "kind", "build"] {
            assert!(v[k].is_string(), "{k} in {v}");
        }
        assert_eq!(v["label"], LABEL);
        assert_eq!(v["kind"], "latency");
        assert_eq!(v["expected_samples"], 2);
        assert!(v["inactive_keys"].is_u64() && v["aborted"] == false);
        assert!(v["painted_frames"].is_null());
        assert!(v["display_hz"].is_number() || v["display_hz"] == "unknown");
        let values = v["values_ms"].as_array().unwrap();
        assert_eq!(values.len(), 2);
        let t1 = records[0]["t1_ms"].as_f64().unwrap();
        assert!((values[0].as_f64().unwrap() - t1).abs() < 0.01);
        assert_eq!(v["t0"], T0_DEF);
        assert_eq!(v["t1"], T1_DEF);
        assert_eq!(v["budget_ms"], BUDGET_MS);
        assert_eq!(v["gate"], "G1");
        assert_eq!(v["graded"], true);
        assert_eq!(v["runs"].as_array().unwrap().len(), 2);
        assert!(v["phases_p95"]["key_fetch_ms"].is_number());
        assert!(v["phases_p95"]["paint_ms"].is_number());
        assert_eq!(v["fixture"]["columns"], COLUMNS);
        assert_eq!(figures.n, 2);
        // Graded on the plaintext variant only (headless timings are not judged here: the
        // budget is the PO's Mac's, perf-gated below).
        assert!(matches!(outcome, Outcome::Pass | Outcome::Fail | Outcome::Invalid));
        if std::env::var_os("HACIVAT_PERF").is_some_and(|p| p == "1") {
            assert!(figures.p95 < BUDGET_MS, "{}", figures.text(1));
        }
        // A missing run is an aborted run, never a pass.
        let (_, short, _) = report(Variant::Plain, &records[..1], 2, &shape, out.path(), true).unwrap();
        assert_eq!(short, Outcome::Fail);
        // A fixture below the stated scale is never graded.
        let small = FixtureShape { tables: 40, ..shape };
        let (_, ungraded, _) = report(Variant::Plain, &records, 2, &small, out.path(), true).unwrap();
        assert_eq!(ungraded, Outcome::NotGraded);
    }

    #[gpui::test]
    fn catalog_cold_bench_encrypted_variant_reports_key_share(cx: &mut TestAppContext) {
        const DELAY: Duration = Duration::from_millis(120);
        let r = headless_run(cx, Variant::Encrypted, DELAY);
        assert_eq!(r["variant"], "encrypted");
        assert_phases(&r);
        let key = r["key_fetch_ms"].as_f64().unwrap();
        // The fixed delay is inside the key share, and the key share inside t0..t1.
        assert!(key >= DELAY.as_secs_f64() * 1000.0, "{r}");
        assert!(key <= r["t1_ms"].as_f64().unwrap(), "{r}");

        let out = tempfile::tempdir().unwrap();
        let shape = FixtureShape {
            namespaces: 1,
            tables: TABLES,
            columns: COLUMNS,
            bytes: 1,
        };
        let (path, outcome, _) =
            report(Variant::Encrypted, &[r], 1, &shape, out.path(), true).unwrap();
        // Recorded, never graded.
        assert_eq!(outcome, Outcome::NotGraded);
        let v: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(v["label"], LABEL_ENCRYPTED);
        assert_eq!(v["graded"], false);
        assert!(v["phases_p95"]["key_fetch_ms"].as_f64().unwrap() >= 120.0);
    }

    #[test]
    fn args_need_a_fixture_and_offline() {
        let a = |s: &[&str]| parse_args(&s.iter().map(|x| x.to_string()).collect::<Vec<_>>());
        assert!(a(&["--offline"]).unwrap_err().contains("--fixture"));
        assert!(a(&["--fixture", "f"]).unwrap_err().contains("--offline"));
        let ok = a(&["--fixture", "f", "--offline"]).unwrap();
        assert_eq!(ok.runs, 10);
        assert_eq!(ok.variants, vec![Variant::Encrypted, Variant::Plain]);
        assert_eq!(ok.child, None);
        let c = a(&[
            "--fixture", "f", "--offline", "--runs", "3", "--variant", "plain", "--child",
            "encrypted", "--key-delay-ms", "40",
        ])
        .unwrap();
        assert_eq!((c.runs, c.child), (3, Some(Variant::Encrypted)));
        assert_eq!(c.variants, vec![Variant::Plain]);
        assert_eq!(c.key_delay, Duration::from_millis(40));
        assert!(a(&["--fixture", "f", "--offline", "--runs", "0"]).is_err());
    }
}
