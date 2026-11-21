//! M4b Task 4: the library scan (HV-SCRIPT-03; R-M4b-12..15, alg/28 C11): names and `lstat`
//! only, never a content read, never a directory link followed, limits that stop with a notice.

use std::cell::Cell;
use std::collections::HashMap;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use hacivat_proto::EnvTag::{self, Production, Staging};
use hacivat_scripts::pathsafe::{NameProblem, display_name};
use hacivat_scripts::scan::{
    Diag, Fs, LimitHit, Limits, Meta, MetaKind, RealFs, Resolved, RootProblem, ScanResult, scan,
};
use hacivat_scripts::{
    EntryKind, EnvScope, EnvSet, LibraryId, LibraryRoot, LinkState, ScopeProblem, ScriptEntry,
    dataless, tree, visible,
};

// ---------------------------------------------------------------------------------------------
// An in-memory file system: the scan sees only what the `Fs` trait gives it.

#[derive(Debug, Clone)]
enum Node {
    Dir,
    File { len: u64, dataless: bool },
    Link(PathBuf),
    Fifo,
}

#[derive(Default)]
struct FakeFs {
    nodes: HashMap<PathBuf, Node>,
    children: HashMap<PathBuf, Vec<OsString>>,
    /// A lazy chain `/lib/d/d/…/d/f.sql` of this many components (the 10,000-deep fixture,
    /// whose eager build would hash 50 million path components).
    chain: Option<usize>,
    read_dirs: Cell<usize>,
    lstats: Cell<usize>,
    resolves: Cell<usize>,
    canonicalizes: Cell<usize>,
}

const ROOT: &str = "/lib";

impl FakeFs {
    fn new() -> Self {
        let mut fs = FakeFs::default();
        fs.nodes.insert(PathBuf::from("/"), Node::Dir);
        fs.children.insert(PathBuf::from("/"), Vec::new());
        fs.put(PathBuf::from(ROOT), Node::Dir);
        fs
    }

    /// Inserts `node` at `path`, creating missing folders on the way (iteratively: the
    /// 10,000-deep fixture would overflow a recursive helper).
    fn insert(&mut self, path: PathBuf, node: Node) {
        let mut missing = Vec::new();
        let mut up = path.parent();
        while let Some(p) = up {
            if self.nodes.contains_key(p) {
                break;
            }
            missing.push(p.to_path_buf());
            up = p.parent();
        }
        for dir in missing.into_iter().rev() {
            self.put(dir, Node::Dir);
        }
        self.put(path, node);
    }

    fn put(&mut self, path: PathBuf, node: Node) {
        let parent = path.parent().unwrap().to_path_buf();
        let name = path.file_name().unwrap().to_os_string();
        if matches!(node, Node::Dir) {
            self.children.entry(path.clone()).or_default();
        }
        if self.nodes.insert(path, node).is_none() {
            self.children.get_mut(&parent).unwrap().push(name);
        }
    }

    fn abs(rel: &str) -> PathBuf {
        Path::new(ROOT).join(rel)
    }

    fn file(&mut self, rel: &str) -> &mut Self {
        self.insert(
            Self::abs(rel),
            Node::File {
                len: 7,
                dataless: false,
            },
        );
        self
    }

    fn os_file(&mut self, parent: &str, name: &OsStr) -> &mut Self {
        let p = if parent.is_empty() {
            PathBuf::from(ROOT)
        } else {
            Self::abs(parent)
        };
        self.insert(
            p.join(name),
            Node::File {
                len: 7,
                dataless: false,
            },
        );
        self
    }

    fn dataless(&mut self, rel: &str) -> &mut Self {
        self.insert(
            Self::abs(rel),
            Node::File {
                len: 4096,
                dataless: true,
            },
        );
        self
    }

    fn dir(&mut self, rel: &str) -> &mut Self {
        self.insert(Self::abs(rel), Node::Dir);
        self
    }

    fn link(&mut self, rel: &str, target: &str) -> &mut Self {
        self.insert(Self::abs(rel), Node::Link(PathBuf::from(target)));
        self
    }

    fn fifo(&mut self, rel: &str) -> &mut Self {
        self.insert(Self::abs(rel), Node::Fifo);
        self
    }

    /// Follows every link of `p` (at most 40 hops, like the kernel).
    fn real(&self, p: &Path, hops: &mut usize) -> io::Result<PathBuf> {
        let mut out = PathBuf::new();
        for c in p.components() {
            out.push(c);
            loop {
                match self.nodes.get(&out) {
                    None => return Err(io::ErrorKind::NotFound.into()),
                    Some(Node::Link(t)) => {
                        *hops += 1;
                        if *hops > 40 {
                            return Err(io::Error::other("loop"));
                        }
                        let t = if t.is_absolute() {
                            t.clone()
                        } else {
                            out.parent().unwrap().join(t)
                        };
                        out = self.real(&t, hops)?;
                    }
                    Some(_) => break,
                }
            }
        }
        Ok(out)
    }
}

impl FakeFs {
    /// Where `p` sits on the lazy chain: `Some(k)` for `/lib` plus `k` components that are all
    /// `d` but the last, which may be `f.sql` at the chain's end.
    fn on_chain(&self, p: &Path) -> Option<(usize, bool)> {
        let n = self.chain?;
        let rest = p.strip_prefix(ROOT).ok()?;
        let parts: Vec<&OsStr> = rest.iter().collect();
        let k = parts.len();
        let file = parts.last().is_some_and(|l| *l == "f.sql");
        let dirs_ok = parts[..k.saturating_sub(usize::from(file))]
            .iter()
            .all(|c| *c == "d");
        (dirs_ok && k <= n && (!file || k == n)).then_some((k, file))
    }
}

impl Fs for FakeFs {
    fn read_dir(&self, dir: &Path) -> io::Result<Vec<OsString>> {
        self.read_dirs.set(self.read_dirs.get() + 1);
        if let Some((k, false)) = self.on_chain(dir) {
            let n = self.chain.unwrap_or(0);
            let next = if k + 1 == n { "f.sql" } else { "d" };
            return Ok(vec![OsString::from(next)]);
        }
        self.children
            .get(dir)
            .cloned()
            .ok_or_else(|| io::ErrorKind::NotFound.into())
    }

    fn symlink_metadata(&self, path: &Path) -> io::Result<Meta> {
        self.lstats.set(self.lstats.get() + 1);
        if let Some((k, file)) = self.on_chain(path)
            && k > 0
        {
            let kind = if file { MetaKind::File } else { MetaKind::Dir };
            return Ok(Meta {
                kind,
                len: 0,
                mtime: SystemTime::UNIX_EPOCH,
                dataless: false,
            });
        }
        let node = self.nodes.get(path).ok_or(io::ErrorKind::NotFound)?;
        let (kind, len, dataless) = match node {
            Node::Dir => (MetaKind::Dir, 0, false),
            Node::File { len, dataless } => (MetaKind::File, *len, *dataless),
            Node::Link(t) => (MetaKind::Symlink, t.as_os_str().len() as u64, false),
            Node::Fifo => (MetaKind::Other, 0, false),
        };
        Ok(Meta {
            kind,
            len,
            mtime: SystemTime::UNIX_EPOCH,
            dataless,
        })
    }

    fn canonicalize(&self, path: &Path) -> io::Result<PathBuf> {
        self.canonicalizes.set(self.canonicalizes.get() + 1);
        self.real(path, &mut 0)
    }

    fn resolve(&self, link: &Path) -> Resolved {
        self.resolves.set(self.resolves.get() + 1);
        let mut hops = 0;
        match self.real(link, &mut hops) {
            Ok(t) => Resolved::Target(t),
            Err(_) if hops > 40 => Resolved::Loop,
            Err(e) if e.kind() == io::ErrorKind::NotFound => Resolved::Dangling,
            Err(e) => Resolved::Error(e.kind()),
        }
    }
}

fn root_of(_fs: &FakeFs) -> LibraryRoot {
    LibraryRoot {
        id: LibraryId::new(),
        path: PathBuf::from(ROOT),
        canonical: PathBuf::from(ROOT),
    }
}

fn scan_fake(fs: &FakeFs) -> ScanResult {
    scan(&root_of(fs), &Limits::default(), fs)
}

/// A fixture of `rel` paths: a name ending in `/` is a folder, anything else a file.
fn fixture(paths: &[&str]) -> FakeFs {
    let mut fs = FakeFs::new();
    for p in paths {
        match p.strip_suffix('/') {
            Some(d) => fs.dir(d),
            None => fs.file(p),
        };
    }
    fs
}

fn scan_fixture(paths: &[&str]) -> ScanResult {
    scan_fake(&fixture(paths))
}

/// `n` scripts, 100 per folder.
fn fake_tree(n: usize) -> FakeFs {
    let mut fs = FakeFs::new();
    for i in 0..n {
        fs.file(&format!("d{:04}/f{i:06}.sql", i / 100));
    }
    fs
}

fn names(r: &ScanResult) -> Vec<&str> {
    r.entries.iter().map(|e| e.rel.as_str()).collect()
}

fn files(r: &ScanResult) -> Vec<&str> {
    r.entries
        .iter()
        .filter(|e| e.kind != EntryKind::Folder)
        .map(|e| e.rel.as_str())
        .collect()
}

fn find<'a>(r: &'a ScanResult, rel: &str) -> &'a ScriptEntry {
    r.entries
        .iter()
        .find(|e| e.rel.as_str() == rel)
        .unwrap_or_else(|| panic!("{rel} not listed in {:?}", names(r)))
}

// ---------------------------------------------------------------------------------------------
// Listing rules.

#[test]
fn skipped_folders() {
    let r = scan_fixture(&[
        ".git/x.sql",
        ".hacivat/x.sql",
        ".idea/x.sql",
        "node_modules/x.sql",
        "ok/x.sql",
    ]);
    assert_eq!(names(&r), ["ok", "ok/x.sql"]);
    assert!(r.diags.is_empty(), "{:?}", r.diags);
}

#[test]
fn node_modules_is_skipped_ascii_case_insensitive_only() {
    let r = scan_fixture(&[
        "NODE_MODULES/a.sql",
        "Node_Modules/b.sql",
        "node_modulés/c.sql",
        "x/node_modules/d.sql",
    ]);
    assert_eq!(names(&r), ["node_modulés", "node_modulés/c.sql"]);
}

#[test]
fn dot_files_are_not_listed() {
    // alg/28 C11: `sed -i` writes `.!<pid>!a.sql`, vim `.a.sql.swp`.
    let r = scan_fixture(&[
        ".!123!a.sql",
        ".a.sql.swp",
        ".hidden.sql",
        "x/.y.sql",
        "x/ok.sql",
        ".md",
    ]);
    assert_eq!(names(&r), ["x", "x/ok.sql"]);
}

#[test]
fn only_sql_and_md_are_listed_ascii_ci() {
    let r = scan_fixture(&[
        "a.SQL",
        "b.md",
        "c.txt",
        "d.sql.bak",
        "e.Md",
        "f.\u{FF33}QL",
        "g.sq\u{130}",
        "sql",
        "md",
    ]);
    assert_eq!(files(&r), ["a.SQL", "b.md", "e.Md"]);
    assert_eq!(find(&r, "a.SQL").kind, EntryKind::Sql);
    assert_eq!(find(&r, "e.Md").kind, EntryKind::Markdown);
}

#[test]
fn folders_without_scripts_are_not_listed() {
    let r = scan_fixture(&[
        "empty/",
        "docs/readme.txt",
        "a/b/c/d.sql",
        "a/b/e/",
        "x.sql/y.md",
    ]);
    assert_eq!(
        names(&r),
        ["a", "a/b", "a/b/c", "a/b/c/d.sql", "x.sql", "x.sql/y.md"]
    );
    // A folder named `x.sql` is a folder.
    assert_eq!(find(&r, "x.sql").kind, EntryKind::Folder);
}

#[test]
fn entries_are_sorted_folders_first_then_exact_bytes() {
    let r = scan_fixture(&[
        "b.sql",
        "a/z.sql",
        "A.sql",
        "a/b/c.sql",
        "a/a.sql",
        "B/x.sql",
        "\u{e9}.sql",
        "e\u{301}.sql",
    ]);
    assert_eq!(
        names(&r),
        [
            "B",
            "B/x.sql",
            "a",
            "a/b",
            "a/b/c.sql",
            "a/a.sql",
            "a/z.sql",
            "A.sql",
            "b.sql",
            "e\u{301}.sql",
            "\u{e9}.sql"
        ]
    );
    // `tree::sort` is the same order, whatever order it is given.
    let mut shuffled: Vec<ScriptEntry> = r.entries.iter().rev().cloned().collect();
    tree::sort(&mut shuffled);
    assert_eq!(shuffled, r.entries);
}

#[test]
fn other_file_kinds_are_not_listed() {
    let mut fs = FakeFs::new();
    fs.fifo("pipe.sql").file("ok.sql");
    let r = scan_fake(&fs);
    assert_eq!(names(&r), ["ok.sql"]);
}

// ---------------------------------------------------------------------------------------------
// Names.

#[test]
fn bad_names_are_skipped_with_an_escaped_diagnostic() {
    let long = "a".repeat(252) + ".sql";
    let mut fs = fixture(&["ok.sql"]);
    fs.os_file("", OsStr::new(&long))
        .os_file("", OsStr::new("a\nb.sql"))
        .os_file("", OsStr::new("bell\u{7}.sql"))
        .os_file("", OsStr::new("esc\u{1b}[2J.sql"));
    // A `\` inside one name exists only on Unix (on Windows it separates components).
    if cfg!(unix) {
        fs.os_file("", OsStr::new("back\\slash.sql"));
    }
    let r = scan_fake(&fs);
    assert_eq!(names(&r), ["ok.sql"]);
    let bad: Vec<(&str, NameProblem)> = r
        .diags
        .iter()
        .filter_map(|d| match d {
            Diag::BadName {
                parent: None,
                name,
                problem,
            } => Some((name.as_str(), *problem)),
            _ => None,
        })
        .collect();
    assert!(
        bad.contains(&("a<U+000A>b.sql", NameProblem::Control)),
        "{bad:?}"
    );
    assert!(
        bad.contains(&("bell<U+0007>.sql", NameProblem::Control)),
        "{bad:?}"
    );
    assert!(
        bad.contains(&("esc<U+001B>[2J.sql", NameProblem::Control)),
        "{bad:?}"
    );
    if cfg!(unix) {
        assert!(
            bad.contains(&("back\\slash.sql", NameProblem::Separator)),
            "{bad:?}"
        );
    }
    assert!(
        bad.iter().any(|(_, p)| *p == NameProblem::TooLong),
        "{bad:?}"
    );
    assert_eq!(bad.len(), if cfg!(unix) { 5 } else { 4 });
}

#[cfg(unix)]
#[test]
fn non_utf8_names_are_skipped() {
    use std::os::unix::ffi::OsStrExt;
    let mut fs = fixture(&["ok.sql"]);
    fs.os_file("", OsStr::from_bytes(b"caf\xe9.sql"));
    let r = scan_fake(&fs);
    assert_eq!(names(&r), ["ok.sql"]);
    assert!(
        matches!(
            r.diags.as_slice(),
            [Diag::BadName { name, problem: NameProblem::NotUtf8, .. }] if name == "caf\u{FFFD}.sql"
        ),
        "{:?}",
        r.diags
    );
}

#[test]
fn nfd_names_display_as_nfc_but_keep_os_bytes() {
    let d = tempfile::tempdir().unwrap();
    let nfd = "mu\u{0308}s\u{0327}teri.sql";
    fs::write(d.path().join(nfd), "SELECT 1;").unwrap();
    let r = scan(&real_root(d.path()), &Limits::default(), &RealFs);
    assert_eq!(r.entries.len(), 1);
    let e = &r.entries[0];
    // Identity is what the file system returned (APFS and ext4 keep the bytes as created).
    let listed: Vec<OsString> = fs::read_dir(d.path())
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(OsStr::new(e.rel.as_str()), listed[0].as_os_str());
    assert_eq!(e.rel.display_text(), "müşteri.sql");
}

#[test]
fn bidi_controls_in_names_are_escaped_for_display() {
    assert_eq!(display_name("a\u{202E}lqs.sql"), "a<U+202E>lqs.sql");
    let r = scan_fixture(&["a\u{202E}lqs.sql"]);
    assert_eq!(r.entries[0].rel.as_str(), "a\u{202E}lqs.sql");
    assert_eq!(r.entries[0].rel.display_text(), "a<U+202E>lqs.sql");
}

// ---------------------------------------------------------------------------------------------
// Environment folders.

#[test]
fn unknown_env_folder_raises_one_diagnostic_naming_a_suggestion() {
    let r = scan_fixture(&["@prod/a.sql", "@prod/b.sql"]);
    assert_eq!(
        r.diags
            .iter()
            .filter(|d| matches!(d, Diag::UnknownEnvFolder { name, suggestion: Some(EnvTag::Production) } if name == "@prod"))
            .count(),
        1
    );
    // The files are listed (the tree hides them), and hidden under every connection.
    for f in ["@prod/a.sql", "@prod/b.sql"] {
        let e = find(&r, f);
        assert!(matches!(&e.scope, EnvScope::Hidden(ScopeProblem::Unknown(n)) if n == "@prod"));
        for tag in EnvTag::ALL {
            assert!(!visible(e, Some(tag)));
        }
        assert!(!visible(e, None));
    }
}

#[test]
fn one_diagnostic_per_unknown_name_and_per_contradiction() {
    let r = scan_fixture(&[
        "x/@prod/a.sql",
        "y/@prod/b.sql",
        "@stg/c.sql",
        "@production/@staging/d.sql",
        "@production/@staging/e.sql",
    ]);
    let unknown: Vec<_> = r
        .diags
        .iter()
        .filter_map(|d| match d {
            Diag::UnknownEnvFolder { name, suggestion } => Some((name.as_str(), *suggestion)),
            _ => None,
        })
        .collect();
    let mut unknown = unknown;
    unknown.sort();
    assert_eq!(unknown, [("@prod", Some(Production)), ("@stg", None)]);
    let contra: Vec<_> = r
        .diags
        .iter()
        .filter_map(|d| match d {
            Diag::EnvContradiction { folder } => Some(folder.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(contra, ["@production/@staging"]);
}

#[test]
fn turkish_dotted_i_in_an_env_folder_hides_and_suggests() {
    let r = scan_fixture(&[
        "@productİon/a.sql",
        "@PRODUCTION/b.sql",
        "@productıon/c.sql",
    ]);
    assert!(matches!(
        find(&r, "@productİon/a.sql").scope,
        EnvScope::Hidden(_)
    ));
    assert!(matches!(
        find(&r, "@productıon/c.sql").scope,
        EnvScope::Hidden(_)
    ));
    assert_eq!(
        find(&r, "@PRODUCTION/b.sql").scope,
        EnvScope::Only(EnvSet::of(Production))
    );
    assert!(visible(find(&r, "@PRODUCTION/b.sql"), Some(Production)));
    for n in ["@productİon", "@productıon"] {
        assert!(
            r.diags.iter().any(|d| matches!(d, Diag::UnknownEnvFolder { name, suggestion: Some(Production) } if name == n)),
            "{n}: {:?}",
            r.diags
        );
    }
}

#[test]
fn stored_scopes_follow_the_folders() {
    let r = scan_fixture(&["@production/a.sql", "@staging/x/b.sql", "c.sql"]);
    assert_eq!(
        find(&r, "@production").scope,
        EnvScope::Only(EnvSet::of(Production))
    );
    assert_eq!(
        find(&r, "@production/a.sql").scope,
        EnvScope::Only(EnvSet::of(Production))
    );
    assert_eq!(
        find(&r, "@staging/x/b.sql").scope,
        EnvScope::Only(EnvSet::of(Staging))
    );
    assert_eq!(find(&r, "c.sql").scope, EnvScope::Any);
}

// ---------------------------------------------------------------------------------------------
// Links (in memory; the real-file-system twins are below).

#[test]
fn fake_links_resolve_once() {
    let mut fs = FakeFs::new();
    fs.file("shared/q.sql")
        .link("in.sql", "/lib/shared/q.sql")
        .link("out.sql", "/elsewhere/q.sql")
        .link("dangling.sql", "/lib/missing.sql")
        .link("a.sql", "/lib/b.sql")
        .link("b.sql", "/lib/a.sql")
        .link("dirlink", "/lib/shared")
        .link("dir.sql", "/lib/shared");
    fs.insert(
        PathBuf::from("/elsewhere/q.sql"),
        Node::File {
            len: 1,
            dataless: false,
        },
    );
    let r = scan_fake(&fs);
    assert_eq!(find(&r, "in.sql").link, LinkState::InsideLink);
    assert_eq!(find(&r, "out.sql").link, LinkState::OutsideLink);
    assert_eq!(find(&r, "dangling.sql").link, LinkState::OutsideLink);
    assert_eq!(find(&r, "a.sql").link, LinkState::Loop);
    assert_eq!(find(&r, "b.sql").link, LinkState::Loop);
    // A link to a folder is not followed; one named like a script is listed disabled.
    assert!(!names(&r).iter().any(|n| n.starts_with("dirlink")));
    assert_eq!(find(&r, "dir.sql").link, LinkState::OutsideLink);
    // Never `lstat` outside the root: the outside target is resolved, not inspected.
    assert_eq!(fs.resolves.get(), 6);
}

#[test]
fn a_link_carries_the_scope_of_its_target() {
    // A plain `a.sql` (shown on production) pointing at a staging-only script must not show
    // the staging script on production: the entry's scope is the intersection.
    let mut fs = FakeFs::new();
    fs.file("@staging/drop_all.sql")
        .link("a.sql", "/lib/@staging/drop_all.sql")
        .file("@production/p.sql")
        .link("@staging/p.sql", "/lib/@production/p.sql")
        .file("@prod/q.sql")
        .link("q.sql", "/lib/@prod/q.sql");
    let r = scan_fake(&fs);
    let a = find(&r, "a.sql");
    assert_eq!(a.link, LinkState::InsideLink);
    assert_eq!(a.scope, EnvScope::Only(EnvSet::of(Staging)));
    assert!(visible(a, Some(Staging)));
    assert!(!visible(a, Some(Production)));
    assert!(!visible(a, None));
    let p = find(&r, "@staging/p.sql");
    assert_eq!(p.scope, EnvScope::Hidden(ScopeProblem::Contradiction));
    for tag in EnvTag::ALL {
        assert!(!visible(p, Some(tag)));
    }
    assert!(matches!(
        find(&r, "q.sql").scope,
        EnvScope::Hidden(ScopeProblem::Unknown(_))
    ));
}

#[test]
fn dataless_flag_is_read_from_metadata_only() {
    let mut fs = FakeFs::new();
    fs.dataless("cloud.sql").file("local.sql");
    let r = scan_fake(&fs);
    assert!(find(&r, "cloud.sql").dataless);
    assert_eq!(find(&r, "cloud.sql").len, 4096);
    assert!(!find(&r, "local.sql").dataless);
    // The flags the real file system reports (macOS `st_flags`, Windows attributes).
    assert!(dataless::from_st_flags(1 << 30));
    assert!(dataless::from_st_flags((1 << 30) | 1));
    assert!(!dataless::from_st_flags(0));
    assert!(!dataless::from_st_flags(1 << 5));
    assert!(dataless::from_file_attributes(1 << 22));
    assert!(dataless::from_file_attributes((1 << 22) | 32));
    assert!(!dataless::from_file_attributes(32));
}

#[test]
fn scan_never_reads_contents() {
    // The `Fs` seam has no way to read a file; the counts show one `lstat` per entry and one
    // `read_dir` per folder, nothing else.
    let fs = fake_tree(500);
    let r = scan(&root_of(&fs), &Limits::default(), &fs);
    assert_eq!(files(&r).len(), 500);
    assert_eq!(fs.read_dirs.get(), 1 + 5);
    assert_eq!(fs.lstats.get(), 500 + 5);
    assert_eq!(fs.resolves.get(), 0);
    assert_eq!(
        fs.canonicalizes.get(),
        2,
        "the root, as chosen and as stored"
    );
}

// ---------------------------------------------------------------------------------------------
// Limits.

#[test]
fn scan_limits_stop_with_a_notice() {
    let fake = fake_tree(20_005);
    let r = scan(&root_of(&fake), &Limits::default(), &fake);
    assert_eq!(files(&r).len(), 20_000);
    assert!(matches!(r.stopped, Some(LimitHit::Files(20_000))));
}

#[test]
fn exactly_the_file_limit_does_not_stop() {
    let fake = fake_tree(20_000);
    let r = scan(&root_of(&fake), &Limits::default(), &fake);
    assert_eq!(files(&r).len(), 20_000);
    assert_eq!(r.stopped, None);
}

#[test]
fn ten_thousand_files_are_all_listed() {
    let fake = fake_tree(10_000);
    let r = scan(&root_of(&fake), &Limits::default(), &fake);
    assert_eq!(files(&r).len(), 10_000);
    assert_eq!(names(&r).len(), 10_100);
    assert_eq!(r.stopped, None);
}

fn chain(depth: usize) -> String {
    (1..depth).map(|i| format!("d{i}/")).collect::<String>() + "f.sql"
}

#[test]
fn depth_limit_is_32() {
    // A file with 32 components is listed; one with 33 stops the scan.
    let at = chain(32);
    assert_eq!(at.split('/').count(), 32);
    let r = scan_fixture(&[&at]);
    assert_eq!(files(&r), [at.as_str()]);
    assert_eq!(r.stopped, None);
    let deeper = chain(33);
    let r = scan_fixture(&["a.sql", &deeper]);
    assert!(
        matches!(r.stopped, Some(LimitHit::Depth(32))),
        "{:?}",
        r.stopped
    );
    assert!(files(&r).contains(&"a.sql"));
    assert!(!files(&r).contains(&deeper.as_str()));
}

#[test]
fn a_ten_thousand_deep_tree_stops_at_the_depth_limit() {
    // No recursion: an explicit stack, bounded by the depth limit.
    let mut fs = FakeFs::new();
    fs.chain = Some(10_000);
    let r = scan_fake(&fs);
    assert!(matches!(r.stopped, Some(LimitHit::Depth(32))));
    assert!(r.entries.is_empty());
    // It went down exactly to the limit: 32 folders read, 33 names looked at.
    assert_eq!(fs.read_dirs.get(), 1 + 32);
    assert_eq!(fs.lstats.get(), 33);
}

#[test]
fn the_lazy_chain_lists_its_file_when_short_enough() {
    // The chain double itself, at a depth the scan lists.
    let mut fs = FakeFs::new();
    fs.chain = Some(32);
    let r = scan_fake(&fs);
    assert_eq!(r.stopped, None);
    assert_eq!(files(&r), [chain_d(32).as_str()]);
}

fn chain_d(depth: usize) -> String {
    "d/".repeat(depth - 1) + "f.sql"
}

#[test]
fn visited_and_diagnostic_limits_bound_a_hostile_tree() {
    let mut fs = FakeFs::new();
    for i in 0..300 {
        fs.file(&format!("n{i:03}.txt"));
    }
    let limits = Limits {
        max_visited: 100,
        ..Limits::default()
    };
    let r = scan(&root_of(&fs), &limits, &fs);
    assert!(matches!(r.stopped, Some(LimitHit::Visited(100))));
    assert!(fs.lstats.get() <= 100);

    let mut fs = FakeFs::new();
    for i in 0..300 {
        fs.os_file("", OsStr::new(&format!("bad\u{1}{i:03}.sql")));
    }
    let r = scan_fake(&fs);
    assert_eq!(r.diags.len(), Limits::default().max_diags);
    assert_eq!(r.suppressed, 300 - Limits::default().max_diags);
}

#[test]
fn default_limits_are_the_plans() {
    let l = Limits::default();
    assert_eq!((l.max_files, l.max_depth), (20_000, 32));
}

// ---------------------------------------------------------------------------------------------
// The root.

fn real_root(path: &Path) -> LibraryRoot {
    LibraryRoot {
        id: LibraryId::new(),
        path: path.to_path_buf(),
        canonical: fs::canonicalize(path).unwrap(),
    }
}

#[test]
fn root_is_compared_canonically() {
    let d = tempfile::tempdir().unwrap();
    fs::create_dir(d.path().join("x")).unwrap();
    fs::write(d.path().join("x/a.sql"), "x").unwrap();
    fs::write(d.path().join("b.md"), "x").unwrap();
    let canonical = fs::canonicalize(d.path()).unwrap();
    let mut spelled = d.path().as_os_str().to_os_string();
    spelled.push(std::path::MAIN_SEPARATOR_STR);
    let chosen = LibraryRoot {
        id: LibraryId::new(),
        path: PathBuf::from(spelled),
        canonical: canonical.clone(),
    };
    let a = scan(&chosen, &Limits::default(), &RealFs);
    let b = scan(&real_root(&canonical), &Limits::default(), &RealFs);
    assert_eq!(a.root, None);
    assert_eq!(names(&a), ["x", "x/a.sql", "b.md"]);
    assert_eq!(names(&a), names(&b));
}

#[test]
fn a_missing_or_stale_root_lists_nothing() {
    let d = tempfile::tempdir().unwrap();
    fs::create_dir(d.path().join("a")).unwrap();
    fs::write(d.path().join("a/x.sql"), "x").unwrap();
    let root = real_root(&d.path().join("a"));
    let stale = LibraryRoot {
        canonical: root.canonical.join("..").join("a"),
        ..root.clone()
    };
    let r = scan(&stale, &Limits::default(), &RealFs);
    assert_eq!(r.root, Some(RootProblem::Changed));
    assert!(r.entries.is_empty());
    fs::remove_file(d.path().join("a/x.sql")).unwrap();
    fs::remove_dir(d.path().join("a")).unwrap();
    let r = scan(&root, &Limits::default(), &RealFs);
    assert_eq!(r.root, Some(RootProblem::Missing));
    assert!(r.entries.is_empty());
}

// ---------------------------------------------------------------------------------------------
// The real file system, Unix: links, loops, FIFOs, permissions.

#[cfg(unix)]
mod unix {
    use std::os::unix::fs::{PermissionsExt, symlink};
    use std::sync::mpsc;

    use super::{
        Diag, Duration, EnvScope, EnvSet, LibraryId, LibraryRoot, Limits, LinkState, Path,
        Production, RealFs, RootProblem, ScanResult, Staging, files, find, fs, names, real_root,
        scan, visible,
    };
    use hacivat_scripts::pathsafe::join;

    fn mkfifo(p: &Path) {
        let ok = std::process::Command::new("mkfifo")
            .arg(p)
            .status()
            .unwrap()
            .success();
        assert!(ok);
    }

    /// Scans on another thread and waits for the result on a condition (10 s ceiling): a scan
    /// that opened a FIFO would block forever.
    fn scan_bounded(root: LibraryRoot) -> ScanResult {
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(scan(&root, &Limits::default(), &RealFs));
        });
        rx.recv_timeout(Duration::from_secs(10))
            .expect("the scan blocked: it opened a file")
    }

    #[test]
    fn symlink_outside_is_listed_disabled_and_never_opened() {
        let d = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let secret = outside.path().join("secret.sql");
        fs::write(&secret, "DROP TABLE accounts;").unwrap();
        let before = fs::metadata(&secret).unwrap().accessed().unwrap();
        symlink(&secret, d.path().join("secret.sql")).unwrap();
        let r = scan(&real_root(d.path()), &Limits::default(), &RealFs);
        let e = find(&r, "secret.sql");
        assert_eq!(e.link, LinkState::OutsideLink);
        assert!(join(&real_root(d.path()), &e.rel).is_err());
        assert_eq!(
            fs::metadata(&secret).unwrap().accessed().unwrap(),
            before,
            "atime unchanged"
        );
    }

    #[test]
    fn fifos_are_never_opened() {
        // In the library, and outside behind a link: opening either for reading would block.
        let d = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        mkfifo(&d.path().join("pipe.sql"));
        mkfifo(&outside.path().join("out.sql"));
        mkfifo(&d.path().join("inner.fifo"));
        symlink(outside.path().join("out.sql"), d.path().join("out.sql")).unwrap();
        symlink("inner.fifo", d.path().join("in.sql")).unwrap();
        fs::write(d.path().join("ok.sql"), "x").unwrap();
        let r = scan_bounded(real_root(d.path()));
        assert_eq!(files(&r), ["in.sql", "ok.sql", "out.sql"]);
        assert_eq!(find(&r, "out.sql").link, LinkState::OutsideLink);
        // A link to a FIFO inside is not a script to open.
        assert_eq!(find(&r, "in.sql").link, LinkState::OutsideLink);
    }

    #[test]
    fn symlinked_env_folder_is_not_followed() {
        let d = tempfile::tempdir().unwrap();
        fs::create_dir(d.path().join("@production")).unwrap();
        fs::write(d.path().join("@production/purge.sql"), "DELETE FROM x;").unwrap();
        symlink("@production", d.path().join("@staging")).unwrap();
        symlink(d.path().join("@production"), d.path().join("@dev")).unwrap();
        let r = scan(&real_root(d.path()), &Limits::default(), &RealFs);
        assert_eq!(names(&r), ["@production", "@production/purge.sql"]);
        let p = find(&r, "@production/purge.sql");
        assert!(!visible(p, Some(Staging)));
    }

    #[test]
    fn symlink_loop_terminates() {
        let d = tempfile::tempdir().unwrap();
        fs::create_dir(d.path().join("d")).unwrap();
        fs::write(d.path().join("d/x.sql"), "x").unwrap();
        symlink("..", d.path().join("d/up")).unwrap();
        symlink(".", d.path().join("self")).unwrap();
        symlink(d.path(), d.path().join("d/root")).unwrap();
        symlink("/", d.path().join("fsroot")).unwrap();
        symlink("b.sql", d.path().join("a.sql")).unwrap();
        symlink("a.sql", d.path().join("b.sql")).unwrap();
        symlink("me.sql", d.path().join("me.sql")).unwrap();
        let r = scan(&real_root(d.path()), &Limits::default(), &RealFs);
        assert_eq!(names(&r), ["d", "d/x.sql", "a.sql", "b.sql", "me.sql"]);
        for n in ["a.sql", "b.sql", "me.sql"] {
            assert_eq!(find(&r, n).link, LinkState::Loop, "{n}");
        }
        assert_eq!(r.stopped, None);
    }

    #[test]
    fn a_link_out_through_dot_dot_is_outside() {
        let d = tempfile::tempdir().unwrap();
        fs::create_dir(d.path().join("lib")).unwrap();
        fs::write(d.path().join("other.sql"), "x").unwrap();
        symlink("../other.sql", d.path().join("lib/rel.sql")).unwrap();
        symlink("../lib/../other.sql", d.path().join("lib/twisty.sql")).unwrap();
        symlink("/etc/hosts", d.path().join("lib/hosts.sql")).unwrap();
        let r = scan(
            &real_root(&d.path().join("lib")),
            &Limits::default(),
            &RealFs,
        );
        for n in ["rel.sql", "twisty.sql", "hosts.sql"] {
            assert_eq!(find(&r, n).link, LinkState::OutsideLink, "{n}");
        }
    }

    #[test]
    fn an_inside_link_is_listed_with_its_target_scope() {
        let d = tempfile::tempdir().unwrap();
        fs::create_dir(d.path().join("@staging")).unwrap();
        fs::write(
            d.path().join("@staging/drop_all.sql"),
            "DROP SCHEMA public CASCADE;",
        )
        .unwrap();
        symlink("@staging/drop_all.sql", d.path().join("a.sql")).unwrap();
        fs::create_dir(d.path().join(".git")).unwrap();
        fs::write(d.path().join(".git/config"), "[core]\n").unwrap();
        symlink(".git/config", d.path().join("cfg.sql")).unwrap();
        let r = scan(&real_root(d.path()), &Limits::default(), &RealFs);
        let a = find(&r, "a.sql");
        assert_eq!(a.link, LinkState::InsideLink);
        assert_eq!(a.scope, EnvScope::Only(EnvSet::of(Staging)));
        assert!(!visible(a, Some(Production)));
        assert_eq!(find(&r, "cfg.sql").link, LinkState::OutsideLink);
    }

    #[test]
    fn an_unreadable_folder_is_skipped_with_a_diagnostic() {
        let d = tempfile::tempdir().unwrap();
        fs::create_dir(d.path().join("locked")).unwrap();
        fs::write(d.path().join("locked/a.sql"), "x").unwrap();
        fs::write(d.path().join("ok.sql"), "x").unwrap();
        let locked = d.path().join("locked");
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();
        let readable_anyway = fs::read_dir(&locked).is_ok(); // running as root
        let r = scan(&real_root(d.path()), &Limits::default(), &RealFs);
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();
        if readable_anyway {
            return;
        }
        assert_eq!(files(&r), ["ok.sql"]);
        assert!(
            r.diags.iter().any(
                |d| matches!(d, Diag::Unreadable { folder, .. } if folder.as_str() == "locked")
            ),
            "{:?}",
            r.diags
        );
    }

    #[test]
    fn control_characters_on_disk_are_skipped() {
        let d = tempfile::tempdir().unwrap();
        fs::write(d.path().join("ok.sql"), "x").unwrap();
        fs::write(d.path().join("a\nb.sql"), "x").unwrap();
        fs::write(d.path().join("t\tab.sql"), "x").unwrap();
        fs::write(d.path().join("e\u{1b}.sql"), "x").unwrap();
        let r = scan(&real_root(d.path()), &Limits::default(), &RealFs);
        assert_eq!(names(&r), ["ok.sql"]);
        assert_eq!(
            r.diags
                .iter()
                .filter(|d| matches!(d, Diag::BadName { .. }))
                .count(),
            3
        );
    }

    #[test]
    fn deep_real_tree_stops_at_the_limit() {
        let d = tempfile::tempdir().unwrap();
        let mut p = d.path().to_path_buf();
        for _ in 0..40 {
            p.push("d");
        }
        fs::create_dir_all(&p).unwrap();
        fs::write(p.join("deep.sql"), "x").unwrap();
        let mut at = d.path().to_path_buf();
        for _ in 0..31 {
            at.push("e");
        }
        fs::create_dir_all(&at).unwrap();
        fs::write(at.join("at.sql"), "x").unwrap();
        let r = scan(&real_root(d.path()), &Limits::default(), &RealFs);
        assert!(
            matches!(r.stopped, Some(super::LimitHit::Depth(32))),
            "{:?}",
            r.stopped
        );
        assert!(!files(&r).iter().any(|f| f.ends_with("deep.sql")));
        // The scan stops where it is: folders are walked in byte order, so `d…` hits the limit
        // before `e…` is reached.
        assert!(!files(&r).iter().any(|f| f.ends_with("at.sql")));
    }

    #[test]
    fn a_root_that_became_a_link_elsewhere_is_unavailable() {
        let d = tempfile::tempdir().unwrap();
        fs::create_dir(d.path().join("one")).unwrap();
        fs::create_dir(d.path().join("two")).unwrap();
        fs::write(d.path().join("one/a.sql"), "x").unwrap();
        fs::write(d.path().join("two/evil.sql"), "x").unwrap();
        symlink(d.path().join("one"), d.path().join("lib")).unwrap();
        let root = LibraryRoot {
            id: LibraryId::new(),
            path: d.path().join("lib"),
            canonical: fs::canonicalize(d.path().join("lib")).unwrap(),
        };
        assert_eq!(names(&scan(&root, &Limits::default(), &RealFs)), ["a.sql"]);
        fs::remove_file(d.path().join("lib")).unwrap();
        symlink(d.path().join("two"), d.path().join("lib")).unwrap();
        let r = scan(&root, &Limits::default(), &RealFs);
        assert_eq!(r.root, Some(RootProblem::Changed));
        assert!(r.entries.is_empty());
    }
}

#[cfg(windows)]
#[test]
fn a_junction_is_not_followed() {
    let d = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    fs::write(outside.path().join("a.sql"), "x").unwrap();
    fs::write(d.path().join("ok.sql"), "x").unwrap();
    let ok = std::process::Command::new("cmd")
        .arg("/C")
        .arg("mklink")
        .arg("/J")
        .arg(d.path().join("j"))
        .arg(outside.path())
        .status()
        .unwrap()
        .success();
    assert!(ok, "mklink /J");
    let r = scan(&real_root(d.path()), &Limits::default(), &RealFs);
    assert_eq!(names(&r), ["ok.sql"]);
}

// ---------------------------------------------------------------------------------------------
// Timing (HACIVAT_PERF=1; never a wall-clock assert otherwise).

fn real_tree(n: usize) -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    for i in 0..n {
        let dir = d.path().join(format!("d{:03}", i / 100));
        if i % 100 == 0 {
            fs::create_dir(&dir).unwrap();
        }
        fs::write(dir.join(format!("f{i:05}.sql")), "SELECT 1;").unwrap();
    }
    d
}

fn timed_scan(n: usize) -> Duration {
    let d = real_tree(n);
    let root = real_root(d.path());
    let start = std::time::Instant::now();
    let r = scan(&root, &Limits::default(), &RealFs);
    let took = start.elapsed();
    assert_eq!(files(&r).len(), n);
    println!("HV-FACT t04 scan_files={n} took_ms={}", took.as_millis());
    took
}

#[test]
#[ignore = "perf"]
fn scan_of_10k_files_under_500ms() {
    if std::env::var_os("HACIVAT_PERF").is_none() {
        return;
    }
    assert!(timed_scan(10_000) < Duration::from_millis(500));
}

#[test]
#[ignore = "perf"]
fn scan_20k_files_under_300ms() {
    if std::env::var_os("HACIVAT_PERF").is_none() {
        return;
    }
    assert!(timed_scan(20_000) < Duration::from_millis(300));
}
