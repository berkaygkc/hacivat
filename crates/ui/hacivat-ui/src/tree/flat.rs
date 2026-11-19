//! The expanded tree as one flat list of rows (pure; no GPUI).
//!
//! A [`FlatRow`] is 32 bytes, so a 160,000-row expanded tree is about 5 MB. [`Flat::build`]
//! walks the model iteratively (no recursion limit) and is called on a generation change;
//! [`Flat::expand_at`] and [`Flat::collapse_at`] splice one subtree in or out, so expanding a
//! node costs its own subtree, not the whole tree.

use std::hash::Hasher;
use std::sync::Arc;

pub use super::KeySet;
use super::{ChildState, TreeKey, TreeModel};

/// A hasher for [`TreeKey`]s: the keys are already stable 128-bit hashes, so their halves are
/// folded and spread with a multiply (Fibonacci hashing) instead of running SipHash over
/// 16 bytes. Looking 160,000 keys up in the expanded set is the flattening's hot loop. The high
/// half is spread before the fold (keys whose halves are swapped or equal stay apart), and the
/// product's high bits are folded into the low ones the table indexes by.
#[derive(Debug, Default, Clone, Copy)]
pub struct KeyHasher(u64);

/// 2^64 / φ, the Fibonacci-hashing multiplier.
const SPREAD: u64 = 11_400_714_819_323_198_485;

impl Hasher for KeyHasher {
    fn finish(&self) -> u64 {
        self.0
    }

    fn write(&mut self, bytes: &[u8]) {
        // Only reached for non-`u128` input (never for a `TreeKey`): FNV-1a.
        for b in bytes {
            self.0 = (self.0 ^ u64::from(*b)).wrapping_mul(1_099_511_628_211);
        }
    }

    fn write_u128(&mut self, v: u128) {
        let (lo, hi) = (v as u64, (v >> 64) as u64);
        let x = (self.0 ^ lo ^ hi.wrapping_mul(SPREAD).rotate_left(32)).wrapping_mul(SPREAD);
        self.0 = x ^ (x >> 32);
    }
}

/// What a flat row is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RowKind {
    /// A node of the model.
    Node,
    /// The "loading" child row of the node `key` (its parent).
    Loading,
    /// The "failed" child row of the node `key` (its parent).
    Failed,
}

/// One visible row of the expanded tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FlatRow {
    /// The node (for a loading or failed row: its parent).
    pub key: TreeKey,
    /// 0 for a root.
    pub depth: u16,
    pub kind: RowKind,
    /// 1-based position among its siblings.
    pub pos: u32,
    /// How many siblings it has, itself included.
    pub set_size: u32,
}

impl FlatRow {
    pub fn is_node(&self) -> bool {
        self.kind == RowKind::Node
    }
}

/// The flattened expanded tree.
#[derive(Debug, Clone, Default)]
pub struct Flat {
    rows: Vec<FlatRow>,
}

fn count(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

impl Flat {
    /// Flattens `model` under `expanded`. Expanded nodes whose children are
    /// [`ChildState::NotLoaded`] are appended to `not_loaded`.
    pub fn build<M: TreeModel + ?Sized>(
        model: &M,
        expanded: &KeySet,
        not_loaded: &mut Vec<TreeKey>,
    ) -> Flat {
        let mut rows = Vec::new();
        walk(model, model.roots(), 0, expanded, not_loaded, &mut rows);
        Flat { rows }
    }

    pub fn rows(&self) -> &[FlatRow] {
        &self.rows
    }

    pub fn len(&self) -> usize {
        self.rows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    pub fn get(&self, ix: usize) -> Option<&FlatRow> {
        self.rows.get(ix)
    }

    /// The row of node `key` (the first, if a key appears twice).
    pub fn index_of(&self, key: TreeKey) -> Option<usize> {
        self.rows.iter().position(|r| r.key == key && r.is_node())
    }

    /// The first index after row `ix` and its visible descendants.
    pub fn subtree_end(&self, ix: usize) -> usize {
        let Some(d) = self.rows.get(ix).map(|r| r.depth) else {
            return self.rows.len();
        };
        let mut j = ix + 1;
        while j < self.rows.len() && self.rows[j].depth > d {
            j += 1;
        }
        j
    }

    /// The row of `ix`'s parent node.
    pub fn parent_of(&self, ix: usize) -> Option<usize> {
        let d = self.rows.get(ix)?.depth;
        if d == 0 {
            return None;
        }
        (0..ix).rev().find(|&j| self.rows[j].depth < d)
    }

    /// Whether row `ix` shows its children (some row follows it one level deeper).
    pub fn is_open(&self, ix: usize) -> bool {
        match (self.rows.get(ix), self.rows.get(ix + 1)) {
            (Some(a), Some(b)) => b.depth > a.depth,
            _ => false,
        }
    }

    /// Splices node row `ix`'s children (and their expanded descendants) in after it, unless
    /// they are there already. Answers how many rows were inserted.
    pub fn expand_at<M: TreeModel + ?Sized>(
        &mut self,
        ix: usize,
        model: &M,
        expanded: &KeySet,
        not_loaded: &mut Vec<TreeKey>,
    ) -> usize {
        let Some(row) = self.rows.get(ix).copied() else {
            return 0;
        };
        if !row.is_node() || self.is_open(ix) {
            return 0;
        }
        let mut sub = Vec::new();
        children_rows(
            model,
            row.key,
            row.depth + 1,
            expanded,
            not_loaded,
            &mut sub,
        );
        let n = sub.len();
        self.rows.splice(ix + 1..ix + 1, sub);
        n
    }

    /// Removes row `ix`'s visible descendants. Answers how many rows were removed.
    pub fn collapse_at(&mut self, ix: usize) -> usize {
        let end = self.subtree_end(ix);
        let n = end.saturating_sub(ix + 1);
        if n > 0 {
            self.rows.drain(ix + 1..end);
        }
        n
    }
}

/// Appends the rows under `key` (whose own row is at `depth - 1`) to `out`.
fn children_rows<M: TreeModel + ?Sized>(
    model: &M,
    key: TreeKey,
    depth: u16,
    expanded: &KeySet,
    not_loaded: &mut Vec<TreeKey>,
    out: &mut Vec<FlatRow>,
) {
    let single = |kind| FlatRow {
        key,
        depth,
        kind,
        pos: 1,
        set_size: 1,
    };
    match model.children(key) {
        ChildState::Leaf => {}
        ChildState::NotLoaded => not_loaded.push(key),
        ChildState::Loading { .. } => out.push(single(RowKind::Loading)),
        ChildState::Failed { .. } => out.push(single(RowKind::Failed)),
        ChildState::Loaded(keys) => walk(model, keys, depth, expanded, not_loaded, out),
    }
}

/// Appends `keys` at `depth` and, depth first, every expanded descendant.
fn walk<M: TreeModel + ?Sized>(
    model: &M,
    keys: Arc<[TreeKey]>,
    depth: u16,
    expanded: &KeySet,
    not_loaded: &mut Vec<TreeKey>,
    out: &mut Vec<FlatRow>,
) {
    // (siblings, next position, their depth)
    let mut stack: Vec<(Arc<[TreeKey]>, usize, u16)> = vec![(keys, 0, depth)];
    while let Some((keys, next, d)) = stack.last_mut() {
        let Some(&key) = keys.get(*next) else {
            stack.pop();
            continue;
        };
        let (pos, size, d) = (*next, keys.len(), *d);
        *next += 1;
        out.push(FlatRow {
            key,
            depth: d,
            kind: RowKind::Node,
            pos: count(pos + 1),
            set_size: count(size),
        });
        if !expanded.contains(&key) {
            continue;
        }
        let child = d.saturating_add(1);
        match model.children(key) {
            ChildState::Loaded(kids) if !kids.is_empty() => stack.push((kids, 0, child)),
            ChildState::Loaded(_) | ChildState::Leaf => {}
            ChildState::NotLoaded => not_loaded.push(key),
            ChildState::Loading { .. } => out.push(FlatRow {
                key,
                depth: child,
                kind: RowKind::Loading,
                pos: 1,
                set_size: 1,
            }),
            ChildState::Failed { .. } => out.push(FlatRow {
                key,
                depth: child,
                kind: RowKind::Failed,
                pos: 1,
                set_size: 1,
            }),
        }
    }
}
