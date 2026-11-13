//! Liveness (HV-CONN-04, B18, Q31; Task 17): keepalive pings, idle disconnect, the idle
//! transaction warning and sleep detection.
//!
//! The [`Supervisor`] is pure: the session actor ticks it every [`LivenessPolicy::tick`] (a
//! Tokio `interval` with `MissedTickBehavior::Skip`) with both clocks, and acts on what it
//! returns. A tick:
//! 1. while the session is busy returns nothing (a running request is activity);
//! 2. after a clock jump (`wall_delta − mono_delta > 5 s`, or `mono_delta > tick + 5 s`, since
//!    a Windows `Instant` may include suspended time, Q31) returns `Suspect` once; the tick
//!    after it, with the transaction idle, compares the idle threshold against the time since
//!    the last activity on the wall clock (or the monotonic one, whichever is longer):
//!    `Disconnect` when it is exceeded, else `Ping`; with a transaction open it is always
//!    `Ping`. Missed ticks are never replayed;
//! 3. at the idle threshold returns `IdleTransaction` once per idle period while a transaction
//!    is open (`InTx` or `Failed`; D-014: notify only), otherwise `Disconnect`;
//! 4. when a keep-alive is due, counted from the later of the last activity and the last
//!    ping, returns `Ping`.
//!
//! Keep-alive pings never count as activity.

use std::time::{Duration, Instant, SystemTime};

use hacivat_driver::TxStatus;
use hacivat_proto::EnvTag;

use crate::profile::{Profile, TransportSpec};

/// How far the clocks may drift apart between two ticks before the session is suspect.
const JUMP: Duration = Duration::from_secs(5);

/// The keep-alive of an SSH profile without one of its own (B18).
const SSH_KEEPALIVE: Duration = Duration::from_secs(60);

const HOUR: u64 = 3600;

/// When a session pings and disconnects.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LivenessPolicy {
    pub keepalive: Option<Duration>,
    pub idle_disconnect: Option<Duration>,
    /// How often the supervisor ticks.
    pub tick: Duration,
}

impl LivenessPolicy {
    /// The policy of `p` (B18):
    /// - keep-alive: `Profile::keepalive_s`; without it 60 s over SSH, otherwise off;
    /// - idle disconnect: `Profile::idle_disconnect_s`; without it by environment: dev 4 h,
    ///   test and staging 2 h, production 1 h, local or untagged off.
    ///
    /// A value of 0 turns either off. The supervisor ticks every second.
    pub fn for_profile(p: &Profile) -> LivenessPolicy {
        let keepalive = match p.keepalive_s {
            Some(s) => seconds(s),
            None if matches!(p.transport, TransportSpec::SshTunnel(_)) => Some(SSH_KEEPALIVE),
            None => None,
        };
        let idle_disconnect = match p.idle_disconnect_s {
            Some(s) => seconds(s),
            None => match p.environment {
                Some(EnvTag::Dev) => Some(Duration::from_secs(4 * HOUR)),
                Some(EnvTag::Test | EnvTag::Staging) => Some(Duration::from_secs(2 * HOUR)),
                Some(EnvTag::Production) => Some(Duration::from_secs(HOUR)),
                Some(EnvTag::Local) | None => None,
            },
        };
        LivenessPolicy {
            keepalive,
            idle_disconnect,
            tick: Duration::from_secs(1),
        }
    }

    /// How long a keep-alive ping may take: half its period, at most 10 s.
    pub fn ping_timeout(&self) -> Duration {
        const MAX: Duration = Duration::from_secs(10);
        self.keepalive.map_or(MAX, |every| (every / 2).min(MAX))
    }

    /// The idle threshold in whole minutes, as `SessionEvent::IdleTransaction` reports it.
    pub fn idle_minutes(&self) -> u32 {
        self.idle_disconnect
            .map_or(0, |d| u32::try_from(d.as_secs() / 60).unwrap_or(u32::MAX))
    }
}

/// `s` seconds, `None` for 0 (off).
fn seconds(s: u32) -> Option<Duration> {
    (s > 0).then(|| Duration::from_secs(u64::from(s)))
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

/// A session's liveness clock (pure state; the actor ticks it). The module docs have the rules.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Supervisor {
    policy: LivenessPolicy,
    last_activity: Instant,
    last_ping: Instant,
    /// Both clocks at the last tick (or resync).
    tick_mono: Instant,
    tick_wall: SystemTime,
    /// Wall time that passed between ticks since the last activity.
    idle_wall: Duration,
    /// A `Suspect` was returned; the next tick decides between `Ping` and `Disconnect`.
    suspect: bool,
    /// `IdleTransaction` was returned in this idle period.
    warned: bool,
}

impl Supervisor {
    pub fn new(p: LivenessPolicy, now: Instant, wall: SystemTime) -> Self {
        Supervisor {
            policy: p,
            last_activity: now,
            last_ping: now,
            tick_mono: now,
            tick_wall: wall,
            idle_wall: Duration::ZERO,
            suspect: false,
            warned: false,
        }
    }

    pub fn policy(&self) -> &LivenessPolicy {
        &self.policy
    }

    /// The action due at `now` (the module docs), if any.
    pub fn tick(
        &mut self,
        now: Instant,
        wall: SystemTime,
        busy: bool,
        tx: TxStatus,
    ) -> Option<LivenessAction> {
        let mono_delta = now.saturating_duration_since(self.tick_mono);
        // A wall clock set backwards counts as no time.
        let wall_delta = wall.duration_since(self.tick_wall).unwrap_or_default();
        self.tick_mono = now;
        self.tick_wall = wall;
        if busy {
            self.note_activity(now);
            return None;
        }
        self.idle_wall = self.idle_wall.saturating_add(wall_delta);
        let jumped =
            wall_delta.saturating_sub(mono_delta) > JUMP || mono_delta > self.policy.tick + JUMP;
        if jumped && !self.suspect {
            self.suspect = true;
            return Some(LivenessAction::Suspect);
        }
        let open_tx = matches!(tx, TxStatus::InTx | TxStatus::Failed);
        if std::mem::take(&mut self.suspect) {
            // B18: one immediate check. Only an idle transaction may end in a disconnect; an
            // open one is pinged (valid in `InTx` and `Failed`), never closed (fix round 1, M1).
            let idle = self
                .idle_wall
                .max(now.saturating_duration_since(self.last_activity));
            if !open_tx
                && self
                    .policy
                    .idle_disconnect
                    .is_some_and(|limit| idle >= limit)
            {
                return Some(LivenessAction::Disconnect);
            }
            return Some(LivenessAction::Ping);
        }
        if let Some(limit) = self.policy.idle_disconnect
            && now.saturating_duration_since(self.last_activity) >= limit
        {
            if !open_tx {
                return Some(LivenessAction::Disconnect);
            }
            if !self.warned {
                self.warned = true;
                return Some(LivenessAction::IdleTransaction);
            }
        }
        if let Some(every) = self.policy.keepalive
            && now.saturating_duration_since(self.last_activity.max(self.last_ping)) >= every
        {
            return Some(LivenessAction::Ping);
        }
        None
    }

    /// The user did something on the session: a new idle period starts.
    pub fn note_activity(&mut self, now: Instant) {
        self.last_activity = now;
        self.idle_wall = Duration::ZERO;
        self.warned = false;
    }

    /// A keep-alive ping answered (never activity).
    pub fn note_ping(&mut self, now: Instant) {
        self.last_ping = now;
    }

    /// The session was away from its ticks (a ping, a reconnect): the next tick measures both
    /// clocks from here, so the time spent is not taken for a jump. Not activity.
    pub fn resync(&mut self, now: Instant, wall: SystemTime) {
        let wall_delta = wall.duration_since(self.tick_wall).unwrap_or_default();
        self.idle_wall = self.idle_wall.saturating_add(wall_delta);
        self.tick_mono = now;
        self.tick_wall = wall;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_ping_timeout_is_half_the_period_and_at_most_ten_seconds() {
        let policy = |keepalive| LivenessPolicy {
            keepalive,
            idle_disconnect: None,
            tick: Duration::from_secs(1),
        };
        assert_eq!(
            policy(Some(Duration::from_secs(60))).ping_timeout(),
            Duration::from_secs(10)
        );
        assert_eq!(
            policy(Some(Duration::from_millis(20))).ping_timeout(),
            Duration::from_millis(10)
        );
        assert_eq!(policy(None).ping_timeout(), Duration::from_secs(10));
    }

    #[test]
    fn a_wall_clock_set_back_is_no_jump_and_resync_hides_time_away() {
        let t0 = Instant::now();
        let w0 = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000);
        let policy = LivenessPolicy {
            keepalive: Some(Duration::from_secs(60)),
            idle_disconnect: None,
            tick: Duration::from_secs(1),
        };
        let mut s = Supervisor::new(policy, t0, w0);
        let at = |n: u64| t0 + Duration::from_secs(n);
        let wall = |n: u64| w0 + Duration::from_secs(n);
        assert_eq!(
            s.tick(at(1), w0 - Duration::from_secs(3600), false, TxStatus::Idle),
            None
        );
        // Ten seconds away in a ping, then resynced: no jump.
        s.resync(at(11), wall(11));
        assert_eq!(s.tick(at(12), wall(12), false, TxStatus::Idle), None);
    }
}
