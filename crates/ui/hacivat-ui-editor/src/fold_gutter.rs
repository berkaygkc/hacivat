//! Folding in the editor (UI-EDIT-10; M3c Task 32; `design/m3b-safe/NOTES.md` surface 15,
//! binding): the folded ranges of the document, the gutter's fold column, the folded header's
//! inlay, and what `editor.fold`/`unfold`/`fold_all`/`unfold_all` do.
//!
//! - **Ranges.** The foldable ranges come from the language service
//!   (`LanguageService::folding_ranges`, asked once per text version); the folded set is
//!   `hacivat_text::FoldSet`, mapped through every commit ([`on_commit`]); its hidden lines
//!   feed the chunked height map ([`height_map`], cached by version and fold generation), which
//!   the element, scrolling, vertical moves and the accessibility window read.
//! - **A caret never hides.** Folding moves a caret on a hidden line to its header's end; a
//!   selection end that lands on a hidden line (go to line, an error's position, undo, a click
//!   the app makes) opens the folds hiding it (`FoldSet::reveal_selection`); a vertical move
//!   steps over a fold (`commands/movement.rs`).
//! - **The fold column** (NOTES M3c-5): the gutter's last 14 px, right of the line numbers (the
//!   gutter's width does not change). A 12 px chevron, `text_2` (`text` under the pointer):
//!   down on an open range, right on a folded one; open ranges show theirs only while the
//!   pointer is over the gutter, a folded header always. A click toggles. Its tooltip names
//!   the lines ("Fold lines 2–7"). Keyboard focus on the chevrons and their AccessKit buttons
//!   are Task 34's (gutter focus).
//!   ## m3c-t34-editor: in "Tab moves focus" mode every chevron of the painted rows is a tab
//!   stop (a 1 px blue ring when focused; Enter or Space toggles), and each is an AccessKit
//!   button named for what it does, expanded while open.
//! - **The folded header** keeps its text; after it, 8 px on, the inlay "⋯ n lines" (the
//!   `inlay` type style, `text_2`, on `seam_soft`, light `seam`, radius 3, 14 px high, 5 px
//!   padding; a click unfolds; its tooltip "Lines 2–7 are folded. Click to unfold."), and a
//!   1 px `seam_soft` line under the row.
//! - **No default chord** (controller ruling 7, NOTES M3c-8): the four commands are palette
//!   rows only.
//!
//! ## m3b-t01b-seams: the seam (Task 1b). ## m3c-t32-fold: the bodies (Task 32).

use std::cell::RefCell;
use std::ops::Range;
use std::sync::Arc;

use hacivat_text::display::HeightMap;
use hacivat_text::fold::{can_fold, fold_lines};
use hacivat_text::{Committed, FoldRange, FoldSet, SelectionRange, SelectionSet, UserEvent};
use hacivat_ui::icon::{Icon, IconName, IconSize};
use hacivat_ui::l10n::UiArg;
use hacivat_ui::popover::Tip;
// ## m3c-t34-editor (named exception): the chevrons are AccessKit buttons.
use hacivat_ui::popover::{A11yDecl, Declare as _};
use hacivat_ui::text::{Label, TextSource};
use hacivat_ui_core::gpui_surface::{
    AnyElement, App, AvailableSpace, Bounds, ClickEvent, ContentMask, Context, CursorStyle, Entity,
    Hitbox, HitboxBehavior, InteractiveElement as _, IntoElement, MouseButton, MouseMoveEvent,
    ParentElement as _, Pixels, Role, SharedString, StatefulInteractiveElement as _, Styled as _,
    Window, div, fill, point, px, size,
};
use hacivat_ui_core::theme::{Theme, ThemeMode};

use crate::commands::{message, select_set};
use crate::element::FramePrepaint;
use crate::gutter::{Inks, NUMBER_PAD};
use crate::view::EditorView;

/// The chevron's width; it sits 1 px into the gutter's last 14 px.
const CHEVRON_W: f32 = 12.0;
/// The inlay: its gap from the header's text, height, side padding, radius, icon gap.
const INLAY_GAP: f32 = 8.0;
const INLAY_H: f32 = 14.0;
const INLAY_PAD: f32 = 5.0;
const INLAY_RADIUS: f32 = 3.0;
const INLAY_ICON_GAP: f32 = 2.0;

/// A text version, the service's ranges at it, and each header line with the last line of the
/// outermost range on it.
type CandidatesAt = (u64, Arc<[FoldRange]>, Headers);
/// Each header line with the last line of the outermost range on it, sorted.
type Headers = Arc<[(u32, u32)]>;
/// A text version, a fold generation, and the lines hidden then.
type HiddenAt = (u64, u64, Arc<[Range<u32>]>);
/// A text version, a fold generation, a line count and a row height, and the map then.
type MapAt = (u64, u64, u32, u64, HeightMap);

/// The folding module's state in `EditorView` (P11).
#[derive(Debug, Default)]
pub struct FoldState {
    pub(crate) folds: FoldSet,
    /// The service's ranges at a text version, with each header line and the last line of the
    /// outermost range on it (sorted by header).
    candidates: Option<CandidatesAt>,
    /// The hidden lines at (text version, fold generation).
    hidden: Option<HiddenAt>,
    /// The height map at (text version, fold generation, lines, row height) (fix round 1, M7).
    map: Option<MapAt>,
    /// The pointer is over the gutter (open ranges show their chevrons), and its row.
    pub(crate) gutter_hovered: bool,
    pub(crate) hover_line: Option<u32>,
    /// The folded header whose inlay the pointer is over (its ink turns `text`, NOTES 15).
    pub(crate) hover_inlay: Option<u32>,
}

/// The folded ranges.
pub(crate) fn folds(v: &EditorView) -> &FoldSet {
    &v.fold.folds
}

/// The lines the folds hide (cached by text version and fold generation).
pub(crate) fn hidden(v: &mut EditorView) -> Arc<[Range<u32>]> {
    let version = v.model.snapshot().version;
    let generation = v.fold.folds.generation();
    if let Some((ver, gen_, h)) = &v.fold.hidden
        && *ver == version
        && *gen_ == generation
    {
        return h.clone();
    }
    let h: Arc<[Range<u32>]> = v.fold.folds.hidden_lines(v.model.buffer()).into();
    v.fold.hidden = Some((version, generation, h.clone()));
    h
}

/// The height map of `lines` rows of `line_height` with the folds' hidden lines (the uniform
/// map when nothing is folded, M3a's); cached by version, fold generation, lines and height.
pub(crate) fn height_map(v: &mut EditorView, lines: u32, line_height: f64) -> HeightMap {
    if v.fold.folds.folded().is_empty() {
        return HeightMap::uniform(lines, line_height);
    }
    let key = (
        v.model.snapshot().version,
        v.fold.folds.generation(),
        lines,
        line_height.to_bits(),
    );
    if let Some((a, b, c, d, map)) = &v.fold.map
        && (*a, *b, *c, *d) == key
    {
        return map.clone();
    }
    let map = HeightMap::chunked(lines, line_height, &hidden(v));
    v.fold.map = Some((key.0, key.1, key.2, key.3, map.clone()));
    map
}

/// A fresh analysis landed (`LanguageService::on_update`, fix round 1 I1): the foldable ranges
/// are asked again at the same version.
pub(crate) fn on_analysis(v: &mut EditorView) {
    v.fold.candidates = None;
}

/// Says politely that lines `from`–`to` (0-based) were folded or unfolded (fix round 1, M9).
fn announce(v: &EditorView, folded: bool, (from, to): (u32, u32), cx: &mut Context<EditorView>) {
    let id = if folded {
        "hv-editor-fold-done"
    } else {
        "hv-editor-unfold-done"
    };
    let text = message(cx, id, &line_args(from, to));
    v.announce(text, cx);
}

/// Unfolds the folded range `r`, saying so.
fn unfold_announced(v: &mut EditorView, r: &Range<usize>, cx: &mut Context<EditorView>) -> bool {
    let Some(lines) = v.fold.folds.lines_of(r) else {
        return false;
    };
    if !v.fold.folds.unfold_range(r) {
        return false;
    }
    announce(v, false, lines, cx);
    cx.notify();
    true
}

/// The accessible descriptions of the folded headers: "n lines folded" by header line (fix
/// round 1, M9; the chevrons' AccessKit buttons are Task 34's).
pub(crate) fn a11y_notes(v: &mut EditorView, cx: &App) -> Arc<[(u32, SharedString)]> {
    if v.fold.folds.folded().is_empty() {
        return Arc::from([]);
    }
    let lines = u32::try_from(v.model.buffer().len_lines()).unwrap_or(u32::MAX);
    let map = height_map(v, lines, 1.0);
    let mut out: Vec<(u32, SharedString)> = v
        .fold
        .folds
        .lines()
        .iter()
        .filter(|(h, last)| last > h && !map.is_hidden(*h))
        .map(|&(h, last)| {
            let n = UiArg::Count(u64::from(last - h));
            (h, message(cx, "hv-editor-fold-hidden", &[("n", n)]))
        })
        .collect();
    out.sort_by_key(|(h, _)| *h);
    out.dedup_by_key(|(h, _)| *h);
    out.into()
}

/// The service's foldable ranges at the current version (those that can fold here), and each
/// header line with the last line of the outermost range on it, sorted by header.
fn candidates(v: &mut EditorView) -> (Arc<[FoldRange]>, Headers) {
    let version = v.model.snapshot().version;
    if let Some((ver, c, h)) = &v.fold.candidates
        && *ver == version
    {
        return (c.clone(), h.clone());
    }
    let buf = v.model.buffer().clone();
    let c: Arc<[FoldRange]> = v
        .service
        .folding_ranges(v.doc, version)
        .iter()
        .filter(|f| can_fold(&buf, &f.range))
        .cloned()
        .collect();
    let mut headers: Vec<(u32, u32)> = c.iter().map(|f| fold_lines(&buf, &f.range)).collect();
    // The outermost range of a header (its largest last line) first, then one per header.
    headers.sort_unstable_by(|a, b| a.0.cmp(&b.0).then(b.1.cmp(&a.1)));
    headers.dedup_by_key(|h| h.0);
    let h: Arc<[(u32, u32)]> = headers.into();
    v.fold.candidates = Some((version, c.clone(), h.clone()));
    (c, h)
}

/// After the folds changed: a range end on a hidden line moves to its header's end (a range
/// that holds the fold hiding it whole, such as select all, keeps it), and the view repaints
/// (the scroll anchor re-pins onto a visible row in the next prepaint).
fn after_change(v: &mut EditorView, cx: &mut Context<EditorView>) {
    let buf = v.model.buffer().clone();
    let len = u32::try_from(buf.len_lines()).unwrap_or(u32::MAX);
    let map = height_map(v, len, 1.0);
    let folded = v.fold.folds.folded().to_vec();
    let out = |at: usize, holds: &Range<usize>| -> usize {
        let line = u32::try_from(buf.byte_to_line(at)).unwrap_or(u32::MAX);
        if !map.is_hidden(line) {
            return at;
        }
        let header = map.visible_line(line);
        // The outermost fold hiding the line starts on its row's header.
        let held = folded.iter().any(|f| {
            let (h, last) = fold_lines(&buf, f);
            h == header && line <= last && holds.start <= f.start && f.end <= holds.end
        });
        if held {
            at
        } else {
            buf.line_range(header as usize, false).end
        }
    };
    let sel = v.model.selection().clone();
    let moved: Vec<SelectionRange> = sel
        .ranges()
        .iter()
        .map(|r| {
            let holds = r.range();
            let (anchor, head) = (out(r.anchor, &holds), out(r.head, &holds));
            if (anchor, head) == (r.anchor, r.head) {
                *r
            } else {
                SelectionRange::new(anchor, head)
            }
        })
        .collect();
    if moved.as_slice() != sel.ranges() {
        let set = if moved.len() == 1 {
            SelectionSet::single(moved[0])
        } else {
            SelectionSet::normalized(moved, sel.main_index())
        };
        select_set(v, set, UserEvent::Select, cx);
    }
    cx.notify();
}

/// Folds `r` (a range of the current text); `true` when it folded. Refused: a range of fewer
/// than three lines, one already folded, one crossing a folded one's edge (`FoldSet::fold`).
pub fn fold_range(v: &mut EditorView, r: Range<usize>, cx: &mut Context<EditorView>) -> bool {
    let buf = v.model.buffer().clone();
    if !v.fold.folds.fold(r.clone(), &buf) {
        return false;
    }
    if let Some(lines) = v.fold.folds.lines_of(&r) {
        announce(v, true, lines, cx);
    }
    after_change(v, cx);
    true
}

/// Folds the outermost foldable range whose header is `line` (0-based), or unfolds the fold
/// there; `true` when anything changed.
pub(crate) fn toggle_fold_at(v: &mut EditorView, line: u32, cx: &mut Context<EditorView>) -> bool {
    let buf = v.model.buffer().clone();
    if let Some(r) = v.fold.folds.fold_on_line(line, &buf) {
        return unfold_announced(v, &r, cx);
    }
    let (cands, _) = candidates(v);
    let outer = cands
        .iter()
        .filter(|f| fold_lines(&buf, &f.range).0 == line)
        .max_by_key(|f| f.range.len());
    match outer {
        Some(f) => fold_range(v, f.range.clone(), cx),
        None => false,
    }
}

/// Every commit, from the fan-out (`EditorView::apply_committed`): the folds follow the text
/// (only a commit that changed it, at a new version: fix round 1, M2), and a selection end that
/// landed on a hidden line opens the folds hiding it, except during a column drag, which
/// reveals when it ends (M5, [`after_drag`]).
pub(crate) fn on_commit(v: &mut EditorView, committed: &Committed, text_changed: bool) {
    if v.fold.folds.folded().is_empty() {
        return;
    }
    let buf = v.model.buffer().clone();
    if text_changed {
        v.fold.folds.map(&committed.desc);
        v.fold.folds.prune(&buf);
    }
    let column = v
        .drag
        .as_ref()
        .is_some_and(|d| d.unit == crate::mouse::ClickUnit::Column);
    if !column {
        let sel = v.model.selection().clone();
        v.fold.folds.reveal_selection(&sel, &buf);
    }
}

/// A drag ended: a column selection reaching into a fold opens it now (fix round 1, M5);
/// `true` when the folds changed.
pub(crate) fn after_drag(v: &mut EditorView) -> bool {
    if v.fold.folds.folded().is_empty() {
        return false;
    }
    let buf = v.model.buffer().clone();
    let sel = v.model.selection().clone();
    v.fold.folds.reveal_selection(&sel, &buf)
}

// ---- the folding commands (`editor.fold`, `unfold`, `fold_all`, `unfold_all`) ---------------
// Registered in `commands/mod.rs` (palette rows; no default chord, controller ruling 7).

/// The main caret's line.
fn caret_line(v: &EditorView) -> u32 {
    let buf = v.model.buffer();
    u32::try_from(buf.byte_to_line(v.model.selection().main().head)).unwrap_or(u32::MAX)
}

/// Folds the innermost foldable range holding the main caret's line (UI-EDIT-10 step 6).
pub(crate) fn fold(v: &mut EditorView, _: &mut Window, cx: &mut Context<EditorView>) {
    let buf = v.model.buffer().clone();
    let line = caret_line(v);
    let (cands, _) = candidates(v);
    let folded = v.fold.folds.folded().to_vec();
    let inner = cands
        .iter()
        .filter(|f| !folded.contains(&f.range))
        .filter(|f| {
            let (h, last) = fold_lines(&buf, &f.range);
            h <= line && line <= last
        })
        .min_by_key(|f| f.range.len());
    if let Some(f) = inner {
        fold_range(v, f.range.clone(), cx);
    }
}

/// Unfolds the fold whose header is the main caret's line, else the innermost one holding the
/// caret.
pub(crate) fn unfold(v: &mut EditorView, _: &mut Window, cx: &mut Context<EditorView>) {
    let buf = v.model.buffer().clone();
    let head = v.model.selection().main().head;
    let r = v
        .fold
        .folds
        .fold_on_line(caret_line(v), &buf)
        .or_else(|| v.fold.folds.fold_at(head));
    if let Some(r) = r {
        unfold_announced(v, &r, cx);
    }
}

/// Folds every foldable range.
pub(crate) fn fold_all(v: &mut EditorView, _: &mut Window, cx: &mut Context<EditorView>) {
    let buf = v.model.buffer().clone();
    let (cands, _) = candidates(v);
    let mut any = false;
    for f in cands.iter() {
        any |= v.fold.folds.fold(f.range.clone(), &buf);
    }
    if any {
        after_change(v, cx);
    }
}

/// Unfolds everything.
pub(crate) fn unfold_all(v: &mut EditorView, _: &mut Window, cx: &mut Context<EditorView>) {
    if v.fold.folds.unfold_all() {
        cx.notify();
    }
}

// ---- the frame: the fold column, the inlays ------------------------------------------------

/// What one frame paints for folding (laid out in [`prepaint`]).
#[derive(Default)]
pub(crate) struct FoldFrame {
    /// The chevrons (the gutter) and the inlays (the text area).
    chevrons: RefCell<Vec<AnyElement>>,
    inlays: RefCell<Vec<AnyElement>>,
    /// The 1 px lines under the folded headers.
    underlines: Vec<Bounds<Pixels>>,
    /// Each inlay's bounds and its header line (the pointer's inlay inks `text`).
    inlay_bounds: Vec<(Bounds<Pixels>, u32)>,
    /// The gutter's hit target (the open chevrons show while it is hovered).
    gutter: Option<Hitbox>,
    /// Each painted row's top and line, for the hovered row.
    rows: Vec<(Pixels, u32)>,
    line_height: Pixels,
}

impl std::fmt::Debug for FoldFrame {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FoldFrame")
            .field("chevrons", &self.chevrons.borrow().len())
            .field("inlays", &self.inlays.borrow().len())
            .field("underlines", &self.underlines.len())
            .finish()
    }
}

/// Lines `from`–`to` (0-based) as the 1-based arguments of a fold message.
fn line_args(from: u32, to: u32) -> [(&'static str, UiArg); 2] {
    [
        ("from", UiArg::Count(u64::from(from) + 1)),
        ("to", UiArg::Count(u64::from(to) + 1)),
    ]
}

/// Lays out the frame's chevrons and inlays (prepaint, after the rows and the text's hit
/// target): records each inlay's width in its row, so the end-of-line statuses keep clear of
/// it.
pub(crate) fn prepaint(
    v: &mut EditorView,
    gutter: Bounds<Pixels>,
    window: &mut Window,
    cx: &mut Context<EditorView>,
) -> FoldFrame {
    let mut out = FoldFrame::default();
    let Some(layout) = v.layout.as_ref() else {
        return out;
    };
    let lh = layout.rows().line_height;
    out.line_height = lh;
    let text_area = layout.rows().bounds;
    let origin_x = layout.origin_x;
    let rows: Vec<_> = layout
        .lines
        .iter()
        .map(|r| {
            (
                r.line,
                r.top,
                r.fold.as_ref().map(|f| (f.last_line, f.inlay_x)),
            )
        })
        .collect();
    out.rows = rows.iter().map(|(l, t, _)| (*t, *l)).collect();
    // The gutter's own hit target first: the chevrons sit over it.
    out.gutter = Some(window.insert_hitbox(gutter, HitboxBehavior::Normal));
    let (_, headers) = candidates(v);
    let view = cx.entity();
    let theme = cx
        .try_global::<Theme>()
        .copied()
        .unwrap_or_else(|| Theme::new(ThemeMode::Dark));
    let inks = Inks::of(cx);
    let well = match theme.mode {
        ThemeMode::Dark => theme.tokens.colors.seam_soft.to_hsla(),
        ThemeMode::Light => theme.tokens.colors.seam.to_hsla(),
    };
    let hovered = v.fold.gutter_hovered;
    let hover_line = v.fold.hover_line;
    let hover_inlay = v.fold.hover_inlay;
    // ## m3c-t34-editor (named exception): gutter focus (NOTES 15 and 17, controller ruling 3).
    // A focused gutter button shows the open chevrons as the pointer does; in "Tab moves focus"
    // mode every chevron of the painted rows is focusable (hidden until shown) and an
    // AccessKit button, Enter or Space toggling it (## m3c-po: the column is one tab stop).
    let tab = v.commands.tab_moves_focus;
    let show_open = hovered || crate::gutter::focused_button(v, window).is_some();
    let mut painted: Vec<crate::gutter::GutterButton> = Vec::new();
    let chevron_x = gutter.right() - px(NUMBER_PAD) + px(1.0);
    let mut widths: Vec<(u32, f32)> = Vec::new();
    // ## m3c-po (named exception; PO M3c-04): the chevron column is one tab stop (roving,
    // `gutter::column_stop`); ↑/↓ walk it, Escape goes back to the editor.
    let fold_stop = if tab {
        let lines: Vec<u32> = rows
            .iter()
            .filter(|(line, _, fold)| {
                fold.is_some() || headers.binary_search_by_key(line, |h| h.0).is_ok()
            })
            .map(|(line, _, _)| *line)
            .collect();
        crate::gutter::column_stop(v, crate::gutter::GutterButton::Fold(0), &lines, window)
    } else {
        None
    };
    for (line, top, fold) in rows {
        let open = headers
            .binary_search_by_key(&line, |h| h.0)
            .ok()
            .map(|i| headers[i].1);
        let (name, tip_id, last) = match (fold, open) {
            (Some((last, _)), _) => (IconName::ChevronRight, "hv-editor-unfold-name", last),
            (None, Some(last)) if show_open || tab => {
                (IconName::ChevronDown, "hv-editor-fold-name", last)
            }
            (None, _) => continue,
        };
        let shown = fold.is_some() || show_open;
        let color = if hover_line == Some(line) {
            inks.text
        } else {
            inks.text_2
        };
        let tip: SharedString = message(cx, tip_id, &line_args(line, last));
        let target = view.clone();
        let keyed = view.clone();
        let mut el = div()
            .id(("hv-fold-chevron", u64::from(line)))
            .w(px(CHEVRON_W))
            .h(lh)
            .flex()
            .items_center()
            .justify_center()
            .cursor(CursorStyle::PointingHand)
            .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                target.update(cx, |v, cx| {
                    toggle_fold_at(v, line, cx);
                });
                cx.stop_propagation();
            })
            // ## m3c-t34-editor: Enter, Space or an assistive technology's click (the mouse's
            // press toggles above).
            .on_click(move |ev, _, cx| {
                if matches!(ev, ClickEvent::Keyboard(_)) {
                    keyed.update(cx, |v, cx| {
                        toggle_fold_at(v, line, cx);
                    });
                }
            });
        if shown {
            el = el
                .child(Icon::new(name).size(IconSize::Chrome).color(color))
                .tooltip(move |_, cx| Tip::view(tip.clone(), cx));
        }
        if tab {
            let button = crate::gutter::GutterButton::Fold(line);
            // ## m3c-po (named exception; PO M3c-04)
            let handle = crate::gutter::handle_of(v, button, cx).tab_stop(fold_stop == Some(line));
            if handle.is_focused(window) {
                el = el.border_1().border_color(inks.blue);
            }
            el = el.track_focus(&handle);
            el = crate::gutter::with_column_keys(el, view.clone(), button);
            painted.push(button);
        }
        let name = message(cx, tip_id, &line_args(line, last));
        let mut el = el
            .declare_full(A11yDecl::new(Role::Button, name).expanded(fold.is_none()))
            .into_any_element();
        el.prepaint_as_root(
            point(chevron_x, top),
            size(
                AvailableSpace::Definite(px(CHEVRON_W)),
                AvailableSpace::Definite(lh),
            ),
            window,
            cx,
        );
        out.chevrons.borrow_mut().push(el);

        let Some((last, inlay_x)) = fold else {
            continue;
        };
        // The inlay "⋯ n lines" and the line under the header.
        let n = u64::from(last - line);
        let label = message(cx, "hv-editor-fold-lines", &[("n", UiArg::Count(n))]);
        let tip: SharedString = message(cx, "hv-editor-fold-tip", &line_args(line, last));
        // ## fix round 1 (M4): `text` under the pointer.
        let ink = if hover_inlay == Some(line) {
            inks.text
        } else {
            inks.text_2
        };
        let target = view.clone();
        let mut el = div()
            .id(("hv-fold-inlay", u64::from(line)))
            .block_mouse_except_scroll()
            .flex()
            .flex_row()
            .flex_none()
            .items_center()
            .gap(px(INLAY_ICON_GAP))
            .h(px(INLAY_H))
            .px(px(INLAY_PAD))
            .rounded(px(INLAY_RADIUS))
            .bg(well)
            .cursor(CursorStyle::PointingHand)
            .child(
                Icon::new(IconName::Ellipsis)
                    .size(IconSize::Chrome)
                    .color(ink),
            )
            .child(Label::new(
                label,
                TextSource::Fluent,
                theme.tokens.type_ramp.inlay,
                ink,
            ))
            .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                target.update(cx, |v, cx| {
                    toggle_fold_at(v, line, cx);
                });
                cx.stop_propagation();
            })
            .tooltip(move |_, cx| Tip::view(tip.clone(), cx))
            .into_any_element();
        let w = el
            .layout_as_root(
                size(
                    AvailableSpace::MinContent,
                    AvailableSpace::Definite(px(INLAY_H)),
                ),
                window,
                cx,
            )
            .width;
        let x = origin_x + f64::from(inlay_x + INLAY_GAP);
        let at = point(px(x as f32), top + (lh - px(INLAY_H)) / 2.0);
        el.prepaint_at(at, window, cx);
        out.inlay_bounds
            .push((Bounds::new(at, size(w, px(INLAY_H))), line));
        out.inlays.borrow_mut().push(el);
        widths.push((line, INLAY_GAP + f32::from(w)));
        out.underlines.push(Bounds::new(
            point(text_area.left(), top + lh - px(1.0)),
            size(text_area.size.width, px(1.0)),
        ));
    }
    if let Some(layout) = v.layout.as_mut() {
        for (line, w) in widths {
            if let Some(f) = layout.row_mut(line).and_then(|r| r.fold.as_mut()) {
                f.inlay_w = w;
            }
        }
    }
    // ## m3c-t34-editor (named exception)
    crate::gutter::prune_folds(v, &painted, window);
    out
}

/// Paints the lines under the folded headers, the inlays (clipped to the text area) and the
/// chevrons.
pub(crate) fn paint(f: &FoldFrame, text_area: Bounds<Pixels>, window: &mut Window, cx: &mut App) {
    let theme = cx
        .try_global::<Theme>()
        .copied()
        .unwrap_or_else(|| Theme::new(ThemeMode::Dark));
    let line = theme.tokens.colors.seam_soft.to_hsla();
    for b in &f.underlines {
        window.paint_quad(fill(*b, line));
    }
    window.with_content_mask(Some(ContentMask { bounds: text_area }), |window| {
        for el in f.inlays.borrow_mut().iter_mut() {
            el.paint(window, cx);
        }
    });
    for el in f.chevrons.borrow_mut().iter_mut() {
        el.paint(window, cx);
    }
}

/// Follows the pointer over the gutter (paint phase): the open chevrons show while it is
/// there, and the row under it inks its chevron `text`.
pub(crate) fn listen(view: &Entity<EditorView>, frame: &FramePrepaint, window: &mut Window) {
    let Some(gutter) = frame.folds.gutter.clone() else {
        return;
    };
    let rows = frame.folds.rows.clone();
    let inlays = frame.folds.inlay_bounds.clone();
    let lh = frame.folds.line_height;
    let v = view.clone();
    window.on_mouse_event(move |ev: &MouseMoveEvent, phase, window, cx| {
        if !phase.bubble() {
            return;
        }
        let over = gutter.is_hovered(window);
        let line = over
            .then(|| {
                rows.iter()
                    .find(|(top, _)| ev.position.y >= *top && ev.position.y < *top + lh)
                    .map(|(_, l)| *l)
            })
            .flatten();
        let inlay = inlays
            .iter()
            .find(|(b, _)| b.contains(&ev.position))
            .map(|(_, l)| *l);
        v.update(cx, |v, cx| {
            if v.fold.gutter_hovered != over
                || v.fold.hover_line != line
                || v.fold.hover_inlay != inlay
            {
                v.fold.gutter_hovered = over;
                v.fold.hover_line = line;
                v.fold.hover_inlay = inlay;
                cx.notify();
            }
        });
    });
}
