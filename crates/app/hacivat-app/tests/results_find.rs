//! M3c Task 37 (HV-GRID-09, -14; NOTES surfaces 19 and 20): the grid find bar and go to row
//! over a result grid.
//!
//! - The find reuses Task 33's matcher (`console::find_bar::build_matcher`) over the **display
//!   strings** of the visible columns (what the user sees: grouped numbers, never the raw
//!   value); NULL never matches; a cut value marks the count "may be incomplete". The scan runs
//!   on the background executor in batches of 1,000 rows from the focus cell, a new query
//!   dropping the old one.
//! - Filter rows builds `ViewMap::Filtered` (the grid scrolls over the kept rows, numbered
//!   1…n), with a note row; a sort under a filter keeps the filter, and Clear sort restores it
//!   (Task 36 review m6).
//! - Go to row reads the format locale's grouping and fetches past the loaded rows of an open
//!   cursor, asking first above a million rows.
//!
//! The consoles are registered straight into the registry over result-store fixtures
//! (R-M3-15), as in `results_sort.rs`.

#![cfg_attr(not(unix), allow(dead_code, unused_imports, unused_variables))]

mod support;

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use gpui::{AppContext as _, Entity, Focusable as _, TestAppContext, VisualTestContext};
use hacivat_app::console::find_bar::FindOpts;
use hacivat_app::console::{ConsoleId, ConsoleState, Consoles};
use hacivat_app::results::adapter::ResultDisplay;
use hacivat_app::results::find::{self, Toggle};
use hacivat_app::results::goto::{self, GotoState};
use hacivat_app::results::view::hosts;
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
use hacivat_values::{
    Chunk, ColumnMeta, LenUnit, LogicalType, NativeType, ResultSchema, TruncatedVal, Value,
};
use support::*;

// ---- fixtures -------------------------------------------------------------------------------

/// `qty` (int8; not an ID column, so it is grouped) and `name` (text), on PostgreSQL.
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
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1 << 41);
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

fn tile(cx: &mut TestAppContext, region: &str) -> Tile {
    let vcx = shell_with_connections(cx, "en");
    // An active window: the focused input is published only then (`FocusedInput`).
    in_window(vcx, |window, _| window.activate_window());
    let profile = add_profile(vcx, test_profile("orders-dev", None, "10.0.0.5"));
    let ws = workspace(vcx);
    let region = region.to_string();
    let (console, results) = in_window(vcx, |window, cx| {
        let tab = ws
            .update(cx, |ws, cx| ws.open_console_tab(Some(profile), window, cx))
            .expect("a console tab opens");
        let log = SessionLog::new();
        let results = cx.new(|_| ConsoleResults::new(log.clone()));
        results.update(cx, |r, cx| {
            r.set_display(
                ResultDisplay::new(&region, DisplayZone::utc(), DbmsKind::Postgres),
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

    fn status(&self) -> find::FindView {
        self.vcx
            .cx
            .read(|cx| find::status(self.console, cx))
            .expect("the find bar is open")
    }

    fn flip(&self, t: Toggle) {
        let console = self.console;
        in_window(self.vcx, move |_, cx| find::flip(console, t, cx));
        self.redraw();
    }

    fn map(&self, grid: &Entity<GridView>) -> ViewMap {
        self.vcx.cx.read(|cx| grid.read(cx).view_map().clone())
    }

    /// The model rows the grid shows, top to bottom.
    fn shown(&self, grid: &Entity<GridView>) -> Vec<u64> {
        let map = self.map(grid);
        (0..map.len())
            .map(|v| map.view_to_model(ViewRow(v)).unwrap().0)
            .collect()
    }

    fn focus(&self, grid: &Entity<GridView>) -> Option<ModelCell> {
        self.vcx.cx.read(|cx| grid.read(cx).selection().focus())
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

// ---- the plan's tests ---------------------------------------------------------------------

/// A value cut at the store's cell cap is searched only up to the cut: the count says "may be
/// incomplete", with how many values were cut in its tooltip.
#[gpui::test]
fn find_over_truncated_marks_incomplete(cx: &mut TestAppContext) {
    let t = tile(cx, "en-US");
    let cut = Value::Truncated(Box::new(TruncatedVal {
        prefix: Value::Text("a long note about apples".into()),
        total_len: Some(5 << 20),
        unit: LenUnit::Bytes,
    }));
    let (source, _w, _) = live(
        vec![
            row(1, "apple pie"),
            vec![Value::Int(2), cut],
            row(3, "pear"),
        ],
        Completeness::Complete,
    );
    let _grid = t.show(source);
    t.find("apple");
    let s = t.status();
    assert_eq!(s.total, Some(2));
    assert_eq!(s.incomplete, 1);
    let more = t.vcx.cx.read(|cx| find::incomplete_text(t.console, cx));
    assert_eq!(more.as_deref(), Some("may be incomplete"));
    // Drawn after the count, with the cut's tooltip.
    assert_eq!(
        text_of(t.vcx, "grid-find-incomplete-text").as_deref(),
        Some("may be incomplete")
    );
    // Without a cut value, nothing is said.
    t.find("pear");
    assert_eq!(t.status().incomplete, 1, "the cut value is still scanned");
    let (source, _w, _) = live(vec![row(1, "pear")], Completeness::Complete);
    let _grid = t.show(source);
    t.find("pear");
    assert_eq!(t.status().incomplete, 0);
    assert_eq!(
        t.vcx.cx.read(|cx| find::incomplete_text(t.console, cx)),
        None
    );
}

/// NULL never matches, not even a query for "NULL"; the scan reads the display strings (a
/// grouped number matches as shown, never as its raw digits).
#[gpui::test]
fn null_never_matches(cx: &mut TestAppContext) {
    let t = tile(cx, "en-US");
    let (source, _w, _) = live(
        vec![
            vec![Value::Int(1234), Value::Null],
            vec![Value::Int(7), Value::Text("NULL".into())],
            vec![Value::Null, Value::Text("null and void".into())],
        ],
        Completeness::Complete,
    );
    let grid = t.show(source);
    t.find("NULL");
    // Only the text cells that say it (case-insensitive): rows 1 and 2's names.
    assert_eq!(t.status().total, Some(2));
    let hits = t.vcx.cx.read(|cx| find::hits(t.console, cx));
    assert_eq!(hits, [cell(1, 1), cell(2, 1)]);
    // What the user sees: `1,234` matches, the raw `1234` does not.
    t.find("1,234");
    assert_eq!(t.status().total, Some(1));
    t.find("1234");
    assert_eq!(t.status().total, Some(0));
    // The marks follow the matches: the cell's text range is marked.
    t.find("1,2");
    let marks = t
        .vcx
        .cx
        .read(|cx| grid.read(cx).highlights().map(|h| h(ModelRow(0), ColId(0))));
    let marks = marks.flatten().expect("row 0's id is marked");
    assert_eq!(marks.ranges.as_slice(), std::slice::from_ref(&(0..3)));
}

/// Filter rows shows only the rows with a match, through `ViewMap::Filtered` (the grid scrolls
/// over them, numbered 1…n), with the note row; Show all rows turns it off.
#[gpui::test]
fn filter_rows_builds_a_filtered_map(cx: &mut TestAppContext) {
    let t = tile(cx, "en-US");
    let rows = vec![
        row(1, "ankara"),
        row(2, "izmir"),
        row(3, "antalya"),
        row(4, "bursa"),
        row(5, "adana"),
    ];
    let (source, _w, _) = live(rows, Completeness::Complete);
    let grid = t.show(source);
    t.find("an");
    t.flip(Toggle::Filter);
    let map = t.map(&grid);
    assert!(matches!(map, ViewMap::Filtered { .. }), "{map:?}");
    assert_eq!(t.shown(&grid), [0, 2, 4]);
    assert_eq!(t.status().filtered, Some(3));
    let note = text_of(t.vcx, "results-filter-note").map(|n| plain(&n));
    assert_eq!(
        note.as_deref(),
        Some(
            "Showing the 3 rows that match, of the 5 fetched. Rows not fetched yet aren't filtered."
        )
    );
    // A new query filters anew.
    t.find("izmir");
    assert_eq!(t.shown(&grid), [1]);
    // Show all rows: the source order, the note gone, the bar still open.
    click(t.vcx, "button-results-filter-show-all");
    t.redraw();
    assert!(t.map(&grid).is_identity());
    assert_eq!(text_of(t.vcx, "results-filter-note"), None);
    assert!(t.vcx.cx.read(|cx| find::is_open(t.console, cx)));
}

/// Past the fetched rows of an open cursor, go to row fetches; above a million rows it asks
/// first (Fetch and go, or Cancel), and Stop fetching ends the asking with the cursor open.
/// Below that it fetches at once, and the grid goes to the row when it arrives.
#[gpui::test]
fn goto_past_loaded_fetches_with_a_confirmation_above_1m(cx: &mut TestAppContext) {
    let t = tile(cx, "tr-TR");
    let rows: Vec<Vec<Value>> = (0..100).map(|i| row(i, "x")).collect();
    let (source, mut writer, asks) = live(rows, Completeness::Suspended(cursor()));
    let grid = t.show(source);
    let console = t.console;
    in_window(t.vcx, move |_, cx| {
        goto::open(console, cx);
        // `1.500.000` is row 1,500,000 in Turkish.
        goto::set_text(console, "1.500.000", cx);
        goto::submit(console, cx);
    });
    t.redraw();
    assert_eq!(
        t.vcx.cx.read(|cx| goto::state(console, cx)),
        Some(GotoState::Ask {
            row: 1_500_000,
            more: 1_499_900
        })
    );
    assert!(
        asks.0.lock().unwrap().is_empty(),
        "nothing fetched before the answer"
    );
    // Fetch and go: fetching, in steps.
    in_window(t.vcx, move |_, cx| goto::fetch_and_go(console, cx));
    t.redraw();
    assert!(matches!(
        t.vcx.cx.read(|cx| goto::state(console, cx)),
        Some(GotoState::Fetching { row: 1_500_000, .. })
    ));
    assert!(!asks.0.lock().unwrap().is_empty(), "a fetch was asked");
    // Stop fetching: the cursor stays open with what was fetched; nothing more is asked.
    in_window(t.vcx, move |_, cx| goto::stop(console, cx));
    t.redraw();
    assert_eq!(
        t.vcx.cx.read(|cx| goto::state(console, cx)),
        Some(GotoState::Idle)
    );
    let asked = asks.0.lock().unwrap().len();
    writer.append(Chunk::from_values(
        &schema(),
        (100..150).map(|i| row(i, "y")).collect(),
    ));
    t.redraw();
    assert_eq!(asks.0.lock().unwrap().len(), asked);
    let still_open = t.vcx.cx.read(|cx| {
        let snapshot = t.results.read(cx).tab(0).unwrap().adapter().snapshot();
        matches!(snapshot.completeness(), Completeness::Suspended(_))
    });
    assert!(still_open, "the cursor stays open");

    // Below a million: no question; the row arrives and the grid goes to it.
    in_window(t.vcx, move |_, cx| {
        goto::open(console, cx);
        goto::set_text(console, "250:name", cx);
        goto::submit(console, cx);
    });
    t.redraw();
    assert!(matches!(
        t.vcx.cx.read(|cx| goto::state(console, cx)),
        Some(GotoState::Fetching { row: 250, .. })
    ));
    writer.append(Chunk::from_values(
        &schema(),
        (150..300).map(|i| row(i, "z")).collect(),
    ));
    t.redraw();
    t.redraw();
    assert_eq!(t.focus(&grid), Some(cell(249, 1)));
    assert!(!t.vcx.cx.read(|cx| goto::is_open(console, cx)));
}

// ---- more ---------------------------------------------------------------------------------

/// The scan runs on the background executor: right after a query nothing is counted or
/// marked; the first match at or after where find started becomes the focus once the count
/// lands (one selection change); steps wrap.
#[gpui::test]
fn the_scan_runs_off_the_ui_thread_and_steps_wrap(cx: &mut TestAppContext) {
    let t = tile(cx, "en-US");
    let rows = vec![
        row(1, "kiwi"),
        row(2, "fig"),
        row(3, "kiwi"),
        row(4, "plum"),
        row(5, "kiwi"),
    ];
    let (source, _w, _) = live(rows, Completeness::Complete);
    let grid = t.show(source);
    let console = t.console;
    // Find starts from the focus cell: row 1 (model), column `name`.
    in_window(t.vcx, |_, cx| {
        grid.update(cx, |g, cx| g.focus_cell(cell(1, 1), cx))
    });
    t.redraw();
    let g = grid.clone();
    let (total, marked) = in_window(t.vcx, move |_, cx| {
        let grid = g;
        find::open(console, cx);
        find::set_query(console, "kiwi", cx);
        let s = find::status(console, cx).unwrap();
        let marked = grid
            .read(cx)
            .highlights()
            .and_then(|h| h(ModelRow(0), ColId(1)))
            .is_some();
        (s.total, marked)
    });
    assert_eq!(total, None, "nothing is counted on the UI thread");
    assert!(!marked);
    t.redraw();
    let s = t.status();
    assert_eq!(s.total, Some(3));
    // The first match at or after the focus: row 2 (view 2), the 2nd of 3.
    assert_eq!(t.focus(&grid), Some(cell(2, 1)));
    assert_eq!(s.current, Some(2));
    in_window(t.vcx, move |_, cx| find::step(console, false, cx));
    t.redraw();
    assert_eq!(t.focus(&grid), Some(cell(4, 1)));
    // Wraps to the first.
    in_window(t.vcx, move |_, cx| find::step(console, false, cx));
    t.redraw();
    assert_eq!(t.focus(&grid), Some(cell(0, 1)));
    assert_eq!(t.status().current, Some(1));
    in_window(t.vcx, move |_, cx| find::step(console, true, cx));
    t.redraw();
    assert_eq!(t.focus(&grid), Some(cell(4, 1)));
    let count = t.vcx.cx.read(|cx| find::count_text(console, cx)).unwrap();
    assert_eq!(plain(&count), "3 of 3");
    // An invalid regex searches nothing and says so in ink.
    t.flip(Toggle::Regex);
    t.find("(kiwi");
    assert!(t.status().invalid);
    assert_eq!(t.status().total, None);
}

/// Task 36 review m6: under a filter a sort orders the kept rows and keeps the filter; Clear
/// sort restores the filter in the source order; turning the filter off shows the sort.
#[gpui::test]
fn a_sort_under_a_filter_keeps_the_filter(cx: &mut TestAppContext) {
    let t = tile(cx, "en-US");
    let rows = vec![
        row(1, "cab"),
        row(2, "zzz"),
        row(3, "abc"),
        row(4, "bca"),
        row(5, "yyy"),
    ];
    let (source, _w, _) = live(rows, Completeness::Complete);
    let grid = t.show(source);
    t.find("a");
    t.flip(Toggle::Filter);
    assert_eq!(t.shown(&grid), [0, 2, 3]);
    // Sort by name ascending: the kept rows in order, still filtered.
    in_window(t.vcx, |_, cx| {
        grid.update(cx, |g, cx| {
            g.request_sort(SortSpec::by(ColId(1), false), cx)
        })
    });
    t.redraw();
    t.redraw();
    assert!(matches!(t.map(&grid), ViewMap::Filtered { .. }));
    assert_eq!(t.shown(&grid), [2, 3, 0]);
    // Clear sort: the filter in the source order.
    in_window(t.vcx, |_, cx| {
        grid.update(cx, |g, cx| g.request_sort(SortSpec::default(), cx))
    });
    t.redraw();
    assert_eq!(t.shown(&grid), [0, 2, 3]);
    // Sort again, then turn the filter off: every row in the sorted order.
    in_window(t.vcx, |_, cx| {
        grid.update(cx, |g, cx| g.request_sort(SortSpec::by(ColId(1), true), cx))
    });
    t.redraw();
    t.redraw();
    assert_eq!(t.shown(&grid), [0, 3, 2]);
    t.flip(Toggle::Filter);
    assert!(matches!(t.map(&grid), ViewMap::Permuted { .. }));
    assert_eq!(t.shown(&grid), [1, 4, 0, 3, 2]);
}

/// Select all occurrences selects every matching cell as its own range, at most 10,000; past
/// the cap the rest is said.
#[gpui::test]
fn select_all_occurrences_caps_at_10000(cx: &mut TestAppContext) {
    let t = tile(cx, "en-US");
    let rows: Vec<Vec<Value>> = (0..10_500).map(|i| row(i, "match")).collect();
    let (source, _w, _) = live(rows, Completeness::Complete);
    let grid = t.show(source);
    t.find("match");
    assert_eq!(t.status().total, Some(10_500));
    let console = t.console;
    in_window(t.vcx, move |_, cx| find::select_all(console, cx));
    t.redraw();
    let ranges = t.vcx.cx.read(|cx| grid.read(cx).selection().ranges());
    assert_eq!(ranges, 10_000);
    let toast = toast_texts(t.vcx);
    assert!(
        toast
            .iter()
            .any(|s| plain(s) == "Selected the first 10,000 of 10,500 matches"),
        "{toast:?}"
    );
}

/// ⌘F (Ctrl+F) in a focused grid opens its find bar, and the field's keys stay the field's
/// (an arrow moves the field's caret, not the grid's focus); Escape closes it and gives the
/// focus back to the grid.
#[gpui::test]
fn primary_f_in_a_grid_opens_its_find_bar(cx: &mut TestAppContext) {
    let t = tile(cx, "en-US");
    let (source, _w, _) = live(
        vec![row(1, "a"), row(2, "b"), row(3, "c")],
        Completeness::Complete,
    );
    let grid = t.show(source);
    in_window(t.vcx, |window, cx| {
        grid.update(cx, |g, cx| g.focus_cell(cell(0, 0), cx));
        let handle = grid.read(cx).focus_handle(cx);
        window.focus(&handle, cx);
    });
    t.redraw();
    let primary = if cfg!(target_os = "macos") {
        "cmd-f"
    } else {
        "ctrl-f"
    };
    t.vcx.simulate_keystrokes(primary);
    t.redraw();
    assert!(t.vcx.cx.read(|cx| find::is_open(t.console, cx)));
    t.vcx.simulate_input("b");
    t.redraw();
    t.vcx.simulate_keystrokes("down");
    t.redraw();
    assert_eq!(
        t.vcx.cx.read(|cx| find::query(t.console, cx)).as_deref(),
        Some("b")
    );
    // The match is the focus; the arrow in the field did not move it.
    assert_eq!(t.focus(&grid), Some(cell(1, 1)));
    assert!(
        t.vcx.cx.read(find::field_focused),
        "the field has the focus: the grid's other commands are off"
    );
    t.vcx.simulate_keystrokes("escape");
    t.redraw();
    assert!(!t.vcx.cx.read(|cx| find::is_open(t.console, cx)));
    let focused = in_window(t.vcx, |window, cx| {
        grid.read(cx).focus_handle(cx).is_focused(window)
    });
    assert!(focused, "the focus is back on the grid");
    // Closing clears the marks.
    assert!(t.vcx.cx.read(|cx| grid.read(cx).highlights().is_none()));
}

/// The perf line (HACIVAT_PERF=1, release): the find scan over 1,048,576 rows × 2 columns (the
/// job the background executor runs) and the filter's map over its matching rows.
#[test]
#[ignore = "perf"]
fn find_scan_and_filter_of_a_million_rows_perf() {
    if std::env::var("HACIVAT_PERF").as_deref() != Ok("1") {
        return;
    }
    const N: i64 = 1 << 20;
    let words = [
        "İzmir", "ılık", "Irmak", "istek", "çam", "şeker", "Ömer", "üzüm",
    ];
    let rows: Vec<Vec<Value>> = (0..N)
        .map(|i| {
            vec![
                Value::Int((i * 7_919) % N),
                Value::Text(
                    format!("{}{}", words[(i as usize * 7) % words.len()], i % 1000).into(),
                ),
            ]
        })
        .collect();
    let store = ResultStore::from_rows(schema(), rows, Completeness::Complete);
    let display = ResultDisplay::new("tr-TR", DisplayZone::utc(), DbmsKind::Postgres);
    let opts = FindOpts {
        tolerant: true,
        ..FindOpts::default()
    };
    let started = Instant::now();
    let summary = find::scan_snapshot(store.snapshot(), &display, "izmir", opts);
    let scan_ms = started.elapsed().as_millis();
    let started = Instant::now();
    let map = find::filtered_map(&summary.rows, N as u64);
    let filter_ms = started.elapsed().as_millis();
    assert_eq!(map.len(), summary.rows.len() as u64);
    assert!(summary.total > 0);
    eprintln!(
        "PERF find over 1,048,576 rows x 2 columns (tr, Turkish-tolerant): scan {scan_ms} ms, {} matches; filter map {filter_ms} ms",
        summary.total
    );
    // Off the UI thread; budgets for the background job.
    assert!(scan_ms <= 3_000, "the scan took {scan_ms} ms");
    assert!(filter_ms <= 200, "the filter map took {filter_ms} ms");
}
