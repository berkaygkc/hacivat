//! Extra options that have a typed setting, and extra options that carry a password (M2a final
//! review I1, I2).
//!
//! A profile keeps one value per setting. The libpq, JDBC and MySQL-client options below have a
//! typed field, so they never stay in `extra`: [`fold_typed_options`] moves them into the typed
//! settings, as a pasted URL's mapped parameters are (`MappedTo`). It runs wherever options
//! enter a profile: a pasted URL (`apply_parse`), the `pg_service.conf` import, the dialog's
//! extra properties, a load of `connections.toml` and every save. The form, the quick fixes and
//! the connect then all read the same typed value; the pipeline never lets an extra override it.
//!
//! The options (keys match ASCII-case-insensitively, without `-` and `_`, so libpq `sslmode`,
//! JDBC `sslMode` and MySQL-client `ssl-mode` are one key; values fold ASCII case and `_`):
//! - TLS mode: `sslmode`: `disable`/`disabled`; `allow`, `prefer`, `preferred` are Auto;
//!   `require`/`required`; `verify-ca`; `verify-full`/`verify-identity`. MySQL and MariaDB also
//!   read `useSSL` (`false` is Disable, `true` Require) when no `sslmode` is given.
//! - CA file: `sslrootcert` for the PostgreSQL family (`system`, or empty, keeps the platform
//!   roots); `sslCa`/`ssl-ca` for the MySQL family.
//! - Client certificate: `sslcert` with `sslkey` (`ssl-cert`, `ssl-key`). One of them alone
//!   folds only when the profile already has the other half.
//! - Timeout: `connect_timeout` in seconds; MySQL's JDBC `connectTimeout` (no separator) in
//!   milliseconds, rounded up. Zero or not a number stays an extra.
//! - Application name: `application_name` (`applicationName`).
//!
//! The last value of a key wins. A value no rule understands (`sslmode=bogus`) stays in `extra`
//! and changes nothing. SQLite options (`mode=ro`) are the file's own and are left alone.

use std::path::PathBuf;

use hacivat_net::{ClientCertPaths, TlsMode};
use hacivat_proto::DbmsKind;

use super::Profile;

/// The typed setting a folded option went to.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum FoldedTo {
    TlsMode(TlsMode),
    /// The CA file; `None` keeps the platform roots (`sslrootcert=system`).
    CaFile(Option<PathBuf>),
    ClientCert,
    ClientKey,
    /// Seconds.
    ConnectTimeout(u32),
    ApplicationName,
}

/// One option taken out of `extra`, with the setting it went to.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FoldedOption {
    pub key: String,
    pub value: String,
    pub to: FoldedTo,
}

/// The engines whose options are libpq's.
fn is_pg_family(kind: DbmsKind) -> bool {
    matches!(
        kind,
        DbmsKind::Postgres
            | DbmsKind::Redshift
            | DbmsKind::Cockroach
            | DbmsKind::Yugabyte
            | DbmsKind::Timescale
    )
}

/// The engines whose options are JDBC's or the MySQL client's.
fn is_mysql_family(kind: DbmsKind) -> bool {
    matches!(kind, DbmsKind::MySql | DbmsKind::MariaDb | DbmsKind::TiDb)
}

/// An option key folded for matching: ASCII lowercase, without `-` and `_`.
fn fold_key(key: &str) -> String {
    key.trim()
        .chars()
        .filter(|c| *c != '-' && *c != '_')
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

/// A TLS mode value (`verify_full`, `VERIFY_IDENTITY`, `prefer`, ...).
fn tls_mode(value: &str) -> Option<TlsMode> {
    Some(
        match value.trim().to_ascii_lowercase().replace('_', "-").as_str() {
            "disable" | "disabled" => TlsMode::Disable,
            "allow" | "prefer" | "preferred" => TlsMode::Auto,
            "require" | "required" => TlsMode::Require,
            "verify-ca" => TlsMode::VerifyCa,
            "verify-full" | "verify-identity" => TlsMode::VerifyFull,
            _ => return None,
        },
    )
}

/// Moves the options of `p.extra` that have a typed setting into it (see the module docs) and
/// returns them, in their `extra` order. Every entry of a folded key leaves `extra`.
pub fn fold_typed_options(p: &mut Profile) -> Vec<FoldedOption> {
    let pg = is_pg_family(p.engine);
    let mysql = is_mysql_family(p.engine);
    if !pg && !mysql {
        return Vec::new();
    }
    let last = |folded: &str| {
        p.extra
            .iter()
            .rev()
            .find(|(k, _)| fold_key(k) == folded)
            .map(|(k, v)| (k.clone(), v.trim().to_string()))
    };
    // (folded key, the setting it went to)
    let mut taken: Vec<(&'static str, FoldedTo)> = Vec::new();

    match last("sslmode") {
        Some((_, v)) => {
            if let Some(mode) = tls_mode(&v) {
                p.tls.mode = mode;
                taken.push(("sslmode", FoldedTo::TlsMode(mode)));
                if mysql {
                    // `useSSL` only counts without a mode; with one it is overridden.
                    taken.push(("usessl", FoldedTo::TlsMode(mode)));
                }
            }
        }
        None if mysql => {
            let mode = match last("usessl")
                .map(|(_, v)| v.to_ascii_lowercase())
                .as_deref()
            {
                Some("false") => Some(TlsMode::Disable),
                Some("true") => Some(TlsMode::Require),
                _ => None,
            };
            if let Some(mode) = mode {
                p.tls.mode = mode;
                taken.push(("usessl", FoldedTo::TlsMode(mode)));
            }
        }
        None => {}
    }

    let ca_key = if mysql { "sslca" } else { "sslrootcert" };
    if let Some((_, v)) = last(ca_key) {
        let system = pg && v.eq_ignore_ascii_case("system");
        let file = (!system && !v.is_empty()).then(|| PathBuf::from(&v));
        p.tls.ca_file = file.clone();
        taken.push((ca_key, FoldedTo::CaFile(file)));
    }

    let typed = p.tls.client_cert.clone();
    let cert = last("sslcert").filter(|(_, v)| !v.is_empty());
    let key = last("sslkey").filter(|(_, v)| !v.is_empty());
    let cert_path = cert
        .as_ref()
        .map(|(_, v)| PathBuf::from(v))
        .or_else(|| typed.as_ref().map(|c| c.cert.clone()));
    let key_path = key
        .as_ref()
        .map(|(_, v)| PathBuf::from(v))
        .or_else(|| typed.as_ref().map(|c| c.key.clone()));
    if (cert.is_some() || key.is_some())
        && let (Some(cert_path), Some(key_path)) = (cert_path, key_path)
    {
        p.tls.client_cert = Some(ClientCertPaths {
            cert: cert_path,
            key: key_path,
        });
        if cert.is_some() {
            taken.push(("sslcert", FoldedTo::ClientCert));
        }
        if key.is_some() {
            taken.push(("sslkey", FoldedTo::ClientKey));
        }
    }

    if let Some((k, v)) = last("connecttimeout")
        && let Ok(n) = v.parse::<u64>()
        && n > 0
    {
        // JDBC's `connectTimeout` (no separator) counts milliseconds.
        let jdbc_ms = mysql && !k.contains(['_', '-']);
        let seconds = if jdbc_ms { n.div_ceil(1000) } else { n };
        let seconds = u32::try_from(seconds).unwrap_or(u32::MAX);
        p.connect_timeout_s = seconds;
        taken.push(("connecttimeout", FoldedTo::ConnectTimeout(seconds)));
    }

    if let Some((_, v)) = last("applicationname") {
        p.application_name = v;
        taken.push(("applicationname", FoldedTo::ApplicationName));
    }

    let mut folded = Vec::new();
    p.extra.retain(|(k, v)| {
        let f = fold_key(k);
        match taken.iter().find(|(t, _)| *t == f) {
            Some((_, to)) => {
                folded.push(FoldedOption {
                    key: k.clone(),
                    value: v.clone(),
                    to: to.clone(),
                });
                false
            }
            None => true,
        }
    });
    folded
}

/// Drops every option of `p.extra` whose key carries a password (`hacivat_proto::
/// is_secret_param`) and returns how many: a password never travels in `extra`, so it never
/// reaches `connections.toml`, a log or a driver option (HV-SEC-05).
pub fn strip_secret_options(p: &mut Profile) -> usize {
    let before = p.extra.len();
    p.extra.retain(|(k, _)| !hacivat_proto::is_secret_param(k));
    before - p.extra.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with(engine: DbmsKind, extra: &[(&str, &str)]) -> Profile {
        Profile {
            engine,
            extra: extra
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            ..Profile::default()
        }
    }

    fn keys(f: &[FoldedOption]) -> Vec<&str> {
        f.iter().map(|o| o.key.as_str()).collect()
    }

    #[test]
    fn libpq_options_become_typed_settings_and_leave_extra() {
        let mut p = with(
            DbmsKind::Postgres,
            &[
                ("SSLMODE", "verify-full"),
                ("connect_timeout", "7"),
                ("sslrootcert", "/certs/ca.pem"),
                ("application_name", "etl"),
                ("options", "-c search_path=x"),
                ("foo", "bar"),
            ],
        );
        let folded = fold_typed_options(&mut p);
        assert_eq!(
            keys(&folded),
            [
                "SSLMODE",
                "connect_timeout",
                "sslrootcert",
                "application_name"
            ]
        );
        assert_eq!(
            (p.tls.mode, p.connect_timeout_s, p.application_name.as_str()),
            (TlsMode::VerifyFull, 7, "etl")
        );
        assert_eq!(p.tls.ca_file, Some(PathBuf::from("/certs/ca.pem")));
        assert_eq!(
            p.extra,
            vec![
                ("options".to_string(), "-c search_path=x".to_string()),
                ("foo".to_string(), "bar".to_string())
            ]
        );
        // A second fold finds nothing.
        assert!(fold_typed_options(&mut p).is_empty());
    }

    #[test]
    fn the_last_value_wins_and_an_unknown_value_stays_an_extra() {
        let mut p = with(
            DbmsKind::Postgres,
            &[("sslmode", "require"), ("sslmode", "disable")],
        );
        fold_typed_options(&mut p);
        assert_eq!(p.tls.mode, TlsMode::Disable);
        assert!(p.extra.is_empty());

        let mut p = with(DbmsKind::Postgres, &[("sslmode", "bogus")]);
        p.tls.mode = TlsMode::VerifyFull;
        assert!(fold_typed_options(&mut p).is_empty());
        assert_eq!(p.tls.mode, TlsMode::VerifyFull);
        assert_eq!(p.extra.len(), 1);

        let mut p = with(DbmsKind::Postgres, &[("sslrootcert", "system")]);
        p.tls.ca_file = Some("/old.pem".into());
        fold_typed_options(&mut p);
        assert_eq!(p.tls.ca_file, None);
        let mut p = with(DbmsKind::Postgres, &[("connect_timeout", "0")]);
        assert!(fold_typed_options(&mut p).is_empty());
        assert_eq!(p.connect_timeout_s, 15);
    }

    #[test]
    fn jdbc_and_mysql_client_options_fold_too() {
        let mut my = with(
            DbmsKind::MySql,
            &[("sslMode", "verify_ca"), ("connectTimeout", "2500")],
        );
        fold_typed_options(&mut my);
        assert_eq!((my.tls.mode, my.connect_timeout_s), (TlsMode::VerifyCa, 3));
        let mut my = with(DbmsKind::MySql, &[("useSSL", "false")]);
        fold_typed_options(&mut my);
        assert_eq!(my.tls.mode, TlsMode::Disable);
        let mut my = with(
            DbmsKind::MariaDb,
            &[("useSSL", "false"), ("ssl-mode", "REQUIRED")],
        );
        fold_typed_options(&mut my);
        assert_eq!(my.tls.mode, TlsMode::Require);
        assert!(my.extra.is_empty(), "useSSL is overridden by the mode");

        for (ca, cert, key) in [
            ("sslCa", "sslCert", "sslKey"),
            ("ssl-ca", "ssl-cert", "ssl-key"),
        ] {
            let mut my = with(
                DbmsKind::MySql,
                &[
                    (ca, "/c/ca.pem"),
                    (cert, "/c/me.pem"),
                    (key, "/c/me.key"),
                    ("connect_timeout", "4"),
                ],
            );
            fold_typed_options(&mut my);
            assert_eq!(my.tls.ca_file, Some(PathBuf::from("/c/ca.pem")), "{ca}");
            assert_eq!(
                my.tls.client_cert,
                Some(ClientCertPaths {
                    cert: "/c/me.pem".into(),
                    key: "/c/me.key".into()
                })
            );
            assert_eq!(my.connect_timeout_s, 4, "seconds, not JDBC milliseconds");
            assert!(my.extra.is_empty());
        }
    }

    #[test]
    fn half_a_client_certificate_folds_only_onto_the_other_half() {
        let mut p = with(DbmsKind::Postgres, &[("sslcert", "/c/me.pem")]);
        assert!(fold_typed_options(&mut p).is_empty());
        assert_eq!(p.tls.client_cert, None);
        p.tls.client_cert = Some(ClientCertPaths {
            cert: "/old.pem".into(),
            key: "/old.key".into(),
        });
        assert_eq!(keys(&fold_typed_options(&mut p)), ["sslcert"]);
        assert_eq!(
            p.tls.client_cert,
            Some(ClientCertPaths {
                cert: "/c/me.pem".into(),
                key: "/old.key".into()
            })
        );
    }

    #[test]
    fn sqlite_options_are_left_alone() {
        let mut p = with(DbmsKind::Sqlite, &[("mode", "ro"), ("sslmode", "disable")]);
        assert!(fold_typed_options(&mut p).is_empty());
        assert_eq!(p.extra.len(), 2);
    }

    #[test]
    fn password_options_are_stripped() {
        let mut p = with(
            DbmsKind::Postgres,
            &[
                ("pgpassword", "hv_test_pw"),
                ("db_password", "hv_test_pw"),
                ("password_db", "kept"),
            ],
        );
        assert_eq!(strip_secret_options(&mut p), 2);
        assert_eq!(p.extra, vec![("password_db".into(), "kept".into())]);
    }
}
