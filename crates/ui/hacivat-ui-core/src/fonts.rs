//! Bundled fonts (Source Sans 3, Fragment Mono; both OFL-1.1) and the mapping from design
//! tokens to GPUI `Font`s.

use std::borrow::Cow;

use gpui::{
    App, Div, Font, FontFeatures, FontWeight, Hsla, ParentElement as _, Styled as _, div, font, px,
};

use crate::theme::TypeStyle;

pub const UI_FAMILY: &str = "Source Sans 3";
pub const MONO_FAMILY: &str = "Fragment Mono";

const FONT_FILES: [&[u8]; 6] = [
    include_bytes!("../assets/fonts/SourceSans3-Regular.ttf"),
    include_bytes!("../assets/fonts/SourceSans3-Medium.ttf"),
    include_bytes!("../assets/fonts/SourceSans3-Semibold.ttf"),
    include_bytes!("../assets/fonts/SourceSans3-Bold.ttf"),
    include_bytes!("../assets/fonts/FragmentMono-Regular.ttf"),
    include_bytes!("../assets/fonts/FragmentMono-Italic.ttf"),
];

/// Registers the bundled fonts with GPUI's text system.
pub fn register_fonts(cx: &mut App) -> anyhow::Result<()> {
    let fonts: Vec<Cow<'static, [u8]>> = FONT_FILES.iter().map(|b| Cow::Borrowed(*b)).collect();
    cx.text_system().add_fonts(fonts)
}

/// Maps a type token to a GPUI `Font`. Token families are CSS stacks
/// (`"Source Sans 3, system-ui, sans-serif"`); the first entry is the bundled family.
pub fn font_for(style: &TypeStyle) -> Font {
    let family = style
        .family
        .split(',')
        .next()
        .unwrap_or(style.family)
        .trim();
    let mut f = font(family.to_string());
    f.weight = FontWeight(style.weight as f32);
    f.features = FontFeatures(std::sync::Arc::new(
        style
            .features
            .iter()
            .map(|tag| (tag.to_string(), 1))
            .collect(),
    ));
    f
}

/// Synthesized small caps: letters render at this fraction of the style's size (Chrome's
/// `font-variant-caps: all-small-caps` synthesis ratio, which the approved mock uses).
pub const SYNTH_SCALE: f32 = 0.7;
/// Share of the token's tracking that matches Chrome's tracked synthesized small caps
/// (empirical, measured against the mock capture: EXPLORER, HACIVAT).
pub const TRACKING_FIT: f32 = 0.5;

/// Per-glyph `(char, size_px)` for synthesized small caps. `text` must already be cased by the
/// caller (locale-aware, e.g. Turkish `i` -> `İ`); this never changes case. Letters shrink to
/// [`SYNTH_SCALE`]; digits, spaces and punctuation keep `size_px`.
pub fn small_caps_glyphs(text: &str, size_px: f32) -> Vec<(char, f32)> {
    text.chars()
        .map(|ch| {
            let size = if ch.is_alphabetic() { size_px * SYNTH_SCALE } else { size_px };
            (ch, size)
        })
        .collect()
}

/// Gap after every glyph of a synthesized small-caps run, in logical px.
pub fn small_caps_tracking_px(style: &TypeStyle) -> f32 {
    style.letter_spacing_px * TRACKING_FIT
}

/// A synthesized small-caps label (DESIGN.md `fontVariantCaps: synthesized`, PO decision M0):
/// pre-cased `text` in `style`'s face, letters at [`SYNTH_SCALE`] x `style.size_px`, each glyph
/// followed by [`small_caps_tracking_px`] (GPUI has no `letter-spacing`, so every glyph is its
/// own flex item; Chrome also adds the spacing after each glyph). Never calls `to_uppercase`.
pub fn small_caps(style: &TypeStyle, color: Hsla, text: &str) -> Div {
    let font = font_for(style);
    let gap = small_caps_tracking_px(style);
    let mut row = div().flex().flex_row().items_baseline().flex_none();
    for (ch, size) in small_caps_glyphs(text, style.size_px) {
        row = row.child(
            div()
                .font(font.clone())
                .text_size(px(size))
                .line_height(px(style.line_height))
                .text_color(color)
                .whitespace_nowrap()
                .mr(px(gap))
                .child(ch.to_string()),
        );
    }
    row
}

/// Whether `features` enables `tag`.
pub fn has_feature(features: &FontFeatures, tag: &str) -> bool {
    features
        .tag_value_list()
        .iter()
        .any(|(t, v)| t == tag && *v != 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::{Caps, DARK};

    #[test]
    fn label_sc_requests_small_caps() {
        // PO decision: small caps are synthesized like the mock, never the font's smcp/c2sc.
        for style in [
            DARK.type_ramp.label_sc,
            DARK.type_ramp.panel_title,
            DARK.type_ramp.capsule_env,
        ] {
            assert_eq!(style.caps, Caps::SynthesizedSmallCaps);
            let f = font_for(&style);
            assert!(!has_feature(&f.features, "smcp") && !has_feature(&f.features, "c2sc"));
        }
        assert_eq!(DARK.type_ramp.body.caps, Caps::Normal);
    }

    #[test]
    fn synthesized_small_caps_shrink_letters_only() {
        let g = small_caps_glyphs("NULL 2·x", 13.0);
        let sc = 13.0 * SYNTH_SCALE;
        assert_eq!(
            g,
            vec![('N', sc), ('U', sc), ('L', sc), ('L', sc), (' ', 13.0), ('2', 13.0), ('·', 13.0), ('x', sc)]
        );
    }

    #[test]
    fn small_caps_take_pre_cased_text_and_never_case_it() {
        // Turkish dotted capital I arrives pre-cased and is kept; lower-case input is not
        // upper-cased here (casing is locale-aware and happens upstream).
        let chars = |s: &str| small_caps_glyphs(s, 10.0).into_iter().map(|g| g.0).collect::<String>();
        assert_eq!(chars("GEZGİN"), "GEZGİN");
        assert_eq!(chars("gezgin"), "gezgin");
    }

    #[test]
    fn small_caps_tracking_is_the_fitted_share_of_the_token() {
        let s = DARK.type_ramp.label_sc;
        assert!((small_caps_tracking_px(&s) - s.letter_spacing_px * TRACKING_FIT).abs() < 1e-6);
        assert_eq!(TRACKING_FIT, 0.5);
        assert_eq!(SYNTH_SCALE, 0.7);
    }

    #[test]
    fn body_requests_tabular_figures() {
        let f = font_for(&DARK.type_ramp.body);
        assert!(has_feature(&f.features, "tnum"));
        assert!(!has_feature(&f.features, "smcp"));
    }

    #[test]
    fn families_resolve_to_bundled_names() {
        assert_eq!(font_for(&DARK.type_ramp.body).family.as_ref(), UI_FAMILY);
        assert_eq!(font_for(&DARK.type_ramp.code).family.as_ref(), MONO_FAMILY);
    }

    #[test]
    fn weight_is_carried() {
        assert_eq!(font_for(&DARK.type_ramp.body).weight, FontWeight(400.0));
    }
}
