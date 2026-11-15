//! Console autosave (Task 32): `<data>/consoles/<profile-uuid>/<n>.sql`, 1 s after the last
//! edit and at quit, written on the core runtime, skipped when the content hash is unchanged.
//!
//! - **Off the UI thread.** The UI thread only takes the editor's O(1) snapshot and hands it to
//!   the core runtime's blocking pool; the text, its hash (O(document), never per keystroke) and
//!   the write happen there, and nothing on the UI thread awaits them.
//! - **In order.** Each write carries the snapshot's version; a write older than the last one
//!   written is dropped, so two writes in flight never leave the older text on disk.
//! - **Atomic, durable, private.** Through `hacivat_settings::fsio::atomic_write` (0600, synced
//!   before the rename, the folder synced after) in a folder made by `create_dir_all_private`
//!   (0700): console SQL can hold secrets (`CREATE USER … PASSWORD`).
//! - **Never over a file it did not read.** Console numbers are reused (the next is one more
//!   than the highest open one), so a fresh console's `<n>.sql` may hold an earlier console's
//!   text: the first write of an autosave that neither restored nor wrote that file moves it
//!   aside first (`fsio::quarantine`, `<n>.sql.replaced-<secs>`).
//! - **At quit** the consoles with unsaved edits are written on the calling thread, like the
//!   settings and the profile store (`flush_writes`): the core runtime is stopping then.
//! - **Restore** reads the file's size first: a file over 20 MB is not read at all (R-M3-49,
//!   P26), and neither is one that cannot be read: it is moved aside unchanged
//!   (`<n>.sql.too-large-<secs>`, `.unreadable-<secs>`), the console says where, opens empty and
//!   saves its new text as usual.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use hacivat_proto::ProfileId;
use hacivat_settings::LocalPaths;
use hacivat_text::Snapshot;

/// How long after the last edit a console is saved.
pub const AUTOSAVE_DELAY: std::time::Duration = std::time::Duration::from_secs(1);

/// The largest saved console that opens (T1's limit; no T2 tier until M4b, R-M3-49).
pub const RESTORE_MAX_BYTES: u64 = 20 << 20;

/// The file of console `n` bound to `profile`.
pub fn console_path(paths: &LocalPaths, profile: ProfileId, n: u32) -> PathBuf {
    paths
        .consoles_dir(&profile.to_string())
        .join(format!("{n}.sql"))
}

/// What reading a saved console found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Restored {
    /// No file: a console that was never edited.
    Missing,
    /// The text and its content hash.
    Text { text: String, hash: u128 },
    /// Over [`RESTORE_MAX_BYTES`]: not read; `kept` is where it was moved aside.
    TooLarge { bytes: u64, kept: Option<PathBuf> },
    /// Unreadable (not UTF-8, a permission problem); `kept` is where it was moved aside.
    Unreadable { kept: Option<PathBuf> },
}

/// Reads the saved console at `path` (blocking: call it off the UI thread). A file it does not
/// open is moved aside unchanged, so the console's own file is free for its new text.
pub fn read(path: &Path) -> Restored {
    let set_aside = |tag: &str| {
        hacivat_settings::fsio::quarantine(path, tag, std::time::SystemTime::now())
            .map_err(|e| tracing::warn!(error = %e, "console file not moved aside"))
            .ok()
    };
    let bytes = match std::fs::metadata(path) {
        Ok(meta) => meta.len(),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Restored::Missing,
        Err(_) => {
            return Restored::Unreadable {
                kept: set_aside("unreadable"),
            };
        }
    };
    if bytes > RESTORE_MAX_BYTES {
        return Restored::TooLarge {
            bytes,
            kept: set_aside("too-large"),
        };
    }
    match std::fs::read_to_string(path) {
        Ok(text) => {
            let hash = hacivat_text::TextBuffer::from(text.as_str()).content_hash();
            Restored::Text { text, hash }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Restored::Missing,
        Err(_) => Restored::Unreadable {
            kept: set_aside("unreadable"),
        },
    }
}

/// The last write of one console: the snapshot version and the content hash on disk.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Written {
    version: Option<u64>,
    hash: Option<u128>,
    /// The file at the path is this console's (restored from it, or written by it).
    claimed: bool,
}

/// One console's autosave.
#[derive(Debug, Clone)]
pub struct Autosave {
    path: PathBuf,
    written: Arc<Mutex<Written>>,
}

impl Autosave {
    /// The autosave of the file at `path`; `on_disk` is the hash of what the file holds (a
    /// restored console), so an unchanged text is never written again.
    pub fn new(path: PathBuf, on_disk: Option<u128>) -> Self {
        Autosave {
            path,
            written: Arc::new(Mutex::new(Written {
                version: None,
                hash: on_disk,
                claimed: on_disk.is_some(),
            })),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Writes `snap` on the core runtime's blocking pool, unless its text is what the file
    /// holds or a newer version was written already. Not awaited.
    pub fn save_off_thread(&self, snap: Snapshot, rt: &tokio::runtime::Handle) {
        let this = self.clone();
        rt.spawn_blocking(move || {
            if let Err(e) = this.write_now(&snap) {
                tracing::warn!(error = %e, "console autosave failed");
            }
        });
    }

    /// Writes `snap` on the calling thread (quit; tests), with the same rules. `Ok(true)` when
    /// it wrote.
    pub fn write_now(&self, snap: &Snapshot) -> std::io::Result<bool> {
        let mut written = self.written.lock().unwrap_or_else(PoisonError::into_inner);
        if written.version.is_some_and(|v| v >= snap.version) {
            return Ok(false);
        }
        let hash = snap.buffer.content_hash();
        if written.hash == Some(hash) {
            written.version = Some(snap.version);
            return Ok(false);
        }
        if let Some(dir) = self.path.parent() {
            hacivat_settings::fsio::create_dir_all_private(dir)?;
        }
        if !written.claimed && self.path.exists() {
            // Another console's text (a reused number): kept, never overwritten.
            hacivat_settings::fsio::quarantine(
                &self.path,
                "replaced",
                std::time::SystemTime::now(),
            )?;
        }
        hacivat_settings::fsio::atomic_write(&self.path, snap.buffer.to_string().as_bytes())?;
        *written = Written {
            version: Some(snap.version),
            hash: Some(hash),
            claimed: true,
        };
        Ok(true)
    }

    /// The version last written (or found unchanged), if any.
    pub fn written_version(&self) -> Option<u64> {
        self.written
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .version
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snap(text: &str, version: u64) -> Snapshot {
        let model = hacivat_text::EditorModel::new(text, hacivat_text::ModelConfig::default());
        let mut s = model.snapshot();
        s.version = version;
        s
    }

    #[test]
    fn writes_once_per_content_and_never_an_older_version() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("p").join("2.sql");
        let a = Autosave::new(path.clone(), None);
        assert!(a.write_now(&snap("SELECT 1;", 3)).expect("write"));
        assert_eq!(std::fs::read_to_string(&path).expect("read"), "SELECT 1;");
        // The same text at a newer version: no write.
        assert!(!a.write_now(&snap("SELECT 1;", 4)).expect("write"));
        // An older version arriving late: dropped.
        assert!(!a.write_now(&snap("old", 2)).expect("write"));
        assert_eq!(std::fs::read_to_string(&path).expect("read"), "SELECT 1;");
        assert!(a.write_now(&snap("SELECT 2;", 5)).expect("write"));
        assert_eq!(std::fs::read_to_string(&path).expect("read"), "SELECT 2;");
        assert!(!path.with_extension("sql.tmp").exists());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).expect("meta").permissions().mode();
            assert_eq!(mode & 0o777, 0o600, "console SQL is private");
            let dir = std::fs::metadata(path.parent().expect("dir")).expect("meta");
            assert_eq!(dir.permissions().mode() & 0o777, 0o700);
        }
    }

    /// C3: a console number is reused; the earlier console's file is moved aside, never
    /// overwritten.
    #[test]
    fn a_fresh_autosave_moves_a_file_it_did_not_read_aside() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("2.sql");
        std::fs::write(&path, "SELECT 'earlier console';").expect("write");
        let a = Autosave::new(path.clone(), None);
        assert!(a.write_now(&snap("SELECT 'new';", 1)).expect("write"));
        assert_eq!(
            std::fs::read_to_string(&path).expect("read"),
            "SELECT 'new';"
        );
        let kept: Vec<_> = std::fs::read_dir(dir.path())
            .expect("dir")
            .filter_map(Result::ok)
            .map(|e| e.file_name().to_string_lossy().to_string())
            .filter(|n| n.starts_with("2.sql.replaced-"))
            .collect();
        assert_eq!(kept.len(), 1, "{kept:?}");
        assert_eq!(
            std::fs::read_to_string(dir.path().join(&kept[0])).expect("read"),
            "SELECT 'earlier console';"
        );
        // Its own file afterwards: overwritten in place, nothing more moved aside.
        assert!(a.write_now(&snap("SELECT 'newer';", 2)).expect("write"));
        assert_eq!(
            std::fs::read_dir(dir.path()).expect("dir").count(),
            2,
            "one kept file and the console's own"
        );
    }

    #[test]
    fn a_restored_text_is_not_written_back() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("1.sql");
        std::fs::write(&path, "SELECT 'İzmir';").expect("write");
        let Restored::Text { text, hash } = read(&path) else {
            panic!("restored");
        };
        assert_eq!(text, "SELECT 'İzmir';");
        let a = Autosave::new(path.clone(), Some(hash));
        assert!(!a.write_now(&snap(&text, 1)).expect("write"));
    }

    #[test]
    fn read_refuses_a_file_over_the_limit_without_reading_it() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("1.sql");
        let f = std::fs::File::create(&path).expect("create");
        f.set_len(RESTORE_MAX_BYTES + 1).expect("sparse file");
        let Restored::TooLarge { bytes, kept } = read(&path) else {
            panic!("refused");
        };
        assert_eq!(bytes, RESTORE_MAX_BYTES + 1);
        let kept = kept.expect("moved aside");
        assert!(!path.exists());
        assert_eq!(
            std::fs::metadata(&kept).expect("kept").len(),
            RESTORE_MAX_BYTES + 1
        );
        assert_eq!(read(&dir.path().join("none.sql")), Restored::Missing);
    }

    #[test]
    fn the_path_is_under_the_profile_folder() {
        let paths = LocalPaths::under(Path::new("/r"));
        let id = ProfileId::new();
        assert_eq!(
            console_path(&paths, id, 3),
            paths.consoles_dir(&id.to_string()).join("3.sql")
        );
    }
}
