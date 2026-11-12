//! The step trail: what the connect pipeline shows while it runs (HV-CONN-01). Types complete
//! since Task 1.

use std::time::{Duration, Instant};

/// One visible connect step (HV-CONN-01).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TrailStep {
    PgPass,
    Resolve,
    Connect,
    Tls,
    SignIn,
    CheckServer,
    OpenFile,
}

impl TrailStep {
    /// The stable ID: `pgpass`, `resolve`, `connect`, `tls`, `sign_in`, `check_server`,
    /// `open_file`.
    pub fn id(self) -> &'static str {
        match self {
            TrailStep::PgPass => "pgpass",
            TrailStep::Resolve => "resolve",
            TrailStep::Connect => "connect",
            TrailStep::Tls => "tls",
            TrailStep::SignIn => "sign_in",
            TrailStep::CheckServer => "check_server",
            TrailStep::OpenFile => "open_file",
        }
    }
}

/// How a step ended, or that it has not (HV-CONN-01).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StepOutcome {
    Pending,
    Running,
    Done,
    Skipped,
    Failed,
}

/// One step of a trail (HV-CONN-01).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct StepTrace {
    pub step: TrailStep,
    pub outcome: StepOutcome,
    pub elapsed: Option<Duration>,
}

/// A connect in progress (HV-CONN-01). UI clocks never read `started`; they read the GPUI
/// executor clock (P22).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Progress {
    pub trail: Vec<StepTrace>,
    pub current: Option<TrailStep>,
    pub started: Instant,
}
