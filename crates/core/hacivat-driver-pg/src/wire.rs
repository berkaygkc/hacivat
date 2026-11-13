//! The wire observer (Task 10, B2, B3, B8, B9, Q11, Q12): a layer on the decrypted side of the
//! injected stream that reads backend messages as they pass to `tokio-postgres`.
//!
//! - **Insecure-auth guard (B8, Q20).** An `AuthenticationCleartextPassword` (`R`, code 3) or
//!   `AuthenticationMD5Password` (`R`, code 5) under [`InsecureAuth::RefuseUnlessVerified`],
//!   when the TLS outcome is not `Full`, `CaOnly` or `Pinned`, fails the read with an
//!   [`InsecureAuthRefusedMarker`] before `tokio-postgres` sees the message, so no password
//!   response is ever written. The check runs on every `R` message, also after
//!   `AuthenticationOk` (a legitimate server never sends one there).
//! - **Backend PID (Q11).** `BackendKeyData` (`K`) after `AuthenticationOk` and before the first
//!   `ReadyForQuery`: `tokio-postgres` keeps the PID private.
//! - **Transaction status (Q12).** Every `ReadyForQuery` (`Z`) after `AuthenticationOk` records
//!   its status byte and advances a counter. The write side counts the frontend messages that
//!   each draw exactly one `ReadyForQuery` (`Query`, `Sync`, `FunctionCall`), so after an error
//!   (`tokio-postgres` returns at `ErrorResponse`, before the `Z` is read) the caller waits
//!   until every one has been answered, and the status is then exact.
//! - **Command tag.** The text of the last `CommandComplete` (first 64 bytes), which the
//!   read-only guard's post-check reads (`INSERT 0 1`, `CREATE TABLE`).
//!
//! **Layering (Q12).** Without TLS the raw observer runs [`ObserverMode::Framed`] from the
//! start. With an `SSLRequest` it starts in [`ObserverMode::AwaitSslResponse`] and sees only the
//! one-byte answer: after `S` it turns [`ObserverMode::Passthrough`] (the bytes that follow are
//! TLS records) and a second, framed observer runs above rustls on the decrypted side; after
//! `N` it frames the plaintext itself. Both observers share one [`WireState`].
//!
//! Framing is one type byte and a big-endian `i32` length that counts itself; a length below 4
//! or above 1 GiB is `InvalidData`. Message bodies are skipped by count: only the first bytes of
//! `R`, `K`, `Z` and `C` bodies are copied, into a fixed buffer.

use std::error::Error;
use std::fmt;
use std::io;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU8, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::task::{Context, Poll};
use std::time::Duration;

use hacivat_driver::{InsecureAuth, TxStatus};
use hacivat_net::TlsVerified;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::sync::Notify;

/// The largest message length accepted (the length field counts itself).
const MAX_LEN: u32 = 1 << 30;
/// How many bytes of a `CommandComplete` tag are kept.
const TAG_CAP: usize = 64;
/// How long a caller waits for the outstanding `ReadyForQuery` messages after an error.
pub(crate) const SETTLE: Duration = Duration::from_secs(1);

/// The inner value of the `io::Error` that refuses an insecure password request (B8). It maps
/// to `RawErrorKind::InsecureAuthRefused`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct InsecureAuthRefusedMarker;

impl fmt::Display for InsecureAuthRefusedMarker {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("the server asked for the password by an insecure method")
    }
}

impl Error for InsecureAuthRefusedMarker {}

/// The inner value of the `io::Error` that refuses a plaintext answer to the `SSLRequest` of a
/// stream that must run TLS (a cancel request of a TLS session).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TlsRefusedMarker;

impl fmt::Display for TlsRefusedMarker {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("the server declined TLS on a stream that requires it")
    }
}

impl Error for TlsRefusedMarker {}

/// What an observer does with the bytes it sees.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ObserverMode {
    /// The next byte read answers the `SSLRequest`: `S` turns `Passthrough`, anything else
    /// `Framed` (as `tokio-postgres` reads it).
    AwaitSslResponse,
    /// Like `AwaitSslResponse`, but only `S` is accepted: anything else fails the read (a
    /// cancel request of a session that ran over TLS never goes out in plaintext).
    AwaitSslAccept,
    /// Backend messages are framed and observed.
    Framed,
    /// TLS records: nothing is observed.
    Passthrough,
}

/// The `CommandComplete` tag buffer.
#[derive(Debug, Clone, Copy)]
struct Tag {
    bytes: [u8; TAG_CAP],
    len: usize,
}

/// What the observers of one connection share (Q11, Q12).
pub struct WireState {
    policy: InsecureAuth,
    /// The TLS outcome of this connection's handshake, captured when it finished.
    outcome: Mutex<Option<TlsVerified>>,
    auth_ok: AtomicBool,
    /// The backend PID; 0 until `BackendKeyData` (PostgreSQL never uses PID 0).
    pid: AtomicI32,
    /// The last `ReadyForQuery` status byte.
    status: AtomicU8,
    /// `ReadyForQuery` messages read since `AuthenticationOk`.
    ready: AtomicU64,
    /// `ReadyForQuery` messages owed: 1 for the startup, plus one per `Query`, `Sync` or
    /// `FunctionCall` written.
    expected: AtomicU64,
    closed: AtomicBool,
    notify: Notify,
    tag: Mutex<Option<Tag>>,
}

impl fmt::Debug for WireState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WireState")
            .field("policy", &self.policy)
            .field("auth_ok", &self.auth_ok.load(Ordering::SeqCst))
            .field("ready", &self.ready.load(Ordering::SeqCst))
            .field("expected", &self.expected.load(Ordering::SeqCst))
            .finish_non_exhaustive()
    }
}

impl WireState {
    pub fn new(policy: InsecureAuth) -> Arc<WireState> {
        Arc::new(WireState {
            policy,
            outcome: Mutex::new(None),
            auth_ok: AtomicBool::new(false),
            pid: AtomicI32::new(0),
            status: AtomicU8::new(b'I'),
            ready: AtomicU64::new(0),
            expected: AtomicU64::new(0),
            closed: AtomicBool::new(false),
            notify: Notify::new(),
            tag: Mutex::new(None),
        })
    }

    /// Records the outcome of the TLS handshake this connection runs over.
    pub fn set_outcome(&self, outcome: Option<TlsVerified>) {
        *self.outcome.lock().unwrap_or_else(PoisonError::into_inner) = outcome;
    }

    /// The TLS outcome captured after the handshake (`None` without TLS).
    pub fn outcome(&self) -> Option<TlsVerified> {
        *self.outcome.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Whether a password may go out by cleartext or MD5 (B8, Q20): outcome-based, never
    /// mode-based.
    pub fn allows_insecure_auth(&self) -> bool {
        self.policy == InsecureAuth::Allow
            || matches!(
                self.outcome(),
                Some(TlsVerified::Full | TlsVerified::CaOnly | TlsVerified::Pinned)
            )
    }

    /// The transaction status of the last `ReadyForQuery`.
    pub fn tx(&self) -> TxStatus {
        match self.status.load(Ordering::SeqCst) {
            b'T' => TxStatus::InTx,
            b'E' => TxStatus::Failed,
            _ => TxStatus::Idle,
        }
    }

    /// The backend PID from `BackendKeyData` (Q11).
    pub fn backend_pid(&self) -> Option<i32> {
        Some(self.pid.load(Ordering::SeqCst)).filter(|p| *p != 0)
    }

    /// `ReadyForQuery` messages read since `AuthenticationOk`.
    pub fn ready_count(&self) -> u64 {
        self.ready.load(Ordering::SeqCst)
    }

    /// Whether every `Query`, `Sync` and `FunctionCall` written has had its `ReadyForQuery`
    /// (or the connection ended).
    pub fn synced(&self) -> bool {
        self.closed.load(Ordering::SeqCst)
            || self.ready.load(Ordering::SeqCst) >= self.expected.load(Ordering::SeqCst)
    }

    /// The text of the last `CommandComplete` since [`WireState::clear_tag`].
    pub fn last_tag(&self) -> Option<String> {
        let tag = *self.tag.lock().unwrap_or_else(PoisonError::into_inner);
        tag.map(|t| String::from_utf8_lossy(&t.bytes[..t.len]).into_owned())
    }

    pub fn clear_tag(&self) {
        *self.tag.lock().unwrap_or_else(PoisonError::into_inner) = None;
    }

    /// The connection ended: nobody waits for a `ReadyForQuery` any more.
    pub fn close(&self) {
        self.closed.store(true, Ordering::SeqCst);
        self.notify.notify_waiters();
    }

    /// Waits until [`WireState::synced`], at most `limit`.
    pub async fn settle(&self, limit: Duration) {
        let wait = async {
            loop {
                let mut notified = std::pin::pin!(self.notify.notified());
                notified.as_mut().enable();
                if self.synced() {
                    return;
                }
                notified.await;
            }
        };
        let _ = tokio::time::timeout(limit, wait).await;
    }

    fn on_auth_ok(&self) {
        if !self.auth_ok.swap(true, Ordering::SeqCst) {
            // The startup's own ReadyForQuery.
            self.expected.store(1, Ordering::SeqCst);
        }
    }

    fn on_ready(&self, status: u8) -> io::Result<()> {
        if !matches!(status, b'I' | b'T' | b'E') {
            return Err(invalid("unknown transaction status"));
        }
        if self.auth_ok.load(Ordering::SeqCst) {
            self.status.store(status, Ordering::SeqCst);
            self.ready.fetch_add(1, Ordering::SeqCst);
            self.notify.notify_waiters();
        }
        Ok(())
    }

    fn on_key_data(&self, pid: i32) {
        if self.auth_ok.load(Ordering::SeqCst) && self.ready.load(Ordering::SeqCst) == 0 {
            self.pid.store(pid, Ordering::SeqCst);
        }
    }

    fn on_tag(&self, bytes: &[u8]) {
        let body = bytes.split(|b| *b == 0).next().unwrap_or_default();
        let mut tag = Tag {
            bytes: [0; TAG_CAP],
            len: body.len().min(TAG_CAP),
        };
        tag.bytes[..tag.len].copy_from_slice(&body[..tag.len]);
        *self.tag.lock().unwrap_or_else(PoisonError::into_inner) = Some(tag);
    }
}

fn invalid(what: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what)
}

/// How many leading body bytes a backend message type needs observed.
fn wanted(typ: u8) -> usize {
    match typ {
        b'R' | b'K' => 4,
        b'Z' => 1,
        b'C' => TAG_CAP,
        _ => 0,
    }
}

/// The backend message framer: a 5-byte header, then the body skipped by count with its first
/// bytes copied.
#[derive(Debug, Clone, Copy)]
struct Reader {
    header: [u8; 5],
    header_len: usize,
    /// Body bytes still to come.
    left: u32,
    capture: [u8; TAG_CAP],
    capture_len: usize,
    capture_want: usize,
    acted: bool,
}

impl Reader {
    fn new() -> Reader {
        Reader {
            header: [0; 5],
            header_len: 0,
            left: 0,
            capture: [0; TAG_CAP],
            capture_len: 0,
            capture_want: 0,
            acted: false,
        }
    }

    fn typ(&self) -> u8 {
        self.header[0]
    }

    fn feed(&mut self, mut data: &[u8], state: &WireState) -> io::Result<()> {
        while !data.is_empty() {
            if self.header_len < 5 {
                let n = (5 - self.header_len).min(data.len());
                self.header[self.header_len..self.header_len + n].copy_from_slice(&data[..n]);
                self.header_len += n;
                data = &data[n..];
                if self.header_len == 5 {
                    let len = u32::from_be_bytes([
                        self.header[1],
                        self.header[2],
                        self.header[3],
                        self.header[4],
                    ]);
                    if !(4..=MAX_LEN).contains(&len) {
                        return Err(invalid("invalid message length"));
                    }
                    self.left = len - 4;
                    self.capture_len = 0;
                    self.capture_want = wanted(self.typ()).min(self.left as usize);
                    self.acted = false;
                    self.after_bytes(state)?;
                }
                continue;
            }
            let n = (self.left as usize).min(data.len());
            let copy = (self.capture_want - self.capture_len).min(n);
            self.capture[self.capture_len..self.capture_len + copy].copy_from_slice(&data[..copy]);
            self.capture_len += copy;
            self.left -= n as u32;
            data = &data[n..];
            self.after_bytes(state)?;
        }
        Ok(())
    }

    /// Acts once the captured prefix is complete (`R`, `K`, `Z`) or the message ended (`C`),
    /// then starts the next header when the body is done.
    fn after_bytes(&mut self, state: &WireState) -> io::Result<()> {
        let typ = self.typ();
        let captured = self.capture_len == self.capture_want;
        let ready = if typ == b'C' {
            self.left == 0
        } else {
            captured
        };
        if ready && !self.acted {
            self.acted = true;
            self.act(typ, state)?;
        }
        if self.left == 0 {
            self.header_len = 0;
        }
        Ok(())
    }

    fn act(&self, typ: u8, state: &WireState) -> io::Result<()> {
        let prefix = &self.capture[..self.capture_len];
        let int = || {
            prefix
                .get(..4)
                .map(|b| i32::from_be_bytes([b[0], b[1], b[2], b[3]]))
        };
        match typ {
            b'R' => match int() {
                Some(0) => state.on_auth_ok(),
                Some(3 | 5) if !state.allows_insecure_auth() => {
                    return Err(io::Error::new(
                        io::ErrorKind::PermissionDenied,
                        InsecureAuthRefusedMarker,
                    ));
                }
                _ => {}
            },
            b'K' => {
                if let Some(pid) = int() {
                    state.on_key_data(pid);
                }
            }
            b'Z' => match prefix.first() {
                Some(status) => state.on_ready(*status)?,
                None => return Err(invalid("ReadyForQuery without a status")),
            },
            b'C' => state.on_tag(prefix),
            _ => {}
        }
        Ok(())
    }
}

/// The frontend message framer (after `AuthenticationOk` only: the startup messages carry no
/// type byte). It counts the messages that each draw one `ReadyForQuery`.
#[derive(Debug, Clone, Copy)]
struct Writer {
    active: bool,
    header: [u8; 5],
    header_len: usize,
    left: u32,
}

impl Writer {
    fn feed(&mut self, mut data: &[u8], state: &WireState) {
        if !self.active {
            // Nothing is written between the server's AuthenticationOk and the first request,
            // so the first write after it starts on a message boundary.
            if !state.auth_ok.load(Ordering::SeqCst) {
                return;
            }
            self.active = true;
        }
        while !data.is_empty() {
            if self.header_len < 5 {
                if self.header_len == 0 && matches!(data[0], b'Q' | b'S' | b'F') {
                    state.expected.fetch_add(1, Ordering::SeqCst);
                }
                let n = (5 - self.header_len).min(data.len());
                self.header[self.header_len..self.header_len + n].copy_from_slice(&data[..n]);
                self.header_len += n;
                data = &data[n..];
                if self.header_len == 5 {
                    let len = u32::from_be_bytes([
                        self.header[1],
                        self.header[2],
                        self.header[3],
                        self.header[4],
                    ]);
                    self.left = len.saturating_sub(4);
                    if self.left == 0 {
                        self.header_len = 0;
                    }
                }
                continue;
            }
            let n = (self.left as usize).min(data.len());
            self.left -= n as u32;
            data = &data[n..];
            if self.left == 0 {
                self.header_len = 0;
            }
        }
    }
}

/// A stream wrapper that observes the PostgreSQL protocol as it passes (see the module docs).
pub struct Observer<S> {
    inner: S,
    state: Arc<WireState>,
    mode: ObserverMode,
    reader: Reader,
    writer: Writer,
}

impl<S> fmt::Debug for Observer<S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Observer")
            .field("mode", &self.mode)
            .finish_non_exhaustive()
    }
}

impl<S> Observer<S> {
    pub fn new(inner: S, state: Arc<WireState>, mode: ObserverMode) -> Observer<S> {
        Observer {
            inner,
            state,
            mode,
            reader: Reader::new(),
            writer: Writer {
                active: false,
                header: [0; 5],
                header_len: 0,
                left: 0,
            },
        }
    }

    pub fn state(&self) -> &Arc<WireState> {
        &self.state
    }

    pub fn mode(&self) -> ObserverMode {
        self.mode
    }

    pub fn get_ref(&self) -> &S {
        &self.inner
    }

    pub fn get_mut(&mut self) -> &mut S {
        &mut self.inner
    }

    /// Observes bytes just read.
    fn observe_read(&mut self, data: &[u8]) -> io::Result<()> {
        let mut data = data;
        if matches!(
            self.mode,
            ObserverMode::AwaitSslResponse | ObserverMode::AwaitSslAccept
        ) {
            let Some((&answer, rest)) = data.split_first() else {
                return Ok(());
            };
            if answer == b'S' {
                // Everything after `S` is TLS: never framed here (Q12).
                self.mode = ObserverMode::Passthrough;
                return Ok(());
            }
            if self.mode == ObserverMode::AwaitSslAccept {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    TlsRefusedMarker,
                ));
            }
            self.mode = ObserverMode::Framed;
            data = rest;
        }
        match self.mode {
            ObserverMode::Framed => self.reader.feed(data, &self.state),
            _ => Ok(()),
        }
    }
}

impl<S: AsyncRead + Unpin> AsyncRead for Observer<S> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let before = buf.filled().len();
        let this = &mut *self;
        match Pin::new(&mut this.inner).poll_read(cx, buf) {
            Poll::Ready(Ok(())) => {}
            other => return other,
        }
        if let Err(e) = this.observe_read(&buf.filled()[before..]) {
            // The refused bytes never reach the reader above.
            buf.set_filled(before);
            return Poll::Ready(Err(e));
        }
        Poll::Ready(Ok(()))
    }
}

impl<S: AsyncWrite + Unpin> AsyncWrite for Observer<S> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        data: &[u8],
    ) -> Poll<io::Result<usize>> {
        let this = &mut *self;
        let r = Pin::new(&mut this.inner).poll_write(cx, data);
        if let Poll::Ready(Ok(n)) = &r
            && this.mode == ObserverMode::Framed
        {
            this.writer.feed(&data[..*n], &this.state);
        }
        r
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}
