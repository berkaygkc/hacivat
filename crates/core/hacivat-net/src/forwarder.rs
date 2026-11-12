//! The loopback forwarder (R18): a local listener that pipes accepted connections into a
//! [`StreamFactory`], for drivers that cannot take an injected stream (MySQL).

use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use futures::future::BoxFuture;
use tokio::net::TcpListener;
use tokio::task::JoinSet;

use crate::cleanup::CleanupGuard;
use crate::stream::{BoxedStream, StreamFactory};

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
    /// are relayed byte for byte (TLS stays end to end). After `max_accepts` connections the
    /// listener closes (a Unix socket file is removed), while the relays keep running. If an
    /// open fails, that accepted connection is closed. The guard closes the listener, stops
    /// every relay and removes the forwarder's private directory.
    ///
    /// In `UnixOrTcp` mode on Unix the socket lives in a fresh `0700` directory inside
    /// `private_dir` (created `0700` if missing) and is `0600` itself.
    pub fn bind(
        factory: StreamFactory,
        private_dir: &Path,
        max_accepts: u32,
        mode: ForwarderMode,
    ) -> BoxFuture<'static, io::Result<(LoopbackForwarder, CleanupGuard)>> {
        let private_dir = private_dir.to_path_buf();
        Box::pin(async move {
            let bound = Listener::bind(&private_dir, mode).await?;
            let addr = bound.addr.clone();
            let dir = bound.dir.clone();
            let accepted = Arc::new(AtomicU64::new(0));
            let task = tokio::spawn(run(bound, factory, max_accepts, accepted.clone()));
            let guard = CleanupGuard::new(move || {
                Box::pin(async move {
                    // Dropping the task drops the listener and its relay set, which aborts
                    // every relay.
                    task.abort();
                    let _ = task.await;
                    if let Some(dir) = dir {
                        let _ = std::fs::remove_dir_all(dir);
                    }
                })
            });
            Ok((LoopbackForwarder { addr, accepted }, guard))
        })
    }

    pub fn addr(&self) -> &ForwarderAddr {
        &self.addr
    }

    /// How many connections it accepted so far.
    pub fn accepted(&self) -> u64 {
        self.accepted.load(Ordering::SeqCst)
    }
}

async fn run(bound: Bound, factory: StreamFactory, max_accepts: u32, accepted: Arc<AtomicU64>) {
    let Bound {
        listener,
        addr,
        dir: _,
    } = bound;
    let mut listener = Some(listener);
    let mut relays = JoinSet::new();
    for n in 1..=u64::from(max_accepts) {
        let Some(l) = listener.as_ref() else { break };
        let Ok(client) = l.accept().await else {
            break;
        };
        accepted.store(n, Ordering::SeqCst);
        if n == u64::from(max_accepts) {
            close(&mut listener, &addr);
        }
        // Opened in accept order: the first open goes to the first connection.
        match factory.open().await {
            Ok(upstream) => {
                relays.spawn(relay(client, upstream));
            }
            Err(_) => drop(client),
        }
    }
    close(&mut listener, &addr);
    while relays.join_next().await.is_some() {}
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
                s.set_nodelay(true)?;
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
