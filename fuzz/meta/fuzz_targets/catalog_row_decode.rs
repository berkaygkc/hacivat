//! Fuzz target `catalog_row_decode` (M4 Task 11): an untrusted cache row never panics
//! (HV-MODEL-03; Review Focus 4).
//!
//! - The input is decoded as a whole payload, and again framed behind a valid header and a
//!   recomputed checksum (the first byte picks this build's minor or a newer one), so the body
//!   decoder sees arbitrary bytes, not only checksum failures.
//! - Oracle: whatever decodes re-encodes and decodes to the same namespace, and fingerprints
//!   without a panic.
//! - The namespace-key and watermark decoders (the cache's key and compare-and-set columns)
//!   and `RowCipher::open` (an encrypted row) take the raw input too; opening arbitrary bytes
//!   is always an error.

#![no_main]

use hacivat_meta::codec::{
    FORMAT_MAJOR, FORMAT_MINOR, HEADER_LEN, decode, decode_namespace, decode_ns_key,
    decode_watermark, encode_namespace, payload_version, reseal_checksum,
};
use hacivat_meta::fingerprint::subtree;
use hacivat_meta::ids::ObjId;
use hacivat_meta::persist::{CATALOG_DOMAIN, RowCipher, RowKey};
use libfuzzer_sys::fuzz_target;

fn check(bytes: &[u8]) {
    let Ok(d) = decode(bytes) else {
        return;
    };
    let again = encode_namespace(&d.ns).expect("a decoded namespace encodes");
    let back = decode_namespace(&again).expect("a re-encoded namespace decodes");
    assert_eq!(back.key(), d.ns.key());
    assert_eq!(back.objs(), d.ns.objs());
    assert_eq!(back.meta(), d.ns.meta());
    let root = ObjId { ns: 0, idx: 0 };
    assert_eq!(subtree(&back, root), subtree(&d.ns, root));
}

fuzz_target!(|data: &[u8]| {
    let _ = payload_version(data);
    check(data);

    let minor = match data.first() {
        Some(b) if b & 1 == 1 => FORMAT_MINOR + 1,
        _ => FORMAT_MINOR,
    };
    let mut framed = Vec::with_capacity(HEADER_LEN + data.len());
    framed.extend_from_slice(b"HVNS");
    framed.extend_from_slice(&FORMAT_MAJOR.to_le_bytes());
    framed.extend_from_slice(&minor.to_le_bytes());
    framed.push(0);
    framed.extend_from_slice(&[0; 8]);
    framed.extend_from_slice(data.get(1..).unwrap_or_default());
    reseal_checksum(&mut framed);
    check(&framed);

    let _ = decode_ns_key(data);
    let _ = decode_watermark(data);
    let cipher = RowCipher::new(RowKey::from_bytes([7; 32]), CATALOG_DOMAIN);
    assert!(cipher.open(b"hv-fuzz-ad", data).is_err());
});
