//! The SSH transport step (Task 14, B11, Q44): at order 40, for a `TransportSpec::SshTunnel`
//! profile, it acquires a `TunnelLease` from `ConnectDeps::tunnels` (outside the connect clock),
//! skips Resolve (the bastion resolves the database host), classifies the address with
//! `ssh_address_class`, and hands the driver the lease's factory.
//!
//! 1. **Key files** go through `hacivat_net::paths::local_path` (the `~` expansion and share
//!    refusal Validate checked), and a private key file other users can read or write is
//!    refused, as OpenSSH does (M6): the hop's own key fails the connect (`config.invalid`), an
//!    imported `IdentityFile` is skipped.
//! 2. **Hop secrets**, per hop: the session overlay's (reconnects, side connections; Q17), then
//!    the dialog's draft (Test; Q23), then the store (`SshPassword { hop }`,
//!    `SshPassphrase { hop }`, with `ssh_retention`) on the blocking pool, outside the deadline.
//! 3. **Acquire** with `share = purpose != Test` and, when interactive, a [`PipelinePrompter`].
//!    `acquire` is not wrapped in the connect clock: the hops' network timeouts come from
//!    `TunnelRequest.connect_timeout`, and prompt time never counts (P23, Q44).
//! 4. The factory is `lease.factory(db_host, db_port)` and `Endpoint::Network.direct` is `None`,
//!    so a driver without stream injection (MySQL) goes through its forwarder. The trail shows
//!    `Ssh`, then `Connect` for the first `direct-tcpip` channel.
//! 5. The lease becomes a `CleanupGuard` (released after the connection closes).

// ## m2b-t14-pipeline

use std::path::Path;
use std::sync::Arc;

use futures::future::BoxFuture;
use hacivat_driver::classify_ssh;
use hacivat_driver::url::FieldId;
use hacivat_net::ssh::{
    HopSecrets, HopSpec, SshAuth, SshError, SshPrompter, SshSpec, TunnelRequest,
};
use hacivat_net::{CleanupGuard, ssh_address_class};
use hacivat_proto::ErrClass;
use hacivat_secrets::{Retention, SecretKey, SecretSlot};

use super::prompts::{PipelinePrompter, read_store};
use super::steps::{ConnectStep, StepCx, StepState, error, hand_over, server_host};
use super::{
    ConnectError, HostKeyMismatch, Interaction, IssueCode, Phase, Purpose, TrailStep,
    ValidationIssue,
};
use crate::manager::PathKind;
use crate::profile::{Profile, ProfileEndpoint, TransportSpec};

/// The SSH transport (order 40).
pub(crate) struct SshStep;

impl ConnectStep for SshStep {
    fn phase(&self) -> Phase {
        Phase::Transport
    }

    fn order(&self) -> i32 {
        40
    }

    fn applies(&self, p: &Profile) -> bool {
        matches!(p.transport, TransportSpec::SshTunnel(_))
            && !matches!(p.endpoint, ProfileEndpoint::File { .. })
    }

    fn apply<'a>(
        &'a self,
        cx: &'a mut StepCx,
    ) -> BoxFuture<'a, Result<Option<CleanupGuard>, ConnectError>> {
        Box::pin(open(cx))
    }
}

async fn open(cx: &mut StepCx) -> Result<Option<CleanupGuard>, ConnectError> {
    let p = cx.profile.clone();
    let TransportSpec::SshTunnel(spec) = &p.transport else {
        return Ok(None);
    };
    let st = &mut cx.state;
    let Some(db_host) = server_host(&p).map(str::to_string) else {
        // A socket names a file on this machine, which a tunnel cannot reach.
        let mut e = error(ErrClass::ConfigInvalid, Phase::Validate);
        e.issues = vec![ValidationIssue {
            field: Some(FieldId::Socket),
            code: IssueCode::TransportUnavailable,
        }];
        return Err(e);
    };
    let db_port = st.port.unwrap_or_default();
    st.trail.start(TrailStep::Ssh);

    let spec = local_keys(spec.clone()).await?;
    let secrets = hop_secrets(&p, st, &spec).await;
    // The hops that cannot sign in without the user: keyboard-interactive (one-time codes),
    // or a password hop with no password from the overlay, the draft or the store.
    let needs_user: Vec<bool> = spec
        .hops
        .iter()
        .zip(&secrets)
        .map(|(hop, given)| match hop.auth {
            SshAuth::KeyboardInteractive => true,
            SshAuth::Password => given.password.is_none(),
            SshAuth::Agent | SshAuth::KeyFile { .. } => false,
        })
        .collect();
    let prompter = match &st.ix {
        Interaction::Interactive(ui) => Some(PipelinePrompter::new(ui.clone(), &p)),
        Interaction::Headless => None,
    };
    let req = TunnelRequest {
        spec: spec.clone(),
        secrets,
        share: st.purpose != Purpose::Test,
        connect_timeout: st.timeout,
    };
    // Not in the connect clock (Q44): the hops' own timeouts bound the network, and a prompt
    // (host key, password, one-time code) never counts toward the deadline (P23).
    let acquired = st
        .tunnels
        .acquire(
            req,
            prompter.clone().map(|p| p as Arc<dyn SshPrompter>),
            cx.cancel.clone(),
        )
        .await;
    let mut lease = match acquired {
        Ok(lease) => lease,
        Err(e) => {
            let headless = matches!(st.ix, Interaction::Headless);
            return Err(ssh_failure(
                &e,
                &spec,
                headless.then_some(needs_user.as_slice()),
                cx.cancel.is_cancelled(),
            ));
        }
    };
    for (hop, key) in lease.trusted_keys() {
        tracing::info!(
            hop,
            host = %spec.hops.get(usize::from(*hop)).map_or("", |h| h.host.as_str()),
            fingerprint = %key.fingerprint(),
            "SSH host key trusted and stored"
        );
    }
    let skipped = st.known_hosts.skipped_lines();
    if skipped > 0 {
        tracing::warn!(
            skipped,
            "known_hosts lines skipped (too long, or past the size limit)"
        );
    }
    let used = lease.take_secrets();
    st.ssh.hops = if used.is_empty() {
        // A lease that joined an open chain authenticated nothing: the overlay's secrets (if
        // any) are still the ones that work.
        std::mem::take(&mut st.overlay.hops)
    } else {
        used
    };
    st.ssh.typed = prompter.map(|p| p.typed()).unwrap_or_default();
    st.ssh.transport = Some(lease.generation());
    st.ssh.via = Some(lease.bastion().to_string());
    st.ssh.path = PathKind::Ssh;
    st.address = ssh_address_class(&db_host);
    st.trail.done(TrailStep::Ssh);
    tracing::debug!(hops = lease.hops(), "SSH tunnel up");

    let factory = lease.factory(&db_host, db_port);
    let guard = CleanupGuard::new(move || {
        Box::pin(async move {
            drop(lease);
        })
    });
    // The bastion resolves the database host: no local lookup (Q44), no direct target.
    hand_over(&p, st, factory, None, db_host, true).await?;
    Ok(Some(guard))
}

/// The error of a failed acquire, naming the hop (`ErrArgs.hop`, `hop_host`). A headless
/// connect (`headless` holds, per hop, whether it needed the user to sign in) that stopped
/// where it would have asked (an unknown host key under `Ask`, a missing password, a one-time
/// code) says so with `needs_interaction` (Q17: a reconnect then becomes Lost with Retry).
fn ssh_failure(
    e: &SshError,
    spec: &SshSpec,
    headless: Option<&[bool]>,
    cancelled: bool,
) -> ConnectError {
    let mut out = error(classify_ssh(e).class, Phase::Transport);
    let hop = match e {
        SshError::HostKeyUnknown { hop, .. }
        | SshError::HostKeyChanged { hop, .. }
        | SshError::HostKeyRevoked { hop, .. }
        | SshError::Auth { hop, .. }
        | SshError::AgentUnavailable { hop }
        | SshError::ChannelOpen { hop, .. }
        | SshError::Unreachable { hop, .. }
        | SshError::Timeout { hop }
        | SshError::Protocol { hop } => Some(*hop),
        SshError::Cancelled => None,
    };
    out.args.hop = hop;
    out.args.hop_host = hop
        .and_then(|h| spec.hops.get(usize::from(h)))
        .map(|h| h.host.trim().to_string());
    match e {
        SshError::HostKeyChanged {
            hop,
            host,
            port,
            old,
            new,
            source,
        } => {
            out.host_key = Some(HostKeyMismatch {
                hop: *hop,
                host: host.clone(),
                port: *port,
                old: old.clone(),
                new: new.clone(),
                source: source.clone(),
            });
        }
        SshError::HostKeyUnknown { .. } if headless.is_some() => out.needs_interaction = true,
        SshError::Auth { hop, .. }
            if headless.is_some_and(|needs| needs.get(usize::from(*hop)) == Some(&true)) =>
        {
            out.needs_interaction = true;
        }
        SshError::Cancelled if headless.is_some() && !cancelled => {
            // The tunnel needed a secret and had no one to ask: not the user's cancel.
            out.class = ErrClass::SshAuth;
            out.needs_interaction = true;
        }
        _ => {}
    }
    tracing::debug!(class = out.class.id(), hop = ?out.args.hop, "SSH tunnel failed");
    out
}

/// `spec` with every key file as the tunnel must open it (`local_path`: `~` expanded, a share
/// or a device refused) and checked like OpenSSH checks a private key's permissions (M6). On
/// the blocking pool.
async fn local_keys(spec: SshSpec) -> Result<SshSpec, ConnectError> {
    tokio::task::spawn_blocking(move || checked_keys(spec))
        .await
        .map_err(|_| error(ErrClass::Internal, Phase::Transport))?
}

fn checked_keys(mut spec: SshSpec) -> Result<SshSpec, ConnectError> {
    let refused = || {
        let mut e = error(ErrClass::ConfigInvalid, Phase::Validate);
        e.issues = vec![ValidationIssue {
            field: None,
            code: IssueCode::PathRefused,
        }];
        e
    };
    for hop in &mut spec.hops {
        if let SshAuth::KeyFile { path } = &mut hop.auth {
            let full = hacivat_net::paths::local_path(path).ok_or_else(refused)?;
            if !private_key_ok(&full) {
                tracing::warn!(
                    "an SSH key file other users can access is refused, as OpenSSH does"
                );
                return Err(refused());
            }
            *path = full;
        }
        // An imported `IdentityFile` that cannot be used is skipped, as OpenSSH skips it.
        hop.identity_files = std::mem::take(&mut hop.identity_files)
            .into_iter()
            .filter_map(|file| hacivat_net::paths::local_path(&file))
            .filter(|file| {
                let ok = private_key_ok(file);
                if !ok {
                    tracing::warn!(
                        "an imported SSH identity file other users can access is skipped"
                    );
                }
                ok
            })
            .collect();
    }
    Ok(spec)
}

/// OpenSSH's rule (`sshkey_perm_ok`): a private key file must not be accessible by the group or
/// others (mode `& 0o077`). OpenSSH checks only files the user owns; without `getuid` (no
/// `unsafe`, no libc) this checks every file, which only refuses more. A file that does not
/// exist passes here; the tunnel reports what it cannot read. Not checked on Windows (no mode
/// bits).
fn private_key_ok(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        hacivat_net::paths::fs_probe::note(path);
        let Ok(meta) = std::fs::metadata(path) else {
            return true;
        };
        meta.permissions().mode() & 0o077 == 0
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        true
    }
}

/// The secrets the request carries for each hop: the overlay's, then the draft's, then the
/// store's (only the slots the hop can use, and none with `ssh_retention` Never).
async fn hop_secrets(p: &Profile, st: &mut StepState, spec: &SshSpec) -> Vec<HopSecrets> {
    let last = spec.hops.len().saturating_sub(1);
    let bastion_key = spec
        .hops
        .last()
        .is_some_and(|h| matches!(h.auth, SshAuth::KeyFile { .. }));
    let mut out: Vec<HopSecrets> = Vec::with_capacity(spec.hops.len());
    let mut wanted: Vec<(usize, SecretSlot)> = Vec::new();
    for (i, hop) in spec.hops.iter().enumerate() {
        let given = |list: &[HopSecrets], f: fn(&HopSecrets) -> Option<&secrecy::SecretString>| {
            list.get(i).and_then(f).cloned()
        };
        let password = given(&st.overlay.hops, |h| h.password.as_ref())
            .or_else(|| given(&st.draft_ssh, |h| h.password.as_ref()));
        let passphrase = given(&st.overlay.hops, |h| h.passphrase.as_ref())
            .or_else(|| given(&st.draft_ssh, |h| h.passphrase.as_ref()));
        let Ok(n) = u8::try_from(i) else {
            out.push(HopSecrets {
                password,
                passphrase,
            });
            continue;
        };
        if p.ssh_retention != Retention::Never {
            if password.is_none() && uses_password(hop) {
                wanted.push((i, SecretSlot::SshPassword { hop: n }));
            }
            if passphrase.is_none() && uses_key(hop, i < last && bastion_key) {
                wanted.push((i, SecretSlot::SshPassphrase { hop: n }));
            }
        }
        out.push(HopSecrets {
            password,
            passphrase,
        });
    }
    let keys = wanted
        .iter()
        .map(|(_, slot)| {
            (
                SecretKey {
                    profile: p.id,
                    slot: *slot,
                },
                p.ssh_retention,
            )
        })
        .collect();
    let stored = read_store(st, keys).await;
    for ((i, slot), secret) in wanted.into_iter().zip(stored) {
        let Some(hop) = out.get_mut(i) else { continue };
        match slot {
            SecretSlot::SshPassword { .. } => hop.password = secret,
            _ => hop.passphrase = secret,
        }
    }
    out
}

/// Whether a hop may sign in with a password.
fn uses_password(hop: &HopSpec) -> bool {
    matches!(hop.auth, SshAuth::Password)
}

/// Whether a hop may need a key passphrase: its own key file, an imported identity, or (a jump
/// host) the bastion's key file (B11).
fn uses_key(hop: &HopSpec, bastion_key: bool) -> bool {
    matches!(hop.auth, SshAuth::KeyFile { .. }) || !hop.identity_files.is_empty() || bastion_key
}

/// What the SSH step leaves for the rest of the connect.
#[derive(Debug)]
pub(crate) struct SshState {
    /// The secrets that authenticated, by hop (for the session overlay, Q17).
    pub(crate) hops: Vec<HopSecrets>,
    /// The secrets the user typed, by hop: stored after the whole connect succeeded (R8).
    pub(crate) typed: Vec<(u8, super::prompts::SshSlot)>,
    pub(crate) transport: Option<hacivat_net::ssh::TransportGeneration>,
    pub(crate) via: Option<String>,
    pub(crate) path: PathKind,
}

impl SshState {
    /// A direct connect's: no tunnel.
    pub(crate) fn direct() -> Self {
        SshState {
            hops: Vec::new(),
            typed: Vec::new(),
            transport: None,
            via: None,
            path: PathKind::Direct,
        }
    }
}
