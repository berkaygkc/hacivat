//! Unit tests of the MySQL driver that need no server: the `Opts` it would connect with, the
//! TLS mode mapping, error mapping and probe evaluation. `Opts` is never `Debug`-formatted (it
//! holds the password, P49).

use std::io;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use hacivat_driver::{
    Driver, DriverConnect, Endpoint, ProbeWarning, RawError, RawErrorKind, ServerMessage,
};
use hacivat_driver_mysql::{
    MySqlDriver, opts_for, opts_for_forwarded, os_roots, probe_warnings, raw_error, ssl_opts,
    user_databases,
};
use hacivat_net::{
    AddressClass, BoxedStream, StreamFactory, Target, TlsFailure, TlsMode, TlsSettings,
    build_tls_params, install_crypto_provider,
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
        tls: build_tls_params(&settings, host, AddressClass::Private).unwrap(),
        user: Some("deploy".into()),
        password: Some(SecretString::from("s3cret")),
        database: Some("orders".into()),
        application_name: "Hacivat".into(),
        read_only: false,
        options: vec![],
        connect_timeout: Duration::from_secs(5),
        events: Arc::new(|_| {}),
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
fn ssl_opts_mapping() {
    assert!(ssl_opts(TlsMode::Disable).is_none());
    let r = ssl_opts(TlsMode::Require).unwrap();
    assert!(r.accept_invalid_certs());
    assert!(r.skip_domain_validation());
    assert!(r.disable_built_in_roots());
    let f = ssl_opts(TlsMode::VerifyFull).unwrap();
    assert!(!f.accept_invalid_certs());
    assert!(!f.skip_domain_validation());
    assert!(f.disable_built_in_roots());
    let a = ssl_opts(TlsMode::Auto).unwrap();
    assert!(!a.accept_invalid_certs());
    assert!(!a.skip_domain_validation());
    assert!(a.disable_built_in_roots());
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

#[tokio::test]
async fn trust_is_the_os_roots_never_the_built_in_ones() {
    let o = opts_for(&req_any());
    let ssl = o.ssl_opts().expect("Auto asks for TLS");
    assert!(ssl.disable_built_in_roots());
    assert_eq!(ssl.root_certs().len(), os_roots().len());
    // Every root is handed over as the OS's DER bytes.
    for (given, os) in ssl.root_certs().iter().zip(os_roots()) {
        assert_eq!(given.read().await.unwrap().as_ref(), os.as_slice());
    }
}

#[test]
fn os_roots_are_all_usable_trust_anchors() {
    for der in os_roots() {
        let mut store = rustls::RootCertStore::empty();
        assert!(
            store
                .add(rustls::pki_types::CertificateDer::from(der.clone()))
                .is_ok()
        );
    }
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

#[tokio::test]
async fn verify_ca_is_refused_before_any_byte_is_sent() {
    // P11: Validate rejects VerifyCa for MySQL; the driver refuses it again.
    let opens = Arc::new(AtomicUsize::new(0));
    let req = base("db.internal", false, TlsMode::VerifyCa, opens.clone());
    let e = match MySqlDriver::new().connect(req).await {
        Ok(_) => panic!("VerifyCa must be refused"),
        Err(e) => e,
    };
    assert_eq!(e.kind, RawErrorKind::Unsupported);
    assert_eq!(opens.load(Ordering::SeqCst), 0);
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
    assert!(matches!(e.kind, RawErrorKind::Io(_)), "{:?}", e.kind);
    assert_eq!(opens.load(Ordering::SeqCst), 1);
}
