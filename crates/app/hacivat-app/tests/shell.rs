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
