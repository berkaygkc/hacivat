//! M2a flows across surfaces (Task 21): the connection dialog (T17), the explorer, the live
//! capsule and the commands (T18), and the test strip, the error dialog and the production
//! prompt (T20), driven together through one app.
//!
//! Every connect runs through `FakeDriver`, which dials itself, so no socket is opened; hosts
//! are IP literals, so no DNS query is sent (P46). The production suggestion therefore comes
//! from the connection's name, never from a host name that would need resolving.

mod support;

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

use gpui::{Entity, Focusable as _, TestAppContext, VisualTestContext};
use hacivat_app::connections::ConnectionsModel;
use hacivat_app::connections::dialog::{
    ConnectionDialog, DialogMode, open_connection_dialog, open_dialog,
};
use hacivat_app::connections::explorer;
use hacivat_driver::fake::FakeDriver;
use hacivat_driver::{RawError, RawErrorKind, ServerMessage};
use hacivat_proto::{DbmsKind, EnvTag};
use hacivat_secrets::{MemoryBackend, SecretBackend, SecretError, SecretKey, SecretString};
use hacivat_session::LiveState;
use hacivat_ui::capsule::{CapsuleModel, CapsuleProbe, EnvKind};
use hacivat_ui::text::{RenderedText, TextSource};
use support::*;

// ---- shells and fakes ---------------------------------------------------------------------------

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

fn ok_pg() -> FakeDriver {
    FakeDriver::new(DbmsKind::Postgres)
}

fn refused() -> RawError {
    RawError {
        kind: RawErrorKind::Io(std::io::ErrorKind::ConnectionRefused),
        server: None,
    }
}

/// A server where the first connect asks for a database that does not exist (3D000); the
/// probe connect that lists databases then works and finds `names`.
fn unknown_db_fake<const N: usize>(names: [&str; N]) -> FakeDriver {
    FakeDriver::new(DbmsKind::Postgres)
        .script(vec![Err(RawError {
            kind: RawErrorKind::Db,
            server: Some(ServerMessage {
                sqlstate: Some("3D000".into()),
                vendor_code: None,
                severity: Some("FATAL".into()),
                text: "database does not exist".into(),
                detail: None,
                hint: None,
                lang_hint: None,
            }),
        })])
        .databases(names.iter().map(|n| n.to_string()).collect())
}

/// An English connections shell over `fake`, with the capsule probe installed.
fn app_with_fake(cx: &mut TestAppContext, fake: FakeDriver) -> &'static mut VisualTestContext {
    app_with_fake_lang(cx, "en", fake)
}

fn app_with_fake_lang(
    cx: &mut TestAppContext,
    lang: &str,
    fake: FakeDriver,
) -> &'static mut VisualTestContext {
    let vcx = shell_with_fake(cx, lang, fake);
    vcx.cx.update(|cx| cx.set_global(CapsuleProbe::default()));
    vcx.run_until_parked();
    vcx
}

/// A shell with a saved profile `name` (dev, loopback).
fn app_with_saved(cx: &mut TestAppContext, name: &str) -> &'static mut VisualTestContext {
    let vcx = app_with_fake(cx, ok_pg());
    add_profile(vcx, test_profile(name, Some(EnvTag::Dev), "127.0.0.1"));
    vcx
}

/// Connects the saved profile `name`, which this adds first (dev, loopback).
fn connect_saved(vcx: &VisualTestContext, name: &str) {
    add_profile(vcx, test_profile(name, Some(EnvTag::Dev), "127.0.0.1"));
    connect(vcx, name);
}

// ---- the dialog ---------------------------------------------------------------------------------

fn primary(key: &str) -> String {
    if cfg!(target_os = "macos") {
        format!("cmd-{key}")
    } else {
        format!("ctrl-{key}")
    }
}

fn open_new_dialog(vcx: &VisualTestContext) {
    in_window(vcx, |window, cx| {
        open_connection_dialog(DialogMode::New, window, cx)
    });
    // The keyring probe runs on the core runtime (P36).
    settle(vcx, ms(20));
}

fn dialog(vcx: &VisualTestContext) -> Entity<ConnectionDialog> {
    in_window(vcx, |window, cx| open_dialog(window, cx)).expect("a connection dialog is open")
}

fn focus_field(vcx: &VisualTestContext, name: &str) {
    let d = dialog(vcx);
    let input = vcx
        .cx
        .read(|cx| d.read(cx).input(name))
        .unwrap_or_else(|| panic!("no field {name}"));
    let handle = vcx.cx.read(|cx| input.focus_handle(cx));
    in_window(vcx, |window, cx| window.focus(&handle, cx));
    vcx.run_until_parked();
}

fn fill(vcx: &mut VisualTestContext, pairs: &[(&str, &str)]) {
    for (name, text) in pairs {
        focus_field(vcx, name);
        vcx.simulate_input(text);
        vcx.run_until_parked();
    }
}

/// A form that validates and connects without asking: a loopback server, a user and a
/// password (without one, the test would open the password prompt, R14).
fn fill_valid(vcx: &mut VisualTestContext) {
    fill(
        vcx,
        &[
            ("host", "127.0.0.1"),
            ("user", "deploy"),
            ("password", "pw"),
        ],
    );
}

fn fill_valid_db(vcx: &mut VisualTestContext, database: &str) {
    fill(
        vcx,
        &[
            ("host", "127.0.0.1"),
            ("database", database),
            ("user", "deploy"),
            ("password", "pw"),
        ],
    );
}

fn field(vcx: &VisualTestContext, name: &str) -> String {
    let d = dialog(vcx);
    vcx.cx
        .read(|cx| d.read(cx).field_value(name, cx))
        .unwrap_or_else(|| panic!("no field {name}"))
}

fn strip_running(vcx: &VisualTestContext) -> bool {
    vcx.run_until_parked();
    let d = dialog(vcx);
    vcx.cx.read(|cx| d.read(cx).strip().read(cx).is_running())
}

/// The text of the label with element ID `id`, if drawn.
fn label(vcx: &VisualTestContext, id: &str) -> Option<String> {
    text_of(vcx, id)
}

fn dialog_title(vcx: &VisualTestContext) -> String {
    label(vcx, "conn-dialog-title").expect("the dialog title is drawn")
}

// ---- the One Filled Badge rule, scoped to the title-bar capsule (T21 ruling) -------------------

#[cfg(feature = "scene-check")]
fn assert_one_filled_badge(vcx: &mut VisualTestContext, production: bool) {
    use hacivat_ui_core::named_rules::assert_one_filled_badge_in;
    use hacivat_ui_core::theme::Theme;
    vcx.cx.update(|cx| cx.refresh_windows());
    vcx.run_until_parked();
    let cp_fill = vcx
        .cx
        .read(|cx| Theme::get(cx).tokens.colors.cp_fill.to_hsla());
    let capsule = vcx.cx.read(|cx| cx.global::<CapsuleProbe>().bounds);
    assert!(capsule.is_some(), "the capsule was painted");
    vcx.update(|window, _| assert_one_filled_badge_in(window, cp_fill, production, capsule));
}

#[cfg(not(feature = "scene-check"))]
fn assert_one_filled_badge(_: &mut VisualTestContext, _: bool) {}

// ---- the brief's flows --------------------------------------------------------------------------

#[gpui::test]
fn save_remote_asks_environment_then_capsule_turns_production(cx: &mut TestAppContext) {
    let vcx = app_with_fake(cx, ok_pg());
    open_new_dialog(vcx);
    fill(
        vcx,
        &[
            ("name", "orders-prod-us"),
            ("host", "10.0.4.12"),
            ("database", "orders"),
            ("user", "deploy"),
            ("password", "pw"),
        ],
    );
    vcx.simulate_keystrokes(&primary("enter"));
    wait_until(vcx, "the production prompt", |vcx| {
        layer_ids(vcx).last() == Some(&"production-prompt")
    });
    assert_eq!(layer_ids(vcx), ["connection-dialog", "production-prompt"]);
    vcx.simulate_keystrokes("enter");
    wait_until(vcx, "the dialog to close", |vcx| layer_ids(vcx).is_empty());
    wait_until(vcx, "the connect", |vcx| {
        matches!(live(vcx, "orders-prod-us"), LiveState::Connected(_))
    });
    assert!(layer_ids(vcx).is_empty());
    let m = capsule_model(vcx);
    assert!(
        matches!(
            m,
            CapsuleModel::Connection {
                env: EnvKind::Production,
                ..
            }
        ),
        "{m:?}"
    );
    assert_one_filled_badge(vcx, true);
}

#[gpui::test]
fn escape_order_in_the_dialog(cx: &mut TestAppContext) {
    // Held at the driver: the test runs for as long as this test looks at it.
    let vcx = app_with_fake(cx, ok_pg().gated());
    open_new_dialog(vcx);
    fill_valid(vcx);
    vcx.simulate_keystrokes(&primary("t"));
    wait_until(vcx, "the test to reach the driver", |vcx| {
        fake(vcx).connect_calls() == 1
    });
    assert!(strip_running(vcx));
    vcx.simulate_keystrokes("escape");
    assert!(!strip_running(vcx));
    assert_eq!(layer_ids(vcx), ["connection-dialog"]);
    vcx.simulate_keystrokes("escape");
    vcx.run_until_parked();
    assert!(layer_ids(vcx).is_empty());
}

#[gpui::test]
fn closing_dialog_cancels_running_test(cx: &mut TestAppContext) {
    // Held at the driver until the dialog is closed, then let through: the late connection
    // must be closed (P23), not left open.
    let vcx = app_with_fake(cx, ok_pg().gated());
    open_new_dialog(vcx);
    fill_valid(vcx);
    vcx.simulate_keystrokes(&primary("t"));
    wait_until(vcx, "the test to reach the driver", |vcx| {
        fake(vcx).connect_calls() == 1
    });
    assert!(strip_running(vcx));
    click(vcx, "button-conn-cancel");
    assert!(layer_ids(vcx).is_empty());
    fake(vcx).release();
    wait_until(vcx, "the late connection to be closed", |vcx| {
        fake(vcx).close_calls() >= 1 && fake(vcx).open_connections() == 0
    });
    let f = fake(vcx);
    assert_eq!(f.connect_calls(), 1);
    assert_eq!(f.close_calls(), f.connect_calls());
    assert_eq!(f.open_connections(), 0);
}

#[gpui::test]
fn inline_test_error_fix_use_database(cx: &mut TestAppContext) {
    let vcx = app_with_fake(cx, unknown_db_fake(["orders"]));
    open_new_dialog(vcx);
    fill_valid_db(vcx, "orders_v2");
    vcx.simulate_keystrokes(&primary("t"));
    wait_until(vcx, "the test to fail", |vcx| !strip_running(vcx));
    settle(vcx, ms(20));
    click(vcx, "button-fix-use-0");
    assert_eq!(field(vcx, "database"), "orders");
}

#[gpui::test]
fn cmd_e_opens_the_edit_dialog(cx: &mut TestAppContext) {
    let vcx = app_with_saved(cx, "orders-prod");
    in_window(vcx, explorer::focus_list);
    vcx.run_until_parked();
    vcx.simulate_keystrokes(&primary("e"));
    settle(vcx, ms(20));
    assert_eq!(layer_ids(vcx), ["connection-dialog"]);
    assert_eq!(dialog_title(vcx), "Edit orders-prod");
}

#[gpui::test]
fn failed_capsule_click_opens_error_dialog(cx: &mut TestAppContext) {
    let vcx = app_with_fake(
        cx,
        FakeDriver::new(DbmsKind::Postgres).connect_result(Err(refused())),
    );
    connect_saved(vcx, "a");
    wait_until(vcx, "the connect to fail", |vcx| {
        matches!(live(vcx, "a"), LiveState::Failed(_))
    });
    settle(vcx, ms(20));
    click(vcx, "capsule-body");
    assert_eq!(layer_ids(vcx), ["connection-error"]);
}

#[gpui::test]
fn tr_ui_end_to_end_strings(cx: &mut TestAppContext) {
    let vcx = app_with_fake_lang(cx, "tr", ok_pg());
    open_new_dialog(vcx);
    assert_eq!(dialog_title(vcx), "Yeni bağlantı");
    let frame = registry_frame(vcx);
    assert!(frame.len() > 20, "the dialog drew its labels");
    let leaks = english_leaks(&frame);
    assert!(leaks.is_empty(), "English in the Turkish UI: {leaks:?}");
}

// ---- consolidation follow-ups (T21) ---------------------------------------------------------------

/// The one-line English values of the UI catalogs that differ from every Turkish value
/// (`TLS`, `pgpass` and the like are the same in both and are not leaks).
fn english_only() -> BTreeSet<String> {
    let values = |lang: &str| -> BTreeSet<String> {
        ["hacivat-ui.ftl", "hui.ftl"]
            .iter()
            .flat_map(|file| {
                let text = workspace_file(&format!("locales/{lang}/{file}"));
                text.lines()
                    .filter(|l| !l.starts_with('#') && !l.starts_with(' '))
                    .filter_map(|l| l.split_once(" = ").map(|(_, v)| v.trim().to_string()))
                    .filter(|v| !v.is_empty() && !v.contains('{'))
                    .collect::<Vec<_>>()
            })
            .collect()
    };
    let tr = values("tr");
    values("en").difference(&tr).cloned().collect()
}

/// The Fluent texts of `frame` that are English-only catalog values.
fn english_leaks(frame: &[RenderedText]) -> Vec<String> {
    let english = english_only();
    frame
        .iter()
        .filter(|r| r.source == TextSource::Fluent)
        .map(|r| {
            r.text
                .chars()
                .filter(|c| !matches!(c, '\u{2068}' | '\u{2069}'))
                .collect::<String>()
        })
        .filter(|t| english.contains(t))
        .collect()
}

#[test]
fn the_leak_check_finds_english_text() {
    let label = |text: &str| RenderedText {
        text: text.to_string().into(),
        source: TextSource::Fluent,
        size_px: 13.0,
        painted_px: 10.0,
        clipped: false,
        truncated: false,
        path: "x.rs:1:1".into(),
    };
    let leaks = english_leaks(&[
        label("New connection"),
        label("Yeni bağlantı"),
        label("TLS"),
    ]);
    assert_eq!(leaks, ["New connection"]);
}

#[gpui::test]
fn the_status_bar_and_the_sheet_use_the_shared_environment_words(cx: &mut TestAppContext) {
    let vcx = app_with_fake_lang(cx, "tr", ok_pg());
    add_profile(
        vcx,
        test_profile("orders-prod", Some(EnvTag::Production), "10.0.4.12"),
    );
    connect(vcx, "orders-prod");
    wait_until(vcx, "the connect", |vcx| {
        matches!(live(vcx, "orders-prod"), LiveState::Connected(_))
    });
    let status = status_bar_text(vcx);
    assert!(status.contains("üretim · okuma-yazma"), "{status}");
}

/// A keyring whose deletes fail (a locked keychain).
struct FailingDelete(MemoryBackend);

impl SecretBackend for FailingDelete {
    fn get(&self, key: &SecretKey) -> Result<Option<SecretString>, SecretError> {
        self.0.get(key)
    }
    fn put(&self, key: &SecretKey, value: &SecretString) -> Result<(), SecretError> {
        self.0.put(key, value)
    }
    fn delete(&self, _: &SecretKey) -> Result<(), SecretError> {
        Err(SecretError::Denied)
    }
    fn name(&self) -> &'static str {
        "failing-delete"
    }
}

#[gpui::test]
fn a_keyring_entry_left_behind_by_a_delete_is_named(cx: &mut TestAppContext) {
    let vcx = shell_with_fake_backend(
        cx,
        "en",
        ok_pg(),
        Arc::new(FailingDelete(MemoryBackend::new())),
        |_| {},
    );
    let mut profile = test_profile("a", Some(EnvTag::Dev), "127.0.0.1");
    profile.auth = hacivat_secrets::AuthProviderId::Password;
    let id = save_draft(
        vcx,
        hacivat_session::profile::ProfileDraft {
            is_new: true,
            password: Some(SecretString::from("hv_test_pw".to_string())),
            ..hacivat_session::profile::ProfileDraft::edit(profile)
        },
    );
    settle(vcx, ms(50));
    vcx.cx
        .update(|cx| hacivat_app::connections::delete::delete_now(id, "a".to_string(), cx));
    wait_until(vcx, "the delete", |vcx| {
        vcx.cx
            .read(|cx| cx.global::<ConnectionsModel>().get(id).is_none())
    });
    // The keyring delete fails on the profile writer, after the profile is gone: its toast
    // comes when the writer is done.
    wait_until(vcx, "the delete's toast", |vcx| {
        !toast_texts(vcx).is_empty()
    });
    assert_eq!(
        toast_texts(vcx),
        [
            "The connection was deleted, but its saved password couldn't be removed from the system keyring."
        ]
    );
}

#[gpui::test]
fn the_connection_sheet_scrolls_from_the_keyboard(cx: &mut TestAppContext) {
    let vcx = app_with_fake(cx, ok_pg());
    let mut p = test_profile("orders-prod", Some(EnvTag::Production), "10.0.4.12");
    p.read_only = true;
    let id = add_profile(vcx, p);
    vcx.cx.update(|cx| ConnectionsModel::select(cx, Some(id)));
    connect(vcx, "orders-prod");
    wait_until(vcx, "the connect", |vcx| {
        matches!(live(vcx, "orders-prod"), LiveState::Connected(_))
    });
    // A short window: the sheet's rows overflow its tile.
    vcx.simulate_resize(gpui::size(gpui::px(1280.0), gpui::px(560.0)));
    vcx.run_until_parked();
    vcx.cx.update(|cx| cx.refresh_windows());
    vcx.run_until_parked();
    let rows = vcx
        .debug_bounds("sheet-rows")
        .expect("the sheet rows are drawn");
    let thumb = |vcx: &mut VisualTestContext| {
        vcx.debug_bounds("sheet-scroll-thumb")
            .expect("a scroll thumb while the rows overflow")
    };
    assert!(
        (thumb(vcx).top() - rows.top()).abs() < gpui::px(1.0),
        "starts at the top"
    );
    // A click puts the focus on the rows; End scrolls to the bottom, Home back, ↓ one row.
    vcx.simulate_click(rows.center(), gpui::Modifiers::none());
    vcx.run_until_parked();
    for (key, at_bottom) in [("end", true), ("home", false)] {
        vcx.simulate_keystrokes(key);
        vcx.run_until_parked();
        let t = thumb(vcx);
        if at_bottom {
            assert!(
                (t.bottom() - rows.bottom()).abs() < gpui::px(1.0),
                "{key}: {t:?}"
            );
        } else {
            assert!((t.top() - rows.top()).abs() < gpui::px(1.0), "{key}: {t:?}");
        }
    }
    vcx.simulate_keystrokes("down");
    vcx.run_until_parked();
    assert!(
        thumb(vcx).top() > rows.top() + gpui::px(0.5),
        "down moves it"
    );
}
