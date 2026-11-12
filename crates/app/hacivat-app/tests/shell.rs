//! The workspace shell (Task 13): runtime language and theme switches, layers, registered tab
//! content, the key-capture hook, layout restore and startup diagnostics. Line 4 of the full test
//! set runs it with `scene-check` and `pseudo`.

mod support;

use std::time::Duration;

use gpui::TestAppContext;
use hacivat_app::theme_pref::theme_mode_for;
use hacivat_l10n::{LoadDiag, ids};
use hacivat_settings::ThemePref;
use hacivat_ui::l10n::{EnglishDefaults, UiTextProvider as _};
use hacivat_ui::panes::Axis;
use hacivat_ui::toast::ToastKind;
use hacivat_ui_core::gpui_surface::WindowAppearance;
use hacivat_ui_core::theme::{LIGHT, Theme, ThemeMode};
use support::*;

#[gpui::test]
fn switching_language_rerenders_without_restart(cx: &mut TestAppContext) {
    let vcx = shell(cx, "en");
    assert_eq!(palette_label(vcx), "Search or run a command");
    switch_to(vcx, "tr");
    vcx.run_until_parked();
    assert_eq!(palette_label(vcx), "Ara veya komut çalıştır");
    assert_eq!(panel_title(vcx), "GEZGİN");
}

#[gpui::test]
fn two_rapid_switches_last_wins(cx: &mut TestAppContext) {
    let vcx = shell(cx, "en");
    switch_to(vcx, "tr");
    switch_to(vcx, "en");
    vcx.run_until_parked();
    assert_eq!(palette_label(vcx), "Search or run a command");
}

#[gpui::test]
fn toast_rerenders_after_language_switch(cx: &mut TestAppContext) {
    let vcx = shell(cx, "en");
    push_toast(vcx, ids::hv::STATUS_NO_CONNECTION.id, ToastKind::Info);
    assert_eq!(toast_texts(vcx), ["No connection"]);
    switch_to(vcx, "tr");
    vcx.run_until_parked();
    assert_eq!(toast_texts(vcx), ["Bağlantı yok"]);
}

#[gpui::test]
fn dialog_rerenders_after_language_switch(cx: &mut TestAppContext) {
    let vcx = shell(cx, "en");
    open_dialog_ids(
        vcx,
        ids::hv::CAPSULE_NO_CONNECTION.id,
        ids::hv::EXPLORER_EMPTY.id,
    );
    assert_eq!(
        (dialog_title(vcx), dialog_body(vcx)),
        ("No connection".into(), "No connections yet".into())
    );
    switch_to(vcx, "tr");
    vcx.run_until_parked();
    assert_eq!(
        (dialog_title(vcx), dialog_body(vcx)),
        ("Bağlantı yok".into(), "Henüz bağlantı yok".into())
    );
    assert_eq!(layer_ids(vcx), ["dialog"]);
}

#[gpui::test]
fn theme_switch_is_live(cx: &mut TestAppContext) {
    let vcx = shell(cx, "en");
    apply(vcx, ThemePref::Light);
    vcx.run_until_parked();
    assert_eq!(root_bg(vcx), LIGHT.colors.ground.to_hsla());
    assert_eq!(vcx.read(|cx| Theme::get(cx).mode), ThemeMode::Light);
}

#[test]
fn system_theme_follows_appearance() {
    assert_eq!(
        theme_mode_for(ThemePref::System, WindowAppearance::VibrantLight),
        ThemeMode::Light
    );
    assert_eq!(
        theme_mode_for(ThemePref::Dark, WindowAppearance::Light),
        ThemeMode::Dark
    );
}

#[cfg(feature = "scene-check")]
#[gpui::test]
fn one_filled_badge_holds(cx: &mut TestAppContext) {
    use hacivat_ui_core::named_rules::assert_one_filled_badge;
    use hacivat_ui_core::theme::DARK;
    let vcx = shell_demo(cx);
    vcx.update(|w, _| assert_one_filled_badge(w, DARK.colors.cp_fill.to_hsla(), true));
    let vcx = shell(cx, "en");
    vcx.update(|w, _| assert_one_filled_badge(w, DARK.colors.cp_fill.to_hsla(), false));
}

#[gpui::test]
fn layout_round_trips_through_workspace_json(cx: &mut TestAppContext) {
    let root = tempfile::tempdir().unwrap();
    let vcx = shell_at(cx, root.path(), "en");
    workspace(vcx).update_in(vcx, |w, win, cx| {
        w.split_focused(Axis::Horizontal, win, cx);
        w.new_console_tab(win, cx);
    });
    vcx.executor().advance_clock(Duration::from_millis(600));
    vcx.run_until_parked();
    let before = layout(vcx);
    assert_eq!(before.0.panes.len(), 2);
    close_all_windows(vcx);
    let vcx2 = shell_at(cx, root.path(), "en");
    assert_eq!(layout(vcx2), before); // layout() = (PaneTree, TileLayout)
}

#[gpui::test]
fn corrupt_overlay_keeps_language_and_reports_diag(cx: &mut TestAppContext) {
    let root = tempfile::tempdir().unwrap();
    write(
        root.path().join("config/locales/tr/hacivat-ui.ftl"),
        "hv-explorer-empty = Bağlantı yok gibi\n!!junk\n",
    );
    let vcx = shell_at(cx, root.path(), "tr");
    assert_eq!(panel_title(vcx), "GEZGİN");
    assert_eq!(explorer_empty(vcx), "Bağlantı yok gibi");
    assert_eq!(
        l10n_diags(vcx)
            .iter()
            .filter(|d| matches!(d, LoadDiag::Junk { .. }))
            .count(),
        1
    );
}

#[cfg(feature = "pseudo")]
#[gpui::test]
fn every_label_in_shell_is_from_fluent_or_exempt(cx: &mut TestAppContext) {
    use hacivat_ui::text::TextSource;
    let vcx = shell(cx, "en-XA");
    enable_registry(vcx);
    vcx.run_until_parked();
    let f = registry_frame(vcx);
    let fluent: Vec<_> = f
        .iter()
        .filter(|r| r.source == TextSource::Fluent)
        .collect();
    assert!(fluent.len() >= 10, "{}", fluent.len());
    for r in fluent {
        assert!(
            r.text.starts_with('[') && r.text.ends_with(']'),
            "{}: {}",
            r.path,
            r.text
        );
    }
    assert!(f.iter().any(|r| r.text == "[HACIVAT]")); // the brand is wrapped but not transformed (F6)
}

#[gpui::test]
fn layers_escape_pops_topmost_and_restores_focus(cx: &mut TestAppContext) {
    let vcx = shell(cx, "en");
    let base = focus_test_input(vcx); // a TextInput inside a test-registered tab
    push_test_layer(vcx, "a", "LayerA");
    let a_focus = focused(vcx);
    push_test_layer(vcx, "b", "LayerB");
    assert_eq!(
        context_stack(vcx).0,
        ["Workspace", "Test", "LayerA", "LayerB"]
    );
    vcx.simulate_keystrokes("escape");
    assert_eq!(layer_ids(vcx), ["a"]);
    assert_eq!(focused(vcx), a_focus);
    vcx.simulate_keystrokes("escape");
    assert!(layer_ids(vcx).is_empty());
    assert_eq!(focused(vcx), Some(base));
}

#[gpui::test]
fn registered_tab_content_renders_and_titles_follow_language(cx: &mut TestAppContext) {
    let vcx = shell(cx, "en");
    register_test_tab(vcx, "test", ids::hv::PANEL_EXPLORER.id); // title TextFn from the ID
    workspace(vcx).update_in(vcx, |w, win, cx| w.open_registered_tab("test", win, cx));
    assert_eq!(tab_title(vcx), "EXPLORER");
    assert!(vcx.debug_bounds("test-tab-content").is_some());
    workspace(vcx).update_in(vcx, |w, win, cx| w.open_registered_tab("test", win, cx));
    assert_eq!(tab_count(vcx), 2); // Console 1 + one test tab, not two
    switch_to(vcx, "tr");
    vcx.run_until_parked();
    assert_eq!(tab_title(vcx), "GEZGİN");
}

#[gpui::test]
fn key_capture_hook_runs_before_children(cx: &mut TestAppContext) {
    let vcx = shell(cx, "en");
    let input = focus_test_input(vcx);
    let seen = install_recording_hook(vcx, "ctrl-j"); // returns true only for ctrl-j
    vcx.simulate_keystrokes("ctrl-j");
    assert_eq!(
        seen.take(),
        vec![("ctrl-j".to_string(), vec!["Workspace", "Test"])]
    );
    assert_eq!(input_value(vcx, &input), "");
    vcx.simulate_input("x");
    assert_eq!(input_value(vcx, &input), "x");
}

#[gpui::test]
fn invalid_settings_value_shows_toast(cx: &mut TestAppContext) {
    let root = tempfile::tempdir().unwrap();
    write(
        root.path().join("config/settings.toml"),
        "[ui]\ntheme = 3\n",
    );
    let vcx = shell_at(cx, root.path(), "en");
    let t = toast_texts(vcx);
    assert_eq!(t.len(), 1);
    assert!(
        t[0].starts_with("Some settings could not be read") && t[0].contains("ui.theme"),
        "{}",
        t[0]
    );
}

#[test]
fn english_defaults_match_l10n_for_every_hui_message() {
    let (e, l) = (EnglishDefaults::new(), l10n_en());
    for id in message_ids_in(&workspace_file("locales/en/hui.ftl")) {
        assert_eq!(
            e.text(&id, &sample_ui_args(&id)).to_string(),
            l.t_dynamic(&id, sample_arg_values(&id)),
            "{id}"
        ); // samples: counts = 2, text vars = "x"
    }
}

// ---- fix round 1 -----------------------------------------------------------------------------

fn settings_file(root: &std::path::Path) -> String {
    std::fs::read_to_string(root.join("config/settings.toml")).unwrap_or_default()
}

#[gpui::test(iterations = 20)]
fn rapid_setting_changes_persist_the_last_value(cx: &mut TestAppContext) {
    use hacivat_app::shell::update_settings;
    use hacivat_settings::LanguagePref;
    let root = tempfile::tempdir().unwrap();
    let vcx = shell_at(cx, root.path(), "en");
    vcx.cx.update(|cx| {
        for tag in ["tr", "en", "tr", "en", "tr", "en"] {
            update_settings(cx, |s| s.ui.language = LanguagePref::Tag(tag.to_string()));
        }
    });
    vcx.run_until_parked();
    let file = settings_file(root.path());
    assert!(file.contains("language = \"en\""), "{file}");
}

#[gpui::test]
fn a_layout_change_survives_a_close_within_the_debounce(cx: &mut TestAppContext) {
    let root = tempfile::tempdir().unwrap();
    let vcx = shell_at(cx, root.path(), "en");
    workspace(vcx).update_in(vcx, |w, win, cx| w.split_focused(Axis::Vertical, win, cx));
    let before = layout(vcx);
    assert_eq!(before.0.panes.len(), 2);
    close_all_windows(vcx); // no clock advance: the 500 ms debounce has not run
    let vcx2 = shell_at(cx, root.path(), "en");
    assert_eq!(layout(vcx2), before);
}

#[gpui::test]
fn a_layout_change_is_written_when_the_app_quits(cx: &mut TestAppContext) {
    let root = tempfile::tempdir().unwrap();
    let vcx = shell_at(cx, root.path(), "en");
    workspace(vcx).update_in(vcx, |w, win, cx| w.toggle_explorer(win, cx));
    let path = root.path().join("data/workspace.json");
    assert!(!path.exists());
    cx.quit();
    let saved = std::fs::read_to_string(&path).unwrap();
    assert!(saved.contains("\"explorer_visible\": false"), "{saved}");
}

#[gpui::test]
fn an_inconsistent_workspace_json_is_set_aside(cx: &mut TestAppContext) {
    let root = tempfile::tempdir().unwrap();
    write(
        root.path().join("data/workspace.json"),
        "{\"version\": 1, \"tiles\": {\"explorer_w\": 1.0}, \"oops\": true}",
    );
    let vcx = shell_at(cx, root.path(), "en");
    assert_eq!(tab_count(vcx), 1);
    let set_aside: Vec<String> = std::fs::read_dir(root.path().join("data"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with("workspace.json.corrupt-"))
        .collect();
    assert_eq!(set_aside.len(), 1, "{set_aside:?}");
}

#[gpui::test]
fn restored_tiles_are_clamped_to_the_window(cx: &mut TestAppContext) {
    let root = tempfile::tempdir().unwrap();
    let vcx = shell_at(cx, root.path(), "en");
    workspace(vcx).update_in(vcx, |w, win, cx| w.new_console_tab(win, cx));
    close_all_windows(vcx);
    let path = root.path().join("data/workspace.json");
    let text = std::fs::read_to_string(&path)
        .unwrap()
        .replace("\"editor_h\": 322.0", "\"editor_h\": 100000.0");
    std::fs::write(&path, text).unwrap();
    let vcx2 = shell_at(cx, root.path(), "en");
    let (_, tiles) = layout(vcx2);
    assert!(
        tiles.editor_h < 2000.0 && tiles.editor_h >= 120.0,
        "{}",
        tiles.editor_h
    );
    assert_eq!(tab_count(vcx2), 2);
}

#[gpui::test]
fn session_overrides_never_write_settings(cx: &mut TestAppContext) {
    use hacivat_app::shell::update_settings;
    use hacivat_app::{SessionOverrides, Startup, start};
    use hacivat_l10n::OsLocale;
    use hacivat_settings::LocalPaths;
    let root = tempfile::tempdir().unwrap();
    let paths = LocalPaths::under(root.path());
    paths.ensure_dirs().unwrap();
    let outcome = hacivat_settings::load(&paths.settings_toml());
    let overrides = SessionOverrides {
        theme: Some(ThemePref::Light),
        language: Some("tr".to_string()),
    };
    let startup = Startup::with_overrides(paths, outcome, OsLocale::default(), &overrides);
    cx.update(|cx| {
        start(cx, startup, None).unwrap();
        assert_eq!(Theme::get(cx).mode, ThemeMode::Light);
        update_settings(cx, |s| s.ui.theme = ThemePref::Dark);
    });
    cx.run_until_parked();
    assert!(!root.path().join("config/settings.toml").exists());
}
