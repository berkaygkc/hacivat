//! The tree's frame budget (release, `HACIVAT_PERF=1`): a 162,041-node tree, fully expanded
//! (1 database, 20 schemas, a "Tables" group each, 100 tables each, 80 columns each), scrolled
//! in jumps and small steps. Each frame builds, lays out and prepaints its rows in under 2 ms
//! at the 95th percentile; the full flatten stays under 50 ms.

use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::{SharedString, TestAppContext, VisualTestContext, px, size};
use hacivat_ui::icon::IconName;
use hacivat_ui::text::TextSource;
use hacivat_ui::tree::{ChildState, TreeKey, TreeModel, TreeRowData, TreeView};
use hacivat_ui_core::theme::{Theme, ThemeMode};

struct Node {
    label: SharedString,
    meta: Option<SharedString>,
    icon: Option<IconName>,
    header: bool,
    children: ChildState,
}

/// Keys are indices + 1.
struct Big {
    nodes: Vec<Node>,
    roots: Arc<[TreeKey]>,
}

impl Big {
    fn new(schemas: usize, tables: usize, columns: usize) -> Big {
        let mut nodes: Vec<Node> = Vec::new();
        let mut add = |label: String, meta: Option<String>, icon, header| {
            nodes.push(Node {
                label: label.into(),
                meta: meta.map(Into::into),
                icon,
                header,
                children: ChildState::Leaf,
            });
            TreeKey(nodes.len() as u128)
        };
        let types = ["int4", "text", "numeric(12,2)", "timestamptz", "bool"];
        let words = ["çağrı", "ığdır", "şişli", "öğrenci", "İstanbul", "üzüm"];
        let db = add("hacivat_demo".into(), None, Some(IconName::Database), false);
        let mut db_kids = Vec::new();
        let mut links: Vec<(TreeKey, Vec<TreeKey>)> = Vec::new();
        for s in 0..schemas {
            let schema = add(format!("schema_{s}"), None, Some(IconName::Layers), false);
            db_kids.push(schema);
            let group = add("Tables".into(), Some(format!("{tables}")), None, true);
            links.push((schema, vec![group]));
            let mut ts = Vec::new();
            for t in 0..tables {
                let table = add(
                    format!("{}_{t}", words[t % words.len()]),
                    None,
                    Some(IconName::Table2),
                    false,
                );
                ts.push(table);
                let cols = (0..columns)
                    .map(|c| {
                        add(
                            format!("col_{c}_{}", words[(c + t) % words.len()]),
                            Some(types[c % types.len()].into()),
                            None,
                            false,
                        )
                    })
                    .collect();
                links.push((table, cols));
            }
            links.push((group, ts));
        }
        links.push((db, db_kids));
        for (parent, kids) in links {
            nodes[(parent.0 - 1) as usize].children = ChildState::Loaded(kids.into());
        }
        Big {
            nodes,
            roots: vec![db].into(),
        }
    }

    fn parents(&self) -> Vec<TreeKey> {
        self.nodes
            .iter()
            .enumerate()
            .filter(|(_, n)| matches!(n.children, ChildState::Loaded(_)))
            .map(|(i, _)| TreeKey(i as u128 + 1))
            .collect()
    }
}

impl TreeModel for Big {
    fn roots(&self) -> Arc<[TreeKey]> {
        self.roots.clone()
    }

    fn row(&self, key: TreeKey) -> Option<TreeRowData> {
        let n = self.nodes.get((key.0 as usize).checked_sub(1)?)?;
        let mut row = TreeRowData::new(n.label.clone(), TextSource::Catalog);
        row.meta = n.meta.clone();
        row.icon = n.icon;
        row.group_header = n.header;
        Some(row)
    }

    fn children(&self, key: TreeKey) -> ChildState {
        (key.0 as usize)
            .checked_sub(1)
            .and_then(|i| self.nodes.get(i))
            .map_or(ChildState::Leaf, |n| n.children.clone())
    }

    fn generation(&self) -> u64 {
        0
    }
}

fn draw(vcx: &mut VisualTestContext) {
    vcx.update(|window, _| window.refresh());
    vcx.run_until_parked();
}

fn p95(mut v: Vec<Duration>) -> Duration {
    v.sort();
    v[v.len() * 95 / 100]
}

#[gpui::test]
#[ignore = "perf"]
fn tree_scroll_frame_160k_expanded_under_2ms(cx: &mut TestAppContext) {
    if std::env::var_os("HACIVAT_PERF").is_none_or(|v| v != "1") {
        return;
    }
    cx.update(|cx| Theme::install(cx, ThemeMode::Dark));
    let big = Big::new(20, 100, 80);
    let parents = big.parents();
    let (tree, vcx) = cx.add_window_view(|_, cx| TreeView::new(big, cx));
    vcx.simulate_resize(size(px(276.0), px(900.0)));
    let at = Instant::now();
    tree.update(vcx, |t, cx| t.set_expanded_keys(parents, cx));
    let flatten = at.elapsed();
    draw(vcx);
    let rows = tree.read_with(vcx, |t, _| t.flat().len());
    assert_eq!(rows, 162_041);

    let mut builds = Vec::new();
    let mut frames = Vec::new();
    let steps = [
        23.0 * 3.0,
        -23.0,
        23.0 * 40.0,
        23.0 * 7_919.0,
        -23.0 * 3_001.0,
        11.5,
    ];
    for i in 0..400 {
        let dy = steps[i % steps.len()];
        let at = Instant::now();
        tree.update(vcx, |t, cx| t.scroll_by(dy, cx));
        draw(vcx);
        frames.push(at.elapsed());
        builds.push(tree.read_with(vcx, |t, _| t.last_build_time()));
    }
    // With assistive technology attached, the items cost on top.
    tree.update(vcx, |t, cx| t.assume_a11y_active(true, cx));
    let mut a11y = Vec::new();
    for i in 0..200 {
        tree.update(vcx, |t, cx| t.scroll_by(steps[i % steps.len()], cx));
        draw(vcx);
        a11y.push(tree.read_with(vcx, |t, _| t.last_build_time()));
    }
    let (build, frame, a11y) = (p95(builds), p95(frames), p95(a11y));
    eprintln!(
        "tree_perf: rows {rows}; flatten {flatten:?}; row build p95 {build:?}; \
         whole test frame p95 {frame:?}; row build with AT p95 {a11y:?}"
    );
    assert!(build < Duration::from_millis(2), "row build p95 {build:?}");
    assert!(
        a11y < Duration::from_millis(2),
        "row build with AT p95 {a11y:?}"
    );
    assert!(flatten < Duration::from_millis(50), "flatten {flatten:?}");
}
