//! An in-process SSH server for tunnel tests (Task 9, Q23, Q46): password, public-key and
//! keyboard-interactive users, `direct-tcpip` forwarding (allowed, denied or redirected to one
//! address), and probes for the channel-hygiene tests.
//!
//! Task 1 ships the API with a stub: the server does not listen (`addr` is port 0), counts
//! nothing, and refuses to open server channels. Task 9 fills it with russh.

use std::fmt;
use std::future::Future;
use std::io;
use std::net::{Ipv4Addr, SocketAddr};
use std::pin::Pin;
use std::time::Duration;

use hacivat_net::ssh::HostKey;
use russh::keys::PublicKey;

/// A boxed `Send` future, as `futures::future::BoxFuture` (the testkit has no `futures`).
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Whether `direct-tcpip` channels are allowed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ForwardPolicy {
    #[default]
    Allow,
    Deny,
}

/// Where an allowed `direct-tcpip` channel goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ForwardTarget {
    /// The host and port the client asked for.
    #[default]
    Any,
    /// Always this address (for names that do not resolve, such as `db.internal`).
    Redirect(SocketAddr),
}

/// One keyboard-interactive prompt and the answer the server expects. Holds a secret (the
/// answer), so `Debug` redacts it and there is no `PartialEq`.
#[derive(Clone)]
pub struct KbdStep {
    pub prompt: String,
    pub echo: bool,
    pub answer: String,
}

impl fmt::Debug for KbdStep {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("KbdStep")
            .field("prompt", &self.prompt)
            .field("echo", &self.echo)
            .field("answer", &"<redacted>")
            .finish()
    }
}

/// A test user. Holds secrets, so `Debug` redacts them and there is no `PartialEq`.
#[derive(Clone)]
pub enum TestUser {
    Password { name: String, password: String },
    Key { name: String, key: PublicKey },
    Keyboard { name: String, steps: Vec<KbdStep> },
}

impl fmt::Debug for TestUser {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TestUser::Password { name, .. } => f
                .debug_struct("Password")
                .field("name", name)
                .field("password", &"<redacted>")
                .finish(),
            TestUser::Key { name, key } => f
                .debug_struct("Key")
                .field("name", name)
                .field("key", key)
                .finish(),
            TestUser::Keyboard { name, steps } => f
                .debug_struct("Keyboard")
                .field("name", name)
                .field("steps", steps)
                .finish(),
        }
    }
}

/// The server's configuration, built with the methods below. `Debug` redacts the users'
/// secrets.
#[derive(Debug, Clone, Default)]
pub struct TestSshConfig {
    pub users: Vec<TestUser>,
    pub forward: ForwardPolicy,
    pub forward_to: ForwardTarget,
    pub max_auth_tries: Option<u32>,
    pub slow_banner: Option<Duration>,
    pub ipv6: bool,
}

impl TestSshConfig {
    pub fn user_pw(mut self, name: &str, pw: &str) -> Self {
        self.users.push(TestUser::Password {
            name: name.to_string(),
            password: pw.to_string(),
        });
        self
    }

    pub fn user_key(mut self, name: &str, key: PublicKey) -> Self {
        self.users.push(TestUser::Key {
            name: name.to_string(),
            key,
        });
        self
    }

    /// A keyboard-interactive user: `(prompt, echo, answer)` per round.
    pub fn user_kbd(mut self, name: &str, steps: &[(&str, bool, &str)]) -> Self {
        self.users.push(TestUser::Keyboard {
            name: name.to_string(),
            steps: steps
                .iter()
                .map(|(prompt, echo, answer)| KbdStep {
                    prompt: (*prompt).to_string(),
                    echo: *echo,
                    answer: (*answer).to_string(),
                })
                .collect(),
        });
        self
    }

    pub fn forward(mut self, policy: ForwardPolicy) -> Self {
        self.forward = policy;
        self
    }

    pub fn forward_to(mut self, target: ForwardTarget) -> Self {
        self.forward_to = target;
        self
    }

    pub fn max_auth_tries(mut self, n: u32) -> Self {
        self.max_auth_tries = Some(n);
        self
    }

    /// The server waits `d` before its banner (timeout tests).
    pub fn slow_banner(mut self, d: Duration) -> Self {
        self.slow_banner = Some(d);
        self
    }

    /// Listen on `[::1]` instead of `127.0.0.1`.
    pub fn ipv6(mut self) -> Self {
        self.ipv6 = true;
        self
    }
}

/// A running test server.
#[derive(Debug)]
pub struct TestSshServer {
    pub addr: SocketAddr,
    pub host_key: HostKey,
    #[allow(dead_code, reason = "Task 9 serves it")]
    config: TestSshConfig,
}

impl TestSshServer {
    /// Starts the server on loopback. Task 1 stub: does not listen (`addr` has port 0).
    pub fn start(config: TestSshConfig) -> TestSshServer {
        TestSshServer {
            addr: SocketAddr::from((Ipv4Addr::LOCALHOST, 0)),
            host_key: HostKey {
                algorithm: "ssh-ed25519".to_string(),
                sha256: [0; 32],
                openssh: String::new(),
            },
            config,
        }
    }

    /// The host as a profile names it (`127.0.0.1` or `::1`).
    pub fn host(&self) -> String {
        self.addr.ip().to_string()
    }

    pub fn auth_attempts(&self) -> usize {
        0
    }

    pub fn sessions_open(&self) -> usize {
        0
    }

    pub fn sessions_accepted(&self) -> usize {
        0
    }

    pub fn channels_opened(&self) -> usize {
        0
    }

    /// Drops every client session (a network cut).
    pub fn kill_sessions(&self) {}

    pub fn stop(&self) {}

    /// Asks the client to open a server-initiated channel (Q46: the client must refuse).
    /// Task 1 stub: fails.
    pub fn open_server_channel(&self) -> BoxFuture<'static, io::Result<()>> {
        Box::pin(async {
            Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "test ssh server: Task 9",
            ))
        })
    }

    /// The originator address and port of the last `direct-tcpip` request (Q46).
    pub fn last_originator(&self) -> Option<(String, u32)> {
        None
    }
}
