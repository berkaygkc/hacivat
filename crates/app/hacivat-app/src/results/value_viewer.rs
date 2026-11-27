//! The value viewer (HV-GRID-12, R-M3-82; M3c Task 38): a 480 px popover with the column and
//! type, the wrapped value, a pretty toggle for JSON and XML, hex for binary, Copy raw value,
//! and "Re-run with a 64 MiB value limit" for a truncated value.
//!
//! ## m3b-t01b-seams: the seam (Task 1b). Task 38 fills it.
//!
//! ## m3c-t38 (M3c Task 38; NOTES surface 22, M3c-31, M3c-32)
//! - **One value, read whole, from the model.** The viewer opens on a **model** cell (the grid's
//!   focus is a model cell, so a sort or a filter never changes which value it reads) of the
//!   console's active result, from the snapshot the adapter holds, and closes when the result
//!   moves on (a re-run, another tab, another view). It is non-modal: anchored under its cell in
//!   **the view shown** (the table, the Transposed view's grid; the text view has no cells, so
//!   the viewer sits at the top of its area), above when there is no room, clamped inside the
//!   grid's area, never over the console toolbar; Escape, × or a click outside closes it and
//!   gives the focus back to the view shown.
//! - **Raw bytes are copied, never the display.** The well shows the value's raw channel
//!   (`valfmt::raw_text_in`: the server's text, ungrouped numbers, bytes as hex). **Pretty**
//!   (JSON and XML) re-indents for display only: JSON keeps its key order and every token as the
//!   server wrote it (only white space outside strings changes), and a value that does not parse
//!   keeps Raw with Pretty disabled. **Copy raw value** always copies the raw text, whatever is
//!   shown: pretty-printing never changes copied bytes. A NULL or a decode error has nothing to
//!   copy.
//! - **Hex** for binary: the offset (`text-3`), 8 bytes per row in 2-byte groups (the popover is
//!   under 560 px), then the ASCII column (`text-2`).
//! - **Nothing heavy on the UI thread** (fix round 1, I2). Opening reads only the cell's shape
//!   (its kind and lengths). The form (JSON validity), the line cuts, the pretty text, the
//!   colour runs and the character count are made on the background executor; the text is read
//!   in place from the result's snapshot (one copy of the value: the store's), a pretty form is
//!   the one owned copy, and hex rows are formatted only when the well shows them. Copy raw
//!   value builds its text off the UI thread too.
//! - **Truncated** (the store cut the value at the console's value limit): "Showing the first
//!   1 MB of 5.2 MB." and **Re-run with a 64 MB value limit**, offered while the kept part is
//!   under the largest limit. It emits `ResultsEvent::RerunWithLimit`, which the run UI handles
//!   like Re-run (`orchestrator::rerun`): only the text that ran, still in the editor, through
//!   Run's pipeline (parameters, classification of the bytes sent, the gate, the
//!   confirmations). **Fix round 1 (I1, controller ruling, fail closed):** only a read is run
//!   again; anything else is refused before it is sent and the viewer says why
//!   (`hv-vv-rerun-not-read`). While the console runs a statement the button is disabled, with
//!   the reason in its tooltip (m7). The viewer stays open; the new result closes it.

use std::cell::Cell;
use std::collections::HashMap;
use std::ops::Range;
use std::rc::{Rc, Weak};
use std::sync::Arc;

use hacivat_l10n::ArgValue;
use hacivat_ui::button::{Button, ButtonKind};
use hacivat_ui::icon::IconName;
use hacivat_ui::popover::{A11yNode, Declare as _, Tip};
use hacivat_ui::segmented::{SegmentContent, SegmentItem, Segmented};
use hacivat_ui::text::{Label, TextSource};
use hacivat_ui::toast::{Toast, ToastKind, ToastQueue};
use hacivat_ui_core::fonts::font_for;
use hacivat_ui_core::gpui_surface::{
    AnyElement, App, AppContext as _, ClipboardItem, Context, Entity, FocusHandle, Focusable as _,
    Global, HighlightStyle, Hsla, InteractiveElement as _, IntoElement, KeyDownEvent,
    ParentElement as _, Render, Role, ScrollHandle, SharedString, StatefulInteractiveElement as _,
    Styled as _, StyledText, Task, WeakEntity, Window, div, point, px,
};
use hacivat_ui_core::theme::{Theme, ThemeMode, focus_outline, popover_surface};
use hacivat_ui_grid::{
    GridDataSource as _, GridView, ModelCell, RowRenderer, ViewRow, VirtualList,
};
use hacivat_values::valfmt::{RawText, raw_text_in};
use hacivat_values::{LenUnit, LogicalType, ValueRef};

use crate::console::ConsoleId;
use crate::exec_ui::marks::message;
use crate::results::adapter::{GridAdapter, RawView, Texts};
use crate::results::view::ResultsEvent;

/// The popover's width (NOTES surface 22).
pub const VIEWER_WIDTH_PX: f32 = 480.0;
/// The value well's tallest and shortest (it shrinks before the popover would cross the
/// console toolbar, then scrolls).
pub const WELL_MAX_PX: f32 = 240.0;
pub const WELL_MIN_PX: f32 = 120.0;
/// Hex rows below a 560 px popover (NOTES surface 22).
pub const HEX_BYTES_PER_ROW: usize = 8;
/// A line of the well (Fragment Mono 11.5/17).
const LINE_PX: f32 = 17.0;
/// The well's padding (7/10) and the popover's (16 at the sides).
const WELL_PAD_X: f32 = 10.0;
const WELL_PAD_Y: f32 = 7.0;
const POPOVER_PAD_X: f32 = 16.0;
/// The rest of the popover around the well, for placing it (header, control row, notice,
/// footer); the well takes what is left.
const CHROME_PX: f32 = 170.0;
// ## m3c-shotfix F8 (fix round 1, m10): the paddings the popover's rows are laid out with.
// The room check adds them to the type ramp's line heights and the control heights
// ([`Rows`]), so a token change moves the layout and the check together; a test pins the
// check to the drawn rows (`results_views.rs`).
const HEADER_PAD_TOP: f32 = 12.0;
const FOOTER_PAD_TOP: f32 = 12.0;
const FOOTER_PAD_BOTTOM: f32 = 14.0;
/// Above the control row, the well and the notice.
const ROW_GAP: f32 = 8.0;
/// Between the notice and Re-run.
const NOTICE_GAP: f32 = 6.0;
/// The short layout's header and footer padding, and its gap above the well.
const SHORT_PAD: f32 = 6.0;
const SHORT_WELL_GAP: f32 = 4.0;
// ## inputnav-1 (named exception, F2)
/// At the bottom of the scrolling middle: the focus ring of what comes last (the well, Re-run)
/// reaches this far below it, and the middle's clip cut it there.
const WELL_RING_ROOM: f32 = hacivat_ui_core::theme::surfaces::FOCUS_RING_EXTENT_PX;

/// The heights of the popover's rows in one layout (fix round 1, I1, m10).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rows {
    pub header: f32,
    pub control: f32,
    /// The well at one line, with its gap above.
    pub well_min: f32,
    pub notice: f32,
    pub footer: f32,
}

impl Rows {
    /// The rows of the roomy layout (`short: false`) or the short one, from the tokens;
    /// `form`: the value has a form switch (JSON, XML, binary).
    fn of(
        t: &hacivat_ui_core::theme::ThemeTokens,
        short: bool,
        form: bool,
        window: &Window,
    ) -> Rows {
        let lh = |style: hacivat_ui_core::theme::TypeStyle| {
            f32::from(window.pixel_snap(px(style.line_height)))
        };
        let control = t.spacing.control_height;
        let field = t.spacing.field_height;
        let compact = hacivat_ui::metrics::COMPACT_HEIGHT_PX;
        let well = LINE_PX + 2.0 * WELL_PAD_Y + 2.0;
        if short {
            Rows {
                // The column name, the form switch and the compact ×, on one row.
                header: SHORT_PAD
                    + lh(t.type_ramp.code)
                        .max(if form { field } else { 0.0 })
                        .max(compact),
                control: 0.0,
                well_min: SHORT_WELL_GAP + well,
                notice: ROW_GAP + lh(t.type_ramp.caption) + NOTICE_GAP + control,
                // The size and the compact Copy raw value (its glyph).
                footer: SHORT_PAD + lh(t.type_ramp.mono_data).max(compact) + SHORT_PAD,
            }
        } else {
            Rows {
                header: HEADER_PAD_TOP
                    + (lh(t.type_ramp.code) + lh(t.type_ramp.body_sm)).max(control),
                control: ROW_GAP + field,
                well_min: ROW_GAP + well,
                notice: ROW_GAP + lh(t.type_ramp.caption) + NOTICE_GAP + control,
                footer: FOOTER_PAD_TOP + lh(t.type_ramp.mono_data).max(control) + FOOTER_PAD_BOTTOM,
            }
        }
    }

    /// The popover's height with a one-line well.
    fn total(&self, has_control: bool, notice: bool) -> f32 {
        self.header
            + if has_control { self.control } else { 0.0 }
            + self.well_min
            + if notice { self.notice } else { 0.0 }
            + self.footer
    }
}

/// The gap between the cell and the popover, and from the grid's edges.
const GAP_PX: f32 = 4.0;

/// How the viewer shows a value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ValueForm {
    /// Wrapped text (numbers, dates, text, NULL, a decode error).
    Text,
    /// JSON: Raw / Pretty; `valid: false` keeps Raw with Pretty disabled.
    Json { valid: bool },
    /// XML: Raw / Pretty.
    Xml,
    /// Bytes as hex.
    Binary,
}

/// What the viewer shows (tests and the shots read it).
#[derive(Debug, Clone, PartialEq)]
pub struct ViewerView {
    /// The column's name (data).
    pub column: SharedString,
    /// "jsonb · row 4".
    pub subtitle: SharedString,
    pub form: ValueForm,
    /// Pretty is pressed.
    pub pretty: bool,
    /// The well's text, as shown (pretty or raw; hex rows for bytes), once the background
    /// load landed (empty before).
    pub body: String,
    /// "48 KB · 48,213 characters", "512 bytes" (empty before the load landed).
    pub footer: SharedString,
    /// The truncation notice.
    pub truncated: Option<SharedString>,
    /// The re-run button's words when it is offered.
    pub rerun: Option<SharedString>,
    /// Why the re-run is off now (a run is busy, or it was refused).
    pub rerun_off: Option<SharedString>,
    /// Copy raw value is enabled (not for NULL or a decode error).
    pub can_copy: bool,
}

/// The kind of value, read from its variant (no pass over its bytes).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Shape {
    /// Text read in place (`Text`, `Json`, `Xml`, a cut prefix of one).
    Borrowed {
        json: bool,
        xml: bool,
    },
    Bytes,
    Null,
    Error,
    /// A number, date, UUID…: a short raw text.
    Scalar,
}

/// What the viewer reads once per open: the cell's shape and the header's facts.
#[derive(Clone)]
struct Meta {
    view: RawView,
    cell: ModelCell,
    shape: Shape,
    column: SharedString,
    subtitle: SharedString,
    // ## m3c-int (M3)
    /// The column's type label: the subtitle is made again where the table shows the row now.
    type_label: SharedString,
    truncated: Option<SharedString>,
    rerun: Option<SharedString>,
    /// The kept part's bytes, when the store cut the value.
    kept: Option<u64>,
    unit_ordinal: u32,
}

/// What the background load made.
#[derive(Clone)]
struct Body {
    form: ValueForm,
    pretty: bool,
    /// Characters (text) or bytes (binary) of what the result holds.
    count: u64,
    bytes: u64,
}

/// The token kinds the well colours.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Tok {
    Key,
    Str,
    Num,
    Lit,
    Tag,
    Attr,
    /// Hex offsets.
    Dim,
    /// Hex ASCII.
    Soft,
}

/// Where the well's text lives.
#[derive(Clone, Default)]
enum WellSrc {
    #[default]
    Empty,
    /// The cell's text, read in place from the snapshot.
    Cell(RawView, ModelCell),
    /// A text the load made (a pretty form, a scalar, a decode error's message).
    Owned(Arc<str>),
    /// The cell's bytes, a hex row formatted when shown.
    Hex(RawView, ModelCell),
}

/// The text of `cell` read in place: a text value (or the kept prefix of one).
fn cell_str(view: &RawView, cell: ModelCell) -> &str {
    match view.snapshot.cell(cell.row.0, cell.col.0) {
        ValueRef::Text(t) | ValueRef::Json(t) | ValueRef::Xml(t) => t,
        ValueRef::Truncated(t) => match t.prefix.view() {
            ValueRef::Text(t) | ValueRef::Json(t) | ValueRef::Xml(t) => t,
            _ => "",
        },
        _ => "",
    }
}

/// The bytes of `cell` read in place (or the kept prefix).
fn cell_bytes(view: &RawView, cell: ModelCell) -> &[u8] {
    match view.snapshot.cell(cell.row.0, cell.col.0) {
        ValueRef::Bytes(b) => b,
        ValueRef::Truncated(t) => match t.prefix.view() {
            ValueRef::Bytes(b) => b,
            _ => &[],
        },
        _ => &[],
    }
}

/// The well's lines: where the text lives, each wrapped line's byte range (or the hex rows),
/// and the coloured runs.
#[derive(Clone, Default)]
struct WellLines {
    src: WellSrc,
    lines: Arc<[Range<usize>]>,
    tokens: Arc<[(Range<usize>, Tok)]>,
}

impl WellLines {
    /// The whole text as shown (tests; small values).
    fn full_text(&self) -> String {
        match &self.src {
            WellSrc::Empty => String::new(),
            WellSrc::Cell(view, cell) => cell_str(view, *cell).to_string(),
            WellSrc::Owned(t) => t.to_string(),
            WellSrc::Hex(view, cell) => {
                hex_lines(cell_bytes(view, *cell), HEX_BYTES_PER_ROW).join("\n")
            }
        }
    }
}

impl RowRenderer for WellLines {
    fn len(&self) -> u64 {
        match &self.src {
            WellSrc::Hex(view, cell) => {
                cell_bytes(view, *cell).len().div_ceil(HEX_BYTES_PER_ROW) as u64
            }
            WellSrc::Empty => 0,
            _ => self.lines.len() as u64,
        }
    }

    fn row(&self, ix: u64, _window: &mut Window, cx: &mut App) -> AnyElement {
        let theme = *Theme::get(cx);
        let t = theme.tokens;
        let c = &t.colors;
        let (text, highlights): (String, Vec<(Range<usize>, HighlightStyle)>) = match &self.src {
            WellSrc::Empty => return div().into_any_element(),
            WellSrc::Hex(view, cell) => {
                let line = hex_row(cell_bytes(view, *cell), ix as usize, HEX_BYTES_PER_ROW);
                let style = |tok| HighlightStyle {
                    color: Some(token_color(tok, &theme)),
                    ..HighlightStyle::default()
                };
                let ascii = hex_ascii_at(HEX_BYTES_PER_ROW);
                let h = vec![
                    (0..8, style(Tok::Dim)),
                    (ascii.min(line.len())..line.len(), style(Tok::Soft)),
                ];
                (line, h)
            }
            src => {
                let Some(range) = self.lines.get(ix as usize).cloned() else {
                    return div().into_any_element();
                };
                let whole: &str = match src {
                    WellSrc::Cell(view, cell) => cell_str(view, *cell),
                    WellSrc::Owned(s) => s,
                    _ => "",
                };
                let text = whole.get(range.clone()).unwrap_or_default().to_string();
                // The coloured runs inside this line (tokens are sorted and do not overlap).
                let first = self.tokens.partition_point(|(r, _)| r.end <= range.start);
                let mut highlights = Vec::new();
                for (r, tok) in self.tokens[first..].iter() {
                    if r.start >= range.end {
                        break;
                    }
                    let (s, e) = (r.start.max(range.start), r.end.min(range.end));
                    if s < e {
                        highlights.push((
                            s - range.start..e - range.start,
                            HighlightStyle {
                                color: Some(token_color(*tok, &theme)),
                                ..HighlightStyle::default()
                            },
                        ));
                    }
                }
                (text, highlights)
            }
        };
        let style = t.type_ramp.mono_data;
        div()
            .h(px(LINE_PX))
            .whitespace_nowrap()
            .font(font_for(&style))
            .text_size(px(style.size_px))
            .line_height(px(LINE_PX))
            .text_color(c.text.to_hsla())
            .child(StyledText::new(SharedString::from(text)).with_highlights(highlights))
            .into_any_element()
    }
}

fn token_color(tok: Tok, theme: &Theme) -> Hsla {
    let c = &theme.tokens.colors;
    match tok {
        Tok::Key | Tok::Attr => c.syn_identifier,
        Tok::Str => c.syn_string,
        Tok::Num => c.syn_number,
        Tok::Lit | Tok::Tag => c.syn_keyword,
        Tok::Dim => c.text_3,
        Tok::Soft => c.text_2,
    }
    .to_hsla()
}

/// The background load (pure; any thread): the form, the well's source and cuts, the colour
/// runs and the count.
fn load_body(
    meta_view: RawView,
    cell: ModelCell,
    shape: Shape,
    pretty: bool,
    cols: usize,
) -> (Body, WellLines) {
    let view = meta_view;
    match shape {
        Shape::Bytes => {
            let n = cell_bytes(&view, cell).len() as u64;
            (
                Body {
                    form: ValueForm::Binary,
                    pretty: false,
                    count: n,
                    bytes: n,
                },
                WellLines {
                    src: WellSrc::Hex(view, cell),
                    lines: Arc::from([]),
                    tokens: Arc::from([]),
                },
            )
        }
        Shape::Borrowed { json, xml } => {
            let text = cell_str(&view, cell);
            let cut = matches!(
                view.snapshot.cell(cell.row.0, cell.col.0),
                ValueRef::Truncated(_)
            );
            let form = if json {
                // A cut JSON value never parses: it stays Raw.
                ValueForm::Json {
                    valid: !cut && is_json(text),
                }
            } else if xml {
                ValueForm::Xml
            } else {
                ValueForm::Text
            };
            let count = text.chars().count() as u64;
            let bytes = text.len() as u64;
            let pretty_text = match (pretty, form) {
                (true, ValueForm::Json { valid: true }) => pretty_json(text).map(Arc::<str>::from),
                (true, ValueForm::Xml) => Some(Arc::<str>::from(pretty_xml(text))),
                _ => None,
            };
            let shown = pretty_text.as_deref().unwrap_or(text);
            let lines = wrap_lines(shown, cols);
            let tokens = match form {
                ValueForm::Json { .. } => json_tokens(shown),
                ValueForm::Xml => xml_tokens(shown),
                _ => Vec::new(),
            };
            let is_pretty = pretty_text.is_some();
            let src = match pretty_text {
                Some(p) => WellSrc::Owned(p),
                None => WellSrc::Cell(view.clone(), cell),
            };
            (
                Body {
                    form,
                    pretty: is_pretty,
                    count,
                    bytes,
                },
                WellLines {
                    src,
                    lines: lines.into(),
                    tokens: tokens.into(),
                },
            )
        }
        Shape::Null | Shape::Error | Shape::Scalar => {
            let value = view.snapshot.cell(cell.row.0, cell.col.0);
            let text: String = match value {
                ValueRef::Null => hacivat_ui_grid::NULL_MARKER.to_string(),
                ValueRef::Error(e) => e.message.to_string(),
                v => {
                    let facts = view.facts.get(cell.col.0 as usize);
                    match facts.map(|f| {
                        raw_text_in(v, &f.view(view.display.engine()), view.display.zone())
                    }) {
                        Some(RawText::Text(t)) => t.into_owned(),
                        _ => String::new(),
                    }
                }
            };
            let count = text.chars().count() as u64;
            let bytes = text.len() as u64;
            let lines = wrap_lines(&text, cols);
            (
                Body {
                    form: ValueForm::Text,
                    pretty: false,
                    count,
                    bytes,
                },
                WellLines {
                    src: WellSrc::Owned(Arc::from(text)),
                    lines: lines.into(),
                    tokens: Arc::from([]),
                },
            )
        }
    }
}

/// One console's value viewer.
pub struct ValueViewer {
    console: ConsoleId,
    open: bool,
    take_focus: bool,
    cell: Option<ModelCell>,
    adapter: Option<Weak<GridAdapter>>,
    /// The adapter generation the cell was read from: a re-run closes the viewer.
    generation: u64,
    /// The grid the viewer is anchored to (the view shown: the table's, or the Transposed
    /// view's) and the cell there.
    anchor: Option<(WeakEntity<GridView>, ModelCell)>,
    /// Where the focus goes back to when the viewer closes (the view shown).
    back: Option<FocusHandle>,
    meta: Option<Meta>,
    body: Option<Body>,
    /// Pretty asked (the load answers whether it applies).
    pretty: bool,
    /// The re-run was refused: why (fix round 1, I1).
    refused: Option<SharedString>,
    well: Entity<VirtualList<WellLines>>,
    /// The width the well's lines were cut for (characters per line), and Pretty.
    cut_for: Option<(usize, bool)>,
    loading: Option<Task<()>>,
    focus: FocusHandle,
    /// The char advance of the well's font, measured once.
    advance: Cell<Option<f32>>,
    // ## m3c-shotfix F8 (fix round 1, I1)
    /// The scroll of what lies between the header and the footer (a short room).
    middle_scroll: ScrollHandle,
    /// Re-run's focus, so a focused Re-run is scrolled into view.
    rerun_focus: FocusHandle,
    // ## m4-t33d (R-T33-1, NOTES 22): Copy raw value's (a Tab stop) and the ×'s (not one),
    // owned here so the tests can name them.
    copy_focus: FocusHandle,
    close_focus: FocusHandle,
    /// What had the focus at the last frame: 0 nothing here, 1 the well, 2 Re-run, 3 the
    /// refusal.
    seen_focus: Cell<u8>,
    // ## m3c-shotfix F8 (fix round 2, I-1)
    /// The refusal line takes the focus when it appears (Re-run turns off under the focus).
    refused_focus: FocusHandle,
    /// A refusal arrived: the next frame focuses it and scrolls it into view.
    focus_refusal: bool,
    /// The layout the last frame drew: short, and its rows (for tests).
    drawn_rows: Cell<Option<(bool, Rows)>>,
}

#[derive(Default)]
struct Viewers(HashMap<ConsoleId, Entity<ValueViewer>>);

impl Global for Viewers {}

fn viewer(console: ConsoleId, cx: &App) -> Option<Entity<ValueViewer>> {
    cx.try_global::<Viewers>()?.0.get(&console).cloned()
}

fn ensure(console: ConsoleId, cx: &mut App) -> Entity<ValueViewer> {
    if let Some(v) = viewer(console, cx) {
        return v;
    }
    let v = cx.new(|cx| ValueViewer::new(console, cx));
    cx.default_global::<Viewers>().0.insert(console, v.clone());
    v
}

/// Opens the viewer on `cell` of console `console`'s active result (the tile calls it on
/// `GridEvent::ValueRequested`, the record inspector's Show more), anchored to that cell in
/// the view shown (fix round 1, I3).
pub fn open(console: ConsoleId, cell: ModelCell, cx: &mut App) {
    let (anchor, back) = super::views::anchor_for(console, cell, cx);
    open_anchored(console, cell, anchor, back, cx);
}

/// [`open`] anchored to `anchor` (a grid and its cell that shows `cell`), the focus going back
/// to `back` when it closes.
pub(crate) fn open_anchored(
    console: ConsoleId,
    cell: ModelCell,
    anchor: Option<(WeakEntity<GridView>, ModelCell)>,
    back: Option<FocusHandle>,
    cx: &mut App,
) {
    let Some((table, adapter)) = super::find::active_target(console, cx) else {
        return;
    };
    let unit_ordinal = crate::console::Consoles::get(cx, console)
        .and_then(|s| s.results.clone())
        .and_then(|r| r.read(cx).active_tab().map(|t| t.key().unit_ordinal))
        .unwrap_or(0);
    let Some(meta) = read_meta(&adapter, &table, cell, unit_ordinal, cx) else {
        return;
    };
    let v = ensure(console, cx);
    v.update(cx, |v, cx| {
        v.open = true;
        v.take_focus = true;
        v.cell = Some(cell);
        v.adapter = Some(Rc::downgrade(&adapter));
        v.generation = adapter.generation();
        v.anchor = anchor;
        v.back = back;
        v.pretty = false;
        v.refused = None;
        v.meta = Some(meta);
        v.body = None;
        v.cut_for = None;
        v.loading = None;
        v.well
            .update(cx, |w, cx| *w.rows_mut(cx) = WellLines::default());
        cx.notify();
    });
    notify_hosts(cx);
}

/// The cell's shape and the header's facts: O(1) reads of the value (its variant and lengths).
fn read_meta(
    adapter: &Rc<GridAdapter>,
    table: &Entity<GridView>,
    cell: ModelCell,
    unit_ordinal: u32,
    cx: &App,
) -> Option<Meta> {
    let view = adapter.raw_view();
    let col = cell.col.0 as usize;
    if cell.row.0 >= view.snapshot.row_count() || col >= view.facts.len() {
        return None;
    }
    let logical = view.facts[col].logical.clone();
    let value = view.snapshot.cell(cell.row.0, cell.col.0);
    let (inner, cut) = match value {
        ValueRef::Truncated(t) => (t.prefix.view(), Some((t.total_len, t.unit))),
        v => (v, None),
    };
    let (shape, kept) = match (inner, &logical) {
        (ValueRef::Bytes(b), _) => (Shape::Bytes, b.len() as u64),
        (ValueRef::Null, _) => (Shape::Null, 0),
        (ValueRef::Error(_), _) => (Shape::Error, 0),
        (ValueRef::Json(t), _) => (
            Shape::Borrowed {
                json: true,
                xml: false,
            },
            t.len() as u64,
        ),
        (ValueRef::Xml(t), _) => (
            Shape::Borrowed {
                json: false,
                xml: true,
            },
            t.len() as u64,
        ),
        (ValueRef::Text(t), LogicalType::Json { .. }) => (
            Shape::Borrowed {
                json: true,
                xml: false,
            },
            t.len() as u64,
        ),
        (ValueRef::Text(t), LogicalType::Xml) => (
            Shape::Borrowed {
                json: false,
                xml: true,
            },
            t.len() as u64,
        ),
        (ValueRef::Text(t), _) => (
            Shape::Borrowed {
                json: false,
                xml: false,
            },
            t.len() as u64,
        ),
        _ => (Shape::Scalar, 0),
    };
    let texts = Texts::of(cx);
    let cap = u64::from(hacivat_results::window::MAX_CELL_CAP);
    let truncated = cut.map(|(total, unit)| {
        let shown = texts.size(kept);
        match total {
            Some(total) => {
                let size = match unit {
                    LenUnit::Bytes => texts.size(total),
                    LenUnit::Chars => texts.tr(
                        hacivat_l10n::ids::hv::GRID_SIZE_CHARS,
                        hacivat_l10n::ids::hv::HvGridSizeChars {
                            n: hacivat_l10n::Count(total),
                        },
                    ),
                };
                message(
                    cx,
                    "hv-vv-truncated",
                    vec![
                        ("shown", ArgValue::Text(shown)),
                        ("size", ArgValue::Text(size)),
                    ],
                )
            }
            None => message(
                cx,
                "hv-vv-truncated-unknown",
                vec![("shown", ArgValue::Text(shown))],
            ),
        }
    });
    // Re-run helps while the kept part is under the largest limit.
    let rerun = (cut.is_some() && kept < cap).then(|| {
        message(
            cx,
            "hv-vv-rerun",
            vec![("limit", ArgValue::Text(texts.size(cap)))],
        )
    });
    let (column, type_label) = adapter
        .columns()
        .get(col)
        .map(|c| (c.name.clone(), c.type_label.clone()))
        .unwrap_or_default();
    let subtitle = subtitle_of(&type_label, cell, table, cx);
    Some(Meta {
        view,
        cell,
        shape,
        column,
        subtitle,
        type_label,
        truncated,
        rerun,
        kept: cut.map(|_| kept),
        unit_ordinal,
    })
}

// ## m3c-int (M3)
/// "jsonb · row 4": `cell`'s row where `table` shows it now (1-based).
fn subtitle_of(
    type_label: &SharedString,
    cell: ModelCell,
    table: &Entity<GridView>,
    cx: &App,
) -> SharedString {
    let row = table
        .read(cx)
        .view_map()
        .model_to_view(cell.row)
        .map_or(cell.row.0, |v: ViewRow| v.0)
        + 1;
    message(
        cx,
        "hv-vv-sub",
        vec![
            ("type", ArgValue::Text(type_label.to_string())),
            ("row", ArgValue::Count(row)),
        ],
    )
}

/// Whether console `console`'s viewer is open.
// ## m3c-shotfix F8 (fix round 1, m10)
/// The layout the open viewer of `console` drew last: whether it is the short one, and the
/// heights of its rows as the room check reckons them.
pub fn drawn_rows(console: ConsoleId, cx: &App) -> Option<(bool, Rows)> {
    viewer(console, cx).and_then(|v| v.read(cx).drawn_rows.get())
}

// ## m3c-shotfix F8 (fix round 2, I-1)
/// What held the focus in the open viewer of `console` at its last frame: 0 nothing of it,
/// 1 the well, 2 Re-run, 3 the refusal line.
pub fn focused_part(console: ConsoleId, cx: &App) -> u8 {
    viewer(console, cx).map_or(0, |v| v.read(cx).seen_focus.get())
}

// ## m4-t33d (R-T33-1)
/// The open viewer's Re-run, Copy raw value and × focus handles (tests: the Tab order).
pub fn tab_handles(console: ConsoleId, cx: &App) -> Option<[FocusHandle; 3]> {
    let v = viewer(console, cx)?;
    let v = v.read(cx);
    Some([
        v.rerun_focus.clone(),
        v.copy_focus.clone(),
        v.close_focus.clone(),
    ])
}

pub fn is_open(console: ConsoleId, cx: &App) -> bool {
    viewer(console, cx).is_some_and(|v| v.read(cx).open)
}

/// What console `console`'s open viewer shows.
pub fn view(console: ConsoleId, cx: &App) -> Option<ViewerView> {
    let v = viewer(console, cx).filter(|v| v.read(cx).open)?;
    Some(v.read(cx).view_of(true, cx))
}

/// The cell the open viewer reads.
pub fn cell(console: ConsoleId, cx: &App) -> Option<ModelCell> {
    let v = viewer(console, cx).filter(|v| v.read(cx).open)?;
    v.read(cx).cell
}

/// The grid the open viewer is anchored to, and the cell there.
pub fn anchor(console: ConsoleId, cx: &App) -> Option<(Entity<GridView>, ModelCell)> {
    let v = viewer(console, cx).filter(|v| v.read(cx).open)?;
    let (g, c) = v.read(cx).anchor.clone()?;
    Some((g.upgrade()?, c))
}

/// Presses Raw (`false`) or Pretty (`true`), as the switch does (refused for invalid JSON).
pub fn set_pretty(console: ConsoleId, pretty: bool, cx: &mut App) {
    if let Some(v) = viewer(console, cx) {
        v.update(cx, |v, cx| v.set_pretty(pretty, cx));
    }
}

/// Copy raw value: the raw text on the clipboard, whatever the well shows.
pub fn copy_raw(console: ConsoleId, cx: &mut App) {
    if let Some(v) = viewer(console, cx) {
        v.update(cx, |v, cx| v.copy_raw(cx));
    }
}

/// Re-run with the largest value limit (when offered).
pub fn rerun(console: ConsoleId, cx: &mut App) {
    if let Some(v) = viewer(console, cx) {
        v.update(cx, |v, cx| v.rerun(cx));
    }
}

/// The run UI refused the viewer's re-run (not a read; fix round 1, I1): the viewer says why.
/// (Public for tests.)
#[doc(hidden)]
pub fn rerun_refused(console: ConsoleId, cx: &mut App) {
    if let Some(v) = viewer(console, cx).filter(|v| v.read(cx).open) {
        v.update(cx, |v, cx| {
            v.refused = Some(message(cx, "hv-vv-rerun-not-read", vec![]));
            // ## m3c-shotfix F8 (fix round 2, I-1): the focus goes to what says why.
            v.focus_refusal = true;
            cx.notify();
        });
        notify_hosts(cx);
    }
}

/// Closes the viewer and gives the focus back to the view shown.
pub fn close(console: ConsoleId, cx: &mut App) {
    if let Some(v) = viewer(console, cx).filter(|v| v.read(cx).open) {
        v.update(cx, |v, cx| v.close(cx));
    }
}

/// Forgets console `console`'s viewer (its console closed).
pub fn console_closed(console: ConsoleId, cx: &mut App) {
    if cx.has_global::<Viewers>() {
        cx.global_mut::<Viewers>().0.remove(&console);
    }
}

/// The tile shows `adapter` for console `console`: a viewer reading another result (another
/// tab, a re-run) closes.
pub(crate) fn sync(console: ConsoleId, adapter: &Rc<GridAdapter>, cx: &mut App) {
    let Some(v) = viewer(console, cx) else {
        return;
    };
    let stale = {
        let v = v.read(cx);
        v.open
            && !v
                .adapter
                .as_ref()
                .and_then(Weak::upgrade)
                .is_some_and(|a| Rc::ptr_eq(&a, adapter) && a.generation() == v.generation)
    };
    if stale {
        cx.defer(move |cx| v.update(cx, |v, cx| v.hide(cx)));
    }
}

/// The open viewer of console `console`, placed in the grid's area (the tile's body).
pub(crate) fn overlay(console: ConsoleId, cx: &App) -> Option<AnyElement> {
    let v = viewer(console, cx).filter(|v| v.read(cx).open)?;
    let (top, bottom, left, _, _) = v.read(cx).placement(cx);
    let mut place = div().absolute().left(px(left));
    place = match (top, bottom) {
        (Some(top), _) => place.top(px(top)),
        (None, Some(bottom)) => place.bottom(px(bottom)),
        (None, None) => place.top(px(GAP_PX)),
    };
    Some(place.child(v).into_any_element())
}

fn notify_hosts(cx: &mut App) {
    for host in super::view::hosts(cx) {
        host.update(cx, |_, cx| cx.notify());
    }
}

impl ValueViewer {
    fn new(console: ConsoleId, cx: &mut Context<Self>) -> Self {
        ValueViewer {
            console,
            open: false,
            take_focus: false,
            cell: None,
            adapter: None,
            generation: 0,
            anchor: None,
            back: None,
            meta: None,
            body: None,
            pretty: false,
            refused: None,
            well: cx.new(|_| VirtualList::new(WellLines::default(), LINE_PX)),
            cut_for: None,
            loading: None,
            // ## m3c-shotfix F8 (fix round 1, I1): a Tab stop (NOTES 22: Raw / Pretty → well →
            // Re-run → Copy raw value).
            focus: cx.focus_handle().tab_stop(true),
            advance: Cell::new(None),
            middle_scroll: ScrollHandle::new(),
            rerun_focus: cx.focus_handle().tab_stop(true),
            copy_focus: cx.focus_handle().tab_stop(true),
            // ## m4-t33d-fr1 (T33d review m2): NOTES 22 "Header": the × is no Tab stop.
            close_focus: cx.focus_handle().tab_stop(false),
            seen_focus: Cell::new(0),
            refused_focus: cx.focus_handle().tab_stop(true),
            focus_refusal: false,
            drawn_rows: Cell::new(None),
        }
    }

    /// What the viewer shows now.
    fn view_of(&self, with_body: bool, cx: &App) -> ViewerView {
        let meta = self.meta.as_ref();
        let body = self.body.as_ref();
        let can_copy = meta.is_some_and(|m| !matches!(m.shape, Shape::Null | Shape::Error));
        let footer = match (meta.map(|m| m.shape), body) {
            (Some(Shape::Null), _) | (_, None) => SharedString::default(),
            (Some(Shape::Bytes), Some(b)) => {
                message(cx, "hv-vv-bytes", vec![("n", ArgValue::Count(b.bytes))])
            }
            (_, Some(b)) => message(
                cx,
                "hv-vv-chars",
                vec![
                    ("size", ArgValue::Text(Texts::of(cx).size(b.bytes))),
                    ("n", ArgValue::Count(b.count)),
                ],
            ),
        };
        let busy = crate::exec_ui::orchestrator::run_busy(self.console, cx);
        let rerun_off = self.refused.clone().or_else(|| {
            (busy && meta.is_some_and(|m| m.rerun.is_some()))
                .then(|| message(cx, "hv-vv-rerun-busy", vec![]))
        });
        ViewerView {
            column: meta.map(|m| m.column.clone()).unwrap_or_default(),
            // ## m3c-int (M3): "row N" follows a sort or a filter while the viewer is open.
            subtitle: meta
                .map(|m| match super::find::active_target(self.console, cx) {
                    Some((table, _)) => subtitle_of(&m.type_label, m.cell, &table, cx),
                    None => m.subtitle.clone(),
                })
                .unwrap_or_default(),
            form: body.map_or(
                match meta.map(|m| m.shape) {
                    Some(Shape::Bytes) => ValueForm::Binary,
                    Some(Shape::Borrowed { json: true, .. }) => ValueForm::Json { valid: false },
                    Some(Shape::Borrowed { xml: true, .. }) => ValueForm::Xml,
                    _ => ValueForm::Text,
                },
                |b| b.form,
            ),
            pretty: body.is_some_and(|b| b.pretty),
            // ## fix round 1 (I2): the whole text only for `view` (tests), never per frame.
            body: if with_body {
                self.well.read(cx).rows().full_text()
            } else {
                String::new()
            },
            footer,
            truncated: meta.and_then(|m| m.truncated.clone()),
            rerun: meta.and_then(|m| m.rerun.clone()),
            rerun_off,
            can_copy,
        }
    }

    fn set_pretty(&mut self, pretty: bool, cx: &mut Context<Self>) {
        let can = matches!(
            self.body.as_ref().map(|b| b.form),
            Some(ValueForm::Json { valid: true } | ValueForm::Xml)
        );
        let pretty = pretty && can;
        if self.pretty == pretty {
            return;
        }
        self.pretty = pretty;
        // The next frame loads the other form (off the UI thread).
        cx.notify();
    }

    fn copy_raw(&mut self, cx: &mut Context<Self>) {
        let Some(meta) = self.meta.clone() else {
            return;
        };
        if matches!(meta.shape, Shape::Null | Shape::Error) {
            return;
        }
        // The raw text is built off the UI thread (a 64 MiB bytea is 128 MB of hex).
        let (view, cell) = (meta.view.clone(), meta.cell);
        let job = cx.background_spawn(async move {
            let value = view.snapshot.cell(cell.row.0, cell.col.0);
            let facts = view.facts.get(cell.col.0 as usize)?;
            match raw_text_in(
                value,
                &facts.view(view.display.engine()),
                view.display.zone(),
            ) {
                RawText::Text(t) => Some(t.into_owned()),
                RawText::Null => None,
            }
        });
        let kept = meta.kept;
        cx.spawn(async move |_, cx| {
            let Some(text) = job.await else { return };
            cx.update(|cx| {
                cx.write_to_clipboard(ClipboardItem::new_string(text));
                let toast = match kept {
                    Some(kept) => {
                        let shown = Texts::of(cx).size(kept);
                        let text = message(
                            cx,
                            "hv-vv-copied-cut",
                            vec![("shown", ArgValue::Text(shown))],
                        );
                        let mut toast =
                            Toast::new(ToastKind::Info, Rc::new(move |_: &App| text.clone()));
                        toast.icon = Some(IconName::TriangleAlert);
                        toast.wide = true;
                        toast
                    }
                    None => {
                        let mut toast = Toast::new(
                            ToastKind::Info,
                            Rc::new(|cx: &App| message(cx, "hv-vv-copied", vec![])),
                        );
                        toast.icon = Some(IconName::CopyCheck);
                        toast
                    }
                };
                ToastQueue::push(cx, toast);
            });
        })
        .detach();
    }

    fn rerun(&mut self, cx: &mut Context<Self>) {
        let Some(meta) = self.meta.as_ref().filter(|m| m.rerun.is_some()) else {
            return;
        };
        // Off while a run is busy, or once refused (m7, I1): the button says why.
        if self.refused.is_some() || crate::exec_ui::orchestrator::run_busy(self.console, cx) {
            return;
        }
        let (console, unit_ordinal) = (self.console, meta.unit_ordinal);
        cx.defer(move |cx| {
            let Some(results) =
                crate::console::Consoles::get(cx, console).and_then(|s| s.results.clone())
            else {
                return;
            };
            results.update(cx, |_, cx| {
                cx.emit(ResultsEvent::RerunWithLimit {
                    console,
                    unit_ordinal,
                    cell_cap: hacivat_results::window::MAX_CELL_CAP,
                })
            });
        });
        cx.notify();
    }

    /// Closes without moving the focus.
    fn hide(&mut self, cx: &mut Context<Self>) {
        self.open = false;
        self.meta = None;
        self.body = None;
        self.loading = None;
        self.refused = None;
        self.well
            .update(cx, |w, cx| *w.rows_mut(cx) = WellLines::default());
        cx.defer(notify_hosts);
        cx.notify();
    }

    /// Closes and gives the focus back to the view shown.
    fn close(&mut self, cx: &mut Context<Self>) {
        let back = self.back.clone().or_else(|| {
            self.anchor
                .as_ref()
                .and_then(|(g, _)| g.upgrade())
                .map(|g| g.read(cx).focus_handle(cx))
        });
        self.hide(cx);
        if let Some(handle) = back {
            cx.defer(move |cx| {
                if let Some(window) = cx.active_window() {
                    let _ = window.update(cx, |_, window, cx| window.focus(&handle, cx));
                }
            });
        }
    }

    /// Where the popover goes in the grid's area: `(top, bottom, left, well height, room)` in
    /// the area's coordinates (module docs). ## m3c-shotfix F8: `room` is the tallest the
    /// popover may be there (from its edge to the area's far edge, less the gap): its value
    /// well gives way first, so its header and footer always show (NOTES cross-cutting 3).
    fn placement(&self, cx: &App) -> (Option<f32>, Option<f32>, f32, f32, Option<f32>) {
        let Some((grid, cell)) = self
            .anchor
            .as_ref()
            .and_then(|(g, c)| Some((g.upgrade()?, *c)))
        else {
            return (Some(GAP_PX), None, GAP_PX, WELL_MAX_PX, None);
        };
        let g = grid.read(cx);
        let Some(geom) = g.geometry() else {
            return (Some(GAP_PX), None, GAP_PX, WELL_MAX_PX, None);
        };
        let area_w = f32::from(geom.bounds.size.width);
        let area_h = f32::from(geom.bounds.size.height);
        let origin = geom.bounds.origin;
        let scroll = g.scroll_state();
        let layout = g.column_layout();
        let view_row = g.view_map().model_to_view(cell.row).map_or(0, |v| v.0);
        let row_top = f32::from(geom.body.top() - origin.y) + scroll.row_top(view_row) as f32;
        let row_bottom = row_top + geom.row_h;
        let x = layout.index_of(cell.col).map_or(GAP_PX, |ix| {
            let screen = if ix < geom.frozen {
                layout.x_of(ix) as f32
            } else {
                layout.screen_x(ix, scroll.left_x) as f32
            };
            f32::from(geom.body.left() - origin.x) + screen
        });
        let left = x.min(area_w - VIEWER_WIDTH_PX - GAP_PX).max(GAP_PX);
        let below = area_h - row_bottom - 2.0 * GAP_PX;
        let above = row_top - 2.0 * GAP_PX;
        let need = CHROME_PX + WELL_MAX_PX;
        // ## m3c-shotfix F8: the room below a top edge, or above a bottom one.
        let room = |edge: f32| Some((area_h - edge - GAP_PX).max(0.0));
        if below >= need {
            let top = row_bottom + GAP_PX;
            (Some(top), None, left, WELL_MAX_PX, room(top))
        } else if above >= need {
            let bottom = area_h - row_top + GAP_PX;
            (None, Some(bottom), left, WELL_MAX_PX, room(bottom))
        } else if below >= above {
            let well = (below - CHROME_PX).clamp(WELL_MIN_PX, WELL_MAX_PX);
            let top = (row_bottom + GAP_PX)
                .min(area_h - CHROME_PX - well)
                .max(GAP_PX);
            (Some(top), None, left, well, room(top))
        } else {
            let well = (above - CHROME_PX).clamp(WELL_MIN_PX, WELL_MAX_PX);
            (Some(GAP_PX), None, left, well, room(GAP_PX))
        }
    }

    /// The well's font advance (Fragment Mono: one advance for every glyph).
    fn advance(&self, window: &Window, cx: &App) -> f32 {
        if let Some(a) = self.advance.get() {
            return a;
        }
        let style = Theme::get(cx).tokens.type_ramp.mono_data;
        let ts = window.text_system();
        let id = ts.resolve_font(&font_for(&style));
        let a = ts
            .em_advance(id, px(style.size_px))
            .map(f32::from)
            .ok()
            .filter(|a| *a > 0.0)
            .unwrap_or(style.size_px * 0.6);
        self.advance.set(Some(a));
        a
    }

    /// Loads the value for `cols` characters per line (the form, the lines, the colour runs, the
    /// count) on the background executor.
    fn load(&mut self, cols: usize, cx: &mut Context<Self>) {
        let Some(meta) = self.meta.as_ref() else {
            return;
        };
        let key = (cols, self.pretty);
        if self.cut_for == Some(key) {
            return;
        }
        self.cut_for = Some(key);
        let (view, cell, shape, pretty) = (meta.view.clone(), meta.cell, meta.shape, self.pretty);
        let job = cx.background_spawn(async move { load_body(view, cell, shape, pretty, cols) });
        let well = self.well.clone();
        self.loading = Some(cx.spawn(async move |this, cx| {
            let (body, lines) = job.await;
            let _ = this.update(cx, |v, cx| {
                v.body = Some(body);
                cx.notify();
            });
            cx.update(|cx| {
                well.update(cx, |w, cx| {
                    *w.rows_mut(cx) = lines;
                    w.scroll_to_top(cx);
                })
            });
        }));
    }
}

impl Render for ValueViewer {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if std::mem::take(&mut self.take_focus) {
            let handle = self.focus.clone();
            window.defer(cx, move |window, cx| window.focus(&handle, cx));
        }
        let theme = *Theme::get(cx);
        let t = theme.tokens;
        let c = &t.colors;
        let (_, _, _, well_h, room) = self.placement(cx);
        let advance = self.advance(window, cx);
        let inner_w = VIEWER_WIDTH_PX - 2.0 * POPOVER_PAD_X - 2.0 * WELL_PAD_X - 6.0;
        let cols = ((inner_w / advance).floor() as usize).max(8);
        self.load(cols, cx);
        if self.meta.is_none() {
            return div().into_any_element();
        }
        let entity = cx.entity();
        let view = self.view_of(false, cx);
        let view = &view;
        // ## m3c-shotfix F8 (fix round 1, I1): the short layout. When the roomy rows with a
        // one-line well do not fit, the header loses its subtitle line (it stays the column's
        // tooltip) and takes the form switch, × and Copy raw value turn compact, so one full line
        // of the value always shows between the header and the footer.
        let has_form = !matches!(view.form, ValueForm::Text);
        let roomy = Rows::of(t, false, has_form, window);
        // ## fix round 2 (m-d): against the body's room (the room less the surface's border).
        let short = room.is_some_and(|r| r - 2.0 < roomy.total(has_form, view.truncated.is_some()));
        let rows = if short {
            Rows::of(t, true, has_form, window)
        } else {
            roomy
        };
        self.drawn_rows.set(Some((short, rows)));
        let mut title = div()
            .id("value-viewer-title")
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .child(
                Label::new(
                    view.column.clone(),
                    TextSource::Catalog,
                    t.type_ramp.code,
                    c.text.to_hsla(),
                )
                .truncate(true)
                .id("value-viewer-column"),
            );
        if short {
            let tip = view.subtitle.clone();
            title = title.tooltip(move |_, cx| Tip::view(tip.clone(), cx));
        } else {
            title = title.child(
                Label::new(
                    view.subtitle.clone(),
                    TextSource::Fluent,
                    t.type_ramp.body_sm,
                    c.text_2.to_hsla(),
                )
                .id("value-viewer-sub"),
            );
        }
        let header_row = div()
            .debug_selector(|| "value-viewer-header".to_string())
            .flex()
            .flex_row()
            .items_start()
            .gap(px(8.0))
            .pt(px(if short { SHORT_PAD } else { HEADER_PAD_TOP }))
            .px(px(POPOVER_PAD_X))
            .child(title);
        let close_button = {
            let e = entity.clone();
            Button::new(
                "value-viewer-close",
                message(cx, "hui-dialog-close", vec![]),
            )
            .kind(ButtonKind::Icon)
            .icon(IconName::X)
            .compact(short)
            .on_raised(true)
            // ## m4-t33d (R-T33-1, NOTES M3b surface 22 "Header"): the × is not a Tab stop,
            // like T2a's dialogs (Escape closes); Copy raw value is one, as NOTES "Keyboard"
            // lists it.
            // ## m4-t33d-fr1 (T33d review m2): the handle is made no stop where it is created
            // (`close_focus`); a `.tab_stop(false)` here only rewrote that handle's flag.
            .focus_handle(self.close_focus.clone())
            .on_click(move |_, _, cx| e.update(cx, |v, cx| v.close(cx)))
        };
        let mut control = div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(8.0))
            .debug_selector(|| "value-viewer-control".to_string())
            .px(px(POPOVER_PAD_X))
            .pt(px(ROW_GAP));
        let mut has_control = false;
        // ## m3c-shotfix F8: the form switch and its note, placed once the room is known.
        let mut seg_el: Option<AnyElement> = None;
        let mut note_el: Option<AnyElement> = None;
        match view.form {
            ValueForm::Json { .. } | ValueForm::Xml => {
                has_control = true;
                let disabled = matches!(view.form, ValueForm::Json { valid: false });
                let (e1, e2) = (entity.clone(), entity.clone());
                let mut seg = Segmented::new("value-viewer-form").item(SegmentItem {
                    content: SegmentContent::Label(message(cx, "hv-vv-raw", vec![])),
                    pressed: !view.pretty,
                    tooltip: message(cx, "hv-vv-raw", vec![]),
                    a11y_name: None,
                    on_click: Rc::new(move |_, _, cx| {
                        e1.update(cx, |v, cx| v.set_pretty(false, cx))
                    }),
                });
                if !disabled {
                    seg = seg.item(SegmentItem {
                        content: SegmentContent::Label(message(cx, "hv-vv-pretty", vec![])),
                        pressed: view.pretty,
                        tooltip: message(cx, "hv-vv-pretty-tip", vec![]),
                        a11y_name: Some(message(cx, "hv-vv-pretty", vec![])),
                        on_click: Rc::new(move |_, _, cx| {
                            e2.update(cx, |v, cx| v.set_pretty(true, cx))
                        }),
                    });
                }
                seg_el = Some(seg.into_any_element());
                if disabled {
                    note_el = Some(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(hacivat_ui::text::paragraph(
                                message(cx, "hv-vv-not-json", vec![]),
                                TextSource::Fluent,
                                t.type_ramp.caption,
                                c.text_2.to_hsla(),
                            ))
                            .into_any_element(),
                    );
                }
            }
            ValueForm::Binary => {
                has_control = true;
                seg_el = Some(
                    Segmented::new("value-viewer-form")
                        .item(SegmentItem {
                            content: SegmentContent::Label(message(cx, "hv-vv-hex", vec![])),
                            pressed: true,
                            tooltip: message(cx, "hv-vv-hex", vec![]),
                            a11y_name: None,
                            on_click: Rc::new(|_, _, _| {}),
                        })
                        .into_any_element(),
                );
            }
            ValueForm::Text => {}
        }
        // ## m3c-shotfix F8: in the short layout the form switch sits in the header row (before
        // ×), so it shows whole.
        let compact = short;
        let header = match (compact, seg_el.take()) {
            (true, Some(seg)) => header_row.child(seg),
            (_, seg) => {
                seg_el = seg;
                header_row
            }
        }
        .child(close_button);
        if let Some(seg) = seg_el {
            control = control.child(seg);
        }
        let has_control = (has_control && !compact) || note_el.is_some();
        control = control.children(note_el);
        let well_fill = match theme.mode {
            ThemeMode::Dark => c.panel,
            ThemeMode::Light => c.ground,
        };
        let lines = self.well.read(cx).rows().len() as f32;
        // ## m3c-shotfix F8 (fix round 2, m-c): the box includes its 1 px border, as its
        // one-line minimum does (a box asked 2 px under its own minimum drew the minimum, and its
        // lines' viewport 2 px short of what was planned).
        let height = (lines * LINE_PX).clamp(LINE_PX, well_h) + 2.0 * WELL_PAD_Y + 2.0;
        let well_focused = self.focus.is_focused(window);
        let well = div()
            .id("value-viewer-well")
            .debug_selector(|| "value-viewer-well".to_string())
            .track_focus(&self.focus)
            .mx(px(POPOVER_PAD_X))
            .mt(px(if short { SHORT_WELL_GAP } else { ROW_GAP }))
            .h(px(height))
            // ## m3c-shotfix F8: in a short room the well gives way first, down to one line.
            .flex_shrink(1.0)
            .min_h(px(LINE_PX + 2.0 * WELL_PAD_Y + 2.0))
            .px(px(WELL_PAD_X))
            .py(px(WELL_PAD_Y))
            .bg(well_fill.to_hsla())
            .border_1()
            .border_color(c.seam_strong.to_hsla())
            .rounded(px(4.0))
            .overflow_hidden()
            // ## m3c-shotfix F8 (fix round 2, m-b): a Tab stop that says what it is (the value
            // of its column, read only) and shows its focus.
            .declare(Role::Document, view.column.clone(), None)
            .child(self.well.clone());
        let well = focus_outline(well, &theme, well_focused);
        // ## m3c-shotfix F8: the header and the footer (the size, Copy raw value) always show:
        // in a room shorter than the popover the value well shrinks first, then what lies
        // between them (the control row, the well, the truncation notice) scrolls.
        let mut body = div()
            .flex()
            .flex_col()
            .w(px(VIEWER_WIDTH_PX))
            .child(header.flex_none());
        if let Some(room) = room {
            // Less the surface's 1 px border on each side.
            body = body.max_h(px((room - 2.0).max(0.0)));
        }
        // ## m3c-shotfix F8 (fix round 1, I1): the middle never goes under one full line of the
        // value, and what takes the focus in it (the well, Re-run) is scrolled into view.
        // ## m3c-shotfix F8 (fix round 2, I-1): ↑/↓/PgUp/PgDn are taken here for any focus
        // inside the middle, in the nested order: ↓ scrolls the focused well to its end, then the
        // middle; ↑ scrolls the middle back to its top first, then the well.
        let e_keys = entity.clone();
        let mut middle = div()
            .id("value-viewer-middle")
            .debug_selector(|| "value-viewer-middle".to_string())
            .flex()
            .flex_col()
            .flex_shrink(1.0)
            // ## inputnav-1 (named exception, F2; fix round 1, m3): room at the middle's bottom
            // for the focus ring of what comes last (the well, Re-run), which the middle's clip
            // cut there. The middle reaches that far into the footer's top padding and pads its
            // content back in, so nothing moves: the well keeps its height in a short room and
            // the popover keeps its size.
            .min_h(px(rows.well_min + WELL_RING_ROOM))
            .mb(px(-WELL_RING_ROOM))
            .pb(px(WELL_RING_ROOM))
            .overflow_y_scroll()
            .track_scroll(&self.middle_scroll)
            .on_key_down(move |ev: &KeyDownEvent, window, cx| {
                let k = &ev.keystroke;
                if k.modifiers.modified() {
                    return;
                }
                let dy = match k.key.as_str() {
                    "up" => -LINE_PX,
                    "down" => LINE_PX,
                    "pageup" => -WELL_MAX_PX,
                    "pagedown" => WELL_MAX_PX,
                    _ => return,
                };
                let (well, mid, in_well) = {
                    let v = e_keys.read(cx);
                    (
                        v.well.clone(),
                        v.middle_scroll.clone(),
                        v.focus.is_focused(window),
                    )
                };
                let s = well.read(cx).scroll_state();
                let well_can = in_well
                    && if dy > 0.0 {
                        s.row_top(s.row_count()) > f64::from(s.viewport().1) + 0.5
                    } else {
                        s.row_top(0) < -0.5
                    };
                let o = mid.offset();
                let max = mid.max_offset().y.max(px(0.0));
                let mid_can = if dy > 0.0 {
                    o.y > -max + px(0.5)
                } else {
                    o.y < px(-0.5)
                };
                let middle_first = dy < 0.0 && mid_can;
                if well_can && !middle_first {
                    well.update(cx, |w, cx| w.scroll_px(f64::from(dy), cx));
                } else if mid_can {
                    mid.set_offset(point(o.x, (o.y - px(dy)).clamp(-max, px(0.0))));
                    e_keys.update(cx, |_, cx| cx.notify());
                }
                cx.stop_propagation();
            })
            .child(hacivat_ui::text::scroll_viewport());
        // The middle's children, counted as they go in (`scroll_to_item`); the probe is 0.
        let mut count = 1;
        // In the short layout the well comes first (what is left of the control row, the "not
        // valid JSON" line, follows it), so the value is what shows.
        let control = has_control.then(|| control.flex_none());
        let (before, after) = if short {
            (None, control)
        } else {
            (control, None)
        };
        if let Some(control) = before {
            middle = middle.child(control);
            count += 1;
        }
        let well_ix = count;
        middle = middle.child(well);
        count += 1;
        if let Some(control) = after {
            middle = middle.child(control);
            count += 1;
        }
        let mut rerun_ix = None;
        let mut refused_ix = None;
        // A focused Re-run that turned off (busy, refused) or went: the focus goes to the
        // refusal when there is one, else to the well (never to a control not drawn).
        let rerun_off = view.rerun.is_none() || view.rerun_off.is_some();
        if self.rerun_focus.is_focused(window) && rerun_off && !self.focus_refusal {
            let well_handle = self.focus.clone();
            window.defer(cx, move |window, cx| window.focus(&well_handle, cx));
        }
        if let Some(notice) = view.truncated.clone() {
            let mut rerun_row = None;
            let mut refused_row = None;
            let notice_row = div()
                .debug_selector(|| "value-viewer-notice".to_string())
                .flex()
                .flex_col()
                .gap(px(NOTICE_GAP))
                .px(px(POPOVER_PAD_X))
                .pt(px(ROW_GAP))
                .child(hacivat_ui::text::paragraph(
                    notice,
                    TextSource::Fluent,
                    t.type_ramp.caption,
                    c.text_2.to_hsla(),
                ));
            if let Some(words) = view.rerun.clone() {
                let e = entity.clone();
                // ## m3c-t38 fix round 1 (m7, I1): off while a run is busy or once refused,
                // with the reason in its tooltip; a refusal is said under it too.
                let off = view.rerun_off.clone();
                let tip = off
                    .clone()
                    .unwrap_or_else(|| message(cx, "hv-vv-rerun-tip", vec![]));
                // ## m3c-shotfix F8 (fix round 1, I1): Re-run is a row of the middle of its own,
                // so focusing it scrolls it (not the whole notice) into view.
                rerun_row = Some(
                    div()
                        .debug_selector(|| "value-viewer-rerun-row".to_string())
                        .flex_none()
                        .px(px(POPOVER_PAD_X))
                        .pt(px(NOTICE_GAP))
                        .child(
                            div()
                                .id("value-viewer-rerun-tip")
                                .tooltip(move |_, cx| Tip::view(tip.clone(), cx))
                                .child(
                                    Button::new("value-viewer-rerun", words)
                                        .kind(ButtonKind::Default)
                                        .icon(IconName::RotateCw)
                                        .on_raised(true)
                                        .focus_handle(self.rerun_focus.clone())
                                        .disabled(off.is_some())
                                        .on_click(move |_, _, cx| {
                                            e.update(cx, |v, cx| v.rerun(cx))
                                        }),
                                ),
                        ),
                );
                if let Some(refused) = self.refused.clone() {
                    // ## m3c-shotfix F8 (fix round 2, I-1): the refusal takes the focus when it
                    // appears and is scrolled into view; it is a Tab stop that names itself.
                    let refused_focused = self.refused_focus.is_focused(window);
                    let line = div()
                        .id("value-viewer-refused-row")
                        .debug_selector(|| "value-viewer-refused-row".to_string())
                        .track_focus(&self.refused_focus)
                        .rounded(px(3.0))
                        .declare(Role::Label, refused.clone(), None);
                    refused_row = Some(
                        div()
                            .flex_none()
                            .px(px(POPOVER_PAD_X))
                            .pt(px(NOTICE_GAP))
                            .child(
                                focus_outline(line, &theme, refused_focused).child(
                                    hacivat_ui::text::paragraph(
                                        refused,
                                        TextSource::Fluent,
                                        t.type_ramp.caption,
                                        c.text.to_hsla(),
                                    )
                                    .id("value-viewer-refused"),
                                ),
                            ),
                    );
                }
            }
            // In the short layout the fact is said in the footer (it never scrolls away); the
            // middle keeps Re-run and the refusal.
            if !short {
                middle = middle.child(notice_row.flex_none());
                count += 1;
            }
            if let Some(r) = rerun_row {
                middle = middle.child(r);
                rerun_ix = Some(count);
                count += 1;
            }
            if let Some(r) = refused_row {
                middle = middle.child(r);
                refused_ix = Some(count);
            }
        }
        if std::mem::take(&mut self.focus_refusal) && refused_ix.is_some() {
            let handle = self.refused_focus.clone();
            window.defer(cx, move |window, cx| window.focus(&handle, cx));
        }
        let focused = if self.focus.is_focused(window) {
            1
        } else if self.rerun_focus.is_focused(window) {
            2
        } else if self.refused_focus.is_focused(window) {
            3
        } else {
            0
        };
        let scroll_to = match focused {
            1 => Some(well_ix),
            2 => rerun_ix,
            3 => refused_ix,
            _ => None,
        };
        if focused != self.seen_focus.replace(focused)
            && let Some(ix) = scroll_to
        {
            self.middle_scroll.scroll_to_item(ix);
        }
        body = body.child(middle);
        let e = entity.clone();
        let (pad_top, pad_bottom) = if short {
            (SHORT_PAD, SHORT_PAD)
        } else {
            (FOOTER_PAD_TOP, FOOTER_PAD_BOTTOM)
        };
        // In the short layout Copy raw value is its glyph (its words the tooltip and name).
        let copy = Button::new("value-viewer-copy", message(cx, "hv-vv-copy", vec![]))
            .kind(if short {
                ButtonKind::Icon
            } else {
                ButtonKind::Default
            })
            .icon(IconName::Copy)
            .compact(short)
            .on_raised(true)
            // ## m4-t33d (R-T33-1)
            .focus_handle(self.copy_focus.clone())
            .disabled(!view.can_copy)
            .on_click(move |_, _, cx| e.update(cx, |v, cx| v.copy_raw(cx)));
        body = body.child(
            div()
                .debug_selector(|| "value-viewer-footer".to_string())
                .flex()
                .flex_row()
                .flex_none()
                .items_center()
                .justify_between()
                .gap(px(8.0))
                .pt(px(pad_top))
                .pb(px(pad_bottom))
                .px(px(POPOVER_PAD_X))
                .child(match (short, view.truncated.clone()) {
                    // ## m3c-shotfix F8 (fix round 2, I-1): in the short layout the truncation
                    // fact takes the size slot, so it never scrolls away: one line, the whole
                    // sentence its tooltip and accessible name.
                    (true, Some(notice)) => {
                        let tip = notice.clone();
                        div()
                            .id("value-viewer-size-slot")
                            .flex_1()
                            .min_w_0()
                            .tooltip(move |_, cx| Tip::view(tip.clone(), cx))
                            .declare(Role::Label, notice.clone(), None)
                            .child(
                                Label::new(
                                    notice,
                                    TextSource::Fluent,
                                    t.type_ramp.caption,
                                    c.text_2.to_hsla(),
                                )
                                .truncate(true)
                                .id("value-viewer-size"),
                            )
                            .into_any_element()
                    }
                    // ## m3c-shotfix F8: the size wraps in the room Copy raw value leaves it.
                    _ => div()
                        .flex_1()
                        .min_w_0()
                        .child(
                            hacivat_ui::text::paragraph(
                                view.footer.clone(),
                                TextSource::Fluent,
                                t.type_ramp.mono_data,
                                c.text_2.to_hsla(),
                            )
                            .id("value-viewer-size"),
                        )
                        .into_any_element(),
                })
                .child(copy),
        );
        let panel = popover_surface(&theme)
            .id(("value-viewer", self.console.0))
            .debug_selector(|| "value-viewer".to_string())
            .on_mouse_down_out(cx.listener(|v, _, _, cx| v.hide(cx)))
            .on_key_down(cx.listener(|v, ev: &KeyDownEvent, _, cx| {
                if ev.keystroke.key == "escape" && !ev.keystroke.modifiers.modified() {
                    cx.stop_propagation();
                    v.close(cx);
                }
            }))
            .child(body);
        // ## m3c-shotfix F8 (fix round 2, m-f): "{type} · row n" is the dialog's description, in
        // either layout (the short one shows it only as a tooltip).
        A11yNode::new(
            "value-viewer-node",
            Role::Dialog,
            message(cx, "hv-vv-label", vec![]),
            false,
            panel,
        )
        .description(view.subtitle.clone())
        .into_any_element()
    }
}

impl std::fmt::Debug for ValueViewer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ValueViewer")
            .field("console", &self.console)
            .field("open", &self.open)
            .field("cell", &self.cell)
            .finish_non_exhaustive()
    }
}

// ---- pure text helpers (any thread) -------------------------------------------------------------

/// Whether `text` is one JSON value.
pub fn is_json(text: &str) -> bool {
    serde_json::from_str::<serde::de::IgnoredAny>(text).is_ok()
}

/// `text` re-indented by two spaces per level, keeping every token (and the key order) as
/// written: only white space outside strings changes. `None` when `text` is not JSON.
pub fn pretty_json(text: &str) -> Option<String> {
    if !is_json(text) {
        return None;
    }
    let mut out = String::with_capacity(text.len() + text.len() / 4);
    let mut depth = 0usize;
    let mut in_str = false;
    let mut escaped = false;
    let mut it = text.chars().peekable();
    let newline = |out: &mut String, depth: usize| {
        out.push('\n');
        for _ in 0..depth {
            out.push_str("  ");
        }
    };
    while let Some(ch) = it.next() {
        if in_str {
            out.push(ch);
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_str = false;
            }
            continue;
        }
        match ch {
            '"' => {
                in_str = true;
                out.push(ch);
            }
            '{' | '[' => {
                let close = if ch == '{' { '}' } else { ']' };
                let mut look = it.clone();
                while look.peek().is_some_and(|c| c.is_whitespace()) {
                    look.next();
                }
                if look.peek() == Some(&close) {
                    // An empty container stays on its line.
                    out.push(ch);
                    out.push(close);
                    it = look;
                    it.next();
                    continue;
                }
                out.push(ch);
                depth += 1;
                newline(&mut out, depth);
            }
            '}' | ']' => {
                depth = depth.saturating_sub(1);
                newline(&mut out, depth);
                out.push(ch);
            }
            ',' => {
                out.push(',');
                newline(&mut out, depth);
            }
            ':' => out.push_str(": "),
            c if c.is_whitespace() => {}
            c => out.push(c),
        }
    }
    Some(out)
}

/// One piece of an XML text.
enum XmlPiece<'a> {
    Open(&'a str),
    Close(&'a str),
    /// A self-closing tag, a declaration, a comment, CDATA.
    Single(&'a str),
    Text(&'a str),
}

fn xml_pieces(text: &str) -> Vec<XmlPiece<'_>> {
    let mut out = Vec::new();
    let mut i = 0;
    let bytes = text.as_bytes();
    while i < bytes.len() {
        if bytes[i] == b'<' {
            // The tag ends at the first `>` outside quotes (comments and CDATA at their end).
            let rest = &text[i..];
            let end = if rest.starts_with("<!--") {
                rest.find("-->").map(|e| e + 3)
            } else if rest.starts_with("<![CDATA[") {
                rest.find("]]>").map(|e| e + 3)
            } else {
                let mut quote = None;
                rest.char_indices().find_map(|(j, c)| match (quote, c) {
                    (None, '"' | '\'') => {
                        quote = Some(c);
                        None
                    }
                    (Some(q), c) if c == q => {
                        quote = None;
                        None
                    }
                    (None, '>') => Some(j + 1),
                    _ => None,
                })
            }
            .unwrap_or(rest.len());
            let tag = &rest[..end];
            out.push(if tag.starts_with("</") {
                XmlPiece::Close(tag)
            } else if tag.ends_with("/>") || tag.starts_with("<?") || tag.starts_with("<!") {
                XmlPiece::Single(tag)
            } else {
                XmlPiece::Open(tag)
            });
            i += end;
        } else {
            let end = text[i..].find('<').map_or(text.len(), |e| i + e);
            out.push(XmlPiece::Text(&text[i..end]));
            i = end;
        }
    }
    out
}

/// `text` (XML) with one element per line, indented by two spaces per level; an element that
/// holds only text stays on one line. Display only.
pub fn pretty_xml(text: &str) -> String {
    if !text.contains('<') {
        return text.to_string();
    }
    let pieces = xml_pieces(text);
    let mut lines: Vec<String> = Vec::new();
    let mut depth = 0usize;
    let indent = |depth: usize| "  ".repeat(depth);
    let mut i = 0;
    while i < pieces.len() {
        match &pieces[i] {
            XmlPiece::Open(tag) => {
                // `<a>text</a>` stays on one line.
                if let (Some(XmlPiece::Text(t)), Some(XmlPiece::Close(close))) =
                    (pieces.get(i + 1), pieces.get(i + 2))
                    && !t.contains('\n')
                {
                    lines.push(format!("{}{tag}{}{close}", indent(depth), t.trim()));
                    i += 3;
                    continue;
                }
                if let Some(XmlPiece::Close(close)) = pieces.get(i + 1) {
                    lines.push(format!("{}{tag}{close}", indent(depth)));
                    i += 2;
                    continue;
                }
                lines.push(format!("{}{tag}", indent(depth)));
                depth += 1;
            }
            XmlPiece::Close(tag) => {
                depth = depth.saturating_sub(1);
                lines.push(format!("{}{tag}", indent(depth)));
            }
            XmlPiece::Single(tag) => lines.push(format!("{}{tag}", indent(depth))),
            XmlPiece::Text(t) => {
                let t = t.trim();
                if !t.is_empty() {
                    lines.push(format!("{}{t}", indent(depth)));
                }
            }
        }
        i += 1;
    }
    lines.join("\n")
}

/// `bytes` as hex rows: the offset, `per_row` bytes in 2-byte groups, the ASCII column (a
/// printable ASCII byte as itself, any other as `.`).
pub fn hex_lines(bytes: &[u8], per_row: usize) -> Vec<String> {
    (0..bytes.len().div_ceil(per_row.max(2)))
        .map(|row| hex_row(bytes, row, per_row))
        .collect()
}

/// Lower-case hex digits.
const HEX_DIGITS: &[u8; 16] = b"0123456789abcdef";

/// Where a hex row's ASCII column starts (the offset, two spaces, the groups, two spaces).
fn hex_ascii_at(per_row: usize) -> usize {
    let per_row = per_row.max(2);
    8 + 2 + per_row * 2 + per_row.div_ceil(2) - 1 + 2
}

/// Row `row` of `bytes` as hex ([`hex_lines`]), built straight into one `String`.
fn hex_row(bytes: &[u8], row: usize, per_row: usize) -> String {
    let per_row = per_row.max(2);
    let start = row * per_row;
    let chunk = bytes
        .get(start..(start + per_row).min(bytes.len()))
        .unwrap_or_default();
    let ascii_at = hex_ascii_at(per_row);
    let mut out = String::with_capacity(ascii_at + per_row);
    for shift in (0..8).rev() {
        out.push(char::from(HEX_DIGITS[(start >> (shift * 4)) & 15]));
    }
    out.push_str("  ");
    for (j, b) in chunk.iter().enumerate() {
        if j > 0 && j % 2 == 0 {
            out.push(' ');
        }
        out.push(char::from(HEX_DIGITS[usize::from(b >> 4)]));
        out.push(char::from(HEX_DIGITS[usize::from(b & 15)]));
    }
    while out.len() < ascii_at {
        out.push(' ');
    }
    for b in chunk {
        out.push(if (b' '..=b'~').contains(b) {
            char::from(*b)
        } else {
            '.'
        });
    }
    out
}

/// The former formatter, kept for its test's shape (unused).
#[cfg(test)]
fn hex_lines_old(bytes: &[u8], per_row: usize) -> Vec<String> {
    let per_row = per_row.max(2);
    // Two hex digits per byte, a space between groups of two bytes.
    let hex_w = per_row * 2 + per_row.div_ceil(2) - 1;
    bytes
        .chunks(per_row)
        .enumerate()
        .map(|(i, chunk)| {
            let mut hex = String::with_capacity(hex_w);
            for (j, b) in chunk.iter().enumerate() {
                if j > 0 && j % 2 == 0 {
                    hex.push(' ');
                }
                hex.push_str(&format!("{b:02x}"));
            }
            let ascii: String = chunk
                .iter()
                .map(|b| {
                    if (b' '..=b'~').contains(b) {
                        char::from(*b)
                    } else {
                        '.'
                    }
                })
                .collect();
            format!("{:08x}  {hex:<hex_w$}  {ascii}", i * per_row)
        })
        .collect()
}

/// The byte ranges of `text`'s lines wrapped at `cols` characters (a line break always ends a
/// line; a long line is cut at `cols`).
fn wrap_lines(text: &str, cols: usize) -> Vec<Range<usize>> {
    let cols = cols.max(1);
    let mut out = Vec::new();
    let mut start = 0;
    for line in text.split('\n') {
        let mut from = start;
        let mut n = 0;
        for (i, _) in line.char_indices() {
            if n == cols {
                out.push(from..start + i);
                from = start + i;
                n = 0;
            }
            n += 1;
        }
        out.push(from..start + line.len());
        start += line.len() + 1;
    }
    out
}

/// JSON's coloured tokens: keys, strings, numbers, literals.
fn json_tokens(text: &str) -> Vec<(Range<usize>, Tok)> {
    let b = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'"' => {
                let start = i;
                i += 1;
                while i < b.len() && b[i] != b'"' {
                    i += if b[i] == b'\\' { 2 } else { 1 };
                }
                i = (i + 1).min(b.len());
                // A string followed by `:` is a key.
                let mut j = i;
                while j < b.len() && b[j].is_ascii_whitespace() {
                    j += 1;
                }
                let tok = if j < b.len() && b[j] == b':' {
                    Tok::Key
                } else {
                    Tok::Str
                };
                out.push((start..i, tok));
            }
            b'-' | b'0'..=b'9' => {
                let start = i;
                while i < b.len() && matches!(b[i], b'-' | b'+' | b'.' | b'e' | b'E' | b'0'..=b'9')
                {
                    i += 1;
                }
                out.push((start..i, Tok::Num));
            }
            b't' | b'f' | b'n' => {
                let start = i;
                while i < b.len() && b[i].is_ascii_alphabetic() {
                    i += 1;
                }
                out.push((start..i, Tok::Lit));
            }
            _ => i += 1,
        }
    }
    out
}

/// XML's coloured tokens: tag names, attribute names, attribute values.
fn xml_tokens(text: &str) -> Vec<(Range<usize>, Tok)> {
    let b = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if b[i] != b'<' {
            i += 1;
            continue;
        }
        // The tag's name.
        let start = i;
        i += 1;
        while i < b.len() && !b[i].is_ascii_whitespace() && b[i] != b'>' {
            if b[i] == b'/' && i > start + 1 {
                break;
            }
            i += 1;
        }
        out.push((start..i, Tok::Tag));
        // Attributes until `>`.
        while i < b.len() && b[i] != b'>' {
            if b[i] == b'"' || b[i] == b'\'' {
                let q = b[i];
                let s = i;
                i += 1;
                while i < b.len() && b[i] != q {
                    i += 1;
                }
                i = (i + 1).min(b.len());
                out.push((s..i, Tok::Str));
            } else if b[i].is_ascii_alphabetic() {
                let s = i;
                while i < b.len()
                    && !matches!(b[i], b'=' | b'>' | b'/')
                    && !b[i].is_ascii_whitespace()
                {
                    i += 1;
                }
                out.push((s..i, Tok::Attr));
            } else {
                i += 1;
            }
        }
        if i < b.len() {
            out.push((i..i + 1, Tok::Tag));
            i += 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pretty_json_keeps_tokens_and_key_order() {
        let raw = r#"{"b":1,"a":[1.50,true,null,"x, y"],"e":{},"f":[]}"#;
        let pretty = pretty_json(raw).unwrap();
        assert_eq!(
            pretty,
            "{\n  \"b\": 1,\n  \"a\": [\n    1.50,\n    true,\n    null,\n    \"x, y\"\n  ],\n  \"e\": {},\n  \"f\": []\n}"
        );
        // Only white space outside strings changed.
        let squeeze = |s: &str| {
            let mut out = String::new();
            let mut in_str = false;
            let mut esc = false;
            for c in s.chars() {
                if in_str {
                    out.push(c);
                    if esc {
                        esc = false;
                    } else if c == '\\' {
                        esc = true;
                    } else if c == '"' {
                        in_str = false;
                    }
                } else if c == '"' {
                    in_str = true;
                    out.push(c);
                } else if !c.is_whitespace() {
                    out.push(c);
                }
            }
            out
        };
        assert_eq!(squeeze(&pretty), squeeze(raw));
        assert_eq!(pretty_json("{\"a\": 1"), None);
        assert_eq!(pretty_json("not json"), None);
    }

    #[test]
    fn pretty_xml_indents_elements() {
        let raw = r#"<?xml version="1.0"?><a x="1"><b>text</b><c/><d><e>1</e></d></a>"#;
        assert_eq!(
            pretty_xml(raw),
            "<?xml version=\"1.0\"?>\n<a x=\"1\">\n  <b>text</b>\n  <c/>\n  <d>\n    <e>1</e>\n  </d>\n</a>"
        );
        assert_eq!(pretty_xml("plain"), "plain");
    }

    #[test]
    fn hex_rows_have_offsets_groups_and_ascii() {
        let bytes: Vec<u8> = (0u8..10).chain(*b"Az").collect();
        let lines = hex_lines(&bytes, 8);
        assert_eq!(
            lines,
            [
                "00000000  0001 0203 0405 0607  ........",
                "00000008  0809 417a            ..Az",
            ]
        );
    }

    #[test]
    fn hex_rows_match_the_formatter_and_keep_the_ascii_column_fixed() {
        // ## m3c-t38 fix round 1 (I2, m8): one String per row, no per-byte formatting.
        let bytes: Vec<u8> = (0..=255u8).cycle().take(1_000).collect();
        assert_eq!(hex_lines(&bytes, 8), hex_lines_old(&bytes, 8));
        // Two spaces in the data never move the ASCII column.
        let row = hex_row(b"  ab  cd", 0, 8);
        assert_eq!(&row[hex_ascii_at(8)..], "  ab  cd");
        assert_eq!(row, "00000000  2020 6162 2020 6364    ab  cd");
    }

    #[test]
    fn lines_wrap_at_the_width_and_at_breaks() {
        let text = "abcdef\nxy\n";
        let lines: Vec<&str> = wrap_lines(text, 4).into_iter().map(|r| &text[r]).collect();
        assert_eq!(lines, ["abcd", "ef", "xy", ""]);
        let text = "ğğğğğ";
        let lines: Vec<&str> = wrap_lines(text, 2).into_iter().map(|r| &text[r]).collect();
        assert_eq!(lines, ["ğğ", "ğğ", "ğ"]);
    }

    #[test]
    fn json_tokens_tell_keys_from_strings() {
        let text = r#"{"k": "v", "n": -1.5e3, "t": true}"#;
        let toks: Vec<(&str, Tok)> = json_tokens(text)
            .into_iter()
            .map(|(r, t)| (&text[r], t))
            .collect();
        assert_eq!(
            toks,
            [
                ("\"k\"", Tok::Key),
                ("\"v\"", Tok::Str),
                ("\"n\"", Tok::Key),
                ("-1.5e3", Tok::Num),
                ("\"t\"", Tok::Key),
                ("true", Tok::Lit),
            ]
        );
    }
}
