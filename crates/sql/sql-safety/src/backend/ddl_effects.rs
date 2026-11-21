// ## m4-t06-ddl-effects
//! The catalog effects each backend reads from its own parse of a statement (M4 Task 6;
//! `crate::ddl_effects` decides the opaque rule and the scope).

pub(crate) mod interim;
pub(crate) mod pg;
