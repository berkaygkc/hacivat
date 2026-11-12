//! The SQLite driver over bundled `rusqlite` (HV-DRV-01): a local file opened with safe flags
//! on its own blocking thread, with journal and writability probes (HV-CONN-07). There is no
//! network, TLS or authentication.

// `RawError` carries the engine's message by value (HV-CONN-03), so its size is by design.
#![allow(
    clippy::result_large_err,
    reason = "RawError is a fixed interface; one per failed call"
)]

mod open;
mod probe;

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

    /// Opens an [`Endpoint::File`]; any other endpoint is `Unsupported`. The connection is
    /// read-only when the endpoint, the request or a `mode=ro` option says so: options can only
    /// tighten, and every other option is ignored.
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
            let read_only = read_only || req.read_only || mode_is_ro(&req.options);
            let conn = open::open(path, create_if_missing, read_only).await?;
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
/// is the engine's own, shown verbatim. SQLite has no SQLSTATE.
fn raw_error(e: rusqlite::Error) -> RawError {
    match e {
        rusqlite::Error::SqliteFailure(failure, message) => RawError {
            kind: RawErrorKind::Db,
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
        },
        rusqlite::Error::NulError(_) | rusqlite::Error::InvalidPath(_) => invalid_path(),
        _ => RawError {
            kind: RawErrorKind::Other,
            server: None,
        },
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
        c.execute_batch("CREATE TABLE t(x); INSERT INTO t VALUES (1);")
            .unwrap();
        drop(c);
        (dir, path)
    }

    async fn open_at(path: &Path, create_if_missing: bool, read_only: bool) -> SqliteConn {
        open(path.to_path_buf(), create_if_missing, read_only)
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
    async fn read_only_open_rejects_writes() {
        let (_dir, f) = tmp_db();
        let mut c = open_at(&f, false, true).await;
        assert_eq!(exec_err_code(&mut c, "CREATE TABLE t2(x)").await, 8);
        assert_eq!(exec_err_code(&mut c, "INSERT INTO t VALUES (2)").await, 8);
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
        };
        // The probe stays quiet about a read-only file only when read-only was asked for, so
        // a write-protected file tells the two apart.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&f, std::fs::Permissions::from_mode(0o444)).unwrap();
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
        #[cfg(not(unix))]
        {
            let _ = request;
        }
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
                "UPDATE sqlite_schema SET sql = 'x' WHERE name = 't'"
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
        };
        match SqliteDriver::new().connect(req).await {
            Ok(_) => panic!("a network endpoint opened"),
            Err(e) => assert_eq!(e.kind, RawErrorKind::Unsupported),
        }
    }
}
