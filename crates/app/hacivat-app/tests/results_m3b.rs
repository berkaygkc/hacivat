//! M3b Task 25: the results tile's M3b additions (HV-EXEC-07 UI; HV-RES-07, HV-RES-10;
//! R-M3-66, R-M3-68, R-M3-76; design/m3b-safe surface 10): one tab per result set titled by
//! its statement's comment with `(2)` repeats, pinned tabs, Count rows, notices and warnings
//! in the log, the evicted strip and the selection it clears, the fetch-all question,
//! visibility for eviction, the incremental log view and its keyboard.

#![cfg_attr(not(unix), allow(dead_code, unused_imports, unused_variables))]

mod support;

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use gpui::{AppContext as _, Entity, TestAppContext, VisualTestContext};
use hacivat_app::console::{ConsoleId, ConsoleState, Consoles};
use hacivat_app::exec_ui::context::SqlContext;
use hacivat_app::results::count::{self, CountHandle, CountJob};
use hacivat_app::results::output_view::code_chip;
use hacivat_app::results::tabs::{CountState, HIDE_GRACE};
use hacivat_app::results::view::{ResultsHost, hosts};
use hacivat_app::results::{ActiveResult, ConsoleResults, ResultsEvent, pin};
use hacivat_driver::fake::{FakeDriver, FakeExec, FakeQuery, FakeResultSet, FakeRows};
use hacivat_driver::{ExecSummary, NoticeSeverity, ServerMessage};
use hacivat_exec::{
    CountError, ErrorView, LogMsg, ResultTab, SessionLog, TabKey, status::error_view,
};
use hacivat_proto::{DbmsKind, EnvTag};
use hacivat_results::{
    CloseReason, Completeness, CursorInfo, FetchControl, FetchPolicy, MemoryBudget, ResultId,
    ResultSource, ResultStore, SourceCaps, StoreSnapshot, StoreSource,
};
use hacivat_session::session::{ClientId, ExecError, ExecErrorKind, RunId, UnitId};
use hacivat_ui_grid::{GridCommand, GridEvent};
use hacivat_values::{Chunk, ColumnMeta, LogicalType, NativeType, ResultSchema, Value};
use support::*;

// ---- fixtures -------------------------------------------------------------------------------

fn int_column(name: &str, native: NativeType) -> ColumnMeta {
    ColumnMeta {
        name: name.into(),
        logical: LogicalType::Int {
            bits: 64,
            signed: true,
        },
        native,
        origin: None,
    }
}

fn int8() -> NativeType {
    NativeType::Pg {
        oid: 20,
        typmod: -1,
        name: "int8".into(),
    }
}

fn schema(cols: usize) -> Arc<ResultSchema> {
    Arc::new(ResultSchema {
        columns: (0..cols)
            .map(|c| int_column(&format!("c{c}"), int8()))
            .collect(),
    })
}

fn rows(range: std::ops::Range<u64>, cols: usize) -> Vec<Vec<Value>> {
    range
        .map(|r| {
            (0..cols)
                .map(|c| Value::Int(r as i64 * 10 + c as i64))
                .collect()
        })
        .collect()
}

fn cursor() -> CursorInfo {
    let now = Instant::now();
    CursorInfo {
        opened_at: now,
        last_fetch: now,
        holds_tx: true,
        idle_bound: Duration::from_secs(600),
    }
}

/// What a result source told its session, and what the app told it about visibility.
#[derive(Default)]
struct Control {
    fetch_alls: Mutex<u32>,
}

impl FetchControl for Control {
    fn fetch_more(&self, _: ResultId, _: u64) {}
    fn close(&self, _: ResultId, _: CloseReason) {}
    fn fetch_all(&self, _: ResultId) {
        *self.fetch_alls.lock().unwrap() += 1;
    }
}

/// A source that records every `set_visible` the app sends (and passes it on).
struct Spy {
    inner: Arc<dyn ResultSource>,
    seen: Mutex<Vec<bool>>,
}

impl Spy {
    fn over(store: Arc<ResultStore>) -> Arc<Spy> {
        Arc::new(Spy {
            inner: StoreSource::new(store, Arc::new(Control::default()), FetchPolicy::default()),
            seen: Mutex::new(Vec::new()),
        })
    }

    fn seen(&self) -> Vec<bool> {
        self.seen.lock().unwrap().clone()
    }
}

impl ResultSource for Spy {
    fn snapshot(&self) -> Arc<StoreSnapshot> {
        self.inner.snapshot()
    }
    fn changes(&self) -> tokio::sync::watch::Receiver<(u64, u64)> {
        self.inner.changes()
    }
    fn ensure_rows(&self, upto: u64) {
        self.inner.ensure_rows(upto);
    }
    fn caps(&self) -> SourceCaps {
        self.inner.caps()
    }
    fn fetch_all(&self) {
        self.inner.fetch_all();
    }
    fn close_cursor(&self) {
        self.inner.close_cursor();
    }
    fn raise_cap(&self) {
        self.inner.raise_cap();
    }
    fn cap_raised(&self) -> bool {
        self.inner.cap_raised()
    }
    fn set_visible(&self, visible: bool) {
        self.seen.lock().unwrap().push(visible);
        self.inner.set_visible(visible);
    }
}

fn finished(n: u64) -> Arc<dyn ResultSource> {
    StoreSource::new(
        ResultStore::from_rows(schema(2), rows(0..n, 2), Completeness::Complete),
        Arc::new(Control::default()),
        FetchPolicy::default(),
    )
}

/// Result set `set` of unit `ordinal` of run `run`, titled `title` by the run service.
fn result_tab(
    run: u64,
    ordinal: u32,
    set: u32,
    title: &str,
    source: Arc<dyn ResultSource>,
) -> ResultTab {
    ResultTab {
        key: TabKey {
            console: 1,
            unit_ordinal: ordinal,
            result_ordinal: set,
        },
        title: title.into(),
        unit: UnitId {
            run: RunId(run),
            ordinal,
        },
        source,
    }
}

/// A shell whose focused pane shows a console bound to `orders-dev`, with empty results and
/// log, and the text registry on.
struct Tile {
    vcx: &'static mut VisualTestContext,
    console: ConsoleId,
    results: Entity<ConsoleResults>,
    log: Arc<SessionLog>,
}

fn tile(cx: &mut TestAppContext) -> Tile {
    let vcx = shell_with_connections(cx, "en");
    let profile = add_profile(
        vcx,
        test_profile("orders-dev", Some(EnvTag::Dev), "10.0.0.5"),
    );
    let ws = workspace(vcx);
    let (console, results, log) = in_window(vcx, |window, cx| {
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
                log: log.clone(),
                service: None,
            },
        );
        (console, results, log)
    });
    enable_registry(vcx);
    vcx.run_until_parked();
    Tile {
        vcx,
        console,
        results,
        log,
    }
}

impl Tile {
    fn apply(&self, tabs: &[ResultTab]) {
        in_window(self.vcx, |_, cx| {
            self.results.update(cx, |r, cx| {
                r.apply(tabs, cx);
            })
        });
        self.redraw();
    }

    fn set_active(&self, active: ActiveResult) {
        in_window(self.vcx, |_, cx| {
            self.results.update(cx, |r, cx| r.set_active(active, cx))
        });
        self.redraw();
    }

    fn redraw(&self) {
        in_window(self.vcx, |window, _| window.refresh());
        self.vcx.run_until_parked();
    }

    fn host(&self) -> Entity<ResultsHost> {
        self.vcx
            .read(|cx| hosts(cx).into_iter().next())
            .expect("the results tile is built")
    }

    fn text(&self, id: &str) -> Option<String> {
        text_of(self.vcx, id).map(|t| t.replace(['\u{2068}', '\u{2069}'], ""))
    }

    fn titles(&self) -> Vec<String> {
        self.vcx.read(|cx| {
            let r = self.results.read(cx);
            (0..r.tabs().len())
                .map(|ix| {
                    r.title(ix, cx)
                        .expect("a tab")
                        .replace(['\u{2068}', '\u{2069}'], "")
                })
                .collect()
        })
    }

    fn click(&mut self, selector: &'static str) {
        click(self.vcx, selector);
        self.redraw();
    }

    /// The events the console's results emit from now on.
    fn events(&self) -> Rc<RefCell<Vec<ResultsEvent>>> {
        let seen = Rc::new(RefCell::new(Vec::new()));
        let sink = seen.clone();
        let results = self.results.clone();
        self.vcx.cx.update(|cx| {
            cx.subscribe(&results, move |_, e: &ResultsEvent, _| {
                sink.borrow_mut().push(e.clone())
            })
            .detach();
        });
        seen
    }

    fn advance(&self, d: Duration) {
        self.vcx.cx.executor().advance_clock(d);
        self.vcx.run_until_parked();
    }
}

// ---- titles and pinning -------------------------------------------------------------------------

/// R-M3-66: one tab per result set, titled by its statement's comment as the run service gave
/// it; a statement's further sets and a second statement under the same comment get `(2)`,
/// `(3)`; a statement without a comment is the console's `Result N`; a comment title is data.
#[gpui::test]
fn comment_titles_and_suffixes_on_tabs(cx: &mut TestAppContext) {
    let t = tile(cx);
    let untyped = Arc::new(ResultSchema {
        columns: vec![ColumnMeta {
            name: "n".into(),
            logical: LogicalType::Text {
                max_chars: None,
                fixed: false,
            },
            native: NativeType::Unknown,
            origin: None,
        }]
        .into(),
    });
    let untyped: Arc<dyn ResultSource> = StoreSource::new(
        ResultStore::from_rows(
            untyped,
            vec![vec![Value::Text("1".into())]],
            Completeness::Complete,
        ),
        Arc::new(Control::default()),
        FetchPolicy::default(),
    );
    // The run service's titles: the comment, ` (2)` for the statement's second set, `Result 2`
    // for a statement without one, the same comment on a later statement.
    t.apply(&[
        result_tab(1, 0, 0, "Şehre göre ciro", finished(3)),
        result_tab(1, 0, 1, "Şehre göre ciro (2)", finished(2)),
        result_tab(1, 1, 0, "Result 2", finished(1)),
        result_tab(1, 2, 0, "Şehre göre ciro", finished(4)),
        result_tab(1, 3, 0, "Result 4", untyped),
    ]);
    assert_eq!(
        t.titles(),
        [
            "Şehre göre ciro",
            "Şehre göre ciro (2)",
            "Result 2",
            "Şehre göre ciro (3)",
            "Result 4"
        ]
    );
    assert_eq!(t.text("results-tab-0").as_deref(), Some("Şehre göre ciro"));
    assert_eq!(
        t.text("results-tab-1").as_deref(),
        Some("Şehre göre ciro (2)")
    );
    assert_eq!(t.text("results-tab-2").as_deref(), Some("Result 2"));
    // A comment title alone is data; a repeat and `Result N` are Fluent messages.
    t.vcx.read(|cx| {
        let r = t.results.read(cx);
        let source = |ix| r.title_with_source(ix, cx).expect("a tab").1;
        assert_eq!(source(0), hacivat_ui::text::TextSource::Data);
        assert_eq!(source(1), hacivat_ui::text::TextSource::Fluent);
        assert_eq!(source(2), hacivat_ui::text::TextSource::Fluent);
        assert_eq!(
            r.tab(0).and_then(|e| e.comment().cloned()).as_deref(),
            Some("Şehre göre ciro")
        );
        assert_eq!(r.tab(2).and_then(|e| e.comment().cloned()), None);
        // The statement's second set shares its N.
        assert_eq!(r.tab(0).map(|e| e.number()), r.tab(1).map(|e| e.number()));
    });
    // The result run as one says it is untyped, the others nothing. Five tabs do not fit the
    // strip: the note takes its short form (NOTES ruling 30); alone it is whole.
    assert_eq!(t.text("results-tab-4-untyped").as_deref(), Some("untyped"));
    assert_eq!(t.text("results-tab-0-untyped"), None);
    in_window(t.vcx, |_, cx| {
        t.results.update(cx, |r, cx| r.close_others(4, cx))
    });
    t.redraw();
    t.redraw();
    assert_eq!(
        t.text("results-tab-0-untyped").as_deref(),
        Some("untyped (run as one)")
    );
    t.apply(&[
        result_tab(2, 0, 0, "Şehre göre ciro", finished(3)),
        result_tab(2, 0, 1, "Şehre göre ciro (2)", finished(2)),
        result_tab(2, 1, 0, "Result 2", finished(1)),
        result_tab(2, 2, 0, "Şehre göre ciro", finished(4)),
    ]);
    // A Turkish console numbers in Turkish; the comment stays as written.
    switch_to(t.vcx, "tr");
    t.redraw();
    assert_eq!(t.text("results-tab-1").as_deref(), Some("Şehre göre ciro"));
    let numbered = t.text("results-tab-3").expect("a numbered tab");
    assert!(!numbered.starts_with("Result"), "{numbered}");
}

/// A pinned tab is never reused: the next run of its statement opens a new tab beside it, and
/// the run after that reuses the new (unpinned) one.
#[gpui::test]
fn pinned_tab_is_never_reused(cx: &mut TestAppContext) {
    let t = tile(cx);
    let first = finished(3);
    t.apply(&[result_tab(1, 0, 0, "orders", first.clone())]);
    // The tab menu offers Pin; choosing it pins the tab and draws the pin glyph.
    let labels = |t: &Tile| -> Vec<String> {
        t.vcx.read(|cx| {
            pin::menu_items(t.console, 0, cx)
                .iter()
                .filter(|i| !i.separator)
                .map(|i| (i.label)(cx).to_string())
                .collect()
        })
    };
    assert_eq!(labels(&t), ["Pin tab", "Close tab", "Close other tabs"]);
    // A right press on the tab opens the menu.
    let at = t
        .vcx
        .debug_bounds("results-tab-0")
        .expect("the tab is drawn")
        .center();
    t.vcx.simulate_event(gpui::MouseDownEvent {
        button: gpui::MouseButton::Right,
        position: at,
        modifiers: gpui::Modifiers::none(),
        click_count: 1,
        first_mouse: false,
    });
    t.vcx.run_until_parked();
    assert_eq!(layer_ids(t.vcx).last(), Some(&"menu"));
    t.vcx.simulate_keystrokes("escape");
    t.vcx.run_until_parked();
    in_window(t.vcx, |_, cx| {
        pin::choose(t.console, 0, "results.tab_menu.pin", cx)
    });
    t.redraw();
    assert!(t.vcx.read(|cx| pin::is_pinned(t.console, 0, cx)));
    assert!(t.vcx.debug_bounds("results-tab-0-pin").is_some());
    assert_eq!(labels(&t), ["Unpin tab", "Close tab", "Close other tabs"]);

    // Run 2 of the same statement: a new tab; the pinned one keeps run 1's result.
    let second = finished(5);
    t.apply(&[result_tab(2, 0, 0, "orders", second.clone())]);
    let sources = |t: &Tile| -> Vec<Arc<dyn ResultSource>> {
        t.vcx.read(|cx| {
            t.results
                .read(cx)
                .tabs()
                .iter()
                .map(|e| e.source())
                .collect()
        })
    };
    let shown = sources(&t);
    assert_eq!(shown.len(), 2);
    assert!(Arc::ptr_eq(&shown[0], &first));
    assert!(Arc::ptr_eq(&shown[1], &second));
    assert_eq!(t.titles(), ["orders", "orders (2)"]);
    assert_eq!(
        t.vcx.read(|cx| t.results.read(cx).active()),
        ActiveResult::Tab(1)
    );
    // Run 3 reuses the unpinned tab.
    let third = finished(7);
    t.apply(&[result_tab(3, 0, 0, "orders", third.clone())]);
    let shown = sources(&t);
    assert_eq!(shown.len(), 2);
    assert!(
        Arc::ptr_eq(&shown[0], &first),
        "the pinned tab is untouched"
    );
    assert!(Arc::ptr_eq(&shown[1], &third));

    // Close other tabs from the pinned tab's menu leaves it alone.
    in_window(t.vcx, |_, cx| {
        pin::choose(t.console, 0, "results.tab_menu.close_others", cx)
    });
    t.redraw();
    let shown = sources(&t);
    assert_eq!(shown.len(), 1);
    assert!(Arc::ptr_eq(&shown[0], &first));
    // Unpinned, the next run reuses it.
    in_window(t.vcx, |_, cx| {
        pin::choose(t.console, 0, "results.tab_menu.unpin", cx)
    });
    let fourth = finished(1);
    t.apply(&[result_tab(4, 0, 0, "orders", fourth.clone())]);
    let shown = sources(&t);
    assert_eq!(shown.len(), 1);
    assert!(Arc::ptr_eq(&shown[0], &fourth));
}

// ---- Count rows ---------------------------------------------------------------------------------

/// A console over a FakeDriver session with its SQL context read.
struct Console {
    vcx: &'static mut VisualTestContext,
    id: ConsoleId,
    editor: Entity<hacivat_ui_editor::EditorView>,
}

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

fn three_rows(prefix: &'static str) -> FakeQuery {
    FakeQuery {
        prefix,
        sets: vec![FakeResultSet {
            columns: vec![(
                "n",
                LogicalType::Int {
                    bits: 64,
                    signed: true,
                },
            )],
            rows: FakeRows::Generated {
                count: 3,
                cell: |r, _| Value::Int(r as i64),
            },
        }],
        ..Default::default()
    }
}

fn pg_console(cx: &mut TestAppContext, queries: Vec<FakeQuery>) -> Console {
    let fake = FakeDriver::new(DbmsKind::Postgres)
        .tracks_transactions()
        .exec(probes("SHOW standard_conforming_strings", "on"))
        .queries(queries);
    let p = hacivat_session::profile::Profile {
        engine: DbmsKind::Postgres,
        ..test_profile("orders-dev", Some(EnvTag::Dev), "127.0.0.1")
    };
    let vcx = shell_with_connections(cx, "en");
    in_window(vcx, |window, _| window.activate_window());
    let profile = add_profile(vcx, p.clone());
    let rt = vcx.cx.read(|cx| {
        cx.global::<hacivat_app::connections::ConnectionsModel>()
            .core_runtime()
            .handle()
    });
    let (session, _fake, _side) =
        futures::executor::block_on(hacivat_session::testing::fake_session_with_side(
            rt,
            fake,
            FakeDriver::new(DbmsKind::Postgres),
            p,
        ));
    let id = register_test_console(vcx, profile, session);
    wait_until(vcx, "the console's SQL context", |vcx| {
        vcx.cx
            .read(|cx| hacivat_app::exec_ui::sql_context(id, cx) != SqlContext::Pending)
    });
    enable_registry(vcx);
    let editor = vcx.cx.read(|cx| {
        Consoles::get(cx, id)
            .and_then(|s| s.editor.clone())
            .expect("the console has an editor")
    });
    Console { vcx, id, editor }
}

impl Console {
    /// Runs `text` whole and waits for its first result tab.
    fn run(&self, text: &str) {
        let text = text.to_string();
        let len = text.len();
        self.vcx.cx.update(|cx| {
            self.editor.update(cx, |e, cx| {
                e.set_text(&text, cx);
                e.select_range(0..len, cx);
            });
        });
        self.vcx.run_until_parked();
        let editor = self.editor.clone();
        in_window(self.vcx, |window, cx| {
            editor.update(cx, |e, cx| assert!(e.run_command("editor.run", window, cx)));
        });
        wait_until(self.vcx, "the result tab", |vcx| {
            vcx.cx.read(|cx| {
                Consoles::get(cx, self.id)
                    .and_then(|s| s.results.clone())
                    .is_some_and(|r| !r.read(cx).tabs().is_empty())
            })
        });
    }

    fn count(&self) -> CountState {
        self.vcx.cx.read(|cx| {
            Consoles::get(cx, self.id)
                .and_then(|s| s.results.clone())
                .and_then(|r| r.read(cx).tab(0).map(|t| t.count()))
                .expect("a tab")
        })
    }

    fn redraw(&self) {
        in_window(self.vcx, |window, _| window.refresh());
        self.vcx.run_until_parked();
    }
}

/// The counts the test counter was asked for, the answer it holds, and its cancels.
#[derive(Default)]
struct Counted {
    jobs: Vec<CountJob>,
    answer: Option<tokio::sync::oneshot::Sender<Result<u64, CountError>>>,
    cancels: u32,
}

/// R-M3-76: Count rows sends `SELECT COUNT(*) FROM (\n<q>\n) hv_c` (a separate query of its
/// own), shows "Counting…" with Cancel while it runs, then the exact count; Cancel goes back
/// to the button and an answer after it is dropped; a statement that can't be counted says so
/// and sends nothing.
#[gpui::test]
fn count_rows_shows_exact_and_can_be_cancelled(cx: &mut TestAppContext) {
    let c = pg_console(
        cx,
        vec![
            three_rows("SELECT * FROM orders"),
            three_rows("SELECT id FROM orders FOR UPDATE"),
        ],
    );
    let counted = Rc::new(RefCell::new(Counted::default()));
    let hook = counted.clone();
    c.vcx.cx.update(|cx| {
        count::set_counter(
            cx,
            Rc::new(move |job: CountJob, _cx: &mut gpui::App| {
                let (tx, done) = tokio::sync::oneshot::channel();
                let mut h = hook.borrow_mut();
                h.jobs.push(job);
                h.answer = Some(tx);
                let cancels = hook.clone();
                Ok(CountHandle {
                    done,
                    cancel: Rc::new(move || cancels.borrow_mut().cancels += 1),
                })
            }),
        );
    });
    c.run("SELECT * FROM orders ORDER BY id;");
    c.redraw();
    assert_eq!(
        text_of(c.vcx, "button-label-results-count").as_deref(),
        Some("Count rows")
    );
    click(&mut *c.vcx, "button-results-count");
    c.redraw();
    {
        let h = counted.borrow();
        assert_eq!(h.jobs.len(), 1);
        // The trailing `;` and the top-level ORDER BY are gone.
        assert_eq!(
            h.jobs[0].sql,
            "SELECT COUNT(*) FROM (\nSELECT * FROM orders\n) hv_c"
        );
        assert_eq!(h.jobs[0].engine, DbmsKind::Postgres);
        assert_eq!(h.jobs[0].console, c.id);
    }
    assert!(matches!(c.count(), CountState::Counting { .. }));
    assert_eq!(
        text_of(c.vcx, "results-count-state").as_deref(),
        Some("Counting…")
    );
    assert_eq!(
        text_of(c.vcx, "button-label-results-count-cancel").as_deref(),
        Some("Cancel")
    );
    // The server's count.
    let answer = counted.borrow_mut().answer.take().expect("the count waits");
    answer.send(Ok(1_048_213)).unwrap();
    wait_until(c.vcx, "the count", |_| {
        c.count() == CountState::Exact(1_048_213)
    });
    c.redraw();
    assert_eq!(
        text_of(c.vcx, "results-count-state").as_deref(),
        Some("1,048,213 rows (exact)")
    );

    // Again, and Cancel: back to the button; the late answer changes nothing.
    click(&mut *c.vcx, "button-results-count");
    c.redraw();
    assert!(matches!(c.count(), CountState::Counting { .. }));
    click(&mut *c.vcx, "button-results-count-cancel");
    c.redraw();
    assert_eq!(counted.borrow().cancels, 1);
    assert_eq!(c.count(), CountState::Idle);
    assert_eq!(text_of(c.vcx, "results-count-state"), None);
    let late = counted
        .borrow_mut()
        .answer
        .take()
        .expect("the second count");
    let _ = late.send(Ok(7));
    c.vcx.run_until_parked();
    assert_eq!(c.count(), CountState::Idle);

    // A count the read-only check refuses says so.
    click(&mut *c.vcx, "button-results-count");
    let refused = counted.borrow_mut().answer.take().expect("the third count");
    let _ = refused.send(Err(CountError::TimedOut));
    wait_until(c.vcx, "the time-out", |_| {
        c.count() == CountState::Refused("hv-results-count-timeout")
    });
    c.redraw();
    assert_eq!(
        text_of(c.vcx, "results-count-state").as_deref(),
        Some("Stopped after 30 seconds.")
    );

    // A locking SELECT can't be counted: nothing is sent.
    c.run("SELECT id FROM orders FOR UPDATE");
    wait_until(c.vcx, "the locking result", |_| {
        c.count() == CountState::Idle
    });
    click(&mut *c.vcx, "button-results-count");
    c.redraw();
    assert_eq!(counted.borrow().jobs.len(), 3, "nothing more was asked");
    assert_eq!(
        c.count(),
        CountState::Refused("hv-results-count-unavailable")
    );
    assert_eq!(
        text_of(c.vcx, "results-count-state").as_deref(),
        Some("This query can't be counted.")
    );
    stop_core_runtime_test(cx);
}

// ---- notices and chips ----------------------------------------------------------------------------

/// R-M3-77 (NOTES ruling 23, cross-cutting 10): a notice is its own row: the label (Fluent),
/// the code chip and the server's text verbatim in its own field, never inside the sentence;
/// a warning has the triangle and a 600 label; the flood line counts what was not shown.
#[gpui::test]
fn notice_line_keeps_server_text_verbatim(cx: &mut TestAppContext) {
    let t = tile(cx);
    let text = "relation \"siparişler\" already exists, skipping {$n} %s";
    t.log.append(
        LogMsg::Notice {
            severity: NoticeSeverity::Notice,
            code: Some(Arc::from("42P07")),
            text: Arc::from(text),
        },
        None,
        None,
    );
    t.log.append(
        LogMsg::Notice {
            severity: NoticeSeverity::Warning,
            code: Some(Arc::from("01000")),
            text: Arc::from("there is no transaction in progress"),
        },
        None,
        None,
    );
    t.log.append(LogMsg::NoticesDropped { n: 97 }, None, None);
    t.redraw();
    assert_eq!(t.text("log-sentence-0").as_deref(), Some("Server notice"));
    assert_eq!(t.text("log-code-0").as_deref(), Some("42P07"));
    assert_eq!(t.text("log-notice-0").as_deref(), Some(text));
    assert_eq!(t.text("log-sentence-1").as_deref(), Some("Server warning"));
    assert_eq!(
        t.text("log-notice-1").as_deref(),
        Some("there is no transaction in progress")
    );
    assert_eq!(
        t.text("log-sentence-2").as_deref(),
        Some("97 more server notices were not shown.")
    );
    // The server's text is in no translated sentence.
    let frame = registry_frame(t.vcx);
    let sentences: Vec<&str> = frame
        .iter()
        .filter(|r| r.path.contains("#log-sentence-"))
        .map(|r| r.text.as_str())
        .collect();
    assert!(
        sentences.iter().all(|s| !s.contains("siparişler")),
        "{sentences:?}"
    );
    // The notice's text is data (never pseudo-localised); the label is Fluent.
    let notice = frame
        .iter()
        .find(|r| r.path.ends_with("#log-notice-0"))
        .expect("the notice's text is drawn");
    assert_eq!(notice.source, hacivat_ui::text::TextSource::Data);
}

/// Ledger 340: a MySQL error's chip shows the vendor code beside the SQLSTATE, as the server's
/// own client does, while the view's code stays the SQLSTATE (what the bypass rows judge by);
/// PostgreSQL's chip is the SQLSTATE alone; a MySQL warning's chip is its vendor code.
#[gpui::test]
fn mysql_chip_shows_vendor_code_and_sqlstate(cx: &mut TestAppContext) {
    let server = |sqlstate: Option<&str>, vendor: Option<i64>| ExecError {
        kind: ExecErrorKind::Db,
        server: Some(ServerMessage {
            sqlstate: sqlstate.map(str::to_string),
            vendor_code: vendor,
            severity: None,
            text: "Cannot execute statement in a READ ONLY transaction.".into(),
            detail: None,
            hint: None,
            lang_hint: None,
            position: None,
        }),
    };
    let mysql = error_view(
        DbmsKind::MySql,
        &server(Some("25006"), Some(1792)),
        "x",
        0,
        None,
    );
    assert_eq!(mysql.code.as_deref(), Some("25006"));
    assert_eq!(mysql.vendor_code.as_deref(), Some("1792"));
    let pg = error_view(
        DbmsKind::Postgres,
        &server(Some("42P01"), None),
        "x",
        0,
        None,
    );
    assert_eq!(pg.vendor_code, None);
    // SQLite: the vendor code is the code; nothing beside it.
    let sqlite = error_view(DbmsKind::Sqlite, &server(None, Some(5)), "x", 0, None);
    assert_eq!(sqlite.code.as_deref(), Some("5"));
    assert_eq!(sqlite.vendor_code, None);

    let t = tile(cx);
    t.log
        .append(LogMsg::Error { n: 2, view: mysql }, Some(0..1), None);
    t.log
        .append(LogMsg::Error { n: 3, view: pg }, Some(0..1), None);
    t.log.append(
        LogMsg::Notice {
            severity: NoticeSeverity::Warning,
            code: Some(Arc::from("1292")),
            text: Arc::from("Truncated incorrect DOUBLE value: 'İzmir'"),
        },
        None,
        None,
    );
    t.redraw();
    assert_eq!(t.text("log-code-0").as_deref(), Some("1792 · 25006"));
    assert_eq!(t.text("log-code-1").as_deref(), Some("42P01"));
    assert_eq!(t.text("log-code-2").as_deref(), Some("1292"));
    let chip = |view: ErrorView| code_chip(&LogMsg::Error { n: 1, view });
    assert_eq!(
        chip(ErrorView {
            title_id: "exec-error-server",
            code: None,
            server_text: None,
            hint_id: None,
            doc_offset: None,
            vendor_code: Some("1792".into()),
        }),
        None,
        "no SQLSTATE, no chip"
    );
}

// ---- eviction -------------------------------------------------------------------------------------

/// A finished store on `budget` of `rows` in two chunks (500, then the rest), as a stream lands.
fn two_chunks(budget: &Arc<MemoryBudget>, id: u64, rows_n: u64) -> Arc<ResultStore> {
    let schema = schema(2);
    let (store, mut writer) = ResultStore::new(ResultId(id), schema.clone(), budget.clone(), 1);
    writer.append(Chunk::from_values(&schema, rows(0..500, 2)));
    writer.append(Chunk::from_values(&schema, rows(500..rows_n, 2)));
    writer.set_state(Completeness::Complete);
    drop(writer);
    store
}

/// R-M3-68 and the Task 8 carry: a background tab, hidden past its grace, is evicted to its
/// first chunk; shown again its strip says "Closed to free memory; the first 500 rows are
/// kept." with Re-run, its selection (which reached past the kept rows) is gone, and a copy
/// past the kept rows is refused with a notice, never clamped. The active tab is never evicted.
#[gpui::test]
fn evicted_result_offers_rerun(cx: &mut TestAppContext) {
    let t = tile(cx);
    let budget = MemoryBudget::new(u64::MAX, u64::MAX);
    let shown = two_chunks(&budget, 9_001, 1_500);
    let background = two_chunks(&budget, 9_002, 1_500);
    let source = |s: &Arc<ResultStore>| -> Arc<dyn ResultSource> {
        StoreSource::new(
            s.clone(),
            Arc::new(Control::default()),
            FetchPolicy::default(),
        )
    };
    t.apply(&[
        result_tab(1, 0, 0, "", source(&shown)),
        result_tab(1, 1, 0, "", source(&background)),
    ]);
    // Select every row of the background tab while it is shown, then leave it.
    t.set_active(ActiveResult::Tab(1));
    let grid = t
        .vcx
        .read(|cx| t.host().read(cx).grid_of(t.console, 1, cx))
        .expect("the grid is built");
    in_window(t.vcx, |window, cx| {
        grid.update(cx, |g, cx| g.dispatch(GridCommand::SelectAll, window, cx))
    });
    let cells = |t: &Tile| {
        t.vcx.read(|cx| {
            let g = grid.read(cx);
            g.selection().cell_count(1_500, g.column_layout().visible())
        })
    };
    assert_eq!(cells(&t), 3_000, "every row is selected");
    t.set_active(ActiveResult::Tab(0));
    // Within its grace the result is still visible to eviction; after it, it is not.
    t.advance(HIDE_GRACE - Duration::from_secs(1));
    assert_eq!(budget.evict_for(u64::MAX), 0, "nothing is hidden yet");
    t.advance(Duration::from_secs(2));
    assert!(budget.evict_for(u64::MAX) > 0);
    assert!(matches!(
        shown.snapshot().completeness(),
        Completeness::Complete
    ));
    assert_eq!(background.snapshot().row_count(), 500);

    let events = t.events();
    t.set_active(ActiveResult::Tab(1));
    assert_eq!(
        t.text("strip-sentence").as_deref(),
        Some("Closed to free memory; the first 500 rows are kept.")
    );
    // The range over rows the result no longer keeps is gone: only the focus cell is left.
    assert_eq!(cells(&t), 1);
    // A copy cut before the eviction, past the kept rows, copies nothing and says why.
    let generation = t.vcx.read(|cx| {
        t.results
            .read(cx)
            .tab(1)
            .map(|e| e.adapter().generation())
            .expect("the tab")
    });
    t.vcx.cx.update(|cx| {
        grid.update(cx, |_, cx| {
            cx.emit(GridEvent::CopyRequested {
                region: hacivat_ui_grid::model::CopyRegion {
                    rows: std::iter::once(0..1_500).collect(),
                    cols: vec![hacivat_ui_grid::ColId(0)],
                },
                headers: false,
                generation,
            })
        })
    });
    t.vcx.run_until_parked();
    assert!(
        toast_ids(t.vcx)
            .iter()
            .any(|id| id == "hv-results-copy-evicted")
    );
    assert_eq!(t.vcx.read_from_clipboard().and_then(|c| c.text()), None);

    let mut t = t;
    t.click("button-strip-rerun");
    assert_eq!(
        events.borrow().as_slice(),
        [ResultsEvent::Rerun {
            console: t.console,
            unit_ordinal: 1
        }]
    );
}

/// R-M3-68: Fetch all asks first when the exact count says the rows would pass the result's
/// memory limit: "Fetch up to the memory limit" (focused) fetches, Cancel fetches nothing.
/// Without a count, or a count that fits, Fetch all fetches at once.
#[gpui::test]
fn fetch_all_past_the_cap_asks_first(cx: &mut TestAppContext) {
    let mut t = tile(cx);
    let control = Arc::new(Control::default());
    let store = ResultStore::from_rows(
        schema(2),
        rows(0..500, 2),
        Completeness::Suspended(cursor()),
    );
    let per_row = store.snapshot().bytes() / 500;
    let source: Arc<dyn ResultSource> =
        StoreSource::new(store, control.clone(), FetchPolicy::default());
    t.apply(&[result_tab(1, 0, 0, "", source)]);
    let fetched = || *control.fetch_alls.lock().unwrap();
    let set_count = |t: &Tile, state| {
        in_window(t.vcx, |_, cx| {
            t.results.update(cx, |r, cx| r.set_count(0, state, cx))
        });
        t.redraw();
    };
    // No count: at once.
    t.click("button-strip-fetch-all");
    assert_eq!(fetched(), 1);
    assert!(!layer_ids(t.vcx).contains(&"dialog"));
    // A count that fits: at once.
    set_count(&t, CountState::Exact(1_000));
    t.click("button-strip-fetch-all");
    assert_eq!(fetched(), 2);
    // A count past the 256 MiB cap: the question first.
    let past = (hacivat_results::budget::PER_RESULT_CAP / per_row.max(1)) * 2;
    set_count(&t, CountState::Exact(past));
    t.click("button-strip-fetch-all");
    assert_eq!(fetched(), 2, "nothing is fetched before the answer");
    assert_eq!(layer_ids(t.vcx).last(), Some(&"dialog"));
    assert_eq!(
        dialog_title(t.vcx),
        "Fetching every row would pass this result's memory limit"
    );
    let buttons = button_labels(t.vcx);
    assert!(
        buttons.iter().any(|b| b == "Fetch up to the memory limit"),
        "{buttons:?}"
    );
    // Escape (Cancel) fetches nothing.
    t.vcx.simulate_keystrokes("escape");
    t.vcx.run_until_parked();
    assert_eq!(fetched(), 2);
    assert!(!layer_ids(t.vcx).contains(&"dialog"));
    // Enter on the focused bounded fetch fetches (the cap pauses it, the cursor stays open).
    t.click("button-strip-fetch-all");
    t.vcx.simulate_keystrokes("enter");
    t.vcx.run_until_parked();
    assert_eq!(fetched(), 3);
    // The palette's Fetch all asks too.
    assert!(run(t.vcx, "results.fetch_all"));
    t.vcx.run_until_parked();
    assert_eq!(layer_ids(t.vcx).last(), Some(&"dialog"));
    assert_eq!(fetched(), 3);
}

/// R-M3-68, the Task 8 carries: every result tab reports its visibility, background tabs
/// included: the active tab at once; a tab out of sight only after its 30 s grace; a pinned tab
/// or one another view shows is never hidden.
#[gpui::test]
fn active_tabs_are_marked_visible(cx: &mut TestAppContext) {
    let t = tile(cx);
    let store = || ResultStore::from_rows(schema(1), rows(0..3, 1), Completeness::Complete);
    let (a, b) = (Spy::over(store()), Spy::over(store()));
    t.apply(&[
        result_tab(1, 0, 0, "", a.clone()),
        result_tab(1, 1, 0, "", b.clone()),
    ]);
    assert_eq!(a.seen(), [true], "the active tab is visible at once");
    assert_eq!(
        b.seen(),
        Vec::<bool>::new(),
        "a background tab has its grace"
    );
    t.advance(HIDE_GRACE - Duration::from_secs(1));
    assert_eq!(b.seen(), Vec::<bool>::new());
    t.advance(Duration::from_secs(2));
    assert_eq!(b.seen(), [false], "hidden once its grace ran out");

    // Switching: the new active tab at once, the old one after its grace.
    t.set_active(ActiveResult::Tab(1));
    assert_eq!(b.seen(), [false, true]);
    t.advance(HIDE_GRACE / 2);
    // Back within the grace: never hidden.
    t.set_active(ActiveResult::Tab(0));
    t.advance(HIDE_GRACE * 2);
    assert_eq!(a.seen(), [true], "shown again within its grace");
    assert_eq!(b.seen(), [false, true, false]);

    // Pinned: visible, whatever is active.
    in_window(t.vcx, |_, cx| {
        t.results.update(cx, |r, cx| r.set_pinned(1, true, cx))
    });
    assert_eq!(b.seen(), [false, true, false, true]);
    t.advance(HIDE_GRACE * 2);
    assert_eq!(b.seen(), [false, true, false, true]);
    // Shown by another view (Task 38's): visible until no view shows it.
    in_window(t.vcx, |_, cx| {
        t.results.update(cx, |r, cx| {
            r.set_pinned(1, false, cx);
            r.set_view_visible(1, 7, true, cx);
        })
    });
    t.advance(HIDE_GRACE * 2);
    assert_eq!(b.seen(), [false, true, false, true]);
    in_window(t.vcx, |_, cx| {
        t.results
            .update(cx, |r, cx| r.set_view_visible(1, 7, false, cx))
    });
    t.advance(HIDE_GRACE * 2);
    assert_eq!(b.seen(), [false, true, false, true, false]);
    // The Output tab shows no result: the active result tab is hidden after its grace.
    t.set_active(ActiveResult::Output);
    t.advance(HIDE_GRACE * 2);
    assert_eq!(a.seen(), [true, false]);
}

// ---- the log view -----------------------------------------------------------------------------------

/// Ledger 336/342/362: a change of the log costs its new lines: the view reads only the lines
/// after the last it took, keeps as many as the ring, counts lines the ring dropped unread,
/// and empties on Clear.
#[gpui::test]
fn output_view_reads_only_new_lines(cx: &mut TestAppContext) {
    let t = tile(cx);
    for ms in 0..1_000 {
        t.log.append(LogMsg::Completed { ms }, None, None);
    }
    t.redraw();
    let list = t
        .vcx
        .read(|cx| t.host().read(cx).log_list(t.console))
        .expect("the log list");
    let rows = |t: &Tile| {
        t.vcx.read(|cx| {
            (
                list.read(cx).rows().taken(),
                list.read(cx).rows().line_count(),
            )
        })
    };
    assert_eq!(rows(&t).1, 1_000);
    t.log.append(LogMsg::Completed { ms: 1_000 }, None, None);
    t.redraw();
    assert_eq!(rows(&t), (1, 1_001), "one new line, one line read");
    for ms in 0..3 {
        t.log.append(LogMsg::Completed { ms }, None, None);
    }
    t.redraw();
    assert_eq!(rows(&t), (3, 1_004));
    assert_eq!(
        t.text("log-sentence-0").as_deref(),
        None,
        "only the visible rows (the newest) are built"
    );
    // A burst past the ring: the view keeps as many lines as the ring and says how many it
    // never showed.
    let burst = SessionLog::CAPACITY as u64 + 250;
    for ms in 0..burst {
        t.log.append(LogMsg::Completed { ms }, None, None);
    }
    t.redraw();
    let (taken, shown) = rows(&t);
    assert_eq!(taken, SessionLog::CAPACITY as u64);
    assert_eq!(shown, SessionLog::CAPACITY);
    let first = t.vcx.read(|cx| list.read(cx).rows().lines()[0].msg.clone());
    assert_eq!(first, LogMsg::Dropped { n: 250 });
    // Clear empties the view.
    t.log.clear();
    t.redraw();
    assert_eq!(rows(&t).1, 0);
    assert!(t.vcx.read(|cx| list.read(cx).rows().is_empty()));
    t.log.append(LogMsg::Completed { ms: 5 }, None, None);
    t.redraw();
    assert_eq!(rows(&t), (1, 1));
}

/// ↑/↓ move the log's selected line while the log has the focus; Enter goes to its statement.
#[gpui::test]
fn log_lines_navigate_with_arrows(cx: &mut TestAppContext) {
    let mut t = tile(cx);
    t.log.append(LogMsg::Completed { ms: 1 }, Some(0..8), None);
    t.log
        .append(LogMsg::RowsAffected { rows: 2, ms: 3 }, Some(10..30), None);
    t.log.append(LogMsg::TxCommitted, None, None);
    t.redraw();
    let list = t
        .vcx
        .read(|cx| t.host().read(cx).log_list(t.console))
        .expect("the log list");
    let selected = |t: &Tile| t.vcx.read(|cx| list.read(cx).rows().selected());
    let events = t.events();
    // A click selects a line and gives the log the focus.
    t.click("log-row-0");
    assert_eq!(selected(&t), Some(0));
    t.vcx.simulate_keystrokes("down");
    t.vcx.run_until_parked();
    assert_eq!(selected(&t), Some(1));
    t.vcx.simulate_keystrokes("down down");
    t.vcx.run_until_parked();
    assert_eq!(selected(&t), Some(2), "the last line stays selected");
    t.vcx.simulate_keystrokes("up");
    t.vcx.run_until_parked();
    assert_eq!(selected(&t), Some(1));
    t.vcx.simulate_keystrokes("enter");
    t.vcx.run_until_parked();
    assert_eq!(
        events.borrow().as_slice(),
        [ResultsEvent::GoToSource {
            console: t.console,
            range: 10..30
        }]
    );
    // A line without a statement goes nowhere.
    t.vcx.simulate_keystrokes("down enter");
    t.vcx.run_until_parked();
    assert_eq!(events.borrow().len(), 1);
}

/// `results.next_tab`/`prev_tab` step through Output and the result tabs, wrapping.
#[gpui::test]
fn next_and_previous_tab_wrap(cx: &mut TestAppContext) {
    let t = tile(cx);
    t.apply(&[
        result_tab(1, 0, 0, "", finished(1)),
        result_tab(1, 1, 0, "", finished(1)),
    ]);
    let active = |t: &Tile| t.vcx.read(|cx| t.results.read(cx).active());
    assert_eq!(active(&t), ActiveResult::Tab(0));
    assert!(run(t.vcx, "results.next_tab"));
    assert_eq!(active(&t), ActiveResult::Tab(1));
    assert!(run(t.vcx, "results.next_tab"));
    assert_eq!(active(&t), ActiveResult::Output);
    assert!(run(t.vcx, "results.prev_tab"));
    assert_eq!(active(&t), ActiveResult::Tab(1));
}

/// R-M3-82: the run's cell cap is `results.max_value_bytes` (1 MiB by default).
#[gpui::test]
fn the_cell_cap_comes_from_the_value_size_setting(cx: &mut TestAppContext) {
    let vcx = shell(cx, "en");
    let cap = |vcx: &VisualTestContext| {
        vcx.cx
            .read(|cx| hacivat_app::exec_ui::fetch_policy_for(ConsoleId(1), cx).cell_cap)
    };
    assert_eq!(cap(vcx), 1 << 20);
    vcx.cx.update(|cx| {
        hacivat_app::shell::update_settings(cx, |s| {
            s.results.max_value_bytes = hacivat_settings::ValueLimit::new(64 << 10).unwrap();
        });
    });
    assert_eq!(cap(vcx), 64 << 10);
}
