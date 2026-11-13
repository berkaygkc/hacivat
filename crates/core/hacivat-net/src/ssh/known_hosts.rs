//! Known host keys (HV-SEC-01, B11, Task 4): Hacivat's own `known_hosts` file, read and
//! written, and the user's OpenSSH files, only read. Loading does no I/O (Q42): the files are
//! read lazily at the first check or trust, and every file is re-read when its `(mtime, len)`
//! changed (Q24), so `ssh-keygen -R` takes effect on the next retry.
//!
//! The format is OpenSSH's: `[marker] hosts keytype base64 [comment]`, where `hosts` is a comma
//! list of patterns (`*`, `?`, `!`) or one hashed name (`|1|salt|hash`, HMAC-SHA1). OpenSSH
//! stores port 22 as the bare host and every other port as `[host]:port` ([`host_pattern`]).
//! Lines are bytes: matching compares raw bytes with the host ASCII-folded (INV-3); nothing here
//! is decoded as text except the key's base64. Comments are dropped, never surfaced.

use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::SystemTime;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use hmac::{Hmac, Mac};
use sha1::Sha1;
use sha2::{Digest, Sha256};

use super::{HostKey, HostKeyCheck, KnownSource};

/// The longest line read (`known_hosts` and `~/.ssh/config`); longer lines are skipped.
pub(super) const MAX_LINE: usize = 16 * 1024;
/// The most bytes read from one `known_hosts` file.
const MAX_KNOWN_HOSTS_BYTES: u64 = 64 << 20;

/// The name OpenSSH stores and looks up for `host:port`: the bare host (ASCII lower-case) on
/// port 22, `[host]:port` otherwise. `ssh-keygen -R` takes exactly this name; quoting it for a
/// shell is the caller's job.
pub fn host_pattern(host: &str, port: u16) -> String {
    let host = host.to_ascii_lowercase();
    if port == 22 {
        host
    } else {
        format!("[{host}]:{port}")
    }
}

impl HostKey {
    /// Parses `<keytype> <base64> [comment]` (one `known_hosts` key). The blob must decode and
    /// name the same key type; the result is canonical (`algorithm` is the key type, `openssh`
    /// has no comment).
    pub fn from_openssh(line: &str) -> Option<HostKey> {
        let mut fields = line.split_ascii_whitespace();
        let key = ParsedKey::parse(fields.next()?.as_bytes(), fields.next()?.as_bytes())?;
        Some(key.host_key())
    }
}

/// The known-host stores.
pub struct KnownHosts {
    openssh_files: Vec<PathBuf>,
    hacivat_file: PathBuf,
    /// The cached files. One mutex orders every check and every write (Q24).
    state: Mutex<State>,
}

impl fmt::Debug for KnownHosts {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("KnownHosts")
            .field("openssh_files", &self.openssh_files)
            .field("hacivat_file", &self.hacivat_file)
            .finish_non_exhaustive()
    }
}

impl KnownHosts {
    /// The stores: `openssh_files` (read only) and `hacivat_file` (Hacivat's own). No I/O.
    pub fn load(openssh_files: Vec<PathBuf>, hacivat_file: PathBuf) -> Arc<KnownHosts> {
        let state = State {
            hacivat: Cached::default(),
            openssh: openssh_files.iter().map(|_| Cached::default()).collect(),
        };
        Arc::new(KnownHosts {
            openssh_files,
            hacivat_file,
            state: Mutex::new(state),
        })
    }

    /// What the stores say about `key` for `host:port`. Blocking (one `stat` per file, a read
    /// when one changed): call it on the core runtime.
    ///
    /// The order is OpenSSH's: a matching `@revoked` line wins; then the key listed for the host
    /// anywhere makes it known; then a listed key of the same type makes it changed (Hacivat's
    /// store first, so the review can offer to replace it). Anything else is unknown, including a
    /// host that only has keys of other types and a host covered only by `@cert-authority`.
    pub fn check(&self, host: &str, port: u16, key: &HostKey) -> HostKeyCheck {
        let mut state = self.lock();
        self.refresh(&mut state);
        let name = host_pattern(host, port);
        self.check_in(&state, name.as_bytes(), key)
    }

    /// Trusts `key` for `host:port` in Hacivat's store (created `0600`, in a `0700` directory).
    /// Blocking. Refuses a host name outside `[A-Za-z0-9.-_:%]`, a key that is not one
    /// consistent OpenSSH key, a key revoked for the host (`PermissionDenied`) and a key that
    /// would change a known one (`AlreadyExists`: a changed key is a hard stop that only
    /// [`KnownHosts::replace`] resolves, so no policy can trust it, B11).
    pub fn trust(&self, host: &str, port: u16, key: &HostKey) -> io::Result<()> {
        let name = store_name(host, port)?;
        let key = store_key(key)?;
        let mut state = self.lock();
        self.refuse(&mut state, &name, &key, true)?;
        let mut body = read_store(&self.hacivat_file)?.unwrap_or_default();
        if !lines(&body).any(|(no, line)| own_entry(no, line, &name, key.sha256)) {
            push_entry(&mut body, &name, &key);
            write_atomic(&self.hacivat_file, &body)?;
        }
        state.hacivat = Cached::default();
        Ok(())
    }

    /// Replaces `old` with `new` for `host:port` in Hacivat's store: the lines that name exactly
    /// this host (plain or hashed) with `old` go, `new` is appended. Blocking. `NotFound` (and no
    /// write) when the store has no such line; the same refusals as [`KnownHosts::trust`].
    pub fn replace(&self, host: &str, port: u16, old: &HostKey, new: &HostKey) -> io::Result<()> {
        let name = store_name(host, port)?;
        let new = store_key(new)?;
        let mut state = self.lock();
        self.refuse(&mut state, &name, &new, false)?;
        let Some(body) = read_store(&self.hacivat_file)? else {
            return Err(not_found());
        };
        let mut kept = Vec::with_capacity(body.len());
        let mut removed = false;
        for (no, line) in lines(&body) {
            if own_entry(no, line, &name, old.sha256) {
                removed = true;
            } else {
                kept.extend_from_slice(line);
                kept.push(b'\n');
            }
        }
        if !removed {
            return Err(not_found());
        }
        if !lines(&kept).any(|(no, line)| own_entry(no, line, &name, new.sha256)) {
            push_entry(&mut kept, &name, &new);
        }
        write_atomic(&self.hacivat_file, &kept)?;
        state.hacivat = Cached::default();
        Ok(())
    }

    /// Hacivat's own `known_hosts` file.
    pub fn hacivat_path(&self) -> &Path {
        &self.hacivat_file
    }

    /// The OpenSSH files the stores read (never write).
    pub fn openssh_files(&self) -> &[PathBuf] {
        &self.openssh_files
    }

    /// Lines over 16 KiB skipped in the files as last read. `hacivat-net` has no logger; the
    /// caller logs this count.
    pub fn skipped_lines(&self) -> usize {
        let state = self.lock();
        state.hacivat.skipped + state.openssh.iter().map(|c| c.skipped).sum::<usize>()
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Re-reads every file whose `(mtime, len)` changed: one `stat` per file.
    fn refresh(&self, state: &mut State) {
        state.hacivat.refresh(&self.hacivat_file);
        for (cached, path) in state.openssh.iter_mut().zip(&self.openssh_files) {
            cached.refresh(path);
        }
    }

    /// The stores in check order, with their sources.
    fn sources<'a>(&'a self, state: &'a State) -> impl Iterator<Item = (&'a Cached, Source<'a>)> {
        std::iter::once((&state.hacivat, Source::Hacivat)).chain(
            state
                .openssh
                .iter()
                .zip(&self.openssh_files)
                .map(|(c, p)| (c, Source::OpenSsh(p))),
        )
    }

    fn check_in(&self, state: &State, name: &[u8], key: &HostKey) -> HostKeyCheck {
        let presented_kind = key_kind(&key.openssh);
        let mut known = None;
        let mut changed = None;
        for (cached, source) in self.sources(state) {
            for entry in cached.entries.iter().filter(|e| e.hosts.matches(name)) {
                match entry.marker {
                    Marker::Revoked if entry.key.sha256 == key.sha256 => {
                        return HostKeyCheck::Revoked {
                            source: source.at(entry.line),
                        };
                    }
                    Marker::Plain if entry.key.sha256 == key.sha256 => {
                        known.get_or_insert_with(|| source.at(entry.line));
                    }
                    Marker::Plain
                        if presented_kind
                            .as_ref()
                            .is_none_or(|kind| *kind == entry.key.kind) =>
                    {
                        changed
                            .get_or_insert_with(|| (entry.key.host_key(), source.at(entry.line)));
                    }
                    // A key of another type, a revocation of another key, a CA line.
                    _ => {}
                }
            }
        }
        match (known, changed) {
            (Some(source), _) => HostKeyCheck::Known(source),
            (None, Some((old, source))) => HostKeyCheck::Changed { old, source },
            (None, None) => HostKeyCheck::Unknown,
        }
    }

    /// Refuses a key revoked for `name`, and with `changed` one that would change a known key.
    fn refuse(
        &self,
        state: &mut State,
        name: &str,
        key: &HostKey,
        changed: bool,
    ) -> io::Result<()> {
        self.refresh(state);
        match self.check_in(state, name.as_bytes(), key) {
            HostKeyCheck::Revoked { .. } => Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "known_hosts: the key is revoked for this host",
            )),
            HostKeyCheck::Changed { .. } if changed => Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "known_hosts: another key is known for this host",
            )),
            _ => Ok(()),
        }
    }
}

#[derive(Debug)]
struct State {
    hacivat: Cached,
    openssh: Vec<Cached>,
}

/// One file as last read.
#[derive(Debug, Default)]
struct Cached {
    /// `None`: never read, missing, or unreadable.
    stamp: Option<Stamp>,
    entries: Vec<Entry>,
    skipped: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Stamp {
    mtime: Option<SystemTime>,
    len: u64,
}

impl Cached {
    fn refresh(&mut self, path: &Path) {
        let stamp = match fs::metadata(path) {
            Ok(meta) if meta.is_file() => Stamp {
                mtime: meta.modified().ok(),
                len: meta.len(),
            },
            _ => {
                *self = Cached::default();
                return;
            }
        };
        if self.stamp == Some(stamp) {
            return;
        }
        let mut entries = Vec::new();
        match read_lines(path, MAX_KNOWN_HOSTS_BYTES, |no, line| {
            entries.extend(Entry::parse(no, line));
        }) {
            Ok(skipped) => {
                *self = Cached {
                    stamp: Some(stamp),
                    entries,
                    skipped,
                };
            }
            // Unreadable: nothing is known from it; the next check tries again.
            Err(_) => *self = Cached::default(),
        }
    }
}

#[derive(Clone, Copy)]
enum Source<'a> {
    Hacivat,
    OpenSsh(&'a Path),
}

impl Source<'_> {
    fn at(self, line: usize) -> KnownSource {
        match self {
            Source::Hacivat => KnownSource::Hacivat,
            Source::OpenSsh(path) => KnownSource::OpenSsh {
                path: path.to_path_buf(),
                line,
            },
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Marker {
    Plain,
    Revoked,
    CertAuthority,
}

/// One `known_hosts` line.
#[derive(Debug)]
struct Entry {
    line: usize,
    marker: Marker,
    hosts: Hosts,
    key: ParsedKey,
}

#[derive(Debug, PartialEq, Eq)]
enum Hosts {
    /// `|1|salt|hash`: HMAC-SHA1 of the name, keyed by the salt.
    Hashed { salt: Vec<u8>, hash: Vec<u8> },
    /// Comma-separated patterns, ASCII lower-cased.
    Patterns(Vec<Vec<u8>>),
}

impl Entry {
    fn parse(line: usize, bytes: &[u8]) -> Option<Entry> {
        let mut fields = bytes
            .split(|b| matches!(b, b' ' | b'\t'))
            .filter(|f| !f.is_empty());
        let mut first = fields.next()?;
        if first.starts_with(b"#") {
            return None;
        }
        let marker = if first.starts_with(b"@") {
            let marker = match first {
                b"@revoked" => Marker::Revoked,
                b"@cert-authority" => Marker::CertAuthority,
                _ => return None,
            };
            first = fields.next()?;
            marker
        } else {
            Marker::Plain
        };
        let hosts = Hosts::parse(first)?;
        let key = ParsedKey::parse(fields.next()?, fields.next()?)?;
        Some(Entry {
            line,
            marker,
            hosts,
            key,
        })
    }
}

impl Hosts {
    fn parse(field: &[u8]) -> Option<Hosts> {
        if let Some(rest) = field.strip_prefix(b"|1|") {
            let mut parts = rest.split(|b| *b == b'|');
            let salt = STANDARD.decode(parts.next()?).ok()?;
            let hash = STANDARD.decode(parts.next()?).ok()?;
            if parts.next().is_some() || hash.len() != 20 || salt.is_empty() {
                return None;
            }
            return Some(Hosts::Hashed { salt, hash });
        }
        if field.starts_with(b"|") {
            return None;
        }
        let patterns: Vec<Vec<u8>> = field
            .split(|b| *b == b',')
            .filter(|p| !p.is_empty())
            .map(<[u8]>::to_ascii_lowercase)
            .collect();
        (!patterns.is_empty()).then_some(Hosts::Patterns(patterns))
    }

    /// Whether `name` (already ASCII lower-cased) matches. A matching negated pattern excludes
    /// the line whatever else matches (OpenSSH's `match_hostname`).
    fn matches(&self, name: &[u8]) -> bool {
        match self {
            Hosts::Hashed { salt, hash } => {
                let Ok(mut mac) = Hmac::<Sha1>::new_from_slice(salt) else {
                    return false;
                };
                mac.update(name);
                mac.verify_slice(hash).is_ok()
            }
            Hosts::Patterns(patterns) => {
                let mut positive = false;
                for pattern in patterns {
                    match pattern.strip_prefix(b"!") {
                        Some(negated) if glob(negated, name) => return false,
                        Some(_) => {}
                        None => positive |= glob(pattern, name),
                    }
                }
                positive
            }
        }
    }

    /// Whether this is exactly `name` alone: what `trust` writes and `replace` removes.
    fn is_exactly(&self, name: &[u8]) -> bool {
        match self {
            Hosts::Hashed { .. } => self.matches(name),
            Hosts::Patterns(patterns) => patterns.len() == 1 && patterns[0] == name,
        }
    }
}

/// `*` (any run) and `?` (one byte) glob over bytes, without backtracking blow-up.
pub(super) fn glob(pattern: &[u8], text: &[u8]) -> bool {
    let (mut p, mut t) = (0, 0);
    let mut star: Option<(usize, usize)> = None;
    while t < text.len() {
        match pattern.get(p) {
            Some(b'*') => {
                star = Some((p, t));
                p += 1;
            }
            Some(&c) if c == b'?' || c == text[t] => {
                p += 1;
                t += 1;
            }
            _ => match star {
                Some((sp, st)) => {
                    p = sp + 1;
                    t = st + 1;
                    star = Some((sp, st + 1));
                }
                None => return false,
            },
        }
    }
    pattern[p.min(pattern.len())..].iter().all(|c| *c == b'*')
}

/// A decoded public key: its type (from the blob), the SHA-256 of the blob and the canonical
/// base64.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ParsedKey {
    kind: Vec<u8>,
    sha256: [u8; 32],
    base64: String,
}

impl ParsedKey {
    fn parse(keytype: &[u8], base64: &[u8]) -> Option<ParsedKey> {
        let token = |b: &u8| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'@' | b'_');
        if keytype.is_empty() || !keytype.iter().all(token) {
            return None;
        }
        let blob = STANDARD.decode(base64).ok()?;
        let kind = blob_kind(&blob)?;
        if kind != keytype {
            return None;
        }
        Some(ParsedKey {
            kind: kind.to_vec(),
            sha256: Sha256::digest(&blob).into(),
            base64: STANDARD.encode(&blob),
        })
    }

    fn host_key(&self) -> HostKey {
        // `kind` passed the token check in `parse`: ASCII.
        let algorithm = String::from_utf8_lossy(&self.kind).into_owned();
        HostKey {
            openssh: format!("{algorithm} {}", self.base64),
            algorithm,
            sha256: self.sha256,
        }
    }
}

/// The key type named inside an SSH public key blob (its first `string`).
fn blob_kind(blob: &[u8]) -> Option<&[u8]> {
    let len = u32::from_be_bytes(blob.get(..4)?.try_into().ok()?) as usize;
    blob.get(4..4usize.checked_add(len)?)
        .filter(|kind| !kind.is_empty())
}

/// The type of a presented key, from its OpenSSH form; `None` when it does not decode (then any
/// other listed key counts as a change: fail closed).
fn key_kind(openssh: &str) -> Option<Vec<u8>> {
    let mut fields = openssh.split_ascii_whitespace();
    let _ = fields.next()?;
    let blob = STANDARD.decode(fields.next()?).ok()?;
    blob_kind(&blob).map(<[u8]>::to_vec)
}

/// The name `trust`/`replace` write: [`host_pattern`] of a host made only of
/// `[A-Za-z0-9.-_:%]` (DNS names, IPv4, IPv6 with a scope id). Anything else could inject a
/// line, a pattern or a marker into the store.
fn store_name(host: &str, port: u16) -> io::Result<String> {
    let allowed =
        |b: u8| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_' | b':' | b'%');
    if host.is_empty() || !host.bytes().all(allowed) || port == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "known_hosts: not a host name",
        ));
    }
    Ok(host_pattern(host, port))
}

/// The canonical form of `key`, refused unless `openssh` is exactly one key whose blob hashes
/// to `sha256`.
fn store_key(key: &HostKey) -> io::Result<HostKey> {
    let invalid = || io::Error::new(io::ErrorKind::InvalidInput, "known_hosts: not one key");
    let mut fields = key.openssh.split(' ');
    let (Some(keytype), Some(base64), None) = (fields.next(), fields.next(), fields.next()) else {
        return Err(invalid());
    };
    let parsed = ParsedKey::parse(keytype.as_bytes(), base64.as_bytes()).ok_or_else(invalid)?;
    if parsed.sha256 != key.sha256 {
        return Err(invalid());
    }
    Ok(parsed.host_key())
}

/// Whether `line` is a plain entry for exactly `name` with the key hashing to `sha256`.
fn own_entry(no: usize, line: &[u8], name: &str, sha256: [u8; 32]) -> bool {
    line.len() <= MAX_LINE
        && Entry::parse(no, strip_cr(line)).is_some_and(|e| {
            e.marker == Marker::Plain
                && e.key.sha256 == sha256
                && e.hosts.is_exactly(name.as_bytes())
        })
}

fn push_entry(body: &mut Vec<u8>, name: &str, key: &HostKey) {
    if !body.is_empty() && !body.ends_with(b"\n") {
        body.push(b'\n');
    }
    body.extend_from_slice(format!("{name} {}\n", key.openssh).as_bytes());
}

/// The lines of `body` (numbered from 1), without their `\n`.
fn lines(body: &[u8]) -> impl Iterator<Item = (usize, &[u8])> {
    body.split(|b| *b == b'\n')
        .enumerate()
        .map(|(i, line)| (i + 1, line))
        .filter(|(_, line)| !line.is_empty())
}

/// Hacivat's store, whole; `None` when it does not exist.
fn read_store(path: &Path) -> io::Result<Option<Vec<u8>>> {
    match fs::metadata(path) {
        Ok(meta) if meta.len() > MAX_KNOWN_HOSTS_BYTES => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "known_hosts: the store is too large",
        )),
        Ok(_) => fs::read(path).map(Some),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

/// Writes `body` to `path` atomically: a `0600` temp file next to it, `fsync`, rename. The
/// directory is created `0700` when missing.
fn write_atomic(path: &Path, body: &[u8]) -> io::Result<()> {
    let dir = path
        .parent()
        .filter(|d| !d.as_os_str().is_empty())
        .ok_or_else(not_found)?;
    if !dir.exists() {
        let mut builder = fs::DirBuilder::new();
        builder.recursive(true);
        #[cfg(unix)]
        std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
        builder.create(dir)?;
    }
    let mut name = path.file_name().ok_or_else(not_found)?.to_os_string();
    name.push(format!(".tmp{}", std::process::id()));
    let tmp = dir.join(name);
    let _ = fs::remove_file(&tmp);
    let result = (|| {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
        let mut file = options.open(&tmp)?;
        file.write_all(body)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result?;
    #[cfg(unix)]
    if let Ok(dir) = File::open(dir) {
        let _ = dir.sync_all();
    }
    Ok(())
}

/// Streams the lines of the regular file at `path` (at most `max_bytes`), numbered from 1,
/// without `\n` or a trailing `\r`. A line over [`MAX_LINE`] is never buffered: it is skipped
/// and counted. Never opens anything but a regular file (no FIFO or device can block it).
pub(super) fn read_lines(
    path: &Path,
    max_bytes: u64,
    mut each: impl FnMut(usize, &[u8]),
) -> io::Result<usize> {
    if !fs::metadata(path)?.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "not a regular file",
        ));
    }
    let mut reader = BufReader::new(File::open(path)?.take(max_bytes));
    let mut line = Vec::new();
    let (mut no, mut skipped, mut too_long) = (0, 0, false);
    loop {
        let buf = reader.fill_buf()?;
        if buf.is_empty() {
            break;
        }
        let end = buf.iter().position(|b| *b == b'\n');
        let chunk = &buf[..end.unwrap_or(buf.len())];
        if !too_long {
            if line.len() + chunk.len() > MAX_LINE + 1 {
                too_long = true;
                line.clear();
            } else {
                line.extend_from_slice(chunk);
            }
        }
        let used = end.map_or(chunk.len(), |e| e + 1);
        reader.consume(used);
        if end.is_some() {
            no += 1;
            finish_line(no, &mut line, &mut too_long, &mut skipped, &mut each);
        }
    }
    if !line.is_empty() || too_long {
        no += 1;
        finish_line(no, &mut line, &mut too_long, &mut skipped, &mut each);
    }
    Ok(skipped)
}

fn finish_line(
    no: usize,
    line: &mut Vec<u8>,
    too_long: &mut bool,
    skipped: &mut usize,
    each: &mut impl FnMut(usize, &[u8]),
) {
    let text = strip_cr(line);
    if *too_long || text.len() > MAX_LINE {
        *skipped += 1;
    } else {
        each(no, text);
    }
    line.clear();
    *too_long = false;
}

fn strip_cr(line: &[u8]) -> &[u8] {
    line.strip_suffix(b"\r").unwrap_or(line)
}

fn not_found() -> io::Error {
    io::Error::new(
        io::ErrorKind::NotFound,
        "known_hosts: no such entry in Hacivat's store",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn glob_matches_like_openssh() {
        assert!(glob(b"*", b""));
        assert!(glob(b"*.acme.io", b"db.acme.io"));
        assert!(!glob(b"*.acme.io", b"acme.io"));
        assert!(glob(b"db?", b"db1"));
        assert!(!glob(b"db?", b"db"));
        assert!(glob(b"a*b*c", b"aXXbYYc"));
        assert!(!glob(b"a*b*c", b"aXXbYY"));
        assert!(glob(b"[db]:*", b"[db]:2222"));
        // A pathological pattern stays linear-ish.
        let text = vec![b'a'; MAX_LINE];
        assert!(!glob(&[b"*a".repeat(64).as_slice(), b"b"].concat(), &text));
    }

    #[test]
    fn a_blob_kind_never_reads_past_the_blob() {
        assert_eq!(
            blob_kind(&[0, 0, 0, 3, b'a', b'b', b'c']),
            Some(&b"abc"[..])
        );
        assert_eq!(blob_kind(&[0, 0, 0, 9, b'a']), None);
        assert_eq!(blob_kind(&[255, 255, 255, 255]), None);
        assert_eq!(blob_kind(&[0, 0, 0, 0]), None);
        assert_eq!(blob_kind(&[0, 0]), None);
    }

    #[test]
    fn the_stores_load_without_io_and_know_nothing_about_missing_files() {
        let missing = PathBuf::from("/nonexistent/hacivat/known_hosts");
        let known = KnownHosts::load(vec![PathBuf::from("/nonexistent/.ssh")], missing.clone());
        assert_eq!(known.hacivat_path(), missing);
        assert_eq!(known.openssh_files(), [PathBuf::from("/nonexistent/.ssh")]);
        let key = HostKey {
            algorithm: "ssh-ed25519".into(),
            sha256: [1; 32],
            openssh: "ssh-ed25519 AAAA".into(),
        };
        assert_eq!(known.check("bastion", 22, &key), HostKeyCheck::Unknown);
        // Not one consistent key: refused before any write.
        assert!(known.trust("bastion", 22, &key).is_err());
        assert!(known.replace("bastion", 22, &key, &key).is_err());
    }
}
