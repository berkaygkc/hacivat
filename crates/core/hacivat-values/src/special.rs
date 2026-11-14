//! Special values the servers send in place of a number or a date (HV-RES-01; Task 6).

use std::sync::Arc;

/// `infinity`, `-infinity`, `NaN`, MySQL's zero date and its partial-zero forms (verbatim).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum SpecialVal {
    PosInfinity,
    NegInfinity,
    NaN,
    ZeroDate,
    PartialZeroDate(Arc<str>),
}
