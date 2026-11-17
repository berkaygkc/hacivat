//! Catalog introspectors (HV-INTRO-*), one module per DBMS family.
//!
//! **M4 pre-work spike** (branch `m4-spike-pg-intro`): only the PostgreSQL introspector
//! ([`pg`]) exists. It produces plain structs local to this crate ([`pg::model`]); the M4
//! metadata model (`hacivat-meta`, HV-MODEL-01) absorbs them through the mapping that
//! [`pg::model`]'s module docs describe. Nothing here is wired into the app.

#![allow(
    clippy::result_large_err,
    reason = "RawError (inside IntroError) is a fixed interface; one per failed pass"
)]

pub mod pg;
