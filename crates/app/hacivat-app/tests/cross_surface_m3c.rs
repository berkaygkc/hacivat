// ## m3c-t43-shots
//! M3c Task 43 (part A): the cross-surface flows of the merged M3c surfaces (`design/m3b-safe`
//! surfaces 14–22), each across two or more tasks' work, through the product's own commands
//! and Run (Task 24's orchestrator) over a FakeDriver session, every wait on a condition.
//!
//! - Several carets type as one edit, one undo takes it back, and Run runs the main caret's
//!   statement only (Task 31, controller ruling 2).
//! - Run at a caret inside a folded statement sends the whole statement, its run ends with a
//!   succeeded mark, and the fold stays (Tasks 32, 34).
//! - The editor find bar's Select all occurrences, then typing, edits every match as one undo
//!   group (Tasks 33, 31).
//! - Sort, then copy: the copy follows the shown order (Task 36).
//! - Grid find with Filter rows, then copy: only the kept rows, in the order shown, under a
//!   sort too (Tasks 37, 36).
//! - Go to row `1.000` in a Turkish format locale reaches view row 1000, under a sort (Tasks 37,
//!   36).
//! - The value viewer's re-run with the raised limit after a sort goes through Run's pipeline
//!   and drops the sort with the old result (Tasks 38, 36, 24).
//! - The grid's AccessKit rows follow a filter and a sort, VoiceOver-free (Tasks 40, 37, 36).
//!
//! **Part B** (after Tasks 39, 41 and 42 merge) adds: the grid menu's Sort / Freeze / Raw display
//! into copy, the narrow pane at 120 px, and the settings rows.

mod support;

use std::sync::Arc;
use std::time::Duration;

use gpui::{
    AppContext as _, Entity, EntityInputHandler as _, Focusable as _, TestAppContext,
    VisualTestContext,
};
use hacivat_app::console::{ConsoleId, ConsoleState, Consoles, find_bar};
use hacivat_app::exec_ui::context::SqlContext;
use hacivat_app::exec_ui::{self, orchestrator};
use hacivat_app::results::adapter::ResultDisplay;
use hacivat_app::results::find::{self, Toggle};
use hacivat_app::results::view::hosts;
use hacivat_app::results::{ActiveResult, ConsoleResults, goto, sort, value_viewer};
use hacivat_driver::ExecSummary;
use hacivat_driver::fake::{
    FakeDriver, FakeExec, FakeQuery, FakeResultSet, FakeRows, FakeTypedCall,
};
use hacivat_exec::{ResultTab, SessionLog, TabKey};
use hacivat_proto::{DbmsKind, EnvTag};
use hacivat_results::{
    CloseReason, Completeness, FetchControl, FetchPolicy, MemoryBudget, ResultId, ResultSource,
    ResultStore, StoreSource,
};
use hacivat_session::session::{ClientId, RunId, UnitId};
use hacivat_text::{SelectionRange, SelectionSet};
use hacivat_ui_editor::{EditorView, ExecMarkState, preview};
use hacivat_ui_grid::model::SortSpec;
use hacivat_ui_grid::{ColId, GridCommand, GridView, ModelCell, ModelRow, ViewRow};
use hacivat_values::valfmt::DisplayZone;
use hacivat_values::{
    Chunk, ColumnMeta, LenUnit, LogicalType, NativeType, ResultSchema, TruncatedVal, Value,
};
use support::*;

// ---- the console ------------------------------------------------------------------------------

/// `n` answers to an internal read starting with `prefix`.
fn reads(prefix: &'static str, row: Vec<&str>) -> Vec<FakeExec> {
    (0..8)
        .map(|_| FakeExec {
            prefix,
            result: Ok(ExecSummary {
                rows: vec![row.iter().map(|v| Some(v.to_string())).collect()],
                row_count: 1,
                ..ExecSummary::default()
            }),
            delay: Duration::ZERO,
            tx_after: None,
            panics: false,
        })
        .collect()
}

/// PostgreSQL answering the console's SQL-context probe and Run's context reads.
fn pg() -> FakeDriver {
    FakeDriver::new(DbmsKind::Postgres)
        .tracks_transactions()
        .exec(
            reads("SHOW standard_conforming_strings", vec!["on"])
                .into_iter()
                .chain(reads(
                    "SELECT pg_catalog.current_setting('standard_conforming_strings')",
                    vec!["on", "\"$user\", public"],
                ))
                .collect(),
        )
}

/// A console over a FakeDriver session of a dev PostgreSQL profile, its SQL context read.
struct Console {
    vcx: &'static mut VisualTestContext,
    id: ConsoleId,
    fake: Arc<FakeDriver>,
    editor: Entity<EditorView>,
}

fn console(cx: &mut TestAppContext, fake: FakeDriver) -> Console {
    let vcx = shell_with_connections(cx, "en");
    in_window(vcx, |window, _| window.activate_window());
    let p = hacivat_session::profile::Profile {
        engine: DbmsKind::Postgres,
        ..test_profile("orders-dev", Some(EnvTag::Dev), "127.0.0.1")
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
        vcx.cx
            .read(|cx| exec_ui::sql_context(id, cx) != SqlContext::Pending)
    });
    let editor = vcx.cx.read(|cx| {
        Consoles::get(cx, id)
            .and_then(|s| s.editor.clone())
            .expect("the console has an editor")
    });
    // The shots and the tests never touch the machine's find pasteboard (Task 33, M10).
    vcx.cx.update(|cx| {
        find_bar::install_find_pasteboard(
            cx,
            std::rc::Rc::new(|_: &gpui::App| None),
            std::rc::Rc::new(|_: &str, _: &gpui::App| {}),
        )
    });
    Console {
        vcx,
        id,
        fake,
        editor,
    }
}

impl Console {
    fn set_text(&self, text: &str) {
        let text = text.to_string();
        self.vcx
            .cx
            .update(|cx| self.editor.update(cx, |e, cx| e.set_text(&text, cx)));
        self.vcx.run_until_parked();
    }

    fn text(&self) -> String {
        self.vcx
            .cx
            .read(|cx| self.editor.read(cx).snapshot().buffer.to_string())
    }

    fn select(&self, ranges: &[(usize, usize)]) {
        let set = SelectionSet::normalized(
            ranges.iter().map(|(a, h)| SelectionRange::new(*a, *h)),
            ranges.len() - 1,
        );
        self.vcx
            .cx
            .update(|cx| self.editor.update(cx, |e, cx| e.select_ranges(set, cx)));
        self.vcx.run_until_parked();
    }

    fn command(&self, id: &'static str) {
        let editor = self.editor.clone();
        in_window(self.vcx, |window, cx| {
            let handle = editor.read(cx).focus_handle(cx);
            window.focus(&handle, cx);
            editor.update(cx, |e, cx| assert!(e.run_command(id, window, cx), "{id}"));
        });
        self.vcx.run_until_parked();
    }

    /// Types `text` at every caret (the platform's text input path).
    fn type_text(&self, text: &str) {
        let editor = self.editor.clone();
        in_window(self.vcx, |window, cx| {
            editor.update(cx, |e, cx| e.replace_text_in_range(None, text, window, cx));
        });
        self.vcx.run_until_parked();
    }

    fn carets(&self) -> u32 {
        self.vcx.cx.read(|cx| self.editor.read(cx).status().carets)
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

    /// Run (⌘↵) at the carets; in a script the chooser asks first (the statement at the caret
    /// preselected, Task 24): Enter takes it.
    fn run(&self) {
        self.command("editor.run");
        if self
            .vcx
            .cx
            .read(|cx| exec_ui::chooser::is_open(self.id, cx))
        {
            in_window(self.vcx, |window, cx| {
                window.dispatch_keystroke(gpui::Keystroke::parse("enter").unwrap(), cx)
            });
            self.vcx.run_until_parked();
        }
        self.wait_run_end();
    }

    fn wait_run_end(&self) {
        wait_until(self.vcx, "the run's end", |vcx| {
            vcx.executor().advance_clock(Duration::from_millis(20));
            vcx.run_until_parked();
            vcx.cx.read(|cx| exec_ui::last_run_ended(self.id, cx))
        });
    }

    fn redraw(&self) {
        in_window(self.vcx, |window, _| window.refresh());
        self.vcx.run_until_parked();
    }

    /// The Run band's units once it shows (the preview runs on a short timer: the clock moves
    /// while the condition is polled).
    fn band(&self) -> Vec<std::ops::Range<usize>> {
        let read = |vcx: &VisualTestContext| {
            vcx.cx
                .read(|cx| preview::current(self.editor.read(cx)).map(|b| b.units.clone()))
        };
        wait_until(self.vcx, "the band", |vcx| {
            vcx.executor().advance_clock(Duration::from_millis(60));
            vcx.run_until_parked();
            read(vcx).is_some()
        });
        read(self.vcx).expect("the band shows").to_vec()
    }
}

// ---- 14 · several carets → one undo → Run ---------------------------------------------------

const MULTI: &str = "SELECT musteri_id FROM siparisler;\nSELECT musteri_id FROM faturalar;\nSELECT musteri_id FROM iadeler;\n";

/// ⌃G twice selects the three `musteri_id`; typing replaces all three as one edit, one undo
/// takes them all back (selection kept), and Run with the three carets runs the main caret's
/// statement only (the last one added; controller ruling 2): the preview shows what is sent.
#[gpui::test]
fn multi_caret_typing_is_one_undo_and_run_runs_the_main_caret(cx: &mut TestAppContext) {
    let c = console(
        cx,
        pg().queries(vec![FakeQuery {
            prefix: "SELECT",
            sets: vec![FakeResultSet {
                columns: vec![("musteri_id", int8())],
                rows: FakeRows::Generated {
                    count: 2,
                    cell: |r, _| Value::Int(r as i64),
                },
            }],
            ..Default::default()
        }]),
    );
    c.set_text(MULTI);
    let first = MULTI.find("musteri_id").unwrap();
    c.select(&[(first, first + "musteri_id".len())]);
    c.command("editor.select_next_occurrence");
    c.command("editor.select_next_occurrence");
    assert_eq!(c.carets(), 3);
    c.type_text("kod");
    assert_eq!(
        c.text(),
        "SELECT kod FROM siparisler;\nSELECT kod FROM faturalar;\nSELECT kod FROM iadeler;\n"
    );
    assert_eq!(c.carets(), 3, "a caret after each replacement");
    c.command("editor.undo");
    assert_eq!(c.text(), MULTI, "one undo takes the three edits back");
    // Carets in the three statements: the band and Run take the main caret's (the third).
    let at = |needle: &str| MULTI.find(needle).unwrap() + 2;
    c.select(&[
        (at("siparisler"), at("siparisler")),
        (at("faturalar"), at("faturalar")),
        (at("iadeler"), at("iadeler")),
    ]);
    let units = c.band();
    assert_eq!(units.len(), 1);
    assert_eq!(&MULTI[units[0].clone()], "SELECT musteri_id FROM iadeler");
    c.run();
    assert_eq!(c.queries(), ["SELECT musteri_id FROM iadeler"]);
    stop_core_runtime_test(cx);
}

// ---- 15 · fold → Run at a caret in the fold ------------------------------------------------

const FOLDS: &str =
    "SELECT o.id,\n       o.total\n  FROM orders o\n WHERE o.total > 100;\n\nSELECT 2;\n";

/// A caret inside a folded statement (the fold keeps the caret on its header) runs the whole
/// statement, the band covers it, the run ends with one succeeded mark, and the fold stays.
#[gpui::test]
fn run_at_a_caret_inside_a_fold_runs_the_whole_statement(cx: &mut TestAppContext) {
    let c = console(
        cx,
        pg().queries(vec![FakeQuery {
            prefix: "SELECT o.id",
            sets: vec![FakeResultSet {
                columns: vec![("id", int8()), ("total", int8())],
                rows: FakeRows::Generated {
                    count: 3,
                    cell: |r, c| Value::Int((r * 10 + c as u64) as i64),
                },
            }],
            ..Default::default()
        }]),
    );
    c.set_text(FOLDS);
    let caret = FOLDS.find("o.total >").unwrap() + 3;
    c.select(&[(caret, caret)]);
    // The folding ranges come from the worker: fold once they landed.
    wait_until(c.vcx, "the statement folds", |vcx| {
        vcx.cx.update(|cx| {
            c.editor.update(cx, |e, cx| {
                if e.folds().folded().is_empty() {
                    e.toggle_fold_at(0, cx);
                }
            })
        });
        vcx.run_until_parked();
        vcx.cx
            .read(|cx| c.editor.read(cx).folds().folded().len() == 1)
    });
    let statement = &FOLDS[..FOLDS.find(";").unwrap()];
    let band = c.band();
    assert_eq!(band.len(), 1);
    assert_eq!(
        &FOLDS[band[0].clone()],
        statement,
        "the band covers the folded statement"
    );
    c.run();
    assert_eq!(c.queries(), [statement]);
    let marks = c.vcx.cx.read(|cx| {
        exec_ui::last_run_marks(c.id, cx)
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
    });
    assert!(
        matches!(marks.as_slice(), [ExecMarkState::Succeeded { .. }]),
        "{marks:?}"
    );
    assert_eq!(
        c.vcx.cx.read(|cx| c.editor.read(cx).folds().folded().len()),
        1,
        "the fold stays"
    );
    stop_core_runtime_test(cx);
}

// ---- 16 · editor find → Select all occurrences → type --------------------------------------

/// The find bar's Select all occurrences gives every match a selection (Turkish-tolerant on:
/// `izmir` finds İzmir, IZMIR and ızmır); typing then edits all of them, one undo group.
#[gpui::test]
fn find_select_all_then_type_edits_every_match_as_one_undo(cx: &mut TestAppContext) {
    let c = console(cx, pg());
    let text = "SELECT 'İzmir', 'IZMIR', 'ızmır', 'Ankara';\n";
    c.set_text(text);
    c.select(&[(0, 0)]);
    c.command("editor.find");
    let id = c.id;
    in_window(c.vcx, |window, cx| {
        find_bar::set_query(id, "izmir", window, cx);
        if !find_bar::options(id, cx).is_some_and(|o| o.tolerant) {
            // Turkish-tolerant (on by default in Turkish only).
            find_bar::flip_toggle(id, 3, window, cx);
        }
    });
    wait_until(c.vcx, "the count", |vcx| {
        vcx.cx
            .read(|cx| c.editor.read(cx).find_status().total == Some(3))
    });
    in_window(c.vcx, |window, cx| {
        assert!(find_bar::run_in_bar(
            "editor.select_all_occurrences",
            window,
            cx
        ))
    });
    c.vcx.run_until_parked();
    assert_eq!(c.carets(), 3);
    c.type_text("Izmir");
    assert_eq!(c.text(), "SELECT 'Izmir', 'Izmir', 'Izmir', 'Ankara';\n");
    c.command("editor.undo");
    assert_eq!(c.text(), text);
    stop_core_runtime_test(cx);
}

// ---- the result tile --------------------------------------------------------------------------

struct NoFetch;

impl FetchControl for NoFetch {
    fn fetch_more(&self, _: ResultId, _: u64) {}
    fn close(&self, _: ResultId, _: CloseReason) {}
    fn fetch_all(&self, _: ResultId) {}
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

fn column(name: &str, logical: LogicalType, native: &str) -> ColumnMeta {
    ColumnMeta {
        name: name.into(),
        logical,
        native: NativeType::Pg {
            oid: 0,
            typmod: -1,
            name: native.into(),
        },
        origin: None,
    }
}

/// `id` and `şehir`.
fn schema() -> Arc<ResultSchema> {
    Arc::new(ResultSchema {
        columns: Arc::from([
            column("id", int8(), "int8"),
            column("şehir", text(), "text"),
        ]),
    })
}

fn row(id: i64, city: &str) -> Vec<Value> {
    vec![Value::Int(id), Value::Text(city.into())]
}

fn source(rows: Vec<Vec<Value>>) -> Arc<dyn ResultSource> {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1 << 44);
    let id = ResultId(NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed));
    let schema = schema();
    let (store, mut writer) =
        ResultStore::new(id, schema.clone(), MemoryBudget::new(u64::MAX, u64::MAX), 1);
    writer.append(Chunk::from_values(&schema, rows));
    writer.set_state(Completeness::Complete);
    StoreSource::new(store, Arc::new(NoFetch), FetchPolicy::default())
}

/// A shell whose focused pane shows a console with results only (no session).
struct Tile {
    vcx: &'static mut VisualTestContext,
    console: ConsoleId,
    results: Entity<ConsoleResults>,
}

fn tile(cx: &mut TestAppContext, locale: &str) -> Tile {
    let vcx = shell_with_connections(cx, "en");
    in_window(vcx, |window, _| window.activate_window());
    let profile = add_profile(vcx, test_profile("orders-dev", None, "10.0.0.5"));
    let ws = workspace(vcx);
    let locale = locale.to_string();
    let (console, results) = in_window(vcx, |window, cx| {
        let tab = ws
            .update(cx, |ws, cx| ws.open_console_tab(Some(profile), window, cx))
            .expect("a console tab opens");
        let log = SessionLog::new();
        let results = cx.new(|_| ConsoleResults::new(log.clone()));
        results.update(cx, |r, cx| {
            r.set_display(
                ResultDisplay::new(&locale, DisplayZone::utc(), DbmsKind::Postgres),
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
    vcx.cx.update(|cx| {
        find_bar::install_find_pasteboard(
            cx,
            std::rc::Rc::new(|_: &gpui::App| None),
            std::rc::Rc::new(|_: &str, _: &gpui::App| {}),
        )
    });
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
                r.apply(
                    &[ResultTab {
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
                    }],
                    cx,
                );
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

    fn dispatch(&self, grid: &Entity<GridView>, cmd: GridCommand) {
        in_window(self.vcx, |window, cx| {
            let handle = grid.read(cx).focus_handle(cx);
            window.focus(&handle, cx);
            grid.update(cx, |g, cx| g.dispatch(cmd, window, cx))
        });
        self.redraw();
    }

    /// Sorts by `col` and waits for the job's order to show.
    fn sort_by(&self, grid: &Entity<GridView>, col: u32, desc: bool) {
        in_window(self.vcx, |_, cx| {
            grid.update(cx, |g, cx| {
                g.request_sort(SortSpec::by(ColId(col), desc), cx)
            })
        });
        let g = grid.clone();
        wait_until(self.vcx, "the sort", move |vcx| {
            in_window(vcx, |window, _| window.refresh());
            vcx.run_until_parked();
            vcx.cx.read(|cx| {
                sort::status(&g, cx).is_some_and(|s| s.running.is_none() && s.sorted.is_some())
            })
        });
    }

    /// The model rows in the order shown.
    fn shown(&self, grid: &Entity<GridView>) -> Vec<u64> {
        self.vcx.cx.read(|cx| {
            let map = grid.read(cx).view_map().clone();
            (0..map.len())
                .map(|v| map.view_to_model(ViewRow(v)).unwrap().0)
                .collect()
        })
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

    fn find(&self, query: &str) {
        let (c, q) = (self.console, query.to_string());
        in_window(self.vcx, move |_, cx| {
            find::open(c, cx);
            find::set_query(c, &q, cx);
        });
        wait_until(self.vcx, "the scan", |vcx| {
            in_window(vcx, |window, _| window.refresh());
            vcx.run_until_parked();
            vcx.cx
                .read(|cx| find::status(c, cx).is_some_and(|v| v.total.is_some()))
        });
    }
}

fn cities() -> Vec<Vec<Value>> {
    vec![
        row(30, "İzmir"),
        row(10, "Ankara"),
        row(50, "izmir"),
        row(20, "Bursa"),
        row(40, "İZMİR"),
    ]
}

// ---- 18 · sort → copy ----------------------------------------------------------------------

/// A local sort, then Select all and Copy: the clipboard holds the rows as shown (ledger 253
/// I5), never the source order; a second sort (descending) copies its own order.
#[gpui::test]
fn sort_then_copy_copies_the_shown_order(cx: &mut TestAppContext) {
    let t = tile(cx, "en-US");
    let grid = t.show(source(cities()));
    t.sort_by(&grid, 0, false);
    assert_eq!(t.shown(&grid), [1, 3, 0, 4, 2]);
    t.dispatch(&grid, GridCommand::SelectAll);
    assert_eq!(
        t.copy(&grid),
        "10\tAnkara\n20\tBursa\n30\tİzmir\n40\tİZMİR\n50\tizmir\n"
    );
    t.sort_by(&grid, 0, true);
    t.dispatch(&grid, GridCommand::SelectAll);
    assert_eq!(
        t.copy(&grid),
        "50\tizmir\n40\tİZMİR\n30\tİzmir\n20\tBursa\n10\tAnkara\n"
    );
}

// ---- 19 · grid find → Filter rows → copy ---------------------------------------------------

/// Grid find with Filter rows keeps only the matching rows (Turkish-tolerant: `izmir` finds
/// İzmir, izmir and İZMİR); Select all and Copy copy only them, in the order shown, and under a
/// sort the filter keeps the sort's order (Task 37 reading 1).
#[gpui::test]
fn grid_find_filter_rows_then_copy_copies_the_kept_rows(cx: &mut TestAppContext) {
    let t = tile(cx, "en-US");
    let grid = t.show(source(cities()));
    t.find("izmir");
    let c = t.console;
    in_window(t.vcx, move |_, cx| {
        if !find::options(c, cx).is_some_and(|(o, _)| o.tolerant) {
            find::flip(c, Toggle::Tolerant, cx);
        }
        find::flip(c, Toggle::Filter, cx);
    });
    wait_until(t.vcx, "the filter", |vcx| {
        in_window(vcx, |window, _| window.refresh());
        vcx.run_until_parked();
        vcx.cx
            .read(|cx| find::status(c, cx).is_some_and(|v| v.filtered == Some(3)))
    });
    assert_eq!(t.shown(&grid), [0, 2, 4]);
    t.dispatch(&grid, GridCommand::SelectAll);
    assert_eq!(t.copy(&grid), "30\tİzmir\n50\tizmir\n40\tİZMİR\n");
    // Sorted by id descending: the kept rows follow the sort.
    t.sort_by(&grid, 0, true);
    wait_until(t.vcx, "the filter in the sort's order", |vcx| {
        in_window(vcx, |window, _| window.refresh());
        vcx.run_until_parked();
        vcx.cx.read(|cx| {
            let map = grid.read(cx).view_map().clone();
            map.len() == 3 && map.view_to_model(ViewRow(0)) == Some(ModelRow(2))
        })
    });
    t.dispatch(&grid, GridCommand::SelectAll);
    assert_eq!(t.copy(&grid), "50\tizmir\n40\tİZMİR\n30\tİzmir\n");
}

// ---- 20 · go to row 1.000 in Turkish -------------------------------------------------------

/// In a Turkish format locale `1.000` is row 1000 (a view row): under a descending sort the
/// focus lands on the model row shown there, column `şehir` by name.
#[gpui::test]
fn go_to_row_1_000_in_turkish_reaches_view_row_1000_under_a_sort(cx: &mut TestAppContext) {
    let t = tile(cx, "tr-TR");
    let rows: Vec<Vec<Value>> = (0..1_200).map(|i| row(i, "Bursa")).collect();
    let grid = t.show(source(rows));
    t.sort_by(&grid, 0, true);
    let c = t.console;
    in_window(t.vcx, move |_, cx| {
        goto::open(c, cx);
        goto::set_text(c, "1.000:şehir", cx);
    });
    t.redraw();
    assert!(
        matches!(
            t.vcx.cx.read(|cx| goto::line(c, cx)),
            Some(goto::GotoLine::Target(_))
        ),
        "the live line names the target"
    );
    in_window(t.vcx, move |_, cx| goto::submit(c, cx));
    t.redraw();
    // View row 1000 of 1,200 descending is id 200 (model row 200).
    let focus = t.vcx.cx.read(|cx| grid.read(cx).selection().focus());
    assert_eq!(
        focus,
        Some(ModelCell {
            row: ModelRow(200),
            col: ColId(1)
        })
    );
    assert!(
        !t.vcx.cx.read(|cx| goto::is_open(c, cx)),
        "Enter goes and closes"
    );
}

// ---- 22 · value viewer → re-run with the raised limit --------------------------------------

/// A text value cut at 1 MiB of 5 MiB.
fn cut_note(_: u64, _: usize) -> Value {
    Value::Truncated(Box::new(TruncatedVal {
        prefix: Value::Text("n".repeat(1 << 20).into()),
        total_len: Some(5 << 20),
        unit: LenUnit::Bytes,
    }))
}

/// A result sorted locally, its cut value opened in the viewer, re-run with the raised limit:
/// the statement goes through Run's pipeline again (sent a second time, with the 64 MiB cap),
/// and the new result shows unsorted (a re-run drops the old result's order).
#[gpui::test]
fn value_viewer_rerun_after_a_sort_takes_runs_path_and_drops_the_sort(cx: &mut TestAppContext) {
    let set = || FakeResultSet {
        columns: vec![("id", int8()), ("note", text())],
        rows: FakeRows::Generated {
            count: 3,
            cell: |r, c| {
                if c == 0 {
                    Value::Int(10 - r as i64)
                } else {
                    cut_note(r, c)
                }
            },
        },
    };
    let c = console(
        cx,
        pg().queries(vec![
            FakeQuery {
                prefix: "SELECT id, note",
                sets: vec![set()],
                ..Default::default()
            },
            FakeQuery {
                prefix: "SELECT id, note",
                sets: vec![set()],
                ..Default::default()
            },
        ]),
    );
    c.set_text("SELECT id, note FROM notes");
    c.select(&[(0, 0)]);
    c.command("editor.run");
    c.wait_run_end();
    let id = c.id;
    let grid_of = |vcx: &VisualTestContext| {
        vcx.cx.read(|cx| {
            hosts(cx)
                .into_iter()
                .find_map(|h| h.read(cx).grid_of(id, 0, cx))
        })
    };
    wait_until(c.vcx, "the result's grid", |vcx| {
        in_window(vcx, |window, _| window.refresh());
        vcx.run_until_parked();
        grid_of(vcx).is_some()
    });
    let grid = grid_of(c.vcx).unwrap();
    in_window(c.vcx, |_, cx| {
        grid.update(cx, |g, cx| {
            g.request_sort(SortSpec::by(ColId(0), false), cx)
        })
    });
    wait_until(c.vcx, "the sort", |vcx| {
        in_window(vcx, |window, _| window.refresh());
        vcx.run_until_parked();
        vcx.cx
            .read(|cx| sort::status(&grid, cx).is_some_and(|s| s.sorted.is_some()))
    });
    in_window(c.vcx, |_, cx| {
        value_viewer::open(
            id,
            ModelCell {
                row: ModelRow(0),
                col: ColId(1),
            },
            cx,
        )
    });
    c.redraw();
    let v = c
        .vcx
        .cx
        .read(|cx| value_viewer::view(id, cx))
        .expect("the viewer is open");
    assert!(v.truncated.is_some() && v.rerun.is_some(), "{v:?}");
    in_window(c.vcx, |_, cx| value_viewer::rerun(id, cx));
    wait_until(c.vcx, "the re-run", |_| c.queries().len() == 2);
    c.wait_run_end();
    assert_eq!(
        c.queries(),
        ["SELECT id, note FROM notes", "SELECT id, note FROM notes"]
    );
    assert_eq!(
        c.vcx.cx.read(|cx| orchestrator::last_run_cell_cap(id, cx)),
        Some(hacivat_results::window::MAX_CELL_CAP),
        "sent with the raised limit"
    );
    wait_until(c.vcx, "the new result, unsorted", |vcx| {
        in_window(vcx, |window, _| window.refresh());
        vcx.run_until_parked();
        grid_of(vcx).is_some_and(|g| vcx.cx.read(|cx| g.read(cx).sort().keys.is_empty()))
    });
    stop_core_runtime_test(cx);
}

// ---- 18–20 · the grid's AccessKit rows (Task 40), VoiceOver-free ---------------------------

/// With assistive technology assumed active, the grid's row nodes name view rows: under Filter
/// rows only the kept rows are nodes, numbered 1…n, and a sort reorders the nodes with it; the
/// header's sorted cell carries its direction.
#[gpui::test]
fn grid_a11y_rows_follow_filter_and_sort(cx: &mut TestAppContext) {
    let t = tile(cx, "en-US");
    let grid = t.show(source(cities()));
    in_window(t.vcx, |_, cx| {
        grid.update(cx, |g, cx| g.assume_a11y_active(true, cx))
    });
    t.redraw();
    let nodes = |t: &Tile| t.vcx.cx.read(|cx| grid.read(cx).a11y_nodes());
    let body_rows = |t: &Tile| {
        nodes(t)
            .iter()
            .filter(|r| !r.is_header())
            .map(|r| (r.view_row().unwrap(), r.cells[0].name.to_string()))
            .collect::<Vec<_>>()
    };
    assert_eq!(body_rows(&t).len(), 5);
    t.find("izmir");
    let c = t.console;
    in_window(t.vcx, move |_, cx| {
        if !find::options(c, cx).is_some_and(|(o, _)| o.tolerant) {
            find::flip(c, Toggle::Tolerant, cx);
        }
        find::flip(c, Toggle::Filter, cx);
    });
    wait_until(t.vcx, "the filtered nodes", |vcx| {
        in_window(vcx, |window, _| window.refresh());
        vcx.run_until_parked();
        vcx.cx.read(|cx| {
            grid.read(cx)
                .a11y_nodes()
                .iter()
                .filter(|r| !r.is_header())
                .count()
                == 3
        })
    });
    let rows = body_rows(&t);
    assert_eq!(
        rows.iter().map(|(v, _)| *v).collect::<Vec<_>>(),
        [0, 1, 2],
        "view rows 1…n"
    );
    assert!(rows[0].1.contains("30"), "{rows:?}");
    t.sort_by(&grid, 0, true);
    wait_until(t.vcx, "the sorted nodes", |vcx| {
        in_window(vcx, |window, _| window.refresh());
        vcx.run_until_parked();
        vcx.cx.read(|cx| {
            grid.read(cx)
                .a11y_nodes()
                .iter()
                .find(|r| r.view_row() == Some(0))
                .is_some_and(|r| r.cells[0].name.contains("50"))
        })
    });
    let header = nodes(&t)
        .iter()
        .find(|r| r.is_header())
        .map(|r| r.cells[0].sort)
        .expect("the header row");
    assert_eq!(header, Some(gpui::accesskit::SortDirection::Descending));
}
