//! Sources for tests and the synthetic B1 bench (feature `test-support`; never in a product
//! build).
//!
//! - [`SyntheticSource`]: generated cells, every row loaded. [`SyntheticSource::new`] keeps Task
//!   1b's `r<row>c<col>` cells; [`SyntheticSource::spike`] is the M0 spike's B1 data
//!   (`spikes/spike-grid/src/source.rs`): columns cycling `int8` / `text` / `numeric(14,2)` /
//!   `timestamp`, every 97th row of a column `NULL` (staggered), Turkish words in the text
//!   columns, grouped amounts, ISO timestamps. Its formatted cells sit in a frame-LRU the size
//!   of the app adapter's display cache, so a cell that stays on screen costs an `Arc` clone, as
//!   the real adapter's does.
//! - [`FixtureSource`]: fixed cells whose row count, total, `more` flag and generation a test
//!   changes between frames; it counts `ensure_rows` per frame.

use std::cell::RefCell;
use std::fmt::Write as _;
use std::rc::Rc;
use std::sync::Arc;

use hacivat_ui::text::TextFn;
use hacivat_ui_core::gpui_surface::{App, SharedString};

use crate::element::text_cache::FrameLru;
use crate::source::{
    Align, CellContent, CellKind, ColId, ColumnKind, ColumnSpec, GridCellFlags, GridDataSource,
    GridFrame, ModelRow, NULL_MARKER, RowCount, SourceCaps,
};
use crate::view::{GridConfig, GridLabels};

/// The spike source's display cache: the app adapter's 64k entries.
pub const SYNTHETIC_DISPLAY_CACHE: usize = 65_536;

/// A grid configuration for tests and benches: 21 px rows, no zebra, no column auto-size (the
/// columns keep the widths their source gives, so a test places cells by them), fixed labels,
/// and the accessibility label `a11y_label` (the app passes Fluent text instead).
pub fn grid_config(a11y_label: &'static str) -> GridConfig {
    let fixed = |s: &'static str| -> TextFn { Rc::new(move |_: &App| SharedString::new_static(s)) };
    GridConfig {
        row_height: crate::model::DEFAULT_ROW_HEIGHT,
        zebra: false,
        auto_size: false,
        labels: GridLabels {
            loading: fixed("Loading"),
            too_many_columns: fixed("Too many columns"),
        },
        a11y_label: fixed(a11y_label),
    }
}

/// A plain cell.
pub fn cell(text: impl Into<SharedString>, kind: CellKind, align: Align) -> CellContent {
    CellContent {
        text: text.into(),
        kind,
        flags: GridCellFlags::empty(),
        align,
        chip: None,
    }
}

/// A text column `name` (catalog text), `width` px wide.
pub fn text_column(id: u32, name: &str, width: f32) -> ColumnSpec {
    ColumnSpec {
        id: ColId(id),
        name: SharedString::from(name.to_string()),
        type_label: "text".into(),
        kind: ColumnKind::Text,
        align: Align::Start,
        default_width: width,
        key: false,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Data {
    /// Task 1b's `r<row>c<col>`.
    Plain,
    /// The M0 spike's B1 data.
    Spike,
}

/// A generated source (module docs).
#[derive(Clone)]
pub struct SyntheticSource {
    rows: u64,
    columns: Arc<[ColumnSpec]>,
    data: Data,
    display: Rc<RefCell<FrameLru<(u64, u32), CellContent>>>,
}

impl std::fmt::Debug for SyntheticSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SyntheticSource")
            .field("rows", &self.rows)
            .field("columns", &self.columns.len())
            .field("data", &self.data)
            .finish()
    }
}

impl SyntheticSource {
    /// `rows` × `cols` cells `r<row>c<col>`; column 0 holds the row number.
    pub fn new(rows: u64, cols: u32) -> Self {
        let columns = (0..cols)
            .map(|c| ColumnSpec {
                id: ColId(c),
                name: format!("c{c}").into(),
                type_label: if c == 0 { "int8" } else { "text" }.into(),
                kind: if c == 0 {
                    ColumnKind::Number
                } else {
                    ColumnKind::Text
                },
                align: if c == 0 { Align::End } else { Align::Start },
                default_width: if c == 0 { 96.0 } else { 160.0 },
                key: c == 0,
            })
            .collect();
        SyntheticSource {
            rows,
            columns,
            data: Data::Plain,
            display: Rc::new(RefCell::new(FrameLru::new(SYNTHETIC_DISPLAY_CACHE))),
        }
    }

    /// The M0 spike's B1 data (module docs), `rows` × `cols`.
    pub fn spike(rows: u64, cols: u32) -> Self {
        let columns = (0..cols)
            .map(|c| {
                let (name, type_label, width, kind) = match spike_kind(c) {
                    SpikeKind::Int => ("id", "int8", 104.0, ColumnKind::Number),
                    SpikeKind::Text => ("label", "text", 168.0, ColumnKind::Text),
                    SpikeKind::Numeric => ("amount", "numeric(14,2)", 148.0, ColumnKind::Number),
                    SpikeKind::Timestamp => {
                        ("created_at", "timestamp", 160.0, ColumnKind::Temporal)
                    }
                };
                ColumnSpec {
                    id: ColId(c),
                    name: format!("{name}_{c}").into(),
                    type_label: type_label.into(),
                    kind,
                    align: if kind == ColumnKind::Number {
                        Align::End
                    } else {
                        Align::Start
                    },
                    default_width: width,
                    key: false,
                }
            })
            .collect();
        SyntheticSource {
            rows,
            columns,
            data: Data::Spike,
            display: Rc::new(RefCell::new(FrameLru::new(SYNTHETIC_DISPLAY_CACHE))),
        }
    }

    /// The cell at `(row, col)`, formatted.
    pub fn format(&self, row: u64, col: u32) -> CellContent {
        match self.data {
            Data::Plain => plain_cell(row, col),
            Data::Spike => spike_cell(row, col),
        }
    }
}

fn plain_cell(row: u64, col: u32) -> CellContent {
    if col == 0 {
        cell(row.to_string(), CellKind::Number, Align::End)
    } else {
        cell(format!("r{row}c{col}"), CellKind::Text, Align::Start)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SpikeKind {
    Int,
    Text,
    Numeric,
    Timestamp,
}

fn spike_kind(col: u32) -> SpikeKind {
    match col % 4 {
        0 => SpikeKind::Int,
        1 => SpikeKind::Text,
        2 => SpikeKind::Numeric,
        _ => SpikeKind::Timestamp,
    }
}

/// Every 97th row of each column, staggered by column, so no row is all `NULL`.
fn spike_null(row: u64, col: u32) -> bool {
    (row + u64::from(col)) % 97 == 96
}

/// SplitMix64's finalizer over `(row, col)` (decimal constants: the golden ratio and the
/// SplitMix64 multipliers).
fn cell_hash(row: u64, col: u32) -> u64 {
    let mut z = row
        .wrapping_mul(11_400_714_819_323_198_485)
        .wrapping_add(u64::from(col).wrapping_mul(15_111_065_706_836_454_659))
        .wrapping_add(7_146_057_691_288_625_177);
    z = (z ^ (z >> 30)).wrapping_mul(13_787_848_793_156_543_929);
    z = (z ^ (z >> 27)).wrapping_mul(10_723_151_780_598_845_931);
    z ^ (z >> 31)
}

/// Mixed ASCII and Turkish words, so shaping meets non-ASCII glyphs too (the spike's list).
const WORDS: [&str; 16] = [
    "Ankara",
    "İstanbul",
    "Çankaya",
    "Kadıköy",
    "Ümraniye",
    "Şişli",
    "invoice",
    "refund",
    "pending",
    "shipped",
    "customer",
    "warehouse",
    "Göztepe",
    "Beşiktaş",
    "priority",
    "archive",
];

/// `v` with en-US thousands separators.
fn push_grouped(out: &mut String, v: u64) {
    let digits = v.to_string();
    let n = digits.len();
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (n - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
}

/// Days since 1970-01-01 to a civil date (Howard Hinnant's algorithm).
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

fn spike_cell(row: u64, col: u32) -> CellContent {
    let kind = spike_kind(col);
    let align = match kind {
        SpikeKind::Int | SpikeKind::Numeric => Align::End,
        _ => Align::Start,
    };
    if spike_null(row, col) {
        return cell(NULL_MARKER, CellKind::Null, align);
    }
    let h = cell_hash(row, col);
    let mut out = String::with_capacity(24);
    let cell_kind = match kind {
        SpikeKind::Int => {
            let _ = write!(out, "{}", (h >> 33) as i64 - (1 << 30));
            CellKind::Number
        }
        SpikeKind::Text => {
            out.push_str(WORDS[(h & 15) as usize]);
            if h & 48 != 0 {
                out.push(' ');
                out.push_str(WORDS[((h >> 8) & 15) as usize]);
            }
            CellKind::Text
        }
        SpikeKind::Numeric => {
            // numeric(14,2): mostly everyday amounts, one cell in eight at full width.
            let full = (h >> 8) % 100_000_000_000_000;
            let cents = if h & 7 == 0 {
                full
            } else {
                full % 1_000_000_000
            };
            if h & 16 != 0 {
                out.push('-');
            }
            push_grouped(&mut out, cents / 100);
            let _ = write!(out, ".{:02}", cents % 100);
            CellKind::Number
        }
        SpikeKind::Timestamp => {
            let secs = 946_684_800 + (h >> 20) % (30 * 365 * 86_400);
            let (y, m, d) = civil_from_days((secs / 86_400) as i64);
            let t = secs % 86_400;
            let _ = write!(
                out,
                "{y:04}-{m:02}-{d:02} {:02}:{:02}:{:02}",
                t / 3600,
                (t / 60) % 60,
                t % 60
            );
            CellKind::Temporal
        }
    };
    cell(out, cell_kind, align)
}

struct SyntheticFrame {
    source: SyntheticSource,
}

impl GridFrame for SyntheticFrame {
    fn generation(&self) -> (u64, u64) {
        (1, 1)
    }

    fn row_count(&self) -> RowCount {
        RowCount {
            loaded: self.source.rows,
            more: false,
            exact_total: Some(self.source.rows),
        }
    }

    fn cell(&self, row: ModelRow, col: ColId) -> CellContent {
        let mut display = self.source.display.borrow_mut();
        display
            .get_or_insert_with((row.0, col.0), || self.source.format(row.0, col.0))
            .0
            .clone()
    }
}

impl GridDataSource for SyntheticSource {
    fn begin_frame(&self, _cx: &App) -> Rc<dyn GridFrame> {
        self.display.borrow_mut().begin_frame();
        Rc::new(SyntheticFrame {
            source: self.clone(),
        })
    }

    fn columns(&self) -> Arc<[ColumnSpec]> {
        self.columns.clone()
    }

    fn caps(&self) -> SourceCaps {
        SourceCaps::default()
    }

    fn ensure_rows(&self, upto: u64, cx: &mut App) {
        let _ = (upto, cx);
    }
}

/// What a [`FixtureSource`] holds.
struct FixtureState {
    /// Cells by row; rows past the end repeat the generator's cells.
    rows: Rc<Vec<Vec<CellContent>>>,
    generated: Option<Rc<dyn Fn(u64, ColId) -> CellContent>>,
    count: RowCount,
    generation: u64,
    version: u64,
    ensure_total: u64,
    ensure_frame: u32,
    last_upto: Option<u64>,
    frames: u64,
}

/// Fixed cells for tests (module docs). Clones share their state, so a test keeps one handle
/// and gives the grid another.
#[derive(Clone)]
pub struct FixtureSource {
    columns: Arc<[ColumnSpec]>,
    state: Rc<RefCell<FixtureState>>,
}

impl std::fmt::Debug for FixtureSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = self.state.borrow();
        f.debug_struct("FixtureSource")
            .field("columns", &self.columns.len())
            .field("count", &s.count)
            .field("generation", &s.generation)
            .finish()
    }
}

impl FixtureSource {
    /// `rows` of cells under `columns`, all loaded and complete.
    pub fn new(columns: Vec<ColumnSpec>, rows: Vec<Vec<CellContent>>) -> Self {
        let n = rows.len() as u64;
        FixtureSource {
            columns: columns.into(),
            state: Rc::new(RefCell::new(FixtureState {
                rows: Rc::new(rows),
                generated: None,
                count: RowCount {
                    loaded: n,
                    more: false,
                    exact_total: Some(n),
                },
                generation: 1,
                version: 1,
                ensure_total: 0,
                ensure_frame: 0,
                last_upto: None,
                frames: 0,
            })),
        }
    }

    /// Text cells from `rows` (each row's texts left to right) under text columns `c0`, `c1`, …
    /// `width` px wide.
    pub fn texts(rows: &[&[&str]], width: f32) -> Self {
        let cols = rows.iter().map(|r| r.len()).max().unwrap_or(0);
        let columns = (0..cols as u32)
            .map(|c| text_column(c, &format!("c{c}"), width))
            .collect();
        let rows = rows
            .iter()
            .map(|r| {
                r.iter()
                    .map(|t| {
                        cell(
                            SharedString::from(t.to_string()),
                            CellKind::Text,
                            Align::Start,
                        )
                    })
                    .collect()
            })
            .collect();
        FixtureSource::new(columns, rows)
    }

    /// `cols` text columns whose cells come from `f`, with `loaded` rows loaded of `total`.
    pub fn generated(
        cols: u32,
        loaded: u64,
        total: Option<u64>,
        more: bool,
        f: impl Fn(u64, ColId) -> CellContent + 'static,
    ) -> Self {
        let columns = (0..cols)
            .map(|c| text_column(c, &format!("c{c}"), 120.0))
            .collect();
        let source = FixtureSource::new(columns, Vec::new());
        {
            let mut s = source.state.borrow_mut();
            s.generated = Some(Rc::new(f));
            s.count = RowCount {
                loaded,
                more,
                exact_total: total,
            };
        }
        source
    }

    /// The same cells under `columns` (names, types, widths).
    pub fn with_columns(mut self, columns: Vec<ColumnSpec>) -> Self {
        self.columns = columns.into();
        self
    }

    /// Sets the loaded rows, whether more come, and the exact total (an append or a stream).
    pub fn set_count(&self, loaded: u64, more: bool, exact_total: Option<u64>) {
        let mut s = self.state.borrow_mut();
        s.count = RowCount {
            loaded,
            more,
            exact_total,
        };
        s.version += 1;
    }

    /// Starts a new generation (a re-run) with `loaded` rows.
    pub fn rerun(&self, loaded: u64, more: bool, exact_total: Option<u64>) {
        self.set_count(loaded, more, exact_total);
        self.state.borrow_mut().generation += 1;
    }

    /// `ensure_rows` calls since the last `begin_frame`.
    pub fn ensure_calls_this_frame(&self) -> u32 {
        self.state.borrow().ensure_frame
    }

    /// Every `ensure_rows` call so far.
    pub fn ensure_calls(&self) -> u64 {
        self.state.borrow().ensure_total
    }

    /// The last `ensure_rows` argument.
    pub fn last_ensure_upto(&self) -> Option<u64> {
        self.state.borrow().last_upto
    }

    /// Frames begun so far.
    pub fn frames(&self) -> u64 {
        self.state.borrow().frames
    }
}

struct FixtureFrame {
    rows: Rc<Vec<Vec<CellContent>>>,
    generated: Option<Rc<dyn Fn(u64, ColId) -> CellContent>>,
    count: RowCount,
    generation: (u64, u64),
}

impl GridFrame for FixtureFrame {
    fn generation(&self) -> (u64, u64) {
        self.generation
    }

    fn row_count(&self) -> RowCount {
        self.count
    }

    fn cell(&self, row: ModelRow, col: ColId) -> CellContent {
        if let Some(c) = self
            .rows
            .get(row.0 as usize)
            .and_then(|r| r.get(col.0 as usize))
        {
            return c.clone();
        }
        match &self.generated {
            Some(f) => f(row.0, col),
            None => cell("", CellKind::Placeholder, Align::Start),
        }
    }
}

impl GridDataSource for FixtureSource {
    fn begin_frame(&self, _cx: &App) -> Rc<dyn GridFrame> {
        let mut s = self.state.borrow_mut();
        s.ensure_frame = 0;
        s.frames += 1;
        Rc::new(FixtureFrame {
            rows: s.rows.clone(),
            generated: s.generated.clone(),
            count: s.count,
            generation: (s.generation, s.version),
        })
    }

    fn columns(&self) -> Arc<[ColumnSpec]> {
        self.columns.clone()
    }

    fn caps(&self) -> SourceCaps {
        SourceCaps::default()
    }

    fn ensure_rows(&self, upto: u64, _cx: &mut App) {
        let mut s = self.state.borrow_mut();
        s.ensure_total += 1;
        s.ensure_frame += 1;
        s.last_upto = Some(upto);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spike_cells_are_deterministic_and_vary() {
        let s = SyntheticSource::spike(10_000_000, 50);
        assert_eq!(s.format(9_999_999, 49), s.format(9_999_999, 49));
        let texts: std::collections::HashSet<_> =
            (0..50).map(|r| s.format(r, 0).text.to_string()).collect();
        assert!(texts.len() > 40);
    }

    #[test]
    fn spike_columns_cycle_their_kinds() {
        let s = SyntheticSource::spike(10, 5);
        let labels: Vec<_> = s.columns.iter().map(|c| c.type_label.to_string()).collect();
        assert_eq!(
            labels,
            ["int8", "text", "numeric(14,2)", "timestamp", "int8"]
        );
        assert_eq!(s.columns[0].align, Align::End);
        assert_eq!(s.columns[3].kind, ColumnKind::Temporal);
    }

    #[test]
    fn every_97th_spike_row_is_null() {
        let s = SyntheticSource::spike(10_000_000, 50);
        for col in [0, 1, 2, 3, 49] {
            let nulls = (0..97 * 100)
                .filter(|&row| s.format(row, col).kind == CellKind::Null)
                .count();
            assert_eq!(nulls, 100, "col {col}");
        }
    }

    #[test]
    fn spike_amounts_are_grouped_with_two_decimals_and_timestamps_are_iso() {
        let mut g = String::new();
        push_grouped(&mut g, 1_234_567);
        assert_eq!(g, "1,234,567");
        g.clear();
        push_grouped(&mut g, 999);
        assert_eq!(g, "999");
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(11_016), (2000, 2, 29));
        let s = SyntheticSource::spike(100, 4);
        let ts = s.format(1, 3).text;
        assert_eq!(ts.len(), 19, "{ts}");
        let amount = s.format(1, 2).text;
        assert!(
            amount.split_once('.').is_some_and(|(_, f)| f.len() == 2),
            "{amount}"
        );
    }
}
