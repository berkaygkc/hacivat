//! SSH transport (HV-SEC-01, B11): the profile's SSH settings, host keys and the
//! `known_hosts` stores, `~/.ssh/config` import, and the tunnel manager that opens jump-host
//! chains and hands out stream factories through them.
//!
//! Types are complete since Task 1; Task 4 filled `config` and `known_hosts`, Task 9 `tunnel`.

pub mod config;
pub mod known_hosts;
pub mod tunnel;

use std::path::PathBuf;
use std::time::Duration;

use base64::Engine as _;
use futures::future::BoxFuture;
use secrecy::SecretString;

use crate::direct::NetError;

pub use config::{
    ConfigIssue, ConfigNotice, SshConfig, SshConfigHost, read_ssh_config, resolve_alias,
};
pub use known_hosts::KnownHosts;
pub use tunnel::{AgentEndpoint, TunnelConfig, TunnelLease, TunnelManager, classify_channel_err};

/// How one hop authenticates.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum SshAuth {
    Agent,
    KeyFile { path: PathBuf },
    Password,
    KeyboardInteractive,
}

/// One SSH hop.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct HopSpec {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub auth: SshAuth,
    pub identities_only: bool,
    /// The `IdentityFile` values imported for this hop.
    pub identity_files: Vec<PathBuf>,
}

/// What to do with a host key Hacivat has not seen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum HostKeyPolicy {
    #[default]
    Ask,
    AcceptNew,
    Strict,
}

impl HostKeyPolicy {
    pub const ALL: [HostKeyPolicy; 3] = [
        HostKeyPolicy::Ask,
        HostKeyPolicy::AcceptNew,
        HostKeyPolicy::Strict,
    ];

    /// The stable ID: `ask`, `accept_new`, `strict`.
    pub fn id(self) -> &'static str {
        match self {
            HostKeyPolicy::Ask => "ask",
            HostKeyPolicy::AcceptNew => "accept_new",
            HostKeyPolicy::Strict => "strict",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|p| p.id() == id)
    }
}

/// A profile's SSH tunnel: the hops in order, the last one the bastion next to the database.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct SshSpec {
    pub hops: Vec<HopSpec>,
    pub host_keys: HostKeyPolicy,
    /// Seconds; `None` keeps the tunnel default.
    pub keepalive_s: Option<u32>,
}

/// A server's host key.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct HostKey {
    /// The SSH algorithm name (`ssh-ed25519`, `ecdsa-sha2-nistp256`, `rsa-sha2-512`, ...).
    pub algorithm: String,
    /// The SHA-256 of the public key blob.
    pub sha256: [u8; 32],
    /// The key in OpenSSH `known_hosts` form (`<algorithm> <base64>`).
    pub openssh: String,
}

impl HostKey {
    /// OpenSSH's fingerprint form: `SHA256:` and the base64 of the hash, without padding.
    pub fn fingerprint(&self) -> String {
        format!(
            "SHA256:{}",
            base64::engine::general_purpose::STANDARD_NO_PAD.encode(self.sha256)
        )
    }
}

/// Where a known host key was found.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum KnownSource {
    Hacivat,
    OpenSsh { path: PathBuf, line: usize },
}

/// What the `known_hosts` stores say about a presented key.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum HostKeyCheck {
    Known(KnownSource),
    Unknown,
    Changed { old: HostKey, source: KnownSource },
    Revoked { source: KnownSource },
}

/// Identifies a shared chain of hops (the hash of its hosts, ports, users and auth).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ChainKey(pub [u8; 32]);

/// One incarnation of a shared chain: a reconnect of the chain bumps `generation`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TransportGeneration {
    pub key: ChainKey,
    pub generation: u64,
}

/// What the tunnel manager reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TunnelEvent {
    /// A chain's SSH session died; connections over that generation are lost.
    Dead { key: ChainKey, generation: u64 },
}

/// Which secret an SSH hop asks for.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum SshSecretKind {
    Password { host: String, user: String },
    Passphrase { key: PathBuf },
}

/// One keyboard-interactive prompt.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct KbdPrompt {
    pub text: String,
    /// Whether the answer may be shown as it is typed.
    pub echo: bool,
}

/// A keyboard-interactive round from the server.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct KbdRequest {
    pub host: String,
    pub name: String,
    pub instructions: String,
    pub prompts: Vec<KbdPrompt>,
}

/// How the tunnel asks the user (the pipeline bridges it to its `UiPort`). `hop` counts from 0.
pub trait SshPrompter: Send + Sync + 'static {
    /// Whether to trust an unknown host key.
    fn host_key(&self, hop: u8, host: &str, port: u16, key: &HostKey) -> BoxFuture<'static, bool>;
    /// A password or a key passphrase; `None` cancels.
    fn secret(
        &self,
        hop: u8,
        kind: SshSecretKind,
        attempt: u8,
    ) -> BoxFuture<'static, Option<SecretString>>;
    /// The answers to a keyboard-interactive round; `None` cancels.
    fn keyboard(
        &self,
        hop: u8,
        req: KbdRequest,
        attempt: u8,
    ) -> BoxFuture<'static, Option<Vec<SecretString>>>;
}

/// The secrets of one hop: a password and a key passphrase. `Debug` redacts them; no
/// `PartialEq`.
#[derive(Debug)]
pub struct HopSecrets {
    pub password: Option<SecretString>,
    pub passphrase: Option<SecretString>,
}

/// One tunnel to open (or share). Holds secrets, so `Debug` only (redacted).
#[derive(Debug)]
pub struct TunnelRequest {
    pub spec: SshSpec,
    /// Per hop, in order; a missing entry means none.
    pub secrets: Vec<HopSecrets>,
    /// Whether an open chain with the same key may be reused.
    pub share: bool,
    pub connect_timeout: Duration,
}

/// Why a tunnel could not be opened. `hop` counts from 0.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum SshError {
    HostKeyUnknown {
        hop: u8,
        host: String,
        port: u16,
        key: HostKey,
    },
    HostKeyChanged {
        hop: u8,
        host: String,
        port: u16,
        old: HostKey,
        new: HostKey,
        source: KnownSource,
    },
    HostKeyRevoked {
        hop: u8,
        host: String,
        port: u16,
    },
    Auth {
        hop: u8,
        user: String,
        host: String,
    },
    AgentUnavailable {
        hop: u8,
    },
    ChannelOpen {
        hop: u8,
        reason: String,
    },
    Unreachable {
        hop: u8,
        net: NetError,
    },
    Timeout {
        hop: u8,
    },
    Cancelled,
    Protocol {
        hop: u8,
    },
}

/// Machine text for logs: the hop index and the host, never a secret (an `SshError` holds none).
impl std::fmt::Display for SshError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SshError::HostKeyUnknown {
                hop,
                host,
                port,
                key,
            } => write!(
                f,
                "ssh hop {hop}: unknown host key {} for {host} port {port}",
                key.fingerprint()
            ),
            SshError::HostKeyChanged {
                hop,
                host,
                port,
                old,
                new,
                ..
            } => write!(
                f,
                "ssh hop {hop}: host key for {host} port {port} changed from {} to {}",
                old.fingerprint(),
                new.fingerprint()
            ),
            SshError::HostKeyRevoked { hop, host, port } => {
                write!(
                    f,
                    "ssh hop {hop}: host key for {host} port {port} is revoked"
                )
            }
            SshError::Auth { hop, user, host } => {
                write!(
                    f,
                    "ssh hop {hop}: authentication failed for {user} at {host}"
                )
            }
            SshError::AgentUnavailable { hop } => write!(f, "ssh hop {hop}: no SSH agent"),
            SshError::ChannelOpen { hop, reason } => {
                write!(f, "ssh hop {hop}: channel refused ({reason})")
            }
            SshError::Unreachable { hop, net } => write!(f, "ssh hop {hop}: unreachable ({net:?})"),
            SshError::Timeout { hop } => write!(f, "ssh hop {hop}: timed out"),
            SshError::Cancelled => f.write_str("ssh: cancelled"),
            SshError::Protocol { hop } => write!(f, "ssh hop {hop}: protocol error"),
        }
    }
}

impl std::error::Error for SshError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn policy_ids_round_trip_and_ask_is_the_default() {
        for p in HostKeyPolicy::ALL {
            assert_eq!(HostKeyPolicy::from_id(p.id()), Some(p));
        }
        assert_eq!(HostKeyPolicy::default(), HostKeyPolicy::Ask);
        assert_eq!(HostKeyPolicy::from_id("Ask"), None);
    }

    #[test]
    fn fingerprints_are_openssh_style() {
        let key = HostKey {
            algorithm: "ssh-ed25519".into(),
            sha256: [0; 32],
            openssh: String::new(),
        };
        assert_eq!(
            key.fingerprint(),
            "SHA256:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"
        );
    }

    #[test]
    fn secrets_never_print() {
        let hop = HopSecrets {
            password: Some(SecretString::from("s3cret")),
            passphrase: Some(SecretString::from("s3cret")),
        };
        let req = TunnelRequest {
            spec: SshSpec::default(),
            secrets: vec![hop],
            share: false,
            connect_timeout: Duration::from_secs(1),
        };
        assert!(!format!("{req:?}").contains("s3cret"));
    }
}
