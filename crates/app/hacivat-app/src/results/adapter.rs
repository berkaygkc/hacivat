//! The grid adapter over a result store (Task 26): one `snapshot()` per frame, columns from
//! the `ResultSchema`, cells through `format_cell` and the display cache, `ensure_rows`
//! forwarded to the result's source.
//!
//! - **Two channels that never mix.** The grid shows the *display* channel
//!   (`valfmt::format_cell`: the format locale, the display zone, the grapheme cap); copy reads
//!   the *raw* channel from the same snapshot (`results::copy`). Nothing here hands display
//!   text to copy.
//! - **Frames.** `begin_frame` is one `ArcSwap` load; a cell is a display-cache lookup, and on
//!   a miss one `format_cell`. Nothing on this path locks, awaits or does I/O. The cache is
//!   keyed by the adapter's generation, never the store's version, so the appends of a
//!   streaming result leave it whole (`results::cache`).
//! - **Generations.** A store's own generation is fixed for its life, and a re-run is a new
//!   store, so the adapter counts its own: [`GridAdapter::retarget`] moves to the new store and
//!   bumps it. The grid sees a new generation and starts over (selection, scroll), keeping the
//!   widths when the column names match.
//! - **Changes.** The adapter subscribes to the source's `changes()` **before** it loads the
//!   first snapshot (a receiver counts the value at subscription as seen; the other order can
//!   miss a publish and then sleep on a finished result). [`GridAdapter::on_change`] runs a
//!   foreground task that awaits the `watch` and calls the listener at most once per painted
//!   frame: after a call it stays quiet until a frame has read the adapter again.
//! - **Text.** Column names and cell texts are data. The truncation chip and the decode-error
//!   text are Fluent, read from the installed runtime when a cell is formatted; a language
//!   switch empties the cache.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;

use hacivat_l10n::{Count, ExactDecimal, IntoFluentArgs, L10n, MsgRef, NoArgs, Text, ids};
use hacivat_proto::DbmsKind;
use hacivat_results::{Completeness, ResultSource, StoreSnapshot};
use hacivat_ui_core::gpui_surface::{App, SharedString, Task};
use hacivat_ui_grid::model::autosize::WidthClass;
use hacivat_ui_grid::{
    Align, CellContent, CellKind, ColId, ColumnKind, ColumnSpec, GridCellFlags, GridDataSource,
    GridFrame, ModelRow, NULL_MARKER, RowCount, SourceCaps,
};
use hacivat_values::valfmt::{
    BinaryDisplay, ChipKind, ColumnFacts, Display, DisplayKind, DisplaySettings, DisplayZone,
    Purpose, format_cell, id_like,
};
use hacivat_values::{
    CellFlags, ColumnMeta, LenUnit, LogicalType, NativeType, NetKind, ResultSchema, SqliteAffinity,
};
use tokio::sync::watch;

use crate::l10n_global::L10nGlobal;
use crate::results::cache::{DisplayCache, DisplayKey};

/// How a console's results are shown and copied: the display settings (format locale, display
/// zone, mode), the same zone for the raw channel (`raw_text_in`, so a copied `timestamptz`
/// reads as its cell does), and the console's engine.
#[derive(Debug, Clone, PartialEq)]
pub struct ResultDisplay {
    settings: DisplaySettings,
    zone: DisplayZone,
    engine: DbmsKind,
}

impl ResultDisplay {
    /// The display of a console on `engine`, for the format locale `region_bcp47` (the OS
    /// region, `tr-TR`) and a zone already loaded. No I/O: load the zone off the UI thread
    /// (`DisplayZone::load`).
    pub fn new(region_bcp47: &str, zone: DisplayZone, engine: DbmsKind) -> Self {
        ResultDisplay {
            settings: DisplaySettings::with_zone(region_bcp47, zone.clone()),
            zone,
            engine,
        }
    }

    pub fn settings(&self) -> &DisplaySettings {
        &self.settings
    }

    /// The display zone, for the raw channel.
    pub fn zone(&self) -> &DisplayZone {
        &self.zone
    }

    pub fn engine(&self) -> DbmsKind {
        self.engine
    }
}

/// What a column contributes to formatting and copying its cells, computed once per schema.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ColFacts {
    pub logical: LogicalType,
    /// The engine's bare type name (`numeric`), for the ID heuristic.
    pub native_name: Box<str>,
    pub id_like: bool,
    /// The column's alignment (a `NULL` takes it).
    pub align: Align,
}

impl ColFacts {
    pub(crate) fn view(&self, engine: DbmsKind) -> ColumnFacts<'_> {
        ColumnFacts {
            logical: &self.logical,
            native_name: &self.native_name,
            id_like: self.id_like,
            binary: BinaryDisplay::Detect,
            engine,
        }
    }
}

/// The result an adapter currently shows.
struct Target {
    source: Arc<dyn ResultSource>,
    /// Subscribed before the first snapshot was loaded.
    changes: watch::Receiver<(u64, u64)>,
    columns: Arc<[ColumnSpec]>,
    facts: Arc<[ColFacts]>,
    /// The adapter's generation: 1 for its first result, one more per retarget.
    generation: u64,
}

impl Target {
    fn over(source: Arc<dyn ResultSource>, engine: DbmsKind, generation: u64) -> Target {
        // The order matters (module docs): subscribe, then load.
        let changes = source.changes();
        let snapshot = source.snapshot();
        let (columns, facts) = columns_of(snapshot.schema(), engine);
        Target {
            source,
            changes,
            columns,
            facts,
            generation,
        }
    }
}

/// What a copy reads: one snapshot, its columns' facts and the display (all `Send`).
#[derive(Clone)]
pub(crate) struct RawView {
    pub snapshot: Arc<StoreSnapshot>,
    pub facts: Arc<[ColFacts]>,
    pub display: Arc<ResultDisplay>,
}

/// What [`GridAdapter::on_change`] calls.
type Listener = Rc<dyn Fn(&mut App)>;

/// A result as a grid source.
pub struct GridAdapter {
    target: RefCell<Target>,
    display: RefCell<Arc<ResultDisplay>>,
    cache: Rc<RefCell<DisplayCache>>,
    /// The listener was called and no frame has read the adapter since.
    notified: Rc<Cell<bool>>,
    listener: RefCell<Option<Listener>>,
    watch: RefCell<Option<Task<()>>>,
    /// The localization runtime's generation the cached Fluent texts were rendered with.
    l10n_generation: Cell<Option<u64>>,
}

impl std::fmt::Debug for GridAdapter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let target = self.target.borrow();
        f.debug_struct("GridAdapter")
            .field("generation", &target.generation)
            .field("columns", &target.columns.len())
            .field("cached", &self.cache.borrow().len())
            .finish_non_exhaustive()
    }
}

impl GridAdapter {
    /// An adapter over `source`, shown as `display` says.
    pub fn new(source: Arc<dyn ResultSource>, display: ResultDisplay) -> Rc<Self> {
        let target = Target::over(source, display.engine, 1);
        Rc::new(GridAdapter {
            target: RefCell::new(target),
            display: RefCell::new(Arc::new(display)),
            cache: Rc::new(RefCell::new(DisplayCache::new())),
            notified: Rc::new(Cell::new(false)),
            listener: RefCell::new(None),
            watch: RefCell::new(None),
            l10n_generation: Cell::new(None),
        })
    }

    /// Calls `listener` when the result changes (rows, state, a retarget, new display
    /// settings): at most once per painted frame (module docs). The grid's host passes a
    /// closure that notifies the view showing this adapter. A second call replaces the first.
    pub fn on_change(&self, cx: &mut App, listener: impl Fn(&mut App) + 'static) {
        *self.listener.borrow_mut() = Some(Rc::new(listener));
        self.start_watch(cx);
    }

    /// Moves the adapter to `source` (a re-run into the same result tab): a new generation,
    /// new columns, and the listener is called.
    pub fn retarget(&self, source: Arc<dyn ResultSource>, cx: &mut App) {
        let engine = self.display.borrow().engine;
        let generation = self.target.borrow().generation + 1;
        *self.target.borrow_mut() = Target::over(source, engine, generation);
        self.start_watch(cx);
        self.tell(cx);
    }

    /// Shows and copies by `display` from now on. The cache keys on the settings' version,
    /// which this bumps, so no cell formatted under the old settings is shown again.
    pub fn set_display(&self, mut display: ResultDisplay, cx: &mut App) {
        let old = self.display.borrow().settings.version;
        display.settings.version = old.wrapping_add(1);
        let engine_changed = display.engine != self.display.borrow().engine;
        *self.display.borrow_mut() = Arc::new(display);
        if engine_changed {
            // The ID heuristic depends on the engine.
            let mut target = self.target.borrow_mut();
            let snapshot = target.source.snapshot();
            let engine = self.display.borrow().engine;
            (target.columns, target.facts) = columns_of(snapshot.schema(), engine);
        }
        self.tell(cx);
    }

    /// The result's source (the cursor strip's commands and state).
    pub fn source(&self) -> Arc<dyn ResultSource> {
        self.target.borrow().source.clone()
    }

    /// The current snapshot (one atomic load).
    pub fn snapshot(&self) -> Arc<StoreSnapshot> {
        self.target.borrow().source.snapshot()
    }

    /// The adapter's generation: 1 for its first result, one more per [`retarget`](Self::retarget).
    pub fn generation(&self) -> u64 {
        self.target.borrow().generation
    }

    pub fn display(&self) -> Arc<ResultDisplay> {
        self.display.borrow().clone()
    }

    /// Cells formatted so far (display-cache misses).
    pub fn format_calls(&self) -> u64 {
        self.cache.borrow().formats()
    }

    /// Cells the display cache holds.
    pub fn cached_cells(&self) -> usize {
        self.cache.borrow().len()
    }

    /// What a copy reads.
    pub(crate) fn raw_view(&self) -> RawView {
        let target = self.target.borrow();
        RawView {
            snapshot: target.source.snapshot(),
            facts: target.facts.clone(),
            display: self.display.borrow().clone(),
        }
    }

    /// Calls the listener now, unless it was called and no frame has read the adapter since.
    fn tell(&self, cx: &mut App) {
        let listener = self.listener.borrow().clone();
        if let Some(listener) = listener
            && !self.notified.replace(true)
        {
            listener(cx);
        }
    }

    /// (Re)starts the foreground task that awaits the target's `watch`.
    fn start_watch(&self, cx: &mut App) {
        let Some(listener) = self.listener.borrow().clone() else {
            return;
        };
        // A clone keeps what the first receiver has seen: a publish between the subscription
        // and now wakes the task at once.
        let mut changes = self.target.borrow().changes.clone();
        let notified = self.notified.clone();
        let task = cx.spawn(async move |cx| {
            // Awaiting a `tokio::sync` channel from a foreground task is allowed (Q4); the
            // loop ends when the store is gone.
            while changes.changed().await.is_ok() {
                if !notified.replace(true) {
                    cx.update(|cx| listener(cx));
                }
            }
        });
        *self.watch.borrow_mut() = Some(task);
    }
}

/// One frame's view of the result.
struct AdapterFrame {
    snapshot: Arc<StoreSnapshot>,
    facts: Arc<[ColFacts]>,
    display: Arc<ResultDisplay>,
    cache: Rc<RefCell<DisplayCache>>,
    generation: u64,
    texts: Texts,
}

impl GridFrame for AdapterFrame {
    fn generation(&self) -> (u64, u64) {
        (self.generation, self.snapshot.version())
    }

    fn row_count(&self) -> RowCount {
        let loaded = self.snapshot.row_count();
        let more = matches!(
            self.snapshot.completeness(),
            Completeness::Streaming | Completeness::Suspended(_)
        );
        RowCount {
            loaded,
            more,
            exact_total: (!more).then_some(loaded),
        }
    }

    fn cell(&self, row: ModelRow, col: ColId) -> CellContent {
        // The store panics for a row past its rows or a column outside its schema: those are
        // placeholders, decided before the read.
        let Some(facts) = self.facts.get(col.0 as usize) else {
            return placeholder();
        };
        if row.0 >= self.snapshot.row_count() {
            return placeholder();
        }
        let key = DisplayKey {
            generation: self.generation,
            row: row.0,
            col: col.0,
            settings_version: self.display.settings.version,
        };
        self.cache.borrow_mut().get_or_format(key, || {
            let value = self.snapshot.cell(row.0, col.0);
            let shown = format_cell(
                value,
                &facts.view(self.display.engine),
                &self.display.settings,
                Purpose::Cell,
            );
            cell_content(shown, facts, &self.texts)
        })
    }
}

impl GridDataSource for GridAdapter {
    fn begin_frame(&self, cx: &App) -> Rc<dyn GridFrame> {
        // A frame reads the adapter: the next change may call the listener again.
        self.notified.set(false);
        let l10n = cx.try_global::<L10nGlobal>().map(|g| g.l10n.clone());
        let generation = l10n.as_ref().map(|l| l.generation());
        let mut cache = self.cache.borrow_mut();
        if self.l10n_generation.replace(generation) != generation {
            // The chips and decode-error texts in the cache are in the old language.
            cache.clear();
        }
        cache.begin_frame();
        let target = self.target.borrow();
        Rc::new(AdapterFrame {
            snapshot: target.source.snapshot(),
            facts: target.facts.clone(),
            display: self.display.borrow().clone(),
            cache: self.cache.clone(),
            generation: target.generation,
            texts: Texts(l10n),
        })
    }

    fn columns(&self) -> Arc<[ColumnSpec]> {
        self.target.borrow().columns.clone()
    }

    fn caps(&self) -> SourceCaps {
        let caps = self.target.borrow().source.caps();
        SourceCaps {
            has_cursor: caps.has_cursor,
            editable: caps.editable,
        }
    }

    fn ensure_rows(&self, upto: u64, cx: &mut App) {
        // The source coalesces (one fetch per snapshot version while suspended).
        let _ = cx;
        self.target.borrow().source.ensure_rows(upto);
    }
}

/// A row that is not loaded, or a column the schema does not have.
fn placeholder() -> CellContent {
    CellContent {
        text: SharedString::new_static(""),
        kind: CellKind::Placeholder,
        flags: GridCellFlags::empty(),
        align: Align::Start,
        chip: None,
    }
}

/// The Fluent texts a formatted cell can carry, from the installed runtime; the message ID
/// when no localization is installed (as `l10n_global::tr`).
pub(crate) struct Texts(pub Option<Arc<L10n>>);

impl Texts {
    pub(crate) fn of(cx: &App) -> Texts {
        Texts(cx.try_global::<L10nGlobal>().map(|g| g.l10n.clone()))
    }

    pub(crate) fn tr<A: IntoFluentArgs>(&self, msg: MsgRef<A>, args: A) -> String {
        match &self.0 {
            Some(l10n) => l10n.t(msg, args),
            None => msg.id.to_string(),
        }
    }

    /// `bytes` as one message with its unit inside (`512 B`, `5.2 MB`), in the format locale.
    pub(crate) fn size(&self, bytes: u64) -> String {
        const KB: u64 = 1 << 10;
        const MB: u64 = 1 << 20;
        const GB: u64 = 1 << 30;
        if bytes < KB {
            self.tr(
                ids::hv::GRID_SIZE_BYTES,
                ids::hv::HvGridSizeBytes { n: Count(bytes) },
            )
        } else if bytes < MB {
            self.tr(
                ids::hv::GRID_SIZE_KB,
                ids::hv::HvGridSizeKb {
                    n: tenths(bytes, KB),
                },
            )
        } else if bytes < GB {
            self.tr(
                ids::hv::GRID_SIZE_MB,
                ids::hv::HvGridSizeMb {
                    n: tenths(bytes, MB),
                },
            )
        } else {
            self.tr(
                ids::hv::GRID_SIZE_GB,
                ids::hv::HvGridSizeGb {
                    n: tenths(bytes, GB),
                },
            )
        }
    }

    /// The chip after a truncated cell: the value's whole size.
    fn truncated(&self, total: u64, unit: LenUnit) -> SharedString {
        let size = match unit {
            LenUnit::Bytes => self.size(total),
            LenUnit::Chars => self.tr(
                ids::hv::GRID_SIZE_CHARS,
                ids::hv::HvGridSizeChars { n: Count(total) },
            ),
        };
        self.tr(
            ids::hv::GRID_TRUNCATED,
            ids::hv::HvGridTruncated { size: Text(size) },
        )
        .into()
    }
}

/// `bytes / unit` rounded to one fraction digit, exact (no float), without a trailing `.0`.
fn tenths(bytes: u64, unit: u64) -> ExactDecimal {
    let t = (u128::from(bytes) * 10 + u128::from(unit) / 2) / u128::from(unit);
    ExactDecimal(if t.is_multiple_of(10) {
        (t / 10).to_string()
    } else {
        format!("{}.{}", t / 10, t % 10)
    })
}

/// A formatted value as the grid's cell.
fn cell_content(shown: Display, facts: &ColFacts, texts: &Texts) -> CellContent {
    let kind = match shown.kind {
        DisplayKind::Text => CellKind::Text,
        DisplayKind::Number => CellKind::Number,
        DisplayKind::Null => CellKind::Null,
        DisplayKind::Empty => CellKind::Empty,
        DisplayKind::Bool => CellKind::Bool,
        DisplayKind::Temporal => CellKind::Temporal,
        DisplayKind::Binary => CellKind::Binary,
        DisplayKind::Json => CellKind::Json,
        DisplayKind::Error => CellKind::Error,
    };
    let align = match shown.kind {
        DisplayKind::Null => facts.align,
        DisplayKind::Bool => Align::Center,
        DisplayKind::Error => Align::Start,
        _ if shown.align_end => Align::End,
        _ => Align::Start,
    };
    let mut flags = GridCellFlags::empty();
    let value_flags = CellFlags::from_bits_truncate(shown.flags);
    flags.set(
        GridCellFlags::TRUNCATED,
        value_flags.contains(CellFlags::TRUNCATED),
    );
    flags.set(GridCellFlags::LOSSY, value_flags.contains(CellFlags::LOSSY));
    let chip = shown.chip.map(|chip| match chip {
        ChipKind::Truncated { total, unit } => texts.truncated(total, unit),
    });
    let text: SharedString = match shown.kind {
        // D-065: the constant, never a Fluent message.
        DisplayKind::Null => SharedString::new_static(NULL_MARKER),
        // The grid draws a decode error's text as chrome; the server-side message stays out of
        // the cell (NOTES surface 10).
        DisplayKind::Error => texts.tr(ids::hv::GRID_DECODE_ERROR, NoArgs).into(),
        _ => {
            for c in shown.text.chars() {
                if c == '\n' {
                    flags |= GridCellFlags::MULTILINE;
                }
                if c.is_control() {
                    flags |= GridCellFlags::HAS_CONTROLS;
                }
            }
            shown.text.into()
        }
    };
    CellContent {
        text,
        kind,
        flags,
        align,
        chip,
    }
}

/// The grid's columns and the formatting facts of `schema`, in server order: a column's ID is
/// its ordinal, so duplicate names keep their places.
fn columns_of(schema: &ResultSchema, engine: DbmsKind) -> (Arc<[ColumnSpec]>, Arc<[ColFacts]>) {
    let mut specs = Vec::with_capacity(schema.columns.len());
    let mut facts = Vec::with_capacity(schema.columns.len());
    for (ordinal, meta) in schema.columns.iter().enumerate() {
        let (kind, align, class) = column_class(&meta.logical);
        let native_name = native_name(meta);
        specs.push(ColumnSpec {
            id: ColId(u32::try_from(ordinal).unwrap_or(u32::MAX)),
            // Catalog text: shown as is, never translated or case-mapped.
            name: SharedString::from(meta.name.clone()),
            type_label: type_label(meta, &native_name).into(),
            kind,
            align,
            default_width: class.default_width(),
            // Key membership needs column origins (M6).
            key: false,
        });
        facts.push(ColFacts {
            id_like: id_like(&meta.name, &meta.logical, &native_name, engine),
            logical: meta.logical.clone(),
            native_name: native_name.into(),
            align,
        });
    }
    (specs.into(), facts.into())
}

/// A column's kind, alignment and starting width class by its logical type.
fn column_class(logical: &LogicalType) -> (ColumnKind, Align, WidthClass) {
    match logical {
        LogicalType::Bool => (ColumnKind::Bool, Align::Center, WidthClass::Bool),
        LogicalType::Int { .. } => (ColumnKind::Number, Align::End, WidthClass::Integer),
        LogicalType::Decimal { .. } | LogicalType::Float { .. } | LogicalType::Money => {
            (ColumnKind::Number, Align::End, WidthClass::Decimal)
        }
        LogicalType::Bytes { .. } => (ColumnKind::Binary, Align::Start, WidthClass::Binary),
        LogicalType::Uuid => (ColumnKind::Uuid, Align::Start, WidthClass::Uuid),
        LogicalType::Date
        | LogicalType::Time { .. }
        | LogicalType::Timestamp { .. }
        | LogicalType::Interval => (ColumnKind::Temporal, Align::Start, WidthClass::Timestamp),
        LogicalType::Json { .. } | LogicalType::Xml => {
            (ColumnKind::Json, Align::Start, WidthClass::Json)
        }
        LogicalType::Text { .. } | LogicalType::Enum(_) => {
            (ColumnKind::Text, Align::Start, WidthClass::Text)
        }
        LogicalType::Array(_)
        | LogicalType::Composite(_)
        | LogicalType::Range(_)
        | LogicalType::Network(_)
        | LogicalType::BitString { .. }
        | LogicalType::Geometry { .. }
        | LogicalType::Document
        | LogicalType::Opaque { .. } => (ColumnKind::Other, Align::Start, WidthClass::Text),
    }
}

/// The engine's bare name of a column's type (`numeric`); the logical type's name when the
/// engine sent none (MySQL reports a type code, not a name).
fn native_name(meta: &ColumnMeta) -> String {
    match &meta.native {
        NativeType::Pg { name, .. } => name.to_string(),
        NativeType::Sqlite {
            decl: Some(decl), ..
        } => decl.to_string(),
        NativeType::Sqlite {
            decl: None,
            affinity,
        } => match affinity {
            SqliteAffinity::Integer => "integer",
            SqliteAffinity::Text => "text",
            SqliteAffinity::Blob => "blob",
            SqliteAffinity::Real => "real",
            SqliteAffinity::Numeric => "numeric",
        }
        .to_string(),
        NativeType::MySql { .. } | NativeType::Unknown => logical_name(&meta.logical),
    }
}

/// The header's type line: the native type with its length or precision (`numeric(14,2)`,
/// `varchar(40)`). A SQLite declaration is shown as declared.
fn type_label(meta: &ColumnMeta, native_name: &str) -> String {
    if matches!(meta.native, NativeType::Sqlite { decl: Some(_), .. }) {
        return native_name.to_string();
    }
    match &meta.logical {
        LogicalType::Decimal {
            precision: Some(p),
            scale,
        } => match scale {
            Some(s) => format!("{native_name}({p},{s})"),
            None => format!("{native_name}({p})"),
        },
        LogicalType::Text {
            max_chars: Some(n), ..
        } => format!("{native_name}({n})"),
        LogicalType::Bytes { max: Some(n) } | LogicalType::BitString { len: Some(n) } => {
            format!("{native_name}({n})")
        }
        _ => native_name.to_string(),
    }
}

/// A type name for a column whose engine sent none: generic SQL spellings (data, never
/// translated).
fn logical_name(logical: &LogicalType) -> String {
    match logical {
        LogicalType::Bool => "boolean".into(),
        LogicalType::Int { bits, signed } => {
            let base = match bits {
                0..=8 => "tinyint",
                9..=16 => "smallint",
                17..=32 => "int",
                _ => "bigint",
            };
            if *signed {
                base.into()
            } else {
                format!("{base} unsigned")
            }
        }
        LogicalType::Decimal { .. } => "decimal".into(),
        LogicalType::Float { bits } => if *bits <= 32 { "float" } else { "double" }.into(),
        LogicalType::Money => "money".into(),
        LogicalType::Text { max_chars, fixed } => match (max_chars, fixed) {
            (Some(_), true) => "char",
            (Some(_), false) => "varchar",
            (None, _) => "text",
        }
        .into(),
        LogicalType::Bytes { .. } => "binary".into(),
        LogicalType::Uuid => "uuid".into(),
        LogicalType::Date => "date".into(),
        LogicalType::Time { tz, .. } => if *tz { "timetz" } else { "time" }.into(),
        LogicalType::Timestamp { tz, .. } => if *tz { "timestamptz" } else { "timestamp" }.into(),
        LogicalType::Interval => "interval".into(),
        LogicalType::Json { binary } => if *binary { "jsonb" } else { "json" }.into(),
        LogicalType::Xml => "xml".into(),
        LogicalType::Array(inner) => format!("{}[]", logical_name(inner)),
        LogicalType::Composite(_) => "record".into(),
        LogicalType::Enum(_) => "enum".into(),
        LogicalType::Range(inner) => format!("range({})", logical_name(inner)),
        LogicalType::Network(kind) => match kind {
            NetKind::Inet => "inet",
            NetKind::Cidr => "cidr",
            NetKind::MacAddr => "macaddr",
            NetKind::MacAddr8 => "macaddr8",
        }
        .into(),
        LogicalType::BitString { .. } => "bit".into(),
        LogicalType::Geometry { .. } => "geometry".into(),
        LogicalType::Document => "document".into(),
        LogicalType::Opaque { type_name } => type_name.to_string(),
    }
}
