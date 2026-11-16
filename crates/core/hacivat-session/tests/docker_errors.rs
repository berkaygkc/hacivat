//! Real error classes from real servers (Task 19, exit criterion 2). The helpers connect with
//! `auto_or_require(&t)` (Require where `tls_default = self_signed`), so TLS never masks the
//! class under test. Run with `cargo xtask docker test --set full -p hacivat-session`.

mod docker_support;

use docker_support::*;
use hacivat_net::{TlsMode, TlsVerified};
use hacivat_proto::ErrClass;
use hacivat_secrets::Retention;
use hacivat_session::profile::Profile;

/// The class of connecting `id` after `edit` with `password`; `None` when the active set
/// leaves `id` out.
async fn class_of(
    id: &str,
    edit: impl FnOnce(Profile) -> Profile,
    password: Option<&str>,
) -> Option<ErrClass> {
    err_of_with(id, edit, password).await.map(|e| e.class)
}

/// Asserts the class of connecting `id` after `edit` with `password`, unless the active set
/// leaves `id` out.
async fn assert_class(
    id: &str,
    edit: impl FnOnce(Profile) -> Profile,
    password: Option<&str>,
    expected: ErrClass,
) {
    if let Some(class) = class_of(id, edit, password).await {
        assert_eq!(class, expected, "{id}");
    }
}

#[tokio::test]
#[ignore = "docker"]
async fn error_classes_against_real_servers() {
    if !docker_enabled() {
        return;
    }
    for id in ["pg17", "mysql84", "mariadb114"] {
        assert_class(
            id,
            |p| p.retention(Retention::Never),
            Some("wrong"),
            ErrClass::AuthInvalid,
        )
        .await;
        if let Some(e) = err_of_with(id, |p| p.database("orders_v2"), None).await {
            assert_eq!(e.class, ErrClass::ConfigUnknownDatabase, "{id}");
            assert_eq!(
                e.suggestions.first().map(String::as_str),
                Some("orders"),
                "{id}: {:?}",
                e.suggestions
            );
        }
    }
    assert_class(
        "mysql84",
        |p| p.user("hv_locked"),
        None,
        ErrClass::AuthLocked,
    )
    .await;
    // MariaDB answers a locked account with its own code (4151, not MySQL's 3118).
    assert_class(
        "mariadb114",
        |p| p.user("hv_locked"),
        None,
        ErrClass::AuthLocked,
    )
    .await;
    assert_class("pg17", |p| p.port(1), None, ErrClass::NetworkRefused).await;
    assert_class(
        "pg17",
        |p| p.host("nonexistent.invalid"),
        None,
        ErrClass::NetworkDns,
    )
    .await;
    assert_class("pg13", |p| p, None, ErrClass::ServerUnsupportedVersion).await;
    assert_class(
        "pg17-tls",
        |p| p.tls(TlsMode::VerifyFull),
        None,
        ErrClass::TlsUntrusted,
    )
    .await;
}

#[tokio::test]
#[ignore = "docker"]
async fn turkish_server_wrong_password_is_auth_invalid() {
    if !docker_enabled() {
        return;
    }
    let Some(e) = err_of_with("pg17-tr", |p| p, Some("wrong")).await else {
        return;
    };
    assert_eq!(e.class, ErrClass::AuthInvalid);
    let server = e.server.expect("the server's message is kept");
    // PostgreSQL 17's Turkish catalogue says "şifre" (not "parola") for "password":
    // `"hv_test" kullanıcısı için şifre doğrulaması başarısız oldu`.
    assert!(
        server
            .text
            .contains("kullanıcısı için şifre doğrulaması başarısız oldu"),
        "server text must be the verbatim Turkish message: {}",
        server.text
    );
    // Classified by its code, never by the localized text.
    assert_eq!(server.sqlstate.as_deref(), Some("28P01"));
    assert_eq!(server.lang_hint.as_deref(), Some("und"));
}

#[tokio::test]
#[ignore = "docker"]
async fn encrypt_without_verifying_fix_connects() {
    if !docker_enabled() {
        return;
    }
    let Some(t) = target_required("pg17-tls") else {
        return;
    };
    let live = connect_target(&t, TlsMode::Require).await;
    assert!(live.tls.encrypted);
    assert_eq!(live.tls.verified, TlsVerified::No);
    live.close().await;
}
