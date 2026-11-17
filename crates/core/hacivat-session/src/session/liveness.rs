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
//!
//! ## m3a-t18-session-run
//! While a console cursor is suspended the actor skips the `Ping` a tick asks for (MySQL and
//! SQLite take no command beside an open query; the cursor's idle bound, at most ten minutes,
//! ends it), and the supervisor asks again on its next tick. The actor ticks with the user's
//! transaction status: Hacivat's own transaction around a cursor counts as idle, so it draws
//! no `IdleTransaction` and does not hold up an idle disconnect (the close releases the cursor
//! first).
//!
//! ## m3a-f1-followups (named exception, follow-up F1-b)
//! The default session's idle disconnect ends the whole profile (`end_live` closes every
//! session with a rollback), so it waits for the profile, not only for itself: while another
//! session of the profile holds a user transaction or runs a user request
//! ([`ProfileActivity`]), or for the idle threshold after the profile's last user activity on
//! any session, a tick that would disconnect does not. Keep-alive pings and the idle
//! transaction warning still follow the default session's own activity (its connection needs
//! its own keep-alive). After a clock jump only a profile held *now* keeps it connected: the
//! profile's last activity is on the monotonic clock, which may not count the sleep.
//!
//! ## m3b-t12-session-carries (C7, R-M3-73; ADR 0004 `m3b-t12`)
//! Every console session of a profile runs its own supervisor with the profile's policy
//! ([`LivenessRole`]): its own keep-alive (its connection needs one), and its own idle clock,
//! so `IdleTransaction` fires per session, for the session whose transaction idles. Only the
//! profile's default session (`LivenessRole::Profile`) ever disconnects: its idle disconnect
//! ends the whole profile, and waits for the profile's last user activity on any session and
//! for every user transaction or user request on any of them (F1-b above). A console's
//! supervisor (`LivenessRole::Console`) never disconnects; past the threshold it keeps
//! pinging.

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant, SystemTime};

use hacivat_driver::TxStatus;
use hacivat_proto::EnvTag;

use crate::profile::{Profile, TransportSpec};
// ## m3a-f1-followups
use crate::session::SessionId;

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

// ## m3b-t12-session-carries (C7, R-M3-73)
/// Which supervisor a session runs (the module docs).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LivenessRole {
    /// The profile's default session: its idle disconnect ends the profile.
    Profile,
    /// Another console session: keep-alive and its own idle-transaction warning, never a
    /// disconnect.
    Console,
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
    // ## m3a-f1-followups
    /// Another session of the profile holds a user transaction or runs a user request.
    profile_held: bool,
    /// The profile's last user activity on any session.
    profile_last: Option<Instant>,
    // ## m3b-t12-session-carries
    /// `Console`: never returns `Disconnect` (R-M3-73).
    role: LivenessRole,
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
            profile_held: false,
            profile_last: None,
            role: LivenessRole::Profile,
        }
    }

    // ## m3b-t12-session-carries
    /// The supervisor of a session in `role` (the module docs); `new` is `Profile`'s.
    pub fn with_role(mut self, role: LivenessRole) -> Self {
        self.role = role;
        self
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
            // ## m3a-f1-followups: nor while the profile is held now.
            if !open_tx
                && !self.profile_held
                && self.role == LivenessRole::Profile
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
            // ## m3a-f1-followups: the profile's other sessions hold the disconnect off.
            // ## m3b-t12-session-carries: only the profile's default disconnects.
            if !open_tx && self.role == LivenessRole::Profile && !self.profile_holds(now, limit) {
                return Some(LivenessAction::Disconnect);
            }
            if open_tx && !self.warned {
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

    // ## m3a-f1-followups
    /// What the profile's other sessions do now (the module docs, F1-b): `held` while one holds
    /// a user transaction or runs a user request, `last` their latest user activity. The actor
    /// passes it before every tick; it never counts as this session's activity.
    pub fn note_profile(&mut self, held: bool, last: Option<Instant>) {
        self.profile_held = held;
        self.profile_last = self.profile_last.max(last);
    }

    // ## m3a-f1-followups
    /// Whether the profile keeps an idle disconnect at `now` off: it is held, or had user
    /// activity within `limit`.
    fn profile_holds(&self, now: Instant, limit: Duration) -> bool {
        self.profile_held
            || self
                .profile_last
                .is_some_and(|at| now.saturating_duration_since(at) < limit)
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

// ## m3a-f1-followups (named exception, follow-up F1-b)
/// What the sessions of one profile tell the default session's supervisor (the module docs):
/// which of them hold a user transaction or run a user request now, and when the profile last
/// had user activity. Every session the profile's `SessionManager` starts reports to it; a
/// session that ends leaves it.
///
/// ## m3b-t26-prod-unlock (fix round 1, A-I1): it also carries the profile's live environment
/// tag (`SessionManager::update_profile` sets it), which a session opened before a retag reads
/// to check its startup script on a reset or reconnect (`startup_check`).
#[derive(Debug, Default)]
pub(crate) struct ProfileActivity {
    inner: Mutex<ProfileInner>,
    production: AtomicBool,
}

#[derive(Debug, Default)]
struct ProfileInner {
    last: Option<Instant>,
    held: HashSet<SessionId>,
}

impl ProfileActivity {
    fn lock(&self) -> std::sync::MutexGuard<'_, ProfileInner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Session `id` had user activity at `at` (if any), and holds the profile (`held`) or not.
    pub(crate) fn report(&self, id: SessionId, at: Option<Instant>, held: bool) {
        let mut inner = self.lock();
        inner.last = inner.last.max(at);
        if held {
            inner.held.insert(id);
        } else {
            inner.held.remove(&id);
        }
    }

    // ## m3b-t26-prod-unlock (fix round 1, A-I1)
    /// Records the profile's live tag: production or not. Returns whether it was production.
    pub(crate) fn set_production(&self, production: bool) -> bool {
        self.production.swap(production, Ordering::SeqCst)
    }

    /// Whether the profile is tagged production now (its live tag, not the one a session
    /// opened with).
    pub(crate) fn production(&self) -> bool {
        self.production.load(Ordering::SeqCst)
    }

    /// Session `id` ended.
    pub(crate) fn leave(&self, id: SessionId) {
        self.lock().held.remove(&id);
    }

    // ## m3b-t12-session-carries (fix round 1, review I1)
    /// Whether session `id` holds the profile now (a user transaction, or a user request
    /// that runs).
    pub(crate) fn holds(&self, id: SessionId) -> bool {
        self.lock().held.contains(&id)
    }

    /// Whether a session other than `me` holds the profile, and the profile's last activity.
    pub(crate) fn others(&self, me: SessionId) -> (bool, Option<Instant>) {
        let inner = self.lock();
        (inner.held.iter().any(|id| *id != me), inner.last)
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

    // ## m3a-f1-followups
    fn idle_policy() -> LivenessPolicy {
        LivenessPolicy {
            keepalive: Some(Duration::from_secs(60)),
            idle_disconnect: Some(Duration::from_secs(300)),
            tick: Duration::from_secs(1),
        }
    }

    /// F1-b: a profile held by another session never lets the default disconnect, its own
    /// keep-alive still runs, and once the hold ends the threshold counts from the profile's
    /// last activity.
    #[test]
    fn a_held_profile_holds_the_idle_disconnect_off() {
        let t0 = Instant::now();
        let w0 = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000);
        let mut s = Supervisor::new(idle_policy(), t0, w0);
        let mut actions = Vec::new();
        s.note_profile(true, None);
        for n in 1..=900 {
            let at = t0 + Duration::from_secs(n);
            let action = s.tick(at, w0 + Duration::from_secs(n), false, TxStatus::Idle);
            if action == Some(LivenessAction::Ping) {
                s.note_ping(at);
            }
            actions.extend(action);
        }
        assert!(
            !actions.contains(&LivenessAction::Disconnect),
            "{actions:?}"
        );
        assert_eq!(
            actions
                .iter()
                .filter(|a| **a == LivenessAction::Ping)
                .count(),
            15,
            "the default's own keep-alive"
        );
        // The hold ends with activity at 900 s: no disconnect before 1200 s, one at 1200 s.
        s.note_profile(false, Some(t0 + Duration::from_secs(900)));
        let tick = |s: &mut Supervisor, n: u64| {
            s.tick(
                t0 + Duration::from_secs(n),
                w0 + Duration::from_secs(n),
                false,
                TxStatus::Idle,
            )
        };
        for n in 901..1200 {
            assert_ne!(tick(&mut s, n), Some(LivenessAction::Disconnect), "{n}");
        }
        assert_eq!(tick(&mut s, 1200), Some(LivenessAction::Disconnect));
    }

    /// After a clock jump only a hold *now* keeps the profile: recent activity on the monotonic
    /// clock does not (it may not count the sleep).
    #[test]
    fn after_a_jump_only_a_current_hold_keeps_the_profile() {
        let t0 = Instant::now();
        let w0 = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000);
        for (held, expect) in [
            (true, LivenessAction::Ping),
            (false, LivenessAction::Disconnect),
        ] {
            let mut s = Supervisor::new(idle_policy(), t0, w0);
            s.note_profile(held, Some(t0));
            // One second of monotonic time, an hour of wall time.
            let (at, wall) = (t0 + Duration::from_secs(1), w0 + Duration::from_secs(3600));
            assert_eq!(
                s.tick(at, wall, false, TxStatus::Idle),
                Some(LivenessAction::Suspect)
            );
            assert_eq!(
                s.tick(at, wall, false, TxStatus::Idle),
                Some(expect),
                "{held}"
            );
        }
    }

    // ## m3b-t12-session-carries
    /// R-M3-73: a console's supervisor keeps pinging past the idle threshold and warns of its
    /// own idle transaction, but never disconnects (only the profile's default does), also
    /// after a clock jump.
    #[test]
    fn a_console_supervisor_never_disconnects() {
        let t0 = Instant::now();
        let w0 = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000);
        let at = |n: u64| (t0 + Duration::from_secs(n), w0 + Duration::from_secs(n));
        let run = |role| {
            let mut s = Supervisor::new(idle_policy(), t0, w0).with_role(role);
            let mut actions = Vec::new();
            for n in 1..=900 {
                let (now, wall) = at(n);
                let action = s.tick(now, wall, false, TxStatus::Idle);
                if action == Some(LivenessAction::Ping) {
                    s.note_ping(now);
                }
                actions.extend(action);
                if action == Some(LivenessAction::Disconnect) {
                    break;
                }
            }
            actions
        };
        let console = run(LivenessRole::Console);
        assert!(
            !console.contains(&LivenessAction::Disconnect),
            "{console:?}"
        );
        assert_eq!(
            console
                .iter()
                .filter(|a| **a == LivenessAction::Ping)
                .count(),
            15,
            "its own keep-alive goes on"
        );
        assert_eq!(
            run(LivenessRole::Profile).last(),
            Some(&LivenessAction::Disconnect)
        );
        // Its own idle transaction is warned of, once.
        let mut s = Supervisor::new(idle_policy(), t0, w0).with_role(LivenessRole::Console);
        let warned = (1..=900)
            .filter_map(|n| {
                let (now, wall) = at(n);
                s.tick(now, wall, false, TxStatus::InTx)
            })
            .filter(|a| *a == LivenessAction::IdleTransaction)
            .count();
        assert_eq!(warned, 1);
        // After a clock jump: a ping, never a disconnect.
        let mut s = Supervisor::new(idle_policy(), t0, w0).with_role(LivenessRole::Console);
        let (now, wall) = (t0 + Duration::from_secs(1), w0 + Duration::from_secs(3600));
        assert_eq!(
            s.tick(now, wall, false, TxStatus::Idle),
            Some(LivenessAction::Suspect)
        );
        assert_eq!(
            s.tick(now, wall, false, TxStatus::Idle),
            Some(LivenessAction::Ping)
        );
    }

    #[test]
    fn profile_activity_tracks_holders_but_not_oneself() {
        let p = ProfileActivity::default();
        let (a, b) = (SessionId(1), SessionId(2));
        let t = Instant::now();
        p.report(b, Some(t), true);
        assert_eq!(p.others(a), (true, Some(t)));
        assert_eq!(p.others(b), (false, Some(t)));
        p.report(b, None, false);
        assert_eq!(p.others(a), (false, Some(t)), "the last activity stays");
        p.report(b, None, true);
        p.leave(b);
        assert!(!p.others(a).0);
    }
}
