//! The policy table (`alg/13` SAFE-07 model; Task 4): a report, a profile and an actor give a
//! verdict.
//!
//! STUB until Task 4 (fail closed): [`decide`] answers `Deny([NotImplemented])`.

use crate::effects::CoarseClass;
use crate::reason::{Reason, ReasonCode};
use crate::report::SafetyReport;

/// What a connection allows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Profile {
    ReadOnly,
    ReadOnlyPlusExplain,
    ReadWriteDml,
    Ddl,
    Admin,
}

/// Who sends the request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Actor {
    Human,
    AiAgent,
    Automation,
}

/// The policy's answer.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Verdict {
    Allow,
    Confirm(Vec<Reason>),
    Deny(Vec<Reason>),
}

/// A verdict with the class it was decided for.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Decision {
    pub verdict: Verdict,
    pub class: CoarseClass,
}

/// The verdict for `r` under `p` for `a`. STUB: `Deny([NotImplemented])`, class `Unknown`.
pub fn decide(r: &SafetyReport, p: Profile, a: Actor) -> Decision {
    let _ = (r, p, a);
    Decision {
        verdict: Verdict::Deny(vec![Reason::new(ReasonCode::NotImplemented)]),
        class: CoarseClass::Unknown,
    }
}
