//! M3b Task 1c seams of the result store: eviction's stub and registry, the source's new
//! defaults, `CloseReason::Evicted`. Task 8 fills the bodies.

use std::sync::Arc;

use hacivat_results::{
    CloseReason, EvictionRegistry, MemoryBudget, ResultId, ResultSource, ResultStore, SourceCaps,
    StoreSnapshot,
};
use hacivat_values::ResultSchema;

fn store(id: u64, budget: &Arc<MemoryBudget>) -> Arc<ResultStore> {
    let schema = Arc::new(ResultSchema {
        columns: Vec::new().into(),
    });
    ResultStore::new(ResultId(id), schema, budget.clone(), 1).0
}

#[test]
fn evict_stub_frees_nothing() {
    assert_eq!(MemoryBudget::new(1 << 20, 1 << 20).evict_for(1 << 19), 0);
}

/// A source that implements only the M3a methods.
struct M3aSource(Arc<ResultStore>);

impl ResultSource for M3aSource {
    fn snapshot(&self) -> Arc<StoreSnapshot> {
        self.0.snapshot()
    }
    fn changes(&self) -> tokio::sync::watch::Receiver<(u64, u64)> {
        self.0.changes()
    }
    fn ensure_rows(&self, _upto: u64) {}
    fn caps(&self) -> SourceCaps {
        SourceCaps::default()
    }
    fn fetch_all(&self) {}
    fn close_cursor(&self) {}
    fn raise_cap(&self) {
        self.0.raise_cap_to_global();
    }
}

#[test]
fn source_cap_raised_defaults_false() {
    let budget = MemoryBudget::new(1 << 20, 1 << 20);
    let source = M3aSource(store(1, &budget));
    assert!(!source.cap_raised());
    // The default ignores visibility; Task 8 makes `StoreSource` report both.
    source.set_visible(true);
    source.raise_cap();
    assert!(
        !source.cap_raised(),
        "the default answers false until Task 8"
    );
}

#[test]
fn registry_holds_stores_weakly() {
    let budget = MemoryBudget::new(1 << 20, 1 << 20);
    let registry = EvictionRegistry::new();
    let a = store(1, &budget);
    let b = store(2, &budget);
    registry.register(&a);
    registry.register(&b);
    registry.set_visible(ResultId(1), true);
    registry.set_visible(ResultId(9), true);
    assert_eq!(registry.len(), 2);
    drop(a);
    assert_eq!(registry.len(), 1, "a closed tab is not kept alive");
    drop(b);
    assert!(registry.is_empty());
}

#[test]
fn evicted_is_a_close_reason_of_its_own() {
    assert_ne!(CloseReason::Evicted, CloseReason::MemoryLimit);
}
