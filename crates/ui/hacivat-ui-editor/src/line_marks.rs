// ## m4b-t17-secrets-ui
//! The host's line marks (named items, ADR 0007 `## m4b-t17`): glyphs a host puts on lines of
//! the text, with a hover tooltip. The app's script tabs use them for secret-like literals
//! (HV-SCRIPT-10, `design/m4b-scripts` surface 8): a `key-round` glyph in `text_2` (no hue) in
//! the gutter's mark slot (x = 8, 12 px by a row).
//!
//! - **Anchors.** A mark is a byte range at a version: [`set`] rebases it through the change
//!   log to the current text (one that is too old or was deleted is dropped), and each edit maps
//!   it on as a container range ([`on_commit`]: it grows with typing at its edges and goes when
//!   one edit deleted all of it). The glyph sits on the row of the range's first line (a hidden
//!   line: its fold's header row).
//! - **Slots** (fix round 1, I1, R-T17-2: as M3c does for execution marks). An execution
//!   mark's glyph keeps its slot. A shown gutter ▶ (hover, a focused column, the column in "Tab
//!   moves focus" mode while it shows) takes the slot too, and carries the row's labels in its
//!   tooltip and its accessible name (`gutter::prepaint_buttons`); the key shows otherwise. A
//!   line mark never removes a ▶: the column's ↑/↓ visit marked rows, and a focused ▶ stays.
//! - **Texts.** The editor knows nothing of what a mark is about: the host gives its label
//!   (the glyph's accessible name and the tooltip's first line) and the tooltip's other lines.
//!   Two marks on one row list both labels once each, then the first mark's other lines.

use std::cell::RefCell;
use std::ops::Range;

use hacivat_text::{ChangeLog, Committed, RangeKind, Rebased, Snapshot, TextBuffer};
use hacivat_ui::icon::{Icon, IconName, IconSize};
use hacivat_ui::popover::{A11yDecl, Declare as _};
use hacivat_ui::text::{TextFn, TextSource, paragraph};
use hacivat_ui_core::gpui_surface::{
    AnyElement, AnyView, App, AppContext as _, AvailableSpace, Context, InteractiveElement as _,
    IntoElement, ParentElement as _, Pixels, Render, Role, SharedString,
    StatefulInteractiveElement as _, Styled as _, Window, div, point, px, size,
};
use hacivat_ui_core::theme::{Theme, popover_surface};

use crate::gutter::{GutterCtx, Inks};

/// The mark slot's x and width (the execution marks' slot).
const GLYPH_X: f32 = 8.0;
const GLYPH_W: f32 = 12.0;
/// The tooltip's width (the gutter tooltip anatomy).
const TIP_W: f32 = 280.0;

/// One line mark: a byte range of the text, its glyph and its texts.
#[derive(Clone)]
pub struct LineMark {
    pub range: Range<usize>,
    pub icon: IconName,
    /// The glyph's accessible name and the tooltip's first line.
    pub label: TextFn,
    /// The tooltip's other lines.
    pub detail: Vec<TextFn>,
}

impl std::fmt::Debug for LineMark {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LineMark")
            .field("range", &self.range)
            .field("icon", &self.icon)
            .finish_non_exhaustive()
    }
}

/// The marks at the model's current version, sorted by range start.
#[derive(Debug, Default)]
pub(crate) struct LineMarks {
    marks: Vec<LineMark>,
    /// Fix round 1: the rows the last painted frame drew a glyph on.
    pub(crate) painted: Vec<u32>,
}

/// What a shown ▶ covers on its row: the marks' icon, their labels (once each, in text order)
/// and (## m4b-t21-shots, T17 re-review n1) the first mark's other lines, as the key's own
/// tooltip lists them.
#[derive(Debug, Clone)]
pub(crate) struct Covered {
    pub icon: IconName,
    pub labels: Vec<SharedString>,
    pub detail: Vec<SharedString>,
}

/// Fix round 1 (I1): what the marks on `line` show, for the ▶ that covers them; `None` when the
/// line has none.
pub(crate) fn on_line(s: &LineMarks, buf: &TextBuffer, line: u32, cx: &App) -> Option<Covered> {
    let mut first: Option<&LineMark> = None;
    let mut labels: Vec<SharedString> = Vec::new();
    for m in s
        .marks
        .iter()
        .filter(|m| line_of(buf, m.range.start) == line)
    {
        first.get_or_insert(m);
        let l = (m.label)(cx);
        if !labels.contains(&l) {
            labels.push(l);
        }
    }
    first.map(|m| Covered {
        icon: m.icon,
        labels,
        detail: m.detail.iter().map(|f| f(cx)).collect(),
    })
}

/// Replaces the marks with `marks`, computed at `version`, rebased to `snap.version`.
pub(crate) fn set(
    s: &mut LineMarks,
    version: u64,
    marks: Vec<LineMark>,
    log: &ChangeLog,
    snap: &Snapshot,
) {
    let len = snap.buffer.len_bytes();
    let mut kept: Vec<LineMark> = marks
        .into_iter()
        .filter_map(|mut m| {
            let r = if version == snap.version {
                m.range.clone()
            } else {
                match log.rebase(version, m.range.clone(), RangeKind::Container) {
                    Rebased::Mapped(r) => r,
                    Rebased::Deleted | Rebased::TooOld => return None,
                }
            };
            (r.start <= r.end && r.end <= len).then(|| {
                m.range = r;
                m
            })
        })
        .collect();
    kept.sort_by_key(|m| (m.range.start, m.range.end));
    s.marks = kept;
}

/// Maps the marks through a committed edit (a mark one edit deleted goes).
pub(crate) fn on_commit(s: &mut LineMarks, c: &Committed) {
    if !c.buffer_changed || s.marks.is_empty() {
        return;
    }
    s.marks.retain_mut(
        |m| match c.desc.map_range(m.range.clone(), RangeKind::Container) {
            Some(r) => {
                m.range = r;
                true
            }
            None => false,
        },
    );
    s.marks.sort_by_key(|m| (m.range.start, m.range.end));
}

/// The lines (0-based, ascending, once each) the marks start on in `buf`.
pub(crate) fn lines(s: &LineMarks, buf: &TextBuffer) -> Vec<u32> {
    let mut out: Vec<u32> = s
        .marks
        .iter()
        .map(|m| line_of(buf, m.range.start))
        .collect();
    out.dedup();
    out
}

fn line_of(buf: &TextBuffer, at: usize) -> u32 {
    u32::try_from(buf.byte_to_line(at.min(buf.len_bytes()))).unwrap_or(u32::MAX)
}

/// One frame's line-mark glyphs (laid out in [`prepaint`], painted by [`paint`]).
#[derive(Default)]
pub(crate) struct LineMarkFrame {
    elements: RefCell<Vec<AnyElement>>,
    /// The rows a glyph was laid out on.
    pub rows: Vec<u32>,
}

impl std::fmt::Debug for LineMarkFrame {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LineMarkFrame")
            .field("rows", &self.rows)
            .finish_non_exhaustive()
    }
}

/// Lays out the glyphs of the painted rows (prepaint). `taken` are the tops of the rows whose
/// mark slot an execution mark holds.
pub(crate) fn prepaint(
    s: &LineMarks,
    g: &GutterCtx<'_>,
    taken: &[Pixels],
    window: &mut Window,
    cx: &mut App,
) -> LineMarkFrame {
    let mut out = LineMarkFrame::default();
    let ov = g.overlay;
    let Some((first, last)) = ov.covered() else {
        return out;
    };
    let buf = &g.snapshot.buffer;
    let inks = Inks::of(cx);
    // The marks by the row they show on, in text order.
    let mut rows: Vec<(u32, Vec<&LineMark>)> = Vec::new();
    for m in &s.marks {
        let own = line_of(buf, m.range.start);
        if own < first || own > last {
            continue;
        }
        let Some(line) = ov.row_showing(own).map(|r| r.line) else {
            continue;
        };
        match rows.iter_mut().find(|(l, _)| *l == line) {
            Some((_, v)) => v.push(m),
            None => rows.push((line, vec![m])),
        }
    }
    for (line, marks) in rows {
        let top = g.top_of(line);
        // An execution mark's glyph, or a shown ▶ (fix round 1, I1), holds the slot.
        if taken.contains(&top) || ov.play_rows.contains(&line) {
            continue;
        }
        let mut labels: Vec<SharedString> = Vec::new();
        for m in &marks {
            let l = (m.label)(cx);
            if !labels.contains(&l) {
                labels.push(l);
            }
        }
        let detail: Vec<SharedString> = marks[0].detail.iter().map(|f| f(cx)).collect();
        let name: SharedString = labels
            .iter()
            .map(|l| l.as_ref())
            .collect::<Vec<_>>()
            .join(", ")
            .into();
        let icon = marks[0].icon;
        // Fix round 1 (m3): the other lines (the hint) are the glyph's accessible description.
        let description: SharedString = detail
            .iter()
            .map(|l| l.as_ref())
            .collect::<Vec<_>>()
            .join(" ")
            .into();
        let tip = LineMarkTip {
            icon,
            labels,
            detail,
        };
        let mut el = div()
            .id(("hv-line-mark", u64::from(line)))
            .debug_selector(move || format!("line-mark-{line}"))
            .w(px(GLYPH_W))
            .h(g.line_height)
            .flex()
            .items_center()
            .justify_center()
            .child(Icon::new(icon).size(IconSize::Chrome).color(inks.text_2))
            .tooltip(move |_, cx| tip.clone().view(cx))
            .declare_full(A11yDecl::new(Role::Image, name).description(description))
            .into_any_element();
        el.prepaint_as_root(
            point(g.bounds.left() + px(GLYPH_X), top),
            size(
                AvailableSpace::Definite(px(GLYPH_W)),
                AvailableSpace::Definite(g.line_height),
            ),
            window,
            cx,
        );
        out.elements.borrow_mut().push(el);
        out.rows.push(line);
    }
    out
}

/// Paints the glyphs laid out by [`prepaint`].
pub(crate) fn paint(f: &LineMarkFrame, window: &mut Window, cx: &mut App) {
    for el in f.elements.borrow_mut().iter_mut() {
        el.paint(window, cx);
    }
}

/// A line mark's tooltip (the popover anatomy): the glyph and each label, then the other lines
/// in `text_2` (the Raised Ink Rule), wrapping at the tooltip's width.
#[derive(Clone)]
struct LineMarkTip {
    icon: IconName,
    labels: Vec<SharedString>,
    detail: Vec<SharedString>,
}

impl LineMarkTip {
    fn view(self, cx: &mut App) -> AnyView {
        cx.new(|_| self).into()
    }
}

impl Render for LineMarkTip {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *Theme::get(cx);
        let t = theme.tokens;
        let (ink, dim) = (t.colors.text.to_hsla(), t.colors.text_2.to_hsla());
        let mut col = popover_surface(&theme)
            .w(px(TIP_W))
            .px(px(8.0))
            .py(px(6.0))
            .flex()
            .flex_col()
            .gap(px(4.0));
        for label in &self.labels {
            col = col.child(
                div()
                    .flex()
                    .flex_row()
                    .items_start()
                    .gap(px(6.0))
                    .child(
                        div()
                            .flex_none()
                            .pt(px(2.0))
                            .child(Icon::new(self.icon).size(IconSize::Chrome).color(dim)),
                    )
                    // Fix round 1: a long label (a long locale) wraps, never clips.
                    .child(div().flex_1().min_w(px(0.0)).child(paragraph(
                        label.clone(),
                        TextSource::Fluent,
                        t.type_ramp.body_strong,
                        ink,
                    ))),
            );
        }
        for line in &self.detail {
            col = col.child(paragraph(
                line.clone(),
                TextSource::Fluent,
                t.type_ramp.body_sm,
                dim,
            ));
        }
        col
    }
}

#[cfg(test)]
mod tests {
    use std::rc::Rc;
    use std::sync::Arc;

    use hacivat_text::{Bias, ChangeSet, SelectionRange, SelectionSet};

    use super::*;

    fn mark(range: Range<usize>) -> LineMark {
        LineMark {
            range,
            icon: IconName::KeyRound,
            label: Rc::new(|_: &App| SharedString::from("label")),
            detail: Vec::new(),
        }
    }

    fn snap(buffer: &TextBuffer, version: u64) -> Snapshot {
        Snapshot {
            buffer: buffer.clone(),
            selection: SelectionSet::single(SelectionRange {
                anchor: 0,
                head: 0,
                goal_x: None,
                assoc: Bias::After,
            }),
            version,
        }
    }

    fn committed(cs: &ChangeSet, version: u64) -> Committed {
        let desc = cs.desc();
        Committed {
            version,
            dirty: desc.changed_range(),
            desc,
            buffer_changed: true,
        }
    }

    /// Marks follow an edit made after they were set; one an edit deleted goes.
    #[test]
    fn marks_follow_the_text_and_deleted_ones_go() {
        let b0 = TextBuffer::from_str("ab\ncd 'x'\nef 'y'\n");
        let mut s = LineMarks::default();
        set(
            &mut s,
            0,
            vec![mark(13..16), mark(6..9)],
            &ChangeLog::default(),
            &snap(&b0, 0),
        );
        assert_eq!(lines(&s, &b0), vec![1, 2], "sorted by position");
        // Two lines inserted at the top move both down.
        let edit = ChangeSet::from_edits(b0.len_bytes(), [(0..0, Arc::from("1\n2\n"))]).unwrap();
        let mut b1 = b0.clone();
        edit.apply(&mut b1);
        on_commit(&mut s, &committed(&edit, 1));
        assert_eq!(lines(&s, &b1), vec![3, 4]);
        // Deleting the first mark's whole line drops it; the second moves up.
        let del = ChangeSet::from_edits(b1.len_bytes(), [(7..14, Arc::from(""))]).unwrap();
        let mut b2 = b1.clone();
        del.apply(&mut b2);
        assert_eq!(b2.slice(0..b2.len_bytes()), "1\n2\nab\nef 'y'\n");
        on_commit(&mut s, &committed(&del, 2));
        assert_eq!(s.marks.len(), 1, "{:?}", s.marks);
        assert_eq!(lines(&s, &b2), vec![3]);
    }

    /// A mark of a version the log no longer holds is dropped (the host sends fresh ones).
    #[test]
    fn a_mark_of_an_unknown_version_is_dropped() {
        let b = TextBuffer::from_str("x 'y'\n");
        let mut s = LineMarks::default();
        set(
            &mut s,
            7,
            vec![mark(2..5)],
            &ChangeLog::default(),
            &snap(&b, 3),
        );
        assert!(s.marks.is_empty());
    }
}
