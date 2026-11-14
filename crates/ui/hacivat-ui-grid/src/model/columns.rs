//! Column layout (HV-GRID-02 step 6; Task 17): view order, widths, `f64` prefix sums over the
//! visible columns, hidden columns. A width change updates only the suffix of the prefix sums;
//! hide and show keep `left_x` anchored to the first visible column.

use std::ops::Range;

use crate::source::{ColId, ColumnSpec};

/// The columns as the grid lays them out.
///
/// Widths are per column (`f32`: a single width is a small number); every position across
/// columns is an `f64` prefix sum, exact to 2^53 px.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ColumnLayout {
    /// Every column in view order, hidden ones included.
    order: Vec<ColId>,
    /// Parallel to `order`; a hidden column keeps its width for when it is shown again.
    widths: Vec<f32>,
    /// Parallel to `order`.
    hidden_flags: Vec<bool>,
    /// The hidden columns, in view order.
    hidden: Vec<ColId>,
    /// The visible columns, in view order.
    visible: Vec<ColId>,
    /// For each visible column, its index in `order` (ascending).
    slots: Vec<u32>,
    /// The left edge of each visible column, then the total width: `visible.len() + 1` entries.
    prefix: Vec<f64>,
}

impl ColumnLayout {
    /// Columns in view order with their widths (a width that is not finite and non-negative is
    /// read as 0).
    pub fn new(columns: impl IntoIterator<Item = (ColId, f32)>) -> Self {
        let (order, widths): (Vec<ColId>, Vec<f32>) = columns
            .into_iter()
            .map(|(id, w)| (id, if valid_width(w) { w } else { 0.0 }))
            .unzip();
        let hidden_flags = vec![false; order.len()];
        let mut layout = ColumnLayout {
            order,
            widths,
            hidden_flags,
            ..ColumnLayout::default()
        };
        layout.rebuild();
        layout
    }

    /// The source's columns at their default widths.
    pub fn from_specs(specs: &[ColumnSpec]) -> Self {
        ColumnLayout::new(specs.iter().map(|s| (s.id, s.default_width)))
    }

    /// Every column in view order, hidden ones included.
    pub fn order(&self) -> &[ColId] {
        &self.order
    }

    /// Hidden columns, in view order.
    pub fn hidden(&self) -> &[ColId] {
        &self.hidden
    }

    /// Visible columns, in view order: the grid's column indices index this slice.
    pub fn visible(&self) -> &[ColId] {
        &self.visible
    }

    /// The number of visible columns.
    pub fn len(&self) -> usize {
        self.visible.len()
    }

    pub fn is_empty(&self) -> bool {
        self.visible.is_empty()
    }

    /// The width of the visible columns together.
    pub fn total_width(&self) -> f64 {
        self.prefix.last().copied().unwrap_or(0.0)
    }

    /// The left edge of visible column `ix`; `len()` (or more) gives the total width.
    pub fn x_of(&self, ix: usize) -> f64 {
        self.prefix
            .get(ix)
            .copied()
            .unwrap_or_else(|| self.total_width())
    }

    /// The width of visible column `ix` (0 past the end).
    pub fn width_at(&self, ix: usize) -> f32 {
        self.slots
            .get(ix)
            .map_or(0.0, |slot| self.widths[*slot as usize])
    }

    /// The width of `id`, hidden or not.
    pub fn width_of(&self, id: ColId) -> Option<f32> {
        self.slot_of(id).map(|slot| self.widths[slot])
    }

    /// The visible index of `id`; `None` when it is hidden or unknown.
    pub fn index_of(&self, id: ColId) -> Option<usize> {
        let slot = self.slot_of(id)?;
        if self.hidden_flags[slot] {
            return None;
        }
        self.slots.binary_search(&(slot as u32)).ok()
    }

    /// The visible columns a body `width` px wide shows from `left_x`:
    /// `c0 = upper_bound(X, left_x) − 1`, `c1 = lower_bound(X, left_x + width)` over the left
    /// edges `X`, in `O(log C)`.
    pub fn visible_range(&self, left_x: f64, width: f32) -> Range<usize> {
        let n = self.len();
        if n == 0 {
            return 0..0;
        }
        let left = if left_x.is_finite() {
            left_x.max(0.0)
        } else {
            0.0
        };
        if left >= self.total_width() {
            return n..n;
        }
        let lefts = &self.prefix[..n];
        let c0 = lefts.partition_point(|x| *x <= left).saturating_sub(1);
        if !width.is_finite() || width <= 0.0 {
            return c0..c0;
        }
        let right = left + f64::from(width);
        let c1 = lefts.partition_point(|x| *x < right);
        c0..c1.max(c0)
    }

    /// The visible column at content `x` (pointer hit testing through the inverse prefix sums).
    pub fn col_at_x(&self, x: f64) -> Option<usize> {
        let n = self.len();
        if !x.is_finite() || x < 0.0 || x >= self.total_width() {
            return None;
        }
        let ix = self.prefix[..n].partition_point(|e| *e <= x);
        Some(ix.saturating_sub(1))
    }

    /// Sets the width of `id` (a hidden column's too). Only the prefix sums after the column
    /// change. `false` for an unknown column or a width that is not finite and non-negative.
    pub fn set_width(&mut self, id: ColId, width: f32) -> bool {
        if !valid_width(width) {
            return false;
        }
        let Some(slot) = self.slot_of(id) else {
            return false;
        };
        self.widths[slot] = width;
        if let Some(ix) = self.index_of(id) {
            for j in ix..self.visible.len() {
                self.prefix[j + 1] =
                    self.prefix[j] + f64::from(self.widths[self.slots[j] as usize]);
            }
        }
        true
    }

    /// Hides `id` and answers the `left_x` that keeps the first visible column where it was on
    /// screen, clamped to the new content for a body `body_w` px wide.
    ///
    /// The caller then passes [`ColumnLayout::total_width`] to `ScrollState::set_content_width`
    /// **before** `ScrollState::set_left_x(returned)`: `set_left_x` clamps to the content width
    /// it knows, so the other order clips a view that showing a column moved right.
    pub fn hide(&mut self, id: ColId, left_x: f64, body_w: f32) -> f64 {
        self.set_hidden(&[id], true, left_x, body_w)
    }

    /// Shows `id` and answers the `left_x` that keeps the first visible column where it was on
    /// screen (at the left edge the view stays at the left edge, so the column shows), clamped
    /// as [`ColumnLayout::hide`] is, with the same call order after it.
    pub fn show(&mut self, id: ColId, left_x: f64, body_w: f32) -> f64 {
        self.set_hidden(&[id], false, left_x, body_w)
    }

    /// Shows every column; answers the anchored, clamped `left_x` (as [`ColumnLayout::show`]).
    pub fn show_all(&mut self, left_x: f64, body_w: f32) -> f64 {
        let hidden = self.hidden.clone();
        self.set_hidden(&hidden, false, left_x, body_w)
    }

    fn set_hidden(&mut self, ids: &[ColId], hide: bool, left_x: f64, body_w: f32) -> f64 {
        let slots: Vec<usize> = ids
            .iter()
            .filter_map(|id| self.slot_of(*id))
            .filter(|slot| self.hidden_flags[*slot] != hide)
            .collect();
        if slots.is_empty() {
            return left_x;
        }
        let anchor = self.anchor(left_x);
        for slot in slots {
            self.hidden_flags[slot] = hide;
        }
        self.rebuild();
        let anchored = match anchor {
            None => 0.0,
            Some(a) => match self.index_of(a.col) {
                Some(ix) => self.prefix[ix] + a.offset,
                // The anchor column itself was hidden: the columns before it did not move, so
                // its old left edge is where the next visible column now starts.
                None => a.left_edge,
            },
        };
        let body = if valid_width(body_w) {
            f64::from(body_w)
        } else {
            0.0
        };
        anchored.clamp(0.0, (self.total_width() - body).max(0.0))
    }

    /// The first visible column at `left_x`, how far into it the view starts, and its left
    /// edge; `None` at the left edge or with no columns.
    fn anchor(&self, left_x: f64) -> Option<Anchor> {
        let n = self.len();
        if n == 0 || !left_x.is_finite() || left_x <= 0.0 {
            return None;
        }
        let ix = self.prefix[..n]
            .partition_point(|e| *e <= left_x)
            .saturating_sub(1);
        Some(Anchor {
            col: self.visible[ix],
            offset: left_x - self.prefix[ix],
            left_edge: self.prefix[ix],
        })
    }

    /// The index of `id` in `order`: by its ordinal when the layout is in source order (M3a
    /// never reorders), else by a scan.
    fn slot_of(&self, id: ColId) -> Option<usize> {
        let ix = id.0 as usize;
        if self.order.get(ix) == Some(&id) {
            return Some(ix);
        }
        self.order.iter().position(|c| *c == id)
    }

    fn rebuild(&mut self) {
        self.visible.clear();
        self.hidden.clear();
        self.slots.clear();
        self.prefix.clear();
        self.prefix.push(0.0);
        let mut x = 0.0;
        for (slot, id) in self.order.iter().enumerate() {
            if self.hidden_flags[slot] {
                self.hidden.push(*id);
            } else {
                self.visible.push(*id);
                self.slots.push(slot as u32);
                x += f64::from(self.widths[slot]);
                self.prefix.push(x);
            }
        }
    }
}

/// Where the view starts relative to its first visible column.
#[derive(Debug, Clone, Copy)]
struct Anchor {
    col: ColId,
    offset: f64,
    left_edge: f64,
}

fn valid_width(w: f32) -> bool {
    w.is_finite() && w >= 0.0
}
