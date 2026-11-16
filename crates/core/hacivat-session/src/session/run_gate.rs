// ## m3b-t01c-core-seams
//! Out-of-band answers to a waiting run (P-M3b-3): the error-policy choice (`NeedsChoice`,
//! Task 11) and the confirm-in-transaction decision (`NeedsCommit`, Task 19).
//!
//! A run that waits sets the question in the session's one [`RunGate`] ([`RunGate::ask`]) and
//! awaits [`RunGate::answered`] beside its stop signal and its deadline. The answer comes as a
//! `Command` (`Choose`, `CommitDecision`), never as a queued `Request`: the actor answers it
//! from `Ladder::command` while a request runs and from its idle loop otherwise, so it never
//! waits behind the request that asked (a queued `Request` is never read by the run that waits
//! for it). The slot works like `StopSignal`: a mutex and a `Notify`, shared by the actor's
//! control side and the running request.
//!
//! An answer counts only when it names the question waiting now: a `Choose` the run's request
//! sequence number, a `CommitDecision` that and the unit. Anything else is answered `false`
//! and changes nothing; so is a second answer to one question.

use std::sync::{Mutex, MutexGuard, PoisonError};

use tokio::sync::Notify;

use crate::session::{CommitAnswer, RunChoice, UnitId};

/// The question a run waits on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(not(test), allow(dead_code, reason = "Tasks 11 and 19 ask"))]
pub(crate) enum Pending {
    /// A unit failed under the `Ask` policy (Task 11).
    Choice { run_seq: u64, unit: UnitId },
    /// A confirmed statement waits for Commit/Roll back (Keep/Undo) (Task 19).
    Commit { run_seq: u64, unit: UnitId },
}

/// The answer a run gets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Answer {
    Choice(RunChoice),
    Commit(CommitAnswer),
}

#[derive(Debug, Default)]
struct Slot {
    pending: Option<Pending>,
    answer: Option<Answer>,
}

/// The session's one slot for out-of-band answers (the module docs).
#[derive(Debug, Default)]
pub(crate) struct RunGate {
    slot: Mutex<Slot>,
    notify: Notify,
}

impl RunGate {
    fn lock(&self) -> MutexGuard<'_, Slot> {
        self.slot.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// A run starts waiting for an answer to `question`; any earlier question (and an answer
    /// it was never given) is gone.
    #[cfg_attr(not(test), allow(dead_code, reason = "Tasks 11 and 19 ask"))]
    pub(crate) fn ask(&self, question: Pending) {
        let mut slot = self.lock();
        slot.pending = Some(question);
        slot.answer = None;
    }

    /// The run stops waiting (a stop, its deadline, the end of its request): a later answer
    /// is `false`.
    pub(crate) fn withdraw(&self) {
        let mut slot = self.lock();
        slot.pending = None;
        slot.answer = None;
    }

    /// The question waiting now.
    #[cfg_attr(not(test), allow(dead_code, reason = "Tasks 11 and 19 read it"))]
    pub(crate) fn pending(&self) -> Option<Pending> {
        self.lock().pending
    }

    /// Takes the answer once it was given.
    pub(crate) fn take_answer(&self) -> Option<Answer> {
        self.lock().answer.take()
    }

    /// Waits until the question is answered (cancel-safe: a dropped wait loses nothing).
    #[cfg_attr(not(test), allow(dead_code, reason = "Tasks 11 and 19 wait"))]
    pub(crate) async fn answered(&self) -> Answer {
        loop {
            if let Some(answer) = self.take_answer() {
                return answer;
            }
            // `notify_one` keeps a permit when nobody waits: an answer given between the check
            // and this wait still wakes it.
            self.notify.notified().await;
        }
    }

    /// Fills the slot when `matches` the waiting question; true when it did.
    fn answer(&self, matches: impl FnOnce(Pending) -> bool, answer: Answer) -> bool {
        let filled = {
            let mut slot = self.lock();
            match slot.pending {
                Some(p) if matches(p) => {
                    slot.pending = None;
                    slot.answer = Some(answer);
                    true
                }
                _ => false,
            }
        };
        if filled {
            self.notify.notify_one();
        }
        filled
    }

    /// A `Command::Choose`: true when a run of request `run_seq` waited for a choice.
    pub(crate) fn choose(&self, run_seq: u64, choice: RunChoice) -> bool {
        self.answer(
            |p| matches!(p, Pending::Choice { run_seq: s, .. } if s == run_seq),
            Answer::Choice(choice),
        )
    }

    /// A `Command::CommitDecision`: true when a run of request `run_seq` waited for the
    /// decision on `unit`.
    pub(crate) fn commit(&self, run_seq: u64, unit: UnitId, answer: CommitAnswer) -> bool {
        self.answer(
            |p| matches!(p, Pending::Commit { run_seq: s, unit: u } if s == run_seq && u == unit),
            Answer::Commit(answer),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::RunId;

    fn unit(ordinal: u32) -> UnitId {
        UnitId {
            run: RunId(1),
            ordinal,
        }
    }

    #[tokio::test]
    async fn an_answer_counts_only_for_the_waiting_question() {
        let gate = RunGate::default();
        assert!(!gate.choose(7, RunChoice::Ignore), "nothing waits");
        gate.ask(Pending::Choice {
            run_seq: 7,
            unit: unit(0),
        });
        assert!(!gate.choose(8, RunChoice::Ignore), "another request");
        assert!(
            !gate.commit(7, unit(0), CommitAnswer::Commit),
            "another question"
        );
        assert!(gate.choose(7, RunChoice::Stop));
        assert!(!gate.choose(7, RunChoice::Ignore), "answered once");
        assert_eq!(gate.answered().await, Answer::Choice(RunChoice::Stop));

        gate.ask(Pending::Commit {
            run_seq: 9,
            unit: unit(2),
        });
        assert!(
            !gate.commit(9, unit(1), CommitAnswer::Commit),
            "another unit"
        );
        assert_eq!(
            gate.pending(),
            Some(Pending::Commit {
                run_seq: 9,
                unit: unit(2)
            })
        );
        gate.withdraw();
        assert!(!gate.commit(9, unit(2), CommitAnswer::Commit), "withdrawn");
        assert_eq!(gate.take_answer(), None);
    }

    #[tokio::test]
    async fn a_waiting_run_wakes_on_the_answer() {
        let gate = std::sync::Arc::new(RunGate::default());
        gate.ask(Pending::Commit {
            run_seq: 3,
            unit: unit(0),
        });
        let waiter = tokio::spawn({
            let gate = gate.clone();
            async move { gate.answered().await }
        });
        tokio::task::yield_now().await;
        assert!(gate.commit(3, unit(0), CommitAnswer::RollBack));
        assert_eq!(
            waiter.await.unwrap(),
            Answer::Commit(CommitAnswer::RollBack)
        );
    }
}
