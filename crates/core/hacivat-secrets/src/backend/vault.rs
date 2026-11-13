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
//!   Argon2's memory is reserved fallibly: a reservation the machine cannot make is an error,
//!   never an abort.
//! - Salt and nonce come from `getrandom::fill`. Every write draws a fresh nonce and keeps the
//!   salt (and therefore the key).
//! - The key lives in one heap `Zeroizing` buffer, the decrypted map in `SecretString` values:
//!   the JSON is built and decrypted inside `Zeroizing<Vec<u8>>` buffers sized up front (never
//!   reallocated while they hold plaintext) and parsed straight into `SecretString`. Argon2
//!   wipes its own state (`zeroize` feature) and its working memory is wiped here.
//! - **Several instances (fix round 1, I1/I2).** Until the single-instance lock (M8) two
//!   Hacivat processes may share one vault. Every unlock, read, read-modify-write, create and
//!   reset holds an exclusive advisory lock on `vault.bin.lock` (`std::fs::File::lock`). Under
//!   it the vault re-reads the header: a changed nonce (another instance wrote) reloads the map
//!   with the held key before the change is applied; a changed salt or a missing file (the vault
//!   was reset or re-created elsewhere) locks this instance, so it never writes the old vault
//!   back. A read that races a replace by something outside the lock is retried; it is never
//!   reported `VaultCorrupt`.
//! - A write is atomic: a temp file with a fresh random name (`vault.bin.<hex>.tmp`,
//!   `create_new`, mode `0600`) is written, `fsync`ed and renamed over `vault.bin`, then the
//!   directory is `fsync`ed (Unix). The temp file only ever holds ciphertext. A symlink or any
//!   non-file at `vault.bin` is refused, and a share or device path is refused before any file
//!   system call.
//! - A failed write changes nothing: the in-memory map is replaced only after the rename.
//! - Every method blocks (Argon2 takes about a second with the default parameters): callers run
//!   them on the core runtime's blocking pool.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::ops::RangeInclusive;
use std::path::{Component, Path, PathBuf, Prefix};
use std::sync::{Mutex, MutexGuard, PoisonError, TryLockError};

use argon2::{Algorithm, Argon2, Block, Params, Version};
use chacha20poly1305::aead::{AeadInPlace, KeyInit};
use chacha20poly1305::{Key, XChaCha20Poly1305, XNonce};
use secrecy::{ExposeSecret, SecretString};
use zeroize::{Zeroize, Zeroizing};

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
    // ## m2b-t21-prompts
    /// Whether a reset may be offered: the vault path holds a regular file, or nothing. A
    /// symlink or any other non-file there is the user's own arrangement (a dotfiles link):
    /// it is reported as `Denied` and never reset (T6 review N3). Blocking (a `stat`).
    fn resettable(&self) -> bool {
        true
    }
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
/// How often a read is retried when the file was replaced between its check and its open.
const READ_ATTEMPTS: usize = 16;
// ## m2b-t21-prompts
/// How long a vault call waits for `vault.bin.lock` held by another instance (T6 review N2).
const LOCK_WAIT: std::time::Duration = std::time::Duration::from_secs(5);
/// How often the lock is tried meanwhile.
const LOCK_POLL: std::time::Duration = std::time::Duration::from_millis(20);

/// The header bounds checked before any key is derived (Q30).
const M_KIB_BOUNDS: RangeInclusive<u32> = 8_192..=1_048_576;
const T_BOUNDS: RangeInclusive<u32> = 1..=10;
const P_BOUNDS: RangeInclusive<u32> = 1..=8;

/// The vault key, allocated once on the heap so moves never copy it.
type VaultKey = Box<Zeroizing<[u8; KEY_LEN]>>;

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
    /// tag, starts with the magic and has a non-zero major.
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
        let header = Header {
            major: u16_at(8),
            minor: u16_at(10),
            params: VaultParams {
                m_kib: u32_at(12),
                t: u32_at(16),
                p: u32_at(20),
            },
            salt,
            nonce,
        };
        (header.major != 0).then_some(header)
    }
}

/// An unlocked vault: the key, the decrypted map and the header it was read from or last
/// written with. Deliberately without `Debug`.
struct Unlocked {
    key: VaultKey,
    params: VaultParams,
    salt: [u8; SALT_LEN],
    /// The nonce of the file as this instance last read or wrote it.
    nonce: [u8; NONCE_LEN],
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
    /// Never blocks: a vault busy in another call shows `unlocked: "busy"` (fix round 1, M1).
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let unlocked: &dyn fmt::Debug = match self.state.try_lock() {
            Ok(state) if state.is_some() => &true,
            Ok(_) => &false,
            Err(TryLockError::Poisoned(state)) if state.get_ref().is_some() => &true,
            Err(TryLockError::Poisoned(_)) => &false,
            Err(TryLockError::WouldBlock) => &"busy",
        };
        f.debug_struct("FileVault")
            .field("path", &self.path)
            .field("params", &self.params)
            .field("unlocked", unlocked)
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

    /// Whether anything is at the vault path. Only "not found" means no vault; any other
    /// failure to look is an error (fix round 1, M4), so a vault that cannot be seen is never
    /// taken for an empty one.
    fn presence(&self) -> Result<bool, SecretError> {
        if names_another_machine(&self.path) {
            return Ok(false);
        }
        match fs::symlink_metadata(&self.path) {
            Ok(_) => Ok(true),
            Err(e)
                if matches!(
                    e.kind(),
                    io::ErrorKind::NotFound | io::ErrorKind::NotADirectory
                ) =>
            {
                Ok(false)
            }
            Err(e) => Err(io_error(&e)),
        }
    }

    /// The answer for a locked vault: nothing stored when there is no vault (`none`),
    /// `VaultLocked` when there is one.
    fn when_locked<T>(&self, none: T) -> Result<T, SecretError> {
        if self.presence()? {
            Err(SecretError::VaultLocked)
        } else {
            Ok(none)
        }
    }

    /// The exclusive lock on `<vault>.lock`, held while this instance reads or writes the vault
    /// file. `None` when the lock file cannot be opened or locked (a read-only directory): a
    /// read then goes ahead unlocked (and retries a racing replace), a write refuses.
    fn file_lock(&self) -> Option<File> {
        // A share or device path is never touched, not even for its lock file.
        if names_another_machine(&self.path) {
            return None;
        }
        let path = sibling(&self.path, ".lock");
        if fs::symlink_metadata(&path).is_ok_and(|m| !m.file_type().is_file()) {
            return None;
        }
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let file = options.open(&path).or_else(|_| File::open(&path)).ok()?;
        // ## m2b-t21-prompts (T6 review N2): a stopped or hung instance holding the lock must not
        // stall every vault call: try for at most `LOCK_WAIT`, then go on as without a lock (a
        // read reads, a write refuses with `VaultReadOnly`, which a Forever put falls back from).
        let deadline = std::time::Instant::now() + LOCK_WAIT;
        loop {
            match file.try_lock() {
                Ok(()) => return Some(file),
                Err(std::fs::TryLockError::WouldBlock) if std::time::Instant::now() < deadline => {
                    std::thread::sleep(LOCK_POLL);
                }
                Err(_) => return None,
            }
        }
    }

    /// [`file_lock`](Self::file_lock) for a write: missing is `VaultReadOnly` (the vault cannot
    /// be written safely).
    fn write_lock(&self) -> Result<File, SecretError> {
        self.file_lock().ok_or(SecretError::VaultReadOnly)
    }

    /// Under the file lock: brings `slot` up to date with the file. A changed nonce (another
    /// instance wrote) reloads the map with the held key. A changed salt or parameters, or a
    /// missing file (reset or re-created elsewhere), locks this instance and is `VaultLocked`;
    /// a file the held key cannot open is `VaultCorrupt` and locks too. Either way nothing of
    /// the old vault is ever written back.
    fn refresh(&self, slot: &mut Option<Unlocked>) -> Result<(), SecretError> {
        let Some(u) = slot.as_mut() else {
            return Err(SecretError::VaultLocked);
        };
        let file = match read_vault(&self.path) {
            Ok(file) => file,
            Err(SecretError::Unavailable) => {
                *slot = None;
                return Err(SecretError::VaultLocked);
            }
            Err(e) => return Err(e),
        };
        let Some(header) = Header::decode(&file) else {
            *slot = None;
            return Err(SecretError::VaultCorrupt);
        };
        if header.salt != u.salt || header.params != u.params {
            *slot = None;
            return Err(SecretError::VaultLocked);
        }
        if header.nonce == u.nonce {
            return Ok(());
        }
        match open_body(&u.key, &header, &file) {
            Ok(entries) => {
                u.entries = entries;
                u.nonce = header.nonce;
                u.read_only = header.major > FORMAT_MAJOR;
                Ok(())
            }
            Err(_) => {
                *slot = None;
                Err(SecretError::VaultCorrupt)
            }
        }
    }

    /// The read-modify-write of `put` and `delete` (fix round 1, I2): under the file lock, the
    /// map is refreshed from the file, `change` is applied to a copy, and the copy is written
    /// and then installed. `change` returns false when nothing changed (nothing is written).
    fn modify(
        &self,
        change: impl FnOnce(&mut BTreeMap<String, SecretString>) -> bool,
    ) -> Result<(), SecretError> {
        let mut state = self.state();
        if state.is_none() {
            return Err(SecretError::VaultLocked);
        }
        let _lock = self.write_lock()?;
        self.refresh(&mut state)?;
        let u = state.as_mut().ok_or(SecretError::VaultLocked)?;
        if u.read_only {
            return Err(SecretError::VaultReadOnly);
        }
        let mut entries = u.entries.clone();
        if !change(&mut entries) {
            return Ok(());
        }
        write_entries(&self.path, u, entries)
    }
}

impl SecretBackend for FileVault {
    /// `VaultLocked` while locked; with no vault file there is nothing stored (`None`).
    fn get(&self, key: &SecretKey) -> Result<Option<SecretString>, SecretError> {
        let mut state = self.state();
        if state.is_none() {
            return self.when_locked(None);
        }
        let lock = self.file_lock();
        match self.refresh(&mut state) {
            Ok(()) => {}
            Err(SecretError::VaultLocked) => {
                drop(lock);
                return self.when_locked(None);
            }
            Err(e) => return Err(e),
        }
        Ok(state
            .as_ref()
            .and_then(|u| u.entries.get(&key.service()).cloned()))
    }

    /// `VaultLocked` while locked (or before the vault exists, or once it was reset
    /// elsewhere), `VaultReadOnly` for a newer format or a file that cannot be written.
    fn put(&self, key: &SecretKey, value: &SecretString) -> Result<(), SecretError> {
        let service = key.service();
        self.modify(|entries| {
            entries.insert(service, value.clone());
            true
        })
    }

    /// Deleting a missing entry, or from a vault that does not exist, succeeds.
    fn delete(&self, key: &SecretKey) -> Result<(), SecretError> {
        let service = key.service();
        match self.modify(|entries| entries.remove(&service).is_some()) {
            Err(SecretError::VaultLocked) => self.when_locked(()),
            other => other,
        }
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
    /// Whether something is (or may be) at the vault path: only "not found" is false. A share
    /// or device path never exists.
    fn exists(&self) -> bool {
        !matches!(self.presence(), Ok(false))
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
        let mut state = self.state();
        create_dir(&parent_dir(&self.path)).map_err(|_| SecretError::VaultReadOnly)?;
        let _lock = self.write_lock()?;
        if self.presence()? {
            return Err(SecretError::Backend("vault-exists".to_string()));
        }
        let mut salt = [0u8; SALT_LEN];
        random(&mut salt)?;
        let key = derive_key(&master, &salt, self.params).map_err(|e| match e {
            KdfError::Params => SecretError::Backend("vault-params".to_string()),
            KdfError::Memory => SecretError::Backend("vault-memory".to_string()),
        })?;
        drop(master);
        let mut u = Unlocked {
            key,
            params: self.params,
            salt,
            nonce: [0u8; NONCE_LEN],
            read_only: false,
            entries: BTreeMap::new(),
        };
        write_entries(&self.path, &mut u, BTreeMap::new())?;
        *state = Some(u);
        Ok(())
    }

    /// A wrong password, or a body that fails the AEAD under a header that parses, is
    /// `WrongMasterPassword` (the AEAD cannot tell tampering from a wrong password, Q30); a
    /// header that does not parse, a truncated file or out-of-bounds parameters are
    /// `VaultCorrupt`; a symlink or other non-file at the path is `Denied` (m2b-t21, T6 review
    /// N3). Nothing is written either way. Holds the state lock and the
    /// file lock from the read to the install (fix round 1, M8).
    fn unlock(&self, master: SecretString) -> Result<(), SecretError> {
        if names_another_machine(&self.path) {
            return Err(SecretError::Unavailable);
        }
        let mut state = self.state();
        let _lock = self.file_lock();
        let file = read_vault(&self.path)?;
        let header = Header::decode(&file).ok_or(SecretError::VaultCorrupt)?;
        if !self.accepts(header.params) {
            return Err(SecretError::VaultCorrupt);
        }
        let key = derive_key(&master, &header.salt, header.params).map_err(|e| match e {
            KdfError::Params => SecretError::VaultCorrupt,
            KdfError::Memory => SecretError::Backend("vault-memory".to_string()),
        })?;
        drop(master);
        let entries = open_body(&key, &header, &file)?;
        *state = Some(Unlocked {
            key,
            params: header.params,
            salt: header.salt,
            nonce: header.nonce,
            read_only: header.major > FORMAT_MAJOR,
            entries,
        });
        Ok(())
    }

    /// Drops the key and the map; both zeroize their memory.
    fn lock(&self) {
        *self.state() = None;
    }

    /// Locks, then deletes the vault file and any stale temp file under the file lock, and
    /// syncs the directory (fix round 1, M3). A missing file is fine; the lock file stays.
    /// A symlink or any other non-file at the path is `Denied` and stays (T6 review N3).
    fn reset(&self) -> Result<(), SecretError> {
        let mut state = self.state();
        *state = None;
        if names_another_machine(&self.path) {
            return Err(SecretError::Unavailable);
        }
        let _lock = self.file_lock();
        // ## m2b-t21-prompts
        if !self.resettable() {
            return Err(SecretError::Denied);
        }
        match fs::remove_file(&self.path) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(_) => return Err(SecretError::VaultReadOnly),
        }
        let dir = parent_dir(&self.path);
        remove_stale_temps(&dir, &self.path);
        sync_dir(&dir);
        Ok(())
    }

    // ## m2b-t21-prompts
    /// A regular file or nothing at the path; never a share or device path, a symlink, a
    /// directory, or a path that cannot be looked at.
    fn resettable(&self) -> bool {
        if names_another_machine(&self.path) {
            return false;
        }
        match fs::symlink_metadata(&self.path) {
            Ok(meta) => meta.file_type().is_file(),
            Err(e) => e.kind() == io::ErrorKind::NotFound,
        }
    }
}

/// Maps an I/O error on the vault file, never carrying a path or content.
fn io_error(e: &io::Error) -> SecretError {
    match e.kind() {
        io::ErrorKind::NotFound => SecretError::Unavailable,
        io::ErrorKind::PermissionDenied => SecretError::Denied,
        _ => SecretError::Backend("vault-read".to_string()),
    }
}

/// Fills `buf` from the OS random source.
fn random(buf: &mut [u8]) -> Result<(), SecretError> {
    getrandom::fill(buf).map_err(|_| SecretError::Backend("random".to_string()))
}

/// Why no key was derived.
enum KdfError {
    /// Argon2 refuses the parameters.
    Params,
    /// The working memory could not be reserved.
    Memory,
}

/// The Argon2id key for `master`. The working memory is reserved fallibly (fix round 1, M7)
/// and wiped before it is freed; Argon2 wipes its own state (`zeroize` feature).
fn derive_key(
    master: &SecretString,
    salt: &[u8; SALT_LEN],
    params: VaultParams,
) -> Result<VaultKey, KdfError> {
    let params = Params::new(params.m_kib, params.t, params.p, Some(KEY_LEN))
        .map_err(|_| KdfError::Params)?;
    let count = params.block_count();
    let mut blocks: Vec<Block> = Vec::new();
    blocks
        .try_reserve_exact(count)
        .map_err(|_| KdfError::Memory)?;
    blocks.resize(count, Block::new());
    let argon = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let mut key: VaultKey = Box::new(Zeroizing::new([0u8; KEY_LEN]));
    let derived = argon.hash_password_into_with_memory(
        master.expose_secret().as_bytes(),
        salt,
        &mut key[..],
        &mut blocks,
    );
    blocks.iter_mut().for_each(Zeroize::zeroize);
    derived.map_err(|_| KdfError::Params)?;
    Ok(key)
}

/// The map sealed in `file` under `header`: `WrongMasterPassword` when the AEAD fails,
/// `VaultCorrupt` when the authenticated plaintext is not a JSON map.
fn open_body(
    key: &VaultKey,
    header: &Header,
    file: &[u8],
) -> Result<BTreeMap<String, SecretString>, SecretError> {
    let (aad, body) = file.split_at(HEADER_LEN);
    let mut plain = Zeroizing::new(Vec::with_capacity(body.len()));
    plain.extend_from_slice(body);
    XChaCha20Poly1305::new(Key::from_slice(&key[..]))
        .decrypt_in_place(XNonce::from_slice(&header.nonce), aad, &mut *plain)
        .map_err(|_| SecretError::WrongMasterPassword)?;
    serde_json::from_slice(&plain).map_err(|_| SecretError::VaultCorrupt)
}

/// Seals `entries` under `u`'s key with a fresh nonce, writes the file atomically and, once it
/// is on disk, installs `entries` and the nonce in `u`. An I/O failure is `VaultReadOnly` (the
/// file cannot be written) and changes nothing.
fn write_entries(
    path: &Path,
    u: &mut Unlocked,
    entries: BTreeMap<String, SecretString>,
) -> Result<(), SecretError> {
    let mut buf = json_map(&entries, TAG_LEN);
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
    write_atomic(path, &file).map_err(|_| SecretError::VaultReadOnly)?;
    u.entries = entries;
    u.nonce = nonce;
    Ok(())
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
/// any file system call; a symlink or a non-file is `Denied` (m2b-t21, T6 review N3); an
/// oversized file is `VaultCorrupt`; a missing file is `Unavailable`. When the file is
/// replaced between the check and the open (a writer outside the lock), the read is retried;
/// a replace that never settles is
/// `Backend("vault-read")`, never `VaultCorrupt` (fix round 1, I1).
fn read_vault(path: &Path) -> Result<Vec<u8>, SecretError> {
    if names_another_machine(path) {
        return Err(SecretError::Unavailable);
    }
    for _ in 0..READ_ATTEMPTS {
        if let Some(bytes) = read_vault_once(path)? {
            return Ok(bytes);
        }
    }
    Err(SecretError::Backend("vault-read".to_string()))
}

/// One check-open-compare-read; `None` when the path was replaced in between. A symlink or any
/// other non-file at the path is `Denied`, never `VaultCorrupt`: it is not damage, and the
/// damaged-vault notice would offer a reset (T6 review N3).
fn read_vault_once(path: &Path) -> Result<Option<Vec<u8>>, SecretError> {
    let link = fs::symlink_metadata(path).map_err(|e| io_error(&e))?;
    // ## m2b-t21-prompts
    if !link.file_type().is_file() {
        return Err(SecretError::Denied);
    }
    if link.len() > MAX_VAULT_BYTES {
        return Err(SecretError::VaultCorrupt);
    }
    let file = match File::open(path) {
        Ok(file) => file,
        // Removed between the check and the open: look again.
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(io_error(&e)),
    };
    let meta = file.metadata().map_err(|e| io_error(&e))?;
    if !meta.is_file() || !same_file(&link, &meta) {
        return Ok(None);
    }
    let mut bytes = Vec::with_capacity(usize::try_from(meta.len()).unwrap_or(0));
    file.take(MAX_VAULT_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| io_error(&e))?;
    if bytes.len() as u64 > MAX_VAULT_BYTES {
        return Err(SecretError::VaultCorrupt);
    }
    Ok(Some(bytes))
}

#[cfg(unix)]
fn same_file(a: &fs::Metadata, b: &fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    a.dev() == b.dev() && a.ino() == b.ino()
}

/// Off Unix the symlink refusal is check-then-use only (`File::open` follows a reparse point
/// swapped in after the check). Accepted: the vault is used only where no keyring is usable,
/// which in practice is Linux; Windows and macOS always have one (B13, fix round 1, M5).
#[cfg(not(unix))]
fn same_file(_a: &fs::Metadata, _b: &fs::Metadata) -> bool {
    true
}

/// `<path><suffix>`, next to the vault.
fn sibling(path: &Path, suffix: &str) -> PathBuf {
    let mut name: OsString = path.file_name().unwrap_or_default().to_os_string();
    name.push(suffix);
    path.with_file_name(name)
}

/// The directory that holds `path`.
fn parent_dir(path: &Path) -> PathBuf {
    match path.parent() {
        Some(dir) if !dir.as_os_str().is_empty() => dir.to_path_buf(),
        _ => PathBuf::from("."),
    }
}

/// A fresh temp path for one write: `<name>.<32 random hex>.tmp` (fix round 1, I2).
fn temp_path(path: &Path) -> Result<PathBuf, SecretError> {
    let mut id = [0u8; 16];
    random(&mut id)?;
    let mut suffix = String::with_capacity(1 + 32 + 4);
    suffix.push('.');
    for b in id {
        suffix.push(char::from(HEX_DIGITS[usize::from(b >> 4)]));
        suffix.push(char::from(HEX_DIGITS[usize::from(b & 15)]));
    }
    suffix.push_str(".tmp");
    Ok(sibling(path, &suffix))
}

/// Removes temp files a crashed write left next to `path` (`<name>.*.tmp`, including M2b's
/// first `<name>.tmp`). Called only under the file lock, when no other write is in flight.
/// `remove_file` removes a link itself, never its target.
fn remove_stale_temps(dir: &Path, path: &Path) {
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
        return;
    };
    let prefix = format!("{name}.");
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let file_name = entry.file_name();
        if file_name
            .to_str()
            .is_some_and(|n| n.starts_with(&prefix) && n.ends_with(".tmp"))
        {
            let _ = fs::remove_file(entry.path());
        }
    }
}

/// Replaces the file at `path` with `bytes` atomically (B13): a fresh, randomly named temp file
/// (mode `0600`, `create_new`), `fsync`, rename, then the directory's `fsync` on Unix. A
/// symlink or other non-file at `path` is refused, never followed or replaced. Called under
/// the file lock.
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
    let dir = parent_dir(path);
    create_dir(&dir)?;
    remove_stale_temps(&dir, path);
    let tmp = temp_path(path).map_err(|_| io::Error::other("random"))?;
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
    sync_dir(&dir);
    Ok(())
}

/// `fsync`s `dir` on Unix, so a rename or removal in it survives a crash. Best effort: the
/// change itself is done; a failed sync only weakens durability.
fn sync_dir(dir: &Path) {
    #[cfg(unix)]
    let _ = File::open(dir).and_then(|d| d.sync_all());
    #[cfg(not(unix))]
    let _ = dir;
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
    }

    #[test]
    fn temp_names_are_fresh_siblings() {
        let path = Path::new("/d/vault.bin");
        let (a, b) = (temp_path(path).unwrap(), temp_path(path).unwrap());
        assert_ne!(a, b);
        let name = a.file_name().unwrap().to_str().unwrap();
        assert!(name.starts_with("vault.bin.") && name.ends_with(".tmp"));
        assert_eq!(name.len(), "vault.bin.".len() + 32 + ".tmp".len());
        assert_eq!(a.parent(), Some(Path::new("/d")));
        assert_eq!(sibling(path, ".lock"), PathBuf::from("/d/vault.bin.lock"));
    }

    /// Debug never waits on (or deadlocks against) a held state lock (M1).
    #[test]
    fn debug_never_blocks_on_the_state_lock() {
        let v = FileVault::open(PathBuf::from("/nonexistent/vault.bin"));
        let guard = v.state();
        let shown = format!("{v:?}");
        drop(guard);
        assert!(shown.contains("busy"), "{shown}");
        assert!(format!("{v:?}").contains("unlocked: false"));
    }
}
