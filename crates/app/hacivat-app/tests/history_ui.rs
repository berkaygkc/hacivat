//! M3b Task 27: the query history dialog (HV-EXEC-12 UI, D-061; `design/m3b-safe` surface 11)
//! over a real `HistoryStore` (Task 17) on the shell's core runtime and a FakeDriver console:
//! the Turkish-tolerant, debounced search, Insert at the caret as one undo group, Insert and
//! run through the console's own Run, the redacted marker, and the empty and no-match states.
#![cfg_attr(not(unix), allow(dead_code, unused_imports))]

mod support;

use std::sync::Arc;

use gpui::{Entity, TestAppContext, VisualTestContext};
use hacivat_app::console::ConsoleId;
use hacivat_app::exec_ui;
use hacivat_app::exec_ui::context::SqlContext;
use hacivat_app::history::dialog::{self, DEBOUNCE, HISTORY_LAYER_ID};
use hacivat_driver::fake::{FakeDriver, FakeTypedCall};
use hacivat_exec::{HistoryConfig, HistoryOrigin, HistoryRecord, HistoryStore, Outcome};
use hacivat_proto::{DbmsKind, EnvTag, ProfileId};
use hacivat_session::session::TxMode;
use hacivat_ui_core::gpui_surface::EntityInputHandler as _;
use hacivat_ui_editor::EditorView;
use support::*;

/// A console over a FakeDriver session of `orders-dev` and a history store holding records.
struct History {
    vcx: &'static mut VisualTestContext,
    console: ConsoleId,
    editor: Entity<EditorView>,
    fake: Arc<FakeDriver>,
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn record(profile: ProfileId, sql: &str, outcome: Outcome, ago_ms: i64) -> HistoryRecord {
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
        outcome,
        rows_affected: None,
        rows_fetched: Some(3),
        error_code: None,
        error_message: None,
        tx_mode: TxMode::Auto,
        run: None,
        redacted: false,
    }
}

/// The shell, the console and the store with `sqls` recorded (the first one newest), handed
/// to the dialog.
fn history(cx: &mut TestAppContext, sqls: &[&str]) -> History {
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
    let (session, fake) = futures::executor::block_on(hacivat_session::testing::fake_session(
        rt.clone(),
        answers_probe(FakeDriver::new(DbmsKind::Postgres)),
        p,
    ));
    let console = register_test_console(vcx, profile, session);
    wait_until(vcx, "the console's SQL context", |vcx| {
        vcx.cx
            .read(|cx| exec_ui::sql_context(console, cx) != SqlContext::Pending)
    });
    let dir = temp_dir(vcx);
    let store = futures::executor::block_on(rt.spawn(HistoryStore::open(
        dir.join("history.sqlite"),
        HistoryConfig::default(),
    )))
    .expect("the open task")
    .expect("the history opens");
    let sink = store.sink();
    for (i, sql) in sqls.iter().enumerate() {
        let outcome = if sql.contains("shipmnts") {
            Outcome::Failed
        } else {
            Outcome::Ok
        };
        sink.try_send(Arc::new(record(profile, sql, outcome, 1_000 * i as i64)))
            .expect("room in the sink");
    }
    vcx.cx
        .update(|cx| hacivat_app::history::set_store(Some(store), cx));
    let editor = vcx.cx.read(|cx| {
        hacivat_app::console::Consoles::get(cx, console)
            .and_then(|s| s.editor.clone())
            .expect("the console has an editor")
    });
    History {
        vcx,
        console,
        editor,
        fake,
    }
}

impl History {
    /// Opens the dialog with `history.open` and waits for its first answer.
    fn open(&self) {
        assert!(run(self.vcx, "history.open"), "history.open runs");
        wait_until(self.vcx, "the first search", |vcx| {
            vcx.cx
                .read(|cx| dialog::shown(cx).is_some_and(|s| s.loaded))
        });
        assert!(layer_ids(self.vcx).contains(&HISTORY_LAYER_ID));
    }

    fn shown(&self) -> dialog::Shown {
        self.vcx.cx.read(dialog::shown).expect("the dialog is open")
    }

    fn keys(&mut self, keys: &str) {
        self.vcx.simulate_keystrokes(keys);
        self.vcx.run_until_parked();
    }

    fn set_text(&self, text: &str, caret: usize) {
        let text = text.to_string();
        self.vcx.cx.update(|cx| {
            self.editor.update(cx, |e, cx| {
                e.set_text(&text, cx);
                e.select_range(caret..caret, cx);
            });
        });
        self.vcx.run_until_parked();
    }

    fn text(&self) -> String {
        editor_text(self.vcx, self.console)
    }

    /// Types `text` into the search field, then lets the debounce run out.
    fn search(&mut self, text: &str) {
        let before = self.shown().searches;
        self.vcx.simulate_input(text);
        self.vcx.run_until_parked();
        // Debounced: nothing is searched until 150 ms pass without a keystroke.
        assert_eq!(
            self.shown().searches,
            before,
            "searched before the debounce"
        );
        self.vcx.executor().advance_clock(DEBOUNCE);
        wait_until(self.vcx, "the debounced search", |_| {
            let s = self.shown();
            s.searches > before && s.answered == s.searches
        });
        self.vcx.run_until_parked();
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

    fn closed(&self) -> bool {
        !layer_ids(self.vcx).contains(&HISTORY_LAYER_ID)
    }
}

fn run_chord() -> &'static str {
    if cfg!(target_os = "macos") {
        "cmd-enter"
    } else {
        "ctrl-enter"
    }
}

#[cfg(unix)]
#[gpui::test]
fn search_is_turkish_tolerant(cx: &mut TestAppContext) {
    let mut h = history(
        cx,
        &[
            "SELECT * FROM MÜŞTERİLER WHERE şehir = 'İSTANBUL';",
            "SELECT c.full_name AS \"müşteri adı\" FROM customers c;",
            "SELECT id FROM orders WHERE status = 'iptal';",
        ],
    );
    h.open();
    assert_eq!(h.shown().statements.len(), 3);
    // `musteri` finds `müşteri` and `MÜŞTERİLER` (case, dotted and dotless i, accents fold).
    h.search("musteri");
    let s = h.shown();
    assert_eq!(
        s.statements,
        vec![
            "SELECT * FROM MÜŞTERİLER WHERE şehir = 'İSTANBUL';".to_string(),
            "SELECT c.full_name AS \"müşteri adı\" FROM customers c;".to_string(),
        ]
    );
    // The dotless/dotted pair: `ıstanbul` finds `İSTANBUL` (never ASCII lowercasing, which
    // would leave `İ` alone).
    h.keys(if cfg!(target_os = "macos") {
        "cmd-a"
    } else {
        "ctrl-a"
    });
    h.search("ıstanbul");
    assert_eq!(
        h.shown().statements,
        vec!["SELECT * FROM MÜŞTERİLER WHERE şehir = 'İSTANBUL';".to_string()]
    );
    stop_core_runtime_test(cx);
}

#[cfg(unix)]
#[gpui::test]
fn enter_inserts_at_the_caret(cx: &mut TestAppContext) {
    let mut h = history(cx, &["SELECT 2 AS newest;", "SELECT 1 AS older;"]);
    let original = "SELECT 0;\n\n-- end";
    h.set_text(original, 10);
    h.open();
    // ↓ moves the selection while the search field keeps the focus.
    h.keys("down");
    assert_eq!(h.shown().selected, Some(1));
    h.keys("enter");
    assert!(h.closed(), "Insert closes the dialog");
    assert_eq!(h.text(), "SELECT 0;\nSELECT 1 AS older;\n-- end");
    assert!(
        h.queries().iter().all(|q| !q.contains("older")),
        "Insert never runs"
    );
    // One undo group takes the whole insert back.
    let editor = h.editor.clone();
    in_window(h.vcx, |window, cx| {
        editor.update(cx, |e, cx| {
            assert!(e.run_command("editor.undo", window, cx))
        });
    });
    h.vcx.run_until_parked();
    assert_eq!(h.text(), original);
    // While a composition is open in the editor nothing is inserted (never queued).
    in_window(h.vcx, |window, cx| {
        editor.update(cx, |e, cx| {
            e.replace_and_mark_text_in_range(None, "ş", None, window, cx);
        });
    });
    h.vcx.run_until_parked();
    let composing = h.text();
    let inserted = h
        .vcx
        .cx
        .update(|cx| exec_ui::insert_at_caret(h.console, "SELECT 9;", cx));
    assert!(!inserted);
    assert_eq!(h.text(), composing);
    in_window(h.vcx, |window, cx| {
        editor.update(cx, |e, cx| e.unmark_text(window, cx));
    });
    stop_core_runtime_test(cx);
}

#[cfg(unix)]
#[gpui::test]
fn cmd_enter_inserts_and_runs_through_the_gate(cx: &mut TestAppContext) {
    let mut h = history(cx, &["SELECT 42 AS answer;"]);
    h.open();
    h.keys(run_chord());
    assert!(h.closed(), "Insert and run closes the dialog");
    assert_eq!(h.text(), "SELECT 42 AS answer;");
    // The editor's own Run took it: the orchestrator resolved it, marked it in the gutter and
    // sent exactly the inserted statement.
    wait_until(h.vcx, "the run of the inserted statement", |_| {
        h.queries()
            .iter()
            .any(|q| q.contains("SELECT 42 AS answer"))
    });
    wait_until(h.vcx, "its mark", |vcx| {
        vcx.cx
            .read(|cx| !exec_ui::last_run_marks(h.console, cx).is_empty())
    });

    // The same checks as any Run: a console that waits to become read-only sends nothing,
    // whatever History hands it.
    h.vcx
        .cx
        .update(|cx| exec_ui::set_pending_read_only(h.console, true, cx));
    h.set_text("", 0);
    let sent = h.queries().len();
    h.open();
    h.keys(run_chord());
    assert!(h.closed());
    assert_eq!(h.text(), "SELECT 42 AS answer;");
    h.vcx.run_until_parked();
    assert_eq!(h.queries().len(), sent, "Run refused it: nothing was sent");
    stop_core_runtime_test(cx);
}

#[cfg(unix)]
#[gpui::test]
fn redacted_entry_shows_the_marker(cx: &mut TestAppContext) {
    let mut h = history(
        cx,
        &[
            "ALTER ROLE raporcu PASSWORD 's3cret';",
            "SELECT 1 AS plain;",
        ],
    );
    h.open();
    // The store kept only the redacted text.
    let s = h.shown();
    assert_eq!(s.selected, Some(0));
    assert!(
        s.statements[0].contains("PASSWORD '***'"),
        "{:?}",
        s.statements
    );
    assert_eq!(
        text_of(h.vcx, "history-redacted-chip").as_deref(),
        Some("redacted")
    );
    assert_eq!(
        text_of(h.vcx, "history-redacted-line").as_deref(),
        Some("A password or secret was removed before this was saved.")
    );
    // The secret is nowhere on screen.
    assert!(
        registry_frame(h.vcx)
            .iter()
            .all(|r| !r.text.contains("s3cret"))
    );
    // An entry without a secret has no marker.
    h.keys("down");
    assert_eq!(text_of(h.vcx, "history-redacted-chip"), None);
    assert_eq!(text_of(h.vcx, "history-redacted-line"), None);
    stop_core_runtime_test(cx);
}

#[cfg(unix)]
#[gpui::test]
fn empty_and_no_match_states(cx: &mut TestAppContext) {
    let mut h = history(cx, &[]);
    h.open();
    assert!(h.shown().statements.is_empty());
    assert_eq!(
        text_of(h.vcx, "history-empty").as_deref(),
        Some("No history yet. Statements you run are kept here for 90 days.")
    );
    assert_eq!(
        text_of(h.vcx, "history-count").as_deref(),
        Some("0 entries")
    );
    h.search("faturalar");
    assert_eq!(
        text_of(h.vcx, "history-empty").as_deref(),
        Some("Nothing matches “faturalar”.")
    );
    // Nothing to insert: Enter does nothing and the dialog stays; Escape closes it.
    h.keys("enter");
    assert!(!h.closed());
    h.keys("escape");
    assert!(h.closed());
    stop_core_runtime_test(cx);
}
