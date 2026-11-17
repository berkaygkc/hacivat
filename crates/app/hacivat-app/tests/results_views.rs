//! M3c Task 38 (HV-GRID-12, R-M3-82; NOTES surfaces 21 and 22): the Transposed, record and text
//! views, and the read-only value viewer.
//!
//! - The view switch (Table / Transposed / Text); Transposed is disabled above 100,000 rows with
//!   its reason, and a result is never transposed for the user.
//! - The record inspector reads the focused **model** row, suffixes duplicate column names, and
//!   steps through the rows as shown.
//! - The text view is a Pretty ASCII table of the rows as shown; whole lines select and copy;
//!   above 100,000 rows it asks first.
//! - The value viewer copies the raw value whatever it shows (Pretty is display only), and a
//!   truncated value offers "Re-run with a 64 MB value limit", which goes through Run's pipeline
//!   (the classifier, the gate, production's confirmations) with the raised limit.
//!
//! The tile tests register consoles straight into the registry over result-store fixtures (as
//! `results_find.rs`); the re-run tests run over FakeDriver sessions (as `exec_ui_m3b.rs`).

#![cfg_attr(not(unix), allow(dead_code, unused_imports, unused_variables))]

mod support;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use gpui::{AppContext as _, Entity, TestAppContext, VisualTestContext};
use hacivat_app::console::{ConsoleId, ConsoleState, Consoles};
use hacivat_app::exec_ui::{confirm, orchestrator};
use hacivat_app::results::adapter::ResultDisplay;
use hacivat_app::results::value_viewer::{self, ValueForm};
use hacivat_app::results::view::hosts;
use hacivat_app::results::views::{self, TextState, ViewMode};
use hacivat_app::results::{ActiveResult, ConsoleResults, ResultsEvent};
use hacivat_driver::ExecSummary;
use hacivat_driver::fake::{
    FakeCall, FakeDriver, FakeExec, FakeQuery, FakeResultSet, FakeRows, FakeTypedCall,
};
use hacivat_exec::{ClassifyPool, ResultTab, SessionLog, TabKey};
use hacivat_proto::{DbmsKind, EnvTag};
use hacivat_results::{
    CloseReason, Completeness, FetchControl, FetchPolicy, MemoryBudget, ResultId, ResultSource,
    ResultStore, StoreSource,
};
use hacivat_session::session::{ClientId, RunId, UnitId};
use hacivat_ui_grid::model::SortSpec;
use hacivat_ui_grid::{ColId, GridDataSource as _, GridView, ModelCell, ModelRow};
use hacivat_values::valfmt::DisplayZone;
use hacivat_values::{
    Chunk, ColumnMeta, LenUnit, LogicalType, NativeType, ResultSchema, TruncatedVal, Value,
};
use support::*;

// ---- fixtures -------------------------------------------------------------------------------

fn pg(oid: u32, name: &str) -> NativeType {
    NativeType::Pg {
        oid,
        typmod: -1,
        name: name.into(),
    }
}

fn int8() -> LogicalType {
    LogicalType::Int {
        bits: 64,
        signed: true,
    }
}

fn text() -> LogicalType {
    LogicalType::Text {
        max_chars: None,
        fixed: false,
    }
}

fn column(name: &str, logical: LogicalType, native: NativeType) -> ColumnMeta {
    ColumnMeta {
        name: name.into(),
        logical,
        native,
        origin: None,
    }
}

/// `qty` (int8), `name` (text) and a second `qty` (int8).
fn dup_schema() -> Arc<ResultSchema> {
    Arc::new(ResultSchema {
        columns: Arc::from([
            column("qty", int8(), pg(20, "int8")),
            column("name", text(), pg(25, "text")),
            column("qty", int8(), pg(20, "int8")),
        ]),
    })
}

/// `qty` (int8) and `name` (text).
fn schema() -> Arc<ResultSchema> {
    Arc::new(ResultSchema {
        columns: Arc::from([
            column("qty", int8(), pg(20, "int8")),
            column("name", text(), pg(25, "text")),
        ]),
    })
}

/// One column `doc` of `logical` (`native`).
fn one(name: &str, logical: LogicalType, native: NativeType) -> Arc<ResultSchema> {
    Arc::new(ResultSchema {
        columns: Arc::from([column(name, logical, native)]),
    })
}

struct NoFetch;

impl FetchControl for NoFetch {
    fn fetch_more(&self, _: ResultId, _: u64) {}
    fn close(&self, _: ResultId, _: CloseReason) {}
    fn fetch_all(&self, _: ResultId) {}
}

/// A complete store of `rows` under `schema`.
fn store(schema: Arc<ResultSchema>, rows: Vec<Vec<Value>>) -> Arc<dyn ResultSource> {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1 << 42);
    let id = ResultId(NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed));
    let budget = MemoryBudget::new(u64::MAX, u64::MAX);
    let (store, mut writer) = ResultStore::new(id, schema.clone(), budget, 1);
    if !rows.is_empty() {
        writer.append(Chunk::from_values(&schema, rows));
    }
    writer.set_state(Completeness::Complete);
    StoreSource::new(store, Arc::new(NoFetch), FetchPolicy::default())
}

/// `n` rows of `(i, "r{i}")`.
fn numbered(n: u64) -> Vec<Vec<Value>> {
    (0..n)
        .map(|i| vec![Value::Int(i as i64), Value::Text(format!("r{i}").into())])
        .collect()
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

/// A shell whose focused pane shows a console with empty results.
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

impl Tile {
    fn show(&self, source: Arc<dyn ResultSource>) -> Entity<GridView> {
        in_window(self.vcx, |_, cx| {
            self.results.update(cx, |r, cx| {
                r.apply(&[tab(source)], cx);
                r.set_active(ActiveResult::Tab(0), cx);
            })
        });
        self.redraw();
        self.vcx
            .cx
            .read(|cx| {
                hosts(cx)
                    .into_iter()
                    .find_map(|h| h.read(cx).grid_of(self.console, 0, cx))
            })
            .expect("the tab's grid was shown")
    }

    fn redraw(&self) {
        in_window(self.vcx, |window, _| window.refresh());
        self.vcx.run_until_parked();
    }

    fn set_mode(&self, mode: ViewMode) -> bool {
        let console = self.console;
        let done = in_window(self.vcx, move |_, cx| views::set_mode(console, mode, cx));
        self.redraw();
        done
    }

    fn mode(&self) -> ViewMode {
        self.vcx.cx.read(|cx| views::mode(self.console, cx))
    }

    fn focus(&self, grid: &Entity<GridView>, row: u64, col: u32) {
        in_window(self.vcx, |_, cx| {
            grid.update(cx, |g, cx| g.focus_cell(cell(row, col), cx))
        });
        self.redraw();
    }

    fn command_enabled(&self, id: &str) -> bool {
        self.vcx.cx.read(|cx| {
            cx.global::<hacivat_ui::actions::ActionRegistry>()
                .all()
                .iter()
                .find(|d| d.id == id)
                .is_some_and(|d| (d.enabled)(cx))
        })
    }

    fn drawn(&self) -> Vec<String> {
        self.redraw();
        registry_frame(self.vcx)
            .into_iter()
            .map(|r| plain(&r.text))
            .collect()
    }

    fn clipboard(&self) -> Option<String> {
        self.vcx.read_from_clipboard().and_then(|c| c.text())
    }
}

/// `text` without the Unicode isolation marks Fluent puts around its arguments.
fn plain(text: &str) -> String {
    text.chars()
        .filter(|c| !matches!(c, '\u{2068}' | '\u{2069}'))
        .collect()
}

fn cell(row: u64, col: u32) -> ModelCell {
    ModelCell {
        row: ModelRow(row),
        col: ColId(col),
    }
}

// ---- the plan's tests -----------------------------------------------------------------------

/// Above 100,000 rows Transposed is refused with its reason (the switch keeps it, disabled, with
/// the reason in its tooltip; the command is disabled); at or under it the rows become columns
/// in the order shown. A result is never shown transposed unless asked.
#[gpui::test]
fn transposed_disabled_above_100k_rows(cx: &mut TestAppContext) {
    let t = tile(cx);
    let _grid = t.show(store(schema(), numbered(100_001)));
    assert_eq!(t.mode(), ViewMode::Table, "never auto-transposed");
    let reason = t
        .vcx
        .cx
        .read(|cx| views::transposed_reason(t.console, cx))
        .map(|r| plain(&r));
    assert_eq!(
        reason.as_deref(),
        Some("Transposed shows up to 100,000 rows; this result has 100,001.")
    );
    assert!(!t.set_mode(ViewMode::Transposed));
    assert_eq!(t.mode(), ViewMode::Table);
    assert!(!t.command_enabled("grid.view_transposed"));
    assert!(t.command_enabled("grid.view_text"));
    // The switch is drawn with all three segments (with or without their words: a narrow bar
    // keeps the icons).
    t.redraw();
    for id in [
        "segment-view-table",
        "segment-view-transposed",
        "segment-view-text",
    ] {
        assert!(t.vcx.debug_bounds(id).is_some(), "{id}");
    }

    // A small result transposes: the names first, then one column per row as shown.
    let grid = t.show(store(schema(), numbered(3)));
    assert_eq!(t.mode(), ViewMode::Table, "never auto-transposed");
    assert!(t.command_enabled("grid.view_transposed"));
    // A sort orders the transposed columns (descending by qty).
    in_window(t.vcx, |_, cx| {
        grid.update(cx, |g, cx| g.request_sort(SortSpec::by(ColId(0), true), cx))
    });
    t.redraw();
    t.redraw();
    t.focus(&grid, 2, 1);
    assert!(t.set_mode(ViewMode::Transposed));
    assert_eq!(t.mode(), ViewMode::Transposed);
    let tgrid = t
        .vcx
        .cx
        .read(|cx| views::transposed_grid(t.console, cx))
        .expect("the transposed grid");
    let names: Vec<String> = t.vcx.cx.read(|cx| {
        tgrid
            .read(cx)
            .column_layout()
            .order()
            .iter()
            .map(|c| {
                views::transposed_source(t.console, cx).unwrap().columns()[c.0 as usize]
                    .name
                    .to_string()
            })
            .collect()
    });
    assert_eq!(names, ["Column", "1", "2", "3"]);
    let source = t
        .vcx
        .cx
        .read(|cx| views::transposed_source(t.console, cx))
        .unwrap();
    assert_eq!(
        source.table_rows(),
        &[ModelRow(2), ModelRow(1), ModelRow(0)],
        "the sorted order"
    );
    assert_eq!(t.vcx.cx.read(|cx| tgrid.read(cx).frozen()), 1);
    // The focus followed: model row 2 is the first column, `name` the second row.
    assert_eq!(
        t.vcx.cx.read(|cx| tgrid.read(cx).selection().focus()),
        Some(cell(1, 1))
    );
    // Find and go to row are the table's: off while transposed.
    assert!(!t.command_enabled("grid.find"));
    assert!(!t.command_enabled("grid.goto_row"));
    // Copy copies what is shown, raw: the `name` row of the first two columns.
    in_window(t.vcx, |window, cx| {
        tgrid.update(cx, |g, cx| {
            g.dispatch(hacivat_ui_grid::GridCommand::SelectRow, window, cx)
        })
    });
    in_window(t.vcx, |window, cx| {
        tgrid.update(cx, |g, cx| {
            g.dispatch(
                hacivat_ui_grid::GridCommand::Copy { headers: false },
                window,
                cx,
            )
        })
    });
    t.redraw();
    assert_eq!(t.clipboard().as_deref(), Some("name\tr2\tr1\tr0\n"));
    // Back to the table: its sort and its focus are kept.
    assert!(t.set_mode(ViewMode::Table));
    assert!(t.vcx.cx.read(|cx| !grid.read(cx).sort().is_empty()));
    assert!(t.command_enabled("grid.find"));
}

/// The record inspector shows the focused row (a model row) with one field per column; a
/// name the result has twice gets "(2)"; ▲ ▼ step through the rows as shown (a sort's order).
#[gpui::test]
fn record_view_navigates_and_suffixes_duplicates(cx: &mut TestAppContext) {
    let t = tile(cx);
    let rows = vec![
        vec![Value::Int(10), Value::Text("ankara".into()), Value::Int(1)],
        vec![Value::Int(30), Value::Text("izmir".into()), Value::Int(2)],
        vec![Value::Int(20), Value::Text("bursa".into()), Value::Int(3)],
    ];
    let grid = t.show(store(dup_schema(), rows));
    // Closed by default.
    assert!(!in_window(t.vcx, |window, cx| views::record_open(
        window, cx
    )));
    assert!(
        t.vcx
            .cx
            .read(|cx| { !hosts(cx).is_empty() && views::record(t.console, cx).is_some() })
    );
    in_window(t.vcx, views::toggle_record);
    t.redraw();
    assert!(in_window(t.vcx, |window, cx| views::record_open(
        window, cx
    )));
    t.focus(&grid, 1, 1);
    let record = t
        .vcx
        .cx
        .read(|cx| views::record(t.console, cx))
        .expect("a record");
    assert_eq!(record.row, ModelRow(1));
    assert_eq!(plain(&record.position), "Row 2 of 3");
    let names: Vec<(String, Option<(u64, u64)>)> = record
        .fields
        .iter()
        .map(|f| (f.name.to_string(), f.dup))
        .collect();
    assert_eq!(
        names,
        [
            ("qty".to_string(), Some((1, 2))),
            ("name".to_string(), None),
            ("qty".to_string(), Some((2, 2)))
        ]
    );
    assert_eq!(record.fields[1].value.as_ref(), "izmir");
    assert!(record.fields[0].end, "a number is end-aligned");
    let drawn = t.drawn();
    assert!(drawn.iter().any(|d| d == "(2)"), "{drawn:?}");
    assert!(drawn.iter().any(|d| d == "Row 2 of 3"), "{drawn:?}");
    // ▼ moves the grid's focus to the next row as shown, in the same column.
    in_window(t.vcx, |_, cx| views::record_step(t.console, false, cx));
    t.redraw();
    assert_eq!(
        t.vcx.cx.read(|cx| grid.read(cx).selection().focus()),
        Some(cell(2, 1))
    );
    // Under a sort (qty ascending: 10, 20, 30 = model rows 0, 2, 1) the steps follow the order
    // shown, and the record stays on its model row.
    in_window(t.vcx, |_, cx| {
        grid.update(cx, |g, cx| {
            g.request_sort(SortSpec::by(ColId(0), false), cx)
        })
    });
    t.redraw();
    t.redraw();
    let record = t.vcx.cx.read(|cx| views::record(t.console, cx)).unwrap();
    assert_eq!(record.row, ModelRow(2), "the same row after the sort");
    assert_eq!(plain(&record.position), "Row 2 of 3");
    in_window(t.vcx, |_, cx| views::record_step(t.console, false, cx));
    t.redraw();
    let record = t.vcx.cx.read(|cx| views::record(t.console, cx)).unwrap();
    assert_eq!(record.row, ModelRow(1));
    assert_eq!(plain(&record.position), "Row 3 of 3");
    // Past the last row nothing moves.
    in_window(t.vcx, |_, cx| views::record_step(t.console, false, cx));
    t.redraw();
    assert_eq!(
        t.vcx
            .cx
            .read(|cx| views::record(t.console, cx))
            .unwrap()
            .row,
        ModelRow(1)
    );
    // ▲ goes back.
    in_window(t.vcx, |_, cx| views::record_step(t.console, true, cx));
    t.redraw();
    assert_eq!(
        t.vcx
            .cx
            .read(|cx| views::record(t.console, cx))
            .unwrap()
            .row,
        ModelRow(2)
    );
    // The toggle closes it again; the layout remembers.
    in_window(t.vcx, views::toggle_record);
    t.redraw();
    assert!(!layout(t.vcx).1.inspector_visible);
}

/// The text view is a Pretty ASCII table of the rows as shown (raw values, numbers aligned
/// right, control characters escaped, NULL as NULL); whole lines select and copy as shown.
#[gpui::test]
fn text_view_pretty_table_and_line_copy(cx: &mut TestAppContext) {
    let t = tile(cx);
    let rows = vec![
        vec![Value::Int(1234), Value::Text("a\tb".into())],
        vec![Value::Int(7), Value::Null],
        vec![Value::Int(-5), Value::Text("line\nnext".into())],
    ];
    let _grid = t.show(store(schema(), rows));
    assert!(t.set_mode(ViewMode::Text));
    t.redraw();
    assert_eq!(
        t.vcx.cx.read(|cx| views::text_state(t.console, cx)),
        Some(TextState::Ready)
    );
    let lines = t.vcx.cx.read(|cx| views::text_lines(t.console, cx));
    assert_eq!(
        lines,
        [
            "+------+------------+",
            "| qty  | name       |",
            "+------+------------+",
            "| 1234 | a\\tb       |",
            "|    7 | NULL       |",
            "|   -5 | line\\nnext |",
            "+------+------------+",
        ],
        "raw (never grouped) and aligned"
    );
    // Click line 3, Shift-click line 4: both copy, as shown.
    in_window(t.vcx, |_, cx| {
        views::text_select(t.console, 3, false, cx);
        views::text_select(t.console, 4, true, cx);
    });
    in_window(t.vcx, |_, cx| views::text_copy(t.console, cx));
    assert_eq!(
        t.clipboard().as_deref(),
        Some("| 1234 | a\\tb       |\n|    7 | NULL       |\n")
    );
    // The toolbar's Copy (`results.copy`) copies the lines too.
    in_window(t.vcx, |_, cx| views::text_select(t.console, 5, false, cx));
    assert!(run(t.vcx, "results.copy"));
    t.redraw();
    assert_eq!(t.clipboard().as_deref(), Some("|   -5 | line\\nnext |\n"));

    // Above 100,000 rows it asks first; Back to the table goes back.
    let _grid = t.show(store(schema(), numbered(100_001)));
    // The grid takes the new rows in its frame; the view follows in the next.
    t.redraw();
    assert!(t.set_mode(ViewMode::Text));
    let state = t.vcx.cx.read(|cx| views::text_state(t.console, cx));
    assert!(
        matches!(state, Some(TextState::Asking { rows: 100_001, .. })),
        "{state:?}"
    );
    let drawn = t.drawn();
    assert!(
        drawn.iter().any(|d| d == "Show 100,001 rows as text?"),
        "{drawn:?}"
    );
    in_window(t.vcx, |_, cx| views::text_back(t.console, cx));
    t.redraw();
    assert_eq!(t.mode(), ViewMode::Table);
    // Show as text builds it (on the background executor).
    assert!(t.set_mode(ViewMode::Text));
    in_window(t.vcx, |_, cx| views::text_show_anyway(t.console, cx));
    t.redraw();
    t.redraw();
    assert_eq!(
        t.vcx.cx.read(|cx| views::text_state(t.console, cx)),
        Some(TextState::Ready)
    );
    assert_eq!(
        t.vcx.cx.read(|cx| views::text_lines(t.console, cx)).len(),
        100_001 + 4
    );
}

/// JSON Pretty is display only: Copy raw value copies the server's text unchanged, Pretty or
/// not. A value that is not JSON keeps Raw (Pretty is refused); bytes show as hex.
#[gpui::test]
fn value_viewer_pretty_json_is_display_only(cx: &mut TestAppContext) {
    let t = tile(cx);
    let json = r#"{"b":1,"a":[true,null,"x, y"]}"#;
    let rows = vec![
        vec![Value::Json(json.into())],
        vec![Value::Json("{\"a\": ".into())],
    ];
    let jsonb = LogicalType::Json { binary: true };
    let grid = t.show(store(one("doc", jsonb, pg(3802, "jsonb")), rows));
    in_window(t.vcx, |_, cx| value_viewer::open(t.console, cell(0, 0), cx));
    t.redraw();
    let v = t
        .vcx
        .cx
        .read(|cx| value_viewer::view(t.console, cx))
        .expect("the viewer is open");
    assert_eq!(v.column.as_ref(), "doc");
    assert_eq!(plain(&v.subtitle), "jsonb · row 1");
    assert_eq!(v.form, ValueForm::Json { valid: true });
    assert_eq!(v.body, json, "Raw first");
    in_window(t.vcx, |_, cx| value_viewer::set_pretty(t.console, true, cx));
    t.redraw();
    let v = t
        .vcx
        .cx
        .read(|cx| value_viewer::view(t.console, cx))
        .unwrap();
    assert!(v.pretty);
    assert_eq!(
        v.body,
        "{\n  \"b\": 1,\n  \"a\": [\n    true,\n    null,\n    \"x, y\"\n  ]\n}"
    );
    in_window(t.vcx, |_, cx| value_viewer::copy_raw(t.console, cx));
    assert_eq!(t.clipboard().as_deref(), Some(json), "the server's text");
    let drawn = t.drawn();
    assert!(drawn.iter().any(|d| d == "Copy raw value"), "{drawn:?}");
    // Escape closes it and the focus goes back to the grid.
    t.vcx.simulate_keystrokes("escape");
    t.redraw();
    assert!(!t.vcx.cx.read(|cx| value_viewer::is_open(t.console, cx)));
    // `grid.open_value` (⇧↵) on the focused cell opens it again.
    t.focus(&grid, 0, 0);
    in_window(t.vcx, |window, cx| {
        grid.update(cx, |g, cx| {
            g.dispatch(hacivat_ui_grid::GridCommand::OpenValue, window, cx)
        })
    });
    t.redraw();
    assert_eq!(
        t.vcx.cx.read(|cx| value_viewer::cell(t.console, cx)),
        Some(cell(0, 0))
    );
    t.vcx.simulate_keystrokes("escape");
    t.redraw();

    // Not JSON: Raw stays, Pretty is refused.
    in_window(t.vcx, |_, cx| value_viewer::open(t.console, cell(1, 0), cx));
    t.redraw();
    in_window(t.vcx, |_, cx| value_viewer::set_pretty(t.console, true, cx));
    let v = t
        .vcx
        .cx
        .read(|cx| value_viewer::view(t.console, cx))
        .unwrap();
    assert_eq!(v.form, ValueForm::Json { valid: false });
    assert!(!v.pretty);
    assert_eq!(v.body, "{\"a\": ");
    let drawn = t.drawn();
    assert!(
        drawn
            .iter()
            .any(|d| d
                == "Not valid JSON, so Pretty is off and the value shows as the server sent it."),
        "{drawn:?}"
    );

    // Bytes: hex rows, and Copy raw value copies the raw channel's `\x` text.
    let bytes: Arc<[u8]> = Arc::from(&b"\x00\x01Hacivat!"[..]);
    let _grid = t.show(store(
        one("blob", LogicalType::Bytes { max: None }, pg(17, "bytea")),
        vec![vec![Value::Bytes(bytes)]],
    ));
    in_window(t.vcx, |_, cx| value_viewer::open(t.console, cell(0, 0), cx));
    t.redraw();
    let v = t
        .vcx
        .cx
        .read(|cx| value_viewer::view(t.console, cx))
        .unwrap();
    assert_eq!(v.form, ValueForm::Binary);
    let second = format!("00000008  {:<19}  t!", "7421");
    assert_eq!(
        v.body,
        format!("00000000  0001 4861 6369 7661  ..Haciva\n{second}")
    );
    assert_eq!(plain(&v.footer), "10 bytes");
    in_window(t.vcx, |_, cx| value_viewer::copy_raw(t.console, cx));
    assert_eq!(t.clipboard().as_deref(), Some("\\x00014861636976617421"));
}

/// A value the store cut at the value limit says what was kept and offers "Re-run with a 64 MB
/// value limit", which asks the run UI to re-run the tab's unit with the largest limit; a whole
/// value offers no re-run.
#[gpui::test]
fn truncated_value_offers_a_rerun_with_the_raised_limit(cx: &mut TestAppContext) {
    let t = tile(cx);
    let cut = Value::Truncated(Box::new(TruncatedVal {
        prefix: Value::Text("a".repeat(1 << 20).into()),
        total_len: Some(5 << 20),
        unit: LenUnit::Bytes,
    }));
    let _grid = t.show(store(
        one("note", text(), pg(25, "text")),
        vec![vec![cut], vec![Value::Text("whole".into())]],
    ));
    let heard: Arc<Mutex<Vec<ResultsEvent>>> = Arc::default();
    let sink = heard.clone();
    in_window(t.vcx, |_, cx| {
        cx.subscribe(&t.results, move |_, e: &ResultsEvent, _| {
            sink.lock().unwrap().push(e.clone())
        })
        .detach()
    });
    in_window(t.vcx, |_, cx| value_viewer::open(t.console, cell(0, 0), cx));
    t.redraw();
    let v = t
        .vcx
        .cx
        .read(|cx| value_viewer::view(t.console, cx))
        .unwrap();
    assert_eq!(
        v.truncated.as_deref().map(plain).as_deref(),
        Some("Showing the first 1 MB of 5 MB.")
    );
    assert_eq!(
        v.rerun.as_deref().map(plain).as_deref(),
        Some("Re-run with a 64 MB value limit")
    );
    let drawn = t.drawn();
    assert!(
        drawn.iter().any(|d| d == "Re-run with a 64 MB value limit"),
        "{drawn:?}"
    );
    in_window(t.vcx, |_, cx| value_viewer::rerun(t.console, cx));
    t.redraw();
    assert_eq!(
        heard.lock().unwrap().as_slice(),
        [ResultsEvent::RerunWithLimit {
            console: t.console,
            unit_ordinal: 1,
            cell_cap: hacivat_results::window::MAX_CELL_CAP,
        }]
    );
    assert!(
        !t.vcx.cx.read(|cx| value_viewer::is_open(t.console, cx)),
        "the viewer closes for the re-run"
    );
    // A whole value: no notice, no re-run.
    in_window(t.vcx, |_, cx| value_viewer::open(t.console, cell(1, 0), cx));
    t.redraw();
    let v = t
        .vcx
        .cx
        .read(|cx| value_viewer::view(t.console, cx))
        .unwrap();
    assert_eq!((v.truncated, v.rerun), (None, None));
}

// ---- the re-run over a session ----------------------------------------------------------------

/// `answer` for the SQL-context probes of a PostgreSQL console.
fn probes(prefix: &'static str, value: &str) -> Vec<FakeExec> {
    (0..8)
        .map(|_| FakeExec {
            prefix,
            result: Ok(ExecSummary {
                rows: vec![vec![Some(value.to_string())]],
                row_count: 1,
                ..ExecSummary::default()
            }),
            delay: Duration::ZERO,
            tx_after: None,
            panics: false,
        })
        .collect()
}

fn context_reads() -> Vec<FakeExec> {
    (0..8)
        .map(|_| FakeExec {
            prefix: "SELECT pg_catalog.current_setting('standard_conforming_strings')",
            result: Ok(ExecSummary {
                rows: vec![vec![Some("on".into()), Some("\"$user\", public".into())]],
                row_count: 1,
                ..ExecSummary::default()
            }),
            delay: Duration::ZERO,
            tx_after: None,
            panics: false,
        })
        .collect()
}

/// A text value cut at 1 MiB of 5 MiB (the store keeps what the driver sent).
fn cut_note(_: u64, _: usize) -> Value {
    Value::Truncated(Box::new(TruncatedVal {
        prefix: Value::Text("n".repeat(1 << 20).into()),
        total_len: Some(5 << 20),
        unit: LenUnit::Bytes,
    }))
}

/// A PostgreSQL FakeDriver whose queries starting with `prefix` return one row with a cut
/// `note`, twice.
fn pg_with_note(prefix: &'static str) -> FakeDriver {
    let set = || FakeResultSet {
        columns: vec![("note", text())],
        rows: FakeRows::Generated {
            count: 1,
            cell: cut_note,
        },
    };
    FakeDriver::new(DbmsKind::Postgres)
        .tracks_transactions()
        .exec(
            probes("SHOW standard_conforming_strings", "on")
                .into_iter()
                .chain(context_reads())
                .collect(),
        )
        .queries(vec![
            FakeQuery {
                prefix,
                sets: vec![set()],
                ..Default::default()
            },
            FakeQuery {
                prefix,
                sets: vec![set()],
                ..Default::default()
            },
        ])
}

struct Console {
    vcx: &'static mut VisualTestContext,
    id: ConsoleId,
    fake: Arc<FakeDriver>,
    editor: Entity<hacivat_ui_editor::EditorView>,
}

fn console(
    cx: &mut TestAppContext,
    env: EnvTag,
    fake: FakeDriver,
    pool: Option<Arc<ClassifyPool>>,
) -> Console {
    let vcx = shell_with_connections(cx, "en");
    in_window(vcx, |window, _| window.activate_window());
    if let Some(pool) = pool {
        vcx.cx
            .update(|cx| orchestrator::install_classifier(pool, cx));
    }
    let p = hacivat_session::profile::Profile {
        engine: DbmsKind::Postgres,
        ..test_profile("orders", Some(env), "127.0.0.1")
    };
    let profile = add_profile(vcx, p.clone());
    let rt = vcx.cx.read(|cx| {
        cx.global::<hacivat_app::connections::ConnectionsModel>()
            .core_runtime()
            .handle()
    });
    let (session, fake) =
        futures::executor::block_on(hacivat_session::testing::fake_session(rt, fake, p));
    let id = register_test_console(vcx, profile, session);
    wait_until(vcx, "the console's SQL context", |vcx| {
        vcx.cx.read(|cx| {
            hacivat_app::exec_ui::sql_context(id, cx)
                != hacivat_app::exec_ui::context::SqlContext::Pending
        })
    });
    let editor = vcx.cx.read(|cx| {
        Consoles::get(cx, id)
            .and_then(|s| s.editor.clone())
            .expect("the console has an editor")
    });
    Console {
        vcx,
        id,
        fake,
        editor,
    }
}

impl Console {
    fn run(&self, text: &str) {
        let text = text.to_string();
        self.vcx.cx.update(|cx| {
            self.editor.update(cx, |e, cx| {
                e.set_text(&text, cx);
                e.select_range(0..0, cx);
            })
        });
        self.vcx.run_until_parked();
        let editor = self.editor.clone();
        in_window(self.vcx, |window, cx| {
            editor.update(cx, |e, cx| assert!(e.run_command("editor.run", window, cx)));
        });
        self.vcx.run_until_parked();
    }

    fn queries(&self) -> Vec<String> {
        self.fake
            .typed_log()
            .into_iter()
            .filter_map(|c| match c {
                FakeTypedCall::Query(sql) => Some(sql),
                _ => None,
            })
            .collect()
    }

    fn redraw(&self) {
        in_window(self.vcx, |window, _| window.refresh());
        self.vcx.run_until_parked();
    }

    /// Waits for the result tab, shows it, and opens the viewer on its first cell.
    fn open_viewer(&self) {
        wait_until(self.vcx, "the result tab and its grid", |vcx| {
            in_window(vcx, |window, _| window.refresh());
            vcx.run_until_parked();
            vcx.cx.read(|cx| {
                hosts(cx)
                    .into_iter()
                    .any(|h| h.read(cx).grid_of(self.id, 0, cx).is_some())
            })
        });
        let id = self.id;
        in_window(self.vcx, |_, cx| value_viewer::open(id, cell(0, 0), cx));
        self.redraw();
    }
}

/// What an observed pool classified.
type Seen = Arc<Mutex<Vec<String>>>;

fn observed() -> (Arc<ClassifyPool>, Seen) {
    let seen: Seen = Arc::default();
    let log = seen.clone();
    let pool = ClassifyPool::observed(
        2,
        Arc::new(move |sql: &str, _| log.lock().unwrap().push(sql.to_string())),
    );
    (pool, seen)
}

/// Re-run with a raised limit takes Run's path: the text that ran, still in the editor, is
/// classified again on the pool (the gate decides on it) and sent with the 64 MiB limit; once
/// the statement was edited, nothing is sent (the preview rule: what is sent is what the user
/// sees).
#[gpui::test]
fn rerun_with_raised_limit_goes_through_the_gate(cx: &mut TestAppContext) {
    let (pool, seen) = observed();
    let c = console(cx, EnvTag::Dev, pg_with_note("SELECT note"), Some(pool));
    c.run("SELECT note FROM t");
    wait_until(c.vcx, "the first run", |_| c.queries().len() == 1);
    assert_eq!(
        c.vcx
            .cx
            .read(|cx| orchestrator::last_run_cell_cap(c.id, cx)),
        Some(1 << 20),
        "the setting's limit"
    );
    c.open_viewer();
    let v = c
        .vcx
        .cx
        .read(|cx| value_viewer::view(c.id, cx))
        .expect("the viewer is open");
    assert!(v.rerun.is_some(), "{v:?}");
    let id = c.id;
    in_window(c.vcx, |_, cx| value_viewer::rerun(id, cx));
    wait_until(c.vcx, "the re-run", |_| c.queries().len() == 2);
    assert_eq!(c.queries(), ["SELECT note FROM t", "SELECT note FROM t"]);
    assert_eq!(
        seen.lock().unwrap().as_slice(),
        ["SELECT note FROM t", "SELECT note FROM t"],
        "classified again before it was sent"
    );
    assert_eq!(
        c.vcx
            .cx
            .read(|cx| orchestrator::last_run_cell_cap(c.id, cx)),
        Some(hacivat_results::window::MAX_CELL_CAP),
        "sent with the raised limit"
    );

    // Edited since: nothing is sent, and the console says why.
    c.open_viewer();
    c.vcx.cx.update(|cx| {
        c.editor
            .update(cx, |e, cx| e.set_text("SELECT note FROM u", cx));
    });
    c.vcx.run_until_parked();
    in_window(c.vcx, |_, cx| value_viewer::rerun(id, cx));
    c.redraw();
    assert_eq!(c.queries().len(), 2, "{:?}", c.queries());
    let toasts = toast_ids(c.vcx);
    assert!(
        toasts.iter().any(|t| t == "hv-exec-rerun-edited"),
        "{toasts:?}"
    );
    stop_core_runtime_test(cx);
}

/// On production the re-run asks as Run does: a write is held in Hacivat's savepoint and the
/// Keep / Undo prompt opens before anything is kept, for the re-run too.
#[gpui::test]
fn rerun_with_raised_limit_asks_on_production(cx: &mut TestAppContext) {
    let c = console(cx, EnvTag::Production, pg_with_note("UPDATE"), None);
    let text = "UPDATE t SET a = 1 WHERE id = 1 RETURNING note";
    c.run(text);
    let commit_open = |vcx: &VisualTestContext| {
        layer_ids(vcx).contains(&hacivat_app::exec_ui::commit_prompt::COMMIT_LAYER_ID)
    };
    wait_until(c.vcx, "the Keep/Undo prompt", |vcx| commit_open(vcx));
    c.vcx
        .executor()
        .advance_clock(confirm::ARM_DELAY + Duration::from_millis(50));
    c.vcx.run_until_parked();
    click(c.vcx, "button-commit-keep");
    c.vcx.run_until_parked();
    wait_until(c.vcx, "the prompt closed", |vcx| !commit_open(vcx));
    c.open_viewer();
    let id = c.id;
    in_window(c.vcx, |_, cx| value_viewer::rerun(id, cx));
    wait_until(c.vcx, "the re-run's Keep/Undo prompt", |vcx| {
        commit_open(vcx)
    });
    assert_eq!(c.queries(), [text, text]);
    let savepoints = c
        .fake
        .call_log()
        .iter()
        .filter(|call| matches!(call, FakeCall::Statement(s) if s == "SAVEPOINT hv_confirm"))
        .count();
    assert_eq!(savepoints, 2, "{:?}", c.fake.call_log());
    assert_eq!(
        c.vcx
            .cx
            .read(|cx| orchestrator::last_run_cell_cap(c.id, cx)),
        Some(hacivat_results::window::MAX_CELL_CAP)
    );
    stop_core_runtime_test(cx);
}
