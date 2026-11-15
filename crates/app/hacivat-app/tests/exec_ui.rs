//! M3a Task 33: the run UI of a console, end to end over FakeDriver sessions: Run resolves and
//! sends exactly the units the user sees, the D-011 chooser, the gutter marks, cancel, the SQL
//! context, the transaction commands and the close and quit prompts.

mod support;

use std::sync::Arc;

use gpui::{Entity, TestAppContext, VisualTestContext};
use hacivat_app::console::{ConsoleId, Consoles};
use hacivat_app::exec_ui::chooser;
use hacivat_driver::fake::{
    FakeDriver, FakeExec, FakeQuery, FakeResultSet, FakeRows, FakeTypedCall,
};
use hacivat_driver::{ExecSummary, TxStatus};
use hacivat_proto::{DbmsKind, EnvTag, ProfileId};
use hacivat_session::session::{SessionHandle, TxOwner};
use hacivat_ui_editor::{EditorView, ExecMarkState, preview};
use hacivat_values::{LogicalType, Value};
use support::*;

/// A console over a FakeDriver session of `profile`, with its SQL context read.
struct Console {
    vcx: &'static mut VisualTestContext,
    id: ConsoleId,
    fake: Arc<FakeDriver>,
    profile: ProfileId,
    session: SessionHandle,
    editor: Entity<EditorView>,
}

fn session_over(
    vcx: &VisualTestContext,
    profile: hacivat_session::profile::Profile,
    fake: FakeDriver,
) -> (SessionHandle, Arc<FakeDriver>) {
    let rt = vcx.cx.read(|cx| {
        cx.global::<hacivat_app::connections::ConnectionsModel>()
            .core_runtime()
            .handle()
    });
    futures::executor::block_on(hacivat_session::testing::fake_session(rt, fake, profile))
}

fn profile(name: &str, engine: DbmsKind) -> hacivat_session::profile::Profile {
    hacivat_session::profile::Profile {
        engine,
        ..test_profile(name, Some(EnvTag::Dev), "127.0.0.1")
    }
}

fn console_with(
    cx: &mut TestAppContext,
    p: hacivat_session::profile::Profile,
    fake: FakeDriver,
) -> Console {
    let vcx = shell_with_connections(cx, "en");
    in_window(vcx, |window, _| window.activate_window());
    let profile = add_profile(vcx, p.clone());
    let (session, fake) = session_over(vcx, p, fake);
    let id = register_test_console(vcx, profile, session.clone());
    wait_until(vcx, "the console's SQL context", |vcx| {
        vcx.cx
            .read(|cx| Consoles::get(cx, id).is_some_and(|s| s.sql.is_some()))
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
        profile,
        session,
        editor,
    }
}

/// A PostgreSQL fake whose connections track transactions (Hacivat's `BEGIN` around a read
/// head's cursor needs it: a suspendable portal outside a transaction is refused, P5).
fn pg() -> FakeDriver {
    FakeDriver::new(DbmsKind::Postgres).tracks_transactions()
}

fn pg_console(cx: &mut TestAppContext, fake: FakeDriver) -> Console {
    console_with(cx, profile("orders-dev", DbmsKind::Postgres), fake)
}

impl Console {
    fn set_text(&self, text: &str) {
        let text = text.to_string();
        self.vcx.cx.update(|cx| {
            self.editor.update(cx, |e, cx| e.set_text(&text, cx));
        });
        self.vcx.run_until_parked();
    }

    fn select(&self, r: std::ops::Range<usize>) {
        self.vcx.cx.update(|cx| {
            self.editor.update(cx, |e, cx| e.select_range(r, cx));
        });
        self.vcx.run_until_parked();
    }

    /// Runs the editor command `id` (`editor.run`, `editor.run_3`) and lets the resolution
    /// land.
    fn command(&self, id: &str) {
        let editor = self.editor.clone();
        in_window(self.vcx, |window, cx| {
            editor.update(cx, |e, cx| assert!(e.run_command(id, window, cx)));
        });
        self.vcx.run_until_parked();
    }

    fn run_at(&self, at: usize) {
        self.select(at..at);
        self.command("editor.run");
    }

    /// The typed queries the session sent.
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

    fn wait_queries(&self, n: usize) {
        wait_until(self.vcx, "the queries to be sent", |_| {
            self.queries().len() >= n
        });
    }

    fn chooser_open(&self) -> bool {
        self.vcx.cx.read(|cx| chooser::is_open(self.id, cx))
    }

    fn chooser_selected(&self) -> Option<usize> {
        self.vcx
            .cx
            .read(|cx| chooser::menu_of(self.id, cx).and_then(|m| m.read(cx).selected()))
    }

    fn band(&self) -> Option<preview::BandPlan> {
        self.vcx
            .cx
            .read(|cx| preview::current(self.editor.read(cx)))
    }

    fn keys(&mut self, keys: &str) {
        self.vcx.simulate_keystrokes(keys);
        self.vcx.run_until_parked();
    }

    /// The marks of the latest run, in unit order.
    fn mark_states(&self) -> Vec<ExecMarkState> {
        self.vcx.cx.read(|cx| {
            hacivat_app::exec_ui::last_run_marks(self.id, cx)
                .into_iter()
                .flatten()
                .collect()
        })
    }

    fn focus_editor(&self) {
        let editor = self.editor.clone();
        in_window(self.vcx, |window, cx| {
            let handle = gpui::Focusable::focus_handle(editor.read(cx), cx);
            window.focus(&handle, cx);
        });
        self.vcx.run_until_parked();
    }

    fn wait_marks(&self, what: &str, ok: impl Fn(&[ExecMarkState]) -> bool) {
        wait_until(self.vcx, what, |_| ok(&self.mark_states()));
    }

    fn tx(&self) -> hacivat_session::session::TxState {
        self.session.state().borrow().tx.clone()
    }

    fn log_ids(&self) -> Vec<&'static str> {
        self.vcx.cx.read(|cx| {
            Consoles::get(cx, self.id)
                .map(|s| s.log.lines().iter().map(|l| l.msg.fluent_id()).collect())
                .unwrap_or_default()
        })
    }
}

fn succeeded(s: &ExecMarkState) -> bool {
    matches!(s, ExecMarkState::Succeeded { .. })
}

fn rows(n: u64) -> FakeResultSet {
    FakeResultSet {
        columns: vec![(
            "n",
            LogicalType::Int {
                bits: 64,
                signed: true,
            },
        )],
        rows: FakeRows::Generated {
            count: n,
            cell: |r, _| Value::Int(r as i64),
        },
    }
}

// ---- Run ---------------------------------------------------------------------------------

#[gpui::test]
fn one_candidate_runs_without_a_chooser(cx: &mut TestAppContext) {
    let c = pg_console(cx, pg());
    c.set_text("SELECT 1");
    c.run_at(3);
    c.wait_queries(1);
    assert_eq!(c.queries(), ["SELECT 1"]);
    assert!(!c.chooser_open());
    c.wait_marks("the mark to succeed", |m| m.len() == 1 && succeeded(&m[0]));
    stop_core_runtime_test(cx);
}

#[gpui::test]
fn selection_of_two_statements_runs_two_units(cx: &mut TestAppContext) {
    let c = pg_console(cx, pg());
    let text = "SELECT 1;\nSELECT 2;\n";
    c.set_text(text);
    c.select(0..text.len());
    c.command("editor.run");
    c.wait_queries(2);
    assert_eq!(
        c.queries(),
        ["SELECT 1", "SELECT 2"],
        "each unit's strict text"
    );
    c.wait_marks("two marks to succeed", |m| {
        m.len() == 2 && m.iter().all(succeeded)
    });
    stop_core_runtime_test(cx);
}

const TWO: &str = "SELECT 1;\nSELECT 2;\n";

#[gpui::test]
fn ask_opens_the_chooser_hover_previews_and_enter_runs_the_preselected(cx: &mut TestAppContext) {
    let mut c = pg_console(cx, pg());
    c.set_text(TWO);
    c.run_at(0);
    assert!(c.chooser_open(), "two candidates: the chooser opens");
    assert!(c.queries().is_empty(), "nothing is sent before a choice");
    // The highlight starts on the preselected candidate (Statement), previewed in the band.
    assert_eq!(c.chooser_selected(), Some(0));
    let band = c.band().expect("the band shows the highlighted candidate");
    assert!(band.from_override);
    assert_eq!(band.units.to_vec(), vec![0..8_usize]);
    // ↓ highlights the whole script; the band follows.
    c.keys("down");
    assert_eq!(c.chooser_selected(), Some(1));
    assert_eq!(c.band().expect("a band").units.as_slice(), [0..8, 10..18]);
    // ↑ back; Enter runs exactly the highlighted row.
    c.keys("up");
    assert_eq!(c.band().expect("a band").units.to_vec(), vec![0..8_usize]);
    c.keys("enter");
    assert!(!c.chooser_open());
    c.wait_queries(1);
    assert_eq!(c.queries(), ["SELECT 1"]);
    // Closing the chooser gives the band back to the service.
    assert!(c.band().is_none_or(|b| !b.from_override));
    stop_core_runtime_test(cx);
}

#[gpui::test]
fn escape_closes_the_chooser_and_sends_nothing(cx: &mut TestAppContext) {
    let mut c = pg_console(cx, pg());
    c.set_text(TWO);
    c.run_at(0);
    assert!(c.chooser_open());
    c.keys("escape");
    assert!(!c.chooser_open());
    settle(c.vcx, std::time::Duration::from_millis(50));
    assert!(c.queries().is_empty());
    assert!(c.band().is_none_or(|b| !b.from_override));
    stop_core_runtime_test(cx);
}

/// Enter runs the highlighted row only on a plain key-down: the ⌘↵ that asked for Run (still
/// held, or pressed again) and a modified Enter never choose a candidate.
#[gpui::test]
fn a_modified_enter_never_chooses(cx: &mut TestAppContext) {
    let mut c = pg_console(cx, pg());
    c.set_text(TWO);
    c.run_at(0);
    assert!(c.chooser_open());
    c.keys("cmd-enter");
    c.keys("shift-enter");
    c.keys("ctrl-enter");
    settle(c.vcx, std::time::Duration::from_millis(50));
    assert!(c.queries().is_empty(), "{:?}", c.queries());
    assert!(c.chooser_open());
    c.keys("enter");
    c.wait_queries(1);
    assert_eq!(c.queries(), ["SELECT 1"]);
    stop_core_runtime_test(cx);
}

#[gpui::test]
fn chooser_remembers_the_kind_per_console(cx: &mut TestAppContext) {
    let mut c = pg_console(cx, pg());
    c.set_text(TWO);
    c.run_at(0);
    c.keys("down enter");
    c.wait_queries(2);
    assert_eq!(
        c.queries(),
        ["SELECT 1", "SELECT 2"],
        "the whole script ran"
    );
    c.wait_marks("the run to end", |m| {
        m.len() == 2 && m.iter().all(succeeded)
    });
    // The next Run preselects the remembered kind.
    c.run_at(0);
    assert!(c.chooser_open());
    assert_eq!(c.chooser_selected(), Some(1), "Whole script, remembered");
    c.keys("escape");
    // Another console of the app remembers nothing.
    let profile2 = profile("orders-dev-2", DbmsKind::Postgres);
    let id2 = add_profile(c.vcx, profile2.clone());
    let (session2, _fake2) = session_over(c.vcx, profile2, pg());
    let other = register_test_console(c.vcx, id2, session2);
    wait_until(c.vcx, "the second console's context", |vcx| {
        vcx.cx
            .read(|cx| Consoles::get(cx, other).is_some_and(|s| s.sql.is_some()))
    });
    let editor2 = c.vcx.cx.read(|cx| {
        Consoles::get(cx, other)
            .and_then(|s| s.editor.clone())
            .expect("an editor")
    });
    c.vcx
        .cx
        .update(|cx| editor2.update(cx, |e, cx| e.set_text(TWO, cx)));
    c.vcx.run_until_parked();
    in_window(c.vcx, |window, cx| {
        editor2.update(cx, |e, cx| {
            e.select_range(0..0, cx);
            assert!(e.run_command("editor.run", window, cx));
        });
    });
    c.vcx.run_until_parked();
    let selected = c
        .vcx
        .cx
        .read(|cx| chooser::menu_of(other, cx).and_then(|m| m.read(cx).selected()));
    assert_eq!(selected, Some(0), "the other console preselects Statement");
    stop_core_runtime_test(cx);
}

#[gpui::test]
fn chooser_assigns_no_access_keys(cx: &mut TestAppContext) {
    let mut c = pg_console(cx, pg());
    c.set_text(TWO);
    c.run_at(0);
    assert!(c.chooser_open());
    // "Whole script" and "Statement" start with these letters; on Windows and Linux a menu
    // would choose by them. The labels are SQL text: typing never runs a statement (P15).
    c.keys("w s 1");
    settle(c.vcx, std::time::Duration::from_millis(50));
    assert!(c.chooser_open(), "letters choose nothing");
    assert!(c.queries().is_empty());
    stop_core_runtime_test(cx);
}

#[gpui::test]
fn ambiguous_resolution_sends_nothing(cx: &mut TestAppContext) {
    let mut c = console_with(
        cx,
        profile("shop-dev", DbmsKind::MySql),
        FakeDriver::new(DbmsKind::MySql),
    );
    c.set_text("SELECT 1 /*! ; */ ;");
    c.run_at(0);
    settle(c.vcx, std::time::Duration::from_millis(50));
    assert!(c.fake.typed_log().is_empty(), "nothing reached the session");
    assert!(c.chooser_open(), "the chooser shows its uncertain state");
    assert_eq!(c.chooser_selected(), None, "with no run item at all");
    enable_registry(c.vcx);
    in_window(c.vcx, |window, _| window.refresh());
    c.vcx.run_until_parked();
    let drawn: Vec<String> = registry_frame(c.vcx)
        .into_iter()
        .map(|r| r.text.to_string())
        .collect();
    assert!(drawn.iter().any(|t| t == "Boundary uncertain"), "{drawn:?}");
    assert!(
        drawn
            .iter()
            .any(|t| t == "A ; sits inside an executable comment (/*! … */)."),
        "{drawn:?}"
    );
    // Enter has nothing to run.
    c.keys("enter");
    settle(c.vcx, std::time::Duration::from_millis(50));
    assert!(c.fake.typed_log().is_empty());
    stop_core_runtime_test(cx);
}

#[gpui::test]
fn read_only_denial_marks_the_gutter_failed_with_its_reason(cx: &mut TestAppContext) {
    let mut p = profile("orders-ro", DbmsKind::Postgres);
    p.read_only = true;
    // The SQL context's probe runs inside the read-only guard: its canary finds no
    // transaction ID.
    let canary = FakeExec {
        prefix: "SELECT pg_catalog.txid_current_if_assigned()",
        result: Ok(ExecSummary {
            rows: vec![vec![None]],
            row_count: 1,
            ..ExecSummary::default()
        }),
        delay: std::time::Duration::ZERO,
        tx_after: None,
        panics: false,
    };
    let c = console_with(cx, p, pg().exec(vec![canary]));
    c.set_text("BEGIN");
    c.run_at(0);
    c.wait_marks("the denial", |m| {
        matches!(m, [ExecMarkState::Failed { sqlstate: None }])
    });
    assert!(c.queries().is_empty(), "nothing was sent");
    enable_registry(c.vcx);
    in_window(c.vcx, |window, _| window.refresh());
    c.vcx.run_until_parked();
    let drawn: Vec<String> = registry_frame(c.vcx)
        .into_iter()
        .map(|r| r.text.to_string())
        .collect();
    assert!(
        drawn.iter().any(|t| t == "not sent: read-only"),
        "{drawn:?}"
    );
    assert!(
        c.log_ids()
            .iter()
            .any(|id| id.starts_with("exec-deny-read-only"))
    );
    stop_core_runtime_test(cx);
}

#[gpui::test]
fn cancel_reaches_the_session_and_stop_fetching_keeps_rows(cx: &mut TestAppContext) {
    let slow = FakeQuery {
        prefix: "SELECT n",
        sets: vec![rows(1_000_000)],
        chunk_delay: std::time::Duration::from_millis(20),
        ..Default::default()
    };
    let mut c = pg_console(cx, pg().tracks_transactions().queries(vec![slow]));
    c.set_text("SELECT n FROM generate_series(1, 1000000) n FOR UPDATE");
    c.run_at(0);
    c.wait_queries(1);
    let result_rows = |c: &Console| {
        c.vcx.cx.read(|cx| {
            Consoles::get(cx, c.id)
                .and_then(|s| s.results.clone())
                .and_then(|r| {
                    r.read(cx)
                        .tabs()
                        .first()
                        .map(|t| t.source().snapshot().row_count())
                })
                .unwrap_or(0)
        })
    };
    wait_until(c.vcx, "rows to stream in", |_| result_rows(&c) > 0);
    // Escape never cancels (R-M3-38): the run keeps streaming.
    c.focus_editor();
    c.keys("escape");
    let before = result_rows(&c);
    wait_until(c.vcx, "more rows after Escape", |_| {
        result_rows(&c) > before
    });
    assert!(matches!(
        c.mark_states().as_slice(),
        [ExecMarkState::Running { .. }]
    ));
    assert!(
        run(c.vcx, "exec.cancel"),
        "exec.cancel runs while a run streams"
    );
    wait_until(c.vcx, "the run to end", |_| {
        let m = c.mark_states();
        m.len() == 1
            && !matches!(
                m[0],
                ExecMarkState::Running { .. }
                    | ExecMarkState::Cancelling { .. }
                    | ExecMarkState::Queued
            )
    });
    let kept = result_rows(&c);
    assert!(kept > 0, "stopping keeps the rows fetched");
    assert!(kept < 1_000_000, "and stops fetching");
    stop_core_runtime_test(cx);
}

#[gpui::test]
fn the_ui_thread_awaits_only_tokio_sync_receivers(cx: &mut TestAppContext) {
    let gated = FakeQuery {
        prefix: "SELECT 42",
        first_delay: std::time::Duration::from_millis(300),
        ..Default::default()
    };
    let c = pg_console(cx, pg().queries(vec![gated]));
    c.set_text("SELECT 42");
    c.run_at(0);
    c.wait_queries(1);
    // The statement is still running on the core runtime: the UI thread is free (it parked
    // without waiting for the reply), it takes edits, and the mark says Running.
    let started = std::time::Instant::now();
    c.vcx.run_until_parked();
    assert!(started.elapsed() < std::time::Duration::from_millis(250));
    assert!(matches!(
        c.mark_states().as_slice(),
        [ExecMarkState::Running { .. } | ExecMarkState::Queued]
    ));
    c.vcx.cx.update(|cx| {
        c.editor.update(cx, |e, cx| {
            e.select_range(9..9, cx);
        });
    });
    c.wait_marks("the statement to finish", |m| {
        m.len() == 1 && succeeded(&m[0])
    });
    stop_core_runtime_test(cx);
}

#[gpui::test]
fn set_statement_refreshes_the_sql_context(cx: &mut TestAppContext) {
    let probe = |answer: &str| FakeExec {
        prefix: "SHOW standard_conforming_strings",
        result: Ok(ExecSummary {
            columns: vec!["standard_conforming_strings".into()],
            rows: vec![vec![Some(answer.to_string())]],
            row_count: 1,
            ..Default::default()
        }),
        delay: std::time::Duration::ZERO,
        tx_after: None,
        panics: false,
    };
    let c = pg_console(cx, pg().exec(vec![probe("on"), probe("off")]));
    let scs = |c: &Console| {
        c.vcx.cx.read(|cx| {
            Consoles::get(cx, c.id)
                .and_then(|s| s.sql)
                .map(|ctx| ctx.modes.standard_conforming_strings)
        })
    };
    assert_eq!(scs(&c), Some(true), "probed at attach");
    c.set_text("SET standard_conforming_strings = off");
    c.run_at(0);
    c.wait_queries(1);
    wait_until(c.vcx, "the context to be probed again", |_| {
        scs(&c) == Some(false)
    });
    let probes = c
        .fake
        .executed()
        .iter()
        .filter(|s| s.starts_with("SHOW standard_conforming_strings"))
        .count();
    assert_eq!(probes, 2);
    stop_core_runtime_test(cx);
}

#[gpui::test]
fn disconnected_console_run_starts_the_connect(cx: &mut TestAppContext) {
    let c = pg_console(cx, pg());
    c.set_text("SELECT 1");
    let _ = futures::executor::block_on(
        c.session
            .close(hacivat_session::session::ClosePolicy::RollBack),
    );
    wait_until(c.vcx, "the session to close", |_| {
        !hacivat_exec::session_live(&c.session.state().borrow().state)
    });
    let before = c.vcx.cx.read(|cx| {
        cx.global::<hacivat_app::connections::ConnectionsModel>()
            .live(c.profile)
    });
    assert!(matches!(before, hacivat_session::LiveState::Disconnected));
    c.run_at(0);
    assert!(c.fake.typed_log().is_empty(), "nothing runs");
    let after = c.vcx.cx.read(|cx| {
        cx.global::<hacivat_app::connections::ConnectionsModel>()
            .live(c.profile)
    });
    assert!(
        !matches!(after, hacivat_session::LiveState::Disconnected),
        "the profile's connect started: {after:?}"
    );
    stop_core_runtime_test(cx);
}

// ---- transactions and the prompts --------------------------------------------------------

/// A PostgreSQL console whose connection tracks transactions, with a user `BEGIN` run and one
/// statement inside it.
fn console_in_transaction(cx: &mut TestAppContext, fail_inside: bool) -> Console {
    // A typed statement's effect on the transaction is scripted (`tx_after`); Hacivat's own
    // control statements are tracked.
    let mut queries = vec![FakeQuery {
        prefix: "BEGIN",
        tx_after: Some(TxStatus::InTx),
        ..Default::default()
    }];
    if fail_inside {
        queries.push(FakeQuery {
            prefix: "INSERT",
            sets: vec![rows(10)],
            fail_after: Some((
                0,
                hacivat_driver::RawError {
                    kind: hacivat_driver::RawErrorKind::Db,
                    server: None,
                },
            )),
            ..Default::default()
        });
    }
    let c = pg_console(cx, pg().queries(queries));
    c.set_text("BEGIN;\nINSERT INTO t VALUES (1);\n");
    c.select(
        0..c.vcx
            .cx
            .read(|cx| c.editor.read(cx).snapshot().buffer.len_bytes()),
    );
    c.command("editor.run");
    let want = if fail_inside {
        TxStatus::Failed
    } else {
        TxStatus::InTx
    };
    wait_until(c.vcx, "the user's transaction", |_| {
        let tx = c.tx();
        tx.owner == TxOwner::User && tx.status == want
    });
    c
}

fn close_console_tab(c: &Console) {
    assert!(run(c.vcx, "workspace.close_tab"));
    c.vcx.run_until_parked();
}

fn console_tab_open(c: &Console) -> bool {
    c.vcx.cx.read(|cx| Consoles::get(cx, c.id).is_some())
}

fn dialog_buttons(vcx: &VisualTestContext) -> Vec<String> {
    texts_drawn_by(vcx, "hacivat-ui/src/button.rs")
}

#[gpui::test]
fn closing_a_console_with_an_open_transaction_asks(cx: &mut TestAppContext) {
    let mut c = console_in_transaction(cx, false);
    enable_registry(c.vcx);
    let tabs = tab_count(c.vcx);
    close_console_tab(&c);
    assert!(layer_ids(c.vcx).contains(&"dialog"), "the prompt opens");
    assert_eq!(tab_count(c.vcx), tabs, "nothing closed yet");
    assert_eq!(
        dialog_title(c.vcx),
        "orders-dev has an open transaction with 1 statement"
    );
    // Cancel has the focus; Enter (key-down) answers it: the console stays.
    c.keys("enter");
    assert!(!layer_ids(c.vcx).contains(&"dialog"));
    assert_eq!(tab_count(c.vcx), tabs);
    assert!(console_tab_open(&c));
    assert_eq!(
        c.tx().status,
        TxStatus::InTx,
        "the transaction is untouched"
    );
    // Again; Roll back and close.
    close_console_tab(&c);
    assert!(layer_ids(c.vcx).contains(&"dialog"));
    c.keys("shift-tab shift-tab");
    c.keys("tab");
    c.keys("enter");
    wait_until(c.vcx, "the console to close", |vcx| {
        tab_count(vcx) == tabs - 1
    });
    assert!(
        c.fake.executed().iter().any(|s| s.trim() == "ROLLBACK"),
        "{:?}",
        c.fake.executed()
    );
    assert!(!console_tab_open(&c));
    stop_core_runtime_test(cx);
}

#[gpui::test]
fn quit_with_an_open_transaction_asks_and_cancel_keeps_the_app(cx: &mut TestAppContext) {
    let mut c = console_in_transaction(cx, false);
    enable_registry(c.vcx);
    assert!(run(c.vcx, "app.quit"));
    wait_until(c.vcx, "the quit prompt", |vcx| {
        layer_ids(vcx).contains(&"dialog")
    });
    assert_eq!(c.vcx.cx.read(hacivat_app::commands::quit_requests), 0);
    let buttons = dialog_buttons(c.vcx);
    assert!(
        buttons.iter().any(|b| b == "Commit and quit"),
        "{buttons:?}"
    );
    // Closing the window answers Cancel and keeps the app.
    assert!(!c.vcx.simulate_close());
    c.vcx.run_until_parked();
    assert!(!layer_ids(c.vcx).contains(&"dialog"));
    assert_eq!(c.vcx.cx.read(hacivat_app::commands::quit_requests), 0);
    assert_eq!(c.tx().status, TxStatus::InTx);
    // Quit again; Commit and quit.
    assert!(run(c.vcx, "app.quit"));
    wait_until(c.vcx, "the quit prompt", |vcx| {
        layer_ids(vcx).contains(&"dialog")
    });
    c.keys("shift-tab shift-tab");
    c.keys("enter");
    wait_until(c.vcx, "the app to quit", |vcx| {
        vcx.cx.read(hacivat_app::commands::quit_requests) == 1
    });
    assert!(
        c.fake.executed().iter().any(|s| s.trim() == "COMMIT"),
        "{:?}",
        c.fake.executed()
    );
    stop_core_runtime_test(cx);
}

#[gpui::test]
fn close_prompt_on_a_failed_transaction_offers_roll_back_only(cx: &mut TestAppContext) {
    let c = console_in_transaction(cx, true);
    enable_registry(c.vcx);
    close_console_tab(&c);
    assert!(layer_ids(c.vcx).contains(&"dialog"));
    assert_eq!(dialog_title(c.vcx), "The transaction on orders-dev failed");
    let buttons = dialog_buttons(c.vcx);
    assert!(
        !buttons.iter().any(|b| b.starts_with("Commit")),
        "{buttons:?}"
    );
    assert!(
        buttons.iter().any(|b| b == "Roll back and close"),
        "{buttons:?}"
    );
    // The commit command is not offered either.
    assert!(!run(c.vcx, "tx.commit") || c.tx().status == TxStatus::Failed);
    stop_core_runtime_test(cx);
}

#[gpui::test]
fn closing_a_console_releases_its_cursor(cx: &mut TestAppContext) {
    let big = FakeQuery {
        prefix: "SELECT n",
        sets: vec![rows(100_000)],
        ..Default::default()
    };
    let c = pg_console(cx, pg().tracks_transactions().queries(vec![big]));
    c.set_text("SELECT n FROM big");
    c.run_at(0);
    wait_until(c.vcx, "the cursor to stay open", |_| {
        c.session.state().borrow().cursor.is_some()
    });
    let tabs = tab_count(c.vcx);
    close_console_tab(&c);
    assert_eq!(
        tab_count(c.vcx),
        tabs - 1,
        "no user transaction: the console closes"
    );
    wait_until(c.vcx, "the cursor to be released", |_| {
        c.session.state().borrow().cursor.is_none()
    });
    assert!(
        c.fake
            .typed_log()
            .iter()
            .any(|call| matches!(call, FakeTypedCall::Close(_))),
        "{:?}",
        c.fake.typed_log()
    );
    stop_core_runtime_test(cx);
}

#[gpui::test]
fn tx_commands_end_the_users_transaction_and_log_it(cx: &mut TestAppContext) {
    let c = console_in_transaction(cx, false);
    assert!(run(c.vcx, "tx.commit"));
    wait_until(c.vcx, "the commit", |_| c.tx().status == TxStatus::Idle);
    wait_until(c.vcx, "the log line", |_| {
        c.log_ids().contains(&"exec-tx-committed")
    });
    stop_core_runtime_test(cx);
}

/// R-M3-16 amended (P16): stopping a PostgreSQL unit that cannot stay suspended, inside the
/// user's transaction, when its next chunk is more than 1 s late, cancels it on the server and
/// fails the transaction, as psql does. The UI says so; the close prompt then offers only Roll
/// back.
#[gpui::test]
fn a_stop_that_fails_the_users_transaction_says_so(cx: &mut TestAppContext) {
    let begin = FakeQuery {
        prefix: "BEGIN",
        tx_after: Some(TxStatus::InTx),
        ..Default::default()
    };
    let slow = FakeQuery {
        prefix: "UPDATE",
        sets: vec![rows(100_000)],
        chunk_delay: std::time::Duration::from_millis(1_500),
        ..Default::default()
    };
    let c = pg_console(cx, pg().queries(vec![begin, slow]));
    enable_registry(c.vcx);
    c.set_text("BEGIN");
    c.run_at(0);
    wait_until(c.vcx, "the user's transaction", |_| {
        c.tx().owner == TxOwner::User && c.tx().status == TxStatus::InTx
    });
    c.set_text("UPDATE t SET a = a + 1 RETURNING a");
    c.run_at(0);
    c.wait_marks("the update to run", |m| {
        matches!(m, [ExecMarkState::Running { .. }])
    });
    assert!(run(c.vcx, "exec.cancel"));
    assert!(
        matches!(
            c.mark_states().as_slice(),
            [ExecMarkState::Cancelling { .. }]
        ),
        "cancelling… at once: {:?}",
        c.mark_states()
    );
    wait_until(c.vcx, "the transaction to fail", |_| {
        c.tx().status == TxStatus::Failed
    });
    wait_until(c.vcx, "the toast", |vcx| {
        in_window(vcx, |window, _| window.refresh());
        toast_texts(vcx).iter().any(|t| {
            t == "Stopping the statement failed the transaction on orders-dev. Only a rollback can end it now."
        })
    });
    // Commit is not offered on a failed transaction.
    assert!(!run(c.vcx, "tx.commit"));
    assert!(run(c.vcx, "tx.rollback"));
    wait_until(c.vcx, "the rollback", |_| c.tx().status == TxStatus::Idle);
    stop_core_runtime_test(cx);
}

// ---- the results tile's requests ---------------------------------------------------------

#[gpui::test]
fn go_to_source_selects_the_range(cx: &mut TestAppContext) {
    let c = pg_console(cx, pg());
    c.set_text("SELECT 1;\nSELECT oops;\n");
    in_window(c.vcx, |window, cx| {
        hacivat_app::exec_ui::handle_results_event(
            &hacivat_app::results::ResultsEvent::GoToSource {
                console: c.id,
                range: 10..21,
            },
            window,
            cx,
        );
    });
    c.vcx.run_until_parked();
    let sel = c
        .vcx
        .cx
        .read(|cx| c.editor.read(cx).snapshot().selection.main().range());
    assert_eq!(sel, 10..21);
    stop_core_runtime_test(cx);
}

// ---- memory ------------------------------------------------------------------------------

#[test]
fn physical_memory_is_known_on_this_platform() {
    assert!(hacivat_ui_core::platform::physical_memory_bytes().unwrap_or(0) > 1 << 30);
}

// ---- preview and Run agree (R-M3-8) -------------------------------------------------------

/// Fragments of scripts that stress the boundaries: comments, strings, dollar bodies, psql
/// commands, `COPY` data, unknown blocks, unterminated constructs, MySQL executable comments
/// and delimiters.
const FRAGMENTS: &[&str] = &[
    "SELECT 1",
    "SELECT 2 FROM t",
    ";",
    ";\n",
    "\n",
    " ",
    "-- c\n",
    "/* x; */",
    "'a;b'",
    "$$ x; $$",
    "BEGIN",
    "COMMIT",
    "COPY t FROM STDIN;\n1\n\\.\n",
    "CREATE FUNCTION f() RETURNS int AS $$ SELECT 1; $$ LANGUAGE sql",
    "CREATE FUNCTION g() RETURNS int BEGIN ATOMIC SELECT 1; END",
    "\\set x 1\n",
    "'open",
    "/* open",
    "DO $b$ BEGIN NULL; END $b$",
    "/*! ; */",
    "DELIMITER //\n",
    "SELECT 3 //",
    "(SELECT 4)",
    "WITH a AS (SELECT 1) SELECT * FROM a",
];

/// A small deterministic generator (xorshift).
struct Gen(u64);

impl Gen {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

fn doc_of(text: &str, id: sql_dialect::DialectId) -> sql_split::DocSnapshot {
    let ctx = sql_dialect::DialectCtx::new(id, None);
    let flavor = hacivat_sql_editor::effective_flavor(&ctx, sql_dialect::ClientFlavor::None);
    sql_split::SqlDocument::new(text.to_string(), ctx, flavor).snapshot()
}

/// Everything the property asks at one place: every action, with and without a remembered
/// chooser kind. Returns how often the raw answers of the two implementations disagreed (a
/// count, for the report) and checks that Run's gated answer always agrees with the band.
fn check_place(
    doc: &sql_split::DocSnapshot,
    at: &hacivat_ui_editor::ExecAt,
    raw_divergences: &mut usize,
) {
    use hacivat_ui_editor::ExecAction;
    use sql_split::exec_range::CandidateKind;
    for action in [
        ExecAction::Execute1,
        ExecAction::Execute2,
        ExecAction::Execute3,
    ] {
        for remembered in [
            None,
            Some(CandidateKind::WholeScript),
            Some(CandidateKind::FromCaret),
        ] {
            let band = hacivat_sql_editor::preview_of(doc, at, action, true, remembered);
            // The same conversion Run uses (`exec_points`), without the oracle.
            let points = hacivat_sql_editor::exec_points(at);
            let action_of = match action {
                ExecAction::Execute1 => hacivat_exec::RunAction::Execute1,
                ExecAction::Execute2 => hacivat_exec::RunAction::Execute2,
                ExecAction::Execute3 => hacivat_exec::RunAction::Execute3,
            };
            let raw = hacivat_exec::resolve_units_with(
                doc,
                &points,
                action_of,
                remembered,
                &hacivat_app::exec_ui::NoOracle,
            );
            if !hacivat_app::exec_ui::agrees(&band, &raw) {
                // The one known divergence: a chooser whose preselected candidate Run refuses
                // (a remembered whole script holding `COPY … FROM STDIN`); the band says
                // uncertain, and Run refuses with it.
                assert!(
                    band.state == hacivat_ui_editor::PreviewState::Uncertain
                        && matches!(raw, hacivat_exec::Resolution::Ask { .. }),
                    "an unexpected divergence at {at:?} {action:?} {remembered:?} in {:?}:
 band {band:?}
 raw {raw:?}",
                    doc.text()
                );
                *raw_divergences += 1;
            }
            let run = hacivat_app::exec_ui::resolve_for_run(doc, at, action, remembered);
            assert!(
                hacivat_app::exec_ui::agrees(&band, &run),
                "Run and its preview disagree at {at:?} {action:?} {remembered:?} in {:?}:\n band {band:?}\n run {run:?}",
                doc.text()
            );
            // Whatever Run sends is the document's text at the unit's range, byte for byte,
            // and a range the band draws as runnable.
            // "N choices": the band counts exactly the rows Run's chooser offers.
            if let hacivat_exec::Resolution::Ask { candidates, .. } = &run {
                assert_eq!(band.choices as usize, candidates.len(), "{:?}", doc.text());
            } else {
                assert_eq!(band.choices, 0);
            }
            if let hacivat_exec::Resolution::Units(units) = &run {
                for u in units {
                    assert_eq!(&doc.text()[u.doc_range.clone()], &*u.text);
                    assert!(band.ranges.contains(&u.doc_range));
                }
            }
        }
    }
}

/// `preview_of == resolve_units` (Task 24 M5): over generated PostgreSQL and MySQL scripts, at
/// every character boundary and over random selections, Run's answer (gated by the band) and
/// the band agree exactly: the same units, the same preselected candidate, nothing where the
/// band says nothing, and nothing sent where the band says uncertain.
#[test]
fn preview_and_run_agree_on_generated_scripts() {
    use hacivat_ui_editor::ExecAt;
    let mut g = Gen(0x5eed_7a5c_0033_0001);
    let mut divergences = 0usize;
    let mut places = 0usize;
    for round in 0..160 {
        let id = if round % 4 == 3 {
            sql_dialect::DialectId::MySql
        } else {
            sql_dialect::DialectId::Postgres
        };
        let n = 1 + g.below(6);
        let text: String = (0..n)
            .map(|_| FRAGMENTS[g.below(FRAGMENTS.len())])
            .collect();
        let doc = doc_of(&text, id);
        for at in (0..=text.len()).filter(|i| text.is_char_boundary(*i)) {
            check_place(
                &doc,
                &ExecAt::Carets(smallvec::smallvec![at]),
                &mut divergences,
            );
            places += 1;
        }
        for _ in 0..6 {
            let mut a = g.below(text.len() + 1);
            let mut b = g.below(text.len() + 1);
            while !text.is_char_boundary(a) {
                a -= 1;
            }
            while !text.is_char_boundary(b) {
                b -= 1;
            }
            let r = a.min(b)..a.max(b);
            check_place(
                &doc,
                &ExecAt::Selections(smallvec::smallvec![r]),
                &mut divergences,
            );
            places += 1;
        }
    }
    assert!(places > 5_000, "{places} places");
    // The raw answers disagree only where Run refuses (the gate above); report how often.
    eprintln!("preview_and_run_agree: {places} places, {divergences} raw divergences refused");
}
