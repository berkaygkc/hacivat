//! The library registry `<config>/libraries.toml` (HV-SCRIPT-01 steps 1, 2, 5; R-M4b-2).
//!
//! ## m4b-t01a-seams
//! Task 1a: the types. Task 3 writes the registry (one writer, `fsio::atomic_write`, 0600).
//!
//! ## m4b-t03-registry
//! Task 3: [`LibraryRegistry`], the **single writer** of `libraries.toml`:
//!
//! ```toml
//! version = 1
//!
//! [[library]]
//! id = "<uuid v4>"
//! path = "<absolute, as the user chose it>"
//! added_at = "<RFC 3339 UTC>"
//! canonical = "<the resolved root>"   # m4b-t10 fix round 2; set by the service
//! inode = "<the root folder's inode>" # m4b-t10 fix round 2; Unix only
//!
//! [[mapping]]
//! profile = "<profile uuid>"
//! database = "<the server's exact name>"
//! library = "<library uuid>"
//! ```
//!
//! The file holds only those keys (by type: no host, user, keyring slot or password). It is
//! written through `fsio::atomic_write` (temp, `fsync`, rename; the file 0600 and a created
//! folder 0700, spec §5.6). Loading follows the profile store's rules: a missing file is an
//! empty registry; one that is not TOML, or has a malformed `version` or table shape, is set
//! aside (`fsio::quarantine(path, "corrupt", now)`); a newer `version` or a file that cannot be
//! read (or set aside) makes the registry read-only, so it is never overwritten. A bad row is
//! dropped with a [`RegistryDiag`]; so is a mapping whose library is not registered. Rows past
//! [`MAX_LIBRARIES`] are dropped.
//!
//! Attaching ([`LibraryRegistry::add`]) compares canonical paths and refuses a filesystem root,
//! the home folder, the app's own folders (or a folder inside or containing one), a folder
//! already registered under another spelling, and a folder inside or containing another library
//! (HV-SCRIPT-01 step 2). A missing folder stays registered (step 5): its
//! [`RegisteredLibrary::root`] fails and the service shows it unavailable.

use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use hacivat_proto::ProfileId;
use hacivat_settings::LocalPaths;
use hacivat_settings::fsio;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use uuid::Uuid;

use crate::manifest;
use crate::names::DatabaseName;

/// A registered library's stable ID (a v4 UUID, never reused). Serialized as its hyphenated
/// text (`libraries.toml`, `workspace.json`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct LibraryId(pub Uuid);

// ## m4b-t01a-fr1 (review M4): no `Default`. A `#[serde(default)]` or `..Default::default()`
// would silently mint a new library identity; a fresh ID is always an explicit `new()`.
#[expect(
    clippy::new_without_default,
    reason = "a library identity is never a default value (T1a review M4)"
)]
impl LibraryId {
    /// A fresh random ID.
    pub fn new() -> Self {
        LibraryId(Uuid::new_v4())
    }
}

impl fmt::Display for LibraryId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0.hyphenated(), f)
    }
}

impl Serialize for LibraryId {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for LibraryId {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        Uuid::parse_str(&s)
            .map(LibraryId)
            .map_err(serde::de::Error::custom)
    }
}

/// A registered folder: the path as the user chose it, and its canonical form, which every
/// open re-validates against (HV-SCRIPT-03 step 8).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LibraryRoot {
    pub id: LibraryId,
    pub path: PathBuf,
    pub canonical: PathBuf,
}

/// A local mapping `(profile, database) → library` that beats automatic binding for that node
/// only (R-M4b-7). Kept in the registry, never in the library.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalMapping {
    pub profile: ProfileId,
    pub database: DatabaseName,
    pub library: LibraryId,
}

/// Why a registry change was refused; the registry is unchanged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegistryError {
    /// Task 1a's service stub (Task 10 replaces it); the registry itself never answers it.
    NotImplemented,
    /// The file is newer than this build, or could not be read or set aside: nothing is
    /// written.
    ReadOnly,
    NotAbsolute,
    NotAFolder,
    /// The path is not UTF-8, so it cannot be stored.
    NotUtf8Path,
    FilesystemRoot,
    HomeFolder,
    /// The app's own config, data, cache or runtime folder, inside one, or containing one.
    AppFolder,
    /// The folder is that library already (another spelling, or a link to it).
    AlreadyRegistered(LibraryId),
    /// The folder is inside that library or contains it.
    Nested(LibraryId),
    /// [`MAX_LIBRARIES`] are registered.
    TooMany,
    UnknownLibrary,
    Io(io::ErrorKind),
}

/// The most libraries (R-M4b-14).
pub const MAX_LIBRARIES: usize = 32;

/// The registry file format this build reads and writes.
const VERSION: i64 = 1;
/// A registry over this size is not ours (32 libraries and their mappings are a few KiB).
const MAX_FILE_BYTES: u64 = 1 << 20;

/// What loading the registry found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegistryDiag {
    /// The file was not a registry; it was moved aside to this path and the registry is empty.
    SetAside(PathBuf),
    /// It could not be moved aside either: the registry is read-only.
    QuarantineFailed(io::ErrorKind),
    /// Written by a newer Hacivat: the registry is read-only and shows nothing.
    NewerVersion(i64),
    /// The file could not be read: the registry is read-only.
    Unreadable(io::ErrorKind),
    /// The `[[library]]` row at `index` was dropped.
    LibraryDropped { index: usize, problem: RowProblem },
    /// The `[[mapping]]` row at `index` was dropped.
    MappingDropped { index: usize, problem: RowProblem },
    /// The `[[mapping]]` row at `index` names no registered library; dropped.
    OrphanMapping { index: usize },
    /// More than [`MAX_LIBRARIES`] rows; the rest were dropped.
    TooManyLibraries,
    /// `libraries.toml` is not a regular file (a FIFO, a folder, a device; a link is followed
    /// to its target, which must be one): never opened, so it cannot hang the start; the
    /// registry is read-only and the file is left alone (fix round 1, review m7).
    NotAFile,
}

/// Why a row was dropped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowProblem {
    /// An ID that is not a UUID.
    BadId,
    /// A library path that is not absolute.
    RelativePath,
    /// The same library ID or path, or the same mapping node, as an earlier row.
    Duplicate,
    /// A field is missing or not a string.
    MissingField,
    /// The folder is inside an earlier row's folder or contains it (canonical paths; fix
    /// round 1, review m6). A different spelling of the same folder is a `Duplicate`.
    Nested,
}

/// A registered library as the file keeps it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegisteredLibrary {
    pub id: LibraryId,
    /// Absolute, as the user chose it.
    pub path: PathBuf,
    /// RFC 3339 UTC (`2026-11-22T10:30:05Z`); empty when a hand-edited row has none.
    pub added_at: String,
    // ## m4b-t10-service fix round 2 (named exception; review m-a, H6b)
    /// The canonical path the folder had when the library service first resolved it; a later
    /// launch whose resolution differs refuses the root (a folder above it replaced by a link).
    /// `None` in rows written before, and by [`LibraryRegistry::add`].
    pub canonical: Option<PathBuf>,
    /// The folder's inode then (Unix); compared across sessions with the canonical path. The
    /// device number is not kept: it can change across a reboot or a remount.
    pub inode: Option<u64>,
    // ## m4b-t10-service
}

impl RegisteredLibrary {
    /// The folder as it is now: its canonical path, or the error when it is missing or
    /// unmounted (it stays registered, HV-SCRIPT-01 step 5).
    pub fn root(&self) -> io::Result<LibraryRoot> {
        Ok(LibraryRoot {
            id: self.id,
            path: self.path.clone(),
            canonical: fs::canonicalize(&self.path)?,
        })
    }
}

/// The folders a library may not be (HV-SCRIPT-01 step 2): the home folder itself and the
/// app's own folders.
#[derive(Debug, Clone)]
pub struct AttachGuard {
    home: Option<PathBuf>,
    app_dirs: Vec<PathBuf>,
}

impl AttachGuard {
    /// The app's folders from `paths`, and the user's home folder when it is known.
    pub fn new(paths: &LocalPaths, home: Option<&Path>) -> Self {
        let mut app_dirs = vec![
            paths.config.clone(),
            paths.data.clone(),
            paths.cache.clone(),
        ];
        app_dirs.extend(paths.runtime.clone());
        AttachGuard {
            home: home.map(Path::to_path_buf),
            app_dirs,
        }
    }
}

/// The script-library registry. The only code that writes `libraries.toml`.
#[derive(Debug)]
pub struct LibraryRegistry {
    path: PathBuf,
    libraries: Vec<RegisteredLibrary>,
    mappings: Vec<LocalMapping>,
    read_only: bool,
}

impl LibraryRegistry {
    /// Loads the registry at `path` (`LocalPaths::libraries_toml`). Never fails: what it could
    /// not use is in the diagnostics.
    pub fn load(path: &Path) -> (Self, Vec<RegistryDiag>) {
        Self::load_with(path, &canonical_lenient)
    }

    // ## m4b-t10-service fix round 2 (named exception; review F1)
    /// [`load`](Self::load) that touches no library folder: rows are compared by their text
    /// (id and path) only, and a row whose folder is inside another's is dropped only when the
    /// paths say so as written. The library service validates each root on its own worker
    /// (canonical nesting included), so a folder on a hung mount never blocks the load.
    pub fn load_text_only(path: &Path) -> (Self, Vec<RegistryDiag>) {
        Self::load_with(path, &Path::to_path_buf)
    }

    /// Records the canonical path and inode a library's root resolved to (the service, after
    /// the first validation of a row that has none). Does not save; `false` when unknown.
    pub fn set_resolved(&mut self, id: LibraryId, canonical: &Path, inode: Option<u64>) -> bool {
        if self.read_only {
            return false;
        }
        match self.libraries.iter_mut().find(|l| l.id == id) {
            Some(l) => {
                l.canonical = Some(canonical.to_path_buf());
                l.inode = inode;
                true
            }
            None => false,
        }
    }

    /// [`add`](Self::add) for a folder already resolved off this thread: `canonical` (and its
    /// inode) come from the caller's worker, the attach guard's checks (filesystem root, home,
    /// app folders) were made there, and `others` are the canonical paths of the registered
    /// libraries as the caller knows them. Makes no file-system call. Does not save.
    pub fn add_resolved(
        &mut self,
        path: &Path,
        canonical: &Path,
        inode: Option<u64>,
        others: &[(LibraryId, PathBuf)],
        now: SystemTime,
    ) -> Result<LibraryRoot, RegistryError> {
        self.writable()?;
        if !path.is_absolute() {
            return Err(RegistryError::NotAbsolute);
        }
        if path.to_str().is_none() {
            return Err(RegistryError::NotUtf8Path);
        }
        for lib in &self.libraries {
            let theirs = others
                .iter()
                .find(|(id, _)| *id == lib.id)
                .map(|(_, c)| c.as_path())
                .or(lib.canonical.as_deref())
                .unwrap_or(&lib.path);
            if theirs == canonical || lib.path == path {
                return Err(RegistryError::AlreadyRegistered(lib.id));
            }
            if overlaps(canonical, theirs) || overlaps(path, &lib.path) {
                return Err(RegistryError::Nested(lib.id));
            }
        }
        if self.libraries.len() >= MAX_LIBRARIES {
            return Err(RegistryError::TooMany);
        }
        let id = LibraryId::new();
        self.libraries.push(RegisteredLibrary {
            id,
            path: path.to_path_buf(),
            added_at: rfc3339(now),
            canonical: Some(canonical.to_path_buf()),
            inode,
        });
        Ok(LibraryRoot {
            id,
            path: path.to_path_buf(),
            canonical: canonical.to_path_buf(),
        })
    }
    // ## m4b-t10-service

    fn load_with(
        path: &Path,
        canonical_of: &dyn Fn(&Path) -> PathBuf,
    ) -> (Self, Vec<RegistryDiag>) {
        let mut reg = LibraryRegistry {
            path: path.to_path_buf(),
            libraries: Vec::new(),
            mappings: Vec::new(),
            read_only: false,
        };
        let mut diags = Vec::new();
        let bytes = match read_capped(path) {
            Ok(Capped::Bytes(bytes)) => bytes,
            Ok(Capped::TooLarge) => {
                reg.set_aside(&mut diags);
                return (reg, diags);
            }
            Ok(Capped::NotAFile) => {
                reg.read_only = true;
                diags.push(RegistryDiag::NotAFile);
                return (reg, diags);
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => return (reg, diags),
            Err(e) => {
                reg.read_only = true;
                diags.push(RegistryDiag::Unreadable(e.kind()));
                return (reg, diags);
            }
        };
        // The parser recurses: a file nested deeper than a registry ever is would exhaust the
        // stack at every start, so it is set aside unparsed.
        let Some(table) = std::str::from_utf8(&bytes)
            .ok()
            .filter(|t| manifest::nesting_depth(t) <= manifest::MAX_NESTING)
            .and_then(|t| t.parse::<toml::Table>().ok())
        else {
            reg.set_aside(&mut diags);
            return (reg, diags);
        };
        let version = match table.get("version") {
            Some(toml::Value::Integer(v)) if *v >= 1 => *v,
            _ => {
                reg.set_aside(&mut diags);
                return (reg, diags);
            }
        };
        if version > VERSION {
            reg.read_only = true;
            diags.push(RegistryDiag::NewerVersion(version));
            return (reg, diags);
        }
        let (Some(libraries), Some(mappings)) = (rows(&table, "library"), rows(&table, "mapping"))
        else {
            reg.set_aside(&mut diags);
            return (reg, diags);
        };
        reg.load_libraries(&libraries, &mut diags, canonical_of);
        reg.load_mappings(&mappings, &mut diags);
        (reg, diags)
    }

    fn load_libraries(
        &mut self,
        rows: &[&toml::Table],
        diags: &mut Vec<RegistryDiag>,
        canonical_of: &dyn Fn(&Path) -> PathBuf,
    ) {
        let mut too_many = false;
        // The kept rows' canonical paths (a missing folder through its deepest existing
        // ancestor), so a hand-edited file cannot project one folder twice.
        let mut canonical: Vec<PathBuf> = Vec::new();
        for (index, row) in rows.iter().enumerate() {
            let parsed = library_row(row).and_then(|lib| {
                let here = canonical_of(&lib.path);
                let same = self
                    .libraries
                    .iter()
                    .zip(&canonical)
                    .any(|(l, c)| l.id == lib.id || l.path == lib.path || *c == here);
                if same {
                    Err(RowProblem::Duplicate)
                } else if canonical.iter().any(|c| overlaps(c, &here)) {
                    Err(RowProblem::Nested)
                } else {
                    Ok((lib, here))
                }
            });
            match parsed {
                Ok(_) if self.libraries.len() == MAX_LIBRARIES => too_many = true,
                Ok((lib, here)) => {
                    self.libraries.push(lib);
                    canonical.push(here);
                }
                Err(problem) => diags.push(RegistryDiag::LibraryDropped { index, problem }),
            }
        }
        if too_many {
            diags.push(RegistryDiag::TooManyLibraries);
        }
    }

    fn load_mappings(&mut self, rows: &[&toml::Table], diags: &mut Vec<RegistryDiag>) {
        for (index, row) in rows.iter().enumerate() {
            let m = match mapping_row(row) {
                Ok(m) => m,
                Err(problem) => {
                    diags.push(RegistryDiag::MappingDropped { index, problem });
                    continue;
                }
            };
            if self.library(m.library).is_none() {
                diags.push(RegistryDiag::OrphanMapping { index });
            } else if self.mapping_index(m.profile, &m.database).is_some() {
                diags.push(RegistryDiag::MappingDropped {
                    index,
                    problem: RowProblem::Duplicate,
                });
            } else {
                self.mappings.push(m);
            }
        }
    }

    /// Moves a file that is not a registry aside (the M1 settings rule); read-only when that
    /// fails too, so it is never overwritten.
    fn set_aside(&mut self, diags: &mut Vec<RegistryDiag>) {
        match fsio::quarantine(&self.path, "corrupt", SystemTime::now()) {
            Ok(moved) => diags.push(RegistryDiag::SetAside(moved)),
            Err(e) => {
                self.read_only = true;
                diags.push(RegistryDiag::QuarantineFailed(e.kind()));
            }
        }
    }

    /// Writes the registry atomically (0600, its folder 0700 when created).
    pub fn save(&self) -> Result<(), RegistryError> {
        self.writable()?;
        let text = self.to_toml().ok_or(RegistryError::NotUtf8Path)?;
        fsio::atomic_write(&self.path, text.as_bytes()).map_err(|e| RegistryError::Io(e.kind()))
    }

    fn to_toml(&self) -> Option<String> {
        #[derive(Serialize)]
        struct File<'a> {
            version: i64,
            #[serde(skip_serializing_if = "Vec::is_empty")]
            library: Vec<LibraryOut<'a>>,
            #[serde(skip_serializing_if = "Vec::is_empty")]
            mapping: Vec<MappingOut<'a>>,
        }
        #[derive(Serialize)]
        struct LibraryOut<'a> {
            id: LibraryId,
            path: &'a str,
            added_at: &'a str,
            // ## m4b-t10-service fix round 2 (m-a): written only once resolved.
            #[serde(skip_serializing_if = "Option::is_none")]
            canonical: Option<&'a str>,
            #[serde(skip_serializing_if = "Option::is_none")]
            inode: Option<String>,
        }
        #[derive(Serialize)]
        struct MappingOut<'a> {
            profile: ProfileId,
            database: &'a DatabaseName,
            library: LibraryId,
        }
        let library = self
            .libraries
            .iter()
            .map(|l| {
                Some(LibraryOut {
                    id: l.id,
                    path: l.path.to_str()?,
                    added_at: &l.added_at,
                    canonical: l.canonical.as_deref().and_then(Path::to_str),
                    inode: l.inode.map(|i| i.to_string()),
                })
            })
            .collect::<Option<Vec<_>>>()?;
        let mapping = self
            .mappings
            .iter()
            .map(|m| MappingOut {
                profile: m.profile,
                database: &m.database,
                library: m.library,
            })
            .collect();
        toml::to_string(&File {
            version: VERSION,
            library,
            mapping,
        })
        .ok()
    }

    /// Whether the registry refuses every change (a newer or unreadable file).
    pub fn is_read_only(&self) -> bool {
        self.read_only
    }

    fn writable(&self) -> Result<(), RegistryError> {
        if self.read_only {
            Err(RegistryError::ReadOnly)
        } else {
            Ok(())
        }
    }

    /// The registered libraries, in the order they were added.
    pub fn libraries(&self) -> &[RegisteredLibrary] {
        &self.libraries
    }

    pub fn library(&self, id: LibraryId) -> Option<&RegisteredLibrary> {
        self.libraries.iter().find(|l| l.id == id)
    }

    /// Every local mapping.
    pub fn mappings(&self) -> &[LocalMapping] {
        &self.mappings
    }

    /// The local mappings of `profile`.
    pub fn mappings_for(&self, profile: ProfileId) -> impl Iterator<Item = &LocalMapping> {
        self.mappings.iter().filter(move |m| m.profile == profile)
    }

    fn mapping_index(&self, profile: ProfileId, database: &DatabaseName) -> Option<usize> {
        self.mappings
            .iter()
            .position(|m| m.profile == profile && m.database == *database)
    }

    /// Registers the folder `path` (absolute, kept as given) after the attach checks of
    /// HV-SCRIPT-01 step 2, all on canonical paths. Does not save.
    pub fn add(
        &mut self,
        path: &Path,
        guard: &AttachGuard,
        now: SystemTime,
    ) -> Result<LibraryRoot, RegistryError> {
        self.writable()?;
        if !path.is_absolute() {
            return Err(RegistryError::NotAbsolute);
        }
        let io_err = |e: io::Error| RegistryError::Io(e.kind());
        let canonical = fs::canonicalize(path).map_err(io_err)?;
        if !fs::metadata(&canonical).map_err(io_err)?.is_dir() {
            return Err(RegistryError::NotAFolder);
        }
        if path.to_str().is_none() {
            return Err(RegistryError::NotUtf8Path);
        }
        if canonical.parent().is_none() {
            return Err(RegistryError::FilesystemRoot);
        }
        if guard.home.as_deref().map(canonical_lenient) == Some(canonical.clone()) {
            return Err(RegistryError::HomeFolder);
        }
        if guard
            .app_dirs
            .iter()
            .map(|d| canonical_lenient(d))
            .any(|d| overlaps(&canonical, &d))
        {
            return Err(RegistryError::AppFolder);
        }
        for lib in &self.libraries {
            let other = canonical_lenient(&lib.path);
            if other == canonical {
                return Err(RegistryError::AlreadyRegistered(lib.id));
            }
            if overlaps(&canonical, &other) {
                return Err(RegistryError::Nested(lib.id));
            }
        }
        if self.libraries.len() >= MAX_LIBRARIES {
            return Err(RegistryError::TooMany);
        }
        let id = LibraryId::new();
        self.libraries.push(RegisteredLibrary {
            id,
            path: path.to_path_buf(),
            added_at: rfc3339(now),
            canonical: None,
            inode: None,
        });
        Ok(LibraryRoot {
            id,
            path: path.to_path_buf(),
            canonical,
        })
    }

    /// Forgets library `id` and its mappings; the folder is never touched. Does not save.
    pub fn remove(&mut self, id: LibraryId) -> Result<(), RegistryError> {
        self.writable()?;
        let index = self
            .libraries
            .iter()
            .position(|l| l.id == id)
            .ok_or(RegistryError::UnknownLibrary)?;
        self.libraries.remove(index);
        self.mappings.retain(|m| m.library != id);
        Ok(())
    }

    /// Maps the node `(m.profile, m.database)` to `m.library`, replacing that node's mapping.
    /// Does not save.
    pub fn set_mapping(&mut self, m: LocalMapping) -> Result<(), RegistryError> {
        self.writable()?;
        if self.library(m.library).is_none() {
            return Err(RegistryError::UnknownLibrary);
        }
        match self.mapping_index(m.profile, &m.database) {
            Some(i) => self.mappings[i] = m,
            None => self.mappings.push(m),
        }
        Ok(())
    }

    /// Removes the node's mapping; whether there was one.
    pub fn remove_mapping(&mut self, profile: ProfileId, database: &DatabaseName) -> bool {
        if self.read_only {
            return false;
        }
        match self.mapping_index(profile, database) {
            Some(i) => {
                self.mappings.remove(i);
                true
            }
            None => false,
        }
    }

    /// Drops every mapping of a deleted profile (R-M4b-2); how many. Does not save.
    pub fn drop_profile(&mut self, profile: ProfileId) -> usize {
        self.retain_profiles(|p| p != profile)
    }

    /// Keeps only the mappings of profiles for which `keep` answers true (at start, against
    /// the profiles that exist); how many were dropped. Does not save.
    pub fn retain_profiles(&mut self, keep: impl Fn(ProfileId) -> bool) -> usize {
        if self.read_only {
            return 0;
        }
        let before = self.mappings.len();
        self.mappings.retain(|m| keep(m.profile));
        before - self.mappings.len()
    }
}

/// What [`read_capped`] found.
enum Capped {
    Bytes(Vec<u8>),
    /// Larger than any registry.
    TooLarge,
    /// Not a regular file (review m7): never opened.
    NotAFile,
}

/// The file's bytes. Only a regular file is opened (a link is followed, as config managers
/// link dot files, but its target must be a regular file), with `O_NONBLOCK` on Unix and an
/// `fstat` re-check, so a FIFO swapped in never blocks the start.
fn read_capped(path: &Path) -> io::Result<Capped> {
    use std::io::Read;
    if !fs::metadata(path)?.is_file() {
        return Ok(Capped::NotAFile);
    }
    let mut opts = fs::OpenOptions::new();
    opts.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.custom_flags(libc::O_NONBLOCK);
    }
    let file = opts.open(path)?;
    if !file.metadata()?.is_file() {
        return Ok(Capped::NotAFile);
    }
    let mut bytes = Vec::new();
    file.take(MAX_FILE_BYTES + 1).read_to_end(&mut bytes)?;
    Ok(if bytes.len() as u64 <= MAX_FILE_BYTES {
        Capped::Bytes(bytes)
    } else {
        Capped::TooLarge
    })
}

/// The rows of the array of tables `key` (none when absent); `None` when it has another shape.
fn rows<'a>(table: &'a toml::Table, key: &str) -> Option<Vec<&'a toml::Table>> {
    match table.get(key) {
        None => Some(Vec::new()),
        Some(toml::Value::Array(items)) => items.iter().map(toml::Value::as_table).collect(),
        Some(_) => None,
    }
}

fn text<'a>(row: &'a toml::Table, key: &str) -> Result<&'a str, RowProblem> {
    row.get(key)
        .and_then(toml::Value::as_str)
        .ok_or(RowProblem::MissingField)
}

fn uuid_field(row: &toml::Table, key: &str) -> Result<Uuid, RowProblem> {
    Uuid::parse_str(text(row, key)?).map_err(|_| RowProblem::BadId)
}

fn library_row(row: &toml::Table) -> Result<RegisteredLibrary, RowProblem> {
    let id = LibraryId(uuid_field(row, "id")?);
    let path = PathBuf::from(text(row, "path")?);
    if !path.is_absolute() {
        return Err(RowProblem::RelativePath);
    }
    let added_at = row
        .get("added_at")
        .and_then(toml::Value::as_str)
        .unwrap_or_default()
        .to_string();
    // ## m4b-t10-service fix round 2 (m-a): the resolved root, when the service recorded it.
    // A malformed value is dropped (the root is resolved again), never trusted.
    let canonical = row
        .get("canonical")
        .and_then(toml::Value::as_str)
        .map(PathBuf::from)
        .filter(|c| c.is_absolute());
    let inode = row
        .get("inode")
        .and_then(toml::Value::as_str)
        .and_then(|i| i.parse::<u64>().ok());
    Ok(RegisteredLibrary {
        id,
        path,
        added_at,
        canonical,
        inode,
    })
}

fn mapping_row(row: &toml::Table) -> Result<LocalMapping, RowProblem> {
    Ok(LocalMapping {
        profile: ProfileId(uuid_field(row, "profile")?),
        database: DatabaseName::new(text(row, "database")?),
        library: LibraryId(uuid_field(row, "library")?),
    })
}

/// Whether one folder is the other or contains it (whole components: `team2` is not inside
/// `team`).
fn overlaps(a: &Path, b: &Path) -> bool {
    a.starts_with(b) || b.starts_with(a)
}

/// The canonical form of `path`, or of its deepest existing ancestor with the rest appended
/// (a folder that does not exist yet still compares like the real ones; macOS `/var` →
/// `/private/var`).
fn canonical_lenient(path: &Path) -> PathBuf {
    for ancestor in path.ancestors() {
        if let Ok(base) = fs::canonicalize(ancestor) {
            let rest = path.strip_prefix(ancestor).unwrap_or(Path::new(""));
            return if rest.as_os_str().is_empty() {
                base
            } else {
                base.join(rest)
            };
        }
    }
    path.to_path_buf()
}

/// `t` as RFC 3339 UTC, to the second.
fn rfc3339(t: SystemTime) -> String {
    let secs = t
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(0));
    let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    let (y, m, d) = civil_from_days(days);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

/// The proleptic Gregorian date of a day count since 1970-01-01 (H. Hinnant's
/// `civil_from_days`).
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn rfc3339_formats_utc_seconds() {
        let at = |s: u64| rfc3339(UNIX_EPOCH + Duration::from_secs(s));
        assert_eq!(at(0), "1970-01-01T00:00:00Z");
        assert_eq!(at(951_782_400), "2000-02-29T00:00:00Z");
        assert_eq!(at(1_795_343_405), "2026-11-22T10:30:05Z");
        assert_eq!(at(4_107_542_399), "2100-02-28T23:59:59Z");
    }
}
// ## m4b-t01a-seams
