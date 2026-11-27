//! Design tokens and the `Theme` global. Every UI element reads colours, type and metrics
//! through `Theme`; `generated.rs` is produced by `cargo xtask tokens`.

use gpui::{App, Global, Hsla, Rgba};

mod generated;
pub mod surfaces;

pub use generated::{Colors, DARK, LIGHT, Motion, Radii, SHARED, SharedColors, Spacing, TypeRamp};
pub use surfaces::{focus_outline, popover_surface};

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
    fn radii_on_scale_in_both_themes() {
        for t in [&DARK, &LIGHT] {
            assert_eq!(
                [t.radii.none, t.radii.xs, t.radii.sm, t.radii.md],
                [0.0, 3.0, 4.0, 6.0]
            );
        }
    }

    #[test]
    fn cp_fill_is_saturated_red_with_white_text_in_both_themes() {
        assert_eq!(DARK.colors.cp_fill.0, 0xCE2C31FF);
        assert_eq!(LIGHT.colors.cp_fill.0, 0xCE2C31FF);
        assert_eq!(DARK.colors.on_cp_fill.0, 0xFFFFFFFF);
    }

    // ## demofix-1 (PO demo F5)
    /// sRGB channel `c` (0–255) to linear light.
    fn linear(c: u32) -> f64 {
        let c = f64::from(c) / 255.0;
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    }

    /// `fg` over `bg` (its alpha), blended in sRGB space as GPUI's framebuffer and the
    /// design mocks blend, then as linear RGB.
    fn composite(fg: Color, bg: Color) -> [f64; 3] {
        let [r, g, b, a] = fg.0.to_be_bytes();
        let [br, bgc, bb, _] = bg.0.to_be_bytes();
        let a = f64::from(a) / 255.0;
        let mix = |f: u8, b: u8| {
            let srgb = f64::from(f) * a + f64::from(b) * (1.0 - a);
            linear(srgb.round() as u32)
        };
        [mix(r, br), mix(g, bgc), mix(b, bb)]
    }

    fn luminance(rgb: [f64; 3]) -> f64 {
        0.2126 * rgb[0] + 0.7152 * rgb[1] + 0.0722 * rgb[2]
    }

    fn contrast(a: [f64; 3], b: [f64; 3]) -> f64 {
        let (x, y) = (luminance(a), luminance(b));
        (x.max(y) + 0.05) / (x.min(y) + 0.05)
    }

    /// OKLab, from linear RGB.
    fn oklab(rgb: [f64; 3]) -> [f64; 3] {
        let [r, g, b] = rgb;
        let l = (0.412_221_470_8 * r + 0.536_332_536_3 * g + 0.051_445_992_9 * b).cbrt();
        let m = (0.211_903_498_2 * r + 0.680_699_545_1 * g + 0.107_396_956_6 * b).cbrt();
        let s = (0.088_302_461_9 * r + 0.281_718_837_6 * g + 0.629_978_700_5 * b).cbrt();
        [
            0.210_454_255_3 * l + 0.793_617_785 * m - 0.004_072_046_8 * s,
            1.977_998_495_1 * l - 2.428_592_205 * m + 0.450_593_709_9 * s,
            0.025_904_037_1 * l + 0.782_771_766_2 * m - 0.808_675_766 * s,
        ]
    }

    fn distance(a: Color, b: Color, bg: Color) -> f64 {
        let (x, y) = (oklab(composite(a, bg)), oklab(composite(b, bg)));
        ((x[0] - y[0]).powi(2) + (x[1] - y[1]).powi(2) + (x[2] - y[2]).powi(2)).sqrt()
    }

    /// The git letters (M, N, D) are each readable (WCAG AA, 4.5:1, at 11 px) on every
    /// surface a tree row has (side, hover, the selection tint over side), and visibly apart
    /// from one another and from the trust and state hues: production red, the capsule fill,
    /// pending amber, direct-path green.
    #[test]
    fn git_badge_inks_meet_aa_and_stay_apart() {
        for t in [&DARK, &LIGHT] {
            let c = &t.colors;
            let side = c.side;
            let selected = composite(c.blue_tint, side);
            let inks = [c.git_modified, c.git_new, c.git_deleted];
            for ink in inks {
                let fg = composite(ink, side);
                for (bg, name) in [
                    (composite(side, side), "side"),
                    (composite(c.hover, side), "hover"),
                    (selected, "selection"),
                ] {
                    let ratio = contrast(fg, bg);
                    assert!(ratio >= 4.5, "{ink:?} on {name}: {ratio:.2}");
                }
                for (other, name) in [
                    (c.red, "red"),
                    (c.cp_fill, "cp-fill"),
                    (c.amber, "amber"),
                    (c.green, "green"),
                ] {
                    let d = distance(ink, other, side);
                    assert!(d >= 0.05, "{ink:?} too close to {name}: {d:.3}");
                }
            }
            for (i, a) in inks.iter().enumerate() {
                for b in &inks[i + 1..] {
                    let d = distance(*a, *b, side);
                    assert!(d >= 0.1, "{a:?} and {b:?} too close: {d:.3}");
                }
            }
        }
    }
    // ## demofix-1

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
