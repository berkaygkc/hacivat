//! Fuzz target `system_snapshot_decode` (M4 Task 1a skeleton; body Task 23): decoding an
//! untrusted system-catalog snapshot never panics (R-M4-33; Review Focus 4). Until Task 23 the
//! decoder refuses every input, so the target only proves the call path.

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = hacivat_meta::system::decode(data);
});
