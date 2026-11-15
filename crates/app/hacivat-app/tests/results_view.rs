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

use gpui::{AppContext as _, Entity, TestAppContext, VisualTestContext};
use hacivat_app::console::{ConsoleId, ConsoleState, Consoles};
use hacivat_app::results::strip::StripModel;
use hacivat_app::results::view::{ResultsHost, hosts};
use hacivat_app::results::{ActiveResult, ConsoleResults, ResultsEvent};
use hacivat_exec::{ErrorView, LogMsg, ResultTab, SessionLog, TabKey};
use hacivat_proto::EnvTag;
use hacivat_results::{
    CloseReason, Completeness, CursorInfo, FetchControl, FetchPolicy, PauseReason, ResultId,
    ResultSource, ResultStore, StoreSource, TruncReason,
};
use hacivat_session::session::{ClientId, RunId, UnitId};
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
    StripModel::of(&store.snapshot(), None, &[], unit(1)).sentence_id()
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
        StripModel::of(&paused.snapshot(), banner, &[], unit(1)).sentence_id(),
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
        StripModel::of(&closed.snapshot(), None, &log.lines(), unit(1)).sentence_id(),
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

    // 2,000 columns exactly: no hint.
    let t2 = tile(cx, "en", None);
    t2.apply(&[tab(
        1,
        source(store_of(1, 2_000, Completeness::Complete), Arc::default()),
    )]);
    assert_eq!(t2.text("results-columns-hint"), None);
}

#[gpui::test]
fn zero_rows_keep_the_header_and_say_so(cx: &mut TestAppContext) {
    let t = tile(cx, "en", None);
    t.apply(&[tab(
        1,
        source(store_of(0, 3, Completeness::Complete), Arc::default()),
    )]);
    assert_eq!(t.text("strip-sentence").as_deref(), Some("0 rows"));
    let drawn = texts_drawn_by(t.vcx, "hacivat-ui-grid/src/view.rs");
    assert!(drawn.iter().any(|s| s == "No rows"), "{drawn:?}");
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
    assert!(
        copied
            .chars()
            .filter(char::is_ascii_digit)
            .all(|c| c.is_ascii()),
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
    assert!(built <= shown + 1, "built {built} rows for {shown} visible");
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
