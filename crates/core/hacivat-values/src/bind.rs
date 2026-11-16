//! Bind parameter values (HV-EXEC-03; M3b uses them, the driver signature carries them now).

use std::sync::Arc;

use crate::logical::LogicalType;

/// One bind parameter: SQL NULL, or text with a type hint.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum BindValue {
    Null,
    Typed { hint: LogicalType, text: Arc<str> },
}
