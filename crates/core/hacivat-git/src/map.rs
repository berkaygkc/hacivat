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
/// names only, so that joining it under the worktree names a path inside the worktree.
///
/// On every OS each component is one `Normal` path component of valid UTF-8, not empty, `.` or
/// `..`, with no `/` and no NUL, and never a spelling of `.git` (`gix_validate::path::component`).
/// On Unix an absolute index path (`/net/x`) starts with an empty component.
///
/// ## m4b-cifix-2 fix round 2 (re-review N2, R-CI2-4): the host's own protections only, as git
/// itself applies them. Windows adds the Windows and NTFS ones: no `\`, no drive (`C:`), UNC
/// (`\\host\share`) or verbatim (`\\?\`) prefix, no NTFS stream (`a:b`), reserved device name
/// (`con.sql`, `aux.sql`), trailing dot or space, `GIT~1`. macOS adds the HFS+ `.git`
/// spellings with ignorable code points. Those names are ordinary files on Linux and macOS,
/// where the OS gives them no meaning, so they keep their true badge there; a backslash cannot
/// reach a UNC path on Unix. Nothing else in this crate joins an index path to a directory
/// before this holds (T7's Unix automounter case, `/net/<host>`, included).
pub(crate) fn is_plain_index_path(path: &[u8]) -> bool {
    use gix::validate::path::component::Options;
    let host = Options {
        protect_windows: cfg!(windows),
        protect_ntfs: cfg!(windows),
        protect_hfs: cfg!(target_os = "macos"),
    };
    // ## m4b-t20-adversarial fix round 4 (named line, I-fr3-1): the one prefix rule, shared with
    // the repository's own path texts (`trust`); on Windows an index path has no prefix at all.
    let no_prefix = || !cfg!(windows) || path.to_str().is_ok_and(|s| !path_prefix(s).has_prefix());
    !path.is_empty()
        && no_prefix()
        && path.split(|b| *b == b'/').all(|c| {
            !c.contains(&0)
                && gix::validate::path::component(c.as_bstr(), None, host).is_ok()
                && c.to_str().is_ok_and(|s| {
                    let mut parts = Path::new(s).components();
                    matches!(
                        (parts.next(), parts.next()),
                        (Some(std::path::Component::Normal(_)), None)
                    )
                })
        })
}

// ## m4b-t20-adversarial fix round 4 (named exception, re-review I-fr3-1; Review Focus 3)
/// How a path text starts, read the way Windows reads it. The reading is the same on every OS,
/// so the rule is one pure function and is tested everywhere.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PathPrefix {
    /// No prefix and no root: a relative path.
    None,
    /// ## m4b-t20-adversarial fix round 5 (re-review m-fr4-3): rooted with no drive (`\x`, `/x`).
    /// On Unix an absolute path; on Windows it resolves against the process's current drive,
    /// which may be a share, so it is not local there.
    Rooted,
    /// `C:\…` or `C:/…`, a drive path.
    Disk,
    /// `C:` or `C:x`, relative to that drive's current folder.
    DriveRelative,
    /// `\\?\C:\…`: the verbatim drive path that a canonical library root itself uses.
    VerbatimDisk,
    /// `\\host\share`, `//host/share`: Windows contacts the host over SMB and sends the user's
    /// NTLM credentials.
    Unc,
    /// `\\?\UNC\host\share`, the same contact.
    VerbatimUnc,
    /// The device and object namespaces: `\\.\…`, `//./…`, `\??\…`, and `\\?\` followed by
    /// anything but a drive or `UNC`.
    Device,
}

impl PathPrefix {
    /// Whether a path with this start may be handed to the filesystem: it stays on a local
    /// drive (a mapped or `subst` drive letter is the user's own setup, not the repository's).
    pub(crate) fn is_local(self) -> bool {
        match self {
            PathPrefix::None | PathPrefix::Disk | PathPrefix::VerbatimDisk => true,
            PathPrefix::Rooted => !cfg!(windows),
            _ => false,
        }
    }

    pub(crate) fn has_prefix(self) -> bool {
        self != PathPrefix::None
    }
}

/// The [`PathPrefix`] of `text` (`\` and `/` are both separators, except in the verbatim
/// `\\?\` form, which Windows reads only with backslashes).
pub(crate) fn path_prefix(text: &str) -> PathPrefix {
    let b = text.as_bytes();
    let sep = |i: usize| b.get(i).is_some_and(|c| matches!(c, b'\\' | b'/'));
    let drive =
        |i: usize| b.get(i).is_some_and(u8::is_ascii_alphabetic) && b.get(i + 1) == Some(&b':');
    if b.starts_with(br"\??\") {
        return PathPrefix::Device;
    }
    if sep(0) && sep(1) {
        if b.starts_with(br"\\?\") {
            let rest = &b[4..];
            let unc = rest.len() >= 3
                && rest[..3].eq_ignore_ascii_case(b"UNC")
                && matches!(rest.get(3), None | Some(b'\\'));
            if unc {
                return PathPrefix::VerbatimUnc;
            }
            if drive(4) && matches!(b.get(6), None | Some(b'\\')) {
                return PathPrefix::VerbatimDisk;
            }
            return PathPrefix::Device;
        }
        if matches!(b.get(2), Some(b'.' | b'?')) && (sep(3) || b.len() == 3) {
            return PathPrefix::Device;
        }
        return PathPrefix::Unc;
    }
    if drive(0) {
        return if sep(2) {
            PathPrefix::Disk
        } else {
            PathPrefix::DriveRelative
        };
    }
    if sep(0) {
        return PathPrefix::Rooted;
    }
    PathPrefix::None
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

/// One listed name: as listed, its NFC form, and whether it is a link (a symlink, or on Windows
/// a junction or other name-surrogate reparse point) by the listing's own `file_type`, which
/// follows nothing.
type Listed = (String, String, bool);

/// Directory listings, read once each (names only, never contents), and (Unix) each listing's
/// file ids, read once on the first case-only lookup in it (n4: no `lstat` per name per path).
#[derive(Default)]
pub(crate) struct Lister {
    dirs: HashMap<PathBuf, Listing>,
    #[cfg(unix)]
    ids: HashMap<PathBuf, HashMap<(u64, u64), String>>,
    /// The tests' counting seam: every path this lister listed or `lstat`ed.
    #[cfg(test)]
    touched: Vec<PathBuf>,
}

/// One folder's names, and (fix round 2) their positions by listed name and by NFC form, so a
/// lookup is no scan of the listing (the dataless check looks up every tracked path).
#[derive(Default)]
struct Listing {
    names: Vec<Listed>,
    exact: HashMap<String, usize>,
    by_nfc: HashMap<String, usize>,
}

impl Lister {
    /// The names in `dir` with their NFC forms.
    fn names(&mut self, dir: &Path) -> Option<&[Listed]> {
        self.listing(dir).map(|l| l.names.as_slice())
    }

    /// Records `p` in the tests' counting seam; nothing in a product build.
    fn touch(&mut self, p: &Path) {
        #[cfg(test)]
        self.touched.push(p.to_path_buf());
        #[cfg(not(test))]
        let _ = p;
    }

    fn listing(&mut self, dir: &Path) -> Option<&Listing> {
        if !self.dirs.contains_key(dir) {
            self.touch(dir);
            let names: Vec<Listed> = std::fs::read_dir(dir)
                .ok()?
                .filter_map(Result::ok)
                .filter_map(|e| {
                    // An entry whose type cannot be read counts as a link: never entered.
                    let link = e.file_type().map_or(true, |t| t.is_symlink());
                    e.file_name().into_string().ok().map(|n| (n, link))
                })
                .map(|(n, link)| {
                    let f = nfc(&n);
                    (n, f, link)
                })
                .collect();
            let mut listing = Listing::default();
            for (i, (n, f, _)) in names.iter().enumerate() {
                listing.exact.insert(n.clone(), i);
                listing.by_nfc.entry(f.clone()).or_insert(i);
            }
            listing.names = names;
            self.dirs.insert(dir.to_path_buf(), listing);
        }
        self.dirs.get(dir)
    }

    // ## m4b-cifix-2 fix round 2 (re-review N1): the dataless check's walk.
    /// The `lstat` of the index path `rel` under `base`, found by [`on_disk`](Self::on_disk)
    /// (no link entered on the way, a non-plain component refused) and `lstat`ed only at its
    /// last component; `None` when it is not there or not reachable without a link.
    pub(crate) fn lstat_under(&mut self, base: &Path, rel: &str) -> Option<std::fs::Metadata> {
        let found = self.on_disk(base, rel)?;
        self.touch(&found.full);
        std::fs::symlink_metadata(&found.full).ok()
    }

    /// `rel` (the index's spelling) as the disk spells it under `base`; `None` when the file
    /// system cannot find it.
    ///
    /// ## m4b-cifix-2 fix round 2 (re-review N1, R-CI2-2): it never descends into a link. Every
    /// component but the last must be a listed name that is not a link (from the listing's own
    /// `file_type`) or, found only by the file system's case folding, one whose `lstat` is not
    /// a link. So no folder outside `base` is listed or looked into, whatever the index names
    /// (`lib/link/x.sql` with `link -> /net/<host>` or `\\host\share`). The last component may
    /// be a link; callers only `lstat` it.
    fn on_disk(&mut self, base: &Path, rel: &str) -> Option<OnDisk> {
        let mut dir = base.to_path_buf();
        let mut parts: Vec<String> = Vec::new();
        let mut case_only = false;
        let count = rel.split('/').count();
        for (i, comp) in rel.split('/').enumerate() {
            // ## m4b-cifix-2-index-paths (C2): no file-system call on a name that is not plain.
            if !is_plain_index_path(comp.as_bytes()) {
                return None;
            }
            let last = i + 1 == count;
            let want = nfc(comp);
            let listing = self.listing(&dir)?;
            let exact = listing
                .exact
                .get(comp)
                .or_else(|| listing.by_nfc.get(&want))
                .map(|&i| {
                    let (n, _, link) = &listing.names[i];
                    (n.clone(), *link)
                });
            let found = match exact {
                Some((n, link)) => {
                    if link && !last {
                        return None;
                    }
                    n
                }
                None => {
                    // The file system's own lookup (case folding): never dropped (R-T7-1).
                    let probe = dir.join(comp);
                    self.touch(&probe);
                    let md = std::fs::symlink_metadata(&probe).ok()?;
                    if md.file_type().is_symlink() && !last {
                        return None;
                    }
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
            for (n, _, _) in self.names(dir)?.to_vec() {
                let p = dir.join(&n);
                self.touch(&p);
                if let Ok(m) = std::fs::symlink_metadata(&p) {
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
        let mut hits = names
            .iter()
            .filter(|(_, f, _)| f.eq_ignore_ascii_case(want));
        match (hits.next(), hits.next()) {
            (Some((n, _, _)), None) => Some(n.clone()),
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

    // ## m4b-t20-adversarial fix round 4 (re-review I-fr3-1)
    /// The prefix classifier, pure and the same on every OS.
    #[test]
    fn path_prefixes_are_classified_as_windows_reads_them() {
        use PathPrefix as P;
        let cases = [
            ("a/b", P::None),
            ("objects", P::None),
            ("../x", P::None),
            ("/abs/x", P::Rooted),
            (r"\rooted\x", P::Rooted),
            ("/", P::Rooted),
            ("", P::None),
            (r"C:\x", P::Disk),
            ("c:/x", P::Disk),
            ("C:", P::DriveRelative),
            ("C:x", P::DriveRelative),
            (r"\\?\C:\x", P::VerbatimDisk),
            (r"\\?\c:", P::VerbatimDisk),
            (r"\\host\share\x", P::Unc),
            ("//host/share/x", P::Unc),
            (r"\/host\share", P::Unc),
            (r"/\host/share", P::Unc),
            (r"\\", P::Unc),
            (r"\\?\UNC\host\share\x", P::VerbatimUnc),
            (r"\\?\unc\host\share", P::VerbatimUnc),
            (r"\\?\UNC", P::VerbatimUnc),
            (r"\\.\pipe\x", P::Device),
            (r"\\.\UNC\host\share", P::Device),
            ("//./c:/x", P::Device),
            (r"\\.", P::Device),
            (r"\??\UNC\host\share", P::Device),
            (r"\??\C:\x", P::Device),
            (r"\\?\GLOBALROOT\Device\Mup\host\share", P::Device),
            (r"\\?\Volume{0}\x", P::Device),
            ("//?/C:/x", P::Device),
            (r"\\?\C:x", P::Device),
        ];
        for (text, want) in cases {
            assert_eq!(path_prefix(text), want, "{text:?}");
        }
        let local: Vec<&str> = cases
            .iter()
            .filter(|(t, _)| path_prefix(t).is_local())
            .map(|(t, _)| *t)
            .collect();
        // Fix round 5 (m-fr4-3): a rooted path with no drive is local on Unix only.
        let mut want = vec!["a/b", "objects", "../x"];
        if !cfg!(windows) {
            want.extend(["/abs/x", r"\rooted\x", "/"]);
        }
        want.extend(["", r"C:\x", "c:/x", r"\\?\C:\x", r"\\?\c:"]);
        assert_eq!(local, want);
    }

    // ## m4b-cifix-2-index-paths (review C2, R-CI2-2)
    /// Hostile index spellings are never joined to a directory: not as an n2 alias head, not as
    /// a path. The structural cases are refused on every OS; fix round 2 (N2, R-CI2-4): the
    /// names only Windows interprets are refused on Windows, and are ordinary names elsewhere.
    #[test]
    fn hostile_index_paths_are_never_candidates() {
        let wt = Path::new(if cfg!(windows) {
            r"C:\w\repo"
        } else {
            "/w/repo"
        });
        // Every OS.
        let structural = [
            "..",      // the parent
            "../x",    // the parent, then a name
            "a/../b",  // a parent in the middle
            ".",       // the folder itself
            "/net/x",  // absolute (macOS's automounter)
            "/abs",    // absolute
            "a//b",    // an empty component
            "a/",      // a trailing empty component
            ".git",    // the repository itself
            ".GIT",    // its case-folded spelling
            "a\u{0}b", // a NUL
        ];
        // Only where Windows gives them a meaning.
        let windows = [
            r"\\host\share",   // UNC: Windows would contact the host
            r"\\host\share/x", // the same, as the first of two components
            r"C:\",            // a drive root
            r"C:\x",           // a drive path
            "C:",              // a bare drive
            "c:x",             // drive-relative
            r"\\?\C:\x",       // verbatim
            r"\\.\pipe\x",     // a device namespace path
            r"..\x",           // `..` behind a backslash
            "lib:stream",      // an NTFS alternate data stream
            "lib::$DATA",      // the default stream, spelled out
            "CON",             // a reserved device name
            "con.sql",         // a reserved device name with an extension
            "aux.sql",         // the same
            "a:b.sql",         // a stream again
            "lib.",            // a trailing dot (NTFS drops it)
            "lib ",            // a trailing space (NTFS drops it)
            "GIT~1",           // `.git`'s 8.3 short name
        ];
        let refused: Vec<&str> = if cfg!(windows) {
            structural.iter().chain(&windows).copied().collect()
        } else {
            structural.to_vec()
        };
        for h in refused {
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
        if !cfg!(windows) {
            for h in windows {
                assert!(
                    is_plain_index_path(h.as_bytes()),
                    "{h:?} is an ordinary name on this OS"
                );
            }
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
        // A backslash name is unsafe on Windows only; elsewhere it is looked up, and the library
        // here does not exist, so it has no badge.
        assert_eq!(
            out.get(r"\\host\share"),
            cfg!(windows).then_some(&FileStatus::Unknown(UnknownWhy::UnsafePath))
        );
    }

    // ## m4b-cifix-2 fix round 2 (re-review N2, R-CI2-4)
    /// Names that only Windows interprets keep their true badge on Linux and macOS, where git
    /// tracks them as ordinary files, and are `Unknown(UnsafePath)` on Windows (never looked
    /// up there).
    #[test]
    fn windows_only_names_keep_their_badge_off_windows() {
        let t = tempfile::tempdir().unwrap();
        let lib = std::fs::canonicalize(t.path()).unwrap();
        let names = ["con.sql", "aux.sql", "a:b.sql"];
        #[cfg(unix)]
        for n in names {
            std::fs::write(lib.join(n), b"select 1;\n").unwrap();
        }
        let mut b = Badges::default();
        for n in names {
            b.add(BStr::new(&format!("lib/{n}")), FileStatus::Modified);
        }
        let out = b.into_library("lib", false, &BTreeSet::new(), &lib, &never);
        let want = if cfg!(windows) {
            FileStatus::Unknown(UnknownWhy::UnsafePath)
        } else {
            FileStatus::Modified
        };
        for n in names {
            assert_eq!(out.get(n), Some(&want), "{n}");
        }
    }

    // ## m4b-cifix-2 fix round 2 (re-review N1, R-CI2-2)
    /// The counting seam: with `lib/link -> <outside folder>` and an index entry under the link
    /// (`link/x.sql`, also spelled by case only), nothing outside the library is listed or
    /// `lstat`ed, and the entry is not found. A real folder next to it still is, and the link
    /// itself (the last component) may be `lstat`ed.
    #[cfg(unix)]
    #[test]
    fn an_index_path_under_a_linked_folder_never_leaves_the_library() {
        let t = tempfile::tempdir().unwrap();
        let top = std::fs::canonicalize(t.path()).unwrap();
        let lib = top.join("lib");
        let outside = top.join("outside");
        std::fs::create_dir_all(lib.join("real")).unwrap();
        std::fs::create_dir_all(outside.join("deeper")).unwrap();
        std::fs::write(outside.join("x.sql"), b"select 1;\n").unwrap();
        std::fs::write(lib.join("real/a.sql"), b"select 2;\n").unwrap();
        std::os::unix::fs::symlink(&outside, lib.join("link")).unwrap();
        let mut walk = Lister::default();
        for rel in ["link/x.sql", "LINK/x.sql", "link/deeper/y.sql"] {
            assert!(walk.on_disk(&lib, rel).is_none(), "{rel}");
            assert!(walk.lstat_under(&lib, rel).is_none(), "{rel}");
        }
        assert!(walk.lstat_under(&lib, "real/a.sql").is_some());
        assert!(
            walk.lstat_under(&lib, "link")
                .is_some_and(|m| m.file_type().is_symlink())
        );
        let escaped: Vec<&PathBuf> = walk
            .touched
            .iter()
            .filter(|p| {
                p.starts_with(&outside)
                    || (p.starts_with(lib.join("link")) && **p != lib.join("link"))
            })
            .collect();
        assert!(
            escaped.is_empty(),
            "touched outside the library: {escaped:?}"
        );
        assert!(!walk.dirs.contains_key(&lib.join("link")));
        assert!(!walk.dirs.contains_key(&outside));
        // The badges path too: a change keyed under the link has no badge, and nothing outside
        // was looked at for it (`into_library` uses the same walk).
        let mut b = Badges::default();
        b.add(BStr::new("lib/link/x.sql"), FileStatus::Modified);
        b.add(BStr::new("lib/real/a.sql"), FileStatus::Modified);
        let out = b.into_library("lib", false, &BTreeSet::new(), &lib, &never);
        assert_eq!(out.get("link/x.sql"), None);
        assert_eq!(out.get("real/a.sql"), Some(&FileStatus::Modified));
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
