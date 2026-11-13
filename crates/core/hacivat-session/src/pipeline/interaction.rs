//! Asking the user during a connect (alg/01 §0.4): headless connects never prompt.

use std::fmt;
use std::net::IpAddr;
use std::path::PathBuf;
use std::sync::Arc;

use futures::future::BoxFuture;
use hacivat_net::CertReview;
use hacivat_net::ssh::{HostKey, KbdRequest};
use hacivat_proto::ProfileId;
use secrecy::SecretString;

/// What the pipeline asks (HV-CONN-01).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Question {
    Password {
        profile: ProfileId,
        name: String,
        user: String,
        /// 1 for the first prompt; the pipeline stops after the third (P8).
        attempt: u8,
    },
    // ## m2b-t01-interfaces
    /// Trust (pin) a certificate the verifier refused, or review a changed pin (Task 19).
    Certificate {
        profile: ProfileId,
        name: String,
        review: CertReview,
    },
    /// Credentials would cross a public network in plaintext (Task 19).
    PlaintextPublic {
        profile: ProfileId,
        name: String,
        host: String,
        ip: Option<IpAddr>,
    },
    /// An unknown SSH host key (Task 21). `hop` counts from 0.
    HostKey {
        profile: ProfileId,
        name: String,
        hop: u8,
        host: String,
        port: u16,
        key: HostKey,
    },
    /// An SSH password, an SSH key passphrase or a TLS key passphrase (Task 21).
    Secret {
        profile: ProfileId,
        name: String,
        prompt: SecretPrompt,
        attempt: u8,
    },
    /// A keyboard-interactive round (one-time codes; Task 21).
    Keyboard {
        profile: ProfileId,
        name: String,
        hop: u8,
        request: KbdRequest,
        attempt: u8,
    },
    /// The vault's master password (Task 21).
    UnlockVault { attempt: u8 },
    /// A new vault's master password, before the first `Forever` secret (Q38, Task 21).
    CreateVault,
}

// ## m2b-t01-interfaces
/// Which secret a [`Question::Secret`] asks for.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum SecretPrompt {
    SshPassword { hop: u8, host: String, user: String },
    SshPassphrase { hop: u8, key: PathBuf },
    TlsKeyPassphrase { key: PathBuf },
}

/// The user's answer (HV-CONN-01). Holds a secret, so `Debug` only.
#[derive(Debug)]
pub enum Answer {
    Password(SecretString),
    Cancelled,
    // ## m2b-t01-interfaces
    /// Pin the reviewed certificate.
    Pin,
    /// Trust the CA file the user picked.
    ImportCa(PathBuf),
    ConnectPlaintext {
        remember: bool,
    },
    /// Connect through SSH instead (opens the dialog's SSH section).
    UseSsh,
    TrustHostKey,
    Secret(SecretString),
    Keyboard(Vec<SecretString>),
    /// The vault's master password.
    Master(SecretString),
    // ## m2b-t21-prompts (fix round 1, Remember seam; named exception in Task 14's files)
    /// An SSH password or key passphrase, with the retention the prompt's Remember chose: the
    /// connect stores its typed SSH secrets under it and, when it differs from the profile's
    /// `ssh_retention`, a successful Main connect reports `ProfileChange::SshRetention`.
    SshSecret {
        secret: SecretString,
        remember: hacivat_secrets::Retention,
    },
}

/// The UI side of a connect: shows a question and resolves with the answer (HV-CONN-01).
pub trait UiPort: Send + Sync + 'static {
    fn ask(&self, q: Question) -> BoxFuture<'static, Answer>;
}

/// Whether a connect may ask the user (alg/01 §0.4). `Headless` fails with
/// `needs_interaction` instead of prompting.
#[derive(Clone)]
pub enum Interaction {
    Interactive(Arc<dyn UiPort>),
    Headless,
}

impl fmt::Debug for Interaction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Interaction::Interactive(_) => f.write_str("Interactive"),
            Interaction::Headless => f.write_str("Headless"),
        }
    }
}
