//! Script tabs across a restart (R-M4b-22, R-M4b-29; NOTES surface 12): `workspace.json`
//! version 2 saves each script tab as its own tab kind; on restore a missing library or file
//! opens the tab with a notice (and its recovery copy), a deleted profile leaves an unbound tab,
//! and the capsule always reads the profile's current environment.
//!
//! ## m4b-t01b-seams: the tab kind and the version bump are in the shell
//! (`shell::workspace`, `SavedKind::Script`); a restored script tab keeps its binding and shows
//! nothing until Task 14 fills this module.
