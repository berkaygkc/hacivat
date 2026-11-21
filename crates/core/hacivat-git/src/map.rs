//! From repository-relative changes to library-relative badges (HV-SCRIPT-11 steps 5 and 6).
//!
//! - **Precedence**: `Unknown` overrides every other badge (a filtered, dataless or submodule
//!   path is never shown as if its comparison were exact); then Conflicted > New > Modified
//!   (`alg/28` §0.4: `AM` is New, a staged rename's destination is New).
//! - **Keys**: the repository path minus the library prefix, spelled as the **disk** spells it,
//!   byte for byte, so that it equals the scanner's `RelPath` (R-T1a-2 carry). The index may
//!   hold a precomposed (NFC) name where macOS keeps the decomposed (NFD) one; each component
//!   is matched against its directory's listing, exactly first, then by NFC equality. A path
//!   that is not on the disk (deleted) has no badge, as the plan says; neither has a name that
//!   is not UTF-8 (the scanner cannot list it).
//! - **Dataless** (R-M4b-15): a cloud placeholder's flag is read from `lstat` metadata only.

// ## m4b-t07-git
use std::collections::{BTreeMap, HashMap};
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
        FileStatus::Unknown(UnknownWhy::Dataless) => 5,
        FileStatus::Unknown(UnknownWhy::Submodule) => 6,
    }
}

/// Repository-relative badges, merged by precedence.
#[derive(Debug, Default)]
pub(crate) struct Badges {
    by_repo_path: BTreeMap<BString, FileStatus>,
}

impl Badges {
    pub(crate) fn add(&mut self, path: &BStr, s: FileStatus) {
        let e = self.by_repo_path.entry(path.to_owned()).or_insert(s);
        if rank(s) > rank(*e) {
            *e = s;
        }
    }

    pub(crate) fn get(&self, path: &BStr) -> Option<FileStatus> {
        self.by_repo_path.get(path).copied()
    }

    pub(crate) fn paths(&self) -> Vec<BString> {
        self.by_repo_path.keys().cloned().collect()
    }

    /// The badges keyed relative to the library root at `library` (canonical), whose path
    /// relative to the worktree is `prefix`; dataless paths found on the way become
    /// `Unknown(Dataless)`.
    pub(crate) fn into_library(
        self,
        prefix: &str,
        library: &Path,
        is_dataless: &dyn Fn(&std::fs::Metadata) -> bool,
    ) -> BTreeMap<GitPath, FileStatus> {
        let mut lister = Lister::default();
        let mut out = BTreeMap::new();
        for (repo_path, s) in self.by_repo_path {
            let Ok(repo_path) = repo_path.to_str() else {
                continue;
            };
            let Some(rel) = strip_prefix(repo_path, prefix) else {
                continue;
            };
            let Some((disk, full)) = lister.on_disk(library, rel) else {
                continue;
            };
            let s = match s {
                FileStatus::Unknown(_) => s,
                _ if std::fs::symlink_metadata(&full).is_ok_and(|m| is_dataless(&m)) => {
                    FileStatus::Unknown(UnknownWhy::Dataless)
                }
                _ => s,
            };
            let key = GitPath::new(&disk);
            let e = out.entry(key).or_insert(s);
            if rank(s) > rank(*e) {
                *e = s;
            }
        }
        out
    }
}

/// `path` relative to the library `prefix` (`""` is the worktree), or `None` outside it.
pub(crate) fn strip_prefix<'a>(path: &'a str, prefix: &str) -> Option<&'a str> {
    if prefix.is_empty() {
        return Some(path).filter(|p| !p.is_empty());
    }
    path.strip_prefix(prefix)?
        .strip_prefix('/')
        .filter(|r| !r.is_empty())
}

/// Whether the repository path lies inside the library `prefix`.
pub(crate) fn under_prefix(path: &BStr, prefix: &str) -> bool {
    prefix.is_empty()
        || path
            .strip_prefix(prefix.as_bytes())
            .is_some_and(|r| r.first() == Some(&b'/'))
}

/// Directory listings, read once each (names only, never contents).
#[derive(Default)]
struct Lister {
    dirs: HashMap<PathBuf, Vec<(String, String)>>,
}

impl Lister {
    /// The names in `dir` with their NFC forms.
    fn names(&mut self, dir: &Path) -> Option<&[(String, String)]> {
        if !self.dirs.contains_key(dir) {
            let names = std::fs::read_dir(dir)
                .ok()?
                .filter_map(Result::ok)
                .filter_map(|e| e.file_name().into_string().ok())
                .map(|n| {
                    let nfc = gix::utils::str::precompose(n.as_str().into()).into_owned();
                    (n, nfc)
                })
                .collect();
            self.dirs.insert(dir.to_path_buf(), names);
        }
        self.dirs.get(dir).map(Vec::as_slice)
    }

    /// `rel` as the disk spells it under `base`, and its full path; `None` when absent.
    fn on_disk(&mut self, base: &Path, rel: &str) -> Option<(String, PathBuf)> {
        let mut dir = base.to_path_buf();
        let mut parts: Vec<String> = Vec::new();
        for comp in rel.split('/') {
            if comp.is_empty() || comp == "." || comp == ".." {
                return None;
            }
            let names = self.names(&dir)?;
            let found = match names.iter().find(|(n, _)| n == comp) {
                Some((n, _)) => n.clone(),
                None => {
                    let want = gix::utils::str::precompose(comp.into());
                    names.iter().find(|(_, nfc)| *nfc == want)?.0.clone()
                }
            };
            dir.push(&found);
            parts.push(found);
        }
        Some((parts.join("/"), dir))
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
        md.file_attributes() & (FILE_ATTRIBUTE_RECALL_ON_OPEN | FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS)
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

    #[test]
    fn prefix_is_stripped_and_paths_outside_are_dropped() {
        assert_eq!(strip_prefix("lib/a.sql", "lib"), Some("a.sql"));
        assert_eq!(strip_prefix("lib/x/a.sql", "lib"), Some("x/a.sql"));
        assert_eq!(strip_prefix("library/a.sql", "lib"), None);
        assert_eq!(strip_prefix("lib", "lib"), None);
        assert_eq!(strip_prefix("a.sql", ""), Some("a.sql"));
        assert!(under_prefix("lib/a.sql".into(), "lib"));
        assert!(!under_prefix("library/a.sql".into(), "lib"));
        assert!(under_prefix("anything".into(), ""));
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
        let out = b.into_library("lib", &lib, &never);
        let want_key = format!("{nfd_dir}/{nfd_file}");
        assert_eq!(out.get(want_key.as_str()), Some(&FileStatus::Modified), "{out:?}");
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
        let out = b.into_library("", &lib, &|m| m.len() == 0);
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
}
