//! Connection budgets: how many sessions a profile may open, by priority (CONN-02, D-035).
//! Bodies in Task 16.

use futures::future::BoxFuture;

use crate::profile::Profile;

/// Who asks for a session, highest first (CONN-02).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Priority {
    Console,
    Editor,
    Job,
    Introspection,
}

/// A held slot of a [`Budget`]; returned on drop (CONN-02).
#[derive(Debug)]
pub struct BudgetPermit {
    _slot: (),
}

/// A profile's connection budget (CONN-02).
#[derive(Debug)]
pub struct Budget {
    max: u8,
}

impl Budget {
    pub fn new(max: u8) -> Self {
        Budget { max }
    }

    pub fn max(&self) -> u8 {
        self.max
    }

    /// Slots held now (Task 16).
    pub fn in_use(&self) -> u8 {
        0
    }

    /// A slot now, or `None` when the budget is spent (Task 16).
    pub fn try_acquire(&self, _p: Priority) -> Option<BudgetPermit> {
        None
    }

    /// A slot once one is free, higher priorities first (Task 16). The stub grants at once.
    pub fn acquire(&self, _p: Priority) -> BoxFuture<'static, BudgetPermit> {
        Box::pin(async { BudgetPermit { _slot: () } })
    }
}

/// `p`'s budget: `Budget::new(p.max_connections)`, which is 1 for SQLite (P4).
pub fn budget_for(p: &Profile) -> Budget {
    Budget::new(p.max_connections)
}
