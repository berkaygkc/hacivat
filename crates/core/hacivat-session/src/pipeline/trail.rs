//! The step trail: what the connect pipeline shows while it runs (HV-CONN-01), and the
//! recorder that keeps it and publishes each change.

use std::cmp::Ordering;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use tokio::sync::watch;

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

/// Records a connect's trail and publishes each change as [`Progress`]. It is shared with the
/// driver's event callback, which reports TLS and sign-in from the driver's own task.
#[derive(Clone)]
pub(crate) struct Recorder {
    inner: Arc<Mutex<RecorderState>>,
}

struct RecorderState {
    trail: Vec<StepTrace>,
    current: Option<TrailStep>,
    started: Instant,
    /// When each running step started, on the Tokio clock (so paused-clock tests agree).
    since: Vec<(TrailStep, tokio::time::Instant)>,
    tx: watch::Sender<Progress>,
    /// The trail is final (a failure): late driver events change nothing and publish nothing.
    sealed: bool,
}

impl RecorderState {
    fn entry(&mut self, step: TrailStep) -> Option<&mut StepTrace> {
        self.trail.iter_mut().find(|t| t.step == step)
    }

    fn publish(&self) {
        self.tx.send_replace(Progress {
            trail: self.trail.clone(),
            current: self.current,
            started: self.started,
        });
    }

    fn finish(&mut self, step: TrailStep, outcome: StepOutcome, elapsed: Option<Duration>) {
        if self.sealed {
            return;
        }
        let Some(t) = self.entry(step) else { return };
        t.outcome = outcome;
        t.elapsed = elapsed;
        self.since.retain(|(k, _)| *k != step);
        if self.current == Some(step) {
            self.current = None;
        }
        self.publish();
    }
}

impl Recorder {
    /// A trail of `plan`, every step pending, published at once.
    pub(crate) fn new(plan: &[TrailStep], tx: watch::Sender<Progress>) -> Self {
        let state = RecorderState {
            trail: plan
                .iter()
                .map(|&step| StepTrace {
                    step,
                    outcome: StepOutcome::Pending,
                    elapsed: None,
                })
                .collect(),
            current: None,
            started: Instant::now(),
            since: Vec::new(),
            tx,
            sealed: false,
        };
        state.publish();
        Recorder {
            inner: Arc::new(Mutex::new(state)),
        }
    }

    fn with<R>(&self, f: impl FnOnce(&mut RecorderState) -> R) -> R {
        let mut state = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        f(&mut state)
    }

    pub(crate) fn contains(&self, step: TrailStep) -> bool {
        self.with(|s| s.trail.iter().any(|t| t.step == step))
    }

    pub(crate) fn outcome(&self, step: TrailStep) -> Option<StepOutcome> {
        self.with(|s| s.trail.iter().find(|t| t.step == step).map(|t| t.outcome))
    }

    /// Marks `step` running from now. A step not in the plan is ignored.
    pub(crate) fn start(&self, step: TrailStep) {
        self.with(|s| {
            if s.sealed {
                return;
            }
            let Some(t) = s.entry(step) else { return };
            t.outcome = StepOutcome::Running;
            t.elapsed = None;
            s.since.retain(|(k, _)| *k != step);
            s.since.push((step, tokio::time::Instant::now()));
            s.current = Some(step);
            s.publish();
        });
    }

    /// Marks `step` done, timed from its `start` (untimed when it never started).
    pub(crate) fn done(&self, step: TrailStep) {
        self.with(|s| {
            let elapsed = s
                .since
                .iter()
                .find(|(k, _)| *k == step)
                .map(|(_, at)| at.elapsed());
            s.finish(step, StepOutcome::Done, elapsed);
        });
    }

    /// Marks `step` done without a duration (the driver did it and reported nothing).
    pub(crate) fn done_untimed(&self, step: TrailStep) {
        self.with(|s| s.finish(step, StepOutcome::Done, None));
    }

    pub(crate) fn skip(&self, step: TrailStep) {
        self.with(|s| s.finish(step, StepOutcome::Skipped, None));
    }

    /// The trail as it stands.
    pub(crate) fn snapshot(&self) -> Vec<StepTrace> {
        self.with(|s| s.trail.clone())
    }

    /// The trail of a failure at `at` (R25, the mock's `trailHTML`): the steps before it are
    /// done (a skipped step stays skipped), `at` failed, the rest pending. When `at` is `None`
    /// or not in the plan, the failure is at the step running now, else at the first step not
    /// yet done. Published as the final progress, then sealed.
    pub(crate) fn fail(&self, at: Option<TrailStep>) -> Vec<StepTrace> {
        self.with(|s| {
            if s.sealed {
                return s.trail.clone();
            }
            let position = |step: TrailStep| s.trail.iter().position(|t| t.step == step);
            let index = at
                .and_then(position)
                .or_else(|| s.current.and_then(position))
                .or_else(|| {
                    s.trail.iter().position(|t| {
                        !matches!(t.outcome, StepOutcome::Done | StepOutcome::Skipped)
                    })
                })
                .unwrap_or(s.trail.len().saturating_sub(1));
            for (i, t) in s.trail.iter_mut().enumerate() {
                match i.cmp(&index) {
                    Ordering::Less => {
                        if t.outcome != StepOutcome::Skipped {
                            t.outcome = StepOutcome::Done;
                        }
                    }
                    Ordering::Equal => {
                        t.outcome = StepOutcome::Failed;
                        t.elapsed = None;
                    }
                    Ordering::Greater => {
                        t.outcome = StepOutcome::Pending;
                        t.elapsed = None;
                    }
                }
            }
            s.current = None;
            s.since.clear();
            s.publish();
            s.sealed = true;
            s.trail.clone()
        })
    }

    /// Clears the trail (a `config.invalid` failure shows issues inline, R25), publishes it
    /// and seals it.
    pub(crate) fn clear(&self) {
        self.with(|s| {
            if s.sealed {
                return;
            }
            s.trail.clear();
            s.current = None;
            s.since.clear();
            s.publish();
            s.sealed = true;
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn outcomes(t: &[StepTrace]) -> Vec<(TrailStep, StepOutcome)> {
        t.iter().map(|s| (s.step, s.outcome)).collect()
    }

    #[tokio::test]
    async fn a_failure_marks_earlier_steps_done_and_later_ones_pending() {
        let (tx, rx) = watch::channel(Progress {
            trail: Vec::new(),
            current: None,
            started: Instant::now(),
        });
        let plan = [
            TrailStep::Resolve,
            TrailStep::Connect,
            TrailStep::Tls,
            TrailStep::SignIn,
        ];
        let r = Recorder::new(&plan, tx);
        r.start(TrailStep::Resolve);
        assert_eq!(rx.borrow().current, Some(TrailStep::Resolve));
        r.done(TrailStep::Resolve);
        r.skip(TrailStep::Tls);
        r.start(TrailStep::Connect);
        // A driver error classified as a sign-in failure lands on Sign in.
        let trail = r.fail(Some(TrailStep::SignIn));
        assert_eq!(
            outcomes(&trail),
            [
                (TrailStep::Resolve, StepOutcome::Done),
                (TrailStep::Connect, StepOutcome::Done),
                (TrailStep::Tls, StepOutcome::Skipped),
                (TrailStep::SignIn, StepOutcome::Failed),
            ]
        );
        assert!(trail[0].elapsed.is_some());
        assert_eq!(rx.borrow().trail, trail);
        assert_eq!(rx.borrow().current, None);

        // A failure at a step not in the plan lands on the step that was running.
        let (tx, _rx) = watch::channel(rx.borrow().clone());
        let r = Recorder::new(&plan, tx);
        r.done_untimed(TrailStep::Resolve);
        r.start(TrailStep::Connect);
        let trail = r.fail(Some(TrailStep::OpenFile));
        assert_eq!(trail[1].outcome, StepOutcome::Failed);
        assert_eq!(trail[3].outcome, StepOutcome::Pending);
    }

    #[tokio::test]
    async fn a_failed_trail_is_sealed_against_late_driver_events() {
        let (tx, mut rx) = watch::channel(Progress {
            trail: Vec::new(),
            current: None,
            started: Instant::now(),
        });
        let r = Recorder::new(&[TrailStep::Connect, TrailStep::SignIn], tx);
        let failed = r.fail(Some(TrailStep::Connect));
        rx.mark_unchanged();
        // A driver event that arrives after the failure (the late connection's task).
        r.start(TrailStep::SignIn);
        r.done(TrailStep::SignIn);
        r.skip(TrailStep::SignIn);
        r.clear();
        assert_eq!(r.snapshot(), failed);
        assert_eq!(r.fail(None), failed);
        assert!(!rx.has_changed().unwrap(), "nothing was published");
    }
}
