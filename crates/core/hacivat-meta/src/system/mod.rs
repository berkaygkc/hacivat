//! The system-catalog layer (HV-INTRO-16 consumer side; L7, D-017, R-M4-6, R-M4-33): the
//! built-in objects of each supported server version, decoded from the shipped snapshots in
//! `assets/system-catalogs/<dbms>-<ver>.hvsnap.zst`. Selection is exact (PostgreSQL by major,
//! MySQL and MariaDB by major.minor); SQLite never uses a snapshot.
//!
//! M4 Task 26:
//! - [`SystemCatalogs::open`] reads a directory's `manifest.toml` (tests pass
//!   `<repo>/assets/system-catalogs`, the app its resources directory); with feature
//!   `embedded-system` (the app only, `EmbeddedSystemOnlyFromApp`)
//!   [`SystemCatalogs::from_embedded`] reads the `include_bytes!` table of the same files.
//!   Neither reads or decodes a snapshot: [`SystemCatalogs::for_server`] decodes a file on its
//!   first use, after checking its manifest digest, and every later caller (and every clone of
//!   the set) shares that one `Arc`. A file that is gone, truncated, tampered with or not
//!   decodable serves nothing (`None`: the version's system schemas are live-introspected or
//!   `Unknown` offline), never a panic.
//! - The shipped virtual foreign keys of the system catalogs (`virtual-fks.toml`, R-M4-12):
//!   hand-written rows, each citing its vendor documentation section; outside the manifest.
//! - [`miss_is_fact`]: a snapshot read from server version *v* proves an absence for a
//!   MySQL/MariaDB server of the same line at a patch release up to *v* (alg/02 HV-INTRO-16
//!   edge 1), and for any PostgreSQL release of *v*'s major (ruling R-W5-3).
//!
//! The manifest and `virtual-fks.toml` are read by a strict reader of the TOML subset they use
//! (`mini_toml`): this crate has no TOML dependency (Task 1a's manifest table is exhaustive).

#[cfg(feature = "system")]
pub mod decode;
#[cfg(feature = "embedded-system")]
pub mod embedded;
#[cfg(feature = "system")]
pub mod normalize;

use std::collections::BTreeMap;
use std::fmt;
use std::sync::{Arc, OnceLock};

use hacivat_proto::DbmsKind;

#[cfg(feature = "system")]
pub use decode::decode;
// ## m4-t23-minicat (named exception in Task 26's module): the checked JSON payload.
#[cfg(feature = "system")]
pub use decode::decode_json;
#[cfg(feature = "system")]
pub use normalize::normalize;

use crate::ids::NsKey;
use crate::name::Name;
use crate::namespace::NamespaceSnapshot;
// ## m4-t23-minicat
use crate::namespace::Obj;
use crate::overlay::{Overlays, ShippedFk};
use sql_catalog::SettingInfo;

/// The built-in objects of one server line, as namespaces flagged `PRE_INTROSPECTED`.
#[derive(Debug, Clone)]
pub struct SystemSnapshot {
    pub dbms: DbmsKind,
    /// The snapshot's line (`17`, `8.4`, `11.4`).
    pub line: Name,
    /// The exact server version it was generated from.
    pub server_version: Name,
    /// The canonical JSON format number (1).
    pub format: u32,
    pub namespaces: BTreeMap<NsKey, Arc<NamespaceSnapshot>>,
    // ## m4-t23-minicat (named exception in Task 26's module): the side tables of a snapshot.
    /// MySQL/MariaDB built-in functions (the generator's name probe): schema-less `Routine`
    /// objects, flagged `SYSTEM | PRE_INTROSPECTED`, with `args = None` (name only, never "no
    /// arguments"). Empty for PostgreSQL, whose built-ins live in `pg_catalog`.
    pub builtins: Vec<Obj>,
    /// PostgreSQL operators with their operand, result and function types.
    pub operators: Arc<[SystemOperator]>,
    /// PostgreSQL types with their SQL spelling, kind, category and array element.
    pub types: Arc<[SystemType]>,
    /// Setting names (PostgreSQL GUCs with type and context; MySQL/MariaDB variables, MariaDB
    /// with scope as `context` and type), never their values; sorted by name.
    pub settings: Arc<[SettingInfo]>,
}

// ## m4-t23-minicat (named exception in Task 26's module)
/// One built-in operator of a snapshot (PostgreSQL).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SystemOperator {
    pub schema: Name,
    pub name: Name,
    /// `infix`, `prefix` or `postfix`.
    pub kind: Name,
    pub left: Option<Name>,
    pub right: Option<Name>,
    pub result: Name,
    /// The implementing function, as the server prints it.
    pub function: Name,
    pub comment: Option<Name>,
}

/// One built-in type of a snapshot (PostgreSQL).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SystemType {
    pub schema: Name,
    pub name: Name,
    /// The SQL spelling (`double precision` for `float8`).
    pub sql_name: Name,
    /// `base`, `composite`, `domain`, `enum`, `pseudo`, `range` or `multirange`.
    pub kind: Name,
    /// `pg_type.typcategory`, one letter.
    pub category: Name,
    /// The element type of an array type.
    pub element: Option<Name>,
    pub comment: Option<Name>,
}

/// Why a snapshot set cannot be read.
#[derive(Debug)]
pub enum SystemError {
    Io(std::io::Error),
    /// Not a snapshot, a bad frame, a checksum or digest mismatch, a truncated asset.
    Corrupt(&'static str),
    /// A format number this build does not read.
    UnknownFormat(u32),
    /// Not written yet (M4 Task 1a stub).
    NotImplemented,
}

impl fmt::Display for SystemError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SystemError::Io(e) => write!(f, "system catalog i/o: {e}"),
            SystemError::Corrupt(why) => write!(f, "corrupt system catalog: {why}"),
            SystemError::UnknownFormat(n) => write!(f, "system catalog format {n}"),
            SystemError::NotImplemented => write!(f, "not implemented"),
        }
    }
}

impl std::error::Error for SystemError {}

// ## m4-t26-overlays

impl SystemSnapshot {
    /// The namespace of system schema `schema` (exact name), if the snapshot holds it.
    pub fn namespace(&self, schema: &str) -> Option<&Arc<NamespaceSnapshot>> {
        self.namespaces
            .iter()
            .find(|(k, _)| k.path.len() == 1 && k.root_name() == schema)
            .map(|(_, ns)| ns)
    }
}

/// The name of the manifest in a snapshot directory.
pub const MANIFEST: &str = "manifest.toml";
/// The name of the shipped virtual foreign keys in a snapshot directory.
pub const VIRTUAL_FKS: &str = "virtual-fks.toml";
/// The largest manifest or `virtual-fks.toml` read.
#[cfg(feature = "system")]
const MAX_TEXT_BYTES: u64 = 1024 * 1024;
/// The largest snapshot file read (the decoder bounds the JSON it holds; a snapshot file is
/// well under 1 MB).
#[cfg(feature = "system")]
const MAX_FILE_BYTES: u64 = 16 * 1024 * 1024;

/// The leading `d+(.d+)*` numbers of a version text, at most three.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Parts {
    n: [u32; 3],
    len: u8,
}

impl Parts {
    fn parse(text: &str) -> Option<Parts> {
        let mut parts = Parts { n: [0; 3], len: 0 };
        for piece in text.split('.') {
            let digits = piece.bytes().take_while(u8::is_ascii_digit).count();
            if digits == 0 || usize::from(parts.len) == parts.n.len() {
                break;
            }
            parts.n[usize::from(parts.len)] = piece[..digits].parse().ok()?;
            parts.len += 1;
            if digits != piece.len() {
                // `17beta1`, `8.0.40-log`: the number ends inside this piece.
                break;
            }
        }
        (parts.len > 0).then_some(parts)
    }
}

/// The parts of a server's version text that name its line: PostgreSQL its major (`17` of
/// `17.2 (Debian …)`), MySQL and MariaDB major.minor (`10.11` of `5.5.5-10.11.9-MariaDB`,
/// whose replication prefix is dropped). `None` for another engine or an unreadable version.
fn line_parts(dbms: DbmsKind, version: &str) -> Option<(Parts, usize)> {
    let text = version.trim();
    let (text, want) = match dbms {
        DbmsKind::Postgres => (text, 1),
        DbmsKind::MySql => (text, 2),
        DbmsKind::MariaDb => (text.strip_prefix("5.5.5-").unwrap_or(text), 2),
        _ => return None,
    };
    let parts = Parts::parse(text)?;
    (usize::from(parts.len) >= want).then_some((parts, want))
}

/// The full patch-level parts a version must have to be compared with a snapshot's:
/// PostgreSQL `major.minor`, MySQL and MariaDB `major.minor.patch`.
fn patch_parts(dbms: DbmsKind, version: &str) -> Option<Parts> {
    let (parts, _) = line_parts(dbms, version)?;
    let want = if dbms == DbmsKind::Postgres { 2 } else { 3 };
    (usize::from(parts.len) == want).then_some(parts)
}

/// Whether a name the snapshot read from `snapshot_version` lacks is certainly absent on a
/// server reporting `live_version`.
///
/// - **MySQL and MariaDB:** only at the same line and a patch release up to the snapshot's
///   (alg/02 HV-INTRO-16 edge 1: their minor releases add functions and system tables, so a
///   newer patch makes a miss `Unknown`).
/// - **PostgreSQL** (ruling R-W5-3): at any release of the snapshot's major. PostgreSQL keeps
///   its system catalogs fixed within a major version: the catalog version (`catversion`)
///   changes only between majors, and a minor release never needs `initdb` (PostgreSQL
///   documentation, "Upgrading a PostgreSQL Cluster"). A development or beta build
///   (`17devel`, `18beta1`, no `M.m` form) or another major is not covered.
///
/// An unreadable version is never covered.
pub fn miss_is_fact(dbms: DbmsKind, live_version: &str, snapshot_version: &str) -> bool {
    match (
        patch_parts(dbms, live_version),
        patch_parts(dbms, snapshot_version),
    ) {
        (Some(live), Some(snap)) if dbms == DbmsKind::Postgres => live.n[0] == snap.n[0],
        (Some(live), Some(snap)) => live.n[..2] == snap.n[..2] && live <= snap,
        _ => false,
    }
}

/// Where a snapshot file's bytes come from.
enum Source {
    #[cfg(feature = "system")]
    File(std::path::PathBuf),
    #[cfg(feature = "embedded-system")]
    Embedded(&'static [u8]),
    /// Listed in the manifest, absent from the embedded table: serves nothing.
    #[cfg(feature = "embedded-system")]
    Missing,
}

/// One manifest row and its lazily decoded snapshot.
struct Entry {
    dbms: DbmsKind,
    line: Name,
    line_parts: Parts,
    file: Name,
    /// The manifest's XXH3-64 of the file.
    #[cfg(feature = "system")]
    digest: u64,
    source: Source,
    /// `None` inside: the file could not be read, verified or decoded (decided once).
    decoded: OnceLock<Option<Arc<SystemSnapshot>>>,
}

impl fmt::Debug for Entry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Entry")
            .field("dbms", &self.dbms)
            .field("line", &self.line)
            .field("file", &self.file)
            .field("decoded", &self.decoded.get().map(Option::is_some))
            .finish_non_exhaustive()
    }
}

impl Entry {
    /// The decoded snapshot, decoding it on first use.
    fn snapshot(&self) -> Option<Arc<SystemSnapshot>> {
        self.decoded
            .get_or_init(|| self.load().map(Arc::new))
            .clone()
    }

    #[cfg(feature = "system")]
    fn load(&self) -> Option<SystemSnapshot> {
        let owned;
        let bytes: &[u8] = match &self.source {
            Source::File(path) => {
                owned = read_bounded(path, MAX_FILE_BYTES).ok()?;
                &owned
            }
            #[cfg(feature = "embedded-system")]
            Source::Embedded(bytes) => bytes,
            #[cfg(feature = "embedded-system")]
            Source::Missing => return None,
        };
        if twox_hash::XxHash3_64::oneshot(bytes) != self.digest {
            return None;
        }
        let snap = decode::decode(bytes).ok()?;
        (snap.dbms == self.dbms && snap.line == self.line).then_some(snap)
    }

    #[cfg(not(feature = "system"))]
    fn load(&self) -> Option<SystemSnapshot> {
        match self.source {}
    }
}

/// The snapshot set an app ships or a test opens.
#[derive(Debug, Clone, Default)]
pub struct SystemCatalogs {
    entries: Arc<[Arc<Entry>]>,
    shipped: Arc<[ShippedFk]>,
}

impl SystemCatalogs {
    /// A set with no snapshot: every server is live-introspected or `Unknown` offline.
    pub fn empty() -> Self {
        SystemCatalogs::default()
    }

    /// The snapshots in `dir`: its `manifest.toml` (required) and `virtual-fks.toml` (if
    /// present). No snapshot file is read here; each is read, checked against its manifest
    /// digest and decoded on first use ([`SystemCatalogs::for_server`]).
    #[cfg(feature = "system")]
    pub fn open(dir: &std::path::Path) -> Result<Self, SystemError> {
        let manifest = read_text(&dir.join(MANIFEST))?;
        let rows = parse_manifest(&manifest)?;
        let fks_path = dir.join(VIRTUAL_FKS);
        let shipped = match std::fs::symlink_metadata(&fks_path) {
            Ok(_) => parse_virtual_fks(&read_text(&fks_path)?)?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Arc::from([]),
            Err(e) => return Err(SystemError::Io(e)),
        };
        let entries = rows
            .into_iter()
            .map(|r| {
                let source = Source::File(dir.join(&*r.file));
                Arc::new(r.into_entry(source))
            })
            .collect();
        Ok(SystemCatalogs { entries, shipped })
    }

    /// The snapshots embedded in the binary (feature `embedded-system`, the app only). As
    /// [`SystemCatalogs::open`]: each file is decoded on its first use. An embedded table that
    /// does not read (a build defect a test catches) gives the empty set.
    #[cfg(feature = "embedded-system")]
    pub fn from_embedded() -> Self {
        Self::from_table(
            embedded::MANIFEST,
            embedded::SNAPSHOTS,
            embedded::VIRTUAL_FKS,
        )
        .unwrap_or_default()
    }

    #[cfg(feature = "embedded-system")]
    fn from_table(
        manifest: &str,
        files: &'static [(&'static str, &'static [u8])],
        virtual_fks: &str,
    ) -> Result<Self, SystemError> {
        let rows = parse_manifest(manifest)?;
        let shipped = parse_virtual_fks(virtual_fks)?;
        let entries = rows
            .into_iter()
            .map(|r| {
                let source = files
                    .iter()
                    .find(|(name, _)| *name == &*r.file)
                    .map_or(Source::Missing, |(_, bytes)| Source::Embedded(bytes));
                Arc::new(r.into_entry(source))
            })
            .collect();
        Ok(SystemCatalogs { entries, shipped })
    }

    /// The snapshot of `dbms` at `version` (exact line match, R-M4-6), if shipped and
    /// readable: PostgreSQL by major, MySQL and MariaDB by major.minor, never SQLite or another
    /// engine. Decoded on first use; every caller shares one `Arc`.
    pub fn for_server(&self, dbms: DbmsKind, version: &str) -> Option<Arc<SystemSnapshot>> {
        let (parts, want) = line_parts(dbms, version)?;
        self.entries
            .iter()
            .find(|e| {
                e.dbms == dbms
                    && usize::from(e.line_parts.len) == want
                    && e.line_parts.n[..want] == parts.n[..want]
            })?
            .snapshot()
    }

    /// The layers a catalog of `dbms` at `version` gets from this set: the version's snapshot
    /// and, with it, the shipped virtual foreign keys of that engine (the rows name system
    /// tables, which only a snapshot holds). Empty without a snapshot.
    pub fn overlays_for(&self, dbms: DbmsKind, version: &str) -> Overlays {
        match self.for_server(dbms, version) {
            Some(system) => Overlays {
                system: Some(system),
                shipped_fks: self.shipped_fks(dbms),
                ..Overlays::default()
            },
            None => Overlays::default(),
        }
    }

    /// The shipped virtual foreign keys of `dbms`.
    pub fn shipped_fks(&self, dbms: DbmsKind) -> Arc<[ShippedFk]> {
        self.shipped
            .iter()
            .filter(|f| f.dbms == dbms)
            .cloned()
            .collect()
    }

    /// The lines this set lists, in manifest order.
    pub fn lines(&self) -> Vec<(DbmsKind, Name)> {
        self.entries
            .iter()
            .map(|e| (e.dbms, e.line.clone()))
            .collect()
    }

    /// How many files have been decoded (or found unreadable) so far.
    pub fn decoded_count(&self) -> usize {
        self.entries
            .iter()
            .filter(|e| e.decoded.get().is_some())
            .count()
    }
}

/// One `[[snapshot]]` row of a manifest.
#[cfg(feature = "system")]
struct ManifestRow {
    dbms: DbmsKind,
    line: Name,
    line_parts: Parts,
    file: Name,
    digest: u64,
}

#[cfg(feature = "system")]
impl ManifestRow {
    fn into_entry(self, source: Source) -> Entry {
        Entry {
            dbms: self.dbms,
            line: self.line,
            line_parts: self.line_parts,
            file: self.file,
            digest: self.digest,
            source,
            decoded: OnceLock::new(),
        }
    }
}

#[cfg(feature = "system")]
fn corrupt(why: &'static str) -> SystemError {
    SystemError::Corrupt(why)
}

/// `path`, at most `limit` bytes.
#[cfg(feature = "system")]
fn read_bounded(path: &std::path::Path, limit: u64) -> Result<Vec<u8>, SystemError> {
    use std::io::Read;
    let file = std::fs::File::open(path).map_err(SystemError::Io)?;
    let mut bytes = Vec::new();
    file.take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(SystemError::Io)?;
    if bytes.len() as u64 > limit {
        return Err(corrupt("file too large"));
    }
    Ok(bytes)
}

#[cfg(feature = "system")]
fn read_text(path: &std::path::Path) -> Result<String, SystemError> {
    String::from_utf8(read_bounded(path, MAX_TEXT_BYTES)?).map_err(|_| corrupt("not UTF-8"))
}

#[cfg(feature = "system")]
fn dbms_named(text: &str) -> Option<DbmsKind> {
    match text {
        "postgres" => Some(DbmsKind::Postgres),
        "mysql" => Some(DbmsKind::MySql),
        "mariadb" => Some(DbmsKind::MariaDb),
        _ => None,
    }
}

/// The rows of a manifest (`xtask minicat build`'s output): top-level `format = 1`, one
/// `[[snapshot]]` table per file with at least `file`, `dbms`, `line`, `format` and
/// `zst_digest`. The file must be named `<dbms>-<line>.hvsnap.zst` (no other path), the line
/// must be the engine's (one number for PostgreSQL, two for MySQL and MariaDB), and no line
/// may repeat. The generator's other keys are not read.
#[cfg(feature = "system")]
fn parse_manifest(text: &str) -> Result<Vec<ManifestRow>, SystemError> {
    use mini_toml::Value;
    let doc = mini_toml::parse(text)?;
    match doc.get_top("format") {
        Some(Value::Int(1)) => {}
        Some(Value::Int(n)) => {
            return Err(SystemError::UnknownFormat(u32::try_from(*n).unwrap_or(0)));
        }
        _ => return Err(corrupt("manifest: no format")),
    }
    let mut rows: Vec<ManifestRow> = Vec::new();
    for (table, keys) in &doc.arrays {
        if table != "snapshot" {
            return Err(corrupt("manifest: unknown table"));
        }
        let text_of = |k: &str| match mini_toml::get(keys, k) {
            Some(Value::Str(s)) => Ok(s.as_str()),
            _ => Err(corrupt("manifest: a snapshot lacks a key")),
        };
        match mini_toml::get(keys, "format") {
            Some(Value::Int(1)) => {}
            Some(Value::Int(n)) => {
                return Err(SystemError::UnknownFormat(u32::try_from(*n).unwrap_or(0)));
            }
            _ => return Err(corrupt("manifest: a snapshot lacks its format")),
        }
        let dbms_text = text_of("dbms")?;
        let dbms = dbms_named(dbms_text).ok_or(corrupt("manifest: unknown dbms"))?;
        let line = text_of("line")?;
        if line.is_empty() || !line.bytes().all(|b| b.is_ascii_digit() || b == b'.') {
            return Err(corrupt("manifest: bad line"));
        }
        let parts = Parts::parse(line).ok_or(corrupt("manifest: bad line"))?;
        let want = if dbms == DbmsKind::Postgres { 1 } else { 2 };
        if usize::from(parts.len) != want || line.split('.').count() != want {
            return Err(corrupt("manifest: bad line"));
        }
        let file = text_of("file")?;
        if file != format!("{dbms_text}-{line}.hvsnap.zst") {
            return Err(corrupt("manifest: a file named outside the rule"));
        }
        let digest = text_of("zst_digest")?
            .strip_prefix("xxh3-64:")
            .filter(|h| h.len() == 16)
            .and_then(|h| u64::from_str_radix(h, 16).ok())
            .ok_or(corrupt("manifest: bad digest"))?;
        if rows.iter().any(|r| r.dbms == dbms && r.line_parts == parts) {
            return Err(corrupt("manifest: a line listed twice"));
        }
        rows.push(ManifestRow {
            dbms,
            line: Name::from(line),
            line_parts: parts,
            file: Name::from(file),
            digest,
        });
    }
    Ok(rows)
}

/// The rows of `virtual-fks.toml` (R-M4-12): `[[fk]]` tables with exactly `dbms`, `from`
/// and `to` (`schema.relation`), `from_columns` and `to_columns` (non-empty, the same length,
/// in FK order), a non-blank `source` (the vendor documentation section the row is taken from)
/// and an optional `name`. Anything else is refused: no row without its source.
#[cfg(feature = "system")]
fn parse_virtual_fks(text: &str) -> Result<Arc<[ShippedFk]>, SystemError> {
    use crate::ids::ObjectKey;
    use crate::kind::ObjectKind;
    use crate::overlay::VirtualFk;
    use mini_toml::Value;

    const KEYS: [&str; 7] = [
        "dbms",
        "from",
        "from_columns",
        "to",
        "to_columns",
        "source",
        "name",
    ];
    let bad = || corrupt("virtual-fks: a bad row");
    let doc = mini_toml::parse(text)?;
    if !doc.top.is_empty() {
        return Err(corrupt("virtual-fks: a key outside a row"));
    }
    let relation = |text: &str| -> Result<ObjectKey, SystemError> {
        let (schema, name) = text.split_once('.').ok_or(bad())?;
        if schema.is_empty() || name.is_empty() || name.contains('.') {
            return Err(bad());
        }
        Ok(ObjectKey {
            kind: ObjectKind::Table,
            path: [Name::from(schema), Name::from(name)].into_iter().collect(),
            server_id: None,
            overload_sig: None,
            raw_kind: None,
        })
    };
    let mut out = Vec::new();
    for (table, keys) in &doc.arrays {
        if table != "fk" {
            return Err(corrupt("virtual-fks: unknown table"));
        }
        if keys.iter().any(|(k, _)| !KEYS.contains(&k.as_str())) {
            return Err(corrupt("virtual-fks: unknown key"));
        }
        let text_of = |k: &str| match mini_toml::get(keys, k) {
            Some(Value::Str(s)) if !s.trim().is_empty() => Ok(s.as_str()),
            _ => Err(bad()),
        };
        let columns = |k: &str| -> Result<Vec<String>, SystemError> {
            let Some(Value::Array(items)) = mini_toml::get(keys, k) else {
                return Err(bad());
            };
            let cols = items
                .iter()
                .map(|v| match v {
                    Value::Str(s) if !s.is_empty() => Ok(s.clone()),
                    _ => Err(bad()),
                })
                .collect::<Result<Vec<_>, _>>()?;
            if cols.is_empty() {
                return Err(bad());
            }
            Ok(cols)
        };
        let dbms = dbms_named(text_of("dbms")?).ok_or(bad())?;
        let (from_cols, to_cols) = (columns("from_columns")?, columns("to_columns")?);
        if from_cols.len() != to_cols.len() {
            return Err(bad());
        }
        let name = match mini_toml::get(keys, "name") {
            None => None,
            Some(Value::Str(s)) if !s.is_empty() => Some(s.clone()),
            Some(_) => return Err(bad()),
        };
        out.push(ShippedFk {
            fk: VirtualFk {
                from: relation(text_of("from")?)?,
                from_cols,
                to: relation(text_of("to")?)?,
                to_cols,
                name,
            },
            dbms,
            source: Name::from(text_of("source")?.trim()),
        });
    }
    Ok(out.into())
}

/// A strict reader of the TOML subset of the manifest and `virtual-fks.toml`: comments, bare
/// keys, `key = value` with basic or literal strings, integers, booleans and (possibly
/// multi-line) arrays of them, and `[[table]]` array headers. Anything else is refused; a key
/// may not repeat within its table.
#[cfg(feature = "system")]
mod mini_toml {
    use super::{SystemError, corrupt};

    #[derive(Debug, Clone, PartialEq)]
    pub enum Value {
        Str(String),
        Int(i64),
        Bool(bool),
        Array(Vec<Value>),
    }

    pub type Table = Vec<(String, Value)>;

    #[derive(Debug, Default)]
    pub struct Doc {
        /// The keys before the first `[[table]]`.
        pub top: Table,
        /// Each `[[name]]` table, in order.
        pub arrays: Vec<(String, Table)>,
    }

    impl Doc {
        pub fn get_top(&self, key: &str) -> Option<&Value> {
            get(&self.top, key)
        }
    }

    pub fn get<'a>(table: &'a Table, key: &str) -> Option<&'a Value> {
        table.iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }

    struct Parser<'a> {
        s: &'a [u8],
        i: usize,
    }

    fn bad() -> SystemError {
        corrupt("not the TOML subset this reader takes")
    }

    pub fn parse(text: &str) -> Result<Doc, SystemError> {
        let mut p = Parser {
            s: text.as_bytes(),
            i: 0,
        };
        let mut doc = Doc::default();
        loop {
            p.skip_blank();
            let Some(c) = p.peek() else { break };
            if c == b'[' {
                p.expect(b"[[")?;
                let name = p.bare_key()?;
                p.expect(b"]]")?;
                p.end_of_line()?;
                doc.arrays.push((name, Vec::new()));
                continue;
            }
            let key = p.bare_key()?;
            p.skip_inline();
            p.expect(b"=")?;
            p.skip_inline();
            let value = p.value(0)?;
            p.end_of_line()?;
            let table = match doc.arrays.last_mut() {
                Some((_, t)) => t,
                None => &mut doc.top,
            };
            if table.iter().any(|(k, _)| *k == key) {
                return Err(corrupt("a key repeats"));
            }
            table.push((key, value));
        }
        Ok(doc)
    }

    impl Parser<'_> {
        fn peek(&self) -> Option<u8> {
            self.s.get(self.i).copied()
        }

        fn skip_inline(&mut self) {
            while matches!(self.peek(), Some(b' ' | b'\t')) {
                self.i += 1;
            }
        }

        fn skip_comment(&mut self) {
            if self.peek() == Some(b'#') {
                while !matches!(self.peek(), None | Some(b'\n')) {
                    self.i += 1;
                }
            }
        }

        /// Whitespace, newlines and comments.
        fn skip_blank(&mut self) {
            loop {
                while matches!(self.peek(), Some(b' ' | b'\t' | b'\r' | b'\n')) {
                    self.i += 1;
                }
                if self.peek() == Some(b'#') {
                    self.skip_comment();
                } else {
                    return;
                }
            }
        }

        fn end_of_line(&mut self) -> Result<(), SystemError> {
            self.skip_inline();
            self.skip_comment();
            if self.peek() == Some(b'\r') {
                self.i += 1;
            }
            match self.peek() {
                None => Ok(()),
                Some(b'\n') => {
                    self.i += 1;
                    Ok(())
                }
                Some(_) => Err(bad()),
            }
        }

        fn expect(&mut self, token: &[u8]) -> Result<(), SystemError> {
            if self.s[self.i..].starts_with(token) {
                self.i += token.len();
                Ok(())
            } else {
                Err(bad())
            }
        }

        fn bare_key(&mut self) -> Result<String, SystemError> {
            let start = self.i;
            while matches!(self.peek(), Some(c) if c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
            {
                self.i += 1;
            }
            if self.i == start {
                return Err(bad());
            }
            Ok(String::from_utf8_lossy(&self.s[start..self.i]).into_owned())
        }

        fn value(&mut self, depth: u8) -> Result<Value, SystemError> {
            match self.peek().ok_or_else(bad)? {
                b'"' => self.basic_string().map(Value::Str),
                b'\'' => self.literal_string().map(Value::Str),
                b'[' if depth < 4 => self.array(depth + 1),
                b't' => self.expect(b"true").map(|()| Value::Bool(true)),
                b'f' => self.expect(b"false").map(|()| Value::Bool(false)),
                b'-' | b'0'..=b'9' => self.integer(),
                _ => Err(bad()),
            }
        }

        fn array(&mut self, depth: u8) -> Result<Value, SystemError> {
            self.i += 1;
            let mut items = Vec::new();
            loop {
                self.skip_blank();
                if self.peek() == Some(b']') {
                    self.i += 1;
                    return Ok(Value::Array(items));
                }
                items.push(self.value(depth)?);
                self.skip_blank();
                match self.peek() {
                    Some(b',') => self.i += 1,
                    Some(b']') => {}
                    _ => return Err(bad()),
                }
            }
        }

        fn integer(&mut self) -> Result<Value, SystemError> {
            let start = self.i;
            if self.peek() == Some(b'-') {
                self.i += 1;
            }
            while matches!(self.peek(), Some(b'0'..=b'9')) {
                self.i += 1;
            }
            std::str::from_utf8(&self.s[start..self.i])
                .ok()
                .and_then(|t| t.parse().ok())
                .map(Value::Int)
                .ok_or_else(bad)
        }

        fn literal_string(&mut self) -> Result<String, SystemError> {
            self.i += 1;
            let start = self.i;
            loop {
                match self.peek() {
                    Some(b'\'') => break,
                    Some(b'\n') | None => return Err(bad()),
                    Some(_) => self.i += 1,
                }
            }
            let text = std::str::from_utf8(&self.s[start..self.i]).map_err(|_| bad())?;
            self.i += 1;
            Ok(text.to_owned())
        }

        fn basic_string(&mut self) -> Result<String, SystemError> {
            self.i += 1;
            let mut out: Vec<u8> = Vec::new();
            loop {
                let c = self.peek().ok_or_else(bad)?;
                self.i += 1;
                match c {
                    b'"' => break,
                    b'\\' => {
                        let e = self.peek().ok_or_else(bad)?;
                        self.i += 1;
                        let ch = match e {
                            b'"' => '"',
                            b'\\' => '\\',
                            b'n' => '\n',
                            b't' => '\t',
                            b'r' => '\r',
                            b'b' => '\u{8}',
                            b'f' => '\u{c}',
                            b'u' => self.unicode(4)?,
                            b'U' => self.unicode(8)?,
                            _ => return Err(bad()),
                        };
                        let mut buf = [0u8; 4];
                        out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
                    }
                    // Control characters other than tab are not allowed in a basic string.
                    c if c < 0x20 && c != b'\t' => return Err(bad()),
                    0x7f => return Err(bad()),
                    c => out.push(c),
                }
            }
            String::from_utf8(out).map_err(|_| bad())
        }

        fn unicode(&mut self, digits: usize) -> Result<char, SystemError> {
            let hex = self.s.get(self.i..self.i + digits).ok_or_else(bad)?;
            self.i += digits;
            let hex = std::str::from_utf8(hex).map_err(|_| bad())?;
            u32::from_str_radix(hex, 16)
                .ok()
                .and_then(char::from_u32)
                .ok_or_else(bad)
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn reads_the_subset_and_refuses_the_rest() {
            let doc = parse(
                "# c\nformat = 1\n\n[[t]]\na = \"x # not a comment\" # c\nb = ['lit', \"\\u00e7\"]\n\
                 c = [\n  1,\n  -2, # c\n]\nd = true\n[[t]]\na = \"\"\n",
            )
            .unwrap();
            assert_eq!(doc.get_top("format"), Some(&Value::Int(1)));
            assert_eq!(doc.arrays.len(), 2);
            let t = &doc.arrays[0].1;
            assert_eq!(get(t, "a"), Some(&Value::Str("x # not a comment".into())));
            assert_eq!(
                get(t, "b"),
                Some(&Value::Array(vec![
                    Value::Str("lit".into()),
                    Value::Str("ç".into())
                ]))
            );
            assert_eq!(
                get(t, "c"),
                Some(&Value::Array(vec![Value::Int(1), Value::Int(-2)]))
            );
            assert_eq!(get(t, "d"), Some(&Value::Bool(true)));
            for bad in [
                "a = 1\na = 2\n",
                "[t]\na = 1\n",
                "a = \"open\n",
                "a = 1 b = 2\n",
                "a.b = 1\n",
                "a = { x = 1 }\n",
                "a = \"\\q\"\n",
                "a = [1 2]\n",
                "a = \"\u{1}\"\n",
                "a = 1.5\n",
                "a = [[[[[[1]]]]]]\n",
            ] {
                assert!(parse(bad).is_err(), "{bad:?}");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_parts_follow_the_engine_rule() {
        let pg = |v| line_parts(DbmsKind::Postgres, v).map(|(p, w)| p.n[..w].to_vec());
        assert_eq!(pg("17.11"), Some(vec![17]));
        assert_eq!(pg("17.2 (Debian 17.2-1.pgdg120+1)"), Some(vec![17]));
        assert_eq!(pg("18beta1"), Some(vec![18]));
        assert_eq!(pg(""), None);
        assert_eq!(pg("PostgreSQL 17"), None);
        let my = |d, v| line_parts(d, v).map(|(p, w)| p.n[..w].to_vec());
        assert_eq!(my(DbmsKind::MySql, "8.0.40-log"), Some(vec![8, 0]));
        assert_eq!(my(DbmsKind::MySql, "9"), None);
        assert_eq!(
            my(DbmsKind::MariaDb, "5.5.5-10.11.9-MariaDB"),
            Some(vec![10, 11])
        );
        assert_eq!(my(DbmsKind::Sqlite, "3.46.0"), None);
        assert_eq!(my(DbmsKind::Cockroach, "17.1"), None);
    }

    #[test]
    fn a_miss_is_a_fact_by_the_engines_rule() {
        let pg = DbmsKind::Postgres;
        assert!(miss_is_fact(pg, "17.12", "17.12"));
        assert!(miss_is_fact(pg, "17.2 (Debian 17.2-1.pgdg120+1)", "17.12"));
        // R-W5-3: any release of the same PostgreSQL major.
        assert!(miss_is_fact(pg, "17.13", "17.12"));
        assert!(miss_is_fact(pg, "17.99", "17.12"));
        assert!(!miss_is_fact(pg, "18beta1", "18.7"));
        assert!(!miss_is_fact(pg, "16.1", "17.12"));
        assert!(!miss_is_fact(pg, "18.0", "17.12"));
        assert!(!miss_is_fact(pg, "17", "17.12"));
        assert!(!miss_is_fact(pg, "17devel", "17.12"));
        assert!(!miss_is_fact(pg, "", "17.12"));
        let my = DbmsKind::MySql;
        assert!(miss_is_fact(my, "8.4.3", "8.4.13"));
        assert!(!miss_is_fact(my, "8.4.14", "8.4.13"));
        assert!(!miss_is_fact(my, "8.0.3", "8.4.13"));
        assert!(miss_is_fact(
            DbmsKind::MariaDb,
            "5.5.5-10.11.9-MariaDB",
            "10.11.19"
        ));
        assert!(!miss_is_fact(
            DbmsKind::MariaDb,
            "10.11.20-MariaDB",
            "10.11.19"
        ));
    }

    #[test]
    fn a_set_without_the_system_feature_serves_nothing() {
        assert!(
            SystemCatalogs::empty()
                .for_server(DbmsKind::Postgres, "17.11")
                .is_none()
        );
        assert_eq!(SystemCatalogs::empty().decoded_count(), 0);
    }
}
