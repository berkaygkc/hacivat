//! The theme preference (`ui.theme`): dark, light, or the OS appearance.

use hacivat_settings::ThemePref;
use hacivat_ui_core::gpui_surface::{App, Global, WindowAppearance};
use hacivat_ui_core::theme::{Theme, ThemeMode};

/// The preference the installed theme follows. With `System`, the workspace window re-applies
/// it whenever the OS appearance changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ThemeSetting(pub ThemePref);

impl Global for ThemeSetting {}

/// The theme mode for `pref` under the OS `appearance`.
pub fn theme_mode_for(pref: ThemePref, appearance: WindowAppearance) -> ThemeMode {
    match pref {
        ThemePref::Dark => ThemeMode::Dark,
        ThemePref::Light => ThemeMode::Light,
        ThemePref::System => match appearance {
            WindowAppearance::Light | WindowAppearance::VibrantLight => ThemeMode::Light,
            WindowAppearance::Dark | WindowAppearance::VibrantDark => ThemeMode::Dark,
        },
    }
}

/// Installs the theme for `pref` and repaints. With `System` the theme then follows the OS
/// appearance (the workspace observes it, see [`follow_appearance`]).
pub fn apply_theme(cx: &mut App, pref: ThemePref) {
    cx.set_global(ThemeSetting(pref));
    let mode = theme_mode_for(pref, cx.window_appearance());
    Theme::install(cx, mode);
    cx.refresh_windows();
}

/// Re-applies a `System` preference after the OS appearance changed to `appearance`; other
/// preferences ignore it.
pub fn follow_appearance(cx: &mut App, appearance: WindowAppearance) {
    let pref = cx
        .try_global::<ThemeSetting>()
        .map_or_else(ThemePref::default, |s| s.0);
    if pref != ThemePref::System {
        return;
    }
    let mode = theme_mode_for(pref, appearance);
    if cx.try_global::<Theme>().map(|t| t.mode) != Some(mode) {
        Theme::install(cx, mode);
        cx.refresh_windows();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_preferences_ignore_the_appearance() {
        for appearance in [
            WindowAppearance::Light,
            WindowAppearance::VibrantLight,
            WindowAppearance::Dark,
            WindowAppearance::VibrantDark,
        ] {
            assert_eq!(theme_mode_for(ThemePref::Dark, appearance), ThemeMode::Dark);
            assert_eq!(
                theme_mode_for(ThemePref::Light, appearance),
                ThemeMode::Light
            );
        }
        assert_eq!(
            theme_mode_for(ThemePref::System, WindowAppearance::VibrantDark),
            ThemeMode::Dark
        );
    }
}
