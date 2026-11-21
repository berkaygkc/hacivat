//! M3c Task 39 (HV-GRID-04 menu, L3 raw setting; NOTES surface 23, M3c-33): the grid's context
//! menu over a cell, a column header or a row number, its disabled items and their reasons,
//! the keyboard menu at the focus cell, auto-fit by a double-click on a header edge, and raw
//! display per column (the menu) and for every grid (`grid.raw_display`).
//!
//! The consoles are registered straight into the registry over result-store fixtures
//! (R-M3-15), as in `results_sort.rs`.

#![cfg_attr(not(unix), allow(dead_code, unused_imports, unused_variables))]

mod support;

use std::sync::Arc;

use gpui::{AppContext as _, Entity, Focusable as _, TestAppContext, VisualTestContext};
use hacivat_app::console::{ConsoleId, ConsoleState, Consoles};
use hacivat_app::results::adapter::ResultDisplay;
use hacivat_app::results::context_menu::{self, MenuKind, MenuTarget};
use hacivat_app::results::view::hosts;
use hacivat_app::results::{ActiveResult, ConsoleResults};
use hacivat_app::shell::update_settings;
use hacivat_exec::{ResultTab, SessionLog, TabKey};
use hacivat_proto::DbmsKind;
use hacivat_results::{
    CloseReason, Completeness, FetchControl, FetchPolicy, MemoryBudget, ResultId, ResultSource,
    ResultStore, StoreSource,
};
use hacivat_session::session::{ClientId, RunId, UnitId};
use hacivat_ui::menu::MenuItemSpec;
use hacivat_ui_grid::model::{SortKey, SortSpec};
use hacivat_ui_grid::{ColId, GridView, ModelCell, ModelRow};
use hacivat_values::valfmt::DisplayZone;
use hacivat_values::{Chunk, ColumnMeta, LogicalType, NativeType, ResultSchema, Value};
use support::*;

// ---- fixtures -------------------------------------------------------------------------------

fn int8(name: &str) -> ColumnMeta {
    ColumnMeta {
        name: name.into(),
        logical: LogicalType::Int {
            bits: 64,
            signed: true,
        },
        native: NativeType::Pg {
            oid: 20,
            typmod: -1,
            name: "int8".into(),
        },
        origin: None,
    }
}

/// `id` (int8), `amount` (int8) and `name` (text), on PostgreSQL.
fn schema() -> Arc<ResultSchema> {
    Arc::new(ResultSchema {
        columns: Arc::from([
            int8("id"),
            int8("amount"),
            ColumnMeta {
                name: "name".into(),
                logical: LogicalType::Text {
                    max_chars: None,
                    fixed: false,
                },
                native: NativeType::Pg {
                    oid: 25,
                    typmod: -1,
                    name: "text".into(),
                },
                origin: None,
            },
        ]),
    })
}

fn row(id: i64, amount: i64, name: &str) -> Vec<Value> {
    vec![Value::Int(id), Value::Int(amount), Value::Text(name.into())]
}

struct NoControl;

impl FetchControl for NoControl {
    fn fetch_more(&self, _: ResultId, _: u64) {}
    fn close(&self, _: ResultId, _: CloseReason) {}
    fn fetch_all(&self, _: ResultId) {}
}

/// A complete result holding `rows`.
fn finished(rows: Vec<Vec<Value>>) -> Arc<dyn ResultSource> {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1 << 41);
    let id = ResultId(NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed));
    let schema = schema();
    let budget = MemoryBudget::new(u64::MAX, u64::MAX);
    let (store, mut writer) = ResultStore::new(id, schema.clone(), budget, 1);
    writer.append(Chunk::from_values(&schema, rows));
    writer.set_state(Completeness::Complete);
    StoreSource::new(store, Arc::new(NoControl), FetchPolicy::default())
}

fn five() -> Arc<dyn ResultSource> {
    finished(vec![
        row(3, 1_234_567, "carrot"),
        row(1, 2_500, "apple"),
        row(5, 10, "elderberry"),
        row(2, 77_000, "banana"),
        row(4, 1_000_000, "a much longer name than the column shows"),
    ])
}

fn tab(source: Arc<dyn ResultSource>) -> ResultTab {
    ResultTab {
        key: TabKey {
            console: 1,
            unit_ordinal: 1,
            result_ordinal: 0,
        },
        title: "".into(),
        unit: UnitId {
            run: RunId(1),
            ordinal: 1,
        },
        source,
    }
}

/// A shell whose focused pane shows a console with empty results, shown in en-US (grouped
/// numbers), with the text registry on.
struct Tile {
    vcx: &'static mut VisualTestContext,
    console: ConsoleId,
    results: Entity<ConsoleResults>,
}

fn tile(cx: &mut TestAppContext) -> Tile {
    let vcx = shell_with_connections(cx, "en");
    in_window(vcx, |window, _| window.activate_window());
    let profile = add_profile(vcx, test_profile("orders-dev", None, "10.0.0.5"));
    let ws = workspace(vcx);
    let (console, results) = in_window(vcx, |window, cx| {
        let tab = ws
            .update(cx, |ws, cx| ws.open_console_tab(Some(profile), window, cx))
            .expect("a console tab opens");
        let log = SessionLog::new();
        let results = cx.new(|_| ConsoleResults::new(log.clone()));
        results.update(cx, |r, cx| {
            r.set_display(
                ResultDisplay::new("en-US", DisplayZone::utc(), DbmsKind::Postgres),
                cx,
            )
        });
        let console = Consoles::register(
            cx,
            ConsoleState {
                id: ConsoleId(0),
                tab,
                n: 1,
                profile: Some(profile),
                client: ClientId::DEFAULT,
                session: None,
                editor: None,
                results: Some(results.clone()),
                sql: None,
                log,
                service: None,
            },
        );
        (console, results)
    });
    enable_registry(vcx);
    vcx.run_until_parked();
    Tile {
        vcx,
        console,
        results,
    }
}

fn cell(row: u64, col: u32) -> ModelCell {
    ModelCell {
        row: ModelRow(row),
        col: ColId(col),
    }
}

fn on_cell(row: u64, col: u32) -> MenuTarget {
    MenuTarget::new(MenuKind::Cell, ColId(col), Some(cell(row, col)))
}

fn on_header(col: u32) -> MenuTarget {
    MenuTarget::new(MenuKind::Header, ColId(col), None)
}

fn on_row(row: u64, col: u32) -> MenuTarget {
    MenuTarget::new(MenuKind::RowNumber, ColId(col), Some(cell(row, col)))
}

/// `text` without the Unicode isolation marks Fluent puts around its arguments.
fn plain(text: &str) -> String {
    text.chars()
        .filter(|c| !matches!(c, '\u{2068}' | '\u{2069}'))
        .collect()
}

impl Tile {
    fn show(&self, source: Arc<dyn ResultSource>) -> Entity<GridView> {
        in_window(self.vcx, |_, cx| {
            self.results.update(cx, |r, cx| {
                r.apply(&[tab(source)], cx);
                r.set_active(ActiveResult::Tab(0), cx);
            })
        });
        self.redraw();
        let grid = self
            .vcx
            .cx
            .read(|cx| {
                hosts(cx)
                    .into_iter()
                    .find_map(|h| h.read(cx).grid_of(self.console, 0, cx))
            })
            .expect("the tab's grid was shown");
        in_window(self.vcx, |window, cx| {
            grid.update(cx, |g, _| g.set_paint_recording(true));
            let handle = grid.read(cx).focus_handle(cx);
            window.focus(&handle, cx);
        });
        self.redraw();
        grid
    }

    fn redraw(&self) {
        // ## m4-t33d (T33 flaky3 follow-up): the last frame renders every view (a frame
        // after `run_until_parked` may reuse a cached view, which records no texts).
        redraw_full(self.vcx);
    }

    /// Each row of the menu over `target`: "|" for a separator, else "✓ " when checked, the
    /// label, " · hint" and " (off)" when disabled.
    fn items(&self, grid: &Entity<GridView>, target: MenuTarget) -> Vec<String> {
        self.vcx.cx.read(|cx| {
            context_menu::menu_items(self.console, grid, target, cx)
                .iter()
                .map(|i| describe(i, cx))
                .collect()
        })
    }

    fn choose(&self, grid: &Entity<GridView>, target: MenuTarget, action: &str) {
        in_window(self.vcx, |_, cx| {
            context_menu::choose(self.console, grid, target, action, cx)
        });
        self.redraw();
    }

    /// The window point of the last frame's middle of view row `row` in visible column `ix`,
    /// of column `ix`'s header, of row `row`'s number, and of column `ix`'s right header edge.
    fn points(
        &self,
        grid: &Entity<GridView>,
        row: u64,
        ix: usize,
    ) -> [gpui::Point<gpui::Pixels>; 4] {
        self.vcx.cx.read(|cx| {
            let g = grid.read(cx);
            let geo = g.geometry().expect("a frame").clone();
            let layout = g.column_layout();
            let left = g.scroll_state().left_x;
            let x0 = f32::from(geo.body.left());
            let x = x0 + ((layout.x_of(ix) + layout.x_of(ix + 1)) / 2.0 - left) as f32;
            let edge = x0 + (layout.x_of(ix + 1) - left) as f32;
            let y = f32::from(geo.body.top()) + geo.row_h * (row as f32 + 0.5);
            [
                gpui::point(gpui::px(x), gpui::px(y)),
                gpui::point(gpui::px(x), geo.header.center().y),
                gpui::point(geo.gutter.center().x, gpui::px(y)),
                gpui::point(gpui::px(edge), geo.header.center().y),
            ]
        })
    }

    // ## m3c-t39 fix round 1 (m4)
    /// What the open grid menu is over, stamp included.
    fn opened(&self) -> MenuTarget {
        self.vcx
            .cx
            .read(context_menu::open_target)
            .expect("a grid menu is open")
    }

    /// The open menu is over `want` (its stamp aside).
    fn assert_opened(&self, want: &MenuTarget) {
        let got = self.opened();
        assert_eq!(
            (got.kind, got.col, got.cell),
            (want.kind, want.col, want.cell)
        );
        assert!(got.result.is_some(), "the menu is stamped with its result");
    }

    fn menu_open(&self) -> bool {
        layer_ids(self.vcx).last() == Some(&"menu")
    }

    /// The painted text of view row `row`'s cell in column `col`.
    fn shown(&self, grid: &Entity<GridView>, row: u64, col: u32) -> String {
        self.vcx.cx.read(|cx| {
            grid.read(cx)
                .painted_cells()
                .iter()
                .find(|c| c.row == row && c.col == ColId(col))
                .map(|c| c.text.to_string())
                .unwrap_or_default()
        })
    }

    fn width(&self, grid: &Entity<GridView>, col: u32) -> f32 {
        self.vcx.cx.read(|cx| {
            grid.read(cx)
                .column_layout()
                .width_of(ColId(col))
                .expect("a shown column")
        })
    }
}

fn press(t: &mut Tile, at: gpui::Point<gpui::Pixels>, button: gpui::MouseButton, clicks: usize) {
    t.vcx.simulate_event(gpui::MouseDownEvent {
        button,
        position: at,
        modifiers: gpui::Modifiers::none(),
        click_count: clicks,
        first_mouse: false,
    });
    t.vcx.simulate_event(gpui::MouseUpEvent {
        button,
        position: at,
        modifiers: gpui::Modifiers::none(),
        click_count: clicks,
    });
    t.redraw();
}

fn describe(item: &MenuItemSpec, cx: &gpui::App) -> String {
    if item.separator {
        return "|".into();
    }
    let mut s = String::new();
    if item.checked {
        s.push_str("✓ ");
    }
    s.push_str(&plain(&(item.label)(cx)));
    if let Some(hint) = &item.hint {
        s.push_str(" · ");
        s.push_str(&plain(&hint(cx)));
    }
    if item.disabled {
        s.push_str(" (off)");
    }
    s
}

fn strs(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

// ---- the plan's tests ---------------------------------------------------------------------

/// The cell, header and row-number menus (NOTES surface 23) as the grid stands: a right press
/// opens the one over what it hits; disabled items keep their place and say why; the header's
/// sort rows check the current direction and offer the next sort level; the menu's own rows
/// sort, freeze, hide, show and copy the column name.
#[gpui::test]
fn menu_variants_and_disabled_reasons(cx: &mut TestAppContext) {
    let mut t = tile(cx);
    let grid = t.show(five());

    // A right press on a cell opens the cell menu at the pointer, nothing active.
    let [cell_at, header_at, row_at, _] = t.points(&grid, 1, 1);
    press(&mut t, cell_at, gpui::MouseButton::Right, 1);
    assert!(t.menu_open(), "{:?}", layer_ids(t.vcx));
    t.assert_opened(&on_cell(1, 1));
    let menu = t
        .vcx
        .cx
        .read(context_menu::open_menu)
        .expect("the menu is open");
    assert_eq!(t.vcx.cx.read(|cx| menu.read(cx).selected()), None);
    let bounds = t.vcx.debug_bounds("popup-menu").expect("drawn");
    // ## m3c-t41 merge: anchored menus snap to whole pixels (T41 m5), so the origin is the
    // pointer to within half a pixel.
    assert!(
        (f32::from(bounds.origin.x) - f32::from(cell_at.x)).abs() <= 0.5
            && (f32::from(bounds.origin.y) - f32::from(cell_at.y)).abs() <= 0.5,
        "{:?} vs {:?}",
        bounds.origin,
        cell_at
    );
    assert_eq!(
        t.items(&grid, on_cell(1, 1)),
        strs(&[
            "Copy",
            "Copy with headers",
            "|",
            "Open value viewer",
            "Raw display · this column",
            "|",
            "Sort ascending",
            "Sort descending",
            "Clear sort · Not sorted (off)",
            "|",
            "Go to row…",
        ])
    );
    t.vcx.simulate_keystrokes("escape");
    t.redraw();
    assert!(!t.menu_open());

    // The header menu.
    press(&mut t, header_at, gpui::MouseButton::Right, 1);
    assert!(t.menu_open());
    t.assert_opened(&on_header(1));
    t.vcx.simulate_keystrokes("escape");
    t.redraw();
    let header = |frozen: &str, hide: &str, show: &str| {
        strs(&[
            "Sort ascending",
            "Sort descending",
            "Clear sort · Not sorted (off)",
            "|",
            frozen,
            hide,
            show,
            "Fit width",
            "|",
            "Raw display · this column",
            "Copy column name",
        ])
    };
    assert_eq!(
        t.items(&grid, on_header(1)),
        header(
            "Freeze up to here",
            "Hide column",
            "Show all columns · None hidden (off)"
        )
    );

    // The row-number menu.
    press(&mut t, row_at, gpui::MouseButton::Right, 1);
    assert!(t.menu_open());
    t.assert_opened(&on_row(1, 1));
    t.vcx.simulate_keystrokes("escape");
    t.redraw();
    assert_eq!(
        t.items(&grid, on_row(1, 1)),
        strs(&[
            "Copy rows",
            "Copy with headers",
            "|",
            "Show as record",
            "Go to row…"
        ])
    );

    // Sorted by `id`: its header checks Sort ascending, Clear sort is on, and `amount`'s
    // header offers the second sort level.
    t.choose(&grid, on_header(0), context_menu::SORT_ASC);
    let sort = |t: &Tile| t.vcx.cx.read(|cx| grid.read(cx).sort().clone());
    assert_eq!(sort(&t), SortSpec::by(ColId(0), false));
    let id_header = t.items(&grid, on_header(0));
    assert_eq!(
        id_header[..3],
        strs(&["✓ Sort ascending", "Sort descending", "Clear sort"])
    );
    let amount_header = t.items(&grid, on_header(1));
    assert_eq!(amount_header[3], "Add as sort level 2");
    t.choose(&grid, on_header(1), context_menu::ADD_SORT);
    assert_eq!(
        sort(&t).keys.as_slice(),
        [
            SortKey {
                col: ColId(0),
                desc: false
            },
            SortKey {
                col: ColId(1),
                desc: false
            }
        ]
    );
    t.choose(&grid, on_cell(0, 2), context_menu::SORT_DESC);
    assert_eq!(sort(&t), SortSpec::by(ColId(2), true));
    assert_eq!(t.items(&grid, on_header(2))[1], "✓ Sort descending");
    t.choose(&grid, on_cell(0, 2), context_menu::CLEAR_SORT);
    assert!(sort(&t).is_empty());

    // Two ranges that share neither rows nor columns: Copy is refused, the viewer needs one
    // cell.
    in_window(t.vcx, |_, cx| {
        grid.update(cx, |g, cx| g.select_cells(&[cell(0, 0), cell(2, 1)], 0, cx))
    });
    t.redraw();
    let cells = t.items(&grid, on_cell(2, 1));
    assert_eq!(cells[0], "Copy · Ranges don't line up (off)");
    assert_eq!(cells[1], "Copy with headers · Ranges don't line up (off)");
    assert_eq!(cells[3], "Open value viewer · Select one cell (off)");
    let rows = t.items(&grid, on_row(2, 1));
    assert_eq!(rows[0], "Copy rows · Ranges don't line up (off)");

    // Hide `name` through the menu itself (its sixth row), then `amount`: `id` is the last
    // shown column; Show all columns brings both back.
    let [_, name_header, ..] = t.points(&grid, 0, 2);
    press(&mut t, name_header, gpui::MouseButton::Right, 1);
    assert!(t.menu_open());
    click(t.vcx, "menu-item-5");
    t.redraw();
    assert!(!t.menu_open());
    let hidden = |t: &Tile| {
        t.vcx
            .cx
            .read(|cx| grid.read(cx).column_layout().hidden().to_vec())
    };
    assert_eq!(hidden(&t), [ColId(2)]);
    t.choose(&grid, on_header(1), context_menu::HIDE);
    assert_eq!(
        t.items(&grid, on_header(0)),
        header(
            "Freeze up to here",
            "Hide column · Last shown column (off)",
            "Show all columns"
        )
    );
    t.choose(&grid, on_header(0), context_menu::HIDE);
    assert_eq!(hidden(&t).len(), 2, "the last shown column stays");
    t.choose(&grid, on_header(0), context_menu::SHOW_ALL);
    assert!(hidden(&t).is_empty());

    // Freeze up to `amount`: the header then offers Unfreeze columns.
    t.choose(&grid, on_header(1), context_menu::FREEZE);
    assert_eq!(t.vcx.cx.read(|cx| grid.read(cx).frozen()), 2);
    assert_eq!(t.items(&grid, on_header(2))[4], "Unfreeze columns");

    // Copy column name copies the name (data).
    t.choose(&grid, on_header(2), context_menu::COPY_NAME);
    assert_eq!(
        t.vcx
            .cx
            .read_from_clipboard()
            .and_then(|i| i.text())
            .as_deref(),
        Some("name")
    );
}

/// A double-click on a header's right edge fits the column to its values, and so does the
/// header menu's Fit width; the console keeps the fitted width (`ColumnResized`).
#[gpui::test]
fn double_click_header_edge_auto_fits(cx: &mut TestAppContext) {
    let mut t = tile(cx);
    let grid = t.show(five());
    let narrow = |t: &Tile| {
        in_window(t.vcx, |_, cx| {
            grid.update(cx, |g, cx| {
                g.set_column_width(ColId(2), 60.0, cx);
            })
        });
        t.redraw();
        assert_eq!(t.width(&grid, 2), 60.0);
    };
    let kept = |t: &Tile| {
        t.vcx.cx.read(|cx| {
            t.results
                .read(cx)
                .tab(0)
                .and_then(|e| e.widths().iter().find(|(c, _)| *c == ColId(2)).map(|w| w.1))
        })
    };
    narrow(&t);
    let [.., edge] = t.points(&grid, 0, 2);
    press(&mut t, edge, gpui::MouseButton::Left, 2);
    let fitted = t.width(&grid, 2);
    assert!(fitted > 120.0, "fitted to the long name: {fitted}");
    assert_eq!(kept(&t), Some(fitted));

    // The header menu's Fit width (its eighth row) does the same.
    narrow(&t);
    let [_, header_at, ..] = t.points(&grid, 0, 2);
    press(&mut t, header_at, gpui::MouseButton::Right, 1);
    assert!(t.menu_open());
    click(t.vcx, "menu-item-7");
    t.redraw();
    assert_eq!(t.width(&grid, 2), fitted);
}

/// Raw display per column (the menu's check item, "this column") and for every grid
/// (`grid.raw_display`): a raw column shows the server's digits, ungrouped; the column's own
/// choice wins over the setting; Copy is raw whichever way the cell is shown.
#[gpui::test]
fn raw_display_per_column_and_global(cx: &mut TestAppContext) {
    let t = tile(cx);
    let grid = t.show(five());
    // `amount` (row 0: 1,234,567) and `id` (an ID column: never grouped).
    assert_eq!(t.shown(&grid, 0, 1), "1,234,567");
    assert_eq!(t.shown(&grid, 3, 1), "77,000");
    assert_eq!(
        t.items(&grid, on_cell(0, 1))[4],
        "Raw display · this column"
    );

    // This column raw.
    t.choose(&grid, on_cell(0, 1), context_menu::RAW);
    assert_eq!(t.shown(&grid, 0, 1), "1234567");
    assert_eq!(t.shown(&grid, 3, 1), "77000");
    assert_eq!(
        t.items(&grid, on_cell(0, 1))[4],
        "✓ Raw display · this column"
    );
    assert_eq!(
        t.items(&grid, on_header(1))[9],
        "✓ Raw display · this column"
    );
    assert!(
        t.vcx
            .cx
            .read(|cx| context_menu::is_raw(t.console, "amount", cx))
    );

    // Copy is raw either way.
    let copy = |t: &Tile| {
        in_window(t.vcx, |_, cx| {
            cx.write_to_clipboard(gpui::ClipboardItem::new_string("sentinel".into()));
            grid.update(cx, |g, cx| g.focus_cell(cell(0, 1), cx));
        });
        t.redraw();
        assert!(run(t.vcx, "grid.copy"));
        t.redraw();
        t.vcx
            .cx
            .read_from_clipboard()
            .and_then(|i| i.text())
            .unwrap_or_default()
    };
    assert_eq!(copy(&t), "1234567\n");
    t.choose(&grid, on_cell(0, 1), context_menu::RAW);
    assert_eq!(t.shown(&grid, 0, 1), "1,234,567");
    assert_eq!(copy(&t), "1234567\n");

    // The setting: every column raw...
    in_window(t.vcx, |_, cx| {
        update_settings(cx, |s| s.grid.raw_display = true);
    });
    t.redraw();
    assert_eq!(t.shown(&grid, 0, 1), "1234567");
    assert_eq!(
        t.items(&grid, on_cell(0, 1))[4],
        "✓ Raw display · this column"
    );
    // ...but a column the user set back shows grouped, and a new result keeps that choice.
    t.choose(&grid, on_cell(0, 1), context_menu::RAW);
    assert_eq!(t.shown(&grid, 0, 1), "1,234,567");
    let grid = t.show(five());
    assert_eq!(t.shown(&grid, 0, 1), "1,234,567");
    // ## m3c-t39 fix round 1 (m2, controller ruling): the column's own choice keeps overriding
    // the setting until the user clears it. Off and on again: still grouped.
    let amount_raw = |t: &Tile| {
        t.vcx
            .cx
            .read(|cx| context_menu::is_raw(t.console, "amount", cx))
    };
    for raw in [false, true] {
        in_window(t.vcx, |_, cx| {
            update_settings(cx, |s| s.grid.raw_display = raw);
        });
        t.redraw();
        assert_eq!(t.shown(&grid, 0, 1), "1,234,567", "setting {raw}");
        assert!(!amount_raw(&t));
    }
    // Choosing the setting's value clears the choice: the column follows the setting again.
    t.choose(&grid, on_cell(0, 1), context_menu::RAW);
    assert_eq!(t.shown(&grid, 0, 1), "1234567");
    in_window(t.vcx, |_, cx| {
        update_settings(cx, |s| s.grid.raw_display = false);
    });
    t.redraw();
    assert_eq!(t.shown(&grid, 0, 1), "1,234,567");
    assert!(!amount_raw(&t));
}

/// ⇧F10 in a focused grid opens the cell menu under the focus cell with its first item
/// active; Escape closes it and gives the focus back to the grid; Enter on an item runs it.
#[gpui::test]
fn menu_opens_from_the_keyboard_at_the_focus_cell(cx: &mut TestAppContext) {
    let t = tile(cx);
    let grid = t.show(five());
    in_window(t.vcx, |_, cx| {
        grid.update(cx, |g, cx| g.focus_cell(cell(2, 1), cx))
    });
    t.redraw();
    t.vcx.simulate_keystrokes("shift-f10");
    t.redraw();
    assert!(t.menu_open(), "{:?}", layer_ids(t.vcx));
    let menu = t
        .vcx
        .cx
        .read(context_menu::open_menu)
        .expect("the menu is open");
    assert_eq!(t.vcx.cx.read(|cx| menu.read(cx).selected()), Some(0));
    // Its top left is the focus cell's bottom left.
    let (left, bottom) = t.vcx.cx.read(|cx| {
        let g = grid.read(cx);
        let geo = g.geometry().expect("a frame").clone();
        let x = f32::from(geo.body.left()) + g.column_layout().x_of(1) as f32;
        let y = f32::from(geo.body.top()) + geo.row_h * 3.0;
        (x, y)
    });
    let bounds = t.vcx.debug_bounds("popup-menu").expect("drawn");
    assert!(
        (f32::from(bounds.origin.x) - left).abs() < 1.0,
        "{bounds:?} vs {left}"
    );
    assert!(
        (f32::from(bounds.origin.y) - bottom).abs() < 1.0,
        "{bounds:?} vs {bottom}"
    );
    assert_eq!(t.items(&grid, on_cell(2, 1))[0], "Copy");

    // Escape: closed, the grid has the focus again.
    t.vcx.simulate_keystrokes("escape");
    t.redraw();
    assert!(!t.menu_open());
    let handle = t.vcx.cx.read(|cx| grid.read(cx).focus_handle(cx));
    assert_eq!(focused(t.vcx), Some(handle));

    // Enter on the first item (Copy) copies the focus cell, raw.
    in_window(t.vcx, |_, cx| {
        cx.write_to_clipboard(gpui::ClipboardItem::new_string("sentinel".into()))
    });
    t.vcx.simulate_keystrokes("shift-f10");
    t.redraw();
    t.vcx.simulate_keystrokes("enter");
    t.redraw();
    assert!(!t.menu_open());
    assert_eq!(
        t.vcx
            .cx
            .read_from_clipboard()
            .and_then(|i| i.text())
            .as_deref(),
        Some("10\n")
    );
}

// ---- fix round 1 ------------------------------------------------------------------------------

/// `secret` (text), `pk` (int8), `x` (int8): another result for a re-run into the same tab.
fn other_schema() -> Arc<ResultSchema> {
    Arc::new(ResultSchema {
        columns: Arc::from([
            ColumnMeta {
                name: "secret".into(),
                logical: LogicalType::Text {
                    max_chars: None,
                    fixed: false,
                },
                native: NativeType::Pg {
                    oid: 25,
                    typmod: -1,
                    name: "text".into(),
                },
                origin: None,
            },
            int8("pk"),
            int8("x"),
        ]),
    })
}

fn rerun_source() -> Arc<dyn ResultSource> {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1 << 45);
    let id = ResultId(NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed));
    let schema = other_schema();
    let budget = MemoryBudget::new(u64::MAX, u64::MAX);
    let (store, mut writer) = ResultStore::new(id, schema.clone(), budget, 1);
    writer.append(Chunk::from_values(
        &schema,
        vec![vec![Value::Text("pw".into()), Value::Int(1), Value::Int(2)]],
    ));
    writer.set_state(Completeness::Complete);
    StoreSource::new(store, Arc::new(NoControl), FetchPolicy::default())
}

fn clipboard(t: &Tile) -> String {
    t.vcx
        .cx
        .read_from_clipboard()
        .and_then(|i| i.text())
        .unwrap_or_default()
}

/// I1 (review probe A): a menu opened over `amount` closes when a re-run's result lands in the
/// same tab, and its rows never act on the new result (here `pk` would take `amount`'s place);
/// nor once the console shows another tab.
#[gpui::test]
fn a_menu_over_an_old_result_closes_and_never_acts_on_the_new_one(cx: &mut TestAppContext) {
    let mut t = tile(cx);
    let grid = t.show(five());
    let [_, header_at, ..] = t.points(&grid, 0, 1);
    press(&mut t, header_at, gpui::MouseButton::Right, 1);
    assert!(t.menu_open());
    let stale = t.opened();
    in_window(t.vcx, |_, cx| {
        cx.write_to_clipboard(gpui::ClipboardItem::new_string("sentinel".into()));
        t.results
            .update(cx, |r, cx| r.apply(&[tab(rerun_source())], cx));
    });
    t.redraw();
    assert!(!t.menu_open(), "the re-run closed the menu");
    let same_grid = t.vcx.cx.read(|cx| {
        hosts(cx)
            .into_iter()
            .find_map(|h| h.read(cx).grid_of(t.console, 0, cx))
    });
    assert_eq!(same_grid.as_ref(), Some(&grid), "retargeted in place");
    // Its rows, chosen anyway, do nothing.
    for action in [
        context_menu::COPY_NAME,
        context_menu::HIDE,
        context_menu::RAW,
        context_menu::SORT_ASC,
        context_menu::FREEZE,
    ] {
        t.choose(&grid, stale.clone(), action);
    }
    assert_eq!(clipboard(&t), "sentinel", "never the new result's `pk`");
    t.vcx.cx.read(|cx| {
        let g = grid.read(cx);
        assert!(g.column_layout().hidden().is_empty());
        assert_eq!(g.frozen(), 0);
        assert!(g.sort().is_empty());
        assert!(!context_menu::is_raw(t.console, "pk", cx));
        assert!(!context_menu::is_raw(t.console, "amount", cx));
    });
    // A menu over the new result names its own column (`pk`, the second).
    let [_, header_at, ..] = t.points(&grid, 0, 1);
    press(&mut t, header_at, gpui::MouseButton::Right, 1);
    assert!(t.menu_open());
    t.choose(&grid, t.opened(), context_menu::COPY_NAME);
    assert_eq!(clipboard(&t), "pk");
    // Another tab shown (Output) closes an open menu too.
    press(&mut t, header_at, gpui::MouseButton::Right, 1);
    assert!(t.menu_open());
    in_window(t.vcx, |_, cx| {
        t.results
            .update(cx, |r, cx| r.set_active(ActiveResult::Output, cx))
    });
    t.redraw();
    assert!(!t.menu_open(), "showing another tab closed the menu");
}

/// m1: "Show as record" from a row number inside a wider row selection shows that row, not
/// the selection's focus row.
#[gpui::test]
fn show_as_record_shows_the_row_the_menu_is_over(cx: &mut TestAppContext) {
    let mut t = tile(cx);
    let grid = t.show(five());
    let [.., row1, _] = t.points(&grid, 1, 0);
    let [.., row3, _] = t.points(&grid, 3, 0);
    // Rows 3 up to 1 selected, the focus on row 1.
    press(&mut t, row3, gpui::MouseButton::Left, 1);
    t.vcx.simulate_event(gpui::MouseDownEvent {
        button: gpui::MouseButton::Left,
        position: row1,
        modifiers: gpui::Modifiers::shift(),
        click_count: 1,
        first_mouse: false,
    });
    t.vcx.simulate_event(gpui::MouseUpEvent {
        button: gpui::MouseButton::Left,
        position: row1,
        modifiers: gpui::Modifiers::shift(),
        click_count: 1,
    });
    t.redraw();
    let focus_row = |t: &Tile| {
        t.vcx
            .cx
            .read(|cx| grid.read(cx).selection().focus().map(|f| f.row))
    };
    assert_eq!(focus_row(&t), Some(ModelRow(1)));
    press(&mut t, row3, gpui::MouseButton::Right, 1);
    assert!(t.menu_open());
    let target = t.opened();
    assert_eq!(target.kind, MenuKind::RowNumber);
    assert_eq!(target.cell.map(|c| c.row), Some(ModelRow(3)));
    // "Show as record" is the fourth row.
    click(t.vcx, "menu-item-3");
    t.redraw();
    let record = t
        .vcx
        .cx
        .read(|cx| hacivat_app::results::views::record(t.console, cx))
        .expect("the inspector shows a row");
    assert_eq!(record.row, ModelRow(3));
    assert_eq!(focus_row(&t), Some(ModelRow(3)));
}

/// m4: a right press under a sort and under a filter opens the menu over the model cell the
/// view row shows; the Transposed view's grid opens a cell menu over the table cell, which
/// stops after Raw display.
#[gpui::test]
fn right_press_under_sort_filter_and_transposed(cx: &mut TestAppContext) {
    let mut t = tile(cx);
    let grid = t.show(five());
    // Sorted by `amount`, largest first: view row 1 shows model row 4 (1,000,000).
    t.choose(&grid, on_header(1), context_menu::SORT_DESC);
    let [at, ..] = t.points(&grid, 1, 1);
    press(&mut t, at, gpui::MouseButton::Right, 1);
    t.assert_opened(&on_cell(4, 1));
    t.vcx.simulate_keystrokes("escape");
    t.redraw();
    t.choose(&grid, on_header(1), context_menu::CLEAR_SORT);
    // Filtered to model rows 1 and 3: view row 1 shows model row 3.
    in_window(t.vcx, |_, cx| {
        grid.update(cx, |g, cx| {
            g.set_view_map(
                hacivat_ui_grid::model::ViewMap::filtered(Arc::from([1u64, 3]), 5),
                cx,
            )
        })
    });
    t.redraw();
    let [at, _, row_at, _] = t.points(&grid, 1, 2);
    press(&mut t, at, gpui::MouseButton::Right, 1);
    t.assert_opened(&on_cell(3, 2));
    t.vcx.simulate_keystrokes("escape");
    t.redraw();
    press(&mut t, row_at, gpui::MouseButton::Right, 1);
    assert_eq!(t.opened().cell.map(|c| c.row), Some(ModelRow(3)));
    t.vcx.simulate_keystrokes("escape");
    t.redraw();
    in_window(t.vcx, |_, cx| {
        grid.update(cx, |g, cx| {
            g.set_view_map(hacivat_ui_grid::model::ViewMap::identity(0), cx)
        })
    });
    t.redraw();

    // The Transposed view.
    in_window(t.vcx, |_, cx| {
        hacivat_app::results::views::set_mode(
            t.console,
            hacivat_app::results::views::ViewMode::Transposed,
            cx,
        );
    });
    t.redraw();
    let tgrid = t
        .vcx
        .cx
        .read(|cx| hacivat_app::results::views::transposed_grid(t.console, cx))
        .expect("the Transposed view's grid");
    let [at, ..] = t.points(&tgrid, 1, 2);
    press(&mut t, at, gpui::MouseButton::Right, 1);
    assert!(t.menu_open());
    let (pressed, table_cell) = t.vcx.cx.read(|cx| {
        let pressed = tgrid
            .read(cx)
            .selection()
            .focus()
            .expect("the press focused a cell");
        let source = hacivat_app::results::views::transposed_source(t.console, cx)
            .expect("the Transposed source");
        (pressed, source.table_cell(pressed))
    });
    let table_cell = table_cell.expect("a data cell");
    assert_ne!(pressed, table_cell, "the axes are swapped");
    let target = t.opened();
    assert_eq!(
        (target.kind, target.cell),
        (MenuKind::Cell, Some(table_cell))
    );
    assert_eq!(
        t.items(&tgrid, target),
        strs(&[
            "Copy",
            "Copy with headers",
            "|",
            "Open value viewer",
            "Raw display · this column",
        ])
    );
}
