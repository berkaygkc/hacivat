//! The one rule for option keys that carry a password (HV-SEC-05).
//!
//! The URL parser, the renderer, the `pg_service.conf` import, the connection dialog's extra
//! properties, the `connections.toml` store and the scrubber all ask [`is_secret_param`], so a
//! key one of them treats as a password is a password for all of them. It lives here because
//! every one of those crates already depends on this one.

/// Words that make a key a password key wherever they appear in its normalized form.
const SECRET_WORDS: [&str; 3] = ["password", "passphrase", "passwd"];

/// Known driver properties that mention a password but never hold one (normalized form;
/// M2a re-review R2-1): Connector/J `passwordCharacterEncoding` and
/// `disconnectOnExpiredPasswords`, pgjdbc `sslpasswordcallback` (a class name). An exact match
/// only: anything longer, and every other key that mentions a password (PostgreSQL's
/// `password_encryption` too), stays refused.
const NON_SECRET_KEYS: [&str; 3] = [
    "passwordcharacterencoding",
    "disconnectonexpiredpasswords",
    "sslpasswordcallback",
];

/// Whether the option `key` carries a password. The key is normalized first: surrounding
/// whitespace and leading dashes (`--password`) dropped, ASCII lowercased, and the separators
/// `_`, `-` and `.` removed (`db_password`, `ssl.key-password`, JDBC `keyStorePassword` all
/// become one shape). It is a password key when that form:
/// - contains `password`, `passphrase` or `passwd` (`trustCertificateKeyStorePassword`,
///   `password1`, `tls_passphrase`, `sslkeypassword`, `password_db`); or
/// - ends with `pwd` (`pwd`, `mysql_pwd`, `x-pwd`); or
/// - is `pass`.
///
/// It errs on the side of refusing: a key that merely mentions a password is kept out of every
/// file, log and driver option too, except the few known properties in `NON_SECRET_KEYS`.
pub fn is_secret_param(key: &str) -> bool {
    let normalized: String = key
        .trim()
        .trim_start_matches('-')
        .chars()
        .filter(|c| !matches!(c, '_' | '-' | '.'))
        .map(|c| c.to_ascii_lowercase())
        .collect();
    if NON_SECRET_KEYS.contains(&normalized.as_str()) {
        return false;
    }
    SECRET_WORDS.iter().any(|w| normalized.contains(w))
        || normalized.ends_with("pwd")
        || normalized == "pass"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn password_keys_in_every_spelling_match_other_keys_do_not() {
        for key in [
            "password",
            "PASSWORD",
            "--password",
            " password",
            "password\t",
            "pass",
            "pwd",
            "passwd",
            "sslpassword",
            "SSLKeyPassword",
            "pgpassword",
            "mysql_pwd",
            "db_password",
            "x-pwd",
            "x-password",
            // M2a final-fix round 2 (N1): camelCase JDBC keys and passphrases.
            "trustCertificateKeyStorePassword",
            "clientCertificateKeyStorePassword",
            "keyStorePassword",
            "trustStorePassword",
            "keyPassword",
            "password1",
            "password2",
            "password3",
            "passphrase",
            "ssl_passphrase",
            "tls-passphrase",
            "key.passphrase",
            "passphrase_hint",
            "password_db",
            "ssl.key-password",
            "ı_pwd_",
        ] {
            assert!(is_secret_param(key), "{key:?}");
        }
        for key in [
            "sslmode",
            "user",
            "host",
            "passfile",
            "pass_through",
            "sslkey",
            "application_name",
            "",
        ] {
            assert!(!is_secret_param(key), "{key:?}");
        }
    }

    /// M2a re-review R2-1: known JDBC properties that only mention a password are not
    /// password keys (in any spelling); everything else the contains-rule catches still is.
    #[test]
    fn known_non_secret_properties_are_not_password_keys() {
        for key in [
            "passwordCharacterEncoding",
            "PASSWORDCHARACTERENCODING",
            "password_character_encoding",
            "disconnectOnExpiredPasswords",
            "sslpasswordcallback",
            "sslPasswordCallback",
        ] {
            assert!(!is_secret_param(key), "{key:?}");
        }
        for key in [
            "password_encryption",
            "passwordEncryption",
            "passwordCharacterEncodingX",
            "xpasswordCharacterEncoding",
            "sslpassword",
        ] {
            assert!(is_secret_param(key), "{key:?}");
        }
    }
}
