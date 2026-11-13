//! M2b Task 21: the SSH and vault prompts (NOTES surface 6's new-host-key prompt; new surfaces
//! 1–4; B11, B13). Questions go through the app's real UI port, as the pipeline asks them from
//! the core runtime. No test touches the platform keyring: the vault tests inject a
//! `FallbackBackend` over `MemoryBackend` whose probe says "no keyring", and wait for the probe
//! to land (Q34). Every wait is a condition (`settle_until`, `wait_until`), never a fixed sleep.
//! Secret fixtures appear only inside assertions that compare them without printing them.

// Unix-only tests (file modes, Unix sockets, ssh-agent) leave their helpers unused on other
// systems; the lints still run in full on Unix.
#![cfg_attr(not(unix), allow(dead_code, unused_imports, unused_variables))]

mod support;

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use gpui::{Entity, EntityInputHandler as _, TestAppContext, VisualTestContext};
use hacivat_app::connections::ConnectionsModel;
use hacivat_app::connections::dialog::{DialogMode, open_connection_dialog, open_dialog};
use hacivat_app::connections::error_view::open_error_dialog;
use hacivat_app::connections::hostkey_prompt::review_changed;
use hacivat_app::connections::interaction::interaction;
use hacivat_app::connections::prompt_kit::parts;
use hacivat_app::l10n_global::{tr, tr_id};
use hacivat_l10n::{Count, ids};
use hacivat_net::ssh::{HopSpec, HostKey, HostKeyPolicy, KbdPrompt, KbdRequest, KnownSource};
use hacivat_net::ssh::{SshAuth, SshSpec};
use hacivat_proto::{EnvTag, ErrClass, FixId, ProfileId};
use hacivat_secrets::{
    AuthProviderId, BackendChoice, ExposeSecret, FallbackBackend, FileVault, MemoryBackend,
    Retention, SecretBackend, SecretError, SecretKey, SecretSlot, SecretStore, SecretString,
    VaultBackend, VaultParams,
};
use hacivat_session::pipeline::{
    Answer, ConnectError, ErrArgs, HostKeyMismatch, Interaction, Phase, Question, SecretPrompt,
};
use hacivat_session::profile::{Profile, ProfileDraft, TransportSpec};
use hacivat_ui::input::TextInput;

use support::*;

// ---- fixtures -------------------------------------------------------------------------------

/// Two valid Ed25519 host keys (any 32 bytes make a key that parses).
const K1: &str = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIAEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEB";
const K2: &str = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIAICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgIC";

fn key(line: &str) -> HostKey {
    HostKey::from_openssh(line).expect("a valid OpenSSH key")
}

/// The master password of the test vaults.
const MASTER: &str = "correcthorse";

thread_local! {
    /// Keeps each fixture's directory alive until the test thread ends.
    static DIRS: RefCell<Vec<tempfile::TempDir>> = const { RefCell::new(Vec::new()) };
}

fn tmp_dir() -> PathBuf {
    let dir = tempfile::tempdir().expect("a temp dir");
    let path = dir.path().to_path_buf();
    DIRS.with(|d| d.borrow_mut().push(dir));
    path
}

fn shell(cx: &mut TestAppContext) -> &'static mut VisualTestContext {
    shell_with_connections(cx, "en")
}

/// A vault in a temp folder with fast Argon2 parameters, and its path.
fn fast_vault_at() -> (Arc<FileVault>, PathBuf) {
    let path = tmp_dir().join("vault.bin");
    let vault = FileVault::with_params(
        path.clone(),
        VaultParams {
            m_kib: 64,
            t: 1,
            p: 1,
        },
    );
    (Arc::new(vault), path)
}

fn fast_vault() -> Arc<FileVault> {
    fast_vault_at().0
}

/// A `FallbackBackend` over `MemoryBackend` whose keyring probe says "unusable" (never the
/// platform keyring, Q34): it chooses `vault` once probed.
fn no_keyring(vault: Arc<FileVault>) -> FallbackBackend {
    FallbackBackend::new(Arc::new(MemoryBackend::new()), vault, Box::new(|| false))
}

/// A shell whose `Forever` secrets go to a vault (not probed yet, no vault file).
fn shell_with_fallback_probe_false(cx: &mut TestAppContext) -> &'static mut VisualTestContext {
    shell_with_backend(cx, "en", None, Arc::new(no_keyring(fast_vault())))
}

/// Like [`shell_with_fallback_probe_false`]; the vault does not exist yet.
fn shell_with_fallback_probe_false_no_vault(
    cx: &mut TestAppContext,
) -> &'static mut VisualTestContext {
    shell_with_fallback_probe_false(cx)
}

/// A shell whose vault exists (created under [`MASTER`]) and was chosen; `unlocked` says
/// whether it stays unlocked.
fn shell_with_vault_state(
    cx: &mut TestAppContext,
    unlocked: bool,
) -> (&'static mut VisualTestContext, Arc<FileVault>) {
    let (vcx, vault, _) = shell_with_vault_at(cx, unlocked);
    (vcx, vault)
}

/// [`shell_with_vault_state`] with the vault file's path.
fn shell_with_vault_at(
    cx: &mut TestAppContext,
    unlocked: bool,
) -> (&'static mut VisualTestContext, Arc<FileVault>, PathBuf) {
    let (vault, path) = fast_vault_at();
    vault
        .create(SecretString::from(MASTER))
        .expect("the vault is created");
    if !unlocked {
        vault.lock();
    }
    let backend = no_keyring(vault.clone());
    assert!(backend.probe_available(), "the probe chose the vault");
    (
        shell_with_backend(cx, "en", None, Arc::new(backend)),
        vault,
        path,
    )
}

fn shell_with_vault(cx: &mut TestAppContext) -> &'static mut VisualTestContext {
    shell_with_vault_state(cx, false).0
}

fn secrets(vcx: &VisualTestContext) -> Arc<SecretStore> {
    vcx.cx
        .read(|cx| cx.global::<ConnectionsModel>().secrets().clone())
}

fn backend_choice(vcx: &VisualTestContext) -> Option<BackendChoice> {
    secrets(vcx).backend_choice()
}

fn vault_exists(vcx: &VisualTestContext) -> bool {
    secrets(vcx).vault().is_some_and(|v| v.exists())
}

/// A saved profile through two hops, `jump-eu.acme.io` (password) then `bastion.acme.io:2222`
/// (a key file), to a database at an IP literal (nothing is ever dialled).
fn ssh_profile(name: &str) -> Profile {
    let mut p = test_profile(name, Some(EnvTag::Production), "10.0.0.5");
    p.transport = TransportSpec::SshTunnel(SshSpec {
        hops: vec![hop("jump-eu.acme.io", 22, SshAuth::Password), bastion()],
        host_keys: HostKeyPolicy::Ask,
        keepalive_s: None,
    });
    p
}

fn hop(host: &str, port: u16, auth: SshAuth) -> HopSpec {
    HopSpec {
        host: host.into(),
        port,
        user: "deploy".into(),
        auth,
        identities_only: false,
        identity_files: Vec::new(),
    }
}

fn bastion() -> HopSpec {
    hop(
        "bastion.acme.io",
        2222,
        SshAuth::KeyFile {
            path: PathBuf::from("/keys/id_ed25519"),
        },
    )
}

// ---- asking ---------------------------------------------------------------------------------

/// What a question was answered with, once it was.
struct Asked(Rc<RefCell<Option<Answer>>>);

impl Asked {
    fn get(&self) -> std::cell::Ref<'_, Option<Answer>> {
        self.0.borrow()
    }
}

/// Asks `q` through the app's UI port, as the pipeline does from the core runtime.
fn ask(vcx: &VisualTestContext, q: Question) -> Asked {
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
    Asked(slot)
}

fn host_key_question(profile: ProfileId, host: &str, key_line: &str) -> Question {
    Question::HostKey {
        profile,
        name: "orders-prod".into(),
        hop: 1,
        host: host.into(),
        port: 2222,
        key: key(key_line),
    }
}

fn kbd_question(name: &str, instructions: &str, prompts: &[(&str, bool)]) -> Question {
    Question::Keyboard {
        profile: ProfileId::new(),
        name: "orders-prod".into(),
        hop: 0,
        request: KbdRequest {
            host: "bastion.acme.io".into(),
            name: name.into(),
            instructions: instructions.into(),
            prompts: prompts
                .iter()
                .map(|(text, echo)| KbdPrompt {
                    text: (*text).into(),
                    echo: *echo,
                })
                .collect(),
        },
        attempt: 1,
    }
}

/// An SSH password question of a connect whose saved `ssh_retention` is Forever.
fn ssh_password_question(profile: ProfileId, hop: u8, host: &str, attempt: u8) -> Question {
    ssh_password_question_remembering(profile, hop, host, attempt, Some(Retention::Forever))
}

/// An SSH password question whose Remember starts from `remember` (`None`: a Test).
fn ssh_password_question_remembering(
    profile: ProfileId,
    hop: u8,
    host: &str,
    attempt: u8,
    remember: Option<Retention>,
) -> Question {
    Question::Secret {
        profile,
        name: "orders-prod".into(),
        prompt: SecretPrompt::SshPassword {
            hop,
            host: host.into(),
            user: "deploy".into(),
        },
        attempt,
        remember,
    }
}

fn mismatch(source: KnownSource) -> HostKeyMismatch {
    HostKeyMismatch {
        hop: 1,
        host: "bastion.acme.io".into(),
        port: 2222,
        old: key(K1),
        new: key(K2),
        source,
    }
}

/// Opens the changed-host-key review for an unsaved profile (a reconnect is a no-op).
fn review(vcx: &VisualTestContext, m: HostKeyMismatch) {
    review_for(vcx, m, ProfileId::new());
}

fn review_for(vcx: &VisualTestContext, m: HostKeyMismatch, profile: ProfileId) {
    in_window(vcx, |window, cx| review_changed(m, profile, window, cx));
    vcx.run_until_parked();
}

// ---- reading and driving the prompts ----------------------------------------------------------

fn text(vcx: &VisualTestContext, id: &str) -> String {
    text_of(vcx, id).unwrap_or_else(|| panic!("no text `{id}` in the frame"))
}

/// A message without its isolation marks, as the frame getters strip them.
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

/// Whether the button `id` is drawn.
fn visible(vcx: &mut VisualTestContext, id: &str) -> bool {
    vcx.run_until_parked();
    let selector: &'static str = Box::leak(format!("button-{id}").into_boxed_str());
    vcx.debug_bounds(selector).is_some()
}

fn click_button(vcx: &mut VisualTestContext, id: &str) {
    let selector: &'static str = Box::leak(format!("button-{id}").into_boxed_str());
    click(vcx, selector);
}

/// Presses each of `keys` down and up, as a keyboard sends it: GPUI turns a clean down-and-up
/// of Enter or Space on a focused button into a click, so the key-up phase is exercised too.
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
fn focused_id(vcx: &VisualTestContext) -> String {
    vcx.run_until_parked();
    in_window(vcx, |window, cx| parts::focused(window, cx)).unwrap_or_default()
}

fn input(vcx: &VisualTestContext, name: &str) -> Entity<TextInput> {
    vcx.cx
        .read(|cx| parts::input(cx, name))
        .unwrap_or_else(|| panic!("no field `{name}`"))
}

fn focus(vcx: &VisualTestContext, name: &str) {
    let handle = vcx
        .cx
        .read(|cx| parts::focus(cx, name))
        .unwrap_or_else(|| panic!("no part `{name}`"));
    in_window(vcx, |window, cx| window.focus(&handle, cx));
    vcx.run_until_parked();
}

/// Types `text` (no spaces) into the field `name`, as keystrokes.
fn type_into(vcx: &mut VisualTestContext, name: &str, text: &str) {
    focus(vcx, name);
    vcx.simulate_input(text);
    vcx.run_until_parked();
}

/// Replaces the field's text with `text` through the platform input path (as a paste or an
/// IME commit lands): the field reports the change like typing does.
fn set_field(vcx: &VisualTestContext, name: &str, text: &str) {
    let field = input(vcx, name);
    in_window(vcx, |window, cx| {
        field.update(cx, |i, cx| {
            i.set_value("");
            i.replace_text_in_range(None, text, window, cx);
        });
    });
    vcx.run_until_parked();
}

fn is_masked(vcx: &VisualTestContext, name: &str) -> bool {
    let field = input(vcx, name);
    vcx.cx.read(|cx| field.read(cx).is_masked())
}

fn value(vcx: &VisualTestContext, name: &str) -> String {
    let field = input(vcx, name);
    vcx.cx.read(|cx| field.read(cx).value().to_string())
}

/// What the field gives platform text services (`text_for_range`, the IME and dictation).
fn ime_text(vcx: &VisualTestContext, name: &str) -> String {
    let field = input(vcx, name);
    in_window(vcx, |window, cx| {
        field.update(cx, |i, cx| {
            let len = i.value().encode_utf16().count();
            i.text_for_range(0..len, &mut None, window, cx)
                .unwrap_or_default()
        })
    })
}

/// The secret kept for `slot` of `profile` under `retention`, compared without printing it.
fn stored(
    vcx: &VisualTestContext,
    profile: ProfileId,
    slot: SecretSlot,
    r: Retention,
) -> Option<String> {
    secrets(vcx)
        .get(&SecretKey { profile, slot }, r)
        .ok()
        .flatten()
        .map(|s| s.expose_secret().to_string())
}

// ---- the new-host-key prompt (M2a surface 6) -------------------------------------------------

#[gpui::test]
fn new_host_key_prompt(cx: &mut TestAppContext) {
    let vcx = shell(cx);
    let a = ask(
        vcx,
        host_key_question(ProfileId::new(), "bastion.acme.io", K1),
    );
    assert!(text(vcx, "hk-fingerprint").starts_with("SHA256:"));
    assert_eq!(text(vcx, "hk-fingerprint"), key(K1).fingerprint());
    assert_eq!(text(vcx, "hk-algorithm"), "ED25519");
    assert!(text(vcx, "hk-store").contains("~/.ssh/known_hosts"));
    assert_eq!(
        text(vcx, "hk-title"),
        plain(vcx.cx.read(|cx| tr(
            cx,
            ids::hv::CONN_HK_TITLE,
            ids::hv::HvConnHkTitle {
                host: hacivat_l10n::Text("bastion.acme.io".into())
            }
        )))
    );
    // Fix round 1 (review I2, controller ruling; deviates from the M2a mock): Cancel has the
    // initial focus and a bare Enter cancels, so a reflexive Enter never trusts a key.
    assert_eq!(focused_id(vcx), "hk-cancel");
    press(vcx, "enter");
    assert!(matches!(*a.get(), Some(Answer::Cancelled)));
    assert!(!layer_open(vcx, "hostkey-prompt"));
    // Trusting takes Tab to Trust and connect, then Space (or a click).
    let b = ask(
        vcx,
        host_key_question(ProfileId::new(), "bastion.acme.io", K1),
    );
    press(vcx, "tab");
    assert_eq!(focused_id(vcx), "hk-trust");
    press(vcx, "space");
    assert!(matches!(*b.get(), Some(Answer::TrustHostKey)));
    assert!(!layer_open(vcx, "hostkey-prompt"));
}

#[gpui::test]
fn new_host_key_prompt_cancels_from_escape_and_cancel(cx: &mut TestAppContext) {
    let vcx = shell(cx);
    let a = ask(
        vcx,
        host_key_question(ProfileId::new(), "bastion.acme.io", K1),
    );
    press(vcx, "escape");
    assert!(matches!(*a.get(), Some(Answer::Cancelled)));
    let b = ask(
        vcx,
        host_key_question(ProfileId::new(), "bastion.acme.io", K1),
    );
    // Enter with the focus on Trust and connect still cancels (I2): never a keyboard trust.
    press(vcx, "tab");
    assert_eq!(focused_id(vcx), "hk-trust");
    press(vcx, "enter");
    assert!(matches!(*b.get(), Some(Answer::Cancelled)));
    let c = ask(
        vcx,
        host_key_question(ProfileId::new(), "bastion.acme.io", K1),
    );
    click_button(vcx, "hk-trust");
    assert!(matches!(*c.get(), Some(Answer::TrustHostKey)));
    assert!(layer_ids(vcx).is_empty());
}

// ---- the changed-host-key review (surface 1) -------------------------------------------------

#[gpui::test]
fn host_key_changed_offers_replace_only_for_hacivat_store(cx: &mut TestAppContext) {
    let vcx = shell(cx);
    review(vcx, mismatch(KnownSource::Hacivat));
    assert!(visible(vcx, "hk-replace"));
    assert_eq!(
        focused_id(vcx),
        "hk-cancel",
        "Q34: Cancel has the initial focus"
    );
    press(vcx, "escape");
    assert!(!layer_open(vcx, "hk-changed"));
    review(
        vcx,
        mismatch(KnownSource::OpenSsh {
            path: "/home/me/.ssh/known_hosts".into(),
            line: 14,
        }),
    );
    assert!(!visible(vcx, "hk-replace"));
    assert!(visible(vcx, "hk-retry"));
    assert_eq!(
        text(vcx, "hk-keygen"),
        "ssh-keygen -R \"[bastion.acme.io]:2222\""
    );
    assert_eq!(focused_id(vcx), "hk-cancel");
}

#[gpui::test]
fn host_key_changed_names_both_keys_and_the_hop(cx: &mut TestAppContext) {
    let vcx = shell(cx);
    let id = add_profile(vcx, ssh_profile("orders-prod"));
    review_for(vcx, mismatch(KnownSource::Hacivat), id);
    assert_eq!(text(vcx, "hk-old-fingerprint"), key(K1).fingerprint());
    assert_eq!(text(vcx, "hk-new-fingerprint"), key(K2).fingerprint());
    let drawn = texts_drawn_by(vcx, "connections/prompt_kit.rs").join("|");
    assert!(
        drawn.contains("bastion.acme.io, hop 2 of 2"),
        "the subtitle names the hop: {drawn}"
    );
    assert!(drawn.contains("orders-prod"));
    assert_eq!(
        text(vcx, "hk-stored-in"),
        t(vcx, "hv-conn-hk-changed-src-hacivat")
    );
}

/// Hacivat's known-hosts file holding `K1` for `[bastion.acme.io]:2222`.
fn seed_hacivat_store(vcx: &VisualTestContext) -> PathBuf {
    let path = vcx.cx.read(|cx| {
        cx.global::<ConnectionsModel>()
            .known_hosts()
            .hacivat_path()
            .to_path_buf()
    });
    std::fs::create_dir_all(path.parent().expect("a parent")).expect("the data folder");
    std::fs::write(&path, format!("[bastion.acme.io]:2222 {K1}\n")).expect("the store");
    path
}

fn store_holds(path: &PathBuf, key_line: &str) -> bool {
    let body = std::fs::read_to_string(path).unwrap_or_default();
    let blob = key_line.split(' ').nth(1).expect("a blob");
    body.contains(blob)
}

#[gpui::test]
fn host_key_changed_enter_cancels(cx: &mut TestAppContext) {
    let vcx = shell(cx);
    let store = seed_hacivat_store(vcx);
    review(vcx, mismatch(KnownSource::Hacivat));
    press(vcx, "enter");
    assert!(!layer_open(vcx, "hk-changed"));
    assert!(store_holds(&store, K1), "nothing was replaced");
    assert!(!store_holds(&store, K2));

    // Q34: Enter cancels even with the focus on the trust action; only a click or Space runs it.
    review(vcx, mismatch(KnownSource::Hacivat));
    press(vcx, "shift-tab");
    assert_eq!(focused_id(vcx), "hk-replace");
    press(vcx, "enter");
    assert!(!layer_open(vcx, "hk-changed"));
    assert!(store_holds(&store, K1) && !store_holds(&store, K2));
}

#[gpui::test]
fn host_key_replace_rewrites_only_hacivats_store(cx: &mut TestAppContext) {
    let vcx = shell(cx);
    let store = seed_hacivat_store(vcx);
    review(vcx, mismatch(KnownSource::Hacivat));
    click_button(vcx, "hk-replace");
    wait_until(vcx, "the key to be replaced", |vcx| {
        store_holds(&store, K2) && !layer_ids(vcx).contains(&"hk-changed")
    });
    assert!(!store_holds(&store, K1), "the old key is gone");
    assert!(
        toast_texts(vcx)
            .iter()
            .any(|s| s.contains("bastion.acme.io")),
        "the trusted toast names the host"
    );
}

#[gpui::test]
fn host_key_replace_runs_from_space_on_the_trust_action(cx: &mut TestAppContext) {
    let vcx = shell(cx);
    let store = seed_hacivat_store(vcx);
    review(vcx, mismatch(KnownSource::Hacivat));
    press(vcx, "shift-tab");
    press(vcx, "space");
    wait_until(vcx, "the key to be replaced", |_| store_holds(&store, K2));
}

#[gpui::test]
fn host_key_review_of_a_known_hosts2_entry_adds_the_file(cx: &mut TestAppContext) {
    let vcx = shell(cx);
    let home = std::env::home_dir().expect("a home directory");
    review(
        vcx,
        HostKeyMismatch {
            port: 22,
            ..mismatch(KnownSource::OpenSsh {
                path: home.join(".ssh").join("known_hosts2"),
                line: 3,
            })
        },
    );
    assert_eq!(
        text(vcx, "hk-keygen"),
        "ssh-keygen -f ~/.ssh/known_hosts2 -R bastion.acme.io"
    );
}

#[gpui::test]
fn host_key_changed_error_offers_the_review(cx: &mut TestAppContext) {
    let vcx = shell(cx);
    let id = add_profile(vcx, ssh_profile("orders-prod"));
    let error = Arc::new(ConnectError {
        class: ErrClass::SshHostKeyChanged,
        phase: Phase::Transport,
        trail: Vec::new(),
        args: ErrArgs {
            name: "orders-prod".into(),
            host: Some("10.0.0.5".into()),
            port: Some(5432),
            user: Some("deploy".into()),
            database: None,
            file: None,
            engine: "PostgreSQL".into(),
            version: None,
            hop: Some(1),
            hop_host: Some("bastion.acme.io".into()),
        },
        server: None,
        fixes: vec![FixId::ReviewHostKey],
        suggestions: Vec::new(),
        issues: Vec::new(),
        elapsed: Duration::ZERO,
        attempts: 0,
        needs_interaction: false,
        certificate: None,
        host_key: Some(mismatch(KnownSource::Hacivat)),
        profile_changes: Vec::new(),
    });
    in_window(vcx, |window, cx| open_error_dialog(id, error, window, cx));
    vcx.run_until_parked();
    assert!(visible(vcx, "fix-review-host-key"));
    click_button(vcx, "fix-review-host-key");
    settle_until(vcx, WAIT, || layer_ids(vcx) == ["hk-changed"]);
    assert_eq!(focused_id(vcx), "hk-cancel");
}

// ---- keyboard-interactive (surface 2) --------------------------------------------------------

#[gpui::test]
fn kbd_prompt_renders_server_text_verbatim_and_masks_non_echo(cx: &mut TestAppContext) {
    let vcx = shell(cx);
    let a = ask(
        vcx,
        kbd_question(
            "Duo",
            "Kod girin",
            &[("Doğrulama kodu: ", false), ("Kullanıcı: ", true)],
        ),
    );
    assert_eq!(text(vcx, "kbd-name"), "Duo");
    assert_eq!(text(vcx, "kbd-instructions"), "Kod girin");
    assert_eq!(text(vcx, "kbd-label-0"), "Doğrulama kodu: ");
    assert!(is_masked(vcx, "kbd-0") && !is_masked(vcx, "kbd-1"));
    assert_eq!(
        focused_id(vcx),
        "kbd-0",
        "the focus starts in the first prompt"
    );
    type_into(vcx, "kbd-0", "123456");
    type_into(vcx, "kbd-1", "deploy");
    press(vcx, "enter");
    let answered = a.get();
    let Some(Answer::Keyboard(v)) = &*answered else {
        panic!("the answers");
    };
    assert_eq!(v.len(), 2);
    assert!(
        v[0].expose_secret() == "123456" && v[1].expose_secret() == "deploy",
        "the answers in the server's order (values not shown)"
    );
    assert!(!layer_open(vcx, "kbd-prompt"));
}

#[gpui::test]
fn kbd_enter_advances_and_never_spends_a_half_filled_round(cx: &mut TestAppContext) {
    let vcx = shell(cx);
    let a = ask(
        vcx,
        kbd_question(
            "",
            "",
            &[("Password: ", false), ("Verification code: ", true)],
        ),
    );
    assert_eq!(
        text(vcx, "kbd-no-text"),
        t(vcx, "hv-conn-kbd-no-text"),
        "no server text: our own words, without the caption"
    );
    // Enter in a prompt that isn't the last moves on, even when it is empty.
    press(vcx, "enter");
    assert_eq!(focused_id(vcx), "kbd-1");
    assert!(a.get().is_none());
    // Enter in the last prompt with an empty masked prompt goes back to it.
    vcx.simulate_input("424242");
    press(vcx, "enter");
    assert_eq!(focused_id(vcx), "kbd-0");
    assert!(a.get().is_none(), "nothing was sent");
    vcx.simulate_input("hv_test_pw");
    press(vcx, "tab");
    press(vcx, "enter");
    assert!(matches!(&*a.get(), Some(Answer::Keyboard(v)) if v.len() == 2));
}

#[gpui::test]
fn kbd_server_text_is_made_visible_and_capped(cx: &mut TestAppContext) {
    let vcx = shell(cx);
    let hostile_name = "\u{1b}[1;32mHacivat: identity verified\u{1b}[0m";
    let hostile_text = format!(
        "Paste the code from \u{202E}exe.edoc\u{202C}\n{}",
        "x".repeat(5000)
    );
    let _a = ask(
        vcx,
        kbd_question(hostile_name, &hostile_text, &[("Code:\u{1b}[8m ", true)]),
    );
    assert_eq!(
        text(vcx, "kbd-name"),
        "\\033[1;32mHacivat: identity verified\\033[0m"
    );
    let shown = text(vcx, "kbd-instructions");
    assert!(shown.starts_with("Paste the code from \\u{202E}exe.edoc\\u{202C}\n"));
    assert!(
        shown.chars().count() < 2100 && shown.ends_with('…'),
        "capped"
    );
    assert_eq!(text(vcx, "kbd-label-0"), "Code:\\033[8m ");
    // Never inside our own sentence: the lead names only the host Hacivat dialled.
    assert_eq!(
        text(vcx, "kbd-asks"),
        plain(vcx.cx.read(|cx| tr(
            cx,
            ids::hv::CONN_KBD_ASKS,
            ids::hv::HvConnKbdAsks {
                host: hacivat_l10n::Text("bastion.acme.io".into())
            }
        )))
    );
}

#[gpui::test]
fn kbd_round_with_too_many_prompts_is_refused(cx: &mut TestAppContext) {
    let vcx = shell(cx);
    let many: Vec<(&str, bool)> = (0..9).map(|_| ("Code: ", false)).collect();
    let a = ask(vcx, kbd_question("", "", &many));
    settle_until(vcx, WAIT, || a.get().is_some());
    assert!(matches!(*a.get(), Some(Answer::Cancelled)));
    assert!(layer_ids(vcx).is_empty(), "no dialog full of fields");
}

#[gpui::test]
fn kbd_rejected_round_says_so(cx: &mut TestAppContext) {
    let vcx = shell(cx);
    let mut q = kbd_question("", "", &[("Code: ", false)]);
    if let Question::Keyboard { attempt, .. } = &mut q {
        *attempt = 2;
    }
    let _a = ask(vcx, q);
    assert_eq!(text(vcx, "kbd-rejected"), t(vcx, "hv-conn-kbd-rejected"));
    assert_eq!(
        text(vcx, "kbd-attempt"),
        plain(vcx.cx.read(|cx| tr(
            cx,
            ids::hv::CONN_KBD_ATTEMPT,
            ids::hv::HvConnKbdAttempt { n: Count(2) }
        )))
    );
}

// ---- SSH password and passphrase (surface 3) ----------------------------------------------------

#[gpui::test]
fn ssh_secret_prompt_names_the_hop(cx: &mut TestAppContext) {
    let vcx = shell(cx);
    let id = add_profile(vcx, ssh_profile("orders-prod"));
    let a = ask(vcx, ssh_password_question(id, 0, "jump-eu.acme.io", 2));
    assert!(text(vcx, "secret-title").contains("jump-eu.acme.io"));
    assert!(is_masked(vcx, "secret-input"));
    assert_eq!(focused_id(vcx), "secret-input");
    let drawn = texts_drawn_by(vcx, "connections/prompt_kit.rs").join("|");
    assert!(drawn.contains("jump-eu.acme.io, hop 1 of 2"), "{drawn}");
    // The route boxes the asking hop; the attempt sits under the field.
    assert_eq!(text(vcx, "route-1"), "jump-eu.acme.io");
    assert!(vcx.debug_bounds("route-asking").is_some());
    assert_eq!(text(vcx, "secret-user"), "deploy");
    assert_eq!(
        text(vcx, "secret-attempt"),
        plain(vcx.cx.read(|cx| tr(
            cx,
            ids::hv::CONN_SSH_SECRET_ATTEMPT,
            ids::hv::HvConnSshSecretAttempt { n: Count(2) }
        )))
    );
    // Remember starts where the question says: the saved ssh_retention (Forever, the
    // keyring's label here).
    assert!(
        select_texts(vcx).contains(&t(vcx, "hv-conn-ret-forever")),
        "{:?}",
        select_texts(vcx)
    );
    assert_eq!(
        text(vcx, "secret-remember-all"),
        t(vcx, "hv-conn-ssh-secret-remember-all")
    );
    type_into(vcx, "secret-input", "hv_test_pw");
    press(vcx, "enter");
    assert!(matches!(
        &*a.get(),
        Some(Answer::SshSecret { secret, remember: Retention::Forever })
            if secret.expose_secret() == "hv_test_pw"
    ));
    assert!(!layer_open(vcx, "secret-prompt"));
}

/// What the Selects of the frame draw (their current values).
fn select_texts(vcx: &VisualTestContext) -> Vec<String> {
    texts_drawn_by(vcx, "hacivat-ui/src/select.rs")
}

/// Fix round 1 (Remember seam): the prompt's Remember travels with the answer.
#[gpui::test]
fn ssh_secret_prompt_answers_the_chosen_remember(cx: &mut TestAppContext) {
    let vcx = shell(cx);
    let id = add_profile(vcx, ssh_profile("orders-prod"));
    let a = ask(vcx, ssh_password_question(id, 0, "jump-eu.acme.io", 1));
    // Tab to the Select, open it with Enter, one row up (For this session), pick it.
    press(vcx, "tab");
    assert_eq!(focused_id(vcx), "secret-remember");
    press(vcx, "enter");
    press(vcx, "up");
    press(vcx, "enter");
    assert!(
        a.get().is_none(),
        "Enter in the Select picks, it never submits"
    );
    assert!(select_texts(vcx).contains(&t(vcx, "hv-conn-ret-session")));
    type_into(vcx, "secret-input", "hv_test_pw");
    press(vcx, "enter");
    assert!(matches!(
        &*a.get(),
        Some(Answer::SshSecret {
            remember: Retention::ForSession,
            ..
        })
    ));
    // Fix round 2 (R2): the prompt keeps nothing. The next question starts where it says: this
    // same connect's choice, or the saved retention.
    let b = ask(
        vcx,
        ssh_password_question_remembering(id, 1, "bastion.acme.io", 1, Some(Retention::ForSession)),
    );
    assert!(select_texts(vcx).contains(&t(vcx, "hv-conn-ret-session")));
    press(vcx, "escape");
    assert!(matches!(&*b.get(), Some(Answer::Cancelled)));
    let _c = ask(vcx, ssh_password_question(id, 1, "bastion.acme.io", 1));
    assert!(select_texts(vcx).contains(&t(vcx, "hv-conn-ret-forever")));
    assert!(!select_texts(vcx).contains(&t(vcx, "hv-conn-ret-session")));
}

/// Fix round 2 (R2, the review's r1-f and r1-g at the prompt): a bare Enter answers the
/// retention the question starts from, whatever an earlier prompt chose; a Test's question
/// (no Remember) shows the static Never row and answers the plain secret.
#[gpui::test]
fn a_bare_enter_answers_the_questions_remember(cx: &mut TestAppContext) {
    let vcx = shell(cx);
    let id = add_profile(vcx, ssh_profile("orders-prod"));
    // A Test: no Select, "Never", a plain secret.
    let test = ask(
        vcx,
        ssh_password_question_remembering(id, 0, "jump-eu.acme.io", 1, None),
    );
    assert!(select_texts(vcx).is_empty());
    assert_eq!(text(vcx, "secret-remember"), t(vcx, "hv-conn-ret-never"));
    type_into(vcx, "secret-input", "hv_test_pw");
    press(vcx, "enter");
    assert!(matches!(&*test.get(), Some(Answer::Secret(_))));
    // A connect picks Forever explicitly (Tab to the Select, open, three rows down from Never).
    let picked = ask(
        vcx,
        ssh_password_question_remembering(id, 0, "jump-eu.acme.io", 1, Some(Retention::Never)),
    );
    press(vcx, "tab");
    press(vcx, "enter");
    for _ in 0..3 {
        press(vcx, "down");
    }
    press(vcx, "enter");
    assert!(select_texts(vcx).contains(&t(vcx, "hv-conn-ret-forever")));
    type_into(vcx, "secret-input", "hv_test_pw");
    press(vcx, "enter");
    assert!(matches!(
        &*picked.get(),
        Some(Answer::SshSecret {
            remember: Retention::Forever,
            ..
        })
    ));
    // The user set Never again; the next connect asks from Never and a bare Enter keeps it.
    let next = ask(
        vcx,
        ssh_password_question_remembering(id, 0, "jump-eu.acme.io", 1, Some(Retention::Never)),
    );
    assert!(select_texts(vcx).contains(&t(vcx, "hv-conn-ret-never")));
    type_into(vcx, "secret-input", "hv_test_pw");
    press(vcx, "enter");
    assert!(matches!(
        &*next.get(),
        Some(Answer::SshSecret { secret, remember: Retention::Never })
            if secret.expose_secret() == "hv_test_pw"
    ));
}

/// A TLS key passphrase keeps the static row (the DB retention) and answers a plain secret.
#[gpui::test]
fn tls_key_passphrase_prompt_has_no_remember_choice(cx: &mut TestAppContext) {
    let vcx = shell(cx);
    let id = add_profile(vcx, ssh_profile("orders-prod"));
    let a = ask(
        vcx,
        Question::Secret {
            profile: id,
            name: "orders-prod".into(),
            prompt: SecretPrompt::TlsKeyPassphrase {
                key: PathBuf::from("/keys/client.key"),
            },
            attempt: 1,
            remember: None,
        },
    );
    assert_eq!(text(vcx, "secret-remember"), t(vcx, "hv-conn-ret-forever"));
    assert!(select_texts(vcx).is_empty());
    type_into(vcx, "secret-input", "hv_test_pw");
    press(vcx, "enter");
    assert!(matches!(&*a.get(), Some(Answer::Secret(_))));
}

#[gpui::test]
fn ssh_passphrase_prompt_names_the_key_and_empty_enter_sends_nothing(cx: &mut TestAppContext) {
    let vcx = shell(cx);
    let id = add_profile(vcx, ssh_profile("orders-prod"));
    let a = ask(
        vcx,
        Question::Secret {
            profile: id,
            name: "orders-prod".into(),
            prompt: SecretPrompt::SshPassphrase {
                hop: 1,
                key: PathBuf::from("/keys/id_ed25519"),
            },
            attempt: 1,
            remember: Some(Retention::Forever),
        },
    );
    assert!(text(vcx, "secret-title").contains("/keys/id_ed25519"));
    assert_eq!(text(vcx, "secret-key"), "/keys/id_ed25519");
    let drawn = texts_drawn_by(vcx, "connections/prompt_kit.rs").join("|");
    assert!(drawn.contains("bastion.acme.io, hop 2 of 2"), "{drawn}");
    press(vcx, "enter");
    assert!(a.get().is_none(), "an empty field answers nothing");
    press(vcx, "escape");
    assert!(matches!(*a.get(), Some(Answer::Cancelled)));
}

// ---- the vault (surface 4) -----------------------------------------------------------------------

#[gpui::test]
fn vault_create_requires_matching_8_char_passwords(cx: &mut TestAppContext) {
    let vcx = shell(cx);
    let a = ask(vcx, Question::CreateVault);
    assert_eq!(focused_id(vcx), "vault-new");
    type_into(vcx, "vault-new", "short");
    type_into(vcx, "vault-repeat", "short");
    click_button(vcx, "vault-create");
    assert!(
        a.get().is_none(),
        "Create stays disabled under 8 characters"
    );
    set_field(vcx, "vault-new", "correct horse");
    set_field(vcx, "vault-repeat", "correct horsE");
    assert_eq!(
        text(vcx, "vault-rule-match"),
        t(vcx, "hv-conn-vault-mismatch")
    );
    click_button(vcx, "vault-create");
    assert!(
        a.get().is_none(),
        "Create stays disabled while the entries differ"
    );
    set_field(vcx, "vault-new", "correct horse");
    set_field(vcx, "vault-repeat", "correct horse");
    assert_eq!(text(vcx, "vault-rule-match"), t(vcx, "hv-conn-vault-match"));
    click_button(vcx, "vault-create");
    assert!(matches!(&*a.get(), Some(Answer::Master(m)) if m.expose_secret() == "correct horse"));
    assert!(!layer_open(vcx, "vault-dialog"));
}

#[gpui::test]
fn vault_create_enter_moves_then_creates(cx: &mut TestAppContext) {
    let vcx = shell(cx);
    let a = ask(vcx, Question::CreateVault);
    vcx.simulate_input(MASTER);
    press(vcx, "enter");
    assert_eq!(focused_id(vcx), "vault-repeat");
    assert!(a.get().is_none());
    vcx.simulate_input(MASTER);
    press(vcx, "enter");
    assert!(matches!(*a.get(), Some(Answer::Master(_))));
}

#[gpui::test]
fn vault_unlock_counts_attempts(cx: &mut TestAppContext) {
    let vcx = shell_with_vault(cx);
    let _a = ask(vcx, Question::UnlockVault { attempt: 2 });
    assert_eq!(
        text(vcx, "vault-attempt"),
        plain(vcx.cx.read(|cx| tr(
            cx,
            ids::hv::CONN_VAULT_ATTEMPT,
            ids::hv::HvConnVaultAttempt { n: Count(2) }
        )))
    );
    assert_eq!(
        text(vcx, "vault-unlock-wrong"),
        t(vcx, "hv-conn-vault-unlock-wrong")
    );
}

#[gpui::test]
fn vault_unlock_answers_the_master_password(cx: &mut TestAppContext) {
    let vcx = shell_with_vault(cx);
    let a = ask(vcx, Question::UnlockVault { attempt: 1 });
    assert!(
        text_of(vcx, "vault-attempt").is_none(),
        "no attempt line on the first try"
    );
    press(vcx, "enter");
    assert!(
        a.get().is_none(),
        "Unlock is disabled while the field is empty"
    );
    type_into(vcx, "vault-unlock", MASTER);
    press(vcx, "enter");
    assert!(matches!(&*a.get(), Some(Answer::Master(m)) if m.expose_secret() == MASTER));
}

/// Opens the unlock dialog the pipeline asks for and, once the blocking pool said the vault
/// may be reset, its reset confirmation.
fn open_vault_reset(vcx: &mut VisualTestContext) -> Asked {
    let a = ask(vcx, Question::UnlockVault { attempt: 1 });
    wait_until_mut(vcx, "the reset entry", |vcx| {
        vcx.debug_bounds("button-vault-reset-open").is_some()
    });
    click_button(vcx, "vault-reset-open");
    settle_until(vcx, WAIT, || layer_open(vcx, "vault-reset"));
    a
}

#[gpui::test]
fn vault_reset_needs_typed_confirmation(cx: &mut TestAppContext) {
    let vcx = shell_with_vault(cx);
    let a = open_vault_reset(vcx);
    assert_eq!(focused_id(vcx), "vault-reset-word");
    click_button(vcx, "vault-reset-confirm");
    assert!(
        vault_exists(vcx),
        "the Danger button is disabled until the word is typed"
    );
    type_into(vcx, "vault-reset-word", "Reset");
    click_button(vcx, "vault-reset-confirm");
    assert!(vault_exists(vcx), "the word must match exactly");
    set_field(vcx, "vault-reset-word", &t(vcx, "hv-conn-vault-reset-word"));
    press(vcx, "enter");
    assert!(vault_exists(vcx), "Enter never resets");
    click_button(vcx, "vault-reset-confirm");
    settle_until(vcx, WAIT, || !vault_exists(vcx));
    settle_until(vcx, WAIT, || a.get().is_some());
    assert!(
        matches!(*a.get(), Some(Answer::Cancelled)),
        "the unlock answers Cancelled"
    );
    assert!(
        layer_ids(vcx).is_empty(),
        "the reset and the unlock both closed"
    );
}

#[gpui::test]
fn vault_reset_escape_returns_to_unlock(cx: &mut TestAppContext) {
    let vcx = shell_with_vault(cx);
    let a = open_vault_reset(vcx);
    press(vcx, "escape");
    assert_eq!(layer_ids(vcx), ["vault-dialog"]);
    assert!(a.get().is_none());
    assert!(vault_exists(vcx));
}

/// What the blocking pool said about resetting the vault, for the open unlock dialog.
#[cfg(unix)]
fn reset_probe(vcx: &VisualTestContext) -> Option<bool> {
    vcx.cx.read(|cx| parts::flag(cx, "vault-resettable"))
}

#[cfg(unix)]
#[gpui::test]
fn a_symlinked_vault_is_never_offered_a_reset(cx: &mut TestAppContext) {
    let (vcx, _vault, path) = shell_with_vault_at(cx, false);
    // The vault file becomes a link to a file elsewhere (a dotfiles arrangement).
    let elsewhere = tmp_dir().join("elsewhere.bin");
    std::fs::rename(&path, &elsewhere).expect("moved");
    std::os::unix::fs::symlink(&elsewhere, &path).expect("linked");
    let _a = ask(vcx, Question::UnlockVault { attempt: 1 });
    settle_until(vcx, WAIT, || reset_probe(vcx).is_some());
    assert_eq!(reset_probe(vcx), Some(false));
    assert!(!visible(vcx, "vault-reset-open"), "N3: never offered");
    assert!(std::fs::symlink_metadata(&path).is_ok_and(|m| m.file_type().is_symlink()));
}

#[gpui::test]
fn prompt_inputs_mask_text_for_range(cx: &mut TestAppContext) {
    let vcx = shell_with_vault(cx);
    let _a = ask(vcx, Question::UnlockVault { attempt: 1 });
    type_into(vcx, "vault-unlock", "s3cret");
    assert!(!ime_text(vcx, "vault-unlock").contains("s3cret"));
    assert_eq!(
        ime_text(vcx, "vault-unlock").chars().count(),
        6,
        "one bullet per unit"
    );
}

#[gpui::test]
fn every_prompt_field_is_cleared_when_it_answers(cx: &mut TestAppContext) {
    let vcx = shell(cx);
    let _a = ask(
        vcx,
        ssh_password_question(ProfileId::new(), 0, "jump-eu.acme.io", 1),
    );
    let field = input(vcx, "secret-input");
    type_into(vcx, "secret-input", "hv_test_pw");
    press(vcx, "enter");
    assert!(
        vcx.cx.read(|cx| field.read(cx).value().is_empty()),
        "the answered field is cleared"
    );
    let _b = ask(vcx, kbd_question("", "", &[("Code: ", false)]));
    let code = input(vcx, "kbd-0");
    type_into(vcx, "kbd-0", "424242");
    press(vcx, "escape");
    assert!(
        vcx.cx.read(|cx| code.read(cx).value().is_empty()),
        "a cancel clears too"
    );
}

// ---- the Remember label and the first Forever save ---------------------------------------------

fn open_new_dialog(vcx: &VisualTestContext) {
    in_window(vcx, |window, cx| {
        open_connection_dialog(DialogMode::New, window, cx)
    });
    vcx.run_until_parked();
}

/// The label of `retention` in the connection dialog's Remember select (its own option list).
fn remember_label(vcx: &VisualTestContext, retention: Retention) -> String {
    let at = Retention::ALL
        .iter()
        .position(|r| *r == retention)
        .expect("a retention");
    let dialog = in_window(vcx, |window, cx| open_dialog(window, cx)).expect("the dialog");
    let select = vcx.cx.read(|cx| dialog.read(cx).remember_select());
    plain(vcx.cx.read(|cx| (select.read(cx).options()[at].label)(cx)))
}

#[gpui::test]
fn forever_label_reads_encrypted_file_when_vault(cx: &mut TestAppContext) {
    let vcx = shell_with_fallback_probe_false(cx);
    open_new_dialog(vcx);
    // The dialog's keyring probe chooses the backend on the core runtime (P36).
    settle_until(vcx, WAIT, || {
        backend_choice(vcx) == Some(BackendChoice::Vault)
    });
    assert_eq!(
        remember_label(vcx, Retention::Forever),
        t(vcx, "hv-conn-ret-forever-vault")
    );
}

#[gpui::test]
fn forever_label_is_the_keyrings_otherwise(cx: &mut TestAppContext) {
    let vcx = shell(cx);
    open_new_dialog(vcx);
    assert_eq!(
        remember_label(vcx, Retention::Forever),
        t(vcx, "hv-conn-ret-forever")
    );
}

/// A new profile kept Forever, with a typed password.
fn forever_draft() -> ProfileDraft {
    let mut p = test_profile("orders-dev", Some(EnvTag::Dev), "10.0.0.5");
    p.auth = AuthProviderId::Password;
    p.retention = Retention::Forever;
    ProfileDraft {
        is_new: true,
        password: Some(SecretString::from("hv_test_pw")),
        ..ProfileDraft::edit(p)
    }
}

fn save_forever_profile(
    vcx: &VisualTestContext,
) -> (
    ProfileId,
    gpui::Task<Result<ProfileId, hacivat_app::connections::SaveError>>,
) {
    let draft = forever_draft();
    let id = draft.profile.id;
    let task = vcx.cx.update(|cx| ConnectionsModel::save(cx, draft));
    vcx.run_until_parked();
    (id, task)
}

#[gpui::test]
fn first_forever_save_asks_to_create_vault(cx: &mut TestAppContext) {
    let vcx = shell_with_fallback_probe_false_no_vault(cx);
    open_new_dialog(vcx);
    settle_until(vcx, WAIT, || backend_choice(vcx).is_some());
    let (id, task) = save_forever_profile(vcx);
    settle_until(vcx, WAIT, || layer_open(vcx, "vault-dialog"));
    // M7: the Forever put waits for the vault, so nothing is stored yet.
    assert!(
        stored(vcx, id, SecretSlot::DbPassword, Retention::UntilRestart).is_none(),
        "nothing stored yet"
    );
    set_field(vcx, "vault-new", MASTER);
    set_field(vcx, "vault-repeat", MASTER);
    click_button(vcx, "vault-create");
    let saved = wait_task(vcx, task);
    assert!(saved.is_ok(), "{saved:?}");
    assert!(vault_exists(vcx));
    let vault = secrets(vcx);
    let in_vault = vault
        .vault()
        .expect("the vault")
        .get(&SecretKey {
            profile: id,
            slot: SecretSlot::DbPassword,
        })
        .expect("readable")
        .is_some_and(|s| s.expose_secret() == "hv_test_pw");
    assert!(in_vault, "the password landed in the new vault");
    assert!(
        toast_texts(vcx).contains(&t(vcx, "hv-conn-vault-created")),
        "{:?}",
        toast_texts(vcx)
    );
}

#[gpui::test]
fn declining_the_vault_keeps_the_secret_until_restart(cx: &mut TestAppContext) {
    let vcx = shell_with_fallback_probe_false_no_vault(cx);
    let (id, task) = save_forever_profile(vcx);
    settle_until(vcx, WAIT, || layer_open(vcx, "vault-dialog"));
    click_button(vcx, "vault-not-now");
    let saved = wait_task(vcx, task);
    assert!(saved.is_ok(), "{saved:?}");
    assert!(!vault_exists(vcx), "nothing was created");
    assert!(
        stored(vcx, id, SecretSlot::DbPassword, Retention::Forever).as_deref()
            == Some("hv_test_pw"),
        "kept in memory, the Forever fallback (values not shown)"
    );
    assert!(toast_texts(vcx).contains(&t(vcx, "hv-conn-keyring-unavailable")));
}

#[gpui::test]
fn a_save_without_a_forever_secret_never_asks_for_the_vault(cx: &mut TestAppContext) {
    let vcx = shell_with_fallback_probe_false_no_vault(cx);
    let mut draft = forever_draft();
    draft.profile.retention = Retention::UntilRestart;
    let task = vcx.cx.update(|cx| ConnectionsModel::save(cx, draft));
    let saved = wait_task(vcx, task);
    assert!(saved.is_ok(), "{saved:?}");
    assert!(layer_ids(vcx).is_empty());
}

/// P15 with a vault: a Forever save that needs no dialog (the vault is there and unlocked)
/// never waits for the UI thread, so a quit right after it still drains the write.
#[gpui::test]
fn a_forever_save_into_an_unlocked_vault_drains_at_quit(cx: &mut TestAppContext) {
    let (vcx, vault) = shell_with_vault_state(cx, true);
    let (id, _task) = save_forever_profile(vcx);
    vcx.executor().allow_parking();
    vcx.cx.update(|cx| cx.shutdown());
    assert!(
        vault
            .get(&SecretKey {
                profile: id,
                slot: SecretSlot::DbPassword
            })
            .expect("readable")
            .is_some(),
        "the queued put reached the vault before the runtime stopped"
    );
}

// ---- a locked vault and Hacivat's own gestures --------------------------------------------------

/// A shell with an unlocked vault holding `orders-dev`'s password, which is then locked.
fn locked_vault_with_profile(
    cx: &mut TestAppContext,
) -> (&'static mut VisualTestContext, ProfileId, Arc<FileVault>) {
    let (vcx, vault) = shell_with_vault_state(cx, true);
    let (id, task) = save_forever_profile(vcx);
    let saved = wait_task(vcx, task);
    assert!(saved.is_ok(), "{saved:?}");
    assert!(
        vault
            .get(&SecretKey {
                profile: id,
                slot: SecretSlot::DbPassword
            })
            .expect("readable")
            .is_some(),
        "the password is in the vault"
    );
    vault.lock();
    (vcx, id, vault)
}

#[gpui::test]
fn delete_with_a_locked_vault_asks_to_unlock_then_deletes(cx: &mut TestAppContext) {
    let (vcx, id, vault) = locked_vault_with_profile(cx);
    let task = vcx.cx.update(|cx| ConnectionsModel::delete(cx, id));
    settle_until(vcx, WAIT, || layer_open(vcx, "vault-dialog"));
    assert!(text(vcx, "vault-unlock-body").contains("orders-dev"));
    type_into(vcx, "vault-unlock", MASTER);
    press(vcx, "enter");
    let deleted = wait_task(vcx, task);
    assert!(deleted.is_ok(), "{deleted:?}");
    assert!(vault.is_unlocked());
    assert!(
        vault
            .get(&SecretKey {
                profile: id,
                slot: SecretSlot::DbPassword
            })
            .expect("readable")
            .is_none(),
        "the secret left the vault with the profile"
    );
}

#[gpui::test]
fn a_cancel_after_a_wrong_master_password_starts_the_next_gesture_at_attempt_1(
    cx: &mut TestAppContext,
) {
    let (vcx, id, vault) = locked_vault_with_profile(cx);
    let task = vcx.cx.update(|cx| ConnectionsModel::duplicate(cx, id));
    settle_until(vcx, WAIT, || layer_open(vcx, "vault-dialog"));
    type_into(vcx, "vault-unlock", "notthemaster");
    press(vcx, "enter");
    wait_until(vcx, "the second attempt", |vcx| {
        text_of(vcx, "vault-attempt").is_some()
    });
    assert_eq!(
        text(vcx, "vault-attempt"),
        plain(vcx.cx.read(|cx| tr(
            cx,
            ids::hv::CONN_VAULT_ATTEMPT,
            ids::hv::HvConnVaultAttempt { n: Count(2) }
        )))
    );
    assert!(
        value(vcx, "vault-unlock").is_empty(),
        "the miss cleared the field"
    );
    press(vcx, "escape");
    let copied = wait_task(vcx, task);
    assert!(copied.is_err(), "the copy is reported, not lost silently");
    assert!(!vault.is_unlocked());

    // A new gesture starts over at attempt 1.
    let again = vcx.cx.update(|cx| ConnectionsModel::delete(cx, id));
    settle_until(vcx, WAIT, || layer_open(vcx, "vault-dialog"));
    assert!(text_of(vcx, "vault-attempt").is_none());
    press(vcx, "escape");
    let _ = wait_task(vcx, again);
}

#[gpui::test]
fn three_wrong_master_passwords_end_the_gesture(cx: &mut TestAppContext) {
    let (vcx, id, vault) = locked_vault_with_profile(cx);
    let task = vcx.cx.update(|cx| ConnectionsModel::delete(cx, id));
    settle_until(vcx, WAIT, || layer_open(vcx, "vault-dialog"));
    for n in 1..=3u64 {
        type_into(vcx, "vault-unlock", "notthemaster");
        press(vcx, "enter");
        if n < 3 {
            let next = attempt_text(vcx, n + 1);
            wait_until(vcx, "the next attempt", |vcx| {
                text_of(vcx, "vault-attempt").as_deref() == Some(next.as_str())
            });
        }
    }
    let deleted = wait_task(vcx, task);
    assert!(deleted.is_err());
    assert!(!vault.is_unlocked());
    assert!(layer_ids(vcx).is_empty());
}

// ---- SSH slots follow their hop (T20 carry) ------------------------------------------------------

fn put_ssh(vcx: &VisualTestContext, id: ProfileId, hop: u8, secret: &str) {
    secrets(vcx)
        .put(
            &SecretKey {
                profile: id,
                slot: SecretSlot::SshPassword { hop },
            },
            SecretString::from(secret),
            Retention::UntilRestart,
        )
        .expect("stored");
}

fn ssh_pw(vcx: &VisualTestContext, id: ProfileId, hop: u8) -> Option<String> {
    stored(
        vcx,
        id,
        SecretSlot::SshPassword { hop },
        Retention::UntilRestart,
    )
}

fn save_hops(vcx: &VisualTestContext, id: ProfileId, hops: Vec<HopSpec>) {
    let mut p = (*vcx
        .cx
        .read(|cx| cx.global::<ConnectionsModel>().get(id))
        .expect("saved"))
    .clone();
    if let TransportSpec::SshTunnel(spec) = &mut p.transport {
        spec.hops = hops;
    }
    let task = vcx
        .cx
        .update(|cx| ConnectionsModel::save(cx, ProfileDraft::edit(p)));
    let saved = wait_task(vcx, task);
    assert!(saved.is_ok(), "{saved:?}");
}

#[gpui::test]
fn ssh_secrets_follow_their_hop_when_hops_move(cx: &mut TestAppContext) {
    let vcx = shell(cx);
    let mut p = ssh_profile("orders-prod");
    p.ssh_retention = Retention::UntilRestart;
    let jump = hop("jump-eu.acme.io", 22, SshAuth::Password);
    let id = add_profile(vcx, p);
    put_ssh(vcx, id, 0, "hv_test_pw_jump");
    put_ssh(vcx, id, 1, "hv_test_pw_bastion");

    // Reordered: each secret moves with its hop.
    save_hops(vcx, id, vec![bastion(), jump.clone()]);
    assert!(
        ssh_pw(vcx, id, 0).as_deref() == Some("hv_test_pw_bastion"),
        "hop 0 (values not shown)"
    );
    assert!(
        ssh_pw(vcx, id, 1).as_deref() == Some("hv_test_pw_jump"),
        "hop 1 (values not shown)"
    );

    // A hop added in front: both move down one.
    let extra = hop("jump-us.acme.io", 22, SshAuth::Agent);
    save_hops(vcx, id, vec![extra.clone(), bastion(), jump.clone()]);
    assert!(ssh_pw(vcx, id, 0).is_none(), "hop 0 holds nothing");
    assert!(
        ssh_pw(vcx, id, 1).as_deref() == Some("hv_test_pw_bastion"),
        "hop 1 (values not shown)"
    );
    assert!(
        ssh_pw(vcx, id, 2).as_deref() == Some("hv_test_pw_jump"),
        "hop 2 (values not shown)"
    );

    // A hop removed: its secret goes; nothing is left past the end.
    save_hops(vcx, id, vec![extra.clone(), bastion()]);
    assert!(
        ssh_pw(vcx, id, 1).as_deref() == Some("hv_test_pw_bastion"),
        "hop 1 (values not shown)"
    );
    assert!(
        ssh_pw(vcx, id, 2).is_none(),
        "no secret waits for a future hop 3"
    );

    // A hop whose user changed is another hop: its secret goes too.
    let mut renamed = bastion();
    renamed.user = "ops".into();
    save_hops(vcx, id, vec![extra, renamed]);
    assert!(
        ssh_pw(vcx, id, 1).is_none(),
        "never a secret on the wrong hop"
    );
}

#[gpui::test]
fn an_edit_that_keeps_the_hops_keeps_their_secrets(cx: &mut TestAppContext) {
    let vcx = shell(cx);
    let mut p = ssh_profile("orders-prod");
    p.ssh_retention = Retention::UntilRestart;
    let id = add_profile(vcx, p);
    put_ssh(vcx, id, 0, "hv_test_pw_jump");
    let mut same = ssh_profile("orders-prod");
    if let TransportSpec::SshTunnel(spec) = &mut same.transport {
        spec.hops[0].host = "JUMP-EU.acme.io".into();
    }
    let hops = match same.transport {
        TransportSpec::SshTunnel(spec) => spec.hops,
        _ => unreachable!(),
    };
    save_hops(vcx, id, hops);
    assert!(
        ssh_pw(vcx, id, 0).as_deref() == Some("hv_test_pw_jump"),
        "host names compare ASCII-folded (values not shown)"
    );
}

/// "Attempt `n` of 3" as the vault dialog draws it.
fn attempt_text(vcx: &VisualTestContext, n: u64) -> String {
    plain(vcx.cx.read(|cx| {
        tr(
            cx,
            ids::hv::CONN_VAULT_ATTEMPT,
            ids::hv::HvConnVaultAttempt { n: Count(n) },
        )
    }))
}

// ---- fix round 1 ----------------------------------------------------------------------------

/// A profile through `a.example` then `b.example` (password hops), its SSH secrets Forever.
fn ab_profile() -> Profile {
    let mut p = test_profile("hops", Some(EnvTag::Dev), "10.0.0.5");
    p.transport = TransportSpec::SshTunnel(SshSpec {
        hops: vec![
            hop("a.example", 22, SshAuth::Password),
            hop("b.example", 22, SshAuth::Password),
        ],
        host_keys: HostKeyPolicy::Ask,
        keepalive_s: None,
    });
    p.ssh_retention = Retention::Forever;
    p
}

fn hop_key(id: ProfileId, hop: u8) -> SecretKey {
    SecretKey {
        profile: id,
        slot: SecretSlot::SshPassword { hop },
    }
}

/// The vault's password at `hop`, compared without printing it.
fn vault_pw_is(vault: &FileVault, id: ProfileId, hop: u8, want: Option<&str>) -> bool {
    let got = vault.get(&hop_key(id, hop)).expect("readable");
    got.as_ref().map(|s| s.expose_secret()) == want
}

fn hosts_of(vcx: &VisualTestContext, id: ProfileId) -> Vec<String> {
    let p = vcx
        .cx
        .read(|cx| cx.global::<ConnectionsModel>().get(id))
        .expect("saved");
    match &p.transport {
        TransportSpec::SshTunnel(spec) => spec.hops.iter().map(|h| h.host.clone()).collect(),
        _ => Vec::new(),
    }
}

fn save_task(
    vcx: &VisualTestContext,
    id: ProfileId,
    hops: Vec<HopSpec>,
) -> gpui::Task<Result<ProfileId, hacivat_app::connections::SaveError>> {
    let mut p = (*vcx
        .cx
        .read(|cx| cx.global::<ConnectionsModel>().get(id))
        .expect("saved"))
    .clone();
    if let TransportSpec::SshTunnel(spec) = &mut p.transport {
        spec.hops = hops;
    }
    vcx.cx
        .update(|cx| ConnectionsModel::save(cx, ProfileDraft::edit(p)))
}

/// Review I1 (the reviewer's hostile scenario): a locked vault, the hops reordered or one
/// removed, the unlock declined. Nothing changes: the stored order stays, and every slot still
/// holds its own hop's password, so a.example's password never reaches b.example.
#[gpui::test]
fn a_hop_change_with_a_declined_unlock_changes_nothing(cx: &mut TestAppContext) {
    let (vcx, vault) = shell_with_vault_state(cx, true);
    let id = add_profile(vcx, ab_profile());
    for (hop, pw) in [(0, "pw-for-a"), (1, "pw-for-b")] {
        secrets(vcx)
            .put(
                &hop_key(id, hop),
                SecretString::from(pw),
                Retention::Forever,
            )
            .expect("stored in the vault");
    }
    let a = hop("a.example", 22, SshAuth::Password);
    let b = hop("b.example", 22, SshAuth::Password);
    for hops in [vec![b.clone(), a.clone()], vec![b.clone()]] {
        vault.lock();
        let task = save_task(vcx, id, hops);
        settle_until(vcx, WAIT, || layer_open(vcx, "vault-dialog"));
        press(vcx, "escape");
        let saved = wait_task(vcx, task);
        assert!(
            matches!(
                saved,
                Err(hacivat_app::connections::SaveError::Unchanged(_))
            ),
            "{saved:?}"
        );
        assert_eq!(hosts_of(vcx, id), ["a.example", "b.example"], "order kept");
        vault.unlock(SecretString::from(MASTER)).expect("unlocked");
        assert!(
            vault_pw_is(&vault, id, 0, Some("pw-for-a")),
            "slot 0 is a's"
        );
        assert!(
            vault_pw_is(&vault, id, 1, Some("pw-for-b")),
            "slot 1 is b's"
        );
    }
    // Unlocked when asked: the move happens, then the new order is saved.
    vault.lock();
    let task = save_task(vcx, id, vec![b, a]);
    settle_until(vcx, WAIT, || layer_open(vcx, "vault-dialog"));
    type_into(vcx, "vault-unlock", MASTER);
    press(vcx, "enter");
    let saved = wait_task(vcx, task);
    assert!(saved.is_ok(), "{saved:?}");
    assert_eq!(hosts_of(vcx, id), ["b.example", "a.example"]);
    assert!(
        vault_pw_is(&vault, id, 0, Some("pw-for-b")),
        "b's password follows b"
    );
    assert!(
        vault_pw_is(&vault, id, 1, Some("pw-for-a")),
        "a's password follows a"
    );
}

/// Review m2: a delete whose unlock is declined keeps the profile and its vault secrets, and
/// says why (`Unchanged`); nothing is orphaned under a dead profile ID.
#[gpui::test]
fn a_delete_with_a_declined_unlock_keeps_the_profile(cx: &mut TestAppContext) {
    let (vcx, id, vault) = locked_vault_with_profile(cx);
    let task = vcx.cx.update(|cx| ConnectionsModel::delete(cx, id));
    settle_until(vcx, WAIT, || layer_open(vcx, "vault-dialog"));
    press(vcx, "escape");
    let deleted = wait_task(vcx, task);
    assert!(
        matches!(
            deleted,
            Err(hacivat_app::connections::SaveError::Unchanged(_))
        ),
        "{deleted:?}"
    );
    assert!(
        vcx.cx
            .read(|cx| cx.global::<ConnectionsModel>().get(id))
            .is_some(),
        "the profile stays"
    );
    vault.unlock(SecretString::from(MASTER)).expect("unlocked");
    let key = SecretKey {
        profile: id,
        slot: SecretSlot::DbPassword,
    };
    assert!(
        vault.get(&key).expect("readable").is_some(),
        "its secret stays with it"
    );
}

/// The Remember seam, app side: a connect's `SshRetention` change re-saves the profile, and the
/// save moves the hops' stored secrets to the new retention.
#[gpui::test]
fn an_ssh_retention_change_re_saves_the_profile_and_moves_the_secrets(cx: &mut TestAppContext) {
    let vcx = shell(cx);
    let id = add_profile(vcx, ssh_profile("orders-prod"));
    secrets(vcx)
        .put(
            &hop_key(id, 1),
            SecretString::from("hv_test_pw"),
            Retention::Forever,
        )
        .expect("stored");
    let manager = vcx
        .cx
        .read(|cx| cx.global::<ConnectionsModel>().manager().clone());
    settle_until(vcx, WAIT, || manager.event_subscribers() > 0);
    manager.publish_event(hacivat_session::ConnectionEvent::ProfileChanges {
        profile: id,
        changes: vec![hacivat_session::pipeline::ProfileChange::SshRetention(
            Retention::UntilRestart,
        )],
    });
    settle_until(vcx, WAIT, || {
        vcx.cx
            .read(|cx| cx.global::<ConnectionsModel>().get(id))
            .is_some_and(|p| p.ssh_retention == Retention::UntilRestart)
    });
    wait_until(vcx, "the hop secret to move", |vcx| {
        secret_backend(vcx)
            .get(&hop_key(id, 1))
            .is_ok_and(|v| v.is_none())
    });
    assert!(
        stored(
            vcx,
            id,
            SecretSlot::SshPassword { hop: 1 },
            Retention::UntilRestart
        )
        .as_deref()
            == Some("hv_test_pw"),
        "kept until restart now (values not shown)"
    );
}

/// Review m7: the buttons are really disabled (their accessibility node says so), not only
/// guarded in their handlers.
#[gpui::test]
fn the_guarded_buttons_are_really_disabled(cx: &mut TestAppContext) {
    let vcx = shell_with_vault(cx);
    let _create = ask(vcx, Question::CreateVault);
    let create = t(vcx, "hv-conn-vault-create-btn");
    assert_eq!(button_disabled(vcx, &create), Some(true));
    set_field(vcx, "vault-new", MASTER);
    set_field(vcx, "vault-repeat", MASTER);
    assert_eq!(button_disabled(vcx, &create), Some(false));
    press(vcx, "escape");

    let _unlock = open_vault_reset(vcx);
    let reset = t(vcx, "hv-conn-vault-reset-btn");
    assert_eq!(button_disabled(vcx, &reset), Some(true));
    set_field(vcx, "vault-reset-word", &t(vcx, "hv-conn-vault-reset-word"));
    assert_eq!(button_disabled(vcx, &reset), Some(false));
    assert_eq!(
        text(vcx, "vault-reset-type-word"),
        "reset",
        "the word is drawn on its own"
    );
}

/// Whether the button labelled `label` is declared disabled in a fresh frame.
fn button_disabled(vcx: &mut VisualTestContext, label: &str) -> Option<bool> {
    use hacivat_ui::popover::{a11y_record, a11y_take};
    a11y_record();
    vcx.cx.update(|cx| cx.refresh_windows());
    vcx.run_until_parked();
    a11y_take()
        .into_iter()
        .rev()
        .find(|d| d.role == Some(gpui::Role::Button) && d.name.as_ref() == label)
        .map(|d| d.disabled)
}

/// Fix round 1 (follow-up): an `SshRetention` change also reaches an open edit dialog of the
/// profile (its draft and its SSH Remember select), as T19's pins, CA file and consent do, so
/// a later Save from that dialog never writes the old retention back.
#[gpui::test]
fn an_open_edit_dialog_takes_an_ssh_retention_change(cx: &mut TestAppContext) {
    use hacivat_app::connections::dialog::DIALOG_LAYER_ID;
    use hacivat_app::connections::dialog::ssh::section_state;

    let vcx = shell(cx);
    // Password hops only: a missing key file would make the dialog refuse to save.
    let id = add_profile(vcx, ab_profile());
    in_window(vcx, |window, cx| {
        open_connection_dialog(DialogMode::Edit(id), window, cx)
    });
    vcx.run_until_parked();
    let task = vcx.cx.update(|cx| {
        ConnectionsModel::apply_changes(
            cx,
            id,
            vec![hacivat_session::pipeline::ProfileChange::SshRetention(
                Retention::UntilRestart,
            )],
        )
    });
    wait_task(vcx, task).expect("saved");
    let dialog = in_window(vcx, |window, cx| open_dialog(window, cx)).expect("the dialog");
    settle_until(vcx, WAIT, || {
        vcx.cx
            .read(|cx| dialog.read(cx).profile().ssh_retention == Retention::UntilRestart)
    });
    let section = in_window(vcx, |window, cx| section_state(window, cx)).expect("SSH section");
    let until_restart = Retention::ALL
        .iter()
        .position(|r| *r == Retention::UntilRestart)
        .expect("a retention");
    settle_until(vcx, WAIT, || {
        vcx.cx
            .read(|cx| section.read(cx).remember_select().read(cx).selected() == until_restart)
    });
    // Save from the dialog: the saved retention stays the new one.
    assert!(run(vcx, "connection_dialog.save"));
    settle_until(vcx, WAIT, || !layer_ids(vcx).contains(&DIALOG_LAYER_ID));
    let saved = vcx
        .cx
        .read(|cx| cx.global::<ConnectionsModel>().get(id))
        .expect("saved");
    assert_eq!(saved.ssh_retention, Retention::UntilRestart);
}

// ---- fix round 2 ----------------------------------------------------------------------------

/// A keyring stand-in for the slot-move tests (fix round 2, R1): `choice()` says Keyring, so no
/// vault gate; the values live in a memory map. Once armed, the reads and puts its rules name
/// fail, and every put and delete is recorded (`n` counts the armed puts of a key, from 0).
#[derive(Default)]
struct Flaky {
    inner: MemoryBackend,
    armed: std::sync::atomic::AtomicBool,
    get_fails: std::sync::Mutex<Vec<(SecretKey, SecretError)>>,
    put_fails: std::sync::Mutex<Vec<(SecretKey, usize, SecretError)>>,
    writes: std::sync::Mutex<Vec<SecretKey>>,
}

impl Flaky {
    fn arm(&self, gets: Vec<(SecretKey, SecretError)>, puts: Vec<(SecretKey, usize, SecretError)>) {
        *self.get_fails.lock().unwrap() = gets;
        *self.put_fails.lock().unwrap() = puts;
        self.armed.store(true, std::sync::atomic::Ordering::SeqCst);
    }

    fn armed(&self) -> bool {
        self.armed.load(std::sync::atomic::Ordering::SeqCst)
    }

    /// The backend's value at `hop`, compared without printing it.
    fn holds(&self, id: ProfileId, hop: u8, want: Option<&str>) -> bool {
        let got = self.inner.get(&hop_key(id, hop)).expect("readable");
        got.as_ref().map(|s| s.expose_secret()) == want
    }
}

impl SecretBackend for Flaky {
    fn get(&self, key: &SecretKey) -> Result<Option<SecretString>, SecretError> {
        if self.armed()
            && let Some((_, e)) = self
                .get_fails
                .lock()
                .unwrap()
                .iter()
                .find(|(k, _)| k == key)
        {
            return Err(e.clone());
        }
        self.inner.get(key)
    }

    fn put(&self, key: &SecretKey, value: &SecretString) -> Result<(), SecretError> {
        if self.armed() {
            let mut writes = self.writes.lock().unwrap();
            let n = writes.iter().filter(|k| *k == key).count();
            writes.push(*key);
            let fails = self.put_fails.lock().unwrap();
            if let Some((_, _, e)) = fails.iter().find(|(k, at, _)| k == key && *at == n) {
                return Err(e.clone());
            }
        }
        self.inner.put(key, value)
    }

    fn delete(&self, key: &SecretKey) -> Result<(), SecretError> {
        if self.armed() {
            self.writes.lock().unwrap().push(*key);
        }
        self.inner.delete(key)
    }

    fn name(&self) -> &'static str {
        "flaky"
    }

    fn choice(&self) -> Option<BackendChoice> {
        Some(BackendChoice::Keyring)
    }
}

/// A shell over a [`Flaky`] keyring holding [`ab_profile`] with a.example's and b.example's
/// passwords (Forever) at hops 0 and 1.
fn flaky_ab(cx: &mut TestAppContext) -> (&'static mut VisualTestContext, Arc<Flaky>, ProfileId) {
    let flaky = Arc::new(Flaky::default());
    let vcx = shell_with_backend(cx, "en", None, flaky.clone());
    let id = add_profile(vcx, ab_profile());
    for (hop, pw) in [(0, "pw-for-a"), (1, "pw-for-b")] {
        let stored = secrets(vcx)
            .put(
                &hop_key(id, hop),
                SecretString::from(pw),
                Retention::Forever,
            )
            .expect("stored");
        assert!(matches!(stored, hacivat_secrets::Stored::AsRequested));
    }
    (vcx, flaky, id)
}

fn a_hop() -> HopSpec {
    hop("a.example", 22, SshAuth::Password)
}

fn b_hop() -> HopSpec {
    hop("b.example", 22, SshAuth::Password)
}

/// Nothing changed: the order stays, each backend slot holds its own hop's password, and the
/// in-memory fallback holds no stray copy.
#[track_caller]
fn nothing_moved(vcx: &VisualTestContext, flaky: &Flaky, id: ProfileId) {
    assert_eq!(hosts_of(vcx, id), ["a.example", "b.example"], "order kept");
    assert!(flaky.holds(id, 0, Some("pw-for-a")), "slot 0 is a's");
    assert!(flaky.holds(id, 1, Some("pw-for-b")), "slot 1 is b's");
    for hop in [0, 1] {
        let slot = SecretSlot::SshPassword { hop };
        assert!(
            stored(vcx, id, slot, Retention::UntilRestart).is_none(),
            "no stray copy"
        );
    }
}

fn unchanged(saved: &Result<ProfileId, hacivat_app::connections::SaveError>) -> bool {
    matches!(
        saved,
        Err(hacivat_app::connections::SaveError::Unchanged(_))
    )
}

/// R1 (a), the review's r1-b: hop 1's put is refused with a Q38-class error, so the store would
/// keep it in memory only (Until restart) while the backend keeps b's password there. That is a
/// failure: hop 0's write is undone, the fallback copy dropped, and the save says `Unchanged`.
#[gpui::test]
fn a_hop_move_whose_put_falls_back_changes_nothing(cx: &mut TestAppContext) {
    let (vcx, flaky, id) = flaky_ab(cx);
    flaky.arm(Vec::new(), vec![(hop_key(id, 1), 0, SecretError::Denied)]);
    let saved = wait_task(vcx, save_task(vcx, id, vec![b_hop(), a_hop()]));
    assert!(unchanged(&saved), "{saved:?}");
    assert!(matches!(
        saved,
        Err(hacivat_app::connections::SaveError::Unchanged(
            SecretError::Denied
        ))
    ));
    nothing_moved(vcx, &flaky, id);
}

/// R1 (b), the review's r1-c: hop 0 replaced by a new host while hop 0's slot read is `Denied`
/// (keychain access refused). The move stops before any write, so a.example's password never
/// stays at hop 0 for x.example.
#[gpui::test]
fn a_hop_move_with_a_denied_read_writes_nothing(cx: &mut TestAppContext) {
    let (vcx, flaky, id) = flaky_ab(cx);
    flaky.arm(vec![(hop_key(id, 0), SecretError::Denied)], Vec::new());
    let x = hop("x.example", 22, SshAuth::Password);
    let saved = wait_task(vcx, save_task(vcx, id, vec![x, b_hop()]));
    assert!(
        matches!(
            saved,
            Err(hacivat_app::connections::SaveError::Unchanged(
                SecretError::Denied
            ))
        ),
        "{saved:?}"
    );
    assert!(flaky.writes.lock().unwrap().is_empty(), "nothing written");
    nothing_moved(vcx, &flaky, id);
}

/// R1 (c): hop 1's put fails hard, and the undo's restore of hop 0 falls back (it would land in
/// memory only, over b's password in the backend). The undo then clears hop 0 instead: never
/// b's password at a.example's hop; the save says `Unchanged`.
#[gpui::test]
fn a_hop_move_whose_undo_falls_back_clears_the_slot(cx: &mut TestAppContext) {
    let (vcx, flaky, id) = flaky_ab(cx);
    flaky.arm(
        Vec::new(),
        vec![
            (hop_key(id, 1), 0, SecretError::Backend("disk full".into())),
            (hop_key(id, 0), 1, SecretError::Denied),
        ],
    );
    let saved = wait_task(vcx, save_task(vcx, id, vec![b_hop(), a_hop()]));
    assert!(unchanged(&saved), "{saved:?}");
    assert_eq!(hosts_of(vcx, id), ["a.example", "b.example"], "order kept");
    assert!(
        flaky.holds(id, 0, None),
        "hop 0 cleared, never b's password"
    );
    assert!(flaky.holds(id, 1, Some("pw-for-b")), "slot 1 is b's");
    let slot = SecretSlot::SshPassword { hop: 0 };
    assert!(
        stored(vcx, id, slot, Retention::UntilRestart).is_none(),
        "no stray copy"
    );
}

fn saved_profile(vcx: &VisualTestContext, id: ProfileId) -> Arc<Profile> {
    vcx.cx
        .read(|cx| cx.global::<ConnectionsModel>().get(id))
        .expect("saved")
}

/// R3, the review's r1-d: a connect's changes arriving while a hop move waits on the vault's
/// unlock wait for it, then build from the saved profile: the hop edit, the CA file and the new
/// SSH retention all survive, and the moved secrets follow both.
#[gpui::test]
fn changes_during_a_hop_move_wait_and_keep_the_hop_edit(cx: &mut TestAppContext) {
    use hacivat_session::pipeline::ProfileChange;

    let (vcx, vault) = shell_with_vault_state(cx, true);
    let id = add_profile(vcx, ab_profile());
    for (hop, pw) in [(0, "pw-for-a"), (1, "pw-for-b")] {
        secrets(vcx)
            .put(
                &hop_key(id, hop),
                SecretString::from(pw),
                Retention::Forever,
            )
            .expect("stored in the vault");
    }
    vault.lock();
    let saving = save_task(vcx, id, vec![b_hop(), a_hop()]);
    settle_until(vcx, WAIT, || layer_open(vcx, "vault-dialog"));
    let ca = PathBuf::from("/certs/ca.pem");
    let applying = vcx.cx.update(|cx| {
        ConnectionsModel::apply_changes(
            cx,
            id,
            vec![
                ProfileChange::SetCaFile(ca.clone()),
                ProfileChange::SshRetention(Retention::UntilRestart),
            ],
        )
    });
    vcx.run_until_parked();
    assert!(
        saved_profile(vcx, id).tls.ca_file.is_none(),
        "the changes wait for the move"
    );
    assert_eq!(saved_profile(vcx, id).ssh_retention, Retention::Forever);

    type_into(vcx, "vault-unlock", MASTER);
    press(vcx, "enter");
    let saved = wait_task(vcx, saving);
    assert!(saved.is_ok(), "{saved:?}");
    let applied = wait_task(vcx, applying);
    assert!(applied.is_ok(), "{applied:?}");
    assert_eq!(
        hosts_of(vcx, id),
        ["b.example", "a.example"],
        "the hop edit stays"
    );
    assert_eq!(
        saved_profile(vcx, id).ssh_retention,
        Retention::UntilRestart
    );
    assert_eq!(
        saved_profile(vcx, id).tls.ca_file.as_deref(),
        Some(ca.as_path())
    );
    let kept = |hop| {
        stored(
            vcx,
            id,
            SecretSlot::SshPassword { hop },
            Retention::UntilRestart,
        )
    };
    assert!(
        kept(0).as_deref() == Some("pw-for-b"),
        "b's password follows b"
    );
    assert!(
        kept(1).as_deref() == Some("pw-for-a"),
        "a's password follows a"
    );
    assert!(vault_pw_is(&vault, id, 0, None), "moved out of the vault");
    assert!(vault_pw_is(&vault, id, 1, None), "moved out of the vault");
}
