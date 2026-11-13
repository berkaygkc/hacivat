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
//!
//! M2b (Task 22, NOTES M2a §7 and M2b surfaces 6–7):
//! - reconnecting: the run marker, the attempt label in `text` and, between attempts, when the
//!   next one starts in mono `text-2`;
//! - a read-only access segment is a button and a Tab stop (`capsule-access`): a click, Enter or
//!   Space reports [`CapsuleSegment::Access`] (the explanation popover); the focus ring is a 1 px
//!   `blue` line inset 2 px;
//! - idle: after an idle disconnect, the environment, the name and an unplug glyph with its
//!   label, both `text-2` (`capsule-idle`).
//!
//! Label IDs for tests and tools: `capsule-text-run`, `capsule-text-next`, `capsule-text-access`,
//! `capsule-text-path`, `capsule-text-idle`.

use std::cell::Cell;
use std::rc::Rc;

use hacivat_ui_core::fonts::font_for;
use hacivat_ui_core::gpui_surface::{
    AnyElement, App, Bounds, ClickEvent, Div, ElementId, Global, Hsla, InteractiveElement,
    IntoElement, KeyDownEvent, ParentElement, Pixels, Point, Role, SharedString,
    StatefulInteractiveElement, Styled, TextRun, Window, canvas, div, px,
};
use hacivat_ui_core::theme::{Theme, ThemeTokens, TypeStyle};

use crate::button::{ClickHandler, focus_visible, hover_listener, interaction_state};
use crate::icon::{Icon, IconName, IconSize};
use crate::popover::{A11yDecl, A11yNode, Declare, Tip, lazy};
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
    // ## m2b-t22-live
    /// The read-only access segment (NOTES M2b surface 6).
    Access,
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
    // ## m2b-t01-interfaces
    /// Reconnecting after a lost connection (Task 22): the attempt label and, between
    /// attempts, when the next one starts.
    Reconnecting {
        env: EnvKind,
        env_label: SharedString,
        name: SharedString,
        engine: SharedString,
        label: SharedString,
        next: Option<SharedString>,
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
    // ## m2b-t22-live
    /// Disconnected after being idle (B18): it reconnects when used (NOTES M2b surface 7).
    Idle {
        env: EnvKind,
        env_label: SharedString,
        name: SharedString,
        engine: SharedString,
        label: SharedString,
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
    // ## m2b-t22-live (fix round 1, I1, M5)
    max_width: Option<f32>,
    access_hint: Option<SharedString>,
}

impl Capsule {
    pub fn new(model: CapsuleModel) -> Self {
        Self {
            model,
            on_click: None,
            on_segment: None,
            inert: false,
            max_width: None,
            access_hint: None,
        }
    }

    // ## m2b-t22-live (fix round 1, ruling I1)
    /// The widest the capsule may be (the title bar's free space). Past it the connected
    /// capsule degrades in a fixed order: the latency goes, then the path, the access and the
    /// unencrypted segments keep only their icons (their full text stays the tooltip and the
    /// accessible name), and last the connection's name ellipsizes. The environment block
    /// never shrinks.
    ///
    /// Fix round 2 (R1-I1): the other states degrade too (the engine, a reconnect's
    /// countdown, then the state's text to its glyph: connecting, reconnecting, idle,
    /// "Couldn't connect"; then Retry's text to its icon), and the capsule is clamped to
    /// `max_width`: it is never laid out wider, whatever is left to give up.
    pub fn max_width(mut self, max_width: Option<f32>) -> Self {
        self.max_width = max_width;
        self
    }

    /// What the read-only access segment does ("Explain read-only access"): its tooltip while
    /// it shows its text, and always its accessible description (NOTES M2b surface 6).
    pub fn access_hint(mut self, hint: SharedString) -> Self {
        self.access_hint = Some(hint);
        self
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

// ## m2b-t22-live
/// The read-only access segment (NOTES M2b surface 6): a button and a Tab stop. A click, or
/// Enter or Space while it has the focus (GPUI's keyboard click), reports
/// [`CapsuleSegment::Access`] at its bottom-left. A pointer click focuses it first, so the
/// popover gives the focus back here when it closes. The focus ring is a 1 px `blue` line inset
/// 2 px (mock `.cp-ro:focus-visible`), shown for keyboard focus only. An inert capsule (a modal
/// layer is open) announces it as disabled and takes no focus.
#[allow(
    clippy::too_many_arguments,
    reason = "one private call site in `Capsule::build`"
)]
fn access_segment(
    t: &ThemeTokens,
    window: &mut Window,
    cx: &mut App,
    inert: bool,
    on_segment: Option<SegmentHandler>,
    name: &SharedString,
    content: Vec<AnyElement>,
    tooltip: Option<SharedString>,
    hint: Option<SharedString>,
) -> AnyElement {
    const ID: &str = "capsule-access";
    let slot = Rc::new(Cell::new(Bounds::default()));
    let mut el = cell(t)
        .id(ID)
        .debug_selector(|| ID.to_string())
        .relative()
        .child(bounds_probe(slot.clone(), 1.0, 0.0))
        .children(content);
    if let Some(tip) = tooltip {
        el = el.tooltip(move |_, cx| Tip::view(tip.clone(), cx));
    }
    if inert {
        let mut node = A11yNode::new("capsule-access-node", Role::Button, name.clone(), false, el)
            .disabled(true);
        if let Some(hint) = hint {
            node = node.description(hint);
        }
        return node.into_any_element();
    }
    let focus = window.use_keyed_state(ElementId::from("capsule-access-focus"), cx, |_, cx| {
        cx.focus_handle()
    });
    let handle = focus.read(cx).clone();
    if focus_visible(&handle, window) {
        el = el.child(
            div()
                .debug_selector(|| "capsule-access-ring".to_string())
                .absolute()
                .top(px(2.0))
                .bottom(px(2.0))
                .left(px(2.0))
                .right(px(2.0))
                .border_1()
                .border_color(t.colors.blue.to_hsla())
                .rounded(px(t.radii.xs)),
        );
    }
    el = el.track_focus(&handle.clone().tab_stop(true));
    if let Some(handler) = on_segment {
        // Enter or Space on key-down (never held, never with a modifier), as the dialogs act;
        // the default is prevented, so GPUI's key-up click does not open it a second time.
        let (on_key, key_slot) = (handler.clone(), slot.clone());
        el = el.on_key_down(move |event: &KeyDownEvent, window, cx| {
            let stroke = &event.keystroke;
            if event.is_held
                || stroke.modifiers.modified()
                || !matches!(stroke.key.as_str(), "enter" | "space")
            {
                return;
            }
            window.prevent_default();
            cx.stop_propagation();
            on_key(
                CapsuleSegment::Access,
                key_slot.get().bottom_left(),
                window,
                cx,
            );
        });
        el = el.on_click(move |_, window, cx| {
            cx.stop_propagation();
            window.focus(&handle, cx);
            handler(CapsuleSegment::Access, slot.get().bottom_left(), window, cx);
        });
    }
    let mut decl = A11yDecl::new(Role::Button, name.clone());
    decl.description = hint;
    el.declare_full(decl).into_any_element()
}

// ## m2b-t22-live (fix round 1, ruling I1)

/// The laid-out width of `text` in `style`, as a `Label` shapes it.
fn text_width(window: &Window, text: &SharedString, style: TypeStyle) -> f32 {
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
}

/// The narrowest a name ellipsizes to once every step is spent: a glyph and the ellipsis (fix
/// round 2, R1-I1: round 1 stopped at about six characters and let the capsule push the
/// palette).
const NAME_FLOOR_PX: f32 = 20.0;
/// Room kept in hand when fitting, against rounding.
const FIT_SLACK_PX: f32 = 2.0;

/// What the capsule gives up to fit its room (ruling I1), in this order: the connected
/// capsule's latency, path text, access text and unencrypted text (the ruling's order); then,
/// in any state, the engine (the status bar and the explorer show it) and a reconnect's
/// countdown (pseudo locales); then (fix round 2, R1-I1) the state's own text (connecting,
/// "Reconnecting · attempt n", idle, "Couldn't connect"), which keeps its glyph, and Retry's
/// text, which keeps its icon; last the name. A collapsed segment keeps its whole text as its
/// tooltip and accessible name.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
struct Fit {
    hide_latency: bool,
    path_icon_only: bool,
    access_icon_only: bool,
    unencrypted_icon_only: bool,
    hide_engine: bool,
    hide_next: bool,
    // ## m2b-t22-live (fix round 2, R1-I1)
    state_icon_only: bool,
    retry_icon_only: bool,
    /// How much narrower the name is drawn (it ellipsizes).
    name_cut: f32,
}

/// The steps of [`Fit`], in order.
const STEPS: usize = 8;

impl Fit {
    /// The degradation that brings a capsule `full` px wide (nothing given up) within `max`:
    /// each step of `steps` (in [`Fit`]'s order; 0 when the step does not apply), then the name
    /// down to [`NAME_FLOOR_PX`]. Returns the fit and the width it saves.
    fn plan(full: f32, max: f32, steps: [f32; STEPS], name_w: f32) -> (Fit, f32) {
        let mut fit = Fit::default();
        let mut over = full - max + FIT_SLACK_PX;
        let mut saved = 0.0;
        for (i, step) in steps.into_iter().enumerate() {
            if over <= 0.0 {
                break;
            }
            if step <= 0.0 {
                continue;
            }
            match i {
                0 => fit.hide_latency = true,
                1 => fit.path_icon_only = true,
                2 => fit.access_icon_only = true,
                3 => fit.unencrypted_icon_only = true,
                4 => fit.hide_engine = true,
                5 => fit.hide_next = true,
                6 => fit.state_icon_only = true,
                _ => fit.retry_icon_only = true,
            }
            over -= step;
            saved += step;
        }
        if over > 0.0 {
            let cut = over.min((name_w - NAME_FLOOR_PX).max(0.0));
            fit.name_cut = cut;
            saved += cut;
        }
        (fit, saved)
    }
}

/// The capsule's natural width (nothing given up), as the last frame measured it: the width
/// of its segments' row (which the clamp never squeezes, fix round 2) and the frame's border,
/// plus what that frame gave up. Kept per window.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
struct FitMemo {
    full: Option<f32>,
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

/// The name of a live state (the one part every state may ellipsize).
fn model_name(model: &CapsuleModel) -> Option<&SharedString> {
    match model {
        CapsuleModel::Placeholder { .. } => None,
        CapsuleModel::Connecting { name, .. }
        | CapsuleModel::Reconnecting { name, .. }
        | CapsuleModel::Idle { name, .. }
        | CapsuleModel::Connection { name, .. }
        | CapsuleModel::Failed { name, .. } => Some(name),
    }
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

        // ## m2b-t22-live (fix round 1, ruling I1): what this frame gives up to fit its room.
        let gap = t.spacing.sm;
        let engine_w = |engine: &SharedString| gap + text_width(window, engine, body);
        let steps = match &self.model {
            CapsuleModel::Connection {
                engine,
                access,
                read_only,
                path,
                latency,
                unencrypted,
                ..
            } => [
                latency
                    .as_ref()
                    .map_or(0.0, |l| gap + text_width(window, l, mono)),
                gap + text_width(window, path, body),
                if *read_only {
                    gap + text_width(window, access, body)
                } else {
                    0.0
                },
                unencrypted
                    .as_ref()
                    .map_or(0.0, |u| gap + text_width(window, u, body)),
                engine_w(engine),
                0.0,
                0.0,
                0.0,
            ],
            // The countdown keeps at least its 44 px column.
            CapsuleModel::Reconnecting {
                engine,
                next,
                label,
                ..
            } => [
                0.0,
                0.0,
                0.0,
                0.0,
                engine_w(engine),
                next.as_ref().map_or(0.0, |n| {
                    gap + text_width(window, n, mono).max(RUN_MONO_MIN_PX)
                }),
                gap + text_width(window, label, body),
                0.0,
            ],
            CapsuleModel::Connecting { engine, label, .. }
            | CapsuleModel::Idle { engine, label, .. } => [
                0.0,
                0.0,
                0.0,
                0.0,
                engine_w(engine),
                0.0,
                gap + text_width(window, label, body),
                0.0,
            ],
            // ## m2b-t22-live (fix round 2, R1-I1): "Couldn't connect", then Retry's text.
            CapsuleModel::Failed { label, retry, .. } => [
                0.0,
                0.0,
                0.0,
                0.0,
                0.0,
                0.0,
                gap + text_width(window, label, body),
                gap + text_width(window, retry, body),
            ],
            CapsuleModel::Placeholder { .. } => [0.0; STEPS],
        };
        let name_w = model_name(&self.model).map_or(0.0, |n| text_width(window, n, strong));
        let memo = window.use_keyed_state(ElementId::from("capsule-fit"), cx, |_, _| {
            Rc::new(Cell::new(FitMemo::default()))
        });
        let memo = memo.read(cx).clone();
        let (fit, saved) = match (memo.get().full, self.max_width) {
            (Some(full), Some(max)) => Fit::plan(full, max, steps, name_w),
            _ => (Fit::default(), 0.0),
        };

        let max_width = self.max_width;
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
        // ## m2b-t22-live (fix round 2, R1-I1): the hard clamp. The frame is never laid out
        // wider than its room, so the palette field and the trailing controls never move. The
        // segments sit in a row of their own, which the clamp never squeezes: the row's width
        // is always the capsule's natural width, so one frame's measure plans the next exactly
        // (round 1's convergence). The plan ends with the name ellipsized down to
        // `NAME_FLOOR_PX`; only a frame planned with a stale width (the first frame of a window
        // or of a wider state) is cut at the frame's edge, and the next one fits.
        if let Some(max) = max_width {
            frame = frame.max_w(px(max));
        }
        let mut row = div()
            .relative()
            .flex()
            .flex_row()
            .items_center()
            .h_full()
            .flex_none()
            // ## m2b-t22-live: the natural width for the next frame's fit (the row and the
            // frame's 1 px border on each side, plus what this frame gave up); a change draws
            // again.
            .child(
                canvas(
                    move |bounds, window, _| {
                        let full = f32::from(bounds.size.width) + 2.0 + saved;
                        if memo.get().full.is_none_or(|f| (f - full).abs() > 0.5) {
                            memo.set(FitMemo { full: Some(full) });
                            window.refresh();
                        }
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .top_0()
                .left_0()
                .size_full(),
            );

        // A segment that reports itself (and not `Body`) when clicked; announced as a button,
        // disabled while the capsule is inert.
        let inert = self.inert;
        let on_segment = if inert { None } else { self.on_segment.clone() };
        let clickable = |id: &'static str,
                         segment: CapsuleSegment,
                         name: &SharedString,
                         content: Vec<AnyElement>,
                         tooltip: Option<SharedString>| {
            let slot = Rc::new(Cell::new(Bounds::default()));
            let mut el = cell(t)
                .id(id)
                .debug_selector(move || id.to_string())
                .relative()
                .child(bounds_probe(slot.clone(), 1.0, 0.0))
                .children(content);
            if let Some(tip) = tooltip {
                el = el.tooltip(move |_, cx| Tip::view(tip.clone(), cx));
            }
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

        let head = |row: Div, head: Head<'_>| {
            let mut frame = row;
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
            // ## m2b-t22-live (ruling I1): the name ellipsizes last, its whole text the tooltip.
            let name_label: AnyElement = if fit.name_cut > 0.0 {
                div()
                    .min_w(px(0.0))
                    .max_w(px((name_w - fit.name_cut).max(NAME_FLOOR_PX)))
                    .child(
                        data(head.name, strong, c.text.to_hsla())
                            .truncate(true)
                            .id("capsule-text-name"),
                    )
                    .into_any_element()
            } else {
                data(head.name, strong, c.text.to_hsla())
                    .id("capsule-text-name")
                    .into_any_element()
            };
            let mut name_cell = cell(t)
                .debug_selector(|| "capsule-body".to_string())
                .child(name_label);
            if head.env.is_none() {
                name_cell = name_cell.border_l_0();
            }
            if let Some(engine) = head.engine.filter(|_| !fit.hide_engine) {
                name_cell = name_cell.child(data(engine, body, c.text_2.to_hsla()));
            }
            frame.child(name_cell)
        };

        // ## m2b-t22-live (fix round 2, R1-I1): a state segment whose text collapsed keeps its
        // glyph; the text becomes its tooltip and accessible name.
        let collapsed = |el: Div, id: &'static str, tip: &SharedString| {
            let (tip, name) = (tip.clone(), tip.clone());
            el.id(id)
                .debug_selector(move || id.to_string())
                .tooltip(move |_, cx| Tip::view(tip.clone(), cx))
                .declare(Role::Label, name, None)
                .into_any_element()
        };

        let name_for_label;
        let mut description = None;
        match &self.model {
            CapsuleModel::Placeholder { label } => {
                name_for_label = label.clone();
                row = row.child(div().flex().items_center().px(px(9.0)).child(text(
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
                row = head(
                    row,
                    Head {
                        env: Some((*env, env_label)),
                        name,
                        engine: Some(engine),
                    },
                );
                let mut run =
                    cell(t).child(run_marker(&theme).debug_selector(|| "capsule-run".to_string()));
                if !fit.state_icon_only {
                    run = run.child(text(label, body, c.text.to_hsla()).id("capsule-text-run"));
                }
                let run = run
                    .child(div().min_w(px(RUN_MONO_MIN_PX)).child(text(
                        step,
                        mono,
                        c.text_2.to_hsla(),
                    )))
                    .child(div().min_w(px(RUN_MONO_MIN_PX)).child(text(
                        clock,
                        mono,
                        c.text_2.to_hsla(),
                    )));
                row = row.child(if fit.state_icon_only {
                    collapsed(run, "capsule-state", label)
                } else {
                    run.into_any_element()
                });
            }
            // ## m2b-t22-live: the run marker, "Reconnecting · attempt n" and, between
            // attempts, "in n s" (NOTES M2a §7, HV-CONN-05).
            CapsuleModel::Reconnecting {
                env,
                env_label,
                name,
                engine,
                label,
                next,
            } => {
                name_for_label = name.clone();
                let mut said = vec![env_label.clone(), label.clone()];
                said.extend(next.iter().cloned());
                description = Some(SharedString::from(said.join(", ")));
                row = head(
                    row,
                    Head {
                        env: Some((*env, env_label)),
                        name,
                        engine: Some(engine),
                    },
                );
                let mut run =
                    cell(t).child(run_marker(&theme).debug_selector(|| "capsule-run".to_string()));
                // Collapsed (R1-I1): the run marker, "Reconnecting · attempt n" its tooltip.
                if !fit.state_icon_only {
                    run = run.child(text(label, body, c.text.to_hsla()).id("capsule-text-run"));
                }
                if let Some(next) = next.as_ref().filter(|_| !fit.hide_next) {
                    run = run.child(
                        div()
                            .min_w(px(RUN_MONO_MIN_PX))
                            .child(text(next, mono, c.text_2.to_hsla()).id("capsule-text-next")),
                    );
                }
                row = row.child(if fit.state_icon_only {
                    collapsed(run, "capsule-state", label)
                } else {
                    run.into_any_element()
                });
            }
            CapsuleModel::Idle {
                env,
                env_label,
                name,
                engine,
                label,
            } => {
                name_for_label = name.clone();
                description = Some(SharedString::from(format!("{env_label}, {label}")));
                row = head(
                    row,
                    Head {
                        env: Some((*env, env_label)),
                        name,
                        engine: Some(engine),
                    },
                );
                // NOTES M2b surface 7: the unplug glyph and its label, both text-2. Collapsed
                // (R1-I1): the glyph alone, the label its tooltip.
                let mut idle = cell(t)
                    .debug_selector(|| "capsule-idle".to_string())
                    .child(icon(IconName::Unplug, c.text_2.to_hsla()));
                row = row.child(if fit.state_icon_only {
                    collapsed(idle, "capsule-state", label)
                } else {
                    idle =
                        idle.child(text(label, body, c.text_2.to_hsla()).id("capsule-text-idle"));
                    idle.into_any_element()
                });
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
                row = head(
                    row,
                    Head {
                        env: Some((*env, env_label)),
                        name,
                        engine: Some(engine),
                    },
                );
                let production = *env == EnvKind::Production;
                // Hue Is Meaning: the lock is red only on production.
                // ## m2b-t22-live: a read-only access segment is a button and a Tab stop that
                // opens the explanation (NOTES M2b surface 6).
                let access_cell = if *read_only {
                    let lock = if production { c.red } else { c.text_2 };
                    // Collapsed (ruling I1): the lock alone, the label its tooltip and name.
                    let mut content = vec![icon(IconName::Lock, lock.to_hsla()).into_any_element()];
                    let tooltip = if fit.access_icon_only {
                        Some(access.clone())
                    } else {
                        content.push(
                            text(access, body, c.text.to_hsla())
                                .id("capsule-text-access")
                                .into_any_element(),
                        );
                        self.access_hint.clone()
                    };
                    access_segment(
                        t,
                        window,
                        cx,
                        inert,
                        on_segment.clone(),
                        access,
                        content,
                        tooltip,
                        self.access_hint.clone(),
                    )
                } else {
                    cell(t)
                        .child(text(access, body, c.text_2.to_hsla()).id("capsule-text-access"))
                        .into_any_element()
                };
                let (path_icon, path_ink) = if *direct {
                    (IconName::ArrowLeftRight, c.green)
                } else {
                    (IconName::Route, c.text_2)
                };
                let mut path_cell = cell(t)
                    .id("capsule-path")
                    .debug_selector(|| "capsule-path".to_string())
                    .child(icon(path_icon, path_ink.to_hsla()));
                if !fit.path_icon_only {
                    path_cell =
                        path_cell.child(text(path, body, c.text.to_hsla()).id("capsule-text-path"));
                }
                if let Some(latency) = latency.as_ref().filter(|_| !fit.hide_latency) {
                    path_cell = path_cell
                        .child(text(latency, mono, c.text_2.to_hsla()).id("capsule-text-latency"));
                }
                if fit.path_icon_only {
                    let tip = path.clone();
                    path_cell = path_cell
                        .tooltip(move |_, cx| Tip::view(tip.clone(), cx))
                        .declare(Role::Label, path.clone(), None);
                }
                row = row.child(access_cell).child(path_cell);
                if let Some(label) = unencrypted {
                    let mut content =
                        vec![icon(IconName::LockOpen, c.text_2.to_hsla()).into_any_element()];
                    if !fit.unencrypted_icon_only {
                        content.push(text(label, body, c.text.to_hsla()).into_any_element());
                    }
                    row = row.child(clickable(
                        "capsule-unencrypted",
                        CapsuleSegment::Unencrypted,
                        label,
                        content,
                        fit.unencrypted_icon_only.then(|| label.clone()),
                    ));
                }
                if let Some(label) = unsupported {
                    // A permanent, hue-free mark (PO Q3).
                    row = row.child(
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
                row = head(
                    row,
                    Head {
                        env: Some((*env, env_label)),
                        name,
                        engine: None,
                    },
                );
                // No new hue for errors: the alert glyph is ink.
                let retry_ink = if hovered { c.text } else { c.text_2 };
                // Collapsed (R1-I1): the alert glyph alone, then Retry's icon alone; each text
                // its tooltip and accessible name.
                let alert = cell(t).child(icon(IconName::TriangleAlert, c.text.to_hsla()));
                let alert = if fit.state_icon_only {
                    collapsed(alert, "capsule-state", label)
                } else {
                    alert
                        .child(text(label, body, c.text.to_hsla()).id("capsule-text-failed"))
                        .into_any_element()
                };
                let mut retry_content =
                    vec![icon(IconName::RotateCw, retry_ink.to_hsla()).into_any_element()];
                if !fit.retry_icon_only {
                    retry_content.push(
                        text(retry, body, retry_ink.to_hsla())
                            .id("capsule-text-retry")
                            .into_any_element(),
                    );
                }
                row = row.child(alert).child(clickable(
                    "capsule-retry",
                    CapsuleSegment::Retry,
                    retry,
                    retry_content,
                    fit.retry_icon_only.then(|| retry.clone()),
                ));
            }
        }
        row = row.child(
            cell(t).px(px(6.0)).child(
                Icon::new(IconName::ChevronsUpDown)
                    .size(IconSize::Chrome)
                    .color(c.text_3.to_hsla()),
            ),
        );
        frame = frame.child(row);
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

// ## m2b-t22-live (fix round 2, R1-I1)
#[cfg(test)]
mod fit_tests {
    use super::*;

    /// A failed capsule: "Couldn't connect" (60 px) goes first, then Retry's text (40 px).
    const FAILED: [f32; STEPS] = [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 60.0, 40.0];

    #[test]
    fn a_failed_capsule_gives_up_its_text_then_retry_then_the_name() {
        let (fit, saved) = Fit::plan(500.0, 470.0, FAILED, 100.0);
        assert!(fit.state_icon_only && !fit.retry_icon_only);
        assert_eq!((fit.name_cut, saved), (0.0, 60.0));
        let (fit, saved) = Fit::plan(500.0, 420.0, FAILED, 100.0);
        assert!(fit.state_icon_only && fit.retry_icon_only);
        assert_eq!((fit.name_cut, saved), (0.0, 100.0));
        // Every step spent: the name ellipsizes, down to its floor.
        let (fit, saved) = Fit::plan(500.0, 300.0, FAILED, 100.0);
        assert_eq!(fit.name_cut, 100.0 - NAME_FLOOR_PX);
        assert_eq!(saved, 100.0 + 100.0 - NAME_FLOOR_PX);
    }

    #[test]
    fn a_reconnect_gives_up_the_engine_the_countdown_then_its_label() {
        let steps = [0.0, 0.0, 0.0, 0.0, 30.0, 50.0, 120.0, 0.0];
        let (fit, _) = Fit::plan(600.0, 560.0, steps, 90.0);
        assert!(fit.hide_engine && fit.hide_next && !fit.state_icon_only);
        let (fit, _) = Fit::plan(600.0, 500.0, steps, 90.0);
        assert!(fit.state_icon_only);
        assert_eq!(fit.name_cut, 0.0);
    }
}
