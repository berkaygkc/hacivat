//! M3a Task 33: the run UI of a console, end to end over FakeDriver sessions: Run resolves and
//! sends exactly the units the user sees, the D-011 chooser, the gutter marks, cancel, the SQL
//! context, the transaction commands and the close and quit prompts.

mod support;

use std::sync::Arc;

use gpui::{Entity, TestAppContext, VisualTestContext};
use hacivat_app::console::{ConsoleId, Consoles};
use hacivat_app::exec_ui::chooser;
use hacivat_app::exec_ui::context::SqlContext;
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
    let side = FakeDriver::new(p.engine);
    console_with_side(cx, p, fake, side).0
}

/// [`console_with`] whose session's side connections (cancel, kill, terminate) go to `side`.
fn console_with_side(
    cx: &mut TestAppContext,
    p: hacivat_session::profile::Profile,
    fake: FakeDriver,
    side: FakeDriver,
) -> (Console, Arc<FakeDriver>) {
    let vcx = shell_with_connections(cx, "en");
    in_window(vcx, |window, _| window.activate_window());
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
    let console = Console {
        vcx,
        id,
        fake,
        profile,
        session,
        editor,
    };
    (console, side)
}

/// A PostgreSQL fake whose connections track transactions (Hacivat's `BEGIN` around a read
/// head's cursor needs it: a suspendable portal outside a transaction is refused, P5).
fn pg() -> FakeDriver {
    FakeDriver::new(DbmsKind::Postgres)
        .tracks_transactions()
        .exec(probes("SHOW standard_conforming_strings", "on"))
}

/// The SQL-context probe's answers on a read-write session (`ExecOpts::values`), enough for
/// the attach and a few reprobes.
fn probes(prefix: &'static str, value: &str) -> Vec<FakeExec> {
    (0..8)
        .map(|_| FakeExec {
            prefix,
            result: Ok(ExecSummary {
                rows: vec![vec![Some(value.to_string())]],
                row_count: 1,
                ..ExecSummary::default()
            }),
            delay: std::time::Duration::ZERO,
            tx_after: None,
            panics: false,
        })
        .collect()
}

/// A MySQL fake whose `sql_mode` is `sql_mode` (read-write sessions).
fn mysql(sql_mode: &str) -> FakeDriver {
    FakeDriver::new(DbmsKind::MySql)
        .tracks_transactions()
        .exec(probes("SELECT @@SESSION.sql_mode", sql_mode))
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

    /// Everything Run had sent has reached the session: a sentinel internal statement, sent
    /// now, is answered only after every request queued before it (Fix round 1, I6: a barrier,
    /// not a sleep).
    fn barrier(&self) {
        self.vcx.run_until_parked();
        let ticket = self
            .session
            .submit(hacivat_session::session::Request::Exec {
                sql: "SELECT 'barrier'".to_string(),
                opts: hacivat_driver::ExecOpts::default(),
                origin: hacivat_session::session::ExecOrigin::Internal { idempotent: true },
            });
        let _ = futures::executor::block_on(ticket.reply);
        self.vcx.run_until_parked();
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
    c.barrier();
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
    c.barrier();
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
    c.barrier();
    assert!(c.chooser_open(), "letters choose nothing");
    assert!(c.queries().is_empty());
    stop_core_runtime_test(cx);
}

#[gpui::test]
fn ambiguous_resolution_sends_nothing(cx: &mut TestAppContext) {
    let mut c = console_with(
        cx,
        profile("shop-dev", DbmsKind::MySql),
        mysql("STRICT_TRANS_TABLES"),
    );
    c.set_text("SELECT 1 /*! ; */ ;");
    c.run_at(0);
    c.barrier();
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
    c.barrier();
    assert!(c.fake.typed_log().is_empty());
    stop_core_runtime_test(cx);
}

#[gpui::test]
fn read_only_denial_marks_the_gutter_failed_with_its_reason(cx: &mut TestAppContext) {
    let mut p = profile("orders-ro", DbmsKind::Postgres);
    p.read_only = true;
    // ## m3a-f1-followups: the SQL-context probe is an internal statement on a read-only
    // session too, outside the guard (F1-c); `pg()` answers it.
    let c = console_with(cx, p, pg());
    c.set_text("BEGIN");
    c.run_at(0);
    c.wait_marks("the denial", |m| {
        matches!(m, [ExecMarkState::Failed { sqlstate: None }])
    });
    // The denied BEGIN was never sent.
    assert!(
        c.queries().is_empty(),
        "nothing was sent: {:?}",
        c.queries()
    );
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
    // The chord, through the keymap and the Console context (Fix round 1, Minor 15).
    c.keys(CANCEL_CHORD);
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
    // The statement's first answer is a minute away: only a cancel ends it.
    let held = FakeQuery {
        prefix: "SELECT 42",
        first_delay: std::time::Duration::from_secs(60),
        ..Default::default()
    };
    let c = pg_console(cx, pg().queries(vec![held]));
    c.set_text("SELECT 42");
    c.run_at(0);
    c.wait_queries(1);
    c.wait_marks("the statement to run", |m| {
        matches!(m, [ExecMarkState::Running { .. }])
    });
    // The reply is far off on the core runtime, and the UI thread is free (Fix round 1, I6:
    // observed, not timed): an edit commits, and the mark still says Running.
    let before = c.vcx.cx.read(|cx| c.editor.read(cx).snapshot().version);
    c.select(9..9);
    c.focus_editor();
    c.vcx.simulate_input(" -- still running");
    c.vcx.run_until_parked();
    let after = c.vcx.cx.read(|cx| c.editor.read(cx).snapshot().version);
    assert!(after > before, "the edit committed while the statement ran");
    assert!(matches!(
        c.mark_states().as_slice(),
        [ExecMarkState::Running { .. }]
    ));
    assert!(run(c.vcx, "exec.cancel"));
    c.wait_marks("the cancel", |m| matches!(m, [ExecMarkState::Cancelled]));
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
    button_labels(vcx)
}

/// Shows the open prompt's access keys, as Linux and Windows do (macOS hides them), so a check
/// of its buttons runs against the split drawing on every platform.
fn force_access_keys(vcx: &VisualTestContext) {
    let dialog = vcx
        .cx
        .read(hacivat_app::exec_ui::close_prompt::open_prompt_dialog)
        .expect("a prompt is open");
    vcx.cx.update(|cx| {
        dialog.update(cx, |d, cx| {
            d.set_access_keys(true);
            cx.notify();
        })
    });
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

/// Where access keys show (Linux, Windows) a button's label is drawn as runs around its key
/// letter; the prompts' buttons still read whole (CI run 43779216264: macOS hides the keys, so
/// this turns them on to check the same drawing on every platform).
#[gpui::test]
fn prompt_buttons_read_whole_where_access_keys_show(cx: &mut TestAppContext) {
    let mut c = console_in_transaction(cx, false);
    enable_registry(c.vcx);
    close_console_tab(&c);
    force_access_keys(c.vcx);
    let buttons = dialog_buttons(c.vcx);
    for want in ["Commit and close", "Roll back and close", "Cancel"] {
        assert!(buttons.iter().any(|b| b == want), "{want}: {buttons:?}");
    }
    c.keys("escape");
    stop_core_runtime_test(cx);
}

#[gpui::test]
fn close_prompt_on_a_failed_transaction_offers_roll_back_only(cx: &mut TestAppContext) {
    let mut c = console_in_transaction(cx, true);
    enable_registry(c.vcx);
    close_console_tab(&c);
    assert!(layer_ids(c.vcx).contains(&"dialog"));
    assert_eq!(dialog_title(c.vcx), "The transaction on orders-dev failed");
    // Review F3 M3: the absence check runs against the split drawing on every platform.
    force_access_keys(c.vcx);
    let buttons = dialog_buttons(c.vcx);
    assert!(
        !buttons.iter().any(|b| b.starts_with("Commit")),
        "{buttons:?}"
    );
    assert!(
        buttons.iter().any(|b| b == "Roll back and close"),
        "{buttons:?}"
    );
    assert_eq!(
        c.vcx.cx.read(
            |cx| hacivat_app::exec_ui::close_prompt::open_prompt_dialog(cx)
                .map(|d| d.read(cx).role())
        ),
        Some(gpui::Role::AlertDialog)
    );
    // Escape keeps the console; the commit command is not offered on a failed transaction.
    c.keys("escape");
    assert!(!layer_ids(c.vcx).contains(&"dialog"));
    assert!(
        !run(c.vcx, "tx.commit"),
        "no Commit on a failed transaction"
    );
    // Roll back and close: ROLLBACK, then the tab goes.
    let tabs = tab_count(c.vcx);
    close_console_tab(&c);
    c.keys("shift-tab");
    c.keys("enter");
    wait_until(c.vcx, "the console to close", |vcx| {
        tab_count(vcx) == tabs - 1
    });
    assert!(
        c.fake.executed().iter().any(|s| s.trim() == "ROLLBACK"),
        "{:?}",
        c.fake.executed()
    );
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

/// Review F3 I1: the run's summary reaches the UI at `RunEnded`, while the session is still in
/// the run's request; it publishes the failed transaction only when it settles. Here a
/// terminate on a side connection (gated) is still in flight when the stopped statement ends,
/// so the session waits for it (`CANCEL_WAIT`) between the two: the toast still comes, once
/// the session has settled.
#[gpui::test]
fn a_stop_that_fails_the_transaction_says_so_after_the_session_settles(cx: &mut TestAppContext) {
    use hacivat_session::session::{Escalation, SessionState};
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
    let main = pg().backend(4242).queries(vec![begin, slow]);
    let (c, side) = console_with_side(
        cx,
        profile("orders-dev", DbmsKind::Postgres),
        main,
        FakeDriver::new(DbmsKind::Postgres).gated(),
    );
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
    // Stop (its server cancel goes out once the call is overdue), then terminate at once: the
    // terminate's side connection waits at the gate.
    assert!(run(c.vcx, "exec.cancel"));
    let seq = match c.session.state().borrow().state {
        SessionState::Busy { req, .. } | SessionState::Cancelling { req, .. } => req,
        ref other => panic!("the update runs: {other:?}"),
    };
    drop(c.session.escalate(seq, Escalation::Terminate));
    wait_until(c.vcx, "the run's summary in the UI", |vcx| {
        vcx.cx
            .read(|cx| hacivat_app::exec_ui::last_run_ended(c.id, cx))
    });
    // The gap: the session has not left the run's request, so its snapshot still shows the
    // transaction as it was before the stop.
    let snap = c.session.state().borrow().clone();
    assert!(
        matches!(snap.state, SessionState::Cancelling { req, .. } if req == seq),
        "the terminate holds the session: {:?}",
        snap.state
    );
    assert_eq!(snap.tx.status, TxStatus::InTx, "not published yet");
    side.release();
    wait_until(c.vcx, "the transaction to fail", |_| {
        c.tx().status == TxStatus::Failed
    });
    wait_until(c.vcx, "the toast", |vcx| {
        in_window(vcx, |window, _| window.refresh());
        toast_texts(vcx).iter().any(|t| {
            t == "Stopping the statement failed the transaction on orders-dev. Only a rollback can end it now."
        })
    });
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

// ---- fix round 1 ----------------------------------------------------------------------------

/// `exec.cancel`'s chord (R-M3-38): `cmd-.` on macOS, `ctrl-f2` on Windows and Linux.
#[cfg(target_os = "macos")]
const CANCEL_CHORD: &str = "cmd-.";
#[cfg(not(target_os = "macos"))]
const CANCEL_CHORD: &str = "ctrl-f2";

/// C1: Run asked on one text and answered after an edit never opens the chooser (or its
/// uncertain state) over the new text: the band would show other text than Enter sends.
#[gpui::test]
fn a_chooser_asked_before_an_edit_never_opens(cx: &mut TestAppContext) {
    let c = pg_console(cx, pg());
    c.set_text(TWO);
    c.select(0..0);
    let editor = c.editor.clone();
    // Run, then an edit in the same turn, before the resolution lands.
    in_window(c.vcx, |window, cx| {
        editor.update(cx, |e, cx| {
            assert!(e.run_command("editor.run", window, cx));
            e.set_text("SELECT 7;\nSELECT 8;\nSELECT 9;\n", cx);
        });
    });
    c.vcx.run_until_parked();
    assert!(!c.chooser_open(), "a stale Ask opens no chooser");
    assert!(c.band().is_none_or(|b| !b.from_override));
    c.barrier();
    assert!(c.queries().is_empty(), "{:?}", c.queries());
    // Asked again on the text as it is now, it opens over that text.
    c.run_at(0);
    assert!(c.chooser_open());
    assert_eq!(c.band().expect("a band").units.to_vec(), vec![0..8_usize]);
    stop_core_runtime_test(cx);
}

/// A result set of three numbered rows, for a statement whose tab Re-run reads.
fn three_rows(prefix: &'static str) -> FakeQuery {
    FakeQuery {
        prefix,
        sets: vec![rows(3)],
        ..Default::default()
    }
}

/// C2: Re-run on a result tab sends the statement behind that tab, never a later statement
/// that ran at the same ordinal without a tab of its own (a DELETE after the SELECT).
#[gpui::test]
fn rerun_sends_the_tabs_own_statement_never_a_later_one(cx: &mut TestAppContext) {
    let c = pg_console(
        cx,
        pg().queries(vec![three_rows("SELECT"), three_rows("SELECT")]),
    );
    let text = "SELECT * FROM t;\nDELETE FROM t WHERE id = 5;\n";
    c.set_text(text);
    c.select(0..16);
    c.command("editor.run");
    c.wait_marks("the select", |m| matches!(m, [s] if succeeded(s)));
    let tabs = |c: &Console| {
        c.vcx.cx.read(|cx| {
            Consoles::get(cx, c.id)
                .and_then(|s| s.results.clone())
                .map_or(0, |r| r.read(cx).tabs().len())
        })
    };
    wait_until(c.vcx, "the select's result tab", |_| tabs(&c) == 1);
    c.select(17..43);
    c.command("editor.run");
    c.wait_marks("the delete", |m| matches!(m, [s] if succeeded(s)));
    assert_eq!(tabs(&c), 1, "the delete made no tab");
    in_window(c.vcx, |window, cx| {
        hacivat_app::exec_ui::handle_results_event(
            &hacivat_app::results::ResultsEvent::Rerun {
                console: c.id,
                unit_ordinal: 0,
            },
            window,
            cx,
        );
    });
    c.wait_queries(3);
    assert_eq!(
        c.queries(),
        [
            "SELECT * FROM t",
            "DELETE FROM t WHERE id = 5",
            "SELECT * FROM t"
        ]
    );
    stop_core_runtime_test(cx);
}

/// C2: Re-run of a statement edited since it ran sends nothing and says so.
#[gpui::test]
fn rerun_of_an_edited_statement_sends_nothing(cx: &mut TestAppContext) {
    let c = pg_console(cx, pg().queries(vec![three_rows("SELECT")]));
    c.set_text("SELECT * FROM t;\n");
    c.select(0..16);
    c.command("editor.run");
    c.wait_marks("the select", |m| matches!(m, [s] if succeeded(s)));
    c.vcx.cx.update(|cx| {
        c.editor.update(cx, |e, cx| {
            e.select_range(7..7, cx);
        });
    });
    // Type inside the statement: its mark goes stale.
    let editor = c.editor.clone();
    in_window(c.vcx, |window, cx| {
        let handle = gpui::Focusable::focus_handle(editor.read(cx), cx);
        window.focus(&handle, cx);
    });
    c.vcx.simulate_input("x");
    c.vcx.run_until_parked();
    assert_eq!(editor_text(c.vcx, c.id), "SELECT x* FROM t;\n");
    in_window(c.vcx, |window, cx| {
        hacivat_app::exec_ui::handle_results_event(
            &hacivat_app::results::ResultsEvent::Rerun {
                console: c.id,
                unit_ordinal: 0,
            },
            window,
            cx,
        );
    });
    c.barrier();
    assert_eq!(c.queries(), ["SELECT * FROM t"], "nothing more was sent");
    stop_core_runtime_test(cx);
}

/// C3: `app.restart` with an open user transaction asks first; Cancel keeps the app; Roll back
/// and restart ends the transaction, then restarts.
#[gpui::test]
fn restart_with_an_open_transaction_asks_and_cancel_keeps_the_app(cx: &mut TestAppContext) {
    let mut c = console_in_transaction(cx, false);
    enable_registry(c.vcx);
    let mut restarted = c.vcx.cx.expect_restart();
    assert!(run(c.vcx, "app.restart"));
    wait_until(c.vcx, "the restart prompt", |vcx| {
        layer_ids(vcx).contains(&"dialog")
    });
    let buttons = dialog_buttons(c.vcx);
    assert!(
        buttons.iter().any(|b| b == "Roll back and restart"),
        "{buttons:?}"
    );
    assert!(
        restarted.try_recv().unwrap().is_none(),
        "asked, not restarted"
    );
    c.keys("enter");
    assert!(!layer_ids(c.vcx).contains(&"dialog"));
    assert!(
        restarted.try_recv().unwrap().is_none(),
        "Cancel keeps the app"
    );
    assert_eq!(c.tx().status, TxStatus::InTx);
    // Again: Roll back and restart.
    assert!(run(c.vcx, "app.restart"));
    wait_until(c.vcx, "the restart prompt", |vcx| {
        layer_ids(vcx).contains(&"dialog")
    });
    c.keys("shift-tab");
    c.keys("enter");
    wait_until(c.vcx, "the restart", |_| {
        matches!(restarted.try_recv(), Ok(Some(_)))
    });
    assert!(
        c.fake.executed().iter().any(|s| s.trim() == "ROLLBACK"),
        "{:?}",
        c.fake.executed()
    );
    stop_core_runtime_test(cx);
}

/// I4: closing the chooser (Escape, or Enter that runs) gives the focus back to the editor,
/// and typing reaches it.
#[gpui::test]
fn closing_the_chooser_gives_the_focus_back_to_the_editor(cx: &mut TestAppContext) {
    let mut c = pg_console(cx, pg());
    c.set_text(TWO);
    c.select(0..0);
    c.focus_editor();
    c.command("editor.run");
    assert!(c.chooser_open());
    let editor_focused = |c: &Console| {
        let editor = c.editor.clone();
        in_window(c.vcx, |window, cx| {
            gpui::Focusable::focus_handle(editor.read(cx), cx).is_focused(window)
        })
    };
    assert!(!editor_focused(&c), "the chooser has the focus");
    c.keys("escape");
    assert!(!c.chooser_open());
    assert!(editor_focused(&c), "Escape gives the focus back");
    c.vcx.simulate_input(" ");
    c.vcx.run_until_parked();
    assert_eq!(editor_text(c.vcx, c.id), format!(" {TWO}"));
    // Enter that runs gives it back too.
    c.select(1..1);
    c.command("editor.run");
    assert!(c.chooser_open());
    c.keys("enter");
    assert!(!c.chooser_open());
    assert!(editor_focused(&c), "Enter gives the focus back");
    stop_core_runtime_test(cx);
}

/// I5: `confirm_release` runs a release at once without a user transaction; with one it asks,
/// and runs it only once the transaction ended.
#[gpui::test]
fn confirm_release_asks_before_a_session_is_released(cx: &mut TestAppContext) {
    let mut c = console_in_transaction(cx, false);
    enable_registry(c.vcx);
    let released = std::rc::Rc::new(std::cell::Cell::new(0u32));
    let count = released.clone();
    let id = c.id;
    let d = c.vcx.cx.update(|cx| {
        hacivat_app::exec_ui::close_prompt::confirm_release(id, cx, move |_| {
            count.set(count.get() + 1)
        })
    });
    assert_eq!(d, hacivat_app::shell::CloseDecision::Defer);
    wait_until(c.vcx, "the release prompt", |vcx| {
        layer_ids(vcx).contains(&"dialog")
    });
    let buttons = dialog_buttons(c.vcx);
    assert!(
        buttons.iter().any(|b| b == "Commit and continue"),
        "{buttons:?}"
    );
    c.keys("escape");
    assert_eq!(released.get(), 0, "Cancel: nothing released");
    assert_eq!(c.tx().status, TxStatus::InTx);
    // Again; Commit and continue.
    let count = released.clone();
    c.vcx.cx.update(|cx| {
        hacivat_app::exec_ui::close_prompt::confirm_release(id, cx, move |_| {
            count.set(count.get() + 1)
        })
    });
    wait_until(c.vcx, "the release prompt", |vcx| {
        layer_ids(vcx).contains(&"dialog")
    });
    c.keys("shift-tab shift-tab");
    c.keys("enter");
    wait_until(c.vcx, "the release", |_| released.get() == 1);
    assert_eq!(c.tx().status, TxStatus::Idle);
    // No transaction: at once.
    let count = released.clone();
    let d = c.vcx.cx.update(|cx| {
        hacivat_app::exec_ui::close_prompt::confirm_release(id, cx, move |_| {
            count.set(count.get() + 1)
        })
    });
    assert_eq!(d, hacivat_app::shell::CloseDecision::Proceed);
    assert_eq!(released.get(), 2);
    stop_core_runtime_test(cx);
}

/// Minor 11: `exec.cancel` never stops Hacivat's own SQL-context probe.
#[gpui::test]
fn cancel_never_stops_the_context_probe(cx: &mut TestAppContext) {
    let slow_probe = FakeExec {
        prefix: "SHOW standard_conforming_strings",
        result: Ok(ExecSummary {
            rows: vec![vec![Some("on".to_string())]],
            row_count: 1,
            ..Default::default()
        }),
        delay: std::time::Duration::from_secs(60),
        tx_after: None,
        panics: false,
    };
    let vcx = shell_with_connections(cx, "en");
    in_window(vcx, |window, _| window.activate_window());
    let p = profile("orders-dev", DbmsKind::Postgres);
    let profile_id = add_profile(vcx, p.clone());
    let (session, _fake) = session_over(vcx, p, pg().exec(vec![slow_probe]));
    let id = register_test_console(vcx, profile_id, session.clone());
    wait_until(vcx, "the probe to run", |_| {
        matches!(
            session.state().borrow().state,
            hacivat_session::session::SessionState::Busy { .. }
        )
    });
    assert!(!run(vcx, "exec.cancel"), "nothing of the user's runs");
    assert!(
        vcx.cx
            .read(|cx| Consoles::get(cx, id).is_some_and(|s| s.sql.is_none()))
    );
    stop_core_runtime_test(cx);
}

// ---- preview and Run agree (R-M3-8) -------------------------------------------------------

/// Fragments of scripts that stress the boundaries: comments, strings, dollar bodies, psql
/// commands, `COPY` data, unknown blocks, unterminated constructs, MySQL executable comments
/// and delimiters.
const FRAGMENTS: &[&str] = &[
    // Fix round 1 (Minor 12): non-ASCII text, a backslash quote, a double-quoted name.
    "SELECT 'ğüş;' AS \"müşteri adı\"",
    "'a\\'",
    "\"İzmir;\"",
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

fn doc_of(text: &str, ctx: sql_dialect::DialectCtx) -> sql_split::DocSnapshot {
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
    for round in 0..200 {
        // Fix round 1 (Minor 12): non-default contexts too: PostgreSQL with
        // `standard_conforming_strings = off`, MySQL under `ANSI_QUOTES`.
        let ctx = match round % 5 {
            3 => sql_dialect::DialectCtx::new(sql_dialect::DialectId::MySql, None),
            4 => {
                let mut c = sql_dialect::DialectCtx::new(sql_dialect::DialectId::MySql, None);
                c.modes = sql_dialect::LexModes::from_mysql_sql_mode("ANSI_QUOTES");
                c
            }
            2 => {
                let mut c = sql_dialect::DialectCtx::new(sql_dialect::DialectId::Postgres, None);
                c.modes.standard_conforming_strings = false;
                c
            }
            _ => sql_dialect::DialectCtx::new(sql_dialect::DialectId::Postgres, None),
        };
        let n = 1 + g.below(6);
        let text: String = (0..n)
            .map(|_| FRAGMENTS[g.below(FRAGMENTS.len())])
            .collect();
        let doc = doc_of(&text, ctx);
        for at in (0..=text.len()).filter(|i| text.is_char_boundary(*i)) {
            check_place(
                &doc,
                &ExecAt::Carets(smallvec::smallvec![at]),
                &mut divergences,
            );
            places += 1;
        }
        // Two carets (Fix round 1, Minor 12).
        for _ in 0..6 {
            let mut a = g.below(text.len() + 1);
            let mut b = g.below(text.len() + 1);
            while !text.is_char_boundary(a) {
                a -= 1;
            }
            while !text.is_char_boundary(b) {
                b -= 1;
            }
            check_place(
                &doc,
                &ExecAt::Carets(smallvec::smallvec![a.min(b), a.max(b)]),
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

// ---- fix round 1: the SQL context on read-only sessions, and an unknown one ------------------

/// T31 finding: on a read-only session the guard forces the extended protocol, whose internal
/// statements carry no values; the probe runs outside the guard instead (follow-up F1-c; it
/// was a typed run), so a read-only MySQL console under `ANSI_QUOTES` lexes `"…"` as a name.
#[gpui::test]
fn a_read_only_mysql_console_reads_its_sql_mode(cx: &mut TestAppContext) {
    let mut p = profile("shop-ro", DbmsKind::MySql);
    p.read_only = true;
    // ## m3a-f1-followups: the probe's value by the values path.
    let c = console_with(cx, p, mysql("ANSI_QUOTES,STRICT_TRANS_TABLES"));
    let ctx = c
        .vcx
        .cx
        .read(|cx| hacivat_app::exec_ui::sql_context(c.id, cx));
    let SqlContext::Known(ctx) = ctx else {
        panic!("the probe's value was read: {ctx:?}");
    };
    assert!(ctx.modes.ansi_quotes, "{ctx:?}");
    assert_eq!(
        c.vcx
            .cx
            .read(|cx| Consoles::get(cx, c.id).and_then(|s| s.sql)),
        Some(ctx)
    );
    // ## m3a-f1-followups: an internal statement, never a typed (guarded) run.
    assert!(
        c.fake
            .executed()
            .iter()
            .any(|q| q == "SELECT @@SESSION.sql_mode"),
        "{:?}",
        c.fake.executed()
    );
    assert!(c.queries().is_empty(), "{:?}", c.fake.typed_log());
    stop_core_runtime_test(cx);
}

/// A probe that answers no value leaves the context unknown (never the defaults as if known):
/// Run sends only what every mode combination resolves alike; a mode-dependent boundary is
/// uncertain and sends nothing.
#[gpui::test]
fn a_probe_without_a_value_leaves_the_context_unknown(cx: &mut TestAppContext) {
    let refused = FakeExec {
        prefix: "SHOW standard_conforming_strings",
        result: Err(hacivat_driver::RawError {
            kind: hacivat_driver::RawErrorKind::Db,
            server: None,
        }),
        delay: std::time::Duration::ZERO,
        tx_after: None,
        panics: false,
    };
    let c = pg_console(cx, pg().exec(vec![refused]));
    let ctx = c
        .vcx
        .cx
        .read(|cx| hacivat_app::exec_ui::sql_context(c.id, cx));
    assert!(matches!(ctx, SqlContext::Unknown(_)), "{ctx:?}");
    assert_eq!(
        c.vcx
            .cx
            .read(|cx| Consoles::get(cx, c.id).and_then(|s| s.sql)),
        None,
        "nothing claims a known context"
    );
    // `'a\'` ends at its second quote with standard_conforming_strings on, and runs on with it
    // off: mode-dependent, so nothing is sent.
    c.set_text("SELECT 'a\\'; SELECT 2;");
    c.run_at(0);
    c.barrier();
    assert!(c.queries().is_empty(), "{:?}", c.queries());
    assert!(c.chooser_open(), "the uncertain state");
    assert_eq!(c.chooser_selected(), None);
    c.set_text("SELECT 1");
    // The same in every mode: it runs.
    c.run_at(0);
    c.wait_queries(1);
    assert_eq!(c.queries(), ["SELECT 1"]);
    stop_core_runtime_test(cx);
}

/// Task 32 contract: while a production-tagged console waits for its open transaction to end
/// before reconnecting read-only (`pending_read_only`), Run sends nothing and says why; Commit
/// and Roll back stay available; once the flag clears, Run runs again.
#[gpui::test]
fn run_refuses_while_the_console_waits_to_become_read_only(cx: &mut TestAppContext) {
    let c = console_in_transaction(cx, false);
    let sent = c.queries().len();
    let id = c.id;
    c.vcx
        .cx
        .update(|cx| hacivat_app::exec_ui::set_pending_read_only(id, true, cx));
    c.set_text("UPDATE t SET a = 1");
    c.run_at(0);
    c.barrier();
    assert_eq!(
        c.queries().len(),
        sent,
        "nothing was sent: {:?}",
        c.queries()
    );
    assert!(!c.chooser_open());
    enable_registry(c.vcx);
    in_window(c.vcx, |window, _| window.refresh());
    c.vcx.run_until_parked();
    assert!(
        toast_texts(c.vcx).iter().any(|t| t
            == "Nothing was sent: this console becomes read-only once its open transaction ends. Commit or roll back first."),
        "{:?}",
        toast_texts(c.vcx)
    );
    // Commit stays available, and ends the transaction.
    assert!(run(c.vcx, "tx.commit"));
    wait_until(c.vcx, "the commit", |_| c.tx().status == TxStatus::Idle);
    // Task 32 clears the flag once the console is read-only; Run runs again.
    c.vcx
        .cx
        .update(|cx| hacivat_app::exec_ui::set_pending_read_only(id, false, cx));
    c.set_text("SELECT 1");
    c.run_at(0);
    c.wait_queries(sent + 1);
    stop_core_runtime_test(cx);
}

/// Task 32 C2: a disconnect (or a profile delete) goes through `confirm_release` too: its
/// continuation closes the session, after Roll back and continue ended the transaction, with
/// `RefuseIfOpenTx` (which then has nothing to refuse).
#[gpui::test]
fn confirm_release_lets_a_disconnect_continue_after_the_rollback(cx: &mut TestAppContext) {
    let mut c = console_in_transaction(cx, false);
    enable_registry(c.vcx);
    let id = c.id;
    let session = c.session.clone();
    let closed = std::rc::Rc::new(std::cell::Cell::new(false));
    let done = closed.clone();
    c.vcx.cx.update(|cx| {
        hacivat_app::exec_ui::close_prompt::confirm_release(id, cx, move |_| {
            let close = session.close(hacivat_session::session::ClosePolicy::RefuseIfOpenTx);
            done.set(futures::executor::block_on(close).is_ok());
        })
    });
    wait_until(c.vcx, "the release prompt", |vcx| {
        layer_ids(vcx).contains(&"dialog")
    });
    // Roll back and continue.
    c.keys("shift-tab");
    c.keys("enter");
    wait_until(c.vcx, "the disconnect", |_| closed.get());
    assert!(
        c.fake.executed().iter().any(|s| s.trim() == "ROLLBACK"),
        "{:?}",
        c.fake.executed()
    );
    wait_until(c.vcx, "the session to close", |_| {
        !hacivat_exec::session_live(&c.session.state().borrow().state)
    });
    stop_core_runtime_test(cx);
}

// ## m3a-f1-followups
/// Follow-up F1-c: a read-only (production) console on a server the read-only guard cannot
/// verify (its canary answers nothing). Binding the console probes its SQL context at once;
/// the probe keeps the session (it was guarded and broke it, so every connect of such a
/// server ended `Lost { Internal }`). The guard stays fail-closed for the user's statements.
#[gpui::test]
fn a_read_only_console_the_guard_cannot_verify_keeps_its_session(cx: &mut TestAppContext) {
    let mut p = profile("orders-prod", DbmsKind::Postgres);
    p.read_only = true;
    let c = console_with(cx, p, pg());
    let ctx = c
        .vcx
        .cx
        .read(|cx| hacivat_app::exec_ui::sql_context(c.id, cx));
    assert!(
        matches!(ctx, SqlContext::Known(ctx) if ctx.modes.standard_conforming_strings),
        "{ctx:?}"
    );
    assert!(
        hacivat_exec::session_live(&c.session.state().borrow().state),
        "the probe kept the session: {:?}",
        c.session.state().borrow().state
    );
    // A user statement goes through the guard, which cannot read its canary here: the
    // session is discarded and the statement reported failed.
    c.set_text("SELECT 1");
    c.run_at(0);
    c.wait_marks("the failure", |m| {
        matches!(m, [ExecMarkState::Failed { .. }])
    });
    wait_until(c.vcx, "the guard to discard the session", |_| {
        !hacivat_exec::session_live(&c.session.state().borrow().state)
    });
    stop_core_runtime_test(cx);
}

// ## m3a-f5-demo (DEMO-9): a MySQL `CALL` that answers three result sets shows the first in a
// result tab, says the others were not shown (strip and Output), on the PO's exact text.
#[gpui::test]
fn a_call_with_three_result_sets_shows_the_first_and_notes_the_rest(cx: &mut TestAppContext) {
    let set = |n: i64| FakeResultSet {
        columns: vec![(
            "n",
            LogicalType::Int {
                bits: 64,
                signed: true,
            },
        )],
        rows: FakeRows::Literal(vec![vec![Value::Int(n)]]),
    };
    let three = FakeQuery {
        prefix: "CALL hv_test.hv_t14_three()",
        sets: vec![set(1), set(2), set(3)],
        ..Default::default()
    };
    let c = console_with(
        cx,
        profile("mysql84", DbmsKind::MySql),
        mysql("ONLY_FULL_GROUP_BY").queries(vec![three]),
    );
    c.set_text("CALL hv_test.hv_t14_three();");
    c.run_at(0);
    c.wait_marks("the CALL to succeed", |m| matches!(m, [s] if succeeded(s)));
    assert_eq!(c.queries(), ["CALL hv_test.hv_t14_three()"]);
    let ids = c.log_ids();
    assert!(ids.contains(&"exec-more-result-sets"), "{ids:?}");
    let tabs = c.vcx.cx.read(|cx| {
        Consoles::get(cx, c.id)
            .and_then(|s| s.results.clone())
            .map(|r| r.read(cx).tabs().len())
    });
    assert_eq!(tabs, Some(1), "one result tab");
    enable_registry(c.vcx);
    in_window(c.vcx, |window, _| window.refresh());
    c.vcx.run_until_parked();
    let drawn: Vec<String> = registry_frame(c.vcx)
        .into_iter()
        .map(|r| r.text.to_string())
        .collect();
    assert!(
        drawn
            .iter()
            .any(|t| t.contains("2 more result sets not shown")),
        "{drawn:?}"
    );
    assert!(drawn.iter().any(|t| t == "Result 1"), "{drawn:?}");
    stop_core_runtime_test(cx);
}
