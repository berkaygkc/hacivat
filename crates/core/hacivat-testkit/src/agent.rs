//! An in-process SSH agent on a Unix socket, for agent-auth tests (Task 9; unix only).
//!
//! Task 1 ships the API with a stub: the agent serves nothing at its socket path.

#[cfg(unix)]
use std::path::PathBuf;

#[cfg(unix)]
use russh::keys::PrivateKey;

/// A running test agent.
#[cfg(unix)]
#[derive(Debug)]
pub struct TestAgent {
    /// The socket path to put in `SSH_AUTH_SOCK` (passed to the code under test, never set
    /// in the environment).
    pub socket: PathBuf,
}

#[cfg(unix)]
impl TestAgent {
    /// Starts an agent holding `keys`. Task 1 stub: nothing listens at `socket`.
    pub fn start(keys: Vec<PrivateKey>) -> TestAgent {
        let _ = keys;
        TestAgent {
            socket: std::env::temp_dir().join("hacivat-test-agent.sock"),
        }
    }
}
