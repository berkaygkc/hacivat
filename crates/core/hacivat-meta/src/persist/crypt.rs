//! Row encryption of the catalog cache (R-M4-15): XChaCha20-Poly1305 under the profile's
//! [`RowKey`], with a domain string and the row's associated data (its namespace key).
//!
//! M4 Task 1a seam until Task 11: [`RowCipher::seal`] seals nothing (it answers an empty
//! vector, never the plaintext) and [`RowCipher::open`] refuses. The cache is memory-only
//! meanwhile, so nothing reaches disk.

use std::fmt;

use super::RowKey;

/// Why a sealed row cannot be opened.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CryptError {
    /// Shorter than a nonce and a tag.
    Truncated,
    /// Wrong key, wrong associated data, or tampered bytes.
    Auth,
    /// Not written yet (M4 Task 1a stub).
    NotImplemented,
}

impl fmt::Display for CryptError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CryptError::Truncated => write!(f, "sealed row too short"),
            CryptError::Auth => write!(f, "sealed row failed authentication"),
            CryptError::NotImplemented => write!(f, "not implemented"),
        }
    }
}

impl std::error::Error for CryptError {}

/// A row cipher bound to one key and one domain (`hv-catalog-v1`).
pub struct RowCipher {
    key: RowKey,
    domain: &'static str,
}

impl fmt::Debug for RowCipher {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RowCipher")
            .field("key", &self.key)
            .field("domain", &self.domain)
            .finish()
    }
}

impl RowCipher {
    pub fn new(key: RowKey, domain: &'static str) -> Self {
        RowCipher { key, domain }
    }

    /// `plaintext` sealed with a fresh nonce under `ad`.
    ///
    /// Task 1a stub: an empty vector (nothing sealed; the plaintext is never returned).
    pub fn seal(&self, ad: &[u8], plaintext: &[u8]) -> Vec<u8> {
        let _ = (ad, plaintext);
        Vec::new()
    }

    /// The plaintext of `sealed` under `ad`.
    ///
    /// Task 1a stub: `Err(CryptError::NotImplemented)`.
    pub fn open(&self, ad: &[u8], sealed: &[u8]) -> Result<Vec<u8>, CryptError> {
        let _ = (ad, sealed);
        Err(CryptError::NotImplemented)
    }
}
