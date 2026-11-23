// ## m4b-t20-adversarial
//! Task 20: the hostile-library suite (plan Review Focus 3 and 5; Task 20 step 1). Every row
//! is a library a teammate, a sync client or an attacker could hand the user, run through the
//! library service (or the scan, for the names-only rows), and every row the earlier tasks
//! already pinned at unit level is taken end to end here: the manifest rows through the
//! service's own read, the file rows through `LibraryService::{read, save, rename, trash}`.
//!
//! The carries of the W1–W4 reviews that route here are rows too: T13 fr1 re-review n4, n5,
//! n7 and the Windows hard link of a file link; T10 fr3 m-j; T16 re-review m4.
//!
//! Watch hints come from fake event sources; every wait is on a condition, 10 s ceiling.
//! Fixtures are built in temp folders at run time (never checked-in links or FIFOs).

use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use hacivat_proto::EnvTag;
use hacivat_scripts::manifest::{self, Diag as ManifestDiag, MANIFEST_MAX_BYTES, MAX_ALIASES};
use hacivat_scripts::pathsafe::display_name;
use hacivat_scripts::scan::{self, Diag, Fs, LimitHit, Limits, Meta, MetaKind};
use hacivat_scripts::service::{FolderTrash, LibraryDiag, LibraryService, ServiceOptions};
use hacivat_scripts::testing::FakeHandle;
use hacivat_scripts::{
    FileOpError, GitSummary, LibraryEvent, LibraryId, LibraryRoot, LibraryState, RawHint, RelPath,
    UnavailableReason,
};
// The FIFO, link and hard-link rows are Unix only.
#[cfg(unix)]
use hacivat_scripts::{ReadError, SaveError, SaveText};
use hacivat_settings::LocalPaths;

const CEILING: Duration = Duration::from_secs(10);

/// An `HV-FACT` line straight to stdout (libtest captures `println!`; M4 T33c).
#[cfg(windows)]
fn fact(line: &str) {
    use std::io::Write as _;
    let mut out = std::io::stdout().lock();
    let _ = out.write_all(format!("HV-FACT {line}\n").as_bytes());
    let _ = out.flush();
}

fn rel(s: &str) -> RelPath {
    RelPath::parse(s).unwrap()
}

fn canonical(p: &Path) -> PathBuf {
    std::fs::canonicalize(p).unwrap()
}

fn write_files(dir: &Path, files: &[&str]) {
    for f in files {
        let p = dir.join(f);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, f.as_bytes()).unwrap();
    }
}

async fn wait_until(what: &str, mut f: impl FnMut() -> bool) {
    let deadline = Instant::now() + CEILING;
    while !f() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

#[cfg(unix)]
fn save_text(s: &str) -> SaveText {
    SaveText {
        text: s.into(),
        meta: Default::default(),
    }
}

/// A service over its own app folders, fed by fake event sources, trashing into a folder of
/// the test's.
struct Rig {
    svc: LibraryService,
    fake: FakeHandle,
    _app: tempfile::TempDir,
    #[cfg_attr(
        not(unix),
        expect(dead_code, reason = "read by the Unix-only trash row")
    )]
    trash: tempfile::TempDir,
}

async fn rig() -> Rig {
    rig_with_fs(Arc::new(scan::RealFs)).await
}

/// [`rig`] with the file system the service and its watchers look through.
async fn rig_with_fs(fs: Arc<dyn Fs + Send + Sync>) -> Rig {
    let app = tempfile::tempdir().unwrap();
    let trash = tempfile::tempdir().unwrap();
    let fake = FakeHandle::default();
    let svc = LibraryService::open_with(
        &LocalPaths::under(app.path()),
        &tokio::runtime::Handle::current(),
        fake.sources(),
        ServiceOptions {
            settle: Duration::from_millis(20),
            git_debounce: Duration::from_millis(40),
            recheck: Duration::from_millis(60),
            home: None,
            trasher: Arc::new(FolderTrash(trash.path().to_path_buf())),
            fs,
            ..ServiceOptions::default()
        },
    );
    wait_until("the service to load", || {
        svc.snapshot().state == LibraryState::Watching
    })
    .await;
    Rig {
        svc,
        fake,
        _app: app,
        trash,
    }
}

impl Rig {
    /// Attaches `dir` and waits for its first scan and status.
    async fn attach(&self, dir: &Path) -> LibraryId {
        let id = self.svc.attach(dir.into(), None, false).await.unwrap();
        self.up(id).await;
        id
    }

    async fn up(&self, id: LibraryId) {
        wait_until("the first scan and status", || {
            self.svc.snapshot().view(None).library(id).is_some_and(|l| {
                l.git() != GitSummary::Pending && l.library().state == LibraryState::Watching
            })
        })
        .await;
        self.svc.settled().await;
    }

    fn listed(&self, id: LibraryId, env: Option<EnvTag>) -> Vec<String> {
        self.svc
            .snapshot()
            .view(env)
            .library(id)
            .map(|l| {
                l.entries()
                    .iter()
                    .map(|e| e.rel.as_str().to_owned())
                    .collect()
            })
            .unwrap_or_default()
    }

    fn diags(&self, id: LibraryId, env: Option<EnvTag>) -> Vec<LibraryDiag> {
        self.svc
            .snapshot()
            .view(env)
            .library(id)
            .map(|l| l.diagnostics())
            .unwrap_or_default()
    }

    fn db(&self, id: LibraryId) -> String {
        self.svc
            .snapshot()
            .library(id)
            .map(|l| l.db.as_str().to_owned())
            .unwrap_or_default()
    }

    fn state(&self, id: LibraryId) -> Option<LibraryState> {
        self.svc.snapshot().library(id).map(|l| l.state.clone())
    }

    fn hint(&self, root: &Path, batch: Vec<RawHint>) {
        assert!(self.fake.emit(root, batch), "nothing watches {root:?}");
    }
}

/// A library folder named `name` under a fresh temp folder, with `.hacivat/workspace.toml`
/// holding `manifest` when given.
fn library_with_manifest(name: &str, manifest: Option<&[u8]>) -> (tempfile::TempDir, PathBuf) {
    let d = tempfile::tempdir().unwrap();
    let lib = d.path().join(name);
    write_files(&lib, &["a.sql"]);
    if let Some(bytes) = manifest {
        std::fs::create_dir_all(lib.join(".hacivat")).unwrap();
        std::fs::write(lib.join(".hacivat/workspace.toml"), bytes).unwrap();
    }
    (d, lib)
}

fn manifest_diags(diags: &[LibraryDiag]) -> Vec<ManifestDiag> {
    diags
        .iter()
        .filter_map(|d| match d {
            LibraryDiag::Manifest(m) => Some(m.clone()),
            _ => None,
        })
        .collect()
}

// ---------------------------------------------------------------------------------------------
// The manifest (HV-SCRIPT-01): through the service's own read
// ---------------------------------------------------------------------------------------------

/// 64 KiB of nested tables: never parsed (the parser recurses), never a panic or a blown
/// stack, nothing binds; the library binds by its folder name and says why.
#[tokio::test(flavor = "multi_thread")]
async fn a_64_kib_manifest_of_nested_tables_binds_the_folder_name() {
    let mut text = String::from("format = 1\ndatabase = \"evil\"\n");
    let mut depth = 0;
    while text.len() + 40 < MANIFEST_MAX_BYTES {
        text.push_str(&format!("[t{depth}]\n"));
        depth += 1;
    }
    let mut nested = String::from("format = 1\ndatabase = \"evil\"\nx = ");
    while nested.len() + 2 < MANIFEST_MAX_BYTES {
        nested.push('[');
    }
    for (what, bytes) in [("tables", text), ("arrays", nested)] {
        assert!(bytes.len() <= MANIFEST_MAX_BYTES, "{what}");
        let (_d, lib) = library_with_manifest("orders", Some(bytes.as_bytes()));
        let r = rig().await;
        let id = r.attach(&lib).await;
        let diags = manifest_diags(&r.diags(id, None));
        if what == "arrays" {
            assert_eq!(diags, [ManifestDiag::TooDeep], "{what}");
            assert_eq!(r.db(id), "orders", "{what}");
        } else {
            // Thousands of empty tables are flat: read, every table an unknown key, and the
            // diagnostics bounded (a few named, the rest counted).
            assert!(diags.len() <= 64, "{what}: {} diagnostics", diags.len());
            assert!(
                matches!(diags.last(), Some(ManifestDiag::MoreUnknownKeys(n)) if *n > 1_000),
                "{what}"
            );
            assert!(
                diags.iter().all(|d| matches!(
                    d,
                    ManifestDiag::UnknownKey(_) | ManifestDiag::MoreUnknownKeys(_)
                )),
                "{what}: {diags:?}"
            );
            assert_eq!(r.db(id), "evil", "{what}");
        }
    }
}

/// 10,000 aliases: past the size cap nothing is parsed; under it, the first `MAX_ALIASES` are
/// kept and the rest said, never bound.
#[test]
fn ten_thousand_aliases_never_bind_past_the_limits() {
    let list = |n: usize, width: usize| {
        let names: Vec<String> = (0..n).map(|i| format!("\"a{i:0width$}\"")).collect();
        format!(
            "format = 1\ndatabase = \"orders\"\naliases = [{}]\n",
            names.join(",")
        )
    };
    let over = list(10_000, 5);
    assert!(over.len() > MANIFEST_MAX_BYTES);
    let r = manifest::read_bytes(over.as_bytes(), "lib");
    assert!(r.db.is_none() && r.aliases.is_empty());
    assert_eq!(r.diags, [ManifestDiag::TooLarge]);
    let under = list(6_000, 4);
    assert!(under.len() <= MANIFEST_MAX_BYTES);
    let r = manifest::read_bytes(under.as_bytes(), "lib");
    assert_eq!(r.db.as_ref().map(|d| d.as_str()), Some("orders"));
    assert_eq!(r.aliases.len(), MAX_ALIASES);
    assert_eq!(r.aliases[0].as_str(), "a0000");
    assert!(r.diags.contains(&ManifestDiag::TooManyAliases));
}

/// A NUL inside `database` (escaped, as TOML allows) is a control character: the name never
/// binds, and no NUL reaches the snapshot.
#[tokio::test(flavor = "multi_thread")]
async fn a_nul_inside_database_never_binds() {
    let (_d, lib) = library_with_manifest(
        "orders",
        Some(b"format = 1\ndatabase = \"pay\\u0000roll\"\naliases = [\"x\\u0000y\", \"ok\"]\n"),
    );
    let r = rig().await;
    let id = r.attach(&lib).await;
    assert_eq!(r.db(id), "orders");
    let snap = r.svc.snapshot();
    let l = snap.library(id).unwrap();
    assert!(!l.db.as_str().contains('\0'));
    assert!(l.aliases.iter().all(|a| !a.as_str().contains('\0')));
    assert_eq!(
        l.aliases.iter().map(|a| a.as_str()).collect::<Vec<_>>(),
        ["ok"]
    );
}

/// A manifest with a UTF-8 BOM (a Windows editor's) binds through the service exactly as
/// without one.
#[tokio::test(flavor = "multi_thread")]
async fn a_bom_prefixed_manifest_binds_through_the_service() {
    let (_d, lib) = library_with_manifest(
        "folder",
        Some(b"\xEF\xBB\xBFformat = 1\r\ndatabase = \"orders\"\r\n"),
    );
    let r = rig().await;
    let id = r.attach(&lib).await;
    assert_eq!(r.db(id), "orders");
    assert_eq!(manifest_diags(&r.diags(id, None)), []);
}

/// Makes a FIFO at `p` (Unix).
#[cfg(unix)]
fn mkfifo(p: &Path) {
    let made = std::process::Command::new("mkfifo")
        .arg(p)
        .status()
        .unwrap();
    assert!(made.success(), "mkfifo {p:?}");
}

/// A manifest that is a FIFO: the service never opens it (an open would block until a writer
/// came), the attach completes, and the library binds by its folder name.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn a_manifest_that_is_a_fifo_never_blocks_the_service() {
    let (_d, lib) = library_with_manifest("orders", None);
    std::fs::create_dir_all(lib.join(".hacivat")).unwrap();
    mkfifo(&lib.join(".hacivat/workspace.toml"));
    let r = rig().await;
    let id = tokio::time::timeout(CEILING, r.attach(&lib))
        .await
        .expect("the attach never blocks on a FIFO");
    assert_eq!(r.db(id), "orders");
    assert_eq!(manifest_diags(&r.diags(id, None)), [ManifestDiag::NotAFile]);
    // A manifest change hint (the watcher's) reads it again, still without blocking.
    let root = canonical(&lib);
    r.hint(
        &root,
        vec![RawHint::Path(root.join(".hacivat/workspace.toml"))],
    );
    tokio::time::timeout(CEILING, r.svc.settled())
        .await
        .expect("a hint on the FIFO never blocks the service");
    assert_eq!(r.db(id), "orders");
}

// ---------------------------------------------------------------------------------------------
// Scripts that are not files (HV-SCRIPT-03, -07)
// ---------------------------------------------------------------------------------------------

/// A script that is a FIFO, and a script link to a device: the FIFO is not listed, the link is
/// listed disabled, and reading or saving either never opens it (nor blocks).
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn a_script_that_is_a_fifo_or_a_device_is_never_opened() {
    let d = tempfile::tempdir().unwrap();
    write_files(d.path(), &["a.sql"]);
    mkfifo(&d.path().join("pipe.sql"));
    std::os::unix::fs::symlink("/dev/zero", d.path().join("zero.sql")).unwrap();
    let r = rig().await;
    let id = r.attach(d.path()).await;
    assert_eq!(r.listed(id, None), ["a.sql", "zero.sql"]);
    let snap = r.svc.snapshot();
    let zero = snap
        .view(None)
        .library(id)
        .unwrap()
        .entry(&rel("zero.sql"))
        .cloned();
    assert_eq!(
        zero.map(|e| e.link),
        Some(hacivat_scripts::LinkState::OutsideLink)
    );
    for name in ["pipe.sql", "zero.sql"] {
        let read = tokio::time::timeout(CEILING, r.svc.read(id, &rel(name), None))
            .await
            .expect("a read never blocks");
        assert!(
            matches!(read, Err(ReadError::PathRefused | ReadError::Io(_))),
            "{name}: {:?}",
            read.map(|d| d.text.len())
        );
        let save = tokio::time::timeout(
            CEILING,
            r.svc
                .save(id, &rel(name), None, save_text("SELECT 1;"), None),
        )
        .await
        .expect("a save never blocks");
        assert!(save.is_err(), "{name}: {save:?}");
    }
    // Neither was turned into a file.
    let meta = std::fs::symlink_metadata(d.path().join("pipe.sql")).unwrap();
    assert!(!meta.is_file());
    assert!(
        std::fs::symlink_metadata(d.path().join("zero.sql"))
            .unwrap()
            .file_type()
            .is_symlink()
    );
}

/// A 300-deep folder chain stops the scan at the depth limit with a notice; the files above
/// it are listed, nothing hangs or overflows a stack.
#[test]
fn a_300_deep_folder_chain_stops_with_a_notice() {
    let d = tempfile::tempdir().unwrap();
    let mut p = canonical(d.path());
    let root_path = p.clone();
    std::fs::write(p.join("top.sql"), "SELECT 1;").unwrap();
    for _ in 0..300 {
        p.push("d");
    }
    std::fs::create_dir_all(&p).unwrap();
    std::fs::write(p.join("deep.sql"), "SELECT 2;").unwrap();
    let root = LibraryRoot {
        id: LibraryId::new(),
        path: root_path.clone(),
        canonical: root_path,
    };
    let found = scan::scan(&root, &Limits::default(), &scan::RealFs);
    assert_eq!(
        found.stopped,
        Some(LimitHit::Depth(Limits::default().max_depth))
    );
    let files: Vec<&str> = found
        .entries
        .iter()
        .filter(|e| e.kind != hacivat_scripts::EntryKind::Folder)
        .map(|e| e.rel.as_str())
        .collect();
    assert_eq!(files, ["top.sql"]);
    assert!(
        found
            .entries
            .iter()
            .all(|e| e.rel.components().count() <= Limits::default().max_depth)
    );
}

/// A file system of one folder with `n` empty files named by `name(i)`; names and `lstat`
/// only (the scan never reads a file).
struct Wide {
    root: PathBuf,
    n: usize,
    name: fn(usize) -> String,
}

impl Fs for Wide {
    fn read_dir(&self, dir: &Path, cap: usize) -> io::Result<Vec<OsString>> {
        if dir != self.root {
            return Ok(Vec::new());
        }
        Ok((0..self.n.min(cap))
            .map(|i| (self.name)(i).into())
            .collect())
    }

    fn symlink_metadata(&self, path: &Path) -> io::Result<Meta> {
        let kind = if path == self.root {
            MetaKind::Dir
        } else if path.parent() == Some(self.root.as_path()) {
            MetaKind::File
        } else {
            return Err(io::ErrorKind::NotFound.into());
        };
        Ok(Meta {
            kind,
            len: 0,
            mtime: SystemTime::UNIX_EPOCH,
            dataless: false,
        })
    }

    fn canonicalize(&self, path: &Path) -> io::Result<PathBuf> {
        Ok(path.to_path_buf())
    }

    fn read_link(&self, _: &Path) -> io::Result<PathBuf> {
        Err(io::ErrorKind::InvalidInput.into())
    }
}

/// 200,000 empty files: the scan stops at its file limit (scripts) or its visit limit (other
/// names) with a notice, listing what it found up to there, and never reads a file.
#[test]
fn two_hundred_thousand_empty_files_stop_with_a_notice() {
    let root_path = PathBuf::from(if cfg!(windows) {
        r"C:\hv-wide\lib"
    } else {
        "/hv-wide/lib"
    });
    let root = LibraryRoot {
        id: LibraryId::new(),
        path: root_path.clone(),
        canonical: root_path.clone(),
    };
    let limits = Limits::default();
    let scripts = Wide {
        root: root_path.clone(),
        n: 200_000,
        name: |i| format!("f{i:06}.sql"),
    };
    let found = scan::scan(&root, &limits, &scripts);
    assert_eq!(found.stopped, Some(LimitHit::Files(limits.max_files)));
    assert_eq!(found.entries.len(), limits.max_files);
    let others = Wide {
        root: root_path,
        n: 200_001,
        name: |i| format!("f{i:06}.txt"),
    };
    let found = scan::scan(&root, &limits, &others);
    assert_eq!(found.stopped, Some(LimitHit::Visited(limits.max_visited)));
    assert!(found.entries.is_empty());
}

/// Every bidi control (and the invisible format characters around them) in a name: the name
/// is listed, shown escaped (`<U+XXXX>`), and a folder spelt to look like `@production` is an
/// unknown environment, hidden everywhere with a diagnostic, never a scope.
#[tokio::test(flavor = "multi_thread")]
async fn every_bidi_control_is_escaped_and_never_scopes() {
    const BIDI: [char; 12] = [
        '\u{061C}', '\u{200E}', '\u{200F}', '\u{202A}', '\u{202B}', '\u{202C}', '\u{202D}',
        '\u{202E}', '\u{2066}', '\u{2067}', '\u{2068}', '\u{2069}',
    ];
    let name: String = BIDI.iter().map(|c| format!("x{c}")).collect::<String>() + ".sql";
    let shown = display_name(&name);
    for c in BIDI {
        assert!(!shown.contains(c), "{c:?} shown raw");
        assert!(shown.contains(&format!("<U+{:04X}>", c as u32)), "{shown}");
    }
    let d = tempfile::tempdir().unwrap();
    write_files(d.path(), &[&name]);
    let spoof = "@\u{202E}noitcudorp\u{202C}".to_string();
    write_files(
        d.path(),
        &[&format!("{spoof}/p.sql"), "@production/real.sql"],
    );
    let r = rig().await;
    let id = r.attach(d.path()).await;
    assert_eq!(r.listed(id, None), [name.as_str()]);
    let prod = r.listed(id, Some(EnvTag::Production));
    assert!(
        prod.contains(&"@production/real.sql".to_owned()),
        "{prod:?}"
    );
    assert!(prod.iter().all(|p| !p.contains("p.sql")), "{prod:?}");
    assert!(
        r.diags(id, None)
            .iter()
            .any(|d| matches!(d, LibraryDiag::Scan(Diag::UnknownEnvFolder { name, .. }) if !name.contains('\u{202E}'))),
        "{:?}",
        r.diags(id, None)
    );
}

// ---------------------------------------------------------------------------------------------
// Links and hard links between a read and a save (HV-SCRIPT-03 step 8, -07)
// ---------------------------------------------------------------------------------------------

/// A file replaced by a link to a file outside between the tab's read and its save: the save
/// never writes through it, and the outside file keeps its bytes.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn a_file_replaced_by_a_link_between_read_and_save_is_never_written_through() {
    let outside = tempfile::tempdir().unwrap();
    let secret = outside.path().join("id_ed25519.sql");
    std::fs::write(&secret, "not yours").unwrap();
    let d = tempfile::tempdir().unwrap();
    write_files(d.path(), &["a.sql"]);
    let r = rig().await;
    let id = r.attach(d.path()).await;
    let read = r.svc.read(id, &rel("a.sql"), None).await.unwrap();
    std::fs::remove_file(d.path().join("a.sql")).unwrap();
    std::os::unix::fs::symlink(&secret, d.path().join("a.sql")).unwrap();
    for expect in [Some(read.stamp), None] {
        let out = r
            .svc
            .save(id, &rel("a.sql"), None, save_text("DROP TABLE x;"), expect)
            .await;
        assert!(
            matches!(
                out,
                Err(SaveError::PathChanged | SaveError::PathRefused | SaveError::ChangedOnDisk(_))
            ),
            "{expect:?}: {out:?}"
        );
    }
    assert_eq!(std::fs::read_to_string(&secret).unwrap(), "not yours");
    assert!(
        std::fs::symlink_metadata(d.path().join("a.sql"))
            .unwrap()
            .file_type()
            .is_symlink()
    );
}

/// The documented residual (ADR 0009 `m4b-t20`): a hard link to a file outside the library is
/// the same inode under a name inside it, indistinguishable by any `lstat`. It is listed, read
/// and saved in place like any file the user put there; the save keeps the link (both names
/// show the new bytes), and nothing else outside is touched.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn a_hard_link_to_a_file_outside_is_the_users_own_inode() {
    use std::os::unix::fs::MetadataExt;
    let outside = tempfile::tempdir().unwrap();
    let theirs = outside.path().join("theirs.sql");
    std::fs::write(&theirs, "SELECT 1;").unwrap();
    let d = tempfile::tempdir().unwrap();
    std::fs::hard_link(&theirs, d.path().join("h.sql")).unwrap();
    let r = rig().await;
    let id = r.attach(d.path()).await;
    assert_eq!(r.listed(id, None), ["h.sql"]);
    let read = r.svc.read(id, &rel("h.sql"), None).await.unwrap();
    assert_eq!(read.text, "SELECT 1;");
    r.svc
        .save(
            id,
            &rel("h.sql"),
            None,
            save_text("SELECT 2;"),
            Some(read.stamp),
        )
        .await
        .unwrap();
    let inside = std::fs::metadata(d.path().join("h.sql")).unwrap();
    let out = std::fs::metadata(&theirs).unwrap();
    assert_eq!((inside.dev(), inside.ino()), (out.dev(), out.ino()));
    assert_eq!(inside.nlink(), 2);
    assert_eq!(std::fs::read_to_string(&theirs).unwrap(), "SELECT 2;");
    let names: Vec<_> = std::fs::read_dir(outside.path())
        .unwrap()
        .flatten()
        .collect();
    assert_eq!(names.len(), 1, "nothing else was written outside");
}

// ---------------------------------------------------------------------------------------------
// Carries (T13 fr1 re-review, T10 fr3, T16 re-review)
// ---------------------------------------------------------------------------------------------

/// T13 n7: a refused name at the library root (a control character) no longer refuses every
/// folder operation as "too large to check"; one inside the folder still does.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn a_bad_name_at_the_root_never_blocks_folder_operations() {
    let d = tempfile::tempdir().unwrap();
    write_files(
        d.path(),
        &["reports/a.sql", "other/b.sql", "bad\u{1}name.sql"],
    );
    let r = rig().await;
    let id = r.attach(d.path()).await;
    assert!(
        r.diags(id, None)
            .iter()
            .any(|d| matches!(d, LibraryDiag::Scan(Diag::BadName { parent: None, .. }))),
        "{:?}",
        r.diags(id, None)
    );
    assert_eq!(
        r.svc
            .rename(id, &rel("reports"), &rel("raporlar"), None)
            .await,
        Ok(())
    );
    assert_eq!(r.svc.trash(id, &rel("other"), None).await, Ok(()));
    assert!(d.path().join("raporlar/a.sql").is_file());
    assert!(r.trash.path().join("other/b.sql").is_file());
    // A refused name inside a folder still keeps that folder's operations closed.
    write_files(d.path(), &["raporlar/worse\u{1}.sql"]);
    r.svc.refresh(id).await;
    r.svc.settled().await;
    assert_eq!(
        r.svc.trash(id, &rel("raporlar"), None).await,
        Err(FileOpError::NotFullyListed)
    );
    assert!(d.path().join("raporlar/a.sql").is_file());
}

/// T13 n5: a script stranded under a case-only rename's temp name (a crash between the two
/// steps) is reported where its folder is shown, never listed, never deleted, and its folder
/// is never trashed or renamed with it unseen.
#[tokio::test(flavor = "multi_thread")]
async fn a_stranded_rename_temp_is_shown_and_keeps_its_folder() {
    let temp = format!(".hv-rename-{}", "0123456789abcdef".repeat(2));
    let d = tempfile::tempdir().unwrap();
    write_files(
        d.path(),
        &[
            "reports/a.sql",
            &format!("reports/{temp}"),
            &format!("@production/{temp}"),
        ],
    );
    let r = rig().await;
    let id = r.attach(d.path()).await;
    assert_eq!(r.listed(id, None), ["reports", "reports/a.sql"]);
    let stranded = |env| {
        r.diags(id, env)
            .into_iter()
            .filter_map(|d| match d {
                LibraryDiag::Scan(Diag::InterruptedRename { folder, name }) => {
                    Some((folder.map(|f| f.as_str().to_owned()), name))
                }
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(stranded(None), [(Some("reports".to_owned()), temp.clone())]);
    // The production one is counted, never named, under another environment.
    assert!(
        r.diags(id, Some(EnvTag::Staging))
            .iter()
            .any(|d| matches!(d, LibraryDiag::HiddenProblems(n) if *n >= 1))
    );
    assert_eq!(stranded(Some(EnvTag::Production)).len(), 2);
    for out in [
        r.svc.trash(id, &rel("reports"), None).await,
        r.svc
            .rename(id, &rel("reports"), &rel("raporlar"), None)
            .await,
    ] {
        assert_eq!(out, Err(FileOpError::NotFullyListed));
    }
    assert!(d.path().join("reports").join(&temp).is_file());
    assert!(d.path().join("@production").join(&temp).is_file());
}

/// T10 m-j: an open file moved into another folder while a checkout adds 40 same-length files
/// beside it: the service's own pairing (a rescan, no per-file hints) still finds it, since a
/// destination that kept the file's name is read first; the read cap is not lifted.
#[tokio::test(flavor = "multi_thread")]
async fn a_moved_open_file_is_paired_past_forty_same_length_arrivals() {
    let body = "SELECT 'the moved script';";
    let d = tempfile::tempdir().unwrap();
    std::fs::write(d.path().join("a.sql"), body).unwrap();
    let r = rig().await;
    let id = r.attach(d.path()).await;
    let root = canonical(d.path());
    r.svc.read(id, &rel("a.sql"), None).await.unwrap();
    let mut rx = r.svc.subscribe(None);
    std::fs::create_dir_all(d.path().join("impostors")).unwrap();
    for i in 0..40u8 {
        let mut other = body.as_bytes().to_vec();
        other[8] = b'a' + i % 26;
        other[9] = b'a' + i / 26;
        std::fs::write(d.path().join(format!("impostors/f{i:02}.sql")), other).unwrap();
    }
    std::fs::create_dir_all(d.path().join("moved")).unwrap();
    std::fs::rename(d.path().join("a.sql"), d.path().join("moved/a.sql")).unwrap();
    r.hint(&root, vec![RawHint::Rescan(root.clone())]);
    let deadline = tokio::time::Instant::now() + CEILING;
    let mut seen = Vec::new();
    let found = loop {
        match tokio::time::timeout_at(deadline, rx.recv()).await {
            Ok(Ok(e)) => {
                seen.push(e.clone());
                let about_a = match &e {
                    LibraryEvent::FileRenamed { from, .. }
                    | LibraryEvent::FileDeleted { rel: from, .. } => from.as_str() == "a.sql",
                    _ => false,
                };
                if about_a {
                    break e;
                }
            }
            Ok(Err(_)) => {}
            Err(_) => panic!("no answer for a.sql; seen {seen:?}"),
        }
    };
    assert_eq!(
        found,
        LibraryEvent::FileRenamed {
            id,
            from: rel("a.sql"),
            to: rel("moved/a.sql"),
        },
        "{seen:?}"
    );
}

/// T16 m4: "Use this folder" adopts only the folder its confirm was shown for. Another folder
/// swapped in while the user thought it over is refused, however valid; the one then found is
/// adopted when confirmed for it.
#[tokio::test(flavor = "multi_thread")]
async fn use_this_folder_adopts_only_the_folder_it_was_shown() {
    let d = tempfile::tempdir().unwrap();
    let lib = d.path().join("scripts");
    write_files(&lib, &["a.sql"]);
    let r = rig().await;
    let id = r.attach(&lib).await;
    let root = canonical(&lib);
    let refused_identity = |r: &Rig| {
        r.svc
            .snapshot()
            .library(id)
            .and_then(|l| l.refused_identity)
    };
    assert_eq!(refused_identity(&r), None);
    // A restore puts another folder at the path: refused, and the snapshot names that folder.
    std::fs::rename(&lib, d.path().join("scripts-old")).unwrap();
    write_files(&lib, &["restored.sql"]);
    r.hint(&root, vec![RawHint::Path(root.clone())]);
    wait_until("RootChanged", || {
        r.state(id) == Some(LibraryState::Unavailable(UnavailableReason::RootChanged))
            && refused_identity(&r).is_some()
    })
    .await;
    let shown = refused_identity(&r);
    assert!(r.svc.snapshot().library(id).unwrap().can_adopt());
    // Before the press, a third folder takes the path.
    std::fs::rename(&lib, d.path().join("scripts-restored")).unwrap();
    write_files(&lib, &["swapped.sql"]);
    assert_eq!(r.svc.adopt_root(id, shown).await, Ok(false));
    assert_eq!(
        r.state(id),
        Some(LibraryState::Unavailable(UnavailableReason::RootChanged))
    );
    // The next validation (a refresh; the refused library has no watcher) names the folder now
    // there; confirmed for it, it is adopted.
    r.svc.refresh(id).await;
    wait_until("the swapped folder is named", || {
        refused_identity(&r).is_some_and(|now| Some(now) != shown)
    })
    .await;
    let now = refused_identity(&r);
    assert_eq!(r.svc.adopt_root(id, now).await, Ok(true));
    wait_until("served again", || {
        r.state(id) == Some(LibraryState::Watching)
    })
    .await;
    r.svc.settled().await;
    assert_eq!(r.listed(id, None), ["swapped.sql"]);
}

/// T13 fr1 re-review: on Windows `CreateHardLinkW` on a symbolic link links its target, so a
/// file link is never renamed through a hard link there (refused like a volume without hard
/// links); the link and its target are untouched. Skipped where symbolic links need a
/// privilege the runner lacks.
#[cfg(windows)]
#[tokio::test(flavor = "multi_thread")]
async fn a_file_link_is_never_renamed_through_a_hard_link_on_windows() {
    let d = tempfile::tempdir().unwrap();
    write_files(d.path(), &["target.sql"]);
    if std::os::windows::fs::symlink_file(d.path().join("target.sql"), d.path().join("l.sql"))
        .is_err()
    {
        fact("t20 windows_file_link_rename=skipped_no_symlink_privilege");
        return;
    }
    let r = rig().await;
    let id = r.attach(d.path()).await;
    assert_eq!(
        r.svc.rename(id, &rel("l.sql"), &rel("m.sql"), None).await,
        Err(FileOpError::NoSafeRename)
    );
    assert!(
        std::fs::symlink_metadata(d.path().join("l.sql"))
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert!(!d.path().join("m.sql").exists());
    fact("t20 windows_file_link_rename=refused");
}

// ## m4b-t20-adversarial fix round 1 (review m3): N3 end to end through the service.

/// A file system that counts the `lstat`s of one path.
struct LockLooks {
    lock: PathBuf,
    looks: std::sync::atomic::AtomicUsize,
}

impl LockLooks {
    fn count(&self) -> usize {
        self.looks.load(std::sync::atomic::Ordering::SeqCst)
    }
}

impl Fs for LockLooks {
    fn read_dir(&self, dir: &Path, cap: usize) -> io::Result<Vec<OsString>> {
        scan::RealFs.read_dir(dir, cap)
    }
    fn symlink_metadata(&self, path: &Path) -> io::Result<Meta> {
        if path == self.lock {
            self.looks.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
        scan::RealFs.symlink_metadata(path)
    }
    fn canonicalize(&self, path: &Path) -> io::Result<PathBuf> {
        scan::RealFs.canonicalize(path)
    }
    fn read_link(&self, link: &Path) -> io::Result<PathBuf> {
        scan::RealFs.read_link(link)
    }
}

/// The git CLI, on this test's own fixture repository only (`NoGitSubprocess` exempts
/// `tests/`), with no global or system config.
fn git_cli(dir: &Path, args: &[&str]) {
    let mut c = std::process::Command::new("git");
    for var in [
        "GIT_DIR",
        "GIT_WORK_TREE",
        "GIT_INDEX_FILE",
        "GIT_CONFIG_PARAMETERS",
    ] {
        c.env_remove(var);
    }
    let out = c
        .args(args)
        .current_dir(dir)
        .env("GIT_CONFIG_GLOBAL", dir.join("hv-t20-no-global"))
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_AUTHOR_NAME", "Fixture")
        .env("GIT_AUTHOR_EMAIL", "fixture@example.invalid")
        .env("GIT_COMMITTER_NAME", "Fixture")
        .env("GIT_COMMITTER_EMAIL", "fixture@example.invalid")
        .output()
        .unwrap();
    assert!(out.status.success(), "git {args:?}");
}

/// A library inside a bigger repository: its status hands the watcher the outer
/// `index.lock`, which a counted hint then makes it look at; a watcher started again (a watch
/// mode switch) has it too; and a status that is no longer `Ready` takes it back, so it is not
/// looked at any more.
#[tokio::test(flavor = "multi_thread")]
async fn git_the_outer_index_lock_reaches_every_watcher_and_leaves_with_the_status() {
    let top = tempfile::tempdir().unwrap();
    let repo = canonical(top.path()).join("repo");
    write_files(&repo, &["lib/a.sql"]);
    git_cli(&repo, &["init", "-q", "-b", "main"]);
    git_cli(&repo, &["add", "-A"]);
    git_cli(&repo, &["commit", "-q", "--no-verify", "-m", "base"]);
    let lib = repo.join("lib");
    let fs = Arc::new(LockLooks {
        lock: repo.join(".git/index.lock"),
        looks: std::sync::atomic::AtomicUsize::new(0),
    });
    let r = rig_with_fs(fs.clone()).await;
    let id = r.attach(&lib).await;
    assert_eq!(
        r.svc.snapshot().view(None).library(id).map(|l| l.git()),
        Some(GitSummary::Ready)
    );
    let root = canonical(&lib);
    let mut rx = r.svc.subscribe(None);
    // One edit, its event awaited: the edit's hint made the watcher look at the outer lock.
    let edit = |n: u32| {
        std::fs::write(root.join("a.sql"), format!("SELECT {n};")).unwrap();
        r.hint(&root, vec![RawHint::Path(root.join("a.sql"))]);
    };
    let changed = |e: &LibraryEvent| matches!(e, LibraryEvent::FileChanged { rel, .. } if rel.as_str() == "a.sql");
    async fn next(rx: &mut hacivat_scripts::Subscription, f: impl Fn(&LibraryEvent) -> bool) {
        let deadline = tokio::time::Instant::now() + CEILING;
        loop {
            match tokio::time::timeout_at(deadline, rx.recv()).await {
                Ok(Ok(e)) if f(&e) => return,
                Ok(_) => {}
                Err(_) => panic!("no such event"),
            }
        }
    }
    // A tab has the file open (its events are the open files').
    r.svc.read(id, &rel("a.sql"), None).await.unwrap();
    edit(1);
    next(&mut rx, changed).await;
    wait_until("the status's lock was looked at", || fs.count() >= 1).await;
    // A watcher started again (watch mode switch) has the lock too. (The restart's rescan also
    // refreshes the status, so this does not isolate the hand-off at watcher start.)
    r.svc
        .set_watch(id, hacivat_scripts::WatchMode::Poll)
        .await
        .unwrap();
    wait_until("the new watcher watches", || r.fake.emit(&root, Vec::new())).await;
    r.svc.settled().await;
    let before = fs.count();
    r.svc.read(id, &rel("a.sql"), None).await.unwrap();
    edit(2);
    next(&mut rx, changed).await;
    assert!(fs.count() > before, "the new watcher looks at the lock");
    // A status that is not `Ready` (an unreadable repository config) takes it back.
    std::fs::write(repo.join(".git/config"), b"[core\n\0garbage").unwrap();
    r.svc.refresh(id).await;
    wait_until("the status is no longer ready", || {
        r.svc
            .snapshot()
            .view(None)
            .library(id)
            .is_some_and(|l| l.git() != GitSummary::Ready && l.git() != GitSummary::Pending)
    })
    .await;
    r.svc.settled().await;
    let after = fs.count();
    r.svc.read(id, &rel("a.sql"), None).await.unwrap();
    edit(3);
    next(&mut rx, changed).await;
    r.svc.settled().await;
    assert_eq!(
        fs.count(),
        after,
        "a path no status validates is not looked at"
    );
}
// ## m4b-t20-adversarial
