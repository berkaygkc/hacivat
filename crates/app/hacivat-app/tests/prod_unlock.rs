//! M3b Task 26: production consoles are writable behind confirmations (R-M3-6 REVISED ends;
//! R-M3-29, R-M3-52, R-M3-78, R-M3-81; D-036; P-M3b-18). Every console here is bound through the
//! product's own wiring (`ConnectionsModel` connect under the boot configuration, Task 32's
//! host, Task 24's Run pipeline) over a `FakeDriver` that dials itself; every wait is on a
//! condition.
//!
//! - A production console opens a read-write session in Manual mode (D-036), with no production
//!   read-only notice left anywhere (R-M3-81).
//! - Nothing of a production write is sent before its confirmation (`DELETE FROM t`: the typed
//!   name, P-M3b-18), and a DML runs in the user's transaction under the savepoint and is kept
//!   only after Keep (`[BEGIN, SAVEPOINT hv_confirm, UPDATE …]`, then the Keep/Undo prompt).
//! - Closing a dialog never sends and never keeps: Escape cancels a confirmation and undoes a
//!   confirm-in-transaction statement.
//! - A profile retagged to production while its console is open gets confirmations from the
//!   next Run, with no forced reconnect (R-M3-78); marking it read-only still reconnects
//!   read-only (M3a's `pending_read_only` flow).
//! - A production Run whose session context read timed out sends nothing (carry MUST 2).
//! - The production policy's flag is gone from every crate, and no app path sends a unit
//!   without a gate decision (the user's `Request::Exec`, a plan-less run).

mod support;

use std::sync::Arc;
use std::time::Duration;

use gpui::{Entity, TestAppContext, VisualTestContext};
use hacivat_app::connections::ConnectionsModel;
use hacivat_app::connections::prompt_kit::parts;
use hacivat_app::console::{ConsoleId, Consoles};
use hacivat_app::exec_ui::{commit_prompt, confirm};
use hacivat_driver::ExecSummary;
use hacivat_driver::fake::{FakeCall, FakeDriver, FakeExec, FakeTypedCall};
use hacivat_proto::{AccessLabel, DbmsKind, EnvTag};
use hacivat_session::LiveState;
use hacivat_session::session::{SessionHandle, TxMode};
use hacivat_ui_editor::EditorView;
use support::*;

const PROD: &str = "orders-prod";

/// The session's own context read (T18's constant text), answering `on` and the default
/// search path, as a server does (`n` reads).
fn context_reads(n: usize) -> Vec<FakeExec> {
    (0..n)
        .map(|_| FakeExec {
            prefix: "SELECT pg_catalog.current_setting('standard_conforming_strings')",
            result: Ok(ExecSummary {
                rows: vec![vec![
                    Some("on".to_string()),
                    Some("\"$user\", public".to_string()),
                ]],
                row_count: 1,
                ..ExecSummary::default()
            }),
            delay: Duration::ZERO,
            tx_after: None,
            panics: false,
        })
        .collect()
}

/// A PostgreSQL fake that answers the console's SQL-context probe and Run's context reads and
/// follows transactions; `first` comes before them (a scripted entry is used once, first
/// match first).
fn pg_fake(first: Vec<FakeExec>) -> FakeDriver {
    // The console's SQL-context probe, as `support::answers_probe` scripts it (`exec` replaces
    // a script, so the whole script is built here).
    let probe = (0..32).map(|_| FakeExec {
        prefix: "SHOW standard_conforming_strings",
        result: Ok(ExecSummary {
            rows: vec![vec![Some("on".to_string())]],
            row_count: 1,
            ..ExecSummary::default()
        }),
        delay: Duration::ZERO,
        tx_after: None,
        panics: false,
    });
    let script = probe.chain(first).chain(context_reads(16)).collect();
    FakeDriver::new(DbmsKind::Postgres)
        .tracks_transactions()
        .exec(script)
}

/// ## m3b-t26-prod-unlock (fix round 1, B-M5): whether any call of the fake's whole log (typed
/// queries, simple statements, batches, executes) carries `needle`; "nothing sent" checks read
/// this, never the typed log alone.
fn ever_sent(f: &FakeDriver, needle: &str) -> bool {
    f.call_log()
        .iter()
        .any(|c| format!("{c:?}").contains(needle))
}

/// The shell with profile `name` (`env`) connected over `fake` and its `Console 1` bound with
/// its SQL context read.
fn bound(
    cx: &mut TestAppContext,
    name: &str,
    env: EnvTag,
    fake: FakeDriver,
) -> &'static mut VisualTestContext {
    let vcx = shell_with_fake(cx, "en", fake);
    in_window(vcx, |window, _| window.activate_window());
    add_profile(vcx, test_profile(name, Some(env), "10.0.0.7"));
    connect(vcx, name);
    wait_until(vcx, "the connect", |vcx| {
        let state = live(vcx, name);
        assert!(!matches!(state, LiveState::Failed(_)), "{state:?}");
        matches!(state, LiveState::Connected(_))
    });
    wait_until(vcx, "Console 1 to bind and read its SQL context", |vcx| {
        active_console(vcx)
            .is_some_and(|c| vcx.cx.read(|cx| hacivat_app::exec_ui::connected(c, cx)))
    });
    vcx
}

/// A production console (the boot configuration) with `text` in its editor; its fake.
fn prod_console(
    cx: &mut TestAppContext,
    text: &str,
) -> (&'static mut VisualTestContext, Arc<FakeDriver>) {
    prod_console_over(cx, text, pg_fake(Vec::new()))
}

fn prod_console_over(
    cx: &mut TestAppContext,
    text: &str,
    fake: FakeDriver,
) -> (&'static mut VisualTestContext, Arc<FakeDriver>) {
    let vcx = bound(cx, PROD, EnvTag::Production, fake);
    set_text(vcx, text);
    let f = support::fake(vcx);
    (vcx, f)
}

/// The console of the focused pane's active tab.
fn active_console(vcx: &VisualTestContext) -> Option<ConsoleId> {
    let tab = with_ws(vcx, |ws, _, _| ws.active_tab())?;
    vcx.cx.read(|cx| Consoles::by_tab(cx, tab))
}

fn console(vcx: &VisualTestContext) -> ConsoleId {
    active_console(vcx).expect("the bound console")
}

fn editor(vcx: &VisualTestContext) -> Entity<EditorView> {
    let id = console(vcx);
    vcx.cx.read(|cx| {
        Consoles::get(cx, id)
            .and_then(|s| s.editor.clone())
            .expect("the console's editor")
    })
}

fn session(vcx: &VisualTestContext) -> SessionHandle {
    let id = console(vcx);
    vcx.cx
        .read(|cx| Consoles::get(cx, id).and_then(|s| s.session.clone()))
        .expect("the console's session")
}

fn set_text(vcx: &VisualTestContext, text: &str) {
    let editor = editor(vcx);
    let text = text.to_string();
    vcx.cx
        .update(|cx| editor.update(cx, |e, cx| e.set_text(&text, cx)));
    vcx.run_until_parked();
}

/// Run with the caret at byte `at` of the console's editor.
fn run_at(vcx: &VisualTestContext, at: usize) {
    let editor = editor(vcx);
    vcx.cx
        .update(|cx| editor.update(cx, |e, cx| e.select_range(at..at, cx)));
    in_window(vcx, |window, cx| {
        editor.update(cx, |e, cx| assert!(e.run_command("editor.run", window, cx)));
    });
    vcx.run_until_parked();
}

fn confirm_dialog_open(vcx: &VisualTestContext) -> bool {
    vcx.cx.read(confirm::confirm_open)
}

fn commit_prompt_open(vcx: &VisualTestContext) -> bool {
    layer_ids(vcx).contains(&commit_prompt::COMMIT_LAYER_ID)
}

/// The console's log message IDs.
fn log_ids(vcx: &VisualTestContext) -> Vec<&'static str> {
    let id = console(vcx);
    vcx.cx.read(|cx| {
        Consoles::get(cx, id)
            .map(|s| s.log.lines().iter().map(|l| l.msg.fluent_id()).collect())
            .unwrap_or_default()
    })
}

/// What the session sent from `mark` on, without Hacivat's own reads (the context read, the
/// SQL-context probe and the schema probe), which are not part of the run.
fn sent_since(f: &FakeDriver, mark: usize) -> Vec<FakeCall> {
    f.call_log()[mark..]
        .iter()
        .filter(|c| match c {
            FakeCall::Statement(s) => {
                !(s.starts_with("SELECT pg_catalog.current_setting")
                    || s.starts_with("SHOW ")
                    || s == hacivat_session::session::schema_probe_sql(DbmsKind::Postgres))
            }
            _ => true,
        })
        .cloned()
        .collect()
}

/// Every text the last frame drew.
fn drawn(vcx: &mut VisualTestContext) -> Vec<String> {
    enable_registry(vcx);
    in_window(vcx, |window, _| window.refresh());
    vcx.run_until_parked();
    registry_frame(vcx)
        .into_iter()
        .map(|r| r.text.to_string())
        .collect()
}

/// `s` without Fluent's isolation marks.
fn plain(s: &str) -> String {
    s.chars()
        .filter(|c| !matches!(c, '\u{2068}' | '\u{2069}'))
        .collect()
}

/// The confirmation's run buttons arm after a moment (T21 fix round 1, I1).
fn arm(vcx: &mut VisualTestContext) {
    vcx.executor()
        .advance_clock(confirm::ARM_DELAY + Duration::from_millis(50));
    vcx.run_until_parked();
}

// ---------------------------------------------------------------------------------------------

#[gpui::test]
fn production_console_is_read_write_in_manual_mode(cx: &mut TestAppContext) {
    let (vcx, f) = prod_console(cx, "SELECT 1");
    let s = session(vcx);
    let snap = s.state().borrow().clone();
    assert!(!snap.read_only, "read-write (R-M3-6 REVISED ends)");
    assert_eq!(snap.access, AccessLabel::ReadWrite);
    assert_eq!(snap.tx.mode, TxMode::Manual, "D-036");
    assert!(
        f.requests().iter().all(|r| !r.read_only),
        "every connect read-write: {:?}",
        f.requests()
    );
    // The environment still shows; the status bar says production and read-write.
    let status = status_bar_text(vcx);
    assert!(status.contains("production · read-write"), "{status}");
    // A read runs at once, without a dialog.
    run_at(vcx, 0);
    wait_until(vcx, "the SELECT", |_| {
        f.typed_log()
            .iter()
            .any(|c| matches!(c, FakeTypedCall::Query(q) if q == "SELECT 1"))
    });
    assert!(!confirm_dialog_open(vcx) && !commit_prompt_open(vcx));
    stop_core_runtime_test(cx);
}

#[gpui::test]
fn production_delete_without_where_asks_before_anything_is_sent(cx: &mut TestAppContext) {
    let (vcx, f) = prod_console(cx, "DELETE FROM t;");
    run_at(vcx, 0);
    wait_until(vcx, "the confirmation", confirm_dialog_open);
    // DmlUnsafe → Confirm before send (P-M3b-18).
    assert!(!ever_sent(&f, "DELETE"), "{:?}", f.call_log());
    assert!(confirm_dialog_open(vcx));
    stop_core_runtime_test(cx);
}

#[gpui::test]
fn production_update_runs_in_a_savepoint_and_asks_keep_or_undo(cx: &mut TestAppContext) {
    let (vcx, f) = prod_console(cx, "UPDATE t SET a=1 WHERE id=3");
    let mark = f.call_log().len();
    run_at(vcx, 0);
    wait_until(vcx, "the Keep/Undo prompt", commit_prompt_open);
    // Manual default: the user's transaction, the savepoint, the statement; nothing kept.
    assert_eq!(
        sent_since(&f, mark),
        [
            FakeCall::Statement("BEGIN".into()),
            FakeCall::Statement("SAVEPOINT hv_confirm".into()),
            FakeCall::Typed(FakeTypedCall::Query("UPDATE t SET a=1 WHERE id=3".into())),
        ]
    );
    assert!(
        !confirm_dialog_open(vcx),
        "a DML asks after it ran, in its savepoint"
    );
    let texts: Vec<String> = drawn(vcx).iter().map(|t| plain(t)).collect();
    assert!(
        texts.iter().any(|t| t == "Keep") && texts.iter().any(|t| t == "Undo"),
        "the savepoint form: {texts:?}"
    );
    stop_core_runtime_test(cx);
}

#[gpui::test]
fn production_delete_without_where_needs_the_typed_name(cx: &mut TestAppContext) {
    let (vcx, f) = prod_console(cx, "DELETE FROM t;");
    run_at(vcx, 0);
    wait_until(vcx, "the confirmation", confirm_dialog_open);
    arm(vcx);
    let texts: Vec<String> = drawn(vcx).iter().map(|t| plain(t)).collect();
    assert!(
        texts.windows(3).any(|w| w == ["Type ", "t", " to run it"]),
        "the typed name: {texts:?}"
    );
    // Until the name is typed, Execute stays off; nothing is sent.
    let buttons = vcx.cx.read(confirm::open_dialog_buttons);
    let execute = buttons
        .iter()
        .find(|b| b.answer == hacivat_exec::ConfirmAnswer::Execute)
        .expect("an Execute button");
    assert!(!execute.enabled, "{buttons:?}");
    assert!(
        buttons
            .iter()
            .all(|b| b.answer != hacivat_exec::ConfirmAnswer::ExecuteAndSuppress),
        "never suppressible with a typed name: {buttons:?}"
    );
    // The wrong name keeps it off; the exact name turns it on.
    let field = vcx
        .cx
        .read(|cx| parts::focus(cx, "confirm-type"))
        .expect("the typed-name field");
    in_window(vcx, |window, cx| window.focus(&field, cx));
    vcx.run_until_parked();
    vcx.simulate_input("T");
    vcx.run_until_parked();
    let off = vcx.cx.read(confirm::open_dialog_buttons);
    assert!(
        off.iter()
            .any(|b| b.answer == hacivat_exec::ConfirmAnswer::Execute && !b.enabled),
        "letter case counts: {off:?}"
    );
    vcx.simulate_keystrokes("backspace");
    vcx.simulate_input("t");
    vcx.run_until_parked();
    let on = vcx.cx.read(confirm::open_dialog_buttons);
    assert!(
        on.iter()
            .any(|b| b.answer == hacivat_exec::ConfirmAnswer::Execute && b.enabled),
        "{on:?}"
    );
    assert!(
        !ever_sent(&f, "DELETE"),
        "nothing sent before Execute: {:?}",
        f.call_log()
    );
    stop_core_runtime_test(cx);
}

/// No dialog close sends or keeps anything: Escape on a confirmation cancels the run before it
/// is sent; Escape on the Keep/Undo prompt undoes the statement (never `RELEASE`, never
/// `COMMIT`).
#[gpui::test]
fn closing_a_dialog_never_sends_or_keeps(cx: &mut TestAppContext) {
    let (vcx, f) = prod_console(cx, "DELETE FROM t;");
    run_at(vcx, 0);
    wait_until(vcx, "the confirmation", confirm_dialog_open);
    arm(vcx);
    vcx.simulate_keystrokes("escape");
    vcx.run_until_parked();
    wait_until(vcx, "the confirmation to close", |vcx| {
        !confirm_dialog_open(vcx)
    });
    assert!(
        !ever_sent(&f, "DELETE"),
        "Cancel sent nothing: {:?}",
        f.call_log()
    );
    assert!(log_ids(vcx).contains(&"exec-confirm-cancelled"));

    set_text(vcx, "UPDATE t SET a=1 WHERE id=3");
    let mark = f.call_log().len();
    run_at(vcx, 0);
    wait_until(vcx, "the Keep/Undo prompt", commit_prompt_open);
    vcx.simulate_keystrokes("escape");
    vcx.run_until_parked();
    wait_until(vcx, "the Undo", |_| {
        sent_since(&f, mark).contains(&FakeCall::Statement(
            "ROLLBACK TO SAVEPOINT hv_confirm".into(),
        ))
    });
    wait_until(vcx, "the prompt to close", |vcx| !commit_prompt_open(vcx));
    let sent = sent_since(&f, mark);
    let at = |sql: &str| {
        sent.iter()
            .position(|c| *c == FakeCall::Statement(sql.into()))
    };
    let undo = at("ROLLBACK TO SAVEPOINT hv_confirm").expect("undone");
    // The savepoint is released only after the statement was rolled back to it.
    assert!(
        at("RELEASE SAVEPOINT hv_confirm").is_none_or(|r| r > undo),
        "{sent:?}"
    );
    assert!(
        !f.executed().iter().any(|s| s.contains("COMMIT")),
        "{:?}",
        f.executed()
    );
    stop_core_runtime_test(cx);
}

#[gpui::test]
fn retag_to_production_applies_confirmations_on_the_next_run(cx: &mut TestAppContext) {
    let vcx = bound(cx, "orders", EnvTag::Dev, pg_fake(Vec::new()));
    let f = support::fake(vcx);
    let before = session(vcx);
    let id = console(vcx);
    // Off production the bounded UPDATE runs at once (no dialog, no savepoint).
    set_text(vcx, "UPDATE t SET a=1 WHERE id=3");
    run_at(vcx, 0);
    wait_until(vcx, "the dev UPDATE", |_| {
        f.typed_log()
            .iter()
            .any(|c| matches!(c, FakeTypedCall::Query(q) if q.starts_with("UPDATE")))
    });
    assert!(!commit_prompt_open(vcx) && !confirm_dialog_open(vcx));
    wait_until(vcx, "the run to end", |vcx| {
        vcx.cx
            .read(|cx| hacivat_app::exec_ui::last_run_ended(id, cx))
    });

    // The profile is tagged production while its console is open.
    let pid = profile_id(vcx, "orders");
    let mut p = vcx
        .cx
        .read(|cx| (*cx.global::<ConnectionsModel>().get(pid).expect("saved")).clone());
    p.environment = Some(EnvTag::Production);
    save_draft(vcx, hacivat_session::profile::ProfileDraft::edit(p));
    // No forced reconnect: the same session, its mode unchanged (R-M3-78).
    assert_eq!(session(vcx).id(), before.id());
    assert_eq!(before.state().borrow().tx.mode, TxMode::Auto);
    assert!(matches!(live(vcx, "orders"), LiveState::Connected(_)));

    // The next Run asks: the same UPDATE now runs in a transaction and waits for the answer.
    let mark = f.call_log().len();
    run_at(vcx, 0);
    wait_until(vcx, "the confirm-in-transaction prompt", commit_prompt_open);
    let sent = sent_since(&f, mark);
    assert!(
        !sent
            .iter()
            .any(|c| matches!(c, FakeCall::Statement(s) if s.contains("COMMIT"))),
        "nothing kept before the answer: {sent:?}"
    );
    assert!(
        sent.contains(&FakeCall::Typed(FakeTypedCall::Query(
            "UPDATE t SET a=1 WHERE id=3".into()
        ))),
        "{sent:?}"
    );
    // And an unbounded DELETE asks before anything is sent.
    vcx.simulate_keystrokes("escape");
    vcx.run_until_parked();
    wait_until(vcx, "the prompt to close", |vcx| !commit_prompt_open(vcx));
    wait_until(vcx, "the run to end", |vcx| {
        vcx.cx
            .read(|cx| hacivat_app::exec_ui::last_run_ended(id, cx))
    });
    set_text(vcx, "DELETE FROM t;");
    run_at(vcx, 0);
    wait_until(vcx, "the confirmation", confirm_dialog_open);
    assert!(
        !ever_sent(&f, "DELETE"),
        "nothing sent before the answer: {:?}",
        f.call_log()
    );
    stop_core_runtime_test(cx);
}

/// ## m3b-t26-prod-unlock (fix round 1, B-M4/A-M4): a profile retagged to production while a
/// confirmation of its console is open. The answer sends nothing: the send reads the live tag
/// again, finds it is not the tag the plan was gated under, and refuses the run.
#[gpui::test]
fn a_retag_while_a_confirmation_is_open_sends_nothing(cx: &mut TestAppContext) {
    let vcx = bound(cx, "orders", EnvTag::Dev, pg_fake(Vec::new()));
    let f = support::fake(vcx);
    set_text(vcx, "DELETE FROM t;");
    run_at(vcx, 0);
    wait_until(vcx, "the confirmation", confirm_dialog_open);
    arm(vcx);
    let execute = vcx
        .cx
        .read(confirm::open_dialog_buttons)
        .into_iter()
        .find(|b| b.answer == hacivat_exec::ConfirmAnswer::Execute && b.enabled)
        .expect("Execute, with no name to type off production");
    // The profile is tagged production while the dialog is open.
    let pid = profile_id(vcx, "orders");
    let mut p = vcx
        .cx
        .read(|cx| (*cx.global::<ConnectionsModel>().get(pid).expect("saved")).clone());
    p.environment = Some(EnvTag::Production);
    save_draft(vcx, hacivat_session::profile::ProfileDraft::edit(p));
    assert!(confirm_dialog_open(vcx), "the dialog is still open");
    click(
        vcx,
        Box::leak(format!("button-{}", execute.id).into_boxed_str()),
    );
    wait_until(vcx, "the refusal", |vcx| {
        log_ids(vcx).contains(&"exec-run-connection-changed")
    });
    assert!(!ever_sent(&f, "DELETE"), "nothing sent: {:?}", f.call_log());
    stop_core_runtime_test(cx);
}

#[gpui::test]
fn read_only_toggle_still_reconnects_read_only(cx: &mut TestAppContext) {
    let vcx = bound(cx, "orders", EnvTag::Production, pg_fake(Vec::new()));
    let rw = session(vcx);
    assert!(!rw.state().borrow().read_only);
    let id = console(vcx);
    let pid = profile_id(vcx, "orders");
    let mut p = vcx
        .cx
        .read(|cx| (*cx.global::<ConnectionsModel>().get(pid).expect("saved")).clone());
    p.read_only = true;
    save_draft(vcx, hacivat_session::profile::ProfileDraft::edit(p));
    // At once: the read-write session is gone from the console, and a read-only one comes.
    let now = vcx
        .cx
        .read(|cx| Consoles::get(cx, id).and_then(|s| s.session.clone()));
    assert!(
        now.is_none_or(|s| s.id() != rw.id()),
        "the read-write session does not survive the read-only mark"
    );
    wait_until(vcx, "a read-only session", |vcx| {
        vcx.cx
            .read(|cx| Consoles::get(cx, id).and_then(|s| s.session.clone()))
            .is_some_and(|s| s.state().borrow().read_only)
    });
    stop_core_runtime_test(cx);
}

#[gpui::test]
fn no_production_read_only_notice_remains(cx: &mut TestAppContext) {
    let (vcx, _f) = prod_console(cx, "SELECT 1");
    let texts = drawn(vcx).join("\n");
    for gone in [
        "Production is read-only in this version",
        "Why this console is read-only",
        "doesn't have them yet",
    ] {
        assert!(!texts.contains(gone), "{gone}: {texts}");
    }
    for id in ["button-console-prod-notice", "button-console-prod-short"] {
        assert!(vcx.debug_bounds(id).is_none(), "{id}");
    }
    assert!(
        vcx.debug_bounds("console-env-swatch").is_some(),
        "the environment mark stays"
    );
    // The strings are gone from both locales' bundles too.
    let locales = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../locales");
    for lang in ["en", "tr"] {
        let ftl = std::fs::read_to_string(locales.join(lang).join("hacivat-ui.ftl"))
            .expect("the UI bundle");
        assert!(!ftl.contains("hv-console-prod-"), "{lang}");
    }
    stop_core_runtime_test(cx);
}

/// Carry MUST 2: on production a Run whose session context read timed out (no context
/// generation) sends nothing and says why.
#[gpui::test]
fn production_run_refuses_when_its_context_read_times_out(cx: &mut TestAppContext) {
    let slow = FakeExec {
        prefix: "SELECT pg_catalog.current_setting('standard_conforming_strings')",
        result: Ok(ExecSummary::default()),
        delay: Duration::from_secs(3600),
        tx_after: None,
        panics: false,
    };
    let (vcx, f) = prod_console_over(cx, "SELECT 1", pg_fake(vec![slow]));
    run_at(vcx, 0);
    // Run waits `CONTEXT_WAIT` (10 s of the executor's clock) for each of its two reads: the
    // clock moves on until the refusal is logged (a condition wait; fix round 1, A-M7).
    wait_until(vcx, "the refusal", |vcx| {
        vcx.executor().advance_clock(Duration::from_secs(11));
        log_ids(vcx).contains(&"exec-run-context-unconfirmed")
    });
    // Fix round 1 (B-M5): nothing of it in the whole call log.
    assert!(
        !ever_sent(&f, "\"SELECT 1\""),
        "nothing sent: {:?}",
        f.call_log()
    );
    stop_core_runtime_test(cx);
}

/// The grep of the plan's Step 1: no identifier of the production policy's flag (the words
/// production, read, only joined by underscores) anywhere in `crates/`.
#[test]
fn no_production_read_only_symbol_remains() {
    let needle = ["production", "read", "only"].join("_");
    let crates = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut found = Vec::new();
    let mut stack = vec![crates];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).expect("a readable directory") {
            let path = entry.expect("an entry").path();
            if path.is_dir() {
                if path.file_name().is_some_and(|n| n == "target") {
                    continue;
                }
                stack.push(path);
            } else if path
                .extension()
                .is_some_and(|e| e == "rs" || e == "toml" || e == "ftl")
            {
                let Ok(text) = std::fs::read_to_string(&path) else {
                    continue;
                };
                // This file's two test names (the plan's, binding) hold the words; nothing
                // else in it may.
                let text = if path.file_name().is_some_and(|n| n == "prod_unlock.rs") {
                    text.replace(&format!("fn no_{needle}_notice_remains"), "")
                        .replace(&format!("fn no_{needle}_symbol_remains"), "")
                } else {
                    text
                };
                if text.contains(&needle) {
                    found.push(path.display().to_string());
                }
            }
        }
    }
    assert!(found.is_empty(), "{found:#?}");
}

/// The probe "a production non-Read unit reaches the session without a gate decision": the
/// app never sends a user statement around the gate. It has no `Request::Exec` of the user's
/// (only Hacivat's own idempotent reads), builds no `RunSpec` of its own, and the one
/// plan-less run (`plan: None`, the run service's M3a path) is the error bar's test seam,
/// compiled only with `test-support`.
#[test]
fn no_ungated_send_path_in_the_app() {
    let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut stack = vec![src.clone()];
    let mut plan_less = Vec::new();
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).expect("a readable directory") {
            let path = entry.expect("an entry").path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if path.extension().is_none_or(|e| e != "rs") {
                continue;
            }
            let text = std::fs::read_to_string(&path).expect("a source file");
            let rel = path.strip_prefix(&src).unwrap().display().to_string();
            assert!(
                !text.contains(&["ExecOrigin", "User"].join("::")),
                "{rel}: a user statement outside Run"
            );
            assert!(
                !text.contains(&["RunSpec", " {"].concat())
                    && !text.contains(&["Request", "::Run("].concat()),
                "{rel}: a run built outside the run service"
            );
            if text.contains(&["plan", ": None"].concat()) {
                plan_less.push((rel, text));
            }
        }
    }
    assert_eq!(
        plan_less
            .iter()
            .map(|(r, _)| r.as_str())
            .collect::<Vec<_>>(),
        [std::path::Path::new("exec_ui")
            .join("error_bar.rs")
            .display()
            .to_string()],
        "the only plan-less run"
    );
    // ## m3b-t26-prod-unlock (fix round 1, A-M2/A-M3): the run service refuses a plan-less
    // request unless it was built `with_plan_less_runs`, and the app builds it so only in a
    // `test-support` build.
    let opt_in = ["with_plan", "_less_runs"].concat();
    let mut stack = vec![src.clone()];
    let mut opt_ins = 0;
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).expect("a readable directory") {
            let path = entry.expect("an entry").path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if path.extension().is_none_or(|e| e != "rs") {
                continue;
            }
            let text = std::fs::read_to_string(&path).expect("a source file");
            for (at, _) in text.match_indices(&opt_in) {
                opt_ins += 1;
                let before: Vec<&str> = text[..at].lines().rev().take(3).collect();
                assert!(
                    before
                        .iter()
                        .any(|l| l.trim() == "#[cfg(feature = \"test-support\")]"),
                    "{}: the plan-less opt-in outside test-support",
                    path.display()
                );
            }
        }
    }
    assert_eq!(opt_ins, 1, "the app opts in once, for its test seam");
    let (_, text) = &plan_less[0];
    let at = text.find(&["plan", ": None"].concat()).unwrap();
    let seam = text[..at]
        .rfind("pub fn run_asking")
        .expect("inside the test seam");
    assert!(
        text[..seam]
            .trim_end()
            .ends_with(&["#[cfg(feature = \"test-support\")]"].concat()),
        "the seam is test-support only"
    );
}
