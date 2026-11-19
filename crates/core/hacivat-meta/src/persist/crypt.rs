//! Row encryption (R-M4-15): XChaCha20-Poly1305 under a [`RowKey`], with a domain string and
//! per-row associated data. Reusable beyond the catalog cache (M7); the catalog cache uses
//! domain [`CATALOG_DOMAIN`] with associated data that starts with a row type, so a row cannot
//! move between namespaces, row types or stores (another store has another key).
//!
//! M4 Task 11 (the spike had no encryption; this module is new code). Fix round 1, ruling
//! R-W2-7: subkeys and lookup tags are HMAC-SHA256.
//!
//! - **Sealed form:** `nonce (24) ‖ ciphertext ‖ tag (16)`. Every [`RowCipher::seal`] draws a
//!   fresh 24-byte nonce from the OS random source; nothing is ever sealed twice under one nonce
//!   (a 192-bit random nonce makes a repeat negligible, which is why the X variant is used).
//!   A random source that fails is an error, never a fallback nonce.
//! - **Associated data:** `len(domain) as u16le ‖ domain ‖ ad`: the length prefix keeps two
//!   domains from ever producing the same associated data.
//! - **Subkeys** (HKDF-expand style): the profile key itself seals nothing. Per domain,
//!   `subkey = HMAC-SHA256(profile key, "hv-rowcipher-v1" ‖ 0 ‖ purpose ‖ len(domain) u16le ‖
//!   domain ‖ 1)`, one for sealing and one for lookup tags. A failure to derive is an error
//!   ([`RowCipher::new`] returns a `Result`), never a fallback key.
//! - **Lookup tags** ([`RowCipher::tag`]): `HMAC-SHA256(lookup subkey, input)[..16]`, so an
//!   encrypted store indexes rows by namespace and sources by hash without writing a name or a
//!   content hash in the clear. A tag reveals nothing about its input without the key, and
//!   crafted inputs cannot be made to collide.
//! - **Wiping:** the subkeys live in `Zeroizing` buffers (`chacha20poly1305` zeroizes its own
//!   key); HMAC output copies are wiped. The HMAC crate's transient inner state is not zeroized
//!   on drop.
//! - `Debug` never shows key bytes; there is no `PartialEq`.

use std::fmt;

use chacha20poly1305::aead::rand_core::RngCore;
use chacha20poly1305::aead::{Aead, KeyInit, OsRng, Payload};
use chacha20poly1305::{Key, XChaCha20Poly1305, XNonce};
use hmac::{Hmac, Mac};
use sha2::Sha256;
use zeroize::{Zeroize, Zeroizing};

use super::RowKey;

/// The domain of the catalog cache's rows (R-M4-15).
pub const CATALOG_DOMAIN: &str = "hv-catalog-v1";
/// Nonce bytes at the start of a sealed row.
pub const NONCE_LEN: usize = 24;
/// Authentication tag bytes at the end of a sealed row.
pub const TAG_LEN: usize = 16;
/// Bytes a sealed row adds to its plaintext.
pub const OVERHEAD: usize = NONCE_LEN + TAG_LEN;

/// Why a row cannot be sealed or opened, or a cipher built.
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
    /// A subkey could not be derived (no cipher is built).
    Key,
}

impl fmt::Display for CryptError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CryptError::Truncated => write!(f, "sealed row too short"),
            CryptError::Auth => write!(f, "sealed row failed authentication"),
            CryptError::Rng => write!(f, "the random source failed"),
            CryptError::TooLong => write!(f, "row too long to seal"),
            CryptError::Key => write!(f, "the row key could not be derived"),
        }
    }
}

impl std::error::Error for CryptError {}

type HmacSha256 = Hmac<Sha256>;

const PURPOSE_SEAL: u8 = 1;
const PURPOSE_LOOKUP: u8 = 2;

/// `HMAC-SHA256(key, parts…)`, the output in a wiped buffer.
fn hmac(key: &[u8], parts: &[&[u8]]) -> Result<Zeroizing<[u8; 32]>, CryptError> {
    let mut mac = <HmacSha256 as Mac>::new_from_slice(key).map_err(|_| CryptError::Key)?;
    for p in parts {
        mac.update(p);
    }
    let mut out = mac.finalize().into_bytes();
    let mut z = Zeroizing::new([0u8; 32]);
    z.copy_from_slice(&out);
    out.as_mut_slice().zeroize();
    Ok(z)
}

/// The subkey of `master` for `purpose` in `domain`.
fn derive(master: &RowKey, purpose: u8, domain: &str) -> Result<Zeroizing<[u8; 32]>, CryptError> {
    let d = domain.as_bytes();
    let len = u16::try_from(d.len()).map_err(|_| CryptError::Key)?;
    hmac(
        &master.0,
        &[
            b"hv-rowcipher-v1",
            &[0, purpose],
            &len.to_le_bytes(),
            d,
            &[1],
        ],
    )
}

/// A row cipher bound to one key and one domain.
pub struct RowCipher {
    seal: XChaCha20Poly1305,
    /// HMAC-SHA256 keyed with the lookup subkey (cloned per tag).
    lookup: HmacSha256,
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
    pub fn new(key: RowKey, domain: &'static str) -> Result<Self, CryptError> {
        let seal = derive(&key, PURPOSE_SEAL, domain)?;
        let lookup = derive(&key, PURPOSE_LOOKUP, domain)?;
        let lookup =
            <HmacSha256 as Mac>::new_from_slice(lookup.as_slice()).map_err(|_| CryptError::Key)?;
        Ok(RowCipher {
            seal: XChaCha20Poly1305::new(Key::from_slice(seal.as_slice())),
            lookup,
            domain,
        })
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

    /// A keyed 16-byte lookup tag of `input`: `HMAC-SHA256(lookup subkey, input)[..16]`. Equal
    /// inputs give equal tags; without the key a tag reveals nothing about its input.
    pub fn tag(&self, input: &[u8]) -> [u8; 16] {
        let full = self
            .lookup
            .clone()
            .chain_update(input)
            .finalize()
            .into_bytes();
        let mut out = [0u8; 16];
        out.copy_from_slice(&full[..16]);
        out
    }
}

/// Pads `data` for sealing: `data ‖ 0x80 ‖ 0x00…` up to a Padmé size of at least `floor`
/// bytes, so a ciphertext length shows only its bucket (at most about 12 % overhead above the
/// floor; Nikitin et al., "Reducing Metadata Leakage from Encrypted Files", PETS 2019).
pub fn pad(data: &[u8], floor: usize) -> Vec<u8> {
    let target = padme((data.len() + 1).max(floor));
    let mut out = Vec::with_capacity(target);
    out.extend_from_slice(data);
    out.push(0x80);
    out.resize(target, 0);
    out
}

/// The data of a [`pad`]ded plaintext, or `None` when the padding is malformed.
pub fn unpad(padded: &[u8]) -> Option<&[u8]> {
    let end = padded.iter().rposition(|&b| b != 0)?;
    (padded[end] == 0x80).then(|| &padded[..end])
}

/// The Padmé length of `len`: `len` rounded up so that only its top ~log2(log2(len)) bits
/// vary.
pub fn padme(len: usize) -> usize {
    if len < 2 {
        return len;
    }
    let e = usize::BITS - 1 - len.leading_zeros(); // floor(log2 len)
    let s = u32::BITS - e.leading_zeros(); // floor(log2 e) + 1
    let last = e.saturating_sub(s);
    let mask = (1usize << last) - 1;
    (len + mask) & !mask
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cipher(byte: u8) -> RowCipher {
        RowCipher::new(RowKey::from_bytes(&[byte; 32]), CATALOG_DOMAIN).unwrap()
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
        let k = RowKey::from_bytes(&[3; 32]);
        let seal = derive(&k, PURPOSE_SEAL, CATALOG_DOMAIN).unwrap();
        let lookup = derive(&k, PURPOSE_LOOKUP, CATALOG_DOMAIN).unwrap();
        assert!(*seal != *lookup && *seal != [3u8; 32]);
        assert!(*derive(&k, PURPOSE_SEAL, "hv-other-v1").unwrap() != *seal);
    }

    #[test]
    fn tags_are_deterministic_keyed_and_domain_separated() {
        let c = cipher(5);
        assert_eq!(c.tag(b"public"), c.tag(b"public"));
        assert_ne!(c.tag(b"public"), c.tag(b"sales"));
        assert_ne!(c.tag(b"public"), cipher(6).tag(b"public"));
        let other = RowCipher::new(RowKey::from_bytes(&[5; 32]), "hv-other-v1").unwrap();
        assert_ne!(c.tag(b"public"), other.tag(b"public"));
    }

    /// RFC 4231 test case 2: the HMAC-SHA256 underneath is the standard one.
    #[test]
    fn hmac_matches_rfc_4231() {
        let out = hmac(b"Jefe", &[b"what do ya want ", b"for nothing?"]).unwrap();
        let expected: [u8; 32] = [
            0x5b, 0xdc, 0xc1, 0x46, 0xbf, 0x60, 0x75, 0x4e, 0x6a, 0x04, 0x24, 0x26, 0x08, 0x95,
            0x75, 0xc7, 0x5a, 0x00, 0x3f, 0x08, 0x9d, 0x27, 0x39, 0x83, 0x9d, 0xec, 0x58, 0xb9,
            0x64, 0xec, 0x38, 0x43,
        ];
        assert!(*out == expected);
    }

    #[test]
    fn padding_round_trips_and_shows_only_the_bucket() {
        for len in [0usize, 1, 15, 255, 256, 4_000, 70_000, 5_000_000] {
            let data = vec![7u8; len];
            let p = pad(&data, 256);
            assert!(p.len() > len && p.len() >= 256);
            assert_eq!(p.len(), padme(p.len()), "{len}");
            assert_eq!(unpad(&p), Some(&data[..]));
            // Overhead above the floor stays under 12 %.
            if len > 256 {
                assert!((p.len() - len) * 100 <= len * 12 + 100, "{len}");
            }
        }
        // Equal buckets for nearby lengths.
        assert_eq!(pad(&[1; 10], 256).len(), pad(&[1; 200], 256).len());
        assert_eq!(unpad(&[1, 2, 0, 0]), None);
        assert_eq!(unpad(&[0, 0]), None);
    }
}
