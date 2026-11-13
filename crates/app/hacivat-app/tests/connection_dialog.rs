//! The connection dialog (Task 17; NOTES surfaces 1 and 2): size and focus, the keyboard path,
//! paste in every field, the URL row, Copy as URL, per-engine fields, retention, validation,
//! save, edit and the inline fixes.
//!
//! Core work (validation, the keyring probe, profile writes) runs on the core runtime, so tests
//! that save wait for it with `settle` (P22).

mod support;

use std::sync::Arc;
use std::time::Duration;

use gpui::{ClipboardItem, Entity, Focusable as _, Modifiers, TestAppContext, VisualTestContext};
use hacivat_app::connections::ConnectionsModel;
use hacivat_app::connections::dialog::{
    ConnectionDialog, DialogMode, open_connection_dialog, open_dialog,
};
use hacivat_app::connections::fixes::FixAction;
use hacivat_app::connections::test_strip::TestStripEvent;
use hacivat_net::TlsMode;
use hacivat_proto::{DbmsKind, EnvTag};
use hacivat_secrets::{
    MemoryBackend, Retention, SecretBackend, SecretError, SecretKey, SecretSlot, SecretString,
};
use hacivat_session::profile::{ProfileDraft, ProfileEndpoint};
use hacivat_ui::select::SelectEvent;
use hacivat_ui::text::TextSource;
use support::*;

// ---- opening ------------------------------------------------------------------------------------

/// A connections shell in `lang` with the New dialog open.
fn open_new(cx: &mut TestAppContext, lang: &str) -> &'static mut VisualTestContext {
    let vcx = shell_with_connections(cx, lang);
    open(vcx, DialogMode::New)
}

fn open(vcx: &'static mut VisualTestContext, mode: DialogMode) -> &'static mut VisualTestContext {
    in_window(vcx, |window, cx| open_connection_dialog(mode, window, cx));
    // The keyring probe runs on the core runtime (P36).
    settle(vcx, Duration::from_millis(20));
    vcx
}

/// A secret backend whose keyring is not usable (P36): the memory store, probing `false`.
struct NoKeyring(MemoryBackend);

impl SecretBackend for NoKeyring {
    fn get(&self, key: &SecretKey) -> Result<Option<SecretString>, SecretError> {
        self.0.get(key)
    }
    fn put(&self, key: &SecretKey, value: &SecretString) -> Result<(), SecretError> {
        self.0.put(key, value)
    }
    fn delete(&self, key: &SecretKey) -> Result<(), SecretError> {
        self.0.delete(key)
    }
    fn name(&self) -> &'static str {
        "no-keyring"
    }
    fn probe_available(&self) -> bool {
        false
    }
}

fn open_new_without_keyring(cx: &mut TestAppContext) -> &'static mut VisualTestContext {
    let vcx = shell_with_backend(cx, "en", None, Arc::new(NoKeyring(MemoryBackend::new())));
    let vcx = open(vcx, DialogMode::New);
    // The probe answer lands whenever the core's blocking pool gets to it (slower than the
    // fixed wait in `open` on Windows runners): wait for it, not for a fixed time.
    for _ in 0..250 {
        if remember_option_disabled(vcx, Retention::Forever) {
            return vcx;
        }
        settle(vcx, Duration::from_millis(10));
    }
    panic!("the keyring probe did not land");
}

/// A shell with a saved production profile `name` whose password is kept by `retention`, and
/// its Edit dialog open.
fn open_edit_saved(
    cx: &mut TestAppContext,
    name: &str,
    retention: Retention,
) -> &'static mut VisualTestContext {
    let vcx = shell_with_connections(cx, "en");
    let mut draft = ProfileDraft::new();
    draft.profile.name = name.to_string();
    draft.profile.environment = Some(EnvTag::Production);
    draft.profile.endpoint = ProfileEndpoint::Server {
        host: "db-prod.acme.io".to_string(),
        port: None,
    };
    draft.profile.retention = retention;
    draft.profile.user = Some("deploy".to_string());
    draft.profile.database = Some("orders".to_string());
    draft.password = Some(SecretString::from("hv_test_pw".to_string()));
    let id = draft.profile.id;
    vcx.cx
        .update(|cx| ConnectionsModel::save(cx, draft).detach());
    settle(vcx, Duration::from_millis(100));
    assert!(
        keyring_entry(vcx).is_some(),
        "the saved password is in the keyring"
    );
    open(vcx, DialogMode::Edit(id))
}

fn dialog(vcx: &VisualTestContext) -> Entity<ConnectionDialog> {
    in_window(vcx, |window, cx| open_dialog(window, cx)).expect("a connection dialog is open")
}

fn read<R>(vcx: &VisualTestContext, f: impl FnOnce(&ConnectionDialog, &gpui::App) -> R) -> R {
    let d = dialog(vcx);
    vcx.cx.read(|cx| f(d.read(cx), cx))
}

// ---- keys and fields ----------------------------------------------------------------------------

fn primary(key: &str) -> String {
    if cfg!(target_os = "macos") {
        format!("cmd-{key}")
    } else {
        format!("ctrl-{key}")
    }
}

fn primary_shift(key: &str) -> String {
    if cfg!(target_os = "macos") {
        format!("cmd-shift-{key}")
    } else {
        format!("ctrl-shift-{key}")
    }
}

fn keys(vcx: &mut VisualTestContext, k: &str) {
    vcx.simulate_keystrokes(k);
    vcx.run_until_parked();
}

fn focus_field(vcx: &VisualTestContext, name: &str) {
    let input = read(vcx, |d, _| d.input(name)).unwrap_or_else(|| panic!("no field {name}"));
    let handle = vcx.cx.read(|cx| input.focus_handle(cx));
    in_window(vcx, |window, cx| window.focus(&handle, cx));
    vcx.run_until_parked();
}

fn type_into(vcx: &mut VisualTestContext, name: &str, text: &str) {
    focus_field(vcx, name);
    vcx.simulate_input(text);
    vcx.run_until_parked();
}

/// Replaces field `name`'s text by typing.
fn set_field(vcx: &mut VisualTestContext, name: &str, text: &str) {
    focus_field(vcx, name);
    keys(vcx, &primary("a"));
    vcx.simulate_input(text);
    vcx.run_until_parked();
}

fn fill(vcx: &mut VisualTestContext, pairs: &[(&str, &str)]) {
    for (name, text) in pairs {
        type_into(vcx, name, text);
    }
}

fn paste_into(vcx: &mut VisualTestContext, name: &str, text: &str) {
    focus_field(vcx, name);
    vcx.cx
        .write_to_clipboard(ClipboardItem::new_string(text.to_string()));
    keys(vcx, &primary("v"));
}

fn field(vcx: &VisualTestContext, name: &str) -> String {
    read(vcx, |d, cx| d.field_value(name, cx)).unwrap_or_else(|| panic!("no field {name}"))
}

fn password_value(vcx: &VisualTestContext) -> String {
    field(vcx, "password")
}

fn url_row(vcx: &VisualTestContext) -> String {
    field(vcx, "url")
}

fn tinted(vcx: &VisualTestContext, name: &str) -> bool {
    read(vcx, |d, _| d.is_tinted(name))
}

fn engine(vcx: &VisualTestContext) -> DbmsKind {
    read(vcx, |d, _| d.profile().engine)
}

fn env_pressed(vcx: &VisualTestContext) -> Option<EnvTag> {
    read(vcx, |d, _| d.profile().environment)
}

fn remember(vcx: &VisualTestContext) -> Retention {
    read(vcx, |d, _| d.profile().retention)
}

fn draft_tls_mode(vcx: &VisualTestContext) -> TlsMode {
    read(vcx, |d, _| d.profile().tls.mode)
}

/// Whether the TLS mode Select offers `mode` disabled: the option list the Select itself holds.
fn tls_option_disabled(vcx: &VisualTestContext, mode: TlsMode) -> bool {
    let at = [
        TlsMode::Auto,
        TlsMode::Require,
        TlsMode::VerifyCa,
        TlsMode::VerifyFull,
        TlsMode::Disable,
    ]
    .iter()
    .position(|m| *m == mode)
    .expect("a mode of the select");
    let select = read(vcx, |d, _| d.tls_select());
    vcx.cx.read(|cx| select.read(cx).options()[at].disabled)
}

/// Whether the Remember Select offers `retention` disabled (its own option list).
fn remember_option_disabled(vcx: &VisualTestContext, retention: Retention) -> bool {
    let at = Retention::ALL
        .iter()
        .position(|r| *r == retention)
        .expect("a retention");
    let select = read(vcx, |d, _| d.remember_select());
    vcx.cx.read(|cx| select.read(cx).options()[at].disabled)
}

fn focused_field(vcx: &VisualTestContext) -> String {
    let d = dialog(vcx);
    in_window(vcx, |window, cx| d.read(cx).focused_field(window, cx))
        .unwrap_or("<outside>")
        .to_string()
}

/// Focuses `start`, then presses Tab until the focus comes back to it (or 40 presses).
fn tab_walk(vcx: &mut VisualTestContext, start: &str) -> Vec<String> {
    focus_field(vcx, start);
    let mut seen = vec![focused_field(vcx)];
    for _ in 0..40 {
        keys(vcx, "tab");
        let now = focused_field(vcx);
        seen.push(now.clone());
        if now == start {
            break;
        }
    }
    seen
}

/// Clicks the center of the element with debug selector `selector`.
fn click_selector(vcx: &mut VisualTestContext, selector: &str) {
    vcx.run_until_parked();
    let selector: &'static str = Box::leak(selector.to_string().into_boxed_str());
    let bounds = vcx
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("no element `{selector}` drawn"));
    vcx.simulate_click(bounds.center(), Modifiers::none());
    vcx.run_until_parked();
}

/// Clicks the button with ID `id`.
fn click(vcx: &mut VisualTestContext, id: &str) {
    click_selector(vcx, &format!("button-{id}"));
}

fn choose_engine(vcx: &mut VisualTestContext, kind: DbmsKind) {
    let at = [
        DbmsKind::Postgres,
        DbmsKind::MySql,
        DbmsKind::MariaDb,
        DbmsKind::Sqlite,
    ]
    .iter()
    .position(|k| *k == kind)
    .expect("an engine of the form");
    click_selector(vcx, &format!("segmented-conn-engine-{at}"));
}

fn has_field(vcx: &mut VisualTestContext, name: &str) -> bool {
    vcx.run_until_parked();
    let selector: &'static str = Box::leak(format!("conn-field-{name}").into_boxed_str());
    vcx.debug_bounds(selector).is_some()
}

fn layer_width(vcx: &mut VisualTestContext) -> f32 {
    vcx.run_until_parked();
    f32::from(
        vcx.debug_bounds("form-dialog")
            .expect("the form dialog is drawn")
            .size
            .width,
    )
}

fn emit_strip_fix(vcx: &mut VisualTestContext, fix: FixAction) {
    let strip = read(vcx, |d, _| d.strip());
    in_window(vcx, |window, cx| {
        let _ = window;
        strip.update(cx, |_, cx| cx.emit(TestStripEvent::Fix(fix)));
    });
    vcx.run_until_parked();
}

fn set_remember(vcx: &mut VisualTestContext, retention: Retention) {
    let select = read(vcx, |d, _| d.remember_select());
    let at = Retention::ALL
        .iter()
        .position(|r| *r == retention)
        .expect("a retention");
    vcx.cx.update(|cx| {
        select.update(cx, |s, cx| {
            s.set_selected(at, cx);
            cx.emit(SelectEvent::Changed(at));
        })
    });
    vcx.run_until_parked();
}

// ---- what the frame shows -----------------------------------------------------------------------

/// The text of the label with ID `id`, if drawn.
fn label(vcx: &VisualTestContext, id: &str) -> Option<String> {
    let needle = format!("#{id}");
    registry_frame(vcx)
        .into_iter()
        .find(|r| r.path.ends_with(&needle))
        .map(|r| r.text.to_string())
}

fn dialog_title(vcx: &VisualTestContext) -> String {
    label(vcx, "conn-dialog-title").expect("the dialog title is drawn")
}

fn dialog_subtitle(vcx: &VisualTestContext) -> String {
    label(vcx, "conn-dialog-subtitle").expect("the dialog subtitle is drawn")
}

fn env_hint(vcx: &VisualTestContext) -> String {
    label(vcx, "env-hint").expect("an environment hint is drawn")
}

fn issue_under(vcx: &VisualTestContext, name: &str) -> Option<String> {
    label(vcx, &format!("issue-{name}"))
}

/// The inline notice's lines in order; a parameter row is its key, a space and the rest.
fn notice_lines(vcx: &VisualTestContext) -> Vec<String> {
    let texts: Vec<_> = registry_frame(vcx)
        .into_iter()
        .filter(|r| r.path.contains("hacivat-ui/src/notice.rs:"))
        .collect();
    let mut out = Vec::new();
    let mut it = texts.into_iter();
    while let Some(r) = it.next() {
        if r.source == TextSource::Data {
            let rest = it.next().map(|n| n.text.to_string()).unwrap_or_default();
            out.push(format!("{} {}", r.text, rest));
        } else {
            out.push(r.text.to_string());
        }
    }
    out
}

/// The line of `path`'s file (`…/checkbox.rs:141:21` gives 141).
fn line_of(path: &str) -> u32 {
    path.rsplit(':')
        .nth(1)
        .and_then(|l| l.parse().ok())
        .unwrap_or(0)
}

/// The checkbox labels drawn (without their sub-labels), in paint order.
fn checkbox_labels(vcx: &VisualTestContext) -> Vec<String> {
    let texts: Vec<_> = registry_frame(vcx)
        .into_iter()
        .filter(|r| r.path.contains("hacivat-ui/src/checkbox.rs:"))
        .collect();
    let first = texts.iter().map(|r| line_of(&r.path)).min().unwrap_or(0);
    texts
        .into_iter()
        .filter(|r| line_of(&r.path) == first)
        .map(|r| r.text.to_string())
        .collect()
}

/// The labels of the Authentication segments (the translated segment labels that are not
/// environment names).
fn auth_segments(vcx: &VisualTestContext) -> Vec<String> {
    let envs = ["local", "dev", "test", "staging", "production"];
    registry_frame(vcx)
        .into_iter()
        .filter(|r| r.path.contains("hacivat-ui/src/segmented.rs:"))
        .filter(|r| r.source == TextSource::Fluent && !envs.contains(&r.text.as_ref()))
        .map(|r| r.text.to_string())
        .collect()
}

fn clipboard(vcx: &VisualTestContext) -> String {
    vcx.cx
        .read_from_clipboard()
        .and_then(|c| c.text())
        .unwrap_or_default()
}

fn profile_names(vcx: &VisualTestContext) -> Vec<String> {
    vcx.cx.read(|cx| {
        cx.global::<ConnectionsModel>()
            .profiles()
            .iter()
            .map(|p| p.name.clone())
            .collect()
    })
}

/// The stored password of the first profile, straight from the backend.
fn keyring_entry(vcx: &VisualTestContext) -> Option<()> {
    let id = vcx.cx.read(|cx| {
        cx.global::<ConnectionsModel>()
            .profiles()
            .first()
            .map(|p| p.id)
            .expect("a saved profile")
    });
    let key = SecretKey {
        profile: id,
        slot: SecretSlot::DbPassword,
    };
    secret_backend(vcx)
        .get(&key)
        .expect("the backend answers")
        .map(|_| ())
}

/// Waits (real time, at most 5 s) until a save in flight is done: validation (which loads the
/// platform roots for TLS) and the profile write run on the core runtime.
fn wait(vcx: &VisualTestContext) {
    for _ in 0..250 {
        settle(vcx, Duration::from_millis(20));
        let busy = in_window(vcx, |window, cx| {
            open_dialog(window, cx).is_some_and(|d| d.read(cx).is_saving())
        });
        if !busy {
            return;
        }
    }
    panic!("the save did not finish");
}

// ---- the brief's tests --------------------------------------------------------------------------

#[gpui::test]
fn new_dialog_is_640_and_focuses_host(cx: &mut TestAppContext) {
    let vcx = open_new(cx, "en");
    assert_eq!(layer_ids(vcx), ["connection-dialog"]);
    assert_eq!(layer_width(vcx), 640.0);
    assert_eq!(focused_field(vcx), "host");
}

#[gpui::test]
fn tab_order_matches_the_mock_without_ssh(cx: &mut TestAppContext) {
    let vcx = open_new(cx, "en");
    assert_eq!(
        tab_walk(vcx, "url"),
        [
            "url",
            "engine",
            "name",
            "environment",
            "read-only",
            "host",
            "port",
            "database",
            "user",
            "auth",
            "password",
            "remember",
            "tls",
            "advanced",
            "test",
            "cancel",
            "save",
            "save-connect",
            "url"
        ]
    );
}

#[gpui::test]
fn paste_url_into_host_fills_tints_and_explains(cx: &mut TestAppContext) {
    let vcx = open_new(cx, "en");
    paste_into(
        vcx,
        "host",
        "postgres://deploy:s3cret@db-prod-us.acme.io:5433/orders?sslmode=verify-full&application_name=hv&foo=bar",
    );
    assert_eq!(field(vcx, "host"), "db-prod-us.acme.io");
    assert_eq!(field(vcx, "port"), "5433");
    assert!(tinted(vcx, "host"));
    assert_eq!(
        notice_lines(vcx),
        [
            "Filled from the pasted URL.",
            "The password was taken out of the URL. It goes to the Keychain when you save.",
            "sslmode=verify-full became Encryption: Verify full",
            "application_name=hv became Application name",
            "foo=bar kept as an extra property"
        ]
    );
    vcx.executor().advance_clock(Duration::from_millis(1840));
    vcx.run_until_parked();
    assert!(!tinted(vcx, "host"));
    click(vcx, "notice-undo");
    assert_eq!(field(vcx, "host"), "");
}

#[gpui::test]
fn paste_into_password_field_is_literal(cx: &mut TestAppContext) {
    let vcx = open_new(cx, "en");
    paste_into(vcx, "password", "postgres://x@y/z");
    assert_eq!(password_value(vcx), "postgres://x@y/z");
    assert_eq!(field(vcx, "host"), "");
    assert!(notice_lines(vcx).is_empty());
}

#[gpui::test]
fn non_url_in_url_field_changes_nothing(cx: &mut TestAppContext) {
    let vcx = open_new(cx, "en");
    paste_into(vcx, "url", "hello world");
    assert_eq!(
        notice_lines(vcx),
        [
            "Hacivat can't read this as a connection URL. Nothing was changed.",
            "It reads postgres://, mysql://, mariadb://, jdbc:… URLs, key=value strings and SQLite file paths."
        ]
    );
}

#[gpui::test]
fn other_engine_asks_before_switching(cx: &mut TestAppContext) {
    let vcx = open_new(cx, "en");
    type_into(vcx, "host", "h");
    paste_into(vcx, "url", "mysql://root@m/db");
    assert_eq!(
        notice_lines(vcx)[0],
        "This is a MySQL URL, and the form is set to PostgreSQL."
    );
    assert_eq!(engine(vcx), DbmsKind::Postgres);
    click(vcx, "notice-switch");
    assert_eq!(
        (engine(vcx), field(vcx, "host")),
        (DbmsKind::MySql, "m".into())
    );
}

#[gpui::test]
fn untouched_form_switches_and_says_so(cx: &mut TestAppContext) {
    let vcx = open_new(cx, "en");
    paste_into(vcx, "url", "mysql://root@m/db");
    assert_eq!(engine(vcx), DbmsKind::MySql);
    assert!(notice_lines(vcx).contains(&"Engine set to MySQL from the URL.".to_string()));
}

#[gpui::test]
fn url_row_mirrors_and_masks(cx: &mut TestAppContext) {
    let vcx = open_new(cx, "en");
    fill(
        vcx,
        &[
            ("host", "db"),
            ("database", "app"),
            ("user", "u"),
            ("password", "pw"),
        ],
    );
    assert_eq!(url_row(vcx), "postgresql://u:****@db/app");
}

#[gpui::test]
fn copy_url_never_includes_password(cx: &mut TestAppContext) {
    let vcx = open_new(cx, "en");
    fill(
        vcx,
        &[
            ("host", "db"),
            ("database", "app"),
            ("user", "u"),
            ("password", "pw"),
        ],
    );
    keys(vcx, &primary_shift("c"));
    assert_eq!(clipboard(vcx), "postgresql://u@db/app");
    // NOTES §2 (P31).
    assert_eq!(
        toast_texts(vcx),
        [
            "Copied as URL",
            "postgresql://u@db/app",
            "The password isn't included."
        ]
    );
}

#[gpui::test]
fn sqlite_swaps_server_for_file(cx: &mut TestAppContext) {
    let vcx = open_new(cx, "en");
    choose_engine(vcx, DbmsKind::Sqlite);
    assert!(has_field(vcx, "file") && !has_field(vcx, "host"));
    assert_eq!(
        checkbox_labels(vcx),
        [
            "Read-only",
            "Open the file read-only",
            "Create the file if it doesn't exist"
        ]
    );
}

#[gpui::test]
fn mysql_hides_pgpass(cx: &mut TestAppContext) {
    let vcx = open_new(cx, "en");
    choose_engine(vcx, DbmsKind::MySql);
    assert_eq!(
        auth_segments(vcx),
        ["Password", "Client certificate", "None"]
    );
}

#[gpui::test]
fn mysql_disables_verify_ca(cx: &mut TestAppContext) {
    let vcx = open_new(cx, "en");
    // P11.
    assert!(!tls_option_disabled(vcx, TlsMode::VerifyCa));
    choose_engine(vcx, DbmsKind::MySql);
    assert!(tls_option_disabled(vcx, TlsMode::VerifyCa));
    choose_engine(vcx, DbmsKind::MariaDb);
    assert!(tls_option_disabled(vcx, TlsMode::VerifyCa));
}

#[gpui::test]
fn escape_in_a_field_closes_the_dialog(cx: &mut TestAppContext) {
    let vcx = open_new(cx, "en");
    // The focus is in Host (P16).
    type_into(vcx, "host", "h");
    keys(vcx, "escape");
    assert!(layer_ids(vcx).is_empty());
}

#[gpui::test]
fn inline_fix_reenter_clears_and_focuses_password(cx: &mut TestAppContext) {
    let vcx = open_new(cx, "en");
    // P40.
    fill(vcx, &[("host", "db"), ("password", "pw")]);
    emit_strip_fix(vcx, FixAction::ReenterPassword);
    assert_eq!(password_value(vcx), "");
    assert_eq!(focused_field(vcx), "password");
    emit_strip_fix(vcx, FixAction::EncryptWithoutVerifying);
    assert_eq!(draft_tls_mode(vcx), TlsMode::Require);
}

#[gpui::test]
fn environment_hint_rules(cx: &mut TestAppContext) {
    let vcx = open_new(cx, "en");
    type_into(vcx, "host", "db.acme.io");
    assert_eq!(
        env_hint(vcx),
        "Asked when you save, because this host isn't on this Mac."
    );
    set_field(vcx, "host", "127.0.0.1");
    assert_eq!(env_hint(vcx), "Set to local: the server runs on this Mac.");
    assert_eq!(env_pressed(vcx), Some(EnvTag::Local));
}

#[gpui::test]
fn remember_defaults_to_forever_and_disables_without_keyring(cx: &mut TestAppContext) {
    assert_eq!(remember(open_new(cx, "en")), Retention::Forever);
    let vcx = open_new_without_keyring(cx);
    assert!(remember_option_disabled(vcx, Retention::Forever));
    assert_eq!(remember(vcx), Retention::UntilRestart);
}

#[gpui::test]
fn validation_issues_block_save_inline(cx: &mut TestAppContext) {
    let vcx = open_new(cx, "en");
    keys(vcx, &primary("s"));
    wait(vcx);
    assert_eq!(issue_under(vcx, "host"), Some("Enter a host.".into()));
    assert_eq!(layer_ids(vcx), ["connection-dialog"]);
}

#[gpui::test]
fn save_local_profile_without_prompt(cx: &mut TestAppContext) {
    let vcx = open_new(cx, "en");
    fill(vcx, &[("host", "localhost"), ("database", "shop")]);
    keys(vcx, &primary("s"));
    wait(vcx);
    assert!(layer_ids(vcx).is_empty());
    assert_eq!(profile_names(vcx), ["shop-local"]);
    assert_eq!(toast_texts(vcx), ["Saved shop-local"]);
}

#[gpui::test]
fn remote_save_waits_for_an_environment(cx: &mut TestAppContext) {
    let vcx = open_new(cx, "en");
    fill(vcx, &[("host", "db.acme.io")]);
    keys(vcx, &primary("s"));
    // The save is held on the production-tag prompt (T20), which waits for the user, so the
    // dialog stays "saving" on purpose: wait for the prompt layer instead of the save.
    for _ in 0..250 {
        settle(vcx, Duration::from_millis(20));
        if layer_ids(vcx).contains(&"production-prompt") {
            break;
        }
    }
    assert!(layer_ids(vcx).contains(&"production-prompt"));
    assert!(profile_names(vcx).is_empty());
    assert!(layer_ids(vcx).contains(&"connection-dialog"));
}

#[gpui::test]
fn edit_mode_titles_and_retention_change(cx: &mut TestAppContext) {
    let vcx = open_edit_saved(cx, "orders-prod", Retention::Forever);
    assert_eq!(dialog_title(vcx), "Edit orders-prod");
    assert_eq!(
        dialog_subtitle(vcx),
        "Saved · changes apply on the next connect"
    );
    set_remember(vcx, Retention::Never);
    keys(vcx, &primary("s"));
    wait(vcx);
    assert!(keyring_entry(vcx).is_none());
}

#[gpui::test]
fn language_switch_keeps_typed_values(cx: &mut TestAppContext) {
    let vcx = open_new(cx, "en");
    type_into(vcx, "host", "db.acme.io");
    switch_to(vcx, "tr");
    vcx.run_until_parked();
    assert_eq!(field(vcx, "host"), "db.acme.io");
    assert_eq!(dialog_title(vcx), "Yeni bağlantı");
}

// ---- beyond the brief ---------------------------------------------------------------------------

#[gpui::test]
fn shift_tab_from_the_url_wraps_to_save_and_connect(cx: &mut TestAppContext) {
    let vcx = open_new(cx, "en");
    focus_field(vcx, "url");
    keys(vcx, "shift-tab");
    assert_eq!(focused_field(vcx), "save-connect");
    keys(vcx, "tab");
    assert_eq!(focused_field(vcx), "url");
}

#[gpui::test]
fn a_picked_file_fills_its_field_and_joins_the_approved_roots(cx: &mut TestAppContext) {
    let vcx = open_new(cx, "en");
    let path = std::path::PathBuf::from("/opt/certs/company-ca.pem");
    let d = dialog(vcx);
    in_window(vcx, |window, cx| {
        d.update(cx, |d, cx| {
            d.picked(
                hacivat_app::connections::dialog::form::Field::Ca,
                path.clone(),
                window,
                cx,
            )
        })
    });
    vcx.run_until_parked();
    assert_eq!(field(vcx, "ca"), "/opt/certs/company-ca.pem");
    assert_eq!(
        read(vcx, |d, _| d.profile().tls.ca_file.clone()),
        Some(path.clone())
    );
    let roots = vcx.cx.read(|cx| {
        cx.global::<hacivat_app::connections::dialog::DialogEnv>()
            .approved_roots
            .list()
    });
    assert!(roots.contains(&path));
}

#[gpui::test]
fn keep_engine_leaves_the_form_alone(cx: &mut TestAppContext) {
    let vcx = open_new(cx, "en");
    type_into(vcx, "host", "h");
    paste_into(vcx, "url", "mysql://root@m/db");
    click(vcx, "notice-keep");
    assert!(notice_lines(vcx).is_empty());
    assert_eq!(
        (engine(vcx), field(vcx, "host")),
        (DbmsKind::Postgres, "h".into())
    );
}

#[gpui::test]
fn save_and_connect_saves_then_connects(cx: &mut TestAppContext) {
    let vcx = open_new(cx, "en");
    fill(vcx, &[("host", "localhost"), ("database", "shop")]);
    keys(vcx, &primary("enter"));
    wait(vcx);
    assert!(layer_ids(vcx).is_empty());
    let (names, current, first) = vcx.cx.read(|cx| {
        let m = cx.global::<ConnectionsModel>();
        let names: Vec<String> = m.profiles().iter().map(|p| p.name.clone()).collect();
        (names, m.current(), m.profiles().first().map(|p| p.id))
    });
    assert_eq!(names, ["shop-local"]);
    assert_eq!(current, first, "the saved profile is the one connecting");
}

#[gpui::test]
fn a_read_only_store_explains_and_saves_nothing(cx: &mut TestAppContext) {
    let vcx = shell_with_connections_file(cx, "en", "version = 2\n");
    let vcx = open(vcx, DialogMode::New);
    let why = "Connections can't be saved now, because the connections file can't be written.";
    // Explained from the start, before any save.
    assert!(
        notice_lines(vcx).contains(&why.to_string()),
        "{:?}",
        notice_lines(vcx)
    );
    // The fields take no input.
    type_into(vcx, "host", "localhost");
    assert_eq!(field(vcx, "host"), "");
    assert!(read(vcx, |d, cx| d
        .input("host")
        .unwrap()
        .read(cx)
        .is_read_only()));
    // Save does nothing but keep the explanation.
    click(vcx, "conn-save");
    keys(vcx, &primary("s"));
    wait(vcx);
    assert!(profile_names(vcx).is_empty());
    assert_eq!(layer_ids(vcx), ["connection-dialog"]);
    assert!(notice_lines(vcx).contains(&why.to_string()));
}

#[gpui::test]
fn a_pasted_password_never_reaches_the_url_row_or_the_clipboard(cx: &mut TestAppContext) {
    let vcx = open_new(cx, "en");
    paste_into(vcx, "host", "postgres://deploy:s3cret@db.acme.io/orders");
    assert_eq!(url_row(vcx), "postgresql://deploy:****@db.acme.io/orders");
    keys(vcx, &primary_shift("c"));
    assert_eq!(clipboard(vcx), "postgresql://deploy@db.acme.io/orders");
    let frame: Vec<String> = registry_frame(vcx)
        .into_iter()
        .map(|r| r.text.to_string())
        .collect();
    assert!(frame.iter().all(|t| !t.contains("s3cret")));
}

// ---- fix round 1 ---------------------------------------------------------------------------

/// The URL row with its port set to `port` (the row's own text, edited).
fn edit_row_port(vcx: &mut VisualTestContext, host: &str, port: &str) {
    let row = url_row(vcx);
    let edited = row.replacen(&format!("@{host}/"), &format!("@{host}:{port}/"), 1);
    assert_ne!(row, edited, "the row names {host}");
    set_field(vcx, "url", &edited);
    keys(vcx, "enter");
}

#[gpui::test]
fn editing_the_url_row_keeps_the_pasted_password(cx: &mut TestAppContext) {
    let vcx = open_new(cx, "en");
    paste_into(vcx, "host", "postgres://deploy:s3cret@db.acme.io/orders");
    let before = password_value(vcx);
    assert!(!before.is_empty());
    edit_row_port(vcx, "db.acme.io", "6000");
    assert_eq!(field(vcx, "port"), "6000");
    // The row's `****` is never taken as a new password.
    assert_eq!(password_value(vcx), before);
    assert_ne!(password_value(vcx), "****");
}

#[gpui::test]
fn editing_the_url_row_in_edit_mode_keeps_the_stored_password(cx: &mut TestAppContext) {
    let vcx = open_edit_saved(cx, "orders-prod", Retention::Forever);
    assert!(url_row(vcx).contains(":****@"), "{}", url_row(vcx));
    edit_row_port(vcx, "db-prod.acme.io", "6000");
    assert_eq!(field(vcx, "port"), "6000");
    assert_eq!(password_value(vcx), "");
    keys(vcx, &primary("s"));
    wait(vcx);
    let id = vcx
        .cx
        .read(|cx| cx.global::<ConnectionsModel>().profiles()[0].id);
    let stored = secret_backend(vcx)
        .get(&SecretKey {
            profile: id,
            slot: SecretSlot::DbPassword,
        })
        .expect("the backend answers")
        .expect("the stored password is still there");
    use secrecy::ExposeSecret as _;
    assert_ne!(stored.expose_secret(), "****");
    assert_eq!(read_port(vcx), Some(6000));
}

fn read_port(vcx: &VisualTestContext) -> Option<u16> {
    vcx.cx.read(
        |cx| match &cx.global::<ConnectionsModel>().profiles()[0].endpoint {
            ProfileEndpoint::Server { port, .. } => *port,
            _ => None,
        },
    )
}

#[gpui::test]
fn machine_text_is_mono_and_secrets_and_placeholders_are_not(cx: &mut TestAppContext) {
    let vcx = open_new(cx, "en");
    fill(vcx, &[("host", "db.acme.io"), ("password", "pw")]);
    vcx.run_until_parked();
    let family = |vcx: &VisualTestContext, name: &str| {
        read(vcx, |d, cx| {
            d.input(name).unwrap().read(cx).painted_family().to_string()
        })
    };
    assert_eq!(family(vcx, "host"), "Fragment Mono");
    // The URL row mirrors the host: mono too.
    assert_eq!(family(vcx, "url"), "Fragment Mono");
    assert_ne!(family(vcx, "password"), "Fragment Mono");
    // An empty field shows its placeholder in the sans face.
    assert_ne!(family(vcx, "database"), "Fragment Mono");
}

#[gpui::test]
fn a_url_pasted_into_an_extra_property_fills_the_form(cx: &mut TestAppContext) {
    let vcx = open_new(cx, "en");
    click_selector(vcx, "disclosure-conn-advanced");
    click(vcx, "conn-extra-add");
    let key = read(vcx, |d, _| d.extra_keys())
        .pop()
        .expect("an extra row");
    let handle = vcx.cx.read(|cx| key.focus_handle(cx));
    in_window(vcx, |window, cx| window.focus(&handle, cx));
    vcx.cx.write_to_clipboard(ClipboardItem::new_string(
        "postgres://u@db.acme.io/shop".to_string(),
    ));
    keys(vcx, &primary("v"));
    assert_eq!(field(vcx, "host"), "db.acme.io");
    assert_eq!(field(vcx, "database"), "shop");
}

#[gpui::test]
fn sqlite_read_only_and_open_read_only_are_two_controls(cx: &mut TestAppContext) {
    let vcx = open_new(cx, "en");
    choose_engine(vcx, DbmsKind::Sqlite);
    click_selector(vcx, "checkbox-conn-sqlite-ro");
    let (guard, mode_ro) = read(vcx, |d, _| {
        let p = d.profile();
        (
            p.read_only,
            p.extra.contains(&("mode".to_string(), "ro".to_string())),
        )
    });
    assert_eq!((guard, mode_ro), (false, true));
    click_selector(vcx, "checkbox-conn-read-only");
    let (guard, extras) = read(vcx, |d, _| {
        (d.profile().read_only, d.profile().extra.clone())
    });
    assert!(guard);
    assert_eq!(extras, [("mode".to_string(), "ro".to_string())]);
    click_selector(vcx, "checkbox-conn-sqlite-ro");
    assert!(read(vcx, |d, _| d.profile().extra.is_empty()
        && d.profile().read_only));
}

#[gpui::test]
fn creating_a_file_opened_read_only_is_an_issue(cx: &mut TestAppContext) {
    let vcx = open_new(cx, "en");
    choose_engine(vcx, DbmsKind::Sqlite);
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("new.db");
    type_into(vcx, "file", &path.to_string_lossy());
    click_selector(vcx, "checkbox-conn-sqlite-create");
    click_selector(vcx, "checkbox-conn-sqlite-ro");
    keys(vcx, &primary("s"));
    wait(vcx);
    assert_eq!(
        issue_under(vcx, "file"),
        Some("A new file can't be opened read-only. Turn one of the two off.".into())
    );
    assert!(profile_names(vcx).is_empty());
}

#[gpui::test]
fn client_certificate_has_no_passphrase_field_and_a_pasted_one_is_kept(cx: &mut TestAppContext) {
    let vcx = open_new(cx, "en");
    click_selector(vcx, "segmented-conn-auth-2");
    assert_eq!(
        read(vcx, |d, _| d.profile().auth),
        hacivat_secrets::AuthProviderId::ClientCert
    );
    assert!(has_field(vcx, "cert") && has_field(vcx, "key") && !has_field(vcx, "key-pass"));
    // Leaving Client certificate drops its files.
    type_into(vcx, "cert", "/tmp/hv-client.crt");
    click_selector(vcx, "segmented-conn-auth-0");
    assert!(read(vcx, |d, _| d.profile().tls.client_cert.is_none()));
    assert_eq!(field(vcx, "cert"), "");
    // A pasted `sslpassword` is still kept as the TLS key passphrase (ruling B).
    paste_into(
        vcx,
        "host",
        "postgres://u@localhost/shop?sslpassword=hv_test_pw",
    );
    keys(vcx, &primary("s"));
    wait(vcx);
    let id = vcx
        .cx
        .read(|cx| cx.global::<ConnectionsModel>().profiles()[0].id);
    let kept = secret_backend(vcx)
        .get(&SecretKey {
            profile: id,
            slot: SecretSlot::TlsKeyPassphrase,
        })
        .expect("the backend answers");
    assert!(kept.is_some());
}

/// Every label drawn with ID `id`.
fn labels(vcx: &VisualTestContext, id: &str) -> Vec<String> {
    let needle = format!("#{id}");
    registry_frame(vcx)
        .into_iter()
        .filter(|r| r.path.ends_with(&needle))
        .map(|r| r.text.to_string())
        .collect()
}

/// Opens the Edit dialog of a saved profile changed by `change` first.
fn open_edit_with(
    cx: &mut TestAppContext,
    change: impl FnOnce(&mut hacivat_session::profile::Profile),
) -> &'static mut VisualTestContext {
    let vcx = shell_with_connections(cx, "en");
    let mut draft = ProfileDraft::new();
    draft.profile.name = "certs".to_string();
    draft.profile.environment = Some(EnvTag::Dev);
    draft.profile.endpoint = ProfileEndpoint::Server {
        host: "db.acme.io".to_string(),
        port: None,
    };
    change(&mut draft.profile);
    let id = draft.profile.id;
    vcx.cx
        .update(|cx| ConnectionsModel::save(cx, draft).detach());
    settle(vcx, Duration::from_millis(100));
    open(vcx, DialogMode::Edit(id))
}

#[gpui::test]
fn a_refused_certificate_path_shows_under_the_certificate(cx: &mut TestAppContext) {
    let vcx = open_edit_with(cx, |p| {
        p.auth = hacivat_secrets::AuthProviderId::ClientCert;
        p.origin = hacivat_proto::Origin::DeepLink;
        p.tls.client_cert = Some(hacivat_net::ClientCertPaths {
            cert: "/hv-outside-roots/client.crt".into(),
            key: "/hv-outside-roots/client.key".into(),
        });
    });
    keys(vcx, &primary("s"));
    wait(vcx);
    let refused = "Hacivat won't open this path. Choose a local file.";
    assert!(
        labels(vcx, "issue-cert").contains(&refused.to_string()),
        "{:?}",
        labels(vcx, "issue-cert")
    );
    assert!(!notice_lines(vcx).contains(&refused.to_string()));
}

#[gpui::test]
fn an_issue_of_a_hidden_field_goes_to_the_top(cx: &mut TestAppContext) {
    let vcx = open_edit_with(cx, |p| {
        p.tls.client_cert = Some(hacivat_net::ClientCertPaths {
            cert: "/hv-missing/client.crt".into(),
            key: "/hv-missing/client.key".into(),
        });
    });
    keys(vcx, &primary("s"));
    wait(vcx);
    assert!(labels(vcx, "issue-cert").is_empty());
    assert!(
        notice_lines(vcx).contains(&"The client certificate or its key can't be read.".to_string()),
        "{:?}",
        notice_lines(vcx)
    );
}

#[gpui::test]
fn a_lost_validation_is_a_failed_save(cx: &mut TestAppContext) {
    let vcx = open_new(cx, "en");
    let d = dialog(vcx);
    in_window(vcx, |_, cx| d.update(cx, |d, cx| d.validation_lost(cx)));
    vcx.run_until_parked();
    assert!(!read(vcx, |d, _| d.is_saving()));
    assert!(
        notice_lines(vcx).contains(&"The connection couldn't be saved. Try again.".to_string())
    );
}

#[gpui::test]
fn closing_removes_the_dialog_layer_even_under_another_layer(cx: &mut TestAppContext) {
    let vcx = open_new(cx, "en");
    let d = dialog(vcx);
    push_test_layer(vcx, "probe", "Test");
    assert_eq!(layer_ids(vcx), ["connection-dialog", "probe"]);
    vcx.cx.update(|cx| {
        d.update(cx, |_, cx| {
            cx.emit(hacivat_app::connections::dialog::ConnectionDialogEvent::Closed)
        })
    });
    vcx.run_until_parked();
    assert_eq!(layer_ids(vcx), ["probe"]);
}

#[gpui::test]
fn rebuilding_extra_rows_drops_their_subscriptions(cx: &mut TestAppContext) {
    let vcx = open_new(cx, "en");
    paste_into(vcx, "url", "postgres://u@db.acme.io/shop?foo=1&bar=2");
    let first = read(vcx, |d, _| d.subscription_count());
    for _ in 0..3 {
        paste_into(vcx, "url", "postgres://u@db.acme.io/shop?foo=1&bar=2");
    }
    assert_eq!(read(vcx, |d, _| d.subscription_count()), first);
}

#[test]
fn a_pasted_text_never_shows_in_debug() {
    let mode = DialogMode::NewFromText("postgres://u:s3cret@h/db".to_string());
    let shown = format!("{mode:?}");
    assert!(!shown.contains("s3cret"), "{shown}");
    assert_eq!(shown, "NewFromText(<redacted>)");
}

// ## m2a-t21-shots

/// The stored TLS key passphrase of the first profile, straight from the backend.
fn stored_key_passphrase(vcx: &VisualTestContext) -> Option<SecretString> {
    let id = vcx
        .cx
        .read(|cx| cx.global::<ConnectionsModel>().profiles()[0].id);
    secret_backend(vcx)
        .get(&SecretKey {
            profile: id,
            slot: SecretSlot::TlsKeyPassphrase,
        })
        .expect("the backend answers")
}

#[gpui::test]
fn sslpassword_mask_in_the_url_row_never_replaces_the_stored_passphrase(cx: &mut TestAppContext) {
    use secrecy::ExposeSecret as _;
    let vcx = shell_with_connections(cx, "en");
    let mut draft = ProfileDraft::new();
    draft.profile.name = "certs".to_string();
    draft.profile.environment = Some(EnvTag::Dev);
    draft.profile.endpoint = ProfileEndpoint::Server {
        host: "db.acme.io".to_string(),
        port: None,
    };
    draft.profile.user = Some("deploy".to_string());
    draft.profile.database = Some("orders".to_string());
    draft.tls_key_passphrase = Some(SecretString::from("hv_test_pp".to_string()));
    let id = draft.profile.id;
    vcx.cx
        .update(|cx| ConnectionsModel::save(cx, draft).detach());
    settle(vcx, Duration::from_millis(100));
    assert!(
        stored_key_passphrase(vcx).is_some(),
        "the passphrase is stored"
    );
    let vcx = open(vcx, DialogMode::Edit(id));
    // The row as shown, edited to carry the Display mask as `sslpassword` and a new port.
    let row = url_row(vcx);
    let edited = row.replacen("@db.acme.io/", "@db.acme.io:6000/", 1);
    assert_ne!(row, edited, "the row names the host: {row}");
    let joiner = if edited.contains('?') { '&' } else { '?' };
    set_field(vcx, "url", &format!("{edited}{joiner}sslpassword=****"));
    keys(vcx, "enter");
    assert_eq!(field(vcx, "port"), "6000");
    keys(vcx, &primary("s"));
    wait(vcx);
    let stored = stored_key_passphrase(vcx).expect("the stored passphrase is still there");
    assert_ne!(stored.expose_secret(), "****");
    assert!(
        stored.expose_secret() == "hv_test_pp",
        "the stored passphrase was replaced"
    );
    assert_eq!(read_port(vcx), Some(6000));
}

#[gpui::test]
fn english_keeps_the_environment_segments(cx: &mut TestAppContext) {
    // English: every environment label fits the row, so the segmented control stays.
    let vcx = open_new(cx, "en");
    vcx.run_until_parked();
    assert!(!read(vcx, |d, _| d.environment_is_select()));
    assert!(vcx.debug_bounds("segmented-conn-env").is_some());
    assert!(vcx.debug_bounds("conn-env-select").is_none());
}

#[cfg(feature = "pseudo")]
#[gpui::test]
fn the_environment_select_sets_the_tag_like_the_segments(cx: &mut TestAppContext) {
    // The pseudo-locale's labels are some 40 % longer: the row takes a Select.
    let vcx = open_new(cx, "tr-XT");
    vcx.run_until_parked();
    assert!(read(vcx, |d, _| d.environment_is_select()));
    assert!(vcx.debug_bounds("conn-env-select").is_some());
    assert!(vcx.debug_bounds("segmented-conn-env").is_none());
    // "Choose…" while no tag is set; picking Staging (index 4) sets it.
    let select = read(vcx, |d, _| d.env_select());
    assert_eq!(vcx.cx.read(|cx| select.read(cx).selected()), 0);
    vcx.cx.update(|cx| {
        select.update(cx, |s, cx| {
            s.set_selected(4, cx);
            cx.emit(SelectEvent::Changed(4));
        })
    });
    vcx.run_until_parked();
    assert_eq!(env_pressed(vcx), Some(EnvTag::Staging));
}

#[cfg(feature = "pseudo")]
#[gpui::test]
fn the_environment_select_follows_the_automatic_local_tag(cx: &mut TestAppContext) {
    let vcx = open_new(cx, "tr-XT");
    vcx.run_until_parked();
    assert!(read(vcx, |d, _| d.environment_is_select()));
    let select = read(vcx, |d, _| d.env_select());
    assert_eq!(vcx.cx.read(|cx| select.read(cx).selected()), 0, "Choose…");
    type_into(vcx, "host", "localhost");
    assert_eq!(env_pressed(vcx), Some(EnvTag::Local));
    // The Select shows "local" (option 1), and draws that option's label.
    assert_eq!(vcx.cx.read(|cx| select.read(cx).selected()), 1);
    let local = vcx
        .cx
        .read(|cx| (select.read(cx).options()[1].label)(cx).to_string());
    let drawn: Vec<String> = registry_frame(vcx)
        .into_iter()
        .filter(|r| r.path.contains("hacivat-ui/src/select.rs:"))
        .map(|r| r.text.to_string())
        .collect();
    assert!(drawn.contains(&local), "{local:?} in {drawn:?}");
    // Back to a remote host: no tag, "Choose…" again.
    set_field(vcx, "host", "10.0.4.7");
    assert_eq!(env_pressed(vcx), None);
    assert_eq!(vcx.cx.read(|cx| select.read(cx).selected()), 0);
}

#[gpui::test]
fn every_in_well_segmented_control_fits_its_row_in_a_long_locale(cx: &mut TestAppContext) {
    use hacivat_ui::segmented::SEGMENT_MIN_PX;
    let vcx = open_new(cx, "tr-XT");
    vcx.simulate_resize(gpui::size(gpui::px(1024.0), gpui::px(700.0)));
    vcx.run_until_parked();
    let card = vcx
        .debug_bounds("form-dialog")
        .expect("the form dialog is drawn");
    for (group, count) in [("conn-engine", 4), ("conn-auth", 4)] {
        let sel: &'static str = Box::leak(format!("segmented-{group}").into_boxed_str());
        let bounds = vcx
            .debug_bounds(sel)
            .unwrap_or_else(|| panic!("{group} drawn"));
        assert!(bounds.right() <= card.right(), "{group} runs past the card");
        for i in 0..count {
            let sel: &'static str = Box::leak(format!("segmented-{group}-{i}").into_boxed_str());
            let w = vcx.debug_bounds(sel).expect("segment drawn").size.width;
            assert!(w >= gpui::px(SEGMENT_MIN_PX - 0.5), "{group}-{i}: {w:?}");
        }
    }
}

// ## m2a-finalfix

/// Focuses `input` and types `text` into it.
fn type_into_input(
    vcx: &mut VisualTestContext,
    input: &Entity<hacivat_ui::input::TextInput>,
    text: &str,
) {
    let handle = vcx.cx.read(|cx| input.focus_handle(cx));
    in_window(vcx, |window, cx| window.focus(&handle, cx));
    vcx.run_until_parked();
    vcx.simulate_input(text);
    vcx.run_until_parked();
}

/// Adds an extra property row (Advanced must be open) and types `key`, then `value`, into it.
/// The window is made active first: GPUI reports a field losing the focus only in an active
/// window, and leaving a row is what settles it.
fn add_extra(vcx: &mut VisualTestContext, key: &str, value: &str) {
    in_window(vcx, |window, _| window.activate_window());
    vcx.run_until_parked();
    click(vcx, "conn-extra-add");
    let k = read(vcx, |d, _| d.extra_keys())
        .pop()
        .expect("an extra row");
    let v = read(vcx, |d, _| d.extra_values())
        .pop()
        .expect("an extra row");
    type_into_input(vcx, &k, key);
    type_into_input(vcx, &v, value);
}

fn extra_key_texts(vcx: &VisualTestContext) -> Vec<String> {
    read(vcx, |d, cx| {
        d.extra_keys()
            .iter()
            .map(|k| k.read(cx).value().to_string())
            .collect()
    })
}

/// I1: a row whose key owns a slot moves into it when the row is left, so the password is
/// masked at once and kept like a typed one: in the keychain, never in the options or the
/// file. `sslpassword` goes to the TLS key passphrase.
#[gpui::test]
fn an_owner_key_extra_moves_into_its_secret_slot(cx: &mut TestAppContext) {
    let vcx = open_new(cx, "en");
    fill(vcx, &[("host", "localhost"), ("database", "shop")]);
    click_selector(vcx, "disclosure-conn-advanced");
    add_extra(vcx, "PGPASSWORD", "hv_test_pw");
    // Still typing in the row: nothing moved, and nothing reached the options.
    assert!(read(vcx, |d, _| d.profile().extra.is_empty()));
    focus_field(vcx, "name");
    assert_eq!(password_value(vcx), "hv_test_pw");
    assert!(extra_key_texts(vcx).is_empty());
    add_extra(vcx, "sslpassword", "k3y-pass");
    focus_field(vcx, "name");
    assert!(extra_key_texts(vcx).is_empty());
    assert!(read(vcx, |d, _| d.profile().extra.is_empty()));
    keys(vcx, &primary("s"));
    wait(vcx);
    assert!(layer_ids(vcx).is_empty(), "saved");
    let text = std::fs::read_to_string(paths(vcx).connections_toml()).expect("the file");
    assert!(!text.contains("hv_test_pw") && !text.contains("k3y-pass"));
    assert!(!text.to_ascii_lowercase().contains("password"), "{text}");
    assert!(
        keyring_entry(vcx).is_some(),
        "the password is in the keychain"
    );
    let passphrase = stored_key_passphrase(vcx).expect("the passphrase is stored");
    use secrecy::ExposeSecret as _;
    assert!(
        passphrase.expose_secret() == "k3y-pass",
        "passphrase mismatch"
    );
}

/// I1: a password-like key that owns no slot (`db_password`) is refused: it never enters the
/// options, the form says why under Advanced, and the save is blocked until it is removed.
#[gpui::test]
fn a_password_like_extra_is_refused_and_blocks_the_save(cx: &mut TestAppContext) {
    use hacivat_session::pipeline::IssueCode;
    let vcx = open_new(cx, "en");
    fill(vcx, &[("host", "localhost"), ("database", "shop")]);
    click_selector(vcx, "disclosure-conn-advanced");
    add_extra(vcx, "db_password", "hv_test_pw");
    focus_field(vcx, "name");
    assert!(read(vcx, |d, _| d.profile().extra.is_empty()));
    assert!(read(vcx, |d, _| d.issues().iter().any(|i| i.code
        == IssueCode::OptionSecret {
            key: "db_password".into()
        })));
    let shown = issue_under(vcx, "app-name")
        .expect("the issue is shown under Advanced")
        .replace(['\u{2068}', '\u{2069}'], "");
    assert_eq!(
        shown,
        "The property db_password holds a password. Remove it and type the password in the Password field."
    );
    keys(vcx, &primary("s"));
    wait(vcx);
    assert_eq!(layer_ids(vcx), ["connection-dialog"]);
    assert!(profile_names(vcx).is_empty());
    // Removed: the issue goes and the save goes through, without the secret.
    click(vcx, "conn-extra-remove-0");
    assert!(read(vcx, |d, _| d.issues().is_empty()));
    focus_field(vcx, "name");
    keys(vcx, &primary("s"));
    wait(vcx);
    assert!(layer_ids(vcx).is_empty());
    let text = std::fs::read_to_string(paths(vcx).connections_toml()).expect("the file");
    assert!(!text.contains("hv_test_pw") && !text.contains("db_password"));
}

/// I2: a row with a typed setting goes to that setting when the row is left, so the form,
/// the fixes and the connect all see one value.
#[gpui::test]
fn a_typed_setting_in_the_extras_goes_to_its_field(cx: &mut TestAppContext) {
    let vcx = open_new(cx, "en");
    click_selector(vcx, "disclosure-conn-advanced");
    add_extra(vcx, "sslmode", "disable");
    focus_field(vcx, "name");
    assert_eq!(draft_tls_mode(vcx), TlsMode::Disable);
    let select = read(vcx, |d, _| d.tls_select());
    let selected = vcx.cx.read(|cx| select.read(cx).selected());
    assert_eq!(selected, 4, "the TLS Select shows Disable");
    add_extra(vcx, "connect_timeout", "9");
    focus_field(vcx, "name");
    assert_eq!(field(vcx, "timeout"), "9");
    assert!(extra_key_texts(vcx).is_empty());
    assert!(read(vcx, |d, _| d.profile().extra.is_empty()));
    // A value no rule knows stays an extra and changes nothing.
    add_extra(vcx, "sslmode", "bogus");
    focus_field(vcx, "name");
    assert_eq!(extra_key_texts(vcx), ["sslmode"]);
    assert_eq!(draft_tls_mode(vcx), TlsMode::Disable);
}

/// I5: a pasted URL makes the profile an import (`Imported { source: Url }`), so DRV-05 and
/// the approved roots apply: a CA file the URL named outside them is refused, and the same
/// file picked in the dialog passes. Text typed into the URL row keeps the user's origin.
#[gpui::test]
fn a_pasted_url_is_an_import_and_a_picked_file_still_passes(cx: &mut TestAppContext) {
    use hacivat_proto::{ImportSource, Origin};
    let vcx = open_new(cx, "en");
    let dir = tempfile::tempdir().expect("temp dir");
    let ca = dir.path().join("ca.pem");
    std::fs::write(
        &ca,
        include_bytes!("../../../core/hacivat-session/tests/fixtures/test-ca.pem"),
    )
    .expect("the CA file");
    let outside_home = std::env::home_dir().is_none_or(|home| !ca.starts_with(home));
    paste_into(
        vcx,
        "host",
        &format!(
            "postgres://deploy@localhost/shop?sslmode=verify-full&sslrootcert={}",
            ca.display()
        ),
    );
    assert_eq!(
        read(vcx, |d, _| d.profile().origin.clone()),
        Origin::Imported {
            source: ImportSource::Url
        }
    );
    assert_eq!(
        read(vcx, |d, _| d.profile().tls.ca_file.clone()),
        Some(ca.clone())
    );
    if outside_home {
        keys(vcx, &primary("s"));
        wait(vcx);
        assert_eq!(layer_ids(vcx), ["connection-dialog"]);
        assert_eq!(
            issue_under(vcx, "ca"),
            Some("Hacivat won't open this path. Choose a local file.".into())
        );
    }
    let d = dialog(vcx);
    in_window(vcx, |window, cx| {
        d.update(cx, |d, cx| {
            d.picked(
                hacivat_app::connections::dialog::form::Field::Ca,
                ca.clone(),
                window,
                cx,
            )
        })
    });
    keys(vcx, &primary("s"));
    wait(vcx);
    assert!(layer_ids(vcx).is_empty(), "the picked file is approved");
    let origin = vcx
        .cx
        .read(|cx| cx.global::<ConnectionsModel>().profiles()[0].origin.clone());
    assert_eq!(
        origin,
        Origin::Imported {
            source: ImportSource::Url
        }
    );
}

/// I5: text the user types into the URL row is their own: the profile keeps its origin.
#[gpui::test]
fn the_url_rows_typed_text_keeps_the_users_origin(cx: &mut TestAppContext) {
    let vcx = open_new(cx, "en");
    fill(
        vcx,
        &[("host", "db.acme.io"), ("user", "u"), ("database", "x")],
    );
    edit_row_port(vcx, "db.acme.io", "6000");
    assert_eq!(field(vcx, "port"), "6000");
    assert_eq!(
        read(vcx, |d, _| d.profile().origin.clone()),
        hacivat_proto::Origin::UserTyped
    );
}
