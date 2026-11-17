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
use hacivat_exec::{HistoryOrigin, HistoryQuery, HistoryRecord, HistoryStore, Outcome};
use hacivat_proto::{DbmsKind, EnvTag, ProfileId};
use hacivat_session::session::TxMode;
use hacivat_ui_core::gpui_surface::EntityInputHandler as _;
use hacivat_ui_editor::EditorView;
use support::*;

/// A console over a FakeDriver session of `orders-dev` and the app's history store (opened at
/// boot, handed to the dialog by `open_history`) holding records.
struct History {
    vcx: &'static mut VisualTestContext,
    console: ConsoleId,
    profile: ProfileId,
    editor: Entity<EditorView>,
    fake: Arc<FakeDriver>,
    store: Arc<HistoryStore>,
    rt: tokio::runtime::Handle,
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

/// The shell, the console and the app's store with `sqls` recorded (the first one newest).
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
    // Fix round 1 (the T24 merge carry): the store the app opens at boot reaches the dialog.
    wait_until(vcx, "the history store", |vcx| {
        vcx.cx.read(|cx| hacivat_app::history::store(cx).is_some())
    });
    let store = vcx
        .cx
        .read(hacivat_app::history::store)
        .expect("the history store");
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
    let editor = vcx.cx.read(|cx| {
        hacivat_app::console::Consoles::get(cx, console)
            .and_then(|s| s.editor.clone())
            .expect("the console has an editor")
    });
    History {
        vcx,
        console,
        profile,
        editor,
        fake,
        store,
        rt,
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

    /// Records `rec` (the dialog's next search sees it).
    fn put(&self, rec: HistoryRecord) {
        self.store
            .sink()
            .try_send(Arc::new(rec))
            .expect("room in the sink");
    }

    /// What the store holds for `q`, read on the core runtime.
    fn stored(&self, q: HistoryQuery) -> Vec<hacivat_exec::HistoryHit> {
        futures::executor::block_on(self.rt.spawn(self.store.search(q)))
            .expect("the search task")
            .expect("the search")
    }

    fn active_tab(&self) -> Option<hacivat_ui::panes::TabId> {
        let ws = workspace(self.vcx);
        self.vcx.cx.read(|cx| ws.read(cx).active_tab())
    }

    fn console_tab(&self) -> hacivat_ui::panes::TabId {
        self.vcx.cx.read(|cx| {
            hacivat_app::console::Consoles::get(cx, self.console)
                .expect("the console")
                .tab
        })
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
    // The first run was recorded (T24's history sink), as the unit it ran: the newest entry.
    assert_eq!(h.text().trim_end_matches(';'), "SELECT 42 AS answer");
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
    // Fix round 1 (m8): a failure never prints the statements (a regressed redaction would
    // print the secret).
    assert!(s.statements[0].contains("PASSWORD '***'"));
    // Fix round 1 (I1): never offered to run; the preview says why.
    assert!(!s.can_run);
    assert!(text_of(h.vcx, "history-masked-line").is_some());
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
        text_of(h.vcx, "history-nomatch").as_deref(),
        Some("Nothing matches “faturalar”.")
    );
    // Nothing to insert: Enter does nothing and the dialog stays; Escape closes it.
    h.keys("enter");
    assert!(!h.closed());
    h.keys("escape");
    assert!(h.closed());
    stop_core_runtime_test(cx);
}

// ## m3b-t27-history (fix round 1)

fn secondary(key: &str) -> String {
    if cfg!(target_os = "macos") {
        format!("cmd-{key}")
    } else {
        format!("ctrl-{key}")
    }
}

/// I1: a text the store or the run service masked never ran as it reads: Insert and run is off
/// (button and chord) and the preview says why; Insert still works. Both the flagged entry and
/// an over-masked one the store did not flag (`mask_secrets` masks every literal once a
/// `password` word appears).
#[cfg(unix)]
#[gpui::test]
fn masked_entries_are_never_run(cx: &mut TestAppContext) {
    let mut h = history(cx, &["ALTER ROLE raporcu PASSWORD 's3cret';"]);
    h.put(record(
        h.profile,
        "UPDATE users SET name = '***' WHERE password IS NULL;",
        Outcome::Ok,
        -1_000,
    ));
    h.open();
    assert_eq!(h.shown().statements.len(), 2);
    for i in 0..2 {
        assert_eq!(h.shown().selected, Some(i));
        assert!(!h.shown().can_run, "an entry is offered to run");
        assert!(text_of(h.vcx, "history-masked-line").is_some());
        let sent = h.queries().len();
        h.keys(run_chord());
        assert!(!h.closed(), "Insert and run acted on a masked entry");
        assert_eq!(h.text(), "");
        assert_eq!(h.queries().len(), sent);
        h.keys("down");
    }
    // Insert stays available.
    h.keys("enter");
    assert!(h.closed());
    assert_eq!(h.text(), "ALTER ROLE raporcu PASSWORD '***';");
    stop_core_runtime_test(cx);
}

/// m4: a statement the store cut at its size cap is never run, and Insert marks the cut.
#[cfg(unix)]
#[gpui::test]
fn a_cut_entry_is_inserted_with_its_mark_and_never_run(cx: &mut TestAppContext) {
    let mut h = history(cx, &[]);
    let long = format!("SELECT '{}';", "x".repeat(70 << 10));
    h.put(record(h.profile, &long, Outcome::Ok, 0));
    h.open();
    assert!(!h.shown().can_run);
    assert!(text_of(h.vcx, "history-truncated-line").is_some());
    h.keys(run_chord());
    assert!(!h.closed());
    h.keys("enter");
    assert!(h.closed());
    let text = h.text();
    assert!(text.ends_with(dialog::TRUNCATION_MARK));
    assert!(text.len() < long.len());
    stop_core_runtime_test(cx);
}

/// I2: opened from another tab (Settings), History brings its console to the front and says
/// which console Insert goes into; ⌘↵ then runs there, in sight.
#[cfg(unix)]
#[gpui::test]
fn history_from_settings_brings_its_console_forward(cx: &mut TestAppContext) {
    let mut h = history(cx, &["SELECT 7 AS seven;"]);
    assert!(run(h.vcx, "workspace.open_settings"));
    let console_tab = h.console_tab();
    assert_ne!(h.active_tab(), Some(console_tab), "Settings is in front");
    h.keys(&secondary("shift-h"));
    wait_until(h.vcx, "the dialog's first search", |vcx| {
        vcx.cx
            .read(|cx| dialog::shown(cx).is_some_and(|s| s.loaded))
    });
    assert_eq!(h.shown().console, h.console);
    assert_eq!(
        h.active_tab(),
        Some(console_tab),
        "the console came forward"
    );
    assert_eq!(
        text_of(h.vcx, "history-target").as_deref(),
        Some("Goes into the console orders-dev; Run checks it there.")
    );
    h.keys(run_chord());
    assert!(h.closed());
    assert_eq!(h.active_tab(), Some(console_tab));
    wait_until(h.vcx, "the run in the console in front", |_| {
        h.queries().iter().any(|q| q.contains("SELECT 7 AS seven"))
    });
    stop_core_runtime_test(cx);
}

/// m7: the scope and outcome filters make the store query, and the list follows.
#[cfg(unix)]
#[gpui::test]
fn filters_make_the_store_query(cx: &mut TestAppContext) {
    let h = history(cx, &["SELECT 1 AS mine;"]);
    let other = ProfileId::new();
    h.put(record(other, "SELECT 2 AS theirs;", Outcome::Ok, 2_000));
    h.put(record(
        h.profile,
        "SELECT * FROM shipmnts;",
        Outcome::Failed,
        3_000,
    ));
    h.open();
    let s = h.shown();
    assert_eq!(s.query_profile, Some(h.profile));
    assert_eq!(s.query_outcome, None);
    assert_eq!(s.statements.len(), 2, "this connection only");
    click(h.vcx, "history-scope-all");
    wait_until(h.vcx, "the all-connections answer", |_| {
        let s = h.shown();
        s.answered == s.searches && s.statements.len() == 3
    });
    assert_eq!(h.shown().query_profile, None);
    for (index, outcome, n) in [
        (1, Some(Outcome::Ok), 2),
        (2, Some(Outcome::Failed), 1),
        (3, Some(Outcome::Cancelled), 0),
        (0, None, 3),
    ] {
        h.vcx.cx.update(|cx| dialog::pick_outcome(index, cx));
        wait_until(h.vcx, "the outcome answer", |_| {
            let s = h.shown();
            s.answered == s.searches && s.statements.len() == n
        });
        assert_eq!(h.shown().query_outcome, outcome);
    }
    click(h.vcx, "history-scope-this");
    wait_until(h.vcx, "this connection again", |_| {
        let s = h.shown();
        s.answered == s.searches && s.statements.len() == 2
    });
    stop_core_runtime_test(cx);
}

/// m7: Pin goes to the store, Copy SQL writes the stored text, Open in new console inserts it
/// into a new console of the entry's connection.
#[cfg(unix)]
#[gpui::test]
fn pin_copy_and_open_in_new_console(cx: &mut TestAppContext) {
    let h = history(cx, &["SELECT 3 AS three;"]);
    h.open();
    click(h.vcx, "button-history-pin");
    wait_until(h.vcx, "the pin", |_| {
        h.stored(HistoryQuery::default())
            .first()
            .is_some_and(|hit| hit.pinned)
    });
    click(h.vcx, "button-history-copy");
    assert_eq!(
        h.vcx
            .cx
            .read_from_clipboard()
            .and_then(|c| c.text())
            .as_deref(),
        Some("SELECT 3 AS three;")
    );
    let before = h.console_tab();
    click(h.vcx, "button-history-open-new");
    assert!(h.closed());
    let ws = workspace(h.vcx);
    settle_until(h.vcx, std::time::Duration::from_secs(10), || {
        h.vcx.cx.read(|cx| {
            let Some(tab) = ws.read(cx).active_tab() else {
                return false;
            };
            tab != before
                && hacivat_app::console::Consoles::by_tab(cx, tab)
                    .and_then(|c| hacivat_app::console::Consoles::get(cx, c))
                    .and_then(|s| s.editor.clone())
                    .is_some_and(|e| {
                        e.read(cx).snapshot().buffer.to_string() == "SELECT 3 AS three;"
                    })
        })
    });
    // The original console is untouched.
    assert_eq!(h.text(), "");
    stop_core_runtime_test(cx);
}

/// m7: an answer to an older search never replaces a newer one.
#[cfg(unix)]
#[gpui::test]
fn a_stale_answer_is_dropped(cx: &mut TestAppContext) {
    let h = history(cx, &["SELECT 1 AS mine;"]);
    h.open();
    // Two searches started in one update: the older (succeeded: one hit) cannot answer before
    // the newer (failed only: none) has started.
    let typed = h.shown().searches;
    h.vcx.cx.update(|cx| {
        dialog::pick_outcome(1, cx);
        dialog::pick_outcome(2, cx);
    });
    wait_until(h.vcx, "the newer answer", |_| {
        let s = h.shown();
        s.searches > typed && s.answered == s.searches
    });
    h.vcx.run_until_parked();
    let s = h.shown();
    assert!(
        s.statements.is_empty(),
        "the older answer replaced the newer one"
    );
    assert!(s.stale >= 1, "the older answer was dropped");
    stop_core_runtime_test(cx);
}

// ## m3b-t29-shots (named exception; the T27 review's m6)
/// The declarations one fresh frame of the window makes.
fn declared(vcx: &VisualTestContext) -> Vec<hacivat_ui::popover::A11yDecl> {
    hacivat_ui::popover::a11y_record();
    in_window(vcx, |window, _| window.refresh());
    vcx.run_until_parked();
    hacivat_ui::popover::a11y_take()
}

/// One list option: its name, whether it is selected, its place in the list.
type OptionDecl = (String, Option<bool>, Option<(usize, usize)>);

fn options(decls: &[hacivat_ui::popover::A11yDecl]) -> Vec<OptionDecl> {
    decls
        .iter()
        .filter(|d| d.role == Some(hacivat_ui_core::gpui_surface::Role::ListBoxOption))
        .map(|d| (d.name.to_string(), d.selected, d.position))
        .collect()
}

/// The polite live nodes that name an entry and its place.
fn announced(decls: &[hacivat_ui::popover::A11yDecl]) -> Vec<(String, Option<(usize, usize)>)> {
    decls
        .iter()
        .filter(|d| {
            d.live == Some(hacivat_ui_core::gpui_surface::accesskit::Live::Polite)
                && d.role == Some(hacivat_ui_core::gpui_surface::Role::Status)
                && d.position.is_some()
        })
        .map(|d| (d.name.to_string(), d.position))
        .collect()
}

/// Every row is a list option (its first SQL line its name, its place in the list, whether it
/// is selected), and a move with ↑/↓, while the search field keeps the focus, is announced by
/// a polite live node naming the newly selected entry and its place.
#[cfg(unix)]
#[gpui::test]
fn rows_are_list_options_and_a_move_is_announced(cx: &mut TestAppContext) {
    // The first one is the newest, shown first.
    let mut h = history(
        cx,
        &[
            "SELECT 1 AS first;",
            "SELECT 2 AS second;",
            "SELECT 3 AS third;",
        ],
    );
    h.open();
    let before = declared(h.vcx);
    assert_eq!(
        options(&before),
        [
            ("SELECT 1 AS first;".to_string(), Some(true), Some((1, 3))),
            ("SELECT 2 AS second;".to_string(), Some(false), Some((2, 3))),
            ("SELECT 3 AS third;".to_string(), Some(false), Some((3, 3))),
        ]
    );
    assert_eq!(
        announced(&before),
        [("SELECT 1 AS first;".to_string(), Some((1, 3)))]
    );
    h.keys("down");
    assert_eq!(h.shown().selected, Some(1));
    let after = declared(h.vcx);
    assert_eq!(
        options(&after)
            .iter()
            .map(|(_, selected, _)| *selected)
            .collect::<Vec<_>>(),
        [Some(false), Some(true), Some(false)]
    );
    assert_eq!(
        announced(&after),
        [("SELECT 2 AS second;".to_string(), Some((2, 3)))],
        "the move is announced"
    );
    stop_core_runtime_test(cx);
}
