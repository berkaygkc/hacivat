//! Code-first error classification, quick fixes and the class registry (HV-CONN-03, Task 10).

use std::io;

use hacivat_driver::errors::{
    Phase, RawError, RawErrorKind, ServerMessage, classify, classify_tls_setup, fixes_for,
    reachable_classes, registry_ids,
};
use hacivat_net::{TlsFailure, TlsSetupError};
use hacivat_proto::DbmsKind::{MariaDb as Maria, MySql as My, Postgres as Pg, Sqlite as Lite};
use hacivat_proto::ErrClass::{self, *};
use hacivat_proto::{DbmsKind, FixId};

fn raw(kind: RawErrorKind) -> RawError {
    RawError { kind, server: None }
}

/// A server error of engine `k` with `code` (a SQLSTATE for PostgreSQL, a vendor code for the
/// others) and English text.
fn db_err(k: DbmsKind, code: &str, text: &str) -> RawError {
    db_err_lang(k, code, text, "en")
}

fn db_err_lang(k: DbmsKind, code: &str, text: &str, lang: &str) -> RawError {
    let (sqlstate, vendor_code) = match k {
        DbmsKind::Postgres => (Some(code.to_string()), None),
        _ => (
            None,
            Some(code.parse::<i64>().expect("a numeric vendor code")),
        ),
    };
    RawError {
        kind: RawErrorKind::Db,
        server: Some(ServerMessage {
            sqlstate,
            vendor_code,
            severity: None,
            text: text.to_string(),
            detail: None,
            hint: None,
            lang_hint: Some(lang.to_string()),
        }),
    }
}

/// A server error with no code at all, only text in `lang`.
fn text_err(text: &str, lang: Option<&str>) -> RawError {
    RawError {
        kind: RawErrorKind::Db,
        server: Some(ServerMessage {
            sqlstate: None,
            vendor_code: None,
            severity: None,
            text: text.to_string(),
            detail: None,
            hint: None,
            lang_hint: lang.map(str::to_string),
        }),
    }
}

// --- The brief's rows ---

#[test]
fn pg_mysql_sqlite_codes() {
    for (k, code, class) in [
        (Pg, "28P01", AuthInvalid),
        (Pg, "3D000", ConfigUnknownDatabase),
        (Pg, "53300", ServerTooManyConnections),
        (Pg, "57P03", ServerStarting),
        (Pg, "25006", ServerReadOnly),
        (My, "1045", AuthInvalid),
        (My, "3118", AuthLocked),
        (My, "1049", ConfigUnknownDatabase),
        (My, "1862", AuthExpired),
        (My, "2061", AuthMethodUnsupported),
        (Maria, "1045", AuthInvalid),
        (Maria, "4151", AuthLocked),
        (Lite, "14", ConfigUnknownDatabase),
        (Lite, "26", ConfigInvalid),
    ] {
        assert_eq!(
            classify(k, Phase::DriverConnect, &db_err(k, code, "x")).class,
            class,
            "{k:?} {code}"
        );
    }
}

#[test]
fn transport_and_tls_rules() {
    assert_eq!(
        classify(Pg, Phase::Transport, &raw(RawErrorKind::Dns)).class,
        NetworkDns
    );
    assert_eq!(
        classify(
            Pg,
            Phase::Transport,
            &raw(RawErrorKind::Io(io::ErrorKind::ConnectionRefused))
        )
        .class,
        NetworkRefused
    );
    assert_eq!(
        classify(
            Pg,
            Phase::DriverConnect,
            &raw(RawErrorKind::Tls(TlsFailure::UnknownIssuer))
        )
        .class,
        TlsUntrusted
    );
    assert_eq!(
        classify(
            My,
            Phase::DriverConnect,
            &raw(RawErrorKind::Tls(TlsFailure::NameMismatch))
        )
        .class,
        TlsHostname
    );
    assert_eq!(
        classify(Pg, Phase::DriverConnect, &raw(RawErrorKind::Cancelled)).class,
        Cancelled
    );
    assert_eq!(
        classify(Pg, Phase::Tls, &raw(RawErrorKind::TlsNotOffered)).class,
        TlsNotSupportedByServer
    );
}

#[test]
fn code_beats_localized_text() {
    let e = db_err_lang(
        Pg,
        "28P01",
        "ÖLÜMCÜL:  \"deploy\" kullanıcısı için parola doğrulaması başarısız oldu",
        "tr",
    );
    assert_eq!(classify(Pg, Phase::DriverConnect, &e).class, AuthInvalid);
}

#[test]
fn fixes_and_retryable() {
    assert_eq!(
        fixes_for(AuthInvalid),
        [FixId::ReenterPassword, FixId::EditConnection]
    );
    assert_eq!(
        fixes_for(ServerUnsupportedVersion),
        [
            FixId::SupportedVersions,
            FixId::EditConnection,
            FixId::ConnectAnyway
        ]
    );
    assert_eq!(
        fixes_for(TlsUntrusted),
        [FixId::EncryptWithoutVerifying, FixId::EditConnection]
    );
    // Controller ruling B (T10 review) overrides the brief's `[]`.
    assert_eq!(fixes_for(ConfigInvalid), [FixId::EditConnection]);
    assert!(!classify(Pg, Phase::DriverConnect, &db_err(Pg, "28P01", "")).retryable);
}

#[test]
fn registry_lists_every_class_once() {
    assert_eq!(
        registry_ids(),
        ErrClass::ALL.iter().map(|c| c.id()).collect::<Vec<_>>()
    );
    assert_eq!(reachable_classes().len(), 26);
}

// --- The rest of the tables ---

#[test]
fn every_code_of_the_tables() {
    for (k, code, class) in [
        (Pg, "28000", AuthzDatabase),
        (Pg, "57P01", ServerShutdown),
        (Pg, "57P02", ServerShutdown),
        (Pg, "08001", NetworkReset),
        (Pg, "08006", NetworkReset),
        (My, "1820", AuthExpired),
        (My, "1040", ServerTooManyConnections),
        (My, "1290", ServerReadOnly),
        (My, "1792", ServerReadOnly),
        (My, "1044", ConfigUnknownDatabase),
        (Maria, "1044", ConfigUnknownDatabase),
        (My, "2059", AuthMethodUnsupported),
        (Maria, "3118", AuthLocked),
        (Maria, "1049", ConfigUnknownDatabase),
        (Lite, "8", ServerReadOnly),
        (Lite, "5", Internal),
        (Lite, "6", Internal),
    ] {
        assert_eq!(
            classify(k, Phase::DriverConnect, &db_err(k, code, "x")).class,
            class,
            "{k:?} {code}"
        );
    }
}

#[test]
fn mariadb_only_codes_stay_mariadb_only() {
    // 4151 is MariaDB's "account locked"; MySQL gives the same number another meaning.
    assert_eq!(
        classify(My, Phase::DriverConnect, &db_err(My, "4151", "x")).class,
        Internal
    );
}

#[test]
fn sqlite_extended_codes_use_their_primary_code() {
    // SQLITE_READONLY_DBMOVED = 8 | (4 << 8); SQLITE_CANTOPEN_ISDIR = 14 | (2 << 8).
    assert_eq!(
        classify(Lite, Phase::DriverConnect, &db_err(Lite, "1032", "x")).class,
        ServerReadOnly
    );
    assert_eq!(
        classify(Lite, Phase::DriverConnect, &db_err(Lite, "526", "x")).class,
        ConfigUnknownDatabase
    );
}

#[test]
fn a_sqlstate_from_mysql_is_not_read_as_postgres() {
    // MySQL also sends SQLSTATEs; 28000 must not become authz.database through the PG table.
    let mut e = db_err(My, "1045", "x");
    e.server.as_mut().unwrap().sqlstate = Some("28000".into());
    assert_eq!(classify(My, Phase::Auth, &e).class, AuthInvalid);
    e.server.as_mut().unwrap().vendor_code = Some(9999);
    assert_eq!(classify(My, Phase::Auth, &e).class, Internal);
}

#[test]
fn every_transport_kind() {
    use io::ErrorKind as K;
    for (kind, class) in [
        (RawErrorKind::Io(K::TimedOut), NetworkTimeout),
        (RawErrorKind::Io(K::ConnectionReset), NetworkReset),
        (RawErrorKind::Io(K::ConnectionAborted), NetworkReset),
        (RawErrorKind::Io(K::BrokenPipe), NetworkReset),
        (RawErrorKind::Io(K::UnexpectedEof), NetworkReset),
        (RawErrorKind::Io(K::HostUnreachable), NetworkUnreachable),
        (RawErrorKind::Io(K::NetworkUnreachable), NetworkUnreachable),
        (RawErrorKind::Io(K::NetworkDown), NetworkUnreachable),
        (RawErrorKind::Io(K::AddrNotAvailable), NetworkUnreachable),
        (RawErrorKind::Io(K::PermissionDenied), Internal),
        (RawErrorKind::Timeout, NetworkTimeout),
        (RawErrorKind::Tls(TlsFailure::Expired), TlsUntrusted),
        (RawErrorKind::Tls(TlsFailure::NotYetValid), TlsUntrusted),
        (RawErrorKind::Tls(TlsFailure::Protocol), TlsProtocol),
        (
            RawErrorKind::Tls(TlsFailure::ClientCertRequired),
            TlsClientCertRequired,
        ),
        (RawErrorKind::Tls(TlsFailure::Other), TlsHandshake),
        (RawErrorKind::Protocol, Internal),
        (RawErrorKind::Unsupported, Internal),
        (RawErrorKind::PasswordRequired, AuthInvalid),
        (RawErrorKind::Other, Internal),
        (RawErrorKind::Db, Internal),
    ] {
        assert_eq!(
            classify(Pg, Phase::Transport, &raw(kind.clone())).class,
            class,
            "{kind:?}"
        );
    }
}

#[test]
fn name_resolution_failures_are_dns() {
    assert_eq!(
        classify(
            Pg,
            Phase::NameResolution,
            &raw(RawErrorKind::Io(io::ErrorKind::Other))
        )
        .class,
        NetworkDns
    );
    // A timeout and a cancel keep their own classes in every phase.
    assert_eq!(
        classify(Pg, Phase::NameResolution, &raw(RawErrorKind::Timeout)).class,
        NetworkTimeout
    );
    assert_eq!(
        classify(Pg, Phase::NameResolution, &raw(RawErrorKind::Cancelled)).class,
        Cancelled
    );
}

#[test]
fn a_cancel_wins_over_a_server_code() {
    let mut e = db_err(Pg, "28P01", "x");
    e.kind = RawErrorKind::Cancelled;
    assert_eq!(classify(Pg, Phase::Auth, &e).class, Cancelled);
}

#[test]
fn a_coded_error_is_never_read_by_its_text() {
    // MySQL 1698 (auth_socket: "Access denied for user 'root'@'localhost'") is not a wrong
    // password, and the tables do not know it: the English fragment must not decide.
    let e = db_err(My, "1698", "Access denied for user 'root'@'localhost'");
    assert_eq!(classify(My, Phase::Auth, &e).class, Internal);
    // A PostgreSQL SQLSTATE the table lacks, with text a fragile rule would match.
    let e = db_err(
        Pg,
        "XX000",
        "FATAL: too many connections for role \"deploy\"",
    );
    assert_eq!(classify(Pg, Phase::Auth, &e).class, Internal);
    // The same texts without any code do reach the fragile rules.
    assert_eq!(
        classify(
            My,
            Phase::Auth,
            &text_err("Access denied for user 'root'@'localhost'", None)
        )
        .class,
        AuthInvalid
    );
}

#[test]
fn fragile_text_rules_only_for_english() {
    let pw = "password authentication failed for user \"deploy\"";
    assert_eq!(
        classify(Pg, Phase::Auth, &text_err(pw, None)).class,
        AuthInvalid
    );
    assert_eq!(
        classify(Pg, Phase::Auth, &text_err(pw, Some("en"))).class,
        AuthInvalid
    );
    assert_eq!(
        classify(Pg, Phase::Auth, &text_err(pw, Some("EN-us"))).class,
        AuthInvalid
    );
    // The same English words from a server that says it speaks Turkish are not trusted.
    assert_eq!(
        classify(Pg, Phase::Auth, &text_err(pw, Some("tr"))).class,
        Internal
    );
    assert_eq!(
        classify(
            My,
            Phase::Auth,
            &text_err("Access denied for user 'deploy'@'10.0.0.5'", None)
        )
        .class,
        AuthInvalid
    );
    assert_eq!(
        classify(
            Pg,
            Phase::DriverConnect,
            &text_err("FATAL: the database system is starting up", None)
        )
        .class,
        ServerStarting
    );
    assert_eq!(
        classify(Pg, Phase::DriverConnect, &text_err("something odd", None)).class,
        Internal
    );
}

#[test]
fn retryable_follows_the_class() {
    for (e, retry) in [
        (
            raw(RawErrorKind::Io(io::ErrorKind::ConnectionRefused)),
            true,
        ),
        (raw(RawErrorKind::Timeout), true),
        (db_err(Pg, "57P03", "x"), true),
        (raw(RawErrorKind::Tls(TlsFailure::UnknownIssuer)), false),
        (raw(RawErrorKind::Dns), false),
        (raw(RawErrorKind::Cancelled), false),
    ] {
        assert_eq!(classify(Pg, Phase::Transport, &e).retryable, retry, "{e:?}");
    }
}

#[test]
fn classified_carries_the_fixes_of_its_class() {
    let c = classify(Pg, Phase::Auth, &db_err(Pg, "3D000", "x"));
    assert_eq!(c.fixes, [FixId::UseDatabase, FixId::EditConnection]);
    let c = classify(Pg, Phase::Transport, &raw(RawErrorKind::Dns));
    assert_eq!(c.fixes, [FixId::TryAgain, FixId::EditConnection]);
}

#[test]
fn the_normative_fix_table() {
    for (class, fixes) in [
        (NetworkDns, &[FixId::TryAgain, FixId::EditConnection][..]),
        (NetworkRefused, &[FixId::TryAgain, FixId::EditConnection]),
        (NetworkTimeout, &[FixId::TryAgain, FixId::EditConnection]),
        (NetworkReset, &[FixId::TryAgain, FixId::EditConnection]),
        (
            NetworkUnreachable,
            &[FixId::TryAgain, FixId::EditConnection],
        ),
        (
            NetworkIpv6Unavailable,
            &[FixId::TryAgain, FixId::EditConnection],
        ),
        (
            TlsHostname,
            &[FixId::EncryptWithoutVerifying, FixId::EditConnection],
        ),
        (
            ConfigUnknownDatabase,
            &[FixId::UseDatabase, FixId::EditConnection],
        ),
        (
            ServerTooManyConnections,
            &[FixId::TryAgain, FixId::EditConnection],
        ),
        (ServerStarting, &[FixId::TryAgain, FixId::EditConnection]),
        (TlsProtocol, &[FixId::EditConnection]),
        (AuthExpired, &[FixId::EditConnection]),
        (ServerShutdown, &[FixId::EditConnection]),
        (Cancelled, &[FixId::EditConnection]),
        (Internal, &[FixId::EditConnection]),
        (KaragozForbidden, &[FixId::EditConnection]),
        (SshHostKeyChanged, &[FixId::EditConnection]),
    ] {
        assert_eq!(fixes_for(class), fixes, "{class:?}");
    }
}

#[test]
fn reachable_classes_are_the_m2a_set() {
    let ids: Vec<&str> = reachable_classes().iter().map(|c| c.id()).collect();
    assert_eq!(
        ids,
        [
            "network.dns",
            "network.refused",
            "network.timeout",
            "network.reset",
            "network.unreachable",
            "network.ipv6_unavailable",
            "tls.handshake",
            "tls.untrusted",
            "tls.hostname",
            "tls.protocol",
            "tls.client_cert_required",
            "tls.not_supported_by_server",
            "auth.invalid",
            "auth.expired",
            "auth.locked",
            "auth.method_unsupported",
            "authz.database",
            "config.invalid",
            "config.unknown_database",
            "server.too_many_connections",
            "server.starting",
            "server.shutdown",
            "server.read_only",
            "server.unsupported_version",
            "cancelled",
            "internal",
        ]
    );
}

#[test]
fn every_classified_class_is_reachable() {
    // Whatever classify can answer must have Fluent texts (L026 checks reachable classes only).
    let reachable = reachable_classes();
    let mut kinds = vec![
        RawErrorKind::Db,
        RawErrorKind::Dns,
        RawErrorKind::TlsNotOffered,
        RawErrorKind::Timeout,
        RawErrorKind::Protocol,
        RawErrorKind::Cancelled,
        RawErrorKind::Unsupported,
        RawErrorKind::PasswordRequired,
        RawErrorKind::Other,
    ];
    kinds.extend(
        [
            TlsFailure::UnknownIssuer,
            TlsFailure::NameMismatch,
            TlsFailure::Expired,
            TlsFailure::NotYetValid,
            TlsFailure::Protocol,
            TlsFailure::ClientCertRequired,
            TlsFailure::Other,
        ]
        .map(RawErrorKind::Tls),
    );
    use io::ErrorKind as K;
    kinds.extend(
        [
            K::ConnectionRefused,
            K::TimedOut,
            K::ConnectionReset,
            K::HostUnreachable,
            K::Other,
        ]
        .map(RawErrorKind::Io),
    );
    let phases = [
        Phase::Validate,
        Phase::PreHooks,
        Phase::CredentialFiles,
        Phase::NameResolution,
        Phase::Transport,
        Phase::Tls,
        Phase::Auth,
        Phase::DriverConnect,
        Phase::PostConnect,
    ];
    for kind in kinds {
        for phase in phases {
            let c = classify(Pg, phase, &raw(kind.clone())).class;
            assert!(reachable.contains(&c), "{kind:?} in {phase:?} -> {c:?}");
        }
    }
    for (k, codes) in [
        (
            Pg,
            &[
                "28P01", "28000", "3D000", "53300", "57P03", "57P01", "57P02", "25006", "08001",
                "08006",
            ][..],
        ),
        (
            Maria,
            &[
                "1045", "1862", "1820", "3118", "4151", "1049", "1040", "1290", "1792", "1044",
                "2059", "2061",
            ],
        ),
        (Lite, &["14", "26", "8", "5", "6"]),
    ] {
        for code in codes {
            let c = classify(k, Phase::DriverConnect, &db_err(k, code, "x")).class;
            assert!(reachable.contains(&c), "{k:?} {code} -> {c:?}");
        }
    }
}

#[test]
fn tls_setup_errors_are_configuration_problems() {
    // `hacivat-net` refuses a host that is neither an IP literal nor a DNS name before any
    // byte is sent; so does an unreadable CA, certificate or key file.
    for e in [
        TlsSetupError::InvalidServerName("db bad".into()),
        TlsSetupError::CaFileUnreadable("/nope.pem".into()),
        TlsSetupError::CertFileUnreadable("/nope.pem".into()),
        TlsSetupError::KeyUnreadable("/nope.key".into()),
        TlsSetupError::KeyEncrypted("/enc.key".into()),
    ] {
        let c = classify_tls_setup(&e);
        assert_eq!(c.class, ConfigInvalid, "{e:?}");
        assert!(!c.retryable);
        assert_eq!(c.fixes, [FixId::EditConnection]);
    }
}
