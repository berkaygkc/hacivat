//! inputnav-1 fix round 1 (m8): a text field's keys through the real app: the dispatcher under
//! a modal dialog, the active keymap reaching the fields, and the documented precedence of a
//! live outer context's row over a field key. The chords come from each platform's keymap
//! data, never spelled for a host.

mod support;

use std::rc::Rc;

use gpui::{
    App, AppContext as _, Context, Entity, Focusable as _, IntoElement, ParentElement as _, Render,
    TestAppContext, VisualTestContext, Window, div,
};
use hacivat_app::shell::{Layer, Workspace};
use hacivat_ui::input::TextInput;
use hacivat_ui::keymaps::load;
use hacivat_ui::text::TextFn;
use hacivat_ui_core::keymap::{KeyboardLayoutId, KeymapPlatform, chord_string};
use hacivat_ui_core::theme::Theme;
use support::*;

const PLATFORMS: [KeymapPlatform; 3] = [
    KeymapPlatform::MacOs,
    KeymapPlatform::Windows,
    KeymapPlatform::Linux,
];

/// A modal layer holding one text field (as a dialog's form).
struct ModalField {
    input: Entity<TextInput>,
}

impl Render for ModalField {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().child(self.input.clone())
    }
}

fn placeholder() -> TextFn {
    Rc::new(|_: &App| "Host".into())
}

/// Opens a modal layer (context `ConnectionDialog`) with a focused field holding `value`.
fn modal_field(vcx: &VisualTestContext, value: &str) -> Entity<TextInput> {
    let value = value.to_string();
    let input = with_ws(vcx, move |w, window, cx| {
        let input = cx.new(|cx| {
            let mut i = TextInput::new(cx, placeholder());
            i.set_value(value);
            i
        });
        let view = cx.new(|_| ModalField {
            input: input.clone(),
        });
        w.push_layer(
            Layer {
                id: "inputnav-modal",
                context: "ConnectionDialog",
                view: view.into(),
                restore_focus: None,
                modal: true,
            },
            window,
            cx,
        );
        let handle = input.focus_handle(cx);
        window.focus(&handle, cx);
        input
    });
    vcx.run_until_parked();
    input
}

/// The first chord `platform`'s default keymap binds to field command `command`.
fn field_chord(platform: KeymapPlatform, command: &str) -> String {
    load(platform, "hacivat", None)
        .0
        .bindings_for(command, &KeyboardLayoutId("us".into()))
        .first()
        .map(|b| chord_string(&b.chord))
        .unwrap_or_else(|| panic!("{platform:?} binds {command}"))
}

fn modal_context(vcx: &VisualTestContext) -> Option<&'static str> {
    with_ws(vcx, |w: &mut Workspace, _, _| w.modal_context())
}

/// Under a modal dialog the dispatcher resolves only the dialog's context; a field key still
/// reaches the field (it resolves its own keys after the dispatcher), on every platform's
/// keymap.
#[gpui::test]
fn a_field_key_reaches_the_field_under_a_modal_dialog(cx: &mut TestAppContext) {
    for platform in PLATFORMS {
        let vcx = shell_with_keymap(cx, platform, "us", None);
        let input = modal_field(vcx, "public.orders");
        assert_eq!(modal_context(vcx), Some("ConnectionDialog"), "{platform:?}");
        vcx.simulate_keystrokes(&field_chord(platform, "input.move_word_left"));
        vcx.run_until_parked();
        assert_eq!(
            input.read_with(vcx, |i, _| i.selection()),
            (7..7, false),
            "{platform:?}: R-IN-1 stops at `orders`"
        );
        vcx.simulate_keystrokes(&field_chord(platform, "input.delete_word_backward"));
        vcx.run_until_parked();
        assert_eq!(
            input.read_with(vcx, |i, _| i.value().to_string()),
            "orders",
            "{platform:?}"
        );
        close_all_windows(vcx);
    }
}

/// A keymap change (a user row remapping a field command) reaches the fields: `InputKeymap`
/// follows `ActiveKeymap`.
#[gpui::test]
fn a_user_keymap_row_remaps_a_field_key_in_the_app(cx: &mut TestAppContext) {
    let user = "[[binding]]\nchord = \"alt-up\"\ncommand = \"input.move_word_left\"\ncontext = \"TextInput\"\n";
    let vcx = shell_with_keymap(cx, KeymapPlatform::Windows, "us", Some(user));
    let handle = focus_test_input(vcx);
    let input = test_input(vcx, &handle);
    vcx.simulate_input("foo bar");
    vcx.simulate_keystrokes("alt-up");
    vcx.run_until_parked();
    assert_eq!(input.read_with(vcx, |i, _| i.selection()), (4..4, false));
}

/// The documented precedence (ADR 0007 `## inputnav-1`): a live outer context's row on a field
/// chord wins, since the dispatcher sees every key first. CI keeps the shipped keymaps free of
/// such rows; a user row can still shadow a field key, and then the outer command runs.
#[gpui::test]
fn a_user_workspace_row_on_a_field_chord_wins_over_the_field(cx: &mut TestAppContext) {
    let user = "[[binding]]\nchord = \"ctrl-home\"\ncommand = \"workspace.toggle_theme\"\ncontext = \"Workspace\"\n";
    let vcx = shell_with_keymap(cx, KeymapPlatform::Windows, "us", Some(user));
    let handle = focus_test_input(vcx);
    let input = test_input(vcx, &handle);
    vcx.simulate_input("foo");
    let before = vcx.read(|cx| Theme::get(cx).mode);
    vcx.simulate_keystrokes("ctrl-home");
    vcx.run_until_parked();
    assert_ne!(
        vcx.read(|cx| Theme::get(cx).mode),
        before,
        "the Workspace row ran"
    );
    assert_eq!(input.read_with(vcx, |i, _| i.selection()), (3..3, false));
}
