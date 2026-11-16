//! Column auto-size (HV-GRID-04; Task 17): type default widths (the first frame never waits for
//! a measurement), a sampled measure through a closure (the element shapes the text), the
//! outlier trim, the header cap, the clamps and the fill.
//!
//! Widths are logical px. The measure closure answers the whole width a cell needs (text,
//! padding and chips), so this module knows no fonts and no GPUI.

use crate::source::ColumnKind;

/// The narrowest column.
pub const MIN_WIDTH: f32 = 48.0;
/// The widest column an initial size picks.
pub const MAX_INITIAL_WIDTH: f32 = 480.0;
/// The widest column anything picks (auto-fit, a drag).
pub const MAX_WIDTH: f32 = 2_000.0;
/// A header widens its column at most this far; a longer name truncates with a tooltip.
pub const MAX_HEADER_WIDTH: f32 = 320.0;
/// Results with at most this many columns fill the viewport.
pub const FILL_MAX_COLUMNS: usize = 8;

/// Rows sampled from the start, from the end, and spread between them.
const SAMPLE_HEAD: u64 = 50;
const SAMPLE_TAIL: u64 = 10;
const SAMPLE_STRATA: u64 = 20;

/// At this many samples the trim repeats (up to three passes).
const MANY_SAMPLES: usize = 20;
/// Up to this many samples nothing is trimmed.
const FEW_SAMPLES: usize = 5;

/// What a column holds, for its starting width (the adapter maps its logical types here).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WidthClass {
    Bool,
    Integer,
    Decimal,
    Uuid,
    Timestamp,
    Text,
    Json,
    Binary,
}

impl WidthClass {
    /// The width before any measurement (HV-GRID-04 step 2).
    pub const fn default_width(self) -> f32 {
        match self {
            WidthClass::Bool => 64.0,
            WidthClass::Integer => 96.0,
            WidthClass::Decimal => 120.0,
            WidthClass::Uuid => 300.0,
            WidthClass::Timestamp => 200.0,
            WidthClass::Text => 160.0,
            WidthClass::Json | WidthClass::Binary => 200.0,
        }
    }
}

/// Whether the fill may grow a column of `kind` (text-like; numbers never grow).
pub fn is_text_like(kind: ColumnKind) -> bool {
    matches!(
        kind,
        ColumnKind::Text | ColumnKind::Json | ColumnKind::Other
    )
}

/// The rows to measure out of `loaded`: the first 50, the last 10, and 20 between them, one at
/// the middle of each twentieth. Sorted, without repeats; every row when there are 80 or fewer.
pub fn sample_rows(loaded: u64) -> Vec<u64> {
    if loaded <= SAMPLE_HEAD + SAMPLE_TAIL + SAMPLE_STRATA {
        return (0..loaded).collect();
    }
    let (lo, hi) = (SAMPLE_HEAD, loaded - SAMPLE_TAIL);
    let span = u128::from(hi - lo);
    let mut rows: Vec<u64> = (0..SAMPLE_HEAD).collect();
    // More than 20 rows between head and tail, so the strata are distinct.
    rows.extend((0..SAMPLE_STRATA).map(|i| {
        let at = u128::from(2 * i + 1) * span / u128::from(2 * SAMPLE_STRATA);
        lo + at as u64
    }));
    rows.extend(hi..loaded);
    rows
}

/// The width candidate of a column's samples and how many were trimmed.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Trimmed {
    pub width: f32,
    pub trimmed: usize,
}

/// The widest sample after trimming outliers (HV-GRID-04 step 4): with more than 5 samples,
/// drop every width at or above twice the mean; repeat up to 3 passes from 20 samples. Widths
/// that are not finite and non-negative are ignored.
pub fn trim_outliers(widths: &[f32]) -> Trimmed {
    let mut kept: Vec<f32> = widths
        .iter()
        .copied()
        .filter(|w| w.is_finite() && *w >= 0.0)
        .collect();
    let passes = if kept.len() >= MANY_SAMPLES { 3 } else { 1 };
    let mut trimmed = 0;
    for _ in 0..passes {
        if kept.len() <= FEW_SAMPLES {
            break;
        }
        let mean = kept.iter().map(|w| f64::from(*w)).sum::<f64>() / kept.len() as f64;
        if mean <= 0.0 {
            break;
        }
        let before = kept.len();
        kept.retain(|w| f64::from(*w) < 2.0 * mean);
        let dropped = before - kept.len();
        if dropped == 0 {
            break;
        }
        trimmed += dropped;
    }
    Trimmed {
        width: kept.iter().copied().fold(0.0, f32::max),
        trimmed,
    }
}

/// A column's initial width from its measured samples and its header's width: the trimmed
/// candidate or the header (capped at [`MAX_HEADER_WIDTH`]), whichever is wider, in whole
/// pixels, clamped to `[MIN_WIDTH, MAX_INITIAL_WIDTH]`.
pub fn column_width(sample_widths: &[f32], header_w: f32) -> f32 {
    sized(sample_widths, header_w, MAX_INITIAL_WIDTH)
}

/// Auto-fit (a double-click on the header border): as [`column_width`], up to [`MAX_WIDTH`].
pub fn fit_width(sample_widths: &[f32], header_w: f32) -> f32 {
    sized(sample_widths, header_w, MAX_WIDTH)
}

/// The initial width of a column of `loaded` rows: `measure(row)` runs on the
/// [`sample_rows`] only.
pub fn initial_width(loaded: u64, header_w: f32, measure: impl FnMut(u64) -> f32) -> f32 {
    let widths: Vec<f32> = sample_rows(loaded).into_iter().map(measure).collect();
    column_width(&widths, header_w)
}

/// A width the user dragged to, clamped to `[MIN_WIDTH, MAX_WIDTH]`.
pub fn clamp_user_width(width: f32) -> f32 {
    if width.is_finite() {
        width.clamp(MIN_WIDTH, MAX_WIDTH)
    } else {
        MIN_WIDTH
    }
}

/// The fill (HV-GRID-04 step 7): when at most [`FILL_MAX_COLUMNS`] columns are narrower than the
/// viewport together, the text-like ones (`text_like`, parallel to `widths`) grow in proportion
/// to their widths until the columns fill it, in whole pixels (the remainder goes to the last
/// text-like column), never past [`MAX_WIDTH`]. Numbers never grow.
pub fn fill(widths: &mut [f32], text_like: &[bool], viewport_w: f32) {
    if widths.is_empty()
        || widths.len() > FILL_MAX_COLUMNS
        || widths.len() != text_like.len()
        || !viewport_w.is_finite()
    {
        return;
    }
    let target = f64::from(viewport_w).floor();
    let sum: f64 = widths.iter().map(|w| f64::from(*w)).sum();
    let text_sum: f64 = widths
        .iter()
        .zip(text_like)
        .filter(|(_, text)| **text)
        .map(|(w, _)| f64::from(*w))
        .sum();
    if sum >= target || text_sum <= 0.0 {
        return;
    }
    let extra = target - sum;
    let mut given = 0.0;
    let mut last = None;
    for (i, (w, text)) in widths.iter_mut().zip(text_like).enumerate() {
        if *text {
            let add = (extra * f64::from(*w) / text_sum).floor();
            *w = (f64::from(*w) + add) as f32;
            given += add;
            last = Some(i);
        }
    }
    if let Some(i) = last {
        widths[i] = (f64::from(widths[i]) + extra - given) as f32;
    }
    for (w, text) in widths.iter_mut().zip(text_like) {
        if *text {
            *w = w.min(MAX_WIDTH);
        }
    }
}

fn sized(sample_widths: &[f32], header_w: f32, max: f32) -> f32 {
    let candidate = trim_outliers(sample_widths).width;
    let header = if header_w.is_finite() {
        header_w.clamp(0.0, MAX_HEADER_WIDTH)
    } else {
        0.0
    };
    candidate.max(header).ceil().clamp(MIN_WIDTH, max)
}
