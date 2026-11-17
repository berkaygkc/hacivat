//! The grid's AccessKit rows and cells (HV-GRID-16; M3b Task 40): zero-paint row and cell
//! children of the grid's node while assistive technology is active, the active-descendant
//! focus, and the live announcements.
//!
//! ## m3b-t01b-seams: the hook (Task 1b), called once per frame by `grid::prepaint_frame` once
//! the frame's geometry is known, so Task 40 never edits `grid.rs`.
//!
//! ## m3c-t40: filled (R-M3-69).
//!
//! - **Zero cost without assistive technology.** While `Window::is_a11y_active` is false the
//!   hook returns at once (and drops what an earlier active frame built), and the view's render
//!   adds nothing: no node, no label, no string is built ([`GridView::a11y_builds`] stays 0).
//! - **The subtree.** [`visible_nodes`] is pure: the header row and the view rows on screen ±
//!   [`A11Y_OVERSCAN_ROWS`], each with a cell per visible column (the frozen ones first, then
//!   the scrolling ones on screen). A row's index is its view row + 1 (the header is row 0); a
//!   cell's column index is its place among the visible columns. Rows past the loaded ones are
//!   one "loading" row each, without cells.
//! - **Ids.** A data row is the element `("hv-grid-a11y-row", model row)` under the grid's own
//!   element (its id carries the grid's entity), a cell the synthetic child keyed by its column:
//!   the node ids come from `(grid, model row, column)`, so they stay put across a scroll and a
//!   sort.
//! - **Names.** A cell reads its source's screen-reader text ([`GridFrame::a11y_cell`]: the
//!   app's `Purpose::A11y`), at most [`A11Y_NAME_MAX_CHARS`] characters (then `…`); NULL reads
//!   `hui-grid-a11y-null`, a truncated value `hui-grid-a11y-truncated` with its full size, a
//!   placeholder the grid's "loading" label. A header cell reads its column's name and carries
//!   its sort direction, with the sort tooltip (level, direction, collation) as its description.
//! - **Elements.** The view's render (one frame behind the hook, like the header tooltips)
//!   adds one zero-paint `Row` element per row at the row's place, whose cells are synthetic
//!   leaves with their bounds; the grid's node (the view's focused root) names the focus cell its
//!   **active descendant** and holds the polite live region. A canvas registers the cells'
//!   actions (Focus, ScrollIntoView, ShowContextMenu) once the leaves have their ids.
//! - **Announcements** (polite, a `Status` leaf, dropped after [`A11Y_LIVE_HOLD`]): a focus that
//!   changes row says "Row 3 of 1,000, email: alice@example.com"; one that stays in its row says
//!   "email: alice@example.com"; a selection of more than one cell says its size instead
//!   (`hui-grid-selection-summary`). The find's count and the go-to dialog keep their own nodes
//!   (Task 37): a find step or a go-to moves the focus, and the grid says only the cell.

use std::cell::{Cell, RefCell};
use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use hacivat_ui::l10n::{UiArg, UiText, UiTextProvider};
use hacivat_ui_core::gpui_surface::{
    A11ySubtreeBuilder, AnyElement, App, Context, Div, ElementId, FluentBuilder as _,
    InteractiveElement as _, IntoElement as _, Role, SharedString, Stateful,
    StatefulInteractiveElement as _, Styled as _, Window, accesskit, canvas, div, px,
};

use super::Geometry;
use crate::model::{Selection, ViewMap};
use crate::source::{
    CellContent, CellKind, ColId, GridCellFlags, GridFrame, ModelCell, ModelRow, ViewRow,
};
use crate::view::{GridCommand, GridView};

/// Rows built above and below the ones on screen (HV-GRID-16: overscan of 5).
pub const A11Y_OVERSCAN_ROWS: u64 = 5;

/// The most characters a cell's name keeps (then `…`).
pub const A11Y_NAME_MAX_CHARS: usize = 1_000;

/// How long an announcement stays in the tree: long enough for every platform to read it,
/// short enough that the grid shown again (a tab switch) does not read it twice.
pub const A11Y_LIVE_HOLD: Duration = Duration::from_secs(3);

/// The actions a data cell answers.
const CELL_ACTIONS: [accesskit::Action; 3] = [
    accesskit::Action::Focus,
    accesskit::Action::ScrollIntoView,
    accesskit::Action::ShowContextMenu,
];

/// Which row a node is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RowKey {
    /// The column headers (row 0).
    Header,
    /// A loaded row, by its model row: the same key wherever a sort or a scroll puts it.
    Model(ModelRow),
    /// A view row past the loaded rows.
    Loading(u64),
}

/// One cell node.
#[derive(Debug, Clone, PartialEq)]
pub struct CellNode {
    pub col: ColId,
    /// The cell's place among the visible columns (its column index).
    pub index: usize,
    pub name: SharedString,
    pub selected: bool,
    /// The focus cell: the grid's active descendant.
    pub focused: bool,
    /// A header cell's sort direction.
    pub sort: Option<accesskit::SortDirection>,
    /// A header cell's sort tooltip.
    pub description: Option<SharedString>,
}

impl CellNode {
    /// The AccessKit node of this cell in row `row_index` (`ColumnHeader` in the header row,
    /// `Cell` with its actions elsewhere).
    pub fn node(&self, row_index: usize, header: bool) -> accesskit::Node {
        let mut node = accesskit::Node::new(if header {
            Role::ColumnHeader
        } else {
            Role::Cell
        });
        node.set_label(self.name.to_string());
        node.set_row_index(row_index);
        node.set_column_index(self.index);
        if let Some(sort) = self.sort {
            node.set_sort_direction(sort);
        }
        if let Some(d) = &self.description {
            node.set_description(d.to_string());
        }
        if !header {
            node.set_selected(self.selected);
            for action in CELL_ACTIONS {
                node.add_action(action);
            }
        }
        node
    }
}

/// One row node and its cells.
#[derive(Debug, Clone, PartialEq)]
pub struct RowNode {
    pub key: RowKey,
    /// The row index: the header is 0, view row `v` is `v + 1`.
    pub index: usize,
    /// The row's name (a loading row's "Loading").
    pub name: Option<SharedString>,
    /// The whole row is selected (the gutter).
    pub selected: bool,
    pub cells: Vec<CellNode>,
}

impl RowNode {
    pub fn is_header(&self) -> bool {
        self.key == RowKey::Header
    }

    /// The view row this row shows (`None` for the header).
    pub fn view_row(&self) -> Option<u64> {
        (!self.is_header()).then(|| self.index as u64 - 1)
    }

    /// The row element's id under the grid's root.
    pub fn element_id(&self) -> ElementId {
        match self.key {
            RowKey::Header => ElementId::Name("hv-grid-a11y-header".into()),
            RowKey::Model(row) => ("hv-grid-a11y-row", row.0).into(),
            RowKey::Loading(view) => ("hv-grid-a11y-loading", view).into(),
        }
    }

    /// The row's own AccessKit node, as its element declares it.
    pub fn node(&self) -> accesskit::Node {
        let mut node = accesskit::Node::new(Role::Row);
        node.set_row_index(self.index);
        if self.selected {
            node.set_selected(true);
        }
        if let Some(name) = &self.name {
            node.set_label(name.to_string());
        }
        node
    }
}

/// The nodes of `rows`: each row and each of its cells.
pub fn count_nodes(rows: &[RowNode]) -> usize {
    rows.iter().map(|r| 1 + r.cells.len()).sum()
}

/// A visible column of the subtree.
#[derive(Debug, Clone, PartialEq)]
pub struct A11yColumn {
    pub id: ColId,
    /// Its place among the visible columns.
    pub index: usize,
    /// The column's name (catalog text).
    pub name: SharedString,
    pub sort: Option<accesskit::SortDirection>,
    pub sort_tip: Option<SharedString>,
}

/// The rows of the subtree.
#[derive(Debug, Clone)]
pub struct A11yRows<'a> {
    /// The view rows on screen.
    pub visible: Range<u64>,
    /// The rows the grid scrolls over (a filter's kept rows, the total, the loaded rows).
    pub shown: u64,
    /// Rows built above and below `visible`.
    pub overscan: u64,
    pub map: &'a ViewMap,
    pub selection: &'a Selection,
}

/// Words a truncated value: `(value, size) → "…, truncated, 5 MB"`.
pub type TruncatedFn = Rc<dyn Fn(&str, &str) -> SharedString>;

/// The words a cell's name can take, resolved once per frame.
#[derive(Clone)]
pub struct A11yWords {
    /// A NULL cell (`hui-grid-a11y-null`).
    pub null: SharedString,
    /// A row or cell not loaded yet (the grid's "loading" label).
    pub loading: SharedString,
    /// A truncated value with its full size: `(value, size) → "…, truncated, 5 MB"`.
    pub truncated: TruncatedFn,
}

impl std::fmt::Debug for A11yWords {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("A11yWords")
            .field("null", &self.null)
            .field("loading", &self.loading)
            .finish_non_exhaustive()
    }
}

/// The header row and the rows `rows` shows (pure): see the module docs.
pub fn visible_nodes(
    frame: &dyn GridFrame,
    rows: &A11yRows<'_>,
    cols: &[A11yColumn],
    words: &A11yWords,
) -> Vec<RowNode> {
    let start = rows.visible.start.saturating_sub(rows.overscan);
    let end = rows
        .visible
        .end
        .saturating_add(rows.overscan)
        .min(rows.shown)
        .max(start);
    let mut out = Vec::with_capacity(1 + (end - start) as usize);
    out.push(RowNode {
        key: RowKey::Header,
        index: 0,
        name: None,
        selected: false,
        cells: cols
            .iter()
            .map(|c| CellNode {
                col: c.id,
                index: c.index,
                name: c.name.clone(),
                selected: false,
                focused: false,
                sort: c.sort,
                description: c.sort_tip.clone(),
            })
            .collect(),
    });
    let loaded = frame.row_count().loaded;
    let focus = rows.selection.focus();
    for v in start..end {
        let index = usize::try_from(v.saturating_add(1)).unwrap_or(usize::MAX);
        let model = rows.map.view_to_model(ViewRow(v)).filter(|m| m.0 < loaded);
        let Some(row) = model else {
            out.push(RowNode {
                key: RowKey::Loading(v),
                index,
                name: Some(words.loading.clone()),
                selected: false,
                cells: Vec::new(),
            });
            continue;
        };
        let cells = cols
            .iter()
            .map(|c| {
                let cell = ModelCell { row, col: c.id };
                CellNode {
                    col: c.id,
                    index: c.index,
                    name: cell_name(&frame.a11y_cell(row, c.id), words),
                    selected: rows.selection.contains(ViewRow(v), c.id, rows.map),
                    focused: focus == Some(cell),
                    sort: None,
                    description: None,
                }
            })
            .collect();
        out.push(RowNode {
            key: RowKey::Model(row),
            index,
            name: None,
            selected: rows.selection.row_selected(row),
            cells,
        });
    }
    out
}

/// What a screen reader reads for `cell` (module docs).
pub fn cell_name(cell: &CellContent, words: &A11yWords) -> SharedString {
    match cell.kind {
        CellKind::Placeholder => words.loading.clone(),
        CellKind::Null => words.null.clone(),
        _ => {
            let value = capped(&cell.text);
            match &cell.chip {
                Some(size) if cell.flags.contains(GridCellFlags::TRUNCATED) => {
                    (words.truncated)(&value, size)
                }
                _ => value,
            }
        }
    }
}

/// `text` cut after [`A11Y_NAME_MAX_CHARS`] characters, with `…`.
fn capped(text: &SharedString) -> SharedString {
    if text.len() <= A11Y_NAME_MAX_CHARS {
        return text.clone();
    }
    match text.char_indices().nth(A11Y_NAME_MAX_CHARS) {
        None => text.clone(),
        Some((cut, _)) => {
            let mut s = String::with_capacity(cut + '…'.len_utf8());
            s.push_str(&text[..cut]);
            s.push('…');
            s.into()
        }
    }
}

/// What a selection change says (before the words are filled in).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Said {
    /// The focus moved to another row: its row and the cell.
    Row(ModelCell),
    /// The focus moved within its row: the cell.
    Cell(ModelCell),
    /// More than one cell is selected: how many.
    Selection(u128),
}

/// The selection as the last announcement left it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Seen {
    pub focus: Option<ModelCell>,
    /// The focus's view row.
    pub view_row: Option<u64>,
    /// The selected cells.
    pub cells: u128,
}

/// What to say when the selection goes from `before` to `now` (pure): the summary once more
/// than one cell is selected (each time the count changes), else the focus cell when it moved
/// (with its row when the row changed); nothing when the focus went away or nothing changed.
pub fn said(before: &Seen, now: &Seen) -> Option<Said> {
    if before == now {
        return None;
    }
    if now.cells > 1 {
        return (now.cells != before.cells).then_some(Said::Selection(now.cells));
    }
    let focus = now.focus?;
    if now.focus == before.focus && before.cells <= 1 {
        return None;
    }
    if now.view_row != before.view_row {
        Some(Said::Row(focus))
    } else {
        Some(Said::Cell(focus))
    }
}

/// The live region's announcement.
#[derive(Debug, Clone, PartialEq)]
struct Live {
    seq: u64,
    text: SharedString,
    since: Instant,
}

/// The subtree's state on the view (empty while no assistive technology is active).
#[derive(Debug, Default)]
pub(crate) struct A11yState {
    /// The last frame's nodes; the render after it declares them.
    nodes: Rc<Vec<RowNode>>,
    /// Frames that built the subtree.
    builds: u64,
    /// The selection as last announced; `None` until the first active frame.
    seen: Option<Seen>,
    live: Option<Live>,
    last_live: u64,
    /// Tests stand in for an attached screen reader (`test-support`).
    forced: bool,
}

impl A11yState {
    fn clear(&mut self) {
        if !self.nodes.is_empty() {
            self.nodes = Rc::default();
        }
        self.seen = None;
        self.live = None;
    }
}

/// Whether the grid builds its subtree this frame.
fn active(v: &GridView, window: &Window) -> bool {
    v.a11y.forced || window.is_a11y_active()
}

/// The message `id` from the installed UI text, or the id itself.
fn message(cx: &App, id: &'static str, args: &[(&'static str, UiArg)]) -> SharedString {
    match cx.try_global::<UiText>() {
        Some(provider) => provider.0.text(id, args),
        None => SharedString::from(id),
    }
}

impl A11yWords {
    /// The words of the installed UI text, and the grid's "loading" label.
    fn of(v: &GridView, cx: &App) -> A11yWords {
        let provider: Option<Arc<dyn UiTextProvider>> =
            cx.try_global::<UiText>().map(|t| t.0.clone());
        A11yWords {
            null: message(cx, "hui-grid-a11y-null", &[]),
            loading: (v.config.labels.loading)(cx),
            truncated: Rc::new(move |value: &str, size: &str| {
                let id = "hui-grid-a11y-truncated";
                match &provider {
                    Some(p) => p.text(
                        id,
                        &[
                            ("value", UiArg::Text(SharedString::from(value.to_string()))),
                            ("size", UiArg::Text(SharedString::from(size.to_string()))),
                        ],
                    ),
                    None => SharedString::from(id),
                }
            }),
        }
    }
}

/// The frame's visible columns: the frozen ones, then the scrolling ones on screen.
fn columns(v: &GridView, geom: &Geometry, cx: &App) -> Vec<A11yColumn> {
    let visible = v.layout.visible();
    let indices = (0..geom.frozen).chain(geom.cols.clone());
    indices
        .filter_map(|index| {
            let id = *visible.get(index)?;
            let name = v
                .columns
                .iter()
                .find(|c| c.id == id)
                .map(|c| c.name.clone())
                .unwrap_or_default();
            let (sort, sort_tip) = match v.sort.spec.level_of(id) {
                Some((level, key)) => (
                    Some(if key.desc {
                        accesskit::SortDirection::Descending
                    } else {
                        accesskit::SortDirection::Ascending
                    }),
                    v.sort.tips.get(level).map(|tip| tip(cx)),
                ),
                None => (None, None),
            };
            Some(A11yColumn {
                id,
                index,
                name,
                sort,
                sort_tip,
            })
        })
        .collect()
}

/// The frame's accessibility children for `v`: the nodes, the announcement, and a render of
/// the view when they changed (module docs). Nothing while no assistive technology is active.
pub(crate) fn prepaint(
    v: &mut GridView,
    frame: &dyn GridFrame,
    geom: &Geometry,
    window: &mut Window,
    cx: &mut Context<GridView>,
) {
    if !active(v, window) {
        v.a11y.clear();
        return;
    }
    v.a11y.builds += 1;
    let words = A11yWords::of(v, cx);
    let cols = columns(v, geom, cx);
    let map = v.view_map().clone();
    let nodes = visible_nodes(
        frame,
        &A11yRows {
            visible: geom.rows.clone(),
            shown: v.data.rows,
            overscan: A11Y_OVERSCAN_ROWS,
            map: &map,
            selection: &v.selection,
        },
        &cols,
        &words,
    );
    let mut changed = false;
    if *v.a11y.nodes != nodes {
        v.a11y.nodes = Rc::new(nodes);
        changed = true;
    }
    let now = cx.background_executor().now();
    if v.a11y
        .live
        .as_ref()
        .is_some_and(|l| now >= l.since + A11Y_LIVE_HOLD)
    {
        v.a11y.live = None;
        changed = true;
    }
    // A selection change is pending its event this frame (`flush_frame_events` runs after
    // this hook): count it once, and say what changed.
    if v.selection_dirty || v.a11y.seen.is_none() {
        let focus = v.selection.focus();
        let seen = Seen {
            focus,
            view_row: focus.and_then(|f| map.model_to_view(f.row)).map(|r| r.0),
            cells: v.selection.cell_count_in(&map, v.layout.visible()),
        };
        let before = v.a11y.seen.replace(seen);
        let text = before
            .and_then(|before| said(&before, &seen))
            .map(|s| announcement(v, frame, &map, s, &words, cx));
        if let Some(text) = text {
            v.a11y.last_live += 1;
            v.a11y.live = Some(Live {
                seq: v.a11y.last_live,
                text,
                since: now,
            });
            changed = true;
        }
    }
    if changed {
        let view = cx.entity();
        window.on_next_frame(move |_, cx| view.update(cx, |_, cx| cx.notify()));
    }
}

/// The words of `s`.
fn announcement(
    v: &GridView,
    frame: &dyn GridFrame,
    map: &ViewMap,
    s: Said,
    words: &A11yWords,
    cx: &App,
) -> SharedString {
    let (focus, with_row) = match s {
        Said::Selection(cells) => {
            let cells = u64::try_from(cells).unwrap_or(u64::MAX);
            return message(
                cx,
                "hui-grid-selection-summary",
                &[("cells", UiArg::Count(cells))],
            );
        }
        Said::Row(f) => (f, true),
        Said::Cell(f) => (f, false),
    };
    let column = v
        .columns
        .iter()
        .find(|c| c.id == focus.col)
        .map(|c| c.name.clone())
        .unwrap_or_default();
    let value = cell_name(&frame.a11y_cell(focus.row, focus.col), words);
    let cell = message(
        cx,
        "hui-grid-a11y-cell",
        &[
            ("column", UiArg::Text(column)),
            ("value", UiArg::Text(value)),
        ],
    );
    if !with_row {
        return cell;
    }
    let row = map.model_to_view(focus.row).map_or(focus.row.0, |r| r.0) + 1;
    let (total, more) = row_total(v);
    let args = [
        ("row", UiArg::Count(row)),
        ("total", UiArg::Count(total)),
        ("cell", UiArg::Text(cell)),
    ];
    if more {
        message(cx, "hui-grid-a11y-row-of-more", &args)
    } else {
        message(cx, "hui-grid-a11y-row-of", &args)
    }
}

/// The grid's description while more rows may come (`hui-grid-a11y-more-rows`).
pub(crate) fn more_rows(cx: &App) -> SharedString {
    message(cx, "hui-grid-a11y-more-rows", &[])
}

/// The data rows the grid announces, and whether more may come: a filter's kept rows; else the
/// exact total when known, the loaded rows otherwise.
pub(crate) fn row_total(v: &GridView) -> (u64, bool) {
    match v.view_map() {
        map @ ViewMap::Filtered { .. } => (map.len(), false),
        _ => match v.data.known_total() {
            Some(total) => (total, false),
            None => (v.data.count.loaded, true),
        },
    }
}

/// Where the cells of the last frame sit, in window px: each visible column's left and right
/// edge, by its index.
struct Columns {
    edges: Vec<(usize, f32, f32)>,
}

impl Columns {
    fn of(v: &GridView, g: &Geometry) -> Columns {
        let left = f32::from(g.body.left());
        let edges = (0..g.frozen)
            .chain(g.cols.clone())
            .map(|ix| {
                let x = left + v.layout.screen_x(ix, v.scroll.left_x) as f32;
                (ix, x, x + v.layout.width_at(ix))
            })
            .collect();
        Columns { edges }
    }

    fn edges(&self, index: usize) -> Option<(f32, f32)> {
        self.edges
            .iter()
            .find(|(ix, ..)| *ix == index)
            .map(|(_, a, b)| (*a, *b))
    }
}

/// The root (the grid's node) and its row children for this render: see the module docs.
/// Without assistive technology (or before the first frame), `root` unchanged and no
/// children.
pub(crate) fn subtree(
    v: &mut GridView,
    root: Stateful<Div>,
    window: &Window,
    cx: &mut Context<GridView>,
) -> (Stateful<Div>, Vec<AnyElement>) {
    if !active(v, window) {
        return (root, Vec::new());
    }
    let Some(g) = v.geometry.clone() else {
        return (root, Vec::new());
    };
    let nodes = v.a11y.nodes.clone();
    let scale = g.scale;
    let origin = g.bounds.origin;
    let columns = Rc::new(Columns::of(v, &g));
    let focus_id: Rc<Cell<Option<accesskit::NodeId>>> = Rc::default();
    let targets: Rc<RefCell<Vec<(accesskit::NodeId, ModelCell)>>> = Rc::default();
    let row_left = f32::from(g.gutter.left());
    let row_w = f32::from(g.gutter.size.width) + f32::from(g.body.size.width);
    let mut children = Vec::with_capacity(nodes.len() + 1);
    for (i, row) in nodes.iter().enumerate() {
        let (top, h) = match row.view_row() {
            None => (f32::from(g.header.top()), f32::from(g.header.size.height)),
            Some(view) => (
                f32::from(g.body.top()) + v.scroll.row_top(view) as f32,
                g.row_h,
            ),
        };
        let mut el = div()
            .id(row.element_id())
            .role(Role::Row)
            .absolute()
            .left(px(row_left - f32::from(origin.x)))
            .top(px(top - f32::from(origin.y)))
            .w(px(row_w))
            .h(px(h))
            .aria_row_index(row.index)
            .when(row.selected, |el| el.aria_selected(true))
            .when_some(row.name.clone(), |el, name| el.aria_label(name));
        if !row.cells.is_empty() {
            let nodes = nodes.clone();
            let columns = columns.clone();
            let focus_id = focus_id.clone();
            let targets = targets.clone();
            el = el.a11y_synthetic_children(move |b| {
                write_cells(
                    b,
                    &nodes[i],
                    &columns,
                    (top, top + h),
                    scale,
                    &focus_id,
                    &targets,
                )
            });
        }
        children.push(el.into_any_element());
    }
    // The cells' actions, once every leaf has its id (prepaint is over when this paints).
    let view = cx.entity().downgrade();
    children.push(
        canvas(
            |_, _, _| (),
            move |_, _, window, _| {
                for (id, cell) in targets.borrow().iter().copied() {
                    for action in CELL_ACTIONS {
                        let view = view.clone();
                        window.on_a11y_action(id, action, move |_, window, cx| {
                            let _ =
                                view.update(cx, |v, cx| v.a11y_action(cell, action, window, cx));
                        });
                    }
                }
            },
        )
        .absolute()
        .size_0()
        .into_any_element(),
    );
    let live = v.a11y.live.clone();
    let root = root.a11y_synthetic_children(move |b| {
        // The rows declared their cells before the root closes: the focus cell is known.
        if let Some(id) = focus_id.get() {
            b.parent_node().set_active_descendant(id);
        }
        if let Some(l) = live {
            let mut node = accesskit::Node::new(Role::Status);
            node.set_live(accesskit::Live::Polite);
            // The Windows and Linux adapters announce a live region by its name, macOS by its
            // value (as the editor's, Task 29).
            node.set_label(l.text.to_string());
            node.set_value(l.text.to_string());
            let id = b.synthetic_node_id(("hv-grid-a11y-live", l.seq));
            b.push_child(id, node);
        }
    });
    (root, children)
}

/// Writes `row`'s cells as synthetic leaves of its element, with their bounds (physical px,
/// as GPUI's own nodes), noting the focus cell's id and every data cell's for its actions.
fn write_cells(
    b: &mut A11ySubtreeBuilder<'_>,
    row: &RowNode,
    columns: &Columns,
    (top, bottom): (f32, f32),
    scale: f32,
    focus_id: &Cell<Option<accesskit::NodeId>>,
    targets: &RefCell<Vec<(accesskit::NodeId, ModelCell)>>,
) {
    let header = row.is_header();
    for cell in &row.cells {
        let mut node = cell.node(row.index, header);
        if let Some((x0, x1)) = columns.edges(cell.index) {
            node.set_bounds(accesskit::Rect {
                x0: f64::from(x0 * scale),
                y0: f64::from(top * scale),
                x1: f64::from(x1 * scale),
                y1: f64::from(bottom * scale),
            });
        }
        let id = b.synthetic_node_id(cell.col.0);
        if b.push_child(id, node)
            && let RowKey::Model(model) = row.key
        {
            if cell.focused {
                focus_id.set(Some(id));
            }
            targets.borrow_mut().push((
                id,
                ModelCell {
                    row: model,
                    col: cell.col,
                },
            ));
        }
    }
}

impl GridView {
    /// Frames that built the accessibility subtree (only while assistive technology is
    /// active).
    pub fn a11y_builds(&self) -> u64 {
        self.a11y.builds
    }

    /// The rows and cells the last active frame built (empty without assistive technology).
    pub fn a11y_nodes(&self) -> Rc<Vec<RowNode>> {
        self.a11y.nodes.clone()
    }

    /// The live region's announcement, while it is held.
    pub fn a11y_announcement(&self) -> Option<SharedString> {
        self.a11y.live.as_ref().map(|l| l.text.clone())
    }

    /// Runs an assistive technology's `action` on `cell`: Focus focuses the grid and the cell,
    /// ScrollIntoView scrolls it into view, ShowContextMenu focuses it and opens the menu there
    /// (`GridEvent::ContextMenuRequested`). A cell that is no longer loaded or shown does
    /// nothing.
    pub fn a11y_action(
        &mut self,
        cell: ModelCell,
        action: accesskit::Action,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if cell.row.0 >= self.data.count.loaded
            || !self.view_map().shows(cell.row)
            || self.layout.index_of(cell.col).is_none()
        {
            return;
        }
        match action {
            accesskit::Action::Focus => {
                window.focus(&self.focus, cx);
                self.focus_cell(cell, cx);
            }
            accesskit::Action::ScrollIntoView => {
                self.reveal(cell);
                cx.notify();
            }
            accesskit::Action::ShowContextMenu => {
                self.focus_cell(cell, cx);
                self.dispatch(GridCommand::ContextMenu, window, cx);
            }
            _ => {}
        }
    }

    /// Builds the subtree as if a screen reader were attached (test windows never attach one).
    #[cfg(feature = "test-support")]
    pub fn assume_a11y_active(&mut self, on: bool, cx: &mut Context<Self>) {
        self.a11y.forced = on;
        cx.notify();
    }
}
