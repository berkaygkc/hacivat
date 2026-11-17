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
    AnyElement, App, Pixels, Point, Role, StatefulInteractiveElement as _, WeakEntity, point,
};

use crate::l10n_global::tr;

/// The status bar's editor segment of one console.
pub struct EditorSegment {
    status: EditorStatus,
    renders: u64,
    _subscription: Subscription,
    // ## m3c-t34-editor (named exception): the editor the line endings menu normalises.
    editor: WeakEntity<EditorView>,
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
                    cx.notify();
                }
            }
        });
        EditorSegment {
            status,
            renders: 0,
            _subscription: subscription,
            editor: editor.downgrade(),
        }
    }

    /// What the segment shows.
    pub fn status(&self) -> EditorStatus {
        self.status
    }

    /// The editor is in reduced mode: the segment's "Reduced mode" cell never gives way.
    pub fn reduced(&self) -> bool {
        self.status.tier == Tier::Reduced
    }

    /// How often it rendered (tests: typing re-renders it, an unrelated change does not).
    pub fn renders(&self) -> u64 {
        self.renders
    }
}

impl Render for EditorSegment {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.renders += 1;
        let theme = *Theme::get(cx);
        let t = theme.tokens;
        let ink = t.colors.text_2.to_hsla();
        let style = t.type_ramp.caption;
        let s = self.status;
        // The status bar's cell gives it its padding and seam and lets it give way (Task 34's
        // row): the segment itself shrinks, clipping, and adds neither.
        let mut row = div()
            .id("editor-segment")
            .flex()
            .flex_row()
            .items_center()
            .min_w(px(0.0))
            .overflow_hidden()
            .gap(px(t.spacing.md));
        if s.tier == Tier::Reduced {
            row = row.child(
                Label::new(
                    tr(cx, ids::hv::EDITOR_REDUCED, NoArgs),
                    TextSource::Fluent,
                    style,
                    t.colors.text.to_hsla(),
                )
                .id("editor-segment-reduced"),
            );
        }
        // ## m3c-t34-editor (named exception, NOTES 17 and M3c-16): "Mixed line endings", after
        // Reduced mode; its menu normalises to one ending (never automatic).
        if s.mixed_line_endings {
            row = row.child(eol_button(self.editor.clone(), cx));
        }
        // ## m3c-t31-multi (named exception, NOTES surface 14): "3 carets" before the selected
        // count, from two carets on; it gives way with it.
        if s.carets > 1 {
            row = row.child(
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
                .id("editor-segment-carets"),
            );
        }
        if s.selected_chars > 0 {
            row = row.child(
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
                .id("editor-segment-selected"),
            );
        }
        row.child(
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
            .id("editor-segment-lncol"),
        )
    }
}

// ## m3c-t34-editor
/// The status bar's "Mixed line endings" button (`text` ink): its menu opens above it.
fn eol_button(editor: WeakEntity<EditorView>, cx: &App) -> AnyElement {
    let theme = *Theme::get(cx);
    let t = theme.tokens;
    let label = tr(cx, ids::hv::EOL_MIXED, NoArgs);
    let tip = tr(cx, ids::hv::EOL_MIXED_TIP, NoArgs);
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
/// first with "most lines", then the other; a row rewrites every ending as one undo group.
pub fn open_eol_menu(
    editor: WeakEntity<EditorView>,
    anchor: Point<Pixels>,
    window: &mut Window,
    cx: &mut App,
) {
    let Some(e) = editor.upgrade() else {
        return;
    };
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
            let to = match action.as_ref() {
                EOL_LF => LineEnding::Lf,
                EOL_CRLF => LineEnding::CrLf,
                _ => return,
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

/// The menu's row actions.
const EOL_LF: &str = "eol.lf";
const EOL_CRLF: &str = "eol.crlf";

/// The line endings menu's rows for `editor`.
pub fn eol_menu_items(editor: &Entity<EditorView>, cx: &App) -> Vec<MenuItemSpec> {
    let s = editor.read(cx).line_endings();
    let (lf, crlf) = (u64::from(s.lf), u64::from(s.crlf));
    let counts: TextFn = Rc::new(move |cx: &App| {
        tr(
            cx,
            ids::hv::EOL_COUNTS,
            ids::hv::HvEolCounts {
                lf: Count(lf),
                crlf: Count(crlf),
            },
        )
    });
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
    let (first, second) = if crlf > lf {
        (row("CRLF", EOL_CRLF), row("LF", EOL_LF))
    } else {
        (row("LF", EOL_LF), row("CRLF", EOL_CRLF))
    };
    vec![
        MenuItemSpec::new(counts, "eol.counts").note(true),
        first.hint(most),
        second,
    ]
}
