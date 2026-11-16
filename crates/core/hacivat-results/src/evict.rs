// ## m3b-t01c-core-seams
//! Eviction of background results (HV-RES-07 eviction, R-M3-68, P-M3b-29; Task 8): when the
//! global budget pauses a run, `MemoryBudget::evict_for` closes background results oldest
//! first (`CloseReason::Evicted`), each keeping its first chunk; a result visible in any
//! console is never evicted.
//!
//! The registry holds every store weakly with the last time it was visible, so a closed tab
//! never stays alive for it.
//!
//! ## m3b-t08-store
//! - **Who joins.** Every `ResultStore` joins its budget's registry when it is made
//!   (`ResultStore::new`), **visible**: a result is a candidate only once the app has said its
//!   tab is in the background (`StoreSource::set_visible(false)`). A console that never
//!   reports visibility therefore loses nothing; the cost is that its results are never
//!   evicted either (the run pauses at the global budget, as in M3a).
//! - **Who goes.** A hidden result whose run has ended (`Complete`, `Closed` for any reason
//!   but `Evicted`, `Cancelled`, `Truncated`) and that holds more than its first chunk. A
//!   result still being written (`Streaming`) or holding a server cursor (`Suspended`) is
//!   never taken: its session would keep appending, and only the session closes a cursor.
//! - **In what order.** Longest out of sight first: by the moment it was last hidden (or
//!   registered, if it never was shown), a logical clock so the order never ties.
//! - **What it does.** Cuts the result to its first chunk and publishes
//!   `Closed { reason: Evicted }` (Re-run brings the rest back); a snapshot taken before keeps
//!   every row it showed, so a copy or a frame in progress never changes under it.
//! - **Atomic with visibility.** The pass holds the registry's lock, so a tab shown during it
//!   is either shown first (and spared) or shown after (and finds the result evicted, which
//!   it says).

use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};

use crate::store::{ResultId, ResultStore};

/// One registered store.
#[derive(Debug)]
struct Entry {
    id: ResultId,
    store: Weak<ResultStore>,
    visible: bool,
    /// When it was last visible (or registered): a tick of the registry's clock.
    last_visible: u64,
}

/// The entries and the clock that orders them.
#[derive(Debug, Default)]
struct Entries {
    list: Vec<Entry>,
    clock: u64,
}

impl Entries {
    fn tick(&mut self) -> u64 {
        self.clock += 1;
        self.clock
    }
}

/// Every result that eviction may close, held weakly (the module docs).
#[derive(Debug, Default)]
pub struct EvictionRegistry {
    entries: Mutex<Entries>,
}

impl EvictionRegistry {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    fn lock(&self) -> MutexGuard<'_, Entries> {
        self.entries.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Registers `store`, visible until `set_visible(.., false)`; a dropped store leaves on
    /// the next registration.
    pub fn register(&self, store: &Arc<ResultStore>) {
        let mut entries = self.lock();
        entries.list.retain(|e| e.store.strong_count() > 0);
        let last_visible = entries.tick();
        entries.list.push(Entry {
            id: store.id(),
            store: Arc::downgrade(store),
            visible: true,
            last_visible,
        });
    }

    /// Result `id`'s tab became visible (or hidden).
    pub fn set_visible(&self, id: ResultId, visible: bool) {
        let mut entries = self.lock();
        let now = entries.tick();
        for e in entries.list.iter_mut().filter(|e| e.id == id) {
            if e.visible || visible {
                e.last_visible = now;
            }
            e.visible = visible;
        }
    }

    /// How many live stores are registered.
    pub fn len(&self) -> usize {
        self.lock()
            .list
            .iter()
            .filter(|e| e.store.strong_count() > 0)
            .count()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    // ## m3b-t08-store
    /// Evicts hidden, finished results, longest out of sight first, until `need` bytes have
    /// left the budget (the module docs); answers those bytes.
    pub(crate) fn evict_for(&self, need: u64) -> u64 {
        if need == 0 {
            return 0;
        }
        let entries = self.lock();
        let mut order: Vec<&Entry> = entries.list.iter().filter(|e| !e.visible).collect();
        order.sort_by_key(|e| e.last_visible);
        let mut released = 0u64;
        for entry in order {
            if released >= need {
                break;
            }
            if let Some(store) = entry.store.upgrade() {
                released = released.saturating_add(store.evict());
            }
        }
        released
    }
}
