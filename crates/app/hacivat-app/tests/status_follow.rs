//! M3a Task 34: the status bar and the capsule follow the console (DESIGN.md "Status Follows
//! The Tab"; design/m3a-query/NOTES.md surface 6).
//!
//! - The capsule describes the focused console's connection; without one it keeps the M2 rule
//!   (the explorer's selection while it has a live state, else the last connect attempt).
//! - The status bar shows the environment and access of the focused console's active result
//!   tab (else the console's), the user's transaction ("Transaction open · N statements", its
//!   Failed form), rows · timing, "Cursor open" (a button that closes the cursor), then the
//!   console's editor segment.
//!
//! Consoles are Task 1b's `register_test_console` (a real editor over a FakeDriver session on
//! the shell's core runtime, P12). Every wait is on a condition (`wait_until`).

mod support;

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use gpui::{Focusable as _, TestAppContext, VisualTestContext};
use hacivat_app::connections::{ConnectionsModel, access_popover, capsule};
use hacivat_app::console::{ConsoleId, Consoles};
use hacivat_app::results::ActiveResult;
use hacivat_driver::fake::{FakeDriver, FakeQuery, FakeResultSet, FakeRows, FakeTypedCall};
use hacivat_driver::{ExecOpts, Protocol, TxStatus};
use hacivat_exec::{ResultTab, TabKey};
use hacivat_proto::{AccessLabel, DbmsKind, EnvTag, ProfileId};
use hacivat_results::{
    CloseReason, Completeness, CursorInfo, FetchControl, FetchPolicy, MemoryBudget, ResultId,
    ResultStore, ResultWriter, StoreSource,
};
use hacivat_session::LiveState;
use hacivat_session::session::{
    ClosePolicy, ErrorPolicy, ExecOrigin, ObserverRef, Request, RunEvent, RunId, RunObserver,
    RunSpec, RunUnit, SessionHandle, SessionSnapshot, TxOwner, UnitId,
};
use hacivat_ui::capsule::{CapsuleModel, EnvKind};
use hacivat_ui::panes::TabId;
use hacivat_values::{ColumnMeta, LogicalType, NativeType, ResultSchema, Value};
use support::*;

// ---------------------------------------------------------------------------------------------
// Fixtures

const PROD: &str = "orders-prod";
const STAGING: &str = "orders-stg";

fn pg() -> FakeDriver {
    FakeDriver::new(DbmsKind::Postgres)
}

/// Connects `name` and waits until it is connected.
fn connect_and_wait(vcx: &VisualTestContext, name: &str) {
    connect(vcx, name);
    wait_until(vcx, "the connect to finish", |vcx| {
        !matches!(live(vcx, name), LiveState::Connecting(_))
    });
    assert!(
        matches!(live(vcx, name), LiveState::Connected(_)),
        "{:?}",
        live(vcx, name)
    );
}

/// A shell with two connected profiles, `orders-prod` (production) and `orders-stg` (staging);
/// staging is connected last, so the M2 rule shows it.
fn shell_two(cx: &mut TestAppContext) -> &'static mut VisualTestContext {
    let vcx = shell_with_fake(cx, "en", pg());
    add_profile(
        vcx,
        test_profile(PROD, Some(EnvTag::Production), "10.0.4.12"),
    );
    add_profile(
        vcx,
        test_profile(STAGING, Some(EnvTag::Staging), "10.0.4.13"),
    );
    connect_and_wait(vcx, PROD);
    connect_and_wait(vcx, STAGING);
    vcx
}

/// A console session of profile `id` over `fake` (as `test_session`, with a fake the test
/// scripts). A production profile's session is read-only, as the product opens it
/// (`effective_read_only` under the boot's production policy; fix round 1, I2).
fn session_over(
    vcx: &VisualTestContext,
    id: ProfileId,
    fake: FakeDriver,
) -> (SessionHandle, Arc<FakeDriver>) {
    session_with(vcx, id, fake, None)
}

/// [`session_over`] whose read-only flag is `read_only` whatever the profile says (`None`:
/// as the product decides it).
fn session_with(
    vcx: &VisualTestContext,
    id: ProfileId,
    fake: FakeDriver,
    read_only: Option<bool>,
) -> (SessionHandle, Arc<FakeDriver>) {
    let (rt, mut profile) = vcx.cx.read(|cx| {
        let m = cx.global::<ConnectionsModel>();
        let profile = m.get(id).expect("a saved profile");
        (m.core_runtime().handle(), (*profile).clone())
    });
    profile.read_only =
        read_only.unwrap_or(profile.read_only || profile.environment == Some(EnvTag::Production));
    futures::executor::block_on(hacivat_session::testing::fake_session(rt, fake, profile))
}

/// A console bound to the profile named `name` over a plain FakeDriver session.
fn console_of(vcx: &VisualTestContext, name: &str) -> ConsoleId {
    let id = profile_id(vcx, name);
    let (session, _) = session_over(vcx, id, pg());
    register_test_console(vcx, id, session)
}

/// Shows console `id`'s tab and gives its editor the focus.
fn focus_console(vcx: &VisualTestContext, id: ConsoleId) {
    activate_tab(vcx, tab_of(vcx, id));
    in_window(vcx, |window, cx| {
        // GPUI reports focus only for the active window.
        window.activate_window();
        let editor = Consoles::get(cx, id)
            .and_then(|s| s.editor.clone())
            .expect("the console has an editor");
        let handle = editor.read(cx).focus_handle(cx);
        window.focus(&handle, cx);
        // Focus events go out when the next frame is drawn.
        window.refresh();
    });
    vcx.run_until_parked();
}

/// Activates `tab` in the focused pane as Ctrl+Tab does: the focus does not move.
fn activate_tab(vcx: &VisualTestContext, tab: TabId) {
    let tabs = tab_count(vcx);
    for _ in 0..tabs {
        if with_ws(vcx, |ws, _, _| ws.active_tab()) == Some(tab) {
            break;
        }
        with_ws(vcx, |ws, window, cx| ws.activate_next_tab(window, cx));
        vcx.run_until_parked();
    }
    assert_eq!(with_ws(vcx, |ws, _, _| ws.active_tab()), Some(tab));
}

/// The tab of console `id`.
fn tab_of(vcx: &VisualTestContext, id: ConsoleId) -> TabId {
    vcx.cx
        .read(|cx| Consoles::get(cx, id).map(|s| s.tab))
        .expect("a registered console")
}

/// The workspace's first tab: `Console 1`, unbound.
const UNBOUND_TAB: TabId = TabId(0);

/// Takes the focus away from every editor.
fn blur(vcx: &VisualTestContext) {
    in_window(vcx, |window, cx| {
        window.blur(cx);
        window.refresh();
    });
    vcx.run_until_parked();
}

/// Every text the status bar drew (the shared segments in `hacivat-ui`, the connection and
/// console segments of the app, the cursor button), joined by spaces.
fn status(vcx: &VisualTestContext) -> String {
    registry_frame(vcx)
        .into_iter()
        .filter(|r| {
            r.path.contains("hacivat-ui/src/statusbar.rs:")
                || r.path.contains("hacivat-app/src/connections/status.rs:")
                || r.path.ends_with("#button-label-status-cursor")
        })
        .map(|r| r.text.to_string())
        .collect::<Vec<_>>()
        .join(" ")
}

fn capsule_name(vcx: &VisualTestContext) -> (String, EnvKind) {
    match capsule_model(vcx) {
        CapsuleModel::Connection { name, env, .. } => (name.to_string(), env),
        other => panic!("not a connection: {other:?}"),
    }
}

fn snapshot(h: &SessionHandle) -> SessionSnapshot {
    h.state().borrow().clone()
}

fn user_exec(h: &SessionHandle, sql: &str) {
    let ticket = h.submit(Request::Exec {
        sql: sql.to_string(),
        opts: ExecOpts::default(),
        origin: ExecOrigin::User,
    });
    let reply = futures::executor::block_on(ticket.reply).expect("the actor replies");
    assert!(reply.is_ok(), "{sql}: {reply:?}");
}

fn int8() -> LogicalType {
    LogicalType::Int {
        bits: 64,
        signed: true,
    }
}

fn id_cell(row: u64, _: usize) -> Value {
    Value::Int(row as i64)
}

/// A scripted typed query answering `count` generated rows to the text starting `prefix`.
fn select(prefix: &'static str, count: u64) -> FakeQuery {
    FakeQuery {
        prefix,
        sets: vec![FakeResultSet {
            columns: vec![("id", int8())],
            rows: FakeRows::Generated {
                count,
                cell: id_cell,
            },
        }],
        ..Default::default()
    }
}

/// Opens a real store for each result of a run.
struct Stores {
    budget: Arc<MemoryBudget>,
    next: AtomicU64,
    events: Mutex<Vec<RunEvent>>,
}

impl Stores {
    fn new() -> Arc<Stores> {
        Arc::new(Stores {
            budget: MemoryBudget::new(1 << 30, 256 << 20),
            next: AtomicU64::new(1),
            events: Mutex::new(Vec::new()),
        })
    }
}

impl RunObserver for Stores {
    fn open_result(&self, _: UnitId, _: u32, schema: Arc<ResultSchema>) -> ResultWriter {
        let id = ResultId(self.next.fetch_add(1, Ordering::SeqCst));
        let (_store, writer) = ResultStore::new(id, schema, self.budget.clone(), 0);
        writer
    }

    fn event(&self, ev: &RunEvent) {
        self.events.lock().unwrap().push(ev.clone());
    }
}

/// Submits a one-unit run of `sql`.
fn run_one(
    h: &SessionHandle,
    sql: &str,
    suspendable: bool,
) -> hacivat_session::session::RequestTicket {
    let run = RunId(1);
    h.run(RunSpec {
        run,
        units: vec![RunUnit {
            id: UnitId { run, ordinal: 0 },
            sql: sql.into(),
            protocol: Protocol::Extended,
            suspendable,
        }]
        .into(),
        policy: ErrorPolicy::StopOnError,
        fetch: FetchPolicy::default(),
        observer: ObserverRef(Stores::new()),
    })
}

/// A source command sink for fixture results (nothing to fetch).
struct NoControl;

impl FetchControl for NoControl {
    fn fetch_more(&self, _: ResultId, _: u64) {}
    fn close(&self, _: ResultId, _: CloseReason) {}
    fn fetch_all(&self, _: ResultId) {}
}

/// Gives console `id` one finished result tab of `rows` rows, timed 12 ms + 119 ms, and makes
/// it the active tab.
fn show_result(vcx: &VisualTestContext, id: ConsoleId, rows: usize) {
    show_result_in(vcx, id, rows, Completeness::Complete);
}

/// [`show_result`] with the result in state `completeness`.
fn show_result_in(vcx: &VisualTestContext, id: ConsoleId, rows: usize, completeness: Completeness) {
    let schema = Arc::new(ResultSchema {
        columns: vec![ColumnMeta {
            name: "id".into(),
            logical: int8(),
            native: NativeType::Pg {
                oid: 20,
                typmod: -1,
                name: "int8".into(),
            },
            origin: None,
        }]
        .into(),
    });
    let values = (0..rows).map(|r| vec![Value::Int(r as i64)]).collect();
    let store = ResultStore::from_rows(schema, values, completeness);
    let source = StoreSource::new(store, Arc::new(NoControl), FetchPolicy::default());
    let key = TabKey {
        console: id.0,
        unit_ordinal: 0,
        result_ordinal: 0,
    };
    let tab = ResultTab {
        key,
        title: "".into(),
        unit: UnitId {
            run: RunId(1),
            ordinal: 0,
        },
        source,
    };
    vcx.cx.update(|cx| {
        let results = Consoles::get(cx, id)
            .and_then(|s| s.results.clone())
            .expect("the console has results");
        results.update(cx, |r, cx| {
            r.apply(&[tab], cx);
            r.set_timing(key, Some(12), Some(119), cx);
            r.set_active(ActiveResult::Tab(0), cx);
        });
    });
    vcx.run_until_parked();
}

fn set_active(vcx: &VisualTestContext, id: ConsoleId, active: ActiveResult) {
    vcx.cx.update(|cx| {
        let results = Consoles::get(cx, id)
            .and_then(|s| s.results.clone())
            .expect("the console has results");
        results.update(cx, |r, cx| r.set_active(active, cx));
    });
    vcx.run_until_parked();
}

// ---------------------------------------------------------------------------------------------
// The capsule follows the focused console

#[gpui::test]
fn focusing_a_production_or_staging_console_swaps_the_capsule(cx: &mut TestAppContext) {
    let vcx = shell_two(cx);
    let prod = console_of(vcx, PROD);
    let staging = console_of(vcx, STAGING);
    // No editor has the focus yet; the staging console's tab, opened last, is the active one.
    assert_eq!(capsule_name(vcx), (STAGING.to_string(), EnvKind::Staging));

    focus_console(vcx, prod);
    assert_eq!(capsule_name(vcx), (PROD.to_string(), EnvKind::Production));
    let s = status(vcx);
    assert!(s.contains("production · read-only (guarded)"), "{s}");
    assert!(!s.contains("staging"), "{s}");

    focus_console(vcx, staging);
    assert_eq!(capsule_name(vcx), (STAGING.to_string(), EnvKind::Staging));
    let s = status(vcx);
    assert!(s.contains("staging · read-write"), "{s}");
    assert!(!s.contains("production"), "{s}");

    // Back to production; the focus then leaves the editor for the results (no explorer
    // pick): the console the user was in is still the one described.
    focus_console(vcx, prod);
    blur(vcx);
    assert_eq!(capsule_name(vcx), (PROD.to_string(), EnvKind::Production));

    // Picking a connection in the explorer hands the capsule back to the M2 rule.
    let staging_id = profile_id(vcx, STAGING);
    vcx.cx
        .update(|cx| ConnectionsModel::select(cx, Some(staging_id)));
    vcx.run_until_parked();
    assert_eq!(capsule_name(vcx), (STAGING.to_string(), EnvKind::Staging));
    stop_core_runtime_test(cx);
}

// ---------------------------------------------------------------------------------------------
// The environment and rows follow the active result tab

#[gpui::test]
fn switching_result_tabs_swaps_the_env_segment(cx: &mut TestAppContext) {
    let vcx = shell_two(cx);
    let prod = console_of(vcx, PROD);
    let staging = console_of(vcx, STAGING);
    show_result(vcx, prod, 3);
    show_result(vcx, staging, 5);

    focus_console(vcx, prod);
    let s = status(vcx);
    assert!(s.contains("production · read-only (guarded)"), "{s}");
    assert!(s.contains("3 rows · 131 ms"), "{s}");

    // Switching to the staging console's result swaps the environment and the rows together.
    focus_console(vcx, staging);
    let s = status(vcx);
    assert!(s.contains("staging · read-write"), "{s}");
    assert!(s.contains("5 rows · 131 ms"), "{s}");
    assert!(!s.contains("production") && !s.contains("3 rows"), "{s}");

    // The Output tab has no result: the console's own segments, without rows.
    set_active(vcx, staging, ActiveResult::Output);
    let s = status(vcx);
    assert!(s.contains("staging · read-write"), "{s}");
    assert!(!s.contains("rows"), "{s}");

    // A result tab keeps the access recorded while its connection was up (fail closed: a
    // drop never upgrades it), while the console's own segments say the connection is gone.
    set_active(vcx, staging, ActiveResult::Tab(0));
    let staging_id = profile_id(vcx, STAGING);
    vcx.cx
        .update(|cx| ConnectionsModel::disconnect(cx, staging_id));
    wait_until(vcx, "staging to disconnect", |vcx| {
        live(vcx, STAGING) == LiveState::Disconnected
    });
    let s = status(vcx);
    assert!(s.contains("staging · read-write"), "{s}");
    assert!(s.contains("5 rows · 131 ms"), "{s}");
    set_active(vcx, staging, ActiveResult::Output);
    let s = status(vcx);
    assert!(s.contains("No connection"), "{s}");
    stop_core_runtime_test(cx);
}

// ---------------------------------------------------------------------------------------------
// Transactions and cursors

#[gpui::test]
fn transaction_segment_only_for_user_transactions(cx: &mut TestAppContext) {
    let vcx = shell_two(cx);
    let staging_id = profile_id(vcx, STAGING);

    // The user's transaction: shown with its statements.
    let (user, _) = session_over(vcx, staging_id, pg().tracks_transactions());
    let user_console = register_test_console(vcx, staging_id, user.clone());
    focus_console(vcx, user_console);
    assert!(!status(vcx).contains("Transaction"), "{}", status(vcx));
    user_exec(&user, "BEGIN");
    user_exec(&user, "UPDATE t SET a = 1");
    let tx = snapshot(&user).tx;
    assert_eq!((tx.status, tx.owner), (TxStatus::InTx, TxOwner::User));
    // BEGIN is transaction control, not a statement of the transaction (M10).
    wait_until(vcx, "the transaction segment", |vcx| {
        status(vcx).contains("Transaction open · 1 statement")
    });
    user_exec(&user, "UPDATE t SET b = 2");
    wait_until(vcx, "the second statement", |vcx| {
        status(vcx).contains("Transaction open · 2 statements")
    });
    user_exec(&user, "COMMIT");
    wait_until(vcx, "the segment to go", |vcx| {
        !status(vcx).contains("Transaction")
    });

    // Hacivat's own transaction around a PostgreSQL cursor (owner Cursor) is not the user's.
    // (Staging: a production profile starts in Manual mode, where every transaction is the
    // user's.)
    let (cursor, _) = session_over(
        vcx,
        staging_id,
        pg().tracks_transactions()
            .queries(vec![select("SELECT", 1_000_000)]),
    );
    let cursor_console = register_test_console(vcx, staging_id, cursor.clone());
    let _ticket = run_one(&cursor, "SELECT * FROM big", true);
    poll_until("the cursor to open", || snapshot(&cursor).cursor.is_some());
    let tx = snapshot(&cursor).tx;
    assert_eq!((tx.status, tx.owner), (TxStatus::InTx, TxOwner::Cursor));
    focus_console(vcx, cursor_console);
    wait_until(vcx, "the cursor segment", |vcx| {
        status(vcx).contains("Cursor open")
    });
    assert!(!status(vcx).contains("Transaction"), "{}", status(vcx));
    stop_core_runtime_test(cx);
}

#[gpui::test]
fn failed_transaction_segment_says_so(cx: &mut TestAppContext) {
    // The ledger's case (T18 ruling 4 / M6): a PostgreSQL unit that cannot suspend, stopped
    // inside the user's transaction after the 1 s grace, fails that transaction; the status
    // bar says so (P16), and never "Transaction open".
    let vcx = shell_two(cx);
    let staging_id = profile_id(vcx, STAGING);
    let slow = FakeQuery {
        first_delay: Duration::from_secs(60),
        ..select("UPDATE", 10)
    };
    let (h, fake) = session_over(
        vcx,
        staging_id,
        pg().tracks_transactions().queries(vec![slow]),
    );
    let console = register_test_console(vcx, staging_id, h.clone());
    focus_console(vcx, console);
    user_exec(&h, "BEGIN");
    let ticket = run_one(&h, "UPDATE t SET a = 1 RETURNING *", false);
    poll_until("the unit to reach the driver", || {
        fake.typed_log()
            .iter()
            .any(|c| matches!(c, FakeTypedCall::Query(_)))
    });
    let _cancel = ticket.cancel(&h);
    wait_until(vcx, "the transaction to fail", |_| {
        snapshot(&h).tx.status == TxStatus::Failed
    });
    assert_eq!(snapshot(&h).tx.owner, TxOwner::User);
    wait_until(vcx, "the failed segment", |vcx| {
        status(vcx).contains("Transaction failed · roll back to end it")
    });
    assert!(!status(vcx).contains("Transaction open"), "{}", status(vcx));
    stop_core_runtime_test(cx);
}

#[gpui::test]
fn cursor_open_segment_closes_the_cursor(cx: &mut TestAppContext) {
    let vcx = shell_two(cx);
    let staging_id = profile_id(vcx, STAGING);
    let (h, fake) = session_over(
        vcx,
        staging_id,
        pg().tracks_transactions()
            .queries(vec![select("SELECT", 1_000_000)]),
    );
    let console = register_test_console(vcx, staging_id, h.clone());
    focus_console(vcx, console);
    assert!(!status(vcx).contains("Cursor open"), "{}", status(vcx));
    let _ticket = run_one(&h, "SELECT * FROM big", true);
    poll_until("the cursor to open", || snapshot(&h).cursor.is_some());
    wait_until(vcx, "the cursor segment", |vcx| {
        status(vcx).contains("Cursor open")
    });

    click(vcx, "button-status-cursor");
    wait_until(vcx, "the cursor to close", |_| {
        snapshot(&h).cursor.is_none()
    });
    assert!(
        fake.typed_log()
            .iter()
            .any(|c| matches!(c, FakeTypedCall::Close(_))),
        "{:?}",
        fake.typed_log()
    );
    wait_until(vcx, "the segment to go", |vcx| {
        !status(vcx).contains("Cursor open")
    });
    stop_core_runtime_test(cx);
}

// ---------------------------------------------------------------------------------------------
// The narrow window

#[gpui::test]
fn at_960_in_turkish_the_trust_segments_keep_their_place(cx: &mut TestAppContext) {
    // NOTES surface 6, "960 in Turkish with everything on": the environment, the transaction
    // and the cursor never give way; the version and the encryption do first.
    let vcx = shell_with_fake(cx, "tr", pg());
    add_profile(
        vcx,
        test_profile(STAGING, Some(EnvTag::Staging), "10.0.4.13"),
    );
    connect_and_wait(vcx, STAGING);
    let staging_id = profile_id(vcx, STAGING);
    // The user's transaction, and a cursor inside it (a savepoint of Hacivat's).
    let (h, _) = session_over(
        vcx,
        staging_id,
        pg().tracks_transactions()
            .queries(vec![select("SELECT", 1_000_000)]),
    );
    let console = register_test_console(vcx, staging_id, h.clone());
    user_exec(&h, "BEGIN");
    let now = std::time::Instant::now();
    show_result_in(
        vcx,
        console,
        10_000,
        Completeness::Suspended(CursorInfo {
            opened_at: now,
            last_fetch: now,
            holds_tx: true,
            idle_bound: Duration::from_secs(600),
        }),
    );
    let _ticket = run_one(&h, "SELECT * FROM big", true);
    poll_until("the cursor to open", || snapshot(&h).cursor.is_some());
    focus_console(vcx, console);
    vcx.simulate_resize(gpui::size(gpui::px(960.0), gpui::px(600.0)));
    wait_until(vcx, "the transaction and cursor segments", |vcx| {
        let s = status(vcx);
        s.contains("İşlem açık") && s.contains("İmleç açık")
    });
    let bar = vcx.debug_bounds("status-bar").expect("the status bar");
    let env = vcx.debug_bounds("status-env").expect("the environment");
    let tx = vcx.debug_bounds("status-tx").expect("the transaction");
    let cursor = vcx
        .debug_bounds("button-status-cursor")
        .expect("the cursor button");
    assert!(env.right() <= tx.left(), "{env:?} {tx:?}");
    assert!(tx.right() <= cursor.left(), "{tx:?} {cursor:?}");
    assert!(cursor.right() <= bar.right(), "{cursor:?} in {bar:?}");
    let s = status(vcx);
    assert!(s.contains("hazırlık · okuma-yazma"), "{s}");
    // More rows may come: "10,000+".
    assert!(s.contains("+ satır · 131 ms"), "{s}");
    // The cells that gave way leave no trace: never two seams side by side, at 960 and at
    // every width up to where nothing gives way (odd steps, so some cell is caught mid-way).
    #[cfg(feature = "scene-check")]
    for width in (960..=1400).step_by(7) {
        vcx.simulate_resize(gpui::size(gpui::px(width as f32), gpui::px(600.0)));
        vcx.run_until_parked();
        let bar = vcx.debug_bounds("status-bar").expect("the status bar");
        // Without the language segment's own seam, the last: a full bar's last left segment
        // meets it with no room between (the M2b bar).
        let mut seams = visible_seams(vcx, bar);
        seams.pop();
        assert!(seams.len() >= 3, "{width}: {seams:?}");
        assert!(
            seams.windows(2).all(|w| w[1] - w[0] > 3.0),
            "{width}: stacked seams at {seams:?}"
        );
    }
    stop_core_runtime_test(cx);
}

/// The x positions (logical px) of the seams the last frame shows inside `bar`: 1 px fills and
/// left or right borders in the `seam` colour, where their content mask lets them show.
#[cfg(feature = "scene-check")]
fn visible_seams(vcx: &mut VisualTestContext, bar: gpui::Bounds<gpui::Pixels>) -> Vec<f32> {
    use hacivat_ui_core::theme::Theme;
    let seam = vcx
        .cx
        .read(|cx| gpui::Rgba::from(Theme::get(cx).tokens.colors.seam.to_hsla()));
    let same = |c: gpui::Hsla| {
        let c = gpui::Rgba::from(c);
        (c.r - seam.r).abs() < 0.01
            && (c.g - seam.g).abs() < 0.01
            && (c.b - seam.b).abs() < 0.01
            && c.a > 0.5
    };
    vcx.update(|window, _| {
        let scale = window.scale_factor();
        let (top, bottom) = (f32::from(bar.top()), f32::from(bar.bottom()));
        let mut xs = Vec::new();
        for q in &window.rendered_scene().quads {
            let (y, h) = (q.bounds.origin.y.0 / scale, q.bounds.size.height.0 / scale);
            if y + h <= top + 2.0 || y >= bottom {
                continue;
            }
            let (left, width) = (q.bounds.origin.x.0 / scale, q.bounds.size.width.0 / scale);
            let mut at = Vec::new();
            if q.background.as_solid().is_some_and(same) && width <= 1.5 {
                at.push(left);
            }
            if same(q.border_color) {
                if q.border_widths.left.0 > 0.0 {
                    at.push(left);
                }
                if q.border_widths.right.0 > 0.0 {
                    at.push(left + width - 1.0);
                }
            }
            let mask_left = q.content_mask.bounds.origin.x.0 / scale;
            let mask_right = mask_left + q.content_mask.bounds.size.width.0 / scale;
            xs.extend(
                at.into_iter()
                    .filter(|x| *x + 1.0 > mask_left && *x < mask_right),
            );
        }
        xs.sort_by(f32::total_cmp);
        xs.dedup_by(|a, b| (*a - *b).abs() < 0.01);
        xs
    })
}

// ---------------------------------------------------------------------------------------------
// The transient message

#[gpui::test]
fn the_editors_sentence_shows_until_the_text_changes(cx: &mut TestAppContext) {
    let vcx = shell_two(cx);
    let staging = console_of(vcx, STAGING);
    focus_console(vcx, staging);
    assert_eq!(text_of(vcx, "status-message"), None);
    let editor = vcx
        .cx
        .read(|cx| Consoles::get(cx, staging).and_then(|s| s.editor.clone()))
        .expect("an editor");
    in_window(vcx, |window, cx| {
        editor.update(cx, |v, cx| {
            assert!(v.run_command("editor.describe_exec_range", window, cx));
        });
    });
    vcx.run_until_parked();
    let said = vcx
        .cx
        .read(|cx| hacivat_ui_editor::commands::status_message(editor.read(cx)))
        .expect("the editor said a sentence");
    let shown = text_of(vcx, "status-message").expect("the status bar shows it");
    assert_eq!(shown, said.replace(['\u{2068}', '\u{2069}'], ""));
    // Typing ends the sentence (the editor's rule); the bar follows.
    vcx.simulate_keystrokes("x");
    assert_eq!(text_of(vcx, "status-message"), None);
    stop_core_runtime_test(cx);
}

// ---------------------------------------------------------------------------------------------
// Fix round 1: the active tab (C1), the session's own access (I2), a session that ended (I3),
// session swaps and closed consoles (M8), the preview sentence (M7)

#[gpui::test]
fn a_tab_switch_without_focus_follows_the_active_console(cx: &mut TestAppContext) {
    // C1: Ctrl+Tab (or a click on the tab) moves no focus. From a focused staging console to
    // the production console's tab, the bar and the capsule must say production.
    let vcx = shell_two(cx);
    let prod = console_of(vcx, PROD);
    let staging = console_of(vcx, STAGING);
    focus_console(vcx, staging);
    assert_eq!(capsule_name(vcx), (STAGING.to_string(), EnvKind::Staging));

    activate_tab(vcx, tab_of(vcx, prod));
    assert_eq!(capsule_name(vcx), (PROD.to_string(), EnvKind::Production));
    let s = status(vcx);
    assert!(s.contains("production · read-only (guarded)"), "{s}");
    assert!(!s.contains("staging"), "{s}");

    // And back, still without moving the focus.
    activate_tab(vcx, tab_of(vcx, staging));
    assert_eq!(capsule_name(vcx), (STAGING.to_string(), EnvKind::Staging));

    // A tab that is not a bound console: the M2 rule. Production is picked in the explorer
    // first, so the M2 answer differs from the console that was shown.
    let prod_id = profile_id(vcx, PROD);
    vcx.cx
        .update(|cx| ConnectionsModel::select(cx, Some(prod_id)));
    vcx.run_until_parked();
    activate_tab(vcx, UNBOUND_TAB);
    assert_eq!(capsule_name(vcx), (PROD.to_string(), EnvKind::Production));
    stop_core_runtime_test(cx);
}

#[gpui::test]
fn the_console_session_access_wins_when_it_protects_less(cx: &mut TestAppContext) {
    // I2: the profile's default connection reads read-only (the production policy), but this
    // console's own session is read-write (opened before the profile changed, say). The bar
    // and the capsule must never claim the protection the session does not enforce.
    let vcx = shell_two(cx);
    let prod_id = profile_id(vcx, PROD);
    let (h, _) = session_with(vcx, prod_id, pg(), Some(false));
    assert!(!snapshot(&h).read_only);
    let console = register_test_console(vcx, prod_id, h);
    focus_console(vcx, console);
    let s = status(vcx);
    assert!(s.contains("production · read-write"), "{s}");
    assert!(!s.contains("read-only"), "{s}");
    match capsule_model(vcx) {
        CapsuleModel::Connection {
            access, read_only, ..
        } => {
            assert_eq!(access.as_ref(), "read-write");
            assert!(!read_only);
        }
        other => panic!("not a connection: {other:?}"),
    }
    // Round 2 (N1): nothing explains a read-only access the session does not have: the
    // palette's "Explain read-only access" is off and runs nothing.
    assert!(!vcx.cx.read(capsule::has_access_explanation));
    assert!(
        vcx.cx
            .read(|cx| access_popover::facts_for(cx, prod_id))
            .is_none()
    );
    assert!(!run(vcx, "connection.explain_access"));
    assert!(!layer_ids(vcx).iter().any(|l| l.contains("popover")));
    // A result tab's recorded label does not outrank the session either.
    show_result(vcx, console, 3);
    let s = status(vcx);
    assert!(s.contains("production · read-write"), "{s}");
    assert!(s.contains("3 rows"), "{s}");
    stop_core_runtime_test(cx);
}

#[gpui::test]
fn a_guarded_session_is_never_explained_as_enforced(cx: &mut TestAppContext) {
    // Round 2 (N1): the default connection reads "enforced by the server's role"; this
    // console's session is only guarded. The bar, the capsule and the explanation say guarded.
    let vcx = shell_two(cx);
    let prod_id = profile_id(vcx, PROD);
    vcx.cx.update(|cx| {
        let m = cx.global_mut::<ConnectionsModel>();
        let LiveState::Connected(mut info) = m.live(prod_id) else {
            panic!("production is connected");
        };
        std::sync::Arc::make_mut(&mut info).access = AccessLabel::ReadOnlyEnforced;
        m.set_live(prod_id, LiveState::Connected(info));
    });
    vcx.run_until_parked();
    let (h, _) = session_over(vcx, prod_id, pg());
    assert_eq!(snapshot(&h).access, AccessLabel::ReadOnlyGuarded);
    let console = register_test_console(vcx, prod_id, h);
    focus_console(vcx, console);
    let s = status(vcx);
    assert!(s.contains("production · read-only (guarded)"), "{s}");
    assert!(!s.contains("enforced"), "{s}");
    match capsule_model(vcx) {
        CapsuleModel::Connection { access, .. } => {
            assert_eq!(access.as_ref(), "read-only (guarded)");
        }
        other => panic!("not a connection: {other:?}"),
    }
    assert!(vcx.cx.read(capsule::has_access_explanation));
    let facts = vcx
        .cx
        .read(|cx| access_popover::facts_for(cx, prod_id))
        .expect("a read-only connection is explained");
    assert_eq!(facts.access, AccessLabel::ReadOnlyGuarded);
    // Without a console the default connection is described again, as it is: enforced.
    activate_tab(vcx, UNBOUND_TAB);
    let facts = vcx
        .cx
        .read(|cx| access_popover::facts_for(cx, prod_id))
        .expect("a read-only connection is explained");
    assert_eq!(facts.access, AccessLabel::ReadOnlyEnforced);
    stop_core_runtime_test(cx);
}

#[gpui::test]
fn a_closed_session_shows_no_transaction(cx: &mut TestAppContext) {
    // I3: closing rolls the user's transaction back; the session publishes Disconnected but
    // keeps the last transaction state. The bar must not show a transaction that is gone.
    let vcx = shell_two(cx);
    let staging_id = profile_id(vcx, STAGING);
    let (h, _) = session_over(vcx, staging_id, pg().tracks_transactions());
    let console = register_test_console(vcx, staging_id, h.clone());
    focus_console(vcx, console);
    user_exec(&h, "BEGIN");
    user_exec(&h, "UPDATE t SET a = 1");
    wait_until(vcx, "the transaction segment", |vcx| {
        status(vcx).contains("Transaction open")
    });
    futures::executor::block_on(h.close(ClosePolicy::RollBack)).expect("the session closes");
    wait_until(vcx, "the segment to go", |vcx| {
        !status(vcx).contains("Transaction")
    });
    stop_core_runtime_test(cx);
}

#[gpui::test]
fn a_new_session_for_the_console_is_followed_and_a_closed_console_is_not_watched(
    cx: &mut TestAppContext,
) {
    // M8: rebinding a console to another session watches the new one; closing a console
    // drops its watcher.
    let vcx = shell_two(cx);
    let staging_id = profile_id(vcx, STAGING);
    let (old, _) = session_over(vcx, staging_id, pg().tracks_transactions());
    let console = register_test_console(vcx, staging_id, old.clone());
    focus_console(vcx, console);
    assert_eq!(
        vcx.cx
            .read(hacivat_app::connections::status::watched_sessions),
        1
    );
    user_exec(&old, "BEGIN");
    wait_until(vcx, "the old session's transaction", |vcx| {
        status(vcx).contains("Transaction open · 0 statements")
    });

    let (new, _) = session_over(vcx, staging_id, pg().tracks_transactions());
    vcx.cx.update(|cx| {
        Consoles::update(cx, console, |s| s.session = Some(new.clone()));
    });
    wait_until(vcx, "the old transaction to leave the bar", |vcx| {
        !status(vcx).contains("Transaction")
    });
    user_exec(&new, "BEGIN");
    user_exec(&new, "UPDATE t SET a = 1");
    wait_until(vcx, "the new session's transaction", |vcx| {
        status(vcx).contains("Transaction open · 1 statement")
    });
    // The old session changes no more than the bar's mirror of it: it is not watched.
    user_exec(&old, "UPDATE t SET a = 1");
    user_exec(&old, "UPDATE t SET a = 2");
    vcx.run_until_parked();
    assert!(
        status(vcx).contains("Transaction open · 1 statement"),
        "{}",
        status(vcx)
    );
    assert_eq!(
        vcx.cx
            .read(hacivat_app::connections::status::watched_sessions),
        1
    );

    vcx.cx.update(|cx| Consoles::remove(cx, console));
    vcx.run_until_parked();
    assert_eq!(
        vcx.cx
            .read(hacivat_app::connections::status::watched_sessions),
        0
    );
    stop_core_runtime_test(cx);
}

#[gpui::test]
fn the_preview_sentence_shows_when_there_is_nothing_to_run(cx: &mut TestAppContext) {
    // M7: Task 28's `preview::status_message` reaches the bar ("Nothing to run here").
    let vcx = shell_two(cx);
    let staging = console_of(vcx, STAGING);
    // What the console host does once the console is connected (Tasks 32 and 33): the editor
    // knows it is connected, the language service has the console's dialect.
    in_window(vcx, |_, cx| {
        let state = Consoles::get(cx, staging).expect("the console").clone();
        let editor = state.editor.expect("an editor");
        let service = state.service.expect("its language service");
        let doc = editor.read(cx).doc();
        editor.update(cx, |v, cx| {
            v.set_doc_context(
                hacivat_ui_editor::DocContext {
                    connected: true,
                    tier: v.status().tier,
                },
                cx,
            );
        });
        service.set_sql_context(
            doc,
            sql_dialect::DialectCtx::new(sql_dialect::DialectId::Postgres, None),
            sql_dialect::ClientFlavor::Psql,
            cx,
        );
    });
    focus_console(vcx, staging);
    vcx.simulate_keystrokes("- - space x enter enter");
    settle_until(vcx, Duration::from_secs(10), || {
        text_of(vcx, "status-message").as_deref() == Some("Nothing to run here")
    });
    stop_core_runtime_test(cx);
}

// ---------------------------------------------------------------------------------------------
// M2 behaviour without a console

#[gpui::test]
fn without_a_console_the_m2_behaviour_is_unchanged(cx: &mut TestAppContext) {
    let vcx = shell_two(cx);
    // The last connect attempt (staging), with its M2 segments.
    assert_eq!(capsule_name(vcx), (STAGING.to_string(), EnvKind::Staging));
    let s = status(vcx);
    assert!(s.contains("staging · read-write"), "{s}");
    assert!(s.contains("unencrypted"), "{s}");
    assert!(!s.contains("Transaction") && !s.contains("rows"), "{s}");
    // The explorer's selection while it has a live state.
    let prod_id = profile_id(vcx, PROD);
    vcx.cx
        .update(|cx| ConnectionsModel::select(cx, Some(prod_id)));
    vcx.run_until_parked();
    assert_eq!(capsule_name(vcx), (PROD.to_string(), EnvKind::Production));
    assert!(
        status(vcx).contains("production · read-only (guarded)"),
        "{}",
        status(vcx)
    );
    // A console whose tab is not the active one changes nothing; the unbound console is not
    // followed either.
    let _staging = console_of(vcx, STAGING);
    activate_tab(vcx, UNBOUND_TAB);
    assert_eq!(capsule_name(vcx), (PROD.to_string(), EnvKind::Production));
    assert!(
        status(vcx).contains("production · read-only (guarded)"),
        "{}",
        status(vcx)
    );
    stop_core_runtime_test(cx);
}
