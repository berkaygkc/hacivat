//! From repository-relative changes to library-relative badges (HV-SCRIPT-11 steps 5 and 6).
//!
//! - **Precedence**: `Unknown` overrides every other badge (a filtered, dataless, submodule,
//!   case-mismatched or non-file path is never shown as if its comparison were exact); then
//!   Conflicted > New > Modified (`alg/28` §0.4: `AM` is New, a staged rename's destination is
//!   New).
//! - **One normal form for comparing** (fix round 1, I1): the library prefix and index paths
//!   are compared component by component in NFC (macOS keeps an NFD folder name while the index
//!   holds NFC under `core.precomposeUnicode`), and ASCII case-insensitively where the file
//!   system folds case (`core.ignoreCase`).
//! - **Keys**: the repository path minus the library prefix, spelled as the **disk** spells it,
//!   byte for byte, so that it equals the scanner's `RelPath` (R-T1a-2 carry). Each component
//!   is matched against its directory's listing: exactly, then by NFC equality, then (I2,
//!   R-T7-1) by the file system's own lookup, identified by inode on Unix and by a unique
//!   ASCII case-insensitive match on Windows. A path found only that last way, or under a
//!   prefix that matched only by case, is `Unknown(CaseMismatch)`, never dropped. A path the
//!   file system cannot find (deleted) has no badge, as the plan says; neither has a name that
//!   is not UTF-8 (the scanner cannot list it).
//! - **Dataless** (R-M4b-15): a cloud placeholder's flag is read from `lstat` metadata only.

// ## m4b-t07-git
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};

use gix::bstr::{BStr, BString, ByteSlice as _};

use crate::{FileStatus, GitPath, UnknownWhy};

/// The order in which badges win when one path has several.
fn rank(s: FileStatus) -> u8 {
    match s {
        FileStatus::Modified => 1,
        FileStatus::New => 2,
        FileStatus::Conflicted => 3,
        FileStatus::Unknown(UnknownWhy::Filtered) => 4,
        FileStatus::Unknown(UnknownWhy::CaseMismatch) => 5,
        FileStatus::Unknown(UnknownWhy::NotAFile) => 6,
        FileStatus::Unknown(UnknownWhy::Dataless) => 7,
        FileStatus::Unknown(UnknownWhy::Submodule) => 8,
        // ## m4b-cifix-2-index-paths: the index path itself is not trusted.
        FileStatus::Unknown(UnknownWhy::UnsafePath) => 9,
    }
}

// ## m4b-cifix-2-index-paths (review C2, R-CI2-2)
/// Whether an index path (`/`-separated bytes, untrusted repository content) is made of plain
/// names only, so that joining it under the worktree names a path inside the worktree on every
/// OS. Each component must pass `gix_validate::path::component` with the Windows, NTFS and HFS
/// protections on (`Options::default()`): no empty, `.` or `..` component, no `/` or `\`, no
/// drive (`C:`) or UNC (`\\host\share`, `\\?\`) prefix, no NTFS stream (`a:b`), reserved device
/// name or `.git` spelling (`GIT~1`, `.git ` …). On Unix an absolute index path (`/net/x`)
/// starts with an empty component. Nothing else in this crate joins an index path to a
/// directory before this holds (T7's Unix automounter case, `/net/<host>`, included).
pub(crate) fn is_plain_index_path(path: &[u8]) -> bool {
    use gix::validate::path::component::Options;
    !path.is_empty()
        && path.split(|b| *b == b'/').all(|c| {
            gix::validate::path::component(c.as_bstr(), None, Options::default()).is_ok()
                && c.to_str().is_ok_and(|s| {
                    let mut parts = Path::new(s).components();
                    matches!(
                        (parts.next(), parts.next()),
                        (Some(std::path::Component::Normal(_)), None)
                    )
                })
        })
}

/// An index folder spelling that n2 may compare with the library folder: plain
/// ([`is_plain_index_path`]), with no 8.3 short-name component (`SCRIPT~1`: another spelling
/// than a case-only rename, which the alias is for), joined under `worktree` and still under
/// it, component for component. `None` means: never touched.
pub(crate) fn alias_candidate(worktree: &Path, head: &str) -> Option<PathBuf> {
    let short_name = |c: &str| {
        c.as_bytes()
            .windows(2)
            .any(|w| w[0] == b'~' && w[1].is_ascii_digit())
    };
    if !is_plain_index_path(head.as_bytes()) || head.split('/').any(short_name) {
        return None;
    }
    let joined = worktree.join(head);
    let depth = worktree.components().count() + head.split('/').count();
    (joined.starts_with(worktree) && joined.components().count() == depth).then_some(joined)
}
// ## m4b-cifix-2-index-paths

/// The stronger of two badges.
pub(crate) fn stronger(a: FileStatus, b: FileStatus) -> FileStatus {
    if rank(b) > rank(a) { b } else { a }
}

/// NFC (the one normal form paths are compared in).
pub(crate) fn nfc(s: &str) -> String {
    gix::utils::str::precompose(s.into()).into_owned()
}

/// Repository-relative badges, merged by precedence.
#[derive(Debug, Default)]
pub(crate) struct Badges {
    by_repo_path: BTreeMap<BString, FileStatus>,
}

impl Badges {
    pub(crate) fn add(&mut self, path: &BStr, s: FileStatus) {
        let e = self.by_repo_path.entry(path.to_owned()).or_insert(s);
        *e = stronger(*e, s);
    }

    pub(crate) fn get(&self, path: &BStr) -> Option<FileStatus> {
        self.by_repo_path.get(path).copied()
    }

    pub(crate) fn paths(&self) -> Vec<BString> {
        self.by_repo_path.keys().cloned().collect()
    }

    /// The badges keyed relative to the library root at `library` (canonical), whose path
    /// relative to the worktree is `prefix` (disk spelling); `icase` when the file system
    /// folds case; `aliases` are index spellings of the library folder that the file system
    /// equates with it (n2). Dataless paths found on the way become `Unknown(Dataless)`,
    /// case-only mismatches `Unknown(CaseMismatch)`.
    pub(crate) fn into_library(
        self,
        prefix: &str,
        icase: bool,
        aliases: &BTreeSet<String>,
        library: &Path,
        is_dataless: &dyn Fn(&std::fs::Metadata) -> bool,
    ) -> BTreeMap<GitPath, FileStatus> {
        let mut lister = Lister::default();
        let mut out = BTreeMap::new();
        for (repo_path, s) in self.by_repo_path {
            let Ok(repo_path) = repo_path.to_str() else {
                continue;
            };
            let Some((rel, prefix_case)) = lib_relative(repo_path, prefix, icase, aliases) else {
                continue;
            };
            // ## m4b-cifix-2-index-paths (C2): keyed as the index spells it, never looked up.
            if !is_plain_index_path(rel.as_bytes()) {
                let s = FileStatus::Unknown(UnknownWhy::UnsafePath);
                let e = out.entry(GitPath::new(&rel)).or_insert(s);
                *e = stronger(*e, s);
                continue;
            }
            let Some(found) = lister.on_disk(library, &rel) else {
                continue;
            };
            let mut s = s;
            if !matches!(s, FileStatus::Unknown(_))
                && std::fs::symlink_metadata(&found.full).is_ok_and(|m| is_dataless(&m))
            {
                s = FileStatus::Unknown(UnknownWhy::Dataless);
            }
            if prefix_case || found.case_only {
                s = stronger(s, FileStatus::Unknown(UnknownWhy::CaseMismatch));
            }
            let e = out.entry(GitPath::new(&found.key)).or_insert(s);
            *e = stronger(*e, s);
        }
        out
    }
}

/// The repository path `path` relative to the library folder `prefix` (disk spelling, `""` for
/// the worktree), in the index's spelling, and whether the folder matched only by folding case.
/// Compared component by component in NFC, ASCII case-insensitively when `icase`, and against
/// `aliases` (index spellings the file system equates with the folder, n2). `None` outside the
/// library.
pub(crate) fn lib_relative(
    path: &str,
    prefix: &str,
    icase: bool,
    aliases: &BTreeSet<String>,
) -> Option<(String, bool)> {
    if prefix.is_empty() {
        return (!path.is_empty()).then(|| (path.to_owned(), false));
    }
    let n = prefix.split('/').count();
    let comps: Vec<&str> = path.split('/').collect();
    if comps.len() <= n {
        return None;
    }
    let raw_head = comps[..n].join("/");
    let (head, want) = (nfc(&raw_head), nfc(prefix));
    let rest = comps[n..].join("/");
    if head == want {
        Some((rest, false))
    } else if (icase && head.eq_ignore_ascii_case(&want)) || aliases.contains(&raw_head) {
        Some((rest, true))
    } else {
        None
    }
}

/// Whether two paths are the same file system entry: `(dev, ino)` from `lstat` on Unix.
///
/// ## m4b-cifix-2-windows-alias: `std` has no stable file id on Windows, so there the entry is
/// identified by the final path the system reports for an open handle (`canonicalize`,
/// `GetFinalPathNameByHandleW`), which spells every component as the disk does. Two spellings
/// of one folder (`Çal` and `çal` under case folding) resolve alike. Neither may be a link or
/// junction, as on Unix where `lstat` does not follow the last component. It opens a handle on
/// each folder and reads no file. Before this, n2's alias match was Unix-only and a non-ASCII
/// case-only rename of the library folder showed its files as `New` (CI 44990057141).
pub(crate) fn same_file(a: &Path, b: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;
        match (std::fs::symlink_metadata(a), std::fs::symlink_metadata(b)) {
            (Ok(x), Ok(y)) => x.dev() == y.dev() && x.ino() == y.ino(),
            _ => false,
        }
    }
    #[cfg(not(unix))]
    {
        let real = |p: &Path| {
            std::fs::symlink_metadata(p)
                .ok()
                .filter(|m| !m.file_type().is_symlink())
                .and_then(|_| std::fs::canonicalize(p).ok())
        };
        matches!((real(a), real(b)), (Some(x), Some(y)) if x == y)
    }
}

/// A path as the disk spells it.
struct OnDisk {
    key: String,
    full: PathBuf,
    /// Some component was found only through the file system's case folding.
    case_only: bool,
}

/// One listed name: as listed, its NFC form.
type Listed = (String, String);

/// Directory listings, read once each (names only, never contents), and (Unix) each listing's
/// file ids, read once on the first case-only lookup in it (n4: no `lstat` per name per path).
#[derive(Default)]
struct Lister {
    dirs: HashMap<PathBuf, Vec<Listed>>,
    #[cfg(unix)]
    ids: HashMap<PathBuf, HashMap<(u64, u64), String>>,
}

impl Lister {
    /// The names in `dir` with their NFC forms.
    fn names(&mut self, dir: &Path) -> Option<&[Listed]> {
        if !self.dirs.contains_key(dir) {
            let names = std::fs::read_dir(dir)
                .ok()?
                .filter_map(Result::ok)
                .filter_map(|e| e.file_name().into_string().ok())
                .map(|n| {
                    let f = nfc(&n);
                    (n, f)
                })
                .collect();
            self.dirs.insert(dir.to_path_buf(), names);
        }
        self.dirs.get(dir).map(Vec::as_slice)
    }

    /// `rel` (the index's spelling) as the disk spells it under `base`; `None` when the file
    /// system cannot find it.
    fn on_disk(&mut self, base: &Path, rel: &str) -> Option<OnDisk> {
        let mut dir = base.to_path_buf();
        let mut parts: Vec<String> = Vec::new();
        let mut case_only = false;
        for comp in rel.split('/') {
            // ## m4b-cifix-2-index-paths (C2): no file-system call on a name that is not plain.
            if !is_plain_index_path(comp.as_bytes()) {
                return None;
            }
            let want = nfc(comp);
            let names = self.names(&dir)?;
            let exact = names
                .iter()
                .find(|(n, _)| n == comp)
                .or_else(|| names.iter().find(|(_, f)| *f == want))
                .map(|(n, _)| n.clone());
            let found = match exact {
                Some(n) => n,
                None => {
                    // The file system's own lookup (case folding): never dropped (R-T7-1).
                    let md = std::fs::symlink_metadata(dir.join(comp)).ok()?;
                    case_only = true;
                    self.same_entry(&dir, &md, &want)
                        .unwrap_or_else(|| comp.to_owned())
                }
            };
            dir.push(&found);
            parts.push(found);
        }
        Some(OnDisk {
            key: parts.join("/"),
            full: dir,
            case_only,
        })
    }

    /// The listed name in `dir` that is the entry `md` describes: by `(dev, ino)` on Unix, from
    /// the listing's id table (built once per folder).
    #[cfg(unix)]
    fn same_entry(&mut self, dir: &Path, md: &std::fs::Metadata, _want: &str) -> Option<String> {
        use std::os::unix::fs::MetadataExt as _;
        if !self.ids.contains_key(dir) {
            let mut table = HashMap::new();
            for (n, _) in self.names(dir)?.to_vec() {
                if let Ok(m) = std::fs::symlink_metadata(dir.join(&n)) {
                    table.insert((m.dev(), m.ino()), n);
                }
            }
            self.ids.insert(dir.to_path_buf(), table);
        }
        self.ids.get(dir)?.get(&(md.dev(), md.ino())).cloned()
    }

    /// The listed name in `dir` that is the entry `md` describes: `std` has no stable file id on
    /// Windows, so the one name equal to `want` ASCII case-insensitively (in NFC). A non-ASCII
    /// case-only rename finds none: the badge keeps the index's spelling, which no scanned row
    /// has (n3, a recorded residual).
    #[cfg(not(unix))]
    fn same_entry(&mut self, dir: &Path, _md: &std::fs::Metadata, want: &str) -> Option<String> {
        let names = self.names(dir)?;
        let mut hits = names.iter().filter(|(_, f)| f.eq_ignore_ascii_case(want));
        match (hits.next(), hits.next()) {
            (Some((n, _)), None) => Some(n.clone()),
            _ => None,
        }
    }
}

/// Whether `md` (from `lstat`) is a cloud placeholder whose content the OS downloads on read:
/// `SF_DATALESS` on macOS, `FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS` or `…_RECALL_ON_OPEN` on
/// Windows (R-M4b-15). Other systems have none.
pub(crate) fn is_dataless(md: &std::fs::Metadata) -> bool {
    #[cfg(target_os = "macos")]
    {
        use std::os::macos::fs::MetadataExt as _;
        const SF_DATALESS: u32 = 0x4000_0000;
        md.st_flags() & SF_DATALESS != 0
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt as _;
        const FILE_ATTRIBUTE_RECALL_ON_OPEN: u32 = 0x0004_0000;
        const FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS: u32 = 0x0040_0000;
        md.file_attributes()
            & (FILE_ATTRIBUTE_RECALL_ON_OPEN | FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS)
            != 0
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        let _ = md;
        false
    }
}
// ## m4b-t07-git

#[cfg(test)]
mod tests {
    use super::*;

    fn never(_: &std::fs::Metadata) -> bool {
        false
    }

    #[test]
    fn precedence_is_unknown_then_conflicted_then_new_then_modified() {
        let mut b = Badges::default();
        let p: &BStr = "a.sql".into();
        b.add(p, FileStatus::Modified);
        b.add(p, FileStatus::New);
        assert_eq!(b.get(p), Some(FileStatus::New));
        b.add(p, FileStatus::Modified);
        assert_eq!(b.get(p), Some(FileStatus::New), "AM stays New");
        b.add(p, FileStatus::Conflicted);
        assert_eq!(b.get(p), Some(FileStatus::Conflicted));
        b.add(p, FileStatus::Unknown(UnknownWhy::Filtered));
        assert_eq!(b.get(p), Some(FileStatus::Unknown(UnknownWhy::Filtered)));
        b.add(p, FileStatus::Conflicted);
        assert_eq!(b.get(p), Some(FileStatus::Unknown(UnknownWhy::Filtered)));
    }

    fn rel(path: &str, prefix: &str, icase: bool) -> Option<(String, bool)> {
        lib_relative(path, prefix, icase, &BTreeSet::new())
    }

    #[test]
    fn prefix_is_stripped_and_paths_outside_are_dropped() {
        assert_eq!(
            rel("lib/a.sql", "lib", false),
            Some(("a.sql".into(), false))
        );
        assert_eq!(
            rel("lib/x/a.sql", "lib", false),
            Some(("x/a.sql".into(), false))
        );
        assert_eq!(rel("library/a.sql", "lib", false), None);
        assert_eq!(rel("lib", "lib", false), None);
        assert_eq!(rel("a.sql", "", false), Some(("a.sql".into(), false)));
        assert_eq!(
            rel("a/b/c.sql", "a/b", false),
            Some(("c.sql".into(), false))
        );
    }

    /// I1: an NFD library folder on disk against the index's NFC spelling (Turkish names).
    #[test]
    fn the_prefix_is_compared_in_nfc() {
        let nfd = "c\u{327}al\u{131}s\u{327}ma";
        let nfc_path = "\u{e7}al\u{131}\u{15f}ma/a.sql";
        assert_eq!(rel(nfc_path, nfd, false), Some(("a.sql".into(), false)));
        assert_eq!(
            rel(&format!("{nfd}/a.sql"), nfd, false),
            Some(("a.sql".into(), false))
        );
        // Dotless `ı` is not `i`, in any normal form.
        assert_eq!(rel("\u{e7}alis\u{327}ma/a.sql", nfd, false), None);
    }

    /// I2: a folder that matches only by case does so only where the file system folds case,
    /// and says so.
    #[test]
    fn the_prefix_matches_by_case_only_when_the_file_system_folds_it() {
        assert_eq!(rel("scripts/a.sql", "Scripts", false), None);
        assert_eq!(
            rel("scripts/a.sql", "Scripts", true),
            Some(("a.sql".into(), true))
        );
        assert_eq!(
            rel("Scripts/a.sql", "Scripts", true),
            Some(("a.sql".into(), false))
        );
    }

    /// I2 (R-T7-1): the index's `Foo.sql` for the disk's `foo.sql`, on a file system that folds
    /// case, is the disk's key with `Unknown(CaseMismatch)`; never dropped. Where the file
    /// system does not fold case, the index spelling is not on disk: no badge.
    #[test]
    fn a_case_only_difference_is_unknown_and_keyed_by_the_disk() {
        let t = tempfile::tempdir().unwrap();
        let lib = std::fs::canonicalize(t.path()).unwrap();
        std::fs::write(lib.join("foo.sql"), b"x").unwrap();
        let folds = std::fs::symlink_metadata(lib.join("FOO.sql")).is_ok();
        let mut b = Badges::default();
        b.add("Foo.sql".into(), FileStatus::Modified);
        let out = b.into_library("", folds, &BTreeSet::new(), &lib, &never);
        if folds {
            assert_eq!(
                out.get("foo.sql"),
                Some(&FileStatus::Unknown(UnknownWhy::CaseMismatch)),
                "{out:?}"
            );
            assert_eq!(out.len(), 1);
        } else {
            assert!(out.is_empty(), "{out:?}");
        }
    }

    #[test]
    fn keys_take_the_disk_spelling_and_deleted_paths_have_no_badge() {
        let t = tempfile::tempdir().unwrap();
        let lib = std::fs::canonicalize(t.path()).unwrap();
        // The disk keeps the decomposed names; the index holds the precomposed ones.
        let nfd_dir = "u\u{308}ber";
        let nfd_file = "cafe\u{301}.sql";
        std::fs::create_dir_all(lib.join(nfd_dir)).unwrap();
        std::fs::write(lib.join(nfd_dir).join(nfd_file), b"x").unwrap();
        std::fs::write(lib.join("plain.sql"), b"x").unwrap();
        let mut b = Badges::default();
        b.add("lib/\u{fc}ber/caf\u{e9}.sql".into(), FileStatus::Modified);
        b.add("lib/plain.sql".into(), FileStatus::New);
        b.add("lib/gone.sql".into(), FileStatus::Modified);
        b.add("outside.sql".into(), FileStatus::Modified);
        let out = b.into_library("lib", false, &BTreeSet::new(), &lib, &never);
        let want_key = format!("{nfd_dir}/{nfd_file}");
        assert_eq!(
            out.get(want_key.as_str()),
            Some(&FileStatus::Modified),
            "{out:?}"
        );
        assert_eq!(out.get("plain.sql"), Some(&FileStatus::New));
        assert_eq!(out.len(), 2, "{out:?}");
    }

    #[test]
    fn a_dataless_path_found_while_mapping_is_unknown() {
        let t = tempfile::tempdir().unwrap();
        let lib = std::fs::canonicalize(t.path()).unwrap();
        std::fs::write(lib.join("cloud.sql"), b"").unwrap();
        std::fs::write(lib.join("local.sql"), b"x").unwrap();
        let mut b = Badges::default();
        b.add("cloud.sql".into(), FileStatus::New);
        b.add("local.sql".into(), FileStatus::New);
        // A stand-in for the flag: empty files are "placeholders" here.
        let out = b.into_library("", false, &BTreeSet::new(), &lib, &|m| m.len() == 0);
        assert_eq!(
            out.get("cloud.sql"),
            Some(&FileStatus::Unknown(UnknownWhy::Dataless))
        );
        assert_eq!(out.get("local.sql"), Some(&FileStatus::New));
    }

    #[test]
    fn an_ordinary_file_is_not_dataless() {
        let t = tempfile::tempdir().unwrap();
        let p = t.path().join("a.sql");
        std::fs::write(&p, b"select 1;\n").unwrap();
        assert!(!is_dataless(&std::fs::symlink_metadata(&p).unwrap()));
    }

    // ## m4b-cifix-2-index-paths (review C2, R-CI2-2)
    /// Hostile index spellings are never joined to a directory: not as an n2 alias head, not as
    /// a path. Every case is refused on every OS (an index written on another system is read
    /// here as-is).
    #[test]
    fn hostile_index_paths_are_never_candidates() {
        let wt = Path::new(if cfg!(windows) {
            r"C:\w\repo"
        } else {
            "/w/repo"
        });
        let hostile = [
            r"\\host\share",   // UNC: Windows would contact the host
            r"\\host\share/x", // the same, as the first of two components
            r"C:\",            // a drive root
            r"C:\x",           // a drive path
            "C:",              // a bare drive
            "c:x",             // drive-relative
            r"\\?\C:\x",       // verbatim
            r"\\.\pipe\x",     // a device namespace path
            r"..\x",           // `..` behind a backslash
            "..",              // the parent
            "../x",            // the parent, then a name
            "a/../b",          // a parent in the middle
            ".",               // the folder itself
            "/net/x",          // absolute (macOS's automounter)
            "/abs",            // absolute
            "a//b",            // an empty component
            "a/",              // a trailing empty component
            "lib:stream",      // an NTFS alternate data stream
            "lib::$DATA",      // the default stream, spelled out
            "CON",             // a reserved device name
            "aux.sql",         // a reserved device name with an extension
            "lib.",            // a trailing dot (NTFS drops it)
            "lib ",            // a trailing space (NTFS drops it)
            ".git",            // the repository itself
            "GIT~1",           // its 8.3 short name
            ".GIT",            // its case-folded spelling
            "a\u{0}b",         // a NUL
        ];
        for h in hostile {
            assert!(
                !is_plain_index_path(h.as_bytes()),
                "{h:?} passed as a plain path"
            );
            assert_eq!(
                alias_candidate(wt, h),
                None,
                "{h:?} became an alias candidate"
            );
        }
        // 8.3 short names are plain inside the worktree, but never an alias for a case-only
        // rename.
        for h in ["SCRIPT~1", "a/PROGRA~2"] {
            assert!(is_plain_index_path(h.as_bytes()), "{h:?}");
            assert_eq!(alias_candidate(wt, h), None, "{h:?}");
        }
        // Ordinary spellings stay candidates, under the worktree.
        for h in ["\u{c7}al", "Scripts/\u{c7}al", ".github", "a.b", "x~y"] {
            let c = alias_candidate(wt, h).unwrap_or_else(|| panic!("{h:?} refused"));
            assert!(c.starts_with(wt), "{c:?}");
        }
        // Non-UTF-8 bytes are refused too.
        assert!(!is_plain_index_path(b"lib/\xff.sql"));
    }

    /// An index path that is not plain is keyed as the index spells it, `Unknown(UnsafePath)`,
    /// and nothing under the library is looked up for it (the library here does not exist, so
    /// any lookup would drop the badge).
    #[test]
    fn an_unsafe_index_path_is_unknown_and_never_looked_up() {
        let mut b = Badges::default();
        b.add(BStr::new("lib/../../x.sql"), FileStatus::Modified);
        b.add(BStr::new(r"lib/\\host\share"), FileStatus::New);
        let out = b.into_library(
            "lib",
            false,
            &BTreeSet::new(),
            Path::new("/nonexistent-cifix2"),
            &never,
        );
        assert_eq!(
            out.get("../../x.sql"),
            Some(&FileStatus::Unknown(UnknownWhy::UnsafePath))
        );
        assert_eq!(
            out.get(r"\\host\share"),
            Some(&FileStatus::Unknown(UnknownWhy::UnsafePath))
        );
    }
    // ## m4b-cifix-2-index-paths

    // ## m4b-cifix-2-windows-alias
    /// n2's identity on every platform: the same folder under two spellings that only the file
    /// system's case folding equates (`Çal`, `çal`) is the same entry; another folder is not;
    /// a link to the folder is not (the last component is never followed).
    #[test]
    fn same_file_equates_spellings_of_one_folder_only() {
        let t = tempfile::tempdir().unwrap();
        let upper = t.path().join("\u{c7}al");
        std::fs::create_dir(&upper).unwrap();
        let lower = t.path().join("\u{e7}al");
        let folds = std::fs::symlink_metadata(&lower).is_ok();
        assert_eq!(same_file(&upper, &lower), folds, "folds case: {folds}");
        assert!(same_file(&upper, &upper));
        let other = t.path().join("other");
        std::fs::create_dir(&other).unwrap();
        assert!(!same_file(&upper, &other));
        assert!(!same_file(&upper, &t.path().join("missing")));
        #[cfg(unix)]
        {
            let link = t.path().join("link");
            std::os::unix::fs::symlink(&upper, &link).unwrap();
            assert!(!same_file(&link, &upper));
        }
    }
    // ## m4b-cifix-2-windows-alias
}
