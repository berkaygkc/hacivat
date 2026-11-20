//! M3a Task 30: the results tile (HV-EXEC-07 UI): the M1 empty state until a console runs, the
//! Output tab and one tab per result, the result and Output toolbars, the cursor strip per
//! completeness with the memory banner (R-M3-33), the 2,000-column hint, and the output log
//! view (HV-EXEC-11) on `VirtualList`.
//!
//! The consoles are registered straight into the registry over `ResultStore::from_rows`
//! fixtures (R-M3-15): the tile needs a console's results and log, not a live session.

#![cfg_attr(not(unix), allow(dead_code, unused_imports, unused_variables))]

mod support;

use std::cell::RefCell;
use std::ops::Range;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use gpui::{AppContext as _, Entity, Focusable as _, TestAppContext, VisualTestContext};
use hacivat_app::console::{ConsoleId, ConsoleState, Consoles};
use hacivat_app::results::strip::{StripFacts, StripModel};
use hacivat_app::results::view::{ResultsHost, hosts};
use hacivat_app::results::{ActiveResult, ConsoleResults, ResultsEvent};
use hacivat_exec::{ErrorView, LogMsg, ResultTab, SessionLog, TabKey};
use hacivat_proto::EnvTag;
use hacivat_results::{
    CloseReason, Completeness, CursorInfo, FetchControl, FetchPolicy, PauseReason, ResultId,
    ResultSource, ResultStore, StoreSource, TruncReason,
};
use hacivat_session::session::{ClientId, RunId, UnitId};
use hacivat_ui_grid::GridView;
use hacivat_ui_grid::RowRenderer as _;
use hacivat_values::{ColumnMeta, LogicalType, NativeType, ResultSchema, Value};
use support::*;

// ---- fixtures -------------------------------------------------------------------------------

fn column(name: &str) -> ColumnMeta {
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

fn schema(cols: usize) -> Arc<ResultSchema> {
    Arc::new(ResultSchema {
        columns: (0..cols).map(|c| column(&format!("c{c}"))).collect(),
    })
}

fn rows(n: u64, cols: usize) -> Vec<Vec<Value>> {
    (0..n)
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

/// What a result source told its session.
#[derive(Default)]
struct Control {
    fetches: Mutex<Vec<u64>>,
    fetch_alls: Mutex<u32>,
    closes: Mutex<Vec<CloseReason>>,
}

impl FetchControl for Control {
    fn fetch_more(&self, _result: ResultId, rows: u64) {
        self.fetches.lock().unwrap().push(rows);
    }
    fn close(&self, _result: ResultId, reason: CloseReason) {
        self.closes.lock().unwrap().push(reason);
    }
    fn fetch_all(&self, _result: ResultId) {
        *self.fetch_alls.lock().unwrap() += 1;
    }
}

fn source(store: Arc<ResultStore>, control: Arc<Control>) -> Arc<dyn ResultSource> {
    StoreSource::new(store, control, FetchPolicy::default())
}

fn store_of(n: u64, cols: usize, c: Completeness) -> Arc<ResultStore> {
    ResultStore::from_rows(schema(cols), rows(n, cols), c)
}

fn unit(ordinal: u32) -> UnitId {
    UnitId {
        run: RunId(1),
        ordinal,
    }
}

fn tab(ordinal: u32, source: Arc<dyn ResultSource>) -> ResultTab {
    ResultTab {
        key: TabKey {
            console: 1,
            unit_ordinal: ordinal,
            result_ordinal: 0,
        },
        title: "".into(),
        unit: unit(ordinal),
        source,
    }
}

/// A shell whose focused pane shows a console bound to `orders-dev` (in `env`), with empty
/// results and log, and the text registry on.
struct Tile {
    vcx: &'static mut VisualTestContext,
    console: ConsoleId,
    results: Entity<ConsoleResults>,
    log: Arc<SessionLog>,
}

fn tile(cx: &mut TestAppContext, lang: &str, env: Option<EnvTag>) -> Tile {
    let vcx = shell_with_connections(cx, lang);
    let profile = add_profile(vcx, test_profile("orders-dev", env, "10.0.0.5"));
    let (console, results, log) = add_console(vcx, profile);
    enable_registry(vcx);
    vcx.run_until_parked();
    Tile {
        vcx,
        console,
        results,
        log,
    }
}

/// Opens a console tab bound to `profile` in the focused pane (it becomes the active tab) and
/// registers a console with empty results and log for it.
fn add_console(
    vcx: &VisualTestContext,
    profile: hacivat_proto::ProfileId,
) -> (ConsoleId, Entity<ConsoleResults>, Arc<SessionLog>) {
    let ws = workspace(vcx);
    let out = in_window(vcx, |window, cx| {
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
    vcx.run_until_parked();
    out
}

/// A finished one-column result holding `values`.
fn store_values(values: &[i64]) -> Arc<ResultStore> {
    ResultStore::from_rows(
        schema(1),
        values.iter().map(|v| vec![Value::Int(*v)]).collect(),
        Completeness::Complete,
    )
}

/// Applies `tabs` to `results` and redraws.
fn apply_to(vcx: &VisualTestContext, results: &Entity<ConsoleResults>, tabs: &[ResultTab]) {
    in_window(vcx, |_, cx| {
        results.update(cx, |r, cx| {
            r.apply(tabs, cx);
        })
    });
    in_window(vcx, |window, _| window.refresh());
    vcx.run_until_parked();
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
        text_of(self.vcx, id)
    }

    fn click(&mut self, selector: &'static str) {
        click(self.vcx, selector);
        self.redraw();
    }
}

// ---- the tile -------------------------------------------------------------------------------

#[gpui::test]
fn empty_tile_is_the_m1_state_until_a_run(cx: &mut TestAppContext) {
    let t = tile(cx, "en", None);
    let markers = rendered_markers(t.vcx);
    assert!(
        markers.iter().any(|m| m == "results-empty"),
        "the M1 tile: {markers:?}"
    );
    assert_eq!(t.text("results-tab-output"), None);

    // The first line of output is a run: the Output tab shows it.
    t.log.append(LogMsg::Completed { ms: 12 }, None, None);
    t.redraw();
    let markers = rendered_markers(t.vcx);
    assert!(!markers.iter().any(|m| m == "results-empty"), "{markers:?}");
    assert_eq!(t.text("results-tab-output").as_deref(), Some("Output"));
    assert_eq!(
        t.text("log-sentence-0").as_deref(),
        Some("Completed in 12 ms")
    );
}

#[gpui::test]
fn switching_tabs_swaps_the_strip_and_toolbar(cx: &mut TestAppContext) {
    let t = tile(cx, "en", Some(EnvTag::Staging));
    let control = Arc::new(Control::default());
    t.apply(&[
        tab(
            1,
            source(store_of(3, 2, Completeness::Complete), control.clone()),
        ),
        tab(
            2,
            source(
                store_of(10, 2, Completeness::Suspended(cursor())),
                control.clone(),
            ),
        ),
    ]);
    // The run's first result is active: complete, three rows.
    assert_eq!(t.text("results-tab-0").as_deref(), Some("Result 1"));
    assert_eq!(t.text("results-tab-env-0").as_deref(), Some("(staging)"));
    assert_eq!(t.text("strip-sentence").as_deref(), Some("3 rows"));
    assert_eq!(t.text("results-rows-timing").as_deref(), Some("3 rows"));
    assert_eq!(t.text("strip-cursor"), None);

    t.set_active(ActiveResult::Tab(1));
    assert_eq!(t.text("strip-sentence").as_deref(), Some("10+ rows"));
    assert_eq!(t.text("results-rows-timing").as_deref(), Some("10+ rows"));
    assert_eq!(
        t.text("strip-cursor").as_deref(),
        Some("Cursor open — holding a database snapshot")
    );

    // The Output tab has its own toolbar and no strip.
    t.set_active(ActiveResult::Output);
    assert_eq!(t.text("strip-sentence"), None);
    assert_eq!(t.text("results-rows-timing"), None);
    assert_eq!(
        t.text("button-label-results-log-clear").as_deref(),
        Some("Clear")
    );
}

/// Each completeness and the sentence the strip leads with.
fn strip_cases() -> Vec<(Completeness, &'static str)> {
    vec![
        (Completeness::Streaming, "hv-strip-fetching"),
        (Completeness::Suspended(cursor()), "hv-strip-rows-more"),
        (Completeness::Complete, "hv-strip-complete"),
        (
            Completeness::Truncated(TruncReason::FollowingResultSets),
            "hv-strip-complete",
        ),
        (
            Completeness::Truncated(TruncReason::ResultCap),
            "hv-strip-truncated",
        ),
        (Completeness::Cancelled, "hv-strip-cancelled"),
        (
            Completeness::Closed {
                reason: CloseReason::SessionNeeded,
            },
            "cursor-closed-session-needed",
        ),
        (
            Completeness::Closed {
                reason: CloseReason::ConnectionLost,
            },
            "cursor-closed-connection-lost",
        ),
    ]
}

fn strip_id(c: Completeness) -> &'static str {
    let store = store_of(4, 1, c);
    StripModel::of(&store.snapshot(), None, StripFacts::default()).sentence_id()
}

#[gpui::test]
fn strip_text_per_completeness(_cx: &mut TestAppContext) {
    for (c, id) in strip_cases() {
        assert_eq!(strip_id(c), id, "{c:?}");
    }
    // A paused result is the memory banner.
    let paused =
        ResultStore::from_rows_paused(schema(1), rows(4, 1), cursor(), PauseReason::ResultCap);
    let banner = Some(hacivat_app::results::tabs::MemoryBanner {
        reason: PauseReason::ResultCap,
        can_raise: true,
    });
    assert_eq!(
        StripModel::of(&paused.snapshot(), banner, StripFacts::default()).sentence_id(),
        "hv-strip-memory"
    );
    // The log's close line says whether Hacivat's transaction was rolled back.
    let log = SessionLog::new();
    log.append(
        LogMsg::CursorClosed {
            rows: 4,
            reason: CloseReason::SessionNeeded,
            rolled_back: true,
            idle_ms: 0,
        },
        None,
        Some(unit(1)),
    );
    let closed = store_of(
        4,
        1,
        Completeness::Closed {
            reason: CloseReason::SessionNeeded,
        },
    );
    assert_eq!(
        StripModel::of(
            &closed.snapshot(),
            None,
            StripFacts::from_log(&log.lines(), unit(1))
        )
        .sentence_id(),
        "cursor-closed-session-needed-rolled-back"
    );
}

#[gpui::test]
fn the_strip_renders_its_sentence_and_more_result_sets(cx: &mut TestAppContext) {
    let t = tile(cx, "en", None);
    let control = Arc::new(Control::default());
    t.log
        .append(LogMsg::MoreResultSets { skipped: 2 }, None, Some(unit(1)));
    t.apply(&[tab(
        1,
        source(
            store_of(
                5,
                1,
                Completeness::Truncated(TruncReason::FollowingResultSets),
            ),
            control,
        ),
    )]);
    assert_eq!(t.text("strip-sentence").as_deref(), Some("5 rows"));
    assert_eq!(
        t.text("strip-more-sets").as_deref(),
        Some("2 more result sets not shown")
    );
}

#[gpui::test]
fn memory_banner_actions_reach_the_source(cx: &mut TestAppContext) {
    let mut t = tile(cx, "en", None);
    let control = Arc::new(Control::default());
    let paused =
        ResultStore::from_rows_paused(schema(2), rows(20, 2), cursor(), PauseReason::GlobalBudget);
    t.apply(&[tab(1, source(paused, control.clone()))]);
    let sentence = t.text("strip-sentence").expect("the banner");
    assert!(
        sentence.starts_with("Memory limit for this result reached (20 rows, "),
        "{sentence}"
    );

    // Fetch more anyway: the cap rises through ConsoleResults::raise_cap and the source
    // fetches on; after a global-budget pause there is nothing further to raise.
    t.click("button-strip-fetch-anyway");
    assert_eq!(*control.fetch_alls.lock().unwrap(), 1);
    let banner = t.vcx.read(|cx| {
        t.results
            .read(cx)
            .tab(0)
            .and_then(|tab| tab.memory_banner())
    });
    assert_eq!(banner.map(|b| b.can_raise), Some(false));
    assert!(t.vcx.debug_bounds("button-strip-fetch-anyway").is_none());

    // Close cursor from the banner closes with the memory limit as its reason.
    t.click("button-strip-close-cursor");
    assert_eq!(
        control.closes.lock().unwrap().as_slice(),
        &[CloseReason::MemoryLimit]
    );
}

#[gpui::test]
fn suspended_strip_buttons_reach_the_source(cx: &mut TestAppContext) {
    let mut t = tile(cx, "en", None);
    let control = Arc::new(Control::default());
    t.apply(&[tab(
        1,
        source(
            store_of(10, 1, Completeness::Suspended(cursor())),
            control.clone(),
        ),
    )]);
    t.click("button-strip-fetch-more");
    assert_eq!(control.fetches.lock().unwrap().len(), 1);
    t.click("button-strip-fetch-all");
    assert_eq!(*control.fetch_alls.lock().unwrap(), 1);
    t.click("button-strip-close-cursor");
    assert_eq!(
        control.closes.lock().unwrap().as_slice(),
        &[CloseReason::UserClosed]
    );
}

#[gpui::test]
fn rerun_names_the_unit(cx: &mut TestAppContext) {
    let mut t = tile(cx, "en", None);
    let events = record_events(&t);
    t.apply(&[tab(
        4,
        source(store_of(2, 1, Completeness::Cancelled), Arc::default()),
    )]);
    assert_eq!(
        t.text("strip-sentence").as_deref(),
        Some("Cancelled after 2 rows")
    );
    t.click("button-strip-rerun");
    assert_eq!(
        events.borrow().as_slice(),
        &[ResultsEvent::Rerun {
            console: t.console,
            unit_ordinal: 4
        }]
    );
}

#[gpui::test]
fn two_thousand_columns_render_with_the_hint(cx: &mut TestAppContext) {
    let t = tile(cx, "en", None);
    t.apply(&[tab(
        1,
        source(store_of(2, 2_412, Completeness::Complete), Arc::default()),
    )]);
    let hint = t.text("results-columns-hint").expect("the hint row");
    assert!(hint.starts_with("2,412 columns. Wide results"), "{hint}");
    let host = t.host();
    let grid = t
        .vcx
        .read(|cx| host.read(cx).grid_of(t.console, 0, cx))
        .expect("the tab's grid");
    assert!(t.vcx.read(|cx| grid.read(cx).frame_stats().cells) > 0);
}

/// 2,000 columns exactly: no hint. (Its own test: one shell window per app. Two workspace
/// windows in one app ping-pong Task 34's `ActiveTab` observer, which refreshes every window,
/// so a second shell here never settles; noted for Task 34.)
#[gpui::test]
fn two_thousand_columns_exactly_have_no_hint(cx: &mut TestAppContext) {
    let t = tile(cx, "en", None);
    t.apply(&[tab(
        1,
        source(store_of(1, 2_000, Completeness::Complete), Arc::default()),
    )]);
    assert_eq!(t.text("results-columns-hint"), None);
}

#[gpui::test]
fn zero_rows_keep_the_header_and_say_so(cx: &mut TestAppContext) {
    let t = tile(cx, "en", None);
    t.apply(&[tab(
        1,
        source(store_of(0, 3, Completeness::Complete), Arc::default()),
    )]);
    assert_eq!(t.text("strip-sentence").as_deref(), Some("0 rows"));
    // ## m4-t32-shots (named exception; carries-w4 flake): under load the first frame after
    // `apply` can be the grid's empty one; wait until the grid has drawn its text.
    wait_until(t.vcx, "the grid draws No rows", |vcx| {
        in_window(vcx, |window, _| window.refresh());
        vcx.run_until_parked();
        texts_drawn_by(vcx, "hacivat-ui-grid/src/view.rs")
            .iter()
            .any(|s| s == "No rows")
    });
}

// ---- the output log -------------------------------------------------------------------------

fn record_events(t: &Tile) -> Rc<RefCell<Vec<ResultsEvent>>> {
    let events = Rc::new(RefCell::new(Vec::new()));
    let sink = events.clone();
    in_window(t.vcx, |_, cx| {
        cx.subscribe(&t.results, move |_, e: &ResultsEvent, _| {
            sink.borrow_mut().push(e.clone())
        })
        .detach()
    });
    events
}

fn error_line(log: &SessionLog, n: u32, range: Range<usize>) {
    log.append(
        LogMsg::Error {
            n,
            view: ErrorView {
                title_id: "exec-error-server",
                code: Some("42P01".into()),
                server_text: Some("relation \"Statement 3 failed\" does not exist".into()),
                hint_id: None,
                doc_offset: None,
                // ## m3b-t25-results (named exception)
                vendor_code: None,
            },
        },
        Some(range),
        Some(unit(n)),
    );
}

#[gpui::test]
fn error_line_keeps_server_text_out_of_the_title(cx: &mut TestAppContext) {
    let t = tile(cx, "en", None);
    error_line(&t.log, 3, 5..20);
    t.redraw();
    assert_eq!(
        t.text("log-sentence-0").as_deref(),
        Some("Statement 3 failed")
    );
    assert_eq!(t.text("log-code-0").as_deref(), Some("42P01"));
    assert_eq!(
        t.text("log-detail-0").as_deref(),
        Some("relation \"Statement 3 failed\" does not exist")
    );
    // The server's text is data, in its own field.
    let detail = registry_frame(t.vcx)
        .into_iter()
        .find(|r| r.path.ends_with("#log-detail-0"))
        .expect("the server text row");
    assert_eq!(detail.source, hacivat_ui::text::TextSource::Data);
}

#[gpui::test]
fn clicking_an_error_line_emits_go_to_source(cx: &mut TestAppContext) {
    let mut t = tile(cx, "en", None);
    let events = record_events(&t);
    t.log.append(LogMsg::Completed { ms: 3 }, None, None);
    error_line(&t.log, 2, 5..20);
    t.redraw();
    // A line without a statement does nothing.
    t.click("log-row-0");
    assert!(events.borrow().is_empty());
    t.click("log-row-1");
    assert_eq!(
        events.borrow().as_slice(),
        &[ResultsEvent::GoToSource {
            console: t.console,
            range: 5..20
        }]
    );
    // The server text's row selects the statement too.
    t.click("log-row-1-detail");
    assert_eq!(events.borrow().len(), 2);
}

#[gpui::test]
fn log_copy_is_iso_and_ascii_under_tr(cx: &mut TestAppContext) {
    let mut t = tile(cx, "tr", None);
    t.log.append(
        LogMsg::RowsRetrieved {
            rows: 12_345,
            more: false,
            exec_ms: 1_500,
            fetch_ms: 0,
        },
        None,
        None,
    );
    error_line(&t.log, 2, 0..4);
    t.redraw();
    t.click("button-results-log-copy");
    let copied = t
        .vcx
        .read_from_clipboard()
        .and_then(|item| item.text())
        .expect("the log was copied");
    let first = copied.lines().next().expect("a line");
    let stamp = first.split('\t').next().unwrap();
    // 2026-11-15T10:20:30.123Z
    assert_eq!(stamp.len(), 24, "{stamp}");
    assert_eq!(&stamp[10..11], "T");
    assert!(stamp.ends_with('Z'), "{stamp}");
    assert!(first.contains("\trows=12345\t"), "{first}");
    assert!(first.contains("duration=1500ms"), "{first}");
    assert!(copied.contains("\tcode=42P01\t"), "{copied}");
    // ## m3a-t30 fix round 1 (M9): no digit outside ASCII (no locale digits, no grouping).
    assert!(
        !copied
            .chars()
            .any(|c| c.is_numeric() && !c.is_ascii_digit()),
        "{copied}"
    );
    assert!(
        !copied.contains("12.345") && !copied.contains("12,345"),
        "{copied}"
    );
    assert!(
        !copied.chars().any(|c| matches!(c, '\u{2068}' | '\u{2069}')),
        "no Fluent isolation marks"
    );
}

#[gpui::test]
fn ten_thousand_log_lines_render_at_constant_cost(cx: &mut TestAppContext) {
    let t = tile(cx, "en", None);
    for i in 0..10_000u64 {
        t.log.append(LogMsg::Completed { ms: i }, None, None);
    }
    t.redraw();
    let host = t.host();
    let list = t
        .vcx
        .read(|cx| host.read(cx).log_list(t.console))
        .expect("the output list");
    let (len, visible) = t.vcx.read(|cx| {
        let l = list.read(cx);
        (l.rows().len(), l.visible_rows())
    });
    assert_eq!(len, 10_000);
    let shown = visible.end - visible.start;
    assert!((1..=60).contains(&shown), "{visible:?}");
    // A frame builds the visible rows only, wherever the list is scrolled.
    t.vcx.read(|cx| list.read(cx).rows().reset_built());
    t.redraw();
    let built = t.vcx.read(|cx| list.read(cx).rows().built());
    // ## m4-t32-shots (named exception; carries-w4, Windows CI 44629840018): a platform may
    // lay a frame out twice (Windows did), so a frame builds the visible rows at most once
    // per layout pass: still a constant cost, never the 10,000 rows.
    const MAX_LAYOUT_PASSES: u64 = 2;
    assert!(
        built <= MAX_LAYOUT_PASSES * (shown + 1),
        "built {built} rows for {shown} visible"
    );
}

// ---- named design rules ---------------------------------------------------------------------

/// The Raised Ink Rule (DESIGN.md): the strip and the result toolbar are raised, so nothing on
/// them is drawn in `text-3`; the strip and the toolbar paint the `raised` fill.
#[cfg(feature = "scene-check")]
#[gpui::test]
fn raised_ink_on_the_strip_and_toolbar(cx: &mut TestAppContext) {
    use gpui::Rgba;
    use hacivat_ui_core::theme::Theme;
    let t = tile(cx, "en", None);
    t.apply(&[tab(
        1,
        source(
            store_of(10, 2, Completeness::Suspended(cursor())),
            Arc::default(),
        ),
    )]);
    hacivat_ui::icon::icon_record();
    t.redraw();
    let icons = hacivat_ui::icon::icon_take();
    let strip = t.vcx.debug_bounds("results-strip").expect("the strip");
    let toolbar = t
        .vcx
        .debug_bounds("toolbar-results-toolbar")
        .expect("the result toolbar");
    let (text_3, raised) = t.vcx.read(|cx| {
        let c = Theme::get(cx).tokens.colors;
        (
            Rgba::from(c.text_3.to_hsla()),
            Rgba::from(c.raised.to_hsla()),
        )
    });
    let same = |a: Rgba, b: Rgba| {
        [(a.r, b.r), (a.g, b.g), (a.b, b.b), (a.a, b.a)]
            .iter()
            .all(|(x, y)| (x - y).abs() <= 1.0 / 255.0)
    };
    let (glyphs, fills) = t.vcx.update(|window, _| {
        let scale = window.scale_factor();
        let scene = window.rendered_scene();
        let logical = |b: gpui::Bounds<gpui::ScaledPixels>| {
            gpui::Bounds::new(
                gpui::point(
                    gpui::px(b.origin.x.0 / scale),
                    gpui::px(b.origin.y.0 / scale),
                ),
                gpui::size(
                    gpui::px(b.size.width.0 / scale),
                    gpui::px(b.size.height.0 / scale),
                ),
            )
        };
        let inside = |b: gpui::Bounds<gpui::Pixels>| {
            [strip, toolbar].iter().any(|r| r.contains(&b.center()))
        };
        let glyphs: Vec<Rgba> = scene
            .monochrome_sprites
            .iter()
            .filter(|s| inside(logical(s.bounds)))
            .map(|s| Rgba::from(s.color))
            .collect();
        let fills: Vec<(gpui::Bounds<gpui::Pixels>, Rgba)> = scene
            .quads
            .iter()
            .filter_map(|q| Some((logical(q.bounds), Rgba::from(q.background.as_solid()?))))
            .collect();
        (glyphs, fills)
    });
    assert!(
        glyphs.iter().all(|g| !same(*g, text_3)),
        "text-3 glyphs on a raised surface"
    );
    // The recorder sees the whole window; these are the glyphs the toolbar (Copy) and the
    // strip (the more-sets layers, its leading glyphs) draw.
    use hacivat_ui::icon::IconName;
    let ours: Vec<_> = icons
        .iter()
        .filter(|i| {
            matches!(
                i.name,
                IconName::Copy
                    | IconName::Layers
                    | IconName::Check
                    | IconName::CircleAlert
                    | IconName::Info
                    | IconName::Ban
            )
        })
        .collect();
    assert!(!ours.is_empty(), "the toolbar's Copy glyph is drawn");
    assert!(
        ours.iter().all(|i| !same(Rgba::from(i.color), text_3)),
        "a text-3 icon on a raised surface: {ours:?}"
    );
    for area in [strip, toolbar] {
        assert!(
            fills
                .iter()
                .any(|(b, fill)| same(*fill, raised) && b.contains(&area.center())),
            "{area:?} is raised"
        );
    }
}

// ## m3a-t30 fix round 1 ------------------------------------------------------------------------

/// Runs command `id` in the shell's window.
fn run_command(vcx: &VisualTestContext, id: &str) -> bool {
    let ran = in_window(vcx, |window, cx| {
        hacivat_ui::actions::ActionRegistry::run(id, window, cx)
    });
    in_window(vcx, |window, _| window.refresh());
    vcx.run_until_parked();
    ran
}

/// The grid of `console`'s result tab `ix` (the tile has shown it).
fn grid_of(vcx: &VisualTestContext, console: ConsoleId, ix: usize) -> Entity<GridView> {
    vcx.cx
        .read(|cx| {
            hosts(cx)
                .into_iter()
                .find_map(|h| h.read(cx).grid_of(console, ix, cx))
        })
        .expect("the tab's grid was shown")
}

/// Focuses `grid` and selects all its cells through the palette command.
fn focus_and_select_all(vcx: &VisualTestContext, grid: &Entity<GridView>) {
    in_window(vcx, |window, cx| {
        let handle = grid.read(cx).focus_handle(cx);
        window.focus(&handle, cx);
    });
    vcx.run_until_parked();
    assert!(run_command(vcx, "grid.select_all"));
    assert!(vcx.cx.read(|cx| grid.read(cx).selection().is_all()));
}

fn put_sentinel(vcx: &VisualTestContext) {
    in_window(vcx, |_, cx| {
        cx.write_to_clipboard(gpui::ClipboardItem::new_string("sentinel".to_string()))
    });
}

fn clipboard(vcx: &VisualTestContext) -> String {
    vcx.cx
        .read_from_clipboard()
        .and_then(|item| item.text())
        .unwrap_or_default()
}

/// C1: a grid the tile no longer shows never answers a command, and loses the focus.
#[gpui::test]
fn copy_never_takes_a_hidden_tabs_cells(cx: &mut TestAppContext) {
    let t = tile(cx, "en", None);
    t.apply(&[
        tab(1, source(store_values(&[4242, 4243]), Arc::default())),
        tab(2, source(store_values(&[9191]), Arc::default())),
    ]);
    let hidden = grid_of(t.vcx, t.console, 0);
    focus_and_select_all(t.vcx, &hidden);

    // Result 2 is shown now; Result 1's grid keeps its selection off screen.
    t.set_active(ActiveResult::Tab(1));
    let focused = in_window(t.vcx, |window, cx| {
        hidden
            .read(cx)
            .focus_handle(cx)
            .contains_focused(window, cx)
    });
    assert!(!focused, "the hidden grid keeps the focus");
    for command in ["results.copy", "grid.copy", "grid.copy_with_headers"] {
        put_sentinel(t.vcx);
        run_command(t.vcx, command);
        let copied = clipboard(t.vcx);
        assert!(!copied.contains("4242"), "{command} copied {copied:?}");
    }
    // Navigation moves the shown grid, never the hidden one.
    run_command(t.vcx, "grid.move_down");
    assert!(t.vcx.cx.read(|cx| hidden.read(cx).selection().is_all()));
}

/// C1 across consoles: another console's grid is never copied from.
#[gpui::test]
fn copy_never_takes_another_consoles_cells(cx: &mut TestAppContext) {
    let t = tile(cx, "en", None);
    t.apply(&[tab(1, source(store_values(&[4242]), Arc::default()))]);
    let hidden = grid_of(t.vcx, t.console, 0);
    focus_and_select_all(t.vcx, &hidden);

    let profile = profile_id(t.vcx, "orders-dev");
    let (other, results, _) = add_console(t.vcx, profile);
    apply_to(
        t.vcx,
        &results,
        &[tab(1, source(store_values(&[9191]), Arc::default()))],
    );
    let shown = t.vcx.read(|cx| t.host().read(cx).shown());
    assert_eq!(shown, Some(other));
    for command in ["results.copy", "grid.copy"] {
        put_sentinel(t.vcx);
        run_command(t.vcx, command);
        let copied = clipboard(t.vcx);
        assert!(!copied.contains("4242"), "{command} copied {copied:?}");
    }
}

/// I6 (R-M3-41): the tile follows the focused pane's active console through a tab switch, a
/// pane focus change and a close.
#[gpui::test]
fn the_tile_follows_the_focused_panes_active_console(cx: &mut TestAppContext) {
    let t = tile(cx, "en", None);
    t.apply(&[tab(1, source(store_values(&[1]), Arc::default()))]);
    let a = t.console;
    let profile = profile_id(t.vcx, "orders-dev");
    let (b, results_b, _) = add_console(t.vcx, profile);
    apply_to(
        t.vcx,
        &results_b,
        &[tab(1, source(store_values(&[1, 2, 3]), Arc::default()))],
    );
    let shown = |t: &Tile| t.vcx.read(|cx| t.host().read(cx).shown());
    assert_eq!(shown(&t), Some(b));
    assert_eq!(t.text("strip-sentence").as_deref(), Some("3 rows"));

    // The other tab of the same pane.
    let ws = workspace(t.vcx);
    in_window(t.vcx, |window, cx| {
        ws.update(cx, |ws, cx| ws.activate_prev_tab(window, cx))
    });
    t.redraw();
    assert_eq!(shown(&t), Some(a));
    assert_eq!(t.text("strip-sentence").as_deref(), Some("1 row"));
    assert_eq!(t.text("results-rows-timing").as_deref(), Some("1 row"));

    // A new pane takes the focus with an unbound console: the M1 tile.
    in_window(t.vcx, |window, cx| {
        ws.update(cx, |ws, cx| {
            ws.split_focused(gpui::Axis::Horizontal, window, cx)
        })
    });
    t.redraw();
    assert_eq!(shown(&t), None);
    assert!(rendered_markers(t.vcx).iter().any(|m| m == "results-empty"));
    assert_eq!(t.text("strip-sentence"), None);

    // Back to the first pane: its active console again.
    in_window(t.vcx, |window, cx| {
        ws.update(cx, |ws, cx| ws.focus_next_pane(window, cx))
    });
    t.redraw();
    assert_eq!(shown(&t), Some(a));
    assert_eq!(t.text("strip-sentence").as_deref(), Some("1 row"));

    // Closing the shown console's tab shows the pane's next active console, never the closed
    // one's rows.
    let tab_a = t
        .vcx
        .read(|cx| Consoles::get(cx, a).map(|s| s.tab))
        .unwrap();
    in_window(t.vcx, |_, cx| {
        ws.update(cx, |ws, cx| ws.force_close_tab(tab_a, cx));
        Consoles::remove(cx, a);
    });
    t.redraw();
    let expected = t.vcx.read(|cx| {
        ws.read(cx)
            .active_tab()
            .and_then(|tab| Consoles::by_tab(cx, tab))
    });
    assert_eq!(shown(&t), expected);
    assert_ne!(shown(&t), Some(a));
    assert_ne!(t.text("strip-sentence").as_deref(), Some("1 row"));
    match expected {
        // The pane's next tab is the shell's unbound console: the M1 tile.
        None => assert!(rendered_markers(t.vcx).iter().any(|m| m == "results-empty")),
        Some(c) => {
            assert_eq!(c, b);
            assert_eq!(t.text("strip-sentence").as_deref(), Some("3 rows"));
        }
    }
    // Activating the other console shows its rows.
    in_window(t.vcx, |window, cx| {
        ws.update(cx, |ws, cx| ws.activate_next_tab(window, cx))
    });
    t.redraw();
    if shown(&t) == Some(b) {
        assert_eq!(t.text("strip-sentence").as_deref(), Some("3 rows"));
    }
    assert_ne!(shown(&t), Some(a));
}

/// I2: the strip's closed and more-sets facts outlive the log's Clear.
#[gpui::test]
fn the_strip_keeps_its_facts_after_the_log_is_cleared(cx: &mut TestAppContext) {
    let t = tile(cx, "en", None);
    t.log.append(
        LogMsg::CursorClosed {
            rows: 4,
            reason: CloseReason::IdleTimeout,
            rolled_back: true,
            idle_ms: 600_000,
        },
        None,
        Some(unit(1)),
    );
    t.log
        .append(LogMsg::MoreResultSets { skipped: 2 }, None, Some(unit(2)));
    t.apply(&[
        tab(
            1,
            source(
                store_of(
                    4,
                    1,
                    Completeness::Closed {
                        reason: CloseReason::IdleTimeout,
                    },
                ),
                Arc::default(),
            ),
        ),
        tab(
            2,
            source(
                store_of(
                    5,
                    1,
                    Completeness::Truncated(TruncReason::FollowingResultSets),
                ),
                Arc::default(),
            ),
        ),
    ]);
    let closed = t.text("strip-sentence").expect("the closed sentence");
    assert!(closed.contains("10 min"), "{closed}");
    assert!(closed.contains("rolled back"), "{closed}");
    t.set_active(ActiveResult::Tab(1));
    assert_eq!(
        t.text("strip-more-sets").as_deref(),
        Some("2 more result sets not shown")
    );

    t.log.clear();
    t.redraw();
    assert_eq!(
        t.text("strip-more-sets").as_deref(),
        Some("2 more result sets not shown")
    );
    t.set_active(ActiveResult::Tab(0));
    assert_eq!(t.text("strip-sentence"), Some(closed));
}

/// I2: an idle close the log never told about says so without inventing a duration.
#[gpui::test]
fn an_idle_close_without_its_log_line_names_no_duration(cx: &mut TestAppContext) {
    let t = tile(cx, "en", None);
    t.apply(&[tab(
        1,
        source(
            store_of(
                4,
                1,
                Completeness::Closed {
                    reason: CloseReason::IdleTimeout,
                },
            ),
            Arc::default(),
        ),
    )]);
    assert_eq!(
        t.text("strip-sentence").as_deref(),
        Some("Cursor closed after 4 rows: it was idle too long.")
    );
}

/// I3: server text and its code are shown with their controls visible, on one line.
#[gpui::test]
fn server_text_controls_are_shown_not_obeyed(cx: &mut TestAppContext) {
    let t = tile(cx, "en", None);
    t.log.append(
        LogMsg::Error {
            n: 1,
            view: ErrorView {
                title_id: "exec-error-server",
                code: Some("42\u{202E}P01".into()),
                server_text: Some("near 'a\nb\u{202E}cd' at line 1".into()),
                hint_id: None,
                doc_offset: None,
                // ## m3b-t25-results (named exception)
                vendor_code: None,
            },
        },
        Some(0..4),
        Some(unit(1)),
    );
    t.redraw();
    let detail = t.text("log-detail-0").expect("the server text row");
    assert!(
        !detail.contains('\n') && !detail.contains('\u{202E}'),
        "{detail:?}"
    );
    assert!(detail.contains('↵'), "{detail}");
    assert!(detail.contains("RLO"), "{detail}");
    let code = t.text("log-code-0").expect("the code chip");
    assert!(!code.contains('\u{202E}'), "{code:?}");
    assert!(code.contains("RLO"), "{code}");
}

/// I4: a result that stopped before its first row is not called empty.
#[gpui::test]
fn a_cancelled_result_without_rows_is_not_called_empty(cx: &mut TestAppContext) {
    let t = tile(cx, "en", None);
    t.apply(&[tab(
        1,
        source(store_of(0, 3, Completeness::Cancelled), Arc::default()),
    )]);
    assert_eq!(
        t.text("strip-sentence").as_deref(),
        Some("Cancelled after 0 rows")
    );
    let drawn = texts_drawn_by(t.vcx, "hacivat-ui-grid/src/view.rs");
    assert!(!drawn.iter().any(|s| s == "No rows"), "{drawn:?}");
}

/// R1-1: a late line of an older run (the old cursor's close, read after the re-run took the
/// tab) never hides the new run's result.
#[gpui::test]
fn a_late_line_of_an_older_run_keeps_the_new_result(cx: &mut TestAppContext) {
    // ## m3a-t30 fix round 2 (R1-1): re-running a SELECT closes its old cursor first; that
    // close line names the older run and may be read after the tab moved to the new run.
    let t = tile(cx, "en", None);
    let run2 = |ordinal: u32, source: Arc<dyn ResultSource>| ResultTab {
        unit: UnitId {
            run: RunId(2),
            ordinal,
        },
        ..tab(ordinal, source)
    };
    t.apply(&[run2(1, source(store_values(&[1, 2]), Arc::default()))]);
    t.log.append(
        LogMsg::CursorClosed {
            rows: 2,
            reason: CloseReason::SessionNeeded,
            rolled_back: true,
            idle_ms: 0,
        },
        None,
        Some(unit(1)),
    );
    t.redraw();
    let active = t.vcx.read(|cx| t.results.read(cx).active());
    assert_eq!(active, ActiveResult::Tab(0));
    assert!(t.text("strip-sentence").is_some());
}

/// I5 (NOTES surface 9): a run that ends in an error or update counts only shows Output; a
/// line of the run whose result is shown leaves the result in view.
#[gpui::test]
fn a_run_without_a_result_shows_the_output(cx: &mut TestAppContext) {
    let t = tile(cx, "en", None);
    t.apply(&[tab(1, source(store_values(&[1, 2]), Arc::default()))]);
    // The same run's later error (a SELECT-then-error script) keeps the result.
    error_line(&t.log, 1, 0..4);
    t.redraw();
    let active = t.vcx.read(|cx| t.results.read(cx).active());
    assert_eq!(active, ActiveResult::Tab(0));
    assert!(t.text("strip-sentence").is_some());

    // A later run that fails without a result: Output, and no strip.
    t.log.append(
        LogMsg::Error {
            n: 1,
            view: ErrorView {
                title_id: "exec-error-server",
                code: Some("42P01".into()),
                server_text: Some("relation \"nope\" does not exist".into()),
                hint_id: None,
                doc_offset: None,
                // ## m3b-t25-results (named exception)
                vendor_code: None,
            },
        },
        Some(0..4),
        Some(UnitId {
            run: RunId(2),
            ordinal: 1,
        }),
    );
    t.redraw();
    let active = t.vcx.read(|cx| t.results.read(cx).active());
    assert_eq!(active, ActiveResult::Output);
    assert_eq!(t.text("strip-sentence"), None);
}

/// The T33 wiring: a Re-run the tile emits for a result tab reaches the run UI, which sends
/// exactly that tab's statement again, never a later statement of the same ordinal that made no
/// tab (a DELETE after the SELECT).
#[gpui::test]
fn rerun_from_a_result_tab_resends_that_tabs_unit(cx: &mut TestAppContext) {
    use hacivat_driver::ExecSummary;
    use hacivat_driver::fake::{
        FakeDriver, FakeExec, FakeQuery, FakeResultSet, FakeRows, FakeTypedCall,
    };
    use hacivat_proto::DbmsKind;
    use hacivat_ui_editor::ExecMarkState;

    let vcx = shell_with_connections(cx, "en");
    in_window(vcx, |window, _| window.activate_window());
    let p = hacivat_session::profile::Profile {
        engine: DbmsKind::Postgres,
        ..test_profile("orders-dev", Some(EnvTag::Dev), "127.0.0.1")
    };
    let profile = add_profile(vcx, p.clone());
    let probes: Vec<FakeExec> = (0..8)
        .map(|_| FakeExec {
            prefix: "SHOW standard_conforming_strings",
            result: Ok(ExecSummary {
                rows: vec![vec![Some("on".to_string())]],
                row_count: 1,
                ..ExecSummary::default()
            }),
            delay: Duration::ZERO,
            tx_after: None,
            panics: false,
        })
        .collect();
    let three_rows = || FakeQuery {
        prefix: "SELECT",
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
    };
    let fake = FakeDriver::new(DbmsKind::Postgres)
        .tracks_transactions()
        .exec(probes)
        .queries(vec![three_rows(), three_rows()]);
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
    let (editor, results) = vcx.cx.read(|cx| {
        let s = Consoles::get(cx, id).expect("the console");
        (
            s.editor.clone().expect("an editor"),
            s.results.clone().expect("results"),
        )
    });
    let queries = || -> Vec<String> {
        fake.typed_log()
            .into_iter()
            .filter_map(|c| match c {
                FakeTypedCall::Query(sql) => Some(sql),
                _ => None,
            })
            .collect()
    };
    let run = |range: Range<usize>| {
        vcx.cx
            .update(|cx| editor.update(cx, |e, cx| e.select_range(range, cx)));
        in_window(vcx, |window, cx| {
            editor.update(cx, |e, cx| assert!(e.run_command("editor.run", window, cx)));
        });
        vcx.run_until_parked();
    };
    let ended = |vcx: &VisualTestContext| {
        vcx.cx.read(|cx| {
            hacivat_app::exec_ui::last_run_marks(id, cx)
                .into_iter()
                .flatten()
                .any(|m| matches!(m, ExecMarkState::Succeeded { .. }))
        })
    };
    vcx.cx.update(|cx| {
        editor.update(cx, |e, cx| {
            e.set_text("SELECT * FROM t;\nDELETE FROM t WHERE id = 5;\n", cx)
        })
    });
    vcx.run_until_parked();
    run(0..16);
    wait_until(vcx, "the select", ended);
    wait_until(vcx, "the select's tab", |vcx| {
        vcx.cx.read(|cx| results.read(cx).tabs().len() == 1)
    });
    run(17..43);
    wait_until(vcx, "the delete", |_| queries().len() == 2);
    wait_until(vcx, "the delete's end", ended);
    // The tile shows the console (its subscription is made on first sight).
    in_window(vcx, |window, _| window.refresh());
    vcx.run_until_parked();
    let ordinal = vcx
        .cx
        .read(|cx| results.read(cx).tab(0).map(|t| t.key().unit_ordinal))
        .expect("the select's tab");
    // What the strip's Re-run does.
    in_window(vcx, |_, cx| {
        results.update(cx, |_, cx| {
            cx.emit(ResultsEvent::Rerun {
                console: id,
                unit_ordinal: ordinal,
            })
        })
    });
    wait_until(vcx, "the re-run", |_| queries().len() == 3);
    assert_eq!(
        queries(),
        [
            "SELECT * FROM t",
            "DELETE FROM t WHERE id = 5",
            "SELECT * FROM t"
        ]
    );
    stop_core_runtime_test(cx);
}

// ## m3c-t35 (named exception): the strip counts a selection of several ranges (NOTES M3c-22).
#[gpui::test]
fn the_strip_counts_the_ranges_of_a_multi_range_selection(cx: &mut TestAppContext) {
    use hacivat_ui_grid::GridCommand;
    use hacivat_ui_grid::model::Dir;

    let t = tile(cx, "en", None);
    t.apply(&[tab(
        1,
        source(store_of(20, 4, Completeness::Complete), Arc::default()),
    )]);
    let host = t.host();
    let grid = t
        .vcx
        .read(|cx| host.read(cx).grid_of(t.console, 0, cx))
        .expect("the tab's grid");
    // One block: rows × columns. (The first key focuses the first cell.)
    in_window(t.vcx, |window, cx| {
        grid.update(cx, |g, cx| {
            for (dir, extend) in [(Dir::Down, false), (Dir::Down, true)] {
                g.dispatch(GridCommand::Move { dir, extend }, window, cx);
            }
        })
    });
    t.redraw();
    assert_eq!(t.text("strip-dims").as_deref(), Some("2 × 1"));
    // Ctrl/Cmd-click on another cell adds a range: "2 ranges · 3 cells".
    let at = t.vcx.read(|cx| {
        let g = grid.read(cx);
        let geom = g.geometry().expect("painted");
        let l = g.column_layout();
        gpui::point(
            geom.body.left() + gpui::px(l.x_of(2) as f32 + 10.0),
            geom.body.top() + gpui::px(g.scroll_state().row_top(5) as f32 + 10.0),
        )
    });
    let add = t.vcx.update(|_, cx| gpui::Modifiers::secondary_key(cx));
    t.vcx.simulate_click(at, add);
    t.redraw();
    assert_eq!(
        t.vcx.read(|cx| grid.read(cx).selection().ranges()),
        2,
        "the click added a range"
    );
    assert_eq!(t.text("strip-dims").as_deref(), Some("2 ranges · 3 cells"));
}
