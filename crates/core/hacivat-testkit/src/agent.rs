//! An in-process SSH agent on a Unix socket, for agent-auth tests (Task 9; unix only).
//!
//! It speaks the agent protocol's two client requests a key login needs
//! (`REQUEST_IDENTITIES`, `SIGN_REQUEST`, draft-miller-ssh-agent §4.4–4.5) and answers anything
//! else with `FAILURE`. russh's own agent server (`russh::keys::agent::server::serve`) takes a
//! `futures::Stream` of connections, and the testkit has no `futures` dependency (Task 1 owns
//! the manifests), so this server is written against tokio directly; keys and signatures are
//! russh's (`ssh-key`). Ed25519 and ECDSA keys sign; RSA keys are listed but refuse to sign
//! (tests use Ed25519).
//!
//! The socket lives in a fresh `0700` directory under the temp dir. Dropping the agent stops it
//! and removes both.

#[cfg(unix)]
use std::path::PathBuf;

#[cfg(unix)]
use russh::keys::PrivateKey;

#[cfg(unix)]
use imp::Server;

/// A running test agent.
#[cfg(unix)]
#[derive(Debug)]
pub struct TestAgent {
    /// The socket path to put in `SSH_AUTH_SOCK` (passed to the code under test, never set
    /// in the environment).
    pub socket: PathBuf,
    server: Server,
}

#[cfg(unix)]
impl TestAgent {
    /// Starts an agent holding `keys`. Needs a tokio runtime.
    pub async fn start(keys: Vec<PrivateKey>) -> TestAgent {
        let server = Server::start(keys);
        TestAgent {
            socket: server.socket.clone(),
            server,
        }
    }

    /// Sign requests answered so far (successful or not).
    pub fn sign_requests(&self) -> usize {
        self.server.sign_requests()
    }
}

#[cfg(unix)]
mod imp {
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

    use russh::keys::PrivateKey;
    use russh::keys::signature::Signer;
    use russh::keys::ssh_encoding::Encode;
    use russh::keys::ssh_key::Signature;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::{UnixListener, UnixStream};
    use tokio::task::JoinHandle;

    const FAILURE: u8 = 5;
    const REQUEST_IDENTITIES: u8 = 11;
    const IDENTITIES_ANSWER: u8 = 12;
    const SIGN_REQUEST: u8 = 13;
    const SIGN_RESPONSE: u8 = 14;
    /// The largest request read (OpenSSH's agent uses 256 KiB).
    const MAX_FRAME: usize = 256 * 1024;

    #[derive(Debug)]
    pub(super) struct Server {
        pub(super) socket: PathBuf,
        dir: PathBuf,
        signs: Arc<AtomicUsize>,
        accept: JoinHandle<()>,
    }

    impl Server {
        pub(super) fn start(keys: Vec<PrivateKey>) -> Server {
            use std::os::unix::fs::DirBuilderExt;
            static N: AtomicU64 = AtomicU64::new(0);
            let dir = std::env::temp_dir().join(format!(
                "hv-agent-{}-{}",
                std::process::id(),
                N.fetch_add(1, Ordering::SeqCst)
            ));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::DirBuilder::new()
                .mode(0o700)
                .create(&dir)
                .expect("the agent's socket directory");
            let socket = dir.join("agent.sock");
            let listener = UnixListener::bind(&socket).expect("bind the agent socket");
            let keys = Arc::new(keys);
            let signs = Arc::new(AtomicUsize::new(0));
            let counter = signs.clone();
            let accept = tokio::spawn(async move {
                while let Ok((stream, _)) = listener.accept().await {
                    tokio::spawn(connection(stream, keys.clone(), counter.clone()));
                }
            });
            Server {
                socket,
                dir,
                signs,
                accept,
            }
        }

        pub(super) fn sign_requests(&self) -> usize {
            self.signs.load(Ordering::SeqCst)
        }
    }

    impl Drop for Server {
        fn drop(&mut self) {
            self.accept.abort();
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    async fn connection(
        mut stream: UnixStream,
        keys: Arc<Vec<PrivateKey>>,
        signs: Arc<AtomicUsize>,
    ) {
        loop {
            let mut len = [0u8; 4];
            if stream.read_exact(&mut len).await.is_err() {
                return;
            }
            let len = u32::from_be_bytes(len) as usize;
            if len == 0 || len > MAX_FRAME {
                return;
            }
            let mut request = vec![0u8; len];
            if stream.read_exact(&mut request).await.is_err() {
                return;
            }
            let reply = match request[0] {
                REQUEST_IDENTITIES => identities(&keys),
                SIGN_REQUEST => {
                    signs.fetch_add(1, Ordering::SeqCst);
                    sign(&keys, &request[1..]).unwrap_or_else(|| vec![FAILURE])
                }
                _ => vec![FAILURE],
            };
            let mut frame = (reply.len() as u32).to_be_bytes().to_vec();
            frame.extend_from_slice(&reply);
            if stream.write_all(&frame).await.is_err() {
                return;
            }
        }
    }

    fn put_string(out: &mut Vec<u8>, bytes: &[u8]) {
        out.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
        out.extend_from_slice(bytes);
    }

    fn take_string<'a>(input: &mut &'a [u8]) -> Option<&'a [u8]> {
        let len = u32::from_be_bytes(input.get(..4)?.try_into().ok()?) as usize;
        let bytes = input.get(4..4usize.checked_add(len)?)?;
        *input = &input[4 + len..];
        Some(bytes)
    }

    fn identities(keys: &[PrivateKey]) -> Vec<u8> {
        let blobs: Vec<Vec<u8>> = keys
            .iter()
            .filter_map(|k| k.public_key().to_bytes().ok())
            .collect();
        let mut out = vec![IDENTITIES_ANSWER];
        out.extend_from_slice(&(blobs.len() as u32).to_be_bytes());
        for blob in &blobs {
            put_string(&mut out, blob);
            put_string(&mut out, b"hacivat-test");
        }
        out
    }

    /// `string key_blob, string data, uint32 flags` → `SIGN_RESPONSE string signature`.
    fn sign(keys: &[PrivateKey], mut body: &[u8]) -> Option<Vec<u8>> {
        let blob = take_string(&mut body)?;
        let data = take_string(&mut body)?;
        let key = keys
            .iter()
            .find(|k| k.public_key().to_bytes().ok().as_deref() == Some(blob))?;
        if key.algorithm().is_rsa() {
            return None;
        }
        let signature: Signature = key.try_sign(data).ok()?;
        let encoded = signature.encode_vec().ok()?;
        let mut out = vec![SIGN_RESPONSE];
        put_string(&mut out, &encoded);
        Some(out)
    }
}
