//! M2b Task 1: the app's prompt routing, the connection-event relay, the SSH quick-fix seam
//! and SSH secrets on save. The shell helpers call `allow_parking()`; every wait is a
//! condition (`settle_until`, Q4 ruling), never a fixed sleep.

mod support;

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

use gpui::{TestAppContext, VisualTestContext};
use hacivat_app::connections::ConnectionsModel;
use hacivat_app::connections::fixes::{FixAction, run_m2b_fix};
use hacivat_app::connections::interaction::interaction;
use hacivat_app::connections::{dialog, events};
use hacivat_net::ssh::{HopSecrets, HopSpec, HostKey, KbdPrompt, KbdRequest, SshAuth, SshSpec};
use hacivat_net::{CertReview, ChainEntry, ChainRole};
use hacivat_proto::{ErrClass, ProfileId};
use hacivat_secrets::{ExposeSecret, SecretKey, SecretSlot, SecretString};
use hacivat_session::pipeline::{
    Answer, ConnectError, ErrArgs, Interaction, Phase, ProfileChange, Question, SecretPrompt,
};
use hacivat_session::profile::{ProfileDraft, TransportSpec};
use hacivat_session::session::LostState;
use hacivat_session::{ConnectionEvent, GapReason, LiveState};

use support::*;

/// One of each question M2b adds.
fn all_m2b_questions() -> Vec<Question> {
    let profile = ProfileId::new();
    let name = "orders".to_string();
    let key = HostKey {
        algorithm: "ssh-ed25519".into(),
        sha256: [7; 32],
        openssh: "ssh-ed25519 AAAA".into(),
    };
    vec![
        Question::Certificate {
            profile,
            name: name.clone(),
            review: CertReview {
                endpoint: "db:5432".into(),
                subject: "CN=db".into(),
                issuer: "CN=db".into(),
                not_before_unix: 0,
                not_after_unix: 1,
                sha256: [1; 32],
                chain: vec![ChainEntry {
                    subject: "CN=db".into(),
                    role: ChainRole::Leaf,
                }],
                pinned: None,
            },
        },
        Question::PlaintextPublic {
            profile,
            name: name.clone(),
            host: "203.0.113.9".into(),
            ip: None,
        },
        Question::HostKey {
            profile,
            name: name.clone(),
            hop: 0,
            host: "bastion".into(),
            port: 22,
            key,
        },
        Question::Secret {
            profile,
            name: name.clone(),
            prompt: SecretPrompt::SshPassword {
                hop: 0,
                host: "bastion".into(),
                user: "deploy".into(),
            },
            attempt: 1,
        },
        Question::Keyboard {
            profile,
            name,
            hop: 0,
            request: KbdRequest {
                host: "bastion".into(),
                name: String::new(),
                instructions: String::new(),
                prompts: vec![KbdPrompt {
                    text: "Code: ".into(),
                    echo: false,
                }],
            },
            attempt: 1,
        },
        Question::UnlockVault { attempt: 1 },
        Question::CreateVault,
    ]
}

/// Asks `q` through the app's UI port, as the pipeline does from the core runtime; the answer
/// lands in the returned slot.
fn ask_through_port(vcx: &VisualTestContext, q: Question) -> Rc<RefCell<Option<Answer>>> {
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

#[gpui::test]
fn every_new_question_routes_and_stub_answers_cancelled(cx: &mut TestAppContext) {
    let vcx = shell_with_connections(cx, "en");
    for q in all_m2b_questions() {
        let what = format!("{q:?}");
        let a = ask_through_port(vcx, q);
        settle_until(vcx, WAIT, || a.borrow().is_some());
        assert!(matches!(*a.borrow(), Some(Answer::Cancelled)), "{what}");
    }
    assert!(layer_ids(vcx).is_empty(), "no stub leaves a layer behind");
}

/// One of each event M2b's `ConnectionManager` sends.
fn emit_all_connection_events(vcx: &VisualTestContext, profile: ProfileId) {
    let manager = vcx
        .cx
        .read(|cx| cx.global::<ConnectionsModel>().manager().clone());
    for event in [
        ConnectionEvent::SessionLost {
            profile,
            lost: LostState::default(),
        },
        ConnectionEvent::IdleDisconnected { profile },
        ConnectionEvent::IdleTransaction {
            profile,
            minutes: 5,
        },
        ConnectionEvent::EnforcementGap {
            profile,
            reason: GapReason::CanaryAssigned,
        },
        ConnectionEvent::ProfileChanges {
            profile,
            changes: vec![ProfileChange::AllowPublicPlaintext],
        },
    ] {
        manager.publish_event(event);
    }
}

fn handler_hits(vcx: &VisualTestContext) -> usize {
    vcx.cx.read(events::handled_events)
}

#[gpui::test]
fn connection_events_reach_their_handlers(cx: &mut TestAppContext) {
    let vcx = shell_with_connections(cx, "en");
    let id = add_profile(vcx, test_profile("orders", None, "127.0.0.1"));
    // The relay subscribes on the core runtime: wait until it listens before publishing.
    settle_until(vcx, WAIT, || {
        vcx.cx.read(|cx| {
            cx.global::<ConnectionsModel>()
                .manager()
                .event_subscribers()
                > 0
        })
    });
    emit_all_connection_events(vcx, id);
    settle_until(vcx, WAIT, || handler_hits(vcx) == 5);
    assert_eq!(handler_hits(vcx), 5);
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

/// A connections shell whose profile `orders` failed with `error`.
fn with_failed(
    cx: &mut TestAppContext,
    error: ConnectError,
) -> (&'static mut VisualTestContext, ProfileId, Arc<ConnectError>) {
    let vcx = shell_with_connections(cx, "en");
    let id = add_profile(vcx, test_profile("orders", None, "127.0.0.1"));
    let error = Arc::new(error);
    let failed = error.clone();
    vcx.cx.update(|cx| {
        cx.global_mut::<ConnectionsModel>()
            .set_live(id, LiveState::Failed(failed));
    });
    (vcx, id, error)
}

fn click_fix(
    vcx: &VisualTestContext,
    action: FixAction,
    profile: ProfileId,
    error: &Arc<ConnectError>,
) {
    in_window(vcx, |window, cx| {
        run_m2b_fix(&action, error, profile, window, cx);
    });
}

fn layer_open(vcx: &VisualTestContext, id: &str) -> bool {
    layer_ids(vcx).contains(&id)
}

/// The Q9 seam: "Connect through SSH" opens the profile's edit dialog.
#[gpui::test]
fn connect_through_ssh_fix_opens_the_edit_dialog(cx: &mut TestAppContext) {
    let (vcx, id, error) = with_failed(cx, dns_error());
    click_fix(vcx, FixAction::ConnectThroughSsh, id, &error);
    settle_until(vcx, WAIT, || layer_open(vcx, dialog::DIALOG_LAYER_ID));
    assert_eq!(layer_ids(vcx), [dialog::DIALOG_LAYER_ID]);
}

fn ssh_draft_with_secrets(password: &str, passphrase: &str) -> ProfileDraft {
    let mut profile = test_profile("via-bastion", None, "10.0.0.5");
    profile.transport = TransportSpec::SshTunnel(SshSpec {
        hops: vec![HopSpec {
            host: "bastion".into(),
            port: 22,
            user: "deploy".into(),
            auth: SshAuth::KeyFile {
                path: PathBuf::from("/keys/id_ed25519"),
            },
            identities_only: true,
            identity_files: Vec::new(),
        }],
        ..Default::default()
    });
    ProfileDraft {
        is_new: true,
        ssh_secrets: vec![HopSecrets {
            password: Some(SecretString::from(password)),
            passphrase: Some(SecretString::from(passphrase)),
        }],
        ..ProfileDraft::edit(profile)
    }
}

fn stored(vcx: &VisualTestContext, profile: ProfileId, slot: SecretSlot) -> Option<String> {
    secret_backend(vcx)
        .get(&SecretKey { profile, slot })
        .expect("the memory backend answers")
        .map(|s| s.expose_secret().to_string())
}

/// Q23: `ConnectionsModel::save` stores the typed SSH secrets per hop, with `ssh_retention`.
#[gpui::test]
fn save_stores_typed_ssh_secrets_by_hop(cx: &mut TestAppContext) {
    let vcx = shell_with_connections(cx, "en");
    let id = save_draft(vcx, ssh_draft_with_secrets("pw0", "pp0"));
    settle_until(vcx, WAIT, || {
        stored(vcx, id, SecretSlot::SshPassword { hop: 0 }).is_some()
    });
    assert_eq!(
        stored(vcx, id, SecretSlot::SshPassword { hop: 0 }).as_deref(),
        Some("pw0")
    );
    assert_eq!(
        stored(vcx, id, SecretSlot::SshPassphrase { hop: 0 }).as_deref(),
        Some("pp0")
    );
    assert_eq!(stored(vcx, id, SecretSlot::SshPassword { hop: 1 }), None);
}
