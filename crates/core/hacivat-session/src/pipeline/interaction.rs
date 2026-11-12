//! Asking the user during a connect (alg/01 §0.4): headless connects never prompt.

use std::fmt;
use std::sync::Arc;

use futures::future::BoxFuture;
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
}

/// The user's answer (HV-CONN-01). Holds a secret, so `Debug` only.
#[derive(Debug)]
pub enum Answer {
    Password(SecretString),
    Cancelled,
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
