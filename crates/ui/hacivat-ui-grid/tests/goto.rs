//! M3c Task 37 (HV-GRID-09, -14; NOTES surfaces 19 and 20): the grid's side of go to row and
//! of the grid find. `parse_goto` reads the format locale's digits and grouping and a column by
//! position, name, ASCII case-insensitive name, then the tolerant key; `place_row` says where a
//! row lands (shown, past the fetched rows, past a known total naming the last row). The view:
//! `GridView::go_to` focuses the cell (view rows, so a filter's rows count 1…n) and refuses a
//! hidden column; `focus_cell` is the public "focus this cell"; `select_cells` selects each
//! match as a range in one change; the find marks (layer 5) outline the matching text, the
//! current one filled.
//!
//! The parse tests need no features; the view tests need `--features test-support` (the fixture
//! sources).

use hacivat_ui_grid::model::{GotoError, GotoTarget, parse_goto};
use hacivat_ui_grid::{Align, ColId, ColumnKind, ColumnSpec};

fn spec(id: u32, name: &str) -> ColumnSpec {
    ColumnSpec {
        id: ColId(id),
        name: name.to_string().into(),
        type_label: "text".into(),
        kind: ColumnKind::Text,
        align: Align::Start,
        default_width: 120.0,
        key: false,
    }
}

/// `id`, `city`, `Name`, `İlçe`, `meta`.
fn cols() -> Vec<ColumnSpec> {
    vec![
        spec(0, "id"),
        spec(1, "city"),
        spec(2, "Name"),
        spec(3, "İlçe"),
        spec(4, "meta"),
    ]
}

#[test]
fn goto_row_reads_locale_grouping() {
    assert_eq!(
        parse_goto("1.000", &cols(), ',', '.'),
        Ok(GotoTarget::Row(1000))
    );
    // In `en` the comma groups, so `1,000` is a thousand too, never row 1 column 0.
    assert_eq!(
        parse_goto("1,000", &cols(), '.', ','),
        Ok(GotoTarget::Row(1000))
    );
    // A space-like group accepts any space (a no-break or narrow no-break space).
    assert_eq!(
        parse_goto("1\u{202F}500", &cols(), ',', '\u{A0}'),
        Ok(GotoTarget::Row(1500))
    );
    // Another script's digits.
    assert_eq!(
        parse_goto("\u{661}\u{662}", &cols(), '.', ','),
        Ok(GotoTarget::Row(12))
    );
    // The decimal separator is not a row; nor is a row 0, a trailing group or a word.
    assert_eq!(
        parse_goto("1.5", &cols(), '.', ','),
        Err(GotoError::NotANumber)
    );
    assert_eq!(
        parse_goto("0", &cols(), '.', ','),
        Err(GotoError::NotANumber)
    );
    assert_eq!(
        parse_goto("12.", &cols(), ',', '.'),
        Err(GotoError::NotANumber)
    );
    assert_eq!(
        parse_goto("abc", &cols(), '.', ','),
        Err(GotoError::NotANumber)
    );
    assert_eq!(parse_goto("  ", &cols(), '.', ','), Err(GotoError::Empty));
}

#[test]
fn goto_row_col_by_name_and_index() {
    let c = cols();
    let en = |t: &str| parse_goto(t, &c, '.', ',');
    // By 1-based position.
    assert_eq!(en("1500:3"), Ok(GotoTarget::RowCol(1500, ColId(2))));
    // Exact, then ASCII case-insensitive, then the tolerant key.
    assert_eq!(en("1500:city"), Ok(GotoTarget::RowCol(1500, ColId(1))));
    assert_eq!(en("7:NAME"), Ok(GotoTarget::RowCol(7, ColId(2))));
    assert_eq!(en("7:ilce"), Ok(GotoTarget::RowCol(7, ColId(3))));
    assert_eq!(en("7: ILÇE "), Ok(GotoTarget::RowCol(7, ColId(3))));
    // In `tr` the comma names the column (it groups nothing there).
    assert_eq!(
        parse_goto("1.000,city", &c, ',', '.'),
        Ok(GotoTarget::RowCol(1000, ColId(1)))
    );
    // An empty column part names no column; an unknown one is an error.
    assert_eq!(en("12:"), Ok(GotoTarget::Row(12)));
    assert_eq!(en("12:ilce_x"), Err(GotoError::UnknownColumn));
    assert_eq!(en("12:6"), Err(GotoError::UnknownColumn));
    assert_eq!(en("12:0"), Err(GotoError::UnknownColumn));
}

#[test]
fn goto_out_of_range_names_the_last_row() {
    use hacivat_ui_grid::ViewRow;
    use hacivat_ui_grid::model::{GotoPlace, place_row};
    assert_eq!(
        place_row(40, 12, Some(12)),
        Err(GotoError::OutOfRange { last: Some(12) })
    );
    assert_eq!(
        place_row(12, 12, Some(12)),
        Ok(GotoPlace::Shown(ViewRow(11)))
    );
    // Past the fetched rows of an open cursor: fetch, then go.
    assert_eq!(
        place_row(1_500_000, 10_000, None),
        Ok(GotoPlace::NotFetched)
    );
}

#[cfg(feature = "test-support")]
mod view {
    use std::cell::RefCell;
    use std::rc::Rc;

    use gpui::{Entity, TestAppContext, VisualTestContext, size};
    use hacivat_ui_core::theme::{Theme, ThemeMode};
    use hacivat_ui_grid::model::{GotoError, GotoPlace, GotoTarget, ViewMap};
    use hacivat_ui_grid::testing::{FixtureSource, cell, grid_config};
    use hacivat_ui_grid::{
        Align, CellKind, CellMarks, ColId, GridDataSource, GridEvent, GridView, ModelCell,
        ModelRow, ViewRow,
    };

    type Events = Rc<RefCell<Vec<GridEvent>>>;

    fn open(
        cx: &mut TestAppContext,
        source: Rc<dyn GridDataSource>,
    ) -> (Entity<GridView>, &mut VisualTestContext, Events) {
        cx.update(|cx| Theme::install(cx, ThemeMode::Dark));
        let (view, vcx) =
            cx.add_window_view(|_, cx| GridView::new(source, grid_config("Result 1"), cx));
        vcx.simulate_resize(size(gpui::px(800.0), gpui::px(400.0)));
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

    fn frame(vcx: &mut VisualTestContext) {
        vcx.update(|window, _| window.refresh());
        vcx.run_until_parked();
    }

    /// `rows` rows of `r<row>c<col>` over three columns, all fetched and complete.
    fn source(rows: u64) -> FixtureSource {
        FixtureSource::generated(3, rows, Some(rows), false, |r, c| {
            cell(format!("r{r}c{}", c.0), CellKind::Text, Align::Start)
        })
    }

    fn go(
        view: &Entity<GridView>,
        vcx: &mut VisualTestContext,
        t: GotoTarget,
    ) -> Result<GotoPlace, GotoError> {
        vcx.update(|_, cx| view.update(cx, |v, cx| v.go_to(t, cx)))
    }

    #[gpui::test]
    fn go_to_focuses_view_rows_and_names_the_last_row(cx: &mut TestAppContext) {
        let (view, vcx, events) = open(cx, Rc::new(source(12)));
        frame(vcx);
        assert_eq!(
            go(&view, vcx, GotoTarget::Row(40)),
            Err(GotoError::OutOfRange { last: Some(12) })
        );
        assert_eq!(
            go(&view, vcx, GotoTarget::RowCol(5, ColId(2))),
            Ok(GotoPlace::Shown(ViewRow(4)))
        );
        let focus = view.read_with(vcx, |v, _| v.selection().focus());
        assert_eq!(
            focus,
            Some(ModelCell {
                row: ModelRow(4),
                col: ColId(2)
            })
        );
        assert!(
            events
                .borrow()
                .iter()
                .any(|e| matches!(e, GridEvent::FocusChanged(Some(c)) if c.row == ModelRow(4)))
        );
        // Under a filter the rows are the kept ones, 1…n: row 2 is model row 9.
        vcx.update(|_, cx| {
            view.update(cx, |v, cx| {
                v.set_view_map(
                    ViewMap::filtered(std::sync::Arc::from([3u64, 9, 11]), 12),
                    cx,
                )
            })
        });
        frame(vcx);
        assert_eq!(
            go(&view, vcx, GotoTarget::Row(2)),
            Ok(GotoPlace::Shown(ViewRow(1)))
        );
        assert_eq!(
            view.read_with(vcx, |v, _| v.selection().focus().map(|c| c.row)),
            Some(ModelRow(9))
        );
        assert_eq!(
            go(&view, vcx, GotoTarget::Row(4)),
            Err(GotoError::OutOfRange { last: Some(3) })
        );
    }

    #[gpui::test]
    fn go_to_past_the_fetched_rows_asks_the_host(cx: &mut TestAppContext) {
        let source = FixtureSource::generated(2, 100, None, true, |r, c| {
            cell(format!("r{r}c{}", c.0), CellKind::Text, Align::Start)
        });
        let (view, vcx, _) = open(cx, Rc::new(source));
        frame(vcx);
        assert_eq!(
            go(&view, vcx, GotoTarget::Row(150)),
            Ok(GotoPlace::NotFetched)
        );
        // Nothing moved.
        assert_eq!(view.read_with(vcx, |v, _| v.selection().focus()), None);
    }

    #[gpui::test]
    fn go_to_a_hidden_column_is_refused_until_it_is_shown(cx: &mut TestAppContext) {
        let (view, vcx, _) = open(cx, Rc::new(source(12)));
        frame(vcx);
        vcx.update(|_, cx| view.update(cx, |v, cx| v.set_column_hidden(ColId(1), true, cx)));
        frame(vcx);
        assert_eq!(
            go(&view, vcx, GotoTarget::RowCol(3, ColId(1))),
            Err(GotoError::HiddenColumn(ColId(1)))
        );
        vcx.update(|_, cx| view.update(cx, |v, cx| v.set_column_hidden(ColId(1), false, cx)));
        frame(vcx);
        assert_eq!(
            go(&view, vcx, GotoTarget::RowCol(3, ColId(1))),
            Ok(GotoPlace::Shown(ViewRow(2)))
        );
    }

    #[gpui::test]
    fn select_cells_selects_each_match_as_one_range(cx: &mut TestAppContext) {
        let (view, vcx, events) = open(cx, Rc::new(source(20)));
        frame(vcx);
        events.borrow_mut().clear();
        let cells: Vec<ModelCell> = [(1, 0), (4, 2), (9, 1)]
            .into_iter()
            .map(|(r, c)| ModelCell {
                row: ModelRow(r),
                col: ColId(c),
            })
            .collect();
        vcx.update(|_, cx| view.update(cx, |v, cx| v.select_cells(&cells, 1, cx)));
        frame(vcx);
        view.read_with(vcx, |v, _| {
            let s = v.selection();
            assert_eq!(s.ranges(), 3);
            assert_eq!(s.focus(), Some(cells[1]));
            let map = v.view_map().clone();
            for c in &cells {
                assert!(s.contains(ViewRow(c.row.0), c.col, &map));
            }
            assert!(!s.contains(ViewRow(2), ColId(0), &map));
        });
        // One selection change.
        let changes = events
            .borrow()
            .iter()
            .filter(|e| matches!(e, GridEvent::SelectionChanged))
            .count();
        assert_eq!(changes, 1);
    }

    #[gpui::test]
    fn find_marks_outline_the_matching_text(cx: &mut TestAppContext) {
        let source = FixtureSource::texts(&[&["alpha", "beta"], &["gamma", "alphabet"]], 160.0);
        let (view, vcx, _) = open(cx, Rc::new(source));
        frame(vcx);
        // "alpha" in (0, 0) and (1, 1); (1, 1) is the current match.
        let marks: hacivat_ui_grid::Highlights =
            Rc::new(|row: ModelRow, col: ColId| match (row.0, col.0) {
                (0, 0) => Some(CellMarks {
                    ranges: smallvec::smallvec![0..5],
                    current: None,
                }),
                (1, 1) => Some(CellMarks {
                    ranges: smallvec::smallvec![0..5],
                    current: Some(0),
                }),
                _ => None,
            });
        vcx.update(|_, cx| view.update(cx, |v, cx| v.set_highlights(Some(marks), cx)));
        frame(vcx);
        let painted = view.read_with(vcx, |v, _| v.painted_find_marks().to_vec());
        assert_eq!(painted.len(), 2, "{painted:?}");
        let other = painted.iter().find(|m| !m.current).expect("an other match");
        let current = painted
            .iter()
            .find(|m| m.current)
            .expect("the current match");
        assert_eq!((other.row, other.col), (0, ColId(0)));
        assert_eq!((current.row, current.col), (1, ColId(1)));
        // The outline covers the text "alpha" only, not the whole cell: "alphabet" is wider.
        let w = current.x1 - current.x0;
        assert!(w > 10.0 && w < 160.0 - 16.0, "{current:?}");
        assert!(
            (other.x1 - other.x0 - w).abs() < 0.5,
            "{other:?} {current:?}"
        );
        // Cleared: nothing painted.
        vcx.update(|_, cx| view.update(cx, |v, cx| v.set_highlights(None, cx)));
        frame(vcx);
        assert!(view.read_with(vcx, |v, _| v.painted_find_marks().is_empty()));
    }
}
