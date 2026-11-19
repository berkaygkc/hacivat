//! Fuzz target `scope_pattern` (M4 Task 1a skeleton; body Task 12): parsing a scope or object
//! filter pattern (HV-INTRO-02, -03) never panics and reports errors at a character position.
//! The grammar's seam is Task 1c's (`hacivat_introspect::scope`); until Task 12 the target
//! accepts its input without calling it.

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = std::str::from_utf8(data);
});
