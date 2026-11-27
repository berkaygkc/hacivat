//! Decoding and encoding script bytes: BOM sniff, strict UTF-8, legacy encodings through
//! `encoding_rs` (UI-EDIT-02; HV-SCRIPT-07 step 1). No `encoding_rs` type is public. Task 5
//! writes it.
//!
//! ## m4b-t01a-seams: declared by Task 1a so later tasks add no `lib.rs` line.
//!
//! ## m4b-t05-fileio
//! Task 5: every conversion is exact. Decoding never substitutes (`*_without_replacement`), so
//! a byte sequence the encoding cannot decode is an error, not a U+FFFD; encoding never
//! substitutes either, so a character the encoding cannot hold is reported with its offset.
//! UTF-16 is encoded here by hand, because `encoding_rs` encodes UTF-16 as UTF-8 (its output
//! encoding). Everything is `pub(crate)`: `BackendInPublicApi` keeps `encoding_rs` out of the
//! public API.

use encoding_rs::{DecoderResult, EncoderResult, Encoding, REPLACEMENT, UTF_8, UTF_16BE, UTF_16LE};

use crate::fileio::TextEncoding;

pub(crate) const UTF8_BOM: &[u8] = &[0xEF, 0xBB, 0xBF];
pub(crate) const UTF16LE_BOM: &[u8] = &[0xFF, 0xFE];
pub(crate) const UTF16BE_BOM: &[u8] = &[0xFE, 0xFF];

pub(crate) const UTF_16LE_ENC: TextEncoding = TextEncoding("UTF-16LE");
pub(crate) const UTF_16BE_ENC: TextEncoding = TextEncoding("UTF-16BE");

/// The encoding a BOM at the start of `bytes` names, and the BOM's length (UTF-8, UTF-16LE,
/// UTF-16BE; the only BOMs UI-EDIT-02 step 1 sniffs).
pub(crate) fn sniff_bom(bytes: &[u8]) -> Option<(TextEncoding, usize)> {
    if bytes.starts_with(UTF8_BOM) {
        Some((TextEncoding::UTF_8, UTF8_BOM.len()))
    } else if bytes.starts_with(UTF16LE_BOM) {
        Some((UTF_16LE_ENC, UTF16LE_BOM.len()))
    } else if bytes.starts_with(UTF16BE_BOM) {
        Some((UTF_16BE_ENC, UTF16BE_BOM.len()))
    } else {
        None
    }
}

/// The BOM `enc` writes when the file had one (none for a legacy encoding).
pub(crate) fn bom_of(enc: TextEncoding) -> &'static [u8] {
    match backend(enc) {
        Some(e) if e == UTF_8 => UTF8_BOM,
        Some(e) if e == UTF_16LE => UTF16LE_BOM,
        Some(e) if e == UTF_16BE => UTF16BE_BOM,
        _ => &[],
    }
}

/// The encoding a user-chosen WHATWG label names (`"windows-1254"`, `"ISO-8859-9"`, which is
/// the same encoding, `"Shift_JIS"`, …). `replacement`, which decodes every input to U+FFFD,
/// is never offered.
pub(crate) fn lookup(label: &str) -> Option<TextEncoding> {
    let e = Encoding::for_label(label.trim().as_bytes())?;
    (e != REPLACEMENT).then(|| TextEncoding(e.name()))
}

/// The `encoding_rs` encoding of a [`TextEncoding`] (always one of its own names).
fn backend(enc: TextEncoding) -> Option<&'static Encoding> {
    Encoding::for_label(enc.name().as_bytes())
        .filter(|e| e.name() == enc.name() && *e != REPLACEMENT)
}

/// `bytes` (without a BOM) decoded exactly, or `None` when they do not decode.
pub(crate) fn decode_exact(enc: TextEncoding, bytes: &[u8]) -> Option<String> {
    let e = backend(enc)?;
    if e == UTF_8 {
        return std::str::from_utf8(bytes).ok().map(str::to_owned);
    }
    e.decode_without_bom_handling_and_without_replacement(bytes)
        .map(std::borrow::Cow::into_owned)
}

/// `text` encoded in `enc`, with its BOM when `bom`; `Err(offset)` names the byte offset in
/// `text` of the first character `enc` cannot hold.
pub(crate) fn encode_exact(enc: TextEncoding, bom: bool, text: &str) -> Result<Vec<u8>, usize> {
    let e = backend(enc).ok_or(0usize)?;
    let mut out = Vec::new();
    if bom {
        out.extend_from_slice(bom_of(enc));
    }
    if e == UTF_8 {
        out.extend_from_slice(text.as_bytes());
        return Ok(out);
    }
    if e == UTF_16LE || e == UTF_16BE {
        let le = e == UTF_16LE;
        out.reserve(text.len() * 2);
        for unit in text.encode_utf16() {
            out.extend_from_slice(&if le {
                unit.to_le_bytes()
            } else {
                unit.to_be_bytes()
            });
        }
        return Ok(out);
    }
    let mut encoder = e.new_encoder();
    let mut src = text;
    let mut consumed = 0usize;
    let mut buf = vec![0u8; 64 << 10];
    loop {
        let (result, read, written) =
            encoder.encode_from_utf8_without_replacement(src, &mut buf, true);
        out.extend_from_slice(&buf[..written]);
        consumed += read;
        src = &src[read..];
        match result {
            EncoderResult::InputEmpty => return Ok(out),
            EncoderResult::OutputFull => {}
            EncoderResult::Unmappable(c) => return Err(consumed - c.len_utf8()),
        }
    }
}

/// An incremental exact decoder for "Run as script" (HV-SCRIPT-12): it removes the encoding's
/// own BOM at the start and keeps a character split across two reads for the next one.
pub(crate) struct StreamDecoder {
    inner: encoding_rs::Decoder,
}

impl StreamDecoder {
    pub(crate) fn new(enc: TextEncoding) -> Option<Self> {
        Some(StreamDecoder {
            inner: backend(enc)?.new_decoder_with_bom_removal(),
        })
    }

    /// The text of `src` (plus what an earlier call kept back); `None` when the bytes do not
    /// decode. With `last`, a character left incomplete at the end is an error too.
    pub(crate) fn feed(&mut self, mut src: &[u8], last: bool) -> Option<String> {
        let mut out = String::new();
        loop {
            let need = self
                .inner
                .max_utf8_buffer_length_without_replacement(src.len())?;
            out.reserve(need);
            let (result, read) = self
                .inner
                .decode_to_string_without_replacement(src, &mut out, last);
            src = &src[read..];
            match result {
                DecoderResult::InputEmpty => return Some(out),
                DecoderResult::OutputFull => {}
                DecoderResult::Malformed(_, _) => return None,
            }
        }
    }
}
// ## m4b-t05-fileio
