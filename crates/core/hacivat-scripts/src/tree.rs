//! The sorted entry tree a scan produces (folders first; HV-SCRIPT-03). Task 4 writes it.
//!
//! ## m4b-t01a-seams: declared by Task 1a so later tasks add no `lib.rs` line.
//!
//! ## m4b-t04-scanner
//! A pre-order walk of the tree as a flat list: every folder before its contents, and among
//! siblings folders first, then by the exact bytes of the name. The order is stable and
//! independent of the locale; display order (I18N-10 collation, NFC) is the app's.

use std::cmp::Ordering;

use crate::scan::{EntryKind, ScriptEntry};

/// Sorts `entries` into tree order ([`order`]).
pub fn sort(entries: &mut [ScriptEntry]) {
    entries.sort_by(order);
}

/// Tree order: an ancestor before its descendants; at the first differing component, a folder
/// before a file, then the component's bytes.
pub fn order(a: &ScriptEntry, b: &ScriptEntry) -> Ordering {
    let mut pa = a.rel.as_str().split('/');
    let mut pb = b.rel.as_str().split('/');
    loop {
        match (pa.next(), pb.next()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(x), Some(y)) if x == y => {}
            (Some(x), Some(y)) => {
                let a_folder = pa.next().is_some() || a.kind == EntryKind::Folder;
                let b_folder = pb.next().is_some() || b.kind == EntryKind::Folder;
                return b_folder
                    .cmp(&a_folder)
                    .then_with(|| x.as_bytes().cmp(y.as_bytes()));
            }
        }
    }
}
// ## m4b-t04-scanner
