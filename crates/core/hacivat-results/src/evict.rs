// ## m3b-t01c-core-seams
//! Eviction of background results (HV-RES-07 eviction, R-M3-68, P-M3b-29; Task 8): when the
//! global budget pauses a run, `MemoryBudget::evict_for` closes background results oldest
//! first (`CloseReason::Evicted`), each keeping its first chunk; a result visible in any
//! console is never evicted.
//!
//! The registry holds every store weakly with the last time it was visible, so a closed tab
//! never stays alive for it. STUB until Task 8: it records stores and visibility; nothing reads
//! it to evict yet (`MemoryBudget::evict_for` answers 0).

use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
use std::time::Instant;

use crate::store::{ResultId, ResultStore};

/// One registered store.
#[derive(Debug)]
struct Entry {
    id: ResultId,
    store: Weak<ResultStore>,
    visible: bool,
    /// When it was last visible (or registered).
    last_visible: Instant,
}

/// Every result that eviction may close, held weakly (the module docs).
#[derive(Debug, Default)]
pub struct EvictionRegistry {
    entries: Mutex<Vec<Entry>>,
}

impl EvictionRegistry {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    fn lock(&self) -> MutexGuard<'_, Vec<Entry>> {
        self.entries.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Registers `store` (hidden until `set_visible`); a dropped store leaves on the next
    /// registration.
    pub fn register(&self, store: &Arc<ResultStore>) {
        let mut entries = self.lock();
        entries.retain(|e| e.store.strong_count() > 0);
        entries.push(Entry {
            id: store.id(),
            store: Arc::downgrade(store),
            visible: false,
            last_visible: Instant::now(),
        });
    }

    /// Result `id`'s tab became visible (or hidden).
    pub fn set_visible(&self, id: ResultId, visible: bool) {
        if let Some(e) = self.lock().iter_mut().find(|e| e.id == id) {
            if e.visible || visible {
                e.last_visible = Instant::now();
            }
            e.visible = visible;
        }
    }

    /// How many live stores are registered.
    pub fn len(&self) -> usize {
        self.lock()
            .iter()
            .filter(|e| e.store.strong_count() > 0)
            .count()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}
