//! A row of cells that gives way whole (M3a follow-up F2; design/m3a-query NOTES cross-cutting
//! #7, "the Overflow Rule decides, not ellipsis"): when the row is narrower than its cells, the
//! cells that may give way leave it whole, in their order, before any chrome would ellipsize; a
//! data cell (the status bar's message, whose SQL is data) instead ellipsizes down to a floor
//! when its turn in that order comes. A trust signal never gives way.
//!
//! How it fits. In `request_layout` every cell is laid out once at its natural width
//! (`AnyElement::layout_as_root`), and the row asks for the sum of those widths, shrinkable to
//! nothing: its container decides the room. In `prepaint` the pure [`give_way`] picks what stays
//! and how wide, and the same cell elements are laid out again at those widths, side by side.
//! A cell that gave way is never prepainted or painted, so it leaves no trace: no seam, no
//! sliver, no recorded label. The room left over (less than the next cell that gave way) goes
//! to the last cell shown, so the row always ends where its container does.
//!
//! A cell fills the width it is given (`w_full`): one that receives the room left over grows.

use hacivat_ui_core::gpui_surface::{
    AnyElement, App, AvailableSpace, Bounds, Element, ElementId, GlobalElementId,
    InspectorElementId, LayoutId, Pixels, Style, Window, point, px, relative, size,
};

/// How a cell gives way.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Yield {
    /// A trust signal: never.
    Never,
    /// Leaves the row whole when it does not fit: the highest rank first, a tie going to the
    /// later cell.
    Whole(u8),
    /// Data that may ellipsize: at its rank's turn it shrinks, down to `floor` px, and stays.
    Shrink { floor: f32, rank: u8 },
}

/// What [`give_way`] decides for each cell: its width, or `None` when it gave way.
pub type Widths = Vec<Option<f32>>;

/// The widths cells of natural `widths` take in `available` px (see the module docs). When even
/// the cells that never give way and the floors do not fit, the row is wider than `available`
/// (its container clips it); that is the caller's ordering to fix, never a reason to cut a trust
/// signal.
pub fn give_way(yields: &[Yield], widths: &[f32], available: f32) -> Widths {
    debug_assert_eq!(yields.len(), widths.len());
    let mut out: Widths = widths.iter().map(|w| Some(*w)).collect();
    let total = |out: &Widths| out.iter().flatten().sum::<f32>();
    let rank = |i: usize| match yields[i] {
        Yield::Whole(r) | Yield::Shrink { rank: r, .. } => Some(r),
        Yield::Never => None,
    };
    let mut order: Vec<usize> = (0..yields.len()).filter(|&i| rank(i).is_some()).collect();
    order.sort_by(|&a, &b| rank(b).cmp(&rank(a)).then(b.cmp(&a)));
    for i in order {
        let excess = total(&out) - available;
        if excess <= 0.0 {
            break;
        }
        out[i] = match (yields[i], out[i]) {
            (Yield::Shrink { floor, .. }, Some(w)) => Some(w - (w - floor.min(w)).min(excess)),
            _ => None,
        };
    }
    // Fix round 1 (M4): room a later cell freed goes back first to data that shrank, up to its
    // natural width; what is left goes to the last cell shown.
    for i in 0..yields.len() {
        let slack = available - total(&out);
        if slack <= 0.0 {
            break;
        }
        if let (Yield::Shrink { .. }, Some(w)) = (yields[i], out[i]) {
            out[i] = Some((w + slack).min(widths[i].max(w)));
        }
    }
    let slack = available - total(&out);
    if slack > 0.0
        && let Some(last) = out.iter_mut().rev().flatten().next()
    {
        *last += slack;
    }
    out
}

/// A row of cells that gives way whole (see the module docs).
pub struct GiveWayRow {
    cells: Vec<(Yield, AnyElement)>,
    margin_left: f32,
    margin_right: f32,
}

impl GiveWayRow {
    pub fn new(cells: Vec<(Yield, AnyElement)>) -> Self {
        Self {
            cells,
            margin_left: 0.0,
            margin_right: 0.0,
        }
    }

    /// Margins on the row's sides (negative to cover its container's padding).
    pub fn margins(mut self, left: f32, right: f32) -> Self {
        self.margin_left = left;
        self.margin_right = right;
        self
    }
}

impl Element for GiveWayRow {
    /// The cells' natural widths.
    type RequestLayoutState = Vec<f32>;
    /// The cells shown, with their widths.
    type PrepaintState = Widths;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Vec<f32>) {
        let natural = size(AvailableSpace::MaxContent, AvailableSpace::MaxContent);
        let widths: Vec<f32> = self
            .cells
            .iter_mut()
            .map(|(_, cell)| f32::from(cell.layout_as_root(natural, window, cx).width))
            .collect();
        let mut style = Style::default();
        style.size.width = px(widths.iter().sum::<f32>()).into();
        style.size.height = relative(1.0).into();
        style.min_size.width = px(0.0).into();
        style.flex_shrink = 1.0;
        style.margin.left = px(self.margin_left).into();
        style.margin.right = px(self.margin_right).into();
        (window.request_layout(style, [], cx), widths)
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        widths: &mut Vec<f32>,
        window: &mut Window,
        cx: &mut App,
    ) -> Widths {
        let yields: Vec<Yield> = self.cells.iter().map(|(y, _)| *y).collect();
        let shown = give_way(&yields, widths, f32::from(bounds.size.width));
        let mut x = bounds.left();
        for ((_, cell), width) in self.cells.iter_mut().zip(&shown) {
            let Some(width) = *width else {
                continue;
            };
            cell.layout_as_root(
                size(
                    AvailableSpace::Definite(px(width)),
                    AvailableSpace::Definite(bounds.size.height),
                ),
                window,
                cx,
            );
            cell.prepaint_at(point(x, bounds.top()), window, cx);
            x += px(width);
        }
        shown
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _widths: &mut Vec<f32>,
        shown: &mut Widths,
        window: &mut Window,
        cx: &mut App,
    ) {
        for ((_, cell), width) in self.cells.iter_mut().zip(shown.iter()) {
            if width.is_some() {
                cell.paint(window, cx);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Yield, give_way};

    const W: Yield = Yield::Never;

    #[test]
    fn everything_fits_and_the_last_cell_takes_the_room_left() {
        let y = [W, Yield::Whole(1), Yield::Whole(2)];
        assert_eq!(
            give_way(&y, &[10.0, 20.0, 30.0], 60.0),
            vec![Some(10.0), Some(20.0), Some(30.0)]
        );
        assert_eq!(
            give_way(&y, &[10.0, 20.0, 30.0], 65.0),
            vec![Some(10.0), Some(20.0), Some(35.0)]
        );
    }

    /// The highest rank gives way first, whole; the room it leaves goes to the last cell shown,
    /// never to a sliver of the cell that gave way.
    #[test]
    fn cells_give_way_whole_highest_rank_first() {
        let y = [W, Yield::Whole(2), Yield::Whole(1), Yield::Whole(3)];
        let w = [10.0, 20.0, 30.0, 40.0];
        assert_eq!(
            give_way(&y, &w, 95.0),
            vec![Some(10.0), Some(20.0), Some(65.0), None]
        );
        assert_eq!(
            give_way(&y, &w, 45.0),
            vec![Some(10.0), None, Some(35.0), None]
        );
        // A tie goes to the later cell.
        let tie = [Yield::Whole(1), Yield::Whole(1)];
        assert_eq!(give_way(&tie, &[10.0, 10.0], 15.0), vec![Some(15.0), None]);
    }

    /// A trust signal never gives way: past the last cell that could, the row is wider than
    /// its room (its container clips it) rather than a trust signal cut.
    #[test]
    fn a_trust_signal_never_gives_way() {
        let y = [W, Yield::Whole(1), W];
        assert_eq!(
            give_way(&y, &[50.0, 20.0, 50.0], 60.0),
            vec![Some(50.0), None, Some(50.0)]
        );
    }

    /// Data ellipsizes down to its floor at its rank's turn: after the cells ranked above it
    /// gave way, before the ones ranked below it; it never leaves the row.
    #[test]
    fn data_shrinks_to_its_floor_at_its_turn() {
        let msg = Yield::Shrink {
            floor: 30.0,
            rank: 2,
        };
        let y = [W, Yield::Whole(3), msg, Yield::Whole(1)];
        let w = [10.0, 20.0, 80.0, 20.0];
        // The higher-ranked cell goes first, whole.
        assert_eq!(
            give_way(&y, &w, 110.0),
            vec![Some(10.0), None, Some(80.0), Some(20.0)]
        );
        // Then the data shrinks, and the lower-ranked cell stays.
        assert_eq!(
            give_way(&y, &w, 90.0),
            vec![Some(10.0), None, Some(60.0), Some(20.0)]
        );
        // Below its floor, the lower-ranked cell gives way too, the room left to the last.
        assert_eq!(
            give_way(&y, &w, 50.0),
            vec![Some(10.0), None, Some(40.0), None]
        );
        assert_eq!(
            give_way(&y, &w, 20.0),
            vec![Some(10.0), None, Some(30.0), None]
        );
        // Fix round 1 (M4): data that shrank before a lower-ranked cell left gets the room
        // that cell freed back, up to its natural width; only the rest goes to the last cell.
        let y = [msg, Yield::Whole(1), W];
        let w = [80.0, 50.0, 10.0];
        assert_eq!(give_way(&y, &w, 70.0), vec![Some(60.0), None, Some(10.0)]);
        assert_eq!(give_way(&y, &w, 45.0), vec![Some(35.0), None, Some(10.0)]);
        // A floor wider than the cell keeps the cell's own width.
        let wide = Yield::Shrink {
            floor: 50.0,
            rank: 1,
        };
        assert_eq!(give_way(&[wide], &[20.0], 10.0), vec![Some(20.0)]);
    }
}
