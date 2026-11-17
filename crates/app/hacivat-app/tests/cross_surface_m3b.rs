//! M3b Task 29: the cross-surface flows of `design/m3b-safe`, each end to end through the
//! product's own Run (Task 24's orchestrator: classify → parameters → gate → confirmations →
//! run) over a FakeDriver session registered on a console, and every wait on a condition.
//!
//! - Production writes (R-M3-55, D-036, D-043, T26): on PostgreSQL a DML runs first and asks
//!   after (Commit / Roll back in Auto-commit, Keep / Undo in Manual), then the log and the
//!   gutter say what happened; on MySQL a DML is confirmed before it is sent; `DELETE FROM t`
//!   needs its name typed; the confirm-in-transaction deadline undoes the statement and says so.
//!   Production never offers "don't ask again" nor Execute all.
//! - The error policy's Roll back and continue (P-M3b-19), a parameter prompt into a bound run
//!   (R-M3-79), Run as one into untyped tabs (R-M3-60), the idle toast's Keep open (D-014), the
//!   slow cancel into the escalation dialog's Keep waiting (HV-EXEC-09), and History's Insert
//!   and run through the gate (D-061).
#![cfg_attr(not(unix), allow(dead_code, unused_imports))]

mod support;

use std::sync::Arc;
use std::time::Duration;

use gpui::{Entity, Focusable as _, TestAppContext, VisualTestContext};
use hacivat_app::connections::prompt_kit::parts;
use hacivat_app::console::{ConsoleId, Consoles, idle_toast, tx_badge};
use hacivat_app::exec_ui::context::SqlContext;
use hacivat_app::exec_ui::{self, commit_prompt, confirm, escalation, orchestrator, params_prompt};
use hacivat_driver::fake::{
    FakeCall, FakeDriver, FakeExec, FakeQuery, FakeResultSet, FakeTypedCall,
};
use hacivat_driver::{ExecSummary, RawError, RawErrorKind, ServerMessage, TxStatus};
use hacivat_exec::{ConfirmAnswer, RunChoice};
use hacivat_proto::{DbmsKind, EnvTag, ProfileId};
use hacivat_session::session::{
    ExecOrigin, Request, SessionHandle, SessionState, TxCommand, TxMode, TxOwner,
};
use hacivat_ui_editor::{EditorView, ExecMarkState};
use hacivat_values::{BindValue, LogicalType};
use support::*;

// ---- the console ------------------------------------------------------------------------------

/// A console over a FakeDriver session of its profile, with its SQL context read.
struct Console {
    vcx: &'static mut VisualTestContext,
    id: ConsoleId,
    profile: ProfileId,
    fake: Arc<FakeDriver>,
    side: Option<Arc<FakeDriver>>,
    session: SessionHandle,
    editor: Entity<EditorView>,
}

fn profile(name: &str, engine: DbmsKind, env: EnvTag) -> hacivat_session::profile::Profile {
    hacivat_session::profile::Profile {
        engine,
        ..test_profile(name, Some(env), "127.0.0.1")
    }
}

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
        .backend(7)
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

/// MySQL answering the console's SQL-context probe.
fn mysql() -> FakeDriver {
    FakeDriver::new(DbmsKind::MySql)
        .tracks_transactions()
        .exec(reads(
            "SELECT @@SESSION.sql_mode",
            vec!["ONLY_FULL_GROUP_BY"],
        ))
}

fn console(
    cx: &mut TestAppContext,
    p: hacivat_session::profile::Profile,
    fake: FakeDriver,
) -> Console {
    console_with(cx, p, fake, None)
}

/// [`console`], its session with a side connection (`fake_session_with_side`) when `side` is
/// given (the escalation dialog's terminate goes there).
fn console_with(
    cx: &mut TestAppContext,
    p: hacivat_session::profile::Profile,
    fake: FakeDriver,
    side: Option<FakeDriver>,
) -> Console {
    let vcx = shell_with_connections(cx, "en");
    in_window(vcx, |window, _| window.activate_window());
    let profile = add_profile(vcx, p.clone());
    let rt = vcx.cx.read(|cx| {
        cx.global::<hacivat_app::connections::ConnectionsModel>()
            .core_runtime()
            .handle()
    });
    let (session, fake, side) = match side {
        Some(side) => {
            let (s, f, d) = futures::executor::block_on(
                hacivat_session::testing::fake_session_with_side(rt, fake, side, p),
            );
            (s, f, Some(d))
        }
        None => {
            let (s, f) =
                futures::executor::block_on(hacivat_session::testing::fake_session(rt, fake, p));
            (s, f, None)
        }
    };
    let id = register_test_console(vcx, profile, session.clone());
    wait_until(vcx, "the console's SQL context", |vcx| {
        vcx.cx
            .read(|cx| exec_ui::sql_context(id, cx) != SqlContext::Pending)
    });
    let editor = vcx.cx.read(|cx| {
        Consoles::get(cx, id)
            .and_then(|s| s.editor.clone())
            .expect("the console has an editor")
    });
    Console {
        vcx,
        id,
        profile,
        fake,
        side,
        session,
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

    fn select(&self, r: std::ops::Range<usize>) {
        self.vcx
            .cx
            .update(|cx| self.editor.update(cx, |e, cx| e.select_range(r, cx)));
        self.vcx.run_until_parked();
    }

    fn run_selected(&self) {
        let editor = self.editor.clone();
        in_window(self.vcx, |window, cx| {
            let handle = editor.read(cx).focus_handle(cx);
            window.focus(&handle, cx);
            editor.update(cx, |e, cx| assert!(e.run_command("editor.run", window, cx)));
        });
        self.vcx.run_until_parked();
    }

    /// Run with the caret at byte `at`.
    fn run_at(&self, at: usize) {
        self.select(at..at);
        self.run_selected();
    }

    /// Sets the text and runs all of it as a selection (one unit per statement).
    fn run_all(&self, text: &str) {
        self.set_text(text);
        self.select(0..text.len());
        self.run_selected();
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

    fn statements(&self) -> Vec<String> {
        self.fake
            .call_log()
            .into_iter()
            .filter_map(|c| match c {
                FakeCall::Statement(s) => Some(s),
                _ => None,
            })
            .collect()
    }

    /// Whether any call the connection heard carries `needle`.
    fn ever_sent(&self, needle: &str) -> bool {
        self.fake
            .call_log()
            .iter()
            .any(|c| format!("{c:?}").contains(needle))
    }

    fn log_ids(&self) -> Vec<&'static str> {
        self.vcx.cx.read(|cx| {
            Consoles::get(cx, self.id)
                .map(|s| s.log.lines().iter().map(|l| l.msg.fluent_id()).collect())
                .unwrap_or_default()
        })
    }

    fn wait_log(&self, id: &'static str) {
        wait_until(self.vcx, id, |_| self.log_ids().contains(&id));
    }

    fn marks(&self) -> Vec<ExecMarkState> {
        self.vcx.cx.read(|cx| {
            exec_ui::last_run_marks(self.id, cx)
                .into_iter()
                .flatten()
                .collect()
        })
    }

    fn wait_run_end(&self) {
        wait_until(self.vcx, "the run's end", |vcx| {
            vcx.cx.read(|cx| exec_ui::last_run_ended(self.id, cx))
        });
    }

    fn confirm_open(&self) -> bool {
        self.vcx.cx.read(confirm::confirm_open)
    }

    /// Waits for a confirmation, then lets its run buttons arm (T21 fix round 1, I1).
    fn wait_confirm(&self) {
        wait_until(self.vcx, "a confirmation", |vcx| {
            vcx.cx.read(confirm::confirm_open)
        });
        self.arm(confirm::ARM_DELAY);
    }

    fn commit_open(&self) -> bool {
        layer_ids(self.vcx).contains(&commit_prompt::COMMIT_LAYER_ID)
    }

    /// Waits for the confirm-in-transaction prompt, then lets Commit / Keep arm.
    fn wait_commit(&self) {
        wait_until(self.vcx, "the confirm-in-transaction prompt", |vcx| {
            layer_ids(vcx).contains(&commit_prompt::COMMIT_LAYER_ID)
        });
        // Commit / Keep arm as the confirmation's run buttons do.
        self.arm(confirm::ARM_DELAY);
    }

    fn arm(&self, delay: Duration) {
        self.vcx
            .executor()
            .advance_clock(delay + Duration::from_millis(50));
        self.vcx.run_until_parked();
    }

    fn click(&mut self, id: &str) {
        let selector: &'static str = Box::leak(format!("button-{id}").into_boxed_str());
        click(self.vcx, selector);
        self.vcx.run_until_parked();
    }

    fn buttons(&self) -> Vec<&'static str> {
        self.vcx
            .cx
            .read(confirm::open_dialog_buttons)
            .into_iter()
            .map(|b| b.id)
            .collect()
    }

    fn set_mode(&self, mode: TxMode) {
        let _ticket = self.session.submit(Request::Tx(TxCommand::SetMode(mode)));
        wait_until(self.vcx, "the mode", |_| {
            self.session.state().borrow().tx.mode == mode
        });
    }

    fn end(self, cx: &mut TestAppContext) {
        self.fake.kill_connections();
        stop_core_runtime_test(cx);
    }
}

fn prod_pg(cx: &mut TestAppContext) -> Console {
    console(
        cx,
        profile("orders-prod", DbmsKind::Postgres, EnvTag::Production),
        pg(),
    )
}

// ---- production writes (surfaces 1, 2, 4, 13) -------------------------------------------------

/// PostgreSQL production in Auto-commit: `UPDATE` runs in Hacivat's own transaction, the prompt
/// asks Commit / Roll back, Commit sends `COMMIT`, then Output says it was committed and the
/// gutter shows the statement succeeded. In Manual (the production default, D-036) the same
/// statement asks Keep / Undo, and Keep leaves it in the user's transaction: the badge counts it.
#[gpui::test]
fn pg_production_update_asks_after_it_runs_then_commit_reaches_log_and_gutter(
    cx: &mut TestAppContext,
) {
    let mut c = prod_pg(cx);
    assert_eq!(c.session.state().borrow().tx.mode, TxMode::Manual, "D-036");
    c.set_mode(TxMode::Auto);
    c.set_text("UPDATE t SET a = 1 WHERE id = 3");
    c.run_at(0);
    c.wait_commit();
    assert!(!c.confirm_open(), "a DML on PostgreSQL asks after it ran");
    assert_eq!(c.queries(), ["UPDATE t SET a = 1 WHERE id = 3"]);
    assert!(
        !c.statements().iter().any(|s| s.contains("COMMIT")),
        "nothing committed before the answer: {:?}",
        c.statements()
    );
    assert!(
        c.vcx.debug_bounds("button-commit-commit").is_some(),
        "the own-transaction form"
    );
    c.click("commit-commit");
    wait_until(c.vcx, "the COMMIT", |_| {
        c.statements().iter().any(|s| s == "COMMIT")
    });
    c.wait_log("exec-confirm-committed");
    c.wait_run_end();
    assert!(!c.commit_open());
    assert!(
        matches!(c.marks().as_slice(), [ExecMarkState::Succeeded { .. }]),
        "{:?}",
        c.marks()
    );

    // Manual: the savepoint form; Keep joins the change to the user's transaction.
    c.set_mode(TxMode::Manual);
    c.run_at(0);
    c.wait_commit();
    assert!(
        c.vcx.debug_bounds("button-commit-keep").is_some(),
        "the savepoint form"
    );
    c.click("commit-keep");
    wait_until(c.vcx, "the badge to count the kept statement", |vcx| {
        vcx.cx
            .read(|cx| tx_badge::user_tx(c.id, cx))
            .is_some_and(|u| u.statements >= 1 && !u.failed)
    });
    c.wait_run_end();
    assert!(
        !c.statements().iter().any(|s| s.starts_with("ROLLBACK")),
        "{:?}",
        c.statements()
    );
    c.end(cx);
}

/// MySQL production (D-043, T26): a DML is confirmed before anything is sent (a table without
/// transactions keeps a write no rollback undoes). The dialog offers neither "don't ask again"
/// nor Execute all; Execute sends it, then the log and the gutter say it ran.
#[gpui::test]
fn mysql_production_update_is_confirmed_before_it_is_sent(cx: &mut TestAppContext) {
    let mut c = console(
        cx,
        profile("shop-prod", DbmsKind::MySql, EnvTag::Production),
        mysql(),
    );
    c.set_text("UPDATE t SET a = 1 WHERE id = 3");
    c.run_at(0);
    c.wait_confirm();
    assert!(c.queries().is_empty(), "nothing sent before the answer");
    let buttons = c.buttons();
    assert!(buttons.contains(&"confirm-execute"), "{buttons:?}");
    assert!(!buttons.contains(&"confirm-suppress"), "{buttons:?}");
    assert!(!buttons.contains(&"confirm-execute-all"), "{buttons:?}");
    c.click("confirm-execute");
    wait_until(c.vcx, "the UPDATE", |_| {
        c.queries() == ["UPDATE t SET a = 1 WHERE id = 3"]
    });
    c.wait_run_end();
    assert!(
        !c.commit_open(),
        "MySQL never asks Keep / Undo after it ran"
    );
    assert!(
        matches!(c.marks().as_slice(), [ExecMarkState::Succeeded { .. }]),
        "{:?}",
        c.marks()
    );
    assert!(!c.log_ids().contains(&"exec-confirm-cancelled"));
    c.end(cx);
}

/// `DELETE FROM t` on production: its name must be typed (exact bytes), nothing is sent until
/// Execute, and Execute is off until the name matches.
#[gpui::test]
fn production_delete_from_t_needs_the_name_typed(cx: &mut TestAppContext) {
    let mut c = prod_pg(cx);
    c.set_text("DELETE FROM t;");
    c.run_at(0);
    c.wait_confirm();
    let execute = |c: &Console| {
        c.vcx
            .cx
            .read(confirm::open_dialog_buttons)
            .into_iter()
            .find(|b| b.answer == ConfirmAnswer::Execute)
            .is_some_and(|b| b.enabled)
    };
    assert!(!execute(&c), "Execute waits for the name");
    assert!(!c.buttons().contains(&"confirm-suppress"));
    let field = c
        .vcx
        .cx
        .read(|cx| parts::focus(cx, "confirm-type"))
        .expect("the typed-name field");
    in_window(c.vcx, |window, cx| window.focus(&field, cx));
    c.vcx.run_until_parked();
    c.vcx.simulate_input("T");
    c.vcx.run_until_parked();
    assert!(!execute(&c), "letter case counts");
    c.vcx.simulate_keystrokes("backspace");
    c.vcx.simulate_input("t");
    c.vcx.run_until_parked();
    assert!(execute(&c));
    assert!(!c.ever_sent("DELETE"), "{:?}", c.fake.call_log());
    c.click("confirm-execute");
    wait_until(c.vcx, "the DELETE", |_| c.ever_sent("DELETE FROM t"));
    c.end(cx);
}

/// The confirm-in-transaction deadline (D-014): with no answer, the prompt answers for the user
/// when its countdown ends; the statement is undone (`ROLLBACK TO SAVEPOINT`), never committed,
/// the user's transaction stays open, and Output says the statement was undone.
#[gpui::test]
fn the_confirm_in_transaction_deadline_undoes_the_statement_and_says_so(cx: &mut TestAppContext) {
    let c = prod_pg(cx);
    c.set_text("UPDATE t SET a = 1 WHERE id = 3");
    c.run_at(0);
    c.wait_commit();
    let (left, _) = c
        .vcx
        .cx
        .read(commit_prompt::countdown_of)
        .expect("the countdown");
    assert!(left.starts_with("4:") || left.starts_with("5:"), "{left}");
    // The executor's clock runs past the deadline (5 min, D-014).
    c.vcx
        .executor()
        .advance_clock(Duration::from_secs(5 * 60 + 1));
    c.vcx.run_until_parked();
    wait_until(c.vcx, "the undo", |_| {
        c.statements()
            .iter()
            .any(|s| s == "ROLLBACK TO SAVEPOINT hv_confirm")
    });
    c.wait_log("exec-confirm-undone");
    wait_until(c.vcx, "the prompt to close", |_| !c.commit_open());
    assert!(
        !c.statements().iter().any(|s| s == "COMMIT"),
        "{:?}",
        c.statements()
    );
    // Only the confirmed statement went: the user's transaction (Manual) is still open.
    assert_eq!(
        c.session.state().borrow().tx.status,
        TxStatus::InTx,
        "the user's transaction stays open"
    );
    c.end(cx);
}

/// Production never offers "don't ask again", nor Execute all (T26 fix round 2, N1/R1/R2):
/// two statements that each ask show only Execute, Skip, Skip all and Cancel. Off production
/// the same shape of script offers both.
#[gpui::test]
fn production_never_offers_dont_ask_again_nor_execute_all(cx: &mut TestAppContext) {
    let c = prod_pg(cx);
    c.run_all("CREATE INDEX i_a ON a (x);\nCREATE INDEX i_b ON b (y);");
    c.wait_confirm();
    let buttons = c.buttons();
    assert!(
        buttons.contains(&"confirm-execute") && buttons.contains(&"confirm-skip-all"),
        "statement 1 of 2: {buttons:?}"
    );
    assert!(!buttons.contains(&"confirm-suppress"), "{buttons:?}");
    assert!(!buttons.contains(&"confirm-execute-all"), "{buttons:?}");
    assert!(!c.ever_sent("CREATE INDEX"));
    c.end(cx);
}

#[gpui::test]
fn off_production_the_same_dialog_offers_both(cx: &mut TestAppContext) {
    let c = console(
        cx,
        profile("orders-staging", DbmsKind::Postgres, EnvTag::Staging),
        pg(),
    );
    c.run_all("UPDATE a SET x = 1;\nUPDATE b SET y = 2;");
    c.wait_confirm();
    let buttons = c.buttons();
    assert!(buttons.contains(&"confirm-suppress"), "{buttons:?}");
    assert!(buttons.contains(&"confirm-execute-all"), "{buttons:?}");
    assert!(!c.ever_sent("UPDATE"));
    c.end(cx);
}

// ---- the error policy (surface 7) ----------------------------------------------------------

fn one_text_row() -> FakeResultSet {
    FakeResultSet {
        columns: vec![(
            "v",
            LogicalType::Int {
                bits: 32,
                signed: true,
            },
        )],
        rows: hacivat_driver::fake::FakeRows::Literal(vec![vec![hacivat_values::Value::Int(1)]]),
    }
}

fn answering(prefix: &'static str) -> FakeQuery {
    FakeQuery {
        prefix,
        sets: vec![one_text_row()],
        ..Default::default()
    }
}

/// A script whose second statement (a write) fails inside the user's PostgreSQL transaction,
/// which fails with it: the run
/// pauses on the bar, which offers Roll back and continue (and no Retry, no Ignore); the answer
/// rolls the transaction back, runs the rest, and Output says so.
#[gpui::test]
fn error_policy_roll_back_and_continue(cx: &mut TestAppContext) {
    // A write (a read runs in a cursor savepoint, which its failure leaves).
    let missing = FakeQuery {
        prefix: "UPDATE missing",
        // The fake fails a statement once it has a result to fail in.
        sets: vec![one_text_row()],
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
        // PostgreSQL: an error inside a transaction fails it.
        tx_after: Some(TxStatus::Failed),
        ..Default::default()
    };
    let mut c = console(
        cx,
        profile("orders-dev", DbmsKind::Postgres, EnvTag::Dev),
        pg().queries(vec![
            FakeQuery {
                prefix: "BEGIN",
                tx_after: Some(TxStatus::InTx),
                ..Default::default()
            },
            answering("SELECT 1"),
            missing,
            answering("SELECT 3"),
        ]),
    );
    c.set_text("BEGIN");
    c.run_at(0);
    wait_until(c.vcx, "the user's transaction", |_| {
        let tx = c.session.state().borrow().tx.clone();
        tx.owner == TxOwner::User && tx.status == TxStatus::InTx
    });
    c.wait_run_end();
    c.run_all("SELECT 1;\nUPDATE missing SET a = 1 WHERE id = 1;\nSELECT 3;");
    wait_until(c.vcx, "the error bar", |vcx| {
        vcx.cx
            .read(|cx| exec_ui::error_bar::open_question(c.id, cx))
            .is_some()
    });
    let (_, _, offered) = c
        .vcx
        .cx
        .read(|cx| exec_ui::error_bar::open_question(c.id, cx))
        .expect("the question");
    assert_eq!(
        offered,
        [
            RunChoice::RollbackAndContinue,
            RunChoice::IgnoreAll,
            RunChoice::Stop
        ]
    );
    assert!(!c.queries().iter().any(|q| q == "SELECT 3"), "paused");
    c.click("errbar-rollback-continue");
    wait_until(c.vcx, "the rollback and the rest", |_| {
        c.statements().iter().any(|s| s == "ROLLBACK")
            && c.queries().last().is_some_and(|q| q == "SELECT 3")
    });
    c.wait_log("exec-run-rolled-back-continue");
    c.wait_run_end();
    assert!(
        c.vcx
            .cx
            .read(|cx| exec_ui::error_bar::open_question(c.id, cx))
            .is_none()
    );
    c.end(cx);
}

// ---- parameters and Run as one (surfaces 8, 9, 10) ------------------------------------------

/// The parameter prompt into a bound run: the value is bound (never spliced), the gutter marks
/// the statement, and the prompt asks again next time with the value remembered.
#[gpui::test]
fn params_prompt_into_a_bound_run(cx: &mut TestAppContext) {
    let c = console(
        cx,
        profile("orders-dev", DbmsKind::Postgres, EnvTag::Dev),
        pg(),
    );
    c.set_text("SELECT * FROM t WHERE id = :id");
    c.run_at(0);
    wait_until(c.vcx, "the parameter prompt", |vcx| {
        vcx.cx.read(|cx| params_prompt::open_prompt(cx).is_some())
            && layer_ids(vcx).contains(&params_prompt::PARAMS_LAYER_ID)
    });
    assert!(c.queries().is_empty(), "nothing sent before the values");
    let prompt = c.vcx.cx.read(params_prompt::open_prompt).expect("open");
    c.vcx
        .cx
        .update(|cx| prompt.update(cx, |p, cx| p.set_text(0, "42", cx)));
    in_window(c.vcx, |window, cx| {
        prompt.update(cx, |p, cx| p.submit(window, cx))
    });
    wait_until(c.vcx, "the bound query", |_| !c.queries().is_empty());
    assert_eq!(c.queries(), ["SELECT * FROM t WHERE id = $1"]);
    let binds: Vec<Vec<BindValue>> = c
        .fake
        .typed_log()
        .into_iter()
        .filter_map(|call| match call {
            FakeTypedCall::Binds(b) => Some(b),
            _ => None,
        })
        .collect();
    assert_eq!(
        binds,
        [vec![BindValue::Typed {
            hint: LogicalType::Int {
                bits: 64,
                signed: true
            },
            text: Arc::from("42"),
        }]]
    );
    assert!(!c.ever_sent("= 42"), "never spliced");
    c.wait_run_end();
    assert!(
        matches!(c.marks().as_slice(), [ExecMarkState::Succeeded { .. }]),
        "{:?}",
        c.marks()
    );
    c.end(cx);
}

/// Run as one (the console's selection policy, CHOOSE-06): the selection goes in one request,
/// and each of its result sets lands in its own tab, untyped.
#[gpui::test]
fn run_as_one_lands_in_untyped_tabs(cx: &mut TestAppContext) {
    let text_set = |name: &'static str| FakeResultSet {
        columns: vec![(
            name,
            LogicalType::Text {
                max_chars: None,
                fixed: false,
            },
        )],
        rows: hacivat_driver::fake::FakeRows::Literal(vec![vec![hacivat_values::Value::Text(
            "1".into(),
        )]]),
    };
    let script = "SELECT 1 AS a;\nSELECT 2 AS b;";
    let c = console(
        cx,
        profile("orders-dev", DbmsKind::Postgres, EnvTag::Dev),
        pg().queries(vec![FakeQuery {
            prefix: "SELECT 1 AS a;",
            sets: vec![text_set("a"), text_set("b")],
            ..Default::default()
        }]),
    );
    c.vcx.cx.update(|cx| {
        let policy = orchestrator::policy_of(c.id, cx);
        orchestrator::set_policy(
            c.id,
            hacivat_sql_editor::RunPolicy {
                selection_as_one: true,
                ..policy
            },
            cx,
        );
    });
    c.run_all(script);
    wait_until(c.vcx, "the one request", |_| !c.queries().is_empty());
    assert_eq!(c.queries(), [script], "one request");
    c.wait_run_end();
    let tabs = || {
        c.vcx.cx.read(|cx| {
            Consoles::get(cx, c.id)
                .and_then(|s| s.results.clone())
                .map(|r| {
                    r.read(cx)
                        .tabs()
                        .iter()
                        .map(|t| t.untyped())
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default()
        })
    };
    wait_until(c.vcx, "two result tabs", |_| tabs().len() == 2);
    assert_eq!(tabs(), [true, true], "both untyped");
    c.end(cx);
}

// ---- the idle toast and the slow cancel (surfaces 5, 6) -------------------------------------

/// The idle toast's Keep open: the toast goes, nothing is sent, the user's transaction and its
/// badge stay.
#[gpui::test]
fn idle_toast_keep_leaves_the_transaction_open(cx: &mut TestAppContext) {
    let c = console(
        cx,
        profile("orders-staging", DbmsKind::Postgres, EnvTag::Staging),
        pg(),
    );
    for sql in ["BEGIN", "UPDATE t SET a = 1 WHERE id = 3"] {
        let _ticket = c.session.submit(Request::Exec {
            sql: sql.to_string(),
            opts: hacivat_driver::ExecOpts::default(),
            origin: ExecOrigin::User,
        });
    }
    wait_until(c.vcx, "the user's transaction", |vcx| {
        vcx.cx.read(|cx| tx_badge::user_tx(c.id, cx)).is_some()
    });
    c.vcx
        .cx
        .update(|cx| idle_toast::show(c.session.id(), 10, cx));
    c.vcx.run_until_parked();
    let heard = c.fake.call_log().len();
    let toast = c
        .vcx
        .cx
        .read(|cx| idle_toast::toast_id(c.session.id(), cx))
        .expect("the idle toast");
    let keep: &'static str = Box::leak(format!("button-toast-action-{toast}-2").into_boxed_str());
    click(c.vcx, keep);
    c.vcx.run_until_parked();
    assert!(
        c.vcx.cx.read(idle_toast::shown).is_empty(),
        "the toast went"
    );
    assert_eq!(c.fake.call_log().len(), heard, "nothing was sent");
    assert_eq!(c.session.state().borrow().tx.status, TxStatus::InTx);
    assert!(c.vcx.cx.read(|cx| tx_badge::user_tx(c.id, cx)).is_some());
    c.end(cx);
}

/// A cancel the server ignores: the slow-cancel toast after 10 s, then the second Stop opens
/// the escalation dialog on Keep waiting; Enter keeps waiting: nothing is terminated or
/// disconnected, the session is still cancelling, and the gutter shows the hourglass.
#[gpui::test]
fn slow_cancel_into_the_dialog_then_keep_waiting(cx: &mut TestAppContext) {
    let deaf = FakeQuery {
        prefix: "SELECT pg_sleep(600)",
        first_delay: Duration::from_secs(600),
        ignores_cancel: true,
        ..Default::default()
    };
    let c = console_with(
        cx,
        profile("orders-dev", DbmsKind::Postgres, EnvTag::Dev),
        pg().queries(vec![deaf]),
        Some(FakeDriver::new(DbmsKind::Postgres)),
    );
    c.set_text("SELECT pg_sleep(600)");
    c.run_at(0);
    wait_until(c.vcx, "the statement to run", |_| {
        matches!(c.marks().as_slice(), [ExecMarkState::Running { .. }])
    });
    assert!(run(c.vcx, "exec.cancel"));
    wait_until(c.vcx, "the session to cancel", |_| {
        matches!(
            c.session.state().borrow().state,
            SessionState::Cancelling { .. }
        )
    });
    // The toast comes after the session's 10 s mark (real time on the core runtime).
    c.vcx.executor().allow_parking();
    let deadline = std::time::Instant::now() + Duration::from_secs(40);
    while c.vcx.cx.read(escalation::slow_toasts).is_empty() {
        assert!(
            std::time::Instant::now() < deadline,
            "the slow-cancel toast"
        );
        std::thread::sleep(Duration::from_millis(20));
        c.vcx.run_until_parked();
    }
    // The second Stop: the dialog, Keep waiting focused.
    assert!(run(c.vcx, "exec.cancel"));
    c.vcx.run_until_parked();
    assert!(c.vcx.cx.read(|cx| escalation::dialog_open(c.id, cx)));
    assert_eq!(
        in_window(c.vcx, |window, cx| parts::focused(window, cx)).as_deref(),
        Some("escalation-keep")
    );
    c.vcx.simulate_keystrokes("enter");
    c.vcx.run_until_parked();
    assert!(!c.vcx.cx.read(|cx| escalation::dialog_open(c.id, cx)));
    let side = c.side.clone().expect("the side connection");
    assert!(
        side.executed().is_empty(),
        "nothing terminated: {:?}",
        side.executed()
    );
    assert!(matches!(
        c.session.state().borrow().state,
        SessionState::Cancelling { .. }
    ));
    assert!(
        matches!(c.marks().as_slice(), [ExecMarkState::Escalated { .. }]),
        "{:?}",
        c.marks()
    );
    c.end(cx);
}

// ---- history (surface 11) -------------------------------------------------------------------

/// History's Insert and run goes through the same gate as any Run: a recorded `DELETE` without
/// a `WHERE` inserts, then asks (the calm confirmation off production) before anything is sent;
/// Cancel sends nothing and says so.
#[cfg(unix)]
#[gpui::test]
fn history_insert_and_run_meets_the_gate(cx: &mut TestAppContext) {
    let mut c = console(
        cx,
        profile("orders-dev", DbmsKind::Postgres, EnvTag::Dev),
        pg(),
    );
    wait_until(c.vcx, "the history store", |vcx| {
        vcx.cx.read(|cx| hacivat_app::history::store(cx).is_some())
    });
    let store = c
        .vcx
        .cx
        .read(hacivat_app::history::store)
        .expect("the history store");
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("after the epoch")
        .as_millis() as i64;
    store
        .sink()
        .try_send(Arc::new(history_record(
            c.profile,
            "DELETE FROM orders;",
            now - 60_000,
        )))
        .expect("room in the sink");
    assert!(run(c.vcx, "history.open"));
    wait_until(c.vcx, "the dialog's first answer", |vcx| {
        vcx.cx.read(|cx| {
            hacivat_app::history::dialog::shown(cx)
                .is_some_and(|s| s.loaded && !s.statements.is_empty())
        })
    });
    c.vcx.simulate_keystrokes(if cfg!(target_os = "macos") {
        "cmd-enter"
    } else {
        "ctrl-enter"
    });
    c.vcx.run_until_parked();
    assert_eq!(editor_text(c.vcx, c.id), "DELETE FROM orders;");
    c.wait_confirm();
    assert!(!c.ever_sent("DELETE"), "{:?}", c.fake.call_log());
    c.click("confirm-cancel");
    c.wait_log("exec-confirm-cancelled");
    assert!(!c.ever_sent("DELETE"), "{:?}", c.fake.call_log());
    c.end(cx);
}

/// A stored history entry of `profile` (Task 17's record) with its masked text `sql`.
fn history_record(profile: ProfileId, sql: &str, at_ms: i64) -> hacivat_exec::HistoryRecord {
    hacivat_exec::HistoryRecord {
        ts_start_ms: at_ms,
        duration_ms: 12,
        profile,
        database: Some("orders".into()),
        schema: Some("public".into()),
        console: 1,
        origin: hacivat_exec::HistoryOrigin::Console,
        statement_masked: Arc::from(sql),
        text_hash: [0; 32],
        fingerprint: 0,
        params_masked: None,
        outcome: hacivat_exec::Outcome::Ok,
        rows_affected: None,
        rows_fetched: Some(0),
        error_code: None,
        error_message: None,
        tx_mode: TxMode::Auto,
        run: None,
        redacted: false,
    }
}
