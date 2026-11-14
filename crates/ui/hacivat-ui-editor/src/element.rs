//! The editor element (UI-EDIT-07 paint, -08 scroll; Task 19): one custom element for the text
//! and the gutter area, in the harman paint order, calling `gutter::paint`, `marks::paint`,
//! `preview::paint_band`, `scrollbar::paint` at their paint points and the `a11y` seams from
//! its accessibility hooks; each frame's layout is kept for `EditorView::last_layout` (P11).
//!
//! Since Task 1b's review rounds, [`EditorElement`] is that element and already calls every
//! module seam with the frame's text:
//! - `request_layout`: the [`A11yFrame`] (only while assistive technology is active);
//! - the accessibility hooks: `a11y::{role, write_info, synthetic_children}`;
//! - `prepaint` ([`prepaint_frame`]): `ime::on_frame`, the frame's geometry, `marks::prepaint`
//!   (hit targets need prepaint);
//! - `paint` ([`paint_frame`]): the input handler, `gutter::paint`, `preview::paint_band`,
//!   `marks::paint`, `scrollbar::paint`.
//!
//! Tasks 25 and 27–29 fill module bodies only. Task 19 replaces the placeholder geometry (a
//! fixed line height, no scrolling, nothing kept for `last_layout`) and keeps the calls.

use hacivat_text::Snapshot;
use hacivat_text::display::ScrollAnchor;
use hacivat_ui_core::gpui_surface::{
    A11ySubtreeBuilder, App, Bounds, Context, Element, ElementId, ElementInputHandler, Entity,
    FocusHandle, GlobalElementId, InspectorElementId, LayoutId, Pixels, Point, Role, Style, Window,
    accesskit, point, px, relative, size,
};

use crate::a11y::{self, A11yFrame, A11yState};
use crate::gutter::{self, GutterCtx};
use crate::marks::{self, MarkHitboxes};
use crate::preview;
use crate::scrollbar::{self, ScrollbarCtx};
use crate::view::EditorView;

/// The painted rows of one frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VisibleRows {
    pub bounds: Bounds<Pixels>,
    pub line_height: Pixels,
    pub first_line: u32,
    pub last_line: u32,
    /// The top of `first_line`, in window coordinates.
    pub first_y: Pixels,
}

/// The last painted frame (Task 19 fills it): what IME bounds and hit testing read.
#[derive(Debug, Clone, PartialEq)]
pub struct FrameLayout {
    rows: VisibleRows,
}

impl FrameLayout {
    /// A frame over `rows` (Task 19 adds the shaped lines).
    pub(crate) fn new(rows: VisibleRows) -> Self {
        FrameLayout { rows }
    }

    /// The rows the frame painted.
    pub fn rows(&self) -> &VisibleRows {
        &self.rows
    }

    /// The x of byte `offset` on a painted line; `None` off the painted lines (Task 19).
    pub fn x_for_offset(&self, offset: usize) -> Option<Pixels> {
        let _ = offset;
        None
    }

    /// The byte offset at `p`; `None` off the painted lines (Task 19).
    pub fn offset_for_point(&self, p: Point<Pixels>) -> Option<usize> {
        let _ = p;
        None
    }
}

/// Task 1b's placeholder geometry (Task 19 measures the font and the gutter digits).
const LINE_HEIGHT: f32 = 20.0;
const GUTTER_WIDTH: f32 = 58.0;
const SCROLLBAR_WIDTH: f32 = 8.0;

/// One frame's geometry and prepaint results, from [`prepaint_frame`] to [`paint_frame`].
#[derive(Debug)]
pub struct FramePrepaint {
    pub snapshot: Snapshot,
    pub rows: VisibleRows,
    pub gutter: Bounds<Pixels>,
    pub track: Bounds<Pixels>,
    pub total_lines: u32,
    pub visible_lines: u32,
    pub caret_line: u32,
    pub digits: u8,
    pub current_statement: Option<std::ops::Range<u32>>,
    /// `marks::prepaint`'s hit targets.
    pub mark_hits: MarkHitboxes,
}

impl FramePrepaint {
    /// The gutter's paint context over this frame.
    fn gutter(&self) -> GutterCtx<'_> {
        GutterCtx {
            bounds: self.gutter,
            line_height: self.rows.line_height,
            first_line: self.rows.first_line,
            last_line: self.rows.last_line,
            digits: self.digits,
            current_line: self.caret_line,
            scroll: ScrollAnchor::default(),
            snapshot: &self.snapshot,
            current_statement: self.current_statement.clone(),
        }
    }
}

/// The prepaint half of a frame: the input handler's owed notifications, the geometry, and the
/// marks' hit targets (Task 19 owns this function and its calls).
pub(crate) fn prepaint_frame(
    v: &mut EditorView,
    bounds: Bounds<Pixels>,
    window: &mut Window,
    cx: &mut Context<EditorView>,
) -> FramePrepaint {
    crate::ime::on_frame(v, window, cx);
    let snapshot = v.model.snapshot();
    let line_height = px(LINE_HEIGHT);
    let total_lines = u32::try_from(snapshot.buffer.len_lines()).unwrap_or(u32::MAX);
    let visible_lines = (f32::from(bounds.size.height) / LINE_HEIGHT)
        .ceil()
        .max(1.0) as u32;
    let last_line = visible_lines.min(total_lines).saturating_sub(1);
    let caret_line = u32::try_from(snapshot.buffer.byte_to_line(snapshot.selection.main().head))
        .unwrap_or(u32::MAX);
    let digits = (total_lines.max(1).ilog10() + 1).max(3) as u8;
    let gutter = Bounds::new(bounds.origin, size(px(GUTTER_WIDTH), bounds.size.height));
    let text = Bounds::new(
        point(bounds.origin.x + px(GUTTER_WIDTH), bounds.origin.y),
        size(
            (bounds.size.width - px(GUTTER_WIDTH + SCROLLBAR_WIDTH)).max(px(0.0)),
            bounds.size.height,
        ),
    );
    let track = Bounds::new(
        point(
            bounds.origin.x + bounds.size.width - px(SCROLLBAR_WIDTH),
            bounds.origin.y,
        ),
        size(px(SCROLLBAR_WIDTH), bounds.size.height),
    );
    let current_statement = preview::current_statement(&v.band, &snapshot);
    let mut frame = FramePrepaint {
        snapshot,
        rows: VisibleRows {
            bounds: text,
            line_height,
            first_line: 0,
            last_line,
            first_y: bounds.origin.y,
        },
        gutter,
        track,
        total_lines,
        visible_lines,
        caret_line,
        digits,
        current_statement,
        mark_hits: MarkHitboxes::default(),
    };
    frame.mark_hits = marks::prepaint(&v.marks, &frame.gutter(), window, cx);
    frame
}

/// The paint half of a frame, in the harman order: gutter, line backgrounds (the band), text
/// (Task 19), marks, scrollbar (Task 19 owns this function and its calls).
pub(crate) fn paint_frame(
    v: &mut EditorView,
    frame: &FramePrepaint,
    window: &mut Window,
    cx: &mut Context<EditorView>,
) {
    let g = frame.gutter();
    gutter::paint(&g, window, cx);
    preview::paint_band(&v.band, &frame.rows, &frame.snapshot, window, cx);
    let layout = FrameLayout::new(frame.rows);
    marks::paint(&v.marks, &g, &layout, &frame.mark_hits, window, cx);
    scrollbar::paint(
        &ScrollbarCtx {
            track: frame.track,
            total_lines: frame.total_lines,
            first_line: frame.rows.first_line,
            visible_lines: frame.visible_lines,
            current_statement: frame.current_statement.clone(),
            snapshot: &frame.snapshot,
        },
        &v.marks,
        window,
        cx,
    );
}

/// The editor's element: the text area, its input handler and its accessibility node.
pub(crate) struct EditorElement {
    view: Entity<EditorView>,
    focus: FocusHandle,
    id: ElementId,
    a11y: A11yState,
    /// Built in `request_layout` only while assistive technology is active.
    a11y_frame: Option<A11yFrame>,
}

impl EditorElement {
    pub(crate) fn new(
        view: Entity<EditorView>,
        focus: FocusHandle,
        id: ElementId,
        a11y: A11yState,
    ) -> Self {
        EditorElement {
            view,
            focus,
            id,
            a11y,
            a11y_frame: None,
        }
    }
}

impl Element for EditorElement {
    type RequestLayoutState = ();
    type PrepaintState = Option<FramePrepaint>;

    fn id(&self) -> Option<ElementId> {
        Some(self.id.clone())
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        self.a11y_frame = if window.is_a11y_active() {
            let focused = self.focus.is_focused(window);
            Some(self.view.update(cx, |v, cx| v.a11y_frame(focused, cx)))
        } else {
            None
        };
        // The rest of the editor's column (the banner sits above it), as `flex_1().w_full()`.
        let mut style = Style {
            flex_grow: 1.0,
            flex_shrink: 1.0,
            flex_basis: relative(0.0).into(),
            ..Style::default()
        };
        style.size.width = relative(1.0).into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) -> Option<FramePrepaint> {
        Some(
            self.view
                .update(cx, |v, cx| prepaint_frame(v, bounds, window, cx)),
        )
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut (),
        prepaint: &mut Option<FramePrepaint>,
        window: &mut Window,
        cx: &mut App,
    ) {
        window.handle_input(
            &self.focus,
            ElementInputHandler::new(bounds, self.view.clone()),
            cx,
        );
        if let Some(frame) = prepaint.as_ref() {
            self.view
                .update(cx, |v, cx| paint_frame(v, frame, window, cx));
        }
    }

    fn a11y_role(&self) -> Option<Role> {
        self.a11y_frame
            .as_ref()
            .and_then(|f| a11y::role(&self.a11y, f))
    }

    fn write_a11y_info(&self, node: &mut accesskit::Node) {
        if let Some(f) = &self.a11y_frame {
            a11y::write_info(&self.a11y, f, node);
        }
    }

    fn a11y_synthetic_children(
        &mut self,
        _prepaint: &mut Option<FramePrepaint>,
        builder: &mut A11ySubtreeBuilder,
    ) {
        if let Some(f) = &self.a11y_frame {
            a11y::synthetic_children(&self.a11y, f, builder);
        }
    }
}
