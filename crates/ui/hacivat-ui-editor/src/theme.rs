//! The editor theme (Task 19): highlight classes to `syn_*` tokens, the caret and selection
//! inks, the code font. Read once per frame from the `Theme` global (dark when none is
//! installed), so a theme switch repaints with the new inks; the colours are part of the shape
//! cache key (`layout_cache`), because a shaped line cannot be recoloured at the GPUI pin.
//!
//! Inks follow `design/m3a-query/NOTES.md` surface 1 and the mock: the code area is on the
//! `panel` tone (the gutter too, with no fill of its own), code is Fragment Mono 12.5/18 with
//! ligatures off, comments are italic, punctuation and operators take `syn_operator`, a MySQL
//! executable comment is comment ink, not italic, on the `seam_soft` inlay well. The caret is
//! `blue`; the selection is `blue_tint_strong` when focused and the neutral `seam_strong`
//! otherwise (NOTES open question 10). A control character is a chip: Fragment Mono 11 `text_2`
//! on `seam_soft`.
//!
//! The executable comment's inlay well is not a run background (GPUI paints those only through
//! `ShapedLine::paint_background`, and a token's ink must not erase the well): a class with
//! `well` marks its bytes, the element keeps those marks apart from the token classes, and the
//! paint pass fills them before the selection (review round 1).
//!
//! [`EditorTheme::get`] allocates nothing: the two built-in themes are built once and shared.

use std::sync::{Arc, OnceLock};

use hacivat_ui_core::fonts::font_for;
use hacivat_ui_core::gpui_surface::{App, Font, FontFeatures, FontStyle, Hsla, Pixels, px};
use hacivat_ui_core::theme::{DARK, LIGHT, Theme, ThemeMode};

use crate::decor::HighlightClass;

/// How one highlight class is drawn.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct ClassInk {
    pub color: Hsla,
    pub italic: bool,
    /// Its bytes sit on the inlay well ([`EditorTheme::well`]), whatever ink the tokens inside
    /// them take (the executable comment).
    pub well: bool,
    /// A 1 px underline (an error token's subtle mark).
    pub underline: Option<Hsla>,
}

/// The editor's inks, metrics and fonts for one frame.
#[derive(Debug, Clone)]
pub(crate) struct EditorTheme {
    /// The code font (Fragment Mono, ligatures off).
    pub font: Font,
    /// The same, italic (comments).
    pub italic: Font,
    pub font_size: Pixels,
    pub line_height: Pixels,
    /// The control chip's label size.
    pub chip_font_size: Pixels,
    /// The code area and the gutter.
    pub background: Hsla,
    pub selection: Hsla,
    pub selection_unfocused: Hsla,
    pub caret: Hsla,
    /// The IME composition's underline.
    pub ime_underline: Hsla,
    pub chip_background: Hsla,
    pub chip_text: Hsla,
    /// The horizontal scroll thumb.
    pub thumb: Hsla,
    /// The inlay well under an executable comment (`seam_soft`).
    pub well: Hsla,
    /// One ink per [`HighlightClass`], indexed by `class as usize`.
    pub classes: [ClassInk; 16],
}

impl EditorTheme {
    /// The theme of `cx` (the `Theme` global; dark when none is installed, as in a bare test).
    /// The built-in dark and light themes are built once and shared, so this allocates nothing
    /// (it runs per frame and per painted line number); any other token set is built afresh.
    pub fn get(cx: &App) -> Arc<EditorTheme> {
        static DARK_THEME: OnceLock<Arc<EditorTheme>> = OnceLock::new();
        static LIGHT_THEME: OnceLock<Arc<EditorTheme>> = OnceLock::new();
        let theme = cx
            .try_global::<Theme>()
            .copied()
            .unwrap_or_else(|| Theme::new(ThemeMode::Dark));
        let shared = match theme.mode {
            ThemeMode::Dark if *theme.tokens == DARK => &DARK_THEME,
            ThemeMode::Light if *theme.tokens == LIGHT => &LIGHT_THEME,
            _ => return Arc::new(Self::of(&theme)),
        };
        shared.get_or_init(|| Arc::new(Self::of(&theme))).clone()
    }

    /// The editor's inks for `theme`.
    pub fn of(theme: &Theme) -> Self {
        let t = theme.tokens;
        let c = &t.colors;
        let code = &t.type_ramp.code;
        let mut font = font_for(code);
        let mut features: Vec<(String, u32)> = font.features.0.as_ref().clone();
        // The product draws code without ligatures (NOTES "Verification": `>=` is two glyphs).
        features.extend([("liga".to_string(), 0), ("calt".to_string(), 0)]);
        font.features = FontFeatures(Arc::new(features));
        let mut italic = font.clone();
        italic.style = FontStyle::Italic;
        let ink = |color: hacivat_ui_core::theme::Color| ClassInk {
            color: color.to_hsla(),
            italic: false,
            well: false,
            underline: None,
        };
        let classes = HighlightClass::ALL.map(|class| match class {
            HighlightClass::Plain
            | HighlightClass::Ident
            | HighlightClass::QuotedIdent
            | HighlightClass::Param => ink(c.syn_identifier),
            HighlightClass::Keyword | HighlightClass::MetaCommand => ink(c.syn_keyword),
            HighlightClass::String | HighlightClass::DollarTag => ink(c.syn_string),
            HighlightClass::Number => ink(c.syn_number),
            HighlightClass::Comment | HighlightClass::Hint => ClassInk {
                italic: true,
                ..ink(c.syn_comment)
            },
            HighlightClass::Operator | HighlightClass::Punct => ink(c.syn_operator),
            HighlightClass::ExecComment => ClassInk {
                well: true,
                ..ink(c.syn_comment)
            },
            HighlightClass::ErrorToken => ClassInk {
                underline: Some(c.text_3.to_hsla()),
                ..ink(c.syn_identifier)
            },
            HighlightClass::Function => ink(c.syn_function),
        });
        EditorTheme {
            font,
            italic,
            font_size: px(code.size_px),
            line_height: px(t.spacing.code_line_height),
            chip_font_size: px(11.0),
            background: c.panel.to_hsla(),
            selection: c.blue_tint_strong.to_hsla(),
            selection_unfocused: c.seam_strong.to_hsla(),
            caret: c.blue.to_hsla(),
            ime_underline: c.text.to_hsla(),
            chip_background: c.seam_soft.to_hsla(),
            chip_text: c.text_2.to_hsla(),
            thumb: c.seam_strong.to_hsla(),
            well: c.seam_soft.to_hsla(),
            classes,
        }
    }

    /// The ink of `class`.
    pub fn ink(&self, class: HighlightClass) -> &ClassInk {
        &self.classes[class as usize]
    }

    /// The font a run of `ink` uses.
    pub fn font_for(&self, ink: &ClassInk) -> &Font {
        if ink.italic { &self.italic } else { &self.font }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hacivat_ui_core::theme::{DARK, LIGHT};

    #[test]
    fn classes_take_the_syntax_tokens_and_comments_are_italic() {
        for (mode, tokens) in [(ThemeMode::Dark, &DARK), (ThemeMode::Light, &LIGHT)] {
            let t = EditorTheme::of(&Theme::new(mode));
            let c = &tokens.colors;
            assert_eq!(
                t.ink(HighlightClass::Keyword).color,
                c.syn_keyword.to_hsla()
            );
            assert_eq!(t.ink(HighlightClass::String).color, c.syn_string.to_hsla());
            assert_eq!(t.ink(HighlightClass::Punct).color, c.syn_operator.to_hsla());
            assert!(t.ink(HighlightClass::Comment).italic);
            assert!(!t.ink(HighlightClass::ExecComment).italic);
            assert!(t.ink(HighlightClass::ExecComment).well);
            assert_eq!(t.well, c.seam_soft.to_hsla());
            assert_eq!(t.background, c.panel.to_hsla());
            assert_eq!(t.selection, c.blue_tint_strong.to_hsla());
            assert_eq!(t.selection_unfocused, c.seam_strong.to_hsla());
            assert_eq!(t.caret, c.blue.to_hsla());
            // Ligatures off: `>=` stays two glyphs.
            assert!(t.font.features.0.contains(&("liga".to_string(), 0)));
            assert_eq!(t.italic.style, FontStyle::Italic);
        }
    }

    /// No class is drawn in a red: Production Red is reserved (DESIGN.md).
    #[test]
    fn no_class_ink_is_red() {
        for (mode, tokens) in [(ThemeMode::Dark, &DARK), (ThemeMode::Light, &LIGHT)] {
            let t = EditorTheme::of(&Theme::new(mode));
            let c = &tokens.colors;
            let reds = [c.red, c.red_line, c.red_tint, c.cp_fill].map(|r| r.to_hsla());
            for ink in &t.classes {
                assert!(!reds.contains(&ink.color), "{mode:?}");
                assert!(ink.underline.is_none_or(|u| !reds.contains(&u)));
            }
            assert!(!reds.contains(&t.well));
        }
    }

    /// The built-in themes are built once: `get` hands out the same one (no allocation per
    /// call), and a switch hands out the other.
    #[gpui::test]
    fn get_shares_the_built_in_themes(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            let a = EditorTheme::get(cx);
            assert!(Arc::ptr_eq(&a, &EditorTheme::get(cx)));
            Theme::install(cx, ThemeMode::Light);
            let b = EditorTheme::get(cx);
            assert!(!Arc::ptr_eq(&a, &b));
            assert_eq!(b.background, LIGHT.colors.panel.to_hsla());
            assert!(Arc::ptr_eq(&b, &EditorTheme::get(cx)));
        });
    }
}
