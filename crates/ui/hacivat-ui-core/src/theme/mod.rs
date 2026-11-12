//! Design tokens and the `Theme` global. Every UI element reads colours, type and metrics
//! through `Theme`; `generated.rs` is produced by `cargo xtask tokens`.

use gpui::{App, Global, Hsla, Rgba};

mod generated;

pub use generated::{Colors, DARK, LIGHT, Motion, Radii, SHARED, SharedColors, Spacing, TypeRamp};

/// Packed `0xRRGGBBAA`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Color(pub u32);

impl Color {
    pub fn to_hsla(self) -> Hsla {
        let [r, g, b, a] = self.0.to_be_bytes();
        Rgba {
            r: r as f32 / 255.0,
            g: g as f32 / 255.0,
            b: b as f32 / 255.0,
            a: a as f32 / 255.0,
        }
        .into()
    }
}

/// How a type style renders capitals. DESIGN.md `fontVariantCaps`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Caps {
    #[default]
    Normal,
    /// Small caps synthesized like the approved mock: pre-cased capitals at
    /// [`SYNTH_SCALE`](crate::fonts::SYNTH_SCALE) of the size, with tracking. Render with
    /// [`small_caps`](crate::fonts::small_caps); the font's `smcp`/`c2sc` are never used (PO
    /// decision, M0).
    SynthesizedSmallCaps,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TypeStyle {
    pub family: &'static str,
    pub size_px: f32,
    pub weight: u16,
    /// Logical pixels.
    pub line_height: f32,
    /// Tracking in logical pixels.
    pub letter_spacing_px: f32,
    pub features: &'static [&'static str],
    pub caps: Caps,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShadowLayer {
    pub x: f32,
    pub y: f32,
    pub blur: f32,
    pub spread: f32,
    pub color: Color,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ThemeTokens {
    pub colors: Colors,
    pub type_ramp: TypeRamp,
    pub radii: Radii,
    pub spacing: Spacing,
    pub popover_shadow: &'static [ShadowLayer],
    pub motion: Motion,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThemeMode {
    Dark,
    Light,
}

#[derive(Debug, Clone, Copy)]
pub struct Theme {
    pub mode: ThemeMode,
    pub tokens: &'static ThemeTokens,
}

impl Global for Theme {}

impl Theme {
    pub fn new(mode: ThemeMode) -> Self {
        let tokens = match mode {
            ThemeMode::Dark => &DARK,
            ThemeMode::Light => &LIGHT,
        };
        Self { mode, tokens }
    }

    /// Installs (or replaces) the theme global.
    pub fn install(cx: &mut App, mode: ThemeMode) {
        cx.set_global(Self::new(mode));
    }

    pub fn get(cx: &App) -> &Theme {
        cx.global::<Theme>()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eleven_px_floor() {
        for s in DARK.type_ramp.all() {
            assert!(s.size_px >= 11.0, "{}", s.family);
        }
    }

    #[test]
    fn label_sc_has_tracking_from_design_md() {
        // DESIGN.md: label-sc is 13px with letterSpacing 0.075em.
        let s = DARK.type_ramp.label_sc;
        assert!(s.letter_spacing_px > 0.0);
        assert!((s.letter_spacing_px - 13.0 * 0.075).abs() < 1e-3);
        assert_eq!(DARK.type_ramp.body.letter_spacing_px, 0.0);
    }

    #[test]
    fn radii_on_scale() {
        assert_eq!(
            [DARK.radii.none, DARK.radii.xs, DARK.radii.sm, DARK.radii.md],
            [0.0, 3.0, 4.0, 6.0]
        );
    }

    #[test]
    fn cp_fill_is_saturated_red_with_white_text_in_both_themes() {
        assert_eq!(DARK.colors.cp_fill.0, 0xCE2C31FF);
        assert_eq!(LIGHT.colors.cp_fill.0, 0xCE2C31FF);
        assert_eq!(DARK.colors.on_cp_fill.0, 0xFFFFFFFF);
    }

    #[test]
    fn color_to_hsla_round_trips_alpha_and_white() {
        let w = Color(0xFFFFFF80).to_hsla();
        assert!((w.l - 1.0).abs() < 1e-4);
        assert!((w.a - 128.0 / 255.0).abs() < 1e-4);
    }

    #[test]
    fn theme_global_is_installed_and_readable() {
        let cx = gpui::TestAppContext::single();
        cx.update(|cx| {
            Theme::install(cx, ThemeMode::Light);
            assert_eq!(Theme::get(cx).mode, ThemeMode::Light);
            assert_eq!(Theme::get(cx).tokens.colors.ground, LIGHT.colors.ground);
        });
    }
}
