//! The driver layer (HV-DRV-01 to -05, HV-CONN-03): the driver and raw-connection traits, the
//! engine catalog and registry, server detection, option sanitizing, error classification, URL
//! templates and parsing, and (feature `fake`) a scripted test driver. Traits and types are
//! complete since Task 1; bodies arrive in Tasks 2, 3, 7 and 10.

// `RawError` carries the server's message by value (HV-CONN-03, I18N-14), so its size is by
// design; it travels once per failed call.
#![allow(
    clippy::result_large_err,
    reason = "RawError is a fixed interface; one per failed call"
)]

pub mod catalog;
pub mod driver;
pub mod errors;
#[cfg(feature = "fake")]
pub mod fake;
pub mod registry;
pub mod sanitize;
pub mod server;
pub mod url;

pub use catalog::{CatalogError, ConnTypeSpec, DefaultOption, EngineCatalog, EngineSpec};
pub use driver::{
    CancelHandle, CancelMechanism, Driver, DriverCaps, DriverConnect, DriverDescriptor,
    DriverEvent, DriverEvents, Endpoint, ProbeWarning, RawConn,
};
pub use errors::{
    Classified, Phase, RawError, RawErrorKind, ServerMessage, classify, fixes_for,
    reachable_classes,
};
pub use registry::DriverRegistry;
pub use sanitize::{SanitizeInput, Violation, ViolationReason, sanitize};
pub use server::{SemVerLoose, ServerInfo, Support, detect_mysql_family, support_status};
