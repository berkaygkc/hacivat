//! Debounced, coalesced saves (HV-MODEL-03 step 4): after a publish the hub offers each
//! changed namespace; a namespace is saved at most once per [`SAVE_DEBOUNCE`] after its first
//! pending offer, with the latest snapshot offered by then. Pure logic over caller-supplied
//! instants, so the hub drives it from its own clock and tests need no timers.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::ids::NsKey;
use crate::namespace::NamespaceSnapshot;

/// The save delay per namespace (alg/02 HV-MODEL-03 step 4: 1 s).
pub const SAVE_DEBOUNCE: Duration = Duration::from_secs(1);

/// Pending saves, one per namespace.
#[derive(Debug)]
pub struct SaveDebouncer {
    delay: Duration,
    pending: HashMap<NsKey, (Arc<NamespaceSnapshot>, Instant)>,
}

impl Default for SaveDebouncer {
    fn default() -> Self {
        SaveDebouncer::new(SAVE_DEBOUNCE)
    }
}

impl SaveDebouncer {
    pub fn new(delay: Duration) -> Self {
        SaveDebouncer {
            delay,
            pending: HashMap::new(),
        }
    }

    /// Offers `ns` for saving at `now`: it replaces a pending snapshot of the same namespace
    /// (coalescing) and keeps that one's due time, so a namespace that keeps changing is still
    /// saved once per delay. A temp namespace is never queued.
    pub fn offer(&mut self, ns: Arc<NamespaceSnapshot>, now: Instant) {
        if ns.is_temp() {
            return;
        }
        let due = now + self.delay;
        self.pending
            .entry(ns.key().clone())
            .and_modify(|(pending, _)| *pending = ns.clone())
            .or_insert((ns, due));
    }

    /// Drops a pending save (the namespace was removed).
    pub fn cancel(&mut self, key: &NsKey) {
        self.pending.remove(key);
    }

    /// The snapshots due at `now`, removed from the queue, in namespace-key order.
    pub fn take_due(&mut self, now: Instant) -> Vec<Arc<NamespaceSnapshot>> {
        let mut due: Vec<NsKey> = self
            .pending
            .iter()
            .filter(|(_, (_, at))| *at <= now)
            .map(|(k, _)| k.clone())
            .collect();
        due.sort();
        due.into_iter()
            .filter_map(|k| self.pending.remove(&k).map(|(ns, _)| ns))
            .collect()
    }

    /// Every pending snapshot (shutdown flush), in namespace-key order.
    pub fn take_all(&mut self) -> Vec<Arc<NamespaceSnapshot>> {
        let mut all: Vec<_> = self.pending.drain().collect();
        all.sort_by(|a, b| a.0.cmp(&b.0));
        all.into_iter().map(|(_, (ns, _))| ns).collect()
    }

    /// When the next save falls due, if any is pending.
    pub fn next_due(&self) -> Option<Instant> {
        self.pending.values().map(|(_, at)| *at).min()
    }

    pub fn len(&self) -> usize {
        self.pending.len()
    }

    pub fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }
}
