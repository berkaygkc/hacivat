//! Direct transport: name resolution and Happy Eyeballs TCP or Unix connects (HV-CONN-01).

use std::collections::VecDeque;
use std::future::{Future, poll_fn};
use std::io;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::task::Poll;
use std::time::Duration;

use futures::StreamExt;
use futures::stream::FuturesUnordered;
use tokio::net::TcpStream;
use tokio::time::{Instant, sleep};

use crate::address::parse_ip_literal;
use crate::stream::{BoxedStream, StreamFactory};

/// The delay before the next address is tried while earlier attempts still run (RFC 8305
/// "Connection Attempt Delay").
pub(crate) const STAGGER: Duration = Duration::from_millis(250);

/// Where a direct transport connects (HV-CONN-01).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Target {
    Tcp { host: String, port: u16 },
    Unix { path: PathBuf },
}

/// Why resolving or connecting failed, before any protocol byte (HV-CONN-03).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum NetError {
    Dns {
        host: String,
    },
    Refused,
    Timeout,
    Unreachable,
    Reset,
    Io {
        kind: io::ErrorKind,
        message: String,
    },
}

/// Resolves `host` with the system resolver. An IP literal (`10.0.0.5`, `::1`, `[::1]`)
/// resolves to itself without a query. Any resolver failure, or an empty answer, is
/// [`NetError::Dns`]. Duplicate answers are dropped; the resolver's order is kept.
pub async fn resolve(host: &str, port: u16) -> Result<Vec<SocketAddr>, NetError> {
    if let Some(ip) = parse_ip_literal(host) {
        return Ok(vec![SocketAddr::new(ip, port)]);
    }
    let dns = || NetError::Dns {
        host: host.to_owned(),
    };
    let answers = tokio::net::lookup_host((host, port))
        .await
        .map_err(|_| dns())?;
    let mut addrs: Vec<SocketAddr> = Vec::new();
    for a in answers {
        if !addrs.contains(&a) {
            addrs.push(a);
        }
    }
    if addrs.is_empty() {
        return Err(dns());
    }
    Ok(addrs)
}

/// A factory that opens a new connection to `target` on every call.
///
/// TCP uses Happy Eyeballs (RFC 8305) over `resolved`: the addresses alternate between IPv6
/// and IPv4 (starting with the family of the first one), a new attempt starts every 250 ms or
/// as soon as the previous one fails, and the first to connect wins. `connect_timeout` bounds
/// the whole open; it fails with `TimedOut`. When every attempt fails the most telling error is
/// returned (a refusal over "network unreachable"). An empty `resolved` fails with
/// `InvalidInput`. Unix targets connect to the socket path (Unix only).
///
/// Windows retries a SYN answered by a reset before it reports the refusal (about 2 s, even on
/// loopback), so there a refused port is `ConnectionRefused` only when `connect_timeout` is
/// longer than that, and `TimedOut` otherwise. The default (15 s) is; turning the retries off
/// (`SIO_TCP_INITIAL_RTO`) would also drop them for real lossy links, so the stack's
/// behaviour is kept.
pub fn direct(
    target: Target,
    resolved: Vec<SocketAddr>,
    connect_timeout: Duration,
) -> StreamFactory {
    match target {
        Target::Tcp { .. } => {
            let addrs: Arc<[SocketAddr]> = interleave_families(resolved).into();
            StreamFactory::new(move || {
                let addrs = addrs.clone();
                Box::pin(async move {
                    let stream = within(
                        connect_timeout,
                        happy_eyeballs(&addrs, STAGGER, connect_tcp),
                    )
                    .await?;
                    Ok(Box::new(stream) as BoxedStream)
                })
            })
        }
        Target::Unix { path } => {
            let path: Arc<PathBuf> = Arc::new(path);
            StreamFactory::new(move || {
                let path = path.clone();
                Box::pin(async move { within(connect_timeout, connect_unix(path)).await })
            })
        }
    }
}

async fn connect_tcp(addr: SocketAddr) -> io::Result<TcpStream> {
    let stream = TcpStream::connect(addr).await?;
    // Database protocols are request/response; Nagle only adds latency.
    stream.set_nodelay(true)?;
    Ok(stream)
}

#[cfg(unix)]
async fn connect_unix(path: Arc<PathBuf>) -> io::Result<BoxedStream> {
    let stream = tokio::net::UnixStream::connect(path.as_path()).await?;
    Ok(Box::new(stream))
}

#[cfg(not(unix))]
async fn connect_unix(_path: Arc<PathBuf>) -> io::Result<BoxedStream> {
    Err(io::ErrorKind::Unsupported.into())
}

/// Runs `fut` with an overall deadline; running out is `TimedOut`.
async fn within<T>(limit: Duration, fut: impl Future<Output = io::Result<T>>) -> io::Result<T> {
    tokio::time::timeout(limit, fut)
        .await
        .unwrap_or_else(|_| Err(io::ErrorKind::TimedOut.into()))
}

/// Orders addresses for Happy Eyeballs: alternate families, starting with the first
/// address's family, each family keeping its own order (RFC 8305 §4).
pub(crate) fn interleave_families(addrs: Vec<SocketAddr>) -> Vec<SocketAddr> {
    let Some(first) = addrs.first() else {
        return addrs;
    };
    let lead_v6 = first.is_ipv6();
    let (mut lead, mut other): (VecDeque<SocketAddr>, VecDeque<SocketAddr>) =
        addrs.iter().partition(|a| a.is_ipv6() == lead_v6);
    let mut out = Vec::with_capacity(addrs.len());
    loop {
        match (lead.pop_front(), other.pop_front()) {
            (None, None) => return out,
            (a, b) => out.extend(a.into_iter().chain(b)),
        }
    }
}

/// Races connection attempts over `addrs` in order: the next starts after `stagger`, or at
/// once when an attempt fails. The first success wins and the rest are dropped.
pub(crate) async fn happy_eyeballs<S, F, Fut>(
    addrs: &[SocketAddr],
    stagger: Duration,
    connect: F,
) -> io::Result<S>
where
    F: Fn(SocketAddr) -> Fut,
    Fut: Future<Output = io::Result<S>>,
{
    let mut pending = addrs.iter().copied();
    let Some(first) = pending.next() else {
        return Err(io::ErrorKind::InvalidInput.into());
    };
    let mut attempts = FuturesUnordered::new();
    attempts.push(connect(first));
    let mut timer = Box::pin(sleep(stagger));
    let mut best: Option<io::Error> = None;
    poll_fn(|cx| {
        loop {
            match attempts.poll_next_unpin(cx) {
                Poll::Ready(Some(Ok(stream))) => return Poll::Ready(Ok(stream)),
                Poll::Ready(Some(Err(e))) => {
                    keep_most_telling(&mut best, e);
                    if let Some(next) = pending.next() {
                        attempts.push(connect(next));
                        timer.as_mut().reset(Instant::now() + stagger);
                    }
                    continue;
                }
                Poll::Ready(None) => {
                    // Every attempt failed and none is left.
                    let e = best
                        .take()
                        .unwrap_or_else(|| io::ErrorKind::InvalidInput.into());
                    return Poll::Ready(Err(e));
                }
                Poll::Pending => {}
            }
            if pending.len() > 0 && timer.as_mut().poll(cx).is_ready() {
                if let Some(next) = pending.next() {
                    attempts.push(connect(next));
                }
                timer.as_mut().reset(Instant::now() + stagger);
                continue;
            }
            return Poll::Pending;
        }
    })
    .await
}

/// Keeps the first error, unless it only says a route was missing and `e` says more (an
/// IPv6 address without a route should not hide an IPv4 refusal).
fn keep_most_telling(best: &mut Option<io::Error>, e: io::Error) {
    let vague = |k: io::ErrorKind| {
        matches!(
            k,
            io::ErrorKind::NetworkUnreachable
                | io::ErrorKind::HostUnreachable
                | io::ErrorKind::AddrNotAvailable
        )
    };
    if best
        .as_ref()
        .is_none_or(|b| vague(b.kind()) && !vague(e.kind()))
    {
        *best = Some(e);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use std::time::Instant as StdInstant;

    fn sa(s: &str) -> SocketAddr {
        s.parse().unwrap()
    }

    #[test]
    fn families_alternate_starting_with_the_first() {
        let v = interleave_families(vec![
            sa("[::1]:1"),
            sa("[::2]:1"),
            sa("[::3]:1"),
            sa("10.0.0.1:1"),
        ]);
        assert_eq!(
            v,
            vec![
                sa("[::1]:1"),
                sa("10.0.0.1:1"),
                sa("[::2]:1"),
                sa("[::3]:1")
            ]
        );
        let v = interleave_families(vec![sa("10.0.0.1:1"), sa("10.0.0.2:1"), sa("[::1]:1")]);
        assert_eq!(v, vec![sa("10.0.0.1:1"), sa("[::1]:1"), sa("10.0.0.2:1")]);
        assert!(interleave_families(vec![]).is_empty());
    }

    #[tokio::test]
    async fn a_hanging_attempt_is_overtaken_after_the_stagger() {
        let started = StdInstant::now();
        let order = Mutex::new(Vec::new());
        let won = happy_eyeballs(
            &[sa("10.0.0.1:1"), sa("10.0.0.2:1")],
            Duration::from_millis(100),
            |a| {
                order.lock().unwrap().push(a);
                async move {
                    if a == sa("10.0.0.1:1") {
                        std::future::pending::<io::Result<SocketAddr>>().await
                    } else {
                        Ok(a)
                    }
                }
            },
        )
        .await
        .unwrap();
        assert_eq!(won, sa("10.0.0.2:1"));
        assert!(started.elapsed() >= Duration::from_millis(100));
        assert_eq!(
            *order.lock().unwrap(),
            vec![sa("10.0.0.1:1"), sa("10.0.0.2:1")]
        );
    }

    #[tokio::test]
    async fn a_failure_starts_the_next_attempt_at_once() {
        let started = StdInstant::now();
        let won = happy_eyeballs(
            &[sa("10.0.0.1:1"), sa("10.0.0.2:1")],
            Duration::from_secs(30),
            |a| async move {
                if a == sa("10.0.0.1:1") {
                    Err(io::ErrorKind::ConnectionRefused.into())
                } else {
                    Ok(a)
                }
            },
        )
        .await
        .unwrap();
        assert_eq!(won, sa("10.0.0.2:1"));
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[tokio::test]
    async fn a_refusal_beats_an_unreachable_route() {
        let err = happy_eyeballs(
            &[sa("[::1]:1"), sa("10.0.0.1:1"), sa("10.0.0.2:1")],
            Duration::from_secs(30),
            |a| async move {
                Err::<(), _>(match a {
                    a if a.is_ipv6() => io::ErrorKind::NetworkUnreachable.into(),
                    a if a == sa("10.0.0.1:1") => io::ErrorKind::ConnectionRefused.into(),
                    _ => io::Error::from(io::ErrorKind::ConnectionReset),
                })
            },
        )
        .await
        .unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::ConnectionRefused);
    }

    #[tokio::test]
    async fn the_overall_timeout_is_timed_out() {
        let started = StdInstant::now();
        let err = within(
            Duration::from_millis(150),
            happy_eyeballs(&[sa("10.0.0.1:1")], STAGGER, |_| {
                std::future::pending::<io::Result<()>>()
            }),
        )
        .await
        .unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::TimedOut);
        assert!(started.elapsed() < Duration::from_secs(5));
    }
}
