//! `MemoryBudget` (Task 7): the global budget shared by every result and the per-result cap
//! (R-M3-19, R-M3-33). Eviction of background results since M3b Task 8 (`evict.rs`).
//!
//! Accounting is exact: a chunk charges its bytes when the writer appends it and gives them
//! back when its last holder drops it (the store's tail and every snapshot that still shows
//! it), so `used` is always the bytes of the chunks some reader can still reach.
//! Admission (`ResultWriter::admit`) reads `used` without reserving: two results admitted at
//! the same moment may each append one chunk past the budget, never more.

use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use crate::evict::{Eviction, EvictionRegistry};

// ## m3b-t08-store
/// Told of every eviction (`MemoryBudget::on_evicted`).
type EvictionListener = Arc<dyn Fn(&Eviction) + Send + Sync>;

/// The per-result cap: 256 MiB (R-M3-19).
pub const PER_RESULT_CAP: u64 = 256 << 20;

/// The app's memory budget for results.
pub struct MemoryBudget {
    global: u64,
    per_result: u64,
    used: AtomicU64,
    // ## m3b-t08-store
    /// Every result on this budget, held weakly with its visibility (`ResultStore::new`
    /// registers it).
    registry: Arc<EvictionRegistry>,
    /// Bytes eviction has cut and is about to drop (outside the registry lock): admission
    /// already counts them as free, so a run waiting on the lock does not evict again for them
    /// (review M1, M3).
    releasing: AtomicU64,
    listeners: Mutex<Vec<EvictionListener>>,
}

impl fmt::Debug for MemoryBudget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MemoryBudget")
            .field("global", &self.global)
            .field("per_result", &self.per_result)
            .field("used", &self.used())
            .finish()
    }
}

impl MemoryBudget {
    /// `global` bytes for every result together, `per_result` for each.
    pub fn new(global: u64, per_result: u64) -> Arc<Self> {
        Arc::new(MemoryBudget {
            global,
            per_result,
            used: AtomicU64::new(0),
            registry: EvictionRegistry::new(),
            releasing: AtomicU64::new(0),
            listeners: Mutex::new(Vec::new()),
        })
    }

    /// min(25 % of `ram`, 4 GiB); 1 GiB when the RAM is unknown; 256 MiB per result.
    pub fn from_ram(ram: Option<u64>) -> Arc<Self> {
        let global = ram.map_or(1 << 30, |ram| (ram / 4).min(4 << 30));
        Self::new(global, PER_RESULT_CAP)
    }

    /// The bytes every result holds now.
    pub fn used(&self) -> u64 {
        self.used.load(Ordering::SeqCst)
    }

    /// The global budget.
    pub fn global(&self) -> u64 {
        self.global
    }

    /// The cap of one result (before "Fetch more anyway").
    pub fn per_result(&self) -> u64 {
        self.per_result
    }

    // ## m3b-t01c-core-seams
    /// Evicts background results (not the visible tab of any console) oldest first, each
    /// keeping its first chunk, until `need` bytes would be released (R-M3-68, P-M3b-29; Task 8,
    /// through the `EvictionRegistry`). Answers the bytes released from the budget once the
    /// evicted chunks drop (a snapshot still holding them delays it).
    ///
    /// The answer counts the bytes that left [`used`](Self::used) during the call: a chunk a
    /// held snapshot still shows leaves it when that snapshot drops, and is not counted, so
    /// eviction goes on to the next result. Only results whose run has ended are taken (the
    /// rules are on `EvictionRegistry`). `need == 0` evicts nothing.
    pub fn evict_for(&self, need: u64) -> u64 {
        // ## m3b-t08-store
        if need == 0 {
            return 0;
        }
        self.evict_while(|cut| cut < need)
    }

    // ## m3b-t08-store
    /// Calls `listener` once for every result eviction cuts, after the cut chunks are freed
    /// and every eviction lock is released, on the thread that evicted (a session actor
    /// inside `admit`, or the caller of `evict_for`). The listener must not block: send the
    /// event on (the run service turns it into `LogMsg::Evicted`, Task 16).
    pub fn on_evicted(&self, listener: impl Fn(&Eviction) + Send + Sync + 'static) {
        self.listeners
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Arc::new(listener));
    }

    /// Whether a chunk of `bytes` would pass the global budget now, counting the bytes
    /// eviction is already freeing as free.
    pub(crate) fn over_global(&self, bytes: u64) -> bool {
        self.used()
            .saturating_sub(self.releasing.load(Ordering::SeqCst))
            .saturating_add(bytes)
            > self.global
    }

    /// Evicts until a chunk of `bytes` fits, re-checking the room before every result under
    /// the registry lock (review M1).
    pub(crate) fn evict_to_fit(&self, bytes: u64) -> u64 {
        self.evict_while(|_| self.over_global(bytes))
    }

    /// One eviction pass while `more(bytes cut so far)` holds: the registry cuts results under
    /// its lock; the cut chunks are dropped (releasing their bytes) and the listeners told
    /// after it is released (review M3), so `set_visible` on the UI thread never waits for a
    /// large free.
    fn evict_while(&self, more: impl FnMut(u64) -> bool) -> u64 {
        let pass = self.registry.evict_while(more, &self.releasing);
        let mut freed = 0u64;
        let mut events = Vec::with_capacity(pass.evictions.len());
        for (eviction, chunks) in pass.evictions {
            for chunk in chunks {
                let bytes = chunk.chunk.bytes();
                // `used` falls first, then `releasing`: admission may briefly count these bytes
                // twice as free (one chunk past the budget at most), never as held.
                drop(chunk);
                self.releasing.fetch_sub(bytes, Ordering::SeqCst);
                freed = freed.saturating_add(bytes);
            }
            events.push(eviction);
        }
        drop(pass.stores);
        if !events.is_empty() {
            let listeners = self
                .listeners
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone();
            for event in &events {
                for listener in &listeners {
                    listener(event);
                }
            }
        }
        freed
    }

    /// The registry the budget's results join.
    pub(crate) fn registry(&self) -> &Arc<EvictionRegistry> {
        &self.registry
    }

    /// A chunk of `bytes` joined a result.
    pub(crate) fn charge(&self, bytes: u64) {
        self.used.fetch_add(bytes, Ordering::SeqCst);
    }

    /// A chunk of `bytes` was dropped by its last holder.
    pub(crate) fn release(&self, bytes: u64) {
        let before = self.used.fetch_sub(bytes, Ordering::SeqCst);
        debug_assert!(
            before >= bytes,
            "released {bytes} bytes with {before} charged"
        );
    }
}
