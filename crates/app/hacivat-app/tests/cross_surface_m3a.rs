//! M3a Task 35: flows across the M3a surfaces, end to end through the product's own wiring: a
//! profile connected through `ConnectionsModel` binds the focused `Console 1` (Task 32's host
//! opens its session; Task 33's run UI attaches), Run goes through the editor, the results
//! tile, the grid, the status bar and the close guard. The server is a `FakeDriver` that
//! dials itself (no socket); every wait is on a condition.
//!
//! - run → rows → copy: the grid shows the rows; Copy puts raw TSV on the clipboard (NULL as
//!   an empty cell, numbers ungrouped) and says so;
//! - a production console's INSERT: read-only session, the server refuses it inside the
//!   guard's read-only transaction; the Output log says so (the server's text and its SQLSTATE)
//!   and the gutter marks it Failed; nothing is committed;
//! - the chooser: Run asks, hovering (↓) previews the whole script in the band, ↑ goes back,
//!   Enter runs exactly the highlighted statement;
//! - stop fetching: a result streaming its rows is stopped with `exec.cancel`; the strip says
//!   Cancelled with the rows it kept;
//! - closing a console with an open transaction asks; Roll back and close rolls it back and
//!   closes the tab.

mod support;

use gpui::{Entity, Focusable as _, TestAppContext, VisualTestContext};
use hacivat_app::console::{ConsoleId, Consoles};
use hacivat_app::results::ActiveResult;
use hacivat_app::results::view::hosts;
use hacivat_driver::fake::{FakeDriver, FakeQuery, FakeResultSet, FakeRows, FakeTypedCall};
use hacivat_driver::{RawError, RawErrorKind, ServerMessage, TxStatus};
use hacivat_exec::LogMsg;
use hacivat_proto::{DbmsKind, EnvTag};
use hacivat_session::LiveState;
use hacivat_session::session::TxOwner;
use hacivat_ui_editor::{EditorView, ExecMarkState, preview};
use hacivat_values::{LogicalType, Value};
use support::*;

const DEV: &str = "orders-dev";
const PROD: &str = "orders-prod";

fn int8() -> LogicalType {
    LogicalType::Int {
        bits: 64,
        signed: true,
    }
}

fn text() -> LogicalType {
    LogicalType::Text {
        max_chars: None,
        fixed: false,
    }
}

/// A shell whose PostgreSQL connections go to a fake that answers the SQL-context probe and
/// `queries`, with the profile `name` (`env`) connected and bound to `Console 1`; the console.
fn connected(
    cx: &mut TestAppContext,
    name: &str,
    env: EnvTag,
    queries: Vec<FakeQuery>,
) -> (&'static mut VisualTestContext, ConsoleId) {
    let fake = answers_probe_with(
        FakeDriver::new(DbmsKind::Postgres).tracks_transactions(),
        queries,
    );
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
    let console = active_console(vcx).expect("the bound console");
    (vcx, console)
}

/// The console of the focused pane's active tab.
fn active_console(vcx: &VisualTestContext) -> Option<ConsoleId> {
    let tab = with_ws(vcx, |ws, _, _| ws.active_tab())?;
    vcx.cx.read(|cx| Consoles::by_tab(cx, tab))
}

fn editor_of(vcx: &VisualTestContext, id: ConsoleId) -> Entity<EditorView> {
    vcx.cx.read(|cx| {
        Consoles::get(cx, id)
            .and_then(|s| s.editor.clone())
            .expect("the console's editor")
    })
}

/// Sets the editor's text, puts the caret at `caret` and focuses the editor.
fn type_script(vcx: &VisualTestContext, editor: &Entity<EditorView>, text: &str, caret: usize) {
    let text = text.to_string();
    vcx.cx.update(|cx| {
        editor.update(cx, |e, cx| {
            e.set_text(&text, cx);
            e.select_range(caret..caret, cx);
        })
    });
    let handle = vcx.cx.read(|cx| editor.read(cx).focus_handle(cx));
    in_window(vcx, |window, cx| window.focus(&handle, cx));
    vcx.run_until_parked();
}

/// Runs the editor command `id`.
fn editor_command(vcx: &VisualTestContext, editor: &Entity<EditorView>, id: &str) {
    let editor = editor.clone();
    in_window(vcx, |window, cx| {
        editor.update(cx, |e, cx| assert!(e.run_command(id, window, cx), "{id}"));
    });
    vcx.run_until_parked();
}

fn marks(vcx: &VisualTestContext, id: ConsoleId) -> Vec<ExecMarkState> {
    vcx.cx.read(|cx| {
        hacivat_app::exec_ui::last_run_marks(id, cx)
            .into_iter()
            .flatten()
            .collect()
    })
}

fn log(vcx: &VisualTestContext, id: ConsoleId) -> Vec<LogMsg> {
    vcx.cx.read(|cx| {
        Consoles::get(cx, id)
            .map(|s| s.log.lines().iter().map(|l| l.msg.clone()).collect())
            .unwrap_or_default()
    })
}

fn typed_queries(fake: &FakeDriver) -> Vec<String> {
    fake.typed_log()
        .into_iter()
        .filter_map(|c| match c {
            FakeTypedCall::Query(sql) => Some(sql),
            _ => None,
        })
        .collect()
}

fn refresh(vcx: &VisualTestContext) {
    in_window(vcx, |window, _| window.refresh());
    vcx.run_until_parked();
}

// ---- run → rows → copy ----------------------------------------------------------------------

#[gpui::test]
fn run_rows_then_copy_puts_raw_tsv_on_the_clipboard(cx: &mut TestAppContext) {
    let shops = FakeQuery {
        prefix: "SELECT id, city FROM shops",
        sets: vec![FakeResultSet {
            columns: vec![("id", int8()), ("city", text())],
            rows: FakeRows::Literal(vec![
                vec![Value::Int(1), Value::Text("İzmir".into())],
                vec![Value::Int(2), Value::Null],
                vec![Value::Int(1_234_567), Value::Text("Şanlıurfa".into())],
            ]),
        }],
        ..Default::default()
    };
    let (vcx, id) = connected(cx, DEV, EnvTag::Dev, vec![shops]);
    enable_registry(vcx);
    let editor = editor_of(vcx, id);
    type_script(vcx, &editor, "SELECT id, city FROM shops;\n", 3);
    editor_command(vcx, &editor, "editor.run");
    wait_until(vcx, "the statement to succeed", |vcx| {
        matches!(marks(vcx, id).as_slice(), [ExecMarkState::Succeeded { .. }])
    });
    // The result tab's grid shows the rows.
    wait_until(vcx, "the grid", |vcx| {
        vcx.cx
            .read(|cx| {
                hosts(cx)
                    .into_iter()
                    .find_map(|h| h.read(cx).grid_of(id, 0, cx))
            })
            .is_some()
    });
    let grid = vcx
        .cx
        .read(|cx| {
            hosts(cx)
                .into_iter()
                .find_map(|h| h.read(cx).grid_of(id, 0, cx))
        })
        .expect("the grid");
    refresh(vcx);
    let cells: Vec<String> = registry_frame(vcx)
        .into_iter()
        .filter(|r| r.path.ends_with("#cell"))
        .map(|r| r.text.to_string())
        .collect();
    assert!(cells.iter().any(|t| t == "İzmir"), "{cells:?}");
    assert!(cells.iter().any(|t| t == "Şanlıurfa"), "{cells:?}");
    // Copy all: raw values, NULL as an empty cell, the number ungrouped.
    in_window(vcx, |window, cx| {
        let handle = grid.read(cx).focus_handle(cx);
        window.focus(&handle, cx);
    });
    vcx.run_until_parked();
    assert!(run(vcx, "grid.select_all"));
    assert!(run(vcx, "results.copy"));
    let copied = vcx
        .cx
        .read_from_clipboard()
        .and_then(|item| item.text())
        .unwrap_or_default();
    assert_eq!(copied, "1\tİzmir\n2\t\n1234567\tŞanlıurfa\n");
    refresh(vcx);
    let toasts = toast_texts(vcx);
    assert!(
        toasts.iter().any(|t| t.contains("Copied 3 × 2 cells")),
        "{toasts:?}"
    );
    // Fix round 1 (M8): the status bar and the tile's toolbar agree on the rows.
    let status_rows = text_of(vcx, "status-rows").unwrap_or_default();
    let tile_rows = text_of(vcx, "results-rows-timing").unwrap_or_default();
    assert!(
        status_rows.starts_with("3 rows") && tile_rows.starts_with("3 rows"),
        "status {status_rows:?}, tile {tile_rows:?}"
    );
    stop_core_runtime_test(cx);
}

// ---- a production console refuses a write ----------------------------------------------------

#[gpui::test]
fn a_production_insert_is_refused_logged_and_marked_failed(cx: &mut TestAppContext) {
    // What PostgreSQL answers a write inside `BEGIN … READ ONLY`.
    let refused = FakeQuery {
        prefix: "INSERT INTO audit",
        // The error comes with the first event (a set whose rows never arrive).
        sets: vec![FakeResultSet {
            columns: vec![("note", text())],
            rows: FakeRows::Literal(vec![vec![Value::Text("x".into())]]),
        }],
        fail_after: Some((
            0,
            RawError {
                kind: RawErrorKind::Db,
                server: Some(ServerMessage {
                    sqlstate: Some("25006".to_string()),
                    vendor_code: None,
                    severity: Some("ERROR".to_string()),
                    text: "cannot execute INSERT in a read-only transaction".to_string(),
                    detail: None,
                    hint: None,
                    lang_hint: None,
                    position: None,
                }),
            },
        )),
        ..Default::default()
    };
    let (vcx, id) = connected(cx, PROD, EnvTag::Production, vec![refused]);
    let session = vcx
        .cx
        .read(|cx| Consoles::get(cx, id).and_then(|s| s.session.clone()))
        .expect("a session");
    assert!(
        session.state().borrow().read_only,
        "a production console's session is read-only (R-M3-6)"
    );
    let editor = editor_of(vcx, id);
    type_script(
        vcx,
        &editor,
        "INSERT INTO audit (note) VALUES ('İzmir');\n",
        3,
    );
    editor_command(vcx, &editor, "editor.run");
    wait_until(vcx, "the Failed mark", |vcx| {
        matches!(
            marks(vcx, id).as_slice(),
            [ExecMarkState::Failed { sqlstate: Some(code) }] if code.as_ref() == "25006"
        )
    });
    let lines = log(vcx, id);
    assert!(
        lines.iter().any(|m| matches!(
            m,
            LogMsg::Error { view, .. }
                if view.code.as_deref() == Some("25006")
                    && view.server_text.as_deref()
                        == Some("cannot execute INSERT in a read-only transaction")
        )),
        "{lines:?}"
    );
    // Fix round 1 (M8): the session's read-only wrapper reached the server before the
    // INSERT did (the 25006 is the fake's script; what the product sent is not).
    let calls = fake(vcx).call_log();
    let insert = calls
        .iter()
        .position(|c| {
            matches!(c, hacivat_driver::fake::FakeCall::Typed(FakeTypedCall::Query(q))
                if q.starts_with("INSERT INTO audit"))
        })
        .expect("the INSERT reached the fake");
    assert!(
        calls[..insert].iter().any(|c| matches!(
            c,
            hacivat_driver::fake::FakeCall::Statement(sql)
                if sql.contains("READ ONLY") || sql.contains("default_transaction_read_only = on")
        )),
        "{calls:?}"
    );
    // Nothing is left open or committed: the guard's read-only transaction ended.
    let tx = session.state().borrow().tx.clone();
    assert_eq!(tx.status, TxStatus::Idle, "{tx:?}");
    // The Output tab says it: the server's text verbatim, on its own row.
    vcx.cx.update(|cx| {
        let results = Consoles::get(cx, id)
            .and_then(|s| s.results.clone())
            .expect("the console's results");
        results.update(cx, |r, cx| r.set_active(ActiveResult::Output, cx));
    });
    enable_registry(vcx);
    refresh(vcx);
    // Fix round 1 (M8): the status bar says production and read-only.
    let env = text_of(vcx, "status-env-label").unwrap_or_default();
    assert!(
        env.contains("production") && env.contains("read-only"),
        "{env:?}"
    );
    let out: Vec<String> = registry_frame(vcx)
        .into_iter()
        .filter(|r| r.path.contains("hacivat-app/src/results/output_view.rs"))
        .map(|r| r.text.to_string())
        .collect();
    assert!(
        out.iter()
            .any(|t| t.contains("cannot execute INSERT in a read-only transaction")),
        "{out:?}"
    );
    stop_core_runtime_test(cx);
}

// ---- the chooser ----------------------------------------------------------------------------

#[gpui::test]
fn the_chooser_previews_on_hover_and_runs_the_highlighted_statement(cx: &mut TestAppContext) {
    let (vcx, id) = connected(cx, DEV, EnvTag::Dev, Vec::new());
    let fake = fake(vcx);
    let editor = editor_of(vcx, id);
    let text = "SELECT 1;\nSELECT 2;\n";
    type_script(vcx, &editor, text, 0);
    editor_command(vcx, &editor, "editor.run");
    wait_until(vcx, "the chooser", |vcx| {
        vcx.cx
            .read(|cx| hacivat_app::exec_ui::chooser::is_open(id, cx))
    });
    let band = |vcx: &VisualTestContext| vcx.cx.read(|cx| preview::current(editor.read(cx)));
    let first: &[std::ops::Range<usize>] = &[std::ops::Range { start: 0, end: 8 }];
    assert_eq!(
        band(vcx).map(|b| b.units.to_vec()).as_deref(),
        Some(first),
        "the preselected statement"
    );
    let probes = typed_queries(&fake).len();
    vcx.simulate_keystrokes("down");
    vcx.run_until_parked();
    assert_eq!(
        band(vcx).map(|b| b.units.to_vec()),
        Some(vec![0..8, 10..18]),
        "hovering the whole script previews both statements"
    );
    vcx.simulate_keystrokes("up");
    vcx.run_until_parked();
    assert_eq!(band(vcx).map(|b| b.units.to_vec()).as_deref(), Some(first));
    assert_eq!(
        typed_queries(&fake).len(),
        probes,
        "nothing sent before Enter"
    );
    vcx.simulate_keystrokes("enter");
    wait_until(vcx, "the statement to run", |vcx| {
        matches!(marks(vcx, id).as_slice(), [ExecMarkState::Succeeded { .. }])
    });
    assert_eq!(&typed_queries(&fake)[probes..], ["SELECT 1"]);
    stop_core_runtime_test(cx);
}

// ---- stop fetching ----------------------------------------------------------------------------

#[gpui::test]
fn stopping_a_streaming_result_keeps_its_rows_and_says_cancelled(cx: &mut TestAppContext) {
    // Two result sets (so the cursor does not suspend): the first chunk arrives, the next is a
    // minute away; only a cancel ends it.
    let set = || FakeResultSet {
        columns: vec![("n", int8())],
        rows: FakeRows::Generated {
            count: 1_000_000,
            cell: |r, _| Value::Int(r as i64),
        },
    };
    let slow = FakeQuery {
        prefix: "SELECT n FROM numbers",
        sets: vec![set(), set()],
        chunk_delay: std::time::Duration::from_secs(60),
        ..Default::default()
    };
    let (vcx, id) = connected(cx, DEV, EnvTag::Dev, vec![slow]);
    enable_registry(vcx);
    let editor = editor_of(vcx, id);
    type_script(vcx, &editor, "SELECT n FROM numbers;\n", 3);
    editor_command(vcx, &editor, "editor.run");
    let rows = |vcx: &VisualTestContext| -> u64 {
        vcx.cx.read(|cx| {
            Consoles::get(cx, id)
                .and_then(|s| s.results.clone())
                .and_then(|r| {
                    r.read(cx)
                        .tabs()
                        .first()
                        .map(|t| t.adapter().snapshot().row_count())
                })
                .unwrap_or(0)
        })
    };
    wait_until(vcx, "the first rows", |vcx| rows(vcx) > 0);
    let kept = rows(vcx);
    assert!(run(vcx, "exec.cancel"));
    wait_until(vcx, "the cancel", |vcx| {
        matches!(marks(vcx, id).as_slice(), [ExecMarkState::Cancelled])
    });
    assert_eq!(rows(vcx), kept, "the rows so far stay");
    refresh(vcx);
    let strip = text_of(vcx, "strip-sentence").unwrap_or_default();
    // Fix round 1 (M8): the strip names the rows it kept.
    let want = vcx.cx.read(|cx| {
        cx.global::<hacivat_app::l10n_global::L10nGlobal>()
            .l10n
            .t_dynamic(
                "hv-strip-cancelled",
                vec![("rows", hacivat_l10n::ArgValue::Count(kept))],
            )
    });
    let plain = |t: &str| t.replace(['\u{2068}', '\u{2069}'], "");
    assert_eq!(plain(&strip), plain(&want));
    assert!(strip.starts_with("Cancelled after"), "{strip:?}");
    stop_core_runtime_test(cx);
}

// ---- close with an open transaction --------------------------------------------------------------

#[gpui::test]
fn closing_a_console_in_a_transaction_asks_and_roll_back_closes_it(cx: &mut TestAppContext) {
    // A typed statement's effect on the transaction is scripted (`tx_after`).
    let begin = FakeQuery {
        prefix: "BEGIN",
        tx_after: Some(TxStatus::InTx),
        ..Default::default()
    };
    let (vcx, id) = connected(cx, DEV, EnvTag::Dev, vec![begin]);
    enable_registry(vcx);
    let fake = fake(vcx);
    let editor = editor_of(vcx, id);
    let text = "BEGIN;\nUPDATE orders SET status = 'shipped' WHERE id = 1042;\n";
    type_script(vcx, &editor, text, 0);
    let all = text.len();
    vcx.cx
        .update(|cx| editor.update(cx, |e, cx| e.select_range(0..all, cx)));
    editor_command(vcx, &editor, "editor.run");
    let session = vcx
        .cx
        .read(|cx| Consoles::get(cx, id).and_then(|s| s.session.clone()))
        .expect("a session");
    wait_until(vcx, "the user's transaction", |_| {
        let tx = session.state().borrow().tx.clone();
        tx.owner == TxOwner::User && tx.status == TxStatus::InTx
    });
    let tabs = tab_count(vcx);
    assert!(run(vcx, "workspace.close_tab"));
    assert!(layer_ids(vcx).contains(&"dialog"), "the prompt opens");
    assert_eq!(tab_count(vcx), tabs, "nothing closed yet");
    // Cancel is focused; Shift-Tab reaches Roll back and close.
    vcx.simulate_keystrokes("shift-tab");
    vcx.simulate_keystrokes("enter");
    wait_until(vcx, "the console to close", |vcx| {
        tab_count(vcx) == tabs - 1
    });
    assert!(
        fake.executed().iter().any(|s| s.trim() == "ROLLBACK"),
        "{:?}",
        fake.executed()
    );
    assert!(vcx.cx.read(|cx| Consoles::get(cx, id).is_none()));
    stop_core_runtime_test(cx);
}
