//! The liveness supervisor (HV-CONN-04, B18, Q31; Task 17): pure, with both clocks driven by
//! the test, one tick per simulated second.

use std::time::{Duration, Instant, SystemTime};

use hacivat_driver::TxStatus;
use hacivat_proto::EnvTag;
use hacivat_session::profile::{Profile, TransportSpec};
use hacivat_session::session::{LivenessAction, LivenessPolicy, Supervisor};

fn secs(n: u64) -> Duration {
    Duration::from_secs(n)
}

/// A supervisor and its two clocks, `t` seconds after the start.
struct Sim {
    s: Supervisor,
    mono: Instant,
    wall: SystemTime,
    t: u64,
}

fn sup(keepalive: Option<u64>, idle: Option<u64>) -> Sim {
    let mono = Instant::now();
    let wall = SystemTime::UNIX_EPOCH + secs(1_700_000_000);
    let policy = LivenessPolicy {
        keepalive: keepalive.map(secs),
        idle_disconnect: idle.map(secs),
        tick: secs(1),
    };
    Sim {
        s: Supervisor::new(policy, mono, wall),
        mono,
        wall,
        t: 0,
    }
}

impl Sim {
    /// Moves the monotonic clock by `mono` and the wall clock by `wall`, then ticks. A ping the
    /// supervisor asks for answers at once.
    fn step(
        &mut self,
        mono: Duration,
        wall: Duration,
        busy: bool,
        tx: TxStatus,
    ) -> Option<LivenessAction> {
        self.mono += mono;
        self.wall += wall;
        self.t += mono.as_secs();
        let action = self.s.tick(self.mono, self.wall, busy, tx);
        if action == Some(LivenessAction::Ping) {
            self.s.note_ping(self.mono);
        }
        action
    }
}

/// The actions of a run, with the second (from the run's start) each came at.
struct Acts(Vec<(u64, LivenessAction)>);

impl Acts {
    fn iter(&self) -> impl Iterator<Item = &LivenessAction> {
        self.0.iter().map(|(_, a)| a)
    }

    fn contains(&self, a: &LivenessAction) -> bool {
        self.iter().any(|x| x == a)
    }
}

/// `n` idle ticks, one a second, with the transaction `tx`.
fn run_ticks(s: &mut Sim, n: u64, tx: TxStatus) -> Acts {
    let mut acts = Vec::new();
    for i in 1..=n {
        if let Some(a) = s.step(secs(1), secs(1), false, tx) {
            acts.push((i, a));
        }
    }
    Acts(acts)
}

fn pings_at(acts: &Acts) -> Vec<u64> {
    acts.0
        .iter()
        .filter(|(_, a)| *a == LivenessAction::Ping)
        .map(|(t, _)| *t)
        .collect()
}

fn disconnect_at(acts: &Acts) -> Option<u64> {
    acts.0
        .iter()
        .find(|(_, a)| *a == LivenessAction::Disconnect)
        .map(|(t, _)| *t)
}

/// `n` ticks while a request runs; the first action, if any.
fn tick_busy(s: &mut Sim, n: u64) -> Option<LivenessAction> {
    (0..n)
        .filter_map(|_| s.step(secs(1), secs(1), true, TxStatus::Idle))
        .next()
}

/// The machine slept: one tick sees the wall clock `by` ahead and the monotonic clock one
/// second ahead (macOS and Linux stop `Instant` while asleep); then one ordinary tick.
fn jump_wall(s: &mut Sim, by: Duration) -> Vec<LivenessAction> {
    let mut acts = Vec::new();
    acts.extend(s.step(secs(1), by + secs(1), false, TxStatus::Idle));
    acts.extend(s.step(secs(1), secs(1), false, TxStatus::Idle));
    acts
}

/// As [`jump_wall`], with both clocks ahead (a Windows `Instant` counts suspended time, Q31).
fn jump_mono_and_wall(s: &mut Sim, by: Duration) -> Vec<LivenessAction> {
    let mut acts = Vec::new();
    acts.extend(s.step(by, by, false, TxStatus::Idle));
    acts.extend(s.step(secs(1), secs(1), false, TxStatus::Idle));
    acts
}

/// Idle ticks for `d`, which must ask for nothing.
fn idle_for(s: &mut Sim, d: Duration) {
    let acts = run_ticks(s, d.as_secs(), TxStatus::Idle);
    assert!(acts.0.is_empty(), "{:?}", acts.0);
}

fn env(tag: EnvTag) -> Profile {
    Profile {
        environment: Some(tag),
        ..Profile::default()
    }
}

fn ssh_profile() -> Profile {
    Profile {
        transport: TransportSpec::SshTunnel(Default::default()),
        ..Profile::default()
    }
}

fn direct_profile() -> Profile {
    Profile::default()
}

#[test]
fn keepalive_every_60_idle_300() {
    let mut s = sup(Some(60), Some(300));
    let acts = run_ticks(&mut s, 301, TxStatus::Idle);
    assert_eq!(pings_at(&acts), [60, 120, 180, 240]);
    assert_eq!(disconnect_at(&acts), Some(300));
}

#[test]
fn open_tx_at_threshold_emits_idle_transaction_not_disconnect() {
    let mut s = sup(None, Some(300));
    let acts = run_ticks(&mut s, 900, TxStatus::InTx);
    assert_eq!(
        acts.iter()
            .filter(|a| **a == LivenessAction::IdleTransaction)
            .count(),
        1
    );
    assert!(!acts.contains(&LivenessAction::Disconnect));
}

#[test]
fn busy_skips_and_keepalive_is_not_activity() {
    let mut s = sup(Some(60), Some(120));
    assert!(tick_busy(&mut s, 200).is_none());
    let acts = run_ticks(&mut s, 121, TxStatus::Idle);
    assert_eq!(disconnect_at(&acts), Some(120));
}

#[test]
fn clock_jump_pings_once_not_per_missed_tick() {
    let mut s = sup(Some(60), None);
    let acts = jump_wall(&mut s, Duration::from_secs(7200));
    assert_eq!(acts, [LivenessAction::Suspect, LivenessAction::Ping]);
}

#[test]
fn monotonic_jump_alone_is_suspect() {
    // Windows QPC (Q31).
    let mut s = sup(Some(60), None);
    assert_eq!(
        jump_mono_and_wall(&mut s, secs(7200)),
        [LivenessAction::Suspect, LivenessAction::Ping]
    );
}

#[test]
fn sleep_past_idle_threshold_disconnects() {
    // Q31: 59 minutes idle, then a night asleep.
    let mut s = sup(None, Some(3600));
    idle_for(&mut s, secs(59 * 60));
    assert_eq!(
        jump_wall(&mut s, secs(8 * 3600)),
        [LivenessAction::Suspect, LivenessAction::Disconnect]
    );
}

#[test]
fn presets() {
    assert_eq!(
        LivenessPolicy::for_profile(&env(EnvTag::Production)).idle_disconnect,
        Some(secs(3600))
    );
    assert_eq!(
        LivenessPolicy::for_profile(&env(EnvTag::Dev)).idle_disconnect,
        Some(secs(4 * 3600))
    );
    assert_eq!(
        LivenessPolicy::for_profile(&env(EnvTag::Local)).idle_disconnect,
        None
    );
    assert_eq!(
        LivenessPolicy::for_profile(&ssh_profile()).keepalive,
        Some(secs(60))
    );
    assert_eq!(
        LivenessPolicy::for_profile(&direct_profile()).keepalive,
        None
    );
}

// Beyond the brief.

#[test]
fn presets_for_test_staging_untagged_and_the_profiles_own_values() {
    for tag in [EnvTag::Test, EnvTag::Staging] {
        assert_eq!(
            LivenessPolicy::for_profile(&env(tag)).idle_disconnect,
            Some(secs(2 * 3600))
        );
    }
    assert_eq!(
        LivenessPolicy::for_profile(&direct_profile()).idle_disconnect,
        None
    );
    let own = Profile {
        keepalive_s: Some(15),
        idle_disconnect_s: Some(600),
        ..env(EnvTag::Production)
    };
    let policy = LivenessPolicy::for_profile(&own);
    assert_eq!(
        (policy.keepalive, policy.idle_disconnect, policy.tick),
        (Some(secs(15)), Some(secs(600)), secs(1))
    );
    // 0 turns either off, over SSH and in production too.
    let off = Profile {
        keepalive_s: Some(0),
        idle_disconnect_s: Some(0),
        ..Profile {
            environment: Some(EnvTag::Production),
            ..ssh_profile()
        }
    };
    let policy = LivenessPolicy::for_profile(&off);
    assert_eq!((policy.keepalive, policy.idle_disconnect), (None, None));
}

#[test]
fn a_failed_transaction_also_only_warns_and_activity_rearms_the_warning() {
    let mut s = sup(None, Some(60));
    let acts = run_ticks(&mut s, 200, TxStatus::Failed);
    assert_eq!(
        acts.0,
        [(60, LivenessAction::IdleTransaction)],
        "once per idle period"
    );
    // The user acts: a new idle period, a new warning.
    let _ = tick_busy(&mut s, 1);
    let acts = run_ticks(&mut s, 61, TxStatus::InTx);
    assert_eq!(acts.0, [(60, LivenessAction::IdleTransaction)]);
}

#[test]
fn a_jump_with_an_open_transaction_never_disconnects() {
    let mut s = sup(Some(60), Some(3600));
    idle_for(&mut s, secs(59));
    let mut acts = Vec::new();
    acts.extend(s.step(secs(1), secs(8 * 3600), false, TxStatus::InTx));
    acts.extend(s.step(secs(1), secs(1), false, TxStatus::InTx));
    assert_eq!(acts[0], LivenessAction::Suspect);
    assert!(!acts.contains(&LivenessAction::Disconnect), "{acts:?}");
}

#[test]
fn a_jump_with_an_open_transaction_and_no_keepalive_still_pings_once() {
    // Review H3 (M1): B18's "Suspect ⇒ one immediate ping" holds with a transaction open; only
    // the disconnect is reserved for an idle one.
    for tx in [TxStatus::InTx, TxStatus::Failed] {
        let mut s = sup(None, Some(3600));
        idle_for(&mut s, secs(59 * 60));
        let mut acts = Vec::new();
        acts.extend(s.step(secs(1), secs(8 * 3600), false, tx));
        acts.extend(s.step(secs(1), secs(1), false, tx));
        assert_eq!(acts, [LivenessAction::Suspect, LivenessAction::Ping], "{tx:?}");
    }
}

#[test]
fn small_drift_is_no_jump_and_busy_ticks_are_never_suspect() {
    let mut s = sup(Some(60), None);
    // Four seconds of drift: under the 5 s threshold.
    assert_eq!(s.step(secs(1), secs(5), false, TxStatus::Idle), None);
    // A long busy period ends: the request was activity, nothing is suspect.
    assert!(tick_busy(&mut s, 1).is_none());
    assert_eq!(s.step(secs(1), secs(1), false, TxStatus::Idle), None);
}
