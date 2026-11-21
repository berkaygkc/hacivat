//! Fuzz target `workspace_toml` (M4b; body: Task 20): reading an untrusted
//! `.hacivat/workspace.toml` (HV-SCRIPT-01 step 3) never panics, whatever the bytes.
//!
//! Task 1a stub: the target is declared and builds; Task 20 calls `manifest::read_bytes` and
//! adds the oracles.

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = data;
});
