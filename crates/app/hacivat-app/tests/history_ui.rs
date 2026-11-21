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

/// What assistive technology reads from the polite live status node: the AccessKit node its
/// element writes, built by the element's own writer (`popover::node_of`) from the recorded
/// declaration, since the test platforms build no AccessKit tree: (label, position and size).
/// T29 fix round 1 (I3).
fn announced(decls: &[hacivat_ui::popover::A11yDecl]) -> Vec<(String, Option<(usize, usize)>)> {
    use hacivat_ui_core::gpui_surface::{Role, accesskit};
    decls
        .iter()
        .filter(|d| d.role == Some(Role::Status) && d.position.is_some())
        .map(hacivat_ui::popover::node_of)
        .filter(|n| n.live() == Some(accesskit::Live::Polite))
        .map(|n| {
            let label: String = n
                .label()
                .unwrap_or_default()
                .chars()
                .filter(|c| !matches!(c, '\u{2068}' | '\u{2069}'))
                .collect();
            (label, n.position_in_set().zip(n.size_of_set()))
        })
        .collect()
}

/// Every row is a list option (its first SQL line its name, its place in the list, whether it
/// is selected), and a move with ↑/↓, while the search field keeps the focus, is announced by
/// a polite live node naming the newly selected entry and its place in words. Two entries with
/// the same first line still announce a move (fix round 1, I3).
#[cfg(unix)]
#[gpui::test]
fn rows_are_list_options_and_a_move_is_announced(cx: &mut TestAppContext) {
    // The first one is the newest, shown first; the last two differ (the store keeps one entry
    // per statement) but share their first line.
    let mut h = history(
        cx,
        &["SELECT 1 AS first;", "SELECT 2;\n-- a", "SELECT 2;\n-- b"],
    );
    h.open();
    let before = declared(h.vcx);
    assert_eq!(
        options(&before),
        [
            ("SELECT 1 AS first;".to_string(), Some(true), Some((1, 3))),
            ("SELECT 2;".to_string(), Some(false), Some((2, 3))),
            ("SELECT 2;".to_string(), Some(false), Some((3, 3))),
        ]
    );
    assert_eq!(
        announced(&before),
        [("SELECT 1 AS first;, 1 of 3".to_string(), Some((1, 3)))]
    );
    h.keys("down");
    assert_eq!(h.shown().selected, Some(1));
    assert_eq!(
        announced(&declared(h.vcx)),
        [("SELECT 2;, 2 of 3".to_string(), Some((2, 3)))]
    );
    h.keys("down");
    assert_eq!(h.shown().selected, Some(2));
    let after = declared(h.vcx);
    assert_eq!(
        options(&after)
            .iter()
            .map(|(_, selected, _)| *selected)
            .collect::<Vec<_>>(),
        [Some(false), Some(false), Some(true)]
    );
    assert_eq!(
        announced(&after),
        [("SELECT 2;, 3 of 3".to_string(), Some((3, 3)))],
        "a move between identical first lines is announced by its place"
    );
    stop_core_runtime_test(cx);
}

// ---- ## m4-t32-shots (named exception; PO ruling R-W5-14 b): "Clear query history…" ----------

fn strip(s: &str) -> String {
    s.chars()
        .filter(|c| !matches!(c, '\u{2068}' | '\u{2069}'))
        .collect()
}

/// The statements the store keeps, every connection, sorted.
fn kept(h: &History) -> Vec<String> {
    let store = h.store.clone();
    let (tx, rx) = std::sync::mpsc::channel();
    h.rt.spawn(async move {
        let hits = store
            .search(HistoryQuery {
                limit: 1_000,
                ..HistoryQuery::default()
            })
            .await;
        let _ = tx.send(hits);
    });
    let mut out: Vec<String> = rx
        .recv()
        .expect("the store answers")
        .expect("search")
        .into_iter()
        .map(|h| h.record.statement_masked.to_string())
        .collect();
    out.sort();
    out
}

// ## m4-t33b-carries: the question opens once the history folder was looked at (on the core
// runtime: are there damaged backups to offer?).
fn asked(vcx: &VisualTestContext) {
    wait_until(vcx, "the clear question", |vcx| {
        vcx.cx
            .read(|cx| hacivat_app::history::clear::question(cx).is_some())
            && layer_ids(vcx).contains(&"dialog")
    });
}

fn menu_labels(vcx: &VisualTestContext, profile: ProfileId) -> Vec<String> {
    vcx.cx.read(|cx| {
        hacivat_app::connections::explorer::menu_items(cx, profile)
            .iter()
            .filter(|i| !i.separator)
            .map(|i| (i.label)(cx).to_string())
            .collect()
    })
}

#[gpui::test]
fn the_history_menu_clears_this_connection_after_asking_with_cancel_first(cx: &mut TestAppContext) {
    let mut h = history(cx, &["SELECT 1", "SELECT 2"]);
    let other = add_profile(h.vcx, test_profile("billing-dev", None, "127.0.0.2"));
    h.put(record(other, "SELECT 'other'", Outcome::Ok, 0));
    h.open();
    assert_eq!(h.shown().statements.len(), 2);
    // The ⋯ button ends the bar's Tab order (search, scope, outcome, ⋯); Enter opens its menu.
    h.keys("tab tab tab enter");
    assert!(layer_ids(h.vcx).contains(&"menu"), "{:?}", layer_ids(h.vcx));
    h.keys("enter");
    asked(h.vcx);
    assert_eq!(
        strip(&dialog_title(h.vcx)),
        "Clear the query history of orders-dev?"
    );
    assert_eq!(
        h.vcx.cx.read(hacivat_app::history::clear::question),
        Some(h.profile)
    );
    // Cancel is the default: Enter cancels and nothing is deleted.
    h.keys("enter");
    assert!(!layer_ids(h.vcx).contains(&"dialog"));
    assert_eq!(kept(&h), ["SELECT 'other'", "SELECT 1", "SELECT 2"]);
    // Clear history (shift-tab from Cancel, then Enter): only this connection's entries go.
    assert!(run(h.vcx, "history.clear"));
    asked(h.vcx);
    h.keys("shift-tab enter");
    wait_until(h.vcx, "the history cleared", |_| kept(&h).len() == 1);
    assert_eq!(kept(&h), ["SELECT 'other'"]);
    // The open dialog searched again and shows no entry; a toast says so.
    wait_until(h.vcx, "the dialog searched again", |vcx| {
        vcx.cx
            .read(|cx| dialog::shown(cx).is_some_and(|s| s.statements.is_empty()))
    });
    assert!(
        toast_texts(h.vcx)
            .iter()
            .any(|t| strip(t) == "The query history of orders-dev was cleared."),
        "{:?}",
        toast_texts(h.vcx)
    );
    stop_core_runtime_test(cx);
}

#[gpui::test]
fn the_connection_menu_clears_the_selected_connections_history(cx: &mut TestAppContext) {
    let h = history(cx, &["SELECT 1"]);
    let other = add_profile(h.vcx, test_profile("billing-dev", None, "127.0.0.2"));
    h.put(record(other, "SELECT 'other'", Outcome::Ok, 0));
    h.vcx.cx.update(|cx| {
        hacivat_app::connections::ConnectionsModel::select(cx, Some(other));
    });
    h.vcx.run_until_parked();
    // The connection menu offers it, before Delete….
    let labels = menu_labels(h.vcx, other);
    let at = labels.iter().position(|l| l == "Clear query history…");
    assert!(
        at.is_some_and(|i| labels.get(i + 1).map(String::as_str) == Some("Delete…")),
        "{labels:?}"
    );
    assert!(run(h.vcx, "connection.clear_history"));
    asked(h.vcx);
    assert_eq!(
        strip(&dialog_title(h.vcx)),
        "Clear the query history of billing-dev?"
    );
    // Escape cancels.
    h.vcx.simulate_keystrokes("escape");
    h.vcx.run_until_parked();
    assert_eq!(kept(&h), ["SELECT 'other'", "SELECT 1"]);
    assert!(run(h.vcx, "connection.clear_history"));
    asked(h.vcx);
    h.vcx.simulate_keystrokes("shift-tab enter");
    wait_until(h.vcx, "billing-dev cleared", |_| kept(&h).len() == 1);
    assert_eq!(kept(&h), ["SELECT 1"]);
    switch_to(h.vcx, "tr");
    h.vcx.run_until_parked();
    assert!(
        menu_labels(h.vcx, other)
            .iter()
            .any(|l| l == "Sorgu geçmişini temizle…"),
        "{:?}",
        menu_labels(h.vcx, other)
    );
    stop_core_runtime_test(cx);
}

// ## m4-t32-shots (fix round 1, review M-1): a console in the middle of a recall forgets the
// cleared statements: ⌃⌥↑ no longer steps to one.
#[gpui::test]
fn a_recall_in_progress_forgets_a_cleared_history(cx: &mut TestAppContext) {
    let h = history(cx, &["SELECT 'newest'", "SELECT 'older'"]);
    wait_until(h.vcx, "the records in the store", |_| kept(&h).len() == 2);
    let editor = h.editor.clone();
    in_window(h.vcx, |window, cx| {
        let handle = gpui::Focusable::focus_handle(editor.read(cx), cx);
        window.focus(&handle, cx);
    });
    h.vcx.run_until_parked();
    let prev = |h: &History| {
        let editor = editor.clone();
        in_window(h.vcx, |window, cx| {
            editor.update(cx, |e, cx| {
                assert!(e.run_command("editor.history_prev", window, cx))
            });
        });
        h.vcx.run_until_parked();
    };
    prev(&h);
    wait_until(h.vcx, "the newest statement recalled", |_| {
        h.text() == "SELECT 'newest'"
    });
    let done = h
        .vcx
        .cx
        .update(|cx| hacivat_app::history::clear::clear(h.profile, cx));
    assert_eq!(wait_task(h.vcx, done), Some(2));
    // Older: nothing to step to any more (the cleared "older" never comes back).
    prev(&h);
    assert_eq!(h.text(), "SELECT 'newest'");
    stop_core_runtime_test(cx);
}

// ## m4-t32-shots (fix round 1, review M-2; ruling: open the store just for the clear): with
// the history turned off (no store open), the file still holds the connection's statements,
// and "Clear query history…" still deletes them.
#[gpui::test]
fn clearing_works_with_the_history_turned_off(cx: &mut TestAppContext) {
    let h = history(cx, &["SELECT 'kept on disk'"]);
    let other = add_profile(h.vcx, test_profile("billing-dev", None, "127.0.0.2"));
    h.put(record(other, "SELECT 'other'", Outcome::Ok, 0));
    wait_until(h.vcx, "the records in the store", |_| kept(&h).len() == 2);
    // History off: the app's store is closed and forgotten (as after a start with it off).
    let store = h.store.clone();
    let (tx, rx) = std::sync::mpsc::channel();
    h.rt.spawn(async move {
        store.shutdown(std::time::Duration::from_secs(2)).await;
        let _ = tx.send(());
    });
    rx.recv().expect("closed");
    h.vcx.cx.update(|cx| {
        hacivat_app::shell::update_settings(cx, |s| s.history.enabled = false);
        hacivat_app::history::set_store(None, cx);
        hacivat_app::connections::ConnectionsModel::select(cx, Some(h.profile));
    });
    h.vcx.run_until_parked();
    assert!(
        run(h.vcx, "connection.clear_history"),
        "offered with the history off"
    );
    asked(h.vcx);
    assert_eq!(
        strip(&dialog_title(h.vcx)),
        "Clear the query history of orders-dev?"
    );
    h.vcx.simulate_keystrokes("shift-tab enter");
    // The file, read again: only the other connection's statement is left.
    let path = h
        .vcx
        .cx
        .read(|cx| cx.global::<hacivat_app::AppState>().paths.history_db());
    let left = |path: std::path::PathBuf| -> Vec<String> {
        let (tx, rx) = std::sync::mpsc::channel();
        h.rt.spawn(async move {
            let s = HistoryStore::open(path, hacivat_exec::HistoryConfig::default())
                .await
                .expect("opens");
            let hits = s
                .search(HistoryQuery {
                    limit: 100,
                    ..HistoryQuery::default()
                })
                .await
                .expect("search");
            s.shutdown(std::time::Duration::from_secs(2)).await;
            let _ = tx.send(hits);
        });
        rx.recv()
            .expect("answers")
            .into_iter()
            .map(|h| h.record.statement_masked.to_string())
            .collect()
    };
    wait_until(h.vcx, "the toast", |vcx| {
        toast_texts(vcx)
            .iter()
            .any(|t| strip(t) == "The query history of orders-dev was cleared.")
    });
    assert_eq!(left(path), ["SELECT 'other'"]);
    // Nothing reopened the history for recording.
    assert!(h.vcx.cx.read(hacivat_app::history::store).is_none());
    stop_core_runtime_test(cx);
}

// ---- ## m4-t33b-carries: the T32 re-review carries (RR-1, RR-2) and the damaged backups ----

/// Closes the app's store and turns the history off (as after a start with it off); answers
/// the history file's path.
fn history_off(h: &History) -> std::path::PathBuf {
    let store = h.store.clone();
    let (tx, rx) = std::sync::mpsc::channel();
    h.rt.spawn(async move {
        store.shutdown(std::time::Duration::from_secs(2)).await;
        let _ = tx.send(());
    });
    rx.recv().expect("closed");
    h.vcx.cx.update(|cx| {
        hacivat_app::shell::update_settings(cx, |s| s.history.enabled = false);
        hacivat_app::history::set_store(None, cx);
        hacivat_app::connections::ConnectionsModel::select(cx, Some(h.profile));
    });
    h.vcx.run_until_parked();
    h.vcx
        .cx
        .read(|cx| cx.global::<hacivat_app::AppState>().paths.history_db())
}

/// Every statement in the file at `path`, read by a store opened with no retention.
fn statements_in(h: &History, path: std::path::PathBuf) -> Vec<String> {
    let (tx, rx) = std::sync::mpsc::channel();
    h.rt.spawn(async move {
        let none = hacivat_exec::HistoryConfig {
            per_source: 0,
            max_age_days: 0,
            ..hacivat_exec::HistoryConfig::default()
        };
        let s = HistoryStore::open(path, none).await.expect("opens");
        let hits = s
            .search(HistoryQuery {
                limit: 100,
                ..HistoryQuery::default()
            })
            .await
            .expect("search");
        s.shutdown(std::time::Duration::from_secs(2)).await;
        let _ = tx.send(hits);
    });
    let mut out: Vec<String> = rx
        .recv()
        .expect("answers")
        .into_iter()
        .map(|h| h.record.statement_masked.to_string())
        .collect();
    out.sort();
    out
}

fn backups_of(h: &History, path: std::path::PathBuf) -> Vec<std::path::PathBuf> {
    futures::executor::block_on(h.rt.spawn(HistoryStore::damaged_backups(path)))
        .expect("the task")
        .expect("listed")
}

// RR-2: with the history off, the store opened just for the clear finds the file damaged. It
// sets the file aside (as a start does), but the toast says the history wasn't cleared, never
// "cleared".
#[gpui::test]
fn clearing_a_damaged_file_with_the_history_off_says_it_was_not_cleared(cx: &mut TestAppContext) {
    let h = history(cx, &["SELECT 'kept on disk'"]);
    wait_until(h.vcx, "the records in the store", |_| kept(&h).len() == 1);
    let path = history_off(&h);
    std::fs::write(&path, b"not a database".repeat(200)).expect("damage the file");
    assert!(run(h.vcx, "connection.clear_history"));
    asked(h.vcx);
    h.vcx.simulate_keystrokes("shift-tab enter");
    let damaged = "The history file was damaged, so Hacivat set it aside as a backup. The query \
                   history of orders-dev wasn't cleared from it; clear again to delete the \
                   damaged backup.";
    wait_until(h.vcx, "the toast", |vcx| {
        toast_texts(vcx).iter().any(|t| strip(t) == damaged)
    });
    assert!(
        !toast_texts(h.vcx)
            .iter()
            .any(|t| strip(t).contains("was cleared")),
        "{:?}",
        toast_texts(h.vcx)
    );
    assert_eq!(backups_of(&h, path).len(), 1, "set aside as a start would");
    stop_core_runtime_test(cx);
}

// RR-2: the store opened just for the clear runs no retention: another connection's old
// statement (older than the 90-day limit) survives the clear of this one.
#[gpui::test]
fn clearing_with_the_history_off_runs_no_retention(cx: &mut TestAppContext) {
    let h = history(cx, &["SELECT 'mine'"]);
    let other = add_profile(h.vcx, test_profile("billing-dev", None, "127.0.0.2"));
    wait_until(h.vcx, "the records in the store", |_| kept(&h).len() == 1);
    let path = history_off(&h);
    // Another connection's statement older than the 90-day limit, written by a store that
    // applies no retention.
    let (tx, rx) = std::sync::mpsc::channel();
    let at = path.clone();
    let old = Arc::new(record(
        other,
        "SELECT 'old other'",
        Outcome::Ok,
        100 * 86_400_000,
    ));
    h.rt.spawn(async move {
        let none = hacivat_exec::HistoryConfig {
            per_source: 0,
            max_age_days: 0,
            ..hacivat_exec::HistoryConfig::default()
        };
        let s = HistoryStore::open(at, none).await.expect("opens");
        s.sink().send(old).await.expect("sent");
        s.shutdown(std::time::Duration::from_secs(2)).await;
        let _ = tx.send(());
    });
    rx.recv().expect("written");
    assert_eq!(
        statements_in(&h, path.clone()),
        ["SELECT 'mine'", "SELECT 'old other'"]
    );
    assert!(run(h.vcx, "connection.clear_history"));
    asked(h.vcx);
    h.vcx.simulate_keystrokes("shift-tab enter");
    wait_until(h.vcx, "the toast", |vcx| {
        toast_texts(vcx)
            .iter()
            .any(|t| strip(t) == "The query history of orders-dev was cleared.")
    });
    assert_eq!(statements_in(&h, path), ["SELECT 'old other'"]);
    stop_core_runtime_test(cx);
}

// The T32 carry (ruling 2026-11-20): damaged backups (`history.sqlite.corrupt-*`) keep every
// connection's old statements and can't be opened to clear one connection. The question says
// so and offers "Also delete damaged history backups", off by default; checked, the clear
// deletes them.
#[gpui::test]
fn the_question_offers_to_delete_damaged_backups_off_by_default(cx: &mut TestAppContext) {
    let h = history(cx, &["SELECT 1"]);
    wait_until(h.vcx, "the records in the store", |_| kept(&h).len() == 1);
    let path = h
        .vcx
        .cx
        .read(|cx| cx.global::<hacivat_app::AppState>().paths.history_db());
    h.vcx.cx.update(|cx| {
        hacivat_app::connections::ConnectionsModel::select(cx, Some(h.profile));
    });
    // No backup: no word of it, no checkbox.
    assert!(run(h.vcx, "connection.clear_history"));
    asked(h.vcx);
    assert!(
        !dialog_body(h.vcx).contains("damaged"),
        "{}",
        dialog_body(h.vcx)
    );
    assert!(
        h.vcx
            .debug_bounds("checkbox-history-clear-backups")
            .is_none()
    );
    h.vcx.simulate_keystrokes("escape");
    h.vcx.run_until_parked();
    // Two damaged copies beside the live file.
    let beside = |suffix: &str| {
        let mut s = path.as_os_str().to_owned();
        s.push(suffix);
        std::path::PathBuf::from(s)
    };
    let copy = beside(".corrupt-1700000000");
    let copy2 = beside(".corrupt-1700000001");
    std::fs::write(&copy, b"old damaged history").expect("a backup");
    std::fs::write(&copy2, b"old damaged history").expect("a backup");
    assert!(run(h.vcx, "connection.clear_history"));
    asked(h.vcx);
    assert_eq!(
        strip(&dialog_body(h.vcx)),
        "Every statement saved for this connection is deleted from this computer, pinned ones \
         too. This can't be undone. Nothing on the server changes. Hacivat also keeps 2 damaged \
         backups of the history, set aside when the file was repaired. They may still hold this \
         connection's statements, and every other connection's."
    );
    assert!(
        h.vcx
            .debug_bounds("checkbox-history-clear-backups")
            .is_some()
    );
    // Default (off): clearing keeps the backups.
    h.vcx.simulate_keystrokes("shift-tab enter");
    wait_until(h.vcx, "cleared", |_| kept(&h).is_empty());
    wait_until(h.vcx, "the toast", |vcx| {
        toast_texts(vcx)
            .iter()
            .any(|t| strip(t) == "The query history of orders-dev was cleared.")
    });
    assert!(copy.exists() && copy2.exists(), "off by default");
    // Checked (Tab from Cancel reaches the checkbox first, Space checks it), then Clear.
    h.put(record(h.profile, "SELECT 2", Outcome::Ok, 0));
    wait_until(h.vcx, "the new record", |_| kept(&h).len() == 1);
    assert!(run(h.vcx, "connection.clear_history"));
    asked(h.vcx);
    h.vcx.simulate_keystrokes("tab space");
    h.vcx.run_until_parked();
    h.vcx.simulate_keystrokes("tab enter");
    wait_until(h.vcx, "the backups deleted", |_| {
        !copy.exists() && !copy2.exists()
    });
    wait_until(h.vcx, "cleared", |_| kept(&h).is_empty());
    wait_until(h.vcx, "the toast", |vcx| {
        toast_texts(vcx).iter().any(|t| {
            strip(t)
                == "The query history of orders-dev was cleared, and 2 damaged backups were \
                    deleted."
        })
    });
    assert!(path.exists(), "the live history stays");
    // Turkish.
    switch_to(h.vcx, "tr");
    std::fs::write(&copy, b"old damaged history").expect("a backup");
    assert!(run(h.vcx, "connection.clear_history"));
    asked(h.vcx);
    assert!(
        strip(&dialog_body(h.vcx)).contains("1 hasarlı yedeğini"),
        "{}",
        dialog_body(h.vcx)
    );
    h.vcx.simulate_keystrokes("escape");
    h.vcx.run_until_parked();
    stop_core_runtime_test(cx);
}

// RR-1: a recall whose read was already on its way when the clear started never brings the
// cleared statements back (a per-profile "cleared" generation). The recall is sent, then the
// clear starts before the foreground runs again; repeated, so the
// order the two answers arrive in varies.
#[gpui::test]
fn a_recall_read_in_flight_during_a_clear_is_dropped(cx: &mut TestAppContext) {
    let h = history(cx, &[]);
    let editor = h.editor.clone();
    in_window(h.vcx, |window, cx| {
        let handle = gpui::Focusable::focus_handle(editor.read(cx), cx);
        window.focus(&handle, cx);
    });
    h.vcx.run_until_parked();
    for round in 0..12 {
        let sql = format!("SELECT {round} AS cleared");
        h.put(record(h.profile, &sql, Outcome::Ok, 0));
        wait_until(h.vcx, "the record", |_| kept(&h) == [sql.clone()]);
        let editor = editor.clone();
        let profile = h.profile;
        // The recall's read is sent (its update ends), then the clear starts, before the
        // foreground runs again.
        in_window(h.vcx, |window, cx| {
            editor.update(cx, |e, cx| {
                assert!(e.run_command("editor.history_prev", window, cx))
            });
        });
        let done = h
            .vcx
            .cx
            .update(|cx| hacivat_app::history::clear::clear(profile, cx));
        assert_eq!(wait_task(h.vcx, done), Some(1));
        h.vcx.run_until_parked();
        assert_eq!(h.text(), "", "round {round}: a cleared statement came back");
    }
    stop_core_runtime_test(cx);
}

// ---- ## m4-t33b-carries (fix round 1: review 3, 6, 7) ----------------------------------------

// Review 3: history off, the live file damaged, one older backup, the box checked. Only the
// listed backup is deleted; the copy this clear set aside stays, so the toast's "clear again to
// delete the damaged backup" is true.
#[gpui::test]
fn a_damaged_live_file_keeps_its_new_backup_when_the_listed_ones_are_deleted(
    cx: &mut TestAppContext,
) {
    let h = history(cx, &["SELECT 'kept on disk'"]);
    wait_until(h.vcx, "the records in the store", |_| kept(&h).len() == 1);
    let path = history_off(&h);
    let mut s = path.as_os_str().to_owned();
    s.push(".corrupt-1700000000");
    let listed = std::path::PathBuf::from(s);
    std::fs::write(&listed, b"old damaged history").expect("a backup");
    std::fs::write(&path, b"not a database".repeat(200)).expect("damage the file");
    assert!(run(h.vcx, "connection.clear_history"));
    asked(h.vcx);
    h.vcx.simulate_keystrokes("tab space");
    h.vcx.run_until_parked();
    h.vcx.simulate_keystrokes("tab enter");
    let damaged = "The history file was damaged, so Hacivat set it aside as a backup. The query \
                   history of orders-dev wasn't cleared from it; clear again to delete the \
                   damaged backup.";
    wait_until(h.vcx, "the toast", |vcx| {
        toast_texts(vcx).iter().any(|t| strip(t) == damaged)
    });
    assert!(!listed.exists(), "the listed backup is deleted");
    let left = backups_of(&h, path);
    assert_eq!(
        left.len(),
        1,
        "the copy this clear set aside stays: {left:?}"
    );
    assert_ne!(left[0], listed);
    stop_core_runtime_test(cx);
}

// Review 6: the entries are cleared, then the backups can't be deleted: the toast says both.
#[cfg(unix)]
#[gpui::test]
fn a_failed_backup_delete_after_a_clear_says_the_entries_were_cleared(cx: &mut TestAppContext) {
    use std::os::unix::fs::PermissionsExt;
    let h = history(cx, &["SELECT 1"]);
    wait_until(h.vcx, "the records in the store", |_| kept(&h).len() == 1);
    let path = h
        .vcx
        .cx
        .read(|cx| cx.global::<hacivat_app::AppState>().paths.history_db());
    let mut s = path.as_os_str().to_owned();
    s.push(".corrupt-1700000000");
    let backup = std::path::PathBuf::from(s);
    std::fs::write(&backup, b"old damaged history").expect("a backup");
    h.vcx.cx.update(|cx| {
        hacivat_app::connections::ConnectionsModel::select(cx, Some(h.profile));
    });
    assert!(run(h.vcx, "connection.clear_history"));
    asked(h.vcx);
    h.vcx.simulate_keystrokes("tab space");
    h.vcx.run_until_parked();
    // The folder turns read-only: the files can be written, not unlinked.
    let dir = path.parent().expect("the history folder").to_path_buf();
    let mode = std::fs::metadata(&dir)
        .expect("folder")
        .permissions()
        .mode();
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o500)).expect("chmod");
    h.vcx.simulate_keystrokes("tab enter");
    let want = "The query history of orders-dev was cleared, but its damaged backups couldn't \
                be deleted.";
    wait_until(h.vcx, "the toast", |vcx| {
        toast_texts(vcx).iter().any(|t| strip(t) == want)
    });
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(mode)).expect("chmod back");
    assert!(kept(&h).is_empty(), "the entries are cleared");
    assert!(backup.exists());
    stop_core_runtime_test(cx);
}

// Review 7: a second ask while the first question's folder listing is still on its way opens
// no second question.
#[gpui::test]
fn asking_twice_opens_one_question(cx: &mut TestAppContext) {
    let h = history(cx, &["SELECT 1"]);
    wait_until(h.vcx, "the records in the store", |_| kept(&h).len() == 1);
    let profile = h.profile;
    h.vcx.cx.update(|cx| {
        hacivat_app::history::clear::ask(profile, cx);
        hacivat_app::history::clear::ask(profile, cx);
    });
    asked(h.vcx);
    for _ in 0..5 {
        h.vcx.run_until_parked();
    }
    assert_eq!(
        layer_ids(h.vcx).iter().filter(|l| **l == "dialog").count(),
        1,
        "{:?}",
        layer_ids(h.vcx)
    );
    // Once it is answered, it can be asked again.
    h.vcx.simulate_keystrokes("escape");
    h.vcx.run_until_parked();
    h.vcx
        .cx
        .update(|cx| hacivat_app::history::clear::ask(profile, cx));
    asked(h.vcx);
    stop_core_runtime_test(cx);
}

// ---- ## m4-t33b-carries (fix round 2, re-review R-1): the in-flight guard never sticks ------

// A question whose layer goes without a dialog event (the workspace's own Escape path,
// `pop_layer`) leaves the profile askable again.
#[gpui::test]
fn a_question_popped_without_an_event_can_be_asked_again(cx: &mut TestAppContext) {
    let h = history(cx, &["SELECT 1"]);
    wait_until(h.vcx, "the records in the store", |_| kept(&h).len() == 1);
    let profile = h.profile;
    h.vcx
        .cx
        .update(|cx| hacivat_app::history::clear::ask(profile, cx));
    asked(h.vcx);
    with_ws(h.vcx, |ws, window, cx| {
        ws.pop_layer(window, cx);
    });
    h.vcx.run_until_parked();
    assert!(!layer_ids(h.vcx).contains(&"dialog"));
    h.vcx
        .cx
        .update(|cx| hacivat_app::history::clear::ask(profile, cx));
    asked(h.vcx);
    stop_core_runtime_test(cx);
}

// With no workspace window when the folder listing answers, no question opens, and the
// profile is not left "being asked".
#[gpui::test]
fn a_question_with_no_window_to_open_in_leaves_no_guard(cx: &mut TestAppContext) {
    let h = history(cx, &["SELECT 1"]);
    wait_until(h.vcx, "the records in the store", |_| kept(&h).len() == 1);
    let profile = h.profile;
    in_window(h.vcx, |window, cx| {
        hacivat_app::history::clear::ask(profile, cx);
        window.remove_window();
    });
    wait_until(h.vcx, "the listing answered, the guard gone", |vcx| {
        vcx.cx
            .read(|cx| !hacivat_app::history::clear::asking(profile, cx))
    });
    assert_eq!(h.vcx.cx.read(hacivat_app::history::clear::question), None);
    stop_core_runtime_test(cx);
}
