//! Connection budgets: how many sessions a profile may open, by priority (CONN-02, D-035, R13).
//!
//! A [`Budget`] hands out at most `max` [`BudgetPermit`]s. A request that finds the budget spent
//! queues; when a permit is returned it goes straight to the waiting request with the highest
//! priority (first come, first served within a priority), so a freed slot is never raced for.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use futures::future::BoxFuture;
use tokio::sync::oneshot;

use crate::profile::Profile;

/// Who asks for a session, highest first (CONN-02).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Priority {
    Console,
    Editor,
    Job,
    Introspection,
}

/// The shared state of a budget.
struct Inner {
    max: u8,
    state: Mutex<State>,
}

struct State {
    in_use: u8,
    next_seq: u64,
    /// Waiting requests, highest priority first, then in arrival order.
    waiters: BTreeMap<(Priority, u64), oneshot::Sender<BudgetPermit>>,
}

impl Inner {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// A permit is coming back: hand it to the best waiter still listening, or free the slot.
    fn release(self: &Arc<Self>) {
        loop {
            let mut state = self.lock();
            let Some((_, tx)) = state.waiters.pop_first() else {
                state.in_use = state.in_use.saturating_sub(1);
                return;
            };
            drop(state);
            if Self::hand_over(self, tx) {
                return;
            }
        }
    }

    /// Sends a fresh permit for the held slot; `false` when the waiter gave up.
    fn hand_over(this: &Arc<Self>, tx: oneshot::Sender<BudgetPermit>) -> bool {
        match tx.send(BudgetPermit::new(this.clone())) {
            Ok(()) => true,
            Err(mut permit) => {
                // The slot stays held by the caller of `release`; this permit must not return it.
                permit.budget = None;
                false
            }
        }
    }
}

/// A held slot of a [`Budget`]; returned on drop (CONN-02).
pub struct BudgetPermit {
    budget: Option<Arc<Inner>>,
}

impl BudgetPermit {
    fn new(budget: Arc<Inner>) -> Self {
        BudgetPermit {
            budget: Some(budget),
        }
    }
}

impl Drop for BudgetPermit {
    fn drop(&mut self) {
        if let Some(budget) = self.budget.take() {
            budget.release();
        }
    }
}

impl fmt::Debug for BudgetPermit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("BudgetPermit")
    }
}

/// A profile's connection budget (CONN-02).
pub struct Budget {
    inner: Arc<Inner>,
}

impl fmt::Debug for Budget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Budget")
            .field("max", &self.inner.max)
            .field("in_use", &self.in_use())
            .finish()
    }
}

impl Budget {
    pub fn new(max: u8) -> Self {
        Budget {
            inner: Arc::new(Inner {
                max,
                state: Mutex::new(State {
                    in_use: 0,
                    next_seq: 0,
                    waiters: BTreeMap::new(),
                }),
            }),
        }
    }

    pub fn max(&self) -> u8 {
        self.inner.max
    }

    /// Slots held now.
    pub fn in_use(&self) -> u8 {
        self.inner.lock().in_use
    }

    // ## m2b-t15-session-manager (named exception, fix round 1 C1)
    /// The priority of the best request still waiting for a slot, if any (a waiter that gave
    /// up does not count): the session pool closes a released session instead of parking it
    /// when someone waits.
    pub fn first_waiter(&self) -> Option<Priority> {
        self.inner
            .lock()
            .waiters
            .iter()
            .find(|(_, tx)| !tx.is_closed())
            .map(|((p, _), _)| *p)
    }

    /// A slot now, or `None` when the budget is spent.
    pub fn try_acquire(&self, _p: Priority) -> Option<BudgetPermit> {
        let mut state = self.inner.lock();
        if state.in_use < self.inner.max && state.waiters.is_empty() {
            state.in_use += 1;
            Some(BudgetPermit::new(self.inner.clone()))
        } else {
            None
        }
    }

    /// A slot once one is free, higher priorities first. The request queues when this is
    /// called, not when the future is first polled, so the order of calls is the order of
    /// service within a priority. Dropping the future gives up the place (or the slot, if it
    /// was already handed over).
    pub fn acquire(&self, p: Priority) -> BoxFuture<'static, BudgetPermit> {
        let mut state = self.inner.lock();
        if state.in_use < self.inner.max && state.waiters.is_empty() {
            state.in_use += 1;
            let permit = BudgetPermit::new(self.inner.clone());
            return Box::pin(async move { permit });
        }
        let (tx, rx) = oneshot::channel();
        let seq = state.next_seq;
        state.next_seq += 1;
        state.waiters.insert((p, seq), tx);
        drop(state);
        // Holding the budget keeps the waiter's sender alive until a permit is sent.
        let budget = self.inner.clone();
        Box::pin(async move {
            let _budget = budget;
            match rx.await {
                Ok(permit) => permit,
                Err(_) => std::future::pending().await,
            }
        })
    }
}

/// `p`'s budget (P4, D-035): always 1 for SQLite (one writer; WAL's 1 writer + 2 readers come
/// later), else `p.max_connections` but at least 1, so a profile saying 0 cannot make every
/// connect wait forever.
pub fn budget_for(p: &Profile) -> Budget {
    if p.engine == hacivat_proto::DbmsKind::Sqlite {
        Budget::new(1)
    } else {
        Budget::new(p.max_connections.max(1))
    }
}

// ## m2b-t15-session-manager
#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn first_waiter_names_the_best_live_waiter() {
        let b = Budget::new(1);
        let held = b.try_acquire(Priority::Console).unwrap();
        assert_eq!(b.first_waiter(), None);
        let intro = b.acquire(Priority::Introspection);
        assert_eq!(b.first_waiter(), Some(Priority::Introspection));
        let console = b.acquire(Priority::Console);
        assert_eq!(b.first_waiter(), Some(Priority::Console));
        // A waiter that gave up no longer counts.
        drop(console);
        assert_eq!(b.first_waiter(), Some(Priority::Introspection));
        drop(intro);
        assert_eq!(b.first_waiter(), None);
        drop(held);
        assert_eq!(b.in_use(), 0);
    }
}
