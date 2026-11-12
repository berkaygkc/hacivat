//! The M2a status primitives (NOTES.md "NEW primitives", §7 "Live capsule states"): the step
//! trail, the dev and test marks, the live capsule segments, the status-bar environment segment,
//! sticky toasts with actions, the explanation popover and the inert title bar.

mod support;

use std::borrow::Cow;
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use gpui::{
    App, AppContext as _, Bounds, Focusable as _, Font, FontId, FontMetrics, FontRun, GlyphId,
    Hsla, InteractiveElement, IntoElement, LineLayout, Modifiers, NoopTextSystem, ParentElement,
    Pixels, PlatformTextSystem, Point, RenderGlyphParams, Role, SharedString, Size, Styled,
    TestAppContext, TestDispatcher, TextRenderingMode, VisualTestContext, div, px, size,
};
use hacivat_ui::button::Button;
use hacivat_ui::capsule::{Capsule, CapsuleModel, CapsuleSegment, EnvKind, env_swatch};
use hacivat_ui::explain::{ExplainEvent, ExplainPopover};
use hacivat_ui::icon::{DrawnIcon, IconName, icon_record, icon_take};
use hacivat_ui::input::TextInput;
use hacivat_ui::popover::{A11yDecl, a11y_record, a11y_take};
use hacivat_ui::statusbar::{StatusBar, env_segment};
use hacivat_ui::step_trail::{StepTrail, TrailA11y, TrailItem, TrailState};
use hacivat_ui::text::{TextFn, TextRegistry};
use hacivat_ui::titlebar::TitleBar;
use hacivat_ui::toast::{
    Toast, ToastAction, ToastKind, ToastQueue, ToastStack, focus_newest_sticky, sticky_demo_fixture,
};
use hacivat_ui_core::fonts::font_for;
use hacivat_ui_core::theme::{DARK, Theme, ThemeMode};
use hacivat_ui_core::window::ChromeKind;
use support::host;

// ---- shared helpers ---------------------------------------------------------------------------

fn text(s: &'static str) -> TextFn {
    Rc::new(move |_: &App| s.into())
}

fn selector(name: impl Into<String>) -> &'static str {
    Box::leak(name.into().into_boxed_str())
}

fn bounds_of(vcx: &mut VisualTestContext, name: impl Into<String>) -> Bounds<Pixels> {
    let name = name.into();
    vcx.debug_bounds(selector(name.clone()))
        .unwrap_or_else(|| panic!("{name} was not drawn"))
}

fn has(vcx: &mut VisualTestContext, name: impl Into<String>) -> bool {
    vcx.debug_bounds(selector(name)).is_some()
}

fn click(vcx: &mut VisualTestContext, name: impl Into<String>) {
    let at = bounds_of(vcx, name).center();
    vcx.simulate_click(at, Modifiers::none());
    vcx.run_until_parked();
}

fn enable_registry(vcx: &mut VisualTestContext) {
    vcx.update(|window, cx| {
        cx.set_global(TextRegistry {
            enabled: true,
            frame: Vec::new(),
        });
        window.refresh();
    });
    vcx.run_until_parked();
}

/// Every string the last frame drew.
fn texts(vcx: &mut VisualTestContext) -> Vec<SharedString> {
    vcx.read(|cx| {
        cx.global::<TextRegistry>()
            .frame
            .iter()
            .map(|r| r.text.clone())
            .collect()
    })
}

/// The icons one fresh frame draws, in drawing order.
fn frame_icons(vcx: &mut VisualTestContext) -> Vec<DrawnIcon> {
    icon_record();
    vcx.update(|window, _| window.refresh());
    vcx.run_until_parked();
    icon_take()
}

fn names(icons: &[DrawnIcon]) -> Vec<IconName> {
    icons.iter().map(|i| i.name).collect()
}

fn color_of(icons: &[DrawnIcon], name: IconName) -> Hsla {
    icons
        .iter()
        .find(|i| i.name == name)
        .unwrap_or_else(|| panic!("no {name:?} in {icons:?}"))
        .color
}

fn red() -> Hsla {
    DARK.colors.red.to_hsla()
}

fn declared(decls: &[A11yDecl], role: Role, name: &str) -> A11yDecl {
    decls
        .iter()
        .find(|d| d.role == Some(role) && d.name.as_ref() == name)
        .unwrap_or_else(|| panic!("no {role:?} named {name:?} in {decls:?}"))
        .clone()
}

// ---- a text system that records the font of every shaped line ---------------------------------

/// GPUI's test platform shapes every font alike, so the family and weight are read from the
/// shaping request.
#[derive(Default)]
struct FontRecorder {
    fonts: Mutex<Vec<Font>>,
    lines: Mutex<Vec<(String, Font)>>,
}

struct RecordingTextSystem {
    inner: NoopTextSystem,
    rec: Arc<FontRecorder>,
}

impl PlatformTextSystem for RecordingTextSystem {
    fn add_fonts(&self, fonts: Vec<Cow<'static, [u8]>>) -> gpui::Result<()> {
        self.inner.add_fonts(fonts)
    }
    fn all_font_names(&self) -> Vec<String> {
        self.inner.all_font_names()
    }
    fn font_id(&self, descriptor: &Font) -> gpui::Result<FontId> {
        let mut fonts = self.rec.fonts.lock().unwrap();
        let at = match fonts.iter().position(|f| f == descriptor) {
            Some(at) => at,
            None => {
                fonts.push(descriptor.clone());
                fonts.len() - 1
            }
        };
        Ok(FontId(at + 1))
    }
    fn font_metrics(&self, font_id: FontId) -> FontMetrics {
        self.inner.font_metrics(font_id)
    }
    fn typographic_bounds(&self, font_id: FontId, glyph_id: GlyphId) -> gpui::Result<Bounds<f32>> {
        self.inner.typographic_bounds(font_id, glyph_id)
    }
    fn advance(&self, font_id: FontId, glyph_id: GlyphId) -> gpui::Result<Size<f32>> {
        self.inner.advance(font_id, glyph_id)
    }
    fn glyph_for_char(&self, font_id: FontId, ch: char) -> Option<GlyphId> {
        self.inner.glyph_for_char(font_id, ch)
    }
    fn glyph_raster_bounds(
        &self,
        params: &RenderGlyphParams,
    ) -> gpui::Result<Bounds<gpui::DevicePixels>> {
        self.inner.glyph_raster_bounds(params)
    }
    fn rasterize_glyph(
        &self,
        params: &RenderGlyphParams,
        raster_bounds: Bounds<gpui::DevicePixels>,
    ) -> gpui::Result<(Size<gpui::DevicePixels>, Vec<u8>)> {
        self.inner.rasterize_glyph(params, raster_bounds)
    }
    fn layout_line(&self, text: &str, font_size: Pixels, runs: &[FontRun]) -> LineLayout {
        if let Some(run) = runs.first() {
            let fonts = self.rec.fonts.lock().unwrap();
            if let Some(font) = fonts.get(run.font_id.0 - 1) {
                self.rec
                    .lines
                    .lock()
                    .unwrap()
                    .push((text.to_string(), font.clone()));
            }
        }
        self.inner.layout_line(text, font_size, runs)
    }
    fn recommended_rendering_mode(&self, font_id: FontId, font_size: Pixels) -> TextRenderingMode {
        self.inner.recommended_rendering_mode(font_id, font_size)
    }
}

fn mono_family() -> String {
    font_for(&DARK.type_ramp.mono_micro).family.to_string()
}

// ---- step trail -------------------------------------------------------------------------------

fn done(label: &'static str, duration: &'static str) -> TrailItem {
    TrailItem {
        label: label.into(),
        state: TrailState::Done {
            duration: Some(duration.into()),
        },
    }
}

fn running(label: &'static str) -> TrailItem {
    TrailItem {
        label: label.into(),
        state: TrailState::Running,
    }
}

fn pending(label: &'static str) -> TrailItem {
    TrailItem {
        label: label.into(),
        state: TrailState::Pending,
    }
}

fn failed(label: &'static str) -> TrailItem {
    TrailItem {
        label: label.into(),
        state: TrailState::Failed,
    }
}

/// A trail drawn in its own app, with the fonts of its shaped lines and its icons recorded.
struct TrailProbe {
    _cx: TestAppContext,
    rec: Arc<FontRecorder>,
    icons: Vec<DrawnIcon>,
    run_markers: usize,
    pending_marks: usize,
    item_heights: Vec<Pixels>,
}

fn mount_trail(items: Vec<TrailItem>) -> TrailProbe {
    let rec = Arc::new(FontRecorder::default());
    let mut cx = TestAppContext::build_with_text_system(
        TestDispatcher::new(0),
        None,
        Arc::new(RecordingTextSystem {
            inner: NoopTextSystem,
            rec: rec.clone(),
        }),
    );
    let n = items.len();
    let vcx = host(&mut cx, move |_, _| {
        div()
            .w(px(600.0))
            .child(StepTrail::new(items.clone()))
            .into_any_element()
    });
    vcx.run_until_parked();
    let icons = frame_icons(vcx);
    let seven = size(px(7.0), px(7.0));
    let count = |vcx: &mut VisualTestContext, kind: &str| {
        (0..n)
            .filter(|i| {
                vcx.debug_bounds(selector(format!("trail-{kind}-{i}")))
                    .is_some_and(|b| b.size == seven)
            })
            .count()
    };
    let run_markers = count(vcx, "run");
    let pending_marks = count(vcx, "pending");
    let item_heights = (0..n)
        .map(|i| bounds_of(vcx, format!("trail-{i}")).size.height)
        .collect();
    TrailProbe {
        _cx: cx,
        rec,
        icons,
        run_markers,
        pending_marks,
        item_heights,
    }
}

impl TrailProbe {
    fn font_of(&self, text: &str) -> Font {
        let lines = self.rec.lines.lock().unwrap();
        lines
            .iter()
            .find(|(t, _)| t == text)
            .unwrap_or_else(|| panic!("{text} was not shaped: {lines:?}"))
            .1
            .clone()
    }

    fn weight_of(&self, text: &str) -> u32 {
        self.font_of(text).weight.0 as u32
    }

    /// The shaped lines set in the mono family, in layout order.
    fn mono_texts(&self) -> Vec<String> {
        let mono = mono_family();
        let mut out: Vec<String> = Vec::new();
        for (text, font) in self.rec.lines.lock().unwrap().iter() {
            if font.family.as_ref() == mono && !out.contains(text) {
                out.push(text.clone());
            }
        }
        out
    }
}

#[gpui::test]
fn trail_states(_: &mut TestAppContext) {
    let probe = mount_trail(vec![
        done("Resolve", "3 ms"),
        done("Connect", "12 ms"),
        running("TLS"),
        pending("Sign in"),
    ]);
    assert_eq!(names(&probe.icons), [IconName::Check, IconName::Check]);
    assert_eq!(
        color_of(&probe.icons, IconName::Check),
        DARK.colors.green.to_hsla()
    );
    assert!(probe.icons.iter().all(|i| i.size_px == 12.0));
    assert_eq!(probe.run_markers, 1);
    assert_eq!(probe.pending_marks, 1);
    assert_eq!(probe.mono_texts(), ["3 ms", "12 ms"]);
    assert_eq!(probe.weight_of("Resolve"), 400);
    assert!(probe.item_heights.iter().all(|h| *h == px(20.0)));

    let probe = mount_trail(vec![done("Resolve", "3 ms"), failed("Sign in")]);
    assert_eq!(probe.weight_of("Sign in"), 600);
    assert_eq!(names(&probe.icons), [IconName::Check, IconName::X]);
    // No new hue for errors: the ✕ is ink.
    assert_eq!(
        color_of(&probe.icons, IconName::X),
        DARK.colors.text.to_hsla()
    );
    assert_eq!(probe.run_markers, 0);
}

// ---- environment marks ------------------------------------------------------------------------

fn swatch_size(cx: &mut TestAppContext, kind: EnvKind) -> Size<Pixels> {
    let vcx = host(cx, move |_, cx| {
        let theme = *Theme::get(cx);
        div()
            .p(px(10.0))
            .child(env_swatch(kind, &theme).debug_selector(|| "swatch".to_string()))
            .into_any_element()
    });
    vcx.run_until_parked();
    bounds_of(vcx, "swatch").size
}

fn swatch_is_round(kind: EnvKind) -> bool {
    let theme = Theme::new(ThemeMode::Dark);
    let mut swatch = env_swatch(kind, &theme);
    swatch.style().corner_radii.top_left.is_some()
}

#[gpui::test]
fn env_marks_are_7px_shapes(cx: &mut TestAppContext) {
    for k in [
        EnvKind::Dev,
        EnvKind::Test,
        EnvKind::Local,
        EnvKind::Staging,
        EnvKind::Production,
    ] {
        assert_eq!(swatch_size(cx, k), size(px(7.0), px(7.0)), "{k:?}");
    }
    // Circles for personal and pre-release environments, squares for shared servers.
    assert!(swatch_is_round(EnvKind::Dev) && swatch_is_round(EnvKind::Test));
    assert!(swatch_is_round(EnvKind::Local));
    assert!(!swatch_is_round(EnvKind::Staging) && !swatch_is_round(EnvKind::Production));
}

#[gpui::test]
fn the_dev_mark_has_a_3px_centre_dot(cx: &mut TestAppContext) {
    let vcx = host(cx, |_, cx| {
        let theme = *Theme::get(cx);
        div()
            .p(px(10.0))
            .child(env_swatch(EnvKind::Dev, &theme).debug_selector(|| "swatch".to_string()))
            .into_any_element()
    });
    vcx.run_until_parked();
    let mark = bounds_of(vcx, "swatch");
    let dot = bounds_of(vcx, "env-dev-dot");
    assert_eq!(dot.size, size(px(3.0), px(3.0)));
    assert_eq!(dot.center(), mark.center());
}

// ---- capsule ----------------------------------------------------------------------------------

type Hits = Rc<RefCell<Vec<(CapsuleSegment, Point<Pixels>)>>>;

fn mount_capsule(cx: &mut TestAppContext, model: CapsuleModel) -> (&mut VisualTestContext, Hits) {
    let hits: Hits = Rc::default();
    let sink = hits.clone();
    let vcx = host(cx, move |_, _| {
        let sink = sink.clone();
        div()
            .p(px(20.0))
            .flex()
            .child(
                Capsule::new(model.clone()).on_segment(move |segment, anchor, _, _| {
                    sink.borrow_mut().push((segment, anchor))
                }),
            )
            .into_any_element()
    });
    vcx.simulate_resize(gpui::size(px(1200.0), px(200.0)));
    vcx.run_until_parked();
    enable_registry(vcx);
    (vcx, hits)
}

fn connecting(name: &'static str, step: &'static str, clock: &'static str) -> CapsuleModel {
    CapsuleModel::Connecting {
        env: EnvKind::Staging,
        env_label: "Staging".into(),
        name: name.into(),
        engine: "PostgreSQL 16".into(),
        label: "Connecting".into(),
        step: step.into(),
        clock: clock.into(),
    }
}

fn connected(env: EnvKind, read_only: bool) -> CapsuleModel {
    CapsuleModel::Connection {
        env,
        env_label: "PRODUCTION".into(),
        name: "orders-prod".into(),
        engine: "PostgreSQL 16".into(),
        access: "read-only".into(),
        read_only,
        path: "Direct".into(),
        direct: true,
        latency: Some("4 ms".into()),
        unencrypted: Some("unencrypted".into()),
        unsupported: Some("unsupported".into()),
    }
}

fn connected_prod_ro_unencrypted() -> CapsuleModel {
    connected(EnvKind::Production, true)
}

fn failed_capsule(name: &'static str) -> CapsuleModel {
    failed_in(EnvKind::Production, name)
}

fn failed_in(env: EnvKind, name: &'static str) -> CapsuleModel {
    CapsuleModel::Failed {
        env,
        env_label: if env == EnvKind::Production {
            "PRODUCTION".into()
        } else {
            "Staging".into()
        },
        label: "Couldn't connect".into(),
        name: name.into(),
        retry: "Retry".into(),
    }
}

/// The One Filled Badge rule: the production block is the capsule's first segment and, with
/// `scene-check`, the only `cp_fill` quad painted.
fn assert_one_filled_badge(vcx: &mut VisualTestContext, production: bool) {
    assert_eq!(has(vcx, "cp-fill"), production);
    if production {
        let capsule = bounds_of(vcx, "capsule");
        let block = bounds_of(vcx, "cp-fill");
        assert_eq!(
            block.left(),
            capsule.left() + px(1.0),
            "the block comes first"
        );
    }
    #[cfg(feature = "scene-check")]
    vcx.update(|w, _| {
        hacivat_ui_core::named_rules::assert_one_filled_badge(
            w,
            DARK.colors.cp_fill.to_hsla(),
            production,
        )
    });
}

#[gpui::test]
fn capsule_states(cx: &mut TestAppContext) {
    let (vcx, _) = mount_capsule(cx, connecting("orders-staging", "TLS", "0.42 s"));
    assert_eq!(bounds_of(vcx, "capsule-run").size, size(px(7.0), px(7.0)));
    let drawn = texts(vcx);
    for t in ["orders-staging", "Connecting", "TLS", "0.42 s"] {
        assert!(drawn.contains(&t.into()), "{t} in {drawn:?}");
    }
    assert_one_filled_badge(vcx, false);

    let (vcx, hits) = mount_capsule(cx, connected_prod_ro_unencrypted());
    assert_one_filled_badge(vcx, true);
    let drawn = texts(vcx);
    assert!(drawn.contains(&"unencrypted".into()));
    assert!(drawn.contains(&"unsupported".into()));
    let icons = frame_icons(vcx);
    assert_eq!(color_of(&icons, IconName::Lock), red());
    assert_eq!(
        color_of(&icons, IconName::LockOpen),
        DARK.colors.text_2.to_hsla()
    );
    click(vcx, "capsule-unencrypted");
    assert_eq!(
        *hits.borrow(),
        [(
            CapsuleSegment::Unencrypted,
            bounds_of(vcx, "capsule-unencrypted").bottom_left()
        )]
    );

    // Hue Is Meaning: the read-only lock is red only on production.
    let (vcx, _) = mount_capsule(cx, connected(EnvKind::Staging, true));
    assert_one_filled_badge(vcx, false);
    assert_eq!(
        color_of(&frame_icons(vcx), IconName::Lock),
        DARK.colors.text_2.to_hsla()
    );
    // Read-write shows no lock at all.
    let (vcx, _) = mount_capsule(cx, connected(EnvKind::Production, false));
    assert!(!names(&frame_icons(vcx)).contains(&IconName::Lock));

    let (vcx, hits) = mount_capsule(cx, failed_capsule("orders-prod"));
    let drawn = texts(vcx);
    assert!(drawn.contains(&"Couldn't connect".into()) && drawn.contains(&"Retry".into()));
    let icons = frame_icons(vcx);
    assert_eq!(
        color_of(&icons, IconName::TriangleAlert),
        DARK.colors.text.to_hsla()
    );
    assert!(names(&icons).contains(&IconName::RotateCw));
    click(vcx, "capsule-retry");
    click(vcx, "capsule-body");
    assert_eq!(
        hits.borrow().iter().map(|(s, _)| *s).collect::<Vec<_>>(),
        [CapsuleSegment::Retry, CapsuleSegment::Body]
    );
    assert_eq!(
        hits.borrow()[0].1,
        bounds_of(vcx, "capsule-retry").bottom_left()
    );
    assert_eq!(hits.borrow()[1].1, bounds_of(vcx, "capsule").bottom_left());
}

/// Ruling A: a failed production connection keeps the PRODUCTION block first (NOTES §7).
#[gpui::test]
fn a_failed_capsule_keeps_its_environment_first(cx: &mut TestAppContext) {
    let (vcx, _) = mount_capsule(cx, failed_capsule("orders-prod"));
    assert_one_filled_badge(vcx, true);
    assert!(bounds_of(vcx, "cp-fill").right() <= bounds_of(vcx, "capsule-body").left());
    assert!(texts(vcx).contains(&"PRODUCTION".into()));

    let (vcx, _) = mount_capsule(cx, failed_in(EnvKind::Staging, "orders-staging"));
    assert_one_filled_badge(vcx, false);
    assert!(texts(vcx).contains(&"Staging".into()));
}

/// Exactly one handler runs per click: the segment handler wins over `on_click`.
#[gpui::test]
fn capsule_click_handlers_are_exclusive(cx: &mut TestAppContext) {
    let clicks = Rc::new(Cell::new(0));
    let hits: Hits = Rc::default();
    let (c, h) = (clicks.clone(), hits.clone());
    let vcx = host(cx, move |_, _| {
        let (c, h) = (c.clone(), h.clone());
        div()
            .p(px(20.0))
            .flex()
            .child(
                Capsule::new(failed_capsule("orders-prod"))
                    .on_click(move |_, _, _| c.set(c.get() + 1))
                    .on_segment(move |s, anchor, _, _| h.borrow_mut().push((s, anchor))),
            )
            .into_any_element()
    });
    vcx.simulate_resize(gpui::size(px(1200.0), px(200.0)));
    vcx.run_until_parked();
    click(vcx, "capsule-body");
    click(vcx, "capsule-retry");
    assert_eq!(clicks.get(), 0);
    assert_eq!(
        hits.borrow().iter().map(|(s, _)| *s).collect::<Vec<_>>(),
        [CapsuleSegment::Body, CapsuleSegment::Retry]
    );

    // Without a segment handler, `on_click` takes the body click (M1).
    let clicks = Rc::new(Cell::new(0));
    let c = clicks.clone();
    let vcx = host(cx, move |_, _| {
        let c = c.clone();
        div()
            .p(px(20.0))
            .flex()
            .child(
                Capsule::new(failed_capsule("orders-prod"))
                    .on_click(move |_, _, _| c.set(c.get() + 1)),
            )
            .into_any_element()
    });
    vcx.simulate_resize(gpui::size(px(1200.0), px(200.0)));
    vcx.run_until_parked();
    click(vcx, "capsule-body");
    assert_eq!(clicks.get(), 1);
}

#[gpui::test]
fn capsule_segments_are_announced(cx: &mut TestAppContext) {
    a11y_record();
    let (_vcx, _) = mount_capsule(cx, failed_capsule("orders-prod"));
    let decls = a11y_take();
    assert_eq!(
        declared(&decls, Role::Button, "orders-prod")
            .description
            .as_deref(),
        Some("PRODUCTION, Couldn't connect")
    );
    declared(&decls, Role::Button, "Retry");

    a11y_record();
    let (_vcx, _) = mount_capsule(cx, connected_prod_ro_unencrypted());
    declared(&a11y_take(), Role::Button, "unencrypted");
}

// ---- status bar -------------------------------------------------------------------------------

fn mount_status(cx: &mut TestAppContext, kind: EnvKind, read_only: bool) -> &mut VisualTestContext {
    let vcx = host(cx, move |_, cx| {
        let theme = *Theme::get(cx);
        div()
            .w(px(800.0))
            .child(
                StatusBar::new()
                    .left(env_segment(
                        &theme,
                        kind,
                        "Production · read-only".into(),
                        read_only,
                    ))
                    .left(div().child("next")),
            )
            .into_any_element()
    });
    vcx.run_until_parked();
    vcx
}

#[gpui::test]
fn the_production_status_segment_is_tinted_edge_to_edge(cx: &mut TestAppContext) {
    let vcx = mount_status(cx, EnvKind::Production, true);
    let bar = bounds_of(vcx, "status-bar");
    let seg = bounds_of(vcx, "status-env");
    // The tint fills the whole cell: from the bar's left edge, the full height under the seam.
    assert_eq!(seg.left(), bar.left());
    assert_eq!(seg.size.height, bar.size.height - px(1.0));
    let icons = frame_icons(vcx);
    assert_eq!(color_of(&icons, IconName::Lock), red());

    let vcx = mount_status(cx, EnvKind::Production, false);
    assert!(!names(&frame_icons(vcx)).contains(&IconName::Lock));
    let vcx = mount_status(cx, EnvKind::Staging, true);
    assert!(!names(&frame_icons(vcx)).contains(&IconName::Lock));
    assert!(bounds_of(vcx, "status-env").left() > bounds_of(vcx, "status-bar").left());
}

// ---- toasts -----------------------------------------------------------------------------------

fn visible(vcx: &mut VisualTestContext) -> Vec<u64> {
    vcx.read(ToastQueue::visible_ids)
}

fn push_sticky(cx: &mut TestAppContext) -> (&mut VisualTestContext, Rc<Cell<bool>>) {
    let ran = Rc::new(Cell::new(false));
    let flag = ran.clone();
    cx.update(|cx| {
        let mut toast = Toast::new(ToastKind::Info, text("Open transactions were rolled back."));
        toast.sticky = true;
        toast.wide = true;
        toast.title = Some(text("Reconnected to orders-prod"));
        toast.bullets = vec![text("Your open transaction"), text("Temporary tables")];
        toast.actions = vec![ToastAction {
            label: text("Show details"),
            run: Rc::new(move |_, _| flag.set(true)),
        }];
        ToastQueue::push(cx, toast);
    });
    let vcx = host(cx, |_, _| {
        div()
            .size_full()
            .child(ToastStack::new())
            .into_any_element()
    });
    vcx.simulate_resize(gpui::size(px(1000.0), px(600.0)));
    vcx.run_until_parked();
    (vcx, ran)
}

#[gpui::test]
fn sticky_toast_stays_and_actions_dismiss(cx: &mut TestAppContext) {
    a11y_record();
    let (vcx, ran) = push_sticky(cx);
    let decls = a11y_take();
    let node = declared(&decls, Role::Alert, "Reconnected to orders-prod");
    assert_eq!(node.live, Some(gpui::accesskit::Live::Assertive));

    vcx.executor().advance_clock(Duration::from_secs(60));
    vcx.run_until_parked();
    let ids = visible(vcx);
    assert_eq!(ids.len(), 1);
    let id = ids[0];
    assert_eq!(bounds_of(vcx, format!("toast-{id}")).size.width, px(380.0));
    enable_registry(vcx);
    let drawn = texts(vcx);
    for t in [
        "Reconnected to orders-prod",
        "Open transactions were rolled back.",
        "Your open transaction",
        "Temporary tables",
        "Show details",
    ] {
        assert!(drawn.contains(&t.into()), "{t} in {drawn:?}");
    }
    assert_eq!(
        bounds_of(vcx, format!("toast-bullet-{id}-1")).size,
        size(px(3.0), px(3.0))
    );

    click(vcx, format!("button-toast-action-{id}-0"));
    assert!(ran.get());
    assert!(visible(vcx).is_empty());
}

#[gpui::test]
fn plain_toasts_keep_their_ttl_and_width(cx: &mut TestAppContext) {
    cx.update(|cx| ToastQueue::push(cx, Toast::new(ToastKind::Info, text("Saved"))));
    let vcx = host(cx, |_, _| {
        div()
            .size_full()
            .child(ToastStack::new())
            .into_any_element()
    });
    vcx.simulate_resize(gpui::size(px(1000.0), px(600.0)));
    vcx.run_until_parked();
    let id = visible(vcx)[0];
    assert_eq!(bounds_of(vcx, format!("toast-{id}")).size.width, px(320.0));
    vcx.executor().advance_clock(Duration::from_secs(7));
    vcx.run_until_parked();
    assert!(visible(vcx).is_empty());
}

#[gpui::test]
fn the_sticky_demo_fixture_pushes_one_sticky_toast(cx: &mut TestAppContext) {
    let vcx = host(cx, |_, _| {
        div()
            .size_full()
            .child(ToastStack::new())
            .into_any_element()
    });
    vcx.simulate_resize(gpui::size(px(1000.0), px(600.0)));
    vcx.update(|window, cx| {
        sticky_demo_fixture(
            text("Reconnected to orders-prod"),
            vec![text("Rolled back"), text("Gone")],
            text("Show details"),
            window,
            cx,
        )
    });
    vcx.run_until_parked();
    enable_registry(vcx);
    vcx.executor().advance_clock(Duration::from_secs(60));
    vcx.run_until_parked();
    let ids = visible(vcx);
    assert_eq!(ids.len(), 1);
    let drawn = texts(vcx);
    for t in [
        "Reconnected to orders-prod",
        "Rolled back",
        "Gone",
        "Show details",
    ] {
        assert!(drawn.contains(&t.into()), "{t} in {drawn:?}");
    }
    click(vcx, format!("button-toast-action-{}-0", ids[0]));
    assert!(visible(vcx).is_empty());
}

// ---- explanation popover ----------------------------------------------------------------------

fn explain_fixture(cx: &mut TestAppContext) -> &mut VisualTestContext {
    let view = cx.update(|cx| {
        ExplainPopover::view(
            "Unencrypted connection".into(),
            IconName::LockOpen,
            vec![
                "ci-mariadb sends queries and results in plain text.".into(),
                "To encrypt, set TLS to Require in Edit connection.".into(),
            ],
            cx,
        )
    });
    let vcx = host(cx, move |_, _| {
        div().p(px(20.0)).child(view.clone()).into_any_element()
    });
    vcx.simulate_resize(gpui::size(px(800.0), px(400.0)));
    vcx.run_until_parked();
    vcx
}

#[gpui::test]
fn explanation_popover_is_300_wide(cx: &mut TestAppContext) {
    let vcx = explain_fixture(cx);
    let card = bounds_of(vcx, "explain-popover");
    assert_eq!(card.size.width, px(300.0));
    // Padding 10/12 inside the 1 px border.
    let title = bounds_of(vcx, "explain-title");
    assert_eq!(title.left(), card.left() + px(1.0 + 12.0));
    assert_eq!(title.top(), card.top() + px(1.0 + 10.0));
    let icons = frame_icons(vcx);
    assert_eq!(names(&icons), [IconName::LockOpen]);
    assert_eq!(icons[0].size_px, 12.0);
    enable_registry(vcx);
    let drawn = texts(vcx);
    assert!(drawn.contains(&"Unencrypted connection".into()));
    assert!(drawn.contains(&"ci-mariadb sends queries and results in plain text.".into()));
}

// ---- title bar --------------------------------------------------------------------------------

fn mount_titlebar_in(
    chrome: ChromeKind,
    cx: &mut TestAppContext,
    inert: bool,
) -> (&mut VisualTestContext, Rc<Cell<usize>>) {
    let opened = Rc::new(Cell::new(0));
    let count = opened.clone();
    let vcx = host(cx, move |_, _| {
        let (a, b, c, d) = (count.clone(), count.clone(), count.clone(), count.clone());
        div()
            .w_full()
            .child(
                TitleBar::new(chrome)
                    .inert(inert)
                    .brand("HACIVAT".into())
                    .app_menu(move |_, _, _| d.set(d.get() + 1))
                    .capsule(
                        Capsule::new(failed_capsule("orders-prod"))
                            .on_segment(move |_, _, _, _| a.set(a.get() + 1)),
                    )
                    .palette_field("Search".into(), "⌘K".into(), move |_, _, _| {
                        b.set(b.get() + 1)
                    })
                    .right(vec![
                        Button::new("theme", "Dark".into())
                            .on_click(move |_, _, _| c.set(c.get() + 1))
                            .into_any_element(),
                    ]),
            )
            .into_any_element()
    });
    vcx.simulate_resize(gpui::size(px(1400.0), px(300.0)));
    vcx.run_until_parked();
    (vcx, opened)
}

#[gpui::test]
fn title_bar_controls_are_inert_while_modal(cx: &mut TestAppContext) {
    let (vcx, opened) = mount_titlebar(cx, true);
    click(vcx, "palette-field");
    click(vcx, "capsule-body");
    click(vcx, "capsule-retry");
    click(vcx, "button-theme");
    assert_eq!(opened.get(), 0);

    // The same bar without a modal layer: every control answers.
    let (vcx, opened) = mount_titlebar(cx, false);
    click(vcx, "palette-field");
    click(vcx, "capsule-body");
    click(vcx, "capsule-retry");
    click(vcx, "button-theme");
    assert_eq!(opened.get(), 4);
}

fn mount_titlebar(
    cx: &mut TestAppContext,
    inert: bool,
) -> (&mut VisualTestContext, Rc<Cell<usize>>) {
    mount_titlebar_in(ChromeKind::MacTrafficLights, cx, inert)
}

#[gpui::test]
fn the_inert_title_bar_marks_its_controls_disabled(cx: &mut TestAppContext) {
    a11y_record();
    let (_vcx, _) = mount_titlebar_in(ChromeKind::LinuxClient, cx, true);
    let decls = a11y_take();
    for name in ["Search", "orders-prod", "Retry"] {
        assert!(declared(&decls, Role::Button, name).disabled, "{name}");
    }
    // The app-menu button (its Fluent name) is announced disabled too.
    assert!(
        decls
            .iter()
            .filter(|d| d.role == Some(Role::Button) && d.disabled)
            .count()
            >= 4,
        "{decls:?}"
    );

    // Inert controls are wrapped in a node that can say "disabled".
    for name in ["Search", "orders-prod", "Retry"] {
        assert!(declared(&decls, Role::Button, name).wrapper, "{name}");
    }

    // Enabled, each control declares its role on the clickable element itself, as `Button`
    // does, so an assistive-technology Click reaches its `on_click`. (GPUI's test platform
    // never attaches assistive technology and its action routing is crate-private, so the
    // Click itself cannot be dispatched here; this is the structural check.)
    a11y_record();
    let (_vcx, _) = mount_titlebar_in(ChromeKind::LinuxClient, cx, false);
    let decls = a11y_take();
    for name in ["Search", "orders-prod", "Retry"] {
        let decl = declared(&decls, Role::Button, name);
        assert!(!decl.disabled && !decl.wrapper, "{name}: {decl:?}");
    }
    a11y_record();
    let (_vcx, _) = mount_capsule(cx, connected_prod_ro_unencrypted());
    let decl = declared(&a11y_take(), Role::Button, "unencrypted");
    assert!(!decl.disabled && !decl.wrapper, "{decl:?}");
}

/// Fix 9: while inert, the drag regions and the window buttons stay live.
#[gpui::test]
fn the_inert_title_bar_keeps_dragging_and_window_buttons_live(cx: &mut TestAppContext) {
    let (vcx, opened) = mount_titlebar_in(ChromeKind::LinuxClient, cx, true);
    let close = bounds_of(vcx, "window-close");
    let bar = bounds_of(vcx, "title-bar");
    for what in ["capsule", "right", "app-menu"] {
        let cover = bounds_of(vcx, format!("titlebar-inert-{what}"));
        assert!(!cover.intersects(&close), "{what} covers the close button");
    }
    // The brand's drag region (left of the capsule) is under no cover.
    let drag_at = gpui::point(bar.left() + px(30.0), bar.center().y);
    let capsule_cover = bounds_of(vcx, "titlebar-inert-capsule");
    assert!(drag_at.x < capsule_cover.left());
    click(vcx, "button-titlebar-app-menu");
    assert_eq!(opened.get(), 0, "the app menu is inert");
    // The Linux close button still closes the window.
    let before = vcx.update(|_, cx| cx.windows().len());
    vcx.simulate_click(close.center(), Modifiers::none());
    vcx.run_until_parked();
    assert_eq!(cx.update(|cx| cx.windows().len()), before - 1);

    // A press on the drag region still starts a window move. GPUI's test platform (at the
    // workspace pin) implements `TestWindow::start_window_move` as a bare `unimplemented!()`, so
    // reaching it panics with "not implemented"; the message does not name the method, so the
    // panic's backtrace (captured by a hook scoped to this thread) must. If GPUI's test platform
    // ever implements `start_window_move`, this check has to observe the move another way.
    let (vcx, _) = mount_titlebar(cx, true);
    let bar = bounds_of(vcx, "title-bar");
    let drag_at = gpui::point(bar.left() + px(20.0), bar.center().y);
    let (pressed, trace) = with_panic_trace(|| {
        vcx.simulate_mouse_down(drag_at, gpui::MouseButton::Left, Modifiers::none());
    });
    let payload = pressed.expect_err("the drag region did not see the press");
    let message = payload
        .downcast_ref::<String>()
        .map(String::as_str)
        .or_else(|| payload.downcast_ref::<&str>().copied())
        .unwrap_or_default();
    assert_eq!(message, "not implemented", "an unrelated panic");
    assert!(
        trace.contains("start_window_move"),
        "the panic did not come from start_window_move:\n{trace}"
    );
}

/// Runs `f`, catching a panic, and returns the panic's message and backtrace as the panic hook
/// saw them on this thread (other threads keep the previous hook).
fn with_panic_trace(f: impl FnOnce()) -> (std::thread::Result<()>, String) {
    type Hook = Box<dyn Fn(&std::panic::PanicHookInfo<'_>) + Sync + Send + 'static>;
    let this_thread = std::thread::current().id();
    let seen = Arc::new(Mutex::new(String::new()));
    let previous: Arc<Hook> = Arc::new(std::panic::take_hook());
    let (sink, fallback) = (seen.clone(), previous.clone());
    std::panic::set_hook(Box::new(move |info| {
        if std::thread::current().id() == this_thread {
            let trace = std::backtrace::Backtrace::force_capture();
            *sink.lock().unwrap() = format!("{info}\n{trace}");
        } else {
            fallback(info);
        }
    }));
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f));
    drop(std::panic::take_hook());
    std::panic::set_hook(Box::new(move |info| previous(info)));
    let trace = seen.lock().unwrap().clone();
    (result, trace)
}

// ---- step trail accessibility (ruling B) ------------------------------------------------------

#[gpui::test]
fn the_trail_announces_each_state_and_where_it_failed(cx: &mut TestAppContext) {
    a11y_record();
    let vcx = host(cx, |_, _| {
        div()
            .w(px(600.0))
            .child(
                StepTrail::new(vec![
                    done("Resolve", "3 ms"),
                    TrailItem {
                        label: "SSH".into(),
                        state: TrailState::Skipped,
                    },
                    failed("Sign in"),
                    pending("Check"),
                ])
                .a11y(TrailA11y {
                    label: "Connection steps".into(),
                    pending: "pending".into(),
                    running: "in progress".into(),
                    done: "done".into(),
                    failed: "failed".into(),
                    skipped: "skipped".into(),
                }),
            )
            .into_any_element()
    });
    vcx.run_until_parked();
    let decls = a11y_take();
    declared(&decls, Role::List, "Connection steps");
    let item = |name: &str| declared(&decls, Role::ListItem, name);
    let resolve = item("Resolve");
    assert_eq!(
        (
            resolve.value.as_deref(),
            resolve.description.as_deref(),
            resolve.position
        ),
        (Some("done"), Some("3 ms"), Some((1, 4)))
    );
    assert_eq!(item("SSH").value.as_deref(), Some("skipped"));
    let failed = item("Sign in");
    assert_eq!(
        (failed.value.as_deref(), failed.position),
        (Some("failed"), Some((3, 4)))
    );
    assert_eq!(item("Check").value.as_deref(), Some("pending"));
    assert!(names(&frame_icons(vcx)).contains(&IconName::Minus));
}

// ---- toasts: keyboard, announcement, slots, modal --------------------------------------------

struct InputHost {
    input: gpui::Entity<TextInput>,
    stack: Rc<dyn Fn() -> ToastStack>,
}

impl gpui::Render for InputHost {
    fn render(&mut self, _: &mut gpui::Window, _: &mut gpui::Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .child(div().w(px(300.0)).child(self.input.clone()))
            .child((self.stack)())
    }
}

/// A window with a focused text input and a toast stack.
fn mount_input_and_toasts(
    cx: &mut TestAppContext,
    stack: impl Fn() -> ToastStack + 'static,
) -> (&mut VisualTestContext, gpui::Entity<TextInput>) {
    cx.update(|cx| Theme::install(cx, ThemeMode::Dark));
    let stack: Rc<dyn Fn() -> ToastStack> = Rc::new(stack);
    let (view, vcx) = cx.add_window_view(move |_, cx| InputHost {
        input: cx.new(|cx| TextInput::new(cx, text("Filter"))),
        stack: stack.clone(),
    });
    vcx.simulate_resize(gpui::size(px(1000.0), px(600.0)));
    let input = view.read_with(vcx, |h, _| h.input.clone());
    let focus = input.clone();
    vcx.update(|window, cx| {
        window.activate_window();
        focus.focus_handle(cx).focus(window, cx);
    });
    vcx.run_until_parked();
    (vcx, input)
}

fn sticky(title: &'static str, ran: Rc<Cell<usize>>) -> Toast {
    let mut toast = Toast::new(ToastKind::Info, text("Open transactions were rolled back."));
    toast.sticky = true;
    toast.title = Some(text(title));
    toast.bullets = vec![text("Your open transaction"), text("Temporary tables")];
    toast.actions = vec![ToastAction {
        label: text("Show details"),
        run: Rc::new(move |_, _| ran.set(ran.get() + 1)),
    }];
    toast
}

fn focused(vcx: &mut VisualTestContext, input: &gpui::Entity<TextInput>) -> bool {
    let input = input.clone();
    vcx.update(|window, cx| input.focus_handle(cx).is_focused(window))
}

#[gpui::test]
fn sticky_toasts_are_reachable_by_keyboard(cx: &mut TestAppContext) {
    let ran = Rc::new(Cell::new(0));
    let r = ran.clone();
    cx.update(|cx| ToastQueue::push(cx, sticky("Reconnected to orders-prod", r)));
    let (vcx, input) = mount_input_and_toasts(cx, ToastStack::new);
    let id = visible(vcx)[0];
    assert!(has(vcx, format!("button-toast-action-{id}-0")));
    assert!(has(vcx, format!("button-toast-dismiss-{id}")));

    // The route the host binds a key to: focus reaches the first action; Enter runs it.
    assert!(vcx.update(focus_newest_sticky));
    vcx.run_until_parked();
    assert!(!focused(vcx, &input));
    // Enter down and up, as a keyboard sends it: GPUI clicks the focused button on key-up.
    vcx.simulate_keystrokes("enter");
    vcx.simulate_event(gpui::KeyUpEvent {
        keystroke: gpui::Keystroke::parse("enter").unwrap(),
    });
    vcx.run_until_parked();
    assert_eq!(ran.get(), 1);
    assert!(visible(vcx).is_empty());
    assert!(focused(vcx, &input), "dismissing gives the focus back");

    // Escape dismisses the focused toast without running its action.
    let r = ran.clone();
    vcx.update(|_, cx| ToastQueue::push(cx, sticky("Reconnected again", r)));
    vcx.run_until_parked();
    assert!(vcx.update(focus_newest_sticky));
    vcx.run_until_parked();
    vcx.simulate_keystrokes("escape");
    vcx.run_until_parked();
    assert_eq!(ran.get(), 1);
    assert!(visible(vcx).is_empty());
    assert!(focused(vcx, &input));

    // Nothing sticky: the route reports it and leaves the focus alone.
    assert!(!vcx.update(focus_newest_sticky));
    assert!(focused(vcx, &input));
}

#[gpui::test]
fn the_toast_announcement_includes_its_bullets(cx: &mut TestAppContext) {
    let ran = Rc::new(Cell::new(0));
    cx.update(|cx| ToastQueue::push(cx, sticky("Reconnected to orders-prod", ran)));
    a11y_record();
    let (_vcx, _) = mount_input_and_toasts(cx, ToastStack::new);
    let node = declared(&a11y_take(), Role::Alert, "Reconnected to orders-prod");
    let value = node.value.unwrap_or_default();
    for line in [
        "Reconnected to orders-prod",
        "Open transactions were rolled back.",
        "Your open transaction",
        "Temporary tables",
    ] {
        assert!(value.contains(line), "{line} in {value:?}");
    }
}

#[gpui::test]
fn sticky_toasts_never_take_a_transient_slot(cx: &mut TestAppContext) {
    let ran = Rc::new(Cell::new(0));
    cx.update(|cx| {
        for title in ["one", "two", "three"] {
            ToastQueue::push(cx, sticky(title, ran.clone()));
        }
        for t in ["a", "b", "c", "d"] {
            ToastQueue::push(cx, Toast::new(ToastKind::Info, text(t)));
        }
        // Two sticky and three transient show; the third sticky and `d` wait.
        assert_eq!(ToastQueue::visible_ids(cx).len(), 5);
        assert_eq!(ToastQueue::waiting(cx), 2);
        // Dismissing a sticky toast promotes the waiting sticky one, not `d`.
        let first = ToastQueue::visible_ids(cx)[0];
        ToastQueue::dismiss(cx, first);
        assert_eq!(ToastQueue::visible_ids(cx).len(), 5);
        assert_eq!(ToastQueue::waiting(cx), 1);
    });
    // The transient ones expire and `d` takes a slot; the sticky ones stay.
    cx.executor().advance_clock(Duration::from_secs(7));
    cx.run_until_parked();
    cx.update(|cx| {
        assert_eq!(ToastQueue::visible_ids(cx).len(), 3);
        assert_eq!(ToastQueue::waiting(cx), 0);
    });
}

#[gpui::test]
fn toast_actions_are_inert_while_a_modal_is_open(cx: &mut TestAppContext) {
    let ran = Rc::new(Cell::new(0));
    let r = ran.clone();
    cx.update(|cx| ToastQueue::push(cx, sticky("Reconnected to orders-prod", r)));
    a11y_record();
    let (vcx, _) = mount_input_and_toasts(cx, || ToastStack::new().inert(true));
    let decls = a11y_take();
    assert!(declared(&decls, Role::Button, "Show details").disabled);
    let id = visible(vcx)[0];
    click(vcx, format!("button-toast-action-{id}-0"));
    click(vcx, format!("button-toast-dismiss-{id}"));
    assert_eq!(ran.get(), 0);
    assert_eq!(visible(vcx), [id]);
}

// ---- explanation popover focus ----------------------------------------------------------------

struct PopoverHost {
    input: gpui::Entity<TextInput>,
    popover: Option<gpui::Entity<ExplainPopover>>,
}

impl gpui::Render for PopoverHost {
    fn render(&mut self, _: &mut gpui::Window, _: &mut gpui::Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .child(div().w(px(300.0)).child(self.input.clone()))
            .children(self.popover.clone())
    }
}

#[gpui::test]
fn the_explanation_popover_takes_and_returns_the_focus(cx: &mut TestAppContext) {
    cx.update(|cx| Theme::install(cx, ThemeMode::Dark));
    let (view, vcx) = cx.add_window_view(|_, cx| PopoverHost {
        input: cx.new(|cx| TextInput::new(cx, text("Filter"))),
        popover: None,
    });
    let input = view.read_with(vcx, |h, _| h.input.clone());
    let focus = input.clone();
    vcx.update(|window, cx| {
        window.activate_window();
        focus.focus_handle(cx).focus(window, cx);
    });
    vcx.run_until_parked();
    assert!(focused(vcx, &input));

    // Open it: it takes the focus.
    let events = Rc::new(RefCell::new(Vec::new()));
    let sink = events.clone();
    let popover = vcx.update(|_, cx| {
        let popover = ExplainPopover::new(
            "Unencrypted connection".into(),
            IconName::LockOpen,
            vec!["ci-mariadb sends queries in plain text.".into()],
            cx,
        );
        cx.subscribe(&popover, move |_, e: &ExplainEvent, _| {
            sink.borrow_mut().push(*e)
        })
        .detach();
        popover
    });
    let p = popover.clone();
    view.update_in(vcx, |h, _, cx| {
        h.popover = Some(p);
        cx.notify();
    });
    vcx.run_until_parked();
    let p = popover.clone();
    assert!(vcx.update(|window, cx| p.focus_handle(cx).is_focused(window)));
    assert!(!focused(vcx, &input));

    // Escape: the focus goes back to the input, and the host hears it.
    vcx.simulate_keystrokes("escape");
    vcx.run_until_parked();
    assert!(focused(vcx, &input));
    assert_eq!(*events.borrow(), [ExplainEvent::Dismissed]);
}

// ## m2a-t21-shots: popover paragraphs face the frame checks

#[gpui::test]
fn explanation_paragraphs_are_recorded_after_the_title_and_unclipped(cx: &mut TestAppContext) {
    let vcx = explain_fixture(cx);
    enable_registry(vcx);
    let frame = vcx.read(|cx| cx.global::<TextRegistry>().frame.clone());
    let drawn: Vec<&str> = frame.iter().map(|r| r.text.as_ref()).collect();
    assert_eq!(
        drawn,
        [
            "Unencrypted connection",
            "ci-mariadb sends queries and results in plain text.",
            "To encrypt, set TLS to Require in Edit connection.",
        ]
    );
    for r in &frame[1..] {
        assert!(r.path.contains("hacivat-ui/src/explain.rs:"), "{}", r.path);
        assert_eq!(r.size_px, 12.5);
        assert!(!r.clipped, "{r:?}");
    }
}

#[gpui::test]
fn a_paragraph_in_a_box_too_short_for_it_is_clipped(cx: &mut TestAppContext) {
    let vcx = host(cx, |_, cx| {
        let t = Theme::get(cx).tokens;
        div()
            .w(px(120.0))
            .h(px(6.0))
            .overflow_hidden()
            .child(hacivat_ui::text::paragraph(
                "A sentence long enough to wrap onto a second line here.",
                hacivat_ui::text::TextSource::Fluent,
                t.type_ramp.body_sm,
                t.colors.text_2.to_hsla(),
            ))
            .into_any_element()
    });
    vcx.simulate_resize(gpui::size(px(400.0), px(200.0)));
    enable_registry(vcx);
    let frame = vcx.read(|cx| cx.global::<TextRegistry>().frame.clone());
    assert_eq!(frame.len(), 1);
    assert!(frame[0].clipped, "{:?}", frame[0]);
}
