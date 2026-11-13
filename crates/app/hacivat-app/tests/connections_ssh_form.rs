//! Task 20: the SSH section of the connection dialog (NOTES M2a surface 6) — the disclosure and
//! its summary, the sign-in method's fields, the jump host list and its keys, the route row,
//! the host-key policy, the `~/.ssh/config` import, the "Connect through SSH" fix, the SSH
//! issues by `IssueCode` (Q9) and the typed secrets by hop (Q23).
//!
//! The shell helpers call `allow_parking()`; every wait is a condition (`settle_until`,
//! `wait_until`), never a fixed sleep (Q4 ruling).

mod support;

use std::sync::Arc;

use gpui::{
    ClipboardItem, Entity, Focusable as _, Keystroke, Modifiers, TestAppContext, VisualTestContext,
};
use hacivat_app::connections::ConnectionsModel;
use hacivat_app::connections::dialog::ssh::{AuthKind, SshImportEnv, SshSection, section_state};
use hacivat_app::connections::dialog::{
    ConnectionDialog, DialogMode, open_connection_dialog, open_dialog,
};
use hacivat_app::connections::fixes::{FixAction, run_m2b_fix};
use hacivat_app::l10n_global::{tr, tr_id};
use hacivat_l10n::{Count, Text, ids};
use hacivat_proto::{ErrClass, ProfileId};
use hacivat_session::LiveState;
use hacivat_session::pipeline::{ConnectError, ErrArgs, IssueCode, Phase};
use hacivat_session::profile::{ProfileDraft, TransportSpec};

use support::*;

// ---- opening ------------------------------------------------------------------------------------

fn open_new(vcx: &'static mut VisualTestContext) -> &'static mut VisualTestContext {
    in_window(vcx, |window, cx| {
        open_connection_dialog(DialogMode::New, window, cx)
    });
    settle_until(vcx, WAIT, || section(vcx).is_some());
    vcx
}

/// A connections shell with the New dialog open.
fn dialog_new(cx: &mut TestAppContext) -> &'static mut VisualTestContext {
    open_new(shell_with_connections(cx, "en"))
}

/// The New dialog with the SSH section open and the tunnel on.
fn dialog_ssh_on(cx: &mut TestAppContext) -> &'static mut VisualTestContext {
    let vcx = dialog_new(cx);
    click_selector(vcx, "disclosure-conn-ssh");
    click_selector(vcx, "checkbox-ssh-use");
    assert!(ssh(vcx, |s, _| s.is_on() && s.is_open()));
    vcx
}

/// A tunnel through `jump-eu.acme.io` and `bastion.acme.io` to `db-prod-eu.acme.io`.
fn dialog_ssh_full(cx: &mut TestAppContext) -> &'static mut VisualTestContext {
    let vcx = dialog_ssh_on(cx);
    type_into_form(vcx, "host", "db-prod-eu.acme.io");
    type_into(vcx, "ssh-host", "bastion.acme.io");
    add_jump(vcx, "jump-eu.acme.io", "deploy");
    vcx
}

/// The New dialog whose import reads `config` (a fixture `~/.ssh/config`).
fn dialog_with_ssh_config(cx: &mut TestAppContext, config: &str) -> &'static mut VisualTestContext {
    let vcx = shell_with_connections(cx, "en");
    let home = tempfile::tempdir().expect("temp home");
    let path = home.path().join(".ssh").join("config");
    std::fs::create_dir_all(path.parent().expect("a folder")).expect("~/.ssh");
    write(&path, config);
    let env = SshImportEnv {
        config: path,
        home: home.path().to_path_buf(),
        local_user: "tester".to_string(),
    };
    vcx.cx.update(|cx| cx.set_global(env));
    // Kept for the app's lifetime (the import reads it on the core runtime).
    std::mem::forget(home);
    let vcx = open_new(vcx);
    click_selector(vcx, "disclosure-conn-ssh");
    vcx
}

/// A connections shell whose profile `orders` failed with `error`.
fn with_failed(
    cx: &mut TestAppContext,
    error: ConnectError,
) -> (&'static mut VisualTestContext, ProfileId) {
    let vcx = shell_with_connections(cx, "en");
    let id = add_profile(vcx, test_profile("orders", None, "127.0.0.1"));
    vcx.cx.update(|cx| {
        cx.global_mut::<ConnectionsModel>()
            .set_live(id, LiveState::Failed(Arc::new(error.clone())));
    });
    vcx.run_until_parked();
    (vcx, id)
}

fn dns_error() -> ConnectError {
    ConnectError {
        class: ErrClass::NetworkDns,
        phase: Phase::NameResolution,
        trail: Vec::new(),
        args: ErrArgs {
            name: "orders".into(),
            host: Some("db.internal".into()),
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
        fixes: Vec::new(),
        suggestions: Vec::new(),
        issues: Vec::new(),
        elapsed: std::time::Duration::ZERO,
        attempts: 0,
        needs_interaction: false,
        certificate: None,
        host_key: None,
        profile_changes: Vec::new(),
    }
}

fn click_fix(vcx: &VisualTestContext, id: ProfileId, action: FixAction) {
    let error = Arc::new(dns_error());
    in_window(vcx, |window, cx| {
        run_m2b_fix(&action, &error, id, window, cx)
    });
    vcx.run_until_parked();
}

// ---- reading ------------------------------------------------------------------------------------

fn dialog(vcx: &VisualTestContext) -> Entity<ConnectionDialog> {
    in_window(vcx, |window, cx| open_dialog(window, cx)).expect("a connection dialog is open")
}

fn section(vcx: &VisualTestContext) -> Option<Entity<SshSection>> {
    in_window(vcx, |window, cx| section_state(window, cx))
}

fn ssh<R>(vcx: &VisualTestContext, f: impl FnOnce(&SshSection, &gpui::App) -> R) -> R {
    let s = section(vcx).expect("the SSH section is drawn");
    vcx.cx.read(|cx| f(s.read(cx), cx))
}

/// The dialog's draft, through `f`.
fn draft<R>(vcx: &VisualTestContext, f: impl FnOnce(&ProfileDraft) -> R) -> R {
    let s = section(vcx).expect("the SSH section is drawn");
    vcx.cx
        .read(|cx| f(s.read(cx).draft(cx).expect("the section's dialog")))
}

fn hop_hosts(vcx: &VisualTestContext) -> Vec<String> {
    draft(vcx, |d| match &d.profile.transport {
        TransportSpec::SshTunnel(spec) => spec.hops.iter().map(|h| h.host.clone()).collect(),
        _ => Vec::new(),
    })
}

/// `text` without the Unicode isolation marks Fluent puts around arguments (the frame's
/// getters strip them too).
fn plain(text: impl AsRef<str>) -> String {
    text.as_ref().replace(['\u{2068}', '\u{2069}'], "")
}

fn t(vcx: &VisualTestContext, id: &str) -> String {
    vcx.cx.read(|cx| plain(tr_id(cx, id)))
}

/// The SSH disclosure's summary as drawn: the line after its title.
fn summary(vcx: &VisualTestContext) -> String {
    let title = t(vcx, "hv-conn-section-ssh");
    let texts = texts_drawn_by(vcx, "hacivat-ui/src/disclosure.rs");
    let at = texts
        .iter()
        .position(|s| *s == title)
        .expect("the SSH disclosure is drawn");
    texts.get(at + 1).cloned().expect("a summary")
}

fn field(vcx: &VisualTestContext, name: &str) -> String {
    let input = ssh(vcx, |s, _| s.input(name)).unwrap_or_else(|| panic!("no field {name}"));
    vcx.cx.read(|cx| input.read(cx).value().to_string())
}

fn visible(vcx: &mut VisualTestContext, name: &str) -> bool {
    vcx.run_until_parked();
    let field: &'static str = Box::leak(format!("ssh-field-{name}").into_boxed_str());
    let other: &'static str = Box::leak(name.to_string().into_boxed_str());
    vcx.debug_bounds(field).is_some() || vcx.debug_bounds(other).is_some()
}

fn focused_id(vcx: &VisualTestContext) -> String {
    let s = section(vcx).expect("the SSH section");
    in_window(vcx, |window, cx| s.read(cx).focused(window, cx)).unwrap_or_default()
}

fn notice_text(vcx: &VisualTestContext) -> String {
    texts_drawn_by(vcx, "hacivat-ui/src/notice.rs").join("\n")
}

fn route(vcx: &VisualTestContext) -> String {
    (0..)
        .map_while(|k| text_of(vcx, &format!("ssh-route-{k}")))
        .collect::<Vec<_>>()
        .join(" → ")
}

fn issue_visible(vcx: &VisualTestContext, place: &str, code: IssueCode, msg: &str) -> bool {
    let id = format!(
        "ssh-issue-{place}-{}",
        hacivat_app::connections::dialog::ssh::issue_slug(&code)
    );
    text_of(vcx, &id).is_some_and(|text| text == t(vcx, msg))
}

// ---- driving ------------------------------------------------------------------------------------

fn click_selector(vcx: &mut VisualTestContext, selector: &str) {
    vcx.run_until_parked();
    let selector: &'static str = Box::leak(selector.to_string().into_boxed_str());
    let bounds = vcx
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("no element `{selector}` drawn"));
    vcx.simulate_click(bounds.center(), Modifiers::none());
    vcx.run_until_parked();
}

fn press(vcx: &mut VisualTestContext, keys: &str) {
    vcx.simulate_keystrokes(keys);
    vcx.run_until_parked();
}

fn primary(key: &str) -> String {
    if cfg!(target_os = "macos") {
        format!("cmd-{key}")
    } else {
        format!("ctrl-{key}")
    }
}

fn focus(vcx: &VisualTestContext, name: &str) {
    let input = ssh(vcx, |s, _| s.input(name)).unwrap_or_else(|| panic!("no field {name}"));
    let handle = vcx.cx.read(|cx| input.focus_handle(cx));
    in_window(vcx, |window, cx| window.focus(&handle, cx));
    vcx.run_until_parked();
}

/// Replaces the text of the SSH field `name` by typing.
fn type_into(vcx: &mut VisualTestContext, name: &str, text: &str) {
    focus(vcx, name);
    press(vcx, &primary("a"));
    vcx.simulate_input(text);
    vcx.run_until_parked();
}

/// Types into the form's own field `name` (`host`, `port`, …).
fn type_into_form(vcx: &mut VisualTestContext, name: &str, text: &str) {
    let d = dialog(vcx);
    let input = vcx
        .cx
        .read(|cx| d.read(cx).input(name))
        .unwrap_or_else(|| panic!("no form field {name}"));
    let handle = vcx.cx.read(|cx| input.focus_handle(cx));
    in_window(vcx, |window, cx| window.focus(&handle, cx));
    vcx.simulate_input(text);
    vcx.run_until_parked();
}

fn pick_segment(vcx: &mut VisualTestContext, kind: AuthKind) {
    let at = AuthKind::ALL
        .iter()
        .position(|k| *k == kind)
        .expect("a method");
    click_selector(vcx, &format!("segmented-ssh-auth-{at}"));
}

/// Adds a jump host row (the button) and types its host and user.
fn add_jump(vcx: &mut VisualTestContext, host: &str, user: &str) {
    click_selector(vcx, "button-ssh-add-jump");
    let at = ssh(vcx, |s, _| s.jump_count()) - 1;
    type_into(vcx, &format!("ssh-jump-host-{at}"), host);
    type_into(vcx, &format!("ssh-jump-user-{at}"), user);
}

fn open_import(vcx: &mut VisualTestContext) {
    click_selector(vcx, "button-ssh-import");
    settle_until(vcx, WAIT, || ssh(vcx, |s, _| s.import_open()));
    vcx.run_until_parked();
}

fn entry(vcx: &VisualTestContext, alias: &str) -> (usize, bool, String) {
    ssh(vcx, |s, cx| {
        s.import_entries(cx)
            .into_iter()
            .enumerate()
            .find(|(_, (a, _, _))| a == alias)
            .map(|(i, (_, usable, hint))| (i, usable, plain(&hint)))
    })
    .unwrap_or_else(|| panic!("no import entry {alias}"))
}

/// Clicks the import menu's row for `alias`.
fn pick(vcx: &mut VisualTestContext, alias: &str) {
    let (at, _, _) = entry(vcx, alias);
    click_selector(vcx, &format!("menu-item-{at}"));
}

// ---- the brief's tests --------------------------------------------------------------------------

#[gpui::test]
fn ssh_disclosure_summary(cx: &mut TestAppContext) {
    let vcx = dialog_new(cx);
    assert_eq!(summary(vcx), t(vcx, "hv-conn-ssh-sum-off"));
    let vcx = {
        click_selector(vcx, "disclosure-conn-ssh");
        click_selector(vcx, "checkbox-ssh-use");
        type_into(vcx, "ssh-host", "bastion.acme.io");
        add_jump(vcx, "jump-eu.acme.io", "deploy");
        click_selector(vcx, "disclosure-conn-ssh");
        vcx
    };
    let want = vcx.cx.read(|cx| {
        plain(tr(
            cx,
            ids::hv::CONN_SSH_SUM_HOPS,
            ids::hv::HvConnSshSumHops {
                host: Text("bastion.acme.io".into()),
                n: Count(1),
            },
        ))
    });
    assert_eq!(summary(vcx), want);
    assert!(
        want.contains("1 jump host") && !want.contains("hosts"),
        "{want}"
    );
}

#[gpui::test]
fn auth_segmented_switches_fields(cx: &mut TestAppContext) {
    let vcx = dialog_ssh_on(cx);
    assert!(visible(vcx, "ssh-agent-note"), "Agent is the default");
    pick_segment(vcx, AuthKind::Key);
    assert!(visible(vcx, "ssh-key-file") && visible(vcx, "ssh-passphrase"));
    assert!(!visible(vcx, "ssh-agent-note"));
    pick_segment(vcx, AuthKind::Kbd);
    assert!(visible(vcx, "ssh-kbd-note") && !visible(vcx, "ssh-passphrase"));
    pick_segment(vcx, AuthKind::Password);
    assert!(visible(vcx, "ssh-password") && !visible(vcx, "ssh-kbd-note"));
}

#[gpui::test]
fn jump_host_list_keyboard(cx: &mut TestAppContext) {
    let vcx = dialog_ssh_on(cx);
    add_jump(vcx, "j1", "u");
    add_jump(vcx, "j2", "u");
    focus(vcx, "ssh-jump-host-1");
    press(vcx, "alt-up");
    assert_eq!(hop_hosts(vcx), ["j2", "j1", ""]);
    assert_eq!(
        focused_id(vcx),
        "ssh-jump-host-0",
        "the focus moves with its row"
    );
    assert_eq!(field(vcx, "ssh-jump-host-0"), "j2");
    press(vcx, "alt-down");
    assert_eq!(hop_hosts(vcx), ["j1", "j2", ""]);
    press(vcx, "alt-down");
    assert_eq!(
        hop_hosts(vcx),
        ["j1", "j2", ""],
        "the last jump row stays last"
    );
    click_selector(vcx, "button-ssh-jump-remove-0");
    assert_eq!(hop_hosts(vcx), ["j2", ""]);
    assert_eq!(ssh(vcx, |s, _| s.jump_count()), 1);
}

#[gpui::test]
fn route_row_draws_real_path(cx: &mut TestAppContext) {
    let vcx = dialog_ssh_full(cx);
    let this_mac = vcx.cx.read(|cx| {
        let os = if cfg!(target_os = "macos") {
            ids::hv::HvConnSshThisMacOs::Mac
        } else {
            ids::hv::HvConnSshThisMacOs::Other
        };
        plain(tr(
            cx,
            ids::hv::CONN_SSH_THIS_MAC,
            ids::hv::HvConnSshThisMac { os },
        ))
    });
    assert_eq!(
        route(vcx),
        format!("{this_mac} → jump-eu.acme.io → bastion.acme.io → db-prod-eu.acme.io:5432")
    );
}

#[gpui::test]
fn host_key_policy_has_no_accept_all(cx: &mut TestAppContext) {
    let vcx = dialog_ssh_on(cx);
    assert_eq!(
        ssh(vcx, |s, _| s.host_key_choices()),
        ["ask", "accept_new", "strict"]
    );
    let select = ssh(vcx, |s, _| s.host_key_select());
    let (count, selected) = vcx
        .cx
        .read(|cx| (select.read(cx).options().len(), select.read(cx).selected()));
    assert_eq!((count, selected), (3, 0), "three choices; Ask by default");
}

#[gpui::test]
fn import_menu_lists_aliases_and_disables_proxycommand(cx: &mut TestAppContext) {
    let vcx = dialog_with_ssh_config(
        cx,
        "Host bastion\n HostName b.acme.io\n User deploy\nHost corp\n ProxyCommand nc %h %p\n",
    );
    open_import(vcx);
    let (_, usable, hint) = entry(vcx, "bastion");
    assert!(usable);
    assert_eq!(hint, "deploy@b.acme.io");
    let (_, usable, hint) = entry(vcx, "corp");
    assert!(!usable);
    assert_eq!(hint, t(vcx, "hv-conn-ssh-proxycmd"));
}

const BASTION_WITH_JUMP: &str = "Host jump-eu\n HostName jump-eu.acme.io\n User deploy\n\
Host bastion\n HostName b.acme.io\n User deploy\n ProxyJump jump-eu\n StrictHostKeyChecking yes\n";

#[gpui::test]
fn import_fills_fields_and_shows_notice(cx: &mut TestAppContext) {
    let vcx = dialog_with_ssh_config(cx, BASTION_WITH_JUMP);
    open_import(vcx);
    assert_eq!(
        entry(vcx, "bastion").2,
        "deploy@b.acme.io · ProxyJump jump-eu"
    );
    pick(vcx, "bastion");
    assert!(!ssh(vcx, |s, _| s.import_open()), "the menu closes");
    assert!(ssh(vcx, |s, _| s.is_on()), "an import turns the tunnel on");
    assert_eq!(field(vcx, "ssh-host"), "b.acme.io");
    assert_eq!(field(vcx, "ssh-user"), "deploy");
    assert_eq!(field(vcx, "ssh-jump-host-0"), "jump-eu.acme.io");
    assert_eq!(hop_hosts(vcx), ["jump-eu.acme.io", "b.acme.io"]);
    assert!(ssh(vcx, |s, _| s.is_tinted("ssh-host")
        && s.is_tinted("ssh-jump-host-0")));
    assert!(notice_text(vcx).contains("bastion"), "{}", notice_text(vcx));
    // The strictest policy of the chain (Task 4): `yes` on the bastion.
    let select = ssh(vcx, |s, _| s.host_key_select());
    assert_eq!(vcx.cx.read(|cx| select.read(cx).selected()), 2);
}

#[gpui::test]
fn connect_through_ssh_fix_opens_dialog_with_ssh_open(cx: &mut TestAppContext) {
    let (vcx, id) = with_failed(cx, dns_error());
    click_fix(vcx, id, FixAction::ConnectThroughSsh);
    settle_until(vcx, WAIT, || section(vcx).is_some());
    assert!(ssh(vcx, |s, _| s.is_open() && s.is_on()));
    assert!(draft(vcx, |d| matches!(
        d.profile.transport,
        TransportSpec::SshTunnel(_)
    )));
    assert_eq!(focused_id(vcx), "ssh-host");
    assert!(vcx.debug_bounds("checkbox-ssh-use").is_some());
}

#[gpui::test]
fn ssh_issues_render_inline(cx: &mut TestAppContext) {
    let vcx = dialog_ssh_on(cx);
    type_into_form(vcx, "host", "10.0.0.5");
    press(vcx, &primary("s"));
    settle_until(vcx, WAIT, || {
        issue_visible(
            vcx,
            "ssh-host",
            IssueCode::SshHostMissing,
            "hv-conn-issue-ssh-host-missing",
        )
    });
    assert!(issue_visible(
        vcx,
        "ssh-user",
        IssueCode::SshUserMissing,
        "hv-conn-issue-ssh-user-missing"
    ));
    // Never twice: the form's top notice leaves the SSH issues to the section.
    let missing = t(vcx, "hv-conn-issue-ssh-host-missing");
    assert!(!notice_text(vcx).contains(&missing), "{}", notice_text(vcx));
    // Typing the host clears its issue.
    type_into(vcx, "ssh-host", "bastion.acme.io");
    assert!(!issue_visible(
        vcx,
        "ssh-host",
        IssueCode::SshHostMissing,
        "hv-conn-issue-ssh-host-missing"
    ));
}

#[gpui::test]
fn typed_ssh_password_goes_into_the_draft_by_hop(cx: &mut TestAppContext) {
    let vcx = dialog_ssh_on(cx);
    pick_segment(vcx, AuthKind::Password);
    type_into(vcx, "ssh-password", "pw0");
    assert!(draft(vcx, |d| d.ssh_secrets[0].password.is_some()));
    assert!(draft(vcx, |d| d.ssh_secrets[0].passphrase.is_none()));
    assert!(!draft(vcx, |d| format!("{d:?}")).contains("pw0"));
    // With a jump host in front, the SSH host is hop 1.
    add_jump(vcx, "jump-eu.acme.io", "deploy");
    assert!(draft(vcx, |d| d.ssh_secrets.len() == 2
        && d.ssh_secrets[0].password.is_none()
        && d.ssh_secrets[1].password.is_some()));
    // Another method drops the typed password from the draft.
    pick_segment(vcx, AuthKind::Agent);
    assert!(draft(vcx, |d| d
        .ssh_secrets
        .iter()
        .all(|s| s.password.is_none())));
}

#[gpui::test]
fn alt_arrows_do_not_register_a_global_binding(cx: &mut TestAppContext) {
    let vcx = dialog_ssh_on(cx);
    add_jump(vcx, "j1", "u");
    for chord in ["alt-up", "alt-down"] {
        let typed = Keystroke::parse(chord).expect("a chord");
        let bound = vcx.cx.read(|cx| {
            cx.key_bindings()
                .borrow()
                .bindings()
                .any(|b| b.match_keystrokes(std::slice::from_ref(&typed)).is_some())
        });
        assert!(!bound, "{chord} is bound globally");
    }
}

// ---- more of surface 6 --------------------------------------------------------------------------

#[gpui::test]
fn the_tunnel_off_keeps_its_fields_and_saves_direct(cx: &mut TestAppContext) {
    let vcx = dialog_ssh_on(cx);
    type_into(vcx, "ssh-host", "bastion.acme.io");
    click_selector(vcx, "checkbox-ssh-use");
    assert!(draft(vcx, |d| matches!(
        d.profile.transport,
        TransportSpec::Direct
    )));
    assert!(
        !visible(vcx, "ssh-host"),
        "only the checkbox and Import show while off"
    );
    click_selector(vcx, "checkbox-ssh-use");
    assert_eq!(hop_hosts(vcx), ["bastion.acme.io"]);
    assert_eq!(field(vcx, "ssh-port"), "22");
}

#[gpui::test]
fn a_jump_row_takes_a_port_and_port_zero_is_an_issue(cx: &mut TestAppContext) {
    let vcx = dialog_ssh_on(cx);
    type_into_form(vcx, "host", "10.0.0.5");
    type_into(vcx, "ssh-host", "bastion.acme.io");
    type_into(vcx, "ssh-user", "deploy");
    add_jump(vcx, "jump:abc", "deploy");
    let port = draft(vcx, |d| match &d.profile.transport {
        TransportSpec::SshTunnel(spec) => spec.hops[0].port,
        _ => u16::MAX,
    });
    assert_eq!(
        port, 0,
        "text that is no port is port 0, which Validate reports"
    );
    press(vcx, &primary("s"));
    settle_until(vcx, WAIT, || {
        issue_visible(
            vcx,
            "ssh-jump-0",
            IssueCode::PortInvalid,
            "hv-conn-issue-port-invalid",
        )
    });
    assert!(!issue_visible(
        vcx,
        "ssh-port",
        IssueCode::PortInvalid,
        "hv-conn-issue-port-invalid"
    ));
}

#[gpui::test]
fn an_import_never_shows_a_refused_secret(cx: &mut TestAppContext) {
    // Hostile values (M2a T7 invariant): a ProxyJump with a password, a User that looks like
    // a DSN with a password.
    let vcx = dialog_with_ssh_config(
        cx,
        "Host jump\n HostName jump.acme.io\n\
         Host leaky\n HostName l.acme.io\n User x password=hv_test_pw\n\
         Host viaj\n HostName v.acme.io\n User deploy\n ProxyJump deploy:hv_test_pw@jump\n",
    );
    open_import(vcx);
    let (_, usable, hint) = entry(vcx, "leaky");
    assert!(!usable);
    assert_eq!(hint, t(vcx, "hv-conn-ssh-unusable-secret"));
    let all: Vec<String> = ssh(vcx, |s, cx| {
        s.import_entries(cx)
            .into_iter()
            .map(|(a, _, h)| format!("{a} {h}"))
            .collect()
    });
    assert!(!all.iter().any(|e| e.contains("hv_test_pw")), "{all:?}");
    pick(vcx, "viaj");
    let notice = notice_text(vcx);
    assert!(
        notice.contains("viaj") && !notice.contains("hv_test_pw"),
        "{notice}"
    );
    assert!(!draft(vcx, |d| format!("{:?}", d.profile)).contains("hv_test_pw"));
}

#[gpui::test]
fn an_unreadable_config_says_so(cx: &mut TestAppContext) {
    let vcx = shell_with_connections(cx, "en");
    vcx.cx.update(|cx| {
        cx.set_global(SshImportEnv {
            config: "/nonexistent/hacivat-ssh-config".into(),
            home: "/nonexistent".into(),
            local_user: "tester".into(),
        })
    });
    let vcx = open_new(vcx);
    click_selector(vcx, "disclosure-conn-ssh");
    open_import(vcx);
    assert!(ssh(vcx, |s, cx| s.import_entries(cx).is_empty()));
    let none = t(vcx, "hv-conn-ssh-import-none");
    assert!(
        registry_frame(vcx).iter().any(|r| plain(&r.text) == none),
        "{none} is drawn"
    );
    press(vcx, "escape");
    assert!(!ssh(vcx, |s, _| s.import_open()));
    assert_eq!(
        focused_id(vcx),
        "ssh-import",
        "Escape returns to the button"
    );
    assert!(
        in_window(vcx, |window, cx| open_dialog(window, cx).is_some()),
        "the dialog stays"
    );
}

#[gpui::test]
fn tab_walks_the_section_in_the_mocks_order(cx: &mut TestAppContext) {
    let vcx = dialog_ssh_on(cx);
    add_jump(vcx, "j1", "u");
    let start = ssh(vcx, |s, _| s.input("ssh-host")).expect("the host field");
    let handle = vcx.cx.read(|cx| start.focus_handle(cx));
    in_window(vcx, |window, cx| window.focus(&handle, cx));
    vcx.run_until_parked();
    let mut seen = vec![focused_id(vcx)];
    for _ in 0..8 {
        press(vcx, "tab");
        seen.push(focused_id(vcx));
    }
    assert_eq!(
        seen,
        [
            "ssh-host",
            "ssh-port",
            "ssh-user",
            "ssh-auth",
            "ssh-jump-host-0",
            "ssh-jump-user-0",
            "ssh-jump-remove-0",
            "ssh-add-jump",
            "ssh-hostkeys",
        ]
    );
}

#[gpui::test]
fn escape_in_an_ssh_field_closes_the_dialog(cx: &mut TestAppContext) {
    let vcx = dialog_ssh_on(cx);
    focus(vcx, "ssh-host");
    press(vcx, "escape");
    assert!(in_window(vcx, |window, cx| open_dialog(window, cx).is_none()));
}

#[gpui::test]
fn a_pasted_url_in_an_ssh_field_fills_the_form(cx: &mut TestAppContext) {
    let vcx = dialog_ssh_on(cx);
    focus(vcx, "ssh-host");
    vcx.cx.write_to_clipboard(ClipboardItem::new_string(
        "postgres://deploy@db.acme.io:5433/orders".into(),
    ));
    press(vcx, &primary("v"));
    let d = dialog(vcx);
    let host = vcx.cx.read(|cx| d.read(cx).field_value("host", cx));
    assert_eq!(host.as_deref(), Some("db.acme.io"));
    assert_eq!(
        field(vcx, "ssh-host"),
        "",
        "the SSH host is not the database"
    );
}
