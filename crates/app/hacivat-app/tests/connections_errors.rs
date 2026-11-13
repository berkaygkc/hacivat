//! The test strip, the connect-error dialog and its quick fixes, the production-tag prompt and
//! the password prompt (Task 20; NOTES surfaces 1 test, 3, 4; R3, R11, R14, R26, P7, P8, P22).
//!
//! Shells come from `ConnectionsBoot::for_tests` with a FakeDriver registry. Hygiene: every
//! profile connects to `127.0.0.1` on the port of a `TcpListener` the test holds, so the
//! pipeline opens a real loopback stream and never sends a DNS query; the fake driver answers.

mod support;

use std::cell::RefCell;
use std::net::TcpListener;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::{
    AppContext as _, Context, Entity, Global, IntoElement, Modifiers, ParentElement as _, Render,
    Role, Styled as _, Task, TestAppContext, VisualTestContext, Window, div,
};
use hacivat_app::connections::ConnectionsModel;
use hacivat_app::connections::error_view::{ErrorView, open_error_dialog_view};
use hacivat_app::connections::fixes::FixAction;
use hacivat_app::connections::interaction::interaction;
use hacivat_app::connections::prod_prompt::ask_environment;
use hacivat_app::connections::test_strip::{TestStrip, TestStripEvent, TestStripState};
use hacivat_app::shell::Layer;
use hacivat_app::{Startup, start};
use hacivat_driver::fake::FakeDriver;
use hacivat_driver::{DriverRegistry, RawError, RawErrorKind, ServerInfo, ServerMessage};
use hacivat_l10n::OsLocale;
use hacivat_net::{TlsMode, TlsSummary, TlsVerified};
use hacivat_proto::{DbmsKind, EnvTag, ErrClass, ProfileId};
use hacivat_secrets::{AuthProviderId, ExposeSecret, SecretString};
use hacivat_session::pipeline::{
    Answer, ConnectError, ErrArgs, Interaction, Phase, Question, StepOutcome, StepTrace, TrailStep,
};
use hacivat_session::profile::{HintSource, Profile, ProfileDraft, ProfileEndpoint};
use hacivat_session::{LiveState, TestReport};
use hacivat_settings::{LanguagePref, LocalPaths};
use hacivat_ui::button::ButtonKind;
use hacivat_ui::popover::{A11yDecl, a11y_record, a11y_take};
use hacivat_ui::text::TextRegistry;
use support::*;
use tokio::sync::oneshot;

// ---- shells ------------------------------------------------------------------------------------

/// What a test shell keeps alive: temp roots, loopback listeners, the fake, the open views.
#[derive(Default)]
struct Kept {
    roots: Vec<tempfile::TempDir>,
    listeners: Vec<TcpListener>,
    fake: Option<Arc<FakeDriver>>,
    view: Option<Entity<ErrorView>>,
    strip: Option<Entity<TestStrip>>,
    profile: Option<ProfileId>,
    strip_events: Rc<RefCell<Vec<TestStripEvent>>>,
}

impl Global for Kept {}

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

/// A connections shell in `lang` whose PostgreSQL driver is `fake` (the other engines get a
/// default fake). A `lang` with a region (`tr-TR`) also sets the regional format: a UI language
/// alone never sets how numbers look (they follow the region, L15 d).
fn shell_with_fake(
    cx: &mut TestAppContext,
    lang: &str,
    fake: FakeDriver,
) -> &'static mut VisualTestContext {
    let root = tempfile::tempdir().expect("temp dir");
    let paths = LocalPaths::under(root.path());
    paths.ensure_dirs().expect("test dirs");
    let fake = Arc::new(fake);
    let mut drivers = DriverRegistry::new();
    drivers.register(fake.clone());
    for kind in [DbmsKind::MySql, DbmsKind::MariaDb, DbmsKind::Sqlite] {
        drivers.register(Arc::new(FakeDriver::new(kind)));
    }
    let boot = hacivat_app::connections::ConnectionsBoot::for_tests(&paths, drivers);
    let mut outcome = hacivat_settings::load(&paths.settings_toml());
    outcome.settings.ui.language = LanguagePref::Tag(lang.to_string());
    if lang.contains('-') {
        outcome.settings.ui.regional_format = LanguagePref::Tag(lang.to_string());
    }
    let mut startup = Startup::new(paths, outcome, OsLocale::default());
    startup.connections = Some(boot);
    let window = cx.update(|cx| {
        let kept = cx.default_global::<Kept>();
        kept.roots.push(root);
        kept.fake = Some(fake);
        cx.set_global(TextRegistry {
            enabled: true,
            frame: Vec::new(),
        });
        start(cx, startup, None).expect("open the workspace")
    });
    let vcx = VisualTestContext::from_window(window.into(), cx).into_mut();
    // Connects and tests run on the core runtime, whose threads wake the UI's tasks (P22).
    vcx.executor().allow_parking();
    vcx.run_until_parked();
    vcx
}

fn fake(vcx: &mut VisualTestContext) -> Arc<FakeDriver> {
    vcx.cx
        .read(|cx| cx.global::<Kept>().fake.clone())
        .expect("a fake shell")
}

/// A loopback port something listens on (the pipeline's transport connects to it).
fn listening_port(vcx: &mut VisualTestContext) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
    let port = listener.local_addr().expect("local addr").port();
    vcx.cx
        .update(|cx| cx.default_global::<Kept>().listeners.push(listener));
    port
}

fn ok_fake() -> FakeDriver {
    FakeDriver::new(DbmsKind::Postgres)
}

/// A server that rejects every password (SQLSTATE 28P01).
fn fake_always_28p01() -> FakeDriver {
    FakeDriver::new(DbmsKind::Postgres).connect_result(Err(RawError {
        kind: RawErrorKind::Db,
        server: Some(ServerMessage {
            sqlstate: Some("28P01".into()),
            vendor_code: None,
            severity: Some("FATAL".into()),
            text: "password authentication failed".into(),
            detail: None,
            hint: None,
            lang_hint: None,
        }),
    }))
}

/// Waits (in real time, P22) until `task` is done and returns its output.
fn wait<T: 'static>(vcx: &mut VisualTestContext, task: Task<T>) -> T {
    let out = Rc::new(RefCell::new(None));
    let slot = out.clone();
    vcx.cx.update(|cx| {
        cx.spawn(async move |_| {
            *slot.borrow_mut() = Some(task.await);
        })
        .detach();
    });
    for _ in 0..400 {
        if let Some(v) = out.borrow_mut().take() {
            return v;
        }
        settle(vcx, ms(5));
    }
    panic!("the task did not finish in 2 s");
}

/// A saved profile "orders-prod" for `deploy` at the loopback listener. A password profile
/// for the authentication tests; otherwise one without a password (no prompt).
fn saved_profile(vcx: &mut VisualTestContext, auth: AuthProviderId) -> ProfileId {
    saved_profile_with(vcx, auth, None)
}

/// Like [`saved_profile`], storing `password` for the profile (retention Forever, the memory
/// backend).
fn saved_profile_with(
    vcx: &mut VisualTestContext,
    auth: AuthProviderId,
    password: Option<&str>,
) -> ProfileId {
    let port = listening_port(vcx);
    let mut draft = ProfileDraft::new();
    draft.password = password.map(SecretString::from);
    draft.profile = Profile {
        name: "orders-prod".into(),
        endpoint: ProfileEndpoint::Server {
            host: "127.0.0.1".into(),
            port: Some(port),
        },
        database: Some("orders_v2".into()),
        user: Some("deploy".into()),
        auth,
        environment: Some(EnvTag::Production),
        ..Profile::default()
    };
    let task = vcx.cx.update(|cx| ConnectionsModel::save(cx, draft));
    let id = wait(vcx, task).expect("the profile saves");
    vcx.cx
        .update(|cx| cx.default_global::<Kept>().profile = Some(id));
    id
}

// ---- errors ------------------------------------------------------------------------------------

/// A trail failed at `failed` (steps before it done, after it pending).
fn trail_failed_at(failed: TrailStep) -> Vec<StepTrace> {
    let steps = [
        TrailStep::Resolve,
        TrailStep::Connect,
        TrailStep::Tls,
        TrailStep::SignIn,
        TrailStep::CheckServer,
    ];
    let at = steps
        .iter()
        .position(|s| *s == failed)
        .expect("a trail step");
    steps
        .iter()
        .enumerate()
        .map(|(i, &step)| StepTrace {
            step,
            outcome: if i < at {
                StepOutcome::Done
            } else if i == at {
                StepOutcome::Failed
            } else {
                StepOutcome::Pending
            },
            elapsed: (i < at).then(|| ms(3)),
        })
        .collect()
}

fn error_of(class: ErrClass, failed: Option<TrailStep>) -> ConnectError {
    ConnectError {
        class,
        phase: Phase::DriverConnect,
        trail: failed.map(trail_failed_at).unwrap_or_default(),
        args: ErrArgs {
            name: "orders-prod".into(),
            host: Some("db-stg.acme.io".into()),
            port: Some(5432),
            user: Some("deploy".into()),
            database: Some("orders_v2".into()),
            file: None,
            engine: "PostgreSQL".into(),
            version: None,
        },
        server: None,
        fixes: hacivat_driver::fixes_for(class),
        suggestions: Vec::new(),
        issues: Vec::new(),
        elapsed: ms(214),
        attempts: 0,
        needs_interaction: false,
    }
}

fn server_message(code: &str, text: &str) -> ServerMessage {
    ServerMessage {
        sqlstate: Some(code.into()),
        vendor_code: None,
        severity: Some("FATAL".into()),
        text: text.into(),
        detail: None,
        hint: None,
        lang_hint: None,
    }
}

/// `auth.invalid` for `user` after `attempts` server rejections (P8).
fn auth_invalid_attempts(user: &str, attempts: u8) -> ConnectError {
    let mut e = error_of(ErrClass::AuthInvalid, Some(TrailStep::SignIn));
    e.args.user = Some(user.into());
    e.attempts = attempts;
    e.server = Some(server_message("28P01", "password authentication failed"));
    e
}

fn auth_invalid_with_server(code: &str, text: &str) -> ConnectError {
    let mut e = auth_invalid_attempts("deploy", 1);
    e.server = Some(server_message(code, text));
    e
}

fn unknown_db(db: &str, suggestions: [&str; 3]) -> ConnectError {
    let mut e = error_of(ErrClass::ConfigUnknownDatabase, Some(TrailStep::SignIn));
    e.args.database = Some(db.into());
    e.suggestions = suggestions.iter().map(|s| s.to_string()).collect();
    e.server = Some(server_message("3D000", "database does not exist"));
    e
}

fn unsupported(version: &str) -> ConnectError {
    let mut e = error_of(
        ErrClass::ServerUnsupportedVersion,
        Some(TrailStep::CheckServer),
    );
    e.args.version = Some(version.into());
    e
}

fn tls_untrusted(host: &str) -> ConnectError {
    let mut e = error_of(ErrClass::TlsUntrusted, Some(TrailStep::Tls));
    e.args.host = Some(host.into());
    e
}

/// A shell over `fake` with a saved profile, and the error dialog open for `error`.
fn error_dialog(
    cx: &mut TestAppContext,
    error: ConnectError,
    fake: FakeDriver,
) -> &'static mut VisualTestContext {
    let vcx = shell_with_fake(cx, "en", fake);
    open_dialog_over(vcx, error);
    vcx
}

fn open_dialog_over(vcx: &mut VisualTestContext, error: ConnectError) {
    let auth = if error.class == ErrClass::AuthInvalid {
        AuthProviderId::Password
    } else {
        AuthProviderId::None
    };
    let id = match vcx.cx.read(|cx| cx.global::<Kept>().profile) {
        Some(id) => id,
        None => saved_profile(vcx, auth),
    };
    let view = in_window(vcx, |window, cx| {
        open_error_dialog_view(id, Arc::new(error), window, cx)
    })
    .expect("the workspace opens the dialog");
    vcx.cx
        .update(|cx| cx.default_global::<Kept>().view = Some(view));
    vcx.run_until_parked();
}

fn view(vcx: &mut VisualTestContext) -> Entity<ErrorView> {
    vcx.cx
        .read(|cx| cx.global::<Kept>().view.clone())
        .expect("an error dialog")
}

fn profile_id(vcx: &mut VisualTestContext) -> ProfileId {
    vcx.cx
        .read(|cx| cx.global::<Kept>().profile)
        .expect("a saved profile")
}

fn profile(vcx: &mut VisualTestContext) -> Arc<Profile> {
    let id = profile_id(vcx);
    vcx.cx
        .read(|cx| cx.global::<ConnectionsModel>().get(id))
        .expect("the profile")
}

fn live_of(vcx: &mut VisualTestContext) -> LiveState {
    let id = profile_id(vcx);
    vcx.cx
        .read(|cx| cx.global::<ConnectionsModel>().manager().state(id))
}

// ---- reading the frame -------------------------------------------------------------------------

fn strip_marks(s: &str) -> String {
    s.chars()
        .filter(|c| !matches!(c, '\u{2068}' | '\u{2069}'))
        .collect()
}

/// The text drawn with ID `id` (a label's `#id`), if any.
fn try_text_of(vcx: &mut VisualTestContext, id: &str) -> Option<String> {
    let needle = format!("#{id}");
    registry_frame(vcx)
        .into_iter()
        .find(|r| r.path.ends_with(&needle))
        .map(|r| r.text.to_string())
}

fn text_of(vcx: &mut VisualTestContext, id: &str) -> String {
    try_text_of(vcx, id).unwrap_or_else(|| panic!("no text `{id}` in the frame"))
}

/// The value drawn beside the label `label` in a `<prefix>-<key>-label` / `-value` table.
fn row_value(vcx: &mut VisualTestContext, prefix: &str, label: &str) -> String {
    let frame = registry_frame(vcx);
    let key = frame
        .iter()
        .filter_map(|r| {
            let (_, id) = r.path.rsplit_once('#')?;
            let key = id.strip_prefix(prefix)?.strip_suffix("-label")?;
            (r.text.as_ref() == label).then(|| key.to_string())
        })
        .next()
        .unwrap_or_else(|| panic!("no `{label}` row"));
    text_of(vcx, &format!("{prefix}{key}-value"))
}

/// A selector as GPUI's lookup wants it (`'static`; a test leaks a few short strings).
fn sel(s: &str) -> &'static str {
    Box::leak(s.to_string().into_boxed_str())
}

fn has_id(vcx: &mut VisualTestContext, selector: &str) -> bool {
    vcx.run_until_parked();
    vcx.debug_bounds(sel(selector)).is_some()
}

/// Clicks the element `id`: a `Button` (`button-<id>`) or any element with that selector.
fn click(vcx: &mut VisualTestContext, id: &str) {
    vcx.run_until_parked();
    let bounds = match vcx.debug_bounds(sel(&format!("button-{id}"))) {
        Some(b) => b,
        None => vcx
            .debug_bounds(sel(id))
            .unwrap_or_else(|| panic!("nothing to click at `{id}`")),
    };
    vcx.simulate_click(bounds.center(), Modifiers::none());
    vcx.run_until_parked();
}

fn clipboard(vcx: &mut VisualTestContext) -> String {
    vcx.read_from_clipboard()
        .and_then(|c| c.text())
        .unwrap_or_default()
}

/// The accessibility declarations of one fresh frame.
fn decls(vcx: &mut VisualTestContext) -> Vec<A11yDecl> {
    a11y_record();
    vcx.cx.update(|cx| cx.refresh_windows());
    vcx.run_until_parked();
    a11y_take()
}

// ---- error dialog readers ----------------------------------------------------------------------

fn error_title(vcx: &mut VisualTestContext) -> String {
    text_of(vcx, "error-title")
}

/// The fix buttons the dialog offers, each checked to be drawn under its ID.
fn fix_buttons(vcx: &mut VisualTestContext) -> Vec<(String, String, ButtonKind, bool)> {
    vcx.run_until_parked();
    let v = view(vcx);
    let buttons = vcx.cx.read(|cx| v.read(cx).fix_buttons(cx));
    buttons
        .into_iter()
        .map(|b| {
            assert!(
                has_id(vcx, &format!("button-{}", b.id)),
                "fix `{}` is drawn",
                b.id
            );
            (b.id.to_string(), strip_marks(&b.label), b.kind, b.disabled)
        })
        .collect()
}

fn fix_labels(vcx: &mut VisualTestContext) -> Vec<String> {
    fix_buttons(vcx).into_iter().map(|b| b.1).collect()
}

fn button_kind(vcx: &mut VisualTestContext, id: &str) -> ButtonKind {
    fix_buttons(vcx)
        .into_iter()
        .find(|b| b.0 == id)
        .map(|b| b.2)
        .unwrap_or_else(|| panic!("no fix `{id}`"))
}

fn attempt_label(vcx: &mut VisualTestContext) -> String {
    text_of(vcx, "reenter-attempt")
}

/// Focuses the re-entry field and types `text` into it.
fn type_password(vcx: &mut VisualTestContext, text: &str) {
    click(vcx, "reenter-field");
    vcx.simulate_input(text);
    vcx.run_until_parked();
}

fn expand(vcx: &mut VisualTestContext, id: &str) {
    click(vcx, id);
}

fn tech_row(vcx: &mut VisualTestContext, label: &str) -> String {
    row_value(vcx, "tech-", label)
}

// ---- strip -------------------------------------------------------------------------------------

/// A 640 px host for a test strip, as the connection dialog places it.
struct StripHost {
    strip: Entity<TestStrip>,
}

impl Render for StripHost {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().w(gpui::px(640.0)).child(self.strip.clone())
    }
}

/// Pushes a layer hosting a new test strip and returns the strip.
fn host_strip(vcx: &mut VisualTestContext) -> Entity<TestStrip> {
    let events = vcx
        .cx
        .update(|cx| cx.default_global::<Kept>().strip_events.clone());
    let strip = with_ws(vcx, |w, window, cx| {
        let strip = cx.new(TestStrip::new);
        let sink = events.clone();
        cx.subscribe(&strip, move |_, _, e: &TestStripEvent, _| {
            sink.borrow_mut().push(e.clone());
        })
        .detach();
        let host = cx.new(|_| StripHost {
            strip: strip.clone(),
        });
        w.push_layer(
            Layer {
                id: "strip-host",
                context: "Dialog",
                view: host.into(),
                restore_focus: None,
                modal: true,
            },
            window,
            cx,
        );
        strip
    });
    vcx.cx
        .update(|cx| cx.default_global::<Kept>().strip = Some(strip.clone()));
    vcx.run_until_parked();
    strip
}

fn strip_fixture(
    cx: &mut TestAppContext,
    lang: &str,
    report: TestReport,
) -> &'static mut VisualTestContext {
    let vcx = shell_with_connections(cx, lang);
    let strip = host_strip(vcx);
    vcx.cx.update(|cx| {
        strip.update(cx, |s, cx| {
            s.show(TestStripState::Done(Arc::new(report)), cx)
        })
    });
    vcx.run_until_parked();
    vcx
}

fn ok_report(total: Duration, ping: Duration, server: &str, tls: TlsSummary) -> TestReport {
    let (product, version) = server.split_once(' ').expect("product and version");
    TestReport {
        ok: true,
        trail: [
            (TrailStep::Resolve, 3),
            (TrailStep::Connect, 14),
            (TrailStep::Tls, 22),
            (TrailStep::SignIn, 11),
            (TrailStep::CheckServer, 4),
        ]
        .iter()
        .map(|&(step, n)| StepTrace {
            step,
            outcome: StepOutcome::Done,
            elapsed: Some(ms(n)),
        })
        .collect(),
        total,
        server: Some(ServerInfo {
            kind: DbmsKind::Postgres,
            product: product.into(),
            version: None,
            raw_version: version.into(),
            is_replica: Some(false),
        }),
        support: None,
        tls: Some(tls),
        auth: AuthProviderId::Password,
        user: Some("deploy".into()),
        ping_median: Some(ping),
        warnings: Vec::new(),
        error: None,
    }
}

fn tls13_verified() -> TlsSummary {
    TlsSummary {
        encrypted: true,
        version: Some("TLSv1.3".into()),
        cipher: Some("TLS13_AES_256_GCM_SHA384".into()),
        verified: TlsVerified::Full,
    }
}

fn strip_title(vcx: &mut VisualTestContext) -> String {
    text_of(vcx, "test-title")
}

fn strip_clock(vcx: &mut VisualTestContext) -> String {
    text_of(vcx, "test-clock")
}

fn diag(vcx: &mut VisualTestContext, label: &str) -> String {
    row_value(vcx, "diag-", label)
}

fn strip(vcx: &mut VisualTestContext) -> Entity<TestStrip> {
    vcx.cx
        .read(|cx| cx.global::<Kept>().strip.clone())
        .expect("a strip")
}

/// A strip running a test against a server that takes 30 s to answer.
fn strip_running(cx: &mut TestAppContext, lang: &str) -> &'static mut VisualTestContext {
    let vcx = shell_with_fake(
        cx,
        lang,
        FakeDriver::new(DbmsKind::Postgres).delay(ms(30_000)),
    );
    let port = listening_port(vcx);
    let strip = host_strip(vcx);
    let mut draft = ProfileDraft::new();
    draft.profile = Profile {
        name: "orders".into(),
        endpoint: ProfileEndpoint::Server {
            host: "127.0.0.1".into(),
            port: Some(port),
        },
        auth: AuthProviderId::None,
        ..Profile::default()
    };
    vcx.cx
        .update(|cx| strip.update(cx, |s, cx| s.run(draft, cx)));
    // The core runtime publishes progress from its own threads (P22).
    settle(vcx, ms(20));
    vcx
}

// ---- production prompt -------------------------------------------------------------------------

fn ask(
    cx: &mut TestAppContext,
    lang: &str,
    name: &str,
    host: &str,
    hint: Option<(&'static str, HintSource)>,
) -> (
    &'static mut VisualTestContext,
    oneshot::Receiver<Option<EnvTag>>,
) {
    let vcx = shell_with_connections(cx, lang);
    let (name, host) = (name.to_string(), host.to_string());
    let rx = in_window(vcx, |window, cx| {
        ask_environment(name, host, hint, window, cx)
    });
    vcx.run_until_parked();
    (vcx, rx)
}

fn choice_labels(vcx: &mut VisualTestContext) -> Vec<String> {
    registry_frame(vcx)
        .into_iter()
        .filter(|r| r.path.contains("hacivat-ui/src/choice_list.rs:") && r.size_px == 13.0)
        .map(|r| r.text.to_string())
        .collect()
}

fn selected_choice(vcx: &mut VisualTestContext) -> Option<usize> {
    decls(vcx)
        .into_iter()
        .filter(|d| d.role == Some(Role::RadioButton))
        .position(|d| d.selected == Some(true))
}

/// The prompt's primary: the last button drawn (the prompt is the top layer).
fn primary(vcx: &mut VisualTestContext) -> A11yDecl {
    decls(vcx)
        .into_iter()
        .rfind(|d| d.role == Some(Role::Button))
        .expect("a button")
}

fn primary_label(vcx: &mut VisualTestContext) -> String {
    strip_marks(&primary(vcx).name)
}

fn primary_disabled(vcx: &mut VisualTestContext) -> bool {
    primary(vcx).disabled
}

fn suggest_line(vcx: &mut VisualTestContext) -> String {
    text_of(vcx, "prod-suggest")
}

fn calm_line(vcx: &mut VisualTestContext) -> String {
    text_of(vcx, "prod-calm")
}

fn recv(vcx: &mut VisualTestContext, mut rx: oneshot::Receiver<Option<EnvTag>>) -> Option<EnvTag> {
    vcx.run_until_parked();
    rx.try_recv().expect("the prompt answered")
}

// ---- tests: the test strip ---------------------------------------------------------------------

#[gpui::test]
fn ok_strip_shows_total_and_diagnostics(cx: &mut TestAppContext) {
    let vcx = strip_fixture(
        cx,
        "en",
        ok_report(ms(54), ms(12), "PostgreSQL 16.4", tls13_verified()),
    );
    assert_eq!(strip_title(vcx), "Connected in 54 ms");
    assert_eq!(
        text_of(vcx, "test-sub"),
        "Nothing was saved. The test opened its own connection and closed it."
    );
    assert_eq!(diag(vcx, "Server"), "PostgreSQL 16.4");
    assert_eq!(diag(vcx, "Latency"), "12 ms · median of 3 pings");
    assert_eq!(diag(vcx, "Encryption"), "TLS 1.3 · verified");
    assert_eq!(diag(vcx, "Signed in as"), "deploy · Password");
    assert_eq!(diag(vcx, "Warnings"), "None");
    assert_eq!(text_of(vcx, "diag-tls-cipher"), "TLS13_AES_256_GCM_SHA384");
    // The trail shows a duration per step.
    let durations: Vec<String> = registry_frame(vcx)
        .into_iter()
        .filter(|r| r.path.contains("hacivat-ui/src/step_trail.rs:") && r.text.ends_with(" ms"))
        .map(|r| r.text.to_string())
        .collect();
    assert_eq!(durations, ["3 ms", "14 ms", "22 ms", "11 ms", "4 ms"]);
}

#[gpui::test]
fn ok_strip_lists_every_warning(cx: &mut TestAppContext) {
    let mut report = ok_report(ms(54), ms(12), "PostgreSQL 18.1", tls13_verified());
    report.warnings = vec![
        hacivat_session::TestWarning::NotTested {
            version: "18.1".into(),
        },
        hacivat_session::TestWarning::Probe(hacivat_driver::ProbeWarning::LowWaitTimeout {
            seconds: 60,
        }),
    ];
    let vcx = strip_fixture(cx, "en", report);
    assert_eq!(
        diag(vcx, "Warnings"),
        "Hacivat hasn't been tested with version 18.1 yet."
    );
    assert_eq!(
        text_of(vcx, "diag-warn-value-1"),
        "The server closes idle connections after 60 s."
    );
}

#[gpui::test]
fn running_strip_ticks_in_region_format(cx: &mut TestAppContext) {
    let vcx = strip_running(cx, "tr-TR");
    assert!(strip(vcx).read_with(vcx, |s, _| s.is_running()));
    assert_eq!(strip_title(vcx), "Test ediliyor");
    vcx.executor().advance_clock(ms(60)); // the executor clock (P22)
    assert_eq!(strip_clock(vcx), "0,06 sn");
}

#[gpui::test]
fn stop_cancels_the_running_test(cx: &mut TestAppContext) {
    let vcx = strip_running(cx, "en");
    let s = strip(vcx);
    let stopped = vcx.cx.update(|cx| s.update(cx, |s, cx| s.stop(cx)));
    assert!(stopped);
    assert_eq!(
        s.read_with(vcx, |s, _| s.state().clone()),
        TestStripState::Idle
    );
    assert!(!vcx.cx.update(|cx| s.update(cx, |s, cx| s.stop(cx))));
    assert!(try_text_of(vcx, "test-clock").is_none());
}

#[gpui::test]
fn a_finished_test_shows_its_report_and_says_so(cx: &mut TestAppContext) {
    let vcx = shell_with_fake(cx, "en", ok_fake());
    let port = listening_port(vcx);
    let strip = host_strip(vcx);
    let mut draft = ProfileDraft::new();
    draft.profile = Profile {
        name: "orders".into(),
        endpoint: ProfileEndpoint::Server {
            host: "127.0.0.1".into(),
            port: Some(port),
        },
        auth: AuthProviderId::None,
        ..Profile::default()
    };
    vcx.cx
        .update(|cx| strip.update(cx, |s, cx| s.run(draft, cx)));
    for _ in 0..100 {
        settle(vcx, ms(10));
        if !strip.read_with(vcx, |s, _| s.is_running()) {
            break;
        }
    }
    assert!(matches!(
        strip.read_with(vcx, |s, _| s.state().clone()),
        TestStripState::Done(r) if r.ok
    ));
    assert!(strip_title(vcx).starts_with("Connected in "));
    let events = vcx
        .cx
        .read(|cx| cx.global::<Kept>().strip_events.borrow().clone());
    assert_eq!(events, [TestStripEvent::Finished]);
}

#[gpui::test]
fn copy_report_is_english_in_a_turkish_ui(cx: &mut TestAppContext) {
    let vcx = strip_fixture(
        cx,
        "tr",
        ok_report(ms(54), ms(12), "PostgreSQL 16.4", tls13_verified()),
    );
    assert_eq!(strip_title(vcx), "54 ms içinde bağlandı");
    click(vcx, "copy-report");
    let text = clipboard(vcx);
    assert!(text.starts_with("Connected in 54 ms"), "{text}");
    assert!(
        text.contains("Latency: 12 ms · median of 3 pings"),
        "{text}"
    );
    assert!(text.contains("Encryption: TLS 1.3 · verified"), "{text}");
    assert!(
        !text.contains('\u{2068}'),
        "no isolation marks on the clipboard"
    );
}

#[gpui::test]
fn a_failed_test_shows_the_error_inline_and_emits_its_fixes(cx: &mut TestAppContext) {
    let mut report = ok_report(ms(54), ms(12), "PostgreSQL 16.4", tls13_verified());
    report.ok = false;
    report.error = Some(auth_invalid_attempts("deploy", 1));
    let vcx = strip_fixture(cx, "en", report);
    assert_eq!(
        error_title(vcx),
        "The server rejected the password for deploy."
    );
    // Inline, the strip sits on the form: no "Edit connection" (the mock).
    assert!(has_id(vcx, "button-fix-reenter"));
    assert!(!has_id(vcx, "button-fix-edit"));
    click(vcx, "fix-reenter");
    let events = vcx
        .cx
        .read(|cx| cx.global::<Kept>().strip_events.borrow().clone());
    assert_eq!(events, [TestStripEvent::Fix(FixAction::ReenterPassword)]);
    // The inline block never opens its own field: the dialog's password field is the one.
    assert!(!has_id(vcx, "reenter-field"));
}

// ---- tests: the error dialog -------------------------------------------------------------------

#[gpui::test]
fn the_error_dialog_is_a_480_modal_with_the_trail(cx: &mut TestAppContext) {
    let vcx = error_dialog(cx, auth_invalid_attempts("deploy", 1), ok_fake());
    assert_eq!(layer_ids(vcx), ["connection-error"]);
    let bounds = vcx.debug_bounds("connection-error").expect("the dialog");
    assert_eq!(bounds.size.width, gpui::px(480.0));
    assert_eq!(
        text_of(vcx, "error-dialog-title"),
        "Couldn't connect to orders-prod"
    );
    // R25: the ✕ at Sign in, the steps before it done.
    assert!(has_id(vcx, "step-trail"));
    let items: Vec<(String, Option<String>)> = decls(vcx)
        .into_iter()
        .filter(|d| d.role == Some(Role::ListItem))
        .map(|d| (d.name.to_string(), d.value.map(|v| v.to_string())))
        .collect();
    assert_eq!(
        items,
        [
            ("Resolve".to_string(), Some("done".to_string())),
            ("Connect".to_string(), Some("done".to_string())),
            ("TLS".to_string(), Some("done".to_string())),
            ("Sign in".to_string(), Some("failed".to_string())),
            ("Check server".to_string(), Some("not started".to_string())),
        ]
    );
    // Escape closes it.
    vcx.simulate_keystrokes("escape");
    assert!(layer_ids(vcx).is_empty());
}

#[gpui::test]
fn auth_invalid_reenter_counts_and_stops_at_three(cx: &mut TestAppContext) {
    // P8: the stored password was rejected once (attempts: 1).
    let vcx = error_dialog(cx, auth_invalid_attempts("deploy", 1), fake_always_28p01());
    assert_eq!(
        error_title(vcx),
        "The server rejected the password for deploy."
    );
    click(vcx, "fix-reenter");
    let mut labels = vec![];
    while has_id(vcx, "reenter-field") {
        labels.push(attempt_label(vcx));
        type_password(vcx, "x");
        click(vcx, "fix-connect");
        settle(vcx, ms(50));
    }
    assert_eq!(labels, ["Attempt 2 of 3", "Attempt 3 of 3"]);
    assert_eq!(fake(vcx).connect_calls(), 2);
    let sent = fake(vcx).last_request().expect("a connect");
    assert!(
        sent.password.as_deref() == Some("x"),
        "the re-entered password filled the password slot"
    );
    // The gesture's budget is spent: no field, no re-enter fix, the dialog stays.
    assert_eq!(view(vcx).read_with(vcx, |v, _| v.rejections()), 3);
    assert!(!fix_labels(vcx).contains(&"Enter password again".to_string()));
    assert_eq!(layer_ids(vcx), ["connection-error"]);
}

#[gpui::test]
fn a_reentered_password_that_works_closes_the_dialog(cx: &mut TestAppContext) {
    let vcx = error_dialog(cx, auth_invalid_attempts("deploy", 1), ok_fake());
    click(vcx, "fix-reenter");
    type_password(vcx, "hv_test_pw");
    vcx.simulate_keystrokes("enter");
    for _ in 0..50 {
        settle(vcx, ms(10));
        if layer_ids(vcx).is_empty() {
            break;
        }
    }
    assert!(layer_ids(vcx).is_empty());
    assert!(matches!(live_of(vcx), LiveState::Connected(_)));
    // The fake saw the re-entered password as the draft (it is never shown).
    let sent = fake(vcx).last_request().and_then(|r| r.password);
    assert!(
        sent.as_deref() == Some("hv_test_pw"),
        "the re-entered password was sent"
    );
}

#[gpui::test]
fn unknown_database_offers_top_three(cx: &mut TestAppContext) {
    let vcx = error_dialog(
        cx,
        unknown_db("orders_v2", ["orders", "orders_archive", "postgres"]),
        ok_fake(),
    );
    assert_eq!(
        fix_labels(vcx),
        [
            "Use orders",
            "Use orders_archive",
            "Use postgres",
            "Edit connection"
        ]
    );
    assert_eq!(button_kind(vcx, "fix-use-0"), ButtonKind::Primary);
    click(vcx, "fix-use-0");
    settle(vcx, ms(50));
    assert_eq!(profile(vcx).database.as_deref(), Some("orders"));
    let sent = fake(vcx).last_request().expect("a connect");
    assert_eq!(sent.database.as_deref(), Some("orders"));
    assert!(matches!(live_of(vcx), LiveState::Connected(_)));
    assert!(
        layer_ids(vcx).is_empty(),
        "a connect that works closes the dialog"
    );
}

#[gpui::test]
fn an_empty_suggestion_list_offers_no_use_buttons(cx: &mut TestAppContext) {
    let mut e = unknown_db("orders_v2", ["a", "b", "c"]);
    e.suggestions.clear(); // SQLite CANTOPEN, or no list
    let vcx = error_dialog(cx, e, ok_fake());
    assert_eq!(fix_labels(vcx), ["Edit connection"]);
}

#[gpui::test]
fn unsupported_offers_ghost_connect_anyway_and_persists(cx: &mut TestAppContext) {
    let vcx = error_dialog(cx, unsupported("9.6.24"), ok_fake());
    assert_eq!(
        error_title(vcx),
        "db-stg.acme.io runs PostgreSQL 9.6.24, which Hacivat doesn't support."
    );
    assert_eq!(
        fix_labels(vcx),
        [
            "Supported versions",
            "Edit connection",
            "Connect anyway, unsupported"
        ]
    );
    assert_eq!(button_kind(vcx, "fix-anyway"), ButtonKind::Ghost);
    click(vcx, "fix-anyway");
    settle(vcx, ms(50));
    assert!(profile(vcx).allow_unsupported_server);
    let file = std::fs::read_to_string(paths(vcx).connections_toml()).expect("the file");
    assert!(file.contains("allow_unsupported_server = true"), "{file}");
}

#[gpui::test]
fn tls_untrusted_encrypt_without_verifying(cx: &mut TestAppContext) {
    let vcx = error_dialog(cx, tls_untrusted("db-qa.acme.io"), ok_fake());
    assert_eq!(fix_labels(vcx)[0], "Encrypt without verifying");
    // R3: what the fix gives up is said plainly.
    assert_eq!(
        text_of(vcx, "fix-require-note"),
        "The connection will be encrypted, but Hacivat won't check that the server is who it claims to be."
    );
    assert_ne!(button_kind(vcx, "fix-require"), ButtonKind::Primary);
    click(vcx, "fix-require");
    settle(vcx, ms(50));
    assert_eq!(profile(vcx).tls.mode, TlsMode::Require);
    let sent = fake(vcx).last_request().expect("a connect");
    assert_eq!(
        sent.tls.expect("TLS parameters").settings.mode,
        TlsMode::Require
    );
}

#[gpui::test]
fn config_invalid_offers_edit_connection(cx: &mut TestAppContext) {
    let vcx = error_dialog(cx, error_of(ErrClass::ConfigInvalid, None), ok_fake());
    assert_eq!(fix_labels(vcx), ["Edit connection"]);
    // R25: config.invalid has no trail.
    assert!(!has_id(vcx, "step-trail"));
}

#[gpui::test]
fn technical_details_keep_server_text_verbatim_and_out_of_the_title(cx: &mut TestAppContext) {
    let vcx = error_dialog(
        cx,
        auth_invalid_with_server("28P01", "ÖLÜMCÜL: parola doğrulaması başarısız"),
        ok_fake(),
    );
    assert!(!has_id(vcx, "tech-box"));
    expand(vcx, "tech-details");
    assert_eq!(
        tech_row(vcx, "Server message"),
        "ÖLÜMCÜL: parola doğrulaması başarısız"
    );
    assert!(!error_title(vcx).contains("ÖLÜMCÜL"));
    assert_eq!(tech_row(vcx, "Class"), "auth.invalid");
    assert_eq!(tech_row(vcx, "Code"), "28P01");
    assert_eq!(tech_row(vcx, "Failed at"), "Sign in");
    assert_eq!(tech_row(vcx, "Elapsed"), "0.214 s");
    // Copy details is English and carries the server's text verbatim.
    click(vcx, "copy-details");
    let text = clipboard(vcx);
    assert!(
        text.starts_with("Couldn't connect to orders-prod\n"),
        "{text}"
    );
    assert!(text.contains("Server message: ÖLÜMCÜL: parola doğrulaması başarısız"));
    assert!(text.contains("Class: auth.invalid"));
}

#[gpui::test]
fn a_read_only_store_turns_profile_fixes_off_and_says_why(cx: &mut TestAppContext) {
    // A connections file from a newer Hacivat: the profile loads, but nothing can be saved.
    let first = shell_with_fake(cx, "en", ok_fake());
    let id = saved_profile(first, AuthProviderId::None);
    let text = std::fs::read_to_string(paths(first).connections_toml()).expect("the file");
    let newer = text.replace("version = 1", "version = 2");
    let vcx = shell_with_connections_file(cx, "en", &newer);
    assert!(
        vcx.cx
            .read(|cx| cx.global::<ConnectionsModel>().store_read_only())
    );
    let view = in_window(vcx, |window, cx| {
        open_error_dialog_view(
            id,
            Arc::new(unknown_db("orders_v2", ["orders", "b", "c"])),
            window,
            cx,
        )
    })
    .expect("the dialog");
    vcx.run_until_parked();
    let disabled: Vec<(String, bool)> = vcx.cx.read(|cx| {
        view.read(cx)
            .fix_buttons(cx)
            .into_iter()
            .map(|b| (b.id.to_string(), b.disabled))
            .collect()
    });
    assert_eq!(
        disabled,
        [
            ("fix-use-0".to_string(), true),
            ("fix-use-1".to_string(), true),
            ("fix-use-2".to_string(), true),
            ("fix-edit".to_string(), false),
        ]
    );
    assert_eq!(
        text_of(vcx, "fix-read-only"),
        "Your connections file can't be changed, so fixes that edit this connection are off."
    );
}

#[gpui::test]
fn tab_stays_inside_the_error_dialog(cx: &mut TestAppContext) {
    let vcx = error_dialog(cx, unsupported("9.6.24"), ok_fake());
    let mut seen = Vec::new();
    for _ in 0..6 {
        vcx.simulate_keystrokes("tab");
        let focused = focused(vcx).expect("something is focused");
        seen.push(focused);
    }
    // 3 fixes, the details toggle and Close: five distinct stops, then the cycle wraps.
    let mut distinct: Vec<_> = Vec::new();
    for handle in &seen[..5] {
        if !distinct.contains(handle) {
            distinct.push(handle.clone());
        }
    }
    assert_eq!(distinct.len(), 5);
    assert_eq!(seen[5], seen[0]);
    assert_eq!(layer_ids(vcx), ["connection-error"]);
}

// ---- tests: the production prompt --------------------------------------------------------------

#[gpui::test]
fn production_prompt_preselects_and_names_the_result(cx: &mut TestAppContext) {
    let (vcx, rx) = ask(
        cx,
        "en",
        "orders-prod-us",
        "db-prod-us.acme.io",
        Some(("prod", HintSource::Host)),
    );
    assert_eq!(layer_ids(vcx), ["production-prompt"]);
    assert_eq!(
        vcx.debug_bounds("production-prompt")
            .expect("the prompt")
            .size
            .width,
        gpui::px(480.0)
    );
    assert_eq!(choice_labels(vcx), ["Production", "Staging", "Test", "Dev"]);
    assert_eq!(selected_choice(vcx), Some(0));
    assert_eq!(
        suggest_line(vcx),
        "Suggested because the host name contains “prod”."
    );
    assert_eq!(text_of(vcx, "prod-word"), "PRODUCTION");
    assert_eq!(primary_label(vcx), "Save as production");
    assert!(!primary_disabled(vcx));
    vcx.simulate_keystrokes("enter");
    assert_eq!(recv(vcx, rx), Some(EnvTag::Production));
    assert!(layer_ids(vcx).is_empty());
}

#[gpui::test]
fn production_prompt_names_a_name_match(cx: &mut TestAppContext) {
    let (vcx, _rx) = ask(
        cx,
        "en",
        "orders-PRD",
        "10.0.4.7",
        Some(("prd", HintSource::Name)),
    ); // P7
    assert_eq!(selected_choice(vcx), Some(0));
    assert_eq!(
        suggest_line(vcx),
        "Suggested because the name contains “prd”."
    );
}

#[gpui::test]
fn production_prompt_without_hint(cx: &mut TestAppContext) {
    let (vcx, rx) = ask(cx, "tr", "crm", "10.0.4.7", None);
    assert_eq!(selected_choice(vcx), None);
    assert!(try_text_of(vcx, "prod-suggest").is_none());
    assert_eq!(primary_label(vcx), "Bir ortam seçin");
    assert!(primary_disabled(vcx));
    vcx.simulate_keystrokes("enter"); // nothing chosen: Enter does nothing
    assert_eq!(layer_ids(vcx), ["production-prompt"]);
    vcx.simulate_keystrokes("down down");
    assert_eq!(
        calm_line(vcx),
        "Ek onay yok. İfadeler çalıştıkça commit edilir."
    );
    assert_eq!(primary_label(vcx), "Hazırlık olarak kaydet");
    vcx.simulate_keystrokes("escape");
    assert_eq!(recv(vcx, rx), None);
    assert!(layer_ids(vcx).is_empty());
}

// ---- tests: the password prompt and the UI port ------------------------------------------------

/// Asks `question` through `ix` on the core runtime; the answer arrives on the returned channel.
fn spawn_on_core(
    vcx: &mut VisualTestContext,
    ix: Interaction,
    attempt: u8,
) -> std::sync::mpsc::Receiver<Answer> {
    let Interaction::Interactive(port) = ix else {
        panic!("the app's interaction is interactive");
    };
    let rt = vcx.cx.read(|cx| cx.global::<ConnectionsModel>().runtime());
    let (tx, rx) = std::sync::mpsc::channel();
    rt.spawn(async move {
        let answer = port
            .ask(Question::Password {
                profile: ProfileId::new(),
                name: "orders-prod".into(),
                user: "deploy".into(),
                attempt,
            })
            .await;
        let _ = tx.send(answer);
    });
    rx
}

fn block(vcx: &mut VisualTestContext, rx: std::sync::mpsc::Receiver<Answer>) -> Answer {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        if let Ok(answer) = rx.try_recv() {
            return answer;
        }
        assert!(Instant::now() < deadline, "no answer in 2 s");
        settle(vcx, ms(5));
    }
}

/// Waits until the prompt is open (the core's question reached the UI thread).
fn prompt_open(vcx: &mut VisualTestContext) {
    for _ in 0..200 {
        settle(vcx, ms(5));
        if layer_ids(vcx) == ["password-prompt"] {
            return;
        }
    }
    panic!("the password prompt did not open");
}

#[gpui::test]
fn interaction_round_trips_from_the_core_runtime(cx: &mut TestAppContext) {
    let vcx = shell_with_connections(cx, "en");
    let ix = vcx.cx.read(interaction);
    let answer = spawn_on_core(vcx, ix, 1);
    prompt_open(vcx);
    assert_eq!(
        text_of(vcx, "password-body"),
        "Enter the password for deploy. It's kept only if the connection works."
    );
    assert!(try_text_of(vcx, "password-attempt").is_none());
    vcx.simulate_input("pw");
    vcx.simulate_keystrokes("enter");
    assert!(matches!(block(vcx, answer), Answer::Password(p) if p.expose_secret() == "pw"));
    assert!(layer_ids(vcx).is_empty());
}

#[gpui::test]
fn a_cancelled_prompt_answers_cancelled(cx: &mut TestAppContext) {
    let vcx = shell_with_connections(cx, "en");
    let ix = vcx.cx.read(interaction);
    let answer = spawn_on_core(vcx, ix, 2);
    prompt_open(vcx);
    assert_eq!(text_of(vcx, "password-attempt"), "Attempt 2 of 3");
    // The focused field swallows Escape and reports it (P16): the prompt cancels.
    vcx.simulate_keystrokes("escape");
    assert!(matches!(block(vcx, answer), Answer::Cancelled));
    assert!(layer_ids(vcx).is_empty());
}

// ---- the shots fixtures (R28, P19) -------------------------------------------------------------

/// Every fixture opens its surface over a `ConnectionsBoot::for_tests` shell.
#[cfg(feature = "shots")]
#[gpui::test]
fn the_shots_fixtures_open_their_surfaces(cx: &mut TestAppContext) {
    use hacivat_app::connections::{error_view, prod_prompt, test_strip};
    type Fixture = fn(&mut Window, &mut gpui::App);
    let fixtures: [(Fixture, &str, &str); 9] = [
        (test_strip::running_fixture, "test-strip", "test-clock"),
        (test_strip::ok_fixture, "test-strip", "diag-latency-value"),
        (test_strip::failed_fixture, "test-strip", "error-title"),
        (error_view::auth_fixture, "connection-error", "error-title"),
        (
            error_view::unknown_db_fixture,
            "connection-error",
            "error-title",
        ),
        (
            error_view::unsupported_fixture,
            "connection-error",
            "error-title",
        ),
        (
            error_view::tls_fixture,
            "connection-error",
            "fix-require-note",
        ),
        (
            prod_prompt::preselected_fixture,
            "production-prompt",
            "prod-suggest",
        ),
        (prod_prompt::unset_fixture, "production-prompt", "prod-body"),
    ];
    for (fixture, layer, text) in fixtures {
        let vcx = shell_with_connections(cx, "en");
        in_window(vcx, fixture);
        vcx.run_until_parked();
        assert_eq!(layer_ids(vcx), [layer]);
        assert!(
            try_text_of(vcx, text).is_some(),
            "{layer}: `{text}` is drawn"
        );
    }
}

/// Enter down and up, as a keyboard sends it: GPUI turns a clean down-and-up on a focused
/// button into a click.
fn press_enter(vcx: &mut VisualTestContext) {
    vcx.simulate_keystrokes("enter");
    vcx.simulate_event(gpui::KeyUpEvent {
        keystroke: gpui::Keystroke::parse("enter").expect("a keystroke"),
    });
    vcx.run_until_parked();
}

// ---- fix round 1 ---------------------------------------------------------------------------------

/// The ID of the dialog's focused control.
fn focused_control(vcx: &mut VisualTestContext) -> Option<String> {
    vcx.run_until_parked();
    let v = view(vcx);
    in_window(vcx, |window, cx| v.read(cx).focused_control(window, cx)).map(|s| s.to_string())
}

#[gpui::test]
fn the_initial_focus_is_safe_for_every_class(cx: &mut TestAppContext) {
    let mut hostname = tls_untrusted("db-qa.acme.io");
    hostname.class = ErrClass::TlsHostname;
    let cases: Vec<(ConnectError, &str)> = vec![
        (auth_invalid_attempts("deploy", 1), "fix-reenter"),
        (
            unknown_db("orders_v2", ["orders", "orders_archive", "postgres"]),
            "fix-use-0",
        ),
        // Never "Encrypt without verifying" (R3) or "Connect anyway" (PO Q3).
        (tls_untrusted("db-qa.acme.io"), "fix-edit"),
        (hostname, "fix-edit"),
        (unsupported("9.6.24"), "fix-edit"),
        (
            error_of(ErrClass::NetworkDns, Some(TrailStep::Resolve)),
            "fix-retry",
        ),
        (error_of(ErrClass::ConfigInvalid, None), "fix-edit"),
    ];
    for (error, expected) in cases {
        let class = error.class;
        // Each case is a new shell with its own saved profile.
        cx.update(|cx| cx.default_global::<Kept>().profile = None);
        let vcx = error_dialog(cx, error, ok_fake());
        assert_eq!(focused_control(vcx).as_deref(), Some(expected), "{class:?}");
        // A reflexive Enter runs exactly the focused fix, and never weakens the connection.
        press_enter(vcx);
        settle(vcx, ms(50));
        let p = profile(vcx);
        assert_eq!(p.tls.mode, TlsMode::Auto, "{class:?}");
        assert!(!p.allow_unsupported_server, "{class:?}");
        let calls = fake(vcx).connect_calls();
        match expected {
            "fix-reenter" => {
                // The field opened; nothing was sent yet.
                assert!(view(vcx).read_with(vcx, |v, _| v.reentering()), "{class:?}");
                assert_eq!(calls, 0, "{class:?}");
                assert_eq!(layer_ids(vcx), ["connection-error"], "{class:?}");
            }
            "fix-use-0" => {
                assert_eq!(p.database.as_deref(), Some("orders"), "{class:?}");
                let sent = fake(vcx).last_request().expect("a connect");
                assert_eq!(sent.database.as_deref(), Some("orders"), "{class:?}");
                assert!(layer_ids(vcx).is_empty(), "{class:?}: connected, closed");
            }
            "fix-retry" => {
                assert_eq!(calls, 1, "{class:?}");
                assert_eq!(p.database.as_deref(), Some("orders_v2"), "{class:?}");
                assert!(layer_ids(vcx).is_empty(), "{class:?}: connected, closed");
            }
            "fix-edit" => {
                // Edit connection closes the dialog (for the connection dialog); no connect,
                // no change to the profile.
                assert_eq!(calls, 0, "{class:?}");
                assert_eq!(p.database.as_deref(), Some("orders_v2"), "{class:?}");
                // The error dialog closed; with Task 17 merged, Edit connection opens the real
                // connection dialog in its place.
                assert!(
                    !layer_ids(vcx).contains(&"connection-error"),
                    "{class:?}: the error dialog closed"
                );
                assert_eq!(
                    layer_ids(vcx),
                    ["connection-dialog"],
                    "{class:?}: the editor opened"
                );
            }
            other => panic!("unexpected focus {other}"),
        }
    }
}

#[gpui::test]
fn a_read_only_store_moves_the_initial_focus_off_disabled_fixes(cx: &mut TestAppContext) {
    let first = shell_with_fake(cx, "en", ok_fake());
    let id = saved_profile(first, AuthProviderId::None);
    let text = std::fs::read_to_string(paths(first).connections_toml()).expect("the file");
    let vcx = shell_with_connections_file(cx, "en", &text.replace("version = 1", "version = 2"));
    let view = in_window(vcx, |window, cx| {
        open_error_dialog_view(
            id,
            Arc::new(unknown_db("orders_v2", ["orders", "b", "c"])),
            window,
            cx,
        )
    })
    .expect("the dialog");
    vcx.run_until_parked();
    let focused = in_window(vcx, |window, cx| view.read(cx).focused_control(window, cx));
    assert_eq!(focused.as_deref(), Some("fix-edit"));
}

/// The re-entry's Connect button as announced.
fn connect_button(vcx: &mut VisualTestContext) -> A11yDecl {
    decls(vcx)
        .into_iter()
        .find(|d| d.role == Some(Role::Button) && d.name.as_ref() == "Connect")
        .expect("the Connect button")
}

#[gpui::test]
fn an_empty_reentry_sends_nothing(cx: &mut TestAppContext) {
    let vcx = error_dialog(cx, auth_invalid_attempts("deploy", 1), fake_always_28p01());
    click(vcx, "fix-reenter");
    assert!(
        connect_button(vcx).disabled,
        "Connect is off while the field is empty"
    );
    vcx.simulate_keystrokes("enter"); // Enter in the empty field
    click(vcx, "fix-connect");
    settle(vcx, ms(30));
    assert_eq!(fake(vcx).connect_calls(), 0);
    assert_eq!(attempt_label(vcx), "Attempt 2 of 3");
    type_password(vcx, "x");
    assert!(!connect_button(vcx).disabled);
}

#[gpui::test]
fn try_again_starts_a_new_gesture(cx: &mut TestAppContext) {
    // A timeout after 2 rejections; Try again uses the stored password, which the server
    // rejects once: the count restarts (controller ruling), so re-entry offers attempt 2.
    let vcx = shell_with_fake(cx, "en", fake_always_28p01());
    saved_profile_with(vcx, AuthProviderId::Password, Some("hv_test_pw"));
    let mut timeout = error_of(ErrClass::NetworkTimeout, Some(TrailStep::Connect));
    timeout.attempts = 2;
    open_dialog_over(vcx, timeout);
    assert_eq!(view(vcx).read_with(vcx, |v, _| v.rejections()), 2);
    click(vcx, "fix-retry");
    for _ in 0..50 {
        settle(vcx, ms(10));
        if view(vcx).read_with(vcx, |v, _| v.error().class) == ErrClass::AuthInvalid {
            break;
        }
    }
    assert_eq!(
        view(vcx).read_with(vcx, |v, _| (v.error().class, v.rejections())),
        (ErrClass::AuthInvalid, 1)
    );
    click(vcx, "fix-reenter");
    assert_eq!(attempt_label(vcx), "Attempt 2 of 3");
}

#[gpui::test]
fn a_fix_on_a_connected_profile_never_sticks_pending(cx: &mut TestAppContext) {
    let vcx = shell_with_fake(cx, "en", ok_fake());
    let id = saved_profile(vcx, AuthProviderId::None);
    vcx.cx.update(|cx| ConnectionsModel::connect(cx, id, None));
    for _ in 0..50 {
        settle(vcx, ms(10));
        if matches!(live_of(vcx), LiveState::Connected(_)) {
            break;
        }
    }
    assert!(matches!(live_of(vcx), LiveState::Connected(_)));
    open_dialog_over(
        vcx,
        error_of(ErrClass::NetworkDns, Some(TrailStep::Resolve)),
    );
    // The manager starts no second connect: there is nothing to wait for.
    click(vcx, "fix-retry");
    settle(vcx, ms(20));
    assert!(layer_ids(vcx).is_empty(), "the profile is connected");
    assert_eq!(fake(vcx).connect_calls(), 1);
}

#[gpui::test]
fn a_reentered_password_never_reaches_the_screen_or_the_clipboard(cx: &mut TestAppContext) {
    let vcx = error_dialog(cx, auth_invalid_attempts("deploy", 1), fake_always_28p01());
    click(vcx, "fix-reenter");
    type_password(vcx, "hv_test_pw");
    click(vcx, "fix-connect");
    settle(vcx, ms(50));
    assert_eq!(attempt_label(vcx), "Attempt 3 of 3");
    expand(vcx, "tech-details");
    click(vcx, "copy-details");
    let copied = clipboard(vcx);
    assert!(!copied.is_empty());
    assert!(!copied.contains("hv_test_pw"), "the copy holds no password");
    assert!(
        registry_frame(vcx)
            .iter()
            .all(|r| !r.text.contains("hv_test_pw")),
        "no drawn text holds the password"
    );
    let v = view(vcx);
    let shown = vcx.cx.read(|cx| format!("{:?}", v.read(cx).error()));
    assert!(!shown.contains("hv_test_pw"), "the error holds no password");
    let sent = fake(vcx).last_request().expect("a connect");
    assert!(
        sent.password.as_deref() == Some("hv_test_pw"),
        "the password slot holds the re-entered password"
    );
    assert!(
        !format!("{sent:?}").contains("hv_test_pw"),
        "the request's Debug redacts it"
    );
}

#[gpui::test]
fn an_orphaned_password_prompt_closes(cx: &mut TestAppContext) {
    let vcx = shell_with_connections(cx, "en");
    let Interaction::Interactive(port) = vcx.cx.read(interaction) else {
        panic!("the app's interaction is interactive");
    };
    let rt = vcx.cx.read(|cx| cx.global::<ConnectionsModel>().runtime());
    let asking = rt.spawn(async move {
        port.ask(Question::Password {
            profile: ProfileId::new(),
            name: "orders-prod".into(),
            user: "deploy".into(),
            attempt: 1,
        })
        .await
    });
    prompt_open(vcx);
    // The connect that asked was cancelled: its future is dropped.
    asking.abort();
    for _ in 0..100 {
        settle(vcx, ms(5));
        if layer_ids(vcx).is_empty() {
            break;
        }
    }
    assert!(layer_ids(vcx).is_empty(), "the orphaned prompt closed");
}

/// The strip's status region's accessible name.
fn status_name(vcx: &mut VisualTestContext) -> String {
    decls(vcx)
        .into_iter()
        .find(|d| d.role == Some(Role::Status))
        .map(|d| strip_marks(&d.name))
        .expect("the strip's status region")
}

#[gpui::test]
fn the_strip_announces_its_result(cx: &mut TestAppContext) {
    let vcx = strip_fixture(
        cx,
        "en",
        ok_report(ms(54), ms(12), "PostgreSQL 16.4", tls13_verified()),
    );
    assert_eq!(status_name(vcx), "Connected in 54 ms");
    let mut failed = ok_report(ms(54), ms(12), "PostgreSQL 16.4", tls13_verified());
    failed.ok = false;
    failed.error = Some(auth_invalid_attempts("deploy", 1));
    let s = strip(vcx);
    vcx.cx.update(|cx| {
        s.update(cx, |s, cx| {
            s.show(TestStripState::Done(Arc::new(failed)), cx)
        })
    });
    assert_eq!(
        status_name(vcx),
        "The server rejected the password for deploy."
    );
}

#[gpui::test]
fn the_production_line_is_one_wrapping_sentence(cx: &mut TestAppContext) {
    let (vcx, _rx) = ask(
        cx,
        "tr",
        "orders-prod-us",
        "db-prod-us.acme.io",
        Some(("prod", HintSource::Host)),
    );
    assert_eq!(
        strip_marks(&text_of(vcx, "prod-c1")),
        "bu bağlantıda çalışırken başlık çubuğunda PROD kapsülünü gösterir,"
    );
    assert_eq!(text_of(vcx, "prod-word"), "PROD");
}

#[gpui::test]
fn a_fast_failure_replaces_the_error_shown(cx: &mut TestAppContext) {
    // Try again with a stored password the server rejects at once: whether the failure lands
    // before the dialog reads the state or after, the new error replaces the old one.
    let vcx = shell_with_fake(cx, "en", fake_always_28p01());
    saved_profile_with(vcx, AuthProviderId::Password, Some("hv_test_pw"));
    open_dialog_over(
        vcx,
        error_of(ErrClass::NetworkDns, Some(TrailStep::Resolve)),
    );
    let old = view(vcx).read_with(vcx, |v, _| v.error().clone());
    click(vcx, "fix-retry");
    for _ in 0..50 {
        settle(vcx, ms(10));
        if !Arc::ptr_eq(&view(vcx).read_with(vcx, |v, _| v.error().clone()), &old) {
            break;
        }
    }
    let new = view(vcx).read_with(vcx, |v, _| v.error().clone());
    assert!(!Arc::ptr_eq(&new, &old), "the new error is shown");
    assert_eq!(new.class, ErrClass::AuthInvalid);
    assert_eq!(view(vcx).read_with(vcx, |v, _| v.rejections()), 1);
    assert_eq!(
        error_title(vcx),
        "The server rejected the password for deploy."
    );
    // Its trail fails at Sign in, and nothing is pending: the fixes are live.
    assert!(
        new.trail
            .iter()
            .any(|t| t.step == TrailStep::SignIn && t.outcome == StepOutcome::Failed)
    );
    assert!(fix_buttons(vcx).iter().all(|b| !b.3), "no fix is disabled");
}

// ## m2a-finalfix

/// I2: a service imported with `sslmode=verify-full` is a Verify full profile; "Encrypt without
/// verifying" saves Require, and the next connect uses Require. No imported extra is left to
/// override the fix (the form, the fix and the connect agree).
#[gpui::test]
fn the_require_fix_wins_over_an_imported_sslmode(cx: &mut TestAppContext) {
    use hacivat_app::connections::import::{self, ImportSources};
    let vcx = shell_with_fake(cx, "en", ok_fake());
    let port = listening_port(vcx);
    let file = paths(vcx).data.join("pg_service.conf");
    std::fs::write(
        &file,
        format!("[orders-prod]\nhost=127.0.0.1\nport={port}\nuser=deploy\nsslmode=verify-full\n"),
    )
    .expect("the service file");
    vcx.cx.update(|cx| cx.set_global(ImportSources(vec![file])));
    in_window(vcx, import::open);
    settle(vcx, ms(100));
    let dialog = vcx.cx.read(import::open_view).expect("the import dialog");
    vcx.cx.update(|cx| dialog.update(cx, |v, cx| v.confirm(cx)));
    settle(vcx, ms(100));
    let id = vcx
        .cx
        .read(|cx| cx.global::<ConnectionsModel>().profiles()[0].id);
    vcx.cx
        .update(|cx| cx.default_global::<Kept>().profile = Some(id));
    assert_eq!(profile(vcx).tls.mode, TlsMode::VerifyFull);
    assert!(profile(vcx).extra.is_empty(), "{:?}", profile(vcx).extra);
    // No password file in a test shell: sign in without one (the TLS settings stay).
    let mut draft = ProfileDraft::edit((*profile(vcx)).clone());
    draft.profile.auth = AuthProviderId::None;
    let task = vcx.cx.update(|cx| ConnectionsModel::save(cx, draft));
    wait(vcx, task).expect("the profile saves");
    assert_eq!(profile(vcx).tls.mode, TlsMode::VerifyFull);

    open_dialog_over(vcx, tls_untrusted("127.0.0.1"));
    click(vcx, "fix-require");
    settle(vcx, ms(50));
    assert_eq!(profile(vcx).tls.mode, TlsMode::Require);
    let sent = fake(vcx).last_request().expect("a connect");
    assert_eq!(
        sent.tls.expect("TLS parameters").settings.mode,
        TlsMode::Require
    );
}

/// M4: a server message that is a connection string loses its password on the screen and in
/// the English copy (`hacivat_secrets::scrub_url`); the rest stays verbatim.
#[gpui::test]
fn a_server_message_never_shows_a_password(cx: &mut TestAppContext) {
    let vcx = error_dialog(
        cx,
        auth_invalid_with_server("28P01", "host=db.acme.io user=deploy password=s3cret-x"),
        ok_fake(),
    );
    expand(vcx, "tech-details");
    let row = tech_row(vcx, "Server message");
    assert_eq!(row, "host=db.acme.io user=deploy");
    click(vcx, "copy-details");
    let text = clipboard(vcx);
    assert!(!text.contains("s3cret"), "a password reached the clipboard");
    assert!(text.contains("Server message: host=db.acme.io user=deploy"));
}

/// N4 (round 2): a server message that only looks like one `key=value` segment to the URL
/// scrubber is never blanked; only a password-like word loses its value.
#[gpui::test]
fn a_server_message_is_never_blanked(cx: &mut TestAppContext) {
    let message = "duplicate key value violates unique constraint \"t_pkey\" password=s3cret-x";
    let vcx = error_dialog(cx, auth_invalid_with_server("23505", message), ok_fake());
    expand(vcx, "tech-details");
    assert_eq!(
        tech_row(vcx, "Server message"),
        "duplicate key value violates unique constraint \"t_pkey\" password=<removed>"
    );
    click(vcx, "copy-details");
    let text = clipboard(vcx);
    assert!(!text.contains("s3cret"), "a password reached the clipboard");
    assert!(text.contains("Server message: duplicate key value violates unique constraint"));
}
