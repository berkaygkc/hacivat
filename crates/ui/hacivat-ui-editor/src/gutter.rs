//! The gutter (UI-EDIT-19; Task 28), after `design/m3a-query/NOTES.md` surfaces 1, 3 and 5
//! (binding): line numbers from the digit cache (`layout_cache::paint_line_number`), right
//! aligned with 14 px of padding, the current line's number in `text` and the others in
//! `text_3`, on the panel (the gutter has no fill of its own); the Run preview's bars at x = 4,
//! 1 px wide (`blue` for what Run executes, dashed `text_2` when a boundary is uncertain, solid
//! `text_3` with no connection; x = 2 when an executed bar is already at x = 4; units of a
//! multi-statement selection leave a 2 px gap). The width (58 px + (digits − 3) × advance, from
//! the document's last line number, so it never jitters while typing) is the element's.
//!
//! [`Overlay`] is what the gutter, the marks and their end-of-line statuses take from one
//! frame's layout: the painted rows (their tops and where their text ends), the text area's
//! right edge, and the band's bars and hints. The element builds it in prepaint ([`prepaint`]).
//!
//! ## m3c-t34-editor (NOTES surface 17, M3c-15): the gutter ▶. On the first row of each statement
//! in view, under the pointer (or on every such row while a gutter button has the focus), a
//! 16 × 18 button in the mark column runs that statement through Run's own request
//! (`EditorEvent::ExecuteRequested`, a caret at its first code byte). It shows only while the
//! host's [`RunGate`] says Run would be enabled, never over a mark that needs the user or is
//! still going, and takes a finished mark's glyph slot (its status stays). Its tooltip lists
//! the statement's last three runs. In "Tab moves focus" mode the ▶ and the fold chevrons are
//! tab stops and AccessKit buttons (controller ruling 3); the tab order runs down the ▶ column,
//! then the fold column.

use std::cell::RefCell;
use std::collections::HashMap;
use std::ops::Range;
use std::rc::Rc;

use hacivat_text::Snapshot;
use hacivat_text::display::{HeightMap, ScrollAnchor};
use hacivat_ui::icon::{Icon, IconName, IconSize};
use hacivat_ui::popover::{A11yDecl, Declare as _};
use hacivat_ui::text::{Label, TextFn, TextSource};
use hacivat_ui_core::gpui_surface::{
    AnyElement, AnyView, App, AppContext as _, AvailableSpace, BorderStyle, Bounds, Context,
    Corners, CursorStyle, Edges, FocusHandle, Hsla, InteractiveElement as _, IntoElement,
    MouseButton, ParentElement as _, Pixels, Point, Render, Role, StatefulInteractiveElement as _,
    Styled as _, Window, div, fill, point, px, quad, size,
};
use hacivat_ui_core::theme::{Theme, ThemeMode, popover_surface};
use smallvec::SmallVec;

use crate::element::FrameLayout;
use crate::layout_cache::paint_line_number;
use crate::marks::{MarkKey, MarkStore};
use crate::preview::{self, BandState, BandStyle, GutterBar, PreviewHint};
use crate::service::{ExecAction, ExecAt, ExecRequest};
use crate::view::{EditorEvent, EditorView};

/// The numbers' right padding.
pub(crate) const NUMBER_PAD: f32 = 14.0;
/// A range bar's x and width; the preview bar's x when an executed bar is already there.
pub(crate) const BAR_X: f32 = 4.0;
pub(crate) const BAR_SHIFTED_X: f32 = 2.0;

/// What the gutter paints in one frame (Task 19 builds it at its paint point; the call site is
/// `element::paint_frame` since Task 1b, fix round 1).
#[derive(Debug, Clone)]
pub struct GutterCtx<'a> {
    pub bounds: Bounds<Pixels>,
    pub line_height: Pixels,
    pub first_line: u32,
    pub last_line: u32,
    /// Digits of the widest line number.
    pub digits: u8,
    pub current_line: u32,
    pub scroll: ScrollAnchor,
    /// The frame's text: byte offsets (marks, the band) to lines through `snapshot.buffer`.
    pub snapshot: &'a Snapshot,
    /// The lines (0-based, end exclusive) of the statement at the caret
    /// (`preview::current_statement`); the scrollbar's marker.
    pub current_statement: Option<Range<u32>>,
    // ## m3a-t28-gutter
    /// The frame's rows, text ends and the band's bars and hints ([`prepaint`]).
    pub overlay: &'a Overlay,
}

impl GutterCtx<'_> {
    // ## m3c-t32-fold (named exception)
    /// The bottom of the row `line` is drawn on (a hidden line: its fold's header row).
    pub fn bottom_of(&self, line: u32) -> Pixels {
        self.top_of(line) + self.line_height
    }

    /// The top of `line`'s row, in window coordinates (a hidden line: its fold's header row).
    pub fn top_of(&self, line: u32) -> Pixels {
        self.overlay.top_of(line).unwrap_or_else(|| {
            let lh = f64::from(f32::from(self.line_height));
            let rel = (f64::from(line) - f64::from(self.scroll.line)) * lh
                - f64::from(self.scroll.offset_in_row);
            self.bounds.origin.y + px(rel as f32)
        })
    }
}

/// One painted row.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OverlayRow {
    pub line: u32,
    pub top: Pixels,
    /// The window x where the row's text ends (a folded header's: its inlay's end).
    pub text_end: Pixels,
    // ## m3c-t32-fold (named exception)
    /// The last line the row covers (a folded header: the fold's last line).
    pub last: u32,
}

/// What the gutter, the marks and the end-of-line statuses take from one frame's layout.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Overlay {
    /// The painted rows, in order.
    pub rows: Vec<OverlayRow>,
    pub line_height: Pixels,
    /// The text area (right of the gutter; the scrollbar track sits over its right edge).
    pub text_area: Bounds<Pixels>,
    /// The band's gutter bars and end-of-line hints.
    pub band: preview::BandFrame,
    // ## m3c-t32-fold (named exception)
    /// The frame's height map (folded lines take no row); `None`: uniform rows.
    pub map: Option<HeightMap>,
    // ## m3c-t34-editor
    /// The rows whose gutter ▶ shows (a finished mark leaves its glyph slot to it).
    pub play_rows: Vec<u32>,
}

impl Overlay {
    /// The top of `line`'s row, extrapolated from the painted rows through the height map (a
    /// hidden line: its fold's header row); `None` when nothing was painted.
    pub fn top_of(&self, line: u32) -> Option<Pixels> {
        let first = self.rows.first()?;
        let rel = match &self.map {
            // ## m3c-t32-fold (named exception)
            Some(m) => m.y_of(m.visible_line(line)) - m.y_of(first.line),
            None => {
                (f64::from(line) - f64::from(first.line)) * f64::from(f32::from(self.line_height))
            }
        };
        Some(first.top + px(rel as f32))
    }

    /// The painted row of `line` (`None` for a hidden line).
    pub fn row(&self, line: u32) -> Option<&OverlayRow> {
        let i = self.rows.binary_search_by_key(&line, |r| r.line).ok()?;
        self.rows.get(i)
    }

    // ## m3c-t32-fold (named exception)
    /// The row `line` is drawn on: its own, or its fold's header's.
    pub fn row_showing(&self, line: u32) -> Option<&OverlayRow> {
        let i = self
            .rows
            .partition_point(|r| r.line <= line)
            .checked_sub(1)?;
        let row = self.rows.get(i)?;
        (line <= row.last).then_some(row)
    }

    /// The painted rows' lines, the last one's fold included (`None` when nothing was
    /// painted).
    pub fn covered(&self) -> Option<(u32, u32)> {
        Some((self.rows.first()?.line, self.rows.last()?.last))
    }
}

/// Builds the frame's [`Overlay`] (prepaint): the rows from `layout`, the band's rectangles into
/// `band` and its bars and hints (an executed bar from `marks` moves a preview bar to x = 2).
pub fn prepaint(
    band: &mut BandState,
    marks: &MarkStore,
    layout: &FrameLayout,
    snap: &Snapshot,
) -> Overlay {
    let rows = layout
        .lines
        .iter()
        .map(|r| OverlayRow {
            line: r.line,
            top: r.top,
            // ## m3c-t32-fold (named exception): a folded header's text ends after its inlay.
            text_end: px((layout.origin_x
                + f64::from(r.fold.map_or(r.width, |f| f.inlay_x + f.inlay_w)))
                as f32),
            last: r.fold.map_or(r.line, |f| f.last_line),
        })
        .collect();
    Overlay {
        rows,
        line_height: layout.rows().line_height,
        text_area: layout.rows().bounds,
        band: preview::layout(band, marks, layout, snap),
        // ## m3c-t32-fold (named exception)
        map: Some(layout.map.clone()),
        // ## m3c-t34-editor: set by the element from `prepaint_buttons`.
        play_rows: Vec::new(),
    }
}

/// The gutter's inks for one frame.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Inks {
    pub text: Hsla,
    pub text_2: Hsla,
    pub text_3: Hsla,
    pub blue: Hsla,
    pub green: Hsla,
    pub amber: Hsla,
    pub seam_strong: Hsla,
    pub seam_soft: Hsla,
    pub panel: Hsla,
}

impl Inks {
    pub fn of(cx: &App) -> Self {
        let theme = cx
            .try_global::<Theme>()
            .copied()
            .unwrap_or_else(|| Theme::new(ThemeMode::Dark));
        let c = &theme.tokens.colors;
        Inks {
            text: c.text.to_hsla(),
            text_2: c.text_2.to_hsla(),
            text_3: c.text_3.to_hsla(),
            blue: c.blue.to_hsla(),
            green: c.green.to_hsla(),
            amber: c.amber.to_hsla(),
            seam_strong: c.seam_strong.to_hsla(),
            seam_soft: c.seam_soft.to_hsla(),
            panel: c.panel.to_hsla(),
        }
    }
}

/// A 1 px dashed vertical line (GPUI's dashed border on the left edge of a 1 px quad).
pub(crate) fn dashed_vline(window: &mut Window, at: Point<Pixels>, height: Pixels, color: Hsla) {
    window.paint_quad(quad(
        Bounds::new(at, size(px(1.0), height)),
        Corners::default(),
        Hsla::transparent_black(),
        Edges {
            left: px(1.0),
            ..Edges::default()
        },
        color,
        BorderStyle::Dashed,
    ));
}

/// A 1 px dashed horizontal line (the uncertain exact tint's lower edge).
pub(crate) fn dashed_hline(window: &mut Window, at: Point<Pixels>, width: Pixels, color: Hsla) {
    window.paint_quad(quad(
        Bounds::new(at, size(width, px(1.0))),
        Corners::default(),
        Hsla::transparent_black(),
        Edges {
            bottom: px(1.0),
            ..Edges::default()
        },
        color,
        BorderStyle::Dashed,
    ));
}

/// Paints the line numbers and the band's gutter bars.
pub fn paint(g: &GutterCtx<'_>, window: &mut Window, cx: &mut App) {
    let inks = Inks::of(cx);
    let right = g.bounds.right() - px(NUMBER_PAD);
    let lines: SmallVec<[(u32, Pixels); 64]> = if g.overlay.rows.is_empty() {
        (g.first_line..=g.last_line.max(g.first_line))
            .filter(|&l| (l as usize) < g.snapshot.buffer.len_lines())
            .map(|l| (l, g.top_of(l)))
            .collect()
    } else {
        g.overlay.rows.iter().map(|r| (r.line, r.top)).collect()
    };
    for (line, top) in lines {
        let color = if line == g.current_line {
            inks.text
        } else {
            inks.text_3
        };
        paint_line_number(window, cx, line.saturating_add(1), right, top, color);
    }
    for bar in &g.overlay.band.bars {
        paint_bar(g, bar, &inks, window);
    }
}

/// One of the band's bars: over its lines, inset 1 px at each end of a unit of several (the
/// 2 px gap between neighbours), at x = 2 or 4 alike (review round 1, Minor 4).
fn paint_bar(g: &GutterCtx<'_>, bar: &GutterBar, inks: &Inks, window: &mut Window) {
    let top = g.top_of(bar.lines.start);
    // ## m3c-t32-fold (named exception): a bar over folded lines collapses onto their header's
    // row.
    let span = g.bottom_of(bar.lines.end.saturating_sub(1).max(bar.lines.start)) - top;
    let (top, height) = if bar.gap {
        (top + px(1.0), span - px(2.0))
    } else {
        (top, span)
    };
    // Clip to the gutter (a bar of a unit that runs past the view).
    let y0 = top.max(g.bounds.top());
    let y1 = (top + height).min(g.bounds.bottom());
    if y1 <= y0 {
        return;
    }
    let x = g.bounds.left() + px(if bar.shifted { BAR_SHIFTED_X } else { BAR_X });
    match bar.style {
        BandStyle::Run => window.paint_quad(fill(
            Bounds::new(point(x, y0), size(px(1.0), y1 - y0)),
            inks.blue,
        )),
        BandStyle::Offline => window.paint_quad(fill(
            Bounds::new(point(x, y0), size(px(1.0), y1 - y0)),
            inks.text_3,
        )),
        BandStyle::Uncertain => dashed_vline(window, point(x, y0), y1 - y0, inks.text_2),
    }
}

/// The hint's glyph and Fluent message (Task 28's block).
pub(crate) fn hint_text(h: PreviewHint) -> (&'static str, Option<(usize, usize)>) {
    match h {
        PreviewHint::Unit { i, n } => ("hv-editor-preview-unit", Some((i, n))),
        PreviewHint::Uncertain => ("hv-editor-preview-uncertain", None),
        PreviewHint::Offline => ("hv-editor-preview-offline", None),
        // ## m3a-t33-exec-ui (named exception): its `n` is passed by `marks::hint_message`.
        PreviewHint::Choices { .. } => ("hv-editor-preview-choices", None),
        // ## m3c-t34-editor: its `n` too.
        PreviewHint::AsOne { .. } => ("hv-editor-preview-as-one", None),
    }
}

// ## m3c-t34-editor
// ---- the gutter's buttons: ▶ (run the row's statement), focus, the last runs (NOTES 17) ----

/// Whether Run would be enabled now (the host's answer: not while a statement runs, waits for
/// an answer, is cancelling or waits for a connection). The ▶ shows only while it says so.
pub type RunGate = Rc<dyn Fn(&App) -> bool>;

/// A button in the gutter, by its row's line (0-based).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GutterButton {
    /// ▶: runs the statement whose first row it is.
    Run(u32),
    /// A fold chevron.
    Fold(u32),
}

/// The gutter buttons' state in `EditorView` (P11): the host's Run gate and, in "Tab moves
/// focus" mode, each button's focus handle (kept while its row is painted or it has the focus).
#[derive(Default)]
pub struct GutterUi {
    pub(crate) gate: Option<RunGate>,
    handles: RefCell<HashMap<GutterButton, FocusHandle>>,
}

impl std::fmt::Debug for GutterUi {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GutterUi")
            .field("gate", &self.gate.is_some())
            .field("handles", &self.handles.borrow().len())
            .finish()
    }
}

/// The ▶'s width, its x in the gutter (its 16 px straddle the 12 px mark slot at x = 8).
const PLAY_W: f32 = 16.0;
const PLAY_X: f32 = 6.0;
/// The last runs tooltip's width.
const RUN_TIP_W: f32 = 300.0;

/// The focus handle of `button` (made on first use).
pub(crate) fn handle_of(v: &EditorView, button: GutterButton, cx: &mut App) -> FocusHandle {
    v.gutter_ui
        .handles
        .borrow_mut()
        .entry(button)
        .or_insert_with(|| cx.focus_handle().tab_stop(true))
        .clone()
}

/// The gutter button that has the focus, if one has.
pub(crate) fn focused_button(v: &EditorView, window: &Window) -> Option<GutterButton> {
    v.gutter_ui
        .handles
        .borrow()
        .iter()
        .find(|(_, h)| h.is_focused(window))
        .map(|(b, _)| *b)
}

/// What one frame paints of the ▶ buttons (laid out in [`prepaint_buttons`]).
#[derive(Default)]
pub(crate) struct ButtonFrame {
    elements: RefCell<Vec<AnyElement>>,
    /// The rows whose ▶ shows (the marks leave their glyph slot to it).
    pub rows: Vec<u32>,
}

impl std::fmt::Debug for ButtonFrame {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ButtonFrame")
            .field("rows", &self.rows)
            .finish_non_exhaustive()
    }
}

/// Asks the host to run the statement whose header is `header` (Run's own path: the same
/// resolve, preview and gate as ⌘↵; the history is sealed as Run seals it).
fn run_row(v: &mut EditorView, header: usize, cx: &mut Context<EditorView>) {
    if v.ime.is_composing() || !run_allowed(v, cx) {
        return;
    }
    v.model.seal_history();
    let snapshot = v.model.snapshot();
    cx.emit(EditorEvent::ExecuteRequested(ExecRequest {
        doc: v.doc,
        action: ExecAction::Execute1,
        at: ExecAt::Carets(smallvec::smallvec![header]),
        snapshot,
    }));
}

/// The host says Run would be enabled.
fn run_allowed(v: &EditorView, cx: &App) -> bool {
    v.gutter_ui.gate.as_ref().is_some_and(|g| g(cx))
}

/// Lays out the frame's ▶ buttons (prepaint): on the first row of each statement in view,
/// while Run would be enabled, never over a mark that needs the user or is still going. One
/// shows on the row under the pointer, or on every such row while a gutter button has the
/// focus; in "Tab moves focus" mode the hidden ones are still there, as tab stops (ruling 3).
pub(crate) fn prepaint_buttons(
    v: &mut EditorView,
    gutter: Bounds<Pixels>,
    window: &mut Window,
    cx: &mut Context<EditorView>,
) -> ButtonFrame {
    let mut out = ButtonFrame::default();
    let tab = v.commands.tab_moves_focus;
    let focused = focused_button(v, window).is_some();
    let Some(layout) = v.layout.as_ref() else {
        return out;
    };
    // Nothing to show: no frame cost while typing (no statement lookup, no host question).
    if !tab && !focused && !v.fold.gutter_hovered {
        prune_runs(v, &[], window);
        return out;
    }
    if !run_allowed(v, cx) {
        if !tab {
            v.gutter_ui
                .handles
                .borrow_mut()
                .retain(|b, h| !matches!(b, GutterButton::Run(_)) || h.is_focused(window));
        }
        return out;
    }
    let lh = layout.rows().line_height;
    let rows: Vec<(u32, Pixels)> = layout.lines.iter().map(|r| (r.line, r.top)).collect();
    let (Some(&(first, _)), Some(last)) = (rows.first(), layout.lines.last()) else {
        return out;
    };
    let last = last.fold.map_or(last.line, |f| f.last_line);
    let snap = v.model.snapshot();
    let buf = &snap.buffer;
    let vis = buf.line_to_byte(first as usize)..buf.line_range(last as usize, true).end;
    let spans = v.service.statements(v.doc, snap.version, vis);
    let hover = v.fold.gutter_hovered.then_some(v.fold.hover_line).flatten();
    let inks = Inks::of(cx);
    let theme = cx
        .try_global::<Theme>()
        .copied()
        .unwrap_or_else(|| Theme::new(ThemeMode::Dark));
    let hover_fill = theme.tokens.colors.hover.to_hsla();
    let view = cx.entity();
    let mut painted = Vec::new();
    for span in spans.iter() {
        let line =
            u32::try_from(buf.byte_to_line(span.header.min(buf.len_bytes()))).unwrap_or(u32::MAX);
        let Some(&(_, top)) = rows.iter().find(|(l, _)| *l == line) else {
            continue;
        };
        if v.marks.busy_on(buf, line) || out.rows.contains(&line) {
            continue;
        }
        let shown = focused || hover == Some(line);
        if !shown && !tab {
            continue;
        }
        let button = GutterButton::Run(line);
        let key = v.marks.at_header(span.header);
        let tip = RunTip {
            runs: key.map(|k| v.marks.run_rows(k, cx)).unwrap_or_default(),
        };
        let name = crate::commands::message(cx, "hv-gutter-run", &[]);
        let header = span.header;
        let target = view.clone();
        let mut el = div()
            .id(("hv-gutter-run", u64::from(line)))
            .w(px(PLAY_W))
            .h(lh)
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(3.0))
            .cursor(CursorStyle::PointingHand)
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(move |_, _, cx| {
                target.update(cx, |v, cx| run_row(v, header, cx));
            });
        if tab {
            let handle = handle_of(v, button, cx);
            let has_focus = handle.is_focused(window);
            el = el.track_focus(&handle);
            if has_focus {
                el = el.border_1().border_color(inks.blue);
            }
        }
        if shown {
            el = el
                .hover(move |s| s.bg(hover_fill))
                .child(Icon::new(IconName::Play).size(IconSize::Chrome).color(
                    if hover == Some(line) {
                        inks.text
                    } else {
                        inks.text_2
                    },
                ))
                .tooltip(move |_, cx| tip.clone().view(cx));
            out.rows.push(line);
        }
        let mut el = el
            .declare_full(A11yDecl::new(Role::Button, name))
            .into_any_element();
        el.prepaint_as_root(
            point(gutter.left() + px(PLAY_X), top),
            size(
                AvailableSpace::Definite(px(PLAY_W)),
                AvailableSpace::Definite(lh),
            ),
            window,
            cx,
        );
        out.elements.borrow_mut().push(el);
        painted.push(button);
    }
    if !tab {
        // Outside the mode only a focused button keeps its handle.
        painted.clear();
    }
    prune_runs(v, &painted, window);
    out
}

/// Forgets the ▶ handles not painted this frame (the chevrons' are `fold_gutter`'s).
fn prune_runs(v: &EditorView, painted: &[GutterButton], window: &Window) {
    v.gutter_ui.handles.borrow_mut().retain(|b, h| {
        !matches!(b, GutterButton::Run(_)) || painted.contains(b) || h.is_focused(window)
    });
}

/// Forgets the chevrons' handles not painted this frame (`fold_gutter::prepaint`).
pub(crate) fn prune_folds(v: &EditorView, painted: &[GutterButton], window: &Window) {
    v.gutter_ui.handles.borrow_mut().retain(|b, h| {
        !matches!(b, GutterButton::Fold(_)) || painted.contains(b) || h.is_focused(window)
    });
}

/// Paints the ▶ buttons.
pub(crate) fn paint_buttons(f: &ButtonFrame, window: &mut Window, cx: &mut App) {
    for el in f.elements.borrow_mut().iter_mut() {
        el.paint(window, cx);
    }
}

/// The ▶'s tooltip (NOTES 17, the gutter tooltip anatomy at 300 px): "Run this statement" with
/// Run's key, "Last runs" and up to three rows, then "Runs through the same checks as Run."; a
/// statement never run says "Not run yet."
#[derive(Clone)]
struct RunTip {
    runs: Vec<crate::marks::RunRow>,
}

impl RunTip {
    fn view(self, cx: &mut App) -> AnyView {
        cx.new(|_| self).into()
    }
}

impl Render for RunTip {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *Theme::get(cx);
        let t = theme.tokens;
        // On the popover (a raised surface) the dim ink is `text_2` (the Raised Ink Rule).
        let (ink, dim) = (t.colors.text.to_hsla(), t.colors.text_2.to_hsla());
        let msg = |cx: &App, id: &'static str| crate::commands::message(cx, id, &[]);
        let key = cx
            .try_global::<crate::commands::ShortcutHints>()
            .cloned()
            .and_then(|h| (h.0)(cx, "editor.run"));
        let mut title = div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(8.0))
            .child(Label::new(
                msg(cx, "hv-gutter-run"),
                TextSource::Fluent,
                t.type_ramp.body_sm,
                ink,
            ));
        if let Some(key) = key {
            title = title.child(Label::new(
                key,
                TextSource::Shortcut,
                t.type_ramp.mono_kbd,
                dim,
            ));
        }
        let mut col = popover_surface(&theme)
            .w(px(RUN_TIP_W))
            .px(px(8.0))
            .py(px(6.0))
            .flex()
            .flex_col()
            .gap(px(4.0))
            .child(title);
        if self.runs.is_empty() {
            col = col.child(Label::new(
                msg(cx, "hv-gutter-never-run"),
                TextSource::Fluent,
                t.type_ramp.body_sm,
                dim,
            ));
        } else {
            col = col.child(Label::new(
                msg(cx, "hv-gutter-last-runs"),
                TextSource::Fluent,
                t.type_ramp.caption,
                dim,
            ));
            for r in &self.runs {
                let mut row = div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(6.0))
                    .child(Icon::new(r.icon).size(IconSize::Chrome).color(r.ink));
                if let Some(when) = r.when.clone() {
                    row = row.child(Label::new(
                        when,
                        TextSource::Fluent,
                        t.type_ramp.mono_micro,
                        dim,
                    ));
                }
                if let Some(text) = r.text.clone() {
                    row = row.child(
                        Label::new(text, TextSource::Fluent, t.type_ramp.body_sm, ink)
                            .truncate(true),
                    );
                }
                if let Some(code) = r.sqlstate.clone() {
                    row = row.child(
                        div()
                            .flex_none()
                            .border_1()
                            .border_color(t.colors.seam_strong.to_hsla())
                            .rounded(px(3.0))
                            .px(px(4.0))
                            .child(Label::new(
                                code,
                                TextSource::Data,
                                t.type_ramp.mono_micro,
                                ink,
                            )),
                    );
                }
                col = col.child(row);
            }
        }
        col.child(Label::new(
            msg(cx, "hv-gutter-same-path"),
            TextSource::Fluent,
            t.type_ramp.caption,
            dim,
        ))
    }
}

impl EditorView {
    /// Installs the host's Run gate: the gutter ▶ shows only while it says Run would be
    /// enabled (`None`: never).
    pub fn set_gutter_run(&mut self, gate: Option<RunGate>, cx: &mut Context<Self>) {
        self.gutter_ui.gate = gate;
        cx.notify();
    }

    /// Sets when mark `key`'s run ended (the app's time text, for the ▶'s "Last runs").
    pub fn set_exec_when(&mut self, key: MarkKey, when: Option<TextFn>, cx: &mut Context<Self>) {
        crate::marks::set_when(&mut self.marks, key, when);
        cx.notify();
    }

    /// The gutter button that has the keyboard focus, if one has.
    pub fn gutter_focus(&self, window: &Window) -> Option<GutterButton> {
        focused_button(self, window)
    }
}

// ## m3c-t32-fold
#[cfg(test)]
mod tests {
    use super::*;

    /// Lines 1–3 folded under line 0: a hidden line is drawn on its header's row, a bar over
    /// folded lines collapses onto it, and the painted rows cover the fold's lines.
    #[test]
    #[allow(clippy::single_range_in_vec_init, reason = "a line range as data")]
    fn hidden_lines_sit_on_their_headers_row() {
        let lh = px(18.0);
        let ov = Overlay {
            rows: vec![
                OverlayRow {
                    line: 0,
                    top: px(100.0),
                    text_end: px(0.0),
                    last: 3,
                },
                OverlayRow {
                    line: 4,
                    top: px(118.0),
                    text_end: px(0.0),
                    last: 4,
                },
            ],
            line_height: lh,
            map: Some(HeightMap::chunked(10, 18.0, &[1..4])),
            ..Overlay::default()
        };
        assert_eq!(ov.top_of(0), Some(px(100.0)));
        assert_eq!(ov.top_of(2), Some(px(100.0)));
        assert_eq!(ov.top_of(4), Some(px(118.0)));
        assert_eq!(ov.top_of(6), Some(px(154.0)), "extrapolated past the rows");
        assert!(ov.row(2).is_none());
        assert_eq!(ov.row_showing(2).map(|r| r.line), Some(0));
        assert_eq!(ov.row_showing(4).map(|r| r.line), Some(4));
        assert_eq!(ov.covered(), Some((0, 4)));
        // Without a map (M3a's uniform rows), as before.
        let flat = Overlay {
            map: None,
            ..ov.clone()
        };
        assert_eq!(flat.top_of(2), Some(px(136.0)));
    }
}
