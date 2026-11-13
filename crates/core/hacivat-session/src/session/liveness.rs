//! Liveness (HV-CONN-04, B18, Q31; Task 17): keepalive pings, idle disconnect, the idle
//! transaction warning and sleep detection.

use std::time::{Duration, Instant, SystemTime};

use hacivat_driver::TxStatus;

use crate::profile::Profile;

/// When a session pings and disconnects.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LivenessPolicy {
    pub keepalive: Option<Duration>,
    pub idle_disconnect: Option<Duration>,
    /// How often the supervisor ticks.
    pub tick: Duration,
}

impl LivenessPolicy {
    /// The policy of `p`. Task 1 stub: no pings, no idle disconnect, a 1 s tick (Task 17).
    pub fn for_profile(_p: &Profile) -> LivenessPolicy {
        LivenessPolicy {
            keepalive: None,
            idle_disconnect: None,
            tick: Duration::from_secs(1),
        }
    }
}

/// What a tick asks the session to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LivenessAction {
    Ping,
    Disconnect,
    IdleTransaction,
    /// The clocks jumped (sleep): check the connection before trusting it.
    Suspect,
}

/// A session's liveness clock (pure state; the actor ticks it).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[allow(dead_code, reason = "Task 17 ticks on them")]
pub struct Supervisor {
    policy: LivenessPolicy,
    last_activity: Instant,
    last_ping: Instant,
    wall: SystemTime,
}

impl Supervisor {
    pub fn new(p: LivenessPolicy, now: Instant, wall: SystemTime) -> Self {
        Supervisor {
            policy: p,
            last_activity: now,
            last_ping: now,
            wall,
        }
    }

    /// The action due at `now`, if any. Task 1 stub: none (Task 17).
    pub fn tick(
        &mut self,
        _now: Instant,
        _wall: SystemTime,
        _busy: bool,
        _tx: TxStatus,
    ) -> Option<LivenessAction> {
        None
    }

    pub fn note_activity(&mut self, now: Instant) {
        self.last_activity = now;
    }

    pub fn note_ping(&mut self, now: Instant) {
        self.last_ping = now;
    }
}
