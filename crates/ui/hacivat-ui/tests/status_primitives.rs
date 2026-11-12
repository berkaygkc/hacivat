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
    App, Bounds, Font, FontId, FontMetrics, FontRun, GlyphId, Hsla, InteractiveElement,
    IntoElement, LineLayout, Modifiers, NoopTextSystem, ParentElement, Pixels, PlatformTextSystem,
    Point, RenderGlyphParams, Role, SharedString, Size, Styled, TestAppContext, TestDispatcher,
    TextRenderingMode, VisualTestContext, div, px, size,
};
use hacivat_ui::button::Button;
use hacivat_ui::capsule::{Capsule, CapsuleModel, CapsuleSegment, EnvKind, env_swatch};
use hacivat_ui::explain::ExplainPopover;
use hacivat_ui::icon::{DrawnIcon, IconName, icon_record, icon_take};
use hacivat_ui::popover::{A11yDecl, a11y_record, a11y_take};
use hacivat_ui::statusbar::{StatusBar, env_segment};
use hacivat_ui::step_trail::{StepTrail, TrailItem, TrailState};
use hacivat_ui::text::{TextFn, TextRegistry};
use hacivat_ui::titlebar::TitleBar;
use hacivat_ui::toast::{
    Toast, ToastAction, ToastKind, ToastQueue, ToastStack, sticky_demo_fixture,
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
    CapsuleModel::Failed {
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

#[gpui::test]
fn capsule_segments_are_announced(cx: &mut TestAppContext) {
    a11y_record();
    let (_vcx, _) = mount_capsule(cx, failed_capsule("orders-prod"));
    let decls = a11y_take();
    assert_eq!(
        declared(&decls, Role::Button, "orders-prod")
            .description
            .as_deref(),
        Some("Couldn't connect")
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

fn mount_titlebar(
    cx: &mut TestAppContext,
    inert: bool,
) -> (&mut VisualTestContext, Rc<Cell<usize>>) {
    let opened = Rc::new(Cell::new(0));
    let count = opened.clone();
    let vcx = host(cx, move |_, _| {
        let (a, b, c) = (count.clone(), count.clone(), count.clone());
        div()
            .w_full()
            .child(
                TitleBar::new(ChromeKind::MacTrafficLights)
                    .inert(inert)
                    .brand("HACIVAT".into())
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
