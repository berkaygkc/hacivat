//! M3c Task 36 (HV-GRID-07, R-M3-70; NOTES surface 18, M3c-20): the local sort of a result
//! grid. The order is built off the UI thread and published as `ViewMap::Permuted`; a stale
//! sort never replaces a newer one; a partial result says it was sorted locally; rows that
//! arrive later wait in the unsorted tail; and **copy follows the order shown** (ledger 253
//! I5): a copy translates the selection's view rows through the map.
//!
//! The consoles are registered straight into the registry over result-store fixtures
//! (R-M3-15), as in `results_view.rs`.

#![cfg_attr(not(unix), allow(dead_code, unused_imports, unused_variables))]

mod support;

use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::{AppContext as _, Entity, Focusable as _, TestAppContext, VisualTestContext};
use hacivat_app::console::{ConsoleId, ConsoleState, Consoles};
use hacivat_app::results::ConsoleResults;
use hacivat_app::results::sort::{self, NoteKind};
use hacivat_app::results::view::hosts;
use hacivat_exec::{ResultTab, SessionLog, TabKey};
use hacivat_results::{
    CloseReason, Completeness, CursorInfo, FetchControl, FetchPolicy, MemoryBudget, ResultId,
    ResultSource, ResultStore, ResultWriter, StoreSource,
};
use hacivat_session::session::{ClientId, RunId, UnitId};
use hacivat_ui_grid::model::{Dir, SortKey, SortSpec, ViewMap};
use hacivat_ui_grid::{ColId, GridCommand, GridView};
use hacivat_values::{Chunk, ColumnMeta, LogicalType, NativeType, ResultSchema, Value};
use support::*;

// ---- fixtures -------------------------------------------------------------------------------

/// `id` (int8) and `name` (text), on PostgreSQL.
fn schema() -> Arc<ResultSchema> {
    Arc::new(ResultSchema {
        columns: Arc::from([
            ColumnMeta {
                name: "id".into(),
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

struct NoControl;

impl FetchControl for NoControl {
    fn fetch_more(&self, _: ResultId, _: u64) {}
    fn close(&self, _: ResultId, _: CloseReason) {}
    fn fetch_all(&self, _: ResultId) {}
}

fn source(store: Arc<ResultStore>) -> Arc<dyn ResultSource> {
    StoreSource::new(store, Arc::new(NoControl), FetchPolicy::default())
}

/// A live store holding `rows`, in state `state`, and its writer (rows can follow).
fn live_store(rows: Vec<Vec<Value>>, state: Completeness) -> (Arc<ResultStore>, ResultWriter) {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1 << 40);
    let id = ResultId(NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed));
    let schema = schema();
    let budget = MemoryBudget::new(u64::MAX, u64::MAX);
    let (store, mut writer) = ResultStore::new(id, schema.clone(), budget, 1);
    if !rows.is_empty() {
        writer.append(Chunk::from_values(&schema, rows));
    }
    writer.set_state(state);
    (store, writer)
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

/// A shell whose focused pane shows a console with empty results (the text registry on).
struct Tile {
    vcx: &'static mut VisualTestContext,
    console: ConsoleId,
    results: Entity<ConsoleResults>,
}

fn tile(cx: &mut TestAppContext) -> Tile {
    let vcx = shell_with_connections(cx, "en");
    let profile = add_profile(vcx, test_profile("orders-dev", None, "10.0.0.5"));
    let ws = workspace(vcx);
    let (console, results) = in_window(vcx, |window, cx| {
        let tab = ws
            .update(cx, |ws, cx| ws.open_console_tab(Some(profile), window, cx))
            .expect("a console tab opens");
        let log = SessionLog::new();
        let results = cx.new(|_| ConsoleResults::new(log.clone()));
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
    fn show(&self, store: Arc<ResultStore>) -> Entity<GridView> {
        in_window(self.vcx, |_, cx| {
            self.results
                .update(cx, |r, cx| r.apply(&[tab(source(store))], cx))
        });
        self.redraw();
        self.grid()
    }

    fn grid(&self) -> Entity<GridView> {
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

    fn request(&self, grid: &Entity<GridView>, spec: SortSpec) {
        in_window(self.vcx, |_, cx| {
            grid.update(cx, |g, cx| g.request_sort(spec, cx))
        });
    }

    fn sort_by(&self, grid: &Entity<GridView>, col: u32, desc: bool) {
        self.request(grid, SortSpec::by(ColId(col), desc));
        self.redraw();
    }

    fn dispatch(&self, grid: &Entity<GridView>, cmd: GridCommand) {
        in_window(self.vcx, |window, cx| {
            grid.update(cx, |g, cx| g.dispatch(cmd, window, cx))
        });
        self.redraw();
    }

    fn focus(&self, grid: &Entity<GridView>) {
        in_window(self.vcx, |window, cx| {
            let handle = grid.read(cx).focus_handle(cx);
            window.focus(&handle, cx);
        });
        self.redraw();
    }

    fn map(&self, grid: &Entity<GridView>) -> ViewMap {
        self.vcx.cx.read(|cx| grid.read(cx).view_map().clone())
    }

    /// The model rows the grid shows, top to bottom.
    fn shown(&self, grid: &Entity<GridView>) -> Vec<u64> {
        let map = self.map(grid);
        (0..map.len())
            .map(|v| map.view_to_model(hacivat_ui_grid::ViewRow(v)).unwrap().0)
            .collect()
    }

    fn note(&self) -> Option<String> {
        text_of(self.vcx, "results-sort-note")
    }

    fn copy(&self, grid: &Entity<GridView>) -> String {
        in_window(self.vcx, |_, cx| {
            cx.write_to_clipboard(gpui::ClipboardItem::new_string("sentinel".to_string()))
        });
        self.dispatch(grid, GridCommand::Copy { headers: false });
        self.vcx
            .cx
            .read_from_clipboard()
            .and_then(|item| item.text())
            .unwrap_or_default()
    }
}

/// `text` without the Unicode isolation marks Fluent puts around its arguments.
fn plain(text: &str) -> String {
    text.chars()
        .filter(|c| !matches!(c, '\u{2068}' | '\u{2069}'))
        .collect()
}

// ---- the plan's tests ---------------------------------------------------------------------

/// A partial result (more rows on the server) sorted locally says so in a note under the
/// toolbar, with Clear sort; a complete one shows no note.
#[gpui::test]
fn partial_result_shows_sorted_locally(cx: &mut TestAppContext) {
    let t = tile(cx);
    let (store, _writer) = live_store(
        vec![row(3, "c"), row(1, "a"), row(2, "b")],
        Completeness::Suspended(cursor()),
    );
    let grid = t.show(store);
    assert_eq!(t.note(), None);
    t.sort_by(&grid, 0, false);
    assert_eq!(t.shown(&grid), [1, 2, 0]);
    assert_eq!(
        t.note().as_deref(),
        Some(
            "Sorted locally: the database might order differently. Only the 3 fetched rows are sorted."
        )
    );
    assert_eq!(
        t.vcx.cx.read(|cx| grid.read(cx).sort().clone()),
        SortSpec::by(ColId(0), false)
    );
    // Clear sort: back to the order the database sent, the note and the indicator gone.
    click(t.vcx, "button-results-sort-clear");
    t.redraw();
    assert!(t.map(&grid).is_identity());
    assert_eq!(t.note(), None);
    assert!(t.vcx.cx.read(|cx| grid.read(cx).sort().is_empty()));

    // A complete result sorts without a note.
    let (store, _writer) = live_store(
        vec![row(3, "c"), row(1, "a"), row(2, "b")],
        Completeness::Complete,
    );
    let grid = t.show(store);
    t.sort_by(&grid, 0, true);
    assert_eq!(t.shown(&grid), [0, 2, 1]);
    assert_eq!(t.note(), None);
}

/// Rows that arrive during or after a sort join the unsorted tail behind a dashed line: on a
/// partial result the note says both facts in one row (M3c-20), on a complete one "N new rows
/// at the end (not sorted)"; Re-sort sorts them in.
#[gpui::test]
fn new_rows_go_to_the_unsorted_tail(cx: &mut TestAppContext) {
    let t = tile(cx);
    let (store, mut writer) = live_store(
        vec![row(30, "c"), row(10, "a"), row(20, "b")],
        Completeness::Suspended(cursor()),
    );
    let grid = t.show(store);
    t.sort_by(&grid, 0, false);
    assert_eq!(t.shown(&grid), [1, 2, 0]);
    // Fetch more: two rows arrive after the sort.
    writer.append(Chunk::from_values(
        &schema(),
        vec![row(5, "e"), row(25, "d")],
    ));
    t.redraw();
    assert_eq!(t.shown(&grid), [1, 2, 0, 3, 4]);
    assert_eq!(t.vcx.cx.read(|cx| grid.read(cx).tail_boundary()), Some(3));
    assert!(
        t.vcx.cx.read(|cx| grid.read(cx).painted_tail_y()).is_some(),
        "the dashed boundary is drawn"
    );
    assert_eq!(
        t.note().as_deref(),
        Some(
            "Sorted locally: the database might order differently. Only the 3 fetched rows are \
             sorted; 2 new rows at the end are not."
        )
    );
    // The result completes: the tail stays, said alone.
    writer.set_state(Completeness::Complete);
    t.redraw();
    assert_eq!(
        t.note().as_deref(),
        Some("2 new rows at the end (not sorted)")
    );
    // Re-sort takes them in: no tail, no note.
    click(t.vcx, "button-results-sort-resort");
    t.redraw();
    assert_eq!(t.shown(&grid), [3, 1, 2, 4, 0]);
    assert_eq!(t.vcx.cx.read(|cx| grid.read(cx).tail_boundary()), None);
    assert_eq!(t.note(), None);
}

/// Ledger 253 I5: a copy after a sort holds exactly the rows the user sees, in the order
/// shown, through a sort and through a hand-shuffled map.
#[gpui::test]
fn copy_after_sort_copies_what_is_shown(cx: &mut TestAppContext) {
    let t = tile(cx);
    let (store, _writer) = live_store(
        vec![
            row(30, "c"),
            row(10, "a"),
            row(50, "e"),
            row(20, "b"),
            row(40, "d"),
        ],
        Completeness::Complete,
    );
    let grid = t.show(store);
    t.focus(&grid);
    t.sort_by(&grid, 0, false);
    // The first two rows shown, both columns.
    t.dispatch(&grid, GridCommand::FirstRow { extend: false });
    t.dispatch(&grid, GridCommand::Home { extend: false });
    t.dispatch(
        &grid,
        GridCommand::Move {
            dir: Dir::Down,
            extend: true,
        },
    );
    t.dispatch(&grid, GridCommand::End { extend: true });
    assert_eq!(t.copy(&grid), "10\ta\n20\tb\n");
    // Everything, in the order shown.
    t.dispatch(&grid, GridCommand::SelectAll);
    assert_eq!(t.copy(&grid), "10\ta\n20\tb\n30\tc\n40\td\n50\te\n");

    // A shuffled order (not a sort of any column): the copy follows it row for row.
    let order: Arc<[u64]> = Arc::from([3u64, 0, 4, 1, 2]);
    in_window(t.vcx, |_, cx| {
        grid.update(cx, |g, cx| {
            g.set_view_map(ViewMap::permuted(order.clone(), 5, 5), cx)
        })
    });
    t.redraw();
    assert_eq!(t.shown(&grid), [3, 0, 4, 1, 2]);
    t.dispatch(&grid, GridCommand::SelectAll);
    assert_eq!(t.copy(&grid), "20\tb\n30\tc\n40\td\n10\ta\n50\te\n");
    // View rows 1..3 (model rows 0 and 4).
    t.dispatch(&grid, GridCommand::FirstRow { extend: false });
    t.dispatch(&grid, GridCommand::Home { extend: false });
    t.dispatch(
        &grid,
        GridCommand::Move {
            dir: Dir::Down,
            extend: false,
        },
    );
    t.dispatch(
        &grid,
        GridCommand::Move {
            dir: Dir::Down,
            extend: true,
        },
    );
    assert_eq!(t.copy(&grid), "30\n40\n");
    // What the grid painted in those rows is what was copied.
    let shown = t.shown(&grid);
    assert_eq!((shown[1], shown[2]), (0, 4));
}

/// A sort of 1,048,576 rows runs off the UI thread (the request returns with the old order
/// shown and the job pending), can be cancelled before it lands (nothing is published), and a
/// later request lands with the whole order.
#[gpui::test]
fn sort_of_a_million_rows_is_cancellable_and_off_thread(cx: &mut TestAppContext) {
    const N: i64 = 1 << 20;
    let t = tile(cx);
    let rows: Vec<Vec<Value>> = (0..N)
        .map(|i| vec![Value::Int((i * 7_919) % N), Value::Text("x".into())])
        .collect();
    let (store, _writer) = live_store(rows, Completeness::Complete);
    let grid = t.show(store);
    // The request returns at once: the job runs on the background executor.
    t.request(&grid, SortSpec::by(ColId(0), false));
    assert!(t.map(&grid).is_identity(), "sorted on the UI thread");
    let status = t
        .vcx
        .cx
        .read(|cx| sort::status(&grid, cx))
        .expect("a sort is pending");
    assert_eq!(status.running, Some(N as u64));
    // Cancel before it lands: the old order stays, nothing is published later.
    in_window(t.vcx, |_, cx| sort::cancel(&grid, cx));
    t.redraw();
    assert!(t.map(&grid).is_identity());
    assert!(t.vcx.cx.read(|cx| grid.read(cx).sort().is_empty()));
    // Again, to the end: the whole order, the smallest value first.
    t.sort_by(&grid, 0, false);
    let map = t.map(&grid);
    let ViewMap::Permuted { order, .. } = &map else {
        panic!("not sorted: {map:?}");
    };
    assert_eq!(order.len(), N as usize);
    assert_eq!(order[0], 0);
    // Model row r holds r · 7919 mod N: the first rows shown hold 0, 1, 2, 3.
    for (v, row) in order.iter().take(4).enumerate() {
        assert_eq!((*row as i64 * 7_919) % N, v as i64);
    }
}

// ---- more cases ---------------------------------------------------------------------------

/// The generation guard: of two requests the later one lands, and a sort that finishes after
/// a re-run never puts its order on the new rows.
#[gpui::test]
fn a_stale_sort_never_replaces_a_newer_one(cx: &mut TestAppContext) {
    let t = tile(cx);
    let rows = vec![row(2, "b"), row(3, "c"), row(1, "a")];
    let (store, _w) = live_store(rows.clone(), Completeness::Complete);
    let grid = t.show(store);
    t.request(&grid, SortSpec::by(ColId(0), false));
    t.request(&grid, SortSpec::by(ColId(0), true));
    t.redraw();
    assert_eq!(t.shown(&grid), [1, 0, 2]);
    assert_eq!(
        t.vcx.cx.read(|cx| grid.read(cx).sort().clone()),
        SortSpec::by(ColId(0), true)
    );
    // A re-run arrives while a sort is pending: the new rows keep their order.
    t.request(&grid, SortSpec::by(ColId(0), false));
    let (store, _w) = live_store(rows, Completeness::Complete);
    in_window(t.vcx, |_, cx| {
        t.results
            .update(cx, |r, cx| r.apply(&[tab(source(store))], cx))
    });
    t.redraw();
    assert!(t.map(&grid).is_identity());
    assert!(t.vcx.cx.read(|cx| grid.read(cx).sort().is_empty()));
    assert_eq!(t.note(), None);
}

/// The header shows the indicator: an up arrow ascending, a down arrow descending, the level
/// numbers only with two or more keys (M3c-19); the tooltips name the level, the direction and
/// how text compares (the result names no collation: code point, said so).
#[gpui::test]
fn the_header_shows_the_sort_and_its_tooltip(cx: &mut TestAppContext) {
    let t = tile(cx);
    let (store, _w) = live_store(
        vec![row(2, "b"), row(1, "b"), row(3, "a")],
        Completeness::Complete,
    );
    let grid = t.show(store);
    t.sort_by(&grid, 1, false);
    let marks = t
        .vcx
        .cx
        .read(|cx| grid.read(cx).painted_sort_marks().to_vec());
    assert_eq!(marks.len(), 1);
    assert_eq!(
        (marks[0].col, marks[0].level, marks[0].desc),
        (ColId(1), 0, false)
    );
    assert_eq!(marks[0].number, None);
    let tips: Vec<String> = t.vcx.cx.read(|cx| {
        grid.read(cx)
            .sort_tips()
            .iter()
            .map(|f| plain(&f(cx)))
            .collect()
    });
    assert_eq!(
        tips,
        [
            "Sort level 1, ascending. The column's collation is not known here, so text compares by code point."
        ]
    );
    // Two levels: name ascending, then id descending.
    let mut spec = SortSpec::by(ColId(1), false);
    spec.keys.push(SortKey {
        col: ColId(0),
        desc: true,
    });
    t.request(&grid, spec.clone());
    t.redraw();
    assert_eq!(t.shown(&grid), [2, 0, 1]);
    let marks = t
        .vcx
        .cx
        .read(|cx| grid.read(cx).painted_sort_marks().to_vec());
    let summary: Vec<_> = marks
        .iter()
        .map(|m| {
            (
                m.col,
                m.level,
                m.desc,
                m.number.as_ref().map(|n| n.to_string()),
            )
        })
        .collect();
    assert!(summary.contains(&(ColId(1), 0, false, Some("1".into()))));
    assert!(summary.contains(&(ColId(0), 1, true, Some("2".into()))));
    let tips: Vec<String> = t.vcx.cx.read(|cx| {
        grid.read(cx)
            .sort_tips()
            .iter()
            .map(|f| plain(&f(cx)))
            .collect()
    });
    assert_eq!(tips[1], "Sort level 2, descending.");
}

/// The sort commands act on the focused grid's focus column; Clear sort goes back.
#[gpui::test]
fn sort_commands_sort_by_the_focus_column(cx: &mut TestAppContext) {
    let t = tile(cx);
    let (store, _w) = live_store(
        vec![row(2, "a"), row(1, "c"), row(3, "b")],
        Completeness::Complete,
    );
    let grid = t.show(store);
    t.focus(&grid);
    t.dispatch(&grid, GridCommand::FirstRow { extend: false });
    t.dispatch(&grid, GridCommand::End { extend: false });
    let run = |id: &str| {
        let ran = in_window(t.vcx, |window, cx| {
            hacivat_ui::actions::ActionRegistry::run(id, window, cx)
        });
        t.redraw();
        ran
    };
    assert!(run("grid.sort_desc"));
    assert_eq!(t.shown(&grid), [1, 2, 0]);
    assert!(run("grid.sort_asc"));
    assert_eq!(t.shown(&grid), [0, 2, 1]);
    assert!(run("grid.clear_sort"));
    assert!(t.map(&grid).is_identity());
}

/// The note's state per M3c-20, as a pure decision.
#[test]
fn the_note_follows_the_sort_state() {
    use sort::note_kind;
    // Nothing sorted, nothing running.
    assert_eq!(note_kind(None, None, 10, false), None);
    // A long sort runs (past 200 ms): "Sorting N rows…" whatever else is true.
    assert_eq!(
        note_kind(Some(1_048_576), Some(10), 20, true),
        Some(NoteKind::Sorting { rows: 1_048_576 })
    );
    // Sorted, complete, no tail: nothing to say.
    assert_eq!(note_kind(None, Some(10), 10, false), None);
    // Sorted, partial.
    assert_eq!(
        note_kind(None, Some(10_000), 10_000, true),
        Some(NoteKind::Partial { sorted: 10_000 })
    );
    // Partial with a tail: both facts.
    assert_eq!(
        note_kind(None, Some(10_000), 11_500, true),
        Some(NoteKind::PartialTail {
            sorted: 10_000,
            tail: 1_500
        })
    );
    // Complete with a tail.
    assert_eq!(
        note_kind(None, Some(10_000), 11_500, false),
        Some(NoteKind::Tail { tail: 1_500 })
    );
}

/// The job itself: the sorted order of a snapshot, and a raised flag stops it.
#[test]
fn the_job_sorts_a_snapshot_and_stops_when_asked() {
    let rows: Vec<Vec<Value>> = (0..10_000i64)
        .map(|i| {
            vec![
                Value::Int((i * 37) % 10_000),
                Value::Text(format!("{i}").into()),
            ]
        })
        .collect();
    let store = ResultStore::from_rows(schema(), rows, Completeness::Complete);
    let snapshot = store.snapshot();
    let keys = sort::job_keys(
        &snapshot,
        &SortSpec::by(ColId(0), false),
        hacivat_proto::DbmsKind::Postgres,
    );
    let order = sort::sort_rows(&snapshot, &keys, &|| false).expect("not cancelled");
    for w in order.windows(2) {
        let (a, b) = (w[0] as i64 * 37 % 10_000, w[1] as i64 * 37 % 10_000);
        assert!(a < b);
    }
    assert!(sort::sort_rows(&snapshot, &keys, &|| true).is_err());
}

/// The perf line (HACIVAT_PERF=1, release): 1,048,576 rows sorted by an integer and by a
/// Turkish-collated text column, off the UI thread (the job function the executor runs).
#[test]
#[ignore = "perf"]
fn sort_of_a_million_rows_perf() {
    if std::env::var("HACIVAT_PERF").as_deref() != Ok("1") {
        return;
    }
    const N: i64 = 1 << 20;
    let words = [
        "İzmir", "ılık", "Irmak", "istek", "çam", "şeker", "Ömer", "üzüm", "zeytin",
    ];
    let rows: Vec<Vec<Value>> = (0..N)
        .map(|i| {
            let w = words[(i as usize * 7) % words.len()];
            vec![
                Value::Int((i * 7_919) % N),
                Value::Text(format!("{w}{}", (i * 31) % 1000).into()),
            ]
        })
        .collect();
    let store = ResultStore::from_rows(schema(), rows, Completeness::Complete);
    let snapshot = store.snapshot();
    let ints = sort::job_keys(
        &snapshot,
        &SortSpec::by(ColId(0), false),
        hacivat_proto::DbmsKind::Postgres,
    );
    let started = Instant::now();
    let order = sort::sort_rows(&snapshot, &ints, &|| false).unwrap();
    let int_ms = started.elapsed().as_millis();
    assert_eq!(order.len(), N as usize);
    let text = sort::job_keys_with(
        &snapshot,
        &SortSpec::by(ColId(1), false),
        hacivat_proto::DbmsKind::Postgres,
        &[None, Some("tr-x-icu")],
    );
    let started = Instant::now();
    let order = sort::sort_rows(&snapshot, &text, &|| false).unwrap();
    let text_ms = started.elapsed().as_millis();
    assert_eq!(order.len(), N as usize);
    let cp = sort::job_keys(
        &snapshot,
        &SortSpec::by(ColId(1), false),
        hacivat_proto::DbmsKind::Postgres,
    );
    let started = Instant::now();
    let _ = sort::sort_rows(&snapshot, &cp, &|| false).unwrap();
    let cp_ms = started.elapsed().as_millis();
    eprintln!(
        "PERF sort 1,048,576 rows: int {int_ms} ms; text tr-x-icu {text_ms} ms; text code point {cp_ms} ms"
    );
    // alg/22 HV-GRID-07: numeric ≤ 300 ms, collated text ≤ 2 s.
    assert!(int_ms <= 300, "int sort took {int_ms} ms");
    assert!(text_ms <= 2_000, "collated sort took {text_ms} ms");
}
