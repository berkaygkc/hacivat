//! Task 19: the TLS UI — the certificate review (pin, import CA), the pin-changed hard stop,
//! the public-plaintext question, the TLS section's pinned row and Disable notice, the key
//! passphrase, `ConnectionsModel::apply_changes`, a Test's changes into the draft only, and the
//! removal of "Encrypt without verifying" (B5–B7, B25, Q26, Q34).
//!
//! The shell helpers call `allow_parking()`; every wait is a condition (`settle_until`,
//! `wait_until`), never a fixed sleep (Q4 ruling).

mod support;

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

use gpui::{Entity, Focusable as _, Modifiers, TestAppContext, VisualTestContext};
use hacivat_app::connections::ConnectionsModel;
use hacivat_app::connections::cert_dialog::{self, CERT_LAYER_ID, CertDialog, Validity};
use hacivat_app::connections::dialog::{
    ConnectionDialog, DialogEnv, DialogMode, open_connection_dialog, open_dialog,
};
use hacivat_app::connections::error_view::{ErrorView, open_error_dialog_view};
use hacivat_app::connections::interaction::interaction;
use hacivat_app::connections::plaintext_prompt::{self, PLAINTEXT_LAYER_ID};
use hacivat_app::connections::test_strip::TestStripState;
use hacivat_app::l10n_global::tr_id;
use hacivat_driver::fake::FakeDriver;
use hacivat_driver::{RawError, RawErrorKind};
use hacivat_net::{
    CertPin, CertReview, ChainEntry, ChainRole, ClientCertPaths, TlsFailure, TlsMode, TlsSummary,
    TlsVerified,
};
use hacivat_proto::{DbmsKind, EnvTag, ErrClass, FixId, ProfileId};
use hacivat_secrets::AuthProviderId;
use hacivat_session::ConnectionEvent;
use hacivat_session::pipeline::{
    Answer, ConnectError, ErrArgs, Interaction, IssueCode, Phase, ProfileChange, Question,
};
use hacivat_session::profile::Profile;
use hacivat_ui::select::SelectEvent;

use support::*;

/// The test CA (a self-signed certificate), as DER and as PEM.
const TEST_CA_DER: &[u8] =
    include_bytes!("../../../core/hacivat-session/tests/fixtures/test-ca.der");
const TEST_CA_PEM: &str = include_str!("../../../core/hacivat-session/tests/fixtures/test-ca.pem");

/// 2020-01-01 and 2100-01-01: a validity window that holds today.
const NOT_BEFORE: i64 = 1_577_836_800;
const NOT_AFTER: i64 = 4_102_444_800;

// ---- fixtures -----------------------------------------------------------------------------------

/// A review of `127.0.0.1:5432`'s certificate for `subject`, issued by Acme's own CA.
fn review_fixture(subject: &str, sha256: [u8; 32]) -> CertReview {
    let leaf = format!("CN={subject},O=Acme");
    CertReview {
        endpoint: "127.0.0.1:5432".into(),
        subject: leaf.clone(),
        issuer: "CN=Acme Internal CA G2,O=Acme".into(),
        not_before_unix: NOT_BEFORE,
        not_after_unix: NOT_AFTER,
        sha256,
        chain: vec![
            ChainEntry {
                subject: leaf,
                role: ChainRole::Leaf,
            },
            ChainEntry {
                subject: "CN=Acme Internal CA G2,O=Acme".into(),
                role: ChainRole::Intermediate,
            },
            ChainEntry {
                subject: "CN=Acme Root CA 2021,O=Acme".into(),
                role: ChainRole::Root,
            },
        ],
        pinned: None,
    }
}

/// The review of a changed pin: `old` was pinned, `new` is presented.
fn pinned_review(old: [u8; 32], new: [u8; 32]) -> CertReview {
    CertReview {
        pinned: Some(old),
        ..review_fixture("db", new)
    }
}

fn cert_question(review: CertReview) -> Question {
    Question::Certificate {
        profile: ProfileId::new(),
        name: "orders".into(),
        review,
    }
}

fn pin(endpoint: &str, sha256: [u8; 32]) -> CertPin {
    CertPin {
        endpoint: endpoint.into(),
        sha256,
    }
}

fn tls_error(class: ErrClass, fixes: Vec<FixId>, review: Option<CertReview>) -> ConnectError {
    ConnectError {
        class,
        phase: Phase::DriverConnect,
        trail: Vec::new(),
        args: ErrArgs {
            name: "orders".into(),
            host: Some("127.0.0.1".into()),
            port: Some(5432),
            user: Some("deploy".into()),
            database: None,
            file: None,
            engine: "PostgreSQL".into(),
            version: None,
            hop: None,
            hop_host: None,
        },
        server: None,
        fixes,
        suggestions: Vec::new(),
        issues: Vec::new(),
        elapsed: std::time::Duration::ZERO,
        attempts: 0,
        needs_interaction: false,
        certificate: review,
        host_key: None,
        profile_changes: Vec::new(),
    }
}

/// `tls.untrusted` with its certificate, as the pipeline leaves it after a cancelled question.
fn untrusted_error_with_review() -> ConnectError {
    tls_error(
        ErrClass::TlsUntrusted,
        vec![
            FixId::ReviewCertificate,
            FixId::ImportCa,
            FixId::EditConnection,
        ],
        Some(review_fixture("db", [0xAB; 32])),
    )
}

/// `tls.pin_changed`: `old` was pinned, `new` is presented.
fn pin_changed_error(old: [u8; 32], new: [u8; 32]) -> ConnectError {
    tls_error(
        ErrClass::TlsPinChanged,
        vec![FixId::ReviewCertificate],
        Some(pinned_review(old, new)),
    )
}

/// A connections shell with the profile `orders` at 127.0.0.1, pinned to `old` when given.
fn with_profile(
    cx: &mut TestAppContext,
    old: Option<[u8; 32]>,
) -> (&'static mut VisualTestContext, ProfileId) {
    let vcx = shell_with_connections(cx, "en");
    let mut p = test_profile("orders", Some(EnvTag::Dev), "127.0.0.1");
    p.tls.pins = old
        .map(|sha| pin("127.0.0.1:5432", sha))
        .into_iter()
        .collect();
    let id = add_profile(vcx, p);
    (vcx, id)
}

// ---- driving and reading ------------------------------------------------------------------------

type Slot = Rc<RefCell<Option<Answer>>>;

/// Asks `q` through the app's UI port, as the pipeline does from the core runtime.
fn ask(vcx: &VisualTestContext, q: Question) -> Slot {
    let slot = Rc::new(RefCell::new(None));
    let out = slot.clone();
    vcx.cx.update(|cx| {
        let Interaction::Interactive(port) = interaction(cx) else {
            panic!("the UI port is installed");
        };
        let answer = port.ask(q);
        cx.spawn(async move |_| {
            *out.borrow_mut() = Some(answer.await);
        })
        .detach();
    });
    slot
}

/// The answer in `slot`, once it came.
fn answered(vcx: &VisualTestContext, slot: &Slot) -> Answer {
    settle_until(vcx, WAIT, || slot.borrow().is_some());
    slot.borrow_mut().take().expect("an answer")
}

fn layer_open(vcx: &VisualTestContext, id: &str) -> bool {
    layer_ids(vcx).contains(&id)
}

fn wait_layer(vcx: &VisualTestContext, id: &str) {
    settle_until(vcx, WAIT, || layer_open(vcx, id));
}

fn press(vcx: &mut VisualTestContext, keys: &str) {
    vcx.simulate_keystrokes(keys);
    settle(vcx, std::time::Duration::ZERO);
}

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
fn click_button(vcx: &mut VisualTestContext, id: &str) {
    click_selector(vcx, &format!("button-{id}"));
}

fn width(vcx: &mut VisualTestContext, selector: &'static str) -> f32 {
    vcx.run_until_parked();
    f32::from(
        vcx.debug_bounds(selector)
            .unwrap_or_else(|| panic!("`{selector}` is drawn"))
            .size
            .width,
    )
}

/// `text` without the Unicode isolation marks Fluent puts around arguments.
fn plain(text: impl AsRef<str>) -> String {
    text.as_ref().replace(['\u{2068}', '\u{2069}'], "")
}

fn t(vcx: &VisualTestContext, id: &str) -> String {
    vcx.cx.read(|cx| plain(tr_id(cx, id)))
}

fn review_view(vcx: &VisualTestContext) -> Entity<CertDialog> {
    vcx.cx
        .read(cert_dialog::open_view)
        .expect("a certificate review is open")
}

fn review<R>(vcx: &VisualTestContext, f: impl FnOnce(&CertDialog) -> R) -> R {
    let view = review_view(vcx);
    vcx.cx.read(|cx| f(view.read(cx)))
}

fn focused_button(vcx: &VisualTestContext) -> Option<&'static str> {
    let view = review_view(vcx);
    in_window(vcx, |window, cx| view.read(cx).focused_button(window))
}

/// The two drawn lines of the fingerprint `prefix` (`cert-fingerprint`, `cert-fingerprint-old`).
fn mono_lines(vcx: &VisualTestContext, prefix: &str) -> Vec<String> {
    (1..=2)
        .filter_map(|n| text_of(vcx, &format!("{prefix}-line-{n}")))
        .collect()
}

/// The 32 bytes `[b; 32]` as the review writes them.
fn hex32(b: u8) -> String {
    vec![format!("{b:02X}"); 32].join(":")
}

fn connections_toml(vcx: &VisualTestContext) -> String {
    std::fs::read_to_string(paths(vcx).connections_toml()).unwrap_or_default()
}

fn saved(vcx: &VisualTestContext, id: ProfileId) -> Arc<Profile> {
    vcx.cx.read(|cx| {
        cx.global::<ConnectionsModel>()
            .get(id)
            .expect("the profile is saved")
    })
}

fn approved_roots(vcx: &VisualTestContext) -> Vec<PathBuf> {
    vcx.cx
        .read(|cx| cx.global::<DialogEnv>().approved_roots.list())
}

fn open_error(vcx: &VisualTestContext, id: ProfileId, error: ConnectError) -> Entity<ErrorView> {
    let view = in_window(vcx, |window, cx| {
        open_error_dialog_view(id, Arc::new(error), window, cx)
    })
    .expect("the error dialog opens");
    vcx.run_until_parked();
    view
}

fn fix_labels(vcx: &VisualTestContext, view: &Entity<ErrorView>) -> Vec<String> {
    vcx.cx.read(|cx| {
        view.read(cx)
            .fix_buttons(cx)
            .into_iter()
            .map(|b| b.label.to_string())
            .collect()
    })
}

fn dialog(vcx: &VisualTestContext) -> Entity<ConnectionDialog> {
    in_window(vcx, |window, cx| open_dialog(window, cx)).expect("a connection dialog is open")
}

fn draft(vcx: &VisualTestContext) -> Profile {
    let d = dialog(vcx);
    vcx.cx.read(|cx| d.read(cx).profile().clone())
}

fn open_edit(vcx: &VisualTestContext, id: ProfileId) {
    in_window(vcx, |window, cx| {
        open_connection_dialog(DialogMode::Edit(id), window, cx);
    });
    vcx.run_until_parked();
}

/// Focuses the dialog's field `name` and types `text` after its value.
fn type_into(vcx: &mut VisualTestContext, name: &str, text: &str) {
    let d = dialog(vcx);
    let input = vcx
        .cx
        .read(|cx| d.read(cx).input(name))
        .unwrap_or_else(|| panic!("no field {name}"));
    let handle = vcx.cx.read(|cx| input.read(cx).focus_handle(cx));
    in_window(vcx, |window, cx| window.focus(&handle, cx));
    vcx.run_until_parked();
    vcx.simulate_input(text);
    vcx.run_until_parked();
}

/// The strip's finished report, once the dialog's test ended.
fn test_report(vcx: &VisualTestContext) -> Arc<hacivat_session::TestReport> {
    let d = dialog(vcx);
    let strip = vcx.cx.read(|cx| d.read(cx).strip());
    let mut out = None;
    settle_until(vcx, WAIT, || {
        out = match vcx.cx.read(|cx| strip.read(cx).state().clone()) {
            TestStripState::Done(report) => Some(report),
            _ => None,
        };
        out.is_some()
    });
    out.expect("a finished test")
}

/// A shell whose PostgreSQL driver refuses the first handshake as untrusted (presenting the
/// test CA) and accepts the next one over checked TLS, with the profile `orders` at 10.0.0.5
/// open for editing.
fn dialog_test_with_untrusted_server(
    cx: &mut TestAppContext,
) -> (&'static mut VisualTestContext, ProfileId) {
    let fake = FakeDriver::new(DbmsKind::Postgres)
        .script(vec![
            Err(RawError {
                kind: RawErrorKind::Tls(TlsFailure::UnknownIssuer),
                server: None,
            }),
            Ok(()),
        ])
        .presented(vec![TEST_CA_DER.to_vec()])
        .tls(TlsSummary {
            encrypted: true,
            version: Some("TLSv1.3".into()),
            cipher: None,
            verified: TlsVerified::Pinned,
        });
    let vcx = shell_with_fake(cx, "en", fake);
    let id = add_profile(vcx, test_profile("orders", Some(EnvTag::Dev), "10.0.0.5"));
    wait_until(vcx, "the profile to reach the file", |vcx| {
        connections_toml(vcx).contains("10.0.0.5")
    });
    open_edit(vcx, id);
    (vcx, id)
}

// ---- the certificate question (NOTES M2a surface 5) ---------------------------------------------

#[gpui::test]
fn certificate_question_opens_review_dialog(cx: &mut TestAppContext) {
    let vcx = shell_with_connections(cx, "en");
    let a = ask(vcx, cert_question(review_fixture("Müşteri DB", [0xAB; 32])));
    wait_layer(vcx, CERT_LAYER_ID);
    assert_eq!(width(vcx, CERT_LAYER_ID), 480.0);
    // The full fingerprint, two lines of 16 bytes: nothing elided.
    let lines = mono_lines(vcx, "cert-fingerprint");
    assert_eq!(lines.len(), 2);
    assert!(lines.iter().all(|l| l.len() == 47), "{lines:?}");
    assert_eq!(lines.join(":"), hex32(0xAB));
    // Server text, verbatim, outside any sentence.
    assert_eq!(
        text_of(vcx, "cert-subject").as_deref(),
        Some("CN=Müşteri DB,O=Acme")
    );
    assert_eq!(
        text_of(vcx, "cert-issuer").as_deref(),
        Some("CN=Acme Internal CA G2,O=Acme")
    );
    assert!(text_of(vcx, "cert-valid").is_some_and(|d| d.contains("2100")));
    assert_eq!(
        plain(text_of(vcx, "cert-title").expect("a title")),
        t(vcx, "hv-conn-tls-title")
    );
    assert_eq!(review(vcx, |r| r.primary_button()), "cert-pin");
    assert!(
        texts_drawn_by(vcx, "hacivat-ui/src/button.rs").contains(&t(vcx, "hv-conn-tls-pin-btn"))
    );
    assert_eq!(
        review(vcx, |r| r.buttons()),
        ["cert-copy", "cert-cancel", "cert-import", "cert-pin"]
    );
    assert!(a.borrow().is_none(), "nothing is answered before a choice");
}

/// PO question (default Cancel, carry-in): the review opens with Cancel focused, so Enter
/// cancels; Tab reaches Pin, and Enter there pins. Escape cancels.
#[gpui::test]
fn the_review_opens_on_cancel_and_enter_presses_the_focused_button(cx: &mut TestAppContext) {
    let vcx = shell_with_connections(cx, "en");
    let a = ask(vcx, cert_question(review_fixture("db", [1; 32])));
    wait_layer(vcx, CERT_LAYER_ID);
    assert_eq!(focused_button(vcx), Some("cert-cancel"));
    press(vcx, "enter");
    assert!(matches!(answered(vcx, &a), Answer::Cancelled));
    assert!(!layer_open(vcx, CERT_LAYER_ID));

    let b = ask(vcx, cert_question(review_fixture("db", [1; 32])));
    wait_layer(vcx, CERT_LAYER_ID);
    press(vcx, "tab");
    assert_eq!(focused_button(vcx), Some("cert-import"));
    press(vcx, "tab");
    assert_eq!(focused_button(vcx), Some("cert-pin"));
    press(vcx, "enter");
    assert!(matches!(answered(vcx, &b), Answer::Pin));
    assert!(!layer_open(vcx, CERT_LAYER_ID));

    let c = ask(vcx, cert_question(review_fixture("db", [1; 32])));
    wait_layer(vcx, CERT_LAYER_ID);
    press(vcx, "escape");
    assert!(matches!(answered(vcx, &c), Answer::Cancelled));
}

#[gpui::test]
fn clicking_pin_answers_pin(cx: &mut TestAppContext) {
    let vcx = shell_with_connections(cx, "en");
    let a = ask(vcx, cert_question(review_fixture("db", [1; 32])));
    wait_layer(vcx, CERT_LAYER_ID);
    click_button(vcx, "cert-pin");
    assert!(matches!(answered(vcx, &a), Answer::Pin));
}

#[gpui::test]
fn import_ca_answers_the_picked_path(cx: &mut TestAppContext) {
    let vcx = shell_with_connections(cx, "en");
    let picked = temp_dir(vcx).join("corp-ca.pem");
    let a = ask(vcx, cert_question(review_fixture("db", [1; 32])));
    wait_layer(vcx, CERT_LAYER_ID);
    click_button(vcx, "cert-import");
    assert!(
        vcx.cx.did_prompt_for_paths(),
        "Import CA opens the file picker"
    );
    let answer = picked.clone();
    vcx.cx
        .simulate_path_prompt_response(move |_| Some(vec![answer]));
    assert!(matches!(answered(vcx, &a), Answer::ImportCa(p) if p == picked));
    assert!(!layer_open(vcx, CERT_LAYER_ID));
}

#[gpui::test]
fn a_cancelled_picker_keeps_the_review_open(cx: &mut TestAppContext) {
    let vcx = shell_with_connections(cx, "en");
    let a = ask(vcx, cert_question(review_fixture("db", [1; 32])));
    wait_layer(vcx, CERT_LAYER_ID);
    click_button(vcx, "cert-import");
    vcx.cx.simulate_path_prompt_response(|_| None);
    settle(vcx, std::time::Duration::ZERO);
    assert!(layer_open(vcx, CERT_LAYER_ID));
    assert!(a.borrow().is_none());
}

#[gpui::test]
fn copy_copies_exactly_the_fingerprint(cx: &mut TestAppContext) {
    let vcx = shell_with_connections(cx, "en");
    let mut sha = [0u8; 32];
    for (i, b) in sha.iter_mut().enumerate() {
        *b = u8::try_from(i * 7).expect("a byte");
    }
    let _a = ask(vcx, cert_question(review_fixture("db", sha)));
    wait_layer(vcx, CERT_LAYER_ID);
    click_button(vcx, "cert-copy");
    let copied = vcx
        .cx
        .read_from_clipboard()
        .and_then(|c| c.text())
        .expect("the clipboard holds text");
    let expected = sha
        .iter()
        .map(|b| format!("{b:02X}"))
        .collect::<Vec<_>>()
        .join(":");
    assert_eq!(copied, expected);
    assert!(layer_open(vcx, CERT_LAYER_ID), "Copy answers nothing");
}

/// Carry-in (T14 review): a pin never extends a certificate's dates, so an expired or not yet
/// valid certificate is offered neither a pin nor a CA, and the review says why.
#[gpui::test]
fn a_certificate_outside_its_dates_is_never_offered_a_pin(cx: &mut TestAppContext) {
    let vcx = shell_with_connections(cx, "en");
    let mut expired = review_fixture("db", [1; 32]);
    expired.not_after_unix = NOT_BEFORE + 1;
    let a = ask(vcx, cert_question(expired));
    wait_layer(vcx, CERT_LAYER_ID);
    assert_eq!(review(vcx, |r| r.validity()), Validity::Expired);
    assert_eq!(review(vcx, |r| r.buttons()), ["cert-copy", "cert-cancel"]);
    assert_eq!(review(vcx, |r| r.primary_button()), "cert-cancel");
    assert!(vcx.debug_bounds("button-cert-pin").is_none());
    assert!(vcx.debug_bounds("button-cert-import").is_none());
    assert!(text_of(vcx, "cert-dates-note").is_some_and(|n| n.contains("expired")));
    // Tab never reaches a trust button; Enter cancels.
    press(vcx, "tab");
    press(vcx, "tab");
    press(vcx, "enter");
    assert!(matches!(answered(vcx, &a), Answer::Cancelled));

    let mut early = review_fixture("db", [1; 32]);
    early.not_before_unix = NOT_AFTER - 1;
    let _b = ask(vcx, cert_question(early));
    wait_layer(vcx, CERT_LAYER_ID);
    assert_eq!(review(vcx, |r| r.validity()), Validity::NotYetValid);
    assert_eq!(review(vcx, |r| r.buttons()), ["cert-copy", "cert-cancel"]);
}

// ---- the pin-changed hard stop (NOTES M2b surface 5) --------------------------------------------

/// A question can never replace a pin: the pipeline never asks on a changed pin (Q26), and the
/// hard stop's only Enter default is Cancel (Q34).
#[gpui::test]
fn pin_changed_dialog_has_cancel_as_default(cx: &mut TestAppContext) {
    let vcx = shell_with_connections(cx, "en");
    let a = ask(vcx, cert_question(pinned_review([1; 32], [2; 32])));
    wait_layer(vcx, CERT_LAYER_ID);
    assert!(review(vcx, |r| r.is_pin_changed()));
    assert_eq!(focused_button(vcx), Some("cert-cancel"));
    assert_eq!(review(vcx, |r| r.primary_button()), "cert-cancel");
    assert_eq!(
        review(vcx, |r| r.buttons()),
        ["cert-copy-old", "cert-copy", "cert-cancel"]
    );
    assert_eq!(mono_lines(vcx, "cert-fingerprint-old").join(":"), hex32(1));
    assert_eq!(mono_lines(vcx, "cert-fingerprint").join(":"), hex32(2));
    assert!(text_of(vcx, "cert-title").is_some_and(|t| plain(t).contains("127.0.0.1:5432")));
    press(vcx, "enter");
    assert!(!layer_open(vcx, CERT_LAYER_ID));
    assert!(matches!(answered(vcx, &a), Answer::Cancelled));
}

/// From the error (Q26): Replace pin is an outline button away from the default; Enter on it
/// still cancels (a hard stop), and only its own click writes `ReplacePin`.
#[gpui::test]
fn replace_pin_from_error_writes_replace_pin(cx: &mut TestAppContext) {
    let (vcx, id) = with_profile(cx, Some([1; 32]));
    let error = open_error(vcx, id, pin_changed_error([1; 32], [2; 32]));
    assert_eq!(fix_labels(vcx, &error), [t(vcx, "hv-conn-fix-review-cert")]);
    click_button(vcx, "fix-review-cert");
    wait_layer(vcx, CERT_LAYER_ID);
    assert_eq!(
        layer_ids(vcx),
        [CERT_LAYER_ID],
        "the error dialog closed first"
    );
    assert_eq!(
        review(vcx, |r| r.buttons()),
        [
            "cert-copy-old",
            "cert-copy",
            "cert-replace-pin",
            "cert-cancel"
        ]
    );
    assert_eq!(review(vcx, |r| r.primary_button()), "cert-cancel");
    assert_eq!(focused_button(vcx), Some("cert-cancel"));
    // Shift-Tab to Replace pin, then Enter: the hard stop cancels.
    press(vcx, "shift-tab");
    assert_eq!(focused_button(vcx), Some("cert-replace-pin"));
    press(vcx, "enter");
    assert!(!layer_open(vcx, CERT_LAYER_ID));
    assert_eq!(saved(vcx, id).tls.pins, [pin("127.0.0.1:5432", [1; 32])]);

    // Its own click replaces the pin, saves it, and connects again.
    open_error(vcx, id, pin_changed_error([1; 32], [2; 32]));
    click_button(vcx, "fix-review-cert");
    wait_layer(vcx, CERT_LAYER_ID);
    click_button(vcx, "cert-replace-pin");
    wait_until(vcx, "the new pin to reach the file", |vcx| {
        connections_toml(vcx).contains(&"02".repeat(32))
    });
    assert!(!connections_toml(vcx).contains(&"01".repeat(32)));
    assert_eq!(saved(vcx, id).tls.pins, [pin("127.0.0.1:5432", [2; 32])]);
    assert!(
        toast_texts(vcx)
            .iter()
            .any(|s| plain(s) == "Pinned the new certificate of 127.0.0.1"),
        "{:?}",
        toast_texts(vcx)
    );
    assert!(!matches!(
        live(vcx, "orders"),
        hacivat_session::LiveState::Disconnected
    ));
}

/// A replace whose old fingerprint is no longer the stored pin changes nothing (two reviews
/// open on one profile, or a stale error).
#[gpui::test]
fn a_stale_replace_pin_changes_nothing(cx: &mut TestAppContext) {
    let (vcx, id) = with_profile(cx, Some([3; 32]));
    open_error(vcx, id, pin_changed_error([1; 32], [2; 32]));
    click_button(vcx, "fix-review-cert");
    wait_layer(vcx, CERT_LAYER_ID);
    click_button(vcx, "cert-replace-pin");
    settle(vcx, std::time::Duration::ZERO);
    assert_eq!(saved(vcx, id).tls.pins, [pin("127.0.0.1:5432", [3; 32])]);
}

#[gpui::test]
fn review_certificate_fix_reopens_dialog_from_error(cx: &mut TestAppContext) {
    let (vcx, id) = with_profile(cx, None);
    open_error(vcx, id, untrusted_error_with_review());
    click_button(vcx, "fix-review-cert");
    wait_layer(vcx, CERT_LAYER_ID);
    assert!(!review(vcx, |r| r.is_pin_changed()));
    // Pin from the error: `AddPin` of the reviewed endpoint, saved, then a new connect.
    click_button(vcx, "cert-pin");
    wait_until(vcx, "the pin to reach the file", |vcx| {
        connections_toml(vcx).contains(&"ab".repeat(32))
    });
    assert_eq!(saved(vcx, id).tls.pins, [pin("127.0.0.1:5432", [0xAB; 32])]);
}

#[gpui::test]
fn import_ca_fix_sets_the_ca_file_and_approves_the_pick(cx: &mut TestAppContext) {
    let (vcx, id) = with_profile(cx, None);
    let picked = temp_dir(vcx).join("corp-ca.pem");
    write(&picked, TEST_CA_PEM);
    open_error(vcx, id, untrusted_error_with_review());
    click_button(vcx, "fix-import-ca");
    settle_until(vcx, WAIT, || vcx.cx.did_prompt_for_paths());
    let answer = picked.clone();
    vcx.cx
        .simulate_path_prompt_response(move |_| Some(vec![answer]));
    settle_until(vcx, WAIT, || {
        saved(vcx, id).tls.ca_file.as_deref() == Some(picked.as_path())
    });
    assert!(approved_roots(vcx).contains(&picked));
}

#[gpui::test]
fn encrypt_without_verifying_is_gone(cx: &mut TestAppContext) {
    let (vcx, id) = with_profile(cx, None);
    let view = open_error(vcx, id, untrusted_error_with_review());
    assert_eq!(
        fix_labels(vcx, &view),
        [
            t(vcx, "hv-conn-fix-review-cert"),
            t(vcx, "hv-conn-fix-import-ca"),
            t(vcx, "hv-conn-fix-edit"),
        ]
    );
    assert!(vcx.debug_bounds("button-fix-require").is_none());
    for file in ["locales/en/hacivat-ui.ftl", "locales/tr/hacivat-ui.ftl"] {
        let ids = message_ids_in(&workspace_file(file));
        assert!(
            !ids.iter().any(|id| id.starts_with("hv-conn-fix-require")),
            "{file}"
        );
    }
    // Without a certificate to review there is no Review certificate (NOTES M2b ruling 10).
    let mut bare = untrusted_error_with_review();
    bare.certificate = None;
    let view = open_error(vcx, id, bare);
    assert!(!fix_labels(vcx, &view).contains(&t(vcx, "hv-conn-fix-review-cert")));
}

/// The pipeline's narrowed list wins over the registry's (T14 review M1: an expired
/// certificate offers neither a pin nor a CA).
#[gpui::test]
fn an_expired_certificate_error_offers_only_edit(cx: &mut TestAppContext) {
    let (vcx, id) = with_profile(cx, None);
    let view = open_error(
        vcx,
        id,
        tls_error(ErrClass::TlsUntrusted, vec![FixId::EditConnection], None),
    );
    assert_eq!(fix_labels(vcx, &view), [t(vcx, "hv-conn-fix-edit")]);
}

// ---- the public-plaintext question ---------------------------------------------------------------

fn plaintext_question(profile: ProfileId) -> Question {
    Question::PlaintextPublic {
        profile,
        name: "orders".into(),
        host: "db.acme.io".into(),
        ip: "203.0.113.10".parse().ok(),
    }
}

fn plaintext_focus(vcx: &VisualTestContext) -> Option<&'static str> {
    let view = vcx
        .cx
        .read(plaintext_prompt::open_view)
        .expect("the question is open");
    in_window(vcx, |window, cx| view.read(cx).focused(window))
}

#[gpui::test]
fn plaintext_question_dialog(cx: &mut TestAppContext) {
    let (vcx, id) = with_profile(cx, None);
    let a = ask(vcx, plaintext_question(id));
    wait_layer(vcx, PLAINTEXT_LAYER_ID);
    assert_eq!(width(vcx, PLAINTEXT_LAYER_ID), 420.0);
    // The safe answer has the focus; plaintext is never the default.
    assert_eq!(plaintext_focus(vcx), Some("plain-ssh"));
    assert!(plain(text_of(vcx, "plain-title").expect("a title")).contains("db.acme.io"));
    assert!(plain(text_of(vcx, "plain-body").expect("a body")).contains("203.0.113.10"));
    click_selector(vcx, "checkbox-plain-remember");
    click_button(vcx, "plain-connect");
    assert!(matches!(
        answered(vcx, &a),
        Answer::ConnectPlaintext { remember: true }
    ));
    assert!(!layer_open(vcx, PLAINTEXT_LAYER_ID));

    let b = ask(vcx, plaintext_question(id));
    wait_layer(vcx, PLAINTEXT_LAYER_ID);
    click_button(vcx, "plain-connect");
    assert!(matches!(
        answered(vcx, &b),
        Answer::ConnectPlaintext { remember: false }
    ));

    let c = ask(vcx, plaintext_question(id));
    wait_layer(vcx, PLAINTEXT_LAYER_ID);
    press(vcx, "escape");
    assert!(matches!(answered(vcx, &c), Answer::Cancelled));
}

/// Enter answers the focused safe default: Use an SSH tunnel, which opens the SSH section.
#[gpui::test]
fn enter_uses_an_ssh_tunnel_and_opens_the_ssh_section(cx: &mut TestAppContext) {
    let (vcx, id) = with_profile(cx, None);
    let a = ask(vcx, plaintext_question(id));
    wait_layer(vcx, PLAINTEXT_LAYER_ID);
    press(vcx, "enter");
    assert!(matches!(answered(vcx, &a), Answer::UseSsh));
    wait_layer(vcx, hacivat_app::connections::dialog::DIALOG_LAYER_ID);
    assert!(!layer_open(vcx, PLAINTEXT_LAYER_ID));
}

// ---- the TLS section --------------------------------------------------------------------------

fn open_tls_section(vcx: &mut VisualTestContext) {
    click_selector(vcx, "disclosure-conn-tls");
}

#[gpui::test]
fn tls_section_pinned_row_and_remove_pin(cx: &mut TestAppContext) {
    let (vcx, id) = with_profile(cx, Some([0xAB; 32]));
    open_edit(vcx, id);
    open_tls_section(vcx);
    assert_eq!(
        text_of(vcx, "tls-pin-row").as_deref(),
        Some("AB:AB:AB:AB…AB:AB")
    );
    click_button(vcx, "tls-remove-pin");
    assert!(draft(vcx).tls.pins.is_empty());
    assert_eq!(
        text_of(vcx, "tls-pin-row").map(plain),
        Some(t(vcx, "hv-conn-tls-pin-none"))
    );
    // Nothing is saved until the dialog saves.
    assert_eq!(saved(vcx, id).tls.pins.len(), 1);
}

/// T3 ruling M2 (carry-in): Disable wins over a pin, and the TLS section says the pin is not
/// checked.
#[gpui::test]
fn disable_with_a_pin_shows_its_notice(cx: &mut TestAppContext) {
    let (vcx, id) = with_profile(cx, Some([0xAB; 32]));
    open_edit(vcx, id);
    open_tls_section(vcx);
    let notice = t(vcx, "hv-conn-issue-tls-disable-ignores-pin");
    let shown = |vcx: &VisualTestContext| {
        texts_drawn_by(vcx, "hacivat-ui/src/notice.rs")
            .iter()
            .any(|s| plain(s) == notice)
    };
    assert!(!shown(vcx));
    let d = dialog(vcx);
    let select = vcx.cx.read(|cx| d.read(cx).tls_select());
    let disable = 4; // TLS_MODES: Auto, Require, VerifyCa, VerifyFull, Disable
    vcx.cx.update(|cx| {
        select.update(cx, |s, cx| {
            s.set_selected(disable, cx);
            cx.emit(SelectEvent::Changed(disable));
        });
    });
    vcx.run_until_parked();
    assert_eq!(draft(vcx).tls.mode, TlsMode::Disable);
    assert!(shown(vcx));
    // A notice, never a refusal: the profile still saves and connects (B5).
    assert!(
        hacivat_session::pipeline::validate(&draft(vcx), hacivat_driver::EngineCatalog::embedded())
            .iter()
            .all(|i| i.code != IssueCode::TlsDisableIgnoresPin)
    );
}

#[gpui::test]
fn key_passphrase_field_visible_with_client_cert(cx: &mut TestAppContext) {
    let vcx = shell_with_connections(cx, "en");
    let mut p = test_profile("orders", Some(EnvTag::Dev), "127.0.0.1");
    p.auth = AuthProviderId::ClientCert;
    p.tls.client_cert = Some(ClientCertPaths {
        cert: PathBuf::from("/tmp/hv-client.crt"),
        key: PathBuf::from("/tmp/hv-client.key"),
    });
    let id = add_profile(vcx, p);
    open_edit(vcx, id);
    vcx.run_until_parked();
    assert!(vcx.debug_bounds("conn-field-key-pass").is_some());
    let d = dialog(vcx);
    let input = vcx
        .cx
        .read(|cx| d.read(cx).input("key-pass"))
        .expect("the passphrase field");
    assert!(vcx.cx.read(|cx| input.read(cx).is_masked()));
    // Remember sits beside it, as beside the password.
    assert!(vcx.debug_bounds("conn-field-password").is_none());
}

#[gpui::test]
fn mysql_verify_ca_enabled(cx: &mut TestAppContext) {
    let vcx = shell_with_connections(cx, "en");
    in_window(vcx, |window, cx| {
        open_connection_dialog(DialogMode::New, window, cx);
    });
    vcx.run_until_parked();
    for engine in [1, 2] {
        click_selector(vcx, &format!("segmented-conn-engine-{engine}"));
        let d = dialog(vcx);
        let select = vcx.cx.read(|cx| d.read(cx).tls_select());
        let verify_ca = 2; // TLS_MODES: Auto, Require, VerifyCa, …
        assert!(
            !vcx.cx
                .read(|cx| select.read(cx).options()[verify_ca].disabled)
        );
    }
}

/// Carry-in (T14 review I2): the remembered plaintext consent was given for one host and port.
#[gpui::test]
fn a_host_or_port_change_clears_allow_public_plaintext(cx: &mut TestAppContext) {
    for field in ["host", "port"] {
        let vcx = shell_with_connections(cx, "en");
        let mut p = test_profile("orders", Some(EnvTag::Dev), "203.0.113.10");
        p.tls.allow_public_plaintext = true;
        let id = add_profile(vcx, p);
        open_edit(vcx, id);
        assert!(draft(vcx).tls.allow_public_plaintext);
        // A change that is not the endpoint keeps it.
        type_into(vcx, "database", "x");
        assert!(draft(vcx).tls.allow_public_plaintext);
        type_into(vcx, field, "1");
        assert!(!draft(vcx).tls.allow_public_plaintext, "{field}");
    }
}

/// Carry-in (T20): a refused SSH key path is shown in the SSH section, never under a TLS
/// field.
#[cfg(unix)]
#[gpui::test]
fn a_refused_ssh_key_path_goes_to_the_ssh_section(cx: &mut TestAppContext) {
    use hacivat_net::ssh::{HopSpec, SshAuth, SshSpec};
    use hacivat_proto::{ImportSource, Origin};
    use hacivat_session::profile::TransportSpec;

    let vcx = shell_with_connections(cx, "en");
    let mut p = test_profile("orders", Some(EnvTag::Dev), "127.0.0.1");
    p.origin = Origin::Imported {
        source: ImportSource::PgService,
    };
    p.transport = TransportSpec::SshTunnel(SshSpec {
        hops: vec![HopSpec {
            host: "bastion.acme.io".into(),
            port: 22,
            user: "deploy".into(),
            auth: SshAuth::KeyFile {
                path: PathBuf::from("/opt/hv-not-approved/id_ed25519"),
            },
            identities_only: false,
            identity_files: Vec::new(),
        }],
        ..SshSpec::default()
    });
    let id = add_profile(vcx, p);
    open_edit(vcx, id);
    let d = dialog(vcx);
    assert!(run(vcx, "connection_dialog.save"));
    settle_until(vcx, WAIT, || {
        vcx.cx.read(|cx| {
            d.read(cx)
                .issues()
                .iter()
                .any(|i| i.code == IssueCode::PathRefused)
        })
    });
    vcx.run_until_parked();
    let refused = t(vcx, "hv-conn-issue-path-refused");
    assert_eq!(
        text_of(vcx, "ssh-issue-ssh-key-file-path-refused").map(plain),
        Some(refused.clone())
    );
    assert!(
        text_of(vcx, "issue-ca").is_none(),
        "never under the CA field"
    );
}

// ---- apply_changes and a Test's changes ---------------------------------------------------------

#[gpui::test]
fn apply_changes_persists_pin(cx: &mut TestAppContext) {
    let (vcx, id) = with_profile(cx, None);
    let other = add_profile(vcx, test_profile("reports", Some(EnvTag::Dev), "127.0.0.2"));
    // The relay subscribes on the core runtime: wait until it listens before publishing.
    settle_until(vcx, WAIT, || {
        vcx.cx.read(|cx| {
            cx.global::<ConnectionsModel>()
                .manager()
                .event_subscribers()
                > 0
        })
    });
    let manager = vcx
        .cx
        .read(|cx| cx.global::<ConnectionsModel>().manager().clone());
    manager.publish_event(ConnectionEvent::ProfileChanges {
        profile: id,
        changes: vec![ProfileChange::AddPin(pin("127.0.0.1:5432", [0xAB; 32]))],
    });
    wait_until(vcx, "the pin to reach the file", |vcx| {
        connections_toml(vcx).contains(&"ab".repeat(32))
    });
    assert_eq!(saved(vcx, id).tls.pins.len(), 1);
    assert!(
        saved(vcx, other).tls.pins.is_empty(),
        "only that profile changes"
    );
    // A second AddPin for the endpoint never replaces the pin (Q26: only Replace pin does).
    let task = vcx.cx.update(|cx| {
        ConnectionsModel::apply_changes(
            cx,
            id,
            vec![ProfileChange::AddPin(pin("127.0.0.1:5432", [0xCD; 32]))],
        )
    });
    assert!(wait_task(vcx, task).is_ok(), "nothing to write is no error");
    assert_eq!(saved(vcx, id).tls.pins, [pin("127.0.0.1:5432", [0xAB; 32])]);
    assert!(!connections_toml(vcx).contains(&"cd".repeat(32)));
}

#[gpui::test]
fn apply_changes_sets_the_ca_and_the_consent(cx: &mut TestAppContext) {
    let (vcx, id) = with_profile(cx, None);
    let ca = temp_dir(vcx).join("ca.pem");
    let task = vcx.cx.update(|cx| {
        ConnectionsModel::apply_changes(
            cx,
            id,
            vec![
                ProfileChange::SetCaFile(ca.clone()),
                ProfileChange::AllowPublicPlaintext,
            ],
        )
    });
    wait_task(vcx, task).expect("the changes are saved");
    let p = saved(vcx, id);
    assert_eq!(p.tls.ca_file.as_deref(), Some(ca.as_path()));
    assert!(p.tls.allow_public_plaintext);
    assert!(connections_toml(vcx).contains("allow_public_plaintext = true"));
}

/// T14 review M3 (carry-in): a Test's Pin goes to the dialog's draft only.
#[gpui::test]
fn test_dialog_pin_updates_draft_only(cx: &mut TestAppContext) {
    let (vcx, id) = dialog_test_with_untrusted_server(cx);
    click_button(vcx, "conn-test");
    wait_layer(vcx, CERT_LAYER_ID);
    click_button(vcx, "cert-pin");
    let report = test_report(vcx);
    assert!(report.ok, "the pinned retry connects: {:?}", report.error);
    assert_eq!(draft(vcx).tls.pins.len(), 1);
    assert_eq!(draft(vcx).tls.pins[0].endpoint, "10.0.0.5:5432");
    assert!(saved(vcx, id).tls.pins.is_empty());
    assert!(!connections_toml(vcx).contains("pins"));
}

/// T14 review M3 (carry-in): a Test's Import CA approves nothing app-wide; the draft gets the
/// file, and saving the profile that names it approves it.
#[gpui::test]
fn a_tests_import_ca_approves_nothing_app_wide(cx: &mut TestAppContext) {
    let (vcx, id) = dialog_test_with_untrusted_server(cx);
    let picked = temp_dir(vcx).join("corp-ca.pem");
    write(&picked, TEST_CA_PEM);
    click_button(vcx, "conn-test");
    wait_layer(vcx, CERT_LAYER_ID);
    click_button(vcx, "cert-import");
    let answer = picked.clone();
    vcx.cx
        .simulate_path_prompt_response(move |_| Some(vec![answer]));
    let report = test_report(vcx);
    assert!(
        report.ok,
        "the retry with the CA connects: {:?}",
        report.error
    );
    assert_eq!(draft(vcx).tls.ca_file.as_deref(), Some(picked.as_path()));
    assert!(!approved_roots(vcx).contains(&picked));
    assert!(saved(vcx, id).tls.ca_file.is_none());
    assert!(run(vcx, "connection_dialog.save"));
    settle_until(vcx, WAIT, || {
        saved(vcx, id).tls.ca_file.as_deref() == Some(picked.as_path())
    });
    assert!(approved_roots(vcx).contains(&picked));
}

// ---- long locales -----------------------------------------------------------------------------

/// Turkish (and the pseudo-locale) never clip the review or the question: the sentences wrap,
/// the buttons stay inside the card.
#[cfg(feature = "scene-check")]
fn assert_fits(vcx: &mut VisualTestContext, layer: &'static str, buttons: &[&str]) {
    vcx.run_until_parked();
    let card = vcx.debug_bounds(layer).expect("the card is drawn");
    for b in buttons {
        let sel: &'static str = Box::leak(format!("button-{b}").into_boxed_str());
        let bounds = vcx
            .debug_bounds(sel)
            .unwrap_or_else(|| panic!("{b} is drawn"));
        assert!(
            bounds.left() >= card.left() && bounds.right() <= card.right(),
            "{b} runs past the card"
        );
    }
    let cut: Vec<String> = registry_frame(vcx)
        .into_iter()
        .filter(|r| {
            r.path.contains("connections/cert_dialog.rs")
                || r.path.contains("connections/plaintext_prompt.rs")
        })
        .filter(|r| r.clipped || r.truncated)
        .map(|r| r.text.to_string())
        .collect();
    assert!(cut.is_empty(), "clipped: {cut:?}");
}

#[cfg(feature = "scene-check")]
fn long_locale_surfaces(cx: &mut TestAppContext, lang: &str) {
    let vcx = shell_with_connections(cx, lang);
    vcx.simulate_resize(gpui::size(gpui::px(960.0), gpui::px(600.0)));
    let mut p = test_profile("orders", Some(EnvTag::Dev), "127.0.0.1");
    p.tls.pins = vec![pin("127.0.0.1:5432", [1; 32])];
    let id = add_profile(vcx, p);

    let a = ask(vcx, cert_question(review_fixture("db", [0xAB; 32])));
    wait_layer(vcx, CERT_LAYER_ID);
    assert_fits(
        vcx,
        CERT_LAYER_ID,
        &["cert-cancel", "cert-import", "cert-pin"],
    );
    press(vcx, "escape");
    let _ = answered(vcx, &a);

    open_error(vcx, id, pin_changed_error([1; 32], [2; 32]));
    click_button(vcx, "fix-review-cert");
    wait_layer(vcx, CERT_LAYER_ID);
    assert_fits(vcx, CERT_LAYER_ID, &["cert-replace-pin", "cert-cancel"]);
    press(vcx, "escape");

    let b = ask(vcx, plaintext_question(id));
    wait_layer(vcx, PLAINTEXT_LAYER_ID);
    assert_fits(
        vcx,
        PLAINTEXT_LAYER_ID,
        &["plain-cancel", "plain-connect", "plain-ssh"],
    );
    press(vcx, "escape");
    let _ = answered(vcx, &b);
}

#[cfg(feature = "scene-check")]
#[gpui::test]
fn turkish_fits_the_review_and_the_question(cx: &mut TestAppContext) {
    long_locale_surfaces(cx, "tr");
}

#[cfg(all(feature = "scene-check", feature = "pseudo"))]
#[gpui::test]
fn the_pseudo_locale_fits_the_review_and_the_question(cx: &mut TestAppContext) {
    long_locale_surfaces(cx, "tr-XT");
}
