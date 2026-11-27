//! ## m3c-int: the M3c integration review's findings where tasks meet (T36–T38 × re-run).
//!
//! - I1: while a re-run streams, the Transposed view never labels the new result's values with
//!   the old result's column names (it rebuilds at a new adapter generation).
//! - I2: a go-to fetching toward a row stops at a re-run, and its > 1,000,000-row question is
//!   never answered for the new result.
//! - I3: find, Filter rows and go to row stay off a tab shown Transposed or Text, also when the
//!   user switches to a tab that is already in that view.
//! - M2: a filter composed under a new sort keeps the rows fetched while the sort ran.
//! - M3: the value viewer's "row N" follows a sort while it is open.
//! - M4: in the Text view the grid commands that do nothing there are disabled.
//!
//! The fixtures are `results_find.rs`'s (consoles registered straight into the registry over
//! result-store fixtures, R-M3-15).

#![cfg_attr(not(unix), allow(dead_code, unused_imports, unused_variables))]

mod support;

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use gpui::{AppContext as _, Entity, TestAppContext, VisualTestContext};
use hacivat_app::console::{ConsoleId, ConsoleState, Consoles};
use hacivat_app::results::adapter::ResultDisplay;
use hacivat_app::results::find::{self, Toggle};
use hacivat_app::results::goto::{self, GotoLine, GotoState};
use hacivat_app::results::value_viewer;
use hacivat_app::results::view::hosts;
use hacivat_app::results::views::{self, ViewMode};
use hacivat_app::results::{ActiveResult, ConsoleResults};
use hacivat_exec::{ResultTab, SessionLog, TabKey};
use hacivat_proto::DbmsKind;
use hacivat_results::{
    CloseReason, Completeness, CursorInfo, FetchControl, FetchPolicy, MemoryBudget, ResultId,
    ResultSource, ResultStore, ResultWriter, StoreSource,
};
use hacivat_session::session::{ClientId, RunId, UnitId};
use hacivat_ui_grid::model::{SortSpec, ViewMap};
use hacivat_ui_grid::{ColId, GridView, ModelCell, ModelRow, ViewRow};
use hacivat_values::valfmt::DisplayZone;
use hacivat_values::{Chunk, ColumnMeta, LogicalType, NativeType, ResultSchema, Value};
use support::*;

// ---- fixtures (results_find.rs) -------------------------------------------------------------

/// `qty` (int8) and `name` (text), on PostgreSQL.
fn schema() -> Arc<ResultSchema> {
    Arc::new(ResultSchema {
        columns: Arc::from([
            ColumnMeta {
                name: "qty".into(),
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
            },
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

/// Another result's columns: `secret` and `owner` (text).
fn schema_other() -> Arc<ResultSchema> {
    let text = |name: &str| ColumnMeta {
        name: name.into(),
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
    };
    Arc::new(ResultSchema {
        columns: Arc::from([text("secret"), text("owner")]),
    })
}

fn row(id: i64, name: &str) -> Vec<Value> {
    vec![Value::Int(id), Value::Text(name.into())]
}

fn cursor() -> CursorInfo {
    let now = Instant::now();
    CursorInfo {
        opened_at: now,
        last_fetch: now,
        holds_tx: false,
        idle_bound: Duration::from_secs(600),
    }
}

/// Records the rows each `fetch_more` asks for.
#[derive(Default)]
struct Asks(Mutex<Vec<u64>>);

impl FetchControl for Asks {
    fn fetch_more(&self, _: ResultId, rows: u64) {
        self.0.lock().unwrap().push(rows);
    }
    fn close(&self, _: ResultId, _: CloseReason) {}
    fn fetch_all(&self, _: ResultId) {}
}

/// A live store holding `rows` in state `state`, its writer, and the asks its source makes.
fn live(
    rows: Vec<Vec<Value>>,
    state: Completeness,
) -> (Arc<dyn ResultSource>, ResultWriter, Arc<Asks>) {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1 << 45);
    let id = ResultId(NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed));
    let schema = schema();
    let budget = MemoryBudget::new(u64::MAX, u64::MAX);
    let (store, mut writer) = ResultStore::new(id, schema.clone(), budget, 1);
    if !rows.is_empty() {
        writer.append(Chunk::from_values(&schema, rows));
    }
    writer.set_state(state);
    let asks = Arc::new(Asks::default());
    let source = StoreSource::new(store, asks.clone(), FetchPolicy::default());
    (source, writer, asks)
}

fn tab(source: Arc<dyn ResultSource>) -> ResultTab {
    tab_n(source, 0)
}

fn tab_n(source: Arc<dyn ResultSource>, ord: u32) -> ResultTab {
    ResultTab {
        key: TabKey {
            console: 1,
            unit_ordinal: 1,
            result_ordinal: ord,
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
    /// Shows `source` as the console's only result tab (again: a re-run into the same tab).
    fn show(&self, source: Arc<dyn ResultSource>) -> Entity<GridView> {
        in_window(self.vcx, |_, cx| {
            self.results.update(cx, |r, cx| {
                r.apply(&[tab(source)], cx);
                r.set_active(ActiveResult::Tab(0), cx);
            })
        });
        self.redraw();
        self.grid(0)
    }

    /// Shows `a` and `b` as two result tabs, the second active.
    fn show_two(&self, a: Arc<dyn ResultSource>, b: Arc<dyn ResultSource>) {
        in_window(self.vcx, |_, cx| {
            self.results.update(cx, |r, cx| {
                r.apply(&[tab_n(a, 0), tab_n(b, 1)], cx);
                r.set_active(ActiveResult::Tab(1), cx);
            })
        });
        self.redraw();
    }

    fn activate(&self, ix: usize) {
        in_window(self.vcx, |_, cx| {
            self.results
                .update(cx, |r, cx| r.set_active(ActiveResult::Tab(ix), cx))
        });
        self.redraw();
        self.redraw();
    }

    fn grid(&self, ix: usize) -> Entity<GridView> {
        self.vcx
            .cx
            .read(|cx| {
                hosts(cx)
                    .into_iter()
                    .find_map(|h| h.read(cx).grid_of(self.console, ix, cx))
            })
            .expect("the tab's grid was shown")
    }

    fn redraw(&self) {
        // ## m4-t33d (T33 flaky3 follow-up): the last frame renders every view (a frame
        // after `run_until_parked` may reuse a cached view, which records no texts).
        redraw_full(self.vcx);
    }

    fn set_mode(&self, mode: ViewMode) -> bool {
        let console = self.console;
        let done = in_window(self.vcx, move |_, cx| views::set_mode(console, mode, cx));
        self.redraw();
        done
    }

    fn find(&self, query: &str) {
        let console = self.console;
        let query = query.to_string();
        in_window(self.vcx, move |_, cx| {
            if !find::is_open(console, cx) {
                find::open(console, cx);
            }
            find::set_query(console, &query, cx);
        });
        self.redraw();
    }

    fn flip(&self, t: Toggle) {
        let console = self.console;
        in_window(self.vcx, move |_, cx| find::flip(console, t, cx));
        self.redraw();
    }

    fn focus(&self, grid: &Entity<GridView>) -> Option<ModelCell> {
        self.vcx.cx.read(|cx| grid.read(cx).selection().focus())
    }

    fn filtered(&self, grid: &Entity<GridView>) -> bool {
        self.vcx
            .cx
            .read(|cx| matches!(grid.read(cx).view_map(), ViewMap::Filtered { .. }))
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

// ---- I1 -------------------------------------------------------------------------------------

/// I1: the Transposed view never labels a re-run's values with the old column names, also while
/// the re-run streams.
#[gpui::test]
fn transposed_view_rebuilds_at_a_rerun_while_it_streams(cx: &mut TestAppContext) {
    let t = tile(cx);
    let (source, _w, _) = live(vec![row(1, "a"), row(2, "b")], Completeness::Complete);
    let _grid = t.show(source);
    let console = t.console;
    assert!(t.set_mode(ViewMode::Transposed));
    let id = ResultId(1 << 50);
    let schema2 = schema_other();
    let budget = MemoryBudget::new(u64::MAX, u64::MAX);
    let (store, mut writer) = ResultStore::new(id, schema2.clone(), budget, 1);
    writer.append(Chunk::from_values(
        &schema2,
        vec![
            vec![Value::Text("pw-hash-1".into()), Value::Text("alice".into())],
            vec![Value::Text("pw-hash-2".into()), Value::Text("bob".into())],
        ],
    ));
    writer.set_state(Completeness::Streaming);
    let source2 = StoreSource::new(store, Arc::new(Asks::default()), FetchPolicy::default());
    let _ = t.show(source2);
    t.redraw();
    t.redraw();
    let read = |t: &Tile| {
        t.vcx.cx.read(|cx| {
            let src = views::transposed_source(console, cx).expect("a transposed source");
            let frame = hacivat_ui_grid::GridDataSource::begin_frame(&*src, cx);
            let names: Vec<String> = (0..src.table_cols().len() as u64)
                .map(|r| frame.cell(ModelRow(r), ColId(0)).text.to_string())
                .collect();
            let value = frame.cell(ModelRow(0), ColId(1)).text.to_string();
            (views::mode(console, cx), names, value)
        })
    };
    let (mode, names, value) = read(&t);
    assert_eq!(mode, ViewMode::Transposed);
    assert_eq!(
        (names.as_slice(), value.as_str()),
        (
            ["secret".to_string(), "owner".to_string()].as_slice(),
            "pw-hash-1"
        ),
        "the transposed view shows the re-run's columns over its values"
    );
    // Rows streaming in after the rebuild do not rebuild it for each append (T38's rule).
    writer.append(Chunk::from_values(
        &schema2,
        vec![vec![
            Value::Text("pw-hash-3".into()),
            Value::Text("carol".into()),
        ]],
    ));
    t.redraw();
    let (_, names, _) = read(&t);
    assert_eq!(names, ["secret", "owner"]);
}

// ---- I2 -------------------------------------------------------------------------------------

/// I2: a go-to fetching toward a row of one result stops at a re-run and never asks the new
/// cursor for rows; the line says why.
#[gpui::test]
fn goto_fetching_stops_at_a_rerun(cx: &mut TestAppContext) {
    let t = tile(cx);
    let rows: Vec<Vec<Value>> = (0..100).map(|i| row(i, "x")).collect();
    let (source, _writer, asks) = live(rows, Completeness::Suspended(cursor()));
    let grid = t.show(source);
    let console = t.console;
    in_window(t.vcx, move |_, cx| {
        goto::open(console, cx);
        goto::set_text(console, "250", cx);
        goto::submit(console, cx);
    });
    t.redraw();
    assert!(matches!(
        t.vcx.cx.read(|cx| goto::state(console, cx)),
        Some(GotoState::Fetching { row: 250, .. })
    ));
    assert!(!asks.0.lock().unwrap().is_empty());
    let rows2: Vec<Vec<Value>> = (0..100).map(|i| row(1000 + i, "new")).collect();
    let (source2, mut writer2, asks2) = live(rows2, Completeness::Suspended(cursor()));
    let _ = t.show(source2);
    t.redraw();
    t.redraw();
    let state = t.vcx.cx.read(|cx| goto::state(console, cx));
    let asked2 = asks2.0.lock().unwrap().clone();
    writer2.append(Chunk::from_values(
        &schema(),
        (100..300).map(|i| row(1000 + i, "new")).collect(),
    ));
    t.redraw();
    t.redraw();
    let focus = t.focus(&grid);
    assert!(
        asked2.is_empty() && !matches!(state, Some(GotoState::Fetching { .. })),
        "after a re-run the old go-to keeps fetching the NEW cursor: state {state:?}, \
         new cursor asked {asked2:?}, focus then {focus:?}"
    );
    assert_ne!(
        focus,
        Some(cell(249, 0)),
        "the old go-to landed in the new rows"
    );
    match t.vcx.cx.read(|cx| goto::line(console, cx)) {
        Some(GotoLine::Error(e)) => assert_eq!(
            plain(&e),
            "Fetching stopped: the result was run again.",
            "the line says why"
        ),
        other => panic!("the line says why the fetching stopped: {other:?}"),
    }
}

/// I2: the > 1,000,000-row question asked for one result is never answered for a re-run's.
#[gpui::test]
fn goto_question_does_not_carry_over_to_a_rerun(cx: &mut TestAppContext) {
    let t = tile(cx);
    let rows: Vec<Vec<Value>> = (0..100).map(|i| row(i, "x")).collect();
    let (source, _writer, asks) = live(rows, Completeness::Suspended(cursor()));
    let _grid = t.show(source);
    let console = t.console;
    in_window(t.vcx, move |_, cx| {
        goto::open(console, cx);
        goto::set_text(console, "2000000", cx);
        goto::submit(console, cx);
    });
    t.redraw();
    assert!(matches!(
        t.vcx.cx.read(|cx| goto::state(console, cx)),
        Some(GotoState::Ask { row: 2_000_000, .. })
    ));
    assert!(
        asks.0.lock().unwrap().is_empty(),
        "nothing fetched before the answer"
    );
    let rows2: Vec<Vec<Value>> = (0..100).map(|i| row(1000 + i, "new")).collect();
    let (source2, _writer2, asks2) = live(rows2, Completeness::Suspended(cursor()));
    let _ = t.show(source2);
    t.redraw();
    // Fetch and go, answered after the re-run: it fetches nothing of the new result.
    in_window(t.vcx, move |_, cx| goto::fetch_and_go(console, cx));
    t.redraw();
    let state = t.vcx.cx.read(|cx| goto::state(console, cx));
    assert!(
        !matches!(
            state,
            Some(GotoState::Ask { .. } | GotoState::Fetching { .. })
        ),
        "the question carried over to the re-run: {state:?}"
    );
    assert!(
        asks2.0.lock().unwrap().is_empty(),
        "the new cursor was fetched without asking: {:?}",
        asks2.0.lock().unwrap()
    );
}

// ---- I3 -------------------------------------------------------------------------------------

/// I3: find and Filter rows stay off a tab shown Transposed: switching to it closes the bar, and
/// its hidden table grid is never scanned or filtered.
#[gpui::test]
fn find_closes_on_a_tab_shown_transposed(cx: &mut TestAppContext) {
    let t = tile(cx);
    let (a, _wa, _) = live(
        vec![row(1, "an"), row(2, "b"), row(3, "an")],
        Completeness::Complete,
    );
    let (b, _wb, _) = live(
        vec![row(4, "an"), row(5, "c"), row(6, "an"), row(7, "d")],
        Completeness::Complete,
    );
    let console = t.console;
    t.show_two(a, b);
    assert!(t.set_mode(ViewMode::Transposed));
    t.activate(0);
    t.find("an");
    t.flip(Toggle::Filter);
    let grid_a = t.grid(0);
    assert!(t.filtered(&grid_a));
    t.activate(1);
    let grid_b = t.grid(1);
    let (mode, open, columns) = t.vcx.cx.read(|cx| {
        (
            views::mode(console, cx),
            find::is_open(console, cx),
            views::transposed_source(console, cx).map(|s| s.table_rows().len()),
        )
    });
    assert_eq!(mode, ViewMode::Transposed);
    assert!(!open, "the find bar stays open on a Transposed tab");
    assert!(!t.filtered(&grid_b), "the hidden table grid was filtered");
    assert_eq!(columns, Some(4), "the Transposed view shows every row");
    assert!(!t.filtered(&grid_a), "the left tab keeps no filter");
}

/// I3: the same for a tab shown as text.
#[gpui::test]
fn find_closes_on_a_tab_shown_as_text(cx: &mut TestAppContext) {
    let t = tile(cx);
    let (a, _wa, _) = live(vec![row(1, "an"), row(2, "b")], Completeness::Complete);
    let (b, _wb, _) = live(
        vec![row(4, "an"), row(5, "c"), row(6, "an")],
        Completeness::Complete,
    );
    let console = t.console;
    t.show_two(a, b);
    assert!(t.set_mode(ViewMode::Text));
    t.activate(0);
    t.find("an");
    t.flip(Toggle::Filter);
    t.activate(1);
    let grid_b = t.grid(1);
    assert_eq!(t.vcx.cx.read(|cx| views::mode(console, cx)), ViewMode::Text);
    assert!(!t.vcx.cx.read(|cx| find::is_open(console, cx)));
    assert!(!t.filtered(&grid_b));
    // The text shows every row (the header's three lines, three rows, the closing rule).
    let lines = t.vcx.cx.read(|cx| views::text_lines(console, cx));
    assert_eq!(lines.iter().filter(|l| l.contains("an")).count(), 2);
    assert!(lines.iter().any(|l| l.contains(" c ")), "{lines:?}");
}

/// I3 (go to): the popover closes on a tab shown Transposed and does not act on its hidden
/// table grid.
#[gpui::test]
fn goto_is_off_on_a_tab_shown_transposed(cx: &mut TestAppContext) {
    let t = tile(cx);
    let (a, _wa, _) = live(vec![row(1, "an"), row(2, "b")], Completeness::Complete);
    let (b, _wb, _) = live(
        vec![row(4, "an"), row(5, "c"), row(6, "an")],
        Completeness::Complete,
    );
    let console = t.console;
    t.show_two(a, b);
    assert!(t.set_mode(ViewMode::Transposed));
    t.activate(0);
    in_window(t.vcx, move |_, cx| goto::open(console, cx));
    t.redraw();
    assert!(t.vcx.cx.read(|cx| goto::is_open(console, cx)));
    t.activate(1);
    assert!(
        !t.vcx.cx.read(|cx| goto::is_open(console, cx)),
        "the popover stays open on a Transposed tab"
    );
    in_window(t.vcx, move |_, cx| {
        goto::set_text(console, "3", cx);
        goto::submit(console, cx);
    });
    t.redraw();
    let grid_b = t.grid(1);
    assert_eq!(
        t.vcx.cx.read(|cx| views::mode(console, cx)),
        ViewMode::Transposed
    );
    let focus_b = t.focus(&grid_b);
    assert_ne!(
        focus_b,
        Some(cell(2, 0)),
        "go to acted on the hidden table grid"
    );
}

// ---- M2 -------------------------------------------------------------------------------------

/// M2: a filter composed in a new sort's order keeps the matching rows fetched while the sort
/// ran (after the sorted rows, in the source order, as the sort's unsorted tail shows them).
#[test]
fn a_filter_under_a_new_sort_keeps_the_rows_fetched_since() {
    // The sort saw 4 rows (descending); the filter has scanned 8 and kept 1, 3, 5 and 6.
    let base = ViewMap::permuted(Arc::from([3u64, 2, 1, 0]), 4, 4);
    let map = find::filter_in_order(&[1, 3, 5, 6], 8, &base);
    let shown: Vec<u64> = (0..map.len())
        .map(|v| map.view_to_model(ViewRow(v)).unwrap().0)
        .collect();
    assert_eq!(shown, [3, 1, 5, 6]);
    assert!(matches!(map, ViewMap::Filtered { .. }));
    // A base that already covers every fetched row is read as it is.
    let base = ViewMap::permuted(Arc::from([2u64, 0, 1]), 3, 3);
    let map = find::filter_in_order(&[0, 2], 3, &base);
    let shown: Vec<u64> = (0..map.len())
        .map(|v| map.view_to_model(ViewRow(v)).unwrap().0)
        .collect();
    assert_eq!(shown, [2, 0]);
}

// ---- M3 -------------------------------------------------------------------------------------

/// M3: the value viewer's "row N" is where the table shows the row now: a sort while the viewer
/// is open moves it.
#[gpui::test]
fn value_viewer_row_follows_a_sort(cx: &mut TestAppContext) {
    let t = tile(cx);
    let (source, _w, _) = live(
        vec![row(1, "a"), row(2, "b"), row(3, "c")],
        Completeness::Complete,
    );
    let grid = t.show(source);
    let console = t.console;
    in_window(t.vcx, move |_, cx| {
        value_viewer::open(console, cell(0, 1), cx)
    });
    t.redraw();
    let subtitle = |t: &Tile| {
        plain(
            &t.vcx
                .cx
                .read(|cx| value_viewer::view(console, cx))
                .expect("the viewer is open")
                .subtitle,
        )
    };
    assert!(subtitle(&t).ends_with("row 1"), "{}", subtitle(&t));
    in_window(t.vcx, |_, cx| {
        grid.update(cx, |g, cx| g.request_sort(SortSpec::by(ColId(0), true), cx))
    });
    t.redraw();
    t.redraw();
    assert!(t.vcx.cx.read(|cx| value_viewer::is_open(console, cx)));
    assert!(subtitle(&t).ends_with("row 3"), "{}", subtitle(&t));
}

// ---- M4 -------------------------------------------------------------------------------------

/// M4: in the Text view only Copy of the grid commands does anything; the rest are disabled
/// (the palette does not list them, their keys pass through).
#[gpui::test]
fn text_view_disables_the_grid_commands_it_ignores(cx: &mut TestAppContext) {
    let t = tile(cx);
    let (source, _w, _) = live(vec![row(1, "a"), row(2, "b")], Completeness::Complete);
    let _grid = t.show(source);
    let inert = [
        "grid.freeze",
        "grid.auto_fit",
        "grid.select_row",
        "grid.move_down",
        "grid.grow_selection",
        "grid.open_value",
    ];
    for id in inert {
        assert!(t.command_enabled(id), "{id} in the table");
    }
    assert!(t.set_mode(ViewMode::Text));
    for id in inert {
        assert!(!t.command_enabled(id), "{id} in the Text view");
    }
    assert!(t.command_enabled("grid.copy"));
    assert!(t.command_enabled("grid.copy_with_headers"));
    // The Transposed view's grid takes them.
    assert!(t.set_mode(ViewMode::Transposed));
    for id in inert {
        assert!(t.command_enabled(id), "{id} in the Transposed view");
    }
}
