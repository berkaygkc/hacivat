//! The selection model (HV-GRID-05; Task 17): anchor and focus, rectangles in view coordinates,
//! row and column sets, select-all; cell counts in `u128` by inclusion–exclusion; copy
//! normalisation (a union that is neither stacked nor concatenable is refused; more than
//! [`MAX_COPY_CELLS`] is refused); a generation change clears the selection and clamps the
//! focus (kept pending until the new generation has rows), an append keeps it.
//!
//! Coordinates (digest 04 ruling R15):
//! - the focus, the anchor and the row set are **model rows**, so they follow the data through
//!   a local sort (M3b);
//! - rectangles are **view rows**;
//! - a column is counted by its position in the full view order (`ColumnLayout::order`, hidden
//!   columns included). M3a never reorders columns, so that position is the `ColId` ordinal. A
//!   hidden column inside a rectangle stays in it; copy and the count skip it.
//!
//! M3a had no local sort: [`ViewMap`] was the identity, so view rows equalled model rows;
//! [`Selection::cell_count`] still assumes that, [`Selection::cell_count_in`] does not.
//!
//! ## m3b-t01b-seams: [`ViewMap::Permuted`] (a local sort, Task 36) and [`ViewMap::Filtered`]
//! (filter rows, Task 37) map working (P-M3b-24: the map is no longer `Copy`).
//!
//! ## m3b-t35 — under a permuted or filtered map (M3c Task 35; digest 04 R15)
//! - [`ViewMap::permuted`] / [`ViewMap::filtered`] build a map with its inverse index (model row
//!   → view row, a `u32` per model row), so [`ViewMap::model_to_view`] is O(1). A map written as
//!   a bare variant builds the index on its first lookup, and an index that does not belong to
//!   its map is never trusted (each hit is checked against the rows).
//! - [`Selection::on_view_map_changed`] (a re-sort, a filter, back to the source order) clears
//!   the rectangles (view rows of the old order); the focus, the row set and the column set
//!   stay. A selection left empty selects its focus cell where the new map shows it.
//! - Row sets copy and count as the view rows that show them; rows a filter hides drop out.
//!   Shift-extending rows under a sort or a filter ([`Selection::extend_rows_to_in`]) takes the
//!   rows **as shown**: a whole-width rectangle in view rows (cleared by the next re-sort, like
//!   any rectangle).
//! - [`Selection::grow`] / [`Selection::shrink`]: cell → row segment → row → all. The row
//!   segment is the focus row across the run of on-screen columns holding the focus (the view
//!   passes it); it is skipped when it is the cell or the whole row.

use std::ops::Range;
use std::sync::{Arc, OnceLock, Weak};

use crate::source::{ColId, ModelCell, ModelRow, ViewRow};

/// The most cells a copy takes (HV-XFER-02 step 2); a larger selection is refused.
pub const MAX_COPY_CELLS: u128 = 1_000_000;

/// How view rows map to model rows.
///
/// ## m3b-t35: build a sort's and a filter's map with [`ViewMap::permuted`] and
/// [`ViewMap::filtered`] (they build the inverse index; Task 36 calls `permuted` on the
/// background executor with its sort). Match the variants with `..`: `index` is a cache.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ViewMap {
    /// No local sort or filter (all of M3a): view row `n` is model row `n`. `loaded` rows
    /// exist; a row past them maps to itself (a placeholder).
    Identity { loaded: u64 },
    // ## m3b-t01b-seams
    /// A local sort (Task 36): view row `i < order.len()` is model row `order[i]`; the rows
    /// after them are the unsorted tail, in source order from model row `tail_from` (rows that
    /// arrived during or after the sort; a row past `loaded` is a placeholder, as for the
    /// identity). `loaded` rows exist. `order` holds each model row below `tail_from` once.
    Permuted {
        order: Arc<[u64]>,
        tail_from: u64,
        loaded: u64,
        // ## m3b-t35
        /// Model row → view row over `order` (a cache, never compared or hashed).
        index: ViewIndex,
    },
    /// Filter rows (Task 37): view row `i` is model row `rows[i]`; no other row is shown.
    /// `source_loaded` is the source's loaded rows the filter ran over.
    Filtered {
        rows: Arc<[u64]>,
        source_loaded: u64,
        // ## m3b-t35
        /// Model row → view row over `rows` (a cache, never compared or hashed).
        index: ViewIndex,
    },
}

// ## m3b-t35
/// A map's inverse (model row → view row), shared between clones of its map and built at most
/// once (by [`ViewMap::permuted`] / [`ViewMap::filtered`], or on a bare variant's first
/// lookup). It never decides equality or hashing (it is a function of the map's rows). Every
/// hit is checked against the rows, and a miss is trusted only when the index was built from
/// the same rows (the same allocation, which the index's weak reference keeps from being
/// reused), so an index taken from another map makes a lookup scan, never answer wrong.
#[derive(Clone, Default)]
pub struct ViewIndex(Arc<OnceLock<Inverse>>);

/// The inverse index's "not shown".
const ABSENT: u32 = u32::MAX;

/// A built inverse and the rows it was built from.
struct Inverse {
    rows: Weak<[u64]>,
    lookup: Lookup,
}

enum Lookup {
    /// A view row (`ABSENT`: not shown) per model row up to the largest shown: O(1).
    Dense(Box<[u32]>),
    /// `(model, view)` pairs sorted by model row, for rows too sparse or too many for `Dense`
    /// (a few kept rows of a huge source): O(log n).
    Sorted(Box<[(u64, u64)]>),
}

impl Inverse {
    fn of(arc: &Arc<[u64]>) -> Self {
        let rows: &[u64] = arc;
        let bound = rows
            .iter()
            .copied()
            .max()
            .and_then(|m| usize::try_from(m).ok()?.checked_add(1))
            .unwrap_or(0);
        let dense =
            rows.len() < ABSENT as usize && bound <= rows.len().saturating_mul(16).max(1 << 20);
        let lookup = if dense {
            let mut index = vec![ABSENT; bound];
            for (view, &model) in rows.iter().enumerate() {
                index[model as usize] = view as u32;
            }
            Lookup::Dense(index.into_boxed_slice())
        } else {
            let mut pairs: Vec<(u64, u64)> = rows
                .iter()
                .enumerate()
                .map(|(view, &model)| (model, view as u64))
                .collect();
            pairs.sort_unstable();
            Lookup::Sorted(pairs.into_boxed_slice())
        };
        Inverse {
            rows: Arc::downgrade(arc),
            lookup,
        }
    }
}

impl ViewIndex {
    /// The index of `rows` (view row `i` shows model row `rows[i]`), built now.
    fn built(rows: &Arc<[u64]>) -> Self {
        let cell = OnceLock::new();
        let _ = cell.set(Inverse::of(rows));
        ViewIndex(Arc::new(cell))
    }

    /// The view row of `model` among `rows`: through the index (built on first use), each hit
    /// checked against `rows`; a scan when the index belongs to other rows.
    fn lookup(&self, arc: &Arc<[u64]>, model: u64) -> Option<u64> {
        let inv = self.0.get_or_init(|| Inverse::of(arc));
        let rows: &[u64] = arc;
        let found = match &inv.lookup {
            Lookup::Dense(index) => usize::try_from(model)
                .ok()
                .and_then(|m| index.get(m))
                .filter(|v| **v != ABSENT)
                .map(|v| u64::from(*v)),
            Lookup::Sorted(pairs) => pairs
                .binary_search_by_key(&model, |(m, _)| *m)
                .ok()
                .map(|i| pairs[i].1),
        };
        if let Some(v) = found
            && usize::try_from(v).ok().and_then(|v| rows.get(v)) == Some(&model)
        {
            return Some(v);
        }
        if std::ptr::addr_eq(inv.rows.as_ptr(), Arc::as_ptr(arc)) {
            return None;
        }
        rows.iter().position(|&r| r == model).map(|i| i as u64)
    }
}

impl std::fmt::Debug for ViewIndex {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self.0.get().map(|i| &i.lookup) {
            None => "ViewIndex(lazy)",
            Some(Lookup::Dense(_)) => "ViewIndex(dense)",
            Some(Lookup::Sorted(_)) => "ViewIndex(sorted)",
        })
    }
}

impl PartialEq for ViewIndex {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

impl Eq for ViewIndex {}

impl std::hash::Hash for ViewIndex {
    fn hash<H: std::hash::Hasher>(&self, _: &mut H) {}
}

impl ViewMap {
    pub fn identity(loaded: u64) -> Self {
        ViewMap::Identity { loaded }
    }

    // ## m3b-t35
    /// A local sort: view row `i` shows model row `order[i]`, then the unsorted tail from model
    /// row `tail_from` in source order; `loaded` rows exist. Builds the inverse index in O(n)
    /// (call it off the UI thread for a large result).
    pub fn permuted(order: Arc<[u64]>, tail_from: u64, loaded: u64) -> Self {
        let index = ViewIndex::built(&order);
        ViewMap::Permuted {
            order,
            tail_from,
            loaded,
            index,
        }
    }

    /// Filter rows: view row `i` shows model row `rows[i]` of the `source_loaded` rows the
    /// filter ran over. Builds the inverse index.
    pub fn filtered(rows: Arc<[u64]>, source_loaded: u64) -> Self {
        let index = ViewIndex::built(&rows);
        ViewMap::Filtered {
            rows,
            source_loaded,
            index,
        }
    }

    /// The same map over `loaded` source rows: rows appended since a sort join its unsorted
    /// tail; a filter shows only its own rows (unchanged). Shares the index.
    pub fn with_loaded(&self, loaded: u64) -> Self {
        match self {
            ViewMap::Identity { .. } => ViewMap::Identity { loaded },
            ViewMap::Permuted {
                order,
                tail_from,
                index,
                ..
            } => ViewMap::Permuted {
                order: order.clone(),
                tail_from: *tail_from,
                loaded,
                index: index.clone(),
            },
            ViewMap::Filtered { .. } => self.clone(),
        }
    }

    /// View rows are model rows.
    pub fn is_identity(&self) -> bool {
        matches!(self, ViewMap::Identity { .. })
    }

    /// The loaded rows, as the view shows them (a filter shows only the rows it kept).
    pub fn len(&self) -> u64 {
        match self {
            ViewMap::Identity { loaded } | ViewMap::Permuted { loaded, .. } => *loaded,
            ViewMap::Filtered { rows, .. } => rows.len() as u64,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The model row view row `row` shows; `None` for a row a filter does not show.
    pub fn view_to_model(&self, row: ViewRow) -> Option<ModelRow> {
        match self {
            ViewMap::Identity { .. } => Some(ModelRow(row.0)),
            ViewMap::Permuted {
                order, tail_from, ..
            } => {
                let sorted = order.len() as u64;
                if row.0 < sorted {
                    order
                        .get(usize::try_from(row.0).ok()?)
                        .copied()
                        .map(ModelRow)
                } else {
                    tail_from.checked_add(row.0 - sorted).map(ModelRow)
                }
            }
            ViewMap::Filtered { rows, .. } => rows
                .get(usize::try_from(row.0).ok()?)
                .copied()
                .map(ModelRow),
        }
    }

    /// The view row that shows model row `row`; `None` for a row a filter does not show. O(1)
    /// through the inverse index (## m3b-t35).
    pub fn model_to_view(&self, row: ModelRow) -> Option<ViewRow> {
        match self {
            ViewMap::Identity { .. } => Some(ViewRow(row.0)),
            ViewMap::Permuted {
                order,
                tail_from,
                index,
                ..
            } => {
                if row.0 >= *tail_from {
                    (order.len() as u64)
                        .checked_add(row.0 - tail_from)
                        .map(ViewRow)
                } else {
                    index.lookup(order, row.0).map(ViewRow)
                }
            }
            ViewMap::Filtered { rows, index, .. } => index.lookup(rows, row.0).map(ViewRow),
        }
    }

    // ## m3b-t35
    /// Whether model row `row` shows among the loaded rows (`false` for a row a filter hides).
    pub fn shows(&self, row: ModelRow) -> bool {
        self.model_to_view(row).is_some_and(|v| v.0 < self.len())
    }
}

/// A rectangle of cells, half-open: view rows × column positions (module docs).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CellRect {
    pub rows: Range<u64>,
    pub cols: Range<u32>,
}

impl CellRect {
    /// The rectangle with corners `a` and `b` (`(view row, column position)`), both included.
    pub fn spanning(a: (u64, u32), b: (u64, u32)) -> Self {
        CellRect {
            rows: span_u64(a.0, b.0),
            cols: span_u32(a.1, b.1),
        }
    }

    pub fn contains(&self, row: u64, col: u32) -> bool {
        self.rows.contains(&row) && self.cols.contains(&col)
    }
}

/// One piece of a selection, in the order the user made them.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum Part {
    Rect(CellRect),
    /// Model rows (gutter clicks).
    Rows(Range<u64>),
    /// Column positions (header clicks).
    Cols(Range<u32>),
}

/// A grid's selection.
///
/// The pieces keep the order they were made in; the last one is the active one, which Shift
/// (click, arrow, gutter or header) extends from the anchor. After select-all with nothing
/// added, select-all itself is the active piece, and extending replaces it.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct Selection {
    anchor: Option<ModelCell>,
    focus: Option<ModelCell>,
    parts: Vec<Part>,
    all: bool,
    /// A focus carried over a re-run whose new generation had no rows yet: nothing is selected
    /// or painted, and [`Selection::on_rows_loaded`] places it once rows arrive.
    pending: Option<ModelCell>,
    /// The rectangles of `parts`, in order.
    rects: Vec<CellRect>,
    /// The row pieces, sorted and merged.
    rows: Vec<Range<u64>>,
    /// The column pieces, sorted and merged.
    cols: Vec<Range<u32>>,
}

impl Selection {
    /// The focused (active) cell.
    pub fn focus(&self) -> Option<ModelCell> {
        self.focus
    }

    /// Where the active piece was started.
    pub fn anchor(&self) -> Option<ModelCell> {
        self.anchor
    }

    /// The rectangles, in the order they were made.
    pub fn rects(&self) -> &[CellRect] {
        &self.rects
    }

    /// The row set (model rows), sorted and merged.
    pub fn rows(&self) -> &[Range<u64>] {
        &self.rows
    }

    /// The column set (column positions), sorted and merged.
    pub fn cols(&self) -> &[Range<u32>] {
        &self.cols
    }

    /// Everything is selected.
    pub fn is_all(&self) -> bool {
        self.all
    }

    /// Nothing is selected (a pending focus selects nothing).
    pub fn is_empty(&self) -> bool {
        !self.all && self.parts.is_empty()
    }

    /// The focus a re-run carried over, waiting for the new generation's first rows.
    pub fn pending_focus(&self) -> Option<ModelCell> {
        self.pending
    }

    // ## m3b-t35
    /// How many ranges the selection holds (the strip's "2 ranges · 10 cells"): its pieces in
    /// the order made; select-all is one; nothing is none.
    pub fn ranges(&self) -> usize {
        if self.all { 1 } else { self.parts.len() }
    }

    /// The view map changed (a re-sort, a filter, back to the source order; digest 04 R15): the
    /// rectangles were view rows of the old order and are cleared; the focus, the row set and
    /// the column set are model rows and columns and stay. When nothing else is left, the focus
    /// cell is selected alone where `map` shows it (a focus a filter hides selects nothing and
    /// stays, for the next key).
    pub fn on_view_map_changed(&mut self, map: &ViewMap) {
        self.parts.retain(|p| !matches!(p, Part::Rect(_)));
        if self.parts.is_empty() && !self.all {
            self.anchor = self.focus;
            if let Some(focus) = self.focus.filter(|f| map.shows(f.row)) {
                let at = corner(focus, map);
                self.parts.push(Part::Rect(CellRect::spanning(at, at)));
            }
        }
        self.refresh();
    }

    /// Shift on the gutter and Select row, through `map`: in the source order this is
    /// [`Selection::extend_rows_to`] (a model row range). Under a sort or a filter the rows from
    /// the anchor's to `row` are taken **as shown**: a whole-width rectangle of view rows (one
    /// row stays a model row piece).
    pub fn extend_rows_to_in(&mut self, row: ModelRow, map: &ViewMap) {
        let Some(anchor) = self.anchor else {
            let col = self.focus.map_or(ColId(0), |f| f.col);
            return self.select_row(row, col);
        };
        let (from, to) = (corner(anchor, map).0, view_of(row, map));
        if map.is_identity() || from == to {
            return self.extend_rows_to(row);
        }
        self.leave_all();
        let piece = Part::Rect(CellRect {
            rows: span_u64(from, to),
            cols: 0..u32::MAX,
        });
        match self.parts.last_mut() {
            Some(last) => *last = piece,
            None => self.parts.push(piece),
        }
        let col = self.focus.map_or(anchor.col, |f| f.col);
        self.focus = Some(ModelCell { row, col });
        self.refresh();
    }

    /// A click: only `cell` is selected, and it is the anchor and the focus.
    pub fn select_cell(&mut self, cell: ModelCell, map: &ViewMap) {
        self.parts.clear();
        self.all = false;
        self.add_cell(cell, map);
    }

    /// Ctrl/Cmd+click: `cell` becomes a new rectangle, the active one.
    pub fn add_cell(&mut self, cell: ModelCell, map: &ViewMap) {
        self.anchor = Some(cell);
        self.focus = Some(cell);
        self.pending = None;
        let at = corner(cell, map);
        self.parts.push(Part::Rect(CellRect::spanning(at, at)));
        self.refresh();
    }

    /// Shift+click, Shift+arrow, a drag: the active piece now reaches from the anchor to
    /// `cell`, which takes the focus. Without an anchor this is a click.
    pub fn extend_to(&mut self, cell: ModelCell, map: &ViewMap) {
        let Some(anchor) = self.anchor else {
            return self.select_cell(cell, map);
        };
        self.leave_all();
        let rect = CellRect::spanning(corner(anchor, map), corner(cell, map));
        match self.parts.last_mut() {
            Some(Part::Rows(rows)) => *rows = span_u64(anchor.row.0, cell.row.0),
            Some(Part::Cols(cols)) => *cols = span_u32(anchor.col.0, cell.col.0),
            Some(Part::Rect(r)) => *r = rect,
            None => self.parts.push(Part::Rect(rect)),
        }
        self.focus = Some(cell);
        self.refresh();
    }

    /// A gutter click: only `row` is selected; the focus is `(row, col)`.
    pub fn select_row(&mut self, row: ModelRow, col: ColId) {
        self.parts.clear();
        self.all = false;
        self.add_row(row, col);
    }

    /// Ctrl/Cmd+click on the gutter: `row` is added as the active piece.
    pub fn add_row(&mut self, row: ModelRow, col: ColId) {
        let cell = ModelCell { row, col };
        self.anchor = Some(cell);
        self.focus = Some(cell);
        self.pending = None;
        self.parts.push(Part::Rows(span_u64(row.0, row.0)));
        self.refresh();
    }

    /// Shift+click on the gutter, and Select row: the active piece becomes the whole rows from
    /// the anchor's row to `row`.
    pub fn extend_rows_to(&mut self, row: ModelRow) {
        let Some(anchor) = self.anchor else {
            let col = self.focus.map_or(ColId(0), |f| f.col);
            return self.select_row(row, col);
        };
        self.leave_all();
        let piece = Part::Rows(span_u64(anchor.row.0, row.0));
        match self.parts.last_mut() {
            Some(last) => *last = piece,
            None => self.parts.push(piece),
        }
        let col = self.focus.map_or(anchor.col, |f| f.col);
        self.focus = Some(ModelCell { row, col });
        self.refresh();
    }

    /// A header click that selects (with its modifier): only `col` is selected; the focus is
    /// `(row, col)`.
    pub fn select_column(&mut self, col: ColId, row: ModelRow) {
        self.parts.clear();
        self.all = false;
        self.add_column(col, row);
    }

    /// Ctrl/Cmd on a header: `col` is added as the active piece.
    pub fn add_column(&mut self, col: ColId, row: ModelRow) {
        let cell = ModelCell { row, col };
        self.anchor = Some(cell);
        self.focus = Some(cell);
        self.pending = None;
        self.parts.push(Part::Cols(span_u32(col.0, col.0)));
        self.refresh();
    }

    /// Shift on a header, and Select column: the active piece becomes the whole columns from
    /// the anchor's column to `col`.
    pub fn extend_columns_to(&mut self, col: ColId) {
        let Some(anchor) = self.anchor else {
            let row = self.focus.map_or(ModelRow(0), |f| f.row);
            return self.select_column(col, row);
        };
        self.leave_all();
        let piece = Part::Cols(span_u32(anchor.col.0, col.0));
        match self.parts.last_mut() {
            Some(last) => *last = piece,
            None => self.parts.push(piece),
        }
        let row = self.focus.map_or(anchor.row, |f| f.row);
        self.focus = Some(ModelCell { row, col });
        self.refresh();
    }

    /// Ctrl/Cmd+A and the corner: everything, in O(1); the focus stays (a pending focus is
    /// dropped: the user chose).
    pub fn select_all(&mut self) {
        self.parts.clear();
        self.all = true;
        self.pending = None;
        self.refresh();
    }

    /// Escape: only the focused cell stays selected. Answers whether anything changed. A
    /// pending focus stays pending.
    pub fn collapse(&mut self, map: &ViewMap) -> bool {
        let before = self.clone();
        match self.focus {
            Some(focus) => self.select_cell(focus, map),
            None => {
                let pending = self.pending;
                *self = Selection::default();
                self.pending = pending;
            }
        }
        *self != before
    }

    /// A new generation (a re-run): the selection clears at once, so nothing old is painted on
    /// the new data, and the focus is clamped into the new grid (`map`'s loaded rows, the
    /// visible columns `order`) and selected alone. When the new generation has no rows (or no
    /// columns) yet, the focus is kept pending and [`Selection::on_rows_loaded`] places it once
    /// rows arrive; a further re-run before then keeps the same pending focus. An append is not
    /// a new generation and keeps the selection.
    pub fn on_generation_changed(&mut self, map: &ViewMap, order: &[ColId]) {
        let focus = self.focus.or(self.pending);
        *self = Selection::default();
        self.pending = focus;
        self.on_rows_loaded(map, order);
    }

    /// Rows arrived (an append, or a re-run's first rows): a pending focus is clamped into the
    /// grid now and selected alone. Answers whether it was placed. Does nothing without a
    /// pending focus, so the view may call it on every append; `apply_command` calls it too.
    pub fn on_rows_loaded(&mut self, map: &ViewMap, order: &[ColId]) -> bool {
        let Some(focus) = self.pending.and_then(|f| clamp_cell(f, map, order)) else {
            return false;
        };
        self.select_cell(focus, map);
        true
    }

    /// Whether `(row, col)` is selected: `all`, or the row set holds its model row, or the
    /// column set holds the column, or a rectangle holds the cell. O(log k) for the sets, O(k)
    /// for the rectangles.
    pub fn contains(&self, row: ViewRow, col: ColId, map: &ViewMap) -> bool {
        if self.all || in_ranges(&self.cols, col.0) {
            return true;
        }
        if map
            .view_to_model(row)
            .is_some_and(|m| in_ranges(&self.rows, m.0))
        {
            return true;
        }
        self.rects.iter().any(|r| r.contains(row.0, col.0))
    }

    /// Whether `row` is selected whole (the gutter's tint).
    pub fn row_selected(&self, row: ModelRow) -> bool {
        self.all || in_ranges(&self.rows, row.0)
    }

    /// Whether `col` is selected whole (the header's tint).
    pub fn col_selected(&self, col: ColId) -> bool {
        self.all || in_ranges(&self.cols, col.0)
    }

    /// The selected cells of a grid of `rows` loaded rows and the visible columns `order`, each
    /// counted once, in `u128`, exactly as many as a copy of the same selection takes: hidden
    /// columns inside a piece are skipped. `|R|·C + |K|·R − |R|·|K|` for the row set R and the
    /// column set K, plus the cells of the rectangles outside both, by a sweep over the
    /// rectangles' row bands. Pieces outside the grid are clipped. O(C) to rank the visible
    /// columns, then O(k²) for k rectangles. (M3a: view rows equal model rows; under a sort or a
    /// filter use [`Selection::cell_count_in`].)
    pub fn cell_count(&self, rows: u64, order: &[ColId]) -> u128 {
        self.count_cells(rows, clip(&self.rows, rows), order)
    }

    // ## m3b-t35
    /// [`Selection::cell_count`] over `map`'s rows: the row set counts the view rows that show
    /// it (rows a filter hides drop out). In the source order this is `cell_count(map.len())`.
    pub fn cell_count_in(&self, map: &ViewMap, order: &[ColId]) -> u128 {
        let rows = map.len();
        let row_set = if map.is_identity() {
            clip(&self.rows, rows)
        } else {
            let views = self
                .rows
                .iter()
                .flat_map(|r| model_rows_to_view(map, r))
                .collect();
            clip(&merge(views), rows)
        };
        self.count_cells(rows, row_set, order)
    }

    /// The count of [`Selection::cell_count`] with the row set `row_set` in view rows (sorted,
    /// merged, clipped to `rows`).
    fn count_cells(&self, rows: u64, row_set: Vec<Range<u64>>, order: &[ColId]) -> u128 {
        // A column counts by its rank among the visible ordinals: a range of ordinals is a range
        // of ranks, and a hidden column has no rank.
        let mut visible: Vec<u32> = order.iter().map(|c| c.0).collect();
        if !visible.is_sorted() {
            visible.sort_unstable();
        }
        visible.dedup();
        let rank = |span: &Range<u32>| -> Range<u64> {
            visible.partition_point(|c| *c < span.start) as u64
                ..visible.partition_point(|c| *c < span.end) as u64
        };
        let (grid_rows, grid_cols) = (u128::from(rows), visible.len() as u128);
        if self.all {
            return grid_rows * grid_cols;
        }
        let col_set = merge(self.cols.iter().map(rank).collect());
        let (r, k) = (len_sum(&row_set), len_sum(&col_set));
        let mut total = r * grid_cols + k * grid_rows - r * k;

        let rects: Vec<(Range<u64>, Range<u64>)> = self
            .rects
            .iter()
            .map(|rect| {
                (
                    rect.rows.start.min(rows)..rect.rows.end.min(rows),
                    rank(&rect.cols),
                )
            })
            .filter(|(rows, cols)| !rows.is_empty() && !cols.is_empty())
            .collect();
        let mut bands: Vec<u64> = rects
            .iter()
            .flat_map(|(rows, _)| [rows.start, rows.end])
            .collect();
        bands.sort_unstable();
        bands.dedup();
        for band in bands.windows(2) {
            let (y0, y1) = (band[0], band[1]);
            let spans = merge(
                rects
                    .iter()
                    .filter(|(rows, _)| rows.start <= y0 && rows.end >= y1)
                    .map(|(_, cols)| cols.clone())
                    .collect(),
            );
            if spans.is_empty() {
                continue;
            }
            let width = len_sum(&spans) - overlap(&spans, &col_set);
            let band = y0..y1;
            let free_rows = u128::from(y1 - y0) - overlap(&row_set, std::slice::from_ref(&band));
            total += free_rows * width;
        }
        total
    }

    /// What a copy takes (HV-GRID-05 step 7), over the loaded rows of `map` and the visible
    /// columns `order` (view order):
    /// - select-all: every loaded row and visible column;
    /// - one piece: its loaded rows and visible columns (a row piece spans every column, a
    ///   column piece every loaded row);
    /// - several pieces: stacked when they cover the same columns, concatenated when they cover
    ///   the same rows, else refused with [`CopyRefusal::MultipleSelection`] (the Excel rule).
    ///
    /// More than [`MAX_COPY_CELLS`] cells is refused with [`CopyRefusal::TooMany`].
    pub fn normalize_for_copy(
        &self,
        map: &ViewMap,
        order: &[ColId],
    ) -> Result<CopyRegion, CopyRefusal> {
        let loaded = map.len();
        let region = if self.all {
            CopyRegion {
                rows: non_empty(0..loaded),
                cols: order.to_vec(),
            }
        } else {
            let every: Vec<usize> = (0..order.len()).collect();
            let covered = |span: &Range<u32>| -> Vec<usize> {
                (0..order.len())
                    .filter(|p| span.contains(&order[*p].0))
                    .collect()
            };
            let clip_rows = |r: &Range<u64>| r.start.min(loaded)..r.end.min(loaded);
            // ## m3b-t35: a piece's rows are a list of view-row ranges (a row set under a sort
            // or a filter shows as several runs).
            let blocks: Vec<(Vec<Range<u64>>, Vec<usize>)> = self
                .parts
                .iter()
                .map(|part| match part {
                    Part::Rect(rect) => (vec![clip_rows(&rect.rows)], covered(&rect.cols)),
                    Part::Rows(rows) if map.is_identity() => (vec![clip_rows(rows)], every.clone()),
                    Part::Rows(rows) => {
                        (clip(&model_rows_to_view(map, rows), loaded), every.clone())
                    }
                    Part::Cols(cols) => (std::iter::once(0..loaded).collect(), covered(cols)),
                })
                .collect();
            stack_or_concatenate(&blocks, order)?
        };
        let cells = region.cell_count();
        if cells > MAX_COPY_CELLS {
            return Err(CopyRefusal::TooMany { cells });
        }
        Ok(region)
    }

    // ## m3b-t35
    /// Where the selection stands on the grow ladder (cell → row segment → row → all) for the
    /// row segment `segment` (visible column indices on screen); `None` for any other shape
    /// (several pieces, a block, a column, a focus a filter hides, nothing).
    pub fn grow_step(
        &self,
        map: &ViewMap,
        order: &[ColId],
        segment: Range<usize>,
    ) -> Option<GrowStep> {
        if self.all {
            return Some(GrowStep::All);
        }
        let focus = self.focus.filter(|f| map.shows(f.row))?;
        let row = corner(focus, map).0;
        match self.parts.as_slice() {
            [Part::Rows(rows)] if *rows == span_u64(focus.row.0, focus.row.0) => {
                Some(GrowStep::Row)
            }
            [Part::Rect(rect)] if rect.rows == span_u64(row, row) => {
                if rect.cols == span_u32(focus.col.0, focus.col.0) {
                    Some(GrowStep::Cell)
                } else if Some(&rect.cols) == row_segment(focus, order, &segment).as_ref() {
                    Some(GrowStep::RowSegment)
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    /// Grow selection (⌥↑ / Alt+Shift+↑ / Ctrl+W): cell → row segment → row → all, from the
    /// focus. A one-row range grows to the segment holding it, else to its row; any other shape
    /// grows to everything. Nothing focused: the first cell. Answers whether anything changed.
    pub fn grow(&mut self, map: &ViewMap, order: &[ColId], segment: Range<usize>) -> bool {
        if map.is_empty() || order.is_empty() || self.all {
            return false;
        }
        let Some(focus) = self.focus.filter(|f| map.shows(f.row)) else {
            let row = map.view_to_model(ViewRow(0)).unwrap_or(ModelRow(0));
            self.select_cell(ModelCell { row, col: order[0] }, map);
            return true;
        };
        if self.parts.is_empty() {
            self.select_cell(focus, map);
            return true;
        }
        let before = self.clone();
        let seg = row_segment(focus, order, &segment);
        let view_row = corner(focus, map).0;
        match self.grow_step(map, order, segment) {
            Some(GrowStep::Cell) => match seg {
                Some(cols) => self.set_row_segment(focus, view_row, cols),
                None if order.len() > 1 => self.set_row(focus),
                None => self.select_all(),
            },
            Some(GrowStep::RowSegment) => self.set_row(focus),
            Some(GrowStep::Row) => self.select_all(),
            Some(GrowStep::All) => {}
            None => match self.parts.as_slice() {
                [Part::Rect(rect)] if rect.rows == span_u64(view_row, view_row) => match seg {
                    Some(cols) if cols.start <= rect.cols.start && rect.cols.end <= cols.end => {
                        self.set_row_segment(focus, view_row, cols)
                    }
                    _ => self.set_row(focus),
                },
                _ => self.select_all(),
            },
        }
        *self != before
    }

    /// Shrink selection (⌥↓ / Alt+Shift+↓ / Ctrl+Shift+W): all → row → row segment → cell, at
    /// the focus; any other shape shrinks to the focus cell. Answers whether anything changed.
    pub fn shrink(&mut self, map: &ViewMap, order: &[ColId], segment: Range<usize>) -> bool {
        if map.is_empty() || order.is_empty() {
            return false;
        }
        let Some(focus) = self.focus.filter(|f| map.shows(f.row)) else {
            return false;
        };
        let before = self.clone();
        let seg = row_segment(focus, order, &segment);
        let view_row = corner(focus, map).0;
        match self.grow_step(map, order, segment) {
            Some(GrowStep::All) => self.set_row(focus),
            Some(GrowStep::Row) => match seg {
                Some(cols) => self.set_row_segment(focus, view_row, cols),
                None => self.select_cell(focus, map),
            },
            Some(GrowStep::RowSegment) => self.select_cell(focus, map),
            Some(GrowStep::Cell) => {}
            None if self.parts.is_empty() && !self.all => {}
            None => self.select_cell(focus, map),
        }
        *self != before
    }

    /// Only the focus row's cells in the column span `cols`, at view row `view_row`.
    fn set_row_segment(&mut self, focus: ModelCell, view_row: u64, cols: Range<u32>) {
        self.all = false;
        self.parts.clear();
        self.parts.push(Part::Rect(CellRect {
            rows: span_u64(view_row, view_row),
            cols,
        }));
        self.anchor = Some(focus);
        self.focus = Some(focus);
        self.refresh();
    }

    /// Only the focus row (a model row piece, so it follows the data through a re-sort).
    fn set_row(&mut self, focus: ModelCell) {
        self.all = false;
        self.parts.clear();
        self.select_row(focus.row, focus.col);
    }

    /// Select-all is the active piece (nothing was added after it): extending replaces it.
    fn leave_all(&mut self) {
        if self.all && self.parts.is_empty() {
            self.all = false;
        }
    }

    fn refresh(&mut self) {
        self.rects = self
            .parts
            .iter()
            .filter_map(|p| match p {
                Part::Rect(r) => Some(r.clone()),
                _ => None,
            })
            .collect();
        self.rows = merge(
            self.parts
                .iter()
                .filter_map(|p| match p {
                    Part::Rows(r) => Some(r.clone()),
                    _ => None,
                })
                .collect(),
        );
        self.cols = merge(
            self.parts
                .iter()
                .filter_map(|p| match p {
                    Part::Cols(c) => Some(c.clone()),
                    _ => None,
                })
                .collect(),
        );
    }
}

/// What a copy takes: row ranges (view rows, in view order) and columns (view order).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CopyRegion {
    pub rows: Vec<Range<u64>>,
    pub cols: Vec<ColId>,
}

impl CopyRegion {
    pub fn row_count(&self) -> u64 {
        self.rows.iter().map(|r| r.end - r.start).sum()
    }

    pub fn cell_count(&self) -> u128 {
        u128::from(self.row_count()) * self.cols.len() as u128
    }
}

/// Why a copy was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CopyRefusal {
    /// Disjoint rectangles that neither stack nor concatenate.
    MultipleSelection,
    TooMany {
        cells: u128,
    },
}

/// The Excel rule over `(view-row ranges, column positions)` blocks.
fn stack_or_concatenate(
    blocks: &[(Vec<Range<u64>>, Vec<usize>)],
    order: &[ColId],
) -> Result<CopyRegion, CopyRefusal> {
    let Some((first_rows, first_cols)) = blocks.first() else {
        return Ok(CopyRegion {
            rows: Vec::new(),
            cols: Vec::new(),
        });
    };
    let ids = |positions: &[usize]| positions.iter().map(|p| order[*p]).collect();
    if blocks.iter().all(|(_, cols)| cols == first_cols) {
        return Ok(CopyRegion {
            rows: merge(
                blocks
                    .iter()
                    .flat_map(|(rows, _)| rows.iter().cloned())
                    .collect(),
            ),
            cols: ids(first_cols),
        });
    }
    if blocks.iter().all(|(rows, _)| rows == first_rows) {
        let mut cols: Vec<usize> = blocks
            .iter()
            .flat_map(|(_, cols)| cols.iter().copied())
            .collect();
        cols.sort_unstable();
        cols.dedup();
        return Ok(CopyRegion {
            rows: first_rows.iter().cloned().flat_map(non_empty).collect(),
            cols: ids(&cols),
        });
    }
    Err(CopyRefusal::MultipleSelection)
}

// ## m3b-t35
/// A step of growing the selection ([`Selection::grow`]): the focus cell, the focus row across
/// the on-screen run of columns holding it, the focus row, everything.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GrowStep {
    Cell,
    RowSegment,
    Row,
    All,
}

/// The column span (ordinals) of the row segment for `focus`: the on-screen run `segment`
/// (visible indices) when it holds the focus column and is neither that column alone nor the
/// whole row.
fn row_segment(focus: ModelCell, order: &[ColId], segment: &Range<usize>) -> Option<Range<u32>> {
    let seg = segment.start..segment.end.min(order.len());
    let at = order.iter().position(|c| *c == focus.col)?;
    if !seg.contains(&at) || seg.len() < 2 || seg == (0..order.len()) {
        return None;
    }
    Some(order[seg.start].0..order[seg.end - 1].0.saturating_add(1))
}

/// `(view row, column position)` of a cell.
fn corner(cell: ModelCell, map: &ViewMap) -> (u64, u32) {
    (view_of(cell.row, map), cell.col.0)
}

/// The view row of `row`; its own number when `map` does not show it.
fn view_of(row: ModelRow, map: &ViewMap) -> u64 {
    map.model_to_view(row).map_or(row.0, |v| v.0)
}

/// The view rows that show model rows `rows`, sorted and merged (rows a filter hides drop
/// out). The identity in the source order; under a sort, the tail is one run and the sorted
/// rows are looked up one by one (O(k log k) for k rows), except a range covering every sorted
/// row, which is the sorted part whole.
fn model_rows_to_view(map: &ViewMap, rows: &Range<u64>) -> Vec<Range<u64>> {
    match map {
        ViewMap::Identity { .. } => non_empty(rows.clone()),
        ViewMap::Permuted {
            order, tail_from, ..
        } => {
            let sorted = order.len() as u64;
            let mut out = Vec::new();
            let head = rows.start..rows.end.min(*tail_from);
            if head.start == 0 && head.end == *tail_from && sorted == *tail_from {
                out.extend(non_empty(0..sorted));
            } else if !head.is_empty() {
                out.extend(
                    head.filter_map(|m| map.model_to_view(ModelRow(m)))
                        .map(|v| v.0..v.0 + 1),
                );
            }
            let tail = rows.start.max(*tail_from)..rows.end;
            if !tail.is_empty() {
                out.push(sorted + (tail.start - tail_from)..sorted + (tail.end - tail_from));
            }
            merge(out)
        }
        ViewMap::Filtered { rows: kept, .. } => {
            let span = rows.clone();
            if span.end.saturating_sub(span.start) > kept.len() as u64 {
                // Fewer kept rows than model rows asked: walk the kept rows instead.
                let views = kept
                    .iter()
                    .enumerate()
                    .filter(|(_, m)| span.contains(m))
                    .map(|(v, _)| v as u64..v as u64 + 1)
                    .collect();
                return merge(views);
            }
            merge(
                span.filter_map(|m| map.model_to_view(ModelRow(m)))
                    .map(|v| v.0..v.0 + 1)
                    .collect(),
            )
        }
    }
}

/// `cell` moved into a grid of `map`'s loaded rows and the visible columns `order`; `None` when
/// the grid has no cell. A column that is gone snaps to the next visible column, or the last.
fn clamp_cell(cell: ModelCell, map: &ViewMap, order: &[ColId]) -> Option<ModelCell> {
    if map.is_empty() || order.is_empty() {
        return None;
    }
    let (view, _) = corner(cell, map);
    let row = map
        .view_to_model(ViewRow(view.min(map.len() - 1)))
        .unwrap_or(cell.row);
    let col = if order.contains(&cell.col) {
        cell.col
    } else {
        order
            .iter()
            .copied()
            .filter(|c| c.0 > cell.col.0)
            .min_by_key(|c| c.0)
            .or_else(|| order.iter().copied().max_by_key(|c| c.0))?
    };
    Some(ModelCell { row, col })
}

fn span_u64(a: u64, b: u64) -> Range<u64> {
    a.min(b)..a.max(b).saturating_add(1)
}

fn span_u32(a: u32, b: u32) -> Range<u32> {
    a.min(b)..a.max(b).saturating_add(1)
}

fn non_empty(r: Range<u64>) -> Vec<Range<u64>> {
    if r.is_empty() { Vec::new() } else { vec![r] }
}

/// Sorted, merged (overlapping and touching ranges joined), without empty ranges.
fn merge<T: Ord + Copy>(mut ranges: Vec<Range<T>>) -> Vec<Range<T>> {
    ranges.retain(|r| r.start < r.end);
    ranges.sort_unstable_by_key(|r| r.start);
    let mut out: Vec<Range<T>> = Vec::with_capacity(ranges.len());
    for r in ranges {
        match out.last_mut() {
            Some(last) if r.start <= last.end => last.end = last.end.max(r.end),
            _ => out.push(r),
        }
    }
    out
}

/// Sorted merged ranges clipped to `[0, limit)`.
fn clip<T: Ord + Copy>(ranges: &[Range<T>], limit: T) -> Vec<Range<T>> {
    ranges
        .iter()
        .map(|r| r.start.min(limit)..r.end.min(limit))
        .filter(|r| r.start < r.end)
        .collect()
}

/// Whether sorted merged `ranges` hold `x`, in O(log k).
fn in_ranges<T: Ord + Copy>(ranges: &[Range<T>], x: T) -> bool {
    let i = ranges.partition_point(|r| r.end <= x);
    ranges.get(i).is_some_and(|r| r.start <= x)
}

fn len_sum<T: Copy + Into<u128>>(ranges: &[Range<T>]) -> u128 {
    ranges.iter().map(|r| r.end.into() - r.start.into()).sum()
}

/// The size of the intersection of two sorted merged range lists.
fn overlap<T: Ord + Copy + Into<u128>>(a: &[Range<T>], b: &[Range<T>]) -> u128 {
    let (mut i, mut j, mut total) = (0, 0, 0u128);
    while i < a.len() && j < b.len() {
        let start = a[i].start.max(b[j].start);
        let end = a[i].end.min(b[j].end);
        if start < end {
            total += end.into() - start.into();
        }
        if a[i].end < b[j].end {
            i += 1;
        } else {
            j += 1;
        }
    }
    total
}
