// ## m4b-t13-explorer
//! Task 13: the Scripts tree's file operations through the library service (R-M4b-18, Q11):
//! New script, New folder, Rename and Move to Trash stay inside the library, never replace a
//! name, follow the connection's environment scope (R-T5-6, R-M4b-6), and Move to Trash goes
//! through the service's `Trasher` (a folder of the test's here, so nothing reaches the user's
//! trash). Hints come from a fake event source; every wait is on a condition (ceiling 10 s).

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use hacivat_proto::EnvTag;
use hacivat_scripts::pathsafe::NameProblem;
use hacivat_scripts::scan::Limits;
use hacivat_scripts::service::{FolderTrash, LibraryService, ServiceOptions};
use hacivat_scripts::testing::FakeHandle;
use hacivat_scripts::{EntryKind, FileOpError, GitSummary, LibraryId, LibraryState, RelPath};
use hacivat_settings::LocalPaths;

const CEILING: Duration = Duration::from_secs(10);

fn rel(s: &str) -> RelPath {
    RelPath::parse(s).unwrap()
}

fn write_files(dir: &Path, files: &[&str]) {
    for f in files {
        let p = dir.join(f);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, f.as_bytes()).unwrap();
    }
}

/// Every path under `dir`, relative, `/`-separated, folders with a trailing `/`, sorted.
fn on_disk(dir: &Path) -> Vec<String> {
    let mut out = Vec::new();
    let mut todo = vec![dir.to_path_buf()];
    while let Some(d) = todo.pop() {
        for e in std::fs::read_dir(&d).unwrap().flatten() {
            let p = e.path();
            let name = p
                .strip_prefix(dir)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            if std::fs::symlink_metadata(&p).unwrap().is_dir() {
                out.push(format!("{name}/"));
                todo.push(p);
            } else {
                out.push(name);
            }
        }
    }
    out.sort();
    out
}

async fn wait_until(what: &str, mut f: impl FnMut() -> bool) {
    let deadline = Instant::now() + CEILING;
    while !f() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

struct Rig {
    svc: LibraryService,
    _fake: FakeHandle,
    _app: tempfile::TempDir,
    lib: tempfile::TempDir,
    trash: tempfile::TempDir,
    id: LibraryId,
}

impl Rig {
    /// The listing a connection whose environment is `env` sees.
    fn listed(&self, env: Option<EnvTag>) -> Vec<String> {
        self.svc
            .snapshot()
            .view(env)
            .library(self.id)
            .map(|l| {
                l.entries()
                    .iter()
                    .map(|e| e.rel.as_str().to_owned())
                    .collect()
            })
            .unwrap_or_default()
    }

    fn lib(&self) -> PathBuf {
        std::fs::canonicalize(self.lib.path()).unwrap()
    }
}

/// A service with one library of `files` attached and scanned, and a test trash folder.
async fn rig(files: &[&str]) -> Rig {
    rig_with(files, Limits::default()).await
}

/// [`rig`] with the scan's `limits`.
async fn rig_with(files: &[&str], limits: Limits) -> Rig {
    let app = tempfile::tempdir().unwrap();
    let lib = tempfile::tempdir().unwrap();
    let trash = tempfile::tempdir().unwrap();
    write_files(lib.path(), files);
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
            limits,
            ..ServiceOptions::default()
        },
    );
    wait_until("the service to load", || {
        svc.snapshot().state == LibraryState::Watching
    })
    .await;
    let id = svc.attach(lib.path().into(), None, false).await.unwrap();
    wait_until("the first scan and status", || {
        svc.snapshot().view(None).library(id).is_some_and(|l| {
            l.git() != GitSummary::Pending && l.library().state == LibraryState::Watching
        })
    })
    .await;
    svc.settled().await;
    Rig {
        svc,
        _fake: fake,
        _app: app,
        lib,
        trash,
        id,
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn new_script_and_folder_are_created_and_listed_without_a_watcher_report() {
    let r = rig(&["a.sql"]).await;
    r.svc
        .create(r.id, &rel("b.sql"), EntryKind::Sql, None)
        .await
        .unwrap();
    r.svc
        .create(r.id, &rel("raporlar"), EntryKind::Folder, None)
        .await
        .unwrap();
    r.svc
        .create(r.id, &rel("raporlar/haftalık.sql"), EntryKind::Sql, None)
        .await
        .unwrap();
    assert_eq!(
        on_disk(&r.lib()),
        ["a.sql", "b.sql", "raporlar/", "raporlar/haftalık.sql"]
    );
    assert_eq!(
        std::fs::read(r.lib().join("b.sql")).unwrap(),
        b"",
        "a new script is empty"
    );
    // The service rescans after the operation: no hint from a watcher is needed.
    wait_until("the new files to be listed", || {
        r.listed(None) == ["raporlar", "raporlar/haftalık.sql", "a.sql", "b.sql"]
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn create_never_replaces_and_refuses_bad_names() {
    let r = rig(&["a.sql"]).await;
    let before = on_disk(&r.lib());
    let cases: [(&str, EntryKind, FileOpError); 7] = [
        ("a.sql", EntryKind::Sql, FileOpError::Exists),
        ("a.sql", EntryKind::Folder, FileOpError::Exists),
        ("b.txt", EntryKind::Sql, FileOpError::NotAScript),
        (
            ".hidden.sql",
            EntryKind::Sql,
            FileOpError::BadName(NameProblem::Skipped),
        ),
        (
            "CON.sql",
            EntryKind::Sql,
            FileOpError::BadName(NameProblem::Reserved),
        ),
        (
            "a b .sql ",
            EntryKind::Sql,
            FileOpError::BadName(NameProblem::TrailingDotOrSpace),
        ),
        ("missing/x.sql", EntryKind::Sql, FileOpError::NotFound),
    ];
    for (name, kind, want) in cases {
        assert_eq!(
            r.svc.create(r.id, &rel(name), kind, None).await,
            Err(want),
            "{name}"
        );
    }
    // A traversal never parses as a library path at all.
    for bad in ["../x.sql", "a/../../x.sql", "/etc/x.sql", "a\\..\\x.sql"] {
        assert!(RelPath::parse(bad).is_err(), "{bad}");
    }
    assert_eq!(on_disk(&r.lib()), before);
}

#[tokio::test(flavor = "multi_thread")]
async fn rename_moves_the_name_only_and_never_replaces() {
    let r = rig(&["a.sql", "b.sql", "notes/x.md"]).await;
    r.svc
        .rename(r.id, &rel("a.sql"), &rel("c.sql"), None)
        .await
        .unwrap();
    assert_eq!(
        on_disk(&r.lib()),
        ["b.sql", "c.sql", "notes/", "notes/x.md"]
    );
    assert_eq!(std::fs::read(r.lib().join("c.sql")).unwrap(), b"a.sql");
    wait_until("the rename to be listed", || {
        r.listed(None) == ["notes", "notes/x.md", "b.sql", "c.sql"]
    })
    .await;
    // A folder.
    r.svc
        .rename(r.id, &rel("notes"), &rel("notlar"), None)
        .await
        .unwrap();
    assert_eq!(
        on_disk(&r.lib()),
        ["b.sql", "c.sql", "notlar/", "notlar/x.md"]
    );
    wait_until("the folder rename to be listed", || {
        r.listed(None) == ["notlar", "notlar/x.md", "b.sql", "c.sql"]
    })
    .await;
    let before = on_disk(&r.lib());
    let refusals: [(&str, &str, FileOpError); 5] = [
        ("b.sql", "c.sql", FileOpError::Exists),
        ("b.sql", "notlar/b.sql", FileOpError::OtherFolder),
        ("b.sql", "b.txt", FileOpError::NotAScript),
        (
            "b.sql",
            "PRN.sql",
            FileOpError::BadName(NameProblem::Reserved),
        ),
        ("gone.sql", "x.sql", FileOpError::NotFound),
    ];
    for (from, to, want) in refusals {
        assert_eq!(
            r.svc.rename(r.id, &rel(from), &rel(to), None).await,
            Err(want),
            "{from} -> {to}"
        );
    }
    assert_eq!(on_disk(&r.lib()), before);
}

#[tokio::test(flavor = "multi_thread")]
async fn trash_moves_through_the_trasher_and_says_nothing_else() {
    let r = rig(&["a.sql", "b.sql"]).await;
    r.svc.trash(r.id, &rel("a.sql"), None).await.unwrap();
    assert_eq!(on_disk(&r.lib()), ["b.sql"]);
    assert_eq!(on_disk(r.trash.path()), ["a.sql"], "it went to the trash");
    wait_until("the trashed file to leave the listing", || {
        r.listed(None) == ["b.sql"]
    })
    .await;
    assert_eq!(
        r.svc.trash(r.id, &rel("a.sql"), None).await,
        Err(FileOpError::NotFound)
    );
}

/// R-T5-6 / R-M4b-6: a staging connection can neither see nor touch `@production`.
#[tokio::test(flavor = "multi_thread")]
async fn file_operations_follow_the_connections_environment() {
    let r = rig(&[
        "a.sql",
        "@production/drop_old.sql",
        "@staging/s.sql",
        "reports/@production/p.sql",
        "reports/r.sql",
    ])
    .await;
    let staging = Some(EnvTag::Staging);
    let before = on_disk(&r.lib());
    let refused = [
        r.svc
            .create(r.id, &rel("@production/x.sql"), EntryKind::Sql, staging)
            .await,
        r.svc
            .create(r.id, &rel("@production"), EntryKind::Folder, staging)
            .await,
        r.svc
            .rename(
                r.id,
                &rel("@production/drop_old.sql"),
                &rel("@production/y.sql"),
                staging,
            )
            .await,
        r.svc
            .rename(r.id, &rel("@staging"), &rel("@production"), staging)
            .await,
        r.svc
            .trash(r.id, &rel("@production/drop_old.sql"), staging)
            .await,
        // An untagged connection sees no `@` folder.
        r.svc.trash(r.id, &rel("@staging/s.sql"), None).await,
    ];
    for (i, got) in refused.into_iter().enumerate() {
        assert_eq!(got, Err(FileOpError::NotListed), "case {i}");
    }
    // A folder that holds a file this connection does not show is neither renamed nor
    // trashed: that would move or delete it unseen.
    assert_eq!(
        r.svc.trash(r.id, &rel("reports"), staging).await,
        Err(FileOpError::HoldsHidden)
    );
    assert_eq!(
        r.svc
            .rename(r.id, &rel("reports"), &rel("raporlar"), staging)
            .await,
        Err(FileOpError::HoldsHidden)
    );
    assert_eq!(on_disk(&r.lib()), before);
    // Production may.
    let prod = Some(EnvTag::Production);
    r.svc
        .trash(r.id, &rel("@production/drop_old.sql"), prod)
        .await
        .unwrap();
    r.svc
        .create(r.id, &rel("@staging/t.sql"), EntryKind::Sql, staging)
        .await
        .unwrap();
    assert!(r.lib().join("@staging/t.sql").exists());
    assert!(!r.lib().join("@production/drop_old.sql").exists());
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn a_folder_swapped_for_a_link_is_never_written_through() {
    let r = rig(&["sub/a.sql"]).await;
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(outside.path().join("a.sql"), b"theirs").unwrap();
    // The listed folder becomes a link to a folder outside the library.
    std::fs::remove_dir_all(r.lib().join("sub")).unwrap();
    std::os::unix::fs::symlink(outside.path(), r.lib().join("sub")).unwrap();
    for got in [
        r.svc
            .create(r.id, &rel("sub/b.sql"), EntryKind::Sql, None)
            .await,
        r.svc
            .rename(r.id, &rel("sub/a.sql"), &rel("sub/c.sql"), None)
            .await,
        r.svc.trash(r.id, &rel("sub/a.sql"), None).await,
    ] {
        assert!(
            matches!(got, Err(FileOpError::Path(_))),
            "refused on the way: {got:?}"
        );
    }
    assert_eq!(on_disk(outside.path()), ["a.sql"]);
    assert_eq!(
        std::fs::read(outside.path().join("a.sql")).unwrap(),
        b"theirs"
    );
    assert!(on_disk(r.trash.path()).is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn refresh_lists_a_change_no_watcher_reported() {
    let r = rig(&["a.sql"]).await;
    write_files(&r.lib(), &["b.sql"]);
    r.svc.refresh(r.id).await;
    wait_until("the refresh to list b.sql", || {
        r.listed(None) == ["a.sql", "b.sql"]
    })
    .await;
}
// ## m4b-t13-explorer

// ## m4b-t13-explorer fix round 1

/// R-T13-1 (review C1): a name taken in any ASCII case is `Exists` on every system, for a
/// rename and a create alike, and both files keep their bytes.
#[tokio::test(flavor = "multi_thread")]
async fn a_case_variant_of_an_existing_name_is_never_replaced() {
    let r = rig(&["b.sql", "c.sql"]).await;
    assert_eq!(
        r.svc.rename(r.id, &rel("b.sql"), &rel("C.sql"), None).await,
        Err(FileOpError::Exists)
    );
    assert_eq!(
        r.svc
            .create(r.id, &rel("C.sql"), EntryKind::Sql, None)
            .await,
        Err(FileOpError::Exists)
    );
    assert_eq!(
        r.svc
            .create(r.id, &rel("B.SQL"), EntryKind::Sql, None)
            .await,
        Err(FileOpError::Exists)
    );
    assert_eq!(on_disk(&r.lib()), ["b.sql", "c.sql"]);
    assert_eq!(std::fs::read(r.lib().join("b.sql")).unwrap(), b"b.sql");
    assert_eq!(std::fs::read(r.lib().join("c.sql")).unwrap(), b"c.sql");
}

/// A case-only rename of the entry itself works on every system and keeps its bytes.
#[tokio::test(flavor = "multi_thread")]
async fn a_case_only_rename_keeps_the_file() {
    let r = rig(&["Rapor.sql", "Notlar/x.md"]).await;
    r.svc
        .rename(r.id, &rel("Rapor.sql"), &rel("rapor.sql"), None)
        .await
        .unwrap();
    r.svc
        .rename(r.id, &rel("Notlar"), &rel("notlar"), None)
        .await
        .unwrap();
    assert_eq!(on_disk(&r.lib()), ["notlar/", "notlar/x.md", "rapor.sql"]);
    assert_eq!(
        std::fs::read(r.lib().join("rapor.sql")).unwrap(),
        b"Rapor.sql"
    );
}

/// Two names of one file (a hard link) are two entries: never "the same entry".
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn a_hard_linked_name_is_not_the_same_entry() {
    let r = rig(&["a.sql"]).await;
    std::fs::hard_link(r.lib().join("a.sql"), r.lib().join("b.sql")).unwrap();
    r.svc.refresh(r.id).await;
    wait_until("b.sql listed", || r.listed(None) == ["a.sql", "b.sql"]).await;
    assert_eq!(
        r.svc.rename(r.id, &rel("a.sql"), &rel("b.sql"), None).await,
        Err(FileOpError::Exists)
    );
    assert_eq!(on_disk(&r.lib()), ["a.sql", "b.sql"]);
}

/// R-T13-2 (review m4): a rename never changes which connections see the entry or anything
/// inside it.
#[tokio::test(flavor = "multi_thread")]
async fn a_rename_never_changes_the_environment_scope() {
    let r = rig(&["@staging/s.sql", "common/c.sql"]).await;
    let staging = Some(EnvTag::Staging);
    let before = on_disk(&r.lib());
    // Out of an environment folder (staging-only scripts would show under production) and
    // into one (unscoped scripts would vanish from other connections).
    assert_eq!(
        r.svc
            .rename(r.id, &rel("@staging"), &rel("common2"), staging)
            .await,
        Err(FileOpError::ScopeChange)
    );
    assert_eq!(
        r.svc
            .rename(r.id, &rel("common"), &rel("@staging"), staging)
            .await,
        Err(FileOpError::ScopeChange)
    );
    assert_eq!(on_disk(&r.lib()), before);
    // The same tag in another case keeps the scope.
    r.svc
        .rename(r.id, &rel("@staging"), &rel("@STAGING"), staging)
        .await
        .unwrap();
    assert!(r.lib().join("@STAGING/s.sql").exists());
}

/// Review H1 / I1: a folder whose subtree the scan did not list in full is neither trashed nor
/// renamed, by any connection, so a hidden `@production` file past a limit is never taken.
#[tokio::test(flavor = "multi_thread")]
async fn a_folder_past_a_scan_limit_is_never_trashed() {
    for limits in [
        Limits {
            max_depth: 3,
            ..Limits::default()
        },
        Limits {
            max_files: 1,
            ..Limits::default()
        },
    ] {
        let r = rig_with(&["reports/a.sql", "reports/deep/@production/z.sql"], limits).await;
        let before = on_disk(&r.lib());
        for env in [Some(EnvTag::Staging), Some(EnvTag::Production)] {
            assert_eq!(
                r.svc.trash(r.id, &rel("reports"), env).await,
                Err(FileOpError::NotFullyListed),
                "{limits:?} {env:?}"
            );
            assert_eq!(
                r.svc
                    .rename(r.id, &rel("reports"), &rel("raporlar"), env)
                    .await,
                Err(FileOpError::NotFullyListed),
                "{limits:?} {env:?}"
            );
        }
        assert_eq!(on_disk(&r.lib()), before);
        assert!(on_disk(r.trash.path()).is_empty());
    }
}

/// A folder holding an environment folder the connection does not show, even one the scan
/// never lists (no script in it), is neither trashed nor renamed: a names-only walk finds it.
#[tokio::test(flavor = "multi_thread")]
async fn a_folder_holding_an_unlisted_hidden_env_folder_is_refused() {
    let r = rig(&["reports/a.sql"]).await;
    let extra = r.lib().join("reports/@production");
    std::fs::create_dir_all(&extra).unwrap();
    std::fs::write(extra.join("data.csv"), b"id\n1\n").unwrap();
    let staging = Some(EnvTag::Staging);
    assert_eq!(
        r.svc.trash(r.id, &rel("reports"), staging).await,
        Err(FileOpError::HoldsHidden)
    );
    assert_eq!(
        r.svc
            .rename(r.id, &rel("reports"), &rel("raporlar"), staging)
            .await,
        Err(FileOpError::HoldsHidden)
    );
    assert!(extra.join("data.csv").exists());
    // Production shows it: the folder may go.
    r.svc
        .trash(r.id, &rel("reports"), Some(EnvTag::Production))
        .await
        .unwrap();
    assert!(!r.lib().join("reports").exists());
}

/// Review H2: Turkish dotted and dotless i are names of their own, never folded onto `i.sql`;
/// an ASCII case variant is `Exists`; `i.sql` is never replaced.
#[tokio::test(flavor = "multi_thread")]
async fn turkish_i_names_never_replace_an_ascii_name() {
    let r = rig(&["i.sql", "rapor.sql"]).await;
    for name in ["İ.sql", "ı.sql"] {
        let got = r.svc.create(r.id, &rel(name), EntryKind::Sql, None).await;
        // ## m4b-t13-explorer (re-review n2): NTFS's upcase table may fold `ı` (or `İ`) onto
        // `I`, the same name as `i.sql` there; the product then refuses, which is the safety
        // property. Elsewhere they are names of their own.
        if cfg!(windows) {
            assert!(
                matches!(got, Ok(()) | Err(FileOpError::Exists)),
                "{name}: {got:?}"
            );
        } else {
            assert_eq!(got, Ok(()), "{name}");
        }
    }
    assert_eq!(
        r.svc
            .create(r.id, &rel("I.sql"), EntryKind::Sql, None)
            .await,
        Err(FileOpError::Exists)
    );
    assert_eq!(
        r.svc
            .rename(r.id, &rel("rapor.sql"), &rel("I.SQL"), None)
            .await,
        Err(FileOpError::Exists)
    );
    assert_eq!(std::fs::read(r.lib().join("i.sql")).unwrap(), b"i.sql");
}

/// Review H3: a folder is never renamed onto an existing one, even an empty one.
#[tokio::test(flavor = "multi_thread")]
async fn a_folder_is_never_renamed_onto_another() {
    let r = rig(&["x/a.sql"]).await;
    std::fs::create_dir(r.lib().join("y")).unwrap();
    assert_eq!(
        r.svc.rename(r.id, &rel("x"), &rel("y"), None).await,
        Err(FileOpError::Exists)
    );
    assert!(r.lib().join("x/a.sql").exists());
    assert!(r.lib().join("y").is_dir());
}
// ## m4b-t13-explorer
