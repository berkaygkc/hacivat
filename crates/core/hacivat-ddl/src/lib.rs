//! DDL generation (HV-DDL-07 view-only producers, -09, -10, -11 subset; alg/03): the IR, the
//! per-dialect renderers and the producers over catalog snapshots. Pure on its default
//! features (`DdlIsPure`).

pub mod ir;
pub mod options;
pub mod produce;
pub mod render;
pub mod script;
