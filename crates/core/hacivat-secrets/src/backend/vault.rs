//! The encrypted-file vault (R5, HV-SEC-05, D-016, B13, Q30): where no OS keyring is usable,
//! `Forever` secrets live in `vault.bin`, XChaCha20-Poly1305 under an Argon2id key derived from
//! a master password.
//!
//! # File format (B13)
//!
//! | Bytes | Field |
//! |---|---|
//! | 0–7 | `HVVAULT1` |
//! | 8–9 | `format_major`, u16 LE = 1 |
//! | 10–11 | `format_minor`, u16 LE = 0 |
//! | 12–15 | Argon2id `m_kib`, u32 LE |
//! | 16–19 | `t`, u32 LE |
//! | 20–23 | `p`, u32 LE |
//! | 24–39 | salt |
//! | 40–63 | nonce |
//!
//! The body is the ciphertext of a JSON object `{ "<service>": "<secret>" }` with its 16-byte
//! Poly1305 tag; the 64-byte header is the associated data, so no header byte can change
//! without the unlock failing.
//!
//! # Rules
//!
//! - The header is unauthenticated until the key is derived, so its parameters are bounded
//!   first (Q30): anything outside `m_kib` 8,192–1,048,576, `t` 1–10, `p` 1–8 is `VaultCorrupt`
//!   and no key is derived (a vault built `with_params` also accepts its own parameters).
//! - Salt and nonce come from `getrandom::fill`. Every write draws a fresh nonce and keeps the
//!   salt (and therefore the key).
//! - The key lives in a `Zeroizing` buffer, the decrypted map in `SecretString` values: the
//!   JSON is built and decrypted inside `Zeroizing<Vec<u8>>` buffers sized up front (never
//!   reallocated while they hold plaintext) and parsed straight into `SecretString`. Argon2's
//!   working memory is overwritten before it is freed.
//! - A write is atomic: `vault.bin.tmp` is created fresh (`create_new`, mode `0600`), written,
//!   `fsync`ed and renamed over `vault.bin`, then the directory is `fsync`ed (Unix). The temp
//!   file only ever holds ciphertext. A symlink or any non-file at `vault.bin` is refused, and
//!   a share or device path is refused before any file system call.
//! - A failed write changes nothing: the in-memory map is replaced only after the rename.
//! - Every method blocks (Argon2 takes about a second with the default parameters): callers run
//!   them on the core runtime's blocking pool.

use std::collections::BTreeMap;
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::ops::RangeInclusive;
use std::path::{Component, Path, PathBuf, Prefix};
use std::sync::{Mutex, MutexGuard, PoisonError};

use argon2::{Algorithm, Argon2, Block, Params, Version};
use chacha20poly1305::aead::{AeadInPlace, KeyInit};
use chacha20poly1305::{Key, XChaCha20Poly1305, XNonce};
use secrecy::{ExposeSecret, SecretString};
use zeroize::Zeroizing;

use super::{BackendChoice, SecretBackend, SecretError};
use crate::store::SecretKey;

/// A backend that must be unlocked with a master password first (R5, HV-SEC-05).
pub trait VaultBackend: SecretBackend {
    /// Whether the vault file exists.
    fn exists(&self) -> bool;
    fn is_unlocked(&self) -> bool;
    /// Creates a new, unlocked vault under `master`.
    fn create(&self, master: SecretString) -> Result<(), SecretError>;
    fn unlock(&self, master: SecretString) -> Result<(), SecretError>;
    /// Forgets the key and the decrypted map.
    fn lock(&self);
    /// Deletes the vault and everything in it.
    fn reset(&self) -> Result<(), SecretError>;
}

/// Argon2id parameters: memory in KiB, iterations, lanes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct VaultParams {
    pub m_kib: u32,
    pub t: u32,
    pub p: u32,
}

impl Default for VaultParams {
    /// 64 MiB, 3 iterations, 1 lane.
    fn default() -> Self {
        VaultParams {
            m_kib: 65536,
            t: 3,
            p: 1,
        }
    }
}

// ## m2b-t06-vault

const MAGIC: &[u8; 8] = b"HVVAULT1";
/// The format this build reads and writes. A newer major is opened read-only.
const FORMAT_MAJOR: u16 = 1;
const FORMAT_MINOR: u16 = 0;
const HEADER_LEN: usize = 64;
const SALT_LEN: usize = 16;
const NONCE_LEN: usize = 24;
const TAG_LEN: usize = 16;
const KEY_LEN: usize = 32;
/// The shortest master password, in characters (B13).
const MIN_MASTER_CHARS: usize = 8;
/// The largest vault file read (16 MiB); a larger one is corrupt, never loaded.
const MAX_VAULT_BYTES: u64 = 16 << 20;

/// The header bounds checked before any key is derived (Q30).
const M_KIB_BOUNDS: RangeInclusive<u32> = 8_192..=1_048_576;
const T_BOUNDS: RangeInclusive<u32> = 1..=10;
const P_BOUNDS: RangeInclusive<u32> = 1..=8;

/// The decoded 64-byte header.
struct Header {
    major: u16,
    minor: u16,
    params: VaultParams,
    salt: [u8; SALT_LEN],
    nonce: [u8; NONCE_LEN],
}

impl Header {
    fn encode(&self) -> [u8; HEADER_LEN] {
        let mut out = [0u8; HEADER_LEN];
        out[0..8].copy_from_slice(MAGIC);
        out[8..10].copy_from_slice(&self.major.to_le_bytes());
        out[10..12].copy_from_slice(&self.minor.to_le_bytes());
        out[12..16].copy_from_slice(&self.params.m_kib.to_le_bytes());
        out[16..20].copy_from_slice(&self.params.t.to_le_bytes());
        out[20..24].copy_from_slice(&self.params.p.to_le_bytes());
        out[24..40].copy_from_slice(&self.salt);
        out[40..64].copy_from_slice(&self.nonce);
        out
    }

    /// The header at the start of `file`, when the file is long enough to hold a header and a
    /// tag and starts with the magic.
    fn decode(file: &[u8]) -> Option<Header> {
        if file.len() < HEADER_LEN + TAG_LEN || &file[0..8] != MAGIC {
            return None;
        }
        let u16_at = |at: usize| u16::from_le_bytes([file[at], file[at + 1]]);
        let u32_at =
            |at: usize| u32::from_le_bytes([file[at], file[at + 1], file[at + 2], file[at + 3]]);
        let mut salt = [0u8; SALT_LEN];
        salt.copy_from_slice(&file[24..40]);
        let mut nonce = [0u8; NONCE_LEN];
        nonce.copy_from_slice(&file[40..64]);
        Some(Header {
            major: u16_at(8),
            minor: u16_at(10),
            params: VaultParams {
                m_kib: u32_at(12),
                t: u32_at(16),
                p: u32_at(20),
            },
            salt,
            nonce,
        })
    }
}

/// An unlocked vault: the key and the decrypted map. Deliberately without `Debug`.
struct Unlocked {
    key: Zeroizing<[u8; KEY_LEN]>,
    params: VaultParams,
    salt: [u8; SALT_LEN],
    /// Written by a newer Hacivat: readable, never written.
    read_only: bool,
    entries: BTreeMap<String, SecretString>,
}

/// The vault file (`LocalPaths::vault_bin`). Opening does no I/O. `Debug` shows the path, the
/// parameters and whether it is unlocked, never a key, a service name or a secret.
pub struct FileVault {
    path: PathBuf,
    params: VaultParams,
    state: Mutex<Option<Unlocked>>,
}

impl fmt::Debug for FileVault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FileVault")
            .field("path", &self.path)
            .field("params", &self.params)
            .field("unlocked", &self.is_unlocked())
            .finish()
    }
}

impl FileVault {
    /// The vault at `path` with the default parameters. Its header must lie within the Q30
    /// bounds.
    pub fn open(path: PathBuf) -> FileVault {
        Self::with_params(path, VaultParams::default())
    }

    /// The vault at `path` with parameters `p` (tests use small ones). It creates vaults with
    /// `p` and also opens a header whose parameters equal `p`, inside the bounds or not.
    pub fn with_params(path: PathBuf, p: VaultParams) -> FileVault {
        FileVault {
            path,
            params: p,
            state: Mutex::new(None),
        }
    }

    fn state(&self) -> MutexGuard<'_, Option<Unlocked>> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Whether a header with `params` may be used to derive a key (Q30).
    fn accepts(&self, params: VaultParams) -> bool {
        params == self.params
            || (M_KIB_BOUNDS.contains(&params.m_kib)
                && T_BOUNDS.contains(&params.t)
                && P_BOUNDS.contains(&params.p))
    }

    /// Seals `entries` under `u`'s key with a fresh nonce and writes the file atomically. An
    /// I/O failure is `VaultReadOnly` (the file cannot be written).
    fn write(
        &self,
        u: &Unlocked,
        entries: &BTreeMap<String, SecretString>,
    ) -> Result<(), SecretError> {
        let file = seal(u, entries)?;
        write_atomic(&self.path, &file).map_err(|_| SecretError::VaultReadOnly)
    }

    /// Writes `entries` and, once they are on disk, makes them the vault's map.
    fn commit(
        &self,
        u: &mut Unlocked,
        entries: BTreeMap<String, SecretString>,
    ) -> Result<(), SecretError> {
        self.write(u, &entries)?;
        u.entries = entries;
        Ok(())
    }
}

impl SecretBackend for FileVault {
    /// `VaultLocked` while locked; with no vault file there is nothing stored (`None`).
    fn get(&self, key: &SecretKey) -> Result<Option<SecretString>, SecretError> {
        match self.state().as_ref() {
            Some(u) => Ok(u.entries.get(&key.service()).cloned()),
            None if !self.exists() => Ok(None),
            None => Err(SecretError::VaultLocked),
        }
    }

    /// `VaultLocked` while locked (or before the vault exists), `VaultReadOnly` for a newer
    /// format or a file that cannot be written.
    fn put(&self, key: &SecretKey, value: &SecretString) -> Result<(), SecretError> {
        let mut state = self.state();
        let u = state.as_mut().ok_or(SecretError::VaultLocked)?;
        if u.read_only {
            return Err(SecretError::VaultReadOnly);
        }
        let mut entries = u.entries.clone();
        entries.insert(key.service(), value.clone());
        self.commit(u, entries)
    }

    /// Deleting a missing entry, or from a vault that does not exist, succeeds.
    fn delete(&self, key: &SecretKey) -> Result<(), SecretError> {
        let mut state = self.state();
        let Some(u) = state.as_mut() else {
            return if self.exists() {
                Err(SecretError::VaultLocked)
            } else {
                Ok(())
            };
        };
        if u.read_only {
            return Err(SecretError::VaultReadOnly);
        }
        let service = key.service();
        if !u.entries.contains_key(&service) {
            return Ok(());
        }
        let mut entries = u.entries.clone();
        entries.remove(&service);
        self.commit(u, entries)
    }

    fn name(&self) -> &'static str {
        "vault"
    }

    fn as_vault(&self) -> Option<&dyn VaultBackend> {
        Some(self)
    }

    fn choice(&self) -> Option<BackendChoice> {
        Some(BackendChoice::Vault)
    }
}

impl VaultBackend for FileVault {
    /// Whether anything is at the vault path (a share or device path never exists).
    fn exists(&self) -> bool {
        !names_another_machine(&self.path) && fs::symlink_metadata(&self.path).is_ok()
    }

    fn is_unlocked(&self) -> bool {
        self.state().is_some()
    }

    /// Refuses a master password under 8 characters (`Backend("master-too-short")`; the dialog
    /// validates first), a share or device path (`Unavailable`) and an existing file
    /// (`Backend("vault-exists")`).
    fn create(&self, master: SecretString) -> Result<(), SecretError> {
        if master.expose_secret().chars().count() < MIN_MASTER_CHARS {
            return Err(SecretError::Backend("master-too-short".to_string()));
        }
        if names_another_machine(&self.path) {
            return Err(SecretError::Unavailable);
        }
        // Held across the check and the write, so two creates cannot both succeed.
        let mut state = self.state();
        if self.exists() {
            return Err(SecretError::Backend("vault-exists".to_string()));
        }
        let mut salt = [0u8; SALT_LEN];
        random(&mut salt)?;
        let key = derive_key(&master, &salt, self.params)
            .ok_or_else(|| SecretError::Backend("vault-params".to_string()))?;
        let u = Unlocked {
            key,
            params: self.params,
            salt,
            read_only: false,
            entries: BTreeMap::new(),
        };
        self.write(&u, &u.entries)?;
        *state = Some(u);
        Ok(())
    }

    /// A wrong password, or a body that fails the AEAD under a header that parses, is
    /// `WrongMasterPassword` (the AEAD cannot tell tampering from a wrong password, Q30); a
    /// header that does not parse, a truncated file, out-of-bounds parameters or a non-file at
    /// the path is `VaultCorrupt`. Nothing is written either way.
    fn unlock(&self, master: SecretString) -> Result<(), SecretError> {
        let file = read_vault(&self.path)?;
        let header = Header::decode(&file).ok_or(SecretError::VaultCorrupt)?;
        if header.major == 0 || !self.accepts(header.params) {
            return Err(SecretError::VaultCorrupt);
        }
        let key =
            derive_key(&master, &header.salt, header.params).ok_or(SecretError::VaultCorrupt)?;
        drop(master);
        let (aad, body) = file.split_at(HEADER_LEN);
        let mut plain = Zeroizing::new(Vec::with_capacity(body.len()));
        plain.extend_from_slice(body);
        XChaCha20Poly1305::new(Key::from_slice(&key[..]))
            .decrypt_in_place(XNonce::from_slice(&header.nonce), aad, &mut *plain)
            .map_err(|_| SecretError::WrongMasterPassword)?;
        let entries: BTreeMap<String, SecretString> =
            serde_json::from_slice(&plain).map_err(|_| SecretError::VaultCorrupt)?;
        *self.state() = Some(Unlocked {
            key,
            params: header.params,
            salt: header.salt,
            read_only: header.major > FORMAT_MAJOR,
            entries,
        });
        Ok(())
    }

    /// Drops the key and the map; both zeroize their memory.
    fn lock(&self) {
        *self.state() = None;
    }

    /// Locks, then deletes the vault file (and a stale temp file). A missing file is fine.
    fn reset(&self) -> Result<(), SecretError> {
        let mut state = self.state();
        *state = None;
        if names_another_machine(&self.path) {
            return Err(SecretError::Unavailable);
        }
        match fs::remove_file(&self.path) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(_) => return Err(SecretError::VaultReadOnly),
        }
        let _ = fs::remove_file(temp_path(&self.path));
        Ok(())
    }
}

/// Fills `buf` from the OS random source.
fn random(buf: &mut [u8]) -> Result<(), SecretError> {
    getrandom::fill(buf).map_err(|_| SecretError::Backend("random".to_string()))
}

/// The Argon2id key for `master` (`None` when Argon2 refuses the parameters). Argon2's working
/// memory is supplied here and overwritten before it is freed.
fn derive_key(
    master: &SecretString,
    salt: &[u8; SALT_LEN],
    params: VaultParams,
) -> Option<Zeroizing<[u8; KEY_LEN]>> {
    let params = Params::new(params.m_kib, params.t, params.p, Some(KEY_LEN)).ok()?;
    let mut blocks = vec![Block::new(); params.block_count()];
    let argon = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let mut key = Zeroizing::new([0u8; KEY_LEN]);
    let derived = argon.hash_password_into_with_memory(
        master.expose_secret().as_bytes(),
        salt,
        &mut key[..],
        &mut blocks,
    );
    // argon2 is built without its `zeroize` feature, so wipe the blocks here; `black_box`
    // keeps the stores from being optimised away before the free.
    blocks.fill(Block::new());
    std::hint::black_box(&blocks);
    derived.ok().map(|()| key)
}

/// The file bytes for `entries` under `u`'s key, salt and parameters, with a fresh nonce.
fn seal(u: &Unlocked, entries: &BTreeMap<String, SecretString>) -> Result<Vec<u8>, SecretError> {
    let mut buf = json_map(entries, TAG_LEN);
    let mut nonce = [0u8; NONCE_LEN];
    random(&mut nonce)?;
    let header = Header {
        major: FORMAT_MAJOR,
        minor: FORMAT_MINOR,
        params: u.params,
        salt: u.salt,
        nonce,
    }
    .encode();
    XChaCha20Poly1305::new(Key::from_slice(&u.key[..]))
        .encrypt_in_place(XNonce::from_slice(&nonce), &header, &mut *buf)
        .map_err(|_| SecretError::Backend("vault-seal".to_string()))?;
    let mut file = Vec::with_capacity(HEADER_LEN + buf.len());
    file.extend_from_slice(&header);
    file.extend_from_slice(&buf);
    Ok(file)
}

/// `entries` as a JSON object, in a buffer sized exactly up front (plus `spare` bytes for the
/// tag), so it is never reallocated, and so never copied, while it holds plaintext.
fn json_map(entries: &BTreeMap<String, SecretString>, spare: usize) -> Zeroizing<Vec<u8>> {
    let len = 2
        + entries
            .iter()
            .map(|(k, v)| json_str_len(k) + 1 + json_str_len(v.expose_secret()))
            .sum::<usize>()
        + entries.len().saturating_sub(1);
    let mut out = Zeroizing::new(Vec::with_capacity(len + spare));
    out.push(b'{');
    for (i, (k, v)) in entries.iter().enumerate() {
        if i > 0 {
            out.push(b',');
        }
        push_json_str(&mut out, k);
        out.push(b':');
        push_json_str(&mut out, v.expose_secret());
    }
    out.push(b'}');
    debug_assert_eq!(out.len(), len);
    out
}

const HEX_DIGITS: &[u8; 16] = b"0123456789abcdef";

/// The encoded length of `s` as a JSON string, quotes included.
fn json_str_len(s: &str) -> usize {
    2 + s
        .bytes()
        .map(|b| match b {
            b'"' | b'\\' => 2,
            b if b.is_ascii_control() => 6,
            _ => 1,
        })
        .sum::<usize>()
}

/// Appends `s` as a JSON string: `"` and `\` escaped, ASCII controls as `\u00XX`.
fn push_json_str(out: &mut Vec<u8>, s: &str) {
    out.push(b'"');
    for b in s.bytes() {
        match b {
            b'"' | b'\\' => out.extend_from_slice(&[b'\\', b]),
            b if b.is_ascii_control() => out.extend_from_slice(&[
                b'\\',
                b'u',
                b'0',
                b'0',
                HEX_DIGITS[usize::from(b >> 4)],
                HEX_DIGITS[usize::from(b & 15)],
            ]),
            _ => out.push(b),
        }
    }
    out.push(b'"');
}

/// The vault file's bytes (ciphertext only). A share or device path is `Unavailable` without
/// any file system call; a symlink, a non-file or an oversized file is `VaultCorrupt`; a
/// missing file is `Unavailable`.
fn read_vault(path: &Path) -> Result<Vec<u8>, SecretError> {
    if names_another_machine(path) {
        return Err(SecretError::Unavailable);
    }
    let io_error = |e: io::Error| match e.kind() {
        io::ErrorKind::NotFound => SecretError::Unavailable,
        io::ErrorKind::PermissionDenied => SecretError::Denied,
        _ => SecretError::Backend("vault-read".to_string()),
    };
    let link = fs::symlink_metadata(path).map_err(io_error)?;
    if !link.file_type().is_file() || link.len() > MAX_VAULT_BYTES {
        return Err(SecretError::VaultCorrupt);
    }
    let file = File::open(path).map_err(io_error)?;
    let meta = file.metadata().map_err(io_error)?;
    if !meta.is_file() || !same_file(&link, &meta) {
        // Swapped for something else between the check and the open.
        return Err(SecretError::VaultCorrupt);
    }
    let mut bytes = Vec::with_capacity(usize::try_from(meta.len()).unwrap_or(0));
    file.take(MAX_VAULT_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(io_error)?;
    if bytes.len() as u64 > MAX_VAULT_BYTES {
        return Err(SecretError::VaultCorrupt);
    }
    Ok(bytes)
}

#[cfg(unix)]
fn same_file(a: &fs::Metadata, b: &fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    a.dev() == b.dev() && a.ino() == b.ino()
}

#[cfg(not(unix))]
fn same_file(_a: &fs::Metadata, _b: &fs::Metadata) -> bool {
    true
}

/// `<path>.tmp`, next to the vault.
fn temp_path(path: &Path) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(".tmp");
    path.with_file_name(name)
}

/// Replaces the file at `path` with `bytes` atomically (B13): a fresh temp file (mode `0600`,
/// never through an existing entry), `fsync`, rename, then the directory's `fsync` on Unix. A
/// symlink or other non-file at `path` is refused, never followed or replaced.
fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    if names_another_machine(path) {
        return Err(io::Error::other("network path"));
    }
    match fs::symlink_metadata(path) {
        Ok(meta) if !meta.file_type().is_file() => return Err(io::Error::other("not a file")),
        Ok(_) => {}
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }
    let dir = match path.parent() {
        Some(dir) if !dir.as_os_str().is_empty() => dir.to_path_buf(),
        _ => PathBuf::from("."),
    };
    create_dir(&dir)?;
    let tmp = temp_path(path);
    // A stale temp file (or anything planted there) is removed, never written through.
    match fs::remove_file(&tmp) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let written = options.open(&tmp).and_then(|mut file| {
        file.write_all(bytes)?;
        file.sync_all()
    });
    if let Err(e) = written.and_then(|()| fs::rename(&tmp, path)) {
        let _ = fs::remove_file(&tmp);
        return Err(e);
    }
    // The rename is done; a failed directory sync only weakens durability, not the content.
    #[cfg(unix)]
    let _ = File::open(&dir).and_then(|d| d.sync_all());
    Ok(())
}

/// Creates `dir` (owner-only on Unix) when it is missing.
fn create_dir(dir: &Path) -> io::Result<()> {
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(dir)
}

/// `hacivat_net::is_network_path`'s rule, which this crate cannot depend on: whether `path`
/// names another machine or the NT object namespace (two leading separators, `\??\`, or a UNC,
/// device or `GLOBALROOT` prefix). Opening one on Windows can send the user's NTLM hash to a
/// host, so the vault refuses it before any file system call.
fn names_another_machine(path: &Path) -> bool {
    let text = path.to_string_lossy();
    let sep = |c: Option<char>| matches!(c, Some('/' | '\\'));
    let mut chars = text.chars();
    let (a, b, c, d) = (chars.next(), chars.next(), chars.next(), chars.next());
    if (sep(a) && sep(b)) || (sep(a) && b == Some('?') && c == Some('?') && sep(d)) {
        return true;
    }
    match path.components().next() {
        Some(Component::Prefix(p)) => match p.kind() {
            Prefix::DeviceNS(_) | Prefix::UNC(..) | Prefix::VerbatimUNC(..) => true,
            Prefix::Verbatim(name) => name.eq_ignore_ascii_case("GLOBALROOT"),
            Prefix::Disk(_) | Prefix::VerbatimDisk(_) => false,
        },
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(pairs: &[(&str, &str)]) -> BTreeMap<String, SecretString> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), SecretString::from(*v)))
            .collect()
    }

    /// The hand-written JSON is exactly what serde_json reads back, with no reallocation.
    #[test]
    fn json_map_round_trips_through_serde_json() {
        for entries in [
            map(&[]),
            map(&[("a", "b")]),
            map(&[
                ("hacivat-dev/x/db-password", "q\"\\\n\u{1}\u{7f}ğ😀"),
                ("b", ""),
            ]),
        ] {
            let json = json_map(&entries, TAG_LEN);
            assert_eq!(json.capacity(), json.len() + TAG_LEN);
            let back: BTreeMap<String, SecretString> = serde_json::from_slice(&json).unwrap();
            assert_eq!(back.len(), entries.len());
            for (k, v) in &entries {
                assert!(back[k].expose_secret() == v.expose_secret());
            }
        }
    }

    #[test]
    fn the_bounds_and_own_params() {
        let open = FileVault::open(PathBuf::from("v"));
        assert!(open.accepts(VaultParams::default()));
        assert!(open.accepts(VaultParams {
            m_kib: 8192,
            t: 1,
            p: 8
        }));
        assert!(!open.accepts(VaultParams {
            m_kib: 64,
            t: 1,
            p: 1
        }));
        let fast = VaultParams {
            m_kib: 64,
            t: 1,
            p: 1,
        };
        assert!(FileVault::with_params(PathBuf::from("v"), fast).accepts(fast));
    }

    #[test]
    fn network_paths_follow_the_net_rule() {
        for p in [
            r"\\h\s\x",
            "//h/s/x",
            r"\??\x",
            r"\\.\pipe\x",
            r"\\?\UNC\h\s\x",
        ] {
            assert!(names_another_machine(Path::new(p)), "{p}");
        }
        for p in ["/home/u/.local/share/hacivat/vault.bin", "vault.bin", "./v"] {
            assert!(!names_another_machine(Path::new(p)), "{p}");
        }
        assert_eq!(
            temp_path(Path::new("/d/vault.bin")),
            PathBuf::from("/d/vault.bin.tmp")
        );
    }
}
