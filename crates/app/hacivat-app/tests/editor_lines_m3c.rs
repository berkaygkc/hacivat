//! M3c Task 34: the app's side of the editor's line commands and gutter (NOTES surface 17): the
//! gutter ▶'s gate follows whether Run would be enabled (M3c-15), history recall at the last
//! line of an empty or recalled console reads the profile's history (⌃⌥↑ / ⌃⌥↓) and says where
//! it is, and the mixed line endings menu offers the dominant ending first.
#![cfg_attr(not(unix), allow(dead_code, unused_imports))]

mod support;

use std::sync::Arc;

use gpui::{Entity, TestAppContext, VisualTestContext};
use hacivat_app::console::{ConsoleId, Consoles};
use hacivat_app::exec_ui::context::SqlContext;
use hacivat_driver::fake::{FakeDriver, FakeQuery, FakeResultSet, FakeRows};
use hacivat_exec::{HistoryOrigin, HistoryRecord, Outcome};
use hacivat_proto::{DbmsKind, EnvTag, ProfileId};
use hacivat_session::session::TxMode;
use hacivat_ui_editor::{EditorView, ExecMarkState};
use hacivat_values::{LogicalType, Value};
use support::*;

/// A console over a FakeDriver session of `orders-dev` (PostgreSQL), its SQL context read.
fn console(
    cx: &mut TestAppContext,
    extra: Vec<FakeQuery>,
) -> (
    &'static mut VisualTestContext,
    ConsoleId,
    ProfileId,
    Entity<EditorView>,
) {
    let vcx = shell_with_connections(cx, "en");
    in_window(vcx, |window, _| window.activate_window());
    let p = hacivat_session::profile::Profile {
        engine: DbmsKind::Postgres,
        ..test_profile("orders-dev", Some(EnvTag::Dev), "127.0.0.1")
    };
    let profile = add_profile(vcx, p.clone());
    let rt = vcx.cx.read(|cx| {
        cx.global::<hacivat_app::connections::ConnectionsModel>()
            .core_runtime()
            .handle()
    });
    let (session, _fake) = futures::executor::block_on(hacivat_session::testing::fake_session(
        rt,
        answers_probe_with(FakeDriver::new(DbmsKind::Postgres), extra),
        p,
    ));
    let id = register_test_console(vcx, profile, session);
    wait_until(vcx, "the console's SQL context", |vcx| {
        vcx.cx
            .read(|cx| hacivat_app::exec_ui::sql_context(id, cx) != SqlContext::Pending)
    });
    let editor = vcx.cx.read(|cx| {
        Consoles::get(cx, id)
            .and_then(|s| s.editor.clone())
            .expect("the console has an editor")
    });
    in_window(vcx, |window, cx| {
        let handle = gpui::Focusable::focus_handle(editor.read(cx), cx);
        window.focus(&handle, cx);
    });
    vcx.run_until_parked();
    (vcx, id, profile, editor)
}

fn set_text(vcx: &mut VisualTestContext, editor: &Entity<EditorView>, text: &str, caret: usize) {
    let text = text.to_string();
    vcx.cx.update(|cx| {
        editor.update(cx, |e, cx| {
            e.set_text(&text, cx);
            e.select_range(caret..caret, cx);
        });
    });
    vcx.run_until_parked();
}

fn command(vcx: &mut VisualTestContext, editor: &Entity<EditorView>, id: &str) {
    let editor = editor.clone();
    in_window(vcx, |window, cx| {
        editor.update(cx, |e, cx| assert!(e.run_command(id, window, cx)));
    });
    vcx.run_until_parked();
}

fn gate(vcx: &VisualTestContext, id: ConsoleId) -> bool {
    vcx.cx
        .read(|cx| hacivat_app::console::commands::run_would_be_enabled(id, cx))
}

/// The gutter ▶ shows only while Run would be enabled: not while a statement runs, then again
/// once the run ended and the session is idle.
#[cfg(unix)]
#[gpui::test]
fn the_gutter_run_gate_follows_run(cx: &mut TestAppContext) {
    let slow = FakeQuery {
        prefix: "UPDATE",
        sets: vec![FakeResultSet {
            columns: vec![(
                "n",
                LogicalType::Int {
                    bits: 64,
                    signed: true,
                },
            )],
            rows: FakeRows::Generated {
                count: 3,
                cell: |r, _| Value::Int(r as i64),
            },
        }],
        first_delay: std::time::Duration::from_millis(400),
        ..Default::default()
    };
    let (vcx, id, _, editor) = console(cx, vec![slow]);
    set_text(
        vcx,
        &editor,
        "UPDATE t SET a = a + 1 WHERE id > 0 RETURNING a",
        0,
    );
    assert!(gate(vcx, id), "idle: Run would be enabled");
    command(vcx, &editor, "editor.run");
    wait_until(vcx, "the statement to run", |vcx| {
        matches!(
            vcx.cx
                .read(|cx| hacivat_app::exec_ui::last_run_marks(id, cx))
                .as_slice(),
            [Some(ExecMarkState::Running { .. } | ExecMarkState::Queued)]
        )
    });
    assert!(!gate(vcx, id), "no ▶ while a statement runs");
    wait_until(vcx, "the run to end and the session to be idle", |vcx| {
        vcx.cx
            .read(|cx| hacivat_app::exec_ui::last_run_ended(id, cx))
            && gate(vcx, id)
    });
    // ## fix round 1 (M1): a closed (disconnected) session: Run connects first, so the ▶
    // shows, as Run is enabled.
    let session = vcx
        .cx
        .read(|cx| Consoles::get(cx, id).and_then(|s| s.session.clone()))
        .expect("a session");
    futures::executor::block_on(session.close(hacivat_session::session::ClosePolicy::RollBack))
        .expect("the session closes");
    wait_until(vcx, "the session to be closed", |_| {
        matches!(
            session.state().borrow().state,
            hacivat_session::session::SessionState::Disconnected
        )
    });
    assert!(gate(vcx, id), "Run is enabled on a disconnected console");
    stop_core_runtime_test(cx);
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn record(profile: ProfileId, sql: &str, ago_ms: i64) -> HistoryRecord {
    HistoryRecord {
        ts_start_ms: now_ms() - ago_ms,
        duration_ms: 12,
        profile,
        database: Some("orders".into()),
        schema: Some("public".into()),
        console: 1,
        origin: HistoryOrigin::Console,
        statement_masked: Arc::from(sql),
        text_hash: [0; 32],
        fingerprint: 0,
        params_masked: None,
        outcome: Outcome::Ok,
        rows_affected: None,
        rows_fetched: Some(1),
        error_code: None,
        error_message: None,
        tx_mode: TxMode::Auto,
        run: None,
        redacted: false,
    }
}

/// `s` without the Unicode isolation marks Fluent puts around arguments.
fn strip(s: &str) -> String {
    s.chars()
        .filter(|c| !matches!(c, '\u{2068}' | '\u{2069}'))
        .collect()
}

fn said(vcx: &VisualTestContext, editor: &Entity<EditorView>) -> String {
    vcx.cx.read(|cx| {
        hacivat_ui_editor::commands::status_message(editor.read(cx))
            .map(|s| strip(&s))
            .unwrap_or_default()
    })
}

/// ⌃⌥↑ at the last line of an empty console puts the profile's newest statement in the
/// editor and says "From history: statement 1 of 2"; again, the older one; ⌃⌥↓ comes back.
/// Typing into the recalled text ends the recall.
#[cfg(unix)]
#[gpui::test]
fn history_recall_at_the_last_line(cx: &mut TestAppContext) {
    let (vcx, id, profile, editor) = console(cx, Vec::new());
    wait_until(vcx, "the history store", |vcx| {
        vcx.cx.read(|cx| hacivat_app::history::store(cx).is_some())
    });
    let store = vcx
        .cx
        .read(hacivat_app::history::store)
        .expect("the history store");
    let sink = store.sink();
    for (i, sql) in ["SELECT 1 FROM newest", "SELECT 2 FROM older"]
        .iter()
        .enumerate()
    {
        sink.try_send(Arc::new(record(profile, sql, 1_000 * (i as i64 + 1))))
            .expect("room in the sink");
    }
    wait_until(vcx, "the records in the store", |_| {
        futures::executor::block_on(store.recall(profile, String::new(), 10))
            .is_ok_and(|r| r.len() == 2)
    });
    let _ = id;
    command(vcx, &editor, "editor.history_prev");
    wait_until(vcx, "the newest statement", |vcx| {
        editor_text(vcx, id) == "SELECT 1 FROM newest"
    });
    assert!(
        said(vcx, &editor).starts_with("From history: statement 1 of 2 · "),
        "{}",
        said(vcx, &editor)
    );
    command(vcx, &editor, "editor.history_prev");
    assert_eq!(editor_text(vcx, id), "SELECT 2 FROM older");
    assert!(said(vcx, &editor).starts_with("From history: statement 2 of 2 · "));
    // Nothing older: it stays.
    command(vcx, &editor, "editor.history_prev");
    assert_eq!(editor_text(vcx, id), "SELECT 2 FROM older");
    command(vcx, &editor, "editor.history_next");
    assert_eq!(editor_text(vcx, id), "SELECT 1 FROM newest");
    // Typed into: no recall any more.
    let len = editor_text(vcx, id).len();
    vcx.cx.update(|cx| {
        editor.update(cx, |e, cx| e.select_range(len..len, cx));
    });
    in_window(vcx, |window, cx| {
        editor.update(cx, |e, cx| {
            gpui::EntityInputHandler::replace_text_in_range(e, None, " WHERE x", window, cx);
        });
    });
    vcx.run_until_parked();
    command(vcx, &editor, "editor.history_prev");
    assert_eq!(editor_text(vcx, id), "SELECT 1 FROM newest WHERE x");
    stop_core_runtime_test(cx);
}

/// Mixed line endings: the editor says so, and the menu lists the counts, then the dominant
/// ending first with "most lines"; its row normalises in one undo group.
#[cfg(unix)]
#[gpui::test]
fn the_mixed_line_endings_menu_offers_the_dominant_ending_first(cx: &mut TestAppContext) {
    let (vcx, id, _, editor) = console(cx, Vec::new());
    set_text(vcx, &editor, "SELECT 1;\r\nSELECT 2;\r\nSELECT 3;\n", 0);
    assert!(
        vcx.cx
            .read(|cx| editor.read(cx).status().mixed_line_endings)
    );
    let items = vcx
        .cx
        .read(|cx| hacivat_app::console::status::eol_menu_items(&editor, cx));
    let labels: Vec<String> = vcx.cx.read(|cx| {
        items
            .iter()
            .map(|i| strip(&(i.label)(cx)))
            .collect::<Vec<_>>()
    });
    assert_eq!(
        labels,
        [
            "Line endings: LF 1, CRLF 2",
            "Normalize to CRLF",
            "Normalize to LF"
        ]
    );
    let hint = vcx
        .cx
        .read(|cx| items[1].hint.as_ref().map(|h| strip(&h(cx))));
    assert_eq!(hint.as_deref(), Some("most lines"));
    vcx.cx.update(|cx| {
        editor.update(cx, |e, cx| {
            e.normalize_line_endings_to(hacivat_text::LineEnding::CrLf, cx);
        });
    });
    vcx.run_until_parked();
    assert_eq!(
        editor_text(vcx, id),
        "SELECT 1;\r\nSELECT 2;\r\nSELECT 3;\r\n"
    );
    assert!(
        !vcx.cx
            .read(|cx| editor.read(cx).status().mixed_line_endings)
    );
    // ## fix round 1 (M3): CR endings are counted and offered; the dominant one (CR here)
    // comes first with "most lines"; the tooltip names the actual mix.
    set_text(vcx, &editor, "a\rb\rc\nd\r", 0);
    let items = vcx
        .cx
        .read(|cx| hacivat_app::console::status::eol_menu_items(&editor, cx));
    let labels: Vec<String> = vcx
        .cx
        .read(|cx| items.iter().map(|i| strip(&(i.label)(cx))).collect());
    assert_eq!(
        labels,
        [
            "Line endings: LF 1, CRLF 0, CR 3",
            "Normalize to CR",
            "Normalize to LF",
            "Normalize to CRLF"
        ]
    );
    let hint = vcx
        .cx
        .read(|cx| items[1].hint.as_ref().map(|h| strip(&h(cx))));
    assert_eq!(hint.as_deref(), Some("most lines"));
    let tip = vcx
        .cx
        .read(|cx| strip(&hacivat_app::console::status::eol_tip(&editor, cx)));
    assert_eq!(
        tip,
        "Lines end in more than one way (LF, CR). Click to normalize them."
    );
    // A read-only editor is not offered the button (its rows would do nothing).
    assert!(
        vcx.cx
            .read(|cx| hacivat_app::console::status::eol_offered(&editor, cx))
    );
    vcx.cx.update(|cx| {
        editor.update(cx, |e, cx| {
            let cfg = hacivat_ui_editor::EditorConfig {
                read_only: true,
                ..*e.config()
            };
            e.set_config(cfg, cx);
        });
    });
    assert!(
        !vcx.cx
            .read(|cx| hacivat_app::console::status::eol_offered(&editor, cx))
    );
    stop_core_runtime_test(cx);
}

// ## m3c-int (M1): the ▶'s gate is Run's whole enablement: not while a run is being prepared
// (here the parameter prompt is open) nor while a read-only reconnect is pending.
#[cfg(unix)]
#[gpui::test]
fn the_gutter_run_gate_is_off_while_preparing_or_pending_read_only(cx: &mut TestAppContext) {
    use hacivat_app::exec_ui::params_prompt;
    let (vcx, id, _, editor) = console(cx, vec![]);
    set_text(vcx, &editor, "SELECT * FROM t WHERE id = :id", 0);
    assert!(gate(vcx, id), "idle: Run would be enabled");
    command(vcx, &editor, "editor.run");
    wait_until(vcx, "the parameter prompt", |vcx| {
        vcx.cx.read(|cx| params_prompt::open_prompt(cx).is_some())
    });
    assert!(!gate(vcx, id), "no ▶ while the parameter prompt asks");
    let prompt = vcx.cx.read(params_prompt::open_prompt).expect("open");
    in_window(vcx, |window, cx| {
        prompt.update(cx, |p, cx| p.cancel(window, cx))
    });
    // The open prompt is the one nobody holds once it closed.
    drop(prompt);
    wait_until(vcx, "the prompt to close and Run to be enabled", |vcx| {
        vcx.cx.read(|cx| params_prompt::open_prompt(cx).is_none()) && gate(vcx, id)
    });
    vcx.cx
        .update(|cx| hacivat_app::exec_ui::set_pending_read_only(id, true, cx));
    assert!(
        !gate(vcx, id),
        "no ▶ while a read-only reconnect is pending"
    );
    vcx.cx
        .update(|cx| hacivat_app::exec_ui::set_pending_read_only(id, false, cx));
    assert!(gate(vcx, id));
    stop_core_runtime_test(cx);
}
