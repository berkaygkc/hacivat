//! The content-keyed shape cache (UI-EDIT-09; Task 19): an LRU of at most 4,096 shaped visual
//! lines or 32 MB, keyed `(text_hash, runs_hash, font_size_bits, column_window)`.
//!
//! - **Content, not position.** The key is the hash of the text that was shaped, never a line
//!   index or a version, so an edit reshapes only the lines whose text changed (one per typed
//!   character), scrolling back finds the lines it shaped on the way down, and a line moved by
//!   an edit above it is still a hit.
//! - **Colours are in the key.** At the GPUI pin a `ShapedLine` keeps its decoration runs
//!   (colours included) private, so a cached shape cannot be repainted in another colour; a
//!   theme switch therefore reshapes the visible lines (GPUI's own frame cache still holds the
//!   font layout, so that is cheap). `runs_hash` covers each run's length, font, colour,
//!   background and underline.
//! - **Long lines** (more than [`LONG_LINE_COLUMNS`] columns or [`LONG_LINE_BYTES`] bytes)
//!   shape a [`MAX_SHAPED_COLUMNS`]-column window around the viewport whose start steps by
//!   [`WINDOW_STEP`] ([`column_window`], ported from the M0 spike with its tests); the window is
//!   part of the key.
//! - **Gutter digits** are shaped once per font size and ink ([`paint_line_number`], the digit
//!   cache Task 28's gutter paints line numbers with).

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::ops::Range;

use hacivat_ui_core::gpui_surface::{
    App, Global, Hsla, Pixels, ShapedLine, SharedString, TextAlign, TextRun, Window, point, px,
};
use lru::LruCache;
use twox_hash::XxHash3_64;

use crate::theme::EditorTheme;

/// Shaped visual lines kept.
pub(crate) const MAX_LINES: usize = 4_096;
/// Bytes (estimated) kept.
pub(crate) const MAX_BYTES: usize = 32 << 20;
/// A line longer than this many columns is shaped through a window.
pub(crate) const LONG_LINE_COLUMNS: usize = 10_000;
/// A line longer than this many bytes is shaped through a window.
pub(crate) const LONG_LINE_BYTES: usize = 64 << 10;
/// The most columns shaped for one long line.
pub(crate) const MAX_SHAPED_COLUMNS: usize = 512;
/// A long line's window start moves in steps of at most this many columns, so scrolling
/// sideways reuses the shaped window until the viewport leaves it.
pub(crate) const WINDOW_STEP: usize = 128;

/// The column range of a long line to shape: at most [`MAX_SHAPED_COLUMNS`] columns, covering
/// `scroll_col..scroll_col + visible_cols` whenever that fits in the cap (the M0 spike's rule).
pub(crate) fn column_window(
    scroll_col: usize,
    visible_cols: usize,
    line_len: usize,
) -> Range<usize> {
    let step = MAX_SHAPED_COLUMNS
        .saturating_sub(visible_cols)
        .min(WINDOW_STEP);
    let start = scroll_col
        .checked_div(step)
        .map_or(scroll_col, |q| q * step);
    let start = start.min(line_len);
    start..(start + MAX_SHAPED_COLUMNS).min(line_len)
}

/// What a shaped line is cached under.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct ShapeKey {
    pub text_hash: u64,
    pub runs_hash: u64,
    pub font_size_bits: u32,
    /// The long line's column window; `(0, 0)` for a line shaped whole.
    pub column_window: (u32, u32),
}

impl ShapeKey {
    /// The key of `text` shaped with `runs` at `font_size` over `column_window`.
    pub fn of(text: &str, runs: &[TextRun], font_size: Pixels, column_window: (u32, u32)) -> Self {
        ShapeKey {
            text_hash: XxHash3_64::oneshot(text.as_bytes()),
            runs_hash: runs_hash(runs),
            font_size_bits: f32::from(font_size).to_bits(),
            column_window,
        }
    }
}

fn hash_hsla(h: &mut impl Hasher, c: Hsla) {
    for v in [c.h, c.s, c.l, c.a] {
        h.write_u32(v.to_bits());
    }
}

/// The hash of every run's length, font, colour, background and underline.
fn runs_hash(runs: &[TextRun]) -> u64 {
    let mut h = XxHash3_64::with_seed(0);
    for r in runs {
        h.write_usize(r.len);
        r.font.hash(&mut h);
        hash_hsla(&mut h, r.color);
        match r.background_color {
            Some(c) => {
                h.write_u8(1);
                hash_hsla(&mut h, c);
            }
            None => h.write_u8(0),
        }
        match &r.underline {
            Some(u) => {
                h.write_u8(1);
                h.write_u32(f32::from(u.thickness).to_bits());
                h.write_u8(u8::from(u.wavy));
                if let Some(c) = u.color {
                    hash_hsla(&mut h, c);
                }
            }
            None => h.write_u8(0),
        }
    }
    h.finish()
}

/// What the cache did (the counter tests and the B3 bench read it).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LayoutStats {
    /// `shape_line` calls (cache misses) since the editor opened.
    pub shapes: u64,
    /// Cache hits since the editor opened.
    pub hits: u64,
    /// The longest text ever shaped for one line, in characters.
    pub longest_shaped: usize,
    /// Entries in the cache now.
    pub entries: usize,
}

struct Entry {
    shaped: ShapedLine,
    bytes: usize,
}

/// An estimate of what a shaped line holds in memory.
fn shaped_bytes(s: &ShapedLine) -> usize {
    let glyphs: usize = s
        .runs
        .iter()
        .map(|r| std::mem::size_of_val(r.glyphs.as_slice()) + 32)
        .sum();
    s.text.len() + glyphs + 128
}

/// The editor's shape cache.
pub(crate) struct LayoutCache {
    lru: LruCache<ShapeKey, Entry>,
    bytes: usize,
    stats: LayoutStats,
    /// The monospace advance, per font size (measured once).
    advance: Option<(u32, Pixels)>,
}

impl std::fmt::Debug for LayoutCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LayoutCache")
            .field("bytes", &self.bytes)
            .field("stats", &self.stats())
            .finish_non_exhaustive()
    }
}

impl Default for LayoutCache {
    fn default() -> Self {
        LayoutCache {
            lru: LruCache::unbounded(),
            bytes: 0,
            stats: LayoutStats::default(),
            advance: None,
        }
    }
}

impl LayoutCache {
    /// `text` shaped with `runs`: from the cache, or shaped now and kept.
    pub fn shape(
        &mut self,
        window: &Window,
        text: &str,
        runs: &[TextRun],
        font_size: Pixels,
        column_window: (u32, u32),
    ) -> ShapedLine {
        let key = ShapeKey::of(text, runs, font_size, column_window);
        if let Some(e) = self.lru.get(&key) {
            self.stats.hits += 1;
            return e.shaped.clone();
        }
        let shaped = window.text_system().shape_line(
            SharedString::from(text.to_string()),
            font_size,
            runs,
            None,
        );
        self.stats.shapes += 1;
        self.stats.longest_shaped = self.stats.longest_shaped.max(text.chars().count());
        let bytes = shaped_bytes(&shaped);
        self.bytes += bytes;
        if let Some((_, old)) = self.lru.push(
            key,
            Entry {
                shaped: shaped.clone(),
                bytes,
            },
        ) {
            self.bytes -= old.bytes;
        }
        while self.lru.len() > MAX_LINES || self.bytes > MAX_BYTES {
            match self.lru.pop_lru() {
                Some((_, e)) => self.bytes -= e.bytes,
                None => break,
            }
        }
        shaped
    }

    /// The code font's monospace advance (one `0`), measured once per font size.
    pub fn advance(&mut self, window: &Window, theme: &EditorTheme) -> Pixels {
        let bits = f32::from(theme.font_size).to_bits();
        if let Some((at, adv)) = self.advance
            && at == bits
        {
            return adv;
        }
        let probe = "0".repeat(64);
        let run = TextRun {
            len: probe.len(),
            font: theme.font.clone(),
            color: theme.chip_text,
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let width = window
            .text_system()
            .shape_line(probe.into(), theme.font_size, &[run], None)
            .width();
        let adv = px((f32::from(width) / 64.0).max(1.0));
        self.advance = Some((bits, adv));
        adv
    }

    pub fn stats(&self) -> LayoutStats {
        LayoutStats {
            entries: self.lru.len(),
            ..self.stats
        }
    }
}

/// The ten digits, shaped once per font size and ink (app-wide: shapes belong to the text
/// system, which the app owns).
#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "Task 28's gutter paints the line numbers through it"
    )
)]
#[derive(Default)]
pub(crate) struct DigitCache {
    digits: HashMap<(u32, [u32; 4]), [ShapedLine; 10]>,
    /// `shape_line` calls made for digits.
    pub shapes: u64,
}

impl Global for DigitCache {}

/// Paints the line number `n` right-aligned at `right` on the row whose top is `top`, in
/// `color`, from the digit cache; returns its width. For Task 28's gutter.
#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "Task 28's gutter paints the line numbers through it"
    )
)]
pub(crate) fn paint_line_number(
    window: &mut Window,
    cx: &mut App,
    n: u32,
    right: Pixels,
    top: Pixels,
    color: Hsla,
) -> Pixels {
    let theme = EditorTheme::get(cx);
    let key = (
        f32::from(theme.font_size).to_bits(),
        [color.h, color.s, color.l, color.a].map(f32::to_bits),
    );
    let digits = {
        let cache = cx.default_global::<DigitCache>();
        match cache.digits.get(&key) {
            Some(d) => d.clone(),
            None => {
                let d: [ShapedLine; 10] = std::array::from_fn(|i| {
                    let text = char::from(b'0' + i as u8).to_string();
                    let run = TextRun {
                        len: text.len(),
                        font: theme.font.clone(),
                        color,
                        background_color: None,
                        underline: None,
                        strikethrough: None,
                    };
                    window
                        .text_system()
                        .shape_line(text.into(), theme.font_size, &[run], None)
                });
                cache.shapes += 10;
                cache.digits.insert(key, d.clone());
                d
            }
        }
    };
    let mut x = right;
    let mut rest = n;
    loop {
        let d = &digits[(rest % 10) as usize];
        x -= d.width();
        let _ = d.paint(
            point(x, top),
            theme.line_height,
            TextAlign::Left,
            None,
            window,
            cx,
        );
        rest /= 10;
        if rest == 0 {
            break;
        }
    }
    right - x
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn column_window_never_shapes_more_than_512_and_covers_viewport() {
        let w = column_window(50_000, 200, 100_000);
        assert!(w.len() <= MAX_SHAPED_COLUMNS);
        assert!(w.start <= 50_000 && w.end >= 50_200, "{w:?}");
        assert_eq!(column_window(0, 150, 40), 0..40);
        assert_eq!(column_window(300, 150, 40), 40..40);
    }

    #[test]
    fn column_window_is_stable_while_the_viewport_stays_inside() {
        assert_eq!(
            column_window(50_050, 200, 100_000),
            column_window(50_100, 200, 100_000)
        );
        let wide = column_window(1_000, 700, 100_000);
        assert_eq!(wide, 1_000..1_512);
    }

    #[test]
    fn the_key_changes_with_text_colour_size_and_window_only() {
        use hacivat_ui_core::gpui_surface::hsla;
        let theme = EditorTheme::of(&hacivat_ui_core::theme::Theme::new(
            hacivat_ui_core::theme::ThemeMode::Dark,
        ));
        let run = |color| TextRun {
            len: 6,
            font: theme.font.clone(),
            color,
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let blue = hsla(0.6, 0.5, 0.5, 1.0);
        let base = ShapeKey::of("SELECT", &[run(blue)], px(12.5), (0, 0));
        assert_eq!(base, ShapeKey::of("SELECT", &[run(blue)], px(12.5), (0, 0)));
        let others = [
            ShapeKey::of("SELECt", &[run(blue)], px(12.5), (0, 0)),
            ShapeKey::of("SELECT", &[run(hsla(0.6, 0.5, 0.6, 1.0))], px(12.5), (0, 0)),
            ShapeKey::of("SELECT", &[run(blue)], px(13.0), (0, 0)),
            ShapeKey::of("SELECT", &[run(blue)], px(12.5), (128, 640)),
        ];
        for k in others {
            assert_ne!(base, k);
        }
    }

    /// Gutter digits are shaped once per font size and ink, however many numbers are painted.
    #[gpui::test]
    fn digits_are_shaped_once_per_size_and_ink(cx: &mut gpui::TestAppContext) {
        use gpui::{IntoElement as _, Styled as _, hsla, point, size};
        let (_, vcx) = cx.add_window_view(|_, _| Blank);
        let widths = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        for (n, color) in [
            (123, hsla(0.0, 0.0, 0.8, 1.0)),
            (4_567, hsla(0.0, 0.0, 0.8, 1.0)),
            (8, hsla(0.0, 0.0, 0.5, 1.0)),
        ] {
            let out = widths.clone();
            vcx.draw(
                point(px(0.0), px(0.0)),
                size(px(200.0), px(40.0)),
                move |_, _| {
                    gpui::canvas(
                        |_, _, _| {},
                        move |b, _, window, cx| {
                            let w = paint_line_number(window, cx, n, b.right(), b.top(), color);
                            out.borrow_mut().push(f32::from(w));
                        },
                    )
                    .size_full()
                    .into_any_element()
                },
            );
        }
        let shapes = vcx.read(|cx| cx.global::<DigitCache>().shapes);
        assert_eq!(shapes, 20, "ten digits per ink, shaped once");
        let w = widths.borrow();
        assert!(
            (w[1] / w[0] - 4.0 / 3.0).abs() < 1e-3,
            "tabular digits: {w:?}"
        );
    }

    struct Blank;

    impl gpui::Render for Blank {
        fn render(
            &mut self,
            _: &mut Window,
            _: &mut gpui::Context<Self>,
        ) -> impl gpui::IntoElement {
            gpui::div()
        }
    }
}
