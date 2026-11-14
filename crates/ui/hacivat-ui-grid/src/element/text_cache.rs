//! The frame-LRU text cache (HV-GRID-03, R-M3-12; Task 20): the M0 spike's `FrameLru`
//! (`spikes/spike-grid/src/element.rs:28-110`), keyed `(content hash, font id, size bits, colour
//! class, width bucket)`, [`TEXT_CACHE_CAPACITY`] entries, never evicting the current frame.
//!
//! - **What is cached** is a [`Shaped`] cell: bare line layouts (`LineLayout`) with the colour
//!   class of each piece. The colour is applied when the piece is painted, so a theme switch
//!   recolours every cached shape and reshapes none.
//! - **The content hash** is a fast word-at-a-time hash of the text. Every entry keeps the text
//!   it was shaped from and a lookup compares it, so a hash collision is a miss, never the wrong
//!   string.
//! - **The width bucket** is [`FULL_WIDTH`] for a shape that is not truncated (it fits any
//!   column at least as wide as it is), [`FAST_PREFIX`] for a fast-scroll prefix, and otherwise
//!   the available width in [`BUCKET_PX`] steps: a truncated shape is cut for the bucket's
//!   narrowest width, so it fits every width in its bucket.
//! - **The recency unit is the frame**: entries touched in a frame carry its generation;
//!   eviction drops the oldest generations first down to 7/8 of the capacity, and an entry of
//!   the current frame is never evicted. A hit allocates nothing.

use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::hash::{BuildHasherDefault, Hash, Hasher};
use std::sync::Arc;

use hacivat_ui_core::fonts::{SYNTH_SCALE, font_for};
use hacivat_ui_core::gpui_surface::{
    App, Font, FontId, LineLayout, SharedString, TextRun, Window, hsla, px,
};
use hacivat_ui_core::theme::{Caps, TypeStyle};
use smallvec::{SmallVec, smallvec};

use super::truncate::{
    self, ELLIPSIS, FAST_PREFIX_CHARS, FastCut, Seg, cap_graphemes, cluster_boundaries,
    fitting_boundary, grapheme_cap,
};

/// The most shapes the grid keeps (R-M3-12; the spike kept 4,096).
pub const TEXT_CACHE_CAPACITY: usize = 50_000;

/// The width bucket of a shape that is not truncated.
pub const FULL_WIDTH: u32 = u32::MAX;

/// The width bucket of a fast-scroll prefix.
pub const FAST_PREFIX: u32 = u32::MAX - 1;

/// The step of the width buckets, in logical px.
pub const BUCKET_PX: f32 = 4.0;

/// The bucket of an available width.
pub fn width_bucket(avail: f32) -> u32 {
    if avail.is_finite() && avail > 0.0 {
        ((avail / BUCKET_PX).floor() as u32).min(FAST_PREFIX - 1)
    } else {
        0
    }
}

/// The narrowest width of `bucket`: what a truncated shape in it is cut for.
pub fn bucket_width(bucket: u32) -> f32 {
    bucket as f32 * BUCKET_PX
}

/// A colour role, resolved from the theme when a piece is painted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ColorClass {
    Text,
    Text2,
    Text3,
    Blue,
}

/// What a shape is cached under.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TextKey {
    pub hash: u64,
    pub font: FontId,
    pub size_bits: u32,
    pub color: ColorClass,
    pub width_bucket: u32,
}

/// One painted piece of a cell: a run of its text, a control chip or the trailing chip.
#[derive(Debug, Clone)]
pub struct Piece {
    /// From the start of the cell's text, in logical px.
    pub x: f32,
    pub layout: Arc<LineLayout>,
    pub color: ColorClass,
    pub kind: PieceKind,
    /// The piece's text in [`Shaped::display`] (an ellipsized piece: its kept prefix).
    pub start: u32,
    pub len: u32,
}

/// How a piece is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PieceKind {
    Text,
    /// A control chip (`↵`, `→`, `⟨RLO⟩`): mono 11 on a `seam-soft` well, radius 3.
    Control,
}

/// A shaped cell (or header line, or chip).
#[derive(Debug, Clone)]
pub struct Shaped {
    /// The text the key was built from (compared on every hit).
    pub source: SharedString,
    /// What is painted, controls made visible, before any ellipsis.
    pub display: SharedString,
    pub pieces: SmallVec<[Piece; 2]>,
    /// From the start of the first piece to the end of the last, in logical px.
    pub width: f32,
    /// Ellipsized to fit its width bucket (a fast-scroll prefix: ends in `…`).
    pub truncated: bool,
    /// A fast-scroll stand-in, not the full-quality shape.
    pub degraded: bool,
    /// A fast-scroll frame paints the skeleton bar for this text (no shapes).
    pub skeleton: bool,
}

impl Shaped {
    /// The bytes the painted pieces cover (an ellipsis counts its three bytes).
    pub fn painted_bytes(&self) -> usize {
        self.pieces.iter().map(|p| p.layout.len).sum()
    }

    /// The control chips' texts, in order.
    pub fn chips(&self) -> impl Iterator<Item = &str> + '_ {
        let display = self.display.as_ref();
        self.pieces
            .iter()
            .filter(|p| p.kind == PieceKind::Control)
            .filter_map(move |p| display.get(p.start as usize..(p.start + p.len) as usize))
    }
}

/// A shape this close to its cell's width still fits (sub-pixel rounding).
pub const FIT_SLACK: f32 = 0.5;

/// A control chip's horizontal padding inside its well, and the margin around the well.
pub const CHIP_PAD_PX: f32 = 2.0;
pub const CHIP_MARGIN_PX: f32 = 1.0;

/// The narrow characters whose smallest advance divides a cell's width into its cap.
const NARROW: [char; 10] = [' ', '.', ',', ':', ';', 'i', 'l', 'j', '|', '\''];

/// A text style the grid paints with: its token's font resolved once, the size its glyphs are
/// shaped at, its colour class, and the two widths truncation needs.
#[derive(Debug, Clone)]
pub struct Role {
    pub font: Font,
    pub font_id: FontId,
    /// The shaping size (synthesized small caps: the reduced size).
    pub size: f32,
    /// The token's size (what the 11 px floor and the text registry see).
    pub nominal: f32,
    pub line_height: f32,
    pub color: ColorClass,
    /// The narrowest advance of the font at `size` (0 when unknown: no cap).
    pub min_advance: f32,
    pub ellipsis_w: f32,
}

impl Role {
    pub fn new(style: &TypeStyle, color: ColorClass, window: &Window) -> Self {
        let size = match style.caps {
            Caps::SynthesizedSmallCaps => style.size_px * SYNTH_SCALE,
            Caps::Normal => style.size_px,
        };
        let font = font_for(style);
        let ts = window.text_system();
        let font_id = ts.resolve_font(&font);
        let min_advance = NARROW
            .iter()
            .filter_map(|c| ts.advance(font_id, px(size), *c).ok())
            .map(|s| f32::from(s.width))
            .filter(|w| *w > 0.0)
            .fold(f32::INFINITY, f32::min);
        let mut role = Role {
            font,
            font_id,
            size,
            nominal: style.size_px,
            line_height: style.line_height,
            color,
            min_advance: if min_advance.is_finite() {
                min_advance
            } else {
                0.0
            },
            ellipsis_w: 0.0,
        };
        role.ellipsis_w = f32::from(role.layout(ELLIPSIS, window).width);
        role
    }

    /// Shapes `text` (one line, no newline) in this role.
    pub fn layout(&self, text: &str, window: &Window) -> Arc<LineLayout> {
        let run = TextRun {
            len: text.len(),
            font: self.font.clone(),
            color: hsla(0.0, 0.0, 0.0, 1.0),
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        window
            .text_system()
            .layout_line(text, px(self.size), &[run], None)
    }

    /// The cache key of a text with content hash `hash` in this role.
    pub fn key(&self, hash: u64, width_bucket: u32) -> TextKey {
        TextKey {
            hash,
            font: self.font_id,
            size_bits: self.size.to_bits(),
            color: self.color,
            width_bucket,
        }
    }
}

/// Buffers the shaper reuses across cells (a miss allocates only what it keeps).
#[derive(Debug, Default)]
pub struct Scratch {
    pub segs: Vec<Seg>,
    pub bounds: Vec<usize>,
    pub buf: String,
}

/// The frame's shaping count and its cap (fast-scroll: [`super::FAST_SHAPES_PER_FRAME`]), and
/// the paint path's debug allocation counter.
#[derive(Debug, Clone, Copy)]
pub struct Budget {
    pub shapes: u32,
    pub limit: u32,
    /// Cells left unpainted because the budget was spent.
    pub skipped: u32,
    /// Allocations the cell paint path made (debug counter): one per shaped layout and one per
    /// cache entry (its text and pieces), plus the debug records. A hit counts nothing.
    pub allocs: u32,
}

impl Budget {
    pub fn new(limit: u32) -> Self {
        Budget {
            shapes: 0,
            limit,
            skipped: 0,
            allocs: 0,
        }
    }

    pub fn spent(&self) -> bool {
        self.shapes >= self.limit
    }
}

fn piece(
    x: f32,
    layout: Arc<LineLayout>,
    color: ColorClass,
    kind: PieceKind,
    start: usize,
    len: usize,
) -> Piece {
    Piece {
        x,
        layout,
        color,
        kind,
        start: start as u32,
        len: len as u32,
    }
}

/// The right edge of a piece, a chip's well and margin included.
fn piece_end(p: &Piece) -> f32 {
    let w = f32::from(p.layout.width);
    match p.kind {
        PieceKind::Text => p.x + w,
        PieceKind::Control => p.x + w + CHIP_PAD_PX + CHIP_MARGIN_PX,
    }
}

/// No pieces: an empty text, or (`skeleton`) a fast-scroll frame's skeleton bar.
fn empty_shape(text: &SharedString, skeleton: bool) -> Shaped {
    Shaped {
        source: text.clone(),
        display: if skeleton {
            SharedString::default()
        } else {
            text.clone()
        },
        pieces: SmallVec::new(),
        width: 0.0,
        truncated: false,
        degraded: skeleton,
        skeleton,
    }
}

/// Shapes `text` in `role` for a cell `avail` px wide, its controls as chips in `chip`: the
/// shape and the width bucket it is cached under ([`FULL_WIDTH`] when it is not truncated).
pub fn shape_full(
    text: &SharedString,
    role: &Role,
    chip: &Role,
    avail: f32,
    window: &Window,
    scratch: &mut Scratch,
    shapes: &mut u32,
) -> (Shaped, u32) {
    if truncate::has_controls(text) {
        shape_with_chips(text, role, chip, avail, window, scratch, shapes)
    } else {
        shape_plain(text, role, avail, window, scratch, shapes)
    }
}

fn shape_plain(
    text: &SharedString,
    role: &Role,
    avail: f32,
    window: &Window,
    scratch: &mut Scratch,
    shapes: &mut u32,
) -> (Shaped, u32) {
    let (capped, cut) = cap_graphemes(text, grapheme_cap(avail, role.min_advance));
    if capped.is_empty() {
        return (empty_shape(text, false), FULL_WIDTH);
    }
    let layout = role.layout(capped, window);
    *shapes += 1;
    let width = f32::from(layout.width);
    if !cut && width <= avail + FIT_SLACK {
        let pieces = smallvec![piece(
            0.0,
            layout,
            role.color,
            PieceKind::Text,
            0,
            text.len()
        )];
        let shaped = Shaped {
            source: text.clone(),
            display: text.clone(),
            pieces,
            width,
            truncated: false,
            degraded: false,
            skeleton: false,
        };
        return (shaped, FULL_WIDTH);
    }
    let bucket = width_bucket(avail);
    cluster_boundaries(capped, &mut scratch.bounds);
    let keep = fitting_boundary(
        &scratch.bounds,
        bucket_width(bucket),
        role.ellipsis_w,
        |i| f32::from(layout.x_for_index(i)),
    );
    scratch.buf.clear();
    scratch.buf.push_str(&capped[..keep]);
    scratch.buf.push_str(ELLIPSIS);
    let cut_layout = role.layout(&scratch.buf, window);
    *shapes += 1;
    let width = f32::from(cut_layout.width);
    let shaped = Shaped {
        source: text.clone(),
        display: text.clone(),
        pieces: smallvec![piece(0.0, cut_layout, role.color, PieceKind::Text, 0, keep)],
        width,
        truncated: true,
        degraded: false,
        skeleton: false,
    };
    (shaped, bucket)
}

fn shape_with_chips(
    text: &SharedString,
    role: &Role,
    chip: &Role,
    avail: f32,
    window: &Window,
    scratch: &mut Scratch,
    shapes: &mut u32,
) -> (Shaped, u32) {
    let narrowest = if role.min_advance > 0.0 && chip.min_advance > 0.0 {
        role.min_advance.min(chip.min_advance)
    } else {
        0.0
    };
    let (capped, cut) = cap_graphemes(text, grapheme_cap(avail, narrowest));
    truncate::segments(capped, &mut scratch.segs);
    let mut display = String::with_capacity(capped.len() + 16);
    let mut pieces: SmallVec<[Piece; 2]> = SmallVec::new();
    let mut x = 0.0f32;
    for seg in &scratch.segs {
        match seg {
            Seg::Text(r) => {
                let t = &capped[r.clone()];
                let layout = role.layout(t, window);
                *shapes += 1;
                let w = f32::from(layout.width);
                let start = display.len();
                pieces.push(piece(
                    x,
                    layout,
                    role.color,
                    PieceKind::Text,
                    start,
                    t.len(),
                ));
                display.push_str(t);
                x += w;
            }
            Seg::Chip(c) => {
                scratch.buf.clear();
                c.write(&mut scratch.buf);
                let layout = chip.layout(&scratch.buf, window);
                *shapes += 1;
                x += CHIP_MARGIN_PX + CHIP_PAD_PX;
                let w = f32::from(layout.width);
                let (start, len) = (display.len(), scratch.buf.len());
                pieces.push(piece(x, layout, chip.color, PieceKind::Control, start, len));
                display.push_str(&scratch.buf);
                x += w + CHIP_PAD_PX + CHIP_MARGIN_PX;
            }
        }
    }
    let display = SharedString::from(display);
    if !cut && x <= avail + FIT_SLACK {
        let shaped = Shaped {
            source: text.clone(),
            display,
            pieces,
            width: x,
            truncated: false,
            degraded: false,
            skeleton: false,
        };
        return (shaped, FULL_WIDTH);
    }
    // Keep the whole pieces that fit with an ellipsis after them; cut the first text piece that
    // does not, or end with the ellipsis where a chip would not fit.
    let bucket = width_bucket(avail);
    let limit = bucket_width(bucket);
    let mut kept: SmallVec<[Piece; 2]> = SmallVec::new();
    let mut end = 0.0f32;
    let mut ended = false;
    for p in pieces {
        let p_end = piece_end(&p);
        if p_end + role.ellipsis_w <= limit {
            end = p_end;
            kept.push(p);
            continue;
        }
        if p.kind == PieceKind::Text {
            let t = &display[p.start as usize..(p.start + p.len) as usize];
            cluster_boundaries(t, &mut scratch.bounds);
            let keep = fitting_boundary(&scratch.bounds, limit - p.x, role.ellipsis_w, |i| {
                f32::from(p.layout.x_for_index(i))
            });
            scratch.buf.clear();
            scratch.buf.push_str(&t[..keep]);
            scratch.buf.push_str(ELLIPSIS);
            let layout = role.layout(&scratch.buf, window);
            *shapes += 1;
            end = p.x + f32::from(layout.width);
            let start = p.start as usize;
            kept.push(piece(p.x, layout, role.color, PieceKind::Text, start, keep));
            ended = true;
        }
        break;
    }
    if !ended {
        let layout = role.layout(ELLIPSIS, window);
        *shapes += 1;
        let at = kept.last().map_or(0, |p| (p.start + p.len) as usize);
        let w = f32::from(layout.width);
        kept.push(piece(end, layout, role.color, PieceKind::Text, at, 0));
        end += w;
    }
    let shaped = Shaped {
        source: text.clone(),
        display,
        pieces: kept,
        width: end,
        truncated: true,
        degraded: false,
        skeleton: false,
    };
    (shaped, bucket)
}

/// The most layouts one fast-scroll miss shapes (a prefix, then its cut to the cell's width).
pub const FAST_MISS_SHAPES: u32 = 2;

/// The fast-scroll shape of `text` for a cell `avail` px wide ([`truncate::fast_cut`]), and
/// the width bucket it is cached under:
/// - printable ASCII within the cap: the full-quality shape (`shape_plain`), cached as such;
/// - otherwise its ASCII-safe prefix of at most [`FAST_PREFIX_CHARS`] characters **followed
///   by `…`**, cut further to the cell's width when needed ([`FAST_PREFIX`]);
/// - no such prefix, or one ending at a number's group separator: the skeleton bar
///   ([`FAST_PREFIX`], no shapes).
pub fn shape_fast(
    text: &SharedString,
    role: &Role,
    avail: f32,
    window: &Window,
    scratch: &mut Scratch,
    shapes: &mut u32,
) -> (Shaped, u32) {
    let cap = grapheme_cap(avail, role.min_advance).min(FAST_PREFIX_CHARS);
    let n = match truncate::fast_cut(text, cap) {
        FastCut::Whole => return shape_plain(text, role, avail, window, scratch, shapes),
        FastCut::Skeleton => return (empty_shape(text, true), FAST_PREFIX),
        FastCut::Prefix(n) => n,
    };
    let prefix = &text[..n];
    scratch.buf.clear();
    scratch.buf.push_str(prefix);
    scratch.buf.push_str(ELLIPSIS);
    let mut layout = role.layout(&scratch.buf, window);
    *shapes += 1;
    let mut keep = n;
    if f32::from(layout.width) > avail + FIT_SLACK {
        // ASCII: every byte is a boundary.
        cluster_boundaries(prefix, &mut scratch.bounds);
        keep = fitting_boundary(&scratch.bounds, avail, role.ellipsis_w, |i| {
            f32::from(layout.x_for_index(i))
        });
        if keep == 0 {
            return (empty_shape(text, true), FAST_PREFIX);
        }
        scratch.buf.clear();
        scratch.buf.push_str(&prefix[..keep]);
        scratch.buf.push_str(ELLIPSIS);
        layout = role.layout(&scratch.buf, window);
        *shapes += 1;
    }
    let width = f32::from(layout.width);
    let shaped = Shaped {
        source: text.clone(),
        display: SharedString::from(prefix[..keep].to_string()),
        pieces: smallvec![piece(0.0, layout, role.color, PieceKind::Text, 0, keep)],
        width,
        truncated: true,
        degraded: true,
        skeleton: false,
    };
    (shaped, FAST_PREFIX)
}

/// Finds the shape of `text` in `role` for a cell `avail` px wide, shaping it on a miss, and
/// hands it to `f`. In fast-scroll mode a miss takes [`shape_fast`], and once the frame's
/// budget cannot pay for another miss ([`FAST_MISS_SHAPES`]) it shapes nothing (`None`,
/// counted in [`Budget::skipped`]); the caller paints the skeleton bar.
///
/// The lookup order: the untruncated shape (when it fits), the truncated shape of this width
/// bucket, then (fast-scroll) the fast shape (when it fits). Each hit compares the cached source
/// text, so a hash collision is a miss. A hit allocates nothing.
#[allow(clippy::too_many_arguments)]
pub fn with_shape<R>(
    cache: &mut FrameLru<TextKey, Shaped>,
    text: &SharedString,
    role: &Role,
    chip: &Role,
    avail: f32,
    fast: bool,
    budget: &mut Budget,
    scratch: &mut Scratch,
    window: &mut Window,
    cx: &mut App,
    f: impl FnOnce(&Shaped, &mut Window, &mut App) -> R,
) -> Option<R> {
    let hash = content_hash(text);
    let full = role.key(hash, FULL_WIDTH);
    if let Some(s) = cache.get(&full)
        && s.source == *text
        && s.width <= avail + FIT_SLACK
    {
        return Some(f(s, window, cx));
    }
    let cut = role.key(hash, width_bucket(avail));
    if let Some(s) = cache.get(&cut)
        && s.source == *text
    {
        return Some(f(s, window, cx));
    }
    if fast {
        let key = role.key(hash, FAST_PREFIX);
        if let Some(s) = cache.get(&key)
            && s.source == *text
            && s.width <= avail + FIT_SLACK
        {
            return Some(f(s, window, cx));
        }
        if budget.shapes + FAST_MISS_SHAPES > budget.limit {
            budget.skipped += 1;
            return None;
        }
        let before = budget.shapes;
        let (shaped, bucket) = shape_fast(text, role, avail, window, scratch, &mut budget.shapes);
        budget.allocs += budget.shapes - before + 1;
        return Some(f(cache.insert(role.key(hash, bucket), shaped), window, cx));
    }
    let before = budget.shapes;
    let (shaped, bucket) = shape_full(text, role, chip, avail, window, scratch, &mut budget.shapes);
    budget.allocs += budget.shapes - before + 1;
    Some(f(cache.insert(role.key(hash, bucket), shaped), window, cx))
}

/// The multiplier of [`FxHasher`] (rustc's `FxHash` constant).
const FX_SEED: u64 = 5_871_781_006_564_002_453;

/// A word-at-a-time multiplicative hasher: the content hash, and the cache map's hasher (its
/// keys already carry a content hash, so SipHash would only cost time).
#[derive(Debug, Default, Clone, Copy)]
pub struct FxHasher(u64);

impl FxHasher {
    fn add(&mut self, word: u64) {
        self.0 = (self.0.rotate_left(5) ^ word).wrapping_mul(FX_SEED);
    }
}

impl Hasher for FxHasher {
    fn finish(&self) -> u64 {
        self.0
    }

    fn write(&mut self, bytes: &[u8]) {
        let (words, rest) = bytes.as_chunks::<8>();
        for word in words {
            self.add(u64::from_le_bytes(*word));
        }
        if !rest.is_empty() {
            let mut word = [0u8; 8];
            word[..rest.len()].copy_from_slice(rest);
            self.add(u64::from_le_bytes(word));
        }
    }

    fn write_u8(&mut self, i: u8) {
        self.add(u64::from(i));
    }

    fn write_u32(&mut self, i: u32) {
        self.add(u64::from(i));
    }

    fn write_u64(&mut self, i: u64) {
        self.add(i);
    }

    fn write_usize(&mut self, i: usize) {
        self.add(i as u64);
    }

    fn write_isize(&mut self, i: isize) {
        self.add(i as u64);
    }
}

/// The content hash of `text`.
pub fn content_hash(text: &str) -> u64 {
    let mut h = FxHasher::default();
    h.write(text.as_bytes());
    h.write_usize(text.len());
    h.finish()
}

type FxBuild = BuildHasherDefault<FxHasher>;

struct Slot<V> {
    value: V,
    generation: u64,
}

/// An LRU whose recency unit is the frame (module docs).
pub struct FrameLru<K, V> {
    map: HashMap<K, Slot<V>, FxBuild>,
    capacity: usize,
    generation: u64,
}

impl<K, V> std::fmt::Debug for FrameLru<K, V> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FrameLru")
            .field("len", &self.map.len())
            .field("capacity", &self.capacity)
            .field("generation", &self.generation)
            .finish()
    }
}

impl<K: Hash + Eq + Copy, V> FrameLru<K, V> {
    pub fn new(capacity: usize) -> Self {
        FrameLru {
            map: HashMap::with_capacity_and_hasher(capacity.min(4_096), FxBuild::default()),
            capacity: capacity.max(1),
            generation: 1,
        }
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Starts a new frame generation.
    pub fn begin_frame(&mut self) {
        self.generation += 1;
    }

    /// The cached value, now marked as used in this frame.
    pub fn get(&mut self, key: &K) -> Option<&V> {
        let generation = self.generation;
        self.map.get_mut(key).map(|slot| {
            slot.generation = generation;
            &slot.value
        })
    }

    /// Stores `value` under `key` (replacing an entry already there), evicting old frames first
    /// when the cache is full.
    pub fn insert(&mut self, key: K, value: V) -> &V {
        if !self.map.contains_key(&key) && self.map.len() >= self.capacity {
            self.evict();
        }
        let generation = self.generation;
        let slot = Slot { value, generation };
        match self.map.entry(key) {
            Entry::Occupied(mut e) => {
                e.insert(slot);
                &e.into_mut().value
            }
            Entry::Vacant(e) => &e.insert(slot).value,
        }
    }

    /// The cached value, built by `build` on a miss; the bool is true on a miss.
    pub fn get_or_insert_with(&mut self, key: K, build: impl FnOnce() -> V) -> (&V, bool) {
        if self.map.contains_key(&key) {
            let value = self.get(&key);
            return (value.expect("checked above"), false);
        }
        (self.insert(key, build()), true)
    }

    pub fn clear(&mut self) {
        self.map.clear();
    }

    /// Drops the least recently used generations until the cache is at 7/8 of its capacity;
    /// entries of the current frame always stay.
    fn evict(&mut self) {
        let target = self.capacity - self.capacity / 8;
        if self.map.len() <= target {
            return;
        }
        let mut generations: Vec<u64> = self.map.values().map(|s| s.generation).collect();
        let drop = generations.len() - target;
        let (_, nth, _) = generations.select_nth_unstable(drop - 1);
        let cutoff = (*nth).min(self.generation - 1);
        self.map.retain(|_, slot| slot.generation > cutoff);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hits_do_not_rebuild() {
        let mut lru: FrameLru<u8, usize> = FrameLru::new(8);
        let mut builds = 0;
        for _ in 0..3 {
            lru.begin_frame();
            let (v, _) = lru.get_or_insert_with(0, || {
                builds += 1;
                7
            });
            assert_eq!(*v, 7);
        }
        assert_eq!(builds, 1);
    }

    #[test]
    fn keys_differ_by_every_field() {
        let base = TextKey {
            hash: content_hash("x"),
            font: FontId(1),
            size_bits: 13f32.to_bits(),
            color: ColorClass::Text,
            width_bucket: FULL_WIDTH,
        };
        let variants = [
            TextKey {
                hash: content_hash("y"),
                ..base
            },
            TextKey {
                font: FontId(2),
                ..base
            },
            TextKey {
                size_bits: 11f32.to_bits(),
                ..base
            },
            TextKey {
                color: ColorClass::Text3,
                ..base
            },
            TextKey {
                width_bucket: 12,
                ..base
            },
        ];
        let mut lru: FrameLru<TextKey, u8> = FrameLru::new(16);
        lru.insert(base, 0);
        for (i, k) in variants.iter().enumerate() {
            assert!(lru.get(k).is_none(), "variant {i} must miss");
            lru.insert(*k, i as u8 + 1);
        }
        assert_eq!(lru.get(&base), Some(&0));
        assert_eq!(lru.len(), 6);
    }

    /// The spike's eviction test: an entry touched every frame survives, the newest frame is
    /// intact, an old frame is gone, and the cache never grows past its capacity.
    #[test]
    fn stays_bounded_and_evicts_oldest_frames_first() {
        let mut lru: FrameLru<(u8, u32), u32> = FrameLru::new(64);
        for frame in 0..100u32 {
            lru.begin_frame();
            lru.get_or_insert_with((0, 0), || 0);
            for i in 0..10 {
                lru.get_or_insert_with((1, frame * 100 + i), || frame);
            }
            assert!(lru.len() <= 64, "len {}", lru.len());
        }
        assert!(!lru.get_or_insert_with((0, 0), || 1).1);
        assert!(!lru.get_or_insert_with((1, 9_900), || 1).1);
        assert!(lru.get_or_insert_with((1, 0), || 1).1);
    }

    #[test]
    fn never_evicts_the_current_frame() {
        let mut lru: FrameLru<u32, u32> = FrameLru::new(4);
        lru.begin_frame();
        for i in 0..10 {
            lru.get_or_insert_with(i, || i);
        }
        for i in 0..10 {
            assert!(!lru.get_or_insert_with(i, || 99).1, "{i}");
        }
    }

    /// A cache entry under `text`'s own key but shaped from another text (a forced content-hash
    /// collision).
    fn impostor(width: f32) -> Shaped {
        Shaped {
            source: SharedString::new_static("a different value"),
            display: SharedString::new_static("a different value"),
            pieces: SmallVec::new(),
            width,
            truncated: false,
            degraded: false,
            skeleton: false,
        }
    }

    /// Forces a collision under each of the three lookup keys (untruncated, this width bucket,
    /// fast-scroll) and checks the lookup paints `text`'s own shape, never the impostor's:
    /// removing a `s.source == *text` guard fails the matching case.
    #[gpui::test]
    fn a_content_hash_collision_is_a_miss_never_the_wrong_text(cx: &mut gpui::TestAppContext) {
        let vcx = cx.add_empty_window();
        vcx.update(|window, cx| {
            let role = Role::new(
                &hacivat_ui_core::theme::DARK.type_ramp.body,
                ColorClass::Text,
                window,
            );
            let text = SharedString::new_static("the real value");
            let hash = content_hash(&text);
            for (name, bucket, avail, fast) in [
                ("untruncated", FULL_WIDTH, 500.0, false),
                ("width bucket", width_bucket(500.0), 500.0, false),
                ("fast-scroll", FAST_PREFIX, 500.0, true),
            ] {
                let mut cache = FrameLru::new(16);
                cache.insert(role.key(hash, bucket), impostor(0.0));
                let mut budget = Budget::new(u32::MAX);
                let painted = with_shape(
                    &mut cache,
                    &text,
                    &role,
                    &role,
                    avail,
                    fast,
                    &mut budget,
                    &mut Scratch::default(),
                    window,
                    cx,
                    |s, _, _| s.source.clone(),
                );
                assert_eq!(painted.as_deref(), Some("the real value"), "{name}");
                assert!(
                    budget.shapes > 0,
                    "{name}: shaped, not served from the impostor"
                );
            }
        });
    }

    #[test]
    fn content_hash_sees_every_byte_and_the_length() {
        assert_ne!(content_hash("abcdefgh1"), content_hash("abcdefgh2"));
        assert_ne!(content_hash("a"), content_hash("a\0"));
        assert_ne!(content_hash(""), content_hash("\0"));
        assert_eq!(content_hash("İstanbul"), content_hash("İstanbul"));
    }

    #[test]
    fn buckets_floor_to_their_narrowest_width() {
        assert_eq!(width_bucket(0.0), 0);
        assert_eq!(width_bucket(-3.0), 0);
        assert_eq!(width_bucket(f32::NAN), 0);
        assert_eq!(width_bucket(7.9), 1);
        assert_eq!(width_bucket(8.0), 2);
        assert!(bucket_width(width_bucket(141.0)) <= 141.0);
        assert!(width_bucket(f32::MAX) < FAST_PREFIX);
    }
}
