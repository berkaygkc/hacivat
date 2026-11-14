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
//! - **A truncated value** copies the prefix the result kept, without a marker.
//! - **A decode error** copies its message; a cell the row did not carry copies as empty.
//! - **Control characters** (bidi overrides, tabs, line breaks inside a value) are kept
//!   verbatim; the grid's chips are display only.
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
//! a toast (export is M6); nothing is put on the clipboard. Above [`BACKGROUND_COPY_CELLS`]
//! cells the text is built on the background executor, from the snapshot taken at the
//! request, and the UI thread only writes the clipboard. The text is one
//! `ClipboardItem::new_string`.

use std::rc::Rc;

use hacivat_l10n::{Count, Text, ids};
use hacivat_ui::icon::IconName;
use hacivat_ui::toast::{Toast, ToastKind, ToastQueue};
use hacivat_ui_core::gpui_surface::{App, AppContext as _, ClipboardItem, Task};
use hacivat_ui_grid::model::{CopyRefusal, CopyRegion};
use hacivat_values::valfmt::{RawText, raw_text_in};

use crate::l10n_global::{text_fn, tr};
use crate::results::adapter::{GridAdapter, RawView, Texts};

/// The most cells a copy takes (HV-XFER-02 step 2).
pub const MAX_COPY_CELLS: u128 = hacivat_ui_grid::model::MAX_COPY_CELLS;

/// The most text a copy takes: 100 MB.
pub const MAX_COPY_BYTES: usize = 100 << 20;

/// Above this many cells the text is built on the background executor.
pub const BACKGROUND_COPY_CELLS: u128 = 10_000;

/// How a copy ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CopyOutcome {
    /// The clipboard holds `rows` × `cols` cells.
    Copied { rows: u64, cols: u64 },
    /// Refused: more than [`MAX_COPY_CELLS`] cells.
    TooManyCells { cells: u128 },
    /// Refused: the text passed [`MAX_COPY_BYTES`].
    TooLarge { cells: u128 },
}

/// The text of a region passed the byte limit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TooLarge;

/// The TSV of `region` of the result `adapter` shows now (module docs), or [`TooLarge`] past
/// `max_bytes`. Rows and columns the result does not have are left out.
pub fn tsv(
    adapter: &GridAdapter,
    region: &CopyRegion,
    headers: bool,
    max_bytes: usize,
) -> Result<String, TooLarge> {
    build_tsv(&adapter.raw_view(), region, headers, max_bytes)
}

/// The TSV of `region` over `view`'s snapshot. Pure: it runs on any thread.
pub(crate) fn build_tsv(
    view: &RawView,
    region: &CopyRegion,
    headers: bool,
    max_bytes: usize,
) -> Result<String, TooLarge> {
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
    for rows in &region.rows {
        for row in rows.start.min(loaded)..rows.end.min(loaded) {
            for (i, col) in cols.iter().enumerate() {
                if i > 0 {
                    out.push('\t');
                }
                let facts = view.facts[*col as usize].view(engine);
                match raw_text_in(snapshot.cell(row, *col), &facts, zone) {
                    RawText::Null => {}
                    RawText::Text(text) => push_cell(&mut out, &text),
                }
            }
            out.push('\n');
            if out.len() > max_bytes {
                return Err(TooLarge);
            }
        }
    }
    if out.len() > max_bytes {
        return Err(TooLarge);
    }
    Ok(out)
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
/// (in the background above [`BACKGROUND_COPY_CELLS`] cells), one clipboard item and the
/// toast. The task resolves when the copy has ended.
pub fn copy_region(
    adapter: &GridAdapter,
    region: CopyRegion,
    headers: bool,
    cx: &mut App,
) -> Task<CopyOutcome> {
    let cells = region.cell_count();
    if cells > MAX_COPY_CELLS {
        refuse_too_much(cells, cx);
        return Task::ready(CopyOutcome::TooManyCells { cells });
    }
    let view = adapter.raw_view();
    let rows = region.row_count();
    let cols = region.cols.len() as u64;
    if cells <= BACKGROUND_COPY_CELLS {
        let text = build_tsv(&view, &region, headers, MAX_COPY_BYTES);
        return Task::ready(finish(text, rows, cols, cells, cx));
    }
    let count = saturating_u64(cells);
    let working = ToastQueue::push_id(
        cx,
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
        ),
    );
    let job =
        cx.background_spawn(async move { build_tsv(&view, &region, headers, MAX_COPY_BYTES) });
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
    text: Result<String, TooLarge>,
    rows: u64,
    cols: u64,
    cells: u128,
    cx: &mut App,
) -> CopyOutcome {
    let Ok(text) = text else {
        refuse_too_much(cells, cx);
        return CopyOutcome::TooLarge { cells };
    };
    cx.write_to_clipboard(ClipboardItem::new_string(text));
    let mut toast = Toast::new(ToastKind::Info, text_fn(ids::hv::COPY_DONE_LINE));
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
    toast.icon = Some(IconName::CopyCheck);
    ToastQueue::push(cx, toast);
    CopyOutcome::Copied { rows, cols }
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
