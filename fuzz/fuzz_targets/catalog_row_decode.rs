//! Fuzz target `catalog_row_decode` (M4 Task 1a skeleton; body Task 11): decoding an untrusted
//! cache row never panics (HV-MODEL-03; Review Focus 4). Until Task 11 the decoder refuses
//! every input, so the target only proves the call path.

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = hacivat_meta::codec::payload_version(data);
    let _ = hacivat_meta::codec::decode_namespace(data);
});
