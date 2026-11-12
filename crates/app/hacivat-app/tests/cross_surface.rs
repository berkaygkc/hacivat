//! Cross-surface tests (Task 19, F3): the palette (Task 16) over the settings tab (Task 17).
//! Line 4 of the full test set runs them with `scene-check` and `pseudo`.

mod support;

use std::cell::RefCell;
use std::rc::Rc;

use gpui::{
    Entity, EntityInputHandler as _, FocusHandle, Focusable as _, Global, Modifiers,
    TestAppContext, VisualTestContext,
};
use hacivat_app::shell::TabKind;
use hacivat_ui::input::{FocusedInput, TextInput};
use hacivat_ui_core::keymap::parse_chord;
use support::*;

// ---- helpers -----------------------------------------------------------------------------------

/// `cmd-<key>` on macOS, `ctrl-<key>` elsewhere: the primary modifier of the default keymaps
/// (the same helper as `tests/commands.rs`; `primary_chords_are_the_default_bindings` pins it).
fn primary(key: &str) -> String {
    if cfg!(target_os = "macos") {
        format!("cmd-{key}")
    } else {
        format!("ctrl-{key}")
    }
}

/// Every `TextInput` built in the app, newest last.
#[derive(Default, Clone)]
struct BuiltInputs(Rc<RefCell<Vec<Entity<TextInput>>>>);

impl Global for BuiltInputs {}

/// `support::shell`, recording every `TextInput` the app builds from now on (the settings
/// search among them), so a test can drive the input behind a focus handle.
fn shell(cx: &mut TestAppContext, lang: &str) -> &'static mut VisualTestContext {
    let vcx = support::shell(cx, lang);
    vcx.cx.update(|cx| {
        let built = BuiltInputs::default();
        cx.set_global(built.clone());
        cx.observe_new::<TextInput>(move |_, _, cx| built.0.borrow_mut().push(cx.entity()))
            .detach();
    });
    vcx
}

/// The recorded input whose focus handle is `handle`.
fn input_of(vcx: &VisualTestContext, handle: &FocusHandle) -> Entity<TextInput> {
    vcx.cx.read(|cx| {
        cx.global::<BuiltInputs>()
            .0
            .borrow()
            .iter()
            .find(|i| i.focus_handle(cx) == *handle)
            .cloned()
            .expect("a recorded text input with this handle")
    })
}

/// Makes the shell's window the active one and repaints: a `TextInput` publishes itself as the
/// `FocusedInput` (which the dispatcher reads for `composing`) only in the active window.
fn activate(vcx: &VisualTestContext) {
    in_window(vcx, |window, cx| {
        window.activate_window();
        cx.refresh_windows();
    });
    vcx.run_until_parked();
}

/// Clicks the settings search field (the settings tab is active) and returns the focused handle,
/// which is the search input's.
fn focus_settings_search(vcx: &mut VisualTestContext) -> FocusHandle {
    activate(vcx);
    let at = vcx
        .debug_bounds("settings-search")
        .expect("the settings search field is on screen")
        .center();
    vcx.simulate_click(at, Modifiers::none());
    vcx.run_until_parked();
    let handle = focused(vcx).expect("the click focused something");
    // The focused element is a recorded text input: the settings search.
    let input = input_of(vcx, &handle);
    assert_eq!(vcx.cx.read(|cx| input.focus_handle(cx)), handle);
    handle
}

/// Starts an IME composition of `text` in the input focused by `handle` (`replace_and_mark`).
fn compose_in(vcx: &VisualTestContext, handle: &FocusHandle, text: &str) {
    activate(vcx);
    let input = input_of(vcx, handle);
    in_window(vcx, |window, cx| {
        input.update(cx, |i, cx| {
            i.replace_and_mark_text_in_range(None, text, None, window, cx)
        })
    });
    vcx.run_until_parked();
    assert!(vcx.read(|cx| cx.global::<FocusedInput>().composing));
}

/// Commits the composition in the input focused by `handle` (`unmark_text`).
fn commit_composition(vcx: &VisualTestContext, handle: &FocusHandle) {
    let input = input_of(vcx, handle);
    in_window(vcx, |window, cx| {
        input.update(cx, |i, cx| i.unmark_text(window, cx))
    });
    vcx.run_until_parked();
    assert!(!vcx.read(|cx| cx.global::<FocusedInput>().composing));
}

/// The kind of the focused pane's active tab.
fn active_tab_kind(vcx: &VisualTestContext) -> TabKind {
    let ws = workspace(vcx);
    vcx.cx
        .read(|cx| ws.read(cx).active_tab_kind())
        .expect("a tab is active")
}

// ---- tests -------------------------------------------------------------------------------------

/// `primary("k")` and `primary("w")` are the chords the shipped keymap binds to the palette and
/// to closing a tab, so the tests below press the real bindings.
#[gpui::test]
fn primary_chords_are_the_default_bindings(cx: &mut TestAppContext) {
    let vcx = shell(cx, "en");
    let km = active_keymap(vcx);
    for (key, command) in [
        ("k", "workspace.command_palette"),
        ("w", "workspace.close_tab"),
    ] {
        let chord = parse_chord(&primary(key)).expect("a valid chord");
        let bound = km
            .keymap
            .bindings_for(command, &km.layout)
            .iter()
            .any(|b| b.chord == chord && b.context == "Workspace");
        assert!(bound, "{} is not bound to {command}", primary(key));
    }
}

#[gpui::test]
fn escape_closes_only_topmost_layer_and_restores_focus(cx: &mut TestAppContext) {
    let vcx = shell(cx, "en");
    run(vcx, "workspace.open_settings");
    let search = focus_settings_search(vcx);
    vcx.simulate_keystrokes(&primary("k"));
    assert_eq!(layer_ids(vcx), ["palette"]);
    vcx.simulate_keystrokes("escape");
    assert!(layer_ids(vcx).is_empty());
    assert_eq!(focused(vcx), Some(search));
    assert_eq!(active_tab_kind(vcx), TabKind::Registered("settings"));
    assert_eq!(tab_count(vcx), 2);
}

#[gpui::test]
fn bindings_do_not_fire_while_composing_in_settings_search(cx: &mut TestAppContext) {
    let vcx = shell(cx, "en");
    run(vcx, "workspace.open_settings");
    let search = focus_settings_search(vcx);
    assert_eq!(tab_count(vcx), 2);
    compose_in(vcx, &search, "ş");
    vcx.simulate_keystrokes(&primary("w"));
    assert_eq!(tab_count(vcx), 2);
    commit_composition(vcx, &search);
    vcx.simulate_keystrokes(&primary("w"));
    assert_eq!(tab_count(vcx), 1);
}
