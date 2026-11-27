//! inputnav-1 (PO demo finding F1): a single-line `TextInput` has the native keyboard and mouse
//! set on every platform, through the keymap's `TextInput` rows.
//!
//! The chords a test presses are read from each platform's keymap data (`chords_of`), never
//! spelled in the test: the field resolves them against the `InputKeymap` the test installs,
//! so the macOS, Windows and Linux sets all run on any host.

use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    AppContext as _, ClipboardItem, Context, Entity, EntityInputHandler as _, Focusable as _,
    InteractiveElement as _, IntoElement, Modifiers, MouseButton, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, ParentElement as _, Pixels, Point, Render, Role, Styled as _, TestAppContext,
    VisualTestContext, Window, div, point, px,
};
use hacivat_ui::input::TextInput;
use hacivat_ui::input::keys::{INPUT_COMMANDS, InputKeymap, TEXT_INPUT_CONTEXT, set_input_keymap};
use hacivat_ui::keymaps::{PRESETS, load};
use hacivat_ui::popover::{a11y_record, a11y_take};
use hacivat_ui::text::TextFn;
use hacivat_ui_core::keymap::{KeyboardLayoutId, KeymapPlatform, chord_string, parse_chord};
use hacivat_ui_core::theme::{Theme, ThemeMode};

const PLATFORMS: [KeymapPlatform; 3] = [
    KeymapPlatform::MacOs,
    KeymapPlatform::Windows,
    KeymapPlatform::Linux,
];

struct Host {
    input: Entity<TextInput>,
}

impl Render for Host {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().w(px(400.0)).child(self.input.clone())
    }
}

fn placeholder() -> TextFn {
    Rc::new(|_| "Search".into())
}

/// A focused field in a key window whose keys follow `platform`'s default keymap.
fn field(
    cx: &mut TestAppContext,
    platform: KeymapPlatform,
) -> (Entity<TextInput>, &mut VisualTestContext) {
    cx.update(|cx| {
        Theme::install(cx, ThemeMode::Dark);
        cx.set_global(InputKeymap::defaults(platform));
    });
    let (host, vcx) = cx.add_window_view(|_, cx| Host {
        input: cx.new(|cx| TextInput::new(cx, placeholder())),
    });
    let input = host.read_with(vcx, |h, _| h.input.clone());
    vcx.update(|w, cx| {
        w.activate_window();
        input.focus_handle(cx).focus(w, cx);
    });
    vcx.run_until_parked();
    (input, vcx)
}

/// Every chord `platform`'s default keymap binds to `command` (the platform helper: tests
/// press what the keymap says, never a spelled chord).
fn chords_of(platform: KeymapPlatform, command: &str) -> Vec<String> {
    load(platform, "hacivat", None)
        .0
        .bindings_for(command, &KeyboardLayoutId("us".into()))
        .into_iter()
        .map(|b| chord_string(&b.chord))
        .collect()
}

/// Presses and releases `chord` on the focused element.
fn press(vcx: &mut VisualTestContext, chord: &str) {
    vcx.simulate_keystrokes(chord);
    // `Keystroke::parse` wants a `'static` text.
    let chord: &'static str = Box::leak(chord.to_string().into_boxed_str());
    vcx.simulate_event(gpui::KeyUpEvent {
        keystroke: gpui::Keystroke::parse(chord).expect("a keystroke"),
    });
    vcx.run_until_parked();
}

/// Sets the value and the selection (bytes; the text is ASCII, so UTF-16 offsets match).
fn reset(vcx: &mut VisualTestContext, input: &Entity<TextInput>, text: &str, sel: Range<usize>) {
    input.update_in(vcx, |i, w, cx| {
        i.set_value(text.to_string());
        i.set_selected_text_range(sel, w, cx);
    });
    vcx.run_until_parked();
}

fn state(vcx: &mut VisualTestContext, input: &Entity<TextInput>) -> (String, Range<usize>, bool) {
    input.read_with(vcx, |i, _| {
        let (sel, reversed) = i.selection();
        (i.value().to_string(), sel, reversed)
    })
}

/// `(command, value after, selection after, reversed)` from "foo bar-baz" with the caret after
/// "foo b" (byte 5).
const FROM_FOO_B: [(&str, &str, Range<usize>, bool); 21] = [
    // ## inputnav-1 fix round 1 (I2): Windows' word moves (here `-` follows `bar`, so they
    // stop where the word end does; `the_windows_word_move_lands_on_the_next_word` tells the
    // two rules apart).
    ("input.move_next_word_start", "foo bar-baz", 7..7, false),
    ("input.select_next_word_start", "foo bar-baz", 5..7, false),
    ("input.delete_to_next_word_start", "foo b-baz", 5..5, false),
    ("input.move_left", "foo bar-baz", 4..4, false),
    ("input.move_right", "foo bar-baz", 6..6, false),
    ("input.select_left", "foo bar-baz", 4..5, true),
    ("input.select_right", "foo bar-baz", 5..6, false),
    ("input.move_word_left", "foo bar-baz", 4..4, false),
    ("input.move_word_right", "foo bar-baz", 7..7, false),
    ("input.select_word_left", "foo bar-baz", 4..5, true),
    ("input.select_word_right", "foo bar-baz", 5..7, false),
    ("input.move_line_start", "foo bar-baz", 0..0, false),
    ("input.move_line_end", "foo bar-baz", 11..11, false),
    ("input.select_line_start", "foo bar-baz", 0..5, true),
    ("input.select_line_end", "foo bar-baz", 5..11, false),
    ("input.delete_backward", "foo ar-baz", 4..4, false),
    ("input.delete_forward", "foo br-baz", 5..5, false),
    ("input.delete_word_backward", "foo ar-baz", 4..4, false),
    ("input.delete_word_forward", "foo b-baz", 5..5, false),
    ("input.delete_to_line_start", "ar-baz", 0..0, false),
    ("input.delete_to_line_end", "foo b", 5..5, false),
];

/// The commands a platform binds no key to: Windows and Linux fields have no line deletes;
/// Windows word moves right end at the next word's start, the others at the word's end.
fn unbound(platform: KeymapPlatform, command: &str) -> bool {
    let line_delete = command.starts_with("input.delete_to_line");
    let word_end = matches!(
        command,
        "input.move_word_right" | "input.select_word_right" | "input.delete_word_forward"
    );
    let next_start = command.contains("next_word_start");
    match platform {
        KeymapPlatform::MacOs => next_start,
        KeymapPlatform::Windows => line_delete || word_end,
        KeymapPlatform::Linux => line_delete || next_start,
    }
}

fn every_chord_does_what_its_command_says(cx: &mut TestAppContext, platform: KeymapPlatform) {
    let (input, vcx) = field(cx, platform);
    let mut pressed = 0;
    for (command, text, sel, reversed) in FROM_FOO_B {
        let chords = chords_of(platform, command);
        assert_eq!(
            chords.is_empty(),
            unbound(platform, command),
            "{platform:?} binds {command}: {chords:?}"
        );
        for chord in chords {
            reset(vcx, &input, "foo bar-baz", 5..5);
            press(vcx, &chord);
            assert_eq!(
                state(vcx, &input),
                (text.to_string(), sel.clone(), reversed),
                "{platform:?} {chord} ({command})"
            );
            pressed += 1;
        }
    }
    // Select all, from the caret.
    for chord in chords_of(platform, "input.select_all") {
        reset(vcx, &input, "foo bar-baz", 5..5);
        press(vcx, &chord);
        assert_eq!(state(vcx, &input).1, 0..11, "{platform:?} {chord}");
        pressed += 1;
    }
    assert!(pressed >= 18, "{platform:?}: {pressed}");
}

#[gpui::test]
fn macos_chords(cx: &mut TestAppContext) {
    every_chord_does_what_its_command_says(cx, KeymapPlatform::MacOs);
}

#[gpui::test]
fn windows_chords(cx: &mut TestAppContext) {
    every_chord_does_what_its_command_says(cx, KeymapPlatform::Windows);
}

#[gpui::test]
fn linux_chords(cx: &mut TestAppContext) {
    every_chord_does_what_its_command_says(cx, KeymapPlatform::Linux);
}

/// The native sets of the brief, per platform (the data the chord tests read is pinned here).
#[test]
fn the_keymaps_carry_the_native_sets() {
    let has = |p, command: &str, chord: &str| {
        assert!(
            chords_of(p, command)
                .iter()
                .any(|c| *c == chord_string(&parse_chord(chord).expect("a chord"))),
            "{p:?}: {chord} → {command}: {:?}",
            chords_of(p, command)
        );
    };
    let mac = KeymapPlatform::MacOs;
    for (command, chords) in [
        ("input.move_left", &["left", "ctrl-b"][..]),
        ("input.move_right", &["right", "ctrl-f"]),
        ("input.select_left", &["shift-left"]),
        ("input.select_right", &["shift-right"]),
        ("input.move_word_left", &["alt-left"]),
        ("input.move_word_right", &["alt-right"]),
        ("input.select_word_left", &["alt-shift-left"]),
        ("input.select_word_right", &["alt-shift-right"]),
        (
            "input.move_line_start",
            &[
                "cmd-left", "up", "home", "ctrl-a", "cmd-up", "alt-up", "ctrl-p",
            ],
        ),
        (
            "input.move_line_end",
            &[
                "cmd-right",
                "down",
                "end",
                "ctrl-e",
                "cmd-down",
                "alt-down",
                "ctrl-n",
            ],
        ),
        (
            "input.select_line_start",
            &[
                "cmd-shift-left",
                "shift-up",
                "shift-home",
                "cmd-shift-up",
                "alt-shift-up",
            ],
        ),
        (
            "input.select_line_end",
            &[
                "cmd-shift-right",
                "shift-down",
                "shift-end",
                "cmd-shift-down",
                "alt-shift-down",
            ],
        ),
        ("input.undo", &["cmd-z"]),
        ("input.redo", &["cmd-shift-z"]),
        ("input.delete_backward", &["backspace", "ctrl-h"]),
        ("input.delete_forward", &["delete", "ctrl-d"]),
        ("input.delete_word_backward", &["alt-backspace"]),
        ("input.delete_word_forward", &["alt-delete"]),
        ("input.delete_to_line_start", &["cmd-backspace"]),
        ("input.delete_to_line_end", &["ctrl-k"]),
        ("input.select_all", &["cmd-a"]),
        ("input.copy", &["cmd-c"]),
        ("input.cut", &["cmd-x"]),
        ("input.paste", &["cmd-v"]),
    ] {
        for chord in chords {
            has(mac, command, chord);
        }
    }
    for p in [KeymapPlatform::Windows, KeymapPlatform::Linux] {
        for (command, chord) in [
            ("input.select_left", "shift-left"),
            ("input.select_right", "shift-right"),
            ("input.move_word_left", "ctrl-left"),
            ("input.select_word_left", "ctrl-shift-left"),
            ("input.move_line_start", "home"),
            ("input.move_line_end", "end"),
            ("input.select_line_start", "shift-home"),
            ("input.select_line_end", "shift-end"),
            ("input.delete_forward", "delete"),
            ("input.delete_word_backward", "ctrl-backspace"),
            ("input.select_all", "ctrl-a"),
            ("input.copy", "ctrl-c"),
            ("input.cut", "ctrl-x"),
            ("input.paste", "ctrl-v"),
            // ## inputnav-1 fix round 1 (m4)
            ("input.move_line_start", "ctrl-home"),
            ("input.move_line_end", "ctrl-end"),
            ("input.select_line_start", "ctrl-shift-home"),
            ("input.select_line_end", "ctrl-shift-end"),
            ("input.cut", "shift-delete"),
            ("input.copy", "ctrl-insert"),
            ("input.paste", "shift-insert"),
            ("input.undo", "ctrl-z"),
            ("input.redo", "ctrl-y"),
            ("input.redo", "ctrl-shift-z"),
        ] {
            has(p, command, chord);
        }
        // ## inputnav-1 fix round 1 (I2): Ctrl+→ and Ctrl+Delete go to the next word's start
        // on Windows, to the word's end on Linux (GTK).
        let (right, select, delete) = if p == KeymapPlatform::Windows {
            (
                "input.move_next_word_start",
                "input.select_next_word_start",
                "input.delete_to_next_word_start",
            )
        } else {
            (
                "input.move_word_right",
                "input.select_word_right",
                "input.delete_word_forward",
            )
        };
        has(p, right, "ctrl-right");
        has(p, select, "ctrl-shift-right");
        has(p, delete, "ctrl-delete");
        // ↑/↓ stay with the field's parents (a list it filters, a form).
        assert!(
            chords_of(p, "input.move_line_start")
                .iter()
                .all(|c| c != "up")
        );
    }
}

/// Every text-field row is in the `TextInput` context, in both presets.
#[test]
fn text_field_rows_live_in_their_own_context() {
    for p in PLATFORMS {
        for preset in PRESETS {
            for b in load(p, preset, None).0.bindings() {
                if b.command.starts_with("input.") {
                    assert_eq!(
                        b.context, TEXT_INPUT_CONTEXT,
                        "{p:?} {preset} {}",
                        b.command
                    );
                }
            }
        }
    }
    assert_eq!(INPUT_COMMANDS.len(), 27);
}

#[gpui::test]
fn turkish_and_emoji_move_by_grapheme_and_word(cx: &mut TestAppContext) {
    let (input, vcx) = field(cx, KeymapPlatform::MacOs);
    let mac = |c: &str| chords_of(KeymapPlatform::MacOs, c)[0].clone();
    let family = "👨\u{200d}👩\u{200d}👧";
    let text = format!("İstanbul'da {family} ışığı");
    vcx.simulate_input(&text);
    press(vcx, &mac("input.select_word_left"));
    assert_eq!(state(vcx, &input).1, text.len() - "ışığı".len()..text.len());
    press(vcx, &mac("input.move_left"));
    press(vcx, &mac("input.select_left"));
    press(vcx, &mac("input.select_left"));
    // The space, then the whole family (one grapheme).
    let family_start = "İstanbul'da ".len();
    assert_eq!(
        state(vcx, &input).1,
        family_start..family_start + family.len() + 1
    );
    press(vcx, &mac("input.move_word_left"));
    press(vcx, &mac("input.move_word_left"));
    assert_eq!(state(vcx, &input).1, 0..0);
    press(vcx, &mac("input.delete_word_forward"));
    assert_eq!(state(vcx, &input).0, format!(" {family} ışığı"));
}

// ---- clipboard, masked, read-only, IME ----------------------------------------------------------

fn clipboard(vcx: &mut VisualTestContext) -> Option<String> {
    vcx.update(|_, cx| cx.read_from_clipboard())
        .and_then(|item| item.text())
}

fn set_clipboard(vcx: &mut VisualTestContext, text: &str) {
    let item = ClipboardItem::new_string(text.to_string());
    vcx.update(|_, cx| cx.write_to_clipboard(item));
}

#[gpui::test]
fn clipboard_chords_follow_the_platform_keymap(cx: &mut TestAppContext) {
    for platform in [KeymapPlatform::Windows, KeymapPlatform::MacOs] {
        let (input, vcx) = field(cx, platform);
        let key = |c: &str| chords_of(platform, c)[0].clone();
        reset(vcx, &input, "şehir", 0..0);
        press(vcx, &key("input.select_all"));
        press(vcx, &key("input.copy"));
        assert_eq!(clipboard(vcx).as_deref(), Some("şehir"));
        press(vcx, &key("input.move_line_end"));
        press(vcx, &key("input.paste"));
        assert_eq!(state(vcx, &input).0, "şehirşehir");
        press(vcx, &key("input.select_word_left"));
        press(vcx, &key("input.cut"));
        assert_eq!(state(vcx, &input).0, "");
        assert_eq!(clipboard(vcx).as_deref(), Some("şehirşehir"));
    }
}

/// The first chord of `command` on `platform`; on Windows the word moves right are the
/// next-word-start commands (I2).
fn first_chord(platform: KeymapPlatform, command: &str) -> String {
    let command = match (platform, command) {
        (KeymapPlatform::Windows, "input.move_word_right") => "input.move_next_word_start",
        (KeymapPlatform::Windows, "input.select_word_right") => "input.select_next_word_start",
        (KeymapPlatform::Windows, "input.delete_word_forward") => "input.delete_to_next_word_start",
        (_, command) => command,
    };
    chords_of(platform, command)[0].clone()
}

#[gpui::test]
fn masked_fields_treat_the_value_as_one_word_and_never_copy(cx: &mut TestAppContext) {
    for platform in PLATFORMS {
        let (input, vcx) = field(cx, platform);
        input.update(vcx, |i, _| i.set_masked(true));
        let key = |c: &str| first_chord(platform, c);
        let value = "correct horse.battery";
        reset(vcx, &input, value, 9..9);
        press(vcx, &key("input.move_word_left"));
        assert_eq!(state(vcx, &input).1, 0..0, "{platform:?}");
        press(vcx, &key("input.move_word_right"));
        assert_eq!(state(vcx, &input).1, value.len()..value.len());
        press(vcx, &key("input.select_word_left"));
        assert_eq!(state(vcx, &input).1, 0..value.len());
        reset(vcx, &input, value, 9..9);
        press(vcx, &key("input.select_word_right"));
        assert_eq!(state(vcx, &input).1, 9..value.len());
        // Copy and cut stay refused, with a selection.
        set_clipboard(vcx, "sentinel");
        press(vcx, &key("input.select_all"));
        // Every copy and cut chord (fix round 1, m4: ⇧Delete, Ctrl+Insert too).
        for command in ["input.copy", "input.cut"] {
            for chord in chords_of(platform, command) {
                press(vcx, &chord);
            }
        }
        assert_eq!(clipboard(vcx).as_deref(), Some("sentinel"));
        assert_eq!(state(vcx, &input).0, value);
        // ## inputnav-1 fix round 1 (m4): a masked field keeps no undo history.
        vcx.simulate_input("x");
        assert_eq!(input.read_with(vcx, |i, _| i.history_depth()), (0, 0));
        press(vcx, &key("input.undo"));
        assert_eq!(state(vcx, &input).0, "x", "{platform:?}");
        reset(vcx, &input, value, 0..0);
        // Word deletes go to the edge.
        reset(vcx, &input, value, 9..9);
        press(vcx, &key("input.delete_word_forward"));
        assert_eq!(state(vcx, &input).0, "correct h");
        press(vcx, &key("input.delete_word_backward"));
        assert_eq!(state(vcx, &input).0, "");
    }
}

#[gpui::test]
fn read_only_fields_move_and_select_but_never_edit(cx: &mut TestAppContext) {
    for platform in PLATFORMS {
        let (input, vcx) = field(cx, platform);
        input.update(vcx, |i, _| i.set_read_only(true));
        reset(vcx, &input, "foo bar-baz", 5..5);
        for (command, text, sel, reversed) in FROM_FOO_B {
            for chord in chords_of(platform, command) {
                reset(vcx, &input, "foo bar-baz", 5..5);
                press(vcx, &chord);
                let (now, now_sel, now_rev) = state(vcx, &input);
                assert_eq!(now, "foo bar-baz", "{platform:?} {chord}");
                if text == "foo bar-baz" {
                    assert_eq!(
                        (now_sel, now_rev),
                        (sel.clone(), reversed),
                        "{platform:?} {chord}"
                    );
                }
            }
        }
        let key = |c: &str| chords_of(platform, c)[0].clone();
        set_clipboard(vcx, "pasted");
        press(vcx, &key("input.paste"));
        press(vcx, &key("input.select_all"));
        // Cut neither removes nor copies.
        press(vcx, &key("input.cut"));
        assert_eq!(state(vcx, &input).0, "foo bar-baz");
        assert_eq!(clipboard(vcx).as_deref(), Some("pasted"));
        // Copy still works.
        press(vcx, &key("input.copy"));
        assert_eq!(clipboard(vcx).as_deref(), Some("foo bar-baz"));
    }
}

#[gpui::test]
fn keys_stay_with_the_ime_while_composing(cx: &mut TestAppContext) {
    let (input, vcx) = field(cx, KeymapPlatform::MacOs);
    vcx.simulate_input("abc ");
    input.update_in(vcx, |i, w, cx| {
        i.replace_and_mark_text_in_range(None, "漢字", Some(2..2), w, cx)
    });
    for command in [
        "input.move_word_left",
        "input.select_line_start",
        "input.delete_word_backward",
        "input.select_all",
    ] {
        press(vcx, &chords_of(KeymapPlatform::MacOs, command)[0]);
    }
    input.update_in(vcx, |i, w, cx| {
        assert!(i.is_composing());
        assert_eq!(i.value(), "abc 漢字");
        assert_eq!(i.marked_text_range(w, cx), Some(4..6));
        assert_eq!(i.selected_text_range(false, w, cx).unwrap().range, 6..6);
    });
}

#[gpui::test]
fn the_platform_sees_the_selection_and_its_direction(cx: &mut TestAppContext) {
    let (input, vcx) = field(cx, KeymapPlatform::Windows);
    reset(vcx, &input, "ağaç", 6..6);
    press(
        vcx,
        &chords_of(KeymapPlatform::Windows, "input.select_left")[0],
    );
    press(
        vcx,
        &chords_of(KeymapPlatform::Windows, "input.select_left")[0],
    );
    input.update_in(vcx, |i, w, cx| {
        let sel = i.selected_text_range(false, w, cx).unwrap();
        // UTF-16: "ağaç" is four units; the head leads at 2.
        assert_eq!((sel.range, sel.reversed), (2..4, true));
    });
}

#[gpui::test]
fn the_accessible_value_follows_edits(cx: &mut TestAppContext) {
    let (input, vcx) = field(cx, KeymapPlatform::Linux);
    reset(vcx, &input, "foo bar-baz", 11..11);
    press(
        vcx,
        &chords_of(KeymapPlatform::Linux, "input.delete_word_backward")[0],
    );
    a11y_record();
    vcx.update(|window, _| window.refresh());
    vcx.run_until_parked();
    let decl = a11y_take()
        .into_iter()
        .find(|d| d.role == Some(Role::TextInput))
        .expect("the field declared itself");
    assert_eq!(decl.value.as_deref(), Some("foo bar-"));
    assert_eq!(state(vcx, &input).0, "foo bar-");
}

#[gpui::test]
fn a_user_keymap_remaps_a_field_key(cx: &mut TestAppContext) {
    let (input, vcx) = field(cx, KeymapPlatform::Windows);
    let user = "[[binding]]\nchord = \"alt-up\"\ncommand = \"input.move_word_left\"\ncontext = \"TextInput\"\n";
    let (km, diags) = load(KeymapPlatform::Windows, "hacivat", Some(user));
    assert!(diags.is_empty(), "{diags:?}");
    let remapped = km
        .bindings_for("input.move_word_left", &KeyboardLayoutId("us".into()))
        .iter()
        .map(|b| chord_string(&b.chord))
        .collect::<Vec<_>>();
    assert_eq!(remapped, ["alt-up"]);
    vcx.update(|_, cx| set_input_keymap(cx, Arc::new(km), KeyboardLayoutId("us".into())));
    reset(vcx, &input, "foo bar", 7..7);
    press(vcx, "alt-up");
    assert_eq!(state(vcx, &input).1, 4..4);
    // The old chord no longer moves by word (it is not bound in the field at all now).
    press(
        vcx,
        &chords_of(KeymapPlatform::Windows, "input.move_word_left")[0],
    );
    assert_eq!(state(vcx, &input).1, 4..4);
}

/// Keys the field does not bind go on to its parents (↑/↓ on Windows and Linux, Tab).
struct Parent {
    input: Entity<TextInput>,
    seen: Rc<std::cell::RefCell<Vec<String>>>,
}

impl Render for Parent {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let seen = self.seen.clone();
        div()
            .w(px(400.0))
            .on_key_down(move |e: &gpui::KeyDownEvent, _, _| {
                seen.borrow_mut().push(e.keystroke.key.clone())
            })
            .child(self.input.clone())
    }
}

#[gpui::test]
fn unbound_keys_reach_the_parent_and_bound_ones_do_not(cx: &mut TestAppContext) {
    cx.update(|cx| {
        Theme::install(cx, ThemeMode::Dark);
        cx.set_global(InputKeymap::defaults(KeymapPlatform::Windows));
    });
    let seen: Rc<std::cell::RefCell<Vec<String>>> = Rc::default();
    let sink = seen.clone();
    let (host, vcx) = cx.add_window_view(move |_, cx| Parent {
        input: cx.new(|cx| TextInput::new(cx, placeholder())),
        seen: sink,
    });
    let input = host.read_with(vcx, |h, _| h.input.clone());
    vcx.update(|w, cx| {
        w.activate_window();
        input.focus_handle(cx).focus(w, cx);
    });
    vcx.run_until_parked();
    press(vcx, "up");
    press(vcx, "tab");
    press(vcx, "home");
    press(
        vcx,
        &chords_of(KeymapPlatform::Windows, "input.move_word_left")[0],
    );
    assert_eq!(*seen.borrow(), ["up", "tab"]);
}

// ---- mouse ---------------------------------------------------------------------------------------

/// The window point at the left edge of the value's UTF-16 offset `at`, mid-height.
fn x_at(vcx: &mut VisualTestContext, input: &Entity<TextInput>, at: usize) -> Point<Pixels> {
    vcx.run_until_parked();
    let b = input
        .update_in(vcx, |i, w, cx| {
            i.bounds_for_range(at..at, gpui::Bounds::default(), w, cx)
        })
        .expect("the line is laid out");
    point(b.origin.x, b.origin.y + b.size.height / 2.0)
}

fn mouse_down(vcx: &mut VisualTestContext, at: Point<Pixels>, clicks: usize, shift: bool) {
    let modifiers = if shift {
        Modifiers::shift()
    } else {
        Modifiers::none()
    };
    vcx.simulate_event(MouseDownEvent {
        button: MouseButton::Left,
        position: at,
        modifiers,
        click_count: clicks,
        first_mouse: false,
    });
    vcx.run_until_parked();
}

fn mouse_up(vcx: &mut VisualTestContext, at: Point<Pixels>, clicks: usize) {
    vcx.simulate_event(MouseUpEvent {
        button: MouseButton::Left,
        position: at,
        modifiers: Modifiers::none(),
        click_count: clicks,
    });
    vcx.run_until_parked();
}

fn click(vcx: &mut VisualTestContext, at: Point<Pixels>, clicks: usize, shift: bool) {
    mouse_down(vcx, at, clicks, shift);
    mouse_up(vcx, at, clicks);
}

#[gpui::test]
fn a_click_places_the_caret_and_shift_click_extends(cx: &mut TestAppContext) {
    let (input, vcx) = field(cx, KeymapPlatform::MacOs);
    reset(vcx, &input, "foo bar-baz", 11..11);
    let at3 = x_at(vcx, &input, 3);
    click(vcx, at3, 1, false);
    assert_eq!(state(vcx, &input).1, 3..3);
    let at9 = x_at(vcx, &input, 9);
    click(vcx, at9, 1, true);
    assert_eq!(state(vcx, &input).1, 3..9);
    let at1 = x_at(vcx, &input, 1);
    click(vcx, at1, 1, true);
    assert_eq!((state(vcx, &input).1, state(vcx, &input).2), (1..3, true));
}

#[gpui::test]
fn a_click_inside_a_cluster_lands_on_its_edge(cx: &mut TestAppContext) {
    let (input, vcx) = field(cx, KeymapPlatform::MacOs);
    let family = "👨\u{200d}👩\u{200d}👧";
    vcx.simulate_input(&format!("a{family}b"));
    // The family is 8 UTF-16 units from offset 1; aim between its first and last units.
    let left = x_at(vcx, &input, 1);
    let right = x_at(vcx, &input, 9);
    let inside = point(left.x + (right.x - left.x) * 0.4, left.y);
    click(vcx, inside, 1, false);
    let caret = state(vcx, &input).1;
    assert!(
        caret == (1..1) || caret == (1 + family.len()..1 + family.len()),
        "{caret:?}"
    );
}

#[gpui::test]
fn a_drag_selects_and_follows_past_the_field(cx: &mut TestAppContext) {
    let (input, vcx) = field(cx, KeymapPlatform::MacOs);
    reset(vcx, &input, "foo bar-baz", 0..0);
    let from = x_at(vcx, &input, 2);
    mouse_down(vcx, from, 1, false);
    let to = x_at(vcx, &input, 6);
    vcx.simulate_event(MouseMoveEvent {
        position: to,
        pressed_button: Some(MouseButton::Left),
        modifiers: Modifiers::none(),
    });
    vcx.run_until_parked();
    assert_eq!(state(vcx, &input).1, 2..6);
    // Leftwards past the field's start: the head follows to 0.
    vcx.simulate_event(MouseMoveEvent {
        position: point(px(-50.0), to.y),
        pressed_button: Some(MouseButton::Left),
        modifiers: Modifiers::none(),
    });
    vcx.run_until_parked();
    assert_eq!((state(vcx, &input).1, state(vcx, &input).2), (0..2, true));
    mouse_up(vcx, point(px(-50.0), to.y), 1);
    // Once released, moving changes nothing.
    vcx.simulate_event(MouseMoveEvent {
        position: to,
        pressed_button: None,
        modifiers: Modifiers::none(),
    });
    vcx.run_until_parked();
    assert_eq!(state(vcx, &input).1, 0..2);
}

#[gpui::test]
fn double_click_selects_a_word_and_triple_click_all(cx: &mut TestAppContext) {
    let (input, vcx) = field(cx, KeymapPlatform::Linux);
    let text = "select müşteri, ad";
    vcx.simulate_input(text);
    // UTF-16 offset 9 is inside "müşteri".
    let at = x_at(vcx, &input, 9);
    click(vcx, at, 1, false);
    click(vcx, at, 2, false);
    assert_eq!(state(vcx, &input).1, 7.."select müşteri".len());
    click(vcx, at, 3, false);
    assert_eq!(state(vcx, &input).1, 0..text.len());
    // A masked value is one word: a double click selects it all.
    input.update(vcx, |i, _| i.set_masked(true));
    reset(vcx, &input, "correct horse", 0..0);
    let at = x_at(vcx, &input, 3);
    click(vcx, at, 1, false);
    click(vcx, at, 2, false);
    assert_eq!(state(vcx, &input).1, 0.."correct horse".len());
}

// ---- inputnav-1 fix round 1 ---------------------------------------------------------------------

/// The window point `share` of the way across the grapheme at UTF-16 offsets `from..to`.
fn inside_glyph(
    vcx: &mut VisualTestContext,
    input: &Entity<TextInput>,
    from: usize,
    to: usize,
    share: f32,
) -> Point<Pixels> {
    let a = x_at(vcx, input, from);
    let b = x_at(vcx, input, to);
    point(a.x + (b.x - a.x) * share, a.y)
}

/// I3: a double click on the right half of a word's last letter selects that word, not the
/// space or comma after it.
#[gpui::test]
fn double_click_on_the_right_half_of_the_last_letter_selects_the_word(cx: &mut TestAppContext) {
    let (input, vcx) = field(cx, KeymapPlatform::MacOs);
    reset(vcx, &input, "foo bar", 7..7);
    let at = inside_glyph(vcx, &input, 2, 3, 0.75);
    click(vcx, at, 1, false);
    click(vcx, at, 2, false);
    assert_eq!(state(vcx, &input).1, 0..3);
    // `müşteri, ad`: the right half of `i` (UTF-16 6..7).
    input.update(vcx, |i, cx| {
        i.set_value("müşteri, ad");
        cx.notify();
    });
    let at = inside_glyph(vcx, &input, 6, 7, 0.75);
    click(vcx, at, 1, false);
    click(vcx, at, 2, false);
    assert_eq!(state(vcx, &input).1, 0.."müşteri".len());
    // R-IN-1 applies to the double click too: `schema.table` gives `table`.
    input.update(vcx, |i, cx| {
        i.set_value("schema.table");
        cx.notify();
    });
    let at = inside_glyph(vcx, &input, 8, 9, 0.5);
    click(vcx, at, 1, false);
    click(vcx, at, 2, false);
    assert_eq!(state(vcx, &input).1, 7..12);
}

/// m7: past the end, a double click selects the last word, not the spaces after it.
#[gpui::test]
fn double_click_past_the_end_selects_the_last_word(cx: &mut TestAppContext) {
    let (input, vcx) = field(cx, KeymapPlatform::Linux);
    reset(vcx, &input, "foo bar  ", 0..0);
    let end = x_at(vcx, &input, 9);
    let past = point(end.x + px(40.0), end.y);
    click(vcx, past, 1, false);
    click(vcx, past, 2, false);
    assert_eq!(state(vcx, &input).1, 4..7);
}

/// m7: a drag after a double click extends by whole words, keeping the first word.
#[gpui::test]
fn a_drag_after_a_double_click_extends_by_words(cx: &mut TestAppContext) {
    let (input, vcx) = field(cx, KeymapPlatform::MacOs);
    reset(vcx, &input, "foo bar baz", 0..0);
    let in_bar = inside_glyph(vcx, &input, 5, 6, 0.5);
    click(vcx, in_bar, 1, false);
    mouse_down(vcx, in_bar, 2, false);
    assert_eq!(state(vcx, &input).1, 4..7);
    fn drag(vcx: &mut VisualTestContext, to: Point<Pixels>) {
        vcx.simulate_event(MouseMoveEvent {
            position: to,
            pressed_button: Some(MouseButton::Left),
            modifiers: Modifiers::none(),
        });
        vcx.run_until_parked();
    }
    let to = inside_glyph(vcx, &input, 9, 10, 0.3);
    drag(vcx, to);
    assert_eq!((state(vcx, &input).1, state(vcx, &input).2), (4..11, false));
    let to = inside_glyph(vcx, &input, 1, 2, 0.5);
    drag(vcx, to);
    assert_eq!((state(vcx, &input).1, state(vcx, &input).2), (0..7, true));
    mouse_up(vcx, in_bar, 2);
}

/// I2: Ctrl+→ and Ctrl+Delete from `foo bar` at 0 land on `bar` (4) on Windows, as native
/// fields do; Linux (GTK) and macOS stop at the end of `foo` (3).
#[gpui::test]
fn the_windows_word_move_lands_on_the_next_word(cx: &mut TestAppContext) {
    for (platform, right, delete, caret, left) in [
        (
            KeymapPlatform::Windows,
            "input.move_next_word_start",
            "input.delete_to_next_word_start",
            4,
            "bar",
        ),
        (
            KeymapPlatform::Linux,
            "input.move_word_right",
            "input.delete_word_forward",
            3,
            " bar",
        ),
        (
            KeymapPlatform::MacOs,
            "input.move_word_right",
            "input.delete_word_forward",
            3,
            " bar",
        ),
    ] {
        let (input, vcx) = field(cx, platform);
        reset(vcx, &input, "foo bar", 0..0);
        press(vcx, &chords_of(platform, right)[0]);
        assert_eq!(state(vcx, &input).1, caret..caret, "{platform:?}");
        reset(vcx, &input, "foo bar", 0..0);
        press(vcx, &chords_of(platform, delete)[0]);
        assert_eq!(state(vcx, &input).0, left, "{platform:?}");
    }
}

/// m4: undo and redo in the field, per platform keymap: a typing run is one step, a caret move
/// ends it, and every undo and redo chord works.
#[gpui::test]
fn undo_and_redo_follow_typing_runs(cx: &mut TestAppContext) {
    for platform in PLATFORMS {
        let (input, vcx) = field(cx, platform);
        let key = |c: &str| chords_of(platform, c);
        vcx.simulate_input("abc");
        press(vcx, &key("input.move_left")[0]);
        vcx.simulate_input("ş");
        assert_eq!(state(vcx, &input).0, "abşc");
        press(vcx, &key("input.undo")[0]);
        assert_eq!(
            state(vcx, &input),
            ("abc".into(), 2..2, false),
            "{platform:?}"
        );
        press(vcx, &key("input.undo")[0]);
        assert_eq!(state(vcx, &input).0, "", "{platform:?}");
        for chord in key("input.redo") {
            press(vcx, &chord);
        }
        // Two redo chords (Ctrl+Y and Ctrl+⇧Z) or one (⇧⌘Z): every step that was undone
        // comes back once there are enough presses.
        let want = if key("input.redo").len() >= 2 {
            "abşc"
        } else {
            "abc"
        };
        assert_eq!(state(vcx, &input).0, want, "{platform:?}");
        // A delete run is one step too.
        press(vcx, &key("input.move_line_end")[0]);
        press(vcx, &key("input.delete_backward")[0]);
        press(vcx, &key("input.delete_backward")[0]);
        press(vcx, &key("input.undo")[0]);
        assert_eq!(state(vcx, &input).0, want, "{platform:?}");
        // A value the app sets starts a new history.
        input.update(vcx, |i, cx| {
            i.set_value("yeni");
            cx.notify();
        });
        assert_eq!(input.read_with(vcx, |i, _| i.history_depth()), (0, 0));
    }
}

/// m4: an IME composition is one undo step, recorded when it commits.
#[gpui::test]
fn a_committed_composition_is_one_undo_step(cx: &mut TestAppContext) {
    let (input, vcx) = field(cx, KeymapPlatform::MacOs);
    vcx.simulate_input("ab ");
    input.update_in(vcx, |i, w, cx| {
        i.replace_and_mark_text_in_range(None, "k", None, w, cx);
        i.replace_and_mark_text_in_range(None, "か", None, w, cx);
        i.replace_and_mark_text_in_range(None, "かん", None, w, cx);
        i.replace_text_in_range(None, "漢", w, cx);
    });
    assert_eq!(state(vcx, &input).0, "ab 漢");
    press(vcx, &chords_of(KeymapPlatform::MacOs, "input.undo")[0]);
    assert_eq!(state(vcx, &input).0, "ab ");
}

/// m4: the Windows and Linux clipboard keys of old (⇧Delete, Ctrl+Insert, ⇧Insert).
#[gpui::test]
fn the_old_clipboard_keys_work_on_windows_and_linux(cx: &mut TestAppContext) {
    for platform in [KeymapPlatform::Windows, KeymapPlatform::Linux] {
        let (input, vcx) = field(cx, platform);
        let old = |command: &str| {
            chords_of(platform, command)
                .into_iter()
                .find(|c| c.contains("insert") || c.contains("delete"))
                .unwrap_or_else(|| panic!("{platform:?} {command}"))
        };
        reset(vcx, &input, "şehir", 0..0);
        press(vcx, &chords_of(platform, "input.select_all")[0]);
        press(vcx, &old("input.copy"));
        assert_eq!(clipboard(vcx).as_deref(), Some("şehir"));
        press(vcx, &old("input.cut"));
        assert_eq!(state(vcx, &input).0, "");
        let paste = old("input.paste");
        press(vcx, &paste);
        press(vcx, &paste);
        assert_eq!(state(vcx, &input).0, "şehirşehir", "{platform:?}");
    }
}

// ---- the focus ring (PO demo finding F2) -------------------------------------------------------

/// A full-width field in a scroll body inside a padded card (the Attach dialog's Database
/// field): with `room`, the body is given room for the ring (`ring_room`).
#[cfg(feature = "scene-check")]
struct ScrollBody {
    input: Entity<TextInput>,
    room: bool,
}

#[cfg(feature = "scene-check")]
impl Render for ScrollBody {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        use gpui::StatefulInteractiveElement as _;
        let body = div()
            .id("body")
            .flex()
            .flex_col()
            .overflow_y_scroll()
            .child(div().h(px(12.0)))
            .child(self.input.clone())
            .child(div().h(px(12.0)));
        let body = if self.room {
            hacivat_ui_core::theme::ring_room(body)
        } else {
            body
        };
        div().w(px(400.0)).p(px(16.0)).flex().flex_col().child(body)
    }
}

/// The focused field's ring: whole on all four sides (each edge 1 px of blue at a 1 px gap),
/// only when the scroll body has room for it.
#[cfg(feature = "scene-check")]
#[gpui::test]
fn a_full_width_field_in_a_scroll_body_draws_its_whole_ring(cx: &mut TestAppContext) {
    use hacivat_ui_core::named_rules::clipped_focus_rings;
    use hacivat_ui_core::theme::{DARK, surfaces::FOCUS_RING_EXTENT_PX};
    cx.update(|cx| Theme::install(cx, ThemeMode::Dark));
    for room in [false, true] {
        let (host, vcx) = cx.add_window_view(|_, cx| ScrollBody {
            input: cx.new(|cx| TextInput::new(cx, placeholder())),
            room,
        });
        let input = host.read_with(vcx, |h, _| h.input.clone());
        vcx.update(|w, cx| {
            w.activate_window();
            input.focus_handle(cx).focus(w, cx);
        });
        vcx.run_until_parked();
        vcx.update(|w, _| w.refresh());
        vcx.run_until_parked();
        let (clipped, rings) = vcx.update(|w, _| {
            let blue = DARK.colors.blue.to_hsla();
            let s = w.scale_factor();
            let rings: Vec<(f32, f32)> = w
                .rendered_scene()
                .shadows
                .iter()
                .filter(|sh| sh.inset == 0 && sh.blur_radius.0 == 0.0 && sh.color == blue)
                .map(|sh| (sh.bounds.origin.x.0 / s, sh.bounds.size.width.0 / s))
                .collect();
            (clipped_focus_rings(w, blue), rings)
        });
        // One ring around the field, which keeps its place and width either way (16 px card
        // padding, 368 px wide): the ring starts the extent before it and is that much wider
        // on each side.
        let e = FOCUS_RING_EXTENT_PX;
        assert_eq!(rings, [(16.0 - e, 368.0 + 2.0 * e)], "room: {room}");
        if room {
            assert_eq!(clipped, Vec::new());
        } else {
            // F2: the body's clip cuts the ring's left and right sides, not its top or bottom.
            assert_eq!(clipped.len(), 1, "{clipped:?}");
            let (ring, clip) = clipped[0];
            assert!(ring.left() < clip.left() && ring.right() > clip.right());
            assert!(ring.top() >= clip.top() && ring.bottom() <= clip.bottom());
        }
    }
}
