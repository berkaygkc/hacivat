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
//! - **A hit is verified, never trusted on the hash** (review round 1): XXH3-64 is not
//!   collision-resistant, and two lines with one key would show the first line's glyphs for the
//!   second (a pasted `DROP TABLE users; -- …` drawn over `DROP TABLE trash; -- …`: the line
//!   shown is not the line that runs). A hit whose stored text or runs differ from the ones asked
//!   for is a miss, and the entry is replaced.
//! - **Long lines** (more than [`LONG_LINE_COLUMNS`] columns or [`LONG_LINE_BYTES`] bytes)
//!   shape a window around the viewport whose start steps by [`WINDOW_STEP`] ([`column_window`],
//!   ported from the M0 spike with its tests): [`MAX_SHAPED_COLUMNS`] columns, or the viewport
//!   plus one step when the viewport is wider, so no visible column is ever left unshaped. The
//!   window is part of the key.
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

/// The column range of a long line to shape: it always covers
/// `scroll_col..scroll_col + visible_cols` (clamped to the line), and is
/// [`MAX_SHAPED_COLUMNS`] long, or `visible_cols + WINDOW_STEP` when the viewport is wider than
/// that allows; its start moves in steps of [`WINDOW_STEP`] (the M0 spike's rule, widened in
/// review round 1 so a viewport wider than 512 columns never shows blank text).
pub(crate) fn column_window(
    scroll_col: usize,
    visible_cols: usize,
    line_len: usize,
) -> Range<usize> {
    let len = MAX_SHAPED_COLUMNS.max(visible_cols + WINDOW_STEP);
    let step = (len - visible_cols).min(WINDOW_STEP);
    let start = scroll_col
        .checked_div(step)
        .map_or(scroll_col, |q| q * step);
    let start = start.min(line_len);
    start..(start + len).min(line_len)
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
    /// Hits refused because the stored text or runs differed (a key collision).
    pub collisions: u64,
    /// Paints GPUI reported as failed (counted, never with their text; the crate has no
    /// logger: `tracing` is not one of its dependencies).
    pub paint_errors: u64,
}

struct Entry {
    shaped: ShapedLine,
    /// The runs it was shaped with (the text is `shaped.text`): what a hit is checked against.
    runs: Vec<TextRun>,
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

/// An estimate of what the stored runs hold in memory.
fn runs_bytes(runs: &[TextRun]) -> usize {
    std::mem::size_of_val(runs)
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
        self.shape_keyed(window, key, text, runs, font_size)
    }

    /// [`LayoutCache::shape`] under a given key: a hit counts only when the stored text and
    /// runs are the ones asked for (a key collision is a miss that replaces the entry).
    pub(crate) fn shape_keyed(
        &mut self,
        window: &Window,
        key: ShapeKey,
        text: &str,
        runs: &[TextRun],
        font_size: Pixels,
    ) -> ShapedLine {
        if let Some(e) = self.lru.get(&key) {
            if e.shaped.text.as_ref() == text && e.runs == runs {
                self.stats.hits += 1;
                return e.shaped.clone();
            }
            self.stats.collisions += 1;
        }
        let shaped = window.text_system().shape_line(
            SharedString::from(text.to_string()),
            font_size,
            runs,
            None,
        );
        self.stats.shapes += 1;
        self.stats.longest_shaped = self.stats.longest_shaped.max(text.chars().count());
        let bytes = shaped_bytes(&shaped) + runs_bytes(runs);
        self.bytes += bytes;
        if let Some((_, old)) = self.lru.push(
            key,
            Entry {
                shaped: shaped.clone(),
                runs: runs.to_vec(),
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

    /// `n` paints GPUI reported as failed this frame.
    pub fn paint_failed(&mut self, n: u64) {
        self.stats.paint_errors += n;
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
    /// Digit paints GPUI reported as failed (counted, never with their text).
    pub paint_errors: u64,
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
        if d.paint(
            point(x, top),
            theme.line_height,
            TextAlign::Left,
            None,
            window,
            cx,
        )
        .is_err()
        {
            cx.default_global::<DigitCache>().paint_errors += 1;
        }
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
    }

    /// A viewport wider than the 512-column cap is still covered (review round 1; the spike
    /// shaped 512 and left the rest of a wide viewport blank).
    #[test]
    fn column_window_covers_a_viewport_wider_than_the_cap() {
        for (scroll, visible) in [(1_000, 700), (50_017, 900), (0, 2_000), (99_000, 640)] {
            let w = column_window(scroll, visible, 100_000);
            assert!(w.start <= scroll, "{scroll} {visible}: {w:?}");
            assert!(
                w.end >= (scroll + visible).min(100_000),
                "{scroll} {visible}: {w:?}"
            );
            assert!(w.len() <= MAX_SHAPED_COLUMNS.max(visible + WINDOW_STEP));
        }
        // Up to 384 visible columns the window stays at the cap.
        for visible in [1, 98, 200, 384] {
            assert!(column_window(70_000, visible, 100_000).len() <= MAX_SHAPED_COLUMNS);
        }
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

    /// Two different lines under one key (a forced XXH3 collision): the second is shaped for
    /// itself, never drawn with the first one's glyphs (review round 1).
    #[gpui::test]
    fn a_key_collision_is_a_miss_never_the_other_lines_glyphs(cx: &mut gpui::TestAppContext) {
        let (_, vcx) = cx.add_window_view(|_, _| Blank);
        let theme = EditorTheme::of(&hacivat_ui_core::theme::Theme::new(
            hacivat_ui_core::theme::ThemeMode::Dark,
        ));
        let run = |len| TextRun {
            len,
            font: theme.font.clone(),
            color: theme.chip_text,
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let users = "DROP TABLE users; -- a";
        let trash = "DROP TABLE trash; -- b";
        let key = ShapeKey::of(users, &[run(users.len())], px(12.5), (0, 0));
        let mut cache = LayoutCache::default();
        let (a, b, again) = vcx.update(|window, _| {
            let a = cache.shape_keyed(window, key, users, &[run(users.len())], px(12.5));
            let b = cache.shape_keyed(window, key, trash, &[run(trash.len())], px(12.5));
            let again = cache.shape_keyed(window, key, trash, &[run(trash.len())], px(12.5));
            (a, b, again)
        });
        assert_eq!(a.text.as_ref(), users);
        assert_eq!(
            b.text.as_ref(),
            trash,
            "the colliding line is shaped for itself"
        );
        assert_eq!(again.text.as_ref(), trash);
        let s = cache.stats();
        assert_eq!((s.shapes, s.hits, s.collisions, s.entries), (2, 1, 1, 1));
        // The same text with other runs under the same key is a miss too.
        let mut red = run(trash.len());
        red.color = gpui::hsla(0.0, 0.6, 0.5, 1.0);
        let c = vcx.update(|window, _| cache.shape_keyed(window, key, trash, &[red], px(12.5)));
        assert_eq!(c.text.as_ref(), trash);
        assert_eq!(cache.stats().collisions, 2);
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
