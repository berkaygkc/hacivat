//! Fuzz target `pathsafe` (M4b; body: Task 20): validating an untrusted file or folder name
//! from a library (HV-SCRIPT-03 step 4) never panics, whatever the bytes.
//!
//! Task 1a stub: the calls only (`pathsafe::component` is itself a stub until Task 4); Task 20
//! adds the oracles (an accepted name is one component, never `.`/`..`, holds no separator,
//! NUL or control character, and parses as a `RelPath`).

#![no_main]

use std::ffi::OsStr;

use hacivat_scripts::{RelPath, pathsafe};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(text) = std::str::from_utf8(data) {
        let _ = pathsafe::component(OsStr::new(text));
        let _ = RelPath::parse(text);
    }
});
