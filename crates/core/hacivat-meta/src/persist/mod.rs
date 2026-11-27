//! The per-profile SQLite catalog cache (HV-MODEL-03, R-M4-15, R-M4-25, D-016): one file per
//! profile and format major (`<profile>.v<major>.sqlite`, mode 0600 with its `-wal`/`-shm`, in
//! a 0700 directory), one row per namespace, optional row encryption, a content-addressed
//! source store, a server identity check, compare-and-set saves and LRU eviction.
//!
//! M4 Task 11, spike origin `d86313b` (the spike's store: WAL, 0600, corrupt-file rotation
//! copied from `hacivat-exec/src/history/schema.rs`, bad-row drop, content-addressed sources);
//! encryption, identity, compare-and-set, debounce and eviction are new.
//!
//! - **Blocking.** Every method does file I/O; callers run them on the core runtime's blocking
//!   pool, never on the UI thread (spec §5.4). A store is `Send`, one owner at a time.
//! - **Memory-only.** Any `open` error means the profile's catalog stays in memory for the
//!   session (a read-only directory, a refused keyring, a file another program holds). The
//!   cache is disposable: nothing in it is user data.
//! - **Secrets.** `hacivat-meta` never calls the secret store (`MetaBelowIntrospect`): the hub
//!   (Task 15) fetches the `CatalogKey` on the blocking pool and passes
//!   [`Encryption::Row`]. No password or key is ever written to the cache, a log or a `Debug`.
//! - **Encrypted stores** (`catalog_sensitive`) hold no name in the clear: payloads, namespace
//!   keys, watermarks, source texts and the server fingerprint and epoch are sealed
//!   ([`crypt::RowCipher`], domain [`crypt::CATALOG_DOMAIN`]) and padded to size buckets
//!   ([`crypt::pad`]); rows are found by keyed lookup tags (HMAC-SHA256). A key that does not
//!   open the file's key check, or a plaintext file opened with a key, is scrubbed (with its
//!   `.corrupt-*` rotations) and rebuilt ([`RebuildReason::WrongKey`]). An encrypted file
//!   opened **without** a key is refused with [`CacheError::Disabled`] and left untouched
//!   (fix round 1, A-I1): turning encryption off deletes the files first (Task 20).
//! - **Untrusted file.** The SQLite schema is checked exactly before any other read or write,
//!   with `trusted_schema = OFF` and `SQLITE_DBCONFIG_DEFENSIVE` (fix round 1, B-I1); a load
//!   reads at most the per-profile cap and no row over `MAX_ROW_BYTES` (B-M3); decoding caps
//!   names and string expansion (`codec`, B-I3).

pub mod crypt;
mod debounce;
mod files;
mod store;

use std::fmt;
use std::path::PathBuf;

use hacivat_proto::ProfileId;

use crate::codec::{DecodeError, FORMAT_MAJOR};
use crate::ids::{NsKey, SourceRef};
use crate::namespace::NamespaceSnapshot;
use crate::props::Props;
use crate::snapshot::{CatalogChange, ContainerKey, LoadState};

pub use crypt::{CATALOG_DOMAIN, CryptError, RowCipher};
pub use debounce::{SAVE_DEBOUNCE, SaveDebouncer};
pub use files::evict_global;
#[cfg(feature = "test-support")]
pub use store::Fault;
pub use store::{CacheStore, Evicted, RowInfo};

/// The cache file name of `profile` (under the data directory's `catalog/`), at this build's
/// format major.
pub fn cache_file_name(profile: &ProfileId) -> String {
    format!("{}.v{FORMAT_MAJOR}.sqlite", profile.0)
}

/// A 32-byte per-profile data key (secret store slot `CatalogKey`). Its `Debug` is redacted,
/// it is wiped on drop, and it has no `PartialEq`: compare through the cipher, never by bytes.
#[derive(Clone)]
pub struct RowKey([u8; 32]);

impl RowKey {
    /// A key copied from `bytes`. It takes a reference so no by-value copy of the key is left on
    /// the caller's stack: keep the source in a `Zeroizing` buffer (as
    /// `SecretStore::catalog_key` returns it).
    pub fn from_bytes(bytes: &[u8; 32]) -> Self {
        RowKey(*bytes)
    }
}

impl zeroize::Zeroize for RowKey {
    fn zeroize(&mut self) {
        zeroize::Zeroize::zeroize(&mut self.0);
    }
}

impl Drop for RowKey {
    /// The key bytes are wiped through `zeroize` (volatile writes the optimizer keeps; fix
    /// round 1, m4).
    fn drop(&mut self) {
        zeroize::Zeroize::zeroize(self);
    }
}

impl zeroize::ZeroizeOnDrop for RowKey {}

impl fmt::Debug for RowKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("RowKey(<redacted>)")
    }
}

/// How rows are stored.
#[derive(Debug, Clone)]
pub enum Encryption {
    /// Plaintext rows (the file itself is 0600, D-016).
    None,
    /// XChaCha20-Poly1305 per row (`RowCipher`, domain `hv-catalog-v1`).
    Row(RowKey),
}

/// Which process writes the file: a save loses to a newer row of another writer (the
/// compare-and-set of HV-MODEL-03 step 5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct WriterId(pub u64);

/// Why a stored row could not be read (the row is deleted and its namespace re-introspected).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RowError {
    /// The payload does not decode (checksum, truncation, a malformed record, another major).
    Decode(DecodeError),
    /// A sealed column failed authentication (tampered, moved, or another key).
    Crypt(CryptError),
    /// The row's columns disagree (its key, lookup tag, minor or payload do not match).
    Mismatch,
    /// The row is larger than one row may be, or past the per-profile cap of this load; it was
    /// not read (B-M3).
    TooLarge,
}

impl fmt::Display for RowError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RowError::Decode(e) => write!(f, "{e}"),
            RowError::Crypt(e) => write!(f, "{e}"),
            RowError::Mismatch => write!(f, "inconsistent cache row"),
            RowError::TooLarge => write!(f, "cache row over the size limit"),
        }
    }
}

/// Why the cache cannot be used, or one operation failed.
#[derive(Debug)]
pub enum CacheError {
    /// Persistence is off (memory-only): an encrypted cache opened without its key (the file
    /// is left untouched), or no key for an encrypted profile.
    Disabled,
    Io(std::io::Error),
    Sqlite(rusqlite::Error),
    /// The file's schema changed while the store was open (fix round 2, N1): the call was
    /// abandoned before any statement ran on it, the file was moved to `renamed_to`, and the
    /// store now holds an empty, valid file (re-introspect; nothing is lost but cache).
    Corrupt {
        renamed_to: PathBuf,
    },
    /// Another writer holds the file beyond the busy timeout.
    Locked,
    /// Another major format.
    Incompatible,
    /// A temp namespace is never persisted.
    TempNamespace,
    /// The namespace was read from a newer minor and is not written back until a fresh
    /// introspection replaces it (edge 4).
    NewerMinor,
    /// Another writer stored a newer watermark for this namespace; its row is kept.
    Superseded,
    /// The disk (or the file's size limit) is full. `warn` is true for the first such failure
    /// of this store only (`model-cache-write-failed` is shown once); the in-memory catalog is
    /// unaffected and the previous row stays.
    Full {
        warn: bool,
    },
    /// A stored row could not be read; it was deleted (re-introspect the namespace).
    Row(RowError),
    /// Sealing failed (the random source).
    Crypt(CryptError),
    /// The namespace could not be encoded.
    Encode(DecodeError),
}

impl fmt::Display for CacheError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CacheError::Disabled => write!(f, "the catalog cache is memory-only"),
            CacheError::Io(e) => write!(f, "cache i/o: {e}"),
            CacheError::Sqlite(e) => write!(f, "cache database: {e}"),
            CacheError::Corrupt { renamed_to } => {
                write!(f, "corrupt cache moved to {}", renamed_to.display())
            }
            CacheError::Locked => write!(f, "the cache is locked by another writer"),
            CacheError::Incompatible => write!(f, "incompatible cache format"),
            CacheError::TempNamespace => write!(f, "temp namespaces are not persisted"),
            CacheError::NewerMinor => write!(f, "row from a newer format minor"),
            CacheError::Superseded => write!(f, "a newer row of another writer is kept"),
            CacheError::Full { .. } => write!(f, "the cache disk is full"),
            CacheError::Row(e) => write!(f, "unreadable cache row dropped: {e}"),
            CacheError::Crypt(e) => write!(f, "cache encryption: {e}"),
            CacheError::Encode(e) => write!(f, "cache encoding: {e}"),
        }
    }
}

impl std::error::Error for CacheError {}

impl From<std::io::Error> for CacheError {
    fn from(e: std::io::Error) -> Self {
        CacheError::Io(e)
    }
}

/// Why an opened cache was started empty (`model-cache-rebuilt`, `$reason`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RebuildReason {
    /// The file was not a database or failed `quick_check`; it was moved aside (`corrupt`).
    Corrupt,
    /// The file's header names another format major (a hand-copied file; `incompatible`).
    MajorChanged,
    /// The key does not open the file, or the file is of the other mode (plaintext versus
    /// encrypted); it was scrubbed (`wrong-key`).
    WrongKey,
    /// The server behind the profile changed (`server-changed`; rows are kept, all stale).
    ServerChanged,
}

impl RebuildReason {
    /// The `$reason` selector of `model-cache-rebuilt`.
    pub fn ftl_reason(self) -> &'static str {
        match self {
            RebuildReason::Corrupt => "corrupt",
            RebuildReason::MajorChanged => "incompatible",
            RebuildReason::WrongKey => "wrong-key",
            RebuildReason::ServerChanged => "server-changed",
        }
    }
}

/// An opened cache, and whether (and why) it had to be started empty.
#[derive(Debug)]
pub struct Opened {
    pub store: CacheStore,
    pub rebuilt: Option<RebuildReason>,
    /// Where a corrupt or foreign-major file was moved (`*.corrupt-<secs>[-n]`).
    pub moved_to: Option<PathBuf>,
}

/// Every namespace a cache holds, plus the rows that could not be read (deleted; only those
/// namespaces are re-introspected).
#[derive(Debug, Default)]
pub struct LoadAll {
    pub namespaces: Vec<NamespaceSnapshot>,
    /// Unreadable rows: their namespace key when it could still be read.
    pub dropped: Vec<(Option<NsKey>, RowError)>,
    /// Namespaces written by a newer format minor: shown, loaded with `Watermark::None`, never
    /// written back until a fresh introspection replaces them.
    pub from_newer_minor: Vec<NsKey>,
}

impl LoadAll {
    /// The publish of a cache load (HV-MODEL-03 step 3): each namespace put, and its state
    /// `Loaded { stale: true }` until a background validation (HV-INTRO-15) clears it.
    pub fn into_changes(self) -> Vec<CatalogChange> {
        let mut out = Vec::with_capacity(self.namespaces.len() * 2);
        for ns in self.namespaces {
            let state = CatalogChange::SetState(
                ContainerKey::Namespace(ns.key().clone()),
                cached_state(&ns),
            );
            out.push(CatalogChange::Put(std::sync::Arc::new(ns)));
            out.push(state);
        }
        out
    }
}

/// The load state of a namespace read from the cache: `Loaded` at its level and watermark,
/// `stale: true` (validated in the background before anything trusts it as current).
pub fn cached_state(ns: &NamespaceSnapshot) -> LoadState {
    let meta = ns.meta();
    let at = std::time::UNIX_EPOCH
        + std::time::Duration::from_millis(u64::try_from(meta.loaded_at_ms).unwrap_or(0));
    LoadState::Loaded {
        level: meta.level,
        at,
        watermark: meta.watermark.clone(),
        stale: true,
    }
}

/// The identity of the server a cache was filled from (a restore, a swapped host): an XXH3-128
/// of the engine's identity facts (PG `system_identifier`, MySQL `@@server_uuid`, SQLite file
/// identity), computed by the introspector.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ServerFingerprint(pub u128);

/// Whether the server behind a profile is the one the cache was filled from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdentityCheck {
    Same,
    /// Every namespace is marked stale (loaded with `Watermark::None`, so the next pass is
    /// full); rows are kept for offline use until replaced.
    Changed,
}

/// The cache size caps (R-M4-25), in bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CacheCaps {
    pub per_profile: u64,
    pub global: u64,
}

impl CacheCaps {
    /// 512 MiB per profile, 4 GiB in all (R-M4-25; `metadata.cache_mb_*` defaults).
    pub const DEFAULT: CacheCaps = CacheCaps {
        per_profile: 512 << 20,
        global: 4096 << 20,
    };

    /// The caps of the settings `metadata.cache_mb_per_profile` and `metadata.cache_mb_total`
    /// (MiB). The total is clamped up to the per-profile cap (T1c concern 5): one profile's
    /// cache may always fill its own cap.
    pub fn from_mib(per_profile_mib: u32, total_mib: u32) -> Self {
        let per_profile = u64::from(per_profile_mib) << 20;
        CacheCaps {
            per_profile,
            global: (u64::from(total_mib) << 20).max(per_profile),
        }
    }
}

/// The source texts a namespace references (views, routines, triggers): the live set for
/// [`CacheStore::gc_sources`].
pub fn source_refs(ns: &NamespaceSnapshot) -> Vec<SourceRef> {
    ns.objs()
        .iter()
        .filter_map(|o| match o.props() {
            Props::View(p) => p.source,
            Props::Routine(p) => p.source,
            Props::Trigger(p) => p.source,
            _ => None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Fix round 1 (m4): `zeroize` clears every key byte (the `Drop` impl calls exactly this),
    /// and the type is `ZeroizeOnDrop`.
    #[test]
    fn row_key_zeroize_clears_every_byte() {
        fn on_drop<T: zeroize::ZeroizeOnDrop>(_: &T) {}
        let mut key = RowKey::from_bytes(&[0x5A; 32]);
        on_drop(&key);
        zeroize::Zeroize::zeroize(&mut key);
        assert_eq!(key.0, [0; 32]);
    }

    #[test]
    fn caps_clamp_the_total_up_to_one_profile() {
        assert_eq!(CacheCaps::from_mib(512, 4096), CacheCaps::DEFAULT);
        let c = CacheCaps::from_mib(1024, 100);
        assert_eq!(c.global, c.per_profile);
        assert_eq!(c.per_profile, 1024 << 20);
    }
}
