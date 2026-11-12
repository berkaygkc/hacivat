//! Small building blocks shared by every region. All colours come from `Theme` tokens.

use std::cell::Cell;

use hacivat_ui_core::fonts::font_for;
use hacivat_ui_core::gpui_surface::*;
use hacivat_ui_core::theme::{Color, Colors, ShadowLayer, Theme, ThemeMode, TypeRamp, TypeStyle};

use super::data::{Env, Lang};

thread_local! {
    static CP_FILL_QUADS: Cell<u32> = const { Cell::new(0) };
}

/// Per-render context: tokens plus the language.
#[derive(Clone, Copy)]
pub struct Ui {
    pub c: Colors,
    pub r: TypeRamp,
    pub mode: ThemeMode,
    pub lang: Lang,
    pub shadow: &'static [ShadowLayer],
}

/// Token colour scaled by an opacity factor (CSS `opacity` on a flat fill).
pub fn alpha(c: Color, k: f32) -> Hsla {
    let mut h = c.to_hsla();
    h.a *= k;
    h
}

impl Ui {
    pub fn new(cx: &App, lang: Lang) -> Self {
        let th = Theme::get(cx);
        Self { c: th.tokens.colors, r: th.tokens.type_ramp, mode: th.mode, lang, shadow: th.tokens.popover_shadow }
    }

    pub fn light(&self) -> bool {
        self.mode == ThemeMode::Light
    }

    pub fn s(&self, key: &str) -> &'static str {
        super::data::t(self.lang, key)
    }

    pub fn nf(&self, v: f64, dp: usize) -> String {
        super::data::nf(self.lang, v, dp)
    }

    /// Source Sans 3 at `size` and `weight`.
    pub fn sans(&self, size: f32, weight: u16) -> TypeStyle {
        TypeStyle { size_px: size, weight, line_height: size * 1.35, letter_spacing_px: 0.0, ..self.r.body }
    }

    /// Fragment Mono at `size`.
    pub fn mono(&self, size: f32) -> TypeStyle {
        TypeStyle { size_px: size, line_height: size * 1.35, ..self.r.code }
    }

    /// A text run in `style` and `color`; the caller adds the string as a child.
    pub fn run(&self, style: TypeStyle, color: Hsla) -> Div {
        div()
            .font(font_for(&style))
            .text_size(px(style.size_px))
            .line_height(px(style.line_height))
            .text_color(color)
            .whitespace_nowrap()
            .flex_none()
    }

    pub fn txt(&self, style: TypeStyle, color: Color, s: impl Into<SharedString>) -> Div {
        self.run(style, color.to_hsla()).child(s.into())
    }

    /// Small-caps label with CSS-style tracking. The reference renders `font-variant-caps:
    /// all-small-caps` with Chrome's synthesis (the Google Fonts subset carries no `smcp`):
    /// letters become capitals at 0.7 x the size. GPUI has no `letter-spacing`, so each glyph is
    /// its own flex item carrying a trailing gap (Chrome adds the spacing after every glyph).
    pub fn sc(&self, size: f32, weight: u16, tracking_em: f32, color: Color, s: &str) -> Div {
        // Adapter workaround (PO decision: small caps stay synthesized like the mock, GPUI has no
        // letter-spacing). SYNTH_SCALE is Chrome's small-caps synthesis ratio.
        const SYNTH_SCALE: f32 = 0.7;
        // Empirical: Chrome's tracked small-caps renders ~half the nominal tracking (measured
        // against the capture: EXPLORER, HACIVAT).
        const TRACKING_FIT: f32 = 0.5;
        let style = self.sans(size, weight);
        let ls = size * tracking_em * TRACKING_FIT;
        let font = font_for(&style);
        let mut row = div().flex().flex_row().items_baseline().flex_none();
        for ch in s.chars() {
            let (txt, sz) = if ch.is_alphabetic() {
                let up = if self.lang == Lang::Tr && ch == 'i' {
                    "\u{130}".to_string()
                } else {
                    ch.to_uppercase().collect::<String>()
                };
                (up, size * SYNTH_SCALE)
            } else {
                (ch.to_string(), size)
            };
            row = row.child(
                div()
                    .font(font.clone())
                    .text_size(px(sz))
                    .line_height(px(style.line_height))
                    .text_color(color.to_hsla())
                    .whitespace_nowrap()
                    .mr(px(ls))
                    .child(txt),
            );
        }
        row
    }

    pub fn icon(&self, name: &str, size: f32, color: Hsla) -> impl IntoElement + use<> {
        let path = if name == "hv-mark" {
            "icons/hacivat/hv-mark.svg".to_string()
        } else {
            format!("icons/lucide/{name}.svg")
        };
        svg().path(path).size(px(size)).flex_none().text_color(color)
    }

    pub fn i12(&self, name: &str, color: Color) -> impl IntoElement + use<> {
        self.icon(name, 12.0, color.to_hsla())
    }

    /// 1 px vertical separator of `h` px.
    pub fn vsep(&self, h: f32) -> Div {
        div().w(px(1.0)).h(px(h)).flex_none().bg(self.c.seam.to_hsla())
    }

    pub fn kbd(&self, s: &str, color: Color) -> Div {
        self.kbd_on(s, color, self.c.seam)
    }

    pub fn kbd_on(&self, s: &str, color: Color, border: Color) -> Div {
        div()
            .flex()
            .items_center()
            .flex_none()
            .h(px(16.0))
            .px(px(4.0))
            .border_1()
            .border_color(border.to_hsla())
            .rounded(px(3.0))
            .child(self.txt(TypeStyle { line_height: 11.0, ..self.mono(11.0) }, color, s.to_string()))
    }

    /// Environment swatch: production is the only filled hue.
    pub fn swatch(&self, env: Env) -> Div {
        let base = div().size(px(7.0)).flex_none();
        match env {
            Env::Prod => base.bg(self.c.red.to_hsla()),
            Env::Stg => base.border_1().border_color(self.c.text_2.to_hsla()),
            Env::Loc => base.border_1().border_color(self.c.text_3.to_hsla()).rounded_full(),
        }
    }

    /// The connection capsule's environment cell fill. This is the only `cp_fill` quad in the
    /// window; the root asserts that in debug builds.
    pub fn cp_fill_quad(&self) -> Div {
        CP_FILL_QUADS.with(|c| c.set(c.get() + 1));
        div().bg(self.c.cp_fill.to_hsla())
    }
}

pub fn reset_cp_fill_count() {
    CP_FILL_QUADS.with(|c| c.set(0));
}

pub fn cp_fill_count() -> u32 {
    CP_FILL_QUADS.with(Cell::get)
}

/// A flat horizontal row container: `flex-direction: row; align-items: center`.
pub fn row() -> Div {
    div().flex().flex_row().items_center()
}

pub fn col() -> Div {
    div().flex().flex_col()
}

/// A 1 px line along the right edge of a `relative` parent (per-edge border colour).
pub fn edge_r(c: Hsla) -> Div {
    div().absolute().right_0().top_0().bottom_0().w(px(1.0)).bg(c)
}

/// A 1 px line along the bottom edge of a `relative` parent.
pub fn edge_b(c: Hsla) -> Div {
    div().absolute().left_0().right_0().bottom_0().h(px(1.0)).bg(c)
}
