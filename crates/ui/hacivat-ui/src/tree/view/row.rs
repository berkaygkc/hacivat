//! One row's anatomy and its give-way order.
//!
//! **Order** (NOTES M4 surface 1): chevron · kind icon · name · type text · badge · flags ·
//! status · pill · spacer · meta mark + meta · row action. 23 px rows (`tree_row_height`), 14 px
//! per level from an 8 px base (`tree_indent`), a 12 px chevron (swapped, never rotated), the
//! 12 px kind icon (`text-2`; `blue` while selected), the 13 px name (`body`, `body_strong` on a
//! connected root), the type text in mono 11 px at `text-2`, the outlined badge (`micro`; One
//! Filled Badge holds), the flags (environment mark; lock: red only on production), the status
//! glyph (amber/green/text-2/text-3, never red), the 16 px level pill (`inlay`; blue when set by
//! the user), the meta mark and the 11 px mono meta at `text-3`. Hover fills `hover`; a selected
//! row fills `blue-tint` with a 1 px `blue` left edge (a child fill: GPUI paints a border-only
//! quad as four strips). The keyboard-focus ring is a 1 px `blue` overlay on the cursor row (no
//! layout shift), shown only after keyboard input. Nothing is drawn below 11 px.
//!
//! **Give-way order** (NOTES M4 cross-cutting #5, through [`crate::give_way::give_way`], fix
//! round 1, I2): when a row is narrower than its cells, (1) the meta ellipsizes and then its
//! mark leaves, (2) the pill leaves whole, (3) the badge collapses to its glyph (the text becomes
//! the glyph's tooltip; a badge without a glyph leaves whole), (4) the type text ellipsizes, and
//! (5) the name ellipsizes down to about seven characters (its full text is its tooltip). The
//! chevron, the icon, the flags, the status glyph and the row action never give way. Widths are
//! the shaped widths of the texts (GPUI caches the line layouts between frames).
//!
//! **Group headers** are a small-caps label at `text-3`, the filter glyph when the host hides
//! objects ([`TreeRowData::filter_glyph`]), a `seam-soft` rule and a mono count; they draw no
//! flags. **Loading** rows draw the static amber run marker in the chevron column when
//! running, the label at 12 px `text-2` (it gives way first) and the mono count; **failed** rows
//! draw the alert glyph in the chevron column, the label in `text` ink and the retry button.
//! No motion anywhere.

use hacivat_ui_core::fonts::font_for;
use hacivat_ui_core::gpui_surface::{
    AnyElement, Context, Div, Hsla, InteractiveElement, IntoElement, MouseButton, MouseDownEvent,
    ParentElement, SharedString, Stateful, StatefulInteractiveElement as _, Styled, TextRun,
    Window, div, px,
};
use hacivat_ui_core::theme::{Theme, TypeStyle};

use super::TreeView;
use super::element::row_id;
use crate::capsule::env_swatch;
use crate::give_way::{Yield, give_way};
use crate::icon::{Icon, IconName, IconSize};
use crate::popover::Tip;
use crate::step_trail::run_marker;
use crate::text::{Label, TextSource};
use crate::tree::flat::RowKind;
use crate::tree::{ChildState, MetaMark, RowFlag, StatusKind, TreeEvent, TreeModel, TreeRowData};

/// How much of a name a crowded row keeps: about seven characters and the ellipsis (NOTES M4
/// cross-cutting #5; the controller's mock-fallback ruling over M2a's six).
pub const NAME_MIN_CHARS: usize = 7;
/// The environment mark's size (`capsule::env_swatch`).
const ENV_MARK_PX: f32 = 7.0;
/// The run marker's size (`step_trail::run_marker`).
const RUN_MARK_PX: f32 = 7.0;
/// The level pill's and the ghost buttons' height.
const PILL_H: f32 = 16.0;
/// Give-way ranks: the highest leaves first.
const RANK_META: u8 = 6;
const RANK_MARK: u8 = 5;
const RANK_PILL: u8 = 4;
const RANK_BADGE: u8 = 3;
const RANK_TYPE: u8 = 2;
const RANK_NAME: u8 = 1;

/// The shaped width of `text` in `style`, as a `Label` lays it out.
fn text_w(text: &SharedString, style: TypeStyle, window: &mut Window) -> f32 {
    if text.is_empty() {
        return 0.0;
    }
    let run = TextRun {
        len: text.len(),
        font: font_for(&style),
        color: Hsla::default(),
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    f32::from(
        window
            .text_system()
            .shape_line(text.clone(), px(style.size_px), &[run], None)
            .width,
    )
    .ceil()
}

/// The narrowest a name gets: its first [`NAME_MIN_CHARS`] characters and the ellipsis, or the
/// whole name when it is that short.
fn name_floor(name: &SharedString, style: TypeStyle, window: &mut Window) -> f32 {
    let mut chars = name.chars();
    let head: String = chars.by_ref().take(NAME_MIN_CHARS).collect();
    if chars.next().is_none() {
        return text_w(name, style, window);
    }
    text_w(&format!("{head}…").into(), style, window)
}

/// The cells of a node row, in order, for [`give_way`].
#[derive(Default)]
struct Plan {
    yields: Vec<Yield>,
    widths: Vec<f32>,
}

impl Plan {
    fn push(&mut self, y: Yield, w: f32) -> usize {
        self.yields.push(y);
        self.widths.push(w);
        self.yields.len() - 1
    }
}

/// What [`give_way`] left of a cell: `None` when it gave way, else `Some(width)` and whether it
/// is narrower than its natural width.
fn decided(shown: &[Option<f32>], natural: &[f32], i: Option<usize>) -> Option<(f32, bool)> {
    let i = i?;
    shown[i].map(|w| (w.min(natural[i]), w + 0.5 < natural[i]))
}

impl<M: TreeModel> TreeView<M> {
    pub(super) fn build_row(
        &mut self,
        ix: usize,
        ring: bool,
        row_w: f32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = *Theme::get(cx);
        let t = theme.tokens;
        let c = &t.colors;
        let row = self.flat.rows()[ix];
        let indent = t.spacing.md + f32::from(row.depth) * t.spacing.tree_indent;
        let chevron_w = IconSize::Chrome.px();
        let gap = t.spacing.xs;
        let key = row.key;
        let is_cursor = ring && self.cursor == Some(key) && row.kind == RowKind::Node;
        let base = div()
            .id(row_id(key, row.kind))
            .relative()
            .flex()
            .flex_row()
            .items_center()
            .w_full()
            .h(px(self.row_h))
            .pl(px(indent))
            .pr(px(t.spacing.md));
        // The keyboard-focus ring: an overlay, so the row's content never shifts by a border.
        let ring_overlay = || {
            div()
                .absolute()
                .inset_0()
                .border_1()
                .border_color(c.blue.to_hsla())
        };
        // A ghost icon button (retry, the row action): pointer only, its word as the tooltip.
        let ghost = |id: &'static str, icon: IconName, tip: SharedString| {
            div()
                .id(id)
                .flex_none()
                .ml(px(gap))
                .size(px(PILL_H))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(t.radii.xs))
                .cursor_pointer()
                .hover(|s| s.bg(c.hover.to_hsla()))
                .child(
                    Icon::new(icon)
                        .size(IconSize::Chrome)
                        .color(c.text_2.to_hsla()),
                )
                .tooltip(move |_, cx| Tip::view(tip.clone(), cx))
        };

        if row.kind != RowKind::Node {
            let mut marker = div()
                .flex_none()
                .w(px(chevron_w))
                .flex()
                .items_center()
                .justify_center();
            return match self.model.children(key) {
                ChildState::Loading {
                    label,
                    running,
                    count,
                } => {
                    if running {
                        marker = marker
                            .debug_selector(move || format!("hv-tree-run-{}", key.0))
                            .child(run_marker(&theme));
                    }
                    let mut el = base.child(marker).child(
                        div().ml(px(gap)).min_w(px(0.0)).flex_shrink(1.0).child(
                            Label::new(
                                label,
                                TextSource::Fluent,
                                t.type_ramp.caption,
                                c.text_2.to_hsla(),
                            )
                            .truncate(true)
                            .id("hv-tree-status-label"),
                        ),
                    );
                    if let Some(count) = count {
                        el = el.child(div().flex_none().ml(px(gap)).child(Label::new(
                            count,
                            TextSource::Fluent,
                            t.type_ramp.mono_micro,
                            c.text_2.to_hsla(),
                        )));
                    }
                    el.into_any_element()
                }
                ChildState::Failed { label, retry } => {
                    marker = marker.child(
                        Icon::new(IconName::CircleAlert)
                            .size(IconSize::Chrome)
                            .color(c.text.to_hsla()),
                    );
                    let mut el = base.child(marker).child(
                        div().ml(px(gap)).min_w(px(0.0)).flex_shrink(1.0).child(
                            Label::new(
                                label,
                                TextSource::Fluent,
                                t.type_ramp.body,
                                c.text.to_hsla(),
                            )
                            .truncate(true)
                            .id("hv-tree-status-label"),
                        ),
                    );
                    if let Some(word) = retry {
                        el = el.child(div().flex_1()).child(
                            ghost("hv-tree-retry", IconName::RotateCw, word)
                                .debug_selector(move || format!("hv-tree-retry-{}", key.0))
                                .on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener(move |this, _: &MouseDownEvent, _, cx| {
                                        this.on_row_button(
                                            key,
                                            |_| Some(TreeEvent::Retry(key)),
                                            cx,
                                        );
                                    }),
                                ),
                        );
                    }
                    el.into_any_element()
                }
                _ => base.into_any_element(),
            };
        }

        let Some(data) = self.model.row(key) else {
            return base.into_any_element();
        };
        let expandable = self.model.children(key).expandable();
        let open = self.expanded.contains(&key);
        let chevron = if expandable {
            div()
                .id("hv-tree-chevron")
                .flex_none()
                .w(px(chevron_w))
                .cursor_pointer()
                .child(
                    Icon::new(if open {
                        IconName::ChevronDown
                    } else {
                        IconName::ChevronRight
                    })
                    .size(IconSize::Chrome)
                    .color(c.text_3.to_hsla()),
                )
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                        this.on_chevron_down(key, e, window, cx);
                    }),
                )
                .into_any_element()
        } else {
            div().flex_none().w(px(chevron_w)).into_any_element()
        };
        let mouse = || {
            cx.listener(move |this: &mut Self, e: &MouseDownEvent, window, cx| {
                this.on_row_mouse_down(key, e, window, cx);
            })
        };
        let hover = cx.listener(move |this: &mut Self, hovered: &bool, _, cx| {
            this.on_row_hover(key, *hovered, cx);
        });
        let base = base.on_hover(hover);
        let base = match data.tooltip.clone() {
            Some(tip) => base.tooltip(move |_, cx| Tip::view(tip.clone(), cx)),
            None => base,
        };

        if data.group_header {
            return Self::header_row(
                base,
                chevron,
                data,
                is_cursor.then(ring_overlay),
                gap,
                &theme,
            )
            .on_mouse_down(MouseButton::Left, mouse())
            .on_mouse_down(MouseButton::Right, mouse())
            .into_any_element();
        }

        let selected = self.selection.contains(&key);
        let ink = if data.dim { c.text_3 } else { c.text };
        let icon_ink = if selected {
            c.blue
        } else if data.dim {
            c.text_3
        } else {
            c.text_2
        };
        let name_style = if data.strong {
            t.type_ramp.body_strong
        } else {
            t.type_ramp.body
        };
        let show_action = data.row_action.is_some() && (selected || self.hovered == Some(key));

        // The give-way plan: each cell's natural width, its leading gap included.
        let mut plan = Plan::default();
        plan.push(Yield::Never, chevron_w);
        if data.icon.is_some() {
            plan.push(Yield::Never, gap + chevron_w + t.spacing.s_2xs);
        }
        let name_w = gap + text_w(&data.label, name_style, window);
        let name_floor = gap + name_floor(&data.label, name_style, window);
        let name_i = Some(plan.push(
            Yield::Shrink {
                floor: name_floor.min(name_w),
                rank: RANK_NAME,
            },
            name_w,
        ));
        let type_i = data.type_text.as_ref().map(|s| {
            let w = gap + text_w(s, t.type_ramp.mono_micro, window);
            plan.push(
                Yield::Shrink {
                    floor: gap,
                    rank: RANK_TYPE,
                },
                w,
            )
        });
        let badge_box = 2.0 + 2.0 * t.spacing.xs;
        let badge_glyph_i = match (&data.badge, data.badge_glyph) {
            (Some(_), Some(_)) => Some(plan.push(Yield::Never, gap + badge_box + chevron_w)),
            _ => None,
        };
        let badge_text_i = data.badge.as_ref().map(|b| {
            let text = text_w(b, t.type_ramp.micro, window);
            let w = if badge_glyph_i.is_some() {
                t.spacing.s_2xs + text
            } else {
                gap + badge_box + text
            };
            plan.push(Yield::Whole(RANK_BADGE), w)
        });
        for flag in &data.flags {
            let w = match flag {
                RowFlag::Env(_) => ENV_MARK_PX,
                RowFlag::Lock { .. } | RowFlag::Icon { .. } => chevron_w,
            };
            plan.push(Yield::Never, gap + w);
        }
        if let Some(status) = &data.status {
            let w = gap + text_w(&status.glyph, t.type_ramp.mono_micro, window);
            plan.push(Yield::Never, w);
        }
        let pill_i = data.pill.as_ref().map(|p| {
            let w = gap + badge_box + text_w(&p.text, t.type_ramp.inlay, window);
            plan.push(Yield::Whole(RANK_PILL), w)
        });
        let mark_i = data.meta_mark.map(|m| {
            let w = match m {
                MetaMark::Running => RUN_MARK_PX,
                MetaMark::Ok | MetaMark::Glyph(_) => chevron_w,
            };
            plan.push(Yield::Whole(RANK_MARK), gap + w)
        });
        let meta_i = data.meta.as_ref().map(|m| {
            let w = gap + text_w(m, t.type_ramp.mono_micro, window);
            plan.push(
                Yield::Shrink {
                    floor: gap,
                    rank: RANK_META,
                },
                w,
            )
        });
        if show_action {
            plan.push(Yield::Never, gap + PILL_H);
        }
        let available = (row_w - indent - t.spacing.md).max(0.0);
        let shown = give_way(&plan.yields, &plan.widths, available);
        let natural = &plan.widths;

        let mut el = base.child(chevron);
        if let Some(icon) = data.icon {
            el = el.child(
                div()
                    .debug_selector(move || format!("hv-tree-icon-{}", key.0))
                    .flex_none()
                    .ml(px(gap))
                    .pr(px(t.spacing.s_2xs))
                    .child(
                        Icon::new(icon)
                            .size(IconSize::Chrome)
                            .color(icon_ink.to_hsla()),
                    ),
            );
        }
        // A text cell: natural, or cut to the width it was given (its text then ellipsizes).
        let text_cell = |w: (f32, bool), sel: &'static str| {
            let cell = div()
                .debug_selector(move || format!("{sel}-{}", key.0))
                .flex_none()
                .ml(px(gap))
                .flex()
                .min_w(px(0.0));
            if w.1 {
                cell.w(px((w.0 - gap).max(0.0)))
            } else {
                cell
            }
        };
        if let Some(w) = decided(&shown, natural, name_i) {
            el = el.child(
                text_cell(w, "hv-tree-name").child(
                    Label::new(data.label, data.label_source, name_style, ink.to_hsla())
                        .truncate(true)
                        .id("hv-tree-label"),
                ),
            );
        }
        if let (Some(w), Some(text)) = (decided(&shown, natural, type_i), data.type_text)
            && w.0 > gap + 0.5
        {
            let type_ink = if data.dim { c.text_3 } else { c.text_2 };
            el = el.child(
                text_cell(w, "hv-tree-type").child(
                    Label::new(
                        text,
                        TextSource::Data,
                        t.type_ramp.mono_micro,
                        type_ink.to_hsla(),
                    )
                    .truncate(true)
                    .id("hv-tree-type-label"),
                ),
            );
        }
        if let Some(badge) = data.badge {
            let text_shown = decided(&shown, natural, badge_text_i).is_some();
            if text_shown || data.badge_glyph.is_some() {
                // Outlined: the capsule keeps the window's one filled badge.
                let mut b = div()
                    .id("hv-tree-badge")
                    .debug_selector(move || format!("hv-tree-badge-{}", key.0))
                    .flex_none()
                    .ml(px(gap))
                    .flex()
                    .items_center()
                    .gap(px(t.spacing.s_2xs))
                    .px(px(t.spacing.xs))
                    .border_1()
                    .border_color(c.seam_strong.to_hsla())
                    .rounded(px(t.radii.xs));
                if let Some(glyph) = data.badge_glyph {
                    b = b.child(
                        Icon::new(glyph)
                            .size(IconSize::Chrome)
                            .color(c.text_2.to_hsla()),
                    );
                }
                if text_shown {
                    b = b.child(Label::new(
                        badge,
                        TextSource::Fluent,
                        t.type_ramp.micro,
                        c.text_2.to_hsla(),
                    ));
                } else {
                    // Collapsed to its glyph: the text is the glyph's tooltip.
                    b = b.tooltip(move |_, cx| Tip::view(badge.clone(), cx));
                }
                el = el.child(b);
            }
        }
        // Trust signals never shrink or clip (Global Constraints; NOTES cross-cutting #5).
        for (i, flag) in data.flags.into_iter().enumerate() {
            let mark = match flag {
                RowFlag::Env(kind) => env_swatch(kind, &theme).into_any_element(),
                RowFlag::Lock { production } => Icon::new(IconName::Lock)
                    .size(IconSize::Chrome)
                    .color(if production { c.red } else { c.text_2 }.to_hsla())
                    .into_any_element(),
                RowFlag::Icon { icon, .. } => Icon::new(icon)
                    .size(IconSize::Chrome)
                    .color(c.text_2.to_hsla())
                    .into_any_element(),
            };
            el = el.child(
                div()
                    .debug_selector(move || format!("hv-tree-flag-{}-{i}", key.0))
                    .flex_none()
                    .ml(px(gap))
                    .flex()
                    .items_center()
                    .child(mark),
            );
        }
        if let Some(status) = data.status {
            // Its own hue per letter (demofix-1, PO F5); the text alternative is the item's
            // description, so the letter and the words carry it, never the colour alone.
            // ## m4b-t13-explorer (named exception, controller ruling R-T2-1 a)
            // Fix round 1 (m6): the recorder notes the very value the label is drawn in.
            let ink = status_ink(status.kind, c).to_hsla();
            status_note(status.kind, ink);
            el = el.child(div().flex_none().ml(px(gap)).child(Label::new(
                status.glyph,
                TextSource::Fluent,
                t.type_ramp.mono_micro,
                ink,
            )));
        }
        if let (Some(pill), Some(i)) = (data.pill, pill_i)
            && shown[i].is_some()
        {
            // Where the pill starts in the row: the cells before it as they were laid out.
            let x = indent
                + (0..i)
                    .filter_map(|j| decided(&shown, natural, Some(j)).map(|w| w.0))
                    .sum::<f32>()
                + gap;
            let (border, text) = if pill.explicit {
                (c.blue, c.blue)
            } else {
                (c.seam_strong, c.text_3)
            };
            let tip = pill.tooltip;
            el = el.child(
                div()
                    .id("hv-tree-pill")
                    .debug_selector(move || format!("hv-tree-pill-{}", key.0))
                    .flex_none()
                    .ml(px(gap))
                    .h(px(PILL_H))
                    .flex()
                    .items_center()
                    .px(px(t.spacing.xs))
                    .border_1()
                    .border_color(border.to_hsla())
                    .rounded(px(t.radii.xs))
                    .cursor_pointer()
                    .child(Label::new(
                        pill.text,
                        TextSource::Fluent,
                        t.type_ramp.inlay,
                        text.to_hsla(),
                    ))
                    .tooltip(move |_, cx| Tip::view(tip.clone(), cx))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _: &MouseDownEvent, _, cx| {
                            this.on_row_button(
                                key,
                                |this| {
                                    this.pill_anchor(key, x, PILL_H)
                                        .map(|at| TreeEvent::Pill(key, at))
                                },
                                cx,
                            );
                        }),
                    ),
            );
        }
        el = el.child(div().flex_1());
        if let (Some(mark), true) = (data.meta_mark, decided(&shown, natural, mark_i).is_some()) {
            let mark = match mark {
                MetaMark::Running => run_marker(&theme).into_any_element(),
                MetaMark::Ok => Icon::new(IconName::Check)
                    .size(IconSize::Chrome)
                    .color(c.green.to_hsla())
                    .into_any_element(),
                MetaMark::Glyph(icon) => Icon::new(icon)
                    .size(IconSize::Chrome)
                    .color(c.text_3.to_hsla())
                    .into_any_element(),
            };
            el = el.child(
                div()
                    .debug_selector(move || format!("hv-tree-mark-{}", key.0))
                    .flex_none()
                    .ml(px(gap))
                    .flex()
                    .items_center()
                    .child(mark),
            );
        }
        if let (Some(w), Some(meta)) = (decided(&shown, natural, meta_i), data.meta)
            && w.0 > gap + 0.5
        {
            el = el.child(
                text_cell(w, "hv-tree-meta").child(
                    Label::new(
                        meta,
                        TextSource::Data,
                        t.type_ramp.mono_micro,
                        c.text_3.to_hsla(),
                    )
                    .truncate(true)
                    .id("hv-tree-meta"),
                ),
            );
        }
        if show_action && let Some(action) = data.row_action {
            el = el.child(
                ghost("hv-tree-row-action", action.icon, action.label)
                    .debug_selector(move || format!("hv-tree-row-action-{}", key.0))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _: &MouseDownEvent, _, cx| {
                            this.on_row_button(key, |_| Some(TreeEvent::RowAction(key)), cx);
                        }),
                    ),
            );
        }
        el = if selected {
            // The 1 px blue left edge: a child quad, so no row carries a border.
            el.bg(c.blue_tint.to_hsla()).child(
                div()
                    .absolute()
                    .left_0()
                    .top_0()
                    .w(px(1.0))
                    .h_full()
                    .bg(c.blue.to_hsla()),
            )
        } else {
            el.hover(|s| s.bg(c.hover.to_hsla()))
        };
        if is_cursor {
            el = el.child(ring_overlay());
        }
        el.on_mouse_down(MouseButton::Left, mouse())
            .on_mouse_down(MouseButton::Right, mouse())
            .into_any_element()
    }

    /// A group header: chevron, small-caps label, filter glyph, rule, count. No flags.
    fn header_row(
        base: Stateful<Div>,
        chevron: AnyElement,
        data: TreeRowData,
        ring: Option<Div>,
        gap: f32,
        theme: &Theme,
    ) -> Stateful<Div> {
        let t = theme.tokens;
        let c = &t.colors;
        let mut el = base
            .child(chevron)
            .child(div().flex_none().ml(px(gap)).child(Label::new(
                data.label,
                data.label_source,
                t.type_ramp.label_sc,
                c.text_3.to_hsla(),
            )));
        if let Some(tip) = data.filter_glyph {
            el = el.child(
                div()
                    .id("hv-tree-filter-glyph")
                    .debug_selector(|| "hv-tree-filter-glyph".to_string())
                    .flex_none()
                    .ml(px(gap))
                    .child(
                        Icon::new(IconName::Filter)
                            .size(IconSize::Chrome)
                            .color(c.text_3.to_hsla()),
                    )
                    .tooltip(move |_, cx| Tip::view(tip.clone(), cx)),
            );
        }
        el = el.child(
            div()
                .flex_1()
                .ml(px(gap))
                .h(px(1.0))
                .bg(c.seam_soft.to_hsla()),
        );
        if let Some(meta) = data.meta {
            el = el.child(div().flex_none().ml(px(gap)).child(Label::new(
                meta,
                TextSource::Fluent,
                t.type_ramp.mono_micro,
                c.text_3.to_hsla(),
            )));
        }
        if let Some(ring) = ring {
            el = el.child(ring);
        }
        el
    }
}

// ## m4b-t13-explorer (named exception, controller ruling R-T2-1 a)
/// The status glyph's ink. ## demofix-1 (PO demo F5, superseding R-T2-1 a's one amber): M
/// `git-modified` (amber), N `git-new` (green), each its own token apart from Pending Amber,
/// Direct-Path Green and Production Red; the letters stay, so colour is never the only cue.
/// Conflicted ink (a refusal-like state, Errors Without A New Hue), the folder dot `text-2`,
/// Unknown `text-3`. `git-deleted` (red, D) is for a deleted summary, which no row shows yet.
/// The glyph stays `mono_micro` (weight 400).
fn status_ink(
    kind: StatusKind,
    c: &hacivat_ui_core::theme::Colors,
) -> hacivat_ui_core::theme::Color {
    match kind {
        StatusKind::Modified => c.git_modified,
        StatusKind::New => c.git_new,
        StatusKind::Conflicted => c.text,
        StatusKind::ContainsChanges => c.text_2,
        StatusKind::Unknown => c.text_3,
    }
}
// ## m4b-t13-explorer

// ## demofix-1 (PO demo F5)
thread_local! {
    static STATUS_DRAWN: std::cell::RefCell<Option<Vec<(StatusKind, hacivat_ui_core::gpui_surface::Hsla)>>> =
        const { std::cell::RefCell::new(None) };
}

/// Starts (or restarts) recording the status glyphs rows draw on this thread, with their
/// ink. GPUI's test platform rasterizes no text, so a check that the git letters do not share
/// one colour reads what the rows asked for.
pub fn status_record() {
    STATUS_DRAWN.with(|d| *d.borrow_mut() = Some(Vec::new()));
}

/// Stops recording; the status glyphs drawn since [`status_record`], in drawing order.
pub fn status_take() -> Vec<(StatusKind, hacivat_ui_core::gpui_surface::Hsla)> {
    STATUS_DRAWN.with(|d| d.borrow_mut().take().unwrap_or_default())
}

fn status_note(kind: StatusKind, ink: hacivat_ui_core::gpui_surface::Hsla) {
    STATUS_DRAWN.with(|d| {
        if let Some(log) = d.borrow_mut().as_mut() {
            log.push((kind, ink));
        }
    });
}
// ## demofix-1

/// Reachable for the row-key helpers' tests.
#[cfg(test)]
mod tests {
    use super::{Yield, give_way};

    // ## m4b-t13-explorer (named exception)
    #[test]
    fn git_status_inks_follow_ruling_r_t2_1_a() {
        use super::{StatusKind, status_ink};
        for tokens in [
            &hacivat_ui_core::theme::DARK,
            &hacivat_ui_core::theme::LIGHT,
        ] {
            let c = &tokens.colors;
            // demofix-1 (PO F5): M and N each their own hue.
            assert_eq!(status_ink(StatusKind::Modified, c), c.git_modified);
            assert_eq!(status_ink(StatusKind::New, c), c.git_new);
            assert_ne!(c.git_modified, c.git_new);
            assert_eq!(status_ink(StatusKind::Conflicted, c), c.text);
            assert_eq!(status_ink(StatusKind::ContainsChanges, c), c.text_2);
            assert_eq!(status_ink(StatusKind::Unknown, c), c.text_3);
            for k in [
                StatusKind::Modified,
                StatusKind::New,
                StatusKind::Conflicted,
                StatusKind::ContainsChanges,
                StatusKind::Unknown,
            ] {
                // Never a trust or state hue: production red, the capsule fill, pending
                // amber, direct-path green.
                for (hue, name) in [
                    (c.red, "red"),
                    (c.cp_fill, "cp-fill"),
                    (c.amber, "amber"),
                    (c.green, "green"),
                ] {
                    assert_ne!(status_ink(k, c), hue, "{k:?} is never {name}");
                }
            }
            // Mono micro, weight 400.
            assert_eq!(tokens.type_ramp.mono_micro.weight, 400);
        }
    }
    // ## m4b-t13-explorer

    #[test]
    fn the_give_way_ranks_follow_notes_cross_cutting_5() {
        // name (shrink, floor 30) · badge text (whole) · pill (whole) · meta (shrink, floor 0)
        let y = [
            Yield::Shrink {
                floor: 30.0,
                rank: super::RANK_NAME,
            },
            Yield::Whole(super::RANK_BADGE),
            Yield::Whole(super::RANK_PILL),
            Yield::Shrink {
                floor: 0.0,
                rank: super::RANK_META,
            },
        ];
        let w = [100.0, 20.0, 20.0, 40.0];
        // 10 px short: the meta ellipsizes.
        assert_eq!(
            give_way(&y, &w, 170.0),
            [Some(100.0), Some(20.0), Some(20.0), Some(30.0)]
        );
        // 50 px short: the meta is at its floor, the pill leaves (its room goes back).
        let shown = give_way(&y, &w, 130.0);
        assert_eq!(shown[2], None);
        assert_eq!(shown[0], Some(100.0));
        // Narrower still: the badge leaves, then the name shrinks to its floor.
        let shown = give_way(&y, &w, 60.0);
        assert_eq!((shown[1], shown[2]), (None, None));
        assert!(shown[0].is_some_and(|n| (30.0..100.0).contains(&n)));
    }
}
