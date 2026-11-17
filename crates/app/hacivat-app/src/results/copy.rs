//! Copy from a result grid (Task 26): the selected region as TSV, with headers on request,
//! refused past the copy limits.
//!
//! # What a copy holds (minimal HV-XFER-02)
//!
//! Copy reads the **raw** channel (`valfmt::raw_text_in`) of the snapshot the grid shows, never
//! the display text: no grouping, no locale, no grapheme cap. So the clipboard is the same in
//! every format locale, and a cell and its copy can differ:
//!
//! - **Numbers** are ungrouped ASCII (`1234567.50` where the cell shows `1.234.567,50`).
//! - **`timestamptz`** is written in the console's display zone with that zone's offset, the
//!   instant the cell shows (an offset the server sent is kept).
//! - **Binary values are always `\x` hex.** The cell may show something else: the grid's
//!   `Detect` mode shows *any* 16-byte value as a UUID and mostly-printable bytes as text. The
//!   copy of such a cell is its bytes in hex, not the UUID or the text on screen.
//! - **Text that is not valid UTF-8** (`Lossy`) is its valid runs with every other byte as
//!   `\xNN`. This form is **not byte-exact and not reversible**: a backslash in the valid part
//!   is not escaped, so a literal `\x41` typed into a text value and the escape of byte 0x41
//!   read the same. A lossless copy needs the private clipboard flavour (M6).
//! - **A truncated value** copies the prefix the result kept: that is all of it the result
//!   has. The cell counts as incomplete (below).
//! - **A value that could not be decoded** copies as an **empty cell**. The decoder's message
//!   is not something the server sent, so it never reaches the clipboard. The cell counts as
//!   incomplete. A cell the row did not carry copies as empty.
//! - **Control characters** (bidi overrides, tabs, line breaks inside a value) are kept
//!   verbatim; the grid's chips are display only.
//!
//! # Incomplete cells
//!
//! A copy that holds truncated or undecodable cells says so: in place of the plain "Copied"
//! line its toast warns how many of the copied values are incomplete
//! (`hv-copy-done-incomplete`), and [`CopyOutcome::Copied`] carries the count.
//!
//! # Which result, which rows
//!
//! - **The generation.** A region is cut from the result the grid last painted. A re-run
//!   moves the adapter to a new result at once, a frame before the grid shows it, so
//!   `GridEvent::CopyRequested` carries the generation its region was cut from and
//!   [`copy_region`] copies only while the adapter is still at it. Otherwise it answers
//!   [`CopyOutcome::Stale`], with no clipboard write and no toast: the rows under the old
//!   selection's positions are rows the user neither saw nor selected.
//! - **View rows go through the map** (## m3c-t36, ledger 253 I5). `CopyRegion::rows` are the
//!   grid's *view* rows. [`copy_region`] takes the grid's `ViewMap` and reads each view row's
//!   model row (`view_to_model`), in view order, so a copy after a sort or under a filter holds
//!   exactly the rows the user sees, in the order shown; a view row the map does not show is
//!   left out. Fix round 1 (m5): every entry point takes the map, so no caller can read view
//!   rows as model rows (a grid without a sort or filter passes `ViewMap::identity`).
//!
//! # The TSV
//!
//! One line per row, each ended by `\n`, cells separated by `\t`. SQL NULL is an empty cell
//! (as is the empty string: TSV has no NULL). A cell is quoted with `"` (inner quotes doubled)
//! when it holds a tab, a line feed, a carriage return or a quote, or begins or ends with
//! white space. With headers, the first line holds the column names, quoted by the same rule.
//!
//! # Limits
//!
//! More than [`MAX_COPY_CELLS`] cells or more than [`MAX_COPY_BYTES`] of text is refused with
//! a toast (export is M6); nothing is put on the clipboard. The byte limit is checked **per
//! cell, before the cell's text is built**, from the length the value is known to need (a
//! binary value's hex is two digits per byte), so one huge value is refused without being
//! expanded.
//!
//! The text is built on the background executor, from the snapshot taken at the request,
//! above [`BACKGROUND_COPY_CELLS`] cells **or** above [`BACKGROUND_COPY_BYTES`] of text
//! (estimated the same way, without building anything); the UI thread then only writes the
//! clipboard. The text is one `ClipboardItem::new_string`.

use std::rc::Rc;

use hacivat_l10n::{Count, Text, ids};
use hacivat_ui::icon::IconName;
use hacivat_ui::toast::{Toast, ToastKind, ToastQueue};
use hacivat_ui_core::gpui_surface::{App, AppContext as _, ClipboardItem, Task};
use hacivat_ui_grid::ViewRow;
use hacivat_ui_grid::model::{CopyRefusal, CopyRegion, ViewMap};
use hacivat_values::ValueRef;
use hacivat_values::valfmt::{RawText, raw_text_in};

use crate::l10n_global::{text_fn, tr};
use crate::results::adapter::{GridAdapter, RawView, Texts};

/// The most cells a copy takes (HV-XFER-02 step 2).
pub const MAX_COPY_CELLS: u128 = hacivat_ui_grid::model::MAX_COPY_CELLS;

/// The most text a copy takes: 100 MB.
pub const MAX_COPY_BYTES: usize = 100 << 20;

/// Above this much text (by [`estimated_bytes`]) the text is built on the background
/// executor, however few cells hold it: 1 MiB.
pub const BACKGROUND_COPY_BYTES: usize = 1 << 20;

/// Above this many cells the text is built on the background executor.
pub const BACKGROUND_COPY_CELLS: u128 = 10_000;

/// How a copy ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CopyOutcome {
    /// The clipboard holds `rows` × `cols` cells, `incomplete` of them cut short or not
    /// decodable.
    Copied {
        rows: u64,
        cols: u64,
        incomplete: u64,
    },
    /// Nothing was copied: the region was cut from a result the adapter no longer shows (a
    /// re-run arrived between the request and the copy).
    Stale,
    /// Refused: more than [`MAX_COPY_CELLS`] cells.
    TooManyCells { cells: u128 },
    /// Refused: the text passed [`MAX_COPY_BYTES`].
    TooLarge { cells: u128 },
}

/// The text of a region passed the byte limit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TooLarge;

/// A region's TSV and how many of its cells are incomplete: a value cut at the cell cap (its
/// kept prefix is in the text) or a value that could not be decoded (an empty cell).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Tsv {
    pub text: String,
    pub incomplete: u64,
}

/// The TSV of `region` (view rows of `map`, module docs) of the result `adapter` shows now,
/// or [`TooLarge`] past `max_bytes`. Rows and columns the result does not have are left out.
pub fn tsv(
    adapter: &GridAdapter,
    region: &CopyRegion,
    map: &ViewMap,
    headers: bool,
    max_bytes: usize,
) -> Result<Tsv, TooLarge> {
    build_tsv(&adapter.raw_view(), region, map, headers, max_bytes)
}

/// The least text `region`'s TSV needs, without building it: every cell's known length (the
/// hex of a binary value, the bytes of a text) plus the separators. Numbers, dates and the
/// quoting are not counted, so the real text is at least this long. `region`'s rows are view
/// rows of `map`.
pub fn estimated_bytes(adapter: &GridAdapter, region: &CopyRegion, map: &ViewMap) -> usize {
    estimate(&adapter.raw_view(), region, map, usize::MAX)
}

// ## m3c-t36
/// The model rows `region`'s view rows show through `map`, in view order: rows the map does not
/// show, or that are not loaded (`loaded` model rows), are left out.
fn model_rows<'a>(
    region: &'a CopyRegion,
    map: &'a ViewMap,
    loaded: u64,
) -> impl Iterator<Item = u64> + 'a {
    region.rows.iter().flat_map(move |rows| {
        // The identity reads the range as it is (no lookup per row).
        let range = if map.is_identity() {
            rows.start.min(loaded)..rows.end.min(loaded)
        } else {
            rows.clone()
        };
        range.filter_map(move |r| {
            map.view_to_model(ViewRow(r))
                .map(|m| m.0)
                .filter(|m| *m < loaded)
        })
    })
}

/// [`estimated_bytes`] over `view`, stopping once it passes `stop_above`.
fn estimate(view: &RawView, region: &CopyRegion, map: &ViewMap, stop_above: usize) -> usize {
    let snapshot = &view.snapshot;
    let cols: Vec<u32> = region
        .cols
        .iter()
        .map(|c| c.0)
        .filter(|c| (*c as usize) < view.facts.len())
        .collect();
    let loaded = snapshot.row_count();
    let mut total = 0usize;
    for row in model_rows(region, map, loaded) {
        for col in &cols {
            total = total.saturating_add(known_len(snapshot.cell(row, *col)) + 1);
        }
        if total > stop_above {
            return total;
        }
    }
    total
}

/// The bytes a cell's raw text is known to need before it is built: exact for binary values
/// (`\x` and two digits per byte), a lower bound for text (quoting and escapes only add), 0
/// for values whose text is short (numbers, dates) or is not copied (a decode error).
fn known_len(value: ValueRef<'_>) -> usize {
    match value {
        ValueRef::Bytes(b) => 2 + 2 * b.len(),
        ValueRef::Text(t) | ValueRef::Json(t) | ValueRef::Xml(t) => t.len(),
        ValueRef::Opaque(o) => o.text.len(),
        ValueRef::Lossy(b) => b.len(),
        ValueRef::Truncated(t) => known_len(t.prefix.view()),
        _ => 0,
    }
}

/// The TSV of `region` over `view`'s snapshot. Pure: it runs on any thread.
///
/// ## m3c-t36: `region.rows` are view rows of `map`; each is read as the model row it shows.
pub(crate) fn build_tsv(
    view: &RawView,
    region: &CopyRegion,
    map: &ViewMap,
    headers: bool,
    max_bytes: usize,
) -> Result<Tsv, TooLarge> {
    let snapshot = &view.snapshot;
    let engine = view.display.engine();
    let zone = view.display.zone();
    let cols: Vec<u32> = region
        .cols
        .iter()
        .map(|c| c.0)
        .filter(|c| (*c as usize) < view.facts.len())
        .collect();
    let mut out = String::new();
    let mut incomplete = 0u64;
    if headers {
        for (i, col) in cols.iter().enumerate() {
            if i > 0 {
                out.push('\t');
            }
            push_cell(&mut out, &snapshot.schema().columns[*col as usize].name);
        }
        out.push('\n');
    }
    let loaded = snapshot.row_count();
    for row in model_rows(region, map, loaded) {
        for (i, col) in cols.iter().enumerate() {
            if i > 0 {
                out.push('\t');
            }
            let value = snapshot.cell(row, *col);
            match value {
                // Not a value the server sent: an empty cell, never the decoder's message.
                ValueRef::Error(_) => {
                    incomplete += 1;
                    continue;
                }
                // The kept prefix is all the result has of it.
                ValueRef::Truncated(_) => incomplete += 1,
                _ => {}
            }
            // Refused before the cell's text exists: a huge value is never expanded.
            if out.len().saturating_add(known_len(value)) > max_bytes {
                return Err(TooLarge);
            }
            let facts = view.facts[*col as usize].view(engine);
            match raw_text_in(value, &facts, zone) {
                RawText::Null => {}
                RawText::Text(text) => push_cell(&mut out, &text),
            }
        }
        out.push('\n');
        if out.len() > max_bytes {
            return Err(TooLarge);
        }
    }
    if out.len() > max_bytes {
        return Err(TooLarge);
    }
    Ok(Tsv {
        text: out,
        incomplete,
    })
}

/// One TSV cell: quoted when it holds a tab, a line break or a quote, or has outer white
/// space; verbatim otherwise.
fn push_cell(out: &mut String, text: &str) {
    let outer_space = text.chars().next().is_some_and(char::is_whitespace)
        || text.chars().next_back().is_some_and(char::is_whitespace);
    if !outer_space && !text.contains(['\t', '\n', '\r', '"']) {
        out.push_str(text);
        return;
    }
    out.push('"');
    for part in text.split_inclusive('"') {
        out.push_str(part);
        if part.ends_with('"') {
            out.push('"');
        }
    }
    out.push('"');
}

/// Copies `region` of `adapter`'s result (`GridEvent::CopyRequested`): the guards, the TSV
/// (in the background above [`BACKGROUND_COPY_CELLS`] cells or [`BACKGROUND_COPY_BYTES`]),
/// one clipboard item and the toast. The task resolves when the copy has ended.
///
/// `expected_generation` is the event's `generation`: the result the region was cut from.
/// When the adapter has moved on (a re-run), nothing is copied ([`CopyOutcome::Stale`]).
///
/// ## m3c-t36: `region`'s rows are view rows of `map` (the grid's map when it asked): the copy
/// holds the rows the user sees, in the order shown (module docs).
pub fn copy_region(
    adapter: &GridAdapter,
    region: CopyRegion,
    map: ViewMap,
    headers: bool,
    expected_generation: u64,
    cx: &mut App,
) -> Task<CopyOutcome> {
    if adapter.generation() != expected_generation {
        return Task::ready(CopyOutcome::Stale);
    }
    let cells = region.cell_count();
    if cells > MAX_COPY_CELLS {
        refuse_too_much(cells, cx);
        return Task::ready(CopyOutcome::TooManyCells { cells });
    }
    let view = adapter.raw_view();
    let rows = region.row_count();
    let cols = region.cols.len() as u64;
    let small = cells <= BACKGROUND_COPY_CELLS
        && estimate(&view, &region, &map, BACKGROUND_COPY_BYTES) <= BACKGROUND_COPY_BYTES;
    if small {
        let text = build_tsv(&view, &region, &map, headers, MAX_COPY_BYTES);
        return Task::ready(finish(text, rows, cols, cells, cx));
    }
    let working = ToastQueue::push_id(cx, working_toast(saturating_u64(cells)));
    let job = cx
        .background_spawn(async move { build_tsv(&view, &region, &map, headers, MAX_COPY_BYTES) });
    cx.spawn(async move |cx| {
        let text = job.await;
        cx.update(|cx| {
            ToastQueue::dismiss(cx, working);
            finish(text, rows, cols, cells, cx)
        })
    })
}

/// The grid refused a copy before asking (`GridEvent::CopyRefused`): the toast.
pub fn copy_refused(refusal: CopyRefusal, cx: &mut App) {
    match refusal {
        CopyRefusal::TooMany { cells } => refuse_too_much(cells, cx),
        CopyRefusal::MultipleSelection => {
            let mut toast = Toast::new(ToastKind::Info, text_fn(ids::hv::COPY_REFUSED_MULTIPLE));
            toast.wide = true;
            toast.icon = Some(IconName::CircleAlert);
            ToastQueue::push(cx, toast);
        }
    }
}

/// Puts the text on the clipboard and says so, or refuses a text past the byte limit.
fn finish(
    text: Result<Tsv, TooLarge>,
    rows: u64,
    cols: u64,
    cells: u128,
    cx: &mut App,
) -> CopyOutcome {
    let Ok(Tsv { text, incomplete }) = text else {
        refuse_too_much(cells, cx);
        return CopyOutcome::TooLarge { cells };
    };
    cx.write_to_clipboard(ClipboardItem::new_string(text));
    ToastQueue::push(cx, copied_toast(rows, cols, incomplete));
    CopyOutcome::Copied {
        rows,
        cols,
        incomplete,
    }
}

// ## m3a-t35-shots (named exception): the copy toasts, built apart from the clipboard write so
// the shots render them without touching the machine's clipboard (`fixtures` below).

/// "Copying 48,000 cells…" (a background copy).
fn working_toast(count: u64) -> Toast {
    Toast::new(
        ToastKind::Info,
        Rc::new(move |cx: &App| {
            tr(
                cx,
                ids::hv::COPY_WORKING,
                ids::hv::HvCopyWorking {
                    cells: Count(count),
                },
            )
        }),
    )
}

/// "Copied 3 × 8 cells" (or the incomplete-values warning).
fn copied_toast(rows: u64, cols: u64, incomplete: u64) -> Toast {
    let mut toast = if incomplete == 0 {
        let mut toast = Toast::new(ToastKind::Info, text_fn(ids::hv::COPY_DONE_LINE));
        toast.icon = Some(IconName::CopyCheck);
        toast
    } else {
        // Some of what was copied is not the whole value: say so instead of plain success.
        let mut toast = Toast::new(
            ToastKind::Info,
            Rc::new(move |cx: &App| {
                tr(
                    cx,
                    ids::hv::COPY_DONE_INCOMPLETE,
                    ids::hv::HvCopyDoneIncomplete {
                        n: Count(incomplete),
                    },
                )
            }),
        );
        toast.icon = Some(IconName::TriangleAlert);
        toast.wide = true;
        toast
    };
    toast.title = Some(Rc::new(move |cx: &App| {
        tr(
            cx,
            ids::hv::COPY_DONE,
            ids::hv::HvCopyDone {
                rows: Count(rows),
                cols: Count(cols),
            },
        )
    }));
    toast
}

/// The "Too much to copy" toast (NOTES surface 14): the selection's cells and both limits.
fn refuse_too_much(cells: u128, cx: &mut App) {
    let count = saturating_u64(cells);
    let mut toast = Toast::new(
        ToastKind::Info,
        Rc::new(move |cx: &App| {
            let size = Texts::of(cx).size(MAX_COPY_BYTES as u64);
            tr(
                cx,
                ids::hv::COPY_REFUSED_BODY,
                ids::hv::HvCopyRefusedBody {
                    cells: Count(count),
                    max: Count(saturating_u64(MAX_COPY_CELLS)),
                    size: Text(size),
                },
            )
        }),
    );
    toast.title = Some(text_fn(ids::hv::COPY_REFUSED_TITLE));
    toast.wide = true;
    toast.icon = Some(IconName::CircleAlert);
    ToastQueue::push(cx, toast);
}

fn saturating_u64(n: u128) -> u64 {
    u64::try_from(n).unwrap_or(u64::MAX)
}

// ## m3a-t35-shots: the copy feedback's shots fixtures (NOTES surface 14).
/// The toasts of NOTES surface 14 without a clipboard write (feature `shots`): the shots'
/// headless platform may share the machine's clipboard, which a shot must never overwrite.
#[cfg(feature = "shots")]
pub mod fixtures {
    use super::*;

    /// "Copied `rows` × `cols` cells", as a copy of that block says it.
    pub fn show_copied(rows: u64, cols: u64, cx: &mut App) {
        ToastQueue::push(cx, copied_toast(rows, cols, 0));
    }

    /// "Copying `cells` cells…", as a background copy says it while it runs.
    pub fn show_copying(cells: u64, cx: &mut App) {
        ToastQueue::push(cx, working_toast(cells));
    }
}
