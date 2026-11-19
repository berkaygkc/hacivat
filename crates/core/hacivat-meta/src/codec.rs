//! The versioned binary encoding of a namespace, the cache row payload (HV-MODEL-03; R-M4-26):
//! `"HVNS" major minor compression checksum body`, every record and union value
//! length-prefixed, so older minors read newer rows. Rows are uncompressed in v1 (the
//! compression byte is reserved).
//!
//! M4 Task 1a seam (spike origin `d86313b`, declarations only): encoding and decoding refuse
//! until Task 11; the cache is memory-only meanwhile.

use std::fmt;

use crate::namespace::NamespaceSnapshot;

/// The format's major version: a reader refuses another major (the cache is rebuilt).
pub const FORMAT_MAJOR: u16 = 1;
/// The format's minor version: a reader reads newer minors and never rewrites them.
pub const FORMAT_MINOR: u16 = 0;

/// Why a payload cannot be encoded or decoded. Decoding never panics on any input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecodeError {
    /// Shorter than its header or a length prefix says.
    Truncated,
    /// Not a namespace payload.
    BadMagic,
    /// Another major format.
    Major { found: u16 },
    /// The body's checksum does not match.
    Checksum,
    /// A reserved compression byte this build does not know.
    Compression(u8),
    /// A malformed record.
    Malformed(&'static str),
    /// Not written yet (M4 Task 1a stub).
    NotImplemented,
}

impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DecodeError::Truncated => write!(f, "truncated payload"),
            DecodeError::BadMagic => write!(f, "not a namespace payload"),
            DecodeError::Major { found } => write!(f, "format major {found}"),
            DecodeError::Checksum => write!(f, "checksum mismatch"),
            DecodeError::Compression(c) => write!(f, "unknown compression {c}"),
            DecodeError::Malformed(why) => write!(f, "malformed payload: {why}"),
            DecodeError::NotImplemented => write!(f, "not implemented"),
        }
    }
}

impl std::error::Error for DecodeError {}

/// The payload of `ns`. Task 1a stub: `Err(DecodeError::NotImplemented)`.
pub fn encode_namespace(ns: &NamespaceSnapshot) -> Result<Vec<u8>, DecodeError> {
    let _ = ns;
    Err(DecodeError::NotImplemented)
}

/// The namespace in `bytes`. Task 1a stub: `Err(DecodeError::NotImplemented)`.
pub fn decode_namespace(bytes: &[u8]) -> Result<NamespaceSnapshot, DecodeError> {
    let _ = bytes;
    Err(DecodeError::NotImplemented)
}

/// The `(major, minor)` of a payload, if its header is readable. Task 1a stub: `None`.
pub fn payload_version(bytes: &[u8]) -> Option<(u16, u16)> {
    let _ = bytes;
    None
}
