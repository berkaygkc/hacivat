//! Fuzz target `pack_parse` (M4 Task 1a skeleton; body Task 4): parsing a query pack
//! (HV-INTRO-04: `--- section ---` headers, closed macros) never panics. The pack parser's seam
//! is Task 1c's (`hacivat_introspect::pack`); until Task 4 the target accepts its input without
//! calling it.

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = std::str::from_utf8(data);
});
