//! M3b Task 22: the transaction UI (D-014, D-036; `design/m3b-safe` surfaces 3, 4, 5 and 12).
//!
//! - The mode chip sets the session's mode and isolation through its menu; while the user's
//!   transaction is open the menu's modes and Isolation are disabled with their reason (ruling
//!   10); a read-only session shows a disabled Auto-commit chip.
//! - The badge shows only the user's transaction (`TxOwner::User`), never Hacivat's own around a
//!   cursor; a failed transaction offers only Roll back; Commit and Roll back wait while the
//!   session runs the user's request.
//! - The idle toast: Keep open sends nothing; a newer reminder replaces the older one.
//! - The single-session checkbox writes through `ConnectionsModel`.
//! - The badge, the chip and the status segment are never clipped at 960 px in Turkish.
//!
//! Consoles are `register_test_console` (a real editor over a FakeDriver session on the shell's
//! core runtime). Every wait is on a condition.

mod support;

use std::time::Duration;

use gpui::{Focusable as _, TestAppContext, VisualTestContext};
use hacivat_app::connections::ConnectionsModel;
use hacivat_app::console::{ConsoleId, Consoles, idle_toast, tx_badge, tx_chip};
use hacivat_driver::fake::{FakeDriver, FakeExec, FakeQuery, FakeResultSet, FakeRows};
use hacivat_driver::{ExecOpts, ExecSummary, Protocol, RawError, RawErrorKind, TxStatus};
use hacivat_l10n::ids;
use hacivat_proto::{DbmsKind, EnvTag, ProfileId};
use hacivat_results::{FetchPolicy, MemoryBudget, ResultId, ResultStore, ResultWriter};
use hacivat_session::LiveState;
use hacivat_session::session::{
    ClosePolicy, ErrorPolicy, ExecOrigin, IsolationLevel, ObserverRef, Request, RunId, RunSpec,
    RunUnit, SessionHandle, SessionSnapshot, TxMode, TxOwner, UnitId,
};
use hacivat_session::session::{RunEvent, RunObserver};
use hacivat_ui::text::RenderedText;
use hacivat_values::{LogicalType, ResultSchema, Value};
use support::*;

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
    assert!(matches!(live(vcx, name), LiveState::Connected(_)));
}

/// A shell in `lang` with `orders-stg` (staging) saved and connected; Console 1 stays unbound
/// (the connect happens with the settings tab in front).
fn shell_staging(cx: &mut TestAppContext, lang: &str) -> &'static mut VisualTestContext {
    let vcx = shell_with_fake(cx, lang, pg());
    add_profile(
        vcx,
        test_profile(STAGING, Some(EnvTag::Staging), "10.0.4.13"),
    );
    with_ws(vcx, |ws, window, cx| {
        ws.open_registered_tab("settings", window, cx)
    });
    connect_and_wait(vcx, STAGING);
    vcx
}

/// A console session of profile `id` over `fake`, read-only as `read_only` says.
fn session_with(
    vcx: &VisualTestContext,
    id: ProfileId,
    fake: FakeDriver,
    read_only: bool,
) -> SessionHandle {
    session_and_fake(vcx, id, fake, read_only).0
}

/// [`session_with`], with the session's fake (its call log).
fn session_and_fake(
    vcx: &VisualTestContext,
    id: ProfileId,
    fake: FakeDriver,
    read_only: bool,
) -> (SessionHandle, std::sync::Arc<FakeDriver>) {
    let (rt, mut profile) = vcx.cx.read(|cx| {
        let m = cx.global::<ConnectionsModel>();
        let profile = m.get(id).expect("a saved profile");
        (m.core_runtime().handle(), (*profile).clone())
    });
    profile.read_only = read_only;
    futures::executor::block_on(hacivat_session::testing::fake_session(rt, fake, profile))
}

/// A focused staging console over `fake` (read-write).
fn console_over(vcx: &VisualTestContext, fake: FakeDriver) -> (ConsoleId, SessionHandle) {
    let id = profile_id(vcx, STAGING);
    let h = session_with(vcx, id, fake, false);
    let console = register_test_console(vcx, id, h.clone());
    focus_console(vcx, console);
    wait_until(vcx, "the mirror", |vcx| {
        vcx.cx.read(|cx| tx_badge::view(console, cx).is_some())
    });
    (console, h)
}

fn focus_console(vcx: &VisualTestContext, id: ConsoleId) {
    in_window(vcx, |window, cx| {
        window.activate_window();
        let editor = Consoles::get(cx, id)
            .and_then(|s| s.editor.clone())
            .expect("the console has an editor");
        let handle = editor.read(cx).focus_handle(cx);
        window.focus(&handle, cx);
        window.refresh();
    });
    vcx.run_until_parked();
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
    let _ = futures::executor::block_on(ticket.reply).expect("the actor replies");
}

/// The drawn label with element ID `id`.
fn label(vcx: &VisualTestContext, id: &str) -> Option<RenderedText> {
    let needle = format!("#{id}");
    registry_frame(vcx)
        .into_iter()
        .find(|r| r.path.ends_with(&needle))
}

fn redraw(vcx: &VisualTestContext) {
    // ## m4-t33d (T33 flaky3 follow-up): the last frame renders every view (a frame after
    // `run_until_parked` may reuse a cached view, which records no texts).
    redraw_full(vcx);
}

fn t(vcx: &VisualTestContext, id: hacivat_l10n::MsgRef<hacivat_l10n::NoArgs>) -> String {
    vcx.cx
        .read(|cx| hacivat_app::l10n_global::tr(cx, id, hacivat_l10n::NoArgs).to_string())
}

fn menu_rows(vcx: &VisualTestContext, console: ConsoleId) -> Vec<(String, bool, Option<String>)> {
    vcx.cx.read(|cx| {
        tx_chip::menu_items(console, cx)
            .into_iter()
            .filter(|i| !i.separator)
            .map(|i| {
                (
                    (i.label)(cx).to_string(),
                    i.disabled,
                    i.hint.as_ref().map(|h| h(cx).to_string()),
                )
            })
            .collect()
    })
}

// ---------------------------------------------------------------------------------------------
// The chip

#[gpui::test]
fn chip_sets_mode_and_isolation(cx: &mut TestAppContext) {
    let vcx = shell_staging(cx, "en");
    let (console, h) = console_over(vcx, pg().tracks_transactions());
    wait_until(vcx, "the chip", |vcx| {
        text_of(vcx, "tx-chip-mode").as_deref() == Some("Auto-commit")
    });
    // The chip opens its menu: the modes with their hints, the current one checked.
    click(vcx, "tx-chip");
    assert_eq!(layer_ids(vcx), ["menu"]);
    let rows = menu_rows(vcx, console);
    assert_eq!(rows[0].0, "Auto-commit");
    assert_eq!(rows[1].0, "Manual");
    assert_eq!(rows[1].2.as_deref(), Some("changes wait for Commit"));
    assert!(rows.iter().take(4).all(|(_, disabled, _)| !disabled));
    // Down, Enter: Manual.
    vcx.simulate_keystrokes("down enter");
    vcx.run_until_parked();
    wait_until(vcx, "the session in Manual", |_| {
        snapshot(&h).tx.mode == TxMode::Manual
    });
    assert!(layer_ids(vcx).is_empty());
    wait_until(vcx, "the chip says Manual", |vcx| {
        text_of(vcx, "tx-chip-mode").as_deref() == Some("Manual")
    });
    // Isolation: two rows down from Manual (Smart, then Isolation past the separator), its own
    // menu, then Serializable at its end.
    click(vcx, "tx-chip");
    vcx.simulate_keystrokes("down down enter");
    vcx.run_until_parked();
    wait_until(vcx, "the isolation menu", |vcx| layer_ids(vcx) == ["menu"]);
    vcx.simulate_keystrokes("down down down down enter");
    vcx.run_until_parked();
    wait_until(vcx, "the session at SERIALIZABLE", |_| {
        snapshot(&h).tx.isolation == IsolationLevel::Serializable
    });
    let rows = menu_rows(vcx, console);
    let isolation = rows
        .iter()
        .find(|r| r.0 == "Isolation")
        .expect("the Isolation row");
    assert_eq!(isolation.2.as_deref(), Some("SERIALIZABLE"));
    // Back to Auto through the headless path the menu rows run.
    vcx.cx
        .update(|cx| tx_chip::choose_headless(console, "tx-chip:mode:auto", cx));
    wait_until(vcx, "the session in Auto", |_| {
        snapshot(&h).tx.mode == TxMode::Auto
    });
    stop_core_runtime_test(cx);
}

#[gpui::test]
fn chip_disabled_with_reason_while_tx_open(cx: &mut TestAppContext) {
    // NOTES ruling 10: the chip stays enabled while a transaction is open; its menu leads with
    // Commit / Roll back, and the modes and Isolation are disabled with the reason.
    let vcx = shell_staging(cx, "en");
    let (console, h) = console_over(vcx, pg().tracks_transactions());
    user_exec(&h, "BEGIN");
    user_exec(&h, "UPDATE t SET a = 1");
    wait_until(vcx, "the user's transaction", |vcx| {
        vcx.cx
            .read(|cx| tx_badge::user_tx(console, cx).is_some_and(|u| u.statements == 1))
    });
    let reason = t(vcx, ids::hv::TX_MENU_REASON_OPEN);
    let rows = menu_rows(vcx, console);
    assert_eq!(rows[0], ("Commit transaction".to_string(), false, None));
    assert_eq!(rows[1], ("Roll back transaction".to_string(), false, None));
    for (label, disabled, hint) in &rows[2..6] {
        assert!(*disabled, "{label}");
        assert_eq!(hint.as_deref(), Some(reason.as_str()), "{label}");
    }
    // The chip itself still opens (a read-write session).
    redraw(vcx);
    click(vcx, "tx-chip");
    assert_eq!(layer_ids(vcx), ["menu"]);
    vcx.simulate_keystrokes("escape");
    vcx.run_until_parked();
    // A mode change is never sent while the transaction is open.
    let before = snapshot(&h).tx.mode;
    vcx.cx.update(|cx| {
        tx_chip::send(
            console,
            hacivat_session::session::TxCommand::SetMode(TxMode::Manual),
            cx,
        )
    });
    vcx.run_until_parked();
    assert_eq!(snapshot(&h).tx.mode, before);
    // T19 carry: the refusal is said, never swallowed.
    let texts = toast_texts(vcx);
    let in_progress = vcx
        .cx
        .read(|cx| hacivat_app::l10n_global::tr_id(cx, "exec-error-tx-in-progress").to_string());
    assert!(
        texts.iter().any(|t| t.contains(in_progress.as_str())),
        "{in_progress} in {texts:?}"
    );

    // A read-only session: a disabled Auto-commit chip whose menu never opens.
    let id = profile_id(vcx, STAGING);
    let ro = session_with(vcx, id, pg().tracks_transactions(), true);
    let ro_console = register_test_console(vcx, id, ro);
    focus_console(vcx, ro_console);
    wait_until(vcx, "the read-only chip", |vcx| {
        vcx.cx
            .read(|cx| tx_badge::view(ro_console, cx).is_some_and(|v| v.read_only))
    });
    redraw(vcx);
    assert_eq!(text_of(vcx, "tx-chip-mode").as_deref(), Some("Auto-commit"));
    click(vcx, "tx-chip");
    assert!(layer_ids(vcx).is_empty());
    let rows = menu_rows(vcx, ro_console);
    assert!(rows.iter().all(|(_, disabled, _)| *disabled), "{rows:?}");
    stop_core_runtime_test(cx);
}

// ---------------------------------------------------------------------------------------------
// The badge

fn select(prefix: &'static str, count: u64) -> FakeQuery {
    FakeQuery {
        prefix,
        sets: vec![FakeResultSet {
            columns: vec![(
                "id",
                LogicalType::Int {
                    bits: 64,
                    signed: true,
                },
            )],
            rows: FakeRows::Generated {
                count,
                cell: id_cell,
            },
        }],
        ..Default::default()
    }
}

fn id_cell(row: u64, _col: usize) -> Value {
    Value::Int(i64::try_from(row).unwrap_or(i64::MAX))
}

/// Opens a real store for each result of a run.
struct Stores {
    budget: std::sync::Arc<MemoryBudget>,
    next: std::sync::atomic::AtomicU64,
}

impl Stores {
    fn new() -> Stores {
        Stores {
            budget: MemoryBudget::new(1 << 30, 256 << 20),
            next: std::sync::atomic::AtomicU64::new(1),
        }
    }
}

impl RunObserver for Stores {
    fn open_result(&self, _: UnitId, _: u32, schema: std::sync::Arc<ResultSchema>) -> ResultWriter {
        let id = ResultId(self.next.fetch_add(1, std::sync::atomic::Ordering::SeqCst));
        let (_store, writer) = ResultStore::new(id, schema, self.budget.clone(), 0);
        writer
    }

    fn event(&self, _: &RunEvent) {}
}

// ## m4-t33d (R-T33-1, M3b NOTES surface 4 "Keyboard": the badge buttons are Tab stops in the
// toolbar): Commit, then Roll back; Enter on the focused Commit commits.
#[gpui::test]
fn the_badge_buttons_are_tab_stops(cx: &mut TestAppContext) {
    let vcx = shell_staging(cx, "en");
    let (_console, h) = console_over(vcx, pg().tracks_transactions());
    redraw(vcx);
    user_exec(&h, "BEGIN");
    user_exec(&h, "UPDATE t SET a = 1");
    wait_until(vcx, "the badge", |vcx| {
        label(vcx, "tx-badge-text").is_some()
    });
    let commit = handle_of(vcx, "button-tx-badge-commit");
    let rollback = handle_of(vcx, "button-tx-badge-rollback");
    assert_ne!(commit, rollback);
    let order = tab_order(vcx);
    assert_consecutive(&order, &[("Commit", &commit), ("Roll back", &rollback)]);
    assert_eq!(snapshot(&h).tx.status, TxStatus::InTx, "nothing clicked");
    in_window(vcx, |window, cx| window.focus(&commit, cx));
    vcx.simulate_keystrokes("enter");
    vcx.simulate_event(gpui::KeyUpEvent {
        keystroke: gpui::Keystroke::parse("enter").unwrap(),
    });
    wait_until(vcx, "the commit", |_| {
        snapshot(&h).tx.status == TxStatus::Idle
    });
}

// ## m4-t33d-fr1 (ruling R-T33d-5; T33d review I1, hostile test H1): Commit goes with the badge
// it commits; the focus lands on the console's editor (its home's primary), and Tab moves on
// from there. Before, the focus stayed on Commit's handle, which no frame drew: Tab did
// nothing and the console's keys were gone.
#[gpui::test]
fn committing_from_the_badge_by_keyboard_lands_the_focus_on_the_editor(cx: &mut TestAppContext) {
    let vcx = shell_staging(cx, "en");
    let (console, h) = console_over(vcx, pg().tracks_transactions());
    redraw(vcx);
    user_exec(&h, "BEGIN");
    user_exec(&h, "UPDATE t SET a = 1");
    wait_until(vcx, "the badge", |vcx| {
        label(vcx, "tx-badge-text").is_some()
    });
    let commit = handle_of(vcx, "button-tx-badge-commit");
    let rollback = handle_of(vcx, "button-tx-badge-rollback");
    let editor = vcx.cx.read(|cx| {
        Consoles::get(cx, console)
            .and_then(|s| s.editor.clone())
            .expect("the console has an editor")
            .read(cx)
            .focus_handle(cx)
    });
    in_window(vcx, |window, cx| window.focus(&commit, cx));
    vcx.run_until_parked();
    vcx.simulate_keystrokes("tab");
    vcx.run_until_parked();
    assert_focus_on(
        vcx,
        "Roll back (baseline: Tab works from Commit)",
        &rollback,
    );
    in_window(vcx, |window, cx| window.focus(&commit, cx));
    vcx.run_until_parked();
    press_key(vcx, "enter");
    wait_until(vcx, "the commit", |_| {
        snapshot(&h).tx.status == TxStatus::Idle
    });
    redraw(vcx);
    assert!(label(vcx, "tx-badge-text").is_none(), "the badge is gone");
    assert_focus_on(vcx, "the editor", &editor);
    // The console's keys are live again: the editor takes Tab (it indents there).
    assert!(
        context_stack(vcx).0.contains(&"Editor"),
        "the editor's key context is back: {:?}",
        context_stack(vcx)
    );
    vcx.simulate_keystrokes("tab");
    vcx.run_until_parked();
    assert_focus_on(vcx, "the editor (it took the Tab)", &editor);
    // And the window's Tab order goes on from there, to a drawn stop.
    let order = tab_order(vcx);
    in_window(vcx, |window, cx| {
        window.focus(&editor, cx);
        window.focus_next(cx);
    });
    vcx.run_until_parked();
    let after = focused(vcx).expect("focus_next moved the focus");
    assert_ne!(after, commit, "never back to the undrawn Commit");
    assert_ne!(after, editor, "on from the editor");
    assert!(order.contains(&after), "to a drawn Tab stop");
}

#[gpui::test]
fn badge_only_for_user_transactions(cx: &mut TestAppContext) {
    let vcx = shell_staging(cx, "en");
    let (console, h) = console_over(vcx, pg().tracks_transactions());
    redraw(vcx);
    assert!(label(vcx, "tx-badge-text").is_none());
    user_exec(&h, "BEGIN");
    user_exec(&h, "UPDATE t SET a = 1");
    wait_until(vcx, "the badge", |vcx| {
        text_of(vcx, "tx-badge-text").as_deref() == Some("Transaction · 1 statement · <1 min")
    });
    // Its age moves once a minute, on the executor clock.
    vcx.cx
        .executor()
        .advance_clock(Duration::from_secs(2 * 60 + 5));
    redraw(vcx);
    wait_until(vcx, "the age", |vcx| {
        text_of(vcx, "tx-badge-text").as_deref() == Some("Transaction · 1 statement · 2 min")
    });
    // Commit through the badge ends it.
    click(vcx, "button-tx-badge-commit");
    wait_until(vcx, "the commit", |_| {
        snapshot(&h).tx.status == TxStatus::Idle
    });
    wait_until(vcx, "the badge to go", |vcx| {
        label(vcx, "tx-badge-text").is_none()
    });
    let _ = console;

    // Hacivat's own transaction around a PostgreSQL cursor (owner Cursor) is not the user's.
    let id = profile_id(vcx, STAGING);
    let cursor = session_with(
        vcx,
        id,
        pg().tracks_transactions()
            .queries(vec![select("SELECT", 1_000_000)]),
        false,
    );
    let cursor_console = register_test_console(vcx, id, cursor.clone());
    focus_console(vcx, cursor_console);
    let run = RunId(7);
    let _ticket = cursor.run(RunSpec {
        run,
        units: vec![RunUnit::plain(
            UnitId { run, ordinal: 0 },
            "SELECT * FROM big".into(),
            Protocol::Extended,
            true,
        )]
        .into(),
        policy: ErrorPolicy::StopOnError,
        fetch: FetchPolicy::default(),
        observer: ObserverRef(std::sync::Arc::new(Stores::new())),
        confirm_deadline: RunSpec::CONFIRM_DEADLINE,
        context_generation: None,
    });
    poll_until("the cursor to open", || snapshot(&cursor).cursor.is_some());
    let tx = snapshot(&cursor).tx;
    assert_eq!((tx.status, tx.owner), (TxStatus::InTx, TxOwner::Cursor));
    wait_until(vcx, "the mirror of the cursor console", |vcx| {
        vcx.cx
            .read(|cx| tx_badge::view(cursor_console, cx).is_some())
    });
    redraw(vcx);
    assert!(
        vcx.cx
            .read(|cx| tx_badge::user_tx(cursor_console, cx))
            .is_none()
    );
    assert!(label(vcx, "tx-badge-text").is_none());

    // A session that ended shows no transaction (the session publishes it idle, T12).
    user_exec(&h, "BEGIN");
    wait_until(vcx, "the second transaction", |vcx| {
        vcx.cx.read(|cx| tx_badge::user_tx(console, cx).is_some())
    });
    let closing = h.close(ClosePolicy::RollBack);
    let _ = futures::executor::block_on(closing);
    wait_until(vcx, "the closed session's transaction to go", |vcx| {
        vcx.cx.read(|cx| tx_badge::user_tx(console, cx).is_none())
    });
    stop_core_runtime_test(cx);
}

#[gpui::test]
fn aborted_badge_offers_only_roll_back(cx: &mut TestAppContext) {
    let vcx = shell_staging(cx, "en");
    let fails = FakeExec {
        prefix: "UPDATE",
        result: Err(RawError {
            kind: RawErrorKind::Db,
            server: None,
        }),
        delay: Duration::ZERO,
        tx_after: Some(TxStatus::Failed),
        panics: false,
    };
    let (console, h) = console_over(vcx, pg().tracks_transactions().exec(vec![fails]));
    user_exec(&h, "BEGIN");
    user_exec(&h, "UPDATE t SET a = 1");
    wait_until(vcx, "the failed transaction", |vcx| {
        vcx.cx
            .read(|cx| tx_badge::user_tx(console, cx).is_some_and(|u| u.failed))
    });
    redraw(vcx);
    assert_eq!(
        text_of(vcx, "tx-badge-failed").as_deref(),
        Some("Transaction failed")
    );
    assert!(vcx.debug_bounds("button-tx-badge-commit").is_none());
    assert!(vcx.debug_bounds("button-tx-badge-rollback").is_some());
    // Commit is offered nowhere: not on the badge, not in the menu.
    assert!(!vcx.cx.update(|cx| tx_badge::press(console, true, cx)));
    let rows = menu_rows(vcx, console);
    assert_eq!(rows[0], ("Commit transaction".to_string(), true, None));
    assert!(!rows[1].1, "{rows:?}");
    click(vcx, "button-tx-badge-rollback");
    wait_until(vcx, "the rollback", |_| {
        snapshot(&h).tx.status == TxStatus::Idle
    });
    wait_until(vcx, "the badge to go", |vcx| {
        label(vcx, "tx-badge-failed").is_none()
    });
    stop_core_runtime_test(cx);
}

#[gpui::test]
fn badge_answers_wait_while_the_session_runs(cx: &mut TestAppContext) {
    // T11 I1: while a run of the user's holds the session (a question it waits on included),
    // Commit and Roll back are disabled: they would only queue behind it.
    let vcx = shell_staging(cx, "en");
    let slow = FakeExec {
        prefix: "UPDATE",
        result: Ok(ExecSummary::default()),
        delay: Duration::from_secs(600),
        tx_after: None,
        panics: false,
    };
    let (console, h) = console_over(vcx, pg().tracks_transactions().exec(vec![slow]));
    user_exec(&h, "BEGIN");
    let _running = h.submit(Request::Exec {
        sql: "UPDATE t SET a = 1".into(),
        opts: ExecOpts::default(),
        origin: ExecOrigin::User,
    });
    wait_until(vcx, "the session busy", |vcx| {
        vcx.cx.read(|cx| tx_badge::answer_waits(console, cx))
    });
    assert!(!vcx.cx.update(|cx| tx_badge::press(console, false, cx)));
    let rows = menu_rows(vcx, console);
    assert!(rows[0].1 && rows[1].1, "{rows:?}");
    // A surface that says a question is open holds them too, and lets go.
    let other = register_test_console(
        vcx,
        profile_id(vcx, STAGING),
        session_with(
            vcx,
            profile_id(vcx, STAGING),
            pg().tracks_transactions(),
            false,
        ),
    );
    vcx.cx
        .update(|cx| tx_badge::set_choice_pending(other, true, cx));
    assert!(vcx.cx.read(|cx| tx_badge::answer_waits(other, cx)));
    vcx.cx
        .update(|cx| tx_badge::set_choice_pending(other, false, cx));
    wait_until(vcx, "the mirror of the other console", |vcx| {
        vcx.cx.read(|cx| tx_badge::view(other, cx).is_some())
    });
    assert!(!vcx.cx.read(|cx| tx_badge::answer_waits(other, cx)));
    stop_core_runtime_test(cx);
}

// ---------------------------------------------------------------------------------------------
// The idle toast

#[gpui::test]
fn idle_toast_keep_never_rolls_back(cx: &mut TestAppContext) {
    let vcx = shell_staging(cx, "en");
    let id = profile_id(vcx, STAGING);
    let (h, fake) = session_and_fake(vcx, id, pg().tracks_transactions(), false);
    let console = register_test_console(vcx, id, h.clone());
    focus_console(vcx, console);
    user_exec(&h, "BEGIN");
    user_exec(&h, "UPDATE t SET a = 1");
    wait_until(vcx, "the user's transaction", |vcx| {
        vcx.cx.read(|cx| tx_badge::user_tx(console, cx).is_some())
    });
    vcx.cx.update(|cx| idle_toast::show(h.id(), 10, cx));
    vcx.run_until_parked();
    let texts = toast_texts(vcx);
    assert!(
        texts
            .iter()
            .any(|t| t.contains("Transaction open for 10 min on") && t.contains(STAGING)),
        "{texts:?}"
    );
    let buttons = button_labels(vcx);
    for b in ["Commit", "Roll back", "Keep open"] {
        assert!(buttons.iter().any(|l| l == b), "{b} in {buttons:?}");
    }
    // Keep open: the toast goes, nothing is sent, the transaction stays.
    let sent = fake_sent(&h);
    let log = fake.executed();
    let keep = toast_button(vcx, &h, 2);
    click(vcx, keep);
    vcx.run_until_parked();
    assert!(vcx.cx.read(idle_toast::shown).is_empty());
    assert_eq!(fake_sent(&h), sent);
    // Fix round 1 (m9): the connection heard nothing.
    assert_eq!(fake.executed(), log);
    assert_eq!(snapshot(&h).tx.status, TxStatus::InTx);
    assert!(vcx.cx.read(|cx| tx_badge::user_tx(console, cx)).is_some());
    // Its Roll back is the user's answer.
    vcx.cx.update(|cx| idle_toast::show(h.id(), 20, cx));
    vcx.run_until_parked();
    let roll_back = toast_button(vcx, &h, 1);
    click(vcx, roll_back);
    wait_until(vcx, "the rollback", |_| {
        snapshot(&h).tx.status == TxStatus::Idle
    });
    stop_core_runtime_test(cx);
}

/// The selector of button `i` of the idle toast of `h`'s session.
fn toast_button(vcx: &VisualTestContext, h: &SessionHandle, i: usize) -> &'static str {
    let id = vcx
        .cx
        .read(|cx| idle_toast::toast_id(h.id(), cx))
        .expect("the idle toast");
    Box::leak(format!("button-toast-action-{id}-{i}").into_boxed_str())
}

/// What the session sent so far, as its transaction status and statement count (a proxy that
/// a Keep open sent nothing: no statement, no status change).
fn fake_sent(h: &SessionHandle) -> (TxStatus, u32) {
    let tx = snapshot(h).tx;
    (tx.status, tx.statements)
}

#[gpui::test]
fn newer_idle_toast_replaces_the_older(cx: &mut TestAppContext) {
    let vcx = shell_staging(cx, "en");
    let (console, h) = console_over(vcx, pg().tracks_transactions());
    user_exec(&h, "BEGIN");
    wait_until(vcx, "the user's transaction", |vcx| {
        vcx.cx.read(|cx| tx_badge::user_tx(console, cx).is_some())
    });
    vcx.cx.update(|cx| idle_toast::show(h.id(), 10, cx));
    vcx.cx.update(|cx| idle_toast::show(h.id(), 20, cx));
    vcx.run_until_parked();
    assert_eq!(vcx.cx.read(idle_toast::shown), [(h.id(), 20)]);
    let texts = toast_texts(vcx);
    assert!(!texts.iter().any(|t| t.contains("10 min")), "{texts:?}");
    // The transaction ends: its toast goes with it.
    user_exec(&h, "ROLLBACK");
    wait_until(vcx, "the toast to go", |vcx| {
        vcx.cx.read(idle_toast::shown).is_empty()
    });
    // No user transaction: no reminder.
    vcx.cx.update(|cx| idle_toast::show(h.id(), 30, cx));
    vcx.run_until_parked();
    assert!(vcx.cx.read(idle_toast::shown).is_empty());
    stop_core_runtime_test(cx);
}

// ---------------------------------------------------------------------------------------------
// Single session

#[gpui::test]
fn single_session_checkbox_writes_through_the_model(cx: &mut TestAppContext) {
    let vcx = shell_staging(cx, "en");
    let id = profile_id(vcx, STAGING);
    in_window(vcx, |window, cx| {
        hacivat_app::connections::dialog::open_connection_dialog(
            hacivat_app::connections::dialog::DialogMode::Edit(id),
            window,
            cx,
        )
    });
    // Fix round 1 (I5): wait for the dialog, never a fixed time.
    wait_until_mut(vcx, "the dialog", |vcx| {
        vcx.debug_bounds("disclosure-conn-advanced").is_some()
    });
    click(vcx, "disclosure-conn-advanced");
    assert_eq!(
        text_of(vcx, "conn-adv-budget").as_deref(),
        Some("Up to 4 open at once; more consoles wait their turn")
    );
    click(vcx, "checkbox-conn-single-session");
    assert_eq!(
        text_of(vcx, "conn-adv-budget").as_deref(),
        Some("One session, shared by every console")
    );
    vcx.simulate_keystrokes(if cfg!(target_os = "macos") {
        "cmd-s"
    } else {
        "ctrl-s"
    });
    wait_until(vcx, "the save", |vcx| {
        vcx.cx.read(|cx| {
            cx.global::<ConnectionsModel>()
                .get(id)
                .is_some_and(|p| p.single_session)
        })
    });
    wait_until(vcx, "the file", |vcx| {
        std::fs::read_to_string(paths(vcx).connections_toml())
            .is_ok_and(|t| t.contains("single_session = true"))
    });
    // Fix round 1 (I1): the connection is connected, so the setting applies after it
    // reconnects; a toast says so, and a console holding a session of its own claims nothing.
    let texts = toast_texts(vcx);
    assert!(
        texts
            .iter()
            .any(|t| t.contains("applies after it reconnects")),
        "{texts:?}"
    );
    let (console, _h) = console_over(vcx, pg().tracks_transactions());
    redraw(vcx);
    assert!(text_of(vcx, "tx-shared-label").is_none());
    assert!(!vcx.cx.read(|cx| tx_badge::shares_session(console, cx)));
    stop_core_runtime_test(cx);
}

// ---------------------------------------------------------------------------------------------
// 960 px in Turkish

#[gpui::test]
fn badge_and_segment_are_never_clipped_at_960_tr_xt(cx: &mut TestAppContext) {
    let lang = if cfg!(feature = "pseudo") {
        "tr-XT"
    } else {
        "tr"
    };
    let vcx = shell_staging(cx, lang);
    vcx.simulate_resize(gpui::size(gpui::px(960.0), gpui::px(600.0)));
    let (console, h) = console_over(vcx, pg().tracks_transactions());
    user_exec(&h, "BEGIN");
    user_exec(&h, "UPDATE t SET a = 1");
    user_exec(&h, "UPDATE t SET b = 2");
    wait_until(vcx, "the badge", |vcx| {
        vcx.cx
            .read(|cx| tx_badge::user_tx(console, cx).is_some_and(|u| u.statements == 2))
    });
    for width in [960.0_f32, 1100.0, 1280.0] {
        vcx.simulate_resize(gpui::size(gpui::px(width), gpui::px(600.0)));
        redraw(vcx);
        for id in ["tx-badge-text", "tx-chip-mode", "status-tx-open"] {
            let l = label(vcx, id).unwrap_or_else(|| panic!("{width}: #{id} is drawn"));
            assert!(
                l.visible && !l.truncated && !l.clipped,
                "{width}: #{id} is cut: {l:?}"
            );
        }
    }
    // The segment is a button: it opens the transaction menu above itself.
    vcx.simulate_resize(gpui::size(gpui::px(960.0), gpui::px(600.0)));
    redraw(vcx);
    let segment = vcx.debug_bounds("status-tx").expect("the segment");
    click(vcx, "status-tx-button");
    assert_eq!(layer_ids(vcx), ["menu"]);
    let rows = menu_rows(vcx, console);
    assert_eq!(rows.len(), 6, "{rows:?}");
    // Fix round 1 (m9): the menu opens above the segment, left edges aligned.
    redraw(vcx);
    let menu = vcx.debug_bounds("popup-menu").expect("the menu");
    assert!(
        menu.bottom() <= segment.top(),
        "menu {menu:?} above segment {segment:?}"
    );
    assert!(
        (f32::from(menu.left()) - f32::from(segment.left())).abs() <= 2.0,
        "menu {menu:?} aligned with segment {segment:?}"
    );
    stop_core_runtime_test(cx);
}

// ---------------------------------------------------------------------------------------------
// Strings

#[test]
fn every_t22_id_exists_in_en_and_tr() {
    for locale in ["en", "tr"] {
        let ui = workspace_file(&format!("locales/{locale}/hacivat-ui.ftl"));
        let core = workspace_file(&format!("locales/{locale}/hacivat-core.ftl"));
        let ui_ids = message_ids_in(&ui);
        let core_ids = message_ids_in(&core);
        for id in [
            "hv-tx-chip-auto",
            "hv-tx-chip-manual",
            "hv-tx-chip-smart",
            "hv-tx-chip-tip",
            "hv-tx-menu-title",
            "hv-tx-menu-isolation",
            "hv-tx-iso-default",
            "hv-tx-menu-reason-open",
            "hv-tx-menu-prod-note",
            "hv-tx-chip-ro-reason",
            "hv-tx-badge",
            "hv-tx-badge-short",
            "hv-tx-badge-failed",
            "hv-tx-segment-tip",
            "hv-tx-idle-title",
            "hv-tx-idle-keep",
            "hv-tx-lost-maybe",
            "hv-conn-single-session",
            "hv-conn-budget-single",
            "hv-console-shared",
        ] {
            assert!(ui_ids.iter().any(|i| i == id), "{id} in {locale}");
        }
        for id in ["exec-dur-lt-min", "exec-startup-script-skipped-read-only"] {
            assert!(core_ids.iter().any(|i| i == id), "{id} in {locale}");
        }
    }
    assert_eq!(
        hacivat_exec::LogMsg::StartupScriptSkipped.fluent_id(),
        "exec-startup-script-skipped-read-only"
    );
}

// ---------------------------------------------------------------------------------------------
// The session carries (T12): one lost toast per close, worded "may have"; no Retry for a
// configuration loss; the settings' time zone reaches the manager.

fn emit(vcx: &VisualTestContext, event: hacivat_session::ConnectionEvent) {
    let m = vcx
        .cx
        .read(|cx| cx.global::<ConnectionsModel>().manager().clone());
    settle_until(vcx, Duration::from_secs(10), || m.event_subscribers() > 0);
    let before = vcx
        .cx
        .read(hacivat_app::connections::events::handled_events);
    m.publish_event(event);
    settle_until(vcx, Duration::from_secs(10), || {
        vcx.cx
            .read(hacivat_app::connections::events::handled_events)
            > before
    });
}

#[gpui::test]
fn one_close_reported_twice_shows_one_lost_toast_that_says_may_have(cx: &mut TestAppContext) {
    use hacivat_session::session::LostState;
    let vcx = shell_staging(cx, "en");
    let id = profile_id(vcx, STAGING);
    // The console's own loss, then the profile's list of the others' (one close, T12 r1-m1).
    emit(
        vcx,
        hacivat_session::ConnectionEvent::SessionLost {
            profile: id,
            lost: LostState {
                open_tx: true,
                temp_objects: true,
                ..LostState::default()
            },
        },
    );
    emit(
        vcx,
        hacivat_session::ConnectionEvent::SessionLost {
            profile: id,
            lost: LostState {
                open_tx: true,
                ..LostState::default()
            },
        },
    );
    vcx.run_until_parked();
    let lost: Vec<_> = vcx
        .cx
        .read(hacivat_app::connections::session_toasts::shown)
        .into_iter()
        .filter(|t| t.kind == "lost")
        .collect();
    assert_eq!(lost.len(), 1, "{lost:?}");
    let lines: Vec<String> = lost[0]
        .lines
        .iter()
        .map(|l| l.replace(['\u{2068}', '\u{2069}'], ""))
        .collect();
    assert!(
        lines.iter().any(|l| l == &t(vcx, ids::hv::TX_LOST_MAYBE)),
        "{lines:?}"
    );
    // What the first report lost is still said.
    assert!(
        lines.iter().any(|l| l.contains("temporary tables")),
        "{lines:?}"
    );
    stop_core_runtime_test(cx);
}

#[test]
fn a_configuration_loss_is_never_retried() {
    use hacivat_app::connections::capsule::is_config_loss;
    use hacivat_proto::ErrClass;
    assert!(is_config_loss(&LiveState::Lost {
        class: ErrClass::ConfigInvalid
    }));
    for &class in ErrClass::ALL.iter() {
        if class != ErrClass::ConfigInvalid {
            assert!(!is_config_loss(&LiveState::Lost { class }), "{class:?}");
        }
    }
    assert!(!is_config_loss(&LiveState::Disconnected));
}

#[gpui::test]
fn the_settings_time_zone_reaches_the_sessions(cx: &mut TestAppContext) {
    // Boot passes `ui.time_zone`, and a change of the setting follows (T12 concern 2): a
    // session opened afterwards sends it.
    let vcx = shell_with_fake(cx, "en", pg());
    add_profile(
        vcx,
        test_profile(STAGING, Some(EnvTag::Staging), "10.0.4.13"),
    );
    with_ws(vcx, |ws, window, cx| {
        ws.open_registered_tab("settings", window, cx)
    });
    vcx.cx.update(|cx| {
        hacivat_app::shell::update_settings(cx, |s| {
            s.ui.time_zone = hacivat_settings::TimeZonePref::Iana("Europe/Istanbul".into());
        });
    });
    vcx.run_until_parked();
    connect_and_wait(vcx, STAGING);
    wait_until(vcx, "the zone sent", |vcx| {
        fake(vcx)
            .executed()
            .iter()
            .any(|s| s == "SET TIME ZONE 'Europe/Istanbul'")
    });
    stop_core_runtime_test(cx);
}

// ## m4-t32-shots (fix round 1, review M-3): a well-formed zone the tz database does not know
// fails closed: sessions get the system zone (nothing is sent), never the unknown name.
#[gpui::test]
fn an_unknown_time_zone_is_never_sent_to_a_session(cx: &mut TestAppContext) {
    let vcx = shell_with_fake(cx, "en", pg());
    add_profile(
        vcx,
        test_profile(STAGING, Some(EnvTag::Staging), "10.0.4.13"),
    );
    vcx.cx.update(|cx| {
        hacivat_app::shell::update_settings(cx, |s| {
            s.ui.time_zone = hacivat_settings::TimeZonePref::Iana("Mars/Olympus".into());
        });
    });
    vcx.run_until_parked();
    connect_and_wait(vcx, STAGING);
    assert!(
        !fake(vcx)
            .executed()
            .iter()
            .any(|s| s.contains("TIME ZONE") || s.contains("Mars")),
        "{:?}",
        fake(vcx).executed()
    );
    stop_core_runtime_test(cx);
}

// ---------------------------------------------------------------------------------------------
// Fix round 1

/// I1: the "Shared session" chip follows what the consoles hold, never the saved setting
/// alone; a change of the setting while connected applies after a reconnect, and says so.
#[gpui::test]
fn the_shared_chip_follows_the_consoles_not_the_saved_setting(cx: &mut TestAppContext) {
    let vcx = shell_staging(cx, "en");
    let id = profile_id(vcx, STAGING);
    let (one, h1) = console_over(vcx, pg().tracks_transactions());
    let (two, _h2) = console_over(vcx, pg().tracks_transactions());
    let shared =
        |vcx: &VisualTestContext, c: ConsoleId| vcx.cx.read(|cx| tx_badge::shares_session(c, cx));
    assert!(!shared(vcx, one) && !shared(vcx, two));
    // Turned on while connected: the consoles keep their own sessions, so no chip claims
    // otherwise, and a toast says when it applies.
    let mut p = vcx
        .cx
        .read(|cx| (*cx.global::<ConnectionsModel>().get(id).expect("saved")).clone());
    p.single_session = true;
    save_draft(vcx, hacivat_session::profile::ProfileDraft::edit(p.clone()));
    wait_until(vcx, "the saved setting", |vcx| {
        vcx.cx.read(|cx| {
            cx.global::<ConnectionsModel>()
                .get(id)
                .is_some_and(|p| p.single_session)
        })
    });
    redraw(vcx);
    assert!(!shared(vcx, one) && !shared(vcx, two));
    let texts = toast_texts(vcx);
    assert!(
        texts
            .iter()
            .any(|t| t.contains("applies after it reconnects")),
        "{texts:?}"
    );
    // Turned off while two consoles hold one session: they still share it, and say so.
    p.single_session = false;
    save_draft(vcx, hacivat_session::profile::ProfileDraft::edit(p));
    let three = register_test_console(vcx, id, h1.clone());
    wait_until(vcx, "the third console's mirror", |vcx| {
        vcx.cx.read(|cx| tx_badge::view(three, cx).is_some())
    });
    assert!(shared(vcx, one) && shared(vcx, three));
    assert!(!shared(vcx, two));
    focus_console(vcx, three);
    redraw(vcx);
    assert_eq!(
        text_of(vcx, "tx-shared-label").as_deref(),
        Some("Shared session")
    );
    stop_core_runtime_test(cx);
}

/// I3: the idle toast's Commit waits as the badge's: while the session runs the user's
/// request, nothing is sent, the reminder stays and says why.
#[gpui::test]
fn idle_toast_answers_wait_while_the_session_runs(cx: &mut TestAppContext) {
    let vcx = shell_staging(cx, "en");
    let slow = FakeExec {
        prefix: "UPDATE",
        result: Ok(ExecSummary::default()),
        delay: Duration::from_secs(600),
        tx_after: None,
        panics: false,
    };
    let (console, h) = console_over(vcx, pg().tracks_transactions().exec(vec![slow]));
    user_exec(&h, "BEGIN");
    wait_until(vcx, "the user's transaction", |vcx| {
        vcx.cx.read(|cx| tx_badge::user_tx(console, cx).is_some())
    });
    vcx.cx.update(|cx| idle_toast::show(h.id(), 10, cx));
    vcx.run_until_parked();
    let _running = h.submit(Request::Exec {
        sql: "UPDATE t SET a = 1".into(),
        opts: ExecOpts::default(),
        origin: ExecOrigin::User,
    });
    wait_until(vcx, "the session busy", |vcx| {
        vcx.cx.read(|cx| tx_badge::answer_waits(console, cx))
    });
    let commit = toast_button(vcx, &h, 0);
    click(vcx, commit);
    vcx.run_until_parked();
    assert_eq!(snapshot(&h).tx.status, TxStatus::InTx);
    assert_eq!(vcx.cx.read(idle_toast::shown), [(h.id(), 10)]);
    let texts = toast_texts(vcx);
    assert!(
        texts.iter().any(|t| t.contains("waiting for an answer")),
        "{texts:?}"
    );
    // The palette's Commit and Roll back wait too.
    vcx.cx.update(|cx| Consoles::set_focused(cx, Some(console)));
    assert!(!run(vcx, "tx.commit"));
    assert!(!run(vcx, "tx.rollback"));
    stop_core_runtime_test(cx);
}

/// I3: the toast belongs to the transaction it was raised for.
#[gpui::test]
fn the_idle_toast_goes_with_its_transaction(cx: &mut TestAppContext) {
    let vcx = shell_staging(cx, "en");
    let (console, h) = console_over(vcx, pg().tracks_transactions());
    user_exec(&h, "BEGIN");
    wait_until(vcx, "the user's transaction", |vcx| {
        vcx.cx.read(|cx| tx_badge::user_tx(console, cx).is_some())
    });
    vcx.cx.update(|cx| idle_toast::show(h.id(), 10, cx));
    vcx.run_until_parked();
    assert_eq!(vcx.cx.read(idle_toast::shown).len(), 1);
    // Committed and begun again before the toast followed: another transaction.
    user_exec(&h, "COMMIT");
    user_exec(&h, "BEGIN");
    wait_until(vcx, "the toast to go", |vcx| {
        vcx.cx.read(idle_toast::shown).is_empty()
    });
    stop_core_runtime_test(cx);
}

fn failing(prefix: &'static str) -> FakeExec {
    FakeExec {
        prefix,
        result: Err(RawError {
            kind: RawErrorKind::Db,
            server: None,
        }),
        delay: Duration::ZERO,
        tx_after: None,
        panics: false,
    }
}

/// I4: a startup script that fails offers "Edit connection", never Retry.
#[gpui::test]
fn a_failed_startup_script_offers_edit_connection(cx: &mut TestAppContext) {
    let vcx = shell_with_fake(cx, "en", pg().exec(vec![failing("SET search_path")]));
    let mut p = test_profile(STAGING, Some(EnvTag::Staging), "10.0.4.13");
    p.startup_script = Some("SET search_path = app".into());
    add_profile(vcx, p);
    connect(vcx, STAGING);
    wait_until(vcx, "the loss", |vcx| {
        matches!(live(vcx, STAGING), LiveState::Lost { .. })
    });
    let id = profile_id(vcx, STAGING);
    assert!(vcx.cx.read(|cx| {
        let live = cx.global::<ConnectionsModel>().live(id);
        hacivat_app::connections::capsule::edits_instead(cx, id, &live)
    }));
    assert!(!vcx.cx.read(hacivat_app::connections::capsule::can_retry));
    match capsule_model(vcx) {
        hacivat_ui::capsule::CapsuleModel::Failed { retry, .. } => {
            assert_eq!(retry.as_ref(), "Edit connection");
        }
        other => panic!("{other:?}"),
    }
    stop_core_runtime_test(cx);
}

/// I4: a time zone the server refused keeps Retry (the profile's sessions no longer send it)
/// and the capsule says which zone.
#[gpui::test]
fn a_refused_time_zone_keeps_retry_and_says_so(cx: &mut TestAppContext) {
    let vcx = shell_with_fake(cx, "en", pg().exec(vec![failing("SET TIME ZONE")]));
    add_profile(
        vcx,
        test_profile(STAGING, Some(EnvTag::Staging), "10.0.4.13"),
    );
    // ## m4-t32-shots (fix round 1, review M-3; named exception): a zone the tz database
    // knows (an unknown one is never sent any more); the fake server refuses it.
    vcx.cx.update(|cx| {
        hacivat_app::shell::update_settings(cx, |s| {
            s.ui.time_zone = hacivat_settings::TimeZonePref::Iana("America/Halifax".into());
        });
    });
    vcx.run_until_parked();
    connect(vcx, STAGING);
    wait_until(vcx, "the loss", |vcx| {
        matches!(live(vcx, STAGING), LiveState::Lost { .. })
    });
    assert!(vcx.cx.read(hacivat_app::connections::capsule::can_retry));
    match capsule_model(vcx) {
        hacivat_ui::capsule::CapsuleModel::Failed { label, retry, .. } => {
            assert_eq!(
                label.replace(['\u{2068}', '\u{2069}'], ""),
                "The server refused time zone America/Halifax"
            );
            assert_eq!(retry.as_ref(), "Retry");
        }
        other => panic!("{other:?}"),
    }
    // Retry connects without the zone.
    vcx.cx.update(hacivat_app::connections::capsule::retry);
    wait_until(vcx, "the reconnect", |vcx| {
        matches!(live(vcx, STAGING), LiveState::Connected(_))
    });
    stop_core_runtime_test(cx);
}
