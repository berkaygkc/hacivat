//! Bundled fonts (Source Sans 3, Fragment Mono; both OFL-1.1) and the mapping from design
//! tokens to GPUI `Font`s.

use std::borrow::Cow;

use gpui::{App, Font, FontFeatures, FontWeight, font};

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
    use crate::theme::DARK;

    #[test]
    fn label_sc_requests_small_caps() {
        let f = font_for(&DARK.type_ramp.label_sc);
        assert!(has_feature(&f.features, "smcp") && has_feature(&f.features, "c2sc"));
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
