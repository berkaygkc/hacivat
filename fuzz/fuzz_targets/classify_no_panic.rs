//! Fuzz target `classify_no_panic` (SQL-T13; Task 20 writes the body). Stub: accepts every input.

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = data;
});
