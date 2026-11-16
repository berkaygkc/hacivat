//! M3b Task 23: cancel escalation and the error-policy bar (HV-EXEC-08, -09 UI; R-M3-61;
//! `design/m3b-safe` surfaces 6 and 7), end to end over FakeDriver sessions.
//!
//! - A cancel the server ignores becomes slow at 10 s: the toast offers only Show options….
//! - A second Stop while cancelling opens the 420 alertdialog with Keep waiting focused and the
//!   connection's subtitle; Escape answers Keep waiting and never sends Terminate or
//!   Deactivate (R-M3-38).
//! - A terminate's outcome is shown whenever it comes: a refusal with the server's text
//!   verbatim, a success with its Output line, also past the session's 2 s answer.
//! - The dialog's consequences are read live (R-M3-73): from the default console Disconnect
//!   names the other consoles' transactions and closes the profile; from another console only
//!   its own session.
//! - The error bar's answer reaches the session's run gate, once; a failed PostgreSQL
//!   transaction offers Roll back and continue (no Retry); F6 enters and leaves the bar; closing
//!   the console answers Stop.
//!
//! Every wait is on a condition. The session's 10 s slow-cancel mark is its real clock: the
//! app's test graph has no paused Tokio clock (`test-util` is a dev feature of the core crates
//! only), so that one test waits for it with a longer bound; the session crate pins the 10 s
//! with a paused clock (`cancel.rs`).

mod support;

use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::{Entity, Focusable as _, TestAppContext, VisualTestContext};
use hacivat_app::connections::ConnectionsModel;
use hacivat_app::connections::prompt_kit::parts;
use hacivat_app::console::tx_badge;
use hacivat_app::console::{ConsoleId, Consoles};
use hacivat_app::exec_ui::context::SqlContext;
use hacivat_app::exec_ui::escalation::{OtherTx, OwnTx, Reach};
use hacivat_app::exec_ui::{error_bar, escalation};
use hacivat_driver::fake::{
    FakeDriver, FakeExec, FakeQuery, FakeResultSet, FakeRows, FakeTypedCall,
};
use hacivat_driver::{ExecOpts, ExecSummary, RawError, RawErrorKind, ServerMessage, TxStatus};
use hacivat_exec::RunChoice;
use hacivat_proto::{DbmsKind, Department, EnvTag, ProfileId};
use hacivat_session::LiveState;
use hacivat_session::pipeline::Interaction;
use hacivat_session::session::{
    ClientId, ExecOrigin, Request, SessionHandle, SessionKind, SessionSpec, SessionState, TxOwner,
};
use hacivat_ui_editor::{EditorView, ExecMarkState};
use hacivat_values::{LogicalType, Value};
use support::*;

/// A console over a FakeDriver session whose side connections (terminate) go to `side`.
struct Console {
    vcx: &'static mut VisualTestContext,
    id: ConsoleId,
    fake: Arc<FakeDriver>,
    side: Arc<FakeDriver>,
    session: SessionHandle,
    editor: Entity<EditorView>,
}

fn profile(name: &str) -> hacivat_session::profile::Profile {
    hacivat_session::profile::Profile {
        engine: DbmsKind::Postgres,
        ..test_profile(name, Some(EnvTag::Dev), "127.0.0.1")
    }
}

/// The SQL-context probe's answers on a read-write session.
fn probes() -> Vec<FakeExec> {
    (0..16)
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
        .collect()
}

/// A PostgreSQL fake (backend 7) that follows transactions and answers the probe.
fn pg() -> FakeDriver {
    FakeDriver::new(DbmsKind::Postgres)
        .backend(7)
        .tracks_transactions()
        .exec(probes())
}

/// A console registered over its own session (`fake_session_with_side`).
fn console(cx: &mut TestAppContext, fake: FakeDriver, side: FakeDriver) -> Console {
    let vcx = shell_with_connections(cx, "en");
    in_window(vcx, |window, _| window.activate_window());
    let p = profile("orders-dev");
    let profile = add_profile(vcx, p.clone());
    let rt = vcx
        .cx
        .read(|cx| cx.global::<ConnectionsModel>().core_runtime().handle());
    let (session, fake, side) = futures::executor::block_on(
        hacivat_session::testing::fake_session_with_side(rt, fake, side, p),
    );
    let id = registered(vcx, profile, &session);
    let editor = editor_of(vcx, id);
    Console {
        vcx,
        id,
        fake,
        side,
        session,
        editor,
    }
}

/// Registers a test console of `profile` over `session` and waits for its SQL context.
fn registered(vcx: &VisualTestContext, profile: ProfileId, session: &SessionHandle) -> ConsoleId {
    let id = register_test_console(vcx, profile, session.clone());
    wait_until(vcx, "the console's SQL context", |vcx| {
        vcx.cx
            .read(|cx| hacivat_app::exec_ui::sql_context(id, cx) != SqlContext::Pending)
    });
    id
}

fn editor_of(vcx: &VisualTestContext, id: ConsoleId) -> Entity<EditorView> {
    vcx.cx.read(|cx| {
        Consoles::get(cx, id)
            .and_then(|s| s.editor.clone())
            .expect("the console has an editor")
    })
}

/// A statement the server never answers and never lets a cancel end.
fn deaf(prefix: &'static str) -> FakeQuery {
    FakeQuery {
        prefix,
        first_delay: Duration::from_secs(600),
        ignores_cancel: true,
        ..Default::default()
    }
}

/// The server's refusal of a terminate.
fn refusal() -> RawError {
    RawError {
        kind: RawErrorKind::Db,
        server: Some(ServerMessage {
            sqlstate: Some("42501".into()),
            vendor_code: None,
            severity: Some("ERROR".into()),
            text: "must be a superuser to terminate superuser process".into(),
            detail: None,
            hint: None,
            lang_hint: None,
            position: None,
        }),
    }
}

/// A side connection whose terminate answers `result` after `delay`.
fn side_terminate(result: Result<ExecSummary, RawError>, delay: Duration) -> FakeDriver {
    FakeDriver::new(DbmsKind::Postgres).exec(vec![FakeExec {
        prefix: "SELECT pg_terminate_backend",
        result,
        delay,
        tx_after: None,
        panics: false,
    }])
}

fn one_row() -> FakeResultSet {
    FakeResultSet {
        columns: vec![(
            "n",
            LogicalType::Int {
                bits: 64,
                signed: true,
            },
        )],
        rows: FakeRows::Literal(vec![vec![Value::Int(1)]]),
    }
}

fn answering(prefix: &'static str) -> FakeQuery {
    FakeQuery {
        prefix,
        sets: vec![one_row()],
        ..Default::default()
    }
}

fn failing(prefix: &'static str) -> FakeQuery {
    FakeQuery {
        prefix,
        sets: vec![one_row()],
        fail_after: Some((
            0,
            RawError {
                kind: RawErrorKind::Db,
                server: Some(ServerMessage {
                    sqlstate: Some("42P01".into()),
                    vendor_code: None,
                    severity: Some("ERROR".into()),
                    text: "relation \"missing\" does not exist".into(),
                    detail: None,
                    hint: None,
                    lang_hint: None,
                    position: None,
                }),
            },
        )),
        ..Default::default()
    }
}

const THREE: [&str; 3] = ["SELECT 1", "SELECT * FROM missing", "SELECT 3"];

/// Runs `text` in console `id`'s editor (caret at its start).
fn run_text(vcx: &VisualTestContext, editor: &Entity<EditorView>, text: &str) {
    let text = text.to_string();
    vcx.cx.update(|cx| {
        editor.update(cx, |e, cx| {
            e.set_text(&text, cx);
            e.select_range(0..0, cx);
        });
    });
    vcx.run_until_parked();
    let editor = editor.clone();
    in_window(vcx, |window, cx| {
        editor.update(cx, |e, cx| assert!(e.run_command("editor.run", window, cx)));
    });
    vcx.run_until_parked();
}

fn marks_of(vcx: &VisualTestContext, id: ConsoleId) -> Vec<ExecMarkState> {
    vcx.cx.read(|cx| {
        hacivat_app::exec_ui::last_run_marks(id, cx)
            .into_iter()
            .flatten()
            .collect()
    })
}

/// Focuses console `id`'s editor (the console `exec.cancel` and F6 act on).
fn focus_editor(vcx: &VisualTestContext, editor: &Entity<EditorView>) {
    let editor = editor.clone();
    in_window(vcx, |window, cx| {
        let handle = editor.read(cx).focus_handle(cx);
        window.focus(&handle, cx);
    });
    vcx.run_until_parked();
}

/// Runs a statement the server never answers on console `id`, and cancels it once.
fn cancel_deaf(
    vcx: &VisualTestContext,
    id: ConsoleId,
    editor: &Entity<EditorView>,
    session: &SessionHandle,
) {
    run_text(vcx, editor, "SELECT pg_sleep(600)");
    wait_until(vcx, "the statement to run", |vcx| {
        matches!(
            marks_of(vcx, id).as_slice(),
            [ExecMarkState::Running { .. }]
        )
    });
    focus_editor(vcx, editor);
    assert!(run(vcx, "exec.cancel"));
    wait_until(vcx, "the session to cancel", |_| {
        matches!(
            session.state().borrow().state,
            SessionState::Cancelling { .. }
        )
    });
}

fn log_ids_of(vcx: &VisualTestContext, id: ConsoleId) -> Vec<&'static str> {
    vcx.cx.read(|cx| {
        Consoles::get(cx, id)
            .map(|s| s.log.lines().iter().map(|l| l.msg.fluent_id()).collect())
            .unwrap_or_default()
    })
}

fn dialog_facts(vcx: &VisualTestContext, id: ConsoleId) -> escalation::Facts {
    vcx.cx
        .read(|cx| escalation::dialog_state(id, cx))
        .expect("the dialog is open")
        .0
}

impl Console {
    fn run_text(&self, text: &str) {
        run_text(self.vcx, &self.editor, text);
    }

    fn marks(&self) -> Vec<ExecMarkState> {
        marks_of(self.vcx, self.id)
    }

    /// Runs a statement the server never answers, and cancels it once.
    fn cancelling(&self) {
        cancel_deaf(self.vcx, self.id, &self.editor, &self.session);
    }

    /// The second press: the dialog opens.
    fn open_dialog(&self) {
        assert!(run(self.vcx, "exec.cancel"));
        self.vcx.run_until_parked();
        assert!(self.dialog_open());
    }

    fn dialog_open(&self) -> bool {
        self.vcx.cx.read(|cx| escalation::dialog_open(self.id, cx))
    }

    fn focused_part(&self) -> Option<String> {
        in_window(self.vcx, |window, cx| parts::focused(window, cx))
    }

    /// Presses Terminate on server from the keyboard (Keep waiting → Disconnect → Terminate).
    fn press_terminate(&mut self) {
        self.vcx.simulate_keystrokes("tab tab");
        assert_eq!(self.focused_part().as_deref(), Some("escalation-terminate"));
        self.vcx.simulate_keystrokes("enter");
        self.vcx.run_until_parked();
    }

    fn refused(&self) -> Option<Option<String>> {
        self.vcx
            .cx
            .read(|cx| escalation::dialog_state(self.id, cx))
            .and_then(|(_, server, refused)| refused.then_some(server))
    }

    fn typed(&self) -> Vec<String> {
        self.fake
            .typed_log()
            .into_iter()
            .filter_map(|c| match c {
                FakeTypedCall::Query(sql) => Some(sql),
                _ => None,
            })
            .collect()
    }

    fn log_ids(&self) -> Vec<&'static str> {
        log_ids_of(self.vcx, self.id)
    }

    /// Runs `units` under the Ask policy (as Task 24's orchestrator will) and waits for the
    /// bar's question.
    fn ask(&self, units: &[&str]) -> (u64, u64, Vec<RunChoice>) {
        let units: Vec<String> = units.iter().map(|s| s.to_string()).collect();
        self.vcx.cx.update(|cx| {
            let refs: Vec<&str> = units.iter().map(String::as_str).collect();
            error_bar::run_asking(self.id, &refs, cx).expect("the run starts");
        });
        let mut question = None;
        wait_until(self.vcx, "the bar's question", |vcx| {
            question = vcx.cx.read(|cx| error_bar::open_question(self.id, cx));
            question.is_some()
        });
        question.expect("a question")
    }

    fn answer(&self, question: u64, choice: RunChoice) -> bool {
        self.vcx
            .cx
            .update(|cx| error_bar::answer(self.id, question, choice, cx))
    }

    /// Ends the deaf statement, so the session and the runtime stop cleanly.
    fn end(self, cx: &mut TestAppContext) {
        self.fake.kill_connections();
        stop_core_runtime_test(cx);
    }
}

/// [`wait_until`] with a bound past the session's 10 s slow-cancel mark.
#[track_caller]
fn wait_long(
    vcx: &VisualTestContext,
    what: &str,
    mut done: impl FnMut(&VisualTestContext) -> bool,
) {
    vcx.executor().allow_parking();
    let deadline = Instant::now() + Duration::from_secs(40);
    loop {
        vcx.run_until_parked();
        if done(vcx) {
            return;
        }
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

// ---- cancel escalation (surface 6) ------------------------------------------------------

#[gpui::test]
fn slow_cancel_toast_after_ten_seconds(cx: &mut TestAppContext) {
    let c = console(
        cx,
        pg().queries(vec![deaf("SELECT pg_sleep(600)")]),
        FakeDriver::new(DbmsKind::Postgres),
    );
    enable_registry(c.vcx);
    c.run_text("SELECT pg_sleep(600)");
    wait_until(c.vcx, "the statement to run", |_| {
        matches!(c.marks().as_slice(), [ExecMarkState::Running { .. }])
    });
    let asked = Instant::now();
    assert!(run(c.vcx, "exec.cancel"));
    // The session's first answer (2 s): still running; no toast yet.
    wait_until(c.vcx, "the cancel to be still running", |_| {
        matches!(
            c.session.state().borrow().state,
            SessionState::Cancelling { .. }
        )
    });
    assert!(c.vcx.cx.read(escalation::slow_toasts).is_empty());
    wait_long(c.vcx, "the slow-cancel toast", |vcx| {
        !vcx.cx.read(escalation::slow_toasts).is_empty()
    });
    assert!(
        asked.elapsed() >= Duration::from_secs(10),
        "the toast came after {:?}",
        asked.elapsed()
    );
    let toasts = c.vcx.cx.read(escalation::slow_toasts);
    assert_eq!(toasts.len(), 1, "one toast");
    assert_eq!(toasts[0].0, c.id);
    in_window(c.vcx, |window, _| window.refresh());
    c.vcx.run_until_parked();
    let texts = toast_texts(c.vcx);
    assert!(
        texts.iter().any(|t| t == "Cancelling is taking long"),
        "{texts:?}"
    );
    assert!(
        texts
            .iter()
            .any(|t| t.starts_with("The server hasn't stopped the statement on orders-dev after")),
        "{texts:?}"
    );
    // One button, and it only opens the dialog: nothing escalates from the toast.
    let labels = button_labels(c.vcx);
    assert!(labels.contains(&"Show options…".to_string()), "{labels:?}");
    assert!(
        !labels.contains(&"Terminate on server".to_string()),
        "{labels:?}"
    );
    assert!(!labels.contains(&"Disconnect".to_string()), "{labels:?}");
    assert!(c.side.executed().is_empty(), "{:?}", c.side.executed());
    // The unit in flight shows the hourglass ("cancel not answered").
    assert!(
        matches!(c.marks().as_slice(), [ExecMarkState::Escalated { .. }]),
        "{:?}",
        c.marks()
    );
    c.end(cx);
}

#[gpui::test]
fn second_press_opens_the_dialog_with_keep_waiting_focused(cx: &mut TestAppContext) {
    let c = console(
        cx,
        pg().queries(vec![deaf("SELECT pg_sleep(600)")]),
        FakeDriver::new(DbmsKind::Postgres),
    );
    enable_registry(c.vcx);
    c.cancelling();
    assert!(!c.dialog_open());
    // The second press: Stop stays enabled while cancelling, and opens the dialog.
    c.open_dialog();
    assert!(layer_ids(c.vcx).contains(&escalation::ESCALATION_LAYER_ID));
    assert_eq!(c.focused_part().as_deref(), Some("escalation-keep"));
    let (facts, server, refused) = c
        .vcx
        .cx
        .read(|cx| escalation::dialog_state(c.id, cx))
        .expect("the dialog's state");
    assert_eq!(facts.terminate, Some("pg_terminate_backend"));
    assert_eq!(server, None);
    assert!(!refused);
    // Fix round 1 (I2): the subtitle names the environment and the connection.
    assert_eq!(facts.env, Some(EnvTag::Dev));
    in_window(c.vcx, |window, _| window.refresh());
    c.vcx.run_until_parked();
    let drawn = registry_frame(c.vcx);
    assert!(
        drawn
            .iter()
            .any(|r| r.text.as_ref() == "orders-dev" && r.path.ends_with("#confirm-connection")),
        "the subtitle's connection name"
    );
    // The buttons, in visual order: Disconnect (far left), Terminate on server, Keep waiting.
    let labels = button_labels(c.vcx);
    let at = |l: &str| labels.iter().position(|x| x == l).expect(l);
    assert!(at("Disconnect") < at("Terminate on server"));
    assert!(at("Terminate on server") < at("Keep waiting"));
    // Nothing was sent past the cancel.
    assert!(c.side.executed().is_empty(), "{:?}", c.side.executed());
    c.end(cx);
}

#[gpui::test]
fn escape_never_escalates(cx: &mut TestAppContext) {
    let c = console(
        cx,
        pg().queries(vec![deaf("SELECT pg_sleep(600)")]),
        FakeDriver::new(DbmsKind::Postgres),
    );
    c.cancelling();
    c.open_dialog();
    // Escape answers Keep waiting: the dialog closes, nothing is terminated or dropped.
    c.vcx.simulate_keystrokes("escape");
    c.vcx.run_until_parked();
    assert!(!c.dialog_open());
    assert!(c.side.executed().is_empty(), "{:?}", c.side.executed());
    assert!(
        matches!(
            c.session.state().borrow().state,
            SessionState::Cancelling { .. }
        ),
        "the statement is still being cancelled, the session not broken"
    );
    // Keep waiting leaves the hourglass in the gutter.
    assert!(
        matches!(c.marks().as_slice(), [ExecMarkState::Escalated { .. }]),
        "{:?}",
        c.marks()
    );
    // Opened again: Escape from any button never escalates either.
    c.open_dialog();
    c.vcx.simulate_keystrokes("tab");
    c.vcx.simulate_keystrokes("escape");
    c.vcx.run_until_parked();
    assert!(!c.dialog_open());
    assert!(c.side.executed().is_empty(), "{:?}", c.side.executed());
    c.end(cx);
}

#[gpui::test]
fn failed_terminate_shows_the_server_text_verbatim(cx: &mut TestAppContext) {
    let mut c = console(
        cx,
        pg().queries(vec![deaf("SELECT pg_sleep(600)")]),
        side_terminate(Err(refusal()), Duration::ZERO),
    );
    enable_registry(c.vcx);
    c.cancelling();
    c.open_dialog();
    c.press_terminate();
    wait_until(c.vcx, "the refusal", |_| c.refused().is_some());
    assert_eq!(c.side.executed(), ["SELECT pg_terminate_backend(7)"]);
    wait_until(c.vcx, "the server's text", |_| {
        c.refused().flatten().as_deref()
            == Some("must be a superuser to terminate superuser process")
    });
    wait_until(c.vcx, "the focus back on Keep waiting", |_| {
        c.focused_part().as_deref() == Some("escalation-keep")
    });
    in_window(c.vcx, |window, _| window.refresh());
    c.vcx.run_until_parked();
    let markers = rendered_markers(c.vcx);
    assert!(markers.contains(&"escalation-terminate-failed".to_string()));
    assert!(markers.contains(&"escalation-server-text".to_string()));
    let drawn = registry_frame(c.vcx);
    assert!(
        drawn.iter().any(|r| r.text.as_ref()
            == "must be a superuser to terminate superuser process"
            && r.path.ends_with("#escalation-server-text")),
        "the server's text is drawn whole, in its own field"
    );
    assert!(
        drawn
            .iter()
            .any(|r| r.text.as_ref() == "42501" && r.path.ends_with("#escalation-server-code"))
    );
    // One line for the one refusal (its reply and its event both reach the follower).
    assert_eq!(
        c.log_ids()
            .iter()
            .filter(|id| **id == "exec-terminate-failed")
            .count(),
        1,
        "{:?}",
        c.log_ids()
    );
    // The statement is still being cancelled.
    assert!(matches!(
        c.session.state().borrow().state,
        SessionState::Cancelling { .. }
    ));
    c.end(cx);
}

// ## m3b-t23-escalation (fix round 1, I1): a side connection slower than the session's 2 s
// answer. The outcome still reaches the dialog and Output.

#[gpui::test]
fn a_late_refusal_still_reaches_the_dialog(cx: &mut TestAppContext) {
    let mut c = console(
        cx,
        pg().queries(vec![deaf("SELECT pg_sleep(600)")]),
        side_terminate(Err(refusal()), Duration::from_secs(3)),
    );
    c.cancelling();
    c.open_dialog();
    let pressed = Instant::now();
    c.press_terminate();
    wait_until(c.vcx, "the late refusal", |_| {
        c.refused().flatten().as_deref()
            == Some("must be a superuser to terminate superuser process")
    });
    // Past the 2 s answer (`StillRunning`), which changed nothing.
    assert!(
        pressed.elapsed() >= Duration::from_secs(2),
        "{:?}",
        pressed.elapsed()
    );
    assert_eq!(
        c.log_ids()
            .iter()
            .filter(|id| **id == "exec-terminate-failed")
            .count(),
        1
    );
    wait_until(c.vcx, "the focus back on Keep waiting", |_| {
        c.focused_part().as_deref() == Some("escalation-keep")
    });
    c.end(cx);
}

#[gpui::test]
fn a_late_terminate_success_writes_its_line(cx: &mut TestAppContext) {
    let mut c = console(
        cx,
        // The statement's answer is 6 s away and no cancel ends it; a dead connection does, at
        // that answer.
        pg().queries(vec![FakeQuery {
            first_delay: Duration::from_secs(6),
            ..deaf("SELECT pg_sleep(600)")
        }]),
        side_terminate(Ok(ExecSummary::default()), Duration::from_secs(3)),
    );
    c.cancelling();
    c.open_dialog();
    let pressed = Instant::now();
    c.press_terminate();
    // The session's 2 s answer comes while the terminate is still on its way: nothing is said
    // yet, and the dialog stays.
    wait_until(c.vcx, "the session's 2 s answer to pass", |_| {
        pressed.elapsed() >= Duration::from_millis(2_500)
    });
    assert!(c.dialog_open());
    assert!(c.refused().is_none());
    assert!(!c.log_ids().contains(&"exec-terminated"));
    // What the terminate does to the server connection: the request ends with it (here at
    // the statement's next answer).
    c.fake.kill_connections();
    wait_until(c.vcx, "the request's end", |_| {
        !matches!(
            c.session.state().borrow().state,
            SessionState::Cancelling { .. }
        )
    });
    wait_until(c.vcx, "the terminated line", |_| {
        c.log_ids().contains(&"exec-terminated-no-tx")
    });
    assert!(!c.dialog_open(), "the request's end closes the dialog");
    assert!(!c.log_ids().contains(&"exec-terminate-failed"));
    stop_core_runtime_test(cx);
}

// ## m3b-t23-escalation (fix round 1, I5): the live consequences and Disconnect (R-M3-73).

/// A connected profile `orders-dev` (manager sessions over one fake) whose default session
/// runs in a test console; the settings tab stays in front, so Console 1 is not bound.
fn connected(
    cx: &mut TestAppContext,
    fake: FakeDriver,
) -> (&'static mut VisualTestContext, ProfileId) {
    let vcx = shell_with_fake(cx, "en", fake);
    in_window(vcx, |window, _| window.activate_window());
    let id = add_profile(vcx, profile("orders-dev"));
    with_ws(vcx, |ws, window, cx| {
        ws.open_registered_tab("settings", window, cx)
    });
    connect(vcx, "orders-dev");
    wait_until(vcx, "the connect", |vcx| {
        !matches!(live(vcx, "orders-dev"), LiveState::Connecting(_))
    });
    assert!(matches!(live(vcx, "orders-dev"), LiveState::Connected(_)));
    (vcx, id)
}

fn manager_sessions(
    vcx: &VisualTestContext,
    id: ProfileId,
) -> Arc<hacivat_session::session::SessionManager> {
    vcx.cx
        .read(|cx| cx.global::<ConnectionsModel>().manager().sessions(id))
        .expect("the profile is connected")
}

/// Opens console client `n`'s own session of profile `id`.
fn own_session(vcx: &VisualTestContext, id: ProfileId, n: u64) -> SessionHandle {
    let sessions = manager_sessions(vcx, id);
    let rt = vcx
        .cx
        .read(|cx| cx.global::<ConnectionsModel>().core_runtime().handle());
    rt.block_on(sessions.open(
        SessionSpec {
            kind: SessionKind::Console(ClientId(n)),
            read_only: false,
            department: Department::Console,
        },
        Interaction::Headless,
    ))
    .expect("the session opens")
}

/// Sends a user statement and waits for its reply.
fn user_exec(h: &SessionHandle, sql: &str) {
    let ticket = h.submit(Request::Exec {
        sql: sql.to_string(),
        opts: ExecOpts::default(),
        origin: ExecOrigin::User,
    });
    let _ = futures::executor::block_on(ticket.reply).expect("the actor replies");
}

/// The shared fake of a connected profile: the probe, a deaf typed statement per console,
/// and one slow user statement.
fn manager_fake() -> FakeDriver {
    let mut exec = probes();
    exec.push(FakeExec {
        prefix: "SELECT pg_sleep(601)",
        result: Ok(ExecSummary::default()),
        delay: Duration::from_secs(600),
        tx_after: None,
        panics: false,
    });
    FakeDriver::new(DbmsKind::Postgres)
        .backend(7)
        .tracks_transactions()
        .exec(exec)
        .queries(vec![
            deaf("SELECT pg_sleep(600)"),
            deaf("SELECT pg_sleep(600)"),
        ])
}

#[gpui::test]
fn disconnect_from_the_default_console_names_the_others_and_closes_the_profile(
    cx: &mut TestAppContext,
) {
    let (vcx, id) = connected(cx, manager_fake());
    let default = manager_sessions(vcx, id)
        .default_session()
        .expect("the default session");
    // Console (2) holds a user transaction of 1 statement; console (3) runs a user request.
    let two_session = own_session(vcx, id, 2);
    let _two = registered(vcx, id, &two_session);
    user_exec(&two_session, "BEGIN");
    user_exec(&two_session, "UPDATE t SET a = 1");
    let three_session = own_session(vcx, id, 3);
    let _three = registered(vcx, id, &three_session);
    let _running = three_session.submit(Request::Exec {
        sql: "SELECT pg_sleep(601)".to_string(),
        opts: ExecOpts::default(),
        origin: ExecOrigin::User,
    });
    wait_until(vcx, "console (3)'s request to run", |_| {
        matches!(
            three_session.state().borrow().state,
            SessionState::Busy { .. }
        )
    });
    // The default console last: its tab is the active one.
    let one = registered(vcx, id, &default);
    let editor = editor_of(vcx, one);
    cancel_deaf(vcx, one, &editor, &default);
    assert!(run(vcx, "exec.cancel"));
    vcx.run_until_parked();
    let facts = dialog_facts(vcx, one);
    assert_eq!(facts.reach, Reach::Profile);
    assert_eq!(facts.own, OwnTx::None);
    let mut others: Vec<OtherTx> = facts.others.iter().map(|(_, tx)| tx.clone()).collect();
    others.sort_by_key(|t| format!("{t:?}"));
    assert_eq!(others, [OtherTx::Maybe, OtherTx::Open(1)]);
    // Disconnect (Tab from Keep waiting reaches it first), Enter.
    vcx.simulate_keystrokes("tab");
    assert_eq!(
        in_window(vcx, |window, cx| parts::focused(window, cx)).as_deref(),
        Some("escalation-disconnect")
    );
    vcx.simulate_keystrokes("enter");
    wait_until(vcx, "the profile to disconnect", |vcx| {
        live(vcx, "orders-dev") == LiveState::Disconnected
    });
    wait_until(vcx, "the disconnected line", |vcx| {
        log_ids_of(vcx, one).contains(&"exec-disconnected")
    });
    fake(vcx).kill_connections();
    stop_core_runtime_test(cx);
}

#[gpui::test]
fn disconnect_from_another_console_closes_only_its_session(cx: &mut TestAppContext) {
    let (vcx, id) = connected(cx, manager_fake());
    let default = manager_sessions(vcx, id)
        .default_session()
        .expect("the default session");
    let _one = registered(vcx, id, &default);
    let two_session = own_session(vcx, id, 2);
    let two = registered(vcx, id, &two_session);
    let editor = editor_of(vcx, two);
    cancel_deaf(vcx, two, &editor, &two_session);
    assert!(run(vcx, "exec.cancel"));
    vcx.run_until_parked();
    let facts = dialog_facts(vcx, two);
    assert_eq!(facts.reach, Reach::Console);
    assert!(facts.others.is_empty(), "{:?}", facts.others);
    vcx.simulate_keystrokes("tab");
    vcx.simulate_keystrokes("enter");
    wait_until(vcx, "console (2)'s session to break", |_| {
        matches!(two_session.state().borrow().state, SessionState::Broken(_))
    });
    wait_until(vcx, "the disconnected line", |vcx| {
        log_ids_of(vcx, two).contains(&"exec-disconnected")
    });
    // The profile and its default session stay.
    assert!(matches!(live(vcx, "orders-dev"), LiveState::Connected(_)));
    assert!(!matches!(
        default.state().borrow().state,
        SessionState::Broken(_) | SessionState::Disconnected
    ));
    fake(vcx).kill_connections();
    stop_core_runtime_test(cx);
}

// ---- the error-policy bar (surface 7) -----------------------------------------------------

#[gpui::test]
fn error_bar_choice_reaches_the_session(cx: &mut TestAppContext) {
    let c = console(
        cx,
        pg().queries(vec![
            answering("SELECT 1"),
            failing("SELECT * FROM missing"),
            answering("SELECT 3"),
        ]),
        FakeDriver::new(DbmsKind::Postgres),
    );
    let (_, question, offered) = c.ask(&THREE);
    assert_eq!(
        offered,
        [
            RunChoice::Retry,
            RunChoice::Ignore,
            RunChoice::IgnoreAll,
            RunChoice::Stop
        ]
    );
    // The run waits: the third statement is not sent; the badge's answers wait too.
    assert_eq!(c.typed(), &THREE[..2]);
    assert!(c.vcx.cx.read(|cx| tx_badge::answer_waits(c.id, cx)));
    // A choice the bar does not offer, or another question, is refused; Ignore goes to the
    // session, once.
    assert!(!c.answer(question, RunChoice::RollbackAndContinue));
    assert!(
        !c.answer(question + 1, RunChoice::Ignore),
        "another question"
    );
    assert!(c.answer(question, RunChoice::Ignore));
    assert!(
        !c.answer(question, RunChoice::Stop),
        "a second answer to the same question"
    );
    wait_until(c.vcx, "the rest of the run", |_| c.typed() == THREE);
    wait_until(c.vcx, "the run's end", |vcx| {
        vcx.cx
            .read(|cx| hacivat_app::exec_ui::last_run_ended(c.id, cx))
    });
    assert!(
        c.vcx
            .cx
            .read(|cx| error_bar::open_question(c.id, cx))
            .is_none()
    );
    assert!(!c.vcx.cx.read(|cx| tx_badge::answer_waits(c.id, cx)));
    c.end(cx);
}

#[gpui::test]
fn error_bar_shows_roll_back_and_continue_in_a_failed_pg_tx(cx: &mut TestAppContext) {
    let c = console(
        cx,
        pg().queries(vec![
            FakeQuery {
                prefix: "BEGIN",
                tx_after: Some(TxStatus::InTx),
                ..Default::default()
            },
            answering("SELECT 1"),
            failing("SELECT * FROM missing"),
            answering("SELECT 3"),
        ]),
        FakeDriver::new(DbmsKind::Postgres),
    );
    c.run_text("BEGIN");
    wait_until(c.vcx, "the user's transaction", |_| {
        let tx = c.session.state().borrow().tx.clone();
        tx.owner == TxOwner::User && tx.status == TxStatus::InTx
    });
    wait_until(c.vcx, "the BEGIN run's end", |vcx| {
        vcx.cx
            .read(|cx| hacivat_app::exec_ui::last_run_ended(c.id, cx))
    });
    let (_, question, offered) = c.ask(&THREE);
    // Roll back and continue in place of Ignore, and no Retry (P-M3b-19).
    assert_eq!(
        offered,
        [
            RunChoice::RollbackAndContinue,
            RunChoice::IgnoreAll,
            RunChoice::Stop
        ]
    );
    for refused in [RunChoice::Retry, RunChoice::Ignore] {
        assert!(!c.answer(question, refused), "{refused:?}");
    }
    assert!(c.vcx.cx.read(|cx| tx_badge::answer_waits(c.id, cx)));
    assert!(c.answer(question, RunChoice::RollbackAndContinue));
    wait_until(c.vcx, "the rollback and the rest", |_| {
        c.fake.executed().iter().any(|s| s == "ROLLBACK")
            && c.typed().ends_with(&["SELECT 3".to_string()])
    });
    // Fix round 1 (m1): written once the session took the answer.
    wait_until(c.vcx, "the rolled-back line", |_| {
        c.log_ids().contains(&"exec-run-rolled-back-continue")
    });
    wait_until(c.vcx, "the run's end", |vcx| {
        vcx.cx
            .read(|cx| hacivat_app::exec_ui::last_run_ended(c.id, cx))
    });
    assert!(!c.vcx.cx.read(|cx| tx_badge::answer_waits(c.id, cx)));
    c.end(cx);
}

// ## m3b-t23-escalation (fix round 1, I3)
#[gpui::test]
fn f6_enters_the_bar_and_leaves_it_without_answering(cx: &mut TestAppContext) {
    let c = console(
        cx,
        pg().queries(vec![
            answering("SELECT 1"),
            failing("SELECT * FROM missing"),
            answering("SELECT 3"),
        ]),
        FakeDriver::new(DbmsKind::Postgres),
    );
    let (_, question, _) = c.ask(&THREE);
    focus_editor(c.vcx, &c.editor);
    let editor_focus = c.vcx.cx.read(|cx| c.editor.read(cx).focus_handle(cx));
    let first = c
        .vcx
        .cx
        .read(|cx| error_bar::button_focus(c.id, cx))
        .first()
        .map(|(_, h)| h.clone())
        .expect("the bar's first button");
    // F6: into the bar (its first button, Retry).
    assert!(run(c.vcx, "workspace.focus_notification"));
    assert_eq!(focused(c.vcx), Some(first));
    // F6 again, no sticky toast up: back to the editor. Nothing was answered.
    assert!(run(c.vcx, "workspace.focus_notification"));
    assert_eq!(focused(c.vcx), Some(editor_focus));
    assert_eq!(
        c.vcx
            .cx
            .read(|cx| error_bar::open_question(c.id, cx))
            .map(|(_, q, _)| q),
        Some(question)
    );
    assert_eq!(c.typed(), &THREE[..2]);
    assert!(c.answer(question, RunChoice::Stop));
    c.end(cx);
}

#[gpui::test]
fn closing_the_console_answers_stop(cx: &mut TestAppContext) {
    let c = console(
        cx,
        pg().queries(vec![
            answering("SELECT 1"),
            failing("SELECT * FROM missing"),
            answering("SELECT 3"),
        ]),
        FakeDriver::new(DbmsKind::Postgres),
    );
    let (run_seq, question, _) = c.ask(&THREE);
    assert!(run(c.vcx, "workspace.close_tab"));
    c.vcx.run_until_parked();
    assert!(
        c.vcx.cx.read(|cx| Consoles::get(cx, c.id).is_none()),
        "the console closed"
    );
    // The question was answered Stop: the run ended without the third statement, and the
    // session refuses any later answer to it.
    wait_until(c.vcx, "the run to stop", |_| {
        !matches!(
            c.session.state().borrow().state,
            SessionState::Busy { req, .. } if req == run_seq
        )
    });
    assert_eq!(c.typed(), &THREE[..2]);
    let late = futures::executor::block_on(c.session.choose(run_seq, question, RunChoice::Ignore));
    assert_eq!(late, Ok(false));
    assert!(
        c.vcx
            .cx
            .read(|cx| error_bar::open_question(c.id, cx))
            .is_none()
    );
    c.end(cx);
}
