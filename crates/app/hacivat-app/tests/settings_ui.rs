//! The settings tab (Task 17): language, regional format, theme, text rendering (restart) and
//! keymap preset, saved to `settings.toml` and applied live where the setting allows.

mod support;

use std::path::PathBuf;

use gpui::{Modifiers, TestAppContext, VisualTestContext};
use hacivat_app::settings_ui::rows::{self, Section};
use hacivat_app::settings_ui::{SearchRow, filter_rows, settings_rows};
use hacivat_app::shell::{
    ActiveKeymap, flush_pending_writes, install_keymap_with, update_settings,
};
use hacivat_l10n::{
    Catalogs, FormatInputs, L10n, MatchMode, NegotiationCtx, PrefSource, SearchOpts, build_runtime,
    negotiate,
};
use hacivat_settings::{KeymapPreset, LocalPaths};
use hacivat_ui_core::app::TextRendering;
use hacivat_ui_core::keymap::{KeyboardLayoutId, KeymapPlatform};
#[cfg(target_os = "macos")]
use hacivat_ui_core::platform::app_font_smoothing_override;
use hacivat_ui_core::platform::apply_text_rendering;
use hacivat_ui_core::theme::{Theme, ThemeMode};
use support::*;

/// A shell over a fresh root with the settings tab open, the process text rendering at the
/// shipped default (F21).
fn settings_tab(cx: &mut TestAppContext) -> (&'static mut VisualTestContext, LocalPaths) {
    settings_tab_with(cx, support_platform(), "us", "UTC")
}

fn support_platform() -> KeymapPlatform {
    hacivat_app::shell::keymap_state::current_platform()
}

/// Like [`settings_tab`] on `platform` and keyboard `layout`, with `ui.time_zone = time_zone`.
fn settings_tab_with(
    cx: &mut TestAppContext,
    platform: KeymapPlatform,
    layout: &str,
    time_zone: &str,
) -> (&'static mut VisualTestContext, LocalPaths) {
    apply_text_rendering(TextRendering::Crisp);
    let root = tempfile::tempdir().expect("temp dir");
    let paths = LocalPaths::under(root.path());
    write(
        paths.settings_toml(),
        &format!("version = 1\n\n[ui]\ntime_zone = \"{time_zone}\"\n"),
    );
    // The temp root lives as long as the test process; the shell writes `workspace.json` there.
    let root: &'static tempfile::TempDir = Box::leak(Box::new(root));
    let vcx = shell_at(cx, root.path(), "en");
    let layout = KeyboardLayoutId(layout.to_string());
    vcx.cx
        .update(|cx| install_keymap_with(cx, platform, layout, None));
    with_ws(vcx, |w, window, cx| {
        w.open_registered_tab("settings", window, cx)
    });
    vcx.run_until_parked();
    (vcx, paths)
}

fn read(path: PathBuf) -> String {
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// A selector string that lives for the process (GPUI's `debug_bounds` wants `&'static str`).
fn selector(s: String) -> &'static str {
    Box::leak(s.into_boxed_str())
}

fn center(vcx: &mut VisualTestContext, name: String) -> gpui::Point<gpui::Pixels> {
    let b = vcx
        .debug_bounds(selector(name.clone()))
        .unwrap_or_else(|| panic!("`{name}` is not on screen"));
    b.center()
}

fn click(vcx: &mut VisualTestContext, name: String) {
    let at = center(vcx, name);
    vcx.simulate_click(at, Modifiers::none());
    vcx.run_until_parked();
}

fn row_visible(vcx: &mut VisualTestContext, key: &str) -> bool {
    vcx.debug_bounds(selector(format!("settings-row-{key}")))
        .is_some()
}

/// Shows the section of row `key` (clicking its navigation item), unless it is on screen.
fn reveal(vcx: &mut VisualTestContext, key: &str) {
    if row_visible(vcx, key) {
        return;
    }
    let section = rows::row_section(key).expect("a known row");
    let index = Section::ALL.iter().position(|s| *s == section).unwrap();
    click(vcx, format!("settings-nav-{index}"));
}

/// Clicks the segment of row `key` whose stored value is `value`.
fn click_segment(vcx: &mut VisualTestContext, key: &str, value: &str) {
    reveal(vcx, key);
    let index = vcx
        .cx
        .read(|cx| rows::choices(cx, key))
        .iter()
        .position(|c| c.value == value)
        .unwrap_or_else(|| panic!("`{key}` has no choice `{value}`"));
    click(vcx, format!("segmented-{key}-{index}"));
    vcx.run_until_parked();
}

fn choose(vcx: &mut VisualTestContext, key: &str, value: &str) {
    click_segment(vcx, key, value);
}

fn label_with_id(vcx: &VisualTestContext, id: &str) -> Option<String> {
    let needle = format!("#{id}");
    registry_frame(vcx)
        .into_iter()
        .find(|r| r.path.ends_with(&needle))
        .map(|r| r.text.to_string())
}

fn nav_label(vcx: &VisualTestContext, index: usize) -> String {
    label_with_id(vcx, &format!("settings-nav-label-{index}")).expect("a navigation label")
}

fn regional_preview(vcx: &VisualTestContext) -> String {
    label_with_id(vcx, "settings-regional-preview").expect("the regional preview is on screen")
}

/// Only the macOS text-rendering tests use it (the row is macOS only).
#[cfg(target_os = "macos")]
fn restart_note_visible(vcx: &VisualTestContext) -> bool {
    label_with_id(vcx, "settings-restart-note").is_some()
}

fn row_title(vcx: &VisualTestContext, key: &str) -> Option<String> {
    label_with_id(vcx, &format!("settings-row-title-{key}"))
}

/// An `L10n` for Turkish over the embedded catalogs.
fn l10n_for(tag: &str) -> L10n {
    let catalogs = Catalogs::embedded();
    let ctx = NegotiationCtx {
        sources: vec![PrefSource::Setting(tag.to_string())],
        ..Default::default()
    };
    let neg = negotiate(&ctx, catalogs.locales());
    L10n::new(build_runtime(
        &catalogs,
        &neg,
        &FormatInputs::new(&neg, "UTC"),
        1,
    ))
}

fn rows_tr() -> Vec<SearchRow> {
    let l10n = l10n_for("tr");
    settings_rows()
        .iter()
        .map(|row| SearchRow::of(row, &l10n))
        .collect()
}

fn tol() -> SearchOpts {
    SearchOpts {
        turkish_tolerant: true,
        mode: MatchMode::Contains,
    }
}

#[gpui::test]
fn theme_change_persists_and_applies(cx: &mut TestAppContext) {
    let (vcx, paths) = settings_tab(cx);
    click_segment(vcx, "ui.theme", "light");
    vcx.run_until_parked();
    assert_eq!(vcx.cx.read(|cx| Theme::get(cx).mode), ThemeMode::Light);
    assert!(read(paths.settings_toml()).contains("theme = \"light\""));
}

#[gpui::test]
fn language_change_is_live(cx: &mut TestAppContext) {
    let (vcx, _) = settings_tab(cx);
    assert_eq!(nav_label(vcx, 0), "General");
    choose(vcx, "ui.language", "tr");
    vcx.run_until_parked();
    assert_eq!(nav_label(vcx, 0), "Genel");
    assert_eq!(tab_title(vcx), "Ayarlar");
}

#[gpui::test]
fn language_choices_are_system_and_the_catalogs_native_names(cx: &mut TestAppContext) {
    let (vcx, _) = settings_tab(cx);
    let values: Vec<String> = vcx
        .cx
        .read(|cx| rows::choices(cx, "ui.language"))
        .into_iter()
        .map(|c| c.value)
        .collect();
    assert_eq!(values, ["system", "en", "tr"]);
    let native: Vec<String> = vcx
        .cx
        .read(|cx| rows::choices(cx, "ui.language"))
        .into_iter()
        .filter_map(|c| match c.label {
            rows::ChoiceLabel::Data(name) => Some(name.to_string()),
            rows::ChoiceLabel::Msg(_) => None,
        })
        .collect();
    assert_eq!(native, ["English", "Türkçe"]);
}

#[cfg(target_os = "macos")]
#[gpui::test]
fn text_rendering_change_needs_restart(cx: &mut TestAppContext) {
    let (vcx, paths) = settings_tab(cx);
    assert_eq!(app_font_smoothing_override(), Some(0));
    reveal(vcx, "ui.text_rendering");
    assert!(!restart_note_visible(vcx));
    click_segment(vcx, "ui.text_rendering", "standard");
    vcx.run_until_parked();
    assert!(read(paths.settings_toml()).contains("text_rendering = \"standard\""));
    assert!(restart_note_visible(vcx));
    // The running process is unchanged: restart-only (R11).
    assert_eq!(app_font_smoothing_override(), Some(0));
    // Back to what the process runs with: nothing to restart for.
    click_segment(vcx, "ui.text_rendering", "crisp");
    vcx.run_until_parked();
    assert!(!restart_note_visible(vcx));
}

#[cfg(not(target_os = "macos"))]
#[gpui::test]
fn text_rendering_row_hidden(cx: &mut TestAppContext) {
    let (vcx, _) = settings_tab(cx);
    reveal(vcx, "ui.theme");
    assert!(!row_visible(vcx, "ui.text_rendering"));
    assert!(settings_rows().iter().all(|r| r.key != "ui.text_rendering"));
}

#[cfg(target_os = "macos")]
#[gpui::test]
fn restart_now_asks_first_and_flushes_before_restarting(cx: &mut TestAppContext) {
    let (vcx, paths) = settings_tab(cx);
    click_segment(vcx, "ui.text_rendering", "standard");
    let mut restarted = vcx.cx.expect_restart();
    click(vcx, "settings-restart-now".to_string());
    assert_eq!(dialog_title(vcx), "Restart Hacivat?");
    assert_eq!(layer_ids(vcx), ["dialog"]);
    assert!(
        restarted.try_recv().unwrap().is_none(),
        "asked, not restarted"
    );
    // "Later" (Escape) closes the dialog and does not restart.
    vcx.simulate_keystrokes("escape");
    assert!(layer_ids(vcx).is_empty());
    assert!(restarted.try_recv().unwrap().is_none());
    // "Restart now" is the focused primary button.
    click(vcx, "settings-restart-now".to_string());
    vcx.simulate_keystrokes("enter");
    vcx.run_until_parked();
    assert!(layer_ids(vcx).is_empty());
    assert!(restarted.try_recv().unwrap().is_some(), "restart requested");
    assert!(read(paths.settings_toml()).contains("text_rendering = \"standard\""));
}

#[gpui::test]
fn a_restart_flushes_the_pending_settings_and_layout_writes(cx: &mut TestAppContext) {
    let (vcx, paths) = settings_tab(cx);
    // Nothing is parked in between: the background writers have not run.
    vcx.cx.update(|cx| {
        update_settings(cx, |s| s.keymap.preset = KeymapPreset::Datagrip);
        flush_pending_writes(cx);
    });
    assert!(read(paths.settings_toml()).contains("preset = \"datagrip\""));
    let workspace = std::fs::read_to_string(paths.workspace_json()).unwrap();
    assert!(workspace.contains("settings"), "{workspace}");
}

#[test]
fn search_matches_english_layer_in_turkish() {
    assert_eq!(filter_rows("theme", &rows_tr(), &tol()), ["ui.theme"]);
    assert_eq!(filter_rows("tema", &rows_tr(), &tol()), ["ui.theme"]);
    assert_eq!(filter_rows("dil", &rows_tr(), &tol()), ["ui.language"]);
}

#[gpui::test]
fn typing_in_the_search_field_filters_the_rows(cx: &mut TestAppContext) {
    let (vcx, _) = settings_tab(cx);
    assert!(row_visible(vcx, "ui.language"));
    assert!(!row_visible(vcx, "ui.theme"), "the General section only");
    click(vcx, "settings-search".to_string());
    vcx.simulate_input("them");
    assert!(row_visible(vcx, "ui.theme"));
    assert!(!row_visible(vcx, "ui.language"));
    assert_eq!(row_title(vcx, "ui.theme").as_deref(), Some("Theme"));
    vcx.simulate_input("xyz");
    assert!(!row_visible(vcx, "ui.theme"));
    assert!(label_with_id(vcx, "settings-no-results").is_some());
    // Choosing a section clears the search.
    click(vcx, "settings-nav-1".to_string());
    assert!(row_visible(vcx, "ui.theme"));
    assert!(label_with_id(vcx, "settings-no-results").is_none());
}

#[gpui::test]
fn the_settings_tab_has_its_own_key_context(cx: &mut TestAppContext) {
    let (vcx, _) = settings_tab(cx);
    assert_eq!(active_tab_kind_name(vcx), Some("settings"));
    assert!(context_stack(vcx).0.contains(&"Settings"));
}

fn active_tab_kind_name(vcx: &VisualTestContext) -> Option<&'static str> {
    use hacivat_app::shell::TabKind;
    let ws = workspace(vcx);
    vcx.cx.read(|cx| match ws.read(cx).active_tab_kind() {
        Some(TabKind::Registered(kind)) => Some(kind),
        _ => None,
    })
}

#[gpui::test]
fn regional_preview_is_turkish(cx: &mut TestAppContext) {
    let (vcx, _) = settings_tab_with(cx, support_platform(), "us", "Europe/Istanbul");
    choose(vcx, "ui.regional_format", "tr-TR");
    vcx.run_until_parked();
    assert_eq!(regional_preview(vcx), "1.234,5 · 11.11.2026 14:05");
    choose(vcx, "ui.regional_format", "en-US");
    vcx.run_until_parked();
    assert!(
        regional_preview(vcx).starts_with("1,234.5 · "),
        "{}",
        regional_preview(vcx)
    );
}

#[gpui::test]
fn keymap_preset_reloads_and_warns_on_polish(cx: &mut TestAppContext) {
    let (vcx, paths) = settings_tab_with(cx, KeymapPlatform::Windows, "pl", "UTC");
    choose(vcx, "keymap.preset", "datagrip");
    vcx.run_until_parked();
    assert_eq!(
        vcx.cx.read(|cx| cx.global::<ActiveKeymap>().preset),
        KeymapPreset::Datagrip
    );
    assert!(read(paths.settings_toml()).contains("preset = \"datagrip\""));
    let t = toast_texts(vcx);
    assert_eq!(t.len(), 1);
    assert!(
        t[0].contains("Ctrl+Alt+S") && t[0].contains('ś'),
        "{}",
        t[0]
    );
    choose(vcx, "keymap.preset", "hacivat");
    vcx.run_until_parked();
    // No new warning: hacivat has no collisions.
    assert_eq!(toast_texts(vcx).len(), 1);
}

#[gpui::test]
fn choosing_the_already_chosen_segment_changes_nothing(cx: &mut TestAppContext) {
    let (vcx, _) = settings_tab_with(cx, KeymapPlatform::Windows, "pl", "UTC");
    choose(vcx, "keymap.preset", "datagrip");
    assert_eq!(toast_texts(vcx).len(), 1);
    choose(vcx, "keymap.preset", "datagrip");
    assert_eq!(toast_texts(vcx).len(), 1, "no duplicate warning");
}

#[gpui::test]
fn collision_items_come_from_fluent_and_follow_the_language(cx: &mut TestAppContext) {
    let (vcx, _) = settings_tab_with(cx, KeymapPlatform::Windows, "pl", "UTC");
    choose(vcx, "keymap.preset", "datagrip");
    let toast = toast_texts(vcx).remove(0);
    assert!(toast.contains("Ctrl+Alt+S \u{2192} \u{15b}"), "{toast}");
    switch_to(vcx, "tr");
    vcx.run_until_parked();
    let toast = toast_texts(vcx).remove(0);
    assert!(toast.starts_with("Bu klavye d"), "{toast}");
    assert!(toast.contains("Ctrl+Alt+S \u{2192} \u{15b}"), "{toast}");
}

#[gpui::test]
fn section_headings_are_pre_cased_small_caps_from_fluent(cx: &mut TestAppContext) {
    let (vcx, _) = settings_tab(cx);
    assert_eq!(
        label_with_id(vcx, "settings-section-title").as_deref(),
        Some("GENERAL")
    );
    switch_to(vcx, "tr");
    vcx.run_until_parked();
    assert_eq!(
        label_with_id(vcx, "settings-section-title").as_deref(),
        Some("GENEL")
    );
}

#[gpui::test]
fn the_title_bar_language_control_is_a_focusable_segmented(cx: &mut TestAppContext) {
    let vcx = shell(cx, "en");
    assert!(status_bar_text(vcx).contains("EN"));
    // Segments are `segmented-language-switch-<i>`; click TR.
    click(vcx, "segmented-language-switch-1".to_string());
    assert_eq!(palette_label(vcx), "Ara veya komut çalıştır");
    // Focus the EN segment and press Enter: only that segment's handler switches to English, so
    // the focused handle is the EN segment's.
    focus_segment(vcx, "segmented-language-switch-0");
    press_enter(vcx);
    assert_eq!(palette_label(vcx), "Search or run a command");
    // And the TR segment's, the other way.
    focus_segment(vcx, "segmented-language-switch-1");
    press_enter(vcx);
    assert_eq!(palette_label(vcx), "Ara veya komut çalıştır");
}

#[gpui::test]
fn the_language_segments_are_announced_by_their_native_names(cx: &mut TestAppContext) {
    use hacivat_ui::popover::{a11y_record, a11y_take};
    let vcx = shell(cx, "en");
    a11y_record();
    vcx.cx.update(|cx| cx.refresh_windows());
    vcx.run_until_parked();
    let names: Vec<String> = a11y_take()
        .into_iter()
        .filter(|d| d.role == Some(gpui::Role::Button))
        .map(|d| d.name.to_string())
        .collect();
    assert!(names.iter().any(|n| n == "English"), "{names:?}");
    assert!(names.iter().any(|n| n == "Türkçe"), "{names:?}");
    assert!(!names.iter().any(|n| n == "TR" || n == "EN"), "{names:?}");
}

/// Focuses the element with debug selector `name` with a mouse down (no release, so no click).
fn focus_segment(vcx: &mut VisualTestContext, name: &str) {
    let at = center(vcx, name.to_string());
    vcx.simulate_mouse_down(at, gpui::MouseButton::Left, Modifiers::none());
    vcx.run_until_parked();
}

/// Enter down and up: GPUI turns it into a click on key-up (the harness sends key-down only).
fn press_enter(vcx: &mut VisualTestContext) {
    vcx.simulate_keystrokes("enter");
    vcx.simulate_event(gpui::KeyUpEvent {
        keystroke: gpui::Keystroke::parse("enter").unwrap(),
    });
    vcx.run_until_parked();
}

// ## m2a-t06-form-primitives

/// The focus handle the window holds now.
fn focused_handle(vcx: &mut VisualTestContext) -> Option<gpui::FocusHandle> {
    vcx.update(|window, cx| window.focused(cx))
}

#[gpui::test]
fn the_title_bar_theme_segments_rove_with_the_arrow_keys(cx: &mut TestAppContext) {
    let (vcx, paths) = settings_tab(cx);
    let dark = vcx.cx.read(|cx| Theme::get(cx).mode) == ThemeMode::Dark;
    let (pressed, other) = if dark { (0, 1) } else { (1, 0) };
    // A press (no release) focuses a segment without clicking it: learn both handles.
    focus_segment(vcx, &format!("segmented-theme-switch-{other}"));
    let other_handle = focused_handle(vcx).expect("the other segment takes focus");
    focus_segment(vcx, &format!("segmented-theme-switch-{pressed}"));
    let pressed_handle = focused_handle(vcx).expect("the pressed segment takes focus");
    assert_ne!(pressed_handle, other_handle);

    // Tab (GPUI's tab order) reaches the pressed segment and never the other one.
    let stops = vcx.update(|window, cx| {
        window.blur(cx);
        let mut seen = Vec::new();
        for _ in 0..256 {
            window.focus_next(cx);
            match window.focused(cx) {
                Some(h) if seen.contains(&h) => break,
                Some(h) => seen.push(h),
                None => break,
            }
        }
        seen
    });
    assert!(stops.contains(&pressed_handle), "the group is a tab stop");
    assert!(!stops.contains(&other_handle), "one tab stop per group");

    // Tab to it, then → moves to the other segment and presses it.
    vcx.update(|window, cx| {
        window.blur(cx);
        for _ in 0..256 {
            window.focus_next(cx);
            if window.focused(cx).as_ref() == Some(&pressed_handle) {
                break;
            }
        }
    });
    assert_eq!(focused_handle(vcx), Some(pressed_handle));
    vcx.simulate_keystrokes("right");
    vcx.run_until_parked();
    let (mode, value) = if dark {
        (ThemeMode::Light, "light")
    } else {
        (ThemeMode::Dark, "dark")
    };
    assert_eq!(vcx.cx.read(|cx| Theme::get(cx).mode), mode);
    assert_eq!(focused_handle(vcx), Some(other_handle));
    assert!(read(paths.settings_toml()).contains(&format!("theme = \"{value}\"")));
}

// ## m3b-t42-settings: the M3b settings rows (R-M3-44; NOTES surface 25).

use hacivat_app::settings_ui::value_limit::SizeUnit;
use hacivat_app::shell::AppState;
use hacivat_settings::{
    EditorSettings, ExecSettings, GridSettings, HistorySettings, Settings, ValueLimit,
};

/// The settings the shell holds.
fn settings(vcx: &VisualTestContext) -> Settings {
    vcx.cx.read(|cx| cx.global::<AppState>().settings.clone())
}

/// The largest-value field's text and unit.
fn value_limit_field(vcx: &VisualTestContext) -> (String, SizeUnit) {
    vcx.cx.read(|cx| {
        let page = hacivat_app::settings_ui::open_page(cx).expect("the settings page is open");
        let page = page.read(cx);
        (page.value_limit_text(cx), page.value_limit_unit())
    })
}

/// Clicks the largest-value field, replaces its text with `text` and presses Enter.
fn type_value_limit(vcx: &mut VisualTestContext, text: &str) {
    reveal(vcx, "results.max_value_bytes");
    click(vcx, "settings-max-value-field".to_string());
    // Clear it wherever the click put the caret: to the end, then back over every character.
    let len = value_limit_field(vcx).0.chars().count();
    for _ in 0..len {
        vcx.simulate_keystrokes("right");
    }
    for _ in 0..len {
        vcx.simulate_keystrokes("backspace");
    }
    vcx.simulate_input(text);
    vcx.simulate_keystrokes("enter");
    vcx.run_until_parked();
}

fn clamp_note(vcx: &VisualTestContext) -> Option<String> {
    label_with_id(vcx, "settings-max-value-clamped")
}

/// Sets the largest value as a hand edit of the file would (not through the row).
fn set_value_limit(vcx: &mut VisualTestContext, bytes: u32) {
    vcx.cx.update(|cx| {
        update_settings(cx, |s| {
            s.results.max_value_bytes = ValueLimit::new(bytes).unwrap();
        });
        cx.refresh_windows();
    });
    vcx.run_until_parked();
}

#[gpui::test]
fn each_row_writes_through_update_settings(cx: &mut TestAppContext) {
    let (vcx, paths) = settings_tab(cx);
    // `auto` is the default and writes nothing (T33): choosing another row leaves it out.
    choose(vcx, "exec.error_policy_ask", "off");
    choose(vcx, "exec.error_policy_ask", "on");
    assert!(!read(paths.settings_toml()).contains("find_turkish_tolerant"));
    // (row, the choice away from its default, the line `settings.toml` then holds).
    let cases = [
        ("editor.tab_size", "8", "tab_size = 8"),
        ("editor.insert_spaces", "off", "insert_spaces = false"),
        (
            "editor.copy_line_without_selection",
            "off",
            "copy_line_without_selection = false",
        ),
        ("editor.caret_blink", "off", "caret_blink = \"off\""),
        (
            "editor.find_turkish_tolerant",
            "on",
            "find_turkish_tolerant = \"on\"",
        ),
        (
            "exec.confirm_unsafe_where",
            "off",
            "confirm_unsafe_where = false",
        ),
        ("exec.error_policy_ask", "off", "error_policy_ask = false"),
        ("grid.raw_display", "on", "raw_display = true"),
        ("history.enabled", "off", "enabled = false"),
    ];
    for (key, value, line) in cases {
        let before = settings(vcx);
        choose(vcx, key, value);
        let after = settings(vcx);
        assert_ne!(before, after, "{key}: the shell's settings changed");
        assert_eq!(rows::current_value(&after, key), value, "{key}");
        let file = read(paths.settings_toml());
        assert!(file.contains(line), "{key}: {file}");
    }
    let s = settings(vcx);
    assert_eq!(s.editor.tab_size.get(), 8);
    assert!(!s.editor.insert_spaces && !s.editor.copy_line_without_selection);
    assert!(!s.exec.confirm_unsafe_where && !s.exec.error_policy_ask);
    assert!(s.grid.raw_display && !s.history.enabled);

    // Back to the defaults: the file holds none of them (`auto` writes nothing, T33).
    for (key, value) in [
        ("editor.tab_size", "4"),
        ("editor.insert_spaces", "on"),
        ("editor.copy_line_without_selection", "on"),
        ("editor.caret_blink", "on"),
        ("editor.find_turkish_tolerant", "auto"),
        ("exec.confirm_unsafe_where", "on"),
        ("exec.error_policy_ask", "on"),
        ("grid.raw_display", "off"),
        ("history.enabled", "on"),
    ] {
        choose(vcx, key, value);
    }
    let s = settings(vcx);
    assert_eq!(s.editor, EditorSettings::default());
    assert_eq!(s.exec, ExecSettings::default());
    assert_eq!(s.grid, GridSettings::default());
    assert_eq!(s.history, HistorySettings::default());
    // The store keeps a key the file has and rewrites its value (`store.rs` `write_key_or`).
    let file = read(paths.settings_toml());
    for (key, _, line) in cases {
        assert!(!file.contains(line), "{key}: {file}");
    }
    assert!(file.contains("find_turkish_tolerant = \"auto\""), "{file}");

    // The largest value, through the same writer.
    type_value_limit(vcx, "2");
    assert_eq!(settings(vcx).results.max_value_bytes.get(), 2 << 20);
    let file = read(paths.settings_toml());
    assert!(file.contains("max_value_bytes = 2097152"), "{file}");
}

#[test]
fn search_finds_rows_in_tr() {
    let rows = rows_tr();
    let t = tol();
    let find = |q: &str| filter_rows(q, &rows, &t);
    assert_eq!(find("sekme"), ["editor.tab_size"]);
    // The Turkish İ/ı fold: dotted capitals and dotless lower case find "girintile".
    assert_eq!(find("GİRİNTİ"), ["editor.insert_spaces"]);
    assert_eq!(find("gırıntı"), ["editor.insert_spaces"]);
    // Accents folded: "imleç", "geçmiş", "başarısız".
    assert_eq!(find("imlec"), ["editor.caret_blink"]);
    assert_eq!(find("gecmis"), ["history.enabled"]);
    assert_eq!(find("basarisiz"), ["exec.error_policy_ask"]);
    assert_eq!(find("esnek"), ["editor.find_turkish_tolerant"]);
    assert_eq!(find("WHERE"), ["exec.confirm_unsafe_where"]);
    assert_eq!(find("hücre"), ["results.max_value_bytes"]);
    assert_eq!(find("ham"), ["grid.raw_display"]);
    // The English layer, in a Turkish UI.
    assert_eq!(find("raw"), ["grid.raw_display"]);
    assert_eq!(find("tab size"), ["editor.tab_size"]);
    assert_eq!(find("history"), ["history.enabled"]);
    assert_eq!(find("largest value"), ["results.max_value_bytes"]);
}

#[gpui::test]
fn a_search_result_names_its_section(cx: &mut TestAppContext) {
    let (vcx, _) = settings_tab(cx);
    switch_to(vcx, "tr");
    vcx.run_until_parked();
    click(vcx, "settings-search".to_string());
    vcx.simulate_input("GEÇMİŞ");
    assert!(row_visible(vcx, "history.enabled"));
    assert!(!row_visible(vcx, "editor.tab_size"));
    assert_eq!(
        label_with_id(vcx, "settings-row-section-history.enabled").as_deref(),
        Some("Geçmiş")
    );
    assert!(label_with_id(vcx, "settings-section-title").is_none());
    // Without a search, a row names no section (its heading does).
    click(vcx, "settings-nav-6".to_string());
    assert!(row_visible(vcx, "history.enabled"));
    assert!(label_with_id(vcx, "settings-row-section-history.enabled").is_none());
    assert_eq!(
        label_with_id(vcx, "settings-section-title").as_deref(),
        Some("GEÇMİŞ")
    );
}

#[gpui::test]
fn value_limit_clamps(cx: &mut TestAppContext) {
    let (vcx, paths) = settings_tab(cx);
    reveal(vcx, "results.max_value_bytes");
    assert_eq!(value_limit_field(vcx), ("1".to_string(), SizeUnit::Mb));
    assert_eq!(clamp_note(vcx), None);
    assert!(
        label_with_id(vcx, "settings-row-note-results.max_value_bytes")
            .is_some_and(|n| n.starts_with("From 64 KB to 64 MB.")),
    );

    // Above 64 MB: set to 64 MB, and the row says so.
    type_value_limit(vcx, "128");
    assert_eq!(settings(vcx).results.max_value_bytes.get(), 64 << 20);
    assert_eq!(value_limit_field(vcx), ("64".to_string(), SizeUnit::Mb));
    assert_eq!(
        clamp_note(vcx).as_deref(),
        Some("128 MB is above the largest allowed, so it was set to 64 MB.")
    );
    assert!(read(paths.settings_toml()).contains("max_value_bytes = 67108864"));

    // Below 64 KB: set to 64 KB, shown in KB.
    type_value_limit(vcx, "0.01");
    assert_eq!(settings(vcx).results.max_value_bytes.get(), 64 << 10);
    assert_eq!(value_limit_field(vcx), ("64".to_string(), SizeUnit::Kb));
    assert_eq!(
        clamp_note(vcx).as_deref(),
        Some("0.01 MB is below the smallest allowed, so it was set to 64 KB.")
    );

    // In range: the note goes; the unit stays KB.
    type_value_limit(vcx, "512");
    assert_eq!(settings(vcx).results.max_value_bytes.get(), 512 << 10);
    assert_eq!(value_limit_field(vcx), ("512".to_string(), SizeUnit::Kb));
    assert_eq!(clamp_note(vcx), None);
    assert!(read(paths.settings_toml()).contains("max_value_bytes = 524288"));

    // Not a size: nothing is written and the field shows the saved size again.
    type_value_limit(vcx, "lots");
    assert_eq!(settings(vcx).results.max_value_bytes.get(), 512 << 10);
    assert_eq!(value_limit_field(vcx), ("512".to_string(), SizeUnit::Kb));
}

#[gpui::test]
fn the_value_limit_reads_and_writes_the_regional_format(cx: &mut TestAppContext) {
    let (vcx, _) = settings_tab_with(cx, support_platform(), "us", "Europe/Istanbul");
    choose(vcx, "ui.regional_format", "tr-TR");
    vcx.run_until_parked();
    set_value_limit(vcx, 1536 << 10);
    reveal(vcx, "results.max_value_bytes");
    assert_eq!(value_limit_field(vcx), ("1.536".to_string(), SizeUnit::Kb));
    // `.` groups in Turkey: 2.048 KB is 2 MB, still shown in KB.
    type_value_limit(vcx, "2.048");
    assert_eq!(settings(vcx).results.max_value_bytes.get(), 2 << 20);
    assert_eq!(value_limit_field(vcx), ("2.048".to_string(), SizeUnit::Kb));
    // `,` is the decimal separator, in the field and in the note.
    type_value_limit(vcx, "1,5");
    assert_eq!(settings(vcx).results.max_value_bytes.get(), 64 << 10);
    assert_eq!(
        clamp_note(vcx).as_deref(),
        Some("1,5 KB is below the smallest allowed, so it was set to 64 KB.")
    );
    // A hand-edited size follows into the field (here a whole number of megabytes).
    set_value_limit(vcx, 8 << 20);
    assert_eq!(value_limit_field(vcx), ("8".to_string(), SizeUnit::Mb));
}

#[gpui::test]
fn choosing_the_other_unit_applies_the_number_in_it(cx: &mut TestAppContext) {
    let (vcx, paths) = settings_tab(cx);
    // Active, so the field's focus-out listener runs as it does in the app.
    in_window(vcx, |window, _| window.activate_window());
    reveal(vcx, "results.max_value_bytes");
    // Type 512 (no Enter), then choose KB: 512 KB, not 512 MB set to the bound on the way.
    click(vcx, "settings-max-value-field".to_string());
    vcx.simulate_keystrokes("right backspace");
    vcx.simulate_input("512");
    click(vcx, "settings-max-value-unit".to_string());
    assert_eq!(
        settings(vcx).results.max_value_bytes.get(),
        1 << 20,
        "not yet"
    );
    click(vcx, "menu-item-0".to_string());
    assert_eq!(settings(vcx).results.max_value_bytes.get(), 512 << 10);
    assert_eq!(value_limit_field(vcx), ("512".to_string(), SizeUnit::Kb));
    assert_eq!(clamp_note(vcx), None);
    assert!(read(paths.settings_toml()).contains("max_value_bytes = 524288"));
    // The number shown, in the other unit: 512 MB, set to 64 MB.
    click(vcx, "settings-max-value-unit".to_string());
    click(vcx, "menu-item-1".to_string());
    assert_eq!(settings(vcx).results.max_value_bytes.get(), 64 << 20);
    assert_eq!(value_limit_field(vcx), ("64".to_string(), SizeUnit::Mb));
    assert_eq!(
        clamp_note(vcx).as_deref(),
        Some("512 MB is above the largest allowed, so it was set to 64 MB.")
    );
}

#[gpui::test]
fn leaving_the_field_keeps_what_was_typed(cx: &mut TestAppContext) {
    let (vcx, _) = settings_tab(cx);
    // Focus-out listeners fire only in an active window.
    in_window(vcx, |window, _| window.activate_window());
    reveal(vcx, "results.max_value_bytes");
    click(vcx, "settings-max-value-field".to_string());
    vcx.simulate_keystrokes("right backspace");
    vcx.simulate_input("3");
    assert_eq!(
        settings(vcx).results.max_value_bytes.get(),
        1 << 20,
        "not yet"
    );
    click(vcx, "settings-search".to_string());
    assert_eq!(settings(vcx).results.max_value_bytes.get(), 3 << 20);
    // Escape puts the saved size back.
    click(vcx, "settings-max-value-field".to_string());
    vcx.simulate_keystrokes("right backspace");
    vcx.simulate_input("9");
    vcx.simulate_keystrokes("escape");
    assert_eq!(value_limit_field(vcx), ("3".to_string(), SizeUnit::Mb));
    assert_eq!(settings(vcx).results.max_value_bytes.get(), 3 << 20);
}

#[gpui::test]
fn the_four_new_sections_show_their_rows_and_notes(cx: &mut TestAppContext) {
    let (vcx, _) = settings_tab(cx);
    let sections = [
        (
            3,
            "Editor",
            "EDITOR",
            &[
                "editor.tab_size",
                "editor.insert_spaces",
                "editor.copy_line_without_selection",
                "editor.caret_blink",
                "editor.find_turkish_tolerant",
            ][..],
        ),
        (
            4,
            "Execution",
            "EXECUTION",
            &["exec.confirm_unsafe_where", "exec.error_policy_ask"][..],
        ),
        (
            5,
            "Results",
            "RESULTS",
            &["results.max_value_bytes", "grid.raw_display"][..],
        ),
        (6, "History", "HISTORY", &["history.enabled"][..]),
    ];
    for (index, nav, heading, keys) in sections {
        assert_eq!(nav_label(vcx, index), nav);
        click(vcx, format!("settings-nav-{index}"));
        assert_eq!(
            label_with_id(vcx, "settings-section-title").as_deref(),
            Some(heading)
        );
        for key in keys {
            assert!(row_visible(vcx, key), "{key}");
            assert!(row_title(vcx, key).is_some(), "{key}");
            let note = rows::row_note(key).is_some();
            let id = format!("settings-row-note-{key}");
            assert_eq!(label_with_id(vcx, &id).is_some(), note, "{key}");
        }
    }
    assert_eq!(
        label_with_id(vcx, "settings-row-note-history.enabled").as_deref(),
        Some(
            "Up to 10,000 statements per connection are kept for 90 days; recognised passwords \
             and secrets are masked."
        )
    );
}

#[gpui::test]
fn editor_rows_reach_the_open_editors(cx: &mut TestAppContext) {
    use hacivat_app::console::{ConsoleId, ConsoleState, Consoles};
    use hacivat_ui_editor::{CaretBlink, DocId, EditorConfig, EditorView, LanguageService};
    let (vcx, _) = settings_tab(cx);
    let editor = in_window(vcx, |window, cx| {
        let service: std::rc::Rc<dyn LanguageService> =
            hacivat_sql_editor::SqlLanguageService::new();
        // Test data, not a product label.
        let label: hacivat_ui::text::TextFn =
            std::rc::Rc::new(|_: &gpui::App| gpui::SharedString::from("SQL"));
        use gpui::AppContext as _;
        let editor = cx.new(|cx| {
            EditorView::new(
                DocId(42),
                "",
                EditorConfig::default(),
                service,
                label,
                window,
                cx,
            )
        });
        Consoles::register(
            cx,
            ConsoleState {
                id: ConsoleId(0),
                tab: hacivat_ui::panes::TabId(42),
                n: 1,
                profile: None,
                client: hacivat_session::session::ClientId::DEFAULT,
                session: None,
                editor: Some(editor.clone()),
                results: None,
                sql: None,
                log: hacivat_exec::SessionLog::new(),
                service: None,
            },
        );
        editor
    });
    choose(vcx, "editor.tab_size", "2");
    choose(vcx, "editor.insert_spaces", "off");
    choose(vcx, "editor.copy_line_without_selection", "off");
    choose(vcx, "editor.caret_blink", "off");
    let cfg = vcx.cx.read(|cx| *editor.read(cx).config());
    assert_eq!(cfg.tab_size, 2);
    assert!(!cfg.insert_spaces && !cfg.copy_line_without_selection);
    assert_eq!(cfg.caret_blink, CaretBlink::Off);
}

/// Every key the settings file can hold, as the store writes them.
fn written_keys() -> Vec<String> {
    use hacivat_settings::{
        CaretBlinkPref, FindTolerancePref, KeymapPreset, LanguagePref, TabSize, TextRenderingPref,
        ThemePref, TimeZonePref,
    };
    let mut s = Settings::default();
    s.ui.language = LanguagePref::Tag("tr".into());
    s.ui.regional_format = LanguagePref::Tag("tr-TR".into());
    s.ui.time_zone = TimeZonePref::Iana("Europe/Istanbul".into());
    s.ui.theme = ThemePref::Light;
    s.ui.text_rendering = TextRenderingPref::Standard;
    s.keymap.preset = KeymapPreset::Datagrip;
    s.editor.tab_size = TabSize::new(2).unwrap();
    s.editor.insert_spaces = false;
    s.editor.copy_line_without_selection = false;
    s.editor.caret_blink = CaretBlinkPref::Off;
    s.editor.find_turkish_tolerant = FindTolerancePref::Off;
    s.exec.confirm_unsafe_where = false;
    s.exec.error_policy_ask = false;
    s.results.max_value_bytes = ValueLimit::new(2 << 20).unwrap();
    s.history.enabled = false;
    s.grid.raw_display = true;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("settings.toml");
    hacivat_settings::save_changes(&path, &s).unwrap();
    let mut table = String::new();
    let mut keys = Vec::new();
    for line in read(path).lines() {
        let line = line.trim();
        if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            table = name.to_string();
        } else if let Some((key, _)) = line.split_once(" =") {
            keys.push(if table.is_empty() {
                key.to_string()
            } else {
                format!("{table}.{key}")
            });
        }
    }
    keys
}

#[test]
fn every_setting_has_a_row_but_the_version_and_the_time_zone() {
    let keys = written_keys();
    assert!(keys.len() > 15, "{keys:?}");
    let without: Vec<&str> = keys
        .iter()
        .map(String::as_str)
        .filter(|k| rows::row_section(k).is_none())
        .collect();
    // The time zone has no row yet (it follows the system unless set by hand): reported.
    assert_eq!(without, ["version", "ui.time_zone"]);
}
