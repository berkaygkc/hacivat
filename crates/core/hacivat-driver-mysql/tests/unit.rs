//! Unit tests of the MySQL driver that need no server: the `Opts` it would connect with, the
//! TLS mode mapping, error mapping and probe evaluation. `Opts` is never `Debug`-formatted (it
//! holds the password, P49).

use std::io;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use hacivat_driver::{
    Driver, DriverConnect, DriverEvent, Endpoint, InsecureAuth, ProbeWarning, RawError,
    RawErrorKind, ServerMessage, TxStatus,
};
use hacivat_driver_mysql::{
    MySqlDriver, command_tag, opts_for, opts_for_forwarded, plaintext_fallback_allowed,
    probe_warnings, raw_error, trail_events, tx_from_status, user_databases,
};
use hacivat_net::{
    AddressClass, BoxedStream, StreamFactory, Target, TlsFailure, TlsMode, TlsSettings,
    TlsVerified, build_tls_params, install_crypto_provider,
};
use secrecy::SecretString;

fn unreachable_factory(opens: Arc<AtomicUsize>) -> StreamFactory {
    StreamFactory::new(move || {
        opens.fetch_add(1, Ordering::SeqCst);
        Box::pin(async { Err::<BoxedStream, _>(io::Error::from(io::ErrorKind::ConnectionRefused)) })
    })
}

fn base(host: &str, direct: bool, tls: TlsMode, opens: Arc<AtomicUsize>) -> DriverConnect {
    install_crypto_provider();
    let resolved = vec![SocketAddr::new(
        IpAddr::V4(Ipv4Addr::new(10, 0, 0, 5)),
        3307,
    )];
    let settings = TlsSettings {
        mode: tls,
        ..TlsSettings::default()
    };
    DriverConnect {
        endpoint: Endpoint::Network {
            factory: unreachable_factory(opens),
            direct: direct.then(|| Target::Tcp {
                host: host.to_string(),
                port: 3307,
            }),
            resolved,
            tls_host: host.to_string(),
        },
        tls: build_tls_params(&settings, host, 3306, AddressClass::Private, None).unwrap(),
        user: Some("deploy".into()),
        password: Some(SecretString::from("s3cret")),
        database: Some("orders".into()),
        application_name: "Hacivat".into(),
        read_only: false,
        options: vec![],
        connect_timeout: Duration::from_secs(5),
        events: Arc::new(|_| {}),
        insecure_auth: Default::default(),
    }
}

fn req_any() -> DriverConnect {
    base("db.internal", true, TlsMode::Auto, Arc::default())
}

fn req_tls(host: &str, mode: TlsMode) -> DriverConnect {
    base(host, false, mode, Arc::default())
}

#[test]
fn no_local_infile_handler_and_no_socket_reconnect() {
    let o = opts_for(&req_any());
    assert!(o.local_infile_handler().is_none());
    assert!(!o.prefer_socket());
    assert!(o.socket().is_none());
}

#[test]
fn forwarded_connect_overrides_the_tls_host_name() {
    let o = opts_for_forwarded(&req_tls("db.acme.io", TlsMode::VerifyFull), 49505);
    assert_eq!(o.ip_or_hostname(), "127.0.0.1");
    assert_eq!(o.tcp_port(), 49505);
    assert!(!o.prefer_socket());
    assert!(o.resolved_ips().is_none());
    assert_eq!(
        o.ssl_opts().unwrap().tls_hostname_override(),
        Some("db.acme.io")
    );
}

#[test]
fn direct_connect_dials_the_tls_host_with_the_resolved_addresses() {
    let o = opts_for(&req_any());
    assert_eq!(o.ip_or_hostname(), "db.internal");
    assert_eq!(o.tcp_port(), 3307);
    assert_eq!(
        o.resolved_ips().as_deref(),
        Some(&[IpAddr::V4(Ipv4Addr::new(10, 0, 0, 5))][..])
    );
    assert!(!o.prefer_socket());
    assert_eq!(o.user(), Some("deploy"));
    assert_eq!(o.db_name(), Some("orders"));
}

#[test]
fn disable_sends_no_ssl_request() {
    let o = opts_for(&base("db.internal", true, TlsMode::Disable, Arc::default()));
    assert!(o.ssl_opts().is_none());
}

#[test]
fn read_only_sets_the_session_transaction_mode() {
    let mut req = req_any();
    assert!(opts_for(&req).setup().is_empty());
    req.read_only = true;
    assert_eq!(
        opts_for(&req).setup(),
        ["SET SESSION TRANSACTION READ ONLY"]
    );
}

#[test]
fn mysql_async_native_options_never_reach_the_opts() {
    // A user-typed option passes sanitize unless it is on the deny list, so the driver must not
    // read `mysql_async`'s own option names from it.
    let mut req = req_any();
    req.options = [
        ("enable_cleartext_plugin", "true"),
        ("prefer_socket", "true"),
        ("socket", "/tmp/other.sock"),
        ("require_ssl", "false"),
        ("secure_auth", "false"),
        ("local_infile", "1"),
        ("allowLoadLocalInfile", "true"),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v.to_string()))
    .collect();
    let o = opts_for(&req);
    assert!(!o.enable_cleartext_plugin());
    assert!(!o.prefer_socket());
    assert!(o.socket().is_none());
    assert!(o.secure_auth());
    assert!(o.local_infile_handler().is_none());
    assert!(o.ssl_opts().is_some());
}

#[test]
fn server_errors_keep_code_and_state() {
    let e = raw_error(&mysql_async::Error::Server(mysql_async::ServerError {
        code: 1045,
        message: "Access denied for user 'deploy'".into(),
        state: "28000".into(),
    }));
    assert_eq!(e.kind, RawErrorKind::Db);
    let ServerMessage {
        sqlstate,
        vendor_code,
        text,
        ..
    } = e.server.unwrap();
    assert_eq!(sqlstate.as_deref(), Some("28000"));
    assert_eq!(vendor_code, Some(1045));
    assert_eq!(text, "Access denied for user 'deploy'");
}

fn tls_io_error(e: rustls::Error) -> mysql_async::Error {
    mysql_async::Error::Io(mysql_async::IoError::Io(io::Error::new(
        io::ErrorKind::InvalidData,
        e,
    )))
}

#[test]
fn rustls_name_errors_are_name_mismatches() {
    use rustls::CertificateError as C;
    let context = C::NotValidForNameContext {
        expected: rustls::pki_types::ServerName::try_from("db.acme.io")
            .unwrap()
            .to_owned(),
        presented: vec![],
    };
    for c in [C::NotValidForName, context] {
        let e = raw_error(&tls_io_error(rustls::Error::InvalidCertificate(c)));
        assert_eq!(e.kind, RawErrorKind::Tls(TlsFailure::NameMismatch));
    }
    let e = raw_error(&tls_io_error(rustls::Error::InvalidCertificate(
        C::UnknownIssuer,
    )));
    assert_eq!(e.kind, RawErrorKind::Tls(TlsFailure::UnknownIssuer));
    let wrapped = mysql_async::Error::Io(mysql_async::IoError::Tls(mysql_async::TlsError::Tls(
        rustls::Error::InvalidCertificate(C::Expired),
    )));
    assert_eq!(
        raw_error(&wrapped).kind,
        RawErrorKind::Tls(TlsFailure::Expired)
    );
}

#[test]
fn driver_errors_are_mapped_without_text() {
    let no_tls = mysql_async::Error::Driver(mysql_async::DriverError::NoClientSslFlagFromServer);
    assert_eq!(
        raw_error(&no_tls),
        RawError {
            kind: RawErrorKind::TlsNotOffered,
            server: None
        }
    );
    let refused = mysql_async::Error::Io(mysql_async::IoError::Io(
        io::ErrorKind::ConnectionRefused.into(),
    ));
    assert_eq!(
        raw_error(&refused).kind,
        RawErrorKind::Io(io::ErrorKind::ConnectionRefused)
    );
    let cleartext = mysql_async::Error::Driver(mysql_async::DriverError::CleartextPluginDisabled);
    assert_eq!(raw_error(&cleartext).kind, RawErrorKind::Unsupported);
}

#[test]
fn probe_findings() {
    assert_eq!(
        probe_warnings(Some(0), Some(0), Some(28800), Some(0)),
        Vec::<ProbeWarning>::new()
    );
    assert_eq!(
        probe_warnings(Some(1), Some(1), Some(30), Some(1)),
        [
            ProbeWarning::ServerReadOnly,
            ProbeWarning::SuperReadOnly,
            ProbeWarning::LowWaitTimeout { seconds: 30 },
            ProbeWarning::LocalInfileOn,
        ]
    );
    // MariaDB has no `super_read_only`; unknown values report nothing.
    assert_eq!(
        probe_warnings(None, None, Some(60), None),
        Vec::<ProbeWarning>::new()
    );
}

#[test]
fn system_schemas_are_not_listed() {
    let names = [
        "information_schema",
        "orders",
        "MYSQL",
        "performance_schema",
        "sys",
        "orders_archive",
        "hv_test",
    ]
    .map(String::from);
    assert_eq!(
        user_databases(names.to_vec(), 10),
        ["orders", "orders_archive", "hv_test"]
    );
    assert_eq!(user_databases(names.to_vec(), 1), ["orders"]);
}

#[test]
fn the_mariadb_handshake_prefix_is_stripped_only_before_a_digit() {
    use hacivat_driver_mysql::server_version_text;
    assert_eq!(
        server_version_text("5.5.5-10.6.21-MariaDB-ubu2004"),
        "10.6.21-MariaDB-ubu2004"
    );
    assert_eq!(server_version_text("5.5.5-"), "5.5.5-");
    assert_eq!(server_version_text("5.5.5-log"), "5.5.5-log");
    assert_eq!(server_version_text("8.4.3"), "8.4.3");
}

#[tokio::test]
async fn a_failed_forwarded_open_is_an_error_not_a_hang() {
    let opens = Arc::new(AtomicUsize::new(0));
    let req = base("db.internal", false, TlsMode::Disable, opens.clone());
    let e = match MySqlDriver::new().connect(req).await {
        Ok(_) => panic!("the factory refuses"),
        Err(e) => e,
    };
    assert_eq!(e.kind, RawErrorKind::Io(io::ErrorKind::ConnectionRefused));
    assert_eq!(opens.load(Ordering::SeqCst), 1);
}

/// An Auto request toward `host` whose pipeline resolved it to `ip`.
fn auto_at(host: &str, ip: [u8; 4], factory: StreamFactory) -> DriverConnect {
    install_crypto_provider();
    let ip = IpAddr::V4(Ipv4Addr::from(ip));
    let resolved = vec![SocketAddr::new(ip, 3306)];
    let class = hacivat_net::classify_host(host, &resolved);
    let settings = TlsSettings::default();
    assert_eq!(settings.mode, TlsMode::Auto);
    DriverConnect {
        endpoint: Endpoint::Network {
            factory,
            direct: None,
            resolved,
            tls_host: host.to_string(),
        },
        tls: build_tls_params(&settings, host, 3306, class, None).unwrap(),
        user: Some("deploy".into()),
        password: Some(SecretString::from("s3cret")),
        database: None,
        application_name: "Hacivat".into(),
        read_only: false,
        options: vec![],
        connect_timeout: Duration::from_secs(5),
        events: Arc::new(|_| {}),
        insecure_auth: Default::default(),
    }
}

#[test]
fn plaintext_fallback_only_toward_loopback() {
    let f = || unreachable_factory(Arc::default());
    let private = auto_at("db.internal", [10, 0, 0, 5], f());
    // hacivat-net would allow it (Auto toward a non-public address); MySQL does not.
    assert!(private.tls.as_ref().unwrap().plaintext_allowed);
    assert!(!plaintext_fallback_allowed(&private));
    assert!(plaintext_fallback_allowed(&auto_at(
        "localhost",
        [127, 0, 0, 1],
        f()
    )));
    assert!(!plaintext_fallback_allowed(&auto_at(
        "db.example.com",
        [203, 0, 113, 7],
        f()
    )));
    let mut require = auto_at("localhost", [127, 0, 0, 1], f());
    let settings = TlsSettings {
        mode: TlsMode::Require,
        ..TlsSettings::default()
    };
    require.tls =
        build_tls_params(&settings, "localhost", 3306, AddressClass::Loopback, None).unwrap();
    assert!(!plaintext_fallback_allowed(&require));
}

/// A MySQL 8.4 initial handshake without `CLIENT_SSL`, as an attacker stripping TLS sends it.
fn handshake_without_tls() -> Vec<u8> {
    let mut p = vec![10u8];
    p.extend_from_slice(b"8.4.3\0");
    p.extend_from_slice(&7u32.to_le_bytes());
    p.extend_from_slice(b"abcdefgh\0");
    // PROTOCOL_41 | SECURE_CONNECTION | LONG_PASSWORD | TRANSACTIONS; PLUGIN_AUTH; no SSL.
    p.extend_from_slice(&0xA201u16.to_le_bytes());
    p.push(0xff);
    p.extend_from_slice(&2u16.to_le_bytes());
    p.extend_from_slice(&0x0008u16.to_le_bytes());
    p.push(21);
    p.extend_from_slice(&[0; 10]);
    p.extend_from_slice(b"ijklmnopqrst\0");
    p.extend_from_slice(b"caching_sha2_password\0");
    let len = u32::try_from(p.len()).unwrap().to_le_bytes();
    let mut packet = vec![len[0], len[1], len[2], 0];
    packet.extend_from_slice(&p);
    packet
}

/// Serves one connection: sends `greeting`, then returns every byte the client sent.
async fn fake_server(greeting: Vec<u8>) -> (SocketAddr, tokio::task::JoinHandle<Vec<u8>>) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let served = tokio::spawn(async move {
        let (mut s, _) = listener.accept().await.unwrap();
        s.write_all(&greeting).await.unwrap();
        let mut got = Vec::new();
        let _ = tokio::time::timeout(Duration::from_secs(2), s.read_to_end(&mut got)).await;
        got
    });
    (addr, served)
}

fn tcp_factory(addr: SocketAddr, opens: Arc<AtomicUsize>) -> StreamFactory {
    StreamFactory::new(move || {
        opens.fetch_add(1, Ordering::SeqCst);
        Box::pin(
            async move { Ok(Box::new(tokio::net::TcpStream::connect(addr).await?) as BoxedStream) },
        )
    })
}

#[tokio::test]
async fn auto_toward_a_private_host_without_tls_sends_no_credentials() {
    let (addr, served) = fake_server(handshake_without_tls()).await;
    let opens = Arc::new(AtomicUsize::new(0));
    let req = auto_at(
        "db.internal",
        [10, 0, 0, 5],
        tcp_factory(addr, opens.clone()),
    );
    let e = match MySqlDriver::new().connect(req).await {
        Ok(_) => panic!("a private host without TLS must not get a plaintext sign-in"),
        Err(e) => e,
    };
    assert_eq!(e.kind, RawErrorKind::TlsNotOffered);
    // One connection, and not a single byte back: no handshake response, no user, no
    // password, no request for the server's RSA key.
    assert_eq!(opens.load(Ordering::SeqCst), 1);
    assert_eq!(served.await.unwrap(), Vec::<u8>::new());
}

#[test]
fn the_trail_follows_what_the_handshake_reached() {
    use DriverEvent::*;
    let db = |code| RawError {
        kind: RawErrorKind::Db,
        server: Some(ServerMessage {
            sqlstate: None,
            vendor_code: Some(code),
            severity: None,
            text: String::new(),
            detail: None,
            hint: None,
            lang_hint: None,
        }),
    };
    let kind = |k| RawError {
        kind: k,
        server: None,
    };
    assert_eq!(
        trail_events(true, Ok(())),
        [TlsStarted, TlsDone, AuthStarted, AuthDone]
    );
    assert_eq!(trail_events(false, Ok(())), [AuthStarted, AuthDone]);
    assert_eq!(
        trail_events(
            true,
            Err(&kind(RawErrorKind::Tls(TlsFailure::UnknownIssuer)))
        ),
        [TlsStarted]
    );
    assert_eq!(
        trail_events(true, Err(&db(1045))),
        [TlsStarted, TlsDone, AuthStarted]
    );
    assert_eq!(trail_events(false, Err(&db(1045))), [AuthStarted]);
    // Refused before the greeting: neither TLS nor sign-in began.
    assert_eq!(trail_events(true, Err(&db(1130))), []);
    assert_eq!(
        trail_events(true, Err(&kind(RawErrorKind::TlsNotOffered))),
        []
    );
    assert_eq!(
        trail_events(
            true,
            Err(&kind(RawErrorKind::Io(io::ErrorKind::ConnectionRefused)))
        ),
        []
    );
}

// ## m2a-finalfix
/// M6: the driver trusts exactly what the pipeline read and checked and never reads the CA
/// file again. M2b (Task 13, Q2): the CA certificates live in `TlsParams.config`'s verifier,
/// so `mysql_async` gets that configuration and no roots of its own.
#[test]
fn the_ca_file_is_never_read_again_by_the_driver() {
    install_crypto_provider();
    let path = std::env::temp_dir().join(format!("hv-mysql-ca-{}.pem", std::process::id()));
    std::fs::write(
        &path,
        include_bytes!("../../hacivat-session/tests/fixtures/test-ca.pem"),
    )
    .unwrap();
    let settings = TlsSettings {
        mode: TlsMode::VerifyFull,
        ca_file: Some(path.clone()),
        ..TlsSettings::default()
    };
    let mut req = req_any();
    req.tls =
        build_tls_params(&settings, "db.internal", 3306, AddressClass::Private, None).unwrap();
    std::fs::remove_file(&path).unwrap();
    let o = opts_for(&req);
    let ssl = o.ssl_opts().expect("VerifyFull asks for TLS");
    assert!(ssl.root_certs().is_empty());
    let config = ssl.rustls_client_config().expect("Hacivat's configuration");
    assert!(Arc::ptr_eq(config, &req.tls.as_ref().unwrap().config));
}

// ## m2b-t13-mysql

/// Q2: every TLS mode hands `mysql_async` Hacivat's own `ClientConfig` (pins, review, outcome,
/// platform verifier, CA file, decrypted client key) and the name to verify; none of
/// `mysql_async`'s own trust settings is used.
#[test]
fn every_tls_mode_uses_hacivats_client_config() {
    for mode in [
        TlsMode::Auto,
        TlsMode::Require,
        TlsMode::VerifyCa,
        TlsMode::VerifyFull,
    ] {
        for direct in [true, false] {
            let req = base("db.acme.io", direct, mode, Arc::default());
            let o = if direct {
                opts_for(&req)
            } else {
                opts_for_forwarded(&req, 49505)
            };
            let ssl = o
                .ssl_opts()
                .unwrap_or_else(|| panic!("{mode:?} asks for TLS"));
            let config = ssl.rustls_client_config().expect("Hacivat's configuration");
            assert!(
                Arc::ptr_eq(config, &req.tls.as_ref().unwrap().config),
                "{mode:?}"
            );
            assert_eq!(ssl.tls_hostname_override(), Some("db.acme.io"), "{mode:?}");
            assert!(ssl.root_certs().is_empty(), "{mode:?}");
            assert!(ssl.client_identity().is_none(), "{mode:?}");
            assert!(!ssl.accept_invalid_certs(), "{mode:?}");
            assert!(!ssl.skip_domain_validation(), "{mode:?}");
        }
    }
}

#[test]
fn read_write_connections_have_multi_statements_and_none_has_local_infile() {
    let mut req = req_any();
    let o = opts_for(&req);
    assert!(o.multi_statements());
    assert!(!o.local_infile_capability());
    assert!(o.local_infile_handler().is_none());
    assert!(!o.enable_cleartext_plugin());
    req.read_only = true;
    let o = opts_for(&req);
    assert!(!o.multi_statements());
    assert!(!o.local_infile_capability());
}

/// Q20: the guard reads the attempt's live TLS outcome when the server asks for the password;
/// it is not decided from the mode when the `Opts` are built.
#[test]
fn the_password_guard_reads_the_live_outcome() {
    let mut req = base("db.internal", true, TlsMode::Require, Arc::default());
    let outcome = req.tls.as_ref().unwrap().outcome.clone();
    let guard = opts_for(&req).password_guard().expect("a guard").clone();
    assert!(!guard(), "no handshake yet");
    for (o, want) in [
        (None, false),
        (Some(TlsVerified::No), false),
        (Some(TlsVerified::Pinned), true),
        (Some(TlsVerified::Full), true),
        (Some(TlsVerified::CaOnly), true),
    ] {
        *outcome.lock().unwrap() = o;
        assert_eq!(guard(), want, "{o:?}");
    }
    // `Allow` lets the password go whatever the outcome.
    req.insecure_auth = InsecureAuth::Allow;
    *outcome.lock().unwrap() = None;
    let guard = opts_for(&req).password_guard().expect("a guard").clone();
    assert!(guard());
}

#[test]
fn a_plaintext_attempt_never_reads_an_outcome() {
    let req = base("db.internal", true, TlsMode::Disable, Arc::default());
    assert!(req.tls.is_none());
    let guard = opts_for(&req).password_guard().expect("a guard").clone();
    assert!(!guard());
}

#[tokio::test]
async fn verify_ca_is_no_longer_refused() {
    // P11 superseded: VerifyCa runs through Hacivat's `ChainOnly` verifier, so the driver dials.
    let opens = Arc::new(AtomicUsize::new(0));
    let req = base("db.internal", false, TlsMode::VerifyCa, opens.clone());
    let e = match MySqlDriver::new().connect(req).await {
        Ok(_) => panic!("the factory refuses"),
        Err(e) => e,
    };
    assert_eq!(e.kind, RawErrorKind::Io(io::ErrorKind::ConnectionRefused));
    assert_eq!(opens.load(Ordering::SeqCst), 1);
}

#[test]
fn insecure_password_refusal_and_interruptions_are_mapped() {
    let refused = mysql_async::Error::Driver(mysql_async::DriverError::InsecurePasswordRefused);
    assert_eq!(raw_error(&refused).kind, RawErrorKind::InsecureAuthRefused);
    for code in [1317, 3024, 1969] {
        let e = raw_error(&mysql_async::Error::Server(mysql_async::ServerError {
            code,
            message: "Query execution was interrupted".into(),
            state: "70100".into(),
        }));
        assert_eq!(e.kind, RawErrorKind::Cancelled, "{code}");
        // The server's code stays: the session reads 1317 once a cancel was asked for.
        assert_eq!(e.server.and_then(|s| s.vendor_code), Some(i64::from(code)));
    }
    let other = raw_error(&mysql_async::Error::Server(mysql_async::ServerError {
        code: 1146,
        message: String::new(),
        state: "42S02".into(),
    }));
    assert_eq!(other.kind, RawErrorKind::Db);
}

#[test]
fn a_refused_password_stops_the_trail_at_sign_in() {
    use DriverEvent::*;
    let refused = RawError {
        kind: RawErrorKind::InsecureAuthRefused,
        server: None,
    };
    assert_eq!(
        trail_events(true, Err(&refused)),
        [TlsStarted, TlsDone, AuthStarted]
    );
    assert_eq!(trail_events(false, Err(&refused)), [AuthStarted]);
}

/// Q35: MySQL has no command tags; the tag is the statement's first keyword, ASCII upper case.
#[test]
fn command_tag_is_the_first_keyword_in_ascii_upper_case() {
    for (sql, want) in [
        ("insert into t values (1)", Some("INSERT")),
        ("  \n\tSelect 1", Some("SELECT")),
        ("-- note\n# more\n/* block */ delete from t", Some("DELETE")),
        ("with x as (select 1) delete from t", Some("WITH")),
        ("call p()", Some("CALL")),
        ("do sleep(1)", Some("DO")),
        // MySQL and MariaDB executable comments run their contents: the keyword inside counts.
        ("/*!50000 DROP TABLE t */", Some("DROP")),
        ("/*! update t set a = 1 */", Some("UPDATE")),
        ("/*M!100000 REPLACE INTO t VALUES (1) */", Some("REPLACE")),
        ("/*!*/ delete from t", Some("DELETE")),
        (
            "/*!50000*/ /* c */ insert into t values (1)",
            Some("INSERT"),
        ),
        (
            "/*!50000 /* nested */ load data infile 'x' into table t */",
            Some("LOAD"),
        ),
        ("/*+ BKA(t) */ select 1", Some("SELECT")),
        ("(select 1)", None),
        ("", None),
        ("-- only a comment", None),
        ("/* unterminated", None),
        ("\u{130}NSERT", None),
    ] {
        assert_eq!(command_tag(sql).as_deref(), want, "{sql:?}");
    }
}

#[test]
fn the_transaction_status_comes_from_the_status_flags() {
    use mysql_async::consts::StatusFlags as F;
    assert_eq!(tx_from_status(F::empty()), TxStatus::Idle);
    assert_eq!(tx_from_status(F::SERVER_STATUS_AUTOCOMMIT), TxStatus::Idle);
    assert_eq!(
        tx_from_status(F::SERVER_STATUS_IN_TRANS | F::SERVER_STATUS_AUTOCOMMIT),
        TxStatus::InTx
    );
    assert_eq!(
        tx_from_status(F::SERVER_STATUS_IN_TRANS | F::SERVER_STATUS_IN_TRANS_READONLY),
        TxStatus::InTx
    );
}
