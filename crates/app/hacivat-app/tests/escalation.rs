//! M3b Task 23: cancel escalation and the error-policy bar (HV-EXEC-08, -09 UI; R-M3-61;
//! `design/m3b-safe` surfaces 6 and 7), end to end over FakeDriver sessions.
//!
//! - A cancel the server ignores becomes slow at 10 s: the toast offers only Show options….
//! - A second Stop while cancelling opens the 420 alertdialog with Keep waiting focused;
//!   Escape answers Keep waiting and never sends Terminate or Deactivate (R-M3-38).
//! - A terminate the server refuses shows its text verbatim, and the dialog stays.
//! - The error bar's answer reaches the session's run gate, once; a failed PostgreSQL
//!   transaction offers Roll back and continue (no Retry); closing the console answers Stop.
//!
//! Every wait is on a condition; the slow cancel's 10 s are the session's real clock (the core
//! runtime is not paused), so that one test waits for it with a longer bound.

mod support;

use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::{Entity, TestAppContext, VisualTestContext};
use hacivat_app::connections::prompt_kit::parts;
use hacivat_app::console::tx_badge;
use hacivat_app::console::{ConsoleId, Consoles};
use hacivat_app::exec_ui::context::SqlContext;
use hacivat_app::exec_ui::{error_bar, escalation};
use hacivat_driver::fake::{
    FakeDriver, FakeExec, FakeQuery, FakeResultSet, FakeRows, FakeTypedCall,
};
use hacivat_driver::{ExecSummary, RawError, RawErrorKind, ServerMessage, TxStatus};
use hacivat_exec::RunChoice;
use hacivat_proto::{DbmsKind, EnvTag};
use hacivat_session::session::{SessionHandle, SessionState, TxOwner};
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
    (0..8)
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

fn console(cx: &mut TestAppContext, fake: FakeDriver, side: FakeDriver) -> Console {
    let vcx = shell_with_connections(cx, "en");
    in_window(vcx, |window, _| window.activate_window());
    let p = profile("orders-dev");
    let profile = add_profile(vcx, p.clone());
    let rt = vcx.cx.read(|cx| {
        cx.global::<hacivat_app::connections::ConnectionsModel>()
            .core_runtime()
            .handle()
    });
    let (session, fake, side) = futures::executor::block_on(
        hacivat_session::testing::fake_session_with_side(rt, fake, side, p),
    );
    let id = register_test_console(vcx, profile, session.clone());
    wait_until(vcx, "the console's SQL context", |vcx| {
        vcx.cx
            .read(|cx| hacivat_app::exec_ui::sql_context(id, cx) != SqlContext::Pending)
    });
    let editor = vcx.cx.read(|cx| {
        Consoles::get(cx, id)
            .and_then(|s| s.editor.clone())
            .expect("the console has an editor")
    });
    Console {
        vcx,
        id,
        fake,
        side,
        session,
        editor,
    }
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

impl Console {
    fn run_text(&self, text: &str) {
        let text = text.to_string();
        self.vcx.cx.update(|cx| {
            self.editor.update(cx, |e, cx| {
                e.set_text(&text, cx);
                e.select_range(0..0, cx);
            });
        });
        self.vcx.run_until_parked();
        let editor = self.editor.clone();
        in_window(self.vcx, |window, cx| {
            editor.update(cx, |e, cx| assert!(e.run_command("editor.run", window, cx)));
        });
        self.vcx.run_until_parked();
    }

    fn marks(&self) -> Vec<ExecMarkState> {
        self.vcx.cx.read(|cx| {
            hacivat_app::exec_ui::last_run_marks(self.id, cx)
                .into_iter()
                .flatten()
                .collect()
        })
    }

    /// Runs a statement the server never answers, and cancels it once.
    fn cancelling(&self) {
        self.run_text("SELECT pg_sleep(600)");
        wait_until(self.vcx, "the statement to run", |_| {
            matches!(self.marks().as_slice(), [ExecMarkState::Running { .. }])
        });
        assert!(run(self.vcx, "exec.cancel"));
        wait_until(self.vcx, "the session to cancel", |_| {
            matches!(
                self.session.state().borrow().state,
                SessionState::Cancelling { .. }
            )
        });
    }

    fn dialog_open(&self) -> bool {
        self.vcx.cx.read(|cx| escalation::dialog_open(self.id, cx))
    }

    fn focused_part(&self) -> Option<String> {
        in_window(self.vcx, |window, cx| parts::focused(window, cx))
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
        self.vcx.cx.read(|cx| {
            Consoles::get(cx, self.id)
                .map(|s| s.log.lines().iter().map(|l| l.msg.fluent_id()).collect())
                .unwrap_or_default()
        })
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
    assert!(run(c.vcx, "exec.cancel"));
    c.vcx.run_until_parked();
    assert!(c.dialog_open());
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
    assert!(run(c.vcx, "exec.cancel"));
    c.vcx.run_until_parked();
    assert!(c.dialog_open());
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
    assert!(run(c.vcx, "exec.cancel"));
    c.vcx.run_until_parked();
    c.vcx.simulate_keystrokes("tab");
    c.vcx.simulate_keystrokes("escape");
    c.vcx.run_until_parked();
    assert!(!c.dialog_open());
    assert!(c.side.executed().is_empty(), "{:?}", c.side.executed());
    c.end(cx);
}

#[gpui::test]
fn failed_terminate_shows_the_server_text_verbatim(cx: &mut TestAppContext) {
    let side = FakeDriver::new(DbmsKind::Postgres).exec(vec![FakeExec {
        prefix: "SELECT pg_terminate_backend",
        result: Err(refusal()),
        delay: Duration::ZERO,
        tx_after: None,
        panics: false,
    }]);
    let c = console(cx, pg().queries(vec![deaf("SELECT pg_sleep(600)")]), side);
    enable_registry(c.vcx);
    c.cancelling();
    assert!(run(c.vcx, "exec.cancel"));
    c.vcx.run_until_parked();
    // Tab to Terminate on server (Keep waiting → Disconnect → Terminate), Enter.
    c.vcx.simulate_keystrokes("tab tab");
    assert_eq!(c.focused_part().as_deref(), Some("escalation-terminate"));
    c.vcx.simulate_keystrokes("enter");
    wait_until(c.vcx, "the refusal", |vcx| {
        vcx.cx
            .read(|cx| escalation::dialog_state(c.id, cx))
            .is_some_and(|(_, _, refused)| refused)
    });
    assert_eq!(c.side.executed(), ["SELECT pg_terminate_backend(7)"]);
    let (_, server, _) = c
        .vcx
        .cx
        .read(|cx| escalation::dialog_state(c.id, cx))
        .expect("the dialog stays");
    assert_eq!(
        server.as_deref(),
        Some("must be a superuser to terminate superuser process")
    );
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
    assert!(
        c.log_ids().contains(&"exec-terminate-failed"),
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
    let (_, _, offered) = c.ask(&THREE);
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
    // A choice the bar does not offer is refused; Ignore goes to the session, once.
    assert!(
        !c.vcx
            .cx
            .update(|cx| error_bar::answer(c.id, RunChoice::RollbackAndContinue, cx))
    );
    assert!(
        c.vcx
            .cx
            .update(|cx| error_bar::answer(c.id, RunChoice::Ignore, cx))
    );
    assert!(
        !c.vcx
            .cx
            .update(|cx| error_bar::answer(c.id, RunChoice::Stop, cx)),
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
    let (_, _, offered) = c.ask(&THREE);
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
        assert!(
            !c.vcx.cx.update(|cx| error_bar::answer(c.id, refused, cx)),
            "{refused:?}"
        );
    }
    assert!(c.vcx.cx.read(|cx| tx_badge::answer_waits(c.id, cx)));
    assert!(
        c.vcx
            .cx
            .update(|cx| error_bar::answer(c.id, RunChoice::RollbackAndContinue, cx))
    );
    wait_until(c.vcx, "the rollback and the rest", |_| {
        c.fake.executed().iter().any(|s| s == "ROLLBACK")
            && c.typed().ends_with(&["SELECT 3".to_string()])
    });
    assert!(
        c.log_ids().contains(&"exec-run-rolled-back-continue"),
        "{:?}",
        c.log_ids()
    );
    wait_until(c.vcx, "the run's end", |vcx| {
        vcx.cx
            .read(|cx| hacivat_app::exec_ui::last_run_ended(c.id, cx))
    });
    assert!(!c.vcx.cx.read(|cx| tx_badge::answer_waits(c.id, cx)));
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
