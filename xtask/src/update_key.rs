//! `xtask check-update-key <secret-key-file>`: refuse a minisign secret key that is not
//! password-protected (L13: the update key lives on the PO's Mac and is always passworded).
//!
//! A minisign secret key file is a comment line followed by one base64 line. The decoded bytes
//! start with the signature algorithm (2 bytes, `Ed`) and the KDF algorithm (2 bytes): `Sc`
//! (scrypt) when passworded, `[0, 0]` when the key was generated with `-W`.

use std::process::ExitCode;

use anyhow::{Context, Result, bail};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;

/// Whether the minisign secret key in `sk_file` is encrypted. Errors when the file is not a
/// minisign secret key at all, so a typo can never read as "unencrypted but fine".
pub fn key_is_encrypted(sk_file: &str) -> Result<bool> {
    let text = std::fs::read_to_string(sk_file).with_context(|| format!("reading {sk_file}"))?;
    let line = text
        .lines()
        .nth(1)
        .context("not a minisign secret key: missing key line")?;
    let bytes = STANDARD
        .decode(line.trim())
        .context("not a minisign secret key: key line is not base64")?;
    if bytes.len() < 4 {
        bail!("not a minisign secret key: key line is too short");
    }
    match &bytes[2..4] {
        b"Sc" => Ok(true),
        [0, 0] => Ok(false),
        other => bail!("unknown minisign KDF algorithm {other:?}"),
    }
}

pub fn run() -> ExitCode {
    let Some(path) = std::env::args().nth(2) else {
        eprintln!("usage: xtask check-update-key <secret-key-file>");
        return ExitCode::from(2);
    };
    match key_is_encrypted(&path) {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => {
            eprintln!(
                "{path} is an unencrypted minisign key; regenerate it with a password \
                 (minisign -G, without -W). Refusing to sign."
            );
            ExitCode::FAILURE
        }
        Err(err) => {
            eprintln!("check-update-key: {err:#}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    /// Writes a minisign-shaped secret key file with the given algorithm bytes and returns its path.
    fn sk_fixture(sig_alg: [u8; 2], kdf_alg: [u8; 2]) -> String {
        static N: AtomicU32 = AtomicU32::new(0);
        let mut raw = Vec::new();
        raw.extend_from_slice(&sig_alg);
        raw.extend_from_slice(&kdf_alg);
        raw.extend_from_slice(b"B2");
        raw.extend_from_slice(&[7u8; 32 + 8 + 8 + 104]);
        let path = std::env::temp_dir().join(format!(
            "hacivat-update-key-{}-{}.key",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        let body = format!("untrusted comment: fixture\n{}\n", STANDARD.encode(raw));
        std::fs::write(&path, body).unwrap();
        path.to_string_lossy().into_owned()
    }

    #[test]
    fn unencrypted_key_is_refused() {
        assert!(!key_is_encrypted(&sk_fixture(*b"Ed", [0, 0])).unwrap());
    }

    #[test]
    fn scrypt_key_is_accepted() {
        assert!(key_is_encrypted(&sk_fixture(*b"Ed", *b"Sc")).unwrap());
    }

    #[test]
    fn garbage_is_an_error() {
        let path = std::env::temp_dir().join(format!("hacivat-garbage-{}.key", std::process::id()));
        std::fs::write(&path, "untrusted comment: x\n!!!\n").unwrap();
        assert!(key_is_encrypted(&path.to_string_lossy()).is_err());
    }

    #[test]
    fn missing_file_is_an_error() {
        assert!(key_is_encrypted("/nonexistent/hacivat/update.key").is_err());
    }
}
