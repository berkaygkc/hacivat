//! Recovery after a lost connection (HV-CONN-05, D-085, B4, B19; Task 17): the reconnect
//! schedule, which failures it retries, and what state a reconnect loses.

use std::time::Duration;

use hacivat_driver::TxStatus;
use hacivat_proto::ErrClass;

/// The reconnect schedule.
#[derive(Debug, Clone, PartialEq)]
pub struct RecoveryPolicy {
    /// The first delays, in order.
    pub schedule: Vec<Duration>,
    /// The delay after the schedule runs out.
    pub steady: Duration,
    pub give_up_after: Duration,
    /// The jitter fraction (0.2 is ±20 %).
    pub jitter: f32,
}

impl RecoveryPolicy {
    /// The delay before `attempt` (from 1), `elapsed` after the loss; `None` gives up. Task 1
    /// stub: gives up at once (Task 17).
    pub fn delay(
        &self,
        _attempt: u32,
        _elapsed: Duration,
        _rng: &mut dyn FnMut() -> f32,
    ) -> Option<Duration> {
        None
    }
}

/// Whether a lost connection of `class` is worth reconnecting. Task 1 stub: never (Task 17).
pub fn is_recovery_case(_class: ErrClass) -> bool {
    false
}

/// What a session did that a reconnect would lose (a first-keyword scan, B19).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct Tracked {
    pub tx: TxStatus,
    pub temp_created: bool,
    pub set_seen: bool,
    pub prepared: u32,
    pub locks_seen: bool,
}

/// Notes what `sql` does to the session's state. Task 1 stub: notes nothing (Task 17).
pub fn track_statement(_t: &mut Tracked, _sql: &str) {}

/// What a reconnect loses, by what the session did.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct LostState {
    pub open_tx: bool,
    pub temp_objects: bool,
    pub session_vars: bool,
    pub prepared: u32,
    pub locks: bool,
}

impl LostState {
    /// Nothing is lost.
    pub fn is_empty(&self) -> bool {
        *self == LostState::default()
    }
}

/// What a reconnect after `t` loses.
pub fn lost_state(t: &Tracked) -> LostState {
    LostState {
        open_tx: t.tx != TxStatus::Idle,
        temp_objects: t.temp_created,
        session_vars: t.set_seen,
        prepared: t.prepared,
        locks: t.locks_seen,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lost_state_follows_what_was_tracked() {
        assert!(lost_state(&Tracked::default()).is_empty());
        let t = Tracked {
            tx: TxStatus::InTx,
            prepared: 2,
            ..Tracked::default()
        };
        let lost = lost_state(&t);
        assert!(lost.open_tx && lost.prepared == 2 && !lost.is_empty());
    }
}
