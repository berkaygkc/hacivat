//! The dialog's test strip: the step trail, the clock and the result (NOTES surface 1 test,
//! HV-CONN-07, Task 20).
//!
//! - **Running:** the amber run marker, "Testing", a clock in seconds with two decimals, and the
//!   trail as the pipeline publishes it. The clock reads the GPUI executor clock captured when
//!   [`TestStrip::run`] starts, never `Progress.started` (P22), and ticks while the test runs
//!   (a progress clock, the only timed thing besides the capsule's, NOTES cross-cutting #7).
//! - **Done, ok:** "Connected in 54 ms" with a green check, the line "Nothing was saved…", the
//!   trail with a duration per step, and the diagnostics (Server, Latency, Signed in as,
//!   Warnings, Encryption). "Copy report" (right) copies an English report (R26).
//! - **Done, failed:** the error block, inline ([`ErrorView::inline`]); its fixes are re-emitted
//!   as [`TestStripEvent::Fix`] for the dialog (P40).
//!
//! [`TestStrip::stop`] cancels the running `TestHandle`. Nothing a test does is saved (R8).

use std::sync::Arc;
use std::time::{Duration, Instant};

use hacivat_l10n::{NoArgs, ids};
use hacivat_session::pipeline::Progress;
use hacivat_session::profile::ProfileDraft;
use hacivat_session::{TestHandle, TestReport};
use hacivat_ui::button::{Button, ButtonKind};
use hacivat_ui::icon::{Icon, IconName, IconSize};
use hacivat_ui::popover::A11yNode;
use hacivat_ui::step_trail::{StepTrail, run_marker};
use hacivat_ui::text::{Label, TextSource};
use hacivat_ui_core::gpui_surface::{
    AnyElement, AppContext as _, ClipboardItem, Context, Div, Entity, EventEmitter, FocusHandle,
    InteractiveElement, IntoElement, ParentElement, Render, Role, SharedString, Styled,
    Subscription, Task, Window, div, px,
};
use hacivat_ui_core::theme::{Theme, ThemeMode};
use tokio::sync::oneshot;

use crate::connections::error_view::{ErrorView, ErrorViewEvent, paragraph};
use crate::connections::fixes::FixAction;
use crate::connections::model::ConnectionsModel;
use crate::connections::report::{
    Fmt, diag_rows, error_title, seconds, test_report_text, trail_a11y, trail_items, whole_ms,
};
use crate::l10n_global::tr;

/// How often the running clock repaints: its resolution (hundredths of a second). A repaint
/// only marks the strip dirty; the window still draws at most once per display frame.
const CLOCK_TICK: Duration = Duration::from_millis(10);
/// The diagnostics' label column (the mock's `auto` column, held so the rows align).
const DIAG_LABEL_PX: f32 = 84.0;
/// The line under the heading starts under its text, past the 14 px check and the gap.
const SUB_INDENT_PX: f32 = 22.0;

/// Where the strip is (NOTES surface 1).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum TestStripState {
    Idle,
    Running(Progress),
    Done(Arc<TestReport>),
}

/// What the strip emits (NOTES surface 1).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum TestStripEvent {
    /// A quick fix was chosen.
    Fix(FixAction),
    Finished,
}

/// The test strip view (NOTES surface 1).
pub struct TestStrip {
    state: TestStripState,
    handle: Option<TestHandle>,
    /// The executor clock when the running test started (P22).
    started: Option<Instant>,
    /// The inline error block of a failed test.
    error: Option<Entity<ErrorView>>,
    error_sub: Option<Subscription>,
    tasks: Vec<Task<()>>,
    /// Bumped by every run, so a stale task never touches a newer run.
    run_id: u64,
    copy_focus: FocusHandle,
}

impl EventEmitter<TestStripEvent> for TestStrip {}

impl TestStrip {
    pub fn new(cx: &mut Context<Self>) -> Self {
        TestStrip {
            state: TestStripState::Idle,
            handle: None,
            started: None,
            error: None,
            error_sub: None,
            tasks: Vec::new(),
            run_id: 0,
            copy_focus: cx.focus_handle().tab_stop(true),
        }
    }

    /// Tests `draft` (HV-CONN-07): a running test is stopped first. The test runs on the core
    /// runtime; its progress and result come back over `tokio::sync` channels. Needs the
    /// connections model (does nothing without it).
    pub fn run(&mut self, draft: ProfileDraft, cx: &mut Context<Self>) {
        self.run_continuing(draft, 0, cx);
    }

    /// [`Self::run`] continuing the dialog's count of server rejections (P8): the pipeline's
    /// budget of 3 includes `prior_rejections`.
    pub fn run_continuing(
        &mut self,
        draft: ProfileDraft,
        prior_rejections: u8,
        cx: &mut Context<Self>,
    ) {
        self.stop(cx);
        if cx.try_global::<ConnectionsModel>().is_none() {
            return;
        }
        let mut handle = ConnectionsModel::test_continuing(cx, draft, prior_rejections);
        let mut progress = handle.progress.clone();
        // The handle stays whole (its guard cancels the test when it drops); the result
        // receiver is taken out of it.
        let result = std::mem::replace(&mut handle.result, oneshot::channel().1);
        self.run_id += 1;
        let run = self.run_id;
        self.started = Some(cx.background_executor().now());
        self.state = TestStripState::Running(progress.borrow().clone());
        self.handle = Some(handle);
        self.clear_error();

        self.tasks.push(cx.spawn(async move |this, cx| {
            while progress.changed().await.is_ok() {
                let p = progress.borrow_and_update().clone();
                let live = this.update(cx, |this, cx| {
                    if this.run_id == run && this.is_running() {
                        this.state = TestStripState::Running(p);
                        cx.notify();
                    }
                });
                if live.is_err() {
                    break;
                }
            }
        }));
        self.tasks.push(cx.spawn(async move |this, cx| {
            if let Ok(report) = result.await {
                let _ = this.update(cx, |this, cx| {
                    if this.run_id == run {
                        this.finish(report, cx);
                    }
                });
            }
        }));
        self.tasks.push(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(CLOCK_TICK).await;
                let running = this.update(cx, |this, cx| {
                    let running = this.run_id == run && this.is_running();
                    if running {
                        cx.notify();
                    }
                    running
                });
                if !matches!(running, Ok(true)) {
                    break;
                }
            }
        }));
        cx.notify();
    }

    /// Stops a running test; `true` if one was running. The strip goes back to idle.
    pub fn stop(&mut self, cx: &mut Context<Self>) -> bool {
        let was_running = self.is_running();
        if let Some(handle) = self.handle.take() {
            handle.cancel();
        }
        self.tasks.clear();
        if was_running {
            self.state = TestStripState::Idle;
            cx.notify();
        }
        was_running
    }

    pub fn state(&self) -> &TestStripState {
        &self.state
    }

    /// Whether a test is running.
    pub fn is_running(&self) -> bool {
        matches!(self.state, TestStripState::Running(_))
    }

    /// Shows `state` without running a test (the shots fixtures and tests): a running state
    /// starts its clock now; a finished report draws as a test's would.
    pub fn show(&mut self, state: TestStripState, cx: &mut Context<Self>) {
        self.stop(cx);
        self.run_id += 1;
        self.clear_error();
        match state {
            TestStripState::Done(report) => self.show_report(report, cx),
            other => {
                self.started = Some(cx.background_executor().now());
                self.state = other;
            }
        }
        cx.notify();
    }

    /// The inline error block of a failed test.
    pub fn error_view(&self) -> Option<&Entity<ErrorView>> {
        self.error.as_ref()
    }

    fn clear_error(&mut self) {
        self.error = None;
        self.error_sub = None;
    }

    fn finish(&mut self, report: TestReport, cx: &mut Context<Self>) {
        self.handle = None;
        self.tasks.clear();
        self.show_report(Arc::new(report), cx);
        cx.emit(TestStripEvent::Finished);
        cx.notify();
    }

    fn show_report(&mut self, report: Arc<TestReport>, cx: &mut Context<Self>) {
        self.clear_error();
        if let Some(error) = report.error.as_ref().filter(|_| !report.ok) {
            let view = cx.new(|cx| ErrorView::inline(Arc::new(error.clone()), cx));
            self.error_sub = Some(cx.subscribe(
                &view,
                |_, _, event: &ErrorViewEvent, cx| match event {
                    ErrorViewEvent::Fix(action) => cx.emit(TestStripEvent::Fix(action.clone())),
                },
            ));
            self.error = Some(view);
        }
        self.state = TestStripState::Done(report);
    }

    fn copy_report(&mut self, cx: &mut Context<Self>) {
        if let TestStripState::Done(report) = &self.state {
            let report = report.clone();
            let text = test_report_text(cx, &report);
            cx.write_to_clipboard(ClipboardItem::new_string(text));
        }
    }

    // ---- drawing -----------------------------------------------------------------------------

    fn running(&self, progress: &Progress, cx: &mut Context<Self>) -> Div {
        let theme = *Theme::get(cx);
        let t = theme.tokens;
        let c = &t.colors;
        let f = Fmt::ui(cx);
        let now = cx.background_executor().now();
        let elapsed = self
            .started
            .map_or(Duration::ZERO, |s| now.saturating_duration_since(s));
        let clock = tr(
            cx,
            ids::hv::CONN_TEST_CLOCK,
            ids::hv::HvConnTestClock {
                seconds: seconds(elapsed, 2),
            },
        );
        div()
            .flex()
            .flex_col()
            .gap(px(t.spacing.md))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(t.spacing.md))
                    .child(run_marker(&theme))
                    .child(
                        Label::new(
                            f.s(ids::hv::CONN_TEST_RUNNING, NoArgs),
                            TextSource::Fluent,
                            t.type_ramp.body_strong,
                            c.text.to_hsla(),
                        )
                        .id("test-title"),
                    )
                    .child(
                        Label::new(
                            clock,
                            TextSource::Fluent,
                            t.type_ramp.mono_data,
                            c.text_2.to_hsla(),
                        )
                        .id("test-clock"),
                    ),
            )
            .child(StepTrail::new(trail_items(&f, &progress.trail)).a11y(trail_a11y(&f)))
    }

    fn ok(&self, report: &TestReport, cx: &mut Context<Self>) -> Div {
        let theme = *Theme::get(cx);
        let t = theme.tokens;
        let c = &t.colors;
        let f = Fmt::ui(cx);
        let title = f.s(
            ids::hv::CONN_TEST_OK,
            ids::hv::HvConnTestOk {
                ms: whole_ms(report.total),
            },
        );
        let copy = Button::new("copy-report", f.s(ids::hv::CONN_TEST_COPY, NoArgs))
            .kind(ButtonKind::Ghost)
            .icon(IconName::Copy)
            .focus_handle(self.copy_focus.clone())
            .on_click(cx.listener(|this, _, _, cx| this.copy_report(cx)));
        let header = div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(t.spacing.md))
            .child(
                Icon::new(IconName::CircleCheck)
                    .size(IconSize::Default)
                    .color(c.green.to_hsla()),
            )
            .child(
                Label::new(
                    title,
                    TextSource::Fluent,
                    t.type_ramp.body_strong,
                    c.text.to_hsla(),
                )
                .id("test-title"),
            )
            .child(div().flex_1())
            .child(copy);
        let sub = div().pl(px(SUB_INDENT_PX)).child(paragraph(
            f.s(ids::hv::CONN_TEST_OK_SUB, NoArgs),
            TextSource::Fluent,
            t.type_ramp.body_sm,
            c.text_2.to_hsla(),
            Some("test-sub"),
            cx,
        ));

        // The diagnostics: two label/value pairs per line, Encryption on its own line.
        let rows = diag_rows(&f, report);
        let pair = |row: &crate::connections::report::DiagRow| -> AnyElement {
            let key = row.key;
            let value_style = match key {
                "server" | "auth" => t.type_ramp.mono_data,
                _ => t.type_ramp.body_sm,
            };
            let value_color = if key == "warn" && report.warnings.is_empty() {
                c.text_2.to_hsla()
            } else {
                c.text.to_hsla()
            };
            let mut values = div().flex().flex_col().min_w_0();
            for (i, v) in row.values.iter().enumerate() {
                let id = if i == 0 {
                    format!("diag-{key}-value")
                } else {
                    format!("diag-{key}-value-{i}")
                };
                // ## m2a-t21-shots: a long value ellipsizes (its tooltip shows it whole).
                let line = div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .min_w_0()
                    .gap(px(t.spacing.sm));
                let line = if key == "tls" && report.tls.as_ref().is_some_and(|t| t.encrypted) {
                    line.child(
                        Icon::new(IconName::Lock)
                            .size(IconSize::Chrome)
                            .color(c.green.to_hsla()),
                    )
                } else {
                    line
                };
                let line = line
                    .child(
                        Label::new(v.clone(), TextSource::Fluent, value_style, value_color)
                            .truncate(true)
                            .id(SharedString::from(id)),
                    )
                    .children(row.extra.clone().filter(|_| i == 0).map(|cipher| {
                        Label::new(
                            cipher,
                            TextSource::Data,
                            t.type_ramp.mono_data,
                            c.text_2.to_hsla(),
                        )
                        .id("diag-tls-cipher")
                    }));
                values = values.child(line);
            }
            div()
                .flex()
                .flex_row()
                .items_start()
                .gap(px(t.spacing.xl))
                .flex_1()
                .min_w_0()
                .child(
                    div().min_w(px(DIAG_LABEL_PX)).flex_none().child(
                        Label::new(
                            row.label.clone(),
                            TextSource::Fluent,
                            t.type_ramp.body_sm,
                            c.text_2.to_hsla(),
                        )
                        .id(SharedString::from(format!("diag-{key}-label"))),
                    ),
                )
                .child(values)
                .into_any_element()
        };
        let mut grid = div()
            .debug_selector(|| "test-diag".to_string())
            .flex()
            .flex_col()
            .gap(px(t.spacing.s_2xs));
        for line in rows.chunks(2) {
            grid = grid.child(
                div()
                    .flex()
                    .flex_row()
                    .gap(px(t.spacing.xl))
                    .children(line.iter().map(&pair)),
            );
        }

        div()
            .flex()
            .flex_col()
            .gap(px(t.spacing.md))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(t.spacing.s_2xs))
                    .child(header)
                    .child(sub),
            )
            .child(StepTrail::new(trail_items(&f, &report.trail)).a11y(trail_a11y(&f)))
            .child(grid)
    }
}

impl Render for TestStrip {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let body = match self.state.clone() {
            TestStripState::Idle => return div().into_any_element(),
            TestStripState::Running(progress) => self.running(&progress, cx),
            TestStripState::Done(report) => match &self.error {
                Some(error) => div().child(error.clone()),
                None => self.ok(&report, cx),
            },
        };
        let theme = *Theme::get(cx);
        let t = theme.tokens;
        let c = &t.colors;
        // The strip sits on the field-well fill (panel in dark, ground in light), above the
        // footer, so it shows at any scroll position.
        let well = match theme.mode {
            ThemeMode::Dark => c.panel,
            ThemeMode::Light => c.ground,
        };
        // The status region is named by what it says, so assistive technology announces the
        // result: "Testing", "Connected in 54 ms", or the error's heading.
        let name: SharedString = match &self.state {
            TestStripState::Running(_) => tr(cx, ids::hv::CONN_TEST_RUNNING, NoArgs),
            TestStripState::Done(report) => match report.error.as_ref().filter(|_| !report.ok) {
                Some(error) => error_title(&Fmt::ui(cx), error).into(),
                None => tr(
                    cx,
                    ids::hv::CONN_TEST_OK,
                    ids::hv::HvConnTestOk {
                        ms: whole_ms(report.total),
                    },
                ),
            },
            TestStripState::Idle => SharedString::default(),
        };
        let strip = div()
            .debug_selector(|| "test-strip".to_string())
            .flex()
            .flex_col()
            .flex_none()
            .border_t_1()
            .border_color(c.seam.to_hsla())
            .bg(well.to_hsla())
            .pt(px(t.spacing.lg))
            .pb(px(t.spacing.xl))
            .px(px(t.spacing.s_3xl))
            .child(body);
        A11yNode::new("test-strip-node", Role::Status, name, false, strip).into_any_element()
    }
}

// ## shots fixtures (R28, P19)

#[cfg(feature = "shots")]
mod shots {
    use std::time::{Duration, Instant};

    use hacivat_driver::ServerInfo;
    use hacivat_net::{TlsSummary, TlsVerified};
    use hacivat_proto::{DbmsKind, ErrClass};
    use hacivat_secrets::AuthProviderId;
    use hacivat_session::TestReport;
    use hacivat_session::pipeline::{Progress, StepOutcome, StepTrace, TrailStep};
    use hacivat_ui::dialog::{DialogSize, dialog_frame};
    use hacivat_ui_core::gpui_surface::{
        App, AppContext as _, Context, Entity, IntoElement, ParentElement, Render, Styled, Window,
        px,
    };
    use hacivat_ui_core::theme::Theme;

    use super::{TestStrip, TestStripState};
    use crate::connections::error_view::sample_error;
    use crate::shell::Workspace;
    use crate::shell::layers::{DIALOG_CONTEXT, Layer};

    /// A 640 px card holding the strip, as the connection dialog places it.
    struct StripHost {
        strip: Entity<TestStrip>,
    }

    impl Render for StripHost {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let theme = *Theme::get(cx);
            dialog_frame(&theme, DialogSize::Form)
                .p(px(0.0))
                .child(self.strip.clone())
        }
    }

    const STEPS: [(TrailStep, u64); 5] = [
        (TrailStep::Resolve, 3),
        (TrailStep::Connect, 14),
        (TrailStep::Tls, 22),
        (TrailStep::SignIn, 11),
        (TrailStep::CheckServer, 4),
    ];

    fn trail(done: usize) -> Vec<StepTrace> {
        STEPS
            .iter()
            .enumerate()
            .map(|(i, &(step, ms))| StepTrace {
                step,
                outcome: if i < done {
                    StepOutcome::Done
                } else if i == done {
                    StepOutcome::Running
                } else {
                    StepOutcome::Pending
                },
                elapsed: (i < done).then(|| Duration::from_millis(ms)),
            })
            .collect()
    }

    fn open(state: TestStripState, window: &mut Window, cx: &mut App) {
        let Some(ws) = window.root::<Workspace>().flatten() else {
            return;
        };
        let strip = cx.new(TestStrip::new);
        strip.update(cx, |s, cx| s.show(state, cx));
        let host = cx.new(|_| StripHost { strip });
        ws.update(cx, |ws, cx| {
            ws.push_layer(
                Layer {
                    id: "test-strip",
                    context: DIALOG_CONTEXT,
                    view: host.into(),
                    restore_focus: None,
                    modal: true,
                },
                window,
                cx,
            );
        });
    }

    /// A test running at TLS.
    pub fn running_fixture(window: &mut Window, cx: &mut App) {
        let progress = Progress {
            trail: trail(2),
            current: Some(TrailStep::Tls),
            started: Instant::now(),
        };
        open(TestStripState::Running(progress), window, cx);
    }

    /// A successful test: PostgreSQL 16.4 over verified TLS 1.3.
    pub fn ok_fixture(window: &mut Window, cx: &mut App) {
        let report = TestReport {
            ok: true,
            trail: trail(STEPS.len()),
            total: Duration::from_millis(54),
            server: Some(ServerInfo {
                kind: DbmsKind::Postgres,
                product: "PostgreSQL".to_string(),
                version: None,
                raw_version: "16.4".to_string(),
                is_replica: Some(false),
            }),
            support: None,
            tls: Some(TlsSummary {
                encrypted: true,
                version: Some("TLSv1.3".to_string()),
                cipher: Some("TLS13_AES_256_GCM_SHA384".to_string()),
                verified: TlsVerified::Full,
            }),
            auth: AuthProviderId::Password,
            user: Some("deploy".to_string()),
            ping_median: Some(Duration::from_millis(12)),
            warnings: Vec::new(),
            error: None,
            // ## m2b-t19-tls
            profile_changes: Vec::new(),
        };
        open(TestStripState::Done(report.into()), window, cx);
    }

    /// A failed test: the password was rejected.
    pub fn failed_fixture(window: &mut Window, cx: &mut App) {
        let error = sample_error(
            ErrClass::AuthInvalid,
            TrailStep::SignIn,
            Some("28P01"),
            "password authentication failed for user \"deploy\"",
        );
        let report = TestReport {
            ok: false,
            trail: error.trail.clone(),
            total: Duration::from_millis(214),
            server: None,
            support: None,
            tls: None,
            auth: AuthProviderId::Password,
            user: Some("deploy".to_string()),
            ping_median: None,
            warnings: Vec::new(),
            error: Some(error),
            // ## m2b-t19-tls
            profile_changes: Vec::new(),
        };
        open(TestStripState::Done(report.into()), window, cx);
    }
}

#[cfg(feature = "shots")]
pub use shots::{failed_fixture, ok_fixture, running_fixture};
