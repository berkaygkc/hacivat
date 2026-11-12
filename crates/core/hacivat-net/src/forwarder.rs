//! The loopback forwarder (R18): a local listener that pipes accepted connections into a
//! [`StreamFactory`], for drivers that cannot take an injected stream (MySQL).

use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use futures::future::BoxFuture;
use tokio::net::TcpListener;
use tokio::task::{JoinHandle, JoinSet};

use crate::cleanup::CleanupGuard;
use crate::stream::{BoxedStream, StreamFactory};

/// How long one `factory.open()` may take before that accepted connection is closed: the
/// profile's default connect timeout. Opens run in accept order, so without a bound a stalled
/// factory (a future SSH channel) would hold every later connection.
pub const OPEN_TIMEOUT: Duration = Duration::from_secs(15);

/// The pause before accepting again when the process is out of file descriptors.
const ACCEPT_BACKOFF: Duration = Duration::from_millis(50);

/// Where a forwarder listens (R18).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ForwarderAddr {
    Tcp(std::net::SocketAddr),
    Unix(PathBuf),
}

/// How a forwarder binds (R18, P10): `Tcp` always binds `127.0.0.1:0` (the MySQL path, every
/// OS); `UnixOrTcp` binds a `0600` Unix socket in a `0700` directory on Unix and `127.0.0.1:0`
/// on Windows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ForwarderMode {
    Tcp,
    UnixOrTcp,
}

/// A bound loopback forwarder (R18). It accepts at most `max_accepts` connections.
#[derive(Debug)]
pub struct LoopbackForwarder {
    addr: ForwarderAddr,
    accepted: Arc<AtomicU64>,
}

impl LoopbackForwarder {
    /// Binds a forwarder for `factory` and starts accepting on the current Tokio runtime.
    ///
    /// Each accepted connection gets the next `factory.open()`, in accept order, and the two
    /// are relayed byte for byte (TLS stays end to end). An open that fails, or takes longer
    /// than [`OPEN_TIMEOUT`], closes that accepted connection. After `max_accepts` connections
    /// the listener closes (a Unix socket file is removed), while the relays keep running.
    ///
    /// A connection that is gone before it is accepted (reset or aborted) does not use up an
    /// accept; running out of file descriptors pauses accepting briefly instead of stopping.
    ///
    /// The guard closes the listener, stops every relay and removes the forwarder's private
    /// directory. Dropping the guard without running it (a cancelled connect) does the same,
    /// except that it does not wait for the accept task to finish.
    ///
    /// In `UnixOrTcp` mode on Unix the socket lives in a fresh `0700` directory inside
    /// `private_dir` (created `0700` if missing) and is `0600` itself.
    pub fn bind(
        factory: StreamFactory,
        private_dir: &Path,
        max_accepts: u32,
        mode: ForwarderMode,
    ) -> BoxFuture<'static, io::Result<(LoopbackForwarder, CleanupGuard)>> {
        bind_with(factory, private_dir, max_accepts, mode, OPEN_TIMEOUT)
    }

    pub fn addr(&self) -> &ForwarderAddr {
        &self.addr
    }

    /// How many connections it accepted so far.
    pub fn accepted(&self) -> u64 {
        self.accepted.load(Ordering::SeqCst)
    }
}

/// [`LoopbackForwarder::bind`] with its open timeout as a parameter (unit tests shorten it).
fn bind_with(
    factory: StreamFactory,
    private_dir: &Path,
    max_accepts: u32,
    mode: ForwarderMode,
    open_timeout: Duration,
) -> BoxFuture<'static, io::Result<(LoopbackForwarder, CleanupGuard)>> {
    let private_dir = private_dir.to_path_buf();
    Box::pin(async move {
        let Bound {
            listener,
            addr,
            dir,
        } = Listener::bind(&private_dir, mode).await?;
        // Owns the directory from here on, so it is removed on every path.
        let mut teardown = Teardown { task: None, dir };
        let accepted = Arc::new(AtomicU64::new(0));
        teardown.task = Some(tokio::spawn(run(
            listener,
            addr.clone(),
            factory,
            max_accepts,
            accepted.clone(),
            open_timeout,
        )));
        let guard = CleanupGuard::new(move || Box::pin(teardown.run()));
        Ok((LoopbackForwarder { addr, accepted }, guard))
    })
}

/// Stops the accept task (dropping it drops the listener and its relay set, which aborts
/// every relay) and removes the private directory: when the guard runs, and also when the
/// guard is dropped without running, because a dropped `JoinHandle` does not abort its task.
struct Teardown {
    task: Option<JoinHandle<()>>,
    dir: Option<PathBuf>,
}

impl Teardown {
    async fn run(mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
            let _ = task.await;
        }
        // `Drop` removes the directory.
    }
}

impl Drop for Teardown {
    fn drop(&mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
        }
        if let Some(dir) = self.dir.take() {
            let _ = std::fs::remove_dir_all(dir);
        }
    }
}

async fn run(
    listener: Listener,
    addr: ForwarderAddr,
    factory: StreamFactory,
    max_accepts: u32,
    accepted: Arc<AtomicU64>,
    open_timeout: Duration,
) {
    let max = u64::from(max_accepts);
    let mut listener = Some(listener);
    let mut relays = JoinSet::new();
    let mut n = 0;
    while n < max {
        let Some(l) = listener.as_ref() else { break };
        let client = match l.accept().await {
            Ok(client) => client,
            Err(e) => match on_accept_error(&e) {
                AcceptErrorAction::Skip => continue,
                AcceptErrorAction::Backoff => {
                    tokio::time::sleep(ACCEPT_BACKOFF).await;
                    continue;
                }
                AcceptErrorAction::Stop => break,
            },
        };
        n += 1;
        accepted.store(n, Ordering::SeqCst);
        if n == max {
            close(&mut listener, &addr);
        }
        // Opened in accept order: the first open goes to the first connection.
        match tokio::time::timeout(open_timeout, factory.open()).await {
            Ok(Ok(upstream)) => {
                relays.spawn(relay(client, upstream));
            }
            Ok(Err(_)) | Err(_) => drop(client),
        }
    }
    close(&mut listener, &addr);
    while relays.join_next().await.is_some() {}
}

#[derive(Debug, PartialEq, Eq)]
enum AcceptErrorAction {
    /// The connection died before it was accepted; take the next one.
    Skip,
    /// Out of file descriptors; wait a little and try again.
    Backoff,
    /// The listener itself is broken.
    Stop,
}

fn on_accept_error(e: &io::Error) -> AcceptErrorAction {
    match e.kind() {
        io::ErrorKind::ConnectionAborted
        | io::ErrorKind::ConnectionReset
        | io::ErrorKind::Interrupted => AcceptErrorAction::Skip,
        _ if out_of_descriptors(e) => AcceptErrorAction::Backoff,
        _ => AcceptErrorAction::Stop,
    }
}

/// `EMFILE` or `ENFILE` (24 and 23 on Linux and the BSDs, macOS included).
#[cfg(unix)]
fn out_of_descriptors(e: &io::Error) -> bool {
    matches!(e.raw_os_error(), Some(23 | 24))
}

/// `WSAEMFILE`.
#[cfg(windows)]
fn out_of_descriptors(e: &io::Error) -> bool {
    matches!(e.raw_os_error(), Some(10024))
}

#[cfg(not(any(unix, windows)))]
fn out_of_descriptors(_e: &io::Error) -> bool {
    false
}

fn close(listener: &mut Option<Listener>, addr: &ForwarderAddr) {
    if listener.take().is_some()
        && let ForwarderAddr::Unix(path) = addr
    {
        let _ = std::fs::remove_file(path);
    }
}

async fn relay(mut client: BoxedStream, mut upstream: BoxedStream) {
    let _ = tokio::io::copy_bidirectional(&mut client, &mut upstream).await;
}

enum Listener {
    Tcp(TcpListener),
    #[cfg(unix)]
    Unix(tokio::net::UnixListener),
}

struct Bound {
    listener: Listener,
    addr: ForwarderAddr,
    /// The forwarder's own directory, removed by the guard.
    dir: Option<PathBuf>,
}

impl Listener {
    async fn bind(private_dir: &Path, mode: ForwarderMode) -> io::Result<Bound> {
        match mode {
            #[cfg(unix)]
            ForwarderMode::UnixOrTcp => bind_unix(private_dir),
            #[cfg(not(unix))]
            ForwarderMode::UnixOrTcp => {
                let _ = private_dir;
                bind_tcp().await
            }
            ForwarderMode::Tcp => bind_tcp().await,
        }
    }

    async fn accept(&self) -> io::Result<BoxedStream> {
        match self {
            Listener::Tcp(l) => {
                let (s, _) = l.accept().await?;
                // A connection reset while it waited in the backlog has no peer any more.
                if s.peer_addr().is_err() {
                    return Err(io::ErrorKind::ConnectionAborted.into());
                }
                // Only latency is at stake; a failure here is not worth dropping the client.
                let _ = s.set_nodelay(true);
                Ok(Box::new(s))
            }
            #[cfg(unix)]
            Listener::Unix(l) => {
                let (s, _) = l.accept().await?;
                Ok(Box::new(s))
            }
        }
    }
}

async fn bind_tcp() -> io::Result<Bound> {
    let l = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).await?;
    let addr = ForwarderAddr::Tcp(l.local_addr()?);
    Ok(Bound {
        listener: Listener::Tcp(l),
        addr,
        dir: None,
    })
}

#[cfg(unix)]
fn bind_unix(private_dir: &Path) -> io::Result<Bound> {
    use std::fs::{DirBuilder, Permissions, set_permissions};
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt};

    DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(private_dir)?;
    let dir = unique_private_dir(private_dir)?;
    let path = dir.join("fwd.sock");
    let bound = tokio::net::UnixListener::bind(&path).and_then(|l| {
        // The directory already keeps others out; the mode is the second fence.
        set_permissions(&path, Permissions::from_mode(0o600))?;
        Ok(l)
    });
    match bound {
        Ok(l) => Ok(Bound {
            listener: Listener::Unix(l),
            addr: ForwarderAddr::Unix(path),
            dir: Some(dir),
        }),
        Err(e) => {
            let _ = std::fs::remove_dir_all(&dir);
            Err(e)
        }
    }
}

/// Creates a new `0700` directory for one forwarder inside `parent`.
#[cfg(unix)]
fn unique_private_dir(parent: &Path) -> io::Result<PathBuf> {
    use std::fs::{DirBuilder, Permissions, set_permissions};
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt};

    static NEXT: AtomicU64 = AtomicU64::new(0);
    for _ in 0..64 {
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let dir = parent.join(format!("hv-fwd-{}-{n}", std::process::id()));
        match DirBuilder::new().mode(0o700).create(&dir) {
            Ok(()) => {
                // The umask can only narrow the mode; set it exactly anyway.
                set_permissions(&dir, Permissions::from_mode(0o700))?;
                return Ok(dir);
            }
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    Err(io::ErrorKind::AlreadyExists.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[test]
    fn accept_errors_skip_back_off_or_stop() {
        use io::ErrorKind::*;
        for kind in [ConnectionAborted, ConnectionReset, Interrupted] {
            assert_eq!(
                on_accept_error(&kind.into()),
                AcceptErrorAction::Skip,
                "{kind:?}"
            );
        }
        if cfg!(unix) {
            for code in [23, 24] {
                assert_eq!(
                    on_accept_error(&io::Error::from_raw_os_error(code)),
                    AcceptErrorAction::Backoff
                );
            }
        }
        assert_eq!(
            on_accept_error(&InvalidInput.into()),
            AcceptErrorAction::Stop
        );
    }

    #[tokio::test]
    async fn a_stalled_open_is_cut_off_and_the_next_connection_is_served() {
        // The first open never finishes; the second hands out a stream that says "ok".
        let calls = Arc::new(AtomicUsize::new(0));
        let counter = calls.clone();
        let factory = StreamFactory::new(move || {
            let first = counter.fetch_add(1, Ordering::SeqCst) == 0;
            Box::pin(async move {
                if first {
                    std::future::pending::<()>().await;
                }
                let (a, mut b) = tokio::io::duplex(64);
                b.write_all(b"ok").await?;
                tokio::spawn(async move {
                    let mut sink = [0u8; 1];
                    let _ = b.read(&mut sink).await;
                });
                Ok(Box::new(a) as BoxedStream)
            })
        });
        let dir = tempfile::tempdir().unwrap();
        let (fwd, guard) = bind_with(
            factory,
            dir.path(),
            2,
            ForwarderMode::Tcp,
            Duration::from_millis(100),
        )
        .await
        .unwrap();
        let ForwarderAddr::Tcp(a) = *fwd.addr() else {
            panic!("expected TCP")
        };
        let mut stalled = tokio::net::TcpStream::connect(a).await.unwrap();
        let mut buf = [0u8; 2];
        let n = tokio::time::timeout(Duration::from_secs(2), stalled.read(&mut buf))
            .await
            .expect("the stalled connection is closed after the open timeout");
        assert!(matches!(n, Ok(0) | Err(_)));

        let mut served = tokio::net::TcpStream::connect(a).await.unwrap();
        tokio::time::timeout(Duration::from_secs(2), served.read_exact(&mut buf))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(&buf, b"ok");
        assert_eq!(fwd.accepted(), 2);
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        guard.run().await;
    }
}
