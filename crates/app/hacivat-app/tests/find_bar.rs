//! M3c Task 33: the editor find bar (UI-EDIT-22 without replace; R-M3-10; D-074; design/m3b-safe
//! surface 16). The matcher is built in the app over the original text (Turkish-tolerant classes
//! `[iIİı]` and accents; exact mode never folds a Turkish letter; whole words by SQL word
//! boundaries); the bar opens on ⌘F / Ctrl+F seeded from the selection, finds as you type from
//! where find started, steps with Enter / Shift+Enter, searches nothing for an invalid pattern,
//! cycles its controls with Tab, remembers Turkish-tolerant, and Escape returns the caret.
//!
//! Consoles are Task 1b's `register_test_console` (a real editor over a FakeDriver session).
//! Every wait is on a condition (`run_until_parked`, `wait_until`).

mod support;

use gpui::{Entity, Focusable as _, TestAppContext, VisualTestContext};
use hacivat_app::console::find_bar::{self, FindOpts, RegexMatcher, build_matcher};
use hacivat_app::console::{ConsoleId, Consoles};
use hacivat_settings::FindTolerancePref;
use hacivat_ui_editor::{EditorView, FindMatcher};
use support::*;

// ---- the matcher ----------------------------------------------------------------------------

fn matcher(q: &str, o: FindOpts) -> RegexMatcher {
    build_matcher(q, o)
        .expect("the pattern compiles")
        .expect("a query")
}

fn tolerant() -> FindOpts {
    FindOpts {
        tolerant: true,
        ..FindOpts::default()
    }
}

fn exact() -> FindOpts {
    FindOpts::default()
}

fn words() -> FindOpts {
    FindOpts {
        words: true,
        ..FindOpts::default()
    }
}

/// Every match of `m` in `hay`, as text.
fn all<'a>(m: &RegexMatcher, hay: &'a str) -> Vec<&'a str> {
    let mut out = Vec::new();
    let mut at = 0;
    while let Some(r) = m.find(hay, at) {
        at = r.end;
        out.push(&hay[r]);
    }
    out
}

#[test]
fn turkish_tolerant_matches_dotted_and_dotless_i() {
    let m = matcher("istanbul", tolerant());
    assert!(m.find("İSTANBUL", 0).is_some());
    assert!(m.find("ıstanbul", 0).is_some());
    assert!(m.find("Istanbul", 0).is_some());
    // A decomposed dotted capital (I + U+0307) is found whole.
    assert_eq!(all(&m, "I\u{307}stanbul'da"), ["I\u{307}stanbul"]);
    // The query's own dotted capital finds every i too.
    let m = matcher("İzmir", tolerant());
    assert_eq!(
        all(&m, "izmir IZMIR ızmır İzmir"),
        ["izmir", "IZMIR", "ızmır", "İzmir"]
    );
    // ALG UI-EDIT-22 test 1: four matches, three as whole words.
    assert_eq!(all(&matcher("id", tolerant()), "ID, İD, ıd, idx").len(), 4);
    let tw = FindOpts {
        words: true,
        ..tolerant()
    };
    assert_eq!(
        all(&matcher("id", tw), "ID, İD, ıd, idx"),
        ["ID", "İD", "ıd"]
    );
}

#[test]
fn turkish_tolerant_ignores_accents_and_match_case_keeps_the_case() {
    let m = matcher("musteri", tolerant());
    assert_eq!(
        all(&m, "müşteri MÜŞTERİ musteri"),
        ["müşteri", "MÜŞTERİ", "musteri"]
    );
    assert_eq!(
        all(&matcher("şehir", tolerant()), "sehir şehir"),
        ["sehir", "şehir"]
    );
    // Decomposed accents in the text are taken with their letter.
    assert_eq!(all(&m, "mu\u{308}s\u{327}teri"), ["mu\u{308}s\u{327}teri"]);
    // Match case: a lower i finds the dotless ı but no capital; a capital I finds İ.
    let mc = FindOpts {
        match_case: true,
        ..tolerant()
    };
    assert_eq!(all(&matcher("i", mc), "i ı I İ"), ["i", "ı"]);
    assert_eq!(all(&matcher("I", mc), "i ı I İ"), ["I", "İ"]);
}

#[test]
fn exact_mode_does_not_fold_turkish_letters() {
    let m = matcher("istanbul", exact());
    assert!(m.find("İSTANBUL", 0).is_none());
    assert!(m.find("ıstanbul", 0).is_none());
    // Plain case folding still applies (Match case is off).
    assert!(m.find("ISTANBUL", 0).is_some());
    assert!(matcher("musteri", exact()).find("müşteri", 0).is_none());
    assert_eq!(all(&matcher("i", exact()), "i ı I İ"), ["i", "I"]);
    let mc = FindOpts {
        match_case: true,
        ..exact()
    };
    assert!(matcher("istanbul", mc).find("Istanbul", 0).is_none());
}

#[test]
fn words_mode_uses_sql_word_boundaries() {
    let m = matcher("id", words());
    let hay = "id, user_id, id2, $id, (id), id$x, idx, x.id";
    assert_eq!(all(&m, hay), ["id", "id", "id"]);
    let found: Vec<usize> = {
        let mut out = Vec::new();
        let mut at = 0;
        while let Some(r) = m.find(hay, at) {
            at = r.end;
            out.push(r.start);
        }
        out
    };
    // `id`, `(id)` and `x.id`: `_`, digits, `$` and letters are word characters; `(`, `.` and
    // `,` are not.
    assert_eq!(found, [0, 24, 42]);
    // Words apply to a regular expression too.
    let rw = FindOpts {
        regex: true,
        words: true,
        ..FindOpts::default()
    };
    assert_eq!(all(&matcher(r"i\w", rw), "id idx id2"), ["id"]);
}

#[test]
fn a_tolerant_regex_rewrites_its_literal_letters_only() {
    let rt = FindOpts {
        regex: true,
        ..tolerant()
    };
    assert_eq!(
        all(&matcher(r"\bist\w+", rt), "İstanbul ıstakoz"),
        ["İstanbul", "ıstakoz"]
    );
    // A quantifier applies to the whole tolerant letter.
    assert_eq!(all(&matcher("i+", rt), "ıİi x"), ["ıİi"]);
    // Escapes, ranges, named groups and flags are left as written.
    assert_eq!(all(&matcher(r"\x69", rt), "ı i"), ["i"]);
    assert_eq!(all(&matcher("[a-c]+", rt), "abç"), ["ab"]);
    assert_eq!(all(&matcher("[iz]+", rt), "ızİ"), ["ızİ"]);
    assert_eq!(all(&matcher("(?P<isim>i)x", rt), "ıx"), ["ıx"]);
    assert_eq!(all(&matcher("(?i:i)", rt), "İ"), ["İ"]);
    assert_eq!(all(&matcher(r"\pL+", rt), "12ab"), ["ab"]);
    // Off: the regex is the user's.
    let r = FindOpts {
        regex: true,
        ..FindOpts::default()
    };
    assert!(matcher(r"\bist\w+", r).find("İstanbul", 0).is_none());
}

#[test]
fn an_invalid_regex_is_an_error_and_an_empty_query_searches_nothing() {
    let r = FindOpts {
        regex: true,
        ..FindOpts::default()
    };
    let e = build_matcher("(a", r).expect_err("an unclosed group");
    assert!(!e.is_empty() && !e.contains('\n'), "{e}");
    // Look-around is not supported by the engine: an error, never a panic.
    assert!(build_matcher("(?<=a)b", r).is_err());
    // The same text is a fine literal.
    assert!(matcher("(a", exact()).find("f(a)", 0).is_some());
    assert!(matches!(build_matcher("", r), Ok(None)));
}

#[test]
fn empty_matches_are_passed_over_and_lines_are_crlf_aware() {
    let r = FindOpts {
        regex: true,
        ..FindOpts::default()
    };
    assert!(matcher("^", r).find("a\nb", 0).is_none());
    assert_eq!(all(&matcher("a*", r), "baa"), ["aa"]);
    assert_eq!(all(&matcher("b$", r), "ab\r\nb"), ["b", "b"]);
    assert_eq!(all(&matcher("^b", r), "a\r\nb"), ["b"]);
}

// ---- the bar ---------------------------------------------------------------------------------

const TEXT: &str = "SELECT a FROM t;\nSELECT b FROM t WHERE a = 1;\n";

/// A shell with one console over `text`, its editor focused with the caret at `caret`.
fn console_with(
    cx: &mut TestAppContext,
    lang: &str,
    text: &str,
    caret: usize,
) -> (
    &'static mut VisualTestContext,
    ConsoleId,
    Entity<EditorView>,
) {
    let vcx = shell_with_connections(cx, lang);
    in_window(vcx, |window, _| window.activate_window());
    let p = test_profile("orders-dev", None, "10.0.0.5");
    let profile = add_profile(vcx, p.clone());
    let (session, _fake) = test_session(vcx, p);
    let id = register_test_console(vcx, profile, session);
    let editor = vcx.cx.read(|cx| {
        Consoles::get(cx, id)
            .and_then(|s| s.editor.clone())
            .expect("the console has an editor")
    });
    in_window(vcx, |window, cx| {
        editor.update(cx, |v, cx| {
            v.set_text(text, cx);
            v.select_range(caret..caret, cx);
        });
        let handle = editor.read(cx).focus_handle(cx);
        window.focus(&handle, cx);
    });
    vcx.run_until_parked();
    (vcx, id, editor)
}

fn main_range(vcx: &VisualTestContext, editor: &Entity<EditorView>) -> std::ops::Range<usize> {
    vcx.cx
        .read(|cx| editor.read(cx).snapshot().selection.main().range())
}

fn primary_f() -> &'static str {
    if cfg!(target_os = "macos") {
        "cmd-f"
    } else {
        "ctrl-f"
    }
}

fn is_open(vcx: &VisualTestContext, id: ConsoleId) -> bool {
    vcx.cx.read(|cx| find_bar::is_open(id, cx))
}

fn editor_has_focus(vcx: &VisualTestContext, editor: &Entity<EditorView>) -> bool {
    in_window(vcx, |window, cx| {
        editor.read(cx).focus_handle(cx).is_focused(window)
    })
}

#[cfg(unix)]
#[gpui::test]
fn primary_f_opens_the_bar_seeded_from_the_selection(cx: &mut TestAppContext) {
    let (vcx, id, editor) = console_with(cx, "en", TEXT, 0);
    assert!(!is_open(vcx, id));
    in_window(vcx, |_, cx| {
        editor.update(cx, |v, cx| v.select_range(14..15, cx))
    });
    vcx.simulate_keystrokes(primary_f());
    vcx.run_until_parked();
    assert!(is_open(vcx, id), "{} opened the bar", primary_f());
    assert_eq!(
        vcx.cx.read(|cx| find_bar::query(id, cx)).as_deref(),
        Some("t")
    );
    // The selection is the current match; both `t`s of the text count ("from t" twice).
    assert_eq!(main_range(vcx, &editor), 14..15);
    let s = vcx.cx.read(|cx| editor.read(cx).find_status());
    assert!(s.active && s.total.is_some_and(|n| n >= 2), "{s:?}");
    // The explorer filter did not take the chord.
    let order = vcx.cx.read(|cx| find_bar::focus_order(id, cx));
    assert_eq!(
        focused(vcx),
        order.first().cloned(),
        "the field has the focus"
    );
}

#[cfg(unix)]
#[gpui::test]
fn escape_returns_the_caret(cx: &mut TestAppContext) {
    let (vcx, id, editor) = console_with(cx, "en", TEXT, 3);
    vcx.simulate_keystrokes(primary_f());
    vcx.run_until_parked();
    assert!(is_open(vcx, id));
    // Typing finds from where find started: the first `a` after the caret.
    vcx.simulate_input("a");
    vcx.run_until_parked();
    assert_eq!(main_range(vcx, &editor), 7..8);
    vcx.simulate_keystrokes("enter");
    vcx.run_until_parked();
    let second = TEXT[8..].find(" a ").unwrap() + 9;
    assert_eq!(main_range(vcx, &editor), second..second + 1);
    vcx.simulate_keystrokes("shift-enter");
    vcx.run_until_parked();
    assert_eq!(main_range(vcx, &editor), 7..8);
    vcx.simulate_keystrokes("escape");
    vcx.run_until_parked();
    assert!(!is_open(vcx, id));
    assert_eq!(
        main_range(vcx, &editor),
        3..3,
        "the caret is back where find started"
    );
    assert!(editor_has_focus(vcx, &editor));
    assert!(!vcx.cx.read(|cx| editor.read(cx).find_status().active));
    // Opened again, the last query is there (selected) and steps from the caret.
    vcx.simulate_keystrokes(primary_f());
    vcx.run_until_parked();
    assert_eq!(
        vcx.cx.read(|cx| find_bar::query(id, cx)).as_deref(),
        Some("a")
    );
    // A caret the user moves stays where it is on Escape.
    in_window(vcx, |window, cx| {
        editor.update(cx, |v, cx| v.select_range(20..20, cx));
        let handle = vcx_order_first(id, cx);
        window.focus(&handle, cx);
    });
    vcx.run_until_parked();
    vcx.simulate_keystrokes("escape");
    vcx.run_until_parked();
    assert!(!is_open(vcx, id));
    assert_eq!(main_range(vcx, &editor), 20..20);
}

fn vcx_order_first(id: ConsoleId, cx: &gpui::App) -> gpui::FocusHandle {
    find_bar::focus_order(id, cx)
        .first()
        .cloned()
        .expect("a field")
}

#[cfg(unix)]
#[gpui::test]
fn invalid_regex_searches_nothing(cx: &mut TestAppContext) {
    let (vcx, id, editor) = console_with(cx, "en", TEXT, 3);
    vcx.simulate_keystrokes(primary_f());
    vcx.run_until_parked();
    in_window(vcx, |window, cx| find_bar::flip_toggle(id, 2, window, cx));
    assert!(vcx.cx.read(|cx| find_bar::options(id, cx)).unwrap().regex);
    vcx.simulate_input("(a");
    vcx.run_until_parked();
    let error = vcx.cx.read(|cx| find_bar::error(id, cx));
    assert!(error.is_some(), "the field says the pattern is invalid");
    let s = vcx.cx.read(|cx| editor.read(cx).find_status());
    assert_eq!((s.active, s.total), (true, Some(0)), "nothing is searched");
    assert_eq!(main_range(vcx, &editor), 3..3, "the caret did not move");
    // Stepping does nothing either.
    vcx.simulate_keystrokes("enter");
    vcx.run_until_parked();
    assert_eq!(main_range(vcx, &editor), 3..3);
    // The drawn count is the ink "Invalid pattern" (no red; NOTES M3c-11).
    assert_eq!(
        text_of(vcx, "find-count-text").as_deref(),
        Some("Invalid pattern")
    );
    // Closing the group makes it valid again.
    vcx.simulate_input(")");
    vcx.run_until_parked();
    assert!(vcx.cx.read(|cx| find_bar::error(id, cx)).is_none());
    assert_eq!(main_range(vcx, &editor), 7..8);
}

#[cfg(unix)]
#[gpui::test]
fn keyboard_loop_through_the_bar(cx: &mut TestAppContext) {
    let (vcx, id, editor) = console_with(cx, "en", TEXT, 0);
    vcx.simulate_keystrokes(primary_f());
    vcx.run_until_parked();
    vcx.simulate_input("FROM");
    vcx.run_until_parked();
    let order = vcx.cx.read(|cx| find_bar::focus_order(id, cx));
    assert_eq!(order.len(), 9);
    assert_eq!(focused(vcx), Some(order[0].clone()));
    // Tab walks field → four toggles → previous → next → Select all → close.
    for (i, want) in order.iter().enumerate().skip(1) {
        vcx.simulate_keystrokes("tab");
        vcx.run_until_parked();
        assert_eq!(focused(vcx), Some(want.clone()), "stop {i}");
    }
    // Back to Whole words with Shift+Tab, and Space presses it.
    for _ in 0..6 {
        vcx.simulate_keystrokes("shift-tab");
    }
    vcx.run_until_parked();
    assert_eq!(focused(vcx), Some(order[2].clone()));
    // A press is a key down and its key up (GPUI's keyboard click).
    vcx.simulate_keystrokes("space");
    vcx.simulate_event(gpui::KeyUpEvent {
        keystroke: gpui::Keystroke::parse("space").unwrap(),
    });
    vcx.run_until_parked();
    assert!(vcx.cx.read(|cx| find_bar::options(id, cx)).unwrap().words);
    // The editor's find chords work in the field: ⌘G / F3 steps.
    in_window(vcx, |window, cx| window.focus(&order[0], cx));
    vcx.run_until_parked();
    let before = main_range(vcx, &editor);
    let next = if cfg!(target_os = "macos") {
        "cmd-g"
    } else {
        "f3"
    };
    vcx.simulate_keystrokes(next);
    vcx.run_until_parked();
    assert_ne!(
        main_range(vcx, &editor),
        before,
        "{next} stepped from the field"
    );
    assert_eq!(
        focused(vcx),
        Some(order[0].clone()),
        "the field keeps the focus"
    );
    // Escape from any control closes the bar and gives the editor the focus.
    in_window(vcx, |window, cx| window.focus(&order[8], cx));
    vcx.simulate_keystrokes("escape");
    vcx.run_until_parked();
    assert!(!is_open(vcx, id));
    assert!(editor_has_focus(vcx, &editor));
}

#[cfg(unix)]
#[gpui::test]
fn select_all_occurrences_from_the_bar(cx: &mut TestAppContext) {
    let (vcx, id, editor) = console_with(cx, "en", TEXT, 0);
    vcx.simulate_keystrokes(primary_f());
    vcx.run_until_parked();
    vcx.simulate_input("from");
    vcx.run_until_parked();
    let chord = if cfg!(target_os = "macos") {
        "ctrl-cmd-g"
    } else {
        "ctrl-shift-l"
    };
    vcx.simulate_keystrokes(chord);
    vcx.run_until_parked();
    let n = vcx
        .cx
        .read(|cx| editor.read(cx).snapshot().selection.ranges().len());
    assert_eq!(n, 2, "both FROMs are selected");
    assert!(editor_has_focus(vcx, &editor), "the carets take the keys");
    assert!(is_open(vcx, id), "the bar stays open");
}

#[gpui::test]
fn turkish_tolerant_is_on_in_turkish_and_remembered(cx: &mut TestAppContext) {
    let (vcx, id, editor) = console_with(cx, "tr", TEXT, 0);
    in_window(vcx, |window, cx| {
        editor.update(cx, |v, cx| v.run_command("editor.find", window, cx));
    });
    vcx.run_until_parked();
    assert!(is_open(vcx, id));
    assert!(
        vcx.cx
            .read(|cx| find_bar::options(id, cx))
            .unwrap()
            .tolerant
    );
    in_window(vcx, |window, cx| find_bar::flip_toggle(id, 3, window, cx));
    let pref = vcx.cx.read(|cx| {
        cx.global::<hacivat_app::AppState>()
            .settings
            .editor
            .find_turkish_tolerant
    });
    assert_eq!(pref, FindTolerancePref::Off);
    // Closed and opened again, it stays off.
    in_window(vcx, |window, cx| {
        editor.update(cx, |v, cx| v.run_command("editor.find_close", window, cx));
    });
    vcx.run_until_parked();
    assert!(!is_open(vcx, id), "editor.find_close closes the bar");
    in_window(vcx, |window, cx| {
        editor.update(cx, |v, cx| v.run_command("editor.find", window, cx));
    });
    vcx.run_until_parked();
    assert!(
        !vcx.cx
            .read(|cx| find_bar::options(id, cx))
            .unwrap()
            .tolerant
    );
}

#[gpui::test]
fn turkish_tolerant_is_off_in_english(cx: &mut TestAppContext) {
    let (vcx, id, editor) = console_with(cx, "en", TEXT, 0);
    in_window(vcx, |window, cx| {
        editor.update(cx, |v, cx| v.run_command("editor.find", window, cx));
    });
    vcx.run_until_parked();
    assert!(
        !vcx.cx
            .read(|cx| find_bar::options(id, cx))
            .unwrap()
            .tolerant
    );
}

#[cfg(unix)]
#[gpui::test]
fn f6_enters_the_open_bar_first(cx: &mut TestAppContext) {
    let (vcx, id, editor) = console_with(cx, "en", TEXT, 0);
    vcx.simulate_keystrokes(primary_f());
    vcx.run_until_parked();
    in_window(vcx, |window, cx| {
        let handle = editor.read(cx).focus_handle(cx);
        window.focus(&handle, cx);
    });
    vcx.run_until_parked();
    assert!(run(vcx, "workspace.focus_notification"));
    vcx.run_until_parked();
    let order = vcx.cx.read(|cx| find_bar::focus_order(id, cx));
    assert_eq!(
        focused(vcx),
        Some(order[0].clone()),
        "F6 entered the find field"
    );
    // From the bar (no error bar, no toast) it goes back to the editor.
    assert!(run(vcx, "workspace.focus_notification"));
    vcx.run_until_parked();
    assert!(editor_has_focus(vcx, &editor));
}

/// ALG UI-EDIT-22 test 3 (the matcher): every `INSERT` of a 20 MB script is found within 200 ms
/// (release build, `HACIVAT_PERF=1`), exact and Turkish-tolerant, as a whole word too.
#[test]
#[ignore = "perf"]
fn matching_20_mb_under_200ms() {
    if std::env::var_os("HACIVAT_PERF").is_none() {
        return;
    }
    let line = "INSERT INTO müşteri (id, şehir) VALUES (1, 'İzmir');\n";
    let text = line.repeat(20 * (1 << 20) / line.len());
    let n = text.len() / line.len();
    for (name, o) in [
        ("exact", exact()),
        ("tolerant", tolerant()),
        (
            "tolerant words",
            FindOpts {
                words: true,
                ..tolerant()
            },
        ),
    ] {
        let at = std::time::Instant::now();
        let m = matcher("insert", o);
        let mut count = 0;
        let mut from = 0;
        while let Some(r) = m.find(&text, from) {
            from = r.end;
            count += 1;
        }
        let took = at.elapsed();
        println!("find perf: {name} over 20 MB {took:?}");
        assert_eq!(count, n, "{name}");
        assert!(took.as_millis() < 200, "{name}: {took:?}");
    }
}
