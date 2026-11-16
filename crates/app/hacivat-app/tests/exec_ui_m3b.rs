//! M3b Task 24: the run pipeline in the app, end to end over FakeDriver sessions: parameters,
//! classification on the pool with the exact bytes sent, the gate, the confirmations before the
//! run, Run as one, the console's run policy (CHOOSE-06), and production read-only until Task 26
//! (## m3b-t26-prod-unlock: since then production writes behind its confirmations).

mod support;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use gpui::{Entity, TestAppContext, VisualTestContext};
use hacivat_app::console::{ConsoleId, Consoles};
use hacivat_app::exec_ui::context::SqlContext;
use hacivat_app::exec_ui::{chooser, confirm, orchestrator, params_prompt, policy_popover};
use hacivat_driver::ExecSummary;
use hacivat_driver::fake::{FakeCall, FakeDriver, FakeExec, FakeTypedCall};
use hacivat_exec::{ClassifyPool, ExecUnit};
use hacivat_proto::{DbmsKind, EnvTag, ProfileId};
use hacivat_session::session::SessionHandle;
use hacivat_sql_editor::InsideChoice;
use hacivat_ui_editor::{EditorView, ExecMarkState};
use hacivat_values::{BindValue, LogicalType};
use support::*;

/// A console over a FakeDriver session of `profile`, with its SQL context read.
struct Console {
    vcx: &'static mut VisualTestContext,
    id: ConsoleId,
    fake: Arc<FakeDriver>,
    #[allow(dead_code)]
    profile: ProfileId,
    session: SessionHandle,
    editor: Entity<EditorView>,
}

fn profile(name: &str, engine: DbmsKind, env: EnvTag) -> hacivat_session::profile::Profile {
    hacivat_session::profile::Profile {
        engine,
        ..test_profile(name, Some(env), "127.0.0.1")
    }
}

/// `answer` for `n` internal reads of `prefix` (the SQL-context probe).
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

/// The session's own context read (T18's constant text; fix round 1, I1), answering `lex`
/// and the search path `path`, for `n` reads.
fn context_reads(lex: &str, path: &str) -> Vec<FakeExec> {
    (0..8)
        .map(|_| FakeExec {
            prefix: "SELECT pg_catalog.current_setting('standard_conforming_strings')",
            result: Ok(ExecSummary {
                rows: vec![vec![Some(lex.to_string()), Some(path.to_string())]],
                row_count: 1,
                ..ExecSummary::default()
            }),
            delay: Duration::ZERO,
            tx_after: None,
            panics: false,
        })
        .collect()
}

fn pg() -> FakeDriver {
    FakeDriver::new(DbmsKind::Postgres)
        .tracks_transactions()
        .exec(
            probes("SHOW standard_conforming_strings", "on")
                .into_iter()
                .chain(context_reads("on", "\"$user\", public"))
                .collect(),
        )
}

fn mysql() -> FakeDriver {
    FakeDriver::new(DbmsKind::MySql)
        .tracks_transactions()
        .exec(probes("SELECT @@SESSION.sql_mode", "ONLY_FULL_GROUP_BY"))
}

/// Shells without an observed pool use boot's.
fn console(
    cx: &mut TestAppContext,
    p: hacivat_session::profile::Profile,
    fake: FakeDriver,
) -> Console {
    console_observed(cx, p, fake, None)
}

/// [`console`], classifying on `pool` (installed before the first run).
fn console_observed(
    cx: &mut TestAppContext,
    p: hacivat_session::profile::Profile,
    fake: FakeDriver,
    pool: Option<Arc<ClassifyPool>>,
) -> Console {
    let vcx = shell_with_connections(cx, "en");
    in_window(vcx, |window, _| window.activate_window());
    if let Some(pool) = pool {
        vcx.cx
            .update(|cx| orchestrator::install_classifier(pool, cx));
    }
    let profile = add_profile(vcx, p.clone());
    let rt = vcx.cx.read(|cx| {
        cx.global::<hacivat_app::connections::ConnectionsModel>()
            .core_runtime()
            .handle()
    });
    let (session, fake) =
        futures::executor::block_on(hacivat_session::testing::fake_session(rt, fake, p));
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
        profile,
        session,
        editor,
    }
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

    fn binds(&self) -> Vec<Vec<BindValue>> {
        self.fake
            .typed_log()
            .into_iter()
            .filter_map(|c| match c {
                FakeTypedCall::Binds(b) => Some(b),
                _ => None,
            })
            .collect()
    }

    fn wait_queries(&self, n: usize) {
        wait_until(self.vcx, "the queries to be sent", |_| {
            self.queries().len() >= n
        });
    }

    /// Everything Run had sent has reached the session (a barrier, not a sleep).
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
            hacivat_app::exec_ui::last_run_marks(self.id, cx)
                .into_iter()
                .flatten()
                .collect()
        })
    }

    /// The open parameter prompt (a closed one a test still holds is not it: Run reads the
    /// session's context first, off the UI thread, so the next prompt opens a moment later).
    fn prompt(&self) -> Entity<params_prompt::ParamsPrompt> {
        wait_until(self.vcx, "the parameter prompt", |vcx| {
            vcx.cx.read(|cx| params_prompt::open_prompt(cx).is_some())
                && layer_ids(vcx).contains(&params_prompt::PARAMS_LAYER_ID)
        });
        self.vcx.cx.read(params_prompt::open_prompt).expect("open")
    }

    fn selected(&self) -> Option<usize> {
        self.vcx
            .cx
            .read(|cx| chooser::menu_of(self.id, cx).and_then(|m| m.read(cx).selected()))
    }

    fn confirm_open(&self) -> bool {
        self.vcx.cx.read(confirm::confirm_open)
    }

    fn wait_confirm(&self) {
        wait_until(self.vcx, "a confirmation", |vcx| {
            vcx.cx.read(confirm::confirm_open)
        });
        // The run buttons arm after a moment (T21 fix round 1, I1).
        self.vcx
            .executor()
            .advance_clock(confirm::ARM_DELAY + Duration::from_millis(50));
        self.vcx.run_until_parked();
    }

    fn drawn(&self) -> Vec<String> {
        enable_registry(self.vcx);
        in_window(self.vcx, |window, _| window.refresh());
        self.vcx.run_until_parked();
        registry_frame(self.vcx)
            .into_iter()
            .map(|r| r.text.to_string())
            .collect()
    }

    fn keys(&mut self, keys: &str) {
        self.vcx.simulate_keystrokes(keys);
        self.vcx.run_until_parked();
    }
}

fn dev(cx: &mut TestAppContext) -> Console {
    console(
        cx,
        profile("orders-dev", DbmsKind::Postgres, EnvTag::Dev),
        pg(),
    )
}

fn click_button(vcx: &mut VisualTestContext, id: &str) {
    let selector: &'static str = Box::leak(format!("button-{id}").into_boxed_str());
    click(vcx, selector);
    vcx.run_until_parked();
}

#[gpui::test]
fn deny_sends_nothing_and_marks_the_gutter(cx: &mut TestAppContext) {
    let mut p = profile("orders-ro", DbmsKind::Postgres, EnvTag::Dev);
    p.read_only = true;
    let c = console(cx, p, pg());
    c.set_text("DELETE FROM t WHERE id = 1");
    c.run_at(0);
    wait_until(c.vcx, "the denial's mark", |_| {
        matches!(
            c.marks().as_slice(),
            [ExecMarkState::Failed { sqlstate: None }]
        )
    });
    c.barrier();
    assert!(
        c.queries().is_empty(),
        "nothing was sent: {:?}",
        c.queries()
    );
    assert!(
        c.log_ids().contains(&"exec-gate-read-only"),
        "{:?}",
        c.log_ids()
    );
    assert!(c.drawn().iter().any(|t| t == "not sent: read-only"));
    stop_core_runtime_test(cx);
}

#[gpui::test]
fn params_prompt_cancel_sends_nothing(cx: &mut TestAppContext) {
    let c = dev(cx);
    c.set_text("SELECT * FROM t WHERE id = :id");
    c.run_at(0);
    let prompt = c.prompt();
    in_window(c.vcx, |window, cx| {
        prompt.update(cx, |p, cx| p.cancel(window, cx))
    });
    c.wait_log("exec-params-cancelled");
    c.barrier();
    assert!(c.queries().is_empty(), "{:?}", c.queries());
    assert!(
        !layer_ids(c.vcx).contains(&params_prompt::PARAMS_LAYER_ID),
        "the prompt closed: {:?}",
        layer_ids(c.vcx)
    );
    stop_core_runtime_test(cx);
}

#[gpui::test]
fn params_are_bound_and_remembered_for_the_session(cx: &mut TestAppContext) {
    let c = dev(cx);
    c.set_text("SELECT * FROM t WHERE id = :id AND n = ?");
    c.run_at(0);
    let prompt = c.prompt();
    let names: Vec<String> = c.vcx.cx.read(|cx| {
        prompt
            .read(cx)
            .prompts()
            .iter()
            .map(|p| p.name.to_string())
            .collect()
    });
    // PostgreSQL's `?` is the jsonb operator, not a parameter (said in the prompt).
    assert_eq!(names, ["id"]);
    c.vcx
        .cx
        .update(|cx| prompt.update(cx, |p, cx| p.set_text(0, "42", cx)));
    in_window(c.vcx, |window, cx| {
        prompt.update(cx, |p, cx| p.submit(window, cx))
    });
    c.wait_queries(1);
    assert_eq!(c.queries(), ["SELECT * FROM t WHERE id = $1 AND n = ?"]);
    assert_eq!(
        c.binds(),
        [vec![BindValue::Typed {
            hint: LogicalType::Int {
                bits: 64,
                signed: true
            },
            text: Arc::from("42"),
        }]]
    );
    wait_until(c.vcx, "the run to end", |vcx| {
        vcx.cx
            .read(|cx| hacivat_app::exec_ui::last_run_ended(c.id, cx))
    });
    // Run again: the prompt asks again (never skipped), its field filled with 42.
    c.run_at(0);
    let prompt = c.prompt();
    let (text, remembered) = c.vcx.cx.read(|cx| {
        let p = prompt.read(cx);
        (p.text(0, cx), p.prompts()[0].remembered.clone())
    });
    assert_eq!(text.as_deref(), Some("42"));
    assert!(remembered.is_some(), "a named value is remembered");
    in_window(c.vcx, |window, cx| {
        prompt.update(cx, |p, cx| p.cancel(window, cx))
    });
    stop_core_runtime_test(cx);
}

/// An ordinal or anonymous parameter's value is never remembered (T15 I4): it belongs to one
/// statement.
#[gpui::test]
fn ordinal_parameters_are_never_remembered(cx: &mut TestAppContext) {
    let c = console(
        cx,
        profile("shop-dev", DbmsKind::MySql, EnvTag::Dev),
        mysql(),
    );
    c.set_text("SELECT * FROM t WHERE id = ?");
    c.run_at(0);
    let prompt = c.prompt();
    c.vcx
        .cx
        .update(|cx| prompt.update(cx, |p, cx| p.set_text(0, "7", cx)));
    in_window(c.vcx, |window, cx| {
        prompt.update(cx, |p, cx| p.submit(window, cx))
    });
    c.wait_queries(1);
    wait_until(c.vcx, "the run to end", |vcx| {
        vcx.cx
            .read(|cx| hacivat_app::exec_ui::last_run_ended(c.id, cx))
    });
    c.run_at(0);
    let prompt = c.prompt();
    let remembered = c
        .vcx
        .cx
        .read(|cx| prompt.read(cx).prompts()[0].remembered.clone());
    assert_eq!(remembered, None);
    in_window(c.vcx, |window, cx| {
        prompt.update(cx, |p, cx| p.cancel(window, cx))
    });
    stop_core_runtime_test(cx);
}

#[gpui::test]
fn turkish_grouped_number_is_flagged_ambiguous(cx: &mut TestAppContext) {
    use params_prompt::{ValueRead, read_value};
    // Machine format first; a dot-grouped number is ambiguous only where the region groups
    // with a dot.
    assert!(params_prompt::groups_with_dot("tr-TR"));
    assert!(!params_prompt::groups_with_dot("en-US"));
    assert!(matches!(
        read_value("1.234", false, true, None),
        ValueRead::Ambiguous { ref value, ref plain } if value == "1.234" && plain == "1234"
    ));
    assert!(matches!(
        read_value("1.234", false, false, None),
        ValueRead::Value(_)
    ));
    assert!(matches!(
        read_value("1.234", false, true, Some("1.234")),
        ValueRead::Value(BindValue::Typed { .. })
    ));
    assert!(matches!(
        read_value("\\x4g", false, false, None),
        ValueRead::BadHex
    ));
    assert!(matches!(
        read_value("x", true, false, None),
        ValueRead::Value(BindValue::Null)
    ));
    // The prompt: Run stays off until the value is edited or "Use 1.234" confirms it.
    let vcx = shell(cx, "en");
    let prompts = vec![hacivat_exec::ParamPrompt {
        name: Arc::from(":tutar"),
        occurrences: 1,
        remembered: None,
        unit: None,
    }];
    let ctx = params_prompt::PromptContext {
        connection: Arc::from("orders-dev"),
        env: Some(EnvTag::Dev),
        engine: DbmsKind::Postgres,
        sql: Arc::from("SELECT * FROM t WHERE tutar = :tutar"),
        notices: Vec::new(),
        grouping_dot: true,
    };
    let mut rx = vcx.cx.update(|cx| params_prompt::open(prompts, ctx, cx));
    vcx.run_until_parked();
    let prompt = vcx.cx.read(params_prompt::open_prompt).expect("the prompt");
    vcx.cx
        .update(|cx| prompt.update(cx, |p, cx| p.set_text(0, "1.234", cx)));
    assert!(
        !vcx.cx.read(|cx| prompt.read(cx).run_enabled(cx)),
        "flagged"
    );
    vcx.cx
        .update(|cx| prompt.update(cx, |p, cx| p.use_value(0, cx)));
    assert!(
        vcx.cx.read(|cx| prompt.read(cx).run_enabled(cx)),
        "confirmed"
    );
    vcx.cx
        .update(|cx| prompt.update(cx, |p, cx| p.set_text(0, "1.500", cx)));
    assert!(
        !vcx.cx.read(|cx| prompt.read(cx).run_enabled(cx)),
        "flagged again"
    );
    vcx.cx
        .update(|cx| prompt.update(cx, |p, cx| p.set_text(0, "1500", cx)));
    assert!(vcx.cx.read(|cx| prompt.read(cx).run_enabled(cx)), "edited");
    in_window(vcx, |window, cx| {
        prompt.update(cx, |p, cx| p.submit(window, cx))
    });
    vcx.run_until_parked();
    let values = rx.try_recv().expect("answered").expect("not cancelled");
    assert!(matches!(
        &values[0].1,
        BindValue::Typed { text, .. } if &**text == "1500"
    ));
}

#[gpui::test]
fn confirmations_asked_in_unit_order_before_the_run(cx: &mut TestAppContext) {
    let c = dev(cx);
    c.set_text("DELETE FROM a;\nDELETE FROM b;");
    c.command("editor.run_3");
    c.wait_confirm();
    assert!(c.drawn().iter().any(|t| t == "statement 1 of 2"));
    c.barrier();
    assert!(c.queries().is_empty(), "nothing before the answers");
    click_button(c.vcx, "confirm-execute");
    c.wait_confirm();
    assert!(c.drawn().iter().any(|t| t == "statement 2 of 2"));
    c.barrier();
    assert!(c.queries().is_empty(), "nothing before every answer");
    click_button(c.vcx, "confirm-execute");
    c.wait_queries(2);
    assert_eq!(c.queries(), ["DELETE FROM a", "DELETE FROM b"]);
    stop_core_runtime_test(cx);
}

#[gpui::test]
fn skip_all_skips_the_rest_and_logs_it(cx: &mut TestAppContext) {
    let c = dev(cx);
    c.set_text("SELECT 1;\nDELETE FROM a;\nDELETE FROM b;");
    c.command("editor.run_3");
    c.wait_confirm();
    click_button(c.vcx, "confirm-skip-all");
    c.wait_queries(1);
    wait_until(c.vcx, "the run to end", |vcx| {
        vcx.cx
            .read(|cx| hacivat_app::exec_ui::last_run_ended(c.id, cx))
    });
    c.barrier();
    assert_eq!(c.queries(), ["SELECT 1"]);
    let skipped = c
        .log_ids()
        .iter()
        .filter(|id| **id == "exec-confirm-skipped")
        .count();
    assert_eq!(skipped, 2, "{:?}", c.log_ids());
    stop_core_runtime_test(cx);
}

#[gpui::test]
fn suppressed_fingerprint_runs_without_asking(cx: &mut TestAppContext) {
    let c = dev(cx);
    c.set_text("DELETE FROM a");
    c.run_at(0);
    c.wait_confirm();
    click_button(c.vcx, "confirm-suppress");
    c.wait_queries(1);
    wait_until(c.vcx, "the run to end", |vcx| {
        vcx.cx
            .read(|cx| hacivat_app::exec_ui::last_run_ended(c.id, cx))
    });
    c.run_at(0);
    c.wait_queries(2);
    assert!(!c.confirm_open(), "not asked again");
    assert_eq!(c.queries(), ["DELETE FROM a", "DELETE FROM a"]);
    assert!(c.log_ids().contains(&"exec-confirm-suppressed"));
    stop_core_runtime_test(cx);
}

#[gpui::test]
fn run_as_one_from_the_uncertain_chooser(cx: &mut TestAppContext) {
    let mut c = console(
        cx,
        profile("shop-dev", DbmsKind::MySql, EnvTag::Dev),
        mysql(),
    );
    c.set_text("SELECT 1 /*! ; */ ;");
    c.run_at(0);
    assert!(c.vcx.cx.read(|cx| chooser::is_open(c.id, cx)), "uncertain");
    assert_eq!(c.selected(), None, "Run as one is never preselected");
    // Enter alone runs nothing.
    c.keys("enter");
    c.barrier();
    assert!(c.queries().is_empty());
    c.run_at(0);
    c.keys("down");
    assert!(c.selected().is_some(), "↓ highlights Run as one");
    c.keys("enter");
    c.wait_queries(1);
    assert_eq!(c.queries(), ["SELECT 1 /*! ; */"]);
    stop_core_runtime_test(cx);
}

#[gpui::test]
fn customize_sets_the_console_policy(cx: &mut TestAppContext) {
    let mut c = dev(cx);
    c.set_text("SELECT 1;\nSELECT 2;");
    c.run_at(12);
    assert!(c.vcx.cx.read(|cx| chooser::is_open(c.id, cx)), "Ask");
    // Customize… is the last row.
    c.keys("end enter");
    assert!(c.vcx.cx.read(|cx| policy_popover::is_open(c.id, cx)));
    c.barrier();
    assert!(c.queries().is_empty(), "Customize… sends nothing");
    // From Ask (checked), two rows down: Largest statement.
    c.keys("down down enter");
    let policy = c.vcx.cx.read(|cx| orchestrator::policy_of(c.id, cx));
    assert_eq!(policy.inside, InsideChoice::Largest);
    assert!(
        c.vcx.cx.read(|cx| policy_popover::is_open(c.id, cx)),
        "it stays open on the choice"
    );
    c.keys("escape");
    assert!(!c.vcx.cx.read(|cx| policy_popover::is_open(c.id, cx)));
    // Run at the caret now runs the statement directly, no chooser.
    c.run_at(12);
    c.wait_queries(1);
    assert_eq!(c.queries(), ["SELECT 2"]);
    wait_until(c.vcx, "the run to end", |vcx| {
        vcx.cx
            .read(|cx| hacivat_app::exec_ui::last_run_ended(c.id, cx))
    });
    // A selection runs as one unit once the console says so.
    c.vcx.cx.update(|cx| {
        orchestrator::set_policy(
            c.id,
            hacivat_sql_editor::RunPolicy {
                selection_as_one: true,
                ..policy
            },
            cx,
        );
    });
    c.select(0..19);
    c.command("editor.run");
    c.wait_queries(2);
    assert_eq!(c.queries()[1], "SELECT 1;\nSELECT 2;");
    stop_core_runtime_test(cx);
}

// ## m3b-t26-prod-unlock: was `production_stays_read_only_until_the_unlock` (the gate denied
// the UPDATE as read-only, nothing sent). Since Task 26 the same UPDATE is confirmed before
// anything is kept: it runs in the user's transaction under the savepoint, the Keep/Undo
// prompt opens, and nothing is committed or released without an answer. A read still runs
// without a dialog.
#[gpui::test]
fn production_writes_only_behind_its_confirmation(cx: &mut TestAppContext) {
    let c = console(
        cx,
        profile("orders-prod", DbmsKind::Postgres, EnvTag::Production),
        pg(),
    );
    assert!(
        !c.session.state().borrow().read_only,
        "the session itself writes"
    );
    c.set_text("UPDATE t SET a = 1 WHERE id = 1");
    c.run_at(0);
    wait_until(c.vcx, "the Keep/Undo prompt", |vcx| {
        layer_ids(vcx).contains(&hacivat_app::exec_ui::commit_prompt::COMMIT_LAYER_ID)
    });
    assert!(
        !c.confirm_open(),
        "a DML asks after it runs, in its savepoint"
    );
    assert_eq!(c.queries(), ["UPDATE t SET a = 1 WHERE id = 1"]);
    let calls = c.fake.call_log();
    assert!(
        calls.contains(&FakeCall::Statement("SAVEPOINT hv_confirm".into())),
        "{calls:?}"
    );
    assert!(
        !calls
            .iter()
            .any(|call| matches!(call, FakeCall::Statement(s)
            if s.contains("COMMIT") || s.starts_with("RELEASE"))),
        "nothing kept before the answer: {calls:?}"
    );
    assert!(!c.log_ids().contains(&"exec-gate-read-only"));
    stop_core_runtime_test(cx);
}

// ## m3b-t26-prod-unlock: the read half of the old test, on its own console.
#[gpui::test]
fn a_production_read_runs_without_a_dialog(cx: &mut TestAppContext) {
    let c = console(
        cx,
        profile("orders-prod", DbmsKind::Postgres, EnvTag::Production),
        pg(),
    );
    c.set_text("SELECT 1");
    c.run_at(0);
    c.wait_queries(1);
    assert_eq!(c.queries(), ["SELECT 1"]);
    assert!(!c.confirm_open());
    assert!(!layer_ids(c.vcx).contains(&hacivat_app::exec_ui::commit_prompt::COMMIT_LAYER_ID));
    stop_core_runtime_test(cx);
}

/// What an observed pool saw: each text and the thread it was classified on.
type Seen = Arc<Mutex<Vec<(String, String)>>>;

fn observed() -> (Arc<ClassifyPool>, Seen) {
    let seen: Seen = Arc::default();
    let log = seen.clone();
    let pool = ClassifyPool::observed(
        2,
        Arc::new(move |sql: &str, _| {
            let thread = std::thread::current().name().unwrap_or("").to_string();
            log.lock().unwrap().push((sql.to_string(), thread));
        }),
    );
    (pool, seen)
}

#[gpui::test]
fn classification_never_runs_on_the_ui_thread(cx: &mut TestAppContext) {
    let (pool, seen) = observed();
    let c = console_observed(
        cx,
        profile("orders-dev", DbmsKind::Postgres, EnvTag::Dev),
        pg(),
        Some(pool),
    );
    c.set_text("SELECT 1");
    c.run_at(0);
    c.wait_queries(1);
    let here = std::thread::current().name().unwrap_or("").to_string();
    let seen = seen.lock().unwrap().clone();
    assert!(!seen.is_empty(), "the run was classified");
    for (_, thread) in &seen {
        assert!(thread.starts_with("hacivat-classify-"), "{thread}");
        assert_ne!(*thread, here);
    }
    stop_core_runtime_test(cx);
}

#[gpui::test]
fn classifier_receives_final_text(cx: &mut TestAppContext) {
    let (pool, seen) = observed();
    let c = console_observed(
        cx,
        profile("orders-dev", DbmsKind::Postgres, EnvTag::Dev),
        pg(),
        Some(pool),
    );
    c.set_text("SELECT :a");
    c.run_at(0);
    let prompt = c.prompt();
    c.vcx
        .cx
        .update(|cx| prompt.update(cx, |p, cx| p.set_text(0, "1", cx)));
    in_window(c.vcx, |window, cx| {
        prompt.update(cx, |p, cx| p.submit(window, cx))
    });
    c.wait_queries(1);
    let texts: Vec<String> = seen
        .lock()
        .unwrap()
        .iter()
        .map(|(s, _)| s.clone())
        .collect();
    assert_eq!(texts, ["SELECT $1"], "the pool saw the text sent (P-M3b-2)");
    assert_eq!(c.queries(), ["SELECT $1"]);
    stop_core_runtime_test(cx);
}

/// T21 carry: a production MySQL `EXPLAIN ANALYZE` of a write asks first; Execute keeps the
/// item's `ExplainAnalyzeRollback` verdict, so the session wraps it in a rolled-back
/// transaction or savepoint (D-062). The gate is asked for production read-write here (Task 26's world, on a
/// FakeDriver session): the app gates production read-only until then.
#[gpui::test]
fn mysql_explain_analyze_execute_keeps_the_rollback_wrap(cx: &mut TestAppContext) {
    use hacivat_exec::{ConfirmAnswer, ConnSafety, GateDecision, SuppressSet};
    use hacivat_session::session::{TxMode, TxOwner, UnitVerdict};
    let vcx = shell_with_connections(cx, "en");
    let p = profile("shop-prod", DbmsKind::MySql, EnvTag::Production);
    // A fake that tracks its transactions, as a MySQL server with `autocommit = 0` does (the
    // session reads the wrap's state from it).
    let rt = vcx.cx.read(|cx| {
        cx.global::<hacivat_app::connections::ConnectionsModel>()
            .core_runtime()
            .handle()
    });
    // Its plan rows; the update it measures opens the Manual session's transaction, as an
    // InnoDB table's does.
    let plan = hacivat_driver::fake::FakeQuery {
        prefix: "EXPLAIN",
        sets: vec![hacivat_driver::fake::FakeResultSet {
            columns: vec![(
                "EXPLAIN",
                LogicalType::Text {
                    max_chars: None,
                    fixed: false,
                },
            )],
            rows: hacivat_driver::fake::FakeRows::Literal(vec![vec![hacivat_values::Value::Text(
                "-> Update t".into(),
            )]]),
        }],
        tx_after: Some(hacivat_driver::TxStatus::InTx),
        ..Default::default()
    };
    let (session, fake) = futures::executor::block_on(hacivat_session::testing::fake_session(
        rt,
        FakeDriver::new(DbmsKind::MySql)
            .tracks_transactions()
            .queries(vec![plan]),
        p,
    ));
    let text = "EXPLAIN ANALYZE UPDATE t SET a = 1 WHERE id = 1";
    let unit = ExecUnit {
        text: Arc::from(text),
        doc_range: 0..text.len(),
        kind: sql_split::exec_range::UnitKind::Statement,
        suspendable: false,
        title: None,
    };
    let dialect =
        hacivat_exec::sqlctx::dialect_ctx(DbmsKind::MySql, None, Some("ONLY_FULL_GROUP_BY"));
    let prepared = match hacivat_exec::prepare_units(
        vec![unit.clone()],
        &dialect,
        &sql_split::ParamSettings::default(),
        &NoValues,
        &ProfileId::new(),
    ) {
        hacivat_exec::Prepared::Ready(p) => p,
        other => panic!("{other:?}"),
    };
    let pool = ClassifyPool::new(1);
    let ctx = hacivat_exec::run_ctx_in(
        DbmsKind::MySql,
        Arc::from(Vec::new()),
        &hacivat_session::session::ClassifierContext {
            lex_modes: Some(Arc::from("ONLY_FULL_GROUP_BY")),
            ..Default::default()
        },
        hacivat_exec::Expect::SingleStatement,
    );
    let reports =
        futures::executor::block_on(pool.classify_units(vec![prepared[0].final_text.clone()], ctx))
            .unwrap_or_else(|never| match never {});
    let conn = ConnSafety {
        engine: DbmsKind::MySql,
        env: Some(EnvTag::Production),
        read_only: false,
        tx_status: hacivat_driver::TxStatus::Idle,
        tx_owner: TxOwner::None,
        tx_mode: TxMode::Auto,
        confirm_unsafe_where: true,
    };
    let mut plan = hacivat_exec::gate(&prepared, &reports, &conn, &SuppressSet::default(), 1);
    assert!(
        matches!(plan.items[0].decision, GateDecision::Confirm { .. }),
        "{:?}",
        plan.items[0]
    );
    assert_eq!(plan.items[0].verdict, UnitVerdict::ExplainAnalyzeRollback);
    let flow = confirm::answer_plan(&mut plan, 0, ConfirmAnswer::Execute, false);
    assert_eq!(flow, confirm::PlanFlow::Go(None));
    assert_eq!(plan.items[0].decision, GateDecision::Run);
    assert_eq!(plan.items[0].verdict, UnitVerdict::ExplainAnalyzeRollback);
    // Sent, the session wraps it: the statement between Hacivat's own BEGIN and ROLLBACK.
    let service =
        hacivat_exec::ExecService::new(hacivat_results::MemoryBudget::new(1 << 26, 1 << 24))
            .with_runtime(vcx.cx.read(|cx| {
                cx.global::<hacivat_app::connections::ConnectionsModel>()
                    .core_runtime()
                    .handle()
            }));
    let ticket = service.run(
        &session,
        hacivat_exec::RunRequest {
            console: 1,
            engine: DbmsKind::MySql,
            units: vec![unit],
            fetch: hacivat_results::FetchPolicy::default(),
            log: hacivat_exec::SessionLog::new(),
            prepared: Some(prepared),
            plan: Some(plan),
            policy: hacivat_session::session::ErrorPolicy::StopOnError,
            run_as_one: false,
            history: None,
            confirm: None,
            context_generation: None,
        },
    );
    let _ = futures::executor::block_on(ticket.done);
    // The session has left the run (its wrap's rollback included) once a later request is
    // answered.
    let barrier = session.submit(hacivat_session::session::Request::Exec {
        sql: "SELECT 'barrier'".to_string(),
        opts: hacivat_driver::ExecOpts::default(),
        origin: hacivat_session::session::ExecOrigin::Internal { idempotent: true },
    });
    let _ = futures::executor::block_on(barrier.reply);
    let calls: Vec<String> = fake
        .call_log()
        .into_iter()
        .map(|c| match c {
            FakeCall::Statement(s) => s,
            FakeCall::Typed(FakeTypedCall::Query(q)) => q,
            other => format!("{other:?}"),
        })
        .collect();
    let at = calls.iter().position(|s| s == text).expect("sent");
    // Its own transaction, or a savepoint in the session's (a production MySQL session is
    // Manual, `autocommit = 0`), rolled back after it.
    assert!(
        calls[..at].iter().any(|s| s.contains("START TRANSACTION")
            || s == "BEGIN"
            || s.starts_with("SAVEPOINT hv_explain")),
        "{calls:?}"
    );
    assert!(
        calls[at..].iter().any(|s| s.starts_with("ROLLBACK")),
        "{calls:?}"
    );
    stop_core_runtime_test(cx);
}

/// T28 carry (M6), the T24 I1 ruling: the one classification context
/// (`orchestrator::classify_ctx_in`) is the session's own: its modes and its search path as
/// the session reads them (`SessionHandle::classifier_context`).
#[gpui::test]
fn the_classify_context_comes_from_the_session(cx: &mut TestAppContext) {
    let fake = FakeDriver::new(DbmsKind::Postgres)
        .tracks_transactions()
        .exec(
            probes("SHOW standard_conforming_strings", "on")
                .into_iter()
                .chain(context_reads("on", "pg_catalog, App, public"))
                .collect(),
        );
    let p = profile("orders-dev", DbmsKind::Postgres, EnvTag::Dev);
    let c = console(cx, p.clone(), fake);
    let sctx = futures::executor::block_on(c.session.classifier_context());
    let ctx = orchestrator::classify_ctx_in(&p, &sctx, hacivat_exec::Expect::SingleStatement);
    assert!(ctx.modes_known, "the session read `on`");
    let path: Vec<&str> = ctx.search_path.iter().map(|s| &**s).collect();
    assert_eq!(path, ["pg_catalog", "app", "public"], "T18's parser");
    stop_core_runtime_test(cx);
}

// ## fix round 1 (I1, I4, I6, M1, M2)

fn as_one(c: &Console) {
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
}

/// I1, the review's example: the console split the text with `standard_conforming_strings`
/// on, but the session now reads it off (a `SET` the console has not probed yet). Run takes the
/// session's context: the units are not what the server would read, so nothing is sent.
#[gpui::test]
fn a_session_reading_in_other_modes_sends_nothing(cx: &mut TestAppContext) {
    let fake = FakeDriver::new(DbmsKind::Postgres)
        .tracks_transactions()
        .exec(
            probes("SHOW standard_conforming_strings", "on")
                .into_iter()
                .chain(context_reads("off", "public"))
                .collect(),
        );
    let c = console(
        cx,
        profile("orders-dev", DbmsKind::Postgres, EnvTag::Dev),
        fake,
    );
    c.set_text("SELECT '\\'\\n' ; DELETE FROM t; --'");
    c.run_at(0);
    c.wait_log("exec-run-context-changed");
    c.barrier();
    assert!(c.queries().is_empty(), "{:?}", c.queries());
    assert!(
        !c.fake.executed().iter().any(|s| s.contains("DELETE")),
        "{:?}",
        c.fake.executed()
    );
    stop_core_runtime_test(cx);
}

/// I1, a shared session: another console's `SET` while this run's confirmation is open
/// changes the session's context; the run is sent only in the context it was classified in, so
/// nothing is sent.
#[gpui::test]
fn a_context_changed_while_confirming_sends_nothing(cx: &mut TestAppContext) {
    let mut reads = context_reads("on", "public");
    reads.truncate(1);
    reads.extend(context_reads("off", "public"));
    let fake = FakeDriver::new(DbmsKind::Postgres)
        .tracks_transactions()
        .exec(
            probes("SHOW standard_conforming_strings", "on")
                .into_iter()
                .chain(reads)
                .collect(),
        );
    let c = console(
        cx,
        profile("orders-dev", DbmsKind::Postgres, EnvTag::Dev),
        fake,
    );
    c.set_text("DELETE FROM a");
    c.run_at(0);
    c.wait_confirm();
    // Another console on the same session.
    let other = c.session.submit(hacivat_session::session::Request::Exec {
        sql: "SET standard_conforming_strings = off".to_string(),
        opts: hacivat_driver::ExecOpts::default(),
        origin: hacivat_session::session::ExecOrigin::User,
    });
    let _ = futures::executor::block_on(other.reply);
    click_button(c.vcx, "confirm-execute");
    c.wait_log("exec-run-context-changed");
    c.barrier();
    assert!(c.queries().is_empty(), "{:?}", c.queries());
    stop_core_runtime_test(cx);
}

/// I6.1, the plan's review probe: a confirmation cancelled, or closed, sends nothing of the run.
#[gpui::test]
fn a_cancelled_or_closed_confirmation_sends_nothing(cx: &mut TestAppContext) {
    let mut c = dev(cx);
    c.set_text("SELECT 1;\nDELETE FROM a;\nDELETE FROM b;");
    c.command("editor.run_3");
    c.wait_confirm();
    click_button(c.vcx, "confirm-cancel");
    c.wait_log("exec-confirm-cancelled");
    c.barrier();
    assert!(c.queries().is_empty(), "{:?}", c.queries());
    assert!(!c.confirm_open());
    c.command("editor.run_3");
    c.wait_confirm();
    c.keys("escape");
    wait_until(c.vcx, "the dialog to close", |vcx| {
        !vcx.cx.read(confirm::confirm_open)
    });
    c.barrier();
    assert!(c.queries().is_empty(), "{:?}", c.queries());
    let cancelled = c
        .log_ids()
        .iter()
        .filter(|id| **id == "exec-confirm-cancelled")
        .count();
    assert_eq!(cancelled, 2, "{:?}", c.log_ids());
    stop_core_runtime_test(cx);
}

/// I6.2: Run as one never sends a unit that needs its own transaction or the D-062 wrap.
#[gpui::test]
fn run_as_one_refuses_a_wrapped_explain_analyze(cx: &mut TestAppContext) {
    let c = dev(cx);
    as_one(&c);
    let text = "EXPLAIN ANALYZE UPDATE t SET a = 1 WHERE id = 1;";
    c.set_text(text);
    c.select(0..text.len());
    c.command("editor.run");
    c.wait_log("exec-gate-run-as-one-refused");
    c.barrier();
    assert!(c.queries().is_empty(), "{:?}", c.queries());
    stop_core_runtime_test(cx);
}

/// I4: a refused run sends nothing, and every unit says so: the refused one why, the others
/// which statement refused the run.
#[gpui::test]
fn every_unit_of_a_refused_run_says_it_was_not_sent(cx: &mut TestAppContext) {
    let mut p = profile("orders-ro", DbmsKind::Postgres, EnvTag::Dev);
    p.read_only = true;
    let c = console(cx, p, pg());
    // The statement that may change the context comes last: the units after one would be
    // classified in an unknown context (sql-safety's `classify_units`), refused too.
    c.set_text("SELECT 1;\nSELECT 2;\nDELETE FROM t WHERE id = 1;");
    c.command("editor.run_3");
    c.wait_log("exec-gate-run-refused");
    c.barrier();
    assert!(c.queries().is_empty(), "{:?}", c.queries());
    let ids = c.log_ids();
    assert!(ids.contains(&"exec-gate-read-only"), "{ids:?}");
    assert_eq!(
        ids.iter()
            .filter(|id| **id == "exec-gate-run-refused")
            .count(),
        2,
        "{ids:?}"
    );
    let named: Vec<String> = c.vcx.cx.read(|cx| {
        Consoles::get(cx, c.id)
            .map(|s| {
                s.log
                    .lines()
                    .iter()
                    .filter(|l| l.msg.fluent_id() == "exec-gate-run-refused")
                    .map(|l| format!("{:?}", l.msg))
                    .collect()
            })
            .unwrap_or_default()
    });
    assert!(
        named.iter().all(|m| m.contains("Str(\"3\")")),
        "each names statement 3: {named:?}"
    );
    let marks = c.marks();
    assert_eq!(marks.len(), 3, "{marks:?}");
    assert!(
        marks
            .iter()
            .all(|m| matches!(m, ExecMarkState::Failed { sqlstate: None })),
        "{marks:?}"
    );
    stop_core_runtime_test(cx);
}

/// M1: a unit run as one carries no bind values: refused before any prompt.
#[gpui::test]
fn run_as_one_with_parameters_is_refused_before_the_prompt(cx: &mut TestAppContext) {
    let c = dev(cx);
    as_one(&c);
    c.set_text("SELECT :a;\nSELECT 2;");
    c.select(0..19);
    c.command("editor.run");
    c.wait_log("exec-param-run-as-one");
    c.barrier();
    assert!(
        c.vcx.cx.read(params_prompt::open_prompt).is_none(),
        "no prompt"
    );
    assert!(c.queries().is_empty(), "{:?}", c.queries());
    stop_core_runtime_test(cx);
}

/// M2: the console's "As one" policy never sends a selection to a read-only session.
#[gpui::test]
fn run_as_one_on_a_read_only_session_is_refused(cx: &mut TestAppContext) {
    let mut p = profile("orders-ro", DbmsKind::Postgres, EnvTag::Dev);
    p.read_only = true;
    let c = console(cx, p, pg());
    as_one(&c);
    c.set_text("SELECT 1;\nSELECT 2;");
    c.select(0..19);
    c.command("editor.run");
    c.wait_log("exec-gate-run-as-one-read-only");
    c.barrier();
    assert!(c.queries().is_empty(), "{:?}", c.queries());
    stop_core_runtime_test(cx);
}

// ## fix round 2 (R1-C1)

/// A typed query that fails at once with a server error.
fn failing(prefix: &'static str) -> hacivat_driver::fake::FakeQuery {
    hacivat_driver::fake::FakeQuery {
        prefix,
        sets: vec![hacivat_driver::fake::FakeResultSet {
            columns: vec![(
                "x",
                LogicalType::Text {
                    max_chars: None,
                    fixed: false,
                },
            )],
            rows: hacivat_driver::fake::FakeRows::Literal(vec![vec![hacivat_values::Value::Text(
                "x".into(),
            )]]),
        }],
        fail_after: Some((
            0,
            hacivat_driver::RawError {
                kind: hacivat_driver::RawErrorKind::Db,
                server: Some(hacivat_driver::ServerMessage {
                    sqlstate: Some("42000".into()),
                    vendor_code: Some(1049),
                    severity: Some("ERROR".into()),
                    text: "Unknown database 'app'".into(),
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

/// R1-C1: `USE app` fails (the database stays `sys`), so `SELECT * FROM session`, classified
/// as a read of `app.session`, must never run: it would read `sys.session`. The run is sent
/// with `StopOnError` whatever the error-policy setting says, so the error bar never offers
/// Ignore or Skip for it; a run without a carried switch still asks.
#[gpui::test]
fn a_failed_use_never_runs_what_was_classified_under_it(cx: &mut TestAppContext) {
    let fake = mysql().queries(vec![failing("USE")]);
    let c = console(cx, profile("shop-dev", DbmsKind::MySql, EnvTag::Dev), fake);
    c.set_text("USE app;\nSELECT * FROM session;");
    c.command("editor.run_3");
    wait_until(c.vcx, "the run to end", |vcx| {
        vcx.cx
            .read(|cx| hacivat_app::exec_ui::last_run_ended(c.id, cx))
    });
    c.barrier();
    assert_eq!(c.queries(), ["USE app"], "nothing after the failed switch");
    assert_eq!(
        c.vcx
            .cx
            .read(|cx| hacivat_app::exec_ui::last_run_policy(c.id, cx)),
        Some(hacivat_session::session::ErrorPolicy::StopOnError),
        "no Ignore or Skip on the error bar"
    );
    // The control: two reads without a switch ask after a failure (the setting's default).
    c.set_text("SELECT 1;\nSELECT 2;");
    c.command("editor.run_3");
    wait_until(c.vcx, "the second run", |vcx| {
        vcx.cx.read(|cx| {
            hacivat_app::exec_ui::last_run_policy(c.id, cx)
                == Some(hacivat_session::session::ErrorPolicy::Ask)
        })
    });
    stop_core_runtime_test(cx);
}

/// R1-C1, PostgreSQL: a literal `SET search_path` is carried too (always the strictest reading,
/// so a failed one is harmless), and the run still stops at its failure.
#[gpui::test]
fn a_failed_set_search_path_stops_the_run(cx: &mut TestAppContext) {
    let fake = pg().queries(vec![failing("SET search_path")]);
    let c = console(
        cx,
        profile("orders-dev", DbmsKind::Postgres, EnvTag::Dev),
        fake,
    );
    // `pg_catalog` no longer first: a context change (with it first, the T18 harmless-`SET`
    // list keeps the old reading, which is the same for every builtin).
    c.set_text("SET search_path TO app, pg_catalog;\nSELECT lower('a') FROM t;");
    c.command("editor.run_3");
    wait_until(c.vcx, "the run to end", |vcx| {
        vcx.cx
            .read(|cx| hacivat_app::exec_ui::last_run_ended(c.id, cx))
    });
    c.barrier();
    assert_eq!(c.queries(), ["SET search_path TO app, pg_catalog"]);
    assert_eq!(
        c.vcx
            .cx
            .read(|cx| hacivat_app::exec_ui::last_run_policy(c.id, cx)),
        Some(hacivat_session::session::ErrorPolicy::StopOnError)
    );
    stop_core_runtime_test(cx);
}

/// No value remembered (the gate test asks for none).
struct NoValues;

impl hacivat_exec::ParamValueStore for NoValues {
    fn get(&self, _: &ProfileId, _: &str) -> Option<BindValue> {
        None
    }
    fn put(&self, _: &ProfileId, _: &str, _: BindValue) {}
}
