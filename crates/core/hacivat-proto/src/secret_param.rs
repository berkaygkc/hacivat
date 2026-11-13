//! The one rule for option keys that carry a password (HV-SEC-05).
//!
//! The URL parser, the renderer, the `pg_service.conf` import, the connection dialog's extra
//! properties, the `connections.toml` store and the scrubber all ask [`is_secret_param`], so a
//! key one of them treats as a password is a password for all of them. It lives here because
//! every one of those crates already depends on this one.

/// Extra-parameter keys that carry a password, compared ASCII-case-insensitively.
const SECRET_PARAMS: [&str; 8] = [
    "password",
    "sslpassword",
    "pass",
    "pwd",
    "passwd",
    "pgpassword",
    "mysql_pwd",
    // The suffix rule needs a separator, so it misses this run-together name.
    "sslkeypassword",
];

/// Key endings that make a key a password key (`db_password`, `x-pwd`).
const SECRET_SUFFIXES: [&str; 4] = ["_password", "-password", "_pwd", "-pwd"];

/// Whether the option `key` carries a password. Leading dashes are ignored (`--password`), and
/// so is ASCII case. A key that only starts with a password word (`password_db`) does not
/// count; one that ends with it (`x_password`) does.
pub fn is_secret_param(key: &str) -> bool {
    let key = key.trim_start_matches('-');
    SECRET_PARAMS.iter().any(|s| s.eq_ignore_ascii_case(key))
        || SECRET_SUFFIXES.iter().any(|s| {
            key.len() >= s.len()
                && key.is_char_boundary(key.len() - s.len())
                && key[key.len() - s.len()..].eq_ignore_ascii_case(s)
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn password_keys_and_suffixes_match_other_keys_do_not() {
        for key in [
            "password",
            "PASSWORD",
            "--password",
            "sslpassword",
            "SSLKeyPassword",
            "pgpassword",
            "mysql_pwd",
            "db_password",
            "x-pwd",
        ] {
            assert!(is_secret_param(key), "{key}");
        }
        for key in [
            "password_db",
            "sslmode",
            "user",
            "passphrase_hint",
            "",
            "ı_pwd_",
        ] {
            assert!(!is_secret_param(key), "{key}");
        }
    }
}
