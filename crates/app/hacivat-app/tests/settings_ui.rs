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
