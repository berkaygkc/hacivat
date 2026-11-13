//! The title-bar connection capsule (DESIGN.md, Title-Bar Capsule): a 26 px segmented capsule,
//! 4 px radius. The production block is the only `cp_fill` quad in a window, with its label
//! in `capsule_env` small caps; it arrives pre-cased from Fluent (`PRODUCTION`, `PROD`).
//!
//! The M2a live states (NOTES §7 and "NEW primitives": capsule states) add segments in the same
//! anatomy, divided by `seam-strong`:
//! - connecting: the 7 px `amber` run marker, the label in `text`, then the step and the clock
//!   in mono 11.5 `text-2`;
//! - connected: the access (a lock that is red only on production, or a `text-2` read-write
//!   label), the path (green arrows for Direct, the route glyph otherwise, latency in mono), an
//!   "unencrypted" segment (lock-open 12 px `text-2`, label `text`) and an "unsupported" mark;
//! - failed: the environment block and the name, a 12 px alert glyph in `text` with the label,
//!   then a Retry segment (rotate-cw, `text-2`, `text` while the capsule is hovered).
//!
//! The production block always comes first. Debug selectors: `capsule`, `cp-fill`,
//! `capsule-body` (the name segment), `capsule-run`, `capsule-retry`, `capsule-unencrypted` and
//! `capsule-unsupported`.

use std::cell::Cell;
use std::rc::Rc;

use hacivat_ui_core::gpui_surface::{
    AnyElement, App, Bounds, ClickEvent, Div, ElementId, Global, Hsla, InteractiveElement,
    IntoElement, ParentElement, Pixels, Point, Role, SharedString, Stateful,
    StatefulInteractiveElement, Styled, Window, canvas, div, px,
};
use hacivat_ui_core::theme::{Theme, ThemeTokens, TypeStyle};

use crate::button::{ClickHandler, hover_listener, interaction_state};
use crate::icon::{Icon, IconName, IconSize};
use crate::popover::{A11yDecl, A11yNode, Declare, lazy};
use crate::step_trail::run_marker;
use crate::text::{Label, TextSource};

/// The size of every environment mark (DESIGN.md: state marks are 7 px).
const MARK_PX: f32 = 7.0;
/// The dev mark's centre dot (NOTES "Environment marks: dev, test").
const DEV_DOT_PX: f32 = 3.0;
/// The step and clock keep at least this width so the capsule does not jump as they tick
/// (mock `.cp-run .mono { min-width: 44px }`).
const RUN_MONO_MIN_PX: f32 = 44.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EnvKind {
    Production,
    Staging,
    Local,
    Other,
    // M2a (Task 1; marks from Task 9).
    Dev,
    Test,
}

/// The 7 px environment mark (NOTES cross-cutting #2): circles for personal and pre-release
/// environments, squares for shared servers.
/// - production: a filled `red` square (the only hue);
/// - staging: a hollow `text-2` square;
/// - local: a hollow `text-3` circle;
/// - dev: a hollow `text-2` circle with a 3 px `text-2` centre dot (◉);
/// - test: a filled `text-2` circle (●).
///
/// `Other` has no mark (an empty 0 px element).
// ## m2a-t17-dialog
/// [`env_swatch`] on a raised surface (a dialog): the local mark's outline is `text-2`, never
/// text-3 (DESIGN.md Raised Ink Rule); every other mark is unchanged.
pub fn env_swatch_on_raised(kind: EnvKind, theme: &Theme) -> Div {
    let swatch = env_swatch(kind, theme);
    match kind {
        EnvKind::Local => swatch.border_color(theme.tokens.colors.text_2.to_hsla()),
        _ => swatch,
    }
}

pub fn env_swatch(kind: EnvKind, theme: &Theme) -> Div {
    let c = &theme.tokens.colors;
    let base = div().size(px(MARK_PX)).flex_none();
    match kind {
        EnvKind::Production => base.bg(c.red.to_hsla()),
        EnvKind::Staging => base.border_1().border_color(c.text_2.to_hsla()),
        EnvKind::Local => base
            .border_1()
            .border_color(c.text_3.to_hsla())
            .rounded_full(),
        EnvKind::Dev => base
            .flex()
            .items_center()
            .justify_center()
            .border_1()
            .border_color(c.text_2.to_hsla())
            .rounded_full()
            .child(
                div()
                    .debug_selector(|| "env-dev-dot".to_string())
                    .size(px(DEV_DOT_PX))
                    .flex_none()
                    .rounded_full()
                    .bg(c.text_2.to_hsla()),
            ),
        EnvKind::Test => base.bg(c.text_2.to_hsla()).rounded_full(),
        EnvKind::Other => div().size(px(0.0)).flex_none(),
    }
}

/// A clickable part of the capsule (M2a, NOTES surface 7).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CapsuleSegment {
    Body,
    Retry,
    Unencrypted,
}

/// Called with the clicked segment and the bottom-left of its bounds, the anchor for
/// `open_anchored` (P4).
pub type SegmentHandler = Rc<dyn Fn(CapsuleSegment, Point<Pixels>, &mut Window, &mut App)>;

/// What the capsule shows. Every string is chrome text from Fluent except the connection's
/// name and engine, which are data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CapsuleModel {
    /// No connection yet.
    Placeholder { label: SharedString },
    /// A connect in progress.
    Connecting {
        env: EnvKind,
        env_label: SharedString,
        name: SharedString,
        engine: SharedString,
        label: SharedString,
        step: SharedString,
        clock: SharedString,
    },
    Connection {
        env: EnvKind,
        /// Pre-cased by Fluent (`PRODUCTION` in English, `PROD` in Turkish).
        env_label: SharedString,
        name: SharedString,
        engine: SharedString,
        access: SharedString,
        read_only: bool,
        path: SharedString,
        direct: bool,
        latency: Option<SharedString>,
        /// The unencrypted badge's label, when the connection has no TLS.
        unencrypted: Option<SharedString>,
        /// The unsupported-version badge's label.
        unsupported: Option<SharedString>,
    },
    /// The last connect failed. The environment block still comes first (a failed production
    /// connection keeps its PRODUCTION block, NOTES §7).
    Failed {
        env: EnvKind,
        env_label: SharedString,
        label: SharedString,
        name: SharedString,
        retry: SharedString,
    },
}

pub struct Capsule {
    model: CapsuleModel,
    on_click: Option<ClickHandler>,
    on_segment: Option<SegmentHandler>,
    inert: bool,
}

impl Capsule {
    pub fn new(model: CapsuleModel) -> Self {
        Self {
            model,
            on_click: None,
            on_segment: None,
            inert: false,
        }
    }

    /// Ignores the pointer and is announced as disabled, with the same look (the title bar
    /// while a modal layer is open, M2a).
    pub fn inert(mut self, inert: bool) -> Self {
        self.inert = inert;
        self
    }

    /// Called when a segment is clicked, with the bottom-left corner of its bounds: `Retry` and
    /// `Unencrypted` for those segments, `Body` (anchored at the capsule's bottom-left) for a
    /// click anywhere else. Exactly one handler runs per click: with `on_segment` set, a click on
    /// the body reports `Body` here and [`Capsule::on_click`] is not called.
    pub fn on_segment(
        mut self,
        handler: impl Fn(CapsuleSegment, Point<Pixels>, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_segment = Some(Rc::new(handler));
        self
    }

    /// Called for a click on the capsule when no [`Capsule::on_segment`] handler is set (M1).
    pub fn on_click(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_click = Some(Rc::new(handler));
        self
    }
}

/// One cell after the environment block: 9 px padding, a `seam_strong` divider on its left.
fn cell(t: &ThemeTokens) -> Div {
    let c = &t.colors;
    div()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(t.spacing.sm))
        .px(px(9.0))
        .h_full()
        .flex_none()
        .border_l_1()
        .border_color(c.seam_strong.to_hsla())
        .group_hover("capsule", |s| s.bg(c.hover.to_hsla()))
}

/// Records the bounds of its parent while it is prepainted (an absolutely placed, empty canvas
/// covering the parent), so a click handler can anchor at the segment it hit. Absolute children
/// are placed inside the parent's border, so the probe reaches out by the parent's border
/// widths (`left` on the left, `rest` on the other sides) to report its outer bounds.
fn bounds_probe(slot: Rc<Cell<Bounds<Pixels>>>, left: f32, rest: f32) -> impl IntoElement {
    canvas(move |bounds, _, _| slot.set(bounds), |_, _, _, _| {})
        .absolute()
        .left(px(-left))
        .top(px(-rest))
        .right(px(-rest))
        .bottom(px(-rest))
}

// ## m2a-t21-shots
/// Where the capsule was painted in the current frame (its frame, border included; `None` when
/// this frame has not drawn it), recorded only while this global is installed: the shots
/// harness and tests scope the One Filled Badge check to it
/// (`named_rules::assert_one_filled_badge_in`). It is reset with each frame's first debug
/// record (`text::begin_record`). The app never installs it.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct CapsuleProbe {
    pub bounds: Option<Bounds<Pixels>>,
}

impl Global for CapsuleProbe {}

/// Records the frame's bounds into [`CapsuleProbe`] when it is installed.
fn record_frame() -> impl IntoElement {
    canvas(
        |bounds, _, cx| {
            if cx.has_global::<CapsuleProbe>() {
                // The probe starts empty with each frame's first record, so a frame without
                // the capsule never reports the bounds of an earlier one.
                crate::text::begin_record(cx);
                cx.global_mut::<CapsuleProbe>().bounds = Some(bounds);
            }
        },
        |_, _, _, _| {},
    )
    .absolute()
    .left(px(-1.0))
    .top(px(-1.0))
    .right(px(-1.0))
    .bottom(px(-1.0))
}

/// The parts every live state shares: the environment block and the name segment.
struct Head<'a> {
    env: Option<(EnvKind, &'a SharedString)>,
    name: &'a SharedString,
    engine: Option<&'a SharedString>,
}

impl IntoElement for Capsule {
    type Element = AnyElement;

    fn into_element(self) -> AnyElement {
        lazy(move |window, cx| self.build(window, cx)).into_any_element()
    }
}

impl Capsule {
    fn build(self, window: &mut Window, cx: &mut App) -> AnyElement {
        let theme = *Theme::get(cx);
        let t = theme.tokens;
        let c = &t.colors;
        let body = t.type_ramp.body;
        let strong = TypeStyle {
            weight: 600,
            ..body
        };
        let mono = t.type_ramp.mono_data;
        // Chrome text is Fluent; the connection's name and engine are user data.
        let text = |s: &SharedString, style, color: Hsla| {
            Label::new(s.clone(), TextSource::Fluent, style, color)
        };
        let data = |s: &SharedString, style, color: Hsla| {
            Label::new(s.clone(), TextSource::Data, style, color)
        };
        let icon = |name, color: Hsla| Icon::new(name).size(IconSize::Chrome).color(color);

        let frame_id = ElementId::from("capsule");
        let (hover, _) = interaction_state(&frame_id, window, cx);
        let hovered = *hover.read(cx);
        let frame_bounds = Rc::new(Cell::new(Bounds::default()));
        let mut frame = div()
            .id(frame_id)
            .group("capsule")
            .debug_selector(|| "capsule".to_string())
            .relative()
            .flex()
            .flex_row()
            .items_center()
            .flex_none()
            .h(px(26.0))
            .border_1()
            .border_color(c.seam_strong.to_hsla())
            .rounded(px(t.radii.sm))
            .overflow_hidden()
            .bg(c.panel.to_hsla())
            .on_hover(hover_listener(hover))
            .child(bounds_probe(frame_bounds.clone(), 1.0, 1.0))
            .child(record_frame());

        // A segment that reports itself (and not `Body`) when clicked; announced as a button,
        // disabled while the capsule is inert.
        let inert = self.inert;
        let on_segment = if inert { None } else { self.on_segment.clone() };
        let clickable = |id: &'static str,
                         segment: CapsuleSegment,
                         name: &SharedString,
                         content: [AnyElement; 2]| {
            let slot = Rc::new(Cell::new(Bounds::default()));
            let mut el = cell(t)
                .id(id)
                .debug_selector(move || id.to_string())
                .relative()
                .child(bounds_probe(slot.clone(), 1.0, 0.0))
                .children(content);
            if let Some(handler) = on_segment.clone() {
                el = el.on_click(move |_, window, cx| {
                    cx.stop_propagation();
                    handler(segment, slot.get().bottom_left(), window, cx);
                });
            }
            // As `Button`: an enabled segment declares its role on the clickable element itself,
            // so an assistive-technology Click reaches `on_click`; an inert one is wrapped in a
            // node that can say "disabled".
            if inert {
                A11yNode::new(
                    ElementId::Name(format!("{id}-node").into()),
                    Role::Button,
                    name.clone(),
                    false,
                    el,
                )
                .disabled(true)
                .into_any_element()
            } else {
                el.declare(Role::Button, name.clone(), None)
                    .into_any_element()
            }
        };

        let head = |frame: Stateful<Div>, head: Head<'_>| {
            let mut frame = frame;
            match head.env {
                Some((EnvKind::Production, env_label)) => {
                    // The one cp_fill quad in the window (named rule), always first.
                    frame = frame.child(
                        div()
                            .debug_selector(|| "cp-fill".to_string())
                            .flex()
                            .flex_row()
                            .items_center()
                            .h_full()
                            .flex_none()
                            .pl(px(9.0))
                            .pr(px(10.0))
                            .rounded_tl(px(t.radii.xs))
                            .rounded_bl(px(t.radii.xs))
                            .bg(c.cp_fill.to_hsla())
                            .child(text(
                                env_label,
                                t.type_ramp.capsule_env,
                                c.on_cp_fill.to_hsla(),
                            )),
                    );
                }
                Some((env, env_label)) => {
                    // A non-production tag: the mark and the tag in text-2 (NOTES).
                    frame = frame.child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(px(t.spacing.sm))
                            .h_full()
                            .flex_none()
                            .pl(px(9.0))
                            .pr(px(10.0))
                            .group_hover("capsule", |s| s.bg(c.hover.to_hsla()))
                            .child(env_swatch(env, &theme))
                            .child(text(env_label, body, c.text_2.to_hsla())),
                    );
                }
                None => {}
            }
            let mut name_cell = cell(t)
                .debug_selector(|| "capsule-body".to_string())
                .child(data(head.name, strong, c.text.to_hsla()));
            if head.env.is_none() {
                name_cell = name_cell.border_l_0();
            }
            if let Some(engine) = head.engine {
                name_cell = name_cell.child(data(engine, body, c.text_2.to_hsla()));
            }
            frame.child(name_cell)
        };

        let name_for_label;
        let mut description = None;
        match &self.model {
            CapsuleModel::Placeholder { label } => {
                name_for_label = label.clone();
                frame = frame.child(div().flex().items_center().px(px(9.0)).child(text(
                    label,
                    body,
                    c.text_3.to_hsla(),
                )));
            }
            CapsuleModel::Connecting {
                env,
                env_label,
                name,
                engine,
                label,
                step,
                clock,
            } => {
                name_for_label = name.clone();
                description = Some(SharedString::from(format!("{env_label}, {label}, {step}")));
                frame = head(
                    frame,
                    Head {
                        env: Some((*env, env_label)),
                        name,
                        engine: Some(engine),
                    },
                );
                frame = frame.child(
                    cell(t)
                        .child(run_marker(&theme).debug_selector(|| "capsule-run".to_string()))
                        .child(text(label, body, c.text.to_hsla()))
                        .child(div().min_w(px(RUN_MONO_MIN_PX)).child(text(
                            step,
                            mono,
                            c.text_2.to_hsla(),
                        )))
                        .child(div().min_w(px(RUN_MONO_MIN_PX)).child(text(
                            clock,
                            mono,
                            c.text_2.to_hsla(),
                        ))),
                );
            }
            CapsuleModel::Connection {
                env,
                env_label,
                name,
                engine,
                access,
                read_only,
                path,
                direct,
                latency,
                unencrypted,
                unsupported,
            } => {
                name_for_label = name.clone();
                let mut parts = vec![env_label.clone(), access.clone(), path.clone()];
                parts.extend(unencrypted.iter().cloned());
                parts.extend(unsupported.iter().cloned());
                description = Some(SharedString::from(parts.join(", ")));
                frame = head(
                    frame,
                    Head {
                        env: Some((*env, env_label)),
                        name,
                        engine: Some(engine),
                    },
                );
                let production = *env == EnvKind::Production;
                // Hue Is Meaning: the lock is red only on production.
                let access_cell = if *read_only {
                    let lock = if production { c.red } else { c.text_2 };
                    cell(t)
                        .child(icon(IconName::Lock, lock.to_hsla()))
                        .child(text(access, body, c.text.to_hsla()))
                } else {
                    cell(t).child(text(access, body, c.text_2.to_hsla()))
                };
                let (path_icon, path_ink) = if *direct {
                    (IconName::ArrowLeftRight, c.green)
                } else {
                    (IconName::Route, c.text_2)
                };
                let mut path_cell = cell(t)
                    .child(icon(path_icon, path_ink.to_hsla()))
                    .child(text(path, body, c.text.to_hsla()));
                if let Some(latency) = latency {
                    path_cell = path_cell.child(text(latency, mono, c.text_2.to_hsla()));
                }
                frame = frame.child(access_cell).child(path_cell);
                if let Some(label) = unencrypted {
                    frame = frame.child(clickable(
                        "capsule-unencrypted",
                        CapsuleSegment::Unencrypted,
                        label,
                        [
                            icon(IconName::LockOpen, c.text_2.to_hsla()).into_any_element(),
                            text(label, body, c.text.to_hsla()).into_any_element(),
                        ],
                    ));
                }
                if let Some(label) = unsupported {
                    // A permanent, hue-free mark (PO Q3).
                    frame = frame.child(
                        cell(t)
                            .debug_selector(|| "capsule-unsupported".to_string())
                            .child(icon(IconName::CircleAlert, c.text_2.to_hsla()))
                            .child(text(label, body, c.text.to_hsla())),
                    );
                }
            }
            CapsuleModel::Failed {
                env,
                env_label,
                label,
                name,
                retry,
            } => {
                name_for_label = name.clone();
                description = Some(SharedString::from(format!("{env_label}, {label}")));
                frame = head(
                    frame,
                    Head {
                        env: Some((*env, env_label)),
                        name,
                        engine: None,
                    },
                );
                // No new hue for errors: the alert glyph is ink.
                let retry_ink = if hovered { c.text } else { c.text_2 };
                frame = frame
                    .child(
                        cell(t)
                            .child(icon(IconName::TriangleAlert, c.text.to_hsla()))
                            .child(text(label, body, c.text.to_hsla())),
                    )
                    .child(clickable(
                        "capsule-retry",
                        CapsuleSegment::Retry,
                        retry,
                        [
                            icon(IconName::RotateCw, retry_ink.to_hsla()).into_any_element(),
                            text(retry, body, retry_ink.to_hsla()).into_any_element(),
                        ],
                    ));
            }
        }
        frame = frame.child(
            cell(t).px(px(6.0)).child(
                Icon::new(IconName::ChevronsUpDown)
                    .size(IconSize::Chrome)
                    .color(c.text_3.to_hsla()),
            ),
        );
        // Exactly one handler per click: the segment handler wins, `on_click` is the M1 fallback.
        if !inert {
            frame = frame.cursor_pointer();
            if let Some(handler) = self.on_segment {
                frame = frame.on_click(move |_, window, cx| {
                    handler(
                        CapsuleSegment::Body,
                        frame_bounds.get().bottom_left(),
                        window,
                        cx,
                    )
                });
            } else if let Some(handler) = self.on_click {
                frame = frame.on_click(move |event, window, cx| handler(event, window, cx));
            }
        }
        // As `Button`: declared on the clickable frame while enabled, wrapped while inert.
        if !inert {
            let mut decl = A11yDecl::new(Role::Button, name_for_label);
            decl.description = description;
            return frame.declare_full(decl).into_any_element();
        }
        let mut node = A11yNode::new("capsule-node", Role::Button, name_for_label, false, frame)
            .disabled(true);
        if let Some(description) = description {
            node = node.description(description);
        }
        node.into_any_element()
    }
}

// ## m2a-t17-dialog
#[cfg(test)]
mod raised_swatch_tests {
    use super::*;
    use hacivat_ui_core::theme::{DARK, ThemeMode};

    #[test]
    fn the_local_mark_on_a_raised_surface_is_outlined_in_text_2() {
        let theme = Theme::new(ThemeMode::Dark);
        let mut raised = env_swatch_on_raised(EnvKind::Local, &theme);
        assert_eq!(
            raised.style().border_color,
            Some(DARK.colors.text_2.to_hsla())
        );
        let mut plain = env_swatch(EnvKind::Local, &theme);
        assert_eq!(
            plain.style().border_color,
            Some(DARK.colors.text_3.to_hsla())
        );
        let mut staging = env_swatch_on_raised(EnvKind::Staging, &theme);
        assert_eq!(
            staging.style().border_color,
            Some(DARK.colors.text_2.to_hsla())
        );
    }
}
