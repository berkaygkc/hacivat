//! The `connections.toml` store: load with diagnostics and quarantine, save atomically, never
//! a secret (HV-SEC-05, P29).
//!
//! ```toml
//! version = 1
//!
//! [[connection]]
//! id = "6f9619ff-8b86-d011-b42d-00c04fc964ff"
//! name = "orders-prod-us"
//! engine = "postgres"
//! environment = "production"
//! read_only = false
//! host = "db-prod-us.acme.io"        # or `socket = "/path"`, or `file = "/a.db"` + `create_if_missing`
//! port = 5433
//! database = "orders"
//! user = "deploy"
//! auth = "pgpass"                   # left out for the default, "password"
//! retention = "forever"
//! tls.mode = "verify_full"            # plus tls.ca_file, tls.client_cert, tls.client_key
//! transport = "direct"                # "ssh_tunnel", or { kind = "karagoz", org, agent_id, resource_id }
//! connect_timeout_s = 15
//! keepalive_s = 60
//! application_name = "Hacivat"
//! max_connections = 4
//! extra = [{ key = "charset", value = "utf8mb4" }]
//! origin = "user_typed"               # or { kind = "perde_managed", org = "acme" }
//! allow_unsupported_server = false
//! ```
//!
//! - **Secrets.** A `password`, `passwd`, `pwd`, `pass` or `sslpassword` key (ASCII case
//!   ignored, at any depth of an entry, or as an `extra` key) is dropped on load with
//!   `SecretKeyIgnored`; outside the entries (top level, unknown tables) with
//!   `SecretKeyOutsideEntries`. It is never echoed and never written back.
//! - **Unknown keys** are kept: a save re-reads the file and edits it with `toml_edit`, so
//!   unknown top-level keys, unknown keys of an entry and whole entries this version cannot read
//!   (an unknown engine, an invalid value) stay as they are. A saved profile keeps the unknown
//!   keys of the entry with its ID. Only the entries of profiles this store loaded or wrote and
//!   that a save no longer carries (deleted) are dropped; later entries with a duplicate ID and
//!   entries added by hand while the app runs are kept.
//! - **Environment tags (D-004).** An unknown `environment` (`uat`) loads as no tag, with
//!   `ValueIgnored` (and stays on disk while the profile has none). `env_source = "org"` counts
//!   only on a profile the organization manages (`origin` `perde_managed`); elsewhere it loads
//!   as the user's tag, with `ValueIgnored`.
//! - **Versions.** A `version` above 1 makes the store read-only (`NewerVersion`); nothing is
//!   written. A file that is not TOML or has a malformed `version` is set aside with
//!   `fsio::quarantine(path, "corrupt", now)`. A file that cannot be read (permissions, a disk
//!   error), or cannot be set aside, is left untouched and the store is read-only
//!   (`Unreadable`, `QuarantineFailed`): never a silent empty list (P29).
//! - **Writes** are atomic (`fsio::atomic_write`): the file is `0600` and a created directory
//!   `0700`. Paths are stored as UTF-8 (a non-UTF-8 path is written lossily).

use std::collections::hash_map::Entry;
use std::collections::{HashMap, HashSet};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::SystemTime;

use hacivat_net::{ClientCertPaths, TlsMode, TlsSettings};
use hacivat_proto::{DbmsKind, EnvTag, ImportSource, Origin, ProfileId, TagSource};
use hacivat_secrets::{AuthProviderId, Retention};
use hacivat_settings::fsio;
use toml_edit::{
    Array, ArrayOfTables, DocumentMut, InlineTable, Item, Table, TableLike, Value, value,
};

use super::{Profile, ProfileEndpoint, TransportSpec};

/// The file format version this build writes.
const VERSION: i64 = 1;

/// Keys that hold a secret in other tools' files; never kept (ASCII case ignored).
const SECRET_KEYS: [&str; 5] = ["password", "passwd", "pwd", "pass", "sslpassword"];

/// A problem found while loading `connections.toml`; shown, never dropped (P29).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ProfileDiag {
    InvalidEntry {
        index: usize,
        reason: String,
    },
    DuplicateId {
        id: ProfileId,
    },
    UnknownEngine {
        index: usize,
        engine: String,
    },
    /// An entry carried a password key; it was ignored (never echoed).
    SecretKeyIgnored {
        index: usize,
    },
    NewerVersion {
        found: u32,
    },
    // ## m2a-t12-profiles (fix round 1)
    /// A value of an entry was not understood and was ignored; the entry still loads. `key` is
    /// `environment` (an unknown tag: no tag) or `env_source` (an unknown source, or an
    /// organization tag on a profile the organization does not manage: the user's tag).
    ValueIgnored {
        index: usize,
        key: String,
    },
    /// A password key outside the `[[connection]]` entries was ignored (never echoed).
    SecretKeyOutsideEntries,
    /// The file exists but could not be read (`reason`: the I/O error kind). It is left
    /// untouched and the store is read-only.
    Unreadable {
        reason: String,
    },
    /// The file could not be parsed and could not be set aside (`reason`: the I/O error kind).
    /// It is left untouched and the store is read-only.
    QuarantineFailed {
        reason: String,
    },
}

/// What a save did besides writing (P29).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct SaveReport {
    /// The file had become unparsable since it was opened; it was set aside here first.
    pub quarantined: Option<PathBuf>,
}

/// What [`ProfileStore::open`] loaded (P29).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct LoadedProfiles {
    pub profiles: Vec<Profile>,
    pub diags: Vec<ProfileDiag>,
    /// Where an unreadable file was set aside.
    pub quarantined: Option<PathBuf>,
}

/// Why saving failed (HV-SEC-05).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum StoreError {
    /// The file is from a newer version, or not writable: nothing is written.
    ReadOnly,
    Io(String),
}

/// `connections.toml` (HV-SEC-05). Blocking: the app calls it on the core runtime.
#[derive(Debug)]
pub struct ProfileStore {
    path: PathBuf,
    read_only: bool,
    /// The IDs this store loaded or wrote. An entry on disk with one of them that a save no
    /// longer carries was deleted; any other entry the save does not carry (added by hand
    /// while the app runs) is kept.
    known: Mutex<HashSet<ProfileId>>,
}

impl ProfileStore {
    /// Opens the store at `path` and loads its profiles. A missing file loads nothing. A file
    /// that cannot be parsed is set aside (`quarantined`); one that cannot be read, or set
    /// aside, is left untouched and the store is read-only, with a diagnostic (P29).
    pub fn open(path: PathBuf) -> (ProfileStore, LoadedProfiles) {
        let mut out = LoadedProfiles::default();
        let doc = match read_doc(&path) {
            Ok(Some(doc)) => doc,
            Ok(None) => return (ProfileStore::at(path, false, &out), out),
            Err(ReadError::Io(reason)) => {
                out.diags.push(ProfileDiag::Unreadable { reason });
                return (ProfileStore::at(path, true, &out), out);
            }
            Err(ReadError::Parse) => return quarantine_on_open(path, out),
        };
        let version = match version_of(&doc) {
            Ok(v) => v,
            Err(ReadError::Io(_) | ReadError::Parse) => return quarantine_on_open(path, out),
        };
        let read_only = version > VERSION;
        if read_only {
            out.diags.push(ProfileDiag::NewerVersion {
                found: u32::try_from(version).unwrap_or(u32::MAX),
            });
        }
        let secret_outside = doc.iter().any(|(key, item)| {
            key != "connection" && (is_secret_key(key) || item_has_secrets(item))
        });
        if secret_outside {
            out.diags.push(ProfileDiag::SecretKeyOutsideEntries);
        }
        match doc.get("connection") {
            None => {}
            Some(Item::ArrayOfTables(entries)) => {
                for (index, entry) in entries.iter().enumerate() {
                    load_entry(index, entry, &mut out);
                }
            }
            Some(_) => out.diags.push(ProfileDiag::InvalidEntry {
                index: 0,
                reason: "connection".to_string(),
            }),
        }
        (ProfileStore::at(path, read_only, &out), out)
    }

    fn at(path: PathBuf, read_only: bool, loaded: &LoadedProfiles) -> ProfileStore {
        ProfileStore {
            path,
            read_only,
            known: Mutex::new(loaded.profiles.iter().map(|p| p.id).collect()),
        }
    }

    /// Writes `profiles`, in order, keeping unknown keys and entries this version cannot read.
    /// A read-only store, or a file that became newer since it was opened, writes nothing.
    pub fn save(&self, profiles: &[Profile]) -> Result<(), StoreError> {
        self.save_reporting(profiles).map(|_| ())
    }

    /// [`save`](Self::save), reporting a file that was set aside first (P29).
    ///
    /// The file is re-read and edited: `profiles` come first, each in the entry with its ID
    /// (keeping that entry's unknown keys), then every other entry as it is (entries this
    /// version cannot read, later entries with a duplicate ID, entries added by hand while the
    /// app runs). Only the entries of profiles this store loaded or wrote and that `profiles`
    /// no longer carries (deleted) are dropped. A file that cannot be read is left untouched
    /// (`Io`); one that cannot be parsed is set aside first.
    pub fn save_reporting(&self, profiles: &[Profile]) -> Result<SaveReport, StoreError> {
        if self.read_only {
            return Err(StoreError::ReadOnly);
        }
        let mut report = SaveReport::default();
        let mut doc = match read_doc(&self.path) {
            Ok(Some(doc)) => doc,
            Ok(None) => DocumentMut::new(),
            Err(ReadError::Io(reason)) => return Err(StoreError::Io(reason)),
            // Broken since it was opened: set it aside, as a load would, and start over.
            Err(ReadError::Parse) => {
                report.quarantined = Some(self.quarantine()?);
                DocumentMut::new()
            }
        };
        match version_of(&doc) {
            Ok(v) if v > VERSION => return Err(StoreError::ReadOnly),
            Ok(_) => {}
            Err(_) => {
                report.quarantined = Some(self.quarantine()?);
                doc = DocumentMut::new();
            }
        }
        // An existing `version = 1` keeps its comments.
        if !doc.contains_key("version") {
            doc.insert("version", value(VERSION));
        }

        let known = lock(&self.known).clone();
        let wanted: HashSet<ProfileId> = profiles.iter().map(|p| p.id).collect();
        let mut matched: HashMap<ProfileId, Table> = HashMap::new();
        let mut kept = Vec::new();
        if let Some(Item::ArrayOfTables(entries)) = doc.remove("connection") {
            for entry in entries {
                match read_entry(&entry).map(|(p, _)| p.id) {
                    Ok(id) if wanted.contains(&id) => match matched.entry(id) {
                        Entry::Vacant(slot) => {
                            slot.insert(entry);
                        }
                        Entry::Occupied(_) => kept.push(entry), // a later duplicate, as it is
                    },
                    Ok(id) if known.contains(&id) => {} // deleted
                    _ => kept.push(entry),
                }
            }
        }
        let mut entries = ArrayOfTables::new();
        for p in profiles {
            let mut entry = matched.remove(&p.id).unwrap_or_default();
            write_entry(&mut entry, p);
            entries.push(entry);
        }
        for entry in kept {
            entries.push(entry);
        }
        if !entries.is_empty() {
            doc.insert("connection", Item::ArrayOfTables(entries));
        }
        // No secret key is ever written back, inside the entries or outside them.
        strip_secrets(doc.as_table_mut());
        fsio::atomic_write(&self.path, doc.to_string().as_bytes()).map_err(io_error)?;
        lock(&self.known).extend(wanted);
        Ok(report)
    }

    fn quarantine(&self) -> Result<PathBuf, StoreError> {
        fsio::quarantine(&self.path, "corrupt", SystemTime::now()).map_err(io_error)
    }

    pub fn read_only(&self) -> bool {
        self.read_only
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

fn io_error(e: io::Error) -> StoreError {
    StoreError::Io(io_reason(&e))
}

/// A stable machine string for an I/O error (its kind), never OS text.
fn io_reason(e: &io::Error) -> String {
    format!("{:?}", e.kind())
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Why the file could not be loaded.
enum ReadError {
    /// It could not be read (permissions, a disk error); the I/O error kind.
    Io(String),
    /// It is not a `connections.toml` (not TOML, or a malformed `version`).
    Parse,
}

/// The parsed file; `None` when it does not exist.
fn read_doc(path: &Path) -> Result<Option<DocumentMut>, ReadError> {
    match std::fs::read_to_string(path) {
        Ok(text) => text
            .parse::<DocumentMut>()
            .map(Some)
            .map_err(|_| ReadError::Parse),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        // Not UTF-8: the bytes were read, but they are not a TOML file.
        Err(e) if e.kind() == io::ErrorKind::InvalidData => Err(ReadError::Parse),
        Err(e) => Err(ReadError::Io(io_reason(&e))),
    }
}

/// The file's `version` (1 when missing); a version that is not a positive integer is corrupt.
fn version_of(doc: &DocumentMut) -> Result<i64, ReadError> {
    match doc.get("version") {
        None => Ok(VERSION),
        Some(item) => match item.as_integer() {
            Some(v) if v >= 1 => Ok(v),
            _ => Err(ReadError::Parse),
        },
    }
}

/// Sets an unparsable file aside. When that fails, the file is left untouched and the store
/// is read-only, with a diagnostic: never a silent empty list (P29).
fn quarantine_on_open(path: PathBuf, mut out: LoadedProfiles) -> (ProfileStore, LoadedProfiles) {
    match fsio::quarantine(&path, "corrupt", SystemTime::now()) {
        Ok(aside) => {
            out.quarantined = Some(aside);
            (ProfileStore::at(path, false, &out), out)
        }
        Err(e) => {
            out.diags.push(ProfileDiag::QuarantineFailed {
                reason: io_reason(&e),
            });
            (ProfileStore::at(path, true, &out), out)
        }
    }
}

/// Loads entry `index` into `out`, with its diagnostics.
fn load_entry(index: usize, entry: &Table, out: &mut LoadedProfiles) {
    if has_secrets(entry) {
        out.diags.push(ProfileDiag::SecretKeyIgnored { index });
    }
    match read_entry(entry) {
        Ok((p, _)) if out.profiles.iter().any(|q| q.id == p.id) => {
            out.diags.push(ProfileDiag::DuplicateId { id: p.id });
        }
        Ok((p, ignored)) => {
            for key in ignored {
                out.diags.push(ProfileDiag::ValueIgnored {
                    index,
                    key: key.to_string(),
                });
            }
            out.profiles.push(p);
        }
        Err(Skip::UnknownEngine(engine)) => {
            out.diags.push(ProfileDiag::UnknownEngine { index, engine });
        }
        Err(Skip::Invalid(key)) => out.diags.push(ProfileDiag::InvalidEntry {
            index,
            reason: key.to_string(),
        }),
    }
}

// ---- secrets ---------------------------------------------------------------------------------

fn is_secret_key(key: &str) -> bool {
    SECRET_KEYS.iter().any(|s| key.eq_ignore_ascii_case(s))
}

/// Whether `item` is an `extra` pair `{ key = "<secret key>", value = … }`.
fn is_secret_pair(table: &dyn TableLike) -> bool {
    table
        .get("key")
        .and_then(Item::as_str)
        .is_some_and(is_secret_key)
}

/// Whether the entry holds a secret key anywhere (see [`strip_secrets`]).
fn has_secrets(entry: &dyn TableLike) -> bool {
    entry.iter().any(|(key, item)| {
        is_secret_key(key) || (key == "extra" && extra_has_secret(item)) || item_has_secrets(item)
    })
}

fn item_has_secrets(item: &Item) -> bool {
    match item {
        Item::Table(t) => has_secrets(t),
        Item::Value(Value::InlineTable(t)) => has_secrets(t),
        Item::Value(Value::Array(a)) => a.iter().any(|v| match v {
            Value::InlineTable(t) => has_secrets(t),
            _ => false,
        }),
        Item::ArrayOfTables(a) => a.iter().any(|t| has_secrets(t)),
        _ => false,
    }
}

fn extra_has_secret(item: &Item) -> bool {
    match item {
        Item::Value(Value::Array(a)) => a.iter().any(|v| match v {
            Value::InlineTable(t) => is_secret_pair(t),
            _ => false,
        }),
        Item::ArrayOfTables(a) => a.iter().any(|t| is_secret_pair(t)),
        _ => false,
    }
}

/// Removes every secret key of the entry, at any depth, and every `extra` pair with a secret
/// key.
fn strip_secrets(entry: &mut dyn TableLike) {
    let secret: Vec<String> = entry
        .iter()
        .filter(|(key, _)| is_secret_key(key))
        .map(|(key, _)| key.to_string())
        .collect();
    for key in secret {
        entry.remove(&key);
    }
    for (key, item) in entry.iter_mut() {
        if key.get() == "extra" {
            strip_secret_pairs(item);
        }
        strip_item(item);
    }
}

fn strip_item(item: &mut Item) {
    match item {
        Item::Table(t) => strip_secrets(t),
        Item::Value(Value::InlineTable(t)) => strip_secrets(t),
        Item::Value(Value::Array(a)) => {
            for v in a.iter_mut() {
                if let Value::InlineTable(t) = v {
                    strip_secrets(t);
                }
            }
        }
        Item::ArrayOfTables(a) => {
            for t in a.iter_mut() {
                strip_secrets(t);
            }
        }
        _ => {}
    }
}

fn strip_secret_pairs(item: &mut Item) {
    match item {
        Item::Value(Value::Array(a)) => {
            a.retain(|v| !matches!(v, Value::InlineTable(t) if is_secret_pair(t)));
        }
        Item::ArrayOfTables(a) => a.retain(|t| !is_secret_pair(t)),
        _ => {}
    }
}

// ---- reading an entry ------------------------------------------------------------------------

/// Why an entry is skipped.
enum Skip {
    UnknownEngine(String),
    /// The key whose value is missing or invalid.
    Invalid(&'static str),
}

type Read<T> = Result<T, Skip>;

fn invalid<T>(key: &'static str) -> Read<T> {
    Err(Skip::Invalid(key))
}

/// An optional string key.
fn opt_str(t: &dyn TableLike, key: &'static str) -> Read<Option<String>> {
    match t.get(key) {
        None => Ok(None),
        Some(item) => item
            .as_str()
            .map(|s| Some(s.to_string()))
            .ok_or(Skip::Invalid(key)),
    }
}

fn opt_bool(t: &dyn TableLike, key: &'static str) -> Read<Option<bool>> {
    match t.get(key) {
        None => Ok(None),
        Some(item) => item.as_bool().map(Some).ok_or(Skip::Invalid(key)),
    }
}

/// An optional integer key that fits `T` and is at least `min`.
fn opt_int<T: TryFrom<i64>>(t: &dyn TableLike, key: &'static str, min: i64) -> Read<Option<T>> {
    match t.get(key) {
        None => Ok(None),
        Some(item) => item
            .as_integer()
            .filter(|v| *v >= min)
            .and_then(|v| T::try_from(v).ok())
            .map(Some)
            .ok_or(Skip::Invalid(key)),
    }
}

/// An optional key holding one of a closed set of IDs.
fn opt_id<T>(
    t: &dyn TableLike,
    key: &'static str,
    from_id: impl Fn(&str) -> Option<T>,
) -> Read<Option<T>> {
    match opt_str(t, key)? {
        None => Ok(None),
        Some(id) => from_id(&id).map(Some).ok_or(Skip::Invalid(key)),
    }
}

/// The profile of entry `t`, and the keys whose values were not understood and were ignored
/// (`environment`, `env_source`; D-004).
fn read_entry(t: &Table) -> Read<(Profile, Vec<&'static str>)> {
    let mut ignored = Vec::new();
    let id: ProfileId = match opt_str(t, "id")? {
        Some(id) => id.parse().map_err(|_| Skip::Invalid("id"))?,
        None => return invalid("id"),
    };
    let name = opt_str(t, "name")?.unwrap_or_default();
    let engine = match opt_str(t, "engine")? {
        Some(e) => DbmsKind::from_id(&e).ok_or(Skip::UnknownEngine(e))?,
        None => return invalid("engine"),
    };
    let defaults = Profile::default();
    let endpoint = if let Some(path) = opt_str(t, "file")? {
        ProfileEndpoint::File {
            path: PathBuf::from(path),
            create_if_missing: opt_bool(t, "create_if_missing")?.unwrap_or(false),
        }
    } else if let Some(path) = opt_str(t, "socket")? {
        ProfileEndpoint::Socket {
            path: PathBuf::from(path),
            port: opt_int(t, "port", 1)?,
        }
    } else {
        ProfileEndpoint::Server {
            host: opt_str(t, "host")?.unwrap_or_default(),
            port: opt_int(t, "port", 1)?,
        }
    };
    let extra = match t.get("extra") {
        None => Vec::new(),
        Some(item) => read_extra(item)?,
    };
    let origin = read_origin(t)?;
    // An unknown tag (a newer version's, or a typo) is no tag: the user is asked again.
    let environment = opt_id(t, "environment", EnvTag::from_id).unwrap_or_else(|_| {
        ignored.push("environment");
        None
    });
    // Only the organization sets an organization tag (D-004): `org` counts only on a profile
    // it manages.
    let env_source = match opt_id(t, "env_source", tag_source_from_id) {
        Ok(Some(TagSource::Org)) if matches!(origin, Origin::PerdeManaged { .. }) => TagSource::Org,
        Ok(None | Some(TagSource::User)) => TagSource::User,
        Ok(Some(TagSource::Org)) | Err(_) => {
            ignored.push("env_source");
            TagSource::User
        }
    };
    let profile = Profile {
        id,
        name,
        engine,
        environment,
        env_source,
        read_only: opt_bool(t, "read_only")?.unwrap_or(false),
        endpoint,
        database: opt_str(t, "database")?,
        user: opt_str(t, "user")?,
        auth: opt_id(t, "auth", AuthProviderId::from_id)?.unwrap_or(defaults.auth),
        retention: opt_id(t, "retention", Retention::from_id)?.unwrap_or(defaults.retention),
        tls: read_tls(t)?,
        transport: read_transport(t)?,
        connect_timeout_s: opt_int(t, "connect_timeout_s", 0)?
            .unwrap_or(defaults.connect_timeout_s),
        keepalive_s: opt_int(t, "keepalive_s", 0)?,
        application_name: opt_str(t, "application_name")?.unwrap_or(defaults.application_name),
        max_connections: opt_int(t, "max_connections", 1)?.unwrap_or(
            if engine == DbmsKind::Sqlite {
                1
            } else {
                defaults.max_connections
            },
        ),
        extra,
        origin,
        allow_unsupported_server: opt_bool(t, "allow_unsupported_server")?.unwrap_or(false),
    };
    Ok((profile, ignored))
}

fn tag_source_from_id(id: &str) -> Option<TagSource> {
    match id {
        "user" => Some(TagSource::User),
        "org" => Some(TagSource::Org),
        _ => None,
    }
}

fn read_tls(t: &Table) -> Read<TlsSettings> {
    let Some(item) = t.get("tls") else {
        return Ok(TlsSettings::default());
    };
    let tls = item.as_table_like().ok_or(Skip::Invalid("tls"))?;
    let mode = opt_id(tls, "mode", TlsMode::from_id)
        .map_err(|_| Skip::Invalid("tls.mode"))?
        .unwrap_or_default();
    let path = |key: &'static str, reason: &'static str| {
        opt_str(tls, key)
            .map(|p| p.map(PathBuf::from))
            .map_err(|_| Skip::Invalid(reason))
    };
    let ca_file = path("ca_file", "tls.ca_file")?;
    let client_cert = match (
        path("client_cert", "tls.client_cert")?,
        path("client_key", "tls.client_key")?,
    ) {
        (None, None) => None,
        (Some(cert), Some(key)) => Some(ClientCertPaths { cert, key }),
        (Some(_), None) => return invalid("tls.client_key"),
        (None, Some(_)) => return invalid("tls.client_cert"),
    };
    Ok(TlsSettings {
        mode,
        ca_file,
        client_cert,
    })
}

fn read_transport(t: &Table) -> Read<TransportSpec> {
    match t.get("transport") {
        None => Ok(TransportSpec::Direct),
        Some(item) => {
            if let Some(kind) = item.as_str() {
                return match kind {
                    "direct" => Ok(TransportSpec::Direct),
                    "ssh_tunnel" => Ok(TransportSpec::SshTunnel),
                    _ => invalid("transport"),
                };
            }
            let table = item.as_table_like().ok_or(Skip::Invalid("transport"))?;
            let field = |key: &'static str| {
                opt_str(table, key)
                    .ok()
                    .flatten()
                    .ok_or(Skip::Invalid("transport"))
            };
            match field("kind")?.as_str() {
                "karagoz" => Ok(TransportSpec::Karagoz {
                    org: field("org")?,
                    agent_id: field("agent_id")?,
                    resource_id: field("resource_id")?,
                }),
                _ => invalid("transport"),
            }
        }
    }
}

fn read_origin(t: &Table) -> Read<Origin> {
    match t.get("origin") {
        None => Ok(Origin::UserTyped),
        Some(item) => {
            if let Some(kind) = item.as_str() {
                return match kind {
                    "user_typed" => Ok(Origin::UserTyped),
                    "imported_pg_service" => Ok(Origin::Imported {
                        source: ImportSource::PgService,
                    }),
                    "imported_url" => Ok(Origin::Imported {
                        source: ImportSource::Url,
                    }),
                    "deep_link" => Ok(Origin::DeepLink),
                    "mcp" => Ok(Origin::Mcp),
                    _ => invalid("origin"),
                };
            }
            let table = item.as_table_like().ok_or(Skip::Invalid("origin"))?;
            match (opt_str(table, "kind"), opt_str(table, "org")) {
                (Ok(Some(kind)), Ok(Some(org))) if kind == "perde_managed" => {
                    Ok(Origin::PerdeManaged { org })
                }
                _ => invalid("origin"),
            }
        }
    }
}

/// `extra = [{ key = "k", value = "v" }, …]` (inline tables, or `[[connection.extra]]`).
/// Pairs with a secret key are left out.
fn read_extra(item: &Item) -> Read<Vec<(String, String)>> {
    let pair = |t: &dyn TableLike| -> Read<Option<(String, String)>> {
        let key = opt_str(t, "key")
            .ok()
            .flatten()
            .ok_or(Skip::Invalid("extra"))?;
        let value = opt_str(t, "value")
            .ok()
            .flatten()
            .ok_or(Skip::Invalid("extra"))?;
        Ok((!is_secret_key(&key)).then_some((key, value)))
    };
    let mut out = Vec::new();
    match item {
        Item::Value(Value::Array(a)) => {
            for v in a.iter() {
                let t = v.as_inline_table().ok_or(Skip::Invalid("extra"))?;
                out.extend(pair(t)?);
            }
        }
        Item::ArrayOfTables(a) => {
            for t in a.iter() {
                out.extend(pair(t)?);
            }
        }
        _ => return invalid("extra"),
    }
    Ok(out)
}

// ---- writing an entry ------------------------------------------------------------------------

/// Sets `key` to `v`, or removes it.
fn set_opt(t: &mut dyn TableLike, key: &str, v: Option<Item>) {
    match v {
        Some(item) => {
            t.insert(key, item);
        }
        None => {
            t.remove(key);
        }
    }
}

fn path_value(p: &Path) -> Item {
    value(p.to_string_lossy().into_owned())
}

/// Writes every field of `p` into `t`, keeping its other keys.
fn write_entry(t: &mut Table, p: &Profile) {
    t.insert("id", value(p.id.to_string()));
    t.insert("name", value(p.name.clone()));
    t.insert("engine", value(p.engine.id()));
    // A tag this version does not know (`uat`) stays on disk while the profile has none.
    let unknown_tag = t
        .get("environment")
        .and_then(Item::as_str)
        .is_some_and(|id| EnvTag::from_id(id).is_none());
    if p.environment.is_some() || !unknown_tag {
        set_opt(t, "environment", p.environment.map(|e| value(e.id())));
    }
    set_opt(
        t,
        "env_source",
        (p.env_source == TagSource::Org).then(|| value("org")),
    );
    t.insert("read_only", value(p.read_only));
    let (host, socket, file, port, create) = match &p.endpoint {
        ProfileEndpoint::Server { host, port } => {
            (Some(value(host.clone())), None, None, *port, None)
        }
        ProfileEndpoint::Socket { path, port } => (None, Some(path_value(path)), None, *port, None),
        ProfileEndpoint::File {
            path,
            create_if_missing,
        } => (
            None,
            None,
            Some(path_value(path)),
            None,
            Some(value(*create_if_missing)),
        ),
    };
    set_opt(t, "host", host);
    set_opt(t, "socket", socket);
    set_opt(t, "file", file);
    set_opt(t, "create_if_missing", create);
    set_opt(t, "port", port.map(|p| value(i64::from(p))));
    set_opt(t, "database", p.database.clone().map(value));
    set_opt(t, "user", p.user.clone().map(value));
    // The default (password sign-in) is left out, so the file never even names a password.
    set_opt(
        t,
        "auth",
        (p.auth != AuthProviderId::Password).then(|| value(p.auth.id())),
    );
    t.insert("retention", value(p.retention.id()));
    write_tls(t, &p.tls);
    t.insert("transport", transport_item(&p.transport));
    t.insert("connect_timeout_s", value(i64::from(p.connect_timeout_s)));
    set_opt(t, "keepalive_s", p.keepalive_s.map(|k| value(i64::from(k))));
    t.insert("application_name", value(p.application_name.clone()));
    t.insert("max_connections", value(i64::from(p.max_connections)));
    set_opt(
        t,
        "extra",
        (!p.extra.is_empty()).then(|| extra_item(&p.extra)),
    );
    t.insert("origin", origin_item(&p.origin));
    t.insert(
        "allow_unsupported_server",
        value(p.allow_unsupported_server),
    );
}

/// `tls.*`, in the entry's existing `tls` table when it has one (keeping its unknown keys),
/// else as dotted keys.
fn write_tls(t: &mut Table, tls: &TlsSettings) {
    if !t.get("tls").is_some_and(|i| i.as_table_like().is_some()) {
        let mut table = Table::new();
        table.set_dotted(true);
        t.insert("tls", Item::Table(table));
    }
    let Some(table) = t.get_mut("tls").and_then(Item::as_table_like_mut) else {
        return;
    };
    table.insert("mode", value(tls.mode.id()));
    set_opt(table, "ca_file", tls.ca_file.as_deref().map(path_value));
    set_opt(
        table,
        "client_cert",
        tls.client_cert.as_ref().map(|c| path_value(&c.cert)),
    );
    set_opt(
        table,
        "client_key",
        tls.client_cert.as_ref().map(|c| path_value(&c.key)),
    );
}

fn transport_item(t: &TransportSpec) -> Item {
    match t {
        TransportSpec::Direct => value("direct"),
        TransportSpec::SshTunnel => value("ssh_tunnel"),
        TransportSpec::Karagoz {
            org,
            agent_id,
            resource_id,
        } => {
            let mut table = InlineTable::new();
            table.insert("kind", "karagoz".into());
            table.insert("org", org.as_str().into());
            table.insert("agent_id", agent_id.as_str().into());
            table.insert("resource_id", resource_id.as_str().into());
            value(table)
        }
    }
}

fn origin_item(o: &Origin) -> Item {
    let id = match o {
        Origin::UserTyped => "user_typed",
        Origin::Imported {
            source: ImportSource::PgService,
        } => "imported_pg_service",
        Origin::Imported {
            source: ImportSource::Url,
        } => "imported_url",
        Origin::DeepLink => "deep_link",
        Origin::Mcp => "mcp",
        Origin::PerdeManaged { org } => {
            let mut table = InlineTable::new();
            table.insert("kind", "perde_managed".into());
            table.insert("org", org.as_str().into());
            return value(table);
        }
    };
    value(id)
}

fn extra_item(extra: &[(String, String)]) -> Item {
    let mut array = Array::new();
    for (k, v) in extra {
        let mut pair = InlineTable::new();
        pair.insert("key", k.as_str().into());
        pair.insert("value", v.as_str().into());
        array.push(pair);
    }
    value(array)
}
