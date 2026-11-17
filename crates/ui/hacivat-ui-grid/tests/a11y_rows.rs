//! M3c Task 40: the grid's AccessKit subtree (HV-GRID-16; R-M3-69): the pure nodes (rows on
//! screen ± overscan, the header row, stable ids, names), the nodes' AccessKit properties, the
//! zero cost without assistive technology, the announcements and actions, and Tab leaving the
//! grid from any cell.
//!
//! Test windows never attach assistive technology (`Window::is_a11y_active` stays false), so
//! the view's frames are checked twice: as they are (nothing is built) and with
//! `GridView::assume_a11y_active`, which builds the subtree as an attached screen reader
//! would. The GPUI element tree itself (the row elements, the synthetic leaves, the active
//! descendant on the grid's node) is only written while a screen reader is attached: the
//! manual VoiceOver check (Task 45) covers it.
//!
//! The pure tests and the perf test build without features (the grid perf line runs them);
//! the view tests need `--features test-support`.

use std::rc::Rc;
use std::sync::Arc;

use gpui::accesskit;
use hacivat_ui::l10n::{EnglishDefaults, UiArg, UiTextProvider};
use hacivat_ui_core::gpui_surface::SharedString;
use hacivat_ui_grid::element::a11y_rows::{
    A11Y_LIVE_GAP, A11Y_LIVE_HOLD, A11Y_NAME_MAX_GRAPHEMES, A11Y_OVERSCAN_ROWS, A11yColumn,
    A11yRows, A11yWords, LiveRegion, RowKey, RowNode, Said, Seen, cell_name, count_nodes, said,
    visible_nodes,
};
use hacivat_ui_grid::model::{KeyCtx, KeyOutcome, Selection, ViewMap, apply_command};
use hacivat_ui_grid::{
    Align, CellContent, CellKind, ColId, GridCellFlags, GridCommand, GridFrame, ModelCell,
    ModelRow, RowCount,
};

// ---- fixtures -----------------------------------------------------------------------------

/// A plain cell (`testing::cell` without the `test-support` feature).
fn cell(text: impl Into<SharedString>, kind: CellKind, align: Align) -> CellContent {
    CellContent {
        text: text.into(),
        kind,
        flags: GridCellFlags::empty(),
        align,
        chip: None,
    }
}

/// A frame of `rows` loaded rows (all of them, exactly) whose cells come from `f`.
struct Frame {
    rows: u64,
    f: Box<dyn Fn(u64, ColId) -> CellContent>,
}

impl GridFrame for Frame {
    fn generation(&self) -> (u64, u64) {
        (1, 1)
    }

    fn row_count(&self) -> RowCount {
        RowCount {
            loaded: self.rows,
            more: false,
            exact_total: Some(self.rows),
        }
    }

    fn cell(&self, row: ModelRow, col: ColId) -> CellContent {
        (self.f)(row.0, col)
    }
}

fn plain(rows: u64) -> Frame {
    Frame {
        rows,
        f: Box::new(|r, c| cell(format!("r{r}c{}", c.0), CellKind::Text, Align::Start)),
    }
}

/// 40 rows of 20 columns.
fn frame_40x20() -> Frame {
    plain(40)
}

fn columns(n: u32) -> Vec<A11yColumn> {
    (0..n)
        .map(|c| A11yColumn {
            id: ColId(c),
            index: c as usize,
            name: format!("c{c}").into(),
            sort: None,
            sort_tip: None,
        })
        .collect()
}

fn words() -> A11yWords {
    A11yWords {
        null: "null".into(),
        loading: "Loading".into(),
        truncated: Rc::new(|value: &str, size: &str| format!("{value}, truncated, {size}").into()),
    }
}

/// The nodes of view rows `visible` with `overscan`, over `map` and `sel`.
fn nodes_in(
    frame: &dyn GridFrame,
    visible: std::ops::Range<u64>,
    overscan: u64,
    map: &ViewMap,
    sel: &Selection,
    cols: &[A11yColumn],
) -> Vec<RowNode> {
    let rows = A11yRows {
        visible,
        shown: map.len().max(frame.row_count().exact_total.unwrap_or(0)),
        overscan,
        map,
        selection: sel,
    };
    visible_nodes(frame, &rows, cols, &words())
}

/// The plan's `visible_nodes(frame, rows, cols)`: the first `rows` view rows and `cols`
/// columns, overscan off, nothing selected.
fn visible(frame: &Frame, rows: u64, cols: u32) -> Vec<RowNode> {
    let map = ViewMap::identity(frame.rows);
    nodes_in(
        frame,
        0..rows,
        0,
        &map,
        &Selection::default(),
        &columns(cols),
    )
}

fn model_row(n: &RowNode) -> Option<u64> {
    match n.key {
        RowKey::Model(r) => Some(r.0),
        _ => None,
    }
}

/// Fluent's bidi isolation marks (U+2068, U+2069) dropped, for comparing announcements.
fn bare(s: &str) -> String {
    s.chars()
        .filter(|c| !matches!(c, '\u{2068}' | '\u{2069}'))
        .collect()
}

// ---- the plan's tests ---------------------------------------------------------------------

#[test]
fn visible_nodes_for_a_40_by_20_viewport_has_861_nodes() {
    // (40 rows + 1 header row) row nodes + 41 × 20 cell nodes; overscan off in this fixture.
    assert_eq!(count_nodes(&visible(&frame_40x20(), 40, 20)), 861);
}

#[test]
fn overscan_adds_five_rows_each_side_within_the_rows() {
    assert_eq!(A11Y_OVERSCAN_ROWS, 5);
    let frame = plain(100);
    let map = ViewMap::identity(100);
    let sel = Selection::default();
    let cols = columns(3);
    let mid = nodes_in(&frame, 10..20, 5, &map, &sel, &cols);
    assert_eq!(mid[0].key, RowKey::Header);
    assert_eq!(mid[0].index, 0);
    let rows: Vec<_> = mid[1..].iter().filter_map(model_row).collect();
    assert_eq!(rows, (5..25).collect::<Vec<_>>());
    let top = nodes_in(&frame, 0..10, 5, &map, &sel, &cols);
    assert_eq!(
        top[1..].iter().filter_map(model_row).collect::<Vec<_>>(),
        (0..15).collect::<Vec<_>>()
    );
    let end = nodes_in(&frame, 92..100, 5, &map, &sel, &cols);
    assert_eq!(
        end[1..].iter().filter_map(model_row).collect::<Vec<_>>(),
        (87..100).collect::<Vec<_>>()
    );
}

#[test]
fn ids_are_stable_across_scroll() {
    let frame = plain(100);
    let sel = Selection::default();
    let cols = columns(3);
    let identity = ViewMap::identity(100);
    let a = nodes_in(&frame, 10..20, 0, &identity, &sel, &cols);
    let b = nodes_in(&frame, 15..25, 0, &identity, &sel, &cols);
    for row in 15..20u64 {
        let find = |ns: &[RowNode]| {
            ns.iter()
                .find(|n| n.key == RowKey::Model(ModelRow(row)))
                .cloned()
                .expect("row built")
        };
        let (x, y) = (find(&a), find(&b));
        assert_eq!(x, y, "row {row} is the same node after the scroll");
        assert_eq!(x.element_id(), y.element_id());
        assert_eq!(x.index as u64, row + 1, "the row index is the view row + 1");
    }
    // A sort moves a row's index, never its id: model row 50 shows at view row 49 in reverse.
    let reversed: Arc<[u64]> = (0..100u64).rev().collect();
    let sorted = ViewMap::permuted(reversed, 100, 100);
    let s = nodes_in(&frame, 45..55, 0, &sorted, &sel, &cols);
    let i = nodes_in(&frame, 45..55, 0, &identity, &sel, &cols);
    let pick = |ns: &[RowNode]| {
        ns.iter()
            .find(|n| n.key == RowKey::Model(ModelRow(50)))
            .cloned()
            .expect("row 50")
    };
    let (in_sort, in_source) = (pick(&s), pick(&i));
    assert_eq!(in_sort.element_id(), in_source.element_id());
    assert_eq!(in_sort.cells, in_source.cells);
    assert_eq!((in_sort.index, in_source.index), (50, 51));
    // The header keeps one id, and the cells are keyed by column within their row.
    assert_eq!(a[0].element_id(), b[0].element_id());
    assert_eq!(
        x_cols(&a[1]),
        vec![ColId(0), ColId(1), ColId(2)],
        "one cell per visible column, in order"
    );
}

fn x_cols(n: &RowNode) -> Vec<ColId> {
    n.cells.iter().map(|c| c.col).collect()
}

#[test]
fn null_and_truncated_names() {
    let long: SharedString = "é".repeat(1_500).into();
    let long_cell = long.clone();
    let frame = Frame {
        rows: 3,
        f: Box::new(move |_, c| match c.0 {
            0 => cell("NULL", CellKind::Null, Align::Start),
            1 => CellContent {
                text: "abc".into(),
                kind: CellKind::Text,
                flags: GridCellFlags::TRUNCATED,
                align: Align::Start,
                chip: Some("5 MB".into()),
            },
            2 => cell(long_cell.clone(), CellKind::Text, Align::Start),
            3 => cell("", CellKind::Placeholder, Align::Start),
            _ => cell("plain", CellKind::Text, Align::Start),
        }),
    };
    // Five rows to show, three loaded: rows 3 and 4 are loading rows without cells.
    let map = ViewMap::identity(3);
    let rows = A11yRows {
        visible: 0..5,
        shown: 5,
        overscan: 0,
        map: &map,
        selection: &Selection::default(),
    };
    let nodes = visible_nodes(&frame, &rows, &columns(5), &words());
    let names: Vec<_> = nodes[1].cells.iter().map(|c| c.name.to_string()).collect();
    assert_eq!(names[0], "null");
    assert_eq!(names[1], "abc, truncated, 5 MB");
    assert_eq!(names[2].chars().count(), A11Y_NAME_MAX_GRAPHEMES + 1);
    assert!(names[2].ends_with('…'));
    assert!(long.starts_with(names[2].trim_end_matches('…')));
    assert_eq!(names[3], "Loading");
    assert_eq!(names[4], "plain");
    assert_eq!(nodes.len(), 6);
    for (n, view) in nodes[4..].iter().zip(3u64..) {
        assert_eq!(n.key, RowKey::Loading(view));
        assert_eq!(n.name.as_deref(), Some("Loading"));
        assert!(n.cells.is_empty());
        assert_eq!(n.index as u64, view + 1);
    }
    // A truncated flag without a chip reads the value alone; exactly 1,000 characters stay.
    let flagged = CellContent {
        chip: None,
        ..frame.cell(ModelRow(0), ColId(1))
    };
    assert_eq!(cell_name(&flagged, &words()), "abc");
    let whole = cell("x".repeat(1_000), CellKind::Text, Align::Start);
    assert_eq!(cell_name(&whole, &words()).len(), 1_000);
    // ## m3c-t40 fix round 1 (m2): the cap counts graphemes and never splits one: a flag is two
    // regional indicators, a family emoji several code points joined.
    let flags = format!("a{}", "🇹🇷".repeat(1_200));
    let cut = cell_name(&cell(flags.clone(), CellKind::Text, Align::Start), &words());
    assert!(cut.ends_with("🇹🇷…"), "whole flags only");
    assert_eq!(cut.trim_end_matches('…'), format!("a{}", "🇹🇷".repeat(999)));
    let family = "👨‍👩‍👧";
    let families = cell(family.repeat(1_001), CellKind::Text, Align::Start);
    assert_eq!(
        cell_name(&families, &words()).as_ref(),
        format!("{}…", family.repeat(1_000))
    );
    let exact = cell(family.repeat(1_000), CellKind::Text, Align::Start);
    assert_eq!(cell_name(&exact, &words()).as_ref(), family.repeat(1_000));
    // The English words (hui.ftl).
    let en = EnglishDefaults::new();
    assert_eq!(bare(&en.text("hui-grid-a11y-null", &[])), "null");
    let truncated = en.text(
        "hui-grid-a11y-truncated",
        &[
            ("value", UiArg::Text("abc".into())),
            ("size", UiArg::Text("5 MB".into())),
        ],
    );
    assert_eq!(bare(&truncated), "abc, truncated, 5 MB");
}

#[test]
fn tab_leaves_from_any_cell() {
    let map = ViewMap::identity(1_000_000);
    let order: Vec<ColId> = (0..5).map(ColId).collect();
    let k = KeyCtx {
        map: &map,
        order: &order,
        page_rows: 20,
    };
    let tab = GridCommand::NextCell { back: false };
    let back = GridCommand::NextCell { back: true };
    for (row, col) in [(0, 0), (0, 2), (500_000, 3), (999_999, 4), (999_999, 0)] {
        let at = ModelCell {
            row: ModelRow(row),
            col: ColId(col),
        };
        let mut s = Selection::default();
        s.select_cell(at, &map);
        assert_eq!(
            apply_command(tab, &mut s, &k),
            KeyOutcome::Leave { back: false },
            "Tab from ({row}, {col})"
        );
        assert_eq!(
            apply_command(back, &mut s, &k),
            KeyOutcome::Leave { back: true },
            "Shift+Tab from ({row}, {col})"
        );
        assert_eq!(s.focus(), Some(at), "leaving keeps the focus");
    }
    // Nothing focused yet: Tab leaves too.
    let mut s = Selection::default();
    assert_eq!(
        apply_command(tab, &mut s, &k),
        KeyOutcome::Leave { back: false }
    );
    assert!(s.is_empty());
}

#[test]
#[ignore = "perf"]
fn node_build_under_1ms_per_frame() {
    if std::env::var_os("HACIVAT_PERF").is_none_or(|v| v != "1") {
        return;
    }
    // About 1,200 cells (HV-GRID-16's budget): 40 rows on screen + 5 each side, 24 columns in
    // the B1 data's shapes (ids, Turkish words, grouped amounts, timestamps, a NULL every 97th
    // row). The cells are formatted once beforehand: the app's display cache hands a cell on
    // screen out as an `Arc` clone, as this frame does.
    const ROWS: u64 = 1_000_000;
    const COLS: u32 = 24;
    let top = 500_000;
    let words_tr = ["çağrı", "ığdır", "şişli", "öğrenci", "İstanbul", "üzüm"];
    let shaped = |r: u64, c: u32| -> CellContent {
        if (r + u64::from(c)).is_multiple_of(97) {
            return cell("NULL", CellKind::Null, Align::Start);
        }
        let w = |i: u64| words_tr[(i % 6) as usize];
        match c % 4 {
            0 => cell(r.to_string(), CellKind::Number, Align::End),
            1 => cell(
                format!("{} {}", w(r), w(r + 3)),
                CellKind::Text,
                Align::Start,
            ),
            2 => cell(
                format!("{}.{:03},{:02}", r / 1_000, r % 1_000, c),
                CellKind::Number,
                Align::End,
            ),
            _ => cell(
                format!("2026-11-{:02} 12:{:02}:00", 1 + r % 28, c),
                CellKind::Temporal,
                Align::Start,
            ),
        }
    };
    let cached: Rc<Vec<Vec<CellContent>>> = Rc::new(
        (top - 5..top + 45)
            .map(|r| (0..COLS).map(|c| shaped(r, c)).collect())
            .collect(),
    );
    let frame = Frame {
        rows: ROWS,
        f: Box::new(move |r, c| cached[(r - (top - 5)) as usize][c.0 as usize].clone()),
    };
    let map = ViewMap::identity(ROWS);
    let mut sel = Selection::default();
    sel.select_cell(
        ModelCell {
            row: ModelRow(top + 3),
            col: ColId(2),
        },
        &map,
    );
    let cols = columns(COLS);
    // What one active frame's hook does with them: build, compare with the frame before (the
    // render is asked again only on a change), and the AccessKit nodes the render's elements
    // write (## m3c-t40 fix round 1, m3). Not timed: the Fluent words and columns of the frame
    // (a few lookups), and GPUI's own half (the row elements' layout, the leaves' pushes, the
    // action listeners), which runs only with a screen reader attached (Task 45).
    let previous = nodes_in(&frame, top..top + 40, 5, &map, &sel, &cols);
    let build = || {
        let nodes = nodes_in(&frame, top..top + 40, 5, &map, &sel, &cols);
        assert!(nodes == previous, "an unchanged frame compares equal");
        // And the AccessKit nodes GPUI takes from them.
        let mut n = 0usize;
        for row in nodes.iter() {
            n += row.node().row_index().unwrap_or(0);
            for c in &row.cells {
                n += c
                    .node(row.index, row.is_header())
                    .column_index()
                    .unwrap_or(0);
            }
        }
        (count_nodes(&nodes), n)
    };
    let (count, _) = build();
    assert_eq!(count, 51 + 51 * 24);
    let iterations = 200;
    let started = std::time::Instant::now();
    let mut sum = 0usize;
    for _ in 0..iterations {
        sum = sum.wrapping_add(build().1);
    }
    let per = started.elapsed().as_secs_f64() * 1_000.0 / f64::from(iterations);
    println!(
        "grid a11y node build + compare + AccessKit nodes: {per:.3} ms per frame for {count} nodes (sum {sum})"
    );
    assert!(per < 1.0, "{per:.3} ms per frame");
}

// ---- the nodes' AccessKit properties ------------------------------------------------------

#[test]
fn nodes_carry_roles_indices_selection_and_sort() {
    let frame = plain(10);
    let map = ViewMap::identity(10);
    let mut sel = Selection::default();
    sel.select_cell(
        ModelCell {
            row: ModelRow(2),
            col: ColId(1),
        },
        &map,
    );
    let mut cols = columns(3);
    cols[0].sort = Some(accesskit::SortDirection::Descending);
    cols[0].sort_tip = Some("Sort level 2, descending".into());
    cols[2].sort = Some(accesskit::SortDirection::Ascending);
    let nodes = nodes_in(&frame, 0..5, 0, &map, &sel, &cols);
    let header = &nodes[0];
    let h0 = header.cells[0].node(header.index, true);
    assert_eq!(h0.role(), accesskit::Role::ColumnHeader);
    assert_eq!(h0.label(), Some("c0"));
    assert_eq!(
        h0.sort_direction(),
        Some(accesskit::SortDirection::Descending)
    );
    assert_eq!(h0.description(), Some("Sort level 2, descending"));
    assert_eq!((h0.row_index(), h0.column_index()), (Some(0), Some(0)));
    assert!(!h0.supports_action(accesskit::Action::Focus));
    let h1 = header.cells[1].node(header.index, true);
    assert_eq!(h1.sort_direction(), None);
    let row = &nodes[3];
    assert_eq!(row.key, RowKey::Model(ModelRow(2)));
    let r = row.node();
    assert_eq!(r.role(), accesskit::Role::Row);
    assert_eq!(r.row_index(), Some(3));
    let focus = &row.cells[1];
    assert!(focus.focused && focus.selected);
    let n = focus.node(row.index, false);
    assert_eq!(n.role(), accesskit::Role::Cell);
    assert_eq!(n.label(), Some("r2c1"));
    assert_eq!((n.row_index(), n.column_index()), (Some(3), Some(1)));
    assert_eq!(n.is_selected(), Some(true));
    for action in [
        accesskit::Action::Focus,
        accesskit::Action::ScrollIntoView,
        accesskit::Action::ShowContextMenu,
    ] {
        assert!(n.supports_action(action), "{action:?}");
    }
    let other = row.cells[0].node(row.index, false);
    assert_eq!(other.is_selected(), Some(false));
    // Exactly one cell is the focus (the grid's active descendant).
    let focused: usize = nodes
        .iter()
        .map(|n| n.cells.iter().filter(|c| c.focused).count())
        .sum();
    assert_eq!(focused, 1);
    // A whole selected row says so.
    let mut rows = Selection::default();
    rows.select_row(ModelRow(1), ColId(0));
    let nodes = nodes_in(&frame, 0..5, 0, &map, &rows, &cols);
    assert!(nodes[2].selected && nodes[2].node().is_selected() == Some(true));
    assert!(!nodes[1].selected);
}

#[test]
fn what_a_selection_change_says() {
    let at = |r, c| ModelCell {
        row: ModelRow(r),
        col: ColId(c),
    };
    let seen = |f: Option<ModelCell>, view: Option<u64>, cells| Seen {
        focus: f,
        view_row: view,
        cells,
    };
    let none = Seen::default();
    let a = seen(Some(at(0, 0)), Some(0), 1);
    let right = seen(Some(at(0, 1)), Some(0), 1);
    let down = seen(Some(at(1, 1)), Some(1), 1);
    assert_eq!(said(&none, &a), Some(Said::Row(at(0, 0))));
    // ## m3c-t40 fix round 1 (I2): a move within the row says nothing (the focused cell is
    // read); a move to another row says only the row.
    assert_eq!(said(&a, &right), None);
    assert_eq!(said(&right, &down), Some(Said::Row(at(1, 1))));
    assert_eq!(said(&down, &down), None);
    let wide = seen(Some(at(2, 1)), Some(2), 4);
    assert_eq!(said(&down, &wide), Some(Said::Selection(4)));
    // The same size again (a re-sort) says nothing; a new size says it.
    assert_eq!(said(&wide, &seen(Some(at(2, 1)), Some(9), 4)), None);
    assert_eq!(
        said(&wide, &seen(Some(at(3, 1)), Some(3), 6)),
        Some(Said::Selection(6))
    );
    // Back to one cell (Escape) on the focus says nothing; no focus says nothing.
    assert_eq!(said(&wide, &seen(Some(at(2, 1)), Some(2), 1)), None);
    assert_eq!(said(&a, &none), None);
    // ## m3c-t40 fix round 1 (m1): a re-sort that moves the focus cell's row says nothing (the
    // active descendant stays on the same cell).
    assert_eq!(said(&down, &seen(Some(at(1, 1)), Some(57), 1)), None);
}

// ## m3c-t40 fix round 1 (I1): the live region is never read twice.
#[test]
fn an_announcement_is_held_then_dropped_and_never_reread_after_a_gap() {
    let t0 = std::time::Instant::now();
    let ms = std::time::Duration::from_millis;
    let mut live = LiveRegion::default();
    assert_eq!(live.declare(t0), None);
    let seq = live.say("Row 2 of 40".into(), t0);
    assert_eq!(live.count(), 1);
    // Declared by the render after it, painted by that frame: delivered.
    assert_eq!(live.declare(t0).map(|(n, _)| n), Some(seq));
    live.painted(t0);
    // A grid painted on and on keeps it within the hold.
    live.painted(t0 + ms(100));
    assert!(live.declare(t0 + ms(120)).is_some());
    live.painted(t0 + ms(120));
    // The hold ends it.
    assert_eq!(live.declare(t0 + A11Y_LIVE_HOLD), None);
    assert_eq!(live.text(), None);

    // Delivered, then the grid stops being painted (a tab switch): shown again, it is gone.
    let mut live = LiveRegion::default();
    live.say("Row 3 of 40".into(), t0);
    assert!(live.declare(t0).is_some());
    live.painted(t0);
    assert_eq!(live.declare(t0 + A11Y_LIVE_GAP + ms(1)), None);
    assert_eq!(live.text(), None);

    // Not yet declared: a quiet grid (the settle timer fires with no frame since the last
    // move) still declares it.
    let mut live = LiveRegion::default();
    live.painted(t0);
    live.say("Row 4 of 40".into(), t0 + ms(400));
    assert!(live.declare(t0 + ms(401)).is_some());

    // The hold's timer drops only its own announcement.
    let mut live = LiveRegion::default();
    let first = live.say("Row 5 of 40".into(), t0);
    let second = live.say("Row 6 of 40".into(), t0 + ms(10));
    assert!(!live.expire(first));
    assert!(live.expire(second));
    assert_eq!(live.text(), None);
}

// ---- the view with a screen reader --------------------------------------------------------

#[cfg(feature = "test-support")]
mod with_a_view {
    //! The view's frames (`--features test-support`).

    use std::cell::RefCell;
    use std::rc::Rc;
    use std::sync::Arc;

    use gpui::{Entity, TestAppContext, VisualTestContext, accesskit, px, size};
    use hacivat_ui::l10n::{EnglishDefaults, UiText};
    use hacivat_ui_core::gpui_surface::SharedString;
    use hacivat_ui_core::theme::{Theme, ThemeMode};
    use std::time::Duration;

    use hacivat_ui_grid::element::a11y_rows::{
        A11Y_LIVE_HOLD, A11Y_OVERSCAN_ROWS, A11Y_SETTLE, RowKey,
    };
    use hacivat_ui_grid::model::{Dir, SortKey, SortSpec, ViewMap};
    use hacivat_ui_grid::testing::{FixtureSource, cell, grid_config};
    use hacivat_ui_grid::{
        Align, CellKind, ColId, GridCommand, GridDataSource, GridEvent, GridView, ModelCell,
        ModelRow,
    };

    use super::{bare, model_row};

    #[gpui::test]
    fn nothing_is_built_without_at(cx: &mut TestAppContext) {
        let source = FixtureSource::generated(4, 500, Some(500), false, |r, c| {
            cell(format!("r{r}c{}", c.0), CellKind::Text, Align::Start)
        });
        let (view, vcx, _) = open(cx, Rc::new(source), 600.0, 300.0);
        run(&view, vcx, mv(Dir::Down));
        run(&view, vcx, mv(Dir::Right));
        run(&view, vcx, GridCommand::SelectAll);
        view.update(vcx, |v, cx| v.scroll_px(400.0, 0.0, cx));
        vcx.run_until_parked();
        view.read_with(vcx, |v, _| {
            assert_eq!(v.a11y_builds(), 0, "no frame built a node");
            assert!(v.a11y_nodes().is_empty());
            assert_eq!(v.a11y_announcement(), None);
            assert_eq!(v.a11y_label_reads(), 0, "the label was never read");
        });
        // With a screen reader the frames build the rows on screen and their cells.
        view.update(vcx, |v, cx| v.assume_a11y_active(true, cx));
        vcx.run_until_parked();
        view.read_with(vcx, |v, _| {
            assert!(v.a11y_builds() > 0);
            let nodes = v.a11y_nodes();
            let g = v.geometry().expect("painted");
            let on_screen = g.rows.end - g.rows.start;
            let built = nodes.len() as u64 - 1;
            assert!(built >= on_screen && built <= on_screen + 2 * A11Y_OVERSCAN_ROWS);
            assert_eq!(nodes[0].cells.len(), 4);
        });
        // Detached again: nothing is kept, nothing more is built.
        view.update(vcx, |v, cx| v.assume_a11y_active(false, cx));
        vcx.run_until_parked();
        let builds = view.read_with(vcx, |v, _| v.a11y_builds());
        run(&view, vcx, mv(Dir::Down));
        view.read_with(vcx, |v, _| {
            assert!(v.a11y_nodes().is_empty());
            assert_eq!(v.a11y_builds(), builds);
        });
    }

    type Events = Rc<RefCell<Vec<GridEvent>>>;

    fn open(
        cx: &mut TestAppContext,
        source: Rc<dyn GridDataSource>,
        w: f32,
        h: f32,
    ) -> (Entity<GridView>, &mut VisualTestContext, Events) {
        cx.update(|cx| {
            Theme::install(cx, ThemeMode::Dark);
            cx.set_global(UiText(Arc::new(EnglishDefaults::new())));
        });
        let (view, vcx) =
            cx.add_window_view(|_, cx| GridView::new(source, grid_config("Result 1"), cx));
        vcx.simulate_resize(size(px(w), px(h)));
        vcx.run_until_parked();
        let events: Events = Rc::default();
        let sink = events.clone();
        vcx.update(|_, cx| {
            cx.subscribe(&view, move |_, e: &GridEvent, _| {
                sink.borrow_mut().push(e.clone())
            })
            .detach()
        });
        (view, vcx, events)
    }

    fn run(view: &Entity<GridView>, vcx: &mut VisualTestContext, cmd: GridCommand) {
        vcx.update(|window, cx| view.update(cx, |v, cx| v.dispatch(cmd, window, cx)));
        vcx.run_until_parked();
    }

    fn mv(dir: Dir) -> GridCommand {
        GridCommand::Move { dir, extend: false }
    }

    fn said_now(view: &Entity<GridView>, vcx: &mut VisualTestContext) -> Option<String> {
        view.read_with(vcx, |v, _| v.a11y_announcement().map(|s| bare(&s)))
    }

    /// Lets the announcement settle (## m3c-t40 fix round 1, I2).
    fn settle(vcx: &mut VisualTestContext) {
        vcx.executor()
            .advance_clock(A11Y_SETTLE + Duration::from_millis(10));
        vcx.run_until_parked();
    }

    fn said_count(view: &Entity<GridView>, vcx: &mut VisualTestContext) -> u64 {
        view.read_with(vcx, |v, _| v.a11y_announcements())
    }

    fn generated(cols: u32, loaded: u64, total: Option<u64>, more: bool) -> FixtureSource {
        FixtureSource::generated(cols, loaded, total, more, |r, c| {
            cell(format!("r{r}c{}", c.0), CellKind::Text, Align::Start)
        })
    }

    #[gpui::test]
    fn the_focus_is_announced_with_its_row_and_a_selection_by_its_size(cx: &mut TestAppContext) {
        let (view, vcx, _) = open(cx, Rc::new(generated(4, 40, Some(40), false)), 600.0, 300.0);
        view.update(vcx, |v, cx| v.assume_a11y_active(true, cx));
        vcx.run_until_parked();
        settle(vcx);
        assert_eq!(said_now(&view, vcx), None, "attaching says nothing");
        run(&view, vcx, mv(Dir::Down));
        // ## m3c-t40 fix round 1 (I2): nothing before the selection settles.
        assert_eq!(said_now(&view, vcx), None);
        settle(vcx);
        assert_eq!(said_now(&view, vcx).as_deref(), Some("Row 1 of 40"));
        // Within the row the focused cell is read; the live region adds nothing.
        run(&view, vcx, mv(Dir::Right));
        settle(vcx);
        assert_eq!(said_count(&view, vcx), 1);
        run(&view, vcx, mv(Dir::Down));
        settle(vcx);
        assert_eq!(said_now(&view, vcx).as_deref(), Some("Row 2 of 40"));
        run(
            &view,
            vcx,
            GridCommand::Move {
                dir: Dir::Down,
                extend: true,
            },
        );
        settle(vcx);
        assert_eq!(said_now(&view, vcx).as_deref(), Some("2 cells selected"));
        run(&view, vcx, GridCommand::SelectAll);
        settle(vcx);
        assert_eq!(said_now(&view, vcx).as_deref(), Some("160 cells selected"));
        assert_eq!(said_count(&view, vcx), 4);
        // The hold's timer drops it, with no frame painted in between.
        vcx.executor().advance_clock(A11Y_LIVE_HOLD);
        vcx.run_until_parked();
        assert_eq!(said_now(&view, vcx), None);
        // The focus cell is the one focused node of the frame (the grid's active descendant).
        let focused = view.read_with(vcx, |v, _| {
            v.a11y_nodes()
                .iter()
                .flat_map(|r| {
                    let key = r.key;
                    r.cells
                        .iter()
                        .filter(|c| c.focused)
                        .map(move |c| (key, c.col))
                        .collect::<Vec<_>>()
                })
                .collect::<Vec<_>>()
        });
        assert_eq!(focused, vec![(RowKey::Model(ModelRow(2)), ColId(1))]);
    }

    #[gpui::test]
    fn a_find_step_or_go_to_says_the_cell_under_a_filter_and_an_open_total(
        cx: &mut TestAppContext,
    ) {
        // An open result: the total is the loaded rows "or more".
        let (view, vcx, _) = open(cx, Rc::new(generated(3, 40, None, true)), 600.0, 300.0);
        view.update(vcx, |v, cx| v.assume_a11y_active(true, cx));
        vcx.run_until_parked();
        let at = |r| ModelCell {
            row: ModelRow(r),
            col: ColId(2),
        };
        settle(vcx);
        view.update(vcx, |v, cx| v.focus_cell(at(7), cx));
        vcx.run_until_parked();
        settle(vcx);
        assert_eq!(said_now(&view, vcx).as_deref(), Some("Row 8 of 40 or more"));
        // A filter keeping rows 5, 7 and 30: the rows count as shown (1…3 of 3).
        view.update(vcx, |v, cx| {
            v.set_view_map(ViewMap::filtered(Arc::from([5u64, 7, 30]), 40), cx)
        });
        vcx.run_until_parked();
        view.update(vcx, |v, cx| v.focus_cell(at(30), cx));
        vcx.run_until_parked();
        settle(vcx);
        // ## m3c-t40 fix round 1 (m5): the filter ran over the fetched rows of an open result,
        // so more may still match.
        assert_eq!(said_now(&view, vcx).as_deref(), Some("Row 3 of 3 or more"));
        let nodes = view.read_with(vcx, |v, _| v.a11y_nodes());
        let rows: Vec<_> = nodes[1..].iter().filter_map(model_row).collect();
        assert_eq!(rows, vec![5, 7, 30]);
        assert_eq!(nodes[3].index, 3);
    }

    // ## m3c-t40 fix round 1 (I2): a held arrow key says one thing, once the focus settles.
    #[gpui::test]
    fn held_arrows_announce_once_after_the_last_move(cx: &mut TestAppContext) {
        let (view, vcx, _) = open(
            cx,
            Rc::new(generated(4, 500, Some(500), false)),
            600.0,
            300.0,
        );
        view.update(vcx, |v, cx| v.assume_a11y_active(true, cx));
        vcx.run_until_parked();
        settle(vcx);
        for _ in 0..30 {
            run(&view, vcx, mv(Dir::Down));
            vcx.executor().advance_clock(Duration::from_millis(33));
            vcx.run_until_parked();
        }
        assert_eq!(said_count(&view, vcx), 0, "nothing while the key repeats");
        settle(vcx);
        assert_eq!(said_count(&view, vcx), 1);
        assert_eq!(said_now(&view, vcx).as_deref(), Some("Row 30 of 500"));
        // Away and back before it settles says nothing.
        run(&view, vcx, mv(Dir::Down));
        run(&view, vcx, mv(Dir::Up));
        settle(vcx);
        assert_eq!(said_count(&view, vcx), 1);
    }

    // ## m3c-t40 fix round 1 (m5): a filter over an open result says more rows may come.
    #[gpui::test]
    fn a_filter_over_an_open_result_says_more_rows_may_come(cx: &mut TestAppContext) {
        use gpui::{Element as _, InteractiveElement as _, div};
        let (view, vcx, _) = open(cx, Rc::new(generated(3, 40, None, true)), 600.0, 300.0);
        view.update(vcx, |v, cx| {
            v.set_view_map(ViewMap::filtered(Arc::from([5u64, 7, 30]), 40), cx)
        });
        vcx.run_until_parked();
        let node = vcx.update(|_, cx| {
            view.update(cx, |v, cx| {
                let el = v.a11y_node(div().id("grid-root"), true, cx);
                let mut node = accesskit::Node::new(accesskit::Role::Unknown);
                el.write_a11y_info(&mut node);
                node
            })
        });
        assert_eq!(node.row_count(), Some(4), "3 kept rows and the header row");
        assert_eq!(
            node.description().map(bare).as_deref(),
            Some("More rows are available")
        );
    }

    #[gpui::test]
    fn the_header_says_the_sort_and_frozen_columns_keep_their_indices(cx: &mut TestAppContext) {
        let (view, vcx, _) = open(
            cx,
            Rc::new(generated(12, 100, Some(100), false)),
            500.0,
            300.0,
        );
        view.update(vcx, |v, cx| {
            v.assume_a11y_active(true, cx);
            v.set_frozen(2, cx);
            let spec = SortSpec {
                keys: [
                    SortKey {
                        col: ColId(1),
                        desc: true,
                    },
                    SortKey {
                        col: ColId(0),
                        desc: false,
                    },
                ]
                .into_iter()
                .collect(),
            };
            let tip = |s: &'static str| -> hacivat_ui::text::TextFn {
                Rc::new(move |_| SharedString::new_static(s))
            };
            v.set_sort(
                spec,
                vec![
                    tip("Sort level 1, descending"),
                    tip("Sort level 2, ascending"),
                ],
                cx,
            );
            v.scroll_px(0.0, 600.0, cx);
        });
        vcx.run_until_parked();
        let header = view.read_with(vcx, |v, _| v.a11y_nodes()[0].clone());
        let indices: Vec<_> = header.cells.iter().map(|c| c.index).collect();
        assert_eq!(&indices[..2], &[0, 1], "the frozen columns first");
        assert!(
            indices[2] > 2,
            "then the scrolled columns on screen: {indices:?}"
        );
        let c1 = header.cells[1].node(0, true);
        assert_eq!(
            c1.sort_direction(),
            Some(accesskit::SortDirection::Descending)
        );
        assert_eq!(c1.description(), Some("Sort level 1, descending"));
        let c0 = header.cells[0].node(0, true);
        assert_eq!(
            c0.sort_direction(),
            Some(accesskit::SortDirection::Ascending)
        );
        assert_eq!(header.cells[2].node(0, true).sort_direction(), None);
    }

    #[gpui::test]
    fn cell_actions_focus_scroll_and_open_the_menu(cx: &mut TestAppContext) {
        let (view, vcx, events) = open(
            cx,
            Rc::new(generated(4, 1_000, Some(1_000), false)),
            600.0,
            300.0,
        );
        let at = |r, c| ModelCell {
            row: ModelRow(r),
            col: ColId(c),
        };
        let act = |vcx: &mut VisualTestContext, cell, action| {
            vcx.update(|window, cx| {
                view.update(cx, |v, cx| v.a11y_action(cell, action, window, cx))
            });
            vcx.run_until_parked();
        };
        act(vcx, at(5, 2), accesskit::Action::Focus);
        view.read_with(vcx, |v, _| {
            assert_eq!(v.selection().focus(), Some(at(5, 2)))
        });
        let focused = vcx.update(|window, cx| {
            hacivat_ui_core::gpui_surface::Focusable::focus_handle(view.read(cx), cx)
                .is_focused(window)
        });
        assert!(focused, "Focus focuses the grid");
        act(vcx, at(600, 0), accesskit::Action::ScrollIntoView);
        view.read_with(vcx, |v, _| {
            let g = v.geometry().expect("painted");
            assert!(g.rows.contains(&600), "row 600 on screen: {:?}", g.rows);
            assert_eq!(
                v.selection().focus(),
                Some(at(5, 2)),
                "scrolling selects nothing"
            );
        });
        events.borrow_mut().clear();
        act(vcx, at(600, 3), accesskit::Action::ShowContextMenu);
        let menu = events.borrow().iter().find_map(|e| match e {
            GridEvent::ContextMenuRequested { cell, header, .. } => Some((*cell, *header)),
            _ => None,
        });
        assert_eq!(menu, Some((Some(at(600, 3)), None)));
        // A row that is not loaded does nothing.
        events.borrow_mut().clear();
        act(vcx, at(5_000, 0), accesskit::Action::Focus);
        view.read_with(vcx, |v, _| {
            assert_eq!(v.selection().focus(), Some(at(600, 3)))
        });
        assert!(events.borrow().is_empty());
    }

    #[gpui::test]
    fn the_grid_node_counts_the_header_row_and_says_when_more_rows_come(cx: &mut TestAppContext) {
        use gpui::{Element as _, InteractiveElement as _, div};
        let node_of = |view: &Entity<GridView>, vcx: &mut VisualTestContext| {
            vcx.update(|_, cx| {
                view.update(cx, |v, cx| {
                    let el = v.a11y_node(div().id("grid-root"), true, cx);
                    let mut node = accesskit::Node::new(accesskit::Role::Unknown);
                    el.write_a11y_info(&mut node);
                    node
                })
            })
        };
        let (view, vcx, _) = open(cx, Rc::new(generated(3, 40, None, true)), 600.0, 300.0);
        let node = node_of(&view, vcx);
        assert_eq!(
            node.row_count(),
            Some(41),
            "40 loaded rows and the header row"
        );
        assert_eq!(node.column_count(), Some(3));
        assert_eq!(
            node.description().map(bare).as_deref(),
            Some("More rows are available")
        );
        let (view, vcx, _) = open(cx, Rc::new(generated(3, 40, Some(40), false)), 600.0, 300.0);
        let node = node_of(&view, vcx);
        assert_eq!(node.row_count(), Some(41));
        assert_eq!(node.description(), None);
    }
}
