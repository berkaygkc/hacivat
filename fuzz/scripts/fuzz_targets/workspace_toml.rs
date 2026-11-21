//! Fuzz target `workspace_toml` (M4b; body: Task 20): reading an untrusted
//! `.hacivat/workspace.toml` (HV-SCRIPT-01 step 3) never panics, whatever the bytes.
//!
//! Task 1a stub: the call only (`manifest::read_bytes` is itself a stub until Task 3); Task 20
//! adds the oracles (a bound name passes the character rules, no key outside the schema binds
//! anything, the size limit holds).

#![no_main]

use hacivat_scripts::manifest;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = manifest::read_bytes(data, "library");
});
