//! M3b Task 1c seams of the result store: eviction's stub and registry, the source's new
//! defaults, `CloseReason::Evicted`. Task 8 fills the bodies.

use std::sync::Arc;

use hacivat_results::{
    CloseReason, Completeness, EvictionRegistry, MemoryBudget, ResultId, ResultSource, ResultStore,
    SourceCaps, StoreSnapshot,
};
use hacivat_values::{Chunk, ColumnMeta, LogicalType, NativeType, ResultSchema, Value};

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

/// An evicted result reads as closed by eviction (Re-run brings it back; R-M3-68), never as a
/// result the memory limit closed, and keeps the rows it holds.
#[test]
fn an_evicted_store_reads_as_closed_by_eviction() {
    let budget = MemoryBudget::new(1 << 20, 1 << 20);
    let schema = Arc::new(ResultSchema {
        columns: vec![ColumnMeta {
            name: "n".into(),
            logical: LogicalType::Int {
                bits: 64,
                signed: true,
            },
            native: NativeType::Unknown,
            origin: None,
        }]
        .into(),
    });
    let (store, mut writer) = ResultStore::new(ResultId(1), schema.clone(), budget, 1);
    writer.append(Chunk::from_values(
        &schema,
        vec![vec![Value::Int(1)], vec![Value::Int(2)]],
    ));
    writer.set_state(Completeness::Closed {
        reason: CloseReason::Evicted,
    });
    let snap = store.snapshot();
    assert_eq!(
        *snap.completeness(),
        Completeness::Closed {
            reason: CloseReason::Evicted
        }
    );
    assert_ne!(
        *snap.completeness(),
        Completeness::Closed {
            reason: CloseReason::MemoryLimit
        }
    );
    assert_eq!(snap.row_count(), 2, "its rows stay");
}
