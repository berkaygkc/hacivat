//! M2b flows across surfaces (Task 24): the certificate review into a pinned connection; an SSH
//! connect through a first-seen bastion into the capsule's SSH path; a reconnect into the
//! session-lost toast and the sheet; the first Forever save on a machine without a keyring into
//! a new vault and a live connection; and the "Use an SSH tunnel" answer to the plaintext
//! question, through the SSH section, end to end.
//!
//! Databases are `FakeDriver`s that dial themselves (no socket to a database; hosts are IP
//! literals, so no DNS query). SSH is the testkit's in-process server on loopback, started on
//! the app's own core runtime. Each prompt is answered from the keyboard where the flow allows
//! (the review probe: every new dialog completes from the keyboard alone). Every wait is a
//! condition (`settle_until`, `wait_until`), never a fixed sleep (Q4); real sockets set the pace
//! of the SSH tests, so none uses a paused clock (Q25).

mod support;

use std::cell::RefCell;
use std::net::SocketAddr;
use std::rc::Rc;
use std::sync::Arc;

use gpui::{EntityInputHandler as _, Focusable as _, TestAppContext, VisualTestContext};
use hacivat_app::connections::dialog::ssh::{AuthKind, section_state};
use hacivat_app::connections::dialog::{DialogMode, open_connection_dialog, open_dialog};
use hacivat_app::connections::error_view::open_error_dialog;
use hacivat_app::connections::interaction::interaction;
use hacivat_app::connections::prompt_kit::parts;
use hacivat_app::connections::{ConnectionsModel, cert_dialog};
use hacivat_app::l10n_global::tr_id;
use hacivat_driver::fake::FakeDriver;
use hacivat_driver::{RawError, RawErrorKind};
use hacivat_net::ssh::{HopSpec, HostKeyPolicy, SshAuth, SshSpec};
use hacivat_net::{
    CertPin, CertReview, ChainEntry, ChainRole, TlsFailure, TlsMode, TlsSummary, TlsVerified,
};
use hacivat_proto::{DbmsKind, EnvTag, ErrClass, FixId, ProfileId};
use hacivat_secrets::{
    BackendChoice, ExposeSecret, FallbackBackend, FileVault, MemoryBackend, Retention,
    SecretBackend as _, SecretKey, SecretSlot, VaultBackend as _, VaultParams,
};
use hacivat_session::pipeline::{Answer, ConnectError, ErrArgs, Interaction, Phase, Question};
use hacivat_session::profile::{Profile, TransportSpec};
use hacivat_session::{LiveState, PathKind};
use hacivat_testkit::ssh_server::{ForwardTarget, TestSshConfig, TestSshServer};
use hacivat_ui::capsule::CapsuleModel;
use hacivat_ui::select::SelectEvent;

use support::*;

/// The test CA (a self-signed certificate) as DER: the chain the untrusted server presents.
const TEST_CA_DER: &[u8] =
    include_bytes!("../../../core/hacivat-session/tests/fixtures/test-ca.der");

/// The SSH user's password on the test server; asserted never to reach a file or the screen.
const SSH_PW: &str = "pw-for-the-test-server";
/// The vault's master password in the vault flow.
const MASTER: &str = "correcthorse";
/// The database password the vault keeps.
const DB_PW: &str = "pw-kept-in-the-vault";

// ---- reading and driving ------------------------------------------------------------------------

/// `s` without Fluent's isolation marks.
fn plain(s: impl AsRef<str>) -> String {
    s.as_ref()
        .chars()
        .filter(|c| !matches!(c, '\u{2068}' | '\u{2069}'))
        .collect()
}

fn t(vcx: &VisualTestContext, id: &str) -> String {
    plain(vcx.cx.read(|cx| tr_id(cx, id)))
}

fn layer_open(vcx: &VisualTestContext, id: &str) -> bool {
    vcx.run_until_parked();
    layer_ids(vcx).contains(&id)
}

/// Presses each of `keys` down and up, as a keyboard sends it (GPUI turns a clean down-and-up of
/// Enter or Space on a focused button into a click, so the key-up phase is exercised too).
fn press(vcx: &mut VisualTestContext, keys: &'static str) {
    for key in keys.split(' ') {
        let Ok(keystroke) = gpui::Keystroke::parse(key) else {
            panic!("not a keystroke: {key}");
        };
        vcx.simulate_keystrokes(key);
        vcx.simulate_event(gpui::KeyUpEvent { keystroke });
        vcx.run_until_parked();
    }
}

/// The registered name of the focused prompt part.
fn focused_part(vcx: &VisualTestContext) -> String {
    vcx.run_until_parked();
    in_window(vcx, |window, cx| parts::focused(window, cx)).unwrap_or_default()
}

fn click_selector(vcx: &mut VisualTestContext, selector: &str) {
    vcx.run_until_parked();
    let selector: &'static str = Box::leak(selector.to_string().into_boxed_str());
    click(vcx, selector);
}

/// The focused button of the open certificate review.
fn review_focus(vcx: &VisualTestContext) -> Option<&'static str> {
    let view = vcx
        .cx
        .read(cert_dialog::open_view)
        .expect("a certificate review is open");
    in_window(vcx, |window, cx| view.read(cx).focused_button(window))
}

/// The text of the capsule's `segment` label (`capsule-text-<segment>`).
fn capsule_text(vcx: &VisualTestContext, segment: &str) -> String {
    plain(
        text_of(vcx, &format!("capsule-text-{segment}"))
            .unwrap_or_else(|| panic!("the capsule draws its `{segment}` text")),
    )
}

/// The status bar's label `status-<segment>`.
fn status_text(vcx: &VisualTestContext, segment: &str) -> String {
    plain(
        text_of(vcx, &format!("status-{segment}"))
            .unwrap_or_else(|| panic!("the status bar draws its `{segment}` text")),
    )
}

fn saved(vcx: &VisualTestContext, id: ProfileId) -> Arc<Profile> {
    vcx.cx
        .read(|cx| cx.global::<ConnectionsModel>().get(id))
        .expect("the profile is saved")
}

fn connections_toml(vcx: &VisualTestContext) -> String {
    std::fs::read_to_string(paths(vcx).connections_toml()).unwrap_or_default()
}

/// Hacivat's own `known_hosts` (the data folder's), as the core runtime wrote it.
fn known_hosts_file(vcx: &VisualTestContext) -> String {
    std::fs::read_to_string(paths(vcx).data.join("known_hosts")).unwrap_or_default()
}

/// Every text the frame drew, joined.
fn frame_text(vcx: &VisualTestContext) -> String {
    registry_frame(vcx)
        .into_iter()
        .map(|r| r.text.to_string())
        .collect::<Vec<_>>()
        .join("\n")
}

// ---- SSH ----------------------------------------------------------------------------------------

/// A database stand-in behind the bastion: it accepts and holds every connection.
fn db_sink(rt: &tokio::runtime::Handle) -> SocketAddr {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind the sink");
    listener.set_nonblocking(true).expect("a non-blocking sink");
    let addr = listener.local_addr().expect("the sink's address");
    rt.spawn(async move {
        let listener = tokio::net::TcpListener::from_std(listener).expect("on the runtime");
        let mut held = Vec::new();
        while let Ok((stream, _)) = listener.accept().await {
            held.push(stream);
        }
    });
    addr
}

/// The in-process SSH server on the app's core runtime: user `deploy` signs in with
/// [`SSH_PW`], and every forwarded channel goes to a held local socket.
fn start_ssh(vcx: &VisualTestContext) -> TestSshServer {
    let rt = vcx.cx.read(|cx| cx.global::<ConnectionsModel>().runtime());
    let sink = db_sink(&rt);
    let _on_runtime = rt.enter();
    TestSshServer::start(
        TestSshConfig::default()
            .user_pw("deploy", SSH_PW)
            .forward_to(ForwardTarget::Redirect(sink)),
    )
}

/// The one hop through `server`, signing in with a password the user types.
fn hop_to(server: &TestSshServer) -> HopSpec {
    HopSpec {
        host: server.host(),
        port: server.addr.port(),
        user: "deploy".to_string(),
        auth: SshAuth::Password,
        identities_only: false,
        identity_files: Vec::new(),
    }
}

/// Answers the new-host-key prompt from the keyboard: Cancel has the focus, Tab reaches Trust
/// and connect, Space presses it.
fn trust_host_key_by_keyboard(vcx: &mut VisualTestContext) {
    assert_eq!(
        focused_part(vcx),
        "hk-cancel",
        "the safe answer has the focus"
    );
    press(vcx, "tab");
    assert_eq!(focused_part(vcx), "hk-trust");
    press(vcx, "space");
}

// ---- 1 · the certificate review into a pinned connection ---------------------------------------

/// A server that refuses the first handshake as untrusted (presenting the test CA) and accepts
/// the pinned one. The review opens over the connecting capsule, Pin is reached from the
/// keyboard, and the pin is exactly the certificate the review showed: the retry carries it,
/// the profile and its file keep it. The connection then reads "TLS 1.3 · pinned" in the status
/// bar and the sheet (fix round 1, I3; M2a NOTES `enc-pinned`), and the next connect carries
/// the pin without asking again.
#[gpui::test]
fn untrusted_then_pin_then_capsule_shows_pinned(cx: &mut TestAppContext) {
    let driver = FakeDriver::new(DbmsKind::Postgres)
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
    let vcx = shell_with_fake(cx, "en", driver);
    let id = add_profile(
        vcx,
        test_profile("reports-qa", Some(EnvTag::Test), "10.0.0.5"),
    );
    connect(vcx, "reports-qa");

    // The pipeline's question opens the review over the connecting capsule.
    settle_until(vcx, WAIT, || layer_open(vcx, "cert-dialog"));
    assert!(matches!(live(vcx, "reports-qa"), LiveState::Connecting(_)));
    assert!(matches!(
        capsule_model(vcx),
        CapsuleModel::Connecting { .. }
    ));
    // The fingerprint the review shows (two lines of 16 bytes).
    let reviewed = (1..=2)
        .map(|n| text_of(vcx, &format!("cert-fingerprint-line-{n}")).expect("a line"))
        .collect::<Vec<_>>()
        .join(":");
    // Keyboard alone: Cancel has the focus; Tab, Tab reaches Pin; Enter presses it.
    assert_eq!(review_focus(vcx), Some("cert-cancel"));
    press(vcx, "tab tab");
    assert_eq!(review_focus(vcx), Some("cert-pin"));
    press(vcx, "enter");

    settle_until(vcx, WAIT, || {
        matches!(live(vcx, "reports-qa"), LiveState::Connected(_))
    });
    assert!(!layer_open(vcx, "cert-dialog"));

    // What Pin did: the profile keeps exactly the reviewed certificate for this endpoint, and
    // the retry the pipeline ran after the answer carried that pin.
    settle_until(vcx, WAIT, || saved(vcx, id).tls.pins.len() == 1);
    let pin = saved(vcx, id).tls.pins[0].clone();
    assert_eq!(pin.endpoint, "10.0.0.5:5432");
    let stored = pin
        .sha256
        .iter()
        .map(|b| format!("{b:02X}"))
        .collect::<Vec<_>>()
        .join(":");
    assert_eq!(
        stored, reviewed,
        "the pin is the certificate the review showed"
    );
    let retry = fake(vcx).requests()[1]
        .tls
        .clone()
        .expect("the retry uses TLS");
    assert_eq!(retry.settings.pins, std::slice::from_ref(&pin));
    wait_until(vcx, "the pin to reach the file", |vcx| {
        connections_toml(vcx).contains(&stored.replace(':', "").to_ascii_lowercase())
    });

    // The capsule shows the live connection, encrypted (no "unencrypted" segment); the status
    // bar and the sheet say how: pinned.
    assert!(matches!(
        capsule_model(vcx),
        CapsuleModel::Connection {
            unencrypted: None,
            ..
        }
    ));
    vcx.cx.update(|cx| ConnectionsModel::select(cx, Some(id)));
    vcx.run_until_parked();
    let status = plain(status_bar_text(vcx));
    assert!(status.contains("TLS 1.3 · pinned"), "{status}");
    assert!(!status.contains("verified"), "{status}");
    assert!(
        texts_drawn_by(vcx, "connections/sheet.rs")
            .iter()
            .any(|s| plain(s) == "TLS 1.3 · pinned"),
        "{:?}",
        texts_drawn_by(vcx, "connections/sheet.rs")
    );

    // The next connect carries the pin and asks nothing.
    vcx.cx.update(|cx| ConnectionsModel::disconnect(cx, id));
    settle_until(vcx, WAIT, || {
        matches!(live(vcx, "reports-qa"), LiveState::Disconnected)
    });
    connect(vcx, "reports-qa");
    settle_until(vcx, WAIT, || {
        matches!(live(vcx, "reports-qa"), LiveState::Connected(_))
    });
    assert!(
        !layer_open(vcx, "cert-dialog"),
        "a pinned endpoint is not asked again"
    );
    let sent = fake(vcx).last_request().and_then(|r| r.tls).expect("TLS");
    assert!(
        sent.settings.pins.contains(&pin),
        "{:?}",
        sent.settings.pins
    );
    assert_eq!(fake(vcx).connect_calls(), 3);
}

// ---- 2 · a first-seen bastion into the capsule's SSH path ---------------------------------------

/// A connect through the in-process SSH server: the new-host-key prompt names the bastion and
/// its key and is trusted from the keyboard; the SSH password prompt names the hop and is
/// answered with Enter; the connection lands in the capsule, the status bar and the explorer as
/// "SSH via" the bastion, and the key in Hacivat's own `known_hosts`. The password reaches no
/// file and no frame.
#[gpui::test]
fn ssh_connect_with_new_host_key_then_capsule_via_bastion(cx: &mut TestAppContext) {
    let vcx = shell_with_fake(cx, "en", FakeDriver::new(DbmsKind::Postgres));
    let server = start_ssh(vcx);
    let mut p = test_profile("orders-staging", Some(EnvTag::Staging), "10.0.0.5");
    p.transport = TransportSpec::SshTunnel(SshSpec {
        hops: vec![hop_to(&server)],
        host_keys: HostKeyPolicy::Ask,
        keepalive_s: None,
    });
    let id = add_profile(vcx, p);
    vcx.cx.update(|cx| ConnectionsModel::select(cx, Some(id)));
    connect(vcx, "orders-staging");

    // The host key prompt, over the capsule's SSH step.
    settle_until(vcx, WAIT, || layer_open(vcx, "hostkey-prompt"));
    let CapsuleModel::Connecting { step, .. } = capsule_model(vcx) else {
        panic!("connecting");
    };
    assert_eq!(step.as_ref(), "SSH");
    assert_eq!(
        plain(text_of(vcx, "hk-fingerprint").expect("the fingerprint")),
        server.host_key.fingerprint()
    );
    assert!(plain(text_of(vcx, "hk-title").expect("a title")).contains(&server.host()));
    trust_host_key_by_keyboard(vcx);

    // The SSH password prompt for that hop; Enter continues.
    settle_until(vcx, WAIT, || layer_open(vcx, "secret-prompt"));
    assert_eq!(focused_part(vcx), "secret-input");
    vcx.simulate_input(SSH_PW);
    press(vcx, "enter");

    settle_until(vcx, WAIT, || {
        matches!(live(vcx, "orders-staging"), LiveState::Connected(_))
    });
    let LiveState::Connected(info) = live(vcx, "orders-staging") else {
        panic!("connected");
    };
    assert_eq!(info.path, PathKind::Ssh);
    let via = format!("SSH via {}", server.host());
    assert_eq!(capsule_text(vcx, "path"), via);
    assert_eq!(status_text(vcx, "path"), via);
    assert!(vcx.debug_bounds("explorer-row-route-0").is_some());
    assert!(server.sessions_accepted() >= 1);

    // Trusted: the key is in Hacivat's store, never in `~/.ssh`.
    let key_body = server
        .host_key
        .openssh
        .split_whitespace()
        .nth(1)
        .expect("a key body")
        .to_string();
    wait_until(vcx, "the trusted key to reach known_hosts", |vcx| {
        known_hosts_file(vcx).contains(&key_body)
    });
    // The typed password reached neither a file nor the screen.
    assert!(!connections_toml(vcx).contains(SSH_PW));
    assert!(!known_hosts_file(vcx).contains(SSH_PW));
    assert!(!frame_text(vcx).contains(SSH_PW));
}

// ---- 3 · a reconnect into the session-lost toast -----------------------------------------------

/// A refused connection error: a reconnect case (`network.refused`).
fn refused() -> RawError {
    RawError {
        kind: RawErrorKind::Io(std::io::ErrorKind::ConnectionRefused),
        server: None,
    }
}

/// End to end over the M2b manager (boot's `ManagerConfig::m2b()`): a 1 s keep-alive finds the
/// dead connection; the capsule and the status bar both count the attempts; the second attempt
/// connects; the session-lost toast says so, and its Show details selects the connection,
/// whose sheet then shows it.
#[gpui::test]
fn reconnect_flow_shows_reconnecting_then_lost_toast(cx: &mut TestAppContext) {
    let mut p = test_profile("orders-dev", Some(EnvTag::Dev), "10.0.4.12");
    p.keepalive_s = Some(1);
    let driver = FakeDriver::new(DbmsKind::Postgres).script(vec![Ok(()), Err(refused()), Ok(())]);
    let vcx = shell_with_fake(cx, "en", driver);
    let id = add_profile(vcx, p);
    let other = add_profile(vcx, test_profile("other", Some(EnvTag::Dev), "10.0.4.13"));
    vcx.cx.update(|cx| ConnectionsModel::select(cx, Some(id)));
    connect(vcx, "orders-dev");
    wait_until(vcx, "the first connect", |vcx| {
        matches!(live(vcx, "orders-dev"), LiveState::Connected(_))
    });

    fake(vcx).kill_connections();
    wait_until(vcx, "the countdown to attempt 2", |vcx| {
        matches!(
            live(vcx, "orders-dev"),
            LiveState::Reconnecting {
                attempt: 2,
                next_in: Some(_)
            }
        )
    });
    // The capsule and the status bar say the same thing.
    assert_eq!(capsule_text(vcx, "run"), "Reconnecting · attempt 2");
    assert!(
        plain(status_bar_text(vcx)).contains("Reconnecting · attempt 2"),
        "{}",
        status_bar_text(vcx)
    );

    wait_until(vcx, "the reconnect", |vcx| {
        matches!(live(vcx, "orders-dev"), LiveState::Connected(_))
    });
    settle_until(vcx, WAIT, || {
        vcx.cx
            .read(hacivat_app::connections::session_toasts::shown)
            .iter()
            .any(|s| s.sticky)
    });
    let toast = vcx
        .cx
        .read(hacivat_app::connections::session_toasts::shown)
        .into_iter()
        .rev()
        .find(|s| s.sticky)
        .expect("the session-lost toast");
    assert_eq!(
        toast.title.as_deref().map(plain).as_deref(),
        Some("Reconnected to orders-dev")
    );
    assert_eq!(fake(vcx).connect_calls(), 3);

    // Show details selects the connection: its sheet shows it.
    vcx.cx
        .update(|cx| ConnectionsModel::select(cx, Some(other)));
    vcx.run_until_parked();
    let toast_id = vcx.cx.read(|cx| {
        *hacivat_ui::toast::ToastQueue::visible_ids(cx)
            .last()
            .expect("a toast")
    });
    click_selector(vcx, &format!("button-toast-action-{toast_id}-0"));
    assert_eq!(
        vcx.cx.read(|cx| cx.global::<ConnectionsModel>().selected()),
        Some(id)
    );
    assert!(
        texts_drawn_by(vcx, "connections/sheet.rs")
            .iter()
            .any(|s| plain(s) == "orders-dev"),
        "{:?}",
        texts_drawn_by(vcx, "connections/sheet.rs")
    );
}

// ---- 4 · the first Forever save on a vault machine ----------------------------------------------

/// The connection dialog's field `name` (`host`, `user`, `password`, …).
fn type_into_form(vcx: &mut VisualTestContext, name: &str, text: &str) {
    let d = in_window(vcx, |window, cx| open_dialog(window, cx)).expect("the dialog");
    let input = vcx
        .cx
        .read(|cx| d.read(cx).input(name))
        .unwrap_or_else(|| panic!("no form field {name}"));
    let handle = vcx.cx.read(|cx| input.focus_handle(cx));
    in_window(vcx, |window, cx| window.focus(&handle, cx));
    vcx.simulate_input(text);
    vcx.run_until_parked();
}

/// A machine without a keyring (B13): the connection dialog's Remember reads "Forever
/// (encrypted file)"; Save and connect asks to create the vault, which is completed from the
/// keyboard; the password lands in the new vault, the connection uses it, and the capsule shows
/// it connected.
#[gpui::test]
fn first_forever_save_on_vault_machine_creates_vault(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().expect("a temp dir");
    let vault = Arc::new(FileVault::with_params(
        dir.path().join("vault.bin"),
        VaultParams {
            m_kib: 64,
            t: 1,
            p: 1,
        },
    ));
    // The keyring probe says "unusable" (never the platform keyring, Q34): Forever goes to the
    // vault, which does not exist yet.
    let backend = FallbackBackend::new(
        Arc::new(MemoryBackend::new()),
        vault.clone(),
        Box::new(|| false),
    );
    let vcx = shell_with_fake_backend(
        cx,
        "en",
        FakeDriver::new(DbmsKind::Postgres),
        Arc::new(backend),
        |_| {},
    );
    let secrets = vcx
        .cx
        .read(|cx| cx.global::<ConnectionsModel>().secrets().clone());
    in_window(vcx, |window, cx| {
        open_connection_dialog(DialogMode::New, window, cx)
    });
    // The dialog's keyring probe chooses the vault on the core runtime (P36).
    settle_until(vcx, WAIT, || {
        secrets.backend_choice() == Some(BackendChoice::Vault)
    });
    type_into_form(vcx, "name", "orders-dev");
    type_into_form(vcx, "host", "127.0.0.1");
    type_into_form(vcx, "user", "deploy");
    type_into_form(vcx, "password", DB_PW);
    // Remember: Forever, which says where it keeps the password here.
    let d = in_window(vcx, |window, cx| open_dialog(window, cx)).expect("the dialog");
    let select = vcx.cx.read(|cx| d.read(cx).remember_select());
    let at = Retention::ALL
        .iter()
        .position(|r| *r == Retention::Forever)
        .expect("Forever");
    assert_eq!(
        plain(vcx.cx.read(|cx| (select.read(cx).options()[at].label)(cx))),
        t(vcx, "hv-conn-ret-forever-vault")
    );
    vcx.cx.update(|cx| {
        select.update(cx, |s, cx| {
            s.set_selected(at, cx);
            cx.emit(SelectEvent::Changed(at));
        });
    });
    vcx.run_until_parked();
    assert!(!vault.exists(), "nothing is created before the user asks");

    // Save and connect asks to create the vault first; completed from the keyboard.
    vcx.simulate_keystrokes(if cfg!(target_os = "macos") {
        "cmd-enter"
    } else {
        "ctrl-enter"
    });
    settle_until(vcx, WAIT, || layer_open(vcx, "vault-dialog"));
    assert_eq!(focused_part(vcx), "vault-new");
    vcx.simulate_input(MASTER);
    press(vcx, "enter");
    assert_eq!(focused_part(vcx), "vault-repeat");
    vcx.simulate_input(MASTER);
    press(vcx, "enter");

    settle_until(vcx, WAIT, || {
        vcx.cx
            .read(|cx| cx.global::<ConnectionsModel>().profiles().len() == 1)
            && matches!(live(vcx, "orders-dev"), LiveState::Connected(_))
    });
    assert!(layer_ids(vcx).is_empty(), "{:?}", layer_ids(vcx));
    let id = profile_id(vcx, "orders-dev");
    assert!(vault.exists());
    let in_vault = vault
        .get(&SecretKey {
            profile: id,
            slot: SecretSlot::DbPassword,
        })
        .expect("readable")
        .is_some_and(|s| s.expose_secret() == DB_PW);
    assert!(in_vault, "the password landed in the new vault");
    let used = fake(vcx).last_request().and_then(|r| r.password);
    assert!(used.as_deref() == Some(DB_PW), "the connect used it");
    assert!(
        toast_texts(vcx).contains(&t(vcx, "hv-conn-vault-created")),
        "{:?}",
        toast_texts(vcx)
    );
    assert!(matches!(
        capsule_model(vcx),
        CapsuleModel::Connection { .. }
    ));
    // Fix round 1 (c): the sheet says where the password is kept, the vault (NOTES M2b
    // `vault-secret`), never the Keychain.
    vcx.cx.update(|cx| ConnectionsModel::select(cx, Some(id)));
    vcx.run_until_parked();
    let sheet = texts_drawn_by(vcx, "connections/sheet.rs");
    assert!(
        sheet
            .iter()
            .any(|s| plain(s) == t(vcx, "hv-conn-secret-vault")),
        "{sheet:?}"
    );
    assert!(
        !sheet
            .iter()
            .any(|s| plain(s) == t(vcx, "hv-conn-secret-forever")),
        "{sheet:?}"
    );
    // Neither secret reached a file or the screen.
    for secret in [DB_PW, MASTER] {
        assert!(!connections_toml(vcx).contains(secret));
        assert!(!frame_text(vcx).contains(secret));
    }
}

// ---- 5 · "Connect through SSH…" end to end --------------------------------------------------------

/// The SSH section the fix or the answer opened: on, open, the SSH host focused, and, at the
/// 960x600 minimum, the SSH host field inside the dialog body's visible part (fix round 1, I2:
/// the body follows the focus).
fn assert_tunnel_section_landed(vcx: &mut VisualTestContext) {
    settle_until(vcx, WAIT, || {
        in_window(vcx, |window, cx| section_state(window, cx)).is_some()
    });
    let section = in_window(vcx, |window, cx| section_state(window, cx)).expect("the section");
    assert!(
        vcx.cx
            .read(|cx| section.read(cx).is_on() && section.read(cx).is_open())
    );
    let focused = in_window(vcx, |window, cx| section.read(cx).focused(window, cx));
    assert_eq!(focused.as_deref(), Some("ssh-host"));
    let shown = |vcx: &mut VisualTestContext| {
        let (Some(body), Some(field)) = (
            vcx.debug_bounds("form-dialog-body"),
            vcx.debug_bounds("ssh-field-ssh-host"),
        ) else {
            return false;
        };
        field.top() >= body.top() && field.bottom() <= body.bottom()
    };
    for _ in 0..20 {
        if shown(vcx) {
            break;
        }
        vcx.cx.update(|cx| cx.refresh_windows());
        vcx.run_until_parked();
    }
    assert!(shown(vcx), "the SSH host field is in view at 960x600");
}

/// Fills the open SSH section with `server`'s tunnel (a password user), saves and connects,
/// trusts the new host key from the keyboard, and checks the connection and the saved profile.
fn fill_tunnel_and_connect(
    vcx: &mut VisualTestContext,
    server: &TestSshServer,
    id: ProfileId,
    name: &str,
) {
    // The rest of the flow at the default window: a click reaches only what is drawn in view.
    vcx.simulate_resize(gpui::size(gpui::px(1280.0), gpui::px(800.0)));
    vcx.run_until_parked();
    let section = in_window(vcx, |window, cx| section_state(window, cx)).expect("the section");
    let ssh_field = |vcx: &mut VisualTestContext, field: &str, text: &str| {
        let input = vcx
            .cx
            .read(|cx| section.read(cx).input(field))
            .unwrap_or_else(|| panic!("no SSH field {field}"));
        in_window(vcx, |window, cx| {
            input.update(cx, |i, cx| {
                i.set_value("");
                i.replace_text_in_range(None, text, window, cx);
            });
        });
        vcx.run_until_parked();
    };
    ssh_field(vcx, "ssh-host", &server.host());
    ssh_field(vcx, "ssh-port", &server.addr.port().to_string());
    ssh_field(vcx, "ssh-user", "deploy");
    let at = AuthKind::ALL
        .iter()
        .position(|k| *k == AuthKind::Password)
        .expect("a method");
    click_selector(vcx, &format!("segmented-ssh-auth-{at}"));
    ssh_field(vcx, "ssh-password", SSH_PW);

    click_selector(vcx, "button-conn-save-connect");
    settle_until(vcx, WAIT, || layer_open(vcx, "hostkey-prompt"));
    trust_host_key_by_keyboard(vcx);
    settle_until(vcx, WAIT, || {
        matches!(live(vcx, name), LiveState::Connected(_))
    });

    let LiveState::Connected(info) = live(vcx, name) else {
        panic!("connected");
    };
    assert_eq!(info.path, PathKind::Ssh);
    assert_eq!(
        capsule_text(vcx, "path"),
        format!("SSH via {}", server.host())
    );
    let p = saved(vcx, id);
    let TransportSpec::SshTunnel(spec) = &p.transport else {
        panic!("the saved profile keeps the tunnel");
    };
    assert_eq!(spec.hops.len(), 1);
    assert_eq!(spec.hops[0].host, server.host());
    assert_eq!(spec.hops[0].port, server.addr.port());
    assert_eq!(fake(vcx).connect_calls(), 2);
    assert!(!connections_toml(vcx).contains(SSH_PW));
    assert!(!frame_text(vcx).contains(SSH_PW));
}

/// The fix itself (fix round 1, I4). A server that offers no TLS to a connection that requires
/// it fails with `tls.not_supported_by_server`; the capsule opens its error dialog, whose fixes
/// are "Connect through SSH…" (M2a NOTES `fix-ssh`) and Edit connection. From the keyboard,
/// Shift-Tab from Edit connection reaches the fix and Space runs it: the connection's dialog
/// opens with the SSH section on and the SSH host in view at 960x600; the tunnel is filled in,
/// saved and connected through the in-process SSH server.
#[gpui::test]
fn connect_through_ssh_fix_end_to_end(cx: &mut TestAppContext) {
    let driver = FakeDriver::new(DbmsKind::Postgres)
        .script(vec![
            Err(RawError {
                kind: RawErrorKind::TlsNotOffered,
                server: None,
            }),
            Ok(()),
        ])
        .tls(TlsSummary {
            encrypted: true,
            version: Some("TLSv1.3".into()),
            cipher: None,
            verified: TlsVerified::Full,
        });
    let vcx = shell_with_fake(cx, "en", driver);
    vcx.simulate_resize(gpui::size(gpui::px(960.0), gpui::px(600.0)));
    let server = start_ssh(vcx);
    let mut p = test_profile("analytics-eu", Some(EnvTag::Staging), "10.0.0.5");
    p.tls.mode = TlsMode::Require;
    let id = add_profile(vcx, p);
    vcx.cx.update(|cx| ConnectionsModel::select(cx, Some(id)));
    connect(vcx, "analytics-eu");
    settle_until(vcx, WAIT, || {
        matches!(live(vcx, "analytics-eu"), LiveState::Failed(_))
    });

    // The capsule's body opens the error; its first fix is the SSH one.
    click_selector(vcx, "capsule-body");
    settle_until(vcx, WAIT, || layer_open(vcx, "connection-error"));
    let fix = t(vcx, "hv-conn-fix-ssh");
    assert!(
        texts_drawn_by(vcx, "hacivat-ui/src/button.rs")
            .iter()
            .any(|s| plain(s) == fix),
        "{:?}",
        texts_drawn_by(vcx, "hacivat-ui/src/button.rs")
    );
    assert!(vcx.debug_bounds("button-fix-ssh").is_some());
    // Edit connection has the initial focus (the SSH fix is not the primary); Shift-Tab reaches
    // the SSH fix and Space runs it. Space on Edit connection would open the dialog with the
    // tunnel off, and Space anywhere else would not open it: the section check below tells.
    press(vcx, "shift-tab");
    press(vcx, "space");

    settle_until(vcx, WAIT, || !layer_open(vcx, "connection-error"));
    assert_tunnel_section_landed(vcx);
    fill_tunnel_and_connect(vcx, &server, id, "analytics-eu");
}

/// The other way in: the server at a public address offers no TLS (`Auto`), the plaintext
/// question opens, and its default, Enter, answers "Use an SSH tunnel"; the same SSH section
/// opens with the SSH host in view at 960x600, and the tunnel connects.
#[gpui::test]
fn use_an_ssh_tunnel_answer_end_to_end(cx: &mut TestAppContext) {
    let driver = FakeDriver::new(DbmsKind::Postgres)
        .script(vec![
            Err(RawError {
                kind: RawErrorKind::TlsNotOffered,
                server: None,
            }),
            Ok(()),
        ])
        .tls(TlsSummary {
            encrypted: true,
            version: Some("TLSv1.3".into()),
            cipher: None,
            verified: TlsVerified::Full,
        });
    let vcx = shell_with_fake(cx, "en", driver);
    vcx.simulate_resize(gpui::size(gpui::px(960.0), gpui::px(600.0)));
    let server = start_ssh(vcx);
    let id = add_profile(
        vcx,
        test_profile("analytics-eu", Some(EnvTag::Staging), "203.0.113.10"),
    );
    vcx.cx.update(|cx| ConnectionsModel::select(cx, Some(id)));
    connect(vcx, "analytics-eu");

    // The plaintext question; its safe default is the tunnel.
    settle_until(vcx, WAIT, || layer_open(vcx, "plaintext-prompt"));
    press(vcx, "enter");
    assert_tunnel_section_landed(vcx);
    assert!(!matches!(
        live(vcx, "analytics-eu"),
        LiveState::Connected(_)
    ));
    fill_tunnel_and_connect(vcx, &server, id, "analytics-eu");
}

// ---- the explanation's triggers (Task 24 polish: `aria-expanded`) ------------------------------

/// The read-only explanation opens from the palette command; while it is open both of its
/// triggers, the capsule's access segment and the sheet's Access value, declare themselves
/// expanded; Escape closes it and both say collapsed again.
#[gpui::test]
fn the_explanations_triggers_say_whether_it_is_open(cx: &mut TestAppContext) {
    use hacivat_driver::{SemVerLoose, ServerInfo, Support};
    use hacivat_net::AddressClass;
    use hacivat_proto::AccessLabel;
    use hacivat_secrets::AuthProviderId;
    use hacivat_session::LiveInfo;

    let vcx = shell_with_fake(cx, "en", FakeDriver::new(DbmsKind::Postgres));
    let mut p = test_profile("orders-qa", Some(EnvTag::Test), "10.0.4.12");
    p.read_only = true;
    let id = add_profile(vcx, p);
    let info = LiveInfo {
        server: ServerInfo {
            kind: DbmsKind::Postgres,
            product: "PostgreSQL".to_string(),
            version: SemVerLoose::parse("16.4"),
            raw_version: "16.4".to_string(),
            is_replica: Some(false),
        },
        support: Support::Supported,
        tls: TlsSummary {
            encrypted: true,
            version: Some("TLSv1.3".to_string()),
            cipher: None,
            verified: TlsVerified::Full,
        },
        address: AddressClass::Private,
        latency: std::time::Duration::from_millis(12),
        access: AccessLabel::ReadOnlyGuarded,
        path: PathKind::Direct,
        user: Some("qa".to_string()),
        auth: AuthProviderId::Password,
        via: None,
        hops: 0,
        access_warnings: Vec::new(),
    };
    vcx.cx.update(|cx| {
        let m = cx.global_mut::<ConnectionsModel>();
        m.set_live(id, LiveState::Connected(Arc::new(info)));
        m.note_attempt(id);
        ConnectionsModel::select(cx, Some(id));
    });
    vcx.run_until_parked();
    // Every read-only access trigger drawn (the capsule's segment, the sheet's value).
    let triggers = |vcx: &VisualTestContext| {
        hacivat_ui::popover::a11y_record();
        vcx.cx.update(|cx| cx.refresh_windows());
        vcx.run_until_parked();
        hacivat_ui::popover::a11y_take()
            .into_iter()
            .filter(|d| {
                d.role == Some(gpui::Role::Button) && plain(&d.name).starts_with("read-only")
            })
            .map(|d| d.expanded)
            .collect::<Vec<_>>()
    };
    assert_eq!(triggers(vcx), [Some(false), Some(false)]);
    assert!(run(vcx, "connection.explain_access"));
    assert!(layer_open(vcx, "popover"));
    assert_eq!(triggers(vcx), [Some(true), Some(true)]);
    vcx.simulate_keystrokes("escape");
    assert!(!layer_open(vcx, "popover"));
    assert_eq!(triggers(vcx), [Some(false), Some(false)]);
}

// ---- the hard stops from the keyboard (fix round 1, M5) -----------------------------------------

/// Asks `q` through the app's UI port, as the pipeline does from the core runtime; the answer
/// lands in the returned slot.
fn ask(vcx: &VisualTestContext, q: Question) -> Rc<RefCell<Option<Answer>>> {
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
    vcx.run_until_parked();
    slot
}

/// Presses Tab (Shift-Tab when `back`) until the prompt part `name` has the focus.
fn tab_to(vcx: &mut VisualTestContext, name: &str, back: bool) {
    for _ in 0..8 {
        if focused_part(vcx) == name {
            return;
        }
        press(vcx, if back { "shift-tab" } else { "tab" });
    }
    assert_eq!(focused_part(vcx), name, "Tab never reached {name}");
}

/// `tls.pin_changed` for `127.0.0.1:5432`: the pinned certificate is `[1; 32]`, the server's
/// `[2; 32]`.
fn pin_changed() -> ConnectError {
    let leaf = "CN=reports-qa,O=Acme".to_string();
    ConnectError {
        class: ErrClass::TlsPinChanged,
        phase: Phase::DriverConnect,
        trail: Vec::new(),
        args: ErrArgs {
            name: "reports-qa".into(),
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
        fixes: vec![FixId::ReviewCertificate],
        suggestions: Vec::new(),
        issues: Vec::new(),
        elapsed: std::time::Duration::ZERO,
        attempts: 0,
        needs_interaction: false,
        certificate: Some(CertReview {
            endpoint: "127.0.0.1:5432".into(),
            subject: leaf.clone(),
            issuer: "CN=Acme Internal CA G2,O=Acme".into(),
            // 2020-01-01 to 2100-01-01: valid today.
            not_before_unix: 1_577_836_800,
            not_after_unix: 4_102_444_800,
            sha256: [2; 32],
            chain: vec![ChainEntry {
                subject: leaf,
                role: ChainRole::Leaf,
            }],
            pinned: Some([1; 32]),
        }),
        host_key: None,
        profile_changes: Vec::new(),
    }
}

/// Replace pin from the keyboard alone: the error dialog's Review certificate… (its primary,
/// with the focus) opens the hard stop on Cancel; Shift-Tab reaches Replace pin and Space
/// presses it (Enter would cancel); the profile then keeps the new pin.
#[gpui::test]
fn replace_pin_completes_from_the_keyboard(cx: &mut TestAppContext) {
    let vcx = shell_with_fake(cx, "en", FakeDriver::new(DbmsKind::Postgres));
    let mut p = test_profile("reports-qa", Some(EnvTag::Test), "127.0.0.1");
    p.tls.pins = vec![CertPin {
        endpoint: "127.0.0.1:5432".into(),
        sha256: [1; 32],
    }];
    let id = add_profile(vcx, p);
    in_window(vcx, |window, cx| {
        open_error_dialog(id, Arc::new(pin_changed()), window, cx)
    });
    settle_until(vcx, WAIT, || layer_open(vcx, "connection-error"));
    press(vcx, "space");
    settle_until(vcx, WAIT, || layer_open(vcx, "cert-dialog"));
    assert_eq!(review_focus(vcx), Some("cert-cancel"));
    press(vcx, "shift-tab");
    assert_eq!(review_focus(vcx), Some("cert-replace-pin"));
    press(vcx, "space");
    settle_until(vcx, WAIT, || {
        saved(vcx, id).tls.pins
            == [CertPin {
                endpoint: "127.0.0.1:5432".into(),
                sha256: [2; 32],
            }]
    });
    assert!(!layer_open(vcx, "cert-dialog"));
}

/// A shell whose `Forever` secrets live in an existing, locked vault (created under
/// [`MASTER`]); and the vault.
fn shell_with_locked_vault(
    cx: &mut TestAppContext,
) -> (
    &'static mut VisualTestContext,
    Arc<FileVault>,
    tempfile::TempDir,
) {
    let dir = tempfile::tempdir().expect("a temp dir");
    let vault = Arc::new(FileVault::with_params(
        dir.path().join("vault.bin"),
        VaultParams {
            m_kib: 64,
            t: 1,
            p: 1,
        },
    ));
    vault.create(MASTER.into()).expect("the vault is created");
    vault.lock();
    let backend = FallbackBackend::new(
        Arc::new(MemoryBackend::new()),
        vault.clone(),
        Box::new(|| false),
    );
    assert!(backend.probe_available(), "the probe chose the vault");
    let vcx = shell_with_fake_backend(
        cx,
        "en",
        FakeDriver::new(DbmsKind::Postgres),
        Arc::new(backend),
        |_| {},
    );
    (vcx, vault, dir)
}

/// The vault reset from the keyboard alone: Unlock's Reset vault… by Tab and Space; the typed
/// word; Tab to Reset vault and Space (Enter in the field never resets). The vault is gone.
#[gpui::test]
fn vault_reset_completes_from_the_keyboard(cx: &mut TestAppContext) {
    let (vcx, vault, _dir) = shell_with_locked_vault(cx);
    let asked = ask(vcx, Question::UnlockVault { attempt: 1 });
    wait_until_mut(vcx, "the reset entry", |vcx| {
        vcx.debug_bounds("button-vault-reset-open").is_some()
    });
    assert_eq!(focused_part(vcx), "vault-unlock");
    tab_to(vcx, "vault-reset-open", false);
    press(vcx, "space");
    settle_until(vcx, WAIT, || layer_open(vcx, "vault-reset"));
    assert_eq!(focused_part(vcx), "vault-reset-word");
    vcx.simulate_input(&t(vcx, "hv-conn-vault-reset-word"));
    press(vcx, "enter");
    assert!(vault.exists(), "Enter never resets");
    tab_to(vcx, "vault-reset-confirm", false);
    press(vcx, "space");
    settle_until(vcx, WAIT, || !vault.exists());
    settle_until(vcx, WAIT, || asked.borrow().is_some());
    assert!(matches!(*asked.borrow(), Some(Answer::Cancelled)));
    assert!(layer_ids(vcx).is_empty(), "{:?}", layer_ids(vcx));
}

/// The damaged-file notice from the keyboard: Close has the focus; Tab reaches Reset vault…
/// and Space opens the reset confirmation, focused on its field.
#[gpui::test]
fn damaged_vault_reset_entry_completes_from_the_keyboard(cx: &mut TestAppContext) {
    let (vcx, vault, _dir) = shell_with_locked_vault(cx);
    let _asked = ask(vcx, Question::VaultDamaged);
    settle_until(vcx, WAIT, || layer_open(vcx, "vault-damaged"));
    wait_until_mut(vcx, "the damaged notice's reset entry", |vcx| {
        vcx.debug_bounds("button-vault-damaged-reset").is_some()
    });
    assert_eq!(focused_part(vcx), "vault-damaged-close");
    tab_to(vcx, "vault-damaged-reset", false);
    press(vcx, "space");
    settle_until(vcx, WAIT, || layer_open(vcx, "vault-reset"));
    assert_eq!(focused_part(vcx), "vault-reset-word");
    assert!(vault.exists(), "opening the confirmation deletes nothing");
}
