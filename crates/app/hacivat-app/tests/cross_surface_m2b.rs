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

use std::net::SocketAddr;
use std::sync::Arc;

use gpui::{EntityInputHandler as _, Focusable as _, TestAppContext, VisualTestContext};
use hacivat_app::connections::dialog::ssh::{AuthKind, section_state};
use hacivat_app::connections::dialog::{DialogMode, open_connection_dialog, open_dialog};
use hacivat_app::connections::prompt_kit::parts;
use hacivat_app::connections::{ConnectionsModel, cert_dialog};
use hacivat_app::l10n_global::tr_id;
use hacivat_driver::fake::FakeDriver;
use hacivat_driver::{RawError, RawErrorKind};
use hacivat_net::ssh::{HopSpec, HostKeyPolicy, SshAuth, SshSpec};
use hacivat_net::{TlsFailure, TlsSummary, TlsVerified};
use hacivat_proto::{DbmsKind, EnvTag, ProfileId};
use hacivat_secrets::{
    BackendChoice, ExposeSecret, FallbackBackend, FileVault, MemoryBackend, Retention,
    SecretBackend as _, SecretKey, SecretSlot, VaultBackend as _, VaultParams,
};
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
/// keyboard, the pinned connect lands in the capsule and the status bar as verified TLS, the
/// pin reaches the file, and the next connect carries it without asking again.
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
    // Keyboard alone: Cancel has the focus; Tab, Tab reaches Pin; Enter presses it.
    assert_eq!(review_focus(vcx), Some("cert-cancel"));
    press(vcx, "tab tab");
    assert_eq!(review_focus(vcx), Some("cert-pin"));
    press(vcx, "enter");

    settle_until(vcx, WAIT, || {
        matches!(live(vcx, "reports-qa"), LiveState::Connected(_))
    });
    assert!(!layer_open(vcx, "cert-dialog"));
    let LiveState::Connected(info) = live(vcx, "reports-qa") else {
        panic!("connected");
    };
    assert_eq!(info.tls.verified, TlsVerified::Pinned);
    // The capsule shows the live connection, encrypted (no "unencrypted" segment); the status
    // bar says how: a pinned certificate reads as verified TLS (no separate "pinned" label).
    assert!(matches!(
        capsule_model(vcx),
        CapsuleModel::Connection {
            unencrypted: None,
            ..
        }
    ));
    assert!(text_of(vcx, "capsule-text-access").is_some());
    let status = plain(status_bar_text(vcx));
    assert!(status.contains("TLS 1.3 · verified"), "{status}");

    // The pin reached the profile and its file.
    settle_until(vcx, WAIT, || saved(vcx, id).tls.pins.len() == 1);
    let pin = saved(vcx, id).tls.pins[0].clone();
    assert_eq!(pin.endpoint, "10.0.0.5:5432");
    wait_until(vcx, "the pin to reach the file", |vcx| {
        connections_toml(vcx).contains("10.0.0.5:5432")
    });

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
    // Neither secret reached a file or the screen.
    for secret in [DB_PW, MASTER] {
        assert!(!connections_toml(vcx).contains(secret));
        assert!(!frame_text(vcx).contains(secret));
    }
}

// ---- 5 · "Use an SSH tunnel" end to end ----------------------------------------------------------

/// The server at a public address offers no TLS (`Auto`): the plaintext question opens and its
/// default, Enter, answers "Use an SSH tunnel"; the connection's dialog opens with the SSH
/// section on and the SSH host focused; the tunnel is filled in and saved; Save and connect
/// meets the new-host-key prompt, trusted from the keyboard; the connection lands in the capsule
/// through the bastion and the saved profile keeps the tunnel.
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
    assert!(!matches!(
        live(vcx, "analytics-eu"),
        LiveState::Connected(_)
    ));

    // The tunnel: the bastion's host and port, its user, a password.
    let ssh_field = |vcx: &mut VisualTestContext, name: &str, text: &str| {
        let input = vcx
            .cx
            .read(|cx| section.read(cx).input(name))
            .unwrap_or_else(|| panic!("no SSH field {name}"));
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
        matches!(live(vcx, "analytics-eu"), LiveState::Connected(_))
    });

    let LiveState::Connected(info) = live(vcx, "analytics-eu") else {
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
