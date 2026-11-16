// ## m3b-t01c-core-seams
//! Out-of-band answers to a waiting run (P-M3b-3): the error-policy choice (`NeedsChoice`,
//! Task 11) and the confirm-in-transaction decision (`NeedsCommit`, Task 19).
//!
//! A run that waits sets the question in the session's one [`RunGate`] ([`RunGate::ask`],
//! which gives the question its id) and awaits [`RunGate::answered`] beside its stop signal
//! and its deadline. It emits the event carrying that id only after `ask` (no `.await`
//! between them). The answer comes as a `Command` (`Choose`, `CommitDecision`), never as a
//! queued `Request`: the actor answers it from `Ladder::command` while a request runs and from
//! its idle loop otherwise, so it never waits behind the request that asked (a queued
//! `Request` is never read by the run that waits for it). The slot works like `StopSignal`: a
//! mutex and a `Notify`, shared by the actor's control side and the running request.
//!
//! **Question identity** (T1c review I1). An answer counts only when it names the question
//! waiting now: its kind, the run's request sequence number, its question id (and, for a
//! commit, the unit). The id comes from a counter that only grows for the session's life, so
//! no two questions share one: a double click, an answer from an older bar that arrives after
//! the run asked again (also a Retry that re-asks for the same unit), or an answer naming any
//! other question is answered `false` and changes nothing (fail closed: nothing is sent on an
//! answer to a question the user never saw).

use std::sync::{Mutex, MutexGuard, PoisonError};

use tokio::sync::Notify;

use crate::session::{CommitAnswer, RunChoice, UnitId};

/// What a run asks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(not(test), allow(dead_code, reason = "Tasks 11 and 19 ask"))]
pub(crate) enum Ask {
    /// A unit failed under the `Ask` policy (Task 11).
    Choice,
    /// A confirmed statement waits for Commit/Roll back (Keep/Undo) (Task 19).
    Commit,
}

/// The question a run waits on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(not(test), allow(dead_code, reason = "Tasks 11 and 19 ask"))]
pub(crate) struct Pending {
    pub(crate) kind: Ask,
    /// The run's request sequence number (`RunTicket::seq`).
    pub(crate) run_seq: u64,
    pub(crate) unit: UnitId,
    /// The question's id (`RunEvent::{NeedsChoice, NeedsCommit}::question`).
    pub(crate) question: u64,
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
    /// The last question id given (ids start at 1).
    last_question: u64,
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

    /// A run of request `run_seq` starts waiting for an answer about `unit`; any earlier
    /// question (and an answer it was never given) is gone. Returns the new question's id,
    /// which the run's event carries.
    #[cfg_attr(not(test), allow(dead_code, reason = "Tasks 11 and 19 ask"))]
    pub(crate) fn ask(&self, kind: Ask, run_seq: u64, unit: UnitId) -> u64 {
        let mut slot = self.lock();
        slot.last_question = slot.last_question.wrapping_add(1).max(1);
        let question = slot.last_question;
        slot.pending = Some(Pending {
            kind,
            run_seq,
            unit,
            question,
        });
        slot.answer = None;
        question
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

    /// A `Command::Choose`: true when the run of request `run_seq` waits for a choice on
    /// question `question`.
    pub(crate) fn choose(&self, run_seq: u64, question: u64, choice: RunChoice) -> bool {
        self.answer(
            |p| p.kind == Ask::Choice && p.run_seq == run_seq && p.question == question,
            Answer::Choice(choice),
        )
    }

    /// A `Command::CommitDecision`: true when the run of request `run_seq` waits for the
    /// decision on `unit` as question `question`.
    pub(crate) fn commit(
        &self,
        run_seq: u64,
        unit: UnitId,
        question: u64,
        answer: CommitAnswer,
    ) -> bool {
        self.answer(
            |p| {
                p.kind == Ask::Commit
                    && p.run_seq == run_seq
                    && p.unit == unit
                    && p.question == question
            },
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
        assert!(!gate.choose(7, 1, RunChoice::Ignore), "nothing waits");
        let q = gate.ask(Ask::Choice, 7, unit(0));
        assert!(!gate.choose(8, q, RunChoice::Ignore), "another request");
        assert!(
            !gate.commit(7, unit(0), q, CommitAnswer::Commit),
            "another kind of question"
        );
        assert!(gate.choose(7, q, RunChoice::Stop));
        assert_eq!(gate.answered().await, Answer::Choice(RunChoice::Stop));

        let c = gate.ask(Ask::Commit, 9, unit(2));
        assert!(c > q, "ids only grow");
        assert!(
            !gate.commit(9, unit(1), c, CommitAnswer::Commit),
            "another unit"
        );
        assert_eq!(
            gate.pending(),
            Some(Pending {
                kind: Ask::Commit,
                run_seq: 9,
                unit: unit(2),
                question: c
            })
        );
        gate.withdraw();
        assert!(
            !gate.commit(9, unit(2), c, CommitAnswer::Commit),
            "withdrawn"
        );
        assert_eq!(gate.take_answer(), None);
    }

    /// A double click on one question: the second answer is refused and never reaches the
    /// run's next question.
    #[tokio::test]
    async fn a_duplicate_answer_is_refused() {
        let gate = RunGate::default();
        let q2 = gate.ask(Ask::Choice, 7, unit(2));
        assert!(gate.choose(7, q2, RunChoice::Ignore));
        assert!(
            !gate.choose(7, q2, RunChoice::IgnoreAll),
            "the double click"
        );
        assert_eq!(gate.answered().await, Answer::Choice(RunChoice::Ignore));
        // The run goes on, unit 3 fails within a round trip: the double click arrives late.
        let q3 = gate.ask(Ask::Choice, 7, unit(3));
        assert!(
            !gate.choose(7, q2, RunChoice::IgnoreAll),
            "late double click"
        );
        assert_eq!(
            gate.take_answer(),
            None,
            "unit 3's question stays unanswered"
        );
        assert_eq!(gate.pending().map(|p| p.question), Some(q3));
    }

    /// Retry re-asks for the same unit: an answer to the first question that arrives after the
    /// second was asked is stale, although it names the same request and unit.
    #[tokio::test]
    async fn a_stale_answer_after_a_retry_is_refused() {
        let gate = RunGate::default();
        let first = gate.ask(Ask::Choice, 7, unit(2));
        assert!(gate.choose(7, first, RunChoice::Retry));
        assert_eq!(gate.answered().await, Answer::Choice(RunChoice::Retry));
        let second = gate.ask(Ask::Choice, 7, unit(2));
        assert_ne!(first, second);
        assert!(!gate.choose(7, first, RunChoice::Ignore), "stale");
        assert_eq!(gate.take_answer(), None);
        assert!(gate.choose(7, second, RunChoice::Stop));
        assert_eq!(gate.answered().await, Answer::Choice(RunChoice::Stop));
    }

    /// While a newer question waits, an answer naming any other id is refused (an older one,
    /// a guessed later one), for choices and commits alike.
    #[tokio::test]
    async fn a_wrong_id_is_refused_while_a_newer_question_waits() {
        let gate = RunGate::default();
        let old = gate.ask(Ask::Commit, 7, unit(1));
        gate.withdraw();
        let new = gate.ask(Ask::Commit, 7, unit(1));
        for wrong in [old, new + 1, 0] {
            assert!(
                !gate.commit(7, unit(1), wrong, CommitAnswer::Commit),
                "{wrong}"
            );
        }
        assert_eq!(gate.take_answer(), None);
        assert!(gate.commit(7, unit(1), new, CommitAnswer::RollBack));
        let choice = gate.ask(Ask::Choice, 7, unit(4));
        assert!(!gate.choose(7, choice - 1, RunChoice::Ignore));
        assert!(gate.pending().is_some());
    }

    #[tokio::test]
    async fn a_waiting_run_wakes_on_the_answer() {
        let gate = std::sync::Arc::new(RunGate::default());
        let q = gate.ask(Ask::Commit, 3, unit(0));
        let waiter = tokio::spawn({
            let gate = gate.clone();
            async move { gate.answered().await }
        });
        tokio::task::yield_now().await;
        assert!(gate.commit(3, unit(0), q, CommitAnswer::RollBack));
        assert_eq!(
            waiter.await.unwrap(),
            Answer::Commit(CommitAnswer::RollBack)
        );
    }
}
