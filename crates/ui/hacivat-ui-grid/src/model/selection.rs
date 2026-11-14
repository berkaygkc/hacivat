//! The selection model (HV-GRID-05; Task 17): anchor and focus, rectangles in view coordinates,
//! row and column sets, select-all; cell counts in `u128` by inclusion–exclusion; copy
//! normalisation (a union that is neither stacked nor concatenable is refused; more than
//! [`MAX_COPY_CELLS`] is refused); a generation change clears the selection and clamps the
//! focus, an append keeps it.
//!
//! Coordinates (digest 04 ruling R15):
//! - the focus, the anchor and the row set are **model rows**, so they follow the data through
//!   a local sort (M3b);
//! - rectangles are **view rows**;
//! - a column is counted by its position in the full view order (`ColumnLayout::order`, hidden
//!   columns included). M3a never reorders columns, so that position is the `ColId` ordinal. A
//!   hidden column inside a rectangle stays in it; copy skips it.
//!
//! M3a has no local sort: [`ViewMap`] is the identity, so view rows equal model rows, and
//! [`Selection::cell_count`] relies on that.

use std::ops::Range;

use crate::source::{ColId, ModelCell, ModelRow, ViewRow};

/// The most cells a copy takes (HV-XFER-02 step 2); a larger selection is refused.
pub const MAX_COPY_CELLS: u128 = 1_000_000;

/// How view rows map to model rows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ViewMap {
    /// No local sort or filter (all of M3a): view row `n` is model row `n`. `loaded` rows
    /// exist; a row past them maps to itself (a placeholder).
    Identity { loaded: u64 },
}

impl ViewMap {
    pub fn identity(loaded: u64) -> Self {
        ViewMap::Identity { loaded }
    }

    /// The loaded rows.
    pub fn len(&self) -> u64 {
        match self {
            ViewMap::Identity { loaded } => *loaded,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn view_to_model(&self, row: ViewRow) -> Option<ModelRow> {
        match self {
            ViewMap::Identity { .. } => Some(ModelRow(row.0)),
        }
    }

    pub fn model_to_view(&self, row: ModelRow) -> Option<ViewRow> {
        match self {
            ViewMap::Identity { .. } => Some(ViewRow(row.0)),
        }
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

    /// Nothing is selected.
    pub fn is_empty(&self) -> bool {
        !self.all && self.parts.is_empty()
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

    /// Ctrl/Cmd+A and the corner: everything, in O(1); the focus stays.
    pub fn select_all(&mut self) {
        self.parts.clear();
        self.all = true;
        self.refresh();
    }

    /// Escape: only the focused cell stays selected. Answers whether anything changed.
    pub fn collapse(&mut self, map: &ViewMap) -> bool {
        let before = self.clone();
        match self.focus {
            Some(focus) => self.select_cell(focus, map),
            None => *self = Selection::default(),
        }
        *self != before
    }

    /// A new generation (a re-run): the selection clears and the focus is clamped into the new
    /// grid (`map`'s loaded rows, the visible columns `order`) and selected alone. With no rows
    /// or no columns there is no cell to focus. An append is not a new generation and keeps
    /// the selection.
    pub fn on_generation_changed(&mut self, map: &ViewMap, order: &[ColId]) {
        let focus = self.focus.and_then(|f| clamp_cell(f, map, order));
        *self = Selection::default();
        if let Some(focus) = focus {
            self.select_cell(focus, map);
        }
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

    /// The selected cells of a `rows` × `cols` grid, each counted once, in `u128`:
    /// `|R|·C + |K|·R − |R|·|K|` for the row set R and the column set K, plus the cells of the
    /// rectangles outside both, by a sweep over the rectangles' row bands. Pieces outside the
    /// grid are clipped. (M3a: view rows equal model rows.)
    pub fn cell_count(&self, rows: u64, cols: u32) -> u128 {
        let (grid_rows, grid_cols) = (u128::from(rows), u128::from(cols));
        if self.all {
            return grid_rows * grid_cols;
        }
        let row_set = clip(&self.rows, rows);
        let col_set = clip(&self.cols, cols);
        let (r, k) = (len_sum(&row_set), len_sum(&col_set));
        let mut total = r * grid_cols + k * grid_rows - r * k;

        let rects: Vec<CellRect> = self
            .rects
            .iter()
            .map(|rect| CellRect {
                rows: rect.rows.start.min(rows)..rect.rows.end.min(rows),
                cols: rect.cols.start.min(cols)..rect.cols.end.min(cols),
            })
            .filter(|rect| !rect.rows.is_empty() && !rect.cols.is_empty())
            .collect();
        let mut bands: Vec<u64> = rects
            .iter()
            .flat_map(|rect| [rect.rows.start, rect.rows.end])
            .collect();
        bands.sort_unstable();
        bands.dedup();
        for band in bands.windows(2) {
            let (y0, y1) = (band[0], band[1]);
            let spans = merge(
                rects
                    .iter()
                    .filter(|rect| rect.rows.start <= y0 && rect.rows.end >= y1)
                    .map(|rect| rect.cols.clone())
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
            let blocks: Vec<(Range<u64>, Vec<usize>)> = self
                .parts
                .iter()
                .map(|part| match part {
                    Part::Rect(rect) => (clip_rows(&rect.rows), covered(&rect.cols)),
                    Part::Rows(rows) => (clip_rows(&model_rows_to_view(map, rows)), every.clone()),
                    Part::Cols(cols) => (0..loaded, covered(cols)),
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

/// The Excel rule over `(rows, column positions)` blocks.
fn stack_or_concatenate(
    blocks: &[(Range<u64>, Vec<usize>)],
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
            rows: merge(blocks.iter().map(|(rows, _)| rows.clone()).collect()),
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
            rows: non_empty(first_rows.clone()),
            cols: ids(&cols),
        });
    }
    Err(CopyRefusal::MultipleSelection)
}

/// `(view row, column position)` of a cell.
fn corner(cell: ModelCell, map: &ViewMap) -> (u64, u32) {
    let row = map.model_to_view(cell.row).map_or(cell.row.0, |v| v.0);
    (row, cell.col.0)
}

/// The view rows of model rows `rows` (the identity in M3a).
fn model_rows_to_view(map: &ViewMap, rows: &Range<u64>) -> Range<u64> {
    match map {
        ViewMap::Identity { .. } => rows.clone(),
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
