//! HV-MODEL-01 memory (alg/02 "Complexity / performance notes": ≤ 250 bytes per column object
//! and ≤ 600 bytes per table object, excluding facets; Inferred). Recorded, not gating: the gate
//! decides memory (M4 Task 3 Step 4).
//!
//! `HACIVAT_PERF=1 cargo test --release -p hacivat-meta --test model_perf -- --ignored
//! --nocapture`. The count is structural (no allocator hook: `unsafe` is confined to the UI
//! platform layer): the arena slot, the props allocation with its `Arc` header, and the
//! table's share of its namespace's name-index entry. Strings are interned or shared in real
//! catalogs and are counted separately.

use std::mem::size_of;
use std::time::Instant;

use hacivat_meta::fixture::{LARGE_SCHEMAS, large_pg_catalog};
use hacivat_meta::kind::ObjectKind;
use hacivat_meta::name::IdentKey;
use hacivat_meta::namespace::Obj;
use hacivat_meta::props::{ColumnProps, Props, TableProps};

fn perf_enabled() -> bool {
    std::env::var_os("HACIVAT_PERF").is_some_and(|v| v == "1")
}

/// The `Arc` header (strong and weak counts) in front of every props allocation.
const ARC_HEADER: usize = 2 * size_of::<usize>();

#[test]
#[ignore = "perf"]
fn memory_per_column_and_table_recorded() {
    if !perf_enabled() {
        return;
    }
    let t = Instant::now();
    let nss = large_pg_catalog();
    let build = t.elapsed();
    let columns: usize = nss.iter().map(|n| n.column_count()).sum();
    let tables: usize = nss
        .iter()
        .map(|n| n.iter_named(&[ObjectKind::Table]).count())
        .sum();
    let objects: usize = nss.iter().map(|n| n.len()).sum();
    assert_eq!(columns, 160_000);
    assert_eq!(tables, LARGE_SCHEMAS.iter().map(|s| s.1).sum::<usize>());
    assert_eq!(tables, 15_000);

    // Text bytes the catalog would hold if no string were shared (an upper bound).
    let mut column_text = 0usize;
    for n in &nss {
        for o in n.objs() {
            if let Props::Column(p) = o.props() {
                column_text += o.name().len() + p.raw_type.len();
                column_text += p.default_raw.as_ref().map_or(0, |s| s.len());
                column_text += p.comment.as_ref().map_or(0, |s| s.len());
            }
        }
    }

    let obj = size_of::<Obj>();
    let column = obj + size_of::<ColumnProps>() + ARC_HEADER;
    // A name-index entry: the key (an `Arc<str>` fat pointer), its `SmallVec<[u32; 2]>` and
    // the `HashMap` control byte, at the map's 7/8 load factor.
    let index_entry =
        (size_of::<IdentKey>() + size_of::<smallvec::SmallVec<[u32; 2]>>() + 1) * 8 / 7;
    let table = obj + size_of::<TableProps>() + ARC_HEADER + index_entry;

    println!("HV-MODEL-01 memory (structural, release):");
    println!(
        "  catalog: {} namespaces, {objects} objects, {tables} tables, {columns} columns; built in {:.1} ms",
        nss.len(),
        build.as_secs_f64() * 1e3
    );
    println!(
        "  size_of::<Obj>() = {obj} B, size_of::<ColumnProps>() = {} B, size_of::<TableProps>() = {} B, Arc header {ARC_HEADER} B",
        size_of::<ColumnProps>(),
        size_of::<TableProps>()
    );
    println!("  bytes per column (excluding shared strings): {column} (alg/02 target <= 250)");
    println!(
        "  bytes per table (excluding columns and shared strings): {table} (alg/02 target <= 600)"
    );
    println!(
        "  unshared column text, upper bound: {:.1} B per column",
        column_text as f64 / columns as f64
    );
}
