//! M3a Task 1b: the console, results, decor and guard hooks route through the shell, and every
//! stub keeps today's rendering (an unbound console shows the M2 home; the results tile is the
//! M1 one). Also the test support later tasks build on: `register_test_console` over a
//! FakeDriver session, `editor_text`, `rendered_markers`, `stop_core_runtime_test`.

mod support;

use std::cell::Cell;
use std::rc::Rc;

use gpui::TestAppContext;
use hacivat_app::console::Consoles;
use hacivat_app::shell::{
    CloseDecision, ShellHooks, TabKind, set_console_view, set_quit_guard, set_tab_close_guard,
};
use hacivat_ui::actions::{ActionDef, ActionRegistry};
use support::*;

/// The shell's hooks.
fn hooks(vcx: &gpui::VisualTestContext) -> ShellHooks {
    vcx.cx.read(|cx| cx.global::<ShellHooks>().clone())
}

/// Redraws the shell's window and waits until it is drawn.
fn redraw(vcx: &gpui::VisualTestContext) {
    in_window(vcx, |window, _| window.refresh());
    vcx.run_until_parked();
}

#[gpui::test]
fn unbound_console_renders_the_m2_home(cx: &mut TestAppContext) {
    let vcx = shell_with_connections(cx, "en");
    assert!(
        hooks(vcx).console_view.is_some(),
        "console::install sets the hook"
    );
    let active = with_ws(vcx, |ws, _, _| ws.active_tab_kind());
    assert!(matches!(
        active,
        Some(TabKind::Console {
            n: 1,
            profile: None
        })
    ));
    let markers = rendered_markers(vcx);
    assert!(
        markers.iter().any(|m| m == "home-title"),
        "the M2 home shows: {markers:?}"
    );
}

#[gpui::test]
fn unbound_console_without_connections_keeps_the_m1_empty_state(cx: &mut TestAppContext) {
    let vcx = shell(cx, "en");
    let markers = rendered_markers(vcx);
    assert!(
        markers.iter().any(|m| m == "editor-empty"),
        "the M1 empty state shows: {markers:?}"
    );
}

#[gpui::test]
fn console_view_is_built_once_per_tab(cx: &mut TestAppContext) {
    let vcx = shell_with_connections(cx, "en");
    let builds = Rc::new(Cell::new(0u32));
    let counted = builds.clone();
    vcx.cx.update(|cx| {
        set_console_view(
            cx,
            Rc::new(move |tab, n, profile, window, cx| {
                counted.set(counted.get() + 1);
                hacivat_app::console::view::build(tab, n, profile, window, cx)
            }),
        );
    });
    with_ws(vcx, |ws, window, cx| ws.open_console_tab(None, window, cx));
    vcx.run_until_parked();
    for _ in 0..3 {
        redraw(vcx);
    }
    assert_eq!(builds.get(), 1);
    assert!(rendered_markers(vcx).iter().any(|m| m == "home-title"));
}

#[gpui::test]
fn the_console_binding_hook_is_installed_and_opens_unbound(cx: &mut TestAppContext) {
    let vcx = shell_with_connections(cx, "en");
    assert!(
        hooks(vcx).console_binding.is_some(),
        "console::install sets the hook"
    );
    assert!(run(vcx, "workspace.new_console_tab"));
    let kind = with_ws(vcx, |ws, _, _| ws.active_tab_kind());
    assert!(matches!(
        kind,
        Some(TabKind::Console {
            n: 2,
            profile: None
        })
    ));
}

#[gpui::test]
fn empty_results_hook_keeps_the_m1_tile(cx: &mut TestAppContext) {
    let vcx = shell(cx, "en");
    assert!(
        hooks(vcx).results_view.is_some(),
        "results::install sets the hook"
    );
    let markers = rendered_markers(vcx);
    assert!(
        markers.iter().any(|m| m == "results-empty"),
        "the M1 results tile shows: {markers:?}"
    );
}

/// Red quads exactly 1 logical px tall in the last frame.
#[cfg(feature = "scene-check")]
fn red_hairlines(vcx: &mut gpui::VisualTestContext) -> Vec<gpui::Bounds<gpui::Pixels>> {
    use hacivat_ui_core::theme::Theme;
    vcx.update(|window, cx| {
        let red = gpui::Rgba::from(Theme::get(cx).tokens.colors.red.to_hsla());
        let scale = window.scale_factor();
        window
            .rendered_scene()
            .quads
            .iter()
            .filter(|q| {
                q.background
                    .as_solid()
                    .is_some_and(|fill| gpui::Rgba::from(fill) == red)
            })
            .map(|q| {
                gpui::Bounds::new(
                    gpui::point(
                        gpui::px(q.bounds.origin.x.0 / scale),
                        gpui::px(q.bounds.origin.y.0 / scale),
                    ),
                    gpui::size(
                        gpui::px(q.bounds.size.width.0 / scale),
                        gpui::px(q.bounds.size.height.0 / scale),
                    ),
                )
            })
            .filter(|b| (f32::from(b.size.height) - 1.0).abs() < 0.01)
            .collect()
    })
}

#[cfg(feature = "scene-check")]
#[gpui::test]
fn hairline_is_one_red_quad_at_the_pane_top(cx: &mut TestAppContext) {
    use hacivat_app::shell::{TabDecor, set_tab_decor};
    let vcx = shell(cx, "en");
    assert!(red_hairlines(vcx).is_empty(), "no decor, no hairline");
    vcx.cx.update(|cx| {
        set_tab_decor(
            cx,
            Rc::new(|_, _| TabDecor {
                hairline: true,
                ..TabDecor::default()
            }),
        );
    });
    redraw(vcx);
    let q = red_hairlines(vcx);
    assert_eq!(q.len(), 1, "one hairline: {q:?}");
    assert_eq!(q[0].size.height, gpui::px(1.0));
}

#[gpui::test]
fn edit_menu_appears_only_with_editor_commands(cx: &mut TestAppContext) {
    let vcx = shell(cx, "en");
    let edit = hacivat_l10n::ids::hv::MENU_EDIT.id;
    let titles = |vcx: &gpui::VisualTestContext| {
        vcx.cx.read(|cx| {
            hacivat_app::menus::menu_model(cx.global::<ActionRegistry>())
                .into_iter()
                .map(|g| g.title_id)
                .collect::<Vec<_>>()
        })
    };
    assert!(!titles(vcx).contains(&edit), "no editor command yet");
    vcx.cx.update(|cx| {
        cx.global_mut::<ActionRegistry>()
            .register(ActionDef {
                id: "editor.undo",
                title_id: "hv-cmd-editor-undo",
                context: "Editor",
                run: Rc::new(|_, _| {}),
                enabled: |_| true,
            })
            .expect("editor.undo is not registered yet");
    });
    assert!(titles(vcx).contains(&edit), "the Edit group shows");
}

#[gpui::test]
fn quit_guard_can_defer_quit(cx: &mut TestAppContext) {
    let vcx = shell(cx, "en");
    let asked = Rc::new(Cell::new(0u32));
    let seen = asked.clone();
    vcx.cx.update(|cx| {
        set_quit_guard(
            cx,
            Rc::new(move |_| {
                seen.set(seen.get() + 1);
                CloseDecision::Defer
            }),
        );
    });
    assert!(run(vcx, "app.quit"));
    vcx.run_until_parked();
    assert_eq!(asked.get(), 1, "the guard was asked");
    assert_eq!(
        vcx.cx.read(hacivat_app::commands::quit_requests),
        0,
        "a deferred quit does not quit"
    );
}

#[gpui::test]
fn closing_the_window_asks_the_quit_guard(cx: &mut TestAppContext) {
    let vcx = shell(cx, "en");
    let asked = Rc::new(Cell::new(0u32));
    let seen = asked.clone();
    vcx.cx.update(|cx| {
        set_quit_guard(
            cx,
            Rc::new(move |_| {
                seen.set(seen.get() + 1);
                CloseDecision::Defer
            }),
        );
    });
    // The title bar's close and the platform's should-close both ask this (fix round 1).
    let decision = in_window(vcx, hacivat_app::commands::request_window_close);
    assert_eq!(decision, CloseDecision::Defer);
    assert_eq!(asked.get(), 1);
    assert_eq!(vcx.cx.read(hacivat_app::commands::quit_requests), 0);
}

#[gpui::test]
fn the_stub_quit_guard_lets_quit_proceed(cx: &mut TestAppContext) {
    let vcx = shell(cx, "en");
    assert!(
        hooks(vcx).quit_guard.is_some(),
        "exec_ui::install sets the guard"
    );
    assert!(run(vcx, "app.quit"));
    vcx.run_until_parked();
    assert_eq!(vcx.cx.read(hacivat_app::commands::quit_requests), 1);
}

#[gpui::test]
fn tab_close_guard_can_defer_and_force_close_closes(cx: &mut TestAppContext) {
    let vcx = shell(cx, "en");
    with_ws(vcx, |ws, window, cx| ws.open_console_tab(None, window, cx));
    vcx.run_until_parked();
    assert_eq!(tab_count(vcx), 2);
    vcx.cx.update(|cx| {
        set_tab_close_guard(cx, Rc::new(|_, _, _| CloseDecision::Defer));
    });
    assert!(run(vcx, "workspace.close_tab"));
    vcx.run_until_parked();
    assert_eq!(tab_count(vcx), 2, "a deferred close keeps the tab");
    let tab = with_ws(vcx, |ws, _, _| ws.active_tab()).expect("an active tab");
    with_ws(vcx, |ws, _, cx| ws.force_close_tab(tab, cx));
    vcx.run_until_parked();
    assert_eq!(tab_count(vcx), 1, "force_close_tab closes without asking");
    vcx.cx.update(|cx| {
        set_tab_close_guard(
            cx,
            Rc::new(hacivat_app::exec_ui::close_prompt::tab_close_guard),
        );
    });
    with_ws(vcx, |ws, window, cx| ws.open_console_tab(None, window, cx));
    vcx.run_until_parked();
    assert!(run(vcx, "workspace.close_tab"));
    vcx.run_until_parked();
    assert_eq!(tab_count(vcx), 1, "the stub guard lets the close proceed");
}

#[gpui::test]
fn register_test_console_binds_a_real_editor_over_a_fake_session(cx: &mut TestAppContext) {
    let vcx = shell_with_connections(cx, "en");
    let profile = test_profile("orders", None, "127.0.0.1");
    let profile_id = profile.id;
    let (session, fake) = test_session(vcx, profile);
    let console = register_test_console(vcx, profile_id, session);
    let tab = with_ws(vcx, |ws, _, _| ws.active_tab()).expect("the console's tab is active");
    let kind = with_ws(vcx, |ws, _, _| ws.tab_kind(tab));
    assert!(matches!(kind, Some(TabKind::Console { profile: Some(p), .. }) if p == profile_id));
    let (by_tab, has_editor, has_session, analysis) = vcx.cx.read(|cx| {
        let s = Consoles::get(cx, console).expect("registered");
        let editor = s.editor.as_ref().expect("an editor").read(cx);
        // The console keeps its language service: Run reuses its analysis (R-M3-22).
        let service = s
            .service
            .as_ref()
            .expect("the console keeps its language service");
        let analysis = service.doc_snapshot(editor.doc(), editor.snapshot().version);
        (
            Consoles::by_tab(cx, tab),
            s.editor.is_some(),
            s.session.is_some(),
            analysis.is_some(),
        )
    });
    assert_eq!(by_tab, Some(console));
    assert!(has_editor && has_session);
    assert!(!analysis, "Task 1b's service has no analysis yet");
    assert_eq!(editor_text(vcx, console), "");
    // Run is fail closed until Task 33: `exec_ui::attach`'s stub says so and sends nothing.
    let before = fake.executed().len();
    let editor = vcx.cx.read(|cx| {
        Consoles::get(cx, console)
            .and_then(|s| s.editor.clone())
            .expect("the console has an editor")
    });
    vcx.cx.update(|cx| {
        editor.update(cx, |view, cx| {
            let req = hacivat_ui_editor::ExecRequest {
                doc: view.doc(),
                action: hacivat_ui_editor::ExecAction::Execute1,
                at: hacivat_ui_editor::ExecAt::Carets(smallvec::smallvec![0]),
                snapshot: view.snapshot(),
            };
            cx.emit(hacivat_ui_editor::EditorEvent::ExecuteRequested(req));
        });
    });
    vcx.run_until_parked();
    assert_eq!(
        toast_texts(vcx),
        ["Running queries isn't available in this console yet."]
    );
    assert_eq!(fake.executed().len(), before, "nothing reached the session");
    stop_core_runtime_test(cx);
}

// ## m3a-t01b-seams (round 2): the two window-close wirings.

/// A quit guard that counts its asks and answers `answer`.
fn counting_quit_guard(vcx: &gpui::VisualTestContext, answer: CloseDecision) -> Rc<Cell<u32>> {
    let asked = Rc::new(Cell::new(0u32));
    let seen = asked.clone();
    vcx.cx.update(|cx| {
        set_quit_guard(
            cx,
            Rc::new(move |_| {
                seen.set(seen.get() + 1);
                answer
            }),
        );
    });
    asked
}

/// The platform's close (macOS's traffic light, Windows' caption close) reaches the quit guard
/// through the should-close callback `Workspace::new` installs: once per close, never twice.
#[gpui::test]
fn the_platform_close_asks_the_quit_guard_once_per_close(cx: &mut TestAppContext) {
    let vcx = shell(cx, "en");
    let asked = counting_quit_guard(vcx, CloseDecision::Defer);
    assert!(!vcx.simulate_close(), "a deferred close keeps the window");
    assert_eq!(asked.get(), 1);
    assert!(!vcx.simulate_close());
    assert_eq!(asked.get(), 2, "one ask per close");
    let proceed = counting_quit_guard(vcx, CloseDecision::Proceed);
    assert!(
        vcx.simulate_close(),
        "the stub's Proceed lets the platform close"
    );
    assert_eq!(proceed.get(), 1);
    assert_eq!(
        vcx.cx.read(hacivat_app::commands::quit_requests),
        0,
        "closing is not app.quit"
    );
}

/// The title bar's client-drawn close (Linux) asks the quit guard once and removes the window
/// only on `Proceed`; removing it does not ask again.
#[gpui::test]
fn the_title_bar_close_asks_the_quit_guard_and_closes_on_proceed(cx: &mut TestAppContext) {
    let vcx = shell(cx, "en");
    let asked = counting_quit_guard(vcx, CloseDecision::Defer);
    let windows = || vcx.cx.update(|cx| cx.windows().len());
    let before = windows();
    let d = in_window(vcx, hacivat_app::commands::close_window);
    assert_eq!(d, CloseDecision::Defer);
    assert_eq!((asked.get(), windows()), (1, before));
    let proceed = counting_quit_guard(vcx, CloseDecision::Proceed);
    let d = in_window(vcx, hacivat_app::commands::close_window);
    vcx.cx.run_until_parked();
    assert_eq!(d, CloseDecision::Proceed);
    assert_eq!(proceed.get(), 1, "removing the window asks nothing more");
    assert_eq!(windows(), before - 1);
}
