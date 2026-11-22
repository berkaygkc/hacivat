//! The editor status segment entity (R-M3-11; Task 32): "Ln 14, Col 49", the selected
//! characters, "Reduced mode".
//!
//! An entity of its own, notified only by its editor (`EditorEvent::StatusChanged`,
//! `TierChanged`), and only when what it shows changed: typing re-renders this segment and the
//! editor, never the results or the grid. `console::editor_segment` hands it to the status bar
//! (Task 34 places it). Order, left to right (NOTES surface 6): Reduced mode · "186 selected" ·
//! "Ln 14, Col 49" (## m3c-t31-multi: "3 carets" before the selected count from two carets on).
//! The line and column are raw numbers like the gutter's; the selected count
//! uses the format locale. "Reduced mode" is a trust signal and never gives way (Task 34's row
//! decides the giving-way order; [`EditorSegment::reduced`] tells it).
//!
//! ## m3c-t41-chrome (F2 M8 = T34 M6; design/m3b-safe surface 24, M3c-35): the status bar
//! places each part in its own cell ([`SegmentCell`], one view per [`SegmentPart`], notified
//! with the segment), so each gives way by its own rank: Reduced mode never; the selected
//! count, the carets and "Mixed line endings" before the path; Ln/Col after it. The segment
//! refreshes the windows only when a part appears or leaves (typing re-renders the cells, not
//! the status bar).

use hacivat_l10n::{Count, NoArgs, ids};
use hacivat_ui::text::{Label, TextSource};
use hacivat_ui_core::gpui_surface::{
    Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _, Render, Styled as _,
    Subscription, Window, div, px,
};
use hacivat_ui_core::theme::Theme;
use hacivat_ui_editor::{EditorEvent, EditorStatus, EditorView, Tier};
// ## m3c-t34-editor
use std::rc::Rc;

use hacivat_text::LineEnding;
use hacivat_ui::menu::{MenuItemSpec, PopupMenu, PopupMenuEvent};
use hacivat_ui::popover::{Declare as _, Tip};
use hacivat_ui::text::TextFn;
use hacivat_ui_core::gpui_surface::{
    AnyElement, App, Pixels, Point, Role, SharedString, StatefulInteractiveElement as _,
    WeakEntity, point,
};

use crate::l10n_global::tr;

/// The status bar's editor segment of one console.
pub struct EditorSegment {
    status: EditorStatus,
    renders: u64,
    _subscription: Subscription,
    // ## m3c-t34-editor (named exception): the editor the line endings menu normalises.
    editor: WeakEntity<EditorView>,
    // ## m3c-t41-chrome
    /// The parts it shows now (a change refreshes the status bar).
    parts: Vec<SegmentPart>,
}

// ## m3c-t41-chrome
/// A part of the editor segment, in the status bar's order (NOTES surface 6 and 17).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SegmentPart {
    /// "Reduced mode": a trust signal, never gives way.
    Reduced,
    /// The "Mixed line endings" button.
    LineEndings,
    /// "3 carets".
    Carets,
    /// "186 selected".
    Selected,
    /// "Ln 14, Col 49".
    LnCol,
}

impl SegmentPart {
    /// Every part, left to right.
    pub const ALL: [SegmentPart; 5] = [
        SegmentPart::Reduced,
        SegmentPart::LineEndings,
        SegmentPart::Carets,
        SegmentPart::Selected,
        SegmentPart::LnCol,
    ];
}

/// The parts `status` shows (`eol`: the editor is offered the line endings button).
fn parts_of(status: EditorStatus, eol: bool) -> Vec<SegmentPart> {
    SegmentPart::ALL
        .into_iter()
        .filter(|p| match p {
            // ## m4b-t14-tabs (named exception, carries-w1 T14/T18): a T2 preview's
            // "Read-only preview" takes the same cell (a trust signal too).
            SegmentPart::Reduced => matches!(status.tier, Tier::Reduced | Tier::Preview),
            SegmentPart::LineEndings => status.mixed_line_endings && eol,
            SegmentPart::Carets => status.carets > 1,
            SegmentPart::Selected => status.selected_chars > 0,
            SegmentPart::LnCol => true,
        })
        .collect()
}

impl EditorSegment {
    /// A segment that follows `editor`.
    pub fn new(editor: &Entity<EditorView>, cx: &mut Context<Self>) -> Self {
        let status = editor.read(cx).status();
        let subscription = cx.subscribe(editor, |this, editor, event: &EditorEvent, cx| {
            if matches!(
                event,
                EditorEvent::StatusChanged | EditorEvent::TierChanged(_)
            ) {
                let now = editor.read(cx).status();
                if now != this.status {
                    this.status = now;
                    // ## m3c-t41-chrome: a part that appears or leaves changes the status
                    // bar's cells.
                    let parts = parts_of(now, eol_offered(&editor, cx));
                    if parts != this.parts {
                        this.parts = parts;
                        cx.refresh_windows();
                    }
                    cx.notify();
                }
            }
        });
        EditorSegment {
            status,
            renders: 0,
            _subscription: subscription,
            editor: editor.downgrade(),
            parts: parts_of(status, eol_offered(editor, cx)),
        }
    }

    // ## m3c-t41-chrome
    /// The parts it shows, left to right.
    pub fn parts(&self) -> &[SegmentPart] {
        &self.parts
    }

    /// What the segment shows.
    pub fn status(&self) -> EditorStatus {
        self.status
    }

    /// The editor is in reduced mode: the segment's "Reduced mode" cell never gives way.
    pub fn reduced(&self) -> bool {
        // ## m4b-t14-tabs (named exception)
        matches!(self.status.tier, Tier::Reduced | Tier::Preview)
    }

    /// How often it rendered (tests: typing re-renders it, an unrelated change does not).
    pub fn renders(&self) -> u64 {
        self.renders
    }
}

impl Render for EditorSegment {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.renders += 1;
        let t = Theme::get(cx).tokens;
        // The status bar's cell gives it its padding and seam and lets it give way (Task 34's
        // row): the segment itself shrinks, clipping, and adds neither.
        // ## m3c-t41-chrome: the status bar shows the parts in cells of their own
        // ([`SegmentCell`]); the whole segment draws them side by side.
        let mut row = div()
            .id("editor-segment")
            .flex()
            .flex_row()
            .items_center()
            .min_w(px(0.0))
            .overflow_hidden()
            .gap(px(t.spacing.md));
        for part in self.parts.clone() {
            if let Some(element) = part_element(part, self.status, &self.editor, cx) {
                row = row.child(element);
            }
        }
        row
    }
}

// ## m3c-t41-chrome
/// One part of the segment as the status bar draws it in its own cell.
pub struct SegmentCell {
    segment: Entity<EditorSegment>,
    part: SegmentPart,
    _observe: Subscription,
}

impl SegmentCell {
    /// The cell of `segment`'s `part`; it repaints with the segment.
    pub fn new(segment: &Entity<EditorSegment>, part: SegmentPart, cx: &mut Context<Self>) -> Self {
        SegmentCell {
            segment: segment.clone(),
            part,
            _observe: cx.observe(segment, |_, _, cx| cx.notify()),
        }
    }

    /// The part it shows.
    pub fn part(&self) -> SegmentPart {
        self.part
    }
}

impl Render for SegmentCell {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (status, editor) = {
            let s = self.segment.read(cx);
            (s.status, s.editor.clone())
        };
        let selector = match self.part {
            SegmentPart::Reduced => "status-cell-reduced",
            SegmentPart::LineEndings => "status-cell-eol",
            SegmentPart::Carets => "status-cell-carets",
            SegmentPart::Selected => "status-cell-selected",
            SegmentPart::LnCol => "status-cell-lncol",
        };
        div()
            .debug_selector(move || selector.to_string())
            .flex()
            .flex_row()
            .items_center()
            .min_w(px(0.0))
            .children(part_element(self.part, status, &editor, cx))
    }
}

/// The element of `part` for `status`; `None` when the part shows nothing.
fn part_element(
    part: SegmentPart,
    s: EditorStatus,
    editor: &WeakEntity<EditorView>,
    cx: &App,
) -> Option<AnyElement> {
    let theme = *Theme::get(cx);
    let t = theme.tokens;
    let ink = t.colors.text_2.to_hsla();
    let style = t.type_ramp.caption;
    match part {
        SegmentPart::Reduced if s.tier == Tier::Reduced => Some(
            Label::new(
                tr(cx, ids::hv::EDITOR_REDUCED, NoArgs),
                TextSource::Fluent,
                style,
                t.colors.text.to_hsla(),
            )
            .id("editor-segment-reduced")
            .into_any_element(),
        ),
        // ## m4b-t14-tabs (named exception, carries-w1 T14/T18): "Read-only preview".
        SegmentPart::Reduced if s.tier == Tier::Preview => Some(
            Label::new(
                crate::scripts::tabs::msg(cx, "hv-scripts-preview-segment", Vec::new()),
                TextSource::Fluent,
                style,
                t.colors.text.to_hsla(),
            )
            .id("editor-segment-preview")
            .into_any_element(),
        ),
        // ## m3c-t34-editor (named exception, NOTES 17 and M3c-16): "Mixed line endings", after
        // Reduced mode; its menu normalises to one ending (never automatic).
        // ## fix round 1 (M3): not on a read-only editor.
        SegmentPart::LineEndings
            if s.mixed_line_endings && editor.upgrade().is_some_and(|e| eol_offered(&e, cx)) =>
        {
            Some(eol_button(editor.clone(), cx))
        }
        // ## m3c-t31-multi (named exception, NOTES surface 14): "3 carets" before the selected
        // count, from two carets on; it gives way with it.
        SegmentPart::Carets if s.carets > 1 => Some(
            Label::new(
                tr(
                    cx,
                    ids::hv::EDITOR_CARETS,
                    ids::hv::HvEditorCarets {
                        n: Count(u64::from(s.carets)),
                    },
                ),
                TextSource::Fluent,
                style,
                ink,
            )
            .truncate(true)
            .id("editor-segment-carets")
            .into_any_element(),
        ),
        SegmentPart::Selected if s.selected_chars > 0 => Some(
            Label::new(
                tr(
                    cx,
                    ids::hv::EDITOR_SELECTED,
                    ids::hv::HvEditorSelected {
                        n: Count(s.selected_chars),
                    },
                ),
                TextSource::Fluent,
                style,
                ink,
            )
            // ## m3a-t35-shots (named exception): ellipsized when the bar gives way, never
            // cut mid-glyph (the 960 px Turkish shots).
            .truncate(true)
            .id("editor-segment-selected")
            .into_any_element(),
        ),
        SegmentPart::LnCol => Some(
            Label::new(
                tr(
                    cx,
                    ids::hv::EDITOR_LNCOL,
                    ids::hv::HvEditorLncol {
                        line: Count(u64::from(s.line)),
                        col: Count(u64::from(s.column)),
                    },
                ),
                TextSource::Fluent,
                style,
                ink,
            )
            // ## m3a-t35-shots (named exception): as the selected count.
            .truncate(true)
            .id("editor-segment-lncol")
            .into_any_element(),
        ),
        _ => None,
    }
}

// ## m3c-t34-editor
/// Whether `editor` is offered the "Mixed line endings" button: its lines end in more than one
/// way and it is editable (## fix round 1, M3: a read-only editor's rows would do nothing).
pub fn eol_offered(editor: &Entity<EditorView>, cx: &App) -> bool {
    let e = editor.read(cx);
    e.status().mixed_line_endings && !e.config().read_only
}

/// The endings `editor`'s text has, as their names (data), in LF, CRLF, CR order.
fn endings_present(editor: &Entity<EditorView>, cx: &App) -> Vec<&'static str> {
    let s = editor.read(cx).line_endings();
    [("LF", s.lf), ("CRLF", s.crlf), ("CR", s.cr)]
        .into_iter()
        .filter(|(_, n)| *n > 0)
        .map(|(name, _)| name)
        .collect()
}

/// The button's tooltip, naming the actual mix (## fix round 1, M3).
pub fn eol_tip(editor: &Entity<EditorView>, cx: &App) -> SharedString {
    let mix = endings_present(editor, cx).join(", ");
    tr(
        cx,
        ids::hv::EOL_MIXED_TIP,
        ids::hv::HvEolMixedTip {
            mix: hacivat_l10n::Text(mix),
        },
    )
}

/// The status bar's "Mixed line endings" button (`text` ink): its menu opens above it.
fn eol_button(editor: WeakEntity<EditorView>, cx: &App) -> AnyElement {
    let theme = *Theme::get(cx);
    let t = theme.tokens;
    let label = tr(cx, ids::hv::EOL_MIXED, NoArgs);
    let tip = editor
        .upgrade()
        .map_or_else(SharedString::default, |e| eol_tip(&e, cx));
    div()
        .id("editor-segment-eol")
        .flex_none()
        .cursor_pointer()
        .child(
            Label::new(
                label.clone(),
                TextSource::Fluent,
                t.type_ramp.caption,
                t.colors.text.to_hsla(),
            )
            .id("editor-segment-eol-label"),
        )
        .on_click(move |ev, window, cx| {
            let at = ev.position();
            open_eol_menu(editor.clone(), point(at.x, at.y - px(4.0)), window, cx);
        })
        .tooltip(move |_, cx| Tip::view(tip.clone(), cx))
        .declare(Role::Button, label, None)
        .into_any_element()
}

/// The line endings menu (NOTES 17): "Line endings: LF 214, CRLF 12", then the dominant ending
/// first with "most lines", then the others; a row rewrites every ending as one undo group.
pub fn open_eol_menu(
    editor: WeakEntity<EditorView>,
    anchor: Point<Pixels>,
    window: &mut Window,
    cx: &mut App,
) {
    let Some(e) = editor.upgrade() else {
        return;
    };
    if !eol_offered(&e, cx) {
        return;
    }
    let items = eol_menu_items(&e, cx);
    let Some(ws) = window
        .root::<crate::shell::workspace::Workspace>()
        .flatten()
    else {
        return;
    };
    let menu = ws.update(cx, |ws, cx| ws.open_popup_menu(anchor, items, window, cx));
    menu.update(cx, |m: &mut PopupMenu, _| m.set_access_keys(false));
    cx.subscribe(&menu, move |_, event: &PopupMenuEvent, cx| {
        if let PopupMenuEvent::Chosen(action) = event {
            let Some(to) = ending_of(action.as_ref()) else {
                return;
            };
            if let Some(e) = editor.upgrade() {
                e.update(cx, |e, cx| {
                    e.normalize_line_endings_to(to, cx);
                });
            }
        }
    })
    .detach();
}

/// The menu's row actions, by ending.
const EOL_ROWS: [(&str, &str, LineEnding); 3] = [
    ("LF", "eol.lf", LineEnding::Lf),
    ("CRLF", "eol.crlf", LineEnding::CrLf),
    ("CR", "eol.cr", LineEnding::Cr),
];

fn ending_of(action: &str) -> Option<LineEnding> {
    EOL_ROWS
        .iter()
        .find(|(_, a, _)| *a == action)
        .map(|(_, _, e)| *e)
}

/// The line endings menu's rows for `editor`: the counts (CR only when some line ends so),
/// the dominant ending (the editor's own rule) first with "most lines", then LF and CRLF, and
/// CR when the text has any (## fix round 1, M3).
pub fn eol_menu_items(editor: &Entity<EditorView>, cx: &App) -> Vec<MenuItemSpec> {
    let s = editor.read(cx).line_endings();
    let dominant = editor.read(cx).snapshot().buffer.dominant_line_ending();
    let (lf, crlf, cr) = (u64::from(s.lf), u64::from(s.crlf), u64::from(s.cr));
    let counts: TextFn = if cr > 0 {
        Rc::new(move |cx: &App| {
            tr(
                cx,
                ids::hv::EOL_COUNTS_CR,
                ids::hv::HvEolCountsCr {
                    lf: Count(lf),
                    crlf: Count(crlf),
                    cr: Count(cr),
                },
            )
        })
    } else {
        Rc::new(move |cx: &App| {
            tr(
                cx,
                ids::hv::EOL_COUNTS,
                ids::hv::HvEolCounts {
                    lf: Count(lf),
                    crlf: Count(crlf),
                },
            )
        })
    };
    let row = |name: &'static str, action: &'static str| {
        MenuItemSpec::new(
            Rc::new(move |cx: &App| {
                tr(
                    cx,
                    ids::hv::EOL_TO,
                    ids::hv::HvEolTo {
                        ending: hacivat_l10n::Text(name.to_string()),
                    },
                )
            }),
            action,
        )
    };
    let most: TextFn = Rc::new(|cx: &App| tr(cx, ids::hv::EOL_MOST, NoArgs));
    let mut items = vec![MenuItemSpec::new(counts, "eol.counts").note(true)];
    if let Some((name, action, _)) = EOL_ROWS.iter().find(|(_, _, e)| *e == dominant) {
        items.push(row(name, action).hint(most));
    }
    for (name, action, ending) in EOL_ROWS {
        if ending != dominant && (ending != LineEnding::Cr || cr > 0) {
            items.push(row(name, action));
        }
    }
    items
}
