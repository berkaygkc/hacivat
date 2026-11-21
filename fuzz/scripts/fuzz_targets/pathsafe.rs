//! Fuzz target `pathsafe` (M4b; body: Task 20): validating an untrusted file or folder name
//! from a library (HV-SCRIPT-03 step 4) never panics, whatever the bytes.
//!
//! Task 1a stub: the target is declared and builds; Task 20 calls `pathsafe::component` and
//! adds the oracles.

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = data;
});
