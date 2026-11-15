//! M3a Task 32: the console host. Binding (R-M3-37), the editor tab and its toolbar, production
//! decor and the read-only rule (R-M3-6), sessions per console and the waiting state (R-M3-43),
//! autosave and restore (R-M3-49), the editor status segment (R-M3-11), the editor commands and
//! the Edit menu. Every console here is the real host's (FakeDriver connections through the
//! real pipeline); every wait is on a condition.

mod support;

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use gpui::{Entity, Focusable as _, TestAppContext, VisualContext as _, VisualTestContext};
use hacivat_app::connections::ConnectionsModel;
use hacivat_app::console::{ConsoleId, Consoles};
use hacivat_app::shell::{ShellHooks, TabKind};
use hacivat_driver::fake::FakeDriver;
use hacivat_proto::{DbmsKind, EnvTag, ProfileId};
use hacivat_session::LiveState;
use hacivat_session::session::SessionHandle;
use hacivat_ui::actions::ActionRegistry;
use hacivat_ui_editor::{EditorEvent, EditorView, ExecAt, ExecRequest};
use support::*;

/// The platform's primary modifier with `key` (`cmd-t` on macOS, `ctrl-t` elsewhere).
fn primary(key: &str) -> String {
    if cfg!(target_os = "macos") {
        format!("cmd-{key}")
    } else {
        format!("ctrl-{key}")
    }
}

/// A shell whose PostgreSQL connections go to a FakeDriver, with profile `name` (`env`).
fn shell_with_profile(
    cx: &mut TestAppContext,
    name: &str,
    env: Option<EnvTag>,
    max_connections: u8,
) -> &'static mut VisualTestContext {
    let vcx = shell_with_fake(cx, "en", FakeDriver::new(DbmsKind::Postgres));
    let mut p = test_profile(name, env, "10.0.0.7");
    p.max_connections = max_connections;
    add_profile(vcx, p);
    vcx
}

fn connect_and_wait(vcx: &VisualTestContext, name: &str) {
    connect(vcx, name);
    wait_until(vcx, &format!("{name} to connect"), |vcx| {
        matches!(live(vcx, name), LiveState::Connected(_))
    });
}

fn activate(vcx: &VisualTestContext) {
    in_window(vcx, |window, _| window.activate_window());
    vcx.run_until_parked();
}

/// The console of the focused pane's active tab.
fn active_console(vcx: &VisualTestContext) -> Option<ConsoleId> {
    let tab = with_ws(vcx, |ws, _, _| ws.active_tab())?;
    vcx.cx.read(|cx| Consoles::by_tab(cx, tab))
}

/// Waits until the active tab has a console with an editor; its ID.
fn wait_active_console(vcx: &VisualTestContext) -> ConsoleId {
    wait_until(vcx, "the active tab's console", |vcx| {
        active_console(vcx).is_some()
    });
    active_console(vcx).expect("a console")
}

fn editor_of(vcx: &VisualTestContext, id: ConsoleId) -> Entity<EditorView> {
    vcx.cx.read(|cx| {
        Consoles::get(cx, id)
            .and_then(|s| s.editor.clone())
            .expect("the console's editor")
    })
}

fn session_of(vcx: &VisualTestContext, id: ConsoleId) -> Option<SessionHandle> {
    vcx.cx
        .read(|cx| Consoles::get(cx, id).and_then(|s| s.session.clone()))
}

fn wait_session(vcx: &VisualTestContext, id: ConsoleId) -> SessionHandle {
    wait_until(vcx, "the console's session", |vcx| {
        session_of(vcx, id).is_some()
    });
    session_of(vcx, id).expect("a session")
}

/// Every text the last frame drew, joined.
fn frame_text(vcx: &VisualTestContext) -> String {
    in_window(vcx, |window, _| window.refresh());
    vcx.run_until_parked();
    registry_frame(vcx)
        .into_iter()
        .map(|r| r.text.to_string().replace(['\u{2068}', '\u{2069}'], ""))
        .collect::<Vec<_>>()
        .join(" | ")
}

fn focus_editor(vcx: &VisualTestContext, editor: &Entity<EditorView>) {
    let handle = vcx.cx.read(|cx| editor.focus_handle(cx));
    in_window(vcx, |window, cx| window.focus(&handle, cx));
    vcx.run_until_parked();
}

fn editor_focused(vcx: &VisualTestContext, editor: &Entity<EditorView>) -> bool {
    let handle = vcx.cx.read(|cx| editor.focus_handle(cx));
    in_window(vcx, |window, _| handle.is_focused(window))
}

/// Records the editor's Run requests.
fn record_runs(
    vcx: &VisualTestContext,
    editor: &Entity<EditorView>,
) -> Rc<RefCell<Vec<ExecRequest>>> {
    let seen: Rc<RefCell<Vec<ExecRequest>>> = Rc::default();
    let sink = seen.clone();
    vcx.cx.update(|cx| {
        cx.subscribe(editor, move |_, event: &EditorEvent, _| {
            if let EditorEvent::ExecuteRequested(r) = event {
                sink.borrow_mut().push(r.clone());
            }
        })
        .detach();
    });
    seen
}

fn text(vcx: &VisualTestContext, editor: &Entity<EditorView>) -> String {
    vcx.cx
        .read(|cx| editor.read(cx).snapshot().buffer.to_string())
}

fn tab_titles(vcx: &VisualTestContext) -> Vec<String> {
    let ws = workspace(vcx);
    let tabs = vcx.cx.read(|cx| {
        let ws = ws.read(cx);
        let (panes, _) = ws.layout();
        panes
            .leaves()
            .into_iter()
            .flat_map(|p| panes.panes[&p].tabs.clone())
            .collect::<Vec<_>>()
    });
    let hook = vcx
        .cx
        .read(|cx| cx.global::<ShellHooks>().tab_decor.clone())
        .expect("the decor hook");
    tabs.into_iter()
        .map(|tab| {
            vcx.cx.read(|cx| {
                hook(tab, cx)
                    .title
                    .map(|t| t.to_string().replace(['\u{2068}', '\u{2069}'], ""))
                    .unwrap_or_else(|| ws.read(cx).tab_title(tab, cx).to_string())
            })
        })
        .collect()
}

// ---- binding ---------------------------------------------------------------------------------

#[gpui::test]
fn unbound_console_shows_the_m2_home_and_binds_on_first_connect(cx: &mut TestAppContext) {
    let vcx = shell_with_profile(cx, "orders-dev", Some(EnvTag::Dev), 4);
    let markers = rendered_markers(vcx);
    assert!(
        markers.iter().any(|m| m == "home-title"),
        "an unbound console is the M2 home: {markers:?}"
    );
    assert_eq!(active_console(vcx), None);

    connect_and_wait(vcx, "orders-dev");
    let id = wait_active_console(vcx);
    let pid = profile_id(vcx, "orders-dev");
    // The binding is the tab's (saved with the layout).
    assert!(matches!(
        with_ws(vcx, |ws, _, _| ws.active_tab_kind()),
        Some(TabKind::Console {
            n: 1,
            profile: Some(p)
        }) if p == pid
    ));
    // The first console of a profile uses its default session.
    let session = wait_session(vcx, id);
    let default = vcx.cx.read(|cx| {
        cx.global::<ConnectionsModel>()
            .manager()
            .sessions(pid)
            .and_then(|s| s.default_session())
            .expect("the default session")
    });
    assert_eq!(session.id(), default.id());
    // The tab is titled by the connection; the editor and its toolbar replace the home.
    assert_eq!(tab_titles(vcx), vec!["orders-dev".to_string()]);
    let frame = frame_text(vcx);
    assert!(frame.contains("orders-dev"), "{frame}");
    assert!(frame.contains("Run"), "{frame}");
    let markers = rendered_markers(vcx);
    assert!(!markers.iter().any(|m| m == "home-title"), "{markers:?}");
    stop_core_runtime_test(cx);
}

/// Only a connect binds: a profile that shows up already connected (or comes back from a
/// loss) leaves the unbound console on the M2 home.
#[gpui::test]
fn a_profile_that_appears_connected_binds_nothing(cx: &mut TestAppContext) {
    let vcx = shell_with_profile(cx, "orders-dev", Some(EnvTag::Dev), 4);
    connect_and_wait(vcx, "orders-dev");
    wait_active_console(vcx);
    let LiveState::Connected(info) = live(vcx, "orders-dev") else {
        panic!("connected");
    };
    // An unbound console in focus (not through the binding hook).
    let tab = with_ws(vcx, |ws, window, cx| ws.open_console_tab(None, window, cx)).expect("tab");
    vcx.run_until_parked();
    assert_eq!(active_console(vcx), None);
    // A new profile that is connected the first time the console sees it.
    let other = test_profile("orders-stg", Some(EnvTag::Staging), "10.0.0.8");
    let other_id = other.id;
    vcx.cx.update(|cx| {
        ConnectionsModel::save(
            cx,
            hacivat_session::profile::ProfileDraft {
                is_new: true,
                ..hacivat_session::profile::ProfileDraft::edit(other)
            },
        )
        .detach();
        cx.global_mut::<ConnectionsModel>()
            .set_live(other_id, LiveState::Connected(info.clone()));
    });
    vcx.run_until_parked();
    // A recovery of the first profile.
    let first = profile_id(vcx, "orders-dev");
    vcx.cx.update(|cx| {
        cx.global_mut::<ConnectionsModel>().set_live(
            first,
            LiveState::Reconnecting {
                attempt: 1,
                next_in: None,
            },
        );
    });
    vcx.run_until_parked();
    vcx.cx.update(|cx| {
        cx.global_mut::<ConnectionsModel>()
            .set_live(first, LiveState::Connected(info.clone()));
    });
    vcx.run_until_parked();
    assert_eq!(vcx.cx.read(|cx| Consoles::by_tab(cx, tab)), None);
    assert!(matches!(
        with_ws(vcx, |ws, _, _| ws.tab_kind(tab)),
        Some(TabKind::Console { profile: None, .. })
    ));
    stop_core_runtime_test(cx);
}

#[gpui::test]
fn second_console_opens_its_own_session_and_waits_when_the_budget_is_full(cx: &mut TestAppContext) {
    let vcx = shell_with_profile(cx, "orders-dev", Some(EnvTag::Dev), 2);
    connect_and_wait(vcx, "orders-dev");
    let first = wait_active_console(vcx);
    let first_session = wait_session(vcx, first);
    activate(vcx);

    // ⌘T: Console 2, bound to the capsule's connection, with its own session.
    vcx.simulate_keystrokes(&primary("t"));
    let second = wait_active_console(vcx);
    assert_ne!(second, first);
    let second_session = wait_session(vcx, second);
    assert_ne!(second_session.id(), first_session.id());
    assert_eq!(
        tab_titles(vcx),
        vec!["orders-dev".to_string(), "orders-dev (2)".to_string()]
    );

    // A third console finds the budget full (2 of 2) and waits, with Cancel.
    vcx.simulate_keystrokes(&primary("t"));
    let third = wait_active_console(vcx);
    wait_until(vcx, "the waiting sentence", |vcx| {
        frame_text(vcx).contains("Waiting for a free connection to orders-dev (2 of 2 in use)")
    });
    assert!(session_of(vcx, third).is_none());
    click(vcx, "button-console-waiting-cancel");
    wait_until(vcx, "the wait to be cancelled", |vcx| {
        !frame_text(vcx).contains("Waiting for a free connection")
    });
    assert!(session_of(vcx, third).is_none());
    stop_core_runtime_test(cx);
}

// ---- production ------------------------------------------------------------------------------

#[gpui::test]
fn production_console_shows_the_notice_and_the_hairline(cx: &mut TestAppContext) {
    let vcx = shell_with_profile(cx, "orders-prod", Some(EnvTag::Production), 4);
    connect_and_wait(vcx, "orders-prod");
    let id = wait_active_console(vcx);
    let session = wait_session(vcx, id);
    assert!(
        session.state().borrow().read_only,
        "a production console's session is read-only (R-M3-6)"
    );
    let tab = with_ws(vcx, |ws, _, _| ws.active_tab()).expect("a tab");
    let decor = vcx.cx.read(|cx| {
        let hook = cx.global::<ShellHooks>().tab_decor.clone().expect("hook");
        hook(tab, cx)
    });
    assert!(decor.hairline, "the 1 px production hairline");
    assert_eq!(decor.env, Some(hacivat_ui::capsule::EnvKind::Production));
    let frame = frame_text(vcx);
    assert!(
        frame.contains("Production is read-only in this version"),
        "{frame}"
    );
    // The notice opens its explanation.
    click(vcx, "button-console-prod-notice");
    let frame = frame_text(vcx);
    assert!(frame.contains("Why this console is read-only"), "{frame}");
    assert!(layer_ids(vcx).contains(&"console-prod-popover"));
    stop_core_runtime_test(cx);
}

#[gpui::test]
fn a_profile_tagged_production_while_its_console_is_open_reconnects_read_only(
    cx: &mut TestAppContext,
) {
    let vcx = shell_with_profile(cx, "orders", Some(EnvTag::Dev), 4);
    connect_and_wait(vcx, "orders");
    let id = wait_active_console(vcx);
    let rw = wait_session(vcx, id);
    assert!(!rw.state().borrow().read_only);

    let pid = profile_id(vcx, "orders");
    let mut p = vcx
        .cx
        .read(|cx| (*cx.global::<ConnectionsModel>().get(pid).expect("saved")).clone());
    p.environment = Some(EnvTag::Production);
    save_draft(vcx, hacivat_session::profile::ProfileDraft::edit(p));
    // At once: the read-write session is gone from the console.
    assert!(
        session_of(vcx, id).is_none_or(|s| s.id() != rw.id()),
        "the read-write session does not survive the tag"
    );
    wait_until(vcx, "a read-only session", |vcx| {
        session_of(vcx, id).is_some_and(|s| s.state().borrow().read_only)
    });
    assert_ne!(session_of(vcx, id).expect("session").id(), rw.id());
    stop_core_runtime_test(cx);
}

// ---- the keyboard ----------------------------------------------------------------------------

#[gpui::test]
fn keyboard_only_walkthrough(cx: &mut TestAppContext) {
    let vcx = shell_with_profile(cx, "orders-dev", Some(EnvTag::Dev), 4);
    connect_and_wait(vcx, "orders-dev");
    wait_active_console(vcx);
    activate(vcx);

    vcx.simulate_keystrokes(&primary("t"));
    let id = wait_active_console(vcx);
    let editor = editor_of(vcx, id);
    wait_until(vcx, "the new console's editor to take the focus", |vcx| {
        editor_focused(vcx, &editor)
    });
    let runs = record_runs(vcx, &editor);
    vcx.simulate_input("SELECT 1;");
    assert_eq!(text(vcx, &editor), "SELECT 1;");
    vcx.simulate_keystrokes("shift-home");
    vcx.simulate_keystrokes(&primary("enter"));
    vcx.run_until_parked();
    {
        let runs = runs.borrow();
        assert_eq!(runs.len(), 1, "Run asked once");
        match &runs[0].at {
            ExecAt::Selections(r) => {
                assert_eq!(r.iter().cloned().collect::<Vec<_>>(), vec![0..9usize])
            }
            other => panic!("the selection runs, not {other:?}"),
        }
    }
    // Escape collapses the selection; Ctrl+M makes Tab move the focus; Tab leaves the editor.
    vcx.simulate_keystrokes("escape");
    vcx.simulate_keystrokes("ctrl-m");
    vcx.simulate_keystrokes("tab");
    vcx.run_until_parked();
    assert!(!editor_focused(vcx, &editor), "Tab left the editor");
    assert_eq!(text(vcx, &editor), "SELECT 1;", "Tab inserted nothing");
    stop_core_runtime_test(cx);
}

#[gpui::test]
fn toolbar_run_ends_a_composition_and_keeps_the_editor_focused(cx: &mut TestAppContext) {
    let vcx = shell_with_profile(cx, "orders-dev", Some(EnvTag::Dev), 4);
    connect_and_wait(vcx, "orders-dev");
    let id = wait_active_console(vcx);
    activate(vcx);
    let editor = editor_of(vcx, id);
    focus_editor(vcx, &editor);
    let runs = record_runs(vcx, &editor);
    vcx.simulate_input("SELECT 'ı");
    in_window(vcx, |window, cx| {
        editor.update(cx, |v, cx| {
            gpui::EntityInputHandler::replace_and_mark_text_in_range(
                v, None, "ş", None, window, cx,
            );
        });
    });
    vcx.run_until_parked();
    assert!(vcx.cx.read(|cx| editor.read(cx).status().composing));
    click(vcx, "button-console-run");
    let runs = runs.borrow();
    assert_eq!(runs.len(), 1, "the click ran once");
    assert_eq!(runs[0].snapshot.buffer.to_string(), "SELECT 'ış");
    assert!(!vcx.cx.read(|cx| editor.read(cx).status().composing));
    assert!(editor_focused(vcx, &editor), "the editor keeps the focus");
    drop(runs);
    stop_core_runtime_test(cx);
}

/// Moving the pane focus moves the typing: the focused pane's console editor takes the keyboard
/// focus (ledger N2).
#[gpui::test]
fn focus_next_pane_focuses_that_panes_editor(cx: &mut TestAppContext) {
    let vcx = shell_with_profile(cx, "orders-dev", Some(EnvTag::Dev), 4);
    connect_and_wait(vcx, "orders-dev");
    let left = wait_active_console(vcx);
    activate(vcx);
    let left_editor = editor_of(vcx, left);
    focus_editor(vcx, &left_editor);
    assert!(run(vcx, "workspace.split_right"));
    wait_until(vcx, "the right pane's console", |vcx| {
        active_console(vcx).is_some_and(|c| c != left)
    });
    let right = active_console(vcx).expect("console");
    let right_editor = editor_of(vcx, right);
    wait_until(vcx, "the new console's editor to take the focus", |vcx| {
        editor_focused(vcx, &right_editor)
    });
    assert!(run(vcx, "workspace.focus_next_pane"));
    vcx.run_until_parked();
    assert_eq!(active_console(vcx), Some(left));
    assert!(
        editor_focused(vcx, &left_editor),
        "the left editor has the keyboard"
    );
    vcx.simulate_input("x");
    assert_eq!(text(vcx, &left_editor), "x");
    assert_eq!(text(vcx, &right_editor), "");
    stop_core_runtime_test(cx);
}

/// The menu bar never acts inside a composition (F15): its Edit items wait until the IME has
/// committed (macOS commits on the mouse-down that opens the menu).
#[gpui::test]
fn the_menu_bar_waits_for_the_composition(cx: &mut TestAppContext) {
    let vcx = shell_with_profile(cx, "orders-dev", Some(EnvTag::Dev), 4);
    connect_and_wait(vcx, "orders-dev");
    let id = wait_active_console(vcx);
    activate(vcx);
    let editor = editor_of(vcx, id);
    focus_editor(vcx, &editor);
    vcx.simulate_input("SELECT ");
    in_window(vcx, |window, cx| {
        editor.update(cx, |v, cx| {
            gpui::EntityInputHandler::replace_and_mark_text_in_range(
                v, None, "ş", None, window, cx,
            );
        });
    });
    vcx.run_until_parked();
    let window = vcx.window_handle();
    let menu = |vcx: &mut VisualTestContext, id: &str| {
        vcx.cx.dispatch_action(
            window,
            hacivat_ui_core::gpui_surface::RunCommand {
                id: id.to_string().into(),
            },
        );
        vcx.run_until_parked();
    };
    menu(vcx, "editor.select_all");
    menu(vcx, "editor.undo");
    assert_eq!(text(vcx, &editor), "SELECT ş");
    assert!(vcx.cx.read(|cx| editor.read(cx).status().composing));
    // Committed: the same item acts.
    in_window(vcx, |window, cx| {
        editor.update(cx, |v, cx| {
            gpui::EntityInputHandler::unmark_text(v, window, cx)
        });
    });
    menu(vcx, "editor.select_all");
    assert_eq!(
        vcx.cx
            .read(|cx| editor.read(cx).snapshot().selection.main().range()),
        0.."SELECT ş".len()
    );
    stop_core_runtime_test(cx);
}

#[gpui::test]
fn an_unhandled_key_in_the_open_context_menu_reaches_no_editor_binding(cx: &mut TestAppContext) {
    let vcx = shell_with_profile(cx, "orders-dev", Some(EnvTag::Dev), 4);
    connect_and_wait(vcx, "orders-dev");
    let id = wait_active_console(vcx);
    activate(vcx);
    let editor = editor_of(vcx, id);
    focus_editor(vcx, &editor);
    vcx.simulate_input("SELECT 1;");
    let before = vcx
        .cx
        .read(|cx| editor.read(cx).snapshot().selection.main().head);
    vcx.simulate_keystrokes("shift-f10");
    assert!(
        vcx.cx
            .read(|cx| editor.read(cx).command_state().menu_open())
    );
    // Keys the menu does not handle: an Editor binding would move or delete.
    vcx.simulate_keystrokes("left backspace home");
    vcx.run_until_parked();
    assert_eq!(text(vcx, &editor), "SELECT 1;");
    assert_eq!(
        vcx.cx
            .read(|cx| editor.read(cx).snapshot().selection.main().head),
        before
    );
    let stack = context_stack(vcx);
    assert!(!stack.0.contains(&"Editor"), "{stack:?}");
    stop_core_runtime_test(cx);
}

// ---- commands and the Edit menu --------------------------------------------------------------

#[gpui::test]
fn editor_commands_are_disabled_without_a_focused_editor(cx: &mut TestAppContext) {
    let vcx = shell_with_profile(cx, "orders-dev", Some(EnvTag::Dev), 4);
    let registered = |vcx: &VisualTestContext| {
        vcx.cx.read(|cx| {
            let reg = cx.global::<ActionRegistry>();
            hacivat_ui_editor::commands()
                .iter()
                .map(|c| {
                    let def = reg
                        .get(c.id)
                        .unwrap_or_else(|| panic!("{} registered", c.id));
                    assert_eq!(def.context, "Editor");
                    assert_eq!(def.title_id, c.title_id);
                    (def.enabled)(cx)
                })
                .collect::<Vec<bool>>()
        })
    };
    assert!(registered(vcx).iter().all(|e| !e), "no editor yet");
    // The Edit menu shows the editor's group.
    let edit = vcx.cx.read(|cx| {
        hacivat_app::menus::menu_model(cx.global::<ActionRegistry>())
            .into_iter()
            .find(|g| g.title_id == "hv-menu-edit")
    });
    assert!(
        edit.is_some(),
        "the Edit menu exists once the commands are registered"
    );

    connect_and_wait(vcx, "orders-dev");
    let id = wait_active_console(vcx);
    activate(vcx);
    let editor = editor_of(vcx, id);
    focus_editor(vcx, &editor);
    assert!(
        registered(vcx).iter().all(|e| *e),
        "an editor has the focus"
    );
    assert!(context_stack(vcx).0.contains(&"Editor"));
    // A command runs on the focused editor.
    vcx.simulate_input("SELECT 1;");
    assert!(run(vcx, "editor.select_all"));
    assert_eq!(
        vcx.cx
            .read(|cx| editor.read(cx).snapshot().selection.main().range()),
        0..9
    );
    // Focus elsewhere: disabled again, and nothing runs.
    focus_test_input(vcx);
    assert!(registered(vcx).iter().all(|e| !e));
    assert!(!run(vcx, "editor.select_all"));
    stop_core_runtime_test(cx);
}

#[gpui::test]
fn text_input_copy_still_works_with_the_edit_menu(cx: &mut TestAppContext) {
    let vcx = shell_with_profile(cx, "orders-dev", Some(EnvTag::Dev), 4);
    activate(vcx);
    let handle = focus_test_input(vcx);
    vcx.simulate_input("İzmir");
    vcx.simulate_keystrokes(&primary("a"));
    vcx.simulate_keystrokes(&primary("c"));
    vcx.run_until_parked();
    let copied = vcx
        .cx
        .update(|cx| cx.read_from_clipboard())
        .and_then(|c| c.text());
    assert_eq!(copied.as_deref(), Some("İzmir"));
    assert_eq!(input_value(vcx, &handle), "İzmir");
    // The Edit menu's Copy is the editor's, and it is off while a text input has the focus.
    assert!(!vcx.cx.read(|cx| {
        (cx.global::<ActionRegistry>()
            .get("editor.copy")
            .expect("registered")
            .enabled)(cx)
    }));
    stop_core_runtime_test(cx);
}

#[gpui::test]
fn the_edit_menu_names_what_undo_and_redo_would_do(cx: &mut TestAppContext) {
    let vcx = shell_with_profile(cx, "orders-dev", Some(EnvTag::Dev), 4);
    connect_and_wait(vcx, "orders-dev");
    let id = wait_active_console(vcx);
    activate(vcx);
    let editor = editor_of(vcx, id);
    focus_editor(vcx, &editor);
    let titles = |vcx: &VisualTestContext| {
        vcx.cx
            .read(hacivat_app::console::commands::edit_menu_history_titles)
    };
    assert_eq!(titles(vcx), ("Undo".into(), "Redo".into()));
    vcx.simulate_input("SELECT");
    assert_eq!(titles(vcx).0, "Undo Typing");
    // Select all closes the typing group; deleting the selection is its own group.
    assert!(run(vcx, "editor.select_all"));
    vcx.simulate_keystrokes("backspace");
    assert_eq!(titles(vcx).0, "Undo Delete");
    assert!(run(vcx, "editor.undo"));
    assert_eq!(titles(vcx).1, "Redo Delete");
    stop_core_runtime_test(cx);
}

// ---- the SQL context -------------------------------------------------------------------------

#[gpui::test]
fn a_connected_console_previews_with_its_sql_context(cx: &mut TestAppContext) {
    let vcx = shell_with_profile(cx, "orders-dev", Some(EnvTag::Dev), 4);
    connect_and_wait(vcx, "orders-dev");
    let id = wait_active_console(vcx);
    activate(vcx);
    let editor = editor_of(vcx, id);
    focus_editor(vcx, &editor);
    vcx.simulate_input("-- x\n\n");
    settle_until(vcx, WAIT, || {
        vcx.cx.read(|cx| {
            hacivat_ui_editor::preview::status_message(editor.read(cx), cx)
                .is_some_and(|m| m.contains("Nothing to run here"))
        })
    });
    stop_core_runtime_test(cx);
}

// ---- autosave --------------------------------------------------------------------------------

/// The autosave file of console `n` of profile `id`.
fn console_file(vcx: &VisualTestContext, id: ProfileId, n: u32) -> std::path::PathBuf {
    paths(vcx)
        .consoles_dir(&id.to_string())
        .join(format!("{n}.sql"))
}

#[gpui::test]
fn autosave_writes_off_the_ui_thread_and_restores(cx: &mut TestAppContext) {
    let vcx = shell_with_profile(cx, "orders-dev", Some(EnvTag::Dev), 4);
    connect_and_wait(vcx, "orders-dev");
    let pid = profile_id(vcx, "orders-dev");
    let id = wait_active_console(vcx);
    activate(vcx);
    let editor = editor_of(vcx, id);
    focus_editor(vcx, &editor);
    vcx.simulate_input("SELECT 'İzmir';");
    let file = console_file(vcx, pid, 1);
    // Nothing before the second has passed, and the UI thread never waits for the write.
    assert!(!file.exists());
    vcx.executor().advance_clock(Duration::from_millis(1100));
    wait_until(vcx, "the autosave", |_| {
        std::fs::read_to_string(&file).is_ok_and(|t| t == "SELECT 'İzmir';")
    });
    let host = vcx.cx.read(|cx| {
        let tab = Consoles::get(cx, id).expect("console").tab;
        hacivat_app::console::view::host_of(cx, tab).expect("host")
    });
    assert_eq!(
        vcx.cx.read(|cx| host.read(cx).autosave_path()).as_deref(),
        Some(file.as_path())
    );

    // A console restored with this binding reopens its saved text.
    let restored = console_file(vcx, pid, 2);
    std::fs::create_dir_all(restored.parent().expect("dir")).expect("dir");
    std::fs::write(&restored, "SELECT 2; -- restored").expect("write");
    let tab = with_ws(vcx, |ws, window, cx| {
        ws.open_console_tab(Some(pid), window, cx)
    })
    .expect("a tab");
    wait_until(vcx, "the restored console", |vcx| {
        vcx.cx
            .read(|cx| Consoles::by_tab(cx, tab))
            .is_some_and(|c| editor_text(vcx, c) == "SELECT 2; -- restored")
    });
    stop_core_runtime_test(cx);
}

#[gpui::test]
fn restored_console_over_20_mb_is_refused_and_kept_on_disk(cx: &mut TestAppContext) {
    let vcx = shell_with_profile(cx, "orders-dev", Some(EnvTag::Dev), 4);
    connect_and_wait(vcx, "orders-dev");
    wait_active_console(vcx);
    let pid = profile_id(vcx, "orders-dev");
    let big = console_file(vcx, pid, 2);
    std::fs::create_dir_all(big.parent().expect("dir")).expect("dir");
    let len = (20u64 << 20) + 1;
    std::fs::File::create(&big)
        .and_then(|f| f.set_len(len))
        .expect("a 20 MB file");
    let tab = with_ws(vcx, |ws, window, cx| {
        ws.open_console_tab(Some(pid), window, cx)
    })
    .expect("a tab");
    wait_until(vcx, "the refused console", |vcx| {
        vcx.cx.read(|cx| Consoles::by_tab(cx, tab)).is_some()
    });
    let id = vcx
        .cx
        .read(|cx| Consoles::by_tab(cx, tab))
        .expect("console");
    assert_eq!(editor_text(vcx, id), "");
    let toasts = toast_texts(vcx).join(" ");
    assert!(
        toasts.contains("orders-dev (2)") && toasts.contains("20.0 MB"),
        "{toasts}"
    );
    // Typing in it never writes over the file.
    activate(vcx);
    let editor = editor_of(vcx, id);
    focus_editor(vcx, &editor);
    vcx.simulate_input("SELECT 1;");
    vcx.executor().advance_clock(Duration::from_millis(1100));
    vcx.run_until_parked();
    assert_eq!(std::fs::metadata(&big).expect("kept").len(), len);
    stop_core_runtime_test(cx);
}

// ---- the status segment ----------------------------------------------------------------------

#[gpui::test]
fn typing_notifies_the_status_segment_but_not_the_grid(cx: &mut TestAppContext) {
    let vcx = shell_with_profile(cx, "orders-dev", Some(EnvTag::Dev), 4);
    connect_and_wait(vcx, "orders-dev");
    let id = wait_active_console(vcx);
    activate(vcx);
    let editor = editor_of(vcx, id);
    focus_editor(vcx, &editor);
    let (segment, results, host) = vcx.cx.read(|cx| {
        let state = Consoles::get(cx, id).expect("console");
        let segment = hacivat_app::console::editor_segment(id, cx).expect("a segment");
        let host = hacivat_app::console::view::host_of(cx, state.tab).expect("host");
        (segment, state.results.clone().expect("results"), host)
    });
    let seg_n = Rc::new(std::cell::Cell::new(0u32));
    let res_n = Rc::new(std::cell::Cell::new(0u32));
    let host_n = Rc::new(std::cell::Cell::new(0u32));
    vcx.cx.update(|cx| {
        let s = seg_n.clone();
        let segment: Entity<hacivat_app::console::status::EditorSegment> =
            segment.clone().downcast().expect("the segment entity");
        cx.observe(&segment, move |_, _| s.set(s.get() + 1))
            .detach();
        let r = res_n.clone();
        cx.observe(&results, move |_, _| r.set(r.get() + 1))
            .detach();
        let h = host_n.clone();
        cx.observe(&host, move |_, _| h.set(h.get() + 1)).detach();
    });
    vcx.simulate_input("abc");
    vcx.run_until_parked();
    assert!(seg_n.get() > 0, "the segment follows the caret");
    assert_eq!(
        res_n.get(),
        0,
        "the results (the grid's model) never hear of typing"
    );
    assert_eq!(host_n.get(), 0, "the console host never hears of typing");
    let status = vcx.cx.read(|cx| {
        let segment: Entity<hacivat_app::console::status::EditorSegment> =
            segment.downcast().expect("the segment entity");
        segment.read(cx).status()
    });
    assert_eq!((status.line, status.column), (1, 4));
    stop_core_runtime_test(cx);
}

#[gpui::test]
fn a_large_console_opens_in_reduced_mode_with_its_size_in_the_regional_format(
    cx: &mut TestAppContext,
) {
    let vcx = shell_with_profile(cx, "orders-dev", Some(EnvTag::Dev), 4);
    let pid = profile_id(vcx, "orders-dev");
    // 5 MB of SQL: past T0's 4 MB, below 20 MB.
    let line = "SELECT 'İzmir', 1234567;\n";
    let text = line.repeat((5 << 20) / line.len() + 1);
    let tab = in_window(vcx, |window, cx| {
        hacivat_app::console::view::fixtures::open(pid, &text, window, cx)
    })
    .expect("a console tab");
    wait_until(vcx, "the fixture console", |vcx| {
        vcx.cx.read(|cx| Consoles::by_tab(cx, tab)).is_some()
    });
    let id = vcx
        .cx
        .read(|cx| Consoles::by_tab(cx, tab))
        .expect("console");
    let segment: Entity<hacivat_app::console::status::EditorSegment> = vcx
        .cx
        .read(|cx| hacivat_app::console::editor_segment(id, cx))
        .expect("a segment")
        .downcast()
        .expect("the segment entity");
    assert!(vcx.cx.read(|cx| segment.read(cx).reduced()));
    // The T1 banner's size comes from the console's format-locale size text.
    let frame = frame_text(vcx);
    assert!(
        frame.contains("This document is large (5.0 MB), so the editor is in reduced mode."),
        "{frame}"
    );
    stop_core_runtime_test(cx);
}

#[gpui::test]
fn closing_a_console_tab_forgets_it_and_closes_its_own_session(cx: &mut TestAppContext) {
    let vcx = shell_with_profile(cx, "orders-dev", Some(EnvTag::Dev), 4);
    connect_and_wait(vcx, "orders-dev");
    let first = wait_active_console(vcx);
    let default = wait_session(vcx, first);
    activate(vcx);
    vcx.simulate_keystrokes(&primary("t"));
    let second = wait_active_console(vcx);
    let own = wait_session(vcx, second);
    assert!(run(vcx, "workspace.close_tab"));
    wait_until(vcx, "the console to be forgotten", |vcx| {
        vcx.cx.read(|cx| Consoles::get(cx, second).is_none())
    });
    wait_until(vcx, "its own session to close", |_| {
        matches!(
            own.state().borrow().state,
            hacivat_session::session::SessionState::Disconnected
        )
    });
    // The profile's default session (the first console's) stays.
    assert!(!matches!(
        default.state().borrow().state,
        hacivat_session::session::SessionState::Disconnected
    ));
    assert!(vcx.cx.read(|cx| Consoles::get(cx, first).is_some()));
    stop_core_runtime_test(cx);
}

#[gpui::test]
fn toolbar_fixtures_show_running_waiting_and_the_lock(cx: &mut TestAppContext) {
    let vcx = shell_with_profile(cx, "orders-dev", Some(EnvTag::Dev), 4);
    let pid = profile_id(vcx, "orders-dev");
    let tab = in_window(vcx, |window, cx| {
        hacivat_app::console::view::fixtures::open(pid, "SELECT 1;", window, cx)
    })
    .expect("a console tab");
    wait_until(vcx, "the fixture console", |vcx| {
        vcx.cx.read(|cx| Consoles::by_tab(cx, tab)).is_some()
    });
    vcx.cx.update(|cx| {
        use hacivat_app::console::toolbar::RunUx;
        use hacivat_app::console::view::fixtures;
        fixtures::show_run(tab, RunUx::Running(3), cx);
        fixtures::show_waiting(tab, 4, 4, cx);
        fixtures::show_lock(tab, hacivat_proto::AccessLabel::ReadOnlyGuarded, cx);
    });
    let frame = frame_text(vcx);
    for expected in [
        "Stop",
        "Running",
        "3 s",
        "Waiting for a free connection to orders-dev (4 of 4 in use)",
        "Cancel",
    ] {
        assert!(frame.contains(expected), "{expected}: {frame}");
    }
    stop_core_runtime_test(cx);
}
