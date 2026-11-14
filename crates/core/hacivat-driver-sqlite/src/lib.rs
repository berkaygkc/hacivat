//! The SQLite driver over bundled `rusqlite` (HV-DRV-01): a local file opened with safe flags
//! on its own blocking thread, with journal and writability probes (HV-CONN-07). There is no
//! network, TLS or authentication. M2b (Task 11): one statement per `execute`, the interrupt
//! as the cancel handle, and the read-only profile's hardening (`harden`).

// `RawError` carries the engine's message by value (HV-CONN-03), so its size is by design.
#![allow(
    clippy::result_large_err,
    reason = "RawError is a fixed interface; one per failed call"
)]

// ## m2b-t01-interfaces: filled by Task 11.
mod exec;
mod harden;
mod open;
mod probe;
// ## m3a-t01b-seams (fix round 1): typed streaming and the value decoders (Task 15).
#[doc(hidden)]
pub mod decode;
#[doc(hidden)]
pub mod typed;

use std::io;

use futures::future::BoxFuture;
use hacivat_driver::{
    CancelMechanism, Driver, DriverCaps, DriverConnect, DriverDescriptor, Endpoint, RawConn,
    RawError, RawErrorKind, ServerMessage,
};
use hacivat_proto::DbmsKind;

static DESCRIPTOR: DriverDescriptor = DriverDescriptor {
    kind: DbmsKind::Sqlite,
    crate_name: "hacivat-driver-sqlite",
    caps: DriverCaps {
        stream_injection: false,
        in_protocol_tls: false,
        cancel: CancelMechanism::Interrupt,
        file_based: true,
    },
};

/// The SQLite driver (HV-DRV-01).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct SqliteDriver;

impl SqliteDriver {
    pub fn new() -> Self {
        SqliteDriver
    }
}

impl Driver for SqliteDriver {
    fn descriptor(&self) -> &DriverDescriptor {
        &DESCRIPTOR
    }

    /// Opens an [`Endpoint::File`]; any other endpoint is `Unsupported`. Two independent
    /// controls make it read-only: the file's read-only mode (the endpoint's flag or a
    /// `mode=ro` option; options can only tighten, and every other option is ignored), and the
    /// read-only profile (`DriverConnect.read_only`), which also hardens the connection with
    /// the default-deny authorizer (B21).
    fn connect(
        &self,
        req: DriverConnect,
    ) -> BoxFuture<'static, Result<Box<dyn RawConn>, RawError>> {
        Box::pin(async move {
            let Endpoint::File {
                path,
                create_if_missing,
                read_only,
            } = req.endpoint
            else {
                return Err(RawError {
                    kind: RawErrorKind::Unsupported,
                    server: None,
                });
            };
            let read_only = read_only || mode_is_ro(&req.options);
            let conn = open::open(
                path,
                create_if_missing,
                read_only,
                req.read_only,
                req.connect_timeout,
            )
            .await?;
            Ok(Box::new(conn) as Box<dyn RawConn>)
        })
    }
}

/// A `mode=ro` option (key and value in any ASCII case).
fn mode_is_ro(options: &[(String, String)]) -> bool {
    options
        .iter()
        .any(|(k, v)| k.eq_ignore_ascii_case("mode") && v.eq_ignore_ascii_case("ro"))
}

/// An SQLite failure keeps its extended result code in `vendor_code` (HV-CONN-03); the text
/// is the engine's own, shown verbatim. SQLite has no SQLSTATE. An interrupted statement
/// (`SQLITE_INTERRUPT`) is `Cancelled`. A second statement in the text is refused like a
/// syntax error (`SQLITE_ERROR`, reason `multiple-statements`).
fn raw_error(e: rusqlite::Error) -> RawError {
    match e {
        rusqlite::Error::SqliteFailure(failure, message) => engine_error(failure, message),
        // A statement SQLite could not prepare (rusqlite's `modern_sqlite` reports where).
        rusqlite::Error::SqlInputError { error, msg, .. } => engine_error(error, Some(msg)),
        rusqlite::Error::MultipleStatement => RawError {
            kind: RawErrorKind::Db,
            server: Some(ServerMessage {
                sqlstate: None,
                vendor_code: Some(i64::from(rusqlite::ffi::SQLITE_ERROR)),
                severity: None,
                text: "multiple-statements".to_string(),
                detail: None,
                hint: None,
                lang_hint: None,
            }),
        },
        rusqlite::Error::NulError(_) | rusqlite::Error::InvalidPath(_) => invalid_path(),
        _ => RawError {
            kind: RawErrorKind::Other,
            server: None,
        },
    }
}

/// The engine's own error: its extended code and text.
fn engine_error(failure: rusqlite::ffi::Error, message: Option<String>) -> RawError {
    RawError {
        kind: if failure.code == rusqlite::ErrorCode::OperationInterrupted {
            RawErrorKind::Cancelled
        } else {
            RawErrorKind::Db
        },
        server: Some(ServerMessage {
            sqlstate: None,
            vendor_code: Some(i64::from(failure.extended_code)),
            severity: None,
            text: message.unwrap_or_else(|| failure.to_string()),
            detail: None,
            hint: None,
            // SQLite's messages are English and never localized.
            lang_hint: Some("en".to_string()),
        }),
    }
}

/// A path SQLite must not see: relative, empty, or not representable.
fn invalid_path() -> RawError {
    RawError {
        kind: RawErrorKind::Io(io::ErrorKind::InvalidInput),
        server: None,
    }
}

/// The connection's thread is gone (closed, or it panicked).
fn gone() -> RawError {
    RawError {
        kind: RawErrorKind::Other,
        server: None,
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use hacivat_net::TlsVerified;
    use rusqlite::OpenFlags;
    use tempfile::TempDir;

    use super::*;
    use crate::open::{SqliteConn, open, open_flags};

    /// An existing database file in a fresh temporary directory (kept alive by the `TempDir`).
    fn tmp_db() -> (TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("existing.db");
        let c = rusqlite::Connection::open(&path).unwrap();
        c.execute_batch("CREATE TABLE seed(x); INSERT INTO seed VALUES (1);")
            .unwrap();
        drop(c);
        (dir, path)
    }

    async fn open_at(path: &Path, create_if_missing: bool, read_only: bool) -> SqliteConn {
        open(
            path.to_path_buf(),
            create_if_missing,
            read_only,
            false,
            std::time::Duration::from_secs(15),
        )
        .await
        .unwrap()
    }

    async fn exec(c: &mut SqliteConn, sql: &str) -> Result<(), RawError> {
        c.exec(sql).await
    }

    async fn exec_err_code(c: &mut SqliteConn, sql: &str) -> i64 {
        c.exec_err_code(sql).await
    }

    #[test]
    fn open_flags_follow_the_request_and_never_take_uris() {
        use OpenFlags as F;
        let rw = F::SQLITE_OPEN_READ_WRITE | F::SQLITE_OPEN_NO_MUTEX;
        assert_eq!(open_flags(false, false), rw);
        assert_eq!(open_flags(true, false), rw | F::SQLITE_OPEN_CREATE);
        let ro = F::SQLITE_OPEN_READ_ONLY | F::SQLITE_OPEN_NO_MUTEX;
        assert_eq!(open_flags(false, true), ro);
        assert_eq!(open_flags(true, true), ro, "read-only never creates");
        for (create, ro) in [(false, false), (true, false), (false, true), (true, true)] {
            assert!(!open_flags(create, ro).contains(F::SQLITE_OPEN_URI));
        }
    }

    #[test]
    fn only_a_mode_ro_option_changes_anything() {
        let opt = |k: &str, v: &str| vec![(k.to_string(), v.to_string())];
        assert!(mode_is_ro(&opt("mode", "ro")));
        assert!(mode_is_ro(&opt("MODE", "RO")));
        assert!(!mode_is_ro(&opt("mode", "rwc")));
        assert!(!mode_is_ro(&opt("mode", "rw")));
        assert!(!mode_is_ro(&opt("vfs", "ro")));
        assert!(!mode_is_ro(&[]));
    }

    #[tokio::test]
    async fn read_write_open_accepts_writes() {
        let (_dir, f) = tmp_db();
        let mut c = open_at(&f, false, false).await;
        exec(&mut c, "CREATE TABLE u(x)").await.unwrap();
        assert_eq!(
            c.query_i64("SELECT count(*) FROM sqlite_schema")
                .await
                .unwrap(),
            2
        );
    }

    #[tokio::test]
    async fn a_network_path_is_refused_before_any_file_call() {
        // `//host/share/a.db` is absolute on Unix, so only the network-path rule refuses it
        // (otherwise SQLite would answer CANTOPEN, a `Db` error).
        for p in [
            "//host/share/a.db",
            r"\\host\share\a.db",
            r"\??\UNC\host\a.db",
        ] {
            for guarded in [false, true] {
                let r = open(
                    PathBuf::from(p),
                    true,
                    false,
                    guarded,
                    std::time::Duration::from_secs(15),
                )
                .await;
                match r {
                    Ok(_) => panic!("{p} opened"),
                    Err(e) => {
                        assert_eq!(e.kind, RawErrorKind::Io(io::ErrorKind::InvalidInput), "{p}")
                    }
                }
            }
        }
    }

    #[tokio::test]
    async fn the_file_mode_and_the_profile_guard_are_independent() {
        let (_dir, f) = tmp_db();
        // The file's read-only mode alone: SQLite refuses the write (SQLITE_READONLY), no
        // authorizer is asked.
        let mut c = open_at(&f, false, true).await;
        assert_eq!(c.query_i64("PRAGMA query_only").await.unwrap(), 0);
        assert_eq!(
            exec_err_code(&mut c, "INSERT INTO seed VALUES (2)").await,
            8
        );
        // The profile guard alone also opens the file read-only, and the authorizer refuses
        // the write before the file is asked (SQLITE_AUTH).
        let mut c = open(
            f.clone(),
            true,
            false,
            true,
            std::time::Duration::from_secs(15),
        )
        .await
        .unwrap();
        assert_eq!(c.query_i64("PRAGMA query_only").await.unwrap(), 1);
        assert_eq!(
            exec_err_code(&mut c, "INSERT INTO seed VALUES (2)").await,
            23
        );
        assert!(
            c.call(|c| c.is_readonly(rusqlite::MAIN_DB))
                .await
                .unwrap()
                .unwrap()
        );
    }

    #[tokio::test]
    async fn read_only_open_rejects_writes() {
        let (_dir, f) = tmp_db();
        let mut c = open_at(&f, false, true).await;
        assert_eq!(exec_err_code(&mut c, "CREATE TABLE t(x)").await, 8);
        assert_eq!(
            exec_err_code(&mut c, "INSERT INTO seed VALUES (2)").await,
            8
        );
    }

    #[tokio::test]
    async fn the_request_or_a_mode_ro_option_makes_the_connection_read_only() {
        let (_dir, f) = tmp_db();
        let request = |read_only: bool, options: Vec<(String, String)>| DriverConnect {
            endpoint: Endpoint::File {
                path: f.clone(),
                create_if_missing: false,
                read_only: false,
            },
            tls: None,
            user: None,
            password: None,
            database: None,
            application_name: "Hacivat".into(),
            read_only,
            options,
            connect_timeout: std::time::Duration::from_secs(15),
            events: std::sync::Arc::new(|_| {}),
            insecure_auth: Default::default(),
        };
        // The probe stays quiet about a read-only file only when read-only was asked for, so
        // a write-protected file tells the two apart. The read-only bit (Unix) or attribute
        // (Windows) works on every platform.
        let mut perms = std::fs::metadata(&f).unwrap().permissions();
        perms.set_readonly(true);
        std::fs::set_permissions(&f, perms).unwrap();
        let not_writable = |w: &Vec<hacivat_driver::ProbeWarning>| {
            w.contains(&hacivat_driver::ProbeWarning::SqliteFileNotWritable)
        };
        let driver = SqliteDriver::new();
        let mut c = driver.connect(request(false, vec![])).await.unwrap();
        assert!(not_writable(&c.probe().await));
        let mut c = driver.connect(request(true, vec![])).await.unwrap();
        assert!(!not_writable(&c.probe().await));
        let ro = vec![("Mode".to_string(), "RO".to_string())];
        let mut c = driver.connect(request(false, ro)).await.unwrap();
        assert!(!not_writable(&c.probe().await));
    }

    #[tokio::test]
    async fn extension_loading_is_disabled() {
        let (_dir, f) = tmp_db();
        let mut c = open_at(&f, false, false).await;
        let err = exec(&mut c, "SELECT load_extension('x')")
            .await
            .unwrap_err();
        assert_eq!(err.kind, RawErrorKind::Db);
        assert!(err.server.unwrap().text.contains("not authorized"));
    }

    #[tokio::test]
    async fn defensive_mode_blocks_schema_corruption() {
        let (_dir, f) = tmp_db();
        let mut c = open_at(&f, false, false).await;
        // Without SQLITE_DBCONFIG_DEFENSIVE this pair rewrites the schema table.
        exec(&mut c, "PRAGMA writable_schema = ON").await.unwrap();
        assert_eq!(
            exec_err_code(
                &mut c,
                "UPDATE sqlite_schema SET sql = 'x' WHERE name = 'seed'"
            )
            .await
                & 0xff,
            1,
            "SQLITE_ERROR: sqlite_schema may not be modified"
        );
    }

    #[tokio::test]
    async fn the_schema_is_not_trusted() {
        let (_dir, f) = tmp_db();
        let mut c = open_at(&f, false, false).await;
        assert_eq!(c.query_i64("PRAGMA trusted_schema").await.unwrap(), 0);
    }

    #[tokio::test]
    async fn tls_summary_is_unencrypted() {
        let (_dir, f) = tmp_db();
        let mut c = open_at(&f, false, false).await;
        let s = c.tls_summary().await;
        assert!(!s.encrypted);
        assert_eq!(s.verified, TlsVerified::No);
        assert_eq!((s.version, s.cipher), (None, None));
    }

    #[tokio::test]
    async fn errors_carry_the_extended_code_and_the_engine_text() {
        let (_dir, f) = tmp_db();
        let mut c = open_at(&f, false, false).await;
        exec(&mut c, "CREATE TABLE k(id INTEGER PRIMARY KEY)")
            .await
            .unwrap();
        exec(&mut c, "INSERT INTO k VALUES (1)").await.unwrap();
        let err = exec(&mut c, "INSERT INTO k VALUES (1)").await.unwrap_err();
        let server = err.server.unwrap();
        // SQLITE_CONSTRAINT_PRIMARYKEY = SQLITE_CONSTRAINT (19) | (6 << 8).
        assert_eq!(server.vendor_code, Some(1555));
        assert_eq!(server.sqlstate, None);
        assert!(
            server.text.contains("UNIQUE constraint failed"),
            "{}",
            server.text
        );
    }

    #[tokio::test]
    async fn a_network_endpoint_is_unsupported() {
        let factory = hacivat_net::StreamFactory::new(|| {
            Box::pin(async { Err(std::io::Error::other("unused")) })
        });
        let req = DriverConnect {
            endpoint: Endpoint::Network {
                factory,
                direct: None,
                resolved: vec![],
                tls_host: "localhost".into(),
            },
            tls: None,
            user: None,
            password: None,
            database: None,
            application_name: "Hacivat".into(),
            read_only: false,
            options: vec![],
            connect_timeout: std::time::Duration::from_secs(15),
            events: std::sync::Arc::new(|_| {}),
            insecure_auth: Default::default(),
        };
        match SqliteDriver::new().connect(req).await {
            Ok(_) => panic!("a network endpoint opened"),
            Err(e) => assert_eq!(e.kind, RawErrorKind::Unsupported),
        }
    }
}
