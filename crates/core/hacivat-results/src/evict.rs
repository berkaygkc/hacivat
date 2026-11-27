// ## m3b-t01c-core-seams
//! Eviction of background results (HV-RES-07 eviction, R-M3-68, P-M3b-29; Task 8): when the
//! global budget pauses a run, `MemoryBudget::evict_for` closes background results oldest
//! first (`CloseReason::Evicted`), each keeping its first chunk; a result visible in any
//! console is never evicted.
//!
//! The registry holds every store weakly with the last time it was visible, so a closed tab
//! never stays alive for it.
//!
//! Since Task 8:
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
//!   `Closed { reason: Evicted }` (Re-run brings the rest back), keeping the state it had
//!   ended in (`StoreSnapshot::before_eviction`); a snapshot taken before keeps every row it
//!   showed, so a copy or a frame in progress never changes under it.
//! - **Atomic with visibility.** The pass holds the registry's lock, so a tab shown during it
//!   is either shown first (and spared) or shown after (and finds the result evicted, which
//!   it says). The cut chunks are freed, and the budget's listeners told ([`Eviction`]),
//!   only after the lock is released.
//! - **Only as much as is needed.** The room is re-checked under the lock before each
//!   result, counting bytes already cut but not yet freed as free, so two runs over the
//!   budget at once do not both evict for the same shortfall.

// ## m3b-t08-store (module docs above, the clock, `Eviction`, `evict_while`)
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};

use crate::cursor::Completeness;
use crate::snapshot::StoredChunk;
use crate::store::{ResultId, ResultStore};

/// One eviction, as the budget's listeners hear of it (`MemoryBudget::on_evicted`): the run
/// service logs it (`LogMsg::Evicted { rows: kept_rows }`, Task 16).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Eviction {
    pub result: ResultId,
    /// The rows the result kept (its first chunk).
    pub kept_rows: u64,
    /// The state it had ended in before (also `StoreSnapshot::before_eviction`).
    pub previous: Completeness,
}

/// What one eviction pass cut, to be freed after the registry lock is released.
pub(crate) struct Pass {
    /// Each eviction and the cut chunks nobody else holds.
    pub(crate) evictions: Vec<(Eviction, Vec<StoredChunk>)>,
    /// The stores the pass upgraded: a closed tab's last reference may be among them, and it
    /// drops outside the lock too.
    pub(crate) stores: Vec<Arc<ResultStore>>,
}

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
    /// Cuts hidden, finished results, longest out of sight first, while `more(bytes cut so
    /// far)` holds; `more` runs under the lock before each result, so it sees every cut made
    /// before it (the module docs). Each cut chunk nobody else holds is added to `releasing`
    /// and handed back for the caller to free once the lock is released.
    pub(crate) fn evict_while(
        &self,
        mut more: impl FnMut(u64) -> bool,
        releasing: &AtomicU64,
    ) -> Pass {
        let mut pass = Pass {
            evictions: Vec::new(),
            stores: Vec::new(),
        };
        let entries = self.lock();
        let mut order: Vec<&Entry> = entries.list.iter().filter(|e| !e.visible).collect();
        order.sort_by_key(|e| e.last_visible);
        let mut cut = 0u64;
        for entry in order {
            if !more(cut) {
                break;
            }
            let Some(store) = entry.store.upgrade() else {
                continue;
            };
            if let Some((eviction, chunks)) = store.evict() {
                let bytes: u64 = chunks.iter().map(|c| c.chunk.bytes()).sum();
                releasing.fetch_add(bytes, Ordering::SeqCst);
                cut = cut.saturating_add(bytes);
                pass.evictions.push((eviction, chunks));
            }
            pass.stores.push(store);
        }
        pass
    }
}
