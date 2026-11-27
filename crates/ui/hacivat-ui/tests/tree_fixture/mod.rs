//! A tree model for the tree tests: an outline parsed from indented text, keys numbered from 1
//! in reading order.
//!
//! - `name` — a leaf; a line with deeper lines below it — a loaded parent;
//! - `name?` — children not loaded yet;
//! - `#Name 12` — a group header (label `Name`, meta `12`).
#![allow(dead_code)]

use std::collections::HashMap;
use std::sync::Arc;

use hacivat_ui::text::TextSource;
use hacivat_ui::tree::{ChildState, TreeKey, TreeModel, TreeRowData};

#[derive(Debug, Clone)]
pub struct Node {
    pub row: TreeRowData,
    pub children: ChildState,
}

#[derive(Debug, Clone, Default)]
pub struct Fixture {
    pub nodes: HashMap<TreeKey, Node>,
    pub roots: Vec<TreeKey>,
    pub generation: u64,
    next: u128,
}

impl Fixture {
    pub fn parse(outline: &str) -> Fixture {
        let mut f = Fixture {
            next: 1,
            ..Default::default()
        };
        // (indent, key) of the open parents.
        let mut stack: Vec<(usize, TreeKey)> = Vec::new();
        let mut kids: HashMap<TreeKey, Vec<TreeKey>> = HashMap::new();
        for line in outline.lines().filter(|l| !l.trim().is_empty()) {
            let indent = line.len() - line.trim_start().len();
            let text = line.trim();
            while stack.last().is_some_and(|(i, _)| *i >= indent) {
                stack.pop();
            }
            let key = TreeKey(f.next);
            f.next += 1;
            let (row, children) = if let Some(h) = text.strip_prefix('#') {
                let (label, meta) = h.split_once(' ').unwrap_or((h, ""));
                let mut row = TreeRowData::new(label.to_string(), TextSource::Fluent);
                row.group_header = true;
                row.meta = (!meta.is_empty()).then(|| meta.to_string().into());
                (row, ChildState::Leaf)
            } else if let Some(name) = text.strip_suffix('?') {
                (
                    TreeRowData::new(name.to_string(), TextSource::Catalog),
                    ChildState::NotLoaded,
                )
            } else {
                (
                    TreeRowData::new(text.to_string(), TextSource::Catalog),
                    ChildState::Leaf,
                )
            };
            f.nodes.insert(key, Node { row, children });
            match stack.last() {
                Some((_, parent)) => kids.entry(*parent).or_default().push(key),
                None => f.roots.push(key),
            }
            stack.push((indent, key));
        }
        for (parent, keys) in kids {
            f.nodes.get_mut(&parent).unwrap().children = ChildState::Loaded(keys.into());
        }
        f
    }

    /// The key of the node labelled `label`.
    pub fn key(&self, label: &str) -> TreeKey {
        let mut found: Vec<_> = self
            .nodes
            .iter()
            .filter(|(_, n)| n.row.label.as_ref() == label)
            .map(|(k, _)| *k)
            .collect();
        found.sort();
        *found.first().unwrap_or_else(|| panic!("no node {label}"))
    }

    pub fn set_children(&mut self, key: TreeKey, children: ChildState) {
        self.nodes.get_mut(&key).unwrap().children = children;
        self.generation += 1;
    }

    /// Adds a new leaf `label` under `parent` (or as a root) at `at`.
    pub fn insert(&mut self, parent: Option<TreeKey>, at: usize, label: &str) -> TreeKey {
        let key = TreeKey(self.next);
        self.next += 1;
        self.nodes.insert(
            key,
            Node {
                row: TreeRowData::new(label.to_string(), TextSource::Catalog),
                children: ChildState::Leaf,
            },
        );
        match parent {
            None => self.roots.insert(at, key),
            Some(p) => {
                let node = self.nodes.get_mut(&p).unwrap();
                let mut keys = match &node.children {
                    ChildState::Loaded(k) => k.to_vec(),
                    _ => Vec::new(),
                };
                keys.insert(at, key);
                node.children = ChildState::Loaded(keys.into());
            }
        }
        self.generation += 1;
        key
    }

    pub fn row_mut(&mut self, key: TreeKey) -> &mut TreeRowData {
        self.generation += 1;
        &mut self.nodes.get_mut(&key).unwrap().row
    }

    /// `n` root leaves `r0`, `r1`, ….
    pub fn flat_roots(n: usize) -> Fixture {
        let mut f = Fixture {
            next: 1,
            ..Default::default()
        };
        for i in 0..n {
            let key = TreeKey(f.next);
            f.next += 1;
            f.nodes.insert(
                key,
                Node {
                    row: TreeRowData::new(format!("r{i}"), TextSource::Catalog),
                    children: ChildState::Leaf,
                },
            );
            f.roots.push(key);
        }
        f
    }
}

impl TreeModel for Fixture {
    fn roots(&self) -> Arc<[TreeKey]> {
        self.roots.clone().into()
    }

    fn row(&self, key: TreeKey) -> Option<TreeRowData> {
        self.nodes.get(&key).map(|n| n.row.clone())
    }

    fn children(&self, key: TreeKey) -> ChildState {
        self.nodes
            .get(&key)
            .map_or(ChildState::Leaf, |n| n.children.clone())
    }

    fn generation(&self) -> u64 {
        self.generation
    }
}
