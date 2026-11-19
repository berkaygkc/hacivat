//! Row encryption (R-M4-15): XChaCha20-Poly1305 under a [`RowKey`], with a domain string and
//! per-row associated data. Reusable beyond the catalog cache (M7); the catalog cache uses
//! domain [`CATALOG_DOMAIN`] with `ad = row type ‖ ns_key ‖ minor`, so a row cannot move
//! between namespaces, row types or stores (another store has another key).
//!
//! M4 Task 11 (spike `d86313b` had no encryption; this module is new code).
//!
//! - **Sealed form:** `nonce (24) ‖ ciphertext ‖ tag (16)`. Every [`RowCipher::seal`] draws a
//!   fresh 24-byte nonce from the OS random source; nothing is ever sealed twice under one nonce
//!   (a 192-bit random nonce makes a repeat negligible, which is why the X variant is used).
//!   A random source that fails is an error, never a fallback nonce.
//! - **Associated data:** `len(domain) as u16le ‖ domain ‖ ad`: the length prefix keeps two
//!   domains from ever producing the same associated data.
//! - **Subkeys:** the profile key itself seals nothing. Two subkeys are derived from it per
//!   domain, one for sealing and one for lookup tags, as XChaCha20 keystream blocks under fixed
//!   labels (the stream cipher used as a PRF; [`derive`]). The subkeys and every intermediate
//!   buffer that holds key material are wiped on drop (`chacha20poly1305` zeroizes its key;
//!   the derived bytes live in `Zeroizing`).
//! - **Lookup tags** ([`RowCipher::tag`]): `PRF(lookup key, XXH3-128(input))`, so an encrypted
//!   store can index rows by namespace and sources by hash without writing a name or a content
//!   hash in the clear. XXH3 is not collision-resistant: crafted inputs can share a tag, which
//!   makes their rows collide (one replaces the other; the associated data still binds each
//!   payload to its full key, so a collided row fails authentication and is re-introspected).
//!   It reveals nothing about the input without the key.
//! - `Debug` never shows key bytes; there is no `PartialEq`.

use std::fmt;

use chacha20poly1305::aead::rand_core::RngCore;
use chacha20poly1305::aead::{Aead, KeyInit, OsRng, Payload};
use chacha20poly1305::{Key, XChaCha20Poly1305, XNonce};
use zeroize::Zeroizing;

use super::RowKey;

/// The domain of the catalog cache's rows (R-M4-15).
pub const CATALOG_DOMAIN: &str = "hv-catalog-v1";
/// Nonce bytes at the start of a sealed row.
pub const NONCE_LEN: usize = 24;
/// Authentication tag bytes at the end of a sealed row.
pub const TAG_LEN: usize = 16;
/// Bytes a sealed row adds to its plaintext.
pub const OVERHEAD: usize = NONCE_LEN + TAG_LEN;

/// Why a row cannot be sealed or opened.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CryptError {
    /// Shorter than a nonce and a tag.
    Truncated,
    /// Wrong key, wrong associated data, or tampered bytes.
    Auth,
    /// The OS random source failed (no nonce, so nothing was sealed).
    Rng,
    /// A plaintext longer than the cipher accepts.
    TooLong,
}

impl fmt::Display for CryptError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CryptError::Truncated => write!(f, "sealed row too short"),
            CryptError::Auth => write!(f, "sealed row failed authentication"),
            CryptError::Rng => write!(f, "the random source failed"),
            CryptError::TooLong => write!(f, "row too long to seal"),
        }
    }
}

impl std::error::Error for CryptError {}

/// The label of a derived subkey: purpose byte, then the domain's XXH3-128, zero-padded to a
/// nonce. The domain is a constant of this program, never user input.
fn label(purpose: u8, domain: &str) -> [u8; NONCE_LEN] {
    let mut n = [0u8; NONCE_LEN];
    n[0] = purpose;
    n[1..17].copy_from_slice(&twox_hash::XxHash3_128::oneshot(domain.as_bytes()).to_le_bytes());
    n[17..].copy_from_slice(b"hv-kdf\x01");
    n
}

const PURPOSE_SEAL: u8 = 1;
const PURPOSE_LOOKUP: u8 = 2;

/// 32 bytes of XChaCha20 keystream under `key` at `nonce` (the cipher as a PRF of the nonce):
/// the encryption of zeros, tag dropped. Every buffer that held the output is wiped.
fn prf32(key: &XChaCha20Poly1305, nonce: &[u8; NONCE_LEN]) -> Zeroizing<[u8; 32]> {
    let mut out = Zeroizing::new([0u8; 32]);
    // Encrypting 32 bytes cannot fail (far below the cipher's length limit).
    if let Ok(ct) = key.encrypt(XNonce::from_slice(nonce), [0u8; 32].as_slice()) {
        let ct = Zeroizing::new(ct);
        out.copy_from_slice(&ct[..32]);
    }
    out
}

/// A subkey of `master` for `purpose` in `domain`.
fn derive(master: &RowKey, purpose: u8, domain: &str) -> XChaCha20Poly1305 {
    let root = XChaCha20Poly1305::new(Key::from_slice(&master.0));
    let sub = prf32(&root, &label(purpose, domain));
    XChaCha20Poly1305::new(Key::from_slice(sub.as_slice()))
}

/// A row cipher bound to one key and one domain.
pub struct RowCipher {
    seal: XChaCha20Poly1305,
    lookup: XChaCha20Poly1305,
    domain: &'static str,
}

impl fmt::Debug for RowCipher {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RowCipher")
            .field("key", &"<redacted>")
            .field("domain", &self.domain)
            .finish()
    }
}

impl RowCipher {
    /// A cipher for `domain` under `key`. The key is consumed (and wiped on drop); only the
    /// derived subkeys are kept.
    pub fn new(key: RowKey, domain: &'static str) -> Self {
        RowCipher {
            seal: derive(&key, PURPOSE_SEAL, domain),
            lookup: derive(&key, PURPOSE_LOOKUP, domain),
            domain,
        }
    }

    pub fn domain(&self) -> &'static str {
        self.domain
    }

    /// The full associated data: `len(domain) as u16le ‖ domain ‖ ad`.
    fn aad(&self, ad: &[u8]) -> Vec<u8> {
        let d = self.domain.as_bytes();
        let mut out = Vec::with_capacity(2 + d.len() + ad.len());
        out.extend_from_slice(&u16::try_from(d.len()).unwrap_or(u16::MAX).to_le_bytes());
        out.extend_from_slice(d);
        out.extend_from_slice(ad);
        out
    }

    /// `plaintext` sealed under `ad` with a fresh random nonce: `nonce ‖ ciphertext ‖ tag`.
    pub fn seal(&self, ad: &[u8], plaintext: &[u8]) -> Result<Vec<u8>, CryptError> {
        let mut nonce = [0u8; NONCE_LEN];
        OsRng
            .try_fill_bytes(&mut nonce)
            .map_err(|_| CryptError::Rng)?;
        let aad = self.aad(ad);
        let ct = self
            .seal
            .encrypt(
                XNonce::from_slice(&nonce),
                Payload {
                    msg: plaintext,
                    aad: &aad,
                },
            )
            .map_err(|_| CryptError::TooLong)?;
        let mut out = Vec::with_capacity(NONCE_LEN + ct.len());
        out.extend_from_slice(&nonce);
        out.extend_from_slice(&ct);
        Ok(out)
    }

    /// The plaintext of `sealed` under `ad`. Any tampering, a wrong key, a wrong domain or wrong
    /// associated data is [`CryptError::Auth`]; never a panic.
    pub fn open(&self, ad: &[u8], sealed: &[u8]) -> Result<Vec<u8>, CryptError> {
        if sealed.len() < OVERHEAD {
            return Err(CryptError::Truncated);
        }
        let (nonce, ct) = sealed.split_at(NONCE_LEN);
        let aad = self.aad(ad);
        self.seal
            .decrypt(XNonce::from_slice(nonce), Payload { msg: ct, aad: &aad })
            .map_err(|_| CryptError::Auth)
    }

    /// A keyed 16-byte lookup tag of `input` (see the module docs): equal inputs give equal
    /// tags; without the key a tag reveals nothing about its input.
    pub fn tag(&self, input: &[u8]) -> [u8; 16] {
        let mut nonce = [0u8; NONCE_LEN];
        nonce[..16].copy_from_slice(&twox_hash::XxHash3_128::oneshot(input).to_le_bytes());
        nonce[16..].copy_from_slice(b"hv-tag\x01\x00");
        let block = prf32(&self.lookup, &nonce);
        let mut out = [0u8; 16];
        out.copy_from_slice(&block[..16]);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cipher(byte: u8) -> RowCipher {
        RowCipher::new(RowKey::from_bytes([byte; 32]), CATALOG_DOMAIN)
    }

    #[test]
    fn seal_open_round_trips_and_nonces_differ() {
        let c = cipher(1);
        let a = c.seal(b"ad", b"payload").unwrap();
        let b = c.seal(b"ad", b"payload").unwrap();
        assert_ne!(a[..NONCE_LEN], b[..NONCE_LEN]);
        assert_ne!(a, b);
        assert_eq!(a.len(), b"payload".len() + OVERHEAD);
        assert_eq!(c.open(b"ad", &a).unwrap(), b"payload");
        assert_eq!(c.open(b"ad", &b).unwrap(), b"payload");
    }

    #[test]
    fn subkeys_differ_from_the_master_and_from_each_other() {
        // A row sealed directly under the master key (no derivation) does not open.
        let master = XChaCha20Poly1305::new(Key::from_slice(&[3u8; 32]));
        let nonce = [9u8; NONCE_LEN];
        let c = cipher(3);
        let aad = c.aad(b"x");
        let ct = master
            .encrypt(
                XNonce::from_slice(&nonce),
                Payload {
                    msg: b"m".as_slice(),
                    aad: &aad,
                },
            )
            .unwrap();
        let mut sealed = nonce.to_vec();
        sealed.extend_from_slice(&ct);
        assert_eq!(c.open(b"x", &sealed), Err(CryptError::Auth));
        // The lookup subkey is not the sealing subkey.
        let a = prf32(&c.seal, &[0; NONCE_LEN]);
        let b = prf32(&c.lookup, &[0; NONCE_LEN]);
        assert_ne!(*a, *b);
    }

    #[test]
    fn tags_are_deterministic_keyed_and_domain_separated() {
        let c = cipher(5);
        assert_eq!(c.tag(b"public"), c.tag(b"public"));
        assert_ne!(c.tag(b"public"), c.tag(b"sales"));
        assert_ne!(c.tag(b"public"), cipher(6).tag(b"public"));
        let other = RowCipher::new(RowKey::from_bytes([5; 32]), "hv-other-v1");
        assert_ne!(c.tag(b"public"), other.tag(b"public"));
    }
}
